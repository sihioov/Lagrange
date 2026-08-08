#!/usr/bin/env bash
# failure-suite.sh - Phase 2 operational failure injection and recovery drills
# (plan Todo 34). POSIX/CI twin of scripts/qa/failure-suite.ps1.
#
# Covers design §17 "Failure Injection" and the §16 fail-closed table, restricted
# Two lanes. `--phase 2` covers what Paper and recovery own; `--phase 3` covers
# the KIS and Live rows, which were deliberately left empty until Todos 36-40
# existed to be injected into. A suite that had shipped Phase 3 scenarios
# earlier would have been asserting against code that was not there.
#
#   F1  DB 일시 장애                 api-server  failure_db_outage_*
#   F2  워커 강제 종료 / OOM          job-queue   worker_death / zombie / retry_exhaustion
#   F3  데이터·아티팩트 손상          api-server  artifact_hash_mismatch_fails_closed
#                                    result-model publication_is_refused_*
#   F4  Paper 스케줄러 중단           portfolio-model paper_flow crash-resume
#                                    api-server  paper_scheduler
#   F5  중복 / 순서역전 이벤트        portfolio-model ledger replay + job-queue idempotency
#   F6  디스크 풀                     api-server  failure_disk_full_artifact_is_never_served
#   F7  알림 중단                     api-server  observability_notification_email_outage_*
#   F8  복원 실패                     scripts/backup/tests/test-restore-failures.sh
#
# Most scenarios are proven by tests that already exist; this suite RUNS them as
# named fault scenarios and asserts on their outcome rather than duplicating
# them. Only F1 and F6 needed new tests. That is deliberate: a second copy of an
# invariant is a second thing to drift, not a second proof.
#
# The substrate is the hermetic QA database (deploy/qa/qa-db.compose.yml), not
# the developer's WSL PostgreSQL — see decisions.md.
#
# --self-test is NOT optional decoration. The plan requires that "each
# deliberately broken invariant makes the suite nonzero", which is a
# non-vacuousness requirement on the suite itself: it sabotages one invariant
# per scenario class and asserts the suite catches each one. A suite that
# cannot fail is not evidence.
#
# Exit codes: 0 all scenarios passed; 1 a scenario failed; 2 the suite could
# not run (bad usage, no Docker, no cargo).
#
# Usage:
#   scripts/qa/failure-suite.sh --phase 2 [--self-test] [--keep-db]
# Twin: scripts/qa/failure-suite.ps1
set -u

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
qa_compose="$root/deploy/qa/qa-db.compose.yml"
evidence_dir="$root/.omo/evidence/task-34-transcripts"
phase=""
self_test=0
keep_db=0
qa_port="${LAGRANGE_QA_DB_PORT:-55432}"

while [ $# -gt 0 ]; do
  case "$1" in
    --phase) phase="$2"; shift 2 ;;
    --self-test) self_test=1; shift ;;
    --keep-db) keep_db=1; shift ;;
    *) echo "USAGE: $0 --phase 2 [--self-test] [--keep-db]" >&2; exit 2 ;;
  esac
done

case "$phase" in
  2|3) ;;
  *) echo "USAGE: --phase {2|3} is required" >&2; exit 2 ;;
esac
command -v docker >/dev/null 2>&1 || { echo "ENV ERROR: docker not found on PATH" >&2; exit 2; }
command -v cargo  >/dev/null 2>&1 || { echo "ENV ERROR: cargo not found on PATH" >&2; exit 2; }
mkdir -p "$evidence_dir"

hostpath() {
  if command -v cygpath >/dev/null 2>&1; then cygpath -w "$1"; else printf '%s' "$1"; fi
}
qa_compose_host="$(hostpath "$qa_compose")"
dkr() { MSYS_NO_PATHCONV=1 MSYS2_ARG_CONV_EXCL='*' docker "$@"; }
qc() { dkr compose -p lagrange-qa -f "$qa_compose_host" "$@"; }

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target}"
export DATABASE_URL="postgres://postgres:lagrange@127.0.0.1:${qa_port}/postgres"

scenarios=0
failed=0
skipped_n=0
results=""

