#!/usr/bin/env bash
# Static contract check for owner-beta local image provenance. This never calls
# Docker, a provider, a database, or a runtime service.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
ops=$root/scripts/ops
build=$ops/build-production-images.sh
compose_release=$ops/compose-release.sh
manifest_lib=$ops/lib/release-image-manifest.sh
compose_file=$root/deploy/compose/compose.yml

die() { echo "production-image-build-static: $*" >&2; exit 1; }

for path in "$build" "$compose_release" "$manifest_lib"; do
  [ -f "$path" ] || die "required provenance helper is missing: $path"
  bash -n "$path" || die "shell syntax failure: $path"
done
self_test=$ops/build-production-images-self-test.sh
bash -n "$self_test" || die 'image build self-test has shell syntax errors'
grep -Fq 'TEST_ENVIRONMENT_ERROR: image-build root fixture requires user namespaces or fakeroot' \
  "$self_test" || die 'root fixture must fail rather than skip when unavailable'
if grep -Fq 'PASS (apply fixture skipped' "$self_test" || grep -Fq 'fakeroot "$@"' "$self_test"; then
  die 'image build root fixture must not pass-on-skip or nest fakeroot'
fi

grep -Fq 'mode=plan' "$build" || die 'image build helper must default to plan'
grep -Fq -- '--preflight' "$build" || die 'image build preflight mode missing'
grep -Fq -- '--apply' "$build" || die 'image build apply mode missing'
grep -Fq -- '--manifest-file' "$build" || die 'strict manifest output option missing'
grep -Fq -- '--apply requires --manifest-file' "$build" || die 'apply must require a manifest output file'
grep -Fq 'LAGRANGE_RELEASE_MANIFEST_V2' "$manifest_lib" || die 'strict V2 manifest format missing'
grep -Fq 'canonical whitelist/order' "$manifest_lib" || die 'strict stable service ordering missing'
grep -Fq 'manifest record count is not canonical' "$manifest_lib" || die 'strict record count missing'
grep -Fq 'trailing separator' "$manifest_lib" || die 'strict field-separator validation missing'
grep -Fq 'manifest configured image reference disagrees with Compose' "$manifest_lib" ||
  die 'strict configured image reference check missing'
grep -Fq 'local Docker image ID' "$manifest_lib" || die 'local image_id terminology missing'
grep -Fq 'RepoDigest' "$manifest_lib" || die 'single-host provenance terminology missing'
grep -Fq 'docker image inspect' "$build" || die 'built-image inspection missing'
grep -Fq '{{.Id}}|{{index .Config.Labels "org.opencontainers.image.revision"}}' "$build" ||
  die 'image_id/revision inspection contract missing'
grep -Fq 'built image_id is not an exact local Docker image ID' "$build" ||
  die 'image_id validation missing'
grep -Fq 'built image revision label does not match source commit' "$build" ||
  die 'image revision mismatch failure missing'
grep -Fq 'manifest-file already exists; refusing to overwrite it' "$build" ||
  die 'manifest overwrite refusal missing'
grep -Fq 'git -c "safe.directory=$root" -C "$root"' "$build" ||
  die 'Git command-local safe.directory fence missing'
grep -Fq "rev-parse --verify 'HEAD^{commit}'" "$build" || die 'build HEAD provenance check missing'
grep -Fq 'status --porcelain=v1 --untracked-files=all' "$build" ||
  die 'tracked/untracked clean-worktree check missing'
grep -Fq '?? docs/kis_openapi_entiredocs_20260818_030007.xlsx' "$build" ||
  die 'official workbook must be the sole untracked build exception'
grep -Fq 'does not match the build root HEAD' "$build" || die 'commit mismatch failure missing'
grep -Fq 'docker compose --env-file' "$build" || die 'Compose env-file invocation missing'
grep -Fq 'config --quiet' "$build" || die 'Compose config preflight missing'
grep -Fq 'build --pull=false' "$build" || die 'pull=false build contract missing'
grep -Fq 'RESEARCH_APP_ENV=prebuild-disabled' "$build" || die 'research prebuild sentinel missing'
grep -Fq 'RESEARCH_ENTITLEMENT_REFERENCE=prebuild-disabled' "$build" ||
  die 'entitlement prebuild sentinel missing'
grep -Fq 'BACKTEST_MIN_FREE_BYTES=0' "$build" || die 'backtest prebuild sentinel missing'
grep -Fq 'never edits the env file' "$build" || die 'env-file no-write documentation missing'
grep -Fq 'network caveat' "$build" || die 'build network caveat missing'
if grep -Fq -- '--repo-root' "$build"; then
  die 'public alternate repo-root override must not exist'
