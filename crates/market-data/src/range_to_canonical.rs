//! Stage4B-0: fail-closed evidence gate for KIS range sessions.
//!
//! This module intentionally stops one boundary before the canonical
//! publication/Curated zones.  A Stage4A session is a current vendor
//! snapshot, not a historical point-in-time observation.  The builder below
//! therefore requires an explicit operator approval for that non-strict PIT
//! policy and returns an in-memory candidate only.  It never writes Raw,
//! Curated, a database row, or a [`PublicationBundle`].

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use domain::{
    AssetClass, BatchId, ContentHash, FixedPoint, InstrumentId, Quantity, TradingDate,
    UtcTimestamp, Venue,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use thiserror::Error;
use uuid::Uuid;

use crate::contract::{
    FetchMode, MARKET_KR, PROVIDER_KIS, PROVIDER_KIS_DAILY_RANGE,
    PROVIDER_KIS_DAILY_RANGE_NORMALIZED, RequestMetadata, ResponseKind, StoredFile,
};
use crate::normalize::bonus_split_factor_from_percent;
use crate::providers::kis::{KR_ETF_CORE_SYMBOLS, KisActionSpec, kis_action_spec};
use crate::range_normalize::{
    ExpectedRangeSessions, RANGE_NORMALIZED_SCHEMA_VERSION, RANGE_NORMALIZER,
    RANGE_NORMALIZER_SCHEMA_VERSION, RangeNormalizationLineage,
    deterministic_range_normalized_batch_id_with_identity,
};
use crate::storage::{FileEntry, ManifestEntry, RawStore, StoreError};

/// Version of this local, non-persisted bridge contract.
pub const RANGE_CANONICAL_BRIDGE_VERSION: &str = "kis-range-to-canonical-evidence-v0";
/// The bounded pre-publication contract approved for the first owner beta.
pub const HISTORICAL_PRICE_ONLY_BETA_CONTRACT: &str = "kis-historical-price-only-beta-v1";
/// The one immutable Stage5 source batch named by the owner beta plan.
pub const HISTORICAL_PRICE_ONLY_BETA_SOURCE_BATCH_ID: &str = "3d4f061f-8b8c-54f3-bb44-4d491b3ad256";
/// Inclusive first session allowed by the owner beta contract.
pub const HISTORICAL_PRICE_ONLY_BETA_START: &str = "2020-01-31";
/// Inclusive final session allowed by the owner beta contract.
pub const HISTORICAL_PRICE_ONLY_BETA_END: &str = "2026-08-19";
/// Exact count in the checked-in, hash-verified XKRX date selection.
pub const HISTORICAL_PRICE_ONLY_BETA_SESSION_COUNT: usize = 1_608;
/// Exact immutable Stage5 file count (11 ETFs x 17 bounded request windows).
pub const HISTORICAL_PRICE_ONLY_BETA_SOURCE_FILE_COUNT: usize = 187;
/// The only PIT policy accepted by this bridge.
pub const NON_STRICT_PIT_POLICY_ID: &str = "kis-historical-vendor-snapshot-v1";
/// The seven KSD response classes that must be covered by an action evidence
/// package, even when every response is an attested zero-result.
pub const REQUIRED_ACTION_KINDS: [&str; 7] = [
    "paidin-subscription",
    "paidin-record",
    "bonus-issue",
    "dividend",
    "merger-split",
    "reverse-split",
    "capital-decrease",
];

const STAGE4A_ENDPOINT: &str = "kis.range.normalized/kis-daily-range-to-session-bars-v2/bars";
const STAGE4A_DATASET_KIND: &str = "kis-daily-range-bars";
const DAILY_RANGE_ENDPOINT: &str =
    "/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice";
const DAILY_RANGE_TR_ID: &str = "FHKST03010100";
const HISTORICAL_PRICE_ONLY_BETA_WINDOW_COUNT: usize = 17;
const STAGE5_QUERY_KEYS: [&str; 6] = [
    "FID_COND_MRKT_DIV_CODE",
    "FID_INPUT_ISCD",
    "FID_INPUT_DATE_1",
    "FID_INPUT_DATE_2",
    "FID_PERIOD_DIV_CODE",
    "FID_ORG_ADJ_PRC",
];
const PIT_MODE: &str = "acquisition-time-vendor-snapshot";
const APPROVED_EVIDENCE_REGISTRY_BYTES: &[u8] =
    include_bytes!("../../../configs/evidence/kis-range-canonical-approved-manifests.json");
// Stage4B deliberately accepts one terminal KSD response page only.  The
// live provider has a separate, bounded multipage implementation, but this
// evidence package does not preserve a continuation chain, so accepting any
// marker here would create an unprovable partial action range.
const ACTION_CONTINUATION_FIELDS: &[&str] = &[
    "cts",
    "ctx_area_fk",
    "ctx_area_nk",
    "ctx_area_fk200",
    "ctx_area_nk200",
];

/// A schedule authority.  The checked-in XKRX dates artifact is audit-only;
/// it cannot be passed as the historical schedule required by this bridge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScheduleAuthority {
    /// A reviewed source with explicit session times and provenance.
    Reviewed { source: String, version: String },
    /// An audit-only source, deliberately rejected by the builder.
    AuditOnly { source: String, version: String },
}

/// One historical session's schedule.  Breaks are retained rather than
/// flattened to the current 09:00/15:30 contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HistoricalSessionScheduleEvidence {
    schema_version: u32,
    calendar_id: String,
    calendar_hash: ContentHash,
    session_date: TradingDate,
    open_utc: UtcTimestamp,
    close_utc: UtcTimestamp,
    break_start_utc: Option<UtcTimestamp>,
    break_end_utc: Option<UtcTimestamp>,
    authority: ScheduleAuthority,
}

/// Listing evidence for one canonical ETF.  `listed_at` is mandatory; an
/// absent `delisted_at` means an explicitly open-ended interval, not a
/// missing interval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListingInstrumentEvidence {
    instrument_id: InstrumentId,
    name: String,
    kind: AssetClass,
    lot_size: Quantity,
    listed_at: Option<TradingDate>,
    delisted_at: Option<TradingDate>,
    acquired_at: UtcTimestamp,
}

/// A versioned listing snapshot. `snapshot_hash` binds the approved universe
/// identity carried by Stage4A; the evidence-package artifact hash binds the
/// full interval-bearing JSON bytes. The current-reference YAML alone cannot
/// satisfy this contract because it has no effective listing intervals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListingMasterEvidence {
    schema_version: u32,
    snapshot_id: String,
    source: String,
    captured_at: UtcTimestamp,
    instruments: Vec<ListingInstrumentEvidence>,
    snapshot_hash: ContentHash,
}

/// One raw action response file covered by the action evidence package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ActionCoverageFileEvidence {
    kind: String,
    endpoint: String,
    file_name: String,
    content_hash: ContentHash,
    size_bytes: u64,
    query_start: TradingDate,
    query_end: TradingDate,
}

/// An action that is safe to carry as evidence at this intermediate boundary.
/// Other KSD event classes are represented by [`RangeAction::Unsupported`] and
/// are rejected before a candidate can be built.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum RangeAction {
    BonusIssue {
        instrument_id: InstrumentId,
        record_date: TradingDate,
        ex_date: TradingDate,
        split_factor: FixedPoint,
        available_at: UtcTimestamp,
    },
    Unsupported {
        kind: String,
        reason: String,
    },
}

/// Complete range/action evidence for one session candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ActionCoverageEvidence {
    range_start: TradingDate,
    range_end: TradingDate,
    raw_batch_id: BatchId,
    raw_manifest_hash: ContentHash,
    files: Vec<ActionCoverageFileEvidence>,
    actions: Vec<RangeAction>,
    acquired_at: UtcTimestamp,
    coverage_hash: ContentHash,
}

impl ActionCoverageEvidence {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        range_start: TradingDate,
        range_end: TradingDate,
        raw_batch_id: BatchId,
        raw_manifest_hash: ContentHash,
        files: Vec<ActionCoverageFileEvidence>,
        actions: Vec<RangeAction>,
        acquired_at: UtcTimestamp,
    ) -> Self {
        let mut value = Self {
            range_start,
            range_end,
            raw_batch_id,
            raw_manifest_hash,
            files,
            actions,
            acquired_at,
            coverage_hash: ContentHash::from_bytes(b"uncomputed"),
        };
        value.coverage_hash = value.computed_hash();
        value
    }

    fn hash_view(&self) -> Value {
        json!({
            "range_start": self.range_start,
            "range_end": self.range_end,
            "raw_batch_id": self.raw_batch_id,
            "raw_manifest_hash": self.raw_manifest_hash,
            "files": self.files,
            "actions": self.actions,
            "acquired_at": self.acquired_at,
        })
    }

    pub(crate) fn computed_hash(&self) -> ContentHash {
        ContentHash::from_bytes(
            &serde_json::to_vec(&self.hash_view()).expect("action hash view is serializable"),
        )
    }

    /// Number of independently pinned KSD response files in this coverage.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Whether every pinned KSD response was verified as an exact empty array.
    pub fn all_action_responses_empty(&self) -> bool {
        self.files.len() == REQUIRED_ACTION_KINDS.len() && self.actions.is_empty()
    }
}

/// Explicit operator approval required because the KIS historical endpoint
/// gives no availability/revision/knowledge-time evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NonStrictPitPolicyApproval {
    schema_version: u32,
    policy_id: String,
    approved: bool,
    approved_by: String,
    approved_at: UtcTimestamp,
    rationale: String,
}

impl NonStrictPitPolicyApproval {
    fn hash(&self) -> ContentHash {
        ContentHash::from_bytes(&serde_json::to_vec(self).expect("PIT policy is serializable"))
    }
}

/// Verified evidence is intentionally constructible only by
/// [`load_verified_range_canonical_evidence`].  The fields are private so a
/// caller cannot turn a self-hashed DTO or a `Reviewed` enum into an accepted
/// evidence package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedRangeCanonicalEvidence {
    manifest_hash: ContentHash,
    schedule_artifact_hash: ContentHash,
    listing_artifact_hash: ContentHash,
    pit_policy_artifact_hash: ContentHash,
    schedule: HistoricalSessionScheduleEvidence,
    listing: ListingMasterEvidence,
    actions: ActionCoverageEvidence,
    pit_policy: NonStrictPitPolicyApproval,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidencePackageManifest {
    schema_version: u32,
    bridge_version: String,
    source_batch_id: BatchId,
    normalized_batch_id: BatchId,
    session_date: TradingDate,
    range_start: TradingDate,
    range_end: TradingDate,
    calendar: EvidenceArtifactRef,
    listing: EvidenceArtifactRef,
    pit_policy: EvidenceArtifactRef,
    actions: Vec<EvidenceActionRef>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceArtifactRef {
    path: String,
    sha256: ContentHash,
    size_bytes: u64,
    schema_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceActionRef {
    kind: String,
    raw_batch_id: BatchId,
    raw_manifest_hash: ContentHash,
    raw_file_name: String,
    content_hash: ContentHash,
    size_bytes: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovedEvidenceRegistry {
    schema_version: u32,
    bridge_version: String,
    approved_manifest_sha256: Vec<ContentHash>,
}

const EVIDENCE_PACKAGE_SCHEMA_VERSION: u32 = 1;

/// Safe numeric representation of one original-price bar.  It contains no
/// adjusted or total-return values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RangeCanonicalBarCandidate {
    pub instrument_id: InstrumentId,
    pub session_date: TradingDate,
    pub open: FixedPoint,
    pub high: FixedPoint,
    pub low: FixedPoint,
    pub close: FixedPoint,
    pub volume: u64,
    pub trading_value: Option<FixedPoint>,
}

/// A safe, explicitly non-persisted canonical candidate.  No downstream
/// publication implementation accepts this scope or this type yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeCanonicalCandidate {
    candidate_id: BatchId,
    bridge_version: String,
    evidence_manifest_hash: ContentHash,
    schedule_artifact_hash: ContentHash,
    listing_artifact_hash: ContentHash,
    pit_policy_artifact_hash: ContentHash,
    source_batch_id: BatchId,
    source_entry_hash: ContentHash,
    source_file_hash: ContentHash,
    upstream_range_batch_id: BatchId,
    upstream_range_manifest_hash: ContentHash,
    session_date: TradingDate,
    acquired_at: UtcTimestamp,
    bars: Vec<RangeCanonicalBarCandidate>,
    schedule: HistoricalSessionScheduleEvidence,
    listing: ListingMasterEvidence,
    actions: Vec<RangeAction>,
    action_coverage: ActionCoverageEvidence,
    pit_policy: NonStrictPitPolicyApproval,
}

impl RangeCanonicalCandidate {
    /// Number of independently verified KSD response files retained as
    /// range coverage.  This does not imply that the candidate is publishable.
    pub fn action_coverage_file_count(&self) -> usize {
        self.action_coverage.file_count()
    }

    /// True only when all seven verified KSD response arrays were empty.
    pub fn action_coverage_is_zero_result(&self) -> bool {
        self.action_coverage.all_action_responses_empty()
    }
}

/// One normalized session proven against immutable Stage5 Raw bytes.
///
/// Fields are intentionally private: callers can inspect the witness through
/// accessors but cannot assemble a value that bypasses [`RawStore`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoricalPriceOnlySessionWitness {
    session_date: TradingDate,
    normalized_batch_id: BatchId,
    normalized_entry_hash: ContentHash,
    normalized_bars_hash: ContentHash,
    acquired_at: UtcTimestamp,
}

impl HistoricalPriceOnlySessionWitness {
    pub fn session_date(&self) -> TradingDate {
        self.session_date
    }

    pub fn normalized_batch_id(&self) -> BatchId {
        self.normalized_batch_id
    }

    pub fn normalized_entry_hash(&self) -> &ContentHash {
        &self.normalized_entry_hash
    }

    pub fn normalized_bars_hash(&self) -> &ContentHash {
        &self.normalized_bars_hash
    }

    pub fn acquired_at(&self) -> UtcTimestamp {
        self.acquired_at
    }
}

/// Opaque, RawStore-authenticated input for the future price-only
/// materializer.
///
/// It is neither serializable nor publicly constructible. A writer must
/// receive this value directly from [`verify_historical_price_only_beta_input`]
/// (or invoke that verifier itself), which prevents a self-hashed DTO from
/// substituting for immutable Stage5 and KSD evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoricalPriceOnlyBetaInput {
    range_start: TradingDate,
    range_end: TradingDate,
    source_batch_id: BatchId,
    source_manifest_hash: ContentHash,
    source_files: Vec<FileEntry>,
    action_batch_id: BatchId,
    action_manifest_hash: ContentHash,
    action_files: Vec<ActionCoverageFileEvidence>,
    sessions: Vec<HistoricalPriceOnlySessionWitness>,
    bars: Vec<RangeCanonicalBarCandidate>,
    actions: Vec<RangeAction>,
}

