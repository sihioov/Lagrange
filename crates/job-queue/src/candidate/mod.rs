//! Common stock-candidate scheduling and worker pipeline.
//!
//! This is separate from [`crate::recommendation`], whose contract remains an
//! owner-scoped ETF target portfolio.

/// Canonical universe identity is owned by the market-data source contract;
/// the queue only re-exports it at the scheduling boundary.
pub use market_data::CandidateUniverseKey;

pub mod input;
pub mod runner;
pub mod schedule;

pub use input::{CandidateInputError, CandidatePayload};
pub use runner::{
    CandidateOutcome, CandidateRunnerConfig, CandidateRunnerError, CandidateRunnerPaths, run_once,
};
pub use schedule::{
    CandidateScheduleBatchReport, CandidateScheduleError, CandidateScheduleFailure,
    CandidateScheduleReport, CandidateScheduleRequest, DatasetSchedulePin, schedule_candidate_run,
    schedule_latest_candidate_run, schedule_latest_candidate_runs,
};
