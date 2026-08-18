# Production operator workflows

These scripts are repository-owned orchestration and preflight helpers. They
do not contain credentials and do not replace the operator's secret manager.
The production `.env` contract is literal `KEY=value`: do not use Compose
interpolation, quotes, escapes, or inline comments; the validator rejects them
before Compose can reinterpret a value.

| Script | Default behavior | External action |
|---|---|---|
| `provision-linux.sh` | `--dry-run` | `--preflight` and `--apply` inspect/change the approved account/directories as root; `--apply` is the only mutating mode |
| `provision-db-secrets.sh` | `--dry-run` | `--check` performs a root-only read-only check of the seven DB source files; `--strip-trailing-newline` is an explicit atomic repair for a complete LF-terminated hex set; `--apply` generates new hex values and never overwrites an existing target |
| `validate-production-config.sh` | strict `--scope release` validation | `--scope infrastructure` checks only DB/bootstrap/runtime inputs; `--scope backfill` adds KIS worker inputs; no network/API call; missing values are `BLOCKED_EXTERNAL` |
| `compose-release.sh` | `--scope release --plan` | `--scope infrastructure --apply` bootstraps PostgreSQL/roles/migrations/Raw/schema without KIS, Auth0/TLS, dataset pins, or worker/API start; `--scope backfill --apply` additionally builds the research-worker image without starting its daemon; release scope starts serving after approval |
| `backfill-production.sh` | `--plan` | `--execute` calls only the read-only research worker after an explicit guard; it does not require future dataset pins |
| `post-backfill-health.sh` | `--scope backfill --plan` | `--check` runs the existing research-worker EOD freshness health gate; no KIS call |
| `self-test.sh` | static/no-infrastructure tests | none |

Production execution is intentionally split into infrastructure, data, and serving approvals:

1. host and non-KIS DB secret provisioning (`provision-linux.sh --apply`,
   `provision-db-secrets.sh --apply`, then
   `deploy/secrets/provision-runtime-secrets.sh --scope infrastructure`),
2. database/raw/schema bootstrap (`compose-release.sh --scope infrastructure --apply`),
3. KIS data bootstrap/backfill (`deploy/secrets/provision-runtime-secrets.sh --scope backfill`, then
   `compose-release.sh --scope backfill --apply`, then
   the bounded ETF `research-worker --once` backfill command), followed by the
   dependency-free worker healthcheck, curated dataset approval, and
4. the full
   serving release (`compose-release.sh --scope release --apply`).

No script enables Compose `live`, asks for a KIS account/order credential, or
calls an order endpoint. KOSPI200/KOSDAQ150 candidate backfill is a separate
blocked workflow until its credentialed candidate bridge and entitlement are
available. See [`docs/runbooks/kis-production-backfill.md`](../../docs/runbooks/kis-production-backfill.md).

`validate-production-config.sh --scope infrastructure` intentionally does not
require KIS credentials, serving-only Auth0/TLS values, or the recommendation
dataset five-pin. The subsequent `--scope backfill` also does not require
serving-only values or pins. Those
values are outputs/approval inputs produced after the ETF Raw→Curated review.
The backfill state identity binds only pre-run inputs (date range, universe,
code commit, entitlement, and source scope), so entering the approved pin later
does not invalidate a resumable backfill. The backfill scope does not start the
worker daemon: each date is an isolated `docker compose run --rm --no-deps
research-worker --once` invocation, preventing the 16:30 daemon from fetching
outside the approved range. After the dependency-free
`scripts/ops/post-backfill-health.sh --scope backfill --check`, run the full
release scope and then `scripts/ops/post-backfill-health.sh --scope release --check`.

## Generate the DB source secrets

After host `--preflight` passes, inspect the DB-only plan first:

```sh
scripts/ops/provision-db-secrets.sh --dry-run
```

The default destination is `/etc/lagrange/secrets`. Apply it explicitly as
root:

```sh
sudo scripts/ops/provision-db-secrets.sh --apply
```

After provisioning (or when the files already exist), verify the complete set
without changing anything:

```sh
sudo scripts/ops/provision-db-secrets.sh --check
```

`--check` requires root because the production source directory is protected.
It checks the exact seven named targets when they are regular, non-symlink
files owned by `root:root` with mode `0600`, containing either exactly 64
lowercase hexadecimal bytes or a canonical 44-character standard Base64 value
that decodes to 32 bytes, with no newline, and pairwise-distinct values. It
prints `DB_SECRET_CHECK: PASS` on success; failures report filenames and safe
shape/metadata reasons only, never secret values or hashes. The check is
read-only.

If all seven files are otherwise valid 64-hex values with exactly one trailing
LF or CRLF, repair them atomically with the explicit root-only command:

```sh
sudo scripts/ops/provision-db-secrets.sh --strip-trailing-newline
```

It refuses missing, malformed, mixed, or partially repairable sets and makes
no changes until the complete set passes preflight. `--normalize` is an alias.

The script creates only these seven files:
`postgres_password`, `db_migration_owner_password`, `db_app_password`,
`db_worker_password`, `db_audit_password`, `db_research_password`, and
`db_admin_password`. Each is an independently generated 32-byte OpenSSL hex
value with no line terminator, mode `0600`, and owner `root:root`. The script
checks every target before writing, stages all values privately, verifies that
they are distinct, and refuses to overwrite or leave a partial set when a
target already exists. Before staging, the destination directory must be
owned by UID 0 and must not be group- or other-writable; its group and exact
mode are otherwise left unchanged, so the host's `0700` or `0750` directory
is valid. It never prints values or hashes. Runtime copies and database role
creation remain separate commands.
