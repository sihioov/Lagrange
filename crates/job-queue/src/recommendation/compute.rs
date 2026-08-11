//! Deterministic factor requirements and close computation for recommendations.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::factor_series::{dataset_shape, factors_for};
use crate::recommendation::input::AttestedDataset;
use crate::resolver::ResolvedConfig;
use domain::TradingDate;
use factor_engine::bars::Bars;
use factor_engine::{FactorSnapshotBuilder, FrozenUniverse};
use market_data::curate::CurateStore;
use selector::baseline::baseline_packages;
use selector::registry::{Actor, Registry};
use selector::universe::parse_manifest;
use thiserror::Error;

/// The exact factor inputs and history a validated strategy configuration needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrategyRequirements {
    pub factor_ids: Vec<String>,
    pub minimum_lookback_sessions: u64,
}

/// The single immutable universe accepted by the first recommendation release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttestedUniverse {
    pub universe_id: String,
    pub snapshot_id: String,
    pub members: Vec<String>,
}

impl AttestedUniverse {
    /// Parse a manifest and require byte-independent canonical equality with
    /// the repository's shipped fixed-universe definition.
    pub fn from_manifest_yaml(yaml: &str) -> Result<Self, RecommendationError> {
        let manifest =
            parse_manifest(yaml).map_err(|error| RecommendationError::InvalidUniverse {
                detail: error.to_string(),
            })?;
        let shipped = parse_manifest(include_str!(
            "../../../../configs/universes/kr-etf-core-v1.yaml"
        ))
        .expect("the embedded fixed-universe manifest is valid");
        let snapshot =
            manifest
                .canonical_hash()
                .map_err(|error| RecommendationError::InvalidUniverse {
                    detail: error.to_string(),
                })?;
        let shipped_snapshot = shipped
            .canonical_hash()
            .expect("the embedded fixed-universe manifest is hashable");
        if snapshot != shipped_snapshot {
            return Err(RecommendationError::InvalidUniverse {
                detail: "manifest is not the shipped kr-etf-core-v1 snapshot".to_owned(),
            });
        }
        let members = manifest
            .universe
            .instruments
            .iter()
            .map(|entry| entry.id.to_string())
            .collect::<Vec<_>>();
        if members.len() != 11 {
            return Err(RecommendationError::InvalidUniverse {
                detail: format!(
                    "fixed universe must contain 11 members, got {}",
                    members.len()
                ),
            });
        }
        Ok(Self {
            universe_id: manifest.universe.id,
            snapshot_id: snapshot.as_str().to_owned(),
            members,
        })
    }
}

/// Raw factor values at exactly one requested close.
#[derive(Debug, Clone, PartialEq)]
pub struct ComputedClose {
    pub as_of: TradingDate,
    pub factors: BTreeMap<String, BTreeMap<String, f64>>,
    pub factor_snapshot_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RecommendationError {
    #[error("invalid recommendation strategy input: {detail}")]
    InvalidStrategy { detail: String },
    #[error("invalid recommendation universe: {detail}")]
    InvalidUniverse { detail: String },
    #[error("recommendation dataset rejected: {detail}")]
    Dataset { detail: String },
    #[error("recommendation factor computation failed: {detail}")]
    Compute { detail: String },
    #[error("recommendation blocking task failed: {detail}")]
    BlockingTask { detail: String },
}

/// Compute only the required factors and expose only finite raw values at the
/// requested close. Paths and the curated version come exclusively from the
/// database-attested dataset pin.
pub fn compute_close(
    pin: &AttestedDataset,
    universe: &AttestedUniverse,
    as_of: TradingDate,
    requirements: &StrategyRequirements,
) -> Result<ComputedClose, RecommendationError> {
    validate_fixed_universe(universe)?;
    let dataset_root = Path::new(&pin.storage_path);
    let shape = dataset_shape(dataset_root).map_err(|error| RecommendationError::Dataset {
        detail: error.to_string(),
    })?;
    let actual = shape.instruments.iter().cloned().collect::<BTreeSet<_>>();
    let expected = universe.members.iter().cloned().collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(RecommendationError::Dataset {
            detail: format!(
                "dataset universe mismatch: expected {} exact members, found {}",
                expected.len(),
                actual.len()
            ),
        });
    }

    let store = CurateStore::new(dataset_root);
    let member_refs = universe
        .members
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let frozen = FrozenUniverse::new(&universe.snapshot_id, &member_refs);
    let bars = Bars::from_curated(
        &store,
        "kr",
        &pin.dataset_id,
        pin.curated_version,
        &frozen,
        as_of,
    )
    .map_err(|error| RecommendationError::Dataset {
        detail: error.to_string(),
    })?;
    for instrument in frozen.instruments() {
        let has_close = bars
            .points(instrument)
            .is_some_and(|points| points.iter().any(|point| point.date == as_of));
        if !has_close {
            return Err(RecommendationError::Dataset {
                detail: format!("{} has no requested close on {as_of}", instrument),
            });
        }
    }

    let unique_factor_ids = requirements.factor_ids.iter().collect::<BTreeSet<_>>();
    if unique_factor_ids.len() != requirements.factor_ids.len() {
        return Err(RecommendationError::InvalidStrategy {
            detail: "factor requirements contain duplicate ids".to_owned(),
        });
    }
    let factors =
        factors_for(&requirements.factor_ids).map_err(|error| RecommendationError::Compute {
            detail: error.to_string(),
        })?;
    let snapshot = FactorSnapshotBuilder::new(
        as_of,
        frozen,
        &store,
        "kr",
        &pin.dataset_id,
        pin.curated_version,
    )
    .with_factors(factors)
    .build()
    .map_err(|error| RecommendationError::Compute {
        detail: error.to_string(),
    })?;

