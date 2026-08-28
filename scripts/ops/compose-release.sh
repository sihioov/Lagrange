#!/usr/bin/env bash
# Ordered production Compose workflow. Default is a read-only plan; --apply is
# explicit operator action after production-config validation succeeds. No path
# here enables live trading or calls a provider by itself.
#
# `infrastructure` and `backfill` retain their isolated image-build behavior.
# `release --apply` is different: it runs only from an installed immutable
# release, consumes its trusted V2 manifest, and never rebuilds serving images.
set -euo pipefail

script_dir=$(cd -P "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
root=$(cd -P "$script_dir/../.." && pwd -P)
source "$script_dir/lib/dotenv.sh"

default_compose_file=$root/deploy/compose/compose.yml
default_env_file=$root/deploy/compose/.env
compose_file=${LAGRANGE_COMPOSE_FILE:-$default_compose_file}
env_file=${LAGRANGE_ENV_FILE:-$default_env_file}
release_root=${LAGRANGE_RELEASE_ROOT:-/opt/lagrange}
mode=plan
scope=release
release_override=
release_commit=
owner_beta_access_mode=disabled
owner_beta_paper_mode=disabled

local_image_services=(
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
persistent_local_services=(
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

usage() {
  cat <<'EOF'
Usage: scripts/ops/compose-release.sh
       [--scope infrastructure|backfill|release]
       [--plan|--preflight|--apply]

  --plan       Validate static inputs and print the ordered commands (default).
  --preflight  Validate inputs and Compose expansion without starting services.
  --apply      Apply the selected scope in dependency order.
  --scope infrastructure
                    Bootstrap PostgreSQL/migrations/raw/schema only; it does
                    not require KIS, Auth0/TLS, or future dataset pins.
  --scope backfill  Bootstrap PostgreSQL/migrations/raw/research-worker only;
                    it does not require serving Auth0/TLS or dataset pins.
  --scope release    Installed owner-beta serving release after approved data.
                    It requires the installed strict V2 manifest and starts
                    every local service by exact local Docker image_id with
                    --no-build. It never enables the live profile.

The apply order is host/runtime preflight, Compose config, then the selected
scope. Infrastructure/backfill build their separately approved image sets.
Release does not build: it validates each manifest image ID/revision immediately
before startup, applies a temporary mode-0600 image-ID Compose override, and
checks each persistent local container's actual .Image plus OCI revision after
it starts. One-shot `run` commands use the same build-reset/image-ID override
and omit the opt-in `--build`; they are not claimed as post-start inspected
after --rm.
EOF
}

die() { echo "compose-release: $*" >&2; exit 1; }
blocked() { echo "BLOCKED_EXTERNAL: $*" >&2; exit 2; }

while [ "$#" -gt 0 ]; do
  case "$1" in
    --scope)
      [ "$#" -ge 2 ] || die '--scope needs infrastructure, backfill, or release'
      scope=$2
      shift 2
      ;;
    --plan) mode=plan; shift ;;
    --preflight) mode=preflight; shift ;;
    --apply) mode=apply; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done

case "$scope" in
  infrastructure|backfill|release) ;;
  *) die '--scope must be infrastructure, backfill, or release' ;;
esac

[ -f "$compose_file" ] && [ ! -L "$compose_file" ] || die "Compose file missing or symlinked: $compose_file"
[ -f "$env_file" ] && [ ! -L "$env_file" ] || blocked "production env file missing or symlinked: $env_file"
command -v docker >/dev/null 2>&1 || blocked 'docker is not installed'
docker compose version >/dev/null 2>&1 || blocked 'Docker Compose v2 is unavailable'

bash "$script_dir/validate-production-config.sh" --scope "$scope" --env-file "$env_file"

