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
    pub(super) fn opened(&self) -> &OpenedDailyCandidate {
        &self.opened
    }
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
    let mode = sealed::Legacy { root, base, policy };
    let mut progress = sealed::Progress::new();
    sealed::first(&mode, expected, &mut progress, |progress| {
        let (opened, admission_bytes) = progress.held.take().expect("returned leaf");
        SealedHistoricalDailyInput {
            base,
            policy,
            opened,
            admission: progress.admission.take().expect("returned admission"),
            admission_bytes,
            pins: expected.clone(),
            bars: progress.bars.take().expect("returned bars"),
        }
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

pub(super) fn counts(
    manifest: &DailyPriceManifestV1Wire,
) -> Result<DailyAdmissionCountsV1Wire, Error> {
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

// C15 shared sealed policy and checked custody.

// The backend is sealed inside admission: siblings share policy, not authority
// constructors. Legacy representations and allocation points remain concrete.
pub(super) mod sealed {
    use super::super::{DailyNormalizationProofV1Wire, DailyPriceManifestV1Wire};
    use super::*;

    pub trait LeafView {
        fn manifest(&self) -> &DailyPriceManifestV1Wire;
        fn normalization(&self) -> &DailyNormalizationProofV1Wire;
        fn admission_bytes(&self) -> &[u8];
        fn bars_ndjson(&self) -> &[u8];
    }
    pub struct LegacyView<'a>(pub &'a OpenedDailyCandidate, pub &'a [u8]);
    impl LeafView for LegacyView<'_> {
        fn manifest(&self) -> &DailyPriceManifestV1Wire {
            self.0.manifest()
        }
        fn normalization(&self) -> &DailyNormalizationProofV1Wire {
            self.0.normalization()
        }
        fn admission_bytes(&self) -> &[u8] {
            self.1
        }
        fn bars_ndjson(&self) -> &[u8] {
            self.0.bars_ndjson()
        }
    }

    #[cfg(any(target_os = "linux", test))]
    pub fn same_owners<R, H>(
        root: &R,
        stored_root: &R,
        historical: &H,
        stored_historical: &H,
    ) -> bool {
        std::ptr::eq(root, stored_root) && std::ptr::eq(historical, stored_historical)
    }
    pub fn retain<T, E>(slot: &mut Option<T>, result: Result<T, E>) -> Result<(), E> {
        *slot = Some(result?);
        Ok(())
    }
    pub struct Progress<H> {
        pub held: Option<H>,
        pub admission: Option<DailyPriceAdmissionV1Wire>,
        pub bars: Option<Vec<HistoricalPriceOnlyBar>>,
    }
    impl<H> Progress<H> {
        pub fn new() -> Self {
            Self {
                held: None,
                admission: None,
                bars: None,
            }
        }
    }
    impl<H> Drop for Progress<H> {
        fn drop(&mut self) {
            drop(self.bars.take());
            drop(self.admission.take());
            drop(self.held.take());
        }
    }
    pub trait Mode {
        type Held;
        type Error;
        type View<'a>: LeafView
        where
            Self: 'a;
        fn policy(&self) -> &PinnedDailyPolicy;
        fn base(&self) -> Result<&ApprovedHistoricalPriceOnlyArtifact, Self::Error>;
        fn read(&self, pins: &DailyPricePinsV1Wire) -> Result<Self::Held, Self::Error>;
        fn view<'a>(&'a self, held: &'a Self::Held) -> Result<Self::View<'a>, Self::Error>;
        fn revalidate(&self, held: &Self::Held) -> Result<(), Self::Error>;
        fn policy_result<T>(&self, value: Result<T, Error>) -> Result<T, Self::Error>;
    }
    pub struct Legacy<'base, 'root> {
        pub root: &'root DailyAdmissionRoot,
        pub base: &'base ApprovedHistoricalPriceOnlyArtifact,
        pub policy: &'base PinnedDailyPolicy,
    }
    impl Mode for Legacy<'_, '_> {
        type Held = (OpenedDailyCandidate, Vec<u8>);
        type Error = Error;
        type View<'a>
            = LegacyView<'a>
        where
            Self: 'a;
        fn policy(&self) -> &PinnedDailyPolicy {
            self.policy
        }
        fn base(&self) -> Result<&ApprovedHistoricalPriceOnlyArtifact, Error> {
            Ok(self.base)
        }
        fn read(&self, expected: &DailyPricePinsV1Wire) -> Result<Self::Held, Error> {
            read_admission_files(
                self.root,
                &expected.admission_sha256,
                &expected.content_sha256,
            )
            .map_err(Error::Artifact)
        }
        fn view<'a>(&'a self, held: &'a Self::Held) -> Result<LegacyView<'a>, Error> {
            Ok(LegacyView(&held.0, &held.1))
        }
        fn revalidate(&self, held: &Self::Held) -> Result<(), Error> {
            held.0.revalidate().map_err(Error::Artifact)
        }
        fn policy_result<T>(&self, value: Result<T, Error>) -> Result<T, Error> {
            value
        }
    }
    pub trait Parent<M: Mode> {
        type View<'a>: LeafView
        where
            Self: 'a,
            M: 'a;
        fn revalidate(&self, mode: &M) -> Result<(), M::Error>;
        fn view<'a>(&'a self, mode: &'a M) -> Result<Self::View<'a>, M::Error>;
        fn pins(&self) -> &DailyPricePinsV1Wire;
        fn bars(&self) -> &[HistoricalPriceOnlyBar];
    }

    pub fn first<M: Mode, R>(
        mode: &M,
        expected: &DailyPricePinsV1Wire,
        progress: &mut Progress<M::Held>,
        finish: impl FnOnce(&mut Progress<M::Held>) -> R,
    ) -> Result<R, M::Error> {
        retain(&mut progress.held, mode.read(expected))?;
        let opened = mode.view(progress.held.as_ref().expect("returned leaf"))?;
        let admission_bytes = opened.admission_bytes();
        let admission = mode.policy_result(
            parse_daily_price_admission_v1(admission_bytes).map_err(|_| Error::InvalidCandidate),
        )?;
        retain(&mut progress.admission, Ok::<_, M::Error>(admission))?;
        let admission = progress.admission.as_ref().expect("returned admission");
        if mode.policy_result(
            encode_daily_price_admission_v1(admission).map_err(|_| Error::InvalidCandidate),
        )? != admission_bytes
        {
            return mode.policy_result(Err(Error::InvalidCandidate));
        }
        mode.policy_result(
            validate_daily_price_wire_bundle_v1(
                mode.policy().wire(),
                opened.manifest(),
                opened.normalization(),
                admission,
                expected,
            )
            .map_err(|_| Error::InvalidCandidate),
        )?;
        let prefix = mode.policy_result(
            ProtectedHistoricalPrefix::from_approved_v3(mode.base()?, mode.policy().wire())
                .map_err(Error::Evidence),
        )?;
        let bars = mode.policy_result(
            read_first_admission_bars(&prefix, opened.manifest(), opened.bars_ndjson())
                .map_err(Error::Evidence),
        )?;
        retain(&mut progress.bars, Ok::<_, M::Error>(bars))?;
        mode.revalidate(progress.held.as_ref().expect("returned leaf"))?;
        drop(opened);
        Ok(finish(progress))
    }

    // Metadata/discovery and the forward replay both use this pending slot.
    // Retiring a failed next leaf precedes retiring the previous live leaf.
    pub struct Pending<T> {
        pub current: Option<T>,
        pub next: Option<T>,
    }
    impl<T> Pending<T> {
        pub fn new() -> Self {
            Self {
                current: None,
                next: None,
            }
        }
        #[cfg(any(target_os = "linux", test))]
        pub fn drain(&mut self, mut retire: impl FnMut(T)) {
            if let Some(next) = self.next.take() {
                retire(next);
            }
            if let Some(previous) = self.current.take() {
                retire(previous);
            }
        }
    }
    impl<T> Drop for Pending<T> {
        fn drop(&mut self) {
            drop(self.next.take());
            drop(self.current.take());
        }
    }
    #[cfg(any(target_os = "linux", test))]
    pub fn promote<T>(pending: &mut Pending<T>, mut retire: impl FnMut(T, bool) -> bool) -> bool {
        if let Some(previous) = pending.current.take()
            && !retire(previous, true)
        {
            if let Some(next) = pending.next.take() {
                retire(next, false);
            }
            return false;
        }
        pending.current = pending.next.take();
        true
    }
    pub fn input<P, E>(parent: P, revalidate: impl FnOnce(&P) -> Result<(), E>) -> Result<P, E> {
        revalidate(&parent)?;
        Ok(parent)
    }
}

#[cfg(target_os = "linux")]
pub(super) mod checked {
    use super::super::artifact::{
        CheckedDailyAdmissionFiles, CheckedDailyAdmissionRoot, CheckedDailyAdmissionView,
        CheckedDailyArtifactFailure, CheckedDailyCompletion, DailyArtifactFirstKind,
        DailyUseFailure,
    };
    use super::sealed::{LeafView, Mode, Progress};
    use super::*;
    use crate::checked_close::{Attempt, Failure, Gap, Report};
    use crate::historical_price_only_approval::CheckedHistoricalApproval;
    use crate::historical_price_only_v3_artifact::HistoricalUseFailure;
    use crate::raw_custody::RawInvocation;
    use std::convert::Infallible;
    use std::fmt;

    pub type Unit = Result<(), Failure<Infallible>>;
    pub struct Local {
        expected: Unit,
        captured: Unit,
    }
    impl Local {
        pub fn expected(&self) -> &Unit {
            &self.expected
        }
        pub fn captured(&self) -> &Unit {
            &self.captured
        }
        pub fn is_clean(&self) -> bool {
            self.expected.is_ok() && self.captured.is_ok()
        }
        fn report(&self) -> Option<&Report> {
            self.expected
                .as_ref()
                .err()
                .or(self.captured.as_ref().err())
                .map(Failure::report)
        }
    }

    pub fn local_scopes(
        invocation: &RawInvocation<'_>,
        stored_expected: &Attempt,
        stored_captured: &Attempt,
        expected: &Attempt,
        captured: &Attempt,
    ) -> Local {
        for token in [stored_expected, stored_captured, expected, captured] {
            if !invocation.attempt().same_scope(token) {
                invocation.attempt().gap(Gap::ScopeMismatch);
                token.gap(Gap::ScopeMismatch);
            }
        }
        let expected = invocation.settle_local_in(expected, Ok::<(), Infallible>(()));
        let captured = invocation.settle_local_in(captured, Ok::<(), Infallible>(()));
        Local { expected, captured }
    }
    pub struct JoinFailure {
        local: Local,
        root: Result<(), DailyUseFailure>,
        history: Result<(), HistoricalUseFailure>,
        leaf: Option<Result<(), DailyUseFailure>>,
        relation: Unit,
    }
    impl JoinFailure {
        fn first_kind(&self) -> Option<FirstKind> {
            if !self.local.is_clean() || self.root.is_err() {
                return Some(FirstKind::Scope);
            }
            if let Err(history) = &self.history {
                let checks = history.checks();
                return if checks.expected().is_ok()
                    && checks.captured().is_ok()
                    && checks.execution_failure().is_some()
                {
                    None
                } else {
                    Some(FirstKind::Scope)
                };
            }
            if self.leaf.as_ref().is_some_and(Result::is_err) || self.relation.is_err() {
                Some(FirstKind::Scope)
            } else {
                None
            }
        }
        pub fn local(&self) -> &Local {
            &self.local
        }
        pub fn root(&self) -> &Result<(), DailyUseFailure> {
            &self.root
        }
        pub fn historical(&self) -> &Result<(), HistoricalUseFailure> {
            &self.history
        }
        pub fn leaf(&self) -> Option<&Result<(), DailyUseFailure>> {
            self.leaf.as_ref()
        }
        pub fn relationship(&self) -> &Unit {
            &self.relation
        }
        pub fn source_report(&self) -> Option<&Report> {
            // Select the first failed result, not the first available report.
            // An absent report on that source never falls through to a later one.
            if !self.local.is_clean() {
                return self.local.report();
            }
            if let Err(error) = &self.root {
                return error.source_report();
            }
            if let Err(error) = &self.history {
                return error.checks().source_report();
            }
            if let Some(Err(error)) = &self.leaf {
                return error.source_report();
            }
            self.relation.as_ref().err().map(Failure::report)
        }
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum FirstKind {
        Scope,
        Artifact,
        Policy,
        Replay,
    }
    pub enum Source {
        Use(Box<JoinFailure>),
        Daily(Box<DailyUseFailure>),
        Artifact(Box<CheckedDailyArtifactFailure>),
        Policy(Box<Failure<Error>>),
        Replay(Box<Failure<super::super::replay::DailyReplayError>>),
        Prior(Box<CheckedDailyPolicyFailure>),
        Joined(Box<(Result<(), Source>, Result<(), Source>)>),
        Late,
    }
    impl Source {
        pub fn execution_failure(&self) -> Option<&crate::ExecutionGateFailure> {
            match self {
                Self::Artifact(f) => f.execution_failure(),
                Self::Prior(f) => f.execution_failure(),
                Self::Joined(pair) => pair
                    .0
                    .as_ref()
                    .err()
                    .and_then(Source::execution_failure)
                    .or_else(|| pair.1.as_ref().err().and_then(Source::execution_failure)),
                Self::Use(f) => f
                    .historical()
                    .as_ref()
                    .err()
                    .and_then(|f| f.checks().execution_failure()),
                _ => None,
            }
        }
        pub fn first_kind(&self) -> Option<FirstKind> {
            match self {
                Self::Use(first) => first.first_kind(),
                Self::Daily(_) => Some(FirstKind::Scope),
                Self::Artifact(e) => match e.first_kind() {
                    Some(DailyArtifactFirstKind::Scope | DailyArtifactFirstKind::Binding) => {
                        Some(FirstKind::Scope)
                    }
                    Some(DailyArtifactFirstKind::Artifact) => Some(FirstKind::Artifact),
                    None => None,
                },
                Self::Policy(e) => e.body().map(|_| FirstKind::Policy),
                Self::Replay(e) => e.body().map(|_| FirstKind::Replay),
                Self::Prior(e) => e.first_kind(),
                Self::Joined(pair) => {
                    if let Err(e) = &pair.0 {
                        e.first_kind()
                    } else {
                        pair.1.as_ref().err().and_then(Source::first_kind)
                    }
                }
                Self::Late => None,
            }
        }
        pub fn report(&self) -> Option<&Report> {
            match self {
                Self::Use(e) => e.source_report(),
                Self::Daily(e) => e.source_report(),
                Self::Artifact(e) => e.source_report(),
                Self::Policy(_) | Self::Replay(_) => None,
                Self::Prior(e) => e.source_report(),
                Self::Joined(pair) => {
                    if let Err(e) = &pair.0 {
                        e.report()
                    } else {
                        pair.1.as_ref().err().and_then(Source::report)
                    }
                }
                Self::Late => None,
            }
        }
    }

    pub(in crate::daily_price_only) fn policy_source(
        invocation: &RawInvocation<'_>,
        expected: &Attempt,
        error: Error,
    ) -> Source {
        Source::Policy(Box::new(
            invocation
                .settle_local_in(expected, Err::<(), _>(error))
                .expect_err("original verification error"),
        ))
    }
    pub(in crate::daily_price_only) fn replay_source(
        invocation: &RawInvocation<'_>,
        expected: &Attempt,
        error: super::super::replay::DailyReplayError,
    ) -> Source {
        Source::Replay(Box::new(
            invocation
                .settle_local_in(expected, Err::<(), _>(error))
                .expect_err("original replay error"),
        ))
    }

    pub struct Returned {
        admission: Option<DailyPriceAdmissionV1Wire>,
        bars: Option<Vec<HistoricalPriceOnlyBar>>,
        pins: Option<DailyPricePinsV1Wire>,
    }
    impl Returned {
        pub fn admission(&self) -> Option<&DailyPriceAdmissionV1Wire> {
            self.admission.as_ref()
        }
        pub fn bars(&self) -> Option<&[HistoricalPriceOnlyBar]> {
            self.bars.as_deref()
        }
        pub fn pins(&self) -> Option<&DailyPricePinsV1Wire> {
            self.pins.as_ref()
        }
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum RetirementKind {
        Metadata,
        Pending,
        Predecessor,
        Tip,
    }
    pub struct Retirement {
        kind: RetirementKind,
        lower: CheckedDailyCompletion,
        returned: Option<Returned>,
    }
    impl Retirement {
        pub fn kind(&self) -> RetirementKind {
            self.kind
        }
        pub fn lower(&self) -> &CheckedDailyCompletion {
            &self.lower
        }
        pub fn returned(&self) -> Option<&Returned> {
            self.returned.as_ref()
        }
        pub fn is_clean(&self) -> bool {
            self.lower.is_clean()
        }
    }
    pub struct Evidence {
        pub original: Result<(), Source>,
        pub returned: Vec<Returned>,
        pub retirements: Vec<Retirement>,
        pub locals: Vec<Local>,
        pub prior: Vec<CheckedDailyPolicyCompletion>,
        pub secondary: Vec<super::super::replay::DailyReplayError>,
    }
    impl Evidence {
        pub fn new() -> Self {
            Self {
                original: Ok(()),
                returned: Vec::new(),
                retirements: Vec::new(),
                locals: Vec::new(),
                prior: Vec::new(),
                secondary: Vec::new(),
            }
        }
        pub fn is_clean(&self) -> bool {
            self.original.is_ok()
                && self.locals.iter().all(Local::is_clean)
                && self.retirements.iter().all(Retirement::is_clean)
                && self
                    .prior
                    .iter()
                    .all(CheckedDailyPolicyCompletion::is_clean)
        }
        pub fn metadata(&mut self, lower: CheckedDailyCompletion) -> bool {
            let clean = lower.is_clean();
            self.retirements.push(Retirement {
                kind: RetirementKind::Metadata,
                lower,
                returned: None,
            });
            clean
        }
        pub fn append(&mut self, completion: CheckedDailyPolicyCompletion) -> bool {
            let clean = completion.is_clean();
            self.prior.push(completion);
            clean
        }
    }
    pub struct CheckedDailyPolicyFailure {
        pub(in crate::daily_price_only) evidence: Box<Evidence>,
    }
    impl CheckedDailyPolicyFailure {
        pub fn execution_failure(&self) -> Option<&crate::ExecutionGateFailure> {
            self.evidence
                .original
                .as_ref()
                .err()
                .and_then(Source::execution_failure)
        }
        pub fn first_kind(&self) -> Option<FirstKind> {
            self.evidence
                .original
                .as_ref()
                .err()
                .and_then(Source::first_kind)
        }
        pub fn source_report(&self) -> Option<&Report> {
            self.evidence
                .original
                .as_ref()
                .err()
                .and_then(Source::report)
        }
        pub fn original_ok(&self) -> bool {
            self.evidence.original.is_ok()
        }
        pub fn source(&self) -> Option<&Source> {
            self.evidence.original.as_ref().err()
        }
        pub fn returned(&self) -> &[Returned] {
            &self.evidence.returned
        }
        pub fn retirements(&self) -> &[Retirement] {
            &self.evidence.retirements
        }
        pub fn local_results(&self) -> &[Local] {
            &self.evidence.locals
        }
        pub fn prior_completions(&self) -> &[CheckedDailyPolicyCompletion] {
            &self.evidence.prior
        }
        pub fn secondary_replay_errors(&self) -> &[super::super::replay::DailyReplayError] {
            &self.evidence.secondary
        }
    }
    pub struct CheckedDailyPolicyCompletion {
        pub(in crate::daily_price_only) evidence: Box<Evidence>,
    }
    impl CheckedDailyPolicyCompletion {
        pub fn is_clean(&self) -> bool {
            self.evidence.is_clean()
        }
        pub fn returned(&self) -> &[Returned] {
            &self.evidence.returned
        }
        pub fn retirements(&self) -> &[Retirement] {
            &self.evidence.retirements
        }
        pub fn local_results(&self) -> &[Local] {
            &self.evidence.locals
        }
        pub fn prior_completions(&self) -> &[CheckedDailyPolicyCompletion] {
            &self.evidence.prior
        }
    }
    pub struct Unwind {
        actual: Attempt,
        armed: bool,
    }
    impl Unwind {
        pub fn new(actual: &Attempt) -> Self {
            Self {
                actual: actual.clone(),
                armed: true,
            }
        }
        pub fn disarm(&mut self) {
            self.armed = false;
        }
    }
    impl Drop for Unwind {
        fn drop(&mut self) {
            if self.armed {
                self.actual.gap(Gap::Abandoned);
            }
        }
    }

    pub struct Checked<'env, 'inv, 'attempt, 'hist> {
        pub invocation: &'inv RawInvocation<'attempt>,
        expected: Attempt,
        captured: Attempt,
        pub root: &'env CheckedDailyAdmissionRoot<'inv, 'attempt>,
        pub historical: &'env CheckedHistoricalApproval<'hist>,
        pub policy: &'env PinnedDailyPolicy,
    }
    impl<'env, 'inv, 'attempt, 'hist> Checked<'env, 'inv, 'attempt, 'hist> {
        pub fn new(
            invocation: &'inv RawInvocation<'attempt>,
            expected: &Attempt,
            captured: &Attempt,
            root: &'env CheckedDailyAdmissionRoot<'inv, 'attempt>,
            historical: &'env CheckedHistoricalApproval<'hist>,
            policy: &'env PinnedDailyPolicy,
        ) -> Self {
            Self {
                invocation,
                expected: expected.clone(),
                captured: captured.clone(),
                root,
                historical,
                policy,
            }
        }
        pub fn local(&self, expected: &Attempt, captured: &Attempt) -> Local {
            local_scopes(
                self.invocation,
                &self.expected,
                &self.captured,
                expected,
                captured,
            )
        }
        pub fn tokens(&self) -> (&Attempt, &Attempt) {
            (&self.expected, &self.captured)
        }
        #[expect(
            clippy::result_large_err,
            reason = "Both actual lower use failures stay whole during unconditional scope joins."
        )]
        pub fn join<'a>(
            &'a self,
            expected: &Attempt,
            captured: &Attempt,
            held: Option<&'a CheckedDailyAdmissionFiles<'env, 'inv, 'attempt>>,
            relationship: bool,
        ) -> Result<
            (
                &'a ApprovedHistoricalPriceOnlyArtifact,
                Option<CheckedDailyAdmissionView<'a>>,
            ),
            Source,
        > {
            let local = self.local(expected, captured);
            let root = self.root.check_scope_in(expected, captured);
            let historical = self.historical.approved_in(expected, captured);
            let leaf = held.map(|h| h.view_in(expected, captured));
            // Object identity is checked after ALL actual owner scope visits.
            let relation = if relationship {
                Ok(())
            } else {
                Err(self.invocation.refused(Gap::ScopeMismatch))
            };
            let clean = local.is_clean()
                && root.is_ok()
                && historical.is_ok()
                && leaf.as_ref().is_none_or(Result::is_ok)
                && relation.is_ok();
            if !clean {
                return Err(Source::Use(Box::new(JoinFailure {
                    local,
                    root,
                    history: historical.map(|_| ()),
                    leaf: leaf.map(|r| r.map(|_| ())),
                    relation,
                })));
            }
            Ok((
                historical.expect("joined historical scope"),
                leaf.map(|r| r.expect("joined leaf scope")),
            ))
        }
        pub fn check(&self) -> Result<(), Source> {
            self.join(&self.expected, &self.captured, None, true)
                .map(|_| ())
        }
        pub fn replay_error(&self, error: super::super::replay::DailyReplayError) -> Source {
            replay_source(self.invocation, &self.expected, error)
        }
        pub fn stop(
            &self,
            original: Result<(), Source>,
            mut progress: Progress<<Self as Mode>::Held>,
        ) -> CheckedDailyPolicyFailure {
            // Own the original BEFORE any gap or mandatory pending retirement.
            let mut evidence = Evidence::new();
            evidence.original = original;
            if evidence.original.is_err() {
                self.invocation.attempt().gap(Gap::Abandoned);
            }
            let returned = Returned {
                admission: progress.admission.take(),
                bars: progress.bars.take(),
                pins: None,
            };
            if let Some(held) = progress.held.take() {
                let lower = held.finish_local();
                evidence.retirements.push(Retirement {
                    kind: RetirementKind::Pending,
                    lower,
                    returned: Some(returned),
                });
            } else {
                evidence.returned.push(returned);
            }
            evidence
                .locals
                .push(self.local(&self.expected, &self.captured));
            CheckedDailyPolicyFailure {
                evidence: Box::new(evidence),
            }
        }
        pub fn seal(
            self,
            original: Result<Built<'env, 'inv, 'attempt>, Source>,
            progress: Progress<<Self as Mode>::Held>,
        ) -> Result<Lease<'env, 'inv, 'attempt, 'hist>, CheckedDailyPolicyFailure> {
            let built = match original {
                Err(error) => return Err(self.stop(Err(error), progress)),
                Ok(built) => built,
            };
            let local = self.local(&self.expected, &self.captured);
            if !local.is_clean() {
                let mut evidence = Evidence::new();
                // A genuinely constructed success stays Ok with all its data;
                // cleanup uncertainty cannot invent an original policy error.
                let lower = built.held.finish_local();
                evidence.retirements.push(Retirement {
                    kind: RetirementKind::Pending,
                    lower,
                    returned: Some(built.data),
                });
                evidence.locals.push(local);
                evidence
                    .locals
                    .push(self.local(&self.expected, &self.captured));
                return Err(CheckedDailyPolicyFailure {
                    evidence: Box::new(evidence),
                });
            }
            Ok(Lease {
                mode: self,
                held: built.held,
                data: built.data,
                local,
            })
        }
    }
    impl<'env, 'inv, 'attempt, 'hist> Mode for Checked<'env, 'inv, 'attempt, 'hist> {
        type Held = CheckedDailyAdmissionFiles<'env, 'inv, 'attempt>;
        type Error = Source;
        type View<'a>
            = CheckedDailyAdmissionView<'a>
        where
            Self: 'a;
        fn policy(&self) -> &PinnedDailyPolicy {
            self.policy
        }
        fn base(&self) -> Result<&ApprovedHistoricalPriceOnlyArtifact, Source> {
            self.join(&self.expected, &self.captured, None, true)
                .map(|(base, _)| base)
        }
        fn read(&self, expected: &DailyPricePinsV1Wire) -> Result<Self::Held, Source> {
            self.check()?;
            self.root
                .read_files_in(
                    &self.expected,
                    &self.captured,
                    &expected.admission_sha256,
                    &expected.content_sha256,
                )
                .map_err(|e| Source::Artifact(Box::new(e)))
        }
        fn view<'a>(&'a self, held: &'a Self::Held) -> Result<Self::View<'a>, Source> {
            self.join(&self.expected, &self.captured, Some(held), true)
                .map(|(_, v)| v.expect("held leaf view"))
        }
        fn revalidate(&self, held: &Self::Held) -> Result<(), Source> {
            self.join(&self.expected, &self.captured, Some(held), true)?;
            held.revalidate_in(&self.expected, &self.captured)
                .map_err(|e| Source::Artifact(Box::new(e)))
        }
        fn policy_result<T>(&self, value: Result<T, Error>) -> Result<T, Source> {
            value.map_err(|error| policy_source(self.invocation, &self.expected, error))
        }
    }
    impl LeafView for CheckedDailyAdmissionView<'_> {
        fn manifest(&self) -> &super::super::DailyPriceManifestV1Wire {
            CheckedDailyAdmissionView::manifest(self)
        }
        fn normalization(&self) -> &super::super::DailyNormalizationProofV1Wire {
            CheckedDailyAdmissionView::normalization(self)
        }
        fn admission_bytes(&self) -> &[u8] {
            CheckedDailyAdmissionView::admission_bytes(self)
        }
        fn bars_ndjson(&self) -> &[u8] {
            CheckedDailyAdmissionView::bars_ndjson(self)
        }
    }

    pub struct Built<'env, 'inv, 'attempt> {
        held: CheckedDailyAdmissionFiles<'env, 'inv, 'attempt>,
        data: Returned,
    }
    pub fn build<'env, 'inv, 'attempt>(
        progress: &mut Progress<CheckedDailyAdmissionFiles<'env, 'inv, 'attempt>>,
        expected: &DailyPricePinsV1Wire,
    ) -> Built<'env, 'inv, 'attempt> {
        Built {
            held: progress.held.take().expect("completed leaf"),
            data: Returned {
                admission: progress.admission.take(),
                bars: progress.bars.take(),
                pins: Some(expected.clone()),
            },
        }
    }

    pub struct Lease<'env, 'inv, 'attempt, 'hist> {
        pub mode: Checked<'env, 'inv, 'attempt, 'hist>,
        pub held: CheckedDailyAdmissionFiles<'env, 'inv, 'attempt>,
        pub data: Returned,
        local: Local,
    }
    impl Lease<'_, '_, '_, '_> {
        pub fn pins(&self) -> &DailyPricePinsV1Wire {
            self.data.pins.as_ref().expect("completed pins")
        }
        pub fn bars(&self) -> &[HistoricalPriceOnlyBar] {
            self.data.bars.as_deref().expect("completed bars")
        }
        pub fn finish(self, kind: RetirementKind) -> CheckedDailyPolicyCompletion {
            let Self {
                mode,
                held,
                data,
                local,
            } = self;
            let mut unwind = Unwind::new(mode.invocation.attempt());
            let lower = held.finish_local();
            let after = mode.local(&mode.expected, &mode.captured);
            let mut evidence = Evidence::new();
            evidence.retirements.push(Retirement {
                kind,
                lower,
                returned: Some(data),
            });
            evidence.locals.push(local);
            evidence.locals.push(after);
            unwind.disarm();
            CheckedDailyPolicyCompletion {
                evidence: Box::new(evidence),
            }
        }
    }
    pub struct CheckedSealedHistoricalDailyInput<'env, 'inv, 'attempt, 'hist> {
        pub(in crate::daily_price_only) lease: Lease<'env, 'inv, 'attempt, 'hist>,
    }
    impl CheckedSealedHistoricalDailyInput<'_, '_, '_, '_> {
        pub fn view_in(
            &self,
            expected: &Attempt,
            captured: &Attempt,
        ) -> Result<CheckedSealedView<'_>, Source> {
            let (_, view) =
                self.lease
                    .mode
                    .join(expected, captured, Some(&self.lease.held), true)?;
            Ok(CheckedSealedView {
                lower: view.expect("held view"),
                returned: &self.lease.data,
            })
        }
        pub fn revalidate_in(&self, expected: &Attempt, captured: &Attempt) -> Result<(), Source> {
            self.lease
                .mode
                .join(expected, captured, Some(&self.lease.held), true)?;
            self.lease
                .held
                .revalidate_in(expected, captured)
                .map_err(|e| Source::Artifact(Box::new(e)))
        }
        pub fn finish_local(self) -> CheckedDailyPolicyCompletion {
            self.lease.finish(RetirementKind::Tip)
        }
    }
    pub struct CheckedSealedView<'a> {
        pub(in crate::daily_price_only) lower: CheckedDailyAdmissionView<'a>,
        pub(in crate::daily_price_only) returned: &'a Returned,
    }
    impl CheckedSealedView<'_> {
        pub fn manifest(&self) -> &super::super::DailyPriceManifestV1Wire {
            self.lower.manifest()
        }
        pub fn normalization(&self) -> &super::super::DailyNormalizationProofV1Wire {
            self.lower.normalization()
        }
        pub fn admission(&self) -> &DailyPriceAdmissionV1Wire {
            self.returned
                .admission
                .as_ref()
                .expect("completed admission")
        }
        pub fn pins(&self) -> &DailyPricePinsV1Wire {
            self.returned.pins.as_ref().expect("completed pins")
        }
        pub fn bars(&self) -> &[HistoricalPriceOnlyBar] {
            self.returned.bars.as_deref().expect("completed bars")
        }
    }
    pub fn read_sealed_historical_daily_input_checked<'env, 'inv, 'attempt, 'hist>(
        invocation: &'inv RawInvocation<'attempt>,
        expected_scope: &Attempt,
        captured: &Attempt,
        root: &'env CheckedDailyAdmissionRoot<'inv, 'attempt>,
        expected: &DailyPricePinsV1Wire,
        historical: &'env CheckedHistoricalApproval<'hist>,
        policy: &'env PinnedDailyPolicy,
    ) -> Result<
        CheckedSealedHistoricalDailyInput<'env, 'inv, 'attempt, 'hist>,
        CheckedDailyPolicyFailure,
    > {
        let mode = Checked::new(
            invocation,
            expected_scope,
            captured,
            root,
            historical,
            policy,
        );
        let mut unwind = Unwind::new(invocation.attempt());
        let mut progress = Progress::new();
        let original = super::sealed::first(&mode, expected, &mut progress, |progress| {
            build(progress, expected)
        });
        let result = mode
            .seal(original, progress)
            .map(|lease| CheckedSealedHistoricalDailyInput { lease });
        unwind.disarm();
        result
    }
    macro_rules! bounded {
        ($($ty:ty),+ $(,)?) => { $(impl fmt::Debug for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_struct(stringify!($ty)).finish_non_exhaustive()
            }
        })+ };
    }
    bounded!(
        Local,
        JoinFailure,
        Source,
        Returned,
        Retirement,
        CheckedDailyPolicyFailure,
        CheckedDailyPolicyCompletion,
        CheckedSealedHistoricalDailyInput<'_, '_, '_, '_>,
        CheckedSealedView<'_>
    );
}
#[cfg(target_os = "linux")]
pub use checked::{
    CheckedDailyPolicyCompletion, CheckedDailyPolicyFailure, CheckedSealedHistoricalDailyInput,
    CheckedSealedView, FirstKind as DailyPolicyFirstKind, JoinFailure as DailyPolicyUseFailure,
    Local as DailyPolicyLocalResult, Retirement as DailyPolicyRetirement,
    RetirementKind as DailyPolicyRetirementKind, Returned as DailyPolicyReturnedData,
    Source as DailyPolicySource, Unit as DailyPolicyUnitResult,
    read_sealed_historical_daily_input_checked,
};

