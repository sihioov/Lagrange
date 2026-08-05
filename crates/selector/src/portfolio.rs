//! `PortfolioConstraints` — the fifth pipeline stage (design §6.6, FR-SEL-004:
//! "종목당 최대 비중, 현금 최소 비중, 총 비중 합계 등 포트폴리오 제약").
//!
//! Deterministic weight algorithm (all arithmetic in integer basis points at
//! `weight_scale` decimals, so identical inputs always produce identical
//! weights):
//!
//! 1. investable = 1 - cash_floor; selected = ranks 1..=min(top_n, eligible).
//! 2. base = investable / selected; when base exceeds max_weight every
//!    selected target is capped at max_weight.
//! 3. Each target weight is truncated to `weight_scale` decimals.
//! 4. residue = investable - sum(truncated) is allocated to **cash** — never
//!    silently dropped, never redistributed (documented rule).
//!
//! The feasibility check (`max_weight * selected` must fit in investable)
//! yields a typed [`SelectorError::ImpossibleConstraints`] before any weight
//! is produced — no partial output.

use std::collections::BTreeMap;

use domain::InstrumentId;

use crate::error::SelectorError;
use crate::rank::RankedInstrument;
use crate::reason::{Reason, ReasonCode};
use crate::spec::SelectionSpec;

/// The weight outcome: per-instrument target weights, the cash weight, and
/// the structured reasons attached to each target / the portfolio.
#[derive(Debug, Clone, PartialEq)]
pub struct WeightAssignment {
    pub weights: BTreeMap<InstrumentId, f64>,
    pub cash_weight: f64,
    pub target_reasons: BTreeMap<InstrumentId, Vec<Reason>>,
    pub portfolio_reasons: Vec<Reason>,
}

/// Stage 5: constrained target weights.
#[derive(Debug, Clone)]
pub struct PortfolioConstraints {
    top_n: usize,
    max_weight: f64,
    cash_floor: f64,
    weight_scale: u8,
    tolerance: f64,
}

impl PortfolioConstraints {
    pub fn from_spec(spec: &SelectionSpec) -> Self {
        Self {
            top_n: spec.top_n,
            max_weight: spec.max_weight,
            cash_floor: spec.cash_floor,
            weight_scale: spec.weight_scale,
            tolerance: spec.tolerance,
        }
    }

    pub fn apply(&self, ranked: &[RankedInstrument]) -> Result<WeightAssignment, SelectorError> {
        let scale10: i128 = 10i128.pow(u32::from(self.weight_scale));
        let budget_bps: i128 = ((1.0 - self.cash_floor) * scale10 as f64).round() as i128;
        let n_selected = self.top_n.min(ranked.len());

        if n_selected == 0 {
            return Ok(WeightAssignment {
                weights: BTreeMap::new(),
                cash_weight: 1.0,
                target_reasons: BTreeMap::new(),
                portfolio_reasons: vec![Reason::new(ReasonCode::AllCashNoEligible, BTreeMap::new())],
            });
        }

        let investable = 1.0 - self.cash_floor;
        let base = investable / n_selected as f64;
        let capped = base > self.max_weight + self.tolerance;

        // Strategy-capacity sanity gate: the declared capacity (every
        // selected target at max weight) must fit within the weight left by
        // the cash floor. A capacity that cannot coexist with the floor
        // (e.g. cash floor > available weight) is a typed error BEFORE any
        // weight is produced — no partial output.
        if self.max_weight * n_selected as f64 > investable + self.tolerance {
            return Err(SelectorError::ImpossibleConstraints {
                detail: format!(
                    "cash floor {} and per-instrument max weight {} cannot fit {} selected target(s): declared capacity {} exceeds available weight {}",
                    self.cash_floor,
                    self.max_weight,
                    n_selected,
                    self.max_weight * n_selected as f64,
                    investable
                ),
            });
        }

        let mut weights: BTreeMap<InstrumentId, f64> = BTreeMap::new();
        let mut target_reasons: BTreeMap<InstrumentId, Vec<Reason>> = BTreeMap::new();
        let mut sum_truncated: i128 = 0;
        for ranked_inst in ranked {
            let mut reasons = vec![Reason::new(
                ReasonCode::SelectedTopN,
                BTreeMap::from([
                    ("rank".to_owned(), ranked_inst.rank.to_string()),
                    ("top_n".to_owned(), self.top_n.to_string()),
                ]),
            )];
            let weight = if ranked_inst.rank <= n_selected {
                let raw = if capped { self.max_weight } else { base };
                if capped {
                    reasons.push(Reason::new(
                        ReasonCode::WeightCappedAtMax,
                        BTreeMap::from([("max_weight".to_owned(), format!("{}", self.max_weight))]),
                    ));
                }
                let bps = (raw * scale10 as f64).floor() as i128;
                sum_truncated += bps;
                bps as f64 / scale10 as f64
            } else {
                reasons.push(Reason::new(
                    ReasonCode::NotSelectedBeyondTopN,
                    BTreeMap::from([
                        ("rank".to_owned(), ranked_inst.rank.to_string()),
                        ("top_n".to_owned(), self.top_n.to_string()),
                    ]),
                ));
                0.0
            };
            weights.insert(ranked_inst.instrument.clone(), weight);
            target_reasons.insert(ranked_inst.instrument.clone(), reasons);
        }

        let residue_bps = budget_bps - sum_truncated;
        if residue_bps < 0 {
            return Err(SelectorError::Internal {
                detail: format!("negative weight-rounding residue {residue_bps}"),
            });
        }
        let cash_bps = scale10 - sum_truncated;
        let cash_weight = cash_bps as f64 / scale10 as f64;

        let mut portfolio_reasons: Vec<Reason> = Vec::new();
        if residue_bps > 0 {
            portfolio_reasons.push(Reason::new(
                ReasonCode::WeightRoundingResidueToCash,
                BTreeMap::from([(
                    "residue".to_owned(),
                    format!(
                        "{:.scale$}",
                        residue_bps as f64 / scale10 as f64,
                        scale = usize::from(self.weight_scale)
                    ),
                )]),
            ));
        }
        if self.cash_floor > 0.0 {
            portfolio_reasons.push(Reason::new(
                ReasonCode::CashFloorApplied,
                BTreeMap::from([("cash_floor".to_owned(), format!("{}", self.cash_floor))]),
            ));
        }

        Ok(WeightAssignment {
            weights,
            cash_weight,
            target_reasons,
            portfolio_reasons,
        })
    }
}
