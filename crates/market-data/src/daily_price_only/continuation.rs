//! Protected daily-parent continuation, separate from current DB authorization.
//! A preceding input is an opaque verified file capability, never caller bars.

use domain::{ContentHash, TradingDate};

use super::admission::{
    DailyAdmissionVerificationError as Error, PinnedDailyPolicy, SealedHistoricalDailyInput,
};
use super::artifact::{DailyAdmissionRoot, OpenedDailyCandidate, read_admission_files};
use super::contract::fixed_limits;
use super::history::{
    ProtectedHistoricalPrefix, checked_addition, compare_preserved_rows, decode_stored_bars,
    prefix_index,
};
use super::{
    DailyPriceAdmissionV1Wire, DailyPriceManifestV1Wire, DailyPricePinsV1Wire,
    encode_daily_price_admission_v1, parse_daily_price_admission_v1,
    validate_daily_price_wire_bundle_v1,
};
use crate::{ApprovedHistoricalPriceOnlyArtifact, HistoricalPriceOnlyBar};

/// Both variants require verified files and an actual approved historical base.
/// This view neither owns nor duplicates file descriptors.
pub enum SealedDailyParent<'parent, 'base> {
    First(&'parent SealedHistoricalDailyInput<'base>),
    Continued(&'parent SealedDailyContinuation<'base>),
}
impl SealedDailyParent<'_, '_> {
    pub fn manifest(&self) -> &DailyPriceManifestV1Wire {
        match self {
            Self::First(value) => value.manifest(),
            Self::Continued(value) => value.manifest(),
        }
    }
    pub fn pins(&self) -> &DailyPricePinsV1Wire {
        match self {
            Self::First(value) => value.pins(),
            Self::Continued(value) => value.pins(),
        }
    }
    pub fn bars(&self) -> &[HistoricalPriceOnlyBar] {
        match self {
            Self::First(value) => value.bars(),
            Self::Continued(value) => value.bars(),
        }
    }
    pub(super) fn opened(&self) -> &OpenedDailyCandidate {
        match self {
            Self::First(value) => value.opened(),
            Self::Continued(value) => &value.opened,
        }
    }
    pub(super) fn reborrow(&self) -> SealedDailyParent<'_, '_> {
        match self {
            Self::First(value) => SealedDailyParent::First(value),
            Self::Continued(value) => SealedDailyParent::Continued(value),
        }
    }
    pub fn revalidate(&self) -> Result<(), Error> {
        match self {
            Self::First(value) => value.revalidate(),
            Self::Continued(value) => value.revalidate(),
        }
    }
}

/// A single reopened cumulative leaf checked against its opaque predecessor.
/// It retains only its own file descriptors, allowing a caller to advance a
/// bounded chain walk without retaining every ancestor's cumulative buffers.
/// A DB ancestry/revocation gate remains necessary for any current use.
pub struct SealedDailyContinuation<'base> {
    base: &'base ApprovedHistoricalPriceOnlyArtifact,
    policy: &'base PinnedDailyPolicy,
    opened: OpenedDailyCandidate,
    admission: DailyPriceAdmissionV1Wire,
    admission_bytes: Vec<u8>,
    pins: DailyPricePinsV1Wire,
    bars: Vec<HistoricalPriceOnlyBar>,
}
impl SealedDailyContinuation<'_> {
    pub fn base(&self) -> &ApprovedHistoricalPriceOnlyArtifact {
        self.base
    }
    pub fn policy(&self) -> &PinnedDailyPolicy {
        self.policy
    }
    pub fn manifest(&self) -> &DailyPriceManifestV1Wire {
        self.opened.manifest()
    }
    pub fn pins(&self) -> &DailyPricePinsV1Wire {
        &self.pins
    }
    pub fn admission(&self) -> &DailyPriceAdmissionV1Wire {
        &self.admission
    }
    pub fn admission_bytes(&self) -> &[u8] {
        &self.admission_bytes
    }
    pub fn bars(&self) -> &[HistoricalPriceOnlyBar] {
        &self.bars
    }
    pub fn sessions(&self) -> &[TradingDate] {
        &self.manifest().sessions
    }
    pub fn range_end(&self) -> TradingDate {
        self.manifest().range_end
    }
    pub fn revalidate(&self) -> Result<(), Error> {
        self.opened.revalidate().map_err(Error::Artifact)
    }
}

