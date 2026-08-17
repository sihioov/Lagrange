//! Provider-wire to provider-neutral normalization for KIS EOD deliveries.
//!
//! KIS responses are deliberately kept in the immutable `provider=kis` raw
//! batch.  This module reads that batch, maps only documented fields into the
//! existing four-document contract, and writes a second immutable
//! `provider=kis-normalized` batch with
//! exactly one document per [`ResponseKind`].  The wire batch is never
//! rewritten or deleted.  Every canonical document carries the complete
//! upstream batch/file/hash lineage in `_lineage` so a normalized row can be
//! traced back without consulting mutable state.

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use std::time::Duration;

use domain::{BatchId, ContentHash, TradingDate};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};
use uuid::Uuid;

use crate::contract::{
    FetchMode, MARKET_KR, PROVIDER_KIS, PROVIDER_KIS_NORMALIZED, RawEnvelope, RequestMetadata,
    ResponseKind, StoredFile,
};
use crate::providers::kis::KR_ETF_CORE_SYMBOLS;
use crate::storage::{BatchSpec, FileEntry, ManifestEntry, RawStore, StoreError};
use crate::validate::validate_response;

const NORMALIZER: &str = "kis-wire-to-canonical-v1";
const NORMALIZER_SCHEMA_VERSION: u32 = 1;
const COLLISION_RETRIES: usize = 100;
const COLLISION_RETRY_DELAY: Duration = Duration::from_millis(2);
const DAILY_BARS_ENDPOINT: &str = "/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice";
const REFERENCE_ENDPOINT: &str = "/uapi/domestic-stock/v1/quotations/inquire-price";
const CALENDAR_ENDPOINT: &str = "/uapi/domestic-stock/v1/quotations/chk-holiday";
const KSD_ENDPOINTS: [&str; 6] = [
    "/uapi/domestic-stock/v1/ksdinfo/paidin-capin",
    "/uapi/domestic-stock/v1/ksdinfo/bonus-issue",
    "/uapi/domestic-stock/v1/ksdinfo/dividend",
    "/uapi/domestic-stock/v1/ksdinfo/merger-split",
    "/uapi/domestic-stock/v1/ksdinfo/rev-split",
    "/uapi/domestic-stock/v1/ksdinfo/cap-dcrs",
];

/// One immutable source file recorded in normalization lineage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NormalizationSourceFile {
    pub kind: ResponseKind,
    pub file_name: String,
    pub content_hash: ContentHash,
}

/// The source identity attached to each canonical document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NormalizationLineage {
    pub schema_version: u32,
    pub normalizer: String,
    pub upstream_provider: String,
    pub upstream_market: String,
    pub upstream_batch_id: BatchId,
    pub upstream_files: Vec<NormalizationSourceFile>,
}

/// A stored canonical KIS batch and the source identity it was derived from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizationOutcome {
    pub source_batch_id: BatchId,
    pub entry: ManifestEntry,
    pub files: Vec<StoredFile>,
    pub lineage: NormalizationLineage,
}

/// Why a KIS wire batch could not be mapped into the neutral contract.
#[derive(Debug, thiserror::Error)]
pub enum NormalizeError {
    #[error(
        "normalization supports only {expected_provider}/{expected_market}, got {provider}/{market}"
    )]
    UnsupportedScope {
        expected_provider: &'static str,
        expected_market: &'static str,
        provider: String,
        market: String,
    },
    #[error("source KIS batch is not credentialed")]
    UnsupportedMode,
    #[error("existing deterministic normalized batch {batch_id} conflicts: {reason}")]
    ExistingBatchConflict { batch_id: BatchId, reason: String },
    #[error("normalized evidence count differs from manifest: expected {expected}, got {actual}")]
    EvidenceCountMismatch { expected: usize, actual: usize },
    #[error("normalized evidence is missing manifest file {file_name}")]
    EvidenceMissing { file_name: String },
    #[error("normalized evidence contains unmanifested file {file_name}")]
    EvidenceUnexpected { file_name: String },
    #[error("normalized evidence hash mismatch for {file_name}: expected {expected}, got {actual}")]
    EvidenceHashMismatch {
        file_name: String,
        expected: String,
        actual: String,
    },
    #[error(
        "normalized evidence size mismatch for {file_name}: expected {expected} bytes, got {actual}"
    )]
    EvidenceSizeMismatch {
        file_name: String,
        expected: u64,
        actual: u64,
    },
    #[error("source Raw read failed: {0}")]
    Store(#[from] StoreError),
    #[error("source KIS batch is missing {kind} response files")]
    MissingKind { kind: ResponseKind },
    #[error("source KIS file {file_name} has unexpected endpoint {endpoint}")]
    UnexpectedEndpoint { file_name: String, endpoint: String },
    #[error("malformed KIS {kind} file {file_name}: {reason}")]
    Malformed {
        kind: ResponseKind,
        file_name: String,
        reason: String,
    },
    #[error("KIS {kind} file {file_name} is missing field {field}")]
    MissingField {
        kind: ResponseKind,
        file_name: String,
        field: String,
    },
    #[error("invalid KIS {kind} field {field} in {file_name}: {value}")]
    InvalidField {
        kind: ResponseKind,
        file_name: String,
        field: String,
        value: String,
    },
    #[error("duplicate {kind} row {key} in KIS source")]
    DuplicateRow { kind: ResponseKind, key: String },
    #[error("conflicting {kind} row {key} in KIS source")]
    ConflictingRow { kind: ResponseKind, key: String },
    #[error("KIS corporate-action row in {file_name} cannot be represented: {reason}")]
    UnsupportedAction { file_name: String, reason: String },
    #[error("canonical {kind} document failed neutral validation: {reason}")]
    CanonicalValidation { kind: ResponseKind, reason: String },
    #[error("canonical {kind} document serialization failed: {reason}")]
    Serialization { kind: ResponseKind, reason: String },
    #[error("calendar has no target-date observation for {target_date}")]
    MissingTargetObservation { target_date: String },
    #[error(
        "target-date bar coverage disagrees with calendar for {target_date}: open={calendar_open}, expected={expected}, actual={actual}"
    )]
    TargetBarCoverage {
        target_date: String,
        calendar_open: bool,
        expected: usize,
        actual: usize,
        missing: Vec<String>,
        unexpected: Vec<String>,
    },
}

