# R1 DB contract repair — WP-3c source/static report

Date: 2026-10-12 KST. Source revision: `ac0a08364788e6c2f234cf85025a8d975f362d31`.
Status: **READY_FOR_HEAVY_VALIDATION**. This report records source/static work only;
the reserved PostgreSQL 18 verification and DB suites remain pending.

## Changed files

- `tests/integration/migration-contract/tests/migration_contract.rs`
  - 4067–4119: the 0039 rollback refusal now identifies migration 39 and SQLSTATE
    55000, preserves the exact pending event/outbox row and successful ledger
    checksums, and unlocks the same acquired connection before reuse.
  - 5678–5904: the gate ledger test now passes at the 47/base-policy shape and
    again after reapplying through 59. The new
    `research_schema_gate_rejects_status_policy_and_acl_drift` test checks
    independent mutations, exact P0001/category rejection, rollback, and a
    valid gate pass after each restoration.
  - 6024–6181: private migration-ledger, full-row JSON snapshot, and disposable
    family helpers. Snapshot values are compared without formatting or logging.
  - 7213–7645: `migration_contract_full` checks the exact 27 writer-policy
    shapes and status ACL tuple, then exercises a coherent singleton RUNNING
    insert, read, fenced update, denied-role reads/writes, writer DELETE/
    TRUNCATE denials, and rejected singleton changes.
  - 11341–11475: the V2 rollback test snapshots both actor views, the scheduled
    job, and auth outbox; verifies disposable tables are empty; successfully
    undoes through 53 and checks the exact retained ledger/data; then refuses
    undo through 52 specifically at migration 53 with SQLSTATE 55000. It
    releases the lock on that same connection and rechecks snapshots, all seven
    relation owners/RLS flags, and actor isolation.
- `deploy/compose/research-schema-check.sql`
  - 8–10, 353–546: explicit successful-59/relation state, exact global
    24-or-27 writer policy inventory, full five-policy status inventory,
    owner/RLS/singleton-check validation, and exact status ACL checks.
  - 687–719: symmetric direct-grant equality conditionally includes only the
    three 0059 writer grants.
  - 749: the forbidden-table allowlist admits status only in the successful-59
    branch. Existing required migration, role, policy, function, constraint,
    index, source, tenant, order, audit, and job checks remain in place.
- `docs/reviews/2026-10-12-r1-db-contract-repair.md`: this report.

The explicit singleton deparse literals (`id = 'kis-eod'::text` and the
corresponding CHECK definition) are drafts from the migration/type contract.
They are marked unverified in source and must be compared byte-for-byte with
the PostgreSQL 18 catalog deparse during the reserved DB run.

## Specification deviations

None. No migration/history/checksum, role/bootstrap/grant DDL, Compose topology,
provider/API boundary, other test, roadmap/plan, diagram, staging, or commit was
changed. Existing shared modifications were preserved.

## Static checks run

- The initially attempted spelling `rustfmt --edition2024 ...` exited 1 because
  the installed formatter requires a space before the edition value.
- `rustfmt --edition 2024 --config skip_children=true tests/integration/migration-contract/tests/migration_contract.rs` — exit 0.
- `rustfmt --edition 2024 --config skip_children=true --check tests/integration/migration-contract/tests/migration_contract.rs` — exit 0.
- `git diff --check` — exit 0.

No Cargo, rustc, Clippy, test, Docker, npm, database, build, network/provider,
or production command was run.

## Pending validation and unresolved items

The following require the separately reserved disposable PostgreSQL 18 DB slot:

- Verify the exact `pg_get_expr` policy predicate and `pg_get_constraintdef`
  singleton CHECK spelling, then adjust only those explicit literals if the
  observed deparse differs.
- Run the focused owner V2 rollback, current/future gate ledger, full migration
  contract, and new gate drift matrix tests. The gate tests must demonstrate
  47/base24 and 59/exact27 passes, future-ledger tolerance, required-33
  reject/restore, and a valid pass after every isolated negative mutation.
- Run the specified 0039 rollback guard and V2 schema isolation tests, plus the
  three service-status fenced-write, stale-takeover, and concurrent-down tests.
