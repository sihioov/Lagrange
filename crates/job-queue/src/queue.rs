//! [`JobQueue`]: short leased claims, idempotent submission, heartbeats, and
//! guarded settling on the frozen T3 schema (design §6.8, NFR-REL-001..003).
//!
//! ## Claim / lease semantics
//!
//! * `claim_next` is ONE short transaction: `SELECT ... FOR UPDATE SKIP
//!   LOCKED LIMIT 1` (design §6.8 scan on `jobs_claim_idx`), then `RUNNING` +
//!   `attempt_count + 1` + `locked_by/locked_at`, then the immutable
//!   `RUNNING` attempt row. The transaction COMMITS before the worker does
//!   any work — a DB transaction is never held across work (NFR-REL-001:
//!   one job's failure must never block the API).
//! * The lease anchor is `jobs.locked_at` set at claim time (design §6.8:
//!   claim sets `locked_by/locked_at` in the same transaction) and extended
//!   by [`JobQueue::heartbeat`]; expiry = `locked_at + config.lease`. The
//!   frozen schema has no lease column, so the lease lives on the claim
//!   itself; `job_attempts.started_at` records the immutable claim instant.
//! * Settling is guarded (`status='RUNNING' AND locked_by=worker`): a zombie
//!   whose lease was swept can never double-settle; a cancel raced against a
//!   settle resolves deterministically (see [`JobQueue::settle_success`]).
//!
//! ## Idempotency
//!
//! `submit` INSERTs with `ON CONFLICT (owner_user_id, idempotency_key) DO
//! NOTHING`; a conflict (including concurrent duplicates, which block on the
//! speculative insertion until the winner commits) returns the EXISTING job
//! (FR-BT-008, AT-03). Unkeyed submits never conflict (NULL keys are
//! distinct under the unique index).

use crate::error::QueueError;
use crate::types::{
    AttemptOutcome, AuditActor, CancelResult, ClaimedJob, HeartbeatStatus, Job, JobStatus,
    SettleResult, SubmitJob, SweepReport,
};
use sqlx::postgres::PgPool;
use sqlx::types::Uuid;
use std::time::Duration;

/// All 18 `jobs` columns, snake_case, in table order.
const JOB_COLUMNS: &str = "id, owner_user_id, job_type, status, priority, idempotency_key, \
     payload_json, max_attempts, attempt_count, available_at, locked_by, locked_at, \
     started_at, finished_at, error_code, error_message, created_at, updated_at";

/// All 10 `job_attempts` columns.
const ATTEMPT_COLUMNS: &str = "id, job_id, attempt_no, outcome, claimed_by, error_code, \
     error_message, started_at, finished_at, created_at";

// sqlx 0.9's compile-time SQL audit accepts only static strings; these
// helpers assemble queries from the compile-time column consts above (never
// user input), wrapped in AssertSqlSafe — the documented audit escape hatch
// (same pattern as the migration-contract harness).

fn select_job(where_clause: &str) -> sqlx::AssertSqlSafe<String> {
    sqlx::AssertSqlSafe(format!("SELECT {JOB_COLUMNS} FROM jobs {where_clause}"))
}

fn update_job_returning(body: &str) -> sqlx::AssertSqlSafe<String> {
    sqlx::AssertSqlSafe(format!("UPDATE jobs {body} RETURNING {JOB_COLUMNS}"))
}

fn insert_job_returning(body: &str) -> sqlx::AssertSqlSafe<String> {
    sqlx::AssertSqlSafe(format!("INSERT INTO jobs {body} RETURNING {JOB_COLUMNS}"))
}

fn insert_attempt_returning() -> sqlx::AssertSqlSafe<String> {
    sqlx::AssertSqlSafe(format!(
        "INSERT INTO job_attempts (job_id, attempt_no, outcome, claimed_by, started_at) \
         VALUES ($1, $2, 'RUNNING', $3, now()) RETURNING {ATTEMPT_COLUMNS}"
    ))
}

/// Lease length and retry backoff (exponential base), fixed per deployment so
/// the sweeper and the claimer agree on what "expired" means.
#[derive(Debug, Clone, Copy)]
pub struct QueueConfig {
    /// Lease duration granted at claim; heartbeats extend it.
    pub lease: Duration,
    /// Backoff for the FIRST retry; attempt N waits `base * 2^(N-1)`.
    pub backoff_base: Duration,
}

