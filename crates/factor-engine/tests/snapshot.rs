//! Red-phase tests: deterministic, hash-stable factor snapshots.
//!
//! - identical inputs -> identical canonical bytes AND identical hash;
//! - a changed factor version -> a different hash;
//! - the cross-sectional universe is FROZEN per date: instruments outside the
//!   frozen universe never appear in rows nor in normalization; each date's
//!   normalization uses exactly that date's members;
//! - normalization math is hand-verified.

mod common;

use common::{FixtureBar, MARKET, VERSION, write_curated};
use domain::{FactorVersion, TradingDate};
use factor_engine::contract::{
    Factor, FactorContext, FactorError, FactorFrame, Field, Lookback, NullPolicy,
};
use factor_engine::factors::ReturnFactor;
use factor_engine::normalize::{NormalizePolicy, PercentilePolicy, WinsorizePolicy, ZScorePolicy};
use factor_engine::snapshot::{FactorSnapshot, FactorSnapshotBuilder, FrozenUniverse};
use market_data::CurateStore;
use tempfile::tempdir;

fn monthly_bars_for(symbol: &str, dir: &std::path::Path) {
    let closes = [
        "100.0000", "110.0000", "121.0000", "133.1000", "146.4100", "161.0510",
    ];
    let dates = [
        "2020-01-31",
        "2020-02-28",
        "2020-03-31",
        "2020-04-30",
        "2020-05-29",
        "2020-06-30",
    ];
    let bars: Vec<FixtureBar> = dates
        .iter()
        .zip(closes)
        .map(|(d, c)| FixtureBar::new(d, c))
        .collect();
    write_curated(dir, symbol, 2020, &bars);
}

fn build_default(dir: &std::path::Path, symbols: &[&str], as_of: &str) -> FactorSnapshot {
    let universe = FrozenUniverse::new("universe-snap-1", symbols);
    let store = CurateStore::new(dir);
    FactorSnapshotBuilder::new(
        TradingDate::parse(as_of).expect("as_of"),
        universe,
        &store,
        MARKET,
        common::DATASET_ID,
        VERSION,
    )
    .build()
    .expect("snapshot builds")
}

#[test]
fn identical_builds_hash_identically() {
    let dir = tempdir().expect("temp");
    monthly_bars_for("A.KRX", dir.path());
    monthly_bars_for("B.KRX", dir.path());
    let a = build_default(dir.path(), &["A.KRX", "B.KRX"], "2020-06-30");
    let b = build_default(dir.path(), &["A.KRX", "B.KRX"], "2020-06-30");
    assert_eq!(
        a.canonical_bytes().expect("bytes"),
        b.canonical_bytes().expect("bytes")
    );
    assert_eq!(a.hash.as_str(), b.hash.as_str());
    assert_eq!(a.hash.as_str(), a.compute_hash().expect("hash").as_str());
    println!(
        "=== IDENTICAL BUILDS hash={} bytes={} (equal rebuild {}) ===",
        a.hash.as_str(),
        a.canonical_bytes().expect("bytes").len(),
        a.hash == b.hash
    );
}

