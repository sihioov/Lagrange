# R2 WP-4 I0a-R — isolated role/harness source review

2026-10-14, Asia/Seoul. Independent bounded review, intermediate/high confidence,
assigned codex/gpt-6.1-sol high. Workspace `wks_10fae7d550ab1704`, checkout
`/data/worktrees/3puw275b/excited-dolphin`. Roadmap scope: R2-04/09, WP-4/I0a-R.
Coordinator owns acceptance, documentation integration, Git and later execution.

**No concrete findings: HIGH 0, MEDIUM 0, LOW 0.** The four-file delta meets the
fixed I0a source brief within the reviewed boundaries. This is a source-review
conclusion, not compilation, SQL execution, actual-role/pool/ACL acceptance or full
I0 readiness. U2 source authorization persists; no operational action is authorized.

The preceding review turn stopped after source checks and an interim assessment,
before creating this report. Its cause is unknown. This continuation uses those
completed reads/checks and verifies final input stability; it does not infer an
execution result from the interruption. D6 was not altered or re-reviewed. Ongoing
I1a market-data files were not inspected as a completed artifact or modified.

## Inputs and checks against the fixed brief

Read applicable AGENTS, current ROADMAP and the final
[I0a brief](../superpowers/plans/2026-10-14-r2-daily-input-implementation.md#L317) and
[I0a-R brief](../superpowers/plans/2026-10-14-r2-daily-input-implementation.md#L363).
Supporting design scope is [D5 A5](2026-10-14-r2-daily-input-design-amendments.md#L445).
Reviewed the exact four working-tree files against baseline
`c073d7c6a0ba6f148e642a7197187f073c540e0d`. Initial HEAD was that baseline; resumed
documentation HEAD is `6f7d0e223f2c5eb3dda9be00c520c6b23e31160e`. The reviewed four
files still match the supplied frozen hashes and line counts; no mismatch was found.

| Reviewed file | Lines | SHA-256 |
| --- | ---: | --- |
| [JQ common](../../crates/job-queue/tests/common/mod.rs) | 399 | `973cb28656db8fd59e17f302f022ec167383b291bd1c0dd095b36425b62ac69d` |
| [API common](../../crates/api-server/tests/common/mod.rs) | 933 | `6735a723f3c874c129ed43685f8633c372a8384ba2d23da407bebb7d18ed7dcb` |
| [Cluster role bootstrap](../../tests/integration/migration-contract/role-bootstrap.sql) | 94 | `9dfe008fdd08b49587940e14fc6f441b2a0053da5f48651fec98fdb53c31b85a` |
| [Scratch DB bootstrap](../../tests/integration/migration-contract/bootstrap.sql) | 33 | `24fcb7f582f927d9d94827597c74989e005c3378530a3b9c5677e6ac29bf3c77` |

### Role creation and fail-closed membership checks

Absent-role creation at [JQ:65–70](../../crates/job-queue/tests/common/mod.rs#L65),
[API:79–84](../../crates/api-server/tests/common/mod.rs#L79) and
[role-bootstrap.sql:43–48](../../tests/integration/migration-contract/role-bootstrap.sql#L43)
sets LOGIN, NOINHERIT, NOSUPERUSER, NOCREATEDB, NOCREATEROLE, NOREPLICATION and
NOBYPASSRLS explicitly. Existing roles are not ALTERed or repaired. The following
preflight checks every required boolean, rejects any pg_auth_members edge with the
admitter as member or roleid, and rejects app/worker reachability to migration_owner
or the admitter. See [JQ:71–115](../../crates/job-queue/tests/common/mod.rs#L71),
[API:85–129](../../crates/api-server/tests/common/mod.rs#L85) and
[SQL:50–94](../../tests/integration/migration-contract/role-bootstrap.sql#L50).

The recursive closure follows member→roleid, retains the originating app/worker OID,
and uses UNION rather than UNION ALL. Its finite OID-pair state deduplicates revisits,
so cycles do not cause an unbounded walk. It considers membership edges regardless
of inheritance or SET-role option flags; NOINHERIT therefore cannot conceal a path.
This intentionally conservative rejection follows the fixed brief. A local text
comparison found the three preflight bodies identical after whitespace normalization.
These observations do not establish actual catalog state or PostgreSQL execution.

### Scratch-only privileges before migration

The new per-database SQL grants CONNECT on safely quoted current_database(), revokes
PUBLIC CREATE on scratch public, grants admitter USAGE and checks effective CONNECT,
USAGE, absence of database/schema CREATE, and retained migration_owner schema CREATE.
See [JQ:120–144](../../crates/job-queue/tests/common/mod.rs#L120),
[API:135–159](../../crates/api-server/tests/common/mod.rs#L135) and
[bootstrap.sql:8–33](../../tests/integration/migration-contract/bootstrap.sql#L8).
API and static SQL explicitly regrant migration_owner CREATE; JQ retains its existing
grant from ROLE_BOOTSTRAP_SQL and verifies it after the PUBLIC revoke.

The JQ call is on fresh scratch connection before MIGRATOR.run at
[JQ:275–297](../../crates/job-queue/tests/common/mod.rs#L275). API parses supervisor
connection options, replaces only the database with its generated scratch name, runs
the new SQL, closes that setup pool, then opens migration_owner and runs migrations at
[API:303–328](../../crates/api-server/tests/common/mod.rs#L303). The migration-contract
caller executes cluster roles under the existing supervisor transaction lock, creates
the scratch DB and executes bootstrap.sql on the scratch connection at
[migration_contract.rs:6571–6605](../../tests/integration/migration-contract/tests/migration_contract.rs#L6571).

No new admitter grant/revoke is present in either Rust cluster-bootstrap string; the
static role-bootstrap file has no GRANT/REVOKE or database-local schema operation.
The existing Rust bootstrap schema grants were already present at c073d7c and remain
baseline behavior. They are not a newly introduced supervisor mutation in this delta.
Effective-privilege calls are source-level checks, not evidence that ACLs were executed.

### Direct login, connection options and cleanup

Both helpers return PgPool, cap it at two connections, use the fixed fixture login
and verify current_user=session_user=daily_input_admitter plus an absent actor GUC.
See [JQ:356–384](../../crates/job-queue/tests/common/mod.rs#L356) and
[API:575–606](../../crates/api-server/tests/common/mod.rs#L575). Neither calls SET ROLE,
actor_pool, after_connect role switching or a GUC setter. API replaces username/password
on parsed app_url options; JQ uses the existing role_url targeting the scratch name.
New connection/identity failures use fixed panic messages, without printing the URL
or submitted credentials. Identity checks reject unexpected session state rather than
clearing it to make the assertion pass. No pool was opened during this review.

Generated DB identifiers remain the existing fixed-prefix numeric/counter names:
[JQ:153–155/249–254](../../crates/job-queue/tests/common/mod.rs#L153) and
[API:164–175](../../crates/api-server/tests/common/mod.rs#L164). New dynamic SQL uses
PostgreSQL format('%I', current_database()); API setup uses PgConnectOptions.database(),
not caller-controlled SQL interpolation. Existing URL construction was not refactored.

The JQ supervisor transaction lock at
[JQ:256–268](../../crates/job-queue/tests/common/mod.rs#L256), API's existing process mutex
and embedded session lock at [API:272–294](../../crates/api-server/tests/common/mod.rs#L272)
and [47/132](../../crates/api-server/tests/common/mod.rs#L47), and migration-contract's
lock remain unchanged. Their different mechanisms are baseline, not a new defect or
a verified cross-harness concurrency guarantee. Existing teardown remains at
[JQ:386–398](../../crates/job-queue/tests/common/mod.rs#L386) and
[API:654–669](../../crates/api-server/tests/common/mod.rs#L654). Returned admitter pools
are expressly caller-owned and must be closed before dropping the scratch DB; the
new API setup pool is closed before migration. No runtime cleanup success is claimed.

Source usage is plausible against the project's SQLx0.9 dependency declarations at
[JQ Cargo.toml:43](../../crates/job-queue/Cargo.toml#L43) and
[API Cargo.toml:53](../../crates/api-server/Cargo.toml#L53). Existing helper patterns and
local cached SQLx0.9.0 source were read: PgConnectOptions username/password/database
take &str and return Self; PgPoolOptions.connect_with accepts connection options;
String's PostgreSQL type compatibility includes NAME for the identity query. API
already imports ConnectOptions for log_statements. This is API plausibility review,
not a compiler or type-check result. No dependency installation or network lookup ran.

## Actual verification, follow-up and limits

Actual completed checks used in this report:

- Bounded source reads and four-file git diff against c073d7c; supporting migration
  harness call order, cleanup and local SQLx source reads. No I1a partial-code review.
- Four exact SHA-256/line-count comparisons matched initially and on resumption.
- Local source-text comparisons: all three role preflights match, new admitter grants
  are absent from supervisor bootstrap strings, static cluster SQL has no grants,
  and both new scratch privilege calls precede MIGRATOR.run.
- `rustfmt --edition 2024 --config skip_children=true --check` on only the two reviewed
  Rust files and scoped git diff --check passed. Report links/line bounds and whitespace
  are checked separately because the new report is untracked; final counts/hash are
  reported in the handoff. No SQL parser or database validation was performed.

Actionable source findings/required bounded fixes: **none**. Coordinator still owns
acceptance and integration. Remaining I0 production role/secret/Compose maps, official
validation wiring and I2/0060 are deferred packages, not blockers invented for this
bounded source delta. First0060 execution remains gated on the broader readiness
requirements in the plan. Later authorized isolated verification must establish clean
and dirty-role behavior, effective scratch ACLs, direct-login identities/GUC absence,
pool limits, migration-owner operation and cleanup. This review authorizes none of it.

Not found: concrete reachable new delta defects, missing required source integration
within these four files, or a required adjacent source change — **none**.
Unverified: compilation/type checking, SQL syntax/execution, actual roles/membership,
effective ACLs/privilege-denied behavior, real pool identities/connection limits/GUCs,
locks/concurrency/cleanup, first0060 behavior and all production/runtime/provider/Owner
behavior. Background activity from this reviewer: none. Permission waits: none.
Further decomposition of I0a-R: none. Deviations from the owned scope: none; the
interrupted report-writing step was completed on resumption without expanding review.
Only this new report was written. No product source, other document, D6/D4, Cargo/lock,
Git staging/commit/push, DB/root/Docker/network/provider/production/runtime/lifecycle,
package installation, test/build/compiler/SQL-parser execution or delegation occurred.
The current memory finish uses the supplied hook envelope and skips durable storage
of this unadopted review; progress is not stored as an adopted decision.
