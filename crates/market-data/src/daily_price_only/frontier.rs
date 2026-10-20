//! Protected observation preparation and refresh; no current DB authority.
//! The file-store adapter must bind these bytes to the admitter-owned state root.

use domain::{ContentHash, TradingDate};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::DailyFileEvidenceV1Wire;
use super::admission::VerifiedHistoricalAdmission;
use super::contract::fixed_limits;
use super::observation::{CanonicalSourceObservation, canonical_source_observation};
use super::verify::DailyVerificationError as Error;

const SCHEMA: &str = "owner-beta-daily-source-frontier-v1";
const OBSERVATION_PREFIX: &[u8] = b"owner-beta-daily-source-observation-v1\n";

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct FrontierRecord {
    schema: String,
    owner: Uuid,
    policy_sha256: ContentHash,
    admission_sha256: ContentHash,
    parent_frontier_sha256: Option<ContentHash>,
    range_start: TradingDate,
    range_end: TradingDate,
    observation_sha256: ContentHash,
    source_file_count: u64,
}

/// Unapproved navigation copied from one protected state record. This is not a
/// source observation, sealed frontier, admission or current database authority.
pub struct DailyFrontierNavigation {
    record: FrontierRecord,
}
impl DailyFrontierNavigation {
    pub fn owner(&self) -> Uuid {
        self.record.owner
    }
    pub fn policy_sha256(&self) -> &ContentHash {
        &self.record.policy_sha256
    }
    pub fn admission_sha256(&self) -> &ContentHash {
        &self.record.admission_sha256
    }
    pub fn parent_frontier_sha256(&self) -> Option<&ContentHash> {
        self.record.parent_frontier_sha256.as_ref()
    }
    pub fn range_start(&self) -> TradingDate {
        self.record.range_start
    }
    pub fn range_end(&self) -> TradingDate {
        self.record.range_end
    }
}

/// Reads only the record, never its observation payload. The returned hash links
/// are navigation hints; read_sealed_source_frontier remains mandatory.
pub fn read_daily_frontier_navigation(
    root: &super::artifact::DailySourceFrontierRoot,
    expected: &ContentHash,
) -> Result<DailyFrontierNavigation, super::admission::DailyAdmissionVerificationError> {
    use super::admission::DailyAdmissionVerificationError as AdmissionError;
    let bytes = super::artifact::read_source_frontier_metadata(root, expected)
        .map_err(AdmissionError::Artifact)?;
    decode_frontier_navigation(&bytes, expected).map_err(AdmissionError::Evidence)
}

pub(super) fn decode_frontier_navigation(
    bytes: &[u8],
    expected: &ContentHash,
) -> Result<DailyFrontierNavigation, Error> {
    if bytes.len() as u64 > fixed_limits().max_admission_bytes {
        return Err(Error::LimitExceeded);
    }
    let record: FrontierRecord =
        serde_json::from_slice(bytes).map_err(|_| Error::InvalidEvidence)?;
    if record.schema != SCHEMA
        || record.owner.is_nil()
        || record.range_end < record.range_start
        || record.observation_sha256 != *expected
        || serde_json::to_vec(&record).map_err(|_| Error::Encoding)? != bytes
    {
        return Err(Error::InvalidEvidence);
    }
    Ok(DailyFrontierNavigation { record })
}

// Private implementations retain the complete verification borrow, including
// captured sources and their lock lifetime. There is no wire-to-proof constructor.
trait VerificationBorrow {
    fn revalidate(&self) -> Result<(), super::admission::DailyAdmissionVerificationError>;
}
impl VerificationBorrow for VerifiedHistoricalAdmission<'_, '_> {
    fn revalidate(&self) -> Result<(), super::admission::DailyAdmissionVerificationError> {
        self.candidate()
            .revalidate()
            .map_err(super::admission::DailyAdmissionVerificationError::Artifact)
    }
}

pub struct PreparedSourceFrontier<'proof> {
    verification: &'proof dyn VerificationBorrow,
    record: FrontierRecord,
    record_bytes: Vec<u8>,
    observation: CanonicalSourceObservation,
}

impl PreparedSourceFrontier<'_> {
    pub fn owner(&self) -> Uuid {
        self.record.owner
    }
    pub fn policy_sha256(&self) -> &ContentHash {
        &self.record.policy_sha256
    }
    pub fn admission_sha256(&self) -> &ContentHash {
        &self.record.admission_sha256
    }
    pub fn sha256(&self) -> &ContentHash {
        self.observation.sha256()
    }
    pub fn record_bytes(&self) -> &[u8] {
        &self.record_bytes
    }
    pub fn observation_bytes(&self) -> &[u8] {
        self.observation.preimage()
    }
}

/// Only the independent verifier's complete observation can prepare a first seal.
/// Durability, protected reopening and SQL commit are separate required steps.
pub fn prepare_first_source_frontier<'proof>(
    verified: &'proof VerifiedHistoricalAdmission<'_, '_>,
) -> Result<PreparedSourceFrontier<'proof>, super::admission::DailyAdmissionVerificationError> {
    use super::admission::DailyAdmissionVerificationError as AdmissionError;
    verified.revalidate()?;
    let source = verified.sources();
    let start = next_day(source.session().previous_frontier).map_err(AdmissionError::Evidence)?;
    let observation = canonical_source_observation(source.observation().files())
        .map_err(AdmissionError::Evidence)?;
    let record = FrontierRecord {
        schema: SCHEMA.into(),
        owner: source.owner(),
        policy_sha256: verified.pins().policy_sha256.clone(),
        admission_sha256: verified.pins().admission_sha256.clone(),
        parent_frontier_sha256: None,
        range_start: start,
        range_end: source.session().session_date,
        observation_sha256: observation.sha256().clone(),
        source_file_count: observation.files().len() as u64,
    };
    let record_bytes = encode_record(&record, &observation).map_err(AdmissionError::Evidence)?;
    verified.revalidate()?;
    Ok(PreparedSourceFrontier {
        verification: verified,
        record,
        record_bytes,
        observation,
    })
}

impl VerificationBorrow for super::continuation::VerifiedDailyContinuation<'_, '_, '_, '_> {
    fn revalidate(&self) -> Result<(), super::admission::DailyAdmissionVerificationError> {
        super::continuation::VerifiedDailyContinuation::revalidate(self)
    }
}

