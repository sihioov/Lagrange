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
//!
//! Todo 9 delivers the **canonical instrument master with KRX/KIS/provider
//! aliases** ([`instrument_master`]) and the **versioned KRX trading
//! calendar** ([`calendar`]):
//!
//! - canonical `InstrumentId = {symbol}.KRX` (design §6.4); a ticker change
//!   remaps the alias and appends versioned alias history — the canonical
//!   identity is never silently changed;
//! - venue/currency/price/size/lot metadata and listing/delisting status with
//!   effective dates (requirements §8.2, FR-DATA-002);
//! - Asia/Seoul calendar with explicit sessions 09:00-15:30 KST, explicit
//!   holiday data with source/version/hash provenance, last/next trading-day
//!   queries, holiday month-end handling, and timezone-aware open/close
//!   instants (FR-DATA-003/005).

pub mod calendar;
pub mod contract;
pub mod entitlement;
pub mod ingest;
pub mod instrument_master;
pub mod provider;
pub mod redact;
pub mod storage;
pub mod validate;

pub use calendar::{
    CalendarError, CalendarProvenance, Holiday, KrCalendar, KrCalendarSpec, SessionTimes, krx_2020,
};
pub use contract::{
    ALL_RESPONSE_KINDS, FetchMode, MARKET_KR, PROVIDER_KRX, RawEnvelope, RequestMetadata,
    ResponseKind, StoredFile,
};
pub use ingest::{IngestError, IngestOutcome, IngestRequest, ingest_bundle};
pub use instrument_master::{
    AliasNamespace, Instrument, InstrumentAlias, InstrumentMaster, ListingReason, MasterError,
    seed_universe,
};
pub use provider::{
    CredentialRef, EodProvider, FetchRequest, KrxMode, KrxProvider, ProviderError, RecordedBundle,
};
pub use storage::{BatchSpec, FileEntry, ManifestEntry, RawStore, StoreError};