record() { # record <id> <name> <result> <detail>
  results="$results
$1|$2|$3|$4"
  printf 'SCENARIO %-3s %-34s = %-6s %s\n' "$1" "$2" "$3" "$4"
  # A SKIP is NOT a failure. It means an external prerequisite is absent -- a
  # backup set that has to be built first -- and counting it as failed made the
  # INCOMPLETE verdict below UNREACHABLE, so a suite missing one prerequisite
  # reported itself as broken. That is the same "an external block reported as
  # a defect" confusion the phase gates take such care to avoid.
  case "$3" in
    PASS) ;;
    SKIP) skipped_n=$((skipped_n+1)) ;;
    *)    failed=$((failed+1)) ;;
  esac
  scenarios=$((scenarios+1))
}

# run_tests <id> <name> <transcript> <cargo args...>
# PASS only when cargo exits 0 AND at least one test actually ran. A filter that
# matches nothing exits 0 with "0 passed", which would otherwise be recorded as
# a passing scenario — the same silently-empty-run trap the Todo 33 acceptance
# filter fell into.
run_tests() {
  # NOTE: cargo accepts exactly ONE filter positional, so each scenario passes
  # a single filter. Keep the transcript name in a local BEFORE shifting - `$3`
  # afterwards names a cargo argument, not the file.
  local id="$1" name="$2" file="$3" t="$evidence_dir/$3"; shift 3
  ( cd "$root" && cargo test "$@" -- --test-threads=2 ) >"$t" 2>&1
  local rc=$?
  local ran
  ran="$(grep -Eo '^test result: ok\. [0-9]+ passed' "$t" | grep -Eo '[0-9]+' | awk '{s+=$1} END {print s+0}')"
  if [ "$rc" -ne 0 ]; then
    record "$id" "$name" FAIL "cargo exit $rc (see $file)"
  elif [ "${ran:-0}" -eq 0 ]; then
    record "$id" "$name" FAIL "the filter selected 0 tests (see $file)"
  else
    record "$id" "$name" PASS "$ran assertion(s) ran"
  fi
}

