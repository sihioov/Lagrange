//! Fixed-point decimal primitives: the [`FixedPoint`] core plus the branded
//! financial wrappers [`Money`], [`Price`], [`Quantity`], and [`Weight`].
//!
//! JSON rule (plan Todo 2): decimal values cross JSON boundaries as STRINGS at
//! a per-type canonical scale, so serialization round-trips byte-equivalently
//! after canonicalization. No float exists anywhere on a monetary path;
//! [`FixedPoint::to_f64`] exists only for reported statistics.

use std::fmt;
use std::str::FromStr;

use serde::de::Error as DeError;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::currency::Currency;
use crate::error::DomainError;

/// Maximum supported decimal scale (digits after the point) for [`FixedPoint`].
pub const MAX_SCALE: u8 = 28;
/// Canonical scale for [`Price`]: four decimal places.
pub const PRICE_SCALE: u8 = 4;
/// Canonical scale for [`Money`]: four decimal places.
pub const MONEY_SCALE: u8 = 4;
/// Canonical scale for [`Quantity`]: integer units only.
pub const QUANTITY_SCALE: u8 = 0;
/// Canonical scale for [`Weight`]: six decimal places.
pub const WEIGHT_SCALE: u8 = 6;

/// `10^n` for `n <= MAX_SCALE`; always fits `i128` (10^28 < i128::MAX).
fn pow10(n: u8) -> i128 {
    debug_assert!(n <= MAX_SCALE);
    10i128.pow(u32::from(n))
}

fn pow10_checked(n: u8) -> Option<i128> {
    10i128.checked_pow(u32::from(n))
}

/// Round `num / den` to the nearest integer, ties to even (banker's rounding).
fn round_div(num: i128, den: i128) -> i128 {
    debug_assert!(den != 0);
    let q = num / den;
    let r = num % den;
    let abs_r = r.unsigned_abs();
    let den_abs = den.unsigned_abs();
    let twice = abs_r.saturating_mul(2);
    let sign = if (num < 0) != (den < 0) { -1 } else { 1 };
    if twice > den_abs {
        q + sign
    } else if twice == den_abs && q % 2 != 0 {
        q + sign
    } else {
        q
    }
}

/// Fixed-point decimal: an unscaled signed 128-bit mantissa plus a scale.
///
/// Equality/ordering/hashing are VALUE-based across scales (1.0 at scale 1
/// equals 1.00 at scale 2). Arithmetic is checked and returns typed
/// [`DomainError`]s instead of panicking or wrapping.
#[derive(Debug, Clone, Copy)]
pub struct FixedPoint {
    bits: i128,
    scale: u8,
}

impl FixedPoint {
    /// The exact value zero at scale 0.
    pub const ZERO: Self = Self { bits: 0, scale: 0 };

    /// Constructs a value from an unscaled mantissa and a scale.
    pub fn from_i128(bits: i128, scale: u8) -> Result<Self, DomainError> {
        if scale > MAX_SCALE {
            return Err(DomainError::ScaleExceedsMax {
                scale,
                max: MAX_SCALE,
            });
        }
        Ok(Self { bits, scale })
    }

