//! Descriptor-safe, deterministic V3 historical price-only artifact.
//!
//! The V3 artifact is intentionally separate from the beta V2 artifact.  It
//! is a materialized, owner-only vendor snapshot whose lineage is pinned to
//! the independently verified ten-year price and action evidence.  This
//! module never reads Raw, contacts a provider, or exposes a publication
//! operation.  It consumes only the opaque V3 candidate supplied by
//! `crate::historical_price_only_v3`.

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use crate::curate::Capability;
use crate::historical_price_only::HistoricalPriceOnlyBar;
use crate::historical_price_only_v3::{
    HISTORICAL_PRICE_ONLY_V3_CONTRACT, HISTORICAL_PRICE_ONLY_V3_MATERIALIZER_VERSION,
    HistoricalPriceOnlyV3Candidate,
};
use crate::providers::kis::KR_ETF_CORE_SYMBOLS;
use crate::range_to_canonical_v3::{
    HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_ID, HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_JSON_SHA256,
    HISTORICAL_PRICE_ONLY_V3_ACTION_FILE_COUNT,
    HISTORICAL_PRICE_ONLY_V3_ACTION_MANIFEST_LINE_SHA256,
    HISTORICAL_PRICE_ONLY_V3_CASH_DIVIDEND_TREATMENT,
};
use crate::range_to_canonical_v3_price::{
    HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_ID, HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_JSON_SHA256,
    HISTORICAL_PRICE_ONLY_V3_PRICE_CAPTURE_COMMIT, HISTORICAL_PRICE_ONLY_V3_PRICE_FILE_COUNT,
    HISTORICAL_PRICE_ONLY_V3_PRICE_MANIFEST_LINE_SHA256,
    HISTORICAL_PRICE_ONLY_V3_PRICE_RESPONSE_MARKER_EVIDENCE,
};
use domain::{BatchId, ContentHash, FixedPoint, InstrumentId, TradingDate, UtcTimestamp};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The durable V3 artifact schema identity.
pub const HISTORICAL_PRICE_ONLY_V3_ARTIFACT_SCHEMA_ID: &str = "kis-historical-price-only-v3";
pub const HISTORICAL_PRICE_ONLY_V3_ARTIFACT_SCHEMA_VERSION: u32 = 3;

const EXPECTED_RANGE_START: &str = "2016-08-29";
const EXPECTED_RANGE_END: &str = "2026-08-28";
const EXPECTED_SESSION_COUNT: usize = 2_452;
const EXPECTED_ROW_COUNT: usize = 26_972;
const EXPECTED_CASH_ROW_COUNT: usize = 157;
const EXPECTED_ACTION_COUNT: usize = 0;
const EXPECTED_PRICE_SCALE: u8 = crate::historical_price_only::HISTORICAL_PRICE_ONLY_PRICE_SCALE;
const EXPECTED_FACTOR_SCALE: u8 = crate::historical_price_only::HISTORICAL_PRICE_ONLY_FACTOR_SCALE;
const MAX_MANIFEST_BYTES: usize = 2 * 1024 * 1024;
const MAX_BARS_BYTES: usize = 64 * 1024 * 1024;
const MAX_BAR_LINE_BYTES: usize = 16 * 1024;

/// A verified V3 artifact.  Its fields are private so callers can obtain one
/// only after a complete writer/readback validation.
#[derive(Clone, PartialEq, Eq)]
pub struct VerifiedHistoricalPriceOnlyV3Artifact {
    path: PathBuf,
    candidate_content_sha256: ContentHash,
    approval_summary: HistoricalPriceOnlyV3ArtifactApprovalSummary,
    approved_bars: Vec<HistoricalPriceOnlyBar>,
}

impl std::fmt::Debug for VerifiedHistoricalPriceOnlyV3Artifact {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VerifiedHistoricalPriceOnlyV3Artifact")
            .field("candidate_content_sha256", &self.candidate_content_sha256)
            .finish()
    }
}

impl VerifiedHistoricalPriceOnlyV3Artifact {
    /// Opaque candidate lineage identity.  This is not recomputed by a reader
    /// because the candidate preimage intentionally includes excluded source
    /// metadata.
    pub fn candidate_content_sha256(&self) -> &ContentHash {
        &self.candidate_content_sha256
    }

    /// The validated, non-sensitive facts needed by a later approval checker.
    pub fn approval_summary(&self) -> &HistoricalPriceOnlyV3ArtifactApprovalSummary {
        &self.approval_summary
    }

    /// The validated price-only bars.  No construction path is public.
    pub fn bars(&self) -> &[HistoricalPriceOnlyBar] {
        &self.approved_bars
    }

    /// Filesystem location of this artifact, retained for operator diagnostics
    /// without including any source payload or credential material.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Immutable facts from the V3 manifest that an independent approval checker
/// may compare against its own registry.  It deliberately carries no Raw
/// request metadata, provider body, account identity, or publication authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoricalPriceOnlyV3ArtifactApprovalSummary {
    artifact_manifest_sha256: ContentHash,
    candidate_content_sha256: ContentHash,
    price_batch_id: BatchId,
    price_batch_json_sha256: ContentHash,
    price_manifest_line_sha256: ContentHash,
    price_bars_sha256: ContentHash,
    price_file_count: usize,
    price_capture_contract_commit: String,
    price_response_marker_evidence: String,
    price_acquired_at: UtcTimestamp,
    action_batch_id: BatchId,
    action_batch_json_sha256: ContentHash,
    action_manifest_line_sha256: ContentHash,
    action_file_count: usize,
    action_count: usize,
    cash_dividend_treatment_id: String,
    cash_dividend_row_count: usize,
    cash_dividend_rows_sha256: ContentHash,
    action_acquired_at: UtcTimestamp,
    schema_id: String,
    schema_version: u32,
    contract: String,
    materializer_version: String,
    audience: String,
    vendor_snapshot: bool,
    strict_pit: bool,
    capability: String,
    materialization_status: String,
    registration_status: String,
    publication_status: String,
    range_start: TradingDate,
    range_end: TradingDate,
    instruments: Vec<String>,
    instrument_count: usize,
    session_count: usize,
    row_count: usize,
    bars_relative_path: String,
    bars_sha256: ContentHash,
    bars_size_bytes: u64,
    bars_row_count: usize,
}

