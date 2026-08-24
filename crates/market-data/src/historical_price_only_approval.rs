//! Shared, read-only approval boundary for the sealed historical price beta.

use std::{fmt, path::Path};

use domain::{ContentHash, TradingDate};
use serde::{Deserialize, Serialize};

use crate::{
    HistoricalPriceOnlyArtifactApprovalSummary, HistoricalPriceOnlyBar, KR_ETF_CORE_SYMBOLS,
    read_historical_price_only_artifact,
};

const REGISTRY_SCHEMA_ID: &str = "kis-historical-price-only-beta-approval-registry";
const REGISTRY_SCHEMA_VERSION: u32 = 2;
const MAX_REGISTRY_BYTES: usize = 256 * 1024;
const EMBEDDED_REGISTRY: &[u8] = include_bytes!(
    "../../../configs/evidence/kis-historical-price-only-beta-approved-artifacts.json"
);

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

/// Approves only the artifact selected by the exact registry compiled into the
/// current image. Callers cannot supply a candidate, registry, pin, or path
/// below the artifact root.
pub fn approve_historical_price_only_artifact(
    artifact_root: &Path,
) -> Result<ApprovedHistoricalPriceOnlyArtifact, HistoricalPriceOnlyApprovalError> {
    approve_with_registry(artifact_root, EMBEDDED_REGISTRY)
}

fn approve_with_registry(
    artifact_root: &Path,
    registry_bytes: &[u8],
) -> Result<ApprovedHistoricalPriceOnlyArtifact, HistoricalPriceOnlyApprovalError> {
    let registry = parse_registry(registry_bytes)?;
    let record = sole_record(&registry)?;
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
    if !fixed_envelope(summary) || !record_matches(record, summary, &pins) {
        return Err(HistoricalPriceOnlyApprovalError::ArtifactNotApproved);
    }
    Ok(ApprovedHistoricalPriceOnlyArtifact {
        pins,
        bars: verified.approved_bars().to_vec(),
    })
}

fn sole_record(registry: &Registry) -> Result<&Record, HistoricalPriceOnlyApprovalError> {
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

fn pins_match(record: &Record, pins: &HistoricalPriceOnlyArtifactPins) -> bool {
    record.candidate_content_sha256 == pins.candidate_content_sha256
        && record.artifact_manifest_sha256 == pins.artifact_manifest_sha256
        && record.stage5_manifest_sha256 == pins.stage5_manifest_sha256
        && record.action_manifest_sha256 == pins.action_manifest_sha256
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
