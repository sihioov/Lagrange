//! Sealed v3 artifact approval and independent date-window preparation.
//!
//! This module reads the approved artifact once, validates its verified
//! envelope against the simulation contract, and derives the schedule from
//! the embedded XKRX calendar rather than dates inferred from bars.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use domain::{ContentHash, FixedPoint, TradingDate};
use market_data::{
    ApprovedHistoricalPriceOnlyArtifact, HistoricalPriceOnlyBar,
    HistoricalPriceOnlyV3ArtifactApprovalSummary, KR_ETF_CORE_SYMBOLS,
    range_normalize::ExpectedRangeSessions,
};

use super::{
    APPROVED_BARS_SHA256, APPROVED_END_DATE, APPROVED_SESSION_COUNT, APPROVED_START_DATE,
    EvaluationPriceRow, Price4, SimulationContractError, SimulationSchedule,
};

const BAR_PRICE_SCALE: u8 = 4;
const MINIMUM_WARMUP_SESSIONS: usize = 200;
const BENCHMARK_INSTRUMENT: &str = "069500.KRX";

/// Sanitized failures from approved-input preparation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SimulationInputError {
    #[error("approved historical artifact could not be verified")]
    ApprovalRejected,
    #[error("approved historical artifact metadata is inconsistent")]
    MetadataInvalid,
    #[error("approved historical bars are incomplete or invalid")]
    BarsInvalid,
    #[error("simulation evaluation window is outside the approved session range")]
    InvalidWindow,
}

/// In-memory data prepared from one sealed approval result. The artifact is
/// retained so each signal date reuses those same verified bytes.
pub(crate) struct ApprovedSimulationInput {
    pub(crate) artifact: ApprovedHistoricalPriceOnlyArtifact,
    pub(crate) schedule: SimulationSchedule,
    pub(crate) evaluation_prices: Vec<EvaluationPriceRow>,
}

/// Calls the fixed v3 approval function exactly once, then derives a schedule
/// and raw benchmark prices from the returned sealed artifact.
pub(crate) fn approve_and_prepare(
    artifact_root: &Path,
    evaluation_start: Option<TradingDate>,
    evaluation_end: Option<TradingDate>,
) -> Result<ApprovedSimulationInput, SimulationInputError> {
    let artifact = market_data::approve_historical_price_only_artifact(artifact_root)
        .map_err(|_| SimulationInputError::ApprovalRejected)?;
    let range_start = TradingDate::parse(APPROVED_START_DATE)
        .map_err(|_| SimulationInputError::MetadataInvalid)?;
    let range_end =
        TradingDate::parse(APPROVED_END_DATE).map_err(|_| SimulationInputError::MetadataInvalid)?;
    let expected = ExpectedRangeSessions::approved_xkrx(range_start, range_end)
        .map_err(|_| SimulationInputError::MetadataInvalid)?;

    validate_approved_metadata(&artifact, &expected.sessions)?;
    validate_bars_against_sessions(artifact.bars(), &expected.sessions)?;

    let schedule = build_schedule(&expected.sessions, evaluation_start, evaluation_end)?;
    let evaluation_prices = evaluation_prices(artifact.bars(), schedule.evaluation_sessions())?;
    Ok(ApprovedSimulationInput {
        artifact,
        schedule,
        evaluation_prices,
    })
}

fn validate_approved_metadata(
    artifact: &ApprovedHistoricalPriceOnlyArtifact,
    expected_sessions: &[TradingDate],
) -> Result<(), SimulationInputError> {
    let summary = artifact
        .v3_approval_summary()
        .ok_or(SimulationInputError::MetadataInvalid)?;
    validate_v3_metadata(summary, artifact.pins(), expected_sessions)
}

