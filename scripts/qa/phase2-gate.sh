#!/usr/bin/env bash
# phase2-gate.sh - Phase 2 Paper and recovery release gate (plan Todo 35).
# POSIX/CI twin of scripts/qa/phase2-gate.ps1.
#
# Assembles the Phase 2 evidence bundle and emits ONE machine-readable verdict:
#
#   VERDICT: APPROVED
#   VERDICT: OWNER_ONLY_BLOCKED_EXTERNAL
#
# APPROVED requires EVERY Phase 2 check to pass AND an active data entitlement
# AND five-user Phase 1 evidence. Phase 2 can be proven Owner-only while Phase 1
# is externally blocked (the KRX written-rights contract and the Auth0 tenant),
# but that state is NOT a release: Member production stays disabled and the gate
# says so in its verdict rather than in a footnote. There is deliberately no
# flag, override, or environment variable that turns a blocked run into an
# APPROVED one — a gate with an escape hatch is not a gate.
#
# Checks (design §17.1 E2E + §16 fail-closed; requirements 500-507, 448-475):
#   P1  backtest-vs-Paper signal parity on identical lineage
#   P2  AT-07 per-user isolation (accounts, targets, notifications)
#   P3  sell-before-buy ordering, cost/ledger reconciliation
#   P4  scheduler restart idempotency (no duplicate fills)
#   P5  notification delivery outcomes recorded (including outages)
#   P6  PITR + file restore verified into an isolated target
#   P7  Phase 2 fault suite
#   E1  data entitlement ACTIVE          (external; gates APPROVED)
#   E2  five-user Phase 1 evidence       (external; gates APPROVED)
#
# Exit codes: 0 = a verdict was emitted (including OWNER_ONLY_BLOCKED_EXTERNAL,
# which is a legitimate outcome, not an error); 2 = the gate could not run.
#
# Usage: scripts/qa/phase2-gate.sh [--keep-db]
# Twin: scripts/qa/phase2-gate.ps1
set -u

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
evidence_dir="$root/.omo/evidence"
transcript_dir="$evidence_dir/task-35-transcripts"
ev_path="$evidence_dir/task-35-lagrange-station-implementation.json"
qa_compose="$root/deploy/qa/qa-db.compose.yml"
data_rights_dir="$root/configs/data-rights"
keep_db=0
qa_port="${LAGRANGE_QA_DB_PORT:-55432}"

while [ $# -gt 0 ]; do
  case "$1" in
    --keep-db) keep_db=1; shift ;;
    *) echo "USAGE: $0 [--keep-db]" >&2; exit 2 ;;
  esac
done

command -v docker >/dev/null 2>&1 || { echo "ENV ERROR: docker not found on PATH" >&2; exit 2; }
command -v cargo  >/dev/null 2>&1 || { echo "ENV ERROR: cargo not found on PATH" >&2; exit 2; }
docker version --format '{{.Server.Version}}' >/dev/null 2>&1 || {
  echo "ENV ERROR: Docker engine is unavailable or this user cannot access its socket" >&2
  exit 2
}
mkdir -p "$evidence_dir" "$transcript_dir"

hostpath() {
  if command -v cygpath >/dev/null 2>&1; then cygpath -w "$1"; else printf '%s' "$1"; fi
}
dkr() { MSYS_NO_PATHCONV=1 MSYS2_ARG_CONV_EXCL='*' docker "$@"; }
qc() { dkr compose -p lagrange-qa -f "$(hostpath "$qa_compose")" "$@"; }

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target}"
export DATABASE_URL="postgres://postgres:lagrange@127.0.0.1:${qa_port}/postgres"

checks=""
add_check() { # add_check <id> <name> <result> <detail>
  checks="$checks
$1 $2 $3 $4"
  printf 'CHECK %-3s %-26s = %-18s %s\n' "$1" "$2" "$3" "$4"
}