pub fn prepare_continued_source_frontier<'proof>(
    verified: &'proof super::continuation::VerifiedDailyContinuation<'_, '_, '_, '_>,
) -> Result<PreparedSourceFrontier<'proof>, super::admission::DailyAdmissionVerificationError> {
    use super::admission::DailyAdmissionVerificationError as AdmissionError;
    verified.revalidate()?;
    let prior = verified.refreshed().sealed();
    let mut files = verified.refreshed().observation().files().to_vec();
    files.extend_from_slice(verified.sources().observation().files());
    let observation = canonical_source_observation(&files).map_err(AdmissionError::Evidence)?;
    let record = FrontierRecord {
        schema: SCHEMA.into(),
        owner: prior.owner(),
        policy_sha256: verified.pins().policy_sha256.clone(),
        admission_sha256: verified.pins().admission_sha256.clone(),
        parent_frontier_sha256: Some(prior.sha256().clone()),
        range_start: prior.range_start(),
        range_end: verified.candidate().manifest().range_end,
        observation_sha256: observation.sha256().clone(),
        source_file_count: observation.files().len() as u64,
    };
    let record_bytes = encode_record(&record, &observation).map_err(AdmissionError::Evidence)?;
    verified.revalidate()?;
    Ok(PreparedSourceFrontier {
        verification: verified,
        record,
        record_bytes,
        observation,
    })
}

fn next_day(date: TradingDate) -> Result<TradingDate, Error> {
    let date = date
        .as_naive_date()
        .succ_opt()
        .ok_or(Error::InvalidContext)?;
    TradingDate::parse(&date.format("%Y-%m-%d").to_string()).map_err(|_| Error::InvalidContext)
}

fn encode_record(
    record: &FrontierRecord,
    observation: &CanonicalSourceObservation,
) -> Result<Vec<u8>, Error> {
    if record.schema != SCHEMA
        || record.owner.is_nil()
        || record.range_end < record.range_start
        || record.source_file_count != observation.files().len() as u64
        || &record.observation_sha256 != observation.sha256()
    {
        return Err(Error::InvalidEvidence);
    }
    let bytes = serde_json::to_vec(record).map_err(|_| Error::Encoding)?;
    if bytes.len() as u64 > fixed_limits().max_admission_bytes {
        return Err(Error::LimitExceeded);
    }
    Ok(bytes)
}

fn decode_record(
    bytes: &[u8],
    observation: &CanonicalSourceObservation,
) -> Result<FrontierRecord, Error> {
    if bytes.len() as u64 > fixed_limits().max_admission_bytes {
        return Err(Error::LimitExceeded);
    }
    let record: FrontierRecord =
        serde_json::from_slice(bytes).map_err(|_| Error::InvalidEvidence)?;
    if encode_record(&record, observation)? != bytes {
        return Err(Error::InvalidEvidence);
    }
    Ok(record)
}

fn decode_observation(bytes: &[u8]) -> Result<CanonicalSourceObservation, Error> {
    if bytes.len() as u64 > fixed_limits().max_manifest_bytes {
        return Err(Error::LimitExceeded);
    }
    let claims = bytes
        .strip_prefix(OBSERVATION_PREFIX)
        .ok_or(Error::InvalidEvidence)?;
    let files: Vec<DailyFileEvidenceV1Wire> =
        serde_json::from_slice(claims).map_err(|_| Error::InvalidEvidence)?;
    let observation = canonical_source_observation(&files)?;
    if observation.preimage() != bytes {
        return Err(Error::InvalidEvidence);
    }
    Ok(observation)
}

/// Expected source claims reopened from the separate admitter-owned state tree.
/// This capability is bound to one sealed input and remains an offline observation,
/// not current eligibility or a database poll receipt.
pub struct SealedSourceFrontier {
    opened: super::artifact::OpenedSourceFrontierFiles,
    record: FrontierRecord,
    observation: CanonicalSourceObservation,
}
impl SealedSourceFrontier {
    pub fn owner(&self) -> Uuid {
        self.record.owner
    }
    pub fn policy_sha256(&self) -> &ContentHash {
        &self.record.policy_sha256
    }
    pub fn admission_sha256(&self) -> &ContentHash {
        &self.record.admission_sha256
    }
    pub fn sha256(&self) -> &ContentHash {
        self.observation.sha256()
    }
    pub fn observation(&self) -> &CanonicalSourceObservation {
        &self.observation
    }
    pub fn range_start(&self) -> TradingDate {
        self.record.range_start
    }
    pub fn range_end(&self) -> TradingDate {
        self.record.range_end
    }
    pub fn revalidate(&self) -> Result<(), super::admission::DailyAdmissionVerificationError> {
        self.opened
            .revalidate()
            .map_err(super::admission::DailyAdmissionVerificationError::Artifact)
    }
}

pub fn read_sealed_source_frontier(
    root: &super::artifact::DailySourceFrontierRoot,
    expected: &ContentHash,
    owner: Uuid,
    input: &super::continuation::SealedDailyParent<'_, '_>,
    previous: Option<&SealedSourceFrontier>,
) -> Result<SealedSourceFrontier, super::admission::DailyAdmissionVerificationError> {
    use super::admission::DailyAdmissionVerificationError as AdmissionError;
    input.revalidate()?;
    if let Some(prior) = previous {
        prior.revalidate()?;
    }
    let opened = super::artifact::read_source_frontier_files(root, expected, input.opened())
        .map_err(AdmissionError::Artifact)?;
    let observation =
        decode_observation(opened.observation_bytes()).map_err(AdmissionError::Evidence)?;
    if observation.sha256() != expected {
        return Err(AdmissionError::InvalidCandidate);
    }
    let record =
        decode_record(opened.record_bytes(), &observation).map_err(AdmissionError::Evidence)?;
    validate_binding(
        &record,
        owner,
        input.manifest(),
        input.pins(),
        previous.map(|v| &v.record),
    )
    .map_err(AdmissionError::Evidence)?;
    input.revalidate()?;
    if let Some(prior) = previous {
        prior.revalidate()?;
    }
    opened.revalidate().map_err(AdmissionError::Artifact)?;
    Ok(SealedSourceFrontier {
        opened,
        record,
        observation,
    })
}

