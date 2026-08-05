//! Manual QA channel: compute ALL MVP factors for the 11-symbol synthetic
//! Korean ETF universe (Todo 12 ids) on one as-of date, print the factor
//! table, then mutate the normalization universe and assert a DIFFERENT,
//! deterministic snapshot. Transcripts are captured from the test output.

mod common;

use common::{FixtureBar, MARKET, VERSION, write_curated};
use domain::TradingDate;
use factor_engine::snapshot::{FactorSnapshotBuilder, FrozenUniverse};
use market_data::CurateStore;
use tempfile::tempdir;

const UNIVERSE: [&str; 11] = [
    "069500.KRX",
    "102110.KRX",
    "229200.KRX",
    "143850.KRX",
    "133690.KRX",
    "195930.KRX",
    "192090.KRX",
    "148070.KRX",
    "114260.KRX",
    "153130.KRX",
    "132030.KRX",
];

fn synth_bars(symbol_idx: usize, n: usize, as_of: TradingDate) -> Vec<FixtureBar> {
    // Deterministic synthetic closes: base 10_000 + drift that grows with the
    // symbol index; a couple of symbols get a mid-series drawdown so the
    // drawdown factor is interesting.
    let start = chrono::NaiveDate::from_ymd_opt(2020, 1, 2).expect("date");
    let mut bars = Vec::with_capacity(n);
    for i in 0..n {
        let d = start + chrono::Duration::days(i as i64);
        if d > as_of.as_naive_date() {
            break;
        }
        let drift = (i as i64) * (symbol_idx as i64 + 1) * 3;
        let dip = if symbol_idx % 3 == 2 && i > n / 2 {
            -2_000
        } else {
            0
        };
        let close = 10_000 + drift + dip;
        let tv = 100_000_000 + (symbol_idx as i64) * 1_000_000 + i as i64 * 10_000;
        bars.push(FixtureBar {
            date: d.format("%Y-%m-%d").to_string(),
            close: format!("{close}.0000"),
            volume: 1_000 + i as i64,
            trading_value: Some(tv),
        });
    }
    bars
}

fn write_all(dir: &std::path::Path, as_of: TradingDate) {
    for (idx, symbol) in UNIVERSE.iter().enumerate() {
        write_curated(dir, symbol, 2020, &synth_bars(idx, 120, as_of));
    }
}

fn build(
    dir: &std::path::Path,
    universe: &[&str],
    as_of: &str,
) -> factor_engine::snapshot::FactorSnapshot {
    let frozen = FrozenUniverse::new("universe-snap-11", universe);
    let store = CurateStore::new(dir);
    FactorSnapshotBuilder::new(
        TradingDate::parse(as_of).expect("as_of"),
        frozen,
        &store,
        MARKET,
        common::DATASET_ID,
        VERSION,
    )
    .build()
    .expect("snapshot builds")
}

