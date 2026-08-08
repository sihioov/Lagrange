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
/// # Why four baselines are still missing
///
/// Not an omission — they cannot produce a correct answer yet. Every baseline
/// except `buy_and_hold` needs a factor series (`return_12m`, `momentum_12_1`,
/// `vol_60`, `trend_50`/`trend_200`), the canonical definitions of those live
/// in the Rust `factor-engine`, and nothing yet carries a factor series into a
/// backtest run.
///
/// They are not given a Python reimplementation to unblock them: a second
/// `return_12m` would make a backtest disagree with the paper and live paths
/// that use the Rust one, and the Paper promotion gate is a parity check. A
/// strategy promoted on a backtest that does not describe how it will behave
/// is worse than one that cannot be backtested at all.
///
/// They no longer fail SILENTLY, which was the actual danger. The adapter
/// records a `MISSING_FACTOR_SUPPLY` failure that the worker turns into a
/// FAILED run, where before the run completed and reported SUCCEEDED with
/// zero orders — indistinguishable from a strategy that chose not to trade.
///
/// Both entries here are proven end to end against the phase-0 dataset, with
/// non-empty `orders`/`fills` artifacts rather than merely a SUCCEEDED status.
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
    fn the_factor_dependent_baselines_are_deliberately_absent() {
        // `buy_and_hold` has left this list, which is what the previous
        // version of this test asked for: the adapter now submits real orders
        // and a runner test asserts non-empty artifacts.
        //
        // The four that remain need a factor series nobody supplies. Adding
        // one before that exists means either a backtest with no orders or a
        // Python reimplementation of a Rust factor -- the first is a wrong
        // answer, the second breaks parity with the paper and live paths that
        // the Paper promotion gate compares against.
        for baseline in [
            "dual_momentum",
            "inverse_volatility",
            "relative_momentum",
            "trend_following",
        ] {
            assert!(
                !DEPLOYED.iter().any(|(id, _, _)| *id == baseline),
                "{baseline} was added to DEPLOYED. That is only correct once a \
                 factor series computed by the Rust factor-engine reaches the \
                 backtest AND a test asserts non-empty orders/fills -- delete \
                 this case in the same commit that proves it."
            );
        }
    }
}
