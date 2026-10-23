//! Existing owner-beta factor and target snapshots mapped into simulation
//! signals. No factor arithmetic or target rule is reimplemented here.

use std::collections::BTreeMap;

use domain::{ContentHash, TradingDate};
use factor_engine::contract::{Lookback, NullPolicy};
use factor_engine::{Factor, Field};
use market_data::{ApprovedHistoricalPriceOnlyArtifact, KR_ETF_CORE_SYMBOLS};
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use crate::{
    factor_series::factors_for,
    owner_beta::{
        OwnerBetaComputation, OwnerBetaPriceRecommendationInput,
        compute::compute_owner_beta_price_recommendation_with_approved,
    },
    recommendation::compute::requirements_for,
    resolver::ResolvedConfig,
};

use super::{SignalSnapshot, SimulationSchedule, canonical::canonical_json};

const STRATEGY_ID: &str = "trend_following";
const STRATEGY_VERSION: &str = "1.0.0";
const FAST_FACTOR_ID: &str = "trend_50";
const SLOW_FACTOR_ID: &str = "trend_200";
const BENCHMARK_INSTRUMENT: &str = "069500.KRX";
const FULL_WEIGHT_PPM: u32 = 1_000_000;
const FACTOR_DEFINITION_SCHEMA: &str = "owner-beta-trend-factor-definition-v1";

const FACTOR_CONTRACT_SOURCE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../factor-engine/src/contract.rs"
));
const TREND_IMPLEMENTATION_SOURCE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../factor-engine/src/factors/trend.rs"
));
const TREND_SUPPORT_SOURCE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../factor-engine/src/lazy_util.rs"
));

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SimulationSignalError {
    #[error("existing owner-beta factor or target computation failed")]
    ComputationRejected,
    #[error("existing factor definition identity is unavailable")]
    FactorIdentityUnavailable,
    #[error("existing owner-beta signal snapshot is invalid")]
    SignalInvalid,
}

