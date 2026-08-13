//! Actor-scoped submission and reads for recommendation-to-Paper previews.

use crate::actor_tx::{actor_uuid, begin_actor_tx};
use crate::error::TenancyError;
use auth::entitlement::Actor;
use chrono::{DateTime, NaiveDate, Utc};
use domain::ContentHash;
use job_queue::paper_preview::PaperPreviewPayload;
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow)]
pub struct RebalancePreviewRow {
    pub id: Uuid,
    pub account_id: Uuid,
    pub recommendation_run_id: Uuid,
    pub target_portfolio_id: Uuid,
    pub strategy_config_id: Uuid,
    pub job_id: Uuid,
    pub status: String,
    pub price_basis: String,
    pub price_date: NaiveDate,
    pub proposed_effective_date: Option<NaiveDate>,
    pub dataset_version_id: Uuid,
    pub dataset_manifest_sha256: String,
    pub target_portfolio_sha256: String,
    pub preview_token: Option<String>,
    pub result_json: Option<Value>,
    pub error_json: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub applied_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct SubmitRebalancePreview {
    pub account_id: Uuid,
    pub recommendation_run_id: Uuid,
    pub idempotency_key: String,
    pub max_jobs_per_owner: u32,
    pub seoul_today: NaiveDate,
}

#[derive(Debug, Clone)]
pub struct SubmittedRebalancePreview {
    pub row: RebalancePreviewRow,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedRebalancePreview {
    pub preview_id: Uuid,
    pub pending_target_id: Uuid,
    pub effective_date: NaiveDate,
    pub source_kind: String,
    pub replayed: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ApplyRebalancePreviewError {
    #[error(transparent)]
    Tenancy(#[from] TenancyError),
    #[error("the preview is not ready")]
    NotReady,
    #[error("the preview inputs changed")]
    Stale,
    #[error("the preview conflicts with an existing target")]
    Conflict,
}

#[derive(Debug, thiserror::Error)]
pub enum SubmitRebalancePreviewError {
    #[error(transparent)]
    Tenancy(#[from] TenancyError),
    #[error("per-owner preview capacity exceeded")]
    CapacityExceeded,
    #[error("idempotency key was already used with different preview input")]
    IdempotencyMismatch,
    #[error("an active matching Paper binding is required")]
    BindingRequired,
    #[error("the recommendation run is not ready")]
    RunNotReady,
    #[error("the recommendation dataset is blocked")]
    DataBlocked,
    #[error("the recommendation entitlement is required")]
    EntitlementRequired,
}

#[derive(Debug, FromRow)]
struct LockedSubmission {
    outcome: String,
    target_portfolio_id: Option<Uuid>,
    strategy_config_id: Option<Uuid>,
    price_date: Option<NaiveDate>,
    dataset_version_id: Option<Uuid>,
    dataset_manifest_sha256: Option<String>,
    weights_json: Option<Value>,
}

#[derive(Debug, FromRow)]
struct ApplyIdentity {
    account_id: Uuid,
    status: String,
    preview_token: Option<String>,
}

#[derive(Debug, FromRow)]
struct LockedApplyInputs {
    status: String,
    preview_token: Option<String>,
    account_state_version: Option<i64>,
    account_state_sha256: Option<String>,
    target_portfolio_sha256: String,
    paper_state_version: i64,
    cash_running: Option<String>,
    cash_replayed: String,
    positions_json: Value,
    weights_json: Value,
}

#[derive(Debug, FromRow)]
struct AppliedBoundaryRow {
    outcome: String,
    pending_target_id: Option<Uuid>,
    effective_date: Option<NaiveDate>,
    source_kind: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RebalancePreviewRepo {
    pool: sqlx::PgPool,
}

impl RebalancePreviewRepo {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    pub async fn submit(
        &self,
        actor: &Actor,
        input: SubmitRebalancePreview,
    ) -> Result<SubmittedRebalancePreview, SubmitRebalancePreviewError> {
        let owner = actor_uuid(actor)?;
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        crate::repos::lock_owner_job_capacity(&mut tx, owner).await?;
        let queue_key = format!("paper-preview:manual:{}", input.idempotency_key);

        let existing_job: Option<(Uuid, String)> = sqlx::query_as(
            "SELECT id,job_type FROM jobs \
             WHERE owner_user_id=$1 AND idempotency_key=$2",
        )
        .bind(owner)
        .bind(&queue_key)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        if let Some((job_id, job_type)) = existing_job {
            if job_type != "paper_rebalance_preview" {
                return Err(SubmitRebalancePreviewError::IdempotencyMismatch);
            }
            let row = sqlx::query_as::<_, RebalancePreviewRow>(sqlx::AssertSqlSafe(format!(
                "{PREVIEW_COLUMNS} FROM paper_rebalance_previews AS preview \
                 WHERE preview.job_id=$1"
            )))
            .bind(job_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?
            .ok_or(SubmitRebalancePreviewError::IdempotencyMismatch)?;
            if row.account_id != input.account_id
                || row.recommendation_run_id != input.recommendation_run_id
            {
                return Err(SubmitRebalancePreviewError::IdempotencyMismatch);
            }
            tx.commit().await.map_err(TenancyError::from_sqlx)?;
            return Ok(SubmittedRebalancePreview {
                row,
                replayed: true,
            });
        }

        let locked = sqlx::query_as::<_, LockedSubmission>(
            "SELECT outcome, target_portfolio_id, strategy_config_id, price_date, \
                    dataset_version_id, dataset_manifest_sha256, weights_json \
             FROM lock_paper_rebalance_preview_submission($1,$2,$3,$4)",
        )
        .bind(owner)
        .bind(input.account_id)
        .bind(input.recommendation_run_id)
        .bind(input.seoul_today)
        .fetch_one(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        match locked.outcome.as_str() {
            "READY" => {}
            "NOT_FOUND" => return Err(TenancyError::NotFound.into()),
            "BINDING_REQUIRED" => return Err(SubmitRebalancePreviewError::BindingRequired),
            "RUN_NOT_READY" => return Err(SubmitRebalancePreviewError::RunNotReady),
            "DATA_BLOCKED" => return Err(SubmitRebalancePreviewError::DataBlocked),
            "ENTITLEMENT_REQUIRED" => {
                return Err(SubmitRebalancePreviewError::EntitlementRequired);
            }
            _ => {
                return Err(TenancyError::ResultIntegrity(
                    "preview submission returned an unknown outcome".into(),
                )
                .into());
            }
        }
        let target_portfolio_id = required(locked.target_portfolio_id, "target portfolio")?;
        let strategy_config_id = required(locked.strategy_config_id, "strategy config")?;
        let price_date = required(locked.price_date, "price date")?;
        let dataset_version_id = required(locked.dataset_version_id, "dataset version")?;
        let dataset_manifest_sha256 = required(locked.dataset_manifest_sha256, "dataset manifest")?;
        let weights_json = required(locked.weights_json, "target weights")?;
        if !weights_json.is_object() {
            return Err(TenancyError::ResultIntegrity(
                "target portfolio weights are not an object".into(),
            )
            .into());
        }
        let target_portfolio_sha256 =
            ContentHash::from_bytes(&serde_json::to_vec(&weights_json).map_err(|_| {
                TenancyError::ResultIntegrity("target weights cannot be serialized".into())
            })?)
            .as_str()
            .strip_prefix("sha256:")
            .expect("ContentHash is sha256")
            .to_owned();

        let active_jobs: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM jobs WHERE owner_user_id=$1 AND status IN ('QUEUED','RUNNING')",
        )
        .bind(owner)
        .fetch_one(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        if active_jobs >= input.max_jobs_per_owner as i64 {
            return Err(SubmitRebalancePreviewError::CapacityExceeded);
        }

        let preview_id = Uuid::new_v4();
        let job_id = Uuid::new_v4();
        let payload = serde_json::to_value(PaperPreviewPayload { preview_id })
            .expect("closed preview payload serializes");
        sqlx::query(
            "INSERT INTO jobs \
             (id,owner_user_id,job_type,status,priority,idempotency_key,payload_json,max_attempts,available_at) \
             VALUES ($1,$2,'paper_rebalance_preview','QUEUED',10,$3,$4,3,now())",
        )
        .bind(job_id)
        .bind(owner)
        .bind(&queue_key)
        .bind(payload)
        .execute(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        let row = sqlx::query_as::<_, RebalancePreviewRow>(sqlx::AssertSqlSafe(format!(
            "INSERT INTO paper_rebalance_previews \
             (id,owner_user_id,account_id,recommendation_run_id,target_portfolio_id, \
              strategy_config_id,job_id,price_date,dataset_version_id, \
              dataset_manifest_sha256,target_portfolio_sha256) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) \
             RETURNING {}",
            PREVIEW_RETURN_COLUMNS
        )))
        .bind(preview_id)
        .bind(owner)
        .bind(input.account_id)
        .bind(input.recommendation_run_id)
        .bind(target_portfolio_id)
        .bind(strategy_config_id)
        .bind(job_id)
        .bind(price_date)
        .bind(dataset_version_id)
        .bind(dataset_manifest_sha256)
        .bind(target_portfolio_sha256)
        .fetch_one(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(SubmittedRebalancePreview {
            row,
            replayed: false,
        })
    }

    pub async fn get(
        &self,
        actor: &Actor,
        account_id: Uuid,
        preview_id: Uuid,
    ) -> Result<RebalancePreviewRow, TenancyError> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let row = sqlx::query_as::<_, RebalancePreviewRow>(sqlx::AssertSqlSafe(format!(
            "{PREVIEW_COLUMNS} FROM paper_rebalance_previews AS preview \
             WHERE preview.id=$1 AND preview.account_id=$2"
        )))
        .bind(preview_id)
        .bind(account_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        row.ok_or(TenancyError::NotFound)
    }

    pub async fn apply(
        &self,
        actor: &Actor,
        account_id: Uuid,
        preview_id: Uuid,
        preview_token: &str,
        seoul_today: NaiveDate,
    ) -> Result<AppliedRebalancePreview, ApplyRebalancePreviewError> {
        let owner = actor_uuid(actor)?;
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let identity: ApplyIdentity = sqlx::query_as(
            "SELECT account_id,status,preview_token \
             FROM paper_rebalance_previews WHERE id=$1 AND account_id=$2",
        )
        .bind(preview_id)
        .bind(account_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?
        .ok_or(TenancyError::NotFound)?;
        if identity.status == "READY" {
            if identity.account_id != account_id
                || identity.preview_token.as_deref() != Some(preview_token)
            {
                return Err(ApplyRebalancePreviewError::Stale);
            }
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text,381901))")
                .bind(account_id)
                .execute(&mut *tx)
                .await
                .map_err(TenancyError::from_sqlx)?;
            let locked: LockedApplyInputs = sqlx::query_as(
                "SELECT preview.status,preview.preview_token,preview.account_state_version, \
                        preview.account_state_sha256,preview.target_portfolio_sha256, \
                        account.paper_state_version, \
                        (SELECT ledger.balance::text FROM cash_ledger AS ledger \
                          WHERE ledger.account_id=preview.account_id \
                            AND ledger.owner_user_id=preview.owner_user_id \
                          ORDER BY ledger.seq DESC LIMIT 1) AS cash_running, \
                        (SELECT COALESCE(sum(replay.amount),0)::text FROM cash_ledger AS replay \
                          WHERE replay.account_id=preview.account_id \
                            AND replay.owner_user_id=preview.owner_user_id) AS cash_replayed, \
                        COALESCE((SELECT jsonb_object_agg(position.instrument_id, \
                            position.quantity::text ORDER BY position.instrument_id) \
                          FROM positions AS position \
                          WHERE position.account_id=preview.account_id \
                            AND position.owner_user_id=preview.owner_user_id \
                            AND position.quantity<>0),'{}'::jsonb) AS positions_json, \
                        portfolio.weights_json \
                 FROM paper_rebalance_previews AS preview \
                 JOIN accounts AS account ON account.id=preview.account_id \
                    AND account.owner_user_id=preview.owner_user_id \
                 JOIN target_portfolios AS portfolio ON portfolio.id=preview.target_portfolio_id \
                    AND portfolio.owner_user_id=preview.owner_user_id \
                    AND portfolio.recommendation_run_id=preview.recommendation_run_id \
                 WHERE preview.id=$1 AND preview.account_id=$2 \
                 FOR SHARE OF account,portfolio",
            )
            .bind(preview_id)
            .bind(account_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?
            .ok_or(TenancyError::NotFound)?;
            if locked.preview_token.as_deref() != Some(preview_token) {
                return Err(ApplyRebalancePreviewError::Stale);
            }
            if locked.status == "READY" {
                if locked.cash_running.as_deref() != Some(locked.cash_replayed.as_str())
                    || locked.account_state_version != Some(locked.paper_state_version)
                {
                    return Err(ApplyRebalancePreviewError::Stale);
                }
                let current_account_hash = job_queue::paper_preview::account_state_sha256(
                    locked.paper_state_version,
                    locked.cash_running.as_deref().expect("checked present"),
                    &locked.positions_json,
                )
                .map_err(|_| {
                    TenancyError::ResultIntegrity("Paper account snapshot cannot be hashed".into())
                })?;
                let current_target_hash = raw_json_sha256(&locked.weights_json)?;
                if locked.account_state_sha256.as_deref() != Some(current_account_hash.as_str())
                    || locked.target_portfolio_sha256 != current_target_hash
                {
                    return Err(ApplyRebalancePreviewError::Stale);
                }
            } else if locked.status != "APPLIED" {
                return Err(ApplyRebalancePreviewError::NotReady);
            }
        } else if identity.status == "APPLIED" {
            if identity.preview_token.as_deref() != Some(preview_token) {
                return Err(ApplyRebalancePreviewError::Stale);
            }
        } else {
            return Err(ApplyRebalancePreviewError::NotReady);
        }

        let boundary: AppliedBoundaryRow = sqlx::query_as(
            "SELECT outcome,pending_target_id,effective_date,source_kind \
             FROM apply_paper_rebalance_preview($1,$2,$3,$4)",
        )
        .bind(owner)
        .bind(preview_id)
        .bind(preview_token)
        .bind(seoul_today)
        .fetch_one(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        let replayed = match boundary.outcome.as_str() {
            "APPLIED" => false,
            "REPLAY" => true,
            "NOT_FOUND" => return Err(TenancyError::NotFound.into()),
            "NOT_READY" => return Err(ApplyRebalancePreviewError::NotReady),
            "STALE" => return Err(ApplyRebalancePreviewError::Stale),
            "CONFLICT" => return Err(ApplyRebalancePreviewError::Conflict),
            _ => {
                return Err(TenancyError::ResultIntegrity(
                    "preview apply returned an unknown outcome".into(),
                )
                .into());
            }
        };
        let applied = AppliedRebalancePreview {
            preview_id,
            pending_target_id: required_apply(boundary.pending_target_id, "pending target")?,
            effective_date: required_apply(boundary.effective_date, "effective date")?,
            source_kind: required_apply(boundary.source_kind, "source kind")?,
            replayed,
        };
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(applied)
    }
}

fn required<T>(value: Option<T>, name: &str) -> Result<T, SubmitRebalancePreviewError> {
    value.ok_or_else(|| {
        TenancyError::ResultIntegrity(format!("preview submission omitted {name}")).into()
    })
}

fn required_apply<T>(value: Option<T>, name: &str) -> Result<T, ApplyRebalancePreviewError> {
    value.ok_or_else(|| {
        TenancyError::ResultIntegrity(format!("preview apply omitted {name}")).into()
    })
}

fn raw_json_sha256(value: &Value) -> Result<String, ApplyRebalancePreviewError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|_| TenancyError::ResultIntegrity("target weights cannot be serialized".into()))?;
    Ok(ContentHash::from_bytes(&bytes)
        .as_str()
        .strip_prefix("sha256:")
        .expect("ContentHash is sha256")
        .to_owned())
}

const PREVIEW_RETURN_COLUMNS: &str = "id,account_id,recommendation_run_id,target_portfolio_id, \
    strategy_config_id,job_id,status,price_basis,price_date,proposed_effective_date, \
    dataset_version_id,dataset_manifest_sha256,target_portfolio_sha256,preview_token, \
    result_json,error_json,created_at,started_at,completed_at,applied_at,updated_at";

const PREVIEW_COLUMNS: &str = "SELECT preview.id,preview.account_id,preview.recommendation_run_id, \
    preview.target_portfolio_id,preview.strategy_config_id,preview.job_id,preview.status, \
    preview.price_basis,preview.price_date,preview.proposed_effective_date, \
    preview.dataset_version_id,preview.dataset_manifest_sha256, \
    preview.target_portfolio_sha256,preview.preview_token,preview.result_json, \
    preview.error_json,preview.created_at,preview.started_at,preview.completed_at, \
    preview.applied_at,preview.updated_at";
