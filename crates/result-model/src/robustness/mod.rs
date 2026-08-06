//! `robustness` — parent/derived run lineage, robustness suites, and the
//! Phase 1 core release gate (plan Todo 21, design §9.5-9.6, requirements
//! FR-ROB-001..005 / FR-BT-008..010 / AT-03..06).
//!
//! Module map:
//! - [`lineage`] — derived-run axes, parent pinning (strategy/data/engine),
//!   and the deterministic lineage registry (design §9.5: one variable per
//!   derived run).
//! - [`holdout`] — train/validation/test split with the selection barrier
//!   (FR-ROB-001: the final test period is never read during selection).
//! - [`replay`] — the deterministic fill→ledger→equity replay used by every
//!   derived-run simulation (cost stress, execution delay).
//! - [`cost`] — cost-stress scenarios (FR-ROB-003, AT-04).
//! - [`split`] — period segments and walk-forward folds (FR-ROB-004).
//! - [`delay`] — execution-delay scenarios (sessions).
//! - [`benchmark`] — benchmark comparison (design §9.5 `BenchmarkComparison`).
//! - [`warnings`] — concentration and recent-degradation warnings (FR-ROB-005)
//!   plus parameter-neighborhood analysis (FR-ROB-002).
//! - [`comparison`] — two-run comparison (FR-BT-010).
//! - [`stability`] — the reference stability score with raw evidence
//!   (design §9.6; explicitly NOT an investment-approval signal).
//! - [`missing`] — missing-data policy (AT-05).
//! - [`gate`] — the Phase 1 core golden gate (golden artifacts for the five
//!   strategies + AT-03/04/05/06 core evidence; rejects unapproved golden
//!   deltas and non-finite values).

pub mod benchmark;
pub mod comparison;
pub mod cost;
pub mod delay;
pub mod error;
pub mod holdout;
pub mod lineage;
pub mod missing;
pub mod replay;
pub mod split;
pub mod stability;
pub mod warnings;

pub use benchmark::{BenchmarkComparison, compare_benchmark};
pub use comparison::{EquityDiff, PresenceDiff, PositionDiff, RunComparison, SummaryDiff, compare_runs};
pub use cost::{CostStressProfile, stress_cost};
pub use delay::delay_execution;
pub use error::RobustnessError;
pub use holdout::{HoldoutBarrier, PeriodSplit, SplitResult, select_equity_series};
pub use missing::{ExclusionRecord, MissingDataOutcome, MissingDataPolicy, MissingInstrument, apply_missing_data_policy, enforce_missing_data_policy};
pub use lineage::{
    DerivedAxis, DerivedRunRequest, LineageRegistry, PinnedContext, RunLineage,
};
pub use replay::{ReplaySpec, replay, replay_with};
pub use warnings::{NeighborhoodAnalysis, NeighborhoodPoint, analyze_neighborhood, recent_degradation_warning, top_trade_concentration_warning, year_concentration_warning};
pub use stability::{StabilityComponent, StabilityEvidence, StabilityScore, analyze_stability, approve_investment};
pub use split::{
    PeriodSegments, Segment, SegmentMetrics, WalkForwardFold, WalkForwardPlan, split_period,
    walk_forward,
};