pub fn write_source_frontier(
    root: &super::artifact::DailySourceFrontierRoot,
    prepared: &PreparedSourceFrontier<'_>,
    input: &super::continuation::SealedDailyParent<'_, '_>,
    previous: Option<&SealedSourceFrontier>,
) -> Result<SealedSourceFrontier, super::admission::DailyAdmissionVerificationError> {
    use super::admission::DailyAdmissionVerificationError as AdmissionError;
    prepared.verification.revalidate()?;
    input.revalidate()?;
    if let Some(prior) = previous {
        prior.revalidate()?;
    }
    validate_binding(
        &prepared.record,
        prepared.owner(),
        input.manifest(),
        input.pins(),
        previous.map(|v| &v.record),
    )
    .map_err(AdmissionError::Evidence)?;
    super::artifact::publish_source_frontier_files(
        root,
        prepared.sha256(),
        input.opened(),
        prepared.record_bytes(),
        prepared.observation_bytes(),
    )
    .map_err(AdmissionError::Artifact)?;
    let sealed =
        read_sealed_source_frontier(root, prepared.sha256(), prepared.owner(), input, previous)?;
    prepared.verification.revalidate()?;
    Ok(sealed)
}

fn validate_binding(
    record: &FrontierRecord,
    owner: Uuid,
    manifest: &super::DailyPriceManifestV1Wire,
    pins: &super::DailyPricePinsV1Wire,
    previous: Option<&FrontierRecord>,
) -> Result<(), Error> {
    let start = next_day(
        manifest
            .daily_sources
            .first()
            .ok_or(Error::InvalidContext)?
            .previous_frontier,
    )?;
    if record.owner != owner
        || record.policy_sha256 != pins.policy_sha256
        || record.policy_sha256 != manifest.policy_sha256
        || record.admission_sha256 != pins.admission_sha256
        || record.range_start != start
        || record.range_end != manifest.range_end
    {
        return Err(Error::InvalidContext);
    }
    match previous {
        None if record.parent_frontier_sha256.is_none()
            && manifest.parent_admission_sha256.is_none() => {}
        Some(prior)
            if record.parent_frontier_sha256.as_ref() == Some(&prior.observation_sha256)
                && manifest.parent_admission_sha256.as_ref() == Some(&prior.admission_sha256)
                && prior.owner == owner
                && prior.policy_sha256 == record.policy_sha256
                && prior.range_start == record.range_start
                && prior.range_end < record.range_end => {}
        _ => return Err(Error::InvalidContext),
    }
    Ok(())
}

/// A changed complete protected observation has attributable immutable evidence.
/// A discovery/IO/parse failure is only a typed poll failure: never synthesize an
/// event hash from an error message or an unknown source.
pub enum SourceFrontierRefreshError {
    Input(super::admission::DailyAdmissionVerificationError),
    Blocked(super::observation::CanonicalSourceEvent),
}
impl std::fmt::Display for SourceFrontierRefreshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Input(error) => std::fmt::Display::fmt(error, f),
            Self::Blocked(event) => f.write_str(event.code().code()),
        }
    }
}
impl From<super::admission::DailyAdmissionVerificationError> for SourceFrontierRefreshError {
    fn from(error: super::admission::DailyAdmissionVerificationError) -> Self {
        Self::Input(error)
    }
}

/// Revalidated against fresh locked scopes, not reconstructed from a caller file
/// list. The borrows retain the protected parent/state and Raw snapshot owners.
pub struct RefreshedSourceFrontier<'proof, 'parent, 'base, 'snapshot> {
    parent: &'proof super::continuation::SealedDailyParent<'parent, 'base>,
    sealed: &'proof SealedSourceFrontier,
    snapshots: &'proof super::verify::RawEvidenceSnapshots<'snapshot>,
    observation: CanonicalSourceObservation,
}
impl<'proof, 'parent, 'base, 'snapshot> RefreshedSourceFrontier<'proof, 'parent, 'base, 'snapshot> {
    pub fn parent(&self) -> &super::continuation::SealedDailyParent<'parent, 'base> {
        self.parent
    }
    pub fn sealed(&self) -> &SealedSourceFrontier {
        self.sealed
    }
    pub fn snapshots(&self) -> &super::verify::RawEvidenceSnapshots<'snapshot> {
        self.snapshots
    }
    pub fn observation(&self) -> &CanonicalSourceObservation {
        &self.observation
    }
}

