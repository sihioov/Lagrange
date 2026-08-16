# Disposable PostgreSQL integration validation

`validate.sh` is the repeatable upgrade lane for migrations 0039–0041. It
reuses the pinned PostgreSQL 18 image from `deploy/qa/qa-db.compose.yml`, the
role bootstrap from `deploy/db/bootstrap-roles.sh`, and the migration image
from `deploy/db/Dockerfile`.

The workflow creates a private temporary directory, generates one-line 256-bit
credentials, and never logs their values. The upgrade cluster is provisioned
with independent role credentials and a seeded 0038 baseline containing a
normalized pending invitation, two Owners for the identity boundary check, and
one terminal legacy Paper target. It then runs 0039, 0040, and 0041 as separate
SQLx source stages, checking migration counts and invariants after each stage.

The SQL preflights also verify migration-owner table/function ownership,
SECURITY DEFINER search paths and EXECUTE ACLs, forced RLS on the new outbox
surfaces, and the absence of direct serving-role outbox DML privileges.

The Rust test harnesses in this repository derive each serving-role URL from
the supervisor URL's password. The selected job-queue helper still uses the
existing synthetic `lagrange` fixture for role URLs, so the second disposable
test cluster uses that non-production fixture only; it never reads an
operator credential. The upgrade cluster still uses independent generated
credentials for every role. Every selected cargo command receives
`DATABASE_URL`; any `SKIP:` output is a hard failure.

Immediately after 0039, the lane inserts one synthetic undelivered auth-audit
row as `migration_owner` and executes the tracked down migration. The command
must fail with SQLSTATE 55000, and a postflight verifies the row and migration
ledger survived before removing only that fixture.

Run the no-daemon checks locally:

```sh
bash deploy/db/integration-validation/static-check.sh
bash deploy/db/integration-validation/validate.sh --self-test
```

Run the full lane on a host with an accessible Docker Engine:

```sh
bash deploy/db/integration-validation/validate.sh \
  --evidence-dir /tmp/lagrange-pg-validation-evidence
```

When `/var/run/docker.sock` is group-readable but the current shell has stale
group membership, refresh the login or wrap the command with `sg docker -c`.

The default exit status is `0` only for an approved run, `1` for a real
validation/test defect, and `2` for an external prerequisite blocker. Both
clusters are torn down with `docker compose down -v --remove-orphans` from a
trap, including partial-start failures. Evidence files are sanitized and
written outside Git by default; inspect `evidence.json`, `evidence.tsv`, and
the individual `*.log` files before making a Go/No-Go decision. Use
`EVIDENCE_TEMPLATE.md` for the review record.

`migration-safety-audit.sh` is an explicit static gate for rollback guards,
identity actor binding, and auth-audit failure retention. It intentionally
fails closed when those migration/application invariants are absent. Do not
silence or work around that result; attach it to the evidence and resolve the
underlying defect before a Go decision.
