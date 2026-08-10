# Research Worker Metadata Publication Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Publish synthetic KRX collector bundles from the immutable Raw store into PostgreSQL, recover interrupted publications idempotently, run the flow as a scheduled `research-worker`, and make the published EOD/calendar metadata immediately usable by the Risk Gateway.

**Architecture:** Keep collection and Raw persistence as the first durable boundary. Convert verified Raw manifests into typed publication records, then write batch metadata, append-only calendar history, and the current calendar projection in one PostgreSQL transaction keyed by the Raw batch ID. A single Rust `research-worker` binary owns one-shot execution, startup recovery, scheduling, retry classification, and health checks; the existing collector CLI gains a manual publication path backed by the same pipeline. Licensed KRX transport remains a later phase, while production rejects synthetic mode before any side effect.

**Tech Stack:** Rust 1.97, Tokio, SQLx/PostgreSQL, chrono/chrono-tz, serde/serde_json, uuid, Docker Compose, PowerShell and POSIX shell QA scripts.

---

## Scope and invariants

- First-phase input is the existing deterministic synthetic KRX fixture bundle. Licensed HTTP transport is explicitly out of scope.
- `APP_ENV=production` plus synthetic fetch mode must fail before provider construction, Raw writes, or database connection.
- Raw objects and manifest entries are immutable. Publication reads every object back through `RawStore`, so existing size/hash verification remains mandatory.
- A Raw batch is the idempotency key. Replaying identical content succeeds without duplicate rows; replaying the same key with different facts is a permanent integrity error.
- All four response kinds create `data_batches` rows. Bars containing the target date publish `EOD`; bars without it publish `EOD_UNAVAILABLE`, which must not refresh Risk Gateway data freshness.
- Calendar corrections are append-only in `trading_calendar_versions`; `trading_calendars` remains the current projection and advances only to a strictly newer `retrieved_at` value.
- Missing calendar dates remain unknown. Explicit holidays publish `CLOSED`. Never infer a closed day from absence alone.
- Database/unavailable-I/O failures are retryable. Invalid Raw hashes, malformed calendar facts, and idempotency conflicts are permanent and must stop that batch without deleting evidence.

## File map

- Create: `migrations/0022_research_publication.up.sql` — source identity columns, calendar history, projection provenance, grants, RLS, append-only enforcement.
- Create: `migrations/0022_research_publication.down.sql` — exact rollback of migration 0022.
- Modify: `tests/integration/migration-contract/bootstrap.sql` — create the `research_writer` test role before migrations run.
- Modify: `tests/integration/migration-contract/tests/migration_contract.rs` — migration, idempotency, append-only, grant, and RLS contracts.
- Modify: `crates/api-server/tests/common/mod.rs` — add the role required by migration 0022.
- Modify: `crates/api-server/tests/tenancy_rls.rs` — add the role required by migration 0022.
- Modify: `crates/job-queue/tests/common/mod.rs` — add the role required by migration 0022.
- Modify: `crates/job-queue/tests/queue_contract.rs` — add the role required by migration 0022.
- Modify: `crates/result-model/tests/manifest_db.rs` — add the role required by migration 0022.
- Modify: `crates/result-model/tests/robustness_harness.rs` — add the role required by migration 0022.
- Create: `crates/market-data/src/publication.rs` — verified Raw-to-publication conversion types and calendar parsing.
- Modify: `crates/market-data/src/lib.rs` — export the publication module.
- Create: `crates/market-data/tests/publication.rs` — mapping and validation tests.
- Modify: `data-pipelines/collectors/Cargo.toml` — library/binary dependencies and test dependencies.
- Create: `data-pipelines/collectors/src/lib.rs` — public collector pipeline surface.
- Create: `data-pipelines/collectors/src/sink.rs` — PostgreSQL transaction and idempotency implementation.
- Create: `data-pipelines/collectors/src/pipeline.rs` — collect/publish/recover orchestration and error classes.
- Create: `data-pipelines/collectors/src/worker.rs` — configuration, production fence, scheduling, retry, and health logic.
- Modify: `data-pipelines/collectors/src/main.rs` — retain Raw-only ingestion and add a manual `ingest-and-publish-krx` command using the shared pipeline.
- Create: `data-pipelines/collectors/src/bin/research-worker.rs` — `--once`, daemon, and `healthcheck` entry point.
- Create: `data-pipelines/collectors/tests/common/mod.rs` — isolated PostgreSQL test schema and role helpers.
- Create: `data-pipelines/collectors/tests/publication_sink.rs` — atomicity, idempotency, correction, and privilege integration tests.
- Create: `data-pipelines/collectors/tests/research_worker.rs` — production fence, startup recovery, one-shot, retry, and scheduling tests.
- Modify: `crates/api-server/src/risk_snapshot.rs` — accept explicit `CLOSED` projection rows.
- Modify: `crates/api-server/Cargo.toml` — add the collector crate as a dev dependency for the end-to-end seam test.
- Modify: `crates/api-server/tests/risk_snapshot_seam.rs` — prove published EOD/calendar rows drive Risk decisions and unavailable bars do not refresh freshness.
- Create: `data-pipelines/collectors/Dockerfile` — pinned Rust builder and Alpine runtime image for `research-worker`.
- Modify: `deploy/compose/compose.yml` — replace the sleep placeholder, use the dedicated writer role, normalize role names, and add a functional health check.
- Create: `deploy/secrets/db_research_password.example` — example secret file without a real credential.
- Modify: `deploy/secrets/README.md` — document the new credential and least-privilege role.
- Create: `scripts/qa/research-worker-smoke.ps1` — Windows static/configuration and functional smoke test.
- Create: `scripts/qa/research-worker-smoke.sh` — POSIX equivalent.
- Modify: `data-pipelines/collectors/README.md` — operator commands, recovery behavior, production fence, and licensed-transport boundary.
- Modify: `docs/STATUS.md` — mark the synthetic metadata publication path complete and retain the external licensed-feed blocker.

