#!/usr/bin/env bash
# Protected, operator-gated XKRX calendar refresh for the KIS daily job.
#
# This path is deliberately separate from Git and from the daily provider
# runner. It uses only the repository's pinned exchange_calendars environment
# and the reviewed override ledger through xkrx-calendar-bootstrap.py. It never
# calls KIS, a browser, or an account/order surface.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$script_dir/../.." && pwd)
bootstrap="$root/scripts/ops/xkrx-calendar-bootstrap.py"
output_dir=${KIS_DAILY_CALENDAR_DIR:-/var/lib/lagrange/state/kis-daily/calendar}
python_bin=${KIS_DAILY_CALENDAR_PYTHON:-$root/nt/.venv/bin/python}
start_date=2020-01-31
end_date=2027-12-31
mode=plan
replace=0

usage() {
  cat <<'EOF'
Usage: scripts/ops/kis-daily-calendar-refresh.sh [--plan|--check|--apply] [--replace]

The default --plan is side-effect free. --check validates the protected
operational artifact for the fixed 2020-01-31..2027-12-31 horizon. --apply
requires root and an already provisioned pinned nt/.venv; it atomically
materializes the dates-only calendar, manifest, and reviewed override ledger
under KIS_DAILY_CALENDAR_DIR. --apply never downloads a package and never
contacts KIS or an interactive UI. --replace is required to replace an existing
different artifact after operator review.
EOF
}

die() { echo "kis-daily-calendar-refresh: $*" >&2; exit 1; }
blocked() { echo "BLOCKED_EXTERNAL: $*" >&2; exit 2; }

while [ "$#" -gt 0 ]; do
  case "$1" in
    --plan) mode=plan; shift ;;
    --check) mode=check; shift ;;
    --apply) mode=apply; shift ;;
    --replace) replace=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done

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
  probe=${path%/}
  [ -n "$probe" ] || probe=/
  while [ "$probe" != / ]; do
    [ ! -L "$probe" ] || die "$label must not traverse a symlink: $probe"
    probe=${probe%/*}
    [ -n "$probe" ] || probe=/
  done
}

ensure_protected_directory() {
  local path=$1 probe=$1
  [ "$(id -u)" -eq 0 ] || blocked '--apply must run as root for the protected calendar directory'
  safe_path "$path" operational-calendar-directory
  [ "$path" != / ] || blocked 'operational calendar directory is too broad'
  local -a missing=()
  while [ "$probe" != / ]; do
    [ ! -L "$probe" ] || blocked "operational calendar path traverses a symlink: $probe"
    if [ -e "$probe" ]; then
      [ -d "$probe" ] || blocked "operational calendar ancestor is not a directory: $probe"
      break
    fi
    missing+=("$probe")
    probe=${probe%/*}
    [ -n "$probe" ] || probe=/
  done
  local existing=$probe shape uid gid mode index current
  while [ "$existing" != / ]; do
    [ ! -L "$existing" ] || blocked "operational calendar ancestor is a symlink: $existing"
    if [ -e "$existing" ]; then
      [ -d "$existing" ] || blocked "operational calendar ancestor is not a directory: $existing"
      shape=$(stat -Lc '%u:%g:%a' -- "$existing") || blocked "cannot inspect operational calendar ancestor: $existing"
      IFS=: read -r uid gid mode <<<"$shape"
      [ "$uid" = 0 ] && [ "$gid" = 0 ] || blocked "operational calendar ancestor must be root:root: $existing"
      (( 8#$mode & 0022 )) && blocked "operational calendar ancestor is group/other writable: $existing"
    fi
    existing=${existing%/*}
    [ -n "$existing" ] || existing=/
  done
  for ((index=${#missing[@]} - 1; index >= 0; index--)); do
    current=${missing[index]}
    [ ! -e "$current" ] && [ ! -L "$current" ] || blocked "operational calendar path appeared during safe creation: $current"
    mkdir -- "$current" || blocked "cannot create operational calendar directory: $current"
    chown root:root -- "$current" || blocked "cannot set operational calendar owner: $current"
    chmod 0700 -- "$current" || blocked "cannot set operational calendar mode: $current"
  done
  shape=$(stat -Lc '%u:%g:%a:%F' -- "$path") || blocked "cannot inspect operational calendar directory: $path"
  [ "$shape" = '0:0:700:directory' ] || blocked 'operational calendar directory must be root:root mode 0700'
}

safe_path "$output_dir" operational-calendar-directory
[ -x "$bootstrap" ] || die 'XKRX calendar bootstrap is missing or not executable'

if [ "$mode" = plan ]; then
  python3 "$bootstrap" --plan --start "$start_date" --end "$end_date" --output-dir "$output_dir"
  echo 'PROTECTED_REFRESH: output is not Git; apply writes only the configured protected directory'
  exit 0
fi

if [ "$mode" = check ]; then
  [ -d "$output_dir" ] && [ ! -L "$output_dir" ] || blocked "protected operational calendar is missing: $output_dir"
  if ! python3 "$bootstrap" --check --start "$start_date" --end "$end_date" --output-dir "$output_dir"; then
    blocked 'protected operational XKRX calendar failed the pinned artifact/manifest/override validation'
  fi
  exit 0
fi

[ "$(id -u)" -eq 0 ] || blocked '--apply must run as root'
safe_path "$python_bin" pinned-calendar-python
[ -x "$python_bin" ] && [ ! -L "$python_bin" ] || blocked "pinned calendar Python is unavailable: $python_bin"
if ! "$python_bin" - "$root/nt/.venv" <<'PY'
import importlib.metadata
from pathlib import Path
import sys

expected_prefix = Path(sys.argv[1]).resolve()
if Path(sys.prefix).resolve() != expected_prefix:
    raise SystemExit("calendar Python is not the repository nt/.venv")
if importlib.metadata.version("exchange_calendars") != "4.13.2":
    raise SystemExit("calendar Python has an unexpected exchange_calendars version")
PY
then
  blocked 'pinned calendar Python/version is not available; refusing uv re-exec or package download'
fi
ensure_protected_directory "$output_dir"

locked_child_token=kis-daily-calendar-refresh-locked-child
apply_args=("$bootstrap" --apply --start "$start_date" --end "$end_date" --output-dir "$output_dir" --_locked-child-token "$locked_child_token")
[ "$replace" -eq 1 ] && apply_args+=(--replace)
if ! XKRX_CALENDAR_BOOTSTRAP_REEXEC="$locked_child_token" "$python_bin" "${apply_args[@]}"; then
  blocked 'pinned XKRX calendar materialization failed closed; no provider or external call was made'
fi

for artifact in calendar.json manifest.json overrides.json; do
  [ -f "$output_dir/$artifact" ] && [ ! -L "$output_dir/$artifact" ] ||
    die "calendar refresh did not produce a regular $artifact"
  chown root:root -- "$output_dir/$artifact"
  chmod 0600 -- "$output_dir/$artifact"
done
echo "KIS_DAILY_CALENDAR_REFRESH: PASS range=$start_date..$end_date output=$output_dir"