impl HistoricalPriceOnlyBetaInput {
    pub fn range_start(&self) -> TradingDate {
        self.range_start
    }

    pub fn range_end(&self) -> TradingDate {
        self.range_end
    }

    pub fn source_batch_id(&self) -> BatchId {
        self.source_batch_id
    }

    pub fn source_manifest_hash(&self) -> &ContentHash {
        &self.source_manifest_hash
    }

    pub fn source_files(&self) -> &[FileEntry] {
        &self.source_files
    }

    pub fn action_batch_id(&self) -> BatchId {
        self.action_batch_id
    }

    pub fn action_manifest_hash(&self) -> &ContentHash {
        &self.action_manifest_hash
    }

    pub fn action_file_count(&self) -> usize {
        self.action_files.len()
    }

    pub fn sessions(&self) -> &[HistoricalPriceOnlySessionWitness] {
        &self.sessions
    }

    pub fn bars(&self) -> &[RangeCanonicalBarCandidate] {
        &self.bars
    }

    pub fn actions(&self) -> &[RangeAction] {
        &self.actions
    }
}

/// Metadata-only pins discovered from committed Raw manifests.
///
/// This accessor deliberately contains no file names, requests, or response
/// bytes, and it is not serializable.  Discovery is a candidate seam only:
/// the returned pins still require explicit owner review and the separate
/// byte-reading verifier before they can become an authenticated input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoricalPriceOnlyBetaPins {
    contract: &'static str,
    range_start: TradingDate,
    range_end: TradingDate,
    source_batch_id: BatchId,
    source_manifest_hash: ContentHash,
    source_file_count: usize,
    action_batch_id: BatchId,
    action_manifest_hash: ContentHash,
    action_file_count: usize,
}

impl HistoricalPriceOnlyBetaPins {
    pub fn contract(&self) -> &'static str {
        self.contract
    }

    pub fn range_start(&self) -> TradingDate {
        self.range_start
    }

    pub fn range_end(&self) -> TradingDate {
        self.range_end
    }

    pub fn source_batch_id(&self) -> BatchId {
        self.source_batch_id
    }

    pub fn source_manifest_hash(&self) -> &ContentHash {
        &self.source_manifest_hash
    }

    pub fn source_file_count(&self) -> usize {
        self.source_file_count
    }

    pub fn action_batch_id(&self) -> BatchId {
        self.action_batch_id
    }

    pub fn action_manifest_hash(&self) -> &ContentHash {
        &self.action_manifest_hash
    }

    pub fn action_file_count(&self) -> usize {
        self.action_file_count
    }
}

#[derive(Debug, Clone)]
struct HistoricalBetaVerificationScope {
    source_batch_id: BatchId,
    range_start: TradingDate,
    range_end: TradingDate,
    sessions: Vec<TradingDate>,
    calendar_id: String,
    calendar_hash: ContentHash,
    listing_snapshot_id: String,
    listing_snapshot_hash: ContentHash,
}

/// Typed fail-closed errors.  In particular, no missing evidence is silently
/// replaced by a synthetic empty calendar/master/action response.
#[derive(Debug, Error)]
pub enum RangeCanonicalError {
    #[error("Stage4B accepts only {expected_provider}/{expected_market}, got {provider}/{market}")]
    UnsupportedScope {
        expected_provider: &'static str,
        expected_market: &'static str,
        provider: String,
        market: String,
    },
    #[error("Stage4A normalized batch must use credentialed mode")]
    UnsupportedMode,
    #[error("historical session schedule is unsupported: {reason}")]
    UnsupportedHistoricalSessionSchedule { reason: String },
    #[error("listing master evidence is missing or invalid: {reason}")]
    MissingListingMasterEvidence { reason: String },
    #[error("action evidence is missing or invalid: {reason}")]
    MissingActionEvidence { reason: String },
    #[error("non-strict PIT policy is not approved: {reason}")]
    NonStrictPitNotApproved { reason: String },
    #[error("unsupported corporate action {kind}")]
    UnsupportedAction { kind: String },
    #[error("invalid bar value {field} for {instrument}: {value} ({reason})")]
    InvalidBarValue {
        instrument: String,
        field: String,
        value: String,
        reason: String,
    },
    #[error("Stage4A document is malformed: {reason}")]
    MalformedStage4A { reason: String },
    #[error(
        "Stage4A legacy schema is unsupported: schema={schema_version}, normalizer={normalizer}"
    )]
    UnsupportedLegacyStage4A {
        schema_version: u32,
        normalizer: String,
    },
    #[error("Stage4A lineage is invalid: {reason}")]
    InvalidLineage { reason: String },
    #[error("session candidate coverage is invalid: {reason}")]
    InvalidSession { reason: String },
    #[error("historical price-only beta contract is invalid: {reason}")]
    HistoricalBetaContract { reason: String },
    #[error("immutable Raw read failed: {0}")]
    Store(#[from] StoreError),
    #[error("serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("evidence package is invalid: {reason}")]
    EvidencePackage { reason: String },
    #[error("evidence package path is unsafe: {path}")]
    UnsafeEvidencePath { path: String },
    #[error("evidence artifact {path} failed its pinned hash/size/schema check: {reason}")]
    EvidenceArtifact { path: String, reason: String },
    #[error("upstream range Raw manifest is invalid: {reason}")]
    UpstreamManifest { reason: String },
    #[error("KSD action Raw evidence is invalid: {reason}")]
    ActionEvidence { reason: String },
    #[error(
        "KSD action pagination is incomplete for {kind}: continuation marker {marker} is non-terminal"
    )]
    IncompleteActionPagination { kind: String, marker: String },
}

#[derive(Debug, Deserialize)]
struct Stage4ADocument {
    schema_version: u32,
    dataset_kind: String,
    date: TradingDate,
    bars: Vec<Value>,
    acquired_at: UtcTimestamp,
    pit: Stage4APit,
    #[serde(rename = "_lineage")]
    lineage: RangeNormalizationLineage,
}

#[derive(Debug, Deserialize)]
struct Stage4APit {
    mode: String,
    strict: bool,
    availability_evidence: bool,
    revision_evidence: bool,
    knowledge_time_evidence: bool,
}

/// Load one pinned evidence package and its immutable KIS action Raw batch.
///
/// Every referenced local artifact is checked for regular-file/path safety,
/// exact size, exact SHA-256, and declared schema before it can become the
/// private verified evidence type.  KSD response bodies are read through
/// `RawStore`, never from an operator-selected path. The manifest's computed
/// hash must be present in the repository-controlled approved registry. The
/// registry is deliberately empty until an operator reviews and commits a
/// real package pin; a caller cannot self-approve a package through this API.
pub fn load_verified_range_canonical_evidence(
    raw: &RawStore,
    normalized_entry: &ManifestEntry,
    root: &Path,
) -> Result<VerifiedRangeCanonicalEvidence, RangeCanonicalError> {
    let root = safe_package_root(root)?;
    let manifest_bytes = read_safe_file(&root, Path::new("manifest.json"))?;
    let pinned_manifest_hash = ContentHash::from_bytes(&manifest_bytes);
    let registry: ApprovedEvidenceRegistry =
        serde_json::from_slice(APPROVED_EVIDENCE_REGISTRY_BYTES).map_err(|error| {
            RangeCanonicalError::EvidencePackage {
                reason: format!("approved evidence registry is malformed: {error}"),
            }
        })?;
    if registry.schema_version != EVIDENCE_PACKAGE_SCHEMA_VERSION
        || registry.bridge_version != RANGE_CANONICAL_BRIDGE_VERSION
        || !registry
            .approved_manifest_sha256
            .iter()
            .any(|hash| hash == &pinned_manifest_hash)
    {
        return Err(RangeCanonicalError::EvidencePackage {
            reason: format!(
                "manifest hash {pinned_manifest_hash} is not in the approved evidence registry"
            ),
        });
    }
    load_verified_range_canonical_evidence_with_pin(
        raw,
        normalized_entry,
        &root,
        &pinned_manifest_hash,
    )
}

/// Unit-test-only loader for temporary fixtures.  Production callers must use
/// [`load_verified_range_canonical_evidence`], which checks the embedded
/// approved registry.  Keeping this helper behind `cfg(test)` prevents a
/// self-created package/hash from becoming a production trust path.
#[cfg(test)]
fn load_with_approved_pin_for_test(
    raw: &RawStore,
    normalized_entry: &ManifestEntry,
    root: &Path,
    pinned_manifest_hash: &ContentHash,
) -> Result<VerifiedRangeCanonicalEvidence, RangeCanonicalError> {
    let root = safe_package_root(root)?;
    load_verified_range_canonical_evidence_with_pin(
        raw,
        normalized_entry,
        &root,
        pinned_manifest_hash,
    )
}

fn load_verified_range_canonical_evidence_with_pin(
    raw: &RawStore,
    normalized_entry: &ManifestEntry,
    root: &Path,
    pinned_manifest_hash: &ContentHash,
) -> Result<VerifiedRangeCanonicalEvidence, RangeCanonicalError> {
    validate_scope(normalized_entry)?;
    let (file, document) = read_stage4a_document(raw, normalized_entry)?;
    // Parse the document before checking the v2 request endpoint.  This
    // preserves the typed legacy-schema rejection for a v1 batch instead of
    // reducing an old serialized contract to a generic malformed request.
    validate_entry(normalized_entry)?;
    validate_document(normalized_entry, file, &document)?;
    validate_lineage(normalized_entry, file, &document.lineage)?;
    verify_upstream_range_manifest(raw, &document.lineage)?;

    let manifest_bytes = read_safe_file(root, Path::new("manifest.json"))?;
    if ContentHash::from_bytes(&manifest_bytes) != *pinned_manifest_hash {
        return Err(RangeCanonicalError::EvidencePackage {
            reason: "evidence package manifest hash differs from pinned hash".to_owned(),
        });
    }
    let package: EvidencePackageManifest =
        serde_json::from_slice(&manifest_bytes).map_err(|error| {
            RangeCanonicalError::EvidencePackage {
                reason: format!("manifest.json is malformed: {error}"),
            }
        })?;
    if package.schema_version != EVIDENCE_PACKAGE_SCHEMA_VERSION
        || package.bridge_version != RANGE_CANONICAL_BRIDGE_VERSION
        || package.source_batch_id != document.lineage.upstream_batch_id
        || package.normalized_batch_id != normalized_entry.batch_id
        || package.session_date != normalized_entry.date
        || package.range_start != document.lineage.source_start
        || package.range_end != document.lineage.source_end
    {
        return Err(RangeCanonicalError::EvidencePackage {
            reason: "package schema or range/session identity differs from Stage4A lineage"
                .to_owned(),
        });
    }

    let schedule_bytes = read_artifact(root, &package.calendar).map_err(map_schedule_error)?;
    let schedule: HistoricalSessionScheduleEvidence = serde_json::from_slice(&schedule_bytes)
        .map_err(|error| {
            map_schedule_error(RangeCanonicalError::EvidenceArtifact {
                path: package.calendar.path.clone(),
                reason: format!("schedule schema is malformed: {error}"),
            })
        })?;
    validate_schedule(&schedule, normalized_entry.date)?;
    if schedule.calendar_id != document.lineage.calendar_id
        || schedule.calendar_hash != document.lineage.calendar_hash
    {
        return Err(RangeCanonicalError::UnsupportedHistoricalSessionSchedule {
            reason: "schedule artifact identity differs from Stage4A lineage".to_owned(),
        });
    }

    let listing_bytes = read_artifact(root, &package.listing).map_err(map_listing_error)?;
    let listing: ListingMasterEvidence =
        serde_json::from_slice(&listing_bytes).map_err(|error| {
            map_listing_error(RangeCanonicalError::EvidenceArtifact {
                path: package.listing.path.clone(),
                reason: format!("listing schema is malformed: {error}"),
            })
        })?;
    validate_listing(
        &listing,
        normalized_entry.date,
        normalized_entry.retrieved_at,
    )?;
    if listing.snapshot_id != document.lineage.listing_snapshot_id
        || listing.snapshot_hash != document.lineage.listing_snapshot_hash
    {
        return Err(RangeCanonicalError::MissingListingMasterEvidence {
            reason: "listing artifact identity differs from Stage4A lineage".to_owned(),
        });
    }

    let pit_bytes = read_artifact(root, &package.pit_policy).map_err(map_pit_error)?;
    let pit_policy: NonStrictPitPolicyApproval =
        serde_json::from_slice(&pit_bytes).map_err(|error| {
            map_pit_error(RangeCanonicalError::EvidenceArtifact {
                path: package.pit_policy.path.clone(),
                reason: format!("PIT approval schema is malformed: {error}"),
            })
        })?;
    validate_pit_policy(&pit_policy)?;

    let actions = load_action_evidence(
        raw,
        &package.actions,
        package.range_start,
        package.range_end,
    )?;
    validate_actions(
        &actions,
        document.lineage.source_start,
        document.lineage.source_end,
        normalized_entry.date,
    )?;

    Ok(VerifiedRangeCanonicalEvidence {
        manifest_hash: pinned_manifest_hash.clone(),
        schedule_artifact_hash: package.calendar.sha256.clone(),
        listing_artifact_hash: package.listing.sha256.clone(),
        pit_policy_artifact_hash: package.pit_policy.sha256.clone(),
        schedule,
        listing,
        actions,
        pit_policy,
    })
}

fn read_stage4a_document<'a>(
    raw: &RawStore,
    entry: &'a ManifestEntry,
) -> Result<(&'a FileEntry, Stage4ADocument), RangeCanonicalError> {
    if entry.files.len() != 1 {
        return Err(RangeCanonicalError::MalformedStage4A {
            reason: "a session batch must contain exactly one bars file".to_owned(),
        });
    }
    let file = &entry.files[0];
    let expected_name = format!("bars-{}.json", entry.date);
    if file.kind != ResponseKind::Bars || file.file_name != expected_name {
        return Err(RangeCanonicalError::MalformedStage4A {
            reason: format!("expected Bars/{expected_name}"),
        });
    }
    let stored = raw.read_batch_bytes(&entry.provider, &entry.market, entry)?;
    let stored_file = stored
        .iter()
        .find(|candidate| candidate.file_name == file.file_name)
        .ok_or_else(|| RangeCanonicalError::MalformedStage4A {
            reason: "manifest file was not returned by Raw readback".to_owned(),
        })?;
    let document = parse_stage4a_document(&stored_file.bytes)?;
    // The returned `FileEntry` remains borrowed from the caller's manifest;
    // all bytes were verified by RawStore before this function returns.
    Ok((file, document))
}

