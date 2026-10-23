//! Compute from a reopened sealed daily capability, never a latest path or wire
//! record. The runtime performs current DB authorization around this pure stage.

use factor_engine::{
    Field,
    daily_price_only::{DailyPriceOnlyFactorSnapshot, DailyPriceOnlyFactorSnapshotBuilder},
};
use market_data::daily_price_only::input::AdmittedDailyPriceInputV1;

use super::daily_input::OwnerBetaDailyRecommendationInputV1;
use super::daily_target::{OwnerBetaDailyTargetSnapshotV1, build_daily_target_snapshot};
use crate::{
    factor_series::factors_for, recommendation::compute::requirements_for, resolver::ResolvedConfig,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OwnerBetaDailyComputationError {
    #[error("owner-beta daily sealed input is invalid")]
    InputInvalid,
    #[error("owner-beta daily strategy is invalid")]
    StrategyInvalid,
    #[error("owner-beta daily factor definition is invalid")]
    FactorDefinitionInvalid,
    #[error("owner-beta daily factor computation is invalid")]
    FactorComputeInvalid,
    #[error("owner-beta daily target is invalid")]
    TargetInvalid,
}

pub struct OwnerBetaDailyComputation {
    factor_snapshot: DailyPriceOnlyFactorSnapshot,
    target_snapshot: OwnerBetaDailyTargetSnapshotV1,
}
impl OwnerBetaDailyComputation {
    pub fn factor_snapshot(&self) -> &DailyPriceOnlyFactorSnapshot {
        &self.factor_snapshot
    }
    pub fn target_snapshot(&self) -> &OwnerBetaDailyTargetSnapshotV1 {
        &self.target_snapshot
    }
}
impl std::fmt::Debug for OwnerBetaDailyComputation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnerBetaDailyComputation")
            .finish_non_exhaustive()
    }
}

pub fn compute_owner_beta_daily_recommendation(
    admitted: &AdmittedDailyPriceInputV1<'_, '_>,
    input: &OwnerBetaDailyRecommendationInputV1,
) -> Result<OwnerBetaDailyComputation, OwnerBetaDailyComputationError> {
    input
        .validate_admitted(admitted)
        .map_err(|_| OwnerBetaDailyComputationError::InputInvalid)?;
    let strategy = input.strategy_snapshot();
    let requirements = requirements_for(&ResolvedConfig {
        strategy_id: strategy.strategy_id().to_owned(),
        strategy_version: strategy.strategy_version().to_owned(),
        config: strategy.config_json().clone(),
    })
    .map_err(|_| OwnerBetaDailyComputationError::StrategyInvalid)?;
    let factors = factors_for(&requirements.factor_ids)
        .map_err(|_| OwnerBetaDailyComputationError::FactorDefinitionInvalid)?;
    if factors
        .iter()
        .any(|factor| factor.required_fields() != [Field::CLOSE])
    {
        return Err(OwnerBetaDailyComputationError::FactorDefinitionInvalid);
    }
    let factor_snapshot = DailyPriceOnlyFactorSnapshotBuilder::new(admitted, input.as_of())
        .with_factors(factors)
        .map_err(|_| OwnerBetaDailyComputationError::FactorDefinitionInvalid)?
        .build()
        .map_err(|_| OwnerBetaDailyComputationError::FactorComputeInvalid)?;
    input
        .validate_factor_snapshot(&factor_snapshot)
        .map_err(|_| OwnerBetaDailyComputationError::FactorComputeInvalid)?;
    let target_snapshot = build_daily_target_snapshot(input, &factor_snapshot)
        .map_err(|_| OwnerBetaDailyComputationError::TargetInvalid)?;
    target_snapshot
        .validate_hash()
        .map_err(|_| OwnerBetaDailyComputationError::TargetInvalid)?;
    input
        .validate_admitted(admitted)
        .map_err(|_| OwnerBetaDailyComputationError::InputInvalid)?;
    Ok(OwnerBetaDailyComputation {
        factor_snapshot,
        target_snapshot,
    })
}
