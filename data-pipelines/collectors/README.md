# data-pipelines/collectors — KRX research ingestion and publication

This deliverable provides the provider-neutral KRX EOD contract, immutable Raw
storage, verified Raw-to-PostgreSQL publication, crash recovery, and the
scheduled `research-worker`. The implementation is operational with recorded
synthetic fixtures for development and QA.

> **Production boundary:** synthetic data is for development and QA only.
> `APP_ENV=production` with `RESEARCH_FETCH_MODE=synthetic` is rejected before
> the worker reads a secret, constructs a provider, touches Raw storage, or
> creates a database pool. A licensed, credentialed, entitlement-aware real KRX
> HTTP transport is **not implemented**. Real credentials, a licensed endpoint,
> entitlement enforcement in that transport, and operator provisioning remain
> external production work; this repository does not claim that a real KRX feed
> is live.

## Provider contract and immutable Raw

`crates/market-data/src/contract.rs` defines the raw response envelope:

| field | meaning |
|---|---|
| `bytes` | provider response/file, stored byte-for-byte before parsing |
| `retrieved_at` | UTC instant the delivery was retrieved |
| `request` | provider request metadata (endpoint, query, redacted headers, mode) |
| `batch_id` | unique ingestion batch for this delivery |
| `content_hash` | `sha256:<64 lowercase hex>` over `bytes` |

The four response classes are `bars`, `reference`, `calendar`, and
`corporate_actions`. `KrxProvider` currently supports deterministic playback of
the recorded synthetic bundle under `tests/fixtures/kr-etf/contract/`. Recorded
timeouts and malformed variants exercise stable failure handling without
network or credentials.

Raw is the durable recovery authority:

```text
data/
└── raw/
    ├── provider=KRX/market=KR/date=2020-01-31/
    │   └── batch=<batch_id>/
    │       ├── bars-response.json
    │       ├── reference-response.json
    │       ├── calendar-response.json
    │       ├── corporate-actions-response.json
    │       └── batch.json
    └── manifests/provider=KRX/market=KR/manifest.jsonl
```

Every successful delivery creates a new batch directory with `create_new`
semantics and appends one manifest line. Identical bytes delivered twice still
produce two batch IDs; no existing evidence is overwritten. Pre-publication
failures are cleaned up or remain non-committed and invisible to recovery. Once
final batch metadata is visible, any later indeterminate durability/manifest
failure preserves the exact identity so manifest discovery can re-sync it.
Verified reads detect missing, changed, or mis-sized evidence. A database
failure after Raw commit therefore preserves the exact source batch for retry
instead of fetching a replacement.

## Manual commands

Run commands from the repository root. Synthetic mode is shown because it is
the only implemented transport.

Raw-only collection (no database configuration or write):

```sh
cargo run -p collectors -- ingest-krx \
  --root data \
  --date 2020-01-31 \
  --mode synthetic \
  --bundle tests/fixtures/kr-etf/contract
```

Collect into immutable Raw and publish the same verified batch to PostgreSQL:

```powershell
$env:DATABASE_URL = 'postgres://research_writer:<password>@127.0.0.1:5432/lagrange'
cargo run -p collectors -- ingest-and-publish-krx `
  --root data `
  --date 2020-01-31 `
  --mode synthetic `
  --bundle tests/fixtures/kr-etf/contract
```

`DATABASE_URL` is required and used as database configuration only by
`ingest-and-publish-krx`. Before command dispatch, the manual `collectors`
process also reads any present `DATABASE_URL` solely to seed log redaction.
Consequently, `ingest-krx` may read the value for redaction but never parses it,
opens a database connection, or performs a database write. `research-worker`
never reads `DATABASE_URL`; it uses only the discrete DB settings documented
below. Both manual commands emit a JSON outcome on stdout and redacted
diagnostics on stderr. Exit codes are 0 for success, 1 for CLI usage, and 2 for
a typed ingest/publication failure.

## Worker commands

With the environment below configured, run a single target date:

```sh
cargo run -p collectors --bin research-worker -- --once --date 2020-01-31
```

Run the daemon (no positional arguments):

```sh
cargo run -p collectors --bin research-worker --
```

The daemon schedules the current Korean civil date once per day at
`RESEARCH_RUN_AT_KST`, default `16:30` KST. A daemon started at or after that
time performs one immediate current-KST-date catch-up after recovery, then
schedules the next day. Starting before the configured time waits for today's
slot. The daemon repeats recovery immediately before every catch-up or scheduled
cycle, so a Raw append racing the preceding completion check is consumed no
later than the next cycle. The one-shot and daemon both perform startup recovery
before checking or fetching their target date. The duplicate check skips a date only when an `EOD`
row already exists, so restart catch-up is idempotent; `EOD_UNAVAILABLE` does
not suppress a later retry.