## Task 1: Add the research publication database contract

**Files:**

- Create: `migrations/0022_research_publication.up.sql`
- Create: `migrations/0022_research_publication.down.sql`
- Modify: `tests/integration/migration-contract/bootstrap.sql`
- Modify: `tests/integration/migration-contract/tests/migration_contract.rs`
- Modify: `crates/api-server/tests/common/mod.rs`
- Modify: `crates/api-server/tests/tenancy_rls.rs`
- Modify: `crates/job-queue/tests/common/mod.rs`
- Modify: `crates/job-queue/tests/queue_contract.rs`
- Modify: `crates/result-model/tests/manifest_db.rs`
- Modify: `crates/result-model/tests/robustness_harness.rs`

- [ ] Add `research_writer` to every test bootstrap that currently creates `admin`, `app`, `worker`, or `audit_writer`. Keep role creation idempotent so the new migration can grant privileges in every workspace test harness.
- [ ] Write failing migration-contract assertions for these nullable provenance columns on `data_batches`: `source_batch_id uuid`, `source_file_name text`, and `fetch_mode text`.
- [ ] Assert a partial unique index equivalent to `(provider, market, source_batch_id, source_file_name) WHERE source_batch_id IS NOT NULL`, a lowercase `fetch_mode IN ('synthetic', 'credentialed')` constraint, and an all-null-or-all-present provenance constraint. Verify legacy inserts with all three provenance fields null still work.
- [ ] Write failing assertions for `trading_calendar_versions` with append-only source facts and for nullable `source_batch_id`, `content_sha256`, and `retrieved_at` columns on `trading_calendars`. Require the three projection provenance fields to be either all null for legacy rows or all present for published rows.
- [ ] Add behavior tests that attempt `UPDATE` and `DELETE` against `trading_calendar_versions` and expect SQLSTATE `55000` from an append-only trigger.
- [ ] Add role tests showing `research_writer` can `SELECT`/`INSERT` `data_batches`, `SELECT`/`INSERT` `trading_calendar_versions`, and `SELECT`/`INSERT`/`UPDATE` `trading_calendars`, but cannot delete those rows or access tenant orders, jobs, audit tables, or schema DDL.
- [ ] Run the migration contract and confirm RED because migration 0022 and its objects do not exist:

  ```powershell
  cargo test --test migration_contract --manifest-path tests/integration/migration-contract/Cargo.toml -- --nocapture
  ```

  Expected: at least one new assertion fails on a missing column/table/privilege.

