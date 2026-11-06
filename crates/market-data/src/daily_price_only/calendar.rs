//! Deterministic calendar evidence from already captured original provider bytes.
//! These private-constructor results are not admission or database capabilities.

use std::collections::BTreeMap;

use chrono::{Datelike, FixedOffset, Timelike};
use domain::{ContentHash, TradingDate, UtcTimestamp};
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use super::contract::validate_calendar_request;
use super::verify::{
    DailyVerificationError, ResolvedOriginal, canonical_json, parse_provider_json,
};
use super::{
    DailyEvidenceKindV1, DailyFileEvidenceV1Wire, DailyFileKeyV1Wire,
    DailyNormalizedDocumentV1Wire, DailyNormalizedSourceCommitmentV1Wire, DailySessionTypeV1,
    daily_normalized_source_preimage_v1,
};
use crate::normalize::validated_kis_calendar_row;

const NORMALIZER_VERSION: &str = "daily-raw-reconstruction-v1";
const MAX_DAYS: usize = 370;
const MAX_ROW_CLAIMS: usize = 262_144;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum CalendarEvidenceKind {
    #[serde(rename = "REQUESTED_TARGET")]
    RequestedTarget,
    #[serde(rename = "DERIVED_SURROUNDING")]
    DerivedSurrounding,
}

/// Exactly the five fields accepted by SQL0060 p_days, with read-only access.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VerifiedCalendarDay {
    session_date: TradingDate,
    session_type: DailySessionTypeV1,
    evidence_kind: CalendarEvidenceKind,
    source_row_ordinal: u32,
    source_row_sha256: ContentHash,
}

impl VerifiedCalendarDay {
    pub fn session_date(&self) -> TradingDate {
        self.session_date
    }
    pub fn session_type(&self) -> DailySessionTypeV1 {
        self.session_type
    }
    pub fn evidence_kind(&self) -> CalendarEvidenceKind {
        self.evidence_kind
    }
    pub fn source_row_ordinal(&self) -> u32 {
        self.source_row_ordinal
    }
    pub fn source_row_sha256(&self) -> &ContentHash {
        &self.source_row_sha256
    }
}

/// Carries the original byte borrow, so projection cannot outlive its snapshot.
pub struct VerifiedCalendarProjection<'snapshot> {
    owner: Uuid,
    policy: ContentHash,
    source_file: DailyFileEvidenceV1Wire,
    canonical_file: Vec<u8>,
    evidence_sha256: ContentHash,
    days: Vec<VerifiedCalendarDay>,
    observed_at: UtcTimestamp,
    _original_bytes: &'snapshot [u8],
}

impl VerifiedCalendarProjection<'_> {
    pub fn owner(&self) -> Uuid {
        self.owner
    }
    pub fn policy_sha256(&self) -> &ContentHash {
        &self.policy
    }
    pub fn evidence_sha256(&self) -> &ContentHash {
        &self.evidence_sha256
    }
    pub fn source_file(&self) -> &DailyFileEvidenceV1Wire {
        &self.source_file
    }
    pub fn canonical_file_bytes(&self) -> &[u8] {
        &self.canonical_file
    }
    pub fn days(&self) -> &[VerifiedCalendarDay] {
        &self.days
    }
    pub fn observed_at(&self) -> UtcTimestamp {
        self.observed_at
    }

    pub fn document(
        &self,
        date: TradingDate,
    ) -> Result<ReconstructedCalendarDocument, DailyVerificationError> {
        let index = self
            .days
            .binary_search_by_key(&date, |day| day.session_date)
            .map_err(|_| DailyVerificationError::CalendarCoverageGap)?;
        let day = &self.days[index];
        let key = DailyFileKeyV1Wire {
            provider: self.source_file.provider.clone(),
            market: self.source_file.market.clone(),
            batch_id: self.source_file.batch_id,
            file_name: self.source_file.file_name.clone(),
            wire_sha256: self.source_file.wire_sha256.clone(),
        };
        let commitment = DailyNormalizedSourceCommitmentV1Wire {
            schema: "owner-beta-daily-normalized-source-v1".to_owned(),
            kind: DailyEvidenceKindV1::Calendar,
            session_date: date,
            source_files: vec![key],
        };
        let source_bytes = daily_normalized_source_preimage_v1(&commitment)
            .map_err(|_| DailyVerificationError::InvalidEvidence)?;
        let source_evidence_sha256 = ContentHash::from_bytes(&source_bytes);
        let document = json!({
            "schema": "owner-beta-daily-reconstructed-document-v1",
            "normalizer_version": NORMALIZER_VERSION,
            "kind": DailyEvidenceKindV1::Calendar,
            "session_date": date,
            "source_evidence_sha256": source_evidence_sha256,
            "rows": [{"date": date, "session_type": day.session_type, "evidence_kind": day.evidence_kind}],
        });
        let bytes = canonical_json(&document)?;
        let descriptor = DailyNormalizedDocumentV1Wire {
            kind: DailyEvidenceKindV1::Calendar,
            session_date: date,
            normalizer_version: NORMALIZER_VERSION.to_owned(),
            sha256: ContentHash::from_bytes(&bytes),
            size_bytes: u64::try_from(bytes.len())
                .map_err(|_| DailyVerificationError::LimitExceeded)?,
            row_count: 1,
            source_files: commitment.source_files,
            source_evidence_sha256,
        };
        Ok(ReconstructedCalendarDocument { bytes, descriptor })
    }
}

