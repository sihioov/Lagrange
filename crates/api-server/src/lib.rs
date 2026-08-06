//! `api-server` — typed tenancy repositories for Lagrange Station.
//!
//! Todo 23: ownership is enforced in depth. Every repository method receives an
//! **authenticated actor** ([`auth::entitlement::Actor`], derived from a
//! validated session — never from a URL, form field, or repository filter) and
//! executes inside a transaction that pins `app.actor_user_id` with
//! `set_config(..., true)` (`SET LOCAL`), so PostgreSQL Row-Level Security
//! (migration `0010_rls`, enabled AND forced on every tenant table) scopes all
//! row access to that actor. The same policy matrix makes:
//!
//! - tenant rows invisible/immutable to every other actor (`Member A cannot
//!   select/update/delete Member B resources` — even by direct ID guess);
//! - crafted `owner_user_id` inserts fail with `42501` at the database;
//! - shared dataset/factor tables read-only for serving roles;
//! - `audit_logs` append-only (INSERT via the `audit_writer` role only);
//! - the `migration_owner` (table owner, FORCE RLS, no policies) unable to
//!   bypass RLS without an explicit actor context;
//! - a separate, explicit, audited admin pathway via the dedicated `admin`
//!   role (Owner-gated at the repository).
//!
//! Session-derived actor context comes from `auth::sessions::SessionInfo`
//! (opaque cookie -> hashed session -> `Actor`), so a repository never trusts
//! a URL user id, UI hiding, or its own `WHERE` filters alone.

pub mod actor_tx;
pub mod contract;
pub mod error;
pub mod http;
pub mod repos;

pub use error::{TenancyError, TenancyResult};
pub use repos::accounts::{AccountRepo, AccountRow, NewAccount};
pub use repos::admin::{AdminRepo, JobRow};
pub use repos::artifacts::{ArtifactRepo, ArtifactRow};
pub use repos::audit::{AuditEntry, AuditWriter};
pub use repos::backtest_runs::{BacktestRunRepo, BacktestRunRow, NewBacktestRun};
pub use repos::shared::{DatasetVersionRow, InstrumentRow, SharedDataRepo, SnapshotManifestRow};
pub use repos::strategy_configs::{NewStrategyConfig, StrategyConfigRepo, StrategyConfigRow};
