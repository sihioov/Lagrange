//! The factor values a backtest needs, computed once per rebalance date.
//!
//! Four of the five baseline strategies decide what to hold from factors:
//! `return_12m`, `momentum_12_1`, `vol_60`, `trend_50`/`trend_200`. Their
//! target generators are Python and their factors are Rust, and until now
//! nothing joined the two — so a baseline backtest had no factors, computed
//! no targets, and placed no orders.
//!
//! # Why the factors are computed HERE and not in the strategy
//!
//! Writing `return_12m` in Python inside the adapter would have been a much
//! smaller change. It would also have created a second definition of every
//! factor, and a backtest would then be measuring something the paper and
//! live paths do not compute. Those paths read the Rust `factor-engine`, and
//! the Paper promotion gate is a PARITY check between backtest and paper — a
//! strategy promoted on a backtest that disagrees with its own live behaviour
//! is worse than one that cannot be backtested at all.
//!
//! So this module calls the same `factor-engine` the rest of the system does,
//! and the adapter receives values rather than a formula.
//!
//! # Raw values, never normalized
//!
//! [`FactorSnapshot`] carries both a raw value and a cross-sectionally
//! normalized one. Only `raw` is emitted, because the generators read these
//! as quantities with units: `dual_momentum` compares `return_12m` against an
//! absolute threshold expressed as a decimal return, so feeding it a z-score
//! would compare a standard-deviation count against `0.0` and silently invert
//! what the strategy means.
//!
//! # The series is also the schedule
//!
//! A backtest rebalances on the dates this produces, so cadence is decided
//! once, here, rather than reimplemented in the adapter. The dates come from
//! the dataset's own sessions, which is what makes them line up exactly with
//! the close events the adapter sees.

use factor_engine::factors::momentum::MomentumFactor;
use factor_engine::factors::returns::ReturnFactor;
use factor_engine::factors::trend::TrendFactor;
use factor_engine::factors::volatility::RealizedVolFactor;
use factor_engine::{Factor, FactorSnapshotBuilder, FrozenUniverse};
use market_data::curate::CurateStore;
use std::collections::BTreeMap;
use std::path::Path;

use crate::phase0::CURATED_VERSION;

/// `date -> instrument -> factor -> raw value`.
pub type FactorSeries = BTreeMap<String, BTreeMap<String, BTreeMap<String, f64>>>;

/// What the dataset says about itself, read once from the curated zone.
#[derive(Debug, Clone)]
pub struct DatasetShape {
    /// Instruments present in the curated bars.
    ///
    /// Taken from the DATA rather than from the request, because the worker
    /// does the same: `simulate.py` overwrites the strategy config's
    /// `instrument_ids` with what it found in the dataset. A universe derived
    /// from anything else would not be the universe the adapter sees.
    pub instruments: Vec<String>,
    /// Every session date present, ascending.
    pub sessions: Vec<String>,
}

#[derive(Debug)]
pub enum FactorSeriesError {
    /// The curated zone could not be read at all.
    Dataset(String),
    /// The engine refused to compute.
    Compute(String),
    /// The dataset is too short for the strategy's declared lookback.
    InsufficientHistory { needed: u64, available: usize },
}

impl std::fmt::Display for FactorSeriesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FactorSeriesError::Dataset(d) => write!(f, "curated dataset unreadable: {d}"),
            FactorSeriesError::Compute(d) => write!(f, "factor computation failed: {d}"),
            FactorSeriesError::InsufficientHistory { needed, available } => write!(
                f,
                "the strategy needs {needed} sessions of history and the dataset has {available}"
            ),
        }
    }
}

