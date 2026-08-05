//! Input layer: resolves the frozen universe's curated bars (Todo 10 layout
//! `data/curated/bars/market={m}/symbol={s}/year={y}/version={v}/...`) into a
//! typed series plus a deterministic Polars LazyFrame.
//!
//! Resolution rules (documented):
//! - only instruments INSIDE the frozen universe are read (the universe is
//!   the input gate; out-of-universe rows are never even opened);
//! - a bar dated after `as_of` is a typed [`FactorError::FutureDatedRow`];
//! - signals use the split-adjusted close; volume/trading value come from the
//!   raw table (never adjusted, per the curated schema docs);
//! - calendar month targets (`target_1m` ... `target_12m`) are resolved here
//!   as pure calendar arithmetic (see [`crate::months`]); the factor MATH on
//!   top of them is polars-lazy.

use std::collections::BTreeMap;
use std::fs;

use chrono::{Datelike, NaiveDate};
use domain::{InstrumentId, TradingDate};
use market_data::CurateStore;
use market_data::curate::schema::{read_adjusted_bars, read_bars};
use polars::prelude::*;

use crate::contract::{FactorError, Field};
use crate::months::{month_end, months_back};
use crate::snapshot::FrozenUniverse;

/// The documented calendar month windows of the return factors.
pub const MONTH_WINDOWS: [u32; 4] = [1, 3, 6, 12];

/// One resolved bar of an instrument (signal series).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarPoint {
    pub date: TradingDate,
    /// Split-adjusted close (f64, finite by construction of curated prices).
    pub close: f64,
    /// Reported trading value (raw, never adjusted), if the provider reported it.
    pub trading_value: Option<f64>,
}

/// The resolved input for one snapshot build.
#[derive(Clone)]
pub struct Bars {
    as_of: TradingDate,
    series: BTreeMap<InstrumentId, Vec<BarPoint>>,
    frame: LazyFrame,
}

impl Bars {
    /// Resolves the frozen universe's curated bars for `dataset_version`.
    ///
    /// Year partitions are discovered deterministically (sorted), and every
    /// row is checked against `as_of` (future rows are typed rejections).
    pub fn from_curated(
        store: &CurateStore,
        market: &str,
        _dataset_id: &str,
        version: u32,
        universe: &FrozenUniverse,
        as_of: TradingDate,
    ) -> Result<Self, FactorError> {
        let mut series = BTreeMap::new();
        for id in universe.instruments() {
            let points = load_symbol(store, market, id, version, as_of)?;
            series.insert(id.clone(), points);
        }
        let frame = build_frame(&series)?;
        Ok(Self {
            as_of,
            series,
            frame,
        })
    }

    /// The snapshot as-of date.
    pub fn as_of(&self) -> TradingDate {
        self.as_of
    }

    /// The resolved instruments, in canonical (sorted) order.
    pub fn instruments(&self) -> impl Iterator<Item = &InstrumentId> {
        self.series.keys()
    }

    /// The typed points of one instrument (sorted by date).
    pub fn points(&self, id: &InstrumentId) -> Option<&[BarPoint]> {
        self.series.get(id).map(Vec::as_slice)
    }

    /// The deterministic lazy frame used by every factor:
    /// `instrument_id`, `trading_date`, `close`, `trading_value`, plus the
    /// month target columns `target_1m|3m|6m|12m` (Date32, nullable).
    pub fn lazy_frame(&self) -> LazyFrame {
        self.frame.clone()
    }

    /// The fields this input layer carries.
    pub fn available_fields(&self) -> &'static [Field] {
        &[Field::CLOSE, Field::TRADING_VALUE]
    }
}