/// Returns the stable canonical batch identity for one KIS source batch.
///
/// The normalizer version is part of the UUID-v5 name, so a future mapping
/// version cannot silently reuse bytes produced by this version.
pub fn deterministic_kis_normalized_batch_id(source_batch_id: BatchId) -> BatchId {
    let name = format!(
        "provider={PROVIDER_KIS_NORMALIZED}\nnormalizer={NORMALIZER}\nsource_batch={source_batch_id}"
    );
    BatchId::from_uuid(Uuid::new_v5(&Uuid::NAMESPACE_URL, name.as_bytes()))
}

/// Reads one stored KIS wire batch, normalizes it, and stores one immutable
/// `provider=kis-normalized` batch. The canonical identity is deterministic
/// for the source batch, so retries return the already verified immutable
/// result instead of appending a second manifest row. The source batch is
/// only read through RawStore's hash verification path and is never changed.
pub fn normalize_kis_batch(
    raw: &RawStore,
    source: &ManifestEntry,
) -> Result<NormalizationOutcome, NormalizeError> {
    validate_source_scope(source)?;
    let stored = raw.read_batch_bytes(&source.provider, &source.market, source)?;
    let batch_id = deterministic_kis_normalized_batch_id(source.batch_id);
    let envelopes = normalize_kis_envelopes_with_batch_id(source, &stored, batch_id)?;
    let source_files = source_lineage(source);
    let lineage = NormalizationLineage {
        schema_version: NORMALIZER_SCHEMA_VERSION,
        normalizer: NORMALIZER.to_owned(),
        upstream_provider: source.provider.clone(),
        upstream_market: source.market.clone(),
        upstream_batch_id: source.batch_id,
        upstream_files: source_files,
    };

    let spec = BatchSpec {
        provider: PROVIDER_KIS_NORMALIZED,
        market: &source.market,
        date: &source.date,
        batch_id,
        entitlement_reference: source.entitlement_reference.as_deref(),
        mode: FetchMode::Credentialed,
    };
    let expected_entry = expected_manifest_entry(source, &spec, &envelopes);

    if let Some(outcome) =
        load_existing_normalized_batch(raw, source, &expected_entry, &envelopes, lineage.clone())?
    {
        return Ok(outcome);
    }

    match raw.store_batch(&spec, &envelopes) {
        Ok(entry) => {
            if entry != expected_entry {
                return Err(existing_batch_conflict(
                    batch_id,
                    "RawStore returned manifest metadata different from the deterministic contract",
                ));
            }
            let files = raw.read_batch_bytes(PROVIDER_KIS_NORMALIZED, MARKET_KR, &entry)?;
            validate_stored_evidence(&entry, &files)?;
            Ok(NormalizationOutcome {
                source_batch_id: source.batch_id,
                entry,
                files,
                lineage,
            })
        }
        Err(error @ StoreError::FileExists { .. }) => {
            // Another caller can create the deterministic directory before its
            // manifest line becomes visible. Re-read a few times so concurrent
            // retries converge once the durable metadata is exposed.
            for _ in 0..COLLISION_RETRIES {
                if let Some(outcome) = load_existing_normalized_batch(
                    raw,
                    source,
                    &expected_entry,
                    &envelopes,
                    lineage.clone(),
                )? {
                    return Ok(outcome);
                }
                std::thread::sleep(COLLISION_RETRY_DELAY);
            }
            Err(NormalizeError::Store(error))
        }
        Err(error) => Err(NormalizeError::Store(error)),
    }
}

