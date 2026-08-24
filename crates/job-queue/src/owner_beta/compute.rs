//! The owner-beta price-only computation seam.
//!
//! This is the one filesystem-facing entry point for the pure target
//! builder.  Approval is performed on every call, and the factor builder is
//! given only the factors derived from the immutable strategy snapshot.

use std::{fmt, path::Path};

use factor_engine::price_only::{PRICE_ONLY_CAPABILITY, PRICE_ONLY_INPUT_KIND};
use factor_engine::{Field, PriceOnlyFactorSnapshot, PriceOnlyFactorSnapshotBuilder};

use super::{
    OWNER_BETA_TARGET_HASH_ALGORITHM, OWNER_BETA_TARGET_SNAPSHOT_SCHEMA,
    OwnerBetaPriceRecommendationInput, OwnerBetaTargetSnapshot, build_target_snapshot,
};
use crate::{
    factor_series::factors_for, recommendation::compute::requirements_for, resolver::ResolvedConfig,
};

/// Static, sanitized computation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OwnerBetaComputationError {
    #[error("owner-beta strategy is invalid")]
    StrategyInvalid,
    #[error("owner-beta artifact approval was rejected")]
    ArtifactApprovalRejected,
    #[error("owner-beta factor definition is invalid")]
    FactorDefinitionInvalid,
    #[error("owner-beta factor computation is invalid")]
    FactorComputeInvalid,
    #[error("owner-beta target is invalid")]
    TargetInvalid,
}

/// The sealed outputs of one owner-beta computation.
pub struct OwnerBetaComputation {
    factor_snapshot: PriceOnlyFactorSnapshot,
    target_snapshot: OwnerBetaTargetSnapshot,
}

impl fmt::Debug for OwnerBetaComputation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnerBetaComputation")
            .field("status", &"complete")
            .field("outputs", &"redacted")
            .finish()
    }
}

impl OwnerBetaComputation {
    /// The immutable factor snapshot produced by the exact strategy factor
    /// set.  The caller receives a read-only reference.
    pub fn factor_snapshot(&self) -> &PriceOnlyFactorSnapshot {
        &self.factor_snapshot
    }

    /// The immutable, hashed target snapshot.  The caller receives a
    /// read-only reference.
    pub fn target_snapshot(&self) -> &OwnerBetaTargetSnapshot {
        &self.target_snapshot
    }
}

/// Approve the fixed artifact, compute exactly the strategy-declared factors,
/// and build the deterministic owner-beta target.
pub fn compute_owner_beta_price_recommendation(
    artifact_root: &Path,
    input: &OwnerBetaPriceRecommendationInput,
) -> Result<OwnerBetaComputation, OwnerBetaComputationError> {
    input
        .validate_strategy_snapshot()
        .map_err(|_| OwnerBetaComputationError::StrategyInvalid)?;

    let approved = market_data::approve_historical_price_only_artifact(artifact_root)
        .map_err(|_| OwnerBetaComputationError::ArtifactApprovalRejected)?;
    input
        .validate_approved_artifact(&approved)
        .map_err(|_| OwnerBetaComputationError::ArtifactApprovalRejected)?;

    let strategy = input.strategy_snapshot();
    let resolved = ResolvedConfig {
        strategy_id: strategy.strategy_id().to_owned(),
        strategy_version: strategy.strategy_version().to_owned(),
        config: strategy.config_json().clone(),
    };
    let requirements =
        requirements_for(&resolved).map_err(|_| OwnerBetaComputationError::StrategyInvalid)?;
    let factors = factors_for(&requirements.factor_ids)
        .map_err(|_| OwnerBetaComputationError::FactorDefinitionInvalid)?;
    if factors
        .iter()
        .any(|factor| factor.required_fields() != [Field::CLOSE])
    {
        return Err(OwnerBetaComputationError::FactorDefinitionInvalid);
    }

    let factor_snapshot = PriceOnlyFactorSnapshotBuilder::new(&approved, input.as_of())
        .with_factors(factors)
        .map_err(|_| OwnerBetaComputationError::FactorDefinitionInvalid)?
        .build()
        .map_err(|_| OwnerBetaComputationError::FactorComputeInvalid)?;
    validate_factor_snapshot(input, &factor_snapshot)?;

    let target_snapshot = build_target_snapshot(input, &factor_snapshot)
        .map_err(|_| OwnerBetaComputationError::TargetInvalid)?;
    target_snapshot
        .validate_hash()
        .map_err(|_| OwnerBetaComputationError::TargetInvalid)?;
    validate_target_identity(input, &factor_snapshot, &target_snapshot)?;

    Ok(OwnerBetaComputation {
        factor_snapshot,
        target_snapshot,
    })
}

