#!/usr/bin/env bash
# Resumable production backfill plan/executor. The default is --plan and does
# not invoke Docker or a provider. --execute is deliberately guarded so a
# sleeping operator cannot accidentally make external API calls.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$script_dir/../.." && pwd)
source "$script_dir/lib/dotenv.sh"
env_file=${LAGRANGE_ENV_FILE:-$root/deploy/compose/.env}
state_file=${LAGRANGE_BACKFILL_STATE:-}
calendar_dir=${LAGRANGE_XKRX_CALENDAR_DIR:-$root/data/calendars/xkrx}
calendar_selection_file=
calendar_metadata_file=
calendar_id=
calendar_artifact_sha256=
calendar_artifact_size=
calendar_artifact_range=
calendar_session_count=0
calendar_non_session_count=0
start_date=
end_date=
universes=etf
mode=plan
auto_resume=0

usage() {
  cat <<'EOF'
Usage: scripts/ops/backfill-production.sh --start YYYY-MM-DD --end YYYY-MM-DD
       [--universe etf|candidate|all] [--plan|--execute]

The plan covers the fixed 11-ETF KIS EOD dataset. Candidate universes
(KOSPI200/KOSDAQ150) remain a separate blocked step until their credentialed
candidate bridge is released; this command never pretends that KIS EOD bars
are candidate source data.

--execute requires BACKFILL_CONFIRM_EXTERNAL=I_UNDERSTAND_READ_ONLY_KIS_CALLS
and a valid backfill-scope production config. It uses the existing idempotent worker path;
no order/account endpoint is called. The state file is append-only and each
session date must complete Raw -> normalized -> DB publication before progress
is recorded. The validated XKRX scheduler artifact supplies the only session
date input: weekends and closures create zero Docker, worker, or KIS calls.
The one worker process receives the exact sorted session list, makes one
allowlisted KIS chk-holiday call for the first needed date, and validates later
dates against its immutable cached snapshot. A date outside that snapshot makes
no second call and fails closed. The state identity
binds pre-run inputs plus the calendar id,
artifact hash, and full artifact range; the curated dataset
pin is produced and approved after this command, so it is never required here.

--auto-resume is reserved for the recurring systemd timer. It permits a fresh
run, an interrupted RUNNING state, RETRYABLE errors, and the exact
KIS_CALENDAR_SNAPSHOT_MISS deferred state. It refuses every other FAILED state
until an operator explicitly reruns without --auto-resume.
EOF
}

die() { echo "backfill-production: $*" >&2; exit 1; }
blocked() { echo "BLOCKED_EXTERNAL: $*" >&2; exit 2; }
while [ "$#" -gt 0 ]; do
  case "$1" in
    --start)
      [ "$#" -ge 2 ] || die '--start needs YYYY-MM-DD'
      start_date=$2
      shift 2
      ;;
    --end)
      [ "$#" -ge 2 ] || die '--end needs YYYY-MM-DD'
      end_date=$2
      shift 2
      ;;
    --universe)
      [ "$#" -ge 2 ] || die '--universe needs etf|candidate|all'
      universes=$2
      shift 2
      ;;
    --plan) mode=plan; shift ;;
    --execute) mode=execute; shift ;;
    --auto-resume) auto_resume=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done

