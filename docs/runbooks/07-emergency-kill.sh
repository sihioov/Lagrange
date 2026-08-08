#!/usr/bin/env bash
# Runbook: EMERGENCY - stop Live now (plan Todo 41)
#
# The one runbook that must work when everything else is on fire, so it is
# the shortest and has no preconditions.
#
# Engaging the kill switch is never blocked, never needs a reason, and never
# waits for reconciliation. A precondition on STOPPING is a precondition that
# fails at the worst possible moment. Everything careful in this system is on
# the other direction: turning Live back on.
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=lib/assert.sh
. "$REPO_ROOT/docs/runbooks/lib/assert.sh"
ACCOUNT="${1:-runbook-acct}"
LOCK_DIR="$(mktemp -d)"
trap 'rm -rf "$LOCK_DIR"' EXIT
printf '%s\n\n' "Runbook: EMERGENCY - stop Live now"

printf 'STEP 1 - ENGAGE. One call. No reason required.\n'
printf '        POST /api/v1/admin/live/kill-switch/enable\n'
printf '        Owner + fresh MFA only. Nothing else gates it.\n'

printf '\nSTEP 2 - confirm new orders are refused\n'
run_node --lock-dir "$LOCK_DIR" status --account "$ACCOUNT" --kill-switch-engaged --reconciliation-green
assert_exit "$RUN_CODE" 2 "killed is not-ready, and that is correct"
assert_json_eq "$RUN_OUT" '.ready' 'false' "no order may be submitted"
assert_json_eq "$RUN_OUT" '.refusal' 'LIVE_KILL_SWITCH_ENGAGED' "the kill switch is reported FIRST"
assert_json_eq "$RUN_OUT" '.metrics.kill_switch_state' '1' "the gauge reads engaged"
assert_json_eq "$RUN_OUT" '.healthy' 'true' "the node is healthy; do not restart it"

printf '\nSTEP 3 - orders already at the broker follow the cancel policy\n'
printf '        LEAVE (default) | CANCEL_WORKING | CANCEL_UNFILLED_ONLY.\n'
printf '        No policy touches an UNKNOWN order: we do not have its broker\n'
printf '        number, so a cancel would fail or name the WRONG order.\n'

printf '\nSTEP 4 - to resume, see 06-reconciliation-mismatch\n'
printf '        Disengaging needs Owner + fresh MFA + a GREEN reconciliation.\n'

runbook_summary "07-emergency-kill"
