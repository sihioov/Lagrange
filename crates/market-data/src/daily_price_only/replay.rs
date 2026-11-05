//! Bounded reopening of one explicitly pinned daily admission chain.
//!
//! Parent records are untrusted navigation only. A capability is returned only
//! after every full bundle is reopened through the existing approved-base and
//! predecessor verification APIs. There is no latest lookup, DB authority or IO
//! write here. Runtime cancellation is observed between bounded leaf operations.

use std::collections::HashSet;

use domain::ContentHash;

use super::admission::{
    DailyAdmissionVerificationError, PinnedDailyPolicy, SealedHistoricalDailyInput,
    read_sealed_historical_daily_input,
};
use super::artifact::{DailyAdmissionRoot, DailyArtifactError, read_admission_metadata};
use super::continuation::{
    SealedDailyContinuation, SealedDailyParent, read_sealed_daily_continuation,
};
use super::contract::fixed_limits;
use super::input::AdmittedDailyPriceInputV1;
use super::{
    DailyPricePinsV1Wire, DailyWireErrorV1, encode_daily_price_admission_v1,
    parse_daily_price_admission_v1,
};
use crate::ApprovedHistoricalPriceOnlyArtifact;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DailyReplayError {
    #[error("daily replay limits are invalid")]
    InvalidLimits,
    #[error("daily replay chain is invalid")]
    InvalidChain,
    #[error("daily replay resource limit exceeded")]
    LimitExceeded,
    #[error("daily replay was interrupted")]
    Interrupted,
    #[error("{0}")]
    Artifact(DailyArtifactError),
    #[error("{0}")]
    Verification(DailyAdmissionVerificationError),
}

/// Explicit local work bound, with no implicit/default activation budget.
/// The D4 per-file bounds still apply to each operation. This is not a provider
/// acquisition quota or a guarantee that a synchronous read is preemptible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DailyReplayLimits {
    max_admissions: usize,
}
impl DailyReplayLimits {
    pub fn new(max_admissions: u64) -> Result<Self, DailyReplayError> {
        if max_admissions == 0 || max_admissions > fixed_limits().max_sessions {
            return Err(DailyReplayError::InvalidLimits);
        }
        Ok(Self {
            max_admissions: usize::try_from(max_admissions)
                .map_err(|_| DailyReplayError::InvalidLimits)?,
        })
    }
}

/// Only the final cumulative leaf is retained. It borrows the caller's actual
/// approved historical base and pinned policy, and owns its own descriptors.
pub struct ReopenedDailyChain<'base> {
    tip: SealedTip<'base>,
}
enum SealedTip<'base> {
    First(SealedHistoricalDailyInput<'base>),
    Continued(SealedDailyContinuation<'base>),
}
impl<'base> ReopenedDailyChain<'base> {
    pub fn as_parent(&self) -> SealedDailyParent<'_, 'base> {
        match &self.tip {
            SealedTip::First(value) => SealedDailyParent::First(value),
            SealedTip::Continued(value) => SealedDailyParent::Continued(value),
        }
    }

    pub fn admitted(&self) -> Result<AdmittedDailyPriceInputV1<'_, 'base>, DailyReplayError> {
        AdmittedDailyPriceInputV1::from_sealed(self.as_parent())
            .map_err(DailyReplayError::Verification)
    }
}

/// Reopens exactly the requested tip. The callback must remain true to continue;
/// a caller can bind it to a deadline/lease/cancellation signal. It is checked
/// before and after every metadata/full-leaf operation. A single synchronous
/// bounded operation cannot be interrupted by this callback. The DB publication
/// gate is still mandatory, including revocation and active-session checks.
pub fn read_sealed_daily_chain<'base>(
    root: &DailyAdmissionRoot,
    expected: &DailyPricePinsV1Wire,
    base: &'base ApprovedHistoricalPriceOnlyArtifact,
    policy: &'base PinnedDailyPolicy,
    limits: DailyReplayLimits,
    mut keep_reading: impl FnMut() -> bool,
) -> Result<ReopenedDailyChain<'base>, DailyReplayError> {
    let mut mode = LegacyReplay { root, base, policy };
    let mut pending = super::admission::sealed::Pending::new();
    replay_with(&mut mode, &mut pending, expected, limits, &mut keep_reading)?;
    Ok(pending.current.take().expect("returned tip"))
}

/// Explicit navigation pins supplied by the caller. Their presence does not
/// attest a DB admission or make a filesystem orphan visible to consumers.
pub struct DailyObservationPins<'pins> {
    pub admission: &'pins DailyPricePinsV1Wire,
    pub source_frontier: &'pins ContentHash,
    pub owner: uuid::Uuid,
}

/// Both independently reopened capabilities; only the current cumulative input
/// and its current observation are retained. Refresh/DB gates remain separate.
pub struct ReopenedDailyObservation<'base> {
    chain: ReopenedDailyChain<'base>,
    frontier: super::frontier::SealedSourceFrontier,
}
impl<'base> ReopenedDailyObservation<'base> {
    pub fn as_parent(&self) -> SealedDailyParent<'_, 'base> {
        self.chain.as_parent()
    }
    pub fn frontier(&self) -> &super::frontier::SealedSourceFrontier {
        &self.frontier
    }
}

/// Reopens exactly the requested admission and frontier pair in one forward
/// walk. No full ancestor is reopened quadratically. Metadata never constructs a
/// capability: every planned pair passes the existing full verification APIs.
pub fn read_sealed_daily_observation<'base>(
    admissions: &DailyAdmissionRoot,
    frontiers: &super::artifact::DailySourceFrontierRoot,
    expected: DailyObservationPins<'_>,
    base: &'base ApprovedHistoricalPriceOnlyArtifact,
    policy: &'base PinnedDailyPolicy,
    limits: DailyReplayLimits,
    mut keep_reading: impl FnMut() -> bool,
) -> Result<ReopenedDailyObservation<'base>, DailyReplayError> {
    let plan = discover_chain(
        expected.admission,
        policy.sha256(),
        limits,
        &mut keep_reading,
        |hash| read_admission_metadata(admissions, hash).map_err(DailyReplayError::Artifact),
    )?;
    let frontier_plan = discover_frontier_chain(&plan, &expected, &mut keep_reading, |hash| {
        super::frontier::read_daily_frontier_navigation(frontiers, hash)
            .map_err(DailyReplayError::Verification)
    })?;
    let mut current: Option<ReopenedDailyObservation<'base>> = None;
    for (pins, frontier_hash) in plan.iter().zip(&frontier_plan).rev() {
        checkpoint(&mut keep_reading)?;
        let next = match current.as_ref() {
            None => SealedTip::First(
                read_sealed_historical_daily_input(admissions, pins, base, policy)
                    .map_err(DailyReplayError::Verification)?,
            ),
            Some(prior) => SealedTip::Continued(
                read_sealed_daily_continuation(admissions, pins, base, policy, &prior.as_parent())
                    .map_err(DailyReplayError::Verification)?,
            ),
        };
        checkpoint(&mut keep_reading)?;
        let chain = ReopenedDailyChain { tip: next };
        let frontier = super::frontier::read_sealed_source_frontier(
            frontiers,
            frontier_hash,
            expected.owner,
            &chain.as_parent(),
            current.as_ref().map(|prior| &prior.frontier),
        )
        .map_err(DailyReplayError::Verification)?;
        checkpoint(&mut keep_reading)?;
        current = Some(ReopenedDailyObservation { chain, frontier });
    }
    let result = current.ok_or(DailyReplayError::InvalidChain)?;
    if result.as_parent().pins() != expected.admission
        || result.frontier.sha256() != expected.source_frontier
        || result.frontier.owner() != expected.owner
    {
        return Err(DailyReplayError::InvalidChain);
    }
    result
        .as_parent()
        .revalidate()
        .map_err(DailyReplayError::Verification)?;
    result
        .frontier
        .revalidate()
        .map_err(DailyReplayError::Verification)?;
    checkpoint(&mut keep_reading)?;
    Ok(result)
}

