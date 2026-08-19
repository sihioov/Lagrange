#!/usr/bin/env bash
# Stage5 Raw-only historical daily-bars range runner.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$script_dir/../.." && pwd)
source "$script_dir/lib/dotenv.sh"

env_file=${LAGRANGE_ENV_FILE:-$root/deploy/compose/.env}
state_file=${LAGRANGE_RANGE_RAW_STATE:-/var/lib/lagrange/state/range-raw/state.tsv}
start_date=
end_date=
mode=plan
mode_seen=0

usage() {
  cat <<'EOF'
Usage: scripts/ops/kis-range-raw-backfill.sh --start YYYY-MM-DD --end YYYY-MM-DD [--plan|--preflight|--execute]

Stage5 captures only the fixed 11-ETF historical daily-bars range. It uses
the dedicated research-range-raw Compose one-shot and stores isolated Raw
scopes. It does not publish, curate, open a DB, or claim strict PIT.

--plan (default) performs no Docker, KIS, secret read, file write, or state
write. --preflight validates configuration and Compose expansion. --execute
requires KIS_RANGE_RAW_CONFIRM=I_UNDERSTAND_READ_ONLY_DAILY_RANGE_KIS_CALLS.

Resume uses the immutable Raw manifest: an exact deterministic source batch is
reused without another KIS request. State contains only a hashed identity,
status, and UUIDv5 source batch identity.
EOF
}

die() { echo "kis-range-raw-backfill: $*" >&2; exit 1; }
blocked() { echo "BLOCKED_EXTERNAL: $*" >&2; exit 2; }

