# R2 I4a single-candidate CI recovery

Date: 2026-10-16 KST. Roadmap: R2-04 / R2-09. Status: FAILURE recovered;
no I4a source or executable acceptance. Prior accepted packages remain preserved.

Candidate `d0536f6ab49cf429d1ab05b130d515469872c3d5`,
[run 38764564342 attempt 1](https://github.com/sihioov/Lagrange/actions/runs/38764564342),
CI/ci.yml, workflow_dispatch, excited-dolphin. One dispatch followed exact-head
all-event query zero and remote equality. No rerun, cancellation or duplicate.

All eight terminal jobs and complete log digests were recovered. Seven checkout
logs contain the exact candidate; all 447 frozen pins and 310 committed blobs
match. Sanitized receipt: `target/r2-daily-beta/ci/d0536f6/ci-evidence.json`,
4,022 lines, SHA256
`267d40ca7971593c6df9d119478eaffd32d809faa179715306dbc45a04e74f59`.
Raw logs were not persisted. Later documentation commits do not change this candidate.

| Evidence | Result |
| --- | --- |
| Web, policy, Python, workspace formatting | SUCCESS |
| Strict Clippy | FAILURE: three E0433 test compilation errors |
| Workspace tests | FAILURE before any workspace test binary; required workspace names NOT_EXECUTED |
| Separate chart phase | Six exact named PASS rows |
| Protected artifact fixture | Required modes, setup and nonrecursive cleanup PASS |
| Dedicated PostgreSQL | FAILURE at new daily schedule target compilation |
| Required aggregate | FAILURE |

The same compiler diagnostics appear in Clippy, workspace and PG logs:
`cannot find module or crate tracing in this scope`, at the candidate's
`owner_beta_daily_support/mod.rs:107:46`, `:132:46` and
`owner_beta_daily_schedule.rs:238:50`. All three calls attempt to disable SQL
statement logging through an undeclared dependency. The existing SQLx 0.9
`ConnectOptions::disable_statement_logging` method disables both regular and
slow statement logging without that dependency. This is a bounded test-only
correction, not a change to runtime authorization or SQL guards.

The dedicated PG lane retains 32 named daily migration cases, 73 fixed subcases,
83 bootstrap cases, 60 exact committed UP SHA384 checksums in both lanes,
full-release NO_OP, direct service roles, 0039/catalog checks and cleanup PASS.
Twelve preceding DB target exits are zero with no SKIP, including eight actual
historical owner-beta publication/recovery rows. The new daily schedule target
exits 101 with the compiler errors; its 14 definitions are NOT_EXECUTED.
The later API admitter-pool target is not reached. Unprinted nested helper names
and inner exits remain UNKNOWN; outer success is not individual nested evidence.

The prior [Wave51 acceptance](2026-10-16-r2-test-correction-ci-acceptance.md)
is preserved independently. After full recovery, the source freeze was released
for the three logging sites and the prepared bounded daily failure/recovery
package. Fresh exact-candidate verification remains necessary. Full daily success
publication, complete approved-input runtime, independent integrated review,
I5-I7, operational activation, Owner use, three trading days and final C_R2 remain open.
