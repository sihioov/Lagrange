//! Red-phase tests: recent-high drawdown = close / running max(close) - 1
//! (<= 0), hand-calculated. The running max starts at the first bar, so the
//! factor is defined from the first observation (no lookback nulls).

mod common;

use common::{MARKET, VERSION, FixtureBar, write_curated};
use domain::{InstrumentId, TradingDate};
use factor_engine::bars::Bars;
use factor_engine::contract::{Factor, FactorContext};
use factor_engine::factors::DrawdownFactor;
use factor_engine::snapshot::FrozenUniverse;
use tempfile::tempdir;

fn ctx_for(dir: &std::path::Path, as_of: &str) -> FactorContext<'_> {
    let universe = FrozenUniverse::new("universe-test-1", &["DD.KRX"]);
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
fn drawdown_hand_calculated() {
    let dir = tempdir().expect("temp");
    let closes = ["100.0000", "110.0000", "105.0000", "130.0000", "120.0000"];
    let bars: Vec<FixtureBar> = closes
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let d = chrono::NaiveDate::from_ymd_opt(2020, 1, 1).expect("date")
                + chrono::Duration::days(i as i64);
            FixtureBar::new(&d.format("%Y-%m-%d").to_string(), c)
        })
        .collect();
    write_curated(dir.path(), "DD.KRX", 2020, &bars);
    let ctx = ctx_for(dir.path(), "2020-01-06");
    let f = DrawdownFactor.compute(&ctx).expect("compute");
    assert_eq!(f.rows.len(), 5);
    assert_eq!(nth_value(&f, 0), Some(0.0), "bar 0 at its own high");
    assert_eq!(nth_value(&f, 1), Some(0.0), "bar 1 new high");
    let a = nth_value(&f, 2).expect("bar 2");
    let expected = 105.0 / 110.0 - 1.0;
    assert!((a - expected).abs() < 1e-12, "bar 2: {a} vs {expected}");
    assert_eq!(nth_value(&f, 3), Some(0.0), "bar 3 new high");
    let a = nth_value(&f, 4).expect("bar 4");
    let expected = 120.0 / 130.0 - 1.0;
    assert!((a - expected).abs() < 1e-12, "bar 4: {a} vs {expected}");
}

#[test]
fn drawdown_never_positive_and_metadata() {
    let dir = tempdir().expect("temp");
    // Oscillating series; drawdown must stay in [-1, 0] on every bar.
    let closes = ["100.0000", "95.0000", "110.0000", "90.0000", "105.0000"];
    let bars: Vec<FixtureBar> = closes
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let d = chrono::NaiveDate::from_ymd_opt(2020, 1, 1).expect("date")
                + chrono::Duration::days(i as i64);
            FixtureBar::new(&d.format("%Y-%m-%d").to_string(), c)
        })
        .collect();
    write_curated(dir.path(), "DD.KRX", 2020, &bars);
    let ctx = ctx_for(dir.path(), "2020-01-06");
    let f = DrawdownFactor.compute(&ctx).expect("compute");
    for i in 0..5 {
        let v = nth_value(&f, i).expect("non-null");
        assert!((-1.0..=0.0).contains(&v), "bar {i} drawdown {v} out of [-1, 0]");
    }
    assert_eq!(DrawdownFactor.id(), "drawdown");
    assert_eq!(DrawdownFactor.version().to_string(), "1.0.0");
    assert_eq!(DrawdownFactor.lookback(), factor_engine::contract::Lookback::FullHistory);
    assert_eq!(
        DrawdownFactor.null_policy(),
        factor_engine::contract::NullPolicy::InsufficientLookback
    );
}
