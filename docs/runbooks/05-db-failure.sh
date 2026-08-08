#!/usr/bin/env bash
# Runbook: the database is unavailable (plan Todo 41)
#
# Design 16: a failed DB write blocks new Live orders. A decision that cannot
# be recorded must not authorise an order, because after a restart there would
# be nothing to reconcile against.
#
# Note the direction of the failure. The system does not keep trading and log
# later; it stops. An operator arriving at this runbook should expect a HALT,
# and its absence is the emergency -- not its presence.
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=lib/assert.sh
. "$REPO_ROOT/docs/runbooks/lib/assert.sh"
ACCOUNT="${1:-runbook-acct}"
LOCK_DIR="$(mktemp -d)"
trap 'rm -rf "$LOCK_DIR"' EXIT
printf '%s\n\n' "Runbook: the database is unavailable"

printf 'STEP 1 - confirm new orders are blocked, not queued\n'
printf '        The Risk Gateway denies with NOT_PERSISTED, graded CRITICAL,\n'
printf '        because an unrecordable decision breaks the audit trail.\n'
printf '        Proven by risk-gateway a_failed_write_denies_an_otherwise_approved_order.\n'

printf '\nSTEP 2 - the node is still HEALTHY; do not restart it\n'
run_node --lock-dir "$LOCK_DIR" status --account "$ACCOUNT"
assert_json_eq "$RUN_OUT" '.healthy' 'true' "the process is fine; the database is not"
assert_exit "$RUN_CODE" 2 "blocked, which is the designed behaviour"

printf '\nSTEP 3 - after the database returns, reconcile BEFORE resuming\n'
printf '        Readiness is NEVER_RECONCILED or stale after an outage, and the\n'
printf '        kill switch cannot be disengaged until a run comes back green.\n'
STATE_FILE="$LOCK_DIR/state.json"
printf '%s\n' '{"intent_states": {"oi-inflight": "SUBMITTED"}, "blocking_mismatch_kinds": ["UNRESOLVED_INTENT"], "fills_to_apply": [], "lookups_required": ["oi-inflight"]}' > "$STATE_FILE"
run_node --lock-dir "$LOCK_DIR" plan-startup --account "$ACCOUNT" --input "$STATE_FILE"
assert_json_eq "$RUN_OUT" '.to_sweep[0]' 'oi-inflight' "orders in flight during the outage are swept"
assert_json_eq "$RUN_OUT" '.may_trade' 'false' "trading stays blocked until they are settled"

runbook_summary "05-db-failure"
