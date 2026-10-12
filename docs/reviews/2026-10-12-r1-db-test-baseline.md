# R1 DB test baseline — WP-3b specification

Date: 2026-10-12 KST. Status: **SPECIFICATION_READY; implementation and execution pending**.
Repository: `/data/worktrees/3puw275b/orange-dugong`.
Source revision: `ac0a08364788e6c2f234cf85025a8d975f362d31`.
Roadmap: R1-01 diagnostic supplement, R1-03/R1-04 repairs, R1-08 final verification.

This assignment writes this report only. It does not authorize implementation,
Rust edits, heavy execution, production access, provider/account/live calls,
staging, commits, or delegation. Shared uncommitted work is preserved. WP-3
retains `deploy/db/integration-validation/*` and
`docs/runbooks/postgres-integration-validation.md`; neither is a repair target.
The coordinator owns roadmap/plan updates after accepting this specification.

## Evidence and classification

Read [AGENTS.md](../../AGENTS.md), [ROADMAP.md](../ROADMAP.md), and the
[R1 execution plan](../superpowers/plans/2026-10-12-r1-ci-recovery.md), including
the disposable stage38/full0059 separation and exclusive heavy-slot rule.

The safe diagnostic is
`target/r1-runtime-fxi6ltmv/evidence-wp3.A8ziPI/db-gated-partial-diagnostics.md`,
SHA-256 `91b49fa51196d229e39d4cb322488a89e1e9890dd0714ba7bcf00bd49c3afa55`.
It records an interrupted migration-contract invocation, exit **130**, three
failures, 14 `ok` reports in a partial excerpt, and no final suite count.
The SKIP scan did not run; subsequent DB suites did not start. Its teardown
and preceding validator successes are WP-3 evidence, not executions by WP-3b.
No fresh unmodified-baseline execution was performed here.

| Failure | Observed diagnostic | Source-backed classification |
| --- | --- | --- |
| `owner_equity_universe_v2_rls_lifecycle_and_lineage_contract` | line 10641: expected 59, actual 53 | Assertion drift: SQLx undo commits each migration separately; 0053 is the intended durable-state refusal after 0059..0054 revert. Preserve the refusal and prove its identity/data invariants. |
| `research_schema_gate_accepts_current_and_future_migration_ledgers` | line 5635: required research_writer policies missing/drifted | Real production gate compatibility defect: the tracked SQL rejects the permissions introduced by 0059. Three gate blocks need consistent correction, with exact policy/ACL negative coverage. |
| `migration_contract_full` | line 6940: actual includes status INSERT/SELECT/UPDATE absent from expected | Exact policy inventory assertion drift caused by 0059. Preserve equality and check the new policies' roles/predicates, rather than merely accepting three additional names. |

`git diff --name-only` and `--numstat` for `migrations/`,
`tests/integration/migration-contract/`, and the tracked gate returned no changes.
Thus these inputs are unchanged from HEAD by R1 at inspection time. Source
causes are present at HEAD, but that does not establish a fresh-run baseline
or a complete failure inventory. The partial diagnostic lacks the complete
catalog dump/error chain; the first gate rejection cannot prove which of its
two OR operands failed at runtime or that later blocks would pass.

## Closed future implementation allowlist

All paths below are absolute. They are a specification for a later authorized
worker, **not write authorization in this assignment**.

| File | Permitted scope |
| --- | --- |
| `/data/worktrees/3puw275b/orange-dugong/tests/integration/migration-contract/tests/migration_contract.rs` | Owner V2 rollback tail in `owner_equity_universe_v2_contract_body`; research_writer ACL/policy assertions in `full_contract_body`; retain/extend `research_schema_gate_accepts_current_and_future_migration_ledgers`; add focused status-policy/gate negative tests in this same file; narrowly scoped read-only ledger/policy snapshot helpers; strengthen the existing 0039 pending-row check if needed. |
| `/data/worktrees/3puw275b/orange-dugong/deploy/compose/research-schema-check.sql` | Research policy inventory/checks at 350–450, exact direct-grant set at 509–545, and forbidden-table allowlist/check at 547–574; an explicit consistent 0059-presence branch and status table owner/RLS/ACL/policy validation. Preserve all unrelated gates and the required migration set. |

