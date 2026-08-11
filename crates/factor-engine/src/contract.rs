//! The documented `Factor` contract (design §6.5) with version,
//! required-field, lookback, and null-policy metadata (FR-SEL-002: "팩터
//! 정의, lookback, 결측값 정책이 버전 관리된다").

use std::fmt;

use domain::{DomainError, FactorVersion, InstrumentId, TradingDate};

use crate::bars::Bars;
use crate::snapshot::FrozenUniverse;

/// The stable identifier of a factor (e.g. `return_1m`).
pub type FactorId = String;

/// One required input field of a factor. The engine validates availability
/// before any computation (typed [`FactorError::MissingField`], never a
/// panic).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Field(&'static str);

impl Field {
    /// The split-adjusted close (signal series).
    pub const CLOSE: Field = Field::new("close");
    /// The reported trading value (거래대금, never adjusted).
    pub const TRADING_VALUE: Field = Field::new("trading_value");

    /// A field with the given stable wire name.
    pub const fn new(name: &'static str) -> Self {
        Self(name)
    }

    /// The stable wire name.
    pub const fn as_str(&self) -> &'static str {
        self.0
    }
}

impl fmt::Display for Field {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// How much history a factor needs, recorded as versioned metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lookback {
    /// N calendar months back, resolved to the last bar on or before the
    /// target MONTH's last day (documented month-end convention).
    CalendarMonths(u32),
    /// A fixed trading-day window that must be FULLY populated.
    TradingDays { window: usize, min_periods: usize },
    /// A fixed row window (full window required).
    FixedWindow { window: usize, min_periods: usize },
    /// Defined from the first observation (running history).
    FullHistory,
}

/// The documented NULL policy (design §6.5 "결측값 정책"), recorded per
/// factor / per normalization policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NullPolicy {
    /// Insufficient lookback history -> the factor value is typed NULL.
    InsufficientLookback,
    /// A NULL input inside a strict window -> NULL output (no partial reuse).
    StrictWindow,
    /// A required field is absent from the input -> typed error.
    MissingRequiredField,
    /// A cross-section with zero variance -> NULL normalized values.
    ZeroVariance,
}

/// Everything a factor needs to compute one frame. `bars` is already gated by
/// the frozen universe and has no row after `as_of` (see [`Bars::from_curated`]).
pub struct FactorContext<'a> {
    /// The snapshot as-of date (frozen).
    pub as_of: TradingDate,
    /// The frozen cross-sectional universe of the snapshot.
    pub universe: &'a FrozenUniverse,
    /// The resolved curated bars (typed series + lazy frame).
    pub bars: &'a Bars,
    /// Point-in-time fundamentals, empty when the dataset carries none.
    ///
    /// Unlike `bars`, a value here must be read PER BAR DATE via
    /// [`crate::fundamentals::Fundamentals::value_on`] rather than once for
    /// the snapshot: the whole point of the type is that the answer changes
    /// with the date you ask from. No shipped factor consumes it yet.
    pub fundamentals: &'a crate::fundamentals::Fundamentals,
}

/// One factor value for one instrument on one bar date.
#[derive(Debug, Clone, PartialEq)]
pub struct FactorValue {
    pub instrument: InstrumentId,
    pub date: TradingDate,
    /// The raw factor value, or NULL per the factor's documented policy.
    pub value: Option<f64>,
}

/// One factor's output frame; rows are sorted by (instrument, date).
#[derive(Debug, Clone, PartialEq)]
pub struct FactorFrame {
    pub factor: FactorId,
    pub rows: Vec<FactorValue>,
}

/// The documented factor interface (design §6.5). Each implementation is a
/// versioned, documented transformation.
pub trait Factor {
    /// The stable factor id (e.g. `return_1m`).
    fn id(&self) -> &str;
    /// The immutable factor version (semver).
    fn version(&self) -> FactorVersion;
    /// The fields this factor requires from the input.
    fn required_fields(&self) -> &[Field];
    /// The documented lookback of this factor version.
    fn lookback(&self) -> Lookback;
    /// The documented NULL behavior of this factor version.
    fn null_policy(&self) -> NullPolicy;
    /// Computes the factor frame over the context (lazy polars expressions).
    fn compute(&self, ctx: &FactorContext) -> Result<FactorFrame, FactorError>;
}

/// A typed factor-engine failure. Computation never panics on malformed
/// input: every failure mode is one of these variants.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum FactorError {
    #[error("future-dated row: {instrument} bar on {date} is after as-of {as_of}")]
    FutureDatedRow {
        instrument: String,
        date: String,
        as_of: String,
    },
    #[error("factor {factor} requires field {field} which the input does not carry")]
    MissingField { factor: String, field: String },
    #[error("invalid factor definition: {detail}")]
    InvalidDefinition { detail: String },
    #[error("curated store failure ({context}): {detail}")]
    StoreIo { context: String, detail: String },
    #[error("polars computation failure: {detail}")]
    Polars { detail: String },
    #[error("canonical serialization failure: {detail}")]
    Serialize { detail: String },
    #[error("non-finite factor value for {factor} {instrument} {date}: {value}")]
    NonFinite {
        factor: String,
        instrument: String,
        date: String,
        value: f64,
    },
    #[error("domain arithmetic failure: {0}")]
    Domain(#[from] DomainError),
}