fn expected_manifest_entry(
    source: &ManifestEntry,
    spec: &BatchSpec<'_>,
    envelopes: &[RawEnvelope],
) -> ManifestEntry {
    ManifestEntry {
        batch_id: spec.batch_id,
        provider: spec.provider.to_owned(),
        market: spec.market.to_owned(),
        date: *spec.date,
        retrieved_at: source.retrieved_at,
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

fn load_existing_normalized_batch(
    raw: &RawStore,
    source: &ManifestEntry,
    expected_entry: &ManifestEntry,
    expected_envelopes: &[RawEnvelope],
    lineage: NormalizationLineage,
) -> Result<Option<NormalizationOutcome>, NormalizeError> {
    let existing = raw
        .read_reconciled_manifest(PROVIDER_KIS_NORMALIZED, MARKET_KR)?
        .into_iter()
        .find(|entry| entry.batch_id == expected_entry.batch_id);
    let Some(entry) = existing else {
        return Ok(None);
    };
    if &entry != expected_entry {
        return Err(existing_batch_conflict(
            entry.batch_id,
            "manifest metadata, canonical shape, lineage, or content hash differs",
        ));
    }
    let files = raw.read_batch_bytes(PROVIDER_KIS_NORMALIZED, MARKET_KR, &entry)?;
    validate_stored_evidence(&entry, &files)?;
    for expected in expected_envelopes {
        let Some(actual) = files
            .iter()
            .find(|file| file.file_name == expected.file_name)
        else {
            return Err(existing_batch_conflict(
                entry.batch_id,
                format!("canonical file {} is missing", expected.file_name),
            ));
        };
        if actual.bytes != expected.bytes {
            return Err(existing_batch_conflict(
                entry.batch_id,
                format!("canonical file {} bytes differ", expected.file_name),
            ));
        }
    }
    Ok(Some(NormalizationOutcome {
        source_batch_id: source.batch_id,
        entry,
        files,
        lineage,
    }))
}

fn existing_batch_conflict(batch_id: BatchId, reason: impl Into<String>) -> NormalizeError {
    NormalizeError::ExistingBatchConflict {
        batch_id,
        reason: reason.into(),
    }
}

/// Normalizes a verified KIS source batch into exactly four canonical raw
/// envelopes without writing them.  This is useful for dry-run validation and
/// deterministic no-network tests; [`normalize_kis_batch`] persists the same
/// envelopes through the immutable RawStore.
pub fn normalize_kis_envelopes(
    source: &ManifestEntry,
    stored: &[StoredFile],
) -> Result<Vec<RawEnvelope>, NormalizeError> {
    validate_source_scope(source)?;
    normalize_kis_envelopes_with_batch_id(source, stored, BatchId::generate())
}

fn normalize_kis_envelopes_with_batch_id(
    source: &ManifestEntry,
    stored: &[StoredFile],
    batch_id: BatchId,
) -> Result<Vec<RawEnvelope>, NormalizeError> {
    validate_source_scope(source)?;
    validate_stored_evidence(source, stored)?;
    let source_files = source_lineage(source);
    let lineage = NormalizationLineage {
        schema_version: NORMALIZER_SCHEMA_VERSION,
        normalizer: NORMALIZER.to_owned(),
        upstream_provider: source.provider.clone(),
        upstream_market: source.market.clone(),
        upstream_batch_id: source.batch_id,
        upstream_files: source_files,
    };
    let envelopes = vec![
        normalize_bars(source, stored, &lineage, batch_id)?,
        normalize_reference(source, stored, &lineage, batch_id)?,
        normalize_calendar(source, stored, &lineage, batch_id)?,
        normalize_actions(source, stored, &lineage, batch_id)?,
    ];
    validate_target_bar_coverage(source, &envelopes)?;
    Ok(envelopes)
}

fn validate_source_scope(source: &ManifestEntry) -> Result<(), NormalizeError> {
    if source.provider != PROVIDER_KIS || source.market != MARKET_KR {
        return Err(NormalizeError::UnsupportedScope {
            expected_provider: PROVIDER_KIS,
            expected_market: MARKET_KR,
            provider: source.provider.clone(),
            market: source.market.clone(),
        });
    }
    if source.mode != FetchMode::Credentialed {
        return Err(NormalizeError::UnsupportedMode);
    }
    Ok(())
}

fn validate_stored_evidence(
    source: &ManifestEntry,
    stored: &[StoredFile],
) -> Result<(), NormalizeError> {
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
        return Err(NormalizeError::EvidenceMissing {
            file_name: (*file_name).to_owned(),
        });
    }
    if let Some(file_name) = actual.difference(&expected).next() {
        return Err(NormalizeError::EvidenceUnexpected {
            file_name: (*file_name).to_owned(),
        });
    }
    if stored.len() != actual.len() || source.files.len() != expected.len() {
        return Err(NormalizeError::EvidenceCountMismatch {
            expected: source.files.len(),
            actual: stored.len(),
        });
    }
    if source.files.len() != stored.len() {
        return Err(NormalizeError::EvidenceCountMismatch {
            expected: source.files.len(),
            actual: stored.len(),
        });
    }
    for metadata in &source.files {
        let file = stored
            .iter()
            .find(|file| file.file_name == metadata.file_name)
            .ok_or_else(|| NormalizeError::EvidenceMissing {
                file_name: metadata.file_name.clone(),
            })?;
        let actual_hash = ContentHash::from_bytes(&file.bytes);
        if actual_hash != metadata.content_hash {
            return Err(NormalizeError::EvidenceHashMismatch {
                file_name: metadata.file_name.clone(),
                expected: metadata.content_hash.to_string(),
                actual: actual_hash.to_string(),
            });
        }
        let actual_size = file.bytes.len() as u64;
        if actual_size != metadata.size_bytes {
            return Err(NormalizeError::EvidenceSizeMismatch {
                file_name: metadata.file_name.clone(),
                expected: metadata.size_bytes,
                actual: actual_size,
            });
        }
    }
    Ok(())
}

fn source_lineage(source: &ManifestEntry) -> Vec<NormalizationSourceFile> {
    let mut files = source
        .files
        .iter()
        .map(|file| NormalizationSourceFile {
            kind: file.kind,
            file_name: file.file_name.clone(),
            content_hash: file.content_hash.clone(),
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| (left.kind, &left.file_name).cmp(&(right.kind, &right.file_name)));
    files
}

fn normalize_bars(
    source: &ManifestEntry,
    stored: &[StoredFile],
    lineage: &NormalizationLineage,
    batch_id: BatchId,
) -> Result<RawEnvelope, NormalizeError> {
    let files = source_files(source, stored, ResponseKind::Bars)?;
    let mut rows = BTreeMap::<(String, TradingDate), Value>::new();
    let mut symbols = BTreeSet::new();
    for (metadata, file) in files {
        require_endpoint(metadata, DAILY_BARS_ENDPOINT)?;
        let document = parse_object(ResponseKind::Bars, &file.file_name, &file.bytes)?;
        require_rt_ok(ResponseKind::Bars, &file.file_name, &document)?;
        let symbol = query_symbol(ResponseKind::Bars, metadata, &file.file_name)?;
        let instrument = canonical_instrument(&symbol);
        symbols.insert(instrument.clone());
        let output = required_array(ResponseKind::Bars, &file.file_name, &document, "output2")?;
        for row in output {
            let row = row.as_object().ok_or_else(|| {
                malformed(
                    ResponseKind::Bars,
                    &file.file_name,
                    "output2 row must be an object",
                )
            })?;
            let date_text =
                required_string(ResponseKind::Bars, &file.file_name, row, "stck_bsop_date")?;
            let date = parse_kis_date(
                ResponseKind::Bars,
                &file.file_name,
                "stck_bsop_date",
                date_text,
            )?;
            let canonical = canonical_bar(&file.file_name, row, &instrument, date)?;
            // Parse and validate every row, but only carry the requested target
            // date into the neutral document.
            if date != source.date {
                continue;
            }
            let key = (instrument.clone(), date);
            if let Some(previous) = rows.insert(key.clone(), canonical.clone()) {
                if previous == canonical {
                    return Err(NormalizeError::DuplicateRow {
                        kind: ResponseKind::Bars,
                        key: format!("{} {}", key.0, key.1),
                    });
                }
                return Err(NormalizeError::ConflictingRow {
                    kind: ResponseKind::Bars,
                    key: format!("{} {}", key.0, key.1),
                });
            }
        }
    }
    let expected_symbols = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| canonical_instrument(symbol))
        .collect::<BTreeSet<_>>();
    if symbols != expected_symbols {
        return Err(NormalizeError::Malformed {
            kind: ResponseKind::Bars,
            file_name: "bars.json".to_owned(),
            reason: "KIS bars did not cover the fixed KR ETF core universe".to_owned(),
        });
    }
    let bars = rows.into_values().collect::<Vec<_>>();
    let instruments = symbols
        .into_iter()
        .map(|symbol| {
            Value::Object(json_object([
                ("symbol", Value::String(symbol)),
                ("currency", Value::String("KRW".to_owned())),
                ("lot_size", Value::Number(Number::from(1))),
            ]))
        })
        .collect::<Vec<_>>();
    let mut document = json_object([
        (
            "dataset_id",
            Value::String(format!("kr-etf-daily-{}", source.date)),
        ),
        ("schema_version", Value::Number(Number::from(1))),
        ("currency", Value::String("KRW".to_owned())),
        ("instruments", Value::Array(instruments)),
        ("bars", Value::Array(bars)),
    ]);
    add_lineage(&mut document, lineage);
    canonical_envelope(
        ResponseKind::Bars,
        "bars.json",
        document,
        source,
        lineage,
        batch_id,
    )
}

fn canonical_bar(
    file_name: &str,
    row: &Map<String, Value>,
    instrument: &str,
    date: TradingDate,
) -> Result<Value, NormalizeError> {
    let mut canonical = json_object([
        ("instrument", Value::String(instrument.to_owned())),
        ("date", Value::String(date.to_iso())),
        (
            "open",
            number_field(ResponseKind::Bars, file_name, row, "stck_oprc")?,
        ),
        (
            "high",
            number_field(ResponseKind::Bars, file_name, row, "stck_hgpr")?,
        ),
        (
            "low",
            number_field(ResponseKind::Bars, file_name, row, "stck_lwpr")?,
        ),
        (
            "close",
            number_field(ResponseKind::Bars, file_name, row, "stck_clpr")?,
        ),
        (
            "volume",
            number_field(ResponseKind::Bars, file_name, row, "acml_vol")?,
        ),
    ]);
    if let Some(value) = optional_number_field(row, "acml_tr_pbmn")? {
        canonical.insert("value".to_owned(), value);
    }
    Ok(Value::Object(canonical))
}

fn normalize_reference(
    source: &ManifestEntry,
    stored: &[StoredFile],
    lineage: &NormalizationLineage,
    batch_id: BatchId,
) -> Result<RawEnvelope, NormalizeError> {
    let files = source_files(source, stored, ResponseKind::Reference)?;
    let mut instruments = BTreeMap::<String, Value>::new();
    for (metadata, file) in files {
        require_endpoint(metadata, REFERENCE_ENDPOINT)?;
        let document = parse_object(ResponseKind::Reference, &file.file_name, &file.bytes)?;
        require_rt_ok(ResponseKind::Reference, &file.file_name, &document)?;
        let symbol = query_symbol(ResponseKind::Reference, metadata, &file.file_name)?;
        let output = required_object(
            ResponseKind::Reference,
            &file.file_name,
            &document,
            "output",
        )?;
        // std_pdno is the documented standard product number.  Some KIS
        // deployments expose the same value under stck_shrn_iscd; if present,
        // it must agree with the request symbol.
        if let Some(provider_symbol) = first_string(output, &["std_pdno", "stck_shrn_iscd"])
            && provider_symbol != symbol
        {
            return Err(NormalizeError::InvalidField {
                kind: ResponseKind::Reference,
                file_name: file.file_name.clone(),
                field: "std_pdno".to_owned(),
                value: provider_symbol.to_owned(),
            });
        }
        let name = first_string(output, &["prdt_name", "hts_kor_isnm"])
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| missing_field(ResponseKind::Reference, &file.file_name, "prdt_name"))?;
        let instrument = json_object([
            ("symbol", Value::String(canonical_instrument(&symbol))),
            ("name", Value::String(name.to_owned())),
            ("lot_size", Value::Number(Number::from(1))),
            ("currency", Value::String("KRW".to_owned())),
            ("kind", Value::String("equity-etf".to_owned())),
        ]);
        let instrument = Value::Object(instrument);
        if let Some(previous) = instruments.insert(symbol.clone(), instrument.clone()) {
            if previous == instrument {
                return Err(NormalizeError::DuplicateRow {
                    kind: ResponseKind::Reference,
                    key: symbol,
                });
            }
            return Err(NormalizeError::ConflictingRow {
                kind: ResponseKind::Reference,
                key: symbol,
            });
        }
    }
    let expected_symbols = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| (*symbol).to_owned())
        .collect::<BTreeSet<_>>();
    if instruments.keys().cloned().collect::<BTreeSet<_>>() != expected_symbols {
        return Err(NormalizeError::Malformed {
            kind: ResponseKind::Reference,
            file_name: "reference.json".to_owned(),
            reason: "KIS reference did not cover the fixed KR ETF core universe".to_owned(),
        });
    }
    let mut document = json_object([
        ("source", Value::String("kis-inquire-price-v1".to_owned())),
        (
            "instruments",
            Value::Array(instruments.into_values().collect()),
        ),
    ]);
    add_lineage(&mut document, lineage);
    canonical_envelope(
        ResponseKind::Reference,
        "reference.json",
        document,
        source,
        lineage,
        batch_id,
    )
}

