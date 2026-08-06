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
    AttemptOutcome, AuditActor, CancelResult, ClaimedJob, ErrorClass, HeartbeatStatus, Job,
    JobStatus, SettleResult, SubmitJob, SweepReport,
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

// ------------------------------------------------------------------
// Cancellation (cooperative + audited) and retry classification
// ------------------------------------------------------------------

impl JobQueue {
    /// Request cancellation of a job. QUEUED/RUNNING -> CANCELED in one short
/// transaction, then an append-only `audit_logs` row (`job.canceled` with
/// actor and before/after status) written through the audit-writer pool.
/// Terminal jobs are returned as [`CancelResult::AlreadyTerminal`] with no
/// side effects; the request is refused with
/// [`QueueError::AuditUnavailable`] when no audit pool was configured —
/// cancellation is audited by contract, so an un-auditable cancel never
/// happens. The RUNNING worker is never interrupted: it observes the cancel
/// through [`JobQueue::check_canceled`] and aborts cooperatively.
pub async fn request_cancel(
    &self,
    job_id: Uuid,
    actor: &AuditActor,
) -> Result<CancelResult, QueueError> {
    let audit = self.audit.as_ref().ok_or(QueueError::AuditUnavailable)?;
    let mut tx = self.pool.begin().await?;
    let before: Option<JobStatus> =
        sqlx::query_scalar("SELECT status FROM jobs WHERE id = $1 FOR UPDATE")
            .bind(job_id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some(before) = before else {
        tx.rollback().await?;
        return Err(QueueError::JobNotFound(job_id));
    };
    if before.is_terminal() {
        tx.rollback().await?;
        return Ok(CancelResult::AlreadyTerminal(self.get_by_id(job_id).await?));
    }
    let job: Job = sqlx::query_as(update_job_returning(
        "SET status = 'CANCELED', finished_at = now(), updated_at = now()
         WHERE id = $1 AND status IN ('QUEUED', 'RUNNING')",
    ))
    .bind(job_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;

    sqlx::query(
        "INSERT INTO audit_logs
             (action, actor_role, actor_user_id, target_type, target_id,
              before_json, after_json, reason, correlation_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind("job.canceled")
    .bind(&actor.role)
    .bind(actor.user_id)
    .bind("job")
    .bind(job_id.to_string())
    .bind(serde_json::json!({ "status": before.as_str() }))
    .bind(serde_json::json!({ "status": "CANCELED" }))
    .bind("cooperative cancel request")
    .bind(&actor.correlation_id)
    .execute(audit)
    .await?;
    Ok(CancelResult::Canceled(job))
}

/// Cooperative cancel checkpoint: `true` once a cancel request won for this
/// job. Workers poll this between work steps (never mid-transaction).
pub async fn check_canceled(&self, job_id: Uuid) -> Result<bool, QueueError> {
    let status: Option<JobStatus> = sqlx::query_scalar("SELECT status FROM jobs WHERE id = $1")
        .bind(job_id)
        .fetch_optional(&self.pool)
        .await?;
    Ok(status == Some(JobStatus::Canceled))
}

/// Worker response to an observed cancel checkpoint: the attempt is recorded
/// `FAILED(error_code='canceled')` and the job stays CANCELED (canceled work
/// is never exposed as a result, FR-BT-009). Only valid once the job IS
/// CANCELED — a stale/foreign claim yields [`QueueError::StaleClaim`].
pub async fn settle_aborted(&self, claim: &ClaimedJob, reason: &str) -> Result<Job, QueueError> {
    let mut tx = self.pool.begin().await?;
    let status: Option<JobStatus> = sqlx::query_scalar("SELECT status FROM jobs WHERE id = $1")
        .bind(claim.job.id)
        .fetch_optional(&mut *tx)
        .await?;
    if status != Some(JobStatus::Canceled) {
        tx.rollback().await?;
        return Err(QueueError::StaleClaim(claim.job.id));
    }
    finalize_attempt(
        &mut tx,
        claim,
        AttemptOutcome::Failed,
        Some("canceled"),
        Some(reason),
    )
    .await?;
    tx.commit().await?;
    self.get_by_id(claim.job.id).await
}

/// Settle a claim as FAILED under the design §6.8 retry policy:
///
/// * [`ErrorClass::Transient`] with attempts remaining: requeue QUEUED with
///   exponential backoff (`available_at = now() + base * 2^(attempt-1)`).
/// * Anything else — input, blocked data, integrity, determinism — or
///   attempts exhausted: the job resolves FAILED immediately, carrying the
///   worker's error code/message. Non-retryable classes NEVER retry, no
///   matter how many attempts remain.
///
/// One short transaction (job + attempt), guarded like
/// [`JobQueue::settle_success`]; a cancel that won meanwhile resolves to
/// [`SettleResult::Canceled`].
pub async fn settle_failure(
    &self,
    claim: &ClaimedJob,
    class: ErrorClass,
    code: &str,
    message: &str,
) -> Result<SettleResult, QueueError> {
    let mut tx = self.pool.begin().await?;
    let requeued: Option<Job> = if class.retryable() {
        sqlx::query_as(update_job_returning(
            "SET status = 'QUEUED', locked_by = NULL, locked_at = NULL,
                 available_at = now() + make_interval(secs => $3),
                 error_code = NULL, error_message = NULL, updated_at = now()
             WHERE id = $1 AND status = 'RUNNING' AND locked_by = $2
               AND attempt_count < max_attempts",
        ))
        .bind(claim.job.id)
        .bind(&claim.worker_id)
        .bind(backoff_seconds(self.config.backoff_base, claim.job.attempt_count))
        .fetch_optional(&mut *tx)
        .await?
    } else {
        None
    };
    let job = match requeued {
        Some(job) => Some(job),
        None => {
        sqlx::query_as(update_job_returning(
                "SET status = 'FAILED', finished_at = now(), locked_by = NULL,
                     locked_at = NULL, error_code = $3, error_message = $4, updated_at = now()
                 WHERE id = $1 AND status = 'RUNNING' AND locked_by = $2",
            ))
            .bind(claim.job.id)
            .bind(&claim.worker_id)
            .bind(code)
            .bind(message)
            .fetch_optional(&mut *tx)
            .await?
        }
    };
    match job {
        Some(job) => {
            finalize_attempt(
                &mut tx,
                claim,
                AttemptOutcome::Failed,
                Some(code),
                Some(message),
            )
            .await?;
            tx.commit().await?;
            Ok(SettleResult::Committed(job))
        }
        None => {
            let result = settle_canceled_race(&mut tx, claim, "job canceled by request").await;
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

/// Exponential backoff in seconds for the attempt that just failed
/// (`attempt_count` is the failed attempt's number): `base * 2^(n-1)`,
/// capped at one hour so retries never schedule into the far future.
fn backoff_seconds(base: Duration, attempt_count: i32) -> f64 {
    let exponent = attempt_count.max(1) as u32 - 1;
    base.as_secs_f64() * 2f64.powi(exponent as i32).min(3600.0)
}

// ------------------------------------------------------------------
// Orphan recovery: the sweeper
// ------------------------------------------------------------------

impl JobQueue {
    /// One sweep pass over expired leases (NFR-REL-003 / design §6.8
    /// "워커 프로세스 비정상 종료: ORPHAN 감지 후 1회").
    ///
    /// Phase 1 reads candidate jobs whose lease anchor is stale
    /// (`status IN ('RUNNING','CANCELED') AND locked_at <= now() - lease`)
    /// WITHOUT taking locks, then each candidate is processed in its own
    /// SHORT transaction: the row is re-read `FOR UPDATE` and the expiry
    /// re-verified (a heartbeat that landed meanwhile wins), the `RUNNING`
    /// attempt is marked `ORPHANED` — an attempt-level outcome, NEVER a job
    /// status — and the job is either requeued exactly once (QUEUED with
    /// backoff; the next claim becomes a NEW attempt) or resolved `FAILED`
    /// (`error_code='attempts_exhausted'`) once `attempt_count` reached
    /// `max_attempts`. A CANCELED job's orphaned attempt is finalized with
    /// NO requeue — cancellation is honored even across worker death. All
    /// updates are guarded, so two racing sweepers or a settling worker can
    /// never orphan an attempt twice.
    pub async fn sweep(&self) -> Result<SweepReport, QueueError> {
        let expired: Vec<(Uuid,)> = sqlx::query_as(
            "SELECT id FROM jobs
             WHERE status IN ('RUNNING', 'CANCELED')
               AND locked_at <= now() - make_interval(secs => $1)",
        )
        .bind(self.config.lease.as_secs_f64())
        .fetch_all(&self.pool)
        .await?;

        let mut report = SweepReport::default();
        for (job_id,) in expired {
            report.jobs_checked += 1;
            self.sweep_job(job_id, &mut report).await?;
        }
        Ok(report)
    }

    /// Process one expired-lease job in its own short transaction.
    ///
    /// Expiry is re-verified with the SERVER clock (`locked_at <= now() -
    /// lease`) — never the client clock, whose skew against the database
    /// would decide liveness differently from the phase-1 scan.
    async fn sweep_job(&self, job_id: Uuid, report: &mut SweepReport) -> Result<(), QueueError> {
        let mut tx = self.pool.begin().await?;
        let row: Option<(i32, i32, String)> = sqlx::query_as(
            "SELECT attempt_count, max_attempts, status FROM jobs
             WHERE id = $1
               AND locked_at <= now() - make_interval(secs => $2)
             FOR UPDATE",
        )
        .bind(job_id)
        .bind(self.config.lease.as_secs_f64())
        .fetch_optional(&mut *tx)
        .await?;
        let Some((attempt_count, max_attempts, status)) = row else {
            tx.rollback().await?;
            return Ok(()); // a heartbeat won the race; the lease is live again
        };

        let latest: Option<(i32, String)> = sqlx::query_as(
            "SELECT attempt_no, outcome FROM job_attempts
             WHERE job_id = $1 ORDER BY attempt_no DESC LIMIT 1",
        )
        .bind(job_id)
        .fetch_optional(&mut *tx)
        .await?;
        match latest {
            Some((attempt_no, outcome)) if outcome == "RUNNING" => {
                let rows = sqlx::query(
                    "UPDATE job_attempts
                     SET outcome = 'ORPHANED', finished_at = now()
                     WHERE job_id = $1 AND attempt_no = $2 AND outcome = 'RUNNING'",
                )
                .bind(job_id)
                .bind(attempt_no)
                .execute(&mut *tx)
                .await?;
                if rows.rows_affected() != 1 {
                    tx.rollback().await?;
                    return Ok(()); // another sweeper won; nothing to do
                }
                report.attempts_orphaned += 1;
                if status == "CANCELED" {
                    // Cancellation is terminal: finalize the attempt, never requeue.
                    tx.commit().await?;
                    return Ok(());
                }
                if attempt_count >= max_attempts {
                    let rows = sqlx::query(
                        "UPDATE jobs
                         SET status = 'FAILED', finished_at = now(),
                             error_code = 'attempts_exhausted',
                             error_message = 'worker crash exhausted retries',
                             locked_by = NULL, locked_at = NULL, updated_at = now()
                         WHERE id = $1 AND status = 'RUNNING'",
                    )
                    .bind(job_id)
                    .execute(&mut *tx)
                    .await?;
                    if rows.rows_affected() == 1 {
                        report.jobs_failed += 1;
                    }
                } else {
                    let rows = sqlx::query(
                        "UPDATE jobs
                         SET status = 'QUEUED', locked_by = NULL, locked_at = NULL,
                             available_at = now() + make_interval(secs => $2),
                             error_code = NULL, error_message = NULL, updated_at = now()
                         WHERE id = $1 AND status = 'RUNNING'",
                    )
                    .bind(job_id)
                    .bind(backoff_seconds(self.config.backoff_base, attempt_count))
                    .execute(&mut *tx)
                    .await?;
                    if rows.rows_affected() == 1 {
                        report.jobs_requeued += 1;
                    }
                }
                tx.commit().await?;
            }
            Some((_no, outcome)) => {
                // Defensive finalize for an impossible-in-practice state (an
                // expired RUNNING job whose latest attempt is already
                // terminal — settle/claim transactions are atomic, so this
                // only occurs after manual tampering): mirror the attempt's
                // terminal outcome on the job instead of leaving it RUNNING.
                if outcome == "SUCCEEDED" {
                    sqlx::query(
                        "UPDATE jobs
                         SET status = 'SUCCEEDED', finished_at = now(),
                             locked_by = NULL, locked_at = NULL, updated_at = now()
                         WHERE id = $1 AND status = 'RUNNING'",
                    )
                    .bind(job_id)
                    .execute(&mut *tx)
                    .await?;
                } else if outcome == "FAILED" {
                    if attempt_count >= max_attempts {
                        sqlx::query(
                            "UPDATE jobs
                             SET status = 'FAILED', finished_at = now(),
                                 error_code = 'attempts_exhausted',
                                 error_message = 'attempt failed before settle completed',
                                 locked_by = NULL, locked_at = NULL, updated_at = now()
                             WHERE id = $1 AND status = 'RUNNING'",
                        )
                        .bind(job_id)
                        .execute(&mut *tx)
                        .await?;
                        report.jobs_failed += 1;
                    } else {
                        sqlx::query(
                            "UPDATE jobs
                             SET status = 'QUEUED', locked_by = NULL, locked_at = NULL,
                                 available_at = now() + make_interval(secs => $2),
                                 error_code = NULL, error_message = NULL, updated_at = now()
                             WHERE id = $1 AND status = 'RUNNING'",
                        )
                        .bind(job_id)
                        .bind(backoff_seconds(self.config.backoff_base, attempt_count))
                        .execute(&mut *tx)
                        .await?;
                        report.jobs_requeued += 1;
                    }
                }
                tx.commit().await?;
            }
            None => {
                // No attempt rows at all (manual tampering): leave the job;
                // nothing to finalize.
                tx.rollback().await?;
            }
        }
        Ok(())
    }
}
