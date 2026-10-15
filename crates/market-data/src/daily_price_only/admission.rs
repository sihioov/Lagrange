//! Independent first-candidate reconstruction, before durable admission/DB commit.
//! This module does not expose an admitted loader handle or assert activation.

use std::collections::BTreeSet;
use std::fmt;

use domain::{CodeCommit, ContentHash, TradingDate};
use uuid::Uuid;

use super::artifact::{
    DailyAdmissionRoot, DailyArtifactError, OpenedDailyCandidate, publish_verified_admission,
    read_admission_files,
};
use super::candidate::build_historical_daily_candidate;
use super::discovery::{DiscoveredSessionSources, SessionSourceRequest, discover_session_sources};
use super::history::{
    ProtectedHistoricalPrefix, read_first_admission_bars, reconstruct_historical_extension,
};
use super::verify::{DailyVerificationError, RawEvidenceSnapshots};
use super::{
    DailyAdmissionCountsV1Wire, DailyPriceAdmissionV1Wire, DailyPriceManifestV1Wire,
    DailyPricePinsV1Wire, DailyPricePolicyV1Wire, encode_daily_price_admission_v1,
    encode_daily_price_policy_v1, parse_daily_price_admission_v1, parse_daily_price_policy_v1,
    validate_daily_price_wire_bundle_v1,
};
use crate::{ApprovedHistoricalPriceOnlyArtifact, HistoricalPriceOnlyBar};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DailyAdmissionVerificationError {
    Artifact(DailyArtifactError),
    Evidence(DailyVerificationError),
    InvalidPolicy,
    InvalidCandidate,
    LimitExceeded,
}
impl fmt::Display for DailyAdmissionVerificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Artifact(error) => fmt::Display::fmt(error, formatter),
            Self::Evidence(error) => fmt::Display::fmt(error, formatter),
            Self::InvalidPolicy => formatter.write_str("DAILY_ADMISSION_INVALID_POLICY"),
            Self::InvalidCandidate => formatter.write_str("DAILY_ADMISSION_INVALID_CANDIDATE"),
            Self::LimitExceeded => formatter.write_str("DAILY_ADMISSION_LIMIT"),
        }
    }
}
impl std::error::Error for DailyAdmissionVerificationError {}
type Error = DailyAdmissionVerificationError;

/// Canonical policy bytes bound to an explicitly supplied expected hash. The
/// caller must obtain that pin from the installed, reviewed policy configuration.
/// This type validates binding; it is not runtime activation or DB authorization.
pub struct PinnedDailyPolicy {
    wire: DailyPricePolicyV1Wire,
    bytes: Vec<u8>,
    sha256: ContentHash,
}
impl PinnedDailyPolicy {
    pub fn from_pinned_bytes(bytes: &[u8], expected: &ContentHash) -> Result<Self, Error> {
        let wire = parse_daily_price_policy_v1(bytes).map_err(|_| Error::InvalidPolicy)?;
        let canonical = encode_daily_price_policy_v1(&wire).map_err(|_| Error::InvalidPolicy)?;
        let sha256 = ContentHash::from_bytes(bytes);
        if canonical != bytes || &sha256 != expected {
            return Err(Error::InvalidPolicy);
        }
        Ok(Self {
            wire,
            bytes: canonical,
            sha256,
        })
    }
    pub fn wire(&self) -> &DailyPricePolicyV1Wire {
        &self.wire
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn sha256(&self) -> &ContentHash {
        &self.sha256
    }
}

/// Privately constructed proof of independent source comparison. It retains the
/// candidate descriptors and source/base/policy lifetimes, but no admitted root,
/// database receipt or loader capability. A process boundary is enforced by the
/// deployment's distinct UIDs/mounts, not inferred from calling this function.
pub struct VerifiedHistoricalAdmission<'proof, 'snapshot> {
    candidate: &'proof OpenedDailyCandidate,
    base: &'proof ApprovedHistoricalPriceOnlyArtifact,
    snapshots: &'proof RawEvidenceSnapshots<'snapshot>,
    policy: &'proof PinnedDailyPolicy,
    sources: DiscoveredSessionSources<'snapshot>,
    admission: DailyPriceAdmissionV1Wire,
    bytes: Vec<u8>,
    pins: DailyPricePinsV1Wire,
}
impl VerifiedHistoricalAdmission<'_, '_> {
    pub fn candidate(&self) -> &OpenedDailyCandidate {
        self.candidate
    }
    pub fn base(&self) -> &ApprovedHistoricalPriceOnlyArtifact {
        self.base
    }
    pub fn snapshots(&self) -> &RawEvidenceSnapshots<'_> {
        self.snapshots
    }
    pub fn policy(&self) -> &PinnedDailyPolicy {
        self.policy
    }
    pub fn sources(&self) -> &DiscoveredSessionSources<'_> {
        &self.sources
    }
    pub fn admission(&self) -> &DailyPriceAdmissionV1Wire {
        &self.admission
    }
    pub fn admission_bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn pins(&self) -> &DailyPricePinsV1Wire {
        &self.pins
    }
}

