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
| `provision-crypto-secrets.sh` | `--dry-run` | `--check` performs a root-only read-only check; `--apply` generates four distinct 256-bit values for session/CSRF/cursor/backup encryption and atomically installs them without overwriting existing targets |
| `provision-auth0-secret.sh` | `--dry-run` | `--check` performs a root-only read-only check; `--apply` reads the secret twice from a hidden terminal prompt; `--import-file` migrates a protected legacy file; both atomically install one root-owned file without overwriting it |
| `validate-production-config.sh` | strict `--scope release` validation (root-only read-only inspection) | `--scope infrastructure` checks only DB/bootstrap/runtime inputs; `--scope backfill` adds KIS worker inputs; no network/API call; missing values are `BLOCKED_EXTERNAL` |
| `compose-release.sh` | `--scope release --plan` (root-only; it invokes the validator) | `--scope infrastructure --apply` bootstraps PostgreSQL/roles/migrations/Raw/schema without KIS, Auth0/TLS, dataset pins, or worker/API start; `--scope backfill --apply` additionally builds the research-worker image without starting its daemon; release scope starts serving after approval |
| `backfill-production.sh` | `--plan` | `--execute` is root-only because it validates protected production secrets, then calls only the read-only research worker after an explicit guard; it does not require future dataset pins |
| `post-backfill-health.sh` | `--scope backfill --plan` | `--check` is root-only because it validates protected production secrets, then runs the existing research-worker EOD freshness health gate; no KIS call |
| `self-test.sh` | static/no-infrastructure tests | none |

Production execution is intentionally split into infrastructure, data, and serving approvals:

1. host and non-KIS source secret provisioning (`provision-linux.sh --apply`,
   `provision-db-secrets.sh --apply`, `provision-crypto-secrets.sh --apply`,
   then
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

## Generate the non-KIS cryptographic source secrets

The four source files that are independent of KIS/Auth0/database access are
`session_secret`, `csrf_secret`, `cursor_secret`, and
`backup_encryption_key`. Inspect the no-change plan first:

```sh
scripts/ops/provision-crypto-secrets.sh --dry-run
```

Generate them only with the explicit root operation, then verify read-only:

```sh
sudo scripts/ops/provision-crypto-secrets.sh --apply
sudo scripts/ops/provision-crypto-secrets.sh --check
```

Each file is an independently generated 256-bit OpenSSL CSPRNG value encoded
as exactly 64 lowercase hexadecimal characters, with no trailing newline,
owned by `root:root`, mode `0600`, and pairwise distinct. This representation
matches the API cursor loader's accepted 64-hex/32-byte contract; session and
CSRF skeletons and the backup encryption passphrase contract require at least
256 bits, which the same representation supplies. The helper performs no KIS,
Auth0, database, or other network/API call. It never prints values, accepts a
secret through argv/environment, or overwrites an existing target.

`--check` reports only filenames plus metadata/shape reasons and does not emit
secret contents or hashes. The safe `--source-dir ABSOLUTE_PATH` override is
available for isolated tests; its directory must be root-owned, not
group/other-writable, and have no `..` component or symlinked ancestor.

## Provision the Auth0 client secret

The production target is `/etc/lagrange/secrets/auth0_client_secret`. Inspect
the plan first; the default mode never reads a secret or changes the host:

```sh
scripts/ops/provision-auth0-secret.sh --dry-run
```

Apply only after the protected source directory is ready:

```sh
sudo scripts/ops/provision-auth0-secret.sh --apply
sudo scripts/ops/provision-auth0-secret.sh --check
```

To migrate an existing legacy file without retyping its value, pass only the
absolute source path. The source is never printed or deleted:

```sh
sudo scripts/ops/provision-auth0-secret.sh \
  --import-file /var/lib/lagrange/legacy/auth0_client_secret
```

The import source must be a regular non-symlink file with no symlinked
ancestor, owner-read permission, and no group/other permission bits (`0600`
is preferred). Its value must pass the same non-empty, printable,
whitespace-free, no-placeholder checks as interactive input.

`--apply` is root-only and reads the value twice from `/dev/tty` with terminal
echo disabled. It does not accept the client secret in an argument, an
environment variable, standard input, or a command substitution, and it never
prints or logs the value. The script rejects empty, multiline, CR/LF,
whitespace, non-printable, and common placeholder values, then writes exactly
one line without a trailing newline as `root:root` mode `0600`. The existing
target is never overwritten; the final install is an atomic same-filesystem
no-clobber link. No Auth0 network/API call or vendor-side secret verification
is performed.

`--check` is root-only and read-only. It reports only the source-directory and
target metadata/shape (regular non-symlink file, ownership, mode, byte shape,
and placeholder status), never the secret or a hash. For an isolated test,
pass a safe absolute directory with no `..` component or symlinked ancestor:

```sh
sudo scripts/ops/provision-auth0-secret.sh --check --source-dir /var/lib/lagrange/test-secrets
```

The target source directory must be owned by UID 0 and must not be group/other
writable. `--source-dir` changes the target parent directory for an isolated
host or test; it does not provide a way to inject a secret non-interactively.

`validate-production-config.sh` is root-only for every validation scope because
the production source secrets and service-specific runtime copies are
root-owned and mode-protected. `--help` remains available without root. When
using `sudo`, preserve the exact build commit explicitly:

```sh
export LAGRANGE_CODE_COMMIT="$(git rev-parse HEAD)"
sudo env LAGRANGE_CODE_COMMIT="$LAGRANGE_CODE_COMMIT" \
  scripts/ops/validate-production-config.sh \
  --scope infrastructure --env-file deploy/compose/.env
```

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
