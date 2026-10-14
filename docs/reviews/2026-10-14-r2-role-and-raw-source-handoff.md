# R2 I0b / I1b-S1 source handoff — 2026-10-14

R2-04/09 remains **DOING**. This handoff freezes the two source packages for independent
review; it does not accept their behavior or extend the previously accepted CI result.

The accepted source baseline is `929fcd68eb99be5ec18bbbfbb585e6cc17655fee`, with
[successful hosted CI](2026-10-14-r2-harness-lint-ci.md). The package brief was committed
in `0a98b9f`, with startup documentation in `da0499e`. The present source changes belong
to the commit adding this handoff. No new workflow was dispatched for these bytes.

## Recovery and scope

The I0b worker `fd32e206-9248-41cd-a86b-de07a33ea464` finished at 05:51:34 UTC;
I1b-S1 `14d3d7c8-892a-4434-baa6-29de925837cb` finished at 05:53:34 UTC.
The coordinator recovered both full reports and verified actual idle, no active turn,
no pending permissions, and the expected workspace/parent. The source-only diff contains
exactly the nine I0b files and two I1b-S1 files below.

I0b supplies a transactional restricted-role bootstrap, the eighth source credential and
scoped projections, Compose bootstrap binding, and isolated-validator checks. The Raw sibling
supplies explicit bounded, read-only manifest records and their original line hashes.
Neither package establishes migration 0060 readiness or a complete approved input snapshot.

## Frozen source inputs

These hashes were computed from the actual files after both workers became idle.

| Path | Lines | SHA-256 |
| --- | ---: | --- |
| [crates/market-data/src/storage.rs](../../crates/market-data/src/storage.rs) | 3654 | `a2c786c27d5031fbff0a413c1ccb8aa3e9e34fac287ae9a2dc8be1564bad6444` |
| [crates/market-data/tests/daily_price_only_storage.rs](../../crates/market-data/tests/daily_price_only_storage.rs) | 558 | `03a20807c06553848699169e4a6422ac8604108f0765550b281eb680a9f7e047` |
| [deploy/compose/.env.example](../../deploy/compose/.env.example) | 149 | `e37029e2b47ef70360666e070eae591de4dc657186f430daef593bad19a9c330` |
| [deploy/compose/compose.yml](../../deploy/compose/compose.yml) | 1632 | `434643b6c749e7ceca8abcca1f29726f55bd9ef86ee42c08db7e53ed1d4c7931` |
| [deploy/db/bootstrap-roles.sh](../../deploy/db/bootstrap-roles.sh) | 303 | `66b91261cdefbb0709234c5c3b2d3399ccf96428594c7efd0c74d81910151ddd` |
| [deploy/db/integration-validation/validate.sh](../../deploy/db/integration-validation/validate.sh) | 1453 | `0676d8839a6fcde7f280371aff84868ec83591311832db5c1ce7a9b9e993838a` |
| [deploy/secrets/provision-runtime-secrets.sh](../../deploy/secrets/provision-runtime-secrets.sh) | 343 | `f3529be0ff55dc0f1f3866a8cd43aa4a83ffa62da36e95796dd3600a25974f5c` |
| [deploy/secrets/runtime-static-check.sh](../../deploy/secrets/runtime-static-check.sh) | 418 | `fa2a9ea7559a09ce0ea53763008836776b017eb1a297702a7509b1fc176f923f` |
| [scripts/ops/provision-db-secrets.sh](../../scripts/ops/provision-db-secrets.sh) | 613 | `2b2650c12fa3914f8258d40e034409dac3ce46cd03a8807646d0a898e4fa6125` |
| [scripts/ops/self-test.sh](../../scripts/ops/self-test.sh) | 1483 | `496702920d258e7252d7da96503adc22deca6609cb1866110c7b0abe67230d17` |
| [scripts/ops/validate-production-config.sh](../../scripts/ops/validate-production-config.sh) | 887 | `5ee7223a8012ce480ad3493878fe3494bab87fa90021000e8a7ac72827165510` |

The I0b final report supplied a **65-character**, invalid SHA-256 for
`validate-production-config.sh`. Its actual 64-character digest is the one above.
The original report is preserved; the coordinator did not treat that value as verified
or mutate the source to match it. The other ten reported source hashes and all eleven
line counts match. The exact hash assertion and strict report-table parser initially
rejected this malformed field; subsequent recovery preserves the discrepancy.

The four I0a reference files, pure wire source/test, Python readiness test, D4 and D5
remain byte-identical to the accepted source baseline. D4 is 950 lines with SHA-256
`fa0529033945437e78eca5b0805f0524a4ae3c75fc4f0a36321ec56b052e717f`; D5 is 740 lines with
`8a8796d992da533389bac97fab6abf86c7e89e75f0772f6deceb3d4051d651a6`.

## Checks and limits

- Coordinator checks passed: seven shell syntax checks, scoped edition-2024 rustfmt check,
  Git whitespace check, exact source scope, hashes/line counts, and nine unchanged references.
- The locally installed Node `yaml` parser accepted Compose with explicit YAML 1.1 semantics.
  Bootstrap credential binding, `999:999` / octal `0400`, and absence of new daily services
  were checked. A prior YAML 1.2 checker interpreted `0400` as decimal 400; correcting the
  checker resolved that diagnostic. This is static parsing, not Docker Compose execution.
- The worker's PyYAML check was skipped because that package was absent; no dependency was installed.
- Shell validators/self-tests, Rust compilation/Clippy/tests, SQL, Docker, root, production,
  provider and secret-provisioning commands were not executed. Written tests are unexecuted.

The new validator's `current_setting('role')` observation must remain distinct from the
actor GUC and pool helper behavior. Negative bootstrap attributes/membership/CREATE and
rollback, actual login/privileges, and credential non-disclosure need appropriate execution
proof. Review must also inspect failure-log paths rather than infer them from success checks.

Raw review must check original byte retention, pre-dedup limits, descriptor/path binding and
replacement guards, trust UID/mode enforcement, typed failures and no-write behavior. The
written UID fixture changes the expected UID; it does not exercise a tree owned by another
OS user or ACLs. Non-Unix behavior and all new regressions remain unexecuted.

## Next gate and retained work

Two independent reviews have separate report ownership under the
[execution plan](../superpowers/plans/2026-10-14-r2-daily-input-implementation.md).
Source bytes remain frozen during review. Concrete findings require bounded corrections,
then a new exact-candidate hosted CI run; the old `929fcd6` result cannot validate this delta.

The new PostgreSQL role and moved Compose evidence require runtime diagram source/PNG
integration in I6 before or with the structural merge to main. No diagram rendering or
main merge occurs in this handoff. Remaining I0/0060, batch/provider-file verification,
copy/normalization/admission, runtime activation and Owner acceptance remain open.

Private recovery evidence is under `target/r2-daily-beta/wp-4-integration/`:
`source-wave-8-recovery.json`, `source-wave-8-final-inputs.json`, and both `wave8-*-final.md`
reports. Directories/files retain 0700/0600 modes. Mem0 context returned MCP_ERROR; no new
source conclusion was recorded as durable memory.
