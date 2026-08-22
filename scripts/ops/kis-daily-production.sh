#!/usr/bin/env bash
# Once-daily, read-only KIS incremental EOD runner.
#
# --plan and --check never invoke the provider. --execute obtains the exact
# missing XKRX session list from the published DB snapshot and delegates one
# bounded invocation to backfill-production.sh. That existing path owns the
# research-worker process, TokenManager, chk-holiday snapshot, progress relay,
# and provider safety boundary.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$script_dir/../.." && pwd)
source "$script_dir/lib/dotenv.sh"
source "$script_dir/lib/db.sh"

env_file=${LAGRANGE_ENV_FILE:-$root/deploy/compose/.env}
calendar_dir=${LAGRANGE_XKRX_CALENDAR_DIR:-${KIS_DAILY_CALENDAR_DIR:-/var/lib/lagrange/state/kis-daily/calendar}}
state_dir=${KIS_DAILY_STATE_DIR:-/var/lib/lagrange/state/backfill}
lock_file=${KIS_DAILY_LOCK_FILE:-/var/lib/lagrange/state/backfill/kis-daily.lock}
mode=plan
max_sessions=10000
today=
range_start=
calendar_selection_file=
calendar_metadata_file=
calendar_identity_line=
calendar_id=
calendar_artifact_sha256=
calendar_artifact_size=
calendar_artifact_range=
calendar_session_count=0
calendar_non_session_count=0
db_snapshot_file=
db_parsed_file=
state_file=
run_identity=

usage() {
  cat <<'EOF'
Usage: scripts/ops/kis-daily-production.sh [--plan|--check|--execute]

The default --plan validates only the literal production dotenv contract and
the protected operational XKRX calendar artifact. It makes no DB, Docker, or
provider call. Set LAGRANGE_XKRX_CALENDAR_DIR explicitly for a local checked-in
artifact review. --check validates the protected backfill configuration, reads one
published KRX/KR credentialed-EOD DB snapshot, and reports the exact missing
XKRX sessions without starting a worker. --execute takes the same snapshot,
then delegates the exact oldest-first missing list to one
research-worker --backfill-session-dates process through the existing
backfill-production.sh path.

The scheduled unit is 16:30 Asia/Seoul with Persistent=true. Execute requires
BACKFILL_CONFIRM_EXTERNAL=I_UNDERSTAND_READ_ONLY_KIS_CALLS and never enables a
live/order/account surface. State and lock paths are root-owned protected
paths; KIS credential values are not read by this wrapper.
EOF
}

die() { echo "kis-daily-production: $*" >&2; exit 1; }
blocked() { echo "BLOCKED_EXTERNAL: $*" >&2; exit 2; }
invalid_config() {
  echo 'INVALID_CONFIG: KIS daily production configuration is malformed' >&2
  printf '  - %s\n' "$@" >&2
  exit 1
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --plan) mode=plan; shift ;;
    --check) mode=check; shift ;;
    --execute) mode=execute; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done

cleanup() {
  rm -f -- "$calendar_selection_file" "$calendar_metadata_file" \
    "$db_snapshot_file" "$db_parsed_file"
}
trap cleanup EXIT

