# WP-4/I0c-R — direct-login helper regression source review

Completed independent source-only review for R2-04/09 on 2026-10-14.
Actual startup: 09:30:26.183 UTC; reporting bound: 10:00:26.183 UTC.
Baseline: `c0ac599cb1ede900e155f724687c57410b4bd0d9`.
Reviewed source/brief commit: `f0c65905317be41689c6cb3b7a3fd63312ee0ac2`.

One concrete P2 source finding: redundant mutable transaction receiver borrows expose
both new tests to the strict Clippy warning gate. No P1 finding. The remaining written
guards and two-call wrapper delta match the fixed I0c contract. This report does not
accept source or establish compilation, lint success, test execution, DB privileges,
production readiness, or Owner/full C_R2 acceptance.

## Finding

**P2 I0c-R01 — remove redundant transaction receiver borrows.**

Anchors: [job-queue CREATE receiver](../../crates/job-queue/tests/owner_beta_publish.rs#L1308),
[job-queue SET ROLE receiver](../../crates/job-queue/tests/owner_beta_publish.rs#L1331),
[API CREATE receiver](../../crates/api-server/tests/http_owner_beta_access.rs#L254), and
[API SET ROLE receiver](../../crates/api-server/tests/http_owner_beta_access.rs#L277).

Each site uses `(&mut *first).begin()`. `PoolConnection<Postgres>` dereferences to
`PgConnection`; the imported `Connection::begin` takes `&mut self`. Method receiver
autoderef/autoref already supplies that borrow, making the explicit outer `&mut`
redundant. This is the `clippy::needless_borrow` pattern in the added tests.

Trigger: the required all-target Clippy check with `-D warnings` includes these two
integration-test targets. Consequence: the redundant receiver-borrow warning blocks
that gate and subsequent verification of the new regression tests. This conclusion is
static source analysis, not an observed fresh Clippy diagnostic; no compiler or lint
execution was performed.

Bounded correction: use `first.begin()` at all four sites with the existing
`sqlx::Connection` imports. Preserve the two separate transactions, typed SQLSTATE
checks, rollback-before-denial order, post-rollback identity checks, and cleanup.
Do not add a lint allowance or change the harness/helper APIs.

No other concrete delta defect was found in the reviewed scope.

## Frozen inputs and preservation

The private inventory is
`target/r2-daily-beta/wp-4-integration/source-wave-21-I0c-review-inputs.json`,
143 lines, SHA256 `aa87e4690a717f913584a755c871b33704a7032e2b92beb0c4a614f85770cd02`.
Its baseline/source-commit fields match this assignment. Four targets, 27 references
and two evidence artifacts matched exact SHA256 and newline counts before review;
the final verification repeats the same complete inventory check.

| Target | Lines | SHA256 | Pin result |
| --- | ---: | --- | --- |
| `crates/job-queue/tests/owner_beta_publish.rs` | 1381 | `37a358adc5f9c91e4c47cbe1770fd0ccf447d3e949b70d23531f456e70a659cc` | MATCH |
| `crates/api-server/tests/http_owner_beta_access.rs` | 327 | `82bbaf2b3c96e000c721e4e7cd21cd1a93e6fc5ff840e5f9a9d4927f5abe4e8a` | MATCH |
| `deploy/db/integration-validation/validate.sh` | 1711 | `29cdbb90ab07a442077af7f3a52d33992b001d50a734dd01e507a6a8569282f4` | MATCH |
| `deploy/db/integration-validation/static-check.sh` | 126 | `b12dad60ab21be461c991405dc0b48865780697555375d579e866ece033f8950` | MATCH |

The job-queue delta changes imports at line29 and appends the test at1206–1381.
The API delta adds imports at10–11 and appends the test at152–327, with its separator
at151. The validator adds only1706–1707; its static checker adds only51–52.
All four working files equal the committed source bytes.

| Baseline target | Lines | SHA256 |
| --- | ---: | --- |
| `crates/job-queue/tests/owner_beta_publish.rs` | 1204 | `97f865199dfbfa4b1ea78675843996ad7d200bd7ef83355b993723472f1b7da5` |
| `crates/api-server/tests/http_owner_beta_access.rs` | 148 | `bf6808bb98833fcf73197976cca88990973e368a08d122d6dc16cd442ee74bf5` |
| `deploy/db/integration-validation/validate.sh` | 1709 | `d2d4008249f0c79d64da7e00281861dfbc7a533e6e5793f585411e7693f6f6b3` |
| `deploy/db/integration-validation/static-check.sh` | 124 | `adb07a2152026df6210d1ea7afdd989aa9a0dca65d00f7ea2ea69f36f77d30a1` |

Removing each appended test and restoring its imports, including the job-queue
import order, reproduces its baseline bytes exactly. Removing the two added lines
from each shell file reproduces its baseline bytes exactly. The seven existing
job-queue tokio tests and one existing API tokio test are unchanged; exactly one new
tokio test is appended per target. Both new `check_result` blocks are byte-identical.
Every readonly reference also equals its baseline bytes; no helper, role/grant,
migration, product API, workflow, D4/D5 or earlier source boundary changed.

| Readonly reference | Lines | SHA256 | Pin result |
| --- | ---: | --- | --- |
| `deploy/secrets/runtime-static-check.sh` | 1023 | `0bede130507a141cd281ded771b0bfb00e77334809f79aabade8eff1c591365f` | MATCH |
| `deploy/db/bootstrap-roles.sh` | 303 | `66b91261cdefbb0709234c5c3b2d3399ccf96428594c7efd0c74d81910151ddd` | MATCH |
| `scripts/ops/provision-db-secrets.sh` | 613 | `2b2650c12fa3914f8258d40e034409dac3ce46cd03a8807646d0a898e4fa6125` | MATCH |
| `deploy/secrets/provision-runtime-secrets.sh` | 343 | `f3529be0ff55dc0f1f3866a8cd43aa4a83ffa62da36e95796dd3600a25974f5c` | MATCH |
| `scripts/ops/validate-production-config.sh` | 889 | `a472d2f8079985f124d66d7efddda2c11e2955858014b7325f563dd3b3ff9ba2` | MATCH |
| `deploy/compose/compose.yml` | 1632 | `434643b6c749e7ceca8abcca1f29726f55bd9ef86ee42c08db7e53ed1d4c7931` | MATCH |
| `deploy/compose/.env.example` | 149 | `e37029e2b47ef70360666e070eae591de4dc657186f430daef593bad19a9c330` | MATCH |
| `scripts/ops/self-test.sh` | 1483 | `496702920d258e7252d7da96503adc22deca6609cb1866110c7b0abe67230d17` | MATCH |
| `crates/market-data/src/storage.rs` | 3656 | `b1dca720210216d49d22858ac65b26a8f3cf230428629c9c4ec0ac64238e6fef` | MATCH |
| `crates/market-data/tests/daily_price_only_storage.rs` | 644 | `1deec0dc3060fbbf880b3801b78cf640b70aae7a1d0f1c99807fe41476000d45` | MATCH |
| `crates/job-queue/tests/common/mod.rs` | 401 | `242977cf561d54c88889feeb695c8de5b90a64910fd3fcbfde346a17a06e68fa` | MATCH |
| `crates/api-server/tests/common/mod.rs` | 933 | `6735a723f3c874c129ed43685f8633c372a8384ba2d23da407bebb7d18ed7dcb` | MATCH |
| `tests/integration/migration-contract/role-bootstrap.sql` | 94 | `9dfe008fdd08b49587940e14fc6f441b2a0053da5f48651fec98fdb53c31b85a` | MATCH |
| `tests/integration/migration-contract/bootstrap.sql` | 33 | `24fcb7f582f927d9d94827597c74989e005c3378530a3b9c5677e6ac29bf3c77` | MATCH |
| `crates/market-data/src/daily_price_only/contract.rs` | 4183 | `f086c065b903a00a022eeef3b37daf3b021691f0fa2fc525b17be1a7ed4ea523` | MATCH |
| `crates/market-data/tests/daily_price_only.rs` | 2385 | `1596ecbe2b673264fdec2f41605e043f3d7fb79ea54463ac5075ad9da3cd0fdd` | MATCH |
| `nt/live-node/tests/test_runtime.py` | 479 | `e2121f61d3be269b4b8b15313c6c0e8c4fe8ed34a218bac1c908cf72ae3dfb97` | MATCH |
| `docs/contracts/owner-beta-daily-etf11-input-v1.md` | 950 | `fa0529033945437e78eca5b0805f0524a4ae3c75fc4f0a36321ec56b052e717f` | MATCH |
| `docs/reviews/2026-10-14-r2-daily-input-design-amendments.md` | 740 | `8a8796d992da533389bac97fab6abf86c7e89e75f0772f6deceb3d4051d651a6` | MATCH |
| `.github/workflows/ci.yml` | 211 | `5df190a1327ce8689c04a8483efde7514cbb0f494ddd7d43907f11bd554858ce` | MATCH |
| `scripts/ops/static-check.sh` | 1150 | `8b45910a397f85661c33b791821a2ac69d231fd7576e5cfd6e45566e1c533998` | MATCH |
| `data-pipelines/collectors/src/pipeline.rs` | 1226 | `b31d2e830e10f17212826cad1f990203a437575e0d4cddc23aab2ec9954c2633` | MATCH |
| `data-pipelines/collectors/src/worker.rs` | 6797 | `9b03eef5aa5f3966d9d06c64bd0a2f42fc1e665457e3162e44d225cf7a3cf15a` | MATCH |
| `deploy/db/migrate-static-check.sh` | 264 | `1a5c8411b7496c40720ed0691097889fb2b6ec6be82178eac7da230bde747589` | MATCH |
| `crates/job-queue/Cargo.toml` | 66 | `a1ec5549e49dd36e384186d00daa7dedd67c49b70b0c51caaf0de151a4a3012d` | MATCH |
| `crates/api-server/Cargo.toml` | 67 | `4e6e430d35c9108ecaea1045a7b193e0ff2f25e8e92e19e1cf89f9dc05ae93ec` | MATCH |
| `Cargo.lock` | 5609 | `d3528661c8ad9d15d4cc7b6d4f4f91e530c851770b8a79751c2c2a680645783e` | MATCH |

References outside the concrete harness/dependency call path were hashed and compared
as bytes only. They were not surveyed as additional review scope. The concurrent Raw
snapshot implementation-contract output was neither read nor modified.

Read handoff: `wave20-I0c-final.md`, 21 lines,
SHA256 `158ff5c4275ec58a7337cda91936a025914d18b7ce20ffb8ee369e381c4e75a2`.
Read recovery: `wave20-I0c-recovery.json`, 39 lines,
SHA256 `9fb50dccb01dd76cc4aacc19f146387d2d26c03318cb95c1953a6aca223b431d`.
Both are under `target/r2-daily-beta/wp-4-integration/`; claims were checked against
source rather than adopted as execution evidence.

| Prior evidence artifact | Lines | SHA256 | Pin result |
| --- | ---: | --- | --- |
| `docs/reviews/2026-10-14-r2-caller-role-raw-ci.md` | 26 | `989a3891493f3ece231de15050f6bb3d139f1034edf3c0fe87e4aef4b686a47c` | MATCH |
| `target/r2-daily-beta/ci/c0ac599/ci-evidence.json` | 1861 | `9016b6c2c97c5e2b05763dec131f0e7199d8d9ed0017da53470e7b2694ab7b0e` | MATCH |

The [prior CI report](2026-10-14-r2-caller-role-raw-ci.md#L1) belongs to c0ac599 /
run38536460035 attempt1. Its reported49 named Linux Rust plus6 chart passing rows,
seven execution jobs and required success are baseline evidence only. The private
receipt was hashed, not used to claim nested execution of these new tests.

## Guard-to-written-test map and dispositions

JQ denotes `daily_input_admitter_pool_is_direct_bounded_and_unprivileged`.
API denotes `http_daily_input_admitter_pool_is_direct_bounded_and_unprivileged`.
All matrix entries describe written source; neither new test has been executed.

| Contract guard | JQ source | API source | Disposition |
| --- | --- | --- | --- |
| Existing scratch harness/helper and fixed absent-URL SKIP | [1208](../../crates/job-queue/tests/owner_beta_publish.rs#L1208) | [154](../../crates/api-server/tests/http_owner_beta_access.rs#L154) | Calls actual `create`/`new` and existing admitter helper; no replacement or impersonation. Configured setup failures remain failures. |
| Options maximum exactly2 | [1215](../../crates/job-queue/tests/owner_beta_publish.rs#L1215) | [161](../../crates/api-server/tests/http_owner_beta_access.rs#L161) | Actual cached SQLx accessor, not a local assumed constant. |
| Two held connections, direct current/session identities, role none, both actor GUCs SQL NULL | [1219](../../crates/job-queue/tests/owner_beta_publish.rs#L1219) | [165](../../crates/api-server/tests/http_owner_beta_access.rs#L165) | Six typed columns; both `Option<String>` values require `None`, not empty strings. Both handles stay held for the capacity probe. |
| Distinct backend PIDs | [1260](../../crates/job-queue/tests/owner_beta_publish.rs#L1260) | [206](../../crates/api-server/tests/http_owner_beta_access.rs#L206) | Compares initial simultaneously-held sessions. |
| Third acquire pending100ms | [1264](../../crates/job-queue/tests/owner_beta_publish.rs#L1264) | [210](../../crates/api-server/tests/http_owner_beta_access.rs#L210) | Requires outer timeout error; an immediate SQLx error also fails. No sleep or busy loop added. |
| Release then acquire within10s; size<=2 and identity preserved | [1271](../../crates/job-queue/tests/owner_beta_publish.rs#L1271) | [217](../../crates/api-server/tests/http_owner_beta_access.rs#L217) | Drops second handle; fresh acquisition requires `Ok(Ok(connection))`. Size checked initially, after reacquire and after rollback probes. |
| Exact seven role attributes | [1288](../../crates/job-queue/tests/owner_beta_publish.rs#L1288) | [234](../../crates/api-server/tests/http_owner_beta_access.rs#L234) | LOGIN true and other six false, queried through first actual admitter session. |
| Effective CONNECT/USAGE true; database/public CREATE false; no incoming/outgoing memberships | [1298](../../crates/job-queue/tests/owner_beta_publish.rs#L1298) | [244](../../crates/api-server/tests/http_owner_beta_access.rs#L244) | Exact five-boolean tuple; membership predicate covers both `member` and `roleid`. No grant repair or role mutation added. |
| CREATE denial, typed42501, rollback first | [1308](../../crates/job-queue/tests/owner_beta_publish.rs#L1308) | [254](../../crates/api-server/tests/http_owner_beta_access.rs#L254) | Written semantics match. Receiver lint finding I0c-R01 remains. Success result is captured then rolled back before failure is returned. |
| SET ROLE denial, typed42501, separate rollback first | [1331](../../crates/job-queue/tests/owner_beta_publish.rs#L1331) | [277](../../crates/api-server/tests/http_owner_beta_access.rs#L277) | Written semantics match. Receiver lint finding I0c-R01 remains. This is a denial probe, not login impersonation. |
| Post-rollback direct identities/role/both actor checks | [1353](../../crates/job-queue/tests/owner_beta_publish.rs#L1353) | [299](../../crates/api-server/tests/http_owner_beta_access.rs#L299) | Rechecks first and reacquired handles after both transactions. No actor mutation added. |
| Fixed diagnostics and cleanup before final failure | [1374](../../crates/job-queue/tests/owner_beta_publish.rs#L1374) | [320](../../crates/api-server/tests/http_owner_beta_access.rs#L320) | Handles leave async block; close pool, teardown, then panic only with static reason. New SQLx errors/URLs/query values are not formatted. |
| Ten official selections preserved; two appended filters | [validator1706](../../deploy/db/integration-validation/validate.sh#L1706) | [validator1707](../../deploy/db/integration-validation/validate.sh#L1707) | Exact required filters before final APPROVED; original ten unchanged. |
| Static checker requires both literal call lines | [checker51](../../deploy/db/integration-validation/static-check.sh#L51) | [checker52](../../deploy/db/integration-validation/static-check.sh#L52) | Two markers in existing `grep -Fq` loop; checker not executed. |

The [JQ helper](../../crates/job-queue/tests/common/mod.rs#L362) logs in directly with
max2 and no role/actor setup. The [API helper](../../crates/api-server/tests/common/mod.rs#L579)
changes connection options to the admitter username, max2, without role/actor setup.
[JQ creation](../../crates/job-queue/tests/common/mod.rs#L240) and
[API creation](../../crates/api-server/tests/common/mod.rs#L281) return `None` on the
established absent-URL path; configured setup errors panic rather than becoming SKIP.
Their existing setup diagnostics and setup-panic cleanup behavior are unchanged.
The new returned-error cleanup guarantee applies to the `check_result` block.

Each held `PoolConnection` is local to that completed async block; a `return Err` or
`?` drops its live handles before outer pool close. A successful early third acquire
also drops its temporary result before leaving the block. `rollback(self)` consumes
the transaction and ends its mutable borrow before the next transaction/query.
A rollback error returns a fixed reason; transaction drop queues remaining rollback,
then handle drop and awaited pool closure precede scratch teardown. No handle is
stored in `check_result`, which contains only unit or a static string.

The cached pool implementation documents acquire cancellation and async handle
return. With both actual handles held and max2 verified, the100ms pending probe has
no available connection to remove. After `drop(second)`, return-to-pool is async;
the10s acquisition awaits that return or a replacement, rather than assuming
synchronous handle return. Awaited `pool.close()` waits for checked-out returns and
closes idle connections before [JQ drop](../../crates/job-queue/tests/common/mod.rs#L388)
or [API teardown](../../crates/api-server/tests/common/mod.rs#L655). Actual scheduling,
cancellation and database behavior remain unverified.

The unchanged [wrapper](../../deploy/db/integration-validation/validate.sh#L1377)
passes DATABASE_URL to locked Cargo, records sanitized evidence, fails nonzero exits,
rejects any SKIP marker and appends successful labels. Its existing
[cleanup trap](../../deploy/db/integration-validation/validate.sh#L659), evidence/outcome
mapping and [bootstrap gate](../../deploy/db/integration-validation/validate.sh#L1136)
are unchanged by exact byte reconstruction. The two new filters select the added
names, not all historical tests in those binaries. Wrapper source is not evidence
that either selected test actually ran.

## Type/API and verification evidence

Read root AGENTS.md, ROADMAP R2-04/09, Wave20/Wave21 and D5 A5/I0 requirements.
Repository instruction discovery found no applicable subdirectory AGENTS/CLAUDE
file for these paths; the only additional files found are under `apps/web`.
The assignment's single-report scope governs roadmap/diagram/source immutability.

Read the two full additions and exact four-file committed delta; adjacent reads were
limited to both common harnesses, role bootstrap SQL, package manifests/Cargo.lock,
D5 I0 requirements and relevant existing validator sections. No parser/storage
architecture or concurrent snapshot contract was reviewed.

Both manifests provide SQLx postgres/runtime and tokio time/macros support.
Cargo.lock pins SQLx, sqlx-core and sqlx-postgres0.9.0 at4240/4253/4356.
Cached dependency source was read under
`/home/l1nnx/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`:
`sqlx-core-0.9.0/src/{pool/mod.rs,pool/options.rs,pool/connection.rs,connection.rs,transaction.rs,from_row.rs,error.rs,query_as.rs,sql_str.rs}`
and `sqlx-0.9.0/src/lib.rs`.

Those sources provide `Pool::options` (567), `get_max_connections` (179), `size` (535),
`acquire` (357), awaited `close` (441), deref to the DB connection (connection52/60),
`Connection::begin(&mut self)` (connection51), consuming rollback (transaction128),
transaction deref (217/229), handle drop (pool/connection199), tuple FromRow coverage
(from_row326 onward), typed database-error code access, and `SqlSafeStr` for static
strings. The identity SQL variable contains a static literal; no dynamic SQL wrapper
or free-form construction was introduced. The six/five/seven-column tuples match
selected column counts and nullable actor fields. Imports are used by the added
transaction/timer calls. No new blanket warning allowance is present. Apart from
I0c-R01, no concrete type/borrow/API or strict-lint defect was identified by inspection;
this is not compiler or all-target lint validation.

Actual checks performed:

- SHA256/newline counts: all4 targets +27 references +2 evidence pins MATCH.
- Committed source equality: all4 targets MATCH f0c6590.
- Baseline byte reconstruction and unchanged-reference comparison: PASS. The first
  inert comparison failed because its restoration omitted job-queue import reorder;
  restoring the actual diff's import order made all comparisons exact. No source changed.
- New-name count and preservation: JQ7+1, API1+1; shared new check blocks identical.
- `rustfmt --edition 2024 --check --config skip_children=true` on both targets: exit0.
- `bash -n` on both shell targets: exit0.
- Scoped `git diff --check c0ac599... f0c6590... --` four targets: exit0.
- Final inventory recheck: all4 targets +27 references +2 evidence pins MATCH.
- Report42 local links/line bounds and whitespace: PASS; no repository
  validator/checker/self-test was invoked.

## Remaining evidence and execution boundaries

Unresolved source follow-up: I0c-R01, four receiver expressions in the two tests.
Coordinator owns bounded correction/acceptance and any future single exact-candidate
hosted verification. No repair was implemented by this reviewer.

Not verified: compilation, all-target `-D warnings` Clippy, either new test, actual
nested no-SKIP execution, real SQLSTATE/privilege/identity results, wall-clock timeout
behavior and successful teardown under a real configured database. Formatting and
bash syntax success establish none of those results. No new CI was queried or run.

Missing inputs: none. Conflicting requirements: none. Required APIs not found: none.
Other concrete findings: none. Scope deviations: none; no production/provider/source
writes or operations occurred. The memory-context hook returned MCP_ERROR and yielded
no usable history; review relied on fresh pinned source/docs. The own fresh finish
hook returned SKIPPED with `no_durable_change`, saved=false; no memory content was stored.

Negative bootstrap corruption/rollback probes,0060, provider/Raw snapshot/admission,
real OS UID/ACL, runtime and Owner/full C_R2 acceptance remain accepted later-package
boundaries, not findings or missing I0c requirements. D4/D5 remain exact.

Further decomposition needed for this review: no. Review scope completed directly
within its bound. Delegation: none. Background review work/processes: none.
Permission waits/pending permissions: none. Auto-review denial: none.
Only this assigned report was written; all frozen inputs are preserved.
