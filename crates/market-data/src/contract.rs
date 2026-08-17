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

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use domain::{BatchId, ContentHash, TradingDate, UtcTimestamp};

/// Canonical provider id of the Korea Exchange data connector.
pub const PROVIDER_KRX: &str = "krx";
/// Canonical provider id of the Korea Investment & Securities Open API connector.
pub const PROVIDER_KIS: &str = "kis";
/// Provider id for the provider-neutral canonical batch derived from KIS wire
/// responses. The wire batch remains under [`PROVIDER_KIS`] forever.
pub const PROVIDER_KIS_NORMALIZED: &str = "kis-normalized";
/// Provider id for KIS candidate-source wire responses.
///
/// Candidate source deliveries are deliberately kept out of `provider=kis`:
/// the EOD recovery scope owns that manifest and must never attempt to feed a
/// candidate response into the four-file EOD normalizer.
pub const PROVIDER_KIS_CANDIDATE: &str = "kis-candidate";
/// Provider id for canonical candidate documents derived from
/// [`PROVIDER_KIS_CANDIDATE`] wire responses.
pub const PROVIDER_KIS_CANDIDATE_NORMALIZED: &str = "kis-candidate-normalized";
/// Canonical market id of the Korean market.
pub const MARKET_KR: &str = "kr";

/// The licensed response classes this contract covers. The first four are the
/// original EOD surface; the final four feed the separate stock-candidate
/// research vertical and remain provider-neutral.
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
    /// Daily foreign/institutional net-flow observations.
    InvestorFlow,
    /// Daily suspension/administrative/audit/capital-impairment flags.
    MarketStatus,
    /// Point-in-time financial statement observations and revisions.
    Fundamentals,
    /// Point-in-time index membership intervals.
    IndexMembership,
    /// Versioned sector/taxonomy classifications.
    SectorClassification,
    /// KIS fixed-width candidate master archives (KOSPI, KOSDAQ, idxcode).
    ///
    /// This is deliberately distinct from [`Reference`]: these ZIP bodies are
    /// candidate-source evidence and must never enter the EOD reference
    /// normalizer or publication path.
    CandidateMaster,
}

impl ResponseKind {
    /// The stable lower-snake-case wire name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bars => "bars",
            Self::Reference => "reference",
            Self::Calendar => "calendar",
            Self::CorporateActions => "corporate_actions",
            Self::InvestorFlow => "investor_flow",
            Self::MarketStatus => "market_status",
            Self::Fundamentals => "fundamentals",
            Self::IndexMembership => "index_membership",
            Self::SectorClassification => "sector_classification",
            Self::CandidateMaster => "candidate_master",
        }
    }

    /// Parses the stable wire name.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "bars" => Some(Self::Bars),
            "reference" => Some(Self::Reference),
            "calendar" => Some(Self::Calendar),
            "corporate_actions" => Some(Self::CorporateActions),
            "investor_flow" => Some(Self::InvestorFlow),
            "market_status" => Some(Self::MarketStatus),
            "fundamentals" => Some(Self::Fundamentals),
            "index_membership" => Some(Self::IndexMembership),
            "sector_classification" => Some(Self::SectorClassification),
            "candidate_master" => Some(Self::CandidateMaster),
            _ => None,
        }
    }
}

impl std::fmt::Display for ResponseKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The original EOD response classes requested by the default collector.
pub const EOD_RESPONSE_KINDS: [ResponseKind; 4] = [
    ResponseKind::Bars,
    ResponseKind::Reference,
    ResponseKind::Calendar,
    ResponseKind::CorporateActions,
];

/// Optional source classes for the separate stock-candidate pipeline.
pub const CANDIDATE_RESPONSE_KINDS: [ResponseKind; 5] = [
    ResponseKind::InvestorFlow,
    ResponseKind::MarketStatus,
    ResponseKind::Fundamentals,
    ResponseKind::IndexMembership,
    ResponseKind::SectorClassification,
];

/// The separate fixed-width KIS candidate-master archive scope.  It is kept
/// out of [`CANDIDATE_RESPONSE_KINDS`] because those JSON classes feed the
/// candidate document pipeline; candidate-master ZIPs are Raw-only evidence.
pub const CANDIDATE_MASTER_RESPONSE_KINDS: [ResponseKind; 1] = [ResponseKind::CandidateMaster];

/// All generic licensed response classes, in stable order. This list is a
/// schema registry, not a promise that every provider bundle supplies every
/// class. The Raw-only candidate-master scope has its own registry above and
/// is intentionally not accepted by generic JSON candidate collectors.
pub const ALL_RESPONSE_KINDS: [ResponseKind; 9] = [
    ResponseKind::Bars,
    ResponseKind::Reference,
    ResponseKind::Calendar,
    ResponseKind::CorporateActions,
    ResponseKind::InvestorFlow,
    ResponseKind::MarketStatus,
    ResponseKind::Fundamentals,
    ResponseKind::IndexMembership,
    ResponseKind::SectorClassification,
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
    /// Canonical validated storage path inside the immutable batch directory.
    pub storage_path: PathBuf,
}

/// The date partition key used by the raw zone: `date=YYYY-MM-DD`.
pub fn date_partition(date: &TradingDate) -> String {
    format!("date={}", date.to_iso())
}