fn normalize_calendar(
    source: &ManifestEntry,
    stored: &[StoredFile],
    lineage: &NormalizationLineage,
    batch_id: BatchId,
) -> Result<RawEnvelope, NormalizeError> {
    let files = source_files(source, stored, ResponseKind::Calendar)?;
    let mut dates = BTreeMap::<TradingDate, bool>::new();
    for (metadata, file) in files {
        require_endpoint(metadata, CALENDAR_ENDPOINT)?;
        let document = parse_object(ResponseKind::Calendar, &file.file_name, &file.bytes)?;
        require_rt_ok(ResponseKind::Calendar, &file.file_name, &document)?;
        let output = document
            .get("output")
            .ok_or_else(|| missing_field(ResponseKind::Calendar, &file.file_name, "output"))?;
        let rows = match output {
            Value::Array(rows) => rows.clone(),
            Value::Object(_) => vec![output.clone()],
            _ => {
                return Err(invalid_field(
                    ResponseKind::Calendar,
                    &file.file_name,
                    "output",
                    output,
                ));
            }
        };
        for row in rows {
            let row = row.as_object().ok_or_else(|| {
                malformed(
                    ResponseKind::Calendar,
                    &file.file_name,
                    "output row must be an object",
                )
            })?;
            let date_text =
                required_string(ResponseKind::Calendar, &file.file_name, row, "bass_dt")?;
            let date = parse_kis_date(
                ResponseKind::Calendar,
                &file.file_name,
                "bass_dt",
                date_text,
            )?;
            let open = required_string(ResponseKind::Calendar, &file.file_name, row, "opnd_yn")?;
            let is_open = match open {
                "Y" => true,
                "N" => false,
                _ => {
                    return Err(NormalizeError::InvalidField {
                        kind: ResponseKind::Calendar,
                        file_name: file.file_name.clone(),
                        field: "opnd_yn".to_owned(),
                        value: open.to_owned(),
                    });
                }
            };
            // KIS can return a window around BASS_DT.  The canonical EOD
            // batch is target-date scoped; surrounding rows are validated but
            // are not copied into this batch.
            if date != source.date {
                continue;
            }
            if let Some(previous) = dates.insert(date, is_open) {
                if previous == is_open {
                    return Err(NormalizeError::DuplicateRow {
                        kind: ResponseKind::Calendar,
                        key: date.to_iso(),
                    });
                }
                return Err(NormalizeError::ConflictingRow {
                    kind: ResponseKind::Calendar,
                    key: date.to_iso(),
                });
            }
        }
    }

    if !dates.contains_key(&source.date) {
        return Err(NormalizeError::MissingTargetObservation {
            target_date: source.date.to_iso(),
        });
    }

    let mut sessions = Vec::new();
    let mut holidays = Vec::new();
    for (date, is_open) in dates {
        if is_open {
            sessions.push(Value::Object(json_object([
                ("date", Value::String(date.to_iso())),
                (
                    "open_utc",
                    Value::String(format!("{}T00:00:00Z", date.to_iso())),
                ),
                (
                    "close_utc",
                    Value::String(format!("{}T06:30:00Z", date.to_iso())),
                ),
            ])));
        } else {
            holidays.push(Value::Object(json_object([(
                "date",
                Value::String(date.to_iso()),
            )])));
        }
    }
    let mut document = json_object([
        (
            "calendar_id",
            Value::String("kis-chk-holiday-v1".to_owned()),
        ),
        ("schema_version", Value::Number(Number::from(1))),
        ("source", Value::String("kis".to_owned())),
        ("timezone", Value::String("Asia/Seoul".to_owned())),
        (
            "session_times_local",
            Value::Object(json_object([
                ("open", Value::String("09:00:00".to_owned())),
                ("close", Value::String("15:30:00".to_owned())),
            ])),
        ),
        ("sessions", Value::Array(sessions)),
        ("holidays", Value::Array(holidays)),
    ]);
    add_lineage(&mut document, lineage);
    canonical_envelope(
        ResponseKind::Calendar,
        "calendar.json",
        document,
        source,
        lineage,
        batch_id,
    )
}

