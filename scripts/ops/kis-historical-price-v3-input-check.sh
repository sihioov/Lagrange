#!/usr/bin/env bash
# Root-side installed-release wrapper for the provider-free combined V3 KIS
# input checker.  It owns only the immutable release/env/Compose boundary; the
# binary owns Raw verification.  No ordinary worker is stopped or started.
set -euo pipefail

script_dir=$(cd -P "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
root=$(cd -P "$script_dir/../.." && pwd -P)
source "$script_dir/lib/dotenv.sh"

compose_file=$root/deploy/compose/compose.yml
env_file=${LAGRANGE_ENV_FILE:-$root/deploy/compose/.env}
compose_service=research-v3-input-check
compose_profile=v3-input-check
mode=plan
mode_seen=0
env_file_seen=0

usage() {
  cat <<'EOF'
Usage: scripts/ops/kis-historical-price-v3-input-check.sh
       [--env-file ABSOLUTE_PATH]
       [--plan|--preflight|--check]

The default --plan is provider-free and does not read the installed env,
repository, Docker, or Raw. --preflight validates the installed immutable
env (absolute, non-symlink, root:root 0600), exact repository HEAD/clean tree,
production range-raw configuration, and Compose interpolation without a build
or container. --check builds the exact commit-tagged combined price/action
checker image, verifies its OCI revision and LAGRANGE_CODE_COMMIT image ENV,
then runs one network_mode:none, --no-deps Raw-read-only container. It never
manages the ordinary research worker and never calls KIS/provider/network APIs.
EOF
}

die() {
  printf 'kis-historical-price-v3-input-check: %s\n' "$*" >&2
  exit 1
}

blocked() {
  printf 'BLOCKED_EXTERNAL: %s\n' "$*" >&2
  exit 2
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --env-file)
      [ "$#" -ge 2 ] || die '--env-file needs an absolute path'
      [ "$env_file_seen" -eq 0 ] || die '--env-file must not be repeated'
      env_file=$2
      env_file_seen=1
      shift 2
      ;;
    --plan|--preflight|--check)
      [ "$mode_seen" -eq 0 ] || die 'choose exactly one mode'
      mode=${1#--}
      mode_seen=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown option: $1"
      ;;
  esac
done

print_plan() {
  printf 'KIS_HISTORICAL_PRICE_V3_INPUT_CHECK_PLAN mode=plan\n'
  printf '  service=%s profile=%s price_batch_id=d746ef9f-7eed-5333-97db-cb064331bd06 price_file_count=275 action_batch_id=fbec8b5d-d87a-4d62-86fa-7af8ebce982b action_file_count=77\n' \
    "$compose_service" "$compose_profile"
  printf '  source=committed KIS daily-range price and KIS action Raw; both batch.json and manifest lines are independently hashed\n'
  printf '  network=none raw_mount=read-only no_deps=true ordinary_worker=untouched\n'
  printf 'PLAN_ONLY: no installed env, Docker, Raw, KIS, provider, or network action made\n'
}

# Plan must remain usable on an unprovisioned host and must not even inspect
# the protected env file. Every validation and external command is below.
if [ "$mode" = plan ]; then
  print_plan
  exit 0
fi

[ "$(id -u)" -eq 0 ] ||
  blocked "--$mode must run as root to inspect installed production paths"

safe_absolute_file() {
  local path=$1 label=$2 probe
  case "$path" in
    /*) ;;
    *) blocked "$label must be an absolute path" ;;
  esac
  case "$path" in
    /) ;;
    */) blocked "$label must not have a trailing slash" ;;
  esac
  case "$path" in
    *$'\n'*|*$'\r'*|*//*|*/../*|*/..|../*|*/./*|*/.)
      blocked "$label has an unsafe path shape"
      ;;
  esac
  probe=$path
  while [ "$probe" != / ]; do
    [ ! -L "$probe" ] || blocked "$label must not traverse a symlink"
    probe=${probe%/*}
    [ -n "$probe" ] || probe=/
  done
}

safe_absolute_file "$env_file" env-file
[ -f "$env_file" ] && [ ! -L "$env_file" ] ||
  blocked 'installed immutable env file is missing or unsafe'
[ "$(stat -c '%u:%g:%a' -- "$env_file" 2>/dev/null)" = '0:0:600' ] ||
  blocked 'installed immutable env file must be root:root mode 0600'
safe_absolute_file "$compose_file" compose-file
[ -f "$compose_file" ] && [ ! -L "$compose_file" ] || die 'Compose file is missing or unsafe'

if ! dotenv_load "$env_file"; then
  blocked 'installed immutable env file is malformed'
fi
if ! dotenv_validate_shell_overrides; then
  blocked 'shell overrides do not exactly match the installed immutable env'
fi

commit=$(dotenv_effective_get LAGRANGE_CODE_COMMIT)
[[ "$commit" =~ ^[0-9a-f]{40}$ ]] ||
  blocked 'LAGRANGE_CODE_COMMIT must be exactly 40 lowercase hexadecimal characters'
[ "$commit" != 0000000000000000000000000000000000000000 ] ||
  die 'LAGRANGE_CODE_COMMIT must not be all zeroes'
head=$(git -c "safe.directory=$root" -C "$root" rev-parse --verify 'HEAD^{commit}' 2>/dev/null) ||
  die 'repository HEAD is unavailable'
[ "$head" = "$commit" ] || blocked 'LAGRANGE_CODE_COMMIT does not match repository HEAD'
worktree_status=$(git -c "safe.directory=$root" -C "$root" \
  status --porcelain=v1 --untracked-files=all 2>/dev/null) ||
  die 'cannot inspect build root worktree status'
[ -z "$worktree_status" ] || blocked 'build root worktree must be clean'

# Reuse the production range-raw gate: it proves the installed data/Raw,
# entitlement, KIS read-only source copies, and production code identity are
# configured. This checker itself receives none of those credentials.
if ! LAGRANGE_CODE_COMMIT="$commit" \
    "$root/scripts/ops/validate-production-config.sh" \
    --scope range-raw --env-file "$env_file" >/dev/null 2>&1; then
  blocked 'production read-only range configuration is not ready'
fi

command -v docker >/dev/null 2>&1 || blocked 'docker is not installed'
docker compose version >/dev/null 2>&1 || blocked 'Docker Compose v2 is unavailable'

compose() {
  LAGRANGE_CODE_COMMIT="$commit" \
    RANGE_RAW_BATCH_ID=compose-config-disabled \
    BACKTEST_MIN_FREE_BYTES=0 \
    BACKTEST_MAX_QUEUED_BACKTESTS=0 \
    BACKTEST_RECONCILE_GRACE_SECS=0 \
    BACKTEST_RECONCILE_INTERVAL_SECS=0 \
    docker compose --profile "$compose_profile" --env-file "$env_file" \
    --file "$compose_file" "$@"
}

compose config --quiet >/dev/null 2>&1 || die 'Compose interpolation/config validation failed'

if [ "$mode" = preflight ]; then
  echo 'KIS_HISTORICAL_PRICE_V3_INPUT_CHECK_PREFLIGHT: PASS (no build, container, KIS call, or Raw write)'
  exit 0
fi

image="lagrange-station-research-v3-input-check:$commit"
compose build --pull=false "$compose_service" >/dev/null 2>&1 ||
  die 'V3 input checker image build failed'
docker image inspect "$image" >/dev/null 2>&1 ||
  die 'V3 input checker image was not produced for the requested commit'
revision=$(docker image inspect --format '{{index .Config.Labels "org.opencontainers.image.revision"}}' \
  "$image" 2>/dev/null) || die 'cannot inspect V3 input checker OCI revision'
[ "$revision" = "$commit" ] ||
  die 'V3 input checker OCI revision does not match LAGRANGE_CODE_COMMIT'
image_commit=$(docker image inspect --format '{{range .Config.Env}}{{println .}}{{end}}' \
  "$image" 2>/dev/null | awk -F= '$1 == "LAGRANGE_CODE_COMMIT" { print substr($0, index($0, "=") + 1); exit }')
[ "$image_commit" = "$commit" ] ||
  die 'V3 input checker image ENV LAGRANGE_CODE_COMMIT does not match the requested commit'

# The Compose service fixes --raw-root /data, mounts only host Raw at
# /data/raw:ro, and sets network_mode:none. No ordinary worker lifecycle is
# inspected or changed here.
compose run --rm --no-deps "$compose_service" >/dev/null 2>&1 ||
  die 'V3 input checker one-shot failed'
echo 'KIS_HISTORICAL_PRICE_V3_INPUT_CHECK: PASS (committed price and action Raw verified; no KIS/provider/network call)'
