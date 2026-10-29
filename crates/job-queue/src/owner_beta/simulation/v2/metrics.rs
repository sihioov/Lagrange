//! Exact, checked v2 metric derivation from the complete price-only ledger.

use domain::TradingDate;
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{Signed, ToPrimitive, Zero};
use serde::Serialize;

use super::super::{Money4, Ratio12};
use super::contract::V2ContractError;
use super::ledger::{V2DailyRecord, V2Position, V2RebalanceRecord};

const RATIO_SCALE: i128 = 1_000_000_000_000;

/// The v2 allowlisted metrics. All values are derived from ledger records;
/// no annualized, Sharpe, PIT, or total-return claim is included.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct V2PerformanceMetrics {
    daily_returns: Vec<V2DailyReturn>,
    period_return: Ratio12,
    ending_nav: Money4,
    ending_cash: Money4,
    ending_holdings: Vec<V2Position>,
    cumulative_costs: Money4,
    buy_notional: Money4,
    sell_notional: Money4,
    maximum_drawdown: Ratio12,
    period_turnover: Ratio12,
    fill_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct V2DailyReturn {
    session_date: TradingDate,
    value: Ratio12,
}

impl V2PerformanceMetrics {
    pub fn daily_returns(&self) -> Vec<(TradingDate, &Ratio12)> {
        self.daily_returns
            .iter()
            .map(|item| (item.session_date, &item.value))
            .collect()
    }

    pub fn period_return(&self) -> &Ratio12 {
        &self.period_return
    }

    pub fn ending_nav(&self) -> &Money4 {
        &self.ending_nav
    }

    pub fn ending_cash(&self) -> &Money4 {
        &self.ending_cash
    }

    pub fn ending_holdings(&self) -> &[V2Position] {
        &self.ending_holdings
    }

    pub fn cumulative_costs(&self) -> &Money4 {
        &self.cumulative_costs
    }

    pub fn buy_notional(&self) -> &Money4 {
        &self.buy_notional
    }

    pub fn sell_notional(&self) -> &Money4 {
        &self.sell_notional
    }

    pub fn maximum_drawdown(&self) -> &Ratio12 {
        &self.maximum_drawdown
    }

    pub fn period_turnover(&self) -> &Ratio12 {
        &self.period_turnover
    }

    pub const fn fill_count(&self) -> u64 {
        self.fill_count
    }
}

