//! Actor-scoped repository over `robustness_suites`/`robustness_children`
//! (tenant tables, `owner_user_id`, FORCE RLS; plan Todo 29).
//!
//! Suite status is never stored redundantly: `suite_status` always joins
//! live `jobs.status`, so there is no cached status column to drift out of
//! sync with the queue.

use crate::actor_tx::{actor_uuid, begin_actor_tx};
use crate::error::{TenancyError, TenancyResult};
use auth::entitlement::Actor;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct RobustnessSuiteRow {
    pub id: Uuid,
    pub owner_user_id: Uuid,
    pub parent_run_id: Uuid,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RobustnessChildRow {
    pub id: Uuid,
    pub run_id: Uuid,
    pub job_id: Uuid,
    pub axis_code: String,
    pub axis_json: serde_json::Value,
    pub job_status: String,
}

/// One planned robustness child and its queue identity.  The repository
/// persists the suite, jobs, and child links as one capacity-checked unit.
#[derive(Debug, Clone)]
pub struct NewRobustnessJob {
    pub run_id: Uuid,
    pub axis_code: String,
    pub axis_json: serde_json::Value,
    pub payload: serde_json::Value,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct SubmittedRobustnessChild {
    pub run_id: Uuid,
    pub job_id: Uuid,
    pub axis_code: String,
    pub job_status: String,
}

#[derive(Debug, Clone)]
pub struct SubmittedRobustnessSuite {
    pub suite: RobustnessSuiteRow,
    pub children: Vec<SubmittedRobustnessChild>,
}

#[derive(Debug, thiserror::Error)]
pub enum SubmitRobustnessError {
    #[error(transparent)]
    Tenancy(#[from] TenancyError),
    #[error("per-owner job capacity exceeded")]
    CapacityExceeded,
    #[error("a robustness idempotency key resolves to incompatible input")]
    IdempotencyMismatch,
}

#[derive(Debug, Clone)]
pub struct RobustnessRepo {
    pool: sqlx::PgPool,
}

impl RobustnessRepo {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    /// Atomically create or resume a suite, enforcing the same global owner
    /// capacity lock as every other API job producer.  Capacity is reserved
    /// for the whole net-new fan-out before any job is inserted, so denial
    /// can never leave a partial batch or an empty suite behind.
    pub async fn submit_suite(
        &self,
        actor: &Actor,
        parent_run_id: Uuid,
        items: &[NewRobustnessJob],
        max_jobs_per_owner: u32,
    ) -> Result<SubmittedRobustnessSuite, SubmitRobustnessError> {
        let owner = actor_uuid(actor)?;
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        crate::repos::lock_owner_job_capacity(&mut tx, owner).await?;

        let suite = match sqlx::query_as::<_, RobustnessSuiteRow>(
            "SELECT id, owner_user_id, parent_run_id, created_at \
             FROM robustness_suites WHERE parent_run_id = $1 ORDER BY created_at, id LIMIT 1 \
             FOR UPDATE",
        )
        .bind(parent_run_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?
        {
            Some(row) => row,
            None => sqlx::query_as::<_, RobustnessSuiteRow>(
                "INSERT INTO robustness_suites (owner_user_id, parent_run_id) VALUES ($1, $2) \
                 RETURNING id, owner_user_id, parent_run_id, created_at",
            )
            .bind(owner)
            .bind(parent_run_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?,
        };

        let mut new_jobs = 0_i64;
        for item in items {
            let child_exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM robustness_children \
                 WHERE suite_id = $1 AND run_id = $2)",
            )
            .bind(suite.id)
            .bind(item.run_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?;
            if child_exists {
                continue;
            }
            let existing_job: Option<(String, serde_json::Value)> = sqlx::query_as(
                "SELECT job_type, payload_json FROM jobs \
                 WHERE owner_user_id = $1 AND idempotency_key = $2 FOR SHARE",
            )
            .bind(owner)
            .bind(&item.idempotency_key)
            .fetch_optional(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?;
            match existing_job {
                Some((job_type, payload))
                    if job_type == "robustness" && payload == item.payload => {}
                Some(_) => return Err(SubmitRobustnessError::IdempotencyMismatch),
                None => new_jobs += 1,
            }
        }

        let active: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM jobs WHERE owner_user_id = $1 \
             AND status IN ('QUEUED', 'RUNNING')",
        )
        .bind(owner)
        .fetch_one(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        if active + new_jobs > max_jobs_per_owner as i64 {
            return Err(SubmitRobustnessError::CapacityExceeded);
        }

        for item in items {
            let existing_child_job: Option<Uuid> = sqlx::query_scalar(
                "SELECT job_id FROM robustness_children WHERE suite_id = $1 AND run_id = $2",
            )
            .bind(suite.id)
            .bind(item.run_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?;
            if existing_child_job.is_some() {
                continue;
            }
            let existing_job_id: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM jobs WHERE owner_user_id = $1 AND idempotency_key = $2",
            )
            .bind(owner)
            .bind(&item.idempotency_key)
            .fetch_optional(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?;
            let job_id = match existing_job_id {
                Some(id) => id,
                None => {
                    let id = Uuid::new_v4();
                    sqlx::query(
                        "INSERT INTO jobs \
                         (id, owner_user_id, job_type, status, priority, idempotency_key, payload_json, max_attempts, available_at) \
                         VALUES ($1, $2, 'robustness', 'QUEUED', 5, $3, $4, 3, now())",
                    )
                    .bind(id)
                    .bind(owner)
                    .bind(&item.idempotency_key)
                    .bind(&item.payload)
                    .execute(&mut *tx)
                    .await
                    .map_err(TenancyError::from_sqlx)?;
                    id
                }
            };
            sqlx::query(
                "INSERT INTO robustness_children \
                 (suite_id, owner_user_id, run_id, job_id, axis_code, axis_json) \
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(suite.id)
            .bind(owner)
            .bind(item.run_id)
            .bind(job_id)
            .bind(&item.axis_code)
            .bind(&item.axis_json)
            .execute(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?;
        }

        let mut children = Vec::with_capacity(items.len());
        for item in items {
            children.push(
                sqlx::query_as::<_, SubmittedRobustnessChild>(
                    "SELECT c.run_id, c.job_id, c.axis_code, j.status AS job_status \
                     FROM robustness_children c JOIN jobs j ON j.id = c.job_id \
                     WHERE c.suite_id = $1 AND c.run_id = $2",
                )
                .bind(suite.id)
                .bind(item.run_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(TenancyError::from_sqlx)?,
            );
        }
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(SubmittedRobustnessSuite { suite, children })
    }

    /// The existing suite for this parent run, if any. Re-requesting a suite
    /// for the same parent must resolve to the identical suite, never a
    /// second row.
    pub async fn find_by_parent(
        &self,
        actor: &Actor,
        parent_run_id: Uuid,
    ) -> TenancyResult<Option<RobustnessSuiteRow>> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let row = sqlx::query_as::<_, RobustnessSuiteRow>(
            "SELECT id, owner_user_id, parent_run_id, created_at \
             FROM robustness_suites WHERE parent_run_id = $1",
        )
        .bind(parent_run_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(row)
    }

    /// One suite's children joined to their job's live status. A suite the
    /// actor cannot see (foreign or absent) is [`TenancyError::NotFound`].
    pub async fn suite_status(
        &self,
        actor: &Actor,
        suite_id: Uuid,
    ) -> TenancyResult<(RobustnessSuiteRow, Vec<RobustnessChildRow>)> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let suite = sqlx::query_as::<_, RobustnessSuiteRow>(
            "SELECT id, owner_user_id, parent_run_id, created_at \
             FROM robustness_suites WHERE id = $1",
        )
        .bind(suite_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        let Some(suite) = suite else {
            tx.rollback().await.map_err(TenancyError::from_sqlx)?;
            return Err(TenancyError::NotFound);
        };
        let rows: Vec<(Uuid, Uuid, Uuid, String, serde_json::Value, String)> = sqlx::query_as(
            "SELECT c.id, c.run_id, c.job_id, c.axis_code, c.axis_json, j.status \
             FROM robustness_children c JOIN jobs j ON j.id = c.job_id \
             WHERE c.suite_id = $1 ORDER BY c.created_at",
        )
        .bind(suite_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        let children = rows
            .into_iter()
            .map(
                |(id, run_id, job_id, axis_code, axis_json, job_status)| RobustnessChildRow {
                    id,
                    run_id,
                    job_id,
                    axis_code,
                    axis_json,
                    job_status,
                },
            )
            .collect();
        Ok((suite, children))
    }
}