fn parse_stage4a_document(bytes: &[u8]) -> Result<Stage4ADocument, RangeCanonicalError> {
    let raw_document: Value =
        serde_json::from_slice(bytes).map_err(|error| RangeCanonicalError::MalformedStage4A {
            reason: error.to_string(),
        })?;
    let schema_version = raw_document
        .get("schema_version")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or_default();
    let normalizer = raw_document
        .get("_lineage")
        .and_then(Value::as_object)
        .and_then(|lineage| lineage.get("normalizer"))
        .and_then(Value::as_str)
        .unwrap_or("<missing>")
        .to_owned();
    if schema_version < RANGE_NORMALIZED_SCHEMA_VERSION
        || (schema_version == 1 && normalizer.ends_with("-v1"))
    {
        return Err(RangeCanonicalError::UnsupportedLegacyStage4A {
            schema_version,
            normalizer,
        });
    }
    serde_json::from_value(raw_document).map_err(|error| RangeCanonicalError::MalformedStage4A {
        reason: error.to_string(),
    })
}

fn safe_package_root(root: &Path) -> Result<PathBuf, RangeCanonicalError> {
    if root.as_os_str().is_empty() || !root.is_absolute() {
        return Err(RangeCanonicalError::UnsafeEvidencePath {
            path: root.display().to_string(),
        });
    }
    let mut current = PathBuf::from(std::path::MAIN_SEPARATOR.to_string());
    for component in root.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(part) => {
                current.push(part);
                let metadata = fs::symlink_metadata(&current).map_err(|_| {
                    RangeCanonicalError::UnsafeEvidencePath {
                        path: current.display().to_string(),
                    }
                })?;
                if metadata.file_type().is_symlink() {
                    return Err(RangeCanonicalError::UnsafeEvidencePath {
                        path: current.display().to_string(),
                    });
                }
            }
            _ => {
                return Err(RangeCanonicalError::UnsafeEvidencePath {
                    path: root.display().to_string(),
                });
            }
        }
    }
    let metadata =
        fs::symlink_metadata(root).map_err(|_| RangeCanonicalError::UnsafeEvidencePath {
            path: root.display().to_string(),
        })?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(RangeCanonicalError::UnsafeEvidencePath {
            path: root.display().to_string(),
        });
    }
    fs::canonicalize(root).map_err(|_| RangeCanonicalError::UnsafeEvidencePath {
        path: root.display().to_string(),
    })
}

fn read_safe_file(root: &Path, relative: &Path) -> Result<Vec<u8>, RangeCanonicalError> {
    let path = safe_join(root, relative)?;
    let metadata =
        fs::symlink_metadata(&path).map_err(|_| RangeCanonicalError::EvidenceArtifact {
            path: relative.display().to_string(),
            reason: "file is missing".to_owned(),
        })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(RangeCanonicalError::UnsafeEvidencePath {
            path: path.display().to_string(),
        });
    }
    fs::read(path).map_err(|error| RangeCanonicalError::EvidenceArtifact {
        path: relative.display().to_string(),
        reason: error.to_string(),
    })
}

fn safe_join(root: &Path, relative: &Path) -> Result<PathBuf, RangeCanonicalError> {
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
    {
        return Err(RangeCanonicalError::UnsafeEvidencePath {
            path: relative.display().to_string(),
        });
    }
    let mut path = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            return Err(RangeCanonicalError::UnsafeEvidencePath {
                path: relative.display().to_string(),
            });
        };
        path.push(part);
        let metadata =
            fs::symlink_metadata(&path).map_err(|_| RangeCanonicalError::EvidenceArtifact {
                path: relative.display().to_string(),
                reason: "path component is missing".to_owned(),
            })?;
        if metadata.file_type().is_symlink() {
            return Err(RangeCanonicalError::UnsafeEvidencePath {
                path: path.display().to_string(),
            });
        }
    }
    let canonical = fs::canonicalize(&path).map_err(|_| RangeCanonicalError::EvidenceArtifact {
        path: relative.display().to_string(),
        reason: "path cannot be canonicalized".to_owned(),
    })?;
    if !canonical.starts_with(root) {
        return Err(RangeCanonicalError::UnsafeEvidencePath {
            path: canonical.display().to_string(),
        });
    }
    Ok(path)
}

fn read_artifact(
    root: &Path,
    artifact: &EvidenceArtifactRef,
) -> Result<Vec<u8>, RangeCanonicalError> {
    if artifact.schema_version != EVIDENCE_PACKAGE_SCHEMA_VERSION || artifact.size_bytes == 0 {
        return Err(RangeCanonicalError::EvidenceArtifact {
            path: artifact.path.clone(),
            reason: "unsupported schema or empty artifact".to_owned(),
        });
    }
    let bytes = read_safe_file(root, Path::new(&artifact.path))?;
    if bytes.len() as u64 != artifact.size_bytes {
        return Err(RangeCanonicalError::EvidenceArtifact {
            path: artifact.path.clone(),
            reason: "size differs from pinned manifest".to_owned(),
        });
    }
    if ContentHash::from_bytes(&bytes) != artifact.sha256 {
        return Err(RangeCanonicalError::EvidenceArtifact {
            path: artifact.path.clone(),
            reason: "content hash differs from pinned manifest".to_owned(),
        });
    }
    Ok(bytes)
}

fn map_schedule_error(error: RangeCanonicalError) -> RangeCanonicalError {
    match error {
        RangeCanonicalError::EvidenceArtifact { path, reason } => {
            RangeCanonicalError::UnsupportedHistoricalSessionSchedule {
                reason: format!("{path}: {reason}"),
            }
        }
        other => other,
    }
}

fn map_listing_error(error: RangeCanonicalError) -> RangeCanonicalError {
    match error {
        RangeCanonicalError::EvidenceArtifact { path, reason } => {
            RangeCanonicalError::MissingListingMasterEvidence {
                reason: format!("{path}: {reason}"),
            }
        }
        other => other,
    }
}

fn map_pit_error(error: RangeCanonicalError) -> RangeCanonicalError {
    match error {
        RangeCanonicalError::EvidenceArtifact { path, reason } => {
            RangeCanonicalError::NonStrictPitNotApproved {
                reason: format!("{path}: {reason}"),
            }
        }
        other => other,
    }
}

fn verify_upstream_range_manifest(
    raw: &RawStore,
    lineage: &RangeNormalizationLineage,
) -> Result<(), RangeCanonicalError> {
    let entries = raw.read_reconciled_manifest(PROVIDER_KIS_DAILY_RANGE, MARKET_KR)?;
    let entry = entries
        .into_iter()
        .find(|candidate| candidate.batch_id == lineage.upstream_batch_id)
        .ok_or_else(|| RangeCanonicalError::UpstreamManifest {
            reason: "lineage source batch is absent from the immutable Raw manifest".to_owned(),
        })?;
    let manifest_hash = ContentHash::from_bytes(&serde_json::to_vec(&entry)?);
    if manifest_hash != lineage.upstream_manifest_hash {
        return Err(RangeCanonicalError::UpstreamManifest {
            reason: "serialized source manifest hash differs from Stage4A lineage".to_owned(),
        });
    }
    if entry.mode != FetchMode::Credentialed || entry.files.is_empty() {
        return Err(RangeCanonicalError::UpstreamManifest {
            reason: "upstream range batch is not a non-empty credentialed Raw batch".to_owned(),
        });
    }
    let source_files = raw.read_batch_bytes(PROVIDER_KIS_DAILY_RANGE, MARKET_KR, &entry)?;
    if entry.files.len() != lineage.source_files.len() {
        return Err(RangeCanonicalError::UpstreamManifest {
            reason: "source file count differs from Stage4A lineage".to_owned(),
        });
    }
    for file in &entry.files {
        let Some(source) = lineage
            .source_files
            .iter()
            .find(|candidate| candidate.file_name == file.file_name)
        else {
            return Err(RangeCanonicalError::UpstreamManifest {
                reason: format!("source file {} is absent from lineage", file.file_name),
            });
        };
        if source.kind != file.kind
            || source.content_hash != file.content_hash
            || source.size_bytes != file.size_bytes
            || source.request != file.request
        {
            return Err(RangeCanonicalError::UpstreamManifest {
                reason: format!("source file metadata mismatch for {}", file.file_name),
            });
        }
        if source_files
            .iter()
            .find(|candidate| candidate.file_name == file.file_name)
            .is_none_or(|stored| stored.bytes.len() as u64 != file.size_bytes)
        {
            return Err(RangeCanonicalError::UpstreamManifest {
                reason: format!("source file readback mismatch for {}", file.file_name),
            });
        }
    }
    for row in &lineage.source_rows {
        let Some(file) = entry
            .files
            .iter()
            .find(|file| file.file_name == row.source_file_name)
        else {
            return Err(RangeCanonicalError::UpstreamManifest {
                reason: format!("row source file {} is absent", row.source_file_name),
            });
        };
        if row.source_file_hash != file.content_hash
            || row.source_file_size_bytes != file.size_bytes
            || row.source_query_start > row.source_query_end
        {
            return Err(RangeCanonicalError::UpstreamManifest {
                reason: format!("row-level source link mismatch for {}", row.symbol),
            });
        }
        let stored = source_files
            .iter()
            .find(|candidate| candidate.file_name == row.source_file_name)
            .ok_or_else(|| RangeCanonicalError::UpstreamManifest {
                reason: format!("row source file {} was not read back", row.source_file_name),
            })?;
        verify_source_row_bytes(file, stored, row)?;
    }
    Ok(())
}

/// Re-check one session's row-level lineage against an already authenticated
/// source entry/readback. The historical range verifier calls this 1,608
/// times while reading the 187 Stage5 files only once.
fn verify_upstream_range_witness(
    entry: &ManifestEntry,
    source_files: &[StoredFile],
    lineage: &RangeNormalizationLineage,
) -> Result<(), RangeCanonicalError> {
    let manifest_hash = ContentHash::from_bytes(&serde_json::to_vec(entry)?);
    if entry.batch_id != lineage.upstream_batch_id
        || manifest_hash != lineage.upstream_manifest_hash
        || entry.mode != FetchMode::Credentialed
        || entry.files.is_empty()
        || entry.files.len() != lineage.source_files.len()
    {
        return Err(RangeCanonicalError::UpstreamManifest {
            reason: "cached Stage5 source identity or file coverage differs from lineage"
                .to_owned(),
        });
    }
    for file in &entry.files {
        let Some(source) = lineage
            .source_files
            .iter()
            .find(|candidate| candidate.file_name == file.file_name)
        else {
            return Err(RangeCanonicalError::UpstreamManifest {
                reason: format!("source file {} is absent from lineage", file.file_name),
            });
        };
        if source.kind != file.kind
            || source.content_hash != file.content_hash
            || source.size_bytes != file.size_bytes
            || source.request != file.request
            || source_files
                .iter()
                .find(|candidate| candidate.file_name == file.file_name)
                .is_none_or(|stored| stored.bytes.len() as u64 != file.size_bytes)
        {
            return Err(RangeCanonicalError::UpstreamManifest {
                reason: format!(
                    "source file readback or metadata mismatch for {}",
                    file.file_name
                ),
            });
        }
    }
    for row in &lineage.source_rows {
        let file = entry
            .files
            .iter()
            .find(|file| file.file_name == row.source_file_name)
            .ok_or_else(|| RangeCanonicalError::UpstreamManifest {
                reason: format!("row source file {} is absent", row.source_file_name),
            })?;
        if row.source_file_hash != file.content_hash
            || row.source_file_size_bytes != file.size_bytes
            || row.source_query_start > row.source_query_end
        {
            return Err(RangeCanonicalError::UpstreamManifest {
                reason: format!("row-level source link mismatch for {}", row.symbol),
            });
        }
        let stored = source_files
            .iter()
            .find(|candidate| candidate.file_name == row.source_file_name)
            .ok_or_else(|| RangeCanonicalError::UpstreamManifest {
                reason: format!("row source file {} was not read back", row.source_file_name),
            })?;
        verify_source_row_bytes(file, stored, row)?;
    }
    Ok(())
}

fn verify_source_row_bytes(
    metadata: &FileEntry,
    stored: &StoredFile,
    row: &crate::range_normalize::RangeNormalizationSourceRow,
) -> Result<(), RangeCanonicalError> {
    let query = query_map(&metadata.request.query)?;
    let expected_symbol =
        query
            .get("FID_INPUT_ISCD")
            .ok_or_else(|| RangeCanonicalError::UpstreamManifest {
                reason: format!("{} lacks the source symbol query", metadata.file_name),
            })?;
    if expected_symbol != &row.symbol {
        return Err(RangeCanonicalError::UpstreamManifest {
            reason: format!("row symbol does not match {} query", metadata.file_name),
        });
    }
    let expected_start = query
        .get("FID_INPUT_DATE_1")
        .and_then(|value| parse_kis_date(value))
        .ok_or_else(|| RangeCanonicalError::UpstreamManifest {
            reason: format!("{} lacks a valid source start query", metadata.file_name),
        })?;
    let expected_end = query
        .get("FID_INPUT_DATE_2")
        .and_then(|value| parse_kis_date(value))
        .ok_or_else(|| RangeCanonicalError::UpstreamManifest {
            reason: format!("{} lacks a valid source end query", metadata.file_name),
        })?;
    if expected_start != row.source_query_start || expected_end != row.source_query_end {
        return Err(RangeCanonicalError::UpstreamManifest {
            reason: format!("row query bounds do not match {}", metadata.file_name),
        });
    }
    let expected_query = BTreeMap::from([
        ("FID_COND_MRKT_DIV_CODE".to_owned(), "J".to_owned()),
        ("FID_INPUT_ISCD".to_owned(), row.symbol.clone()),
        (
            "FID_INPUT_DATE_1".to_owned(),
            kis_date(row.source_query_start),
        ),
        (
            "FID_INPUT_DATE_2".to_owned(),
            kis_date(row.source_query_end),
        ),
        ("FID_PERIOD_DIV_CODE".to_owned(), "D".to_owned()),
        ("FID_ORG_ADJ_PRC".to_owned(), "1".to_owned()),
    ]);
    if query != expected_query {
        return Err(RangeCanonicalError::UpstreamManifest {
            reason: format!(
                "{} is not the exact original-price daily-bar request",
                metadata.file_name
            ),
        });
    }
    let document: Value = serde_json::from_slice(&stored.bytes).map_err(|error| {
        RangeCanonicalError::UpstreamManifest {
            reason: format!("{} source JSON is malformed: {error}", metadata.file_name),
        }
    })?;
    let output1_symbol = document
        .get("output1")
        .and_then(Value::as_object)
        .and_then(|object| object.get("stck_shrn_iscd"))
        .and_then(Value::as_str)
        .ok_or_else(|| RangeCanonicalError::UpstreamManifest {
            reason: format!("{} lacks output1 symbol identity", metadata.file_name),
        })?;
    if output1_symbol != expected_symbol {
        return Err(RangeCanonicalError::UpstreamManifest {
            reason: format!(
                "{} output1 symbol differs from its request",
                metadata.file_name
            ),
        });
    }
    let rows = document
        .get("output2")
        .and_then(Value::as_array)
        .ok_or_else(|| RangeCanonicalError::UpstreamManifest {
            reason: format!("{} lacks output2 rows", metadata.file_name),
        })?;
    let mut date_matches = 0usize;
    let mut hash_matches = 0usize;
    for candidate in rows {
        let Some(object) = candidate.as_object() else {
            return Err(RangeCanonicalError::UpstreamManifest {
                reason: format!("{} output2 contains a non-object row", metadata.file_name),
            });
        };
        let Some(date) = object
            .get("stck_bsop_date")
            .and_then(Value::as_str)
            .and_then(parse_kis_date)
        else {
            return Err(RangeCanonicalError::UpstreamManifest {
                reason: format!(
                    "{} output2 contains an invalid row date",
                    metadata.file_name
                ),
            });
        };
        if date != row.row_date {
            continue;
        }
        date_matches += 1;
        let bytes = serde_json::to_vec(&Value::Object(object.clone()))?;
        if ContentHash::from_bytes(&bytes) == row.row_content_hash
            && bytes.len() as u64 == row.row_size_bytes
        {
            hash_matches += 1;
        }
    }
    if date_matches != 1 || hash_matches != 1 {
        return Err(RangeCanonicalError::UpstreamManifest {
            reason: format!(
                "{} row {} is not one unique output2 row with matching hash/size",
                metadata.file_name, row.row_date,
            ),
        });
    }
    Ok(())
}