Run the database-backed health probe:

```sh
cargo run -p collectors --bin research-worker -- healthcheck
```

The healthcheck needs only `RESEARCH_MAX_PUBLICATION_AGE_SECS` and the five DB
settings. It verifies database connectivity and the newest KRX/KR row whose
kind is exactly `EOD` and whose batch date is no later than the current Korean
civil date. The freshness instant is the earlier of its retrieval time and the
end of its Korean batch date. This keeps a current-date publication accurate
to its retrieval time while ensuring a newly replayed historical backfill stays
historically stale. Future batch dates cannot supersede an applicable EOD, and
future/negative effective ages fail closed. The default maximum is `345600`
seconds (four days). `EOD_UNAVAILABLE` means the provider response contained no
bar for the requested date—for example, because that day's EOD was not yet
available—and is intentionally excluded from both freshness and
duplicate-success checks. Risk Gateway uses the same selection and effective
freshness rule.

### Worker environment

These are the complete worker environment keys accepted by the executable;
there are no environment-configurable retry, query, pool, or child-process
timeout variables.

| variable | required/default | contract |
|---|---|---|
| `APP_ENV` | required | `development`, `qa`, or `production` |
| `RESEARCH_FETCH_MODE` | required | `synthetic` or `credentialed`; credentialed currently fails permanently because the real provider is not implemented |
| `RESEARCH_RUN_AT_KST` | default `16:30` | exact `HH:MM` daily daemon time in KST |
| `RESEARCH_MAX_PUBLICATION_AGE_SECS` | default `345600` | positive healthcheck maximum age in seconds |
| `RESEARCH_RAW_ROOT` | required | writable data root; `RawStore` appends `/raw`, so Compose sets `/data` while mounting host `${LAGRANGE_DATA_DIR}/raw` at `/data/raw` |
| `RESEARCH_SYNTHETIC_BUNDLE` | default `tests/fixtures/kr-etf/contract` | recorded bundle path; development/QA only |
| `DB_HOST` | required | PostgreSQL host |
| `DB_PORT` | required | positive PostgreSQL port |
| `DB_NAME` | required | PostgreSQL database |
| `DB_USER` | required | must be the least-privilege `research_writer` in deployment |
| `DB_PASSWORD_FILE` | required | path to a readable file containing a nonempty DB password |
| `KRX_CREDENTIAL_FILE` | credentialed mode only | path to a readable file containing a nonempty provider credential; reading it does not make the unimplemented transport available |

`DB_PASSWORD_FILE` and `KRX_CREDENTIAL_FILE` are paths, not secret values. The
worker reads the file, trims surrounding whitespace, rejects missing, unreadable,
or empty files, and never falls back to a plaintext password environment
variable. It does not accept `DATABASE_URL`. On Windows only, `SYSTEMROOT` is
validated and passed to contained helper processes as an OS requirement; it is
not worker configuration.

Timeouts are fixed in code: a whole helper attempt is 60 seconds, pool acquire
is 10 seconds, individual query supervision and PostgreSQL `statement_timeout`
are 15 seconds, PostgreSQL `lock_timeout` is 5 seconds, and child reaping is 5
seconds. Retry backoff is stable exponential delay from 10 seconds, capped at
600 seconds.

### Recovery, failures, and events

Recovery captures the append-order manifest's immutable batch-ID high-water,
then sorts only that fixed prefix oldest-first by `retrieved_at` and batch ID.
Each contained helper emits at most 16 strict per-batch NDJSON events plus a
terminal `snapshot_high_water`/cursor/`has_more`. Events carry the same
high-water so the parent can retain the last validated snapshot and cursor
across a retryable timeout/failure; the next 60-second helper resumes strictly
after it. A commit whose event was lost is safely exact-replayed once. After a
snapshot finishes, recovery consumes only the append suffix after its
high-water and completes only when a fresh completion check observes the same
append-order high-water. Thus a concurrently appended backdated batch cannot
sort behind an already emitted cursor and be skipped. Missing or mismatched
high-water/cursor values are permanent integrity failures. Only after an
unchanged completion check is the in-memory position reset, preserving
authoritative full-history replay on the next process start. For every exact
source batch recovery checks PostgreSQL state:

- missing: verify every Raw file and atomically publish that same batch;
- complete: replay and verify exact idempotency, then report it as skipped;
- partial or conflicting: stop with a permanent error and preserve the Raw
  evidence for investigation.

Recovery completes before any fresh fetch. After a retryable ingest/publication
failure, the worker returns to recovery before another fetch, preventing a new
batch from replacing the durable failed batch. PostgreSQL publication is one
transaction: four `data_batches` rows plus immutable calendar history and the
latest calendar projection commit together or roll back together.

