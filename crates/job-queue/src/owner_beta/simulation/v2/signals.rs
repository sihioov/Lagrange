//! Existing Rust factor and target calculation adapters for simulation v2.
//!
//! Each factor snapshot is built at its signal date from the retained sealed
//! artifact. Readiness is checked before the existing target builder runs.

use std::collections::{BTreeMap, BTreeSet};

use chrono::NaiveDate;
use domain::{ContentHash, TradingDate};
use factor_engine::{
    Factor, Field, PriceOnlyFactorSnapshot, PriceOnlyFactorSnapshotBuilder,
    contract::{Lookback, NullPolicy},
    normalize::{NormalizePolicy, ZScorePolicy},
    price_only::{PRICE_ONLY_CAPABILITY, PRICE_ONLY_INPUT_KIND},
};
use market_data::ApprovedHistoricalPriceOnlyArtifact;
use serde::Serialize;
use uuid::Uuid;

use crate::{
    factor_series::factors_for,
    owner_beta::{
        OwnerBetaPriceRecommendationInput, OwnerBetaStrategySnapshot,
        target::{OWNER_BETA_TARGET_HASH_ALGORITHM, OWNER_BETA_TARGET_SNAPSHOT_SCHEMA},
    },
    recommendation::compute::requirements_for,
    resolver::ResolvedConfig,
};

use super::{
    V2FactorDefinition, V2Strategy, V2StrategyIdentity, V2TargetAllocation,
    contract::approved_instruments, input::V2PreparedInput,
};

const FACTOR_DEFINITION_SCHEMA: &str = "owner-beta-v2-factor-definition-v1";
const NORMALIZATION_POLICY: &str = "z_score";

