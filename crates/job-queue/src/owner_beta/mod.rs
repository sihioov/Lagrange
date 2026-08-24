//! Owner-beta job contracts.
//!
//! This module contains the sealed price-only recommendation input, pure
//! computation, target snapshot, dedicated atomic publication boundary, and
//! its lease-supervised runner.  The runner has no generic recommendation,
//! Curated, Paper, provider, process, or order path.

pub mod compute;
pub mod input;
pub mod publish;
pub mod recovery;
pub mod runner;
pub mod target;

pub use compute::{
    OwnerBetaComputation, OwnerBetaComputationError, compute_owner_beta_price_recommendation,
};

pub use input::{
    OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE, OWNER_BETA_STRATEGY_CONFIG_SNAPSHOT_SCHEMA,
    OwnerBetaPriceRecommendationInput, OwnerBetaPriceRecommendationInputError,
    OwnerBetaPriceRecommendationPins, OwnerBetaStrategySnapshot,
};

pub use publish::{
    OwnerBetaPublicationError, OwnerBetaPublicationFailure, OwnerBetaPublicationOutcome,
    publish_owner_beta_success, settle_owner_beta_failure,
};

pub use recovery::{OwnerBetaRecoveryError, OwnerBetaRecoveryReport, recover_owner_beta_claims};

pub use runner::{
    OwnerBetaOutcome, OwnerBetaRunnerConfig, OwnerBetaRunnerConfigError, OwnerBetaRunnerError,
    OwnerBetaRunnerPaths, run_once,
};

pub use target::{
    OWNER_BETA_TARGET_HASH_ALGORITHM, OWNER_BETA_TARGET_SNAPSHOT_SCHEMA,
    OWNER_BETA_TARGET_WEIGHT_SCALE, OwnerBetaReason, OwnerBetaReasonCode, OwnerBetaTargetItem,
    OwnerBetaTargetSnapshot, OwnerBetaTargetSnapshotError, build_target_snapshot,
};
