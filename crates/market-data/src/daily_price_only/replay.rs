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
    let plan = discover_chain(
        expected,
        policy.sha256(),
        limits,
        &mut keep_reading,
        |hash| read_admission_metadata(root, hash).map_err(DailyReplayError::Artifact),
    )?;
    let mut current: Option<ReopenedDailyChain<'base>> = None;
    for pins in plan.iter().rev() {
        checkpoint(&mut keep_reading)?;
        let next = match current.as_ref() {
            None => SealedTip::First(
                read_sealed_historical_daily_input(root, pins, base, policy)
                    .map_err(DailyReplayError::Verification)?,
            ),
            Some(parent) => SealedTip::Continued(
                read_sealed_daily_continuation(root, pins, base, policy, &parent.as_parent())
                    .map_err(DailyReplayError::Verification)?,
            ),
        };
        checkpoint(&mut keep_reading)?;
        // The new sealed continuation does not borrow its predecessor. Release
        // all predecessor descriptors/buffers once the full comparison succeeds.
        current = Some(ReopenedDailyChain { tip: next });
    }
    let result = current.ok_or(DailyReplayError::InvalidChain)?;
    if result.as_parent().pins() != expected {
        return Err(DailyReplayError::InvalidChain);
    }
    // The leaf operation above already revalidated the returned descriptor set.
    // admitted() also revalidates before exposing a computation input.
    Ok(result)
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
    if expected.policy_sha256 != *policy {
        return Err(DailyReplayError::InvalidChain);
    }
    let mut result = Vec::new();
    let mut visited = HashSet::new();
    let mut requested = expected.admission_sha256.clone();
    let mut next_added = None;
    loop {
        checkpoint(keep_reading)?;
        if result.len() >= limits.max_admissions {
            return Err(DailyReplayError::LimitExceeded);
        }
        visited
            .try_reserve(1)
            .map_err(|_| DailyReplayError::LimitExceeded)?;
        if !visited.insert(requested.clone()) {
            return Err(DailyReplayError::InvalidChain);
        }
        let bytes = read(&requested)?;
        checkpoint(keep_reading)?;
        let record = decode_navigation(&bytes, &requested, policy)?;
        if record.added_sessions > limits.max_admissions as u64 {
            return Err(DailyReplayError::LimitExceeded);
        }
        if next_added.is_some_and(|count| count != record.added_sessions)
            || (result.is_empty() && record.pins != *expected)
        {
            return Err(DailyReplayError::InvalidChain);
        }
        result
            .try_reserve(1)
            .map_err(|_| DailyReplayError::LimitExceeded)?;
        result.push(record.pins);
        match (record.parent, record.added_sessions) {
            (None, 1) => return Ok(result),
            (Some(parent), count) if count > 1 => {
                requested = parent;
                next_added = Some(count - 1);
            }
            _ => return Err(DailyReplayError::InvalidChain),
        }
    }
}

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
}
