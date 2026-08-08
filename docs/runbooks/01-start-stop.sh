#!/usr/bin/env bash
# Runbook: start and stop a Live node (plan Todo 41)
#
# Starting a Live node is NOT the same as permitting it to trade, and this
# runbook exists mainly to make that impossible to confuse. `start` leaves the
# node in RECONCILING and exits 2; only a green reconciliation moves it on.
# An operator who read "started" as "trading" would believe orders were
# flowing when nothing had been checked against the broker.
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=lib/assert.sh
. "$REPO_ROOT/docs/runbooks/lib/assert.sh"
ACCOUNT="${1:-runbook-acct}"
LOCK_DIR="$(mktemp -d)"
trap 'rm -rf "$LOCK_DIR"' EXIT
printf '%s\n\n' "Runbook: start and stop a Live node"

printf 'STEP 1 - claim the account and begin reconciling\n'
run_node --lock-dir "$LOCK_DIR" start --account "$ACCOUNT"
assert_exit "$RUN_CODE" 2 "a fresh start is running but NOT ready"
assert_json_eq "$RUN_OUT" '.started' 'true' "the account was claimed"
assert_json_eq "$RUN_OUT" '.state' 'RECONCILING' "a node never starts READY"
assert_json_eq "$RUN_OUT" '.ready' 'false' "trading is not permitted yet"
assert_json_eq "$RUN_OUT" '.healthy' 'true' "the process itself is fine"

printf '\nSTEP 2 - a second node for the same account must be refused\n'
# The lock names a LIVE process, so the refusal has to be demonstrated against
# one. Two sequential CLI invocations would NOT do it: the first process has
# already exited, so its lock is correctly reclaimed. That is the
# crash-recovery path, not the duplicate-node path, and asserting a refusal
# there would have been asserting the wrong thing.
# A REAL operating-system pid of a live process. `$$` is not usable here:
# under Git Bash it is the MSYS pid, which Windows OpenProcess cannot resolve,
# so the lock would look stale and be reclaimed. The runbook would then "pass"
# while demonstrating the exact opposite of the property it claims to check.
python3 -c "import os,sys,time; open(sys.argv[1],'w').write(str(os.getpid())); time.sleep(25)" "$LOCK_DIR/holder.pid" &
HOLDER_BG=$!
trap 'kill "$HOLDER_BG" 2>/dev/null || true; rm -rf "$LOCK_DIR"' EXIT
sleep 1
HOLDER_PID="$(cat "$LOCK_DIR/holder.pid")"
printf '%s\n%s' "$HOLDER_PID" "$ACCOUNT" > "$LOCK_DIR/live-node-$ACCOUNT.lock"
run_node --lock-dir "$LOCK_DIR" start --account "$ACCOUNT"
assert_exit "$RUN_CODE" 1 "a duplicate node is a FAILURE, not a safe refusal"
assert_json_eq "$RUN_OUT" '.error' 'NODE_ALREADY_RUNNING' "the refusal names itself"
assert_json_eq "$RUN_OUT" '.started' 'false' "nothing was claimed twice"
assert_json_eq "$RUN_OUT" '.held_by_pid' "$HOLDER_PID" "the refusal names who holds it"

printf '\nSTEP 2b - a lock left by a DEAD process is reclaimed, not fatal\n'
printf '        Otherwise a crash would need a human to delete a file before\n'
printf '        Live could come back, and "just remove the lock" becomes\n'
printf '        routine until it stops meaning anything the one time it matters.\n'
printf '%s\n%s' "999999999" "$ACCOUNT" > "$LOCK_DIR/live-node-$ACCOUNT.lock"
run_node --lock-dir "$LOCK_DIR" start --account "$ACCOUNT"
assert_exit "$RUN_CODE" 2 "a stale lock does not block a restart"
assert_json_eq "$RUN_OUT" '.started' 'true' "the account was reclaimed"

printf '\nSTEP 3 - status reports the running node\n'
run_node --lock-dir "$LOCK_DIR" status --account "$ACCOUNT"
assert_exit "$RUN_CODE" 2 "running, still not ready"
assert_json_eq "$RUN_OUT" '.running' 'true' "the lock is held"

printf '\nSTEP 4 - stop: release the lock, then the account is free\n'
rm -f "$LOCK_DIR"/live-node-"$ACCOUNT".lock
run_node --lock-dir "$LOCK_DIR" status --account "$ACCOUNT"
assert_json_eq "$RUN_OUT" '.running' 'false' "no node holds the account"
assert_json_eq "$RUN_OUT" '.state' 'STARTING' "absent is not the same as STOPPED"

runbook_summary "01-start-stop"