    /// Parses a decimal string (optional sign, integer and fraction parts).
    ///
    /// Accepted forms: `123`, `-123`, `123.45`, `-123.45`, `.5`, `5.`. The
    /// result keeps the string's own scale (no canonicalization at this layer).
    pub fn parse(s: &str) -> Result<Self, DomainError> {
        let s = s.trim();
        if s.is_empty() {
            return Err(DomainError::InvalidDecimalString {
                value: s.to_owned(),
            });
        }
        let (negative, digits) = match s.as_bytes()[0] {
            b'+' => (false, &s[1..]),
            b'-' => (true, &s[1..]),
            _ => (false, s),
        };
        let mut parts = digits.split('.');
        let int_part = parts.next().unwrap_or("");
        let frac_part = parts.next().unwrap_or("");
        if parts.next().is_some() {
            return Err(DomainError::InvalidDecimalString {
                value: s.to_owned(),
            });
        }
        if int_part.is_empty() && frac_part.is_empty() {
            return Err(DomainError::InvalidDecimalString {
                value: s.to_owned(),
            });
        }
        let valid = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
        if !int_part.is_empty() && !valid(int_part) {
            return Err(DomainError::InvalidDecimalString {
                value: s.to_owned(),
            });
        }
        if !frac_part.is_empty() && !valid(frac_part) {
            return Err(DomainError::InvalidDecimalString {
                value: s.to_owned(),
            });
        }
        let scale = u8::try_from(frac_part.len())
            .map_err(|_| DomainError::ScaleExceedsMax {
                scale: 255,
                max: MAX_SCALE,
            })?;
        if scale > MAX_SCALE {
            return Err(DomainError::ScaleExceedsMax {
                scale,
                max: MAX_SCALE,
            });
        }
        let mut bits: i128 = 0;
        for ch in int_part.bytes().chain(frac_part.bytes()) {
            let digit = i128::from(ch - b'0');
            bits = bits
                .checked_mul(10)
                .and_then(|b| b.checked_add(digit))
                .ok_or_else(|| DomainError::Overflow {
                    operation: "decimal string parse".to_owned(),
                })?;
        }
        if negative {
            bits = -bits;
        }
        Ok(Self { bits, scale })
    }

    /// The unscaled signed mantissa (`bits * 10^-scale`).
    pub fn bits(&self) -> i128 {
        self.bits
    }

    /// The decimal scale (digits after the point).
    pub fn scale(&self) -> u8 {
        self.scale
    }

    /// Whether the value is an exact whole number.
    pub fn is_integer(&self) -> bool {
        self.bits % pow10(self.scale) == 0
    }

    /// Rescales to `new_scale`, rounding half-to-even when reducing precision
    /// and extending exactly (with overflow check) when increasing it.
    pub fn with_scale(&self, new_scale: u8) -> Result<Self, DomainError> {
        if new_scale > MAX_SCALE {
            return Err(DomainError::ScaleExceedsMax {
                scale: new_scale,
                max: MAX_SCALE,
            });
        }
        if new_scale == self.scale {
            return Ok(*self);
        }
        if new_scale > self.scale {
            let factor = pow10(new_scale - self.scale);
            let bits = self.bits.checked_mul(factor).ok_or_else(|| {
                DomainError::Overflow {
                    operation: "scale increase".to_owned(),
                }
            })?;
            return Ok(Self {
                bits,
                scale: new_scale,
            });
        }
        let diff = self.scale - new_scale;
        let factor = pow10(diff);
        let q = self.bits / factor;
        let r = self.bits % factor;
        let abs_r = r.unsigned_abs();
        let half = factor.unsigned_abs() / 2;
        let sign = if self.bits < 0 { -1 } else { 1 };
        let bits = if abs_r > half {
            q + sign
        } else if abs_r == half && q % 2 != 0 {
            q + sign
        } else {
            q
        };
        Ok(Self {
            bits,
            scale: new_scale,
        })
    }

    /// Checked addition (operands aligned to the larger scale).
    pub fn checked_add(&self, other: &Self) -> Result<Self, DomainError> {
        let scale = self.scale.max(other.scale);
        let a = self.to_bits_at(scale).ok_or_else(|| DomainError::Overflow {
            operation: "addition scale alignment".to_owned(),
        })?;
        let b = other
            .to_bits_at(scale)
            .ok_or_else(|| DomainError::Overflow {
                operation: "addition scale alignment".to_owned(),
            })?;
        let bits = a.checked_add(b).ok_or_else(|| DomainError::Overflow {
            operation: "addition".to_owned(),
        })?;
        Ok(Self { bits, scale })
    }

