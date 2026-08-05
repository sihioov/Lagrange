//! `ScoreComposer` — the third pipeline stage (design §6.6). Composes the
//! weighted normalized score of every eligible instrument:
//!
//! `score = sum(weight_f * normalized_f)` over the spec's factor weights.
//!
//! A non-mandatory factor that is NULL contributes 0 (documented policy);
//! mandatory NULL factors never reach this stage (they are excluded). Every
//! referenced factor must exist in the snapshot registry, else a typed
//! [`SelectorError::UnknownFactor`].

use std::collections::{BTreeMap, BTreeSet};

use domain::InstrumentId;
use factor_engine::FactorSnapshot;

use crate::eligibility::{EligibilityOutcome, FactorEvidence};
use crate::error::SelectorError;
use crate::spec::SelectionSpec;

/// One eligible instrument with its composed score.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredInstrument {
    pub instrument: InstrumentId,
    pub score: f64,
    pub factors: BTreeMap<String, FactorEvidence>,
}

/// Stage 3: weighted normalized score composition.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScoreComposer;

impl ScoreComposer {
    pub fn new() -> Self {
        Self
    }

    pub fn compose(
        &self,
        outcome: &EligibilityOutcome,
        factors: &FactorSnapshot,
        spec: &SelectionSpec,
    ) -> Result<Vec<ScoredInstrument>, SelectorError> {
        let known: BTreeSet<&String> = factors.factor_versions.keys().collect();
        for factor in spec
            .factor_weights
            .keys()
            .chain(spec.mandatory_factors.iter())
        {
            if !known.contains(factor) {
                let known_list: Vec<&str> = known.iter().map(|s| s.as_str()).collect();
                return Err(SelectorError::UnknownFactor {
                    factor: factor.clone(),
                    known: known_list.join(", "),
                });
            }
        }

        let mut scored = Vec::with_capacity(outcome.eligible.len());
        for eligible in &outcome.eligible {
            let mut score = 0.0;
            for (factor, weight) in &spec.factor_weights {
                if let Some(value) = eligible
                    .factors
                    .get(factor)
                    .and_then(|ev| ev.normalized)
                {
                    score += weight * value;
                }
            }
            if !score.is_finite() {
                return Err(SelectorError::Internal {
                    detail: format!("non-finite composed score for {}", eligible.instrument),
                });
            }
            scored.push(ScoredInstrument {
                instrument: eligible.instrument.clone(),
                score,
                factors: eligible.factors.clone(),
            });
        }
        Ok(scored)
    }
}