impl Default for QueueConfig {
    fn default() -> Self {
        QueueConfig {
            lease: Duration::from_secs(60),
            backoff_base: Duration::from_secs(30),
        }
    }
}

/// PostgreSQL job queue client.
///
/// `pool` runs the queue statements under whatever role granted them
/// (production: `app` for submit/cancel, `worker` for claim/settle/sweep;
/// the role matrix is proven in the contract tests). `audit` is the
/// audit-writer pool REQUIRED for audited cancellation
/// ([`QueueError::AuditUnavailable`] otherwise).
#[derive(Clone)]
pub struct JobQueue {
    pool: PgPool,
    audit: Option<PgPool>,
    config: QueueConfig,
}

impl JobQueue {
    pub fn new(pool: PgPool, audit: Option<PgPool>, config: QueueConfig) -> JobQueue {
        JobQueue {
            pool,
            audit,
            config,
        }
    }

    pub fn config(&self) -> &QueueConfig {
        &self.config
    }

    // ------------------------------------------------------------------
    // Submission
    // ------------------------------------------------------------------

    /// Submit a job. A duplicate `idempotency_key` for the same owner returns
    /// the EXISTING job (any state, including terminal — AT-03 reuses the
    /// prior run instead of re-running it). Malformed input is rejected with
    /// [`QueueError::InvalidInput`] before any row lands.
    pub async fn submit(&self, job: SubmitJob) -> Result<Job, QueueError> {
        validate_submit(&job)?;
        let row: Option<Job> = sqlx::query_as(insert_job_returning(
            "(owner_user_id, job_type, status, priority, idempotency_key, payload_json,
             max_attempts, available_at)
             VALUES ($1, $2, 'QUEUED', $3, $4, $5, $6, COALESCE($7, now()))
             ON CONFLICT (owner_user_id, idempotency_key) DO NOTHING",
        ))
        .bind(job.owner_user_id)
        .bind(&job.job_type)
        .bind(job.priority)
        .bind(&job.idempotency_key)
        .bind(&job.payload)
        .bind(job.max_attempts)
        .bind(job.available_at)
        .fetch_optional(&self.pool)
        .await?;
        match row {
            Some(job) => Ok(job),
            None => {
                let key = job.idempotency_key.as_deref().ok_or_else(|| {
                    QueueError::Internal(
                        "unkeyed insert reported a conflict, which is impossible".into(),
                    )
                })?;
                let existing: Option<Job> = sqlx::query_as(select_job(
                    "WHERE owner_user_id = $1 AND idempotency_key = $2",
                ))
                .bind(job.owner_user_id)
                .bind(key)
                .fetch_optional(&self.pool)
                .await?;
                existing.ok_or_else(|| {
                    QueueError::Internal("idempotent conflict but no existing row".into())
                })
            }
        }
    }

    /// Read a job by id.
    pub async fn get_by_id(&self, id: Uuid) -> Result<Job, QueueError> {
        let job: Option<Job> = sqlx::query_as(select_job("WHERE id = $1"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        job.ok_or(QueueError::JobNotFound(id))
    }

    // ------------------------------------------------------------------
    // Claiming and leases
    // ------------------------------------------------------------------

    /// Claim the highest-priority eligible job (design §6.8 scan) or return
    /// `None` if the queue is empty. The claim transaction is SHORT: it
    /// commits before returning, so no transaction is held during work.
    pub async fn claim_next(&self, worker_id: &str) -> Result<Option<ClaimedJob>, QueueError> {
        if worker_id.trim().is_empty() {
            return Err(QueueError::InvalidInput(
                "worker_id must not be empty".into(),
            ));
        }
        let mut tx = self.pool.begin().await?;
        let candidate: Option<(Uuid,)> = sqlx::query_as(
            "SELECT id FROM jobs
             WHERE status = 'QUEUED' AND available_at <= now()
             ORDER BY priority DESC, created_at
             FOR UPDATE SKIP LOCKED
             LIMIT 1",
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some((job_id,)) = candidate else {
            tx.rollback().await?;
            return Ok(None);
        };

        let job: Job = sqlx::query_as(update_job_returning(
            "SET status = 'RUNNING',
                 attempt_count = attempt_count + 1,
                 locked_by = $1,
                 locked_at = now(),
                 started_at = COALESCE(started_at, now()),
                 updated_at = now()
             WHERE id = $2 AND status = 'QUEUED' AND available_at <= now()",
        ))
        .bind(worker_id)
        .bind(job_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| QueueError::Internal("claimed job vanished under row lock".into()))?;

        let attempt: crate::types::JobAttempt = sqlx::query_as(insert_attempt_returning())
                .bind(job.id)
                .bind(job.attempt_count)
                .bind(worker_id)
                .fetch_one(&mut *tx)
                .await?;

        let locked_at = job
            .locked_at
            .ok_or_else(|| QueueError::Internal("claim did not set locked_at".into()))?;
        let claim = ClaimedJob {
            lease_expires_at: locked_at + self.config.lease,
            worker_id: worker_id.to_string(),
            job,
            attempt,
        };
        tx.commit().await?;
        Ok(Some(claim))
    }

    /// Extend the lease (`locked_at = now()`). Reports
    /// [`HeartbeatStatus::LeaseLost`] if the lease already expired (the
    /// sweeper owns the attempt from then on) and
    /// [`HeartbeatStatus::Canceled`] if a cancel request won meanwhile.
    /// Fails with [`QueueError::Database`]/[`QueueError::JobNotFound`] only.
    pub async fn heartbeat(&self, claim: &ClaimedJob) -> Result<HeartbeatStatus, QueueError> {
        let rows = sqlx::query(
            "UPDATE jobs
             SET locked_at = now(), updated_at = now()
             WHERE id = $1 AND status = 'RUNNING' AND locked_by = $2
               AND locked_at > now() - make_interval(secs => $3)",
        )
        .bind(claim.job.id)
        .bind(&claim.worker_id)
        .bind(self.config.lease.as_secs_f64())
        .execute(&self.pool)
        .await?;
        if rows.rows_affected() == 1 {
            return Ok(HeartbeatStatus::Extended);
        }
        let status: Option<JobStatus> =
            sqlx::query_scalar("SELECT status FROM jobs WHERE id = $1")
                .bind(claim.job.id)
                .fetch_optional(&self.pool)
                .await?;
        match status {
            Some(JobStatus::Canceled) => Ok(HeartbeatStatus::Canceled),
            Some(_) => Ok(HeartbeatStatus::LeaseLost),
            None => Err(QueueError::JobNotFound(claim.job.id)),
        }
    }

    // ------------------------------------------------------------------
    // Settling
    // ------------------------------------------------------------------

    /// Settle a claim as SUCCEEDED. One short transaction: the guarded job
    /// update first (serializes against cancels and the sweeper), then the
    /// attempt. If a cancel won before the settle, the attempt is recorded
    /// `FAILED(error_code='canceled')` and the job stays CANCELED — canceled
    /// work is never exposed as a result (FR-BT-009). A claim whose lease was
    /// swept (job requeued) yields [`QueueError::StaleClaim`] and the zombie
    /// worker must not touch the attempt.
    pub async fn settle_success(&self, claim: &ClaimedJob) -> Result<SettleResult, QueueError> {
        let mut tx = self.pool.begin().await?;
        let job: Option<Job> = sqlx::query_as(update_job_returning(
            "SET status = 'SUCCEEDED', finished_at = now(), locked_by = NULL,
                 locked_at = NULL, updated_at = now()
             WHERE id = $1 AND status = 'RUNNING' AND locked_by = $2",
        ))
        .bind(claim.job.id)
        .bind(&claim.worker_id)
        .fetch_optional(&mut *tx)
        .await?;
        match job {
            Some(job) => {
                finalize_attempt(&mut tx, claim, AttemptOutcome::Succeeded, None, None).await?;
                tx.commit().await?;
                Ok(SettleResult::Committed(job))
            }
            None => {
                let result =
                    settle_canceled_race(&mut tx, claim, "job canceled by request").await;
                match result {
                    Ok(SettleResult::Canceled(job)) => {
                        tx.commit().await?;
                        Ok(SettleResult::Canceled(job))
                    }
                    Ok(other) => {
                        tx.rollback().await?;
                        Err(QueueError::Internal(format!(
                            "unexpected settle outcome {other:?}"
                        )))
                    }
                    Err(e) => {
                        tx.rollback().await?;
                        Err(e)
                    }
                }
            }
        }
    }
}

fn validate_submit(job: &SubmitJob) -> Result<(), QueueError> {
    if job.job_type.is_empty()
        || job.job_type.len() > 64
        || !job
            .job_type
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
    {
        return Err(QueueError::InvalidInput(
            "job_type must match [a-z0-9_-]{1,64}".into(),
        ));
    }
    if job.max_attempts < 1 {
        return Err(QueueError::InvalidInput(
            "max_attempts must be >= 1".into(),
        ));
    }
    if !job.payload.is_object() {
        return Err(QueueError::InvalidInput(
            "payload must be a JSON object".into(),
        ));
    }
    if let Some(key) = &job.idempotency_key {
        if key.is_empty() || key.len() > 128 {
            return Err(QueueError::InvalidInput(
                "idempotency_key must be 1..=128 chars".into(),
            ));
        }
    }
    Ok(())
}

/// Mark the claim's attempt terminal within `tx`. The attempt is guarded by
/// `outcome='RUNNING' AND claimed_by=worker`, so a stale/swept claim (attempt
/// already ORPHANED) cannot be double-written.
async fn finalize_attempt(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    claim: &ClaimedJob,
    outcome: AttemptOutcome,
    error_code: Option<&str>,
    error_message: Option<&str>,
) -> Result<(), QueueError> {
    let rows = sqlx::query(
        "UPDATE job_attempts
         SET outcome = $3, error_code = $4, error_message = $5, finished_at = now()
         WHERE job_id = $1 AND attempt_no = $2 AND outcome = 'RUNNING' AND claimed_by = $6",
    )
    .bind(claim.job.id)
    .bind(claim.attempt.attempt_no)
    .bind(outcome.as_str())
    .bind(error_code)
    .bind(error_message)
    .bind(&claim.worker_id)
    .execute(&mut **tx)
    .await?;
    if rows.rows_affected() != 1 {
        return Err(QueueError::StaleClaim(claim.job.id));
    }
    Ok(())
}

/// Resolve the "job is not RUNNING-owned" branch of a settle: if a cancel won
/// the race, record the attempt `FAILED(error_code='canceled')` and return
/// the CANCELED job; anything else is a stale claim.
async fn settle_canceled_race(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    claim: &ClaimedJob,
    reason: &str,
) -> Result<SettleResult, QueueError> {
    let status: Option<JobStatus> = sqlx::query_scalar("SELECT status FROM jobs WHERE id = $1")
        .bind(claim.job.id)
        .fetch_optional(&mut **tx)
        .await?;
    match status {
        Some(JobStatus::Canceled) => {
            finalize_attempt(tx, claim, AttemptOutcome::Failed, Some("canceled"), Some(reason))
                .await?;
            let job: Job = sqlx::query_as(select_job("WHERE id = $1"))
                .bind(claim.job.id)
                .fetch_one(&mut **tx)
                .await?;
            Ok(SettleResult::Canceled(job))
        }
        _ => Err(QueueError::StaleClaim(claim.job.id)),
    }
}

// Referenced by later increments (cancellation, retry classification,
// sweeper); declared now so the public API surface is stable.
#[allow(unused)]
impl JobQueue {
    /// Cooperative cancel request (audited). Implemented in the cancellation
    /// increment; present as a documented seam.
    pub async fn request_cancel(
        &self,
        _job_id: Uuid,
        _actor: &AuditActor,
    ) -> Result<CancelResult, QueueError> {
        Err(QueueError::Internal("request_cancel not implemented".into()))
    }

    /// Failure settle with retry classification. Implemented in the retry
    /// increment; present as a documented seam.
    pub async fn settle_failure(
        &self,
        _claim: &ClaimedJob,
        _class: crate::types::ErrorClass,
        _code: &str,
        _message: &str,
    ) -> Result<SettleResult, QueueError> {
        Err(QueueError::Internal(
            "settle_failure not implemented".into(),
        ))
    }

    /// Cooperative cancel checkpoint: `true` once a cancel request won for
    /// this job. Implemented in the cancellation increment.
    pub async fn check_canceled(&self, _job_id: Uuid) -> Result<bool, QueueError> {
        Err(QueueError::Internal(
            "check_canceled not implemented".into(),
        ))
    }

    /// Worker response to an observed cancel checkpoint: attempt recorded
    /// `FAILED(error_code='canceled')`, job stays CANCELED. Implemented in
    /// the cancellation increment.
    pub async fn settle_aborted(
        &self,
        _claim: &ClaimedJob,
        _reason: &str,
    ) -> Result<Job, QueueError> {
        Err(QueueError::Internal(
            "settle_aborted not implemented".into(),
        ))
    }

    /// Orphan-recovery sweep. Implemented in the sweeper increment.
    pub async fn sweep(&self) -> Result<SweepReport, QueueError> {
        Err(QueueError::Internal("sweep not implemented".into()))
    }
}