/// Re-enumerates captured scopes and reconstructs the entire first continuation;
/// the producer's listed files/documents are never used to select the evidence.
/// No files, SQL, providers, random identity or wall-clock timestamp are written.
pub fn verify_historical_daily_candidate<'proof, 'snapshot>(
    candidate: &'proof OpenedDailyCandidate,
    base: &'proof ApprovedHistoricalPriceOnlyArtifact,
    snapshots: &'proof RawEvidenceSnapshots<'snapshot>,
    policy: &'proof PinnedDailyPolicy,
    owner: Uuid,
    verifier_commit: CodeCommit,
) -> Result<VerifiedHistoricalAdmission<'proof, 'snapshot>, Error> {
    candidate.revalidate().map_err(Error::Artifact)?;
    let manifest = candidate.manifest();
    if manifest.parent_admission_sha256.is_some() || &manifest.policy_sha256 != policy.sha256() {
        return Err(Error::InvalidCandidate);
    }
    let prefix = ProtectedHistoricalPrefix::from_approved_v3(base, policy.wire())
        .map_err(Error::Evidence)?;
    let sources = discover_session_sources(
        snapshots,
        &SessionSourceRequest {
            owner,
            policy_sha256: policy.sha256().clone(),
            previous_frontier: prefix.frontier(),
            session_date: manifest.range_end,
        },
    )
    .map_err(Error::Evidence)?;
    let extension =
        reconstruct_historical_extension(&prefix, &sources, snapshots).map_err(Error::Evidence)?;
    let rebuilt = build_historical_daily_candidate(&extension, manifest.code_commit.clone())
        .map_err(Error::Evidence)?;
    candidate
        .matches_reconstruction(&rebuilt)
        .map_err(Error::Artifact)?;
    let admission = DailyPriceAdmissionV1Wire {
        schema: "owner-beta-daily-etf11-admission-v1".into(),
        content_sha256: rebuilt.content_sha256().clone(),
        manifest_sha256: rebuilt.manifest_sha256().clone(),
        policy_sha256: policy.sha256().clone(),
        parent_admission_sha256: None,
        base: manifest.base.clone(),
        verifier_code_commit: verifier_commit,
        normalizer_version: "daily-raw-reconstruction-v1".into(),
        evidence_sha256: rebuilt.normalization().evidence_sha256.clone(),
        counts: counts(rebuilt.manifest())?,
    };
    let bytes = encode_daily_price_admission_v1(&admission).map_err(|_| Error::InvalidCandidate)?;
    let pins = DailyPricePinsV1Wire {
        content_sha256: admission.content_sha256.clone(),
        manifest_sha256: admission.manifest_sha256.clone(),
        admission_sha256: ContentHash::from_bytes(&bytes),
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
    candidate.revalidate().map_err(Error::Artifact)?;
    Ok(VerifiedHistoricalAdmission {
        candidate,
        base,
        snapshots,
        policy,
        sources,
        admission,
        bytes,
        pins,
    })
}

/// Verified immutable first-daily file evidence. Offline replay may retain this
/// after revocation. Current authorization/publication still requires the guarded
/// DB admission and ancestry checks; this is not their receipt or a live data pin.
pub struct SealedHistoricalDailyInput<'base> {
    base: &'base ApprovedHistoricalPriceOnlyArtifact,
    policy: &'base PinnedDailyPolicy,
    opened: OpenedDailyCandidate,
    admission: DailyPriceAdmissionV1Wire,
    admission_bytes: Vec<u8>,
    pins: DailyPricePinsV1Wire,
    bars: Vec<HistoricalPriceOnlyBar>,
}
impl SealedHistoricalDailyInput<'_> {
    pub fn base(&self) -> &ApprovedHistoricalPriceOnlyArtifact {
        self.base
    }
    pub fn policy(&self) -> &PinnedDailyPolicy {
        self.policy
    }
    pub fn pins(&self) -> &DailyPricePinsV1Wire {
        &self.pins
    }
    pub fn manifest(&self) -> &DailyPriceManifestV1Wire {
        self.opened.manifest()
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

pub fn read_sealed_historical_daily_input<'base>(
    root: &DailyAdmissionRoot,
    expected: &DailyPricePinsV1Wire,
    base: &'base ApprovedHistoricalPriceOnlyArtifact,
    policy: &'base PinnedDailyPolicy,
) -> Result<SealedHistoricalDailyInput<'base>, Error> {
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
    let prefix = ProtectedHistoricalPrefix::from_approved_v3(base, policy.wire())
        .map_err(Error::Evidence)?;
    let bars = read_first_admission_bars(&prefix, opened.manifest(), opened.bars_ndjson())
        .map_err(Error::Evidence)?;
    opened.revalidate().map_err(Error::Artifact)?;
    Ok(SealedHistoricalDailyInput {
        base,
        policy,
        opened,
        admission,
        admission_bytes,
        pins: expected.clone(),
        bars,
    })
}