/// The factor implementations behind the ids a strategy declares.
///
/// A closed match rather than a lookup by string, so an id no build can
/// compute is a refusal here instead of a column of NULLs the generator would
/// read as "this instrument was excluded".
pub(crate) fn factors_for(ids: &[String]) -> Result<Vec<Box<dyn Factor>>, FactorSeriesError> {
    let mut out: Vec<Box<dyn Factor>> = Vec::new();
    for id in ids {
        let factor: Box<dyn Factor> = match id.as_str() {
            "return_1m" => Box::new(ReturnFactor::one_month()),
            "return_3m" => Box::new(ReturnFactor::three_months()),
            "return_6m" => Box::new(ReturnFactor::six_months()),
            "return_12m" => Box::new(ReturnFactor::twelve_months()),
            "momentum_12_1" => Box::new(MomentumFactor),
            other if other.starts_with("trend_") => Box::new(
                TrendFactor::new(parse_bounded_window(other, "trend_", 5, 500)?)
                    .map_err(compute_err)?,
            ),
            other if other.starts_with("vol_") => Box::new(
                RealizedVolFactor::new(parse_bounded_window(other, "vol_", 2, 252)?)
                    .map_err(compute_err)?,
            ),
            other => {
                return Err(FactorSeriesError::Compute(format!(
                    "no implementation for factor {other:?}"
                )));
            }
        };
        out.push(factor);
    }
    Ok(out)
}

fn parse_bounded_window(
    id: &str,
    prefix: &str,
    minimum: usize,
    maximum: usize,
) -> Result<usize, FactorSeriesError> {
    let suffix = id.strip_prefix(prefix).unwrap_or_default();
    let window = suffix
        .parse::<usize>()
        .map_err(|_| FactorSeriesError::Compute(format!("invalid factor window in {id:?}")))?;
    if suffix != window.to_string() || !(minimum..=maximum).contains(&window) {
        return Err(FactorSeriesError::Compute(format!(
            "factor window in {id:?} must be canonical and within {minimum}..={maximum}"
        )));
    }
    Ok(window)
}

fn compute_err<E: std::fmt::Display>(e: E) -> FactorSeriesError {
    FactorSeriesError::Compute(e.to_string())
}

/// Reads the instruments and session dates out of the curated bars.
pub fn dataset_shape(dataset_root: &Path) -> Result<DatasetShape, FactorSeriesError> {
    let bars_root = dataset_root.join("curated").join("bars").join("market=kr");
    let entries = std::fs::read_dir(&bars_root)
        .map_err(|e| FactorSeriesError::Dataset(format!("list {}: {e}", bars_root.display())))?;

    let mut instruments = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(symbol) = name.strip_prefix("symbol=") {
            instruments.push(symbol.to_string());
        }
    }
    instruments.sort();
    instruments.dedup();

    if instruments.is_empty() {
        return Err(FactorSeriesError::Dataset(format!(
            "no instruments under {}",
            bars_root.display()
        )));
    }

    // Sessions come from the bars of the FIRST instrument, which is enough
    // because the curated zone is a single market on a shared calendar. Using
    // a union across instruments would invent rebalance dates on which some
    // instrument never traded.
    let sessions = sessions_of(dataset_root, &instruments[0])?;
    Ok(DatasetShape {
        instruments,
        sessions,
    })
}

/// Session dates for one symbol, read through the same engine that reads the
/// prices, so a date here is always a date the factor engine also has.
fn sessions_of(dataset_root: &Path, symbol: &str) -> Result<Vec<String>, FactorSeriesError> {
    let store = CurateStore::new(dataset_root);
    let universe = FrozenUniverse::new("session-probe", &[symbol]);
    // A far-future as_of so nothing is filtered: the point is the full span.
    let as_of = domain::TradingDate::parse("9999-12-31")
        .map_err(|e| FactorSeriesError::Dataset(e.to_string()))?;
    let bars = factor_engine::bars::Bars::from_curated(
        &store,
        "kr",
        "dataset",
        CURATED_VERSION,
        &universe,
        as_of,
    )
    .map_err(|e| FactorSeriesError::Dataset(e.to_string()))?;
    let id = universe
        .instruments()
        .next()
        .ok_or_else(|| FactorSeriesError::Dataset("frozen universe is empty".into()))?;
    let points = bars
        .points(id)
        .ok_or_else(|| FactorSeriesError::Dataset(format!("no bars for {symbol}")))?;
    Ok(points.iter().map(|p| p.date.to_string()).collect())
}