#[test]
fn changed_factor_version_changes_hash() {
    let dir = tempdir().expect("temp");
    monthly_bars_for("A.KRX", dir.path());
    let base = build_default(dir.path(), &["A.KRX"], "2020-06-30");

    // Same math, different factor version: the snapshot hash MUST change.
    struct Return1mV2;
    impl Factor for Return1mV2 {
        fn id(&self) -> &'static str {
            "return_1m"
        }
        fn version(&self) -> FactorVersion {
            FactorVersion::parse("1.0.1").expect("version")
        }
        fn required_fields(&self) -> &[Field] {
            &[Field::CLOSE]
        }
        fn lookback(&self) -> Lookback {
            Lookback::CalendarMonths(1)
        }
        fn null_policy(&self) -> NullPolicy {
            NullPolicy::InsufficientLookback
        }
        fn compute(&self, ctx: &FactorContext) -> Result<FactorFrame, FactorError> {
            ReturnFactor::one_month().compute(ctx)
        }
    }
    let universe = FrozenUniverse::new("universe-snap-1", &["A.KRX"]);
    let store = CurateStore::new(dir.path());
    let v2 = FactorSnapshotBuilder::new(
        TradingDate::parse("2020-06-30").expect("as_of"),
        universe,
        &store,
        MARKET,
        common::DATASET_ID,
        VERSION,
    )
    .with_factors(vec![Box::new(Return1mV2)])
    .build()
    .expect("v2 builds");
    assert_eq!(v2.factor_versions["return_1m"], "1.0.1");
    assert_ne!(
        base.hash.as_str(),
        v2.hash.as_str(),
        "version change must alter the hash"
    );
    println!(
        "=== VERSION CHANGE base={} v1.0.1={} (different {}) ===",
        base.hash.as_str(),
        v2.hash.as_str(),
        base.hash != v2.hash
    );
}

#[test]
fn changed_price_changes_hash() {
    let dir = tempdir().expect("temp");
    monthly_bars_for("A.KRX", dir.path());
    let base = build_default(dir.path(), &["A.KRX"], "2020-06-30");
    // Rewrite A with a different last close and rebuild.
    let bars = vec![
        FixtureBar::new("2020-01-31", "100.0000"),
        FixtureBar::new("2020-02-28", "110.0000"),
        FixtureBar::new("2020-03-31", "121.0000"),
        FixtureBar::new("2020-04-30", "133.1000"),
        FixtureBar::new("2020-05-29", "146.4100"),
        FixtureBar::new("2020-06-30", "170.0000"),
    ];
    write_curated(dir.path(), "A.KRX", 2020, &bars);
    let changed = build_default(dir.path(), &["A.KRX"], "2020-06-30");
    assert_ne!(base.hash.as_str(), changed.hash.as_str());
}

#[test]
fn universe_is_frozen_per_date() {
    let dir = tempdir().expect("temp");
    // A = 100, 110, ... ; B = same shape; C = same shape but NOT in universe.
    monthly_bars_for("A.KRX", dir.path());
    monthly_bars_for("B.KRX", dir.path());
    monthly_bars_for("C.KRX", dir.path());

    let snap = build_default(dir.path(), &["A.KRX", "B.KRX"], "2020-06-30");
    // C never appears in rows, even though its data is on disk.
    assert!(
        snap.rows.iter().all(|r| r.instrument != "C.KRX"),
        "C excluded from rows"
    );
    assert!(snap.rows.iter().any(|r| r.instrument == "A.KRX"));

    // Cross-section on 2020-06-30: A and B only (2 < min sample 3):
    // return_1m @06-30: A = 161.0510/146.4100 - 1 = 0.1; B identical -> both 0.1.
    // mean = 0.1, sample-variance = 0 -> zero variance -> NULL normalized.
    let a_rows: Vec<_> = snap
        .rows
        .iter()
        .filter(|r| r.factor == "return_1m" && r.instrument == "A.KRX" && r.date == "2020-06-30")
        .collect();
    assert_eq!(a_rows.len(), 1);
    assert!(
        a_rows[0].normalized.is_none(),
        "zero-variance cross-section -> NULL"
    );
}