- [ ] Implement `0022_research_publication.up.sql`. Use the following externally visible contract:

  ```sql
  ALTER TABLE data_batches
      ADD COLUMN source_batch_id uuid,
      ADD COLUMN source_file_name text,
      ADD COLUMN fetch_mode text,
      ADD CONSTRAINT data_batches_fetch_mode_check
          CHECK (fetch_mode IS NULL OR fetch_mode IN ('synthetic', 'credentialed')),
      ADD CONSTRAINT data_batches_source_provenance_check CHECK (
          (source_batch_id IS NULL AND source_file_name IS NULL AND fetch_mode IS NULL)
          OR
          (source_batch_id IS NOT NULL AND source_file_name IS NOT NULL AND fetch_mode IS NOT NULL)
      );

  CREATE UNIQUE INDEX data_batches_source_file_uq
      ON data_batches (provider, market, source_batch_id, source_file_name)
      WHERE source_batch_id IS NOT NULL;

  CREATE TABLE trading_calendar_versions (
      id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
      exchange text NOT NULL,
      session_date date NOT NULL,
      session_type text NOT NULL CHECK (session_type IN ('TRADING', 'CLOSED')),
      timezone text NOT NULL CHECK (timezone = 'Asia/Seoul'),
      source text NOT NULL,
      source_version text NOT NULL,
      source_batch_id uuid NOT NULL,
      content_sha256 text NOT NULL CHECK (content_sha256 ~ '^[0-9a-f]{64}$'),
      retrieved_at timestamptz NOT NULL,
      created_at timestamptz NOT NULL DEFAULT now(),
      UNIQUE (exchange, session_date, source_version)
  );

  ALTER TABLE trading_calendars
      ADD COLUMN source_batch_id uuid,
      ADD COLUMN content_sha256 text,
      ADD COLUMN retrieved_at timestamptz,
      ADD CONSTRAINT trading_calendars_content_sha256_check
          CHECK (content_sha256 IS NULL OR content_sha256 ~ '^[0-9a-f]{64}$'),
      ADD CONSTRAINT trading_calendars_source_provenance_check CHECK (
          (source_batch_id IS NULL AND content_sha256 IS NULL AND retrieved_at IS NULL)
          OR
          (source_batch_id IS NOT NULL AND content_sha256 IS NOT NULL AND retrieved_at IS NOT NULL)
      );
  ```

- [ ] Add an append-only trigger rejecting `UPDATE` and `DELETE`, enable RLS on the history table, grant the existing read roles `SELECT`, and grant only the required DML to `research_writer`. Add `research_writer` policies on the two existing RLS tables and on the new history table; privileges remain the outer boundary.
- [ ] Implement the down migration in reverse dependency order: policies/grants, trigger/function, table/index, then projection and batch columns. Do not drop the externally created role.
- [ ] Run the migration contract again and confirm GREEN, including up/down/up migration coverage.
- [ ] Run all crates that own duplicated database bootstraps to catch a missing role declaration:

  ```powershell
  cargo test -p api-server -p job-queue -p result-model --no-fail-fast
  ```

  Expected: all tests pass; no migration fails with `role "research_writer" does not exist`.

- [ ] Run `git diff --check` and commit:

  ```powershell
  git add migrations/0022_research_publication.* tests/integration/migration-contract crates/api-server/tests crates/job-queue/tests crates/result-model/tests
  git commit -m "feat(db): add research publication contract"
  ```

## Task 2: Convert verified Raw bundles into typed publication records

**Files:**

- Create: `crates/market-data/src/publication.rs`
- Modify: `crates/market-data/src/lib.rs`
- Create: `crates/market-data/tests/publication.rs`

- [ ] Write failing unit tests that build a synthetic bundle, reopen its manifest, and call a new `PublicationBundle::from_raw(&RawStore, &ManifestEntry)` API.
- [ ] Assert one publication record per manifest file with the exact mappings below:

  ```rust
  ResponseKind::Bars             => DataBatchKind::Eod, // target-date row exists
  ResponseKind::Bars             => DataBatchKind::EodUnavailable, // target-date row absent
  ResponseKind::Reference        => DataBatchKind::Reference,
  ResponseKind::Calendar         => DataBatchKind::Calendar,
  ResponseKind::CorporateActions => DataBatchKind::CorporateActions,
  ```

