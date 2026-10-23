# R2 Wave64 I5-C01 full CI recovery

Roadmap R2-04/09; candidate3480afeff1666bef6a940046f94e2d3798a4ea96.
[CI38812634642 attempt1](https://github.com/sihioov/Lagrange/actions/runs/38812634642)
finished FAILURE. All eight terminal jobs and complete log hashes recovered;
seven actual checkouts,702pins and373committed blobs matched. No rerun/cancel/
duplicate occurred. Later documentation HEAD does not alter the tested candidate.

| Job | Outcome | Boundary |
| --- | --- | --- |
| format | PASS | Workspace format |
| clippy | PASS | Workspace/all-target/all-feature/-D warnings |
| policy | PASS | Exact hosted job |
| python-tests | PASS | 487 passed,10 existing skipped,7 warnings |
| workspace-tests | FAIL | Exactly one printed failure: daily state_no_fallback |
| postgres-integration-validation | FAIL | First17targets PASS; finaldailyAPI target failed |
| web | FAIL | OpenAPI/lint passed; five TS4111 test-only accesses |
| required | FAIL | Correctly retained execution failures |

I5 Rust:24/25 required named cases passed; only
http_daily_state_is_read_only_and_never_uses_generic_ready_data failed.
All listed carried workspace regression groups passed, including separate six
chart cases. Named results were matched within their phases, not inferred from
aggregate output. Three platform exclusions remain exclusions.

Dedicated PG ten new daily HTTP cases:9PASS/1FAIL, the same name with SQLSTATE22P02.
Two existing access/pool cases passed. Three selected price-only tests did not
execute: combined Cargo invocation stops after failed daily binary, no
--no-fail-fast; full log has no price-only startup. The ordinary workspace
--no-fail-fast run did execute its later binaries. Dedicated PG gate remains failed.
Publish8/schedule32/dailyPublish6/recovery6/runner24,daily32/subcase73/bootstrap83,
62migration SHA384 checksums twice,no-op/directroles/0039/catalog/cleanup passed.
Unprinted nested helper rows/inner exits remain UNKNOWN.

Web typecheck locations: r2-daily-api-contracts.test.ts63,144,165,168,172.
The existing noPropertyAccessFromIndexSignature requires bracket access to
Record<string,unknown>. Web unit/build/E2E following typecheck were skipped;
six new Web cases are NOT_EXECUTED. No UI acceptance is claimed.

The failing DB test uses migration_owner for both cross-table count observations.
jobs FORCE RLS expects actor UUID; owner-beta runs are actor-scoped; the audit
table has no migration_owner SELECT policy. This cannot observe the intended
cross-table deltas. The existing read-only admin pool has explicit SELECT
policies for all three. Switch only these two verification reads. Keep the
HTTP app pool, serving RLS, expected +1job/+1historicalrun/+0audit delta unchanged.
The log reports SQLSTATE but not the exact failing SQL statement; no runtime
statement-level root cause beyond that evidence is claimed.

Private receipt: target/r2-daily-beta/ci/3480afe/ci-evidence.json,
6486lines/cc4d3ff60b02972d9d8cef20344f737c449a83af8f00f3bd4569d7cee035e18c.
Web and PG diagnostic supplements preserve fixed source locations and full-log
hashes; raw logs were never persisted. Prior Wave62success/Wave63failure preserved.

Next Wave65 I5-C02 changes only the Web contract test and daily API HTTP test.
No product source, privileges, workflow, compiler/lint settings or assertions
are weakened. No new definitions. New exact-candidate CI only after bounded
source review, freeze, all-event head query0 and remote verification. No local
project execution or delegation. I5 acceptance,I7,I6,real threeUID/approved-base
integration,independent final review,G4/Owner/three trading days/C_R2 remain open.
