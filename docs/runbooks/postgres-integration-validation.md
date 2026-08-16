# PostgreSQL integration-validation runbook

Use this runbook before approving the 0039–0041 database upgrade. The lane is
disposable and uses only synthetic credentials/data. It must not be pointed at
a production or operator-owned PostgreSQL cluster.

## Preconditions

The operator needs the repository checkout, Bash, OpenSSL, Cargo, and the
Docker CLI with a reachable Docker Engine. The workflow uses the pinned image
digest already checked into `deploy/qa/qa-db.compose.yml`; it does not install
host packages or accept real credentials.

Check the local harness first:

```sh
bash deploy/db/integration-validation/validate.sh --self-test
bash deploy/db/integration-validation/static-check.sh
```

The second command includes the migration safety audit. A failure is a No-Go
and should be attached to the evidence; do not edit migration SQL solely to
make the harness green without first reproducing and reviewing the defect.

## Full disposable run

```sh
evidence_dir="$(mktemp -d /tmp/lagrange-pg-validation-evidence.XXXXXX)"
chmod 700 "$evidence_dir"
bash deploy/db/integration-validation/validate.sh \
  --evidence-dir "$evidence_dir"
```

The workflow performs the following sequence:

1. Generate private one-line 256-bit credentials outside Git and render two
   temporary Compose overrides.
2. Start the upgrade PostgreSQL 18.4 cluster and provision
   `migration_owner`, `app`, `worker`, `audit_writer`, `research_writer`, and
   `admin` with the existing fail-closed bootstrap script.
3. Build/use the existing `deploy/db` migration image and apply a staged 0038
   source, giving a representative pre-0039 baseline.
4. Run disk/connection hazard checks, normalized pending-invite duplicate
   checks, role/schema/version/ownership checks, and terminal Paper counts.
5. Apply 0039, preflight its auth-audit objects, apply 0040, preflight its
   normalized invite index and identity functions, then apply 0041 and verify
   backfill/obligation coverage. Rerun 0041 and repeat the final preflight to
   prove a no-op rerun.
6. Execute the tracked 0039 down migration against a synthetic undelivered
   auth-audit row; require SQLSTATE 55000 and verify the ledger/table survive.
7. Start a separate test cluster and run the migration-contract, API RLS and
   Paper execution/notification/scheduler/runner, and job-queue contract and
   Paper-preview suites with `DATABASE_URL` set. Any `SKIP:` marker fails the
   lane.
8. Collect sanitized evidence and tear down both clusters, including a
   partially started service.

## Go/No-Go

Go only when `evidence.json` says `APPROVED`, all sequential versions and
preflights and the 0039 down-guard probe are present, direct service-role logins report
`current_user=session_user`, all public tables are migration-owner owned,
normalized pending-invite duplicates are zero, every terminal Paper target has
an active/archive settlement obligation, disk and connection headroom pass,
all selected tests pass with no `SKIP:`, the migration safety audit passes,
and teardown evidence shows no retained validation containers.

No-Go on any test failure, skipped DB gate, migration safety-audit finding,
version/count mismatch, duplicate, ownership/privilege drift, outbox gap,
hazard failure, missing/unsanitized evidence, or Docker/PostgreSQL blocker.
Use `deploy/db/integration-validation/EVIDENCE_TEMPLATE.md` to record the
decision and attach the sanitized log directory.

## External blocker handling

If Docker CLI is present but `docker info` cannot reach the daemon, the full
lane exits `2` as `BLOCKED_EXTERNAL` and writes a sanitized `docker-info.log`.
If the operator is already listed in the `docker` group but the current shell
has stale supplementary groups, refresh the login or use a temporary
`sg docker -c '...'` wrapper; do not enable a trust-auth database or point at
an operator-owned cluster.
This is not an approval and does not justify setting `DATABASE_URL` to a real
database. Resolve the daemon/host prerequisite, then rerun the exact command.
The `--self-test` and shell syntax/diff checks remain valid evidence that the
harness itself is syntactically and structurally ready; `static-check.sh` may
also report independent migration/application safety findings that remain
No-Go until fixed.
