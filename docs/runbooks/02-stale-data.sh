#!/usr/bin/env bash
# Runbook: market data has gone stale (plan Todo 41)
#
# Stale data blocks new orders (Risk Gateway check 3, AT-08). The node stays
# HEALTHY throughout: nothing is wrong with the process, and restarting it
# would neither refresh the feed nor preserve the record of what happened.
# The instinct to restart is the thing this runbook is written against.
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=lib/assert.sh
. "$REPO_ROOT/docs/runbooks/lib/assert.sh"
ACCOUNT="${1:-runbook-acct}"
LOCK_DIR="$(mktemp -d)"
trap 'rm -rf "$LOCK_DIR"' EXIT
printf '%s\n\n' "Runbook: market data has gone stale"

printf 'STEP 1 - confirm the node refuses to trade on stale data\n'
run_node --lock-dir "$LOCK_DIR" status --account "$ACCOUNT" --reconciliation-green --data-stale
assert_exit "$RUN_CODE" 2 "stale data blocks, and blocking is not a fault"
assert_json_eq "$RUN_OUT" '.ready' 'false' "no order may be submitted"
assert_json_eq "$RUN_OUT" '.refusal' 'DATA_STALE' "the reason names the feed"
assert_json_eq "$RUN_OUT" '.healthy' 'true' "do NOT restart the node"

printf '\nSTEP 2 - the stale-data block lifts by itself once data is fresh\n'
printf '        (no restart, no intervention)\n'
run_node --lock-dir "$LOCK_DIR" status --account "$ACCOUNT" --reconciliation-green
# NOT asserted as "nothing refuses at all". `status` reconstructs state rather
# than querying a running node -- there is no IPC to a live node yet -- so it
# reports NODE_NOT_READY regardless. What IS provable here, and what the
# runbook is actually about, is that DATA_STALE is no longer the reason.
assert_json_eq "$RUN_OUT" '.refusal' 'NODE_NOT_READY' "the stale-data refusal is gone"
assert_json_eq "$RUN_OUT" '.healthy' 'true' "still no reason to restart anything"

printf '\nSTEP 3 - the block is counted, so it is visible on a dashboard\n'
assert_json_eq "$RUN_OUT" '.metrics.stale_data_blocks' '0' "the metric is reported, not absent"

runbook_summary "02-stale-data"
