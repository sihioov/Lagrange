//! Owner-beta price-only simulation v2.
//!
//! This is a new contract boundary. The single-asset v1 contract and its
//! canonical bytes remain owned by the parent module and are not relaxed here.

mod contract;
mod engine;
mod input;
mod ledger;
mod metrics;
mod schedule;
mod signals;

pub use contract::{
    APPROVED_CONTRACT_SHA256, CONTRACT_VERSION, RESULT_SCHEMA_VERSION, V2ContractError,
    V2FactorDefinition, V2InputIdentity, V2InstrumentWeight, V2SimulationPair, V2SimulationResult,
    V2Strategy, V2StrategyIdentity, V2TargetAllocation,
};
pub use engine::{
    V2HashedSimulationPair, V2SimulationRunError, V2SimulationRunRequest,
    prepare_v2_simulation_input, prepare_v2_simulation_schedule, run_prepared_v2_simulation,
    run_v2_simulation,
};
pub use input::{V2InputError, V2PreparedInput};
pub use ledger::{
    V2CloseMark, V2InstrumentPrice, V2Ledger, V2LedgerRun, V2MarketSession, V2Order, V2OrderSide,
    V2Position, V2RebalanceRecord, V2TargetObservation, V2UnfilledReason,
};
pub use metrics::V2PerformanceMetrics;
pub use schedule::{V2Decision, V2DecisionKind, V2Schedule};
pub use signals::V2SignalError;