pub struct ReconstructedCalendarDocument {
    bytes: Vec<u8>,
    descriptor: DailyNormalizedDocumentV1Wire,
}

impl ReconstructedCalendarDocument {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn descriptor(&self) -> &DailyNormalizedDocumentV1Wire {
        &self.descriptor
    }
}

/// Projects a bounded, complete set of resolved calendar candidates. Historical
/// range collectors sometimes stored an unlinked cached calendar under a later
/// batch date. Such a delivery is never a witness: it may be excluded only when
/// an uncopied, valid KIS original in this same snapshot has identical bytes and
/// request/response metadata and was retrieved no later. Missing, changed or
/// otherwise invalid evidence still fails. No copy chain is synthesized.
pub fn project_calendar_candidates<'snapshot>(
    originals: &[ResolvedOriginal<'snapshot>],
    owner: Uuid,
    policy: &ContentHash,
) -> Result<Vec<VerifiedCalendarProjection<'snapshot>>, DailyVerificationError> {
    let mut projections = Vec::new();
    candidates_with(
        originals,
        owner,
        policy,
        &mut projections,
        project_calendar,
        |_| Ok(()),
        |e| e,
    )?;
    Ok(projections)
}

// Private adapters only. Descriptive bytes never become a live checked original.
trait CalendarOriginal {
    fn evidence(&self) -> &DailyFileEvidenceV1Wire;
    fn original_bytes(&self) -> &[u8];
    fn canonical_evidence_bytes(&self) -> &[u8];
    fn observed_at(&self) -> UtcTimestamp;
}
impl CalendarOriginal for ResolvedOriginal<'_> {
    fn evidence(&self) -> &DailyFileEvidenceV1Wire {
        self.evidence()
    }
    fn original_bytes(&self) -> &[u8] {
        self.original_bytes()
    }
    fn canonical_evidence_bytes(&self) -> &[u8] {
        self.canonical_evidence_bytes()
    }
    fn observed_at(&self) -> UtcTimestamp {
        self.observed_at()
    }
}
fn candidates_with<O: CalendarOriginal, P, E>(
    originals: &[O],
    owner: Uuid,
    policy: &ContentHash,
    projections: &mut Vec<P>,
    mut project: impl FnMut(&O, Uuid, &ContentHash) -> Result<P, DailyVerificationError>,
    mut guard: impl FnMut(usize) -> Result<(), E>,
    policy_error: impl Fn(DailyVerificationError) -> E,
) -> Result<(), E> {
    if originals.len() as u64 > super::contract::fixed_limits().max_file_evidence_per_session {
        return Err(policy_error(DailyVerificationError::LimitExceeded));
    }
    let mut verified: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    let mut rejected = Vec::new();
    let mut bytes = 0u64;
    for (index, original) in originals.iter().enumerate() {
        guard(index)?;
        bytes = bytes
            .checked_add(original.original_bytes().len() as u64)
            .ok_or_else(|| policy_error(DailyVerificationError::LimitExceeded))?;
        if bytes > super::verify::MAX_SOURCE_BYTES {
            return Err(policy_error(DailyVerificationError::LimitExceeded));
        }
        match project(original, owner, policy) {
            Ok(projection) => {
                let source = original.evidence();
                if source.provider == "kis" && source.copied_from.is_empty() {
                    verified
                        .entry(source.wire_sha256.as_str())
                        .or_default()
                        .push(index);
                }
                projections.push(projection);
            }
            Err(error) => rejected.push((index, error)),
        }
    }
    for (index, error) in rejected {
        guard(index)?;
        let cached = &originals[index];
        let file = cached.evidence();
        let redundant = error == DailyVerificationError::InvalidEvidence
            && file.provider == "kis"
            && file.kind == DailyEvidenceKindV1::Calendar
            && file.copied_from.is_empty()
            && verified
                .get(file.wire_sha256.as_str())
                .is_some_and(|matches| {
                    matches.iter().any(|&original_index| {
                        let original = &originals[original_index];
                        let source = original.evidence();
                        source.batch_date < file.batch_date
                            && source.retrieved_at.as_timestamp()
                                <= file.retrieved_at.as_timestamp()
                            && source.market == file.market
                            && source.file_name == file.file_name
                            && source.size_bytes == file.size_bytes
                            && source.request == file.request
                            && source.response_continuation == file.response_continuation
                            && original.original_bytes() == cached.original_bytes()
                    })
                });
        if !redundant {
            return Err(policy_error(error));
        }
    }
    Ok(())
}

