//! Red-phase tests: future-dated rows are rejected with a typed error - and
//! only rows of instruments INSIDE the frozen universe are ever read.

mod common;

use common::{MARKET, VERSION, FixtureBar, monthly_growth_bars, write_curated};
use domain::TradingDate;
use factor_engine::contract::FactorError;
use factor_engine::snapshot::{FactorSnapshotBuilder, FrozenUniverse};
use market_data::CurateStore;
use tempfile::tempdir;

#[test]
fn future_dated_row_is_typed_rejection() {
    let dir = tempdir().expect("temp");
    let mut bars = monthly_growth_bars(12);
    bars.push(FixtureBar::new("2021-01-29", "300.0000")); // after as-of 2020-12-31
    write_curated(dir.path(), "A.KRX", 2021, &bars);
    let universe = FrozenUniverse::new("universe-test-1", &["A.KRX"]);
    let store = CurateStore::new(dir.path());
    let err = FactorSnapshotBuilder::new(
        TradingDate::parse("2020-12-31").expect("as_of"),
        universe,
        &store,
        MARKET,
        common::DATASET_ID,
        VERSION,
    )
    .build()
    .expect_err("future-dated row must be rejected");
    match err {
        FactorError::FutureDatedRow { instrument, date, as_of } => {
            assert_eq!(instrument, "A.KRX");
            assert_eq!(date, "2021-01-29");
            assert_eq!(as_of, "2020-12-31");
        }
        other => panic!("expected FutureDatedRow, got {other}"),
    }
}

#[test]
fn future_row_outside_universe_is_never_read() {
    let dir = tempdir().expect("temp");
    // FUTURE.KRX has a bar after as-of but is NOT in the frozen universe:
    // the engine must not even read it, so the build succeeds.
    write_curated(dir.path(), "A.KRX", 2020, &monthly_growth_bars(12));
    write_curated(dir.path(), "FUTURE.KRX", 2021, &[FixtureBar::new("2021-06-30", "1.0000")]);
    let universe = FrozenUniverse::new("universe-test-1", &["A.KRX"]);
    let store = CurateStore::new(dir.path());
    let snap = FactorSnapshotBuilder::new(
        TradingDate::parse("2020-12-31").expect("as_of"),
        universe,
        &store,
        MARKET,
        common::DATASET_ID,
        VERSION,
    )
    .build()
    .expect("universe gates the read");
    assert!(snap.rows.iter().all(|r| r.instrument == "A.KRX"));
}

#[test]
fn bar_exactly_on_as_of_is_included() {
    let dir = tempdir().expect("temp");
    write_curated(dir.path(), "A.KRX", 2020, &monthly_growth_bars(12));
    let universe = FrozenUniverse::new("universe-test-1", &["A.KRX"]);
    let store = CurateStore::new(dir.path());
    let snap = FactorSnapshotBuilder::new(
        TradingDate::parse("2020-12-31").expect("as_of"),
        universe,
        &store,
        MARKET,
        common::DATASET_ID,
        VERSION,
    )
    .build()
    .expect("as-of bar included");
    assert!(
        snap.rows.iter().any(|r| r.date == "2020-12-31"),
        "as-of date row must be present"
    );
}
