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
use job_queue::paper_execution::set_paper_transaction_timeouts;
use result_model::paper_parity::ParityReport;
use sqlx::FromRow;
use uuid::Uuid;

/// A `pending_targets` row as the actor may see it.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct PendingTargetRow {
    pub id: Uuid,
    pub owner_user_id: Uuid,
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
    pub source_kind: String,
    pub recommendation_run_id: Option<Uuid>,
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
    pub source_kind: String,
    pub recommendation_run_id: Option<Uuid>,
    pub status: String,
    pub executed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// The immutable announcement payload created with a terminal target.
///
/// This is intentionally a small, API-independent value object.  The Paper
/// session layer supplies the already-graded message and parity snapshot;
/// dispatch can then resume it without re-running settlement or recomputing a
/// potentially different report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaperSettlementAnnouncement {
    pub severity: String,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub parity_json: Option<serde_json::Value>,
    /// Structured reason retained on a SKIPPED target.  Keeping this in the
    /// same transaction as the status makes preflight denials auditable even
    /// when the dispatcher is unavailable.
    pub non_execution_reason: Option<serde_json::Value>,
}

/// One durable Paper notification intent.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct PaperSettlementOutboxRow {
    pub id: Uuid,
    pub pending_target_id: Uuid,
    pub owner_user_id: Uuid,
    pub severity: String,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub parity_json: Option<serde_json::Value>,
    pub attempts: i32,
    pub max_attempts: i32,
    pub available_at: DateTime<Utc>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub exhausted_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    /// Worker lease token. `None` means the row was produced by an inline
    /// settlement path and is not currently leased by the recovery scanner.
    pub claim_token: Option<Uuid>,
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

