#!/usr/bin/env bash
# full-system-gate.sh - the whole-system gate (plan F3).
# POSIX/CI twin of scripts/qa/full-system-gate.ps1.
#
# Runs every phase gate in order and reduces their verdicts to ONE:
#
#   VERDICT: APPROVED
#   VERDICT: BLOCKED_EXTERNAL
#   VERDICT: DENIED
#
# It does NOT re-derive any phase's result. Each phase gate owns its own
# evidence and emits its own verdict; this reads them. A composite that
# recomputed what its children already decided could not detect the one thing
# it most needs to detect -- that a phase's evidence was never produced at all.
#
# The reduction is deliberately pessimistic, in this order:
#
#   any DENIED / missing / unparseable  -> DENIED
#   any BLOCKED_*                       -> BLOCKED_EXTERNAL
#   otherwise                           -> APPROVED
#
# DENIED outranks BLOCKED because a real defect must never be reported as
# "waiting on an external party": that phrasing invites waiting, and the defect
# would then be discovered when the external party arrived and money was at
# stake. A MISSING verdict is also DENIED, not blocked -- "the gate did not
# run" and "the gate ran and was blocked" are different situations, and only
# one of them is a resting state.
#
# Usage:
#   scripts/qa/full-system-gate.sh [--clean] [--include-failure]
#                                  [--include-restore]
#                                  [--include-vendor-when-configured]
# No twin, deliberately. The phase gates each ship a .sh/.ps1 pair because
# phase1's cargo lane is WSL-only and phase2/3 shell out to docker; this
# composite does neither -- it reads evidence files and shells out to the phase
# gates -- so the one script runs on both hosts, Git Bash included. The header
# claimed a `full-system-gate.ps1` that has never existed, which is the kind of
# thing someone discovers while looking for the reason their Windows run
# behaved differently.
set -u

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
evidence_dir="$root/.omo/evidence"
out="$evidence_dir/final-F3-operational-e2e.json"
clean=0; include_failure=0; include_restore=0; include_vendor=0

while [ $# -gt 0 ]; do
  case "$1" in
    --clean) clean=1; shift ;;
    --include-failure) include_failure=1; shift ;;
    --include-restore) include_restore=1; shift ;;
    --include-vendor-when-configured) include_vendor=1; shift ;;
    *) echo "USAGE: $0 [--clean] [--include-failure] [--include-restore] [--include-vendor-when-configured]" >&2; exit 2 ;;
  esac
done

mkdir -p "$evidence_dir"
phases=""
add_phase() { # add_phase <id> <verdict> <detail>
  phases="$phases
$1 $2 $3"
  printf 'PHASE %-8s = %-30s %s\n' "$1" "$2" "$3"
}

# read_verdict <evidence.json> -- the phase's OWN answer, never recomputed.
read_verdict() {
  [ -f "$1" ] || { printf 'MISSING'; return; }
  sed -n 's/.*"verdict": *"\([A-Z_]*\)".*/\1/p' "$1" | head -n1
}

# This gate OWNS the QA database for its whole run.
#
# It matters more than it sounds. The QA database keeps its data directory on a
# 7.9GB tmpfs -- a RAM disk, chosen so a disposable database is fast and leaves
# nothing behind -- and every DB-gated test creates a scratch database inside
# it. Running phase after phase with `--keep-db` (which each child needs, so it
# does not tear the database out from under the next one) accumulates scratch
# databases until the tmpfs is 100% full. What happens then is NOT an error
# that names itself: every test fails with `No space left on device` from
# inside postgres, which reads as a broken test suite rather than a full
# volume. It cost an hour to find once; the recreate below is why it will not
# again.
qa_compose="$root/deploy/qa/qa-db.compose.yml"
hostpath() {
  if command -v cygpath >/dev/null 2>&1; then cygpath -w "$1"; else printf '%s' "$1"; fi
}
dkr() { MSYS_NO_PATHCONV=1 MSYS2_ARG_CONV_EXCL='*' docker "$@"; }
qc() { dkr compose -p lagrange-qa -f "$(hostpath "$qa_compose")" "$@"; }