pub fn refresh_source_frontier<'proof, 'parent, 'base, 'snapshot>(
    parent: &'proof super::continuation::SealedDailyParent<'parent, 'base>,
    sealed: &'proof SealedSourceFrontier,
    snapshots: &'proof super::verify::RawEvidenceSnapshots<'snapshot>,
) -> Result<RefreshedSourceFrontier<'proof, 'parent, 'base, 'snapshot>, SourceFrontierRefreshError>
{
    use super::admission::DailyAdmissionVerificationError as AdmissionError;
    use super::discovery::{SessionSourceRequest, discover_session_sources};
    use super::observation::compare_source_observation;
    parent.revalidate()?;
    sealed.revalidate()?;
    let manifest = parent.manifest();
    let first = manifest
        .daily_sources
        .first()
        .ok_or(AdmissionError::InvalidCandidate)?;
    if sealed.record.admission_sha256 != parent.pins().admission_sha256
        || sealed.record.policy_sha256 != parent.pins().policy_sha256
        || sealed.range_end() != manifest.range_end
        || sealed.range_start()
            != next_day(first.previous_frontier).map_err(AdmissionError::Evidence)?
    {
        return Err(AdmissionError::InvalidCandidate.into());
    }
    // Bound repeated discovery before doing it. Captured files are immutable;
    // this conservative upper bound includes irrelevant scopes and copy bytes.
    let mut scan_bytes = 0u64;
    let mut scan_files = 0u64;
    for scope in snapshots.scopes() {
        for batch in scope.batches() {
            for file in batch.files() {
                scan_bytes = scan_bytes
                    .checked_add(file.bytes().len() as u64)
                    .ok_or(AdmissionError::LimitExceeded)?;
                scan_files = scan_files
                    .checked_add(1)
                    .ok_or(AdmissionError::LimitExceeded)?;
            }
        }
    }
    let passes = (manifest.daily_sources.len() as u64)
        .checked_add(1)
        .ok_or(AdmissionError::LimitExceeded)?;
    check_refresh_budget(scan_bytes, scan_files, passes).map_err(AdmissionError::Evidence)?;
    // An expected commitment in a successfully captured complete scope has an
    // attributable identity even if its file/lineage is now missing or changed.
    // Missing scopes and resource/IO failures remain ordinary poll failures.
    for expected in sealed.observation().files() {
        for (provider, market) in std::iter::once((&expected.provider, &expected.market)).chain(
            expected
                .copied_from
                .iter()
                .map(|origin| (&origin.provider, &origin.market)),
        ) {
            if !snapshots.scopes().iter().any(|scope| {
                scope.scope().provider() == provider && scope.scope().market() == market
            }) {
                return Err(AdmissionError::Evidence(Error::InvalidContext).into());
            }
        }
        let reference = crate::contract::SourceFileReference {
            provider: expected.provider.clone(),
            market: expected.market.clone(),
            batch_id: expected.batch_id,
            file_name: expected.file_name.clone(),
            content_hash: expected.wire_sha256.clone(),
            retrieved_at: expected.retrieved_at.as_timestamp(),
        };
        match super::verify::resolve_original(snapshots, &reference) {
            Ok(current) if current.evidence() == expected => {}
            Ok(current) => {
                return Err(SourceFrontierRefreshError::Blocked(
                    expected_source_event(
                        &sealed.record,
                        expected,
                        Some(current.evidence()),
                        super::observation::DailyBlockCode::SealedSourceChanged,
                    )
                    .map_err(AdmissionError::Evidence)?,
                ));
            }
            Err(error) if known_integrity_error(error) => {
                return Err(SourceFrontierRefreshError::Blocked(
                    expected_source_event(
                        &sealed.record,
                        expected,
                        None,
                        super::observation::DailyBlockCode::SourceIntegrity,
                    )
                    .map_err(AdmissionError::Evidence)?,
                ));
            }
            Err(error) => return Err(AdmissionError::Evidence(error).into()),
        }
    }
    // Observe additional committed identities before reconstruction can reject a
    // duplicate/contradictory delivery. Such a same-date append must leave an
    // attributable blocked event even when no valid new document can be built.
    let appended =
        appended_sealed_observation(sealed, snapshots).map_err(AdmissionError::Evidence)?;
    if let Some(event) = compare_source_observation(
        sealed.owner(),
        sealed.range_start(),
        sealed.range_end(),
        sealed.observation(),
        &appended,
    )
    .map_err(AdmissionError::Evidence)?
    {
        return Err(SourceFrontierRefreshError::Blocked(event));
    }
    let mut files = Vec::new();
    let mut session_changed = false;
    for (index, previous) in manifest.daily_sources.iter().enumerate() {
        let current = discover_session_sources(
            snapshots,
            &SessionSourceRequest {
                owner: sealed.owner(),
                policy_sha256: sealed.policy_sha256().clone(),
                previous_frontier: previous.previous_frontier,
                session_date: previous.session_date,
            },
        )
        .map_err(AdmissionError::Evidence)?;
        if index == 0 {
            let conflict = super::price_extension::compare_observed_prefix(
                super::price_extension::PricePrefixView {
                    bars: parent.bars(),
                    sessions: &manifest.sessions,
                    policy_sha256: &manifest.policy_sha256,
                },
                &current,
                snapshots,
            )
            .map_err(AdmissionError::Evidence)?;
            if let Some(source) = conflict {
                let observation =
                    canonical_source_observation(&[source]).map_err(AdmissionError::Evidence)?;
                let event = super::observation::canonical_source_event(
                    sealed.owner(),
                    super::observation::DailySourceKind::Bar,
                    super::observation::DailyBlockCode::SourceConflict,
                    manifest.range_start,
                    manifest.range_end,
                    &observation,
                )
                .map_err(AdmissionError::Evidence)?;
                return Err(SourceFrontierRefreshError::Blocked(event));
            }
        }
        super::candidate::validate_documents(
            &current.session().normalized_documents,
            current
                .documents()
                .iter()
                .map(|d| (d.descriptor(), d.bytes())),
        )
        .map_err(AdmissionError::Evidence)?;
        session_changed |= current.session() != previous;
        files.extend_from_slice(current.observation().files());
    }
    let observation = canonical_source_observation(&files).map_err(AdmissionError::Evidence)?;
    if let Some(event) = compare_source_observation(
        sealed.owner(),
        sealed.range_start(),
        sealed.range_end(),
        sealed.observation(),
        observation.files(),
    )
    .map_err(AdmissionError::Evidence)?
    {
        return Err(SourceFrontierRefreshError::Blocked(event));
    }
    if session_changed {
        return Err(AdmissionError::InvalidCandidate.into());
    }
    parent.revalidate()?;
    sealed.revalidate()?;
    Ok(RefreshedSourceFrontier {
        parent,
        sealed,
        snapshots,
        observation,
    })
}

