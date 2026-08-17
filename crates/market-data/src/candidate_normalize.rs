//! KIS candidate wire-to-canonical normalization.
//!
//! Candidate wire responses live under the dedicated kis-candidate scope and
//! are never handed directly to the candidate publisher. This module maps the
//! documented investor-flow fields into provider-neutral candidate documents,
//! with complete upstream file/hash lineage. Finance responses are retained as
//! immutable Raw evidence but remain blocked from canonical output until their
//! units, statement scope, and disclosure timing have a reviewed mapping.

use std::collections::{BTreeMap, BTreeSet};

use domain::{BatchId, ContentHash, TradingDate, UtcTimestamp};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};
use uuid::Uuid;

use crate::contract::{
    FetchMode, MARKET_KR, PROVIDER_KIS_CANDIDATE, PROVIDER_KIS_CANDIDATE_NORMALIZED, RawEnvelope,
    RequestMetadata, ResponseKind, StoredFile,
};
use crate::providers::kis_candidate::{INVESTOR_FLOW_PATH, KIS_CANDIDATE_SUPPORTED_KINDS};
use crate::storage::{BatchSpec, FileEntry, ManifestEntry, RawStore, StoreError};
use crate::validate::validate_response;

const NORMALIZER: &str = "kis-candidate-wire-to-canonical-v1";
const NORMALIZER_SCHEMA_VERSION: u32 = 1;
const FLOW_REVISION: &str = "kis-investor-trade-by-stock-daily-v1";