echo "== full system gate =="
[ "$clean" -eq 1 ] && echo "(clean run requested)"

if command -v docker >/dev/null 2>&1; then
  echo "   recreating the QA database (its tmpfs is the shared resource)"
  qc down -v --remove-orphans >/dev/null 2>&1 || true
  qc up -d --wait qa-db >/dev/null 2>&1 || {
    echo "ENV ERROR: the QA database did not become healthy" >&2; exit 2; }
  # Torn down at the end, not by any child: a child that reclaimed the shared
  # database would leave every later phase without one.
  trap 'qc down -v --remove-orphans >/dev/null 2>&1 || true' EXIT
fi

# --- phase 1 --------------------------------------------------------------------
# NOT re-run here. phase1-gate.sh has no --keep-db flag, so running it would
# tear down the database this gate owns and every later phase would fail with
# a connection error that looks nothing like the real cause. Its verdict is
# read from the evidence it already wrote, which is this gate's rule anyway:
# read a phase's answer, never recompute it.
# Phase 1's gate writes its evidence under Todo 28, not 19. Read the file that
# actually carries `"gate": "phase1"` rather than a path guessed from the plan.
v="$(read_verdict "$evidence_dir/task-28-lagrange-station-implementation.json")"
[ -n "$v" ] || v="MISSING"
add_phase phase1 "$v" "Phase 1 five-user, auth, licensing, worker, artifacts"

# --- phase 2 --------------------------------------------------------------------
v="$(read_verdict "$evidence_dir/task-35-lagrange-station-implementation.json")"
[ -n "$v" ] || v="MISSING"
add_phase phase2 "$v" "Phase 2 Paper, restart, restore"

# --- phase 3 (always re-run: it is the one this wave is about) -------------------
# `|| true` used to be here, and it hid the one exit code that matters.
#
# phase3-gate exits 2 for "could not run, no verdict" -- and when it does, it
# has written no evidence. Swallowing that left the read below picking up
# WHATEVER task-42 was already on disk, possibly days old, and presenting it as
# this run's phase-3 result. A composite whose whole stated job is to notice
# that a phase's evidence was never produced would have been the last place to
# notice it.
#
# 2 propagates: this gate's own contract already reserves exit 2 for the same
# meaning, and a composite that cannot see one of its phases has no verdict to
# give either.
( cd "$root" && bash scripts/qa/phase3-gate.sh --keep-db ) >"$evidence_dir/f3-phase3.log" 2>&1
phase3_rc=$?
if [ "$phase3_rc" -eq 2 ]; then
  echo "ENV ERROR: phase3-gate could not run (exit 2); see $evidence_dir/f3-phase3.log" >&2
  exit 2
fi
v="$(read_verdict "$evidence_dir/task-42-lagrange-station-implementation.json")"
[ -n "$v" ] || v="MISSING"
add_phase phase3 "$v" "Phase 3 Live safety invariants"