fn appended_sealed_observation(
    sealed: &SealedSourceFrontier,
    snapshots: &super::verify::RawEvidenceSnapshots<'_>,
) -> Result<Vec<DailyFileEvidenceV1Wire>, Error> {
    use super::DailyEvidenceKindV1 as Kind;
    use crate::contract::{ResponseKind, SourceFileReference};
    let mut known = std::collections::BTreeSet::new();
    let mut files = sealed.observation().files().to_vec();
    let mut evidence_bytes = 0u64;
    for file in &files {
        let (_, bytes) =
            super::contract::canonical_file_evidence(file).map_err(|_| Error::InvalidEvidence)?;
        evidence_bytes = evidence_bytes
            .checked_add(bytes.len() as u64)
            .ok_or(Error::LimitExceeded)?;
        known.insert(bytes);
    }
    let mut calendars = Vec::new();
    for scope in snapshots.scopes() {
        for batch in scope.batches() {
            if !snapshots.matches_entitlement(batch) {
                continue;
            }
            for file in batch.files() {
                if !matches!(
                    file.kind(),
                    ResponseKind::Bars
                        | ResponseKind::Reference
                        | ResponseKind::Calendar
                        | ResponseKind::CorporateActions
                ) {
                    continue;
                }
                let source = super::verify::resolve_original(
                    snapshots,
                    &SourceFileReference {
                        provider: batch.entry().provider.clone(),
                        market: batch.entry().market.clone(),
                        batch_id: batch.batch_id(),
                        file_name: file.file_name().to_owned(),
                        content_hash: file.content_hash().clone(),
                        retrieved_at: batch.retrieved_at(),
                    },
                )?;
                if source.evidence().kind == Kind::Calendar {
                    calendars.push(source);
                    continue;
                }
                if known.contains(source.canonical_evidence_bytes()) {
                    continue;
                }
                let evidence = source.evidence();
                let relevant = match evidence.kind {
                    Kind::Bars | Kind::Reference => {
                        price_identity_touches(evidence, sealed.range_start(), sealed.range_end())?
                    }
                    Kind::Calendar => return Err(Error::InvalidEvidence),
                    Kind::CorporateActions => {
                        let (start, end, _) =
                            super::reconstruct::range_and_scope(&evidence.request.query)?;
                        if end < sealed.range_start() || start > sealed.range_end() {
                            false
                        } else {
                            super::reconstruct::action_page_identity(evidence)?;
                            true
                        }
                    }
                };
                if relevant {
                    evidence_bytes = evidence_bytes
                        .checked_add(source.canonical_evidence_bytes().len() as u64)
                        .ok_or(Error::LimitExceeded)?;
                    if files.len() as u64 >= fixed_limits().max_file_evidence_per_session
                        || evidence_bytes > fixed_limits().max_manifest_bytes
                    {
                        return Err(Error::LimitExceeded);
                    }
                    known.insert(source.canonical_evidence_bytes().to_vec());
                    files.push(evidence.clone());
                }
            }
        }
    }
    for projection in super::calendar::project_calendar_candidates(
        &calendars,
        sealed.owner(),
        sealed.policy_sha256(),
    )? {
        if known.contains(projection.canonical_file_bytes())
            || !projection.days().iter().any(|day| {
                day.session_date() >= sealed.range_start()
                    && day.session_date() <= sealed.range_end()
            })
        {
            continue;
        }
        evidence_bytes = evidence_bytes
            .checked_add(projection.canonical_file_bytes().len() as u64)
            .ok_or(Error::LimitExceeded)?;
        if files.len() as u64 >= fixed_limits().max_file_evidence_per_session
            || evidence_bytes > fixed_limits().max_manifest_bytes
        {
            return Err(Error::LimitExceeded);
        }
        known.insert(projection.canonical_file_bytes().to_vec());
        files.push(projection.source_file().clone());
    }
    Ok(files)
}

fn price_identity_touches(
    evidence: &DailyFileEvidenceV1Wire,
    start: TradingDate,
    end: TradingDate,
) -> Result<bool, Error> {
    let original_date = evidence
        .copied_from
        .last()
        .map_or(evidence.batch_date, |origin| origin.batch_date);
    if original_date < start || original_date > end {
        return Ok(false);
    }
    let symbol = evidence
        .request
        .query
        .iter()
        .find(|(key, _)| key == "FID_INPUT_ISCD")
        .map(|(_, value)| value.as_str())
        .ok_or(Error::InvalidIdentity)?;
    if symbol.len() != 6 || !symbol.bytes().all(|value| value.is_ascii_digit()) {
        return Err(Error::InvalidIdentity);
    }
    if !crate::providers::kis::KR_ETF_CORE_SYMBOLS.contains(&symbol) {
        return Ok(false);
    }
    super::contract::validate_file_request_for_symbol(
        evidence,
        evidence.kind,
        symbol,
        original_date,
    )
    .map_err(|_| Error::InvalidEvidence)?;
    Ok(true)
}

fn known_integrity_error(error: Error) -> bool {
    use super::source::DailyEvidenceError;
    matches!(
        error,
        Error::Snapshot(DailyEvidenceError::NotCommitted | DailyEvidenceError::ReferenceMismatch)
            | Error::CopyMismatch
            | Error::CopyCycle
            | Error::CopyDepth
    )
}

fn expected_source_event(
    record: &FrontierRecord,
    expected: &DailyFileEvidenceV1Wire,
    observed: Option<&DailyFileEvidenceV1Wire>,
    code: super::observation::DailyBlockCode,
) -> Result<super::observation::CanonicalSourceEvent, Error> {
    use super::observation::{DailySourceKind, canonical_source_event};
    let mut files = vec![expected.clone()];
    if let Some(current) = observed {
        files.push(current.clone());
    }
    let observation = canonical_source_observation(&files)?;
    let kind = match expected.kind {
        super::DailyEvidenceKindV1::Bars => DailySourceKind::Bar,
        super::DailyEvidenceKindV1::Reference => DailySourceKind::Reference,
        super::DailyEvidenceKindV1::Calendar => DailySourceKind::Calendar,
        super::DailyEvidenceKindV1::CorporateActions => DailySourceKind::Action,
    };
    canonical_source_event(
        record.owner,
        kind,
        code,
        record.range_start,
        record.range_end,
        &observation,
    )
}