    /// Checked subtraction (operands aligned to the larger scale).
    pub fn checked_sub(&self, other: &Self) -> Result<Self, DomainError> {
        let scale = self.scale.max(other.scale);
        let a = self.to_bits_at(scale).ok_or_else(|| DomainError::Overflow {
            operation: "subtraction scale alignment".to_owned(),
        })?;
        let b = other
            .to_bits_at(scale)
            .ok_or_else(|| DomainError::Overflow {
                operation: "subtraction scale alignment".to_owned(),
            })?;
        let bits = a.checked_sub(b).ok_or_else(|| DomainError::Overflow {
            operation: "subtraction".to_owned(),
        })?;
        Ok(Self { bits, scale })
    }

    /// Checked multiplication (result scale = sum of operand scales).
    pub fn checked_mul(&self, other: &Self) -> Result<Self, DomainError> {
        let scale = self
            .scale
            .checked_add(other.scale)
            .filter(|s| *s <= MAX_SCALE)
            .ok_or_else(|| DomainError::ScaleExceedsMax {
                scale: self.scale.saturating_add(other.scale),
                max: MAX_SCALE,
            })?;
        let bits = self
            .bits
            .checked_mul(other.bits)
            .ok_or_else(|| DomainError::Overflow {
                operation: "multiplication".to_owned(),
            })?;
        Ok(Self { bits, scale })
    }

    /// Checked division rounded to `result_scale` (ties to even).
    pub fn checked_div(&self, other: &Self, result_scale: u8) -> Result<Self, DomainError> {
        if result_scale > MAX_SCALE {
            return Err(DomainError::ScaleExceedsMax {
                scale: result_scale,
                max: MAX_SCALE,
            });
        }
        if other.bits == 0 {
            return Err(DomainError::DivisionByZero);
        }
        let k = i64::from(result_scale) + i64::from(other.scale) - i64::from(self.scale);
        let (num, den) = if k >= 0 {
            let scale_up = u8::try_from(k).map_err(|_| DomainError::Overflow {
                operation: "division scale alignment".to_owned(),
            })?;
            let num = self.bits.checked_mul(pow10_checked(scale_up).ok_or_else(|| {
                DomainError::Overflow {
                    operation: "division scale alignment".to_owned(),
                }
            })?).ok_or_else(|| DomainError::Overflow {
                operation: "division scale alignment".to_owned(),
            })?;
            (num, other.bits)
        } else {
            let scale_down = u8::try_from(-k).map_err(|_| DomainError::Overflow {
                operation: "division scale alignment".to_owned(),
            })?;
            let den = other
                .bits
                .checked_mul(pow10_checked(scale_down).ok_or_else(|| {
                    DomainError::Overflow {
                        operation: "division scale alignment".to_owned(),
                    }
                })?)
                .ok_or_else(|| DomainError::Overflow {
                    operation: "division scale alignment".to_owned(),
                })?;
            (self.bits, den)
        };
        Ok(Self {
            bits: round_div(num, den),
            scale: result_scale,
        })
    }

    /// Checked negation (i128::MIN is not representable).
    pub fn checked_neg(&self) -> Result<Self, DomainError> {
        let bits = self.bits.checked_neg().ok_or_else(|| DomainError::Overflow {
            operation: "negation".to_owned(),
        })?;
        Ok(Self {
            bits,
            scale: self.scale,
        })
    }

    /// Absolute value (saturates the unrepresentable i128::MIN).
    pub fn abs(&self) -> Self {
        Self {
            bits: self.bits.checked_abs().unwrap_or(i128::MAX),
            scale: self.scale,
        }
    }

    /// Whether the value is exactly zero.
    pub fn is_zero(&self) -> bool {
        self.bits == 0
    }

    /// Whether the value is strictly positive.
    pub fn is_positive(&self) -> bool {
        self.bits > 0
    }

    /// Whether the value is strictly negative.
    pub fn is_negative(&self) -> bool {
        self.bits < 0
    }

    /// Sign of the value (-1, 0, or 1).
    pub fn signum(&self) -> i8 {
        if self.bits > 0 {
            1
        } else if self.bits < 0 {
            -1
        } else {
            0
        }
    }

