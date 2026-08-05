//! Red-phase tests: 20/60/120-day realized volatility = sqrt(252) x sample
//! std (ddof=1) of daily log returns, against hand-calculated fixtures.
//!
//! Documented null policy: a full window of N log returns is required, so the
//! first N bars of a series produce NULL (bar i has return i-1, i.e. N returns
//! need N+1 bars).

mod common;

use common::{FixtureBar, TestCtx, write_curated};

use factor_engine::contract::Factor;
use factor_engine::factors::RealizedVolFactor;
use tempfile::tempdir;

fn closes_bars(n: usize, close_at: impl Fn(usize) -> i64) -> Vec<FixtureBar> {
    let start = chrono::NaiveDate::from_ymd_opt(2020, 1, 1).expect("date");
    (0..n)
        .map(|i| {
            let d = start + chrono::Duration::days(i as i64);
            FixtureBar::new(
                &d.format("%Y-%m-%d").to_string(),
                &format!("{}.0000", close_at(i)),
            )
        })
        .collect()
}

fn nth_value(frame: &factor_engine::contract::FactorFrame, i: usize) -> Option<f64> {
    frame.rows[i].value
}

/// Reference calculation for the documented formula on the fixture's own
/// IEEE f64 closes: logret_i = ln(c_i) - ln(c_{i-1}); vol = sqrt(252) x
/// sample std (ddof = 1) over the trailing window of log returns.
fn expected_vol(closes: &[f64], window: usize, i: usize) -> Option<f64> {
    if i < window {
        return None;
    }
    let rets: Vec<f64> = (1..=i)
        .map(|k| closes[k].ln() - closes[k - 1].ln())
        .collect();
    let win = &rets[i - window..i];
    let n = win.len() as f64;
    let mean = win.iter().sum::<f64>() / n;
    let var = win.iter().map(|r| (r - mean) * (r - mean)).sum::<f64>() / (n - 1.0);
    Some(252.0f64.sqrt() * var.sqrt())
}

#[test]
fn vol_20_constant_series_is_zero_and_null_before_window() {
    let dir = tempdir().expect("temp");
    let n = 61;
    write_curated(dir.path(), "VOL.KRX", 2020, &closes_bars(n, |_| 100));
    let t = TestCtx::new(dir.path(), &["VOL.KRX"], "2020-03-02");
    let f = RealizedVolFactor::new(20)
        .expect("window")
        .compute(&t.ctx())
        .expect("compute");

    assert_eq!(f.rows.len(), n);
    for i in 0..20 {
        assert!(
            nth_value(&f, i).is_none(),
            "bar {i} NULL (fewer than 20 returns)"
        );
    }
    for i in 20..n {
        assert_eq!(
            nth_value(&f, i),
            Some(0.0),
            "bar {i} vol of constant series = 0.0"
        );
    }
}

#[test]
fn vol_20_hand_calculated_reference() {
    let dir = tempdir().expect("temp");
    let n = 41usize;
    // Non-constant closes: 100 + i with a jump pattern; expected computed from
    // the documented formula on the same f64 closes.
    let closes: Vec<i64> = (0..n)
        .map(|i| 100 + (i % 7) as i64 * 3 + (i / 7) as i64)
        .collect();
    write_curated(dir.path(), "VOL.KRX", 2020, &closes_bars(n, |i| closes[i]));
    let t = TestCtx::new(dir.path(), &["VOL.KRX"], "2020-02-10");
    let f = RealizedVolFactor::new(20)
        .expect("window")
        .compute(&t.ctx())
        .expect("compute");

    let f64s: Vec<f64> = closes.iter().map(|&c| c as f64).collect();
    for i in 0..n {
        let exp = expected_vol(&f64s, 20, i);
        match (nth_value(&f, i), exp) {
            (None, None) => {}
            (Some(a), Some(e)) => {
                assert!(
                    (a - e).abs() <= 1e-9 * e.abs().max(1.0),
                    "bar {i}: {a} vs {e}"
                );
            }
            (a, e) => panic!("bar {i}: expected {e:?} got {a:?}"),
        }
    }
    // The 21st bar (index 20) is the first with a full 20-return window.
    assert!(nth_value(&f, 20).is_some(), "bar 20 first non-null");
}

#[test]
fn vol_60_and_120_metadata() {
    let dir = tempdir().expect("temp");
    let n = 130;
    write_curated(
        dir.path(),
        "VOL.KRX",
        2020,
        &closes_bars(n, |i| 100 + i as i64),
    );
    let t = TestCtx::new(dir.path(), &["VOL.KRX"], "2020-05-10");
    let f60 = RealizedVolFactor::new(60)
        .expect("window")
        .compute(&t.ctx())
        .expect("compute");
    assert_eq!(f60.factor, "vol_60");
    assert!(
        nth_value(&f60, 59).is_none(),
        "bar 59 NULL (59 returns < 60)"
    );
    assert!(nth_value(&f60, 60).is_some(), "bar 60 first non-null");
    let f120 = RealizedVolFactor::new(120)
        .expect("window")
        .compute(&t.ctx())
        .expect("compute");
    assert_eq!(f120.factor, "vol_120");
    assert!(nth_value(&f120, 119).is_none(), "bar 119 NULL");
    assert!(
        RealizedVolFactor::new(10).is_err(),
        "unsupported window rejected"
    );
    assert_eq!(RealizedVolFactor::new(20).expect("w").id(), "vol_20");
}