fn normalize_actions(
    source: &ManifestEntry,
    stored: &[StoredFile],
    lineage: &NormalizationLineage,
    batch_id: BatchId,
) -> Result<RawEnvelope, NormalizeError> {
    let files = source_files(source, stored, ResponseKind::CorporateActions)?;
    for (metadata, file) in files {
        if !KSD_ENDPOINTS.contains(&metadata.request.endpoint.as_str()) {
            return Err(NormalizeError::UnexpectedEndpoint {
                file_name: metadata.file_name.clone(),
                endpoint: metadata.request.endpoint.clone(),
            });
        }
        let document = parse_object(ResponseKind::CorporateActions, &file.file_name, &file.bytes)?;
        require_rt_ok(ResponseKind::CorporateActions, &file.file_name, &document)?;
        let output = required_array_or_object(
            ResponseKind::CorporateActions,
            &file.file_name,
            &document,
            "output1",
        )?;
        if !output.is_empty() {
            return Err(NormalizeError::UnsupportedAction {
                file_name: file.file_name.clone(),
                reason:
                    "non-empty KIS corporate-action rows require an explicit reviewed field mapping"
                        .to_owned(),
            });
        }
    }
    let mut document = json_object([
        (
            "dataset_id",
            Value::String(format!("kr-etf-daily-{}", source.date)),
        ),
        ("schema_version", Value::Number(Number::from(1))),
        ("actions", Value::Array(Vec::new())),
    ]);
    add_lineage(&mut document, lineage);
    canonical_envelope(
        ResponseKind::CorporateActions,
        "corporate-actions.json",
        document,
        source,
        lineage,
        batch_id,
    )
}

