//! Red-phase tests: the no-forward-fill contract.
//!
//! (a) A gap in an instrument's series: the reference bar for a month window
//!     is the LAST bar on or before the calendar target - a bar AFTER the
//!     target (even the very next one) is never pulled back in time.
//! (b) A series that starts after the target has no reference: NULL, not a
//!     forward-filled value.
//! (c) Rows dated after as-of are rejected (see future_rows.rs for the full
//!     typed-rejection surface).

mod common;

use common::{MARKET, VERSION, FixtureBar, write_curated};
use domain::{InstrumentId, TradingDate};
use factor_engine::bars::Bars;
use factor_engine::contract::{Factor, FactorContext};
use factor_engine::factors::ReturnFactor;
use factor_engine::snapshot::FrozenUniverse;
use tempfile::tempdir;

fn ctx_for(dir: &std::path::Path, as_of: &str) -> FactorContext<'_> {
    let universe = FrozenUniverse::new("universe-test-1", &["GAP.KRX"]);
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

fn value_of(frame: &factor_engine::contract::FactorFrame, date: &str) -> Option<f64> {
    frame
        .rows
        .iter()
        .find(|r| {
            r.instrument == InstrumentId::parse("GAP.KRX").expect("id")
                && r.date == TradingDate::parse(date).expect("date")
        })
        .map(|r| r.value)
}

#[test]
fn gap_uses_last_bar_before_target_never_next_bar() {
    let dir = tempdir().expect("temp");
    // March is missing entirely: 2020-01-31(100), 2020-02-28(110), 2020-04-30(121).
    let bars = vec![
        FixtureBar::new("2020-01-31", "100.0000"),
        FixtureBar::new("2020-02-28", "110.0000"),
        FixtureBar::new("2020-04-30", "121.0000"),
    ];
    write_curated(dir.path(), "GAP.KRX", 2020, &bars);
    let ctx = ctx_for(dir.path(), "2020-12-31");
    let f = ReturnFactor::one_month().compute(&ctx).expect("compute");

    // 1m at 2020-04-30: target = 2020-03-30. The 2020-04-30 bar is AFTER the
    // target and must NOT be used as its own reference (would yield 0.0).
    let a = value_of(&f, "2020-04-30").expect("non-null");
    let expected = 121.0 / 110.0 - 1.0; // reference = 02-28, not 04-30
    assert!((a - expected).abs() < 1e-12, "gap 1m: {a} vs {expected}");
    assert!((a - 0.0).abs() > 0.1, "gap 1m must not self-reference (forward fill)");
}

#[test]
fn target_after_last_bar_yields_null_not_fill() {
    let dir = tempdir().expect("temp");
    // Series starts 2020-06-30; as-of 2020-07-31. The 6m target (2020-01-31)
    // precedes every bar: 6m return must be NULL.
    let bars = vec![
        FixtureBar::new("2020-06-30", "100.0000"),
        FixtureBar::new("2020-07-31", "110.0000"),
    ];
    write_curated(dir.path(), "GAP.KRX", 2020, &bars);
    let ctx = ctx_for(dir.path(), "2020-12-31");
    let f6 = ReturnFactor::six_months().compute(&ctx).expect("compute");
    assert!(value_of(&f6, "2020-07-31").is_none(), "6m @07-31 NULL (no history)");
    let f1 = ReturnFactor::one_month().compute(&ctx).expect("compute");
    let a = value_of(&f1, "2020-07-31").expect("1m has a reference");
    assert!((a - 0.1).abs() < 1e-12, "1m @07-31 = {a}");
}

#[test]
fn bar_after_calendar_target_is_excluded() {
    let dir = tempdir().expect("temp");
    // Monthly bars with a 05-29 bar: 1m at 05-29 has target 04-29, and the
    // 04-30 bar is AFTER the target - the reference must be 03-31.
    // Documented rule: reference = last bar on or before the target date.
    let bars = vec![
        FixtureBar::new("2020-01-31", "100.0000"),
        FixtureBar::new("2020-02-28", "110.0000"),
        FixtureBar::new("2020-03-31", "121.0000"),
        FixtureBar::new("2020-04-30", "133.1000"),
        FixtureBar::new("2020-05-29", "146.4100"),
    ];
    write_curated(dir.path(), "GAP.KRX", 2020, &bars);
    let ctx = ctx_for(dir.path(), "2020-12-31");
    let f = ReturnFactor::one_month().compute(&ctx).expect("compute");
    let a = value_of(&f, "2020-05-29").expect("non-null");
    let expected = 146.41 / 121.0 - 1.0; // 03-31, NOT 04-30
    assert!((a - expected).abs() < 1e-12, "1m @05-29: {a} vs {expected}");
    let a = value_of(&f, "2020-04-30").expect("non-null");
    let expected = 133.1 / 110.0 - 1.0; // target 03-30 < 03-31 bar -> 02-28
    assert!((a - expected).abs() < 1e-12, "1m @04-30: {a} vs {expected}");
}
