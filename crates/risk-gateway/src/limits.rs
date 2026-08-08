//! The versioned limits a decision was measured against.
//!
//! Limits are versioned because a decision has to remain explicable after the
//! policy changes. A persisted decision names its `version`; re-deriving it
//! later loads that same row, not whatever the limits happen to be now.

use domain::{Currency, DomainError, FixedPoint, Money};
use serde::{Deserialize, Serialize};

/// One basis point is 1/10000. Weights are compared in integer basis points
/// rather than as a decimal fraction so that the per-symbol check is an exact
/// integer comparison — `0.30` is not representable in binary floating point,
/// and a limit of "30%" that admits 30.000000000000004% is not a limit.
pub const BASIS_POINTS_PER_UNIT: i128 = 10_000;

/// A published, immutable set of risk limits (§6.13 checks 3, 7-10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiskLimits {
    /// Primary key of `risk_limits`; recorded on every decision.
    pub version: String,
    /// Check 7: maximum share of account equity in one instrument, in bp.
    pub max_symbol_weight_bp: u32,
    /// Check 8: maximum value of a single order.
    pub max_order_value: Money,
    /// Check 9: maximum cumulative order value in one day.
    pub max_daily_order_value: Money,
    /// Check 10: maximum loss in one day, as a positive amount.
    pub max_daily_loss: Money,
    /// Check 3 / AT-08: market data older than this blocks the order.
    pub max_data_age_secs: i64,
}

impl RiskLimits {
    /// The currency every limit in this set is denominated in.
    ///
    /// A limit set that mixed currencies could not be compared against an
    /// order value at all, so the constructor refuses to build one.
    pub fn currency(&self) -> Currency {
        self.max_order_value.currency()
    }

    /// Builds a limit set, rejecting the combinations that cannot be enforced.
    ///
    /// The database applies the same rules (`risk_limits_*` CHECK
    /// constraints). Both exist because either alone would let the other's
    /// writer through: the DB cannot stop an in-memory limit set built by a
    /// test double, and the constructor cannot stop a `psql` INSERT.
    pub fn new(
        version: impl Into<String>,
        max_symbol_weight_bp: u32,
        max_order_value: Money,
        max_daily_order_value: Money,
        max_daily_loss: Money,
        max_data_age_secs: i64,
    ) -> Result<Self, LimitsError> {
        let version = version.into();
        if version.is_empty() {
            return Err(LimitsError::EmptyVersion);
        }
        if max_symbol_weight_bp == 0 || i128::from(max_symbol_weight_bp) > BASIS_POINTS_PER_UNIT {
            return Err(LimitsError::WeightOutOfRange {
                bp: max_symbol_weight_bp,
            });
        }
        if max_data_age_secs <= 0 {
            return Err(LimitsError::NonPositiveDataAge {
                secs: max_data_age_secs,
            });
        }
        // A zero maximum would deny every order; that is safe but is a
        // misconfiguration wearing the costume of a policy, so it is refused
        // here rather than silently halting Live trading.
        for (what, value) in [
            ("max_order_value", &max_order_value),
            ("max_daily_order_value", &max_daily_order_value),
            ("max_daily_loss", &max_daily_loss),
        ] {
            if value.is_zero() {
                return Err(LimitsError::ZeroLimit { what });
            }
        }
        let currency = max_order_value.currency();
        for (what, value) in [
            ("max_daily_order_value", &max_daily_order_value),
            ("max_daily_loss", &max_daily_loss),
        ] {
            if value.currency() != currency {
                return Err(LimitsError::MixedCurrency { what });
            }
        }
        Ok(Self {
            version,
            max_symbol_weight_bp,
            max_order_value,
            max_daily_order_value,
            max_daily_loss,
            max_data_age_secs,
        })
    }
}

/// Why a limit set could not be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LimitsError {
    EmptyVersion,
    WeightOutOfRange { bp: u32 },
    NonPositiveDataAge { secs: i64 },
    ZeroLimit { what: &'static str },
    MixedCurrency { what: &'static str },
}

impl std::fmt::Display for LimitsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LimitsError::EmptyVersion => write!(f, "limits version must not be empty"),
            LimitsError::WeightOutOfRange { bp } => {
                write!(f, "max_symbol_weight_bp {bp} is outside (0, 10000]")
            }
            LimitsError::NonPositiveDataAge { secs } => {
                write!(f, "max_data_age_secs {secs} must be positive")
            }
            LimitsError::ZeroLimit { what } => {
                write!(f, "{what} must be positive; zero would deny every order")
            }
            LimitsError::MixedCurrency { what } => {
                write!(f, "{what} is in a different currency to max_order_value")
            }
        }
    }
}

impl std::error::Error for LimitsError {}

/// Multiplies a quantity by a price to get an order value, in exact fixed
/// point.
///
/// Fallible, and deliberately so: an overflow here would otherwise wrap into a
/// small number and slip past the order-value limit, approving an order many
/// times larger than any limit allows. The caller turns the error into
/// `InputUnavailable`, which denies.
pub fn order_value(
    quantity: &FixedPoint,
    price: &FixedPoint,
    currency: Currency,
) -> Result<Money, DomainError> {
    Money::from_fixed(quantity.checked_mul(price)?, currency)
}