while [ "$#" -gt 0 ]; do
  case "$1" in
    --start) [ "$#" -ge 2 ] || die '--start needs YYYY-MM-DD'; start_date=$2; shift 2 ;;
    --end) [ "$#" -ge 2 ] || die '--end needs YYYY-MM-DD'; end_date=$2; shift 2 ;;
    --env-file) [ "$#" -ge 2 ] || die '--env-file needs an absolute path'; env_file=$2; shift 2 ;;
    --state-file) [ "$#" -ge 2 ] || die '--state-file needs an absolute path'; state_file=$2; shift 2 ;;
    --plan|--preflight|--execute)
      [ "$mode_seen" -eq 0 ] || die 'choose exactly one mode'
      mode=${1#--}; mode_seen=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done

[ -n "$start_date" ] && [ -n "$end_date" ] || die '--start and --end are required'
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

safe_absolute_file() {
  local path=$1 label=$2 probe=$1
  case "$path" in
    /*) ;;
    *) die "$label must be absolute: $path" ;;
  esac
  case "$path" in
    */../*|*/..) die "$label must not contain '..': $path" ;;
  esac
  while [ "$probe" != / ]; do
    [ ! -L "$probe" ] || die "$label must not traverse a symlink: $probe"
    probe=${probe%/*}
    [ -n "$probe" ] || probe=/
  done
  if [ -e "$path" ]; then
    [ -f "$path" ] && [ ! -L "$path" ] || die "$label must be regular non-symlink file: $path"
  fi
}

safe_absolute_file "$env_file" env-file
safe_absolute_file "$state_file" state-file
[ -f "$env_file" ] && [ ! -L "$env_file" ] || blocked "production env file missing: $env_file"
dotenv_load "$env_file" || {
  echo 'INVALID_CONFIG: production env file is malformed' >&2
  printf '  - %s\n' "${DOTENV_ERRORS[@]}" >&2
  exit 1
}
dotenv_validate_shell_overrides || {
  echo 'INVALID_CONFIG: shell overrides do not match production env file' >&2
  printf '  - %s\n' "${DOTENV_SHELL_ERRORS[@]}" >&2
  exit 1
}

commit=${LAGRANGE_CODE_COMMIT:-}
file_commit=$(dotenv_get LAGRANGE_CODE_COMMIT)
[ -n "$commit" ] || commit=$file_commit
[[ "$commit" =~ ^[0-9a-f]{40}$ ]] || blocked 'LAGRANGE_CODE_COMMIT must be exactly 40 lowercase hexadecimal characters'
[ "$commit" != 0000000000000000000000000000000000000000 ] || die 'LAGRANGE_CODE_COMMIT must not be all zeroes'
[ -z "$file_commit" ] || [ "$file_commit" = "$commit" ] || die 'LAGRANGE_CODE_COMMIT differs from env-file value'
head=$(git -c "safe.directory=$root" -C "$root" rev-parse --verify 'HEAD^{commit}' 2>/dev/null) || die 'repository HEAD is unavailable'
[ "$head" = "$commit" ] || die 'LAGRANGE_CODE_COMMIT does not match repository HEAD'
if [ "$mode" != plan ]; then
  worktree_status=$(git -c "safe.directory=$root" -C "$root" \
    status --porcelain=v1 --untracked-files=all 2>/dev/null) ||
    die 'cannot inspect build root worktree status'
  while IFS= read -r status_line; do
    [ -z "$status_line" ] && continue
    [ "$status_line" = '?? docs/kis_openapi_entiredocs_20260818_030007.xlsx' ] ||
      blocked 'Stage5 requires a clean tracked worktree; only the official KIS workbook may remain untracked'
  done <<<"$worktree_status"
fi

identity_material="stage5
range=$start_date..$end_date
universe=kr-etf-core-v1
source_scope=kis-daily-range
normalized_scope=kis-daily-range-normalized
normalizer=kis-daily-range-to-session-bars-v2
code=$commit
entitlement=$(dotenv_get RESEARCH_ENTITLEMENT_REFERENCE)
"
identity=$(printf '%s' "$identity_material" | sha256sum | awk '{print $1}')
stored_batch_id=$(python3 - "$identity" <<'PY'
import sys
import uuid

NAMESPACE = uuid.UUID("7fb4e3e8-5e85-5a4e-9d3b-5c8d14a3e2b1")
print(uuid.uuid5(NAMESPACE, sys.argv[1]))
PY
)

# Validate the scheduler-only approved session artifact before any mode can
# proceed. This is a local read-only check; it never imports exchange_calendars
# or calls KIS. The worker repeats the same pinned validation at execution.
python3 "$root/scripts/ops/xkrx-calendar-bootstrap.py" \
  --emit-sessions --start "$start_date" --end "$end_date" \
  --output-dir "$root/data/calendars/xkrx" >/dev/null 2>&1 ||
  blocked 'requested range is outside the approved XKRX session artifact'

print_plan() {
  echo 'KIS_RANGE_RAW_PLAN mode=plan'
  echo "  range=$start_date..$end_date universe=kr-etf-core-v1 symbols=11"
  echo '  source_scope=kis-daily-range normalized_scope=kis-daily-range-normalized'
  echo '  endpoint=/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice tr_id=FHKST03010100'
  echo '  requests=one process-owned TokenManager; normally one OAuth token POST within its lifetime + daily-itemchartprice GET windows; no tr_cont continuation'
  echo '  policy=current vendor snapshot acquired_at; strict PIT/READY/publication/Curated/DB unsupported'
  echo "  code_commit=$commit source_batch_id=$stored_batch_id state=$state_file identity_sha256=$identity"
  echo '  execute=KIS_RANGE_RAW_CONFIRM=I_UNDERSTAND_READ_ONLY_DAILY_RANGE_KIS_CALLS ... --execute'
  echo 'PLAN_ONLY: no Docker, KIS, secret read, file write, or state write made'
}

[ "$mode" = plan ] && { print_plan; exit 0; }
[ "$(id -u)" -eq 0 ] || blocked "--$mode must run as root for protected runtime/state files"

validate_production() {
  LAGRANGE_CODE_COMMIT="$commit" "$root/scripts/ops/validate-production-config.sh" \
    --scope range-raw --env-file "$env_file"
}

compose() {
  LAGRANGE_CODE_COMMIT="$commit" \
    BACKTEST_MIN_FREE_BYTES=0 \
    BACKTEST_MAX_QUEUED_BACKTESTS=0 \
    BACKTEST_RECONCILE_GRACE_SECS=0 \
    BACKTEST_RECONCILE_INTERVAL_SECS=0 \
    RANGE_RAW_BATCH_ID="$stored_batch_id" \
    docker compose --profile range-raw \
    --env-file "$env_file" -f "$root/deploy/compose/compose.yml" "$@"
}

verify_image_provenance() {
  local image revision image_commit
  image="lagrange-station-research-range-raw:${commit}"
  docker image inspect "$image" >/dev/null 2>&1 ||
    die "research-range-raw image was not produced: $image"
  revision=$(docker image inspect --format '{{index .Config.Labels "org.opencontainers.image.revision"}}' "$image") ||
    die 'cannot inspect research-range-raw OCI revision'
  [ "$revision" = "$commit" ] ||
    die 'research-range-raw OCI revision does not match LAGRANGE_CODE_COMMIT'
  image_commit=$(docker image inspect --format '{{range .Config.Env}}{{println .}}{{end}}' "$image" |
    awk -F= '$1 == "LAGRANGE_CODE_COMMIT" { print substr($0, index($0, "=") + 1); exit }')
  [ "$image_commit" = "$commit" ] ||
    die 'research-range-raw image ENV LAGRANGE_CODE_COMMIT does not match the requested commit'
}

validate_production
command -v docker >/dev/null 2>&1 || blocked 'docker is not installed'
docker compose version >/dev/null 2>&1 || blocked 'Docker Compose v2 is unavailable'
compose config --quiet || die 'Compose interpolation/config validation failed'

if [ "$mode" = preflight ]; then
  echo 'KIS_RANGE_RAW_PREFLIGHT: PASS (no build, KIS call, or container lifecycle)'
  exit 0
fi

[ "${KIS_RANGE_RAW_CONFIRM:-}" = I_UNDERSTAND_READ_ONLY_DAILY_RANGE_KIS_CALLS ] ||
  blocked 'set KIS_RANGE_RAW_CONFIRM=I_UNDERSTAND_READ_ONLY_DAILY_RANGE_KIS_CALLS for execute'

running_services=$(compose ps --status running --services 2>/dev/null || true)
printf '%s\n' "$running_services" | grep -qx 'research-worker' &&
  blocked 'ordinary research-worker daemon is running; stop it before the isolated range one-shot' || true
printf '%s\n' "$running_services" | grep -qx 'research-range-raw' &&
  blocked 'another research-range-raw one-shot is already running' || true

state_dir=${state_file%/*}
[ "$state_dir" != "$state_file" ] || die 'state-file must include a protected parent directory'

validate_directory_ancestors() {
  local path=$1 label=$2 probe=$1
  case "$path" in
    /*) ;;
    *) blocked "$label must be absolute: $path" ;;
  esac
  case "$path" in
    */../*|*/..) blocked "$label must not contain '..': $path" ;;
  esac
  while [ "$probe" != / ]; do
    [ ! -L "$probe" ] || blocked "$label must not traverse a symlink: $probe"
    if [ -e "$probe" ]; then
      [ -d "$probe" ] || blocked "$label ancestor must be a directory: $probe"
    fi
    probe=${probe%/*}
    [ -n "$probe" ] || probe=/
  done
}

validate_trusted_state_ancestors() {
  local path=$1 probe=$1 shape mode uid gid
  # The production state tree is root-owned and not writable by a service
  # account. /tmp is allowed only for isolated self-tests; its sticky parent
  # is still followed by the no-symlink/regular-directory checks above.
  while [ "$probe" != / ]; do
    case "$probe" in
      /tmp|/var/lib) break ;;
    esac
    if [ -e "$probe" ]; then
      [ -d "$probe" ] || blocked "range raw state ancestor must be a directory: $probe"
      shape=$(stat -Lc '%u:%g:%a' -- "$probe") ||
        blocked "cannot inspect range raw state ancestor: $probe"
      IFS=: read -r uid gid mode <<<"$shape"
      [ "$uid" = 0 ] && [ "$gid" = 0 ] ||
        blocked "range raw state ancestor must be root:root: $probe"
      if (( 8#$mode & 0022 )); then
        blocked "range raw state ancestor must not be group/other writable: $probe"
      fi
    fi
    probe=${probe%/*}
    [ -n "$probe" ] || probe=/
  done
}

ensure_state_directory() {
  local path=$1 probe=$1
  case "$path" in
    /|/*/../*|*/..) blocked 'range raw state directory path is unsafe' ;;
  esac
  local -a missing=()
  while [ "$probe" != / ]; do
    [ ! -L "$probe" ] || blocked "range raw state directory must not traverse a symlink: $probe"
    if [ -e "$probe" ]; then
      [ -d "$probe" ] || blocked "range raw state ancestor is not a directory: $probe"
      break
    fi
    missing+=("$probe")
    probe=${probe%/*}
    [ -n "$probe" ] || probe=/
  done
  validate_directory_ancestors "$probe" range-raw-state-ancestor
  validate_trusted_state_ancestors "$probe"
  local index current
  for ((index=${#missing[@]} - 1; index >= 0; index--)); do
    current=${missing[index]}
    [ ! -e "$current" ] && [ ! -L "$current" ] ||
      blocked "range raw state directory appeared during safe creation: $current"
    mkdir -- "$current" || blocked "cannot create range raw state directory: $current"
    chown root:root -- "$current" || blocked "cannot set range raw state directory owner: $current"
    chmod 0700 -- "$current" || blocked "cannot set range raw state directory mode: $current"
  done
  validate_directory_ancestors "$path" range-raw-state-ancestor
  validate_trusted_state_ancestors "$path"
  [ "$(stat -Lc '%u:%g:%a:%F' -- "$path")" = '0:0:700:directory' ] ||
    blocked 'range raw state directory must be root:root mode 0700'
}

ensure_protected_state_file() {
  local path=$1 label=$2
  if [ -L "$path" ]; then
    blocked "$label must not be a symlink: $path"
  fi
  if [ ! -e "$path" ]; then
    if ! (umask 077; set -C; : >"$path"); then
      [ ! -L "$path" ] && [ -e "$path" ] ||
        blocked "$label could not be created without following a symlink: $path"
    else
      chown root:root -- "$path" || blocked "cannot set $label owner: $path"
      chmod 0600 -- "$path" || blocked "cannot set $label mode: $path"
    fi
  fi
  [ ! -L "$path" ] && [ -f "$path" ] ||
    blocked "$label must be a regular non-symlink file: $path"
  case "$(stat -Lc '%u:%g:%a:%F' -- "$path")" in
    '0:0:600:regular file'|'0:0:600:regular empty file') ;;
    *) blocked "$label must be root:root mode 0600" ;;
  esac
}

ensure_state_directory "$state_dir"
case "$state_file" in
  "$state_dir"/*) ;;
  *) blocked 'range raw state must be a file below its protected state directory' ;;
esac
ensure_protected_state_file "$state_file" range-raw-state
lock_file="$state_dir/lock"
ensure_protected_state_file "$lock_file" range-raw-state-lock
exec 9>>"$lock_file"
flock -n 9 || blocked 'another range raw operation holds the state lock'
verify_lock_fd_identity() {
  local path_identity fd_identity
  [ ! -L "$lock_file" ] || blocked 'range raw lock became a symlink after open'
  path_identity=$(stat -Lc '%d:%i:%u:%g:%a:%F' -- "$lock_file") ||
    blocked 'cannot inspect range raw lock after open'
  fd_identity=$(stat -Lc '%d:%i:%u:%g:%a:%F' -- /proc/$$/fd/9) ||
    blocked 'cannot inspect opened range raw lock descriptor'
  [ "$path_identity" = "$fd_identity" ] ||
    blocked 'opened range raw lock descriptor does not match its path'
  case "$path_identity" in
    *:0:0:600:regular\ file|*:0:0:600:regular\ empty\ file) ;;
    *) blocked 'range raw lock changed owner, mode, or type after open' ;;
  esac
}
verify_lock_fd_identity

verify_state_file_identity() {
  local path_identity fd_identity
  [ ! -L "$state_file" ] || blocked 'range raw state became a symlink after atomic publish'
  path_identity=$(stat -Lc '%d:%i:%u:%g:%a:%F' -- "$state_file") ||
    blocked 'cannot inspect range raw state after atomic publish'
  exec 8<"$state_file" || blocked 'cannot open range raw state for identity check'
  fd_identity=$(stat -Lc '%d:%i:%u:%g:%a:%F' -- /proc/$$/fd/8) || {
    exec 8<&-
    blocked 'cannot inspect opened range raw state descriptor'
  }
  exec 8<&-
  [ "$path_identity" = "$fd_identity" ] ||
    blocked 'opened range raw state descriptor does not match its path'
  case "$path_identity" in
    *:0:0:600:regular\ file|*:0:0:600:regular\ empty\ file) ;;
    *) blocked 'range raw state changed owner, mode, or type after atomic publish' ;;
  esac
}

if [ -e "$state_file" ] && [ -s "$state_file" ]; then
  [ -f "$state_file" ] && [ ! -L "$state_file" ] || blocked 'state file is not regular non-symlink'
  [ "$(wc -l <"$state_file")" -eq 1 ] || blocked 'state file must contain exactly one record'
  IFS=$'\t' read -r version stored_identity stored_status stored_batch_id rest <"$state_file" || true
  [ "$version" = V2 ] && [ "$stored_identity" = "$identity" ] || blocked 'state identity/version mismatch'
  [ -z "${rest:-}" ] || blocked 'state record has unexpected fields'
  [ "$stored_batch_id" = "$(python3 - "$identity" <<'PY'
import sys
import uuid
print(uuid.uuid5(uuid.UUID("7fb4e3e8-5e85-5a4e-9d3b-5c8d14a3e2b1"), sys.argv[1]))
PY
)" ] || blocked 'state source batch identity does not match the deterministic run identity'
  case "$stored_status" in
    COMPLETED) echo 'KIS_RANGE_RAW: PASS (exact identity already completed; immutable evidence retained)'; exit 0 ;;
    RUNNING|FAILED) ;;
    *) blocked 'state status is invalid' ;;
  esac
fi

write_state() {
  local status=$1 tmp parent
  tmp=$(mktemp "$state_dir/.state.XXXXXX")
  chmod 0600 "$tmp"
  printf 'V2\t%s\t%s\t%s\n' "$identity" "$status" "$stored_batch_id" >"$tmp"
  python3 - "$tmp" <<'PY'
import os
import sys
with open(sys.argv[1], "rb") as handle:
    os.fsync(handle.fileno())
PY
  chown root:root "$tmp"
  mv -T -- "$tmp" "$state_file"
  parent=${state_file%/*}
  python3 - "$parent" <<'PY'
import os
import sys
fd = os.open(sys.argv[1], os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
try:
    os.fsync(fd)
finally:
    os.close(fd)
PY
  verify_lock_fd_identity
  verify_state_file_identity
}

write_state RUNNING
compose build --pull=false research-range-raw
verify_image_provenance
RANGE_RAW_BATCH_ID="$stored_batch_id" compose run --rm --no-deps research-range-raw \
  --range-raw --start "$start_date" --end "$end_date"
write_state COMPLETED
echo 'KIS_RANGE_RAW: PASS (Raw-only capture/normalization completed; no publication/Curated/DB action)'
