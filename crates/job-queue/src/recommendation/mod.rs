//! Recommendation-worker input, computation, child, validation, publication,
//! and queue-orchestration boundaries.

pub mod child;
pub mod compute;
pub mod input;
pub mod publish;
mod runner;
pub mod schedule;
pub mod validate;

pub use runner::{
    RecommendationOutcome, RecommendationRunnerConfig, RecommendationRunnerConfigError,
    RecommendationRunnerError, RecommendationRunnerPaths, run_once,
};
