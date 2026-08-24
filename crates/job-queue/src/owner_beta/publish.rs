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
            Self::CommitUnknown => "owner-beta publication commit outcome is unknown",
        }
    }

    pub const fn class(self) -> ErrorClass {
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

/// The only externally selectable owner-beta settlement causes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerBetaPublicationFailure {
    InputInvalid,
    EntitlementDenied,
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
            Self::EntitlementDenied => ErrorClass::DataBlocked,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_codes_are_closed_uppercase_and_bounded() {
        for failure in [
            OwnerBetaPublicationFailure::InputInvalid,
            OwnerBetaPublicationFailure::EntitlementDenied,
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
