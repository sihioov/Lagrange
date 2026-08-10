# Research Worker Metadata Pipeline Design

**Date:** 2026-08-10  
**Status:** Draft for written review  
**Scope:** Synthetic KRX fixture ingestion through immutable Raw storage, PostgreSQL metadata publication, scheduled worker execution, and Risk Gateway consumption

## 1. Goal

Complete the repository-owned part of the KRX operational data path without
pretending that licensed connectivity exists. A synthetic delivery must travel
through the same verified Raw and PostgreSQL publication path that a licensed
delivery will later use. Production must reject synthetic mode and remain
fail-closed until a documented KRX transport and credentials are supplied.

The completed first phase has this data flow:

```text
EodProvider
  -> structural validation
  -> immutable Raw batch + append-only manifest
  -> stored-byte hash verification
  -> one PostgreSQL publication transaction
       -> four data_batches rows
       -> append-only calendar version rows
       -> current trading_calendars projection
  -> Risk Gateway market-session and freshness reads
```

## 2. Scope

### Included

- A PostgreSQL sink for all four response kinds in one Raw delivery: bars,
  reference, calendar, and corporate actions.
- Stable Raw-batch lineage and replay-safe uniqueness in `data_batches`.
- Append-only calendar-version history plus a current projection for the Risk
  Gateway.
- A real Rust `research-worker` process with deterministic one-shot execution,
  daemon scheduling, retry, and startup recovery.
- A hard environment fence that forbids synthetic mode in production.
- A dedicated least-privilege `research_writer` database role.
- Replacement of the Compose sleep placeholder and process-only healthcheck.
- Synthetic end-to-end QA from fixture bytes through Risk Gateway inputs.
- Normalization of Compose database role names to the actual repository role
  contract (`app`, `worker`, `audit_writer`, and `research_writer`).

### Excluded

- The licensed KRX HTTP transport, authentication scheme, pagination, and rate
  limiting. Those require the official contract and endpoint documentation.
- Real KRX credentials or secret values.
- KIS live-order execution and X1/X2 release evidence.
- New curation, factor, or Nautilus catalog behavior.
- Treating synthetic data as evidence that an operational KRX feed is live.

## 3. Architecture

The existing `market-data` crate remains independent of PostgreSQL. It owns the
provider-neutral envelopes, validation, immutable Raw storage, manifest format,
and typed conversion of verified provider bytes into a publication model. It
must not acquire a SQLx dependency.

The `collectors` package gains a reusable orchestration library and PostgreSQL
sink. Its existing manual CLI and the new daemon binary call the same
`ingest_and_publish` pipeline. There is one deployed worker process, not a
separate manifest-tail service.

The new `research-worker` binary supports:

- `--once --date YYYY-MM-DD` for deterministic QA and operator recovery;
- daemon mode that runs at `RESEARCH_RUN_AT_KST` (default `16:30`) and uses
  the current Asia/Seoul civil date as the target without inferring whether it
  is a trading session;
- startup recovery that publishes verified Raw batches which have no complete
  DB publication before considering a new fetch;
- a health command/probe that checks DB reachability and the most recent
  successful publication rather than merely checking that a process exists.
  Publication age is compared with `RESEARCH_MAX_PUBLICATION_AGE_SECS`
  (default four days, configurable for the operating calendar).

The worker reads database and provider secrets only through `_FILE` inputs.
Neither command lines, logs, Raw request metadata, nor database rows may contain
credential values.

## 4. PostgreSQL Model

### 4.1 Raw batch lineage

`data_batches` receives nullable lineage columns for compatibility with legacy
rows:

- `source_batch_id uuid`;
- `source_file_name text`;
- `fetch_mode text`, constrained to `synthetic` or `credentialed` when present.

New sink writes always populate all three columns. A partial unique index on
`(provider, market, source_batch_id, source_file_name)` where
`source_batch_id IS NOT NULL` makes exact publication replay a no-op without
inventing lineage for old rows.

Each verified Raw file creates one row with this mapping:

| Raw response kind | `data_batches.kind` |
|---|---|
| `bars` with at least one row for the requested target date | `EOD` |
| `bars` without a row for the requested target date | `EOD_UNAVAILABLE` |
| `reference` | `REFERENCE` |
| `calendar` | `CALENDAR` |
| `corporate_actions` | `CORPORATE_ACTIONS` |

The stored hash is the manifest SHA-256 hex without a display prefix. The
storage path names the exact immutable batch file, not only its parent date
partition.

The `EOD_UNAVAILABLE` distinction is safety-critical: an empty holiday or
incomplete response must not refresh the `EOD` timestamp consumed by the Risk
Gateway. It is still published as Raw metadata for audit, but the freshness
query ignores it.

### 4.2 Calendar history and projection

A new `trading_calendar_versions` table stores append-only version rows. Each
row carries exchange, session date, session type, timezone, source,
source-version identity, Raw batch id, content hash, and retrieval timestamp.
Its uniqueness includes the source version and session date, so a corrected
calendar is a new publication rather than an in-place mutation.

The existing `trading_calendars` table remains the current projection read by
the Risk Gateway. It gains nullable lineage columns for Raw batch id, content
hash, and retrieval timestamp. A newer publication updates this projection in
the same transaction that inserts its immutable history. An older or replayed
publication cannot replace a projection with a later retrieval timestamp. An
equal timestamp is accepted only when its projected facts and content hash are
identical; an equal timestamp with different facts is an integrity failure.

