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
/// Explicit limits for retaining every captured Daily leaf. These are resource
/// bounds, not admission, source-discovery or current authorization evidence.
/// `max_validation_bytes` bounds the visitor's whole-history byte checks per
/// visit. Native reconstruction and Raw verification retain their own source
/// limits; this value is not an aggregate I/O, allocation or elapsed-time meter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DailyHistoryLimits {
    max_admissions: usize,
    max_retained_bytes: u64,
    max_validation_bytes: u64,
}
impl DailyHistoryLimits {
    pub fn new(
        max_admissions: u64,
        max_retained_bytes: u64,
        max_validation_bytes: u64,
    ) -> Result<Self, DailyReplayError> {
        let source = fixed_limits();
        let maximum_work = source
            .max_bars_bytes
            .checked_mul(source.max_sessions)
            .ok_or(DailyReplayError::InvalidLimits)?;
        if max_admissions == 0
            || max_admissions > source.max_sessions
            || max_retained_bytes == 0
            || max_retained_bytes > source.max_bars_bytes
            || max_validation_bytes == 0
            || max_validation_bytes > maximum_work
        {
            return Err(DailyReplayError::InvalidLimits);
        }
        Ok(Self {
            max_admissions: usize::try_from(max_admissions)
                .map_err(|_| DailyReplayError::InvalidLimits)?,
            max_retained_bytes,
            max_validation_bytes,
        })
    }
}

/// The complete captured names and every reopened sealed leaf remain owned.
/// This proves file/parent/frontier closure only; independent Raw reconstruction
/// remains mandatory. No caller list, latest pointer or copied seal constructs it.
pub(crate) struct CapturedDailyHistory<'base> {
    admissions: super::artifact::CapturedDailyArtifactNames,
    frontiers: super::artifact::CapturedDailyArtifactNames,
    nodes: Vec<ReopenedDailyObservation<'base>>,
    parents: Vec<Option<usize>>,
    retained_bytes: u64,
    max_validation_bytes: u64,
}

