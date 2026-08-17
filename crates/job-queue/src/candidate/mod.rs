//! Common stock-candidate scheduling and worker pipeline.
//!
//! This is separate from [`crate::recommendation`], whose contract remains an
//! owner-scoped ETF target portfolio.

pub mod input;
pub mod runner;
pub mod schedule;

pub use input::{CandidateInputError, CandidatePayload};
pub use runner::{
    CandidateOutcome, CandidateRunnerConfig, CandidateRunnerError, CandidateRunnerPaths, run_once,
};
pub use schedule::{
    CandidateScheduleError, CandidateScheduleReport, CandidateScheduleRequest, DatasetSchedulePin,
    schedule_candidate_run, schedule_latest_candidate_run,
};
