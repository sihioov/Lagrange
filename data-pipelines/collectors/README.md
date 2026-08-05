# data-pipelines/collectors — KRX raw ingestion

Todo 8 deliverable. The collector drives the provider-neutral EOD contract and
the KRX provider adapter implemented in `crates/market-data` (package
`market-data`), persisting **immutable Raw** under `data/raw/`.

## Provider contract (provider-neutral EOD)

`crates/market-data/src/contract.rs` defines the raw response envelope:

| field | meaning |
|---|---|
| `bytes` | the provider response/file, stored byte-for-byte, never parsed |
| `retrieved_at` | UTC instant the delivery was retrieved |
| `request` | provider request metadata (endpoint, query, REDACTED headers, mode) |
| `batch_id` | the ingestion batch this response belongs to |
| `content_hash` | `sha256:<64 hex>` over `bytes` (FR-DATA-001 immutability proof) |

Four licensed response classes are covered: `bars`, `reference`, `calendar`,
`corporate_actions`. Providers implement `EodProvider` (`provider.rs`); the
`KrxProvider` adapter has two modes:

- **synthetic** — playback of recorded synthetic contract fixtures under
  `tests/fixtures/kr-etf/contract/` (CI; no network, no credentials). Failure
  modes are recorded in the bundle manifest (`"simulate": "timeout"`) or are
  real structural violations (`contract-variants/malformed-bars`).
- **credentialed** — the Owner-only licensed mode. Requires real KRX
  credentials (`KRX_CREDENTIAL_REF`, `KRX_BASE_URL`), which do not exist in
  this environment: without them every fetch fails typed with
  `CredentialsUnavailable`. Implemented but never exercised; no real KRX keys
  exist. Never scrapes undocumented endpoints — only the documented licensed
  endpoint ids declared in recorded bundles.

## Storage layout (immutable Raw)

```
data/
└── raw/
    ├── provider=krx/market=kr/date=2020-01-31/
    │   └── batch=<batch_id>/          one dir per delivery — never overwritten
    │       ├── bars-response.json     exact provider bytes (create_new)
    │       ├── reference-response.json
    │       ├── calendar-response.json
    │       ├── corporate-actions-response.json
    │       └── batch.json             the manifest row for this batch
    └── manifests/provider=krx/market=kr/manifest.jsonl   append-only, one row per batch
```

Invariants (all proven by `crates/market-data/tests/raw_store.rs` and
`tests/krx_raw_ingestion.rs`):

- identical bytes delivered twice ⇒ TWO batches, SAME content hash, first
  batch never modified;
- manifest is append-only JSONL — one row per delivery, never rewritten;
- provider file names are validated against path traversal (`..\..\evil.json`
  ⇒ typed `UnsafeFileName`, nothing written);
- failed deliveries (timeout / malformed schema / traversal / store error)
  leave NO partial batch and no manifest row;
- reads verify stored bytes against the recorded content hash (tamper
  detection).

## Entitlement wiring (Todo 5)

The governing `krx_eod_bars` entitlement's contract reference is recorded on
each manifest row (`entitlement_reference`). A batch is **Owner-only** unless
the governing entitlement is ACTIVE on the as-of date; any Member-facing read
of an Owner-only batch is denied with `DATA_ENTITLEMENT_REQUIRED`
(`tests/raw_entitlement.rs`). Owner-only development reads stay allowed in any
entitlement state.

## CLI

```
cargo run -p collectors -- ingest-krx --root data --date 2020-01-31 \
    --mode synthetic --bundle tests/fixtures/kr-etf/contract
```

- stdout: JSON outcome (batch id, hashes, manifest path); stderr: redacted log.
- `--mode credentialed`: fails typed with `CredentialsUnavailable` (exit 2)
  unless real KRX credentials exist.
- exit codes: 0 success, 1 usage, 2 typed ingest failure.
- every log line is routed through `market_data::redact::Redactor`
  (secrets, `KEY=value` credential pairs, `Bearer` tokens).

## Manual QA

`pwsh qa/ingest-twice.ps1` ingests the recorded bundle twice into a scratch
root, asserts two batches with identical hashes and an untouched first batch,
then exercises traversal / malformed / timeout / credentialed failure modes and
checks the scratch root stays clean. Requires `cargo` on PATH.