    /// Conversion to `f64` — REPORTED STATISTICS ONLY, never on a monetary
    /// path. Check [`f64::is_finite`] at the boundary.
    pub fn to_f64(&self) -> f64 {
        self.bits as f64 / 10f64.powi(i32::from(self.scale))
    }

    fn to_bits_at(&self, target_scale: u8) -> Option<i128> {
        debug_assert!(target_scale >= self.scale);
        self.bits.checked_mul(pow10(target_scale - self.scale))
    }

    /// The value's reduced (trailing-zero-free) mantissa and scale.
    fn normalized_parts(&self) -> (i128, u8) {
        let mut bits = self.bits;
        let mut scale = self.scale;
        while scale > 0 && bits % 10 == 0 {
            bits /= 10;
            scale -= 1;
        }
        (bits, scale)
    }
}

impl PartialEq for FixedPoint {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
}

impl Eq for FixedPoint {}

impl PartialOrd for FixedPoint {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FixedPoint {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let target = self.scale.max(other.scale);
        match (self.to_bits_at(target), other.to_bits_at(target)) {
            (Some(a), Some(b)) => a.cmp(&b),
            // Only reachable with mantissas near i128::MAX and differing
            // scales; fall back to a deterministic float comparison.
            _ => {
                let a = self.bits as f64 / 10f64.powi(i32::from(self.scale));
                let b = other.bits as f64 / 10f64.powi(i32::from(other.scale));
                a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
            }
        }
    }
}

impl std::hash::Hash for FixedPoint {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let (bits, scale) = self.normalized_parts();
        bits.hash(state);
        scale.hash(state);
    }
}

impl fmt::Display for FixedPoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let scale = usize::from(self.scale);
        if scale == 0 {
            return write!(f, "{}", self.bits);
        }
        let negative = self.bits < 0;
        let abs = self.bits.unsigned_abs();
        let factor = 10u128.pow(u32::from(self.scale));
        let int_part = abs / factor;
        let frac_part = abs % factor;
        if negative {
            f.write_str("-")?;
        }
        write!(f, "{int_part}.{frac_part:0width$}", width = scale)
    }
}

impl FromStr for FixedPoint {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Serialize for FixedPoint {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for FixedPoint {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).map_err(DeError::custom)
    }
}

/// Validator for [`Price`]: strictly positive.
fn validate_price(value: &FixedPoint) -> Result<(), DomainError> {
    if !value.is_positive() {
        return Err(DomainError::NonPositivePrice {
            value: value.to_string(),
        });
    }
    Ok(())
}

/// Validator for [`Quantity`]: non-negative whole units.
fn validate_quantity(value: &FixedPoint) -> Result<(), DomainError> {
    if value.is_negative() {
        return Err(DomainError::NegativeQuantity {
            value: value.to_string(),
        });
    }
    if !value.is_integer() {
        return Err(DomainError::FractionalQuantity {
            value: value.to_string(),
        });
    }
    Ok(())
}

/// Validator for [`Weight`]: closed interval [0, 1].
fn validate_weight(value: &FixedPoint) -> Result<(), DomainError> {
    let max = FixedPoint::from_i128(1_000_000, WEIGHT_SCALE).expect("weight max fits");
    if value.is_negative() || *value > max {
        return Err(DomainError::WeightOutOfRange {
            value: value.to_string(),
        });
    }
    Ok(())
}

