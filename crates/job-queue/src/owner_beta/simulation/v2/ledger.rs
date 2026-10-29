//! Checked whole-share, multiple-instrument v2 ledger.

use std::collections::BTreeMap;

use domain::TradingDate;
use serde::{Deserialize, Serialize};

use super::super::{CostProfile, Money4, Price4};
use super::contract::{V2ContractError, V2TargetAllocation, approved_instruments};
use super::metrics::{V2PerformanceMetrics, derive_v2_performance_metrics};
use super::schedule::{V2Decision, V2DecisionKind, V2Schedule};

/// One member of a complete raw-open/raw-close ETF11 session inventory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct V2InstrumentPrice {
    instrument_id: String,
    open: Price4,
    close: Price4,
    volume: u64,
}

impl V2InstrumentPrice {
    pub fn instrument_id(&self) -> &str {
        &self.instrument_id
    }

    pub fn open(&self) -> &Price4 {
        &self.open
    }

    pub fn close(&self) -> &Price4 {
        &self.close
    }

    pub const fn volume(&self) -> u64 {
        self.volume
    }
}

/// One complete, sorted price and volume inventory for an approved session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct V2MarketSession {
    date: TradingDate,
    prices: Vec<V2InstrumentPrice>,
}

impl V2MarketSession {
    pub fn new(
        date: &str,
        prices: Vec<(String, String, String, u64)>,
    ) -> Result<Self, V2ContractError> {
        let date_text = date;
        let date = TradingDate::parse(date_text).map_err(|_| V2ContractError::InvalidValue)?;
        if date.is_weekend() || date.to_iso() != date_text {
            return Err(V2ContractError::InvalidValue);
        }
        let expected = approved_instruments();
        if prices.len() != expected.len()
            || prices
                .iter()
                .map(|(instrument, _, _, _)| instrument.as_str())
                .ne(expected.iter().map(String::as_str))
        {
            return Err(V2ContractError::InvalidValue);
        }
        let prices = prices
            .into_iter()
            .map(|(instrument_id, open, close, volume)| {
                Ok(V2InstrumentPrice {
                    instrument_id,
                    open: Price4::parse(&open).map_err(V2ContractError::from)?,
                    close: Price4::parse(&close).map_err(V2ContractError::from)?,
                    volume,
                })
            })
            .collect::<Result<Vec<_>, V2ContractError>>()?;
        Ok(Self { date, prices })
    }

    pub const fn date(&self) -> TradingDate {
        self.date
    }

    pub fn prices(&self) -> &[V2InstrumentPrice] {
        &self.prices
    }

    fn price(&self, instrument_id: &str) -> Result<&V2InstrumentPrice, V2ContractError> {
        self.prices
            .binary_search_by(|item| item.instrument_id.as_str().cmp(instrument_id))
            .map(|index| &self.prices[index])
            .map_err(|_| V2ContractError::InvalidValue)
    }
}

/// A nonzero whole-share portfolio holding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct V2Position {
    instrument_id: String,
    quantity: u64,
}

impl V2Position {
    pub fn instrument_id(&self) -> &str {
        &self.instrument_id
    }

    pub const fn quantity(&self) -> u64 {
        self.quantity
    }
}

/// Trade direction in the canonical v2 ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum V2OrderSide {
    #[serde(rename = "SELL")]
    Sell,
    #[serde(rename = "BUY")]
    Buy,
}

/// A budget-constrained buy with no filled shares is recorded once at that
/// decision. It is reconsidered only at the next scheduled rebalance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum V2UnfilledReason {
    #[serde(rename = "INSUFFICIENT_COST_INCLUSIVE_BUDGET")]
    InsufficientCostInclusiveBudget,
}

/// One combined instrument/direction order for one execution session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct V2Order {
    pub(crate) instrument_id: String,
    pub(crate) side: V2OrderSide,
    pub(crate) requested_quantity: u64,
    pub(crate) filled_quantity: u64,
    pub(crate) price: Price4,
    pub(crate) notional: Money4,
    pub(crate) cost: Money4,
    pub(crate) cash_after: Money4,
    pub(crate) position_after: u64,
    pub(crate) unfilled_reason: Option<V2UnfilledReason>,
}

impl V2Order {
    pub fn instrument_id(&self) -> &str {
        &self.instrument_id
    }

    pub const fn side(&self) -> V2OrderSide {
        self.side
    }

    pub const fn requested_quantity(&self) -> u64 {
        self.requested_quantity
    }

    pub const fn filled_quantity(&self) -> u64 {
        self.filled_quantity
    }

    pub fn notional(&self) -> &Money4 {
        &self.notional
    }

    pub fn cost(&self) -> &Money4 {
        &self.cost
    }

    pub const fn unfilled_reason(&self) -> Option<V2UnfilledReason> {
        self.unfilled_reason
    }
}

/// Complete auditable outcome of one scheduled rebalance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct V2RebalanceRecord {
    pub(crate) signal_date: TradingDate,
    pub(crate) execution_date: TradingDate,
    pub(crate) decision_kind: V2DecisionKind,
    pub(crate) target: V2TargetAllocation,
    pub(crate) pre_trade_nav: Money4,
    pub(crate) open_prices: Vec<V2InstrumentPrice>,
    pub(crate) target_quantities: Vec<V2Position>,
    pub(crate) orders: Vec<V2Order>,
    pub(crate) cash_after: Money4,
    pub(crate) positions_after: Vec<V2Position>,
    pub(crate) session_costs: Money4,
    pub(crate) buy_notional: Money4,
    pub(crate) sell_notional: Money4,
}

impl V2RebalanceRecord {
    pub const fn signal_date(&self) -> TradingDate {
        self.signal_date
    }

    pub const fn execution_date(&self) -> TradingDate {
        self.execution_date
    }

    pub const fn decision_kind(&self) -> V2DecisionKind {
        self.decision_kind
    }

    pub fn target(&self) -> &V2TargetAllocation {
        &self.target
    }

    pub fn pre_trade_nav(&self) -> &Money4 {
        &self.pre_trade_nav
    }

    pub fn orders(&self) -> &[V2Order] {
        &self.orders
    }

    pub fn cash_after(&self) -> &Money4 {
        &self.cash_after
    }

    pub fn positions_after(&self) -> &[V2Position] {
        &self.positions_after
    }
}

/// One checked target output observed for a scheduled signal. Trend schedules
/// retain every daily observation, including unchanged allocation states.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct V2TargetObservation {
    signal_date: TradingDate,
    execution_date: TradingDate,
    decision_kind: V2DecisionKind,
    target: V2TargetAllocation,
}

impl V2TargetObservation {
    pub const fn signal_date(&self) -> TradingDate {
        self.signal_date
    }

    pub const fn execution_date(&self) -> TradingDate {
        self.execution_date
    }

    pub const fn decision_kind(&self) -> V2DecisionKind {
        self.decision_kind
    }

