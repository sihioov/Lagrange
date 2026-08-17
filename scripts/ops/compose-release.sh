#!/usr/bin/env bash
# Ordered production Compose workflow. Default is a read-only plan; --apply
# is the explicit operator action after production-config validation succeeds.
# No command here generates secrets, enables live trading, or calls KIS itself.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$script_dir/../.." && pwd)
compose_file=${LAGRANGE_COMPOSE_FILE:-$root/deploy/compose/compose.yml}
env_file=${LAGRANGE_ENV_FILE:-$root/deploy/compose/.env}
mode=plan

usage() {
  cat <<'EOF'
Usage: scripts/ops/compose-release.sh [--plan|--preflight|--apply]

  --plan       Validate static inputs and print the ordered commands (default).
  --preflight  Validate inputs and Compose expansion without starting services.
  --apply      Build and start the production stack in dependency order.

The apply order is: host/runtime preflight, Compose config, image builds,
PostgreSQL, role bootstrap, migrations, raw ownership, schema check, then
API/Web/workers/reverse-proxy. One-shot failures stop the release.
EOF
}

die() { echo "compose-release: $*" >&2; exit 1; }
blocked() { echo "BLOCKED_EXTERNAL: $*" >&2; exit 2; }
while [ "$#" -gt 0 ]; do
  case "$1" in
    --plan) mode=plan; shift ;;
    --preflight) mode=preflight; shift ;;
    --apply) mode=apply; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done

[ -f "$compose_file" ] || die "Compose file missing: $compose_file"
[ -f "$env_file" ] || blocked "production env file missing: $env_file"
command -v docker >/dev/null 2>&1 || blocked 'docker is not installed'
docker compose version >/dev/null 2>&1 || blocked 'Docker Compose v2 is unavailable'

bash "$script_dir/validate-production-config.sh" --env-file "$env_file"

if [ "$mode" != plan ]; then
  # Host preparation is deliberately a separate explicit operator action; the
  # release workflow only verifies it and never silently runs root provisioning.
  bash "$script_dir/provision-linux.sh" --preflight
fi

compose() {
  docker compose --env-file "$env_file" -f "$compose_file" "$@"
}

compose config --quiet || die 'Compose interpolation/config validation failed'

cat <<'EOF'
COMPOSE_RELEASE_ORDER:
  1. build --pull=false api-server web research-worker recommendation-runner candidate-runner nt-backtest-worker-1 nt-backtest-worker-2 paper-scheduler reverse-proxy
  2. up --wait postgres
  3. run --rm --no-deps db-role-bootstrap (exit code is the gate)
  4. run --rm --no-deps db-migrate (exit code is the gate)
  5. run --rm --no-deps research-raw-init (exit code is the gate)
  6. run --rm --no-deps research-schema-check (exit code is the gate)
  7. up --wait api-server web research-worker recommendation-runner candidate-runner nt-backtest-worker-1 nt-backtest-worker-2 paper-scheduler reverse-proxy
  8. ps and healthcheck inspection; restart only after diagnosing a failed health gate
Live profile is not included. KIS account/order secrets are not required.
EOF

if [ "$mode" = plan ]; then
  echo 'PLAN_ONLY: no build, migration, service start, or network call made'
  exit 0
fi
if [ "$mode" = preflight ]; then
  echo 'PREFLIGHT: PASS'
  exit 0
fi

compose build --pull=false \
  api-server web research-worker recommendation-runner candidate-runner \
  nt-backtest-worker-1 nt-backtest-worker-2 paper-scheduler reverse-proxy
compose up --wait postgres
compose run --rm --no-deps db-role-bootstrap
compose run --rm --no-deps db-migrate
compose run --rm --no-deps research-raw-init
compose run --rm --no-deps research-schema-check
compose up --wait api-server web research-worker recommendation-runner \
  candidate-runner nt-backtest-worker-1 nt-backtest-worker-2 paper-scheduler reverse-proxy
compose ps
echo 'COMPOSE_RELEASE: PASS'