macro_rules! fixed_wrapper {
    ($(#[$doc:meta])* $name:ident, $scale:expr, $validate:path) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name {
            inner: FixedPoint,
        }

        impl $name {
            /// Parses a decimal string at the canonical scale (ties to even).
            pub fn parse(s: &str) -> Result<Self, DomainError> {
                let raw = FixedPoint::parse(s)?;
                $validate(&raw)?;
                let value = raw.with_scale($scale)?;
                $validate(&value)?;
                Ok(Self { inner: value })
            }

            /// Wraps a fixed-point value, rescaling and validating it.
            pub fn from_fixed(value: FixedPoint) -> Result<Self, DomainError> {
                $validate(&value)?;
                let value = value.with_scale($scale)?;
                $validate(&value)?;
                Ok(Self { inner: value })
            }

            /// The canonical fixed-point amount.
            pub fn amount(&self) -> FixedPoint {
                self.inner
            }

            /// Canonical decimal string (always at the canonical scale).
            pub fn as_decimal_string(&self) -> String {
                self.inner.to_string()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.inner)
            }
        }

        impl FromStr for $name {
            type Err = DomainError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::parse(s)
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.as_decimal_string())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let s = String::deserialize(deserializer)?;
                Self::parse(&s).map_err(DeError::custom)
            }
        }
    };
}

fixed_wrapper! {
    /// A strictly positive price, canonical scale [`PRICE_SCALE`] (4 dp).
    Price,
    PRICE_SCALE,
    validate_price
}

impl Price {
    /// Checked addition (result stays strictly positive).
    pub fn checked_add(&self, other: &Self) -> Result<Self, DomainError> {
        Self::from_fixed(self.inner.checked_add(&other.inner)?)
    }

    /// Checked subtraction (result must stay strictly positive).
    pub fn checked_sub(&self, other: &Self) -> Result<Self, DomainError> {
        Self::from_fixed(self.inner.checked_sub(&other.inner)?)
    }

    /// Price scaled by a ratio (e.g. slippage `open * (1 + bp)`).
    pub fn checked_mul_ratio(&self, factor: &FixedPoint) -> Result<Self, DomainError> {
        Self::from_fixed(self.inner.checked_mul(factor)?)
    }

    /// Price divided by a ratio, rounded to the canonical scale.
    pub fn checked_div_ratio(&self, factor: &FixedPoint) -> Result<Self, DomainError> {
        Self::from_fixed(self.inner.checked_div(factor, PRICE_SCALE)?)
    }

    /// `price * quantity` as an unscaled fixed-point amount (scale 4).
    pub fn checked_mul_quantity(&self, quantity: &Quantity) -> Result<FixedPoint, DomainError> {
        self.inner.checked_mul(&quantity.inner)
    }
}

fixed_wrapper! {
    /// A non-negative whole-unit quantity, canonical scale [`QUANTITY_SCALE`] (0 dp).
    Quantity,
    QUANTITY_SCALE,
    validate_quantity
}

impl Quantity {
    /// The zero quantity.
    pub fn zero() -> Result<Self, DomainError> {
        Self::parse("0")
    }

    /// Whether the quantity is zero.
    pub fn is_zero(&self) -> bool {
        self.inner.is_zero()
    }

    /// Checked addition.
    pub fn checked_add(&self, other: &Self) -> Result<Self, DomainError> {
        Self::from_fixed(self.inner.checked_add(&other.inner)?)
    }

    /// Checked subtraction (negative result is rejected as a typed error).
    pub fn checked_sub(&self, other: &Self) -> Result<Self, DomainError> {
        Self::from_fixed(self.inner.checked_sub(&other.inner)?)
    }

    /// `quantity * price` in the given currency (`Money` is currency-aware).
    pub fn checked_mul_price(&self, price: &Price, currency: Currency) -> Result<Money, DomainError> {
        let amount = self.inner.checked_mul(&price.inner)?;
        Money::from_fixed(amount, currency)
    }

    /// The whole-unit count as `u64` (for lot sizing).
    pub fn to_u64(&self) -> Result<u64, DomainError> {
        u64::try_from(self.inner.bits()).map_err(|_| DomainError::Overflow {
            operation: "quantity to u64".to_owned(),
        })
    }
}

fixed_wrapper! {
    /// A target weight in the closed interval [0, 1], canonical scale [`WEIGHT_SCALE`] (6 dp).
    Weight,
    WEIGHT_SCALE,
    validate_weight
}

