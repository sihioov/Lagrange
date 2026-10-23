//! Pure, checked trading ledgers for the approved price-only simulation.
//!
//! This module accepts only a validated schedule, validated signal snapshots,
//! and one checked open/close row per evaluation session. It performs no I/O,
//! signal generation, calendar construction, or market-data access.

use domain::TradingDate;

use super::{
    contract::{
        BenchmarkLedger, CostProfile, LedgerMetrics, LedgerPhase, LedgerPhaseRecord, Money4,
        PRODUCTION_INITIAL_CAPITAL, PositionMark, Price4, SignalSnapshot, SimulationContractError,
        SimulationSchedule, StrategyLedger, TradeEvent, TradeSide, UnfilledReason,
    },
    metrics::{derive_benchmark_return_difference, derive_performance_metrics},
};

const TREND_INSTRUMENT: &str = "069500.KRX";
const FULL_WEIGHT_PPM: u32 = 1_000_000;
const ILLUSTRATIVE_FEE_DENOMINATOR: i128 = 1_000;

/// One checked raw-price row for an evaluation session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluationPriceRow {
    session_date: TradingDate,
    raw_open: Price4,
    raw_close: Price4,
    raw_volume: u64,
}

impl EvaluationPriceRow {
    /// Constructs a row from already checked, positive fixed-scale prices.
    pub fn new(
        session_date: &str,
        raw_open: Price4,
        raw_close: Price4,
        raw_volume: u64,
    ) -> Result<Self, SimulationContractError> {
        Ok(Self {
            session_date: TradingDate::parse(session_date)
                .map_err(|_| SimulationContractError::InvalidValue)?,
            raw_open,
            raw_close,
            raw_volume,
        })
    }

    pub fn session_date(&self) -> TradingDate {
        self.session_date
    }

    pub fn raw_open(&self) -> &Price4 {
        &self.raw_open
    }

    pub fn raw_close(&self) -> &Price4 {
        &self.raw_close
    }

    pub fn raw_volume(&self) -> u64 {
        self.raw_volume
    }
}

/// Strategy and benchmark ledgers generated under one cost profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CostProfileLedgers {
    profile: CostProfile,
    strategy: StrategyLedger,
    benchmark: BenchmarkLedger,
}

impl CostProfileLedgers {
    pub fn profile(&self) -> CostProfile {
        self.profile
    }

    pub fn strategy(&self) -> &StrategyLedger {
        &self.strategy
    }

    pub fn benchmark(&self) -> &BenchmarkLedger {
        &self.benchmark
    }
}

/// Complete comparison under both approved cost profiles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimulationLedgers {
    illustrative_10bp_per_side: CostProfileLedgers,
    zero_cost_reference: CostProfileLedgers,
}

impl SimulationLedgers {
    pub fn illustrative_10bp_per_side(&self) -> &CostProfileLedgers {
        &self.illustrative_10bp_per_side
    }

    pub fn zero_cost_reference(&self) -> &CostProfileLedgers {
        &self.zero_cost_reference
    }
}

/// Produces strategy and independent buy-and-hold benchmark ledgers under
/// both fixed cost profiles. No partial comparison is returned on failure.
pub fn simulate_ledgers(
    schedule: &SimulationSchedule,
    signals: &[SignalSnapshot],
    evaluation_prices: &[EvaluationPriceRow],
) -> Result<SimulationLedgers, SimulationContractError> {
    let initial_cash = Money4::parse(PRODUCTION_INITIAL_CAPITAL)?;
    build_ledgers(schedule, signals, evaluation_prices, initial_cash)
}

fn build_ledgers(
    schedule: &SimulationSchedule,
    signals: &[SignalSnapshot],
    evaluation_prices: &[EvaluationPriceRow],
    initial_cash: Money4,
) -> Result<SimulationLedgers, SimulationContractError> {
    validate_inputs(schedule, signals, evaluation_prices)?;
    if initial_cash.units()? <= 0 {
        return Err(SimulationContractError::InvalidLedger);
    }

    let illustrative_10bp_per_side = build_cost_profile_ledgers(
        schedule,
        signals,
        evaluation_prices,
        &initial_cash,
        CostProfile::Illustrative10BpPerSideV1,
    )?;
    let zero_cost_reference = build_cost_profile_ledgers(
        schedule,
        signals,
        evaluation_prices,
        &initial_cash,
        CostProfile::ZeroCostReferenceV1,
    )?;

    Ok(SimulationLedgers {
        illustrative_10bp_per_side,
        zero_cost_reference,
    })
}