fn load_action_evidence(
    raw: &RawStore,
    refs: &[EvidenceActionRef],
    range_start: TradingDate,
    range_end: TradingDate,
) -> Result<ActionCoverageEvidence, RangeCanonicalError> {
    let entries = raw.read_reconciled_manifest(PROVIDER_KIS, MARKET_KR)?;
    load_action_evidence_from_entries(raw, &entries, refs, range_start, range_end)
}

fn load_action_evidence_from_entries(
    raw: &RawStore,
    entries: &[ManifestEntry],
    refs: &[EvidenceActionRef],
    range_start: TradingDate,
    range_end: TradingDate,
) -> Result<ActionCoverageEvidence, RangeCanonicalError> {
    if refs.len() != REQUIRED_ACTION_KINDS.len() {
        return Err(RangeCanonicalError::MissingActionEvidence {
            reason: "package must pin exactly one Raw file for each of the seven KSD classes"
                .to_owned(),
        });
    }
    let required = REQUIRED_ACTION_KINDS
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let actual = refs
        .iter()
        .map(|item| item.kind.as_str())
        .collect::<BTreeSet<_>>();
    if actual != required || actual.len() != refs.len() {
        return Err(RangeCanonicalError::MissingActionEvidence {
            reason: "action package kinds are not the exact seven KSD classes".to_owned(),
        });
    }
    let raw_batch_id = refs[0].raw_batch_id;
    let raw_manifest_hash = refs[0].raw_manifest_hash.clone();
    if refs.iter().any(|item| {
        item.raw_batch_id != raw_batch_id || item.raw_manifest_hash != raw_manifest_hash
    }) {
        return Err(RangeCanonicalError::MissingActionEvidence {
            reason: "all action files must come from one pinned immutable Raw manifest".to_owned(),
        });
    }
    let entry = entries
        .iter()
        .find(|candidate| candidate.batch_id == raw_batch_id)
        .ok_or_else(|| RangeCanonicalError::ActionEvidence {
            reason: "pinned KIS action Raw batch is absent".to_owned(),
        })?;
    let actual_manifest_hash = ContentHash::from_bytes(&serde_json::to_vec(&entry)?);
    if actual_manifest_hash != raw_manifest_hash {
        return Err(RangeCanonicalError::ActionEvidence {
            reason: "pinned KIS action Raw manifest hash differs".to_owned(),
        });
    }
    if entry.mode != FetchMode::Credentialed || entry.files.len() != refs.len() {
        return Err(RangeCanonicalError::ActionEvidence {
            reason: "action Raw batch has unexpected mode or file count".to_owned(),
        });
    }
    let stored = raw.read_batch_bytes(PROVIDER_KIS, MARKET_KR, entry)?;
    let mut files = Vec::with_capacity(refs.len());
    let mut actions = Vec::new();
    for item in refs {
        let Some(metadata) = entry
            .files
            .iter()
            .find(|file| file.file_name == item.raw_file_name)
        else {
            return Err(RangeCanonicalError::ActionEvidence {
                reason: format!(
                    "action file {} is absent from Raw manifest",
                    item.raw_file_name
                ),
            });
        };
        if metadata.kind != ResponseKind::CorporateActions
            || metadata.content_hash != item.content_hash
            || metadata.size_bytes != item.size_bytes
        {
            return Err(RangeCanonicalError::ActionEvidence {
                reason: format!(
                    "action file metadata/hash mismatch for {}",
                    item.raw_file_name
                ),
            });
        }
        let bytes = stored
            .iter()
            .find(|file| file.file_name == item.raw_file_name)
            .ok_or_else(|| RangeCanonicalError::ActionEvidence {
                reason: format!("action file {} was not read back", item.raw_file_name),
            })?;
        let spec = action_spec(&item.kind).ok_or_else(|| RangeCanonicalError::ActionEvidence {
            reason: format!("action kind {} is not allowlisted", item.kind),
        })?;
        validate_action_request(metadata, &item.kind, &spec, range_start, range_end)?;
        let document: Value = serde_json::from_slice(&bytes.bytes).map_err(|error| {
            RangeCanonicalError::ActionEvidence {
                reason: format!(
                    "action response {} is malformed: {error}",
                    item.raw_file_name
                ),
            }
        })?;
        let object = document
            .as_object()
            .ok_or_else(|| RangeCanonicalError::ActionEvidence {
                reason: format!("action response {} is not an object", item.raw_file_name),
            })?;
        if object.get("rt_cd").and_then(Value::as_str) != Some("0") {
            return Err(RangeCanonicalError::ActionEvidence {
                reason: format!("action response {} is not rt_cd=0", item.raw_file_name),
            });
        }
        validate_terminal_action_response(&item.kind, object)?;
        let rows = object
            .get("output1")
            .and_then(Value::as_array)
            .ok_or_else(|| RangeCanonicalError::ActionEvidence {
                reason: format!("action response {} lacks array output1", item.raw_file_name),
            })?;
        if !rows.is_empty() {
            if item.kind == "bonus-issue" {
                for row in rows {
                    actions.push(parse_bonus_action(row, entry.retrieved_at)?);
                }
            } else {
                actions.push(RangeAction::Unsupported {
                    kind: item.kind.clone(),
                    reason: "nonempty KSD action output has no reviewed canonical mapping"
                        .to_owned(),
                });
            }
        }
        files.push(ActionCoverageFileEvidence {
            kind: item.kind.clone(),
            endpoint: metadata.request.endpoint.clone(),
            file_name: metadata.file_name.clone(),
            content_hash: metadata.content_hash.clone(),
            size_bytes: metadata.size_bytes,
            query_start: range_start,
            query_end: range_end,
        });
    }
    Ok(ActionCoverageEvidence::new(
        range_start,
        range_end,
        raw_batch_id,
        raw_manifest_hash,
        files,
        actions,
        entry.retrieved_at,
    ))
}

type ActionSpec = KisActionSpec;

fn action_spec(kind: &str) -> Option<ActionSpec> {
    kis_action_spec(kind)
}

fn expected_action_query(
    spec: &ActionSpec,
    range_start: TradingDate,
    range_end: TradingDate,
) -> BTreeMap<String, String> {
    let mut expected = BTreeMap::from([
        ("CTS".to_owned(), String::new()),
        ("F_DT".to_owned(), kis_date(range_start)),
        ("T_DT".to_owned(), kis_date(range_end)),
        ("SHT_CD".to_owned(), String::new()),
    ]);
    for (key, value) in spec.extra {
        expected.insert((*key).to_owned(), (*value).to_owned());
    }
    expected
}

fn validate_action_request(
    metadata: &FileEntry,
    kind: &str,
    spec: &ActionSpec,
    range_start: TradingDate,
    range_end: TradingDate,
) -> Result<(), RangeCanonicalError> {
    if metadata.request.endpoint != spec.path || metadata.request.mode != FetchMode::Credentialed {
        return Err(RangeCanonicalError::ActionEvidence {
            reason: format!(
                "{} endpoint/mode is not the allowlisted KSD contract",
                metadata.file_name
            ),
        });
    }
    let query = query_map(&metadata.request.query)?;
    let expected = expected_action_query(spec, range_start, range_end);
    if query != expected {
        return Err(RangeCanonicalError::ActionEvidence {
            reason: format!(
                "{} query differs from exact KSD range contract",
                metadata.file_name
            ),
        });
    }
    let tr_ids = metadata
        .request
        .headers
        .iter()
        .filter(|(key, _)| key.eq_ignore_ascii_case("tr_id"))
        .map(|(_, value)| value.as_str())
        .collect::<Vec<_>>();
    if tr_ids.as_slice() != [spec.tr_id] {
        return Err(RangeCanonicalError::ActionEvidence {
            reason: format!("{} tr_id differs from allowlist", metadata.file_name),
        });
    }
    let continuations = metadata
        .request
        .headers
        .iter()
        .filter(|(key, _)| key.eq_ignore_ascii_case("tr_cont"))
        .map(|(_, value)| value.as_str())
        .collect::<Vec<_>>();
    if continuations.as_slice() != [""] {
        return Err(RangeCanonicalError::IncompleteActionPagination {
            kind: kind.to_owned(),
            marker: "tr_cont".to_owned(),
        });
    }
    Ok(())
}

fn validate_terminal_action_response(
    kind: &str,
    object: &Map<String, Value>,
) -> Result<(), RangeCanonicalError> {
    for (key, value) in object {
        let normalized_key = key.to_ascii_lowercase();
        if !ACTION_CONTINUATION_FIELDS.contains(&normalized_key.as_str()) {
            continue;
        }
        let nonempty = match value {
            Value::Null => false,
            Value::String(value) => !value.trim().is_empty(),
            // A continuation field has an endpoint-defined string shape;
            // empty arrays/objects are not an attested terminal value.
            Value::Array(_) | Value::Object(_) | Value::Bool(_) | Value::Number(_) => true,
        };
        if nonempty {
            return Err(RangeCanonicalError::IncompleteActionPagination {
                kind: kind.to_owned(),
                marker: key.clone(),
            });
        }
    }
    Ok(())
}

fn parse_bonus_action(
    row: &Value,
    available_at: UtcTimestamp,
) -> Result<RangeAction, RangeCanonicalError> {
    let object = row
        .as_object()
        .ok_or_else(|| RangeCanonicalError::ActionEvidence {
            reason: "bonus output1 row is not an object".to_owned(),
        })?;
    let symbol = required_action_string(object, "sht_cd")?;
    let instrument_id = InstrumentId::from_parts(symbol, Venue::Krx).map_err(|_| {
        RangeCanonicalError::ActionEvidence {
            reason: "bonus action has an invalid canonical symbol".to_owned(),
        }
    })?;
    let record_date = parse_action_date(object, "record_date")?;
    let ex_date = parse_action_date(object, "right_dt")?;
    let rate = FixedPoint::parse(required_action_string(object, "fix_rate")?).map_err(|_| {
        RangeCanonicalError::ActionEvidence {
            reason: "bonus action fix_rate is not an exact decimal".to_owned(),
        }
    })?;
    let split_factor =
        bonus_split_factor_from_percent(rate).map_err(|_| RangeCanonicalError::ActionEvidence {
            reason: "bonus action split factor overflows".to_owned(),
        })?;
    if split_factor <= FixedPoint::parse("1").expect("one") {
        return Err(RangeCanonicalError::UnsupportedAction {
            kind: "bonus-issue".to_owned(),
        });
    }
    Ok(RangeAction::BonusIssue {
        instrument_id,
        record_date,
        ex_date,
        split_factor,
        available_at,
    })
}

fn required_action_string<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<&'a str, RangeCanonicalError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.trim() == *value)
        .ok_or_else(|| RangeCanonicalError::ActionEvidence {
            reason: format!("bonus action field {field} is missing or malformed"),
        })
}

fn parse_action_date(
    object: &Map<String, Value>,
    field: &str,
) -> Result<TradingDate, RangeCanonicalError> {
    let value = required_action_string(object, field)?;
    let normalized = if value.len() == 8 && value.bytes().all(|byte| byte.is_ascii_digit()) {
        format!("{}-{}-{}", &value[..4], &value[4..6], &value[6..])
    } else {
        value.to_owned()
    };
    TradingDate::parse(&normalized).map_err(|_| RangeCanonicalError::ActionEvidence {
        reason: format!("bonus action field {field} is not a valid date"),
    })
}

fn kis_date(date: TradingDate) -> String {
    date.to_iso().replace('-', "")
}

fn parse_kis_date(value: &str) -> Option<TradingDate> {
    if value.len() == 8 && value.bytes().all(|byte| byte.is_ascii_digit()) {
        TradingDate::parse(&format!("{}-{}-{}", &value[..4], &value[4..6], &value[6..])).ok()
    } else {
        TradingDate::parse(value).ok()
    }
}

