//! Red-phase tests: typed NULL / rejection behaviors of the documented null
//! policy - never a panic. Covers (a) short lookback, (b) zero-variance
//! normalization, (c) missing required field, (d) property invariants.

mod common;

use common::{FixtureBar, MARKET, TestCtx, VERSION, monthly_growth_bars, write_curated};
use domain::TradingDate;

use factor_engine::contract::{
    Factor, FactorContext, FactorError, FactorFrame, Field, Lookback, NullPolicy,
};
use factor_engine::factors::ReturnFactor;
use factor_engine::normalize::ZScorePolicy;
use factor_engine::snapshot::{FactorSnapshotBuilder, FrozenUniverse};
use tempfile::tempdir;

#[test]
fn short_lookback_is_typed_null_never_panic() {
    let dir = tempdir().expect("temp");
    write_curated(dir.path(), "SHORT.KRX", 2020, &monthly_growth_bars(2));
    let t = TestCtx::new(dir.path(), &["SHORT.KRX"], "2020-03-31");
    // Only 2 bars: 12-month return cannot exist at all - must be NULL, not a panic.
    let f = ReturnFactor::twelve_months()
        .compute(&t.ctx())
        .expect("compute");
    assert_eq!(f.rows.len(), 2);
    assert!(
        f.rows.iter().all(|r| r.value.is_none()),
        "all 12m values NULL"
    );
    let f = ReturnFactor::one_month()
        .compute(&t.ctx())
        .expect("compute");
    assert!(
        f.rows[0].value.is_none(),
        "1m @01-31 NULL (no bar in Dec 2019)"
    );
    let v = f.rows[1]
        .value
        .expect("1m @02-28 references the January bar");
    assert!((v - 0.1).abs() < 1e-12, "1m @02-28 = {v}");
}

/// A probe factor that declares a field the input layer does not carry.
struct HighProbe;

const HIGH_FIELDS: [Field; 1] = [Field::new("high")];

impl Factor for HighProbe {
    fn id(&self) -> &'static str {
        "high_probe"
    }
    fn version(&self) -> domain::FactorVersion {
        domain::FactorVersion::parse("1.0.0").expect("version")
    }
    fn required_fields(&self) -> &[Field] {
        &HIGH_FIELDS
    }
    fn lookback(&self) -> Lookback {
        Lookback::FullHistory
    }
    fn null_policy(&self) -> NullPolicy {
        NullPolicy::MissingRequiredField
    }
    fn compute(&self, _ctx: &FactorContext) -> Result<FactorFrame, FactorError> {
        unreachable!("compute must not run when a required field is missing")
    }
}

#[test]
fn missing_required_field_is_typed_error() {
    let dir = tempdir().expect("temp");
    write_curated(dir.path(), "A.KRX", 2020, &monthly_growth_bars(12));
    let universe = FrozenUniverse::new("universe-test-1", &["A.KRX"]);
    let store = market_data::CurateStore::new(dir.path());
    let err = FactorSnapshotBuilder::new(
        TradingDate::parse("2020-12-31").expect("as_of"),
        universe,
        &store,
        MARKET,
        common::DATASET_ID,
        VERSION,
    )
    .with_factors(vec![Box::new(HighProbe)])
    .build()
    .expect_err("missing field must be a typed error, not a panic");
    match err {
        FactorError::MissingField { factor, field } => {
            assert_eq!(factor, "high_probe");
            assert_eq!(field, "high");
        }
        other => panic!("expected MissingField, got {other}"),
    }
}

