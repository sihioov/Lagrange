#!/usr/bin/env bash
# Fake-host integration checks for daily -> backfill session selection.
# No real Docker, provider, production env, calendar, or protected host state is used.
set -euo pipefail
PATH=/usr/bin:/bin
export PATH

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$script_dir/../.." && pwd)
mode=full
if [ "${1:-}" = --reproduce-before-fix ]; then
  mode=baseline
  shift
fi
[ "$#" -eq 0 ] || { echo 'BACKFILL_SELF_TEST: no positional arguments are accepted' >&2; exit 2; }

fail() { echo "BACKFILL_SELF_TEST: $*" >&2; exit 1; }
test_root=$(/usr/bin/mktemp -d /tmp/lagrange-backfill-self-test.XXXXXX)
case "$test_root" in
  /tmp/lagrange-backfill-self-test.??????) ;;
  *) fail 'mktemp returned a path outside the fixed self-test root pattern' ;;
esac
[ -d "$test_root" ] && [ "$(readlink -e -- "$test_root")" = "$test_root" ] ||
  fail 'mktemp root is not a canonical real directory'
readonly test_root
cleanup() {
  if [ "${BACKFILL_SELF_TEST_KEEP:-0}" = 1 ]; then
    echo "BACKFILL_SELF_TEST_FIXTURE: $test_root" >&2
  else
    case "$test_root" in
      /tmp/lagrange-backfill-self-test.??????)
        [ -d "$test_root" ] && [ "$(readlink -e -- "$test_root")" = "$test_root" ] || {
          echo 'BACKFILL_SELF_TEST: refusing cleanup outside the canonical mktemp root' >&2
          return 1
        }
        /usr/bin/rm -rf -- "$test_root"
        ;;
      *)
        echo 'BACKFILL_SELF_TEST: refusing cleanup outside the fixed mktemp pattern' >&2
        return 1
        ;;
    esac
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
    /usr/bin/env -i PATH=/usr/bin:/bin LC_ALL=C \
      /usr/bin/git -C "$root" show "cea7baa6298402db79d655140ad4c199f57f8697:$source" >"$target"
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
[ "$#" -eq 3 ] && [ "$1" = root:root ] && [ "$2" = -- ] || exit 90
fixture_root=$(readlink -e -- "${FIXTURE_ROOT:?}") || exit 91
[ "$fixture_root" = "$FIXTURE_ROOT" ] || exit 91
resolved=$(readlink -e -- "$3") || exit 91
case "$resolved" in
  "$fixture_root"|"$fixture_root"/*) ;;
  *) exit 91 ;;
esac
[ ! -L "$3" ] && { [ -d "$resolved" ] || [ -f "$resolved" ]; } || exit 91
exit 0
SH
cat >"$fake_bin/stat" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
format=
path=
stat_mode=${1-}
if [ "${1-}" = -Lc ]; then
  format=${2-}
  [ "${3-}" = -- ] || exit 90
  path=${4-}
elif [ "${1-}" = -c ]; then
  format=${2-}
  if [ "${3-}" = -- ]; then path=${4-}; else path=${3-}; fi
else
  exit 90
fi
fixture_root=$(readlink -e -- "${FIXTURE_ROOT:?}") || exit 91
[ "$fixture_root" = "$FIXTURE_ROOT" ] || exit 91
resolved=$(readlink -e -- "$path") || exit 91
is_fixture_ancestor() {
  local candidate=$1 allowed
  while IFS= read -r allowed; do
    [ "$candidate" = "$allowed" ] && return 0
  done <<<"${FIXTURE_ANCESTOR_ALLOWLIST:?}"
  return 1
}
in_fixture=0
case "$resolved" in
  "$fixture_root"|"$fixture_root"/*) in_fixture=1 ;;
esac
ancestor_allowlisted=0
if [ -d "$path" ] && [ "$resolved" = "$path" ] && is_fixture_ancestor "$resolved"; then
  ancestor_allowlisted=1
fi
case "$format" in
  '%d:%i:%u:%g:%a:%F')
    [ "$in_fixture" -eq 1 ] && [ -f "$resolved" ] || exit 91
    inode=$(/usr/bin/stat -Lc '%d:%i' -- "$resolved")
    printf '%s:0:0:600:regular file\n' "$inode"
    ;;
  '%u:%g:%a:%F')
    [ "$in_fixture" -eq 1 ] || exit 91
    if [ -d "$resolved" ]; then
      printf '0:0:700:directory\n'
    elif [ -f "$resolved" ]; then
      if [ -s "$resolved" ]; then printf '0:0:600:regular file\n'; else printf '0:0:600:regular empty file\n'; fi
    else
      exit 91
    fi
    ;;
  '%u:%g:%a')
    if [ "$in_fixture" -eq 1 ]; then
      if [ -d "$resolved" ]; then printf '0:0:700\n'; elif [ -f "$resolved" ]; then printf '0:0:600\n'; else exit 91; fi
    elif [ "$ancestor_allowlisted" -eq 1 ] && [ "$stat_mode" = -Lc ] && [ "${3-}" = -- ]; then
      # Deliberate test-only model for the daily ancestor guard. This is
      # synthetic metadata for canonical directories in this fixture's exact
      # ancestor chain, not evidence about their real host ownership or mode.
      printf '0:0:755\n'
    else
      exit 91
    fi
    ;;
  '%a')
    [ "$in_fixture" -eq 1 ] && [ -f "$resolved" ] || exit 91
    /usr/bin/stat -c '%a' -- "$resolved"
    ;;
  *) exit 90 ;;
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
  raw_mode=0
  for ((index=0; index<${#args[@]}; index++)); do
    if [ "${args[index]}" = --r2-action-supply ]; then
      [ "${RESEARCH_R2_ACTION_SUPPLY_CONFIG_HEX:-}" = '7b2266697874757265223a747275657d' ] || exit 95
      printf 'ACTION_SUPPLY_FIXTURE: %s\n' "${args[*]}"
      [ "${FIXTURE_WORKER_MODE:-}" != supply-failure ] || exit 2
      exit 0
    fi
    if [ "${args[index]}" = --eod-raw-check-session-dates ]; then
      target=${args[index+1]}
      present=false
      batch=null
      if [ "${FIXTURE_WORKER_MODE:-}" = raw-existing ]; then
        present=true
        batch='"00000000-0000-4000-8000-000000000001"'
      fi
      printf '{"status":"event","event":"eod_source_checked","phase":"raw_check","target_date":"%s","source_present":%s,"batch_id":%s,"ready":false,"publication":false,"curated":false,"db":false}\n' "$target" "$present" "$batch"
      printf '{"status":"ok","phase":"raw_check","outcome":"raw_checked","sessions":1,"ready":false,"publication":false,"curated":false,"db":false}\n'
      exit 0
    fi
    if [ "${args[index]}" = --backfill-session-dates ] || [ "${args[index]}" = --eod-raw-session-dates ]; then
      [ "${args[index]}" != --eod-raw-session-dates ] || raw_mode=1
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
        if [ "$raw_mode" -eq 1 ]; then
          printf '{"status":"event","event":"raw_committed","phase":"raw_capture","target_date":"%s","batch_id":"00000000-0000-4000-8000-000000000001","reused_existing_source":false,"ready":false,"publication":false,"curated":false,"db":false}\n' "$session"
        else
          printf '{"status":"event","event":"published","phase":"canonical_publication","target_date":"%s","batch_id":"00000000-0000-4000-8000-000000000001"}\n' "$session"
        fi
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
  mkdir -p "$CASE_DIR/state" "$CASE_DIR/calendar" "$CASE_DIR/tmp" "$CASE_DIR/home"
  FIXTURE_ROOT=$(readlink -e -- "$CASE_DIR") || fail 'cannot resolve fixture root'
  [ "$FIXTURE_ROOT" = "$CASE_DIR" ] && [ -d "$FIXTURE_ROOT" ] ||
    fail 'fixture root is not a canonical real directory'
  FIXTURE_ANCESTOR_ALLOWLIST=
  ancestor=$FIXTURE_ROOT
  while :; do
    [ -d "$ancestor" ] && [ "$(readlink -e -- "$ancestor")" = "$ancestor" ] ||
      fail "fixture ancestor is not a canonical real directory: $ancestor"
    FIXTURE_ANCESTOR_ALLOWLIST+="$ancestor"$'\n'
    [ "$ancestor" = / ] && break
    ancestor=${ancestor%/*}
    [ -n "$ancestor" ] || ancestor=/
  done
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
  export FIXTURE_TODAY FIXTURE_NOW_HMS FIXTURE_EPOCH FIXTURE_CALENDAR_DATES
  export FIXTURE_WORKER_MODE FIXTURE_DB_SNAPSHOT FIXTURE_WORKER_ARG_LOG
  export FIXTURE_DOCKER_LOG FIXTURE_VALIDATOR_LOG FIXTURE_CLOCK_LOG
  export FIXTURE_ROOT FIXTURE_ANCESTOR_ALLOWLIST
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

run_fixture() {
  local fixture=$1 backfill_state=$2
  shift 2
  /usr/bin/env -i \
    PATH="$fake_bin:/usr/bin:/bin" \
    HOME="$fixture/home" LC_ALL=C \
    TMPDIR="$fixture/tmp" \
    FIXTURE_TODAY="$FIXTURE_TODAY" FIXTURE_NOW_HMS="$FIXTURE_NOW_HMS" FIXTURE_EPOCH="$FIXTURE_EPOCH" \
    FIXTURE_CALENDAR_DATES="$FIXTURE_CALENDAR_DATES" FIXTURE_WORKER_MODE="$FIXTURE_WORKER_MODE" \
    FIXTURE_DB_SNAPSHOT="$FIXTURE_DB_SNAPSHOT" FIXTURE_WORKER_ARG_LOG="$FIXTURE_WORKER_ARG_LOG" \
    FIXTURE_DOCKER_LOG="$FIXTURE_DOCKER_LOG" FIXTURE_VALIDATOR_LOG="$FIXTURE_VALIDATOR_LOG" \
    FIXTURE_CLOCK_LOG="$FIXTURE_CLOCK_LOG" FIXTURE_ROOT="$FIXTURE_ROOT" \
    FIXTURE_ANCESTOR_ALLOWLIST="$FIXTURE_ANCESTOR_ALLOWLIST" \
    LAGRANGE_ENV_FILE="$fixture/production.env" \
    LAGRANGE_XKRX_CALENDAR_DIR="$fixture/calendar" \
    LAGRANGE_BACKFILL_STATE="$backfill_state" \
    KIS_DAILY_STATE_DIR="$fixture/state" \
    KIS_DAILY_STATE_FILE="$fixture/state/kis-daily-${FIXTURE_TODAY}.tsv" \
    KIS_DAILY_EARLIEST_TODAY_KST=16:30 \
    KIS_DAILY_LOCK_FILE="$fixture/state/kis-daily.lock" \
    BACKFILL_CONFIRM_EXTERNAL=I_UNDERSTAND_READ_ONLY_KIS_CALLS \
    "$@"
}

invoke_daily() {
  local fixture=$1 run_mode=$2
  shift 2
  set +e
  run_fixture "$fixture" "$fixture/state/kis-daily-${FIXTURE_TODAY}.tsv" \
    bash "$ops/kis-daily-production.sh" "--$run_mode" "$@" \
    >"$fixture/daily.out" 2>"$fixture/daily.err"
  LAST_RC=$?
  set -e
}

invoke_backfill() {
  local fixture=$1 state_path=$2
  shift 2
  set +e
  run_fixture "$fixture" "$state_path" \
    bash "$ops/backfill-production.sh" "$@" \
    >"$fixture/backfill.out" 2>"$fixture/backfill.err"
  LAST_RC=$?
  set -e
}

invoke_backfill_filter() {
  local fixture=$1 state_path=$2 csv=$3 start=$4 end=$5
  shift 5
  set +e
  run_fixture "$fixture" "$state_path" \
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

assert_fake_rejects_external_file() {
  local fixture=$1 external_file=$2 external_dir stat_rc chown_rc dir_stat_rc
  external_dir=$(dirname -- "$external_file")
  set +e
  /usr/bin/env -i \
    PATH="$fake_bin:/usr/bin:/bin" FIXTURE_ROOT="$FIXTURE_ROOT" \
    FIXTURE_ANCESTOR_ALLOWLIST="$FIXTURE_ANCESTOR_ALLOWLIST" \
    "$fake_bin/stat" -Lc '%u:%g:%a:%F' -- "$external_file" \
    >"$fixture/external-stat.out" 2>"$fixture/external-stat.err"
  stat_rc=$?
  /usr/bin/env -i \
    PATH="$fake_bin:/usr/bin:/bin" FIXTURE_ROOT="$FIXTURE_ROOT" \
    FIXTURE_ANCESTOR_ALLOWLIST="$FIXTURE_ANCESTOR_ALLOWLIST" \
    "$fake_bin/stat" -Lc '%u:%g:%a' -- "$external_dir" \
    >"$fixture/external-dir-stat.out" 2>"$fixture/external-dir-stat.err"
  dir_stat_rc=$?
  /usr/bin/env -i \
    PATH="$fake_bin:/usr/bin:/bin" FIXTURE_ROOT="$FIXTURE_ROOT" \
    "$fake_bin/chown" root:root -- "$external_file" \
    >"$fixture/external-chown.out" 2>"$fixture/external-chown.err"
  chown_rc=$?
  set -e
  [ "$stat_rc" -ne 0 ] && [ ! -s "$fixture/external-stat.out" ] ||
    fail 'fake stat certified an external test-owned file'
  [ "$dir_stat_rc" -ne 0 ] && [ ! -s "$fixture/external-dir-stat.out" ] ||
    fail 'fake stat certified a sibling directory outside the active fixture'
  [ "$chown_rc" -ne 0 ] && [ ! -s "$fixture/external-chown.out" ] ||
    fail 'fake chown certified an external test-owned file'
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
sentinel_dir="$test_root/outside-case"
mkdir -p "$sentinel_dir"
hostile_state_sentinel="$sentinel_dir/daily-state-sentinel.tsv"
sentinel_before="$test_root/daily-state-sentinel.before"
printf 'test-owned external state sentinel\nbyte-two\n' >"$hostile_state_sentinel"
cp -- "$hostile_state_sentinel" "$sentinel_before"
assert_fake_rejects_external_file "$CASE_DIR" "$hostile_state_sentinel"
had_inherited_state_file=${KIS_DAILY_STATE_FILE+x}
inherited_state_file=${KIS_DAILY_STATE_FILE-}
had_inherited_cutoff=${KIS_DAILY_EARLIEST_TODAY_KST+x}
inherited_cutoff=${KIS_DAILY_EARLIEST_TODAY_KST-}
export KIS_DAILY_STATE_FILE="$hostile_state_sentinel"
export KIS_DAILY_EARLIEST_TODAY_KST=00:00
[ "$KIS_DAILY_STATE_FILE" = "$hostile_state_sentinel" ] &&
  [ "$KIS_DAILY_EARLIEST_TODAY_KST" = 00:00 ] ||
  fail 'hostile state/cutoff values were not inherited by the fixture caller'
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
cmp -s "$sentinel_before" "$hostile_state_sentinel" ||
  fail 'hostile inherited state-file override changed the external sentinel'
grep -Fxq '16:29:59' "$CASE_DIR/clock.log" || fail 'fixture did not exercise 16:29:59 KST'
daily_state="$CASE_DIR/state/kis-daily-2026-10-13.tsv"
[ -f "$daily_state" ] || fail 'canonical daily state filename was not used'
[ "$(readlink -e -- "$daily_state")" = "$daily_state" ] ||
  fail 'daily state file did not resolve to the canonical fixture path'
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
[ "$(head -n 1 "$CASE_DIR/worker-argv.log")" = 2026-10-12 ] ||
  fail 'hostile inherited cutoff changed the pre-close worker argument'
[ "$(tail -n 1 "$CASE_DIR/worker-argv.log")" = 2026-10-13 ] ||
  fail 'newly eligible today was skipped on repeated execution'
[ "$(wc -l <"$CASE_DIR/clock.log")" -gt 0 ] || fail 'fixture clock log was empty'
[ "$(head -n 1 "$daily_state")" = "$state_identity_before" ] ||
  fail 'full-range V4 state identity changed across the selection filter'
grep -Fq $'2026-10-12\tPUBLISHED\t' "$daily_state" || fail 'repeated execution lost older PUBLISHED state'
grep -Fq $'2026-10-13\tPUBLISHED\t' "$daily_state" || fail 'today did not publish after close'
grep -Fxq '16:30:00' "$CASE_DIR/clock.log" || fail 'fixture did not exercise the exact 16:30:00 KST boundary'
cmp -s "$sentinel_before" "$hostile_state_sentinel" ||
  fail 'hostile inherited state-file override changed the sentinel after repeat execution'
if [ "$had_inherited_state_file" = x ]; then
  export KIS_DAILY_STATE_FILE="$inherited_state_file"
else
  unset KIS_DAILY_STATE_FILE
fi
if [ "$had_inherited_cutoff" = x ]; then
  export KIS_DAILY_EARLIEST_TODAY_KST="$inherited_cutoff"
else
  unset KIS_DAILY_EARLIEST_TODAY_KST
fi

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
/usr/bin/env -i PATH=/usr/bin:/bin HOME="$CASE_DIR/home" LC_ALL=C TMPDIR="$CASE_DIR/tmp" \
  python3 -B - >"$big_csv_file" <<'PY'
import datetime as dt
start = dt.date(2000, 1, 1)
print(",".join((start + dt.timedelta(days=index)).isoformat() for index in range(10_001)))
PY
big_csv=$(<"$big_csv_file")
FIXTURE_CALENDAR_DATES=$big_csv
export FIXTURE_CALENDAR_DATES
first_big_date=2000-01-01
last_big_date=$(/usr/bin/env -i PATH=/usr/bin:/bin HOME="$CASE_DIR/home" LC_ALL=C TMPDIR="$CASE_DIR/tmp" \
  python3 -B -c 'import datetime as d; print((d.date(2000,1,1)+d.timedelta(days=10000)).isoformat())')
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

new_case raw-historical
state_path="$CASE_DIR/state/raw.tsv"
invoke_backfill "$CASE_DIR" "$state_path" --raw-source --start 2026-10-09 --end 2026-10-12 --execute
[ "$LAST_RC" -eq 0 ] || fail "Raw historical capture failed rc=$LAST_RC"
grep -q '^LAGRANGE_EOD_RAW_STATE_V1' "$state_path" || fail 'Raw state schema missing'
[ "$(grep -c $'\tRAW_COMMITTED\t' "$state_path")" -eq 2 ] || fail 'Raw progress count mismatch'
! grep -q PUBLISHED "$state_path" || fail 'Raw capture was labelled publication'
grep -q -- '--eod-raw-session-dates 2026-10-09,2026-10-12' "$CASE_DIR/docker.log" || fail 'Raw mode did not reach explicit worker command'
invoke_backfill "$CASE_DIR" "$state_path" --start 2026-10-09 --end 2026-10-12 --execute
[ "$LAST_RC" -eq 2 ] || fail 'legacy mode accepted Raw state'

new_case raw-mixed-current
invoke_backfill "$CASE_DIR" "$CASE_DIR/persistent/raw.tsv" --raw-source --start 2026-10-09 --end 2026-10-13 --execute
[ "$LAST_RC" -eq 2 ] || fail 'Raw mode accepted mixed current/historical dates'
assert_no_execute_effects "$CASE_DIR" "$CASE_DIR/persistent/raw.tsv"

new_case raw-daily-check
invoke_daily "$CASE_DIR" check --raw-source
[ "$LAST_RC" -eq 0 ] || fail "Raw daily check failed rc=$LAST_RC"
grep -q -- '--eod-raw-check-session-dates 2026-10-13' "$CASE_DIR/docker.log" || fail 'Raw check was not invoked'
! grep -q db-migrate "$CASE_DIR/docker.log" || fail 'Raw check used canonical DB frontier'
[ ! -s "$CASE_DIR/worker-argv.log" ] || fail 'Raw check invoked provider worker'
! test -f "$CASE_DIR/state/kis-daily-2026-10-13.tsv" || fail 'Raw check wrote progress'

new_case raw-daily-execute
invoke_daily "$CASE_DIR" execute --raw-source
[ "$LAST_RC" -eq 0 ] || fail "Raw daily execute failed rc=$LAST_RC"
expect_worker_log '2026-10-13' "$CASE_DIR/worker-argv.log"
grep -q $'\tRAW_COMMITTED\t' "$CASE_DIR/state/kis-daily-2026-10-13.tsv" || fail 'Raw daily did not persist source progress'
! grep -q db-migrate "$CASE_DIR/docker.log" || fail 'Raw execution used DB frontier'

new_case raw-daily-existing
FIXTURE_WORKER_MODE=raw-existing
invoke_daily "$CASE_DIR" execute --raw-source
[ "$LAST_RC" -eq 0 ] || fail 'existing Raw source was not reused'
[ ! -s "$CASE_DIR/worker-argv.log" ] || fail 'existing Raw source triggered provider worker'

new_case raw-daily-early
FIXTURE_NOW_HMS=09:00:00
invoke_daily "$CASE_DIR" execute --raw-source
[ "$LAST_RC" -eq 0 ] || fail 'early Raw run did not defer'
[ ! -s "$CASE_DIR/docker.log" ] || fail 'early Raw run invoked Docker'

new_case raw-daily-closed
FIXTURE_CALENDAR_DATES=2026-10-09,2026-10-12
invoke_daily "$CASE_DIR" execute --raw-source
[ "$LAST_RC" -eq 0 ] || fail 'closed-day Raw run did not skip'
[ ! -s "$CASE_DIR/docker.log" ] || fail 'closed-day Raw run invoked Docker'

new_case supply-requires-raw
invoke_daily "$CASE_DIR" execute --action-supply
[ "$LAST_RC" -ne 0 ] || fail 'supplier accepted legacy mode'
[ ! -s "$CASE_DIR/docker.log" ] || fail 'invalid supplier mode invoked Docker'

new_case supply-missing-config
invoke_daily "$CASE_DIR" execute --raw-source --action-supply
[ "$LAST_RC" -eq 2 ] || fail 'supplier accepted missing protected configuration'
[ ! -s "$CASE_DIR/docker.log" ] || fail 'missing supplier configuration invoked Docker'

new_case supply-check
printf '%s\n' 'RESEARCH_R2_ACTION_SUPPLY_CONFIG_HEX=7b2266697874757265223a747275657d' >>"$CASE_DIR/production.env"
invoke_daily "$CASE_DIR" check --raw-source --action-supply
[ "$LAST_RC" -eq 0 ] || fail 'supplier check failed'
grep -q -- '--r2-action-supply --check' "$CASE_DIR/docker.log" || fail 'supplier check mode missing'
[ ! -s "$CASE_DIR/worker-argv.log" ] || fail 'supplier check invoked source capture'

new_case supply-after-capture
printf '%s\n' 'RESEARCH_R2_ACTION_SUPPLY_CONFIG_HEX=7b2266697874757265223a747275657d' >>"$CASE_DIR/production.env"
invoke_daily "$CASE_DIR" execute --raw-source --action-supply
[ "$LAST_RC" -eq 0 ] || fail 'supplier after Raw capture failed'
python3 - "$CASE_DIR/docker.log" <<'PY'
import sys
text = open(sys.argv[1]).read()
assert text.index("--eod-raw-session-dates") < text.index("--r2-action-supply")
assert "--r2-action-supply --check" not in text
PY

new_case supply-after-existing-raw
printf '%s\n' 'RESEARCH_R2_ACTION_SUPPLY_CONFIG_HEX=7b2266697874757265223a747275657d' >>"$CASE_DIR/production.env"
FIXTURE_WORKER_MODE=raw-existing
invoke_daily "$CASE_DIR" execute --raw-source --action-supply
[ "$LAST_RC" -eq 0 ] || fail 'existing Raw suppressed supplier'
grep -q -- '--r2-action-supply' "$CASE_DIR/docker.log" || fail 'existing Raw skipped supplier'
[ ! -s "$CASE_DIR/worker-argv.log" ] || fail 'existing Raw recaptured source'

new_case supply-source-failure
printf '%s\n' 'RESEARCH_R2_ACTION_SUPPLY_CONFIG_HEX=7b2266697874757265223a747275657d' >>"$CASE_DIR/production.env"
FIXTURE_WORKER_MODE=permanent
invoke_daily "$CASE_DIR" execute --raw-source --action-supply
[ "$LAST_RC" -ne 0 ] || fail 'failed Raw source was accepted'
! grep -q -- '--r2-action-supply' "$CASE_DIR/docker.log" || fail 'supplier ran after source failure'

new_case supply-failure-propagated
printf '%s\n' 'RESEARCH_R2_ACTION_SUPPLY_CONFIG_HEX=7b2266697874757265223a747275657d' >>"$CASE_DIR/production.env"
FIXTURE_WORKER_MODE=supply-failure
invoke_daily "$CASE_DIR" check --raw-source --action-supply
[ "$LAST_RC" -eq 2 ] || fail 'supplier failure was swallowed'

echo 'BACKFILL_SELF_TEST: PASS (legacy selection/resume, seven Raw source and seven supplier integration cases; fake host only)'