- [ ] Assert database hashes are lowercase 64-character hex strings with the Raw `sha256:` prefix removed, while the original manifest is never mutated.
- [ ] Add calendar fixture tests for `TRADING` sessions and explicit `CLOSED` holidays, including the sparse contract fixture and the richer dated fixture. Assert the stable source-version identity is derived from both `calendar_id` and `schema_version`.
- [ ] Add failure tests for a tampered object, a size mismatch, malformed JSON, an unsupported calendar timezone, session UTC timestamps inconsistent with 09:00–15:30 Asia/Seoul, duplicate contradictory facts for one date, and a target-date bars payload whose date field is invalid.
- [ ] Run and confirm RED because the publication API does not exist:

  ```powershell
  cargo test -p market-data --test publication -- --nocapture
  ```

- [ ] Implement explicit, transport-neutral types rather than passing JSON into the sink:

  ```rust
  pub struct PublicationBundle {
      pub source_batch_id: BatchId,
      pub provider: String,
      pub market: String,
      pub target_date: TradingDate,
      pub retrieved_at: UtcTimestamp,
      pub fetch_mode: FetchMode,
      pub files: Vec<PublicationFile>,
      pub calendar_facts: Vec<CalendarFact>,
  }

  pub struct PublicationFile {
      pub file_name: String,
      pub kind: DataBatchKind,
      pub content_sha256: String,
      pub storage_path: String,
      pub bytes_size: u64,
  }

  pub struct CalendarFact {
      pub exchange: String,
      pub session_date: TradingDate,
      pub session_type: CalendarSessionType,
      pub timezone: String,
      pub source: String,
      pub source_version: String,
  }
  ```

- [ ] Make `PublicationBundle::from_raw` call `RawStore::read_batch_bytes` for every manifest entry before parsing anything. Model conversion failures with a dedicated typed error so invalid evidence can later be classified as permanent.
- [ ] Parse only documented calendar fields, require `Asia/Seoul`, construct `source_version` as `<calendar_id>:schema-<schema_version>`, verify session UTC instants agree with the declared 09:00–15:30 local times, deduplicate identical facts, and reject contradictory session types for the same exchange/date/source version.
- [ ] Build `storage_path` from the exact immutable batch directory plus file name, and take `bytes_size` from the verified manifest entry. Reject values that cannot be represented by the PostgreSQL `bigint` contract.
- [ ] Determine EOD availability from a parsed bars row whose date equals `manifest.date`; do not use file existence or an empty response as freshness evidence.
- [ ] Export the module from `market-data`, run the focused test until GREEN, then run the entire crate:

  ```powershell
  cargo test -p market-data
  cargo clippy -p market-data --all-targets --all-features -- -D warnings
  ```

- [ ] Commit:

  ```powershell
  git add crates/market-data
  git commit -m "feat(market-data): derive publication records from raw bundles"
  ```

## Task 3: Implement the atomic PostgreSQL publication sink

**Files:**

- Modify: `data-pipelines/collectors/Cargo.toml`
- Create: `data-pipelines/collectors/src/lib.rs`
- Create: `data-pipelines/collectors/src/sink.rs`
- Create: `data-pipelines/collectors/tests/common/mod.rs`
- Create: `data-pipelines/collectors/tests/publication_sink.rs`

- [ ] Add `async-trait 0.1`, `tokio 1`, `sqlx 0.9` with PostgreSQL/runtime/chrono/uuid features, `chrono 0.4`, `chrono-tz 0.10`, `thiserror 2.0`, and `uuid 1` dependencies without wildcard versions. Add the existing migration-test utilities only as dev dependencies.
- [ ] Write a failing integration test that publishes one synthetic bundle through a `PostgresPublicationSink`, then asserts four `data_batches` rows (one per response file, with bars classified as `EOD`), calendar version rows, and the current projection.
- [ ] Write replay tests: an identical batch returns `AlreadyPublished` with unchanged row counts; the same source batch/file key with a different kind/hash returns `PublicationError::Conflict` and leaves every table unchanged.
- [ ] Write correction tests using a new source-version identity: a later calendar `retrieved_at` appends history and updates the projection; an older correction appends history but does not replace the current row; an equal timestamp with identical facts is idempotent; an equal timestamp with different facts rolls back as a conflict. Reusing one source version with different content is also a permanent conflict.
- [ ] Write an injected mid-transaction failure test and assert no `data_batches`, version, or projection row survives.
- [ ] Connect as `research_writer` for the happy path, then explicitly prove attempts to read orders or delete publication evidence fail.
- [ ] Run and confirm RED because the sink does not exist:

  ```powershell
  cargo test -p collectors --test publication_sink -- --nocapture
  ```