#[test]
fn compute_all_factors_for_11_symbol_universe() {
    let dir = tempdir().expect("temp");
    let as_of = "2020-04-30";
    let as_of_date = TradingDate::parse(as_of).expect("as_of");
    write_all(dir.path(), as_of_date);

    let snap = build(dir.path(), &UNIVERSE, as_of);
    assert!(!snap.rows.is_empty());

    // All 11 symbols present on the as-of date for every factor.
    let as_of_rows: Vec<_> = snap.rows.iter().filter(|r| r.date == as_of).collect();
    let factors: std::collections::BTreeSet<&str> =
        as_of_rows.iter().map(|r| r.factor.as_str()).collect();
    assert_eq!(factors.len(), 13, "all 13 MVP factors on as-of date");
    for symbol in UNIVERSE {
        for factor in &factors {
            assert!(
                as_of_rows
                    .iter()
                    .any(|r| r.instrument == symbol && r.factor == *factor),
                "{symbol} {factor} row on as-of date"
            );
        }
    }

    // Print the factor table (transcript): as-of row, per symbol, raw +
    // normalized, one line per factor.
    println!(
        "=== FACTOR TABLE as_of={as_of} universe={} (snapshot {}) ===",
        UNIVERSE.len(),
        &snap.hash.as_str()[..16]
    );
    let mut order: Vec<&str> = factors.iter().copied().collect();
    order.sort();
    println!("symbol,{}", order.join(","));
    for symbol in UNIVERSE {
        let mut line = String::from(symbol);
        for factor in &order {
            let v = as_of_rows
                .iter()
                .find(|r| r.instrument == symbol && r.factor == *factor)
                .and_then(|r| r.raw)
                .map(|x| format!("{x:.6}"))
                .unwrap_or_else(|| "-".to_owned());
            line.push(',');
            line.push_str(&v);
        }
        println!("{line}");
    }

    // Sanity: monotone-up symbols have drawdown ~0 at the end; dip symbols < 0.
    let dd = |symbol: &str| {
        as_of_rows
            .iter()
            .find(|r| r.instrument == symbol && r.factor == "drawdown")
            .and_then(|r| r.raw)
            .expect("drawdown non-null")
    };
    assert!(
        dd("069500.KRX") > -1e-9,
        "monotone symbol near-zero drawdown"
    );
    assert!(dd("114260.KRX") < 0.0, "dip symbol negative drawdown");

    // Determinism: rebuilding produces the identical snapshot.
    let again = build(dir.path(), &UNIVERSE, as_of);
    assert_eq!(
        snap.canonical_bytes().expect("bytes"),
        again.canonical_bytes().expect("bytes")
    );
    assert_eq!(
        snap.hash.as_str(),
        again.hash.as_str(),
        "rebuild is byte-identical"
    );
}

#[test]
fn mutated_normalization_universe_yields_different_deterministic_snapshot() {
    let dir = tempdir().expect("temp");
    let as_of = "2020-04-30";
    write_all(dir.path(), TradingDate::parse(as_of).expect("as_of"));

    let full = build(dir.path(), &UNIVERSE, as_of);
    let reduced: Vec<&str> = UNIVERSE[..10].to_vec(); // drop 132030.KRX
    let cut = build(dir.path(), &reduced, as_of);

    // The normalization universe is frozen per build: hashes must differ.
    assert_ne!(
        full.hash.as_str(),
        cut.hash.as_str(),
        "universe mutation must change the snapshot"
    );
    // The removed symbol is absent from rows; the others keep every row.
    assert!(cut.rows.iter().all(|r| r.instrument != "132030.KRX"));
    assert!(full.rows.iter().any(|r| r.instrument == "132030.KRX"));
    // Deterministic: rebuilding the reduced universe repeats its hash.
    let cut_again = build(dir.path(), &reduced, as_of);
    assert_eq!(cut.hash.as_str(), cut_again.hash.as_str());

    println!(
        "=== UNIVERSE MUTATION full={} cut={} (identical rebuild {}) ===",
        &full.hash.as_str()[..16],
        &cut.hash.as_str()[..16],
        cut.hash.as_str() == cut_again.hash.as_str()
    );
    // Normalized values shift because the cross-section changed (spot-check
    // one instrument whose raw value is unchanged but z-score moves).
    let raw_of = |snap: &factor_engine::snapshot::FactorSnapshot, sym: &str, factor: &str| {
        snap.rows
            .iter()
            .find(|r| r.instrument == sym && r.date == as_of && r.factor == factor)
            .and_then(|r| r.raw)
    };
    let a_raw = raw_of(&full, "069500.KRX", "avg_value_20").expect("raw");
    assert_eq!(
        a_raw,
        raw_of(&cut, "069500.KRX", "avg_value_20").expect("raw"),
        "raw unchanged"
    );
    let z_full = full
        .rows
        .iter()
        .find(|r| r.instrument == "069500.KRX" && r.date == as_of && r.factor == "avg_value_20")
        .and_then(|r| r.normalized)
        .expect("full z");
    let z_cut = cut
        .rows
        .iter()
        .find(|r| r.instrument == "069500.KRX" && r.date == as_of && r.factor == "avg_value_20")
        .and_then(|r| r.normalized)
        .expect("cut z");
    assert!(
        (z_full - z_cut).abs() > 1e-6,
        "cross-section changed -> z-score changed (full {z_full}, cut {z_cut})"
    );
    println!("=== z-score of 069500.KRX avg_value_20: full={z_full:.6} cut={z_cut:.6} ===");
}