fn discover_frontier_chain(
    admissions: &[DailyPricePinsV1Wire],
    expected: &DailyObservationPins<'_>,
    keep_reading: &mut impl FnMut() -> bool,
    mut read: impl FnMut(
        &ContentHash,
    ) -> Result<super::frontier::DailyFrontierNavigation, DailyReplayError>,
) -> Result<Vec<ContentHash>, DailyReplayError> {
    if admissions.is_empty()
        || admissions.first() != Some(expected.admission)
        || expected.owner.is_nil()
    {
        return Err(DailyReplayError::InvalidChain);
    }
    let mut result = Vec::new();
    let mut visited = HashSet::new();
    let mut requested = expected.source_frontier.clone();
    let mut next_range: Option<(domain::TradingDate, domain::TradingDate)> = None;
    for (index, pins) in admissions.iter().enumerate() {
        checkpoint(keep_reading)?;
        visited
            .try_reserve(1)
            .map_err(|_| DailyReplayError::LimitExceeded)?;
        if !visited.insert(requested.clone()) {
            return Err(DailyReplayError::InvalidChain);
        }
        let nav = read(&requested)?;
        checkpoint(keep_reading)?;
        if nav.owner() != expected.owner
            || nav.policy_sha256() != &pins.policy_sha256
            || nav.admission_sha256() != &pins.admission_sha256
            || next_range
                .is_some_and(|(start, end)| nav.range_start() != start || nav.range_end() >= end)
        {
            return Err(DailyReplayError::InvalidChain);
        }
        result
            .try_reserve(1)
            .map_err(|_| DailyReplayError::LimitExceeded)?;
        result.push(requested.clone());
        next_range = Some((nav.range_start(), nav.range_end()));
        match (nav.parent_frontier_sha256(), index + 1 == admissions.len()) {
            (None, true) => (),
            (Some(parent), false) => requested = parent.clone(),
            _ => return Err(DailyReplayError::InvalidChain),
        }
    }
    Ok(result)
}

/// Unapproved, canonical pin navigation for one admission record. The full
/// guarded replay and DB equality checks are still required for any current use.
pub fn read_daily_admission_pins(
    root: &DailyAdmissionRoot,
    admission: &ContentHash,
    policy: &ContentHash,
) -> Result<DailyPricePinsV1Wire, DailyReplayError> {
    let bytes = read_admission_metadata(root, admission).map_err(DailyReplayError::Artifact)?;
    Ok(decode_navigation(&bytes, admission, policy)?.pins)
}

fn checkpoint(keep_reading: &mut impl FnMut() -> bool) -> Result<(), DailyReplayError> {
    if keep_reading() {
        Ok(())
    } else {
        Err(DailyReplayError::Interrupted)
    }
}

struct Navigation {
    pins: DailyPricePinsV1Wire,
    parent: Option<ContentHash>,
    added_sessions: u64,
}
fn decode_navigation(
    bytes: &[u8],
    requested: &ContentHash,
    policy: &ContentHash,
) -> Result<Navigation, DailyReplayError> {
    if bytes.len() as u64 > fixed_limits().max_admission_bytes {
        return Err(DailyReplayError::LimitExceeded);
    }
    if ContentHash::from_bytes(bytes) != *requested {
        return Err(DailyReplayError::InvalidChain);
    }
    let invalid_wire = |error| match error {
        DailyWireErrorV1::LimitExceeded => DailyReplayError::LimitExceeded,
        _ => DailyReplayError::InvalidChain,
    };
    let record = parse_daily_price_admission_v1(bytes).map_err(invalid_wire)?;
    if encode_daily_price_admission_v1(&record).map_err(invalid_wire)? != bytes
        || record.policy_sha256 != *policy
    {
        return Err(DailyReplayError::InvalidChain);
    }
    Ok(Navigation {
        pins: DailyPricePinsV1Wire {
            content_sha256: record.content_sha256,
            manifest_sha256: record.manifest_sha256,
            admission_sha256: requested.clone(),
            policy_sha256: record.policy_sha256,
        },
        parent: record.parent_admission_sha256,
        added_sessions: record.counts.added_session_count,
    })
}

fn discover_chain(
    expected: &DailyPricePinsV1Wire,
    policy: &ContentHash,
    limits: DailyReplayLimits,
    keep_reading: &mut impl FnMut() -> bool,
    mut read: impl FnMut(&ContentHash) -> Result<Vec<u8>, DailyReplayError>,
) -> Result<Vec<DailyPricePinsV1Wire>, DailyReplayError> {
    let mut mode = LegacyNavigation { read: &mut read };
    let mut pending = None;
    discover_chain_with(
        &mut mode,
        &mut pending,
        expected,
        policy,
        limits,
        keep_reading,
    )
}

// C15 shared sealed policy and checked custody.

