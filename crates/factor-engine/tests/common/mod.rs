//! Shared fixture builders for factor-engine integration tests.
//!
//! Every fixture is written in the documented Curated layout (design §7.1):
//! `data/curated/bars/market=kr/symbol={s}/year={y}/version={v}/{bars,adjusted_bars}.parquet`
//! via the public Todo 10 writers, so the engine's real curated input path is
//! exercised end-to-end. Scratch space lives in temp dirs (C: drive only).

use std::path::Path;

use domain::{ContentHash, Currency, FixedPoint, InstrumentId, Price, TradingDate, UtcTimestamp};
use factor_engine::bars::Bars;
use factor_engine::contract::FactorContext;
use factor_engine::snapshot::FrozenUniverse;
use market_data::CurateStore;
use market_data::curate::adjust::{AdjustmentBar, AdjustmentKind};
use market_data::curate::schema::{CuratedBar, write_adjusted_bars, write_bars};

pub const MARKET: &str = "kr";
pub const VERSION: u32 = 1;
pub const DATASET_ID: &str = "kr-etf-daily-test";

/// Owns the universe + bars so a [`FactorContext`] can borrow them safely.
/// Not every test binary uses it (snapshot-level tests build directly).
#[allow(dead_code)]
pub struct TestCtx {
    pub universe: FrozenUniverse,
    pub bars: Bars,
    pub as_of: TradingDate,
}

#[allow(dead_code)]
impl TestCtx {
    pub fn new(dir: &Path, symbols: &[&str], as_of: &str) -> Self {
        let as_of = TradingDate::parse(as_of).expect("as_of");
        let universe = FrozenUniverse::new("universe-test-1", symbols);
        let store = CurateStore::new(dir);
        let bars = Bars::from_curated(&store, MARKET, DATASET_ID, VERSION, &universe, as_of)
            .expect("bars load");
        Self {
            universe,
            bars,
            as_of,
        }
    }

    #[allow(dead_code)]
    pub fn ctx(&self) -> FactorContext<'_> {
        FactorContext {
            as_of: self.as_of,
            universe: &self.universe,
            bars: &self.bars,
        }
    }
}

#[derive(Clone)]
pub struct FixtureBar {
    pub date: String,
    pub close: String,
    pub volume: i64,
    pub trading_value: Option<i64>,
}

impl FixtureBar {
    pub fn new(date: &str, close: &str) -> Self {
        Self {
            date: date.to_owned(),
            close: close.to_owned(),
            volume: 1_000,
            trading_value: Some(100_000_000),
        }
    }
}

pub fn fixture_ts() -> UtcTimestamp {
    UtcTimestamp::parse_rfc3339("2020-01-01T00:00:00Z").expect("fixed ts")
}

pub fn close_price(close: &str) -> Price {
    Price::from_fixed(FixedPoint::parse(close).expect("close parses")).expect("positive price")
}

/// Writes raw + split-adjusted curated parquet for one symbol/year/version.
pub fn write_curated(dir: &Path, symbol: &str, year: i32, bars: &[FixtureBar]) {
    let store = CurateStore::new(dir);
    let raw: Vec<CuratedBar> = bars.iter().map(|b| raw_row(symbol, b)).collect();
    write_bars(&store.bars_path(MARKET, symbol, year, VERSION), &raw).expect("write raw bars");
    let adjusted: Vec<AdjustmentBar> = bars.iter().map(|b| adjusted_row(symbol, b)).collect();
    write_adjusted_bars(
        &store.adjusted_bars_path(MARKET, symbol, year, VERSION),
        &adjusted,
    )
    .expect("write adjusted bars");
}

pub fn raw_row(symbol: &str, b: &FixtureBar) -> CuratedBar {
    CuratedBar {
        instrument_id: InstrumentId::parse(symbol).expect("instrument id"),
        trading_date: TradingDate::parse(&b.date).expect("date"),
        market_open_ts: fixture_ts(),
        market_close_ts: fixture_ts(),
        open: close_price(&b.close),
        high: close_price(&b.close),
        low: close_price(&b.close),
        close: close_price(&b.close),
        volume: b.volume,
        trading_value: b.trading_value,
        currency: Currency::from_code("KRW").expect("currency"),
        source: "test".to_owned(),
        ingested_at: fixture_ts(),
        batch_id: "00000000-0000-0000-0000-000000000001"
            .parse()
            .expect("batch id"),
        raw_hash: ContentHash::from_bytes(b"fixture"),
    }
}

pub fn adjusted_row(symbol: &str, b: &FixtureBar) -> AdjustmentBar {
    AdjustmentBar {
        instrument_id: InstrumentId::parse(symbol).expect("instrument id"),
        trading_date: TradingDate::parse(&b.date).expect("date"),
        market_open_ts: fixture_ts(),
        market_close_ts: fixture_ts(),
        open: close_price(&b.close),
        high: close_price(&b.close),
        low: close_price(&b.close),
        close: close_price(&b.close),
        volume: b.volume,
        trading_value: b.trading_value,
        adjustment_kind: AdjustmentKind::Split,
        adjustment_factor: FixedPoint::parse("1.00000000").expect("factor"),
        adjustment_events: String::new(),
        currency: Currency::from_code("KRW").expect("currency"),
        source: "test".to_owned(),
        ingested_at: fixture_ts(),
        batch_id: "00000000-0000-0000-0000-000000000001"
            .parse()
            .expect("batch id"),
        raw_hash: ContentHash::from_bytes(b"fixture"),
    }
}

/// Monthly bars, close = 100 x 1.1^n (exact decimal strings, scale 4).
#[allow(dead_code)]
pub fn monthly_growth_bars(n: usize) -> Vec<FixtureBar> {
    const DATES: [&str; 12] = [
        "2020-01-31",
        "2020-02-28",
        "2020-03-31",
        "2020-04-30",
        "2020-05-29",
        "2020-06-30",
        "2020-07-31",
        "2020-08-31",
        "2020-09-30",
        "2020-10-30",
        "2020-11-30",
        "2020-12-31",
    ];
    const CLOSES: [&str; 12] = [
        "100.0000", "110.0000", "121.0000", "133.1000", "146.4100", "161.0510", "177.1561",
        "194.8717", "214.3589", "235.7948", "259.3742", "285.3117",
    ];
    let n = n.min(DATES.len());
    (0..n)
        .map(|i| FixtureBar::new(DATES[i], CLOSES[i]))
        .collect()
}

#[allow(dead_code)]
pub fn monthly_with_2019() -> Vec<FixtureBar> {
    let mut bars = vec![FixtureBar::new("2019-12-31", "90.9091")];
    bars.extend(monthly_growth_bars(12));
    bars
}
