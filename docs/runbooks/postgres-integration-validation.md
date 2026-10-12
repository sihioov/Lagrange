# PostgreSQL integration-validation runbook

Use this runbook before accepting the staged 0038-to-0041 upgrade and the
empty-database full-release migration lane. Both use disposable PostgreSQL
clusters and synthetic data only. Never point this harness at a production or
operator-owned database.

## Preconditions and static checks

The operator needs the repository checkout, Bash, OpenSSL, Cargo, the Docker
CLI, and an accessible Docker Engine. The workflow uses the pinned database
image in `deploy/qa/qa-db.compose.yml`; credentials are generated locally and
are not supplied through command-line arguments.

Run the no-daemon checks first:

```sh
bash deploy/db/integration-validation/validate.sh --self-test
bash deploy/db/integration-validation/static-check.sh
bash deploy/db/integration-validation/migration-safety-audit.sh
```

A migration safety-audit finding is a No-Go. Do not edit migration history or
weaken an assertion merely to make the validator pass.

## Disposable validation run

```sh
evidence_dir="$(mktemp -d /tmp/lagrange-pg-validation-evidence.XXXXXX)"
chmod 700 "$evidence_dir"
bash deploy/db/integration-validation/validate.sh \
  --evidence-dir "$evidence_dir" \
  --main-port 55432 --test-port 55433 --full-port 55434
```

The three clusters run sequentially under distinct Compose project names and
generated credentials:

1. **Staged upgrade cluster:** apply stage 0038 with the migration image's
   SQLx CLI through a `/bin/bash` entrypoint override. The container reads the
   generated migration-owner password file, requires exactly 64 lowercase hex
   characters, constructs its fixed database URL internally, and sets finite
   `PGOPTIONS` timeouts. This schema-only operation preserves the SQLx ledger
   without installing the current catalog before 0039.
2. **Staged upgrade continuation:** seed only the synthetic legacy fixture,
   then apply 0039, 0040, 0041, and rerun 0041 through the normal migration
   entrypoint. The first catalog install follows 0039 and must enqueue exactly
   one event with the baseline file hash, action, target, reason, and released
   time. Require exactly six strategies: five manifest-derived Draft baseline
   rows and the unchanged `postgres_validation_paper` Paper fixture; versions
   and parameter schemas remain exactly five baseline rows. Disposable
   negative probes inject an unexpected strategy, remove and mutate a baseline,
   and add the Paper fixture to the full lane. Each must fail with its exact
   SQLSTATE/reason, explicitly roll back, and preserve the catalog/audit
   snapshot. Run a normal 0039 catalog-sync rerun and compare catalog rows and
   audit identity/count. A synthetic conflict probe runs the actual catalog
   SQL inside a transaction as `migration_owner`; it must fail with SQLSTATE
   23505, roll back, and leave the original snapshot intact. The down-guard
   probe remains separate: the tracked 0039 down migration must refuse the
   synthetic undelivered fixture with SQLSTATE 55000, and cleanup removes only
   that fixture while retaining the catalog event. Check the terminal-Paper
   count in the supervisor pass because forced RLS hides `pending_targets`
   from `migration_owner`; keep catalog audit visibility checks under the
   actual `migration_owner` login.
3. **Empty full-release cluster:** start a separately owned, unseeded cluster
   with the same pinned database image, generated credentials, role bootstrap,
   and migration image. Run the normal image entrypoint through all current up
   migrations (0059 at baseline `ac0a083`), then run it again. Record the count,
   latest version, and every SQLx checksum for both runs. The second ledger and
   catalog/audit snapshot must exactly match the first. Require exactly five
   baseline strategies and no Paper fixture. Tear down this owned cluster and
   verify its project has no remaining containers before starting the DB-gated
   test cluster.
4. **DB-gated test cluster:** use the existing synthetic compatibility
   fixture, bootstrap its roles, and run the listed Rust suites with
   `DATABASE_URL`. Any `SKIP:` marker is a hard failure.

The workflow checks direct role logins, table ownership, catalog function
identity, its safe `search_path`, EXECUTE ACL, forced RLS, migration counts,
and available disk space. It does not alter grants. Each started cluster is
torn down with `docker compose down -v --remove-orphans`, including partial
startup failures.

## Evidence and Go/No-Go

The evidence directory contains sanitized logs, `evidence.json`, and
`evidence.tsv`. The staged 0038-to-0041 results, catalog negative probes, and
full-release first/second results are recorded separately; full-release
checksum lists are included in both records. Do not copy generated passwords, `DATABASE_URL`, Docker inspect
environment, or unsanitized command output into a review report.

Go only when the evidence verdict is `APPROVED`, the staged counts are 38, 39,
40, and 41 with the 0041 rerun unchanged, the down guard observed SQLSTATE
55000, and the catalog audit/conflict/rerun checks pass. The empty full-release
lane must reach the current endpoint with matching first/second ledger
checksums and a no-op rerun. Every required DB-gated test must pass without
`SKIP:`, direct role logins and ownership checks must pass, and teardown must
leave no validator clusters behind.

An unavailable Docker Engine is `BLOCKED_EXTERNAL`, not database acceptance.
If the socket is group-readable but the shell has stale supplementary groups,
refresh the login or use a temporary `sg docker -c '...'` wrapper. A blocker
does not authorize a real database URL or trust-authentication workaround.