trait NavigationMode {
    type Metadata;
    type Error;
    fn error(&self, error: DailyReplayError) -> Self::Error;
    fn checkpoint(&mut self, callback: &mut impl FnMut() -> bool) -> Result<(), Self::Error>;
    fn read(&mut self, hash: &ContentHash) -> Result<Self::Metadata, Self::Error>;
    fn bytes<'a>(&'a self, metadata: &'a Self::Metadata) -> Result<&'a [u8], Self::Error>;
    fn retire(&mut self, metadata: &mut Option<Self::Metadata>) -> Result<(), Self::Error>;
}
struct LegacyNavigation<'a, F> {
    read: &'a mut F,
}
impl<F: FnMut(&ContentHash) -> Result<Vec<u8>, DailyReplayError>> NavigationMode
    for LegacyNavigation<'_, F>
{
    type Metadata = Vec<u8>;
    type Error = DailyReplayError;
    fn error(&self, error: DailyReplayError) -> DailyReplayError {
        error
    }
    fn checkpoint(&mut self, callback: &mut impl FnMut() -> bool) -> Result<(), DailyReplayError> {
        checkpoint(callback)
    }
    fn read(&mut self, hash: &ContentHash) -> Result<Vec<u8>, DailyReplayError> {
        (self.read)(hash)
    }
    fn bytes<'a>(&'a self, metadata: &'a Vec<u8>) -> Result<&'a [u8], DailyReplayError> {
        Ok(metadata)
    }
    fn retire(&mut self, metadata: &mut Option<Vec<u8>>) -> Result<(), DailyReplayError> {
        drop(metadata.take());
        Ok(())
    }
}
fn discover_chain_with<M: NavigationMode>(
    mode: &mut M,
    pending: &mut Option<M::Metadata>,
    expected: &DailyPricePinsV1Wire,
    policy: &ContentHash,
    limits: DailyReplayLimits,
    keep_reading: &mut impl FnMut() -> bool,
) -> Result<Vec<DailyPricePinsV1Wire>, M::Error> {
    if expected.policy_sha256 != *policy {
        return Err(mode.error(DailyReplayError::InvalidChain));
    }
    let mut result = Vec::new();
    let mut visited = HashSet::new();
    let mut requested = expected.admission_sha256.clone();
    let mut next_added = None;
    loop {
        mode.checkpoint(keep_reading)?;
        if result.len() >= limits.max_admissions {
            return Err(mode.error(DailyReplayError::LimitExceeded));
        }
        visited
            .try_reserve(1)
            .map_err(|_| mode.error(DailyReplayError::LimitExceeded))?;
        if !visited.insert(requested.clone()) {
            return Err(mode.error(DailyReplayError::InvalidChain));
        }
        super::admission::sealed::retain(pending, mode.read(&requested))?;
        mode.checkpoint(keep_reading)?;
        let bytes = mode.bytes(pending.as_ref().expect("returned metadata"))?;
        let record = decode_navigation(bytes, &requested, policy).map_err(|e| mode.error(e))?;
        if record.added_sessions > limits.max_admissions as u64 {
            return Err(mode.error(DailyReplayError::LimitExceeded));
        }
        if next_added.is_some_and(|count| count != record.added_sessions)
            || (result.is_empty() && record.pins != *expected)
        {
            return Err(mode.error(DailyReplayError::InvalidChain));
        }
        result
            .try_reserve(1)
            .map_err(|_| mode.error(DailyReplayError::LimitExceeded))?;
        result.push(record.pins);
        match (record.parent, record.added_sessions) {
            (None, 1) => {
                mode.retire(pending)?;
                return Ok(result);
            }
            (Some(parent), count) if count > 1 => {
                requested = parent;
                next_added = Some(count - 1);
                mode.retire(pending)?;
            }
            _ => return Err(mode.error(DailyReplayError::InvalidChain)),
        }
    }
}
trait ReplayMode {
    type Tip;
    type Error;
    fn discover(
        &mut self,
        expected: &DailyPricePinsV1Wire,
        limits: DailyReplayLimits,
        callback: &mut impl FnMut() -> bool,
    ) -> Result<Vec<DailyPricePinsV1Wire>, Self::Error>;
    fn checkpoint(
        &mut self,
        current: Option<&Self::Tip>,
        next: Option<&Self::Tip>,
        callback: &mut impl FnMut() -> bool,
    ) -> Result<(), Self::Error>;
    fn first(&self, pins: &DailyPricePinsV1Wire) -> Result<Self::Tip, Self::Error>;
    fn child(
        &self,
        pins: &DailyPricePinsV1Wire,
        parent: &Self::Tip,
    ) -> Result<Self::Tip, Self::Error>;
    fn replace(
        &mut self,
        pending: &mut super::admission::sealed::Pending<Self::Tip>,
    ) -> Result<(), Self::Error>;
    fn tip(&self, current: &Self::Tip, expected: &DailyPricePinsV1Wire) -> Result<(), Self::Error>;
    fn empty(&self) -> Self::Error;
}
fn replay_with<M: ReplayMode>(
    mode: &mut M,
    pending: &mut super::admission::sealed::Pending<M::Tip>,
    expected: &DailyPricePinsV1Wire,
    limits: DailyReplayLimits,
    keep_reading: &mut impl FnMut() -> bool,
) -> Result<(), M::Error> {
    let plan = mode.discover(expected, limits, keep_reading)?;
    for pins in plan.iter().rev() {
        mode.checkpoint(pending.current.as_ref(), None, keep_reading)?;
        let next = match pending.current.as_ref() {
            None => mode.first(pins)?,
            Some(parent) => mode.child(pins, parent)?,
        };
        pending.next = Some(next);
        mode.checkpoint(
            pending.current.as_ref(),
            pending.next.as_ref(),
            keep_reading,
        )?;
        mode.replace(pending)?;
    }
    let result = pending.current.as_ref().ok_or_else(|| mode.empty())?;
    mode.tip(result, expected)?;
    Ok(())
}
struct LegacyReplay<'root, 'base> {
    root: &'root DailyAdmissionRoot,
    base: &'base ApprovedHistoricalPriceOnlyArtifact,
    policy: &'base PinnedDailyPolicy,
}
impl<'base> ReplayMode for LegacyReplay<'_, 'base> {
    type Tip = ReopenedDailyChain<'base>;
    type Error = DailyReplayError;
    fn discover(
        &mut self,
        expected: &DailyPricePinsV1Wire,
        limits: DailyReplayLimits,
        keep_reading: &mut impl FnMut() -> bool,
    ) -> Result<Vec<DailyPricePinsV1Wire>, DailyReplayError> {
        discover_chain(
            expected,
            self.policy.sha256(),
            limits,
            keep_reading,
            |hash| read_admission_metadata(self.root, hash).map_err(DailyReplayError::Artifact),
        )
    }
    fn checkpoint(
        &mut self,
        _: Option<&Self::Tip>,
        _: Option<&Self::Tip>,
        callback: &mut impl FnMut() -> bool,
    ) -> Result<(), DailyReplayError> {
        checkpoint(callback)
    }
    fn first(&self, pins: &DailyPricePinsV1Wire) -> Result<Self::Tip, DailyReplayError> {
        Ok(ReopenedDailyChain {
            tip: SealedTip::First(
                read_sealed_historical_daily_input(self.root, pins, self.base, self.policy)
                    .map_err(DailyReplayError::Verification)?,
            ),
        })
    }
    fn child(
        &self,
        pins: &DailyPricePinsV1Wire,
        parent: &Self::Tip,
    ) -> Result<Self::Tip, DailyReplayError> {
        Ok(ReopenedDailyChain {
            tip: SealedTip::Continued(
                read_sealed_daily_continuation(
                    self.root,
                    pins,
                    self.base,
                    self.policy,
                    &parent.as_parent(),
                )
                .map_err(DailyReplayError::Verification)?,
            ),
        })
    }
    fn replace(
        &mut self,
        pending: &mut super::admission::sealed::Pending<Self::Tip>,
    ) -> Result<(), DailyReplayError> {
        // Exactly the original replacement/drop boundary; no extra allocation
        // or fallible work is introduced in the Legacy lane.
        pending.current = pending.next.take();
        Ok(())
    }
    fn tip(
        &self,
        current: &Self::Tip,
        expected: &DailyPricePinsV1Wire,
    ) -> Result<(), DailyReplayError> {
        if current.as_parent().pins() != expected {
            return Err(DailyReplayError::InvalidChain);
        }
        Ok(())
    }
    fn empty(&self) -> DailyReplayError {
        DailyReplayError::InvalidChain
    }
}

#[cfg(target_os = "linux")]
mod checked {
    use super::super::admission::checked::{
        Checked, CheckedDailyPolicyCompletion, CheckedDailyPolicyFailure, Evidence, RetirementKind,
        Source, Unwind,
    };
    use super::super::admission::{
        CheckedSealedHistoricalDailyInput, read_sealed_historical_daily_input_checked,
    };
    use super::super::artifact::{CheckedDailyAdmissionMetadata, CheckedDailyAdmissionRoot};
    use super::super::continuation::{
        CheckedSealedDailyContinuation, CheckedSealedDailyParent,
        read_sealed_daily_continuation_checked,
    };
    use super::*;
    use crate::checked_close::Attempt;
    use crate::historical_price_only_approval::CheckedHistoricalApproval;
    use crate::raw_custody::RawInvocation;

