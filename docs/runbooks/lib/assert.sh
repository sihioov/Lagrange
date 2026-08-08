#!/usr/bin/env bash
# Shared assertions for the Live runbooks (plan Todo 41).
#
# Every runbook here is EXECUTABLE and asserts on machine-readable output
# rather than instructing a human to eyeball a screen. The reason is the one
# the whole Live surface is built around: a procedure nobody can verify is a
# procedure that has quietly stopped working, and you find that out during the
# incident it was written for.
#
# The rules these helpers enforce:
#
#   * an assertion that selects NOTHING fails. `assert_json_eq` on a key that
#     does not exist is a failure, not a pass on `""` == `""`. This is the
#     phase-2 gate rule, and it is the difference between a runbook that
#     checks something and one that only appears to.
#   * exit code 2 from the CLI means "running but not ready", which is the
#     system working as designed. Runbooks distinguish it from 1 so a safe
#     refusal is never escalated as an outage.

set -euo pipefail

RUNBOOK_CHECKS=0
RUNBOOK_FAILURES=0

_pass() {
  RUNBOOK_CHECKS=$((RUNBOOK_CHECKS + 1))
  printf '  ok   %s\n' "$1"
}

_fail() {
  RUNBOOK_CHECKS=$((RUNBOOK_CHECKS + 1))
  RUNBOOK_FAILURES=$((RUNBOOK_FAILURES + 1))
  printf '  FAIL %s\n' "$1" >&2
}

# assert_json_eq <json> <path> <expected> <description>
#
# Fails if the path is ABSENT, which is the whole point: an assertion against a
# renamed field must fail rather than compare "" to "" and pass. `jsonpath.py`
# exits 1 for a path that does not resolve.
#
# Python rather than `jq`, because jq is not installed on every host that has
# to run these during an incident -- and a runbook that cannot run on the
# machine in front of you is not a runbook. Python is already a hard dependency
# here, since the node itself is Python.
assert_json_eq() {
  local json="$1" path="$2" expected="$3" desc="$4" actual
  if ! actual=$(printf '%s' "$json" | python3 "$REPO_ROOT/docs/runbooks/lib/jsonpath.py" "$path" 2>/dev/null); then
    _fail "$desc (path $path is absent - assertion selected nothing)"
    return 0
  fi
  if [ "$actual" = "$expected" ]; then
    _pass "$desc"
  else
    _fail "$desc (expected '$expected', got '$actual')"
  fi
}

# assert_exit <actual> <expected> <description>
assert_exit() {
  if [ "$1" = "$2" ]; then
    _pass "$3 (exit $2)"
  else
    _fail "$3 (expected exit $2, got $1)"
  fi
}

# assert_contains <haystack> <needle> <description>
assert_contains() {
  case "$1" in
    *"$2"*) _pass "$3" ;;
    *) _fail "$3 (missing '$2')" ;;
  esac
}

runbook_summary() {
  printf '\n%s: %d checks, %d failures\n' "$1" "$RUNBOOK_CHECKS" "$RUNBOOK_FAILURES"
  if [ "$RUNBOOK_CHECKS" -eq 0 ]; then
    printf 'FAILED: the runbook asserted nothing at all\n' >&2
    exit 1
  fi
  [ "$RUNBOOK_FAILURES" -eq 0 ] || exit 1
}

# Runs `python -m live_node ...` and captures stdout plus exit code, without
# tripping `set -e` on the expected non-zero codes.
# `live_node` lives in a hyphenated directory and is not installed
# (`[tool.uv] package = false`), so its parent must be on PYTHONPATH. Without
# this every invocation fails with ModuleNotFoundError and exits 1 -- which
# looks exactly like the node refusing to start, and would have an operator
# debugging the wrong thing.
run_node() {
  set +e
  RUN_OUT=$(PYTHONPATH="$REPO_ROOT/nt/live-node" uv run --project "$REPO_ROOT/nt" python -m live_node "$@" 2>/dev/null)
  RUN_CODE=$?
  set -e
}