/// A verified orphan becomes durable file evidence here. Only the subsequent
/// guarded SQL commit makes it usable for scheduling; failure never deletes an
/// existing leaf. The admitter UID must differ from the candidate producer UID.
pub fn write_verified_historical_admission<'base>(
    root: &DailyAdmissionRoot,
    verified: &VerifiedHistoricalAdmission<'base, '_>,
) -> Result<SealedHistoricalDailyInput<'base>, Error> {
    publish_verified_admission(root, verified).map_err(Error::Artifact)?;
    read_sealed_historical_daily_input(root, &verified.pins, verified.base, verified.policy)
}

fn counts(manifest: &DailyPriceManifestV1Wire) -> Result<DailyAdmissionCountsV1Wire, Error> {
    let mut files = BTreeSet::new();
    let mut calendar = 0u64;
    let mut actions = 0u64;
    let mut pages = 0u64;
    let mut cash = 0u64;
    let mut documents = 0u64;
    for session in &manifest.daily_sources {
        add(&mut calendar, session.calendar.len())?;
        add(&mut actions, session.actions.len())?;
        add(&mut documents, session.normalized_documents.len())?;
        for chain in &session.actions {
            add(&mut pages, chain.pages.len())?;
            add(&mut cash, chain.cash_dividends.len())?;
        }
        for file in session
            .bars
            .iter()
            .chain(&session.reference)
            .chain(session.calendar.iter().map(|row| &row.source_file))
            .chain(session.actions.iter().flat_map(|chain| &chain.pages))
        {
            files.insert((
                file.provider.as_str(),
                file.market.as_str(),
                file.batch_id,
                file.file_name.as_str(),
            ));
        }
    }
    Ok(DailyAdmissionCountsV1Wire {
        instrument_count: count(manifest.instruments.len())?,
        session_count: count(manifest.sessions.len())?,
        bar_count: manifest.bars.row_count,
        added_session_count: count(manifest.daily_sources.len())?,
        source_file_count: count(files.len())?,
        calendar_row_count: calendar,
        action_chain_count: actions,
        action_page_count: pages,
        cash_dividend_row_count: cash,
        normalized_document_count: documents,
    })
}
fn count(value: usize) -> Result<u64, Error> {
    u64::try_from(value).map_err(|_| Error::LimitExceeded)
}
fn add(total: &mut u64, value: usize) -> Result<(), Error> {
    *total = total
        .checked_add(count(value)?)
        .ok_or(Error::LimitExceeded)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::contract::fixed_limits;
    use super::super::{
        DailyBasePinsV1Wire, DailyBaseV1Wire, DailyEvidenceKindV1, DailySourceKindV1Wire,
    };
    use super::*;
    fn hash(text: &str) -> ContentHash {
        ContentHash::from_bytes(text.as_bytes())
    }
    fn policy() -> DailyPricePolicyV1Wire {
        DailyPricePolicyV1Wire {
            schema: "owner-beta-daily-etf11-policy-v1".into(),
            capability: "PRICE_RETURN_ONLY".into(),
            audience: "OWNER_ONLY".into(),
            vendor_snapshot: true,
            strict_pit: false,
            base: DailyBaseV1Wire {
                kind: "historical_price_only_v3".into(),
                pins: DailyBasePinsV1Wire {
                    candidate_content_sha256: hash("content"),
                    artifact_manifest_sha256: hash("manifest"),
                    stage5_manifest_sha256: hash("price"),
                    action_manifest_sha256: hash("action"),
                    approval_registry_sha256: hash("registry"),
                },
            },
            instruments: crate::providers::kis::KR_ETF_CORE_SYMBOLS
                .iter()
                .map(|symbol| domain::InstrumentId::parse(&format!("{symbol}.KRX")).unwrap())
                .collect(),
            price_semantics: "KIS_FID_ORG_ADJ_PRC_1_RAW_OHLC".into(),
            cash_action_policy: "CASH_DIVIDENDS_EXCLUDED_NONCASH_BLOCKED_V1".into(),
            normalizer_version: "daily-raw-reconstruction-v1".into(),
            materializer_version: "daily-price-only-v1".into(),
            source_kinds: vec![
                DailyEvidenceKindV1::Bars,
                DailyEvidenceKindV1::Calendar,
                DailyEvidenceKindV1::CorporateActions,
                DailyEvidenceKindV1::Reference,
            ]
            .into_iter()
            .map(|kind| DailySourceKindV1Wire {
                provider: "kis".into(),
                market: "kr".into(),
                kind,
            })
            .chain([DailySourceKindV1Wire {
                provider: "kis-calendar".into(),
                market: "kr".into(),
                kind: DailyEvidenceKindV1::Calendar,
            }])
            .collect(),
            limits: fixed_limits(),
        }
    }

    #[test]
    fn s5c_policy_pin_requires_exact_canonical_bytes() {
        let bytes = encode_daily_price_policy_v1(&policy()).unwrap();
        let pin = ContentHash::from_bytes(&bytes);
        let checked = PinnedDailyPolicy::from_pinned_bytes(&bytes, &pin).unwrap();
        assert_eq!(checked.bytes(), bytes);
        assert_eq!(checked.sha256(), &pin);
        assert_eq!(checked.wire().base, policy().base);
        assert!(matches!(
            PinnedDailyPolicy::from_pinned_bytes(&bytes, &hash("different")),
            Err(Error::InvalidPolicy)
        ));
    }

    #[test]
    fn s5c_policy_pin_rejects_whitespace_even_with_matching_raw_hash() {
        let bytes = encode_daily_price_policy_v1(&policy()).unwrap();
        let mut changed = bytes.clone();
        changed.push(b'\n');
        assert!(matches!(
            PinnedDailyPolicy::from_pinned_bytes(&changed, &ContentHash::from_bytes(&changed)),
            Err(Error::InvalidPolicy)
        ));
        assert!(
            PinnedDailyPolicy::from_pinned_bytes(&bytes, &ContentHash::from_bytes(&bytes)).is_ok()
        );
    }

    #[test]
    fn s5c_policy_pin_rejects_duplicate_and_unknown_fields() {
        let bytes = String::from_utf8(encode_daily_price_policy_v1(&policy()).unwrap()).unwrap();
        for text in [
            bytes.replacen('{', "{\"schema\":\"owner-beta-daily-etf11-policy-v1\",", 1),
            bytes.replacen('{', "{\"extra\":true,", 1),
        ] {
            assert!(matches!(
                PinnedDailyPolicy::from_pinned_bytes(
                    text.as_bytes(),
                    &ContentHash::from_bytes(text.as_bytes())
                ),
                Err(Error::InvalidPolicy)
            ));
        }
    }

    #[test]
    fn s5c_policy_changed_base_pin_cannot_reuse_old_expected_hash() {
        let original = encode_daily_price_policy_v1(&policy()).unwrap();
        let mut changed = policy();
        changed.base.pins.artifact_manifest_sha256 = hash("changed");
        let bytes = encode_daily_price_policy_v1(&changed).unwrap();
        assert!(matches!(
            PinnedDailyPolicy::from_pinned_bytes(&bytes, &ContentHash::from_bytes(&original)),
            Err(Error::InvalidPolicy)
        ));
    }

    #[test]
    fn s5c_policy_invalid_semantics_do_not_become_valid_via_new_hash() {
        let bytes = String::from_utf8(encode_daily_price_policy_v1(&policy()).unwrap()).unwrap();
        for (from, to) in [
            ("PRICE_RETURN_ONLY", "TOTAL_RETURN"),
            ("OWNER_ONLY", "PUBLIC"),
            ("daily-raw-reconstruction-v1", "other"),
            ("CASH_DIVIDENDS_EXCLUDED_NONCASH_BLOCKED_V1", "ALLOW_ALL"),
        ] {
            let changed = bytes.replace(from, to);
            assert_ne!(changed, bytes);
            assert!(matches!(
                PinnedDailyPolicy::from_pinned_bytes(
                    changed.as_bytes(),
                    &ContentHash::from_bytes(changed.as_bytes())
                ),
                Err(Error::InvalidPolicy)
            ));
        }
    }
}