// KIS reports the investor-flow trade amount in million KRW while the
// provider-neutral observation contract stores KRW. Quantities are the
// documented net share counts and remain unscaled with `volume_unit=SHARE`.
const FLOW_AMOUNT_SCALE: u64 = 1_000_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateNormalizationSourceFile {
    pub kind: ResponseKind,
    pub file_name: String,
    pub content_hash: ContentHash,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateNormalizationLineage {
    pub schema_version: u32,
    pub normalizer: String,
    pub upstream_provider: String,
    pub upstream_market: String,
    pub upstream_batch_id: BatchId,
    pub upstream_files: Vec<CandidateNormalizationSourceFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateNormalizationOutcome {
    pub source_batch_id: BatchId,
    pub entry: ManifestEntry,
    pub files: Vec<StoredFile>,
    pub lineage: CandidateNormalizationLineage,
}

#[derive(Debug, thiserror::Error)]
pub enum CandidateNormalizeError {
    #[error(
        "candidate normalization supports only {expected_provider}/{expected_market}, got {provider}/{market}"
    )]
    UnsupportedScope {
        expected_provider: &'static str,
        expected_market: &'static str,
        provider: String,
        market: String,
    },
    #[error("candidate source batch is not credentialed")]
    UnsupportedMode,
    #[error("candidate source contains unsupported response kind {0}")]
    UnsupportedKind(ResponseKind),
    #[error("existing deterministic candidate batch {batch_id} conflicts: {reason}")]
    ExistingBatchConflict { batch_id: BatchId, reason: String },
    #[error("candidate evidence count differs from manifest: expected {expected}, got {actual}")]
    EvidenceCountMismatch { expected: usize, actual: usize },
    #[error("candidate evidence is missing manifest file {file_name}")]
    EvidenceMissing { file_name: String },
    #[error("candidate evidence contains unmanifested file {file_name}")]
    EvidenceUnexpected { file_name: String },
    #[error("candidate evidence hash mismatch for {file_name}: expected {expected}, got {actual}")]
    EvidenceHashMismatch {
        file_name: String,
        expected: String,
        actual: String,
    },
    #[error("candidate {kind} file {file_name} has unexpected endpoint {endpoint}")]
    UnexpectedEndpoint {
        kind: ResponseKind,
        file_name: String,
        endpoint: String,
    },
    #[error("malformed candidate {kind} file {file_name}: {reason}")]
    Malformed {
        kind: ResponseKind,
        file_name: String,
        reason: String,
    },
    #[error("candidate {kind} file {file_name} is missing field {field}")]
    MissingField {
        kind: ResponseKind,
        file_name: String,
        field: String,
    },
    #[error("invalid candidate {kind} field {field} in {file_name}: {value}")]
    InvalidField {
        kind: ResponseKind,
        file_name: String,
        field: String,
        value: String,
    },
    #[error("duplicate candidate {kind} row {key}")]
    DuplicateRow { kind: ResponseKind, key: String },
    #[error("conflicting candidate {kind} row {key}")]
    ConflictingRow { kind: ResponseKind, key: String },
    #[error("candidate canonical {kind} failed neutral validation: {reason}")]
    CanonicalValidation { kind: ResponseKind, reason: String },
    #[error("candidate canonical {kind} serialization failed: {reason}")]
    Serialization { kind: ResponseKind, reason: String },
    #[error("candidate finance normalization is permanently blocked for {file_name}: {reason}")]
    UnverifiedFinanceSemantics { file_name: String, reason: String },
    #[error("candidate source Raw read failed: {0}")]
    Store(#[from] StoreError),
}

pub fn deterministic_kis_candidate_normalized_batch_id(source_batch_id: BatchId) -> BatchId {
    let name = format!(
        "provider={}\nnormalizer={}\nsource_batch={}",
        PROVIDER_KIS_CANDIDATE_NORMALIZED, NORMALIZER, source_batch_id
    );
    BatchId::from_uuid(Uuid::new_v5(&Uuid::NAMESPACE_URL, name.as_bytes()))
}

pub fn normalize_kis_candidate_batch(
    raw: &RawStore,
    source: &ManifestEntry,
) -> Result<CandidateNormalizationOutcome, CandidateNormalizeError> {
    validate_source_scope(source)?;
    let stored = raw.read_batch_bytes(&source.provider, &source.market, source)?;
    let batch_id = deterministic_kis_candidate_normalized_batch_id(source.batch_id);
    let envelopes = normalize_kis_candidate_envelopes_with_batch_id(source, &stored, batch_id)?;
    let lineage = lineage_for(source);
    let spec = BatchSpec {
        provider: PROVIDER_KIS_CANDIDATE_NORMALIZED,
        market: &source.market,
        date: &source.date,
        batch_id,
        entitlement_reference: source.entitlement_reference.as_deref(),
        mode: FetchMode::Credentialed,
    };
    let expected = expected_manifest_entry(&spec, source.retrieved_at, &envelopes);
    if let Some(existing) = load_existing(raw, source, &expected, &envelopes, lineage.clone())? {
        return Ok(existing);
    }
    match raw.store_batch(&spec, &envelopes) {
        Ok(entry) => {
            if entry != expected {
                return Err(existing_conflict(
                    batch_id,
                    "RawStore returned metadata different from deterministic contract",
                ));
            }
            let files =
                raw.read_batch_bytes(PROVIDER_KIS_CANDIDATE_NORMALIZED, MARKET_KR, &entry)?;
            validate_stored_evidence(&entry, &files)?;
            Ok(CandidateNormalizationOutcome {
                source_batch_id: source.batch_id,
                entry,
                files,
                lineage,
            })
        }
        Err(error @ StoreError::FileExists { .. }) => {
            for _ in 0..100 {
                if let Some(existing) =
                    load_existing(raw, source, &expected, &envelopes, lineage.clone())?
                {
                    return Ok(existing);
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Err(CandidateNormalizeError::Store(error))
        }
        Err(error) => Err(CandidateNormalizeError::Store(error)),
    }
}

pub fn normalize_kis_candidate_envelopes(
    source: &ManifestEntry,
    stored: &[StoredFile],
) -> Result<Vec<RawEnvelope>, CandidateNormalizeError> {
    validate_source_scope(source)?;
    normalize_kis_candidate_envelopes_with_batch_id(source, stored, BatchId::generate())
}

fn normalize_kis_candidate_envelopes_with_batch_id(
    source: &ManifestEntry,
    stored: &[StoredFile],
    batch_id: BatchId,
) -> Result<Vec<RawEnvelope>, CandidateNormalizeError> {
    validate_source_scope(source)?;
    validate_stored_evidence(source, stored)?;
    if let Some(finance) = source
        .files
        .iter()
        .find(|file| file.kind == ResponseKind::Fundamentals)
    {
        return Err(CandidateNormalizeError::UnverifiedFinanceSemantics {
            file_name: finance.file_name.clone(),
            reason: "KIS finance amount units, StatementScope, and disclosure timestamp are not proven; 99.99-style sentinel values must not become canonical metrics".to_owned(),
        });
    }
    let lineage = lineage_for(source);
    let mut output = Vec::new();
    if source
        .files
        .iter()
        .any(|file| file.kind == ResponseKind::InvestorFlow)
    {
        output.push(normalize_flows(source, stored, &lineage, batch_id)?);
    }
    if output.is_empty() {
        return Err(CandidateNormalizeError::Malformed {
            kind: ResponseKind::InvestorFlow,
            file_name: "candidate-batch".to_owned(),
            reason: "candidate source batch contains no supported response class".to_owned(),
        });
    }
    Ok(output)
}

fn validate_source_scope(source: &ManifestEntry) -> Result<(), CandidateNormalizeError> {
    if source.provider != PROVIDER_KIS_CANDIDATE || source.market != MARKET_KR {
        return Err(CandidateNormalizeError::UnsupportedScope {
            expected_provider: PROVIDER_KIS_CANDIDATE,
            expected_market: MARKET_KR,
            provider: source.provider.clone(),
            market: source.market.clone(),
        });
    }
    if source.mode != FetchMode::Credentialed {
        return Err(CandidateNormalizeError::UnsupportedMode);
    }
    for file in &source.files {
        if !KIS_CANDIDATE_SUPPORTED_KINDS.contains(&file.kind) {
            return Err(CandidateNormalizeError::UnsupportedKind(file.kind));
        }
    }
    Ok(())
}

fn lineage_for(source: &ManifestEntry) -> CandidateNormalizationLineage {
    CandidateNormalizationLineage {
        schema_version: NORMALIZER_SCHEMA_VERSION,
        normalizer: NORMALIZER.to_owned(),
        upstream_provider: source.provider.clone(),
        upstream_market: source.market.clone(),
        upstream_batch_id: source.batch_id,
        upstream_files: source_lineage(source),
    }
}

fn source_lineage(source: &ManifestEntry) -> Vec<CandidateNormalizationSourceFile> {
    let mut files = source
        .files
        .iter()
        .map(|file| CandidateNormalizationSourceFile {
            kind: file.kind,
            file_name: file.file_name.clone(),
            content_hash: file.content_hash.clone(),
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| (left.kind, &left.file_name).cmp(&(right.kind, &right.file_name)));
    files
}

fn expected_manifest_entry(
    spec: &BatchSpec<'_>,
    retrieved_at: UtcTimestamp,
    envelopes: &[RawEnvelope],
) -> ManifestEntry {
    ManifestEntry {
        batch_id: spec.batch_id,
        provider: spec.provider.to_owned(),
        market: spec.market.to_owned(),
        date: *spec.date,
        retrieved_at,
        mode: spec.mode,
        entitlement_reference: spec.entitlement_reference.map(str::to_owned),
        files: envelopes
            .iter()
            .map(|envelope| FileEntry {
                kind: envelope.kind,
                file_name: envelope.file_name.clone(),
                content_hash: envelope.content_hash.clone(),
                size_bytes: envelope.bytes.len() as u64,
                request: envelope.request.clone(),
            })
            .collect(),
    }
}

fn existing_conflict(batch_id: BatchId, reason: impl Into<String>) -> CandidateNormalizeError {
    CandidateNormalizeError::ExistingBatchConflict {
        batch_id,
        reason: reason.into(),
    }
}

fn load_existing(
    raw: &RawStore,
    source: &ManifestEntry,
    expected: &ManifestEntry,
    expected_envelopes: &[RawEnvelope],
    lineage: CandidateNormalizationLineage,
) -> Result<Option<CandidateNormalizationOutcome>, CandidateNormalizeError> {
    let existing = raw
        .read_reconciled_manifest(PROVIDER_KIS_CANDIDATE_NORMALIZED, MARKET_KR)?
        .into_iter()
        .find(|entry| entry.batch_id == expected.batch_id);
    let Some(entry) = existing else {
        return Ok(None);
    };
    if &entry != expected {
        return Err(existing_conflict(
            entry.batch_id,
            "manifest metadata, canonical shape, or hash differs",
        ));
    }
    let files = raw.read_batch_bytes(PROVIDER_KIS_CANDIDATE_NORMALIZED, MARKET_KR, &entry)?;
    validate_stored_evidence(&entry, &files)?;
    for expected_file in expected_envelopes {
        let actual = files
            .iter()
            .find(|file| file.file_name == expected_file.file_name)
            .ok_or_else(|| existing_conflict(entry.batch_id, "canonical file is missing"))?;
        if actual.bytes != expected_file.bytes {
            return Err(existing_conflict(
                entry.batch_id,
                format!("canonical file {} bytes differ", expected_file.file_name),
            ));
        }
    }
    Ok(Some(CandidateNormalizationOutcome {
        source_batch_id: source.batch_id,
        entry,
        files,
        lineage,
    }))
}

fn validate_stored_evidence(
    source: &ManifestEntry,
    stored: &[StoredFile],
) -> Result<(), CandidateNormalizeError> {
    let expected = source
        .files
        .iter()
        .map(|file| file.file_name.as_str())
        .collect::<BTreeSet<_>>();
    let actual = stored
        .iter()
        .map(|file| file.file_name.as_str())
        .collect::<BTreeSet<_>>();
    if let Some(file_name) = expected.difference(&actual).next() {
        return Err(CandidateNormalizeError::EvidenceMissing {
            file_name: (*file_name).to_owned(),
        });
    }
    if let Some(file_name) = actual.difference(&expected).next() {
        return Err(CandidateNormalizeError::EvidenceUnexpected {
            file_name: (*file_name).to_owned(),
        });
    }
    if expected.len() != stored.len() || expected.len() != source.files.len() {
        return Err(CandidateNormalizeError::EvidenceCountMismatch {
            expected: source.files.len(),
            actual: stored.len(),
        });
    }
    for metadata in &source.files {
        let file = stored
            .iter()
            .find(|file| file.file_name == metadata.file_name)
            .ok_or_else(|| CandidateNormalizeError::EvidenceMissing {
                file_name: metadata.file_name.clone(),
            })?;
        let actual_hash = ContentHash::from_bytes(&file.bytes);
        if actual_hash != metadata.content_hash {
            return Err(CandidateNormalizeError::EvidenceHashMismatch {
                file_name: metadata.file_name.clone(),
                expected: metadata.content_hash.to_string(),
                actual: actual_hash.to_string(),
            });
        }
    }
    Ok(())
}

fn source_files<'a>(
    source: &'a ManifestEntry,
    stored: &'a [StoredFile],
    kind: ResponseKind,
) -> Result<Vec<(&'a FileEntry, &'a StoredFile)>, CandidateNormalizeError> {
    let mut selected = source
        .files
        .iter()
        .filter(|metadata| metadata.kind == kind)
        .map(|metadata| {
            let file = stored
                .iter()
                .find(|file| file.file_name == metadata.file_name)
                .ok_or_else(|| CandidateNormalizeError::EvidenceMissing {
                    file_name: metadata.file_name.clone(),
                })?;
            Ok((metadata, file))
        })
        .collect::<Result<Vec<_>, CandidateNormalizeError>>()?;
    selected.sort_by(|left, right| left.0.file_name.cmp(&right.0.file_name));
    if selected.is_empty() {
        return Err(CandidateNormalizeError::MissingField {
            kind,
            file_name: "candidate-batch".to_owned(),
            field: kind.as_str().to_owned(),
        });
    }
    Ok(selected)
}

fn canonical_envelope(
    kind: ResponseKind,
    file_name: &str,
    mut document: Map<String, Value>,
    source: &ManifestEntry,
    lineage: &CandidateNormalizationLineage,
    batch_id: BatchId,
) -> Result<RawEnvelope, CandidateNormalizeError> {
    // Candidate structs use deny_unknown_fields, while the immutable
    // publication contract intentionally carries a lineage field at the
    // document root. Validate the typed payload before adding provenance.
    let collection = match kind {
        ResponseKind::InvestorFlow => "flows",
        ResponseKind::MarketStatus => "statuses",
        ResponseKind::IndexMembership => "memberships",
        ResponseKind::SectorClassification => "sectors",
        other => {
            return Err(CandidateNormalizeError::CanonicalValidation {
                kind,
                reason: format!("unsupported candidate kind {other}"),
            });
        }
    };
    let mut typed_document = Map::new();
    typed_document.insert(
        collection.to_owned(),
        document.get(collection).cloned().unwrap_or(Value::Null),
    );
    let typed_bytes = serde_json::to_vec(&Value::Object(typed_document)).map_err(|error| {
        CandidateNormalizeError::Serialization {
            kind,
            reason: error.to_string(),
        }
    })?;
    let typed_envelope = RawEnvelope::new(
        batch_id,
        kind,
        file_name,
        typed_bytes,
        source.retrieved_at,
        RequestMetadata {
            endpoint: "kis-candidate.typed-validation".to_owned(),
            query: Vec::new(),
            headers: Vec::new(),
            mode: FetchMode::Credentialed,
        },
    );
    crate::parse_candidate_envelope(&typed_envelope).map_err(|error| {
        CandidateNormalizeError::CanonicalValidation {
            kind,
            reason: error.to_string(),
        }
    })?;
    document.insert(
        "_lineage".to_owned(),
        serde_json::to_value(lineage).map_err(|error| CandidateNormalizeError::Serialization {
            kind,
            reason: error.to_string(),
        })?,
    );
    let lineage_query =
        serde_json::to_string(lineage).map_err(|error| CandidateNormalizeError::Serialization {
            kind,
            reason: error.to_string(),
        })?;
    let bytes = serde_json::to_vec(&Value::Object(document)).map_err(|error| {
        CandidateNormalizeError::Serialization {
            kind,
            reason: error.to_string(),
        }
    })?;
    validate_response(kind, &bytes).map_err(|error| {
        CandidateNormalizeError::CanonicalValidation {
            kind,
            reason: error.reason,
        }
    })?;
    let envelope = RawEnvelope::new(
        batch_id,
        kind,
        file_name,
        bytes,
        source.retrieved_at,
        RequestMetadata {
            endpoint: format!("kis-candidate.normalized/{NORMALIZER}/{kind}"),
            query: vec![
                ("upstream_batch_id".to_owned(), source.batch_id.to_string()),
                ("upstream_provider".to_owned(), source.provider.clone()),
                ("upstream_lineage".to_owned(), lineage_query),
            ],
            headers: Vec::new(),
            mode: FetchMode::Credentialed,
        },
    );
    Ok(envelope)
}

fn parse_object(
    kind: ResponseKind,
    file_name: &str,
    bytes: &[u8],
) -> Result<Map<String, Value>, CandidateNormalizeError> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| malformed(kind, file_name, format!("invalid JSON: {error}")))?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| malformed(kind, file_name, "response must be an object"))
}

