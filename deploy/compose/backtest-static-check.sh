#!/usr/bin/env bash
# Static contract check for the Compose backtest supervisors.
#
# The Rust supervisor's shutdown budget is 15 seconds. Compose must allow
# longer than that before sending SIGKILL, or a normal deployment stop can
# strand a RUNNING claim and bypass the supervisor's settlement path.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
compose="$root/deploy/compose/compose.yml"
publication_doc="$root/deploy/runtime/backtest-artifact-publication.md"
dockerfile="$root/crates/job-queue/Dockerfile.backtest-runner"

die() {
  echo "backtest-compose-static-check: $*" >&2
  exit 1
}

[ -f "$compose" ] || die "missing Compose file"
[ -f "$publication_doc" ] || die "missing immutable publication runbook"
[ -f "$dockerfile" ] || die "missing backtest-runner Dockerfile"
grep -Fq 'generations/<40-hex-code-commit>/<publication-uuid>/artifacts' "$publication_doc" \
  || die 'publication runbook must document the immutable generation layout'
grep -Fq 'ambiguous' "$publication_doc" \
  || die 'publication runbook must document ambiguous COMMIT cleanup'
grep -Fq 'COMMIT_UNKNOWN.json' "$publication_doc" \
  || die 'publication runbook must document the durable commit-unknown marker'
grep -Fq 'BACKTEST_PUBLICATION_COMMIT_UNKNOWN' "$publication_doc" \
  || die 'publication runbook must document the stable commit-unknown event code'
grep -Fq 'backtest-runner", "readiness"' "$dockerfile" \
  || die 'backtest-runner image healthcheck must expose readiness'

service_block() {
  local service=$1
  awk -v service="$service" '
    $0 == "  " service ":" { in_service=1; print; next }
    in_service && $0 ~ /^  [^[:space:]][^:]*:/ { exit }
    in_service { print }
  ' "$compose"
}

for service in nt-backtest-worker-1 nt-backtest-worker-2; do
  block=$(service_block "$service")
  [ -n "$block" ] || die "missing service: $service"
  grep -Fxq '    stop_grace_period: 20s' <<<"$block" \
    || die "$service must allow more than the 15s Rust shutdown budget"
  grep -Fxq '    init: true' <<<"$block" \
    || die "$service must run with an init supervisor"
  grep -Fq 'entrypoint: ["/usr/local/bin/backtest-runner"]' <<<"$block" \
    || die "$service must invoke the Rust backtest supervisor"
done

grep -Fq 'immutable generations/<commit>/<publication-uuid>' "$compose" \
  || die 'Compose must document immutable generation mounts'

for service in nt-backtest-worker-1 nt-backtest-worker-2; do
  block=$(service_block "$service")
  for setting in BACKTEST_MIN_FREE_BYTES BACKTEST_MAX_QUEUED_BACKTESTS \
    BACKTEST_RECONCILE_GRACE_SECS BACKTEST_RECONCILE_INTERVAL_SECS; do
    grep -Fq "$setting" <<<"$block" \
      || die "$service must configure fail-closed $setting"
  done
  grep -Fq '"/usr/local/bin/backtest-runner", "readiness"' <<<"$block" \
    || die "$service healthcheck must expose capacity/readiness"
done

grep -Fq 'BACKTEST_MIN_FREE_BYTES=' "$root/deploy/compose/.env.example" \
  || die 'Compose env example must document backtest capacity settings'

echo 'BACKTEST_COMPOSE_STATIC: PASS'