pub fn project_calendar<'snapshot>(
    original: &ResolvedOriginal<'snapshot>,
    owner: Uuid,
    policy: &ContentHash,
) -> Result<VerifiedCalendarProjection<'snapshot>, DailyVerificationError> {
    let data = project_calendar_with(original, owner, policy)?;
    Ok(VerifiedCalendarProjection {
        owner: data.owner,
        policy: data.policy,
        source_file: data.source_file,
        canonical_file: data.canonical_file,
        evidence_sha256: data.evidence_sha256,
        days: data.days,
        observed_at: data.observed_at,
        _original_bytes: original.original_bytes(),
    })
}

/// Owned diagnostics; no public constructor and no unchecked admission conversion.
/// ```compile_fail
/// use market_data::daily_price_only::calendar::{CalendarData, VerifiedCalendarProjection};
/// fn cannot_promote(data: CalendarData) -> VerifiedCalendarProjection<'static> { data }
/// ```
pub struct CalendarData {
    owner: Uuid,
    policy: ContentHash,
    source_file: DailyFileEvidenceV1Wire,
    canonical_file: Vec<u8>,
    evidence_sha256: ContentHash,
    days: Vec<VerifiedCalendarDay>,
    observed_at: UtcTimestamp,
}
impl CalendarData {
    pub fn owner(&self) -> Uuid {
        self.owner
    }
    pub fn policy_sha256(&self) -> &ContentHash {
        &self.policy
    }
    pub fn days(&self) -> &[VerifiedCalendarDay] {
        &self.days
    }
    pub fn source_file(&self) -> &DailyFileEvidenceV1Wire {
        &self.source_file
    }
    pub fn canonical_file_bytes(&self) -> &[u8] {
        &self.canonical_file
    }
    pub fn evidence_sha256(&self) -> &ContentHash {
        &self.evidence_sha256
    }
    pub fn observed_at(&self) -> UtcTimestamp {
        self.observed_at
    }
}
impl std::fmt::Debug for CalendarData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CalendarData")
            .field("days", &self.days.len())
            .finish_non_exhaustive()
    }
}
fn project_calendar_with<O: CalendarOriginal>(
    original: &O,
    owner: Uuid,
    policy: &ContentHash,
) -> Result<CalendarData, DailyVerificationError> {
    if owner.is_nil() {
        return Err(DailyVerificationError::InvalidIdentity);
    }
    let source = original.evidence();
    validate_calendar_request(source).map_err(|_| DailyVerificationError::InvalidEvidence)?;
    let target = source
        .copied_from
        .last()
        .map_or(source.batch_date, |hop| hop.batch_date);
    let document = parse_provider_json(original.original_bytes())?;
    let object = document
        .as_object()
        .ok_or(DailyVerificationError::InvalidProviderJson)?;
    if object.get("rt_cd").and_then(|value| value.as_str()) != Some("0") {
        return Err(DailyVerificationError::ProviderResponse);
    }
    let output = object
        .get("output")
        .ok_or(DailyVerificationError::InvalidCalendar)?;
    let rows = match output {
        serde_json::Value::Array(rows) => rows.as_slice(),
        serde_json::Value::Object(_) => std::slice::from_ref(output),
        _ => return Err(DailyVerificationError::InvalidCalendar),
    };
    if rows.is_empty() || rows.len() > MAX_DAYS {
        return Err(DailyVerificationError::LimitExceeded);
    }
    let mut days = BTreeMap::new();
    for (ordinal, row) in rows.iter().enumerate() {
        let fields = row
            .as_object()
            .ok_or(DailyVerificationError::InvalidCalendar)?;
        let (date, open) = validated_kis_calendar_row(fields, &source.file_name)
            .map_err(|_| DailyVerificationError::InvalidCalendar)?;
        if !(1..=9999).contains(&date.as_naive_date().year()) {
            return Err(DailyVerificationError::InvalidCalendar);
        }
        let vacant = match days.entry(date) {
            std::collections::btree_map::Entry::Occupied(_) => {
                return Err(DailyVerificationError::CalendarConflict);
            }
            std::collections::btree_map::Entry::Vacant(entry) => entry,
        };
        let source_row_sha256 = ContentHash::from_bytes(&canonical_json(row)?);
        let day = VerifiedCalendarDay {
            session_date: date,
            session_type: if open {
                DailySessionTypeV1::Trading
            } else {
                DailySessionTypeV1::Closed
            },
            evidence_kind: if date == target {
                CalendarEvidenceKind::RequestedTarget
            } else {
                CalendarEvidenceKind::DerivedSurrounding
            },
            source_row_ordinal: u32::try_from(ordinal)
                .map_err(|_| DailyVerificationError::LimitExceeded)?,
            source_row_sha256,
        };
        vacant.insert(day);
    }
    if !days.contains_key(&target) {
        return Err(DailyVerificationError::CalendarTargetMissing);
    }
    let first = days
        .first_key_value()
        .ok_or(DailyVerificationError::InvalidCalendar)?
        .0;
    let last = days
        .last_key_value()
        .ok_or(DailyVerificationError::InvalidCalendar)?
        .0;
    checked_range_len(*first, *last)?;
    let mut identity =
        format!("owner-beta-daily-calendar-source-v1\n{owner}\n{policy}\n{NORMALIZER_VERSION}\n")
            .into_bytes();
    identity.extend_from_slice(original.canonical_evidence_bytes());
    Ok(CalendarData {
        owner,
        policy: policy.clone(),
        source_file: source.clone(),
        canonical_file: original.canonical_evidence_bytes().to_vec(),
        evidence_sha256: ContentHash::from_bytes(&identity),
        days: days.into_values().collect(),
        observed_at: original.observed_at(),
    })
}