/// Build one in-memory candidate from exactly one Stage4A session batch.
pub fn build_range_canonical_candidate(
    raw: &RawStore,
    entry: &ManifestEntry,
    evidence: &VerifiedRangeCanonicalEvidence,
) -> Result<RangeCanonicalCandidate, RangeCanonicalError> {
    validate_scope(entry)?;
    let file = validate_entry(entry)?;
    let stored = raw.read_batch_bytes(&entry.provider, &entry.market, entry)?;
    let stored_file = stored
        .iter()
        .find(|candidate| candidate.file_name == file.file_name)
        .ok_or_else(|| RangeCanonicalError::MalformedStage4A {
            reason: "manifest file was not returned by Raw readback".to_owned(),
        })?;
    let document = parse_stage4a_document(&stored_file.bytes)?;
    validate_document(entry, file, &document)?;
    validate_lineage(entry, file, &document.lineage)?;

    let schedule = &evidence.schedule;
    validate_schedule(schedule, entry.date)?;
    if schedule.calendar_id != document.lineage.calendar_id
        || schedule.calendar_hash != document.lineage.calendar_hash
    {
        return Err(RangeCanonicalError::UnsupportedHistoricalSessionSchedule {
            reason: "schedule identity differs from Stage4A lineage".to_owned(),
        });
    }

    let listing = &evidence.listing;
    validate_listing(listing, entry.date, entry.retrieved_at)?;
    if listing.snapshot_id != document.lineage.listing_snapshot_id
        || listing.snapshot_hash != document.lineage.listing_snapshot_hash
    {
        return Err(RangeCanonicalError::MissingListingMasterEvidence {
            reason: "listing snapshot identity differs from Stage4A lineage".to_owned(),
        });
    }

    let actions = &evidence.actions;
    validate_actions(
        actions,
        document.lineage.source_start,
        document.lineage.source_end,
        entry.date,
    )?;

    let pit_policy = &evidence.pit_policy;
    validate_pit_policy(pit_policy)?;

    let bars = parse_bars(&document.bars, entry.date)?;
    let source_entry_hash = ContentHash::from_bytes(&serde_json::to_vec(entry)?);
    let source_file_hash = file.content_hash.clone();
    let candidate_id = deterministic_candidate_id(
        entry,
        &source_entry_hash,
        &source_file_hash,
        &document.lineage,
        schedule,
        listing,
        actions,
        pit_policy,
        &evidence.manifest_hash,
        &evidence.schedule_artifact_hash,
        &evidence.listing_artifact_hash,
        &evidence.pit_policy_artifact_hash,
    );

    Ok(RangeCanonicalCandidate {
        candidate_id,
        bridge_version: RANGE_CANONICAL_BRIDGE_VERSION.to_owned(),
        evidence_manifest_hash: evidence.manifest_hash.clone(),
        schedule_artifact_hash: evidence.schedule_artifact_hash.clone(),
        listing_artifact_hash: evidence.listing_artifact_hash.clone(),
        pit_policy_artifact_hash: evidence.pit_policy_artifact_hash.clone(),
        source_batch_id: entry.batch_id,
        source_entry_hash,
        source_file_hash,
        upstream_range_batch_id: document.lineage.upstream_batch_id,
        upstream_range_manifest_hash: document.lineage.upstream_manifest_hash.clone(),
        session_date: entry.date,
        // The Stage4A acquisition time is the only safe visibility instant.
        acquired_at: document.acquired_at,
        bars,
        schedule: schedule.clone(),
        listing: listing.clone(),
        actions: actions
            .actions
            .iter()
            .filter(|action| match action {
                RangeAction::BonusIssue { record_date, .. } => *record_date == entry.date,
                RangeAction::Unsupported { .. } => true,
            })
            .cloned()
            .collect(),
        action_coverage: actions.clone(),
        pit_policy: pit_policy.clone(),
    })
}

fn approved_historical_beta_scope() -> Result<HistoricalBetaVerificationScope, RangeCanonicalError>
{
    let range_start = TradingDate::parse(HISTORICAL_PRICE_ONLY_BETA_START)
        .expect("checked-in beta start date is valid");
    let range_end = TradingDate::parse(HISTORICAL_PRICE_ONLY_BETA_END)
        .expect("checked-in beta end date is valid");
    let expected =
        ExpectedRangeSessions::approved_xkrx(range_start, range_end).map_err(|error| {
            RangeCanonicalError::HistoricalBetaContract {
                reason: format!("approved XKRX session selection is unavailable: {error}"),
            }
        })?;
    if expected.sessions.len() != HISTORICAL_PRICE_ONLY_BETA_SESSION_COUNT {
        return Err(RangeCanonicalError::HistoricalBetaContract {
            reason: format!(
                "checked-in XKRX selection has {} sessions, expected {}",
                expected.sessions.len(),
                HISTORICAL_PRICE_ONLY_BETA_SESSION_COUNT
            ),
        });
    }
    Ok(HistoricalBetaVerificationScope {
        source_batch_id: HISTORICAL_PRICE_ONLY_BETA_SOURCE_BATCH_ID
            .parse()
            .expect("checked-in Stage5 batch id is valid"),
        range_start,
        range_end,
        sessions: expected.sessions,
        calendar_id: expected.calendar_id,
        calendar_hash: expected.calendar_hash,
        listing_snapshot_id: expected.listing_snapshot_id,
        listing_snapshot_hash: expected.listing_snapshot_hash,
    })
}

fn historical_beta_contract_error(reason: &'static str) -> RangeCanonicalError {
    RangeCanonicalError::HistoricalBetaContract {
        reason: reason.to_owned(),
    }
}

fn has_fixed_stage5_headers(request: &RequestMetadata) -> bool {
    let mut seen = BTreeSet::new();
    let mut has_tr_id = false;
    let mut has_tr_cont = false;
    for (key, value) in &request.headers {
        let normalized = key.to_ascii_lowercase();
        if !matches!(
            normalized.as_str(),
            "authorization" | "appkey" | "appsecret" | "tr_id" | "tr_cont"
        ) || !seen.insert(normalized.clone())
        {
            return false;
        }
        match normalized.as_str() {
            "tr_id" => {
                if value != DAILY_RANGE_TR_ID {
                    return false;
                }
                has_tr_id = true;
            }
            "tr_cont" => {
                if !value.is_empty() {
                    return false;
                }
                has_tr_cont = true;
            }
            "authorization" | "appkey" | "appsecret" => {
                if value != "[REDACTED]" {
                    return false;
                }
            }
            _ => unreachable!("header allowlist checked above"),
        }
    }
    seen.len() == 5 && has_tr_id && has_tr_cont
}

fn validate_fixed_stage5_source_metadata(
    entry: &ManifestEntry,
    range_start: TradingDate,
    range_end: TradingDate,
) -> Result<(), RangeCanonicalError> {
    if entry.provider != PROVIDER_KIS_DAILY_RANGE
        || entry.market != MARKET_KR
        || entry.mode != FetchMode::Credentialed
        || entry.date != range_start
        || entry.files.len() != HISTORICAL_PRICE_ONLY_BETA_SOURCE_FILE_COUNT
    {
        return Err(historical_beta_contract_error(
            "fixed Stage5 source manifest scope, mode, or file count is invalid",
        ));
    }

    let expected_start = kis_date(range_start);
    let expected_end = kis_date(range_end);
    for (symbol_index, symbol) in KR_ETF_CORE_SYMBOLS.iter().enumerate() {
        let mut previous_window_end = None;
        for window in 1..=HISTORICAL_PRICE_ONLY_BETA_WINDOW_COUNT {
            let file_index = symbol_index * HISTORICAL_PRICE_ONLY_BETA_WINDOW_COUNT + window - 1;
            let file = entry.files.get(file_index).ok_or_else(|| {
                historical_beta_contract_error("fixed Stage5 source file order is incomplete")
            })?;
            let expected_file_name =
                format!("daily-bars-range-window-{window}-{symbol}-page-01.json");
            if file.file_name != expected_file_name
                || file.kind != ResponseKind::Bars
                || file.request.mode != FetchMode::Credentialed
                || file.request.endpoint != DAILY_RANGE_ENDPOINT
                || !has_fixed_stage5_headers(&file.request)
            {
                return Err(historical_beta_contract_error(
                    "fixed Stage5 source file metadata is outside the contract",
                ));
            }

            let query = query_map(&file.request.query).map_err(|_| {
                historical_beta_contract_error("fixed Stage5 source query metadata is invalid")
            })?;
            if query.len() != STAGE5_QUERY_KEYS.len()
                || STAGE5_QUERY_KEYS
                    .iter()
                    .any(|key| !query.contains_key(*key))
                || query.get("FID_COND_MRKT_DIV_CODE").map(String::as_str) != Some("J")
                || query.get("FID_INPUT_ISCD").map(String::as_str) != Some(*symbol)
                || query.get("FID_INPUT_DATE_1").map(String::as_str)
                    != Some(expected_start.as_str())
                || query.get("FID_PERIOD_DIV_CODE").map(String::as_str) != Some("D")
                || query.get("FID_ORG_ADJ_PRC").map(String::as_str) != Some("1")
            {
                return Err(historical_beta_contract_error(
                    "fixed Stage5 source query keys or values are invalid",
                ));
            }
            let Some(end_value) = query.get("FID_INPUT_DATE_2") else {
                return Err(historical_beta_contract_error(
                    "fixed Stage5 source end-date query is missing",
                ));
            };
            if end_value.len() != 8 || !end_value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(historical_beta_contract_error(
                    "fixed Stage5 source end-date query is not compact",
                ));
            }
            let Some(window_end) = parse_kis_date(end_value) else {
                return Err(historical_beta_contract_error(
                    "fixed Stage5 source end-date query is not a date",
                ));
            };
            if window_end < range_start || window_end > range_end {
                return Err(historical_beta_contract_error(
                    "fixed Stage5 source end-date query is outside the beta range",
                ));
            }
            if window == 1 {
                if end_value != &expected_end {
                    return Err(historical_beta_contract_error(
                        "fixed Stage5 first window does not end at the beta end",
                    ));
                }
            } else if previous_window_end.is_some_and(|previous| window_end >= previous) {
                return Err(historical_beta_contract_error(
                    "fixed Stage5 window end dates are not strictly decreasing",
                ));
            }
            previous_window_end = Some(window_end);
        }
    }
    Ok(())
}

fn find_fixed_stage5_source(
    entries: &[ManifestEntry],
    expected_batch_id: BatchId,
    range_start: TradingDate,
    range_end: TradingDate,
) -> Result<&ManifestEntry, RangeCanonicalError> {
    let mut fixed_entries = entries
        .iter()
        .filter(|entry| entry.batch_id == expected_batch_id);
    let entry = fixed_entries.next().ok_or_else(|| {
        historical_beta_contract_error("the contractual Stage5 source batch is absent")
    })?;
    if fixed_entries.next().is_some() {
        return Err(historical_beta_contract_error(
            "the contractual Stage5 source batch is not unique",
        ));
    }
    validate_fixed_stage5_source_metadata(entry, range_start, range_end)?;
    Ok(entry)
}

/// Discover candidate pins from committed manifest metadata only.
///
/// This function never calls [`RawStore::read_batch_bytes`], never parses a
/// response body, and never writes or reconciles Raw.  Its result is an
/// unapproved metadata candidate; the explicit verifier remains the second
/// owner-reviewed step.
pub fn discover_historical_price_only_beta_pins(
    raw: &RawStore,
) -> Result<HistoricalPriceOnlyBetaPins, RangeCanonicalError> {
    let scope = approved_historical_beta_scope()?;
    let source_entries = raw.read_committed_manifest(PROVIDER_KIS_DAILY_RANGE, MARKET_KR)?;
    let source_entry = find_fixed_stage5_source(
        &source_entries,
        scope.source_batch_id,
        scope.range_start,
        scope.range_end,
    )?;
    let source_manifest_hash = ContentHash::from_bytes(&serde_json::to_vec(source_entry)?);

    let action_entries = raw.read_committed_manifest(PROVIDER_KIS, MARKET_KR)?;
    let action_refs = find_matching_action_evidence_in_entries(
        &action_entries,
        scope.range_start,
        scope.range_end,
    )?;
    let action_pin =
        action_refs
            .first()
            .ok_or_else(|| RangeCanonicalError::MissingActionEvidence {
                reason: "the matching KSD action candidate has no files".to_owned(),
            })?;

    Ok(HistoricalPriceOnlyBetaPins {
        contract: HISTORICAL_PRICE_ONLY_BETA_CONTRACT,
        range_start: scope.range_start,
        range_end: scope.range_end,
        source_batch_id: source_entry.batch_id,
        source_manifest_hash,
        source_file_count: source_entry.files.len(),
        action_batch_id: action_pin.raw_batch_id,
        action_manifest_hash: action_pin.raw_manifest_hash.clone(),
        action_file_count: action_refs.len(),
    })
}

/// Authenticate the exact owner-beta Stage5 and KSD inputs from immutable Raw.
///
/// Both hashes are independently reviewed pins; discovering a convenient
/// batch is not enough. The verifier re-reads the pinned Stage5 files once,
/// proves all 1,608 deterministic Stage4A sessions and their 11 row-level
/// source witnesses, and revalidates the exact seven-file KSD batch. It makes
/// no listing-interval, intraday-schedule, strict-PIT, or total-return claim.
pub fn verify_historical_price_only_beta_input(
    raw: &RawStore,
    approved_stage5_manifest_hash: &ContentHash,
    approved_action_manifest_hash: &ContentHash,
) -> Result<HistoricalPriceOnlyBetaInput, RangeCanonicalError> {
    let verified = verify_historical_price_only_beta_input_for_scope(
        raw,
        approved_stage5_manifest_hash,
        approved_action_manifest_hash,
        &approved_historical_beta_scope()?,
    )?;
    if verified.source_files.len() != HISTORICAL_PRICE_ONLY_BETA_SOURCE_FILE_COUNT
        || verified.sessions.len() != HISTORICAL_PRICE_ONLY_BETA_SESSION_COUNT
        || verified.bars.len()
            != HISTORICAL_PRICE_ONLY_BETA_SESSION_COUNT * KR_ETF_CORE_SYMBOLS.len()
        || verified.action_files.len() != REQUIRED_ACTION_KINDS.len()
    {
        return Err(RangeCanonicalError::HistoricalBetaContract {
            reason: "verified Raw witness counts differ from the fixed owner-beta contract"
                .to_owned(),
        });
    }
    Ok(verified)
}