fi
if grep -Eq '\.RepoDigests|RepoDigest[s]?[[:space:]:=]' "$build"; then
  die 'build manifest must not claim Docker RepoDigest provenance'
fi

services=(
  db-role-bootstrap
  db-migrate
  api-server
  web
  research-worker
  recommendation-runner
  candidate-runner
  owner-beta-runner
  nt-backtest-worker-1
  nt-backtest-worker-2
  paper-scheduler
)
grep -Fq 'local_image_services=("${RELEASE_IMAGE_SERVICES[@]}")' "$build" ||
  die 'build helper must derive scope only from the canonical manifest whitelist'

service_block() {
  local service=$1
  awk -v service="$service" '
    $0 == "  " service ":" { inside=1; next }
    inside && /^  [A-Za-z0-9_-]+:/ { exit }
    inside { print }
  ' "$compose_file"
}

for service in "${services[@]}"; do
  grep -Fq -- "  $service" "$manifest_lib" || die "local build service missing: $service"
  block=$(service_block "$service")
  [ -n "$block" ] || die "Compose service block missing: $service"
  grep -Fq "image: lagrange-station-$service:\${LAGRANGE_CODE_COMMIT:?" <<<"$block" ||
    die "Compose image must use exact commit-specific tag: $service"
  grep -Fq 'LAGRANGE_CODE_COMMIT: ${LAGRANGE_CODE_COMMIT:?' <<<"$block" ||
    die "Compose build arg missing for local image: $service"
done

declare -A service_dockerfile=(
  [db-role-bootstrap]=deploy/db/Dockerfile
  [db-migrate]=deploy/db/Dockerfile
  [api-server]=crates/api-server/Dockerfile
  [web]=apps/web/Dockerfile
  [research-worker]=data-pipelines/collectors/Dockerfile
  [recommendation-runner]=crates/job-queue/Dockerfile
  [candidate-runner]=crates/job-queue/Dockerfile
  [owner-beta-runner]=crates/job-queue/Dockerfile.owner-beta-runner
  [nt-backtest-worker-1]=crates/job-queue/Dockerfile.backtest-runner
  [nt-backtest-worker-2]=crates/job-queue/Dockerfile.backtest-runner
  [paper-scheduler]=deploy/runtime/Dockerfile.paper-runner
)
for service in "${services[@]}"; do
  dockerfile=$root/${service_dockerfile[$service]}
  grep -Fq 'ARG LAGRANGE_CODE_COMMIT' "$dockerfile" ||
    die "Dockerfile build ARG missing: $service"
  grep -Fq 'LABEL org.opencontainers.image.revision="$LAGRANGE_CODE_COMMIT"' "$dockerfile" ||
    die "Dockerfile OCI revision label missing: $service"
  grep -Fq "grep -Eq '^[0-9a-f]{40}$'" "$dockerfile" ||
    die "Dockerfile exact revision validation missing: $service"
  case "$service" in
    db-role-bootstrap|db-migrate|web) ;;
    *)
      grep -Fq 'COPY configs/evidence/kis-historical-price-only-beta-approved-artifacts.json ./configs/evidence/kis-historical-price-only-beta-approved-artifacts.json' "$dockerfile" ||
        die "Dockerfile missing embedded historical-price-only approval registry: $service"
      ;;
  esac
done

# The profile-gated range service has a deliberately different, operator-gated
# history-capture contract and may not enter the serving build/manifest scope.
range_block=$(service_block research-range-raw)
grep -Fq 'profiles: ["range-raw"]' <<<"$range_block" || die 'range-raw profile gate missing'
grep -Fq 'owner-beta serving manifest' "$compose_file" ||
  die 'range-raw manifest exclusion is undocumented'
if sed -n '/RELEASE_IMAGE_SERVICES=(/,/)/p' "$manifest_lib" | grep -Fq 'research-range-raw'; then
  die 'range-raw must not join the local serving image manifest'
fi

grep -Fq 'prepare_installed_release_manifest' "$compose_release" ||
  die 'installed-manifest release guard missing'
grep -Fq 'release_image_manifest_write_compose_override' "$compose_release" ||
  die 'immutable Compose override generator missing'
