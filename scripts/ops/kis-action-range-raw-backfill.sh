#!/usr/bin/env bash
# Root-only operator wrapper for the KIS KSD action-range Raw one-shot.
#
# The plan is deliberately local and does not inspect the production env,
# secrets, Docker, or the filesystem data root. Preflight/execute validate the
# installed immutable env and clean source tree before any Compose operation.
# This wrapper never stops or starts the ordinary research worker; the caller
# must arrange that lifecycle separately when an execute is approved.
set -euo pipefail

script_dir=$(cd -P "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
root=$(cd -P "$script_dir/../.." && pwd -P)
source "$script_dir/lib/dotenv.sh"

compose_file=$root/deploy/compose/compose.yml
env_file=${LAGRANGE_ENV_FILE:-$root/deploy/compose/.env}
start_date=
end_date=
scope=etf11
scope_seen=0
mode=plan
mode_seen=0
env_file_seen=0
compose_service=research-action-range-raw
compose_profile=action-range-raw
confirmation=I_UNDERSTAND_READ_ONLY_KIS_ACTION_RANGE_CALLS

usage() {
  cat <<'EOF'
Usage: scripts/ops/kis-action-range-raw-backfill.sh
       --start YYYY-MM-DD --end YYYY-MM-DD
       [--scope etf11|whole-market]
       [--env-file ABSOLUTE_PATH]
       [--plan|--preflight|--execute]

The default --plan prints the fixed KIS KSD action request shape only. It does
not read the production env or secrets, invoke Docker, write data, or use a
network. --preflight is root-only and validates the installed immutable env,
exact source HEAD, clean tree, production read-only range scope, and Compose
config without building or starting a container. --execute additionally
requires KIS_ACTION_RANGE_CONFIRM=I_UNDERSTAND_READ_ONLY_KIS_ACTION_RANGE_CALLS,
builds and provenance-checks the dedicated action image, then runs exactly one
profile-gated no-deps one-shot. It never stops or starts research-worker.
EOF
}

die() {
  printf 'kis-action-range-raw-backfill: %s\n' "$*" >&2
  exit 1
}

blocked() {
  printf 'BLOCKED_EXTERNAL: %s\n' "$*" >&2
  exit 2
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --start)
      [ "$#" -ge 2 ] || die '--start needs YYYY-MM-DD'
      [ -z "$start_date" ] || die '--start must not be repeated'
      start_date=$2
      shift 2
      ;;
    --end)
      [ "$#" -ge 2 ] || die '--end needs YYYY-MM-DD'
      [ -z "$end_date" ] || die '--end must not be repeated'
      end_date=$2
      shift 2
      ;;
    --scope)
      [ "$#" -ge 2 ] || die '--scope needs etf11 or whole-market'
      [ "$scope_seen" -eq 0 ] || die '--scope must not be repeated'
      case "$2" in
        etf11|whole-market) scope=$2 ;;
        *) die '--scope must be etf11 or whole-market' ;;
      esac
      scope_seen=1
      shift 2
      ;;
    --env-file)
      [ "$#" -ge 2 ] || die '--env-file needs an absolute path'
      [ "$env_file_seen" -eq 0 ] || die '--env-file must not be repeated'
      env_file=$2
      env_file_seen=1
      shift 2
      ;;
    --plan|--preflight|--execute)
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

[ -n "$start_date" ] && [ -n "$end_date" ] ||
  die '--start and --end are required'