- Run the complete DB-gated validator, confirm required tests reached execution
  with no unexpected `SKIP`, and run the reserved Linux research-worker smoke
  after integration. Focused success alone does not replace this full rerun.

No missing or contradictory source contract was found. Empirical PostgreSQL 18
deparse, all DB positive/negative/rollback results, the full validator result,
and Linux smoke result are unverified here. Windows and production results remain
outside this assignment.

## Structural diagram impact

None. These edits change a validation test and the existing SQL gate contract,
not static dependencies, Compose runtime/deployment, or service boundaries; the
architecture diagrams remain untouched.

## WP-5 review correction and reserved verification addendum

Date: 2026-10-12 KST. This addendum preserves the preceding 89-line report as
historical evidence and supersedes its “deviations: none”, draft-deparse, and
DB-pending statements where this addendum records later findings. The accepted
432-line baseline and reviewer DB1–DB7 section were reread before correction.

### DB1–DB7 corrections

- **DB1 — SQLx 0.9 safety/lifetime:** the gate mutation helper now accepts
  `&'static str`, matching its literal-only call sites (Rust 5714–5764). Every
  new formatted SQL statement over a closed table-name array is passed through
  `sqlx::AssertSqlSafe(query)` (Rust 6235–6242, 6276–6326, 11733–11742).
  Clippy’s mechanically related constant-assertion findings at the 0039 and
  0053 rollback match arms now use `panic!` while preserving both assertions
  (Rust 4085, 11672).
- **DB2 — forbidden-table scan:** the status exception now uses
  `OR NOT status_migration_successful` (SQL 752). Full59 passes and the
  existing forbidden `orders` PUBLIC privilege mutation still rejects in the
  intended category.
- **DB3 — absent relation safety:** the exact ledger/relation consistency
  check remains before status-only work (SQL 355–367). Owner/RLS/check/policy,
  direct-grant, and effective-ACL checks are separate nested statements under
  `IF status_migration_successful THEN` (SQL 441–550). The required base47
  relation-absent pass and full59 pass both executed successfully; missing,
  failed, and relation-without-success mutations rejected with P0001 rather
  than an absent-relation cast error.
- **DB4 — empirical PostgreSQL 18.4 deparse:** pristine
  `0059_research_execution_status.up.sql` was applied to a fresh probe database
  on the pinned PostgreSQL 18.4 image. The five exact policy rows deparsed as:

  ```text
  research_execution_status_admin_select|(id = 'kis-eod'::text)|NULL
  research_execution_status_migration_owner_all|(id = 'kis-eod'::text)|(id = 'kis-eod'::text)
  research_execution_status_research_writer_insert|NULL|(id = 'kis-eod'::text)
  research_execution_status_research_writer_select|(id = 'kis-eod'::text)|NULL
  research_execution_status_research_writer_update|(id = 'kis-eod'::text)|(id = 'kis-eod'::text)
  ```

  The singleton constraint deparsed as exactly
  `CHECK (id = 'kis-eod'::text)`. SQL expected literals at 424–426 and 485–489
  and Rust writer tuples at 7539–7557 now use those observed values, including
  outer policy-expression parentheses and NULL positions. Evidence is in
  `target/r1-runtime-fxi6ltmv/wp3c-focused-20261012-a7e2d6/postgres18-deparse.log`.
- **DB5 — complete role projection:** the production gate and full-contract
  query both use raw `pg_policy.polroles`, `LEFT JOIN pg_roles`, and explicit
  OID-0 `PUBLIC` mapping; both retain exact set/tuple equality (SQL 372–395,
  462–496; Rust 7454–7480). The reviewer’s exact count-preserving
  `research_writer + PUBLIC` catalog state could not be produced with
  PostgreSQL 18.4 `ALTER POLICY ... TO research_writer, PUBLIC`: the observed
  raw-role projection is `["PUBLIC"]`, the total `pg_policy` row count remains
  309, and research-writer-bearing rows change 27→26. The test records this
  observed behavior, compares the complete actual status SELECT tuple against
  the explicit research-writer-only tuple, requires mismatch, then verifies
  P0001/category rejection and restored pass (Rust 5944–6030). It therefore
  verifies the reachable PUBLIC widening and raw OID-0 shape handling, but it
  does **not** establish the requested mixed-OID, unchanged-writer-count case.
  No system-catalog mutation was used to manufacture that state. The gate and
  full-contract equality would reject a mixed role array if present; a
  coordinator decision is needed on whether the empirical DDL case is an
  acceptable substitute or a separate catalog-level test is authorized.
