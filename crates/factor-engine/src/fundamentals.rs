//! Point-in-time fundamentals: what was KNOWN on a date, not what is true now.
//!
//! [`crate::bars`] enforces one temporal rule -- no bar after `as_of`. That is
//! enough for prices, which are never restated. A financial statement is
//! restated, and it carries two dates: the period it describes and the date it
//! became public. A table with only the first is how §14's 미래정보 참조 turns
//! into 허위 성과 without anything looking wrong: a 2021 correction quietly
//! improves every 2020 decision, and the backtest reports a strategy that
//! could not have been run.
//!
//! # The rule, and why it is not one as-of join
//!
//! Resolution happens PER BAR DATE, not once per snapshot. A snapshot dated
//! 2021 that resolved fundamentals once would hand its 2021-restated figure to
//! every 2020 bar -- exactly the bias this module exists to prevent. So:
//!
//! - the snapshot ceiling drops anything with `known_from > as_of` (nothing
//!   later than the snapshot is loaded at all), and
//! - for a given bar date `D`, [`Fundamentals::value_on`] answers with what a
//!   reader standing on `D` would have seen.
//!
//! Standing on `D` means, in order:
//!
//! 1. keep only rows with `known_from <= D` -- an announcement is invisible
//!    before it is announced;
//! 2. among those, take the LATEST `period_end`; then
//! 3. within that period, the latest `known_from`, breaking ties by the higher
//!    `revision`.
//!
//! Step 2 before step 3 is the subtle half. Ordering by `known_from` alone
//! looks equivalent and is not: a Q1 restatement published in September would
//! outrank Q2's August original and the strategy would act on stale-period
//! data it never chose. `period_end` decides WHICH period is current;
//! `known_from` decides which version of that period is visible.

use std::collections::BTreeMap;

use domain::{InstrumentId, TradingDate};
use market_data::CurateStore;
use market_data::curate::schema::read_fundamentals;

use crate::contract::FactorError;
use crate::snapshot::FrozenUniverse;

/// One observation, already inside the snapshot ceiling.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FundamentalPoint {
    pub period_end: TradingDate,
    pub value: f64,
    pub known_from: TradingDate,
    pub revision: i64,
}

/// The point-in-time fundamentals of one snapshot's universe.
#[derive(Debug, Clone, Default)]
pub struct Fundamentals {
    as_of: Option<TradingDate>,
    /// `(instrument, metric)` -> observations, unordered by construction and
    /// scanned per query. Kept simple deliberately: correctness of the
    /// three-step rule is the deliverable here, and a skeleton that is fast
    /// and subtly wrong is worth less than nothing.
    rows: BTreeMap<(InstrumentId, String), Vec<FundamentalPoint>>,
}

impl Fundamentals {
    /// Reads the universe's fundamentals zone, applying the snapshot ceiling.
    ///
    /// A missing zone is EMPTY, not an error. Fundamentals are an optional
    /// input that no shipped factor consumes yet, and a dataset without them
    /// must keep building the snapshots it always built.
    ///
    /// A row with `known_from > as_of` is skipped rather than rejected --
    /// deliberately unlike [`crate::bars`], which types a future-dated bar as
    /// an error. The zone is one file per instrument covering all history, so
    /// rows the snapshot cannot yet see are the normal case, not corruption.
    /// The invisibility is the requirement; refusing to load the file would
    /// only mean every snapshot needed its own copy of it.
    pub fn from_curated(
        store: &CurateStore,
        market: &str,
        version: u32,
        universe: &FrozenUniverse,
        as_of: TradingDate,
    ) -> Result<Self, FactorError> {
        let mut rows: BTreeMap<(InstrumentId, String), Vec<FundamentalPoint>> = BTreeMap::new();
        for id in universe.instruments() {
            let path = store.fundamentals_path(market, &id.to_string(), version);
            if !path.exists() {
                continue;
            }
            let read = read_fundamentals(&path).map_err(|e| FactorError::StoreIo {
                context: format!("read {}", path.display()),
                detail: e.to_string(),
            })?;
            for row in read {
                // The snapshot ceiling.
                if row.known_from > as_of {
                    continue;
                }
                // Only instruments inside the frozen universe are read at all,
                // and a file must not smuggle in another instrument's rows.
                if &row.instrument_id != id {
                    return Err(FactorError::InvalidDefinition {
                        detail: format!(
                            "fundamentals for {id} contain a row for {}",
                            row.instrument_id
                        ),
                    });
                }
                rows.entry((id.clone(), row.metric.clone()))
                    .or_default()
                    .push(FundamentalPoint {
                        period_end: row.period_end,
                        value: row.value,
                        known_from: row.known_from,
                        revision: row.revision,
                    });
            }
        }
        Ok(Self {
            as_of: Some(as_of),
            rows,
        })
    }

    /// The snapshot ceiling this was resolved under, if any.
    pub fn as_of(&self) -> Option<TradingDate> {
        self.as_of
    }

    /// The value a reader standing on `date` would have seen, or `None`.
    ///
    /// `None` means "not knowable then" -- before the first announcement, or
    /// for a metric this instrument does not report. It is deliberately not a
    /// zero or a carried-forward default: a factor that silently reads 0 for
    /// "no earnings reported yet" ranks that instrument as though it had
    /// reported, and the caller can no longer tell the two apart.
    ///
    /// `known_from == date` is VISIBLE. The field is defined as the first
    /// trading date the value may be used, so excluding its own date would
    /// delay every figure by a day.
    pub fn value_on(&self, id: &InstrumentId, metric: &str, date: TradingDate) -> Option<f64> {
        self.point_on(id, metric, date).map(|p| p.value)
    }

    /// [`Self::value_on`] with the provenance kept, for callers that need to
    /// say which period and which revision produced a number.
    pub fn point_on(
        &self,
        id: &InstrumentId,
        metric: &str,
        date: TradingDate,
    ) -> Option<FundamentalPoint> {
        let points = self.rows.get(&(id.clone(), metric.to_string()))?;
        // Step 1: nothing announced after `date` exists yet.
        let visible = points.iter().filter(|p| p.known_from <= date);
        // Steps 2 and 3, as one ordering: the latest PERIOD first, and only
        // then the latest revision OF that period. Swapping these two keys is
        // the bug this ordering exists to prevent -- a Q1 restatement would
        // outrank Q2's original and the strategy would act on the older
        // period.
        visible
            .max_by(|a, b| {
                a.period_end
                    .cmp(&b.period_end)
                    .then(a.known_from.cmp(&b.known_from))
                    .then(a.revision.cmp(&b.revision))
            })
            .copied()
    }

    /// The metrics carried for an instrument, in canonical order.
    pub fn metrics(&self, id: &InstrumentId) -> Vec<&str> {
        self.rows
            .keys()
            .filter(|(i, _)| i == id)
            .map(|(_, m)| m.as_str())
            .collect()
    }

    /// Whether any observation was resolved at all.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}
