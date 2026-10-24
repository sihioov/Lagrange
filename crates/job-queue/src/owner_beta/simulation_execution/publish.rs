//! Short, job-first transactions for owner-beta simulation publication.

use chrono::NaiveDate;
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    QueueError,
    error::{database_error_class, queue_error_class},
    owner_beta::{
        simulation::{
            APPROVED_ACTION_MANIFEST_SHA256, APPROVED_ARTIFACT_MANIFEST_SHA256,
            APPROVED_BARS_SHA256, APPROVED_CANDIDATE_CONTENT_SHA256, APPROVED_CONTRACT_SHA256,
            APPROVED_END_DATE, APPROVED_REGISTRY_SHA256, APPROVED_STAGE5_MANIFEST_SHA256,
            APPROVED_START_DATE, EngineIdentity,
        },
        simulation_execution::{
            CheckedSimulationPair, OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE,
            OWNER_BETA_PRICE_ONLY_SIMULATION_SCHEMA, SimulationExecutionError, SimulationJobInput,
        },
    },
    queue::JobQueue,
    types::{AttemptOutcome, ClaimedJob, ErrorClass, Job, JobStatus, SettleResult},
};

use super::storage::check_stored_simulation_pair;

const IDEMPOTENCY_PREFIX: &str = "owner-beta:price-only-simulation:v1:";
pub(super) const RUN_PENDING: &str = "PENDING";
pub(super) const RUN_SUCCEEDED: &str = "SUCCEEDED";
pub(super) const RUN_FAILED: &str = "FAILED";
pub(super) const RUN_CANCELED: &str = "CANCELED";

/// Static publication failures. No provider body, rejected bytes, or database
/// diagnostic is included in these values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SimulationPublicationError {
    #[error("simulation database is temporarily unavailable")]
    DatabaseTransient,
    #[error("simulation database integrity check failed")]
    DatabaseIntegrity,
    #[error("simulation queue is temporarily unavailable")]
    QueueTransient,
    #[error("simulation queue claim is no longer current")]
    QueueClaimLost,
    #[error("simulation queue state is inconsistent")]
    QueueIntegrity,
    #[error("simulation binding or stored pair failed validation")]
    Integrity,
    #[error("simulation entitlement does not cover every consumed session")]
    EntitlementDenied,
    #[error("simulation publication commit acknowledgement is unknown")]
    CommitUnknown,
}

impl SimulationPublicationError {
    /// Queue failure classification for callers. CommitUnknown is explicitly
    /// separate from a safe retry: callers must reconcile before deciding.
    pub const fn error_class(self) -> ErrorClass {
        match self {
            Self::DatabaseTransient | Self::QueueTransient => ErrorClass::Transient,
            Self::EntitlementDenied => ErrorClass::DataBlocked,
            Self::DatabaseIntegrity
            | Self::QueueClaimLost
            | Self::QueueIntegrity
            | Self::Integrity
            | Self::CommitUnknown => ErrorClass::Integrity,
        }
    }
}

/// Outcome mirrored by the run and queue transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimulationPublicationOutcome {
    Published,
    Retrying,
    Failed,
    Canceled,
}

/// Result of checking the exact original publication after a lost commit ack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimulationCommitState {
    Published,
    NotPublished,
    Unresolved,
}

/// Bounded worker failures. Only InfrastructureUnavailable is retryable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimulationPublicationFailure {
    InputInvalid,
    EntitlementDenied,
    ProvenanceInvalid,
    ResultInvalid,
    ComputationTimedOut,
    ComputationFailed,
    InfrastructureUnavailable,
    Canceled,
}

impl SimulationPublicationFailure {
    pub const fn error_class(self) -> ErrorClass {
        match self {
            Self::InfrastructureUnavailable => ErrorClass::Transient,
            Self::EntitlementDenied => ErrorClass::DataBlocked,
            Self::InputInvalid
            | Self::ProvenanceInvalid
            | Self::ResultInvalid
            | Self::ComputationTimedOut
            | Self::ComputationFailed
            | Self::Canceled => ErrorClass::Determinism,
        }
    }

    pub const fn code(self) -> &'static str {
        match self {
            Self::InputInvalid => "SIMULATION_INPUT_INVALID",
            Self::EntitlementDenied => "SIMULATION_ENTITLEMENT_DENIED",
            Self::ProvenanceInvalid => "SIMULATION_PROVENANCE_INVALID",
            Self::ResultInvalid => "SIMULATION_RESULT_INVALID",
            Self::ComputationTimedOut => "SIMULATION_COMPUTATION_TIMED_OUT",
            Self::ComputationFailed => "SIMULATION_COMPUTATION_FAILED",
            Self::InfrastructureUnavailable => "SIMULATION_INFRASTRUCTURE_UNAVAILABLE",
            Self::Canceled => "CANCELED",
        }
    }

    const fn message(self) -> &'static str {
        match self {
            Self::InputInvalid => "owner-beta simulation input rejected",
            Self::EntitlementDenied => "owner-beta simulation entitlement unavailable",
            Self::ProvenanceInvalid => "owner-beta simulation provenance rejected",
            Self::ResultInvalid => "owner-beta simulation result rejected",
            Self::ComputationTimedOut => "owner-beta simulation computation timed out",
            Self::ComputationFailed => "owner-beta simulation computation failed",
            Self::InfrastructureUnavailable => "owner-beta simulation infrastructure unavailable",
            Self::Canceled => "owner-beta simulation canceled",
        }
    }
}

/// Fixed recovery counters. These report only the bounded pass performed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SimulationRecoveryReport {
    pub jobs_checked: usize,
    pub attempts_orphaned: usize,
    pub jobs_requeued: usize,
    pub jobs_failed: usize,
    pub runs_canceled: usize,
    pub runs_failed_healed: usize,
    pub jobs_skipped: usize,
}