    enum Tip<'env, 'inv, 'attempt, 'hist> {
        First(CheckedSealedHistoricalDailyInput<'env, 'inv, 'attempt, 'hist>),
        Continued(CheckedSealedDailyContinuation<'env, 'inv, 'attempt, 'hist>),
    }
    impl<'env, 'inv, 'attempt, 'hist> Tip<'env, 'inv, 'attempt, 'hist> {
        fn parent(&self) -> CheckedSealedDailyParent<'_, 'env, 'inv, 'attempt, 'hist> {
            match self {
                Self::First(value) => value.as_parent(),
                Self::Continued(value) => value.as_parent(),
            }
        }
        fn finish(self, kind: RetirementKind) -> CheckedDailyPolicyCompletion {
            match self {
                Self::First(value) => value.lease.finish(kind),
                Self::Continued(value) => value.retire(kind),
            }
        }
    }
    pub(super) fn complete_callback_results(
        returned: Result<(), Source>,
        after: Result<(), Source>,
    ) -> Result<(), Source> {
        match (returned, after) {
            (Ok(()), after) => after,
            (Err(first), Ok(())) => Err(first),
            (Err(first), Err(later)) => Err(Source::Joined(Box::new((Err(first), Err(later))))),
        }
    }

    struct CheckedReplay<'env, 'inv, 'attempt, 'hist> {
        mode: Checked<'env, 'inv, 'attempt, 'hist>,
        evidence: Evidence,
    }
    impl<'env, 'inv, 'attempt, 'hist> CheckedReplay<'env, 'inv, 'attempt, 'hist> {
        fn check_holders(
            &self,
            current: Option<&Tip<'env, 'inv, 'attempt, 'hist>>,
            next: Option<&Tip<'env, 'inv, 'attempt, 'hist>>,
        ) -> Result<(), Source> {
            let own = self.mode.check();
            let (expected, captured) = self.mode.tokens();
            let previous = current
                .map(|tip| tip.parent().view_in(expected, captured).map(|_| ()))
                .unwrap_or(Ok(()));
            let pending = next
                .map(|tip| tip.parent().view_in(expected, captured).map(|_| ()))
                .unwrap_or(Ok(()));
            let holders = match (previous, pending) {
                (Ok(()), Ok(())) => Ok(()),
                (previous, pending) => Err(Source::Joined(Box::new((previous, pending)))),
            };
            match (own, holders) {
                (Ok(()), Ok(())) => Ok(()),
                (own, holders) => Err(Source::Joined(Box::new((own, holders)))),
            }
        }
        fn checked_callback(&mut self, callback: &mut impl FnMut() -> bool) -> Result<(), Source> {
            self.mode.check()?;
            let returned = checkpoint(callback);
            let after = self.mode.check();
            let returned = returned.map_err(|error| self.mode.replay_error(error));
            complete_callback_results(returned, after)
        }
    }
    impl<'env, 'inv, 'attempt, 'hist> NavigationMode for CheckedReplay<'env, 'inv, 'attempt, 'hist> {
        type Metadata = CheckedDailyAdmissionMetadata<'env, 'inv, 'attempt>;
        type Error = Source;
        fn error(&self, error: DailyReplayError) -> Source {
            self.mode.replay_error(error)
        }
        fn checkpoint(&mut self, callback: &mut impl FnMut() -> bool) -> Result<(), Source> {
            self.checked_callback(callback)
        }
        fn read(&mut self, hash: &ContentHash) -> Result<Self::Metadata, Source> {
            self.mode.check()?;
            let (expected, captured) = self.mode.tokens();
            self.mode
                .root
                .read_metadata_in(expected, captured, hash)
                .map_err(|e| Source::Artifact(Box::new(e)))
        }
        fn bytes<'a>(&'a self, metadata: &'a Self::Metadata) -> Result<&'a [u8], Source> {
            self.mode.check()?;
            let (expected, captured) = self.mode.tokens();
            metadata
                .bytes_in(expected, captured)
                .map_err(|e| Source::Daily(Box::new(e)))
        }
        fn retire(&mut self, metadata: &mut Option<Self::Metadata>) -> Result<(), Source> {
            if let Some(metadata) = metadata.take()
                && !self.evidence.metadata(metadata.into_completion())
            {
                return Err(Source::Late);
            }
            Ok(())
        }
    }
    impl<'env, 'inv, 'attempt, 'hist> ReplayMode for CheckedReplay<'env, 'inv, 'attempt, 'hist> {
        type Tip = Tip<'env, 'inv, 'attempt, 'hist>;
        type Error = Source;
        fn discover(
            &mut self,
            expected: &DailyPricePinsV1Wire,
            limits: DailyReplayLimits,
            keep_reading: &mut impl FnMut() -> bool,
        ) -> Result<Vec<DailyPricePinsV1Wire>, Source> {
            let mut pending = None;
            let policy = self.mode.policy.sha256().clone();
            let original =
                discover_chain_with(self, &mut pending, expected, &policy, limits, keep_reading);
            // The returned error is owned before mandatory metadata retirement.
            let retirement = NavigationMode::retire(self, &mut pending);
            match original {
                Err(error) => Err(error),
                Ok(plan) => retirement.map(|()| plan),
            }
        }
        fn checkpoint(
            &mut self,
            current: Option<&Self::Tip>,
            next: Option<&Self::Tip>,
            callback: &mut impl FnMut() -> bool,
        ) -> Result<(), Source> {
            self.check_holders(current, next)?;
            let returned = checkpoint(callback);
            let after = self.check_holders(current, next);
            let returned = returned.map_err(|error| self.mode.replay_error(error));
            complete_callback_results(returned, after)
        }
        fn first(&self, pins: &DailyPricePinsV1Wire) -> Result<Self::Tip, Source> {
            let (expected, captured) = self.mode.tokens();
            read_sealed_historical_daily_input_checked(
                self.mode.invocation,
                expected,
                captured,
                self.mode.root,
                pins,
                self.mode.historical,
                self.mode.policy,
            )
            .map(Tip::First)
            .map_err(|e| Source::Prior(Box::new(e)))
        }
        fn child(
            &self,
            pins: &DailyPricePinsV1Wire,
            parent: &Self::Tip,
        ) -> Result<Self::Tip, Source> {
            let (expected, captured) = self.mode.tokens();
            read_sealed_daily_continuation_checked(
                self.mode.invocation,
                expected,
                captured,
                self.mode.root,
                pins,
                self.mode.historical,
                self.mode.policy,
                &parent.parent(),
            )
            .map(Tip::Continued)
            .map_err(|e| Source::Prior(Box::new(e)))
        }
        fn replace(
            &mut self,
            pending: &mut super::super::admission::sealed::Pending<Self::Tip>,
        ) -> Result<(), Source> {
            let clean =
                super::super::admission::sealed::promote(pending, |previous, predecessor| {
                    self.evidence.append(previous.finish(if predecessor {
                        RetirementKind::Predecessor
                    } else {
                        RetirementKind::Pending
                    }))
                });
            if clean { Ok(()) } else { Err(Source::Late) }
        }
        fn tip(
            &self,
            current: &Self::Tip,
            expected_pins: &DailyPricePinsV1Wire,
        ) -> Result<(), Source> {
            let (expected, captured) = self.mode.tokens();
            let parent = current.parent();
            let view = parent.view_in(expected, captured)?;
            if view.pins() != expected_pins {
                return Err(self.mode.replay_error(DailyReplayError::InvalidChain));
            }
            Ok(())
        }
        fn empty(&self) -> Source {
            self.mode.replay_error(DailyReplayError::InvalidChain)
        }
    }
    pub struct CheckedReopenedDailyChain<'env, 'inv, 'attempt, 'hist> {
        tip: Tip<'env, 'inv, 'attempt, 'hist>,
        evidence: Evidence,
    }
    impl<'env, 'inv, 'attempt, 'hist> CheckedReopenedDailyChain<'env, 'inv, 'attempt, 'hist> {
        pub fn as_parent(&self) -> CheckedSealedDailyParent<'_, 'env, 'inv, 'attempt, 'hist> {
            self.tip.parent()
        }
        pub fn admitted(
            &self,
            expected: &Attempt,
            captured: &Attempt,
        ) -> Result<
            super::super::input::CheckedAdmittedDailyPriceInputV1<'_, 'env, 'inv, 'attempt, 'hist>,
            Source,
        > {
            super::super::input::CheckedAdmittedDailyPriceInputV1::from_sealed(
                self.as_parent(),
                expected,
                captured,
            )
        }
        pub fn finish_local(self) -> CheckedDailyPolicyCompletion {
            let Self { tip, mut evidence } = self;
            evidence.append(tip.finish(RetirementKind::Tip));
            CheckedDailyPolicyCompletion {
                evidence: Box::new(evidence),
            }
        }
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "Scope identity, borrowed genuine owners, explicit limits and cancellation stay explicit."
    )]
    pub fn read_sealed_daily_chain_checked<'env, 'inv, 'attempt, 'hist>(
        invocation: &'inv RawInvocation<'attempt>,
        expected_scope: &Attempt,
        captured: &Attempt,
        root: &'env CheckedDailyAdmissionRoot<'inv, 'attempt>,
        expected: &DailyPricePinsV1Wire,
        historical: &'env CheckedHistoricalApproval<'hist>,
        policy: &'env PinnedDailyPolicy,
        limits: DailyReplayLimits,
        mut keep_reading: impl FnMut() -> bool,
    ) -> Result<CheckedReopenedDailyChain<'env, 'inv, 'attempt, 'hist>, CheckedDailyPolicyFailure>
    {
        let mode = Checked::new(
            invocation,
            expected_scope,
            captured,
            root,
            historical,
            policy,
        );
        let mut replay = CheckedReplay {
            mode,
            evidence: Evidence::new(),
        };
        let mut unwind = Unwind::new(invocation.attempt());
        let mut pending = super::super::admission::sealed::Pending::new();
        let original = replay_with(
            &mut replay,
            &mut pending,
            expected,
            limits,
            &mut keep_reading,
        );
        // Own the WHOLE result before gap, child retirement or local settlement.
        replay.evidence.original = original;
        let (e, c) = replay.mode.tokens();
        replay.evidence.locals.push(replay.mode.local(e, c));
        let result = if replay.evidence.is_clean() {
            Ok(CheckedReopenedDailyChain {
                tip: pending.current.take().expect("returned tip"),
                evidence: replay.evidence,
            })
        } else {
            if replay.evidence.original.is_err() {
                invocation
                    .attempt()
                    .gap(crate::checked_close::Gap::Abandoned);
            }
            pending.drain(|tip| {
                replay.evidence.append(tip.finish(RetirementKind::Pending));
            });
            let (e, c) = replay.mode.tokens();
            replay.evidence.locals.push(replay.mode.local(e, c));
            Err(CheckedDailyPolicyFailure {
                evidence: Box::new(replay.evidence),
            })
        };
        unwind.disarm();
        result
    }
    impl std::fmt::Debug for CheckedReopenedDailyChain<'_, '_, '_, '_> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("CheckedReopenedDailyChain")
                .finish_non_exhaustive()
        }
    }
}
#[cfg(target_os = "linux")]
pub use checked::{CheckedReopenedDailyChain, read_sealed_daily_chain_checked};

// End C15 additions.

#[cfg(test)]
mod tests {
    use super::super::{
        DailyAdmissionCountsV1Wire, DailyBasePinsV1Wire, DailyBaseV1Wire, DailyPriceAdmissionV1Wire,
    };
    use super::*;
    use domain::CodeCommit;
    use std::collections::BTreeMap;