No other source/test files are in this repair allowlist. In particular no
migration SQL/history/checksums, Cargo files, roles, bootstrap, Compose service
structure, provider/API surface, collector/status repository, release helper,
smoke script, or WP-3 files may change. If implementation needs another file,
return a concrete scope amendment to the coordinator before editing it.

## A. Owner V2 rollback: prove 0053, not ledger-wide atomicity

Affected call chain:
`owner_equity_universe_v2_rls_lifecycle_and_lineage_contract` (10275) →
`create_contract_db` (5908) → `owner_equity_universe_v2_contract_body` (10291) →
`MIGRATOR.run` → `MIGRATOR.undo(owner, 1)` (10639) →
`migrate_pg_code` (5691) and `applied_count` (5752).
`up_migration_count` (5894) counts all non-down entries, not the retained prefix.

The current fixture creates durable membership/event/generation/admission and
published signal rows and a scheduled incremental job. It does not populate
the status or stream/intraday tables. Relevant source contracts:

- [0053 down](../../migrations/0053_owner_managed_equity_universe_v2.down.sql)
  lines 4–38 locks all seven V2 tables, temporarily removes FORCE RLS so an
  unscoped owner sees every tenant's durable rows, and raises **55000** if any
  of the six lineage tables is nonempty. The policy table is deliberately
  excluded from that guard; its derived rows may be deleted only after the
  guard passes (40–43). Failed migration transaction rollback must restore FORCE.
- [0053 up](../../migrations/0053_owner_managed_equity_universe_v2.up.sql)
  279–349 preserves ownership, ENABLE/FORCE RLS and narrow column grants;
  351–503 supplies actor-local policies; 654–695 records membership events;
  699–940 enforces generation/admission/snapshot lineage and append-only data;
  992–1037 soft-disables membership; 1041–1249 schedules an exact idempotent job.
  These functions create V2 lineage, not an auth-audit outbox obligation.
- 0059 down locks the status table, refuses a nonempty table, otherwise drops
  it. Its ordinary RAISE has SQLSTATE **P0001**, not 55000. 0058/0057/0056 remove
  status-read/runtime functions/columns/indexes; 0055/0054 drop their separate
  disposable stream/intraday objects. These downs do not remove V2 lineage.
- Locked SQLx is 0.9.0. Read local primary implementation at
  `/home/l1nnx/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`:
  `sqlx-core-0.9.0/src/migrate/migrator.rs:314–358` walks applied down entries
  in descending order with `version > target`; each `conn.revert` returns on
  error. `sqlx-postgres-0.9.0/src/migrate.rs:267–285,341–358` executes a down
  and deletes its ledger row in **that migration's** transaction, then commits.
  There is no transaction wrapping the whole undo walk.

Therefore successful 59..54 removals remain committed when 53 fails. Actual
53 is consistent with this fixture and 55000 had already passed the preceding
assertion. The diagnostic did not expose `ExecuteMigration(..., version)`;
**prove version 53 in the repaired test**, rather than using the count as proof.

Required repair sequence:

1. Retain all existing lifecycle, owner isolation, draft hiding, SQLSTATE,
   immutable admission, soft-disable/re-add and scheduling assertions.
2. Capture ordered ledger `(version, success, checksum)` and complete synthetic
   V2 row snapshots for both owners under the existing actor pools. Include
   policy rows, memberships/events, generations/admissions, signal headers/rows,
   and the scheduled job's id/payload. Keep snapshots internal, never dump them.
   Snapshot auth outbox obligations as migration_owner too. Verify newer
   disposable tables are empty before undo, especially status.
3. Explicitly `MIGRATOR.undo(owner, 53)` successfully to separate the 59..54
   stage from the guarded stage. Assert the **exact ordered retained ledger**
   equals the original entries with version <=53, including unchanged successful
   flags/checksums. Verify the removed-family objects are gone and all retained
   V2 data/jobs/outbox obligations equal their snapshots. Do not just replace
   the current count with literal 53.
