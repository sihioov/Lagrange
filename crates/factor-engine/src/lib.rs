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
pub mod contract;
pub mod factors;
pub mod fundamentals;
pub mod lazy_util;
pub mod months;
pub mod normalize;
pub mod snapshot;

pub use contract::{Factor, FactorContext, FactorError, FactorFrame, FactorValue, Field};
pub use normalize::{NormalizePolicy, PercentilePolicy, WinsorizePolicy, ZScorePolicy};
pub use snapshot::{FactorRow, FactorSnapshot, FactorSnapshotBuilder, FrozenUniverse};