impl HistoricalPriceOnlyV3ArtifactApprovalSummary {
    pub fn artifact_manifest_sha256(&self) -> &ContentHash {
        &self.artifact_manifest_sha256
    }
    pub fn candidate_content_sha256(&self) -> &ContentHash {
        &self.candidate_content_sha256
    }
    pub const fn price_batch_id(&self) -> BatchId {
        self.price_batch_id
    }
    pub fn price_batch_json_sha256(&self) -> &ContentHash {
        &self.price_batch_json_sha256
    }
    pub fn price_manifest_line_sha256(&self) -> &ContentHash {
        &self.price_manifest_line_sha256
    }
    pub fn price_bars_sha256(&self) -> &ContentHash {
        &self.price_bars_sha256
    }
    pub const fn price_file_count(&self) -> usize {
        self.price_file_count
    }
    pub fn price_capture_contract_commit(&self) -> &str {
        &self.price_capture_contract_commit
    }
    pub fn price_response_marker_evidence(&self) -> &str {
        &self.price_response_marker_evidence
    }
    pub const fn price_acquired_at(&self) -> UtcTimestamp {
        self.price_acquired_at
    }
    pub const fn action_batch_id(&self) -> BatchId {
        self.action_batch_id
    }
    pub fn action_batch_json_sha256(&self) -> &ContentHash {
        &self.action_batch_json_sha256
    }
    pub fn action_manifest_line_sha256(&self) -> &ContentHash {
        &self.action_manifest_line_sha256
    }
    pub const fn action_file_count(&self) -> usize {
        self.action_file_count
    }
    pub const fn action_count(&self) -> usize {
        self.action_count
    }
    pub fn cash_dividend_treatment_id(&self) -> &str {
        &self.cash_dividend_treatment_id
    }
    pub const fn cash_dividend_row_count(&self) -> usize {
        self.cash_dividend_row_count
    }
    pub fn cash_dividend_rows_sha256(&self) -> &ContentHash {
        &self.cash_dividend_rows_sha256
    }
    pub const fn action_acquired_at(&self) -> UtcTimestamp {
        self.action_acquired_at
    }
    /// Acquisition time shared by the action batch and its cash-evidence
    /// commitment.  The timestamp is provenance only, never a price date.
    pub const fn cash_dividend_acquired_at(&self) -> UtcTimestamp {
        self.action_acquired_at
    }
    pub fn schema_id(&self) -> &str {
        &self.schema_id
    }
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }
    pub fn contract(&self) -> &str {
        &self.contract
    }
    pub fn materializer_version(&self) -> &str {
        &self.materializer_version
    }
    pub fn audience(&self) -> &str {
        &self.audience
    }
    pub const fn vendor_snapshot(&self) -> bool {
        self.vendor_snapshot
    }
    pub const fn strict_pit(&self) -> bool {
        self.strict_pit
    }
    pub fn capability(&self) -> &str {
        &self.capability
    }
    pub fn materialization_status(&self) -> &str {
        &self.materialization_status
    }
    pub fn registration_status(&self) -> &str {
        &self.registration_status
    }
    pub fn publication_status(&self) -> &str {
        &self.publication_status
    }
    pub const fn range_start(&self) -> TradingDate {
        self.range_start
    }
    pub const fn range_end(&self) -> TradingDate {
        self.range_end
    }
    pub fn instruments(&self) -> &[String] {
        &self.instruments
    }
    pub const fn instrument_count(&self) -> usize {
        self.instrument_count
    }
    pub const fn session_count(&self) -> usize {
        self.session_count
    }
    pub const fn row_count(&self) -> usize {
        self.row_count
    }
    pub fn bars_relative_path(&self) -> &str {
        &self.bars_relative_path
    }
    pub fn bars_sha256(&self) -> &ContentHash {
        &self.bars_sha256
    }
    pub const fn bars_size_bytes(&self) -> u64 {
        self.bars_size_bytes
    }
    pub const fn bars_row_count(&self) -> usize {
        self.bars_row_count
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct BarDto {
    instrument_id: String,
    session_date: TradingDate,
    raw_open: String,
    raw_high: String,
    raw_low: String,
    raw_close: String,
    raw_volume: u64,
    raw_trading_value: Option<String>,
    adjusted_open: String,
    adjusted_high: String,
    adjusted_low: String,
    adjusted_close: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct PriceSourceDto {
    batch_id: BatchId,
    batch_json_sha256: ContentHash,
    manifest_line_sha256: ContentHash,
    bars_sha256: ContentHash,
    file_count: usize,
    capture_contract_commit: String,
    response_marker_evidence: String,
    acquired_at: UtcTimestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ActionSourceDto {
    batch_id: BatchId,
    batch_json_sha256: ContentHash,
    manifest_line_sha256: ContentHash,
    file_count: usize,
    action_count: usize,
    cash_dividend_treatment_id: String,
    cash_dividend_row_count: usize,
    cash_dividend_rows_sha256: ContentHash,
    acquired_at: UtcTimestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct BarsDto {
    relative_path: String,
    schema_id: String,
    schema_version: u32,
    sha256: ContentHash,
    size_bytes: u64,
    row_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct UnsignedManifest {
    schema_id: String,
    schema_version: u32,
    contract: String,
    materializer_version: String,
    candidate_content_sha256: ContentHash,
    audience: String,
    vendor_snapshot: bool,
    strict_pit: bool,
    capability: String,
    materialization_status: String,
    registration_status: String,
    publication_status: String,
    range_start: TradingDate,
    range_end: TradingDate,
    instruments: Vec<String>,
    instrument_count: usize,
    session_count: usize,
    row_count: usize,
    price_scale: u8,
    factor_scale: u8,
    price_source: PriceSourceDto,
    action_source: ActionSourceDto,
    bars: BarsDto,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_id: String,
    schema_version: u32,
    contract: String,
    materializer_version: String,
    candidate_content_sha256: ContentHash,
    audience: String,
    vendor_snapshot: bool,
    strict_pit: bool,
    capability: String,
    materialization_status: String,
    registration_status: String,
    publication_status: String,
    range_start: TradingDate,
    range_end: TradingDate,
    instruments: Vec<String>,
    instrument_count: usize,
    session_count: usize,
    row_count: usize,
    price_scale: u8,
    factor_scale: u8,
    price_source: PriceSourceDto,
    action_source: ActionSourceDto,
    bars: BarsDto,
    manifest_sha256: ContentHash,
}

struct ValidatedManifest {
    unsigned: UnsignedManifest,
    approval_summary: HistoricalPriceOnlyV3ArtifactApprovalSummary,
}

impl Manifest {
    fn from_unsigned(
        unsigned: UnsignedManifest,
    ) -> Result<Self, HistoricalPriceOnlyV3ArtifactError> {
        let bytes = serde_json::to_vec(&unsigned)
            .map_err(|_| HistoricalPriceOnlyV3ArtifactError::InvalidArtifact)?;
        let manifest_sha256 = ContentHash::from_bytes(&bytes);
        Ok(Self {
            schema_id: unsigned.schema_id,
            schema_version: unsigned.schema_version,
            contract: unsigned.contract,
            materializer_version: unsigned.materializer_version,
            candidate_content_sha256: unsigned.candidate_content_sha256,
            audience: unsigned.audience,
            vendor_snapshot: unsigned.vendor_snapshot,
            strict_pit: unsigned.strict_pit,
            capability: unsigned.capability,
            materialization_status: unsigned.materialization_status,
            registration_status: unsigned.registration_status,
            publication_status: unsigned.publication_status,
            range_start: unsigned.range_start,
            range_end: unsigned.range_end,
            instruments: unsigned.instruments,
            instrument_count: unsigned.instrument_count,
            session_count: unsigned.session_count,
            row_count: unsigned.row_count,
            price_scale: unsigned.price_scale,
            factor_scale: unsigned.factor_scale,
            price_source: unsigned.price_source,
            action_source: unsigned.action_source,
            bars: unsigned.bars,
            manifest_sha256,
        })
    }

    fn unsigned(&self) -> UnsignedManifest {
        UnsignedManifest {
            schema_id: self.schema_id.clone(),
            schema_version: self.schema_version,
            contract: self.contract.clone(),
            materializer_version: self.materializer_version.clone(),
            candidate_content_sha256: self.candidate_content_sha256.clone(),
            audience: self.audience.clone(),
            vendor_snapshot: self.vendor_snapshot,
            strict_pit: self.strict_pit,
            capability: self.capability.clone(),
            materialization_status: self.materialization_status.clone(),
            registration_status: self.registration_status.clone(),
            publication_status: self.publication_status.clone(),
            range_start: self.range_start,
            range_end: self.range_end,
            instruments: self.instruments.clone(),
            instrument_count: self.instrument_count,
            session_count: self.session_count,
            row_count: self.row_count,
            price_scale: self.price_scale,
            factor_scale: self.factor_scale,
            price_source: self.price_source.clone(),
            action_source: self.action_source.clone(),
            bars: self.bars.clone(),
        }
    }

    fn approval_summary(&self) -> HistoricalPriceOnlyV3ArtifactApprovalSummary {
        HistoricalPriceOnlyV3ArtifactApprovalSummary {
            artifact_manifest_sha256: self.manifest_sha256.clone(),
            candidate_content_sha256: self.candidate_content_sha256.clone(),
            price_batch_id: self.price_source.batch_id,
            price_batch_json_sha256: self.price_source.batch_json_sha256.clone(),
            price_manifest_line_sha256: self.price_source.manifest_line_sha256.clone(),
            price_bars_sha256: self.price_source.bars_sha256.clone(),
            price_file_count: self.price_source.file_count,
            price_capture_contract_commit: self.price_source.capture_contract_commit.clone(),
            price_response_marker_evidence: self.price_source.response_marker_evidence.clone(),
            price_acquired_at: self.price_source.acquired_at,
            action_batch_id: self.action_source.batch_id,
            action_batch_json_sha256: self.action_source.batch_json_sha256.clone(),
            action_manifest_line_sha256: self.action_source.manifest_line_sha256.clone(),
            action_file_count: self.action_source.file_count,
            action_count: self.action_source.action_count,
            cash_dividend_treatment_id: self.action_source.cash_dividend_treatment_id.clone(),
            cash_dividend_row_count: self.action_source.cash_dividend_row_count,
            cash_dividend_rows_sha256: self.action_source.cash_dividend_rows_sha256.clone(),
            action_acquired_at: self.action_source.acquired_at,
            schema_id: self.schema_id.clone(),
            schema_version: self.schema_version,
            contract: self.contract.clone(),
            materializer_version: self.materializer_version.clone(),
            audience: self.audience.clone(),
            vendor_snapshot: self.vendor_snapshot,
            strict_pit: self.strict_pit,
            capability: self.capability.clone(),
            materialization_status: self.materialization_status.clone(),
            registration_status: self.registration_status.clone(),
            publication_status: self.publication_status.clone(),
            range_start: self.range_start,
            range_end: self.range_end,
            instruments: self.instruments.clone(),
            instrument_count: self.instrument_count,
            session_count: self.session_count,
            row_count: self.row_count,
            bars_relative_path: self.bars.relative_path.clone(),
            bars_sha256: self.bars.sha256.clone(),
            bars_size_bytes: self.bars.size_bytes,
            bars_row_count: self.bars.row_count,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ArtifactBytes {
    bars_ndjson: Vec<u8>,
    manifest_json: Vec<u8>,
}

/// Commitment produced by the V3 price replay verifier for the normalized
/// price-bar sequence.  It is distinct from the hash of the artifact's
/// newline-delimited DTO bytes below.
const EXPECTED_SOURCE_BARS_SHA256: &str =
    "sha256:20c750f0ca415073da37650ae2bb0c942a181b4c86f167defe95895e4499dcf2";
const EXPECTED_CASH_ROWS_SHA256: &str =
    "sha256:b22a5c9808a8a1a2c892aa3ff46d529672c909620a2c45c0e46d48d0538d17e8";

fn expected_date(value: &str) -> TradingDate {
    TradingDate::parse(value).expect("checked-in artifact date is valid")
}

fn expected_hash(value: &str) -> ContentHash {
    ContentHash::parse(value).expect("checked-in artifact hash is valid")
}

fn expected_instruments() -> Vec<String> {
    let mut instruments = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| format!("{symbol}.KRX"))
        .collect::<Vec<_>>();
    instruments.sort();
    instruments
}

/// Project the opaque candidate to deterministic artifact bytes.  No source
/// file, request metadata, provider payload, or secret is serialized here.
fn project_candidate(
    candidate: &HistoricalPriceOnlyV3Candidate,
) -> Result<ArtifactBytes, HistoricalPriceOnlyV3ArtifactError> {
    validate_candidate(candidate)?;

    let mut rows = candidate
        .bars()
        .iter()
        .map(|bar| BarDto {
            instrument_id: bar.instrument_id.to_string(),
            session_date: bar.session_date,
            raw_open: bar.raw_open.to_string(),
            raw_high: bar.raw_high.to_string(),
            raw_low: bar.raw_low.to_string(),
            raw_close: bar.raw_close.to_string(),
            raw_volume: bar.raw_volume,
            raw_trading_value: bar.raw_trading_value.map(|value| value.to_string()),
            adjusted_open: bar.adjusted_open.to_string(),
            adjusted_high: bar.adjusted_high.to_string(),
            adjusted_low: bar.adjusted_low.to_string(),
            adjusted_close: bar.adjusted_close.to_string(),
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        left.instrument_id
            .cmp(&right.instrument_id)
            .then(left.session_date.cmp(&right.session_date))
    });

    let mut bars_ndjson = Vec::with_capacity(rows.len().saturating_mul(160));
    for row in rows {
        serde_json::to_writer(&mut bars_ndjson, &row)
            .map_err(|_| HistoricalPriceOnlyV3ArtifactError::InvalidArtifact)?;
        bars_ndjson.push(b'\n');
    }
    if bars_ndjson.len() > MAX_BARS_BYTES {
        return Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact);
    }
    let artifact_bars_hash = ContentHash::from_bytes(&bars_ndjson);
    let source_bars_hash = candidate.source_bars_sha256().clone();

    let metadata = candidate.metadata();
    let unsigned = UnsignedManifest {
        schema_id: HISTORICAL_PRICE_ONLY_V3_ARTIFACT_SCHEMA_ID.to_owned(),
        schema_version: HISTORICAL_PRICE_ONLY_V3_ARTIFACT_SCHEMA_VERSION,
        contract: candidate.contract().to_owned(),
        materializer_version: candidate.materializer_version().to_owned(),
        candidate_content_sha256: candidate.content_hash().clone(),
        audience: metadata.audience.as_str().to_owned(),
        vendor_snapshot: metadata.vendor_snapshot,
        strict_pit: metadata.strict_pit,
        capability: capability_string(metadata.capability),
        materialization_status: "MATERIALIZED".to_owned(),
        registration_status: "UNREGISTERED".to_owned(),
        publication_status: "NOT_PUBLISHED".to_owned(),
        range_start: candidate.range_start(),
        range_end: candidate.range_end(),
        instruments: expected_instruments(),
        instrument_count: 11,
        session_count: candidate.session_count(),
        row_count: candidate.bars().len(),
        price_scale: EXPECTED_PRICE_SCALE,
        factor_scale: EXPECTED_FACTOR_SCALE,
        price_source: PriceSourceDto {
            batch_id: candidate.source_batch_id(),
            batch_json_sha256: candidate.source_batch_json_sha256().clone(),
            manifest_line_sha256: candidate.source_manifest_line_sha256().clone(),
            bars_sha256: source_bars_hash,
            file_count: candidate.source_file_count(),
            capture_contract_commit: candidate.source_capture_contract_commit().to_owned(),
            response_marker_evidence: candidate.source_response_marker_evidence().to_owned(),
            acquired_at: candidate.source_acquired_at(),
        },
        action_source: ActionSourceDto {
            batch_id: candidate.action_batch_id(),
            batch_json_sha256: candidate.action_source_batch_json_sha256().clone(),
            manifest_line_sha256: candidate.action_source_manifest_line_sha256().clone(),
            file_count: candidate.action_file_count(),
            action_count: candidate.action_count(),
            cash_dividend_treatment_id: candidate.cash_dividend_treatment_id().to_owned(),
            cash_dividend_row_count: candidate.cash_dividend_row_count(),
            cash_dividend_rows_sha256: candidate.cash_dividend_rows_sha256().clone(),
            acquired_at: candidate.action_acquired_at(),
        },
        bars: BarsDto {
            relative_path: "bars.ndjson".to_owned(),
            schema_id: "historical-price-only-bars".to_owned(),
            schema_version: 1,
            sha256: artifact_bars_hash,
            size_bytes: u64::try_from(bars_ndjson.len())
                .map_err(|_| HistoricalPriceOnlyV3ArtifactError::InvalidArtifact)?,
            row_count: candidate.bars().len(),
        },
    };
    let manifest = Manifest::from_unsigned(unsigned)?;
    let mut manifest_json = serde_json::to_vec(&manifest)
        .map_err(|_| HistoricalPriceOnlyV3ArtifactError::InvalidArtifact)?;
    manifest_json.push(b'\n');
    validate_artifact_bytes(candidate.content_hash(), &bars_ndjson, &manifest_json)?;
    Ok(ArtifactBytes {
        bars_ndjson,
        manifest_json,
    })
}

fn capability_string(capability: Capability) -> String {
    capability.to_string()
}

fn validate_candidate(
    candidate: &HistoricalPriceOnlyV3Candidate,
) -> Result<(), HistoricalPriceOnlyV3ArtifactError> {
    let metadata = candidate.metadata();
    let expected_source_batch_id = BatchId::from_str(HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_ID)
        .map_err(|_| HistoricalPriceOnlyV3ArtifactError::InvalidCandidate)?;
    let expected_action_batch_id = BatchId::from_str(HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_ID)
        .map_err(|_| HistoricalPriceOnlyV3ArtifactError::InvalidCandidate)?;
    let source_batch_json = expected_hash(HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_JSON_SHA256);
    let source_manifest_line = expected_hash(HISTORICAL_PRICE_ONLY_V3_PRICE_MANIFEST_LINE_SHA256);
    let source_bars = expected_hash(EXPECTED_SOURCE_BARS_SHA256);
    let action_batch_json = expected_hash(HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_JSON_SHA256);
    let action_manifest_line = expected_hash(HISTORICAL_PRICE_ONLY_V3_ACTION_MANIFEST_LINE_SHA256);
    let cash_rows = expected_hash(EXPECTED_CASH_ROWS_SHA256);

    let instruments = candidate
        .instruments()
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let sessions = approved_sessions(
        expected_date(EXPECTED_RANGE_START),
        expected_date(EXPECTED_RANGE_END),
    )?;
    if candidate.range_start() != expected_date(EXPECTED_RANGE_START)
        || candidate.range_end() != expected_date(EXPECTED_RANGE_END)
        || candidate.session_count() != EXPECTED_SESSION_COUNT
        || candidate.bars().len() != EXPECTED_ROW_COUNT
        || candidate.source_batch_id() != expected_source_batch_id
        || candidate.source_batch_json_sha256() != &source_batch_json
        || candidate.source_manifest_line_sha256() != &source_manifest_line
        || candidate.source_bars_sha256() != &source_bars
        || candidate.source_file_count() != HISTORICAL_PRICE_ONLY_V3_PRICE_FILE_COUNT
        || candidate.source_capture_contract_commit()
            != HISTORICAL_PRICE_ONLY_V3_PRICE_CAPTURE_COMMIT
        || candidate.source_response_marker_evidence()
            != HISTORICAL_PRICE_ONLY_V3_PRICE_RESPONSE_MARKER_EVIDENCE
        || candidate.pit_policy() != "PRICE_RETURN_ONLY"
        || candidate.action_batch_id() != expected_action_batch_id
        || candidate.action_source_batch_json_sha256() != &action_batch_json
        || candidate.action_source_manifest_line_sha256() != &action_manifest_line
        || candidate.action_file_count() != HISTORICAL_PRICE_ONLY_V3_ACTION_FILE_COUNT
        || candidate.action_count() != EXPECTED_ACTION_COUNT
        || candidate.cash_dividend_treatment_id()
            != HISTORICAL_PRICE_ONLY_V3_CASH_DIVIDEND_TREATMENT
        || candidate.cash_dividend_row_count() != EXPECTED_CASH_ROW_COUNT
        || candidate.cash_dividend_rows_sha256() != &cash_rows
        || instruments != expected_instruments()
        || candidate.sessions() != sessions
        || metadata.audience.as_str() != "OWNER_ONLY"
        || !metadata.vendor_snapshot
        || metadata.strict_pit
        || metadata.capability != Capability::PriceReturnOnly
        || metadata.materialized
        || !metadata.in_memory
        || metadata.ready
        || candidate.contract() != HISTORICAL_PRICE_ONLY_V3_CONTRACT
        || candidate.materializer_version() != HISTORICAL_PRICE_ONLY_V3_MATERIALIZER_VERSION
    {
        return Err(HistoricalPriceOnlyV3ArtifactError::InvalidCandidate);
    }
    Ok(())
}

fn validate_artifact_bytes(
    expected_candidate_hash: &ContentHash,
    bars_ndjson: &[u8],
    manifest_json: &[u8],
) -> Result<(), HistoricalPriceOnlyV3ArtifactError> {
    if bars_ndjson.len() > MAX_BARS_BYTES || manifest_json.len() > MAX_MANIFEST_BYTES {
        return Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact);
    }
    let validated = validate_manifest_bytes(expected_candidate_hash, manifest_json)?;
    let bars = &validated.unsigned.bars;
    if bars.relative_path != "bars.ndjson"
        || bars.size_bytes
            != u64::try_from(bars_ndjson.len())
                .map_err(|_| HistoricalPriceOnlyV3ArtifactError::InvalidArtifact)?
        || bars.sha256 != ContentHash::from_bytes(bars_ndjson)
        || bars.row_count != EXPECTED_ROW_COUNT
    {
        return Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact);
    }
    validate_bars(
        bars_ndjson,
        &validated.unsigned.range_start,
        &validated.unsigned.range_end,
        &validated.unsigned.instruments,
    )
}

fn validate_manifest_bytes(
    expected_candidate_hash: &ContentHash,
    manifest_json: &[u8],
) -> Result<ValidatedManifest, HistoricalPriceOnlyV3ArtifactError> {
    if manifest_json.len() > MAX_MANIFEST_BYTES || !manifest_json.ends_with(b"\n") {
        return Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact);
    }
    let manifest: Manifest = serde_json::from_slice(manifest_json)
        .map_err(|_| HistoricalPriceOnlyV3ArtifactError::InvalidArtifact)?;
    let unsigned = manifest.unsigned();
    let expected_source_batch_id = BatchId::from_str(HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_ID)
        .map_err(|_| HistoricalPriceOnlyV3ArtifactError::InvalidArtifact)?;
    let expected_action_batch_id = BatchId::from_str(HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_ID)
        .map_err(|_| HistoricalPriceOnlyV3ArtifactError::InvalidArtifact)?;
    let expected_contract_hash = expected_hash(EXPECTED_SOURCE_BARS_SHA256);
    if &unsigned.candidate_content_sha256 != expected_candidate_hash
        || unsigned.schema_id != HISTORICAL_PRICE_ONLY_V3_ARTIFACT_SCHEMA_ID
        || unsigned.schema_version != HISTORICAL_PRICE_ONLY_V3_ARTIFACT_SCHEMA_VERSION
        || unsigned.contract != HISTORICAL_PRICE_ONLY_V3_CONTRACT
        || unsigned.materializer_version != HISTORICAL_PRICE_ONLY_V3_MATERIALIZER_VERSION
        || unsigned.audience != "OWNER_ONLY"
        || !unsigned.vendor_snapshot
        || unsigned.strict_pit
        || unsigned.capability != "PRICE_RETURN_ONLY"
        || unsigned.materialization_status != "MATERIALIZED"
        || unsigned.registration_status != "UNREGISTERED"
        || unsigned.publication_status != "NOT_PUBLISHED"
        || unsigned.range_start != expected_date(EXPECTED_RANGE_START)
        || unsigned.range_end != expected_date(EXPECTED_RANGE_END)
        || unsigned.instruments != expected_instruments()
        || unsigned.instrument_count != 11
        || unsigned.session_count != EXPECTED_SESSION_COUNT
        || unsigned.row_count != EXPECTED_ROW_COUNT
        || unsigned.price_scale != EXPECTED_PRICE_SCALE
        || unsigned.factor_scale != EXPECTED_FACTOR_SCALE
        || unsigned.price_source.batch_id != expected_source_batch_id
        || unsigned.price_source.batch_json_sha256
            != expected_hash(HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_JSON_SHA256)
        || unsigned.price_source.manifest_line_sha256
            != expected_hash(HISTORICAL_PRICE_ONLY_V3_PRICE_MANIFEST_LINE_SHA256)
        || unsigned.price_source.bars_sha256 != expected_contract_hash
        || unsigned.price_source.file_count != HISTORICAL_PRICE_ONLY_V3_PRICE_FILE_COUNT
        || unsigned.price_source.capture_contract_commit
            != HISTORICAL_PRICE_ONLY_V3_PRICE_CAPTURE_COMMIT
        || unsigned.price_source.response_marker_evidence
            != HISTORICAL_PRICE_ONLY_V3_PRICE_RESPONSE_MARKER_EVIDENCE
        || unsigned.action_source.batch_id != expected_action_batch_id
        || unsigned.action_source.batch_json_sha256
            != expected_hash(HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_JSON_SHA256)
        || unsigned.action_source.manifest_line_sha256
            != expected_hash(HISTORICAL_PRICE_ONLY_V3_ACTION_MANIFEST_LINE_SHA256)
        || unsigned.action_source.file_count != HISTORICAL_PRICE_ONLY_V3_ACTION_FILE_COUNT
        || unsigned.action_source.action_count != EXPECTED_ACTION_COUNT
        || unsigned.action_source.cash_dividend_treatment_id
            != HISTORICAL_PRICE_ONLY_V3_CASH_DIVIDEND_TREATMENT
        || unsigned.action_source.cash_dividend_row_count != EXPECTED_CASH_ROW_COUNT
        || unsigned.action_source.cash_dividend_rows_sha256
            != expected_hash(EXPECTED_CASH_ROWS_SHA256)
        || unsigned.bars.relative_path != "bars.ndjson"
        || unsigned.bars.schema_id != "historical-price-only-bars"
        || unsigned.bars.schema_version != 1
        || unsigned.bars.row_count != EXPECTED_ROW_COUNT
        || unsigned.bars.size_bytes > MAX_BARS_BYTES as u64
    {
        return Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact);
    }
    let expected_manifest_hash = ContentHash::from_bytes(
        &serde_json::to_vec(&unsigned)
            .map_err(|_| HistoricalPriceOnlyV3ArtifactError::InvalidArtifact)?,
    );
    if expected_manifest_hash != manifest.manifest_sha256 {
        return Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact);
    }
    let mut canonical = serde_json::to_vec(&manifest)
        .map_err(|_| HistoricalPriceOnlyV3ArtifactError::InvalidArtifact)?;
    canonical.push(b'\n');
    if canonical != manifest_json {
        return Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact);
    }
    validate_manifest_semantics(&unsigned)?;
    Ok(ValidatedManifest {
        approval_summary: manifest.approval_summary(),
        unsigned,
    })
}