Retryable failures are transient provider timeout/I/O, Raw I/O, retryable
PostgreSQL conditions, helper I/O, and attempt timeout. The worker retries these
indefinitely with capped backoff until success or shutdown. Configuration,
synthetic-in-production, secret-file, missing credentialed provider, malformed
or tampered Raw, unsafe paths, partial/conflicting publication, unhealthy probe,
and invalid helper output are permanent and stop the command/cycle.

Stdout is newline-delimited JSON. Event records use stable `event` values
`retrying`, `failed`, `recovered`, `completed`, and `skipped`, with `phase`,
`class` (`success`, `retryable`, or `permanent`), target date, and batch ID when
available. Public command final records report `published`,
`already_published`, `healthy`, or `shutdown`; failures exit 2 with a stable
`error_code` and class.

## PostgreSQL and Compose operator contract

`research_writer` is deliberately narrower than a generic worker. It has schema
usage and only:

- `SELECT, INSERT` on `data_batches`;
- `SELECT, INSERT` on append-only `trading_calendar_versions`;
- `SELECT, INSERT, UPDATE` on the current `trading_calendars` projection.

It has no schema `CREATE` and no table `DELETE`, `TRUNCATE`, ownership, role, or
migration authority. Calendar corrections append a new immutable history row;
only a strictly newer `retrieved_at` may advance the current projection. Equal
time must match, and older evidence never rolls the projection back.

Operators must provision the external role and secret files, then apply all
migrations with the migration owner and a finite external lock timeout before
starting the worker:

```sh
PGOPTIONS='-c lock_timeout=5s' sqlx migrate run
```

The Compose `research-schema-check` does not migrate. It runs as the pinned
PostgreSQL image's non-root UID with all capabilities dropped, a read-only root,
and only the administrator secret. It fails closed on migration-ledger,
exact normalized PK/unique/CHECK definition, required column type/nullability/
identity/default, exact valid/ready index, RLS/policy, append-only trigger,
role-attribute/membership, or exact grant drift. `research-raw-init` separately
runs without network or secrets and recursively prepares existing directories
and regular files on the Raw filesystem for UID/GID `10001:10001`. It does not
follow symlinks or cross filesystems; directories are `0750`, immutable evidence
is read-only to the worker, and `manifest.jsonl`/`commit.lock` remain writable.
Only after PostgreSQL is healthy and both one-shots succeed may Compose start
the unprivileged worker. If a host CLI writes new Raw content after this
ownership transfer, rerun the init one-shot or use an operator-approved shared
ownership workflow before restarting the worker.

Compose deliberately keeps the host bind `${LAGRANGE_DATA_DIR}/raw:/data/raw`
while setting `RESEARCH_RAW_ROOT=/data`, because `RawStore` itself appends the
`raw` component. Evidence is therefore directly under
`${LAGRANGE_DATA_DIR}/raw/provider=...` and the manifest under
`${LAGRANGE_DATA_DIR}/raw/manifests/...`; `/raw/raw` is always a configuration
error.

Compose mounts `db_research_password` and `krx_api_key` from external files at
`/run/secrets/...`; real secret files must remain untracked. See
`deploy/secrets/README.md` for role and secret provisioning. Start or repair the
service with:

```sh
docker compose -f deploy/compose/compose.yml up -d research-worker
```

Run the full static and functional Compose contract smoke test with:

```powershell
pwsh -NoProfile -File scripts/qa/research-worker-smoke.ps1
```

The smoke creates an isolated Compose project, proves the gate fails before
migrations and under same-name CHECK, dropped-column, index, policy, trigger,
and overprivilege mutations, applies migrations with the finite external lock
timeout, and restores a passing gate. A named-volume Linux probe also starts
with nested host-owned Raw, proves UID 10001 can read evidence and append/lock
the manifest after init, and proves an outside symlink target was untouched.
It then creates Raw with manual `collectors ingest-krx --root <data>`, proves the
direct host path (never `raw/raw`), and proves worker startup recovery publishes
that same manifest before any fetch. It verifies health, all four equal non-null
source batch IDs, and idempotent replay, then removes containers, volumes, local
images, and temporary secret files in a `finally` block. Recovery itself is a
worker startup/one-shot/daemon responsibility; there is no separate public
manual recovery subcommand.

## Manual Raw QA

`pwsh data-pipelines/collectors/qa/ingest-twice.ps1` ingests the recorded bundle twice into a scratch
root, asserts two batches with identical hashes and an untouched first batch,
then exercises traversal, malformed, timeout, and credentialed failure modes.
It requires `cargo` on `PATH`.