fn require_rt_ok(
    kind: ResponseKind,
    file_name: &str,
    object: &Map<String, Value>,
) -> Result<(), CandidateNormalizeError> {
    if object.get("rt_cd").and_then(Value::as_str) != Some("0") {
        return Err(CandidateNormalizeError::InvalidField {
            kind,
            file_name: file_name.to_owned(),
            field: "rt_cd".to_owned(),
            value: object
                .get("rt_cd")
                .map(ToString::to_string)
                .unwrap_or_else(|| "<missing>".to_owned()),
        });
    }
    Ok(())
}

fn require_endpoint(metadata: &FileEntry, expected: &str) -> Result<(), CandidateNormalizeError> {
    if metadata.request.endpoint != expected {
        return Err(CandidateNormalizeError::UnexpectedEndpoint {
            kind: metadata.kind,
            file_name: metadata.file_name.clone(),
            endpoint: metadata.request.endpoint.clone(),
        });
    }
    Ok(())
}

fn query_value<'a>(metadata: &'a FileEntry, key: &str) -> Result<&'a str, CandidateNormalizeError> {
    metadata
        .request
        .query
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| CandidateNormalizeError::MissingField {
            kind: metadata.kind,
            file_name: metadata.file_name.clone(),
            field: format!("request.query.{key}"),
        })
}

