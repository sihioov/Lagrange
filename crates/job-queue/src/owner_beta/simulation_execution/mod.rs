//! Owner-scoped queue input and whole-pair validation for price-only runs.
//!
//! This boundary validates data bindings only. Reusing a queued input still
//! requires approving the artifact at execution time, and an EngineIdentity
//! is not proof that a particular executable produced a result.

mod child;
mod input;
mod pair;
mod provenance;
mod publish;
mod recovery;
mod runner;
mod storage;

pub use input::SimulationJobInput;
pub use pair::CheckedSimulationPair;
pub use provenance::TrustedSimulationRuntime;
pub use publish::{
    SimulationCommitState, SimulationPublicationError, SimulationPublicationFailure,
    SimulationPublicationOutcome, SimulationRecoveryReport, publish_simulation_success,
    reconcile_simulation_commit, settle_invalid_simulation_claim, settle_simulation_failure,
};
pub use recovery::recover_simulation_claims;
pub use runner::{
    SimulationWorkerError, SimulationWorkerOutcome, run_simulation_child_mode, run_simulation_once,
};
pub use storage::check_stored_simulation_pair;

pub const OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE: &str = "owner_beta_price_only_simulation";
pub const OWNER_BETA_PRICE_ONLY_SIMULATION_SCHEMA: &str = "owner-beta-price-only-simulation-job-v1";

/// Static, value-free failures at the queue input and checked-pair boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SimulationExecutionError {
    #[error("simulation job input is invalid")]
    InputInvalid,
    #[error("simulation evaluation window is invalid")]
    WindowInvalid,
    #[error("simulation strategy snapshot is invalid")]
    StrategyInvalid,
    #[error("simulation result pair is invalid")]
    PairInvalid,
    #[error("simulation provenance binding does not match")]
    ProvenanceMismatch,
    #[error("simulation result pair exceeds the size limit")]
    SizeLimitExceeded,
}
