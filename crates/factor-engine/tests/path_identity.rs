//! Curated partition identity is part of the factor input contract.

#[allow(dead_code)]
mod common;

use common::{FixtureBar, MARKET, VERSION, adjusted_row, raw_row};
use domain::TradingDate;
use factor_engine::bars::Bars;
use factor_engine::{FactorError, FrozenUniverse};
use market_data::CurateStore;
use market_data::curate::schema::{write_adjusted_bars, write_bars};
use tempfile::tempdir;

fn load_one(
    path_symbol: &str,
    raw_symbol: &str,
    adjusted_symbol: &str,
) -> Result<Bars, FactorError> {
    let dir = tempdir().expect("tempdir");
    let store = CurateStore::new(dir.path());
    let bar = FixtureBar::new("2020-01-31", "100.0000");
    write_bars(
        &store.bars_path(MARKET, path_symbol, 2020, VERSION),
        &[raw_row(raw_symbol, &bar)],
    )
    .expect("write raw fixture");
    write_adjusted_bars(
        &store.adjusted_bars_path(MARKET, path_symbol, 2020, VERSION),
        &[adjusted_row(adjusted_symbol, &bar)],
    )
    .expect("write adjusted fixture");
    Bars::from_curated(
        &store,
        MARKET,
        common::DATASET_ID,
        VERSION,
        &FrozenUniverse::new("path-identity", &[path_symbol]),
        TradingDate::parse("2020-01-31").unwrap(),
    )
}

#[test]
fn raw_row_identity_must_match_its_symbol_partition() {
    let Err(error) = load_one("B.KRX", "A.KRX", "B.KRX") else {
        panic!("a raw row cannot be attributed to its directory name");
    };
    assert!(
        matches!(error, FactorError::CorruptData { .. }),
        "{error:?}"
    );
}

#[test]
fn adjusted_row_identity_must_match_its_symbol_partition() {
    let Err(error) = load_one("B.KRX", "B.KRX", "A.KRX") else {
        panic!("an adjusted row cannot be attributed to its directory name");
    };
    assert!(
        matches!(error, FactorError::CorruptData { .. }),
        "{error:?}"
    );
}

#[test]
fn raw_and_adjusted_dates_must_align_exactly() {
    let dir = tempdir().expect("tempdir");
    let store = CurateStore::new(dir.path());
    let jan = FixtureBar::new("2020-01-30", "99.0000");
    let close = FixtureBar::new("2020-01-31", "100.0000");
    write_bars(
        &store.bars_path(MARKET, "A.KRX", 2020, VERSION),
        &[raw_row("A.KRX", &jan), raw_row("A.KRX", &close)],
    )
    .expect("write raw fixture");
    write_adjusted_bars(
        &store.adjusted_bars_path(MARKET, "A.KRX", 2020, VERSION),
        &[adjusted_row("A.KRX", &close)],
    )
    .expect("write adjusted fixture");

    let Err(error) = Bars::from_curated(
        &store,
        MARKET,
        common::DATASET_ID,
        VERSION,
        &FrozenUniverse::new("date-alignment", &["A.KRX"]),
        TradingDate::parse("2020-01-31").unwrap(),
    ) else {
        panic!("an unmatched raw row must not be silently ignored");
    };
    assert!(
        matches!(error, FactorError::CorruptData { .. }),
        "{error:?}"
    );
}