fn canonical_instrument(
    symbol: &str,
    kind: ResponseKind,
    file_name: &str,
) -> Result<String, CandidateNormalizeError> {
    if symbol.len() != 6 || !symbol.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(CandidateNormalizeError::InvalidField {
            kind,
            file_name: file_name.to_owned(),
            field: "FID_INPUT_ISCD".to_owned(),
            value: symbol.to_owned(),
        });
    }
    Ok(format!("{symbol}.KRX"))
}

fn normalize_flows(
    source: &ManifestEntry,
    stored: &[StoredFile],
    lineage: &CandidateNormalizationLineage,
    batch_id: BatchId,
) -> Result<RawEnvelope, CandidateNormalizeError> {
    let mut rows = BTreeMap::<(String, TradingDate, &'static str), Value>::new();
    for (metadata, file) in source_files(source, stored, ResponseKind::InvestorFlow)? {
        require_endpoint(metadata, INVESTOR_FLOW_PATH)?;
        let object = parse_object(ResponseKind::InvestorFlow, &file.file_name, &file.bytes)?;
        require_rt_ok(ResponseKind::InvestorFlow, &file.file_name, &object)?;
        let symbol = query_value(metadata, "FID_INPUT_ISCD")?;
        let instrument = canonical_instrument(symbol, ResponseKind::InvestorFlow, &file.file_name)?;
        let output = rows_from_field(
            ResponseKind::InvestorFlow,
            &file.file_name,
            &object,
            "output2",
        )?;
        for row in output {
            let date_text = required_string_any(
                ResponseKind::InvestorFlow,
                &file.file_name,
                row,
                &["stck_bsop_date", "bsop_date"],
            )?;
            let trade_date = parse_date(
                ResponseKind::InvestorFlow,
                &file.file_name,
                "stck_bsop_date",
                date_text,
            )?;
            if trade_date > source.date {
                return Err(CandidateNormalizeError::InvalidField {
                    kind: ResponseKind::InvestorFlow,
                    file_name: file.file_name.clone(),
                    field: "stck_bsop_date".to_owned(),
                    value: date_text.to_owned(),
                });
            }
            for (class, amount_field, volume_field) in [
                ("FOREIGN", "frgn_ntby_tr_pbmn", "frgn_ntby_qty"),
                ("INSTITUTION", "orgn_ntby_tr_pbmn", "orgn_ntby_qty"),
            ] {
                let amount = scale_flow_amount(
                    required_number(
                        ResponseKind::InvestorFlow,
                        &file.file_name,
                        row,
                        amount_field,
                    )?,
                    FLOW_AMOUNT_SCALE,
                    ResponseKind::InvestorFlow,
                    &file.file_name,
                    amount_field,
                )?;
                let volume = required_number(
                    ResponseKind::InvestorFlow,
                    &file.file_name,
                    row,
                    volume_field,
                )?;
                let revision = format!("{FLOW_REVISION}:{}", source.date.to_iso());
                let canonical = json_object([
                    ("instrument", Value::String(instrument.to_owned())),
                    ("trade_date", Value::String(trade_date.to_iso())),
                    ("investor_class", Value::String(class.to_owned())),
                    ("net_amount", amount),
                    ("net_volume", volume),
                    ("currency", Value::String("KRW".to_owned())),
                    ("volume_unit", Value::String("SHARE".to_owned())),
                    ("source_revision", Value::String(revision.clone())),
                    ("available_at", timestamp_value(source.retrieved_at)),
                ]);
                let key = (instrument.to_owned(), trade_date, class);
                if let Some(previous) = rows.insert(key.clone(), Value::Object(canonical.clone())) {
                    if previous == Value::Object(canonical) {
                        return Err(CandidateNormalizeError::DuplicateRow {
                            kind: ResponseKind::InvestorFlow,
                            key: format!("{} {} {}", key.0, key.1, key.2),
                        });
                    }
                    return Err(CandidateNormalizeError::ConflictingRow {
                        kind: ResponseKind::InvestorFlow,
                        key: format!("{} {} {}", key.0, key.1, key.2),
                    });
                }
            }
        }
    }
    let document = json_object([
        ("schema_version", Value::Number(Number::from(1))),
        ("source", Value::String(FLOW_REVISION.to_owned())),
        (
            "flows",
            Value::Array(rows.into_values().collect::<Vec<Value>>()),
        ),
    ]);
    canonical_envelope(
        ResponseKind::InvestorFlow,
        "investor-flow.json",
        document,
        source,
        lineage,
        batch_id,
    )
}

fn rows_from_field<'a>(
    kind: ResponseKind,
    file_name: &str,
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<Vec<&'a Map<String, Value>>, CandidateNormalizeError> {
    let value = object
        .get(field)
        .ok_or_else(|| CandidateNormalizeError::MissingField {
            kind,
            file_name: file_name.to_owned(),
            field: field.to_owned(),
        })?;
    match value {
        Value::Array(rows) => rows
            .iter()
            .map(|row| {
                row.as_object().ok_or_else(|| {
                    malformed(kind, file_name, format!("{field} row must be an object"))
                })
            })
            .collect(),
        Value::Object(row) => Ok(vec![row]),
        _ => Err(malformed(
            kind,
            file_name,
            format!("{field} must be an object or array"),
        )),
    }
}