- [ ] Implement a transaction-scoped sink API:

  ```rust
  pub enum PublishOutcome {
      Published,
      AlreadyPublished,
  }

  #[async_trait]
  pub trait PublicationSink {
      async fn publication_state(&self, batch_id: BatchId) -> Result<PublicationState, PublicationError>;
      async fn publish(&self, bundle: &PublicationBundle) -> Result<PublishOutcome, PublicationError>;
      async fn has_eod(&self, date: TradingDate) -> Result<bool, PublicationError>;
  }
  ```

- [ ] At transaction start, load every existing row for the source batch. Accept only a complete exact match; treat a partial set or differing fact as `Conflict`. Never silently fill a partially published batch because the transaction invariant says partial publication cannot be legitimate.
- [ ] Insert each `data_batches` row with provider/market/date/kind/hash/exact storage path/file size/retrieved time plus source provenance. Use the lowercase database representation `synthetic` or `credentialed` consistently with `FetchMode`.
- [ ] Insert history facts using the source file hash and batch ID. On a uniqueness hit, query the existing fact and accept it only when all semantic fields match.
- [ ] Upsert the current projection with an explicit `retrieved_at` comparison. Lock the current row before deciding so concurrent replays cannot violate newer-wins behavior.
- [ ] Classify SQL connectivity, serialization, and pool errors as retryable; unique conflicts with mismatched content and invariant violations as permanent.
- [ ] Run focused tests until GREEN, then:

  ```powershell
  cargo test -p collectors --test publication_sink
  cargo clippy -p collectors --all-targets --all-features -- -D warnings
  ```

- [ ] Commit:

  ```powershell
  git add data-pipelines/collectors/Cargo.toml data-pipelines/collectors/src/lib.rs data-pipelines/collectors/src/sink.rs data-pipelines/collectors/tests
  git commit -m "feat(collectors): publish raw metadata atomically"
  ```

## Task 4: Build the shared collect, publish, and recovery pipeline

**Files:**

- Create: `data-pipelines/collectors/src/pipeline.rs`
- Modify: `data-pipelines/collectors/src/lib.rs`
- Modify: `data-pipelines/collectors/src/main.rs`
- Modify: `data-pipelines/collectors/tests/research_worker.rs`

- [ ] Write a failing test for `ingest_and_publish`: it must call the existing `market_data::ingest_bundle`, reopen and verify the durable Raw batch, derive a `PublicationBundle`, and publish it through a fake sink.
- [ ] Add failure-point tests proving: a Raw failure never calls the sink; a DB failure leaves a readable Raw manifest; replay after the DB recovers publishes the same batch ID; a permanent conversion conflict is returned without retry classification.
- [ ] Write startup recovery tests with several manifests: already published batches are skipped, unpublished batches are published oldest-first, and a permanently invalid batch stops with its path/batch ID in the error.
- [ ] Run and confirm RED:

  ```powershell
  cargo test -p collectors --test research_worker pipeline_ -- --nocapture
  ```

- [ ] Implement the shared sequence with no database side effect before Raw durability:

  ```rust
  pub async fn ingest_and_publish(
      request: &IngestionRequest,
      store: &RawStore,
      sink: &dyn PublicationSink,
  ) -> Result<RunOutcome, PipelineError> {
      let manifest = ingest_bundle(request, store)?;
      let bundle = PublicationBundle::from_raw(store, &manifest)?;
      let published = sink.publish(&bundle).await?;
      Ok(RunOutcome { manifest, published })
  }
  ```

- [ ] Implement `recover_unpublished` with the existing `RawStore::read_manifest(provider, market)` API. Sort by `retrieved_at`, ask the sink for batch state, and process only `Missing`. Treat `Partial` as a permanent integrity failure.
- [ ] Keep the existing `ingest-krx` Raw-only command for backwards-compatible fixture QA. Add `ingest-and-publish-krx` with the same date/mode/options plus database configuration; it must call the shared pipeline rather than duplicate SQL.
- [ ] Ensure errors print a stable class (`retryable` or `permanent`), batch ID when available, and a causal message without secret values.
- [ ] Run focused tests and the existing Raw-ingest QA test to GREEN:

  ```powershell
  cargo test -p collectors --test research_worker
  cargo test -p collectors
  ```

