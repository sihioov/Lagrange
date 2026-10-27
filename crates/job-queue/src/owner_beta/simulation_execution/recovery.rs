//! Dedicated exact-type lease and terminal-mirror recovery.

use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::{
    owner_beta::simulation_execution::{
        OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE, VersionedSimulationJobInput,
    },
    queue::{JobQueue, backoff_seconds},
    types::{Job, JobStatus},
};

use super::publish::{
    RUN_CANCELED, RUN_FAILED, RUN_PENDING, SimulationPublicationError,
    SimulationPublicationFailure, SimulationRecoveryReport, commit_result, ensure_pending_run,
    lock_simulation_run, lock_simulation_run_for_job, map_database_error, required_dates,
    rollback_error, settle_run_terminal, validate_job_input, validate_raw_job_run_binding,
    validate_run_input,
};

#[derive(Debug, sqlx::FromRow)]
struct LatestAttempt {
    id: Uuid,
    job_id: Uuid,
    attempt_no: i32,
    outcome: String,
    claimed_by: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecoveryAction {
    Skipped,
    Requeued { attempt_orphaned: bool },
    Failed { attempt_orphaned: bool },
    Canceled { attempt_orphaned: bool },
    HealedFailedRun,
}

/// Recover only the dedicated simulation job type. Candidate discovery is an
/// unlocked read; every decision is rechecked after locking job then run.
pub async fn recover_simulation_claims(
    queue: &JobQueue,
) -> Result<SimulationRecoveryReport, SimulationPublicationError> {
    let candidates: Vec<(Uuid,)> = sqlx::query_as(
        "SELECT job.id
           FROM public.jobs AS job
           JOIN public.owner_beta_simulation_runs AS run
             ON run.job_id = job.id AND run.owner_user_id = job.owner_user_id
          WHERE job.job_type = $2
            AND run.status = 'PENDING'
            AND (
                (job.status = 'RUNNING'
                 AND job.locked_at <= pg_catalog.now() - make_interval(secs => $1))
             OR (job.status = 'CANCELED'
                 AND (job.locked_at IS NULL
                      OR job.locked_at <= pg_catalog.now() - make_interval(secs => $1)))
             OR (job.status = 'FAILED' AND job.error_code = 'attempts_exhausted')
            )
          ORDER BY job.created_at, job.id",
    )
    .bind(queue.config().lease.as_secs_f64())
    .bind(OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE)
    .fetch_all(&queue.pool())
    .await
    .map_err(map_database_error)?;

    let mut report = SimulationRecoveryReport {
        jobs_checked: candidates.len(),
        ..SimulationRecoveryReport::default()
    };
    for (job_id,) in candidates {
        match recover_one(queue, job_id).await? {
            RecoveryAction::Skipped => report.jobs_skipped += 1,
            RecoveryAction::Requeued { attempt_orphaned } => {
                report.jobs_requeued += 1;
                report.attempts_orphaned += usize::from(attempt_orphaned);
            }
            RecoveryAction::Failed { attempt_orphaned } => {
                report.jobs_failed += 1;
                report.attempts_orphaned += usize::from(attempt_orphaned);
            }
            RecoveryAction::Canceled { attempt_orphaned } => {
                report.runs_canceled += 1;
                report.attempts_orphaned += usize::from(attempt_orphaned);
            }
            RecoveryAction::HealedFailedRun => report.runs_failed_healed += 1,
        }
    }
    Ok(report)
}

async fn recover_one(
    queue: &JobQueue,
    job_id: Uuid,
) -> Result<RecoveryAction, SimulationPublicationError> {
    let mut transaction = queue
        .begin()
        .await
        .map_err(super::publish::map_queue_error)?;
    let job = match lock_candidate_job(&mut transaction, job_id, queue.config().lease.as_secs_f64())
        .await?
    {
        Some(job) => job,
        None => return commit_result(transaction, RecoveryAction::Skipped).await,
    };
    if job.job_type != OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE {
        return rollback_error(transaction, SimulationPublicationError::QueueIntegrity).await;
    }
    let serialized = serde_json::to_vec(&job.payload_json)
        .map_err(|_| SimulationPublicationError::QueueIntegrity)?;
    let input = match VersionedSimulationJobInput::from_json(&serialized) {
        Ok(input) => input,
        Err(_) => {
            let action = recover_invalid_payload(&mut transaction, &job).await?;
            return commit_result(transaction, action).await;
        }
    };
    validate_job_input(&job, &input)?;
    let run = lock_simulation_run(&mut transaction, &input, job.owner_user_id).await?;
    validate_run_input(&run, &input, &job)?;
    if run.status != RUN_PENDING
        || run.error_code.is_some()
        || run.engine_json.is_some()
        || run.published_attempt_id.is_some()
    {
        return rollback_error(transaction, SimulationPublicationError::Integrity).await;
    }
    let pair_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM public.owner_beta_simulation_result_pairs
              WHERE run_id = $1 AND owner_user_id = $2
         )",
    )
    .bind(run.id)
    .bind(run.owner_user_id)
    .fetch_one(&mut *transaction)
    .await
    .map_err(map_database_error)?;
    if pair_exists {
        return rollback_error(transaction, SimulationPublicationError::Integrity).await;
    }
    // Reconstructing `required_dates` here proves that recovery is operating
    // on the same fixed calendar schedule as publication; rights are checked
    // only before a new publication, never to prevent cancellation recovery.
    let dates = required_dates(&input)?;
    if dates.len() != run.session_count as usize {
        return rollback_error(transaction, SimulationPublicationError::Integrity).await;
    }

    let action = match job.status {
        JobStatus::Running => recover_expired_running(queue, &mut transaction, &job, &run).await?,
        JobStatus::Canceled => recover_canceled(&mut transaction, &job, &run).await?,
        JobStatus::Failed if job.error_code.as_deref() == Some("attempts_exhausted") => {
            heal_exhausted_run(&mut transaction, &job, &run).await?
        }
        JobStatus::Queued | JobStatus::Succeeded | JobStatus::Failed => RecoveryAction::Skipped,
    };
    commit_result(transaction, action).await
}