    fn hash(value: &str) -> ContentHash {
        ContentHash::from_bytes(value.as_bytes())
    }
    // Canonical navigation records only: no approved base, valid source bundle,
    // DB grant, sealed input or computation capability is created by this fixture.
    fn record(parent: Option<ContentHash>, count: u64) -> DailyPriceAdmissionV1Wire {
        DailyPriceAdmissionV1Wire {
            schema: "owner-beta-daily-etf11-admission-v1".into(),
            content_sha256: hash(&format!("content-{count}")),
            manifest_sha256: hash(&format!("manifest-{count}")),
            policy_sha256: hash("policy"),
            parent_admission_sha256: parent,
            base: DailyBaseV1Wire {
                kind: "historical_price_only_v3".into(),
                pins: DailyBasePinsV1Wire {
                    candidate_content_sha256: hash("base"),
                    artifact_manifest_sha256: hash("base-manifest"),
                    stage5_manifest_sha256: hash("prices"),
                    action_manifest_sha256: hash("actions"),
                    approval_registry_sha256: hash("registry"),
                },
            },
            verifier_code_commit: CodeCommit::parse(&"a".repeat(40)).unwrap(),
            normalizer_version: "daily-raw-reconstruction-v1".into(),
            evidence_sha256: hash("evidence"),
            counts: DailyAdmissionCountsV1Wire {
                instrument_count: 11,
                session_count: 2 + count,
                bar_count: 11 * (2 + count),
                added_session_count: count,
                source_file_count: 1,
                calendar_row_count: count,
                action_chain_count: 7 * count,
                action_page_count: 7 * count,
                cash_dividend_row_count: 0,
                normalized_document_count: 4 * count,
            },
        }
    }
    fn encode(record: &DailyPriceAdmissionV1Wire) -> (DailyPricePinsV1Wire, Vec<u8>) {
        let bytes = encode_daily_price_admission_v1(record).unwrap();
        let pins = DailyPricePinsV1Wire {
            content_sha256: record.content_sha256.clone(),
            manifest_sha256: record.manifest_sha256.clone(),
            admission_sha256: ContentHash::from_bytes(&bytes),
            policy_sha256: record.policy_sha256.clone(),
        };
        (pins, bytes)
    }
    struct ChainFixture {
        tip: DailyPricePinsV1Wire,
        records: BTreeMap<ContentHash, Vec<u8>>,
        pins: Vec<DailyPricePinsV1Wire>,
    }
    fn chain(count: u64) -> ChainFixture {
        let mut records = BTreeMap::new();
        let mut pins = Vec::new();
        let mut parent = None;
        for count in 1..=count {
            let (current, bytes) = encode(&record(parent, count));
            parent = Some(current.admission_sha256.clone());
            records.insert(current.admission_sha256.clone(), bytes);
            pins.push(current);
        }
        ChainFixture {
            tip: pins.last().unwrap().clone(),
            records,
            pins,
        }
    }
    fn discover(
        expected: &DailyPricePinsV1Wire,
        records: &BTreeMap<ContentHash, Vec<u8>>,
        limit: u64,
    ) -> Result<Vec<DailyPricePinsV1Wire>, DailyReplayError> {
        discover_chain(
            expected,
            &hash("policy"),
            DailyReplayLimits::new(limit).unwrap(),
            &mut || true,
            |hash| {
                records
                    .get(hash)
                    .cloned()
                    .ok_or(DailyReplayError::InvalidChain)
            },
        )
    }

    #[test]
    fn i4d_replay_limits_have_no_zero_or_above_schema_bound() {
        assert_eq!(
            DailyReplayLimits::new(0),
            Err(DailyReplayError::InvalidLimits)
        );
        assert_eq!(
            DailyReplayLimits::new(20_001),
            Err(DailyReplayError::InvalidLimits)
        );
        assert!(DailyReplayLimits::new(1).is_ok());
        assert!(DailyReplayLimits::new(20_000).is_ok());
    }

    #[test]
    fn i4d_navigation_returns_exact_tip_to_first_order() {
        let ChainFixture {
            tip,
            records,
            mut pins,
        } = chain(3);
        pins.reverse();
        assert_eq!(discover(&tip, &records, 3).unwrap(), pins);
        let ChainFixture {
            tip: first,
            records,
            pins,
        } = chain(1);
        assert_eq!(discover(&first, &records, 1).unwrap(), pins);
    }

    #[test]
    fn i4d_navigation_declared_depth_stops_before_parent_open() {
        let ChainFixture { tip, records, .. } = chain(3);
        let mut reads = Vec::new();
        let result = discover_chain(
            &tip,
            &hash("policy"),
            DailyReplayLimits::new(2).unwrap(),
            &mut || true,
            |hash| {
                reads.push(hash.clone());
                Ok(records[hash].clone())
            },
        );
        assert_eq!(result, Err(DailyReplayError::LimitExceeded));
        assert_eq!(reads, vec![tip.admission_sha256]);
    }

    #[test]
    fn i4d_navigation_pinned_policy_mismatch_performs_no_read() {
        let ChainFixture { mut tip, .. } = chain(1);
        tip.policy_sha256 = hash("foreign");
        let result = discover_chain(
            &tip,
            &hash("policy"),
            DailyReplayLimits::new(1).unwrap(),
            &mut || true,
            |_| panic!("mismatched policy must not read"),
        );
        assert_eq!(result, Err(DailyReplayError::InvalidChain));
    }

    #[test]
    fn i4d_navigation_all_tip_pins_are_bound() {
        let ChainFixture { tip, records, .. } = chain(2);
        for index in 0..4 {
            let mut changed = tip.clone();
            match index {
                0 => changed.content_sha256 = hash("foreign"),
                1 => changed.manifest_sha256 = hash("foreign"),
                2 => changed.admission_sha256 = hash("foreign"),
                _ => changed.policy_sha256 = hash("foreign"),
            }
            assert_eq!(
                discover(&changed, &records, 2),
                Err(DailyReplayError::InvalidChain)
            );
        }
    }

    #[test]
    fn i4d_navigation_rejects_missing_parent_and_wrong_depth() {
        let ChainFixture {
            tip,
            mut records,
            pins,
        } = chain(3);
        records.remove(&pins[1].admission_sha256);
        assert_eq!(
            discover(&tip, &records, 3),
            Err(DailyReplayError::InvalidChain)
        );
        let (first, bytes) = encode(&record(None, 1));
        let (bad, bad_bytes) = encode(&record(Some(first.admission_sha256.clone()), 3));
        let records = BTreeMap::from([
            (first.admission_sha256, bytes),
            (bad.admission_sha256.clone(), bad_bytes),
        ]);
        assert_eq!(
            discover(&bad, &records, 3),
            Err(DailyReplayError::InvalidChain)
        );
    }

    #[test]
    fn i4d_navigation_rejects_missing_first_or_spurious_first_parent() {
        for malformed in [record(None, 2), record(Some(hash("missing")), 1)] {
            // Bypass only the canonical *encoder* for malformed wire input;
            // production decoding must reject the self-consistent hash claim.
            let bytes = serde_json::to_vec(&malformed).unwrap();
            let pins = DailyPricePinsV1Wire {
                content_sha256: malformed.content_sha256,
                manifest_sha256: malformed.manifest_sha256,
                admission_sha256: ContentHash::from_bytes(&bytes),
                policy_sha256: malformed.policy_sha256,
            };
            let records = BTreeMap::from([(pins.admission_sha256.clone(), bytes)]);
            assert_eq!(
                discover(&pins, &records, 2),
                Err(DailyReplayError::InvalidChain)
            );
        }
    }

    #[test]
    fn i4d_navigation_rejects_changed_bytes_noncanonical_and_foreign_policy() {
        let (pins, mut bytes) = encode(&record(None, 1));
        bytes.push(b' ');
        assert!(matches!(
            decode_navigation(&bytes, &pins.admission_sha256, &hash("policy")),
            Err(DailyReplayError::InvalidChain)
        ));
        assert!(matches!(
            decode_navigation(&bytes, &ContentHash::from_bytes(&bytes), &hash("policy")),
            Err(DailyReplayError::InvalidChain)
        ));
        let (_, bytes) = encode(&record(None, 1));
        assert!(matches!(
            decode_navigation(&bytes, &ContentHash::from_bytes(&bytes), &hash("foreign")),
            Err(DailyReplayError::InvalidChain)
        ));
    }