fn validate_v3_metadata(
    summary: &HistoricalPriceOnlyV3ArtifactApprovalSummary,
    pins: &market_data::HistoricalPriceOnlyArtifactPins,
    expected_sessions: &[TradingDate],
) -> Result<(), SimulationInputError> {
    let mut expected_instruments = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| format!("{symbol}.KRX"))
        .collect::<Vec<_>>();
    expected_instruments.sort();
    let expected_start = TradingDate::parse(APPROVED_START_DATE)
        .map_err(|_| SimulationInputError::MetadataInvalid)?;
    let expected_end =
        TradingDate::parse(APPROVED_END_DATE).map_err(|_| SimulationInputError::MetadataInvalid)?;
    let expected_bars_hash = ContentHash::parse(APPROVED_BARS_SHA256)
        .map_err(|_| SimulationInputError::MetadataInvalid)?;

    if summary.candidate_content_sha256() != pins.candidate_content_sha256()
        || summary.artifact_manifest_sha256() != pins.artifact_manifest_sha256()
        || summary.price_manifest_line_sha256() != pins.stage5_manifest_sha256()
        || summary.action_manifest_line_sha256() != pins.action_manifest_sha256()
        || !approved_price_only_metadata(
            summary.audience(),
            summary.capability(),
            summary.vendor_snapshot(),
            summary.strict_pit(),
            summary.action_count(),
        )
        || summary.range_start() != expected_start
        || summary.range_end() != expected_end
        || summary.instruments() != expected_instruments.as_slice()
        || summary.instrument_count() != expected_instruments.len()
        || summary.session_count() != APPROVED_SESSION_COUNT as usize
        || summary.row_count() != (APPROVED_SESSION_COUNT as usize * expected_instruments.len())
        || summary.bars_row_count() != summary.row_count()
        || summary.bars_sha256() != &expected_bars_hash
        || expected_sessions.len() != APPROVED_SESSION_COUNT as usize
        || expected_sessions.first() != Some(&expected_start)
        || expected_sessions.last() != Some(&expected_end)
    {
        return Err(SimulationInputError::MetadataInvalid);
    }

    Ok(())
}

fn approved_price_only_metadata(
    audience: &str,
    capability: &str,
    vendor_snapshot: bool,
    strict_pit: bool,
    non_cash_action_count: usize,
) -> bool {
    audience == "OWNER_ONLY"
        && capability == "PRICE_RETURN_ONLY"
        && vendor_snapshot
        && !strict_pit
        && non_cash_action_count == 0
}

fn validate_bars_against_sessions(
    bars: &[HistoricalPriceOnlyBar],
    expected_sessions: &[TradingDate],
) -> Result<(), SimulationInputError> {
    let expected_instruments = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| format!("{symbol}.KRX"))
        .collect::<BTreeSet<_>>();
    let expected_dates = expected_sessions.iter().copied().collect::<BTreeSet<_>>();
    let expected_bar_count = expected_sessions
        .len()
        .checked_mul(expected_instruments.len())
        .ok_or(SimulationInputError::BarsInvalid)?;
    if expected_sessions.is_empty()
        || expected_dates.len() != expected_sessions.len()
        || bars.len() != expected_bar_count
    {
        return Err(SimulationInputError::BarsInvalid);
    }

    let mut observed = BTreeSet::<(TradingDate, String)>::new();
    for bar in bars {
        let instrument = bar.instrument_id.as_str();
        if !expected_dates.contains(&bar.session_date)
            || !expected_instruments.contains(&instrument)
            || !prices_are_valid(bar)
            || !observed.insert((bar.session_date, instrument))
        {
            return Err(SimulationInputError::BarsInvalid);
        }
    }

    for session in expected_sessions {
        for instrument in &expected_instruments {
            if !observed.contains(&(*session, instrument.clone())) {
                return Err(SimulationInputError::BarsInvalid);
            }
        }
    }
    Ok(())
}

fn prices_are_valid(bar: &HistoricalPriceOnlyBar) -> bool {
    let raw = [&bar.raw_open, &bar.raw_high, &bar.raw_low, &bar.raw_close];
    let adjusted = [
        &bar.adjusted_open,
        &bar.adjusted_high,
        &bar.adjusted_low,
        &bar.adjusted_close,
    ];
    raw.iter().zip(adjusted).all(|(raw, adjusted)| {
        raw.bits() > 0
            && raw.scale() <= BAR_PRICE_SCALE
            && adjusted.bits() > 0
            && adjusted.scale() <= BAR_PRICE_SCALE
            && **raw == *adjusted
    })
}

