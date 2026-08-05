//! The per-strategy selection specification (FR-SEL-003: "필터·점수·상위 N개
//! 선정 규칙을 전략별로 구성"). A spec is validated at construction; invalid
//! specs are typed [`SelectorError::InvalidSpec`], never accepted.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::error::SelectorError;

/// One strategy's selection rules: factor score weights, mandatory factors,
/// top-N, and the documented portfolio constraints.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectionSpec {
    /// The stable strategy id (Todo 17 registry).
    pub strategy_id: String,
    /// The immutable strategy version (semver).
    pub strategy_version: String,
    /// Score weights per factor id (factor ids must exist in the snapshot).
    pub factor_weights: BTreeMap<String, f64>,
    /// Mandatory factors: an instrument with any NULL mandatory factor is
    /// excluded (design §6.5 결측값 정책).
    pub mandatory_factors: BTreeSet<String>,
    /// The number of targets (ranks 1..=top_n receive weight).
    pub top_n: usize,
    /// Per-instrument target-weight ceiling (0 < max <= 1).
    pub max_weight: f64,
    /// Minimum cash weight (0 <= floor < 1).
    pub cash_floor: f64,
    /// Decimal places used when rounding target weights (1..=6).
    pub weight_scale: u8,
    /// Declared tolerance for weight-sum / constraint assertions.
    pub tolerance: f64,
}

impl SelectionSpec {
    /// Validates the documented contract; `Err(InvalidSpec)` on violation.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        strategy_id: &str,
        strategy_version: &str,
        factor_weights: BTreeMap<String, f64>,
        mandatory_factors: BTreeSet<String>,
        top_n: usize,
        max_weight: f64,
        cash_floor: f64,
        weight_scale: u8,
        tolerance: f64,
    ) -> Result<Self, SelectorError> {
        let spec = Self {
            strategy_id: strategy_id.to_owned(),
            strategy_version: strategy_version.to_owned(),
            factor_weights,
            mandatory_factors,
            top_n,
            max_weight,
            cash_floor,
            weight_scale,
            tolerance,
        };
        spec.validate()?;
        Ok(spec)
    }

    /// The documented spec contract (invalid specs are typed errors).
    pub fn validate(&self) -> Result<(), SelectorError> {
        let invalid = |detail: String| SelectorError::InvalidSpec { detail };
        if self.strategy_id.is_empty() {
            return Err(invalid("strategy_id must not be empty".to_owned()));
        }
        if self.strategy_version.is_empty() {
            return Err(invalid("strategy_version must not be empty".to_owned()));
        }
        if self.factor_weights.is_empty() {
            return Err(invalid("factor_weights must not be empty".to_owned()));
        }
        let mut total = 0.0;
        for (factor, weight) in &self.factor_weights {
            if factor.is_empty() {
                return Err(invalid("factor ids must not be empty".to_owned()));
            }
            if !weight.is_finite() {
                return Err(invalid(format!(
                    "factor {factor} weight {weight} is not finite"
                )));
            }
            if *weight < 0.0 {
                return Err(invalid(format!(
                    "factor {factor} weight {weight} is negative"
                )));
            }
            total += *weight;
        }
        if total <= 0.0 {
            return Err(invalid(
                "factor weights must sum to a positive value".to_owned(),
            ));
        }
        for factor in &self.mandatory_factors {
            if factor.is_empty() {
                return Err(invalid("mandatory factor ids must not be empty".to_owned()));
            }
        }
        if self.top_n == 0 {
            return Err(invalid("top_n must be at least 1".to_owned()));
        }
        if !(0.0..1.0).contains(&self.cash_floor) {
            return Err(invalid(format!(
                "cash_floor {} must be in [0, 1)",
                self.cash_floor
            )));
        }
        if !(0.0..=1.0).contains(&self.max_weight) {
            return Err(invalid(format!(
                "max_weight {} must be in (0, 1]",
                self.max_weight
            )));
        }
        if !(1..=6).contains(&self.weight_scale) {
            return Err(invalid(format!(
                "weight_scale {} must be in 1..=6",
                self.weight_scale
            )));
        }
        if !self.tolerance.is_finite() || self.tolerance <= 0.0 {
            return Err(invalid(format!(
                "tolerance {} must be finite and positive",
                self.tolerance
            )));
        }
        Ok(())
    }
}
