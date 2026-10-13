#!/usr/bin/env bash
# Fake-host integration checks for daily -> backfill session selection.
# No real Docker, provider, production env, calendar, or protected host state is used.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$script_dir/../.." && pwd)
mode=full
if [ "${1:-}" = --reproduce-before-fix ]; then
  mode=baseline
  shift
fi
[ "$#" -eq 0 ] || { echo 'BACKFILL_SELF_TEST: no positional arguments are accepted' >&2; exit 2; }

fail() { echo "BACKFILL_SELF_TEST: $*" >&2; exit 1; }
test_root=$(mktemp -d "${TMPDIR:-/tmp}/lagrange-backfill-self-test.XXXXXX")
cleanup() {
  if [ "${BACKFILL_SELF_TEST_KEEP:-0}" = 1 ]; then
    echo "BACKFILL_SELF_TEST_FIXTURE: $test_root" >&2
  else
    rm -rf -- "$test_root"
  fi
}
trap cleanup EXIT

repo="$test_root/repo"
ops="$repo/scripts/ops"
fake_bin="$test_root/fake-bin"
mkdir -p "$ops/lib" "$repo/deploy/compose" "$fake_bin"
for source in \
  scripts/ops/kis-daily-production.sh \
  scripts/ops/backfill-production.sh \
  scripts/ops/lib/dotenv.sh \
  scripts/ops/lib/db.sh \
  scripts/ops/lib/kis-read-compose.sh \
  scripts/ops/lib/kis-daily-state.py \
  scripts/ops/lib/backfill-progress.py; do
  target="$repo/$source"
  mkdir -p "$(dirname "$target")"
  if [ "$mode" = baseline ]; then
    git -C "$root" show "cea7baa6298402db79d655140ad4c199f57f8697:$source" >"$target"
  else
    cp -- "$root/$source" "$target"
  fi
done
chmod 0755 "$ops/kis-daily-production.sh" "$ops/backfill-production.sh"
touch "$repo/deploy/compose/compose.yml"

cat >"$ops/xkrx-calendar-bootstrap.py" <<'PY'
#!/usr/bin/env python3
import datetime as dt
import json
import os
import sys

args = sys.argv[1:]
try:
    start = dt.date.fromisoformat(args[args.index("--start") + 1])
    end = dt.date.fromisoformat(args[args.index("--end") + 1])
except (ValueError, IndexError):
    raise SystemExit(2)
if end < start:
    raise SystemExit(2)
sessions = os.environ.get("FIXTURE_CALENDAR_DATES", "").split(",")
sessions = [value for value in sessions if value and start.isoformat() <= value <= end.isoformat()]
if sessions != sorted(set(sessions)):
    raise SystemExit(2)
for value in sessions:
    print(value)
metadata = {
    "schema": "xkrx-historical-session-selection-v1",
    "requested_range": {"start": start.isoformat(), "end": end.isoformat()},
    "artifact_range": {"start": "1950-01-01", "end": "2050-12-31"},
    "calendar_id": "fixture-xkrx-v1",
    "artifact_sha256": "sha256:" + "a" * 64,
    "artifact_size_bytes": 1,
    "session_count": len(sessions),
    "skipped_non_session_count": (end - start).days + 1 - len(sessions),
}
print(json.dumps(metadata, separators=(",", ":")), file=sys.stderr)
PY
chmod 0755 "$ops/xkrx-calendar-bootstrap.py"

cat >"$ops/validate-production-config.sh" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
printf 'validate\n' >>"${FIXTURE_VALIDATOR_LOG:?}"
exit 0
SH
chmod 0755 "$ops/validate-production-config.sh"

cat >"$fake_bin/date" <<'SH'
#!/usr/bin/env bash
case "${*: -1}" in
  +%F) printf '%s\n' "${FIXTURE_TODAY:?}" ;;
  +%H:%M) printf '%s\n' "${FIXTURE_NOW_HMS:?}" | cut -c1-5 ;;
  +%s) printf '%s\n' "${FIXTURE_EPOCH:?}" ;;
  *) exit 90 ;;
