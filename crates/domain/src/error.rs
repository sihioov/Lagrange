//! Typed domain errors.
//!
//! [`DomainError`] is the single error type for every domain-contract
//! violation. It serializes with a stable `code` tag (the snake_case variant
//! name) plus any structured payload fields, so errors cross JSON boundaries
//! as typed values rather than bare strings. [`DomainError::code`] returns the
//! same code string used by the `result-model` envelope.

use serde::{Deserialize, Serialize};

use crate::currency::{Currency, Venue};

/// Typed error for every domain-contract violation.
///
/// Serde uses internal tagging on the `code` field, so a rejected negative
/// price serializes as `{"code":"non_positive_price","value":"-100.0000"}` and
/// deserializes back into the exact same variant (typed, not stringly-typed).
#[derive(Debug, Clone, PartialEq, Eq, Hash, thiserror::Error, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum DomainError {
    /// A string could not be parsed as a fixed-point decimal.
    #[error("invalid decimal string: {value}")]
    InvalidDecimalString { value: String },

    /// A requested decimal scale exceeds [`crate::fixed::MAX_SCALE`].
    #[error("decimal scale {scale} exceeds maximum {max}")]
    ScaleExceedsMax { scale: u8, max: u8 },

    /// A checked arithmetic operation overflowed the fixed-point range.
    #[error("arithmetic overflow: {operation}")]
    Overflow { operation: String },

    /// Division by zero.
    #[error("division by zero")]
    DivisionByZero,

    /// Money would become negative (negative cash is never allowed).
    #[error("money cannot be negative: {amount}")]
    NegativeMoney { amount: String },

    /// A price must be strictly positive.
    #[error("price must be positive: {value}")]
    NonPositivePrice { value: String },

    /// A quantity must be a non-negative whole number of units.
    #[error("quantity cannot be negative: {value}")]
    NegativeQuantity { value: String },

    /// A quantity must be a whole number of units (no fractional lots).
    #[error("quantity must be a whole number of units: {value}")]
    FractionalQuantity { value: String },

    /// A weight must lie in the closed interval [0, 1].
    #[error("weight {value} is outside [0, 1]")]
    WeightOutOfRange { value: String },

    /// An arithmetic operation combined two different currencies.
    #[error("currency mismatch: {left} != {right}")]
    CurrencyMismatch { left: Currency, right: Currency },

    /// A branded identifier failed its validation rule.
    #[error("invalid {kind}: {value}")]
    InvalidId { kind: String, value: String },

    /// An ISO 4217 currency code is not exactly three uppercase letters.
    #[error("invalid currency code: {value}")]
    InvalidCurrency { value: String },

    /// An unknown venue code.
    #[error("invalid venue: {value}")]
    InvalidVenue { value: String },

    /// An invalid IANA timezone name.
    #[error("invalid timezone: {value}")]
    InvalidTimeZone { value: String },

    /// A calendar date that does not exist (e.g. 2026-02-30).
    #[error("invalid trading date: {value}")]
    InvalidTradingDate { value: String },

    /// A venue-local wall clock time that occurs twice (DST fall-back).
    #[error("ambiguous local time for {venue} at {local}")]
    AmbiguousLocalTime { venue: Venue, local: String },

    /// A venue-local wall clock time that does not exist (DST spring-forward).
    #[error("nonexistent local time for {venue} at {local}")]
    NonexistentLocalTime { venue: Venue, local: String },

    /// A reported statistic must be a finite float (no NaN / Infinity).
    #[error("non-finite metric: {metric}")]
    NonFiniteMetric { metric: String },

    /// A content hash is not in `sha256:<64 lowercase hex>` form.
    #[error("invalid content hash: {value}")]
    InvalidContentHash { value: String },

    /// A semantic version string is malformed.
    #[error("invalid semantic version: {value}")]
    InvalidVersion { value: String },

    /// A git commit reference is not a hex string.
    #[error("invalid code commit: {value}")]
    InvalidCodeCommit { value: String },
}

impl DomainError {
    /// Stable snake_case error code (matches the serde `code` tag).
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidDecimalString { .. } => "invalid_decimal_string",
            Self::ScaleExceedsMax { .. } => "scale_exceeds_max",
            Self::Overflow { .. } => "overflow",
            Self::DivisionByZero => "division_by_zero",
            Self::NegativeMoney { .. } => "negative_money",
            Self::NonPositivePrice { .. } => "non_positive_price",
            Self::NegativeQuantity { .. } => "negative_quantity",
            Self::FractionalQuantity { .. } => "fractional_quantity",
            Self::WeightOutOfRange { .. } => "weight_out_of_range",
            Self::CurrencyMismatch { .. } => "currency_mismatch",
            Self::InvalidId { .. } => "invalid_id",
            Self::InvalidCurrency { .. } => "invalid_currency",
            Self::InvalidVenue { .. } => "invalid_venue",
            Self::InvalidTimeZone { .. } => "invalid_time_zone",
            Self::InvalidTradingDate { .. } => "invalid_trading_date",
            Self::AmbiguousLocalTime { .. } => "ambiguous_local_time",
            Self::NonexistentLocalTime { .. } => "nonexistent_local_time",
            Self::NonFiniteMetric { .. } => "non_finite_metric",
            Self::InvalidContentHash { .. } => "invalid_content_hash",
            Self::InvalidVersion { .. } => "invalid_version",
            Self::InvalidCodeCommit { .. } => "invalid_code_commit",
        }
    }
}
