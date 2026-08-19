#!/usr/bin/env bash
# Ordered production Compose workflow. Default is a read-only plan; --apply
# is the explicit operator action after production-config validation succeeds.
# No command here generates secrets, enables live trading, or calls KIS itself.
#
# The `infrastructure` scope starts only the DB/raw/schema gates and never
# needs KIS credentials, Auth0/TLS, or future dataset pins. The `backfill`
# scope additionally builds the research-worker image for later one-shot KIS
# reads. The `release` scope starts the full serving stack after curation
# approval. A clean database has no EOD row yet, while research-worker's
# healthcheck correctly fails closed until the first approved KIS publication
# exists. The post-backfill-health.sh gate owns that later assertion.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$script_dir/../.." && pwd)
source "$script_dir/lib/dotenv.sh"
compose_file=${LAGRANGE_COMPOSE_FILE:-$root/deploy/compose/compose.yml}
env_file=${LAGRANGE_ENV_FILE:-$root/deploy/compose/.env}
mode=plan
scope=release

usage() {
  cat <<'EOF'
Usage: scripts/ops/compose-release.sh
       [--scope infrastructure|backfill|release]
       [--plan|--preflight|--apply]

  --plan       Validate static inputs and print the ordered commands (default).
  --preflight  Validate inputs and Compose expansion without starting services.
  --apply      Build and start the selected scope in dependency order.
  --scope infrastructure
                    Bootstrap PostgreSQL/migrations/raw/schema only; it does
                    not require KIS, Auth0/TLS, or future dataset pins.
  --scope backfill  Bootstrap PostgreSQL/migrations/raw/research-worker only;
                    it does not require serving Auth0/TLS or dataset pins.
  --scope release    Full serving release after the approved dataset pin
                     (default).

The apply order is host/runtime preflight, Compose config, image builds,
PostgreSQL, role bootstrap, migrations, raw ownership, schema check, and the
selected scope's services. Infrastructure scope stops after those one-shot
gates; it performs no provider/API call and starts no worker daemon. Release
scope adds API/Web, data-dependent workers, and reverse-proxy; backfill scope
stops after the research-worker image build and one-shot infrastructure gates,
before any worker daemon starts. One-shot failures stop the release. A clean
install is not reported data-healthy until post-backfill-health.sh --check
passes.
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

[ -f "$compose_file" ] || die "Compose file missing: $compose_file"
[ -f "$env_file" ] || blocked "production env file missing: $env_file"
command -v docker >/dev/null 2>&1 || blocked 'docker is not installed'
docker compose version >/dev/null 2>&1 || blocked 'Docker Compose v2 is unavailable'

bash "$script_dir/validate-production-config.sh" --scope "$scope" --env-file "$env_file"

# Reuse the validator's non-evaluating dotenv contract so host preflight uses
# the same absolute data root that Compose will bind.  In particular, do not
# source the env file: shell interpolation precedence has already been checked
# above and a custom LAGRANGE_DATA_DIR must not silently fall back to
# provision-linux.sh's /var/lib default.
dotenv_load "$env_file" || die "cannot parse production env file: $env_file"
data_dir=$(dotenv_effective_get LAGRANGE_DATA_DIR)
[ -n "$data_dir" ] || die 'production env is missing LAGRANGE_DATA_DIR'
[[ "$data_dir" = /* ]] || die 'LAGRANGE_DATA_DIR must be absolute'

if [ "$mode" != plan ]; then
  # Host preparation is deliberately a separate explicit operator action; the
  # release workflow only verifies it and never silently runs root provisioning.
  LAGRANGE_DATA_ROOT="$data_dir" bash "$script_dir/provision-linux.sh" --preflight
fi

compose() {
  # The profile-gated range service is never started by this workflow, but
  # Compose expands its interpolation even when the profile is inactive.
  # Keep a non-secret config-only sentinel here; the isolated Stage5 wrapper
  # always replaces it with its deterministic UUID before a range run.
  if [ "$scope" = infrastructure ]; then
    # Compose expands the complete file even when only infrastructure services
    # are selected. Keep deferred worker settings out of the operator env
    # contract by supplying process-local, fail-closed sentinels solely for
    # this scope. They are never written to .env and backfill/release never use
    # this branch; the infrastructure path does not start any worker.
    RESEARCH_APP_ENV=infrastructure-disabled \
    RESEARCH_ENTITLEMENT_REFERENCE=infrastructure-disabled \
    BACKTEST_MIN_FREE_BYTES=0 \
    BACKTEST_MAX_QUEUED_BACKTESTS=0 \
    BACKTEST_RECONCILE_GRACE_SECS=0 \
    BACKTEST_RECONCILE_INTERVAL_SECS=0 \
    RANGE_RAW_BATCH_ID=compose-config-disabled \
      docker compose --env-file "$env_file" -f "$compose_file" "$@"
  else
    RANGE_RAW_BATCH_ID=compose-config-disabled \
      docker compose --env-file "$env_file" -f "$compose_file" "$@"
  fi
}

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
  1. build --pull=false db-role-bootstrap db-migrate api-server web research-worker recommendation-runner candidate-runner nt-backtest-worker-1 nt-backtest-worker-2 paper-scheduler reverse-proxy
  2. up --wait postgres
  3. run --rm --no-deps db-role-bootstrap (exit code is the gate)
  4. run --rm --no-deps db-migrate (exit code is the gate)
  5. run --rm --no-deps research-raw-init (exit code is the gate)
  6. run --rm --no-deps research-schema-check (exit code is the gate)
  7. up --wait --no-deps api-server (the DB/schema gates already passed)
  8. up --wait --no-deps web (HTTP liveness only)
  9. up --no-deps -d research-worker recommendation-runner candidate-runner nt-backtest-worker-1 nt-backtest-worker-2 paper-scheduler (bootstrap; data readiness is intentionally not awaited)
  10. up --wait --no-deps reverse-proxy (edge liveness)
  11. ps; after the approved backfill run post-backfill-health.sh --scope release --check
Live profile is not included. KIS account/order secrets are not required.
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
  compose build --pull=false db-role-bootstrap db-migrate research-worker
  compose up --wait postgres
  compose run --rm --no-deps db-role-bootstrap
  compose run --rm --no-deps db-migrate
  compose run --rm --no-deps research-raw-init
  compose run --rm --no-deps research-schema-check
  compose ps
  echo 'COMPOSE_BACKFILL_BOOTSTRAP: PASS (worker image built; run backfill one-shots, then post-backfill-health.sh --scope backfill --check)'
  exit 0
fi

compose build --pull=false \
  db-role-bootstrap db-migrate api-server web research-worker recommendation-runner candidate-runner \
  nt-backtest-worker-1 nt-backtest-worker-2 paper-scheduler reverse-proxy
compose up --wait postgres
compose run --rm --no-deps db-role-bootstrap
compose run --rm --no-deps db-migrate
compose run --rm --no-deps research-raw-init
compose run --rm --no-deps research-schema-check
compose up --wait --no-deps api-server
compose up --wait --no-deps web
# These services are intentionally detached without --wait. Their functional
# healthchecks require approved EOD/curated data, which is absent on a clean
# database. post-backfill-health.sh is the explicit data-readiness gate.
compose up --no-deps -d research-worker recommendation-runner candidate-runner \
  nt-backtest-worker-1 nt-backtest-worker-2 paper-scheduler
compose up --wait --no-deps reverse-proxy
compose ps
echo 'COMPOSE_RELEASE: PASS (run post-backfill-health.sh --scope release --check to assert data readiness)'
