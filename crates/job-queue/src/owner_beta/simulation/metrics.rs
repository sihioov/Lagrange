//! Exact, reusable performance metric derivation for simulation ledgers.
//!
//! Values stay as integer money units and rational fractions until the final
//! 12-place ties-to-even serialization step. No displayed ratio is used as an
//! input to another calculation.

use std::collections::BTreeMap;

use domain::TradingDate;
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{Signed, ToPrimitive, Zero};

use super::contract::{LedgerPhase, LedgerPhaseRecord, Money4, Ratio12, SimulationContractError};

const RATIO_SCALE: i128 = 1_000_000_000_000;

/// Derived values for one complete, ordered evaluation ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedPerformanceMetrics {
    daily_returns: Vec<(TradingDate, Ratio12)>,
    period_return: Ratio12,
    maximum_drawdown: Ratio12,
    period_turnover: Ratio12,
}

impl DerivedPerformanceMetrics {
    /// Daily close returns in evaluation-session order.
    pub fn daily_returns(&self) -> &[(TradingDate, Ratio12)] {
        &self.daily_returns
    }

    /// Return from initial cash to the last evaluation close.
    pub fn period_return(&self) -> &Ratio12 {
        &self.period_return
    }

    /// Maximum close-to-close drawdown including initial cash as the first peak.
    pub fn maximum_drawdown(&self) -> &Ratio12 {
        &self.maximum_drawdown
    }

    /// Sum of daily two-sided turnover fractions, rounded once at the end.
    pub fn period_turnover(&self) -> &Ratio12 {
        &self.period_turnover
    }
}

/// Derives all ledger-based ratios from their original money values.
///
/// The records must contain one positive `PRE_OPEN_MARK` and `CLOSE_MARK` per
/// evaluation date, with at least two dates. Trade notionals are counted from
/// `BUY` and `SELL` events; fees are intentionally excluded from turnover.
pub fn derive_performance_metrics(
    initial_cash: &Money4,
    records: &[LedgerPhaseRecord],
) -> Result<DerivedPerformanceMetrics, SimulationContractError> {
    let initial_units = initial_cash.units()?;
    if initial_units <= 0 {
        return Err(SimulationContractError::InvalidLedger);
    }

    let mut opening_navs = BTreeMap::new();
    let mut closing_navs = BTreeMap::new();
    let mut daily_notionals: BTreeMap<TradingDate, BigInt> = BTreeMap::new();
    let mut previous_record_date = None;
    for record in records {
        if previous_record_date.is_some_and(|previous| record.session_date() < previous) {
            return Err(SimulationContractError::InvalidLedger);
        }
        previous_record_date = Some(record.session_date());
        match record.phase() {
            LedgerPhase::PreOpenMark => {
                let nav = record.nav().units()?;
                if nav <= 0 || opening_navs.insert(record.session_date(), nav).is_some() {
                    return Err(SimulationContractError::InvalidLedger);
                }
            }
            LedgerPhase::CloseMark => {
                let nav = record.nav().units()?;
                if nav <= 0 || closing_navs.insert(record.session_date(), nav).is_some() {
                    return Err(SimulationContractError::InvalidLedger);
                }
            }
            LedgerPhase::Buy | LedgerPhase::Sell => {
                let notional = record.events().iter().try_fold(
                    BigInt::zero(),
                    |sum, event| -> Result<BigInt, SimulationContractError> {
                        let units = event.notional().units()?;
                        if units < 0 {
                            return Err(SimulationContractError::InvalidLedger);
                        }
                        Ok(sum + BigInt::from(units))
                    },
                )?;
                *daily_notionals
                    .entry(record.session_date())
                    .or_insert_with(BigInt::zero) += notional;
            }
            LedgerPhase::InitialCash | LedgerPhase::SignalForNextSession => {}
        }
    }

    if closing_navs.len() < 2 || opening_navs.len() != closing_navs.len() {
        return Err(SimulationContractError::InvalidLedger);
    }
    if opening_navs.keys().ne(closing_navs.keys())
        || daily_notionals
            .keys()
            .any(|date| !closing_navs.contains_key(date))
    {
        return Err(SimulationContractError::InvalidLedger);
    }

    let mut daily_returns = Vec::with_capacity(closing_navs.len());
    let mut previous_close = initial_units;
    let mut high_watermark = initial_units;
    let mut maximum_drawdown = BigRational::zero();
    for (date, close_units) in &closing_navs {
        if previous_close <= 0 || *close_units < 0 {
            return Err(SimulationContractError::InvalidLedger);
        }
        let daily_numerator = BigInt::from(*close_units) - BigInt::from(previous_close);
        let daily_ratio = fraction_to_ratio12(daily_numerator, BigInt::from(previous_close))?;
        daily_returns.push((*date, daily_ratio));

        if *close_units > high_watermark {
            high_watermark = *close_units;
        }
        let drawdown = BigRational::new(
            BigInt::from(high_watermark - *close_units),
            BigInt::from(high_watermark),
        );
        if drawdown > maximum_drawdown {
            maximum_drawdown = drawdown;
        }
        previous_close = *close_units;
    }

    let ending_nav = *closing_navs
        .last_key_value()
        .ok_or(SimulationContractError::InvalidLedger)?
        .1;
    let period_return = fraction_to_ratio12(
        BigInt::from(ending_nav) - BigInt::from(initial_units),
        BigInt::from(initial_units),
    )?;

    let turnover_terms = closing_navs.keys().map(
        |date| -> Result<(BigInt, BigInt), SimulationContractError> {
            let open_nav = *opening_navs
                .get(date)
                .ok_or(SimulationContractError::InvalidLedger)?;
            if open_nav <= 0 {
                return Err(SimulationContractError::InvalidLedger);
            }
            let notional = daily_notionals.get(date).cloned().unwrap_or_default();
            Ok((notional, BigInt::from(open_nav) * BigInt::from(2)))
        },
    );
    let period_turnover = ratio_to_ratio12(sum_turnover_fractions(turnover_terms)?)?;

    Ok(DerivedPerformanceMetrics {
        daily_returns,
        period_return,
        maximum_drawdown: ratio_to_ratio12(maximum_drawdown)?,
        period_turnover,
    })
}