fn validate_manifest_semantics(
    manifest: &UnsignedManifest,
) -> Result<(), HistoricalPriceOnlyV3ArtifactError> {
    let approved = crate::range_normalize::ExpectedRangeSessions::approved_xkrx(
        manifest.range_start,
        manifest.range_end,
    )
    .map_err(|_| HistoricalPriceOnlyV3ArtifactError::InvalidArtifact)?;
    if approved.sessions.len() != EXPECTED_SESSION_COUNT {
        return Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact);
    }
    Ok(())
}

fn validate_bars(
    bytes: &[u8],
    range_start: &TradingDate,
    range_end: &TradingDate,
    instruments: &[String],
) -> Result<(), HistoricalPriceOnlyV3ArtifactError> {
    if bytes.len() > MAX_BARS_BYTES {
        return Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact);
    }
    let sessions = approved_sessions(*range_start, *range_end)?;
    let mut validator = BarValidator::new(&sessions, instruments, None);
    validator.feed(bytes)?;
    validator.finish().map(|_| ())
}

fn approved_sessions(
    start: TradingDate,
    end: TradingDate,
) -> Result<Vec<TradingDate>, HistoricalPriceOnlyV3ArtifactError> {
    let approved = crate::range_normalize::ExpectedRangeSessions::approved_xkrx(start, end)
        .map_err(|_| HistoricalPriceOnlyV3ArtifactError::InvalidArtifact)?;
    if approved.sessions.len() != EXPECTED_SESSION_COUNT {
        return Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact);
    }
    Ok(approved.sessions)
}

struct BarValidator<'a> {
    sessions: &'a [TradingDate],
    session_set: BTreeSet<TradingDate>,
    instruments: &'a [String],
    expected_hash: Option<&'a ContentHash>,
    expected_row_count: usize,
    hasher: Sha256,
    line: Vec<u8>,
    seen: BTreeSet<(String, TradingDate)>,
    previous: Option<(String, TradingDate)>,
    count: usize,
    total_bytes: usize,
    saw_terminal_lf: bool,
    approved_bars: Vec<HistoricalPriceOnlyBar>,
}

