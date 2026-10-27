# Disposable PostgreSQL integration validation

`validate.sh` runs three isolated database lanes in sequence. It uses the
pinned PostgreSQL 18 image from `deploy/qa/qa-db.compose.yml`, the existing
role bootstrap, and the migration image from `deploy/db/Dockerfile`. Each lane
has its own Compose project and credentials; each owned cluster is removed by
the exit trap.

## Staged upgrade lane

Before migrations, the unchanged production role bootstrap is exercised by
`bootstrap-negative.sh` in this owned disposable cluster. The fixture runs as
the non-root postgres OS user and emits 83 distinct case outcomes: three
positive controls, seven invalid-role attributes, two membership directions,
eight direct/transitive protected-role paths, six CREATE privilege cases,
56 failures across all eight credential files, and one real lock-timeout
rollback. Credentials, password verifiers and raw failure output are never
printed. The JSON evidence includes every named outcome.

Each rejected bootstrap must preserve the captured role attributes/password
verifiers, memberships, database ownership/ACL and three fixture schema
ownership/ACLs. ACLs are compared as sorted effective grant entries. A second
connection acknowledges its database ACL lock before the late-failure case;
the real bootstrap must complete all role statements and revoke public-schema
CREATE, then fail at the following database GRANT with its existing lock
timeout. The rollback must restore the earlier ACL and leave the new admitter
absent. This verifies bootstrap transaction rollback, not migration0060 DOWN.
The helper refuses migrated/colliding fixtures, has finite timeouts and removes
its private credential copies. Existing role-login checks follow it.

The upgrade cluster starts empty and receives the schema-only SQLx stage 0038.
For this disposable stage only, the workflow overrides the migration image
entrypoint with `/bin/bash`, validates its generated migration-owner password
as exactly 64 lowercase hex characters inside the container, constructs the
fixed `qa-db:5432/postgres` URL in that container's environment, and invokes
the image's SQLx CLI with finite lock and statement timeouts. It does not run
the current catalog installer before 0039.

After the synthetic 0038 baseline fixture is prepared, stages 0039, 0040, 0041,
and the 0041 rerun use the normal migration image entrypoint. The first catalog
installation therefore follows 0039, where its audit function exists. The
preflights require exactly six strategy rows: the five exact Draft baseline
IDs/content from `baseline-v1.json` plus the unchanged
`postgres_validation_paper` Paper fixture. Versions and parameter schemas are
exactly the five baseline rows. They also verify the one exact catalog-install
audit payload, migration-owner ownership and function identity, safe
`search_path`, EXECUTE ACL, and forced RLS. Disposable negative probes inject
an unexpected strategy, remove or mutate a baseline, and add the Paper fixture
to the full lane; each must fail with its exact SQLSTATE/reason, explicitly
roll back, and preserve the catalog/audit snapshot. A 0039 rerun verifies
stable catalog rows and audit identity. A synthetic catalog conflict must fail
with SQLSTATE 23505 and roll back. The separate 0039 down-guard fixture still
requires SQLSTATE 55000 and removes only its own synthetic row; a subsequent
preflight confirms the catalog event remains. The legacy terminal-Paper count
is checked in a supervisor pass because `migration_owner` is subject to forced
RLS on `pending_targets`; catalog audit access remains verified by the actual
`migration_owner` login.

The same staged cluster then advances to 0059. `preflight-0060.sql` uses a direct
migration-owner login to seed eight synthetic historical recommendation rows
(v2/v3 pins × PENDING/FAILED/CANCELED/SUCCEEDED), retaining their complete JSON,
items, job payloads, timestamps, the prior function definitions and 59 checksums.
After 0060, both the initial pass and no-op rerun must preserve that history and
produce 60 ledger rows. The ten daily tables must be migration-owned with forced
RLS, with no control activation or config opt-in.

`actual-role-0060.sql` separately logs in as app, worker, research_writer, admin
and daily_input_admitter and checks CREATE, SET ROLE, DML and private-function
denials plus the rightful empty-control entrypoints. `rollback-0060-guard.sql`
rehearses unused DOWN/UP in one transaction, comparing the old functions, grants,
history and RLS. A second Owner's edited-and-restored config then requires DOWN
to refuse with SQLSTATE 55000 despite being invisible to the first actor. Its
failed transaction must preserve schema, history, epochs and FORCE RLS.