[[ "$start_date" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || die 'invalid --start date'
[[ "$end_date" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || die 'invalid --end date'
python3 - "$start_date" "$end_date" <<'PY'
import datetime as dt
import sys

try:
    start = dt.date.fromisoformat(sys.argv[1])
    end = dt.date.fromisoformat(sys.argv[2])
except ValueError as exc:
    raise SystemExit(f"invalid calendar date: {exc}")
if end < start:
    raise SystemExit("--end precedes --start")
PY

case "$scope" in
  etf11)
    initial_calls=77
    scope_description='fixed ETF11 (11 symbols x 7 KSD classes)'
    ;;
  whole-market)
    initial_calls=7
    scope_description='whole-market (blank SHT_CD x 7 KSD classes)'
    ;;
  *)
    die 'invalid action-range scope'
    ;;
esac

print_plan() {
  printf 'KIS_ACTION_RANGE_RAW_PLAN mode=plan\n'
  printf '  range=%s..%s scope=%s (%s)\n' "$start_date" "$end_date" "$scope" "$scope_description"
  printf '  initial_requests=%s max_pages_per_class=10\n' "$initial_calls"
  printf '  source=KIS KSD read-only corporate-action classes (7 logical classes)\n'
  printf '  service=%s profile=%s image=commit-tagged-action-range-image\n' "$compose_service" "$compose_profile"
  printf '  raw=write-only /data/raw; no catalog/publication/trading path\n'
  printf 'PLAN_ONLY: no Docker, secret, production-env, data-root write, or network action made\n'
}

# Plan must be usable without access to protected production files. All
# validation and every external command below are intentionally after this
# return point.
if [ "$mode" = plan ]; then
  print_plan
  exit 0
fi

[ "$(id -u)" -eq 0 ] ||
  blocked "--$mode must run as root to inspect installed production paths"

safe_absolute_file() {
  local path=$1 label=$2 probe=$1
  case "$path" in
    /*) ;;
    *) blocked "$label must be an absolute path" ;;
  esac
  case "$path" in
    *$'\n'*|*$'\r'*|*/../*|*/..|../*)
      blocked "$label has an unsafe path shape"
      ;;
  esac
  while [ "$probe" != / ]; do
    [ ! -L "$probe" ] || blocked "$label must not traverse a symlink"
    probe=${probe%/*}
    [ -n "$probe" ] || probe=/
  done
  if [ -e "$path" ]; then
    [ -f "$path" ] && [ ! -L "$path" ] || blocked "$label must be a regular file"
  fi
}

safe_absolute_file "$env_file" env-file
[ -f "$env_file" ] && [ ! -L "$env_file" ] ||
  blocked 'installed immutable env file is missing'
env_metadata=$(stat -c '%u:%g:%a' -- "$env_file") ||
  blocked 'installed immutable env file metadata is unreadable'
[ "$env_metadata" = 0:0:600 ] ||
  blocked 'installed immutable env file must be root:root mode 0600'
[ -f "$compose_file" ] && [ ! -L "$compose_file" ] ||
  die 'Compose file is missing or unsafe'

if ! dotenv_load "$env_file"; then
  echo 'INVALID_CONFIG: production env file is malformed' >&2
  printf '  - %s\n' "${DOTENV_ERRORS[@]}" >&2
  exit 1
fi
if ! dotenv_validate_shell_overrides; then
  echo 'INVALID_CONFIG: shell overrides do not match production env file' >&2
  printf '  - %s\n' "${DOTENV_SHELL_ERRORS[@]}" >&2
  exit 1
fi

commit=$(dotenv_effective_get LAGRANGE_CODE_COMMIT)
[[ "$commit" =~ ^[0-9a-f]{40}$ ]] ||
  blocked 'LAGRANGE_CODE_COMMIT must be exactly 40 lowercase hexadecimal characters'
[ "$commit" != 0000000000000000000000000000000000000000 ] ||
  die 'LAGRANGE_CODE_COMMIT must not be all zeroes'
head=$(git -c "safe.directory=$root" -C "$root" rev-parse --verify 'HEAD^{commit}' 2>/dev/null) ||
  die 'repository HEAD is unavailable'
[ "$head" = "$commit" ] ||
  blocked 'LAGRANGE_CODE_COMMIT does not match repository HEAD'
worktree_status=$(git -c "safe.directory=$root" -C "$root" \
  status --porcelain=v1 --untracked-files=all 2>/dev/null) ||
  die 'cannot inspect build root worktree status'
[ -z "$worktree_status" ] ||
  blocked 'build root worktree must be clean'

validate_production() {
  if LAGRANGE_CODE_COMMIT="$commit" \
      "$root/scripts/ops/validate-production-config.sh" \
      --scope range-raw --env-file "$env_file" >/dev/null 2>&1; then
    return 0
  fi
  # The validator deliberately emits only typed/configuration diagnostics. Do
  # not forward its output here: this wrapper's output must never include an
  # entitlement value or any secret-adjacent environment content.
  return 1
}

validate_production || blocked 'production read-only range configuration is not ready'

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

compose config --quiet || die 'Compose interpolation/config validation failed'

if [ "$mode" = preflight ]; then
  echo 'KIS_ACTION_RANGE_RAW_PREFLIGHT: PASS (no build, KIS call, or container lifecycle)'
  exit 0
fi

[ "${KIS_ACTION_RANGE_CONFIRM:-}" = "$confirmation" ] ||
  blocked 'set KIS_ACTION_RANGE_CONFIRM=I_UNDERSTAND_READ_ONLY_KIS_ACTION_RANGE_CALLS for execute'

running_services=$(compose ps --status running --services 2>/dev/null) ||
  blocked 'cannot inspect running Compose services'
if grep -Fxq research-worker <<<"$running_services"; then
  blocked 'ordinary research-worker daemon is running; stop it through the separate operator protection workflow'
fi
if grep -Fxq "$compose_service" <<<"$running_services"; then
  blocked 'another research-action-range-raw one-shot is already running'
fi

compose build --pull=false "$compose_service" ||
  die 'action-range Raw image build failed'

image="lagrange-station-research-action-range-raw:$commit"
docker image inspect "$image" >/dev/null 2>&1 ||
  die 'research-action-range-raw image was not produced for the requested commit'
revision=$(docker image inspect --format '{{index .Config.Labels "org.opencontainers.image.revision"}}' \
  "$image" 2>/dev/null) || die 'cannot inspect action image OCI revision'
[ "$revision" = "$commit" ] ||
  die 'research-action-range-raw image OCI revision does not match LAGRANGE_CODE_COMMIT'
image_commit=$(docker image inspect --format '{{range .Config.Env}}{{println .}}{{end}}' \
  "$image" 2>/dev/null | awk -F= '$1 == "LAGRANGE_CODE_COMMIT" { print substr($0, index($0, "=") + 1); exit }')
[ "$image_commit" = "$commit" ] ||
  die 'research-action-range-raw image ENV LAGRANGE_CODE_COMMIT does not match the requested commit'

# The acknowledgement is passed as a Compose run override rather than stored
# in the production env file. It is not a credential and is exact by design.
compose run --rm --no-deps -e "KIS_ACTION_RANGE_CONFIRM=$confirmation" "$compose_service" \
  --start "$start_date" --end "$end_date" --scope "$scope" --execute ||
  die 'action-range Raw one-shot failed'
echo 'KIS_ACTION_RANGE_RAW: PASS (Raw-only action capture completed; no publication/catalog action)'
