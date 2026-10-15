//! Atomic publication for the sealed owner-beta price recommendation.
//!
//! This module deliberately writes only the dedicated owner-beta tables.  It
//! has no bridge to another publication domain, Paper, or Curated data.

use std::{collections::BTreeSet, fmt};

use factor_engine::{
    PriceOnlyFactorSnapshot,
    price_only::{PRICE_ONLY_CAPABILITY, PRICE_ONLY_INPUT_KIND},
};
use market_data::KR_ETF_CORE_SYMBOLS;
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};

use crate::{
    QueueError,
    error::{database_error_class, queue_error_class},
    queue::JobQueue,
    types::{ClaimedJob, ErrorClass, JobStatus, SettleResult},
};

use super::{
    OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE, OWNER_BETA_TARGET_HASH_ALGORITHM,
    OWNER_BETA_TARGET_SNAPSHOT_SCHEMA, OWNER_BETA_TARGET_WEIGHT_SCALE,
    OwnerBetaPriceRecommendationInput, OwnerBetaTargetSnapshot,
};

/// Sanitized publisher failures.  Deliberately value-free: values that reach
/// this boundary include private config and provenance pins.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OwnerBetaPublicationError {
    DatabaseTransient,
    DatabaseIntegrity,
    QueueTransient,
    QueueClaimLost,
    QueueIntegrity,
    Integrity,
    EntitlementDenied,
    DailyAuthorityDenied,
    CommitUnknown,
}

impl fmt::Debug for OwnerBetaPublicationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl fmt::Display for OwnerBetaPublicationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::error::Error for OwnerBetaPublicationError {}

impl OwnerBetaPublicationError {
    const fn as_str(self) -> &'static str {
        match self {
            Self::DatabaseTransient => "owner-beta publication database is temporarily unavailable",
            Self::DatabaseIntegrity => "owner-beta publication database contract failed",
            Self::QueueTransient => "owner-beta publication queue is temporarily unavailable",
            Self::QueueClaimLost => "owner-beta publication queue claim was lost",
            Self::QueueIntegrity => "owner-beta publication queue contract failed",
            Self::Integrity => "owner-beta publication integrity check failed",
            Self::EntitlementDenied => "owner-beta publication entitlement is inactive",
            Self::DailyAuthorityDenied => "owner-beta daily authority is unavailable",
            Self::CommitUnknown => "owner-beta publication commit outcome is unknown",
        }
    }

    pub const fn class(self) -> ErrorClass {
        match self {
            Self::DatabaseTransient | Self::QueueTransient => ErrorClass::Transient,
            Self::EntitlementDenied | Self::DailyAuthorityDenied => ErrorClass::DataBlocked,
            Self::DatabaseIntegrity
            | Self::QueueClaimLost
            | Self::QueueIntegrity
            | Self::Integrity
            | Self::CommitUnknown => ErrorClass::Integrity,
        }
    }
}

/// The only externally selectable owner-beta settlement causes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerBetaPublicationFailure {
    InputInvalid,
    EntitlementDenied,
    DailyAuthorityDenied,
    DailyExecutionDisabled,
    FactorInvalid,
    TargetInvalid,
    PublicationUnavailable,
    ComputationUnavailable,
    ComputationFailed,
    Canceled,
}

impl OwnerBetaPublicationFailure {
    pub const fn class(self) -> ErrorClass {
        match self {
            Self::PublicationUnavailable | Self::ComputationUnavailable => ErrorClass::Transient,
            Self::EntitlementDenied | Self::DailyAuthorityDenied | Self::DailyExecutionDisabled => {
                ErrorClass::DataBlocked
            }
            Self::ComputationFailed => ErrorClass::Determinism,
            Self::InputInvalid | Self::FactorInvalid | Self::TargetInvalid | Self::Canceled => {
                ErrorClass::Integrity
            }
        }
    }

    pub const fn code(self) -> &'static str {
        match self {
            Self::InputInvalid => "OWNER_BETA_INPUT_INVALID",
            Self::EntitlementDenied => "OWNER_BETA_ENTITLEMENT_DENIED",
            Self::DailyAuthorityDenied => "OWNER_BETA_DAILY_AUTHORITY_DENIED",
            Self::DailyExecutionDisabled => "OWNER_BETA_DAILY_EXECUTION_DISABLED",
            Self::FactorInvalid => "OWNER_BETA_FACTOR_INVALID",
            Self::TargetInvalid => "OWNER_BETA_TARGET_INVALID",
            Self::PublicationUnavailable => "OWNER_BETA_PUBLICATION_UNAVAILABLE",
            Self::ComputationUnavailable => "OWNER_BETA_COMPUTATION_UNAVAILABLE",
            Self::ComputationFailed => "OWNER_BETA_COMPUTATION_FAILED",
            Self::Canceled => "CANCELED",
        }
    }

    const fn message(self) -> &'static str {
        match self {
            Self::InputInvalid => "owner-beta input rejected",
            Self::EntitlementDenied => "owner-beta entitlement unavailable",
            Self::DailyAuthorityDenied => "owner-beta daily authority unavailable",
            Self::DailyExecutionDisabled => "owner-beta daily execution disabled",
            Self::FactorInvalid => "owner-beta factor snapshot rejected",
            Self::TargetInvalid => "owner-beta target snapshot rejected",
            Self::PublicationUnavailable => "owner-beta publication unavailable",
            Self::ComputationUnavailable => "owner-beta computation unavailable",
            Self::ComputationFailed => "owner-beta computation failed",
            Self::Canceled => "owner-beta recommendation canceled",
        }
    }
}

/// Final state visible to the owner-beta worker after this module settles a
/// claimed job. A success publication can return [`Self::Canceled`] when the
/// audited cancellation transaction won before publication acquired the job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerBetaPublicationOutcome {
    Published,
    Retrying,
    Failed,
    Canceled,
}

#[derive(sqlx::FromRow)]
struct LockedRun {
    status: String,
    owner_user_id: uuid::Uuid,
    strategy_config_id: uuid::Uuid,
    strategy_id: String,
    strategy_version: String,
    strategy_config_json: Value,
    strategy_config_sha256: String,
    as_of: chrono::NaiveDate,
    job_id: uuid::Uuid,
    input_kind: String,
    capability: String,
    candidate_content_sha256: String,
    artifact_manifest_sha256: String,
    stage5_manifest_sha256: String,
    action_manifest_sha256: String,
    approval_registry_sha256: String,
    factor_snapshot_sha256: Option<String>,
    target_snapshot_sha256: Option<String>,
    cash_weight: Option<String>,
    error_code: Option<String>,
}

/// Writes one exact eleven-member target, transitions the dedicated run, and
/// settles its claim within one commit boundary.
pub async fn publish_owner_beta_success(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
    input: &OwnerBetaPriceRecommendationInput,
    factor: &PriceOnlyFactorSnapshot,
    target: &OwnerBetaTargetSnapshot,
) -> Result<OwnerBetaPublicationOutcome, OwnerBetaPublicationError> {
    validate_success_inputs(claim, input, factor, target)?;

    let mut transaction = pool.begin().await.map_err(database_error)?;
    let publication =
        publish_success_in(&mut transaction, queue, claim, input, factor, target).await;
    match publication {
        Ok(outcome) => match transaction.commit().await {
            Ok(()) => Ok(outcome),
            Err(_) => Err(OwnerBetaPublicationError::CommitUnknown),
        },
        Err(error) => {
            let _ = transaction.rollback().await;
            Err(error)
        }
    }
}