grep -Fq 'build: !reset null' "$manifest_lib" || die 'Compose build reset contract missing'
grep -Fq 'compose up --no-build' "$compose_release" || die 'release up must pass --no-build'
run_without_build_count=$(awk '
  /^# Owner-beta serving release:/ { in_release = 1 }
  in_release && /compose run --rm --no-deps/ { count++ }
  END { print count + 0 }
' "$compose_release")
[ "$run_without_build_count" -eq 4 ] || die 'release must have exactly four manifest-bound one-shot runs without --build'
if awk '/^# Owner-beta serving release:/ { in_release = 1 } in_release { print }' \
    "$compose_release" | grep -Eq 'compose run .*--build|compose run --no-build'; then
  die 'release one-shots must omit the opt-in --build and unsupported --no-build flags'
fi
grep -Fq 'verify_manifest_images' "$compose_release" || die 'pre-start image_id verification missing'
grep -Fq "'{{.Image}}|{{index .Config.Labels \"org.opencontainers.image.revision\"}}'" \
  "$compose_release" || die 'running-container image_id/revision inspection missing'
grep -Fq 'verify_running_container api-server' "$compose_release" ||
  die 'API running-container identity check missing'
grep -Fq 'verify_running_container web' "$compose_release" ||
  die 'web running-container identity check missing'
grep -Fq 'persistent service image_id mismatch' "$compose_release" ||
  die 'running-container image_id mismatch failure missing'
grep -Fq 'persistent service image revision mismatch' "$compose_release" ||
  die 'running-container revision mismatch failure missing'
release_apply_block=$(awk '
  /^# Owner-beta serving release:/ { inside=1 }
  inside { print }
' "$compose_release")
if grep -Fq 'compose build' <<<"$release_apply_block"; then
  die 'owner-beta release apply must not rebuild images'
fi
if grep -E '^compose up ' <<<"$release_apply_block" | grep -Fv -- '--no-build' >/dev/null; then
  die 'every owner-beta release up must pass --no-build'
fi
if grep -E '^compose run ' <<<"$release_apply_block" | grep -Eq -- '--build|--no-build'; then
  die 'owner-beta release run must omit both --build and unsupported --no-build'
fi

# Cargo resolves every workspace member manifest while loading a package, even
# when a member is only a dev-dependency. Keep this pre-existing build-context
# contract explicit for every Dockerfile that runs a workspace cargo build.
while IFS= read -r dockerfile; do
  if ! grep -Eq '^[[:space:]]*RUN[[:space:]].*cargo[[:space:]]+build([[:space:]]|$)' "$dockerfile"; then
    continue
  fi
  for copy_contract in \
    'COPY Cargo.toml Cargo.lock rust-toolchain.toml ./' \
    'COPY crates ./crates' \
    'COPY data-pipelines/collectors ./data-pipelines/collectors' \
    'COPY apps/api-server/auth ./apps/api-server/auth' \
    'COPY tests/integration/migration-contract ./tests/integration/migration-contract' \
    'COPY configs/evidence/kis-range-canonical-approved-manifests.json ./configs/evidence/kis-range-canonical-approved-manifests.json' \
    'COPY data/calendars/xkrx/calendar.json ./data/calendars/xkrx/calendar.json' \
    'COPY data/calendars/xkrx/manifest.json ./data/calendars/xkrx/manifest.json' \
    'COPY data/calendars/xkrx/overrides.json ./data/calendars/xkrx/overrides.json'; do
    grep -Fq -- "$copy_contract" "$dockerfile" ||
      die "Rust workspace Dockerfile is missing required copy contract: $dockerfile ($copy_contract)"
  done
done < <(find "$root" -type f \( -name Dockerfile -o -name 'Dockerfile.*' \) \
  -not -path "$root/.git/*" -print | sort)

collector_dockerfile=$root/data-pipelines/collectors/Dockerfile
universe_copy='COPY configs/universes/kr-etf-core-v1.yaml ./configs/universes/kr-etf-core-v1.yaml'
grep -Fq -- "$universe_copy" "$collector_dockerfile" ||
  die "range worker Dockerfile is missing immutable input copy: $universe_copy"
for range_context in \
  '!configs/evidence/kis-range-canonical-approved-manifests.json' \
  '!data/calendars/xkrx/calendar.json' \
  '!data/calendars/xkrx/manifest.json' \
  '!data/calendars/xkrx/overrides.json'; do
  grep -Fqx -- "$range_context" "$root/.dockerignore" ||
    die "Docker build context does not allow required range input: $range_context"
done

if grep -Eq '^[[:space:]]*(docker compose|compose)[[:space:]].*(up|run|restart|start)[[:space:]]' "$build"; then
  die 'image prebuild helper must not invoke a lifecycle command'
fi
for forbidden in kis_app_key kis_app_secret auth0_client_secret psql curl wget; do
  if grep -Eiq "^[^#]*$forbidden" "$build"; then
    die "image prebuild helper must not read or invoke $forbidden"
  fi
done
if grep -Eq '^[[:space:]]*git[[:space:]]+rev-parse' "$build"; then
  die 'image prebuild helper must not derive the commit from a shell checkout command'
fi

echo 'PRODUCTION_IMAGE_BUILD_STATIC: PASS'
