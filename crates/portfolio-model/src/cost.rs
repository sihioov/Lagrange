//! Versioned cost profiles (design §9.3-9.4).
//!
//! The standard fee model configures, independently: the buy/sell commission
//! rate, the minimum commission, the sell-side tax, a fixed basis-point
//! slippage, the minimum trade size, and the rebalance weight-difference
//! threshold. Rates are NEVER code constants: they live in a versioned
//! [`CostProfile`] (design: "세율과 수수료는 변경 가능하므로 코드 상수로
//! 고정하지 않고 설정 버전으로 관리한다").
//!
//! Execution-price semantics (design §9.3):
//! - `buy  = open x (1 + slippage_rate)`
//! - `sell = open x (1 - slippage_rate)`
//!
//! The execution layer derives fill prices with [`CostProfile::execution_price`]
//! from the raw open (Todo 10 execution-price basis); the ledger therefore
//! NEVER re-applies slippage (a fill's price already embeds it).
//!
//! [`CostBreakdown`] mirrors the design interface exactly: `commission`, `tax`,
//! `slippage`, and `total = commission + tax + slippage`. Of these, the cash
//! ledger debits/credits `commission + tax`; `slippage` is the informational
//! money-equivalent of the bps already embedded in the execution price (it is
//! never charged twice). The fee-balance identity is asserted by the property
//! suite: for every fill, `cash_after - cash_before == +/-notional -/+ (commission + tax)`
//! exactly, and every fee component is a scale-4 KRW [`Money`].

use domain::{Currency, FixedPoint, Money, PRICE_SCALE, Price, Quantity};
use serde::{Deserialize, Serialize};

use crate::error::PortfolioError;
use crate::side::Side;

/// The version of the shipped `KRX_ETF_DEFAULT` profile.
pub const KRX_ETF_DEFAULT_PROFILE_VERSION: u32 = 1;

/// The identity of a cost profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CostProfileId {
    /// The versioned Korean ETF default (commission 0.015%, min 1,000 KRW,
    /// sell tax 0%, 10 bps slippage, min trade 100,000 KRW, 50 bps threshold).
    KrxEtfDefault,
    /// An operator-supplied profile with explicit settings.
    Custom,
}

/// A versioned set of cost settings (design §9.4).
///
/// The `KRX_ETF_DEFAULT` values below are documented, deterministic
/// placeholders for the operator to confirm; they are config, not constants,
/// and a change is a NEW version (equality includes `version`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostProfile {
    /// The profile identity.
    pub profile_id: CostProfileId,
    /// The settings version (bumps on any change).
    pub version: u32,
    /// Per-side commission rate (e.g. `0.00015` = 0.015%).
    pub commission_rate: FixedPoint,
    /// The minimum commission charged per fill.
    pub min_commission: Money,
    /// Sell-side securities tax rate (`0` for the ETF default).
    pub sell_tax_rate: FixedPoint,
    /// Fixed slippage in basis points (embedded in execution prices).
    pub slippage_bps: u64,
    /// Orders below this trade size are skipped.
    pub min_trade: Money,
    /// |target weight - current weight| below this is not traded.
    pub rebalance_threshold: FixedPoint,
}

impl CostProfile {
    /// The versioned `KRX_ETF_DEFAULT` profile.
    pub fn krx_etf_default() -> Result<Self, PortfolioError> {
        Ok(Self {
            profile_id: CostProfileId::KrxEtfDefault,
            version: KRX_ETF_DEFAULT_PROFILE_VERSION,
            commission_rate: FixedPoint::parse("0.00015")?,
            min_commission: Money::parse("1000", Currency::KRW)?,
            sell_tax_rate: FixedPoint::parse("0")?,
            slippage_bps: 10,
            min_trade: Money::parse("100000", Currency::KRW)?,
            rebalance_threshold: FixedPoint::parse("0.005")?,
        })
    }

    /// A `CUSTOM` profile with explicit settings (KRW).
    ///
    /// Rates and the threshold must lie in `[0, 1]`; slippage in
    /// `[0, 10_000]` bps. Rejects are typed [`PortfolioError`]s.
    pub fn custom(
        commission_rate: &str,
        min_commission: &str,
        sell_tax_rate: &str,
        slippage_bps: u64,
        min_trade: &str,
        rebalance_threshold: &str,
    ) -> Result<Self, PortfolioError> {
        if slippage_bps > 10_000 {
            return Err(PortfolioError::SlippageOutOfRange { bps: slippage_bps });
        }
        Ok(Self {
            profile_id: CostProfileId::Custom,
            version: 1,
            commission_rate: Self::validate_rate("commission_rate", commission_rate)?,
            min_commission: Money::parse(min_commission, Currency::KRW)?,
            sell_tax_rate: Self::validate_rate("sell_tax_rate", sell_tax_rate)?,
            slippage_bps,
            min_trade: Money::parse(min_trade, Currency::KRW)?,
            rebalance_threshold: Self::validate_rate("rebalance_threshold", rebalance_threshold)?,
        })
    }