fn validate_target_bar_coverage(
    source: &ManifestEntry,
    envelopes: &[RawEnvelope],
) -> Result<(), NormalizeError> {
    let bars = envelopes
        .iter()
        .find(|envelope| envelope.kind == ResponseKind::Bars)
        .expect("normalization always creates bars");
    let calendar = envelopes
        .iter()
        .find(|envelope| envelope.kind == ResponseKind::Calendar)
        .expect("normalization always creates calendar");
    let bars: Value = serde_json::from_slice(&bars.bytes).map_err(|error| {
        NormalizeError::CanonicalValidation {
            kind: ResponseKind::Bars,
            reason: error.to_string(),
        }
    })?;
    let calendar: Value = serde_json::from_slice(&calendar.bytes).map_err(|error| {
        NormalizeError::CanonicalValidation {
            kind: ResponseKind::Calendar,
            reason: error.to_string(),
        }
    })?;
    let target = source.date.to_iso();
    let calendar_open = calendar["sessions"]
        .as_array()
        .is_some_and(|sessions| sessions.iter().any(|row| row["date"] == target));
    let target_bars = bars["bars"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter(|row| row["date"] == target)
                .filter_map(|row| row["instrument"].as_str().map(str::to_owned))
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    let expected_symbols = if calendar_open {
        KR_ETF_CORE_SYMBOLS
            .iter()
            .map(|symbol| canonical_instrument(symbol))
            .collect::<BTreeSet<_>>()
    } else {
        BTreeSet::new()
    };
    // A provider may legitimately answer before the requested session's EOD
    // is published.  Preserve that delivery as an empty canonical bars file;
    // PublicationBundle will classify it as EOD_UNAVAILABLE.  Partial target
    // coverage remains a hard integrity failure: trading-day publication is
    // all-or-nothing for the fixed ETF universe.
    if target_bars.is_empty() {
        return Ok(());
    }
    if target_bars != expected_symbols {
        return Err(NormalizeError::TargetBarCoverage {
            target_date: target,
            calendar_open,
            expected: expected_symbols.len(),
            actual: target_bars.len(),
            missing: expected_symbols.difference(&target_bars).cloned().collect(),
            unexpected: target_bars.difference(&expected_symbols).cloned().collect(),
        });
    }
    Ok(())
}

fn source_files<'a>(
    source: &'a ManifestEntry,
    stored: &'a [StoredFile],
    kind: ResponseKind,
) -> Result<Vec<(&'a FileEntry, &'a StoredFile)>, NormalizeError> {
    let mut selected = source
        .files
        .iter()
        .filter(|metadata| metadata.kind == kind)
        .map(|metadata| {
            let file = stored
                .iter()
                .find(|file| file.file_name == metadata.file_name)
                .ok_or_else(|| NormalizeError::EvidenceMissing {
                    file_name: metadata.file_name.clone(),
                })?;
            Ok((metadata, file))
        })
        .collect::<Result<Vec<_>, NormalizeError>>()?;
    selected.sort_by(|left, right| left.0.file_name.cmp(&right.0.file_name));
    if selected.is_empty() {
        return Err(NormalizeError::MissingKind { kind });
    }
    Ok(selected)
}