/// Derives strategy-minus-benchmark return from original ending NAVs.
///
/// The shared initial cash is the positive denominator. The difference is
/// rounded once, rather than subtracting the two displayed period returns.
pub fn derive_benchmark_return_difference(
    initial_cash: &Money4,
    strategy_ending_nav: &Money4,
    benchmark_ending_nav: &Money4,
) -> Result<Ratio12, SimulationContractError> {
    let initial = initial_cash.units()?;
    if initial <= 0 {
        return Err(SimulationContractError::InvalidLedger);
    }
    let numerator =
        BigInt::from(strategy_ending_nav.units()?) - BigInt::from(benchmark_ending_nav.units()?);
    fraction_to_ratio12(numerator, BigInt::from(initial))
}

fn fraction_to_ratio12(
    numerator: BigInt,
    denominator: BigInt,
) -> Result<Ratio12, SimulationContractError> {
    if denominator <= BigInt::zero() {
        return Err(SimulationContractError::InvalidLedger);
    }
    ratio_to_ratio12(BigRational::new(numerator, denominator))
}

fn ratio_to_ratio12(value: BigRational) -> Result<Ratio12, SimulationContractError> {
    let scaled = value.numer() * BigInt::from(RATIO_SCALE);
    let denominator = value.denom();
    if denominator <= &BigInt::zero() {
        return Err(SimulationContractError::InvalidLedger);
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
        .ok_or(SimulationContractError::ArithmeticOverflow)?;
    Ratio12::from_units(units)
}

fn rounding_direction(value: &BigInt) -> BigInt {
    if value.is_negative() {
        BigInt::from(-1)
    } else {
        BigInt::from(1)
    }
}

fn sum_turnover_fractions<I>(terms: I) -> Result<BigRational, SimulationContractError>
where
    I: IntoIterator<Item = Result<(BigInt, BigInt), SimulationContractError>>,
{
    terms
        .into_iter()
        .try_fold(BigRational::zero(), |total, term| {
            let (notional, denominator) = term?;
            if notional < BigInt::zero() || denominator <= BigInt::zero() {
                return Err(SimulationContractError::InvalidLedger);
            }
            Ok(total + BigRational::new(notional, denominator))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rounded(numerator: i128, denominator: i128) -> Ratio12 {
        fraction_to_ratio12(BigInt::from(numerator), BigInt::from(denominator)).unwrap()
    }

    #[test]
    fn rounds_positive_and_negative_halfway_values_to_even() {
        let half_scale = RATIO_SCALE * 2;
        assert_eq!(rounded(1, half_scale), Ratio12::zero());
        assert_eq!(rounded(3, half_scale).as_str(), "0.000000000002");
        assert_eq!(rounded(5, half_scale).as_str(), "0.000000000002");
        assert_eq!(rounded(7, half_scale).as_str(), "0.000000000004");
        assert_eq!(rounded(-1, half_scale), Ratio12::zero());
        assert_eq!(rounded(-3, half_scale).as_str(), "-0.000000000002");
        assert_eq!(rounded(-5, half_scale).as_str(), "-0.000000000002");
        assert_eq!(rounded(-7, half_scale).as_str(), "-0.000000000004");
    }

    #[test]
    fn sums_long_turnover_series_before_rounding() {
        let terms = (0..10_000)
            .map(|_| Ok::<_, SimulationContractError>((BigInt::from(1), BigInt::from(3))));
        let exact = sum_turnover_fractions(terms).unwrap();
        assert_eq!(
            ratio_to_ratio12(exact).unwrap().as_str(),
            "3333.333333333333"
        );
    }

    #[test]
    fn benchmark_difference_rounds_the_exact_nav_fraction_once() {
        let initial = Money4::from_units(100 * RATIO_SCALE).unwrap();
        let strategy = Money4::from_units(100 * RATIO_SCALE + 49).unwrap();
        let benchmark = Money4::from_units(100 * RATIO_SCALE - 49).unwrap();
        assert_eq!(
            fraction_to_ratio12(BigInt::from(49), BigInt::from(initial.units().unwrap())).unwrap(),
            Ratio12::zero()
        );
        assert_eq!(
            fraction_to_ratio12(BigInt::from(-49), BigInt::from(initial.units().unwrap())).unwrap(),
            Ratio12::zero()
        );
        assert_eq!(
            derive_benchmark_return_difference(&initial, &strategy, &benchmark)
                .unwrap()
                .as_str(),
            "0.000000000001"
        );
        assert_eq!(
            derive_benchmark_return_difference(&initial, &benchmark, &strategy)
                .unwrap()
                .as_str(),
            "-0.000000000001"
        );
    }

    #[test]
    fn rejects_non_positive_denominators_and_unrepresentable_ratios() {
        assert_eq!(
            fraction_to_ratio12(BigInt::from(1), BigInt::zero()),
            Err(SimulationContractError::InvalidLedger)
        );
        let too_large = BigInt::from(i128::MAX) * BigInt::from(RATIO_SCALE);
        assert_eq!(
            ratio_to_ratio12(BigRational::from_integer(too_large)),
            Err(SimulationContractError::ArithmeticOverflow)
        );
    }
}
