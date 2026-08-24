//! Owner-beta job contracts.
//!
//! This module deliberately contains only the sealed, price-only recommendation
//! input. Queue persistence and execution remain outside this boundary.

pub mod input;
pub mod target;

pub use input::{
    OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE, OWNER_BETA_STRATEGY_CONFIG_SNAPSHOT_SCHEMA,
    OwnerBetaPriceRecommendationInput, OwnerBetaPriceRecommendationInputError,
    OwnerBetaPriceRecommendationPins, OwnerBetaStrategySnapshot,
};

pub use target::{
    OWNER_BETA_TARGET_HASH_ALGORITHM, OWNER_BETA_TARGET_SNAPSHOT_SCHEMA,
    OWNER_BETA_TARGET_WEIGHT_SCALE, OwnerBetaReason, OwnerBetaReasonCode, OwnerBetaTargetItem,
    OwnerBetaTargetSnapshot, OwnerBetaTargetSnapshotError, build_target_snapshot,
};
