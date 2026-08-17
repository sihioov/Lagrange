//! Raw-to-PostgreSQL bridge for one coherent candidate-source delivery.
//!
//! Credential handling stays in the provider adapter. This module starts from
//! immutable Raw read-back evidence, reconstructs typed envelopes, validates
//! the candidate documents present in that immutable delivery, and publishes
//! them in one DB transaction. Daily flow/status may therefore be refreshed
//! without fabricating new revisions for unchanged PIT reference sources.

use std::collections::{BTreeMap, BTreeSet};

use domain::{BatchId, TradingDate, UtcTimestamp};
use market_data::{
    CANDIDATE_RESPONSE_KINDS, CandidateDataError, CandidateDocument, CandidateSourcePin, FetchMode,
    FundamentalDocument, IndexMembershipDocument, IngestError, IngestOutcome, InvestorFlowDocument,
    MARKET_KR, MarketStatusDocument, PROVIDER_KRX, RawEnvelope, RawStore, ResponseKind,
    SectorDocument, parse_candidate_envelope, validate_candidate_document,
};
use uuid::Uuid;

use crate::{CandidateSourcePublication, PostgresCandidateSourceSink, PublishOutcome, SinkError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateDatasetBinding {
    pub kind: ResponseKind,
    pub dataset_version_id: Uuid,
    pub entitlement_id: Uuid,
    pub license_ref: String,
    pub dataset_id: String,
    pub dataset_version: String,
    pub manifest_sha256: String,
    pub reused_existing: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreparedCandidateSource {
    pub dataset_version_id: Uuid,
    pub pin: CandidateSourcePin,
    pub document: CandidateDocument,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreparedCandidateBatch {
    pub batch_id: BatchId,
    pub raw_manifest_sha256: String,
    pub fetch_mode: FetchMode,
    pub as_of: TradingDate,
    pub cutoff_at: UtcTimestamp,
    pub sources: Vec<PreparedCandidateSource>,
}

#[derive(Debug, thiserror::Error)]
pub enum CandidatePipelineError {
    #[error("candidate raw batch contract is invalid: {0}")]
    InvalidRaw(String),
    #[error(transparent)]
    Ingest(#[from] IngestError),
    #[error(transparent)]
    InvalidDocument(#[from] CandidateDataError),
    #[error(transparent)]
    Publish(#[from] SinkError),
}

/// Prepare exactly the candidate source classes requested in this immutable
/// Raw delivery. The entitlement reference is mandatory because every
/// displayed source must remain attributable to its licensed contract.
pub fn prepare_candidate_batch(
    outcome: &IngestOutcome,
    as_of: TradingDate,
    cutoff_at: UtcTimestamp,
    bindings: &[CandidateDatasetBinding],
) -> Result<PreparedCandidateBatch, CandidatePipelineError> {
    let allowed = BTreeSet::from(CANDIDATE_RESPONSE_KINDS);
    let mut by_kind = BTreeMap::new();
    for binding in bindings {
        if !allowed.contains(&binding.kind) || by_kind.insert(binding.kind, binding).is_some() {
            return Err(CandidatePipelineError::InvalidRaw(
                "dataset bindings must contain each requested candidate response class exactly once"
                    .to_owned(),
            ));
        }
    }
    let expected = by_kind.keys().copied().collect::<BTreeSet<_>>();
    if expected.is_empty() {
        return Err(CandidatePipelineError::InvalidRaw(
            "candidate delivery must contain at least one requested source class".to_owned(),
        ));
    }
    let raw_license_ref = outcome
        .entry
        .entitlement_reference
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            CandidatePipelineError::InvalidRaw(
                "candidate raw batch requires an entitlement reference".to_owned(),
            )
        })?;
    if outcome.entry.batch_id != outcome.batch_id {
        return Err(CandidatePipelineError::InvalidRaw(
            "raw outcome batch identity is inconsistent".to_owned(),
        ));
    }

    let mut stored_by_name = BTreeMap::new();
    for stored in &outcome.files {
        if stored_by_name
            .insert(stored.file_name.as_str(), stored)
            .is_some()
        {
            return Err(CandidatePipelineError::InvalidRaw(
                "raw read-back contains a duplicate file name".to_owned(),
            ));
        }
    }
    if outcome.entry.files.len() < expected.len()
        || stored_by_name.len() != outcome.entry.files.len()
    {
        return Err(CandidatePipelineError::InvalidRaw(
            "candidate raw batch must contain every source class and matching read-back files"
                .to_owned(),
        ));
    }

    let mut observed = BTreeSet::new();
    let mut pages = BTreeMap::<ResponseKind, Vec<(String, CandidateDocument)>>::new();
    for file in &outcome.entry.files {
        if !expected.contains(&file.kind) {
            return Err(CandidatePipelineError::InvalidRaw(
                "candidate raw batch has an unexpected response class".to_owned(),
            ));
        }
        observed.insert(file.kind);
        let stored = stored_by_name.get(file.file_name.as_str()).ok_or_else(|| {
            CandidatePipelineError::InvalidRaw(
                "candidate manifest file is missing from verified read-back".to_owned(),
            )
        })?;
        let envelope = RawEnvelope::new(
            outcome.batch_id,
            file.kind,
            file.file_name.clone(),
            stored.bytes.clone(),
            outcome.entry.retrieved_at,
            file.request.clone(),
        );
        if envelope.content_hash != file.content_hash {
            return Err(CandidatePipelineError::InvalidRaw(
                "candidate read-back hash differs from immutable manifest".to_owned(),
            ));
        }
        let document = parse_candidate_envelope(&envelope)?;
        pages
            .entry(file.kind)
            .or_default()
            .push((file.file_name.clone(), document));
    }
    if observed != expected {
        return Err(CandidatePipelineError::InvalidRaw(
            "candidate raw batch is missing a response class".to_owned(),
        ));
    }

    let mut sources = Vec::with_capacity(expected.len());
    for kind in expected {
        let mut kind_pages = pages
            .remove(&kind)
            .expect("all source classes were observed");
        kind_pages.sort_by(|left, right| left.0.cmp(&right.0));
        let document = merge_candidate_pages(
            kind,
            kind_pages.into_iter().map(|(_, document)| document),
            outcome.entry.retrieved_at,
        )?;
        let binding = by_kind[&kind];
        if binding.entitlement_id.is_nil()
            || binding.license_ref.trim().is_empty()
            || binding.license_ref != raw_license_ref
        {
            return Err(CandidatePipelineError::InvalidRaw(
                "candidate source binding must match the exact Raw entitlement".to_owned(),
            ));
        }
        let expected_dataset = dataset_for(kind).expect("candidate kind has a dataset");
        if binding.dataset_id != expected_dataset {
            return Err(CandidatePipelineError::InvalidRaw(format!(
                "{} must bind dataset {expected_dataset}",
                kind
            )));
        }
        let pin = CandidateSourcePin {
            provider: outcome.entry.provider.clone(),
            entitlement_id: binding.entitlement_id,
            license_ref: binding.license_ref.clone(),
            dataset_id: binding.dataset_id.clone(),
            dataset_version: binding.dataset_version.clone(),
            manifest_sha256: binding.manifest_sha256.clone(),
            retrieved_at: outcome.entry.retrieved_at,
        };
        pin.validate()?;
        if !binding.reused_existing {
            sources.push(PreparedCandidateSource {
                dataset_version_id: binding.dataset_version_id,
                pin,
                document,
            });
        }
    }
    sources.sort_by_key(|source| source_dataset(&source.document));
    Ok(PreparedCandidateBatch {
        batch_id: outcome.batch_id,
        raw_manifest_sha256: raw_manifest_sha256(&outcome.entry)?,
        fetch_mode: outcome.entry.mode,
        as_of,
        cutoff_at,
        sources,
    })
}

fn merge_candidate_pages(
    kind: ResponseKind,
    pages: impl IntoIterator<Item = CandidateDocument>,
    retrieved_at: UtcTimestamp,
) -> Result<CandidateDocument, CandidatePipelineError> {
    let mut merged = match kind {
        ResponseKind::InvestorFlow => {
            CandidateDocument::InvestorFlow(InvestorFlowDocument { flows: Vec::new() })
        }
        ResponseKind::MarketStatus => CandidateDocument::MarketStatus(MarketStatusDocument {
            statuses: Vec::new(),
        }),
        ResponseKind::Fundamentals => CandidateDocument::Fundamentals(FundamentalDocument {
            fundamentals: Vec::new(),
        }),
        ResponseKind::IndexMembership => {
            CandidateDocument::IndexMembership(IndexMembershipDocument {
                memberships: Vec::new(),
            })
        }
        ResponseKind::SectorClassification => {
            CandidateDocument::SectorClassification(SectorDocument {
                sectors: Vec::new(),
            })
        }
        _ => {
            return Err(CandidatePipelineError::InvalidRaw(
                "candidate pagination includes an unsupported response class".to_owned(),
            ));
        }
    };
    for page in pages {
        match (&mut merged, page) {
            (CandidateDocument::InvestorFlow(target), CandidateDocument::InvestorFlow(page)) => {
                target.flows.extend(page.flows);
            }
            (CandidateDocument::MarketStatus(target), CandidateDocument::MarketStatus(page)) => {
                target.statuses.extend(page.statuses);
            }
            (CandidateDocument::Fundamentals(target), CandidateDocument::Fundamentals(page)) => {
                target.fundamentals.extend(page.fundamentals);
            }
            (
                CandidateDocument::IndexMembership(target),
                CandidateDocument::IndexMembership(page),
            ) => target.memberships.extend(page.memberships),
            (
                CandidateDocument::SectorClassification(target),
                CandidateDocument::SectorClassification(page),
            ) => target.sectors.extend(page.sectors),
            _ => {
                return Err(CandidatePipelineError::InvalidRaw(
                    "candidate page kind does not match its manifest class".to_owned(),
                ));
            }
        }
    }
    validate_candidate_document(&merged, retrieved_at)?;
    Ok(merged)
}

/// Atomically publish every prepared source. A database failure leaves the Raw
/// batch intact; callers may rebuild the same prepared batch and retry safely.
pub async fn publish_candidate_batch(
    sink: &PostgresCandidateSourceSink,
    batch: &PreparedCandidateBatch,
) -> Result<PublishOutcome, CandidatePipelineError> {
    let publications = batch
        .sources
        .iter()
        .map(|source| CandidateSourcePublication {
            raw_batch_id: batch.batch_id.as_uuid(),
            raw_manifest_sha256: &batch.raw_manifest_sha256,
            fetch_mode: batch.fetch_mode,
            dataset_version_id: source.dataset_version_id,
            as_of: batch.as_of,
            cutoff_at: batch.cutoff_at,
            pin: &source.pin,
            document: &source.document,
        })
        .collect::<Vec<_>>();
    sink.publish_batch(&publications)
        .await
        .map_err(CandidatePipelineError::from)
}

fn raw_manifest_sha256(
    entry: &market_data::ManifestEntry,
) -> Result<String, CandidatePipelineError> {
    let canonical = serde_json::to_vec(entry).map_err(|error| {
        CandidatePipelineError::InvalidRaw(format!(
            "candidate Raw manifest serialization failed: {error}"
        ))
    })?;
    Ok(domain::ContentHash::from_bytes(&canonical)
        .as_str()
        .strip_prefix("sha256:")
        .expect("content hashes have a sha256 prefix")
        .to_owned())
}

/// Replay every immutable candidate Raw batch through the same typed and DB
/// gates. Dataset discovery is cutoff-bounded and the original contract
/// reference is required, so recovery cannot silently adopt newer lineage or
/// different rights. Exact DB replay is idempotent.
pub async fn recover_candidate_batches(
    store: &RawStore,
    sink: &PostgresCandidateSourceSink,
) -> Result<usize, CandidatePipelineError> {
    let manifest = store
        .read_manifest(PROVIDER_KRX, MARKET_KR)
        .map_err(|error| CandidatePipelineError::Ingest(IngestError::Store(error)))?;
    let mut published = 0;
    for entry in manifest {
        if sink.raw_batch_is_terminal(&entry, "source").await? {
            continue;
        }
        entry
            .entitlement_reference
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                CandidatePipelineError::InvalidRaw(
                    "candidate recovery requires the original Raw entitlement".to_owned(),
                )
            })?;
        let files = store
            .read_batch_bytes(PROVIDER_KRX, MARKET_KR, &entry)
            .map_err(|source| {
                CandidatePipelineError::Ingest(IngestError::Readback {
                    entry: Box::new(entry.clone()),
                    source,
                })
            })?;
        let outcome = IngestOutcome {
            batch_id: entry.batch_id,
            entry: entry.clone(),
            files,
        };
        let (rights_first_date, rights_last_date) =
            crate::candidate_sink::candidate_source_rights_window(&outcome)?;
        let bindings = match sink.catalog_candidate_batch(&outcome).await {
            Ok(bindings) => bindings,
            Err(original) => {
                if sink
                    .block_raw_batch_for_inactive_rights(
                        &entry,
                        "source",
                        rights_first_date,
                        rights_last_date,
                    )
                    .await
                    .is_ok()
                {
                    continue;
                }
                return Err(CandidatePipelineError::Publish(original));
            }
        };
        let batch = prepare_candidate_batch(&outcome, entry.date, entry.retrieved_at, &bindings)?;
        if publish_candidate_batch(sink, &batch).await? == PublishOutcome::Published {
            published += 1;
        }
    }
    Ok(published)
}

fn dataset_for(kind: ResponseKind) -> Option<&'static str> {
    match kind {
        ResponseKind::InvestorFlow => Some("krx_investor_flows"),
        ResponseKind::MarketStatus => Some("krx_market_status"),
        ResponseKind::Fundamentals => Some("krx_fundamentals"),
        ResponseKind::IndexMembership => Some("krx_kospi200_membership"),
        ResponseKind::SectorClassification => Some("krx_sector_classification"),
        ResponseKind::Bars
        | ResponseKind::Reference
        | ResponseKind::Calendar
        | ResponseKind::CorporateActions => None,
    }
}

fn source_dataset(document: &CandidateDocument) -> &'static str {
    match document {
        CandidateDocument::InvestorFlow(_) => "krx_investor_flows",
        CandidateDocument::MarketStatus(_) => "krx_market_status",
        CandidateDocument::Fundamentals(_) => "krx_fundamentals",
        CandidateDocument::IndexMembership(_) => "krx_kospi200_membership",
        CandidateDocument::SectorClassification(_) => "krx_sector_classification",
    }
}