/// Settles a sealed owner-beta failure and mirrors the resulting job state to
/// its run.  The queue is always locked/settled before the run is locked.
pub async fn settle_owner_beta_failure(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
    input: &OwnerBetaPriceRecommendationInput,
    failure: OwnerBetaPublicationFailure,
) -> Result<OwnerBetaPublicationOutcome, OwnerBetaPublicationError> {
    validate_failure_claim(claim, input)?;
    let mut transaction = pool.begin().await.map_err(database_error)?;
    let result = settle_failure_in(&mut transaction, queue, claim, input, failure).await;
    match result {
        Ok(outcome) => match transaction.commit().await {
            Ok(()) => Ok(outcome),
            Err(_) => Err(OwnerBetaPublicationError::CommitUnknown),
        },
        Err(error) => {
            let _ = transaction.rollback().await;
            Err(error)
        }
    }
}

/// Settles a malformed exact owner-beta claim without inventing a sealed
/// payload. The queue transition is authoritative and occurs before an
/// optional run mirror selected solely by `job_id` and owner.
pub(super) async fn settle_malformed_owner_beta_claim(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
) -> Result<OwnerBetaPublicationOutcome, OwnerBetaPublicationError> {
    if claim.job.job_type != OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE {
        return Err(OwnerBetaPublicationError::Integrity);
    }
    let mut transaction = pool.begin().await.map_err(database_error)?;
    let result = settle_malformed_claim_in(&mut transaction, queue, claim).await;
    match result {
        Ok(outcome) => match transaction.commit().await {
            Ok(()) => Ok(outcome),
            Err(_) => Err(OwnerBetaPublicationError::CommitUnknown),
        },
        Err(error) => {
            let _ = transaction.rollback().await;
            Err(error)
        }
    }
}

