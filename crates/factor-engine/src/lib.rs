//! `factor-engine` — Lagrange Station research factors (design §6.5,
//! requirements FR-SEL-002).
//!
//! Versioned factor definitions computed with **Polars lazy expressions** on
//! the curated bars zone (Todo 10 layout), normalized with versioned
//! winsorization / z-score / percentile policies, and frozen into
//! byte-hash-stable cross-sectional snapshots.
//!
//! Pipeline (consumed by the Todo 16 selector):
//!
//! ```text
//! curated bars (per-instrument parquet, versioned)
//!   -> Bars::from_curated        (universe gate + future-row rejection)
//!   -> Factor::compute           (lazy polars expressions, typed NULLs)
//!   -> NormalizePolicy::apply    (per-date frozen cross-section)
//!   -> FactorSnapshot            (canonical bytes + SHA-256 hash)
//! ```
//!
//! No-forward-fill / no-look-ahead invariants: the reference bar of a month
//! window is the LAST bar on or before the calendar target date; a bar after
//! the target is never used. Bars dated after `as_of` are typed rejections.
//! Every factor value is finite or typed NULL — never NaN/Inf.

pub mod bars;
pub mod candidate;
pub mod contract;
pub mod factors;
pub mod fixed_stock_price_beta;
pub mod fundamentals;
pub mod lazy_util;
pub mod months;
pub mod normalize;
pub mod price_only;
pub mod snapshot;

pub use candidate::{
    CandidateAnalysis, CandidateAxis, CandidateAxisScore, CandidateExclusion, CandidateFactorValue,
    CandidateFlags, CandidateInstrumentInput, CandidateScenario, CandidateScoreError,
    CandidateScoringConfig, CandidateSession, EvidenceStrength, NormalizationScope,
    score_candidates, top_five,
};
pub use contract::{Factor, FactorContext, FactorError, FactorFrame, FactorValue, Field};
pub use fixed_stock_price_beta::{
    BEARISH_DRAWDOWN_MAX, BEARISH_RETURN_20_MAX, BULLISH_RETURN_20_MIN, BULLISH_VOLATILITY_120_MAX,
    FIXED_STOCK_PRICE_BETA_FACTOR_VERSION, FIXED_STOCK_PRICE_BETA_SIGNAL_ACTIVITY_LABEL,
    FIXED_STOCK_PRICE_BETA_SIGNAL_AUDIENCE, FIXED_STOCK_PRICE_BETA_SIGNAL_CAPABILITY,
    FIXED_STOCK_PRICE_BETA_SIGNAL_FACTOR_VERSION, FIXED_STOCK_PRICE_BETA_SIGNAL_FLOAT_TOLERANCE,
    FIXED_STOCK_PRICE_BETA_SIGNAL_INDEX_MEMBERSHIP, FIXED_STOCK_PRICE_BETA_SIGNAL_SCHEMA_ID,
    FIXED_STOCK_PRICE_BETA_SIGNAL_SCHEMA_VERSION, FIXED_STOCK_PRICE_BETA_SIGNAL_SELECTION_BASIS,
    FIXED_STOCK_PRICE_BETA_SIGNAL_UNIVERSE_ID, FIXED_STOCK_PRICE_BETA_SIGNAL_WARNING,
    ORIGINAL_PRICE_WARNING, PRICE_VOLUME_SIGNAL_ACTIVITY_LABEL, PRICE_VOLUME_SIGNAL_AUDIENCE,
    PRICE_VOLUME_SIGNAL_CAPABILITY, PRICE_VOLUME_SIGNAL_INDEX_MEMBERSHIP,
    PRICE_VOLUME_SIGNAL_SCHEMA_ID, PRICE_VOLUME_SIGNAL_SCHEMA_VERSION,
    PRICE_VOLUME_SIGNAL_SELECTION_BASIS, PRICE_VOLUME_SIGNAL_WARNING, PriceVolumeSignalError,
    PriceVolumeSignalRow, PriceVolumeSignalSnapshot, ResearchCondition,
    read_fixed_stock_price_beta_snapshot, read_fixed_stock_price_beta_snapshot_against,
    write_fixed_stock_price_beta_snapshot, write_fixed_stock_price_beta_snapshot_against,
};
pub use normalize::{NormalizePolicy, PercentilePolicy, WinsorizePolicy, ZScorePolicy};
pub use price_only::{PriceOnlyFactorSnapshot, PriceOnlyFactorSnapshotBuilder};
pub use snapshot::{FactorRow, FactorSnapshot, FactorSnapshotBuilder, FrozenUniverse};
