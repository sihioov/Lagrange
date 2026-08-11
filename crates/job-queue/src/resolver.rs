//! Turning a config id into the code that runs it.
//!
//! Two steps, deliberately separate. The database says which STRATEGY and
//! which PARAMETERS a config names; a table compiled into this binary says
//! which module and class that strategy is. The second step never consults the
//! database and never consults the request.
//!
//! # Why the module path is not stored
//!
//! It would be convenient to keep `module:Class` in `strategies` and read it
//! with the rest. It would also mean anything that can write that row chooses
//! what code the worker imports, and a backtest submission would be one SQL
//! injection away from remote code execution. [`DEPLOYED`] is a closed set
//! fixed at compile time for that reason: an id that is not in it does not
//! run, whatever the database says.

use crate::runner::{ResolveError, ResolvedStrategy, StrategyResolver};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

/// An active strategy configuration resolved under an explicit owner.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedConfig {
    pub strategy_id: String,
    pub strategy_version: String,
    pub config: serde_json::Value,
}

/// Resolve an active configuration without choosing an executable module.
pub async fn resolve_config(
    pool: &PgPool,
    config_id: Uuid,
    owner_user_id: Uuid,
) -> Result<ResolvedConfig, ResolveError> {
    let mut connection = pool
        .acquire()
        .await
        .map_err(|e| ResolveError::Unavailable(format!("strategy registry unreadable: {e}")))?;
    resolve_config_on(&mut connection, config_id, owner_user_id).await
}

pub(crate) async fn resolve_config_on(
    connection: &mut PgConnection,
    config_id: Uuid,
    owner_user_id: Uuid,
) -> Result<ResolvedConfig, ResolveError> {
    // The owner is part of the predicate, not checked after fetching. Worker
    // RLS intentionally exposes all tenants so the claimed identity must be
    // the authorization boundary here.
    let found: Option<(String, String, serde_json::Value)> = sqlx::query_as(
        "SELECT strategy_id, strategy_version, config_json \
         FROM user_strategy_configs \
         WHERE id = $1 AND owner_user_id = $2 AND is_active",
    )
    .bind(config_id)
    .bind(owner_user_id)
    .fetch_optional(connection)
    .await
    .map_err(|e| ResolveError::Unavailable(format!("strategy registry unreadable: {e}")))?;

    let (strategy_id, strategy_version, config) = found
        .ok_or_else(|| ResolveError::NotFound(format!("no active strategy config {config_id}")))?;
    Ok(ResolvedConfig {
        strategy_id,
        strategy_version,
        config,
    })
}

/// The strategies this build can actually run.
///
/// `(strategy_id, strategy_path, config_path)`.
///
/// # Why two baselines are still missing
///
/// `dual_momentum` and `relative_momentum` are blocked by the DATA, not by
/// this build. Both declare a 252-session lookback and the phase-0 dataset
/// holds 260 sessions, which leaves no month-end that is not also the final
/// session — and a rebalance on the final session has no later open to fill
/// it. The runner settles them permanently as `DATASET_TOO_SHORT` with both
/// numbers in the message, because a dataset does not grow between retries.
///
/// Nothing is faked to unblock them. Extending the synthetic span would
/// create a new immutable version after `kr-etf-daily-phase0-v2`, and
/// reimplementing their factors in Python would make a backtest disagree with
/// the paper and live paths that use the Rust engine — the Paper promotion
/// gate is a parity check between exactly those two.
///
/// Every entry here is proven end to end against the phase-0 dataset with
/// non-empty `orders`/`fills`, rather than merely a SUCCEEDED status.
const DEPLOYED: &[(&str, &str, &str)] = &[
    (
        "ma200_trend",
        "ma200_trend:MA200Trend",
        "ma200_trend:MA200TrendConfig",
    ),
    (
        "buy_and_hold",
        "strategies.buy_and_hold.adapter:BuyAndHoldAdapter",
        "strategies.buy_and_hold.adapter:BuyAndHoldConfig",
    ),
    (
        "inverse_volatility",
        "strategies.inverse_volatility.adapter:InverseVolatilityAdapter",
        "strategies.inverse_volatility.adapter:InverseVolatilityConfig",
    ),
    (
        "trend_following",
        "strategies.trend_following.adapter:TrendFollowingAdapter",
        "strategies.trend_following.adapter:TrendFollowingConfig",
    ),
];

/// Looks a config up in `user_strategy_configs`.
pub struct DbStrategyResolver {
    pool: PgPool,
}

impl DbStrategyResolver {
    pub fn new(pool: PgPool) -> DbStrategyResolver {
        DbStrategyResolver { pool }
    }
}

impl StrategyResolver for DbStrategyResolver {
    async fn resolve(
        &self,
        strategy_config_id: &str,
        owner_user_id: Uuid,
    ) -> Result<ResolvedStrategy, ResolveError> {
        let config_id = Uuid::parse_str(strategy_config_id).map_err(|_| {
            ResolveError::NotFound(format!(
                "strategy_config_id {strategy_config_id:?} is not a uuid"
            ))
        })?;

        let resolved = resolve_config(&self.pool, config_id, owner_user_id).await?;

        let (_, strategy_path, config_path) = DEPLOYED
            .iter()
            .find(|(id, _, _)| *id == resolved.strategy_id)
            .ok_or_else(|| {
                ResolveError::Unknown(format!(
                    "strategy {:?} has no runnable implementation in this build",
                    resolved.strategy_id
                ))
            })?;

        Ok(ResolvedStrategy {
            strategy_path: (*strategy_path).to_string(),
            config_path: (*config_path).to_string(),
            strategy_id: resolved.strategy_id,
            strategy_version: resolved.strategy_version,
            config: resolved.config,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_deployed_entry_names_a_module_and_a_class() {
        // A path without a colon silently becomes `module=""` in the worker's
        // `partition(":")`, and the import fails at run time inside a child
        // process -- far from this table.
        for (id, strategy_path, config_path) in DEPLOYED {
            for path in [strategy_path, config_path] {
                let (module, class) = path.split_once(':').unwrap_or_else(|| {
                    panic!("{id}: {path:?} is not module:Class");
                });
                assert!(!module.is_empty() && !class.is_empty(), "{id}: {path:?}");
            }
        }
    }

    #[test]
    fn the_data_blocked_baselines_are_deliberately_absent() {
        // `inverse_volatility` and `trend_following` have left this list, as
        // the previous version of this test asked: the factor series computed
        // by the Rust engine now reaches the backtest, and runner tests
        // assert non-empty orders rather than a SUCCEEDED status.
        //
        // These two are blocked by the DATASET, which no change here can fix.
        // Both declare a 252-session lookback and phase-0 holds 260 sessions,
        // so they have no month-end that is not also the final session -- and
        // a rebalance there has no later open to fill it.
        for baseline in ["dual_momentum", "relative_momentum"] {
            assert!(
                !DEPLOYED.iter().any(|(id, _, _)| *id == baseline),
                "{baseline} was added to DEPLOYED. It declares a 252-session \
                 lookback and phase-0 holds 260 sessions, so it has no valid \
                 rebalance date -- delete this case only alongside a dataset \
                 that gives it one, and a test asserting non-empty orders."
            );
        }
    }
}
