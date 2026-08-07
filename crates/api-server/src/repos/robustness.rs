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

/// One planned child ready to persist: its lineage run id, its axis, and the
/// job-queue job actually submitted for it.
#[derive(Debug, Clone)]
pub struct NewChild {
    pub run_id: Uuid,
    pub axis_code: String,
    pub axis_json: serde_json::Value,
    pub job_id: Uuid,
}

#[derive(Debug, Clone)]
pub struct RobustnessRepo {
    pool: sqlx::PgPool,
}

impl RobustnessRepo {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
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

    /// Creates a fresh suite row owned by `actor`.
    pub async fn create_suite(
        &self,
        actor: &Actor,
        parent_run_id: Uuid,
    ) -> TenancyResult<RobustnessSuiteRow> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let row = sqlx::query_as::<_, RobustnessSuiteRow>(
            "INSERT INTO robustness_suites (owner_user_id, parent_run_id) VALUES ($1, $2) \
             RETURNING id, owner_user_id, parent_run_id, created_at",
        )
        .bind(actor_uuid(actor)?)
        .bind(parent_run_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(row)
    }

    /// Persists the planned children. Re-planning is a no-op per child: the
    /// deterministic lineage run id backs a `UNIQUE(suite_id, run_id)`
    /// constraint, so a repeated child is silently skipped rather than
    /// duplicated.
    pub async fn insert_children(
        &self,
        actor: &Actor,
        suite_id: Uuid,
        children: &[NewChild],
    ) -> TenancyResult<()> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let owner = actor_uuid(actor)?;
        for child in children {
            sqlx::query(
                "INSERT INTO robustness_children \
                 (suite_id, owner_user_id, run_id, job_id, axis_code, axis_json) \
                 VALUES ($1, $2, $3, $4, $5, $6) \
                 ON CONFLICT (suite_id, run_id) DO NOTHING",
            )
            .bind(suite_id)
            .bind(owner)
            .bind(child.run_id)
            .bind(child.job_id)
            .bind(&child.axis_code)
            .bind(&child.axis_json)
            .execute(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?;
        }
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(())
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
