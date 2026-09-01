//! Shared, read-only approval boundary for the sealed historical price beta.

use std::{fmt, path::Path};

use domain::{BatchId, ContentHash, TradingDate, UtcTimestamp};
use serde::{Deserialize, Serialize};

use crate::{
    HistoricalPriceOnlyArtifactApprovalSummary, HistoricalPriceOnlyBar,
    HistoricalPriceOnlyV3ArtifactApprovalSummary, KR_ETF_CORE_SYMBOLS,
    read_historical_price_only_artifact, read_historical_price_only_v3_artifact,
};

use crate::historical_price_only_v3::{
    HISTORICAL_PRICE_ONLY_V3_CONTRACT, HISTORICAL_PRICE_ONLY_V3_MATERIALIZER_VERSION,
    HISTORICAL_PRICE_ONLY_V3_SCHEMA_ID,
};
use crate::historical_price_only_v3_artifact::{
    HISTORICAL_PRICE_ONLY_V3_ARTIFACT_SCHEMA_ID, HISTORICAL_PRICE_ONLY_V3_ARTIFACT_SCHEMA_VERSION,
};
use crate::range_to_canonical_v3::{
    HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_ID, HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_JSON_SHA256,
    HISTORICAL_PRICE_ONLY_V3_ACTION_FILE_COUNT,
    HISTORICAL_PRICE_ONLY_V3_ACTION_MANIFEST_LINE_SHA256,
    HISTORICAL_PRICE_ONLY_V3_CASH_DIVIDEND_TREATMENT,
};
use crate::range_to_canonical_v3_price::{
    HISTORICAL_PRICE_ONLY_V3_PRICE_BAR_COUNT, HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_ID,
    HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_JSON_SHA256,
    HISTORICAL_PRICE_ONLY_V3_PRICE_CAPTURE_COMMIT, HISTORICAL_PRICE_ONLY_V3_PRICE_FILE_COUNT,
    HISTORICAL_PRICE_ONLY_V3_PRICE_MANIFEST_LINE_SHA256, HISTORICAL_PRICE_ONLY_V3_PRICE_RANGE_END,
    HISTORICAL_PRICE_ONLY_V3_PRICE_RANGE_START,
    HISTORICAL_PRICE_ONLY_V3_PRICE_RESPONSE_MARKER_EVIDENCE,
    HISTORICAL_PRICE_ONLY_V3_PRICE_SESSION_COUNT,
};

const V2_REGISTRY_SCHEMA_ID: &str = "kis-historical-price-only-beta-approval-registry";
const V2_REGISTRY_SCHEMA_VERSION: u32 = 2;
const V3_REGISTRY_SCHEMA_ID: &str = "kis-historical-price-only-v3-approval-registry";
const V3_REGISTRY_SCHEMA_VERSION: u32 = 1;
const MAX_REGISTRY_BYTES: usize = 256 * 1024;
const V3_CANDIDATE_CONTENT_SHA256: &str =
    "sha256:b0fb82f6a580f3def13a1e1e34bea68e30d95dc720306e7ee54b6c3199cf402d";
const V3_ARTIFACT_MANIFEST_SHA256: &str =
    "sha256:53587f32ce67ae1a8488b9c00096465185d03e35b1b38a04f39ed5462da7f6b6";
const V3_PRICE_BARS_SHA256: &str =
    "sha256:20c750f0ca415073da37650ae2bb0c942a181b4c86f167defe95895e4499dcf2";
const V3_ARTIFACT_BARS_SHA256: &str =
    "sha256:23354c54708d7a458aadc4f2cb2fca77469969e91256b6d3b119101ebae98ffe";
const V3_CASH_ROWS_SHA256: &str =
    "sha256:b22a5c9808a8a1a2c892aa3ff46d529672c909620a2c45c0e46d48d0538d17e8";
const V3_PRICE_ACQUIRED_AT: &str = "2026-08-28T18:20:14Z";
const V3_ACTION_ACQUIRED_AT: &str = "2026-08-28T18:54:49Z";
const EMBEDDED_V2_REGISTRY: &[u8] = include_bytes!(
    "../../../configs/evidence/kis-historical-price-only-beta-approved-artifacts.json"
);
const EMBEDDED_V3_REGISTRY: &[u8] = include_bytes!(
    "../../../configs/evidence/kis-historical-price-only-v3-approved-artifacts.json"
);

// Keep the old private names available to the V2 unit tests and make their
// meaning explicit: all new entry points use the V3 registry by default.
const REGISTRY_SCHEMA_ID: &str = V2_REGISTRY_SCHEMA_ID;
const REGISTRY_SCHEMA_VERSION: u32 = V2_REGISTRY_SCHEMA_VERSION;
#[cfg(test)]
const EMBEDDED_REGISTRY: &[u8] = EMBEDDED_V2_REGISTRY;

/// The five immutable pins bound by the owner-only approval contract.
#[derive(Clone, PartialEq, Eq)]
pub struct HistoricalPriceOnlyArtifactPins {
    candidate_content_sha256: ContentHash,
    artifact_manifest_sha256: ContentHash,
    stage5_manifest_sha256: ContentHash,
    action_manifest_sha256: ContentHash,
    approval_registry_sha256: ContentHash,
}

impl HistoricalPriceOnlyArtifactPins {
    pub fn candidate_content_sha256(&self) -> &ContentHash {
        &self.candidate_content_sha256
    }
    pub fn artifact_manifest_sha256(&self) -> &ContentHash {
        &self.artifact_manifest_sha256
    }
    pub fn stage5_manifest_sha256(&self) -> &ContentHash {
        &self.stage5_manifest_sha256
    }
    pub fn action_manifest_sha256(&self) -> &ContentHash {
        &self.action_manifest_sha256
    }
    pub fn approval_registry_sha256(&self) -> &ContentHash {
        &self.approval_registry_sha256
    }
}

impl fmt::Debug for HistoricalPriceOnlyArtifactPins {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HistoricalPriceOnlyArtifactPins")
            .field("approval_registry_sha256", &self.approval_registry_sha256)
            .finish_non_exhaustive()
    }
}

/// Nonconstructible, owner-only artifact proven against the embedded registry.
pub struct ApprovedHistoricalPriceOnlyArtifact {
    pins: HistoricalPriceOnlyArtifactPins,
    bars: Vec<HistoricalPriceOnlyBar>,
}

impl ApprovedHistoricalPriceOnlyArtifact {
    pub fn pins(&self) -> &HistoricalPriceOnlyArtifactPins {
        &self.pins
    }
    pub fn bars(&self) -> &[HistoricalPriceOnlyBar] {
        &self.bars
    }
}

impl fmt::Debug for ApprovedHistoricalPriceOnlyArtifact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApprovedHistoricalPriceOnlyArtifact")
            .field("status", &"approved")
            .field("bar_count", &self.bars.len())
            .finish()
    }
}

