//! First historical continuation assembled in memory from retained evidence.
//! A candidate conveys neither durable storage nor policy/DB admission authority.

use std::collections::BTreeMap;

use domain::{CodeCommit, ContentHash, TradingDate};

use super::history::ReconstructedHistoricalExtension;
use super::verify::DailyVerificationError as Error;
use super::{
    DailyBarsDescriptorV1Wire, DailyBaseV1Wire, DailyEvidenceKindV1,
    DailyNormalizationDescriptorV1Wire, DailyNormalizationProofV1Wire,
    DailyNormalizedDocumentV1Wire, DailyPriceManifestDraftV1Wire, DailyPriceManifestV1Wire,
    daily_price_evidence_preimage_v1, encode_daily_normalization_proof_v1,
    encode_daily_price_manifest_v1, parse_daily_normalization_proof_v1,
    seal_daily_price_manifest_v1,
};

/// Private construction retains the full approved-base and source proof borrows.
/// No public constructor from wires/bars and no Deserialize or approval receipt.
pub struct HistoricalDailyCandidate<'candidate, 'proof, 'base, 'snapshot> {
    extension: &'candidate ReconstructedHistoricalExtension<'proof, 'base, 'snapshot>,
    manifest: DailyPriceManifestV1Wire,
    manifest_bytes: Vec<u8>,
    manifest_sha256: ContentHash,
    normalization: DailyNormalizationProofV1Wire,
    normalization_bytes: Vec<u8>,
}

impl HistoricalDailyCandidate<'_, '_, '_, '_> {
    pub fn extension(&self) -> &ReconstructedHistoricalExtension<'_, '_, '_> {
        self.extension
    }
    pub fn manifest(&self) -> &DailyPriceManifestV1Wire {
        &self.manifest
    }
    pub fn manifest_bytes(&self) -> &[u8] {
        &self.manifest_bytes
    }
    pub fn manifest_sha256(&self) -> &ContentHash {
        &self.manifest_sha256
    }
    pub fn content_sha256(&self) -> &ContentHash {
        &self.manifest.content_sha256
    }
    pub fn normalization(&self) -> &DailyNormalizationProofV1Wire {
        &self.normalization
    }
    pub fn normalization_bytes(&self) -> &[u8] {
        &self.normalization_bytes
    }
    pub fn bars_ndjson(&self) -> &[u8] {
        self.extension.bars_ndjson()
    }
}

/// One first-session candidate. The release commit is explicit lineage; neither
/// the clock, current Git HEAD nor a generated ID enters the deterministic bytes.
pub fn build_historical_daily_candidate<'candidate, 'proof, 'base, 'snapshot>(
    extension: &'candidate ReconstructedHistoricalExtension<'proof, 'base, 'snapshot>,
    producer_commit: CodeCommit,
) -> Result<HistoricalDailyCandidate<'candidate, 'proof, 'base, 'snapshot>, Error> {
    let prefix = extension.prefix();
    let sources = extension.sources();
    let session = sources.session();
    validate_documents(
        &session.normalized_documents,
        sources
            .documents()
            .iter()
            .map(|document| (document.descriptor(), document.bytes())),
    )?;
    let evidence = daily_price_evidence_preimage_v1(std::slice::from_ref(session))
        .map_err(|_| Error::InvalidEvidence)?;
    if evidence != sources.evidence_preimage() {
        return Err(Error::InvalidEvidence);
    }
    let proof = DailyNormalizationProofV1Wire {
        schema: "owner-beta-daily-etf11-normalization-v1".into(),
        normalizer_version: "daily-raw-reconstruction-v1".into(),
        evidence_sha256: ContentHash::from_bytes(&evidence),
        documents: session.normalized_documents.clone(),
    };
    let normalization_bytes =
        encode_daily_normalization_proof_v1(&proof).map_err(|_| Error::InvalidEvidence)?;
    let normalization = parse_daily_normalization_proof_v1(&normalization_bytes)
        .map_err(|_| Error::InvalidEvidence)?;
    let mut sessions = prefix.sessions().to_vec();
    sessions.push(session.session_date);
    let row_count = prefix
        .bars()
        .len()
        .checked_add(extension.added_bars().len())
        .and_then(|count| u64::try_from(count).ok())
        .ok_or(Error::LimitExceeded)?;
    let draft = DailyPriceManifestDraftV1Wire {
        schema: "owner-beta-daily-etf11-input-v1".into(),
        capability: "PRICE_RETURN_ONLY".into(),
        audience: "OWNER_ONLY".into(),
        vendor_snapshot: true,
        strict_pit: false,
        base: DailyBaseV1Wire {
            kind: "historical_price_only_v3".into(),
            pins: prefix.pins().clone(),
        },
        parent_admission_sha256: None,
        range_start: *prefix.sessions().first().ok_or(Error::InvalidContext)?,
        range_end: session.session_date,
        instruments: session.instruments.clone(),
        sessions,
        daily_sources: vec![session.clone()],
        bars: DailyBarsDescriptorV1Wire {
            file: "bars.ndjson".into(),
            sha256: extension.bars_sha256().clone(),
            size_bytes: extension.bars_ndjson().len() as u64,
            row_count,
        },
        normalization: DailyNormalizationDescriptorV1Wire {
            file: "normalization.json".into(),
            sha256: ContentHash::from_bytes(&normalization_bytes),
            size_bytes: normalization_bytes.len() as u64,
            row_count: normalization.documents.len() as u64,
        },
        policy_sha256: prefix.policy_sha256().clone(),
        materializer_version: "daily-price-only-v1".into(),
        code_commit: producer_commit,
    };
    let manifest = seal_daily_price_manifest_v1(&draft).map_err(|_| Error::InvalidEvidence)?;
    let manifest_bytes =
        encode_daily_price_manifest_v1(&manifest).map_err(|_| Error::InvalidEvidence)?;
    let manifest_sha256 = ContentHash::from_bytes(&manifest_bytes);
    Ok(HistoricalDailyCandidate {
        extension,
        manifest,
        manifest_bytes,
        manifest_sha256,
        normalization,
        normalization_bytes,
    })
}