    pub fn target(&self) -> &V2TargetAllocation {
        &self.target
    }
}

/// The state machine is intentionally explicit: open a session, apply at most
/// one scheduled target at raw open, then mark raw close.
pub struct V2Ledger {
    profile: CostProfile,
    initial_capital: Money4,
    cash_units: i128,
    positions: BTreeMap<String, u64>,
    active_target: Option<V2TargetAllocation>,
    target_observations: Vec<V2TargetObservation>,
    current_session: Option<V2MarketSession>,
    opening_nav_units: Option<i128>,
    opening_cash_units: Option<i128>,
    opening_positions: Option<BTreeMap<String, u64>>,
    opening_marks: Vec<V2CloseMark>,
    daily_records: Vec<V2DailyRecord>,
    rebalances: Vec<V2RebalanceRecord>,
    market_sessions: Vec<V2MarketSession>,
    last_date: Option<TradingDate>,
}

impl V2Ledger {
    /// Production capital is fixed at 10,000,000.0000 KRW. Synthetic tests
    /// use a module-private constructor; no production caller can lower the
    /// starting balance.
    pub fn new(profile: CostProfile) -> Result<Self, V2ContractError> {
        Self::from_initial_capital(
            profile,
            Money4::parse(super::contract::V2_INITIAL_CAPITAL).map_err(V2ContractError::from)?,
        )
    }

    fn from_initial_capital(
        profile: CostProfile,
        initial_capital: Money4,
    ) -> Result<Self, V2ContractError> {
        let cash_units = initial_capital.units().map_err(V2ContractError::from)?;
        if cash_units <= 0 {
            return Err(V2ContractError::InvalidLedger);
        }
        Ok(Self {
            profile,
            initial_capital,
            cash_units,
            positions: BTreeMap::new(),
            active_target: None,
            target_observations: Vec::new(),
            current_session: None,
            opening_nav_units: None,
            opening_cash_units: None,
            opening_positions: None,
            opening_marks: Vec::new(),
            daily_records: Vec::new(),
            rebalances: Vec::new(),
            market_sessions: Vec::new(),
            last_date: None,
        })
    }

    #[cfg(test)]
    fn with_test_capital(
        profile: CostProfile,
        initial_capital: &str,
    ) -> Result<Self, V2ContractError> {
        Self::from_initial_capital(
            profile,
            Money4::parse(initial_capital).map_err(V2ContractError::from)?,
        )
    }

    pub const fn profile(&self) -> CostProfile {
        self.profile
    }

    pub fn open_session(&mut self, session: V2MarketSession) -> Result<Money4, V2ContractError> {
        if self.current_session.is_some() || self.last_date.is_some_and(|date| date >= session.date)
        {
            return Err(V2ContractError::InvalidLedger);
        }
        let mut nav = self.cash_units;
        let mut marks = Vec::with_capacity(self.positions.len());
        for (instrument_id, quantity) in &self.positions {
            let price = session.price(instrument_id)?.open.clone();
            nav = nav
                .checked_add(
                    price
                        .units()
                        .map_err(V2ContractError::from)?
                        .checked_mul(i128::from(*quantity))
                        .ok_or(V2ContractError::ArithmeticOverflow)?,
                )
                .ok_or(V2ContractError::ArithmeticOverflow)?;
            marks.push(V2CloseMark {
                instrument_id: instrument_id.clone(),
                price,
            });
        }
        if nav <= 0 {
            return Err(V2ContractError::InvalidLedger);
        }
        self.opening_nav_units = Some(nav);
        self.opening_cash_units = Some(self.cash_units);
        self.opening_positions = Some(self.positions.clone());
        self.opening_marks = marks;
        self.current_session = Some(session);
        Money4::from_units(nav).map_err(V2ContractError::from)
    }

    /// Applies an initial or monthly scheduled target. Trend decisions use
    /// [`Self::observe_trend_target`] so unchanged daily targets cannot trade.
    pub fn rebalance(
        &mut self,
        decision: &V2Decision,
        target: V2TargetAllocation,
    ) -> Result<&V2RebalanceRecord, V2ContractError> {
        if decision.kind() == V2DecisionKind::DailyTrendTransitionCheck
            || self
                .target_observations
                .last()
                .is_some_and(|last| last.execution_date >= decision.execution_date())
        {
            return Err(V2ContractError::InvalidLedger);
        }
        self.rebalance_core(decision, target.clone())?;
        self.target_observations
            .push(target_observation(decision, target.clone()));
        self.active_target = Some(target);
        self.rebalances.last().ok_or(V2ContractError::InvalidLedger)
    }

    /// Records every daily trend target. A new rebalance is allowed only when
    /// the allocation state changes between the two approved 0/100% states;
    /// target hashes alone never trigger execution.
    pub fn observe_trend_target(
        &mut self,
        schedule: &V2Schedule,
        decision: &V2Decision,
        target: V2TargetAllocation,
    ) -> Result<bool, V2ContractError> {
        if schedule.strategy() != super::contract::V2Strategy::TrendFollowing
            || schedule
                .decisions()
                .get(self.target_observations.len())
                .is_none_or(|expected| expected != decision)
            || !matches!(
                decision.kind(),
                V2DecisionKind::InitialCommonSession | V2DecisionKind::DailyTrendTransitionCheck
            )
            || self
                .target_observations
                .last()
                .is_some_and(|last| last.execution_date >= decision.execution_date())
            || !is_valid_trend_target(&target)
            || self
                .current_session
                .as_ref()
                .is_none_or(|session| session.date != decision.execution_date())
        {
            return Err(V2ContractError::InvalidLedger);
        }

        let transitioned = self
            .target_observations
            .last()
            .is_none_or(|previous| !same_target_allocation(&previous.target, &target));
        if transitioned {
            self.rebalance_core(decision, target.clone())?;
            self.active_target = Some(target.clone());
        }
        self.target_observations
            .push(target_observation(decision, target));
        Ok(transitioned)
    }