    #[test]
    fn i4d_navigation_metadata_limit_precedes_hash_or_decode() {
        let bytes = vec![0; fixed_limits().max_admission_bytes as usize + 1];
        assert!(matches!(
            decode_navigation(&bytes, &hash("not-the-bytes"), &hash("policy")),
            Err(DailyReplayError::LimitExceeded)
        ));
    }

    #[test]
    fn i4d_navigation_cancellation_prevents_first_or_next_read() {
        let ChainFixture { tip, records, .. } = chain(2);
        let result = discover_chain(
            &tip,
            &hash("policy"),
            DailyReplayLimits::new(2).unwrap(),
            &mut || false,
            |_| panic!("canceled before first read"),
        );
        assert_eq!(result, Err(DailyReplayError::Interrupted));
        let mut checkpoints = 0;
        let mut reads = 0;
        let result = discover_chain(
            &tip,
            &hash("policy"),
            DailyReplayLimits::new(2).unwrap(),
            &mut || {
                checkpoints += 1;
                checkpoints < 2
            },
            |hash| {
                reads += 1;
                Ok(records[hash].clone())
            },
        );
        assert_eq!(result, Err(DailyReplayError::Interrupted));
        assert_eq!(reads, 1);
    }

    #[test]
    fn i4d_navigation_preserves_typed_io_error_without_values() {
        let ChainFixture { tip, .. } = chain(1);
        let result = discover_chain(
            &tip,
            &hash("policy"),
            DailyReplayLimits::new(1).unwrap(),
            &mut || true,
            |_| {
                Err(DailyReplayError::Artifact(
                    DailyArtifactError::SourceChanged,
                ))
            },
        );
        assert_eq!(
            result,
            Err(DailyReplayError::Artifact(
                DailyArtifactError::SourceChanged
            ))
        );
        assert_eq!(
            DailyReplayError::InvalidChain.to_string(),
            "daily replay chain is invalid"
        );
        assert_eq!(
            DailyReplayError::Interrupted.to_string(),
            "daily replay was interrupted"
        );
    }

    fn observation_owner() -> uuid::Uuid {
        uuid::Uuid::from_u128(101)
    }
    fn frontier_navigation_bytes(
        pins: &DailyPricePinsV1Wire,
        frontier: &ContentHash,
        parent: Option<&ContentHash>,
        index: u64,
    ) -> Vec<u8> {
        // Unapproved metadata, never a synthetic sealed observation.
        let parent = parent.map_or_else(|| "null".into(), |hash| format!("\"{hash}\""));
        format!(
            "{{\"schema\":\"owner-beta-daily-source-frontier-v1\",\"owner\":\"{}\",\"policy_sha256\":\"{}\",\"admission_sha256\":\"{}\",\"parent_frontier_sha256\":{},\"range_start\":\"2026-09-01\",\"range_end\":\"2026-09-{:02}\",\"observation_sha256\":\"{}\",\"source_file_count\":1}}",
            observation_owner(),pins.policy_sha256,pins.admission_sha256,parent,index,frontier
        ).into_bytes()
    }
    fn frontier_plan_fixture(
        pins: &[DailyPricePinsV1Wire],
    ) -> (ContentHash, BTreeMap<ContentHash, Vec<u8>>) {
        let mut records = BTreeMap::new();
        let mut parent = None;
        for (index, pins) in pins.iter().enumerate() {
            let current = hash(&format!("frontier-{index}"));
            let bytes =
                frontier_navigation_bytes(pins, &current, parent.as_ref(), index as u64 + 1);
            records.insert(current.clone(), bytes);
            parent = Some(current);
        }
        (parent.unwrap(), records)
    }
    fn read_frontier_navigation_fixture(
        records: &BTreeMap<ContentHash, Vec<u8>>,
        requested: &ContentHash,
    ) -> Result<super::super::frontier::DailyFrontierNavigation, DailyReplayError> {
        let bytes = records
            .get(requested)
            .ok_or(DailyReplayError::InvalidChain)?;
        super::super::frontier::decode_frontier_navigation(bytes, requested).map_err(|error| {
            DailyReplayError::Verification(DailyAdmissionVerificationError::Evidence(error))
        })
    }

    #[test]
    fn i4g_paired_navigation_returns_exact_matching_ancestry() {
        let ChainFixture { tip, pins, .. } = chain(3);
        let (frontier, records) = frontier_plan_fixture(&pins);
        let expected = DailyObservationPins {
            admission: &tip,
            source_frontier: &frontier,
            owner: observation_owner(),
        };
        let ordered = pins.into_iter().rev().collect::<Vec<_>>();
        let mut reads = Vec::new();
        let plan = discover_frontier_chain(&ordered, &expected, &mut || true, |hash| {
            reads.push(hash.clone());
            read_frontier_navigation_fixture(&records, hash)
        })
        .unwrap();
        assert_eq!(
            plan,
            vec![hash("frontier-2"), hash("frontier-1"), hash("frontier-0")]
        );
        assert_eq!(reads, plan);
    }

    #[test]
    fn i4g_paired_navigation_rejects_missing_parent_extra_parent_and_cycle() {
        let ChainFixture { tip, pins, .. } = chain(2);
        let (frontier, records) = frontier_plan_fixture(&pins);
        let expected = DailyObservationPins {
            admission: &tip,
            source_frontier: &frontier,
            owner: observation_owner(),
        };
        let ordered = pins.iter().rev().cloned().collect::<Vec<_>>();
        for change in 0..3 {
            let mut changed = records.clone();
            match change {
                0 => {
                    changed.insert(
                        frontier.clone(),
                        frontier_navigation_bytes(&pins[1], &frontier, None, 2),
                    );
                }
                1 => {
                    let first = hash("frontier-0");
                    changed.insert(
                        first.clone(),
                        frontier_navigation_bytes(&pins[0], &first, Some(&hash("extra")), 1),
                    );
                }
                _ => {
                    changed.insert(
                        frontier.clone(),
                        frontier_navigation_bytes(&pins[1], &frontier, Some(&frontier), 2),
                    );
                }
            }
            assert!(matches!(
                discover_frontier_chain(&ordered, &expected, &mut || true, |hash| {
                    read_frontier_navigation_fixture(&changed, hash)
                }),
                Err(DailyReplayError::InvalidChain)
            ));
        }
    }

    #[test]
    fn i4g_paired_navigation_rejects_foreign_owner_policy_and_admission() {
        let ChainFixture { tip, pins, .. } = chain(2);
        let (frontier, records) = frontier_plan_fixture(&pins);
        let ordered = pins.iter().rev().cloned().collect::<Vec<_>>();
        let expected = DailyObservationPins {
            admission: &tip,
            source_frontier: &frontier,
            owner: uuid::Uuid::from_u128(102),
        };
        assert!(matches!(
            discover_frontier_chain(&ordered, &expected, &mut || true, |hash| {
                read_frontier_navigation_fixture(&records, hash)
            }),
            Err(DailyReplayError::InvalidChain)
        ));
        let expected = DailyObservationPins {
            admission: &tip,
            source_frontier: &frontier,
            owner: observation_owner(),
        };
        for change in 0..2 {
            let mut changed = records.clone();
            let mut foreign = pins[1].clone();
            if change == 0 {
                foreign.policy_sha256 = hash("foreign");
            } else {
                foreign.admission_sha256 = hash("foreign");
            }
            changed.insert(
                frontier.clone(),
                frontier_navigation_bytes(&foreign, &frontier, Some(&hash("frontier-0")), 2),
            );
            assert!(matches!(
                discover_frontier_chain(&ordered, &expected, &mut || true, |hash| {
                    read_frontier_navigation_fixture(&changed, hash)
                }),
                Err(DailyReplayError::InvalidChain)
            ));
        }
    }

    #[test]
    fn i4g_paired_navigation_rejects_range_regression_and_changed_start() {
        let ChainFixture { tip, pins, .. } = chain(2);
        let (frontier, records) = frontier_plan_fixture(&pins);
        let expected = DailyObservationPins {
            admission: &tip,
            source_frontier: &frontier,
            owner: observation_owner(),
        };
        let ordered = pins.iter().rev().cloned().collect::<Vec<_>>();
        for change in 0..2 {
            let mut changed = records.clone();
            let first = hash("frontier-0");
            let bytes =
                frontier_navigation_bytes(&pins[0], &first, None, if change == 0 { 2 } else { 1 });
            let bytes = if change == 1 {
                String::from_utf8(bytes)
                    .unwrap()
                    .replace(
                        "\"range_start\":\"2026-09-01\"",
                        "\"range_start\":\"2026-08-31\"",
                    )
                    .into_bytes()
            } else {
                bytes
            };
            changed.insert(first, bytes);
            assert!(matches!(
                discover_frontier_chain(&ordered, &expected, &mut || true, |hash| {
                    read_frontier_navigation_fixture(&changed, hash)
                }),
                Err(DailyReplayError::InvalidChain)
            ));
        }
    }

