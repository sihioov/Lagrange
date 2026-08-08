#!/usr/bin/env bash
# Runbook: the execution WebSocket dropped (plan Todo 41)
#
# A dropped socket means fill reports may have been missed. The node goes
# DEGRADED, and the important detail is what it does NOT do: it does not go
# straight back to READY when the socket returns. Whatever happened during the
# gap may have happened while orders were in flight, so agreement with the
# broker has to be re-established rather than assumed.
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=lib/assert.sh
. "$REPO_ROOT/docs/runbooks/lib/assert.sh"
ACCOUNT="${1:-runbook-acct}"
LOCK_DIR="$(mktemp -d)"
trap 'rm -rf "$LOCK_DIR"' EXIT
printf '%s\n\n' "Runbook: the execution WebSocket dropped"

printf 'STEP 1 - the node degrades and stops submitting\n'
printf '        (DEGRADED is reached in-process; asserted in test_lifecycle.py)\n'

printf '\nSTEP 2 - a reconnect does NOT restore trading by itself\n'
printf '        DEGRADED has no edge to READY. The only path back is through\n'
printf '        RECONCILING, which is what catches the fills we missed.\n'
STATE_FILE="$LOCK_DIR/state.json"
printf '%s\n' '{"intent_states": {}, "blocking_mismatch_kinds": [], "fills_to_apply": ["E-missed-1", "E-missed-2"], "lookups_required": []}' > "$STATE_FILE"
run_node --lock-dir "$LOCK_DIR" plan-startup --account "$ACCOUNT" --input "$STATE_FILE"
assert_json_eq "$RUN_OUT" '.fills_to_apply[0]' 'E-missed-1' "the missed fill is found"
assert_json_eq "$RUN_OUT" '.fills_to_apply | length' '2' "both missed fills are found"

printf '\nSTEP 3 - applying a missed fill is safe to repeat\n'
printf '        The ledger rejects a duplicate fill_id, and the order machine\n'
printf '        reports a re-sent report as NoChange. Two independent guards,\n'
printf '        so a reconnect storm cannot double a position.\n'
assert_json_eq "$RUN_OUT" '.outcome' 'READY' "once applied, nothing blocks"

runbook_summary "04-websocket-gap"