/// Equal facts keep all distinct source identities, in deterministic order.
#[derive(Debug, PartialEq, Eq)]
pub struct ProvenCalendarDay {
    date: TradingDate,
    session_type: DailySessionTypeV1,
    witnesses: Vec<ContentHash>,
    row_witnesses: Vec<(ContentHash, u32, ContentHash)>,
}

impl ProvenCalendarDay {
    pub fn date(&self) -> TradingDate {
        self.date
    }
    pub fn session_type(&self) -> DailySessionTypeV1 {
        self.session_type
    }
    pub fn witnesses(&self) -> &[ContentHash] {
        &self.witnesses
    }
    pub fn selected_evidence(&self) -> &ContentHash {
        &self.witnesses[0]
    }
    /// Match one exact row from a verified projection, without granting
    /// freshness or admission authority to the supplied database fields.
    pub fn has_row_witness(&self, evidence: &str, ordinal: u32, row_sha256: &str) -> bool {
        self.row_witnesses.iter().any(|(source, row, hash)| {
            source.as_str() == evidence && *row == ordinal && hash.as_str() == row_sha256
        })
    }
}

/// A civil-date proof only; not a freshness, source-frontier or admission seal.
pub struct CalendarRangeProof {
    owner: Uuid,
    policy: ContentHash,
    days: Vec<ProvenCalendarDay>,
}

impl CalendarRangeProof {
    pub fn owner(&self) -> Uuid {
        self.owner
    }
    pub fn policy_sha256(&self) -> &ContentHash {
        &self.policy
    }
    pub fn days(&self) -> &[ProvenCalendarDay] {
        &self.days
    }
}

pub fn prove_calendar_range(
    projections: &[&VerifiedCalendarProjection<'_>],
    start: TradingDate,
    end: TradingDate,
) -> Result<CalendarRangeProof, DailyVerificationError> {
    prove_range_claims(projections, start, end)
}

// Private common projection interface: callers cannot turn detached diagnostic
// data into a proof. The checked entry point below requires the live view.
trait RangeClaims {
    fn owner(&self) -> Uuid;
    fn policy(&self) -> &ContentHash;
    fn days(&self) -> &[VerifiedCalendarDay];
    fn evidence(&self) -> &ContentHash;
}

impl RangeClaims for VerifiedCalendarProjection<'_> {
    fn owner(&self) -> Uuid {
        self.owner
    }
    fn policy(&self) -> &ContentHash {
        &self.policy
    }
    fn days(&self) -> &[VerifiedCalendarDay] {
        &self.days
    }
    fn evidence(&self) -> &ContentHash {
        &self.evidence_sha256
    }
}

#[cfg(target_os = "linux")]
impl RangeClaims for CalendarData {
    fn owner(&self) -> Uuid {
        self.owner
    }
    fn policy(&self) -> &ContentHash {
        &self.policy
    }
    fn days(&self) -> &[VerifiedCalendarDay] {
        &self.days
    }
    fn evidence(&self) -> &ContentHash {
        &self.evidence_sha256
    }
}