fn required_string_any<'a>(
    kind: ResponseKind,
    file_name: &str,
    row: &'a Map<String, Value>,
    fields: &[&str],
) -> Result<&'a str, CandidateNormalizeError> {
    for field in fields {
        if let Some(value) = row.get(*field).and_then(Value::as_str)
            && !value.trim().is_empty()
        {
            return Ok(value);
        }
    }
    Err(CandidateNormalizeError::MissingField {
        kind,
        file_name: file_name.to_owned(),
        field: fields.join("|"),
    })
}

fn required_number(
    kind: ResponseKind,
    file_name: &str,
    row: &Map<String, Value>,
    field: &str,
) -> Result<Value, CandidateNormalizeError> {
    let raw = row
        .get(field)
        .ok_or_else(|| CandidateNormalizeError::MissingField {
            kind,
            file_name: file_name.to_owned(),
            field: field.to_owned(),
        })?;
    number_value(kind, file_name, field, raw)
}

fn number_value(
    kind: ResponseKind,
    file_name: &str,
    field: &str,
    value: &Value,
) -> Result<Value, CandidateNormalizeError> {
    let parsed = match value {
        Value::Number(number) => number
            .as_f64()
            .ok_or_else(|| invalid_field(kind, file_name, field, value))?,
        Value::String(string) => string
            .trim()
            .replace(',', "")
            .parse::<f64>()
            .map_err(|_| invalid_field(kind, file_name, field, value))?,
        _ => return Err(invalid_field(kind, file_name, field, value)),
    };
    Number::from_f64(parsed)
        .map(Value::Number)
        .ok_or_else(|| invalid_field(kind, file_name, field, value))
}