fn build_schedule(
    sessions: &[TradingDate],
    evaluation_start: Option<TradingDate>,
    evaluation_end: Option<TradingDate>,
) -> Result<SimulationSchedule, SimulationInputError> {
    if sessions.len() < MINIMUM_WARMUP_SESSIONS + 2
        || sessions.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(SimulationInputError::InvalidWindow);
    }

    let start_index = match evaluation_start {
        Some(date) => sessions.iter().position(|session| *session == date),
        None => Some(MINIMUM_WARMUP_SESSIONS),
    }
    .ok_or(SimulationInputError::InvalidWindow)?;
    let end_index = match evaluation_end {
        Some(date) => sessions.iter().position(|session| *session == date),
        None => Some(sessions.len() - 1),
    }
    .ok_or(SimulationInputError::InvalidWindow)?;
    if start_index < MINIMUM_WARMUP_SESSIONS || end_index <= start_index {
        return Err(SimulationInputError::InvalidWindow);
    }

    let warmup_sessions = sessions[..start_index]
        .iter()
        .map(TradingDate::to_iso)
        .collect();
    let evaluation_sessions = sessions[start_index..=end_index]
        .iter()
        .map(TradingDate::to_iso)
        .collect();
    SimulationSchedule::new(warmup_sessions, evaluation_sessions)
        .map_err(|_| SimulationInputError::InvalidWindow)
}

fn evaluation_prices(
    bars: &[HistoricalPriceOnlyBar],
    evaluation_sessions: &[TradingDate],
) -> Result<Vec<EvaluationPriceRow>, SimulationInputError> {
    let benchmark = bars
        .iter()
        .filter(|bar| bar.instrument_id.as_str() == BENCHMARK_INSTRUMENT)
        .map(|bar| (bar.session_date, bar))
        .collect::<BTreeMap<_, _>>();
    evaluation_sessions
        .iter()
        .map(|session| {
            let bar = benchmark
                .get(session)
                .ok_or(SimulationInputError::BarsInvalid)?;
            EvaluationPriceRow::new(
                &session.to_iso(),
                price4(&bar.raw_open)?,
                price4(&bar.raw_close)?,
                bar.raw_volume,
            )
            .map_err(|_| SimulationInputError::BarsInvalid)
        })
        .collect()
}

fn price4(value: &FixedPoint) -> Result<Price4, SimulationInputError> {
    if value.scale() > BAR_PRICE_SCALE || value.bits() <= 0 {
        return Err(SimulationInputError::BarsInvalid);
    }
    let scaled = value
        .with_scale(BAR_PRICE_SCALE)
        .map_err(|_| SimulationInputError::BarsInvalid)?;
    Price4::parse(&scaled.to_string()).map_err(|_| SimulationInputError::BarsInvalid)
}

#[cfg(test)]
mod tests {
    use chrono::{Datelike, Duration, NaiveDate, Weekday};
    use domain::{FixedPoint, InstrumentId, TradingDate};
    use market_data::{HistoricalPriceOnlyBar, KR_ETF_CORE_SYMBOLS};

    use super::{SimulationInputError, build_schedule, validate_bars_against_sessions};