4. Acquire one owner connection, then `MIGRATOR.undo(&mut *guarded, 52)`.
   Assert the error is `MigrateError::ExecuteMigration(_, 53)` and its nested
   database code is exactly **55000**. Release the migrator's advisory lock on
   this same acquired connection before returning it to the pool, following
   the existing `pg_advisory_unlock_all()` pattern at 2012–2017 and 5312–5318.
   SQLx's early error return precedes its normal unlock; using a pooled undo
   without this cleanup can strand the session lock during later validation.
5. After refusal, compare the complete retained ledger and data snapshots
   again. Check all seven V2 relations still belong to migration_owner and have
   `relrowsecurity=true`, `relforcerowsecurity=true`; an unscoped owner must
   still read zero tenant rows. Actor A must retain its own rows and exclude B.
   No delivery flag, job, admission/hash, published row, or membership state
   may change to manufacture a safe rollback. Optionally reapply 54..59 and
   prove full ledger restoration plus unchanged V2 data; this is not necessary
   for the smallest guard repro.

The explicit stage is a supported disposable-test repair, not a production
rollback recipe. Do not make all migrations atomic, disable migration locking,
catch arbitrary migration errors, bypass RLS, or edit 0053 to satisfy the count.

Separate earlier guard contracts remain mandatory:

- `auth_audit_rollback_guard_is_fail_closed_and_allows_delivered_rows` (4048)
  runs only to39, inserts one pending event, and undo-to38 must fail at39 with
  **55000**. Preserve the same event id with `delivered_at IS NULL`, outbox
  existence and exact successful39 ledger/checksum after refusal. Only the
  existing explicit fixture delivery transition allows subsequent undo.
  [0039 down](../../migrations/0039_auth_audit_outbox.down.sql) 4–16 guards before
  any destructive drop. Acquire/unlock the same connection if extending this
  test; do not delete the undelivered fixture to avoid the guard.
- `paper_settlement_outbox_db_contract` (3545; rollback tail 3939–3954)
  preserves the 0041 pending obligation and exact **55000** refusal; its later
  delivered fixture permits undo. `identity_provisioning_body` (4352; tail
  5312–5342) likewise refuses pending audit obligations before explicit cleanup.
  These are adjacent verification contracts, not a reason to weaken their tests.
- WP-3's existing 0039 guard/postflight verifies pending-row and ledger39
  preservation. It is read-only supporting evidence here and excluded from edits.

## B. Research schema gate: production correction required

The constant at migration-contract lines 111–112 resolves exactly to
`/data/worktrees/3puw275b/orange-dugong/deploy/compose/research-schema-check.sql`.
It is the production tracked SQL, not a validator-only fixture.

The ledger test executes it four times (5597,5609,5615,5627): after all actual
migrations, after adding one successful future ledger entry, after marking
required33 unsuccessful (must reject), and after restoring33 (must pass).
The failure happens before later intended ledger checks can be established.

Three source defects are coupled:

1. At350 the global writer-policy count is exactly24; current0059 adds three
   explicit writer policies. The existing eight-row matcher at354–371 requires
   command, permissive mode, single writer role and exact true/null predicates.
   **Do not insert singleton policies into that true-only matcher.**
2. At509–544 the bidirectional `EXCEPT` grant equality omits status
   SELECT/INSERT/UPDATE; valid0059 grants therefore conflict even after (1).
3. At553–572 the table allowlist omits status; its intended writer privilege
   is consequently treated as forbidden. An allowlist addition alone would
   leave (1)/(2) broken and must not grant blanket status DML.

Repair the gate without any schema/role/migration mutation. Preserve the
current successful required set **22,23,24,25,33,34,35,42,45,46,47**, its missing33
negative, and successful future-ledger tolerance. This assignment does not
authorize raising the minimum to59. Support two explicit shapes:

- Without a59 ledger entry: retain the base24 writer policies/grants and require
  no `public.research_execution_status` relation/policies/grants.