fn document_key(document: &DailyNormalizedDocumentV1Wire) -> (TradingDate, DailyEvidenceKindV1) {
    (document.session_date, document.kind)
}

pub(super) fn validate_documents<'a>(
    expected: &[DailyNormalizedDocumentV1Wire],
    documents: impl Iterator<Item = (&'a DailyNormalizedDocumentV1Wire, &'a [u8])>,
) -> Result<(), Error> {
    let mut declarations = BTreeMap::new();
    for document in expected {
        if declarations
            .insert(document_key(document), document)
            .is_some()
        {
            return Err(Error::InvalidEvidence);
        }
    }
    if declarations.is_empty() {
        return Err(Error::InvalidEvidence);
    }
    let mut count = 0usize;
    for (descriptor, bytes) in documents {
        let declared = declarations
            .remove(&document_key(descriptor))
            .ok_or(Error::InvalidEvidence)?;
        if declared != descriptor
            || descriptor.size_bytes != bytes.len() as u64
            || descriptor.sha256 != ContentHash::from_bytes(bytes)
        {
            return Err(Error::InvalidEvidence);
        }
        count = count.checked_add(1).ok_or(Error::LimitExceeded)?;
    }
    if count != expected.len() || !declarations.is_empty() {
        return Err(Error::InvalidEvidence);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(bytes: &[u8]) -> DailyNormalizedDocumentV1Wire {
        DailyNormalizedDocumentV1Wire {
            kind: DailyEvidenceKindV1::Bars,
            session_date: TradingDate::parse("2026-08-31").unwrap(),
            normalizer_version: "daily-raw-reconstruction-v1".into(),
            sha256: ContentHash::from_bytes(bytes),
            size_bytes: bytes.len() as u64,
            row_count: 11,
            source_files: vec![],
            source_evidence_sha256: ContentHash::from_bytes(b"fixture"),
        }
    }

    #[test]
    fn s5b_candidate_document_binding_rejects_mutated_bytes_hash_or_length() {
        let bytes = b"fixed fixture document";
        let expected = document(bytes);
        assert_eq!(
            validate_documents(
                std::slice::from_ref(&expected),
                [(&expected, bytes.as_slice())].into_iter()
            ),
            Ok(())
        );
        assert_eq!(
            validate_documents(
                std::slice::from_ref(&expected),
                [(&expected, b"changed fixture document".as_slice())].into_iter()
            ),
            Err(Error::InvalidEvidence)
        );
        for field in 0..2 {
            let mut changed = expected.clone();
            if field == 0 {
                changed.sha256 = ContentHash::from_bytes(b"other");
            } else {
                changed.size_bytes += 1;
            }
            assert_eq!(
                validate_documents(
                    std::slice::from_ref(&changed),
                    [(&changed, bytes.as_slice())].into_iter()
                ),
                Err(Error::InvalidEvidence)
            );
        }
    }

    #[test]
    fn s5b_candidate_document_binding_rejects_omission_extra_and_duplicate_keys() {
        let bytes = b"document";
        let expected = document(bytes);
        assert_eq!(
            validate_documents(std::slice::from_ref(&expected), std::iter::empty()),
            Err(Error::InvalidEvidence)
        );
        assert_eq!(
            validate_documents(&[], [(&expected, bytes.as_slice())].into_iter()),
            Err(Error::InvalidEvidence)
        );
        assert_eq!(
            validate_documents(
                &[expected.clone(), expected.clone()],
                [(&expected, bytes.as_slice())].into_iter()
            ),
            Err(Error::InvalidEvidence)
        );
        assert_eq!(
            validate_documents(
                std::slice::from_ref(&expected),
                [(&expected, bytes.as_slice()), (&expected, bytes.as_slice())].into_iter()
            ),
            Err(Error::InvalidEvidence)
        );
    }

    #[test]
    fn s5b_candidate_document_binding_includes_every_descriptor_field() {
        let bytes = b"document";
        let expected = document(bytes);
        for field in 0..5 {
            let mut changed = expected.clone();
            match field {
                0 => changed.kind = DailyEvidenceKindV1::Reference,
                1 => changed.session_date = TradingDate::parse("2026-09-01").unwrap(),
                2 => changed.row_count = 12,
                3 => changed.normalizer_version = "other".into(),
                _ => changed.source_evidence_sha256 = ContentHash::from_bytes(b"other"),
            };
            assert_eq!(
                validate_documents(
                    std::slice::from_ref(&expected),
                    [(&changed, bytes.as_slice())].into_iter()
                ),
                Err(Error::InvalidEvidence)
            );
        }
    }

    #[test]
    fn s5b_candidate_document_binding_is_independent_of_iteration_order() {
        let first = document(b"first");
        let mut second = document(b"second");
        second.kind = DailyEvidenceKindV1::Reference;
        let expected = [first.clone(), second.clone()];
        assert_eq!(
            validate_documents(
                &expected,
                [
                    (&second, b"second".as_slice()),
                    (&first, b"first".as_slice())
                ]
                .into_iter()
            ),
            Ok(())
        );
    }
}