#[test]
fn zero_variance_normalization_is_typed_null() {
    let dir = tempdir().expect("temp");
    // Three symbols with IDENTICAL series: every cross-section has zero
    // variance (and meets the min-sample of 3), so the documented
    // zero-variance policy applies: all normalized values NULL, no panic.
    write_curated(dir.path(), "A.KRX", 2020, &monthly_growth_bars(12));
    write_curated(dir.path(), "B.KRX", 2020, &monthly_growth_bars(12));
    write_curated(dir.path(), "C.KRX", 2020, &monthly_growth_bars(12));
    let universe = FrozenUniverse::new("universe-test-1", &["A.KRX", "B.KRX", "C.KRX"]);
    let store = market_data::CurateStore::new(dir.path());
    let snap = FactorSnapshotBuilder::new(
        TradingDate::parse("2020-12-31").expect("as_of"),
        universe,
        &store,
        MARKET,
        common::DATASET_ID,
        VERSION,
    )
    .with_normalization(Box::new(ZScorePolicy::new(None)))
    .build()
    .expect("snapshot builds without panic");
    // Raw values exist; normalized values are all NULL (zero variance policy).
    let with_raw = snap
        .rows
        .iter()
        .filter(|r| r.factor == "return_1m" && r.raw.is_some())
        .count();
    assert!(with_raw > 0, "raw values exist");
    let normalized_non_null = snap
        .rows
        .iter()
        .filter(|r| r.factor == "return_1m" && r.normalized.is_some())
        .count();
    assert_eq!(
        normalized_non_null, 0,
        "zero variance -> NULL normalized values"
    );
}

#[test]
fn tiny_cross_section_normalization_is_null() {
    let dir = tempdir().expect("temp");
    write_curated(dir.path(), "A.KRX", 2020, &monthly_growth_bars(12));
    let universe = FrozenUniverse::new("universe-test-1", &["A.KRX"]);
    let store = market_data::CurateStore::new(dir.path());
    let snap = FactorSnapshotBuilder::new(
        TradingDate::parse("2020-12-31").expect("as_of"),
        universe,
        &store,
        MARKET,
        common::DATASET_ID,
        VERSION,
    )
    .build()
    .expect("snapshot builds");
    // Single-member cross-section is below the documented min sample (3):
    // raw values present, normalized NULL.
    assert!(snap.rows.iter().any(|r| r.raw.is_some()));
    assert!(
        snap.rows.iter().all(|r| r.normalized.is_none()),
        "normalized must be NULL below min sample"
    );
}

#[test]
fn drawdown_and_returns_are_finite_everywhere() {
    let dir = tempdir().expect("temp");
    write_curated(dir.path(), "A.KRX", 2020, &monthly_growth_bars(12));
    let t = TestCtx::new(dir.path(), &["A.KRX"], "2020-12-31");
    for f in [
        Box::new(ReturnFactor::one_month()) as Box<dyn Factor>,
        Box::new(ReturnFactor::twelve_months()),
        Box::new(factor_engine::factors::MomentumFactor),
        Box::new(factor_engine::factors::DrawdownFactor),
    ] {
        let frame = f.compute(&t.ctx()).expect("compute");
        for row in &frame.rows {
            if let Some(v) = row.value {
                assert!(v.is_finite(), "{} non-finite value {v}", f.id());
            }
        }
    }
}

#[test]
fn property_drawdown_bounded_by_min_over_series() {
    // Randomized: drawdown at every bar equals (close - running max)/running
    // max, which lies in [-1, 0]; the minimum over the series equals the
    // observed min of (close/cummax - 1).
    let dir = tempdir().expect("temp");
    let mut rng = 42u64;
    let closes: Vec<i64> = (0..200)
        .map(|_| {
            rng = rng
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (5_000 + (rng >> 33) % 10_000) as i64
        })
        .collect();
    let bars: Vec<FixtureBar> = closes
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let d = chrono::NaiveDate::from_ymd_opt(2020, 1, 1).expect("date")
                + chrono::Duration::days(i as i64);
            FixtureBar::new(&d.format("%Y-%m-%d").to_string(), &format!("{}.0000", c))
        })
        .collect();
    write_curated(dir.path(), "A.KRX", 2020, &bars);
    let t = TestCtx::new(dir.path(), &["A.KRX"], "2020-07-18");
    let f = factor_engine::factors::DrawdownFactor
        .compute(&t.ctx())
        .expect("compute");
    let mut running_max = f64::MIN;
    for (i, row) in f.rows.iter().enumerate() {
        let c = closes[i] as f64;
        running_max = running_max.max(c);
        let expected = c / running_max - 1.0;
        let v = row.value.expect("non-null");
        assert!((v - expected).abs() < 1e-12, "bar {i}: {v} vs {expected}");
        assert!((-1.0..=0.0).contains(&v), "bar {i}: {v}");
    }
}