#[derive(Debug, sqlx::FromRow)]
pub(super) struct LockedSimulationRun {
    pub id: Uuid,
    pub owner_user_id: Uuid,
    pub strategy_config_id: Uuid,
    pub job_id: Uuid,
    pub strategy_id: String,
    pub strategy_version: String,
    pub strategy_config_json: Value,
    pub strategy_config_sha256: String,
    pub evaluation_start: NaiveDate,
    pub evaluation_end: NaiveDate,
    pub session_count: i32,
    pub input_kind: String,
    pub capability: String,
    pub audience: String,
    pub vendor_snapshot: bool,
    pub strict_pit: bool,
    pub action_count: i32,
    pub candidate_content_sha256: String,
    pub artifact_manifest_sha256: String,
    pub stage5_manifest_sha256: String,
    pub action_manifest_sha256: String,
    pub approval_registry_sha256: String,
    pub contract_document_sha256: String,
    pub bars_sha256: String,
    pub status: String,
    pub error_code: Option<String>,
    pub engine_json: Option<Value>,
    pub published_attempt_id: Option<Uuid>,
}

pub(super) async fn lock_simulation_run(
    transaction: &mut Transaction<'_, Postgres>,
    input: &SimulationJobInput,
    owner_user_id: Uuid,
) -> Result<LockedSimulationRun, SimulationPublicationError> {
    sqlx::query_as::<_, LockedSimulationRun>(
        "SELECT id, owner_user_id, strategy_config_id, job_id,
                strategy_id, strategy_version, strategy_config_json,
                strategy_config_sha256, evaluation_start, evaluation_end,
                session_count, input_kind, capability, audience, vendor_snapshot,
                strict_pit, action_count, candidate_content_sha256,
                artifact_manifest_sha256, stage5_manifest_sha256,
                action_manifest_sha256, approval_registry_sha256,
                contract_document_sha256, bars_sha256, status, error_code,
                engine_json, published_attempt_id
           FROM public.owner_beta_simulation_runs
          WHERE id = $1 AND owner_user_id = $2
          FOR UPDATE",
    )
    .bind(input.run_id())
    .bind(owner_user_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(map_database_error)?
    .ok_or(SimulationPublicationError::Integrity)
}

pub(super) async fn lock_simulation_run_for_job(
    transaction: &mut Transaction<'_, Postgres>,
    job_id: Uuid,
    owner_user_id: Uuid,
) -> Result<LockedSimulationRun, SimulationPublicationError> {
    sqlx::query_as::<_, LockedSimulationRun>(
        "SELECT id, owner_user_id, strategy_config_id, job_id,
                strategy_id, strategy_version, strategy_config_json,
                strategy_config_sha256, evaluation_start, evaluation_end,
                session_count, input_kind, capability, audience, vendor_snapshot,
                strict_pit, action_count, candidate_content_sha256,
                artifact_manifest_sha256, stage5_manifest_sha256,
                action_manifest_sha256, approval_registry_sha256,
                contract_document_sha256, bars_sha256, status, error_code,
                engine_json, published_attempt_id
           FROM public.owner_beta_simulation_runs
          WHERE job_id = $1 AND owner_user_id = $2
          FOR UPDATE",
    )
    .bind(job_id)
    .bind(owner_user_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(map_database_error)?
    .ok_or(SimulationPublicationError::Integrity)
}

/// Validate the immutable row-to-row binding without interpreting calendar
/// dates. This is used only after the stored payload itself failed the closed
/// SimulationJobInput parser, so it can safely settle an invalid schedule.
pub(super) fn validate_raw_job_run_binding(
    job: &Job,
    run: &LockedSimulationRun,
) -> Result<(), SimulationPublicationError> {
    let expected_payload = serde_json::json!({
        "schema": OWNER_BETA_PRICE_ONLY_SIMULATION_SCHEMA,
        "run_id": run.id,
        "strategy_config_id": run.strategy_config_id,
        "evaluation_start": run.evaluation_start.to_string(),
        "evaluation_end": run.evaluation_end.to_string(),
        "pins": {
            "candidate_content_sha256": run.candidate_content_sha256,
            "artifact_manifest_sha256": run.artifact_manifest_sha256,
            "stage5_manifest_sha256": run.stage5_manifest_sha256,
            "action_manifest_sha256": run.action_manifest_sha256,
            "approval_registry_sha256": run.approval_registry_sha256
        },
        "contract_document_sha256": run.contract_document_sha256,
        "bars_sha256": run.bars_sha256,
        "strategy": {
            "strategy_id": run.strategy_id,
            "strategy_version": run.strategy_version,
            "config_json": run.strategy_config_json,
            "config_sha256": run.strategy_config_sha256
        }
    });
    let approved_start = NaiveDate::parse_from_str(APPROVED_START_DATE, "%Y-%m-%d")
        .map_err(|_| SimulationPublicationError::Integrity)?;
    let approved_end = NaiveDate::parse_from_str(APPROVED_END_DATE, "%Y-%m-%d")
        .map_err(|_| SimulationPublicationError::Integrity)?;
    if job.job_type != OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE
        || job.max_attempts != 3
        || job
            .idempotency_key
            .as_deref()
            .is_none_or(|key| !key.starts_with(IDEMPOTENCY_PREFIX))
        || job.id != run.job_id
        || job.owner_user_id != run.owner_user_id
        || run.strategy_id != "trend_following"
        || run.strategy_version != "1.0.0"
        || run.strategy_config_json
            != serde_json::json!({
                "benchmark_instrument": "069500.KRX",
                "fast_ma": 50,
                "slow_ma": 200
            })
        || run.strategy_config_sha256
            != "sha256:90e5b4c6972678e9186d4655d76496676e7cfb8dbe473a249f062963873c04a0"
        || run.input_kind != "owner_beta_historical_price_only_v1"
        || run.capability != "PRICE_RETURN_ONLY"
        || run.audience != "OWNER_ONLY"
        || !run.vendor_snapshot
        || run.strict_pit
        || run.action_count != 0
        || !(202..=2452).contains(&run.session_count)
        || run.evaluation_start < approved_start
        || run.evaluation_end > approved_end
        || run.evaluation_end < run.evaluation_start
        || run.candidate_content_sha256 != APPROVED_CANDIDATE_CONTENT_SHA256
        || run.artifact_manifest_sha256 != APPROVED_ARTIFACT_MANIFEST_SHA256
        || run.stage5_manifest_sha256 != APPROVED_STAGE5_MANIFEST_SHA256
        || run.action_manifest_sha256 != APPROVED_ACTION_MANIFEST_SHA256
        || run.approval_registry_sha256 != APPROVED_REGISTRY_SHA256
        || run.contract_document_sha256 != APPROVED_CONTRACT_SHA256
        || run.bars_sha256 != APPROVED_BARS_SHA256
        || job.payload_json != expected_payload
    {
        return Err(SimulationPublicationError::Integrity);
    }
    Ok(())
}

fn claim_snapshot_matches_job(claim: &ClaimedJob, job: &Job) -> bool {
    claim.job.id == job.id
        && claim.job.owner_user_id == job.owner_user_id
        && claim.job.job_type == OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE
        && job.job_type == OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE
        && claim.job.priority == job.priority
        && claim.job.idempotency_key == job.idempotency_key
        && claim.job.status == JobStatus::Running
        && claim.job.max_attempts == 3
        && job.max_attempts == 3
        && claim.job.attempt_count == claim.attempt.attempt_no
        && job.attempt_count == claim.attempt.attempt_no
        && claim.job.locked_by.as_deref() == Some(claim.worker_id.as_str())
        && job.locked_by.as_deref() == Some(claim.worker_id.as_str())
        && claim.attempt.job_id == job.id
        && claim.attempt.attempt_no > 0
        && claim.attempt.outcome == AttemptOutcome::Running
        && claim.attempt.claimed_by.as_deref() == Some(claim.worker_id.as_str())
        && claim.job.payload_json == job.payload_json
}

fn stored_payload_is_invalid(job: &Job) -> Result<bool, SimulationPublicationError> {
    let bytes = serde_json::to_vec(&job.payload_json)
        .map_err(|_| SimulationPublicationError::QueueIntegrity)?;
    Ok(SimulationJobInput::from_json(&bytes).is_err())
}

pub(super) fn validate_job_input(
    job: &Job,
    input: &SimulationJobInput,
) -> Result<(), SimulationPublicationError> {
    let expected_payload =
        serde_json::to_value(input).map_err(|_| SimulationPublicationError::Integrity)?;
    let parsed_input = serde_json::to_vec(&job.payload_json)
        .map_err(|_| SimulationPublicationError::QueueIntegrity)
        .and_then(|bytes| SimulationJobInput::from_json(&bytes).map_err(map_execution_error))?;
    if job.job_type != OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE
        || job
            .idempotency_key
            .as_deref()
            .is_none_or(|key| !key.starts_with(IDEMPOTENCY_PREFIX))
        || job.max_attempts != 3
        || job.payload_json != expected_payload
        || parsed_input != *input
    {
        return Err(SimulationPublicationError::QueueIntegrity);
    }
    Ok(())
}

pub(super) fn validate_run_input(
    run: &LockedSimulationRun,
    input: &SimulationJobInput,
    job: &Job,
) -> Result<(), SimulationPublicationError> {
    let strategy = input.strategy();
    let pins = input.pins();
    let dates = required_dates(input)?;
    if run.id != input.run_id()
        || run.owner_user_id != job.owner_user_id
        || run.strategy_config_id != input.strategy_config_id()
        || run.job_id != job.id
        || run.strategy_id != strategy.strategy_id()
        || run.strategy_version != strategy.strategy_version()
        || run.strategy_config_json != *strategy.config_json()
        || run.strategy_config_sha256 != strategy.config_sha256().as_str()
        || run.evaluation_start != input.evaluation_start().as_naive_date()
        || run.evaluation_end != input.evaluation_end().as_naive_date()
        || run.session_count != i32::try_from(dates.len()).unwrap_or_default()
        || run.input_kind != "owner_beta_historical_price_only_v1"
        || run.capability != "PRICE_RETURN_ONLY"
        || run.audience != "OWNER_ONLY"
        || !run.vendor_snapshot
        || run.strict_pit
        || run.action_count != 0
        || run.candidate_content_sha256 != pins.candidate_content_sha256().as_str()
        || run.artifact_manifest_sha256 != pins.artifact_manifest_sha256().as_str()
        || run.stage5_manifest_sha256 != pins.stage5_manifest_sha256().as_str()
        || run.action_manifest_sha256 != pins.action_manifest_sha256().as_str()
        || run.approval_registry_sha256 != pins.approval_registry_sha256().as_str()
        || run.contract_document_sha256 != input.contract_document_sha256().as_str()
        || run.bars_sha256 != input.bars_sha256().as_str()
    {
        return Err(SimulationPublicationError::Integrity);
    }
    Ok(())
}

pub(super) fn required_dates(
    input: &SimulationJobInput,
) -> Result<Vec<NaiveDate>, SimulationPublicationError> {
    let mut dates = input
        .schedule()
        .warmup_sessions()
        .iter()
        .chain(input.schedule().evaluation_sessions())
        .map(|date| date.as_naive_date())
        .collect::<Vec<_>>();
    if dates.len() < 202
        || dates.len() > 2452
        || dates.windows(2).any(|window| window[0] >= window[1])
        || input.schedule().evaluation_sessions().len() < 2
        || input.schedule().warmup_sessions().len() < 200
    {
        return Err(SimulationPublicationError::Integrity);
    }
    dates.shrink_to_fit();
    Ok(dates)
}

pub(super) async fn ensure_pending_run(
    transaction: &mut Transaction<'_, Postgres>,
    run: &LockedSimulationRun,
) -> Result<(), SimulationPublicationError> {
    let pair_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(
             SELECT 1 FROM public.owner_beta_simulation_result_pairs
              WHERE run_id = $1 AND owner_user_id = $2
         )",
    )
    .bind(run.id)
    .bind(run.owner_user_id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(map_database_error)?;
    if run.status != RUN_PENDING
        || run.error_code.is_some()
        || run.engine_json.is_some()
        || run.published_attempt_id.is_some()
        || pair_exists
    {
        return Err(SimulationPublicationError::Integrity);
    }
    Ok(())
}

/// Validate a whole checked pair, then publish it with the exact live attempt
/// and queue state at one commit boundary.
pub async fn publish_simulation_success(
    queue: &JobQueue,
    claim: &ClaimedJob,
    input: &SimulationJobInput,
    pair: &CheckedSimulationPair,
    expected_engine: &EngineIdentity,
) -> Result<SimulationPublicationOutcome, SimulationPublicationError> {
    let validated_pair = check_stored_simulation_pair(
        pair.illustrative_10bp_per_side_bytes(),
        pair.illustrative_10bp_per_side_sha256().as_str(),
        pair.zero_cost_reference_bytes(),
        pair.zero_cost_reference_sha256().as_str(),
        input,
        expected_engine,
    )?;
    let mut transaction = queue.begin().await.map_err(map_queue_error)?;
    let locked_job = match queue.lock_claim_in(&mut transaction, claim).await {
        Ok(job) => job,
        Err(QueueError::StaleClaim(_)) => {
            return settle_lost_claim(queue, transaction, claim, input, pair, expected_engine)
                .await;
        }
        Err(error) => return rollback_error(transaction, map_queue_error(error)).await,
    };
    validate_job_input(&locked_job, input)?;
    let run = lock_simulation_run(&mut transaction, input, locked_job.owner_user_id).await?;
    validate_run_input(&run, input, &locked_job)?;
    ensure_pending_run(&mut transaction, &run).await?;

    let required_dates = required_dates(input)?;
    let entitled: bool =
        sqlx::query_scalar("SELECT public.lock_owner_beta_simulation_entitlement($1, $2)")
            .bind(locked_job.owner_user_id)
            .bind(&required_dates)
            .fetch_one(&mut *transaction)
            .await
            .map_err(map_database_error)?;
    if !entitled {
        return rollback_error(transaction, SimulationPublicationError::EntitlementDenied).await;
    }

    let inserted = sqlx::query(
        "INSERT INTO public.owner_beta_simulation_result_pairs
            (run_id, owner_user_id, illustrative_canonical, illustrative_sha256,
             zero_cost_canonical, zero_cost_sha256)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(run.id)
    .bind(run.owner_user_id)
    .bind(validated_pair.illustrative_10bp_per_side_bytes())
    .bind(validated_pair.illustrative_10bp_per_side_sha256().as_str())
    .bind(validated_pair.zero_cost_reference_bytes())
    .bind(validated_pair.zero_cost_reference_sha256().as_str())
    .execute(&mut *transaction)
    .await;
    if let Err(error) = inserted {
        return rollback_error(transaction, map_database_error(error)).await;
    }

    let engine_json =
        serde_json::to_value(expected_engine).map_err(|_| SimulationPublicationError::Integrity)?;
    let updated = sqlx::query(
        "UPDATE public.owner_beta_simulation_runs
            SET status = 'SUCCEEDED', error_code = NULL, engine_json = $3,
                published_attempt_id = $4, started_at = COALESCE(started_at, pg_catalog.now()),
                finished_at = pg_catalog.now(), updated_at = pg_catalog.now()
          WHERE id = $1 AND owner_user_id = $2 AND status = 'PENDING'",
    )
    .bind(run.id)
    .bind(run.owner_user_id)
    .bind(engine_json)
    .bind(claim.attempt.id)
    .execute(&mut *transaction)
    .await
    .map_err(map_database_error)?;
    if updated.rows_affected() != 1 {
        return rollback_error(transaction, SimulationPublicationError::Integrity).await;
    }

    match queue.settle_success_in(&mut transaction, claim).await {
        Ok(SettleResult::Committed(job)) if job.status == JobStatus::Succeeded => {}
        Ok(_) => {
            return rollback_error(transaction, SimulationPublicationError::QueueClaimLost).await;
        }
        Err(error) => return rollback_error(transaction, map_queue_error(error)).await,
    }

    match transaction.commit().await {
        Ok(()) => Ok(SimulationPublicationOutcome::Published),
        Err(error) => Err(map_commit_error(error)),
    }
}

/// Settle a typed deterministic or infrastructure failure. Only the latter
/// can retry, and queue.max_attempts remains the fixed three-claim bound.
pub async fn settle_simulation_failure(
    queue: &JobQueue,
    claim: &ClaimedJob,
    input: &SimulationJobInput,
    failure: SimulationPublicationFailure,
) -> Result<SimulationPublicationOutcome, SimulationPublicationError> {
    let mut transaction = queue.begin().await.map_err(map_queue_error)?;
    let locked_job = match queue.lock_claim_in(&mut transaction, claim).await {
        Ok(job) => job,
        Err(QueueError::StaleClaim(_)) => {
            return settle_lost_failure(queue, transaction, claim, input).await;
        }
        Err(error) => return rollback_error(transaction, map_queue_error(error)).await,
    };
    validate_job_input(&locked_job, input)?;
    let run = lock_simulation_run(&mut transaction, input, locked_job.owner_user_id).await?;
    validate_run_input(&run, input, &locked_job)?;
    ensure_pending_run(&mut transaction, &run).await?;

    if failure == SimulationPublicationFailure::Canceled {
        return rollback_error(transaction, SimulationPublicationError::QueueIntegrity).await;
    }
    sqlx::query(
        "UPDATE public.owner_beta_simulation_runs
            SET started_at = COALESCE(started_at, pg_catalog.now()),
                updated_at = pg_catalog.now()
          WHERE id = $1 AND owner_user_id = $2 AND status = 'PENDING'",
    )
    .bind(run.id)
    .bind(run.owner_user_id)
    .execute(&mut *transaction)
    .await
    .map_err(map_database_error)?;

    let result = queue
        .settle_failure_in(
            &mut transaction,
            claim,
            failure.error_class(),
            failure.code(),
            failure.message(),
        )
        .await
        .map_err(map_queue_error);
    let result = match result {
        Ok(result) => result,
        Err(error) => return rollback_error(transaction, error).await,
    };
    let outcome = match result {
        SettleResult::Committed(job) if job.status == JobStatus::Queued => {
            if failure != SimulationPublicationFailure::InfrastructureUnavailable {
                return rollback_error(transaction, SimulationPublicationError::QueueIntegrity)
                    .await;
            }
            SimulationPublicationOutcome::Retrying
        }
        SettleResult::Committed(job) if job.status == JobStatus::Failed => {
            settle_run_terminal(&mut transaction, &run, RUN_FAILED, failure.code()).await?;
            SimulationPublicationOutcome::Failed
        }
        SettleResult::Canceled(job) if job.status == JobStatus::Canceled => {
            settle_run_terminal(&mut transaction, &run, RUN_CANCELED, "CANCELED").await?;
            SimulationPublicationOutcome::Canceled
        }
        _ => return rollback_error(transaction, SimulationPublicationError::QueueIntegrity).await,
    };
    match transaction.commit().await {
        Ok(()) => Ok(outcome),
        Err(error) => Err(map_commit_error(error)),
    }
}

/// Terminally settle a claimed simulation whose stored payload is not a valid
/// `SimulationJobInput`. This deliberately cannot be used for valid payloads.
pub async fn settle_invalid_simulation_claim(
    queue: &JobQueue,
    claim: &ClaimedJob,
) -> Result<SimulationPublicationOutcome, SimulationPublicationError> {
    let mut transaction = queue.begin().await.map_err(map_queue_error)?;
    let job = match queue.lock_claim_in(&mut transaction, claim).await {
        Ok(job) => job,
        Err(QueueError::StaleClaim(_)) => {
            return settle_lost_invalid_claim(queue, transaction, claim).await;
        }
        Err(error) => return rollback_error(transaction, map_queue_error(error)).await,
    };
    if job.job_type != OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE {
        return rollback_error(transaction, SimulationPublicationError::Integrity).await;
    }
    if !stored_payload_is_invalid(&job)? {
        return rollback_error(transaction, SimulationPublicationError::Integrity).await;
    }
    if !claim_snapshot_matches_job(claim, &job) {
        return rollback_error(transaction, SimulationPublicationError::QueueClaimLost).await;
    }

    let run = lock_simulation_run_for_job(&mut transaction, job.id, job.owner_user_id).await?;
    validate_raw_job_run_binding(&job, &run)?;
    ensure_pending_run(&mut transaction, &run).await?;

    let result = queue
        .settle_failure_in(
            &mut transaction,
            claim,
            SimulationPublicationFailure::InputInvalid.error_class(),
            SimulationPublicationFailure::InputInvalid.code(),
            SimulationPublicationFailure::InputInvalid.message(),
        )
        .await
        .map_err(map_queue_error);
    let result = match result {
        Ok(result) => result,
        Err(error) => return rollback_error(transaction, error).await,
    };
    let outcome = match result {
        SettleResult::Committed(job) if job.status == JobStatus::Failed => {
            settle_run_terminal(
                &mut transaction,
                &run,
                RUN_FAILED,
                SimulationPublicationFailure::InputInvalid.code(),
            )
            .await?;
            SimulationPublicationOutcome::Failed
        }
        SettleResult::Canceled(job) if job.status == JobStatus::Canceled => {
            settle_run_terminal(&mut transaction, &run, RUN_CANCELED, "CANCELED").await?;
            SimulationPublicationOutcome::Canceled
        }
        _ => return rollback_error(transaction, SimulationPublicationError::QueueIntegrity).await,
    };
    commit_result(transaction, outcome).await
}

async fn settle_lost_invalid_claim(
    queue: &JobQueue,
    mut transaction: Transaction<'_, Postgres>,
    claim: &ClaimedJob,
) -> Result<SimulationPublicationOutcome, SimulationPublicationError> {
    let job = match lock_job_by_id(&mut transaction, claim.job.id).await? {
        Some(job) => job,
        None => {
            return rollback_error(transaction, SimulationPublicationError::QueueClaimLost).await;
        }
    };
    if job.job_type != OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE
        || !stored_payload_is_invalid(&job)?
    {
        return rollback_error(transaction, SimulationPublicationError::Integrity).await;
    }
    if !claim_snapshot_matches_job(claim, &job) || job.status != JobStatus::Canceled {
        return rollback_error(transaction, SimulationPublicationError::QueueClaimLost).await;
    }

    let run = lock_simulation_run_for_job(&mut transaction, job.id, job.owner_user_id).await?;
    validate_raw_job_run_binding(&job, &run)?;
    if invalid_claim_cancellation_is_mirrored(&mut transaction, claim, &job, &run).await? {
        return commit_result(transaction, SimulationPublicationOutcome::Canceled).await;
    }
    ensure_pending_run(&mut transaction, &run).await?;
    match queue
        .settle_aborted_in(
            &mut transaction,
            claim,
            SimulationPublicationFailure::InputInvalid.message(),
        )
        .await
    {
        Ok(settled) if settled.status == JobStatus::Canceled => {}
        Ok(_) => {
            return rollback_error(transaction, SimulationPublicationError::QueueIntegrity).await;
        }
        Err(QueueError::StaleClaim(_)) => {
            return rollback_error(transaction, SimulationPublicationError::QueueClaimLost).await;
        }
        Err(error) => return rollback_error(transaction, map_queue_error(error)).await,
    }
    settle_run_terminal(&mut transaction, &run, RUN_CANCELED, "CANCELED").await?;
    commit_result(transaction, SimulationPublicationOutcome::Canceled).await
}

async fn invalid_claim_cancellation_is_mirrored(
    transaction: &mut Transaction<'_, Postgres>,
    claim: &ClaimedJob,
    job: &Job,
    run: &LockedSimulationRun,
) -> Result<bool, SimulationPublicationError> {
    if job.status != JobStatus::Canceled
        || run.status != RUN_CANCELED
        || run.error_code.as_deref() != Some("CANCELED")
        || run.engine_json.is_some()
        || run.published_attempt_id.is_some()
    {
        return Ok(false);
    }
    let exact_canceled_attempt: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM public.job_attempts
              WHERE id = $1 AND job_id = $2 AND attempt_no = $3
                AND claimed_by = $4
                AND ((outcome = 'FAILED' AND error_code = 'canceled')
                     OR (outcome = 'ORPHANED' AND error_code IS NULL))
         )",
    )
    .bind(claim.attempt.id)
    .bind(job.id)
    .bind(job.attempt_count)
    .bind(&claim.worker_id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(map_database_error)?;
    let pair_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM public.owner_beta_simulation_result_pairs
              WHERE run_id = $1 AND owner_user_id = $2
         )",
    )
    .bind(run.id)
    .bind(run.owner_user_id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(map_database_error)?;
    Ok(exact_canceled_attempt && !pair_exists)
}

/// Inspect the committed state after an uncertain acknowledgement. This only
/// reads and locks the original owner/run/job/attempt; it never inserts again.
pub async fn reconcile_simulation_commit(
    queue: &JobQueue,
    claim: &ClaimedJob,
    input: &SimulationJobInput,
    pair: &CheckedSimulationPair,
    expected_engine: &EngineIdentity,
) -> Result<SimulationCommitState, SimulationPublicationError> {
    let expected_pair = check_stored_simulation_pair(
        pair.illustrative_10bp_per_side_bytes(),
        pair.illustrative_10bp_per_side_sha256().as_str(),
        pair.zero_cost_reference_bytes(),
        pair.zero_cost_reference_sha256().as_str(),
        input,
        expected_engine,
    )?;
    let mut transaction = queue.begin().await.map_err(map_queue_error)?;
    let job = match lock_job_by_id(&mut transaction, claim.job.id).await {
        Ok(Some(job)) => job,
        Ok(None) => return rollback_state(transaction, SimulationCommitState::Unresolved).await,
        Err(error) => return rollback_error(transaction, error).await,
    };
    if job.owner_user_id != claim.job.owner_user_id || validate_job_input(&job, input).is_err() {
        return rollback_state(transaction, SimulationCommitState::Unresolved).await;
    }
    let run = match lock_simulation_run(&mut transaction, input, job.owner_user_id).await {
        Ok(run) => run,
        Err(SimulationPublicationError::Integrity) => {
            return rollback_state(transaction, SimulationCommitState::Unresolved).await;
        }
        Err(error) => return rollback_error(transaction, error).await,
    };
    if validate_run_input(&run, input, &job).is_err() {
        return rollback_state(transaction, SimulationCommitState::Unresolved).await;
    }

    let stored: Option<StoredPair> = sqlx::query_as(
        "SELECT illustrative_canonical, illustrative_sha256,
                zero_cost_canonical, zero_cost_sha256
           FROM public.owner_beta_simulation_result_pairs
          WHERE run_id = $1 AND owner_user_id = $2",
    )
    .bind(run.id)
    .bind(run.owner_user_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(map_database_error)?;

    let original_attempt: Option<(Uuid, i32, String, Option<String>)> = sqlx::query_as(
        "SELECT job_id, attempt_no, outcome, claimed_by
           FROM public.job_attempts WHERE id = $1",
    )
    .bind(claim.attempt.id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(map_database_error)?;
    let attempt_identity_matches =
        original_attempt
            .as_ref()
            .is_some_and(|(job_id, attempt_no, _, claimed_by)| {
                *job_id == claim.job.id
                    && *attempt_no == claim.attempt.attempt_no
                    && claimed_by.as_deref() == Some(claim.worker_id.as_str())
            });
    let attempt_succeeded = attempt_identity_matches
        && original_attempt
            .as_ref()
            .is_some_and(|(_, _, outcome, _)| outcome == "SUCCEEDED");
    let attempt_not_published = attempt_identity_matches
        && original_attempt
            .as_ref()
            .is_some_and(|(_, _, outcome, _)| outcome != "SUCCEEDED");

    let state = match (stored, job.status, run.status.as_str()) {
        (Some(stored), JobStatus::Succeeded, RUN_SUCCEEDED) => {
            if run.published_attempt_id != Some(claim.attempt.id)
                || claim.attempt.attempt_no != job.attempt_count
                || !attempt_succeeded
                || run.engine_json.as_ref() != Some(&engine_value(expected_engine)?)
                || run.error_code.is_some()
            {
                SimulationCommitState::Unresolved
            } else {
                match check_stored_simulation_pair(
                    &stored.illustrative_canonical,
                    &stored.illustrative_sha256,
                    &stored.zero_cost_canonical,
                    &stored.zero_cost_sha256,
                    input,
                    expected_engine,
                ) {
                    Ok(stored_pair) => {
                        let bytes_match = stored_pair.illustrative_10bp_per_side_bytes()
                            == expected_pair.illustrative_10bp_per_side_bytes()
                            && stored_pair.zero_cost_reference_bytes()
                                == expected_pair.zero_cost_reference_bytes()
                            && stored_pair.illustrative_10bp_per_side_sha256()
                                == expected_pair.illustrative_10bp_per_side_sha256()
                            && stored_pair.zero_cost_reference_sha256()
                                == expected_pair.zero_cost_reference_sha256();
                        if bytes_match {
                            SimulationCommitState::Published
                        } else {
                            SimulationCommitState::Unresolved
                        }
                    }
                    _ => SimulationCommitState::Unresolved,
                }
            }
        }
        (None, JobStatus::Succeeded, _) | (Some(_), _, _) => SimulationCommitState::Unresolved,
        (None, JobStatus::Canceled, RUN_PENDING)
            if attempt_not_published
                && run.engine_json.is_none()
                && run.published_attempt_id.is_none()
                && run.error_code.is_none() =>
        {
            SimulationCommitState::NotPublished
        }
        (None, JobStatus::Canceled, RUN_CANCELED)
            if attempt_not_published
                && run.engine_json.is_none()
                && run.published_attempt_id.is_none() =>
        {
            SimulationCommitState::NotPublished
        }
        (None, JobStatus::Failed, RUN_FAILED)
            if attempt_not_published
                && run.engine_json.is_none()
                && run.published_attempt_id.is_none() =>
        {
            SimulationCommitState::NotPublished
        }
        (None, JobStatus::Queued | JobStatus::Running, RUN_PENDING)
            if attempt_not_published
                && run.engine_json.is_none()
                && run.published_attempt_id.is_none()
                && run.error_code.is_none() =>
        {
            SimulationCommitState::NotPublished
        }
        _ => SimulationCommitState::Unresolved,
    };
    rollback_state(transaction, state).await
}

#[derive(Debug, sqlx::FromRow)]
struct StoredPair {
    illustrative_canonical: Vec<u8>,
    illustrative_sha256: String,
    zero_cost_canonical: Vec<u8>,
    zero_cost_sha256: String,
}

pub(super) async fn lock_job_by_id(
    transaction: &mut Transaction<'_, Postgres>,
    job_id: Uuid,
) -> Result<Option<Job>, SimulationPublicationError> {
    sqlx::query_as::<_, Job>(
        "SELECT id, owner_user_id, job_type, status, priority, idempotency_key,
                payload_json, max_attempts, attempt_count, available_at, locked_by,
                locked_at, started_at, finished_at, error_code, error_message,
                created_at, updated_at
           FROM public.jobs WHERE id = $1 FOR UPDATE",
    )
    .bind(job_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(map_database_error)
}

pub(super) async fn settle_run_terminal(
    transaction: &mut Transaction<'_, Postgres>,
    run: &LockedSimulationRun,
    status: &str,
    error_code: &str,
) -> Result<(), SimulationPublicationError> {
    if !matches!(status, RUN_FAILED | RUN_CANCELED) {
        return Err(SimulationPublicationError::Integrity);
    }
    let update = sqlx::query(
        "UPDATE public.owner_beta_simulation_runs
            SET status = $3, error_code = $4, engine_json = NULL,
                published_attempt_id = NULL, started_at = COALESCE(started_at, pg_catalog.now()),
                finished_at = pg_catalog.now(), updated_at = pg_catalog.now()
          WHERE id = $1 AND owner_user_id = $2 AND status = 'PENDING'",
    )
    .bind(run.id)
    .bind(run.owner_user_id)
    .bind(status)
    .bind(error_code)
    .execute(&mut **transaction)
    .await
    .map_err(map_database_error)?;
    if update.rows_affected() != 1 {
        return Err(SimulationPublicationError::Integrity);
    }
    Ok(())
}

async fn settle_lost_claim(
    queue: &JobQueue,
    mut transaction: Transaction<'_, Postgres>,
    claim: &ClaimedJob,
    input: &SimulationJobInput,
    pair: &CheckedSimulationPair,
    expected_engine: &EngineIdentity,
) -> Result<SimulationPublicationOutcome, SimulationPublicationError> {
    let job = match lock_job_by_id(&mut transaction, claim.job.id).await? {
        Some(job) => job,
        None => {
            return rollback_error(transaction, SimulationPublicationError::QueueClaimLost).await;
        }
    };
    if job.status == JobStatus::Succeeded {
        transaction.rollback().await.map_err(map_database_error)?;
        return match reconcile_simulation_commit(queue, claim, input, pair, expected_engine).await?
        {
            SimulationCommitState::Published => Ok(SimulationPublicationOutcome::Published),
            SimulationCommitState::NotPublished => Err(SimulationPublicationError::QueueIntegrity),
            SimulationCommitState::Unresolved => Err(SimulationPublicationError::CommitUnknown),
        };
    }
    if job.status != JobStatus::Canceled {
        return rollback_error(transaction, SimulationPublicationError::QueueClaimLost).await;
    }
    validate_job_input(&job, input)?;
    let run = lock_simulation_run(&mut transaction, input, job.owner_user_id).await?;
    validate_run_input(&run, input, &job)?;
    if run.status == RUN_CANCELED && run.error_code.as_deref() == Some("CANCELED") {
        return commit_result(transaction, SimulationPublicationOutcome::Canceled).await;
    }
    ensure_pending_run(&mut transaction, &run).await?;
    match queue
        .settle_aborted_in(&mut transaction, claim, "owner-beta simulation canceled")
        .await
    {
        Ok(_) => {}
        Err(QueueError::StaleClaim(_)) => {
            return rollback_error(transaction, SimulationPublicationError::QueueClaimLost).await;
        }
        Err(error) => return rollback_error(transaction, map_queue_error(error)).await,
    }
    settle_run_terminal(&mut transaction, &run, RUN_CANCELED, "CANCELED").await?;
    commit_result(transaction, SimulationPublicationOutcome::Canceled).await
}

async fn settle_lost_failure(
    queue: &JobQueue,
    mut transaction: Transaction<'_, Postgres>,
    claim: &ClaimedJob,
    input: &SimulationJobInput,
) -> Result<SimulationPublicationOutcome, SimulationPublicationError> {
    let job = match lock_job_by_id(&mut transaction, claim.job.id).await? {
        Some(job) => job,
        None => {
            return rollback_error(transaction, SimulationPublicationError::QueueClaimLost).await;
        }
    };
    if job.status != JobStatus::Canceled {
        return rollback_error(transaction, SimulationPublicationError::QueueClaimLost).await;
    }
    validate_job_input(&job, input)?;
    let run = lock_simulation_run(&mut transaction, input, job.owner_user_id).await?;
    validate_run_input(&run, input, &job)?;
    if run.status == RUN_CANCELED && run.error_code.as_deref() == Some("CANCELED") {
        return commit_result(transaction, SimulationPublicationOutcome::Canceled).await;
    }
    ensure_pending_run(&mut transaction, &run).await?;
    match queue
        .settle_aborted_in(&mut transaction, claim, "owner-beta simulation canceled")
        .await
    {
        Ok(_) => {}
        Err(QueueError::StaleClaim(_)) => {
            return rollback_error(transaction, SimulationPublicationError::QueueClaimLost).await;
        }
        Err(error) => return rollback_error(transaction, map_queue_error(error)).await,
    }
    settle_run_terminal(&mut transaction, &run, RUN_CANCELED, "CANCELED").await?;
    commit_result(transaction, SimulationPublicationOutcome::Canceled).await
}

pub(super) async fn commit_result<T>(
    transaction: Transaction<'_, Postgres>,
    value: T,
) -> Result<T, SimulationPublicationError> {
    transaction.commit().await.map_err(map_commit_error)?;
    Ok(value)
}

pub(super) async fn rollback_error<T>(
    transaction: Transaction<'_, Postgres>,
    error: SimulationPublicationError,
) -> Result<T, SimulationPublicationError> {
    let _ = transaction.rollback().await;
    Err(error)
}

async fn rollback_state(
    transaction: Transaction<'_, Postgres>,
    state: SimulationCommitState,
) -> Result<SimulationCommitState, SimulationPublicationError> {
    transaction.rollback().await.map_err(map_database_error)?;
    Ok(state)
}

fn engine_value(engine: &EngineIdentity) -> Result<Value, SimulationPublicationError> {
    serde_json::to_value(engine).map_err(|_| SimulationPublicationError::Integrity)
}

fn map_execution_error(error: SimulationExecutionError) -> SimulationPublicationError {
    match error {
        SimulationExecutionError::InputInvalid
        | SimulationExecutionError::WindowInvalid
        | SimulationExecutionError::StrategyInvalid
        | SimulationExecutionError::PairInvalid
        | SimulationExecutionError::ProvenanceMismatch
        | SimulationExecutionError::SizeLimitExceeded => SimulationPublicationError::Integrity,
    }
}

pub(super) fn map_database_error(error: sqlx::Error) -> SimulationPublicationError {
    match database_error_class(&error) {
        ErrorClass::Transient => SimulationPublicationError::DatabaseTransient,
        ErrorClass::Input
        | ErrorClass::DataBlocked
        | ErrorClass::Integrity
        | ErrorClass::Determinism => SimulationPublicationError::DatabaseIntegrity,
    }
}

fn map_commit_error(error: sqlx::Error) -> SimulationPublicationError {
    match error {
        sqlx::Error::Database(database_error) => {
            let acknowledgement_unknown = database_error.code().as_deref().is_some_and(|code| {
                code.starts_with("08") || matches!(code, "57P01" | "57P02" | "57P03")
            });
            if acknowledgement_unknown {
                SimulationPublicationError::CommitUnknown
            } else {
                map_database_error(sqlx::Error::Database(database_error))
            }
        }
        sqlx::Error::RowNotFound
        | sqlx::Error::ColumnNotFound(_)
        | sqlx::Error::TypeNotFound { .. }
        | sqlx::Error::ColumnDecode { .. }
        | sqlx::Error::Decode(_) => SimulationPublicationError::DatabaseIntegrity,
        _ => SimulationPublicationError::CommitUnknown,
    }
}

pub(super) fn map_queue_error(error: QueueError) -> SimulationPublicationError {
    match error {
        QueueError::StaleClaim(_) => SimulationPublicationError::QueueClaimLost,
        QueueError::CommitUnknown { .. } => SimulationPublicationError::CommitUnknown,
        QueueError::Database(error) => match queue_error_class(&QueueError::Database(error)) {
            ErrorClass::Transient => SimulationPublicationError::QueueTransient,
            _ => SimulationPublicationError::QueueIntegrity,
        },
        QueueError::JobNotFound(_)
        | QueueError::AlreadyTerminal(_, _)
        | QueueError::InvalidInput(_)
        | QueueError::AuditUnavailable
        | QueueError::Internal(_) => SimulationPublicationError::QueueIntegrity,
    }
}