impl<'a> BarValidator<'a> {
    fn new(
        sessions: &'a [TradingDate],
        instruments: &'a [String],
        expected_hash: Option<&'a ContentHash>,
    ) -> Self {
        Self {
            sessions,
            session_set: sessions.iter().copied().collect(),
            instruments,
            expected_hash,
            expected_row_count: EXPECTED_ROW_COUNT,
            hasher: Sha256::new(),
            line: Vec::with_capacity(MAX_BAR_LINE_BYTES),
            seen: BTreeSet::new(),
            previous: None,
            count: 0,
            total_bytes: 0,
            saw_terminal_lf: false,
            approved_bars: Vec::with_capacity(EXPECTED_ROW_COUNT),
        }
    }

    #[cfg(test)]
    fn new_for_test(sessions: &'a [TradingDate], instruments: &'a [String]) -> Self {
        let mut validator = Self::new(sessions, instruments, None);
        validator.expected_row_count = sessions.len().saturating_mul(instruments.len());
        validator.approved_bars = Vec::with_capacity(validator.expected_row_count);
        validator
    }

    fn feed(&mut self, bytes: &[u8]) -> Result<(), HistoricalPriceOnlyV3ArtifactError> {
        self.total_bytes = self
            .total_bytes
            .checked_add(bytes.len())
            .ok_or(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact)?;
        if self.total_bytes > MAX_BARS_BYTES {
            return Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact);
        }
        self.hasher.update(bytes);
        for byte in bytes {
            if *byte == b'\n' {
                self.validate_line()?;
                self.line.clear();
                self.saw_terminal_lf = true;
            } else {
                self.saw_terminal_lf = false;
                if self.line.len() >= MAX_BAR_LINE_BYTES {
                    return Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact);
                }
                self.line.push(*byte);
            }
        }
        Ok(())
    }

    fn validate_line(&mut self) -> Result<(), HistoricalPriceOnlyV3ArtifactError> {
        if self.line.is_empty() {
            return Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact);
        }
        let row: BarDto = serde_json::from_slice(&self.line)
            .map_err(|_| HistoricalPriceOnlyV3ArtifactError::InvalidArtifact)?;
        if serde_json::to_vec(&row)
            .map_err(|_| HistoricalPriceOnlyV3ArtifactError::InvalidArtifact)?
            != self.line
        {
            return Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact);
        }
        let key = (row.instrument_id.clone(), row.session_date);
        if self.previous.as_ref().is_some_and(|old| old >= &key) || !self.seen.insert(key.clone()) {
            return Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact);
        }
        self.previous = Some(key);
        if !self.instruments.contains(&row.instrument_id)
            || !self.session_set.contains(&row.session_date)
        {
            return Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact);
        }

        let raw_open = parse_canonical_price(&row.raw_open)?;
        let raw_high = parse_canonical_price(&row.raw_high)?;
        let raw_low = parse_canonical_price(&row.raw_low)?;
        let raw_close = parse_canonical_price(&row.raw_close)?;
        if !raw_open.is_positive()
            || !raw_high.is_positive()
            || !raw_low.is_positive()
            || !raw_close.is_positive()
            || raw_high < raw_open
            || raw_high < raw_close
            || raw_low > raw_open
            || raw_low > raw_close
            || raw_low > raw_high
        {
            return Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact);
        }
        let adjusted_open = parse_canonical_price(&row.adjusted_open)?;
        let adjusted_high = parse_canonical_price(&row.adjusted_high)?;
        let adjusted_low = parse_canonical_price(&row.adjusted_low)?;
        let adjusted_close = parse_canonical_price(&row.adjusted_close)?;
        if raw_open != adjusted_open
            || raw_high != adjusted_high
            || raw_low != adjusted_low
            || raw_close != adjusted_close
            || !adjusted_open.is_positive()
            || !adjusted_high.is_positive()
            || !adjusted_low.is_positive()
            || !adjusted_close.is_positive()
            || adjusted_high < adjusted_open
            || adjusted_high < adjusted_close
            || adjusted_low > adjusted_open
            || adjusted_low > adjusted_close
            || adjusted_low > adjusted_high
        {
            return Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact);
        }
        let raw_trading_value = row
            .raw_trading_value
            .as_deref()
            .map(parse_canonical_price)
            .transpose()?;
        if raw_trading_value
            .as_ref()
            .is_some_and(FixedPoint::is_negative)
        {
            return Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact);
        }
        self.count = self
            .count
            .checked_add(1)
            .ok_or(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact)?;
        if self.count > self.expected_row_count {
            return Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact);
        }
        self.approved_bars.push(HistoricalPriceOnlyBar {
            instrument_id: InstrumentId::parse(&row.instrument_id)
                .map_err(|_| HistoricalPriceOnlyV3ArtifactError::InvalidArtifact)?,
            session_date: row.session_date,
            raw_open,
            raw_high,
            raw_low,
            raw_close,
            raw_volume: row.raw_volume,
            raw_trading_value,
            adjusted_open,
            adjusted_high,
            adjusted_low,
            adjusted_close,
        });
        Ok(())
    }

    fn finish(self) -> Result<Vec<HistoricalPriceOnlyBar>, HistoricalPriceOnlyV3ArtifactError> {
        let session_dates = self.sessions.iter().copied().collect::<BTreeSet<_>>();
        if !self.saw_terminal_lf
            || !self.line.is_empty()
            || self.count != self.expected_row_count
            || self.seen.len() != self.expected_row_count
            || self.instruments.iter().any(|instrument| {
                self.seen
                    .iter()
                    .filter(|(name, _)| name == instrument)
                    .map(|(_, date)| *date)
                    .collect::<BTreeSet<_>>()
                    != session_dates
            })
        {
            return Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact);
        }
        if let Some(expected) = self.expected_hash {
            let digest = self.hasher.finalize();
            let hex = digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            let actual = ContentHash::parse(&format!("sha256:{hex}"))
                .map_err(|_| HistoricalPriceOnlyV3ArtifactError::InvalidArtifact)?;
            if &actual != expected {
                return Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact);
            }
        }
        Ok(self.approved_bars)
    }
}

fn parse_canonical_price(value: &str) -> Result<FixedPoint, HistoricalPriceOnlyV3ArtifactError> {
    let parsed = FixedPoint::parse(value)
        .map_err(|_| HistoricalPriceOnlyV3ArtifactError::InvalidArtifact)?;
    if parsed.to_string() != value {
        return Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact);
    }
    Ok(parsed)
}

/// Materialize and atomically publish one V3 artifact under `operator_root`.
/// The supplied root must already exist and be a dedicated operator-owned
/// directory; this function never creates or recursively modifies it.
pub fn write_historical_price_only_v3_artifact(
    operator_root: &Path,
    candidate: &HistoricalPriceOnlyV3Candidate,
) -> Result<VerifiedHistoricalPriceOnlyV3Artifact, HistoricalPriceOnlyV3ArtifactError> {
    #[cfg(not(unix))]
    {
        let _ = (operator_root, candidate);
        Err(HistoricalPriceOnlyV3ArtifactError::UnsupportedPlatform)
    }
    #[cfg(unix)]
    {
        let expected = project_candidate(candidate)?;
        write_artifact_unix(operator_root, candidate.content_hash(), &expected)
    }
}

/// Reopen a previously materialized V3 artifact.  The candidate hash is an
/// opaque lineage pin and is never used to reconstruct the candidate preimage.
pub fn read_historical_price_only_v3_artifact(
    operator_root: &Path,
    candidate_content_sha256: &ContentHash,
) -> Result<VerifiedHistoricalPriceOnlyV3Artifact, HistoricalPriceOnlyV3ArtifactError> {
    #[cfg(not(unix))]
    {
        let _ = (operator_root, candidate_content_sha256);
        Err(HistoricalPriceOnlyV3ArtifactError::UnsupportedPlatform)
    }
    #[cfg(unix)]
    {
        read_artifact_unix(operator_root, candidate_content_sha256, None)
    }
}

#[cfg(unix)]
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(unix)]
struct TrustedRoot {
    directories: Vec<std::os::fd::OwnedFd>,
    names: Vec<Vec<u8>>,
    snapshots: Vec<StatSnapshot>,
}

#[cfg(unix)]
impl TrustedRoot {
    fn fd(&self) -> &std::os::fd::OwnedFd {
        self.directories.last().expect("trusted root is held")
    }

    fn parent_fd(&self) -> &std::os::fd::OwnedFd {
        &self.directories[self.directories.len() - 2]
    }

    fn name(&self) -> &[u8] {
        self.names.last().expect("trusted root name is held")
    }

    fn revalidate(&self) -> Result<(), HistoricalPriceOnlyV3ArtifactError> {
        for (index, name) in self.names.iter().enumerate() {
            revalidate_named_identity(&self.directories[index], name, &self.snapshots[index])?;
        }
        revalidate_named_identity(
            self.parent_fd(),
            self.name(),
            self.snapshots.last().unwrap(),
        )
    }
}

#[cfg(unix)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct StatSnapshot {
    dev: u64,
    ino: u64,
    file_type: rustix::fs::FileType,
    uid: u64,
    mode: u64,
    nlink: u64,
    size: i128,
}

#[cfg(unix)]
fn stat_snapshot(stat: &rustix::fs::Stat) -> StatSnapshot {
    use rustix::fs::{FileType, Mode};
    StatSnapshot {
        dev: stat.st_dev,
        ino: stat.st_ino,
        file_type: FileType::from_raw_mode(stat.st_mode),
        uid: stat.st_uid as u64,
        mode: Mode::from_raw_mode(stat.st_mode).bits() as u64,
        nlink: stat.st_nlink,
        size: stat.st_size as i128,
    }
}

#[cfg(unix)]
fn same_stable_identity(actual: &StatSnapshot, expected: &StatSnapshot) -> bool {
    actual.dev == expected.dev
        && actual.ino == expected.ino
        && actual.file_type == expected.file_type
        && actual.uid == expected.uid
        && actual.mode == expected.mode
}

#[cfg(unix)]
fn io_error(error: rustix::io::Errno) -> HistoricalPriceOnlyV3ArtifactError {
    if error == rustix::io::Errno::LOOP || error == rustix::io::Errno::NOTDIR {
        HistoricalPriceOnlyV3ArtifactError::UnsafePath
    } else {
        HistoricalPriceOnlyV3ArtifactError::Io(error.into())
    }
}

#[cfg(unix)]
fn lexical_root_components(
    path: &Path,
) -> Result<Vec<Vec<u8>>, HistoricalPriceOnlyV3ArtifactError> {
    use std::os::unix::ffi::OsStrExt;

    let bytes = path.as_os_str().as_bytes();
    if bytes.len() < 2 || bytes[0] != b'/' || bytes[1] == b'/' || bytes.ends_with(b"/") {
        return Err(HistoricalPriceOnlyV3ArtifactError::UnsafePath);
    }
    let mut components = Vec::new();
    for component in bytes[1..].split(|byte| *byte == b'/') {
        if component.is_empty() || component == b"." || component == b".." || component.contains(&0)
        {
            return Err(HistoricalPriceOnlyV3ArtifactError::UnsafePath);
        }
        components.push(component.to_vec());
    }
    if components.is_empty() {
        return Err(HistoricalPriceOnlyV3ArtifactError::UnsafePath);
    }
    Ok(components)
}