/// Reads one exact child after its predecessor has been reopened. This function
/// does not search a latest pointer, recurse through attacker-chosen paths or
/// retrieve DB pins. A bounded orchestrator supplies the explicit next pin set.
pub fn read_sealed_daily_continuation<'base>(
    root: &DailyAdmissionRoot,
    expected: &DailyPricePinsV1Wire,
    base: &'base ApprovedHistoricalPriceOnlyArtifact,
    policy: &'base PinnedDailyPolicy,
    parent: &SealedDailyParent<'_, '_>,
) -> Result<SealedDailyContinuation<'base>, Error> {
    parent.revalidate()?;
    let prefix = ProtectedHistoricalPrefix::from_approved_v3(base, policy.wire())
        .map_err(Error::Evidence)?;
    if parent.manifest().base.pins != *prefix.pins()
        || parent.pins().policy_sha256 != *policy.sha256()
    {
        return Err(Error::InvalidCandidate);
    }
    let (opened, admission_bytes) =
        read_admission_files(root, &expected.admission_sha256, &expected.content_sha256)
            .map_err(Error::Artifact)?;
    let admission =
        parse_daily_price_admission_v1(&admission_bytes).map_err(|_| Error::InvalidCandidate)?;
    if encode_daily_price_admission_v1(&admission).map_err(|_| Error::InvalidCandidate)?
        != admission_bytes
    {
        return Err(Error::InvalidCandidate);
    }
    validate_daily_price_wire_bundle_v1(
        policy.wire(),
        opened.manifest(),
        opened.normalization(),
        &admission,
        expected,
    )
    .map_err(|_| Error::InvalidCandidate)?;
    validate_transition(parent.manifest(), parent.pins(), opened.manifest())?;
    let bars = read_child_bars(parent.bars(), opened.manifest(), opened.bars_ndjson())?;
    parent.revalidate()?;
    opened.revalidate().map_err(Error::Artifact)?;
    Ok(SealedDailyContinuation {
        base,
        policy,
        opened,
        admission,
        admission_bytes,
        pins: expected.clone(),
        bars,
    })
}

fn validate_transition(
    previous: &DailyPriceManifestV1Wire,
    pins: &DailyPricePinsV1Wire,
    next: &DailyPriceManifestV1Wire,
) -> Result<(), Error> {
    let sessions = previous
        .sessions
        .len()
        .checked_add(1)
        .ok_or(Error::LimitExceeded)?;
    let sources = previous
        .daily_sources
        .len()
        .checked_add(1)
        .ok_or(Error::LimitExceeded)?;
    if sessions as u64 > fixed_limits().max_sessions {
        return Err(Error::LimitExceeded);
    }
    if previous.content_sha256 != pins.content_sha256
        || previous.policy_sha256 != pins.policy_sha256
        || next.parent_admission_sha256.as_ref() != Some(&pins.admission_sha256)
        || next.base != previous.base
        || next.policy_sha256 != previous.policy_sha256
        || next.instruments != previous.instruments
        || next.range_start != previous.range_start
        || next.range_end <= previous.range_end
        || next.sessions.len() != sessions
        || !next.sessions.starts_with(&previous.sessions)
        || next.sessions.last() != Some(&next.range_end)
        || next.daily_sources.len() != sources
        || !next.daily_sources.starts_with(&previous.daily_sources)
    {
        return Err(Error::InvalidCandidate);
    }
    let added = next.daily_sources.last().ok_or(Error::InvalidCandidate)?;
    if added.previous_frontier != previous.range_end || added.session_date != next.range_end {
        return Err(Error::InvalidCandidate);
    }
    Ok(())
}