/// Month-end sessions at or after the point the lookback is satisfied.
///
/// The first `minimum_lookback` sessions are skipped deliberately. A factor
/// asked for a window it does not have returns NULL, the generator reads that
/// instrument as excluded, and a backtest that started on day one would spend
/// its first year holding cash for a reason no user could see.
fn rebalance_dates(sessions: &[String], minimum_lookback: u64) -> Vec<String> {
    let start = minimum_lookback as usize;
    if sessions.len() <= start {
        return Vec::new();
    }
    let mut dates = Vec::new();
    for (i, date) in sessions.iter().enumerate().skip(start) {
        let this_month = &date[..7.min(date.len())];
        let next_month = sessions.get(i + 1).map(|d| &d[..7.min(d.len())]);
        // The last session of a calendar month, and the last session overall
        // is not one: rebalancing on the final bar would place an order that
        // no later open can fill.
        if next_month.is_some_and(|m| m != this_month) {
            dates.push(date.clone());
        }
    }
    dates
}

/// Computes the whole series from ONE snapshot.
///
/// # Why one snapshot and not one per rebalance date
///
/// A snapshot carries a row for every session it computed over, not only for
/// its own as-of date — 260 dates from a single build on the phase-0 zone.
/// Building it once at the LAST session and slicing out the rebalance dates
/// is therefore one disk read instead of a dozen.
///
/// It is also the only shape that works. [`Bars::from_curated`] REFUSES a
/// store containing any bar after `as_of`, which is right for the live path
/// (a bar after today means the zone is corrupt) and impossible for a
/// walk-forward, where every historical as-of has later data behind it by
/// definition.
///
/// This does not weaken the no-look-ahead guarantee, because that guarantee
/// does not live in the refusal. Every factor is a TRAILING computation —
/// "the reference bar of a month window is the LAST bar on or before the
/// calendar target date; a bar after the target is never used" — so a value
/// for date `d` is the same whether the snapshot ends at `d` or later. That
/// is a property this design depends on, so it is asserted rather than
/// quoted: see `a_factor_value_does_not_change_when_later_bars_exist`.
pub fn build(
    dataset_root: &Path,
    shape: &DatasetShape,
    required_factors: &[String],
    minimum_lookback: u64,
) -> Result<FactorSeries, FactorSeriesError> {
    let dates = rebalance_dates(&shape.sessions, minimum_lookback);
    if dates.is_empty() {
        return Err(FactorSeriesError::InsufficientHistory {
            needed: minimum_lookback,
            available: shape.sessions.len(),
        });
    }
    let wanted: std::collections::BTreeSet<&str> = dates.iter().map(|d| d.as_str()).collect();

    let last = shape
        .sessions
        .last()
        .ok_or_else(|| FactorSeriesError::Dataset("dataset has no sessions".into()))?;
    let as_of = domain::TradingDate::parse(last)
        .map_err(|e| FactorSeriesError::Compute(format!("{last}: {e}")))?;

    let store = CurateStore::new(dataset_root);
    let symbols: Vec<&str> = shape.instruments.iter().map(|s| s.as_str()).collect();
    let snapshot = FactorSnapshotBuilder::new(
        as_of,
        FrozenUniverse::new("backtest-universe", &symbols),
        &store,
        "kr",
        "dataset",
        CURATED_VERSION,
    )
    .with_factors(factors_for(required_factors)?)
    .build()
    .map_err(|e| FactorSeriesError::Compute(e.to_string()))?;

    let mut series: FactorSeries = BTreeMap::new();
    for date in &dates {
        series.insert(date.clone(), BTreeMap::new());
    }
    for row in snapshot.rows {
        if !wanted.contains(row.date.as_str()) {
            continue;
        }
        // A NULL is an exclusion, not a zero. Omitting the key lets
        // `generate_target` apply its own documented null policy instead of
        // reading a fabricated value.
        let Some(raw) = row.raw else { continue };
        if !raw.is_finite() {
            continue;
        }
        series
            .entry(row.date)
            .or_default()
            .entry(row.instrument)
            .or_default()
            .insert(row.factor, raw);
    }
    Ok(series)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rebalance_never_lands_on_the_last_session() {
        // An order placed on the final bar has no later open to fill it, so
        // the run would end with a target that never became a position.
        let sessions: Vec<String> = ["2020-01-30", "2020-01-31", "2020-02-27", "2020-02-28"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let dates = rebalance_dates(&sessions, 0);
        assert_eq!(dates, vec!["2020-01-31".to_string()]);
    }

    #[test]
    fn the_lookback_is_skipped_rather_than_computed_as_null() {
        // With a lookback longer than the data there is no honest date to
        // rebalance on, and an empty schedule is what says so.
        let sessions: Vec<String> = (1..=5).map(|d| format!("2020-01-{d:02}")).collect();
        assert!(rebalance_dates(&sessions, 252).is_empty());
    }

    #[test]
    fn an_unknown_factor_id_is_refused_rather_than_silently_dropped() {
        // Dropping it would produce a series missing exactly the column the
        // generator needs, which reads downstream as "every instrument was
        // excluded" -- a backtest holding cash for an invisible reason.
        // Matched rather than unwrapped: `dyn Factor` is not `Debug`, so the
        // Ok side cannot be printed.
        let Err(err) = factors_for(&["not_a_factor".to_string()]) else {
            panic!("an unknown factor id must be refused");
        };
        assert!(matches!(err, FactorSeriesError::Compute(_)), "{err:?}");
    }

    #[test]
    fn parameterized_factor_ids_are_canonical_and_bounded() {
        for id in ["trend_0", "trend_050", "trend_501", "vol_1", "vol_999999"] {
            assert!(
                factors_for(&[id.to_owned()]).is_err(),
                "invalid or unbounded factor id {id:?} must be refused"
            );
        }
        for id in [
            "trend_50",
            "trend_100",
            "trend_200",
            "vol_20",
            "vol_60",
            "vol_120",
        ] {
            factors_for(&[id.to_owned()])
                .unwrap_or_else(|error| panic!("documented factor {id:?}: {error}"));
        }
    }

    /// The phase-0 curated zone, when a developer has generated it.
    ///
    /// `None` rather than a failure on a checkout that has not run the
    /// generator, because `data/phase0` is a build artifact and is not
    /// tracked.
    fn phase0_root() -> Option<std::path::PathBuf> {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()?
            .parent()?
            .join("data/phase0/curated");
        root.join("curated/bars/market=kr").is_dir().then_some(root)
    }

    /// Copies only the `year=<year>` partitions into a fresh store.
    ///
    /// The point is a store that genuinely ENDS earlier, which is the only
    /// way to ask what a factor would have been computed from at the time —
    /// `from_curated` refuses any store holding a later bar, so pointing a
    /// past as-of at the full zone tests nothing.
    fn store_truncated_to_year(src: &std::path::Path, year: &str) -> tempfile::TempDir {
        let dst = tempfile::tempdir().expect("tempdir");
        let market = src.join("curated/bars/market=kr");
        for symbol in std::fs::read_dir(&market).expect("market dir").flatten() {
            let from = symbol
                .path()
                .join(year)
                .join(format!("version={CURATED_VERSION}"));
            if !from.is_dir() {
                continue;
            }
            let to = dst
                .path()
                .join("curated/bars/market=kr")
                .join(symbol.file_name())
                .join(year)
                .join(format!("version={CURATED_VERSION}"));
            std::fs::create_dir_all(&to).expect("mkdir");
            for file in std::fs::read_dir(&from).expect("partition").flatten() {
                std::fs::copy(file.path(), to.join(file.file_name())).expect("copy");
            }
        }
        dst
    }

    fn trend_50_at(root: &std::path::Path, as_of: &str, date: &str) -> Option<f64> {
        let store = CurateStore::new(root);
        FactorSnapshotBuilder::new(
            domain::TradingDate::parse(as_of).expect("date"),
            FrozenUniverse::new("u", &["069500.KRX", "114260.KRX", "229200.KRX"]),
            &store,
            "kr",
            "dataset",
            CURATED_VERSION,
        )
        .with_factors(factors_for(&["trend_50".to_string()]).expect("factor"))
        .build()
        .expect("snapshot")
        .rows
        .into_iter()
        .find(|r| r.instrument == "069500.KRX" && r.date == date)
        .and_then(|r| r.raw)
    }

    #[test]
    fn the_hand_computed_factor_value_is_what_the_engine_returns() {
        // Pinned against arithmetic done by hand from the curated closes:
        // close / mean(last 50 closes) - 1 = 137800000 / 127422800 - 1.
        //
        // Without this, every other assertion here only proves the engine is
        // self-consistent -- including with a definition that disagrees with
        // what the Python generators expect their factors to mean.
        let Some(root) = phase0_root() else {
            eprintln!("SKIP: data/phase0 has not been generated");
            return;
        };
        let value = trend_50_at(&root, "2021-01-29", "2020-12-30").expect("value");
        assert!(
            (value - 0.081_439_114_506_979_94).abs() < 1e-12,
            "trend_50 drifted from the hand-computed value: {value}"
        );
    }

    #[test]
    fn a_factor_value_does_not_change_when_later_bars_exist() {
        // The property the whole design rests on.
        //
        // One snapshot is built at the LAST session and sliced per rebalance
        // date, because `Bars::from_curated` refuses a store holding any bar
        // after its as-of -- correct for the live path, impossible for a
        // walk-forward. That is only sound if every factor is TRAILING, so
        // that the value for date `d` is identical whether the snapshot ends
        // at `d` or a year later.
        //
        // If a factor ever stops being trailing, this fails and the whole
        // series becomes look-ahead contaminated in a way no downstream
        // assertion would notice.
        let Some(root) = phase0_root() else {
            eprintln!("SKIP: data/phase0 has not been generated");
            return;
        };
        // A store that genuinely ends in 2020, and the full one that runs on
        // into 2021.
        let truncated = store_truncated_to_year(&root, "year=2020");
        let shape_2020 = dataset_shape(truncated.path()).expect("truncated shape");
        let probe_date = shape_2020
            .sessions
            .last()
            .expect("the truncated store has sessions")
            .clone();
        assert!(
            probe_date.starts_with("2020-"),
            "the truncation must actually cut the store short, got {probe_date}"
        );

        let without_later_bars =
            trend_50_at(truncated.path(), &probe_date, &probe_date).expect("value when it was new");
        let with_later_bars =
            trend_50_at(&root, "2021-01-29", &probe_date).expect("value inside a longer snapshot");

        assert_eq!(
            without_later_bars, with_later_bars,
            "trend_50 on {probe_date} changed once later bars existed -- the \
             factor is not trailing, and slicing one snapshot into a series \
             would leak the future into every rebalance"
        );
    }

    #[test]
    fn the_series_covers_exactly_the_rebalance_dates() {
        let Some(root) = phase0_root() else {
            return;
        };
        let shape = dataset_shape(&root).expect("shape");
        let series = build(&root, &shape, &["vol_60".to_string()], 60).expect("series");
        let expected = rebalance_dates(&shape.sessions, 60);
        assert_eq!(
            series.keys().cloned().collect::<Vec<_>>(),
            expected,
            "the series is the rebalance schedule, so its keys are the dates"
        );
        // Every date must actually carry values, or the schedule promises a
        // rebalance the generator cannot perform.
        for (date, per_instrument) in &series {
            assert!(
                !per_instrument.is_empty(),
                "{date} is scheduled but carries no factor values"
            );
        }
    }

    #[test]
    fn a_dataset_too_short_for_the_lookback_says_so() {
        // Phase-0 is 260 sessions and the 12-month strategies need 252, which
        // leaves no month-end that is not also the final session. The error
        // names both numbers so an operator can see it is the DATA that is
        // short, not the strategy that is broken.
        let Some(root) = phase0_root() else {
            return;
        };
        let shape = dataset_shape(&root).expect("shape");
        let err = build(&root, &shape, &["return_12m".to_string()], 252).unwrap_err();
        let FactorSeriesError::InsufficientHistory { needed, available } = err else {
            panic!("expected InsufficientHistory, got {err:?}");
        };
        assert_eq!(needed, 252);
        assert_eq!(available, shape.sessions.len());
    }

    #[test]
    fn every_baseline_required_factor_has_an_implementation() {
        // The ids the five packages declare, mirrored from
        // `selector::baseline`. A factor a strategy needs and this build
        // cannot compute must fail HERE rather than at run time in a child
        // process.
        for id in [
            "return_12m",
            "momentum_12_1",
            "vol_60",
            "trend_50",
            "trend_200",
        ] {
            factors_for(&[id.to_string()])
                .unwrap_or_else(|e| panic!("{id} must be computable: {e}"));
        }
    }
}
