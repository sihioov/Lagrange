//! Owner-only Live broker connections, nodes, and the kill switch
//! (plan Todo 37).
//!
//! These rows hold NO credential material — only references to where a
//! credential lives, plus a masked account number for display. Migration 0016's
//! CHECK constraints enforce that; this layer never widens it.
//!
//! `broker_connections` is a 0007 TENANT table with FORCE RLS on the actor
//! GUC, so every call runs inside an actor transaction exactly like the other
//! repositories. That RLS is a SECOND fence: the Owner-only boundary is
//! enforced above this layer by role plus fresh MFA, and a Member request never
//! reaches here at all. The repo holds its actor so the gate cannot be skipped
//! by calling a method without one.

use crate::actor_tx::begin_actor_tx;
use crate::error::{TenancyError, TenancyResult};
use auth::entitlement::Actor;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// A configured broker connection. Note what is absent: no key, no secret, no
/// full account number.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct BrokerConnectionRow {
    pub id: Uuid,
    pub label: String,
    pub profile: String,
    pub account_no_masked: String,
    pub account_product_code: String,
    pub app_key_ref: String,
    pub secret_ref: String,
    /// Where the FULL account number lives. Present on the ROW because the
    /// submission path needs to resolve it, and deliberately ABSENT from
    /// `BrokerConnectionDto`: the API discloses reference locations for the
    /// credentials an Owner configured, but the account reference is only ever
    /// read by the server.
    pub account_ref: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A running (or stopped) Live node.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct BrokerNodeRow {
    pub id: Uuid,
    pub connection_id: Uuid,
    pub status: String,
    pub process_id: Option<String>,
    pub started_at: DateTime<Utc>,
    pub stopped_at: Option<DateTime<Utc>>,
    pub stop_reason: Option<String>,
}

/// Input for configuring a connection. Every credential field is a REFERENCE.
#[derive(Debug, Clone)]
pub struct NewBrokerConnection {
    pub label: String,
    pub profile: String,
    pub account_no_masked: String,
    pub account_product_code: String,
    /// Where the FULL account number lives (0007's column). The masked form is
    /// the only rendering any surface may show.
    pub account_ref: String,
    pub app_key_ref: String,
    pub secret_ref: String,
}

// Column lists are written inline in each query rather than shared as
// constants: sqlx's compile-time SQL audit rejects an interpolated fragment,
// so a shared constant could only ever be checked by a test that the queries
// themselves do not read — assurance that looks real and is not.

#[derive(Debug, Clone)]
pub struct LiveRepo {
    pool: sqlx::PgPool,
    actor: Actor,
}

impl LiveRepo {
    pub fn new(pool: sqlx::PgPool, actor: Actor) -> Self {
        Self { pool, actor }
    }