# --- optional suites ------------------------------------------------------------
if [ "$include_failure" -eq 1 ]; then
  if [ -f "$root/scripts/qa/failure-suite.sh" ]; then
    : >"$evidence_dir/f3-failure.log"
    for ph in 2 3; do
      ( cd "$root" && bash scripts/qa/failure-suite.sh --phase "$ph" --keep-db ) \
        >>"$evidence_dir/f3-failure.log" 2>&1 || true
    done
    # The child's VERDICT, not its exit code. INCOMPLETE exits 0 -- it is a
    # legitimate partial run rather than an error -- so an exit-code check
    # would report a suite that skipped half its scenarios as a clean pass.
    f_failed="$(grep -c '_FAULTS_FAILED' "$evidence_dir/f3-failure.log" || true)"
    f_incomplete="$(grep -c '_FAULTS_INCOMPLETE' "$evidence_dir/f3-failure.log" || true)"
    f_passed="$(grep -c '_FAULTS_PASSED' "$evidence_dir/f3-failure.log" || true)"
    if [ "${f_failed:-0}" -gt 0 ]; then
      add_phase failures DENIED "a fault scenario failed (see f3-failure.log)"
    elif [ "${f_incomplete:-0}" -gt 0 ]; then
      add_phase failures BLOCKED_EXTERNAL "fault coverage incomplete: an external prerequisite is absent"
    elif [ "${f_passed:-0}" -eq 2 ]; then
      add_phase failures PASS "fault injection, phases 2 and 3"
    else
      add_phase failures MISSING "the fault suite emitted no verdict"
    fi
  else
    add_phase failures MISSING "scripts/qa/failure-suite.sh not found"
  fi
fi

if [ "$include_restore" -eq 1 ]; then
  # Read, never re-derived: a restore verdict this gate computed itself could
  # not tell you that the restore drill was never run.
  rv="${LAGRANGE_PHASE2_RESTORE_VERDICT:-}"
  if [ -n "$rv" ] && [ -f "$rv" ] && grep -q '"verdict": *"SUCCESS"' "$rv"; then
    add_phase restore PASS "PITR restore verified"
  else
    add_phase restore BLOCKED_EXTERNAL "no restore verdict; set LAGRANGE_PHASE2_RESTORE_VERDICT"
  fi
fi

if [ "$include_vendor" -eq 1 ]; then
  # "when configured" is the operative phrase: an absent vendor tenant is a
  # known external block, not a failure, and must not read as one.
  if [ -n "${LAGRANGE_VENDOR_TENANT:-}" ]; then
    add_phase vendor PASS "vendor tenant configured"
  else
    add_phase vendor BLOCKED_EXTERNAL "no vendor tenant configured (expected on this host)"
  fi
fi

# --- reduce ---------------------------------------------------------------------
denied="$(printf '%s\n' "$phases" | sed '/^$/d' | grep -cE ' (DENIED|MISSING) ' || true)"
blocked="$(printf '%s\n' "$phases" | sed '/^$/d' | grep -cE ' BLOCKED' || true)"

if [ "${denied:-0}" -gt 0 ]; then
  verdict="DENIED"
elif [ "${blocked:-0}" -gt 0 ]; then
  verdict="BLOCKED_EXTERNAL"
else
  verdict="APPROVED"
fi

echo
printf 'VERDICT: %s\n' "$verdict"
if [ "$verdict" != "APPROVED" ]; then
  printf 'NOT_APPROVED_BECAUSE:\n'
  printf '%s\n' "$phases" | sed '/^$/d' | grep -vE ' (APPROVED|PASS) ' | sed 's/^/  - /'
fi

python3 - "$out" "$verdict" "$phases" <<'PYEOF'
import json, sys, datetime
path, verdict, phases = sys.argv[1], sys.argv[2], sys.argv[3]
items = []
for line in phases.splitlines():
    parts = line.split()
    if len(parts) >= 3:
        items.append({"phase": parts[0], "verdict": parts[1], "detail": " ".join(parts[2:])})
json.dump({
    "artifact": "final-F3-operational-e2e", "verdict": verdict,
    "emitted_at": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "phases": items,
    "reduction_rule": (
        "DENIED or MISSING in any phase -> DENIED; any BLOCKED_* -> BLOCKED_EXTERNAL; else APPROVED. "
        "DENIED outranks BLOCKED so a real defect is never reported as waiting on an external party, "
        "and MISSING is DENIED because 'the gate did not run' is not a resting state."
    ),
}, open(path, "w", encoding="utf-8"), indent=2, ensure_ascii=False)
PYEOF
printf 'EVIDENCE_WRITTEN: %s\n' "$out"
exit 0