/// Operational state of the durable Paper settlement queue.  `ready` is
/// false when an undelivered obligation is exhausted or older than the
/// configured readiness budget.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct PaperSettlementBacklog {
    pub pending_count: i64,
    pub oldest_pending_age_secs: i64,
    pub failed_count: i64,
    pub exhausted_count: i64,
    pub ready: bool,
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
        let mut tx = pool.begin().await.map_err(TenancyError::from_sqlx)?;
        set_paper_transaction_timeouts(&mut tx)
            .await
            .map_err(TenancyError::from_sqlx)?;
        let rows = sqlx::query_as::<_, WorkerPendingTargetRow>(
            "SELECT id, account_id, owner_user_id, strategy_config_id, computed_on, \
                    effective_date, targets_json, dataset_version, dataset_version_id, \
                    dataset_manifest_sha256, non_execution_reason, source_kind, \
                    recommendation_run_id, status, executed_at, created_at \
             FROM pending_targets \
             WHERE status = 'PENDING' AND effective_date <= $1 \
             ORDER BY effective_date, account_id, id",
        )
        .bind(session_date)
        .fetch_all(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(rows)
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
             RETURNING id, owner_user_id, account_id, strategy_config_id, computed_on, effective_date, \
                       targets_json, dataset_version, dataset_version_id, dataset_manifest_sha256, \
                       non_execution_reason, source_kind, recommendation_run_id, \
                       status, executed_at, created_at",
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
                "SELECT id, owner_user_id, account_id, strategy_config_id, computed_on, effective_date, \
                        targets_json, dataset_version, dataset_version_id, dataset_manifest_sha256, \
                        non_execution_reason, source_kind, recommendation_run_id, \
                        status, executed_at, created_at \
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
            "SELECT id, owner_user_id, account_id, strategy_config_id, computed_on, effective_date, \
                    targets_json, dataset_version, dataset_version_id, dataset_manifest_sha256, \
                    non_execution_reason, source_kind, recommendation_run_id, \
                    status, executed_at, created_at \
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
            "SELECT id, owner_user_id, account_id, strategy_config_id, computed_on, effective_date, \
                    targets_json, dataset_version, dataset_version_id, dataset_manifest_sha256, \
                    non_execution_reason, source_kind, recommendation_run_id, \
                    status, executed_at, created_at \
             FROM pending_targets WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        crate::error::map_optional(row)
    }

    /// Settles a target after its session ran.  This compatibility helper
    /// retains the original repository API, but now uses the same atomic
    /// target-plus-outbox transaction as the Paper session seam.  Callers that
    /// know the full graded message should prefer
    /// [`Self::settle_with_announcement`].
    pub async fn settle(
        &self,
        actor: &Actor,
        id: Uuid,
        status: &str,
    ) -> TenancyResult<PendingTargetRow> {
        let (severity, kind) = if status == "EXECUTED" {
            ("INFO", "job")
        } else {
            ("WARNING", "alert")
        };
        let announcement = PaperSettlementAnnouncement {
            severity: severity.to_owned(),
            kind: kind.to_owned(),
            title: format!("Paper target {id} settled"),
            body: format!(
                "Paper target {id} settled as {status}; its completion notification is durable."
            ),
            parity_json: None,
            non_execution_reason: None,
        };
        self.settle_with_announcement(actor, id, status, &announcement)
            .await
            .map(|(target, _)| target)
    }

    /// Settle a pending target and enqueue its immutable announcement in one
    /// actor transaction.  A commit therefore exposes either both the
    /// terminal target and its retryable outbox intent, or neither.
    pub async fn settle_with_announcement(
        &self,
        actor: &Actor,
        id: Uuid,
        status: &str,
        announcement: &PaperSettlementAnnouncement,
    ) -> TenancyResult<(PendingTargetRow, PaperSettlementOutboxRow)> {
        let announcement = announcement.clone();
        self.settle_with_exact_parity(actor, id, status, move |_target, parity| {
            let mut announcement = announcement.clone();
            if announcement.parity_json.is_none() {
                announcement.parity_json =
                    parity.and_then(|snapshot| serde_json::to_value(snapshot).ok());
            }
            announcement
        })
        .await
        .map(|(target, outbox, _parity)| (target, outbox))
    }

    /// Lock one pending target, derive parity from its exact recommendation
    /// run in that same transaction, transition the target, and enqueue the
    /// outbox obligation atomically.  The callback only formats the immutable
    /// announcement after the locked parity snapshot is available; it cannot
    /// perform another database read or substitute a newer same-day run.
    pub async fn settle_with_exact_parity<F>(
        &self,
        actor: &Actor,
        id: Uuid,
        status: &str,
        build_announcement: F,
    ) -> TenancyResult<(
        PendingTargetRow,
        PaperSettlementOutboxRow,
        Option<ParityReport>,
    )>
    where
        F: FnOnce(&PendingTargetRow, Option<&ParityReport>) -> PaperSettlementAnnouncement,
    {
        if !matches!(status, "EXECUTED" | "SKIPPED") {
            return Err(TenancyError::InvalidState(
                "invalid terminal Paper target status".to_owned(),
            ));
        }
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let target = sqlx::query_as::<_, PendingTargetRow>(
            "SELECT id, owner_user_id, account_id, strategy_config_id, computed_on, effective_date, \
                    targets_json, dataset_version, dataset_version_id, dataset_manifest_sha256, \
                    non_execution_reason, source_kind, recommendation_run_id, \
                    status, executed_at, created_at \
             FROM pending_targets WHERE id = $1 FOR UPDATE",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        let target = crate::error::map_optional(target)?;
        if target.status != "PENDING" {
            return Err(TenancyError::NotFound);
        }

        // The target row is locked until COMMIT.  The exact-run query uses
        // target.recommendation_run_id and all immutable lineage columns;
        // it never searches by latest config/as_of.
        let parity = if status == "EXECUTED" {
            Some(crate::repos::parity::ParityRepo::report_for_target_tx(&mut tx, &target).await?)
        } else {
            None
        };
        let announcement = build_announcement(&target, parity.as_ref());
        let target = sqlx::query_as::<_, PendingTargetRow>(
            "UPDATE pending_targets SET status = $2, executed_at = pg_catalog.now(), \
                                         non_execution_reason = $3 \
             WHERE id = $1 AND status = 'PENDING' \
             RETURNING id, account_id, strategy_config_id, computed_on, effective_date, \
                       owner_user_id, targets_json, dataset_version, dataset_version_id, dataset_manifest_sha256, \
                       non_execution_reason, source_kind, recommendation_run_id, \
                       status, executed_at, created_at",
        )
        .bind(id)
        .bind(status)
        .bind(&announcement.non_execution_reason)
        .fetch_one(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        let outbox_id: Uuid = sqlx::query_scalar(
            "SELECT public.enqueue_paper_settlement_outbox(\
                 $1, $2, $3, $4, $5, $6)",
        )
        .bind(target.id)
        .bind(&announcement.severity)
        .bind(&announcement.kind)
        .bind(&announcement.title)
        .bind(&announcement.body)
        .bind(&announcement.parity_json)
        .fetch_one(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        let outbox = sqlx::query_as::<_, PaperSettlementOutboxRow>(
            "SELECT id, pending_target_id, owner_user_id, severity, kind, title, body, \
                    parity_json, attempts, max_attempts, available_at, delivered_at, \
                    exhausted_at, last_error, created_at, claim_token \
             FROM paper_settlement_outbox WHERE id = $1 \
             UNION ALL \
             SELECT archive.id, archive.pending_target_id, archive.owner_user_id, \
                    archive.severity, archive.kind, archive.title, archive.body, \
                    archive.parity_json, archive.attempts, archive.max_attempts, \
                    archive.created_at, archive.delivered_at, NULL::timestamptz, \
                    NULL::text, archive.created_at, NULL::uuid \
             FROM paper_settlement_outbox_archive archive \
             WHERE archive.id = $1 \
               AND NOT EXISTS (SELECT 1 FROM paper_settlement_outbox WHERE id = $1)",
        )
        .bind(outbox_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        let outbox = crate::error::map_optional(outbox)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok((target, outbox, parity))
    }

    /// Attach an announcement to a terminal target whose status was written
    /// by the trusted preflight function.  This is idempotent on the target
    /// key, so a process that was interrupted after preflight can safely
    /// resume without manufacturing another intent.
    pub async fn enqueue_terminal_announcement(
        &self,
        actor: &Actor,
        id: Uuid,
        announcement: &PaperSettlementAnnouncement,
    ) -> TenancyResult<(PendingTargetRow, PaperSettlementOutboxRow)> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let target = sqlx::query_as::<_, PendingTargetRow>(
            "SELECT id, owner_user_id, account_id, strategy_config_id, computed_on, effective_date, \
                    targets_json, dataset_version, dataset_version_id, dataset_manifest_sha256, \
                    non_execution_reason, source_kind, recommendation_run_id, \
                    status, executed_at, created_at \
             FROM pending_targets WHERE id = $1 FOR UPDATE",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        let target = crate::error::map_optional(target)?;
        if target.status == "PENDING" {
            return Err(TenancyError::InvalidState(
                "a terminal Paper announcement requires a settled target".to_owned(),
            ));
        }
        let outbox_id: Uuid = sqlx::query_scalar(
            "SELECT public.enqueue_paper_settlement_outbox(\
                 $1, $2, $3, $4, $5, $6)",
        )
        .bind(target.id)
        .bind(&announcement.severity)
        .bind(&announcement.kind)
        .bind(&announcement.title)
        .bind(&announcement.body)
        .bind(&announcement.parity_json)
        .fetch_one(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        let outbox = sqlx::query_as::<_, PaperSettlementOutboxRow>(
            "SELECT id, pending_target_id, owner_user_id, severity, kind, title, body, \
                    parity_json, attempts, max_attempts, available_at, delivered_at, \
                    exhausted_at, last_error, created_at, claim_token \
             FROM paper_settlement_outbox WHERE id = $1 \
             UNION ALL \
             SELECT archive.id, archive.pending_target_id, archive.owner_user_id, \
                    archive.severity, archive.kind, archive.title, archive.body, \
                    archive.parity_json, archive.attempts, archive.max_attempts, \
                    archive.created_at, archive.delivered_at, NULL::timestamptz, \
                    NULL::text, archive.created_at, NULL::uuid \
             FROM paper_settlement_outbox_archive archive \
             WHERE archive.id = $1 \
               AND NOT EXISTS (SELECT 1 FROM paper_settlement_outbox WHERE id = $1)",
        )
        .bind(outbox_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok((target, outbox))
    }

    /// Trusted worker scan for terminal Paper targets whose notification has
    /// not been durably dispatched yet.  This is deliberately independent of
    /// [`Self::due_worker`], which only scans executable PENDING targets.
    pub async fn due_announcements_worker(
        pool: &sqlx::PgPool,
        limit: i32,
    ) -> TenancyResult<Vec<PaperSettlementOutboxRow>> {
        let mut tx = pool.begin().await.map_err(TenancyError::from_sqlx)?;
        set_paper_transaction_timeouts(&mut tx)
            .await
            .map_err(TenancyError::from_sqlx)?;
        let rows = sqlx::query_as::<_, PaperSettlementOutboxRow>(
            "SELECT id, pending_target_id, owner_user_id, severity, kind, title, body, \
                    parity_json, attempts, max_attempts, available_at, delivered_at, \
                    exhausted_at, last_error, created_at, claim_token \
             FROM public.claim_paper_settlement_outbox($1, 60)",
        )
        .bind(limit.clamp(1, 1000))
        .fetch_all(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(rows)
    }

    /// Mark one announcement delivered after all recipient rows have been
    /// persisted.  Repeating this update is harmless and keeps dispatch
    /// idempotent if a worker crashes just after the first mark.
    pub async fn mark_announcement_delivered(
        &self,
        actor: &Actor,
        outbox_id: Uuid,
        claim_token: Option<Uuid>,
    ) -> TenancyResult<bool> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let changed: bool =
            sqlx::query_scalar("SELECT public.mark_paper_settlement_outbox_delivered($1, $2, $3)")
                .bind(outbox_id)
                .bind(actor_uuid(actor)?)
                .bind(claim_token)
                .fetch_one(&mut *tx)
                .await
                .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(changed)
    }

    /// Record a bounded retry delay while retaining the immutable intent.
    pub async fn record_announcement_failure(
        &self,
        actor: &Actor,
        outbox_id: Uuid,
        error: &str,
        claim_token: Option<Uuid>,
    ) -> TenancyResult<()> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        sqlx::query(
            "SELECT attempts, exhausted FROM public.fail_paper_settlement_outbox($1, $2, $3, $4)",
        )
        .bind(outbox_id)
        .bind(actor_uuid(actor)?)
        .bind(error)
        .bind(claim_token)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(())
    }

    /// Read the queue health snapshot through the worker-only definer
    /// function.  No serving role receives direct outbox UPDATE/DELETE.
    pub async fn settlement_backlog_worker(
        pool: &sqlx::PgPool,
    ) -> TenancyResult<PaperSettlementBacklog> {
        let row = sqlx::query_as::<_, PaperSettlementBacklog>(
            "SELECT pending_count, oldest_pending_age_secs, failed_count, \
                    exhausted_count, ready \
             FROM public.paper_settlement_outbox_stats()",
        )
        .fetch_one(pool)
        .await
        .map_err(TenancyError::from_sqlx)?;
        Ok(row)
    }

    /// Archive delivered obligations past the bounded retention window.  The
    /// database function inserts the immutable archive row before deleting an
    /// active row, so pruning never weakens the terminal-target invariant.
    pub async fn prune_settlement_outbox_worker(
        pool: &sqlx::PgPool,
        keep_seconds: i64,
        limit: i32,
    ) -> TenancyResult<i64> {
        let deleted: i64 =
            sqlx::query_scalar("SELECT public.prune_paper_settlement_outbox($1, $2)")
                .bind(keep_seconds)
                .bind(limit)
                .fetch_one(pool)
                .await
                .map_err(TenancyError::from_sqlx)?;
        Ok(deleted)
    }

    /// All of the account's targets, oldest first (audit view).
    pub async fn history(
        &self,
        actor: &Actor,
        account_id: Uuid,
    ) -> TenancyResult<Vec<PendingTargetRow>> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let rows = sqlx::query_as::<_, PendingTargetRow>(
            "SELECT id, owner_user_id, account_id, strategy_config_id, computed_on, effective_date, \
                    targets_json, dataset_version, dataset_version_id, dataset_manifest_sha256, \
                    non_execution_reason, source_kind, recommendation_run_id, \
                    status, executed_at, created_at \
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
