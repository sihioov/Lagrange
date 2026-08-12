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

#[derive(Debug, Clone)]
pub struct NewQueuedBacktest {
    pub run: NewBacktestRun,
    pub payload: Value,
    pub idempotency_key: Option<String>,
    pub max_jobs_per_owner: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum SubmitBacktestError {
    #[error(transparent)]
    Tenancy(#[from] TenancyError),
    #[error("per-owner job capacity exceeded")]
    CapacityExceeded,
    #[error("idempotency key was already used with different backtest input")]
    IdempotencyMismatch,
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

    /// Atomically reserve global owner capacity, create the run, and enqueue
    /// its job under the same lock used by recommendation submissions.
    pub async fn create_and_enqueue(
        &self,
        actor: &Actor,
        input: NewQueuedBacktest,
    ) -> Result<BacktestRunRow, SubmitBacktestError> {
        let owner = actor_uuid(actor)?;
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        crate::repos::lock_owner_job_capacity(&mut tx, owner).await?;

        if let Some(key) = input.idempotency_key.as_deref() {
            let existing: Option<(Uuid, String, Value)> = sqlx::query_as(
                "SELECT id, job_type, payload_json FROM jobs \
                 WHERE owner_user_id = $1 AND idempotency_key = $2 FOR SHARE",
            )
            .bind(owner)
            .bind(key)
            .fetch_optional(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?;
            if let Some((job_id, job_type, mut payload)) = existing {
                if let Some(object) = payload.as_object_mut() {
                    object.remove("run_id");
                }
                if job_type != "backtest" || payload != input.payload {
                    return Err(SubmitBacktestError::IdempotencyMismatch);
                }
                let row = sqlx::query_as::<_, BacktestRunRow>(
                    "SELECT id, owner_user_id, job_id, strategy_id, strategy_version, \
                            dataset_version, engine, engine_version, config_sha256, code_commit, \
                            random_seed, timezone, status, summary_json, started_at, finished_at, created_at \
                     FROM backtest_runs WHERE job_id = $1",
                )
                .bind(job_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(TenancyError::from_sqlx)?
                .ok_or(TenancyError::NotFound)?;
                tx.commit().await.map_err(TenancyError::from_sqlx)?;
                return Ok(row);
            }
        }

        let active: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM jobs WHERE owner_user_id = $1 AND status IN ('QUEUED', 'RUNNING')",
        )
        .bind(owner)
        .fetch_one(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        if active >= input.max_jobs_per_owner as i64 {
            return Err(SubmitBacktestError::CapacityExceeded);
        }

        let run_id = Uuid::new_v4();
        let job_id = Uuid::new_v4();
        let mut payload = input.payload;
        payload["run_id"] = serde_json::json!(run_id);
        sqlx::query(
            "INSERT INTO jobs \
             (id, owner_user_id, job_type, status, priority, idempotency_key, payload_json, max_attempts, available_at) \
             VALUES ($1, $2, 'backtest', 'QUEUED', 10, $3, $4, 3, now())",
        )
        .bind(job_id)
        .bind(owner)
        .bind(&input.idempotency_key)
        .bind(&payload)
        .execute(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        let run = input.run;
        let row = sqlx::query_as::<_, BacktestRunRow>(
            "INSERT INTO backtest_runs \
             (id, owner_user_id, job_id, strategy_id, strategy_version, dataset_version, \
              engine, engine_version, config_sha256, code_commit, random_seed, timezone, summary_json) \
             VALUES ($1, $2, $3, $4, $5, $6, 'nautilustrader', $7, $8, $9, $10, $11, $12) \
             RETURNING id, owner_user_id, job_id, strategy_id, strategy_version, \
                       dataset_version, engine, engine_version, config_sha256, code_commit, \
                       random_seed, timezone, status, summary_json, started_at, finished_at, created_at",
        )
        .bind(run_id)
        .bind(owner)
        .bind(job_id)
        .bind(&run.strategy_id)
        .bind(&run.strategy_version)
        .bind(&run.dataset_version)
        .bind(&run.engine_version)
        .bind(&run.config_sha256)
        .bind(&run.code_commit)
        .bind(run.random_seed)
        .bind(&run.timezone)
        .bind(&run.summary_json)
        .fetch_one(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(row)
    }

    /// List `actor`'s own runs, keyset-paginated on `(created_at, id)`.
    pub async fn list_page(
        &self,
        actor: &Actor,
        after: Option<&crate::http::pagination::Cursor>,
        limit: usize,
    ) -> TenancyResult<(Vec<BacktestRunRow>, Option<crate::http::pagination::Cursor>)> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let sql = match after {
            Some(_) => {
                "SELECT id, owner_user_id, job_id, strategy_id, strategy_version, \
                        dataset_version, engine, engine_version, config_sha256, \
                        code_commit, random_seed, timezone, status, summary_json, \
                        started_at, finished_at, created_at \
                 FROM backtest_runs WHERE (created_at, id) > ($1::timestamptz, $2::uuid) \
                 ORDER BY created_at, id LIMIT $3"
            }
            None => {
                "SELECT id, owner_user_id, job_id, strategy_id, strategy_version, \
                        dataset_version, engine, engine_version, config_sha256, \
                        code_commit, random_seed, timezone, status, summary_json, \
                        started_at, finished_at, created_at \
                 FROM backtest_runs ORDER BY created_at, id LIMIT $1"
            }
        };
        let mut q = sqlx::query_as::<_, BacktestRunRow>(sql);
        if let Some(c) = after {
            q = q
                .bind(c.k.clone())
                .bind(uuid::Uuid::parse_str(&c.i).map_err(|_| TenancyError::NotFound)?);
        }
        let rows = q
            .bind(limit as i64 + 1)
            .fetch_all(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(crate::repos::split_page(rows, limit, |r| {
            (r.created_at.to_rfc3339(), r.id.to_string())
        }))
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
