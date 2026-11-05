//! Bounded, read-only financial inputs captured by the Paper evidence checker.
//!
//! These values are descriptive data. They carry no permission, process or
//! runtime proof. A consumer must independently hold protected current/runtime
//! evidence, rerun eligibility immediately before economics, and use SQL fences.

use std::collections::{BTreeMap, BTreeSet};

use crate::owner_beta::{
    daily_input::OwnerBetaDailyRecommendationInputV1, daily_target::OwnerBetaDailyTargetSnapshotV1,
};
use chrono::{DateTime, NaiveDate, Utc};
use domain::{TradingDate, UtcTimestamp};
use market_data::daily_price_only::calendar::CalendarRangeProof;
use serde_json::Value;
use sqlx::PgPool;

pub(super) use super::super::calculation::{PaperPriceSnapshotV1, PriceAvailabilityObservation};
use super::super::{
    contract::{PaperEligibilityDecision, PaperEligibilityReport, UntrustedPaperContractV1},
    input::UntrustedPaperSessionReadbackV1,
};
use super::{EvaluationClock, Reason};

/// Requested additional price dates and an optional witnessed calendar range.
/// Repeated dates are deduplicated when the request is constructed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaperCalculationInputRequest {
    price_dates: BTreeSet<NaiveDate>,
    calendar_range: Option<(TradingDate, TradingDate)>,
}

impl PaperCalculationInputRequest {
    pub fn new(
        price_dates: impl IntoIterator<Item = NaiveDate>,
        calendar_range: Option<(TradingDate, TradingDate)>,
    ) -> Self {
        Self {
            price_dates: price_dates.into_iter().collect(),
            calendar_range,
        }
    }

    pub fn price_dates(&self) -> impl ExactSizeIterator<Item = NaiveDate> + '_ {
        self.price_dates.iter().copied()
    }

    pub fn calendar_range(&self) -> Option<(TradingDate, TradingDate)> {
        self.calendar_range
    }

    fn validate(&self, now: DateTime<Utc>) -> Result<(), PaperCalculationInputError> {
        if self.price_dates.len() > 2 {
            return Err(PaperCalculationInputError::InvalidRequest);
        }
        let eligible = market_data::daily_price_only::calendar::eligible_calendar_date(
            UtcTimestamp::from_datetime(now),
        )
        .map_err(|_| PaperCalculationInputError::InvalidRequest)?
        .as_naive_date();
        if self.price_dates.iter().any(|date| *date > eligible) {
            return Err(PaperCalculationInputError::FuturePriceDate);
        }
        if let Some((start, end)) = self.calendar_range {
            let span = end
                .as_naive_date()
                .signed_duration_since(start.as_naive_date())
                .num_days();
            if start >= end || span < 1 || span as u64 > super::MAX_CALENDAR_DAYS {
                return Err(PaperCalculationInputError::InvalidRequest);
            }
        }
        Ok(())
    }
}

/// Payload-free result errors. Eligibility failures retain the checker's
/// stable reason code; no SQL/provider details cross this boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PaperCalculationInputError {
    #[error("PAPER_CALCULATION_INPUT_INVALID_REQUEST")]
    InvalidRequest,
    #[error("PAPER_CALCULATION_INPUT_FUTURE_PRICE_DATE")]
    FuturePriceDate,
    #[error("PAPER_CALCULATION_INPUT_ELIGIBILITY_REJECTED: {0:?}")]
    Eligibility(super::super::contract::PaperEligibilityReasonCode),
    #[error("PAPER_CALCULATION_INPUT_INELIGIBLE")]
    Ineligible,
    #[error("PAPER_CALCULATION_INPUT_CAPTURE_INCOMPLETE")]
    CaptureIncomplete,
}

/// Actual checked values retained after the complete checker has published an
/// eligible result and every owned resource has finished cleanly.
pub struct ObservedPaperCalculationInputs {
    report: PaperEligibilityReport,
    current_binding: Value,
    sessions: BTreeMap<NaiveDate, PaperSessionObservation>,
    calendar_range_proof: Option<CalendarRangeProof>,
}

impl std::fmt::Debug for ObservedPaperCalculationInputs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObservedPaperCalculationInputs")
            .field("report", &self.report)
            .field("session_count", &self.sessions.len())
            .field(
                "calendar_range_proof_present",
                &self.calendar_range_proof.is_some(),
            )
            .finish_non_exhaustive()
    }
}