fn read_child_bars(
    previous: &[HistoricalPriceOnlyBar],
    manifest: &DailyPriceManifestV1Wire,
    bytes: &[u8],
) -> Result<Vec<HistoricalPriceOnlyBar>, Error> {
    if manifest.bars.size_bytes != bytes.len() as u64
        || manifest.bars.sha256 != ContentHash::from_bytes(bytes)
    {
        return Err(Error::InvalidCandidate);
    }
    let maximum = manifest
        .sessions
        .len()
        .checked_mul(11)
        .ok_or(Error::LimitExceeded)?;
    let bars = decode_stored_bars(bytes, maximum).map_err(Error::Evidence)?;
    if bars.len() != maximum || manifest.bars.row_count != maximum as u64 {
        return Err(Error::InvalidCandidate);
    }
    let index = prefix_index(
        &bars,
        &manifest.sessions,
        manifest.range_start,
        manifest.range_end,
    )
    .map_err(Error::Evidence)?;
    compare_preserved_rows(previous, &bars, &index).map_err(Error::Evidence)?;
    checked_addition(
        bars.iter()
            .filter(|bar| bar.session_date == manifest.range_end)
            .cloned()
            .collect(),
        manifest.range_end,
    )
    .map_err(Error::Evidence)?;
    Ok(bars)
}

/// Unapproved cumulative candidate from an opaque daily parent and a fresh source
/// discovery. Sealed source-frontier comparison remains a separate prerequisite
/// of admission; this builder grants no write/admission or current DB authority.
pub struct DailyContinuationCandidate<'parent, 'base, 'snapshot> {
    parent: SealedDailyParent<'parent, 'base>,
    sources: &'parent super::discovery::DiscoveredSessionSources<'snapshot>,
    prices: super::price_extension::ReconstructedPrices,
    manifest: DailyPriceManifestV1Wire,
    manifest_bytes: Vec<u8>,
    manifest_sha256: ContentHash,
    normalization: super::DailyNormalizationProofV1Wire,
    normalization_bytes: Vec<u8>,
}
impl DailyContinuationCandidate<'_, '_, '_> {
    pub fn parent(&self) -> &SealedDailyParent<'_, '_> {
        &self.parent
    }
    pub fn sources(&self) -> &super::discovery::DiscoveredSessionSources<'_> {
        self.sources
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
    pub fn normalization(&self) -> &super::DailyNormalizationProofV1Wire {
        &self.normalization
    }
    pub fn normalization_bytes(&self) -> &[u8] {
        &self.normalization_bytes
    }
    pub fn bars_ndjson(&self) -> &[u8] {
        &self.prices.bars_ndjson
    }
    pub fn inspected_rows(&self) -> usize {
        self.prices.inspected_rows
    }
    pub fn compared_rows(&self) -> usize {
        self.prices.compared_rows
    }
}

