//! Actor-scoped transaction: the single place that pins the RLS actor GUC.
//!
//! Every repository method opens a transaction via [`begin_actor_tx`], which
//! executes `SELECT set_config('app.actor_user_id', $1, true)` — the SQL
//! equivalent of `SET LOCAL` — before any statement runs. The GUC is scoped to
//! the transaction and resets at COMMIT/ROLLBACK, so a shared pool connection
//! never leaks one user's actor context into another request.
//!
//! Policy side (migration `0010_rls`) reads the same GUC:
//! `owner_user_id = current_setting('app.actor_user_id', true)::uuid`.
//! Unset GUC => the comparison is NULL => the row is invisible (reads) or the
//! statement is denied (`42501`, writes).

use crate::error::{TenancyError, TenancyResult};
use auth::entitlement::Actor;
use job_queue::paper_execution::set_paper_transaction_timeouts;
use sqlx::{PgConnection, Postgres, Transaction};

/// The custom GUC that carries the authenticated actor into RLS policies.
pub const ACTOR_GUC: &str = "app.actor_user_id";

/// Parse the actor's user id as a database `uuid`. Tenant columns are `uuid`;
/// PostgreSQL has NO implicit text->uuid assignment cast, so the repositories
/// bind the parsed value. An actor whose id is not a uuid cannot address a
/// tenant row and is treated as forbidden (fail closed).
pub fn actor_uuid(actor: &Actor) -> TenancyResult<uuid::Uuid> {
    uuid::Uuid::parse_str(&actor.user_id.0).map_err(|_| TenancyError::Forbidden)
}

/// Open a transaction and pin the actor context (`SET LOCAL`) inside it.
///
/// Returns a transaction whose every statement is RLS-scoped to `actor`.
/// Callers MUST commit (or rollback); the transaction is not auto-committed.
pub async fn begin_actor_tx(
    pool: &sqlx::PgPool,
    actor: &Actor,
) -> TenancyResult<Transaction<'static, Postgres>> {
    let mut tx = pool.begin().await.map_err(TenancyError::from_sqlx)?;
    set_actor_guc(&mut tx, actor).await?;
    set_paper_transaction_timeouts(&mut tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
    Ok(tx)
}

/// Pin `app.actor_user_id` on an open transaction (SET LOCAL semantics).
pub async fn set_actor_guc(conn: &mut PgConnection, actor: &Actor) -> TenancyResult<()> {
    sqlx::query("SELECT set_config($1, $2, true)")
        .bind(ACTOR_GUC)
        .bind(actor.user_id.0.as_str())
        .execute(conn)
        .await
        .map_err(TenancyError::from_sqlx)?;
    Ok(())
}

/// Build a `PgPool` whose connections carry a fixed actor context
/// (`app.actor_user_id` as a connection startup option). Used by the
/// integration harness to emulate "connection as user X" for raw SQL probes.
pub async fn pool_for_actor(
    url: &str,
    user_id: &str,
    max_connections: u32,
) -> TenancyResult<sqlx::PgPool> {
    let opts = url
        .parse::<sqlx::postgres::PgConnectOptions>()
        .map_err(TenancyError::from_sqlx)?
        .options([(ACTOR_GUC, user_id.to_string())]);
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(max_connections)
        .connect_with(opts)
        .await
        .map_err(TenancyError::from_sqlx)
}