fn load_symbol(
    store: &CurateStore,
    market: &str,
    id: &InstrumentId,
    version: u32,
    as_of: TradingDate,
) -> Result<Vec<BarPoint>, FactorError> {
    let symbol = id.to_string();
    let symbol_dir = store
        .root()
        .join("curated")
        .join("bars")
        .join(format!("market={market}"))
        .join(format!("symbol={symbol}"));
    let mut years: Vec<i32> = Vec::new();
    let entries = fs::read_dir(&symbol_dir).map_err(|e| FactorError::StoreIo {
        context: format!("list {}", symbol_dir.display()),
        detail: e.to_string(),
    })?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(rest) = name.strip_prefix("year=")
            && let Ok(year) = rest.parse::<i32>()
        {
            years.push(year);
        }
    }
    years.sort_unstable();

    let mut points: Vec<BarPoint> = Vec::new();
    for year in years {
        let bars_path = store.bars_path(market, &symbol, year, version);
        let dir = bars_path.parent().expect("bars path has a parent");
        if !dir.join("bars.parquet").exists() {
            continue;
        }
        let raw = read_bars(&bars_path).map_err(|e| FactorError::StoreIo {
            context: format!("read {}", bars_path.display()),
            detail: e.to_string(),
        })?;
        let adjusted_path = dir.join("adjusted_bars.parquet");
        let adjusted = read_adjusted_bars(&adjusted_path).map_err(|e| FactorError::StoreIo {
            context: format!("read {}", adjusted_path.display()),
            detail: e.to_string(),
        })?;
        let raw_by_date: BTreeMap<TradingDate, &market_data::curate::schema::CuratedBar> =
            raw.iter().map(|b| (b.trading_date, b)).collect();
        for a in &adjusted {
            if a.trading_date > as_of {
                return Err(FactorError::FutureDatedRow {
                    instrument: symbol.clone(),
                    date: a.trading_date.to_iso(),
                    as_of: as_of.to_iso(),
                });
            }
            let r = raw_by_date
                .get(&a.trading_date)
                .ok_or_else(|| FactorError::StoreIo {
                    context: format!("merge {} {}", symbol, a.trading_date),
                    detail: "raw and adjusted series disagree on dates".to_owned(),
                })?;
            let close = a.close.amount().to_f64();
            if !close.is_finite() {
                return Err(FactorError::NonFinite {
                    factor: "input".to_owned(),
                    instrument: symbol.clone(),
                    date: a.trading_date.to_iso(),
                    value: close,
                });
            }
            points.push(BarPoint {
                date: a.trading_date,
                close,
                trading_value: r.trading_value.or(a.trading_value).map(|v| v as f64),
            });
        }
    }
    points.sort_by_key(|p| p.date.as_naive_date());
    Ok(points)
}

fn date_to_days(date: TradingDate) -> i32 {
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).expect("epoch");
    (date.as_naive_date() - epoch).num_days() as i32
}

pub(crate) fn days_to_date(days: i32) -> TradingDate {
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).expect("epoch");
    let naive = epoch + chrono::Duration::days(i64::from(days));
    TradingDate::new(naive.year(), naive.month(), naive.day())
        .expect("epoch days map to a valid date")
}

/// The last bar index of `pts` (within `pts[..=i]`) whose date is on or
/// before `target`, or `None` when the series has no such bar.
fn last_bar_on_or_before(pts: &[BarPoint], i: usize, target: NaiveDate) -> Option<usize> {
    let j = pts[..=i].partition_point(|p| p.date.as_naive_date() <= target);
    if j == 0 { None } else { Some(j - 1) }
}

fn build_frame(series: &BTreeMap<InstrumentId, Vec<BarPoint>>) -> Result<LazyFrame, FactorError> {
    let mut instruments: Vec<String> = Vec::new();
    let mut dates: Vec<i32> = Vec::new();
    let mut closes: Vec<f64> = Vec::new();
    let mut values: Vec<Option<f64>> = Vec::new();
    let mut targets: [Vec<Option<i32>>; 4] = std::array::from_fn(|_| Vec::new());
    for (id, pts) in series {
        for (i, p) in pts.iter().enumerate() {
            instruments.push(id.to_string());
            dates.push(date_to_days(p.date));
            closes.push(p.close);
            values.push(p.trading_value);
            for (w, col) in MONTH_WINDOWS.iter().zip(targets.iter_mut()) {
                let target = month_end(months_back(p.date.as_naive_date(), *w));
                col.push(last_bar_on_or_before(pts, i, target).map(|j| date_to_days(pts[j].date)));
            }
        }
    }

    let mut columns: Vec<Column> = Vec::with_capacity(8);
    let instrument_refs: Vec<&str> = instruments.iter().map(String::as_str).collect();
    columns.push(Column::new("instrument_id".into(), &instrument_refs));
    columns.push(
        Column::new("trading_date".into(), &dates)
            .cast(&DataType::Date)
            .map_err(|e| FactorError::Polars {
                detail: format!("trading_date cast: {e}"),
            })?,
    );
    columns.push(Column::new("close".into(), &closes));
    columns.push(Column::new("trading_value".into(), &values));
    for (w, col) in MONTH_WINDOWS.iter().zip(targets.iter()) {
        let name = format!("target_{w}m");
        columns.push(
            Column::new(name.as_str().into(), col)
                .cast(&DataType::Date)
                .map_err(|e| FactorError::Polars {
                    detail: format!("{name} cast: {e}"),
                })?,
        );
    }
    let df = DataFrame::new_infer_height(columns).map_err(|e| FactorError::Polars {
        detail: format!("frame build: {e}"),
    })?;
    Ok(df.lazy())
}
