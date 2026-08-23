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

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use chrono::{Datelike, NaiveDate};
use domain::{InstrumentId, TradingDate};
use market_data::curate::schema::{read_adjusted_bars, read_bars};
use market_data::{
    ApprovedHistoricalPriceOnlyArtifact, CurateError, CurateStore, HistoricalPriceOnlyBar,
    KR_ETF_CORE_SYMBOLS,
};
use polars::prelude::*;

use crate::contract::{FactorError, Field};
use crate::months::{month_end, months_back};
use crate::snapshot::FrozenUniverse;

/// The documented calendar month windows of the return factors.
pub const MONTH_WINDOWS: [u32; 4] = [1, 3, 6, 12];

const CURATED_FIELDS: &[Field] = &[Field::CLOSE, Field::TRADING_VALUE];
const PRICE_ONLY_FIELDS: &[Field] = &[Field::CLOSE];

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
    available_fields: &'static [Field],
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
            available_fields: CURATED_FIELDS,
        })
    }

    /// Resolves the approved owner-beta historical price artifact.
    ///
    /// This constructor is intentionally private to the factor-engine input
    /// layer. The public [`PriceOnlyBars`] wrapper below is the only production
    /// path that can obtain this representation, and it accepts no store,
    /// dataset, path, or caller-supplied provenance.
    fn from_approved_price_only(
        artifact: &ApprovedHistoricalPriceOnlyArtifact,
        as_of: TradingDate,
    ) -> Result<Self, FactorError> {
        Self::from_approved_price_only_bars(artifact.bars(), as_of)
    }

    fn from_approved_price_only_bars(
        artifact_bars: &[HistoricalPriceOnlyBar],
        as_of: TradingDate,
    ) -> Result<Self, FactorError> {
        let expected: BTreeSet<InstrumentId> = KR_ETF_CORE_SYMBOLS
            .iter()
            .map(|symbol| {
                InstrumentId::parse(&format!("{symbol}.KRX")).map_err(FactorError::Domain)
            })
            .collect::<Result<_, _>>()?;
        let mut observed = BTreeSet::new();
        let mut as_of_instruments = BTreeSet::new();
        let mut series: BTreeMap<InstrumentId, Vec<BarPoint>> = expected
            .iter()
            .cloned()
            .map(|id| (id, Vec::new()))
            .collect();

        for bar in artifact_bars {
            if !expected.contains(&bar.instrument_id) {
                return Err(FactorError::InvalidDefinition {
                    detail:
                        "approved price-only artifact contains an instrument outside fixed ETF11"
                            .to_owned(),
                });
            }
            observed.insert(bar.instrument_id.clone());
            if bar.session_date == as_of {
                as_of_instruments.insert(bar.instrument_id.clone());
            }

            // The approved artifact covers the complete historical range. A
            // price-only snapshot is a point-in-time view, so rows after the
            // requested session are deliberately invisible rather than typed
            // FutureDatedRow failures.
            if bar.session_date > as_of {
                continue;
            }
            let close = bar.adjusted_close.to_f64();
            if !close.is_finite() {
                return Err(FactorError::NonFinite {
                    factor: "input".to_owned(),
                    instrument: bar.instrument_id.to_string(),
                    date: bar.session_date.to_iso(),
                    value: close,
                });
            }
            let points = series.get_mut(&bar.instrument_id).ok_or_else(|| {
                FactorError::InvalidDefinition {
                    detail: "approved price-only artifact instrument gate failed".to_owned(),
                }
            })?;
            if points.iter().any(|point| point.date == bar.session_date) {
                return Err(FactorError::CorruptData {
                    context: "approved price-only bars".to_owned(),
                    detail: "duplicate instrument/session row".to_owned(),
                });
            }
            // Raw close and raw trading value are intentionally not read. The
            // adjusted close is the sole price-only factor input.
            points.push(BarPoint {
                date: bar.session_date,
                close,
                trading_value: None,
            });
        }

        if observed != expected {
            return Err(FactorError::InvalidDefinition {
                detail: "approved price-only artifact does not cover exactly fixed ETF11"
                    .to_owned(),
            });
        }
        if as_of_instruments != expected {
            return Err(FactorError::MissingData {
                detail: "as_of is not a complete approved ETF11 session".to_owned(),
            });
        }
        for points in series.values_mut() {
            points.sort_by_key(|point| point.date.as_naive_date());
        }
        let frame = build_frame(&series)?;
        Ok(Self {
            as_of,
            series,
            frame,
            available_fields: PRICE_ONLY_FIELDS,
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
        self.available_fields
    }
}

/// The bounded factor input derived from the owner-approved historical
/// price-only artifact.
///
/// This distinct type prevents callers from accidentally treating the
/// owner-beta input as a generic curated dataset. It carries adjusted close
/// only; raw OHLCV and trading value never enter the factor frame.
#[derive(Clone)]
pub struct PriceOnlyBars {
    bars: Bars,
}

impl PriceOnlyBars {
    /// Builds a point-in-time price-only input from the nonconstructible
    /// approved artifact.
    pub fn from_approved(
        artifact: &ApprovedHistoricalPriceOnlyArtifact,
        as_of: TradingDate,
    ) -> Result<Self, FactorError> {
        Ok(Self {
            bars: Bars::from_approved_price_only(artifact, as_of)?,
        })
    }

    /// The snapshot as-of date.
    pub fn as_of(&self) -> TradingDate {
        self.bars.as_of()
    }

    /// The resolved fixed ETF11 instruments in canonical order.
    pub fn instruments(&self) -> impl Iterator<Item = &InstrumentId> {
        self.bars.instruments()
    }

    /// The adjusted-close points of one instrument.
    pub fn points(&self, id: &InstrumentId) -> Option<&[BarPoint]> {
        self.bars.points(id)
    }

    /// The deterministic lazy factor frame.
    pub fn lazy_frame(&self) -> LazyFrame {
        self.bars.lazy_frame()
    }

    /// The only field carried by this input.
    pub fn available_fields(&self) -> &'static [Field] {
        self.bars.available_fields()
    }

    pub(crate) fn as_bars(&self) -> &Bars {
        &self.bars
    }

    #[cfg(test)]
    pub(crate) fn from_test_parts(
        bars: &[HistoricalPriceOnlyBar],
        as_of: TradingDate,
    ) -> Result<Self, FactorError> {
        Ok(Self {
            bars: Bars::from_approved_price_only_bars(bars, as_of)?,
        })
    }
}

