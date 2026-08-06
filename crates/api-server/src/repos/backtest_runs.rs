//! Actor-scoped repository over `backtest_runs` (tenant table,
//! `owner_user_id`, FORCE RLS).

use crate::actor_tx::{actor_uuid, begin_actor_tx};
use crate::error::{TenancyError, TenancyResult};
use auth::entitlement::Actor;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

/// A row of `backtest_runs` as the actor may see it.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct BacktestRunRow {
    pub id: Uuid,
    pub owner_user_id: Uuid,
    pub job_id: Option<Uuid>,
    pub strategy_id: String,
    pub strategy_version: String,
    pub dataset_version: String,
    pub engine: String,
    pub engine_version: String,
    pub config_sha256: String,
    pub code_commit: String,
    pub random_seed: Option<i32>,
    pub timezone: String,
    pub status: String,
    pub summary_json: Value,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Input for creating a backtest run. Ownership is derived from the actor only.
#[derive(Debug, Clone)]
pub struct NewBacktestRun {
    pub strategy_id: String,
    pub strategy_version: String,
    pub dataset_version: String,
    pub engine_version: String,
    pub config_sha256: String,
    pub code_commit: String,
    pub random_seed: Option<i32>,
    pub timezone: String,
    pub summary_json: Value,
}

/// Typed repository over `backtest_runs`.
#[derive(Debug, Clone)]
pub struct BacktestRunRepo {
    pool: sqlx::PgPool,
}

impl BacktestRunRepo {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    /// Create a run owned by `actor`.
    pub async fn create(
        &self,
        actor: &Actor,
        input: NewBacktestRun,
    ) -> TenancyResult<BacktestRunRow> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let row = sqlx::query_as::<_, BacktestRunRow>(
            "INSERT INTO backtest_runs \
             (owner_user_id, strategy_id, strategy_version, dataset_version, \
              engine, engine_version, config_sha256, code_commit, random_seed, \
              timezone, summary_json) \
             VALUES ($1, $2, $3, $4, 'nautilustrader', $5, $6, $7, $8, $9, $10) \
             RETURNING id, owner_user_id, job_id, strategy_id, strategy_version, \
                       dataset_version, engine, engine_version, config_sha256, \
                       code_commit, random_seed, timezone, status, summary_json, \
                       started_at, finished_at, created_at",
        )
        .bind(actor_uuid(actor)?)
        .bind(&input.strategy_id)
        .bind(&input.strategy_version)
        .bind(&input.dataset_version)
        .bind(&input.engine_version)
        .bind(&input.config_sha256)
        .bind(&input.code_commit)
        .bind(input.random_seed)
        .bind(&input.timezone)
        .bind(&input.summary_json)
        .fetch_one(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(row)
    }

    /// Fetch one of `actor`'s runs by id; a foreign row => NotFound.
    pub async fn get(&self, actor: &Actor, id: Uuid) -> TenancyResult<BacktestRunRow> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let row = sqlx::query_as::<_, BacktestRunRow>(
            "SELECT id, owner_user_id, job_id, strategy_id, strategy_version, \
                    dataset_version, engine, engine_version, config_sha256, \
                    code_commit, random_seed, timezone, status, summary_json, \
                    started_at, finished_at, created_at \
             FROM backtest_runs WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        crate::error::map_optional(row)
    }

    /// List `actor`'s own runs.
    pub async fn list(&self, actor: &Actor) -> TenancyResult<Vec<BacktestRunRow>> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let rows = sqlx::query_as::<_, BacktestRunRow>(
            "SELECT id, owner_user_id, job_id, strategy_id, strategy_version, \
                    dataset_version, engine, engine_version, config_sha256, \
                    code_commit, random_seed, timezone, status, summary_json, \
                    started_at, finished_at, created_at \
             FROM backtest_runs ORDER BY created_at",
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(rows)
    }
}
