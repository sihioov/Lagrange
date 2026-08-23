//! Owner-beta job contracts.
//!
//! This module deliberately contains only the sealed, price-only recommendation
//! input. Queue persistence and execution remain outside this boundary.

pub mod input;

pub use input::{
    OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE, OwnerBetaPriceRecommendationInput,
    OwnerBetaPriceRecommendationInputError, OwnerBetaPriceRecommendationPins,
};
