//! Actor-scoped repository over `user_strategy_configs` (tenant table,
//! `owner_user_id`, FORCE RLS). Every method takes an authenticated actor and
//! pins `app.actor_user_id` for the transaction.

use crate::actor_tx::begin_actor_tx;
use crate::error::{TenancyError, TenancyResult};
use auth::entitlement::Actor;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

/// A row of `user_strategy_configs` as the actor may see it.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct StrategyConfigRow {
    pub id: Uuid,
    pub owner_user_id: Uuid,
    pub strategy_id: String,
    pub strategy_version: String,
    pub config_json: Value,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating a strategy config. The `owner_user_id` is ALWAYS derived
/// from the actor inside the repository — a caller-supplied owner is ignored
/// (defense against crafted owner IDs; RLS `WITH CHECK` denies it anyway).
#[derive(Debug, Clone)]
pub struct NewStrategyConfig {
    pub strategy_id: String,
    pub strategy_version: String,
    pub config_json: Value,
    pub is_active: bool,
}

/// Typed repository over `user_strategy_configs`.
#[derive(Debug, Clone)]
pub struct StrategyConfigRepo {
    pool: sqlx::PgPool,
}

impl StrategyConfigRepo {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    /// Create a config owned by `actor`.
    pub async fn create(
        &self,
        actor: &Actor,
        input: NewStrategyConfig,
    ) -> TenancyResult<StrategyConfigRow> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let row = sqlx::query_as::<_, StrategyConfigRow>(
            "INSERT INTO user_strategy_configs \
             (owner_user_id, strategy_id, strategy_version, config_json, is_active) \
             VALUES ($1, $2, $3, $4, $5) \
             RETURNING id, owner_user_id, strategy_id, strategy_version, config_json, \
                       is_active, created_at, updated_at",
        )
        .bind(crate::actor_tx::actor_uuid(actor)?)
        .bind(&input.strategy_id)
        .bind(&input.strategy_version)
        .bind(&input.config_json)
        .bind(input.is_active)
        .fetch_one(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(row)
    }

    /// Fetch one of `actor`'s configs by id. A foreign row is indistinguishable
    /// from a missing one (RLS => zero rows => NotFound).
    pub async fn get(&self, actor: &Actor, id: Uuid) -> TenancyResult<StrategyConfigRow> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let row = sqlx::query_as::<_, StrategyConfigRow>(
            "SELECT id, owner_user_id, strategy_id, strategy_version, config_json, \
                    is_active, created_at, updated_at \
             FROM user_strategy_configs WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        crate::error::map_optional(row)
    }

    /// List `actor`'s own configs.
    pub async fn list(&self, actor: &Actor) -> TenancyResult<Vec<StrategyConfigRow>> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let rows = sqlx::query_as::<_, StrategyConfigRow>(
            "SELECT id, owner_user_id, strategy_id, strategy_version, config_json, \
                    is_active, created_at, updated_at \
             FROM user_strategy_configs ORDER BY created_at",
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(rows)
    }

    /// Update one of `actor`'s configs (config_json + is_active). A foreign row
    /// updates zero rows => NotFound.
    pub async fn update(
        &self,
        actor: &Actor,
        id: Uuid,
        config_json: Value,
        is_active: bool,
    ) -> TenancyResult<StrategyConfigRow> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let row = sqlx::query_as::<_, StrategyConfigRow>(
            "UPDATE user_strategy_configs SET config_json = $2, is_active = $3, \
                    updated_at = now() \
             WHERE id = $1 \
             RETURNING id, owner_user_id, strategy_id, strategy_version, config_json, \
                       is_active, created_at, updated_at",
        )
        .bind(id)
        .bind(&config_json)
        .bind(is_active)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        crate::error::map_optional(row)
    }

    /// Delete one of `actor`'s configs. A foreign row deletes zero rows =>
    /// NotFound.
    pub async fn delete(&self, actor: &Actor, id: Uuid) -> TenancyResult<()> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let deleted = sqlx::query("DELETE FROM user_strategy_configs WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        if deleted.rows_affected() == 0 {
            return Err(TenancyError::NotFound);
        }
        Ok(())
    }
}