    fn rebalance_core(
        &mut self,
        decision: &V2Decision,
        target: V2TargetAllocation,
    ) -> Result<(), V2ContractError> {
        let session = self
            .current_session
            .as_ref()
            .ok_or(V2ContractError::InvalidLedger)?;
        if session.date != decision.execution_date()
            || self
                .rebalances
                .last()
                .is_some_and(|record| record.execution_date == session.date)
        {
            return Err(V2ContractError::InvalidLedger);
        }
        let nav = self
            .opening_nav_units
            .ok_or(V2ContractError::InvalidLedger)?;
        let target_quantities = target_quantities(nav, &target, session)?;
        let mut orders = Vec::new();
        let mut total_cost = 0_i128;
        let mut buy_notional = 0_i128;
        let mut sell_notional = 0_i128;

        // First sell every excess quantity, in canonical instrument order.
        for desired in &target_quantities {
            let held = self
                .positions
                .get(&desired.instrument_id)
                .copied()
                .unwrap_or(0);
            if held <= desired.quantity {
                continue;
            }
            let quantity = held - desired.quantity;
            let quote = session.price(&desired.instrument_id)?;
            require_volume_for_fill(quote, quantity)?;
            let notional = checked_notional(&quote.open, quantity)?;
            let cost = fee_units(self.profile, notional)?;
            let proceeds = notional
                .checked_sub(cost)
                .ok_or(V2ContractError::ArithmeticOverflow)?;
            self.cash_units = self
                .cash_units
                .checked_add(proceeds)
                .ok_or(V2ContractError::ArithmeticOverflow)?;
            if desired.quantity == 0 {
                self.positions.remove(&desired.instrument_id);
            } else {
                self.positions
                    .insert(desired.instrument_id.clone(), desired.quantity);
            }
            total_cost = total_cost
                .checked_add(cost)
                .ok_or(V2ContractError::ArithmeticOverflow)?;
            sell_notional = sell_notional
                .checked_add(notional)
                .ok_or(V2ContractError::ArithmeticOverflow)?;
            orders.push(order(
                &desired.instrument_id,
                V2OrderSide::Sell,
                quantity,
                quantity,
                quote.open.clone(),
                notional,
                cost,
                self.cash_units,
                desired.quantity,
                None,
            )?);
        }

        // The cash reserve is based on the one frozen, cost-free pretrade NAV.
        let desired_reserve = checked_mul_div_floor(
            nav,
            i128::from(target.cash_weight_ppm()),
            i128::from(super::contract::V2_WEIGHT_SCALE_PPM),
        )?;
        let reserve = self.cash_units.min(desired_reserve);
        let buy_budget = self
            .cash_units
            .checked_sub(reserve)
            .ok_or(V2ContractError::ArithmeticOverflow)?;

        let mut deficits = Vec::new();
        let mut total_needed = 0_i128;
        for desired in &target_quantities {
            let held = self
                .positions
                .get(&desired.instrument_id)
                .copied()
                .unwrap_or(0);
            let quantity = desired.quantity.saturating_sub(held);
            if quantity == 0 {
                continue;
            }
            let quote = session.price(&desired.instrument_id)?;
            let notional = checked_notional(&quote.open, quantity)?;
            let fee = fee_units(self.profile, notional)?;
            let need = notional
                .checked_add(fee)
                .ok_or(V2ContractError::ArithmeticOverflow)?;
            total_needed = total_needed
                .checked_add(need)
                .ok_or(V2ContractError::ArithmeticOverflow)?;
            deficits.push((desired, quantity, need));
        }

        for (desired, shortage, need) in deficits {
            let allocation = if total_needed <= buy_budget {
                need
            } else if total_needed == 0 {
                0
            } else {
                checked_mul_div_floor(buy_budget, need, total_needed)?
            };
            let quote = session.price(&desired.instrument_id)?;
            let quantity = max_affordable_shares(self.profile, &quote.open, shortage, allocation)?;
            if quantity == 0 {
                orders.push(order(
                    &desired.instrument_id,
                    V2OrderSide::Buy,
                    shortage,
                    0,
                    quote.open.clone(),
                    0,
                    0,
                    self.cash_units,
                    self.positions
                        .get(&desired.instrument_id)
                        .copied()
                        .unwrap_or(0),
                    Some(V2UnfilledReason::InsufficientCostInclusiveBudget),
                )?);
                continue;
            }
            require_volume_for_fill(quote, quantity)?;
            let notional = checked_notional(&quote.open, quantity)?;
            let cost = fee_units(self.profile, notional)?;
            let debit = notional
                .checked_add(cost)
                .ok_or(V2ContractError::ArithmeticOverflow)?;
            if debit > self.cash_units {
                return Err(V2ContractError::InvalidLedger);
            }
            self.cash_units = self
                .cash_units
                .checked_sub(debit)
                .ok_or(V2ContractError::ArithmeticOverflow)?;
            let position_after = self
                .positions
                .get(&desired.instrument_id)
                .copied()
                .unwrap_or(0)
                .checked_add(quantity)
                .ok_or(V2ContractError::ArithmeticOverflow)?;
            self.positions
                .insert(desired.instrument_id.clone(), position_after);
            total_cost = total_cost
                .checked_add(cost)
                .ok_or(V2ContractError::ArithmeticOverflow)?;
            buy_notional = buy_notional
                .checked_add(notional)
                .ok_or(V2ContractError::ArithmeticOverflow)?;
            orders.push(order(
                &desired.instrument_id,
                V2OrderSide::Buy,
                shortage,
                quantity,
                quote.open.clone(),
                notional,
                cost,
                self.cash_units,
                position_after,
                (quantity < shortage).then_some(V2UnfilledReason::InsufficientCostInclusiveBudget),
            )?);
        }

        let record = V2RebalanceRecord {
            signal_date: decision.signal_date(),
            execution_date: decision.execution_date(),
            decision_kind: decision.kind(),
            target,
            pre_trade_nav: Money4::from_units(nav).map_err(V2ContractError::from)?,
            open_prices: session.prices.clone(),
            target_quantities,
            orders,
            cash_after: Money4::from_units(self.cash_units).map_err(V2ContractError::from)?,
            positions_after: positions(&self.positions),
            session_costs: Money4::from_units(total_cost).map_err(V2ContractError::from)?,
            buy_notional: Money4::from_units(buy_notional).map_err(V2ContractError::from)?,
            sell_notional: Money4::from_units(sell_notional).map_err(V2ContractError::from)?,
        };
        self.rebalances.push(record);
        Ok(())
    }

    pub fn close_session(&mut self) -> Result<(), V2ContractError> {
        let session = self
            .current_session
            .take()
            .ok_or(V2ContractError::InvalidLedger)?;
        let session_date = session.date;
        let mut nav = self.cash_units;
        let mut close_marks = Vec::with_capacity(self.positions.len());
        for (instrument_id, quantity) in &self.positions {
            if *quantity == 0 {
                return Err(V2ContractError::InvalidLedger);
            }
            let price = session.price(instrument_id)?.close.clone();
            nav = nav
                .checked_add(
                    price
                        .units()
                        .map_err(V2ContractError::from)?
                        .checked_mul(i128::from(*quantity))
                        .ok_or(V2ContractError::ArithmeticOverflow)?,
                )
                .ok_or(V2ContractError::ArithmeticOverflow)?;
            close_marks.push(V2CloseMark {
                instrument_id: instrument_id.clone(),
                price,
            });
        }
        if nav <= 0 {
            return Err(V2ContractError::InvalidLedger);
        }
        self.daily_records.push(V2DailyRecord {
            date: session.date,
            opening_cash: Money4::from_units(
                self.opening_cash_units
                    .take()
                    .ok_or(V2ContractError::InvalidLedger)?,
            )
            .map_err(V2ContractError::from)?,
            opening_positions: positions(
                &self
                    .opening_positions
                    .take()
                    .ok_or(V2ContractError::InvalidLedger)?,
            ),
            opening_marks: std::mem::take(&mut self.opening_marks),
            opening_nav: Money4::from_units(
                self.opening_nav_units
                    .take()
                    .ok_or(V2ContractError::InvalidLedger)?,
            )
            .map_err(V2ContractError::from)?,
            closing_cash: Money4::from_units(self.cash_units).map_err(V2ContractError::from)?,
            closing_positions: positions(&self.positions),
            close_marks,
            closing_nav: Money4::from_units(nav).map_err(V2ContractError::from)?,
            target_lineage: self.active_target.clone(),
        });
        self.market_sessions.push(session);
        self.last_date = Some(session_date);
        Ok(())
    }