# run_check <id> <name> <transcript> <cargo args...>
# PASS only when cargo exits 0 AND at least one test ran. A filter that selects
# nothing exits 0 with "0 passed"; recording that as evidence would let the gate
# approve a release on the strength of tests that never executed.
run_check() {
  local id="$1" name="$2" file="$3" t="$transcript_dir/$3"; shift 3
  ( cd "$root" && cargo test "$@" -- --test-threads=2 ) >"$t" 2>&1
  local rc=$? ran
  ran="$(grep -Eo '^test result: ok\. [0-9]+ passed' "$t" | grep -Eo '[0-9]+' | awk '{s+=$1} END {print s+0}')"
  if [ "$rc" -ne 0 ]; then
    add_check "$id" "$name" FAIL "cargo exit $rc ($file)"
  elif [ "${ran:-0}" -eq 0 ]; then
    add_check "$id" "$name" FAIL "0 tests selected ($file)"
  else
    add_check "$id" "$name" PASS "$ran assertion(s)"
  fi
}

cleanup() { [ "$keep_db" -eq 1 ] || qc down -v --remove-orphans >/dev/null 2>&1 || true; }
trap cleanup EXIT

echo "== Phase 2 release gate =="
if ! qc up -d --wait qa-db >/dev/null 2>&1; then
  echo "ENV ERROR: the QA database did not become healthy" >&2
  exit 2
fi

# --- P1 backtest-vs-Paper signal parity ---------------------------------------
# The plan's core Phase 2 claim: one promoted strategy uses IDENTICAL signal
# rules in backtest and Paper. paper_parity proves the comparison itself
# (including that a changed lineage is NOT_COMPARABLE rather than a false
# match); http_paper proves the served report.
run_check P1 paper-parity p1-paper-parity.txt -p result-model paper_parity
run_check P1b parity-route p1b-parity-route.txt -p api-server --test http_paper

# --- P2 AT-07 isolation --------------------------------------------------------
run_check P2 at07-isolation p2-isolation.txt -p api-server --test paper_notifications two_members
run_check P2b tenancy-rls p2b-tenancy.txt -p api-server --test tenancy_rls

# --- P3 ordering, cost, ledger reconciliation ----------------------------------
run_check P3 sell-before-buy p3-sell-before-buy.txt -p portfolio-model --test paper_flow sells_before_buys
run_check P3b ledger-reconciliation p3b-ledger.txt -p portfolio-model --test ledger

# --- P4 scheduler restart idempotency ------------------------------------------
run_check P4 restart-idempotency p4-restart.txt -p portfolio-model --test paper_flow crash
run_check P4b target-claim-once p4b-claim.txt -p api-server --test paper_scheduler claimed_twice

# --- P5 notification delivery outcomes -----------------------------------------
run_check P5 delivery-outcomes p5-notifications.txt -p api-server --test notifications

