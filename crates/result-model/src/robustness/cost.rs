//! Cost-stress scenarios (FR-ROB-003, AT-04, plan Todo 21).
//!
//! A cost stress re-runs the parent's fills through the deterministic
//! [`replay`] under a stressed [`CostStressProfile`] (commission/tax rates,
//! minimum commission, slippage). Semantics:
//!   - slippage stress is RELATIVE: the parent's fills already embed the
//!     base slippage; the stress moves each execution price by the bps
//!     difference (`buy × (1 + Δbps/10_000)`, `sell × (1 - Δbps/10_000)`);
//!   - fees are recomputed from the stressed execution price:
//!     `commission = max(notional × commission_rate, min_commission)`,
//!     `tax = sell ? notional × sell_tax_rate : 0` (same model as the
//!     versioned `KRX_ETF_DEFAULT` profile of Todo 18; values are config,
//!     never code constants);
//!   - the stressed result MUST pass [`BacktestResult::validate`] — the cash
//!     ledger reconciles with fills + fees, so AT-04's "cost totals match the
//!     trade records" holds by construction.

use domain::{Currency, FixedPoint, Money};
use serde::{Deserialize, Serialize};

use crate::backtest::{BacktestResult, OrderSide};
use crate::robustness::RobustnessError;
use crate::robustness::replay::replay_with;

/// A versioned cost-stress profile (aligned with the Todo 18 `KRX_ETF_DEFAULT`
/// settings; decimal strings for rates, KRW for the minimum commission).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostStressProfile {
    /// The profile identity (e.g. `krx_etf_default`, `stress-2x`).
    pub profile_id: String,
    /// The settings version (any change bumps it).
    pub version: u32,
    /// Per-fill commission rate as a decimal string (e.g. `0.00015`).
    pub commission_rate: String,
    /// Minimum commission charged per fill, KRW decimal string.
    pub min_commission: String,
    /// Sell-side securities tax rate as a decimal string (`0` for the ETF
    /// default).
    pub sell_tax_rate: String,
    /// Slippage in basis points (`[0, 10_000]`).
    pub slippage_bps: u64,
}

impl CostStressProfile {
    /// The documented `KRX_ETF_DEFAULT` settings (Todo 18: 0.015% commission,
    /// 1,000 KRW minimum, no sell tax, 10 bps slippage).
    pub fn krx_etf_default() -> Self {
        Self {
            profile_id: "krx_etf_default".to_owned(),
            version: 1,
            commission_rate: "0.00015".to_owned(),
            min_commission: "1000".to_owned(),
            sell_tax_rate: "0".to_owned(),
            slippage_bps: 10,
        }
    }

    /// A `CUSTOM` profile with explicit settings; rates must lie in
    /// `[0, 1]` and slippage in `[0, 10_000]` bps (typed rejections).
    pub fn custom(
        profile_id: &str,
        version: u32,
        commission_rate: &str,
        min_commission: &str,
        sell_tax_rate: &str,
        slippage_bps: u64,
    ) -> Result<Self, RobustnessError> {
        let validate_rate = |label: &str, raw: &str| -> Result<FixedPoint, RobustnessError> {
            let rate = FixedPoint::parse(raw).map_err(|e| RobustnessError::InvalidCostProfile {
                detail: format!("{label} {raw:?}: {e}"),
            })?;
            let one = FixedPoint::parse("1").expect("constant 1 parses");
            if rate.is_negative() || rate > one {
                return Err(RobustnessError::InvalidCostProfile {
                    detail: format!("{label} must lie in [0, 1], got {raw:?}"),
                });
            }
            Ok(rate)
        };
        validate_rate("commission_rate", commission_rate)?;
        validate_rate("sell_tax_rate", sell_tax_rate)?;
        Money::parse(min_commission, Currency::KRW).map_err(|e| {
            RobustnessError::InvalidCostProfile {
                detail: format!("min_commission {min_commission:?}: {e}"),
            }
        })?;
        if slippage_bps > 10_000 {
            return Err(RobustnessError::InvalidCostProfile {
                detail: format!("slippage_bps {slippage_bps} exceeds 10_000"),
            });
        }
        Ok(Self {
            profile_id: profile_id.to_owned(),
            version,
            commission_rate: commission_rate.to_owned(),
            min_commission: min_commission.to_owned(),
            sell_tax_rate: sell_tax_rate.to_owned(),
            slippage_bps,
        })
    }
}

/// Stresses a parent result under `profile`; `base_slippage_bps` is the
/// slippage already embedded in the parent's fill prices.
pub fn stress_cost(
    result: &BacktestResult,
    profile: &CostStressProfile,
    base_slippage_bps: u64,
) -> Result<BacktestResult, RobustnessError> {
    let commission_rate = FixedPoint::parse(&profile.commission_rate).map_err(|e| {
        RobustnessError::InvalidCostProfile {
            detail: format!("commission_rate: {e}"),
        }
    })?;
    let sell_tax_rate = FixedPoint::parse(&profile.sell_tax_rate).map_err(|e| {
        RobustnessError::InvalidCostProfile {
            detail: format!("sell_tax_rate: {e}"),
        }
    })?;
    let min_commission =
        Money::parse(&profile.min_commission, result.summary.currency).map_err(|e| {
            RobustnessError::InvalidCostProfile {
                detail: format!("min_commission: {e}"),
            }
        })?;
    let delta = FixedPoint::from_i128(
        i128::from(profile.slippage_bps) - i128::from(base_slippage_bps),
        4,
    )
    .map_err(|e| RobustnessError::InvalidCostProfile {
        detail: format!("slippage delta: {e}"),
    })?;

    replay_with(
        result,
        |fill| {
            let mut adjusted = fill.clone();
            let factor = match fill.side {
                OrderSide::Buy => FixedPoint::parse("1")
                    .and_then(|one| one.checked_add(&delta))
                    .expect("buy factor fits"),
                OrderSide::Sell => FixedPoint::parse("1")
                    .and_then(|one| one.checked_sub(&delta))
                    .expect("sell factor fits"),
            };
            let scaled = fill
                .price
                .amount()
                .checked_mul(&factor)
                .expect("price scaling fits");
            adjusted.price = domain::Price::from_fixed(scaled).expect("stressed price is positive");
            adjusted
        },
        |fill| {
            let notional = fill
                .quantity
                .checked_mul_price(&fill.price, result.summary.currency)
                .expect("notional fits");
            let commission_raw = notional
                .checked_mul(&commission_rate)
                .expect("commission fits");
            let commission = if commission_raw.amount() < min_commission.amount() {
                min_commission
            } else {
                commission_raw
            };
            let tax = match fill.side {
                OrderSide::Buy => Money::zero(result.summary.currency),
                OrderSide::Sell => notional.checked_mul(&sell_tax_rate).expect("tax fits"),
            };
            (commission, tax)
        },
    )
}
