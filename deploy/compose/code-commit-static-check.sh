#!/usr/bin/env bash
# Static contract check for immutable API, research-worker, and backtest image
# provenance.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
compose="$root/deploy/compose/compose.yml"
dockerfile="$root/crates/api-server/Dockerfile"
worker_dockerfile="$root/data-pipelines/collectors/Dockerfile"
backtest_dockerfile="$root/crates/job-queue/Dockerfile.backtest-runner"
env_example="$root/deploy/compose/.env.example"

die() {
  echo "code-commit-static-check: $*" >&2
  exit 1
}

[ -f "$compose" ] || die "missing Compose file"
[ -f "$dockerfile" ] || die "missing API Dockerfile"
[ -f "$worker_dockerfile" ] || die "missing research-worker Dockerfile"
[ -f "$backtest_dockerfile" ] || die "missing backtest-runner Dockerfile"
[ -f "$env_example" ] || die "missing Compose env example"
for image_dockerfile in "$dockerfile" "$worker_dockerfile" "$backtest_dockerfile"; do
  grep -Fq 'ARG LAGRANGE_CODE_COMMIT' "$image_dockerfile" \
    || die "$image_dockerfile must declare LAGRANGE_CODE_COMMIT"
  grep -Fq "grep -Eq '^[0-9a-f]{40}$'" "$image_dockerfile" \
    || die "$image_dockerfile must validate an exact lowercase 40-hex commit"
  grep -Fq 'LABEL org.opencontainers.image.revision="$LAGRANGE_CODE_COMMIT"' "$image_dockerfile" \
    || die "$image_dockerfile must label the image revision"
  grep -Fq 'ENV LAGRANGE_CODE_COMMIT="$LAGRANGE_CODE_COMMIT"' "$image_dockerfile" \
    || die "$image_dockerfile must bake the commit into runtime ENV"
done
grep -Fq 'STOPSIGNAL SIGTERM' "$backtest_dockerfile" \
  || die 'backtest-runner image must deliver SIGTERM to its supervisor'
build_arg_count=$(grep -Fc 'LAGRANGE_CODE_COMMIT: ${LAGRANGE_CODE_COMMIT:?' "$compose")
[ "$build_arg_count" -ge 5 ] \
  || die 'Compose must pass the required commit to API, workers, and both backtest images'
grep -Fq 'export LAGRANGE_CODE_COMMIT="$(git rev-parse HEAD)"' "$env_example" \
  || die 'env example must document the exact CI commit command'

# API shutdown is bounded by the 30s in-flight request drain plus the 10s
# durable audit outbox drain. Keep Compose's stop window above that 40s
# process-level budget, with a 5s margin for signal delivery and cleanup.
api_stop_grace=$(awk '
  $0 == "  api-server:" { inside=1; next }
  inside && $0 ~ /^  [^ ]/ { inside=0 }
  inside && $1 == "stop_grace_period:" { print $2; exit }
' "$compose")
[ "$api_stop_grace" = "45s" ] \
  || die 'api-server stop_grace_period must be 45s (above the 40s shutdown contract)'

# Paper's process-level shutdown budget is 20 seconds. Compose must leave a
# larger stop window so SIGTERM cancellation and pool closure can complete.
paper_stop_grace=$(awk '
  $0 == "  paper-scheduler:" { inside=1; next }
  inside && $0 ~ /^  [^ ]/ { inside=0 }
  inside && $1 == "stop_grace_period:" { print $2; exit }
' "$compose")
[ "$paper_stop_grace" = "30s" ] \
  || die 'paper-scheduler stop_grace_period must be 30s (above the 20s shutdown contract)'

# Runtime provenance must come from the image ENV, not a mutable Compose
# environment override. A build-arg value is eight spaces deep; an environment
# key would be six spaces deep inside each service block.
for service in api-server nt-backtest-worker-1 nt-backtest-worker-2; do
  if awk -v service="$service" '
    $0 == "  " service ":" { inside=1; next }
    inside && $0 ~ /^  [^ ]/ { inside=0 }
    inside && $0 ~ /^      LAGRANGE_CODE_COMMIT:/ { found=1 }
    END { exit found ? 0 : 1 }
  ' "$compose"; then
    die "$service must not override the image-baked commit at runtime"
  fi
done

command -v docker >/dev/null 2>&1 || die 'Docker Compose CLI is required'
valid_commit=0123456789abcdef0123456789abcdef01234567
if ! LAGRANGE_CODE_COMMIT="$valid_commit" \
  RANGE_RAW_BATCH_ID=compose-config-disabled \
  RESEARCH_APP_ENV=production RESEARCH_FETCH_MODE=credentialed \
  docker compose --env-file "$env_example" -f "$compose" config --quiet; then
  die 'Compose rejected a valid 40-hex CI commit'
fi
if env -u LAGRANGE_CODE_COMMIT \
  RANGE_RAW_BATCH_ID=compose-config-disabled \
  RESEARCH_APP_ENV=production RESEARCH_FETCH_MODE=credentialed \
  docker compose --env-file "$env_example" -f "$compose" config --quiet \
  >/tmp/lagrange-code-commit-missing.out 2>&1; then
  die 'Compose accepted a missing CI commit'
fi
grep -Fq 'LAGRANGE_CODE_COMMIT' /tmp/lagrange-code-commit-missing.out \
  || die 'missing-commit failure did not identify LAGRANGE_CODE_COMMIT'
rm -f /tmp/lagrange-code-commit-missing.out

"$root/deploy/compose/backtest-static-check.sh"

echo 'CODE_COMMIT_STATIC: PASS'