#[test]
fn normalization_uses_per_date_cross_section() {
    let dir = tempdir().expect("temp");
    // 25 daily bars per symbol; trading value constant per symbol:
    // A = 10_000, B = 20_000, D = 30_000 (in universe); C = 30_000 (out).
    let bars_for = |value: i64| -> Vec<FixtureBar> {
        (0..25)
            .map(|i| {
                let d = chrono::NaiveDate::from_ymd_opt(2020, 1, 1).expect("date")
                    + chrono::Duration::days(i as i64);
                FixtureBar {
                    date: d.format("%Y-%m-%d").to_string(),
                    close: "100.0000".to_owned(),
                    volume: 1_000,
                    trading_value: Some(value),
                }
            })
            .collect()
    };
    write_curated(dir.path(), "A.KRX", 2020, &bars_for(10_000));
    write_curated(dir.path(), "B.KRX", 2020, &bars_for(20_000));
    write_curated(dir.path(), "D.KRX", 2020, &bars_for(30_000));
    write_curated(dir.path(), "C.KRX", 2020, &bars_for(30_000));

    let snap = build_default(dir.path(), &["A.KRX", "B.KRX", "D.KRX"], "2020-01-25");
    let rows = |sym: &str, factor: &str| {
        snap.rows
            .iter()
            .filter(|r| r.instrument == sym && r.factor == factor && r.raw.is_some())
            .cloned()
            .collect::<Vec<_>>()
    };
    let a = &rows("A.KRX", "avg_value_20")[0];
    let b = &rows("B.KRX", "avg_value_20")[0];
    let d = &rows("D.KRX", "avg_value_20")[0];
    assert_eq!(a.raw, Some(10_000.0));
    assert_eq!(b.raw, Some(20_000.0));
    assert_eq!(d.raw, Some(30_000.0));
    // Population std over {10k, 20k, 30k}: mean 20k, sd sqrt(200M/3):
    // z_A = -1.2247..., z_B = 0, z_D = +1.2247...
    let mean: f64 = 20_000.0;
    let sd = (((10_000.0 - mean).powi(2) + (20_000.0 - mean).powi(2) + (30_000.0 - mean).powi(2))
        / 3.0)
        .sqrt();
    assert!((a.normalized.expect("A normalized") - (10_000.0 - mean) / sd).abs() < 1e-9);
    assert!(
        (b.normalized.expect("B normalized") - 0.0).abs() < 1e-9,
        "B at the mean -> z 0"
    );
    assert!((d.normalized.expect("D normalized") - (30_000.0 - mean) / sd).abs() < 1e-9);
    // Out-of-universe C must never appear in ANY row or cross-section.
    assert!(snap.rows.iter().all(|r| r.instrument != "C.KRX"));
}

#[test]
fn percentile_policy_hand_calculated() {
    let xs: Vec<Option<f64>> = [1.0, 2.0, 3.0, 4.0, 5.0].into_iter().map(Some).collect();
    let p = PercentilePolicy;
    let out = p.apply(&xs);
    // pct(x) = (# values < x) / (n - 1) -> 0, .25, .5, .75, 1.
    let expected = [0.0, 0.25, 0.5, 0.75, 1.0];
    for (o, e) in out.iter().zip(expected) {
        assert!((o.expect("non-null") - e).abs() < 1e-12, "{o:?} vs {e}");
    }
}

#[test]
fn winsorize_policy_hand_calculated() {
    let xs: Vec<Option<f64>> = [1.0, 2.0, 3.0, 100.0].into_iter().map(Some).collect();
    let w = WinsorizePolicy::new(0.25, 0.75).expect("valid");
    let out = w.apply(&xs);
    // sorted [1,2,3,100]: lower = sorted[floor((4-1)*0.25)] = sorted[0] = 1.0;
    // upper = sorted[ceil((4-1)*0.75)] = sorted[3] = 100.0 -> no clipping here.
    assert_eq!(out, xs);
    // Extreme outlier case: upper clip binds.
    let w2 = WinsorizePolicy::new(0.25, 0.75).expect("valid");
    let xs2: Vec<Option<f64>> = [1.0, 2.0, 3.0, 4.0, 5.0, 1_000.0]
        .into_iter()
        .map(Some)
        .collect();
    let out2 = w2.apply(&xs2);
    // sorted [1,2,3,4,5,1000]: lower = sorted[floor(5*0.25)] = sorted[1] = 2.0;
    // upper = sorted[ceil(5*0.75)] = sorted[4] = 5.0
    assert_eq!(out2[5], Some(5.0), "outlier clipped to upper quantile");
    assert_eq!(out2[0], Some(2.0), "low value clipped to lower quantile");
    assert_eq!(out2[3], Some(4.0), "middle values untouched");
}

