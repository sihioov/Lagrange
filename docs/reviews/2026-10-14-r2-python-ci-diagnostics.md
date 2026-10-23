# I1a Python CI failure diagnostics

## Run and test result

This diagnostic uses only the Python job from run [38506113191](https://github.com/sihioov/Lagrange/actions/runs/38506113191), attempt 1, exact candidate `02a400e0a6f5f413c32ebd7a19f70af2e46dbfff`. The job log response was 30,991 bytes, SHA-256 `8e9ff87905cf16146dd8f52cbee305faccdeb5155592ea0caf65088260ac2893`. The `uv run --project nt pytest -q` step exited 1. Pytest reported 1 failed, 481 passed, 10 skipped, and 7 warnings.

The failed node is `nt/live-node/tests/test_runtime.py::test_sigterm_stops_runtime_and_releases_account_lock`. The exception is pytest `Failed`, raised by `pytest.fail` in `wait_for_cycle`; the test traceback anchors at `test_runtime.py:264`, with helper failure at `:43`. The observed condition is that the test did not read a status mapping with `cycle >= 1` during 200 polls, each followed by a 0.005 second sleep (nominal sleep budget 1 second).

## Exact-candidate source sequence

In `nt/live-node/tests/test_runtime.py:234-275`, the test starts a child Python module with temporary status and lock paths and captured stdout/stderr. It waits for the first cycle before calling `process.terminate()`. Only after that wait succeeds does the test assert `process.wait(timeout=2) == 0`, absence of the lock file, persisted `STOPPED`, and stdout `STOPPED`. The failure at line 264 precedes the SIGTERM call and those normal-path assertions.

The helper at `test_runtime.py:37-43` accepts only a parsed status mapping whose cycle is at least one. `live_node/runtime.py:225-233` treats missing, unreadable, invalid UTF-8, or invalid JSON status as absent, so the test failure does not identify which of those states applied (or whether a valid lower cycle was seen). `runtime.py:236-257` publishes status by writing a temporary file and replacing the target.

The cleanup `finally` in `test_runtime.py:267-270` conditionally calls `poll()`, then `kill()` and `wait(timeout=2)` if the child still runs. The log’s final pytest exception remains `Failed` from the readiness helper; it does not report the child return code or whether that conditional kill branch ran.

For the successful path, exact-candidate `live_node/__main__.py:314-374` installs a SIGTERM handler that sets a stop event, calls the runtime, restores prior handlers in `finally`, then emits the final payload. `live_node/runtime.py:393-450` runs under the `AccountLock` context, publishes reconciliation cycles, and on stop publishes the final stopped state; `live_node/isolation.py:420-424` releases the lock on context exit. These source facts describe the intended sequence; because readiness failed first, this CI execution does not demonstrate that graceful SIGTERM sequence.

## What remains unknown

The log contains no child subprocess return code and no `TimeoutExpired`/`TimeoutError`. The parent pytest step’s exit code is 1. It does not show whether the status file was absent, unreadable/malformed, delayed, or remained below cycle one; nor does it report the child state at cleanup, whether `kill()` ran, or final lock/status/stdout contents. The evidence cannot distinguish a test timing issue from a runtime defect and establishes neither a flaky test nor a product bug.

Raw logs were filtered in memory and not persisted. No local test, reproduction, child process, signal, build, or operational action was run. The prior CI report and receipt hashes were verified unchanged before writing. No implementation or further decomposition is included in this evidence task.
