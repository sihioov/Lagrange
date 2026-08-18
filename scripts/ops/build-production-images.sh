#!/usr/bin/env bash
# Prebuild the production service images without starting or running any
# container. The default is a read-only plan; --preflight validates Docker
# Compose expansion; --apply is the only mode that performs a build.
#
# This helper deliberately does not invoke the production validator: image
# prebuild does not need KIS credentials, provider entitlement, dataset pins,
# database state, or runtime secret files. Compose receives only process-local
# fail-closed interpolation sentinels; deploy/compose/.env is never written.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$script_dir/../.." && pwd)
compose_file=$root/deploy/compose/compose.yml
env_file=$root/deploy/compose/.env
mode=plan
mode_seen=0

build_services=(
  db-role-bootstrap
  db-migrate
  api-server
  web
  research-worker
  recommendation-runner
  candidate-runner
  nt-backtest-worker-1
  nt-backtest-worker-2
  paper-scheduler
  reverse-proxy
)

usage() {
  cat <<'EOF'
Usage: scripts/ops/build-production-images.sh [--plan|--preflight|--apply]
       [--compose-file ABSOLUTE_PATH] [--env-file ABSOLUTE_PATH]

Modes:
  --plan       Validate local paths and print the exact build plan (default).
  --preflight  Read-only Docker/Compose availability and config check (the
               host may require root for the Docker socket); it performs no
               build or container lifecycle action.
  --apply      Root-only Compose image build with --pull=false. It never runs
               up, run, restart, start, migration, database, or provider work.

LAGRANGE_CODE_COMMIT must already be present in the process environment as the
exact 40-character lowercase Git commit. It is passed process-locally to
Compose and must exactly match the clean build context HEAD. This helper never
derives it from a mutable checkout and never edits the env file. A build may
fetch language/base-image dependencies when the Docker cache is incomplete; no
provider/API credential is used by this helper.
EOF
}

die() { echo "build-production-images: $*" >&2; exit 1; }

while [ "$#" -gt 0 ]; do
  case "$1" in
    --plan)
      [ "$mode_seen" -eq 0 ] || die 'choose exactly one mode: --plan, --preflight, or --apply'
      mode=plan
      mode_seen=1
      shift
      ;;
    --preflight)
      [ "$mode_seen" -eq 0 ] || die 'choose exactly one mode: --plan, --preflight, or --apply'
      mode=preflight
      mode_seen=1
      shift
      ;;
    --apply)
      [ "$mode_seen" -eq 0 ] || die 'choose exactly one mode: --plan, --preflight, or --apply'
      mode=apply
      mode_seen=1
      shift
      ;;
    --compose-file|--env-file)
      [ "$#" -ge 2 ] || die "$1 needs an absolute path"
      case "$1" in
        --compose-file) compose_file=$2 ;;
        --env-file) env_file=$2 ;;
      esac
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *) die 'unknown option (use --help)' ;;
  esac
done

if [ "$mode" = apply ] && [ "$(id -u)" -ne 0 ]; then
  die '--apply must run as root; use --plan or --preflight for read-only checks'
fi

