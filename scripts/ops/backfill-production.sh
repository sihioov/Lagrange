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
worker run must complete Raw -> normalized -> DB publication before progress
is recorded. The state identity binds only pre-run inputs; the curated dataset
pin is produced and approved after this command, so it is never required here.
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
  local line number=0 state_date state_status state_id extra
  while IFS= read -r line || [ -n "$line" ]; do
    number=$((number + 1))
    if [ "$number" -eq 1 ]; then
      [ "$line" = "$expected_header" ] ||
        blocked "backfill state has a stale/foreign schema or run identity; use a new state path"
      continue
    fi
    IFS=$'\t' read -r state_date state_status state_id extra <<<"$line"
    [ "$line" = "$state_date$(printf '\t')$state_status$(printf '\t')$state_id" ] ||
      blocked "backfill state line $number is malformed"
    [[ "$state_date" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] ||
      blocked "backfill state line $number has an invalid date"
    case "$state_status" in RUNNING|PUBLISHED|FAILED) ;; *)
      blocked "backfill state line $number has an invalid status" ;;
    esac
    [ "$state_id" = "$run_identity" ] ||
      blocked "backfill state line $number has a foreign run identity"
  done <"$state_file"
  [ "$number" -gt 0 ] || blocked 'backfill state is unexpectedly empty'
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

  check_state_path "$state_file" backfill-state
  state_dir=$(dirname -- "$state_file")
  mkdir -p -- "$state_dir"
  check_state_path "$state_file" backfill-state
  state_lock="${state_file}.lock"
  check_state_path "$state_lock" backfill-state-lock
  exec 9>>"$state_lock" || die "cannot open backfill state lock: $state_lock"
  flock -n 9 || blocked 'another backfill execution already holds the state lock'

  : >>"$state_file"
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
while IFS= read -r date; do
  if grep -Fqx "$date$published_suffix" "$state_file"; then
    echo "BACKFILL_SKIP date=$date state=PUBLISHED"
    continue
  fi
  printf '%s\tRUNNING\t%s\n' "$date" "$run_identity" >>"$state_file"
  if "${compose[@]}" run --rm --no-deps research-worker --once --date "$date"; then
    printf '%s\tPUBLISHED\t%s\n' "$date" "$run_identity" >>"$state_file"
    echo "BACKFILL_DONE date=$date"
  else
    printf '%s\tFAILED\t%s\n' "$date" "$run_identity" >>"$state_file"
    echo "BACKFILL_STOPPED date=$date (rerun resumes after operator review)" >&2
    exit 1
  fi
done <<<"$dates"
echo 'BACKFILL: PASS'