#[cfg(unix)]
fn open_trusted_root(path: &Path) -> Result<TrustedRoot, HistoricalPriceOnlyV3ArtifactError> {
    use rustix::fs::{Mode, OFlags, fstat, open, openat};

    let components = lexical_root_components(path)?;
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let slash = open(Path::new("/"), flags, Mode::empty()).map_err(io_error)?;
    let mut directories = vec![slash];
    let mut names = Vec::with_capacity(components.len());
    let mut snapshots = Vec::with_capacity(components.len());
    for component in components {
        let parent = directories.last().expect("root descriptor is held");
        let fd = openat(parent, component.as_slice(), flags, Mode::empty()).map_err(io_error)?;
        let stat = fstat(&fd).map_err(io_error)?;
        if rustix::fs::FileType::from_raw_mode(stat.st_mode) != rustix::fs::FileType::Directory {
            return Err(HistoricalPriceOnlyV3ArtifactError::UnsafePath);
        }
        directories.push(fd);
        names.push(component);
        snapshots.push(stat_snapshot(&stat));
    }
    let root_fd = directories.last().expect("at least one root component");
    let owner = rustix::process::geteuid().as_raw();
    let root_stat = fstat(root_fd).map_err(io_error)?;
    validate_directory_stat(&root_stat, owner, None)?;
    let root_snapshot = stat_snapshot(&root_stat);
    if snapshots.last() != Some(&root_snapshot) {
        return Err(HistoricalPriceOnlyV3ArtifactError::UnsafePath);
    }
    let root = TrustedRoot {
        directories,
        names,
        snapshots,
    };
    root.revalidate()?;
    Ok(root)
}

#[cfg(unix)]
fn candidate_directory_name(
    candidate_hash: &ContentHash,
) -> Result<String, HistoricalPriceOnlyV3ArtifactError> {
    let digest = candidate_hash
        .as_str()
        .strip_prefix("sha256:")
        .ok_or(HistoricalPriceOnlyV3ArtifactError::UnsafePath)?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(HistoricalPriceOnlyV3ArtifactError::UnsafePath);
    }
    Ok(digest.to_owned())
}

#[cfg(unix)]
fn validate_directory_stat(
    stat: &rustix::fs::Stat,
    owner: rustix::process::RawUid,
    exact_mode: Option<u32>,
) -> Result<(), HistoricalPriceOnlyV3ArtifactError> {
    use rustix::fs::{FileType, Mode};
    if FileType::from_raw_mode(stat.st_mode) != FileType::Directory || stat.st_uid != owner {
        return Err(HistoricalPriceOnlyV3ArtifactError::UnsafePath);
    }
    let mode = Mode::from_raw_mode(stat.st_mode);
    if let Some(expected) = exact_mode {
        if mode.bits() != expected {
            return Err(HistoricalPriceOnlyV3ArtifactError::UnsafePath);
        }
    } else if mode.intersects(Mode::WGRP | Mode::WOTH) {
        return Err(HistoricalPriceOnlyV3ArtifactError::UnsafePath);
    }
    Ok(())
}

#[cfg(unix)]
fn open_directory_at(
    parent: &impl std::os::fd::AsFd,
    name: &[u8],
    owner: rustix::process::RawUid,
    exact_mode: Option<u32>,
) -> Result<(std::os::fd::OwnedFd, rustix::fs::Stat), HistoricalPriceOnlyV3ArtifactError> {
    use rustix::fs::{AtFlags, FileType, Mode, OFlags, fstat, openat, statat};

    let observed = statat(parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(io_error)?;
    validate_directory_stat(&observed, owner, exact_mode)?;
    let fd = openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(io_error)?;
    let stat = fstat(&fd).map_err(io_error)?;
    validate_directory_stat(&stat, owner, exact_mode)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::Directory
        || stat_snapshot(&stat) != stat_snapshot(&observed)
    {
        return Err(HistoricalPriceOnlyV3ArtifactError::UnsafePath);
    }
    revalidate_named_identity(parent, name, &stat_snapshot(&stat))?;
    Ok((fd, stat))
}

#[cfg(unix)]
fn open_leaf(
    parent: &impl std::os::fd::AsFd,
    name: &[u8],
    owner: rustix::process::RawUid,
    max_bytes: usize,
) -> Result<(std::fs::File, StatSnapshot), HistoricalPriceOnlyV3ArtifactError> {
    use rustix::fs::{AtFlags, FileType, Mode, OFlags, fstat, openat, statat};

    let observed = statat(parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(io_error)?;
    validate_leaf_stat(&observed, owner, max_bytes)?;
    let observed_snapshot = stat_snapshot(&observed);
    let fd = openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(io_error)?;
    let stat = fstat(&fd).map_err(io_error)?;
    validate_leaf_stat(&stat, owner, max_bytes)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
        || stat_snapshot(&stat) != observed_snapshot
    {
        return Err(HistoricalPriceOnlyV3ArtifactError::UnsafePath);
    }
    Ok((std::fs::File::from(fd), observed_snapshot))
}

#[cfg(unix)]
fn validate_leaf_stat(
    stat: &rustix::fs::Stat,
    owner: rustix::process::RawUid,
    max_bytes: usize,
) -> Result<(), HistoricalPriceOnlyV3ArtifactError> {
    use rustix::fs::{FileType, Mode};
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
        || stat.st_uid != owner
        || Mode::from_raw_mode(stat.st_mode).bits() != 0o600
        || stat.st_nlink != 1
        || stat.st_size < 0
        || (stat.st_size as u128) > max_bytes as u128
    {
        return Err(HistoricalPriceOnlyV3ArtifactError::UnsafePath);
    }
    Ok(())
}

#[cfg(unix)]
fn revalidate_named_identity(
    parent: &impl std::os::fd::AsFd,
    name: &[u8],
    expected: &StatSnapshot,
) -> Result<(), HistoricalPriceOnlyV3ArtifactError> {
    use rustix::fs::{AtFlags, statat};
    let stat = statat(parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(io_error)?;
    if !same_stable_identity(&stat_snapshot(&stat), expected) {
        return Err(HistoricalPriceOnlyV3ArtifactError::UnsafePath);
    }
    Ok(())
}

#[cfg(unix)]
fn revalidate_named(
    parent: &impl std::os::fd::AsFd,
    name: &[u8],
    expected: &StatSnapshot,
) -> Result<(), HistoricalPriceOnlyV3ArtifactError> {
    use rustix::fs::{AtFlags, statat};
    let stat = statat(parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(io_error)?;
    if stat_snapshot(&stat) != *expected {
        return Err(HistoricalPriceOnlyV3ArtifactError::UnsafePath);
    }
    Ok(())
}

#[cfg(unix)]
fn revalidate_leaf(
    parent: &impl std::os::fd::AsFd,
    name: &[u8],
    file: &std::fs::File,
    expected: &StatSnapshot,
) -> Result<(), HistoricalPriceOnlyV3ArtifactError> {
    use rustix::fs::fstat;
    if stat_snapshot(&fstat(file).map_err(io_error)?) != *expected {
        return Err(HistoricalPriceOnlyV3ArtifactError::UnsafePath);
    }
    revalidate_named(parent, name, expected)
}

#[cfg(unix)]
fn enumerate_directory_entries(
    directory: &std::os::fd::OwnedFd,
) -> Result<Vec<Vec<u8>>, HistoricalPriceOnlyV3ArtifactError> {
    use rustix::fs::RawDir;
    use rustix::io::dup;
    use std::mem::MaybeUninit;

    rewind_directory(directory)?;
    let duplicate = dup(directory).map_err(io_error)?;
    let mut storage = [MaybeUninit::<u8>::uninit(); 4096];
    let mut dir = RawDir::new(&duplicate, &mut storage);
    let mut entries = Vec::new();
    while let Some(entry) = dir.next() {
        let entry = entry.map_err(io_error)?;
        let name = entry.file_name().to_bytes();
        if name == b"." || name == b".." {
            continue;
        }
        let name = name.to_vec();
        if entries.contains(&name) {
            return Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact);
        }
        entries.push(name);
    }
    entries.sort_unstable();
    Ok(entries)
}

#[cfg(unix)]
fn rewind_directory(
    directory: &std::os::fd::OwnedFd,
) -> Result<(), HistoricalPriceOnlyV3ArtifactError> {
    use rustix::fs::{SeekFrom, seek};
    seek(directory, SeekFrom::Start(0))
        .map(|_| ())
        .map_err(io_error)
}

#[cfg(unix)]
fn exact_artifact_entries(
    directory: &std::os::fd::OwnedFd,
) -> Result<(), HistoricalPriceOnlyV3ArtifactError> {
    let entries = enumerate_directory_entries(directory)?;
    if entries != [b"bars.ndjson".to_vec(), b"manifest.json".to_vec()] {
        return Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact);
    }
    Ok(())
}

#[cfg(unix)]
fn read_bounded(
    file: &std::fs::File,
    size: i128,
    max_bytes: usize,
) -> Result<Vec<u8>, HistoricalPriceOnlyV3ArtifactError> {
    if size < 0 || size > max_bytes as i128 {
        return Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact);
    }
    let size =
        usize::try_from(size).map_err(|_| HistoricalPriceOnlyV3ArtifactError::InvalidArtifact)?;
    let mut bytes = Vec::with_capacity(size);
    let mut remaining = size;
    let mut chunk = [0u8; 8192];
    while remaining != 0 {
        let amount = remaining.min(chunk.len());
        let read = (&*file)
            .read(&mut chunk[..amount])
            .map_err(HistoricalPriceOnlyV3ArtifactError::Io)?;
        if read == 0 {
            return Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact);
        }
        bytes.extend_from_slice(&chunk[..read]);
        remaining -= read;
    }
    Ok(bytes)
}

#[cfg(unix)]
fn stream_validate_bars(
    file: &std::fs::File,
    size: i128,
    range_start: TradingDate,
    range_end: TradingDate,
    instruments: &[String],
    expected_hash: &ContentHash,
    expected_bytes: Option<&[u8]>,
) -> Result<Vec<HistoricalPriceOnlyBar>, HistoricalPriceOnlyV3ArtifactError> {
    if size < 0 || size > MAX_BARS_BYTES as i128 {
        return Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact);
    }
    let sessions = approved_sessions(range_start, range_end)?;
    let mut validator = BarValidator::new(&sessions, instruments, Some(expected_hash));
    let mut comparator = expected_bytes.map(ExpectedBytesComparator::new);
    let mut remaining =
        usize::try_from(size).map_err(|_| HistoricalPriceOnlyV3ArtifactError::InvalidArtifact)?;
    let mut chunk = [0u8; 8192];
    while remaining != 0 {
        let amount = remaining.min(chunk.len());
        let read = (&*file)
            .read(&mut chunk[..amount])
            .map_err(HistoricalPriceOnlyV3ArtifactError::Io)?;
        if read == 0 {
            return Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact);
        }
        if let Some(comparator) = comparator.as_mut() {
            comparator.feed(&chunk[..read])?;
        }
        validator.feed(&chunk[..read])?;
        remaining -= read;
    }
    let bars = validator.finish()?;
    if let Some(comparator) = comparator {
        comparator.finish()?;
    }
    Ok(bars)
}

#[cfg(unix)]
struct ExpectedBytesComparator<'a> {
    expected: &'a [u8],
    offset: usize,
}

#[cfg(unix)]
impl<'a> ExpectedBytesComparator<'a> {
    fn new(expected: &'a [u8]) -> Self {
        Self {
            expected,
            offset: 0,
        }
    }

    fn feed(&mut self, actual: &[u8]) -> Result<(), HistoricalPriceOnlyV3ArtifactError> {
        let end = self
            .offset
            .checked_add(actual.len())
            .ok_or(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact)?;
        if end > self.expected.len() || self.expected[self.offset..end] != *actual {
            return Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact);
        }
        self.offset = end;
        Ok(())
    }

    fn finish(self) -> Result<(), HistoricalPriceOnlyV3ArtifactError> {
        if self.offset == self.expected.len() {
            Ok(())
        } else {
            Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact)
        }
    }
}