impl ObservedPaperCalculationInputs {
    pub fn report(&self) -> &PaperEligibilityReport {
        &self.report
    }

    /// Clone of the selector from the actual protected CURRENT snapshot that
    /// was checked against the caller's untrusted selector.
    pub fn current_binding(&self) -> &Value {
        &self.current_binding
    }

    pub fn sessions(&self) -> &BTreeMap<NaiveDate, PaperSessionObservation> {
        &self.sessions
    }

    pub fn calendar_range_proof(&self) -> Option<&CalendarRangeProof> {
        self.calendar_range_proof.as_ref()
    }
}

/// Checked session data extracted at the genuine checked sealed-view point.
pub struct PaperSessionObservation {
    snapshot: PaperPriceSnapshotV1,
    readback: UntrustedPaperSessionReadbackV1,
    observed_at: DateTime<Utc>,
    availability: PriceAvailabilityObservation,
    input: OwnerBetaDailyRecommendationInputV1,
    target: OwnerBetaDailyTargetSnapshotV1,
}

impl std::fmt::Debug for PaperSessionObservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaperSessionObservation")
            .field("as_of", &self.snapshot.as_of())
            .field("observed_at", &self.observed_at)
            .finish_non_exhaustive()
    }
}

impl PaperSessionObservation {
    pub fn snapshot(&self) -> &PaperPriceSnapshotV1 {
        &self.snapshot
    }

    pub fn readback(&self) -> &UntrustedPaperSessionReadbackV1 {
        &self.readback
    }

    /// Safe clone of the complete validated untrusted SQL value.
    pub fn readback_value(&self) -> Value {
        self.readback.as_untrusted_value().clone()
    }

    pub fn observed_at(&self) -> DateTime<Utc> {
        self.observed_at
    }

    /// Original Raw retrieval and SQL admission times, with actual local observation.
    pub fn availability(&self) -> PriceAvailabilityObservation {
        self.availability
    }

    pub fn input(&self) -> &OwnerBetaDailyRecommendationInputV1 {
        &self.input
    }

    /// Recomputed from the genuine checked input and matched to persisted F/W.
    pub fn target(&self) -> &OwnerBetaDailyTargetSnapshotV1 {
        &self.target
    }
}

/// Opens the existing checker path with bounded data capture. Invalid requests
/// are rejected before opening protected files or starting a database session.
pub async fn read_paper_calculation_inputs(
    pool: &PgPool,
    selector: &UntrustedPaperContractV1,
    request: &PaperCalculationInputRequest,
) -> Result<ObservedPaperCalculationInputs, PaperCalculationInputError> {
    request.validate(Utc::now())?;
    let mut clock = EvaluationClock::start().map_err(PaperCalculationInputError::Eligibility)?;
    super::read_paper_calculation_inputs_at(pool, selector, request, &mut clock).await
}

pub(super) struct CapturedSession {
    pub(super) date: NaiveDate,
    pub(super) snapshot: PaperPriceSnapshotV1,
    pub(super) readback: UntrustedPaperSessionReadbackV1,
    pub(super) observed_at: DateTime<Utc>,
    pub(super) availability: PriceAvailabilityObservation,
    pub(super) input: OwnerBetaDailyRecommendationInputV1,
    pub(super) target: OwnerBetaDailyTargetSnapshotV1,
}

pub(super) fn publish_observed(
    report: PaperEligibilityReport,
    current_binding: Value,
    captures: Vec<CapturedSession>,
    calendar_range_proof: Option<CalendarRangeProof>,
    requested_dates: &BTreeSet<NaiveDate>,
    expects_calendar_proof: bool,
) -> Result<ObservedPaperCalculationInputs, PaperCalculationInputError> {
    validate_report_gate(&report)?;
    if expects_calendar_proof != calendar_range_proof.is_some() {
        return Err(PaperCalculationInputError::CaptureIncomplete);
    }
    let mut sessions = BTreeMap::new();
    for capture in captures {
        if sessions
            .insert(
                capture.date,
                PaperSessionObservation {
                    snapshot: capture.snapshot,
                    readback: capture.readback,
                    observed_at: capture.observed_at,
                    availability: capture.availability,
                    input: capture.input,
                    target: capture.target,
                },
            )
            .is_some()
        {
            return Err(PaperCalculationInputError::CaptureIncomplete);
        }
    }
    let mut required_dates = requested_dates.clone();
    for date in report.sessions() {
        let date = NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .map_err(|_| PaperCalculationInputError::CaptureIncomplete)?;
        required_dates.insert(date);
    }
    if !capture_coverage_complete(&required_dates, sessions.keys().copied()) {
        return Err(PaperCalculationInputError::CaptureIncomplete);
    }
    Ok(ObservedPaperCalculationInputs {
        report,
        current_binding,
        sessions,
        calendar_range_proof,
    })
}