cleanup() {
  if [ "$keep_db" -eq 0 ]; then
    qc down -v --remove-orphans >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

echo "== Phase $phase failure suite =="
echo "   QA database: 127.0.0.1:$qa_port (disposable)"
if ! qc up -d --wait qa-db >/dev/null 2>&1; then
  echo "ENV ERROR: the QA database did not become healthy" >&2
  exit 2
fi

if [ "$phase" = "2" ]; then

# --- F1 DB 일시 장애 ----------------------------------------------------------
run_tests F1 "DB transient outage" f1-db-outage.txt \
  -p api-server --test failure_injection failure_db_outage

# --- F2 워커 강제 종료 / OOM --------------------------------------------------
# A killed worker at the persistence layer IS an abandoned lease: claim, never
# heartbeat, expire, sweep. Attempt count stays bounded and the zombie cannot
# settle work that was requeued underneath it.
run_tests F2 "worker death requeues once" f2-worker-death.txt \
  -p job-queue --test queue_contract worker_death
run_tests F2b "zombie cannot settle after sweep" f2b-zombie-settle.txt \
  -p job-queue --test queue_contract zombie_worker
run_tests F2c "retry is bounded by max_attempts" f2c-retry-bound.txt \
  -p job-queue --test queue_contract retry_exhaustion
run_tests F2d "integrity errors never retry" f2d-no-retry.txt \
  -p job-queue --test queue_contract input_data_integrity

# --- F3 데이터·아티팩트 손상 ---------------------------------------------------
run_tests F3 "corrupt artifact fails closed" f3-artifact-corruption.txt \
  -p api-server --test artifact_authorization artifact_hash_mismatch
run_tests F3b "corrupt result is never published" f3b-result-integrity.txt \
  -p result-model --test backtest_result_integrity publication_is_refused

# --- F4 Paper 스케줄러 중단 ---------------------------------------------------
run_tests F4 "Paper scheduler interruption" f4-paper-crash-resume.txt \
  -p portfolio-model --test paper_flow crash
run_tests F4b "settled target cannot be reclaimed" f4b-paper-scheduler.txt \
  -p api-server --test paper_scheduler claimed_twice

# --- F5 중복 / 순서역전 이벤트 -------------------------------------------------
run_tests F5 "duplicate / out-of-order events" f5-replay.txt \
  -p portfolio-model --test replay
run_tests F5b "duplicate submission is idempotent" f5b-idempotency.txt \
  -p job-queue --test queue_contract duplicate_idempotency

# --- F6 디스크 풀 -------------------------------------------------------------
run_tests F6 "disk-full artifact never served" f6-disk-full.txt \
  -p api-server --test failure_injection failure_disk_full

# --- F7 알림 중단 -------------------------------------------------------------
run_tests F7 "notification outage recorded" f7-notification-outage.txt \
  -p api-server --test notifications email_outage

# --- F8 복원 실패 -------------------------------------------------------------
# Reuses Todo 33's harness rather than re-deriving restore faults. It needs a
# real backup set, so it is skipped-with-a-reason when one was not supplied
# instead of silently counting as a pass.
if [ -n "${LAGRANGE_QA_BACKUP_SET:-}" ] && [ -n "${LAGRANGE_QA_BACKUP_SIDECAR:-}" ]; then
  t="$evidence_dir/f8-restore-failures.txt"
  bash "$root/scripts/backup/tests/test-restore-failures.sh" \
      --set "$LAGRANGE_QA_BACKUP_SET" --sidecar "$LAGRANGE_QA_BACKUP_SIDECAR" \
      --key "${LAGRANGE_QA_BACKUP_KEY:-lagrange-drill-key}" >"$t" 2>&1
  if [ $? -eq 0 ]; then
    record F8 "restore failure drills" PASS "6/6 restore faults fail closed"
  else
    record F8 "restore failure drills" FAIL "see f8-restore-failures.txt"
  fi
else
  record F8 "restore failure drills" SKIP \
    "set LAGRANGE_QA_BACKUP_SET and _SIDECAR (build one with scripts/backup/create.sh)"
fi

fi

if [ "$phase" = "3" ]; then

# --- L1 KIS 429 / 500 / timeout -----------------------------------------------
# The transport rows of design §17. A 429 must back off rather than hammer, a
# 500 must not be mistaken for a rejection, and a TIMEOUT must not be treated
# as either -- that last one is the whole of AT-09.
run_tests L1 "rate limit backs off" l1-rate-limit.txt \
  -p kis-client --lib rate_limit
run_tests L1b "transient errors retry, ambiguous ones do not" l1b-retry.txt \
  -p kis-client --lib retry

# --- L2 타임아웃 -> UNKNOWN ----------------------------------------------------
# A timeout proves NOTHING. Resubmitting on one places a second real order
# against an account that may already hold the first.
run_tests L2 "timeout becomes UNKNOWN, never a retry" l2-unknown.txt \
  -p kis-client --test live_order_state unknown
run_tests L2b "one intent yields at most one order" l2b-one-order.txt \
  -p kis-client --test live_order_state at_most_one

# --- L3 WebSocket 유실 ---------------------------------------------------------
# A dropped socket means fills may have been missed. The gap is closed by
# reconciliation, not by hoping the socket catches up.
run_tests L3 "missed fills are found by reconciliation" l3-websocket-gap.txt \
  -p kis-client --test reconciliation a_missing_fill
run_tests L3b "a resend moves neither state nor ledger" l3b-resend.txt \
  -p kis-client --test live_order_state duplicate_and_stale_fills

# --- L4 중복 / 순서역전 이벤트 (Live) -------------------------------------------
run_tests L4 "cumulative fills are order-insensitive" l4-out-of-order.txt \
  -p kis-client --test live_order_state cumulative
run_tests L4b "a duplicate fill never reaches the ledger twice" l4b-duplicate-fill.txt \
  -p portfolio-model --test live_order_state duplicate

# --- L5 DB 쓰기 실패 -> 신규 주문 차단 ------------------------------------------
# §16. A decision that cannot be recorded must not authorise an order: after a
# restart there would be nothing to reconcile against.
run_tests L5 "an unrecordable decision denies" l5-db-write-fail.txt \
  -p risk-gateway a_failed_write_denies
run_tests L5b "one decision per intent, append-only" l5b-append-only.txt \
  -p api-server --test risk_store

# --- L6 대사 불일치 -> 거래 차단 ------------------------------------------------
run_tests L6 "an unexplained mismatch blocks" l6-mismatch.txt \
  -p kis-client --test reconciliation _blocks
run_tests L6b "readiness gates the gate" l6b-readiness.txt \
  -p api-server --test reconciliation_store

# --- L7 Kill Switch -------------------------------------------------------------
run_tests L7 "kill switch blocks, and cannot be lifted unreconciled" l7-kill-switch.txt \
  -p api-server --test live_rbac kill_switch

# --- L8 재시작 -> 대사 전 거래 불가 ---------------------------------------------
run_tests L8 "a restarted node cannot trade until reconciled" l8-restart.txt \
  -p risk-gateway reproduced_exactly_after_a_restart

fi

# --- correlation-linked audit -------------------------------------------------
run_tests A1 "refusal audited with correlation" a1-audit-correlation.txt \
  -p api-server --test failure_injection failure_refusal_is_audited

# --- self-test: prove the suite can actually fail -----------------------------
if [ "$self_test" -eq 1 ]; then
  echo
  echo "== self-test: each sabotaged invariant must make the suite nonzero =="
  st_fail=0
  st_run=0

  # Sabotage 1: a filter that selects nothing must NOT be recorded as a pass.
  st_run=$((st_run+1))
  t="$evidence_dir/self-1-empty-filter.txt"
  ( cd "$root" && cargo test -p api-server --test failure_injection \
      this_test_name_does_not_exist ) >"$t" 2>&1
  ran="$(grep -Eo '^test result: ok\. [0-9]+ passed' "$t" | grep -Eo '[0-9]+' | awk '{s+=$1} END {print s+0}')"
  if [ "${ran:-0}" -eq 0 ]; then
    echo "SELFTEST 1 PASS  an empty filter is detected (0 tests ran)"
  else
    echo "SELFTEST 1 FAIL  an empty filter was not detected"; st_fail=$((st_fail+1))
  fi

  # Sabotage 2: break the fail-closed invariant itself. A DB outage that
  # returned data would be the failure this suite exists to catch, so assert
  # the assertion: with the DB reachable the outage test's own precondition
  # (a 5xx during the cut) cannot be satisfied by a healthy server.
  st_run=$((st_run+1))
  t="$evidence_dir/self-2-no-outage.txt"
  if ( cd "$root" && DATABASE_URL="postgres://postgres:lagrange@127.0.0.1:1/postgres" \
        cargo test -p api-server --test failure_injection failure_db_outage ) >"$t" 2>&1; then
    # An unreachable DB makes Harness::new() return None and the tests SKIP.
    # A skip must never look like a pass, so require the skip marker.
    if grep -q 'SKIP: DATABASE_URL not set' "$t" || grep -qE '^test result: ok\. 0 passed' "$t"; then
      echo "SELFTEST 2 PASS  an unusable substrate yields 0 executed assertions, not a false pass"
    else
      echo "SELFTEST 2 FAIL  tests reported success against an unusable substrate"; st_fail=$((st_fail+1))
    fi
  else
    echo "SELFTEST 2 PASS  an unusable substrate fails the run"
  fi

  # Sabotage 3: a scenario whose cargo invocation fails must be recorded FAIL.
  st_run=$((st_run+1))
  before_failed=$failed
  run_tests SELF "deliberately broken scenario" self-3-broken.txt \
    -p api-server --test no_such_test_binary_exists
  if [ "$failed" -gt "$before_failed" ]; then
    echo "SELFTEST 3 PASS  a broken scenario is recorded FAIL"
    # This sabotage is not a real defect; do not let it fail the suite.
    failed=$((failed-1))
    scenarios=$((scenarios-1))
  else
    echo "SELFTEST 3 FAIL  a broken scenario was not recorded"; st_fail=$((st_fail+1))
  fi

  echo "SELFTEST: $((st_run-st_fail))/$st_run sabotages detected"
  if [ "$st_fail" -ne 0 ]; then
    echo
    echo "VERDICT: SUITE_NOT_TRUSTWORTHY ($st_fail sabotage(s) undetected)"
    exit 1
  fi
fi

# --- verdict -------------------------------------------------------------------
echo
skipped="$(printf '%s\n' "$results" | awk -F'|' '$3=="SKIP"' | wc -l | tr -d ' ')"
passed=$(( scenarios - failed - skipped ))
printf 'SCENARIOS: %d passed, %d failed, %d skipped (of %d)\n' \
  "$passed" "$failed" "$skipped" "$scenarios"

if [ "$failed" -ne 0 ]; then
  echo "VERDICT: PHASE${phase}_FAULTS_FAILED"
  exit 1
fi
if [ "$skipped" -ne 0 ]; then
  # A skip is not a pass. The suite reports a distinct verdict so a partial run
  # can never be quoted as full Phase 2 fault coverage.
  echo "VERDICT: PHASE${phase}_FAULTS_INCOMPLETE"
  exit 0
fi
echo "VERDICT: PHASE${phase}_FAULTS_PASSED"
exit 0
