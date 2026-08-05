//! Todo 3 migration contract gate: schemas, roles, and immutable state
//! boundaries for the Lagrange Station PostgreSQL database.
//!
//! This crate exists because of a PLAN-COMMAND DEFECT: the plan's QA line for
//! Todo 3 is `cargo test -p api-server --test migration_contract`, but
//! `apps/api-server` is the Node/TypeScript application, not a Rust crate. The
//! Rust gate therefore lives here (root workspace member `migration-contract`)
//! and embeds `migrations/` via `sqlx::migrate!("../../../migrations")`.
//!
//! See `tests/migration_contract.rs` for the contract assertions.
