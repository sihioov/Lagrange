//! Provider-neutral **end-of-day (EOD) raw contract** (Todo 8).
//!
//! Every licensed provider response — bars, reference, calendar, corporate
//! actions — crosses the pipeline as an opaque [`RawEnvelope`]:
//!
//! ```text
//! bytes            - the provider response/file, stored byte-for-byte, never parsed
//! retrieved_at     - UTC instant the delivery was retrieved
//! request          - provider request metadata (endpoint, query, REDACTED headers, mode)
//! batch_id         - the ingestion batch this response belongs to
//! content_hash     - sha256 of `bytes` (immutability proof, FR-DATA-001)
//! ```
//!
//! Providers implement [`crate::provider::EodProvider`]; the collector persists
//! envelopes unchanged under `data/raw/provider=krx/market=kr/date=...` and
//! records them in an append-only manifest (see [`crate::storage`]).
//!
//! Headers in [`RequestMetadata`] MUST already be redacted before they enter the
//! envelope — the [`crate::redact::Redactor`] scans every log line, but the raw
//! metadata itself never carries credentials.

use serde::{Deserialize, Serialize};

use domain::{BatchId, ContentHash, TradingDate, UtcTimestamp};

/// Canonical provider id of the Korea Exchange data connector.
pub const PROVIDER_KRX: &str = "krx";
/// Canonical market id of the Korean market.
pub const MARKET_KR: &str = "kr";

/// The four licensed KRX response classes this contract covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseKind {
    /// Per-instrument daily OHLCV bars.
    Bars,
    /// Instrument reference/master data.
    Reference,
    /// Trading calendar/session metadata.
    Calendar,
    /// Corporate actions (splits, dividends, ticker changes).
    CorporateActions,
}

impl ResponseKind {
    /// The stable wire name (`bars`, `reference`, `calendar`, `corporate_actions`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bars => "bars",
            Self::Reference => "reference",
            Self::Calendar => "calendar",
            Self::CorporateActions => "corporate_actions",
        }
    }

    /// Parses the stable wire name.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "bars" => Some(Self::Bars),
            "reference" => Some(Self::Reference),
            "calendar" => Some(Self::Calendar),
            "corporate_actions" => Some(Self::CorporateActions),
            _ => None,
        }
    }
}

impl std::fmt::Display for ResponseKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// All four licensed response classes, in stable order.
pub const ALL_RESPONSE_KINDS: [ResponseKind; 4] = [
    ResponseKind::Bars,
    ResponseKind::Reference,
    ResponseKind::Calendar,
    ResponseKind::CorporateActions,
];

/// How a delivery was fetched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchMode {
    /// Playback of recorded synthetic contract fixtures (CI; no network).
    Synthetic,
    /// Owner-only credentialed mode against the licensed KRX endpoint.
    /// Never exercised in CI: no real KRX credentials exist.
    Credentialed,
}

impl FetchMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Synthetic => "synthetic",
            Self::Credentialed => "credentialed",
        }
    }
}

impl std::fmt::Display for FetchMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Provider request metadata recorded with every delivery.
///
/// `headers` MUST contain redacted values only — never credentials. The
/// collector additionally routes every log line through the redactor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestMetadata {
    /// The documented licensed endpoint id (e.g. `krx.eod.bars.v1`).
    pub endpoint: String,
    /// Query parameters as sent (values already redacted where sensitive).
    pub query: Vec<(String, String)>,
    /// Request headers with REDACTED values (never auth/keys).
    pub headers: Vec<(String, String)>,
    /// Fetch mode used for this delivery.
    pub mode: FetchMode,
}

/// The raw response envelope: opaque provider bytes plus immutable provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEnvelope {
    /// The ingestion batch this response belongs to.
    pub batch_id: BatchId,
    /// Which licensed response class the bytes represent.
    pub kind: ResponseKind,
    /// Provider file name, used as the storage key inside the batch dir.
    /// Must be a plain file name: validated against path traversal on write.
    pub file_name: String,
    /// The provider response bytes — stored byte-for-byte, never modified.
    pub bytes: Vec<u8>,
    /// SHA-256 of `bytes` (see [`domain::ContentHash`]).
    pub content_hash: ContentHash,
    /// UTC instant this delivery was retrieved.
    pub retrieved_at: UtcTimestamp,
    /// Provider request metadata (redacted).
    pub request: RequestMetadata,
}

impl RawEnvelope {
    /// Builds an envelope, computing the content hash over `bytes`.
    pub fn new(
        batch_id: BatchId,
        kind: ResponseKind,
        file_name: impl Into<String>,
        bytes: Vec<u8>,
        retrieved_at: UtcTimestamp,
        request: RequestMetadata,
    ) -> Self {
        let content_hash = ContentHash::from_bytes(&bytes);
        Self {
            batch_id,
            kind,
            file_name: file_name.into(),
            bytes,
            content_hash,
            retrieved_at,
            request,
        }
    }
}

/// A stored raw file (as read back from the immutable zone).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredFile {
    /// The provider file name inside the batch dir.
    pub file_name: String,
    /// The stored bytes, verified against the recorded content hash.
    pub bytes: Vec<u8>,
}

/// The date partition key used by the raw zone: `date=YYYY-MM-DD`.
pub fn date_partition(date: &TradingDate) -> String {
    format!("date={}", date.to_iso())
}