fn prove_range_claims<P: RangeClaims>(
    projections: &[&P],
    start: TradingDate,
    end: TradingDate,
) -> Result<CalendarRangeProof, DailyVerificationError> {
    let count = checked_range_len(start, end)?;
    if projections.is_empty() || projections.len() > MAX_ROW_CLAIMS {
        return Err(DailyVerificationError::InvalidContext);
    }
    let first = projections[0];
    let mut total = 0usize;
    // Preflight all claims before allocating merged witnesses.
    for projection in projections {
        if projection.owner() != first.owner() || projection.policy() != first.policy() {
            return Err(DailyVerificationError::InvalidIdentity);
        }
        total = total
            .checked_add(projection.days().len())
            .ok_or(DailyVerificationError::LimitExceeded)?;
        if total > MAX_ROW_CLAIMS {
            return Err(DailyVerificationError::LimitExceeded);
        }
    }
    let mut facts: BTreeMap<TradingDate, ProvenCalendarDay> = BTreeMap::new();
    for projection in projections {
        for day in projection.days() {
            let fact = facts
                .entry(day.session_date)
                .or_insert_with(|| ProvenCalendarDay {
                    date: day.session_date,
                    session_type: day.session_type,
                    witnesses: Vec::new(),
                    row_witnesses: Vec::new(),
                });
            if fact.session_type != day.session_type {
                return Err(DailyVerificationError::CalendarConflict);
            }
            fact.witnesses.push(projection.evidence().clone());
            fact.row_witnesses.push((
                projection.evidence().clone(),
                day.source_row_ordinal,
                day.source_row_sha256.clone(),
            ));
        }
    }
    let mut days = Vec::with_capacity(count);
    for offset in 0..count {
        let date = start
            .checked_add_days(
                i64::try_from(offset).map_err(|_| DailyVerificationError::LimitExceeded)?,
            )
            .map_err(|_| DailyVerificationError::InvalidCalendar)?;
        let mut day = facts
            .remove(&date)
            .ok_or(DailyVerificationError::CalendarCoverageGap)?;
        day.witnesses.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        day.witnesses.dedup();
        day.row_witnesses.sort_by(|a, b| {
            (a.0.as_str(), a.1, a.2.as_str()).cmp(&(b.0.as_str(), b.1, b.2.as_str()))
        });
        day.row_witnesses.dedup();
        days.push(day);
    }
    Ok(CalendarRangeProof {
        owner: first.owner(),
        policy: first.policy().clone(),
        days,
    })
}

fn checked_range_len(
    start: TradingDate,
    end: TradingDate,
) -> Result<usize, DailyVerificationError> {
    if !(1..=9999).contains(&start.as_naive_date().year())
        || !(1..=9999).contains(&end.as_naive_date().year())
    {
        return Err(DailyVerificationError::InvalidCalendar);
    }
    let span = end
        .as_naive_date()
        .signed_duration_since(start.as_naive_date())
        .num_days();
    if !(0..370).contains(&span) {
        return Err(DailyVerificationError::LimitExceeded);
    }
    usize::try_from(span + 1).map_err(|_| DailyVerificationError::LimitExceeded)
}

pub fn eligible_calendar_date(now: UtcTimestamp) -> Result<TradingDate, DailyVerificationError> {
    let offset = FixedOffset::east_opt(9 * 3600).ok_or(DailyVerificationError::InvalidContext)?;
    let kst = now.as_datetime().with_timezone(&offset);
    let today = TradingDate::parse(&kst.date_naive().to_string())
        .map_err(|_| DailyVerificationError::InvalidCalendar)?;
    let eligible = if (kst.hour(), kst.minute()) >= (16, 30) {
        today
    } else {
        today
            .checked_add_days(-1)
            .map_err(|_| DailyVerificationError::InvalidCalendar)?
    };
    checked_range_len(eligible, eligible)?;
    Ok(eligible)
}

pub struct ExpectedCalendarSession {
    session: TradingDate,
    eligible: TradingDate,
    proof: CalendarRangeProof,
}

impl ExpectedCalendarSession {
    pub fn session(&self) -> TradingDate {
        self.session
    }
    pub fn eligible_date(&self) -> TradingDate {
        self.eligible
    }
    pub fn proof(&self) -> &CalendarRangeProof {
        &self.proof
    }
}