impl Weight {
    /// The zero weight.
    pub fn zero() -> Result<Self, DomainError> {
        Self::parse("0")
    }

    /// The full (100%) weight.
    pub fn one() -> Result<Self, DomainError> {
        Self::parse("1")
    }

    /// Whether the weight is exactly zero.
    pub fn is_zero(&self) -> bool {
        self.inner.is_zero()
    }

    /// Checked addition (result must stay within [0, 1]).
    pub fn checked_add(&self, other: &Self) -> Result<Self, DomainError> {
        Self::from_fixed(self.inner.checked_add(&other.inner)?)
    }

    /// Checked subtraction (result must stay within [0, 1]).
    pub fn checked_sub(&self, other: &Self) -> Result<Self, DomainError> {
        Self::from_fixed(self.inner.checked_sub(&other.inner)?)
    }
}

/// A non-negative, currency-aware monetary amount at canonical scale
/// [`MONEY_SCALE`] (4 dp). Serializes as `{"amount":"<string>","currency":"<code>"}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Money {
    amount: FixedPoint,
    currency: Currency,
}

impl Money {
    /// Parses an amount string in the given currency (never negative).
    pub fn parse(amount: &str, currency: Currency) -> Result<Self, DomainError> {
        let raw = FixedPoint::parse(amount)?;
        if raw.is_negative() {
            return Err(DomainError::NegativeMoney {
                amount: raw.to_string(),
            });
        }
        let value = raw.with_scale(MONEY_SCALE)?;
        if value.is_negative() {
            return Err(DomainError::NegativeMoney {
                amount: value.to_string(),
            });
        }
        Ok(Self { amount: value, currency })
    }

    /// Wraps a fixed-point amount in a currency, rescaling and validating.
    pub fn from_fixed(amount: FixedPoint, currency: Currency) -> Result<Self, DomainError> {
        if amount.is_negative() {
            return Err(DomainError::NegativeMoney {
                amount: amount.to_string(),
            });
        }
        let amount = amount.with_scale(MONEY_SCALE)?;
        if amount.is_negative() {
            return Err(DomainError::NegativeMoney {
                amount: amount.to_string(),
            });
        }
        Ok(Self { amount, currency })
    }

    /// The zero amount in the given currency.
    pub fn zero(currency: Currency) -> Self {
        Self {
            amount: FixedPoint {
                bits: 0,
                scale: MONEY_SCALE,
            },
            currency,
        }
    }

    /// The canonical fixed-point amount.
    pub fn amount(&self) -> FixedPoint {
        self.amount
    }

    /// The currency of this amount.
    pub fn currency(&self) -> Currency {
        self.currency
    }

    /// Whether the amount is exactly zero.
    pub fn is_zero(&self) -> bool {
        self.amount.is_zero()
    }

    /// Canonical decimal string (always at the canonical scale).
    pub fn as_decimal_string(&self) -> String {
        self.amount.to_string()
    }

    /// Checked addition; rejects a currency mismatch as a typed error.
    pub fn checked_add(&self, other: &Self) -> Result<Self, DomainError> {
        self.ensure_same_currency(other)?;
        let amount = self.amount.checked_add(&other.amount)?;
        Ok(Self { amount, currency: self.currency })
    }

    /// Checked subtraction; rejects currency mismatch and negative cash.
    pub fn checked_sub(&self, other: &Self) -> Result<Self, DomainError> {
        self.ensure_same_currency(other)?;
        let amount = self.amount.checked_sub(&other.amount)?;
        if amount.is_negative() {
            return Err(DomainError::NegativeMoney {
                amount: amount.to_string(),
            });
        }
        Ok(Self { amount, currency: self.currency })
    }

    /// Checked multiplication by a fixed-point factor (e.g. fee rates).
    pub fn checked_mul(&self, factor: &FixedPoint) -> Result<Self, DomainError> {
        let raw = self.amount.checked_mul(factor)?;
        if raw.is_negative() {
            return Err(DomainError::NegativeMoney {
                amount: raw.to_string(),
            });
        }
        let amount = raw.with_scale(MONEY_SCALE)?;
        Ok(Self { amount, currency: self.currency })
    }

