# Live operational runbooks (plan Todo 41)

Seven procedures, each as a **shell/PowerShell pair that actually executes**
and asserts on machine-readable output.

They are executable for one reason: a procedure nobody can verify is a
procedure that has quietly stopped working, and you find that out during the
incident it was written for. Every assertion here reads a JSON field from
`python -m live_node`, so a renamed field breaks the runbook in CI rather than
in front of an operator at 3am.

| Runbook | Situation |
|---|---|
| `01-start-stop` | Start and stop a node; duplicate and stale-lock handling |
| `02-stale-data` | Market data has gone stale (AT-08) |
| `03-unknown-order` | An order is in UNKNOWN state (AT-09) — the dangerous one |
| `04-websocket-gap` | The execution socket dropped and fills were missed |
| `05-db-failure` | The database is unavailable |
| `06-reconciliation-mismatch` | Our books disagree with the broker |
| `07-emergency-kill` | Stop Live now |

## Running them

```bash
REPO_ROOT="$(pwd)" bash docs/runbooks/01-start-stop.sh     # POSIX / Git Bash
pwsh -NoProfile -File docs/runbooks/01-start-stop.ps1      # PowerShell
```

Both twins print the same step headings and the same assertion descriptions, so
their outputs can be diffed line for line. That is deliberate: an operator
should not get different behaviour depending on which shell they happened to
open.

## The rules the assertions enforce

* **An assertion that selects nothing FAILS.** Asserting on a field that no
  longer exists is an error, never a pass comparing `""` to `""`. Without this
  a runbook full of stale field names reports success while checking nothing.
* **Exit code 2 is not a failure.** It means "running but not ready" — a kill
  switch, a red reconciliation, stale data. That is the system working exactly
  as designed, and a runbook that could not tell it from a crash (exit 1) would
  escalate every safe refusal as an outage.
* **A runbook that asserted nothing at all fails.** `runbook_summary` exits
  non-zero when the check count is zero.

`jq` is deliberately not used: it is absent on some hosts that must run these,
and a runbook that cannot run on the machine in front of you is not a runbook.
`lib/jsonpath.py` covers the small subset of paths the assertions need.
