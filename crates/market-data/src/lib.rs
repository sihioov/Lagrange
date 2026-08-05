//! `market-data` - Lagrange Station market-data domain: instruments, calendars, bars, quality, curation.
//!
//! Todo 8 delivers the **provider-neutral EOD raw contract** and the **KRX
//! provider adapter** with **immutable Raw ingestion**:
//!
//! - [`contract`] - the raw response envelope (bytes, retrieval time, provider
//!   request metadata, batch id, content hash) and response-kind taxonomy.
//! - [`provider`] - the `EodProvider` trait and the `KrxProvider` adapter with a
//!   recorded-synthetic mode (CI) and an Owner-only credentialed mode.
//! - [`storage`] - the immutable raw zone (`data/raw/provider=krx/market=kr/...`)
//!   with append-only manifests.
//! - [`ingest`] - the collector pipeline: fetch -> validate -> store -> manifest.
//! - [`entitlement`] - Todo 5 gate wiring: non-ACTIVE batches are Owner-only and
//!   Member reads fail with `DATA_ENTITLEMENT_REQUIRED`.
//! - [`redact`] - secret/redaction scan for logs (never expose provider keys/data).

pub mod contract;
pub mod entitlement;
pub mod ingest;
pub mod provider;
pub mod redact;
pub mod storage;
pub mod validate;

pub use contract::{
    ALL_RESPONSE_KINDS, FetchMode, MARKET_KR, PROVIDER_KRX, RawEnvelope, RequestMetadata,
    ResponseKind, StoredFile,
};
pub use ingest::{IngestError, IngestOutcome, IngestRequest, ingest_bundle};
pub use provider::{
    CredentialRef, EodProvider, FetchRequest, KrxMode, KrxProvider, ProviderError, RecordedBundle,
};
pub use storage::{BatchSpec, FileEntry, ManifestEntry, RawStore, StoreError};
