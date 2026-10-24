//! The dedicated owner-beta price-only simulation result contract.
//!
//! This module owns the sealed v3 input adapter, existing owner-beta signal
//! connection, checked result contracts, and pure ledger engine. It has no
//! persistence, provider, process-spawn, or job-queue path.

mod canonical;
pub mod contract;
mod engine;
mod input;
mod ledger;
pub mod metrics;
mod signals;

pub(crate) use canonical::{canonical_json, parse_strict_json};
pub(crate) use contract::{
    APPROVED_ACTION_MANIFEST_SHA256, APPROVED_ARTIFACT_MANIFEST_SHA256,
    APPROVED_CANDIDATE_CONTENT_SHA256, APPROVED_REGISTRY_SHA256, APPROVED_STAGE5_MANIFEST_SHA256,
};
pub(crate) use input::reconstruct_approved_simulation_schedule;

pub use contract::{
    APPROVED_BARS_SHA256, APPROVED_CONTRACT_SHA256, APPROVED_END_DATE, APPROVED_SESSION_COUNT,
    APPROVED_START_DATE, BenchmarkLedger, CONTRACT_VERSION, CostProfile, EngineIdentity,
    FullGitRevision, LedgerMetrics, LedgerPhase, LedgerPhaseRecord, Money4,
    PRODUCTION_INITIAL_CAPITAL, PositionMark, Price4, RESULT_SCHEMA_VERSION, Ratio12, Sha256Digest,
    SignalSnapshot, SimulationContractError, SimulationConventions, SimulationResult,
    SimulationSchedule, StrategyIdentity, StrategyLedger, TradeEvent, TradeSide, UnfilledReason,
};
pub use engine::{
    HashedSimulationResult, PriceOnlySimulationResults, SimulationRunError, SimulationRunIdentity,
    SimulationRunRequest, run_approved_historical_price_only_simulation,
};
pub use input::{SimulationInputError, prepare_approved_simulation_window};
pub use ledger::{CostProfileLedgers, EvaluationPriceRow, SimulationLedgers, simulate_ledgers};
pub use metrics::{
    DerivedPerformanceMetrics, derive_benchmark_return_difference, derive_performance_metrics,
};