fn verify_historical_price_only_beta_input_for_scope(
    raw: &RawStore,
    approved_stage5_manifest_hash: &ContentHash,
    approved_action_manifest_hash: &ContentHash,
    scope: &HistoricalBetaVerificationScope,
) -> Result<HistoricalPriceOnlyBetaInput, RangeCanonicalError> {
    if scope.sessions.is_empty()
        || scope.sessions.first() != Some(&scope.range_start)
        || scope.sessions.last() != Some(&scope.range_end)
        || scope.sessions.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(RangeCanonicalError::HistoricalBetaContract {
            reason: "verification scope is empty, unsorted, duplicated, or range-mismatched"
                .to_owned(),
        });
    }

    let source_entries = raw.read_committed_manifest(PROVIDER_KIS_DAILY_RANGE, MARKET_KR)?;
    let mut source_matches = source_entries
        .into_iter()
        .filter(|entry| entry.batch_id == scope.source_batch_id);
    let source_entry =
        source_matches
            .next()
            .ok_or_else(|| RangeCanonicalError::HistoricalBetaContract {
                reason: "approved Stage5 batch is absent from immutable Raw".to_owned(),
            })?;
    if source_matches.next().is_some() {
        return Err(RangeCanonicalError::HistoricalBetaContract {
            reason: "approved Stage5 batch is not unique".to_owned(),
        });
    }
    let source_manifest_hash = ContentHash::from_bytes(&serde_json::to_vec(&source_entry)?);
    if &source_manifest_hash != approved_stage5_manifest_hash
        || source_entry.provider != PROVIDER_KIS_DAILY_RANGE
        || source_entry.market != MARKET_KR
        || source_entry.mode != FetchMode::Credentialed
        || source_entry.files.is_empty()
    {
        return Err(RangeCanonicalError::HistoricalBetaContract {
            reason: "Stage5 source pin, scope, mode, or file coverage is invalid".to_owned(),
        });
    }
    let source_files = raw.read_batch_bytes(PROVIDER_KIS_DAILY_RANGE, MARKET_KR, &source_entry)?;

    let normalized_entries =
        raw.read_committed_manifest(PROVIDER_KIS_DAILY_RANGE_NORMALIZED, MARKET_KR)?;
    let normalized_by_id = normalized_entries
        .into_iter()
        .map(|entry| (entry.batch_id, entry))
        .collect::<BTreeMap<_, _>>();
    let expected_instruments = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| format!("{symbol}.KRX"))
        .collect::<BTreeSet<_>>();
    let mut sessions = Vec::with_capacity(scope.sessions.len());
    let mut bars = Vec::with_capacity(scope.sessions.len() * KR_ETF_CORE_SYMBOLS.len());

    for session in &scope.sessions {
        let normalized_batch_id = deterministic_range_normalized_batch_id_with_identity(
            &source_entry,
            &source_manifest_hash,
            *session,
            &scope.calendar_hash,
            &scope.listing_snapshot_hash,
        );
        let entry = normalized_by_id.get(&normalized_batch_id).ok_or_else(|| {
            RangeCanonicalError::HistoricalBetaContract {
                reason: format!("deterministic normalized session {session} is missing"),
            }
        })?;
        validate_scope(entry)?;
        let (file, document) = read_stage4a_document(raw, entry)?;
        validate_entry(entry)?;
        validate_document(entry, file, &document)?;
        validate_lineage(entry, file, &document.lineage)?;
        if entry.date != *session
            || document.lineage.upstream_batch_id != scope.source_batch_id
            || document.lineage.upstream_manifest_hash != source_manifest_hash
            || document.lineage.source_start != scope.range_start
            || document.lineage.source_end != scope.range_end
            || document.lineage.calendar_id != scope.calendar_id
            || document.lineage.calendar_hash != scope.calendar_hash
            || document.lineage.listing_snapshot_id != scope.listing_snapshot_id
            || document.lineage.listing_snapshot_hash != scope.listing_snapshot_hash
        {
            return Err(RangeCanonicalError::HistoricalBetaContract {
                reason: format!("normalized session {session} escapes the pinned Stage5 scope"),
            });
        }
        verify_upstream_range_witness(&source_entry, &source_files, &document.lineage)?;
        let mut session_bars = parse_bars(&document.bars, *session)?;
        let actual_instruments = session_bars
            .iter()
            .map(|bar| bar.instrument_id.as_str())
            .collect::<BTreeSet<_>>();
        if actual_instruments != expected_instruments
            || session_bars.len() != KR_ETF_CORE_SYMBOLS.len()
        {
            return Err(RangeCanonicalError::HistoricalBetaContract {
                reason: format!("normalized session {session} does not contain exactly ETF11"),
            });
        }
        session_bars.sort_by(|left, right| left.instrument_id.cmp(&right.instrument_id));
        let normalized_entry_hash = ContentHash::from_bytes(&serde_json::to_vec(entry)?);
        sessions.push(HistoricalPriceOnlySessionWitness {
            session_date: *session,
            normalized_batch_id,
            normalized_entry_hash,
            normalized_bars_hash: file.content_hash.clone(),
            acquired_at: document.acquired_at,
        });
        bars.extend(session_bars);
    }

    let action_coverage = load_pinned_action_coverage(
        raw,
        approved_action_manifest_hash,
        scope.range_start,
        scope.range_end,
    )?;
    validate_actions(
        &action_coverage,
        scope.range_start,
        scope.range_end,
        scope.range_start,
    )?;

    Ok(HistoricalPriceOnlyBetaInput {
        range_start: scope.range_start,
        range_end: scope.range_end,
        source_batch_id: source_entry.batch_id,
        source_manifest_hash,
        source_files: source_entry.files,
        action_batch_id: action_coverage.raw_batch_id,
        action_manifest_hash: action_coverage.raw_manifest_hash.clone(),
        action_files: action_coverage.files.clone(),
        sessions,
        bars,
        actions: action_coverage.actions,
    })
}

fn validate_scope(entry: &ManifestEntry) -> Result<(), RangeCanonicalError> {
    if entry.provider != PROVIDER_KIS_DAILY_RANGE_NORMALIZED || entry.market != MARKET_KR {
        return Err(RangeCanonicalError::UnsupportedScope {
            expected_provider: PROVIDER_KIS_DAILY_RANGE_NORMALIZED,
            expected_market: MARKET_KR,
            provider: entry.provider.clone(),
            market: entry.market.clone(),
        });
    }
    if entry.mode != FetchMode::Credentialed {
        return Err(RangeCanonicalError::UnsupportedMode);
    }
    Ok(())
}

fn validate_entry(entry: &ManifestEntry) -> Result<&FileEntry, RangeCanonicalError> {
    if entry.files.len() != 1 {
        return Err(RangeCanonicalError::MalformedStage4A {
            reason: "a session batch must contain exactly one bars file".to_owned(),
        });
    }
    let file = &entry.files[0];
    let expected_name = format!("bars-{}.json", entry.date);
    if file.kind != ResponseKind::Bars || file.file_name != expected_name {
        return Err(RangeCanonicalError::MalformedStage4A {
            reason: format!("expected Bars/{expected_name}"),
        });
    }
    if file.request.endpoint != STAGE4A_ENDPOINT || file.request.mode != FetchMode::Credentialed {
        return Err(RangeCanonicalError::MalformedStage4A {
            reason: "normalized request endpoint or mode is not the Stage4A contract".to_owned(),
        });
    }
    let query = query_map(&file.request.query)?;
    if query.get("session_date") != Some(&entry.date.to_iso()) {
        return Err(RangeCanonicalError::MalformedStage4A {
            reason: "normalized request does not bind the session date".to_owned(),
        });
    }
    Ok(file)
}

fn query_map(query: &[(String, String)]) -> Result<BTreeMap<String, String>, RangeCanonicalError> {
    let mut out = BTreeMap::new();
    for (key, value) in query {
        if out.insert(key.clone(), value.clone()).is_some() {
            return Err(RangeCanonicalError::MalformedStage4A {
                reason: format!("duplicate query key {key}"),
            });
        }
    }
    Ok(out)
}

fn validate_document(
    entry: &ManifestEntry,
    file: &FileEntry,
    document: &Stage4ADocument,
) -> Result<(), RangeCanonicalError> {
    if document.schema_version != RANGE_NORMALIZED_SCHEMA_VERSION
        || document.dataset_kind != STAGE4A_DATASET_KIND
        || document.date != entry.date
        || document.acquired_at != entry.retrieved_at
    {
        return Err(RangeCanonicalError::MalformedStage4A {
            reason: "schema, dataset kind, date, or acquisition timestamp mismatch".to_owned(),
        });
    }
    if document.pit.mode != PIT_MODE
        || document.pit.strict
        || document.pit.availability_evidence
        || document.pit.revision_evidence
        || document.pit.knowledge_time_evidence
    {
        return Err(RangeCanonicalError::NonStrictPitNotApproved {
            reason: "Stage4A PIT flags do not describe the acquisition-time vendor snapshot"
                .to_owned(),
        });
    }
    if file.size_bytes == 0 {
        return Err(RangeCanonicalError::MalformedStage4A {
            reason: "bars file has zero size".to_owned(),
        });
    }
    Ok(())
}

fn validate_lineage(
    entry: &ManifestEntry,
    file: &FileEntry,
    lineage: &RangeNormalizationLineage,
) -> Result<(), RangeCanonicalError> {
    if lineage.schema_version != RANGE_NORMALIZER_SCHEMA_VERSION
        || lineage.normalizer != RANGE_NORMALIZER
        || lineage.upstream_market != MARKET_KR
        || lineage.upstream_provider != "kis-daily-range"
        || lineage.selected_session != entry.date
        || lineage.acquired_at != entry.retrieved_at
        || lineage.availability_evidence
        || lineage.revision_evidence
        || lineage.knowledge_time_evidence
    {
        return Err(RangeCanonicalError::InvalidLineage {
            reason: "normalizer, upstream scope, selected date, acquisition, or PIT flags mismatch"
                .to_owned(),
        });
    }
    if lineage.source_start > lineage.source_end
        || entry.date < lineage.source_start
        || entry.date > lineage.source_end
        || lineage.source_files.is_empty()
        || lineage.source_rows.len() != KR_ETF_CORE_SYMBOLS.len()
    {
        return Err(RangeCanonicalError::InvalidLineage {
            reason: "source range or source row lineage is incomplete".to_owned(),
        });
    }
    let query = query_map(&file.request.query)?;
    if query.get("source_batch_id") != Some(&lineage.upstream_batch_id.to_string())
        || query.get("source_manifest_hash") != Some(&lineage.upstream_manifest_hash.to_string())
    {
        return Err(RangeCanonicalError::InvalidLineage {
            reason: "normalized request does not bind the upstream source identity".to_owned(),
        });
    }
    let expected = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| format!("{symbol}.KRX"))
        .collect::<BTreeSet<_>>();
    let actual = lineage
        .source_rows
        .iter()
        .map(|row| format!("{}.KRX", row.symbol))
        .collect::<BTreeSet<_>>();
    if actual != expected
        || lineage
            .source_rows
            .iter()
            .any(|row| row.row_date != entry.date || row.source_file_name.is_empty())
    {
        return Err(RangeCanonicalError::InvalidLineage {
            reason: "source row lineage does not cover exactly the fixed ETF session".to_owned(),
        });
    }
    for source_file in &lineage.source_files {
        if source_file.kind != ResponseKind::Bars
            || source_file.file_name.is_empty()
            || source_file.size_bytes == 0
            || source_file.request.mode != FetchMode::Credentialed
        {
            return Err(RangeCanonicalError::InvalidLineage {
                reason: "source file lineage has invalid kind, mode, name, or size".to_owned(),
            });
        }
    }
    Ok(())
}

fn validate_schedule(
    schedule: &HistoricalSessionScheduleEvidence,
    session: TradingDate,
) -> Result<(), RangeCanonicalError> {
    let (source, version) = match &schedule.authority {
        ScheduleAuthority::Reviewed { source, version } => (source, version),
        ScheduleAuthority::AuditOnly { .. } => {
            return Err(RangeCanonicalError::UnsupportedHistoricalSessionSchedule {
                reason: "dates-only/audit-only schedule cannot establish historical open/close"
                    .to_owned(),
            });
        }
    };
    if schedule.schema_version != EVIDENCE_PACKAGE_SCHEMA_VERSION
        || schedule.calendar_id.trim().is_empty()
        || schedule.session_date != session
        || schedule.open_utc >= schedule.close_utc
        || source.trim().is_empty()
        || version.trim().is_empty()
    {
        return Err(RangeCanonicalError::UnsupportedHistoricalSessionSchedule {
            reason: "reviewed schedule identity, date, authority, or interval is invalid"
                .to_owned(),
        });
    }
    match (schedule.break_start_utc, schedule.break_end_utc) {
        (Some(start), Some(end))
            if schedule.open_utc <= start && start < end && end <= schedule.close_utc => {}
        (None, None) => {}
        _ => {
            return Err(RangeCanonicalError::UnsupportedHistoricalSessionSchedule {
                reason: "break must be absent or a bounded interval inside the session".to_owned(),
            });
        }
    }
    Ok(())
}

fn validate_listing(
    listing: &ListingMasterEvidence,
    session: TradingDate,
    source_acquired_at: UtcTimestamp,
) -> Result<(), RangeCanonicalError> {
    if listing.schema_version != EVIDENCE_PACKAGE_SCHEMA_VERSION
        || listing.snapshot_id.trim().is_empty()
        || listing.source.trim().is_empty()
        || listing.instruments.len() != KR_ETF_CORE_SYMBOLS.len()
        || listing.captured_at > source_acquired_at
    {
        return Err(RangeCanonicalError::MissingListingMasterEvidence {
            reason: "listing snapshot identity/hash/count is invalid".to_owned(),
        });
    }
    let expected = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| format!("{symbol}.KRX"))
        .collect::<BTreeSet<_>>();
    let mut actual = BTreeSet::new();
    for instrument in &listing.instruments {
        if !actual.insert(instrument.instrument_id.as_str())
            || instrument.kind != AssetClass::Etf
            || instrument.name.trim().is_empty()
            || instrument.lot_size.is_zero()
            || instrument.listed_at.is_none()
            || instrument.acquired_at > source_acquired_at
            || instrument.acquired_at < listing.captured_at
        {
            return Err(RangeCanonicalError::MissingListingMasterEvidence {
                reason: "listing kind, identity, name, lot, or interval is invalid".to_owned(),
            });
        }
        let listed = instrument.listed_at.expect("checked above");
        if let Some(delisted) = instrument.delisted_at {
            if listed >= delisted {
                return Err(RangeCanonicalError::MissingListingMasterEvidence {
                    reason: "listing interval is empty or reversed".to_owned(),
                });
            }
            if session < listed || session >= delisted {
                return Err(RangeCanonicalError::MissingListingMasterEvidence {
                    reason: format!("{} is not listed on {session}", instrument.instrument_id),
                });
            }
        } else if session < listed {
            return Err(RangeCanonicalError::MissingListingMasterEvidence {
                reason: format!("{} is not listed on {session}", instrument.instrument_id),
            });
        }
    }
    if actual != expected {
        return Err(RangeCanonicalError::MissingListingMasterEvidence {
            reason: "listing snapshot does not equal the fixed 11 ETF universe".to_owned(),
        });
    }
    Ok(())
}

