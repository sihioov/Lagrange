//! One-transaction publication of a validated recommendation portfolio.

use crate::error::QueueError;
use crate::queue::JobQueue;
use crate::recommendation::compute::AttestedUniverse;
use crate::recommendation::input::{AttestedDatasetStatus, AttestedRecommendationInput};
use crate::recommendation::validate::ValidatedPortfolio;
use crate::types::{ClaimedJob, ErrorClass, SettleResult};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationOutcome {
    Published,
    AlreadyPublished,
}

#[derive(Debug, Error)]
pub enum PublicationError {
    #[error("recommendation publication database unavailable: {0}")]
    Database(#[from] sqlx::Error),
    #[error("recommendation publication lost its queue claim: {0}")]
    Queue(#[from] QueueError),
    #[error("recommendation publication integrity failure: {detail}")]
    Integrity { detail: String },
}

impl PublicationError {
    pub const fn class(&self) -> ErrorClass {
        match self {
            Self::Database(_) => ErrorClass::Transient,
            Self::Queue(QueueError::Database(_)) => ErrorClass::Transient,
            Self::Queue(_) | Self::Integrity { .. } => ErrorClass::Integrity,
        }
    }

    pub const fn code(&self) -> &'static str {
        match self {
            Self::Database(_) | Self::Queue(QueueError::Database(_)) => {
                "RECOMMENDATION_PUBLISH_UNAVAILABLE"
            }
            Self::Queue(QueueError::StaleClaim(_)) => "RECOMMENDATION_PUBLISH_STALE_CLAIM",
            Self::Queue(_) => "RECOMMENDATION_PUBLISH_QUEUE_INTEGRITY",
            Self::Integrity { .. } => "RECOMMENDATION_PUBLISH_INTEGRITY",
        }
    }
}

#[derive(sqlx::FromRow)]
struct LockedRun {
    status: String,
    owner_user_id: uuid::Uuid,
    strategy_config_id: Option<uuid::Uuid>,
    as_of: chrono::NaiveDate,
    job_id: Option<uuid::Uuid>,
    trigger_kind: String,
    dataset_version_id: Option<uuid::Uuid>,
    dataset_manifest_sha256: Option<String>,
}

#[derive(sqlx::FromRow)]
struct PublishedRow {
    job_status: String,
    run_status: String,
    summary_json: Value,
    item_count: i64,
    portfolio_count: i64,
    cash_weight: String,
    weights_json: Value,
    attempt_outcome: String,
}

#[derive(sqlx::FromRow)]
struct CurrentDataset {
    dataset_id: String,
    version: String,
    status: String,
    manifest_sha256: String,
    storage_path: String,
}

/// Publish all result rows, the run transition and queue settlement under one
/// worker transaction. No database write is visible unless every guard and
/// write succeeds.
pub async fn publish_recommendation(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
    input: &AttestedRecommendationInput,
    universe: &AttestedUniverse,
    portfolio: &ValidatedPortfolio,
) -> Result<PublicationOutcome, PublicationError> {
    let mut transaction = pool.begin().await?;

    let locked_job = match queue.lock_claim_in(&mut transaction, claim).await {
        Ok(job) => job,
        Err(error) => {
            if matches!(error, QueueError::StaleClaim(_))
                && already_published(&mut transaction, claim, input, portfolio).await?
            {
                transaction.commit().await?;
                return Ok(PublicationOutcome::AlreadyPublished);
            }
            transaction.rollback().await?;
            return Err(PublicationError::Queue(error));
        }
    };

    validate_claim_identity(&locked_job, input)?;
    let run: LockedRun = sqlx::query_as(
        "SELECT status, owner_user_id, strategy_config_id, as_of, job_id, trigger_kind, \
                dataset_version_id, dataset_manifest_sha256 \
         FROM recommendation_runs \
         WHERE id = $1 AND owner_user_id = $2 FOR UPDATE",
    )
    .bind(input.payload.run_id)
    .bind(claim.job.owner_user_id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or_else(|| integrity("owned recommendation run is missing"))?;
    validate_run(&run, claim, input)?;
    reattest_current_rows(
        &mut transaction,
        claim.job.owner_user_id,
        input,
        universe,
        portfolio,
    )
    .await?;

    for item in &portfolio.items {
        sqlx::query(
            "INSERT INTO recommendation_items \
             (recommendation_run_id, owner_user_id, instrument_id, rank, target_weight, \
              reason_codes, factors_json, excluded, exclusion_reason) \
             VALUES ($1, $2, $3, $4, $5::numeric, $6, $7, $8, $9)",
        )
        .bind(input.payload.run_id)
        .bind(claim.job.owner_user_id)
        .bind(&item.instrument_id)
        .bind(item.rank)
        .bind(item.target_weight.as_deref())
        .bind(&item.reason_codes)
        .bind(&item.factors_json)
        .bind(item.excluded)
        .bind(&item.exclusion_reason)
        .execute(&mut *transaction)
        .await?;
    }

    let weights_json = serde_json::to_value(&portfolio.positive_weights)
        .map_err(|_| integrity("validated weights cannot be represented as JSON"))?;
    sqlx::query(
        "INSERT INTO target_portfolios \
         (owner_user_id, recommendation_run_id, universe_snapshot_id, as_of, cash_weight, weights_json) \
         VALUES ($1, $2, $3, $4, $5::numeric, $6)",
    )
    .bind(claim.job.owner_user_id)
    .bind(input.payload.run_id)
    .bind(&portfolio.universe_snapshot_id)
    .bind(input.payload.as_of)
    .bind(&portfolio.cash_weight)
    .bind(&weights_json)
    .execute(&mut *transaction)
    .await?;

    let warnings: Vec<&str> = match input.dataset.status {
        AttestedDatasetStatus::Ready => Vec::new(),
        AttestedDatasetStatus::Warning => vec!["DATASET_STATUS_WARNING"],
    };
    let summary = json!({
        "dataset_id": input.dataset.dataset_id,
        "dataset_version": input.dataset.version,
        "dataset_version_id": input.dataset.id,
        "curated_version": input.dataset.curated_version,
        "manifest_sha256": input.dataset.manifest_sha256,
        "universe_snapshot_id": portfolio.universe_snapshot_id,
        "factor_snapshot_hash": portfolio.factor_snapshot_hash,
        "portfolio_snapshot_id": portfolio.portfolio_snapshot_id,
        "selected_count": portfolio.selected_count,
        "excluded_count": portfolio.excluded_count,
        "cash_weight": portfolio.cash_weight,
        "trigger_kind": run.trigger_kind,
        "warnings": warnings,
        "portfolio_reasons": portfolio.portfolio_reasons,
    });
    let updated = sqlx::query(
        "UPDATE recommendation_runs SET status = 'SUCCEEDED', summary_json = $3 \
         WHERE id = $1 AND owner_user_id = $2 AND status = 'PENDING'",
    )
    .bind(input.payload.run_id)
    .bind(claim.job.owner_user_id)
    .bind(&summary)
    .execute(&mut *transaction)
    .await?;
    if updated.rows_affected() != 1 {
        return rollback_integrity(transaction, "recommendation run changed before publication")
            .await;
    }

    match queue.settle_success_in(&mut transaction, claim).await? {
        SettleResult::Committed(job) if job.status == crate::types::JobStatus::Succeeded => {}
        _ => {
            return rollback_integrity(transaction, "queue did not commit a successful settlement")
                .await;
        }
    }
    transaction.commit().await?;
    Ok(PublicationOutcome::Published)
}

fn validate_claim_identity(
    job: &crate::types::Job,
    input: &AttestedRecommendationInput,
) -> Result<(), PublicationError> {
    let payload = serde_json::to_value(&input.payload)
        .map_err(|_| integrity("attested payload cannot be represented as JSON"))?;
    if job.job_type != "recommendation" || job.payload_json != payload {
        return Err(integrity(
            "claim identity does not match attested recommendation input",
        ));
    }
    Ok(())
}

fn validate_run(
    run: &LockedRun,
    claim: &ClaimedJob,
    input: &AttestedRecommendationInput,
) -> Result<(), PublicationError> {
    if run.status != "PENDING"
        || run.owner_user_id != claim.job.owner_user_id
        || run.strategy_config_id != Some(input.payload.strategy_config_id)
        || run.as_of != input.payload.as_of
        || run.job_id != Some(claim.job.id)
        || !matches!(run.trigger_kind.as_str(), "MANUAL" | "SCHEDULED")
        || run.dataset_version_id != Some(input.dataset.id)
        || run.dataset_manifest_sha256.as_deref() != Some(input.dataset.manifest_sha256.as_str())
    {
        return Err(integrity(
            "recommendation run lineage changed before publication",
        ));
    }
    Ok(())
}

async fn reattest_current_rows(
    transaction: &mut Transaction<'_, Postgres>,
    owner_user_id: uuid::Uuid,
    input: &AttestedRecommendationInput,
    universe: &AttestedUniverse,
    portfolio: &ValidatedPortfolio,
) -> Result<(), PublicationError> {
    let config: Option<(String, String, Value)> = sqlx::query_as(
        "SELECT strategy_id, strategy_version, config_json FROM user_strategy_configs \
         WHERE id = $1 AND owner_user_id = $2 AND is_active",
    )
    .bind(input.payload.strategy_config_id)
    .bind(owner_user_id)
    .fetch_optional(&mut **transaction)
    .await?;
    if config
        != Some((
            input.resolved_config.strategy_id.clone(),
            input.resolved_config.strategy_version.clone(),
            input.resolved_config.config.clone(),
        ))
    {
        return Err(integrity(
            "strategy configuration changed before publication",
        ));
    }

    let dataset: Option<CurrentDataset> = sqlx::query_as(
        "SELECT dataset_id, version, status, manifest_sha256, storage_path \
         FROM dataset_versions WHERE id = $1",
    )
    .bind(input.dataset.id)
    .fetch_optional(&mut **transaction)
    .await?;
    let expected_status = match input.dataset.status {
        AttestedDatasetStatus::Ready => "READY",
        AttestedDatasetStatus::Warning => "WARNING",
    };
    if !matches!(
        dataset,
        Some(CurrentDataset {
            dataset_id,
            version,
            status,
            manifest_sha256,
            storage_path,
        }) if dataset_id == input.dataset.dataset_id
            && version == input.dataset.version
            && status == expected_status
            && manifest_sha256 == input.dataset.manifest_sha256
            && storage_path == input.dataset.storage_path
    ) {
        return Err(integrity("dataset lineage changed before publication"));
    }

    if portfolio.universe_snapshot_id != universe.snapshot_id {
        return Err(integrity(
            "validated portfolio universe changed before publication",
        ));
    }
    let stored_members: Option<Value> = sqlx::query_scalar(
        "SELECT instruments_json FROM universe_snapshots WHERE snapshot_id = $1",
    )
    .bind(&universe.snapshot_id)
    .fetch_optional(&mut **transaction)
    .await?;
    if stored_members != Some(json!(universe.members)) {
        return Err(integrity(
            "universe snapshot is missing or has different membership",
        ));
    }
    Ok(())
}

async fn already_published(
    transaction: &mut Transaction<'_, Postgres>,
    claim: &ClaimedJob,
    input: &AttestedRecommendationInput,
    portfolio: &ValidatedPortfolio,
) -> Result<bool, PublicationError> {
    let expected_payload = serde_json::to_value(&input.payload)
        .map_err(|_| integrity("attested payload cannot be represented as JSON"))?;
    let row: Option<PublishedRow> = sqlx::query_as(
        "SELECT j.status AS job_status, r.status AS run_status, r.summary_json, \
                (SELECT count(*) FROM recommendation_items i WHERE i.recommendation_run_id = r.id) AS item_count, \
                (SELECT count(*) FROM target_portfolios existing WHERE existing.recommendation_run_id = r.id) AS portfolio_count, \
                p.cash_weight::text AS cash_weight, p.weights_json, a.outcome AS attempt_outcome \
         FROM jobs j \
         JOIN recommendation_runs r ON r.job_id = j.id AND r.owner_user_id = j.owner_user_id \
         JOIN target_portfolios p ON p.recommendation_run_id = r.id AND p.owner_user_id = r.owner_user_id \
         JOIN job_attempts a ON a.job_id = j.id AND a.attempt_no = $5 \
         WHERE j.id = $1 AND j.owner_user_id = $2 AND r.id = $3 \
           AND r.strategy_config_id = $4 AND j.payload_json = $6 \
         FOR UPDATE OF j, r",
    )
    .bind(claim.job.id)
    .bind(claim.job.owner_user_id)
    .bind(input.payload.run_id)
    .bind(input.payload.strategy_config_id)
    .bind(claim.attempt.attempt_no)
    .bind(expected_payload)
    .fetch_optional(&mut **transaction)
    .await?;
    let Some(row) = row else {
        return Ok(false);
    };
    Ok(row.job_status == "SUCCEEDED"
        && row.run_status == "SUCCEEDED"
        && row.attempt_outcome == "SUCCEEDED"
        && row.item_count == 11
        && row.portfolio_count == 1
        && row
            .summary_json
            .get("portfolio_snapshot_id")
            .and_then(Value::as_str)
            == Some(portfolio.portfolio_snapshot_id.as_str())
        && row.cash_weight == portfolio.cash_weight
        && row.weights_json
            == serde_json::to_value(&portfolio.positive_weights)
                .map_err(|_| integrity("validated weights cannot be represented as JSON"))?)
}

async fn rollback_integrity<T>(
    transaction: Transaction<'_, Postgres>,
    detail: &str,
) -> Result<T, PublicationError> {
    transaction.rollback().await?;
    Err(integrity(detail))
}

fn integrity(detail: &str) -> PublicationError {
    PublicationError::Integrity {
        detail: detail.to_owned(),
    }
}