    fn synthetic_weekdays(count: usize) -> Vec<TradingDate> {
        let mut date = NaiveDate::from_ymd_opt(2020, 1, 1).expect("valid test date");
        let mut sessions = Vec::with_capacity(count);
        while sessions.len() < count {
            if !matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
                sessions.push(
                    TradingDate::new(date.year(), date.month(), date.day()).expect("valid weekday"),
                );
            }
            date += Duration::days(1);
        }
        sessions
    }

    fn synthetic_bars(dates: &[TradingDate]) -> Vec<HistoricalPriceOnlyBar> {
        dates
            .iter()
            .flat_map(|date| {
                KR_ETF_CORE_SYMBOLS.iter().map(move |symbol| {
                    let price = FixedPoint::parse("100.0000").expect("price");
                    HistoricalPriceOnlyBar {
                        instrument_id: InstrumentId::parse(&format!("{symbol}.KRX"))
                            .expect("instrument"),
                        session_date: *date,
                        raw_open: price.clone(),
                        raw_high: price.clone(),
                        raw_low: price.clone(),
                        raw_close: price.clone(),
                        raw_volume: 1_000,
                        raw_trading_value: None,
                        adjusted_open: price.clone(),
                        adjusted_high: price.clone(),
                        adjusted_low: price.clone(),
                        adjusted_close: price,
                    }
                })
            })
            .collect()
    }

    #[test]
    fn schedule_requires_two_hundred_warmup_and_two_evaluation_sessions() {
        let sessions = synthetic_weekdays(203);
        assert!(matches!(
            build_schedule(&sessions, Some(sessions[199]), Some(sessions[201])),
            Err(SimulationInputError::InvalidWindow)
        ));

        let schedule = build_schedule(&sessions, Some(sessions[200]), Some(sessions[202]))
            .expect("200 warmup and 3 evaluation sessions");
        assert_eq!(schedule.warmup_sessions().len(), 200);
        assert_eq!(schedule.evaluation_sessions().len(), 3);
        let pairs = schedule.signal_execution_pairs();
        assert_eq!(pairs[0], (sessions[199], sessions[200]));
        assert_eq!(pairs[2], (sessions[201], sessions[202]));
    }

    #[test]
    fn default_window_starts_after_warmup_and_out_of_range_dates_fail_closed() {
        let sessions = synthetic_weekdays(207);
        let schedule = build_schedule(&sessions, None, None).expect("default window");
        assert_eq!(schedule.warmup_sessions().len(), 200);
        assert_eq!(schedule.evaluation_sessions()[0], sessions[200]);
        assert_eq!(schedule.evaluation_sessions().last(), sessions.last());

        let valid_two_sessions =
            build_schedule(&sessions, Some(sessions[201]), Some(sessions[202]))
                .expect("valid two-session window");
        assert_eq!(valid_two_sessions.evaluation_sessions().len(), 2);

        let outside = sessions
            .last()
            .expect("last session")
            .as_naive_date()
            .succ_opt()
            .expect("date after fixture range");
        let outside =
            TradingDate::new(outside.year(), outside.month(), outside.day()).expect("date");
        assert!(!sessions.contains(&outside));
        assert!(matches!(
            build_schedule(&sessions, Some(outside), Some(sessions[202])),
            Err(SimulationInputError::InvalidWindow)
        ));
        assert!(matches!(
            build_schedule(&sessions, Some(sessions[202]), Some(sessions[201])),
            Err(SimulationInputError::InvalidWindow)
        ));
    }

    #[test]
    fn full_session_cross_product_rejects_missing_duplicate_and_extra_bars() {
        let sessions = [
            TradingDate::parse("2020-01-02").expect("date"),
            TradingDate::parse("2020-01-03").expect("date"),
        ];
        let bars = synthetic_bars(&sessions);
        assert!(validate_bars_against_sessions(&bars, &sessions).is_ok());

        let mut missing = bars.clone();
        missing.pop();
        assert_eq!(
            validate_bars_against_sessions(&missing, &sessions),
            Err(SimulationInputError::BarsInvalid)
        );

        let mut duplicate = bars.clone();
        duplicate.push(duplicate[0].clone());
        assert_eq!(
            validate_bars_against_sessions(&duplicate, &sessions),
            Err(SimulationInputError::BarsInvalid)
        );

        let mut extra = bars;
        extra[0].session_date = TradingDate::parse("2020-01-06").expect("date");
        assert_eq!(
            validate_bars_against_sessions(&extra, &sessions),
            Err(SimulationInputError::BarsInvalid)
        );
    }

    #[test]
    fn raw_adjusted_mismatch_and_nonpositive_prices_fail_closed() {
        let sessions = [TradingDate::parse("2020-01-02").expect("date")];
        let mut mismatch = synthetic_bars(&sessions);
        mismatch[0].adjusted_close = FixedPoint::parse("101.0000").expect("price");
        assert_eq!(
            validate_bars_against_sessions(&mismatch, &sessions),
            Err(SimulationInputError::BarsInvalid)
        );

        let mut zero = synthetic_bars(&sessions);
        zero[0].raw_open = FixedPoint::parse("0.0000").expect("price");
        zero[0].adjusted_open = FixedPoint::parse("0.0000").expect("price");
        assert_eq!(
            validate_bars_against_sessions(&zero, &sessions),
            Err(SimulationInputError::BarsInvalid)
        );
    }

    #[test]
    fn metadata_rejects_non_cash_actions_and_changed_rights_labels() {
        assert!(super::approved_price_only_metadata(
            "OWNER_ONLY",
            "PRICE_RETURN_ONLY",
            true,
            false,
            0,
        ));
        assert!(!super::approved_price_only_metadata(
            "OWNER_ONLY",
            "PRICE_RETURN_ONLY",
            true,
            false,
            1,
        ));
        assert!(!super::approved_price_only_metadata(
            "PUBLIC",
            "PRICE_RETURN_ONLY",
            true,
            false,
            0,
        ));
    }
}