fn canonical_envelope(
    kind: ResponseKind,
    file_name: &str,
    document: Map<String, Value>,
    source: &ManifestEntry,
    lineage: &NormalizationLineage,
    batch_id: BatchId,
) -> Result<RawEnvelope, NormalizeError> {
    let bytes = serde_json::to_vec(&Value::Object(document)).map_err(|error| {
        NormalizeError::Serialization {
            kind,
            reason: error.to_string(),
        }
    })?;
    validate_response(kind, &bytes).map_err(|error| NormalizeError::CanonicalValidation {
        kind,
        reason: error.reason,
    })?;
    let lineage_query =
        serde_json::to_string(lineage).map_err(|error| NormalizeError::Serialization {
            kind,
            reason: error.to_string(),
        })?;
    Ok(RawEnvelope::new(
        batch_id,
        kind,
        file_name,
        bytes,
        source.retrieved_at,
        RequestMetadata {
            endpoint: format!("kis.normalized/{NORMALIZER}/{kind}"),
            query: vec![
                ("upstream_batch_id".to_owned(), source.batch_id.to_string()),
                ("upstream_lineage".to_owned(), lineage_query),
            ],
            headers: Vec::new(),
            mode: FetchMode::Credentialed,
        },
    ))
}

fn add_lineage(document: &mut Map<String, Value>, lineage: &NormalizationLineage) {
    document.insert(
        "_lineage".to_owned(),
        serde_json::to_value(lineage).expect("lineage is serializable"),
    );
}

fn require_endpoint(metadata: &FileEntry, expected: &str) -> Result<(), NormalizeError> {
    if metadata.request.endpoint != expected {
        return Err(NormalizeError::UnexpectedEndpoint {
            file_name: metadata.file_name.clone(),
            endpoint: metadata.request.endpoint.clone(),
        });
    }
    Ok(())
}

