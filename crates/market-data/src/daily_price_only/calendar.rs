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

pub fn project_calendar<'snapshot>(
    original: &ResolvedOriginal<'snapshot>,
    owner: Uuid,
    policy: &ContentHash,
) -> Result<VerifiedCalendarProjection<'snapshot>, DailyVerificationError> {
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
    Ok(VerifiedCalendarProjection {
        owner,
        policy: policy.clone(),
        source_file: source.clone(),
        canonical_file: original.canonical_evidence_bytes().to_vec(),
        evidence_sha256: ContentHash::from_bytes(&identity),
        days: days.into_values().collect(),
        observed_at: original.observed_at(),
        _original_bytes: original.original_bytes(),
    })
}

/// Equal facts keep all distinct source identities, in deterministic order.
#[derive(Debug, PartialEq, Eq)]
pub struct ProvenCalendarDay {
    date: TradingDate,
    session_type: DailySessionTypeV1,
    witnesses: Vec<ContentHash>,
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
    let count = checked_range_len(start, end)?;
    if projections.is_empty() || projections.len() > MAX_ROW_CLAIMS {
        return Err(DailyVerificationError::InvalidContext);
    }
    let first = projections[0];
    let mut total = 0usize;
    // Preflight all claims before allocating merged witnesses.
    for projection in projections {
        if projection.owner != first.owner || projection.policy != first.policy {
            return Err(DailyVerificationError::InvalidIdentity);
        }
        total = total
            .checked_add(projection.days.len())
            .ok_or(DailyVerificationError::LimitExceeded)?;
        if total > MAX_ROW_CLAIMS {
            return Err(DailyVerificationError::LimitExceeded);
        }
    }
    let mut facts: BTreeMap<TradingDate, ProvenCalendarDay> = BTreeMap::new();
    for projection in projections {
        for day in &projection.days {
            let fact = facts
                .entry(day.session_date)
                .or_insert_with(|| ProvenCalendarDay {
                    date: day.session_date,
                    session_type: day.session_type,
                    witnesses: Vec::new(),
                });
            if fact.session_type != day.session_type {
                return Err(DailyVerificationError::CalendarConflict);
            }
            fact.witnesses.push(projection.evidence_sha256.clone());
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
        days.push(day);
    }
    Ok(CalendarRangeProof {
        owner: first.owner,
        policy: first.policy.clone(),
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