fn check_refresh_budget(bytes: u64, files: u64, passes: u64) -> Result<(), Error> {
    if passes == 0 || passes > fixed_limits().max_sessions {
        return Err(Error::InvalidContext);
    }
    if bytes.checked_mul(passes).ok_or(Error::LimitExceeded)? > fixed_limits().max_bars_bytes
        || files.checked_mul(passes).ok_or(Error::LimitExceeded)?
            > fixed_limits().max_file_evidence_per_session
    {
        return Err(Error::LimitExceeded);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const GOLDEN_FILE: &str = r#"{"provider":"kis","market":"kr","batch_id":"00000000-0000-4000-8000-000000000001","batch_date":"2026-10-12","batch_json_sha256":"sha256:0000000000000000000000000000000000000000000000000000000000000001","manifest_line_sha256":"sha256:0000000000000000000000000000000000000000000000000000000000000002","file_name":"calendar.json","kind":"calendar","wire_sha256":"sha256:0000000000000000000000000000000000000000000000000000000000000003","size_bytes":17,"request":{"endpoint":"/uapi/domestic-stock/v1/quotations/chk-holiday","query":[["BASS_DT","20261012"],["CTX_AREA_FK",""],["CTX_AREA_NK",""]],"headers":[["appkey","[REDACTED]"],["appsecret","[REDACTED]"],["authorization","[REDACTED]"],["tr_cont",""],["tr_id","CTCA0903R"]],"mode":"credentialed"},"response_continuation":"M","retrieved_at":"2026-10-12T07:31:00.000000000Z","retrieval_precision":"batch","original_retrieved_at":"2026-10-12T07:31:00.000000000Z","original_retrieval_precision":"batch","copied_from":[]}"#;
    const GOLDEN_OBSERVATION: &str =
        "sha256:4b8e4195b3b84233b98859c8cc210b01ffd77c75a1998c219bc3e5d828c232d7";

    fn hash(value: &str) -> ContentHash {
        ContentHash::from_bytes(value.as_bytes())
    }
    fn date(value: &str) -> TradingDate {
        TradingDate::parse(value).unwrap()
    }
    fn observation() -> CanonicalSourceObservation {
        decode_observation(
            format!("owner-beta-daily-source-observation-v1\n[{GOLDEN_FILE}]").as_bytes(),
        )
        .unwrap()
    }
    fn record(observation: &CanonicalSourceObservation) -> FrontierRecord {
        FrontierRecord {
            schema: SCHEMA.into(),
            owner: Uuid::parse_str("00000000-0000-4000-8000-000000000099").unwrap(),
            policy_sha256: hash("policy"),
            admission_sha256: hash("admission"),
            parent_frontier_sha256: None,
            range_start: date("2026-10-12"),
            range_end: date("2026-10-12"),
            observation_sha256: observation.sha256().clone(),
            source_file_count: 1,
        }
    }
    #[test]
    fn s5e_frontier_preserves_exact_d5_observation_hash() {
        let observed = observation();
        assert_eq!(observed.sha256().as_str(), GOLDEN_OBSERVATION);
        let r = record(&observed);
        let bytes = encode_record(&r, &observed).unwrap();
        let decoded = decode_record(&bytes, &observed).unwrap();
        assert!(decoded == r);
        assert_eq!(encode_record(&decoded, &observed).unwrap(), bytes);
        assert!(!bytes.ends_with(b"\n"));
        assert!(!observed.preimage().ends_with(b"\n"));
    }
    #[test]
    fn s5e_frontier_record_rejects_unknown_duplicate_and_noncanonical_fields() {
        let observed = observation();
        let bytes =
            String::from_utf8(encode_record(&record(&observed), &observed).unwrap()).unwrap();
        for text in [
            format!("{bytes}\n"),
            bytes.replacen('{', "{ ", 1),
            bytes.replacen('{', "{\"unknown\":1,", 1),
            bytes.replacen('{', "{\"source_file_count\":1,", 1),
        ] {
            assert_eq!(
                decode_record(text.as_bytes(), &observed).err(),
                Some(Error::InvalidEvidence)
            );
        }
    }
    #[test]
    fn s5e_frontier_record_binds_owner_range_hash_and_count() {
        let observed = observation();
        for case in 0..5 {
            let mut r = record(&observed);
            match case {
                0 => r.owner = Uuid::nil(),
                1 => r.range_end = date("2026-10-11"),
                2 => r.observation_sha256 = hash("other"),
                3 => r.source_file_count = 2,
                _ => r.schema = "other".into(),
            }
            assert_eq!(encode_record(&r, &observed), Err(Error::InvalidEvidence));
        }
    }
    #[test]
    fn s5e_frontier_observation_rejects_wrong_domain_empty_duplicates_and_layout() {
        for bytes in [
            format!("other\n[{GOLDEN_FILE}]"),
            "owner-beta-daily-source-observation-v1\n[]".into(),
            format!("owner-beta-daily-source-observation-v1\n[{GOLDEN_FILE},{GOLDEN_FILE}]"),
            format!("owner-beta-daily-source-observation-v1\n[ {GOLDEN_FILE}]"),
            format!("owner-beta-daily-source-observation-v1\n[{GOLDEN_FILE}]\n"),
            format!(
                "owner-beta-daily-source-observation-v1\n[{}]",
                GOLDEN_FILE.replacen('{', "{\"unknown\":1,", 1)
            ),
        ] {
            assert!(decode_observation(bytes.as_bytes()).is_err());
        }
    }
    #[test]
    fn s5e_frontier_changed_claim_cannot_reuse_old_record_binding() {
        let observed = observation();
        let bytes = encode_record(&record(&observed), &observed).unwrap();
        let changed = format!(
            "owner-beta-daily-source-observation-v1\n[{}]",
            GOLDEN_FILE.replace("\"size_bytes\":17", "\"size_bytes\":18")
        );
        let changed = decode_observation(changed.as_bytes()).unwrap();
        assert_ne!(changed.sha256(), observed.sha256());
        assert_eq!(
            decode_record(&bytes, &changed).err(),
            Some(Error::InvalidEvidence)
        );
    }
    #[test]
    fn s5e_frontier_civil_start_preserves_closed_days() {
        assert_eq!(next_day(date("2026-08-28")).unwrap(), date("2026-08-29"));
        assert_eq!(next_day(date("2028-02-28")).unwrap(), date("2028-02-29"));
        assert_eq!(next_day(date("2026-12-31")).unwrap(), date("2027-01-01"));
    }
    #[test]
    fn s5e_frontier_binding_rejects_owner_policy_admission_and_range_substitution() {
        let (manifest, pins, _) = super::super::continuation::tests::fixture();
        let observed = observation();
        let mut original = record(&observed);
        original.policy_sha256 = pins.policy_sha256.clone();
        original.admission_sha256 = pins.admission_sha256.clone();
        original.range_start = date("2026-08-29");
        original.range_end = manifest.range_end;
        assert_eq!(
            validate_binding(&original, original.owner, &manifest, &pins, None),
            Ok(())
        );
        for case in 0..6 {
            let mut changed =
                decode_record(&encode_record(&original, &observed).unwrap(), &observed).unwrap();
            match case {
                0 => changed.owner = Uuid::nil(),
                1 => changed.policy_sha256 = hash("wrong-policy"),
                2 => changed.admission_sha256 = hash("wrong-admission"),
                3 => changed.range_start = date("2026-08-30"),
                4 => changed.range_end = date("2026-09-01"),
                _ => changed.parent_frontier_sha256 = Some(hash("unexpected-parent")),
            }
            assert_eq!(
                validate_binding(&changed, original.owner, &manifest, &pins, None),
                Err(Error::InvalidContext)
            );
        }
    }
    #[test]
    fn s5e_frontier_child_binds_the_exact_predecessor_and_ignores_release() {
        let (previous, prior_pins, mut child) = super::super::continuation::tests::fixture();
        let observed = observation();
        let mut prior = record(&observed);
        prior.policy_sha256 = prior_pins.policy_sha256.clone();
        prior.admission_sha256 = prior_pins.admission_sha256.clone();
        prior.range_start = date("2026-08-29");
        prior.range_end = previous.range_end;
        let pins = super::super::DailyPricePinsV1Wire {
            content_sha256: child.content_sha256.clone(),
            manifest_sha256: hash("child-manifest"),
            admission_sha256: hash("child-admission"),
            policy_sha256: prior_pins.policy_sha256.clone(),
        };
        let mut current = record(&observed);
        current.policy_sha256 = pins.policy_sha256.clone();
        current.admission_sha256 = pins.admission_sha256.clone();
        current.range_start = prior.range_start;
        current.range_end = child.range_end;
        current.parent_frontier_sha256 = Some(prior.observation_sha256.clone());
        assert_eq!(
            validate_binding(&current, current.owner, &child, &pins, Some(&prior)),
            Ok(())
        );
        child.code_commit = domain::CodeCommit::parse(&"c".repeat(40)).unwrap();
        assert_eq!(
            validate_binding(&current, current.owner, &child, &pins, Some(&prior)),
            Ok(())
        );
        assert_eq!(
            validate_binding(&current, current.owner, &child, &pins, None),
            Err(Error::InvalidContext)
        );
        for case in 0..6 {
            let mut wrong =
                decode_record(&encode_record(&prior, &observed).unwrap(), &observed).unwrap();
            match case {
                0 => wrong.observation_sha256 = hash("wrong-observation"),
                1 => wrong.admission_sha256 = hash("wrong-admission"),
                2 => wrong.owner = Uuid::nil(),
                3 => wrong.policy_sha256 = hash("wrong-policy"),
                4 => wrong.range_start = date("2026-08-30"),
                _ => wrong.range_end = current.range_end,
            }
            assert_eq!(
                validate_binding(&current, current.owner, &child, &pins, Some(&wrong)),
                Err(Error::InvalidContext)
            );
        }
    }
    #[test]
    fn s5e_refresh_work_is_bounded_before_repeated_discovery() {
        let limits = fixed_limits();
        assert_eq!(
            check_refresh_budget(
                limits.max_bars_bytes,
                limits.max_file_evidence_per_session,
                1
            ),
            Ok(())
        );
        assert_eq!(
            check_refresh_budget(
                limits.max_bars_bytes / 2,
                limits.max_file_evidence_per_session / 2,
                2
            ),
            Ok(())
        );
        for (bytes, files, passes) in [
            (limits.max_bars_bytes + 1, 1, 1),
            (1, limits.max_file_evidence_per_session + 1, 1),
            (u64::MAX, 1, 2),
            (1, u64::MAX, 2),
        ] {
            assert_eq!(
                check_refresh_budget(bytes, files, passes),
                Err(Error::LimitExceeded)
            );
        }
        assert_eq!(check_refresh_budget(1, 1, 0), Err(Error::InvalidContext));
        assert_eq!(
            check_refresh_budget(1, 1, limits.max_sessions + 1),
            Err(Error::InvalidContext)
        );
    }
    #[test]
    fn s5e_unattributable_failures_do_not_become_permanent_source_events() {
        use super::super::source::DailyEvidenceError;
        for error in [
            Error::Snapshot(DailyEvidenceError::NotCommitted),
            Error::Snapshot(DailyEvidenceError::ReferenceMismatch),
            Error::CopyMismatch,
            Error::CopyCycle,
            Error::CopyDepth,
        ] {
            assert!(known_integrity_error(error));
        }
        for error in [
            Error::Snapshot(DailyEvidenceError::Io),
            Error::Snapshot(DailyEvidenceError::ScopeMismatch),
            Error::Snapshot(DailyEvidenceError::SourceScanLimit),
            Error::InvalidProviderJson,
            Error::LimitExceeded,
            Error::InvalidContext,
        ] {
            assert!(!known_integrity_error(error));
        }
    }
    #[test]
    fn s5e_expected_missing_source_uses_its_known_commitment_not_an_error_hash() {
        use super::super::observation::{DailyBlockCode, DailySourceKind};
        let observed = observation();
        let r = record(&observed);
        let event = expected_source_event(
            &r,
            &observed.files()[0],
            None,
            DailyBlockCode::SourceIntegrity,
        )
        .unwrap();
        assert_eq!(event.kind(), DailySourceKind::Calendar);
        assert_eq!(event.code(), DailyBlockCode::SourceIntegrity);
        assert_eq!(event.observation_sha256(), observed.sha256());
        let expected = format!(
            "owner-beta-daily-source-event-v1\n00000000-0000-4000-8000-000000000099\nCALENDAR\nSOURCE_INTEGRITY\n2026-10-12\n2026-10-12\n{GOLDEN_OBSERVATION}\n"
        );
        assert_eq!(event.preimage(), expected.as_bytes());
        assert_eq!(
            event.sha256(),
            &ContentHash::from_bytes(expected.as_bytes())
        );
    }
    #[test]
    fn s5e_changed_expected_source_preserves_both_variants_without_release_identity() {
        use super::super::observation::DailyBlockCode;
        let observed = observation();
        let mut r = record(&observed);
        let expected = &observed.files()[0];
        let mut changed = expected.clone();
        changed.size_bytes += 1;
        let union = canonical_source_observation(&[expected.clone(), changed.clone()]).unwrap();
        let first = expected_source_event(
            &r,
            expected,
            Some(&changed),
            DailyBlockCode::SealedSourceChanged,
        )
        .unwrap();
        assert_eq!(union.files().len(), 2);
        assert_eq!(first.observation_sha256(), union.sha256());
        r.admission_sha256 = hash("other-release-admission");
        let second = expected_source_event(
            &r,
            expected,
            Some(&changed),
            DailyBlockCode::SealedSourceChanged,
        )
        .unwrap();
        assert_eq!(first.preimage(), second.preimage());
        assert_eq!(first.sha256(), second.sha256());
    }

    fn price_identity() -> DailyFileEvidenceV1Wire {
        let mut file: DailyFileEvidenceV1Wire = serde_json::from_str(GOLDEN_FILE).unwrap();
        file.kind = super::super::DailyEvidenceKindV1::Reference;
        file.file_name = "ref-069500.json".into();
        file.request.endpoint = "/uapi/domestic-stock/v1/quotations/inquire-price".into();
        file.request.query = vec![
            ("FID_COND_MRKT_DIV_CODE".into(), "J".into()),
            ("FID_INPUT_ISCD".into(), "069500".into()),
        ];
        file.request
            .headers
            .iter_mut()
            .find(|(key, _)| key == "tr_id")
            .unwrap()
            .1 = "FHKST01010100".into();
        file
    }
    #[test]
    fn s5e_new_price_identity_checks_original_date_and_etf_scope() {
        let mut file = price_identity();
        let start = date("2026-10-12");
        assert_eq!(price_identity_touches(&file, start, start), Ok(true));
        assert_eq!(
            price_identity_touches(&file, date("2026-10-13"), date("2026-10-15")),
            Ok(false)
        );
        assert_eq!(
            price_identity_touches(&file, date("2026-10-09"), date("2026-10-11")),
            Ok(false)
        );
        file.request.query[1].1 = "000020".into();
        assert_eq!(price_identity_touches(&file, start, start), Ok(false));
        file.request.query[1].1 = "bad".into();
        assert_eq!(
            price_identity_touches(&file, start, start),
            Err(Error::InvalidIdentity)
        );
        file = price_identity();
        file.request.endpoint = "/unapproved".into();
        assert_eq!(
            price_identity_touches(&file, start, start),
            Err(Error::InvalidEvidence)
        );
    }
    #[test]
    fn s5e_later_copy_partition_does_not_hide_a_sealed_original_date() {
        let original = price_identity();
        let mut copy = original.clone();
        copy.copied_from.push(super::super::DailyFileOriginV1Wire {
            provider: original.provider.clone(),
            market: original.market.clone(),
            batch_id: original.batch_id,
            batch_date: original.batch_date,
            batch_json_sha256: original.batch_json_sha256.clone(),
            manifest_line_sha256: original.manifest_line_sha256.clone(),
            file_name: original.file_name.clone(),
            kind: original.kind,
            wire_sha256: original.wire_sha256.clone(),
            size_bytes: original.size_bytes,
            request: original.request.clone(),
            response_continuation: original.response_continuation.clone(),
            retrieved_at: original.retrieved_at,
            retrieval_precision: original.retrieval_precision.clone(),
        });
        copy.batch_id = domain::BatchId::from_uuid(Uuid::from_u128(2));
        copy.batch_date = date("2026-10-15");
        copy.retrieved_at =
            super::super::DailyWireTimestampV1::parse("2026-10-15T07:31:00.000000000Z").unwrap();
        assert_eq!(
            price_identity_touches(&copy, date("2026-10-12"), date("2026-10-12")),
            Ok(true)
        );
        assert_eq!(
            price_identity_touches(&copy, date("2026-10-15"), date("2026-10-15")),
            Ok(false)
        );
    }

    #[test]
    fn i4g_frontier_navigation_is_canonical_record_only() {
        let observation = observation();
        let record = record(&observation);
        let bytes = serde_json::to_vec(&record).unwrap();
        let nav = decode_frontier_navigation(&bytes, observation.sha256()).unwrap();
        assert_eq!(nav.owner(), record.owner);
        assert_eq!(nav.policy_sha256(), &record.policy_sha256);
        assert_eq!(nav.admission_sha256(), &record.admission_sha256);
        assert_eq!(
            nav.parent_frontier_sha256(),
            record.parent_frontier_sha256.as_ref()
        );
        assert_eq!(nav.range_start(), record.range_start);
        assert_eq!(nav.range_end(), record.range_end);
        // No observation body or sealed input was supplied by this metadata test.
    }

    #[test]
    fn i4g_frontier_navigation_rejects_noncanonical_unknown_duplicate_and_hash() {
        let observation = observation();
        let bytes = serde_json::to_vec(&record(&observation)).unwrap();
        let mut whitespace = bytes.clone();
        whitespace.push(b' ');
        let unknown = String::from_utf8(bytes.clone())
            .unwrap()
            .replacen("{", "{\"extra\":0,", 1)
            .into_bytes();
        let duplicate = String::from_utf8(bytes.clone())
            .unwrap()
            .replacen(
                "{",
                "{\"schema\":\"owner-beta-daily-source-frontier-v1\",",
                1,
            )
            .into_bytes();
        for changed in [whitespace, unknown, duplicate] {
            assert!(matches!(
                decode_frontier_navigation(&changed, observation.sha256()),
                Err(Error::InvalidEvidence)
            ));
        }
        assert!(matches!(
            decode_frontier_navigation(&bytes, &hash("wrong-observation")),
            Err(Error::InvalidEvidence)
        ));
    }

    #[test]
    fn i4g_frontier_navigation_rejects_invalid_record_envelope_before_planning() {
        let observation = observation();
        for change in 0..3 {
            let mut changed = record(&observation);
            match change {
                0 => changed.schema = "other".into(),
                1 => changed.owner = Uuid::nil(),
                _ => changed.range_end = date("2000-01-01"),
            }
            let bytes = serde_json::to_vec(&changed).unwrap();
            assert!(matches!(
                decode_frontier_navigation(&bytes, observation.sha256()),
                Err(Error::InvalidEvidence)
            ));
        }
        let oversized = vec![0; fixed_limits().max_admission_bytes as usize + 1];
        assert!(matches!(
            decode_frontier_navigation(&oversized, observation.sha256()),
            Err(Error::LimitExceeded)
        ));
    }
}