pub fn expected_calendar_session(
    projections: &[&VerifiedCalendarProjection<'_>],
    continuous_from: TradingDate,
    now: UtcTimestamp,
) -> Result<ExpectedCalendarSession, DailyVerificationError> {
    if projections
        .iter()
        .any(|projection| projection.observed_at > now)
    {
        return Err(DailyVerificationError::FutureRetrieval);
    }
    let eligible = eligible_calendar_date(now)?;
    let proof = prove_calendar_range(projections, continuous_from, eligible)?;
    let session = proof
        .days
        .iter()
        .rev()
        .find(|day| day.session_type == DailySessionTypeV1::Trading)
        .map(|day| day.date)
        .ok_or(DailyVerificationError::NoTradingSession)?;
    Ok(ExpectedCalendarSession {
        session,
        eligible,
        proof,
    })
}

#[cfg(target_os = "linux")]
mod checked_calendar {
    use super::super::verify::{
        CheckedLineageFailure, CheckedLineageLocal, CheckedResolvedOriginal,
        CheckedResolvedOriginalView,
    };
    use super::*;
    impl CalendarOriginal for CheckedResolvedOriginalView<'_, '_> {
        fn evidence(&self) -> &DailyFileEvidenceV1Wire {
            self.evidence()
        }
        fn original_bytes(&self) -> &[u8] {
            self.original_bytes()
        }
        fn canonical_evidence_bytes(&self) -> &[u8] {
            self.canonical_evidence_bytes()
        }
        fn observed_at(&self) -> UtcTimestamp {
            self.observed_at()
        }
    }
    pub enum CheckedCalendarSource {
        Policy(DailyVerificationError),
        Lineage(CheckedLineageFailure),
    }
    /// Lifetime-free whole per-call failure and the actual successful projection prefix.
    pub struct CheckedCalendarFailure {
        source: CheckedCalendarSource,
        data: Vec<CalendarData>,
        locals: Vec<CheckedLineageLocal>,
        later: Vec<CheckedLineageFailure>,
    }
    impl CheckedCalendarFailure {
        pub fn source(&self) -> &CheckedCalendarSource {
            &self.source
        }
        pub fn source_report(&self) -> Option<&crate::checked_close::Report> {
            match &self.source {
                CheckedCalendarSource::Policy(_) => None,
                CheckedCalendarSource::Lineage(e) => e.source_report(),
            }
        }
        pub fn retained(&self) -> &[CalendarData] {
            &self.data
        }
        pub fn locals(&self) -> &[CheckedLineageLocal] {
            &self.locals
        }
        pub fn later(&self) -> &[CheckedLineageFailure] {
            &self.later
        }
    }
    pub struct CheckedCalendarCandidates<'o, 'set, 's, 'i, 'a> {
        originals: &'o [CheckedResolvedOriginal<'set, 's, 'i, 'a>],
        data: Vec<CalendarData>,
        locals: Vec<CheckedLineageLocal>,
    }
    pub struct CheckedCalendarView<'v> {
        data: &'v [CalendarData],
        locals: Vec<CheckedLineageLocal>,
    }
    impl CheckedCalendarView<'_> {
        pub fn projections(&self) -> &[CalendarData] {
            self.data
        }
        /// Prove a bounded range from this actual checked view. The owner must
        /// still collect the view diagnostics and finish its source holders.
        pub fn prove_range(
            &self,
            start: TradingDate,
            end: TradingDate,
        ) -> Result<CalendarRangeProof, DailyVerificationError> {
            if self.data.len() > MAX_ROW_CLAIMS {
                return Err(DailyVerificationError::InvalidContext);
            }
            let claims = self.data.iter().collect::<Vec<_>>();
            prove_range_claims(&claims, start, end)
        }
        pub fn into_diagnostics(self) -> Vec<CheckedLineageLocal> {
            self.locals
        }
    }
    pub struct CheckedCalendarCompletion {
        data: Vec<CalendarData>,
        locals: Vec<CheckedLineageLocal>,
        failure: Option<CheckedCalendarFailure>,
    }
    impl CheckedCalendarCompletion {
        pub fn is_clean(&self) -> bool {
            self.failure.is_none() && self.locals.iter().all(CheckedLineageLocal::is_clean)
        }
        pub fn retained(&self) -> &[CalendarData] {
            &self.data
        }
        pub fn failure(&self) -> Option<&CheckedCalendarFailure> {
            self.failure.as_ref()
        }
    }
    pub fn project_calendar_candidates_checked<'o, 'set, 's, 'i, 'a>(
        originals: &'o [CheckedResolvedOriginal<'set, 's, 'i, 'a>],
        owner: Uuid,
        policy: &ContentHash,
    ) -> Result<CheckedCalendarCandidates<'o, 'set, 's, 'i, 'a>, CheckedCalendarFailure> {
        let mut views = Vec::new();
        for original in originals {
            match original.view_in() {
                Ok(view) => views.push(view),
                Err(error) => {
                    return Err(CheckedCalendarFailure {
                        source: CheckedCalendarSource::Lineage(error),
                        data: Vec::new(),
                        locals: views
                            .into_iter()
                            .map(CheckedResolvedOriginalView::into_local)
                            .collect(),
                        later: Vec::new(),
                    });
                }
            }
        }
        let mut data = Vec::new();
        let mut locals = Vec::new();
        let result = candidates_with(
            &views,
            owner,
            policy,
            &mut data,
            project_calendar_with,
            |index| {
                let view = originals[index]
                    .view_in()
                    .map_err(CheckedCalendarSource::Lineage)?;
                locals.push(view.into_local());
                Ok(())
            },
            CheckedCalendarSource::Policy,
        );
        locals.extend(
            views
                .into_iter()
                .map(CheckedResolvedOriginalView::into_local),
        );
        match result {
            Ok(()) => Ok(CheckedCalendarCandidates {
                originals,
                data,
                locals,
            }),
            Err(source) => Err(CheckedCalendarFailure {
                source,
                data,
                locals,
                later: Vec::new(),
            }),
        }
    }
    impl CheckedCalendarCandidates<'_, '_, '_, '_, '_> {
        pub fn view_in(&self) -> Result<CheckedCalendarView<'_>, CheckedCalendarFailure> {
            let mut locals = Vec::new();
            // Visit every genuine source even when an earlier comparison dirtied a scope.
            let mut first = None;
            let mut later = Vec::new();
            for original in self.originals {
                match original.view_in() {
                    Ok(view) => locals.push(view.into_local()),
                    Err(error) => {
                        if first.is_none() {
                            first = Some(error);
                        } else {
                            later.push(error);
                        }
                    }
                }
            }
            if let Some(error) = first {
                return Err(CheckedCalendarFailure {
                    source: CheckedCalendarSource::Lineage(error),
                    data: Vec::new(),
                    locals,
                    later,
                });
            }
            Ok(CheckedCalendarView {
                data: &self.data,
                locals,
            })
        }
        pub fn finish_local(self) -> CheckedCalendarCompletion {
            let result = self.view_in().map(CheckedCalendarView::into_diagnostics);
            let Self {
                data, mut locals, ..
            } = self;
            let failure = match result {
                Ok(later) => {
                    locals.extend(later);
                    None
                }
                Err(e) => Some(e),
            };
            CheckedCalendarCompletion {
                data,
                locals,
                failure,
            }
        }
    }
    impl std::fmt::Debug for CheckedCalendarFailure {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("CheckedCalendarFailure")
                .field("projections", &self.data.len())
                .field("source_report_present", &self.source_report().is_some())
                .finish_non_exhaustive()
        }
    }
}
#[cfg(target_os = "linux")]
pub use checked_calendar::{
    CheckedCalendarCandidates, CheckedCalendarCompletion, CheckedCalendarFailure,
    CheckedCalendarSource, CheckedCalendarView, project_calendar_candidates_checked,
};

