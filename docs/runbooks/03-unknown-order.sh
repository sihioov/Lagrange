#!/usr/bin/env bash
# Runbook: an order is in UNKNOWN state (plan Todo 41)
#
# The single most dangerous state in the system. A timeout proves NOTHING:
# the order may be live at the broker. Resubmitting places a second real
# order against an account that may already hold the first.
#
# There is exactly one way out, and it is a broker lookup. The machine has no
# transition from UNKNOWN back into the submission path -- not a check that
# could be skipped, an edge that does not exist -- so this runbook cannot
# tell you to retry even if you want to.
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=lib/assert.sh
. "$REPO_ROOT/docs/runbooks/lib/assert.sh"
ACCOUNT="${1:-runbook-acct}"
LOCK_DIR="$(mktemp -d)"
trap 'rm -rf "$LOCK_DIR"' EXIT
printf '%s\n\n' "Runbook: an order is in UNKNOWN state"

printf 'STEP 1 - see what a restart WOULD do before doing it\n'
STATE_FILE="$LOCK_DIR/state.json"
printf '%s\n' '{"intent_states": {"oi-timedout": "SUBMITTING"}, "blocking_mismatch_kinds": ["UNRESOLVED_INTENT"], "fills_to_apply": [], "lookups_required": ["oi-timedout"]}' > "$STATE_FILE"
run_node --lock-dir "$LOCK_DIR" plan-startup --account "$ACCOUNT" --input "$STATE_FILE"
assert_exit "$RUN_CODE" 2 "an unresolved intent blocks trading"
assert_json_eq "$RUN_OUT" '.outcome' 'LOOKUPS_REQUIRED' "the next action is a LOOKUP, not a retry"
assert_json_eq "$RUN_OUT" '.to_sweep[0]' 'oi-timedout' "the in-flight intent is swept to UNKNOWN first"
assert_json_eq "$RUN_OUT" '.lookups_required[0]' 'oi-timedout' "the broker must be asked about this order"
assert_json_eq "$RUN_OUT" '.may_trade' 'false' "nothing trades until it is settled"

printf '\nSTEP 2 - ask the broker (operator action)\n'
printf '        Query the order by its client order id. Do NOT resubmit.\n'
printf '        Design 16: resubmission is forbidden before a lookup resolves it.\n'

printf '\nSTEP 3 - once resolved, the intent is settled and startup proceeds\n'
printf '%s\n' '{"intent_states": {"oi-timedout": "ACCEPTED"}, "blocking_mismatch_kinds": [], "fills_to_apply": [], "lookups_required": []}' > "$STATE_FILE"
run_node --lock-dir "$LOCK_DIR" plan-startup --account "$ACCOUNT" --input "$STATE_FILE"
assert_exit "$RUN_CODE" 0 "a settled intent no longer blocks"
assert_json_eq "$RUN_OUT" '.to_sweep | length' '0' "a settled order is not swept"

runbook_summary "03-unknown-order"
