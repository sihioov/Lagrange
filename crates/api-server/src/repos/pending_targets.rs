//! Actor-scoped repository over `pending_targets` (tenant table,
//! `owner_user_id`, FORCE RLS; plan Todo 31).
//!
//! A target is queued at close(T) for the session at `effective_date`
//! (T+1). `UNIQUE (account_id, effective_date)` makes queueing idempotent
//! at the schema level: a scheduler that recomputes the same close resolves
//! to the SAME row rather than queueing a second target for that session.
//! Targets are never deleted — an entitlement pause or a missed session
//! leaves a `PENDING` row rather than a hole.

use crate::actor_tx::{actor_uuid, begin_actor_tx};
use crate::error::{TenancyError, TenancyResult};
use auth::entitlement::Actor;
use chrono::{DateTime, NaiveDate, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// A `pending_targets` row as the actor may see it.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct PendingTargetRow {
    pub id: Uuid,
    pub account_id: Uuid,
    pub strategy_config_id: Uuid,
    pub computed_on: NaiveDate,
    pub effective_date: NaiveDate,
    pub targets_json: serde_json::Value,
    /// The dataset the target was computed from; NULL means unknown and a
    /// parity report degrades to NOT_COMPARABLE rather than claiming
    /// comparability it cannot prove.
    pub dataset_version: Option<String>,
    pub dataset_version_id: Option<Uuid>,
    pub dataset_manifest_sha256: Option<String>,
    pub non_execution_reason: Option<serde_json::Value>,
    pub status: String,
    pub executed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// A pending target as seen by the trusted Paper worker.
///
/// Unlike [`PendingTargetRow`], this cross-tenant scan includes the owner UUID
/// so the worker can establish the actor context before calling the normal
/// actor-scoped settlement seam. The worker role is intentionally required at
/// the call site; this type is not an alternative tenant API.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct WorkerPendingTargetRow {
    pub id: Uuid,
    pub account_id: Uuid,
    pub owner_user_id: Uuid,
    pub strategy_config_id: Uuid,
    pub computed_on: NaiveDate,
    pub effective_date: NaiveDate,
    pub targets_json: serde_json::Value,
    pub dataset_version: Option<String>,
    pub dataset_version_id: Option<Uuid>,
    pub dataset_manifest_sha256: Option<String>,
    pub non_execution_reason: Option<serde_json::Value>,
    pub status: String,
    pub executed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Input for queueing a target at close(T).
#[derive(Debug, Clone)]
pub struct NewPendingTarget {
    pub account_id: Uuid,
    pub strategy_config_id: Uuid,
    pub computed_on: NaiveDate,
    pub effective_date: NaiveDate,
    pub targets_json: serde_json::Value,
    pub dataset_version: Option<String>,
}

/// Typed repository over `pending_targets`.
#[derive(Debug, Clone)]
pub struct PendingTargetRepo {
    pool: sqlx::PgPool,
}

impl PendingTargetRepo {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    /// All due pending targets across tenants, for the trusted worker role.
    ///
    /// This query deliberately takes a pool rather than using the repository's
    /// actor-scoped app pool. The Paper daemon serves every tenant and the
    /// worker RLS policy is the explicit cross-tenant boundary; settlement
    /// still re-enters the actor-scoped API seam before any notification.
    pub async fn due_worker(
        pool: &sqlx::PgPool,
        session_date: NaiveDate,
    ) -> TenancyResult<Vec<WorkerPendingTargetRow>> {
        sqlx::query_as::<_, WorkerPendingTargetRow>(
            "SELECT id, account_id, owner_user_id, strategy_config_id, computed_on, \
                    effective_date, targets_json, dataset_version, dataset_version_id, \
                    dataset_manifest_sha256, non_execution_reason, status, executed_at, created_at \
             FROM pending_targets \
             WHERE status = 'PENDING' AND effective_date <= $1 \
             ORDER BY effective_date, account_id, id",
        )
        .bind(session_date)
        .fetch_all(pool)
        .await
        .map_err(TenancyError::from_sqlx)
    }

    /// Queues a target for its effective session. Re-queueing the same
    /// `(account, effective_date)` returns the EXISTING row untouched —
    /// recomputing a close never produces a duplicate target, and never
    /// silently overwrites one the runner may already be executing.
    pub async fn queue(
        &self,
        actor: &Actor,
        input: NewPendingTarget,
    ) -> TenancyResult<PendingTargetRow> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let row = sqlx::query_as::<_, PendingTargetRow>(
            "INSERT INTO pending_targets \
             (account_id, owner_user_id, strategy_config_id, computed_on, effective_date, \
              targets_json, dataset_version) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (account_id, effective_date) DO NOTHING \
             RETURNING id, account_id, strategy_config_id, computed_on, effective_date, \
                       targets_json, dataset_version, dataset_version_id, dataset_manifest_sha256, \
                       non_execution_reason, status, executed_at, created_at",
        )
        .bind(input.account_id)
        .bind(actor_uuid(actor)?)
        .bind(input.strategy_config_id)
        .bind(input.computed_on)
        .bind(input.effective_date)
        .bind(&input.targets_json)
        .bind(&input.dataset_version)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        let row = match row {
            Some(r) => r,
            None => sqlx::query_as::<_, PendingTargetRow>(
                "SELECT id, account_id, strategy_config_id, computed_on, effective_date, \
                        targets_json, dataset_version, dataset_version_id, dataset_manifest_sha256, \
                        non_execution_reason, status, executed_at, created_at \
                 FROM pending_targets WHERE account_id = $1 AND effective_date = $2",
            )
            .bind(input.account_id)
            .bind(input.effective_date)
            .fetch_one(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?,
        };
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(row)
    }

    /// The actor's targets still awaiting execution at or before
    /// `session_date` (the runner's claim scan).
    pub async fn due(
        &self,
        actor: &Actor,
        session_date: NaiveDate,
    ) -> TenancyResult<Vec<PendingTargetRow>> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let rows = sqlx::query_as::<_, PendingTargetRow>(
            "SELECT id, account_id, strategy_config_id, computed_on, effective_date, \
                    targets_json, dataset_version, dataset_version_id, dataset_manifest_sha256, \
                    non_execution_reason, status, executed_at, created_at \
             FROM pending_targets \
             WHERE status = 'PENDING' AND effective_date <= $1 \
             ORDER BY effective_date, account_id",
        )
        .bind(session_date)
        .fetch_all(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(rows)
    }

    /// One target by id (a foreign row => NotFound).
    pub async fn get(&self, actor: &Actor, id: Uuid) -> TenancyResult<PendingTargetRow> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let row = sqlx::query_as::<_, PendingTargetRow>(
            "SELECT id, account_id, strategy_config_id, computed_on, effective_date, \
                    targets_json, dataset_version, dataset_version_id, dataset_manifest_sha256, \
                    non_execution_reason, status, executed_at, created_at \
             FROM pending_targets WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        crate::error::map_optional(row)
    }

    /// Settles a target after its session ran. Guarded on `status =
    /// 'PENDING'`: a target already settled by another runner updates zero
    /// rows and is reported as [`TenancyError::NotFound`], so two runners
    /// racing the same session can never both claim it.
    pub async fn settle(
        &self,
        actor: &Actor,
        id: Uuid,
        status: &str,
    ) -> TenancyResult<PendingTargetRow> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let row = sqlx::query_as::<_, PendingTargetRow>(
            "UPDATE pending_targets SET status = $2, executed_at = now() \
             WHERE id = $1 AND status = 'PENDING' \
             RETURNING id, account_id, strategy_config_id, computed_on, effective_date, \
                       targets_json, dataset_version, dataset_version_id, dataset_manifest_sha256, \
                       non_execution_reason, status, executed_at, created_at",
        )
        .bind(id)
        .bind(status)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        crate::error::map_optional(row)
    }

    /// All of the account's targets, oldest first (audit view).
    pub async fn history(
        &self,
        actor: &Actor,
        account_id: Uuid,
    ) -> TenancyResult<Vec<PendingTargetRow>> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let rows = sqlx::query_as::<_, PendingTargetRow>(
            "SELECT id, account_id, strategy_config_id, computed_on, effective_date, \
                    targets_json, dataset_version, dataset_version_id, dataset_manifest_sha256, \
                    non_execution_reason, status, executed_at, created_at \
             FROM pending_targets WHERE account_id = $1 ORDER BY effective_date",
        )
        .bind(account_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(rows)
    }
}