#[test]
fn zscore_cap_and_null_hand_calculated() {
    let z = ZScorePolicy::new(Some(2.0));
    // [10, 20, 30]: mean 20, population sd sqrt(200/3) -> z = +-1.2247..., 0.
    let xs: Vec<Option<f64>> = [10.0, 20.0, 30.0].into_iter().map(Some).collect();
    let out = z.apply(&xs);
    let mean: f64 = 20.0;
    let sd = (((10.0 - mean).powi(2) + (20.0 - mean).powi(2) + (30.0 - mean).powi(2)) / 3.0).sqrt();
    let za = out[0].expect("z");
    let zb = out[1].expect("z");
    let zc = out[2].expect("z");
    assert!((za - (10.0 - mean) / sd).abs() < 1e-12, "z of 10: {za}");
    assert!((zb - 0.0).abs() < 1e-12, "z of 20 (the mean): {zb}");
    assert!((zc - (30.0 - mean) / sd).abs() < 1e-12, "z of 30: {zc}");
    assert!((za + zc).abs() < 1e-12, "symmetric z-scores");
    // Cap: an extreme outlier is clipped to +-cap. Five 1s + 1000 gives
    // z_max = sqrt(5) ~ 2.236 uncapped (hand-derived), clipped to 2.0.
    let xs2: Vec<Option<f64>> = [1.0, 1.0, 1.0, 1.0, 1.0, 1000.0]
        .into_iter()
        .map(Some)
        .collect();
    let out2 = z.apply(&xs2);
    let uncapped = ZScorePolicy::new(None).apply(&xs2);
    assert!(
        out2[5].expect("capped") == 2.0,
        "outlier clipped exactly to +cap"
    );
    assert!(
        (uncapped[5].expect("raw z") - 5f64.sqrt()).abs() < 1e-9,
        "uncapped z = sqrt(5)"
    );
    assert!(
        out2.iter()
            .all(|o| o.expect("non-null").abs() <= 2.0 + 1e-12),
        "all capped"
    );
    // Nulls pass through as nulls; too-small cross-section -> all null.
    let xs3: Vec<Option<f64>> = [Some(1.0), None, Some(2.0), Some(3.0)]
        .into_iter()
        .collect();
    let out3 = z.apply(&xs3);
    assert_eq!(out3[1], None);
    assert!(out3[0].is_some() && out3[2].is_some() && out3[3].is_some());
    let tiny: Vec<Option<f64>> = [Some(1.0)].into_iter().collect();
    assert!(
        z.apply(&tiny).iter().all(Option::is_none),
        "below min sample -> NULL"
    );
}

#[test]
fn snapshot_metadata_is_recorded() {
    let dir = tempdir().expect("temp");
    monthly_bars_for("A.KRX", dir.path());
    let snap = build_default(dir.path(), &["A.KRX"], "2020-06-30");
    assert_eq!(snap.as_of, TradingDate::parse("2020-06-30").expect("date"));
    assert_eq!(snap.universe_snapshot_id, "universe-snap-1");
    assert_eq!(snap.dataset_version, VERSION);
    assert_eq!(snap.factor_versions["return_1m"], "1.0.0");
    assert_eq!(snap.factor_versions["drawdown"], "1.0.0");
    assert_eq!(snap.factor_versions.len(), 13, "all MVP factors versioned");
    assert_eq!(snap.normalization.id, "z_score");
    assert_eq!(snap.normalization.version, "1.0.0");
    // Rows sorted deterministically: (date, instrument, factor).
    let keys: Vec<(&str, &str, &str)> = snap
        .rows
        .iter()
        .map(|r| (r.date.as_str(), r.instrument.as_str(), r.factor.as_str()))
        .collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted, "rows are canonically ordered");
    assert!(snap.rows.iter().all(|r| r.instrument == "A.KRX"));
}