pub fn build_daily_continuation_candidate<'parent, 'base, 'snapshot>(
    parent: SealedDailyParent<'parent, 'base>,
    sources: &'parent super::discovery::DiscoveredSessionSources<'snapshot>,
    snapshots: &super::verify::RawEvidenceSnapshots<'snapshot>,
    producer_commit: domain::CodeCommit,
) -> Result<DailyContinuationCandidate<'parent, 'base, 'snapshot>, Error> {
    use super::price_extension::{PricePrefixView, reconstruct_prefix_prices};
    use super::{
        DailyBarsDescriptorV1Wire, DailyNormalizationDescriptorV1Wire,
        DailyNormalizationProofV1Wire, DailyPriceManifestDraftV1Wire,
        daily_price_evidence_preimage_v1, encode_daily_normalization_proof_v1,
        encode_daily_price_manifest_v1, parse_daily_normalization_proof_v1,
        seal_daily_price_manifest_v1,
    };
    parent.revalidate()?;
    let previous = parent.manifest();
    let prices = reconstruct_prefix_prices(
        PricePrefixView {
            bars: parent.bars(),
            sessions: &previous.sessions,
            policy_sha256: &previous.policy_sha256,
        },
        sources,
        snapshots,
    )
    .map_err(Error::Evidence)?;
    let added = sources.session();
    super::candidate::validate_documents(
        &added.normalized_documents,
        sources
            .documents()
            .iter()
            .map(|d| (d.descriptor(), d.bytes())),
    )
    .map_err(Error::Evidence)?;
    if daily_price_evidence_preimage_v1(std::slice::from_ref(added))
        .map_err(|_| Error::InvalidCandidate)?
        != sources.evidence_preimage()
    {
        return Err(Error::InvalidCandidate);
    }
    let mut daily_sources = previous.daily_sources.clone();
    daily_sources.push(added.clone());
    let evidence =
        daily_price_evidence_preimage_v1(&daily_sources).map_err(|_| Error::InvalidCandidate)?;
    let proof = DailyNormalizationProofV1Wire {
        schema: "owner-beta-daily-etf11-normalization-v1".into(),
        normalizer_version: "daily-raw-reconstruction-v1".into(),
        evidence_sha256: ContentHash::from_bytes(&evidence),
        documents: daily_sources
            .iter()
            .flat_map(|s| s.normalized_documents.iter().cloned())
            .collect(),
    };
    let normalization_bytes =
        encode_daily_normalization_proof_v1(&proof).map_err(|_| Error::InvalidCandidate)?;
    let normalization = parse_daily_normalization_proof_v1(&normalization_bytes)
        .map_err(|_| Error::InvalidCandidate)?;
    let mut sessions = previous.sessions.clone();
    sessions.push(added.session_date);
    let rows = parent
        .bars()
        .len()
        .checked_add(prices.added_bars.len())
        .ok_or(Error::LimitExceeded)?;
    let draft = DailyPriceManifestDraftV1Wire {
        schema: "owner-beta-daily-etf11-input-v1".into(),
        capability: "PRICE_RETURN_ONLY".into(),
        audience: "OWNER_ONLY".into(),
        vendor_snapshot: true,
        strict_pit: false,
        base: previous.base.clone(),
        parent_admission_sha256: Some(parent.pins().admission_sha256.clone()),
        range_start: previous.range_start,
        range_end: added.session_date,
        instruments: previous.instruments.clone(),
        sessions,
        daily_sources,
        bars: DailyBarsDescriptorV1Wire {
            file: "bars.ndjson".into(),
            sha256: prices.bars_sha256.clone(),
            size_bytes: prices.bars_ndjson.len() as u64,
            row_count: rows as u64,
        },
        normalization: DailyNormalizationDescriptorV1Wire {
            file: "normalization.json".into(),
            sha256: ContentHash::from_bytes(&normalization_bytes),
            size_bytes: normalization_bytes.len() as u64,
            row_count: normalization.documents.len() as u64,
        },
        policy_sha256: previous.policy_sha256.clone(),
        materializer_version: "daily-price-only-v1".into(),
        code_commit: producer_commit,
    };
    let manifest = seal_daily_price_manifest_v1(&draft).map_err(|_| Error::InvalidCandidate)?;
    validate_transition(previous, parent.pins(), &manifest)?;
    let manifest_bytes =
        encode_daily_price_manifest_v1(&manifest).map_err(|_| Error::InvalidCandidate)?;
    let manifest_sha256 = ContentHash::from_bytes(&manifest_bytes);
    parent.revalidate()?;
    Ok(DailyContinuationCandidate {
        parent,
        sources,
        prices,
        manifest,
        manifest_bytes,
        manifest_sha256,
        normalization,
        normalization_bytes,
    })
}

/// Independent reconstruction of a child after complete protected-frontier
/// refresh. It retains both current snapshots and the prior protected handles.
/// Its admission bytes alone do not grant storage, DB or loader authority.
pub struct VerifiedDailyContinuation<'proof, 'parent, 'base, 'snapshot> {
    candidate: &'proof OpenedDailyCandidate,
    refreshed: &'proof super::frontier::RefreshedSourceFrontier<'proof, 'parent, 'base, 'snapshot>,
    policy: &'proof PinnedDailyPolicy,
    sources: super::discovery::DiscoveredSessionSources<'snapshot>,
    admission: DailyPriceAdmissionV1Wire,
    admission_bytes: Vec<u8>,
    pins: DailyPricePinsV1Wire,
}
impl VerifiedDailyContinuation<'_, '_, '_, '_> {
    pub fn candidate(&self) -> &OpenedDailyCandidate {
        self.candidate
    }
    pub fn refreshed(&self) -> &super::frontier::RefreshedSourceFrontier<'_, '_, '_, '_> {
        self.refreshed
    }
    pub fn sources(&self) -> &super::discovery::DiscoveredSessionSources<'_> {
        &self.sources
    }
    pub fn policy(&self) -> &PinnedDailyPolicy {
        self.policy
    }
    pub fn admission(&self) -> &DailyPriceAdmissionV1Wire {
        &self.admission
    }
    pub fn admission_bytes(&self) -> &[u8] {
        &self.admission_bytes
    }
    pub fn pins(&self) -> &DailyPricePinsV1Wire {
        &self.pins
    }
    pub fn revalidate(&self) -> Result<(), Error> {
        self.candidate.revalidate().map_err(Error::Artifact)?;
        self.refreshed.parent().revalidate()?;
        self.refreshed.sealed().revalidate()
    }
}