- **DB6 — equal-count replacements:** the independent same-name, same-command,
  same-role, same-predicate wrong-table and `pg_temp` replacements retain the
  global writer count at 27, reach the intended policy-inventory P0001, roll
  back independently, and pass the gate afterward (Rust 5931–5942). Existing
  status-policy, grant, RLS/owner, base-policy, CHECK, and forbidden privilege
  mutations remain in the drift test. Its base47 and full59 positive passes
  also executed.
- **DB7 — physical emptiness:** 0054 intraday tables remain counted through
  the worker pool. Each 0055 stream table is counted through an acquired
  migration-owner connection (all-true owner policies), before 0053 undo
  (Rust 6276–6326). A synthetic REVOKED grant is inserted in a transaction,
  hidden from worker under transaction-local `SET ROLE`, rejected by the same
  physical-empty helper after restoring migration_owner, then removed by
  transaction rollback. The first attempt used DELETE and observed the
  intended immutable-grant 55000 guard; that cleanup was replaced with
  rollback. The owner V2 test now passes through exact undo53 and the
  `ExecuteMigration(_, 53)`/55000 refusal at undo52, with ledger/checksum/data/
  job/outbox snapshots, same-connection unlock, RLS/owner, and actor checks
  intact (Rust 11591–11742).

### Static and focused database results

Static commands and final outcomes:

- `rustfmt --edition 2024 --config skip_children=true tests/integration/migration-contract/tests/migration_contract.rs` — exit 0.
- `rustfmt --edition 2024 --config skip_children=true --check tests/integration/migration-contract/tests/migration_contract.rs` — exit 0.
- `git diff --check` — exit 0.
- `CARGO_BUILD_JOBS=2 COMPOSE_PARALLEL_LIMIT=1 CARGO_HOME=/data/worktrees/3puw275b/orange-dugong/target/r1-runtime-fxi6ltmv/cargo-home CARGO_TARGET_DIR=/data/worktrees/3puw275b/orange-dugong/target/r1-runtime-fxi6ltmv/cargo-target CARGO_NET_OFFLINE=true cargo check --locked --offline -p migration-contract --test migration_contract` — exit 0.
- With the same environment, `cargo clippy --locked --offline -p migration-contract --test migration_contract -- -D warnings` — final exit 0. Earlier runs reported two `assertions_on_constants` diagnostics and one `explicit_auto_deref`; the rollback arms were changed to equivalent `panic!` arms and the transaction argument to the clippy-suggested auto-deref form, without weakening assertions.
- `bash -n` on both private QA scripts — exit 0. The focused runner script, which contained the required synthetic connection string only in process setup, was removed after execution; safe summaries remain under the private evidence directory.

The successful final focused run used the pinned image digest from
`deploy/qa/qa-db.compose.yml`, project `lagrange-r1-wp3c-a7e2d6-tests`, and
checked loopback port 55435. Cargo used `--locked --offline`, the required
private `CARGO_HOME`/`CARGO_TARGET_DIR`, two build jobs, and the synthetic
postgres/lagrange TCP connection. Nine exact tests ran serially; each reported
one passed, zero failed, no SKIP, and no zero-match result:

```text
cargo test --locked --offline -p migration-contract --test migration_contract owner_equity_universe_v2_rls_lifecycle_and_lineage_contract -- --exact --nocapture
cargo test --locked --offline -p migration-contract --test migration_contract research_schema_gate_accepts_current_and_future_migration_ledgers -- --exact --nocapture
cargo test --locked --offline -p migration-contract --test migration_contract research_schema_gate_rejects_status_policy_and_acl_drift -- --exact --nocapture
cargo test --locked --offline -p migration-contract --test migration_contract migration_contract_full -- --exact --nocapture
cargo test --locked --offline -p migration-contract --test migration_contract auth_audit_rollback_guard_is_fail_closed_and_allows_delivered_rows -- --exact --nocapture
cargo test --locked --offline -p migration-contract --test migration_contract owner_equity_universe_v2_schema_is_separate_actor_scoped_and_fail_closed -- --exact --nocapture
cargo test --locked --offline -p collectors --test service_status status_repository_is_fenced_best_effort_and_publication_bound -- --exact --nocapture
cargo test --locked --offline -p collectors --test service_status actual_attempt_takes_over_only_a_strictly_stale_running_fence -- --exact --nocapture
cargo test --locked --offline -p collectors --test service_status down_locks_before_testing_a_concurrent_first_insert -- --exact --nocapture
```

`migration_contract_full` observed non-singleton status INSERT and singleton
id-change rejection as SQLSTATE **42501** with no reported constraint name for
either. These are observed outcomes; the test does not assert a guessed
CHECK-versus-RLS precedence.

The first pristine-deparse probe exited 3 because the disposable probe lacked
the existing bootstrap roles; its trap removed the owned container/network and
verified port release. Rerunning with the repository role bootstrap exited 0
and captured the deparse above. All test-project runs, including earlier
diagnostic attempts, verified owned-container/network removal and port release.
No QA container, network, or process remains; the exclusive DB slot is released.

### Deviations and remaining verification

The DB5 mixed `research_writer,PUBLIC` role-array case with unchanged
research-writer-bearing policy count remains unresolved for the PostgreSQL
18.4 DDL reason above. This addendum does not claim final WP-5, full-validator,
workspace, CI, Linux smoke, Windows, production, or Owner acceptance. The
separately reserved full DB-gated validator and Linux smoke remain pending.
No migration, history/checksum, role/bootstrap/grant DDL, other test/source,
workflow, validator, diagram, roadmap, staging, or commit was changed.

Structural diagram impact: none. The production gate’s checks changed, but no
role, table group, service, dependency, network, or mount structure changed.

Final source SHA-256 values and the accepted-contract hash are recorded in the
coordinator’s safe focused evidence inventory; the baseline contract remained
unchanged at `d295e892eca43c6176f49ea3066fb7c94279e00257d5037660140e17f1c1dce5`.

## Append-only report correction

Correction to the preceding paragraph: I had not verified that these hashes
were present in a coordinator inventory. The hashes below are the verified
values from this worktree, captured at HEAD
`ac0a08364788e6c2f234cf85025a8d975f362d31` after source edits. This correction
does not change or remove the historical evidence above.

Changed-file spans:

- `tests/integration/migration-contract/tests/migration_contract.rs`: rollback
  guard and typed diagnostics 4067–4117, gate positive/drift cases 5678–6039,
  typed error helpers 6097–6153, actor snapshots and disposable-family checks
  6221–6394, full policy inventory and status behavior 7426–7810, and owner V2
  rollback lifecycle 11223–11227 and 11578–11762.
- `deploy/compose/research-schema-check.sql`: migration/relation consistency,
  exact policy inventory, and guarded status contract 353–549; required
  relation exception in the forbidden-table scan at 752.
- `docs/reviews/2026-10-12-r1-db-contract-repair.md`: this correction is
  appended at lines 231 onward; the original 89-line report and the previous
  verification addendum remain intact.

Verified SHA-256 values:

```text
c65b4345d9a8747c8cdb545e2cff7986623be23b79170987438bc4722d17eb30  tests/integration/migration-contract/tests/migration_contract.rs
e84cfc17ed44af25e51d082252675dcbb279e8019d582303e190838f10d9e7b1  deploy/compose/research-schema-check.sql
d295e892eca43c6176f49ea3066fb7c94279e00257d5037660140e17f1c1dce5  docs/reviews/2026-10-12-r1-db-test-baseline.md (accepted baseline, unchanged)
31c38416ad828c1b463dcf3698c92a4b80b437680223c54c723689c9f2be4e51  docs/reviews/2026-10-12-r1-review.md
f2a7df36c2983823eb8834af9d10a7c91b75f44602134943e313de804d3fbb0c  migrations/0059_research_execution_status.up.sql (read-only source)
ec0bdd0cc8096d2b80318e756cba8c6b8b8296adde10ee9c5bf80839aebc2d9d  deploy/qa/qa-db.compose.yml (read-only pinned QA definition)
11f3b3f42f9447fe6e34593add3e2940128b98172aae10fc9015222abc076f7e  target/r1-runtime-fxi6ltmv/wp3c-focused-20261012-a7e2d6/postgres18-deparse.log
e0b27b22398b099e277aaa57bb6dc9866ab0eaa544502cfc98478e32393c0c76  target/r1-runtime-fxi6ltmv/wp3c-focused-20261012-a7e2d6/auth-audit-rollback.safe
18c4e7e9fb3934b27b70888ab9951f92ef2c482aadcc0d511fad3304f33bb740  target/r1-runtime-fxi6ltmv/wp3c-focused-20261012-a7e2d6/full-contract.safe
f0a834958d58acfcae66764df0b4ced62bffa6b136077f84ca776e46ed8ebe13  target/r1-runtime-fxi6ltmv/wp3c-focused-20261012-a7e2d6/gate-drift.safe
400889ad165862369790b2fe2dcde70e80b08f7692ebe3af3bae2bf8105dd82f  target/r1-runtime-fxi6ltmv/wp3c-focused-20261012-a7e2d6/gate-ledger.safe
ab037a2423b08e7b8131f6c42e15a12d29a4c96bdc6d79c464714ef9ebf68a2e  target/r1-runtime-fxi6ltmv/wp3c-focused-20261012-a7e2d6/owner-v2-isolation.safe
2165fad69541f6ffe284278f1a2634b49fcd79df600b2836808d6386c7e2cec1  target/r1-runtime-fxi6ltmv/wp3c-focused-20261012-a7e2d6/owner-v2-rollback.safe
e8f09b61b9aa23ce4c5d02d71fe0de33269b0f8698391c241e4e1d1512e47d42  target/r1-runtime-fxi6ltmv/wp3c-focused-20261012-a7e2d6/status-concurrent-down.safe
457715d4a0ef1b73cb0c77b0a1fb4e75923db21dc8beaeeaf5b646841581ca45  target/r1-runtime-fxi6ltmv/wp3c-focused-20261012-a7e2d6/status-fenced-write.safe
06b80cc0b1fce1af8de344e63a55df84b520f319eb297d26005b89f48b9977da  target/r1-runtime-fxi6ltmv/wp3c-focused-20261012-a7e2d6/status-stale-takeover.safe
```

All nine named exact tests in the preceding addendum completed 1 passed, 0
failed, serially. They cover current59/base47/future-ledger acceptance,
required33 and drift rejection/restoration, full writer contract, the 0039
undelivered-audit refusal, V2 isolation and undo53/undo52 refusal, and the
status fence/stale-takeover/concurrent-down cases. There are no remaining
focused-test failures or SKIPs. The DB5 same-writer-count mixed-role mutation
remains the single unresolved accepted-case mismatch: PostgreSQL 18.4
canonicalizes the attempted DDL to PUBLIC-only, as described above. No other
missing or unverified contract input was found. The full DB-gated validator and
its separately reserved end-to-end acceptance remain pending.

## Final execution exit-code correction

The nine exact focused commands listed above each exited **0** in the final
serial run; each reported one passed and zero failed. The focused package check
and `-D warnings` Clippy command also exited **0**. Earlier diagnostic attempts
were not counted as passing evidence: owner V2 initially exited 101 when a
fixture DELETE met the intended immutable-row 55000 guard, then passed after
the fixture switched to transaction rollback; the gate-ledger attempt exited
101 on SQLSTATE 42601 at cursor 20115, then passed after the CASE expression
was parenthesized; and an earlier DB5 attempt exited 101 because the test
expected writer count 27 after PostgreSQL 18.4 had canonicalized the widened
role list to PUBLIC-only and count 26. The final DB5 test now passes for that
empirically reachable DDL outcome and still records the unmet
same-writer-count mixed-role requirement. The safe test summary records each
attempt, exit status, final rerun, and owned-resource cleanup.