fn parse_object(
    kind: ResponseKind,
    file_name: &str,
    bytes: &[u8],
) -> Result<Map<String, Value>, NormalizeError> {
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
) -> Result<(), NormalizeError> {
    if object.get("rt_cd").and_then(Value::as_str) != Some("0") {
        return Err(NormalizeError::InvalidField {
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

fn required_array<'a>(
    kind: ResponseKind,
    file_name: &str,
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<&'a Vec<Value>, NormalizeError> {
    object
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| missing_field(kind, file_name, field))
}

fn required_array_or_object(
    kind: ResponseKind,
    file_name: &str,
    object: &Map<String, Value>,
    field: &str,
) -> Result<Vec<Value>, NormalizeError> {
    match object.get(field) {
        Some(Value::Array(rows)) => Ok(rows.clone()),
        Some(Value::Object(row)) if row.is_empty() => Ok(Vec::new()),
        Some(Value::Object(row)) => Ok(vec![Value::Object(row.clone())]),
        Some(value) => Err(invalid_field(kind, file_name, field, value)),
        None => Err(missing_field(kind, file_name, field)),
    }
}

fn required_object<'a>(
    kind: ResponseKind,
    file_name: &str,
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<&'a Map<String, Value>, NormalizeError> {
    object
        .get(field)
        .and_then(Value::as_object)
        .ok_or_else(|| missing_field(kind, file_name, field))
}

fn required_string<'a>(
    kind: ResponseKind,
    file_name: &str,
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<&'a str, NormalizeError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| missing_field(kind, file_name, field))
}

fn query_symbol(
    kind: ResponseKind,
    metadata: &FileEntry,
    file_name: &str,
) -> Result<String, NormalizeError> {
    let value = metadata
        .request
        .query
        .iter()
        .find(|(key, _)| key == "FID_INPUT_ISCD")
        .map(|(_, value)| value.as_str())
        .filter(|value| value.len() == 6 && value.bytes().all(|byte| byte.is_ascii_digit()))
        .ok_or_else(|| missing_field(kind, file_name, "FID_INPUT_ISCD"))?;
    Ok(value.to_owned())
}

fn canonical_instrument(symbol: &str) -> String {
    format!("{symbol}.KRX")
}

fn parse_kis_date(
    kind: ResponseKind,
    file_name: &str,
    field: &str,
    value: &str,
) -> Result<TradingDate, NormalizeError> {
    let normalized = if value.len() == 8 && value.bytes().all(|byte| byte.is_ascii_digit()) {
        format!("{}-{}-{}", &value[..4], &value[4..6], &value[6..])
    } else {
        value.to_owned()
    };
    TradingDate::parse(&normalized).map_err(|_| NormalizeError::InvalidField {
        kind,
        file_name: file_name.to_owned(),
        field: field.to_owned(),
        value: value.to_owned(),
    })
}

fn number_field(
    kind: ResponseKind,
    file_name: &str,
    object: &Map<String, Value>,
    field: &str,
) -> Result<Value, NormalizeError> {
    let value = object
        .get(field)
        .ok_or_else(|| missing_field(kind, file_name, field))?;
    canonical_number(kind, file_name, field, value)
}

fn optional_number_field(
    object: &Map<String, Value>,
    field: &str,
) -> Result<Option<Value>, NormalizeError> {
    object
        .get(field)
        .map(|value| canonical_number(ResponseKind::Bars, "<bar>", field, value))
        .transpose()
}

fn canonical_number(
    kind: ResponseKind,
    file_name: &str,
    field: &str,
    value: &Value,
) -> Result<Value, NormalizeError> {
    match value {
        Value::Number(number) => Ok(Value::Number(number.clone())),
        Value::String(value) if !value.trim().is_empty() => Number::from_str(value)
            .map(Value::Number)
            .map_err(|_| NormalizeError::InvalidField {
                kind,
                file_name: file_name.to_owned(),
                field: field.to_owned(),
                value: value.to_owned(),
            }),
        _ => Err(invalid_field(kind, file_name, field, value)),
    }
}

fn first_string<'a>(object: &'a Map<String, Value>, fields: &[&str]) -> Option<&'a str> {
    fields
        .iter()
        .find_map(|field| object.get(*field).and_then(Value::as_str))
}

fn json_object<const N: usize>(fields: [(&str, Value); N]) -> Map<String, Value> {
    fields
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

fn malformed(kind: ResponseKind, file_name: &str, reason: impl Into<String>) -> NormalizeError {
    NormalizeError::Malformed {
        kind,
        file_name: file_name.to_owned(),
        reason: reason.into(),
    }
}

fn missing_field(kind: ResponseKind, file_name: &str, field: &str) -> NormalizeError {
    NormalizeError::MissingField {
        kind,
        file_name: file_name.to_owned(),
        field: field.to_owned(),
    }
}

fn invalid_field(
    kind: ResponseKind,
    file_name: &str,
    field: &str,
    value: &Value,
) -> NormalizeError {
    NormalizeError::InvalidField {
        kind,
        file_name: file_name.to_owned(),
        field: field.to_owned(),
        value: value.to_string(),
    }
}
