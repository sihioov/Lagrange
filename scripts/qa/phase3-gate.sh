#!/usr/bin/env bash
# phase3-gate.sh - Phase 3 Live release gate (plan Todo 42).
# POSIX/CI twin of scripts/qa/phase3-gate.ps1.
#
# Assembles the Phase 3 evidence bundle and emits ONE machine-readable verdict:
#
#   VERDICT: APPROVED
#   VERDICT: BLOCKED_EXTERNAL_CREDENTIALS
#   VERDICT: DENIED
#
# The distinction between the last two is the point of this file. DENIED means
# a Live safety invariant does not hold: something is WRONG and must be fixed.
# BLOCKED_EXTERNAL_CREDENTIALS means every invariant we can prove does hold,
# but the evidence that can only come from a real broker account -- one bounded
# order, actually placed, actually reconciled -- does not exist. Nothing is
# broken; the proof is simply not obtainable here. Reporting the second as the
# first would send someone hunting a bug that is not there, and reporting
# either as APPROVED would put real money behind untested code.
#
# There is deliberately NO flag, environment variable, or override that turns a
# blocked or denied run into an APPROVED one. A gate with an escape hatch is
# not a gate, and this is the gate standing between this system and a live
# brokerage account.
#
# Checks (design §6.12, §16, §17; requirements FR-LIVE-001..006, AT-08, AT-09):
#   L1  AT-08 stale data blocks, with reason, metric, and audit
#   L2  AT-09 timeout -> UNKNOWN -> lookup, never a duplicate order
#   L3  no-Member-Live boundary (absent, not hidden; 404, never 403)
#   L4  persisted gatekeeper survives restart (decision replays identically)
#   L5  reconciliation: one definition of green, blocked until it holds
#   L6  kill switch: engage unconditional, disengage needs green + fresh MFA
#   L7  order intent idempotency (a retry never places a second order)
#   L8  live node isolation, lifecycle, and cancel policy
#   L9  executable runbooks, both shells, with machine assertions
#   L10 secret and account redaction (no credential material at rest)
#   L11 migration contract (append-only decisions, one per intent)
#   X1  real broker credentials         (external; gates APPROVED)
#   X2  one bounded low-value order     (external; gates APPROVED)
#
# Exit codes: 0 = a verdict was emitted (including BLOCKED_EXTERNAL_CREDENTIALS,
# which is a legitimate outcome, not an error); 2 = the gate could not run.
#
# Usage: scripts/qa/phase3-gate.sh [--keep-db]
# Twin: scripts/qa/phase3-gate.ps1
set -u

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
evidence_dir="$root/.omo/evidence"
transcript_dir="$evidence_dir/task-42-transcripts"
ev_path="$evidence_dir/task-42-lagrange-station-implementation.json"
qa_compose="$root/deploy/qa/qa-db.compose.yml"
keep_db=0
qa_port="${LAGRANGE_QA_DB_PORT:-55432}"

while [ $# -gt 0 ]; do
  case "$1" in
    --keep-db) keep_db=1; shift ;;
    *) echo "USAGE: $0 [--keep-db]" >&2; exit 2 ;;
  esac
done

command -v cargo >/dev/null 2>&1 || { echo "ENV ERROR: cargo not found on PATH" >&2; exit 2; }
command -v uv    >/dev/null 2>&1 || { echo "ENV ERROR: uv not found on PATH" >&2; exit 2; }
mkdir -p "$evidence_dir" "$transcript_dir"

hostpath() {
  if command -v cygpath >/dev/null 2>&1; then cygpath -w "$1"; else printf '%s' "$1"; fi
}
dkr() { MSYS_NO_PATHCONV=1 MSYS2_ARG_CONV_EXCL='*' docker "$@"; }
qc() { dkr compose -p lagrange-qa -f "$(hostpath "$qa_compose")" "$@"; }

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target}"
export DATABASE_URL="postgres://postgres:lagrange@127.0.0.1:${qa_port}/postgres"
export REPO_ROOT="$root"