async fn settle_malformed_claim_in(
    transaction: &mut Transaction<'_, Postgres>,
    queue: &JobQueue,
    claim: &ClaimedJob,
) -> Result<OwnerBetaPublicationOutcome, OwnerBetaPublicationError> {
    let status = match queue
        .settle_failure_in(
            transaction,
            claim,
            ErrorClass::Integrity,
            OwnerBetaPublicationFailure::InputInvalid.code(),
            OwnerBetaPublicationFailure::InputInvalid.message(),
        )
        .await
        .map_err(queue_error)?
    {
        SettleResult::Committed(job) | SettleResult::Canceled(job) => job.status,
    };
    let (outcome, run_status, error_code) = match status {
        JobStatus::Failed => (
            OwnerBetaPublicationOutcome::Failed,
            "FAILED",
            OwnerBetaPublicationFailure::InputInvalid.code(),
        ),
        JobStatus::Canceled => (
            OwnerBetaPublicationOutcome::Canceled,
            "CANCELED",
            "CANCELED",
        ),
        JobStatus::Queued | JobStatus::Running | JobStatus::Succeeded => {
            return Err(OwnerBetaPublicationError::QueueIntegrity);
        }
    };
    let linked_run: Option<(uuid::Uuid,)> = sqlx::query_as(
        "SELECT id FROM public.owner_beta_recommendation_runs \
         WHERE job_id = $1 AND owner_user_id = $2 FOR UPDATE",
    )
    .bind(claim.job.id)
    .bind(claim.job.owner_user_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(database_error)?;
    if let Some((run_id,)) = linked_run {
        let updated = sqlx::query(
            "UPDATE public.owner_beta_recommendation_runs \
             SET status = $3, factor_snapshot_sha256 = NULL, target_snapshot_sha256 = NULL, \
                 cash_weight = NULL, error_code = $4, started_at = COALESCE(started_at, now()), \
                 finished_at = now(), updated_at = now() \
             WHERE id = $1 AND owner_user_id = $2 AND status = 'PENDING' \
               AND NOT EXISTS (SELECT 1 FROM public.owner_beta_recommendation_items \
                               WHERE recommendation_run_id = $1 AND owner_user_id = $2)",
        )
        .bind(run_id)
        .bind(claim.job.owner_user_id)
        .bind(run_status)
        .bind(error_code)
        .execute(&mut **transaction)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() != 1 {
            return Err(OwnerBetaPublicationError::Integrity);
        }
    }
    Ok(outcome)
}

async fn publish_success_in(
    transaction: &mut Transaction<'_, Postgres>,
    queue: &JobQueue,
    claim: &ClaimedJob,
    input: &OwnerBetaPriceRecommendationInput,
    factor: &PriceOnlyFactorSnapshot,
    target: &OwnerBetaTargetSnapshot,
) -> Result<OwnerBetaPublicationOutcome, OwnerBetaPublicationError> {
    // Lock the queue authority before any owner-beta row, consistently with
    // queue settlement and cancellation.
    let locked_job = match queue.lock_claim_in(transaction, claim).await {
        Ok(job) => job,
        Err(QueueError::StaleClaim(_)) => {
            return settle_failure_in(
                transaction,
                queue,
                claim,
                input,
                OwnerBetaPublicationFailure::Canceled,
            )
            .await;
        }
        Err(error) => return Err(queue_error(error)),
    };
    validate_locked_job(&locked_job, claim, input)?;

    let run: LockedRun = sqlx::query_as(
        "SELECT status, owner_user_id, strategy_config_id, strategy_id, strategy_version, \
                strategy_config_json, strategy_config_sha256, as_of, job_id, input_kind, capability, \
                candidate_content_sha256, artifact_manifest_sha256, stage5_manifest_sha256, \
                action_manifest_sha256, approval_registry_sha256, factor_snapshot_sha256, \
                target_snapshot_sha256, cash_weight::text AS cash_weight, error_code \
         FROM public.owner_beta_recommendation_runs \
         WHERE id = $1 AND owner_user_id = $2 FOR UPDATE",
    )
    .bind(input.run_id())
    .bind(claim.job.owner_user_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(database_error)?
    .ok_or(OwnerBetaPublicationError::Integrity)?;
    validate_locked_run(&run, claim, input)?;

    let entitled: bool =
        sqlx::query_scalar("SELECT public.lock_recommendation_entitlement($1, 'krx_eod_bars', $2)")
            .bind(claim.job.owner_user_id)
            .bind(input.as_of().as_naive_date())
            .fetch_one(&mut **transaction)
            .await
            .map_err(database_error)?;
    if !entitled {
        return Err(OwnerBetaPublicationError::EntitlementDenied);
    }

    let item_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.owner_beta_recommendation_items \
         WHERE recommendation_run_id = $1 AND owner_user_id = $2",
    )
    .bind(input.run_id())
    .bind(claim.job.owner_user_id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(database_error)?;
    if item_count != 0 {
        return Err(OwnerBetaPublicationError::Integrity);
    }

    for item in target.items() {
        let reason_codes = Value::Array(
            item.reasons()
                .iter()
                .map(|reason| Value::String(reason.code().to_owned()))
                .collect(),
        );
        let factors_json = serde_json::to_value(item.factors())
            .map_err(|_| OwnerBetaPublicationError::Integrity)?;
        let target_weight = item.target_weight();
        let excluded = target_weight.is_none();
        let exclusion_reason = excluded
            .then(|| item.reasons().first().map(|reason| reason.code()))
            .flatten();
        let inserted = sqlx::query(
            "INSERT INTO public.owner_beta_recommendation_items \
             (recommendation_run_id, owner_user_id, instrument_id, rank, target_weight, \
              reason_codes, factors_json, excluded, exclusion_reason) \
             VALUES ($1, $2, $3, $4, $5::numeric, $6, $7, $8, $9)",
        )
        .bind(input.run_id())
        .bind(claim.job.owner_user_id)
        .bind(item.instrument_id())
        .bind(
            item.rank()
                .map(i32::try_from)
                .transpose()
                .map_err(|_| OwnerBetaPublicationError::Integrity)?,
        )
        .bind(target_weight)
        .bind(reason_codes)
        .bind(factors_json)
        .bind(excluded)
        .bind(exclusion_reason)
        .execute(&mut **transaction)
        .await
        .map_err(database_error)?;
        if inserted.rows_affected() != 1 {
            return Err(OwnerBetaPublicationError::Integrity);
        }
    }

    let updated = sqlx::query(
        "UPDATE public.owner_beta_recommendation_runs \
         SET status = 'SUCCEEDED', factor_snapshot_sha256 = $3, target_snapshot_sha256 = $4, \
             cash_weight = $5::numeric, error_code = NULL, started_at = COALESCE(started_at, now()), \
             finished_at = now(), updated_at = now() \
         WHERE id = $1 AND owner_user_id = $2 AND status = 'PENDING'",
    )
    .bind(input.run_id())
    .bind(claim.job.owner_user_id)
    .bind(factor.hash.as_str())
    .bind(target.target_snapshot_sha256().as_str())
    .bind(target.cash_weight())
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    if updated.rows_affected() != 1 {
        return Err(OwnerBetaPublicationError::Integrity);
    }

    match queue
        .settle_success_in(transaction, claim)
        .await
        .map_err(queue_error)?
    {
        SettleResult::Committed(job) if job.status == JobStatus::Succeeded => {
            Ok(OwnerBetaPublicationOutcome::Published)
        }
        _ => Err(OwnerBetaPublicationError::QueueClaimLost),
    }
}

async fn settle_failure_in(
    transaction: &mut Transaction<'_, Postgres>,
    queue: &JobQueue,
    claim: &ClaimedJob,
    input: &OwnerBetaPriceRecommendationInput,
    failure: OwnerBetaPublicationFailure,
) -> Result<OwnerBetaPublicationOutcome, OwnerBetaPublicationError> {
    let status = if failure == OwnerBetaPublicationFailure::Canceled {
        queue
            .settle_aborted_in(transaction, claim, failure.message())
            .await
            .map_err(queue_error)?
            .status
    } else {
        match queue
            .settle_failure_in(
                transaction,
                claim,
                failure.class(),
                failure.code(),
                failure.message(),
            )
            .await
            .map_err(queue_error)?
        {
            SettleResult::Committed(job) | SettleResult::Canceled(job) => job.status,
        }
    };

    let (outcome, run_status, error_code) = match status {
        JobStatus::Queued => (OwnerBetaPublicationOutcome::Retrying, "PENDING", None),
        JobStatus::Failed => (
            OwnerBetaPublicationOutcome::Failed,
            "FAILED",
            Some(failure.code()),
        ),
        JobStatus::Canceled => (
            OwnerBetaPublicationOutcome::Canceled,
            "CANCELED",
            Some("CANCELED"),
        ),
        JobStatus::Running | JobStatus::Succeeded => {
            return Err(OwnerBetaPublicationError::QueueIntegrity);
        }
    };
    let updated = if run_status == "PENDING" {
        sqlx::query(
            "UPDATE public.owner_beta_recommendation_runs \
             SET status = 'PENDING', factor_snapshot_sha256 = NULL, target_snapshot_sha256 = NULL, \
                 cash_weight = NULL, error_code = NULL, started_at = NULL, finished_at = NULL, updated_at = now() \
             WHERE id = $1 AND owner_user_id = $2 AND job_id = $3 AND status = 'PENDING' \
               AND NOT EXISTS (SELECT 1 FROM public.owner_beta_recommendation_items \
                               WHERE recommendation_run_id = $1 AND owner_user_id = $2)",
        )
        .bind(input.run_id())
        .bind(claim.job.owner_user_id)
        .bind(claim.job.id)
        .execute(&mut **transaction)
        .await
        .map_err(database_error)?
    } else {
        sqlx::query(
            "UPDATE public.owner_beta_recommendation_runs \
             SET status = $3, factor_snapshot_sha256 = NULL, target_snapshot_sha256 = NULL, \
                 cash_weight = NULL, error_code = $4, started_at = COALESCE(started_at, now()), \
                 finished_at = now(), updated_at = now() \
             WHERE id = $1 AND owner_user_id = $2 AND job_id = $5 AND status = 'PENDING' \
               AND NOT EXISTS (SELECT 1 FROM public.owner_beta_recommendation_items \
                               WHERE recommendation_run_id = $1 AND owner_user_id = $2)",
        )
        .bind(input.run_id())
        .bind(claim.job.owner_user_id)
        .bind(run_status)
        .bind(error_code)
        .bind(claim.job.id)
        .execute(&mut **transaction)
        .await
        .map_err(database_error)?
    };
    if updated.rows_affected() != 1 {
        return Err(OwnerBetaPublicationError::Integrity);
    }
    Ok(outcome)
}

fn validate_success_inputs(
    claim: &ClaimedJob,
    input: &OwnerBetaPriceRecommendationInput,
    factor: &PriceOnlyFactorSnapshot,
    target: &OwnerBetaTargetSnapshot,
) -> Result<(), OwnerBetaPublicationError> {
    if claim.job.job_type != OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE
        || serde_json::to_value(input).ok().as_ref() != Some(&claim.job.payload_json)
    {
        return Err(OwnerBetaPublicationError::Integrity);
    }
    input
        .validate_strategy_snapshot()
        .map_err(|_| OwnerBetaPublicationError::Integrity)?;
    input
        .validate_factor_snapshot(factor)
        .map_err(|_| OwnerBetaPublicationError::Integrity)?;
    if factor
        .compute_hash()
        .map_err(|_| OwnerBetaPublicationError::Integrity)?
        != factor.hash
    {
        return Err(OwnerBetaPublicationError::Integrity);
    }
    target
        .validate_hash()
        .map_err(|_| OwnerBetaPublicationError::Integrity)?;
    let strategy = input.strategy_snapshot();
    if target.schema() != OWNER_BETA_TARGET_SNAPSHOT_SCHEMA
        || target.hash_algorithm() != OWNER_BETA_TARGET_HASH_ALGORITHM
        || target.input_kind() != PRICE_ONLY_INPUT_KIND
        || target.capability() != PRICE_ONLY_CAPABILITY
        || target.as_of() != input.as_of()
        || target.strategy_id() != strategy.strategy_id()
        || target.strategy_version() != strategy.strategy_version()
        || target.strategy_config_sha256() != strategy.config_sha256()
        || target.factor_snapshot_sha256() != &factor.hash
        || target.pins() != input.pins()
    {
        return Err(OwnerBetaPublicationError::Integrity);
    }
    validate_target_items(target)
}

fn validate_target_items(
    target: &OwnerBetaTargetSnapshot,
) -> Result<(), OwnerBetaPublicationError> {
    let expected: BTreeSet<String> = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| [*symbol, ".KRX"].concat())
        .collect();
    let actual: BTreeSet<&str> = target
        .items()
        .iter()
        .map(|item| item.instrument_id())
        .collect();
    if target.items().len() != KR_ETF_CORE_SYMBOLS.len()
        || actual.len() != target.items().len()
        || actual.iter().copied().collect::<BTreeSet<_>>()
            != expected.iter().map(String::as_str).collect()
    {
        return Err(OwnerBetaPublicationError::Integrity);
    }

    let mut seen_ranks = BTreeSet::new();
    let mut total = target.cash_weight_ppm();
    if !(0..=OWNER_BETA_TARGET_WEIGHT_SCALE).contains(&total) {
        return Err(OwnerBetaPublicationError::Integrity);
    }
    for item in target.items() {
        let has_weight = item.target_weight_ppm().is_some();
        match (has_weight, item.rank()) {
            (true, Some(rank)) if (1..=KR_ETF_CORE_SYMBOLS.len() as u32).contains(&rank) => {
                if !seen_ranks.insert(rank) {
                    return Err(OwnerBetaPublicationError::Integrity);
                }
            }
            (false, None) => {}
            _ => return Err(OwnerBetaPublicationError::Integrity),
        }
        let weight = item.target_weight_ppm().unwrap_or_default();
        if !(0..=OWNER_BETA_TARGET_WEIGHT_SCALE).contains(&weight) {
            return Err(OwnerBetaPublicationError::Integrity);
        }
        total = total
            .checked_add(weight)
            .ok_or(OwnerBetaPublicationError::Integrity)?;
        if item.reasons().is_empty() || item.reasons().len() > 16 || item.factors().len() > 64 {
            return Err(OwnerBetaPublicationError::Integrity);
        }
        for reason in item.reasons() {
            let code = reason.code();
            if code.is_empty()
                || code.len() > 64
                || !code
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte == b'_')
            {
                return Err(OwnerBetaPublicationError::Integrity);
            }
        }
        for (key, value) in item.factors() {
            if key.is_empty() || key.len() > 64 || value.is_empty() || value.len() > 64 {
                return Err(OwnerBetaPublicationError::Integrity);
            }
        }
    }
    if total != OWNER_BETA_TARGET_WEIGHT_SCALE {
        return Err(OwnerBetaPublicationError::Integrity);
    }
    Ok(())
}