fn validate_actions(
    actions: &ActionCoverageEvidence,
    source_start: TradingDate,
    source_end: TradingDate,
    session: TradingDate,
) -> Result<(), RangeCanonicalError> {
    if actions.range_start > actions.range_end
        || actions.range_start != source_start
        || actions.range_end != source_end
        || session < source_start
        || session > source_end
        || actions.coverage_hash != actions.computed_hash()
    {
        return Err(RangeCanonicalError::MissingActionEvidence {
            reason: "action coverage identity, range, or hash is invalid".to_owned(),
        });
    }
    let mut kinds = BTreeSet::new();
    for file in &actions.files {
        if !kinds.insert(file.kind.as_str())
            || file.endpoint.trim().is_empty()
            || file.file_name.trim().is_empty()
            || file.size_bytes == 0
            || file.query_start != actions.range_start
            || file.query_end != actions.range_end
        {
            return Err(RangeCanonicalError::MissingActionEvidence {
                reason: "action file identity, hash/size, or exact query range is invalid"
                    .to_owned(),
            });
        }
    }
    let required = REQUIRED_ACTION_KINDS
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if kinds != required {
        return Err(RangeCanonicalError::MissingActionEvidence {
            reason: "all seven KSD response classes are required".to_owned(),
        });
    }
    if actions.actions.is_empty() && actions.files.len() != REQUIRED_ACTION_KINDS.len() {
        return Err(RangeCanonicalError::MissingActionEvidence {
            reason: "empty action list requires an attested exact-range zero result".to_owned(),
        });
    }
    for action in &actions.actions {
        match action {
            RangeAction::Unsupported { kind, .. } => {
                return Err(RangeCanonicalError::UnsupportedAction { kind: kind.clone() });
            }
            RangeAction::BonusIssue {
                instrument_id,
                record_date,
                ex_date,
                split_factor,
                available_at,
            } => {
                if !KR_ETF_CORE_SYMBOLS.iter().any(|symbol| {
                    instrument_id == &InstrumentId::from_parts(symbol, Venue::Krx).unwrap()
                }) || *record_date < actions.range_start
                    || *record_date > actions.range_end
                    || *ex_date < actions.range_start
                    || *ex_date > actions.range_end
                    || *split_factor <= FixedPoint::parse("1").expect("one")
                    || *available_at != actions.acquired_at
                {
                    return Err(RangeCanonicalError::MissingActionEvidence {
                        reason:
                            "bonus action dates, instrument, factor, or acquisition time is invalid"
                                .to_owned(),
                    });
                }
            }
        }
    }
    Ok(())
}

fn validate_pit_policy(policy: &NonStrictPitPolicyApproval) -> Result<(), RangeCanonicalError> {
    if policy.schema_version != EVIDENCE_PACKAGE_SCHEMA_VERSION
        || policy.policy_id != NON_STRICT_PIT_POLICY_ID
        || !policy.approved
        || policy.approved_by.trim().is_empty()
        || policy.rationale.trim().is_empty()
    {
        return Err(RangeCanonicalError::NonStrictPitNotApproved {
            reason: "policy id, approval, approver, or rationale is invalid".to_owned(),
        });
    }
    Ok(())
}

fn parse_bars(
    rows: &[Value],
    session: TradingDate,
) -> Result<Vec<RangeCanonicalBarCandidate>, RangeCanonicalError> {
    if rows.len() != KR_ETF_CORE_SYMBOLS.len() {
        return Err(RangeCanonicalError::InvalidSession {
            reason: format!("expected exactly 11 bars, got {}", rows.len()),
        });
    }
    let expected = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| format!("{symbol}.KRX"))
        .collect::<BTreeSet<_>>();
    let mut actual = BTreeSet::new();
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let object = row
            .as_object()
            .ok_or_else(|| RangeCanonicalError::InvalidBarValue {
                instrument: "<unknown>".to_owned(),
                field: "row".to_owned(),
                value: row.to_string(),
                reason: "bar row must be an object".to_owned(),
            })?;
        let instrument = required_string(object, "instrument")?;
        let instrument_id =
            InstrumentId::parse(instrument).map_err(|_| RangeCanonicalError::InvalidBarValue {
                instrument: instrument.to_owned(),
                field: "instrument".to_owned(),
                value: instrument.to_owned(),
                reason: "invalid canonical ETF instrument".to_owned(),
            })?;
        if !actual.insert(instrument.to_owned()) {
            return Err(RangeCanonicalError::InvalidBarValue {
                instrument: instrument.to_owned(),
                field: "instrument".to_owned(),
                value: instrument.to_owned(),
                reason: "duplicate instrument".to_owned(),
            });
        }
        if !expected.contains(instrument) {
            return Err(RangeCanonicalError::InvalidBarValue {
                instrument: instrument.to_owned(),
                field: "instrument".to_owned(),
                value: instrument.to_owned(),
                reason: "instrument is outside fixed ETF universe".to_owned(),
            });
        }
        let row_date = TradingDate::parse(required_string(object, "date")?).map_err(|_| {
            RangeCanonicalError::InvalidBarValue {
                instrument: instrument.to_owned(),
                field: "date".to_owned(),
                value: object
                    .get("date")
                    .map_or_else(|| "<missing>".to_owned(), Value::to_string),
                reason: "invalid trading date".to_owned(),
            }
        })?;
        if row_date != session {
            return Err(RangeCanonicalError::InvalidBarValue {
                instrument: instrument.to_owned(),
                field: "date".to_owned(),
                value: row_date.to_string(),
                reason: "row date differs from selected session".to_owned(),
            });
        }
        let open = parse_decimal(object, instrument, "open")?;
        let high = parse_decimal(object, instrument, "high")?;
        let low = parse_decimal(object, instrument, "low")?;
        let close = parse_decimal(object, instrument, "close")?;
        for (field, value) in [
            ("open", open),
            ("high", high),
            ("low", low),
            ("close", close),
        ] {
            if !value.is_positive() {
                return Err(invalid_bar(
                    instrument,
                    field,
                    value.to_string(),
                    "price must be positive",
                ));
            }
        }
        if high < open || high < close || low > open || low > close || low > high {
            return Err(invalid_bar(
                instrument,
                "ohlc",
                format!("{open}/{high}/{low}/{close}"),
                "OHLC invariant is violated",
            ));
        }
        let volume = parse_nonnegative_integer(object, instrument, "volume")?;
        let trading_value = object
            .get("value")
            .map(|_| parse_decimal(object, instrument, "value"))
            .transpose()?
            .map(|value| {
                if value.is_negative() {
                    Err(invalid_bar(
                        instrument,
                        "value",
                        value.to_string(),
                        "value is negative",
                    ))
                } else {
                    Ok(value)
                }
            })
            .transpose()?;
        out.push(RangeCanonicalBarCandidate {
            instrument_id,
            session_date: session,
            open,
            high,
            low,
            close,
            volume,
            trading_value,
        });
    }
    if actual != expected {
        return Err(RangeCanonicalError::InvalidSession {
            reason: "bar rows do not cover exactly the fixed 11 ETF universe".to_owned(),
        });
    }
    out.sort_by_key(|bar| bar.instrument_id.clone());
    Ok(out)
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<&'a str, RangeCanonicalError> {
    let value = object.get(field).and_then(Value::as_str).ok_or_else(|| {
        RangeCanonicalError::InvalidBarValue {
            instrument: object
                .get("instrument")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>")
                .to_owned(),
            field: field.to_owned(),
            value: object
                .get(field)
                .map_or_else(|| "<missing>".to_owned(), Value::to_string),
            reason: "field must be a string".to_owned(),
        }
    })?;
    if value.trim() != value || value.is_empty() {
        return Err(RangeCanonicalError::InvalidBarValue {
            instrument: object
                .get("instrument")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>")
                .to_owned(),
            field: field.to_owned(),
            value: value.to_owned(),
            reason: "string must be nonempty and whitespace-free at the boundary".to_owned(),
        });
    }
    Ok(value)
}

fn parse_decimal(
    object: &Map<String, Value>,
    instrument: &str,
    field: &str,
) -> Result<FixedPoint, RangeCanonicalError> {
    let raw = object
        .get(field)
        .ok_or_else(|| invalid_bar(instrument, field, "<missing>", "field is required"))?;
    let text = match raw {
        Value::String(value) if !value.is_empty() && value.trim() == value => value.clone(),
        Value::Number(value) => value.to_string(),
        _ => {
            return Err(invalid_bar(
                instrument,
                field,
                raw.to_string(),
                "must be a decimal string or JSON number",
            ));
        }
    };
    FixedPoint::parse(&text)
        .map_err(|_| invalid_bar(instrument, field, text, "invalid finite decimal"))
}

fn parse_nonnegative_integer(
    object: &Map<String, Value>,
    instrument: &str,
    field: &str,
) -> Result<u64, RangeCanonicalError> {
    let raw = object
        .get(field)
        .ok_or_else(|| invalid_bar(instrument, field, "<missing>", "field is required"))?;
    let value = match raw {
        Value::String(value) if !value.is_empty() && value.trim() == value => {
            value.parse::<u64>().map_err(|_| ())
        }
        Value::Number(value) => value.as_u64().ok_or(()),
        _ => Err(()),
    };
    value.map_err(|_| {
        invalid_bar(
            instrument,
            field,
            raw.to_string(),
            "must be a nonnegative integer",
        )
    })
}

fn invalid_bar(
    instrument: &str,
    field: &str,
    value: impl Into<String>,
    reason: &str,
) -> RangeCanonicalError {
    RangeCanonicalError::InvalidBarValue {
        instrument: instrument.to_owned(),
        field: field.to_owned(),
        value: value.into(),
        reason: reason.to_owned(),
    }
}

#[allow(clippy::too_many_arguments)]
fn deterministic_candidate_id(
    entry: &ManifestEntry,
    source_entry_hash: &ContentHash,
    source_file_hash: &ContentHash,
    lineage: &RangeNormalizationLineage,
    schedule: &HistoricalSessionScheduleEvidence,
    listing: &ListingMasterEvidence,
    actions: &ActionCoverageEvidence,
    pit_policy: &NonStrictPitPolicyApproval,
    evidence_manifest_hash: &ContentHash,
    schedule_artifact_hash: &ContentHash,
    listing_artifact_hash: &ContentHash,
    pit_policy_artifact_hash: &ContentHash,
) -> BatchId {
    let name = format!(
        "bridge={RANGE_CANONICAL_BRIDGE_VERSION}\nevidence_manifest_hash={}\nschedule_artifact_hash={}\nlisting_artifact_hash={}\npit_policy_artifact_hash={}\nsource_batch={}\nsource_entry_hash={}\nsource_file_hash={}\nupstream_batch={}\nupstream_manifest_hash={}\nsession={}\ncalendar_hash={}\nlisting_hash={}\naction_hash={}\npit_policy_hash={}",
        evidence_manifest_hash,
        schedule_artifact_hash,
        listing_artifact_hash,
        pit_policy_artifact_hash,
        entry.batch_id,
        source_entry_hash,
        source_file_hash,
        lineage.upstream_batch_id,
        lineage.upstream_manifest_hash,
        entry.date,
        schedule.calendar_hash,
        listing.snapshot_hash,
        actions.coverage_hash,
        pit_policy.hash(),
    );
    BatchId::from_uuid(Uuid::new_v5(&Uuid::NAMESPACE_URL, name.as_bytes()))
}

/// Identifies which of the seven KSD classes one Raw KIS action file belongs
/// to by matching its request endpoint/tr_id/query against the allowlisted
/// [`kis_action_spec`] contract for the given range. Returns `None` for a
/// file that matches no allowlisted class (wrong response kind, mode,
/// endpoint, or query shape).
fn identify_action_kind(
    file: &FileEntry,
    range_start: TradingDate,
    range_end: TradingDate,
) -> Option<&'static str> {
    if file.kind != ResponseKind::CorporateActions || file.request.mode != FetchMode::Credentialed {
        return None;
    }
    let query = query_map(&file.request.query).ok()?;
    for kind in REQUIRED_ACTION_KINDS {
        let Some(spec) = action_spec(kind) else {
            continue;
        };
        if file.request.endpoint != spec.path {
            continue;
        }
        if query != expected_action_query(&spec, range_start, range_end) {
            continue;
        }
        let tr_ids = file
            .request
            .headers
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case("tr_id"))
            .map(|(_, value)| value.as_str())
            .collect::<Vec<_>>();
        if tr_ids.as_slice() == [spec.tr_id] {
            return Some(kind);
        }
    }
    None
}

/// True only for the first page of a KSD request (an explicit empty
/// `tr_cont`).  A continuation page of the same class is deliberately never
/// selected as evidence: this bridge has no persisted continuation chain.
fn is_initial_action_page(file: &FileEntry) -> bool {
    let continuations = file
        .request
        .headers
        .iter()
        .filter(|(key, _)| key.eq_ignore_ascii_case("tr_cont"))
        .map(|(_, value)| value.as_str())
        .collect::<Vec<_>>();
    continuations.as_slice() == [""]
}

/// Load the exact independently pinned seven-file KSD witness.
///
/// Unlike the Stage4B package discovery path below, an explicit reviewed hash
/// disambiguates repeated captures. The selected entry must still be the
/// exact allowlisted seven-class, initial-page shape and all response bytes
/// are re-read and validated before an opaque input is returned.
fn load_pinned_action_coverage(
    raw: &RawStore,
    expected_manifest_hash: &ContentHash,
    range_start: TradingDate,
    range_end: TradingDate,
) -> Result<ActionCoverageEvidence, RangeCanonicalError> {
    let entries = raw.read_committed_manifest(PROVIDER_KIS, MARKET_KR)?;
    let mut matches = Vec::new();
    for entry in &entries {
        if ContentHash::from_bytes(&serde_json::to_vec(&entry)?) == *expected_manifest_hash {
            matches.push(entry);
        }
    }
    let entry = matches
        .pop()
        .ok_or_else(|| RangeCanonicalError::MissingActionEvidence {
            reason: "pinned KSD action manifest is absent from immutable Raw".to_owned(),
        })?;
    if !matches.is_empty() {
        return Err(RangeCanonicalError::MissingActionEvidence {
            reason: "pinned KSD action manifest is not unique".to_owned(),
        });
    }
    if entry.mode != FetchMode::Credentialed || entry.files.len() != REQUIRED_ACTION_KINDS.len() {
        return Err(RangeCanonicalError::MissingActionEvidence {
            reason: "pinned KSD action batch is not the exact seven-file credentialed shape"
                .to_owned(),
        });
    }
    let mut chosen = BTreeMap::new();
    for file in &entry.files {
        let kind = identify_action_kind(file, range_start, range_end).ok_or_else(|| {
            RangeCanonicalError::MissingActionEvidence {
                reason: "pinned KSD file does not match an allowlisted action class".to_owned(),
            }
        })?;
        if !is_initial_action_page(file) || chosen.insert(kind, file).is_some() {
            return Err(RangeCanonicalError::MissingActionEvidence {
                reason: "pinned KSD action classes are duplicated or not initial pages".to_owned(),
            });
        }
    }
    if chosen.len() != REQUIRED_ACTION_KINDS.len() {
        return Err(RangeCanonicalError::MissingActionEvidence {
            reason: "pinned KSD batch does not cover all seven action classes".to_owned(),
        });
    }
    let refs = chosen
        .into_iter()
        .map(|(kind, file)| EvidenceActionRef {
            kind: kind.to_owned(),
            raw_batch_id: entry.batch_id,
            raw_manifest_hash: expected_manifest_hash.clone(),
            raw_file_name: file.file_name.clone(),
            content_hash: file.content_hash.clone(),
            size_bytes: file.size_bytes,
        })
        .collect::<Vec<_>>();
    load_action_evidence_from_entries(raw, &entries, &refs, range_start, range_end)
}

