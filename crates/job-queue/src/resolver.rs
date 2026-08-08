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
use sqlx::PgPool;
use uuid::Uuid;

/// The strategies this build can actually run.
///
/// `(strategy_id, strategy_path, config_path)`.
///
/// # Why this list is one entry long
///
/// The five baseline packages in `nt/strategies/` are NOT here, and leaving
/// them out is the finding rather than an omission. They subclass
/// `TargetExecutionStrategy`, which records what it did in `order_intents`
/// and keeps no fills; the backtest worker collects results with
/// `getattr(strategy, "orders", [])` and `getattr(strategy, "fills", [])`.
/// Nothing raises — the getattr defaults hide the mismatch — so a baseline
/// strategy would complete, report SUCCEEDED, and write artifacts containing
/// zero orders and zero fills.
///
/// A user cannot tell that from a strategy that legitimately never traded, so
/// mapping them would turn a missing seam into a wrong ANSWER. Refusing is
/// louder and honest: the job fails with `STRATEGY_NOT_DEPLOYED`.
///
/// The entry that IS here is proven end to end — `nt/backtest-worker/tests/
/// test_worker.py` and `tests/golden/phase0/runner.py` both drive
/// `ma200_trend:MA200Trend` through the worker, and `backtest_runner.rs`
/// drives it through this runner.
///
/// Adding a baseline is therefore not an edit to this table. It is teaching
/// the adapters to expose `orders`/`fills` (or the worker to read
/// `order_intents`), proving it with a test that asserts a non-empty
/// artifact, and then adding the row.
const DEPLOYED: &[(&str, &str, &str)] = &[(
    "ma200_trend",
    "ma200_trend:MA200Trend",
    "ma200_trend:MA200TrendConfig",
)];

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

        // `owner_user_id` is bound, not merely checked afterwards. The `worker`
        // role's RLS policy on this table is `USING (true)` -- it serves every
        // tenant and has no `app.actor_user_id` to be filtered by -- so this
        // predicate is the ONLY thing standing between a job and another
        // tenant's configuration.
        let found: Option<(String, String, serde_json::Value)> = sqlx::query_as(
            "SELECT strategy_id, strategy_version, config_json \
             FROM user_strategy_configs \
             WHERE id = $1 AND owner_user_id = $2 AND is_active",
        )
        .bind(config_id)
        .bind(owner_user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| ResolveError::Unavailable(format!("strategy registry unreadable: {e}")))?;

        // Absent and not-yours are ONE answer on purpose. Distinguishing them
        // would let a caller probe which config ids exist under other owners.
        let (strategy_id, strategy_version, config) = found.ok_or_else(|| {
            ResolveError::NotFound(format!("no active strategy config {config_id}"))
        })?;

        let (_, strategy_path, config_path) = DEPLOYED
            .iter()
            .find(|(id, _, _)| *id == strategy_id)
            .ok_or_else(|| {
                ResolveError::Unknown(format!(
                    "strategy {strategy_id:?} has no runnable implementation in this build"
                ))
            })?;

        Ok(ResolvedStrategy {
            strategy_path: (*strategy_path).to_string(),
            config_path: (*config_path).to_string(),
            strategy_id,
            strategy_version,
            config,
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
    fn the_baseline_packages_are_deliberately_absent() {
        // Guards the reasoning in DEPLOYED's docs rather than the list itself.
        // Adding one of these without first fixing the adapter/worker
        // collection mismatch produces backtests that report SUCCEEDED with
        // zero orders -- a wrong answer the user cannot distinguish from a
        // strategy that chose not to trade.
        for baseline in [
            "buy_and_hold",
            "dual_momentum",
            "inverse_volatility",
            "relative_momentum",
            "trend_following",
        ] {
            assert!(
                !DEPLOYED.iter().any(|(id, _, _)| *id == baseline),
                "{baseline} was added to DEPLOYED. That is only correct once the \
                 adapter exposes `orders`/`fills` (or the worker reads \
                 `order_intents`) AND a test asserts a non-empty artifact -- \
                 delete this case in the same commit that proves it."
            );
        }
    }
}