async fn recover_invalid_payload(
    transaction: &mut Transaction<'_, Postgres>,
    job: &Job,
) -> Result<RecoveryAction, SimulationPublicationError> {
    let run = lock_simulation_run_for_job(transaction, job.id, job.owner_user_id).await?;
    validate_raw_job_run_binding(job, &run)?;
    ensure_pending_run(transaction, &run).await?;

    match job.status {
        JobStatus::Running => recover_expired_invalid(transaction, job, &run).await,
        JobStatus::Canceled => recover_canceled(transaction, job, &run).await,
        JobStatus::Failed if job.error_code.as_deref() == Some("attempts_exhausted") => {
            heal_exhausted_run(transaction, job, &run).await
        }
        JobStatus::Queued | JobStatus::Succeeded | JobStatus::Failed => Ok(RecoveryAction::Skipped),
    }
}

async fn lock_candidate_job(
    transaction: &mut Transaction<'_, Postgres>,
    job_id: Uuid,
    lease_secs: f64,
) -> Result<Option<Job>, SimulationPublicationError> {
    sqlx::query_as::<_, Job>(
        "SELECT job.id, job.owner_user_id, job.job_type, job.status, job.priority,
                job.idempotency_key, job.payload_json, job.max_attempts,
                job.attempt_count, job.available_at, job.locked_by, job.locked_at,
                job.started_at, job.finished_at, job.error_code, job.error_message,
                job.created_at, job.updated_at
           FROM public.jobs AS job
          WHERE job.id = $1
            AND job.job_type = $2
            AND EXISTS (
                SELECT 1 FROM public.owner_beta_simulation_runs AS run
                 WHERE run.job_id = job.id
                   AND run.owner_user_id = job.owner_user_id
                   AND run.status = 'PENDING'
            )
            AND (
                (job.status = 'RUNNING'
                 AND job.locked_at <= pg_catalog.now() - make_interval(secs => $3))
             OR (job.status = 'CANCELED'
                 AND (job.locked_at IS NULL
                      OR job.locked_at <= pg_catalog.now() - make_interval(secs => $3)))
             OR (job.status = 'FAILED' AND job.error_code = 'attempts_exhausted')
            )
          FOR UPDATE OF job",
    )
    .bind(job_id)
    .bind(OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE)
    .bind(lease_secs)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(map_database_error)
}

