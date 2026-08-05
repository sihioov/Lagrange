//! Red-phase table-driven tests: 1/3/6/12-month return and 12-minus-1 momentum
//! against hand-calculated fixtures (monthly bars, close = 100 x 1.1^n).
//!
//! Documented semantics: the reference bar is the LAST bar on or before the
//! calendar target `as_of - N months` (day-of-month clamped to the target
//! month's last day). No forward fill: a bar after the target is never used.

mod common;

use common::{MARKET, VERSION, monthly_growth_bars, monthly_with_2019, write_curated};
use domain::{InstrumentId, TradingDate};
use factor_engine::contract::{Factor, FactorContext};
use factor_engine::factors::ReturnFactor;
use factor_engine::snapshot::FrozenUniverse;
use factor_engine::bars::Bars;
use tempfile::tempdir;

fn ctx_with(dir: &std::path::Path, as_of: &str, symbols: &[&str]) -> FactorContext<'_> {
    let universe = FrozenUniverse::new("universe-test-1", symbols);
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

fn value_of(frame: &factor_engine::contract::FactorFrame, symbol: &str, date: &str) -> Option<f64> {
    frame
        .rows
        .iter()
        .find(|r| {
            r.instrument == InstrumentId::parse(symbol).expect("id")
                && r.date == TradingDate::parse(date).expect("date")
        })
        .map(|r| r.value)
}

fn expect_close(actual: Option<f64>, expected: f64, tol: f64, what: &str) {
    let a = actual.unwrap_or_else(|| panic!("{what}: expected {expected}, got NULL"));
    assert!(
        (a - expected).abs() <= tol,
        "{what}: expected {expected}, got {a}"
    );
}

#[test]
fn return_1m_hand_calculated() {
    let dir = tempdir().expect("temp");
    write_curated(dir.path(), "FIXTURE.KRX", 2020, &monthly_growth_bars(12));
    let ctx = ctx_with(dir.path(), "2020-12-31", &["FIXTURE.KRX"]);
    let f = ReturnFactor::one_month().compute(&ctx).expect("compute");

    // 285.3117 / 259.3742 - 1 (exact ratio of the fixture's decimal closes).
    expect_close(value_of(&f, "FIXTURE.KRX", "2020-12-31"), 285.3117 / 259.3742 - 1.0, 1e-12, "1m @12-31");
    // Exact: 121.0000 / 110.0000 - 1 == 0.1 (both closes are exact 4dp values).
    expect_close(value_of(&f, "FIXTURE.KRX", "2020-03-31"), 0.1, 1e-12, "1m @03-31 (month-end clamp)");
    expect_close(value_of(&f, "FIXTURE.KRX", "2020-06-30"), 161.0510 / 146.4100 - 1.0, 1e-12, "1m @06-30");
    // Insufficient history: target 2020-01-28 precedes the first bar 2020-01-31.
    assert!(
        value_of(&f, "FIXTURE.KRX", "2020-02-28").is_none(),
        "1m @02-28 must be NULL (target before first bar)"
    );
    assert!(
        value_of(&f, "FIXTURE.KRX", "2020-01-31").is_none(),
        "1m @01-31 must be NULL (target 2019-12-31 has no bar)"
    );
}

#[test]
fn return_3m_6m_hand_calculated() {
    let dir = tempdir().expect("temp");
    write_curated(dir.path(), "FIXTURE.KRX", 2020, &monthly_growth_bars(12));
    let ctx = ctx_with(dir.path(), "2020-12-31", &["FIXTURE.KRX"]);

    let f3 = ReturnFactor::three_months().compute(&ctx).expect("compute");
    expect_close(value_of(&f3, "FIXTURE.KRX", "2020-12-31"), 285.3117 / 214.3589 - 1.0, 1e-12, "3m @12-31");
    expect_close(value_of(&f3, "FIXTURE.KRX", "2020-06-30"), 161.0510 / 121.0000 - 1.0, 1e-12, "3m @06-30");
    assert!(value_of(&f3, "FIXTURE.KRX", "2020-02-28").is_none(), "3m @02-28 NULL");

    let f6 = ReturnFactor::six_months().compute(&ctx).expect("compute");
    expect_close(value_of(&f6, "FIXTURE.KRX", "2020-12-31"), 285.3117 / 161.0510 - 1.0, 1e-12, "6m @12-31");
    expect_close(value_of(&f6, "FIXTURE.KRX", "2020-06-30"), 161.0510 / 100.0000 - 1.0, 1e-12, "6m @06-30");
}