    /// The slippage as a fixed-point rate (`bps / 10_000`).
    fn slippage_rate(&self) -> FixedPoint {
        FixedPoint::from_i128(i128::from(self.slippage_bps), 4).expect("slippage fits scale 4")
    }

    /// The execution price for a side: `open x (1 +/- slippage_rate)`.
    ///
    /// This is the ONLY place slippage enters pricing; the ledger records
    /// fills at this price and never applies slippage again.
    pub fn execution_price(&self, raw: &Price, side: Side) -> Result<Price, PortfolioError> {
        let slip = self.slippage_rate();
        let one = FixedPoint::parse("1")?;
        let factor = match side {
            Side::Buy => one.checked_add(&slip)?,
            Side::Sell => one.checked_sub(&slip)?,
        };
        let scaled = raw.amount().checked_mul(&factor)?;
        let exec = scaled.with_scale(PRICE_SCALE)?;
        if !exec.is_positive() {
            return Err(PortfolioError::NonPositiveExecutionPrice {
                raw: raw.amount(),
                slippage_bps: self.slippage_bps,
            });
        }
        Ok(Price::from_fixed(exec)?)
    }

    /// The fee breakdown for a fill at an execution price.
    ///
    /// - `commission = max(notional x rate, min_commission)`
    /// - `tax = sell ? notional x sell_tax_rate : 0`
    /// - `slippage` is the informational money-equivalent of the bps already
    ///   inside `price` (never charged separately by the ledger).
    /// - `total = commission + tax + slippage` (design interface).
    ///
    /// A zero quantity is charged zero (defensive; the ledger rejects
    /// zero-quantity events before this is reachable).
    pub fn estimate(
        &self,
        side: Side,
        quantity: &Quantity,
        price: &Price,
    ) -> Result<CostBreakdown, PortfolioError> {
        let currency = self.min_commission.currency();
        if quantity.is_zero() {
            return Ok(CostBreakdown::zero(currency));
        }
        let notional = quantity.checked_mul_price(price, currency)?;
        let commission_raw = notional.checked_mul(&self.commission_rate)?;
        let commission = if commission_raw.amount() < self.min_commission.amount() {
            self.min_commission
        } else {
            commission_raw
        };
        let tax = match side {
            Side::Buy => Money::zero(currency),
            Side::Sell => notional.checked_mul(&self.sell_tax_rate)?,
        };
        let slippage = notional.checked_mul(&self.slippage_rate())?;
        let total = commission.checked_add(&tax)?.checked_add(&slippage)?;
        Ok(CostBreakdown {
            commission,
            tax,
            slippage,
            total,
        })
    }

    fn validate_rate(field: &'static str, value: &str) -> Result<FixedPoint, PortfolioError> {
        let fp = FixedPoint::parse(value)?;
        if fp.is_negative() {
            return Err(PortfolioError::NegativeRate {
                field,
                value: fp.to_string(),
            });
        }
        let max = FixedPoint::parse("1")?;
        if fp > max {
            return Err(PortfolioError::RateOutOfRange {
                field,
                value: fp.to_string(),
            });
        }
        Ok(fp)
    }
}

/// The per-fill fee breakdown (design §9.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostBreakdown {
    /// The commission charged to cash (`>= min_commission` for any fill).
    pub commission: Money,
    /// The sell-side tax charged to cash (zero on buys).
    pub tax: Money,
    /// Informational money-equivalent of the slippage embedded in the
    /// execution price (never charged separately).
    pub slippage: Money,
    /// `commission + tax + slippage` (design interface).
    pub total: Money,
}

impl CostBreakdown {
    /// The zero breakdown in a currency.
    pub fn zero(currency: Currency) -> Self {
        let zero = Money::zero(currency);
        Self {
            commission: zero,
            tax: zero,
            slippage: zero,
            total: zero,
        }
    }

    /// The explicit cash items the ledger debits/credits (`commission + tax`).
    pub fn cash_fees(&self) -> Result<Money, PortfolioError> {
        Ok(self.commission.checked_add(&self.tax)?)
    }
}