    #[test]
    fn i4g_paired_navigation_cancellation_stops_before_or_after_each_read() {
        let ChainFixture { tip, pins, .. } = chain(2);
        let (frontier, records) = frontier_plan_fixture(&pins);
        let expected = DailyObservationPins {
            admission: &tip,
            source_frontier: &frontier,
            owner: observation_owner(),
        };
        let ordered = pins.into_iter().rev().collect::<Vec<_>>();
        assert!(matches!(
            discover_frontier_chain(&ordered, &expected, &mut || false, |_| panic!(
                "no read after cancellation"
            )),
            Err(DailyReplayError::Interrupted)
        ));
        let mut checks = 0;
        let mut reads = 0;
        let result = discover_frontier_chain(
            &ordered,
            &expected,
            &mut || {
                checks += 1;
                checks < 2
            },
            |hash| {
                reads += 1;
                read_frontier_navigation_fixture(&records, hash)
            },
        );
        assert!(matches!(result, Err(DailyReplayError::Interrupted)));
        assert_eq!(reads, 1);
    }

    #[test]
    fn i4g_paired_navigation_invalid_request_and_errors_do_not_become_proof() {
        let ChainFixture { tip, pins, .. } = chain(1);
        let frontier = hash("frontier-0");
        let mut expected = DailyObservationPins {
            admission: &tip,
            source_frontier: &frontier,
            owner: observation_owner(),
        };
        assert!(matches!(
            discover_frontier_chain(&[], &expected, &mut || true, |_| panic!(
                "empty plan must not read"
            )),
            Err(DailyReplayError::InvalidChain)
        ));
        expected.owner = uuid::Uuid::nil();
        assert!(matches!(
            discover_frontier_chain(&pins, &expected, &mut || true, |_| panic!(
                "nil owner must not read"
            )),
            Err(DailyReplayError::InvalidChain)
        ));
        expected.owner = observation_owner();
        assert!(matches!(
            discover_frontier_chain(&pins, &expected, &mut || true, |_| Err(
                DailyReplayError::Artifact(DailyArtifactError::SourceChanged)
            )),
            Err(DailyReplayError::Artifact(
                DailyArtifactError::SourceChanged
            ))
        ));
    }
    // C15 replay tests begin.

    // C15 production-shared ordinary replay contracts. No sealed/native
    // authority is created by these navigation records or ordinary row holders.
    struct ReturnedNavigation {
        bytes: Option<Vec<u8>>,
        reads: usize,
        borrows: std::cell::Cell<usize>,
        retired: Vec<Vec<u8>>,
        late: bool,
    }
    #[derive(Debug, PartialEq, Eq)]
    enum NavigationError {
        Policy(DailyReplayError),
        Late,
    }
    impl NavigationMode for ReturnedNavigation {
        type Metadata = Vec<u8>;
        type Error = NavigationError;
        fn error(&self, error: DailyReplayError) -> NavigationError {
            NavigationError::Policy(error)
        }
        fn checkpoint(
            &mut self,
            callback: &mut impl FnMut() -> bool,
        ) -> Result<(), NavigationError> {
            checkpoint(callback).map_err(NavigationError::Policy)
        }
        fn read(&mut self, _: &ContentHash) -> Result<Vec<u8>, NavigationError> {
            self.reads += 1;
            self.bytes
                .take()
                .ok_or(NavigationError::Policy(DailyReplayError::InvalidChain))
        }
        fn bytes<'a>(&'a self, bytes: &'a Vec<u8>) -> Result<&'a [u8], NavigationError> {
            self.borrows.set(self.borrows.get() + 1);
            Ok(bytes)
        }
        fn retire(&mut self, pending: &mut Option<Vec<u8>>) -> Result<(), NavigationError> {
            if let Some(bytes) = pending.take() {
                self.retired.push(bytes);
            }
            if self.late {
                Err(NavigationError::Late)
            } else {
                Ok(())
            }
        }
    }
    fn returned_navigation(bytes: Vec<u8>) -> ReturnedNavigation {
        ReturnedNavigation {
            bytes: Some(bytes),
            reads: 0,
            borrows: std::cell::Cell::new(0),
            retired: Vec::new(),
            late: false,
        }
    }
    #[test]
    fn checked_metadata_is_retained_before_post_callback_refusal_and_decode() {
        let (pins, bytes) = encode(&record(None, 1));
        let address = bytes.as_ptr();
        let mut mode = returned_navigation(bytes);
        let mut pending = None;
        let mut calls = 0;
        let result = discover_chain_with(
            &mut mode,
            &mut pending,
            &pins,
            &hash("policy"),
            DailyReplayLimits::new(1).unwrap(),
            &mut || {
                calls += 1;
                calls != 2
            },
        );
        assert_eq!(
            result,
            Err(NavigationError::Policy(DailyReplayError::Interrupted))
        );
        assert_eq!(mode.reads, 1);
        assert_eq!(mode.borrows.get(), 0);
        assert_eq!(pending.as_ref().unwrap().as_ptr(), address);
        assert!(mode.retire(&mut pending).is_ok());
        assert_eq!(mode.retired[0].as_ptr(), address);
        assert!(pending.is_none());
    }
    #[test]
    fn checked_metadata_consumption_preserves_bytes_when_retirement_is_late() {
        let (pins, bytes) = encode(&record(None, 1));
        let address = bytes.as_ptr();
        let mut mode = returned_navigation(bytes);
        mode.late = true;
        let mut pending = None;
        let result = discover_chain_with(
            &mut mode,
            &mut pending,
            &pins,
            &hash("policy"),
            DailyReplayLimits::new(1).unwrap(),
            &mut || true,
        );
        assert_eq!(result, Err(NavigationError::Late));
        assert!(pending.is_none());
        assert_eq!(mode.retired.len(), 1);
        assert_eq!(mode.retired[0].as_ptr(), address);
        assert_eq!(mode.borrows.get(), 1);
    }
    #[test]
    fn checked_navigation_keeps_policy_before_callback_and_lower_error_has_no_second_finish() {
        let (mut pins, bytes) = encode(&record(None, 1));
        pins.policy_sha256 = hash("different");
        let mut mode = returned_navigation(bytes);
        let mut pending = None;
        let mut calls = 0;
        assert_eq!(
            discover_chain_with(
                &mut mode,
                &mut pending,
                &pins,
                &hash("policy"),
                DailyReplayLimits::new(1).unwrap(),
                &mut || {
                    calls += 1;
                    true
                }
            ),
            Err(NavigationError::Policy(DailyReplayError::InvalidChain))
        );
        assert_eq!((mode.reads, calls), (0, 0));
        pins.policy_sha256 = hash("policy");
        mode.bytes = None;
        assert!(
            discover_chain_with(
                &mut mode,
                &mut pending,
                &pins,
                &hash("policy"),
                DailyReplayLimits::new(1).unwrap(),
                &mut || true
            )
            .is_err()
        );
        assert!(pending.is_none());
        assert!(mode.retired.is_empty());
    }
    struct OrdinaryTip {
        ordinal: usize,
        value: Box<str>,
    }
    struct OrdinaryReplay {
        plan: Vec<DailyPricePinsV1Wire>,
        reads: std::cell::Cell<usize>,
        retired: Vec<(OrdinaryTip, bool)>,
        late_predecessor: bool,
        fail_child: bool,
    }
    impl ReplayMode for OrdinaryReplay {
        type Tip = OrdinaryTip;
        type Error = NavigationError;
        fn discover(
            &mut self,
            _: &DailyPricePinsV1Wire,
            _: DailyReplayLimits,
            _: &mut impl FnMut() -> bool,
        ) -> Result<Vec<DailyPricePinsV1Wire>, NavigationError> {
            Ok(self.plan.clone())
        }
        fn checkpoint(
            &mut self,
            _: Option<&OrdinaryTip>,
            _: Option<&OrdinaryTip>,
            callback: &mut impl FnMut() -> bool,
        ) -> Result<(), NavigationError> {
            checkpoint(callback).map_err(NavigationError::Policy)
        }
        fn first(&self, _: &DailyPricePinsV1Wire) -> Result<OrdinaryTip, NavigationError> {
            self.reads.set(self.reads.get() + 1);
            Ok(OrdinaryTip {
                ordinal: self.reads.get(),
                value: "actual ordinary constructor return".into(),
            })
        }
        fn child(
            &self,
            pins: &DailyPricePinsV1Wire,
            _: &OrdinaryTip,
        ) -> Result<OrdinaryTip, NavigationError> {
            if self.fail_child {
                return Err(NavigationError::Policy(DailyReplayError::InvalidChain));
            }
            self.first(pins)
        }
        fn replace(
            &mut self,
            pending: &mut super::super::admission::sealed::Pending<OrdinaryTip>,
        ) -> Result<(), NavigationError> {
            if super::super::admission::sealed::promote(pending, |tip, predecessor| {
                self.retired.push((tip, predecessor));
                !(self.late_predecessor && predecessor)
            }) {
                Ok(())
            } else {
                Err(NavigationError::Late)
            }
        }
        fn tip(&self, _: &OrdinaryTip, _: &DailyPricePinsV1Wire) -> Result<(), NavigationError> {
            Ok(())
        }
        fn empty(&self) -> NavigationError {
            NavigationError::Policy(DailyReplayError::InvalidChain)
        }
    }
    fn ordinary_replay(count: u64) -> (OrdinaryReplay, DailyPricePinsV1Wire) {
        let fixture = chain(count);
        (
            OrdinaryReplay {
                plan: fixture.pins.into_iter().rev().collect(),
                reads: std::cell::Cell::new(0),
                retired: Vec::new(),
                late_predecessor: false,
                fail_child: false,
            },
            fixture.tip,
        )
    }
    #[test]
    fn checked_replay_stores_next_before_callback_and_drains_next_before_previous() {
        let (mut mode, pins) = ordinary_replay(2);
        let mut pending = super::super::admission::sealed::Pending::new();
        let mut callbacks = 0;
        let original = replay_with(
            &mut mode,
            &mut pending,
            &pins,
            DailyReplayLimits::new(2).unwrap(),
            &mut || {
                callbacks += 1;
                callbacks != 4
            },
        );
        assert_eq!(
            original,
            Err(NavigationError::Policy(DailyReplayError::Interrupted))
        );
        assert_eq!(pending.current.as_ref().unwrap().ordinal, 1);
        assert_eq!(pending.next.as_ref().unwrap().ordinal, 2);
        let address = pending.next.as_ref().unwrap().value.as_ptr();
        pending.drain(|tip| {
            mode.retired.push((tip, false));
        });
        assert_eq!(
            mode.retired.iter().map(|r| r.0.ordinal).collect::<Vec<_>>(),
            [2, 1]
        );
        assert_eq!(mode.retired[0].0.value.as_ptr(), address);
    }
    #[test]
    fn checked_replay_late_predecessor_preserves_child_and_blocks_third_read() {
        let (mut mode, pins) = ordinary_replay(3);
        mode.late_predecessor = true;
        let mut pending = super::super::admission::sealed::Pending::new();
        let result = replay_with(
            &mut mode,
            &mut pending,
            &pins,
            DailyReplayLimits::new(3).unwrap(),
            &mut || true,
        );
        assert_eq!(result, Err(NavigationError::Late));
        assert_eq!(mode.reads.get(), 2);
        assert_eq!(
            mode.retired
                .iter()
                .map(|r| (r.0.ordinal, r.1))
                .collect::<Vec<_>>(),
            [(1, true), (2, false)]
        );
        assert!(pending.current.is_none() && pending.next.is_none());
        assert_eq!(
            mode.retired[1].0.value.as_ref(),
            "actual ordinary constructor return"
        );
    }
    #[test]
    fn checked_replay_child_error_leaves_only_previous_for_mandatory_retirement() {
        let (mut mode, pins) = ordinary_replay(2);
        mode.fail_child = true;
        let mut pending = super::super::admission::sealed::Pending::new();
        let original = replay_with(
            &mut mode,
            &mut pending,
            &pins,
            DailyReplayLimits::new(2).unwrap(),
            &mut || true,
        );
        assert_eq!(
            original,
            Err(NavigationError::Policy(DailyReplayError::InvalidChain))
        );
        assert!(pending.next.is_none());
        pending.drain(|tip| {
            mode.retired.push((tip, true));
        });
        assert_eq!(mode.retired.len(), 1);
        assert_eq!(mode.retired[0].0.ordinal, 1);
    }