pub(crate) struct ComputedSimulationSignals {
    pub(crate) signals: Vec<SignalSnapshot>,
    pub(crate) strategy_snapshot_sha256: ContentHash,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct TrendFactorDefinitionDescriptor {
    schema: &'static str,
    factors: Vec<TrendFactorDescriptor>,
    sources: TrendFactorDefinitionSourcesDescriptor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct TrendFactorDescriptor {
    id: String,
    version: String,
    required_fields: Vec<String>,
    lookback: TrendLookbackDescriptor,
    null_policy: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct TrendLookbackDescriptor {
    kind: &'static str,
    window: usize,
    min_periods: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct TrendFactorDefinitionSourcesDescriptor {
    factor_contract_sha256: String,
    trend_impl_sha256: String,
    trend_support_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrendFactorDefinition {
    descriptor: TrendFactorDefinitionDescriptor,
    factor_versions: BTreeMap<String, String>,
    pub(crate) version: String,
    pub(crate) sha256: ContentHash,
}

#[derive(Clone, Copy)]
struct TrendFactorSourceBytes<'a> {
    factor_contract: &'a [u8],
    trend_implementation: &'a [u8],
    trend_support: &'a [u8],
}

pub(crate) fn compute_signals(
    artifact: &ApprovedHistoricalPriceOnlyArtifact,
    schedule: &SimulationSchedule,
    run_id: Uuid,
    strategy_config_id: Uuid,
    definition: &TrendFactorDefinition,
) -> Result<ComputedSimulationSignals, SimulationSignalError> {
    let resolved = trend_following_config();
    let mut signals = Vec::with_capacity(schedule.signal_execution_pairs().len());
    let mut strategy_snapshot_sha256 = None;

    for (signal_date, _) in schedule.signal_execution_pairs() {
        let input = OwnerBetaPriceRecommendationInput::from_approved_artifact(
            run_id,
            strategy_config_id,
            signal_date,
            artifact,
            &resolved,
        )
        .map_err(|_| SimulationSignalError::ComputationRejected)?;
        let computation = compute_owner_beta_price_recommendation_with_approved(artifact, &input)
            .map_err(|_| SimulationSignalError::ComputationRejected)?;
        let factor_snapshot = computation.factor_snapshot();
        if !definition.matches_version_map(&factor_snapshot.factor_versions) {
            return Err(SimulationSignalError::FactorIdentityUnavailable);
        }

        if strategy_snapshot_sha256
            .as_ref()
            .is_some_and(|expected| expected != input.strategy_snapshot().config_sha256())
        {
            return Err(SimulationSignalError::FactorIdentityUnavailable);
        }
        strategy_snapshot_sha256
            .get_or_insert_with(|| input.strategy_snapshot().config_sha256().clone());

        signals.push(signal_from_computation(signal_date, &computation)?);
    }

    Ok(ComputedSimulationSignals {
        signals,
        strategy_snapshot_sha256: strategy_snapshot_sha256
            .ok_or(SimulationSignalError::ComputationRejected)?,
    })
}

pub(crate) fn derive_trend_factor_definition()
-> Result<TrendFactorDefinition, SimulationSignalError> {
    let requirements = requirements_for(&trend_following_config())
        .map_err(|_| SimulationSignalError::FactorIdentityUnavailable)?;
    let mut sorted_ids = requirements.factor_ids.clone();
    sorted_ids.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    if sorted_ids.len() != 2
        || sorted_ids[0] != SLOW_FACTOR_ID
        || sorted_ids[1] != FAST_FACTOR_ID
        || requirements.minimum_lookback_sessions != 200
    {
        return Err(SimulationSignalError::FactorIdentityUnavailable);
    }
    let factors = factors_for(&requirements.factor_ids)
        .map_err(|_| SimulationSignalError::FactorIdentityUnavailable)?;
    factor_definition_from_factors(
        factors,
        TrendFactorSourceBytes {
            factor_contract: FACTOR_CONTRACT_SOURCE,
            trend_implementation: TREND_IMPLEMENTATION_SOURCE,
            trend_support: TREND_SUPPORT_SOURCE,
        },
    )
}

fn factor_definition_from_factors(
    mut factors: Vec<Box<dyn Factor>>,
    sources: TrendFactorSourceBytes<'_>,
) -> Result<TrendFactorDefinition, SimulationSignalError> {
    factors.sort_by(|left, right| left.id().as_bytes().cmp(right.id().as_bytes()));
    if factors.len() != 2 || factors[0].id() != SLOW_FACTOR_ID || factors[1].id() != FAST_FACTOR_ID
    {
        return Err(SimulationSignalError::FactorIdentityUnavailable);
    }

    let mut factor_versions = BTreeMap::new();
    let mut descriptors = Vec::with_capacity(factors.len());
    let mut common_version: Option<String> = None;
    for factor in factors {
        let expected_window = match factor.id() {
            SLOW_FACTOR_ID => 200,
            FAST_FACTOR_ID => 50,
            _ => return Err(SimulationSignalError::FactorIdentityUnavailable),
        };
        if factor.required_fields() != [Field::CLOSE]
            || factor.null_policy() != NullPolicy::InsufficientLookback
        {
            return Err(SimulationSignalError::FactorIdentityUnavailable);
        }
        let (window, min_periods) = match factor.lookback() {
            Lookback::TradingDays {
                window,
                min_periods,
            } if window == expected_window && min_periods == expected_window => {
                (window, min_periods)
            }
            _ => return Err(SimulationSignalError::FactorIdentityUnavailable),
        };
        let version = factor.version().to_string();
        if common_version
            .as_ref()
            .is_some_and(|expected| expected != &version)
        {
            return Err(SimulationSignalError::FactorIdentityUnavailable);
        }
        common_version.get_or_insert_with(|| version.clone());
        factor_versions.insert(factor.id().to_owned(), version.clone());
        descriptors.push(TrendFactorDescriptor {
            id: factor.id().to_owned(),
            version,
            required_fields: factor
                .required_fields()
                .iter()
                .map(|field| field.as_str().to_owned())
                .collect(),
            lookback: TrendLookbackDescriptor {
                kind: "trading_days",
                window,
                min_periods,
            },
            null_policy: "insufficient_lookback",
        });
    }

    let descriptor = TrendFactorDefinitionDescriptor {
        schema: FACTOR_DEFINITION_SCHEMA,
        factors: descriptors,
        sources: TrendFactorDefinitionSourcesDescriptor {
            factor_contract_sha256: ContentHash::from_bytes(sources.factor_contract)
                .as_str()
                .to_owned(),
            trend_impl_sha256: ContentHash::from_bytes(sources.trend_implementation)
                .as_str()
                .to_owned(),
            trend_support_sha256: ContentHash::from_bytes(sources.trend_support)
                .as_str()
                .to_owned(),
        },
    };
    let canonical = canonical_json(&descriptor)
        .map_err(|_| SimulationSignalError::FactorIdentityUnavailable)?;
    Ok(TrendFactorDefinition {
        descriptor,
        factor_versions,
        version: common_version.ok_or(SimulationSignalError::FactorIdentityUnavailable)?,
        sha256: ContentHash::from_bytes(&canonical),
    })
}

impl TrendFactorDefinition {
    fn matches_version_map(&self, snapshot_versions: &BTreeMap<String, String>) -> bool {
        snapshot_versions == &self.factor_versions
    }
}

fn trend_following_config() -> ResolvedConfig {
    ResolvedConfig {
        strategy_id: STRATEGY_ID.to_owned(),
        strategy_version: STRATEGY_VERSION.to_owned(),
        config: json!({
            "benchmark_instrument": BENCHMARK_INSTRUMENT,
            "fast_ma": 50,
            "slow_ma": 200
        }),
    }
}

fn signal_from_computation(
    signal_date: TradingDate,
    computation: &OwnerBetaComputation,
) -> Result<SignalSnapshot, SimulationSignalError> {
    let factor_snapshot = computation.factor_snapshot();
    let target_snapshot = computation.target_snapshot();
    let mut targets = target_snapshot
        .items()
        .iter()
        .map(|item| {
            let weight = item.target_weight_ppm().unwrap_or(0);
            if weight < 0 {
                return Err(SimulationSignalError::SignalInvalid);
            }
            let weight = u32::try_from(weight).map_err(|_| SimulationSignalError::SignalInvalid)?;
            Ok((item.instrument_id().to_owned(), weight))
        })
        .collect::<Result<Vec<_>, SimulationSignalError>>()?;
    targets.sort_by(|left, right| left.0.cmp(&right.0));
    let cash_weight = u32::try_from(target_snapshot.cash_weight_ppm())
        .map_err(|_| SimulationSignalError::SignalInvalid)?;

    validate_trend_target(&targets, cash_weight)?;
    SignalSnapshot::new(
        &signal_date.to_iso(),
        factor_snapshot.hash.as_str(),
        target_snapshot.target_snapshot_sha256().as_str(),
        targets,
        cash_weight,
    )
    .map_err(|_| SimulationSignalError::SignalInvalid)
}

fn validate_trend_target(
    targets: &[(String, u32)],
    cash_weight_ppm: u32,
) -> Result<(), SimulationSignalError> {
    let mut expected_instruments = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| format!("{symbol}.KRX"))
        .collect::<Vec<_>>();
    expected_instruments.sort();
    if targets.len() != expected_instruments.len()
        || targets
            .iter()
            .map(|(instrument, _)| instrument.as_str())
            .ne(expected_instruments.iter().map(String::as_str))
    {
        return Err(SimulationSignalError::SignalInvalid);
    }
    let benchmark_weight = targets
        .iter()
        .find(|(instrument, _)| instrument == BENCHMARK_INSTRUMENT)
        .map(|(_, weight)| *weight)
        .ok_or(SimulationSignalError::SignalInvalid)?;
    if !matches!(benchmark_weight, 0 | FULL_WEIGHT_PPM)
        || targets
            .iter()
            .any(|(instrument, weight)| instrument != BENCHMARK_INSTRUMENT && *weight != 0)
        || cash_weight_ppm != FULL_WEIGHT_PPM - benchmark_weight
    {
        return Err(SimulationSignalError::SignalInvalid);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use domain::{ContentHash, FactorVersion};
    use factor_engine::contract::{Lookback, NullPolicy};
    use factor_engine::factors::TrendFactor;
    use factor_engine::{Factor, FactorContext, FactorError, FactorFrame, Field};
    use market_data::KR_ETF_CORE_SYMBOLS;

    use super::{
        BENCHMARK_INSTRUMENT, FAST_FACTOR_ID, FULL_WEIGHT_PPM, SLOW_FACTOR_ID,
        SimulationSignalError, TrendFactorSourceBytes, derive_trend_factor_definition,
        factor_definition_from_factors, validate_trend_target,
    };
    use crate::owner_beta::simulation::SignalSnapshot;

    struct VersionedTrendFactor {
        inner: TrendFactor,
        version: FactorVersion,
    }

    impl Factor for VersionedTrendFactor {
        fn id(&self) -> &str {
            self.inner.id()
        }

        fn version(&self) -> FactorVersion {
            self.version.clone()
        }

        fn required_fields(&self) -> &[Field] {
            self.inner.required_fields()
        }

        fn lookback(&self) -> Lookback {
            self.inner.lookback()
        }

        fn null_policy(&self) -> NullPolicy {
            self.inner.null_policy()
        }

        fn compute(&self, context: &FactorContext<'_>) -> Result<FactorFrame, FactorError> {
            self.inner.compute(context)
        }
    }

    fn source_bytes() -> TrendFactorSourceBytes<'static> {
        TrendFactorSourceBytes {
            factor_contract: b"factor contract source",
            trend_implementation: b"trend implementation source",
            trend_support: b"trend support source",
        }
    }

    fn versioned_trend_factors(version: &str) -> Vec<Box<dyn Factor>> {
        [200, 50]
            .into_iter()
            .map(|window| {
                Box::new(VersionedTrendFactor {
                    inner: TrendFactor::new(window).expect("trend factor"),
                    version: FactorVersion::parse(version).expect("factor version"),
                }) as Box<dyn Factor>
            })
            .collect()
    }

    fn mismatched_trend_factor_versions() -> Vec<Box<dyn Factor>> {
        [(200, "1.0.0"), (50, "1.0.1")]
            .into_iter()
            .map(|(window, version)| {
                Box::new(VersionedTrendFactor {
                    inner: TrendFactor::new(window).expect("trend factor"),
                    version: FactorVersion::parse(version).expect("factor version"),
                }) as Box<dyn Factor>
            })
            .collect()
    }

    #[test]
    fn definition_descriptor_matches_the_real_trend_factor_metadata() {
        let definition = derive_trend_factor_definition().expect("derived trend definition");
        assert_eq!(
            definition.descriptor.schema,
            "owner-beta-trend-factor-definition-v1"
        );
        assert_eq!(definition.sha256.as_str().len(), 71);
        assert!(definition.sha256.as_str().starts_with("sha256:"));
        assert_eq!(definition.version, "1.0.0");
        assert_eq!(
            definition
                .descriptor
                .factors
                .iter()
                .map(|factor| factor.id.as_str())
                .collect::<Vec<_>>(),
            vec!["trend_200", "trend_50"]
        );
        let actual_factors = crate::factor_series::factors_for(&[
            SLOW_FACTOR_ID.to_owned(),
            FAST_FACTOR_ID.to_owned(),
        ])
        .expect("existing factors_for implementations");
        for actual in actual_factors {
            let descriptor = definition
                .descriptor
                .factors
                .iter()
                .find(|factor| factor.id.as_str() == actual.id())
                .expect("factor descriptor");
            assert_eq!(descriptor.version, actual.version().to_string());
            let expected_window = if actual.id() == SLOW_FACTOR_ID {
                200
            } else {
                50
            };
            assert_eq!(
                descriptor.required_fields,
                actual
                    .required_fields()
                    .iter()
                    .map(|field| field.as_str().to_owned())
                    .collect::<Vec<_>>()
            );
            assert_eq!(descriptor.required_fields, vec!["close".to_owned()]);
            let (window, min_periods) = match actual.lookback() {
                Lookback::TradingDays {
                    window,
                    min_periods,
                } => (window, min_periods),
                other => panic!("unexpected lookback {other:?}"),
            };
            assert_eq!(window, expected_window);
            assert_eq!(min_periods, expected_window);
            assert_eq!(descriptor.lookback.kind, "trading_days");
            assert_eq!(descriptor.lookback.window, window);
            assert_eq!(descriptor.lookback.min_periods, min_periods);
            assert_eq!(actual.null_policy(), NullPolicy::InsufficientLookback);
            assert_eq!(descriptor.null_policy, "insufficient_lookback");
        }
        assert_eq!(
            definition
                .descriptor
                .sources
                .factor_contract_sha256
                .as_str(),
            ContentHash::from_bytes(super::FACTOR_CONTRACT_SOURCE).as_str()
        );
        assert_eq!(
            definition.descriptor.sources.trend_impl_sha256.as_str(),
            ContentHash::from_bytes(super::TREND_IMPLEMENTATION_SOURCE).as_str()
        );
        assert_eq!(
            definition.descriptor.sources.trend_support_sha256.as_str(),
            ContentHash::from_bytes(super::TREND_SUPPORT_SOURCE).as_str()
        );
        assert!(definition.matches_version_map(&BTreeMap::from([
            (SLOW_FACTOR_ID.to_owned(), "1.0.0".to_owned()),
            (FAST_FACTOR_ID.to_owned(), "1.0.0".to_owned()),
        ])));
    }

    #[test]
    fn definition_digest_is_deterministic_and_binds_metadata_and_source_bytes() {
        let first =
            factor_definition_from_factors(versioned_trend_factors("1.0.0"), source_bytes())
                .expect("first definition");
        let mut reversed_factors = versioned_trend_factors("1.0.0");
        reversed_factors.reverse();
        let repeated = factor_definition_from_factors(reversed_factors, source_bytes())
            .expect("repeated definition");
        let changed_metadata =
            factor_definition_from_factors(versioned_trend_factors("1.0.1"), source_bytes())
                .expect("changed metadata");
        let mut changed_sources = source_bytes();
        changed_sources.trend_implementation = b"changed trend implementation source";
        let changed_source =
            factor_definition_from_factors(versioned_trend_factors("1.0.0"), changed_sources)
                .expect("changed source bytes");

        assert_eq!(first.sha256, repeated.sha256);
        assert_ne!(first.sha256, changed_metadata.sha256);
        assert_ne!(first.sha256, changed_source.sha256);
        assert!(
            factor_definition_from_factors(mismatched_trend_factor_versions(), source_bytes())
                .is_err()
        );
    }

    #[test]
    fn mismatched_or_extra_snapshot_factor_versions_are_rejected() {
        let definition = derive_trend_factor_definition().expect("definition");
        assert!(!definition.matches_version_map(&BTreeMap::from([
            (SLOW_FACTOR_ID.to_owned(), "1.0.1".to_owned()),
            (FAST_FACTOR_ID.to_owned(), "1.0.0".to_owned()),
        ])));
        assert!(!definition.matches_version_map(&BTreeMap::from([
            (SLOW_FACTOR_ID.to_owned(), "1.0.0".to_owned()),
            (FAST_FACTOR_ID.to_owned(), "1.0.0".to_owned()),
            ("trend_100".to_owned(), "1.0.0".to_owned()),
        ])));
    }

    fn cash_targets() -> Vec<(String, u32)> {
        let mut targets = KR_ETF_CORE_SYMBOLS
            .iter()
            .map(|symbol| (format!("{symbol}.KRX"), 0))
            .collect::<Vec<_>>();
        targets.sort_by(|left, right| left.0.cmp(&right.0));
        targets
    }

    #[test]
    fn maps_existing_cash_target_without_recomputing_the_rule() {
        let targets = cash_targets();
        assert_eq!(validate_trend_target(&targets, FULL_WEIGHT_PPM), Ok(()));
        let signal = SignalSnapshot::new(
            "2020-01-02",
            &format!("sha256:{}", "a".repeat(64)),
            &format!("sha256:{}", "b".repeat(64)),
            targets,
            FULL_WEIGHT_PPM,
        )
        .expect("synthetic cash signal");
        assert_eq!(signal.cash_weight_ppm(), FULL_WEIGHT_PPM);
        assert_eq!(
            signal
                .targets()
                .find(|(instrument, _)| *instrument == BENCHMARK_INSTRUMENT),
            Some((BENCHMARK_INSTRUMENT, 0))
        );
    }

    #[test]
    fn unchanged_full_weight_target_maps_deterministically() {
        let mut targets = cash_targets();
        let benchmark = targets
            .iter_mut()
            .find(|(instrument, _)| instrument == BENCHMARK_INSTRUMENT)
            .expect("benchmark");
        benchmark.1 = FULL_WEIGHT_PPM;
        assert_eq!(validate_trend_target(&targets, 0), Ok(()));
        let first = SignalSnapshot::new(
            "2020-01-02",
            &format!("sha256:{}", "a".repeat(64)),
            &format!("sha256:{}", "b".repeat(64)),
            targets.clone(),
            0,
        )
        .expect("first signal");
        let second = SignalSnapshot::new(
            "2020-01-02",
            &format!("sha256:{}", "a".repeat(64)),
            &format!("sha256:{}", "b".repeat(64)),
            targets,
            0,
        )
        .expect("second signal");
        assert_eq!(first, second);
    }

    #[test]
    fn invalid_or_fractional_trend_targets_fail_closed() {
        let mut targets = cash_targets();
        let benchmark = targets
            .iter_mut()
            .find(|(instrument, _)| instrument == BENCHMARK_INSTRUMENT)
            .expect("benchmark");
        benchmark.1 = FULL_WEIGHT_PPM / 2;
        assert_eq!(
            validate_trend_target(&targets, FULL_WEIGHT_PPM / 2),
            Err(SimulationSignalError::SignalInvalid)
        );
    }
}