    pub fn finish(self, schedule: &V2Schedule) -> Result<V2LedgerRun, V2ContractError> {
        if self.current_session.is_some()
            || self.daily_records.len() != schedule.evaluation_sessions().len()
            || self
                .daily_records
                .iter()
                .map(|record| record.date)
                .ne(schedule.evaluation_sessions().iter().copied())
        {
            return Err(V2ContractError::InvalidLedger);
        }
        let metrics = derive_v2_performance_metrics(
            &self.initial_capital,
            &self.daily_records,
            &self.rebalances,
        )?;
        let run = V2LedgerRun {
            profile: self.profile,
            initial_capital: self.initial_capital,
            daily_records: self.daily_records,
            rebalances: self.rebalances,
            market_sessions: self.market_sessions,
            target_observations: self.target_observations,
            metrics,
        };
        run.validate_trace(schedule)?;
        Ok(run)
    }
}

/// Raw-close price evidence for a held instrument on one valuation date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct V2CloseMark {
    instrument_id: String,
    price: Price4,
}

impl V2CloseMark {
    pub fn instrument_id(&self) -> &str {
        &self.instrument_id
    }

    pub fn price(&self) -> &Price4 {
        &self.price
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct V2DailyRecord {
    pub(crate) date: TradingDate,
    pub(crate) opening_cash: Money4,
    pub(crate) opening_positions: Vec<V2Position>,
    pub(crate) opening_marks: Vec<V2CloseMark>,
    pub(crate) opening_nav: Money4,
    pub(crate) closing_cash: Money4,
    pub(crate) closing_positions: Vec<V2Position>,
    pub(crate) close_marks: Vec<V2CloseMark>,
    pub(crate) closing_nav: Money4,
    pub(crate) target_lineage: Option<V2TargetAllocation>,
}

/// Complete portfolio trace for one independently executed cost profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct V2LedgerRun {
    pub(crate) profile: CostProfile,
    pub(crate) initial_capital: Money4,
    pub(crate) daily_records: Vec<V2DailyRecord>,
    pub(crate) rebalances: Vec<V2RebalanceRecord>,
    pub(crate) market_sessions: Vec<V2MarketSession>,
    pub(crate) target_observations: Vec<V2TargetObservation>,
    pub(crate) metrics: V2PerformanceMetrics,
}

impl V2LedgerRun {
    pub const fn profile(&self) -> CostProfile {
        self.profile
    }

    pub fn initial_capital(&self) -> &Money4 {
        &self.initial_capital
    }

    pub fn metrics(&self) -> &V2PerformanceMetrics {
        &self.metrics
    }

    pub fn daily_session_count(&self) -> usize {
        self.daily_records.len()
    }

    pub fn rebalances(&self) -> &[V2RebalanceRecord] {
        &self.rebalances
    }

    /// Complete approved-session market path used to replay this result.
    pub fn market_sessions(&self) -> &[V2MarketSession] {
        &self.market_sessions
    }

    /// Target lineage active at the close of one evaluation session. This is
    /// carried forward between decisions and does not imply a trade occurred.
    pub fn target_lineage_for_session(&self, date: TradingDate) -> Option<&V2TargetAllocation> {
        self.daily_records
            .iter()
            .find(|record| record.date == date)
            .and_then(|record| record.target_lineage.as_ref())
    }

    pub fn positions_at_close(&self, date: TradingDate) -> Option<&[V2Position]> {
        self.daily_records
            .iter()
            .find(|record| record.date == date)
            .map(|record| record.closing_positions.as_slice())
    }

    pub fn target_observations(&self) -> &[V2TargetObservation] {
        &self.target_observations
    }

    pub(crate) fn validate_trace(&self, schedule: &V2Schedule) -> Result<(), V2ContractError> {
        if self
            .initial_capital
            .units()
            .map_err(V2ContractError::from)?
            <= 0
            || self.daily_records.len() != schedule.evaluation_sessions().len()
            || self.market_sessions.len() != schedule.evaluation_sessions().len()
            || self
                .daily_records
                .iter()
                .map(|record| record.date)
                .ne(schedule.evaluation_sessions().iter().copied())
            || self.target_observations.len() != schedule.decisions().len()
        {
            return Err(V2ContractError::InvalidLedger);
        }

        for (observation, decision) in self.target_observations.iter().zip(schedule.decisions()) {
            if observation.signal_date != decision.signal_date()
                || observation.execution_date != decision.execution_date()
                || observation.decision_kind != decision.kind()
                || (schedule.strategy() == super::contract::V2Strategy::TrendFollowing
                    && !is_valid_trend_target(&observation.target))
            {
                return Err(V2ContractError::InvalidLedger);
            }
        }

        let mut replay =
            V2Ledger::from_initial_capital(self.profile, self.initial_capital.clone())?;
        let mut observation_index = 0_usize;
        for (index, expected_date) in schedule.evaluation_sessions().iter().enumerate() {
            let stored = self
                .daily_records
                .get(index)
                .ok_or(V2ContractError::InvalidLedger)?;
            let market = self
                .market_sessions
                .get(index)
                .ok_or(V2ContractError::InvalidLedger)?;
            if stored.date != *expected_date || market.date != *expected_date {
                return Err(V2ContractError::InvalidLedger);
            }
            replay.open_session(market.clone())?;
            if replay.opening_marks != stored.opening_marks
                || replay.opening_nav_units
                    != Some(stored.opening_nav.units().map_err(V2ContractError::from)?)
                || replay.cash_units
                    != stored.opening_cash.units().map_err(V2ContractError::from)?
                || positions(&replay.positions) != stored.opening_positions
            {
                return Err(V2ContractError::InvalidLedger);
            }

            if let Some(decision) = schedule.decision_for_execution(*expected_date) {
                let observation = self
                    .target_observations
                    .get(observation_index)
                    .ok_or(V2ContractError::InvalidLedger)?;
                if schedule.strategy() == super::contract::V2Strategy::TrendFollowing {
                    replay.observe_trend_target(schedule, decision, observation.target.clone())?;
                } else {
                    replay.rebalance(decision, observation.target.clone())?;
                }
                observation_index += 1;
            }
            replay.close_session()?;
            if replay.daily_records.last() != Some(stored) {
                return Err(V2ContractError::InvalidLedger);
            }
        }
        if observation_index != self.target_observations.len()
            || replay.market_sessions != self.market_sessions
            || replay.target_observations != self.target_observations
            || replay.rebalances != self.rebalances
            || replay.daily_records != self.daily_records
        {
            return Err(V2ContractError::InvalidLedger);
        }
        let recomputed = derive_v2_performance_metrics(
            &self.initial_capital,
            &self.daily_records,
            &self.rebalances,
        )?;
        if recomputed != self.metrics {
            return Err(V2ContractError::InvalidLedger);
        }
        Ok(())
    }