safe_path() {
  local path=$1 label=$2 probe
  [ -n "$path" ] || die "$label must not be empty"
  case "$path" in
    /*) ;;
    *) die "$label must be absolute: $path" ;;
  esac
  case "$path" in
    */../*|*/..) die "$label must not contain '..': $path" ;;
  esac
  case "$path" in
    /|/etc|/opt|/usr|/usr/local|/var|/var/lib|/tmp|/run)
      die "$label is too broad: $path"
      ;;
  esac
  probe=${path%/}
  [ -n "$probe" ] || probe=/
  while [ "$probe" != / ]; do
    [ ! -L "$probe" ] || die "$label must not traverse a symlink: $probe"
    probe=${probe%/*}
    [ -n "$probe" ] || probe=/
  done
}

check_inputs() {
  local commit=${LAGRANGE_CODE_COMMIT:-} head status
  safe_path "$compose_file" compose-file
  safe_path "$env_file" env-file
  [ -f "$compose_file" ] && [ ! -L "$compose_file" ] ||
    die 'Compose file must be a regular non-symlink file'
  [ -r "$compose_file" ] || die 'Compose file is not readable'
  [ -f "$env_file" ] && [ ! -L "$env_file" ] ||
    die 'Compose env file must be a regular non-symlink file'
  [ -r "$env_file" ] || die 'Compose env file is not readable'
  [[ "$commit" =~ ^[0-9a-f]{40}$ ]] ||
    die 'LAGRANGE_CODE_COMMIT must be exactly 40 lowercase hexadecimal characters'
  [ "$commit" != 0000000000000000000000000000000000000000 ] ||
    die 'LAGRANGE_CODE_COMMIT must not be all zeroes'
  command -v git >/dev/null 2>&1 || die 'git is required to verify build provenance'
  head=$(git -c "safe.directory=$root" -C "$root" \
    rev-parse --verify 'HEAD^{commit}' 2>/dev/null) ||
    die 'build root is not a Git worktree with a commit'
  [ "$head" = "$commit" ] ||
    die 'LAGRANGE_CODE_COMMIT does not match the build root HEAD'
  status=$(git -c "safe.directory=$root" -C "$root" \
    status --porcelain=v1 --untracked-files=all 2>/dev/null) ||
    die 'cannot inspect build root worktree status'
  [ -z "$status" ] || die 'build root worktree is not clean (tracked or untracked changes present)'
}

print_plan() {
  echo 'PRODUCTION_IMAGE_BUILD_PLAN mode=plan'
  echo "  compose_file=$compose_file"
  echo "  env_file=$env_file"
  echo "  build_root=$root (clean HEAD provenance verified)"
  echo '  LAGRANGE_CODE_COMMIT=validated-process-value'
  echo "  services=${build_services[*]}"
  echo '  command: docker compose --env-file <env> --file <compose> build --pull=false <services...>'
  echo '  no up/run/restart/start, migration, database, provider/API, or secret provisioning action'
  echo '  network caveat: Docker may fetch base/language dependencies when its cache is incomplete'
  echo 'PLAN_ONLY: no Docker command or env-file write made'
}

compose() {
  # Required Compose interpolation values are intentionally inert and
  # process-local. They are sufficient for config expansion only and are never
  # written to .env or passed to a container lifecycle command.
  LAGRANGE_CODE_COMMIT="$LAGRANGE_CODE_COMMIT" \
  RESEARCH_APP_ENV=prebuild-disabled \
  RESEARCH_ENTITLEMENT_REFERENCE=prebuild-disabled \
  BACKTEST_MIN_FREE_BYTES=0 \
  BACKTEST_MAX_QUEUED_BACKTESTS=0 \
  BACKTEST_RECONCILE_GRACE_SECS=0 \
  BACKTEST_RECONCILE_INTERVAL_SECS=0 \
  COMPOSE_PROFILES= \
  LIVE_NODE_MODE=disabled \
  LIVE_NODE_DRY_RUN=1 \
    docker compose --env-file "$env_file" --file "$compose_file" "$@"
}

check_inputs

if [ "$mode" = plan ]; then
  print_plan
  exit 0
fi

command -v docker >/dev/null 2>&1 || die 'docker is not installed'
docker compose version >/dev/null 2>&1 || die 'Docker Compose v2 is unavailable'
compose config --quiet || die 'Compose interpolation/config validation failed'

if [ "$mode" = preflight ]; then
  echo 'PRODUCTION_IMAGE_BUILD_PREFLIGHT: PASS (no build or container lifecycle action)'
  exit 0
fi

compose build --pull=false "${build_services[@]}"
echo 'PRODUCTION_IMAGE_BUILD: PASS (images built; no container lifecycle action requested)'