fn validate_locked_job(
    job: &crate::types::Job,
    claim: &ClaimedJob,
    input: &OwnerBetaPriceRecommendationInput,
) -> Result<(), OwnerBetaPublicationError> {
    let payload = serde_json::to_value(input).map_err(|_| OwnerBetaPublicationError::Integrity)?;
    if job.id != claim.job.id
        || job.owner_user_id != claim.job.owner_user_id
        || job.job_type != OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE
        || job.payload_json != payload
        || job.attempt_count != claim.job.attempt_count
    {
        return Err(OwnerBetaPublicationError::Integrity);
    }
    Ok(())
}

fn validate_failure_claim(
    claim: &ClaimedJob,
    input: &OwnerBetaPriceRecommendationInput,
) -> Result<(), OwnerBetaPublicationError> {
    let payload = serde_json::to_value(input).map_err(|_| OwnerBetaPublicationError::Integrity)?;
    if claim.job.job_type != OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE
        || claim.job.payload_json != payload
    {
        return Err(OwnerBetaPublicationError::Integrity);
    }
    Ok(())
}

fn validate_locked_run(
    run: &LockedRun,
    claim: &ClaimedJob,
    input: &OwnerBetaPriceRecommendationInput,
) -> Result<(), OwnerBetaPublicationError> {
    let strategy = input.strategy_snapshot();
    let pins = input.pins();
    if run.status != "PENDING"
        || run.owner_user_id != claim.job.owner_user_id
        || run.strategy_config_id != input.strategy_config_id()
        || run.strategy_id != strategy.strategy_id()
        || run.strategy_version != strategy.strategy_version()
        || run.strategy_config_json != *strategy.config_json()
        || run.strategy_config_sha256 != strategy.config_sha256().as_str()
        || run.as_of != input.as_of().as_naive_date()
        || run.job_id != claim.job.id
        || run.input_kind != PRICE_ONLY_INPUT_KIND
        || run.capability != PRICE_ONLY_CAPABILITY
        || run.candidate_content_sha256 != pins.candidate_content_sha256().as_str()
        || run.artifact_manifest_sha256 != pins.artifact_manifest_sha256().as_str()
        || run.stage5_manifest_sha256 != pins.stage5_manifest_sha256().as_str()
        || run.action_manifest_sha256 != pins.action_manifest_sha256().as_str()
        || run.approval_registry_sha256 != pins.approval_registry_sha256().as_str()
        || run.factor_snapshot_sha256.is_some()
        || run.target_snapshot_sha256.is_some()
        || run.cash_weight.is_some()
        || run.error_code.is_some()
    {
        return Err(OwnerBetaPublicationError::Integrity);
    }
    Ok(())
}

fn database_error(error: sqlx::Error) -> OwnerBetaPublicationError {
    match database_error_class(&error) {
        ErrorClass::Transient => OwnerBetaPublicationError::DatabaseTransient,
        ErrorClass::Input
        | ErrorClass::DataBlocked
        | ErrorClass::Integrity
        | ErrorClass::Determinism => OwnerBetaPublicationError::DatabaseIntegrity,
    }
}

fn queue_error(error: QueueError) -> OwnerBetaPublicationError {
    match error {
        QueueError::StaleClaim(_) | QueueError::AlreadyTerminal(_, _) => {
            OwnerBetaPublicationError::QueueClaimLost
        }
        other => match queue_error_class(&other) {
            ErrorClass::Transient => OwnerBetaPublicationError::QueueTransient,
            ErrorClass::Input
            | ErrorClass::DataBlocked
            | ErrorClass::Integrity
            | ErrorClass::Determinism => OwnerBetaPublicationError::QueueIntegrity,
        },
    }
}