- [ ] Commit:

  ```powershell
  git add data-pipelines/collectors/src data-pipelines/collectors/tests/research_worker.rs
  git commit -m "feat(collectors): add recoverable publication pipeline"
  ```

## Task 5: Add the one-shot and scheduled research worker

**Files:**

- Create: `data-pipelines/collectors/src/worker.rs`
- Create: `data-pipelines/collectors/src/bin/research-worker.rs`
- Modify: `data-pipelines/collectors/src/lib.rs`
- Modify: `data-pipelines/collectors/tests/research_worker.rs`

- [ ] Write configuration tests for `APP_ENV`, `RESEARCH_FETCH_MODE`, `RESEARCH_RUN_AT_KST`, `RESEARCH_MAX_PUBLICATION_AGE_SECS`, Raw root, and database settings. Test `_FILE` trimming and missing/unreadable/empty secret files without logging their contents.
- [ ] Write the mandatory production fence test with spies for provider, Raw store, and pool construction. `APP_ENV=production` plus `RESEARCH_FETCH_MODE=synthetic` must return `SyntheticForbidden` while every spy remains untouched.
- [ ] Write `--once --date 2020-01-31` tests showing startup recovery runs first, an existing target-date EOD suppresses a duplicate scheduled collection, and `EOD_UNAVAILABLE` does not count as an existing EOD.
- [ ] Use an injected clock/sleeper to test daemon scheduling at the default `16:30` Asia/Seoul time, an overridden valid time, invalid time rejection, and exponential retry delays `10, 20, 40, ... 600` seconds.
- [ ] Assert permanent failures are not retried and retry counters reset after a successful cycle.
- [ ] Write health tests requiring a database round trip and newest KRX/KR `EOD` publication no older than `RESEARCH_MAX_PUBLICATION_AGE_SECS` (default `345600`, four days). No EOD row, stale EOD, or database failure must be unhealthy; `EOD_UNAVAILABLE` must be ignored.
- [ ] Run and confirm RED:

  ```powershell
  cargo test -p collectors --test research_worker worker_ -- --nocapture
  ```

- [ ] Implement configuration validation as a pure first step. Call `validate_synthetic_policy` before constructing provider/store/pool objects:

  ```rust
  if config.app_env.eq_ignore_ascii_case("production")
      && config.fetch_mode == FetchMode::Synthetic
  {
      return Err(WorkerError::SyntheticForbidden);
  }
  ```

- [ ] Implement three binary modes: `research-worker --once --date YYYY-MM-DD`, default daemon mode, and `research-worker healthcheck`. The daemon uses the current KST civil date only as the collection target; it never derives market-open/closed status itself.
- [ ] Read the deployed password exclusively from `DB_PASSWORD_FILE`; construct the PostgreSQL connect options from `DB_HOST`, `DB_PORT`, `DB_NAME`, and `DB_USER`. Permit `DATABASE_URL` only in test/manual CLI code paths, not the Compose worker configuration.
- [ ] On every process start run recovery before the scheduled/current batch. Retry only `PipelineError::Retryable`, cap backoff at ten minutes, and keep retrying until success or shutdown.
- [ ] Run focused tests, CLI help/argument tests, and clippy:

  ```powershell
  cargo test -p collectors
  cargo run -p collectors --bin research-worker -- --help
  cargo clippy -p collectors --all-targets --all-features -- -D warnings
  ```

- [ ] Commit:

  ```powershell
  git add data-pipelines/collectors/src data-pipelines/collectors/tests/research_worker.rs
  git commit -m "feat(collectors): add scheduled research worker"
  ```

## Task 6: Prove the Risk Gateway consumes published metadata

**Files:**

- Modify: `crates/api-server/src/risk_snapshot.rs`
- Modify: `crates/api-server/Cargo.toml`
- Modify: `crates/api-server/tests/risk_snapshot_seam.rs`