validate_absolute_path() {
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

validate_date() {
  local value=$1
  [[ "$value" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || return 1
  python3 - "$value" <<'PY'
import datetime as dt
import sys

value = sys.argv[1]
try:
    parsed = dt.date.fromisoformat(value)
except ValueError:
    raise SystemExit(1)
if parsed.isoformat() != value:
    raise SystemExit(1)
PY
}

today=$(TZ=Asia/Seoul date +%F)
validate_date "$today" || die 'Asia/Seoul current date is invalid'
validate_absolute_path "$env_file" production-env-file
validate_absolute_path "$calendar_dir" xkrx-calendar-directory

load_literal_env() {
  [ -f "$env_file" ] || blocked "production env file missing: $env_file"
  [ ! -L "$env_file" ] || die "production env file must not be a symlink: $env_file"
  if ! dotenv_load "$env_file"; then
    invalid_config "${DOTENV_ERRORS[@]}"
  fi
  local data_dir
  data_dir=$(dotenv_get LAGRANGE_DATA_DIR)
  [ -n "$data_dir" ] || invalid_config 'LAGRANGE_DATA_DIR is missing'
  [[ "$data_dir" = /* ]] || invalid_config 'LAGRANGE_DATA_DIR must be absolute'
  case "$data_dir" in
    */../*|*/..) invalid_config 'LAGRANGE_DATA_DIR must not contain ..' ;;
  esac
  if ! dotenv_validate_shell_overrides; then
    invalid_config "${DOTENV_SHELL_ERRORS[@]}"
  fi
}

load_calendar_selection() {
  local start_date=$1 end_date=$2
  validate_date "$start_date" || die 'calendar start date is invalid'
  validate_date "$end_date" || die 'calendar end date is invalid'
  calendar_selection_file=$(mktemp "${TMPDIR:-/tmp}/lagrange-kis-daily-sessions.XXXXXX") ||
    die 'cannot stage the XKRX session selection'
  calendar_metadata_file=$(mktemp "${TMPDIR:-/tmp}/lagrange-kis-daily-metadata.XXXXXX") ||
    die 'cannot stage the XKRX calendar metadata'
  if ! python3 "$root/scripts/ops/xkrx-calendar-bootstrap.py" \
    --emit-sessions --start "$start_date" --end "$end_date" \
    --output-dir "$calendar_dir" >"$calendar_selection_file" 2>"$calendar_metadata_file"; then
    blocked 'XKRX scheduler artifact validation failed; no DB, Docker, worker, or KIS call was made'
  fi

  if ! calendar_identity_line=$(python3 - "$calendar_metadata_file" \
    "$calendar_selection_file" "$start_date" "$end_date" <<'PY'
import datetime as dt
import json
import re
import sys
from pathlib import Path

metadata_path, selection_path, start_text, end_text = sys.argv[1:]
try:
    start = dt.date.fromisoformat(start_text)
    end = dt.date.fromisoformat(end_text)
except ValueError as exc:
    raise SystemExit(f"invalid requested range: {exc}")
if end < start:
    raise SystemExit("requested calendar range is reversed")

metadata_lines = Path(metadata_path).read_text(encoding="utf-8").splitlines()
if len(metadata_lines) != 1:
    raise SystemExit("XKRX emitter metadata must contain exactly one JSON line")
try:
    metadata = json.loads(metadata_lines[0])
except json.JSONDecodeError as exc:
    raise SystemExit(f"XKRX emitter metadata is not valid JSON: {exc}")
if not isinstance(metadata, dict) or metadata.get("schema") != "xkrx-historical-session-selection-v1":
    raise SystemExit("XKRX emitter metadata schema is invalid")
if metadata.get("requested_range") != {"start": start.isoformat(), "end": end.isoformat()}:
    raise SystemExit("XKRX emitter metadata requested range mismatch")
artifact_range = metadata.get("artifact_range")
if not isinstance(artifact_range, dict):
    raise SystemExit("XKRX emitter metadata artifact range is invalid")
try:
    artifact_start = dt.date.fromisoformat(artifact_range["start"])
    artifact_end = dt.date.fromisoformat(artifact_range["end"])
except (KeyError, TypeError, ValueError) as exc:
    raise SystemExit(f"XKRX emitter metadata artifact range is invalid: {exc}")
if not artifact_start <= start <= end <= artifact_end:
    raise SystemExit("requested range is outside the validated XKRX artifact range")
calendar_id = metadata.get("calendar_id")
artifact_sha = metadata.get("artifact_sha256")
artifact_size = metadata.get("artifact_size_bytes")
if not isinstance(calendar_id, str) or not re.fullmatch(r"[a-z0-9][a-z0-9._-]{0,127}", calendar_id):
    raise SystemExit("XKRX calendar id is invalid")
if not isinstance(artifact_sha, str) or not re.fullmatch(r"sha256:[0-9a-f]{64}", artifact_sha):
    raise SystemExit("XKRX artifact SHA-256 is invalid")
if not isinstance(artifact_size, int) or artifact_size <= 0:
    raise SystemExit("XKRX artifact size is invalid")
try:
    session_count = int(metadata["session_count"])
    non_session_count = int(metadata["skipped_non_session_count"])
except (KeyError, TypeError, ValueError) as exc:
    raise SystemExit(f"XKRX selection counts are invalid: {exc}")
if session_count < 0 or non_session_count < 0:
    raise SystemExit("XKRX selection counts are negative")
if session_count + non_session_count != (end - start).days + 1:
    raise SystemExit("XKRX selection counts do not cover the requested civil range")

dates = Path(selection_path).read_text(encoding="ascii").splitlines()
if any(not re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}", value) for value in dates):
    raise SystemExit("XKRX session selection contains a malformed date")
for value in dates:
    try:
        parsed = dt.date.fromisoformat(value)
    except ValueError:
        raise SystemExit("XKRX session selection contains an impossible date")
    if parsed.isoformat() != value:
        raise SystemExit("XKRX session selection contains a non-canonical date")
if dates != sorted(dates) or len(dates) != len(set(dates)) or len(dates) != session_count:
    raise SystemExit("XKRX session selection is not sorted, unique, or count-matched")
if any(not (start.isoformat() <= value <= end.isoformat()) for value in dates):
    raise SystemExit("XKRX session selection escapes the requested range")
print("\t".join((
    calendar_id,
    artifact_sha,
    str(artifact_size),
    f"{artifact_start}..{artifact_end}",
    str(session_count),
    str(non_session_count),
)))
PY
  ); then
    blocked 'XKRX scheduler metadata or session list failed closed; no DB, Docker, worker, or KIS call was made'
  fi
  IFS=$'\t' read -r calendar_id calendar_artifact_sha256 calendar_artifact_size \
    calendar_artifact_range calendar_session_count calendar_non_session_count <<<"$calendar_identity_line"
}

ensure_state_directory() {
  local path=$1 probe=$1
  [ "$(id -u)" -eq 0 ] || blocked 'execute must run as root for protected state and lock files'
  validate_absolute_path "$path" protected-state-directory
  [ "$path" != / ] || blocked 'protected state directory is too broad'

  local -a missing=()
  while [ "$probe" != / ]; do
    [ ! -L "$probe" ] || blocked "protected state directory must not traverse a symlink: $probe"
    if [ -e "$probe" ]; then
      [ -d "$probe" ] || blocked "protected state ancestor is not a directory: $probe"
      break
    fi
    missing+=("$probe")
    probe=${probe%/*}
    [ -n "$probe" ] || probe=/
  done

  local existing=$probe shape uid gid mode index current
  while [ "$existing" != / ]; do
    [ ! -L "$existing" ] || blocked "protected state ancestor is a symlink: $existing"
    if [ -e "$existing" ]; then
      [ -d "$existing" ] || blocked "protected state ancestor is not a directory: $existing"
      shape=$(stat -Lc '%u:%g:%a' -- "$existing") || blocked "cannot inspect protected state ancestor: $existing"
      IFS=: read -r uid gid mode <<<"$shape"
      [ "$uid" = 0 ] && [ "$gid" = 0 ] || blocked "protected state ancestor must be root:root: $existing"
      (( 8#$mode & 0022 )) && blocked "protected state ancestor is group/other writable: $existing"
    fi
    existing=${existing%/*}
    [ -n "$existing" ] || existing=/
  done

  for ((index=${#missing[@]} - 1; index >= 0; index--)); do
    current=${missing[index]}
    [ ! -e "$current" ] && [ ! -L "$current" ] || blocked "protected state directory appeared during safe creation: $current"
    mkdir -- "$current" || blocked "cannot create protected state directory: $current"
    chown root:root -- "$current" || blocked "cannot set protected state owner: $current"
    chmod 0700 -- "$current" || blocked "cannot set protected state mode: $current"
  done

  shape=$(stat -Lc '%u:%g:%a:%F' -- "$path") || blocked "cannot inspect protected state directory: $path"
  [ "$shape" = '0:0:700:directory' ] || blocked 'protected state directory must be root:root mode 0700'
}

ensure_protected_file() {
  local path=$1 label=$2
  [ ! -L "$path" ] || blocked "$label must not be a symlink: $path"
  if [ ! -e "$path" ]; then
    if ! (umask 077; set -C; : >"$path"); then
      [ ! -L "$path" ] && [ -e "$path" ] || blocked "$label could not be created without following a symlink: $path"
    else
      chown root:root -- "$path" || blocked "cannot set $label owner: $path"
      chmod 0600 -- "$path" || blocked "cannot set $label mode: $path"
    fi
  fi
  [ ! -L "$path" ] && [ -f "$path" ] || blocked "$label must be a regular non-symlink file: $path"
  case "$(stat -Lc '%u:%g:%a:%F' -- "$path")" in
    '0:0:600:regular file'|'0:0:600:regular empty file') ;;
    *) blocked "$label must be root:root mode 0600" ;;
  esac
}

verify_lock_fd_identity() {
  local path_identity fd_identity
  [ ! -L "$lock_file" ] || blocked 'daily lock became a symlink after open'
  path_identity=$(stat -Lc '%d:%i:%u:%g:%a:%F' -- "$lock_file") || blocked 'cannot inspect opened daily lock path'
  fd_identity=$(stat -Lc '%d:%i:%u:%g:%a:%F' -- /proc/$$/fd/9) || blocked 'cannot inspect opened daily lock descriptor'
  [ "$path_identity" = "$fd_identity" ] || blocked 'opened daily lock descriptor does not match its path'
  case "$path_identity" in
    *:0:0:600:regular\ file|*:0:0:600:regular\ empty\ file) ;;
    *) blocked 'daily lock changed owner, mode, or type after open' ;;
  esac
}

acquire_daily_lock() {
  [ "$(id -u)" -eq 0 ] || blocked 'execute must run as root for the single-run lock'
  validate_absolute_path "$lock_file" daily-lock
  local lock_parent
  lock_parent=$(dirname -- "$lock_file")
  ensure_state_directory "$lock_parent"
  case "$lock_file" in
    "$lock_parent"/*) ;;
    *) blocked 'daily lock must be directly below its protected directory' ;;
  esac
  ensure_protected_file "$lock_file" daily-lock
  exec 9>>"$lock_file" || die "cannot open daily lock: $lock_file"
  verify_lock_fd_identity
  flock -n 9 || blocked 'another KIS daily execution already holds the single-run lock'
  verify_lock_fd_identity
}

run_config_validator() {
  bash "$script_dir/validate-production-config.sh" --scope backfill --env-file "$env_file"
  if ! dotenv_validate_shell_overrides; then
    invalid_config "${DOTENV_SHELL_ERRORS[@]}"
  fi
}

snapshot_database() {
  db_init
  db_snapshot_file=$(mktemp "${TMPDIR:-/tmp}/lagrange-kis-daily-db.XXXXXX") || die 'cannot stage the published DB snapshot'
  local snapshot_sql
  snapshot_sql=$'WITH grouped AS (\n'
  snapshot_sql+=$'  SELECT batch_date::date AS batch_date, count(*)::bigint AS eod_rows\n'
  snapshot_sql+=$'    FROM public.data_batches\n'
  snapshot_sql+=$'   WHERE provider=\'KRX\' AND market=\'KR\' AND kind=\'EOD\'\n'
  snapshot_sql+=$'     AND fetch_mode=\'credentialed\'\n'
  snapshot_sql+=$'   GROUP BY batch_date\n'
  snapshot_sql+=$'), rows AS (\n'
  snapshot_sql+=$'  SELECT 0 AS row_order, \'META\' AS row_type,\n'
  snapshot_sql+=$'         COALESCE(min(batch_date)::text, \'\') AS first_date,\n'
  snapshot_sql+=$'         COALESCE(max(batch_date)::text, \'\') AS last_date,\n'
  snapshot_sql+=$'         count(*)::text AS date_count,\n'
  snapshot_sql+=$'         COALESCE(sum(eod_rows), 0)::text AS row_count\n'
  snapshot_sql+=$'    FROM grouped\n'
  snapshot_sql+=$'  UNION ALL\n'
  snapshot_sql+=$'  SELECT 1, \'DATE\', batch_date::text, eod_rows::text, \'-\', \'-\'\n'
  snapshot_sql+=$'    FROM grouped\n'
  snapshot_sql+=$')\n'
  snapshot_sql+=$'SELECT row_type, first_date, last_date, date_count, row_count\n'
  snapshot_sql+=$'  FROM rows ORDER BY row_order, first_date;'

  if ! db_psql -qAt -F $'\t' -c "$snapshot_sql" >"$db_snapshot_file"; then
    blocked 'published DB snapshot query failed; no Docker worker or KIS call was made'
  fi
  db_parsed_file=$(mktemp "${TMPDIR:-/tmp}/lagrange-kis-daily-db-parsed.XXXXXX") || die 'cannot stage the parsed DB snapshot'
  if ! python3 - "$db_snapshot_file" "$today" >"$db_parsed_file" 2>/dev/null <<'PY'
import datetime as dt
import re
import sys
from pathlib import Path

snapshot_path, today_text = sys.argv[1:]
today = dt.date.fromisoformat(today_text)
lines = Path(snapshot_path).read_text(encoding="ascii").splitlines()
if not lines:
    raise SystemExit("DB snapshot is empty")

meta = None
dates = []
for line in lines:
    fields = line.split("\t")
    if len(fields) != 5:
        raise SystemExit("DB snapshot row shape is invalid")
    row_type = fields[0]
    if row_type == "META":
        if meta is not None or dates:
            raise SystemExit("DB snapshot has an ambiguous META row")
        meta = fields[1:]
        continue
    if row_type != "DATE" or meta is None or fields[3:] != ["-", "-"]:
        raise SystemExit("DB snapshot row type/order is invalid")
    date_text, eod_count = fields[1], fields[2]
    if not re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}", date_text):
        raise SystemExit("DB snapshot contains a malformed date")
    try:
        parsed = dt.date.fromisoformat(date_text)
        count = int(eod_count)
    except ValueError:
        raise SystemExit("DB snapshot contains an invalid date/count")
    if parsed.isoformat() != date_text or parsed > today:
        raise SystemExit("DB snapshot contains a non-canonical or future date")
    if count != 1:
        raise SystemExit("DB snapshot contains an ambiguous EOD date")
    dates.append(date_text)

if meta is None:
    raise SystemExit("DB snapshot META row is missing")
if len(dates) == 0:
    raise SystemExit("DB has no credentialed KRX/KR EOD publication frontier")
if dates != sorted(dates) or len(dates) != len(set(dates)):
    raise SystemExit("DB snapshot dates are not sorted and unique")
first_text, last_text, date_count_text, row_count_text = meta
try:
    date_count = int(date_count_text)
    row_count = int(row_count_text)
except ValueError:
    raise SystemExit("DB snapshot META counts are invalid")
if first_text != dates[0] or last_text != dates[-1]:
    raise SystemExit("DB snapshot META range does not match DATE rows")
if date_count != len(dates) or row_count != len(dates):
    raise SystemExit("DB snapshot META counts do not match DATE rows")

print(f"META\t{first_text}\t{last_text}\t{date_count}\t{row_count}")
for date_text in dates:
    print(f"DATE\t{date_text}\t1\t-\t-")
PY
  then
    blocked 'published DB state is missing, malformed, future-dated, or ambiguous; no worker or KIS call was made'
  fi
}

load_literal_env

if [ "$mode" = plan ]; then
  load_calendar_selection "$today" "$today"
  cat <<EOF
KIS_DAILY_PLAN: read-only incremental ETF EOD
  schedule: one systemd activation daily at 16:30 Asia/Seoul (Persistent=true)
  validated XKRX scheduler: calendar_id=$calendar_id artifact_sha256=$calendar_artifact_sha256
  artifact range: $calendar_artifact_range
  today session count: $calendar_session_count (non-session skips: $calendar_non_session_count)
  source scope: KIS read-only market data; DB logical provider KRX/KR; universe etf
  execution: one exact missing-session list through backfill-production.sh --auto-resume
  safety: protected single-run lock; one worker process owns one TokenManager and one chk-holiday snapshot
PLAN_ONLY: no DB, Docker, worker, KIS, secret-value, or state operation made
EOF
  exit 0
fi

if [ "$mode" = execute ]; then
  acquire_daily_lock
  run_config_validator
else
  run_config_validator
fi

snapshot_database
range_start=$(awk -F '\t' '$1 == "META" {print $2; exit}' "$db_parsed_file")
[ -n "$range_start" ] || blocked 'DB snapshot did not provide a publication frontier'
load_calendar_selection "$range_start" "$today"

declare -A published_dates=()
published_count=0
while IFS=$'\t' read -r row_type date_text eod_count ignored_one ignored_two; do
  [ "$row_type" = DATE ] || continue
  published_dates["$date_text"]=1
  published_count=$((published_count + 1))
done <"$db_parsed_file"
[ "$published_count" -gt 0 ] || blocked 'DB publication frontier is empty'

declare -A session_dates=()
while IFS= read -r date_text || [ -n "$date_text" ]; do
  [ -n "$date_text" ] || continue
  session_dates["$date_text"]=1
done <"$calendar_selection_file"

for date_text in "${!published_dates[@]}"; do
  [ -n "${session_dates[$date_text]+set}" ] || blocked "DB-published date is not present in the validated XKRX session artifact: $date_text"
done

pending_dates=()
while IFS= read -r date_text || [ -n "$date_text" ]; do
  [ -n "$date_text" ] || continue
  if [ -z "${published_dates[$date_text]+set}" ]; then
    pending_dates+=("$date_text")
  fi
done <"$calendar_selection_file"

[ "${#pending_dates[@]}" -le "$max_sessions" ] || blocked "exact missing XKRX session list exceeds the worker bound of $max_sessions"
if [ "${#pending_dates[@]}" -eq 0 ]; then
  echo "KIS_DAILY: PASS (published=$published_count sessions=$calendar_session_count skipped_non_sessions=$calendar_non_session_count; no worker/Docker/KIS call)"
  exit 0
fi

session_dates_csv=$(IFS=,; printf '%s' "${pending_dates[*]}")
[ "${#session_dates_csv}" -le 1000000 ] || blocked 'exact missing XKRX session list is too large for one worker invocation'

if [ "$mode" = check ]; then
  echo "KIS_DAILY_CHECK: published=$published_count sessions=$calendar_session_count missing=${#pending_dates[@]}"
  echo "  range=$range_start..$today oldest_missing=${pending_dates[0]} newest_missing=${pending_dates[$((${#pending_dates[@]} - 1))]}"
  echo "  exact_missing_session_dates=$session_dates_csv"
  echo 'CHECK_ONLY: no worker, Docker, or KIS call made'
  exit 0
fi

[ "${BACKFILL_CONFIRM_EXTERNAL:-}" = I_UNDERSTAND_READ_ONLY_KIS_CALLS ] || blocked 'execution is disabled until BACKFILL_CONFIRM_EXTERNAL is explicitly set'

state_file=${KIS_DAILY_STATE_FILE:-$state_dir/kis-daily-${today}.tsv}
validate_absolute_path "$state_file" daily-state-file
state_parent=$(dirname -- "$state_file")
ensure_state_directory "$state_parent"
case "$state_file" in
  "$state_parent"/*) ;;
  *) blocked 'daily state must be a direct file below its protected state directory' ;;
esac
ensure_protected_file "$state_file" daily-state

code_commit=$(dotenv_effective_get LAGRANGE_CODE_COMMIT)
entitlement_reference=$(dotenv_get RESEARCH_ENTITLEMENT_REFERENCE)
[ -n "$code_commit" ] || die 'production env is missing LAGRANGE_CODE_COMMIT'
[ -n "$entitlement_reference" ] || die 'production env is missing RESEARCH_ENTITLEMENT_REFERENCE'
identity_payload=$(cat <<EOF
schema=4
start_date=$range_start
end_date=$today
universe=etf
code_commit=$code_commit
entitlement_reference=$entitlement_reference
source_scope=kis/kr|kis-normalized/kr|KRX/KR|etf
calendar_id=$calendar_id
calendar_artifact_sha256=$calendar_artifact_sha256
calendar_artifact_range=$calendar_artifact_range
EOF
)
run_identity=$(printf '%s' "$identity_payload" | sha256sum | awk '{print $1}')
[[ "$run_identity" =~ ^[0-9a-f]{64}$ ]] || die 'could not derive daily backfill run identity'

if ! python3 - "$state_file" "$run_identity" "$db_parsed_file" 2>/dev/null <<'PY'
import os
import sys

state_path, identity, db_path = sys.argv[1:]
header = f"LAGRANGE_BACKFILL_STATE_V4\t{identity}"
with open(state_path, encoding="ascii") as state:
    lines = state.read().splitlines()
if lines and lines[0] != header:
    raise SystemExit("daily state identity mismatch")

published = set()
for line in lines[1:]:
    fields = line.split("\t")
    if len(fields) == 3 and fields[1] == "PUBLISHED" and fields[2] == identity:
        published.add(fields[0])

db_dates = []
with open(db_path, encoding="ascii") as db:
    for line in db:
        fields = line.rstrip("\n").split("\t")
        if fields and fields[0] == "DATE":
            db_dates.append(fields[1])

to_append = [value for value in db_dates if value not in published]
with open(state_path, "a", encoding="ascii") as state:
    if not lines:
        state.write(header + "\n")
    for value in to_append:
        state.write(f"{value}\tPUBLISHED\t{identity}\n")
    state.flush()
    os.fsync(state.fileno())
PY
then
  blocked 'daily state is missing, stale, malformed, or not appendable; no worker or KIS call was made'
fi

echo "KIS_DAILY_EXECUTE: range=$range_start..$today missing=${#pending_dates[@]} state=$state_file"
echo "  exact_missing_session_dates=$session_dates_csv"
echo '  worker_contract=one research-worker --backfill-session-dates process; shared TokenManager/chk-holiday snapshot'

set +e
LAGRANGE_ENV_FILE="$env_file" \
LAGRANGE_XKRX_CALENDAR_DIR="$calendar_dir" \
LAGRANGE_BACKFILL_STATE="$state_file" \
  bash "$script_dir/backfill-production.sh" \
    --start "$range_start" --end "$today" --universe etf --auto-resume --execute
worker_rc=$?
set -e
case "$worker_rc" in
  0) exit 0 ;;
  74|75) exit "$worker_rc" ;;
  *) exit "$worker_rc" ;
esac
