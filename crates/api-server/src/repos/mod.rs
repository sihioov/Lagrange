//! Tenant repositories: actor-scoped CRUD over `user_strategy_configs`,
//! `accounts`, `backtest_runs`, `result_artifacts`, shared read-only data,
//! and the audited admin pathway.

pub mod accounts;
pub mod admin;
pub mod artifacts;
pub mod audit;
pub mod backtest_runs;
pub mod entitlements;
pub mod live;
pub mod metrics;
pub mod ops;
pub mod order_intents;
pub mod paper;
pub mod parity;
pub mod pending_targets;
pub mod recommendations;
pub mod risk;
pub mod robustness;
pub mod shared;
pub mod strategies;
pub mod strategy_configs;

use crate::http::pagination::Cursor;

/// Split a `limit+1` probe into (page, next_cursor) under the stable
/// `(created_at, id)` ordering.
pub fn split_page<T: Clone>(
    rows: Vec<T>,
    limit: usize,
    anchor: impl Fn(&T) -> (String, String),
) -> (Vec<T>, Option<Cursor>) {
    if rows.len() > limit {
        let (page, _) = rows.split_at(limit);
        let (k, i) = anchor(page.last().expect("non-empty page"));
        (page.to_vec(), Some(Cursor { k, i }))
    } else {
        (rows, None)
    }
}