/// Recomputes the allowlisted metrics from original fixed-point ledger values.
/// Ratios remain exact until the final 12-place ties-to-even conversion.
pub(crate) fn derive_v2_performance_metrics(
    initial_cash: &Money4,
    daily_records: &[V2DailyRecord],
    rebalances: &[V2RebalanceRecord],
) -> Result<V2PerformanceMetrics, V2ContractError> {
    let initial = initial_cash.units().map_err(V2ContractError::from)?;
    if initial <= 0 || daily_records.len() < 2 {
        return Err(V2ContractError::InvalidLedger);
    }

    let mut daily_returns = Vec::with_capacity(daily_records.len());
    let mut previous = initial;
    let mut high_watermark = initial;
    let mut max_drawdown = BigRational::zero();
    let mut total_cost = 0_i128;
    let mut total_buy = 0_i128;
    let mut total_sell = 0_i128;
    let mut fill_count = 0_u64;
    let mut turnover = BigRational::zero();

    let rebalance_by_date = rebalances
        .iter()
        .map(|record| (record.execution_date, record))
        .collect::<std::collections::BTreeMap<_, _>>();
    if rebalance_by_date.len() != rebalances.len() {
        return Err(V2ContractError::InvalidLedger);
    }

    for (index, day) in daily_records.iter().enumerate() {
        if index > 0 && daily_records[index - 1].date >= day.date {
            return Err(V2ContractError::InvalidLedger);
        }
        let opening_nav = day.opening_nav.units().map_err(V2ContractError::from)?;
        let closing_nav = day.closing_nav.units().map_err(V2ContractError::from)?;
        if opening_nav <= 0 || closing_nav <= 0 {
            return Err(V2ContractError::InvalidLedger);
        }
        daily_returns.push(V2DailyReturn {
            session_date: day.date,
            value: fraction_to_ratio12(
                BigInt::from(closing_nav) - BigInt::from(previous),
                BigInt::from(previous),
            )?,
        });
        previous = closing_nav;
        if closing_nav > high_watermark {
            high_watermark = closing_nav;
        }
        let drawdown = BigRational::new(
            BigInt::from(high_watermark - closing_nav),
            BigInt::from(high_watermark),
        );
        if drawdown > max_drawdown {
            max_drawdown = drawdown;
        }

        if let Some(rebalance) = rebalance_by_date.get(&day.date) {
            let pretrade_nav = rebalance
                .pre_trade_nav
                .units()
                .map_err(V2ContractError::from)?;
            if pretrade_nav != opening_nav {
                return Err(V2ContractError::InvalidLedger);
            }
            let mut day_notional = 0_i128;
            for order in &rebalance.orders {
                let notional = order.notional.units().map_err(V2ContractError::from)?;
                let cost = order.cost.units().map_err(V2ContractError::from)?;
                total_cost = total_cost
                    .checked_add(cost)
                    .ok_or(V2ContractError::ArithmeticOverflow)?;
                day_notional = day_notional
                    .checked_add(notional)
                    .ok_or(V2ContractError::ArithmeticOverflow)?;
                if order.filled_quantity > 0 {
                    fill_count = fill_count
                        .checked_add(1)
                        .ok_or(V2ContractError::ArithmeticOverflow)?;
                    match order.side {
                        super::ledger::V2OrderSide::Buy => {
                            total_buy = total_buy
                                .checked_add(notional)
                                .ok_or(V2ContractError::ArithmeticOverflow)?;
                        }
                        super::ledger::V2OrderSide::Sell => {
                            total_sell = total_sell
                                .checked_add(notional)
                                .ok_or(V2ContractError::ArithmeticOverflow)?;
                        }
                    }
                }
            }
            turnover += BigRational::new(
                BigInt::from(day_notional),
                BigInt::from(opening_nav) * BigInt::from(2),
            );
        }
    }

    let first = daily_records
        .first()
        .ok_or(V2ContractError::InvalidLedger)?;
    let last = daily_records.last().ok_or(V2ContractError::InvalidLedger)?;
    let ending_nav_units = last.closing_nav.units().map_err(V2ContractError::from)?;
    let period_return = fraction_to_ratio12(
        BigInt::from(ending_nav_units) - BigInt::from(initial),
        BigInt::from(initial),
    )?;
    let ending_cash = last.closing_cash.clone();

    // The opening record establishes that the first value is measured from
    // initial cash, while later values chain from the previous raw close.
    if first.opening_cash.units().map_err(V2ContractError::from)? != initial {
        return Err(V2ContractError::InvalidLedger);
    }

    Ok(V2PerformanceMetrics {
        daily_returns,
        period_return,
        ending_nav: last.closing_nav.clone(),
        ending_cash,
        ending_holdings: last.closing_positions.clone(),
        cumulative_costs: Money4::from_units(total_cost).map_err(V2ContractError::from)?,
        buy_notional: Money4::from_units(total_buy).map_err(V2ContractError::from)?,
        sell_notional: Money4::from_units(total_sell).map_err(V2ContractError::from)?,
        maximum_drawdown: ratio_to_ratio12(max_drawdown)?,
        period_turnover: ratio_to_ratio12(turnover)?,
        fill_count,
    })
}

fn fraction_to_ratio12(numerator: BigInt, denominator: BigInt) -> Result<Ratio12, V2ContractError> {
    if denominator <= BigInt::zero() {
        return Err(V2ContractError::InvalidLedger);
    }
    ratio_to_ratio12(BigRational::new(numerator, denominator))
}

fn ratio_to_ratio12(value: BigRational) -> Result<Ratio12, V2ContractError> {
    let scaled = value.numer() * BigInt::from(RATIO_SCALE);
    let denominator = value.denom();
    if denominator <= &BigInt::zero() {
        return Err(V2ContractError::InvalidLedger);
    }
    let quotient = &scaled / denominator;
    let remainder = &scaled % denominator;
    let twice_remainder = remainder.abs() * BigInt::from(2);
    let rounded = match twice_remainder.cmp(denominator) {
        std::cmp::Ordering::Less => quotient,
        std::cmp::Ordering::Greater => quotient + rounding_direction(&scaled),
        std::cmp::Ordering::Equal => {
            if (&quotient % BigInt::from(2)).is_zero() {
                quotient
            } else {
                quotient + rounding_direction(&scaled)
            }
        }
    };
    let units = rounded
        .to_i128()
        .ok_or(V2ContractError::ArithmeticOverflow)?;
    Ratio12::from_units(units).map_err(V2ContractError::from)
}

fn rounding_direction(value: &BigInt) -> BigInt {
    if value.is_negative() {
        BigInt::from(-1)
    } else {
        BigInt::from(1)
    }
}