# --- P6 PITR + file restore ----------------------------------------------------
# Requires a real restore verdict from Todo 33. The gate reads the VERDICT, it
# does not re-derive it: a gate that recomputed its own evidence could not
# detect that the evidence was never produced.
p6_verdict="${LAGRANGE_PHASE2_RESTORE_VERDICT:-}"
if [ -n "$p6_verdict" ] && [ -f "$p6_verdict" ]; then
  if grep -q '"verdict": *"SUCCESS"' "$p6_verdict"; then
    lsn="$(sed -n 's/.*"recovery_target_lsn": *"\([^"]*\)".*/\1/p' "$p6_verdict" | head -n1)"
    cp "$p6_verdict" "$transcript_dir/p6-restore-verdict.json" 2>/dev/null || true
    add_check P6 pitr-restore PASS "verified at LSN ${lsn:-unknown}"
  else
    add_check P6 pitr-restore FAIL "restore verdict is not SUCCESS"
  fi
else
  add_check P6 pitr-restore MISSING_EVIDENCE \
    "set LAGRANGE_PHASE2_RESTORE_VERDICT to a restore-and-verify verdict JSON"
fi

# --- P7 Phase 2 fault suite -----------------------------------------------------
p7_out="$transcript_dir/p7-fault-suite.txt"
if bash "$root/scripts/qa/failure-suite.sh" --phase 2 --keep-db >"$p7_out" 2>&1; then
  if grep -q '^VERDICT: PHASE2_FAULTS_PASSED' "$p7_out"; then
    add_check P7 fault-suite PASS "all Phase 2 faults fail closed"
  else
    # PHASE2_FAULTS_INCOMPLETE exits 0 by design; a partial fault run must not
    # be quotable as gate evidence.
    add_check P7 fault-suite MISSING_EVIDENCE "fault suite incomplete (see p7-fault-suite.txt)"
  fi
else
  add_check P7 fault-suite FAIL "fault suite nonzero (see p7-fault-suite.txt)"
fi

# --- E1 data entitlement (external) ---------------------------------------------
# Phase 1's blocker, re-checked here because APPROVED depends on it. The written
# rights artifact must exist AND be ACTIVE with a real document hash; a
# placeholder with a zeroed hash is not an entitlement.
e1_state="BLOCKED_EXTERNAL"
e1_detail="no ACTIVE written-rights artifact in configs/data-rights/"
for f in "$data_rights_dir"/*.json; do
  [ -f "$f" ] || continue
  if grep -q '"status" *: *"ACTIVE"' "$f" && ! grep -qE '"document_sha256" *: *"0{64}"' "$f"; then
    e1_state="PASS"
    e1_detail="ACTIVE written rights: $(basename "$f")"
    break
  fi
done
add_check E1 data-entitlement "$e1_state" "$e1_detail"

# --- E2 five-user Phase 1 evidence (external) -----------------------------------
e2_state="BLOCKED_EXTERNAL"
e2_detail="Phase 1 gate has not emitted APPROVED"
p1_ev="$evidence_dir/task-28-lagrange-station-implementation.json"
if [ -f "$p1_ev" ] && grep -q '"verdict" *: *"APPROVED"' "$p1_ev"; then
  e2_state="PASS"
  e2_detail="Phase 1 APPROVED with five-user evidence"
fi
add_check E2 phase1-five-user "$e2_state" "$e2_detail"

# --- verdict ---------------------------------------------------------------------
hard_fail="$(printf '%s' "$checks" | grep -c ' FAIL ' || true)"
missing="$(printf '%s' "$checks" | grep -c ' MISSING_EVIDENCE ' || true)"
blocked="$(printf '%s' "$checks" | grep -c ' BLOCKED_EXTERNAL ' || true)"

if [ "$hard_fail" -eq 0 ] && [ "$missing" -eq 0 ] && [ "$blocked" -eq 0 ]; then
  verdict="APPROVED"
else
  verdict="OWNER_ONLY_BLOCKED_EXTERNAL"
fi

echo
printf 'VERDICT: %s\n' "$verdict"
printf 'EVIDENCE: %s\n' "$ev_path"
if [ "$verdict" != "APPROVED" ]; then
  printf 'NOT_APPROVED_BECAUSE:\n'
  printf '%s\n' "$checks" | sed '/^$/d' | grep -v ' PASS ' | sed 's/^/  - /'
  printf 'MEMBER_PRODUCTION: DISABLED\n'
  printf 'NOTE: OWNER_ONLY_BLOCKED_EXTERNAL means the Phase 2 Paper and recovery invariants may hold while the EXTERNAL Phase 1 preconditions do not. Owner-only work continues; Member KR-derived surfaces stay denied; this state must never be reported as a release.\n'
else
  printf 'MEMBER_PRODUCTION: ELIGIBLE\n'
fi

python3 - "$ev_path" "$verdict" "$checks" "$transcript_dir" <<'PYEOF'
import json, sys, datetime
path, verdict, checks, tdir = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
items = []
for line in checks.splitlines():
    parts = line.split()
    if len(parts) >= 4:
        items.append({"id": parts[0], "name": parts[1], "result": parts[2],
                      "detail": " ".join(parts[3:])})
summary = {
    "gate": "phase2", "task": 35, "verdict": verdict,
    "member_production": "ELIGIBLE" if verdict == "APPROVED" else "DISABLED",
    "emitted_at": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "checks": items,
    "evidence_dir": tdir,
    "approval_rule": "APPROVED requires every Phase 2 check to pass AND an ACTIVE data entitlement AND five-user Phase 1 evidence. There is no override.",
}
with open(path, "w", encoding="utf-8") as f:
    json.dump(summary, f, indent=2, ensure_ascii=False)
PYEOF
printf 'EVIDENCE_WRITTEN: %s\n' "$ev_path"
exit 0