checks=""
add_check() { # add_check <id> <name> <result> <detail>
  checks="$checks
$1 $2 $3 $4"
  printf 'CHECK %-4s %-26s = %-26s %s\n' "$1" "$2" "$3" "$4"
}

# run_check <id> <name> <transcript> <cargo args...>
# PASS only when cargo exits 0 AND at least one test ran. A filter that selects
# nothing exits 0 with "0 passed"; recording that as evidence would let this
# gate approve a LIVE release on the strength of tests that never executed.
# Todo 33 shipped exactly that mistake, and Todo 39 nearly repeated it.
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

echo "== Phase 3 Live release gate =="
# The QA database is REQUIRED, and a gate that cannot reach it must not emit a
# verdict.
#
# This block used to be `if command -v docker; then qc up ... || true; fi`, and
# both halves were wrong. `command -v docker` succeeds while Docker Desktop's
# engine is stopped -- the CLI is on PATH either way -- and `|| true` then
# swallowed the failed bring-up. Every check ran against a dead
# 127.0.0.1:$qa_port, 8 of them recorded `cargo exit 101` (in fact
# `PoolTimedOut`), and this gate published `VERDICT: DENIED`.
#
# That is the worst possible answer. DENIED means a real defect, and the header
# above says so: it outranks BLOCKED precisely so a defect is never reported as
# "waiting on someone else". Reporting a stopped daemon as a defect spends
# somebody's day looking for a bug in code that was fine -- it cost one here.
# Exit 2 already means "the gate could not run, no verdict"; this now uses it.
command -v docker >/dev/null 2>&1 || {
  echo "ENV ERROR: docker not found on PATH" >&2
  exit 2
}
if ! dkr version --format '{{.Server.Version}}' >/dev/null 2>&1; then
  echo "ENV ERROR: Docker engine is unavailable or this user cannot access its socket" >&2
  exit 2
fi
if ! qc up -d --wait qa-db >/dev/null 2>&1; then
  echo "ENV ERROR: the QA database did not become healthy" >&2
  exit 2
fi

# --- L1 AT-08 stale data --------------------------------------------------------
run_check L1 at08-stale-data l1-stale-data.txt -p risk-gateway at_08

# --- L2 AT-09 UNKNOWN resolution -------------------------------------------------
# The single most dangerous state: a timeout proves nothing, so a resubmission
# may place a second real order.
run_check L2 at09-unknown-order l2-unknown.txt -p kis-client --test live_order_state unknown
run_check L2b one-order-per-intent l2b-one-order.txt -p kis-client --test live_order_state at_most_one

# --- L3 no-Member-Live -----------------------------------------------------------
run_check L3 no-member-live l3-rbac.txt -p api-server --test live_rbac

# --- L4 persisted gatekeeper, restart-safe ---------------------------------------
# The decision must replay identically from its stored snapshot, or "blocked
# until green" would not survive the restart it exists to survive.
run_check L4 gate-replay l4-replay.txt -p risk-gateway reproduced_exactly_after_a_restart
run_check L4b gate-persistence l4b-persistence.txt -p api-server --test risk_store

# --- L5 reconciliation -----------------------------------------------------------
run_check L5 reconciliation l5-reconciliation.txt -p kis-client reconciliation
run_check L5b readiness l5b-readiness.txt -p api-server --test reconciliation_store

# --- L6 kill switch --------------------------------------------------------------
run_check L6 kill-switch l6-kill-switch.txt -p api-server --test live_rbac kill_switch

# --- L7 order intent idempotency -------------------------------------------------
run_check L7 intent-idempotency l7-idempotency.txt -p api-server --test live_order_state_store

# --- L8 live node ----------------------------------------------------------------
l8_t="$transcript_dir/l8-live-node.txt"
( cd "$root" && uv run --project nt python -m pytest nt/live-node/tests -q ) >"$l8_t" 2>&1
l8_rc=$?
l8_ran="$(grep -Eo '[0-9]+ passed' "$l8_t" | grep -Eo '[0-9]+' | head -n1)"
if [ "$l8_rc" -ne 0 ]; then
  add_check L8 live-node FAIL "pytest exit $l8_rc"
