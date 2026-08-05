//! `UniverseBuilder` — the first pipeline stage (design §6.6). Assembles the
//! selection context from the Todo 11 quality report, the Todo 12 published
//! universe snapshot, and the Todo 15 factor snapshot, fail-closed:
//!
//! - a `BLOCKED` dataset denies recommendation use with a typed
//!   [`SelectorError::DataBlocked`] (DATA_BLOCKED);
//! - the as-of date must fall inside the universe effective window
//!   ([`SelectorError::AsOfOutsideWindow`]);
//! - the factor snapshot must have been frozen over the SAME published
//!   universe (stale-state guard, [`SelectorError::UniverseMismatch`]).

use domain::TradingDate;
use factor_engine::FactorSnapshot;
use market_data::{DataUse, QualityReport};

use crate::error::SelectorError;
use crate::publish::PublishedSnapshot;

/// The validated selection context handed to the next stage.
#[derive(Debug, Clone)]
pub struct PreparedUniverse<'a> {
    /// The published universe snapshot (Todo 12).
    pub universe: &'a PublishedSnapshot,
    /// The factor snapshot as-of date (the only allowed selection date).
    pub as_of: TradingDate,
    /// Dataset id carried from the factor snapshot.
    pub dataset_id: String,
    /// Dataset version carried from the factor snapshot.
    pub dataset_version: u32,
}

/// Stage 1: gate + window + stale-state checks.
#[derive(Debug, Clone, Copy, Default)]
pub struct UniverseBuilder;

impl UniverseBuilder {
    pub fn new() -> Self {
        Self
    }

    pub fn build<'a>(
        &self,
        quality: &QualityReport,
        universe: &'a PublishedSnapshot,
        factors: &FactorSnapshot,
    ) -> Result<PreparedUniverse<'a>, SelectorError> {
        if let Err(denial) = quality.permits(DataUse::Recommendation) {
            let blocking: Vec<String> = denial
                .blocking_issues
                .iter()
                .map(|code| code.as_str().to_owned())
                .collect();
            return Err(SelectorError::DataBlocked {
                dataset_id: quality.dataset_id.to_string(),
                state: denial.state.to_string(),
                blocking_issues: blocking.join(", "),
            });
        }

        let as_of = factors.as_of;
        if as_of < universe.effective_from
            || universe.effective_until.is_some_and(|until| as_of > until)
        {
            return Err(SelectorError::AsOfOutsideWindow {
                as_of: as_of.to_iso(),
                from: universe.effective_from.to_iso(),
                until: universe
                    .effective_until
                    .map(|d| d.to_iso())
                    .unwrap_or_else(|| "open".to_owned()),
            });
        }

        if factors.universe_snapshot_id != universe.universe_snapshot_id.as_str() {
            return Err(SelectorError::UniverseMismatch {
                snapshot_universe: factors.universe_snapshot_id.clone(),
                published_universe: universe.universe_snapshot_id.as_str().to_owned(),
            });
        }

        Ok(PreparedUniverse {
            universe,
            as_of,
            dataset_id: factors.dataset_id.clone(),
            dataset_version: factors.dataset_version,
        })
    }
}
