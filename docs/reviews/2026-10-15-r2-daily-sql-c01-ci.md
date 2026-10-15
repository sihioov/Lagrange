# R2 I2-C01 — exact-candidate hosted verification

2026-10-15; candidate `6fbc97623dda8d31b2d24e45266a8d6f84d25d0f`.
[CI 38690484091](https://github.com/sihioov/Lagrange/actions/runs/38690484091),
attempt1, CI/ci.yml, workflow_dispatch, excited-dolphin, sihioov/Lagrange.

**IN_PROGRESS; no acceptance.** [C01 source disposition](2026-10-15-r2-daily-sql-c01.md)
follows complete recovery of the failedf5122dd attempt. All230 frozen pins and197
candidate blobs match. An all-event exact-head query returned0 and remote SHA
matched at06:06:32 UTC; one dispatch created the run at06:06:33. No rerun/cancel.

Recover8 jobs/steps/logs,7 actual checkout SHAs,32 named DB tests/73 subcases,
99 prior Rust/6 separate chart/83 bootstrap/3 transport, strict lint/format,
60-checksum/no-op/roles/rollback/12 numeric DB exits and cleanup. Missing results
remain UNKNOWN unless stage evidence proves NOT_EXECUTED. Raw logs stay in memory;
only sanitized receipts are retained. No local execution or new delegation.

Private inputs: wave38-I2-C01-source-inputs.json and
wave38-I2-C01-definition-inventory.json under target/r2-daily-beta/wp-4-integration.
The predecessor failure and all earlier partial successes remain preserved.
R2-04/09, I3–I7, runtime/Owner/three trading days and final C_R2 remain open.

## Complete recovery — 2026-10-15

The launch status above is historical. Attempt1 completed FAILURE at06:29:15 UTC.
All8 jobs/84 steps and their log hashes were recovered:80 success,3 failure,
1 skipped step;7 actual checkout SHAs match the candidate. All230 frozen pins
and197 tracked candidate blobs matched before any C02 source edit.

Format, strict Clippy (`--workspace --all-targets --all-features --locked -- -D
warnings`), policy, Python and Web including E2E passed. Workspace tests,
PostgreSQL validation and the required aggregate failed.

- Workspace: all99 prior R2 qualified Rust names,6 separate chart names and3
  transport names passed. The32 new daily definitions produced16 PASS/16 FAIL.
  Another28 legacy cases failed:6 owner-beta publication,18 queue contract,
  2 manifest-writer and2 robustness-worker. These44 failed named rows are
  retained individually; the2974 printed rows are not a unique acceptance count.
- New daily failures reached SQLSTATE42702 (one assertion expected23514 and
  instead received42702). SQL source inspection identifies a local `k` and a
  derived-column `k` in manifest aggregation. The bounded qualification must
  still be proved by fresh hosted DB execution.
- Legacy publication fixtures received permission denial on
  user_strategy_configs; they explicitly supplied an ID that the new column
  grant intentionally excludes. Other legacy failures received jobs permission
  denial. The new deferred consistency trigger reads/locks non-daily jobs before
  returning, while those fixtures install older job tables as postgres.
- The dedicated PG lane applied0060 and passed its actual-role/unused-DOWN/
  hidden-history rollback harness. Both full-release passes have60 SHA384
  migration checksums matching candidate blobs, with NO_OP rerun, seven direct
  principals in both lanes,0039 guard and four catalog negative probes. All83
  bootstrap cases passed. Cleanup is PASS_NO_OWNED_CONTAINERS.
- Its first migration-contract build exited101: three synthetic UUID JSON values
  at test lines12773/13410 require Serialize in an isolated build. Workspace
  feature unification masked this. The dedicated32 definitions/73 subcases and
  later11 DB labels were NOT_EXECUTED. Workspace success output does not print
  their73 subcases; those row results remain UNKNOWN, not inferred from parents.

Private complete receipt: target/r2-daily-beta/ci/6fbc976/ci-evidence.json,
3346lines, SHA2568e032c320f40374d1c7962d3d96c6c8ac3ae3c18b733ed7d5fa7a5a69042b8ca.
The earlier partial observation and bounded diagnostic rereads are preserved;
the workspace log digest agrees across them. Raw bodies were never persisted.

C01's earlier parser/type/lint conditions are resolved at their observed stages.
I2 remains unaccepted. C02 is limited to manifest identifier qualification,
non-daily job trigger scoping, UUID string construction and actual-app fixture
identity allocation. No role expansion, dependency feature, guard weakening,
rerun/cancel, local execution or delegation is authorized by this partial result.