/// Finds the one immutable Raw KIS batch whose files are exactly the seven
/// initial-page KSD action responses over `[range_start, range_end]` — one
/// per class, and nothing else — and returns the corresponding evidence
/// refs.  A batch that merely *contains* those seven responses is not a
/// candidate, because the loader can never accept it.
/// Fails closed (as [`RangeCanonicalError::MissingActionEvidence`]) when zero
/// or more than one Raw batch qualifies, so a package can never silently pin
/// an arbitrary or ambiguous action batch.
fn find_matching_action_evidence(
    raw: &RawStore,
    range_start: TradingDate,
    range_end: TradingDate,
) -> Result<Vec<EvidenceActionRef>, RangeCanonicalError> {
    let entries = raw.read_reconciled_manifest(PROVIDER_KIS, MARKET_KR)?;
    find_matching_action_evidence_in_entries(&entries, range_start, range_end)
}

fn find_matching_action_evidence_in_entries(
    entries: &[ManifestEntry],
    range_start: TradingDate,
    range_end: TradingDate,
) -> Result<Vec<EvidenceActionRef>, RangeCanonicalError> {
    let mut candidates: Vec<Vec<EvidenceActionRef>> = Vec::new();
    for entry in entries {
        // `load_action_evidence` requires the pinned batch to hold exactly
        // one file per KSD class and nothing else, so a batch with extra
        // files -- the daily EOD bundle, or a paginated KSD range batch --
        // can never be loaded.  Mirroring that constraint here keeps such a
        // batch out of the candidate set entirely, instead of letting it be
        // selected and then rejected, or counted toward ambiguity against a
        // batch that is genuinely loadable.
        if entry.provider != PROVIDER_KIS
            || entry.market != MARKET_KR
            || entry.mode != FetchMode::Credentialed
            || entry.files.len() != REQUIRED_ACTION_KINDS.len()
        {
            continue;
        }
        let mut chosen: BTreeMap<&'static str, &FileEntry> = BTreeMap::new();
        let mut ambiguous = false;
        for file in &entry.files {
            let Some(kind) = identify_action_kind(file, range_start, range_end) else {
                continue;
            };
            if !is_initial_action_page(file) {
                continue;
            }
            if chosen.insert(kind, file).is_some() {
                ambiguous = true;
            }
        }
        if ambiguous || chosen.len() != REQUIRED_ACTION_KINDS.len() {
            continue;
        }
        let raw_manifest_hash = ContentHash::from_bytes(&serde_json::to_vec(entry)?);
        candidates.push(
            chosen
                .into_iter()
                .map(|(kind, file)| EvidenceActionRef {
                    kind: kind.to_owned(),
                    raw_batch_id: entry.batch_id,
                    raw_manifest_hash: raw_manifest_hash.clone(),
                    raw_file_name: file.file_name.clone(),
                    content_hash: file.content_hash.clone(),
                    size_bytes: file.size_bytes,
                })
                .collect(),
        );
    }
    match candidates.len() {
        1 => Ok(candidates.remove(0)),
        0 => Err(RangeCanonicalError::MissingActionEvidence {
            reason: "no Raw KIS batch has exactly one initial-page response for each of the \
                     seven KSD action classes over this range"
                .to_owned(),
        }),
        _ => Err(RangeCanonicalError::MissingActionEvidence {
            reason: "multiple Raw KIS batches match the seven KSD action classes over this \
                     range; the pin is ambiguous"
                .to_owned(),
        }),
    }
}

fn sync_directory(path: &Path) -> Result<(), RangeCanonicalError> {
    fs::File::open(path)
        .and_then(|handle| handle.sync_all())
        .map_err(|error| RangeCanonicalError::EvidencePackage {
            reason: format!("failed to sync directory {}: {error}", path.display()),
        })
}

/// A staging directory that becomes the caller's package directory only on
/// one atomic rename.
///
/// Two properties matter here.  First, the whole parent chain of `out_dir` is
/// proven absolute and symlink-free *before* anything is created: directory
/// creation follows symlinks while [`safe_package_root`] rejects them, so
/// checking afterwards would already have created a directory outside the
/// intended tree.  Second, every artifact is assembled under a sibling
/// staging path, so a failure partway through the four writes leaves no
/// partial package at `out_dir` for a reviewer to mistake for a complete one
/// and no half-written directory to block a retry.
struct PackageStaging {
    staging: PathBuf,
    target: PathBuf,
    parent: PathBuf,
    committed: bool,
}

impl PackageStaging {
    fn create(out_dir: &Path, unique: BatchId) -> Result<Self, RangeCanonicalError> {
        let unsafe_out = || RangeCanonicalError::UnsafeEvidencePath {
            path: out_dir.display().to_string(),
        };
        if out_dir.as_os_str().is_empty() || !out_dir.is_absolute() {
            return Err(unsafe_out());
        }
        let (Some(parent), Some(name)) = (out_dir.parent(), out_dir.file_name()) else {
            return Err(unsafe_out());
        };
        // Only the final component may be created, and only after its whole
        // ancestry is verified.  A missing intermediate directory is an
        // operator error, not something to materialize silently.
        let parent = safe_package_root(parent)?;
        let target = parent.join(name);
        match fs::symlink_metadata(&target) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(unsafe_out());
                }
                if fs::read_dir(&target)
                    .map_err(|_| unsafe_out())?
                    .next()
                    .is_some()
                {
                    return Err(RangeCanonicalError::EvidencePackage {
                        reason: format!(
                            "package directory {} already exists and is not empty",
                            target.display()
                        ),
                    });
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(unsafe_out()),
        }

        let staging = parent.join(format!(".{unique}.evidence-package.partial"));
        fs::create_dir(&staging).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                RangeCanonicalError::EvidencePackage {
                    reason: format!(
                        "staging directory {} already exists; remove the residue of an \
                         interrupted run before retrying",
                        staging.display()
                    ),
                }
            } else {
                RangeCanonicalError::UnsafeEvidencePath {
                    path: staging.display().to_string(),
                }
            }
        })?;
        let staging = match safe_package_root(&staging) {
            Ok(root) => root,
            Err(error) => {
                let _ = fs::remove_dir(&staging);
                return Err(error);
            }
        };
        Ok(Self {
            staging,
            target,
            parent,
            committed: false,
        })
    }

    fn root(&self) -> &Path {
        &self.staging
    }

    /// Publishes the assembled package.  The staging directory is fsynced so
    /// its entries are durable, renamed onto `target`, and the parent is then
    /// fsynced so the rename itself is durable.  A parent-sync failure after
    /// the rename is still reported as an error, but the published directory
    /// is complete and is deliberately not removed.
    fn commit(mut self) -> Result<(), RangeCanonicalError> {
        sync_directory(&self.staging)?;
        fs::rename(&self.staging, &self.target).map_err(|error| {
            RangeCanonicalError::EvidencePackage {
                reason: format!("failed to publish {}: {error}", self.target.display()),
            }
        })?;
        self.committed = true;
        sync_directory(&self.parent)
    }
}

impl Drop for PackageStaging {
    fn drop(&mut self) {
        if !self.committed {
            // Best effort.  The staging path is one this type created inside
            // a verified, symlink-free parent and is never the caller's own
            // `out_dir`, so this cannot remove caller-supplied data.
            let _ = fs::remove_dir_all(&self.staging);
        }
    }
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), RangeCanonicalError> {
    let mut handle = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| RangeCanonicalError::EvidencePackage {
            reason: format!("failed to create {}: {error}", path.display()),
        })?;
    handle
        .write_all(bytes)
        .map_err(|error| RangeCanonicalError::EvidencePackage {
            reason: format!("failed to write {}: {error}", path.display()),
        })?;
    handle
        .sync_all()
        .map_err(|error| RangeCanonicalError::EvidencePackage {
            reason: format!("failed to sync {}: {error}", path.display()),
        })
}

fn write_evidence_artifact(
    root: &Path,
    name: &str,
    bytes: &[u8],
) -> Result<EvidenceArtifactRef, RangeCanonicalError> {
    write_new_file(&root.join(name), bytes)?;
    Ok(EvidenceArtifactRef {
        path: name.to_owned(),
        sha256: ContentHash::from_bytes(bytes),
        size_bytes: bytes.len() as u64,
        schema_version: EVIDENCE_PACKAGE_SCHEMA_VERSION,
    })
}

/// Assembles a Stage4B-0 evidence package on disk: `schedule.json`,
/// `listing.json`, `pit.json`, and `manifest.json` inside `out_dir`.
///
/// `schedule_bytes`/`listing_bytes`/`pit_policy_bytes` are caller-supplied
/// evidence, never synthesized here. Every one of them is fully validated
/// against the immutable Stage4A lineage before anything is written, and the
/// seven KSD action files are discovered from the immutable Raw KIS batch
/// that exactly covers this session's range (never caller-selected). The
/// action coverage is also verified end-to-end via [`load_action_evidence`]
/// *and* its paired [`validate_actions`] — the second is what rejects action
/// content the first deliberately admits — so the returned hash is proven
/// loadable, not merely schema-shaped: every check
/// [`load_verified_range_canonical_evidence`] runs afterward has already run
/// here, on the same Raw store and the same evidence bytes. What it cannot
/// prove is that the bytes on disk stay unchanged between the two calls; the
/// loader re-reads and re-hashes them for exactly that reason.
///
/// Exactly one check is deliberately left unsatisfied, because this function
/// has no authority over it: the returned [`ContentHash`] must still be
/// reviewed and committed to
/// `configs/evidence/kis-range-canonical-approved-manifests.json` by an
/// operator before [`load_verified_range_canonical_evidence`] will accept it.
pub fn write_evidence_package(
    raw: &RawStore,
    normalized_entry: &ManifestEntry,
    schedule_bytes: &[u8],
    listing_bytes: &[u8],
    pit_policy_bytes: &[u8],
    out_dir: &Path,
) -> Result<ContentHash, RangeCanonicalError> {
    validate_scope(normalized_entry)?;
    let (file, document) = read_stage4a_document(raw, normalized_entry)?;
    validate_entry(normalized_entry)?;
    validate_document(normalized_entry, file, &document)?;
    validate_lineage(normalized_entry, file, &document.lineage)?;
    verify_upstream_range_manifest(raw, &document.lineage)?;

    let schedule: HistoricalSessionScheduleEvidence = serde_json::from_slice(schedule_bytes)
        .map_err(
            |error| RangeCanonicalError::UnsupportedHistoricalSessionSchedule {
                reason: format!("supplied schedule evidence is malformed: {error}"),
            },
        )?;
    validate_schedule(&schedule, normalized_entry.date)?;
    if schedule.calendar_id != document.lineage.calendar_id
        || schedule.calendar_hash != document.lineage.calendar_hash
    {
        return Err(RangeCanonicalError::UnsupportedHistoricalSessionSchedule {
            reason: "supplied schedule identity differs from Stage4A lineage".to_owned(),
        });
    }

    let listing: ListingMasterEvidence =
        serde_json::from_slice(listing_bytes).map_err(|error| {
            RangeCanonicalError::MissingListingMasterEvidence {
                reason: format!("supplied listing evidence is malformed: {error}"),
            }
        })?;
    validate_listing(
        &listing,
        normalized_entry.date,
        normalized_entry.retrieved_at,
    )?;
    if listing.snapshot_id != document.lineage.listing_snapshot_id
        || listing.snapshot_hash != document.lineage.listing_snapshot_hash
    {
        return Err(RangeCanonicalError::MissingListingMasterEvidence {
            reason: "supplied listing identity differs from Stage4A lineage".to_owned(),
        });
    }

    let pit_policy: NonStrictPitPolicyApproval =
        serde_json::from_slice(pit_policy_bytes).map_err(|error| {
            RangeCanonicalError::NonStrictPitNotApproved {
                reason: format!("supplied PIT policy evidence is malformed: {error}"),
            }
        })?;
    validate_pit_policy(&pit_policy)?;

    let action_refs = find_matching_action_evidence(
        raw,
        document.lineage.source_start,
        document.lineage.source_end,
    )?;
    // Fully exercise the same acceptance path the loader will run later, so
    // a package that fails to load can never be written in the first place.
    // `load_action_evidence` is deliberately permissive about content -- a
    // nonempty non-bonus KSD response becomes `RangeAction::Unsupported` and
    // loads fine -- so the paired `validate_actions` call is what actually
    // rejects action content the loader will refuse.  Running only the first
    // would print a hash for a package that can never load, and an approved
    // hash for an unloadable package is permanently dead gate evidence.
    let coverage = load_action_evidence(
        raw,
        &action_refs,
        document.lineage.source_start,
        document.lineage.source_end,
    )?;
    validate_actions(
        &coverage,
        document.lineage.source_start,
        document.lineage.source_end,
        normalized_entry.date,
    )?;

    let staging = PackageStaging::create(out_dir, normalized_entry.batch_id)?;
    let calendar = write_evidence_artifact(staging.root(), "schedule.json", schedule_bytes)?;
    let listing_ref = write_evidence_artifact(staging.root(), "listing.json", listing_bytes)?;
    let pit_ref = write_evidence_artifact(staging.root(), "pit.json", pit_policy_bytes)?;

    let manifest = EvidencePackageManifest {
        schema_version: EVIDENCE_PACKAGE_SCHEMA_VERSION,
        bridge_version: RANGE_CANONICAL_BRIDGE_VERSION.to_owned(),
        source_batch_id: document.lineage.upstream_batch_id,
        normalized_batch_id: normalized_entry.batch_id,
        session_date: normalized_entry.date,
        range_start: document.lineage.source_start,
        range_end: document.lineage.source_end,
        calendar,
        listing: listing_ref,
        pit_policy: pit_ref,
        actions: action_refs,
    };
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    write_new_file(&staging.root().join("manifest.json"), &manifest_bytes)?;
    staging.commit()?;
    Ok(ContentHash::from_bytes(&manifest_bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_action_kinds_are_stable() {
        assert_eq!(REQUIRED_ACTION_KINDS.len(), 7);
        assert!(REQUIRED_ACTION_KINDS.contains(&"bonus-issue"));
    }
}

#[cfg(test)]
#[path = "range_to_canonical_tests.rs"]
mod acceptance_tests;