// End C15 additions.

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

// C15 ordinary custody tests.

#[cfg(test)]
mod sealed_custody_tests {
    use super::sealed::{Pending, Progress, input, promote, retain};
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    struct Row(Box<str>);
    struct Original(Box<str>);
    struct Dropped(u8, Rc<RefCell<Vec<u8>>>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.1.borrow_mut().push(self.0);
        }
    }
    #[test]
    fn returned_nonclone_value_precedes_later_failure_without_invented_prefix() {
        let mut slot = None;
        let value = Row("genuine returned row".into());
        let address = value.0.as_ptr();
        assert!(retain(&mut slot, Ok::<_, Original>(value)).is_ok());
        let error = Original("actual next operation".into());
        let error_address = error.0.as_ptr();
        let mut absent: Option<Row> = None;
        let error = match retain(&mut absent, Err(error)) {
            Err(error) => error,
            Ok(()) => panic!("original error"),
        };
        assert_eq!(error.0.as_ptr(), error_address);
        assert!(absent.is_none());
        let mut empty = None;
        assert!(retain(&mut empty, Ok::<_, Original>(Vec::<Row>::new())).is_ok());
        assert!(empty.as_ref().is_some_and(Vec::is_empty));
        assert_eq!(slot.as_ref().unwrap().0.as_ptr(), address);
    }
    #[test]
    fn pending_failure_drops_next_before_previous_and_finishes_each_once() {
        let order = Rc::new(RefCell::new(Vec::new()));
        {
            let mut progress = Progress::new();
            progress.held = Some(Dropped(1, Rc::clone(&order)));
            // The same pending seam is generic; no opaque native holder is made.
            drop(progress);
            let mut pending = Pending::new();
            pending.current = Some(Dropped(2, Rc::clone(&order)));
            pending.next = Some(Dropped(3, Rc::clone(&order)));
        }
        assert_eq!(&*order.borrow(), &[1, 3, 2]);
    }
    #[test]
    fn previous_uncertainty_retires_pending_success_without_promotion() {
        let mut pending = Pending::new();
        pending.current = Some(Row("previous".into()));
        pending.next = Some(Row("returned child".into()));
        let mut retired = Vec::new();
        let clean = promote(&mut pending, |row, predecessor| {
            retired.push((row, predecessor));
            !predecessor
        });
        assert!(!clean);
        assert!(pending.current.is_none() && pending.next.is_none());
        assert_eq!(retired[0].0.0.as_ref(), "previous");
        assert_eq!(retired[1].0.0.as_ref(), "returned child");
        assert!(retired[0].1 && !retired[1].1);
    }
    #[test]
    fn input_borrow_uses_same_revalidation_once_and_never_moves_parent_owner() {
        let parent = Row("held outside input".into());
        let address = parent.0.as_ptr();
        let calls = Cell::new(0);
        let borrowed = input(&parent, |row| {
            calls.set(calls.get() + 1);
            assert_eq!(row.0.as_ptr(), address);
            Ok::<(), Original>(())
        })
        .ok()
        .unwrap();
        assert_eq!(calls.get(), 1);
        assert_eq!(borrowed.0.as_ptr(), address);
    }

    struct NoLeafView;
    impl super::sealed::LeafView for NoLeafView {
        fn manifest(&self) -> &crate::daily_price_only::DailyPriceManifestV1Wire {
            unreachable!("no read returned")
        }
        fn normalization(&self) -> &crate::daily_price_only::DailyNormalizationProofV1Wire {
            unreachable!("no read returned")
        }
        fn admission_bytes(&self) -> &[u8] {
            unreachable!("no read returned")
        }
        fn bars_ndjson(&self) -> &[u8] {
            unreachable!("no read returned")
        }
    }
    struct ReadFailureMode(RefCell<Option<Original>>, Cell<usize>);
    impl super::sealed::Mode for ReadFailureMode {
        type Held = Row;
        type Error = Original;
        type View<'a>
            = NoLeafView
        where
            Self: 'a;
        fn policy(&self) -> &super::PinnedDailyPolicy {
            unreachable!("after failing read")
        }
        fn base(&self) -> Result<&crate::ApprovedHistoricalPriceOnlyArtifact, Original> {
            unreachable!("after failing read")
        }
        fn read(&self, _: &crate::daily_price_only::DailyPricePinsV1Wire) -> Result<Row, Original> {
            self.1.set(self.1.get() + 1);
            Err(self
                .0
                .borrow_mut()
                .take()
                .expect("one actual ordinary error"))
        }
        fn view<'a>(&'a self, _: &'a Row) -> Result<NoLeafView, Original> {
            unreachable!("after failing read")
        }
        fn revalidate(&self, _: &Row) -> Result<(), Original> {
            unreachable!("after failing read")
        }
        fn policy_result<T>(&self, _: Result<T, super::Error>) -> Result<T, Original> {
            unreachable!("after failing read")
        }
    }
    #[test]
    fn first_engine_keeps_nonclone_read_error_before_any_parse_or_history_access() {
        let error = Original("ordinary original read error".into());
        let address = error.0.as_ptr();
        let mode = ReadFailureMode(RefCell::new(Some(error)), Cell::new(0));
        let hash = domain::ContentHash::from_bytes(b"navigation only");
        let pins = crate::daily_price_only::DailyPricePinsV1Wire {
            content_sha256: hash.clone(),
            manifest_sha256: hash.clone(),
            admission_sha256: hash.clone(),
            policy_sha256: hash,
        };
        let mut progress = Progress::new();
        let error = super::sealed::first(&mode, &pins, &mut progress, |_| ()).unwrap_err();
        assert_eq!(error.0.as_ptr(), address);
        assert_eq!(mode.1.get(), 1);
        assert!(progress.held.is_none() && progress.admission.is_none() && progress.bars.is_none());
    }
    #[test]
    fn same_scope_never_substitutes_equal_data_for_actual_owner_object_identity() {
        let root = Row("same contents".into());
        let different_root = Row("same contents".into());
        let historical = Row("same history".into());
        let different_historical = Row("same history".into());
        assert!(super::sealed::same_owners(
            &root,
            &root,
            &historical,
            &historical
        ));
        assert!(!super::sealed::same_owners(
            &root,
            &different_root,
            &historical,
            &historical
        ));
        assert!(!super::sealed::same_owners(
            &root,
            &root,
            &historical,
            &different_historical
        ));
    }

    #[cfg(target_os = "linux")]
    mod checked {
        use super::*;
        use crate::checked_close::{Attempt, Close, Errno, Gap, Role};
        use crate::daily_price_only::admission::checked::{Evidence, Source, Unwind, local_scopes};
        use crate::raw_custody::RawInvocation;
        struct Counted(Rc<Cell<usize>>, bool);
        impl Close for Counted {
            fn close(self) -> Result<(), Errno> {
                self.0.set(self.0.get() + 1);
                if self.1 {
                    Ok(())
                } else {
                    Err(Errno::BadDescriptor)
                }
            }
        }
        #[test]
        fn dirty_scope_still_visits_all_stored_and_supplied_foreign_ledgers() {
            let actual = Attempt::default();
            let stored_expected = Attempt::default();
            let stored_captured = Attempt::default();
            let expected = Attempt::default();
            let captured = Attempt::default();
            actual.gap(Gap::Unbound);
            let invocation = RawInvocation::new(&actual);
            let checks = local_scopes(
                &invocation,
                &stored_expected,
                &stored_captured,
                &expected,
                &captured,
            );
            assert!(!checks.is_clean());
            for scope in [&stored_expected, &stored_captured, &expected, &captured] {
                assert!(!scope.check());
                assert!(scope.report().first().is_some());
            }
        }
        #[test]
        fn late_only_keeps_original_value_and_absent_first_report() {
            let actual = Attempt::default();
            let invocation = RawInvocation::new(&actual);
            let calls = Rc::new(Cell::new(0));
            let owner = actual.adopt(Counted(Rc::clone(&calls), false), Role::Source);
            let mut returned = None;
            let row = Row("successful returned data".into());
            let address = row.0.as_ptr();
            assert!(retain(&mut returned, Ok::<_, Original>(row)).is_ok());
            assert!(!owner.finish());
            let mut evidence = Evidence::new();
            evidence.locals.push(local_scopes(
                &invocation,
                &actual,
                &actual,
                &actual,
                &actual,
            ));
            assert!(evidence.original.is_ok());
            assert!(!evidence.is_clean());
            assert_eq!(returned.as_ref().unwrap().0.as_ptr(), address);
            assert_eq!(calls.get(), 1);
            assert_eq!(actual.report().unknown(), 1);
            assert_eq!(actual.report().closed(), 0);
            let original = Source::Late;
            let later = Source::Policy(Box::new(
                invocation
                    .settle_local_in(&actual, Err::<(), _>(super::super::Error::InvalidCandidate))
                    .unwrap_err(),
            ));
            let joined = Source::Joined(Box::new((Err(original), Err(later))));
            assert!(joined.first_kind().is_none());
            assert!(joined.report().is_none());
        }
        #[test]
        fn borrowed_local_settlement_keeps_unrelated_owner_and_unwind_is_gap_only() {
            let actual = Attempt::default();
            let invocation = RawInvocation::new(&actual);
            let calls = Rc::new(Cell::new(0));
            let owner = actual.adopt(Counted(Rc::clone(&calls), true), Role::Ancestor);
            assert!(local_scopes(&invocation, &actual, &actual, &actual, &actual).is_clean());
            assert_eq!(actual.report().live(), 1);
            assert_eq!(calls.get(), 0);
            drop(Unwind::new(&actual));
            assert!(!actual.check());
            assert_eq!(calls.get(), 0);
            assert!(!owner.finish());
            assert_eq!(calls.get(), 1);
            assert_eq!(actual.report().closed(), 1);
        }
        #[test]
        fn actual_lower_none_gate_refuses_before_native_open() {
            use crate::daily_price_only::artifact::{
                CheckedDailyAdmissionRoot, DailyArtifactFirstKind,
            };
            let actual = Attempt::default();
            let invocation = RawInvocation::new(&actual);
            let failure = CheckedDailyAdmissionRoot::open_in(
                &invocation,
                &actual,
                &actual,
                std::path::Path::new("/not-opened-by-none-gate"),
                0,
                0,
            )
            .unwrap_err();
            assert_eq!(failure.first_kind(), Some(DailyArtifactFirstKind::Binding));
            assert_eq!(actual.report().live(), 0);
            assert_eq!(actual.report().attempted(), 0);
        }
        #[test]
        fn diagnostics_are_bounded_and_do_not_require_body_debug_or_clone() {
            let actual = Attempt::default();
            let invocation = RawInvocation::new(&actual);
            let checks = local_scopes(&invocation, &actual, &actual, &actual, &actual);
            let text = format!("{checks:?} {:?}", Source::Late);
            assert!(!text.contains("returned data"));
            assert!(text.len() < 256);
        }
    }
}

// End C15 ordinary custody tests.