#[cfg(unix)]
fn read_artifact_unix(
    operator_root: &Path,
    candidate_hash: &ContentHash,
    expected: Option<&ArtifactBytes>,
) -> Result<VerifiedHistoricalPriceOnlyV3Artifact, HistoricalPriceOnlyV3ArtifactError> {
    use rustix::fs::fstat;
    use rustix::process::geteuid;

    let candidate_name = candidate_directory_name(candidate_hash)?;
    let root = open_trusted_root(operator_root)?;
    let owner = geteuid().as_raw();
    let (candidate_fd, candidate_stat) =
        open_directory_at(root.fd(), candidate_name.as_bytes(), owner, Some(0o700))?;
    exact_artifact_entries(&candidate_fd)?;

    let (manifest_file, manifest_snapshot) =
        open_leaf(&candidate_fd, b"manifest.json", owner, MAX_MANIFEST_BYTES)?;
    let manifest_bytes = read_bounded(&manifest_file, manifest_snapshot.size, MAX_MANIFEST_BYTES)?;
    if expected.is_some_and(|bytes| bytes.manifest_json != manifest_bytes) {
        return Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact);
    }
    revalidate_leaf(
        &candidate_fd,
        b"manifest.json",
        &manifest_file,
        &manifest_snapshot,
    )?;
    let validated = validate_manifest_bytes(candidate_hash, &manifest_bytes)?;
    let bars_expected = &validated.unsigned.bars;

    let (bars_file, bars_snapshot) =
        open_leaf(&candidate_fd, b"bars.ndjson", owner, MAX_BARS_BYTES)?;
    if bars_expected.size_bytes
        != u64::try_from(bars_snapshot.size)
            .map_err(|_| HistoricalPriceOnlyV3ArtifactError::InvalidArtifact)?
    {
        return Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact);
    }
    let approved_bars = stream_validate_bars(
        &bars_file,
        bars_snapshot.size,
        validated.unsigned.range_start,
        validated.unsigned.range_end,
        &validated.unsigned.instruments,
        &bars_expected.sha256,
        expected.map(|bytes| bytes.bars_ndjson.as_slice()),
    )?;
    revalidate_leaf(&candidate_fd, b"bars.ndjson", &bars_file, &bars_snapshot)?;

    // Check the exact tree and every held identity again immediately before
    // returning.  Reads always came from descriptors; these checks close the
    // pathname replacement window around the handoff.
    exact_artifact_entries(&candidate_fd)?;
    revalidate_leaf(
        &candidate_fd,
        b"manifest.json",
        &manifest_file,
        &manifest_snapshot,
    )?;
    revalidate_leaf(&candidate_fd, b"bars.ndjson", &bars_file, &bars_snapshot)?;
    let candidate_stat_now = fstat(&candidate_fd).map_err(io_error)?;
    if !same_stable_identity(
        &stat_snapshot(&candidate_stat_now),
        &stat_snapshot(&candidate_stat),
    ) {
        return Err(HistoricalPriceOnlyV3ArtifactError::UnsafePath);
    }
    root.revalidate()?;
    revalidate_named_identity(
        root.fd(),
        candidate_name.as_bytes(),
        &stat_snapshot(&candidate_stat),
    )?;
    let path = operator_root.join(&candidate_name);
    Ok(VerifiedHistoricalPriceOnlyV3Artifact {
        path,
        candidate_content_sha256: candidate_hash.clone(),
        approval_summary: validated.approval_summary,
        approved_bars,
    })
}

#[cfg(unix)]
struct StagingDirectory {
    fd: std::os::fd::OwnedFd,
    name: Vec<u8>,
    snapshot: StatSnapshot,
}

#[cfg(unix)]
#[derive(Clone)]
struct WrittenStaging {
    manifest: Option<StatSnapshot>,
    bars: Option<StatSnapshot>,
}

#[cfg(unix)]
fn write_artifact_unix(
    operator_root: &Path,
    candidate_hash: &ContentHash,
    expected: &ArtifactBytes,
) -> Result<VerifiedHistoricalPriceOnlyV3Artifact, HistoricalPriceOnlyV3ArtifactError> {
    use rustix::fs::{fstat, fsync};
    use rustix::process::geteuid;

    let candidate_name = candidate_directory_name(candidate_hash)?;
    let root = open_trusted_root(operator_root)?;
    let owner = geteuid().as_raw();

    // An already committed exact artifact is idempotent.  A partial or
    // different destination is never repaired or overwritten.
    if destination_exists(&root.fd(), candidate_name.as_bytes())? {
        return match read_artifact_unix(operator_root, candidate_hash, Some(expected)) {
            Ok(artifact) => Ok(artifact),
            Err(HistoricalPriceOnlyV3ArtifactError::UnsafePath) => {
                Err(HistoricalPriceOnlyV3ArtifactError::UnsafePath)
            }
            Err(_) => Err(HistoricalPriceOnlyV3ArtifactError::Conflict {
                candidate_content_sha256: candidate_hash.clone(),
            }),
        };
    }

    let staging = create_staging_directory(root.fd(), owner)?;
    let written = match populate_staging(&staging, owner, expected) {
        Ok(written) => written,
        Err(error) => {
            return fail_before_publish(&root.fd(), &staging, Some(&error.written), error.error);
        }
    };
    if let Err(error) = revalidate_staging(&root.fd(), &staging, &written) {
        return fail_before_publish(&root.fd(), &staging, Some(&written), error);
    }
    if let Err(error) = root.revalidate() {
        return fail_before_publish(&root.fd(), &staging, Some(&written), error);
    }
    match fstat(&staging.fd)
        .map_err(io_error)
        .map(|stat| stat_snapshot(&stat))
    {
        Ok(snapshot) if same_stable_identity(&snapshot, &staging.snapshot) => {}
        Ok(_) => {
            return fail_before_publish(
                &root.fd(),
                &staging,
                Some(&written),
                HistoricalPriceOnlyV3ArtifactError::UnsafePath,
            );
        }
        Err(error) => {
            return fail_before_publish(&root.fd(), &staging, Some(&written), error);
        }
    }
    if let Err(error) = fsync(&staging.fd).map_err(io_error) {
        return fail_before_publish(&root.fd(), &staging, Some(&written), error);
    }
    if let Err(error) = revalidate_staging(&root.fd(), &staging, &written) {
        return fail_before_publish(&root.fd(), &staging, Some(&written), error);
    }

    let publication = match publish_noreplace(&root.fd(), &staging.name, candidate_name.as_bytes())
    {
        Ok(publication) => publication,
        Err(error) => {
            return fail_before_publish(&root.fd(), &staging, Some(&written), error);
        }
    };
    match publication {
        PublishOutcome::Published => {
            if root.revalidate().is_err() {
                return Err(HistoricalPriceOnlyV3ArtifactError::IndeterminateCommit);
            }
            if fsync(root.fd()).map_err(io_error).is_err() {
                return Err(HistoricalPriceOnlyV3ArtifactError::IndeterminateCommit);
            }
            match read_artifact_unix(operator_root, candidate_hash, Some(expected)) {
                Ok(artifact) => Ok(artifact),
                Err(_) => Err(HistoricalPriceOnlyV3ArtifactError::IndeterminateCommit),
            }
        }
        PublishOutcome::DestinationExists => {
            cleanup_and_sync(&root.fd(), &staging, Some(&written))?;
            match read_artifact_unix(operator_root, candidate_hash, Some(expected)) {
                Ok(artifact) => Ok(artifact),
                Err(HistoricalPriceOnlyV3ArtifactError::UnsafePath) => {
                    Err(HistoricalPriceOnlyV3ArtifactError::UnsafePath)
                }
                Err(_) => Err(HistoricalPriceOnlyV3ArtifactError::Conflict {
                    candidate_content_sha256: candidate_hash.clone(),
                }),
            }
        }
    }
}

#[cfg(unix)]
fn destination_exists(
    parent: &impl std::os::fd::AsFd,
    name: &[u8],
) -> Result<bool, HistoricalPriceOnlyV3ArtifactError> {
    use rustix::fs::{AtFlags, statat};
    match statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(_) => Ok(true),
        Err(error) if error == rustix::io::Errno::NOENT => Ok(false),
        Err(error) if error == rustix::io::Errno::LOOP => {
            Err(HistoricalPriceOnlyV3ArtifactError::UnsafePath)
        }
        Err(error) => Err(io_error(error)),
    }
}

#[cfg(unix)]
fn create_staging_directory(
    parent: &impl std::os::fd::AsFd,
    owner: rustix::process::RawUid,
) -> Result<StagingDirectory, HistoricalPriceOnlyV3ArtifactError> {
    use rustix::fs::{Mode, fsync, mkdirat};

    static NEXT_STAGE: AtomicU64 = AtomicU64::new(1);
    for _ in 0..128 {
        let sequence = NEXT_STAGE.fetch_add(1, Ordering::Relaxed);
        let name = format!(".stage-pid-{}-{}", std::process::id(), sequence).into_bytes();
        match mkdirat(parent, name.as_slice(), Mode::from_raw_mode(0o700)) {
            Ok(()) => {
                let (fd, stat) = open_directory_at(parent, &name, owner, Some(0o700))?;
                let staging = StagingDirectory {
                    fd,
                    name,
                    snapshot: stat_snapshot(&stat),
                };
                if let Err(error) = fsync(parent).map_err(io_error) {
                    let _ = cleanup_staging_tree(parent, &staging, None);
                    return Err(error);
                }
                return Ok(staging);
            }
            Err(error) if error == rustix::io::Errno::EXIST => continue,
            Err(error) => return Err(io_error(error)),
        }
    }
    Err(HistoricalPriceOnlyV3ArtifactError::StagingNameExhausted)
}

#[cfg(unix)]
struct StagingPopulationError {
    error: HistoricalPriceOnlyV3ArtifactError,
    written: WrittenStaging,
}

#[cfg(unix)]
#[allow(clippy::result_large_err)]
fn populate_staging(
    staging: &StagingDirectory,
    owner: rustix::process::RawUid,
    expected: &ArtifactBytes,
) -> Result<WrittenStaging, StagingPopulationError> {
    let manifest = match write_staging_file(
        &staging.fd,
        b"manifest.json",
        &expected.manifest_json,
        owner,
        MAX_MANIFEST_BYTES,
    ) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return Err(StagingPopulationError {
                error: error.error,
                written: WrittenStaging {
                    manifest: error.snapshot,
                    bars: None,
                },
            });
        }
    };
    let bars = match write_staging_file(
        &staging.fd,
        b"bars.ndjson",
        &expected.bars_ndjson,
        owner,
        MAX_BARS_BYTES,
    ) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return Err(StagingPopulationError {
                error: error.error,
                written: WrittenStaging {
                    manifest: Some(manifest),
                    bars: error.snapshot,
                },
            });
        }
    };
    let written = WrittenStaging {
        manifest: Some(manifest),
        bars: Some(bars),
    };
    if enumerate_directory_entries(&staging.fd).map_err(|error| StagingPopulationError {
        error,
        written: written.clone(),
    })? != [b"bars.ndjson".to_vec(), b"manifest.json".to_vec()]
    {
        return Err(StagingPopulationError {
            error: HistoricalPriceOnlyV3ArtifactError::InvalidArtifact,
            written,
        });
    }
    Ok(written)
}

#[cfg(unix)]
struct StagingWriteFailure {
    error: HistoricalPriceOnlyV3ArtifactError,
    snapshot: Option<StatSnapshot>,
}

