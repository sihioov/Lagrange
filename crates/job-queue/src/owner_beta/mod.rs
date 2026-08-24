//! Owner-beta job contracts.
//!
//! This module contains the sealed price-only recommendation input, pure
//! computation, target snapshot, and dedicated atomic publication boundary.
//! Worker process orchestration remains outside this boundary.

pub mod compute;
pub mod input;
pub mod publish;
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

pub use target::{
    OWNER_BETA_TARGET_HASH_ALGORITHM, OWNER_BETA_TARGET_SNAPSHOT_SCHEMA,
    OWNER_BETA_TARGET_WEIGHT_SCALE, OwnerBetaReason, OwnerBetaReasonCode, OwnerBetaTargetItem,
    OwnerBetaTargetSnapshot, OwnerBetaTargetSnapshotError, build_target_snapshot,
};
