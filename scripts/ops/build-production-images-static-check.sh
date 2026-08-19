#!/usr/bin/env bash
# Static contract check for the KIS/pin-independent production image prebuild.
# This never invokes Docker or any service lifecycle command.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
script="$root/scripts/ops/build-production-images.sh"

die() { echo "production-image-build-static: $*" >&2; exit 1; }

[ -f "$script" ] || die 'image build helper is missing'
[ -x "$script" ] || die 'image build helper must be executable'
bash -n "$script" || die 'image build helper has shell syntax errors'

grep -Fq 'mode=plan' "$script" || die 'image build helper must default to plan'
grep -Fq -- '--preflight' "$script" || die 'image build preflight mode missing'
grep -Fq -- '--apply' "$script" || die 'image build apply mode missing'
grep -Fq 'LAGRANGE_CODE_COMMIT' "$script" || die 'commit input contract missing'
grep -Fq '[[ "$commit" =~ ^[0-9a-f]{40}$ ]]' "$script" \
  || die 'exact lowercase 40-hex commit validation missing'
grep -Fq 'git -c "safe.directory=$root" -C "$root"' "$script" \
  || die 'Git command-local safe.directory fence missing'
grep -Fq "rev-parse --verify 'HEAD^{commit}'" "$script" \
  || die 'build HEAD provenance check missing'
grep -Fq 'status --porcelain=v1 --untracked-files=all' "$script" \
  || die 'tracked/untracked clean-worktree check missing'
grep -Fq '?? docs/kis_openapi_entiredocs_20260818_030007.xlsx' "$script" \
  || die 'official workbook must be the sole untracked build exception'
grep -Fq 'does not match the build root HEAD' "$script" \
  || die 'commit mismatch failure missing'
grep -Fq 'docker compose --env-file' "$script" || die 'Compose env-file invocation missing'
grep -Fq 'config --quiet' "$script" || die 'Compose config preflight missing'
grep -Fq 'build --pull=false' "$script" || die 'pull=false build contract missing'
grep -Fq 'RESEARCH_APP_ENV=prebuild-disabled' "$script" \
  || die 'research prebuild sentinel missing'
grep -Fq 'RESEARCH_ENTITLEMENT_REFERENCE=prebuild-disabled' "$script" \
  || die 'entitlement prebuild sentinel missing'
grep -Fq 'BACKTEST_MIN_FREE_BYTES=0' "$script" || die 'backtest prebuild sentinel missing'
grep -Fq 'never edits' "$script" || die 'env-file no-write documentation missing'
grep -Fq 'network caveat' "$script" || die 'build network caveat missing'
if grep -Fq -- '--repo-root' "$script"; then
  die 'public alternate repo-root override must not exist'
fi

for service in db-role-bootstrap db-migrate api-server web research-worker \
  recommendation-runner candidate-runner nt-backtest-worker-1 \
  nt-backtest-worker-2 paper-scheduler reverse-proxy; do
  grep -Fq -- "  $service" "$script" || die "release service missing: $service"
done

# Cargo resolves every workspace member manifest while loading a package, even
# when a member is only a dev-dependency (api-server/job-queue use collectors
# that way). Keep this an explicit Dockerfile contract rather than allowing
# each Rust image to discover a missing workspace path one build at a time.
# The loop covers every tracked Dockerfile that runs a workspace cargo build;
# deploy/db/Dockerfile intentionally uses `cargo install` and is not in scope.
while IFS= read -r dockerfile; do
  if ! grep -Eq '^[[:space:]]*RUN[[:space:]].*cargo[[:space:]]+build([[:space:]]|$)' "$dockerfile"; then
    continue
  fi
  for copy_contract in \
    'COPY Cargo.toml Cargo.lock rust-toolchain.toml ./' \
    'COPY crates ./crates' \
    'COPY data-pipelines/collectors ./data-pipelines/collectors' \
    'COPY apps/api-server/auth ./apps/api-server/auth' \
    'COPY tests/integration/migration-contract ./tests/integration/migration-contract'; do
    grep -Fq -- "$copy_contract" "$dockerfile" ||
      die "Rust workspace Dockerfile is missing required copy contract: $dockerfile ($copy_contract)"
  done
done < <(find "$root" -type f \( -name Dockerfile -o -name 'Dockerfile.*' \) \
  -not -path "$root/.git/*" -print | sort)

# The isolated range worker compiles market-data's approved-evidence and XKRX
# loaders with include_bytes!. Keep those exact immutable inputs in the
# production build context; a successful local workspace build must not hide
# a missing Docker COPY contract.
collector_dockerfile="$root/data-pipelines/collectors/Dockerfile"
for range_copy in \
  'COPY configs/evidence/kis-range-canonical-approved-manifests.json ./configs/evidence/kis-range-canonical-approved-manifests.json' \
  'COPY configs/universes/kr-etf-core-v1.yaml ./configs/universes/kr-etf-core-v1.yaml' \
  'COPY data/calendars/xkrx/calendar.json ./data/calendars/xkrx/calendar.json' \
  'COPY data/calendars/xkrx/manifest.json ./data/calendars/xkrx/manifest.json'; do
  grep -Fq -- "$range_copy" "$collector_dockerfile" \
    || die "range worker Dockerfile is missing immutable input copy: $range_copy"
done
for range_context in \
  '!configs/evidence/kis-range-canonical-approved-manifests.json' \
  '!data/calendars/xkrx/calendar.json' \
  '!data/calendars/xkrx/manifest.json'; do
  grep -Fqx -- "$range_context" "$root/.dockerignore" \
    || die "Docker build context does not allow required range input: $range_context"
done

# The only Docker Compose verbs allowed in this helper are version, config,
# and build.  Keep this check line-oriented so prose explaining forbidden
# lifecycle verbs does not produce a false positive.
if grep -Eq '^[[:space:]]*(docker compose|compose)[[:space:]].*(up|run|restart|start)[[:space:]]' "$script"; then
  die 'image prebuild helper must not invoke a lifecycle command'
fi
for forbidden in kis_app_key kis_app_secret auth0_client_secret psql curl wget; do
  if grep -Eiq "^[^#]*$forbidden" "$script"; then
    die "image prebuild helper must not read or invoke $forbidden"
  fi
done
if grep -Eq '^[[:space:]]*git[[:space:]]+rev-parse' "$script"; then
  die 'image prebuild helper must not derive the commit from a shell checkout command'
fi

echo 'PRODUCTION_IMAGE_BUILD_STATIC: PASS'