- With a successful59 entry: require the status relation owned by migration_owner,
  ENABLE+FORCE RLS, exact status policies and ACLs below, and the base24 plus
  exactly three writer policies/grants. A failed59 entry, successful59 with a
  missing relation, or a status relation without successful59 must reject.
  Do not infer an allowance from table existence or `version >=59` alone.
  A successful unknown future ledger row does not authorize new privileges.

Use exact set equality (both `actual EXCEPT expected` and its inverse) for
writer policy shape: schema, table, name, command, permissive flag, complete
role-name set, USING and WITH CHECK. All writer-bearing policies in all schemas
must be accounted for; extra policies in another schema must not escape the
old global deny boundary. Normalize role-array order only, not membership.
Use explicit expected values sourced below; do not derive expected from actual.
Retain existing specific policy/function/index/constraint checks. Never weaken
an equality to >=, a subset, arbitrary name patterns, or a UNION with actual.

### Migration-backed exact writer policy inventory

`NULL` below means no explicit expression; it must not be treated as `true`.
All these policies are permissive. All named tables are in public.

| Migration / source lines | Table(s) and exact policy names | Command / complete roles / USING / WITH CHECK |
| --- | --- | --- |
| 0022 up:76–95 | data_batches: `data_batches_select_research_writer`, `data_batches_insert_research_writer` | SELECT / {research_writer} / true / NULL; INSERT / {research_writer} / NULL / true |
| 0022 up:81–86 | trading_calendars: `trading_calendars_select_research_writer`, `trading_calendars_insert_research_writer`, `trading_calendars_update_research_writer` | SELECT / {research_writer} / true / NULL; INSERT / {research_writer} / NULL / true; UPDATE / {research_writer} / true / true |
| 0022 up:92–95 | trading_calendar_versions: `trading_calendar_versions_select_research_writer`, `trading_calendar_versions_insert_research_writer` | SELECT / {research_writer} / true / NULL; INSERT / {research_writer} / NULL / true |
| 0042 up:10–14 | dataset_versions: `candidate_dataset_versions_select_research_writer` | SELECT / {research_writer} / true / NULL |
| 0042 up:2078–2093 | candidate_instrument_registrations: `candidate_instrument_registrations_select` | SELECT / {worker, admin, research_writer} / true / NULL |
| 0042 up:1952–1983 | candidate_raw_batch_publications, candidate_raw_batch_datasets: `candidate_source_select_` + exact table name | SELECT / {worker, admin, research_writer} / true / NULL |
| 0042 up:1985–2029 | candidate_universe_snapshots, candidate_universe_members, candidate_investor_flows, candidate_investor_flow_snapshot_rows, candidate_market_status_observations, candidate_fundamental_observations, candidate_sector_versions, candidate_sector_entries: `candidate_source_select_` + exact table name | SELECT / {worker, admin, research_writer} / true / NULL |
| 0042 up:2034–2073 | candidate_price_publications, candidate_price_instrument_coverage, candidate_price_instrument_sessions: `candidate_source_select_` + exact table name | SELECT / {worker, admin, research_writer} / true / NULL |
| 0045 up:47–48 | candidate_universe_registry: `candidate_universe_registry_select_research_writer` | SELECT / {research_writer} / true / NULL |
| 0046 up:783–786 | candidate_price_revalidation_events: `candidate_price_revalidation_events_select` | SELECT / {research_writer} / true / NULL |
| 0059 up:65–74 | research_execution_status: `research_execution_status_research_writer_select`, `research_execution_status_research_writer_insert`, `research_execution_status_research_writer_update` | SELECT / {research_writer} / singleton / NULL; INSERT / {research_writer} / NULL / singleton; UPDATE / {research_writer} / singleton / singleton |

Base count24 is derived as 7+1+1+2+8+3+1+1; full0059 count27 is base+3.
`singleton` is exactly SQL `id = 'kis-eod'`, never `(true)`, a prefix check or
another id. PostgreSQL18 canonical `pg_get_expr` spelling must be collected in
the reserved repro before finalizing expression literals; no catalog deparse
was executed here. Compare the exact resulting expression, retaining casts,
operators and explicit NULLs; do not use substring containment for predicates.