## Empty full-release lane

A second disposable cluster has its own generated credentials and role
bootstrap. It uses the same pinned PostgreSQL image and the same migration
image, starts with no seeded application data, and runs the normal image
entrypoint through every current up migration. The 0060 candidate requires
exactly 60 migrations. It then runs the normal entrypoint a second time and records
the migration count, latest version, and every SQLx checksum for both passes.
The second pass must leave the ledger checksums and catalog row/audit identity
unchanged and require exactly five strategies, with no Paper fixture. This
lane is separate from the staged 0038-to-0060 upgrade cluster. The full-release
cluster is torn down and its Compose project checked for remaining containers
before the DB-gated test cluster starts.

## DB-gated Rust tests

The selected test suites use a third disposable cluster. Existing integration
helpers derive serving-role URLs from the supervisor URL; the selected
job-queue helper still uses the repository's synthetic `lagrange` compatibility
fixture. It is not an operator or production credential. Every selected Cargo
command receives `DATABASE_URL`; any `SKIP:` output is a hard failure.

The 32 `daily_0060_*` definitions use direct logins and isolated scratch databases.
Their source also defines 73 fixed subcase outcomes, including real two-connection
blocking observations. The cutoff-boundary case uses a private fixture clock
adapter feeding the unchanged pure cutoff function; it is not live wall-clock
observation. The isolated DOWN-category fixtures temporarily remove dependencies
inside transactions that are always rolled back. They exercise refusal guards,
not a production method for bypassing constraints.

The harness requires exactly one successful libtest row for each of the 32 names
and one PASS for each of the 73 subcases. It emits `PG_DB_TEST_ROW`,
`DAILY_0060_SUBCASE` and `PG_DB_TEST_EXIT` fixed records and retains them in
`evidence.json`, including each of the twelve Cargo process exit codes and SKIP
checks. Missing rows fail the harness. Written definitions and workspace tests
without DATABASE_URL do not count as DB execution evidence.

### R4-06 source registration

The simulation execution lane now requires 16 exact named libtest rows,
including `official_owner_filter_and_exact_attempt_heartbeat`. The R4 verification
builder also registers 292 fully qualified private process tests plus the
expanded runner, CLI and process inventories. The existing production-ops
self-test registers the simulation operations and backup Python suites before
its fixture PATH shims. These are SOURCE registrations, **NOT RUN** results.

Actual observer SQL projections, PostgreSQL defaults, SQLx raw decoding and
transaction-command responses still require disposable-DB execution. Migration
text checks and synthetic seams do not establish those properties. All existing
required CI aggregation, DB suites, Web checks and research smoke gates remain.

## Run

Run the no-daemon checks locally:

```sh
bash deploy/db/integration-validation/static-check.sh
bash deploy/db/integration-validation/validate.sh --self-test
```

Run all three lanes on a host with an accessible Docker Engine:

```sh
bash deploy/db/integration-validation/validate.sh \
  --evidence-dir /tmp/lagrange-pg-validation-evidence \
  --main-port 55432 --test-port 55433 --full-port 55434
```

The defaults use those same three distinct loopback ports. The runner does not
accept database credentials or arbitrary DB connection components. If the
current shell has stale Docker group membership, refresh the login or run the
command through `sg docker -c`.

Exit status is `0` only for `APPROVED`, `1` for a validation defect, and `2`
for an external prerequisite blocker. Sanitized `evidence.json`,
`evidence.tsv`, and named logs distinguish the staged lane, its catalog
negative probes, full-release first and second runs, the 0039 and 0041 reruns,
the catalog conflict probe, full-release cleanup, and the DB-gated tests.
Inspect the recorded cleanup logs before a Go/No-Go decision.

`migration-safety-audit.sh` remains an explicit static gate for rollback
guards, identity actor binding, and auth-audit failure retention. Do not
silence a finding to make the harness green.