    let mut values = universe
        .members
        .iter()
        .map(|member| (member.clone(), BTreeMap::new()))
        .collect::<BTreeMap<_, _>>();
    let wanted_date = as_of.to_iso();
    for row in snapshot.rows {
        if row.date != wanted_date {
            continue;
        }
        let Some(raw) = row.raw else { continue };
        if !raw.is_finite() {
            return Err(RecommendationError::Compute {
                detail: format!("non-finite raw value for {} {}", row.instrument, row.factor),
            });
        }
        values
            .get_mut(&row.instrument)
            .expect("snapshot is frozen to the attested members")
            .insert(row.factor, raw);
    }
    Ok(ComputedClose {
        as_of,
        factors: values,
        factor_snapshot_hash: snapshot.hash.as_str().to_owned(),
    })
}

/// Tokio boundary for the CPU- and filesystem-heavy factor engine.
pub async fn compute_close_async(
    pin: AttestedDataset,
    universe: AttestedUniverse,
    as_of: TradingDate,
    requirements: StrategyRequirements,
) -> Result<ComputedClose, RecommendationError> {
    tokio::task::spawn_blocking(move || compute_close(&pin, &universe, as_of, &requirements))
        .await
        .map_err(|error| RecommendationError::BlockingTask {
            detail: error.to_string(),
        })?
}

fn validate_fixed_universe(universe: &AttestedUniverse) -> Result<(), RecommendationError> {
    let shipped = AttestedUniverse::from_manifest_yaml(include_str!(
        "../../../../configs/universes/kr-etf-core-v1.yaml"
    ))?;
    if universe != &shipped {
        return Err(RecommendationError::InvalidUniverse {
            detail: "universe attestation does not match the shipped fixed snapshot".to_owned(),
        });
    }
    Ok(())
}

/// Validate one resolved config against its shipped immutable package and
/// derive the parameter-dependent factors used by the target generator.
pub fn requirements_for(
    resolved: &ResolvedConfig,
) -> Result<StrategyRequirements, RecommendationError> {
    let package = baseline_packages()
        .into_iter()
        .find(|package| {
            package.strategy_id == resolved.strategy_id
                && package.version.to_string() == resolved.strategy_version
        })
        .ok_or_else(|| invalid("strategy id/version is not shipped by this build"))?;

    let owner = Actor::Owner;
    let member = Actor::Member("recommendation-worker".to_owned());
    let mut registry = Registry::new();
    registry
        .register(&owner, package)
        .map_err(|error| invalid(error.to_string()))?;
    registry
        .apply_member_config(
            &member,
            &resolved.strategy_id,
            &resolved.strategy_version,
            resolved.config.clone(),
        )
        .map_err(|error| invalid(error.to_string()))?;

    let requirement = match resolved.strategy_id.as_str() {
        "buy_and_hold" => StrategyRequirements {
            factor_ids: Vec::new(),
            minimum_lookback_sessions: 0,
        },
        "trend_following" => {
            let fast = integer_parameter(resolved, "fast_ma")?;
            let slow = integer_parameter(resolved, "slow_ma")?;
            let mut factor_ids = vec![format!("trend_{fast}")];
            if fast != slow {
                factor_ids.push(format!("trend_{slow}"));
            }
            StrategyRequirements {
                factor_ids,
                minimum_lookback_sessions: slow.max(fast),
            }
        }
        "relative_momentum" => match integer_parameter(resolved, "lookback_months")? {
            6 => StrategyRequirements {
                factor_ids: vec!["return_6m".to_owned()],
                minimum_lookback_sessions: 126,
            },
            12 => StrategyRequirements {
                factor_ids: vec!["momentum_12_1".to_owned()],
                minimum_lookback_sessions: 252,
            },
            _ => {
                return Err(invalid(
                    "validated relative-momentum lookback is unsupported",
                ));
            }
        },
        "dual_momentum" => match integer_parameter(resolved, "lookback_months")? {
            6 => StrategyRequirements {
                factor_ids: vec!["return_6m".to_owned()],
                minimum_lookback_sessions: 126,
            },
            12 => StrategyRequirements {
                factor_ids: vec!["return_12m".to_owned()],
                minimum_lookback_sessions: 252,
            },
            _ => return Err(invalid("validated dual-momentum lookback is unsupported")),
        },
        "inverse_volatility" => {
            let window = integer_parameter(resolved, "vol_window")?;
            StrategyRequirements {
                factor_ids: vec![format!("vol_{window}")],
                minimum_lookback_sessions: window,
            }
        }
        _ => return Err(invalid("strategy is not a recommendation baseline")),
    };
    Ok(requirement)
}

fn integer_parameter(
    resolved: &ResolvedConfig,
    name: &'static str,
) -> Result<u64, RecommendationError> {
    resolved
        .config
        .get(name)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            invalid(format!(
                "validated parameter {name} is not an unsigned integer"
            ))
        })
}

fn invalid(detail: impl Into<String>) -> RecommendationError {
    RecommendationError::InvalidStrategy {
        detail: detail.into(),
    }
}