fn validate_factor_snapshot(
    input: &OwnerBetaPriceRecommendationInput,
    factor_snapshot: &PriceOnlyFactorSnapshot,
) -> Result<(), OwnerBetaComputationError> {
    input
        .validate_factor_snapshot(factor_snapshot)
        .map_err(|_| OwnerBetaComputationError::FactorComputeInvalid)?;
    let computed_hash = factor_snapshot
        .compute_hash()
        .map_err(|_| OwnerBetaComputationError::FactorComputeInvalid)?;
    if computed_hash != factor_snapshot.hash {
        return Err(OwnerBetaComputationError::FactorComputeInvalid);
    }
    Ok(())
}

fn validate_target_identity(
    input: &OwnerBetaPriceRecommendationInput,
    factor_snapshot: &PriceOnlyFactorSnapshot,
    target_snapshot: &OwnerBetaTargetSnapshot,
) -> Result<(), OwnerBetaComputationError> {
    let strategy = input.strategy_snapshot();
    if target_snapshot.schema() != OWNER_BETA_TARGET_SNAPSHOT_SCHEMA
        || target_snapshot.hash_algorithm() != OWNER_BETA_TARGET_HASH_ALGORITHM
        || target_snapshot.input_kind() != PRICE_ONLY_INPUT_KIND
        || target_snapshot.capability() != PRICE_ONLY_CAPABILITY
        || target_snapshot.as_of() != input.as_of()
        || target_snapshot.strategy_id() != strategy.strategy_id()
        || target_snapshot.strategy_version() != strategy.strategy_version()
        || target_snapshot.strategy_config_sha256() != strategy.config_sha256()
        || target_snapshot.factor_snapshot_sha256() != &factor_snapshot.hash
        || target_snapshot.pins() != input.pins()
    {
        return Err(OwnerBetaComputationError::TargetInvalid);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_are_static_and_sanitized() {
        let variants = [
            OwnerBetaComputationError::StrategyInvalid,
            OwnerBetaComputationError::ArtifactApprovalRejected,
            OwnerBetaComputationError::FactorDefinitionInvalid,
            OwnerBetaComputationError::FactorComputeInvalid,
            OwnerBetaComputationError::TargetInvalid,
        ];
        for error in variants {
            let display = error.to_string();
            let debug = format!("{error:?}");
            assert!(!display.contains("/"));
            assert!(!display.contains("sha256:"));
            assert!(!debug.contains("/"));
            assert!(!debug.contains("sha256:"));
        }
    }

    #[test]
    fn orchestration_keeps_approval_and_exact_factor_builder_calls() {
        let source = include_str!("compute.rs");
        let production = source.split("#[cfg(test)]").next().expect("production");
        let approval = production
            .find("approve_historical_price_only_artifact(artifact_root)")
            .expect("approval call");
        let builder = production
            .find("PriceOnlyFactorSnapshotBuilder::new(&approved, input.as_of())")
            .expect("exact factor builder");
        let target = production
            .find("build_target_snapshot(input, &factor_snapshot)")
            .expect("target call");
        assert!(approval < builder && builder < target);
        assert!(!production.contains("std::env"));
        assert!(!production.contains("Command::"));
    }
}
