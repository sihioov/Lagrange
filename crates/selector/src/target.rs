//! `TargetPortfolio` — the final pipeline stage output (design §6.6). The
//! explainable constrained target portfolio: targets with ranks, scores,
//! factor raw/normalized values, weights and structured reasons, plus the
//! cash weight, exclusions, and every snapshot/provenance id carried through.
//!
//! Targets only: this model contains weights, never orders / fills /
//! quantities — execution layers translate weights into orders downstream.

use std::collections::BTreeMap;

use domain::{ContentHash, InstrumentId, TradingDate};
use serde::Serialize;

use crate::eligibility::{Exclusion, FactorEvidence};
use crate::error::SelectorError;
use crate::reason::Reason;

/// One ranked instrument with its target weight and structured evidence.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TargetRow {
    pub instrument_id: InstrumentId,
    pub rank: usize,
    pub score: f64,
    /// factor id -> raw + normalized values (FR-SEL-005).
    pub factors: BTreeMap<String, FactorEvidence>,
    /// The target weight (0.0 for ranks beyond top_n).
    pub target_weight: f64,
    /// Structured reasons (code + ko/en text).
    pub reasons: Vec<Reason>,
}

/// The constraints the portfolio was built under (FR-SEL-004).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConstraintSummary {
    pub top_n: usize,
    pub max_weight: f64,
    pub cash_floor: f64,
    pub weight_scale: u8,
    pub tolerance: f64,
}

/// The explainable constrained target portfolio.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TargetPortfolio {
    /// The as-of date (frozen factor snapshot date).
    pub as_of: TradingDate,
    /// `strategy_id@strategy_version` (Todo 17 registry key).
    pub strategy_version: String,
    /// Todo 12 immutable universe snapshot id, carried through.
    pub universe_snapshot_id: String,
    /// Todo 15 immutable factor snapshot hash, carried through.
    pub factor_snapshot_hash: String,
    /// The dataset the factors were frozen over.
    pub dataset_id: String,
    pub dataset_version: u32,
    /// All eligible instruments in rank order (targets + explained zeroes).
    pub targets: Vec<TargetRow>,
    /// Every excluded instrument with its structured reason.
    pub exclusions: Vec<Exclusion>,
    /// The cash weight (>= cash floor by construction).
    pub cash_weight: f64,
    pub constraints: ConstraintSummary,
    /// Portfolio-level reasons (all-cash, cash floor, rounding residue).
    pub portfolio_reasons: Vec<Reason>,
    /// Immutable content hash over the canonical portfolio bytes.
    pub portfolio_snapshot_id: String,
}

impl TargetPortfolio {
    /// The canonical bytes the snapshot id covers (everything but the id).
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, SelectorError> {
        #[derive(Serialize)]
        struct Canonical<'a> {
            as_of: &'a str,
            strategy_version: &'a str,
            universe_snapshot_id: &'a str,
            factor_snapshot_hash: &'a str,
            dataset_id: &'a str,
            dataset_version: u32,
            targets: &'a [TargetRow],
            exclusions: &'a [Exclusion],
            cash_weight: f64,
            constraints: &'a ConstraintSummary,
            portfolio_reasons: &'a [Reason],
        }
        let canonical = Canonical {
            as_of: &self.as_of.to_iso(),
            strategy_version: &self.strategy_version,
            universe_snapshot_id: &self.universe_snapshot_id,
            factor_snapshot_hash: &self.factor_snapshot_hash,
            dataset_id: &self.dataset_id,
            dataset_version: self.dataset_version,
            targets: &self.targets,
            exclusions: &self.exclusions,
            cash_weight: self.cash_weight,
            constraints: &self.constraints,
            portfolio_reasons: &self.portfolio_reasons,
        };
        serde_json::to_vec(&canonical).map_err(|e| SelectorError::Internal {
            detail: format!("canonical portfolio serialization failed: {e}"),
        })
    }

    /// The SHA-256 over the canonical bytes: identical inputs -> identical id.
    pub fn compute_portfolio_snapshot_id(&self) -> Result<ContentHash, SelectorError> {
        Ok(ContentHash::from_bytes(&self.canonical_bytes()?))
    }
}