fn scale_flow_amount(
    value: Value,
    scale: u64,
    kind: ResponseKind,
    file_name: &str,
    field: &str,
) -> Result<Value, CandidateNormalizeError> {
    let number = value
        .as_f64()
        .ok_or_else(|| invalid_field(kind, file_name, field, &value))?;
    Number::from_f64(number * scale as f64)
        .map(Value::Number)
        .ok_or_else(|| invalid_field(kind, file_name, field, &value))
}

fn timestamp_value(timestamp: UtcTimestamp) -> Value {
    Value::String(timestamp.to_rfc3339())
}

fn malformed(
    kind: ResponseKind,
    file_name: &str,
    reason: impl Into<String>,
) -> CandidateNormalizeError {
    CandidateNormalizeError::Malformed {
        kind,
        file_name: file_name.to_owned(),
        reason: reason.into(),
    }
}

fn invalid_field(
    kind: ResponseKind,
    file_name: &str,
    field: &str,
    value: impl ToString,
) -> CandidateNormalizeError {
    CandidateNormalizeError::InvalidField {
        kind,
        file_name: file_name.to_owned(),
        field: field.to_owned(),
        value: value.to_string(),
    }
}

fn json_object<const N: usize>(fields: [(&str, Value); N]) -> Map<String, Value> {
    fields
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

fn parse_date(
    kind: ResponseKind,
    file_name: &str,
    field: &str,
    value: &str,
) -> Result<TradingDate, CandidateNormalizeError> {
    let normalized = if value.len() == 8 {
        format!("{}-{}-{}", &value[..4], &value[4..6], &value[6..8])
    } else {
        value.to_owned()
    };
    TradingDate::parse(&normalized).map_err(|_| CandidateNormalizeError::InvalidField {
        kind,
        file_name: file_name.to_owned(),
        field: field.to_owned(),
        value: value.to_owned(),
    })
}