    pub(crate) fn from_json_value(
        value: serde_json::Value,
        schedule: &V2Schedule,
    ) -> Result<Self, V2ContractError> {
        let wire: V2LedgerRunWire =
            serde_json::from_value(value).map_err(|_| V2ContractError::InvalidSchema)?;
        let daily_records = wire
            .daily_records
            .into_iter()
            .map(|day| {
                Ok(V2DailyRecord {
                    date: parse_date(&day.date)?,
                    opening_cash: parse_money(&day.opening_cash)?,
                    opening_positions: parse_positions(day.opening_positions, false)?,
                    opening_marks: parse_marks(day.opening_marks)?,
                    opening_nav: parse_money(&day.opening_nav)?,
                    closing_cash: parse_money(&day.closing_cash)?,
                    closing_positions: parse_positions(day.closing_positions, false)?,
                    close_marks: parse_marks(day.close_marks)?,
                    closing_nav: parse_money(&day.closing_nav)?,
                    target_lineage: day.target_lineage.map(parse_target).transpose()?,
                })
            })
            .collect::<Result<Vec<_>, V2ContractError>>()?;
        let rebalances = wire
            .rebalances
            .into_iter()
            .map(parse_rebalance)
            .collect::<Result<Vec<_>, _>>()?;
        let market_sessions = wire
            .market_sessions
            .into_iter()
            .map(parse_market_session)
            .collect::<Result<Vec<_>, _>>()?;
        let target_observations = wire
            .target_observations
            .into_iter()
            .map(parse_target_observation)
            .collect::<Result<Vec<_>, _>>()?;
        let initial_capital = parse_money(&wire.initial_capital)?;
        let metrics = derive_v2_performance_metrics(&initial_capital, &daily_records, &rebalances)?;
        let encoded_metrics =
            serde_json::to_value(&metrics).map_err(|_| V2ContractError::InvalidJson)?;
        let supplied_metrics =
            serde_json::to_value(&wire.metrics).map_err(|_| V2ContractError::InvalidJson)?;
        if encoded_metrics != supplied_metrics {
            return Err(V2ContractError::InvalidLedger);
        }
        let result = Self {
            profile: wire.profile,
            initial_capital,
            daily_records,
            rebalances,
            market_sessions,
            target_observations,
            metrics,
        };
        result.validate_trace(schedule)?;
        Ok(result)
    }

    pub fn ending_positions(&self) -> &[V2Position] {
        self.daily_records
            .last()
            .map_or(&[], |record| record.closing_positions.as_slice())
    }

    pub fn ending_nav(&self) -> &Money4 {
        &self
            .daily_records
            .last()
            .expect("finish requires a complete schedule")
            .closing_nav
    }

    pub fn economically_equal(&self, other: &Self) -> bool {
        self.profile == other.profile
            && self.initial_capital == other.initial_capital
            && self.daily_records.len() == other.daily_records.len()
            && self
                .daily_records
                .iter()
                .zip(&other.daily_records)
                .all(|(left, right)| {
                    left.date == right.date
                        && left.opening_cash == right.opening_cash
                        && left.opening_positions == right.opening_positions
                        && left.opening_marks == right.opening_marks
                        && left.opening_nav == right.opening_nav
                        && left.closing_cash == right.closing_cash
                        && left.closing_positions == right.closing_positions
                        && left.close_marks == right.close_marks
                        && left.closing_nav == right.closing_nav
                })
            && self.economic_rebalances() == other.economic_rebalances()
            && self.market_sessions == other.market_sessions
            && self.metrics == other.metrics
    }

