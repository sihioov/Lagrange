//! Protected recovery for owner-beta recommendation claims.
//!
//! Owner-beta jobs have a second durable authority (`owner_beta_recommendation_runs`)
//! whose state must remain coherent with the queue.  They therefore cannot use
//! [`JobQueue::sweep`]: the generic sweeper deliberately excludes this job
//! type, and this module owns the queue/run reconciliation boundary instead.

use std::{fmt, time::Duration};

use chrono::{DateTime, NaiveDate, Utc};
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::{
    QueueError,
    error::database_error_class,
    owner_beta::OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE,
    queue::{JobQueue, backoff_seconds},
    types::{ErrorClass, JobStatus},
};

const OWNER_BETA_ATTEMPTS_EXHAUSTED: &str = "OWNER_BETA_ATTEMPTS_EXHAUSTED";
const CANCELED: &str = "CANCELED";
const JOB_ATTEMPTS_EXHAUSTED: &str = "attempts_exhausted";
const JOB_EXHAUSTED_MESSAGE: &str = "owner-beta worker crash exhausted retries";

/// Sanitized failures returned by owner-beta recovery.
///
/// No database/provider text crosses this boundary.  A commit failure is
/// intentionally separate because the caller cannot know whether PostgreSQL
/// committed the transition.
#[derive(Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OwnerBetaRecoveryError {
    #[error("owner-beta recovery database is temporarily unavailable")]
    DatabaseTransient,
    #[error("owner-beta recovery database contract failed")]
    DatabaseIntegrity,
    #[error("owner-beta recovery integrity check failed")]
    Integrity,
    #[error("owner-beta recovery commit outcome is unknown")]
    CommitUnknown,
}

impl fmt::Debug for OwnerBetaRecoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::DatabaseTransient => "DatabaseTransient",
            Self::DatabaseIntegrity => "DatabaseIntegrity",
            Self::Integrity => "Integrity",
            Self::CommitUnknown => "CommitUnknown",
        })
    }
}

impl OwnerBetaRecoveryError {
    pub const fn class(self) -> ErrorClass {
        match self {
            Self::DatabaseTransient => ErrorClass::Transient,
            Self::DatabaseIntegrity | Self::Integrity | Self::CommitUnknown => {
                ErrorClass::Integrity
            }
        }
    }
}

/// Counts produced by one protected recovery pass.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OwnerBetaRecoveryReport {
    /// Candidate rows found by the unlocked phase-1 scan.
    pub jobs_checked: usize,
    /// Latest RUNNING attempts transitioned to ORPHANED.
    pub attempts_orphaned: usize,
    /// Expired RUNNING jobs returned to QUEUED.
    pub jobs_requeued: usize,
    /// Jobs resolved FAILED after exhausting attempts.
    pub jobs_failed: usize,
    /// Runs mirrored to CANCELED.
    pub runs_canceled: usize,
    /// Already FAILED `attempts_exhausted` jobs whose PENDING runs were healed.
    pub runs_failed_healed: usize,
    /// Candidates that became ineligible or were already healed by a racer.
    pub jobs_skipped: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecoveryAction {
    Skipped,
    Requeued { attempt_orphaned: bool },
    Exhausted { attempt_orphaned: bool },
    Canceled { attempt_orphaned: bool },
    Healed,
}

#[derive(Debug, sqlx::FromRow)]
struct RecoveryJob {
    id: Uuid,
    owner_user_id: Uuid,
    job_type: String,
    status: JobStatus,
    max_attempts: i32,
    attempt_count: i32,
    locked_by: Option<String>,
    locked_at: Option<DateTime<Utc>>,
    started_at: Option<DateTime<Utc>>,
    error_code: Option<String>,
    payload_json: Value,
}

