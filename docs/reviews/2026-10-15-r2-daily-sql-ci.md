# R2 I2 — first exact-candidate hosted verification

2026-10-15; coordinator WP-4/I2-CI, Wave37, R2-04/R2-09.
Candidate `f5122dd5cc1725213cd0adef8f3ec4689a3460ad`.
[CI 38689151958](https://github.com/sihioov/Lagrange/actions/runs/38689151958),
attempt1, CI/ci.yml, workflow_dispatch, excited-dolphin, sihioov/Lagrange.

## Dispatch evidence

Remote candidate matched and an all-event exact-head query returned zero at
05:52:25 UTC. One dispatch completed at05:52:26; run metadata was recovered at
05:52:45 with matching SHA, event, workflow and attempt1. No rerun, cancellation
or duplicate dispatch. Subsequent documentation commits do not change this target.

The [source disposition](2026-10-15-r2-daily-sql-source.md) is coordinator inspection.
All224 frozen source/reference/artifact pins and195 tracked candidate blobs matched.
Private wave37-I2-final-source-inputs.json supersedes the preliminary receipt after
one UP EOF blank line was removed during staged whitespace inspection; original
receipts are preserved. The32 new definitions and73 named subcases are frozen in
wave37-I2-definition-inventory.json. Prior99 Rust,6 separate chart,83 bootstrap,
three transport and12 DB-label expectations are preserved.

## Recovery status at launch

**IN_PROGRESS; no executable acceptance.** The coordinator will recover all jobs,
steps, actual checkout SHAs, complete log hashes, direct test/subcase rows,
60-checksum/no-op ledgers, direct-role/rollback evidence and cleanup. Missing rows
remain UNKNOWN unless a failed or skipped stage proves NOT_EXECUTED. Aggregate
counts alone do not prove individual tests. Unprinted nested helper rows retain
their prior UNKNOWN status; new printed numeric exits are evaluated separately.

Raw hosted logs remain in memory only. Retained receipts contain sanitized fixed
metadata, named rows and diagnostics. No local compiler/test/SQL/DB/helper/Docker,
provider or production execution; no new worker or delegation.

R2-04/09 remain DOING. The old57ca4d1/run38672658570 partial acceptance and all prior
failures remain intact. Provider/copy reconstruction, I3–I7, runtime/Owner,
three trading days and final C_R2 are open. This CI does not start observation.

## Full result recovery

**FAILURE**, all8 jobs terminal and all8 full log bodies recovered;7 actual
checkout SHAs,224 pins and195 candidate blobs matched.84 steps:78 success,
4 failure,2 skipped. Raw logs were not persisted. Additional read-only diagnostic
extractions of workspace/PG logs preserved matching log hashes.

- Format, policy, Python and Web including E2E succeeded. Python485 pass,
  10 skip,7 warnings is a suite summary, not individual-node evidence.
- Strict Clippy failed on unused import at test line12335, E0521 at12177 and
  E0277 at13156. These are new test helper compilation/audit conditions.
- PostgreSQL reached0059, then0060 application failed. Chart6 each failed in
  shared migration setup with SQLSTATE42601, `syntax error at end of input`,
  migration character45953 (line649). The following workspace test step was skipped.
- All99 prior workspace Rust rows,32 new DB cases and73 subcases are
  **NOT_EXECUTED** for this candidate. Chart6 are **FAILED**, not unknown or passed.
- All83 bootstrap names passed. Staged0039 catalog/negative probes/guard and
  direct role login evidence precede the failure;0060/full-release/DB-test gates
  did not run. Workspace QA cleanup succeeded. PG trap cleanup output is retained
  by its full log hash; complete no-owned-container absence is not asserted here.

Private `target/r2-daily-beta/ci/f5122dd/ci-evidence.json`:3528lines,
SHA256 `107618bfa0e98505e2de0c77eedcdb6ae7f6a8c35a84a3d28d210ed2424c8f99`.
The original receipt and SQL diagnostic supplements are0600 under a0700 directory.

[Bounded C01 correction](2026-10-15-r2-daily-sql-c01.md) follows only after full
recovery. No acceptance, old-run rerun/cancel or production action. Prior57ca4d1
acceptance remains unchanged; R2-04/09 and final C_R2 remain open.
