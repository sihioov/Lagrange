//! Tenant repositories: actor-scoped CRUD over `user_strategy_configs`,
//! `accounts`, `backtest_runs`, `result_artifacts`, shared read-only data,
//! and the audited admin pathway.

pub mod accounts;
pub mod admin;
pub mod artifacts;
pub mod audit;
pub mod backtest_runs;
pub mod shared;
pub mod strategy_configs;