#[derive(Debug, sqlx::FromRow)]
struct RecoveryRun {
    id: Uuid,
    owner_user_id: Uuid,
    job_id: Uuid,
    strategy_config_id: Uuid,
    as_of: NaiveDate,
    status: String,
    factor_snapshot_sha256: Option<String>,
    target_snapshot_sha256: Option<String>,
    cash_weight: Option<String>,
    error_code: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct RecoveryAttempt {
    id: Uuid,
    attempt_no: i32,
    outcome: String,
}

/// Recover only protected owner-beta claims and their linked PENDING runs.
///
/// Phase one is an unlocked candidate read.  Every candidate then gets its
/// own short transaction opened through [`JobQueue::begin`].  The transaction
/// locks the queue job first and the linked owner-beta run second, preserving
/// the publication/cancellation lock order and making racing recoveries
/// idempotent.
pub async fn recover_owner_beta_claims(
    queue: &JobQueue,
) -> Result<OwnerBetaRecoveryReport, OwnerBetaRecoveryError> {
    let pool = queue.pool();
    let candidates: Vec<(Uuid,)> = sqlx::query_as(
        "SELECT j.id
         FROM jobs AS j
         WHERE j.job_type = $2
           AND (
                (j.status = 'RUNNING'
                 AND j.locked_at <= now() - make_interval(secs => $1))
             OR (j.status = 'CANCELED'
                 AND (j.locked_at IS NULL
                      OR j.locked_at <= now() - make_interval(secs => $1)))
             OR (j.status = 'FAILED'
                 AND j.error_code = 'attempts_exhausted'
                 AND EXISTS (
                     SELECT 1
                       FROM public.owner_beta_recommendation_runs AS r
                      WHERE r.job_id = j.id
                        AND r.owner_user_id = j.owner_user_id
                        AND r.status = 'PENDING'
                 ))
           )
         ORDER BY j.created_at, j.id",
    )
    .bind(queue.config().lease.as_secs_f64())
    .bind(OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE)
    .fetch_all(&pool)
    .await
    .map_err(database_error)?;

    let mut report = OwnerBetaRecoveryReport {
        jobs_checked: candidates.len(),
        ..OwnerBetaRecoveryReport::default()
    };
    for (job_id,) in candidates {
        match recover_one(queue, job_id).await? {
            RecoveryAction::Skipped => report.jobs_skipped += 1,
            RecoveryAction::Requeued { attempt_orphaned } => {
                report.jobs_requeued += 1;
                report.attempts_orphaned += usize::from(attempt_orphaned);
            }
            RecoveryAction::Exhausted { attempt_orphaned } => {
                report.jobs_failed += 1;
                report.attempts_orphaned += usize::from(attempt_orphaned);
            }
            RecoveryAction::Canceled { attempt_orphaned } => {
                report.runs_canceled += 1;
                report.attempts_orphaned += usize::from(attempt_orphaned);
            }
            RecoveryAction::Healed => report.runs_failed_healed += 1,
        }
    }
    Ok(report)
}

async fn recover_one(
    queue: &JobQueue,
    job_id: Uuid,
) -> Result<RecoveryAction, OwnerBetaRecoveryError> {
    let mut transaction = queue.begin().await.map_err(queue_error)?;
    let result = recover_one_in(
        &mut transaction,
        queue.config().lease,
        queue.config().backoff_base,
        job_id,
    )
    .await;
    match result {
        Ok(action) => match transaction.commit().await {
            Ok(()) => Ok(action),
            Err(_) => Err(OwnerBetaRecoveryError::CommitUnknown),
        },
        Err(error) => {
            let _ = transaction.rollback().await;
            Err(error)
        }
    }
}

async fn recover_one_in(
    transaction: &mut Transaction<'_, Postgres>,
    lease: Duration,
    backoff_base: Duration,
    job_id: Uuid,
) -> Result<RecoveryAction, OwnerBetaRecoveryError> {
    // This is intentionally the first lock in the transaction.  Publishers,
    // normal settlement, and cancellation all serialize through the job row.
    let job: Option<RecoveryJob> = sqlx::query_as(
        "SELECT id, owner_user_id, job_type, status, max_attempts, attempt_count,
                locked_by, locked_at, started_at, error_code, payload_json
         FROM public.jobs
         WHERE id = $1
           AND job_type = $2
           AND (
                (status = 'RUNNING'
                 AND locked_at <= now() - make_interval(secs => $3))
             OR (status = 'CANCELED'
                 AND (locked_at IS NULL
                      OR locked_at <= now() - make_interval(secs => $3)))
             OR (status = 'FAILED' AND error_code = 'attempts_exhausted'
                 AND EXISTS (
                     SELECT 1
                       FROM public.owner_beta_recommendation_runs AS r
                      WHERE r.job_id = jobs.id
                        AND r.owner_user_id = jobs.owner_user_id
                        AND r.status = 'PENDING'
                 ))
           )
         FOR UPDATE",
    )
    .bind(job_id)
    .bind(OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE)
    .bind(lease.as_secs_f64())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(database_error)?;
    let Some(job) = job else {
        return Ok(RecoveryAction::Skipped);
    };
    if job.job_type != OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE {
        return Ok(RecoveryAction::Skipped);
    }

    // The unique job_id key makes this the exact linked run.  Deliberately do
    // not filter owner here: a wrong-owner row must be observed and rejected,
    // not hidden as a missing run.
    let run: RecoveryRun = sqlx::query_as(
        "SELECT id, owner_user_id, job_id, strategy_config_id, as_of, status,
                factor_snapshot_sha256, target_snapshot_sha256,
                cash_weight::text AS cash_weight, error_code
         FROM public.owner_beta_recommendation_runs
         WHERE job_id = $1
         FOR UPDATE",
    )
    .bind(job.id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(database_error)?
    .ok_or(OwnerBetaRecoveryError::Integrity)?;

    validate_job_run_binding(&job, &run)?;
    let item_count: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM public.owner_beta_recommendation_items
         WHERE recommendation_run_id = $1",
    )
    .bind(run.id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(database_error)?;
    let no_result_outputs = run.factor_snapshot_sha256.is_none()
        && run.target_snapshot_sha256.is_none()
        && run.cash_weight.is_none()
        && item_count == 0;
    let pending_clean = no_result_outputs && run.error_code.is_none();

    if run.status != "PENDING" {
        // A second recovery may have committed immediately before this
        // transaction acquired the job lock.  Recognize only the exact
        // terminal mirror this module writes; every other non-PENDING state is
        // an integrity violation.
        let already_healed = no_result_outputs
            && match job.status {
                JobStatus::Canceled => {
                    run.status == "CANCELED" && run.error_code.as_deref() == Some(CANCELED)
                }
                JobStatus::Failed => {
                    run.status == "FAILED"
                        && run.error_code.as_deref() == Some(OWNER_BETA_ATTEMPTS_EXHAUSTED)
                }
                JobStatus::Running | JobStatus::Queued | JobStatus::Succeeded => false,
            };
        return if already_healed {
            Ok(RecoveryAction::Skipped)
        } else {
            Err(OwnerBetaRecoveryError::Integrity)
        };
    }
    if !pending_clean {
        return Err(OwnerBetaRecoveryError::Integrity);
    }

    match job.status {
        JobStatus::Running => recover_expired_running(transaction, backoff_base, &job, &run).await,
        JobStatus::Canceled => recover_canceled(transaction, &job, &run).await,
        JobStatus::Failed => recover_failed_run(transaction, &job, &run).await,
        JobStatus::Queued | JobStatus::Succeeded => Ok(RecoveryAction::Skipped),
    }
}

async fn recover_expired_running(
    transaction: &mut Transaction<'_, Postgres>,
    backoff_base: Duration,
    job: &RecoveryJob,
    run: &RecoveryRun,
) -> Result<RecoveryAction, OwnerBetaRecoveryError> {
    if job.locked_at.is_none()
        || job
            .locked_by
            .as_deref()
            .is_none_or(|worker| worker.trim().is_empty())
    {
        return Err(OwnerBetaRecoveryError::Integrity);
    }
    let latest = latest_attempt(transaction, job.id).await?;
    let Some(attempt) = latest else {
        return Err(OwnerBetaRecoveryError::Integrity);
    };
    if attempt.outcome != "RUNNING" {
        return Err(OwnerBetaRecoveryError::Integrity);
    }
    orphan_attempt(transaction, &attempt).await?;

    if job.attempt_count < job.max_attempts {
        let updated = sqlx::query(
            "UPDATE public.jobs
             SET status = 'QUEUED', locked_by = NULL, locked_at = NULL,
                 available_at = now() + make_interval(secs => $2),
                 error_code = NULL, error_message = NULL, updated_at = now()
             WHERE id = $1 AND job_type = $3 AND status = 'RUNNING'",
        )
        .bind(job.id)
        .bind(backoff_seconds(backoff_base, job.attempt_count))
        .bind(OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE)
        .execute(&mut **transaction)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() != 1 {
            return Err(OwnerBetaRecoveryError::Integrity);
        }
        // `run` is borrowed only to make the validation ordering explicit:
        // it was locked and proven PENDING/no-output before the queue update.
        let _ = run;
        Ok(RecoveryAction::Requeued {
            attempt_orphaned: true,
        })
    } else {
        let updated = sqlx::query(
            "UPDATE public.jobs
             SET status = 'FAILED', finished_at = now(),
                 error_code = 'attempts_exhausted',
                 error_message = $2, locked_by = NULL, locked_at = NULL,
                 updated_at = now()
             WHERE id = $1 AND job_type = $3 AND status = 'RUNNING'",
        )
        .bind(job.id)
        .bind(JOB_EXHAUSTED_MESSAGE)
        .bind(OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE)
        .execute(&mut **transaction)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() != 1 {
            return Err(OwnerBetaRecoveryError::Integrity);
        }
        mirror_failed_run(transaction, run, job).await?;
        Ok(RecoveryAction::Exhausted {
            attempt_orphaned: true,
        })
    }
}

async fn recover_canceled(
    transaction: &mut Transaction<'_, Postgres>,
    job: &RecoveryJob,
    run: &RecoveryRun,
) -> Result<RecoveryAction, OwnerBetaRecoveryError> {
    if job.locked_at.is_some()
        && job
            .locked_by
            .as_deref()
            .is_none_or(|worker| worker.trim().is_empty())
    {
        return Err(OwnerBetaRecoveryError::Integrity);
    }
    if job.locked_at.is_none() && job.locked_by.is_some() {
        return Err(OwnerBetaRecoveryError::Integrity);
    }
    let mut attempt_orphaned = false;
    if let Some(attempt) = latest_attempt(transaction, job.id).await?
        && attempt.outcome == "RUNNING"
    {
        // A NULL lease on a RUNNING attempt is not an expired claim and must
        // fail closed rather than allowing recovery to steal it.
        if job.locked_at.is_none()
            || job
                .locked_by
                .as_deref()
                .is_none_or(|worker| worker.trim().is_empty())
        {
            return Err(OwnerBetaRecoveryError::Integrity);
        }
        orphan_attempt(transaction, &attempt).await?;
        attempt_orphaned = true;
    }

    let updated = sqlx::query(
        "UPDATE public.jobs
         SET locked_by = NULL, locked_at = NULL, updated_at = now()
         WHERE id = $1 AND job_type = $2 AND status = 'CANCELED'",
    )
    .bind(job.id)
    .bind(OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE)
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    if updated.rows_affected() != 1 {
        return Err(OwnerBetaRecoveryError::Integrity);
    }
    mirror_canceled_run(transaction, run, job).await?;
    Ok(RecoveryAction::Canceled { attempt_orphaned })
}

async fn recover_failed_run(
    transaction: &mut Transaction<'_, Postgres>,
    job: &RecoveryJob,
    run: &RecoveryRun,
) -> Result<RecoveryAction, OwnerBetaRecoveryError> {
    if job.error_code.as_deref() != Some(JOB_ATTEMPTS_EXHAUSTED) {
        return Ok(RecoveryAction::Skipped);
    }
    mirror_failed_run(transaction, run, job).await?;
    Ok(RecoveryAction::Healed)
}

async fn latest_attempt(
    transaction: &mut Transaction<'_, Postgres>,
    job_id: Uuid,
) -> Result<Option<RecoveryAttempt>, OwnerBetaRecoveryError> {
    sqlx::query_as(
        "SELECT id, attempt_no, outcome
         FROM public.job_attempts
         WHERE job_id = $1
         ORDER BY attempt_no DESC
         LIMIT 1
         FOR UPDATE",
    )
    .bind(job_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(database_error)
}

async fn orphan_attempt(
    transaction: &mut Transaction<'_, Postgres>,
    attempt: &RecoveryAttempt,
) -> Result<(), OwnerBetaRecoveryError> {
    let updated = sqlx::query(
        "UPDATE public.job_attempts
         SET outcome = 'ORPHANED', finished_at = now()
         WHERE id = $1 AND attempt_no = $2 AND outcome = 'RUNNING'",
    )
    .bind(attempt.id)
    .bind(attempt.attempt_no)
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    if updated.rows_affected() != 1 {
        return Err(OwnerBetaRecoveryError::Integrity);
    }
    Ok(())
}

async fn mirror_failed_run(
    transaction: &mut Transaction<'_, Postgres>,
    run: &RecoveryRun,
    job: &RecoveryJob,
) -> Result<(), OwnerBetaRecoveryError> {
    let updated = sqlx::query(
        "UPDATE public.owner_beta_recommendation_runs
         SET status = 'FAILED', factor_snapshot_sha256 = NULL,
             target_snapshot_sha256 = NULL, cash_weight = NULL,
             error_code = $3,
             started_at = COALESCE($4, started_at, now()),
             finished_at = now(), updated_at = now()
         WHERE id = $1 AND owner_user_id = $2 AND job_id = $5
           AND status = 'PENDING'",
    )
    .bind(run.id)
    .bind(job.owner_user_id)
    .bind(OWNER_BETA_ATTEMPTS_EXHAUSTED)
    .bind(job.started_at)
    .bind(job.id)
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    if updated.rows_affected() != 1 {
        return Err(OwnerBetaRecoveryError::Integrity);
    }
    Ok(())
}

async fn mirror_canceled_run(
    transaction: &mut Transaction<'_, Postgres>,
    run: &RecoveryRun,
    job: &RecoveryJob,
) -> Result<(), OwnerBetaRecoveryError> {
    let updated = sqlx::query(
        "UPDATE public.owner_beta_recommendation_runs
         SET status = 'CANCELED', factor_snapshot_sha256 = NULL,
             target_snapshot_sha256 = NULL, cash_weight = NULL,
             error_code = $3,
             started_at = COALESCE($4, started_at),
             finished_at = now(), updated_at = now()
         WHERE id = $1 AND owner_user_id = $2 AND job_id = $5
           AND status = 'PENDING'",
    )
    .bind(run.id)
    .bind(job.owner_user_id)
    .bind(CANCELED)
    .bind(job.started_at)
    .bind(job.id)
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    if updated.rows_affected() != 1 {
        return Err(OwnerBetaRecoveryError::Integrity);
    }
    Ok(())
}

fn validate_job_run_binding(
    job: &RecoveryJob,
    run: &RecoveryRun,
) -> Result<(), OwnerBetaRecoveryError> {
    if job.id != run.job_id
        || job.owner_user_id != run.owner_user_id
        || run.status.is_empty()
        || payload_uuid(&job.payload_json, "run_id") != Some(run.id)
        || payload_uuid(&job.payload_json, "strategy_config_id") != Some(run.strategy_config_id)
        || payload_as_of(&job.payload_json) != Some(run.as_of)
    {
        return Err(OwnerBetaRecoveryError::Integrity);
    }
    Ok(())
}

fn payload_uuid(payload: &Value, key: &str) -> Option<Uuid> {
    payload.get(key)?.as_str()?.parse().ok()
}

fn payload_as_of(payload: &Value) -> Option<NaiveDate> {
    payload.get("as_of")?.as_str()?.parse().ok()
}

fn database_error(error: sqlx::Error) -> OwnerBetaRecoveryError {
    match database_error_class(&error) {
        ErrorClass::Transient => OwnerBetaRecoveryError::DatabaseTransient,
        ErrorClass::Input
        | ErrorClass::DataBlocked
        | ErrorClass::Integrity
        | ErrorClass::Determinism => OwnerBetaRecoveryError::DatabaseIntegrity,
    }
}

fn queue_error(error: QueueError) -> OwnerBetaRecoveryError {
    match error {
        QueueError::Database(error) => database_error(error),
        QueueError::CommitUnknown { .. } => OwnerBetaRecoveryError::CommitUnknown,
        QueueError::JobNotFound(_)
        | QueueError::StaleClaim(_)
        | QueueError::AlreadyTerminal(_, _)
        | QueueError::InvalidInput(_)
        | QueueError::AuditUnavailable
        | QueueError::Internal(_) => OwnerBetaRecoveryError::Integrity,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_errors_are_static_and_classified() {
        for error in [
            OwnerBetaRecoveryError::DatabaseTransient,
            OwnerBetaRecoveryError::DatabaseIntegrity,
            OwnerBetaRecoveryError::Integrity,
            OwnerBetaRecoveryError::CommitUnknown,
        ] {
            let text = error.to_string();
            assert!(!text.contains("SELECT"));
            assert!(!text.contains("sha256:"));
            assert!(!text.contains("owner_user_id"));
        }
        assert_eq!(
            OwnerBetaRecoveryError::CommitUnknown.class(),
            ErrorClass::Integrity
        );
    }

    #[test]
    fn recovery_lock_order_is_job_then_run() {
        let source = include_str!("recovery.rs");
        let recovery = source
            .split("async fn recover_one_in")
            .nth(1)
            .expect("recovery transaction");
        let job_lock = recovery.find("FROM public.jobs").expect("job lock query");
        let run_lock = recovery
            .find("FROM public.owner_beta_recommendation_runs")
            .expect("run lock query");
        assert!(job_lock < run_lock);
        assert!(recovery.contains("FOR UPDATE"));
    }
}