#[cfg(test)]
mod tests {
    use market_data::{
        IngestRequest, KrxProvider, MARKET_KR, RawStore, RecordedBundle, ingest_bundle_with_kinds,
    };

    use super::*;

    fn date(value: &str) -> TradingDate {
        TradingDate::parse(value).expect("valid date")
    }

    fn timestamp(value: &str) -> UtcTimestamp {
        UtcTimestamp::parse_rfc3339(value).expect("valid timestamp")
    }

    fn bindings() -> Vec<CandidateDatasetBinding> {
        CANDIDATE_RESPONSE_KINDS
            .into_iter()
            .enumerate()
            .map(|(index, kind)| CandidateDatasetBinding {
                kind,
                dataset_version_id: Uuid::from_u128((index + 1) as u128),
                entitlement_id: Uuid::from_u128(100),
                license_ref: "fixture://candidate-license".to_owned(),
                dataset_id: dataset_for(kind).expect("candidate dataset").to_owned(),
                dataset_version: "synthetic-v1".to_owned(),
                manifest_sha256: format!("{:x}", index + 10).repeat(64),
                reused_existing: false,
            })
            .collect()
    }

    fn outcome() -> IngestOutcome {
        let root = tempfile::tempdir().expect("raw root");
        let store = RawStore::new(root.path());
        let provider = KrxProvider::synthetic(
            RecordedBundle::open(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/kr-candidates/contract"
            ))
            .expect("candidate bundle"),
        );
        let request = IngestRequest::new(
            MARKET_KR.to_owned(),
            date("2026-08-14"),
            timestamp("2026-08-14T07:00:00Z"),
        );
        ingest_bundle_with_kinds(
            &store,
            &provider,
            &request,
            Some("fixture://candidate-license"),
            &CANDIDATE_RESPONSE_KINDS,
        )
        .expect("immutable candidate ingestion")
    }

    #[test]
    fn raw_readback_prepares_one_exact_typed_candidate_batch() {
        let prepared = prepare_candidate_batch(
            &outcome(),
            date("2026-08-14"),
            timestamp("2026-08-14T06:55:00Z"),
            &bindings(),
        )
        .expect("prepare candidate batch");
        assert_eq!(prepared.sources.len(), CANDIDATE_RESPONSE_KINDS.len());
        assert_eq!(
            prepared
                .sources
                .iter()
                .map(|source| source.pin.dataset_id.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "krx_fundamentals",
                "krx_investor_flows",
                "krx_kospi200_membership",
                "krx_market_status",
                "krx_sector_classification",
            ])
        );
        assert!(
            prepared
                .sources
                .iter()
                .all(|source| source.pin.license_ref == "fixture://candidate-license")
        );
    }

    #[test]
    fn raw_readback_fails_closed_on_binding_license_or_hash_drift() {
        let mut missing = bindings();
        missing.pop();
        assert!(matches!(
            prepare_candidate_batch(
                &outcome(),
                date("2026-08-14"),
                timestamp("2026-08-14T06:55:00Z"),
                &missing,
            ),
            Err(CandidatePipelineError::InvalidRaw(_))
        ));

        let mut unlicensed = outcome();
        unlicensed.entry.entitlement_reference = None;
        assert!(matches!(
            prepare_candidate_batch(
                &unlicensed,
                date("2026-08-14"),
                timestamp("2026-08-14T06:55:00Z"),
                &bindings(),
            ),
            Err(CandidatePipelineError::InvalidRaw(_))
        ));

        let mut tampered = outcome();
        tampered.files[0].bytes.push(b' ');
        assert!(matches!(
            prepare_candidate_batch(
                &tampered,
                date("2026-08-14"),
                timestamp("2026-08-14T06:55:00Z"),
                &bindings(),
            ),
            Err(CandidatePipelineError::InvalidRaw(_))
        ));
    }

    #[test]
    fn paginated_source_pages_merge_in_file_order_and_revalidate_natural_keys() {
        let page = |name: &str, instrument: &str| {
            let envelope = RawEnvelope::new(
                BatchId::generate(),
                ResponseKind::InvestorFlow,
                name,
                format!(
                    "{{\"flows\":[{{\"instrument\":\"{instrument}\",\"trade_date\":\"2026-08-14\",\"investor_class\":\"FOREIGN\",\"net_amount\":1,\"net_volume\":2,\"currency\":\"KRW\",\"volume_unit\":\"SHARE\",\"source_revision\":\"r1\",\"available_at\":\"2026-08-14T07:00:00Z\"}}]}}"
                )
                .into_bytes(),
                timestamp("2026-08-14T07:00:00Z"),
                market_data::RequestMetadata {
                    endpoint: "fixture".to_owned(),
                    query: Vec::new(),
                    headers: Vec::new(),
                    mode: market_data::FetchMode::Synthetic,
                },
            );
            parse_candidate_envelope(&envelope).expect("valid page")
        };
        let first = page("page-1.json", "005930.KRX");
        let second = page("page-2.json", "000660.KRX");
        let merged = merge_candidate_pages(
            ResponseKind::InvestorFlow,
            [first.clone(), second],
            timestamp("2026-08-14T07:00:00Z"),
        )
        .expect("distinct pages merge");
        let CandidateDocument::InvestorFlow(merged) = merged else {
            panic!("expected investor-flow document");
        };
        assert_eq!(merged.flows.len(), 2);

        let duplicate = merge_candidate_pages(
            ResponseKind::InvestorFlow,
            [first.clone(), first],
            timestamp("2026-08-14T07:00:00Z"),
        );
        assert!(matches!(
            duplicate,
            Err(CandidatePipelineError::InvalidDocument(_))
        ));
    }
}