elif [ "${l8_ran:-0}" -eq 0 ]; then
  add_check L8 live-node FAIL "0 tests selected"
else
  add_check L8 live-node PASS "$l8_ran assertion(s)"
fi

# --- L9 executable runbooks ------------------------------------------------------
# Run, not read. A procedure nobody can verify is one that has quietly stopped
# working, and you find that out during the incident it was written for.
l9_t="$transcript_dir/l9-runbooks.txt"
: >"$l9_t"
l9_fail=0
l9_checks=0
for rb in "$root"/docs/runbooks/0*.sh; do
  if bash "$rb" >>"$l9_t" 2>&1; then :; else l9_fail=$((l9_fail + 1)); fi
done
l9_checks="$(grep -c '^  ok ' "$l9_t" 2>/dev/null || echo 0)"
if [ "$l9_fail" -ne 0 ]; then
  add_check L9 runbooks FAIL "$l9_fail runbook(s) failed"
elif [ "$l9_checks" -eq 0 ]; then
  add_check L9 runbooks FAIL "runbooks asserted nothing"
else
  add_check L9 runbooks PASS "$l9_checks assertion(s) across 7 runbooks"
fi

# --- L10 secret and account redaction --------------------------------------------
# Migration 0016's CHECK constraints refuse credential material outright, so
# this asserts the SHAPE cannot hold a secret rather than that today's rows
# happen not to.
# Two filters, not one: `secret` proves a pasted credential is refused BEFORE
# storage, and `holding_no_secret` proves the stored shape has nowhere to put
# one. Either alone would leave the other half unproven.
run_check L10 secret-refused l10-secret-refused.txt -p api-server --test live_rbac a_pasted_secret
run_check L10b no-secret-at-rest l10b-no-secret.txt -p api-server --test live_rbac holding_no_secret

# --- L11 migration contract ------------------------------------------------------
run_check L11 migration-contract l11-migrations.txt -p migration-contract

# --- X1/X2 external evidence -----------------------------------------------------
#
# The FIRST version of this section could be talked into APPROVED by two files
# containing arbitrary JSON: it checked that a path existed and that the text
# `"reconciled": true` appeared somewhere inside it. That is exactly the false
# approval this gate exists to prevent, and it was a hole big enough to drive a
# release through.
#
# The rule now: evidence is verified against state the SYSTEM produced, never
# against an assertion someone wrote down. The claim file supplies only an
# intent_ref -- a pointer -- and scripts/qa/verify-live-order.py reads the rest
# out of the database: a gate decision that approved this intent, an order
# bound to a real broker order number, and a green reconciliation that
# finished AFTER the order. Forging that means forging an append-only audit
# trail in a database where the app role holds no UPDATE or DELETE grant.
x1_ref="${LAGRANGE_PHASE3_KIS_CREDENTIAL_REF:-}"
x2_ref="${LAGRANGE_PHASE3_LIVE_ORDER_EVIDENCE:-}"

# X1 is not independently provable -- a credential reference can only be tested
# by USING it -- so it is not allowed to stand alone. It reports what was
# supplied; X2 is what actually gates approval, and X2 cannot pass without a
# real order having been placed with real credentials.
if [ -n "$x1_ref" ] && [ -f "$x1_ref" ]; then
  add_check X1 kis-credentials SUPPLIED_UNVERIFIED \
    "a credential reference was supplied; only X2 can prove it works"
else
  add_check X1 kis-credentials BLOCKED_EXTERNAL_CREDENTIALS \
    "no real KIS account; set LAGRANGE_PHASE3_KIS_CREDENTIAL_REF"
fi

x2_t="$transcript_dir/x2-live-order-verify.txt"
if [ -z "$x2_ref" ] || [ ! -f "$x2_ref" ]; then
  add_check X2 bounded-live-order BLOCKED_EXTERNAL_CREDENTIALS \
    "no executed low-value order evidence; requires a real account"