/// Explicit aliases for callers that name the temporary owner-beta path.
pub type OwnerBetaPriceOnlyBars = PriceOnlyBars;

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
        match fs::metadata(&bars_path) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => {
                return Err(FactorError::CorruptData {
                    context: format!("inspect {}", bars_path.display()),
                    detail: "bars path is not a regular file".to_owned(),
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => match fs::metadata(dir) {
                Ok(metadata) if metadata.is_dir() => {
                    return Err(FactorError::MissingData {
                        detail: format!("required bars component {}", bars_path.display()),
                    });
                }
                Ok(_) => {
                    return Err(FactorError::CorruptData {
                        context: format!("inspect {}", dir.display()),
                        detail: "version partition is not a directory".to_owned(),
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(FactorError::StoreIo {
                        context: format!("inspect {}", dir.display()),
                        detail: error.to_string(),
                    });
                }
            },
            Err(error) => {
                return Err(FactorError::StoreIo {
                    context: format!("inspect {}", bars_path.display()),
                    detail: error.to_string(),
                });
            }
        }
        let raw = read_bars(&bars_path).map_err(|error| curated_read_error(&bars_path, error))?;
        let adjusted_path = dir.join("adjusted_bars.parquet");
        let adjusted = read_adjusted_bars(&adjusted_path)
            .map_err(|error| curated_read_error(&adjusted_path, error))?;
        require_partition_identity(id, &raw, &adjusted)?;
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
                .ok_or_else(|| FactorError::CorruptData {
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

fn require_partition_identity(
    expected_symbol: &InstrumentId,
    raw: &[market_data::curate::schema::CuratedBar],
    adjusted: &[market_data::curate::adjust::AdjustmentBar],
) -> Result<(), FactorError> {
    if let Some(row) = raw.iter().find(|row| &row.instrument_id != expected_symbol) {
        return Err(FactorError::CorruptData {
            context: format!("raw partition symbol={expected_symbol}"),
            detail: format!("row declares instrument {}", row.instrument_id),
        });
    }
    if let Some(row) = adjusted
        .iter()
        .find(|row| &row.instrument_id != expected_symbol)
    {
        return Err(FactorError::CorruptData {
            context: format!("adjusted partition symbol={expected_symbol}"),
            detail: format!("row declares instrument {}", row.instrument_id),
        });
    }
    let mut raw_dates = raw.iter().map(|row| row.trading_date).collect::<Vec<_>>();
    let mut adjusted_dates = adjusted
        .iter()
        .map(|row| row.trading_date)
        .collect::<Vec<_>>();
    raw_dates.sort_unstable();
    adjusted_dates.sort_unstable();
    if raw_dates != adjusted_dates {
        return Err(FactorError::CorruptData {
            context: format!("raw/adjusted alignment symbol={expected_symbol}"),
            detail: "raw and adjusted series do not contain identical dates".to_owned(),
        });
    }
    Ok(())
}

fn curated_read_error(path: &std::path::Path, error: CurateError) -> FactorError {
    match error {
        CurateError::MissingCuratedComponent { .. } => FactorError::MissingData {
            detail: error.to_string(),
        },
        CurateError::StoreIo { context, detail } => FactorError::StoreIo { context, detail },
        other => FactorError::CorruptData {
            context: format!("read {}", path.display()),
            detail: other.to_string(),
        },
    }
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