impl<'base> CapturedDailyHistory<'base> {
    pub(crate) fn revalidate(&self) -> Result<(), DailyReplayError> {
        // Keep checking later owners even after one failed. No cached clean bit.
        let mut first = None;
        for result in [
            self.admissions
                .revalidate()
                .map_err(DailyReplayError::Artifact),
            self.frontiers
                .revalidate()
                .map_err(DailyReplayError::Artifact),
        ] {
            if first.is_none() {
                first = result.err();
            }
        }
        for node in &self.nodes {
            for result in [
                node.as_parent()
                    .revalidate()
                    .map_err(DailyReplayError::Verification),
                node.frontier
                    .revalidate()
                    .map_err(DailyReplayError::Verification),
            ] {
                if first.is_none() {
                    first = result.err();
                }
            }
        }
        match first {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    pub(crate) fn visit(
        &self,
        keep_reading: &mut impl FnMut() -> bool,
        mut consume: impl FnMut(
            &ReopenedDailyObservation<'base>,
            Option<&ReopenedDailyObservation<'base>>,
        ) -> Result<(), DailyReplayError>,
    ) -> Result<(), DailyReplayError> {
        let work = history_validation_work(self.retained_bytes, self.nodes.len())?;
        if work > self.max_validation_bytes {
            return Err(DailyReplayError::LimitExceeded);
        }
        checkpoint(keep_reading)?;
        self.revalidate()?;
        for (index, node) in self.nodes.iter().enumerate() {
            checkpoint(keep_reading)?;
            self.revalidate()?;
            let result = consume(node, self.parents[index].map(|parent| &self.nodes[parent]));
            // A consumer failure cannot suppress the post-use original check.
            let custody = self.revalidate();
            result?;
            custody?;
            checkpoint(keep_reading)?;
        }
        checkpoint(keep_reading)?;
        self.revalidate()
    }
}

fn history_validation_work(retained_bytes: u64, nodes: usize) -> Result<u64, DailyReplayError> {
    let nodes = u64::try_from(nodes).map_err(|_| DailyReplayError::LimitExceeded)?;
    let passes = nodes
        .checked_mul(2)
        .and_then(|value| value.checked_add(2))
        .ok_or(DailyReplayError::LimitExceeded)?;
    retained_bytes
        .checked_mul(passes)
        .ok_or(DailyReplayError::LimitExceeded)
}

struct HistoryNavigation {
    admission: Navigation,
    frontier: ContentHash,
    owner: uuid::Uuid,
    parent_frontier: Option<ContentHash>,
    range_start: domain::TradingDate,
    range_end: domain::TradingDate,
}

/// Both collections come only from the captured complete roots in production.
/// Keeping this algorithm independent permits exhaustive malformed-graph tests;
/// a navigation result is deliberately not a sealed or source capability.
fn order_history(
    admissions: Vec<Navigation>,
    frontiers: Vec<(ContentHash, super::frontier::DailyFrontierNavigation)>,
    limits: DailyHistoryLimits,
) -> Result<Vec<(HistoryNavigation, Option<usize>)>, DailyReplayError> {
    use std::collections::HashMap;
    if admissions.len() > limits.max_admissions || frontiers.len() > limits.max_admissions {
        return Err(DailyReplayError::LimitExceeded);
    }
    if admissions.len() != frontiers.len() {
        return Err(DailyReplayError::InvalidChain);
    }
    let mut by_admission = HashMap::new();
    by_admission
        .try_reserve(admissions.len())
        .map_err(|_| DailyReplayError::LimitExceeded)?;
    for admission in admissions {
        let key = admission.pins.admission_sha256.clone();
        if admission.added_sessions == 0
            || admission.added_sessions > limits.max_admissions as u64
            || by_admission.insert(key, admission).is_some()
        {
            return Err(DailyReplayError::InvalidChain);
        }
    }
    let mut frontier_names = HashSet::new();
    frontier_names
        .try_reserve(frontiers.len())
        .map_err(|_| DailyReplayError::LimitExceeded)?;
    let mut nodes = Vec::new();
    nodes
        .try_reserve_exact(frontiers.len())
        .map_err(|_| DailyReplayError::LimitExceeded)?;
    for (frontier, navigation) in frontiers {
        if !frontier_names.insert(frontier.clone()) || navigation.owner().is_nil() {
            return Err(DailyReplayError::InvalidChain);
        }
        let admission = by_admission
            .remove(navigation.admission_sha256())
            .ok_or(DailyReplayError::InvalidChain)?;
        if admission.pins.policy_sha256 != *navigation.policy_sha256() {
            return Err(DailyReplayError::InvalidChain);
        }
        nodes.push(HistoryNavigation {
            admission,
            frontier,
            owner: navigation.owner(),
            parent_frontier: navigation.parent_frontier_sha256().cloned(),
            range_start: navigation.range_start(),
            range_end: navigation.range_end(),
        });
    }
    if !by_admission.is_empty() {
        return Err(DailyReplayError::InvalidChain);
    }
    nodes.sort_by(|left, right| {
        left.admission
            .added_sessions
            .cmp(&right.admission.added_sessions)
            .then_with(|| {
                left.admission
                    .pins
                    .admission_sha256
                    .as_str()
                    .cmp(right.admission.pins.admission_sha256.as_str())
            })
    });
    let mut seen: HashMap<ContentHash, usize> = HashMap::new();
    seen.try_reserve(nodes.len())
        .map_err(|_| DailyReplayError::LimitExceeded)?;
    let mut ordered: Vec<(HistoryNavigation, Option<usize>)> = Vec::new();
    ordered
        .try_reserve_exact(nodes.len())
        .map_err(|_| DailyReplayError::LimitExceeded)?;
    for node in nodes {
        let parent = match &node.admission.parent {
            None => {
                if node.admission.added_sessions != 1 || node.parent_frontier.is_some() {
                    return Err(DailyReplayError::InvalidChain);
                }
                None
            }
            Some(hash) => {
                let index = *seen.get(hash).ok_or(DailyReplayError::InvalidChain)?;
                let previous = &ordered[index].0;
                if previous.admission.added_sessions.checked_add(1)
                    != Some(node.admission.added_sessions)
                    || node.parent_frontier.as_ref() != Some(&previous.frontier)
                    || node.owner != previous.owner
                    || node.admission.pins.policy_sha256 != previous.admission.pins.policy_sha256
                    || node.range_start != previous.range_start
                    || node.range_end <= previous.range_end
                {
                    return Err(DailyReplayError::InvalidChain);
                }
                Some(index)
            }
        };
        seen.insert(node.admission.pins.admission_sha256.clone(), ordered.len());
        ordered.push((node, parent));
    }
    Ok(ordered)
}

struct CapturedHistoryNavigation {
    admissions: super::artifact::CapturedDailyArtifactNames,
    frontiers: super::artifact::CapturedDailyArtifactNames,
    ordered: Vec<(HistoryNavigation, Option<usize>)>,
}

fn capture_history_navigation(
    admissions: &DailyAdmissionRoot,
    frontiers: &super::artifact::DailySourceFrontierRoot,
    policy: &ContentHash,
    limits: DailyHistoryLimits,
    mut keep_reading: impl FnMut() -> bool,
) -> Result<CapturedHistoryNavigation, DailyReplayError> {
    let name_limits = super::artifact::DailyArtifactNameLimits::new(limits.max_admissions as u64)
        .map_err(DailyReplayError::Artifact)?;
    checkpoint(&mut keep_reading)?;
    let admission_names = admissions
        .capture_names(name_limits)
        .map_err(DailyReplayError::Artifact)?;
    let frontier_names = frontiers
        .capture_names(name_limits)
        .map_err(DailyReplayError::Artifact)?;
    let revalidate_names = || {
        let first = admission_names
            .revalidate()
            .map_err(DailyReplayError::Artifact);
        let second = frontier_names
            .revalidate()
            .map_err(DailyReplayError::Artifact);
        first.and(second)
    };
    revalidate_names()?;
    let mut admission_records = Vec::new();
    admission_records
        .try_reserve_exact(admission_names.names().len())
        .map_err(|_| DailyReplayError::LimitExceeded)?;
    let mut frontier_records = Vec::new();
    frontier_records
        .try_reserve_exact(frontier_names.names().len())
        .map_err(|_| DailyReplayError::LimitExceeded)?;
    for name in admission_names.names() {
        checkpoint(&mut keep_reading)?;
        revalidate_names()?;
        let bytes =
            read_admission_metadata(admissions, name).map_err(DailyReplayError::Artifact)?;
        admission_records.push(decode_navigation(&bytes, name, policy)?);
        revalidate_names()?;
        checkpoint(&mut keep_reading)?;
    }
    for name in frontier_names.names() {
        checkpoint(&mut keep_reading)?;
        revalidate_names()?;
        let record = super::frontier::read_daily_frontier_navigation(frontiers, name)
            .map_err(DailyReplayError::Verification)?;
        frontier_records.push((name.clone(), record));
        revalidate_names()?;
        checkpoint(&mut keep_reading)?;
    }
    let ordered = order_history(admission_records, frontier_records, limits)?;
    revalidate_names()?;
    Ok(CapturedHistoryNavigation {
        admissions: admission_names,
        frontiers: frontier_names,
        ordered,
    })
}

pub(crate) fn read_captured_daily_history<'base>(
    admissions: &DailyAdmissionRoot,
    frontiers: &super::artifact::DailySourceFrontierRoot,
    base: &'base ApprovedHistoricalPriceOnlyArtifact,
    policy: &'base PinnedDailyPolicy,
    limits: DailyHistoryLimits,
    mut keep_reading: impl FnMut() -> bool,
) -> Result<CapturedDailyHistory<'base>, DailyReplayError> {
    let CapturedHistoryNavigation {
        admissions: admission_names,
        frontiers: frontier_names,
        ordered,
    } = capture_history_navigation(
        admissions,
        frontiers,
        policy.sha256(),
        limits,
        &mut keep_reading,
    )?;
    let revalidate_names = || {
        let first = admission_names
            .revalidate()
            .map_err(DailyReplayError::Artifact);
        let second = frontier_names
            .revalidate()
            .map_err(DailyReplayError::Artifact);
        first.and(second)
    };
    let mut nodes: Vec<ReopenedDailyObservation<'base>> = Vec::new();
    nodes
        .try_reserve_exact(ordered.len())
        .map_err(|_| DailyReplayError::LimitExceeded)?;
    let mut parents = Vec::new();
    parents
        .try_reserve_exact(ordered.len())
        .map_err(|_| DailyReplayError::LimitExceeded)?;
    let mut retained_bytes = 0u64;
    for (navigation, parent) in ordered {
        checkpoint(&mut keep_reading)?;
        revalidate_names()?;
        let previous = parent.map(|index| &nodes[index]);
        let tip = match previous {
            None => SealedTip::First(
                read_sealed_historical_daily_input(
                    admissions,
                    &navigation.admission.pins,
                    base,
                    policy,
                )
                .map_err(DailyReplayError::Verification)?,
            ),
            Some(previous) => SealedTip::Continued(
                read_sealed_daily_continuation(
                    admissions,
                    &navigation.admission.pins,
                    base,
                    policy,
                    &previous.as_parent(),
                )
                .map_err(DailyReplayError::Verification)?,
            ),
        };
        let chain = ReopenedDailyChain { tip };
        let frontier = super::frontier::read_sealed_source_frontier(
            frontiers,
            &navigation.frontier,
            navigation.owner,
            &chain.as_parent(),
            previous.map(|value| &value.frontier),
        )
        .map_err(DailyReplayError::Verification)?;
        let node = ReopenedDailyObservation { chain, frontier };
        let input = node.as_parent();
        let opened = input.opened();
        let admission_bytes = match &input {
            SealedDailyParent::First(value) => value.admission_bytes(),
            SealedDailyParent::Continued(value) => value.admission_bytes(),
        };
        // Retained original byte volume is explicit. The existing per-document,
        // parsed row/depth/session and descriptor limits also remain enforced.
        for count in [
            admission_bytes.len(),
            opened.manifest_bytes().len(),
            opened.normalization_bytes().len(),
            opened.bars_ndjson().len(),
            node.frontier
                .retained_byte_len()
                .ok_or(DailyReplayError::LimitExceeded)?,
            input
                .bars()
                .len()
                .checked_mul(std::mem::size_of::<crate::HistoricalPriceOnlyBar>())
                .ok_or(DailyReplayError::LimitExceeded)?,
        ] {
            retained_bytes = retained_bytes
                .checked_add(count as u64)
                .ok_or(DailyReplayError::LimitExceeded)?;
            if retained_bytes > limits.max_retained_bytes {
                return Err(DailyReplayError::LimitExceeded);
            }
        }
        input.revalidate().map_err(DailyReplayError::Verification)?;
        node.frontier
            .revalidate()
            .map_err(DailyReplayError::Verification)?;
        revalidate_names()?;
        checkpoint(&mut keep_reading)?;
        nodes.push(node);
        parents.push(parent);
    }
    revalidate_names()?;
    let result = CapturedDailyHistory {
        admissions: admission_names,
        frontiers: frontier_names,
        nodes,
        parents,
        retained_bytes,
        max_validation_bytes: limits.max_validation_bytes,
    };
    checkpoint(&mut keep_reading)?;
    result.revalidate()?;
    Ok(result)
}

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

    fn history_limits() -> DailyHistoryLimits {
        DailyHistoryLimits::new(16, 16 * 1024 * 1024, 128 * 1024 * 1024).unwrap()
    }

    fn history_navigation_fixture(
        count: u64,
    ) -> (
        Vec<Navigation>,
        Vec<(ContentHash, super::super::frontier::DailyFrontierNavigation)>,
    ) {
        let ChainFixture { records, pins, .. } = chain(count);
        let (_, frontiers) = frontier_plan_fixture(&pins);
        let admissions = pins
            .iter()
            .map(|pin| {
                decode_navigation(
                    &records[&pin.admission_sha256],
                    &pin.admission_sha256,
                    &hash("policy"),
                )
                .unwrap()
            })
            .collect();
        let frontiers = frontiers
            .iter()
            .map(|(name, bytes)| {
                (
                    name.clone(),
                    super::super::frontier::decode_frontier_navigation(bytes, name).unwrap(),
                )
            })
            .collect();
        (admissions, frontiers)
    }

    #[test]
    fn archive_history_navigation_visits_all_first_and_continued_records() {
        let (mut admissions, mut frontiers) = history_navigation_fixture(3);
        admissions.reverse();
        frontiers.reverse();
        let ordered = order_history(admissions, frontiers, history_limits()).unwrap();
        assert_eq!(ordered.len(), 3);
        assert_eq!(
            ordered
                .iter()
                .map(|(node, parent)| (node.admission.added_sessions, *parent))
                .collect::<Vec<_>>(),
            vec![(1, None), (2, Some(0)), (3, Some(1))]
        );
        assert_eq!(ordered[0].0.frontier, hash("frontier-0"));
        assert_eq!(ordered[2].0.frontier, hash("frontier-2"));
    }

    #[test]
    fn archive_history_navigation_retains_standalone_first_admission() {
        let (mut admissions, mut frontiers) = history_navigation_fixture(2);
        let mut other = record(None, 1);
        other.evidence_sha256 = hash("independent-standalone");
        let (pins, bytes) = encode(&other);
        admissions
            .push(decode_navigation(&bytes, &pins.admission_sha256, &pins.policy_sha256).unwrap());
        let name = hash("standalone-frontier");
        let bytes = frontier_navigation_bytes(&pins, &name, None, 1);
        frontiers.push((
            name.clone(),
            super::super::frontier::decode_frontier_navigation(&bytes, &name).unwrap(),
        ));
        let ordered = order_history(admissions, frontiers, history_limits()).unwrap();
        assert_eq!(ordered.len(), 3);
        assert_eq!(
            ordered
                .iter()
                .filter(|(_, parent)| parent.is_none())
                .count(),
            2
        );
        assert!(ordered.iter().any(|(node, _)| node.admission.pins == pins));
    }

    #[test]
    fn archive_history_navigation_rejects_missing_and_unused_captured_records() {
        for which in 0..4 {
            let (mut admissions, mut frontiers) = history_navigation_fixture(2);
            match which {
                0 => {
                    admissions.pop();
                }
                1 => {
                    frontiers.pop();
                }
                2 => {
                    admissions[1].parent = Some(hash("missing-parent"));
                }
                _ => {
                    let extra = hash("unused-frontier");
                    let bytes = frontier_navigation_bytes(&admissions[1].pins, &extra, None, 2);
                    frontiers.push((
                        extra.clone(),
                        super::super::frontier::decode_frontier_navigation(&bytes, &extra).unwrap(),
                    ));
                }
            }
            assert!(matches!(
                order_history(admissions, frontiers, history_limits()),
                Err(DailyReplayError::InvalidChain)
            ));
        }
    }

    #[test]
    fn archive_history_navigation_rejects_duplicate_admission_or_frontier() {
        for duplicate_frontier in [false, true] {
            let (mut admissions, mut frontiers) = history_navigation_fixture(2);
            if duplicate_frontier {
                frontiers[1].0 = frontiers[0].0.clone();
            } else {
                admissions[1].pins.admission_sha256 = admissions[0].pins.admission_sha256.clone();
            }
            assert!(matches!(
                order_history(admissions, frontiers, history_limits()),
                Err(DailyReplayError::InvalidChain)
            ));
        }
    }

    #[test]
    fn archive_history_navigation_rejects_cycle_depth_skip_and_first_parent() {
        for change in 0..4 {
            let (mut admissions, frontiers) = history_navigation_fixture(2);
            match change {
                0 => admissions[1].parent = Some(admissions[1].pins.admission_sha256.clone()),
                1 => admissions[0].parent = Some(admissions[1].pins.admission_sha256.clone()),
                2 => admissions[1].added_sessions = 3,
                _ => admissions[0].added_sessions = 2,
            }
            assert!(matches!(
                order_history(admissions, frontiers, history_limits()),
                Err(DailyReplayError::InvalidChain)
            ));
        }
    }

    #[test]
    fn archive_history_navigation_rejects_owner_policy_frontier_and_range_substitution() {
        for change in 0..5 {
            let (admissions, mut frontiers) = history_navigation_fixture(2);
            let key = hash("frontier-1");
            let index = frontiers.iter().position(|(hash, _)| hash == &key).unwrap();
            let parent = if change == 2 {
                hash("unrelated-frontier")
            } else {
                hash("frontier-0")
            };
            let bytes = frontier_navigation_bytes(&admissions[1].pins, &key, Some(&parent), 2);
            let text = String::from_utf8(bytes).unwrap();
            let text = match change {
                0 => text.replace(
                    &observation_owner().to_string(),
                    &uuid::Uuid::from_u128(999).to_string(),
                ),
                1 => text.replace(hash("policy").as_str(), hash("different-policy").as_str()),
                3 => text.replace(
                    "\"range_end\":\"2026-09-02\"",
                    "\"range_end\":\"2026-09-01\"",
                ),
                4 => text.replace(
                    "\"range_start\":\"2026-09-01\"",
                    "\"range_start\":\"2026-09-02\"",
                ),
                _ => text,
            };
            frontiers[index].1 =
                super::super::frontier::decode_frontier_navigation(text.as_bytes(), &key).unwrap();
            assert!(matches!(
                order_history(admissions, frontiers, history_limits()),
                Err(DailyReplayError::InvalidChain)
            ));
        }
    }

    #[test]
    fn archive_history_limits_reject_zero_overflow_and_oversized_inventory() {
        for values in [
            (0, 1, 1),
            (1, 0, 1),
            (1, 1, 0),
            (u64::MAX, 1, 1),
            (1, u64::MAX, 1),
            (1, 1, u64::MAX),
        ] {
            assert_eq!(
                DailyHistoryLimits::new(values.0, values.1, values.2),
                Err(DailyReplayError::InvalidLimits)
            );
        }
        let (admissions, frontiers) = history_navigation_fixture(2);
        let limits = DailyHistoryLimits::new(1, 1024, 1024).unwrap();
        assert!(matches!(
            order_history(admissions, frontiers, limits),
            Err(DailyReplayError::LimitExceeded)
        ));
    }

    #[test]
    fn archive_history_validation_work_counts_each_visit_and_rejects_overflow() {
        // Entry/exit plus before/after every callback, independently counted.
        assert_eq!(history_validation_work(7, 0), Ok(14));
        assert_eq!(history_validation_work(7, 1), Ok(28));
        assert_eq!(history_validation_work(7, 3), Ok(56));
        assert_eq!(history_validation_work(0, 3), Ok(0));
        assert_eq!(
            history_validation_work(u64::MAX, 0),
            Err(DailyReplayError::LimitExceeded)
        );
        if usize::BITS == 64 {
            assert_eq!(
                history_validation_work(1, usize::MAX),
                Err(DailyReplayError::LimitExceeded)
            );
        }
    }

    // These fixtures deliberately exercise protected navigation only. Their
    // opaque other members cannot create an approved base or sealed source proof.
    #[cfg(target_os = "linux")]
    struct HistoryNavigationFiles {
        _temporary: tempfile::TempDir,
        admissions_path: std::path::PathBuf,
        frontiers_path: std::path::PathBuf,
        admissions: DailyAdmissionRoot,
        frontiers: super::super::artifact::DailySourceFrontierRoot,
    }
    #[cfg(target_os = "linux")]
    impl HistoryNavigationFiles {
        fn new(count: u64) -> Self {
            use std::os::unix::fs::PermissionsExt;
            let parent = std::env::var_os("LAGRANGE_DAILY_ARTIFACT_TEST_ROOT")
                .unwrap_or_else(|| std::env::var_os("HOME").expect("protected fixture home"));
            let temporary = tempfile::Builder::new()
                .prefix(".daily-history-")
                .tempdir_in(std::path::PathBuf::from(parent))
                .unwrap();
            let admissions_path = temporary.path().join("admitted");
            let frontiers_path = temporary.path().join("frontiers");
            for path in [&admissions_path, &frontiers_path] {
                std::fs::create_dir(path).unwrap();
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o750)).unwrap();
            }
            let (records, frontiers) = if count == 0 {
                (BTreeMap::new(), BTreeMap::new())
            } else {
                let ChainFixture { records, pins, .. } = chain(count);
                (records, frontier_plan_fixture(&pins).1)
            };
            for (name, bytes) in &records {
                let leaf = admissions_path.join(name.as_str().strip_prefix("sha256:").unwrap());
                std::fs::create_dir(&leaf).unwrap();
                std::fs::set_permissions(&leaf, std::fs::Permissions::from_mode(0o750)).unwrap();
                for member in [
                    "admission.json",
                    "manifest.json",
                    "bars.ndjson",
                    "normalization.json",
                ] {
                    let path = leaf.join(member);
                    std::fs::write(
                        &path,
                        if member == "admission.json" {
                            bytes.as_slice()
                        } else {
                            b"unapproved navigation-only member"
                        },
                    )
                    .unwrap();
                    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o440)).unwrap();
                }
            }
            for (name, bytes) in &frontiers {
                let leaf = frontiers_path.join(name.as_str().strip_prefix("sha256:").unwrap());
                std::fs::create_dir(&leaf).unwrap();
                std::fs::set_permissions(&leaf, std::fs::Permissions::from_mode(0o750)).unwrap();
                for member in ["frontier.json", "observation.json"] {
                    let path = leaf.join(member);
                    std::fs::write(
                        &path,
                        if member == "frontier.json" {
                            bytes.as_slice()
                        } else {
                            b"not a sealed observation"
                        },
                    )
                    .unwrap();
                    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o440)).unwrap();
                }
            }
            let owner = rustix::process::geteuid().as_raw();
            let group = rustix::process::getegid().as_raw();
            let admissions = DailyAdmissionRoot::open(&admissions_path, owner, group).unwrap();
            let frontiers = super::super::artifact::DailySourceFrontierRoot::open(
                &frontiers_path,
                owner,
                group,
            )
            .unwrap();
            Self {
                _temporary: temporary,
                admissions_path,
                frontiers_path,
                admissions,
                frontiers,
            }
        }
        fn capture(&self) -> Result<CapturedHistoryNavigation, DailyReplayError> {
            capture_history_navigation(
                &self.admissions,
                &self.frontiers,
                &hash("policy"),
                history_limits(),
                || true,
            )
        }
        fn capture_empty(&self) -> CapturedDailyHistory<'static> {
            let captured = self.capture().unwrap();
            assert!(captured.ordered.is_empty());
            CapturedDailyHistory {
                admissions: captured.admissions,
                frontiers: captured.frontiers,
                nodes: Vec::new(),
                parents: Vec::new(),
                retained_bytes: 0,
                max_validation_bytes: 1,
            }
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn archive_history_actual_capture_includes_every_protected_name() {
        let fixture = HistoryNavigationFiles::new(3);
        let captured = fixture.capture().unwrap();
        assert_eq!(captured.admissions.names().len(), 3);
        assert_eq!(captured.frontiers.names().len(), 3);
        assert_eq!(captured.ordered.len(), 3);
        captured.admissions.revalidate().unwrap();
        captured.frontiers.revalidate().unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn archive_history_actual_missing_frontier_is_not_a_smaller_success() {
        let fixture = HistoryNavigationFiles::new(2);
        let missing = fixture
            .frontiers_path
            .join(hash("frontier-1").as_str().strip_prefix("sha256:").unwrap());
        std::fs::remove_dir_all(missing).unwrap();
        assert!(matches!(
            fixture.capture(),
            Err(DailyReplayError::InvalidChain)
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn archive_history_actual_capture_detects_same_name_directory_replacement() {
        let fixture = HistoryNavigationFiles::new(2);
        let captured = fixture.capture().unwrap();
        let name = captured.admissions.names()[0]
            .as_str()
            .strip_prefix("sha256:")
            .unwrap();
        let leaf = fixture.admissions_path.join(name);
        let moved = fixture.admissions_path.join("moved");
        std::fs::rename(&leaf, &moved).unwrap();
        std::fs::create_dir(&leaf).unwrap();
        assert!(captured.admissions.revalidate().is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn archive_history_actual_capture_refuses_unknown_members_and_changed_record_bytes() {
        for extra in [false, true] {
            let fixture = HistoryNavigationFiles::new(2);
            if extra {
                std::fs::create_dir(fixture.admissions_path.join("unknown-original")).unwrap();
            } else {
                let leaf = fixture
                    .frontiers_path
                    .join(hash("frontier-1").as_str().strip_prefix("sha256:").unwrap());
                std::fs::write(leaf.join("frontier.json"), b"changed metadata").unwrap();
            }
            assert!(fixture.capture().is_err());
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn archive_history_actual_capture_cancellation_has_no_returned_owner() {
        let fixture = HistoryNavigationFiles::new(2);
        for stop in 0..10 {
            let mut calls = 0;
            let result = capture_history_navigation(
                &fixture.admissions,
                &fixture.frontiers,
                &hash("policy"),
                history_limits(),
                || {
                    calls += 1;
                    calls <= stop
                },
            );
            assert!(matches!(result, Err(DailyReplayError::Interrupted)));
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn archive_history_empty_inventory_visit_keeps_actual_roots_live() {
        let fixture = HistoryNavigationFiles::new(0);
        let captured = fixture.capture_empty();
        let mut visited = 0;
        let mut checkpoints = 0;
        captured
            .visit(
                &mut || {
                    checkpoints += 1;
                    true
                },
                |_, _| {
                    visited += 1;
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(visited, 0);
        assert_eq!(checkpoints, 2);
        captured.revalidate().unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn archive_history_final_checkpoint_cannot_append_an_unchecked_original() {
        let fixture = HistoryNavigationFiles::new(0);
        let captured = fixture.capture_empty();
        let mut checkpoints = 0;
        let outcome = captured.visit(
            &mut || {
                checkpoints += 1;
                if checkpoints == 2 {
                    std::fs::create_dir(fixture.admissions_path.join("new-original")).unwrap();
                }
                true
            },
            |_, _| panic!("empty captured inventory has no callback rows"),
        );
        assert_eq!(checkpoints, 2);
        assert!(matches!(outcome, Err(DailyReplayError::Artifact(_))));
        assert!(captured.revalidate().is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn archive_history_empty_inventory_cancellation_cannot_be_success() {
        for stop in 0..2 {
            let fixture = HistoryNavigationFiles::new(0);
            let captured = fixture.capture_empty();
            let mut checkpoints = 0;
            let outcome = captured.visit(
                &mut || {
                    checkpoints += 1;
                    checkpoints <= stop
                },
                |_, _| panic!("empty captured inventory has no callback rows"),
            );
            assert!(matches!(outcome, Err(DailyReplayError::Interrupted)));
        }
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