#[test]
fn return_12m_hand_calculated() {
    let dir = tempdir().expect("temp");
    write_curated(dir.path(), "FIXTURE.KRX", 2019, &monthly_with_2019());
    write_curated(dir.path(), "FIXTURE.KRX", 2020, &monthly_growth_bars(12));
    let ctx = ctx_with(dir.path(), "2020-12-31", &["FIXTURE.KRX"]);
    let f = ReturnFactor::twelve_months().compute(&ctx).expect("compute");

    // Reference = 2019-12-31 bar (exactly 12 calendar months back).
    expect_close(value_of(&f, "FIXTURE.KRX", "2020-12-31"), 285.3117 / 90.9091 - 1.0, 1e-12, "12m @12-31");
    // Sanity pin: ~1.1^12 - 1 given the 4dp-rounded fixture.
    expect_close(value_of(&f, "FIXTURE.KRX", "2020-12-31"), 1.1f64.powi(12) - 1.0, 1e-3, "12m ~1.1^12-1");
    assert!(value_of(&f, "FIXTURE.KRX", "2020-06-30").is_none(), "12m @06-30 NULL (no 2019-06-30 bar)");
}

#[test]
fn momentum_12_minus_1_hand_calculated() {
    let dir = tempdir().expect("temp");
    write_curated(dir.path(), "FIXTURE.KRX", 2019, &monthly_with_2019());
    write_curated(dir.path(), "FIXTURE.KRX", 2020, &monthly_growth_bars(12));
    let ctx = ctx_with(dir.path(), "2020-12-31", &["FIXTURE.KRX"]);
    let f = factor_engine::factors::MomentumFactor.compute(&ctx).expect("compute");

    // momentum = ref_1m / ref_12m - 1 = close(2020-11-30) / close(2019-12-31) - 1.
    expect_close(value_of(&f, "FIXTURE.KRX", "2020-12-31"), 259.3742 / 90.9091 - 1.0, 1e-12, "momentum @12-31");
    // Sanity: ~1.1^11 - 1 (months 2..12 of the monthly growth).
    expect_close(value_of(&f, "FIXTURE.KRX", "2020-12-31"), 1.1f64.powi(11) - 1.0, 1e-3, "momentum ~1.1^11-1");
    assert!(value_of(&f, "FIXTURE.KRX", "2020-06-30").is_none(), "momentum @06-30 NULL (no 12m ref)");
    assert!(value_of(&f, "FIXTURE.KRX", "2020-02-28").is_none(), "momentum @02-28 NULL (no 1m ref)");
}

#[test]
fn return_factor_metadata() {
    let r = ReturnFactor::one_month();
    assert_eq!(r.id(), "return_1m");
    assert_eq!(r.version().to_string(), "1.0.0");
    assert_eq!(r.required_fields().len(), 1);
    assert_eq!(
        factor_engine::contract::Lookback::CalendarMonths(1),
        r.lookback()
    );
    assert_eq!(
        factor_engine::contract::NullPolicy::InsufficientLookback,
        r.null_policy()
    );
    assert_eq!(ReturnFactor::three_months().id(), "return_3m");
    assert_eq!(ReturnFactor::six_months().id(), "return_6m");
    assert_eq!(ReturnFactor::twelve_months().id(), "return_12m");
    assert_eq!(factor_engine::factors::MomentumFactor.id(), "momentum_12_1");
}