- [ ] Add a failing unit/seam assertion that a `trading_calendars.session_type = 'CLOSED'` row maps to `MarketSession::Closed` rather than `Unknown`.
- [ ] Add a failing end-to-end seam test that publishes the synthetic fixture through `PostgresPublicationSink`, asks `risk_snapshot::for_submission` for the target date/time, and observes known calendar/freshness inputs without direct metadata SQL inserts in the test.
- [ ] Add a bars-without-target-date fixture case publishing `EOD_UNAVAILABLE`; assert the Risk freshness query remains `Unknown` or reflects only the prior true `EOD` row.
- [ ] Run and confirm RED:

  ```powershell
  cargo test -p api-server --test risk_snapshot_seam -- --nocapture
  ```

- [ ] Extend the existing session mapping exactly:

  ```rust
  match session_type.as_str() {
      "TRADING" => /* existing intraday open/closed calculation */,
      "SETTLEMENT" | "CLOSED" => MarketSession::Closed,
      _ => MarketSession::Unknown,
  }
  ```

- [ ] Keep the freshness SQL filter exactly on `kind = 'EOD'`; do not broaden it to `EOD%` or use the newest arbitrary batch.
- [ ] Run the seam and evaluator suites to GREEN:

  ```powershell
  cargo test -p api-server --test risk_snapshot_seam
  cargo test -p risk-gateway --test twelve_checks
  cargo clippy -p api-server --all-targets --all-features -- -D warnings
  ```

- [ ] Commit:

  ```powershell
  git add crates/api-server
  git commit -m "feat(risk): consume published research metadata"
  ```

## Task 7: Replace the Compose placeholder and add QA smoke coverage

**Files:**

- Create: `data-pipelines/collectors/Dockerfile`
- Modify: `deploy/compose/compose.yml`
- Create: `deploy/secrets/db_research_password.example`
- Modify: `deploy/secrets/README.md`
- Create: `scripts/qa/research-worker-smoke.ps1`
- Create: `scripts/qa/research-worker-smoke.sh`

- [ ] Write the smoke scripts first. Their static phase must fail unless the Compose service runs `research-worker`, has no `sleep` placeholder, uses `DB_USER=research_writer`, references `db_research_password`, mounts Raw read/write, mounts other data roots read-only or not at all, and invokes `research-worker healthcheck`.
- [ ] Make the smoke scripts validate that every Dockerfile `FROM` has an immutable digest and that no real secret file is tracked.
- [ ] Add a functional phase that starts PostgreSQL, applies migrations, builds/starts the research worker in QA synthetic mode, executes `--once --date 2020-01-31`, waits for healthy status, and queries for matching source-batch/data/calendar rows. Run the same one-shot twice and assert row counts do not increase.
- [ ] Run the PowerShell script and confirm RED against the current `sleep 1e9` placeholder:

  ```powershell
  pwsh -NoProfile -File scripts/qa/research-worker-smoke.ps1
  ```

- [ ] Create a multi-stage Dockerfile using the repository-approved pinned bases:

  ```dockerfile
  FROM rust:1.97.1-alpine@sha256:3c38f3f82c2f3d73da3b38e18d279393a04cb43ddded0e35088a8c3324d40900 AS builder
  # install musl build dependencies and build only the research-worker binary
  FROM alpine:3.21@sha256:48b0309ca019d89d40f670aa1bc06e426dc0931948452e8491e3d65087abc07d
  # install CA certificates, copy the binary, run as an unprivileged user,
  # and set the binary entrypoint
  ```

- [ ] Replace the Compose `research-worker` service image/command with the local pinned-base build, pass `APP_ENV`, `RESEARCH_FETCH_MODE`, schedule/max-age variables, canonical PostgreSQL host fields, `DB_USER=research_writer`, and `DB_PASSWORD_FILE=/run/secrets/db_research_password`.
- [ ] Normalize other Compose database role spellings from `lagrange_app`/`lagrange_worker` to the actual migration roles `app`/`worker`. Do not change their secret identities unless the secret content contract requires it.
- [ ] Add `db_research_password` to Compose and create only a `.example` file. Document that operators must provision the real ignored file and create/set the matching database role password outside Git.
- [ ] Make the health check call the binary, not inspect a process:

  ```yaml
  healthcheck:
    test: ["CMD", "/usr/local/bin/research-worker", "healthcheck"]
  ```

- [ ] Run both script variants where their shells are available. If Bash is unavailable on Windows, run its static syntax check in the worker image or CI-equivalent environment and record that evidence:

  ```powershell
  pwsh -NoProfile -File scripts/qa/research-worker-smoke.ps1
  bash scripts/qa/research-worker-smoke.sh
  docker compose -f deploy/compose/compose.yml config --quiet
  ```

  Expected: static checks pass, one-shot publication is idempotent, and the service becomes healthy.