/// Whether `part` of `whole` exceeds `limit_bp` basis points.
///
/// Exact integer arithmetic: `part * 10000 > limit_bp * whole`, so nothing is
/// rounded and no division happens. A zero `whole` means an account with no
/// equity, where any position is infinitely concentrated — reported as
/// exceeding, which denies.
pub fn exceeds_basis_points(
    part: &FixedPoint,
    whole: &FixedPoint,
    limit_bp: u32,
) -> Result<bool, DomainError> {
    if whole.is_zero() {
        return Ok(!part.is_zero());
    }
    let scaled_part = part.checked_mul(&FixedPoint::from_i128(BASIS_POINTS_PER_UNIT, 0)?)?;
    let scaled_limit = whole.checked_mul(&FixedPoint::from_i128(i128::from(limit_bp), 0)?)?;
    Ok(scaled_part > scaled_limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn krw(v: &str) -> Money {
        Money::parse(v, Currency::KRW).expect("valid money")
    }

    fn limits() -> RiskLimits {
        RiskLimits::new(
            "v1",
            3_000,
            krw("1000000"),
            krw("5000000"),
            krw("500000"),
            300,
        )
        .expect("valid limits")
    }

    #[test]
    fn a_limit_set_records_its_version_and_currency() {
        let l = limits();
        assert_eq!(l.version, "v1");
        assert_eq!(l.currency(), Currency::KRW);
    }

    #[test]
    fn unenforceable_limit_sets_are_refused() {
        assert_eq!(
            RiskLimits::new("", 3_000, krw("1"), krw("1"), krw("1"), 300),
            Err(LimitsError::EmptyVersion)
        );
        // 0 bp would deny every order; >10000 bp is more than the whole
        // account and so is not a limit at all.
        assert_eq!(
            RiskLimits::new("v", 0, krw("1"), krw("1"), krw("1"), 300),
            Err(LimitsError::WeightOutOfRange { bp: 0 })
        );
        assert_eq!(
            RiskLimits::new("v", 10_001, krw("1"), krw("1"), krw("1"), 300),
            Err(LimitsError::WeightOutOfRange { bp: 10_001 })
        );
        assert_eq!(
            RiskLimits::new("v", 3_000, krw("1"), krw("1"), krw("1"), 0),
            Err(LimitsError::NonPositiveDataAge { secs: 0 })
        );
        assert_eq!(
            RiskLimits::new("v", 3_000, krw("0"), krw("1"), krw("1"), 300),
            Err(LimitsError::ZeroLimit {
                what: "max_order_value"
            })
        );
        // 100% is a legitimate (if permissive) limit and must be accepted.
        assert!(RiskLimits::new("v", 10_000, krw("1"), krw("1"), krw("1"), 300).is_ok());
    }

    #[test]
    fn a_mixed_currency_limit_set_is_refused() {
        let usd = Money::parse("1000", Currency::from_code("USD").unwrap()).unwrap();
        assert_eq!(
            RiskLimits::new("v", 3_000, krw("1000"), usd, krw("1"), 300),
            Err(LimitsError::MixedCurrency {
                what: "max_daily_order_value"
            })
        );
    }

    #[test]
    fn basis_point_comparison_is_exact_at_the_boundary() {
        let whole = FixedPoint::parse("1000000").unwrap();
        // Exactly 30% of 1,000,000 with a 3000bp limit: at the limit, not over
        // it. A float computation of 0.3 * 1_000_000 can land either side of
        // this, which is the entire reason for the integer form.
        let at = FixedPoint::parse("300000").unwrap();
        assert!(!exceeds_basis_points(&at, &whole, 3_000).unwrap());
        // One ten-thousandth of a won over the line still exceeds.
        let over = FixedPoint::parse("300000.0001").unwrap();
        assert!(exceeds_basis_points(&over, &whole, 3_000).unwrap());
        let under = FixedPoint::parse("299999.9999").unwrap();
        assert!(!exceeds_basis_points(&under, &whole, 3_000).unwrap());
    }

    #[test]
    fn an_account_with_no_equity_concentrates_everything() {
        let zero = FixedPoint::parse("0").unwrap();
        let some = FixedPoint::parse("1").unwrap();
        // Any position in a zero-equity account is infinite concentration, so
        // it must exceed rather than divide by zero or pass vacuously.
        assert!(exceeds_basis_points(&some, &zero, 10_000).unwrap());
        // ...but a zero order in a zero-equity account is not a violation.
        assert!(!exceeds_basis_points(&zero, &zero, 10_000).unwrap());
    }

    #[test]
    fn order_value_is_exact_and_overflow_is_an_error_not_a_wrap() {
        let q = FixedPoint::parse("10").unwrap();
        let p = FixedPoint::parse("7250.5").unwrap();
        let v = order_value(&q, &p, Currency::KRW).unwrap();
        assert_eq!(v.as_decimal_string(), "72505.0000");

        // A wrapped multiplication would produce a small value that sails
        // past every limit; it must be an error instead.
        let huge = FixedPoint::from_i128(i128::MAX / 2, 0).unwrap();
        assert!(order_value(&huge, &huge, Currency::KRW).is_err());
    }
}