0059 also creates exactly `research_execution_status_admin_select` (SELECT,
{admin}, singleton, NULL) and `research_execution_status_migration_owner_all`
(ALL, {migration_owner}, singleton, singleton), up62–64,75–78. Check the full
five-policy status set, not just its writer members, so an added PUBLIC policy
or widened existing role set cannot bypass the status check.

### Grants and preserved gate contracts

Keep base direct grants exactly as gate516–538: data_batches and calendar
versions SELECT/INSERT, calendar projection SELECT/INSERT/UPDATE, and SELECT
only on the listed dataset/candidate evidence tables. Add only status
SELECT/INSERT/UPDATE under successful59; retain symmetric grant equality.
Status effective writer DELETE/TRUNCATE/REFERENCES/TRIGGER/MAINTAIN must all
be false, including privileges inherited from PUBLIC. Check admin SELECT only;
app/worker/audit_writer must have no status privileges, PUBLIC no grants, no
grant option. Require migration_owner ownership and both RLS flags. Maintain
the status singleton CHECK from0059, not a CHECK(true) substitute.

Retain writer LOGIN with no SUPERUSER/BYPASSRLS/CREATEDB/CREATEROLE/REPLICATION,
no inbound/outbound role memberships, public USAGE without CREATE, and no
identity-sequence privileges. Preserve all source ownership, validated CHECK/
column/index equality, candidate publisher SECURITY DEFINER/search_path and
EXECUTE restrictions, recommendation locks, append-only trigger/function-body
checks, dataset_versions DML denial, data_entitlements SELECT denial, and
tenant/order/audit/job denial. No role grant, bootstrap or network change.

`docs/contracts/service-status-v1.md:583–605` explicitly defines these0059
permissions and nonempty-down refusal. `data-pipelines/collectors/src/service_status.rs`
uses INSERT ON CONFLICT and fenced UPDATE/RETURNING (74–136,164–210,274 onward);
those need the existing status SELECT/INSERT/UPDATE grants. Keep the >90s stale
takeover, attempt fence, best-effort recording and exact Curated-publication
binding unchanged. No status observer gains a write/restart path.

## C. Full migration test: exact inventory, with behavior coverage

Affected chain: `migration_contract_full` (5999) → `full_contract_body` (6016) →
full MIGRATOR run/rerun → effective `research_writer` pool (6771) → exact ACL
checks (6772–6807) → ordered public policy query (6808–6814) →
`expected_writer_policies` (6815–6938) → equality (6939–6943).

Add precisely the three0059 writer tuples in the correct table/policy sort
position, between dataset_versions and trading_calendar_versions. Preserve
the exact equality and every previous expected tuple. Expand policy projection
to assert complete roles, permissive mode, qual/check expressions (or add an
adjacent exact assertion for that shape); a name/command triple alone does not
verify the singleton restriction. Cross-schema extras must still be rejected
by the production gate; keep the full test's public inventory equality too.

Extend the exact eight-privilege writer ACL tuple to status:
`(SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN)` =
`(true,true,true,false,false,false,false,false)`. Exercise a valid singleton
INSERT, SELECT and fenced-style UPDATE with current DB timestamps; no initial
status row may be fabricated by migration. Admin may SELECT only. Denied roles
must fail actual status SELECT/DML; writer DELETE/TRUNCATE must return42501.
Writer non-singleton INSERT and changing the singleton id must fail and leave
the original row unchanged; do not invent an exact precedence-dependent code
for CHECK versus RLS without observing it. Preserve clock/state/finish/failure
constraints; use a coherent RUNNING fixture rather than relaxing constraints.

Keep all existing writer positives and negatives at6956–7033, including Raw
publication, calendar history/projection, sequence denial, immutable data,
orders/jobs/audit reads and schema CREATE denial. Keep the rest of full_contract
unchanged. Synthetic DB order-table tests do not authorize any account/provider
network access.

## All affected readers/callers and verification boundaries

Direct tracked-gate readers/invocations found by bounded `rg` searches:

- Migration-contract constant111, static token check5537, and the four ledger
  test invocations above; `tests/integration/migration-contract/build.rs:11`
  forces rebuild when gate SQL changes. No build-script edit is required.