Provider sessions publish as `TRADING`. Explicit provider holidays publish as
`CLOSED`. Dates absent from both lists remain absent and therefore remain
`MarketSession::Unknown`; the sink does not infer sessions or closures from
weekdays. The Risk Gateway maps `CLOSED` to `MarketSession::Closed`.

### 4.3 Publication transaction

One transaction covers all four `data_batches` rows, calendar history, and the
current projection. Exact replay first verifies that existing lineage, hash,
path, kind, and file size match. Matching rows are accepted as idempotent;
conflicting rows for the same Raw identity abort the entire transaction as an
integrity failure.

Legacy rows without Raw lineage remain readable but are never guessed or
backfilled from path or hash similarities.

## 5. Calendar Parsing Contract

Calendar publication uses a typed response DTO. It requires a calendar id,
schema version, source, `Asia/Seoul` timezone, session-local open and close
times, explicit sessions, and an explicit holidays array. Session and holiday
dates must parse as civil dates. Open and close timestamps must agree with the
declared local session times.

The parser rejects:

- unsupported timezone or malformed timestamps;
- a date present as both a session and a holiday;
- duplicate rows whose facts disagree;
- a session whose UTC instants disagree with 09:00-15:30 Asia/Seoul;
- missing provenance required to identify a calendar version.

Parsing occurs only after the stored file bytes have been read back and their
hash has been verified against the Raw manifest.

## 6. Failure and Recovery Semantics

Raw storage is committed before DB publication. This order ensures the DB
never points at bytes that were not durably written.

- Provider, validation, or Raw-store failure writes no DB publication.
- DB connection, timeout, or serialization failure is retryable. The verified
  Raw batch remains and is retried by its original `source_batch_id`. Daemon
  retries use exponential backoff from ten seconds to a ten-minute cap.
- Hash mismatch, invalid calendar facts, or conflicting replay identity is a
  permanent integrity failure. The batch is retained for investigation and is
  not partially published.
- Worker startup scans manifest entries for incomplete publication and retries
  them before fetching another delivery.
- A target date with a complete published EOD batch is not fetched again by
  the scheduler. Operator-directed replay may republish the same batch but may
  not silently create a replacement identity.
- No automatic path deletes Raw, calendar history, or published metadata.

Structured logs include provider, market, target date, batch id, phase, and
typed error class. They exclude response payloads, account data, and secret
values.

## 7. Runtime and Security

Synthetic mode is allowed only when the environment is explicitly
`development` or `qa`. `LAGRANGE_ENV=production` combined with synthetic mode
is a startup error before any Raw or DB write.

The worker uses a dedicated `research_writer` role. It receives only the
privileges needed to select and insert `data_batches`, insert immutable calendar
history, and select/insert/update the current calendar projection. It receives
no tenant-table, audit-log, account, order, or schema-creation privilege.

Role creation is added to the test and deployment bootstrap contract; grants
remain in migrations. Compose uses a separate `db_research_password` secret and
the actual role name. Existing Compose DB role spellings are aligned with the
repository's role contract without embedding passwords.

The Compose research-worker command executes the Rust binary instead of the
sleep placeholder. Its healthcheck distinguishes process liveness from
readiness: a live process with an unreachable DB or overdue publication is not
healthy.

## 8. Test Strategy

Implementation follows test-driven development.

### Parser and mapping tests

- Map all four response kinds to stable DB kinds.
- Keep an empty or target-date-missing bars response out of the `EOD`
  freshness source by mapping it to `EOD_UNAVAILABLE`.
- Parse the canonical synthetic calendar.
- Reject timezone, timestamp, provenance, duplicate, and session/holiday
  contradictions.
- Reject altered stored bytes whose hash no longer matches the manifest.

### Migration and privilege tests

- Assert lineage columns, checks, and partial uniqueness.
- Assert calendar history cannot be updated or deleted.
- Assert `research_writer` can perform only its publication duties.
- Assert `app`, `worker`, and `audit_writer` cannot publish shared market data.

### PostgreSQL integration tests

- Publish one delivery into four batch rows plus calendar history/projection.
- Replay the same batch without duplicates.
- Roll back all rows on a conflicting replay.
- Preserve a newer projection when an older calendar publication is replayed.
- Recover a Raw batch after an injected DB publication failure.

### Worker and seam tests

- Run `--once --date` through the synthetic fixture end to end.
- Refuse synthetic mode in production before side effects.
- Avoid re-fetching an already published target date.
- Recover unpublished manifest entries before a new fetch.
- Build a Risk Gateway snapshot from worker-published data and observe
  `Open`, `Closed`, and `DataFreshness::Age` results.
- Verify Compose contains the real command, role/secret, writable Raw mount,
  and functional healthcheck.

## 9. Completion Criteria

The first phase is complete when:

1. the canonical synthetic bundle flows through immutable Raw, PostgreSQL, and
   the Risk Gateway without manual SQL;
2. exact retries are idempotent and conflicting retries fail atomically;
3. restart recovery publishes previously stored Raw before new collection;
4. production rejects synthetic mode and missing real credentials remain
   fail-closed;
5. the Compose research-worker no longer sleeps as a placeholder;
6. focused tests, clippy, the workspace test suite, and the QA smoke test pass;
7. documentation states clearly that licensed KRX HTTP connectivity remains a
   second-phase external integration.

Passing this phase proves the repository-owned pipeline and sink. It does not
claim that KRX credentials, licensed transport, or fresh operational market
data exist.
