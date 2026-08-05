//! Red-phase tests: 20-day average trading value with the documented STRICT
//! window policy (a NULL input inside the window -> NULL output; shorter
//! history -> NULL; never a partial-window mean).

mod common;

use common::{MARKET, VERSION, FixtureBar, write_curated};
use domain::{InstrumentId, TradingDate};
use factor_engine::bars::Bars;
use factor_engine::contract::{Factor, FactorContext};
use factor_engine::factors::AvgValueFactor;
use factor_engine::snapshot::FrozenUniverse;
use tempfile::tempdir;

fn ctx_for(dir: &std::path::Path, as_of: &str) -> FactorContext<'_> {
    let universe = FrozenUniverse::new("universe-test-1", &["LIQ.KRX"]);
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

fn date_at(i: usize) -> String {
    let d = chrono::NaiveDate::from_ymd_opt(2020, 1, 1).expect("date")
        + chrono::Duration::days(i as i64);
    d.format("%Y-%m-%d").to_string()
}

#[test]
fn avg_value_20_hand_calculated() {
    let dir = tempdir().expect("temp");
    // trading_value = 1000 + 10*i (i = 0..40). Window mean at bar 19:
    // mean(1000, 1010, ..., 1190) = 1095.0.
    let bars: Vec<FixtureBar> = (0..40)
        .map(|i| FixtureBar {
            date: date_at(i),
            close: "100.0000".to_owned(),
            volume: 1_000,
            trading_value: Some(1_000 + 10 * i as i64),
        })
        .collect();
    write_curated(dir.path(), "LIQ.KRX", 2020, &bars);
    let ctx = ctx_for(dir.path(), "2020-02-10");
    let f = AvgValueFactor.compute(&ctx).expect("compute");
    assert_eq!(f.rows.len(), 40);
    for i in 0..19 {
        assert!(nth_value(&f, i).is_none(), "bar {i} NULL (short lookback)");
    }
    let a = nth_value(&f, 19).expect("bar 19 non-null");
    assert!((a - 1095.0).abs() < 1e-9, "bar 19: {a} vs 1095.0");
    let a = nth_value(&f, 39).expect("bar 39 non-null");
    let expected_39 = (1020..=1410).step_by(10).sum::<i64>() as f64 / 20.0;
    assert!((a - expected_39).abs() < 1e-9, "bar 39: {a} vs {expected_39}");
}

#[test]
fn avg_value_20_strict_window_null_propagates() {
    let dir = tempdir().expect("temp");
    // Bar 10 has no trading value. STRICT policy: every window containing
    // bar 10 outputs NULL; the first clean window is bars 11..=30 (bar 30).
    let mut bars: Vec<FixtureBar> = (0..40)
        .map(|i| FixtureBar {
            date: date_at(i),
            close: "100.0000".to_owned(),
            volume: 1_000,
            trading_value: Some(100_000_000),
        })
        .collect();
    bars[10].trading_value = None;
    write_curated(dir.path(), "LIQ.KRX", 2020, &bars);
    let ctx = ctx_for(dir.path(), "2020-02-10");
    let f = AvgValueFactor.compute(&ctx).expect("compute");

    for i in 0..10 {
        assert!(nth_value(&f, i).is_none(), "bar {i} NULL (short lookback)");
    }
    for i in 10..=29 {
        assert!(nth_value(&f, i).is_none(), "bar {i} NULL (window contains the missing value)");
    }
    assert!(nth_value(&f, 30).is_some(), "bar 30 first clean window");
    // The mean at bar 30 must be computed over 11..=30 (20 values), not a
    // partial window that includes a zero-filled bar 10.
    let a = nth_value(&f, 30).expect("bar 30");
    assert!((a - 100_000_000.0).abs() < 1e-6, "bar 30: {a}");
}

#[test]
fn avg_value_20_all_missing_is_all_null() {
    let dir = tempdir().expect("temp");
    let bars: Vec<FixtureBar> = (0..25)
        .map(|i| FixtureBar {
            date: date_at(i),
            close: "100.0000".to_owned(),
            volume: 1_000,
            trading_value: None,
        })
        .collect();
    write_curated(dir.path(), "LIQ.KRX", 2020, &bars);
    let ctx = ctx_for(dir.path(), "2020-01-25");
    let f = AvgValueFactor.compute(&ctx).expect("compute");
    for i in 0..25 {
        assert!(nth_value(&f, i).is_none(), "bar {i} NULL (no trading value anywhere)");
    }
    assert_eq!(AvgValueFactor.id(), "avg_value_20");
    assert_eq!(AvgValueFactor.version().to_string(), "1.0.0");
    assert_eq!(
        AvgValueFactor.lookback(),
        factor_engine::contract::Lookback::FixedWindow { window: 20, min_periods: 20 }
    );
    assert_eq!(AvgValueFactor.null_policy(), factor_engine::contract::NullPolicy::StrictWindow);
}
