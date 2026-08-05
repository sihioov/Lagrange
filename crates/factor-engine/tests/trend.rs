//! Red-phase tests: 50/100/200-day trend (price vs moving average) against
//! hand-calculated fixtures, plus the documented null policy (full-window
//! required; shorter history -> NULL).

mod common;

use common::{MARKET, VERSION, FixtureBar, write_curated};
use domain::{InstrumentId, TradingDate};
use factor_engine::bars::Bars;
use factor_engine::contract::{Factor, FactorContext};
use factor_engine::factors::TrendFactor;
use factor_engine::snapshot::FrozenUniverse;
use tempfile::tempdir;

fn consecutive_bars(n: usize, close_at: impl Fn(usize) -> i64) -> Vec<FixtureBar> {
    let start = chrono::NaiveDate::from_ymd_opt(2020, 1, 1).expect("date");
    (0..n)
        .map(|i| {
            let d = start + chrono::Duration::days(i as i64);
            FixtureBar::new(&d.format("%Y-%m-%d").to_string(), &format!("{}.0000", close_at(i)))
        })
        .collect()
}

fn ctx_for(dir: &std::path::Path, as_of: &str) -> FactorContext<'_> {
    let universe = FrozenUniverse::new("universe-test-1", &["TREND.KRX"]);
    let store = market_data::CurateStore::new(dir);
    let bars = Bars::from_curated(
        &store,
        MARKET,
        "kr-etf-daily-test",
        VERSION,
        &universe,
        TradingDate::parse(as_of).expect("as_of"),
    )
    .expect("bars load");
    FactorContext {
        as_of: TradingDate::parse(as_of).expect("as_of"),
        universe: &universe,
        bars: &bars,
    }
}

fn nth_value(frame: &factor_engine::contract::FactorFrame, i: usize) -> Option<f64> {
    frame.rows[i].value
}

#[test]
fn trend_50_hand_calculated() {
    // closes 100, 101, ..., 159 (60 bars). MA50 at bar 49 = mean(100..=149) = 124.5.
    let dir = tempdir().expect("temp");
    let bars = consecutive_bars(60, |i| 100 + i as i64);
    write_curated(dir.path(), "TREND.KRX", 2020, &bars);
    let ctx = ctx_for(dir.path(), "2020-03-02");
    let f = TrendFactor::new(50).expect("window").compute(&ctx).expect("compute");

    assert_eq!(f.rows.len(), 60, "one row per bar");
    // First 49 bars: NULL (window of 50 not yet filled).
    for i in 0..49 {
        assert!(nth_value(&f, i).is_none(), "bar {i} must be NULL (short lookback)");
    }
    let expected_49 = 149.0 / 124.5 - 1.0;
    let a = nth_value(&f, 49).expect("bar 49 non-null");
    assert!((a - expected_49).abs() < 1e-12, "bar 49: {a} vs {expected_49}");
    let expected_59 = 159.0 / 134.5 - 1.0;
    let a = nth_value(&f, 59).expect("bar 59 non-null");
    assert!((a - expected_59).abs() < 1e-12, "bar 59: {a} vs {expected_59}");
}

#[test]
fn trend_50_constant_series_is_zero() {
    let dir = tempdir().expect("temp");
    write_curated(dir.path(), "TREND.KRX", 2020, &consecutive_bars(60, |_| 100));
    let ctx = ctx_for(dir.path(), "2020-03-02");
    let f = TrendFactor::new(50).expect("window").compute(&ctx).expect("compute");
    for i in 49..60 {
        assert_eq!(nth_value(&f, i), Some(0.0), "bar {i} trend must be exactly 0.0");
    }
}

#[test]
fn trend_100_and_200_metadata_and_values() {
    let dir = tempdir().expect("temp");
    // 210 bars with close = 100 + i: MA100 at bar 99 = mean(100..=199) = 149.5.
    write_curated(dir.path(), "TREND.KRX", 2020, &consecutive_bars(210, |i| 100 + i as i64));
    let ctx = ctx_for(dir.path(), "2020-08-01");
    let f100 = TrendFactor::new(100).expect("window").compute(&ctx).expect("compute");
    let expected_99 = 199.0 / 149.5 - 1.0;
    let a = nth_value(&f100, 99).expect("bar 99");
    assert!((a - expected_99).abs() < 1e-12, "trend_100 bar 99: {a} vs {expected_99}");
    assert!(nth_value(&f100, 98).is_none(), "bar 98 NULL for 100-day window");

    let f200 = TrendFactor::new(200).expect("window").compute(&ctx).expect("compute");
    let expected_209 = 309.0 / 209.5 - 1.0;
    let a = nth_value(&f200, 209).expect("bar 209");
    assert!((a - expected_209).abs() < 1e-12, "trend_200 bar 209: {a} vs {expected_209}");
    assert!(nth_value(&f200, 198).is_none(), "bar 198 NULL for 200-day window");

    assert_eq!(f100.factor, "trend_100");
    assert_eq!(f200.factor, "trend_200");
    assert_eq!(TrendFactor::new(50).expect("w").id(), "trend_50");
    assert!(TrendFactor::new(7).is_err(), "unsupported window rejected");
}

#[test]
fn trend_factor_metadata() {
    let f = TrendFactor::new(50).expect("window");
    assert_eq!(f.version().to_string(), "1.0.0");
    assert_eq!(f.required_fields().len(), 1);
    assert_eq!(
        f.lookback(),
        factor_engine::contract::Lookback::TradingDays { window: 50, min_periods: 50 }
    );
    assert_eq!(
        f.null_policy(),
        factor_engine::contract::NullPolicy::InsufficientLookback
    );
}