#[cfg(unix)]
fn write_staging_file(
    parent: &impl std::os::fd::AsFd,
    name: &[u8],
    bytes: &[u8],
    owner: rustix::process::RawUid,
    max_bytes: usize,
) -> Result<StatSnapshot, StagingWriteFailure> {
    use rustix::fs::{Mode, OFlags, fstat, fsync, openat};

    if bytes.len() > max_bytes {
        return Err(StagingWriteFailure {
            error: HistoricalPriceOnlyV3ArtifactError::InvalidArtifact,
            snapshot: None,
        });
    }
    let fd = openat(
        parent,
        name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )
    .map_err(|error| StagingWriteFailure {
        error: io_error(error),
        snapshot: None,
    })?;
    let mut file = std::fs::File::from(fd);
    let initial = fstat(&file).map_err(|error| StagingWriteFailure {
        error: io_error(error),
        snapshot: None,
    })?;
    validate_leaf_stat(&initial, owner, max_bytes).map_err(|error| StagingWriteFailure {
        error,
        snapshot: None,
    })?;
    let initial_snapshot = stat_snapshot(&initial);
    revalidate_named(parent, name, &initial_snapshot).map_err(|error| StagingWriteFailure {
        error,
        snapshot: None,
    })?;
    if let Err(error) = file.write_all(bytes) {
        return Err(StagingWriteFailure {
            error: HistoricalPriceOnlyV3ArtifactError::Io(error),
            snapshot: safe_current_leaf_snapshot(&file, parent, name, owner, max_bytes),
        });
    }
    if let Err(error) = fsync(&file).map_err(io_error) {
        return Err(StagingWriteFailure {
            error,
            snapshot: safe_current_leaf_snapshot(&file, parent, name, owner, max_bytes),
        });
    }
    let final_stat = fstat(&file).map_err(|error| StagingWriteFailure {
        error: io_error(error),
        snapshot: Some(initial_snapshot.clone()),
    })?;
    validate_leaf_stat(&final_stat, owner, max_bytes).map_err(|error| StagingWriteFailure {
        error,
        snapshot: Some(initial_snapshot.clone()),
    })?;
    let final_snapshot = stat_snapshot(&final_stat);
    if final_snapshot.size != bytes.len() as i128 {
        return Err(StagingWriteFailure {
            error: HistoricalPriceOnlyV3ArtifactError::InvalidArtifact,
            snapshot: Some(final_snapshot),
        });
    }
    revalidate_named(parent, name, &final_snapshot).map_err(|error| StagingWriteFailure {
        error,
        snapshot: None,
    })?;
    Ok(final_snapshot)
}

#[cfg(unix)]
fn safe_current_leaf_snapshot(
    file: &std::fs::File,
    parent: &impl std::os::fd::AsFd,
    name: &[u8],
    owner: rustix::process::RawUid,
    max_bytes: usize,
) -> Option<StatSnapshot> {
    use rustix::fs::fstat;

    let stat = fstat(file).ok()?;
    validate_leaf_stat(&stat, owner, max_bytes).ok()?;
    let snapshot = stat_snapshot(&stat);
    revalidate_named(parent, name, &snapshot).ok()?;
    Some(snapshot)
}

#[cfg(unix)]
fn revalidate_staging(
    parent: &impl std::os::fd::AsFd,
    staging: &StagingDirectory,
    written: &WrittenStaging,
) -> Result<(), HistoricalPriceOnlyV3ArtifactError> {
    use rustix::fs::fstat;

    let (Some(manifest), Some(bars)) = (&written.manifest, &written.bars) else {
        return Err(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact);
    };
    revalidate_named_identity(parent, &staging.name, &staging.snapshot)?;
    if !same_stable_identity(
        &stat_snapshot(&fstat(&staging.fd).map_err(io_error)?),
        &staging.snapshot,
    ) {
        return Err(HistoricalPriceOnlyV3ArtifactError::UnsafePath);
    }
    exact_artifact_entries(&staging.fd)?;
    revalidate_named(&staging.fd, b"manifest.json", manifest)?;
    revalidate_named(&staging.fd, b"bars.ndjson", bars)
}

#[cfg(unix)]
fn fail_before_publish<T>(
    parent: &impl std::os::fd::AsFd,
    staging: &StagingDirectory,
    written: Option<&WrittenStaging>,
    error: HistoricalPriceOnlyV3ArtifactError,
) -> Result<T, HistoricalPriceOnlyV3ArtifactError> {
    if cleanup_and_sync(parent, staging, written).is_err() {
        Err(HistoricalPriceOnlyV3ArtifactError::CleanupFailed)
    } else {
        Err(error)
    }
}

#[cfg(unix)]
fn cleanup_and_sync(
    parent: &impl std::os::fd::AsFd,
    staging: &StagingDirectory,
    written: Option<&WrittenStaging>,
) -> Result<(), HistoricalPriceOnlyV3ArtifactError> {
    cleanup_staging_tree(parent, staging, written)?;
    rustix::fs::fsync(parent).map_err(io_error)
}

#[cfg(unix)]
fn cleanup_staging_tree(
    parent: &impl std::os::fd::AsFd,
    staging: &StagingDirectory,
    written: Option<&WrittenStaging>,
) -> Result<(), HistoricalPriceOnlyV3ArtifactError> {
    use rustix::fs::{AtFlags, statat, unlinkat};

    revalidate_named_identity(parent, &staging.name, &staging.snapshot)
        .map_err(|_| HistoricalPriceOnlyV3ArtifactError::CleanupFailed)?;
    let entries = enumerate_directory_entries(&staging.fd)
        .map_err(|_| HistoricalPriceOnlyV3ArtifactError::CleanupFailed)?;
    let expected_names = match written {
        Some(written) => {
            let mut names = Vec::new();
            if written.bars.is_some() {
                names.push(b"bars.ndjson".to_vec());
            }
            if written.manifest.is_some() {
                names.push(b"manifest.json".to_vec());
            }
            names.sort_unstable();
            names
        }
        None => Vec::new(),
    };
    if entries != expected_names {
        return Err(HistoricalPriceOnlyV3ArtifactError::CleanupFailed);
    }
    let mut snapshots = Vec::new();
    if let Some(written) = written {
        if let Some(snapshot) = &written.bars {
            snapshots.push((b"bars.ndjson".as_slice(), snapshot));
        }
        if let Some(snapshot) = &written.manifest {
            snapshots.push((b"manifest.json".as_slice(), snapshot));
        }
    }
    snapshots.sort_unstable_by(|left, right| left.0.cmp(right.0));
    for (name, expected) in &snapshots {
        let current = statat(&staging.fd, *name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|_| HistoricalPriceOnlyV3ArtifactError::CleanupFailed)?;
        if stat_snapshot(&current) != **expected {
            return Err(HistoricalPriceOnlyV3ArtifactError::CleanupFailed);
        }
    }
    if !entries.is_empty() {
        // Enumerate and snapshot again immediately before unlinking.  This
        // prevents cleanup from following a replacement or deleting an
        // unexpected entry if the staging directory was tampered with.
        if enumerate_directory_entries(&staging.fd)
            .map_err(|_| HistoricalPriceOnlyV3ArtifactError::CleanupFailed)?
            != expected_names
        {
            return Err(HistoricalPriceOnlyV3ArtifactError::CleanupFailed);
        }
        for (name, expected) in &snapshots {
            let current = statat(&staging.fd, *name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|_| HistoricalPriceOnlyV3ArtifactError::CleanupFailed)?;
            if stat_snapshot(&current) != **expected {
                return Err(HistoricalPriceOnlyV3ArtifactError::CleanupFailed);
            }
        }
        for (name, _) in &snapshots {
            unlinkat(&staging.fd, *name, AtFlags::empty())
                .map_err(|_| HistoricalPriceOnlyV3ArtifactError::CleanupFailed)?;
        }
    }
    if !enumerate_directory_entries(&staging.fd)
        .map_err(|_| HistoricalPriceOnlyV3ArtifactError::CleanupFailed)?
        .is_empty()
    {
        return Err(HistoricalPriceOnlyV3ArtifactError::CleanupFailed);
    }
    revalidate_named_identity(parent, &staging.name, &staging.snapshot)
        .map_err(|_| HistoricalPriceOnlyV3ArtifactError::CleanupFailed)?;
    unlinkat(parent, &staging.name, AtFlags::REMOVEDIR)
        .map_err(|_| HistoricalPriceOnlyV3ArtifactError::CleanupFailed)
}

#[cfg(unix)]
enum PublishOutcome {
    Published,
    DestinationExists,
}

#[cfg(all(
    unix,
    any(
        target_os = "android",
        target_os = "linux",
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "visionos",
        target_os = "watchos",
        target_os = "redox"
    )
))]
fn publish_noreplace(
    parent: &impl std::os::fd::AsFd,
    staging: &[u8],
    destination: &[u8],
) -> Result<PublishOutcome, HistoricalPriceOnlyV3ArtifactError> {
    use rustix::fs::{RenameFlags, renameat_with};
    match renameat_with(parent, staging, parent, destination, RenameFlags::NOREPLACE) {
        Ok(()) => Ok(PublishOutcome::Published),
        Err(error) if error == rustix::io::Errno::EXIST => Ok(PublishOutcome::DestinationExists),
        Err(error)
            if error == rustix::io::Errno::NOSYS
                || error == rustix::io::Errno::INVAL
                || error == rustix::io::Errno::NOTSUP
                || error == rustix::io::Errno::OPNOTSUPP =>
        {
            Err(HistoricalPriceOnlyV3ArtifactError::UnsupportedAtomicNoReplace)
        }
        Err(error) => Err(io_error(error)),
    }
}

#[cfg(all(
    unix,
    not(any(
        target_os = "android",
        target_os = "linux",
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "visionos",
        target_os = "watchos",
        target_os = "redox"
    ))
))]
fn publish_noreplace(
    _parent: &impl std::os::fd::AsFd,
    _staging: &[u8],
    _destination: &[u8],
) -> Result<PublishOutcome, HistoricalPriceOnlyV3ArtifactError> {
    Err(HistoricalPriceOnlyV3ArtifactError::UnsupportedAtomicNoReplace)
}