pub fn verify_daily_continuation_candidate<'proof, 'parent, 'base, 'snapshot>(
    candidate: &'proof OpenedDailyCandidate,
    refreshed: &'proof super::frontier::RefreshedSourceFrontier<'proof, 'parent, 'base, 'snapshot>,
    policy: &'proof PinnedDailyPolicy,
    verifier_commit: domain::CodeCommit,
) -> Result<VerifiedDailyContinuation<'proof, 'parent, 'base, 'snapshot>, Error> {
    candidate.revalidate().map_err(Error::Artifact)?;
    refreshed.parent().revalidate()?;
    refreshed.sealed().revalidate()?;
    if candidate.manifest().policy_sha256 != *policy.sha256()
        || refreshed.parent().pins().policy_sha256 != *policy.sha256()
        || candidate.manifest().base != policy.wire().base
    {
        return Err(Error::InvalidCandidate);
    }
    validate_transition(
        refreshed.parent().manifest(),
        refreshed.parent().pins(),
        candidate.manifest(),
    )?;
    let sources = super::discovery::discover_session_sources(
        refreshed.snapshots(),
        &super::discovery::SessionSourceRequest {
            owner: refreshed.sealed().owner(),
            policy_sha256: policy.sha256().clone(),
            previous_frontier: refreshed.parent().manifest().range_end,
            session_date: candidate.manifest().range_end,
        },
    )
    .map_err(Error::Evidence)?;
    let rebuilt = build_daily_continuation_candidate(
        refreshed.parent().reborrow(),
        &sources,
        refreshed.snapshots(),
        candidate.manifest().code_commit.clone(),
    )?;
    candidate
        .matches_continuation_reconstruction(&rebuilt)
        .map_err(Error::Artifact)?;
    let admission = DailyPriceAdmissionV1Wire {
        schema: "owner-beta-daily-etf11-admission-v1".into(),
        content_sha256: rebuilt.content_sha256().clone(),
        manifest_sha256: rebuilt.manifest_sha256().clone(),
        policy_sha256: policy.sha256().clone(),
        parent_admission_sha256: Some(refreshed.parent().pins().admission_sha256.clone()),
        base: candidate.manifest().base.clone(),
        verifier_code_commit: verifier_commit,
        normalizer_version: "daily-raw-reconstruction-v1".into(),
        evidence_sha256: rebuilt.normalization().evidence_sha256.clone(),
        counts: super::admission::counts(rebuilt.manifest())?,
    };
    let admission_bytes =
        encode_daily_price_admission_v1(&admission).map_err(|_| Error::InvalidCandidate)?;
    let pins = DailyPricePinsV1Wire {
        content_sha256: admission.content_sha256.clone(),
        manifest_sha256: admission.manifest_sha256.clone(),
        admission_sha256: ContentHash::from_bytes(&admission_bytes),
        policy_sha256: admission.policy_sha256.clone(),
    };
    validate_daily_price_wire_bundle_v1(
        policy.wire(),
        rebuilt.manifest(),
        rebuilt.normalization(),
        &admission,
        &pins,
    )
    .map_err(|_| Error::InvalidCandidate)?;
    let verified = VerifiedDailyContinuation {
        candidate,
        refreshed,
        policy,
        sources,
        admission,
        admission_bytes,
        pins,
    };
    verified.revalidate()?;
    Ok(verified)
}

