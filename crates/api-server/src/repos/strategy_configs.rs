//! Actor-scoped strategy configuration access. The database owns the daily epoch.
use crate::actor_tx::begin_actor_tx;
use crate::error::{TenancyError, TenancyResult};
use auth::entitlement::Actor;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct StrategyConfigRow {
    pub id: Uuid,
    pub owner_user_id: Uuid,
    pub strategy_id: String,
    pub strategy_version: String,
    pub config_json: Value,
    pub is_active: bool,
    pub daily_enabled: bool,
    pub daily_epoch: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
/// Existing callers create ordinary configurations without automatic opt-in.
#[derive(Debug, Clone)]
pub struct NewStrategyConfig {
    pub strategy_id: String,
    pub strategy_version: String,
    pub config_json: Value,
    pub is_active: bool,
}
#[derive(Debug, Clone)]
pub struct StrategyConfigRepo {
    pool: sqlx::PgPool,
}
impl StrategyConfigRepo {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
    pub async fn create(
        &self,
        actor: &Actor,
        input: NewStrategyConfig,
    ) -> TenancyResult<StrategyConfigRow> {
        self.create_with_daily(actor, input, false).await
    }
    pub async fn create_with_daily(
        &self,
        actor: &Actor,
        input: NewStrategyConfig,
        daily_enabled: bool,
    ) -> TenancyResult<StrategyConfigRow> {
        if daily_enabled && !actor.is_owner() {
            return Err(TenancyError::Forbidden);
        }
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let row = sqlx::query_as::<_, StrategyConfigRow>(
            "INSERT INTO user_strategy_configs
             (owner_user_id,strategy_id,strategy_version,config_json,is_active,daily_enabled)
             VALUES($1,$2,$3,$4,$5,$6)
             RETURNING id,owner_user_id,strategy_id,strategy_version,config_json,is_active,
                       daily_enabled,daily_epoch,created_at,updated_at",
        )
        .bind(crate::actor_tx::actor_uuid(actor)?)
        .bind(input.strategy_id)
        .bind(input.strategy_version)
        .bind(input.config_json)
        .bind(input.is_active)
        .bind(daily_enabled)
        .fetch_one(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(row)
    }
    pub async fn get(&self, actor: &Actor, id: Uuid) -> TenancyResult<StrategyConfigRow> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let row = sqlx::query_as::<_, StrategyConfigRow>(
            "SELECT id,owner_user_id,strategy_id,strategy_version,config_json,is_active,
                    daily_enabled,daily_epoch,created_at,updated_at
             FROM user_strategy_configs WHERE id=$1",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        crate::error::map_optional(row)
    }
    pub async fn list(&self, actor: &Actor) -> TenancyResult<Vec<StrategyConfigRow>> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let rows = sqlx::query_as::<_, StrategyConfigRow>(
            "SELECT id,owner_user_id,strategy_id,strategy_version,config_json,is_active,
                    daily_enabled,daily_epoch,created_at,updated_at
             FROM user_strategy_configs ORDER BY created_at,id",
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(rows)
    }
    /// The old API preserves opt-in; a normal parameter edit still advances SQL's epoch.
    pub async fn update(
        &self,
        actor: &Actor,
        id: Uuid,
        config_json: Value,
        is_active: bool,
    ) -> TenancyResult<StrategyConfigRow> {
        self.update_with_daily(actor, id, Some(config_json), Some(is_active), None)
            .await
    }
    pub async fn update_with_daily(
        &self,
        actor: &Actor,
        id: Uuid,
        config_json: Option<Value>,
        is_active: Option<bool>,
        daily_enabled: Option<bool>,
    ) -> TenancyResult<StrategyConfigRow> {
        if daily_enabled == Some(true) && !actor.is_owner() {
            return Err(TenancyError::Forbidden);
        }
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let row = sqlx::query_as::<_, StrategyConfigRow>(
            "UPDATE user_strategy_configs SET config_json=COALESCE($2,config_json),
                 is_active=COALESCE($3,is_active),daily_enabled=COALESCE($4,daily_enabled),
                 updated_at=now()
             WHERE id=$1
             RETURNING id,owner_user_id,strategy_id,strategy_version,config_json,is_active,
                       daily_enabled,daily_epoch,created_at,updated_at",
        )
        .bind(id)
        .bind(config_json)
        .bind(is_active)
        .bind(daily_enabled)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        crate::error::map_optional(row)
    }
    pub async fn delete(&self, actor: &Actor, id: Uuid) -> TenancyResult<()> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let deleted = sqlx::query("DELETE FROM user_strategy_configs WHERE id=$1")
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
