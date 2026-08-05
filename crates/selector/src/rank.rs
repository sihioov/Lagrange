//! `Ranker` — the fourth pipeline stage (design §6.6). Sorts the scored
//! instruments deterministically: score descending, ties broken by canonical
//! `InstrumentId` (FR-SEL-003: "동일 입력과 설정은 동일 선정 결과를 반환").
//! Ranks are 1-based and contiguous over the eligible set.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use domain::InstrumentId;

use crate::eligibility::FactorEvidence;
use crate::score::ScoredInstrument;

/// One eligible instrument with its final rank.
#[derive(Debug, Clone, PartialEq)]
pub struct RankedInstrument {
    pub instrument: InstrumentId,
    pub rank: usize,
    pub score: f64,
    pub factors: BTreeMap<String, FactorEvidence>,
}

/// Stage 4: deterministic score ranking with canonical-ID tie-break.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ranker;

impl Ranker {
    pub fn new() -> Self {
        Self
    }

    pub fn rank(&self, mut scored: Vec<ScoredInstrument>) -> Vec<RankedInstrument> {
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(Ordering::Equal)
                .then_with(|| a.instrument.cmp(&b.instrument))
        });
        scored
            .into_iter()
            .enumerate()
            .map(|(index, s)| RankedInstrument {
                instrument: s.instrument,
                rank: index + 1,
                score: s.score,
                factors: s.factors,
            })
            .collect()
    }
}