fn validate_report_gate(report: &PaperEligibilityReport) -> Result<(), PaperCalculationInputError> {
    if report.decision() != PaperEligibilityDecision::Eligible {
        return Err(PaperCalculationInputError::Ineligible);
    }
    if report.expires_at() <= Utc::now() {
        return Err(PaperCalculationInputError::Eligibility(
            Reason::EligibilityExpired,
        ));
    }
    Ok(())
}

fn capture_coverage_complete(
    required: &BTreeSet<NaiveDate>,
    captured: impl IntoIterator<Item = NaiveDate>,
) -> bool {
    let captured = captured.into_iter().collect::<BTreeSet<_>>();
    required == &captured && (3..=5).contains(&captured.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::TradingDate;

    fn td(value: &str) -> TradingDate {
        TradingDate::parse(value).expect("test date")
    }

    fn prior_dates() -> [NaiveDate; 3] {
        let eligible = market_data::daily_price_only::calendar::eligible_calendar_date(
            UtcTimestamp::from_datetime(Utc::now()),
        )
        .expect("current eligible date")
        .as_naive_date();
        [
            eligible - chrono::Days::new(3),
            eligible - chrono::Days::new(2),
            eligible - chrono::Days::new(1),
        ]
    }

    #[test]
    fn requests_deduplicate_before_the_two_date_bound() {
        let [one, two, three] = prior_dates();
        let request = PaperCalculationInputRequest::new([one, one, two], None);
        assert_eq!(request.price_dates.len(), 2);
        assert!(request.validate(Utc::now()).is_ok());

        let too_many = PaperCalculationInputRequest::new([one, two, three], None);
        assert_eq!(
            too_many.validate(Utc::now()),
            Err(PaperCalculationInputError::InvalidRequest)
        );
    }

    #[test]
    fn request_rejects_future_prices_and_invalid_calendar_ranges() {
        let future = Utc::now().date_naive() + chrono::Days::new(5);
        let request = PaperCalculationInputRequest::new([future], None);
        assert_eq!(
            request.validate(Utc::now()),
            Err(PaperCalculationInputError::FuturePriceDate)
        );
        let start = td("2026-11-04");
        let reversed = PaperCalculationInputRequest::new([], Some((start, start)));
        assert_eq!(
            reversed.validate(Utc::now()),
            Err(PaperCalculationInputError::InvalidRequest)
        );
    }

    #[test]
    fn capture_coverage_does_not_confuse_extra_dates_with_latest_three() {
        let [first, second, third] = prior_dates();
        let latest_three = [first, second, third].into_iter().collect::<BTreeSet<_>>();
        let extra = [first - chrono::Days::new(2)]
            .into_iter()
            .collect::<BTreeSet<_>>();
        let captures = latest_three.union(&extra).copied().collect::<BTreeSet<_>>();
        let required = latest_three.union(&extra).copied().collect::<BTreeSet<_>>();
        assert!(capture_coverage_complete(
            &required,
            captures.iter().copied()
        ));
        assert!(!capture_coverage_complete(
            &required,
            latest_three.iter().copied()
        ));
        assert_eq!(
            latest_three.len(),
            3,
            "extras never replace required sessions"
        );
    }

    #[test]
    fn ineligible_and_expired_reports_cannot_publish_capture() {
        let now = Utc::now();
        let ineligible = PaperEligibilityReport::new(
            PaperEligibilityDecision::Ineligible,
            vec![Reason::CalendarUnproven],
            vec![],
            now,
            now + chrono::Duration::seconds(20),
        );
        assert_eq!(
            validate_report_gate(&ineligible),
            Err(PaperCalculationInputError::Ineligible)
        );

        let expired = PaperEligibilityReport::new(
            PaperEligibilityDecision::Eligible,
            vec![],
            vec![
                "2026-11-03".to_owned(),
                "2026-11-04".to_owned(),
                "2026-11-05".to_owned(),
            ],
            now - chrono::Duration::seconds(10),
            now - chrono::Duration::seconds(1),
        );
        assert_eq!(
            validate_report_gate(&expired),
            Err(PaperCalculationInputError::Eligibility(
                Reason::EligibilityExpired
            ))
        );
    }
}
