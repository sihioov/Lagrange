use selector::baseline::{BASELINE_STRATEGY_IDS, baseline_packages};
use serde::Deserialize;
use serde_json::Value;

const CATALOG_BYTES: &[u8] = include_bytes!("../../../configs/strategies/baseline-v1.json");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    schema_id: String,
    schema_version: u64,
    released_at_epoch: u64,
    strategies: Vec<CatalogStrategy>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogStrategy {
    strategy_id: String,
    version: String,
    display_name: String,
    description: String,
    risk_description: String,
    state: String,
    required_factors: Vec<String>,
    min_lookback: u64,
    supported_market: String,
    cadence: String,
    parameter_schema: Value,
    default_parameters: Value,
}

#[test]
fn immutable_release_catalog_exactly_projects_the_code_registry() {
    assert_eq!(CATALOG_BYTES.last(), Some(&b'\n'));
    let catalog: Catalog =
        serde_json::from_slice(CATALOG_BYTES).expect("catalog must be valid JSON");
    assert_eq!(catalog.schema_id, "lagrange-baseline-strategy-catalog");
    assert_eq!(catalog.schema_version, 1);
    assert_eq!(catalog.released_at_epoch, 1_787_670_000);
    assert_eq!(catalog.strategies.len(), BASELINE_STRATEGY_IDS.len());

    let packages = baseline_packages();
    assert_eq!(packages.len(), catalog.strategies.len());
    for (expected_id, (record, package)) in BASELINE_STRATEGY_IDS
        .iter()
        .zip(catalog.strategies.iter().zip(packages.iter()))
    {
        assert_eq!(record.strategy_id, *expected_id);
        assert_eq!(record.strategy_id, package.strategy_id);
        assert_eq!(record.version, package.version.to_string());
        assert_eq!(record.display_name, package.name);
        assert_eq!(record.description, package.description);
        assert_eq!(record.risk_description, package.risk_description);
        assert_eq!(record.state, package.state.to_string());

        let mut factors = package.required_factors.iter().cloned().collect::<Vec<_>>();
        factors.sort();
        assert_eq!(record.required_factors, factors);
        assert_eq!(record.min_lookback, package.minimum_lookback_sessions);
        assert_eq!(
            record.supported_market,
            package.markets[0].as_str().to_uppercase()
        );
        assert_eq!(record.cadence, package.cadences[0].as_str());
        assert_eq!(record.parameter_schema, package.parameter_schema);
        assert_eq!(record.default_parameters, package.default_parameters);
    }
}