    pub async fn create_connection(
        &self,
        owner_user_id: Uuid,
        input: NewBrokerConnection,
    ) -> TenancyResult<BrokerConnectionRow> {
        let mut tx = begin_actor_tx(&self.pool, &self.actor).await?;
        // `broker` and `account_ref` are 0007's NOT NULL columns. Phase 3 is
        // KIS-only, so the broker is fixed here rather than caller-supplied.
        let row = sqlx::query_as::<_, BrokerConnectionRow>(
            "INSERT INTO broker_connections \
             (owner_user_id, broker, account_ref, label, profile, account_no_masked, \
              account_product_code, app_key_ref, secret_ref) \
             VALUES ($1, 'KIS', $2, $3, $4, $5, $6, $7, $8) \
             RETURNING id, label, profile, account_no_masked, account_product_code, \
                       app_key_ref, secret_ref, account_ref, status, created_at, updated_at",
        )
        .bind(owner_user_id)
        .bind(&input.account_ref)
        .bind(&input.label)
        .bind(&input.profile)
        .bind(&input.account_no_masked)
        .bind(&input.account_product_code)
        .bind(&input.app_key_ref)
        .bind(&input.secret_ref)
        .fetch_one(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(row)
    }

    pub async fn list_connections(&self) -> TenancyResult<Vec<BrokerConnectionRow>> {
        let mut tx = begin_actor_tx(&self.pool, &self.actor).await?;
        let rows = sqlx::query_as::<_, BrokerConnectionRow>(
            "SELECT id, label, profile, account_no_masked, account_product_code, \
                    app_key_ref, secret_ref, account_ref, status, created_at, updated_at \
             FROM broker_connections ORDER BY created_at",
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(rows)
    }

    pub async fn get_connection(&self, id: Uuid) -> TenancyResult<BrokerConnectionRow> {
        let mut tx = begin_actor_tx(&self.pool, &self.actor).await?;
        let row = sqlx::query_as::<_, BrokerConnectionRow>(
            "SELECT id, label, profile, account_no_masked, account_product_code, \
                    app_key_ref, secret_ref, account_ref, status, created_at, updated_at \
             FROM broker_connections WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        crate::error::map_optional(row)
    }

    /// Start a node.
    ///
    /// The schema's partial unique index refuses a second active node for the
    /// same connection, so two concurrent start requests cannot both win. Two
    /// nodes on one account would double every order it places.
    pub async fn start_node(
        &self,
        owner_user_id: Uuid,
        connection_id: Uuid,
    ) -> TenancyResult<BrokerNodeRow> {
        let mut tx = begin_actor_tx(&self.pool, &self.actor).await?;
        let row = sqlx::query_as::<_, BrokerNodeRow>(
            "INSERT INTO broker_nodes (connection_id, owner_user_id) VALUES ($1, $2) \
             RETURNING id, connection_id, status, process_id, started_at, stopped_at, stop_reason",
        )
        .bind(connection_id)
        .bind(owner_user_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(row)
    }

    /// Stop a node. Guarded on a non-STOPPED status, so two stop requests
    /// cannot both claim to have stopped it.
    pub async fn stop_node(&self, node_id: Uuid, reason: &str) -> TenancyResult<BrokerNodeRow> {
        let mut tx = begin_actor_tx(&self.pool, &self.actor).await?;
        let row = sqlx::query_as::<_, BrokerNodeRow>(
            "UPDATE broker_nodes SET status = 'STOPPED', stopped_at = now(), stop_reason = $2 \
             WHERE id = $1 AND status <> 'STOPPED' \
             RETURNING id, connection_id, status, process_id, started_at, stopped_at, stop_reason",
        )
        .bind(node_id)
        .bind(reason)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        crate::error::map_optional(row)
    }

    pub async fn active_node(&self, connection_id: Uuid) -> TenancyResult<Option<BrokerNodeRow>> {
        let mut tx = begin_actor_tx(&self.pool, &self.actor).await?;
        let row = sqlx::query_as::<_, BrokerNodeRow>(
            "SELECT id, connection_id, status, process_id, started_at, stopped_at, stop_reason \
             FROM broker_nodes WHERE connection_id = $1 AND status <> 'STOPPED'",
        )
        .bind(connection_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(row)
    }

    /// Whether Live is currently disabled. Read before every start.
    pub async fn kill_switch_engaged(&self) -> TenancyResult<bool> {
        let mut tx = begin_actor_tx(&self.pool, &self.actor).await?;
        let engaged =
            sqlx::query_scalar::<_, bool>("SELECT engaged FROM live_kill_switch WHERE id = true")
                .fetch_one(&mut *tx)
                .await
                .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(engaged)
    }

    /// Engage or disengage the kill switch.
    ///
    /// Engaging is a safety action; DISENGAGING is the dangerous one, and the
    /// API layer gives it its own route and audit action for that reason.
    pub async fn set_kill_switch(
        &self,
        engaged: bool,
        reason: &str,
        changed_by: Uuid,
    ) -> TenancyResult<bool> {
        let mut tx = begin_actor_tx(&self.pool, &self.actor).await?;
        let now = sqlx::query_scalar::<_, bool>(
            "UPDATE live_kill_switch SET engaged = $1, reason = $2, changed_by = $3, \
                    changed_at = now() WHERE id = true RETURNING engaged",
        )
        .bind(engaged)
        .bind(reason)
        .bind(changed_by)
        .fetch_one(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(now)
    }
}
