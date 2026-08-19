#!/usr/bin/env bash
# Explicit post-backfill readiness gate for the read-only KIS release.
# Default is a no-infrastructure plan. --check performs no writes or provider
# calls; it runs the existing research-worker healthcheck as a dependency-free
# one-shot and requires a fresh published EOD row. The default backfill
# scope works before serving-only Auth0/TLS/dataset approval; release scope
# rechecks the complete serving configuration.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$script_dir/../.." && pwd)
compose_file=${LAGRANGE_COMPOSE_FILE:-$root/deploy/compose/compose.yml}
env_file=${LAGRANGE_ENV_FILE:-$root/deploy/compose/.env}
mode=plan
scope=backfill

usage() {
  cat <<'EOF'
Usage: scripts/ops/post-backfill-health.sh [--scope backfill|release]
       [--plan|--check]

  --plan   Print the readiness gate without Docker, DB, or file writes (default).
  --check  Validate production config, then execute the existing
           research-worker healthcheck as a dependency-free one-shot. The
           check requires a fresh credentialed KIS EOD publication; it never
           calls a KIS endpoint and does not require a worker daemon.
  --scope backfill  Check the worker before serving approval (default).
  --scope release    Also require the full serving configuration and pins.
EOF
}

die() { echo "post-backfill-health: $*" >&2; exit 1; }
blocked() { echo "BLOCKED_EXTERNAL: $*" >&2; exit 2; }
while [ "$#" -gt 0 ]; do
  case "$1" in
    --scope)
      [ "$#" -ge 2 ] || die '--scope needs backfill or release'
      scope=$2
      shift 2
      ;;
    --plan) mode=plan; shift ;;
    --check) mode=check; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done

case "$scope" in
  backfill|release) ;;
  *) die '--scope must be backfill or release' ;;
esac

[ -f "$compose_file" ] || die "Compose file missing: $compose_file"

echo "POST_BACKFILL_HEALTH_GATE: scope=$scope"
cat <<'EOF'
  1. validate production config and runtime secret shape
  2. run `research-worker healthcheck` with no Compose dependencies
  3. require credentialed KRX/KR EOD freshness from the published DB row
Raw/Curated bytes, dataset pins, and migration/schema contracts are checked by
their own startup/publication gates; this command does not claim those checks
are part of the worker healthcheck.
EOF

if [ "$mode" = plan ]; then
  echo 'PLAN_ONLY: no Docker, DB, provider, or file operation made'
  exit 0
fi

[ -f "$env_file" ] || blocked "production env file missing: $env_file"
command -v docker >/dev/null 2>&1 || blocked 'docker is not installed'
docker compose version >/dev/null 2>&1 || blocked 'Docker Compose v2 is unavailable'
bash "$script_dir/validate-production-config.sh" --scope "$scope" --env-file "$env_file"

compose() {
  RANGE_RAW_BATCH_ID=compose-config-disabled \
    docker compose --env-file "$env_file" -f "$compose_file" "$@"
}

compose config --quiet || die 'Compose interpolation/config validation failed'

# `run --no-deps` deliberately invokes the exact binary used by the service
# healthcheck while avoiding a daemon precondition, dependency restart, or a
# second KIS ingestion. The one-shot healthcheck subcommand is read-only.
compose run --rm --no-deps research-worker healthcheck ||
  die 'research-worker healthcheck failed; EOD freshness/readiness is not established'

echo "POST_BACKFILL_HEALTH: PASS (scope=$scope; credentialed EOD freshness gate passed)"
