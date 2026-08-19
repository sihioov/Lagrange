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
date must complete Raw -> normalized -> DB publication before progress is
recorded. One bounded worker process owns the inclusive range so its in-memory
KIS token and one exact chk-holiday calendar snapshot are reused; the token is
never persisted by this script. If the broker calendar snapshot does not cover
the next date, the worker stops fail-closed without issuing a second calendar
request; rerun the same state after review to advance the snapshot window. The
state identity binds only pre-run inputs; the curated dataset
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
# This is needed even for --plan so the default state path follows the
# effective absolute LAGRANGE_DATA_DIR rather than a host-specific fallback.
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
[ -n "$state_file" ] || state_file="$data_dir/backfill/state.tsv"

date_list() {
  python3 - "$start_date" "$end_date" <<'PY'
import datetime as dt
import sys
start = dt.date.fromisoformat(sys.argv[1])
end = dt.date.fromisoformat(sys.argv[2])
if end < start:
    raise SystemExit("backfill-production: --end precedes --start")
day = start
while day <= end:
    print(day.isoformat())
    day += dt.timedelta(days=1)
PY
}

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

validate_state() {
  local expected_header=$'LAGRANGE_BACKFILL_STATE_V3\t'"$run_identity"
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
        if [ "${#fields[@]}" -eq 4 ]; then
          [[ "$state_code" =~ ^[A-Z][A-Z0-9_]{0,63}$ ]] ||
            blocked "backfill state line $number has an invalid error code"
        elif [ "${#fields[@]}" -ne 3 ]; then
          blocked "backfill state line $number is malformed"
        fi
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
  # A freshly initialized V3 state contains only its identity header.  The
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
  docker compose version >/dev/null 2>&1 || blocked 'Docker Compose v2 is unavailable'
  command -v flock >/dev/null 2>&1 || blocked 'flock is required to serialize backfill executions'
  command -v sha256sum >/dev/null 2>&1 || blocked 'sha256sum is required for backfill state identity'
  command -v mktemp >/dev/null 2>&1 || blocked 'mktemp is required for the token issue window'

  check_state_path "$state_file" backfill-state
  state_dir=$(dirname -- "$state_file")
  mkdir -p -- "$state_dir"
  check_state_path "$state_file" backfill-state
  state_lock="${state_file}.lock"
  check_state_path "$state_lock" backfill-state-lock
  exec 9>>"$state_lock" || die "cannot open backfill state lock: $state_lock"
  flock -n 9 || blocked 'another backfill execution already holds the state lock'

  if [ -e "$state_file" ]; then
    [ "$(stat -c '%u:%g:%a' "$state_file")" = 0:0:600 ] ||
      blocked 'backfill state must be root:root mode 0600'
  else
    (umask 077; : >>"$state_file")
    chmod 0600 -- "$state_file"
  fi
  check_state_path "$state_file" backfill-state

  code_commit=$(dotenv_effective_get LAGRANGE_CODE_COMMIT)
  entitlement_reference=$(dotenv_get RESEARCH_ENTITLEMENT_REFERENCE)
  [ -n "$entitlement_reference" ] ||
    die 'production env is missing RESEARCH_ENTITLEMENT_REFERENCE'
  identity_payload=$(cat <<EOF
schema=3
start_date=$start_date
end_date=$end_date
universe=$universes
code_commit=$code_commit
entitlement_reference=$entitlement_reference
source_scope=kis/kr|kis-normalized/kr|KRX/KR|etf
EOF
)
  run_identity=$(printf '%s' "$identity_payload" | sha256sum | awk '{print $1}')
  [[ "$run_identity" =~ ^[0-9a-f]{64}$ ]] || die 'could not derive backfill run identity'
  if [ ! -s "$state_file" ]; then
    printf 'LAGRANGE_BACKFILL_STATE_V3\t%s\n' "$run_identity" >>"$state_file"
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
  source dates: $start_date..$end_date (provider decides trading-day/holiday response)
  raw scope: kis/kr; normalized scope: kis-normalized/kr; DB logical provider: KRX
  idempotency: deterministic normalized batch/source_batch_id and exact manifest replay
  state identity: V3 pre-run inputs only (date range, universe, code, entitlement, source scope)
  verification: raw manifest+hashes -> normalize -> four canonical docs -> DB publication
  token safety: one bounded worker/provider reuses one in-memory token and one exact
                chk-holiday snapshot; issue attempts are gated to one per minute and no
                bearer token is persisted; an uncovered date stops fail-closed
  approval: operator reviews manifest/hash/counts, then pins one dataset version for recommendation/backtest/Paper
  state: $state_file (created only by --execute)
PLAN_ONLY: no KIS call, Docker run, file write, or account/order access
EOF
  exit 0
fi

compose=(docker compose --env-file "$env_file" -f "$root/deploy/compose/compose.yml")
if ! dates=$(date_list); then
  die 'failed to enumerate the validated date range'
fi
published_suffix=$'\tPUBLISHED\t'"$run_identity"
pending_dates=()
while IFS= read -r date; do
  if grep -Fqx "$date$published_suffix" "$state_file"; then
    echo "BACKFILL_SKIP date=$date state=PUBLISHED"
    continue
  fi
  pending_dates+=("$date")
done <<<"$dates"

if [ "${#pending_dates[@]}" -eq 0 ]; then
  echo 'BACKFILL: PASS'
  exit 0
fi

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
  --backfill-range --start "$start_date" --end "$end_date" | \
  python3 "$script_dir/lib/backfill-progress.py" \
    "$state_file" "$run_identity" "$start_date" "$end_date"; then
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
echo 'BACKFILL: PASS'