    // C15 replay tests end.
}

#[cfg(all(test, target_os = "linux"))]
mod c15_review_regressions {
    use super::checked::complete_callback_results;
    use super::*;
    use crate::checked_close::{Attempt, Gap};
    use crate::daily_price_only::admission::PinnedDailyPolicy;
    use crate::daily_price_only::admission::checked::{
        FirstKind, Source, policy_source, replay_source,
    };
    use crate::daily_price_only::artifact::{CheckedDailyAdmissionRoot, DailyArtifactFirstKind};
    use crate::raw_custody::RawInvocation;
    use std::path::Path;

    fn later_scope_failure(invocation: &RawInvocation<'_>, actual: &Attempt) -> Source {
        let foreign = Attempt::default();
        let failure = match CheckedDailyAdmissionRoot::open_in(
            invocation,
            &foreign,
            actual,
            Path::new("/must-not-open"),
            10003,
            10001,
        ) {
            Err(failure) => failure,
            Ok(_) => panic!("foreign scope must refuse before native work"),
        };
        assert_eq!(failure.first_kind(), Some(DailyArtifactFirstKind::Scope));
        Source::Artifact(Box::new(failure))
    }

    #[test]
    fn interrupted_precedes_later_scope_refusal_in_shared_callback_dispatch() {
        let actual = Attempt::default();
        let invocation = RawInvocation::new(&actual);
        let returned = checkpoint(&mut || {
            actual.gap(Gap::Abandoned);
            false
        });
        let after = Err(later_scope_failure(&invocation, &actual));
        let returned = returned.map_err(|error| replay_source(&invocation, &actual, error));
        let failure = complete_callback_results(returned, after).unwrap_err();
        assert_eq!(failure.first_kind(), Some(FirstKind::Replay));
        assert!(failure.report().is_none());
        let Source::Joined(pair) = failure else {
            panic!("both original and later sources must remain whole");
        };
        let Err(Source::Replay(first)) = &pair.0 else {
            panic!("actual interruption remains first");
        };
        assert!(matches!(first.body(), Some(DailyReplayError::Interrupted)));
        assert!(matches!(
            pair.1.as_ref().err().unwrap().first_kind(),
            Some(FirstKind::Scope)
        ));
        assert!(pair.1.as_ref().err().unwrap().report().is_some());
        assert_eq!(actual.report().live(), 0);
    }

    #[test]
    fn plain_policy_and_replay_keep_absent_original_reports_and_whole_diagnostics() {
        for replay in [false, true] {
            let actual = Attempt::default();
            let invocation = RawInvocation::new(&actual);
            let source = if replay {
                let error = checkpoint(&mut || false).unwrap_err();
                replay_source(&invocation, &actual, error)
            } else {
                let error = match PinnedDailyPolicy::from_pinned_bytes(
                    b"not-policy-json",
                    &ContentHash::from_bytes(b"different-pin"),
                ) {
                    Err(error) => error,
                    Ok(_) => panic!("invalid policy pin must refuse"),
                };
                policy_source(&invocation, &actual, error)
            };
            assert!(source.report().is_none());
            match &source {
                Source::Policy(failure) => assert!(failure.body().is_some()),
                Source::Replay(failure) => assert!(failure.body().is_some()),
                _ => panic!("actual canonical diagnostic must remain typed"),
            }
            actual.gap(Gap::Abandoned);
            let failure = complete_callback_results(
                Err(source),
                Err(later_scope_failure(&invocation, &actual)),
            )
            .unwrap_err();
            assert!(failure.report().is_none());
            assert_eq!(
                failure.first_kind(),
                Some(if replay {
                    FirstKind::Replay
                } else {
                    FirstKind::Policy
                }),
            );
            let Source::Joined(pair) = failure else {
                panic!("later diagnostic stays separate");
            };
            assert!(pair.1.as_ref().err().unwrap().report().is_some());
            assert_eq!(actual.report().live(), 0);
        }
    }
}
