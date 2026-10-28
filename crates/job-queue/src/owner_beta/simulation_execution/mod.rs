//! Owner-scoped queue input and whole-pair validation for price-only runs.
//!
//! This boundary validates data bindings only. Reusing a queued input still
//! requires approving the artifact at execution time, and an EngineIdentity
//! is not proof that a particular executable produced a result.

mod child;
mod input;
mod pair;
mod process_acl;
mod process_config;
mod process_configured_input_rights;
mod process_health;
mod process_migrations;
mod process_observation;
mod process_parent;
mod process_read;
mod process_rls_metadata;
mod process_routine_metadata;
mod process_startup;
mod process_trigger_metadata;
mod process_write;
mod provenance;
mod publish;
mod recovery;
mod runner;
mod storage;
pub mod v2;
mod versioned;

#[cfg(all(test, feature = "r4-pg-observer-tests"))]
#[path = "../../../tests/common/mod.rs"]
mod pg_test_common;

pub use input::SimulationJobInput;
pub use pair::CheckedSimulationPair;
pub use process_config::{
    SimulationProcessConfig, SimulationProcessConfigError, SimulationProcessLimits,
};
pub use process_health::{
    SimulationHealthError, SimulationHealthEvent, SimulationHealthFailure, SimulationHealthPhase,
    SimulationHealthRecord, SimulationHealthTracker, SimulationProcessGeneration,
};
pub use process_observation::{
    SimulationProcessObservation, SimulationProcessObservationError,
    observe_current_simulation_process, observe_simulation_process,
};
pub use process_parent::run_official_simulation_daemon;
pub use process_read::{
    SimulationProcessReadError, check_simulation_process_health, read_simulation_process_config,
};
pub use process_write::{
    SimulationHealthWriteError, SimulationHealthWriter, open_simulation_health_writer,
};
pub use provenance::TrustedSimulationRuntime;
pub use publish::{
    SimulationCommitState, SimulationPublicationError, SimulationPublicationFailure,
    SimulationPublicationOutcome, SimulationRecoveryReport, publish_simulation_success,
    publish_versioned_simulation_success, reconcile_simulation_commit,
    reconcile_versioned_simulation_commit, settle_invalid_simulation_claim,
    settle_simulation_failure, settle_versioned_simulation_failure,
};
pub use recovery::recover_simulation_claims;
pub use runner::{
    SimulationWorkerError, SimulationWorkerOutcome, run_simulation_child_mode, run_simulation_once,
};
pub use storage::check_stored_simulation_pair;
pub use v2::{V2CheckedSimulationPair, V2SimulationJobInput};
pub use versioned::{
    OWNER_BETA_PRICE_ONLY_SIMULATION_SCHEMA_V2, SimulationContractVersion,
    VersionedCheckedSimulationPair, VersionedSimulationJobInput, VersionedSimulationPairRef,
    VersionedSimulationResultRef, check_stored_versioned_simulation_pair,
};

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