else
  python3 "$root/scripts/qa/verify-live-order.py" "$x2_ref" >"$x2_t" 2>&1
  x2_rc=$?
  cp "$x2_ref" "$transcript_dir/x2-live-order-claim.json" 2>/dev/null || true
  if [ "$x2_rc" -eq 0 ]; then
    add_check X2 bounded-live-order PASS "order cross-verified against recorded state"
  elif [ "$x2_rc" -eq 2 ]; then
    # "Could not check" is NOT "the evidence is not available". They are
    # different situations and only one of them is an acceptable resting
    # state, so this gets its own result -- and, like FAIL, it prevents
    # APPROVED. Approving because the verification could not run is the worst
    # outcome available here.
    add_check X2 bounded-live-order UNVERIFIABLE_ENVIRONMENT \
      "could not verify: $(head -n1 "$x2_t" 2>/dev/null)"
  else
    # A claim that exists but does NOT match recorded state is not a missing
    # proof -- it is a false one, and that is a DENIAL, not a block.
    add_check X2 bounded-live-order FAIL "claim contradicts recorded state: $(head -n1 "$x2_t" 2>/dev/null)"
  fi
fi

# --- verdict ---------------------------------------------------------------------
# Order matters. A FAIL outranks a block: if something is actually WRONG, that
# is what an operator must be told, and hiding it behind "waiting on
# credentials" would let a real defect sit unfixed until the credentials
# arrived -- at which point it would be discovered with money at stake.
fails="$(printf '%s\n' "$checks" | sed '/^$/d' | grep -cE ' (FAIL|UNVERIFIABLE_ENVIRONMENT) ' || true)"
blocked="$(printf '%s\n' "$checks" | sed '/^$/d' | grep -c ' BLOCKED_EXTERNAL_CREDENTIALS ' || true)"

if [ "${fails:-0}" -gt 0 ]; then
  verdict="DENIED"
elif [ "${blocked:-0}" -gt 0 ]; then
  verdict="BLOCKED_EXTERNAL_CREDENTIALS"
else
  verdict="APPROVED"
fi

echo
printf 'VERDICT: %s\n' "$verdict"
printf 'EVIDENCE: %s\n' "$ev_path"
if [ "$verdict" = "APPROVED" ]; then
  printf 'LIVE_TRADING: ELIGIBLE (Owner-only; Phase 3 is never exposed to Members)\n'
else
  printf 'LIVE_TRADING: DISABLED\n'
  printf 'NOT_APPROVED_BECAUSE:\n'
  printf '%s\n' "$checks" | sed '/^$/d' | grep -v ' PASS ' | sed 's/^/  - /'
fi
if [ "$verdict" = "DENIED" ]; then
  printf 'NOTE: DENIED means a Live safety invariant does NOT hold. Fix it; do not re-run hoping for a different answer.\n'
elif [ "$verdict" = "BLOCKED_EXTERNAL_CREDENTIALS" ]; then
  printf 'NOTE: BLOCKED_EXTERNAL_CREDENTIALS means every invariant provable here DOES hold, and the remaining evidence can only come from a real brokerage account. Nothing is broken. This is NOT a release, and it must never be reported as one.\n'
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
    "gate": "phase3", "task": 42, "verdict": verdict,
    "live_trading": "ELIGIBLE" if verdict == "APPROVED" else "DISABLED",
    "member_exposure": "NEVER (Phase 3 is Owner-only by approved scope)",
    "emitted_at": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "checks": items,
    "evidence_dir": tdir,
    "approval_rule": (
        "APPROVED requires every Live invariant to pass AND real broker credentials AND one "
        "bounded low-value order actually placed and reconciled. A FAIL outranks a block, so a "
        "real defect is never hidden behind 'waiting on credentials'. There is no override."
    ),
}
with open(path, "w", encoding="utf-8") as f:
    json.dump(summary, f, indent=2, ensure_ascii=False)
PYEOF
printf 'EVIDENCE_WRITTEN: %s\n' "$ev_path"
exit 0
