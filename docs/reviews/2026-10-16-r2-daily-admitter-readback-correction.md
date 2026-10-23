# R2 I4g2 C01 — static SQL privilege queries

Date: 2026-10-16 KST. Roadmap R2-04/R2-09, Wave61.
Direct coordinator review, not independent integration review.
Baseline: `ef6227921be0a45b6947a03b98441d8fc343eff5`.
[Failed candidate fully recovered](2026-10-16-r2-daily-admitter-readback-ci.md).

The direct-principal test now passes two fixed SQL literals to SQLx0.9 query_scalar.
The statements and bound receipt ID are identical to the previous format output.
Each app, worker and supervisor pool still must receive SQLSTATE42501 for both
functions. The admitted-input query, SET ROLE rejection, and table-grant matrix
remain unchanged. SqlSafeStr is satisfied by literal types; no AssertSqlSafe bypass,
new capability, lint suppression or relaxed assertion was added.

Only [owner_beta_daily_schedule.rs](../../crates/job-queue/tests/owner_beta_daily_schedule.rs#L549)
changes: one hunk +5/-3, final 944lines, SHA256
`373a75e5a1dbb4d0919b1a60af3e751e3dabf408716477491d2e4173bb1b89fa`. All code outside that single test is byte-identical
to the failed candidate. Production Rust,0060/0061/0062 and validation harness are
unchanged. All18readback definitions and28dedicated PG names are retained.

Scoped standalone edition2024 rustfmt check and whitespace check pass. Source
inspection confirms both query literals exactly match the old two-name template.
The frozen successor inventory records updated source lines and explicit unexecuted
status for this new candidate. This does not establish compiler, lint or test success.

One fresh exact-candidate hosted run follows source freeze, committed blob/pin
comparison, all-event existing-run query and remote SHA equality. Never rerun or
cancel the failed candidate. No local Cargo/compiler/test/project helper/SQL/DB,
provider access, production action or delegation occurred.

After full hosted recovery, continue complete producer/admitter state and process
wiring. Approved-base runtime, distinct UID/ACL, crash/lost transport, I5-I7,
independent integration review, I6 diagrams, operational/Owner/three trading days
and final C_R2 remain open. Ralph continues.