#[cfg(test)]
mod checker_calendar_tests {
    use super::super::{DailyRequestMetadataV1Wire, DailyWireTimestampV1};
    use super::*;
    struct OrdinaryOriginal {
        file: DailyFileEvidenceV1Wire,
        bytes: Vec<u8>,
    }
    impl CalendarOriginal for OrdinaryOriginal {
        fn evidence(&self) -> &DailyFileEvidenceV1Wire {
            &self.file
        }
        fn original_bytes(&self) -> &[u8] {
            &self.bytes
        }
        fn canonical_evidence_bytes(&self) -> &[u8] {
            b"{}"
        }
        fn observed_at(&self) -> UtcTimestamp {
            self.file.retrieved_at.as_timestamp()
        }
    }
    fn ordinary(date: &str) -> OrdinaryOriginal {
        let time = DailyWireTimestampV1::parse("2026-11-02T00:00:00.000000000Z").unwrap();
        let hash = ContentHash::from_bytes(b"same bytes");
        OrdinaryOriginal {
            file: DailyFileEvidenceV1Wire {
                provider: "kis".into(),
                market: "kr".into(),
                batch_id: serde_json::from_value(json!("00000000-0000-0000-0000-000000000001"))
                    .unwrap(),
                batch_date: TradingDate::parse(date).unwrap(),
                batch_json_sha256: hash.clone(),
                manifest_line_sha256: hash.clone(),
                file_name: "calendar.json".into(),
                kind: DailyEvidenceKindV1::Calendar,
                wire_sha256: hash,
                size_bytes: 10,
                request: DailyRequestMetadataV1Wire {
                    endpoint: "/ordinary-fixture".into(),
                    query: Vec::new(),
                    headers: Vec::new(),
                    mode: "fixture".into(),
                },
                response_continuation: None,
                retrieved_at: time,
                retrieval_precision: "fixture".into(),
                original_retrieved_at: time,
                original_retrieval_precision: "fixture".into(),
                copied_from: Vec::new(),
            },
            bytes: b"same bytes".to_vec(),
        }
    }
    #[test]
    fn checked_calendar_keeps_valid_redundant_cache_rule_and_first_error_order() {
        let owner = Uuid::from_u128(1);
        let policy = ContentHash::from_bytes(b"policy");
        let mut originals = vec![ordinary("2026-11-01"), ordinary("2026-11-02")];
        let project = |original: &OrdinaryOriginal, _, _: &ContentHash| {
            if original.file.batch_date == TradingDate::parse("2026-11-01").unwrap() {
                Ok(7)
            } else {
                Err(DailyVerificationError::InvalidEvidence)
            }
        };
        let mut out = Vec::new();
        assert!(
            candidates_with(
                &originals,
                owner,
                &policy,
                &mut out,
                project,
                |_| Ok::<(), DailyVerificationError>(()),
                |e| e
            )
            .is_ok()
        );
        assert_eq!(out, vec![7]);
        originals[1].bytes.push(b'!');
        let mut out = Vec::new();
        assert_eq!(
            candidates_with(
                &originals,
                owner,
                &policy,
                &mut out,
                project,
                |_| Ok::<(), DailyVerificationError>(()),
                |e| e
            ),
            Err(DailyVerificationError::InvalidEvidence)
        );
        assert_eq!(out, vec![7]);
        originals[1].bytes.pop();
        originals.push(ordinary("2026-11-03"));
        let mut out = Vec::new();
        let result = candidates_with(
            &originals,
            owner,
            &policy,
            &mut out,
            |original, owner, policy| {
                if original.file.batch_date == TradingDate::parse("2026-11-03").unwrap() {
                    Err(DailyVerificationError::ProviderResponse)
                } else {
                    project(original, owner, policy)
                }
            },
            |_| Ok::<(), DailyVerificationError>(()),
            |e| e,
        );
        assert_eq!(result, Err(DailyVerificationError::ProviderResponse));
        assert_eq!(out, vec![7]);
        // The metadata comparison is independent of same bytes: a changed request cannot be hidden.
        originals.pop();
        originals[1]
            .file
            .request
            .query
            .push(("changed".into(), "request".into()));
        let mut out = Vec::new();
        assert_eq!(
            candidates_with(
                &originals,
                owner,
                &policy,
                &mut out,
                project,
                |_| Ok::<(), DailyVerificationError>(()),
                |e| e
            ),
            Err(DailyVerificationError::InvalidEvidence)
        );
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn checked_calendar_actual_returned_prefix_survives_later_scope_failure() {
        use crate::{
            checked_close::{Attempt, Failure},
            raw_custody::RawInvocation,
        };
        use std::cell::Cell;
        use std::convert::Infallible;
        enum Error {
            Scope(Failure<Infallible>),
            Policy(DailyVerificationError),
        }
        struct Row(Box<str>);
        let actual = Attempt::default();
        let foreign = Attempt::default();
        let invocation = RawInvocation::new(&actual);
        let originals = [ordinary("2026-11-01"), ordinary("2026-11-02")];
        let calls = Cell::new(0);
        let mut out = Vec::new();
        let result = candidates_with(
            &originals,
            Uuid::from_u128(1),
            &ContentHash::from_bytes(b"policy"),
            &mut out,
            |_, _, _| {
                calls.set(calls.get() + 1);
                Ok(Row("returned prefix".into()))
            },
            |index| {
                if index == 0 {
                    Ok(())
                } else {
                    invocation
                        .settle_local_in(&foreign, Ok::<(), Infallible>(()))
                        .map_err(Error::Scope)
                }
            },
            Error::Policy,
        );
        match result {
            Err(Error::Scope(failure)) => assert!(failure.body().is_none()),
            Err(Error::Policy(policy)) => panic!("unexpected policy {policy:?}"),
            Ok(()) => panic!("foreign scope accepted"),
        }
        assert_eq!(calls.get(), 1);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0.as_ref(), "returned prefix");
        assert!(actual.finish(Ok::<(), Infallible>(())).is_err());
        assert!(foreign.finish(Ok::<(), Infallible>(())).is_err());
    }
}