- `deploy/compose/compose.yml:511–550` mounts the tracked SQL read-only and
  executes `psql -X -v ON_ERROR_STOP=1`; db-migrate must complete first.
  Research-worker waits for successful gate completion at641–642.
- `scripts/ops/compose-release.sh:676,690,708`: infrastructure, backfill and
  serving release paths all invoke this gate. These were read only; **no
  production deployment was examined or run**. No release-helper edit needed.
- Linux smoke: `scripts/qa/research-worker-smoke.sh:11,66,111–114,345–361,680–681,723–777`:
  tracked/copied/static SQL checks, unmigrated rejection, migrated acceptance
  and mutation/recovery probes. Preserve CHECK, storage_path, index, policy,
  trigger/function, forbidden order grant and source-owner negatives.
- PowerShell smoke: `scripts/qa/research-worker-smoke.ps1:18,54,121,264–293,419–468`
  contains corresponding readers/helpers and mutation tests. Windows execution
  was not performed; existing parity gaps are not repaired in this scope.
- `scripts/ci/test_ci_contract.py:335` inspects the Linux gate pass helper;
  it does not provide DB proof. Existing CI/smoke changes belong to another WP.

No other direct reference to the gate SQL or constant was found in the bounded
repository search. This is not a whole-repository security audit or proof that
generic copying/release packaging never includes it indirectly.

Adjacent existing safety verification (read-only, no edits):
`collectors/tests/service_status.rs:374–445` checks empty initial state, real
role permissions and ENABLE/FORCE; the test
`status_repository_is_fenced_best_effort_and_publication_bound` (863) covers
fenced recording; `actual_attempt_takes_over_only_a_strictly_stale_running_fence`
(1172) covers takeover; `down_locks_before_testing_a_concurrent_first_insert`
(1293) observes a real AccessExclusiveLock wait, then exact **P0001** nonempty
refusal and surviving row before empty-down success (1181–1299). Preserve
these tests and0059 history. They were not run here.

## Smallest meaningful reserved validation — commands NOT run

Preconditions: WP-3 releases the exclusive heavy slot; coordinator authorizes
the closed repair; use a new owned disposable PostgreSQL18 cluster with a
nonempty supervisor DATABASE_URL supplied privately. No production URL, live
profile, entitlement/provider data or credentials. Existing helpers create
unique scratch DBs; reject `SKIP:` even if Cargo returns zero. Record the exact
source revision, final count, every exit and owned teardown. No concurrent
Cargo/DB/build execution.

First capture a fresh unmodified source baseline with the three exact filters
below, before implementing changes, if a fresh baseline claim is needed.
Capture only typed codes, failing migration version and safe catalog shape;
do not retain URL/password/raw provider output. This separates unchanged
source behavior from shared-tree or interrupted-run attribution.

After repairs, run sequentially:

```sh
cargo test --locked -p migration-contract --test migration_contract owner_equity_universe_v2_rls_lifecycle_and_lineage_contract -- --exact --nocapture
cargo test --locked -p migration-contract --test migration_contract research_schema_gate_accepts_current_and_future_migration_ledgers -- --exact --nocapture
cargo test --locked -p migration-contract --test migration_contract migration_contract_full -- --exact --nocapture
```

Add one narrowly focused DB test in the allowed test file named
`research_schema_gate_rejects_status_policy_and_acl_drift`. Run it separately:

```sh
cargo test --locked -p migration-contract --test migration_contract research_schema_gate_rejects_status_policy_and_acl_drift -- --exact --nocapture
```

Its minimum matrix is migrated47/base24 pass and full59/exact27 pass; restored
valid shape passes after **each independent** rejected mutation. Use transaction
rollback or separate scratch DBs for mutations; an exception aborts its current
transaction, so do not query through that aborted transaction. No cluster-global
role mutations are needed. Exercise:

| Mutation / boundary | Required result |
| --- | --- |
| Current59, successful unknown future ledger row, required33 false/restore | pass, pass, reject, pass (retain original ledger test) |
| Successful59 missing status table; failed59; status table without successful59 | reject each; restore shape passes |
| Drop each of the three writer status policies, then restore exactly | reject each; restore passes |
| Status SELECT or UPDATE USING=true; INSERT or UPDATE WITH CHECK=true | reject each despite remaining singleton CHECK; restore passes |
| Correct name but wrong command, restrictive mode, widened roles, wrong singleton id | reject independently; restore passes |
| Extra writer policy, extra PUBLIC status policy, or writer policy in another schema | reject, including equal-count substitution; restore passes |
| Missing writer INSERT/UPDATE grant; extra DELETE/TRUNCATE; PUBLIC-granted privilege | reject each; restore passes; no forbidden effective privilege |
| Status DISABLE RLS, NO FORCE RLS, drifted owner | reject each; restore passes |
| Missing old publication policy, weakened old CHECK, forbidden orders grant | reject each; restore passes; old gate purpose remains effective |

For PUBLIC privilege probes ensure a complete ACL snapshot restores it; no role
definition/membership changes. Expected SQLSTATE for production gate exceptions
is ordinary RAISE **P0001**; assert the intended safe diagnostic category too
so unrelated failed SQL cannot satisfy the negative test. Verify 0059 singleton
predicate deparse directly here before finalizing exact expected expression.

Small rollback/safety follow-through, also sequential and NOT run:

```sh
cargo test --locked -p migration-contract --test migration_contract auth_audit_rollback_guard_is_fail_closed_and_allows_delivered_rows -- --exact --nocapture
cargo test --locked -p migration-contract --test migration_contract owner_equity_universe_v2_schema_is_separate_actor_scoped_and_fail_closed -- --exact --nocapture
cargo test --locked -p collectors --test service_status status_repository_is_fenced_best_effort_and_publication_bound -- --exact --nocapture
cargo test --locked -p collectors --test service_status actual_attempt_takes_over_only_a_strictly_stale_running_fence -- --exact --nocapture
cargo test --locked -p collectors --test service_status down_locks_before_testing_a_concurrent_first_insert -- --exact --nocapture
```

Keep paper-settlement/identity rollback coverage in the subsequent complete
migration-contract run. Focused success cannot stand in for WP-3's interrupted
suite. The coordinator must reserve a full DB-gated validator rerun and Linux
research smoke after integration; unchanged required suites/negative probes
must actually reach execution. No source edit or new command in WP-3's owned
validator is requested here. Final CI uses the existing feature-enabled full
workspace command, not a reduced target list. Windows parity and actual
production deployment remain separate, unverified work.

## Work performed, deviations, unresolved items

Actual operations: `pwd`, `git status --short`, `git rev-parse HEAD`, narrowly
scoped `git diff --name-only/--numstat`, `cat`, `sed`, `nl`, `wc`, `rg/rg --files`,
and `sha256sum`; local SQLx0.9.0 source inspection. These were read-only source/
evidence checks, not tests. Report creation is the sole repository write.
No Cargo/rustc/test/Docker/npm/DB/build command, network/provider/production
operation, delegation, staging or commit was performed.

Deviations: none from the WP-3b ownership/execution boundary. Mandatory roadmap
updates are left to the coordinator because this task authorizes only this
new report. An initial guessed0023 filename and guessed collector status-module
filename did not exist; bounded file discovery resolved them to
`0023_research_publication_validate_constraints.up.sql` and
`data-pipelines/collectors/src/service_status.rs`, both subsequently read.

Unresolved/follow-up: authorization/assignment of the two-file implementation;
reserved fresh baseline if required; actual PostgreSQL18 policy-expression
deparse and gate negative repro; repaired focused results; complete DB suites,
SKIP audit and smoke; independent review and coordinator roadmap update.

Not found/not verified: complete raw error chains/catalog predicates for the
three failures; recorded failing migration version (source predicts53);
first gate branch's actual27 count versus its eight-policy match count;
runtime results for the later exact-grant/forbidden-table blocks; final suite
count/SKIP status and suites after interruption; any fresh baseline execution;
Windows/production/Owner acceptance. No missing source contract remains in
this bounded specification. Further diagnostic decomposition is unnecessary;
implementation and reserved execution are the remaining separate work.
