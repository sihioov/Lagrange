//! `selector` - Lagrange Station fixed-universe selector and constrained target portfolios.
//!
//! Todo 12 delivers the **fixed Korean ETF v1 universe** as a versioned,
//! immutable snapshot ([`universe`], [`publish`]):
//!
//! - [`universe::parse_manifest`] parses `configs/universes/kr-etf-core-v1.yaml`
//!   into a typed [`universe::UniverseManifest`] (exact canonical
//!   `InstrumentId = {symbol}.KRX` entries, benchmark, KRW base currency,
//!   unleveraged/non-inverse eligibility, effective window, KRX source
//!   snapshot) — malformed YAML, unknown fields, and inverted windows are
//!   typed [`universe::UniverseError`] errors, never panics;
//! - [`universe::UniverseManifest::canonical_hash`] computes the immutable
//!   `universe_snapshot_id` (SHA-256 over the canonical manifest): repeated
//!   builds hash identically, a changed manifest yields a new id;
//! - [`publish::UniversePublisher`] resolves every member against the
//!   instrument master (Todo 9), the entitlement gate (Todo 5), and the
//!   required-dataset quality report (Todo 11). Publication BLOCKS — typed
//!   error naming the exact instrument + reason — when an id is inactive,
//!   unsupported (asset class / leverage / inverse), duplicated, or
//!   unlicensed; it NEVER substitutes a different product automatically.
//!
//! Todo 16 (target portfolios) builds directly on the published snapshot.
//!
//! The selection pipeline (design §6.6, FR-SEL-003/004/005):
//!
//! ```text
//! UniverseBuilder      -> quality gate (BLOCKED = typed DATA_BLOCKED denial)
//!                         + as-of window + stale-state universe-id checks
//! EligibilityFilter    -> mandatory-factor NULL exclusions over the frozen
//!                         universe (canonical order)
//! FactorSnapshot       -> the consumed snapshot (raw + normalized values per
//!                         date; never recomputed here)
//! ScoreComposer        -> weighted normalized scores
//! Ranker               -> score-descending ranks with canonical InstrumentId
//!                         tie-break
//! PortfolioConstraints -> top-N target weights under max-weight / cash-floor
//!                         with deterministic rounding-residue-to-cash
//! TargetPortfolio      -> targets + cash + reasons + snapshot/provenance ids
//! ```
//!
//! The selector outputs **targets only**: it never creates orders. There are
//! no order / fill / quantity types in this crate's output; downstream
//! execution layers (backtest / Paper / Live) translate target weights into
//! orders (design §6.6: "Selector는 주문을 직접 만들지 않는다").

pub mod baseline;
pub mod builder;
pub mod eligibility;
pub mod error;
pub mod portfolio;
pub mod publish;
pub mod rank;
pub mod reason;
pub mod registry;
pub mod score;
pub mod spec;
pub mod target;
pub mod universe;

pub use builder::{PreparedUniverse, UniverseBuilder};
pub use eligibility::{EligibilityFilter, EligibilityOutcome, EligibleInstrument, Exclusion};
pub use error::SelectorError;
pub use portfolio::{PortfolioConstraints, WeightAssignment};
pub use publish::{ProductKind, PublishedSnapshot, UniversePublisher};
pub use rank::{RankedInstrument, Ranker};
pub use score::{ScoreComposer, ScoredInstrument};
pub use target::{ConstraintSummary, TargetPortfolio, TargetRow};
pub use universe::{
    Eligibility, SourceSnapshot, UniverseError, UniverseInstrumentEntry, UniverseManifest,
    UniverseSpec, parse_manifest,
};

use factor_engine::FactorSnapshot;
use market_data::QualityReport;

use crate::spec::SelectionSpec;

/// Runs the full documented pipeline over one frozen universe + factor
/// snapshot, returning a complete explainable target portfolio or a typed
/// error — never a partial result.
pub fn select_targets(
    spec: &SelectionSpec,
    quality: &QualityReport,
    universe: &PublishedSnapshot,
    factors: &FactorSnapshot,
) -> Result<TargetPortfolio, SelectorError> {
    let prepared = UniverseBuilder::new().build(quality, universe, factors)?;
    let outcome = EligibilityFilter::new().filter(&prepared, factors, spec)?;
    let scored = ScoreComposer::new().compose(&outcome, factors, spec)?;
    let ranked = Ranker::new().rank(scored);
    let assigned = PortfolioConstraints::from_spec(spec).apply(&ranked)?;

    let targets: Vec<TargetRow> = ranked
        .iter()
        .map(|r| TargetRow {
            instrument_id: r.instrument.clone(),
            rank: r.rank,
            score: r.score,
            factors: r.factors.clone(),
            target_weight: assigned.weights[&r.instrument],
            reasons: assigned.target_reasons[&r.instrument].clone(),
        })
        .collect();

    let portfolio = TargetPortfolio {
        as_of: prepared.as_of,
        strategy_version: format!("{}@{}", spec.strategy_id, spec.strategy_version),
        universe_snapshot_id: universe.universe_snapshot_id.as_str().to_owned(),
        factor_snapshot_hash: factors.hash.as_str().to_owned(),
        dataset_id: prepared.dataset_id,
        dataset_version: prepared.dataset_version,
        targets,
        exclusions: outcome.exclusions,
        cash_weight: assigned.cash_weight,
        constraints: ConstraintSummary {
            top_n: spec.top_n,
            max_weight: spec.max_weight,
            cash_floor: spec.cash_floor,
            weight_scale: spec.weight_scale,
            tolerance: spec.tolerance,
        },
        portfolio_reasons: assigned.portfolio_reasons,
        portfolio_snapshot_id: String::new(),
    };
    let id = portfolio.compute_portfolio_snapshot_id()?;
    Ok(TargetPortfolio {
        portfolio_snapshot_id: id.as_str().to_owned(),
        ..portfolio
    })
}