/// Typed, sanitized failures from the V3 artifact boundary.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HistoricalPriceOnlyV3ArtifactError {
    #[error("historical price-only V3 artifact requires Unix support")]
    UnsupportedPlatform,
    #[error("historical price-only V3 artifact path is unsafe")]
    UnsafePath,
    #[error("historical price-only V3 candidate does not match the fixed contract")]
    InvalidCandidate,
    #[error("historical price-only V3 artifact bytes violate the fixed contract")]
    InvalidArtifact,
    #[error("historical price-only V3 artifact filesystem error")]
    Io(#[source] std::io::Error),
    #[error("historical price-only V3 artifact destination already exists")]
    Conflict {
        candidate_content_sha256: ContentHash,
    },
    #[error("historical price-only V3 artifact atomic no-replace publication is unavailable")]
    UnsupportedAtomicNoReplace,
    #[error("historical price-only V3 artifact commit state is indeterminate")]
    IndeterminateCommit,
    #[error("historical price-only V3 artifact staging cleanup failed")]
    CleanupFailed,
    #[error("historical price-only V3 artifact staging name allocation exhausted")]
    StagingNameExhausted,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_timestamp() -> UtcTimestamp {
        UtcTimestamp::parse_rfc3339("2026-08-29T00:00:00Z").unwrap()
    }

    fn test_batch_id(value: &str) -> BatchId {
        BatchId::from_str(value).unwrap()
    }

    fn test_candidate_hash() -> ContentHash {
        ContentHash::from_bytes(b"historical-price-only-v3-artifact-test-candidate")
    }

    fn test_unsigned_manifest(candidate_hash: &ContentHash) -> UnsignedManifest {
        UnsignedManifest {
            schema_id: HISTORICAL_PRICE_ONLY_V3_ARTIFACT_SCHEMA_ID.to_owned(),
            schema_version: HISTORICAL_PRICE_ONLY_V3_ARTIFACT_SCHEMA_VERSION,
            contract: HISTORICAL_PRICE_ONLY_V3_CONTRACT.to_owned(),
            materializer_version: HISTORICAL_PRICE_ONLY_V3_MATERIALIZER_VERSION.to_owned(),
            candidate_content_sha256: candidate_hash.clone(),
            audience: "OWNER_ONLY".to_owned(),
            vendor_snapshot: true,
            strict_pit: false,
            capability: "PRICE_RETURN_ONLY".to_owned(),
            materialization_status: "MATERIALIZED".to_owned(),
            registration_status: "UNREGISTERED".to_owned(),
            publication_status: "NOT_PUBLISHED".to_owned(),
            range_start: expected_date(EXPECTED_RANGE_START),
            range_end: expected_date(EXPECTED_RANGE_END),
            instruments: expected_instruments(),
            instrument_count: 11,
            session_count: EXPECTED_SESSION_COUNT,
            row_count: EXPECTED_ROW_COUNT,
            price_scale: EXPECTED_PRICE_SCALE,
            factor_scale: EXPECTED_FACTOR_SCALE,
            price_source: PriceSourceDto {
                batch_id: test_batch_id(HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_ID),
                batch_json_sha256: expected_hash(HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_JSON_SHA256),
                manifest_line_sha256: expected_hash(
                    HISTORICAL_PRICE_ONLY_V3_PRICE_MANIFEST_LINE_SHA256,
                ),
                bars_sha256: expected_hash(EXPECTED_SOURCE_BARS_SHA256),
                file_count: HISTORICAL_PRICE_ONLY_V3_PRICE_FILE_COUNT,
                capture_contract_commit: HISTORICAL_PRICE_ONLY_V3_PRICE_CAPTURE_COMMIT.to_owned(),
                response_marker_evidence: HISTORICAL_PRICE_ONLY_V3_PRICE_RESPONSE_MARKER_EVIDENCE
                    .to_owned(),
                acquired_at: test_timestamp(),
            },
            action_source: ActionSourceDto {
                batch_id: test_batch_id(HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_ID),
                batch_json_sha256: expected_hash(HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_JSON_SHA256),
                manifest_line_sha256: expected_hash(
                    HISTORICAL_PRICE_ONLY_V3_ACTION_MANIFEST_LINE_SHA256,
                ),
                file_count: HISTORICAL_PRICE_ONLY_V3_ACTION_FILE_COUNT,
                action_count: EXPECTED_ACTION_COUNT,
                cash_dividend_treatment_id: HISTORICAL_PRICE_ONLY_V3_CASH_DIVIDEND_TREATMENT
                    .to_owned(),
                cash_dividend_row_count: EXPECTED_CASH_ROW_COUNT,
                cash_dividend_rows_sha256: expected_hash(EXPECTED_CASH_ROWS_SHA256),
                acquired_at: test_timestamp(),
            },
            bars: BarsDto {
                relative_path: "bars.ndjson".to_owned(),
                schema_id: "historical-price-only-bars".to_owned(),
                schema_version: 1,
                sha256: ContentHash::from_bytes(b"fixture-bars"),
                size_bytes: 0,
                row_count: EXPECTED_ROW_COUNT,
            },
        }
    }

    fn encode_manifest(unsigned: UnsignedManifest) -> Vec<u8> {
        let manifest = Manifest::from_unsigned(unsigned).unwrap();
        let mut bytes = serde_json::to_vec(&manifest).unwrap();
        bytes.push(b'\n');
        bytes
    }

    fn test_bar(instrument_id: &str, session_date: TradingDate) -> BarDto {
        BarDto {
            instrument_id: instrument_id.to_owned(),
            session_date,
            raw_open: "1".to_owned(),
            raw_high: "2".to_owned(),
            raw_low: "1".to_owned(),
            raw_close: "2".to_owned(),
            raw_volume: 1,
            raw_trading_value: Some("10".to_owned()),
            adjusted_open: "1".to_owned(),
            adjusted_high: "2".to_owned(),
            adjusted_low: "1".to_owned(),
            adjusted_close: "2".to_owned(),
        }
    }

    fn encode_bar(row: &BarDto) -> Vec<u8> {
        let mut bytes = serde_json::to_vec(row).unwrap();
        bytes.push(b'\n');
        bytes
    }

    fn small_fixture() -> (Vec<TradingDate>, Vec<String>) {
        let sessions = vec![expected_date("2026-08-28")];
        let instruments = vec!["069500.KRX".to_owned()];
        (sessions, instruments)
    }

    #[test]
    fn canonical_manifest_round_trip_binds_fixed_contract_and_range() {
        let candidate_hash = test_candidate_hash();
        let bytes = encode_manifest(test_unsigned_manifest(&candidate_hash));
        let validated = validate_manifest_bytes(&candidate_hash, &bytes).unwrap();
        let mut canonical =
            serde_json::to_vec(&Manifest::from_unsigned(validated.unsigned).unwrap()).unwrap();
        canonical.push(b'\n');
        assert_eq!(canonical, bytes);
        assert_eq!(validated.approval_summary.schema_version(), 3);
        assert_eq!(
            validated.approval_summary.contract(),
            HISTORICAL_PRICE_ONLY_V3_CONTRACT
        );
        assert_eq!(
            validated.approval_summary.session_count(),
            EXPECTED_SESSION_COUNT
        );
    }

    #[test]
    fn manifest_rejects_unknown_tamper_hash_count_range_and_contract_version() {
        let candidate_hash = test_candidate_hash();
        let original = test_unsigned_manifest(&candidate_hash);
        let original_bytes = encode_manifest(original.clone());

        let mut unknown = original_bytes.clone();
        let insertion = unknown.iter().position(|byte| *byte == b'{').unwrap() + 1;
        unknown.splice(
            insertion..insertion,
            br#"\"unexpected\":true,"#.iter().copied(),
        );
        assert!(validate_manifest_bytes(&candidate_hash, &unknown).is_err());

        let mut hash_tampered: serde_json::Value = serde_json::from_slice(&original_bytes).unwrap();
        hash_tampered["manifest_sha256"] =
            serde_json::Value::String(ContentHash::from_bytes(b"tampered-manifest").to_string());
        let mut hash_tampered_bytes = serde_json::to_vec(&hash_tampered).unwrap();
        hash_tampered_bytes.push(b'\n');
        assert!(validate_manifest_bytes(&candidate_hash, &hash_tampered_bytes).is_err());

        let mut wrong_count = original.clone();
        wrong_count.row_count -= 1;
        assert!(validate_manifest_bytes(&candidate_hash, &encode_manifest(wrong_count)).is_err());

        let mut wrong_range = original.clone();
        wrong_range.range_end = expected_date("2026-08-27");
        assert!(validate_manifest_bytes(&candidate_hash, &encode_manifest(wrong_range)).is_err());

        let mut wrong_contract = original.clone();
        wrong_contract.contract = "wrong-contract".to_owned();
        assert!(
            validate_manifest_bytes(&candidate_hash, &encode_manifest(wrong_contract)).is_err()
        );

        let mut wrong_version = original;
        wrong_version.materializer_version = "wrong-materializer".to_owned();
        assert!(validate_manifest_bytes(&candidate_hash, &encode_manifest(wrong_version)).is_err());
    }

    #[test]
    fn bars_validator_accepts_one_complete_test_matrix() {
        let (sessions, instruments) = small_fixture();
        let mut validator = BarValidator::new_for_test(&sessions, &instruments);
        let bytes = encode_bar(&test_bar(&instruments[0], sessions[0]));
        validator.feed(&bytes).unwrap();
        let bars = validator.finish().unwrap();
        assert_eq!(bars.len(), 1);
        assert_eq!(bars[0].instrument_id.to_string(), instruments[0]);
        assert_eq!(bars[0].session_date, sessions[0]);
    }

    #[test]
    fn bars_validator_rejects_duplicate_gap_and_raw_adjusted_mismatch() {
        let (sessions, instruments) = small_fixture();
        let mut duplicate = BarValidator::new_for_test(&sessions, &instruments);
        let bytes = encode_bar(&test_bar(&instruments[0], sessions[0]));
        duplicate.feed(&bytes).unwrap();
        assert!(duplicate.feed(&bytes).is_err());

        let sessions_with_gap = vec![expected_date("2026-08-27"), expected_date("2026-08-28")];
        let instruments_for_gap = vec!["069500.KRX".to_owned()];
        let mut gap = BarValidator::new_for_test(&sessions_with_gap, &instruments_for_gap);
        gap.feed(&bytes).unwrap();
        assert!(gap.finish().is_err());

        let mut mismatched = test_bar(&instruments[0], sessions[0]);
        mismatched.adjusted_close = "1".to_owned();
        let raw_adjusted_sessions = vec![sessions[0]];
        let raw_adjusted_instruments = vec![instruments[0].clone()];
        let mut raw_adjusted =
            BarValidator::new_for_test(&raw_adjusted_sessions, &raw_adjusted_instruments);
        assert!(raw_adjusted.feed(&encode_bar(&mismatched)).is_err());
    }

    #[test]
    fn bars_validator_rejects_noncanonical_or_non_terminated_lines() {
        let (sessions, instruments) = small_fixture();
        let mut validator = BarValidator::new_for_test(&sessions, &instruments);
        let mut noncanonical = test_bar(&instruments[0], sessions[0]);
        noncanonical.raw_open = "01".to_owned();
        assert!(validator.feed(&encode_bar(&noncanonical)).is_err());

        let (sessions, instruments) = small_fixture();
        let mut unterminated = BarValidator::new_for_test(&sessions, &instruments);
        let bytes = encode_bar(&test_bar(&instruments[0], sessions[0]));
        unterminated.feed(&bytes[..bytes.len() - 1]).unwrap();
        assert!(unterminated.finish().is_err());
    }

    #[cfg(unix)]
    fn full_fixture() -> (ContentHash, ArtifactBytes) {
        let candidate_hash = test_candidate_hash();
        let sessions = approved_sessions(
            expected_date(EXPECTED_RANGE_START),
            expected_date(EXPECTED_RANGE_END),
        )
        .unwrap();
        let instruments = expected_instruments();
        let mut bars_ndjson = Vec::with_capacity(EXPECTED_ROW_COUNT * 150);
        for instrument in &instruments {
            for session in &sessions {
                bars_ndjson.extend_from_slice(&encode_bar(&test_bar(instrument, *session)));
            }
        }
        let mut unsigned = test_unsigned_manifest(&candidate_hash);
        unsigned.bars.sha256 = ContentHash::from_bytes(&bars_ndjson);
        unsigned.bars.size_bytes = bars_ndjson.len() as u64;
        let manifest_json = encode_manifest(unsigned);
        let artifact = ArtifactBytes {
            bars_ndjson,
            manifest_json,
        };
        validate_artifact_bytes(
            &candidate_hash,
            &artifact.bars_ndjson,
            &artifact.manifest_json,
        )
        .unwrap();
        (candidate_hash, artifact)
    }

    #[cfg(unix)]
    #[test]
    fn descriptor_safe_write_read_is_idempotent_and_conflict_safe() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o750)).unwrap();
        let (candidate_hash, artifact) = full_fixture();
        let first = write_artifact_unix(root.path(), &candidate_hash, &artifact).unwrap();
        let second = write_artifact_unix(root.path(), &candidate_hash, &artifact).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.bars().len(), EXPECTED_ROW_COUNT);
        assert_eq!(first.approval_summary().row_count(), EXPECTED_ROW_COUNT);

        let mut different = artifact.clone();
        different.bars_ndjson[0] = if different.bars_ndjson[0] == b'{' {
            b'['
        } else {
            b'{'
        };
        let error = write_artifact_unix(root.path(), &candidate_hash, &different).unwrap_err();
        assert!(matches!(
            error,
            HistoricalPriceOnlyV3ArtifactError::Conflict { .. }
        ));

        let reopened = read_artifact_unix(root.path(), &candidate_hash, None).unwrap();
        assert_eq!(reopened, first);
    }

    #[cfg(unix)]
    #[test]
    fn descriptor_safe_root_rejects_symlink_and_non_directory() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let link = root.path().join("link");
        symlink(&real, &link).unwrap();
        assert!(matches!(
            open_trusted_root(&link),
            Err(HistoricalPriceOnlyV3ArtifactError::UnsafePath)
        ));

        let file = root.path().join("file");
        std::fs::write(&file, b"not a directory").unwrap();
        assert!(matches!(
            open_trusted_root(&file),
            Err(HistoricalPriceOnlyV3ArtifactError::UnsafePath)
        ));
    }
}