/// Fail-closed approval errors intentionally omit paths and artifact contents.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HistoricalPriceOnlyApprovalError {
    #[error("historical price-only approval registry is invalid")]
    RegistryInvalid,
    #[error("historical price-only artifact is not approved")]
    ArtifactNotApproved,
    #[error("historical price-only approval registry is ambiguous")]
    RegistryAmbiguous,
    #[error("historical price-only approved artifact could not be verified")]
    ArtifactRejected,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Registry {
    schema_id: String,
    schema_version: u32,
    approved_artifacts: Vec<Record>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Record {
    candidate_content_sha256: ContentHash,
    artifact_manifest_sha256: ContentHash,
    stage5_manifest_sha256: ContentHash,
    action_manifest_sha256: ContentHash,
    cash_dividend_treatment_id: String,
    ignored_cash_dividend_row_count: usize,
    ignored_cash_dividend_rows_sha256: ContentHash,
    ignored_cash_dividend_source_file_sha256: ContentHash,
    ignored_cash_dividend_acquired_at: domain::UtcTimestamp,
    artifact_schema_id: String,
    artifact_schema_version: u32,
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
    bar_count: usize,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct V3Registry {
    schema_id: String,
    schema_version: u32,
    approved_artifacts: Vec<V3Record>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct V3PriceSourceRecord {
    batch_id: BatchId,
    batch_json_sha256: ContentHash,
    manifest_line_sha256: ContentHash,
    bars_sha256: ContentHash,
    file_count: usize,
    capture_contract_commit: String,
    response_marker_evidence: String,
    acquired_at: UtcTimestamp,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct V3ActionSourceRecord {
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

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct V3Record {
    // These four fields preserve the historical five-pin wire shape.  The
    // approval-registry pin is the hash of the exact registry bytes and is
    // therefore supplied to, rather than repeated inside, the record.
    candidate_content_sha256: ContentHash,
    artifact_manifest_sha256: ContentHash,
    stage5_manifest_sha256: ContentHash,
    action_manifest_sha256: ContentHash,
    price_source: V3PriceSourceRecord,
    action_source: V3ActionSourceRecord,
    artifact_schema_id: String,
    artifact_schema_version: u32,
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
    bar_count: usize,
    bars_relative_path: String,
    bars_sha256: ContentHash,
    bars_size_bytes: u64,
    bars_row_count: usize,
}

/// Approves the active V3 artifact selected by the exact registry compiled into
/// the current image. Callers cannot supply a candidate, registry, pin, or path
/// below the artifact root.
pub fn approve_historical_price_only_artifact(
    artifact_root: &Path,
) -> Result<ApprovedHistoricalPriceOnlyArtifact, HistoricalPriceOnlyApprovalError> {
    let registry = parse_v3_registry(EMBEDDED_V3_REGISTRY)?;
    let record = sole_v3_record(&registry)?;
    let registry_hash = ContentHash::from_bytes(EMBEDDED_V3_REGISTRY);
    let pins = HistoricalPriceOnlyArtifactPins {
        candidate_content_sha256: record.candidate_content_sha256.clone(),
        artifact_manifest_sha256: record.artifact_manifest_sha256.clone(),
        stage5_manifest_sha256: record.stage5_manifest_sha256.clone(),
        action_manifest_sha256: record.action_manifest_sha256.clone(),
        approval_registry_sha256: registry_hash,
    };
    approve_v3_with_pins(artifact_root, EMBEDDED_V3_REGISTRY, &pins)
}

/// Replays one of the independently pinned V2 or V3 artifacts.
///
/// The registry hash is an explicit selector.  No other registry or artifact
/// is accepted, and all five pins must match both the selected registry record
/// and the verified artifact summary.
pub fn approve_historical_price_only_artifact_for_pin_values(
    artifact_root: &Path,
    candidate_content_sha256: &ContentHash,
    artifact_manifest_sha256: &ContentHash,
    stage5_manifest_sha256: &ContentHash,
    action_manifest_sha256: &ContentHash,
    approval_registry_sha256: &ContentHash,
) -> Result<ApprovedHistoricalPriceOnlyArtifact, HistoricalPriceOnlyApprovalError> {
    let pins = HistoricalPriceOnlyArtifactPins {
        candidate_content_sha256: candidate_content_sha256.clone(),
        artifact_manifest_sha256: artifact_manifest_sha256.clone(),
        stage5_manifest_sha256: stage5_manifest_sha256.clone(),
        action_manifest_sha256: action_manifest_sha256.clone(),
        approval_registry_sha256: approval_registry_sha256.clone(),
    };
    let v2_registry_hash = ContentHash::from_bytes(EMBEDDED_V2_REGISTRY);
    if approval_registry_sha256 == &v2_registry_hash {
        return approve_v2_with_pins(artifact_root, EMBEDDED_V2_REGISTRY, &pins);
    }
    let v3_registry_hash = ContentHash::from_bytes(EMBEDDED_V3_REGISTRY);
    if approval_registry_sha256 == &v3_registry_hash {
        return approve_v3_with_pins(artifact_root, EMBEDDED_V3_REGISTRY, &pins);
    }
    Err(HistoricalPriceOnlyApprovalError::ArtifactNotApproved)
}

fn approve_v2_with_pins(
    artifact_root: &Path,
    registry_bytes: &[u8],
    requested_pins: &HistoricalPriceOnlyArtifactPins,
) -> Result<ApprovedHistoricalPriceOnlyArtifact, HistoricalPriceOnlyApprovalError> {
    if requested_pins.approval_registry_sha256 != ContentHash::from_bytes(registry_bytes) {
        return Err(HistoricalPriceOnlyApprovalError::ArtifactNotApproved);
    }
    let registry = parse_registry(registry_bytes)?;
    let record = sole_record(&registry)?;
    if !pins_match(record, requested_pins) {
        return Err(HistoricalPriceOnlyApprovalError::ArtifactNotApproved);
    }
    let verified =
        read_historical_price_only_artifact(artifact_root, &record.candidate_content_sha256)
            .map_err(|_| HistoricalPriceOnlyApprovalError::ArtifactRejected)?;
    let summary = verified.approval_summary();
    let pins = HistoricalPriceOnlyArtifactPins {
        candidate_content_sha256: verified.candidate_content_sha256().clone(),
        artifact_manifest_sha256: summary.artifact_manifest_sha256().clone(),
        stage5_manifest_sha256: summary.stage5_manifest_sha256().clone(),
        action_manifest_sha256: summary.action_manifest_sha256().clone(),
        approval_registry_sha256: ContentHash::from_bytes(registry_bytes),
    };
    if !fixed_envelope(summary)
        || !record_matches(record, summary, &pins)
        || pins != *requested_pins
    {
        return Err(HistoricalPriceOnlyApprovalError::ArtifactNotApproved);
    }
    Ok(ApprovedHistoricalPriceOnlyArtifact {
        pins,
        bars: verified.approved_bars().to_vec(),
    })
}

fn approve_v3_with_pins(
    artifact_root: &Path,
    registry_bytes: &[u8],
    requested_pins: &HistoricalPriceOnlyArtifactPins,
) -> Result<ApprovedHistoricalPriceOnlyArtifact, HistoricalPriceOnlyApprovalError> {
    if requested_pins.approval_registry_sha256 != ContentHash::from_bytes(registry_bytes) {
        return Err(HistoricalPriceOnlyApprovalError::ArtifactNotApproved);
    }
    let registry = parse_v3_registry(registry_bytes)?;
    let record = sole_v3_record(&registry)?;
    if !fixed_v3_record(record) || !v3_pins_match(record, requested_pins) {
        return Err(HistoricalPriceOnlyApprovalError::ArtifactNotApproved);
    }
    let verified =
        read_historical_price_only_v3_artifact(artifact_root, &record.candidate_content_sha256)
            .map_err(|_| HistoricalPriceOnlyApprovalError::ArtifactRejected)?;
    let summary = verified.approval_summary();
    let pins = HistoricalPriceOnlyArtifactPins {
        candidate_content_sha256: verified.candidate_content_sha256().clone(),
        artifact_manifest_sha256: summary.artifact_manifest_sha256().clone(),
        stage5_manifest_sha256: summary.price_manifest_line_sha256().clone(),
        action_manifest_sha256: summary.action_manifest_line_sha256().clone(),
        approval_registry_sha256: ContentHash::from_bytes(registry_bytes),
    };
    if !fixed_v3_envelope(summary)
        || !v3_record_matches(record, summary, &pins)
        || pins != *requested_pins
    {
        return Err(HistoricalPriceOnlyApprovalError::ArtifactNotApproved);
    }
    Ok(ApprovedHistoricalPriceOnlyArtifact {
        pins,
        bars: verified.bars().to_vec(),
    })
}

fn sole_record(registry: &Registry) -> Result<&Record, HistoricalPriceOnlyApprovalError> {
    match registry.approved_artifacts.as_slice() {
        [] => Err(HistoricalPriceOnlyApprovalError::ArtifactNotApproved),
        [record] => Ok(record),
        _ => Err(HistoricalPriceOnlyApprovalError::RegistryAmbiguous),
    }
}

fn sole_v3_record(registry: &V3Registry) -> Result<&V3Record, HistoricalPriceOnlyApprovalError> {
    match registry.approved_artifacts.as_slice() {
        [] => Err(HistoricalPriceOnlyApprovalError::ArtifactNotApproved),
        [record] => Ok(record),
        _ => Err(HistoricalPriceOnlyApprovalError::RegistryAmbiguous),
    }
}

fn parse_registry(bytes: &[u8]) -> Result<Registry, HistoricalPriceOnlyApprovalError> {
    if bytes.len() > MAX_REGISTRY_BYTES {
        return Err(HistoricalPriceOnlyApprovalError::RegistryInvalid);
    }
    let registry: Registry = serde_json::from_slice(bytes)
        .map_err(|_| HistoricalPriceOnlyApprovalError::RegistryInvalid)?;
    let mut canonical = serde_json::to_vec(&registry)
        .map_err(|_| HistoricalPriceOnlyApprovalError::RegistryInvalid)?;
    canonical.push(b'\n');
    if bytes != canonical
        || registry.schema_id != REGISTRY_SCHEMA_ID
        || registry.schema_version != REGISTRY_SCHEMA_VERSION
    {
        return Err(HistoricalPriceOnlyApprovalError::RegistryInvalid);
    }
    Ok(registry)
}

fn parse_v3_registry(bytes: &[u8]) -> Result<V3Registry, HistoricalPriceOnlyApprovalError> {
    if bytes.len() > MAX_REGISTRY_BYTES {
        return Err(HistoricalPriceOnlyApprovalError::RegistryInvalid);
    }
    let registry: V3Registry = serde_json::from_slice(bytes)
        .map_err(|_| HistoricalPriceOnlyApprovalError::RegistryInvalid)?;
    let mut canonical = serde_json::to_vec(&registry)
        .map_err(|_| HistoricalPriceOnlyApprovalError::RegistryInvalid)?;
    canonical.push(b'\n');
    if bytes != canonical
        || registry.schema_id != V3_REGISTRY_SCHEMA_ID
        || registry.schema_version != V3_REGISTRY_SCHEMA_VERSION
    {
        return Err(HistoricalPriceOnlyApprovalError::RegistryInvalid);
    }
    Ok(registry)
}

fn fixed_v3_record(record: &V3Record) -> bool {
    let mut instruments = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| format!("{symbol}.KRX"))
        .collect::<Vec<_>>();
    instruments.sort();
    record.candidate_content_sha256.as_str() == V3_CANDIDATE_CONTENT_SHA256
        && record.artifact_manifest_sha256.as_str() == V3_ARTIFACT_MANIFEST_SHA256
        && record.stage5_manifest_sha256.as_str()
            == HISTORICAL_PRICE_ONLY_V3_PRICE_MANIFEST_LINE_SHA256
        && record.action_manifest_sha256.as_str()
            == HISTORICAL_PRICE_ONLY_V3_ACTION_MANIFEST_LINE_SHA256
        && record.price_source.batch_id.to_string() == HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_ID
        && record.price_source.batch_json_sha256.as_str()
            == HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_JSON_SHA256
        && record.price_source.manifest_line_sha256.as_str()
            == HISTORICAL_PRICE_ONLY_V3_PRICE_MANIFEST_LINE_SHA256
        && record.price_source.bars_sha256.as_str() == V3_PRICE_BARS_SHA256
        && record.price_source.file_count == HISTORICAL_PRICE_ONLY_V3_PRICE_FILE_COUNT
        && record.price_source.capture_contract_commit
            == HISTORICAL_PRICE_ONLY_V3_PRICE_CAPTURE_COMMIT
        && record.price_source.response_marker_evidence
            == HISTORICAL_PRICE_ONLY_V3_PRICE_RESPONSE_MARKER_EVIDENCE
        && record.price_source.acquired_at.to_string() == V3_PRICE_ACQUIRED_AT
        && record.action_source.batch_id.to_string() == HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_ID
        && record.action_source.batch_json_sha256.as_str()
            == HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_JSON_SHA256
        && record.action_source.manifest_line_sha256.as_str()
            == HISTORICAL_PRICE_ONLY_V3_ACTION_MANIFEST_LINE_SHA256
        && record.action_source.file_count == HISTORICAL_PRICE_ONLY_V3_ACTION_FILE_COUNT
        && record.action_source.action_count == 0
        && record.action_source.cash_dividend_treatment_id
            == HISTORICAL_PRICE_ONLY_V3_CASH_DIVIDEND_TREATMENT
        && record.action_source.cash_dividend_row_count == 157
        && record.action_source.cash_dividend_rows_sha256.as_str() == V3_CASH_ROWS_SHA256
        && record.action_source.acquired_at.to_string() == V3_ACTION_ACQUIRED_AT
        && record.artifact_schema_id == HISTORICAL_PRICE_ONLY_V3_ARTIFACT_SCHEMA_ID
        && record.artifact_schema_version == HISTORICAL_PRICE_ONLY_V3_ARTIFACT_SCHEMA_VERSION
        && record.contract == HISTORICAL_PRICE_ONLY_V3_CONTRACT
        && record.materializer_version == HISTORICAL_PRICE_ONLY_V3_MATERIALIZER_VERSION
        && record.audience == "OWNER_ONLY"
        && record.vendor_snapshot
        && !record.strict_pit
        && record.capability == "PRICE_RETURN_ONLY"
        && record.materialization_status == "MATERIALIZED"
        && record.registration_status == "UNREGISTERED"
        && record.publication_status == "NOT_PUBLISHED"
        && record.range_start.to_string() == HISTORICAL_PRICE_ONLY_V3_PRICE_RANGE_START
        && record.range_end.to_string() == HISTORICAL_PRICE_ONLY_V3_PRICE_RANGE_END
        && record.instruments == instruments
        && record.instrument_count == KR_ETF_CORE_SYMBOLS.len()
        && record.session_count == HISTORICAL_PRICE_ONLY_V3_PRICE_SESSION_COUNT
        && record.bar_count == HISTORICAL_PRICE_ONLY_V3_PRICE_BAR_COUNT
        && record.bars_relative_path == "bars.ndjson"
        && record.bars_sha256.as_str() == V3_ARTIFACT_BARS_SHA256
        && record.bars_size_bytes == 7_672_914
        && record.bars_row_count == HISTORICAL_PRICE_ONLY_V3_PRICE_BAR_COUNT
}

fn fixed_envelope(summary: &HistoricalPriceOnlyArtifactApprovalSummary) -> bool {
    let mut instruments = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| format!("{symbol}.KRX"))
        .collect::<Vec<_>>();
    instruments.sort();
    summary.schema_id() == "kis-historical-price-only-beta"
        && summary.schema_version() == 2
        && summary.cash_dividend_treatment_id()
            == crate::HISTORICAL_PRICE_ONLY_CASH_DIVIDEND_TREATMENT
        && summary.ignored_cash_dividend_row_count() > 0
        && summary.audience() == "OWNER_ONLY"
        && summary.vendor_snapshot()
        && !summary.strict_pit()
        && summary.capability() == "PRICE_RETURN_ONLY"
        && summary.materialization_status() == "MATERIALIZED"
        && summary.registration_status() == "UNREGISTERED"
        && summary.publication_status() == "NOT_PUBLISHED"
        && summary.range_start().to_string() == "2020-01-31"
        && summary.range_end().to_string() == "2026-08-19"
        && summary.instruments() == instruments
        && summary.instrument_count() == 11
        && summary.session_count() == 1608
        && summary.bar_count() == 17688
}

fn fixed_v3_envelope(summary: &HistoricalPriceOnlyV3ArtifactApprovalSummary) -> bool {
    let mut instruments = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| format!("{symbol}.KRX"))
        .collect::<Vec<_>>();
    instruments.sort();
    summary.candidate_content_sha256().as_str() == V3_CANDIDATE_CONTENT_SHA256
        && summary.artifact_manifest_sha256().as_str() == V3_ARTIFACT_MANIFEST_SHA256
        && summary.price_batch_id().to_string() == HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_ID
        && summary.price_batch_json_sha256().as_str()
            == HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_JSON_SHA256
        && summary.price_manifest_line_sha256().as_str()
            == HISTORICAL_PRICE_ONLY_V3_PRICE_MANIFEST_LINE_SHA256
        && summary.price_bars_sha256().as_str() == V3_PRICE_BARS_SHA256
        && summary.price_file_count() == HISTORICAL_PRICE_ONLY_V3_PRICE_FILE_COUNT
        && summary.price_capture_contract_commit() == HISTORICAL_PRICE_ONLY_V3_PRICE_CAPTURE_COMMIT
        && summary.price_response_marker_evidence()
            == HISTORICAL_PRICE_ONLY_V3_PRICE_RESPONSE_MARKER_EVIDENCE
        && summary.price_acquired_at().to_string() == V3_PRICE_ACQUIRED_AT
        && summary.action_batch_id().to_string() == HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_ID
        && summary.action_batch_json_sha256().as_str()
            == HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_JSON_SHA256
        && summary.action_manifest_line_sha256().as_str()
            == HISTORICAL_PRICE_ONLY_V3_ACTION_MANIFEST_LINE_SHA256
        && summary.action_file_count() == HISTORICAL_PRICE_ONLY_V3_ACTION_FILE_COUNT
        && summary.action_count() == 0
        && summary.cash_dividend_treatment_id() == HISTORICAL_PRICE_ONLY_V3_CASH_DIVIDEND_TREATMENT
        && summary.cash_dividend_row_count() == 157
        && summary.cash_dividend_rows_sha256().as_str()
            == "sha256:b22a5c9808a8a1a2c892aa3ff46d529672c909620a2c45c0e46d48d0538d17e8"
        && summary.action_acquired_at().to_string() == V3_ACTION_ACQUIRED_AT
        && summary.schema_id() == HISTORICAL_PRICE_ONLY_V3_ARTIFACT_SCHEMA_ID
        && summary.schema_id() == HISTORICAL_PRICE_ONLY_V3_SCHEMA_ID
        && summary.schema_version() == HISTORICAL_PRICE_ONLY_V3_ARTIFACT_SCHEMA_VERSION
        && summary.contract() == HISTORICAL_PRICE_ONLY_V3_CONTRACT
        && summary.materializer_version() == HISTORICAL_PRICE_ONLY_V3_MATERIALIZER_VERSION
        && summary.audience() == "OWNER_ONLY"
        && summary.vendor_snapshot()
        && !summary.strict_pit()
        && summary.capability() == "PRICE_RETURN_ONLY"
        && summary.materialization_status() == "MATERIALIZED"
        && summary.registration_status() == "UNREGISTERED"
        && summary.publication_status() == "NOT_PUBLISHED"
        && summary.range_start().to_string() == HISTORICAL_PRICE_ONLY_V3_PRICE_RANGE_START
        && summary.range_end().to_string() == HISTORICAL_PRICE_ONLY_V3_PRICE_RANGE_END
        && summary.instruments() == instruments
        && summary.instrument_count() == KR_ETF_CORE_SYMBOLS.len()
        && summary.session_count() == HISTORICAL_PRICE_ONLY_V3_PRICE_SESSION_COUNT
        && summary.row_count() == HISTORICAL_PRICE_ONLY_V3_PRICE_BAR_COUNT
        && summary.bars_relative_path() == "bars.ndjson"
        && summary.bars_sha256().as_str() == V3_ARTIFACT_BARS_SHA256
        && summary.bars_size_bytes() == 7_672_914
        && summary.bars_row_count() == HISTORICAL_PRICE_ONLY_V3_PRICE_BAR_COUNT
}

fn record_matches(
    record: &Record,
    summary: &HistoricalPriceOnlyArtifactApprovalSummary,
    pins: &HistoricalPriceOnlyArtifactPins,
) -> bool {
    pins_match(record, pins)
        && record.artifact_schema_id == summary.schema_id()
        && record.artifact_schema_version == summary.schema_version()
        && record.cash_dividend_treatment_id == summary.cash_dividend_treatment_id()
        && record.ignored_cash_dividend_row_count == summary.ignored_cash_dividend_row_count()
        && &record.ignored_cash_dividend_rows_sha256 == summary.ignored_cash_dividend_rows_sha256()
        && &record.ignored_cash_dividend_source_file_sha256
            == summary.ignored_cash_dividend_source_file_sha256()
        && record.ignored_cash_dividend_acquired_at == summary.ignored_cash_dividend_acquired_at()
        && record.audience == summary.audience()
        && record.vendor_snapshot == summary.vendor_snapshot()
        && record.strict_pit == summary.strict_pit()
        && record.capability == summary.capability()
        && record.materialization_status == summary.materialization_status()
        && record.registration_status == summary.registration_status()
        && record.publication_status == summary.publication_status()
        && record.range_start == summary.range_start()
        && record.range_end == summary.range_end()
        && record.instruments == summary.instruments()
        && record.instrument_count == summary.instrument_count()
        && record.session_count == summary.session_count()
        && record.bar_count == summary.bar_count()
}

fn v3_record_matches(
    record: &V3Record,
    summary: &HistoricalPriceOnlyV3ArtifactApprovalSummary,
    pins: &HistoricalPriceOnlyArtifactPins,
) -> bool {
    v3_pins_match(record, pins)
        && record.price_source.batch_id == summary.price_batch_id()
        && record.price_source.batch_json_sha256 == *summary.price_batch_json_sha256()
        && record.price_source.manifest_line_sha256 == *summary.price_manifest_line_sha256()
        && record.price_source.bars_sha256 == *summary.price_bars_sha256()
        && record.price_source.file_count == summary.price_file_count()
        && record.price_source.capture_contract_commit == summary.price_capture_contract_commit()
        && record.price_source.response_marker_evidence == summary.price_response_marker_evidence()
        && record.price_source.acquired_at == summary.price_acquired_at()
        && record.action_source.batch_id == summary.action_batch_id()
        && record.action_source.batch_json_sha256 == *summary.action_batch_json_sha256()
        && record.action_source.manifest_line_sha256 == *summary.action_manifest_line_sha256()
        && record.action_source.file_count == summary.action_file_count()
        && record.action_source.action_count == summary.action_count()
        && record.action_source.cash_dividend_treatment_id == summary.cash_dividend_treatment_id()
        && record.action_source.cash_dividend_row_count == summary.cash_dividend_row_count()
        && record.action_source.cash_dividend_rows_sha256 == *summary.cash_dividend_rows_sha256()
        && record.action_source.acquired_at == summary.action_acquired_at()
        && record.artifact_schema_id == summary.schema_id()
        && record.artifact_schema_version == summary.schema_version()
        && record.contract == summary.contract()
        && record.materializer_version == summary.materializer_version()
        && record.audience == summary.audience()
        && record.vendor_snapshot == summary.vendor_snapshot()
        && record.strict_pit == summary.strict_pit()
        && record.capability == summary.capability()
        && record.materialization_status == summary.materialization_status()
        && record.registration_status == summary.registration_status()
        && record.publication_status == summary.publication_status()
        && record.range_start == summary.range_start()
        && record.range_end == summary.range_end()
        && record.instruments == summary.instruments()
        && record.instrument_count == summary.instrument_count()
        && record.session_count == summary.session_count()
        && record.bar_count == summary.row_count()
        && record.bars_relative_path == summary.bars_relative_path()
        && record.bars_sha256 == *summary.bars_sha256()
        && record.bars_size_bytes == summary.bars_size_bytes()
        && record.bars_row_count == summary.bars_row_count()
}

fn pins_match(record: &Record, pins: &HistoricalPriceOnlyArtifactPins) -> bool {
    record.candidate_content_sha256 == pins.candidate_content_sha256
        && record.artifact_manifest_sha256 == pins.artifact_manifest_sha256
        && record.stage5_manifest_sha256 == pins.stage5_manifest_sha256
        && record.action_manifest_sha256 == pins.action_manifest_sha256
}

fn v3_pins_match(record: &V3Record, pins: &HistoricalPriceOnlyArtifactPins) -> bool {
    record.candidate_content_sha256 == pins.candidate_content_sha256
        && record.artifact_manifest_sha256 == pins.artifact_manifest_sha256
        && record.stage5_manifest_sha256 == pins.stage5_manifest_sha256
        && record.action_manifest_sha256 == pins.action_manifest_sha256
        && record.price_source.manifest_line_sha256 == pins.stage5_manifest_sha256
        && record.action_source.manifest_line_sha256 == pins.action_manifest_sha256
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(value: &str) -> ContentHash {
        ContentHash::from_bytes(value.as_bytes())
    }

    fn pinned_hash(value: &str) -> ContentHash {
        ContentHash::parse(value).unwrap()
    }

    fn record() -> Record {
        Record {
            candidate_content_sha256: pinned_hash(
                "sha256:0877d42eab6626de5066c5d38d1c11959b7e2dac005a6c884eff0004c9eab050",
            ),
            artifact_manifest_sha256: pinned_hash(
                "sha256:afd0735dc41e56a5c07403480d66de7baf89fc638d715d0e90507032fb42fc67",
            ),
            stage5_manifest_sha256: pinned_hash(
                "sha256:6f1414852fd50ccf35c7604c63af70fedc83020fc71685d8db5c2a5c431cbdc4",
            ),
            action_manifest_sha256: pinned_hash(
                "sha256:6692f7e5dc215ddce145e63e647344f8264724497ef0d6f6c441b06dedd4f0bd",
            ),
            cash_dividend_treatment_id: crate::HISTORICAL_PRICE_ONLY_CASH_DIVIDEND_TREATMENT.into(),
            ignored_cash_dividend_row_count: 1,
            ignored_cash_dividend_rows_sha256: pinned_hash(
                "sha256:847315aa05b79b520230f82b504e8bf6cf4ecde2bc44e5e6376fd95ce674bc48",
            ),
            ignored_cash_dividend_source_file_sha256: pinned_hash(
                "sha256:906f3429e5ef366763f5161d924c1fd33eafca073e1a639308f0ad66beb270d8",
            ),
            ignored_cash_dividend_acquired_at: domain::UtcTimestamp::parse_rfc3339(
                "2026-08-24T09:11:17Z",
            )
            .unwrap(),
            artifact_schema_id: "kis-historical-price-only-beta".into(),
            artifact_schema_version: 2,
            audience: "OWNER_ONLY".into(),
            vendor_snapshot: true,
            strict_pit: false,
            capability: "PRICE_RETURN_ONLY".into(),
            materialization_status: "MATERIALIZED".into(),
            registration_status: "UNREGISTERED".into(),
            publication_status: "NOT_PUBLISHED".into(),
            range_start: TradingDate::parse("2020-01-31").unwrap(),
            range_end: TradingDate::parse("2026-08-19").unwrap(),
            instruments: vec![
                "069500.KRX".into(),
                "102110.KRX".into(),
                "114260.KRX".into(),
                "132030.KRX".into(),
                "133690.KRX".into(),
                "143850.KRX".into(),
                "148070.KRX".into(),
                "153130.KRX".into(),
                "192090.KRX".into(),
                "195930.KRX".into(),
                "229200.KRX".into(),
            ],
            instrument_count: 11,
            session_count: 1608,
            bar_count: 17688,
        }
    }

    fn pins(record: &Record) -> HistoricalPriceOnlyArtifactPins {
        HistoricalPriceOnlyArtifactPins {
            candidate_content_sha256: record.candidate_content_sha256.clone(),
            artifact_manifest_sha256: record.artifact_manifest_sha256.clone(),
            stage5_manifest_sha256: record.stage5_manifest_sha256.clone(),
            action_manifest_sha256: record.action_manifest_sha256.clone(),
            approval_registry_sha256: hash("registry"),
        }
    }

    fn summary(record: &Record) -> HistoricalPriceOnlyArtifactApprovalSummary {
        HistoricalPriceOnlyArtifactApprovalSummary {
            artifact_manifest_sha256: record.artifact_manifest_sha256.clone(),
            stage5_manifest_sha256: record.stage5_manifest_sha256.clone(),
            action_manifest_sha256: record.action_manifest_sha256.clone(),
            cash_dividend_treatment_id: record.cash_dividend_treatment_id.clone(),
            ignored_cash_dividend_row_count: record.ignored_cash_dividend_row_count,
            ignored_cash_dividend_rows_sha256: record.ignored_cash_dividend_rows_sha256.clone(),
            ignored_cash_dividend_source_file_sha256: record
                .ignored_cash_dividend_source_file_sha256
                .clone(),
            ignored_cash_dividend_acquired_at: record.ignored_cash_dividend_acquired_at,
            schema_id: record.artifact_schema_id.clone(),
            schema_version: record.artifact_schema_version,
            audience: record.audience.clone(),
            vendor_snapshot: record.vendor_snapshot,
            strict_pit: record.strict_pit,
            capability: record.capability.clone(),
            materialization_status: record.materialization_status.clone(),
            registration_status: record.registration_status.clone(),
            publication_status: record.publication_status.clone(),
            range_start: record.range_start,
            range_end: record.range_end,
            instruments: record.instruments.clone(),
            instrument_count: record.instrument_count,
            session_count: record.session_count,
            bar_count: record.bar_count,
        }
    }
    fn v3_record() -> V3Record {
        let registry = parse_v3_registry(EMBEDDED_V3_REGISTRY).unwrap();
        sole_v3_record(&registry).unwrap().clone()
    }

    fn v3_pins(record: &V3Record) -> HistoricalPriceOnlyArtifactPins {
        HistoricalPriceOnlyArtifactPins {
            candidate_content_sha256: record.candidate_content_sha256.clone(),
            artifact_manifest_sha256: record.artifact_manifest_sha256.clone(),
            stage5_manifest_sha256: record.stage5_manifest_sha256.clone(),
            action_manifest_sha256: record.action_manifest_sha256.clone(),
            approval_registry_sha256: ContentHash::from_bytes(EMBEDDED_V3_REGISTRY),
        }
    }

    #[test]
    fn embedded_v3_registry_is_canonical_and_binds_every_v3_fact() {
        let registry = parse_v3_registry(EMBEDDED_V3_REGISTRY).unwrap();
        assert_eq!(registry.schema_id, V3_REGISTRY_SCHEMA_ID);
        assert_eq!(registry.schema_version, V3_REGISTRY_SCHEMA_VERSION);
        assert_eq!(
            ContentHash::from_bytes(EMBEDDED_V3_REGISTRY).as_str(),
            "sha256:5d3aa2b354d8c0c51d0d7d029e9fd3f92e0570fe074262bfc900829a5a2bb707"
        );
        assert_ne!(
            ContentHash::from_bytes(EMBEDDED_V2_REGISTRY),
            ContentHash::from_bytes(EMBEDDED_V3_REGISTRY)
        );
        assert_eq!(registry.approved_artifacts.len(), 1);
        let record = sole_v3_record(&registry).unwrap();
        assert_eq!(
            record.candidate_content_sha256.as_str(),
            V3_CANDIDATE_CONTENT_SHA256
        );
        assert_eq!(
            record.artifact_manifest_sha256.as_str(),
            V3_ARTIFACT_MANIFEST_SHA256
        );
        assert_eq!(
            record.stage5_manifest_sha256,
            record.price_source.manifest_line_sha256
        );
        assert_eq!(
            record.action_manifest_sha256,
            record.action_source.manifest_line_sha256
        );
        assert_eq!(record.price_source.file_count, 275);
        assert_eq!(record.action_source.file_count, 77);
        assert_eq!(record.action_source.action_count, 0);
        assert_eq!(record.action_source.cash_dividend_row_count, 157);
        assert_eq!(record.bars_relative_path, "bars.ndjson");
        assert_eq!(record.bars_size_bytes, 7_672_914);

        let pins = v3_pins(record);
        assert!(fixed_v3_record(record));
        assert!(v3_pins_match(record, &pins));

        let mut canonical = serde_json::to_vec(&registry).unwrap();
        canonical.push(b'\n');
        assert_eq!(EMBEDDED_V3_REGISTRY, canonical.as_slice());
        let changed = [EMBEDDED_V3_REGISTRY, b" "].concat();
        assert!(matches!(
            parse_v3_registry(&changed),
            Err(HistoricalPriceOnlyApprovalError::RegistryInvalid)
        ));
        let mut unknown: serde_json::Value = serde_json::from_slice(EMBEDDED_V3_REGISTRY).unwrap();
        unknown["unexpected"] = serde_json::Value::Bool(true);
        let mut unknown_bytes = serde_json::to_vec(&unknown).unwrap();
        unknown_bytes.push(b'\n');
        assert!(matches!(
            parse_v3_registry(&unknown_bytes),
            Err(HistoricalPriceOnlyApprovalError::RegistryInvalid)
        ));
    }

    #[test]
    fn v3_registry_and_all_five_pins_are_required() {
        let record = v3_record();
        let pins = v3_pins(&record);
        assert!(v3_pins_match(&record, &pins));
        for index in 0..4 {
            let mut changed = record.clone();
            match index {
                0 => changed.candidate_content_sha256 = hash("other-v3-candidate"),
                1 => changed.artifact_manifest_sha256 = hash("other-v3-artifact"),
                2 => changed.stage5_manifest_sha256 = hash("other-v3-stage5"),
                _ => changed.action_manifest_sha256 = hash("other-v3-action"),
            }
            assert!(!v3_pins_match(&changed, &pins));
        }
        let unknown_registry = hash("other-v3-registry");
        assert!(matches!(
            approve_historical_price_only_artifact_for_pin_values(
                Path::new("/approval-v3-nonexistent-root-sentinel"),
                pins.candidate_content_sha256(),
                pins.artifact_manifest_sha256(),
                pins.stage5_manifest_sha256(),
                pins.action_manifest_sha256(),
                &unknown_registry,
            ),
            Err(HistoricalPriceOnlyApprovalError::ArtifactNotApproved)
        ));
    }

    #[test]
    fn v3_record_source_and_envelope_mutations_fail_closed() {
        let mutations = [
            "candidate",
            "artifact",
            "stage5",
            "action",
            "price_batch_json",
            "price_manifest",
            "price_bars",
            "price_file_count",
            "price_capture_commit",
            "price_marker",
            "price_acquired_at",
            "action_batch_json",
            "action_file_count",
            "action_count",
            "cash_treatment",
            "cash_count",
            "cash_hash",
            "action_acquired_at",
            "artifact_schema",
            "artifact_version",
            "contract",
            "materializer",
            "audience",
            "vendor_snapshot",
            "strict_pit",
            "capability",
            "materialization",
            "registration",
            "publication",
            "range_start",
            "range_end",
            "instruments",
            "instrument_count",
            "session_count",
            "bar_count",
            "bars_path",
            "bars_hash",
            "bars_size",
            "bars_row_count",
        ];
        for mutation in mutations {
            let mut changed: serde_json::Value =
                serde_json::from_slice(EMBEDDED_V3_REGISTRY).unwrap();
            let record = &mut changed["approved_artifacts"][0];
            match mutation {
                "candidate" => {
                    record["candidate_content_sha256"] =
                        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                            .into()
                }
                "artifact" => {
                    record["artifact_manifest_sha256"] =
                        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                            .into()
                }
                "stage5" => {
                    record["stage5_manifest_sha256"] =
                        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                            .into()
                }
                "action" => {
                    record["action_manifest_sha256"] =
                        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                            .into()
                }
                "price_batch_json" => {
                    record["price_source"]["batch_json_sha256"] =
                        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                            .into()
                }
                "price_manifest" => {
                    record["price_source"]["manifest_line_sha256"] =
                        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                            .into()
                }
                "price_bars" => {
                    record["price_source"]["bars_sha256"] =
                        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                            .into()
                }
                "price_file_count" => record["price_source"]["file_count"] = 274.into(),
                "price_capture_commit" => {
                    record["price_source"]["capture_contract_commit"] =
                        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()
                }
                "price_marker" => {
                    record["price_source"]["response_marker_evidence"] = "OTHER".into()
                }
                "price_acquired_at" => {
                    record["price_source"]["acquired_at"] = "2026-08-28T18:20:15Z".into()
                }
                "action_batch_json" => {
                    record["action_source"]["batch_json_sha256"] =
                        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                            .into()
                }
                "action_file_count" => record["action_source"]["file_count"] = 76.into(),
                "action_count" => record["action_source"]["action_count"] = 1.into(),
                "cash_treatment" => {
                    record["action_source"]["cash_dividend_treatment_id"] = "OTHER".into()
                }
                "cash_count" => record["action_source"]["cash_dividend_row_count"] = 156.into(),
                "cash_hash" => {
                    record["action_source"]["cash_dividend_rows_sha256"] =
                        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                            .into()
                }
                "action_acquired_at" => {
                    record["action_source"]["acquired_at"] = "2026-08-28T18:54:50Z".into()
                }
                "artifact_schema" => record["artifact_schema_id"] = "OTHER".into(),
                "artifact_version" => record["artifact_schema_version"] = 4.into(),
                "contract" => record["contract"] = "OTHER".into(),
                "materializer" => record["materializer_version"] = "OTHER".into(),
                "audience" => record["audience"] = "OTHER".into(),
                "vendor_snapshot" => record["vendor_snapshot"] = false.into(),
                "strict_pit" => record["strict_pit"] = true.into(),
                "capability" => record["capability"] = "OTHER".into(),
                "materialization" => record["materialization_status"] = "OTHER".into(),
                "registration" => record["registration_status"] = "REGISTERED".into(),
                "publication" => record["publication_status"] = "PUBLISHED".into(),
                "range_start" => record["range_start"] = "2016-08-30".into(),
                "range_end" => record["range_end"] = "2026-08-27".into(),
                "instruments" => record["instruments"][0] = "OTHER.KRX".into(),
                "instrument_count" => record["instrument_count"] = 10.into(),
                "session_count" => record["session_count"] = 2451.into(),
                "bar_count" => record["bar_count"] = 26971.into(),
                "bars_path" => record["bars_relative_path"] = "other.ndjson".into(),
                "bars_hash" => {
                    record["bars_sha256"] =
                        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                            .into()
                }
                "bars_size" => record["bars_size_bytes"] = 7672913.into(),
                "bars_row_count" => record["bars_row_count"] = 26971.into(),
                _ => unreachable!("covered mutation: {mutation}"),
            }
            let changed: V3Registry = serde_json::from_value(changed).unwrap();
            assert!(
                !fixed_v3_record(sole_v3_record(&changed).unwrap()),
                "mutation should be rejected: {mutation}"
            );
        }
    }

    #[test]
    fn pin_targeted_resolver_selects_v2_or_v3_and_rejects_unknown_registry() {
        let v2 = parse_registry(EMBEDDED_V2_REGISTRY).unwrap();
        let v2 = sole_record(&v2).unwrap();
        let v2_registry_hash = ContentHash::from_bytes(EMBEDDED_V2_REGISTRY);
        assert_eq!(
            v2_registry_hash.as_str(),
            "sha256:4111f51d945a48a7559b22863cc4ed2eae9c760d5ac9288e554aefe5575e3380"
        );
        let v2_result = approve_historical_price_only_artifact_for_pin_values(
            Path::new("/approval-v2-nonexistent-root-sentinel"),
            &v2.candidate_content_sha256,
            &v2.artifact_manifest_sha256,
            &v2.stage5_manifest_sha256,
            &v2.action_manifest_sha256,
            &v2_registry_hash,
        );
        assert!(matches!(
            v2_result,
            Err(HistoricalPriceOnlyApprovalError::ArtifactRejected)
        ));

        let v3 = v3_record();
        let v3 = v3_pins(&v3);
        let v3_result = approve_historical_price_only_artifact_for_pin_values(
            Path::new("/approval-v3-nonexistent-root-sentinel"),
            v3.candidate_content_sha256(),
            v3.artifact_manifest_sha256(),
            v3.stage5_manifest_sha256(),
            v3.action_manifest_sha256(),
            v3.approval_registry_sha256(),
        );
        assert!(matches!(
            v3_result,
            Err(HistoricalPriceOnlyApprovalError::ArtifactRejected)
        ));

        let unknown = hash("unknown-registry");
        let unknown_result = approve_historical_price_only_artifact_for_pin_values(
            Path::new("/approval-unknown-nonexistent-root-sentinel"),
            v3.candidate_content_sha256(),
            v3.artifact_manifest_sha256(),
            v3.stage5_manifest_sha256(),
            v3.action_manifest_sha256(),
            &unknown,
        );
        assert!(matches!(
            unknown_result,
            Err(HistoricalPriceOnlyApprovalError::ArtifactNotApproved)
        ));
    }

    #[test]
    fn embedded_registry_is_canonical_and_binds_exact_sole_record() {
        let registry = parse_registry(EMBEDDED_REGISTRY).unwrap();
        assert_eq!(registry.schema_id, REGISTRY_SCHEMA_ID);
        assert_eq!(registry.schema_version, REGISTRY_SCHEMA_VERSION);
        assert_eq!(registry.approved_artifacts.len(), 1);
        let record = sole_record(&registry).unwrap();
        assert_eq!(
            record.candidate_content_sha256.as_str(),
            "sha256:0877d42eab6626de5066c5d38d1c11959b7e2dac005a6c884eff0004c9eab050"
        );
        assert_eq!(
            record.artifact_manifest_sha256.as_str(),
            "sha256:afd0735dc41e56a5c07403480d66de7baf89fc638d715d0e90507032fb42fc67"
        );
        assert_eq!(
            record.stage5_manifest_sha256.as_str(),
            "sha256:6f1414852fd50ccf35c7604c63af70fedc83020fc71685d8db5c2a5c431cbdc4"
        );
        assert_eq!(
            record.action_manifest_sha256.as_str(),
            "sha256:6692f7e5dc215ddce145e63e647344f8264724497ef0d6f6c441b06dedd4f0bd"
        );
        assert_eq!(
            record.cash_dividend_treatment_id,
            "CASH_ONLY_EXCLUDED_FROM_PRICE_RETURN_ONLY_V1"
        );
        assert_eq!(record.ignored_cash_dividend_row_count, 1);
        assert_eq!(
            record.ignored_cash_dividend_rows_sha256.as_str(),
            "sha256:847315aa05b79b520230f82b504e8bf6cf4ecde2bc44e5e6376fd95ce674bc48"
        );
        assert_eq!(
            record.ignored_cash_dividend_source_file_sha256.as_str(),
            "sha256:906f3429e5ef366763f5161d924c1fd33eafca073e1a639308f0ad66beb270d8"
        );
        assert_eq!(
            record.ignored_cash_dividend_acquired_at,
            domain::UtcTimestamp::parse_rfc3339("2026-08-24T09:11:17Z").unwrap()
        );

        let summary = summary(record);
        let pins = pins(record);
        assert!(fixed_envelope(&summary));
        assert!(record_matches(record, &summary, &pins));

        let mut canonical = serde_json::to_vec(&registry).unwrap();
        canonical.push(b'\n');
        assert_eq!(EMBEDDED_REGISTRY, canonical.as_slice());
        let changed = [EMBEDDED_REGISTRY, b" "].concat();
        assert!(matches!(
            parse_registry(&changed),
            Err(HistoricalPriceOnlyApprovalError::RegistryInvalid)
        ));
        assert_ne!(
            ContentHash::from_bytes(EMBEDDED_REGISTRY),
            ContentHash::from_bytes(&changed)
        );
    }

    #[test]
    fn every_artifact_pin_is_required() {
        let record = record();
        let pins = pins(&record);
        assert!(pins_match(&record, &pins));
        for index in 0..4 {
            let mut changed = record.clone();
            match index {
                0 => changed.candidate_content_sha256 = hash("other-candidate"),
                1 => changed.artifact_manifest_sha256 = hash("other-artifact"),
                2 => changed.stage5_manifest_sha256 = hash("other-stage5"),
                _ => changed.action_manifest_sha256 = hash("other-action"),
            }
            assert!(!pins_match(&changed, &pins));
        }
    }

    #[test]
    fn every_cash_dividend_treatment_field_is_registry_bound() {
        let record = record();
        let pins = pins(&record);
        let summary = summary(&record);
        assert!(record_matches(&record, &summary, &pins));
        for index in 0..5 {
            let mut changed = record.clone();
            match index {
                0 => changed.cash_dividend_treatment_id = "OTHER".into(),
                1 => changed.ignored_cash_dividend_row_count += 1,
                2 => changed.ignored_cash_dividend_rows_sha256 = hash("other-rows"),
                3 => changed.ignored_cash_dividend_source_file_sha256 = hash("other-source"),
                _ => {
                    changed.ignored_cash_dividend_acquired_at =
                        domain::UtcTimestamp::parse_rfc3339("2026-08-20T00:00:00Z").unwrap()
                }
            }
            assert!(!record_matches(&changed, &summary, &pins));
        }
    }

    #[test]
    fn zero_and_multiple_records_are_not_approvable() {
        let empty = Registry {
            schema_id: REGISTRY_SCHEMA_ID.into(),
            schema_version: REGISTRY_SCHEMA_VERSION,
            approved_artifacts: Vec::new(),
        };
        let multiple = Registry {
            schema_id: REGISTRY_SCHEMA_ID.into(),
            schema_version: REGISTRY_SCHEMA_VERSION,
            approved_artifacts: vec![record(), record()],
        };
        assert!(matches!(
            sole_record(&empty),
            Err(HistoricalPriceOnlyApprovalError::ArtifactNotApproved)
        ));
        assert!(matches!(
            sole_record(&multiple),
            Err(HistoricalPriceOnlyApprovalError::RegistryAmbiguous)
        ));
    }

    #[test]
    fn approval_debug_and_errors_expose_no_path_or_request_metadata() {
        let record = record();
        let approved = ApprovedHistoricalPriceOnlyArtifact {
            pins: pins(&record),
            bars: Vec::new(),
        };
        let debug = format!("{approved:?}");
        assert!(!debug.contains("/operator-root-sentinel"));
        assert!(!debug.contains("request-metadata-sentinel"));
        let error = HistoricalPriceOnlyApprovalError::ArtifactRejected;
        assert!(!format!("{error:?} {error}").contains("/operator-root-sentinel"));
    }
}