async fn latest_attempt(
    transaction: &mut Transaction<'_, Postgres>,
    job_id: Uuid,
) -> Result<Option<LatestAttempt>, SimulationPublicationError> {
    sqlx::query_as::<_, LatestAttempt>(
        "SELECT id, job_id, attempt_no, outcome, claimed_by
           FROM public.job_attempts
          WHERE job_id = $1
          ORDER BY attempt_no DESC
          LIMIT 1
          FOR UPDATE",
    )
    .bind(job_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(map_database_error)
}

async fn recover_expired_running(
    queue: &JobQueue,
    transaction: &mut Transaction<'_, Postgres>,
    job: &Job,
    run: &super::publish::LockedSimulationRun,
) -> Result<RecoveryAction, SimulationPublicationError> {
    if job.locked_at.is_none()
        || job
            .locked_by
            .as_deref()
            .is_none_or(|worker| worker.trim().is_empty())
    {
        return Err(SimulationPublicationError::QueueIntegrity);
    }
    let Some(attempt) = latest_attempt(transaction, job.id).await? else {
        return Err(SimulationPublicationError::QueueIntegrity);
    };
    if attempt.job_id != job.id
        || attempt.attempt_no != job.attempt_count
        || attempt.outcome != "RUNNING"
        || attempt.claimed_by != job.locked_by
    {
        return Err(SimulationPublicationError::QueueIntegrity);
    }
    orphan_attempt(transaction, &attempt).await?;

    if job.attempt_count < job.max_attempts {
        let updated = sqlx::query(
            "UPDATE public.jobs
                SET status = 'QUEUED', locked_by = NULL, locked_at = NULL,
                    available_at = pg_catalog.now() + make_interval(secs => $2),
                    error_code = NULL, error_message = NULL, updated_at = pg_catalog.now()
              WHERE id = $1 AND job_type = $3 AND status = 'RUNNING'",
        )
        .bind(job.id)
        .bind(backoff_seconds(
            queue.config().backoff_base,
            job.attempt_count,
        ))
        .bind(OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE)
        .execute(&mut **transaction)
        .await
        .map_err(map_database_error)?;
        if updated.rows_affected() != 1 {
            return Err(SimulationPublicationError::QueueIntegrity);
        }
        Ok(RecoveryAction::Requeued {
            attempt_orphaned: true,
        })
    } else {
        let updated = sqlx::query(
            "UPDATE public.jobs
                SET status = 'FAILED', finished_at = pg_catalog.now(),
                    error_code = 'attempts_exhausted',
                    error_message = 'owner-beta simulation attempts exhausted',
                    locked_by = NULL, locked_at = NULL, updated_at = pg_catalog.now()
              WHERE id = $1 AND job_type = $2 AND status = 'RUNNING'",
        )
        .bind(job.id)
        .bind(OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE)
        .execute(&mut **transaction)
        .await
        .map_err(map_database_error)?;
        if updated.rows_affected() != 1 {
            return Err(SimulationPublicationError::QueueIntegrity);
        }
        settle_run_terminal(
            transaction,
            run,
            RUN_FAILED,
            "SIMULATION_ATTEMPTS_EXHAUSTED",
        )
        .await?;
        Ok(RecoveryAction::Failed {
            attempt_orphaned: true,
        })
    }
}

async fn recover_expired_invalid(
    transaction: &mut Transaction<'_, Postgres>,
    job: &Job,
    run: &super::publish::LockedSimulationRun,
) -> Result<RecoveryAction, SimulationPublicationError> {
    let Some(worker_id) = job.locked_by.as_deref() else {
        return Err(SimulationPublicationError::QueueIntegrity);
    };
    if job.locked_at.is_none() || worker_id.trim().is_empty() {
        return Err(SimulationPublicationError::QueueIntegrity);
    }
    let Some(attempt) = latest_attempt(transaction, job.id).await? else {
        return Err(SimulationPublicationError::QueueIntegrity);
    };
    if attempt.job_id != job.id
        || attempt.attempt_no != job.attempt_count
        || attempt.outcome != "RUNNING"
        || attempt.claimed_by.as_deref() != Some(worker_id)
    {
        return Err(SimulationPublicationError::QueueIntegrity);
    }
    orphan_attempt(transaction, &attempt).await?;

    let updated = sqlx::query(
        "UPDATE public.jobs
            SET status = 'FAILED', finished_at = pg_catalog.now(),
                error_code = 'SIMULATION_INPUT_INVALID',
                error_message = 'owner-beta simulation input rejected',
                locked_by = NULL, locked_at = NULL, updated_at = pg_catalog.now()
          WHERE id = $1 AND owner_user_id = $2 AND job_type = $3
            AND status = 'RUNNING' AND attempt_count = $4 AND locked_by = $5",
    )
    .bind(job.id)
    .bind(job.owner_user_id)
    .bind(OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE)
    .bind(job.attempt_count)
    .bind(worker_id)
    .execute(&mut **transaction)
    .await
    .map_err(map_database_error)?;
    if updated.rows_affected() != 1 {
        return Err(SimulationPublicationError::QueueIntegrity);
    }
    settle_run_terminal(
        transaction,
        run,
        RUN_FAILED,
        SimulationPublicationFailure::InputInvalid.code(),
    )
    .await?;
    Ok(RecoveryAction::Failed {
        attempt_orphaned: true,
    })
}

async fn recover_canceled(
    transaction: &mut Transaction<'_, Postgres>,
    job: &Job,
    run: &super::publish::LockedSimulationRun,
) -> Result<RecoveryAction, SimulationPublicationError> {
    if job.locked_at.is_some() != job.locked_by.is_some()
        || job
            .locked_by
            .as_deref()
            .is_some_and(|worker| worker.trim().is_empty())
    {
        return Err(SimulationPublicationError::QueueIntegrity);
    }
    // `lock_candidate_job` already rechecked cancellation lease expiry with
    // the database clock after acquiring the job row lock.
    let latest = latest_attempt(transaction, job.id).await?;
    let attempt_orphaned = match latest {
        Some(attempt) if attempt.attempt_no == job.attempt_count && attempt.job_id == job.id => {
            match attempt.outcome.as_str() {
                "RUNNING" => {
                    if job.locked_at.is_none()
                        || attempt.claimed_by.as_deref() != job.locked_by.as_deref()
                    {
                        return Err(SimulationPublicationError::QueueIntegrity);
                    }
                    orphan_attempt(transaction, &attempt).await?;
                    true
                }
                "FAILED" | "ORPHANED" => false,
                _ => return Err(SimulationPublicationError::QueueIntegrity),
            }
        }
        None if job.attempt_count == 0 && job.locked_at.is_none() => false,
        _ => return Err(SimulationPublicationError::QueueIntegrity),
    };
    settle_run_terminal(transaction, run, RUN_CANCELED, "CANCELED").await?;
    Ok(RecoveryAction::Canceled { attempt_orphaned })
}

async fn heal_exhausted_run(
    transaction: &mut Transaction<'_, Postgres>,
    job: &Job,
    run: &super::publish::LockedSimulationRun,
) -> Result<RecoveryAction, SimulationPublicationError> {
    let Some(attempt) = latest_attempt(transaction, job.id).await? else {
        return Err(SimulationPublicationError::QueueIntegrity);
    };
    if attempt.job_id != job.id
        || attempt.attempt_no != job.attempt_count
        || !matches!(attempt.outcome.as_str(), "FAILED" | "ORPHANED")
        || job.locked_at.is_some()
        || job.locked_by.is_some()
    {
        return Err(SimulationPublicationError::QueueIntegrity);
    }
    settle_run_terminal(
        transaction,
        run,
        RUN_FAILED,
        "SIMULATION_ATTEMPTS_EXHAUSTED",
    )
    .await?;
    Ok(RecoveryAction::HealedFailedRun)
}

async fn orphan_attempt(
    transaction: &mut Transaction<'_, Postgres>,
    attempt: &LatestAttempt,
) -> Result<(), SimulationPublicationError> {
    let updated = sqlx::query(
        "UPDATE public.job_attempts
            SET outcome = 'ORPHANED', finished_at = pg_catalog.now()
          WHERE id = $1 AND job_id = $2 AND attempt_no = $3 AND outcome = 'RUNNING'",
    )
    .bind(attempt.id)
    .bind(attempt.job_id)
    .bind(attempt.attempt_no)
    .execute(&mut **transaction)
    .await
    .map_err(map_database_error)?;
    if updated.rows_affected() != 1 {
        return Err(SimulationPublicationError::QueueIntegrity);
    }
    Ok(())
}
