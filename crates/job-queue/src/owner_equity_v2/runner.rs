//! Bounded lease/heartbeat wrapper for the Owner Equity V2 worker.

use std::time::Duration;

use serde_json::Value;
use sqlx::PgPool;
use tokio::time::MissedTickBehavior;
use uuid::Uuid;

use crate::{HeartbeatStatus, JobQueue, SweepReport};

use super::{
    OWNER_EQUITY_V2_JOB_TYPE, OwnerEquityJobAction, OwnerEquityJobPayload, OwnerEquityRunOutcome,
    OwnerEquityWorkerAdapter, begin_worker_tx, process_owner_equity_claim,
};

const CRASH_EXHAUSTED: &str = "WORKER_CRASH_ATTEMPTS_EXHAUSTED";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnerEquityRunnerConfig {
    heartbeat_interval: Duration,
    lease: Duration,
    work_timeout: Duration,
}

impl OwnerEquityRunnerConfig {
    pub fn new(
        heartbeat_interval: Duration,
        lease: Duration,
        work_timeout: Duration,
    ) -> Result<Self, OwnerEquityRunnerError> {
        if heartbeat_interval.is_zero()
            || lease.is_zero()
            || work_timeout.is_zero()
            || heartbeat_interval >= lease
        {
            return Err(OwnerEquityRunnerError::ConfigInvalid);
        }
        Ok(Self {
            heartbeat_interval,
            lease,
            work_timeout,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OwnerEquityRunnerError {
    #[error("owner equity runner configuration is invalid")]
    ConfigInvalid,
    #[error("owner equity queue is unavailable")]
    QueueUnavailable,
    #[error("owner equity lease was lost")]
    LeaseLost,
    #[error("owner equity work timed out")]
    WorkTimedOut,
    #[error("owner equity worker failed")]
    WorkerFailed,
}

pub async fn run_owner_equity_runner_once<A: OwnerEquityWorkerAdapter>(
    pool: &PgPool,
    queue: &JobQueue,
    worker_id: &str,
    adapter: &A,
    config: OwnerEquityRunnerConfig,
) -> Result<OwnerEquityRunOutcome, OwnerEquityRunnerError> {
    if config.lease != queue.config().lease {
        return Err(OwnerEquityRunnerError::ConfigInvalid);
    }
    let Some(claim) = queue
        .claim_next_for(worker_id, OWNER_EQUITY_V2_JOB_TYPE)
        .await
        .map_err(|_| OwnerEquityRunnerError::QueueUnavailable)?
    else {
        return Ok(OwnerEquityRunOutcome::Idle);
    };
    let work = process_owner_equity_claim(pool, queue, &claim, adapter);
    tokio::pin!(work);
    let timeout = tokio::time::sleep(config.work_timeout);
    tokio::pin!(timeout);
    let mut heartbeat = tokio::time::interval(config.heartbeat_interval);
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);
    heartbeat.tick().await;
    loop {
        tokio::select! {
            result = &mut work => {
                return result.map_err(|_| OwnerEquityRunnerError::WorkerFailed);
            }
            _ = &mut timeout => return Err(OwnerEquityRunnerError::WorkTimedOut),
            _ = heartbeat.tick() => {
                match queue.heartbeat(&claim).await {
                    Ok(HeartbeatStatus::Extended) => {}
                    Ok(HeartbeatStatus::Canceled) => return Ok(OwnerEquityRunOutcome::Canceled),
                    Ok(HeartbeatStatus::LeaseLost) => return Err(OwnerEquityRunnerError::LeaseLost),
                    Err(_) => return Err(OwnerEquityRunnerError::QueueUnavailable),
                }
            }
        }
    }
}

/// The queue's guarded stale-claim recovery is bounded by configured lease,
/// attempts, and backoff. The queue remains the claim authority; the second
/// pass idempotently mirrors exhausted Add/Retry jobs into membership state.
pub async fn recover_owner_equity_claims(
    queue: &JobQueue,
) -> Result<SweepReport, OwnerEquityRunnerError> {
    let report = queue
        .sweep()
        .await
        .map_err(|_| OwnerEquityRunnerError::QueueUnavailable)?;
    reconcile_exhausted_memberships(queue).await?;
    Ok(report)
}

async fn reconcile_exhausted_memberships(queue: &JobQueue) -> Result<(), OwnerEquityRunnerError> {
    let pool = queue.pool();
    let candidates: Vec<(Uuid, Uuid, Value)> = sqlx::query_as(
        "SELECT j.id, j.owner_user_id, j.payload_json
         FROM public.jobs AS j
         WHERE j.job_type = $1
           AND j.status = 'FAILED'
           AND j.error_code = 'attempts_exhausted'
           AND EXISTS (
               SELECT 1 FROM public.owner_equity_memberships AS m
               WHERE m.owner_user_id = j.owner_user_id
                 AND m.id::text = j.payload_json ->> 'membership_id'
                 AND m.instrument_id = j.payload_json ->> 'instrument_id'
                 AND m.state IN ('VALIDATING', 'BACKFILLING', 'MATERIALIZING')
           )
         ORDER BY j.created_at, j.id",
    )
    .bind(OWNER_EQUITY_V2_JOB_TYPE)
    .fetch_all(&pool)
    .await
    .map_err(|_| OwnerEquityRunnerError::QueueUnavailable)?;
    for (job_id, owner, value) in candidates {
        let payload: OwnerEquityJobPayload =
            serde_json::from_value(value).map_err(|_| OwnerEquityRunnerError::WorkerFailed)?;
        payload
            .validate()
            .map_err(|_| OwnerEquityRunnerError::WorkerFailed)?;
        if !matches!(
            payload.action,
            OwnerEquityJobAction::Add | OwnerEquityJobAction::Retry
        ) {
            continue;
        }
        let mut transaction = begin_worker_tx(&pool, owner)
            .await
            .map_err(|_| OwnerEquityRunnerError::QueueUnavailable)?;
        let job_is_exhausted: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                 SELECT 1 FROM public.jobs
                 WHERE id = $1 AND owner_user_id = $2
                   AND job_type = $3 AND status = 'FAILED'
                   AND error_code = 'attempts_exhausted'
             )",
        )
        .bind(job_id)
        .bind(owner)
        .bind(OWNER_EQUITY_V2_JOB_TYPE)
        .fetch_one(&mut *transaction)
        .await
        .map_err(|_| OwnerEquityRunnerError::QueueUnavailable)?;
        if job_is_exhausted {
            sqlx::query(
                "UPDATE public.owner_equity_memberships
                 SET state = 'FAILED', transition_actor_user_id = $2,
                     transition_code_commit = $4,
                     transition_entitlement_sha256 = $5,
                     error_code = $6, error_retryable = true,
                     disabled_at = NULL, updated_at = now()
                 WHERE id = $1 AND owner_user_id = $2 AND instrument_id = $3
                   AND state IN ('VALIDATING', 'BACKFILLING', 'MATERIALIZING')",
            )
            .bind(payload.membership_id)
            .bind(owner)
            .bind(&payload.instrument_id)
            .bind(&payload.code_commit)
            .bind(&payload.entitlement_sha256)
            .bind(CRASH_EXHAUSTED)
            .execute(&mut *transaction)
            .await
            .map_err(|_| OwnerEquityRunnerError::QueueUnavailable)?;
        }
        transaction
            .commit()
            .await
            .map_err(|_| OwnerEquityRunnerError::QueueUnavailable)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_config_is_positive_ordered_and_bounded() {
        assert_eq!(
            OwnerEquityRunnerConfig::new(
                Duration::ZERO,
                Duration::from_secs(60),
                Duration::from_secs(600),
            ),
            Err(OwnerEquityRunnerError::ConfigInvalid)
        );
        assert_eq!(
            OwnerEquityRunnerConfig::new(
                Duration::from_secs(60),
                Duration::from_secs(60),
                Duration::from_secs(600),
            ),
            Err(OwnerEquityRunnerError::ConfigInvalid)
        );
        assert!(
            OwnerEquityRunnerConfig::new(
                Duration::from_secs(10),
                Duration::from_secs(60),
                Duration::from_secs(600),
            )
            .is_ok()
        );
    }

    #[test]
    fn recovery_reconciles_only_exhausted_add_retry_lifecycles() {
        let source = include_str!("runner.rs");
        let production = source.split("#[cfg(test)]").next().unwrap();
        assert!(production.contains("j.error_code = 'attempts_exhausted'"));
        assert!(production.contains("OwnerEquityJobAction::Add | OwnerEquityJobAction::Retry"));
        assert!(production.contains("state IN ('VALIDATING', 'BACKFILLING', 'MATERIALIZING')"));
        assert!(production.contains("error_retryable = true"));
        assert!(!production.contains("state = 'READY'"));
    }
}