fn validate_inputs(
    schedule: &SimulationSchedule,
    signals: &[SignalSnapshot],
    evaluation_prices: &[EvaluationPriceRow],
) -> Result<(), SimulationContractError> {
    let sessions = schedule.evaluation_sessions();
    let pairs = schedule.signal_execution_pairs();
    if evaluation_prices.len() != sessions.len() || signals.len() != pairs.len() {
        return Err(SimulationContractError::InvalidSchedule);
    }

    for ((session, pair), (price, signal)) in sessions
        .iter()
        .zip(pairs.iter())
        .zip(evaluation_prices.iter().zip(signals))
    {
        if pair.1 != *session || price.session_date != *session || signal.signal_date() != pair.0 {
            return Err(SimulationContractError::InvalidSchedule);
        }
        let trend_weight = signal
            .targets()
            .find(|(instrument, _)| *instrument == TREND_INSTRUMENT)
            .map(|(_, weight)| weight)
            .ok_or(SimulationContractError::InvalidValue)?;
        if !matches!(trend_weight, 0 | FULL_WEIGHT_PPM)
            || signal.cash_weight_ppm() != FULL_WEIGHT_PPM - trend_weight
        {
            return Err(SimulationContractError::InvalidValue);
        }
        // The validated schedule is strictly increasing, so exact ordered
        // equality above also rejects duplicate, missing, and extra sessions.
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct CashPosition {
    cash_units: i128,
    quantity: u64,
}

fn build_cost_profile_ledgers(
    schedule: &SimulationSchedule,
    signals: &[SignalSnapshot],
    evaluation_prices: &[EvaluationPriceRow],
    initial_cash: &Money4,
    profile: CostProfile,
) -> Result<CostProfileLedgers, SimulationContractError> {
    let strategy_records =
        build_strategy_records(schedule, signals, evaluation_prices, initial_cash, profile)?;
    let benchmark_records = build_benchmark_records(evaluation_prices, initial_cash, profile)?;
    let strategy_metrics = derive_ledger_metrics(initial_cash, &strategy_records)?;
    let benchmark_metrics = derive_ledger_metrics(initial_cash, &benchmark_records)?;
    let strategy_ending_nav = ending_nav(&strategy_records)?;
    let benchmark_ending_nav = ending_nav(&benchmark_records)?;
    let return_difference = derive_benchmark_return_difference(
        initial_cash,
        strategy_ending_nav,
        benchmark_ending_nav,
    )?;

    Ok(CostProfileLedgers {
        profile,
        strategy: StrategyLedger::new(
            initial_cash.clone(),
            strategy_records,
            strategy_metrics,
            return_difference,
        )?,
        benchmark: BenchmarkLedger::new(
            initial_cash.clone(),
            benchmark_records,
            benchmark_metrics,
        )?,
    })
}

fn build_strategy_records(
    schedule: &SimulationSchedule,
    signals: &[SignalSnapshot],
    evaluation_prices: &[EvaluationPriceRow],
    initial_cash: &Money4,
    profile: CostProfile,
) -> Result<Vec<LedgerPhaseRecord>, SimulationContractError> {
    let mut records = Vec::with_capacity(record_capacity(evaluation_prices.len())?);
    let first_date = *schedule
        .evaluation_sessions()
        .first()
        .ok_or(SimulationContractError::InvalidSchedule)?;
    append_record(
        &mut records,
        first_date,
        LedgerPhase::InitialCash,
        CashPosition {
            cash_units: initial_cash.units()?,
            quantity: 0,
        },
        None,
        Vec::new(),
        UnfilledReason::NotApplicable,
    )?;

    let mut state = CashPosition {
        cash_units: initial_cash.units()?,
        quantity: 0,
    };
    let mut previous_target = 0;
    for (index, (signal, row)) in signals.iter().zip(evaluation_prices).enumerate() {
        let target = signal
            .targets()
            .find(|(instrument, _)| *instrument == TREND_INSTRUMENT)
            .map(|(_, weight)| weight)
            .ok_or(SimulationContractError::InvalidValue)?;

        append_record(
            &mut records,
            row.session_date,
            LedgerPhase::PreOpenMark,
            state,
            Some(&row.raw_open),
            Vec::new(),
            UnfilledReason::NotApplicable,
        )?;

        let mut sell_events = Vec::new();
        let mut sell_reason = UnfilledReason::NoOrderRequired;
        if previous_target == FULL_WEIGHT_PPM && target == 0 && state.quantity > 0 {
            if row.raw_volume == 0 {
                return Err(SimulationContractError::InvalidLedger);
            }
            let (cash_units, event) = execute_sell(state, &row.raw_open, profile)?;
            state.cash_units = cash_units;
            state.quantity = 0;
            sell_events.push(event);
            sell_reason = UnfilledReason::Filled;
        }
        append_record(
            &mut records,
            row.session_date,
            LedgerPhase::Sell,
            state,
            Some(&row.raw_open),
            sell_events,
            sell_reason,
        )?;

        let mut buy_events = Vec::new();
        let mut buy_reason = UnfilledReason::NoOrderRequired;
        if previous_target == 0 && target == FULL_WEIGHT_PPM {
            let quantity =
                maximum_affordable_quantity(state.cash_units, row.raw_open.units()?, profile)?;
            if quantity == 0 {
                buy_reason = UnfilledReason::ZeroShareQuantity;
            } else {
                if row.raw_volume == 0 {
                    return Err(SimulationContractError::InvalidLedger);
                }
                let (cash_units, event) = execute_buy(state, quantity, &row.raw_open, profile)?;
                state.cash_units = cash_units;
                state.quantity = quantity;
                buy_events.push(event);
                buy_reason = UnfilledReason::Filled;
            }
        }
        append_record(
            &mut records,
            row.session_date,
            LedgerPhase::Buy,
            state,
            Some(&row.raw_open),
            buy_events,
            buy_reason,
        )?;

        append_record(
            &mut records,
            row.session_date,
            LedgerPhase::CloseMark,
            state,
            Some(&row.raw_close),
            Vec::new(),
            UnfilledReason::NotApplicable,
        )?;
        if index + 1 < evaluation_prices.len() {
            append_record(
                &mut records,
                row.session_date,
                LedgerPhase::SignalForNextSession,
                state,
                Some(&row.raw_close),
                Vec::new(),
                UnfilledReason::NotApplicable,
            )?;
        }
        previous_target = target;
    }
    Ok(records)
}

fn build_benchmark_records(
    evaluation_prices: &[EvaluationPriceRow],
    initial_cash: &Money4,
    profile: CostProfile,
) -> Result<Vec<LedgerPhaseRecord>, SimulationContractError> {
    let mut records = Vec::with_capacity(record_capacity(evaluation_prices.len())?);
    let first_date = evaluation_prices
        .first()
        .ok_or(SimulationContractError::InvalidSchedule)?
        .session_date;
    append_record(
        &mut records,
        first_date,
        LedgerPhase::InitialCash,
        CashPosition {
            cash_units: initial_cash.units()?,
            quantity: 0,
        },
        None,
        Vec::new(),
        UnfilledReason::NotApplicable,
    )?;

    let mut state = CashPosition {
        cash_units: initial_cash.units()?,
        quantity: 0,
    };
    for (index, row) in evaluation_prices.iter().enumerate() {
        append_record(
            &mut records,
            row.session_date,
            LedgerPhase::PreOpenMark,
            state,
            Some(&row.raw_open),
            Vec::new(),
            UnfilledReason::NotApplicable,
        )?;
        append_record(
            &mut records,
            row.session_date,
            LedgerPhase::Sell,
            state,
            Some(&row.raw_open),
            Vec::new(),
            UnfilledReason::NoOrderRequired,
        )?;

        let mut buy_events = Vec::new();
        let mut buy_reason = UnfilledReason::NoOrderRequired;
        if index == 0 {
            let quantity =
                maximum_affordable_quantity(state.cash_units, row.raw_open.units()?, profile)?;
            if quantity == 0 {
                buy_reason = UnfilledReason::ZeroShareQuantity;
            } else {
                if row.raw_volume == 0 {
                    return Err(SimulationContractError::InvalidLedger);
                }
                let (cash_units, event) = execute_buy(state, quantity, &row.raw_open, profile)?;
                state.cash_units = cash_units;
                state.quantity = quantity;
                buy_events.push(event);
                buy_reason = UnfilledReason::Filled;
            }
        }
        append_record(
            &mut records,
            row.session_date,
            LedgerPhase::Buy,
            state,
            Some(&row.raw_open),
            buy_events,
            buy_reason,
        )?;
        append_record(
            &mut records,
            row.session_date,
            LedgerPhase::CloseMark,
            state,
            Some(&row.raw_close),
            Vec::new(),
            UnfilledReason::NotApplicable,
        )?;
        if index + 1 < evaluation_prices.len() {
            append_record(
                &mut records,
                row.session_date,
                LedgerPhase::SignalForNextSession,
                state,
                Some(&row.raw_close),
                Vec::new(),
                UnfilledReason::NotApplicable,
            )?;
        }
    }
    Ok(records)
}

fn record_capacity(session_count: usize) -> Result<usize, SimulationContractError> {
    session_count
        .checked_mul(5)
        .ok_or(SimulationContractError::ArithmeticOverflow)
}

fn append_record(
    records: &mut Vec<LedgerPhaseRecord>,
    session_date: TradingDate,
    phase: LedgerPhase,
    state: CashPosition,
    mark_price: Option<&Price4>,
    events: Vec<TradeEvent>,
    unfilled_reason: UnfilledReason,
) -> Result<(), SimulationContractError> {
    let mut positions = Vec::new();
    if state.quantity > 0 {
        let price = mark_price.ok_or(SimulationContractError::InvalidLedger)?;
        positions.push(PositionMark::new(
            TREND_INSTRUMENT,
            state.quantity,
            price.clone(),
        )?);
    }
    let marked_value = positions.iter().try_fold(0i128, |sum, position| {
        sum.checked_add(position.market_value().units()?)
            .ok_or(SimulationContractError::ArithmeticOverflow)
    })?;
    let nav_units = state
        .cash_units
        .checked_add(marked_value)
        .ok_or(SimulationContractError::ArithmeticOverflow)?;
    if state.cash_units < 0 || nav_units <= 0 {
        return Err(SimulationContractError::InvalidLedger);
    }
    let date = session_date.to_string();
    records.push(LedgerPhaseRecord::new(
        &date,
        phase,
        events,
        unfilled_reason,
        Money4::from_units(state.cash_units)?,
        positions,
        Money4::from_units(nav_units)?,
    )?);
    Ok(())
}

fn execute_buy(
    state: CashPosition,
    quantity: u64,
    price: &Price4,
    profile: CostProfile,
) -> Result<(i128, TradeEvent), SimulationContractError> {
    if quantity == 0 || state.quantity != 0 {
        return Err(SimulationContractError::InvalidLedger);
    }
    let notional = checked_notional(price, quantity)?;
    let fee = fee_units(notional, profile)?;
    let debit = notional
        .checked_add(fee)
        .ok_or(SimulationContractError::ArithmeticOverflow)?;
    let cash_after = state
        .cash_units
        .checked_sub(debit)
        .ok_or(SimulationContractError::ArithmeticOverflow)?;
    if cash_after < 0 {
        return Err(SimulationContractError::InvalidLedger);
    }
    let event = TradeEvent::new(
        TREND_INSTRUMENT,
        TradeSide::Buy,
        quantity,
        price.clone(),
        Money4::from_units(fee)?,
        Money4::from_units(cash_after)?,
        quantity,
    )?;
    Ok((cash_after, event))
}

fn execute_sell(
    state: CashPosition,
    price: &Price4,
    profile: CostProfile,
) -> Result<(i128, TradeEvent), SimulationContractError> {
    if state.quantity == 0 {
        return Err(SimulationContractError::InvalidLedger);
    }
    let notional = checked_notional(price, state.quantity)?;
    let fee = fee_units(notional, profile)?;
    let proceeds = notional
        .checked_sub(fee)
        .ok_or(SimulationContractError::ArithmeticOverflow)?;
    let cash_after = state
        .cash_units
        .checked_add(proceeds)
        .ok_or(SimulationContractError::ArithmeticOverflow)?;
    let event = TradeEvent::new(
        TREND_INSTRUMENT,
        TradeSide::Sell,
        state.quantity,
        price.clone(),
        Money4::from_units(fee)?,
        Money4::from_units(cash_after)?,
        0,
    )?;
    Ok((cash_after, event))
}

fn checked_notional(price: &Price4, quantity: u64) -> Result<i128, SimulationContractError> {
    price
        .units()?
        .checked_mul(i128::from(quantity))
        .ok_or(SimulationContractError::ArithmeticOverflow)
}

fn fee_units(notional_units: i128, profile: CostProfile) -> Result<i128, SimulationContractError> {
    if notional_units < 0 {
        return Err(SimulationContractError::InvalidLedger);
    }
    match profile {
        CostProfile::Illustrative10BpPerSideV1 => {
            let quotient = notional_units / ILLUSTRATIVE_FEE_DENOMINATOR;
            let remainder = notional_units % ILLUSTRATIVE_FEE_DENOMINATOR;
            quotient
                .checked_add(if remainder == 0 { 0 } else { 1 })
                .ok_or(SimulationContractError::ArithmeticOverflow)
        }
        CostProfile::ZeroCostReferenceV1 => Ok(0),
    }
}

fn maximum_affordable_quantity(
    cash_units: i128,
    price_units: i128,
    profile: CostProfile,
) -> Result<u64, SimulationContractError> {
    if cash_units < 0 || price_units <= 0 {
        return Err(SimulationContractError::InvalidLedger);
    }
    if cash_units == 0 {
        return Ok(0);
    }

    // Ignoring a nonnegative fee gives a checked upper bound. Binary search
    // visits at most 127 candidates, independent of the share count.
    let upper_bound = cash_units / price_units;
    let mut low = 0i128;
    let mut high = upper_bound;
    while low < high {
        let distance = high
            .checked_sub(low)
            .ok_or(SimulationContractError::ArithmeticOverflow)?;
        let midpoint = low
            .checked_add(distance / 2)
            .and_then(|value| value.checked_add(distance % 2))
            .ok_or(SimulationContractError::ArithmeticOverflow)?;
        if quantity_is_affordable(midpoint, cash_units, price_units, profile)? {
            low = midpoint;
        } else {
            high = midpoint
                .checked_sub(1)
                .ok_or(SimulationContractError::ArithmeticOverflow)?;
        }
    }
    if !quantity_is_affordable(low, cash_units, price_units, profile)?
        || (low < upper_bound
            && quantity_is_affordable(
                low.checked_add(1)
                    .ok_or(SimulationContractError::ArithmeticOverflow)?,
                cash_units,
                price_units,
                profile,
            )?)
    {
        return Err(SimulationContractError::InvalidLedger);
    }
    u64::try_from(low).map_err(|_| SimulationContractError::ArithmeticOverflow)
}

fn quantity_is_affordable(
    quantity: i128,
    cash_units: i128,
    price_units: i128,
    profile: CostProfile,
) -> Result<bool, SimulationContractError> {
    if quantity < 0 {
        return Err(SimulationContractError::InvalidLedger);
    }
    let notional = price_units
        .checked_mul(quantity)
        .ok_or(SimulationContractError::ArithmeticOverflow)?;
    if notional > cash_units {
        return Ok(false);
    }
    let remaining_cash = cash_units
        .checked_sub(notional)
        .ok_or(SimulationContractError::ArithmeticOverflow)?;
    Ok(fee_units(notional, profile)? <= remaining_cash)
}

fn derive_ledger_metrics(
    initial_cash: &Money4,
    records: &[LedgerPhaseRecord],
) -> Result<LedgerMetrics, SimulationContractError> {
    let derived = derive_performance_metrics(initial_cash, records)?;
    let mut costs = 0i128;
    let mut buys = 0i128;
    let mut sells = 0i128;
    let mut fill_count = 0u64;
    for event in records.iter().flat_map(|record| record.events()) {
        costs = checked_sum(costs, event.cost().units()?)?;
        match event.side() {
            TradeSide::Buy => buys = checked_sum(buys, event.notional().units()?)?,
            TradeSide::Sell => sells = checked_sum(sells, event.notional().units()?)?,
        }
        fill_count = fill_count
            .checked_add(1)
            .ok_or(SimulationContractError::ArithmeticOverflow)?;
    }

    let final_close = records
        .iter()
        .rev()
        .find(|record| record.phase() == LedgerPhase::CloseMark)
        .ok_or(SimulationContractError::InvalidLedger)?;
    let ending_holdings = final_close
        .positions()
        .iter()
        .map(|position| (position.instrument_id().to_owned(), position.quantity()))
        .collect();
    let daily_returns = derived
        .daily_returns()
        .iter()
        .map(|(date, value)| (date.to_string(), value.clone()))
        .collect();

    LedgerMetrics::new(
        daily_returns,
        derived.period_return().clone(),
        final_close.nav().clone(),
        final_close.cash().clone(),
        ending_holdings,
        Money4::from_units(costs)?,
        Money4::from_units(buys)?,
        Money4::from_units(sells)?,
        derived.maximum_drawdown().clone(),
        derived.period_turnover().clone(),
        fill_count,
    )
}

fn ending_nav(records: &[LedgerPhaseRecord]) -> Result<&Money4, SimulationContractError> {
    records
        .last()
        .filter(|record| record.phase() == LedgerPhase::CloseMark)
        .map(LedgerPhaseRecord::nav)
        .ok_or(SimulationContractError::InvalidLedger)
}

fn checked_sum(left: i128, right: i128) -> Result<i128, SimulationContractError> {
    left.checked_add(right)
        .ok_or(SimulationContractError::ArithmeticOverflow)
}

#[cfg(test)]
fn simulate_for_test(
    schedule: &SimulationSchedule,
    signals: &[SignalSnapshot],
    evaluation_prices: &[EvaluationPriceRow],
    initial_capital: &str,
) -> Result<SimulationLedgers, SimulationContractError> {
    build_ledgers(
        schedule,
        signals,
        evaluation_prices,
        Money4::parse(initial_capital)?,
    )
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use chrono::{Datelike, NaiveDate, Weekday};
    use num_bigint::BigInt;
    use num_rational::BigRational;
    use num_traits::{Signed, ToPrimitive, Zero};

    use super::*;
    use crate::owner_beta::simulation::contract::{
        EngineIdentity, Ratio12, SimulationResult, StrategyIdentity,
    };
    use market_data::KR_ETF_CORE_SYMBOLS;

    const HASH_A: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const HASH_B: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const HASH_C: &str = "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    const RATIO_SCALE: i128 = 1_000_000_000_000;

    struct Fixture {
        schedule: SimulationSchedule,
        signals: Vec<SignalSnapshot>,
        prices: Vec<EvaluationPriceRow>,
    }

    fn weekday_dates(count: usize) -> Vec<String> {
        let mut date = NaiveDate::parse_from_str("2016-08-29", "%Y-%m-%d").unwrap();
        let mut dates = Vec::with_capacity(count);
        while dates.len() < count {
            if !matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
                dates.push(date.format("%Y-%m-%d").to_string());
            }
            date = date.succ_opt().unwrap();
        }
        dates
    }

    fn signal(signal_date: &str, buy: bool) -> SignalSnapshot {
        let mut targets = KR_ETF_CORE_SYMBOLS
            .iter()
            .map(|symbol| {
                let instrument = format!("{symbol}.KRX");
                let weight = if buy && instrument == TREND_INSTRUMENT {
                    FULL_WEIGHT_PPM
                } else {
                    0
                };
                (instrument, weight)
            })
            .collect::<Vec<_>>();
        targets.sort_by(|left, right| left.0.cmp(&right.0));
        SignalSnapshot::new(
            signal_date,
            HASH_A,
            HASH_B,
            targets,
            if buy { 0 } else { FULL_WEIGHT_PPM },
        )
        .unwrap()
    }

    fn fixture(buys: &[bool], prices: &[(&str, &str)], volumes: &[u64]) -> Fixture {
        assert_eq!(buys.len(), prices.len());
        assert_eq!(buys.len(), volumes.len());
        let all_dates = weekday_dates(200 + buys.len());
        let schedule =
            SimulationSchedule::new(all_dates[..200].to_vec(), all_dates[200..].to_vec()).unwrap();
        let signals = (0..buys.len())
            .map(|index| {
                let signal_date = if index == 0 {
                    &all_dates[199]
                } else {
                    &all_dates[199 + index]
                };
                signal(signal_date, buys[index])
            })
            .collect();
        let evaluation = (0..buys.len())
            .map(|index| {
                EvaluationPriceRow::new(
                    &all_dates[200 + index],
                    Price4::parse(prices[index].0).unwrap(),
                    Price4::parse(prices[index].1).unwrap(),
                    volumes[index],
                )
                .unwrap()
            })
            .collect();
        Fixture {
            schedule,
            signals,
            prices: evaluation,
        }
    }

    fn metrics_json(metrics: &LedgerMetrics) -> serde_json::Value {
        serde_json::to_value(metrics).unwrap()
    }

    fn metric_text(metrics: &LedgerMetrics, field: &str) -> String {
        metrics_json(metrics)[field].as_str().unwrap().to_owned()
    }

    fn assert_state(
        record: &LedgerPhaseRecord,
        cash: &str,
        quantity: u64,
        mark_price: Option<&str>,
        nav: &str,
    ) {
        assert_eq!(record.cash().as_str(), cash);
        assert_eq!(record.nav().as_str(), nav);
        match (quantity, mark_price, record.positions()) {
            (0, _, []) => {}
            (expected, Some(price), [position]) => {
                assert_eq!(position.quantity(), expected);
                assert_eq!(position.price().as_str(), price);
            }
            other => panic!("unexpected position state: {other:?}"),
        }
    }

    fn expected_records<'a>(
        pair: &'a CostProfileLedgers,
        phases: &[LedgerPhase],
        dates: &[TradingDate],
    ) -> Vec<&'a LedgerPhaseRecord> {
        let records = pair.strategy().records();
        assert_eq!(records.len(), phases.len());
        for (record, (phase, date)) in records.iter().zip(phases.iter().zip(dates)) {
            assert_eq!(record.phase(), *phase);
            assert_eq!(record.session_date(), *date);
        }
        records.iter().collect()
    }

    #[test]
    fn hand_checked_two_session_ledger_matches_both_cost_profiles() {
        let fixture = fixture(
            &[true, false],
            &[("100.0000", "110.0000"), ("120.0000", "120.0000")],
            &[1_000, 1_000],
        );
        let result = simulate_for_test(
            &fixture.schedule,
            &fixture.signals,
            &fixture.prices,
            "1000.0000",
        )
        .unwrap();
        let illustrative = result.illustrative_10bp_per_side();
        assert_eq!(
            illustrative.profile(),
            CostProfile::Illustrative10BpPerSideV1
        );

        let phases = [
            LedgerPhase::InitialCash,
            LedgerPhase::PreOpenMark,
            LedgerPhase::Sell,
            LedgerPhase::Buy,
            LedgerPhase::CloseMark,
            LedgerPhase::SignalForNextSession,
            LedgerPhase::PreOpenMark,
            LedgerPhase::Sell,
            LedgerPhase::Buy,
            LedgerPhase::CloseMark,
        ];
        let dates = [
            fixture.schedule.evaluation_sessions()[0],
            fixture.schedule.evaluation_sessions()[0],
            fixture.schedule.evaluation_sessions()[0],
            fixture.schedule.evaluation_sessions()[0],
            fixture.schedule.evaluation_sessions()[0],
            fixture.schedule.evaluation_sessions()[0],
            fixture.schedule.evaluation_sessions()[1],
            fixture.schedule.evaluation_sessions()[1],
            fixture.schedule.evaluation_sessions()[1],
            fixture.schedule.evaluation_sessions()[1],
        ];
        let records = expected_records(illustrative, &phases, &dates);
        assert_state(records[0], "1000.0000", 0, None, "1000.0000");
        assert_state(records[1], "1000.0000", 0, None, "1000.0000");
        assert_state(records[2], "1000.0000", 0, None, "1000.0000");
        assert_eq!(
            records[2].unfilled_reason(),
            UnfilledReason::NoOrderRequired
        );
        assert_eq!(records[3].unfilled_reason(), UnfilledReason::Filled);
        assert_eq!(records[3].events()[0].quantity(), 9);
        assert_eq!(records[3].events()[0].notional().as_str(), "900.0000");
        assert_eq!(records[3].events()[0].cost().as_str(), "0.9000");
        assert_state(records[3], "99.1000", 9, Some("100.0000"), "999.1000");
        assert_state(records[4], "99.1000", 9, Some("110.0000"), "1089.1000");
        assert_state(records[5], "99.1000", 9, Some("110.0000"), "1089.1000");
        assert_state(records[6], "99.1000", 9, Some("120.0000"), "1179.1000");
        assert_eq!(records[7].events()[0].side(), TradeSide::Sell);
        assert_eq!(records[7].events()[0].quantity(), 9);
        assert_eq!(records[7].events()[0].notional().as_str(), "1080.0000");
        assert_eq!(records[7].events()[0].cost().as_str(), "1.0800");
        assert_state(records[7], "1178.0200", 0, None, "1178.0200");
        assert_eq!(
            records[8].unfilled_reason(),
            UnfilledReason::NoOrderRequired
        );
        assert_state(records[9], "1178.0200", 0, None, "1178.0200");

        let strategy_metrics = illustrative.strategy().metrics();
        assert_eq!(metric_text(strategy_metrics, "ending_nav"), "1178.0200");
        assert_eq!(metric_text(strategy_metrics, "ending_cash"), "1178.0200");
        assert_eq!(metric_text(strategy_metrics, "cumulative_costs"), "1.9800");
        assert_eq!(metric_text(strategy_metrics, "buy_notional"), "900.0000");
        assert_eq!(metric_text(strategy_metrics, "sell_notional"), "1080.0000");
        assert_eq!(metrics_json(strategy_metrics)["fill_count"], 2);
        assert_eq!(
            metric_text(strategy_metrics, "period_return"),
            "0.178020000000"
        );
        assert_eq!(
            metric_text(strategy_metrics, "period_turnover"),
            "0.907976422695"
        );
        assert_eq!(
            metric_text(illustrative.benchmark().metrics(), "ending_nav"),
            "1179.1000"
        );
        assert_eq!(
            metric_text(illustrative.benchmark().metrics(), "period_return"),
            "0.179100000000"
        );
        assert_eq!(
            metric_text(illustrative.benchmark().metrics(), "period_turnover"),
            "0.450000000000"
        );
        let benchmark_records = illustrative.benchmark().records();
        assert_eq!(benchmark_records[7].phase(), LedgerPhase::Sell);
        assert!(benchmark_records[7].events().is_empty());
        assert_eq!(
            benchmark_records[7].unfilled_reason(),
            UnfilledReason::NoOrderRequired
        );
        assert_eq!(benchmark_records[9].positions()[0].quantity(), 9);
        assert_eq!(benchmark_records[9].cash().as_str(), "99.1000");
        assert_eq!(
            illustrative.strategy().records()[7].events()[0]
                .cost()
                .as_str(),
            "1.0800"
        );
        let difference = metrics_json(strategy_metrics);
        assert_eq!(
            difference["period_return"].as_str().unwrap(),
            "0.178020000000"
        );
        assert_eq!(
            serde_json::to_value(illustrative.strategy()).unwrap()["benchmark_return_difference"],
            "-0.001080000000"
        );

        let zero_cost = result.zero_cost_reference();
        assert_eq!(zero_cost.profile(), CostProfile::ZeroCostReferenceV1);
        let zero_strategy = zero_cost.strategy();
        let zero_benchmark = zero_cost.benchmark();
        assert_eq!(zero_strategy.records()[3].events()[0].quantity(), 10);
        assert_eq!(zero_strategy.records()[3].cash().as_str(), "0.0000");
        assert_eq!(
            zero_strategy.records().last().unwrap().nav().as_str(),
            "1200.0000"
        );
        assert_eq!(
            zero_benchmark.records().last().unwrap().nav().as_str(),
            "1200.0000"
        );
        assert_eq!(
            metric_text(zero_strategy.metrics(), "cumulative_costs"),
            "0.0000"
        );
        assert_eq!(
            metric_text(zero_strategy.metrics(), "buy_notional"),
            "1000.0000"
        );
        assert_eq!(
            metric_text(zero_strategy.metrics(), "sell_notional"),
            "1200.0000"
        );
        assert_eq!(metrics_json(zero_strategy.metrics())["fill_count"], 2);
        assert_eq!(
            metric_text(zero_strategy.metrics(), "period_return"),
            "0.200000000000"
        );
        assert_eq!(
            metric_text(zero_benchmark.metrics(), "period_return"),
            "0.200000000000"
        );
    }

    #[test]
    fn cost_ceiling_and_maximum_whole_share_boundary_are_exact() {
        let profile = CostProfile::Illustrative10BpPerSideV1;
        assert_eq!(fee_units(999, profile).unwrap(), 1);
        assert_eq!(fee_units(1_000, profile).unwrap(), 1);
        assert_eq!(fee_units(1_001, profile).unwrap(), 2);
        assert_eq!(
            maximum_affordable_quantity(10_000, 999, profile).unwrap(),
            10
        );
        assert!(quantity_is_affordable(10, 10_000, 999, profile).unwrap());
        assert!(!quantity_is_affordable(11, 10_000, 999, profile).unwrap());
        assert_eq!(
            fee_units(1_001, CostProfile::ZeroCostReferenceV1).unwrap(),
            0
        );
    }

    #[test]
    fn unchanged_buy_target_keeps_residual_cash_and_does_not_retry() {
        let fixture = fixture(
            &[true, true],
            &[("100.0000", "110.0000"), ("90.0000", "100.0000")],
            &[10, 10],
        );
        let result = simulate_for_test(
            &fixture.schedule,
            &fixture.signals,
            &fixture.prices,
            "1000.0000",
        )
        .unwrap();
        let records = result.illustrative_10bp_per_side().strategy().records();
        assert_eq!(records[3].events()[0].quantity(), 9);
        assert_eq!(records[3].cash().as_str(), "99.1000");
        assert_eq!(
            records[8].unfilled_reason(),
            UnfilledReason::NoOrderRequired
        );
        assert!(records[8].events().is_empty());
        assert_eq!(records[8].cash().as_str(), "99.1000");
        assert_eq!(records[8].positions()[0].quantity(), 9);

        let unaffordable_then_cheaper = self::fixture(
            &[true, true, false],
            &[
                ("2000.0000", "2000.0000"),
                ("100.0000", "100.0000"),
                ("90.0000", "90.0000"),
            ],
            &[0, 10, 0],
        );
        let no_retry = simulate_for_test(
            &unaffordable_then_cheaper.schedule,
            &unaffordable_then_cheaper.signals,
            &unaffordable_then_cheaper.prices,
            "1000.0000",
        )
        .unwrap();
        let records = no_retry.illustrative_10bp_per_side().strategy().records();
        assert_eq!(
            records[3].unfilled_reason(),
            UnfilledReason::ZeroShareQuantity
        );
        assert_eq!(records[3].cash().as_str(), "1000.0000");
        assert_eq!(
            records[8].unfilled_reason(),
            UnfilledReason::NoOrderRequired
        );
        assert!(records[8].events().is_empty());
        assert_eq!(records[8].cash().as_str(), "1000.0000");
        assert_eq!(records[12].phase(), LedgerPhase::Sell);
        assert_eq!(
            records[12].unfilled_reason(),
            UnfilledReason::NoOrderRequired
        );
        assert!(records[12].events().is_empty());
        assert_eq!(
            records[13].unfilled_reason(),
            UnfilledReason::NoOrderRequired
        );
        assert_eq!(records[13].cash().as_str(), "1000.0000");
    }

    #[test]
    fn overnight_gap_is_marked_before_trading_and_final_day_is_not_liquidated() {
        let fixture = fixture(
            &[true, true],
            &[("100.0000", "110.0000"), ("120.0000", "130.0000")],
            &[10, 10],
        );
        let result = simulate_for_test(
            &fixture.schedule,
            &fixture.signals,
            &fixture.prices,
            "1000.0000",
        )
        .unwrap();
        let records = result.illustrative_10bp_per_side().strategy().records();
        assert_eq!(records[4].nav().as_str(), "1089.1000");
        assert_eq!(records[6].phase(), LedgerPhase::PreOpenMark);
        assert_eq!(records[6].nav().as_str(), "1179.1000");
        assert_eq!(records[6].positions()[0].price().as_str(), "120.0000");
        assert_eq!(
            records[7].unfilled_reason(),
            UnfilledReason::NoOrderRequired
        );
        assert_eq!(
            records[8].unfilled_reason(),
            UnfilledReason::NoOrderRequired
        );
        assert_eq!(records[9].phase(), LedgerPhase::CloseMark);
        assert_eq!(records[9].nav().as_str(), "1269.1000");
        assert_eq!(records[9].positions()[0].quantity(), 9);
        assert_eq!(records[9].positions()[0].price().as_str(), "130.0000");
    }

    #[test]
    fn no_trade_cash_returns_are_zero_and_volume_is_needed_only_for_a_fill() {
        let no_trade = fixture(
            &[false, false],
            &[("2000.0000", "1900.0000"), ("1800.0000", "1700.0000")],
            &[0, 0],
        );
        let result = simulate_for_test(
            &no_trade.schedule,
            &no_trade.signals,
            &no_trade.prices,
            "1000.0000",
        )
        .unwrap();
        let metrics = result.illustrative_10bp_per_side().strategy().metrics();
        assert_eq!(metric_text(metrics, "ending_cash"), "1000.0000");
        assert_eq!(metric_text(metrics, "ending_nav"), "1000.0000");
        assert_eq!(metric_text(metrics, "period_return"), "0.000000000000");
        assert_eq!(metric_text(metrics, "maximum_drawdown"), "0.000000000000");
        assert_eq!(metric_text(metrics, "period_turnover"), "0.000000000000");
        assert_eq!(metrics_json(metrics)["fill_count"], 0);
        assert!(
            result
                .illustrative_10bp_per_side()
                .strategy()
                .records()
                .iter()
                .filter(|record| record.phase() == LedgerPhase::Buy)
                .all(|record| record.unfilled_reason() == UnfilledReason::NoOrderRequired)
        );

        let required_fill_without_volume = fixture(
            &[true, false],
            &[("100.0000", "110.0000"), ("120.0000", "120.0000")],
            &[0, 0],
        );
        assert_eq!(
            simulate_for_test(
                &required_fill_without_volume.schedule,
                &required_fill_without_volume.signals,
                &required_fill_without_volume.prices,
                "1000.0000",
            )
            .unwrap_err(),
            SimulationContractError::InvalidLedger
        );
    }

    #[test]
    fn second_cost_profile_failure_rejects_the_entire_comparison() {
        let scenario = fixture(
            &[true, false],
            &[("1000.0000", "1000.0000"), ("1000.0000", "1000.0000")],
            &[0, 0],
        );
        let initial_cash = Money4::parse("1000.0000").unwrap();
        // The fee makes one share unaffordable in the first profile. The
        // zero-cost profile requires a fill and therefore rejects zero volume.
        assert!(
            build_cost_profile_ledgers(
                &scenario.schedule,
                &scenario.signals,
                &scenario.prices,
                &initial_cash,
                CostProfile::Illustrative10BpPerSideV1,
            )
            .is_ok()
        );
        assert_eq!(
            simulate_for_test(
                &scenario.schedule,
                &scenario.signals,
                &scenario.prices,
                "1000.0000",
            )
            .unwrap_err(),
            SimulationContractError::InvalidLedger
        );
    }

    #[test]
    fn positive_negative_zero_returns_and_close_to_close_drawdown_are_recorded() {
        let fixture = fixture(
            &[true, true, true],
            &[
                ("100.0000", "110.0000"),
                ("100.0000", "80.0000"),
                ("90.0000", "100.0000"),
            ],
            &[100, 100, 100],
        );
        let result = simulate_for_test(
            &fixture.schedule,
            &fixture.signals,
            &fixture.prices,
            "1000.0000",
        )
        .unwrap();
        let metrics = result.illustrative_10bp_per_side().strategy().metrics();
        let daily = metrics_json(metrics);
        let daily_returns = daily["daily_returns"].as_array().unwrap();
        let first = Ratio12::parse(daily_returns[0]["value"].as_str().unwrap()).unwrap();
        let second = Ratio12::parse(daily_returns[1]["value"].as_str().unwrap()).unwrap();
        let third = Ratio12::parse(daily_returns[2]["value"].as_str().unwrap()).unwrap();
        assert!(first.units().unwrap() > 0);
        assert!(second.units().unwrap() < 0);
        assert!(third.units().unwrap() > 0);
        assert_eq!(metric_text(metrics, "maximum_drawdown"), "0.247911119273");

        let flat = self::fixture(
            &[false, false],
            &[("2000.0000", "2000.0000"), ("2000.0000", "2000.0000")],
            &[0, 0],
        );
        let flat_result =
            simulate_for_test(&flat.schedule, &flat.signals, &flat.prices, "1000.0000").unwrap();
        assert_eq!(
            metric_text(
                flat_result
                    .illustrative_10bp_per_side()
                    .strategy()
                    .metrics(),
                "period_return"
            ),
            "0.000000000000"
        );
    }

    #[test]
    fn row_and_signal_alignment_errors_fail_the_entire_comparison() {
        let fixture = fixture(
            &[true, false],
            &[("100.0000", "110.0000"), ("120.0000", "120.0000")],
            &[10, 10],
        );
        assert_eq!(
            simulate_for_test(
                &fixture.schedule,
                &fixture.signals,
                &fixture.prices[..1],
                "1000.0000",
            )
            .unwrap_err(),
            SimulationContractError::InvalidSchedule
        );

        let mut duplicate_rows = fixture.prices.clone();
        duplicate_rows[1] = duplicate_rows[0].clone();
        assert_eq!(
            simulate_for_test(
                &fixture.schedule,
                &fixture.signals,
                &duplicate_rows,
                "1000.0000",
            )
            .unwrap_err(),
            SimulationContractError::InvalidSchedule
        );

        let mut misordered_rows = fixture.prices.clone();
        misordered_rows.swap(0, 1);
        assert_eq!(
            simulate_for_test(
                &fixture.schedule,
                &fixture.signals,
                &misordered_rows,
                "1000.0000",
            )
            .unwrap_err(),
            SimulationContractError::InvalidSchedule
        );

        assert_eq!(
            simulate_for_test(
                &fixture.schedule,
                &fixture.signals[..1],
                &fixture.prices,
                "1000.0000",
            )
            .unwrap_err(),
            SimulationContractError::InvalidSchedule
        );
        let mut duplicate_signals = fixture.signals.clone();
        duplicate_signals[1] = duplicate_signals[0].clone();
        assert_eq!(
            simulate_for_test(
                &fixture.schedule,
                &duplicate_signals,
                &fixture.prices,
                "1000.0000",
            )
            .unwrap_err(),
            SimulationContractError::InvalidSchedule
        );
        let mut misordered_signals = fixture.signals.clone();
        misordered_signals.swap(0, 1);
        assert_eq!(
            simulate_for_test(
                &fixture.schedule,
                &misordered_signals,
                &fixture.prices,
                "1000.0000",
            )
            .unwrap_err(),
            SimulationContractError::InvalidSchedule
        );
        let unmatched = signal("2017-01-02", true);
        let mut unmatched_signals = fixture.signals.clone();
        unmatched_signals[0] = unmatched;
        assert_eq!(
            simulate_for_test(
                &fixture.schedule,
                &unmatched_signals,
                &fixture.prices,
                "1000.0000",
            )
            .unwrap_err(),
            SimulationContractError::InvalidSchedule
        );
    }

    #[test]
    fn invalid_prices_cash_and_arithmetic_overflow_fail_closed() {
        assert!(Price4::parse("0.0000").is_err());
        assert!(
            EvaluationPriceRow::new(
                "not-a-date",
                Price4::parse("1.0000").unwrap(),
                Price4::parse("1.0000").unwrap(),
                1,
            )
            .is_err()
        );
        let invalid_capital = fixture(
            &[false, false],
            &[("1.0000", "1.0000"), ("1.0000", "1.0000")],
            &[0, 0],
        );
        assert_eq!(
            simulate_for_test(
                &invalid_capital.schedule,
                &invalid_capital.signals,
                &invalid_capital.prices,
                "0.0000",
            )
            .unwrap_err(),
            SimulationContractError::InvalidLedger
        );
        assert_eq!(
            maximum_affordable_quantity(i128::MAX, 1, CostProfile::ZeroCostReferenceV1,)
                .unwrap_err(),
            SimulationContractError::ArithmeticOverflow
        );
        assert_eq!(
            checked_notional(
                &Price4::parse("17014118346046923173168730371588410.5727").unwrap(),
                2,
            )
            .unwrap_err(),
            SimulationContractError::ArithmeticOverflow
        );
    }

    #[test]
    fn maximum_approved_schedule_length_accumulates_varying_turnover_exactly() {
        let total_dates = weekday_dates(2_452);
        let evaluation_count = 2_452 - 200;
        let schedule =
            SimulationSchedule::new(total_dates[..200].to_vec(), total_dates[200..].to_vec())
                .unwrap();
        let signals = (0..evaluation_count)
            .map(|index| {
                let signal_date = if index == 0 {
                    &total_dates[199]
                } else {
                    &total_dates[199 + index]
                };
                signal(signal_date, index % 2 == 0)
            })
            .collect::<Vec<_>>();
        let prices = (0..evaluation_count)
            .map(|index| {
                let open = 50 + index % 13;
                let close = 45 + index % 17;
                EvaluationPriceRow::new(
                    &total_dates[200 + index],
                    Price4::parse(&format!("{open}.0000")).unwrap(),
                    Price4::parse(&format!("{close}.0000")).unwrap(),
                    1_000_000,
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let result = simulate_ledgers(&schedule, &signals, &prices).unwrap();
        let records = result.illustrative_10bp_per_side().strategy().records();
        assert_eq!(records.len(), evaluation_count * 5);
        assert_eq!(
            metrics_json(result.illustrative_10bp_per_side().strategy().metrics())["daily_returns"]
                .as_array()
                .unwrap()
                .len(),
            evaluation_count
        );

        let mut by_date = BTreeMap::<TradingDate, (i128, i128)>::new();
        for record in records {
            match record.phase() {
                LedgerPhase::PreOpenMark => {
                    by_date.entry(record.session_date()).or_default().0 =
                        record.nav().units().unwrap();
                }
                LedgerPhase::Buy | LedgerPhase::Sell => {
                    let entry = by_date.entry(record.session_date()).or_default();
                    for event in record.events() {
                        entry.1 += event.notional().units().unwrap();
                    }
                }
                _ => {}
            }
        }
        let mut exact_turnover = BigRational::zero();
        let mut denominators = BTreeSet::new();
        for (open_nav, notional) in by_date.values() {
            if *notional > 0 {
                let denominator = BigInt::from(*open_nav) * BigInt::from(2);
                denominators.insert(denominator.clone());
                exact_turnover += BigRational::new(BigInt::from(*notional), denominator);
            }
        }
        assert!(denominators.len() > 10);
        assert_eq!(
            metric_text(
                result.illustrative_10bp_per_side().strategy().metrics(),
                "period_turnover"
            ),
            independent_ratio12(exact_turnover).as_str()
        );
    }

    #[test]
    fn production_capital_ledgers_pass_the_closed_simulation_result_validator() {
        let fixture = fixture(
            &[true, false],
            &[("100.0000", "110.0000"), ("120.0000", "120.0000")],
            &[1_000_000, 1_000_000],
        );
        let ledgers =
            simulate_ledgers(&fixture.schedule, &fixture.signals, &fixture.prices).unwrap();
        assert_eq!(
            ledgers
                .illustrative_10bp_per_side()
                .strategy()
                .initial_cash()
                .as_str(),
            PRODUCTION_INITIAL_CAPITAL
        );
        let engine = EngineIdentity::new(
            &"a".repeat(40),
            HASH_C,
            "rustc-1.97.1",
            "x86_64-unknown-linux-gnu",
            "test",
        )
        .unwrap();
        let strategy =
            StrategyIdentity::trend_following_v1(HASH_A, "price-only-v1", HASH_B).unwrap();
        let result = SimulationResult::new(
            engine,
            strategy,
            fixture.schedule,
            CostProfile::Illustrative10BpPerSideV1,
            fixture.signals,
            ledgers.illustrative_10bp_per_side().strategy().clone(),
            ledgers.illustrative_10bp_per_side().benchmark().clone(),
        )
        .unwrap();
        assert!(!result.canonical_json().unwrap().is_empty());
    }

    fn independent_ratio12(value: BigRational) -> Ratio12 {
        let scaled = value.numer() * BigInt::from(RATIO_SCALE);
        let denominator = value.denom();
        let quotient = &scaled / denominator;
        let remainder = &scaled % denominator;
        let rounded = match (remainder.abs() * BigInt::from(2)).cmp(denominator) {
            std::cmp::Ordering::Less => quotient,
            std::cmp::Ordering::Greater => quotient + independent_rounding_direction(&scaled),
            std::cmp::Ordering::Equal => {
                if (&quotient % BigInt::from(2)).is_zero() {
                    quotient
                } else {
                    quotient + independent_rounding_direction(&scaled)
                }
            }
        };
        Ratio12::from_units(rounded.to_i128().unwrap()).unwrap()
    }

    fn independent_rounding_direction(value: &BigInt) -> BigInt {
        if value.is_negative() {
            BigInt::from(-1)
        } else {
            BigInt::from(1)
        }
    }
}