- [ ] Run `git diff --check` and commit:

  ```powershell
  git add data-pipelines/collectors/Dockerfile deploy/compose/compose.yml deploy/secrets scripts/qa/research-worker-smoke.*
  git commit -m "feat(deploy): run research worker publication service"
  ```

## Task 8: Document operations and run the complete verification matrix

**Files:**

- Modify: `data-pipelines/collectors/README.md`
- Modify: `docs/STATUS.md`

- [ ] Document exact Raw-only, manual collect-and-publish, one-shot worker, daemon, recovery, and health commands. Include every environment variable and the distinction between retryable and permanent failures.
- [ ] State prominently that synthetic mode is QA/development only and is rejected in production before side effects. State that licensed KRX credential/entitlement-aware transport is not implemented by this phase.
- [ ] Document the least-privilege `research_writer` role, `_FILE` secret setup, default 16:30 KST schedule, four-day health window, and the operational meaning of `EOD_UNAVAILABLE`.
- [ ] Update `docs/STATUS.md`: mark Raw-to-PostgreSQL publication, append-only calendar correction, recovery, Compose unit, and Risk seam complete; leave licensed external KRX collection and real production credentials as remaining external/operational work.
- [ ] Run formatting and repository hygiene checks:

  ```powershell
  cargo fmt --all -- --check
  git diff --check
  git status --short
  ```

- [ ] Run focused release gates:

  ```powershell
  cargo test -p market-data
  cargo test -p collectors
  cargo test -p api-server --test risk_snapshot_seam
  cargo test --test migration_contract --manifest-path tests/integration/migration-contract/Cargo.toml
  cargo test -p risk-gateway --test twelve_checks
  cargo clippy --workspace --all-targets --all-features -- -D warnings
  ```

- [ ] Verify at least 20 GB free on the selected target disk, then run the full workspace suite without reusing the constrained repository disk:

  ```powershell
  Get-PSDrive C | Select-Object Name,Free,Used
  $env:CARGO_TARGET_DIR = 'C:\cargo-target\lagrange-full-20260810'
  cargo test --workspace --no-fail-fast
  ```

  Expected: exit code 0 with no failed test binaries. If the target directory changes, record the actual path and free-space check in the execution notes.

- [ ] Run the Compose QA smoke one final time and retain the command/output summary in the commit or handoff:

  ```powershell
  pwsh -NoProfile -File scripts/qa/research-worker-smoke.ps1
  ```

- [ ] Inspect `git diff --stat`, `git diff`, and `git status --short` for unintended files, secret material, placeholder markers, and generated artifacts.
- [ ] Commit documentation and any final verification-only corrections:

  ```powershell
  git add data-pipelines/collectors/README.md docs/STATUS.md
  git commit -m "docs(research): document metadata publication operations"
  ```

## Final acceptance checklist

- [ ] A QA synthetic bundle reaches immutable Raw storage, `data_batches`, calendar history/projection, and the Risk Gateway without manual SQL.
- [ ] Replaying an identical batch is idempotent; conflicting or partial replays fail atomically and preserve Raw evidence.
- [ ] Startup recovery publishes valid missing batches and reports permanent corrupt batches without deleting them.
- [ ] Bars lacking the target date publish `EOD_UNAVAILABLE` and cannot refresh the Risk Gateway EOD freshness check.
- [ ] Calendar history is append-only; current projection ordering is deterministic under older, equal, and newer corrections.
- [ ] Production synthetic mode is rejected before provider, filesystem, or database side effects.
- [ ] Compose contains a real Rust worker, a functional database/publication health check, canonical role names, and a dedicated `_FILE` secret.
- [ ] `research_writer` has only the intended metadata privileges and cannot read tenant/order/audit/job data or delete evidence.
- [ ] Focused tests, migration contract, clippy, Compose smoke, and `cargo test --workspace --no-fail-fast` pass on a sufficiently large target disk.
- [ ] No credential, generated Raw data, target artifact, `TODO`, `TBD`, or placeholder command is introduced.
- [ ] The remaining limitation is explicit: real licensed KRX HTTP collection still requires the provider contract, entitlement, credentials, and production endpoint integration.