const FACTOR_CONTRACT_SOURCE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../factor-engine/src/contract.rs"
));
const FACTOR_BARS_SOURCE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../factor-engine/src/bars.rs"
));
const FACTOR_MONTHS_SOURCE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../factor-engine/src/months.rs"
));
const FACTOR_LAZY_SUPPORT_SOURCE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../factor-engine/src/lazy_util.rs"
));
const FACTOR_PRICE_ONLY_SOURCE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../factor-engine/src/price_only.rs"
));
const FACTOR_NORMALIZATION_SOURCE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../factor-engine/src/normalize.rs"
));
const HISTORICAL_PRICE_SOURCE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../market-data/src/historical_price_only.rs"
));
const FACTOR_SERIES_SOURCE: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/factor_series.rs"));
const REQUIREMENTS_SOURCE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/recommendation/compute.rs"
));
const TARGET_SOURCE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/owner_beta/target.rs"
));
const OWNER_INPUT_SOURCE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/owner_beta/input.rs"
));
const OWNER_COMPUTE_SOURCE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/owner_beta/compute.rs"
));
const BASELINE_SOURCE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../selector/src/baseline.rs"
));
const TREND_SOURCE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../factor-engine/src/factors/trend.rs"
));
const MOMENTUM_SOURCE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../factor-engine/src/factors/momentum.rs"
));
const RETURNS_SOURCE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../factor-engine/src/factors/returns.rs"
));
const VOLATILITY_SOURCE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../factor-engine/src/factors/volatility.rs"
));

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum V2SignalError {
    #[error("v2 strategy configuration is invalid")]
    InvalidStrategy,
    #[error("existing factor requirements are inconsistent")]
    FactorDefinitionInvalid,
    #[error("existing factor computation or identity check failed")]
    FactorSnapshotInvalid,
    #[error("the approved prefix is too short or lacks common factor readiness")]
    InsufficientReadiness,
    #[error("one or more required signals are unavailable")]
    RequiredSignalUnavailable,
    #[error("existing owner-beta target validation failed")]
    TargetInvalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Completeness {
    Complete,
    NotReady,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct FactorDefinitionDescriptor {
    schema: &'static str,
    id: String,
    version: String,
    required_fields: Vec<String>,
    lookback: LookbackDescriptor,
    null_policy: &'static str,
    minimum_close_count: Option<usize>,
    normalization_policy: &'static str,
    sources: Vec<SourceDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum LookbackDescriptor {
    CalendarMonths { months: u32 },
    TradingDays { window: usize, min_periods: usize },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct SourceDescriptor {
    path: &'static str,
    sha256: String,
}

#[derive(Clone, Copy)]
struct SourceBytes {
    path: &'static str,
    bytes: &'static [u8],
}

const COMMON_SOURCES: [SourceBytes; 13] = [
    SourceBytes {
        path: "crates/factor-engine/src/contract.rs",
        bytes: FACTOR_CONTRACT_SOURCE,
    },
    SourceBytes {
        path: "crates/factor-engine/src/bars.rs",
        bytes: FACTOR_BARS_SOURCE,
    },
    SourceBytes {
        path: "crates/factor-engine/src/months.rs",
        bytes: FACTOR_MONTHS_SOURCE,
    },
    SourceBytes {
        path: "crates/factor-engine/src/lazy_util.rs",
        bytes: FACTOR_LAZY_SUPPORT_SOURCE,
    },
    SourceBytes {
        path: "crates/factor-engine/src/price_only.rs",
        bytes: FACTOR_PRICE_ONLY_SOURCE,
    },
    SourceBytes {
        path: "crates/factor-engine/src/normalize.rs",
        bytes: FACTOR_NORMALIZATION_SOURCE,
    },
    SourceBytes {
        path: "crates/market-data/src/historical_price_only.rs",
        bytes: HISTORICAL_PRICE_SOURCE,
    },
    SourceBytes {
        path: "crates/job-queue/src/factor_series.rs",
        bytes: FACTOR_SERIES_SOURCE,
    },
    SourceBytes {
        path: "crates/job-queue/src/recommendation/compute.rs",
        bytes: REQUIREMENTS_SOURCE,
    },
    SourceBytes {
        path: "crates/job-queue/src/owner_beta/target.rs",
        bytes: TARGET_SOURCE,
    },
    SourceBytes {
        path: "crates/job-queue/src/owner_beta/input.rs",
        bytes: OWNER_INPUT_SOURCE,
    },
    SourceBytes {
        path: "crates/job-queue/src/owner_beta/compute.rs",
        bytes: OWNER_COMPUTE_SOURCE,
    },
    SourceBytes {
        path: "crates/selector/src/baseline.rs",
        bytes: BASELINE_SOURCE,
    },
];

/// Resolves only one of the five exact canonical DEFAULT configurations.
pub(crate) fn strategy_for_config(resolved: &ResolvedConfig) -> Result<V2Strategy, V2SignalError> {
    let strategy = V2Strategy::ALL
        .into_iter()
        .find(|strategy| strategy.as_str() == resolved.strategy_id)
        .ok_or(V2SignalError::InvalidStrategy)?;
    let canonical =
        serde_json::to_string(&resolved.config).map_err(|_| V2SignalError::InvalidStrategy)?;
    if resolved.strategy_version != "1.0.0" || canonical != strategy.default_config_json() {
        return Err(V2SignalError::InvalidStrategy);
    }
    OwnerBetaStrategySnapshot::from_resolved_config(resolved)
        .map_err(|_| V2SignalError::InvalidStrategy)?;
    Ok(strategy)
}

pub(crate) fn default_config(strategy: V2Strategy) -> Result<ResolvedConfig, V2SignalError> {
    let config = serde_json::from_str(strategy.default_config_json())
        .map_err(|_| V2SignalError::InvalidStrategy)?;
    Ok(ResolvedConfig {
        strategy_id: strategy.as_str().to_owned(),
        strategy_version: "1.0.0".to_owned(),
        config,
    })
}

pub(crate) fn strategy_identity(
    resolved: &ResolvedConfig,
    strategy: V2Strategy,
) -> Result<V2StrategyIdentity, V2SignalError> {
    if strategy_for_config(resolved)? != strategy {
        return Err(V2SignalError::InvalidStrategy);
    }
    let strategy_snapshot = OwnerBetaStrategySnapshot::from_resolved_config(resolved)
        .map_err(|_| V2SignalError::InvalidStrategy)?;
    let factors = factor_definitions(resolved, strategy)?;
    V2StrategyIdentity::from_default_snapshot(
        strategy.as_str(),
        &resolved.strategy_version,
        &serde_json::to_string(&resolved.config).map_err(|_| V2SignalError::InvalidStrategy)?,
        strategy_snapshot.config_sha256().as_str(),
        factors,
    )
    .map_err(|_| V2SignalError::FactorDefinitionInvalid)
}

fn factor_definitions(
    resolved: &ResolvedConfig,
    strategy: V2Strategy,
) -> Result<Vec<V2FactorDefinition>, V2SignalError> {
    let requirements =
        requirements_for(resolved).map_err(|_| V2SignalError::FactorDefinitionInvalid)?;
    let expected_names = strategy
        .factor_names()
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();
    if requirements.factor_ids != expected_names {
        return Err(V2SignalError::FactorDefinitionInvalid);
    }

    let factors = factors_for(&requirements.factor_ids)
        .map_err(|_| V2SignalError::FactorDefinitionInvalid)?;
    if factors.len() != expected_names.len() {
        return Err(V2SignalError::FactorDefinitionInvalid);
    }
    factors
        .iter()
        .zip(expected_names.iter())
        .map(|(factor, expected_name)| {
            let descriptor = describe_factor(factor.as_ref())?;
            if factor.id() != expected_name {
                return Err(V2SignalError::FactorDefinitionInvalid);
            }
            let bytes = serde_json::to_vec(&descriptor)
                .map_err(|_| V2SignalError::FactorDefinitionInvalid)?;
            let sha = ContentHash::from_bytes(&bytes);
            V2FactorDefinition::new(factor.id(), &factor.version().to_string(), sha.as_str())
                .map_err(|_| V2SignalError::FactorDefinitionInvalid)
        })
        .collect()
}

fn describe_factor(factor: &dyn Factor) -> Result<FactorDefinitionDescriptor, V2SignalError> {
    if factor.required_fields() != [Field::CLOSE]
        || factor.null_policy() != NullPolicy::InsufficientLookback
        || factor.version().to_string() != "1.0.0"
    {
        return Err(V2SignalError::FactorDefinitionInvalid);
    }
    let (lookback, minimum_close_count) = match (factor.id(), factor.lookback()) {
        (
            "trend_50",
            Lookback::TradingDays {
                window: 50,
                min_periods: 50,
            },
        ) => (
            LookbackDescriptor::TradingDays {
                window: 50,
                min_periods: 50,
            },
            Some(50),
        ),
        (
            "trend_200",
            Lookback::TradingDays {
                window: 200,
                min_periods: 200,
            },
        ) => (
            LookbackDescriptor::TradingDays {
                window: 200,
                min_periods: 200,
            },
            Some(200),
        ),
        (
            "vol_60",
            Lookback::TradingDays {
                window: 60,
                min_periods: 60,
            },
        ) => (
            LookbackDescriptor::TradingDays {
                window: 60,
                min_periods: 60,
            },
            Some(61),
        ),
        ("return_12m", Lookback::CalendarMonths(12))
        | ("momentum_12_1", Lookback::CalendarMonths(12)) => {
            (LookbackDescriptor::CalendarMonths { months: 12 }, None)
        }
        _ => return Err(V2SignalError::FactorDefinitionInvalid),
    };
    let implementation = match factor.id() {
        "trend_50" | "trend_200" => SourceBytes {
            path: "crates/factor-engine/src/factors/trend.rs",
            bytes: TREND_SOURCE,
        },
        "momentum_12_1" => SourceBytes {
            path: "crates/factor-engine/src/factors/momentum.rs",
            bytes: MOMENTUM_SOURCE,
        },
        "return_12m" => SourceBytes {
            path: "crates/factor-engine/src/factors/returns.rs",
            bytes: RETURNS_SOURCE,
        },
        "vol_60" => SourceBytes {
            path: "crates/factor-engine/src/factors/volatility.rs",
            bytes: VOLATILITY_SOURCE,
        },
        _ => return Err(V2SignalError::FactorDefinitionInvalid),
    };
    let mut sources = COMMON_SOURCES
        .iter()
        .copied()
        .chain(std::iter::once(implementation))
        .map(source_descriptor)
        .collect::<Vec<_>>();
    sources.sort_by(|left, right| left.path.cmp(right.path));
    Ok(FactorDefinitionDescriptor {
        schema: FACTOR_DEFINITION_SCHEMA,
        id: factor.id().to_owned(),
        version: factor.version().to_string(),
        required_fields: factor
            .required_fields()
            .iter()
            .map(|field| field.as_str().to_owned())
            .collect(),
        lookback,
        null_policy: "insufficient_lookback",
        minimum_close_count,
        normalization_policy: NORMALIZATION_POLICY,
        sources,
    })
}

fn source_descriptor(source: SourceBytes) -> SourceDescriptor {
    SourceDescriptor {
        path: source.path,
        sha256: ContentHash::from_bytes(source.bytes).as_str().to_owned(),
    }
}

/// Finds the first signal date where every fixed strategy has all eleven
/// required values and a valid target. Only typed NULL readiness can advance
/// the search; all other failures stop it.
pub(crate) fn first_common_ready_start(
    prepared: &V2PreparedInput,
    run_id: Uuid,
    strategy_config_id: Uuid,
) -> Result<usize, V2SignalError> {
    let sessions = prepared.sessions();
    let first_candidate = minimum_common_prefix()?;
    if sessions.len() < 3 || sessions.len() - 2 <= first_candidate {
        return Err(V2SignalError::InsufficientReadiness);
    }
    let candidates = &sessions[..sessions.len() - 2];
    let first_ready = first_ready_index_with(candidates, first_candidate, |signal_date| {
        if !calendar_references_available(sessions, signal_date)? {
            return Ok(None);
        }
        let mut targets = Vec::with_capacity(V2Strategy::ALL.len());
        for strategy in V2Strategy::ALL {
            let resolved = default_config(strategy)?;
            let Some(target) = compute_target_at(
                &prepared.approved,
                run_id,
                strategy_config_id,
                &resolved,
                strategy,
                signal_date,
            )?
            else {
                return Ok(None);
            };
            targets.push(target);
        }
        Ok(Some(targets))
    })?;
    first_ready
        .map(|(index, _)| index + 1)
        .ok_or(V2SignalError::InsufficientReadiness)
}

/// Returns no target only when one or more required values are not ready.
/// Broken definitions, identities, snapshots, and targets remain errors.
pub(crate) fn compute_target_at(
    artifact: &ApprovedHistoricalPriceOnlyArtifact,
    run_id: Uuid,
    strategy_config_id: Uuid,
    resolved: &ResolvedConfig,
    strategy: V2Strategy,
    as_of: TradingDate,
) -> Result<Option<V2TargetAllocation>, V2SignalError> {
    if strategy_for_config(resolved)? != strategy {
        return Err(V2SignalError::InvalidStrategy);
    }
    let input = OwnerBetaPriceRecommendationInput::from_approved_artifact(
        run_id,
        strategy_config_id,
        as_of,
        artifact,
        resolved,
    )
    .map_err(|_| V2SignalError::FactorSnapshotInvalid)?;
    input
        .validate_strategy_snapshot()
        .map_err(|_| V2SignalError::FactorSnapshotInvalid)?;
    input
        .validate_approved_artifact(artifact)
        .map_err(|_| V2SignalError::FactorSnapshotInvalid)?;

    let requirements =
        requirements_for(resolved).map_err(|_| V2SignalError::FactorDefinitionInvalid)?;
    let factors = factors_for(&requirements.factor_ids)
        .map_err(|_| V2SignalError::FactorDefinitionInvalid)?;
    if requirements.factor_ids
        != strategy
            .factor_names()
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>()
        || factors
            .iter()
            .any(|factor| factor.required_fields() != [Field::CLOSE])
    {
        return Err(V2SignalError::FactorDefinitionInvalid);
    }

    let snapshot = PriceOnlyFactorSnapshotBuilder::new(artifact, as_of)
        .with_factors(factors)
        .map_err(|_| V2SignalError::FactorDefinitionInvalid)?
        .build()
        .map_err(|_| V2SignalError::FactorSnapshotInvalid)?;
    if !validate_snapshot_identity(&input, &snapshot, &requirements.factor_ids)? {
        return Ok(None);
    }

    let target = crate::owner_beta::build_target_snapshot(&input, &snapshot)
        .map_err(|_| V2SignalError::TargetInvalid)?;
    validate_target_identity(&input, &snapshot, &target, strategy, as_of)?;
    target_allocation(&target).map(Some)
}

fn validate_snapshot_identity(
    input: &OwnerBetaPriceRecommendationInput,
    snapshot: &PriceOnlyFactorSnapshot,
    factor_ids: &[String],
) -> Result<bool, V2SignalError> {
    input
        .validate_factor_snapshot(snapshot)
        .map_err(|_| V2SignalError::FactorSnapshotInvalid)?;
    let computed_hash = snapshot
        .compute_hash()
        .map_err(|_| V2SignalError::FactorSnapshotInvalid)?;
    if computed_hash != snapshot.hash {
        return Err(V2SignalError::FactorSnapshotInvalid);
    }

    let factors = factors_for(factor_ids).map_err(|_| V2SignalError::FactorDefinitionInvalid)?;
    let expected_versions = factors
        .iter()
        .map(|factor| (factor.id().to_owned(), factor.version().to_string()))
        .collect::<BTreeMap<_, _>>();
    if expected_versions != snapshot.factor_versions {
        return Err(V2SignalError::FactorDefinitionInvalid);
    }
    let normalizer = ZScorePolicy::default();
    if snapshot.normalization.id != normalizer.id()
        || snapshot.normalization.version != normalizer.version().to_string()
        || snapshot.normalization.params != normalizer.params()
    {
        return Err(V2SignalError::FactorDefinitionInvalid);
    }

    let expected_instruments = approved_instruments();
    let at_as_of = factor_values_at_as_of(
        snapshot.as_of,
        &snapshot.rows,
        factor_ids,
        &expected_instruments,
    )?;

    if factor_ids.is_empty() {
        return if at_as_of.is_empty() {
            Ok(true)
        } else {
            Err(V2SignalError::FactorSnapshotInvalid)
        };
    }
    if at_as_of.len() != factor_ids.len() * expected_instruments.len() {
        return Err(V2SignalError::FactorSnapshotInvalid);
    }

    Ok(
        required_values_at_as_of(&expected_instruments, factor_ids, &at_as_of)?
            == Completeness::Complete,
    )
}

fn factor_values_at_as_of(
    as_of: TradingDate,
    rows: &[factor_engine::snapshot::FactorRow],
    factor_ids: &[String],
    instruments: &[String],
) -> Result<BTreeMap<(String, String), Option<f64>>, V2SignalError> {
    let expected_factor_set = factor_ids.iter().cloned().collect::<BTreeSet<_>>();
    let expected_instrument_set = instruments.iter().cloned().collect::<BTreeSet<_>>();
    let as_of_text = as_of.to_iso();
    let mut observed = BTreeSet::<(String, String, String)>::new();
    let mut at_as_of = BTreeMap::<(String, String), Option<f64>>::new();
    for row in rows {
        let row_date =
            TradingDate::parse(&row.date).map_err(|_| V2SignalError::FactorSnapshotInvalid)?;
        if row_date > as_of
            || !expected_instrument_set.contains(&row.instrument)
            || !expected_factor_set.contains(&row.factor)
            || !observed.insert((row.date.clone(), row.instrument.clone(), row.factor.clone()))
        {
            return Err(V2SignalError::FactorSnapshotInvalid);
        }
        if row.date == as_of_text
            && at_as_of
                .insert((row.factor.clone(), row.instrument.clone()), row.raw)
                .is_some()
        {
            return Err(V2SignalError::FactorSnapshotInvalid);
        }
    }
    if !factor_ids.is_empty() && at_as_of.len() != factor_ids.len() * instruments.len() {
        return Err(V2SignalError::FactorSnapshotInvalid);
    }
    Ok(at_as_of)
}

fn validate_target_identity(
    input: &OwnerBetaPriceRecommendationInput,
    snapshot: &PriceOnlyFactorSnapshot,
    target: &crate::owner_beta::OwnerBetaTargetSnapshot,
    strategy: V2Strategy,
    as_of: TradingDate,
) -> Result<(), V2SignalError> {
    let strategy_snapshot = input.strategy_snapshot();
    if target.schema() != OWNER_BETA_TARGET_SNAPSHOT_SCHEMA
        || target.hash_algorithm() != OWNER_BETA_TARGET_HASH_ALGORITHM
        || target.input_kind() != PRICE_ONLY_INPUT_KIND
        || target.capability() != PRICE_ONLY_CAPABILITY
        || target.as_of() != as_of
        || target.strategy_id() != strategy.as_str()
        || target.strategy_version() != "1.0.0"
        || target.strategy_config_sha256() != strategy_snapshot.config_sha256()
        || target.factor_snapshot_sha256() != &snapshot.hash
        || target.pins() != input.pins()
        || target.validate_hash().is_err()
    {
        return Err(V2SignalError::TargetInvalid);
    }
    Ok(())
}

fn target_allocation(
    target: &crate::owner_beta::OwnerBetaTargetSnapshot,
) -> Result<V2TargetAllocation, V2SignalError> {
    let expected = approved_instruments();
    if target.items().len() != expected.len()
        || !(0..=1_000_000).contains(&target.cash_weight_ppm())
    {
        return Err(V2SignalError::TargetInvalid);
    }
    let mut weights = BTreeMap::new();
    for item in target.items() {
        let weight = item.target_weight_ppm().unwrap_or(0);
        if !(0..=1_000_000).contains(&weight)
            || weights
                .insert(item.instrument_id().to_owned(), weight as u32)
                .is_some()
        {
            return Err(V2SignalError::TargetInvalid);
        }
    }
    if weights.len() != expected.len()
        || weights
            .keys()
            .map(String::as_str)
            .ne(expected.iter().map(String::as_str))
    {
        return Err(V2SignalError::TargetInvalid);
    }
    let ordered = expected
        .iter()
        .map(|instrument| {
            weights
                .get(instrument)
                .map(|weight| (instrument.clone(), *weight))
                .ok_or(V2SignalError::TargetInvalid)
        })
        .collect::<Result<Vec<_>, _>>()?;
    V2TargetAllocation::new(
        target.target_snapshot_sha256().as_str(),
        ordered,
        target.cash_weight_ppm() as u32,
    )
    .map_err(|_| V2SignalError::TargetInvalid)
}

fn minimum_common_prefix() -> Result<usize, V2SignalError> {
    let mut minimum = 1_usize;
    for strategy in V2Strategy::ALL {
        let resolved = default_config(strategy)?;
        let requirements =
            requirements_for(&resolved).map_err(|_| V2SignalError::FactorDefinitionInvalid)?;
        let factors = factors_for(&requirements.factor_ids)
            .map_err(|_| V2SignalError::FactorDefinitionInvalid)?;
        for factor in factors {
            let needed = match factor.lookback() {
                Lookback::TradingDays {
                    window,
                    min_periods,
                } => {
                    if factor.id() == "vol_60" {
                        window.checked_add(1)
                    } else {
                        Some(min_periods)
                    }
                }
                Lookback::FixedWindow { min_periods, .. } => Some(min_periods),
                Lookback::CalendarMonths(_) | Lookback::FullHistory => Some(1),
            }
            .ok_or(V2SignalError::FactorDefinitionInvalid)?;
            minimum = minimum.max(needed);
        }
    }
    Ok(minimum.saturating_sub(1))
}

fn calendar_references_available(
    sessions: &[TradingDate],
    as_of: TradingDate,
) -> Result<bool, V2SignalError> {
    let as_of_date = naive_date(as_of)?;
    for months in [1_u32, 12_u32] {
        let target = factor_engine::months::month_end(factor_engine::months::months_back(
            as_of_date, months,
        ));
        let mut found = false;
        for date in sessions.iter().copied().take_while(|date| *date <= as_of) {
            if naive_date(date)? <= target {
                found = true;
            }
        }
        if !found {
            return Ok(false);
        }
    }
    Ok(true)
}

fn naive_date(date: TradingDate) -> Result<NaiveDate, V2SignalError> {
    NaiveDate::parse_from_str(&date.to_iso(), "%Y-%m-%d")
        .map_err(|_| V2SignalError::FactorDefinitionInvalid)
}

fn first_ready_index_with<T, E>(
    sessions: &[TradingDate],
    first_candidate: usize,
    mut evaluate: impl FnMut(TradingDate) -> Result<Option<T>, E>,
) -> Result<Option<(usize, T)>, E> {
    for (index, date) in sessions.iter().copied().enumerate().skip(first_candidate) {
        if let Some(value) = evaluate(date)? {
            return Ok(Some((index, value)));
        }
    }
    Ok(None)
}

fn required_values_at_as_of(
    instruments: &[String],
    factor_ids: &[String],
    values: &BTreeMap<(String, String), Option<f64>>,
) -> Result<Completeness, V2SignalError> {
    if values.len() != instruments.len() * factor_ids.len() {
        return Err(V2SignalError::FactorSnapshotInvalid);
    }
    let mut complete = true;
    for factor in factor_ids {
        for instrument in instruments {
            match values.get(&(factor.clone(), instrument.clone())) {
                Some(Some(value)) if value.is_finite() => {
                    if factor == "vol_60" && *value <= 0.0 {
                        complete = false;
                    }
                }
                Some(Some(_)) | Some(None) => complete = false,
                None => return Err(V2SignalError::FactorSnapshotInvalid),
            }
        }
    }
    Ok(if complete {
        Completeness::Complete
    } else {
        Completeness::NotReady
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::{Datelike, Duration, NaiveDate, Weekday};
    use domain::TradingDate;
    use factor_engine::contract::{Lookback, NullPolicy};
    use factor_engine::snapshot::FactorRow;

    use crate::owner_beta::{
        OwnerBetaStrategySnapshot,
        simulation::v2::{V2Strategy, contract::approved_instruments},
    };

    use super::{
        Completeness, V2SignalError, calendar_references_available, default_config,
        describe_factor, factor_definitions, factor_values_at_as_of, first_ready_index_with,
        required_values_at_as_of, strategy_for_config,
    };

    fn weekday_dates(start: NaiveDate, count: usize) -> Vec<TradingDate> {
        let mut date = start;
        let mut out = Vec::with_capacity(count);
        while out.len() < count {
            if !matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
                out.push(TradingDate::new(date.year(), date.month(), date.day()).expect("date"));
            }
            date += Duration::days(1);
        }
        out
    }

    #[test]
    fn factor_definitions_follow_wp1_order_and_commit_real_sources() {
        let resolved = default_config(V2Strategy::TrendFollowing).expect("default");
        let definitions = factor_definitions(&resolved, V2Strategy::TrendFollowing).expect("defs");
        assert_eq!(
            definitions
                .iter()
                .map(|item| item.name())
                .collect::<Vec<_>>(),
            ["trend_50", "trend_200"]
        );
        assert!(
            definitions
                .iter()
                .all(|item| item.sha256().as_str().starts_with("sha256:"))
        );

        let relative = default_config(V2Strategy::RelativeMomentum).expect("default");
        let definitions =
            factor_definitions(&relative, V2Strategy::RelativeMomentum).expect("defs");
        assert_eq!(definitions[0].name(), "momentum_12_1");
        assert_ne!(
            definitions[0].sha256().as_str(),
            "sha256:0000000000000000000000000000000000000000000000000000000000000000"
        );
    }

    #[test]
    fn custom_or_noncanonical_default_configuration_is_rejected() {
        let mut resolved = default_config(V2Strategy::TrendFollowing).expect("default");
        resolved.config["slow_ma"] = serde_json::json!(201);
        assert_eq!(
            strategy_for_config(&resolved),
            Err(V2SignalError::InvalidStrategy)
        );
    }

    #[test]
    fn readiness_search_returns_first_ready_date_and_does_not_skip_errors() {
        let sessions = weekday_dates(NaiveDate::from_ymd_opt(2020, 1, 1).expect("date"), 5);
        let ready = first_ready_index_with(&sessions, 0, |date| {
            if date == sessions[0] || date == sessions[1] {
                Ok::<_, V2SignalError>(None)
            } else {
                Ok(Some(date))
            }
        })
        .expect("search");
        assert_eq!(ready, Some((2, sessions[2])));

        let error = first_ready_index_with(&sessions, 0, |date| {
            if date == sessions[0] {
                Ok(None::<NaiveDate>)
            } else {
                Err(V2SignalError::TargetInvalid)
            }
        });
        assert_eq!(error, Err(V2SignalError::TargetInvalid));
    }

    #[test]
    fn month_reference_uses_month_end_with_leap_and_midmonth_dates() {
        let sessions = weekday_dates(NaiveDate::from_ymd_opt(2019, 2, 1).expect("date"), 430);
        let leap_month_midmonth = TradingDate::new(2020, 3, 16).expect("weekday");
        assert!(calendar_references_available(&sessions, leap_month_midmonth).expect("refs"));
        let too_early = TradingDate::new(2019, 2, 1).expect("date");
        assert!(!calendar_references_available(&sessions, too_early).expect("refs"));
    }

    #[test]
    fn all_eleven_required_values_must_be_finite_and_positive_for_volatility() {
        let instruments = approved_instruments();
        let rows = instruments
            .iter()
            .map(|instrument| (("vol_60".to_owned(), instrument.clone()), Some(0.1)))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            required_values_at_as_of(&instruments, &["vol_60".to_owned()], &rows),
            Ok(Completeness::Complete)
        );

        let mut null_value = rows.clone();
        null_value.insert(("vol_60".to_owned(), instruments[3].clone()), None);
        assert_eq!(
            required_values_at_as_of(&instruments, &["vol_60".to_owned()], &null_value),
            Ok(Completeness::NotReady)
        );

        let mut zero_vol = rows.clone();
        zero_vol.insert(("vol_60".to_owned(), instruments[4].clone()), Some(0.0));
        assert_eq!(
            required_values_at_as_of(&instruments, &["vol_60".to_owned()], &zero_vol),
            Ok(Completeness::NotReady)
        );

        let mut nonfinite = rows.clone();
        nonfinite.insert(
            ("vol_60".to_owned(), instruments[5].clone()),
            Some(f64::NAN),
        );
        assert_eq!(
            required_values_at_as_of(&instruments, &["vol_60".to_owned()], &nonfinite),
            Ok(Completeness::NotReady)
        );

        let mut missing = rows;
        missing.remove(&("vol_60".to_owned(), instruments[0].clone()));
        assert_eq!(
            required_values_at_as_of(&instruments, &["vol_60".to_owned()], &missing),
            Err(V2SignalError::FactorSnapshotInvalid)
        );
    }

    #[test]
    fn factor_inventory_rejects_rows_after_the_signal_date() {
        let as_of = TradingDate::new(2024, 5, 31).expect("signal date");
        let future = TradingDate::new(2024, 6, 3).expect("future date");
        let instruments = approved_instruments();
        let factor_ids = vec!["vol_60".to_owned()];
        let mut rows = instruments
            .iter()
            .map(|instrument| FactorRow {
                date: as_of.to_iso(),
                instrument: instrument.clone(),
                factor: "vol_60".to_owned(),
                raw: Some(0.1),
                normalized: Some(0.0),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            factor_values_at_as_of(as_of, &rows, &factor_ids, &instruments)
                .expect("complete as-of values")
                .len(),
            instruments.len()
        );

        rows.push(FactorRow {
            date: future.to_iso(),
            instrument: instruments[0].clone(),
            factor: "vol_60".to_owned(),
            raw: Some(100.0),
            normalized: Some(1.0),
        });
        assert_eq!(
            factor_values_at_as_of(as_of, &rows, &factor_ids, &instruments),
            Err(V2SignalError::FactorSnapshotInvalid)
        );
    }

    #[test]
    fn prefix_history_boundaries_are_derived_from_real_lookbacks() {
        let factors =
            crate::factor_series::factors_for(&["trend_200".to_owned(), "vol_60".to_owned()])
                .expect("factors");
        let trend = factors
            .iter()
            .find(|factor| factor.id() == "trend_200")
            .expect("trend");
        let volatility = factors
            .iter()
            .find(|factor| factor.id() == "vol_60")
            .expect("vol");
        assert_eq!(
            describe_factor(trend.as_ref())
                .expect("trend")
                .minimum_close_count,
            Some(200)
        );
        assert_eq!(
            describe_factor(volatility.as_ref())
                .expect("vol")
                .minimum_close_count,
            Some(61)
        );
        assert_eq!(trend.null_policy(), NullPolicy::InsufficientLookback);
        assert!(matches!(
            trend.lookback(),
            Lookback::TradingDays {
                window: 200,
                min_periods: 200
            }
        ));
    }

    fn target_parts(
        strategy: V2Strategy,
        value_for: impl Fn(&str, &str) -> f64,
    ) -> crate::owner_beta::target::ComputedTargetParts {
        let resolved = default_config(strategy).expect("default config");
        let snapshot =
            OwnerBetaStrategySnapshot::from_resolved_config(&resolved).expect("strategy snapshot");
        let date = TradingDate::new(2024, 5, 31).expect("date");
        let mut versions = BTreeMap::new();
        let mut rows = Vec::new();
        for factor in strategy.factor_names() {
            versions.insert((*factor).to_owned(), "1.0.0".to_owned());
            for instrument in approved_instruments() {
                let value = value_for(factor, &instrument);
                rows.push(FactorRow {
                    date: date.to_iso(),
                    instrument: instrument.clone(),
                    factor: (*factor).to_owned(),
                    raw: Some(value),
                    normalized: Some(value),
                });
            }
        }
        crate::owner_beta::target::compute_target_parts(&snapshot, date, &versions, &rows)
            .expect("existing Rust target helper")
    }

    #[test]
    fn existing_rust_targets_cover_five_strategy_rules_and_ties() {
        let buy_and_hold = target_parts(V2Strategy::BuyAndHold, |_, _| 0.0);
        assert_eq!(buy_and_hold.cash_weight_ppm, 0);
        assert_eq!(
            buy_and_hold
                .items
                .iter()
                .find(|item| item.instrument_id() == "069500.KRX")
                .and_then(|item| item.target_weight_ppm()),
            Some(1_000_000)
        );

        let trend = target_parts(V2Strategy::TrendFollowing, |factor, _| match factor {
            "trend_50" => -0.1,
            "trend_200" => 0.1,
            _ => 0.0,
        });
        assert_eq!(trend.cash_weight_ppm, 0);
        let trend_tie = target_parts(V2Strategy::TrendFollowing, |_, _| 0.0);
        assert_eq!(trend_tie.cash_weight_ppm, 1_000_000);

        let mut expected = approved_instruments();
        expected.sort();
        let relative = target_parts(V2Strategy::RelativeMomentum, |_, _| -0.2);
        let selected = relative
            .items
            .iter()
            .filter(|item| item.target_weight_ppm().unwrap_or(0) > 0)
            .map(|item| item.instrument_id().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(selected, expected[..3]);
        assert_eq!(relative.cash_weight_ppm, 100);

        let dual = target_parts(V2Strategy::DualMomentum, |_, _| 0.1);
        let selected = dual
            .items
            .iter()
            .filter(|item| item.target_weight_ppm().unwrap_or(0) > 0)
            .map(|item| item.instrument_id().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(selected, vec!["229200.KRX".to_owned()]);
        let dual_cash = target_parts(V2Strategy::DualMomentum, |_, _| 0.0);
        assert_eq!(dual_cash.cash_weight_ppm, 1_000_000);

        let inverse = target_parts(V2Strategy::InverseVolatility, |_, instrument| {
            if instrument == expected.first().expect("ETF11") {
                0.01
            } else {
                1.0
            }
        });
        assert!(
            inverse
                .items
                .iter()
                .filter_map(|item| item.target_weight_ppm())
                .all(|weight| weight <= 300_000)
        );
        assert_eq!(
            inverse
                .items
                .iter()
                .find(|item| item.instrument_id() == expected.first().expect("ETF11"))
                .and_then(|item| item.target_weight_ppm()),
            Some(300_000)
        );
        assert_eq!(
            inverse
                .items
                .iter()
                .filter_map(|item| item.target_weight_ppm())
                .sum::<i64>()
                + inverse.cash_weight_ppm,
            1_000_000
        );
        assert!(inverse.cash_weight_ppm > 0);
    }
}