# Reuse the validator's non-evaluating dotenv contract. In particular, never
# source the env file: a shell interpolation must not become an operator action.
dotenv_load "$env_file" || die "cannot parse production env file: $env_file"
data_dir=$(dotenv_effective_get LAGRANGE_DATA_DIR)
[ -n "$data_dir" ] || die 'production env is missing LAGRANGE_DATA_DIR'
[[ "$data_dir" = /* ]] || die 'LAGRANGE_DATA_DIR must be absolute'
owner_beta_access_mode=$(dotenv_effective_get OWNER_BETA_ACCESS_MODE)
[ -n "$owner_beta_access_mode" ] || owner_beta_access_mode=disabled
owner_beta_paper_mode=$(dotenv_effective_get OWNER_BETA_PAPER_MODE)
[ -n "$owner_beta_paper_mode" ] || owner_beta_paper_mode=disabled

if [ "$mode" != plan ]; then
  # Host preparation remains a distinct operator action. The release workflow
  # only verifies it and never silently provisions directories or secrets.
  LAGRANGE_DATA_ROOT="$data_dir" bash "$script_dir/provision-linux.sh" --preflight
fi

cleanup_release_override() {
  [ -z "${release_override:-}" ] || rm -f -- "$release_override"
}

prepare_installed_release_manifest() {
  local current_link expected_root manifest
  [ "$scope" = release ] && [ "$mode" = apply ] ||
    die 'internal installed-release manifest guard misuse'
  [ "$(id -u)" -eq 0 ] || die 'release --apply must run as root'
  [ "$compose_file" = "$default_compose_file" ] ||
    die 'release --apply must use the installed release Compose file'
  [ "$env_file" = "$default_env_file" ] ||
    die 'release --apply must use the installed protected Compose env file'

  source "$script_dir/lib/release-image-manifest.sh"
  release_image_manifest_require_absolute_path "$release_root" release-root ||
    die "$RELEASE_IMAGE_MANIFEST_ERROR"
  [ -d "$release_root" ] && [ ! -L "$release_root" ] ||
    die 'release-root is absent or symlinked'
  if ! release_image_manifest_trusted_directory "$release_root" release-root; then
    die "$RELEASE_IMAGE_MANIFEST_ERROR"
  fi
  release_commit=$(dotenv_effective_get LAGRANGE_CODE_COMMIT)
  release_image_manifest_is_commit "$release_commit" ||
    die 'installed release env has no exact LAGRANGE_CODE_COMMIT'
  expected_root=$release_root/releases/$release_commit
  [ "$root" = "$expected_root" ] ||
    die 'release --apply must execute the installed current release script'
  current_link=$release_root/current
  [ -L "$current_link" ] || die 'installed release current link is missing'
  [ "$(readlink -- "$current_link")" = "releases/$release_commit" ] ||
    die 'installed release current link does not match manifest commit'
  [ -f "$compose_file" ] && [ ! -L "$compose_file" ] ||
    die 'installed release Compose file is missing or symlinked'
  if ! release_image_manifest_trusted_file "$env_file" installed-release-env; then
    die "$RELEASE_IMAGE_MANIFEST_ERROR"
  fi
  manifest=$root/.lagrange-release-manifest
  if ! release_image_manifest_trusted_file "$manifest" installed-release-manifest; then
    die "$RELEASE_IMAGE_MANIFEST_ERROR"
  fi
  if ! release_image_manifest_load "$manifest" "$release_commit"; then
    die "$RELEASE_IMAGE_MANIFEST_ERROR"
  fi

  release_override=$(mktemp -- "$release_root/.release-image-override.$release_commit.XXXXXX") ||
    die 'cannot create temporary immutable image override'
  chmod 0600 -- "$release_override"
  if ! release_image_manifest_write_compose_override "$release_override"; then
    die "cannot write temporary immutable image override: $RELEASE_IMAGE_MANIFEST_ERROR"
  fi
  [ "$(stat -c '%u:%g:%a' -- "$release_override")" = 0:0:600 ] ||
    die 'temporary immutable image override metadata is unsafe'
  trap cleanup_release_override EXIT
}

inspect_manifest_image() {
  local service=$1 expected_id=$2 expected_revision=$3 inspected actual_id actual_revision
  inspected=$(docker image inspect \
    --format '{{.Id}}|{{index .Config.Labels "org.opencontainers.image.revision"}}' \
    "$expected_id") || die "manifest image is absent locally: $service"
  case "$inspected" in
    *'|'*) ;;
    *) die "manifest image inspection omitted its revision label: $service" ;;
  esac
  actual_id=${inspected%%|*}
  actual_revision=${inspected#*|}
  release_image_manifest_is_image_id "$actual_id" ||
    die "manifest image returned a non-local image_id: $service"
  [ "$actual_id" = "$expected_id" ] ||
    die "manifest image_id mismatch: $service"
  [ "$actual_revision" = "$expected_revision" ] ||
    die "manifest image revision mismatch: $service"
}

verify_manifest_images() {
  local service expected_id expected_revision
  [ -n "$release_override" ] || die 'immutable image override is not prepared'
  for service in "${local_image_services[@]}"; do
    expected_id=${RELEASE_IMAGE_MANIFEST_IDS[$service]:-}
    expected_revision=${RELEASE_IMAGE_MANIFEST_REVISIONS[$service]:-}
    release_image_manifest_is_image_id "$expected_id" ||
      die "manifest lacks an exact image_id: $service"
    release_image_manifest_is_commit "$expected_revision" ||
      die "manifest lacks an exact revision: $service"
    inspect_manifest_image "$service" "$expected_id" "$expected_revision"
  done
}

run_owner_beta_approval_gate() {
  local wrapper output
  [ "$owner_beta_access_mode" = owner_only ] || return 0
  [ "$owner_beta_paper_mode" = disabled ] || blocked 'owner_beta_paper_evidence_unavailable'
  wrapper=$root/scripts/ops/kis-historical-price-beta-artifact.sh
  [ -f "$wrapper" ] && [ ! -L "$wrapper" ] && [ -x "$wrapper" ] ||
    blocked 'owner_beta_approval_wrapper_unavailable'

  # The installed wrapper binds this request to the same current release,
  # exact research-worker image ID/revision, embedded registry, and dedicated
  # read-only artifact mount. Discard its diagnostics and never repeat the
  # artifact identity or approval hashes in this release workflow's output.
  output=$("$wrapper" --approval-check 2>/dev/null) ||
    blocked 'owner_beta_artifact_not_approved'
  [[ "$output" =~ ^HISTORICAL_PRICE_BETA_APPROVAL\ status=ok\ operation=check\ approval_registry_sha256=sha256:[0-9a-f]{64}\ approval_status=APPROVED\ audience=OWNER_ONLY\ vendor_snapshot=true\ strict_pit=false\ capability=PRICE_RETURN_ONLY\ materialization_status=MATERIALIZED\ registration_status=UNREGISTERED\ publication_status=NOT_PUBLISHED\ instrument_count=11\ session_count=2452\ bar_count=26972$ ]] ||
    blocked 'owner_beta_artifact_not_approved'
  printf 'OWNER_BETA_RELEASE_GATE: PASS access=owner_only paper=disabled\n'
}

verify_running_container() {
  local service=$1 expected_id expected_revision container_id inspected actual_id actual_revision
  expected_id=${RELEASE_IMAGE_MANIFEST_IDS[$service]:-}
  expected_revision=${RELEASE_IMAGE_MANIFEST_REVISIONS[$service]:-}
  release_image_manifest_is_image_id "$expected_id" ||
    die "manifest lacks an exact image_id for persistent service: $service"
  release_image_manifest_is_commit "$expected_revision" ||
    die "manifest lacks an exact revision for persistent service: $service"
  mapfile -t container_ids < <(compose ps -q "$service")
  [ "${#container_ids[@]}" -eq 1 ] ||
    die "persistent service did not resolve to exactly one container: $service"
  container_id=${container_ids[0]}
  [[ "$container_id" =~ ^[0-9A-Za-z][0-9A-Za-z_.-]*$ ]] ||
    die "persistent service returned an invalid container identifier: $service"
  inspected=$(docker inspect \
    --format '{{.Image}}|{{index .Config.Labels "org.opencontainers.image.revision"}}' \
    "$container_id") || die "cannot inspect persistent service container: $service"
  case "$inspected" in
    *'|'*) ;;
    *) die "persistent service inspection omitted its revision label: $service" ;;
  esac
  actual_id=${inspected%%|*}
  actual_revision=${inspected#*|}
  release_image_manifest_is_image_id "$actual_id" ||
    die "persistent service returned a non-local image_id: $service"
  [ "$actual_id" = "$expected_id" ] ||
    die "persistent service image_id mismatch: $service"
  [ "$actual_revision" = "$expected_revision" ] ||
    die "persistent service image revision mismatch: $service"
}

compose() {
  local -a files=(--env-file "$env_file" -f "$compose_file")
  [ -z "$release_override" ] || files+=(-f "$release_override")
  # The range profile is never selected here. The live profile is explicitly
  # disabled rather than inheriting a shell/ambient COMPOSE_PROFILES value.
  # These process-local, fail-closed sentinels exist only for inactive Compose
  # interpolation; they are never written to .env.
  if [ "$scope" = infrastructure ]; then
    RESEARCH_APP_ENV=infrastructure-disabled \
    RESEARCH_ENTITLEMENT_REFERENCE=infrastructure-disabled \
    BACKTEST_MIN_FREE_BYTES=0 \
    BACKTEST_MAX_QUEUED_BACKTESTS=0 \
    BACKTEST_RECONCILE_GRACE_SECS=0 \
    BACKTEST_RECONCILE_INTERVAL_SECS=0 \
    RANGE_RAW_BATCH_ID=compose-config-disabled \
    COMPOSE_PROFILES= \
      docker compose "${files[@]}" "$@"
  else
    RANGE_RAW_BATCH_ID=compose-config-disabled \
    COMPOSE_PROFILES= \
      docker compose "${files[@]}" "$@"
  fi
}

if [ "$scope" = release ] && [ "$mode" = apply ]; then
  prepare_installed_release_manifest
fi

compose config --quiet || die 'Compose interpolation/config validation failed'

if [ "$scope" = infrastructure ]; then
  cat <<'EOF'
COMPOSE_INFRASTRUCTURE_ORDER:
  1. build --pull=false db-role-bootstrap db-migrate
  2. up --wait postgres
  3. run --rm --no-deps db-role-bootstrap (exit code is the gate)
  4. run --rm --no-deps db-migrate (exit code is the gate)
  5. run --rm --no-deps research-raw-init (exit code is the gate)
  6. run --rm --no-deps research-schema-check (exit code is the gate)
  7. ps; hand off to the explicit backfill scope only after KIS credentials are available
No research-worker/API/Web/recommendation/candidate/backtest/Paper/reverse-proxy or live profile is started, and no provider/API call is made.
EOF
elif [ "$scope" = backfill ]; then
  cat <<'EOF'
COMPOSE_BACKFILL_BOOTSTRAP_ORDER:
  1. build --pull=false db-role-bootstrap db-migrate research-worker (worker image only; no worker daemon)
  2. up --wait postgres
  3. run --rm --no-deps db-role-bootstrap (exit code is the gate)
  4. run --rm --no-deps db-migrate (exit code is the gate)
  5. run --rm --no-deps research-raw-init (exit code is the gate)
  6. run --rm --no-deps research-schema-check (exit code is the gate)
  7. ps; run backfill-production.sh one-shot dates, then post-backfill-health.sh --scope backfill --check
No API/Web/recommendation/candidate/backtest/Paper/reverse-proxy or live profile is started.
EOF
else
  cat <<'EOF'
COMPOSE_RELEASE_ORDER:
  1. validate installed strict V2 manifest and every local image_id/revision
  2. generate a mode-0600 temporary Compose override (image: sha256:<id>; build reset)
  3. owner_only only: require the embedded-registry artifact approval before the first Compose up
  4. up --no-build --wait postgres
  5. run --rm --no-deps db-role-bootstrap and db-migrate (no --build)
  6. run --rm --no-deps research-raw-init and research-schema-check (no --build)
  7. up --no-build persistent local services; disabled leaves owner-beta-runner inactive, while owner_only starts it only after step 3; Paper remains excluded pending a future evidence gate
  8. up --no-build reverse-proxy; ps
No serving image rebuild, manifest-less activation, range-raw profile, or live profile is allowed.
EOF
fi

if [ "$mode" = plan ]; then
  echo 'PLAN_ONLY: no build, migration, service start, or network call made'
  exit 0
fi
if [ "$mode" = preflight ]; then
  echo "PREFLIGHT: PASS (scope=$scope)"
  exit 0
fi

if [ "$scope" = infrastructure ]; then
  compose build --pull=false db-role-bootstrap db-migrate
  compose up --wait postgres
  compose run --rm --no-deps db-role-bootstrap
  compose run --rm --no-deps db-migrate
  compose run --rm --no-deps research-raw-init
  compose run --rm --no-deps research-schema-check
  compose ps
  echo 'COMPOSE_INFRASTRUCTURE: PASS (no worker daemon or provider/API call made; next run backfill after KIS credentials are available)'
  exit 0
fi

if [ "$scope" = backfill ]; then
  compose build --pull=false \
    db-role-bootstrap db-migrate research-worker
  compose up --wait postgres
  compose run --rm --no-deps db-role-bootstrap
  compose run --rm --no-deps db-migrate
  compose run --rm --no-deps research-raw-init
  compose run --rm --no-deps research-schema-check
  compose ps
  echo 'COMPOSE_BACKFILL_BOOTSTRAP: PASS (worker image built; run backfill one-shots, then post-backfill-health.sh --scope backfill --check)'
  exit 0
fi

# Owner-beta serving release: the helper above has already bound Compose to the
# installed manifest. No mutable tag is used for these eleven local services.
verify_manifest_images
run_owner_beta_approval_gate
compose up --no-build --wait postgres
verify_manifest_images
compose run --rm --no-deps db-role-bootstrap
compose run --rm --no-deps db-migrate
compose run --rm --no-deps research-raw-init
compose run --rm --no-deps research-schema-check
verify_manifest_images
# The legacy ordering contract was `compose up --wait --no-deps api-server`;
# owner-beta additionally supplies --no-build before it reaches Docker.
compose up --no-build --wait --no-deps api-server
verify_running_container api-server
compose up --no-build --wait --no-deps web
verify_running_container web
# These services remain detached without --wait: their functional healthchecks
# require approved EOD/curated data. post-backfill-health.sh owns that gate;
# use post-backfill-health.sh --check only after the approved backfill path.
# The legacy ordering contract was `compose up --no-deps -d research-worker recommendation-runner candidate-runner`;
# owner-beta additionally supplies --no-build before it reaches Docker.
release_worker_services=(
  research-worker recommendation-runner candidate-runner
  nt-backtest-worker-1 nt-backtest-worker-2
)
if [ "$owner_beta_access_mode" = disabled ]; then
  release_worker_services+=(paper-scheduler)
elif [ "$owner_beta_access_mode" = owner_only ]; then
  # Explicitly target this profile service only after the immutable approval
  # gate above passed; ambient COMPOSE_PROFILES remains cleared in compose().
  release_worker_services=(
    research-worker recommendation-runner candidate-runner owner-beta-runner
    nt-backtest-worker-1 nt-backtest-worker-2
  )
fi
compose up --no-build --no-deps -d "${release_worker_services[@]}"
for service in "${release_worker_services[@]}"; do
  verify_running_container "$service"
done
compose up --no-build --wait --no-deps reverse-proxy
compose ps
echo 'COMPOSE_RELEASE: PASS (immutable manifest image IDs bound; run post-backfill-health.sh --scope release --check to assert data readiness)'