[ -n "$start_date" ] && [ -n "$end_date" ] || die '--start and --end are required'
[[ "$start_date" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || die 'invalid --start date'
[[ "$end_date" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || die 'invalid --end date'
validate_date_range() {
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
}
if ! date_error=$(validate_date_range 2>&1); then
  die "$date_error"
fi
case "$universes" in
  etf) ;;
  candidate|all)
    blocked 'KOSPI200/KOSDAQ150 credentialed candidate bridge is not part of the current EOD release; use the candidate runbook after that bridge is released'
    ;;
  *) die 'universe must be etf, candidate, or all' ;;
esac

# Read the same strict, non-evaluating dotenv contract used by the validator.
# This is needed even for --plan so the effective absolute data root is still
# validated before a production operation.  Progress state intentionally does
# not follow that worker-writable data root; its default is the protected
# /var/lib/lagrange/state tree below.
[ -f "$env_file" ] || blocked "production env file missing: $env_file"
if ! dotenv_load "$env_file"; then
  echo 'INVALID_CONFIG: production env file is malformed' >&2
  printf '  - %s\n' "${DOTENV_ERRORS[@]}" >&2
  exit 1
fi
data_dir=$(dotenv_get LAGRANGE_DATA_DIR)
[ -n "$data_dir" ] || die 'production env is missing LAGRANGE_DATA_DIR'
[[ "$data_dir" = /* ]] || die 'LAGRANGE_DATA_DIR must be absolute'
case "$data_dir" in
  */../*|*/..) die 'LAGRANGE_DATA_DIR must not contain ..' ;;
esac
# Progress state is deliberately outside the worker-writable data tree.  The
# old /var/lib/lagrange/data/backfill location remains readable only when an
# operator explicitly supplies LAGRANGE_BACKFILL_STATE; it is never migrated
# or removed automatically.
[ -n "$state_file" ] || state_file=/var/lib/lagrange/state/backfill/state.tsv

check_state_path() {
  local path=$1 label=$2 probe
  case "$path" in
    /*) ;;
    *) die "$label must be absolute: $path" ;;
  esac
  case "$path" in
    */../*|*/..) die "$label must not contain '..': $path" ;;
  esac
  probe=$path
  while [ "$probe" != / ]; do
    [ ! -L "$probe" ] || die "$label must not traverse a symlink: $probe"
    probe=${probe%/*}
    [ -n "$probe" ] || probe=/
  done
  if [ -e "$path" ] && { [ -L "$path" ] || [ ! -f "$path" ]; }; then
    die "$label must be a regular non-symlink file: $path"
  fi
}

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
  # /tmp is a sticky system boundary used only by local tests; the random
  # test root below it is still checked.  /var/lib is the production boundary
  # for /var/lib/lagrange/state.  Do not treat a worker-owned data directory
  # as trusted merely because its child was created root-owned.
  while [ "$probe" != / ]; do
    case "$probe" in
      /tmp|/var/lib) break ;;
    esac
    if [ -e "$probe" ]; then
      [ -d "$probe" ] || blocked "backfill state ancestor must be a directory: $probe"
      shape=$(stat -Lc '%u:%g:%a' -- "$probe") ||
        blocked "cannot inspect backfill state ancestor: $probe"
      IFS=: read -r uid gid mode <<<"$shape"
      [ "$uid" = 0 ] && [ "$gid" = 0 ] ||
        blocked "backfill state ancestor must be root:root: $probe"
      if (( 8#$mode & 0022 )); then
        blocked "backfill state ancestor must not be group/other writable: $probe"
      fi
    fi
    probe=${probe%/*}
    [ -n "$probe" ] || probe=/
  done
}

ensure_state_directory() {
  local path=$1 probe=$1
  [ "$(id -u)" -eq 0 ] || blocked 'backfill execute must run as root for protected state/lock files'
  case "$path" in
    /|/*/../*|*/..) blocked 'backfill state directory path is unsafe' ;;
  esac

  # Build missing components one at a time. mkdir never follows an existing
  # symlink here; a race that creates a component between the check and mkdir
  # is treated as a failure rather than followed.
  local -a missing=()
  while [ "$probe" != / ]; do
    [ ! -L "$probe" ] || blocked "backfill state directory must not traverse a symlink: $probe"
    if [ -e "$probe" ]; then
      [ -d "$probe" ] || blocked "backfill state directory ancestor is not a directory: $probe"
      break
    fi
    missing+=("$probe")
    probe=${probe%/*}
    [ -n "$probe" ] || probe=/
  done
  validate_directory_ancestors "$probe" backfill-state-ancestor
  # Check the first existing ancestor before creating any child.  In
  # particular, never create a root-owned state directory below a
  # worker-owned data directory and then claim the path is protected.
  validate_trusted_state_ancestors "$probe"

  local index current
  for ((index=${#missing[@]} - 1; index >= 0; index--)); do
    current=${missing[index]}
    [ ! -e "$current" ] && [ ! -L "$current" ] ||
      blocked "backfill state directory appeared during safe creation: $current"
    mkdir -- "$current" || blocked "cannot create backfill state directory: $current"
    chown root:root -- "$current" || blocked "cannot set backfill state directory owner: $current"
    chmod 0700 -- "$current" || blocked "cannot set backfill state directory mode: $current"
  done

  validate_directory_ancestors "$path" backfill-state-ancestor
  validate_trusted_state_ancestors "$path"
  [ "$(stat -Lc '%u:%g:%a:%F' -- "$path")" = '0:0:700:directory' ] ||
    blocked 'backfill state directory must be root:root mode 0700'
}

ensure_protected_state_file() {
  local path=$1 label=$2
  if [ -L "$path" ]; then
    blocked "$label must not be a symlink: $path"
  fi
  if [ ! -e "$path" ]; then
    # noclobber maps to an exclusive create for the shell's regular-file
    # redirection. The root-only state directory prevents an unprivileged
    # replacement, and a concurrent creator is revalidated below.
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
    *)
    blocked "$label must be root:root mode 0600"
    ;;
  esac
}

verify_lock_fd_identity() {
  local path_identity fd_identity
  [ ! -L "$state_lock" ] || blocked 'backfill state lock became a symlink after open'
  path_identity=$(stat -Lc '%d:%i:%u:%g:%a:%F' -- "$state_lock") ||
    blocked 'cannot inspect backfill state lock after open'
  fd_identity=$(stat -Lc '%d:%i:%u:%g:%a:%F' -- /proc/$$/fd/9) ||
    blocked 'cannot inspect opened backfill state lock descriptor'
  [ "$path_identity" = "$fd_identity" ] ||
    blocked 'opened backfill state lock descriptor does not match its path'
  case "$path_identity" in
    *:0:0:600:regular\ file|*:0:0:600:regular\ empty\ file) ;;
    *) blocked 'backfill state lock changed owner, mode, or type after open' ;;
  esac
}

load_calendar_selection() {
  command -v python3 >/dev/null 2>&1 || blocked 'python3 is required to validate the XKRX session artifact'
  calendar_selection_file=$(mktemp "${TMPDIR:-/tmp}/lagrange-xkrx-sessions.XXXXXX") ||
    die 'cannot stage the validated XKRX session list'
  calendar_metadata_file=$(mktemp "${TMPDIR:-/tmp}/lagrange-xkrx-metadata.XXXXXX") ||
    die 'cannot stage the validated XKRX calendar metadata'
  trap 'rm -f -- "$calendar_selection_file" "$calendar_metadata_file"' EXIT
  if ! python3 "$root/scripts/ops/xkrx-calendar-bootstrap.py" \
    --emit-sessions --start "$start_date" --end "$end_date" \
    --output-dir "$calendar_dir" >"$calendar_selection_file" 2>"$calendar_metadata_file"; then
    blocked 'XKRX scheduler artifact validation failed; no KIS, Docker, or worker call was made'
  fi

  local identity_line
  if ! identity_line=$(python3 - "$calendar_metadata_file" "$calendar_selection_file" "$start_date" "$end_date" <<'PY'
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
lines = Path(metadata_path).read_text(encoding="utf-8").splitlines()
if len(lines) != 1:
    raise SystemExit("XKRX emitter metadata must contain exactly one JSON line")
try:
    metadata = json.loads(lines[0])
except json.JSONDecodeError as exc:
    raise SystemExit(f"XKRX emitter metadata is not valid JSON: {exc}")
if not isinstance(metadata, dict) or metadata.get("schema") != "xkrx-historical-session-selection-v1":
    raise SystemExit("XKRX emitter metadata schema is invalid")
requested = metadata.get("requested_range")
artifact_range = metadata.get("artifact_range")
if requested != {"start": start.isoformat(), "end": end.isoformat()}:
    raise SystemExit("XKRX emitter metadata requested range mismatch")
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
if session_count < 0 or non_session_count < 0 or session_count + non_session_count != (end - start).days + 1:
    raise SystemExit("XKRX selection counts do not cover the requested civil range")
dates = Path(selection_path).read_text(encoding="ascii").splitlines()
if any(not re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}", value) for value in dates):
    raise SystemExit("XKRX session selection contains a malformed date")
if dates != sorted(dates) or len(dates) != len(set(dates)) or len(dates) != session_count:
    raise SystemExit("XKRX session selection is not sorted, unique, or count-matched")
if any(not (start.isoformat() <= value <= end.isoformat()) for value in dates):
    raise SystemExit("XKRX session selection escapes the requested range")
print("\t".join((calendar_id, artifact_sha, str(artifact_size), f"{artifact_start}..{artifact_end}", str(session_count), str(non_session_count))))
PY
  ); then
    blocked 'XKRX scheduler metadata or session list failed closed validation; no KIS, Docker, or worker call was made'
  fi
  IFS=$'\t' read -r calendar_id calendar_artifact_sha256 calendar_artifact_size \
    calendar_artifact_range calendar_session_count calendar_non_session_count <<<"$identity_line"
}

validate_state() {
  local expected_header=$'LAGRANGE_BACKFILL_STATE_V4\t'"$run_identity"
  local line number=0 state_date state_status state_id state_code
  declare -A last_status=()
  while IFS= read -r line || [ -n "$line" ]; do
    number=$((number + 1))
    if [ "$number" -eq 1 ]; then
      [ "$line" = "$expected_header" ] ||
        blocked "backfill state has a stale/foreign schema or run identity; use a new state path"
      continue
    fi
    IFS=$'\t' read -r -a fields <<<"$line"
    [ "${#fields[@]}" -ge 3 ] && [ "${#fields[@]}" -le 4 ] ||
      blocked "backfill state line $number is malformed"
    state_date=${fields[0]}
    state_status=${fields[1]}
    state_id=${fields[2]}
    state_code=${fields[3]:-}
    [ "$state_id" = "$run_identity" ] ||
      blocked "backfill state line $number has a foreign run identity"
    [[ "$state_date" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] ||
      blocked "backfill state line $number has an invalid date"
    [[ "$state_date" < "$start_date" || "$state_date" > "$end_date" ]] &&
      blocked "backfill state line $number is outside the requested date range"
    case "$state_status" in
      RUNNING|PUBLISHED)
        [ "${#fields[@]}" -eq 3 ] && [ -z "$state_code" ] ||
          blocked "backfill state line $number has an unexpected error code"
        ;;
      FAILED)
        [ "${#fields[@]}" -eq 4 ] ||
          blocked "backfill state line $number requires an error code"
        [[ "$state_code" =~ ^[A-Z][A-Z0-9_]{0,63}$ ]] ||
          blocked "backfill state line $number has an invalid error code"
        ;;
      DEFERRED|RETRYABLE)
        [ "${#fields[@]}" -eq 4 ] ||
          blocked "backfill state line $number requires an error code"
        [[ "$state_code" =~ ^[A-Z][A-Z0-9_]{0,63}$ ]] ||
          blocked "backfill state line $number has an invalid error code"
        ;;
      *)
        blocked "backfill state line $number has an invalid status" ;;
    esac
    if [ "${last_status[$state_date]:-}" = PUBLISHED ] && [ "$state_status" != PUBLISHED ]; then
      blocked "backfill state line $number contradicts an already published date"
    fi
    last_status[$state_date]=$state_status
  done <"$state_file"
  [ "$number" -gt 0 ] || blocked 'backfill state is unexpectedly empty'
}

check_auto_resume_state() {
  [ "$auto_resume" -eq 1 ] || return 0
  local last_line last_status last_code
  # A freshly initialized V4 state contains only its identity header.  The
  # first timer invocation is allowed to start the bounded range.
  [ "$(wc -l <"$state_file")" -eq 1 ] && return 0
  last_line=$(tail -n 1 -- "$state_file")
  IFS=$'\t' read -r -a fields <<<"$last_line"
  last_status=${fields[1]:-}
  last_code=${fields[3]:-}
  case "$last_status" in
    RUNNING|PUBLISHED|DEFERRED|RETRYABLE) ;;
    FAILED)
      blocked "automatic resume is blocked by permanent/unknown failure${last_code:+ ($last_code)}; review state and rerun without --auto-resume"
      ;;
    *)
      blocked 'automatic resume requires a validated resumable state'
      ;;
  esac
}

# The calendar artifact is validated before either the plan is printed or any
# execute gate is evaluated.  This makes a missing/tampered/out-of-range
# scheduler input a fail-closed local error, and gives both paths identical
# session counts and identity material.
load_calendar_selection

if [ "$mode" = execute ]; then
  [ "${BACKFILL_CONFIRM_EXTERNAL:-}" = I_UNDERSTAND_READ_ONLY_KIS_CALLS ] ||
    blocked 'execution is disabled until BACKFILL_CONFIRM_EXTERNAL is explicitly set'
  bash "$script_dir/validate-production-config.sh" --scope backfill --env-file "$env_file"
  # The validator is the first effective-environment gate on an external run;
  # repeat the shared parser check here before deriving identity or writing
  # state, so this path cannot drift from Compose's interpolation semantics.
  if ! dotenv_validate_shell_overrides; then
    echo 'INVALID_CONFIG: shell environment would override production env values' >&2
    printf '  - %s\n' "${DOTENV_SHELL_ERRORS[@]}" >&2
    exit 1
  fi
  command -v docker >/dev/null 2>&1 || blocked 'docker is not installed'
  command -v flock >/dev/null 2>&1 || blocked 'flock is required to serialize backfill executions'
  command -v sha256sum >/dev/null 2>&1 || blocked 'sha256sum is required for backfill state identity'
  command -v mktemp >/dev/null 2>&1 || blocked 'mktemp is required for the token issue window'

  state_dir=$(dirname -- "$state_file")
  ensure_state_directory "$state_dir"
  case "$state_file" in
    "$state_dir"/*) ;;
    *) blocked 'backfill state must be a direct file below its protected state directory' ;;
  esac
  ensure_protected_state_file "$state_file" backfill-state
  state_lock="${state_file}.lock"
  ensure_protected_state_file "$state_lock" backfill-state-lock
  exec 9>>"$state_lock" || die "cannot open backfill state lock: $state_lock"
  verify_lock_fd_identity
  flock -n 9 || blocked 'another backfill execution already holds the state lock'
  verify_lock_fd_identity

  code_commit=$(dotenv_effective_get LAGRANGE_CODE_COMMIT)
  entitlement_reference=$(dotenv_get RESEARCH_ENTITLEMENT_REFERENCE)
  [ -n "$entitlement_reference" ] ||
    die 'production env is missing RESEARCH_ENTITLEMENT_REFERENCE'
  identity_payload=$(cat <<EOF
schema=4
start_date=$start_date
end_date=$end_date
universe=$universes
code_commit=$code_commit
entitlement_reference=$entitlement_reference
source_scope=kis/kr|kis-normalized/kr|KRX/KR|etf
calendar_id=$calendar_id
calendar_artifact_sha256=$calendar_artifact_sha256
calendar_artifact_range=$calendar_artifact_range
EOF
)
  run_identity=$(printf '%s' "$identity_payload" | sha256sum | awk '{print $1}')
  [[ "$run_identity" =~ ^[0-9a-f]{64}$ ]] || die 'could not derive backfill run identity'
  if [ ! -s "$state_file" ]; then
    printf 'LAGRANGE_BACKFILL_STATE_V4\t%s\n' "$run_identity" >>"$state_file"
  fi
  validate_state
  check_auto_resume_state
fi

if [ "$mode" = plan ] && ! dotenv_validate_shell_overrides; then
  echo 'INVALID_CONFIG: shell environment would override production env values' >&2
  printf '  - %s\n' "${DOTENV_SHELL_ERRORS[@]}" >&2
  exit 1
fi

if [ "$mode" = plan ]; then
  cat <<EOF
BACKFILL_PLAN: KIS read-only fixed ETF EOD
  civil range: $start_date..$end_date
  validated XKRX scheduler: calendar_id=$calendar_id artifact_sha256=$calendar_artifact_sha256
  artifact range: $calendar_artifact_range
  session dates: $calendar_session_count (non-session skips: $calendar_non_session_count)
  source dates: only validated XKRX sessions; no worker/KIS/Docker call for a skipped date
  raw scope: kis/kr; normalized scope: kis-normalized/kr; DB logical provider: KRX
  idempotency: deterministic normalized batch/source_batch_id and exact manifest replay
  state identity: V4 pre-run inputs plus calendar id/hash/full artifact range
  verification: raw manifest+hashes -> normalize -> four canonical docs -> DB publication
  token safety: one bounded worker/provider reuses one in-memory token and one exact
                chk-holiday snapshot; issue attempts are gated to one per minute and no
                bearer token is persisted; an uncovered date stops fail-closed
  approval: operator reviews manifest/hash/counts, then pins one dataset version for recommendation/backtest/Paper
  state: $state_file (created only by --execute)
PLAN_ONLY: no KIS call, Docker run, persistent state/artifact write, or account/order access
EOF
  exit 0
fi

dates=$(cat -- "$calendar_selection_file")
published_suffix=$'\tPUBLISHED\t'"$run_identity"
pending_dates=()
while IFS= read -r date || [ -n "$date" ]; do
  [ -n "$date" ] || continue
  if grep -Fqx "$date$published_suffix" "$state_file"; then
    echo "BACKFILL_SKIP date=$date state=PUBLISHED"
    continue
  fi
  pending_dates+=("$date")
done <<<"$dates"

if [ "${#pending_dates[@]}" -eq 0 ]; then
  echo "BACKFILL: PASS (sessions=$calendar_session_count skipped_non_sessions=$calendar_non_session_count; no worker/KIS/Docker call)"
  exit 0
fi

docker compose version >/dev/null 2>&1 ||
  blocked 'Docker Compose v2 is unavailable'
# Compose interpolates the WHOLE file even to touch one service, so the
# unrelated Stage5 research-range-raw services abort every invocation here over
# a required RANGE_RAW_BATCH_ID we have no reason to supply — we never run them.
# post-backfill-health.sh and compose-release.sh already pass this exact
# placeholder; export it so both `compose` uses below inherit it.
export RANGE_RAW_BATCH_ID=${RANGE_RAW_BATCH_ID:-compose-config-disabled}
compose=(docker compose --env-file "$env_file" -f "$root/deploy/compose/compose.yml")

# Keep the exact validated session sequence in one bounded argv value.  The
# worker rejects unsorted/duplicate/empty input and iterates these dates only;
# it never reconstructs a civil range or asks KIS about a skipped date.
session_dates_csv=$(IFS=,; printf '%s' "${pending_dates[*]}")
# 1000000, not 1_000_000: bash arithmetic has no digit separators, so the
# underscore form made `[` abort with "integer expected" and fall through to
# the blocked branch below — every run refused as "too large" regardless of
# the actual length.
[ "${#session_dates_csv}" -le 1000000 ] ||
  blocked 'validated XKRX session list is too large for one worker invocation'
worker_start=${pending_dates[0]}
worker_end=${pending_dates[$((${#pending_dates[@]} - 1))]}

# A separately running daemon would own a different process-local token cache.
# Refuse the overlap rather than risk a second issue request.
if ! running_services=$("${compose[@]}" ps --status running --services); then
  blocked 'cannot determine whether the research-worker daemon is running'
fi
if grep -Fxq research-worker <<<"$running_services"; then
  blocked 'research-worker daemon is running; stop it before the bounded backfill range'
fi

# Persist only the time at which this process may have attempted issuance, not
# the bearer token. This closes the one-minute guard across failed container
# starts and operator reruns. The state lock serializes writers and the final
# rename replaces a path entry rather than following a target symlink.
token_window_file="${state_file}.token-window"
check_state_path "$token_window_file" backfill-token-window
now_epoch=$(date +%s)
if [ -e "$token_window_file" ]; then
  [ "$(stat -c '%u:%g:%a' "$token_window_file")" = 0:0:600 ] ||
    blocked 'backfill token issue window must be root:root mode 0600'
  IFS= read -r last_issue_epoch <"$token_window_file" ||
    blocked 'backfill token issue window is unreadable'
  [[ "$last_issue_epoch" =~ ^[0-9]{1,11}$ ]] ||
    blocked 'backfill token issue window is malformed'
  elapsed=$((now_epoch - last_issue_epoch))
  if [ "$elapsed" -lt 60 ]; then
    [ "$elapsed" -ge 0 ] || elapsed=0
    blocked "wait $((60 - elapsed)) seconds before another possible token issue"
  fi
fi
token_window_tmp=$(mktemp "$state_dir/.token-window.XXXXXX") ||
  die 'cannot stage the backfill token issue window'
chmod 0600 "$token_window_tmp"
printf '%s\n' "$now_epoch" >"$token_window_tmp"
mv -fT -- "$token_window_tmp" "$token_window_file"
check_state_path "$token_window_file" backfill-token-window

for date in "${pending_dates[@]}"; do
  printf '%s\tRUNNING\t%s\n' "$date" "$run_identity" >>"$state_file"
done

# The worker flushes one allowlisted, body-free event after each date reaches
# durable publication. Validate the exact inclusive sequence and fsync each
# corresponding state append so a later date failure cannot erase progress.
progress_rc=0
if "${compose[@]}" run --rm --no-deps research-worker \
  --backfill-session-dates "$session_dates_csv" | \
  python3 "$script_dir/lib/backfill-progress.py" \
    "$state_file" "$run_identity" "$worker_start" "$worker_end" "$session_dates_csv"; then
  progress_rc=0
else
  progress_rc=$?
fi
case "$progress_rc" in
  0) ;;
  75)
    echo 'BACKFILL_DEFERRED code=KIS_CALENDAR_SNAPSHOT_MISS (next recurring run may advance the calendar window)' >&2
    exit 75
    ;;
  74)
    echo 'BACKFILL_RETRYABLE (next recurring run may retry after operator review)' >&2
    exit 74
    ;;
  *)
    echo 'BACKFILL_STOPPED range failed; automatic resume is blocked until operator review' >&2
    exit 1
    ;;
esac
echo "BACKFILL: PASS (civil_range=$start_date..$end_date sessions=$calendar_session_count skipped_non_sessions=$calendar_non_session_count)"
