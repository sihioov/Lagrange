# R2 I4g2 admitter readback — hosted failure recovery

Date: 2026-10-16 KST. Roadmap R2-04/R2-09, Wave60.
Coordinator direct evidence review; not independent integration review.
Candidate: `ef6227921be0a45b6947a03b98441d8fc343eff5`.
[Run38796372356 attempt1](https://github.com/sihioov/Lagrange/actions/runs/38796372356)
completed FAILURE. CI/ci.yml, workflow_dispatch, excited-dolphin.
Full recovery: 2026-10-15T19:59:10.819424+00:00. Later documentation HEAD is not the candidate.

The source is not accepted. Workspace tests, strict Clippy and dedicated PostgreSQL
validation share one E0277 test-compilation defect at
[owner_beta_daily_schedule.rs:557](../../crates/job-queue/tests/owner_beta_daily_schedule.rs#L557).
The new direct-principal test builds a String from two fixed function names, then
passes its reference to SQLx0.9 query_scalar. SQLx requires a SqlSafeStr input; the
installed pinned sqlx-core0.9.0 source implements it for static string literals.
This is a test-query construction defect; no SQL execution or injection is inferred.

| Evidence | Observation |
|---|---|
| Jobs and logs | All8 terminal jobs,8 full-log hashes,7 actual checkout SHAs match |
| Frozen source | All603pins and338 committed candidate blobs match |
| Success jobs | policy,format,python-tests,web |
| Failure jobs | workspace-tests,clippy,postgres-integration-validation,required |
| Workspace |6 chart names pass; subsequent workspace compilation fails before test binaries |
| New definitions |18 readback definitions NOT_EXECUTED; all required workspace groups NOT_EXECUTED |
| Dedicated schedule |28 names NOT_EXECUTED: exact target compile diagnostic and exit101 |
| Other PG |32 staged0060,73subcases,83bootstrap negatives,8historicalpublish names pass |
| Full release | Both62UP SHA384 sets match; NO_OP,roles,0039,catalog and cleanup pass |
| Remaining PG groups |6daily publish,6recovery,24runner rows absent and UNKNOWN in receipt |
| Web |34supervisor and121off+14stream unique cases pass; no retry/flaky/skip; cleanup pass |
| Python |487pass,10skip,7warnings |
| Steps |81success,4failure,1skipped; no skipped gate promoted to success |

The dedicated PG aggregate is not APPROVED. Passed earlier stages do not establish
the new real-role readback tests. Unprinted nested helper rows and inner exits
remain UNKNOWN. Exact rows, stage observations and failure locations are retained;
aggregate totals are not unique test evidence. All prior failures remain preserved.

Private receipt: `target/r2-daily-beta/ci/ef62279/ci-evidence.json`, 7146lines,
SHA256 `a58c2eb7a1bfdc6be764c8bb353067c96094f7c139f703b24046aa892bb48b0d`. Directory0700/files0600; raw logs processed in memory only.
No local project/test/compiler/SQL/DB execution, provider access, delegation or
production activation. Failed run was not rerun, canceled or duplicated.

Bounded C01: replace only this test's dynamic query construction with the two exact
fixed SQL literals. Preserve the complete principal/table-grant assertions,
production Rust,0060/0061/0062,harness,18 definitions and28 dedicated PG names.
Do not use AssertSqlSafe, blanket lint suppression or weaker guards. New direct
review, successor pins/inventory and one new exact-candidate hosted run are required.

Full producer/admitter state and CLI, real approved input, I5-I7, independent
integration review, UID/ACL/crash/lost-reply evidence, I6 diagrams before main,
operational activation, Owner use, three trading days and final C_R2 remain open.