// Daily settlement deliberately has a separate input binding. Requeued runs
// remain byte-identical: 0060 rejects even timestamp-only PENDING updates.
#[derive(sqlx::FromRow)]
struct LockedDailyRun {
    status: String,
    owner_user_id: uuid::Uuid,
    strategy_config_id: uuid::Uuid,
    strategy_id: String,
    strategy_version: String,
    strategy_config_json: Value,
    strategy_config_sha256: String,
    as_of: chrono::NaiveDate,
    job_id: uuid::Uuid,
    input_kind: String,
    trigger_kind: String,
    capability: String,
    daily_pins: Value,
    daily_config_epoch: i64,
    daily_activation_epoch: i64,
    daily_trigger_version: String,
    historical_pins_absent: bool,
    factor_snapshot_sha256: Option<String>,
    target_snapshot_sha256: Option<String>,
    cash_weight: Option<String>,
    error_code: Option<String>,
    started_at: Option<chrono::DateTime<chrono::Utc>>,
    finished_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Settle a daily claim even when current source/config authority has been
/// revoked. Current authority permits success; an exact fresh claim still
/// needs to release capacity on failure. Stale claims remain recovery-owned.
pub async fn settle_owner_beta_daily_failure(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
    input: &super::daily_input::OwnerBetaDailyRecommendationInputV1,
    failure: OwnerBetaPublicationFailure,
) -> Result<OwnerBetaPublicationOutcome, OwnerBetaPublicationError> {
    validate_daily_claim(claim, input)?;
    let mut transaction = pool.begin().await.map_err(database_error)?;
    let result = settle_daily_failure_in(&mut transaction, queue, claim, input, failure).await;
    match result {
        Ok(outcome) => transaction
            .commit()
            .await
            .map(|()| outcome)
            .map_err(|_| OwnerBetaPublicationError::CommitUnknown),
        Err(error) => {
            let _ = transaction.rollback().await;
            Err(error)
        }
    }
}

fn validate_daily_claim(
    claim: &ClaimedJob,
    input: &super::daily_input::OwnerBetaDailyRecommendationInputV1,
) -> Result<(), OwnerBetaPublicationError> {
    input
        .validate()
        .map_err(|_| OwnerBetaPublicationError::Integrity)?;
    let payload = serde_json::to_value(input).map_err(|_| OwnerBetaPublicationError::Integrity)?;
    if claim.job.job_type != OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE
        || claim.job.payload_json != payload
    {
        return Err(OwnerBetaPublicationError::Integrity);
    }
    Ok(())
}

async fn lock_daily_run(
    transaction: &mut Transaction<'_, Postgres>,
    claim: &ClaimedJob,
    input: &super::daily_input::OwnerBetaDailyRecommendationInputV1,
) -> Result<LockedDailyRun, OwnerBetaPublicationError> {
    let run: LockedDailyRun=sqlx::query_as(
        "SELECT status, owner_user_id, strategy_config_id, strategy_id, strategy_version, \
                strategy_config_json, strategy_config_sha256, as_of, job_id, input_kind, trigger_kind, capability, \
                daily_pins, daily_config_epoch, daily_activation_epoch, daily_trigger_version, \
                candidate_content_sha256 IS NULL AND artifact_manifest_sha256 IS NULL \
                  AND stage5_manifest_sha256 IS NULL AND action_manifest_sha256 IS NULL \
                  AND approval_registry_sha256 IS NULL AS historical_pins_absent, \
                factor_snapshot_sha256,target_snapshot_sha256,cash_weight::text AS cash_weight, \
                error_code,started_at,finished_at \
         FROM public.owner_beta_recommendation_runs WHERE id=$1 FOR UPDATE")
        .bind(input.run_id()).fetch_optional(&mut **transaction).await.map_err(database_error)?
        .ok_or(OwnerBetaPublicationError::Integrity)?;
    let strategy = input.strategy_snapshot();
    let pins = input.pins();
    let expected_pins = serde_json::json!({
        "content_sha256":pins.content_sha256(),"manifest_sha256":pins.manifest_sha256(),
        "admission_sha256":pins.admission_sha256(),"policy_sha256":pins.policy_sha256(),
    });
    if run.status != "PENDING"
        || run.owner_user_id != claim.job.owner_user_id
        || run.strategy_config_id != input.strategy_config_id()
        || run.strategy_id != strategy.strategy_id()
        || run.strategy_version != strategy.strategy_version()
        || run.strategy_config_json != *strategy.config_json()
        || run.strategy_config_sha256 != strategy.config_sha256().as_str()
        || run.as_of != input.as_of().as_naive_date()
        || run.job_id != claim.job.id
        || run.input_kind != factor_engine::daily_price_only::DAILY_PRICE_ONLY_INPUT_KIND
        || run.trigger_kind != "DAILY"
        || run.capability != PRICE_ONLY_CAPABILITY
        || run.daily_pins != expected_pins
        || run.daily_config_epoch != input.config_epoch()
        || run.daily_activation_epoch != input.activation_epoch()
        || run.daily_trigger_version != super::daily_input::OWNER_BETA_DAILY_TRIGGER_VERSION
        || !run.historical_pins_absent
        || run.factor_snapshot_sha256.is_some()
        || run.target_snapshot_sha256.is_some()
        || run.cash_weight.is_some()
        || run.error_code.is_some()
        || run.started_at.is_some()
        || run.finished_at.is_some()
    {
        return Err(OwnerBetaPublicationError::Integrity);
    }
    let items:i64=sqlx::query_scalar(
        "SELECT count(*) FROM public.owner_beta_recommendation_items WHERE recommendation_run_id=$1")
        .bind(input.run_id()).fetch_one(&mut **transaction).await.map_err(database_error)?;
    if items != 0 {
        return Err(OwnerBetaPublicationError::Integrity);
    }
    Ok(run)
}

async fn settle_daily_failure_in(
    transaction: &mut Transaction<'_, Postgres>,
    queue: &JobQueue,
    claim: &ClaimedJob,
    input: &super::daily_input::OwnerBetaDailyRecommendationInputV1,
    failure: OwnerBetaPublicationFailure,
) -> Result<OwnerBetaPublicationOutcome, OwnerBetaPublicationError> {
    // Existing queue methods lock and fence job/attempt before the run.
    let status = if failure == OwnerBetaPublicationFailure::Canceled {
        queue
            .settle_aborted_in(transaction, claim, failure.message())
            .await
            .map_err(queue_error)?
            .status
    } else {
        match queue
            .settle_failure_in(
                transaction,
                claim,
                failure.class(),
                failure.code(),
                failure.message(),
            )
            .await
            .map_err(queue_error)?
        {
            SettleResult::Committed(job) | SettleResult::Canceled(job) => job.status,
        }
    };
    let locked: (uuid::Uuid, String, Value, i32) = sqlx::query_as(
        "SELECT owner_user_id,job_type,payload_json,attempt_count FROM public.jobs WHERE id=$1",
    )
    .bind(claim.job.id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(database_error)?;
    if locked.0 != claim.job.owner_user_id
        || locked.1 != OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE
        || locked.2 != claim.job.payload_json
        || locked.3 != claim.job.attempt_count
        || locked.3 != claim.attempt.attempt_no
    {
        return Err(OwnerBetaPublicationError::Integrity);
    }
    let _run = lock_daily_run(transaction, claim, input).await?;
    let (outcome, run_status, error_code) = match status {
        JobStatus::Queued => return Ok(OwnerBetaPublicationOutcome::Retrying),
        JobStatus::Failed => (
            OwnerBetaPublicationOutcome::Failed,
            "FAILED",
            failure.code(),
        ),
        JobStatus::Canceled => (
            OwnerBetaPublicationOutcome::Canceled,
            "CANCELED",
            "CANCELED",
        ),
        JobStatus::Running | JobStatus::Succeeded => {
            return Err(OwnerBetaPublicationError::QueueIntegrity);
        }
    };
    if status == JobStatus::Canceled {
        // Generic cancellation settlement records the attempt but deliberately
        // retains its lease. Daily consistency requires its fenced release in
        // this same transaction before the run mirror can commit.
        let cleared = sqlx::query(
            "UPDATE public.jobs SET locked_by=NULL,locked_at=NULL,updated_at=now() \
             WHERE id=$1 AND owner_user_id=$2 AND status='CANCELED' \
               AND locked_by=$3 AND attempt_count=$4",
        )
        .bind(claim.job.id)
        .bind(claim.job.owner_user_id)
        .bind(&claim.worker_id)
        .bind(claim.attempt.attempt_no)
        .execute(&mut **transaction)
        .await
        .map_err(database_error)?;
        if cleared.rows_affected() != 1 {
            return Err(OwnerBetaPublicationError::QueueIntegrity);
        }
    }
    let updated=sqlx::query(
        "UPDATE public.owner_beta_recommendation_runs \
         SET status=$3,error_code=$4,started_at=COALESCE(started_at,now()),finished_at=now(),updated_at=now() \
         WHERE id=$1 AND owner_user_id=$2 AND job_id=$5 AND status='PENDING'")
        .bind(input.run_id()).bind(claim.job.owner_user_id).bind(run_status).bind(error_code)
        .bind(claim.job.id).execute(&mut **transaction).await.map_err(database_error)?;
    if updated.rows_affected() != 1 {
        return Err(OwnerBetaPublicationError::Integrity);
    }
    Ok(outcome)
}

/// A momentary DB check, never a retained publication capability. Success must
/// repeat the full gate inside its atomic publication transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DailyClaimPreflight {
    Ready,
    Canceled,
}

pub async fn preflight_owner_beta_daily_claim(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
    input: &super::daily_input::OwnerBetaDailyRecommendationInputV1,
) -> Result<DailyClaimPreflight, OwnerBetaPublicationError> {
    validate_daily_claim(claim, input)?;
    let mut transaction = pool.begin().await.map_err(database_error)?;
    let result = preflight_daily_claim_in(&mut transaction, queue, claim, input).await;
    match result {
        Ok(outcome) => transaction
            .commit()
            .await
            .map(|()| outcome)
            .map_err(|_| OwnerBetaPublicationError::CommitUnknown),
        Err(error) => {
            let _ = transaction.rollback().await;
            Err(error)
        }
    }
}

async fn preflight_daily_claim_in(
    transaction: &mut Transaction<'_, Postgres>,
    queue: &JobQueue,
    claim: &ClaimedJob,
    input: &super::daily_input::OwnerBetaDailyRecommendationInputV1,
) -> Result<DailyClaimPreflight, OwnerBetaPublicationError> {
    // Runtime uses one direct worker principal, never an elevated/SET ROLE pool.
    let direct_worker: bool = sqlx::query_scalar(
        "SELECT session_user::text='worker' AND current_user::text='worker' \
         AND NOT rolsuper AND NOT rolcreatedb AND NOT rolcreaterole \
         AND NOT rolreplication AND NOT rolbypassrls \
         FROM pg_catalog.pg_roles WHERE rolname=current_user",
    )
    .fetch_one(&mut **transaction)
    .await
    .map_err(database_error)?;
    if !direct_worker {
        return Err(OwnerBetaPublicationError::DailyAuthorityDenied);
    }
    let job = match queue.lock_claim_in(transaction, claim).await {
        Ok(job) => job,
        Err(QueueError::StaleClaim(_)) => {
            return match settle_daily_failure_in(
                transaction,
                queue,
                claim,
                input,
                OwnerBetaPublicationFailure::Canceled,
            )
            .await?
            {
                OwnerBetaPublicationOutcome::Canceled => Ok(DailyClaimPreflight::Canceled),
                _ => Err(OwnerBetaPublicationError::QueueIntegrity),
            };
        }
        Err(error) => return Err(queue_error(error)),
    };
    if job.id != claim.job.id
        || job.owner_user_id != claim.job.owner_user_id
        || job.job_type != OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE
        || job.payload_json != claim.job.payload_json
        || job.attempt_count != claim.job.attempt_count
    {
        return Err(OwnerBetaPublicationError::Integrity);
    }
    lock_daily_run(transaction, claim, input).await?;
    let allowed: bool = sqlx::query_scalar("SELECT public.lock_owner_beta_daily_run_gate($1)")
        .bind(input.run_id())
        .fetch_one(&mut **transaction)
        .await
        .map_err(daily_gate_error)?;
    if !allowed {
        return Err(OwnerBetaPublicationError::DailyAuthorityDenied);
    }
    Ok(DailyClaimPreflight::Ready)
}

/// Publish a daily computation only while its exact reopened file capability
/// remains valid. Serialized factors/targets alone cannot enter this boundary.
/// Current SQL authority is rechecked under the same transaction as all writes.
pub async fn publish_owner_beta_daily_success(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
    input: &super::daily_input::OwnerBetaDailyRecommendationInputV1,
    admitted: &market_data::daily_price_only::input::AdmittedDailyPriceInputV1<'_, '_>,
    computation: &super::daily_compute::OwnerBetaDailyComputation,
) -> Result<OwnerBetaPublicationOutcome, OwnerBetaPublicationError> {
    validate_daily_claim(claim, input)?;
    input
        .validate_admitted(admitted)
        .map_err(|_| OwnerBetaPublicationError::Integrity)?;
    validate_daily_computation(
        input,
        computation.factor_snapshot(),
        computation.target_snapshot(),
    )?;
    let mut transaction = pool.begin().await.map_err(database_error)?;
    let result =
        publish_daily_success_in(&mut transaction, queue, claim, input, admitted, computation)
            .await;
    match result {
        Ok(outcome) => {
            // A failed final descriptor check rolls back every output and queue
            // transition. An uncertain commit is never retried inline.
            if outcome == OwnerBetaPublicationOutcome::Published
                && input.validate_admitted(admitted).is_err()
            {
                let _ = transaction.rollback().await;
                return Err(OwnerBetaPublicationError::Integrity);
            }
            transaction
                .commit()
                .await
                .map(|()| outcome)
                .map_err(|_| OwnerBetaPublicationError::CommitUnknown)
        }
        Err(error) => {
            let _ = transaction.rollback().await;
            Err(error)
        }
    }
}

fn validate_daily_computation(
    input: &super::daily_input::OwnerBetaDailyRecommendationInputV1,
    factor: &factor_engine::daily_price_only::DailyPriceOnlyFactorSnapshot,
    target: &super::daily_target::OwnerBetaDailyTargetSnapshotV1,
) -> Result<(), OwnerBetaPublicationError> {
    input
        .validate()
        .map_err(|_| OwnerBetaPublicationError::Integrity)?;
    input
        .validate_factor_snapshot(factor)
        .map_err(|_| OwnerBetaPublicationError::Integrity)?;
    target
        .validate_hash()
        .map_err(|_| OwnerBetaPublicationError::Integrity)?;
    let expected = super::daily_target::build_daily_target_snapshot(input, factor)
        .map_err(|_| OwnerBetaPublicationError::Integrity)?;
    if &expected != target {
        return Err(OwnerBetaPublicationError::Integrity);
    }
    Ok(())
}

fn daily_item_weight(item: &super::OwnerBetaTargetItem) -> String {
    // 0060 requires excluded daily items to have numeric zero; historical
    // publication retains its existing NULL representation.
    item.target_weight()
        .unwrap_or_else(|| "0.000000".to_owned())
}

fn daily_gate_error(error: sqlx::Error) -> OwnerBetaPublicationError {
    if error
        .as_database_error()
        .and_then(|error| error.code())
        .as_deref()
        == Some("23514")
    {
        OwnerBetaPublicationError::DailyAuthorityDenied
    } else {
        database_error(error)
    }
}

async fn publish_daily_success_in(
    transaction: &mut Transaction<'_, Postgres>,
    queue: &JobQueue,
    claim: &ClaimedJob,
    input: &super::daily_input::OwnerBetaDailyRecommendationInputV1,
    admitted: &market_data::daily_price_only::input::AdmittedDailyPriceInputV1<'_, '_>,
    computation: &super::daily_compute::OwnerBetaDailyComputation,
) -> Result<OwnerBetaPublicationOutcome, OwnerBetaPublicationError> {
    let job = match queue.lock_claim_in(transaction, claim).await {
        Ok(job) => job,
        Err(QueueError::StaleClaim(_)) => {
            return settle_daily_failure_in(
                transaction,
                queue,
                claim,
                input,
                OwnerBetaPublicationFailure::Canceled,
            )
            .await;
        }
        Err(error) => return Err(queue_error(error)),
    };
    if job.id != claim.job.id
        || job.owner_user_id != claim.job.owner_user_id
        || job.job_type != OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE
        || job.payload_json != claim.job.payload_json
        || job.attempt_count != claim.job.attempt_count
    {
        return Err(OwnerBetaPublicationError::Integrity);
    }
    lock_daily_run(transaction, claim, input).await?;
    let allowed: bool = sqlx::query_scalar("SELECT public.lock_owner_beta_daily_run_gate($1)")
        .bind(input.run_id())
        .fetch_one(&mut **transaction)
        .await
        .map_err(daily_gate_error)?;
    if !allowed {
        return Err(OwnerBetaPublicationError::DailyAuthorityDenied);
    }
    input
        .validate_admitted(admitted)
        .map_err(|_| OwnerBetaPublicationError::Integrity)?;
    let target = computation.target_snapshot();
    for item in target.items() {
        let excluded = item.target_weight_ppm().is_none();
        let reasons = Value::Array(
            item.reasons()
                .iter()
                .map(|reason| Value::String(reason.code().to_owned()))
                .collect(),
        );
        let factors = serde_json::to_value(item.factors())
            .map_err(|_| OwnerBetaPublicationError::Integrity)?;
        let rank = item
            .rank()
            .map(i32::try_from)
            .transpose()
            .map_err(|_| OwnerBetaPublicationError::Integrity)?;
        let exclusion = if excluded {
            Some(
                item.reasons()
                    .first()
                    .ok_or(OwnerBetaPublicationError::Integrity)?
                    .code(),
            )
        } else {
            None
        };
        let changed = sqlx::query(
            "INSERT INTO public.owner_beta_recommendation_items
             (recommendation_run_id,owner_user_id,instrument_id,rank,target_weight,reason_codes,factors_json,excluded,exclusion_reason)
             VALUES($1,$2,$3,$4,$5::numeric,$6,$7,$8,$9)")
            .bind(input.run_id()).bind(claim.job.owner_user_id).bind(item.instrument_id())
            .bind(rank).bind(daily_item_weight(item)).bind(reasons).bind(factors).bind(excluded).bind(exclusion)
            .execute(&mut **transaction).await.map_err(database_error)?;
        if changed.rows_affected() != 1 {
            return Err(OwnerBetaPublicationError::Integrity);
        }
    }
    input
        .validate_admitted(admitted)
        .map_err(|_| OwnerBetaPublicationError::Integrity)?;
    // The daily run trigger rechecks authority with clock_timestamp() and owns
    // started_at/finished_at/updated_at, including waits across the KST cutoff.
    let changed = sqlx::query(
        "UPDATE public.owner_beta_recommendation_runs
         SET status='SUCCEEDED',factor_snapshot_sha256=$3,target_snapshot_sha256=$4,cash_weight=$5::numeric,error_code=NULL
         WHERE id=$1 AND owner_user_id=$2 AND status='PENDING'")
        .bind(input.run_id()).bind(claim.job.owner_user_id)
        .bind(computation.factor_snapshot().hash.as_str()).bind(target.target_snapshot_sha256().as_str())
        .bind(target.cash_weight()).execute(&mut **transaction).await.map_err(database_error)?;
    if changed.rows_affected() != 1 {
        return Err(OwnerBetaPublicationError::Integrity);
    }
    match queue
        .settle_success_in(transaction, claim)
        .await
        .map_err(queue_error)?
    {
        SettleResult::Committed(job) if job.status == JobStatus::Succeeded => {
            Ok(OwnerBetaPublicationOutcome::Published)
        }
        _ => Err(OwnerBetaPublicationError::QueueClaimLost),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_codes_are_closed_uppercase_and_bounded() {
        for failure in [
            OwnerBetaPublicationFailure::InputInvalid,
            OwnerBetaPublicationFailure::EntitlementDenied,
            OwnerBetaPublicationFailure::DailyAuthorityDenied,
            OwnerBetaPublicationFailure::DailyExecutionDisabled,
            OwnerBetaPublicationFailure::FactorInvalid,
            OwnerBetaPublicationFailure::TargetInvalid,
            OwnerBetaPublicationFailure::PublicationUnavailable,
            OwnerBetaPublicationFailure::ComputationUnavailable,
            OwnerBetaPublicationFailure::ComputationFailed,
            OwnerBetaPublicationFailure::Canceled,
        ] {
            assert!(failure.code().len() <= 64);
            assert!(
                failure
                    .code()
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte == b'_')
            );
        }
    }

    #[test]
    fn malformed_claim_settles_queue_before_optional_run_mirror() {
        let source = include_str!("publish.rs");
        let malformed = source
            .split("async fn settle_malformed_claim_in")
            .nth(1)
            .expect("malformed claim seam");
        let settled = malformed
            .find(".settle_failure_in(")
            .expect("queue settles malformed input");
        let linked = malformed
            .find("WHERE job_id = $1 AND owner_user_id = $2 FOR UPDATE")
            .expect("linked run is selected by job and owner");
        assert!(settled < linked);
        assert!(malformed.contains("OwnerBetaPublicationFailure::InputInvalid.code()"));
        assert!(
            malformed.contains("NOT EXISTS (SELECT 1 FROM public.owner_beta_recommendation_items")
        );
        assert!(source.contains("settle_malformed_owner_beta_claim"));
        assert!(source.contains("Err(_) => Err(OwnerBetaPublicationError::CommitUnknown)"));
    }

    #[test]
    fn an_unknown_commit_is_never_classified_for_settlement_retry() {
        assert_eq!(
            OwnerBetaPublicationError::CommitUnknown.class(),
            ErrorClass::Integrity
        );
        assert!(!OwnerBetaPublicationError::CommitUnknown.class().retryable());
    }

    #[test]
    fn errors_are_static_and_redacted() {
        for error in [
            OwnerBetaPublicationError::DatabaseTransient,
            OwnerBetaPublicationError::DatabaseIntegrity,
            OwnerBetaPublicationError::QueueTransient,
            OwnerBetaPublicationError::QueueClaimLost,
            OwnerBetaPublicationError::QueueIntegrity,
            OwnerBetaPublicationError::Integrity,
            OwnerBetaPublicationError::EntitlementDenied,
            OwnerBetaPublicationError::CommitUnknown,
        ] {
            let text = error.to_string();
            let debug = format!("{error:?}");
            for forbidden in ["sha256:", "/", "config", "pin", "SELECT", "INSERT"] {
                assert!(!text.contains(forbidden));
                assert!(!debug.contains(forbidden));
            }
        }
    }

    #[test]
    fn publisher_is_dedicated_and_locks_queue_before_run() {
        let source = include_str!("publish.rs");
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("production source");
        let success = production
            .split("async fn publish_success_in")
            .nth(1)
            .expect("success publisher");
        let queue_lock = success
            .find("queue.lock_claim_in(transaction, claim)")
            .expect("queue lock");
        let canceled_mirror = success
            .find("OwnerBetaPublicationFailure::Canceled")
            .expect("canceled run mirror");
        let run_lock = success
            .find("owner_beta_recommendation_runs")
            .expect("run table");
        assert!(queue_lock < run_lock);
        assert!(queue_lock < canceled_mirror);
        assert!(canceled_mirror < run_lock);
        assert!(!production.contains("paper_"));
        assert!(!production.contains("curated"));
        assert!(production.contains("owner_beta_recommendation_items"));
    }

    #[test]
    fn canonical_etf11_projection_is_exact() {
        let expected: BTreeSet<String> = KR_ETF_CORE_SYMBOLS
            .iter()
            .map(|symbol| [*symbol, ".KRX"].concat())
            .collect();
        assert_eq!(expected.len(), 11);
        assert!(
            expected
                .iter()
                .all(|instrument| instrument.ends_with(".KRX"))
        );
    }
}

#[cfg(test)]
mod daily_success_tests {
    // Synthetic computation-only fixtures. They cannot construct an admitted
    // input, enter the public publisher or prove filesystem/SQL integration.
    use super::super::daily_input::OwnerBetaDailyRecommendationInputV1;
    use super::super::daily_target::{OwnerBetaDailyTargetSnapshotV1, build_daily_target_snapshot};
    use super::*;
    use crate::resolver::ResolvedConfig;
    use domain::{ContentHash, TradingDate};
    use factor_engine::daily_price_only::{
        DAILY_FACTOR_SNAPSHOT_SCHEMA, DAILY_PRICE_ONLY_INPUT_KIND, DailyPriceOnlyFactorSnapshot,
    };
    use factor_engine::snapshot::{FactorRow, NormalizationMeta};
    use std::collections::BTreeMap;

    fn input(weight: f64) -> OwnerBetaDailyRecommendationInputV1 {
        let strategy=super::super::OwnerBetaStrategySnapshot::from_resolved_config(&ResolvedConfig {
            strategy_id:"buy_and_hold".to_owned(),strategy_version:"1.0.0".to_owned(),
            config:serde_json::json!({"benchmark_instrument":"069500.KRX","target_weight":weight}),
        }).unwrap();
        serde_json::from_value(serde_json::json!({
            "schema":"owner-beta-daily-recommendation-v1",
            "run_id":"00000000-0000-4000-8000-000000000001",
            "strategy_config_id":"00000000-0000-4000-8000-000000000002","as_of":"2026-08-31",
            "input":{"kind":"owner_beta_daily_etf11_price_only_v1","content_sha256":ContentHash::from_bytes(b"content"),
                "manifest_sha256":ContentHash::from_bytes(b"manifest"),"admission_sha256":ContentHash::from_bytes(b"admission"),
                "policy_sha256":ContentHash::from_bytes(b"policy")},
            "strategy":strategy,
            "trigger":{"kind":"DAILY","version":"owner-beta-daily-trigger-v1","config_epoch":1,"activation_epoch":2}
        })).unwrap()
    }
    fn factor(input: &OwnerBetaDailyRecommendationInputV1) -> DailyPriceOnlyFactorSnapshot {
        let mut value = DailyPriceOnlyFactorSnapshot {
            schema: DAILY_FACTOR_SNAPSHOT_SCHEMA.to_owned(),
            input_kind: DAILY_PRICE_ONLY_INPUT_KIND.to_owned(),
            capability: PRICE_ONLY_CAPABILITY.to_owned(),
            as_of: input.as_of(),
            content_sha256: input.pins().content_sha256().to_string(),
            manifest_sha256: input.pins().manifest_sha256().to_string(),
            admission_sha256: input.pins().admission_sha256().to_string(),
            policy_sha256: input.pins().policy_sha256().to_string(),
            factor_versions: BTreeMap::new(),
            normalization: NormalizationMeta {
                id: "fixture".to_owned(),
                version: "1".to_owned(),
                params: BTreeMap::new(),
            },
            rows: Vec::new(),
            hash: ContentHash::from_bytes(b"placeholder"),
        };
        value.hash = value.compute_hash().unwrap();
        value
    }
    fn fixtures() -> (
        OwnerBetaDailyRecommendationInputV1,
        DailyPriceOnlyFactorSnapshot,
        OwnerBetaDailyTargetSnapshotV1,
    ) {
        let input = input(1.0);
        let factor = factor(&input);
        let target = build_daily_target_snapshot(&input, &factor).unwrap();
        (input, factor, target)
    }
    #[test]
    fn daily_publication_validation_binds_exact_computation() {
        let (input, factor, target) = fixtures();
        assert_eq!(validate_daily_computation(&input, &factor, &target), Ok(()));
    }
    #[test]
    fn daily_publication_validation_rejects_each_rehashed_foreign_pin() {
        let (input, factor, target) = fixtures();
        for index in 0..4 {
            let mut changed = factor.clone();
            let other = ContentHash::from_bytes(b"foreign").to_string();
            match index {
                0 => changed.content_sha256 = other,
                1 => changed.manifest_sha256 = other,
                2 => changed.admission_sha256 = other,
                _ => changed.policy_sha256 = other,
            }
            changed.hash = changed.compute_hash().unwrap();
            assert_eq!(
                validate_daily_computation(&input, &changed, &target),
                Err(OwnerBetaPublicationError::Integrity)
            );
        }
    }
    #[test]
    fn daily_publication_validation_rejects_stale_hash_and_date() {
        let (input, factor, target) = fixtures();
        let mut changed = factor.clone();
        changed.hash = ContentHash::from_bytes(b"stale");
        assert_eq!(
            validate_daily_computation(&input, &changed, &target),
            Err(OwnerBetaPublicationError::Integrity)
        );
        changed = factor;
        changed.as_of = TradingDate::parse("2026-09-01").unwrap();
        changed.hash = changed.compute_hash().unwrap();
        assert_eq!(
            validate_daily_computation(&input, &changed, &target),
            Err(OwnerBetaPublicationError::Integrity)
        );
    }
    #[test]
    fn daily_publication_validation_rejects_nonfinite_factor_rows() {
        let (input, mut factor, target) = fixtures();
        factor.rows.push(FactorRow {
            date: input.as_of().to_iso(),
            instrument: "069500.KRX".to_owned(),
            factor: "unexpected".to_owned(),
            raw: Some(f64::NAN),
            normalized: None,
        });
        assert_eq!(
            validate_daily_computation(&input, &factor, &target),
            Err(OwnerBetaPublicationError::Integrity)
        );
    }
    #[test]
    fn daily_publication_validation_rejects_other_valid_strategy_target() {
        let (original, factor, target) = fixtures();
        let other = input(0.5);
        let other_target = build_daily_target_snapshot(&other, &factor).unwrap();
        assert!(other_target.validate_hash().is_ok());
        assert_eq!(
            validate_daily_computation(&original, &factor, &other_target),
            Err(OwnerBetaPublicationError::Integrity)
        );
        assert_eq!(
            validate_daily_computation(&other, &factor, &target),
            Err(OwnerBetaPublicationError::Integrity)
        );
    }
    #[test]
    fn daily_publication_validation_rejects_changed_discriminants() {
        let (input, factor, target) = fixtures();
        for index in 0..3 {
            let mut changed = factor.clone();
            match index {
                0 => changed.schema = "wrong".to_owned(),
                1 => changed.input_kind = PRICE_ONLY_INPUT_KIND.to_owned(),
                _ => changed.capability = "wrong".to_owned(),
            }
            changed.hash = changed.compute_hash().unwrap();
            assert_eq!(
                validate_daily_computation(&input, &changed, &target),
                Err(OwnerBetaPublicationError::Integrity)
            );
        }
    }
    #[test]
    fn daily_excluded_projection_has_zero_weight_and_no_rank() {
        let (_, _, target) = fixtures();
        let mut selected = 0;
        let mut excluded = 0;
        for item in target.items() {
            if item.target_weight_ppm().is_none() {
                excluded += 1;
                assert_eq!(daily_item_weight(item), "0.000000");
                assert!(item.rank().is_none());
                assert!(!item.reasons().is_empty());
            } else {
                selected += 1;
                assert_eq!(daily_item_weight(item), "1.000000");
                assert_eq!(item.rank(), Some(1));
            }
        }
        assert_eq!((selected, excluded), (1, 10));
        assert_eq!(target.cash_weight(), "0.000000");
    }
    #[test]
    fn daily_authority_denial_is_value_free_and_not_retryable() {
        let error = OwnerBetaPublicationError::DailyAuthorityDenied;
        assert_eq!(error.class(), ErrorClass::DataBlocked);
        assert!(!error.class().retryable());
        assert_eq!(
            error.to_string(),
            "owner-beta daily authority is unavailable"
        );
        assert_eq!(format!("{error:?}"), error.to_string());
    }
}