    fn economic_rebalances(&self) -> Vec<EconomicRebalance<'_>> {
        self.rebalances
            .iter()
            .map(|record| EconomicRebalance {
                signal_date: record.signal_date,
                execution_date: record.execution_date,
                target_weights: record.target.weights(),
                cash_weight_ppm: record.target.cash_weight_ppm(),
                open_prices: &record.open_prices,
                target_quantities: &record.target_quantities,
                orders: &record.orders,
                cash_after: &record.cash_after,
                positions_after: &record.positions_after,
                cumulative_costs: &record.session_costs,
                buy_notional: &record.buy_notional,
                sell_notional: &record.sell_notional,
            })
            .collect()
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct V2LedgerRunWire {
    profile: CostProfile,
    initial_capital: String,
    daily_records: Vec<V2DailyRecordWire>,
    rebalances: Vec<V2RebalanceRecordWire>,
    market_sessions: Vec<V2MarketSessionWire>,
    target_observations: Vec<V2TargetObservationWire>,
    metrics: V2MetricsWire,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct V2DailyRecordWire {
    date: String,
    opening_cash: String,
    opening_positions: Vec<V2PositionWire>,
    opening_marks: Vec<V2CloseMarkWire>,
    opening_nav: String,
    closing_cash: String,
    closing_positions: Vec<V2PositionWire>,
    close_marks: Vec<V2CloseMarkWire>,
    closing_nav: String,
    target_lineage: Option<V2TargetAllocationWire>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct V2PositionWire {
    instrument_id: String,
    quantity: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct V2CloseMarkWire {
    instrument_id: String,
    price: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct V2InstrumentWeightWire {
    instrument_id: String,
    weight_ppm: u32,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct V2TargetAllocationWire {
    target_sha256: String,
    allocation_sha256: String,
    lineage_sha256: String,
    weights: Vec<V2InstrumentWeightWire>,
    cash_weight_ppm: u32,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct V2InstrumentPriceWire {
    instrument_id: String,
    open: String,
    close: String,
    volume: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct V2MarketSessionWire {
    date: String,
    prices: Vec<V2InstrumentPriceWire>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct V2TargetObservationWire {
    signal_date: String,
    execution_date: String,
    decision_kind: V2DecisionKind,
    target: V2TargetAllocationWire,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct V2OrderWire {
    instrument_id: String,
    side: V2OrderSide,
    requested_quantity: u64,
    filled_quantity: u64,
    price: String,
    notional: String,
    cost: String,
    cash_after: String,
    position_after: u64,
    unfilled_reason: Option<V2UnfilledReason>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct V2RebalanceRecordWire {
    signal_date: String,
    execution_date: String,
    decision_kind: V2DecisionKind,
    target: V2TargetAllocationWire,
    pre_trade_nav: String,
    open_prices: Vec<V2InstrumentPriceWire>,
    target_quantities: Vec<V2PositionWire>,
    orders: Vec<V2OrderWire>,
    cash_after: String,
    positions_after: Vec<V2PositionWire>,
    session_costs: String,
    buy_notional: String,
    sell_notional: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct V2DailyReturnWire {
    session_date: String,
    value: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct V2MetricsWire {
    daily_returns: Vec<V2DailyReturnWire>,
    period_return: String,
    ending_nav: String,
    ending_cash: String,
    ending_holdings: Vec<V2PositionWire>,
    cumulative_costs: String,
    buy_notional: String,
    sell_notional: String,
    maximum_drawdown: String,
    period_turnover: String,
    fill_count: u64,
}

fn parse_rebalance(wire: V2RebalanceRecordWire) -> Result<V2RebalanceRecord, V2ContractError> {
    let execution_date = parse_date(&wire.execution_date)?;
    let market = V2MarketSession::new(
        &wire.execution_date,
        wire.open_prices
            .into_iter()
            .map(|price| (price.instrument_id, price.open, price.close, price.volume))
            .collect(),
    )?;
    let target = parse_target(wire.target)?;
    let orders = wire
        .orders
        .into_iter()
        .map(|order| {
            Ok(V2Order {
                instrument_id: order.instrument_id,
                side: order.side,
                requested_quantity: order.requested_quantity,
                filled_quantity: order.filled_quantity,
                price: Price4::parse(&order.price).map_err(V2ContractError::from)?,
                notional: parse_money(&order.notional)?,
                cost: parse_money(&order.cost)?,
                cash_after: parse_money(&order.cash_after)?,
                position_after: order.position_after,
                unfilled_reason: order.unfilled_reason,
            })
        })
        .collect::<Result<Vec<_>, V2ContractError>>()?;
    Ok(V2RebalanceRecord {
        signal_date: parse_date(&wire.signal_date)?,
        execution_date,
        decision_kind: wire.decision_kind,
        target,
        pre_trade_nav: parse_money(&wire.pre_trade_nav)?,
        open_prices: market.prices,
        target_quantities: parse_positions(wire.target_quantities, true)?,
        orders,
        cash_after: parse_money(&wire.cash_after)?,
        positions_after: parse_positions(wire.positions_after, false)?,
        session_costs: parse_money(&wire.session_costs)?,
        buy_notional: parse_money(&wire.buy_notional)?,
        sell_notional: parse_money(&wire.sell_notional)?,
    })
}

fn parse_target(wire: V2TargetAllocationWire) -> Result<V2TargetAllocation, V2ContractError> {
    let weights = wire
        .weights
        .into_iter()
        .map(|weight| (weight.instrument_id, weight.weight_ppm))
        .collect();
    let target = V2TargetAllocation::new(&wire.target_sha256, weights, wire.cash_weight_ppm)?;
    if target.allocation_sha256().as_str() != wire.allocation_sha256
        || target.lineage_sha256().as_str() != wire.lineage_sha256
    {
        return Err(V2ContractError::InvalidLedger);
    }
    Ok(target)
}

fn parse_market_session(wire: V2MarketSessionWire) -> Result<V2MarketSession, V2ContractError> {
    V2MarketSession::new(
        &wire.date,
        wire.prices
            .into_iter()
            .map(|price| (price.instrument_id, price.open, price.close, price.volume))
            .collect(),
    )
}

fn parse_target_observation(
    wire: V2TargetObservationWire,
) -> Result<V2TargetObservation, V2ContractError> {
    Ok(V2TargetObservation {
        signal_date: parse_date(&wire.signal_date)?,
        execution_date: parse_date(&wire.execution_date)?,
        decision_kind: wire.decision_kind,
        target: parse_target(wire.target)?,
    })
}

fn parse_date(value: &str) -> Result<TradingDate, V2ContractError> {
    let date = TradingDate::parse(value).map_err(|_| V2ContractError::InvalidValue)?;
    if date.to_iso() != value || date.is_weekend() {
        return Err(V2ContractError::InvalidValue);
    }
    Ok(date)
}

fn parse_money(value: &str) -> Result<Money4, V2ContractError> {
    Money4::parse(value).map_err(V2ContractError::from)
}

fn parse_positions(
    values: Vec<V2PositionWire>,
    allow_zero: bool,
) -> Result<Vec<V2Position>, V2ContractError> {
    let expected = approved_instruments();
    let mut output = Vec::with_capacity(values.len());
    for value in values {
        if (!allow_zero && value.quantity == 0)
            || !expected
                .binary_search_by(|item| item.as_str().cmp(&value.instrument_id))
                .is_ok()
            || output.last().is_some_and(|previous: &V2Position| {
                previous.instrument_id.as_str() >= value.instrument_id.as_str()
            })
        {
            return Err(V2ContractError::InvalidLedger);
        }
        output.push(V2Position {
            instrument_id: value.instrument_id,
            quantity: value.quantity,
        });
    }
    if allow_zero && output.len() != expected.len() {
        return Err(V2ContractError::InvalidLedger);
    }
    Ok(output)
}

fn parse_marks(values: Vec<V2CloseMarkWire>) -> Result<Vec<V2CloseMark>, V2ContractError> {
    let expected = approved_instruments();
    let mut output = Vec::with_capacity(values.len());
    for value in values {
        if !expected
            .binary_search_by(|item| item.as_str().cmp(&value.instrument_id))
            .is_ok()
            || output.last().is_some_and(|previous: &V2CloseMark| {
                previous.instrument_id.as_str() >= value.instrument_id.as_str()
            })
        {
            return Err(V2ContractError::InvalidLedger);
        }
        output.push(V2CloseMark {
            instrument_id: value.instrument_id,
            price: Price4::parse(&value.price).map_err(V2ContractError::from)?,
        });
    }
    Ok(output)
}

#[derive(PartialEq, Eq)]
struct EconomicRebalance<'a> {
    signal_date: TradingDate,
    execution_date: TradingDate,
    target_weights: &'a [super::contract::V2InstrumentWeight],
    cash_weight_ppm: u32,
    open_prices: &'a [V2InstrumentPrice],
    target_quantities: &'a [V2Position],
    orders: &'a [V2Order],
    cash_after: &'a Money4,
    positions_after: &'a [V2Position],
    cumulative_costs: &'a Money4,
    buy_notional: &'a Money4,
    sell_notional: &'a Money4,
}

fn target_quantities(
    nav: i128,
    target: &V2TargetAllocation,
    session: &V2MarketSession,
) -> Result<Vec<V2Position>, V2ContractError> {
    if nav <= 0 {
        return Err(V2ContractError::InvalidLedger);
    }
    target
        .weights()
        .iter()
        .map(|weight| {
            let price = session
                .price(weight.instrument_id())?
                .open
                .units()
                .map_err(V2ContractError::from)?;
            let desired = checked_mul_div_floor(
                nav,
                i128::from(weight.weight_ppm()),
                i128::from(super::contract::V2_WEIGHT_SCALE_PPM)
                    .checked_mul(price)
                    .ok_or(V2ContractError::ArithmeticOverflow)?,
            )?;
            let quantity =
                u64::try_from(desired).map_err(|_| V2ContractError::ArithmeticOverflow)?;
            Ok(V2Position {
                instrument_id: weight.instrument_id().to_owned(),
                quantity,
            })
        })
        .collect()
}

fn target_observation(decision: &V2Decision, target: V2TargetAllocation) -> V2TargetObservation {
    V2TargetObservation {
        signal_date: decision.signal_date(),
        execution_date: decision.execution_date(),
        decision_kind: decision.kind(),
        target,
    }
}

fn same_target_allocation(left: &V2TargetAllocation, right: &V2TargetAllocation) -> bool {
    left.weights() == right.weights() && left.cash_weight_ppm() == right.cash_weight_ppm()
}

fn is_valid_trend_target(target: &V2TargetAllocation) -> bool {
    let is_cash = target.cash_weight_ppm() == super::contract::V2_WEIGHT_SCALE_PPM
        && target
            .weights()
            .iter()
            .all(|weight| weight.weight_ppm() == 0);
    let is_invested = target.cash_weight_ppm() == 0
        && target.weights().iter().all(|weight| {
            if weight.instrument_id() == "069500.KRX" {
                weight.weight_ppm() == super::contract::V2_WEIGHT_SCALE_PPM
            } else {
                weight.weight_ppm() == 0
            }
        });
    is_cash || is_invested
}

fn checked_notional(price: &Price4, quantity: u64) -> Result<i128, V2ContractError> {
    price
        .units()
        .map_err(V2ContractError::from)?
        .checked_mul(i128::from(quantity))
        .ok_or(V2ContractError::ArithmeticOverflow)
}

fn fee_units(profile: CostProfile, notional: i128) -> Result<i128, V2ContractError> {
    if notional < 0 {
        return Err(V2ContractError::InvalidLedger);
    }
    match profile {
        CostProfile::Illustrative10BpPerSideV1 => notional
            .checked_add(999)
            .ok_or(V2ContractError::ArithmeticOverflow)
            .map(|value| value / 1_000),
        CostProfile::ZeroCostReferenceV1 => Ok(0),
    }
}

fn checked_mul_div_floor(a: i128, b: i128, denominator: i128) -> Result<i128, V2ContractError> {
    if a < 0 || b < 0 || denominator <= 0 {
        return Err(V2ContractError::InvalidLedger);
    }
    a.checked_mul(b)
        .ok_or(V2ContractError::ArithmeticOverflow)
        .map(|product| product / denominator)
}

fn max_affordable_shares(
    profile: CostProfile,
    price: &Price4,
    maximum: u64,
    budget: i128,
) -> Result<u64, V2ContractError> {
    let mut low = 0_u64;
    let mut high = maximum;
    while low < high {
        let distance = high - low;
        let midpoint = low + distance / 2 + distance % 2;
        let notional = checked_notional(price, midpoint)?;
        let required = notional
            .checked_add(fee_units(profile, notional)?)
            .ok_or(V2ContractError::ArithmeticOverflow)?;
        if required <= budget {
            low = midpoint;
        } else {
            high = midpoint - 1;
        }
    }
    Ok(low)
}

fn require_volume_for_fill(
    price: &V2InstrumentPrice,
    quantity: u64,
) -> Result<(), V2ContractError> {
    if quantity > 0 && price.volume == 0 {
        Err(V2ContractError::InvalidLedger)
    } else {
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn order(
    instrument_id: &str,
    side: V2OrderSide,
    requested_quantity: u64,
    filled_quantity: u64,
    price: Price4,
    notional: i128,
    cost: i128,
    cash_after: i128,
    position_after: u64,
    unfilled_reason: Option<V2UnfilledReason>,
) -> Result<V2Order, V2ContractError> {
    if requested_quantity == 0
        || filled_quantity > requested_quantity
        || (filled_quantity < requested_quantity) != unfilled_reason.is_some()
        || (filled_quantity == 0 && (notional != 0 || cost != 0))
    {
        return Err(V2ContractError::InvalidLedger);
    }
    Ok(V2Order {
        instrument_id: instrument_id.to_owned(),
        side,
        requested_quantity,
        filled_quantity,
        price,
        notional: Money4::from_units(notional).map_err(V2ContractError::from)?,
        cost: Money4::from_units(cost).map_err(V2ContractError::from)?,
        cash_after: Money4::from_units(cash_after).map_err(V2ContractError::from)?,
        position_after,
        unfilled_reason,
    })
}

fn positions(positions: &BTreeMap<String, u64>) -> Vec<V2Position> {
    positions
        .iter()
        .filter(|(_, quantity)| **quantity > 0)
        .map(|(instrument_id, quantity)| V2Position {
            instrument_id: instrument_id.clone(),
            quantity: *quantity,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::owner_beta::simulation::v2::V2Strategy;

    const A: &str = "069500.KRX";
    const B: &str = "102110.KRX";

    fn test_hash(byte: u8) -> String {
        format!("sha256:{byte:064x}")
    }

    fn target(weights: &[(&str, u32)], cash: u32, hash_byte: u8) -> V2TargetAllocation {
        let requested = weights.iter().copied().collect::<BTreeMap<_, _>>();
        let weights = approved_instruments()
            .into_iter()
            .map(|instrument| {
                (
                    instrument.clone(),
                    requested.get(instrument.as_str()).copied().unwrap_or(0),
                )
            })
            .collect();
        V2TargetAllocation::new(&test_hash(hash_byte), weights, cash).unwrap()
    }

    fn market(
        date: &str,
        a_open: &str,
        a_close: &str,
        b_open: &str,
        b_close: &str,
    ) -> V2MarketSession {
        let prices = approved_instruments()
            .into_iter()
            .map(|instrument| {
                let (open, close) = match instrument.as_str() {
                    A => (a_open, a_close),
                    B => (b_open, b_close),
                    _ => ("20000.0000", "20000.0000"),
                };
                (instrument, open.to_owned(), close.to_owned(), 10)
            })
            .collect();
        V2MarketSession::new(date, prices).unwrap()
    }

    fn schedule(strategy: V2Strategy, dates: &[&str]) -> V2Schedule {
        V2Schedule::new(
            strategy,
            vec!["2026-01-28".to_owned(), "2026-01-29".to_owned()],
            dates.iter().map(|date| (*date).to_owned()).collect(),
        )
        .unwrap()
    }

    fn run_initial_two_asset_target(profile: CostProfile) -> V2LedgerRun {
        let schedule = schedule(V2Strategy::BuyAndHold, &["2026-01-30", "2026-02-02"]);
        let target = target(&[(A, 500_000), (B, 500_000)], 0, 10);
        let mut ledger = V2Ledger::with_test_capital(profile, "1000.0000").unwrap();
        for date in ["2026-01-30", "2026-02-02"] {
            ledger
                .open_session(market(date, "100.0000", "100.0000", "50.0000", "50.0000"))
                .unwrap();
            if date == "2026-01-30" {
                ledger
                    .rebalance(&schedule.decisions()[0], target.clone())
                    .unwrap();
            }
            ledger.close_session().unwrap();
        }
        ledger.finish(&schedule).unwrap()
    }

    #[test]
    fn adopted_small_arithmetic_examples_match_exact_units() {
        let zero = run_initial_two_asset_target(CostProfile::ZeroCostReferenceV1);
        assert_eq!(zero.ending_positions()[0].quantity(), 5);
        assert_eq!(zero.ending_positions()[1].quantity(), 10);
        assert_eq!(zero.metrics().ending_cash().as_str(), "0.0000");
        assert_eq!(zero.metrics().cumulative_costs().as_str(), "0.0000");

        let costed = run_initial_two_asset_target(CostProfile::Illustrative10BpPerSideV1);
        assert_eq!(costed.ending_positions()[0].quantity(), 4);
        assert_eq!(costed.ending_positions()[1].quantity(), 9);
        assert_eq!(costed.metrics().ending_cash().as_str(), "149.1500");
        assert_eq!(costed.metrics().cumulative_costs().as_str(), "0.8500");
    }

    #[test]
    fn adopted_ten_share_rotation_sells_before_proportional_buy() {
        let schedule = schedule(V2Strategy::RelativeMomentum, &["2026-01-30", "2026-02-02"]);
        let mut ledger =
            V2Ledger::with_test_capital(CostProfile::Illustrative10BpPerSideV1, "1000.0000")
                .unwrap();
        ledger.cash_units = 0;
        ledger.positions.insert(A.to_owned(), 10);
        ledger
            .open_session(market(
                "2026-01-30",
                "100.0000",
                "100.0000",
                "50.0000",
                "50.0000",
            ))
            .unwrap();
        let record = ledger
            .rebalance(&schedule.decisions()[0], target(&[(B, 1_000_000)], 0, 11))
            .unwrap();
        assert_eq!(record.orders().len(), 2);
        assert_eq!(record.orders()[0].side(), V2OrderSide::Sell);
        assert_eq!(record.orders()[0].instrument_id(), A);
        assert_eq!(record.orders()[0].filled_quantity(), 10);
        assert_eq!(record.orders()[1].side(), V2OrderSide::Buy);
        assert_eq!(record.orders()[1].instrument_id(), B);
        assert_eq!(record.orders()[1].filled_quantity(), 19);
        assert_eq!(record.cash_after().as_str(), "48.0500");
        assert_eq!(record.session_costs.as_str(), "1.9500");
    }

    #[test]
    fn adopted_monthly_same_target_rebalances_price_drift() {
        let schedule = schedule(V2Strategy::RelativeMomentum, &["2026-01-30", "2026-02-02"]);
        let target = target(&[(A, 500_000), (B, 500_000)], 0, 12);
        let mut ledger =
            V2Ledger::with_test_capital(CostProfile::Illustrative10BpPerSideV1, "1000.0000")
                .unwrap();
        for (index, date) in ["2026-01-30", "2026-02-02"].iter().enumerate() {
            let prices = if index == 0 {
                market(date, "100.0000", "100.0000", "50.0000", "50.0000")
            } else {
                market(date, "150.0000", "150.0000", "50.0000", "50.0000")
            };
            ledger.open_session(prices).unwrap();
            ledger
                .rebalance(&schedule.decisions()[index], target.clone())
                .unwrap();
            ledger.close_session().unwrap();
        }
        let run = ledger.finish(&schedule).unwrap();
        assert_eq!(run.rebalances()[0].positions_after()[0].quantity(), 4);
        assert_eq!(run.rebalances()[0].positions_after()[1].quantity(), 9);
        assert_eq!(run.rebalances()[0].cash_after().as_str(), "149.1500");
        assert_eq!(run.ending_positions()[0].quantity(), 3);
        assert_eq!(run.ending_positions()[1].quantity(), 11);
        assert_eq!(run.metrics().ending_cash().as_str(), "198.9000");
        assert_eq!(run.metrics().cumulative_costs().as_str(), "1.1000");
        assert_eq!(run.rebalances()[1].session_costs.as_str(), "0.2500");
    }

    #[test]
    fn cash_reserve_and_target_cap_are_distinct_from_realized_weight() {
        let schedule = schedule(V2Strategy::BuyAndHold, &["2026-01-30", "2026-02-02"]);
        let target = target(&[(A, 300_000)], 700_000, 13);
        let mut ledger =
            V2Ledger::with_test_capital(CostProfile::ZeroCostReferenceV1, "1000.0000").unwrap();
        for (index, date) in ["2026-01-30", "2026-02-02"].iter().enumerate() {
            let price = if index == 0 { "100.0000" } else { "200.0000" };
            ledger
                .open_session(market(date, price, price, "50.0000", "50.0000"))
                .unwrap();
            if index == 0 {
                ledger
                    .rebalance(&schedule.decisions()[0], target.clone())
                    .unwrap();
            }
            ledger.close_session().unwrap();
        }
        let run = ledger.finish(&schedule).unwrap();
        assert_eq!(target.weights()[0].weight_ppm(), 300_000);
        assert_eq!(target.cash_weight_ppm(), 700_000);
        assert_eq!(run.metrics().ending_cash().as_str(), "700.0000");
        assert_eq!(run.ending_positions()[0].quantity(), 3);
        let realized_value = i128::from(run.ending_positions()[0].quantity()) * 200_0000;
        let ending_nav = run.metrics().ending_nav().units().unwrap();
        assert!(realized_value * 1_000_000 > ending_nav * 300_000);
        assert_eq!(
            run.target_lineage_for_session(domain::TradingDate::parse("2026-02-02").unwrap(),)
                .unwrap()
                .target_sha256()
                .as_str(),
            target.target_sha256().as_str()
        );
    }

    #[test]
    fn checked_cost_and_budget_arithmetic_rejects_overflow() {
        assert_eq!(
            fee_units(CostProfile::Illustrative10BpPerSideV1, i128::MAX),
            Err(V2ContractError::ArithmeticOverflow)
        );
        assert_eq!(
            checked_mul_div_floor(i128::MAX, 2, 1),
            Err(V2ContractError::ArithmeticOverflow)
        );
    }
}
