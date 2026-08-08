#!/usr/bin/env bash
# Runbook: our books disagree with the broker (plan Todo 41)
#
# Design 16: an internal-vs-broker position mismatch pauses Live strategies
# and requires Owner approval.
#
# The rule is that the BROKER is the truth about the broker. A position we
# cannot account for is a position that really exists in a real account, and
# adopting our own number would hide it. Exactly one difference is resolvable
# automatically -- a fill we simply had not applied -- and everything else
# needs an Owner.
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=lib/assert.sh
. "$REPO_ROOT/docs/runbooks/lib/assert.sh"
ACCOUNT="${1:-runbook-acct}"
LOCK_DIR="$(mktemp -d)"
trap 'rm -rf "$LOCK_DIR"' EXIT
printf '%s\n\n' "Runbook: our books disagree with the broker"

printf 'STEP 1 - identify the mismatch and confirm it blocks\n'
STATE_FILE="$LOCK_DIR/state.json"
printf '%s\n' '{"intent_states": {}, "blocking_mismatch_kinds": ["POSITION", "UNMAPPED_BROKER_ORDER"], "fills_to_apply": [], "lookups_required": []}' > "$STATE_FILE"
run_node --lock-dir "$LOCK_DIR" plan-startup --account "$ACCOUNT" --input "$STATE_FILE"
assert_exit "$RUN_CODE" 2 "an unexplained difference blocks trading"
assert_json_eq "$RUN_OUT" '.outcome' 'BLOCKED' "this needs a person"
assert_json_eq "$RUN_OUT" '.blocking_reasons[0]' 'POSITION' "the position difference is named"
assert_json_eq "$RUN_OUT" '.blocking_reasons[1]' 'UNMAPPED_BROKER_ORDER' "so is the order nobody manages"

printf '\nSTEP 2 - an UNMAPPED_BROKER_ORDER is the most serious kind\n'
printf '        A real order at the broker that no intent of ours manages.\n'
printf '        Do not cancel it blindly; establish what placed it first.\n'

printf '\nSTEP 3 - a missed fill, by contrast, resolves without an Owner\n'
printf '%s\n' '{"intent_states": {}, "blocking_mismatch_kinds": [], "fills_to_apply": ["E-1"], "lookups_required": []}' > "$STATE_FILE"
run_node --lock-dir "$LOCK_DIR" plan-startup --account "$ACCOUNT" --input "$STATE_FILE"
assert_json_eq "$RUN_OUT" '.outcome' 'READY' "applying the fill clears it"
assert_json_eq "$RUN_OUT" '.blocking_reasons | length' '0' "nothing needed a judgement call"

printf '\nSTEP 4 - re-enabling Live requires a GREEN run, not an explanation\n'
printf '        POST /api/v1/admin/live/kill-switch/disable answers 409\n'
printf '        LIVE_RECONCILIATION_REQUIRED until readiness is READY.\n'

runbook_summary "06-reconciliation-mismatch"
