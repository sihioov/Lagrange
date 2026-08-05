//! Red-phase tests: the documented Factor contract surface and the versioned
//! MVP factor registry (design §6.5, requirements FR-SEL-002).

use domain::FactorVersion;
use factor_engine::contract::{Factor, Field, Lookback, NullPolicy};
use factor_engine::factors;

#[test]
fn mvp_registry_has_all_documented_factors() {
    let registry = factors::all_mvp_factors();
    let ids: Vec<&'static str> = registry.iter().map(|f| f.id()).collect();
    assert_eq!(
        ids,
        vec![
            "return_1m",
            "return_3m",
            "return_6m",
            "return_12m",
            "momentum_12_1",
            "trend_50",
            "trend_100",
            "trend_200",
            "vol_20",
            "vol_60",
            "vol_120",
            "avg_value_20",
            "drawdown",
        ]
    );
}

#[test]
fn every_factor_is_versioned_with_metadata() {
    for f in factors::all_mvp_factors() {
        let id = f.id();
        assert!(!id.is_empty(), "factor id");
        assert!(f.version().to_string().len() >= 5, "{id}: semver version");
        assert!(!f.required_fields().is_empty(), "{id}: required fields");
        match f.lookback() {
            Lookback::CalendarMonths(m) => assert!(m > 0, "{id} months"),
            Lookback::TradingDays {
                window,
                min_periods,
            } => {
                assert!(
                    window > 0 && min_periods > 0 && min_periods <= window,
                    "{id} window"
                );
            }
            Lookback::FixedWindow {
                window,
                min_periods,
            } => {
                assert!(
                    window > 0 && min_periods > 0 && min_periods <= window,
                    "{id} window"
                );
            }
            Lookback::FullHistory => {}
        }
        let _ = f.null_policy();
    }
}

#[test]
fn field_constants_are_documented() {
    assert_eq!(Field::CLOSE.as_str(), "close");
    assert_eq!(Field::TRADING_VALUE.as_str(), "trading_value");
}

#[test]
fn factor_versions_are_parseable_semver() {
    for f in factors::all_mvp_factors() {
        assert_eq!(
            FactorVersion::parse(&f.version().to_string())
                .expect("semver")
                .to_string(),
            f.version().to_string()
        );
    }
}

#[test]
fn null_policy_is_inspectable() {
    use factors::{AvgValueFactor, DrawdownFactor, MomentumFactor, RealizedVolFactor};
    assert_eq!(
        MomentumFactor.null_policy(),
        NullPolicy::InsufficientLookback
    );
    assert_eq!(
        RealizedVolFactor::new(20).expect("w").null_policy(),
        NullPolicy::InsufficientLookback
    );
    assert_eq!(AvgValueFactor.null_policy(), NullPolicy::StrictWindow);
    assert_eq!(
        DrawdownFactor.null_policy(),
        NullPolicy::InsufficientLookback
    );
}