/// Copy and reopen only after independent reconstruction. The base/policy remain
/// explicit immutable installed capabilities; the reader rebinds every pin.
pub fn write_verified_daily_continuation<'base>(
    root: &DailyAdmissionRoot,
    verified: &VerifiedDailyContinuation<'_, '_, '_, '_>,
    base: &'base ApprovedHistoricalPriceOnlyArtifact,
    policy: &'base PinnedDailyPolicy,
) -> Result<SealedDailyContinuation<'base>, Error> {
    verified.revalidate()?;
    let prefix = ProtectedHistoricalPrefix::from_approved_v3(base, policy.wire())
        .map_err(Error::Evidence)?;
    if prefix.pins() != &verified.admission().base.pins
        || policy.sha256() != verified.policy().sha256()
    {
        return Err(Error::InvalidCandidate);
    }
    super::artifact::publish_verified_continuation(root, verified).map_err(Error::Artifact)?;
    let opened = read_sealed_daily_continuation(
        root,
        verified.pins(),
        base,
        policy,
        verified.refreshed().parent(),
    )?;
    verified.revalidate()?;
    Ok(opened)
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::history::encode_cumulative_bars;
    use super::super::{
        DailyBarsDescriptorV1Wire, DailyBasePinsV1Wire, DailyBaseV1Wire, DailyEvidenceKindV1,
        DailyNormalizationDescriptorV1Wire, DailyNormalizedDocumentV1Wire,
        DailySessionEvidenceV1Wire,
    };
    use super::*;
    use domain::{CodeCommit, FixedPoint, InstrumentId};

    fn date(value: &str) -> TradingDate {
        TradingDate::parse(value).unwrap()
    }
    fn hash(value: &str) -> ContentHash {
        ContentHash::from_bytes(value.as_bytes())
    }
    fn ids() -> Vec<InstrumentId> {
        crate::providers::kis::KR_ETF_CORE_SYMBOLS
            .iter()
            .map(|symbol| InstrumentId::parse(&format!("{symbol}.KRX")).unwrap())
            .collect()
    }
    fn session(previous: &str, target: &str) -> DailySessionEvidenceV1Wire {
        // Structural transition fixture only. Empty source sets are not valid D4
        // evidence and never construct a sealed input or an approved base.
        DailySessionEvidenceV1Wire {
            session_date: date(target),
            previous_frontier: date(previous),
            instruments: ids(),
            bars: vec![],
            reference: vec![],
            calendar: vec![],
            actions: vec![],
            normalized_documents: vec![DailyNormalizedDocumentV1Wire {
                kind: DailyEvidenceKindV1::Bars,
                session_date: date(target),
                normalizer_version: "daily-raw-reconstruction-v1".into(),
                sha256: hash(target),
                size_bytes: 1,
                row_count: 11,
                source_files: vec![],
                source_evidence_sha256: hash(previous),
            }],
        }
    }
    pub(in crate::daily_price_only) fn fixture() -> (
        DailyPriceManifestV1Wire,
        DailyPricePinsV1Wire,
        DailyPriceManifestV1Wire,
    ) {
        let old = DailyPriceManifestV1Wire {
            schema: "owner-beta-daily-etf11-input-v1".into(),
            capability: "PRICE_RETURN_ONLY".into(),
            audience: "OWNER_ONLY".into(),
            vendor_snapshot: true,
            strict_pit: false,
            base: DailyBaseV1Wire {
                kind: "historical_price_only_v3".into(),
                pins: DailyBasePinsV1Wire {
                    candidate_content_sha256: hash("base-content"),
                    artifact_manifest_sha256: hash("base-manifest"),
                    stage5_manifest_sha256: hash("base-prices"),
                    action_manifest_sha256: hash("base-actions"),
                    approval_registry_sha256: hash("base-registry"),
                },
            },
            parent_admission_sha256: None,
            range_start: date("2026-08-28"),
            range_end: date("2026-08-31"),
            instruments: ids(),
            sessions: vec![date("2026-08-28"), date("2026-08-31")],
            daily_sources: vec![session("2026-08-28", "2026-08-31")],
            bars: DailyBarsDescriptorV1Wire {
                file: "bars.ndjson".into(),
                sha256: hash("bars"),
                size_bytes: 1,
                row_count: 22,
            },
            normalization: DailyNormalizationDescriptorV1Wire {
                file: "normalization.json".into(),
                sha256: hash("proof"),
                size_bytes: 1,
                row_count: 1,
            },
            policy_sha256: hash("policy"),
            materializer_version: "daily-price-only-v1".into(),
            code_commit: CodeCommit::parse(&"a".repeat(40)).unwrap(),
            content_sha256: hash("parent-content"),
        };
        let pins = DailyPricePinsV1Wire {
            content_sha256: old.content_sha256.clone(),
            manifest_sha256: hash("parent-manifest"),
            admission_sha256: hash("parent-admission"),
            policy_sha256: old.policy_sha256.clone(),
        };
        let mut next = old.clone();
        next.parent_admission_sha256 = Some(pins.admission_sha256.clone());
        next.range_end = date("2026-09-01");
        next.sessions.push(next.range_end);
        next.daily_sources.push(session("2026-08-31", "2026-09-01"));
        next.content_sha256 = hash("child-content");
        (old, pins, next)
    }
    fn grid(dates: &[TradingDate]) -> Vec<HistoricalPriceOnlyBar> {
        let price = FixedPoint::parse("10.00").unwrap();
        ids()
            .into_iter()
            .flat_map(|instrument| {
                dates
                    .iter()
                    .copied()
                    .map(move |session_date| HistoricalPriceOnlyBar {
                        instrument_id: instrument.clone(),
                        session_date,
                        raw_open: price,
                        raw_high: price,
                        raw_low: price,
                        raw_close: price,
                        raw_volume: 7,
                        raw_trading_value: Some(FixedPoint::parse("70.00").unwrap()),
                        adjusted_open: price,
                        adjusted_high: price,
                        adjusted_low: price,
                        adjusted_close: price,
                    })
            })
            .collect()
    }
    fn encoded(
        manifest: &mut DailyPriceManifestV1Wire,
        bars: &[HistoricalPriceOnlyBar],
    ) -> Vec<u8> {
        let bytes = encode_cumulative_bars(bars, &[], 100_000).unwrap();
        manifest.bars.sha256 = ContentHash::from_bytes(&bytes);
        manifest.bars.size_bytes = bytes.len() as u64;
        manifest.bars.row_count = bars.len() as u64;
        bytes
    }

    #[test]
    fn s5d_transition_extends_exactly_one_session_and_all_lineage() {
        let (old, pins, mut next) = fixture();
        assert_eq!(validate_transition(&old, &pins, &next), Ok(()));
        next.code_commit = CodeCommit::parse(&"b".repeat(40)).unwrap();
        assert_eq!(validate_transition(&old, &pins, &next), Ok(()));
        assert_eq!(
            next.daily_sources[..old.daily_sources.len()],
            old.daily_sources
        );
    }
    #[test]
    fn s5d_transition_rejects_parent_policy_or_base_substitution() {
        let (old, pins, next) = fixture();
        for case in 0..10 {
            let mut changed = next.clone();
            match case {
                0 => changed.parent_admission_sha256 = None,
                1 => changed.parent_admission_sha256 = Some(hash("other")),
                2 => changed.policy_sha256 = hash("other"),
                3 => changed.base.pins.candidate_content_sha256 = hash("other"),
                4 => changed.base.pins.artifact_manifest_sha256 = hash("other"),
                5 => changed.base.pins.stage5_manifest_sha256 = hash("other"),
                6 => changed.base.pins.action_manifest_sha256 = hash("other"),
                7 => changed.base.pins.approval_registry_sha256 = hash("other"),
                8 => changed.range_start = date("2026-08-27"),
                _ => {
                    changed.instruments.pop();
                }
            }
            assert_eq!(
                validate_transition(&old, &pins, &changed),
                Err(Error::InvalidCandidate)
            );
        }
        let mut wrong = pins.clone();
        wrong.content_sha256 = hash("other");
        assert_eq!(
            validate_transition(&old, &wrong, &next),
            Err(Error::InvalidCandidate)
        );
    }
    #[test]
    fn s5d_transition_rejects_omitted_reordered_or_multiple_sessions() {
        let (old, pins, next) = fixture();
        for case in 0..4 {
            let mut changed = next.clone();
            match case {
                0 => {
                    changed.sessions.remove(0);
                }
                1 => changed.sessions.swap(0, 1),
                2 => changed.sessions.push(date("2026-09-02")),
                _ => changed.range_end = old.range_end,
            }
            assert_eq!(
                validate_transition(&old, &pins, &changed),
                Err(Error::InvalidCandidate)
            );
        }
    }
    #[test]
    fn s5d_transition_rejects_any_rewritten_previous_source_commitment() {
        let (old, pins, next) = fixture();
        for case in 0..5 {
            let mut changed = next.clone();
            let previous = &mut changed.daily_sources[0];
            match case {
                0 => previous.previous_frontier = date("2026-08-27"),
                1 => previous.session_date = date("2026-08-30"),
                2 => previous.normalized_documents[0].sha256 = hash("other"),
                3 => previous.normalized_documents[0].source_evidence_sha256 = hash("other"),
                _ => previous.normalized_documents[0].size_bytes += 1,
            }
            assert_eq!(
                validate_transition(&old, &pins, &changed),
                Err(Error::InvalidCandidate)
            );
        }
    }
    #[test]
    fn s5d_transition_rejects_wrong_new_source_frontier_or_date() {
        let (old, pins, next) = fixture();
        for first in [false, true] {
            let mut changed = next.clone();
            let source = changed.daily_sources.last_mut().unwrap();
            if first {
                source.previous_frontier = date("2026-08-28");
            } else {
                source.session_date = date("2026-09-02");
            }
            assert_eq!(
                validate_transition(&old, &pins, &changed),
                Err(Error::InvalidCandidate)
            );
        }
    }
    #[test]
    fn s5d_readback_preserves_entire_daily_prefix_and_new_grid() {
        let (old, _, mut next) = fixture();
        let prior = grid(&old.sessions);
        let all = grid(&next.sessions);
        let bytes = encoded(&mut next, &all);
        assert_eq!(read_child_bars(&prior, &next, &bytes).unwrap(), all);
    }
    #[test]
    fn s5d_readback_rejects_historical_and_daily_price_rewrites() {
        let (old, _, next) = fixture();
        let prior = grid(&old.sessions);
        for day in 0..2 {
            for adjusted in [false, true] {
                let mut changed = grid(&next.sessions);
                let price = FixedPoint::parse("10.0").unwrap();
                if adjusted {
                    changed[day].adjusted_open = price;
                } else {
                    changed[day].raw_open = price;
                }
                let mut manifest = next.clone();
                let bytes = encoded(&mut manifest, &changed);
                assert!(read_child_bars(&prior, &manifest, &bytes).is_err());
            }
        }
    }
    #[test]
    fn s5d_readback_rejects_missing_new_row_or_adjusted_new_price() {
        let (old, _, next) = fixture();
        let prior = grid(&old.sessions);
        for remove in [false, true] {
            let mut changed = grid(&next.sessions);
            if remove {
                changed.remove(2);
            } else {
                changed[2].adjusted_open = FixedPoint::parse("10.0").unwrap();
            }
            let mut manifest = next.clone();
            let bytes = encoded(&mut manifest, &changed);
            assert!(read_child_bars(&prior, &manifest, &bytes).is_err());
        }
    }
    #[test]
    fn s5d_readback_rejects_hash_size_or_count_mismatch() {
        let (old, _, mut next) = fixture();
        let prior = grid(&old.sessions);
        let all = grid(&next.sessions);
        let bytes = encoded(&mut next, &all);
        for case in 0..3 {
            let mut changed = next.clone();
            match case {
                0 => changed.bars.sha256 = hash("other"),
                1 => changed.bars.size_bytes += 1,
                _ => changed.bars.row_count += 1,
            }
            assert!(read_child_bars(&prior, &changed, &bytes).is_err());
        }
    }
}