esac
if [[ "${*: -1}" = +%H:%M ]]; then
  printf '%s\n' "${FIXTURE_NOW_HMS:?}" >>"${FIXTURE_CLOCK_LOG:?}"
fi
SH
cat >"$fake_bin/id" <<'SH'
#!/usr/bin/env bash
[ "$#" -eq 1 ] && [ "$1" = -u ] || exit 90
printf '0\n'
SH
cat >"$fake_bin/chown" <<'SH'
#!/usr/bin/env bash
[ "$#" -eq 3 ] && [ "$2" = -- ] || exit 90
case "$3" in
  "${FIXTURE_ROOT:?}"/*) ;;
  *) exit 91 ;;
esac
exit 0
SH
cat >"$fake_bin/stat" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
format=
path=
if [ "${1-}" = -Lc ]; then
  format=${2-}
  [ "${3-}" = -- ] || exit 90
  path=${4-}
elif [ "${1-}" = -c ]; then
  format=${2-}
  path=${3-}
else
  exec /usr/bin/stat "$@"
fi
case "$format" in
  '%d:%i:%u:%g:%a:%F')
    resolved=$(readlink -f -- "$path")
    inode=$(/usr/bin/stat -Lc '%d:%i' -- "$resolved")
    printf '%s:0:0:600:regular file\n' "$inode"
    ;;
  '%u:%g:%a:%F')
    if [ -d "$path" ]; then printf '0:0:700:directory\n'; else printf '0:0:600:regular file\n'; fi
    ;;
  '%u:%g:%a')
    if [ -d "$path" ]; then printf '0:0:700\n'; else printf '0:0:600\n'; fi
    ;;
  '%a') /usr/bin/stat -c '%a' -- "$path" ;;
  *) exit 92 ;;
esac
SH
cat >"$fake_bin/docker" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
args=("$@")
printf '%s\n' "${args[*]}" >>"${FIXTURE_DOCKER_LOG:?}"
has_db=0
has_worker=0
has_ps=0
for arg in "${args[@]}"; do
  [ "$arg" != db-migrate ] || has_db=1
  [ "$arg" != research-worker ] || has_worker=1
  [ "$arg" != ps ] || has_ps=1
done
if [ "$has_db" -eq 1 ]; then
  cat "${FIXTURE_DB_SNAPSHOT:?}"
  exit 0
fi
if [ "${args[1]-}" = version ]; then
  printf 'Docker Compose version v2.fixture\n'
  exit 0
fi
if [ "$has_ps" -eq 1 ]; then
  exit 0
fi
if [ "$has_worker" -eq 1 ]; then
  session_csv=
  for ((index=0; index<${#args[@]}; index++)); do
    if [ "${args[index]}" = --backfill-session-dates ]; then
      [ "$((index + 1))" -lt "${#args[@]}" ] || exit 93
      session_csv=${args[index+1]}
      break
    fi
  done
  [ -n "$session_csv" ] || exit 94
  printf '%s\n' "$session_csv" >>"${FIXTURE_WORKER_ARG_LOG:?}"
  case "${FIXTURE_WORKER_MODE:-success}" in
    success)
      IFS=, read -r -a sessions <<<"$session_csv"
      for session in "${sessions[@]}"; do
        printf '{"status":"event","event":"published","phase":"canonical_publication","target_date":"%s","batch_id":"00000000-0000-4000-8000-000000000001"}\n' "$session"
      done
      ;;
    retryable)
      first=${session_csv%%,*}
      printf '{"status":"error","error_code":"FIXTURE_RETRYABLE","target_date":"%s","phase":"provider","class":"retryable"}\n' "$first"
      ;;
    deferred)
      first=${session_csv%%,*}
      printf '{"status":"error","error_code":"KIS_CALENDAR_SNAPSHOT_MISS","target_date":"%s","phase":"provider","class":"retryable"}\n' "$first"
      ;;
    permanent)
      first=${session_csv%%,*}
      printf '{"status":"error","error_code":"FIXTURE_PERMANENT","target_date":"%s","phase":"publication","class":"permanent"}\n' "$first"
      ;;
    *) exit 95 ;;
  esac
  exit 0
fi
exit 96
SH
chmod 0755 "$fake_bin/date" "$fake_bin/id" "$fake_bin/chown" "$fake_bin/stat" "$fake_bin/docker"

new_case() {
  CASE_DIR="$test_root/$1"
  mkdir -p "$CASE_DIR/state" "$CASE_DIR/calendar"
  printf 'LAGRANGE_DATA_DIR=%s/data\nLAGRANGE_CODE_COMMIT=0123456789abcdef0123456789abcdef01234567\nRESEARCH_ENTITLEMENT_REFERENCE=fixture-entitlement\nPOSTGRES_DB=lagrange\n' \
    "$CASE_DIR" >"$CASE_DIR/production.env"
  chmod 0600 "$CASE_DIR/production.env"
  : >"$CASE_DIR/docker.log"
  : >"$CASE_DIR/validator.log"
  : >"$CASE_DIR/worker-argv.log"
  : >"$CASE_DIR/clock.log"
  FIXTURE_TODAY=2026-10-13
  FIXTURE_NOW_HMS=16:30:00
  FIXTURE_EPOCH=1791900000
  FIXTURE_CALENDAR_DATES=2026-10-09,2026-10-12,2026-10-13
  FIXTURE_WORKER_MODE=success
  FIXTURE_DB_SNAPSHOT="$CASE_DIR/db.tsv"
  FIXTURE_WORKER_ARG_LOG="$CASE_DIR/worker-argv.log"
  FIXTURE_DOCKER_LOG="$CASE_DIR/docker.log"
  FIXTURE_VALIDATOR_LOG="$CASE_DIR/validator.log"
  FIXTURE_CLOCK_LOG="$CASE_DIR/clock.log"
  FIXTURE_ROOT="$CASE_DIR"
  export FIXTURE_TODAY FIXTURE_NOW_HMS FIXTURE_EPOCH FIXTURE_CALENDAR_DATES
  export FIXTURE_WORKER_MODE FIXTURE_DB_SNAPSHOT FIXTURE_WORKER_ARG_LOG
  export FIXTURE_DOCKER_LOG FIXTURE_VALIDATOR_LOG FIXTURE_CLOCK_LOG FIXTURE_ROOT
  export PATH="$fake_bin:/usr/bin:/bin"
}

write_db() {
  local first last count date_text
  [ "$#" -gt 0 ] || fail 'test DB must contain at least one published session'
  first=$1
  last=${!#}
  count=$#
  {
    printf 'META\t%s\t%s\t%s\t%s\n' "$first" "$last" "$count" "$count"
    for date_text in "$@"; do printf 'DATE\t%s\t1\t-\t-\n' "$date_text"; done
  } >"$FIXTURE_DB_SNAPSHOT"
}

invoke_daily() {
  local fixture=$1 run_mode=$2
  set +e
  env \
    PATH="$fake_bin:/usr/bin:/bin" \
    FIXTURE_TODAY="$FIXTURE_TODAY" FIXTURE_NOW_HMS="$FIXTURE_NOW_HMS" FIXTURE_EPOCH="$FIXTURE_EPOCH" \
    FIXTURE_CALENDAR_DATES="$FIXTURE_CALENDAR_DATES" FIXTURE_WORKER_MODE="$FIXTURE_WORKER_MODE" \
    FIXTURE_DB_SNAPSHOT="$FIXTURE_DB_SNAPSHOT" FIXTURE_WORKER_ARG_LOG="$FIXTURE_WORKER_ARG_LOG" \
    FIXTURE_DOCKER_LOG="$FIXTURE_DOCKER_LOG" FIXTURE_VALIDATOR_LOG="$FIXTURE_VALIDATOR_LOG" \
    FIXTURE_CLOCK_LOG="$FIXTURE_CLOCK_LOG" FIXTURE_ROOT="$FIXTURE_ROOT" \
    LAGRANGE_ENV_FILE="$fixture/production.env" \
    LAGRANGE_XKRX_CALENDAR_DIR="$fixture/calendar" \
    KIS_DAILY_STATE_DIR="$fixture/state" \
    KIS_DAILY_LOCK_FILE="$fixture/state/kis-daily.lock" \
    BACKFILL_CONFIRM_EXTERNAL=I_UNDERSTAND_READ_ONLY_KIS_CALLS \
    bash "$ops/kis-daily-production.sh" "--$run_mode" \
    >"$fixture/daily.out" 2>"$fixture/daily.err"
  LAST_RC=$?
  set -e
}

invoke_backfill() {
  local fixture=$1 state_path=$2
  shift 2
  set +e
  env \
    PATH="$fake_bin:/usr/bin:/bin" \
    FIXTURE_TODAY="$FIXTURE_TODAY" FIXTURE_NOW_HMS="$FIXTURE_NOW_HMS" FIXTURE_EPOCH="$FIXTURE_EPOCH" \
    FIXTURE_CALENDAR_DATES="$FIXTURE_CALENDAR_DATES" FIXTURE_WORKER_MODE="$FIXTURE_WORKER_MODE" \
    FIXTURE_DB_SNAPSHOT="$FIXTURE_DB_SNAPSHOT" FIXTURE_WORKER_ARG_LOG="$FIXTURE_WORKER_ARG_LOG" \
    FIXTURE_DOCKER_LOG="$FIXTURE_DOCKER_LOG" FIXTURE_VALIDATOR_LOG="$FIXTURE_VALIDATOR_LOG" \
    FIXTURE_CLOCK_LOG="$FIXTURE_CLOCK_LOG" FIXTURE_ROOT="$FIXTURE_ROOT" \
    LAGRANGE_ENV_FILE="$fixture/production.env" \
    LAGRANGE_XKRX_CALENDAR_DIR="$fixture/calendar" \
    LAGRANGE_BACKFILL_STATE="$state_path" \
    BACKFILL_CONFIRM_EXTERNAL=I_UNDERSTAND_READ_ONLY_KIS_CALLS \
    bash "$ops/backfill-production.sh" "$@" \
    >"$fixture/backfill.out" 2>"$fixture/backfill.err"
  LAST_RC=$?
  set -e
}

invoke_backfill_filter() {
  local fixture=$1 state_path=$2 csv=$3 start=$4 end=$5
  shift 5
  set +e
  env \
    PATH="$fake_bin:/usr/bin:/bin" \
    FIXTURE_TODAY="$FIXTURE_TODAY" FIXTURE_NOW_HMS="$FIXTURE_NOW_HMS" FIXTURE_EPOCH="$FIXTURE_EPOCH" \
    FIXTURE_CALENDAR_DATES="$FIXTURE_CALENDAR_DATES" FIXTURE_WORKER_MODE="$FIXTURE_WORKER_MODE" \
    FIXTURE_DB_SNAPSHOT="$FIXTURE_DB_SNAPSHOT" FIXTURE_WORKER_ARG_LOG="$FIXTURE_WORKER_ARG_LOG" \
    FIXTURE_DOCKER_LOG="$FIXTURE_DOCKER_LOG" FIXTURE_VALIDATOR_LOG="$FIXTURE_VALIDATOR_LOG" \
    FIXTURE_CLOCK_LOG="$FIXTURE_CLOCK_LOG" FIXTURE_ROOT="$FIXTURE_ROOT" \
    LAGRANGE_ENV_FILE="$fixture/production.env" \
    LAGRANGE_XKRX_CALENDAR_DIR="$fixture/calendar" \
    LAGRANGE_BACKFILL_STATE="$state_path" \
    BACKFILL_CONFIRM_EXTERNAL=I_UNDERSTAND_READ_ONLY_KIS_CALLS \
    bash "$ops/backfill-production.sh" --start "$start" --end "$end" \
      --session-dates "$csv" "$@" \
    >"$fixture/backfill.out" 2>"$fixture/backfill.err"
  LAST_RC=$?
  set -e
}

expect_worker_log() {
  local expected=$1 file=$2
  [ -f "$file" ] || fail "worker argv log missing: $file"
  [ "$(wc -l <"$file")" -eq 1 ] || fail "expected exactly one worker invocation in $file"
  [ "$(<"$file")" = "$expected" ] ||
    fail "worker argv mismatch: expected '$expected', got '$(<"$file")'"
}

expect_no_worker() {
  [ ! -s "$1" ] || fail "worker was called unexpectedly: $(<"$1")"
}

assert_no_execute_effects() {
  local fixture=$1 state_path=$2
  [ ! -e "$state_path" ] || fail 'invalid filter created persistent backfill state'
  [ ! -e "$(dirname "$state_path")" ] || fail 'invalid filter created a persistent state directory'
  [ ! -s "$fixture/validator.log" ] || fail 'invalid filter reached production config validation'
  [ ! -s "$fixture/docker.log" ] || fail 'invalid filter reached Docker'
}

if [ "$mode" = baseline ]; then
  new_case baseline
  write_db 2026-10-09
  FIXTURE_NOW_HMS=16:29:59
  export FIXTURE_NOW_HMS
  invoke_daily "$CASE_DIR" execute
  [ "$LAST_RC" -eq 0 ] || fail "baseline daily execution failed with rc=$LAST_RC"
  expect_worker_log '2026-10-12,2026-10-13' "$CASE_DIR/worker-argv.log"
  grep -Fq 'exact_missing_session_dates=2026-10-12' "$CASE_DIR/daily.out" ||
    fail 'baseline daily check/execute fixture did not retain its filtered list'
  printf 'F1_BASELINE_REPRODUCED: daily selected 2026-10-12 at 16:29:59; real backfill selection sent 2026-10-12,2026-10-13\n'
  exit 0
fi

# Check and execute must hand the same already-filtered sequence across the real
# backfill date selection path. Today must become eligible exactly at 16:30 KST.
new_case preclose
write_db 2026-10-09
FIXTURE_NOW_HMS=16:29:59
export FIXTURE_NOW_HMS
invoke_daily "$CASE_DIR" check
[ "$LAST_RC" -eq 0 ] || fail "pre-close check failed with rc=$LAST_RC"
grep -Fq 'exact_missing_session_dates=2026-10-12' "$CASE_DIR/daily.out" ||
  fail 'pre-close check included today or omitted the older hole'
expect_no_worker "$CASE_DIR/worker-argv.log"
invoke_daily "$CASE_DIR" execute
[ "$LAST_RC" -eq 0 ] || fail "pre-close execute failed with rc=$LAST_RC"
expect_worker_log '2026-10-12' "$CASE_DIR/worker-argv.log"
grep -Fxq '16:29:59' "$CASE_DIR/clock.log" || fail 'fixture did not exercise 16:29:59 KST'
daily_state="$CASE_DIR/state/kis-daily-2026-10-13.tsv"
[ -f "$daily_state" ] || fail 'canonical daily state filename was not used'
state_identity_before=$(head -n 1 "$daily_state")
grep -Fq $'2026-10-12\tPUBLISHED\t' "$daily_state" || fail 'older hole was not published in state'
grep -Fq $'2026-10-13\tPUBLISHED\t' "$daily_state" && fail 'today was marked PUBLISHED before close'

FIXTURE_NOW_HMS=16:30:00
FIXTURE_EPOCH=1791900061
export FIXTURE_NOW_HMS FIXTURE_EPOCH
write_db 2026-10-09 2026-10-12
invoke_daily "$CASE_DIR" execute
[ "$LAST_RC" -eq 0 ] || fail "16:30 execution failed with rc=$LAST_RC"
[ "$(wc -l <"$CASE_DIR/worker-argv.log")" -eq 2 ] || fail 'expected one worker invocation before and one at close'
[ "$(tail -n 1 "$CASE_DIR/worker-argv.log")" = 2026-10-13 ] ||
  fail 'newly eligible today was skipped on repeated execution'
[ "$(head -n 1 "$daily_state")" = "$state_identity_before" ] ||
  fail 'full-range V4 state identity changed across the selection filter'
grep -Fq $'2026-10-12\tPUBLISHED\t' "$daily_state" || fail 'repeated execution lost older PUBLISHED state'
grep -Fq $'2026-10-13\tPUBLISHED\t' "$daily_state" || fail 'today did not publish after close'

# Today-only pending work before close and a fully published range both return
# without crossing into the backfill worker.
new_case today-only
FIXTURE_NOW_HMS=16:29:59
export FIXTURE_NOW_HMS
write_db 2026-10-09 2026-10-12
invoke_daily "$CASE_DIR" execute
[ "$LAST_RC" -eq 0 ] || fail "today-only pre-close execution failed with rc=$LAST_RC"
expect_no_worker "$CASE_DIR/worker-argv.log"
[ ! -e "$CASE_DIR/state/kis-daily-2026-10-13.tsv" ] ||
  fail 'today-only pre-close path wrote daily progress state'

new_case no-missing
write_db 2026-10-09 2026-10-12 2026-10-13
invoke_daily "$CASE_DIR" execute
[ "$LAST_RC" -eq 0 ] || fail "no-missing execution failed with rc=$LAST_RC"
expect_no_worker "$CASE_DIR/worker-argv.log"

# Several interior holes stay holes; PUBLISHED sessions between them are kept
# out of the worker argument even though the full calendar range is reloaded.
new_case interior-holes
FIXTURE_TODAY=2026-10-15
FIXTURE_NOW_HMS=16:30:00
FIXTURE_EPOCH=1791901000
FIXTURE_CALENDAR_DATES=2026-10-09,2026-10-12,2026-10-13,2026-10-14,2026-10-15
export FIXTURE_TODAY FIXTURE_NOW_HMS FIXTURE_EPOCH FIXTURE_CALENDAR_DATES
write_db 2026-10-09 2026-10-13 2026-10-15
invoke_daily "$CASE_DIR" execute
[ "$LAST_RC" -eq 0 ] || fail "interior-hole execution failed with rc=$LAST_RC"
expect_worker_log '2026-10-12,2026-10-14' "$CASE_DIR/worker-argv.log"

# Failures remain fail-closed under auto-resume. Retryable and deferred worker
# classes retain their exact 74/75 exit values through both wrappers.
for worker_case in retryable deferred; do
  new_case "$worker_case"
  write_db 2026-10-09
  FIXTURE_WORKER_MODE=$worker_case
  export FIXTURE_WORKER_MODE
  invoke_daily "$CASE_DIR" execute
  if [ "$worker_case" = retryable ]; then expected_rc=74; else expected_rc=75; fi
  [ "$LAST_RC" -eq "$expected_rc" ] || fail "$worker_case exit propagation was $LAST_RC, expected $expected_rc"
  [ "$(wc -l <"$CASE_DIR/worker-argv.log")" -eq 1 ] || fail "$worker_case did not call exactly one worker"
done

new_case permanent
write_db 2026-10-09
FIXTURE_WORKER_MODE=permanent
export FIXTURE_WORKER_MODE
invoke_daily "$CASE_DIR" execute
[ "$LAST_RC" -eq 1 ] || fail "permanent worker failure returned rc=$LAST_RC, expected 1"
worker_docker_count=$(grep -c 'research-worker' "$CASE_DIR/docker.log")
FIXTURE_WORKER_MODE=success
FIXTURE_EPOCH=1791900061
export FIXTURE_WORKER_MODE FIXTURE_EPOCH
invoke_daily "$CASE_DIR" execute
[ "$LAST_RC" -eq 2 ] || fail "auto-resume accepted FAILED state (rc=$LAST_RC)"
[ "$(wc -l <"$CASE_DIR/worker-argv.log")" -eq 1 ] || fail 'FAILED auto-resume invoked the worker again'
[ "$(grep -c 'research-worker' "$CASE_DIR/docker.log")" -eq "$worker_docker_count" ] || fail 'FAILED auto-resume reached the worker'
grep -Fq $'2026-10-12\tFAILED\t' "$CASE_DIR/state/kis-daily-2026-10-13.tsv" ||
  fail 'permanent failure was not preserved in state'

# Direct backfill checks cover filter validation before every execute-side
# write/call and preserve the default no-filter selection behavior.
run_invalid_filter() {
  local label=$1 csv=$2
  new_case "invalid-$label"
  FIXTURE_CALENDAR_DATES=2026-10-09,2026-10-12,2026-10-13
  export FIXTURE_CALENDAR_DATES
  local state_path="$CASE_DIR/persistent/state.tsv"
  invoke_backfill_filter "$CASE_DIR" "$state_path" "$csv" 2026-10-09 2026-10-13 --execute
  [ "$LAST_RC" -eq 1 ] || fail "invalid $label filter returned rc=$LAST_RC"
  grep -Fq 'session-date filter' "$CASE_DIR/backfill.err" || fail "invalid $label had no typed filter diagnostic"
  assert_no_execute_effects "$CASE_DIR" "$state_path"
}
run_invalid_filter empty ''
run_invalid_filter malformed '2026-10-12,2026-2-13'
run_invalid_filter invalid-date '2026-02-30'
run_invalid_filter out-of-range '2026-10-08'
run_invalid_filter non-session '2026-10-10'
run_invalid_filter duplicate '2026-10-12,2026-10-12'
run_invalid_filter unsorted '2026-10-13,2026-10-12'

new_case too-many-sessions
big_csv_file="$CASE_DIR/sessions.csv"
python3 -B - >"$big_csv_file" <<'PY'
import datetime as dt
start = dt.date(2000, 1, 1)
print(",".join((start + dt.timedelta(days=index)).isoformat() for index in range(10_001)))
PY
big_csv=$(<"$big_csv_file")
FIXTURE_CALENDAR_DATES=$big_csv
export FIXTURE_CALENDAR_DATES
first_big_date=2000-01-01
last_big_date=$(python3 -B -c 'import datetime as d; print((d.date(2000,1,1)+d.timedelta(days=10000)).isoformat())')
big_state="$CASE_DIR/persistent/state.tsv"
invoke_backfill_filter "$CASE_DIR" "$big_state" "$big_csv" "$first_big_date" "$last_big_date" --execute
[ "$LAST_RC" -eq 1 ] || fail "more than 10000 filter dates returned rc=$LAST_RC"
grep -Fq 'at most 10000 dates' "$CASE_DIR/backfill.err" || fail '10000-session limit diagnostic was missing'
assert_no_execute_effects "$CASE_DIR" "$big_state"

new_case default-no-filter
FIXTURE_CALENDAR_DATES=2026-10-09,2026-10-12,2026-10-13
export FIXTURE_CALENDAR_DATES
state_path="$CASE_DIR/state/state.tsv"
invoke_backfill "$CASE_DIR" "$state_path" --start 2026-10-09 --end 2026-10-13 --execute
[ "$LAST_RC" -eq 0 ] || fail "default backfill behavior failed with rc=$LAST_RC"
expect_worker_log '2026-10-09,2026-10-12,2026-10-13' "$CASE_DIR/worker-argv.log"
[ "$(head -n 1 "$state_path")" = "$state_identity_before" ] ||
  fail 'session-date filter changed the full-range V4 run identity'

echo 'BACKFILL_SELF_TEST: PASS (daily/backfill fake-host selection, filter validation, state and 74/75 behavior)'
