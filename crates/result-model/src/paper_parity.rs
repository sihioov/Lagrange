//! Backtest-vs-Paper signal parity (plan Todo 32; design §10.2, §15.3;
//! requirements FR-PAPER-004, FR-RPT-001).
//!
//! Parity answers one question honestly: **did the Paper account act on the
//! same signals the backtest did?** That has three outcomes, not two —
//! [`ParityStatus::NotComparable`] exists because reporting a lineage
//! mismatch as a "divergence" would misdescribe what was actually compared.
//!
//! ## What the lineage covers, and what it deliberately does not
//!
//! Comparable requires identical `strategy_id`, `strategy_version`,
//! `dataset_version`, and `as_of`. It does NOT include `engine` /
//! `engine_version`: the backtest runs on NautilusTrader while the Paper
//! runner models the session open itself (Todo 31), so an engine difference
//! is EXPECTED. Folding it into the lineage check would make every real
//! report `NotComparable` and the feature useless. That expected difference
//! is instead surfaced explicitly and unconditionally through
//! [`ParityReport::fill_model_difference`], which is design §10.2's
//! requirement to show the fill-model difference rather than smooth it over.
//!
//! ## Exactness
//!
//! Both sides' weights come from the same deterministic selector at scale 6,
//! so comparison is exact equality. A tolerance would only hide bugs the
//! golden guarantees already exclude.

use std::collections::{BTreeMap, BTreeSet};

use domain::provenance::RunProvenance;
use domain::{InstrumentId, Weight};
use serde::{Deserialize, Serialize};

/// The documented, expected execution difference between the two sides.
/// Stated on EVERY report — a reader must never assume the two executions
/// are interchangeable (design §10.2).
pub const FILL_MODEL_DIFFERENCE: &str = "Backtest fills come from the NautilusTrader engine's \
     execution model; Paper fills are modeled at the next session's raw open plus the configured \
     slippage. Identical signals can therefore still produce different fills, quantities, and \
     realized costs.";

/// One side of a parity comparison: the signals a run produced plus the
/// provenance that decides whether they are comparable at all.
#[derive(Debug, Clone, PartialEq)]
pub struct SignalSet {
    /// The run's provenance (strategy/data/engine versions).
    pub provenance: RunProvenance,
    /// The signal date both sides must share (`YYYY-MM-DD`).
    pub as_of: String,
    /// Target weight per instrument, canonical order.
    pub targets: BTreeMap<InstrumentId, Weight>,
}

/// The outcome of a parity comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ParityStatus {
    /// Same lineage AND identical signals.
    Match,
    /// Same lineage, different signals — a real finding to explain.
    Divergent,
    /// Different strategy/data/as-of inputs: no parity claim is meaningful.
    NotComparable,
}

impl ParityStatus {
    /// The stable wire value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Match => "MATCH",
            Self::Divergent => "DIVERGENT",
            Self::NotComparable => "NOT_COMPARABLE",
        }
    }
}

/// One lineage field compared across the two sides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineageField {
    pub field: String,
    pub backtest: String,
    pub paper: String,
}

impl LineageField {
    fn agrees(&self) -> bool {
        self.backtest == self.paper
    }
}

/// The field-by-field lineage comparison behind the status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineageComparison {
    pub fields: Vec<LineageField>,
}

impl LineageComparison {
    /// Whether every compared field agrees (i.e. the sides are comparable).
    pub fn matches(&self) -> bool {
        self.fields.iter().all(LineageField::agrees)
    }

    /// The names of the fields that differ — the report's "divergence
    /// reason" for a [`ParityStatus::NotComparable`] outcome.
    pub fn mismatched_fields(&self) -> Vec<&str> {
        self.fields
            .iter()
            .filter(|f| !f.agrees())
            .map(|f| f.field.as_str())
            .collect()
    }
}

/// One instrument whose target weight differs across the two sides.
/// `None` means the side did not target that instrument at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalDivergence {
    pub instrument_id: InstrumentId,
    pub backtest_weight: Option<Weight>,
    pub paper_weight: Option<Weight>,
}

/// The full parity report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParityReport {
    pub status: ParityStatus,
    pub lineage: LineageComparison,
    /// Empty unless [`ParityStatus::Divergent`]: an incomparable report
    /// makes no signal claims at all.
    pub divergences: Vec<SignalDivergence>,
    /// The expected execution difference, stated unconditionally.
    pub fill_model_difference: String,
}

impl ParityReport {
    /// Whether this report is worth raising a WARNING-grade alert for
    /// (design §15.3 grades "Paper 불일치" as WARNING → web + admin).
    ///
    /// Both a divergence and an incomparable lineage qualify: the latter
    /// means the Paper account drifted off the strategy/data it was bound
    /// to, which is at least as important to surface.
    pub fn warrants_alert(&self) -> bool {
        !matches!(self.status, ParityStatus::Match)
    }
}

/// Compares a backtest's signals against a Paper account's for the same
/// session.
///
/// Never fails: an unusable comparison is a reported
/// [`ParityStatus::NotComparable`] with the offending fields named, not an
/// error the caller has to interpret.
pub fn evaluate_parity(backtest: &SignalSet, paper: &SignalSet) -> ParityReport {
    let lineage = compare_lineage(backtest, paper);
    if !lineage.matches() {
        return ParityReport {
            status: ParityStatus::NotComparable,
            lineage,
            divergences: Vec::new(),
            fill_model_difference: FILL_MODEL_DIFFERENCE.to_owned(),
        };
    }

    let instruments: BTreeSet<&InstrumentId> = backtest
        .targets
        .keys()
        .chain(paper.targets.keys())
        .collect();
    let divergences: Vec<SignalDivergence> = instruments
        .into_iter()
        .filter_map(|id| {
            let b = backtest.targets.get(id).copied();
            let p = paper.targets.get(id).copied();
            (b != p).then(|| SignalDivergence {
                instrument_id: id.clone(),
                backtest_weight: b,
                paper_weight: p,
            })
        })
        .collect();

    ParityReport {
        status: if divergences.is_empty() {
            ParityStatus::Match
        } else {
            ParityStatus::Divergent
        },
        lineage,
        divergences,
        fill_model_difference: FILL_MODEL_DIFFERENCE.to_owned(),
    }
}

/// The lineage fields that decide comparability.
///
/// `engine`/`engine_version` are deliberately absent — see the module docs.
fn compare_lineage(backtest: &SignalSet, paper: &SignalSet) -> LineageComparison {
    LineageComparison {
        fields: vec![
            LineageField {
                field: "strategy_id".to_owned(),
                backtest: backtest.provenance.strategy_id.to_string(),
                paper: paper.provenance.strategy_id.to_string(),
            },
            LineageField {
                field: "strategy_version".to_owned(),
                backtest: backtest.provenance.strategy_version.to_string(),
                paper: paper.provenance.strategy_version.to_string(),
            },
            LineageField {
                field: "dataset_version".to_owned(),
                backtest: backtest.provenance.dataset_version.to_string(),
                paper: paper.provenance.dataset_version.to_string(),
            },
            LineageField {
                field: "as_of".to_owned(),
                backtest: backtest.as_of.clone(),
                paper: paper.as_of.clone(),
            },
        ],
    }
}
