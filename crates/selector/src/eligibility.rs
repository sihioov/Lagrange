//! `EligibilityFilter` — the second pipeline stage (design §6.6). Over the
//! frozen universe (canonical order) it binds the factor snapshot rows of the
//! as-of date and excludes instruments with any NULL mandatory factor
//! (design §6.5: "전략별 필수 팩터가 NULL이면 제외"), recording a structured
//! [`Exclusion`] for each.
//!
//! Fail-closed membership: a universe member without an as-of row is a typed
//! [`SelectorError::MissingFactorRow`], and a snapshot row naming an
//! instrument outside the published universe is a typed
//! [`SelectorError::UnknownSnapshotInstrument`] — never a silent skip.

use std::collections::{BTreeMap, BTreeSet};

use domain::InstrumentId;
use factor_engine::FactorSnapshot;

use crate::builder::PreparedUniverse;
use crate::error::SelectorError;
use crate::reason::{Reason, ReasonCode};
use crate::spec::SelectionSpec;

/// Raw + normalized values of one factor for one instrument on the as-of date
/// (FR-SEL-005: "팩터 원값, 정규화 점수").
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct FactorEvidence {
    /// The factor id.
    pub factor: String,
    /// The raw factor value (NULL per the factor's documented policy).
    pub raw: Option<f64>,
    /// The cross-sectionally normalized value.
    pub normalized: Option<f64>,
}

/// One eligible instrument with its bound factor evidence.
#[derive(Debug, Clone, PartialEq)]
pub struct EligibleInstrument {
    pub instrument: InstrumentId,
    /// factor id -> evidence (canonically sorted).
    pub factors: BTreeMap<String, FactorEvidence>,
}

/// A structured exclusion: the instrument, the reason (code + ko/en text),
/// and the missing mandatory factors.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Exclusion {
    pub instrument: InstrumentId,
    pub reason: Reason,
    pub missing_factors: Vec<String>,
}

/// The eligibility outcome: eligible instruments (canonical order) plus the
/// recorded exclusions.
#[derive(Debug, Clone, PartialEq)]
pub struct EligibilityOutcome {
    pub eligible: Vec<EligibleInstrument>,
    pub exclusions: Vec<Exclusion>,
}

/// Stage 2: mandatory-factor NULL exclusions over the frozen universe.
#[derive(Debug, Clone, Copy, Default)]
pub struct EligibilityFilter;

impl EligibilityFilter {
    pub fn new() -> Self {
        Self
    }

    pub fn filter(
        &self,
        prepared: &PreparedUniverse,
        factors: &FactorSnapshot,
        spec: &SelectionSpec,
    ) -> Result<EligibilityOutcome, SelectorError> {
        let as_of = prepared.as_of.to_iso();
        let members: BTreeMap<String, InstrumentId> = prepared
            .universe
            .instruments
            .iter()
            .map(|i| (i.as_str(), i.clone()))
            .collect();

        for row in &factors.rows {
            if row.date == as_of && !members.contains_key(&row.instrument) {
                return Err(SelectorError::UnknownSnapshotInstrument {
                    instrument: row.instrument.clone(),
                    date: as_of,
                });
            }
        }

        let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
        let mut evidence: BTreeMap<String, BTreeMap<String, FactorEvidence>> = BTreeMap::new();
        for row in &factors.rows {
            if row.date != as_of {
                continue;
            }
            let key = (row.instrument.clone(), row.factor.clone());
            if !seen.insert(key.clone()) {
                return Err(SelectorError::Internal {
                    detail: format!("duplicate factor row for {} {} on {}", key.0, key.1, as_of),
                });
            }
            evidence.entry(key.0.clone()).or_default().insert(
                key.1.clone(),
                FactorEvidence {
                    factor: key.1.clone(),
                    raw: row.raw,
                    normalized: row.normalized,
                },
            );
        }

        let mut eligible = Vec::new();
        let mut exclusions = Vec::new();
        for (symbol, instrument) in &members {
            let Some(bound) = evidence.get(symbol) else {
                return Err(SelectorError::MissingFactorRow {
                    instrument: instrument.as_str(),
                    date: as_of,
                });
            };
            let missing: Vec<String> = spec
                .mandatory_factors
                .iter()
                .filter(|f| {
                    !bound
                        .get(*f)
                        .is_some_and(|ev| ev.raw.is_some() && ev.normalized.is_some())
                })
                .cloned()
                .collect();
            if missing.is_empty() {
                eligible.push(EligibleInstrument {
                    instrument: instrument.clone(),
                    factors: bound.clone(),
                });
            } else {
                exclusions.push(Exclusion {
                    instrument: instrument.clone(),
                    reason: Reason::new(
                        ReasonCode::ExcludedMandatoryFactorNull,
                        BTreeMap::from([("factor".to_owned(), missing.join(", "))]),
                    ),
                    missing_factors: missing,
                });
            }
        }

        Ok(EligibilityOutcome {
            eligible,
            exclusions,
        })
    }
}