    /// Checked division by a fixed-point divisor, rounded to the canonical scale.
    pub fn checked_div(&self, divisor: &FixedPoint) -> Result<Self, DomainError> {
        let raw = self.amount.checked_div(divisor, MONEY_SCALE)?;
        if raw.is_negative() {
            return Err(DomainError::NegativeMoney {
                amount: raw.to_string(),
            });
        }
        Ok(Self { amount: raw, currency: self.currency })
    }

    fn ensure_same_currency(&self, other: &Self) -> Result<(), DomainError> {
        if self.currency == other.currency {
            Ok(())
        } else {
            Err(DomainError::CurrencyMismatch {
                left: self.currency,
                right: other.currency,
            })
        }
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.amount, self.currency)
    }
}

impl Serialize for Money {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("Money", 2)?;
        state.serialize_field("amount", &self.amount.to_string())?;
        state.serialize_field("currency", &self.currency)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for Money {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct MoneyRepr {
            amount: String,
            currency: Currency,
        }
        let repr = MoneyRepr::deserialize(deserializer)?;
        Self::parse(&repr.amount, repr.currency).map_err(DeError::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_strings() {
        let p = Price::parse("34570.5").unwrap();
        assert_eq!(p.as_decimal_string(), "34570.5000");
        assert_eq!(FixedPoint::parse("0.5").unwrap().to_string(), "0.5");
        assert_eq!(FixedPoint::parse("-12.34").unwrap().to_string(), "-12.34");
        assert_eq!(FixedPoint::parse("007").unwrap().to_string(), "7");
    }

    #[test]
    fn value_equality_across_scales() {
        let a = FixedPoint::parse("1.0").unwrap();
        let b = FixedPoint::parse("1.00").unwrap();
        assert_eq!(a, b);
        assert_ne!(FixedPoint::parse("12.3").unwrap(), FixedPoint::parse("1.23").unwrap());
        assert!(FixedPoint::parse("12.3").unwrap() > FixedPoint::parse("1.23").unwrap());
    }

    #[test]
    fn round_half_to_even() {
        // 1.5 -> 2, 2.5 -> 2, -1.5 -> -2, -2.5 -> -2
        assert_eq!(FixedPoint::parse("1.5").unwrap().with_scale(0).unwrap().to_string(), "2");
        assert_eq!(FixedPoint::parse("2.5").unwrap().with_scale(0).unwrap().to_string(), "2");
        assert_eq!(FixedPoint::parse("-1.5").unwrap().with_scale(0).unwrap().to_string(), "-2");
        assert_eq!(FixedPoint::parse("-2.5").unwrap().with_scale(0).unwrap().to_string(), "-2");
        assert_eq!(FixedPoint::parse("1.5001").unwrap().with_scale(0).unwrap().to_string(), "2");
        assert_eq!(FixedPoint::parse("1.4999").unwrap().with_scale(0).unwrap().to_string(), "1");
    }

    #[test]
    fn division_rounds() {
        let ten = FixedPoint::parse("10").unwrap();
        let three = FixedPoint::parse("3").unwrap();
        assert_eq!(ten.checked_div(&three, 2).unwrap().to_string(), "3.33");
        // 1/2 at scale 0 -> round-half-even -> 0
        assert_eq!(
            FixedPoint::parse("1").unwrap().checked_div(&FixedPoint::parse("2").unwrap(), 0).unwrap().to_string(),
            "0"
        );
    }

    #[test]
    fn quantity_rejects_fractional() {
        assert!(matches!(
            Quantity::parse("0.5"),
            Err(DomainError::FractionalQuantity { .. })
        ));
        assert_eq!(Quantity::parse("100.0").unwrap().as_decimal_string(), "100");
    }
}
