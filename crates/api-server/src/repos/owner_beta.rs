//! Dedicated durable enqueue boundary for the owner-beta price-only route.
//!
//! This repository intentionally does not use [`RecommendationRepo`] or the
//! generic [`job_queue::JobQueue`].  The owner-beta queue payload and result
//! row have a separate schema contract, so the jobs row and the
//! `owner_beta_recommendation_runs` row are written by one actor-scoped
//! transaction and can never be mistaken for a normal recommendation.

use crate::actor_tx::{actor_uuid, begin_actor_tx};
use crate::error::TenancyError;
use crate::http::pagination::Cursor;
use auth::entitlement::Actor;
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use job_queue::owner_beta::{
    OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE, OwnerBetaPriceRecommendationInput,
};
use job_queue::recommendation::compute::requirements_for;
use job_queue::resolver::ResolvedConfig;
use market_data::{ApprovedHistoricalPriceOnlyArtifact, KR_ETF_CORE_SYMBOLS};
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;
use uuid::Uuid;

/// Fixed fields of the owner-beta read contract. These values are repeated
/// here instead of inferred from the queue payload so a durable row cannot
/// widen the public surface by changing its own metadata.
pub const OWNER_BETA_PRICE_INPUT_KIND: &str = "owner_beta_historical_price_only_v1";
pub const OWNER_BETA_PRICE_CAPABILITY: &str = "PRICE_RETURN_ONLY";
pub const OWNER_BETA_PRICE_AUDIENCE: &str = "OWNER_ONLY";

const OWNER_BETA_STATUSES: [&str; 5] = ["PENDING", "RUNNING", "SUCCEEDED", "FAILED", "CANCELED"];

const OWNER_BETA_REASON_CODES: [&str; 14] = [
    "SELECTED_TOP_N",
    "NOT_SELECTED_BEYOND_TOP_N",
    "EXCLUDED_MANDATORY_FACTOR_NULL",
    "ALL_CASH_NO_ELIGIBLE",
    "WEIGHT_CAPPED_AT_MAX",
    "WEIGHT_ROUNDING_RESIDUE_TO_CASH",
    "CASH_FLOOR_APPLIED",
    "BENCHMARK_HELD",
    "TREND_POSITIVE",
    "TREND_NEGATIVE_CASH",
    "ABSOLUTE_MOMENTUM_PASSED",
    "DEFENSIVE_CASH_SELECTED",
    "INVERSE_VOL_WEIGHTED",
    "NOT_SELECTED_BY_STRATEGY",
];

const OWNER_BETA_ERROR_CODES: [&str; 8] = [
    "OWNER_BETA_INPUT_INVALID",
    "OWNER_BETA_ENTITLEMENT_DENIED",
    "OWNER_BETA_FACTOR_INVALID",
    "OWNER_BETA_TARGET_INVALID",
    "OWNER_BETA_PUBLICATION_UNAVAILABLE",
    "OWNER_BETA_COMPUTATION_UNAVAILABLE",
    "OWNER_BETA_COMPUTATION_FAILED",
    "OWNER_BETA_ATTEMPTS_EXHAUSTED",
];

const OWNER_BETA_CANCELED_CODE: &str = "CANCELED";
const OWNER_BETA_WEIGHT_SCALE: i64 = 1_000_000;

/// Durable owner-beta run projection used by the read routes. `item_count`
/// is an internal integrity witness and is never serialized.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OwnerBetaPriceOnlyReadRunRow {
    pub id: Uuid,
    pub job_id: Uuid,
    pub strategy_config_id: Uuid,
    pub strategy_id: String,
    pub strategy_version: String,
    pub as_of: NaiveDate,
    pub status: String,
    pub input_kind: String,
    pub capability: String,
    pub audience: String,
    pub vendor_snapshot: bool,
    pub strict_pit: bool,
    pub strategy_config_sha256: String,
    pub candidate_content_sha256: String,
    pub artifact_manifest_sha256: String,
    pub stage5_manifest_sha256: String,
    pub action_manifest_sha256: String,
    pub approval_registry_sha256: String,
    pub factor_snapshot_sha256: Option<String>,
    pub target_snapshot_sha256: Option<String>,
    pub cash_weight: Option<String>,
    pub error_code: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub item_count: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OwnerBetaPriceOnlyReadItemRow {
    pub recommendation_run_id: Uuid,
    pub instrument_id: String,
    pub instrument_name: Option<String>,
    pub instrument_asset_class: Option<String>,
    pub rank: Option<i32>,
    pub target_weight: Option<String>,
    pub reason_codes: Value,
    pub factors_json: Value,
    pub excluded: bool,
    pub exclusion_reason: Option<String>,
}

/// The public result needed by the HTTP response. `replay` is derived from
/// the durable queue row, never from the process-local idempotency cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnerBetaPriceRecommendationRun {
    pub run_id: Uuid,
    pub job_id: Uuid,
    pub status: &'static str,
    pub replay: bool,
}

/// Static repository failures. Values from SQLx, the request body, the
/// artifact path, and approval pins intentionally cannot cross this boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum OwnerBetaPriceRecommendationError {
    #[error("resource not found")]
    NotFound,
    #[error("forbidden")]
    Forbidden,
    #[error("owner-beta recommendation capacity exceeded")]
    CapacityExceeded,
    #[error("owner-beta strategy is unsupported")]
    StrategyUnsupported,
    #[error("idempotency key was already used with a different owner-beta request")]
    IdempotencyMismatch,
    #[error("internal error")]
    Internal,
}

impl From<TenancyError> for OwnerBetaPriceRecommendationError {
    fn from(error: TenancyError) -> Self {
        match error {
            TenancyError::NotFound => Self::NotFound,
            TenancyError::Forbidden => Self::Forbidden,
            TenancyError::Database(_)
            | TenancyError::NotImplemented
            | TenancyError::DatasetBlocked(_)
            | TenancyError::InvalidState(_)
            | TenancyError::ResultIntegrity(_) => Self::Internal,
        }
    }
}

/// Dedicated actor-scoped repository for the sealed owner-beta enqueue.
#[derive(Debug, Clone)]
pub struct OwnerBetaRecommendationRepo {
    pool: sqlx::PgPool,
}

#[derive(Clone, sqlx::FromRow)]
struct OwnerBetaRunBinding {
    id: Uuid,
    job_id: Uuid,
    owner_user_id: Uuid,
    strategy_config_id: Uuid,
    strategy_id: String,
    strategy_version: String,
    strategy_config_json: Value,
    strategy_config_sha256: String,
    as_of: NaiveDate,
    #[allow(dead_code)]
    status: String,
    candidate_content_sha256: String,
    artifact_manifest_sha256: String,
    stage5_manifest_sha256: String,
    action_manifest_sha256: String,
    approval_registry_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ApprovalPinStrings {
    candidate_content_sha256: String,
    artifact_manifest_sha256: String,
    stage5_manifest_sha256: String,
    action_manifest_sha256: String,
    approval_registry_sha256: String,
}

impl ApprovalPinStrings {
    fn from_artifact(artifact: &ApprovedHistoricalPriceOnlyArtifact) -> Self {
        let pins = artifact.pins();
        Self {
            candidate_content_sha256: pins.candidate_content_sha256().to_string(),
            artifact_manifest_sha256: pins.artifact_manifest_sha256().to_string(),
            stage5_manifest_sha256: pins.stage5_manifest_sha256().to_string(),
            action_manifest_sha256: pins.action_manifest_sha256().to_string(),
            approval_registry_sha256: pins.approval_registry_sha256().to_string(),
        }
    }

    fn from_input(input: &OwnerBetaPriceRecommendationInput) -> Self {
        let pins = input.pins();
        Self {
            candidate_content_sha256: pins.candidate_content_sha256().to_string(),
            artifact_manifest_sha256: pins.artifact_manifest_sha256().to_string(),
            stage5_manifest_sha256: pins.stage5_manifest_sha256().to_string(),
            action_manifest_sha256: pins.action_manifest_sha256().to_string(),
            approval_registry_sha256: pins.approval_registry_sha256().to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReplayExpectation {
    owner_user_id: Uuid,
    strategy_config_id: Uuid,
    as_of: NaiveDate,
    as_of_trading: domain::TradingDate,
    pins: ApprovalPinStrings,
}

impl ReplayExpectation {
    fn from_approved_artifact(
        owner_user_id: Uuid,
        strategy_config_id: Uuid,
        as_of: NaiveDate,
        artifact: &ApprovedHistoricalPriceOnlyArtifact,
    ) -> Result<Self, OwnerBetaPriceRecommendationError> {
        let as_of_trading = domain::TradingDate::new(as_of.year(), as_of.month(), as_of.day())
            .map_err(|_| OwnerBetaPriceRecommendationError::Internal)?;
        Ok(Self {
            owner_user_id,
            strategy_config_id,
            as_of,
            as_of_trading,
            pins: ApprovalPinStrings::from_artifact(artifact),
        })
    }
}

struct SubmissionProjection {
    payload_json: serde_json::Value,
    pins: ApprovalPinStrings,
    strategy_id: String,
    strategy_version: String,
    strategy_config_json: Value,
    strategy_config_sha256: String,
}

impl SubmissionProjection {
    fn from_approved_artifact(
        run_id: Uuid,
        strategy_config_id: Uuid,
        as_of: NaiveDate,
        artifact: &ApprovedHistoricalPriceOnlyArtifact,
        resolved_config: &ResolvedConfig,
    ) -> Result<Self, OwnerBetaPriceRecommendationError> {
        let as_of = domain::TradingDate::new(as_of.year(), as_of.month(), as_of.day())
            .map_err(|_| OwnerBetaPriceRecommendationError::Internal)?;
        let input = OwnerBetaPriceRecommendationInput::from_approved_artifact(
            run_id,
            strategy_config_id,
            as_of,
            artifact,
            resolved_config,
        )
        .map_err(|_| OwnerBetaPriceRecommendationError::StrategyUnsupported)?;
        Self::from_input(input)
    }

    fn from_input(
        input: OwnerBetaPriceRecommendationInput,
    ) -> Result<Self, OwnerBetaPriceRecommendationError> {
        input
            .validate_strategy_snapshot()
            .map_err(|_| OwnerBetaPriceRecommendationError::StrategyUnsupported)?;
        let pins = ApprovalPinStrings::from_input(&input);
        let strategy = input.strategy_snapshot();
        let strategy_id = strategy.strategy_id().to_owned();
        let strategy_version = strategy.strategy_version().to_owned();
        let strategy_config_json = strategy.config_json().clone();
        let strategy_config_sha256 = strategy.config_sha256().to_string();
        let payload_json =
            serde_json::to_value(input).map_err(|_| OwnerBetaPriceRecommendationError::Internal)?;
        Ok(Self {
            payload_json,
            pins,
            strategy_id,
            strategy_version,
            strategy_config_json,
            strategy_config_sha256,
        })
    }
}

const JOB_INSERT_SQL: &str = "INSERT INTO jobs
        (id, owner_user_id, job_type, status, priority, idempotency_key,
         payload_json, max_attempts, available_at)
     VALUES ($1, $2, $3, 'QUEUED', 10, $4, $5, 3, now())";

const RUN_INSERT_SQL: &str = "INSERT INTO owner_beta_recommendation_runs
        (id, owner_user_id, strategy_config_id, strategy_id, strategy_version,
         strategy_config_json, strategy_config_sha256, job_id, as_of,
         candidate_content_sha256, artifact_manifest_sha256,
         stage5_manifest_sha256, action_manifest_sha256,
         approval_registry_sha256)
     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)";

impl OwnerBetaRecommendationRepo {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    /// Enqueue one owner-beta price-only recommendation or return the exact
    /// durable replay for its namespaced client key.
    ///
    /// The caller must have approved `artifact` immediately before entering
    /// this method.  The artifact is borrowed only long enough to derive the
    /// sealed payload; no artifact bytes or filesystem path enter SQL.
    pub async fn submit(
        &self,
        actor: &Actor,
        strategy_config_id: Uuid,
        as_of: NaiveDate,
        client_key: &str,
        artifact: &ApprovedHistoricalPriceOnlyArtifact,
        max_jobs_per_owner: u32,
    ) -> Result<OwnerBetaPriceRecommendationRun, OwnerBetaPriceRecommendationError> {
        let owner = actor_uuid(actor).map_err(OwnerBetaPriceRecommendationError::from)?;
        let mut tx = begin_actor_tx(&self.pool, actor)
            .await
            .map_err(OwnerBetaPriceRecommendationError::from)?;

        // All job producers share this per-owner advisory lock. The lock is
        // held through the replay probe, capacity count, and both inserts.
        crate::repos::lock_owner_job_capacity(&mut tx, owner)
            .await
            .map_err(OwnerBetaPriceRecommendationError::from)?;

        let queue_key = format!("owner-beta:price-only:v1:{client_key}");
        let replay_expectation =
            ReplayExpectation::from_approved_artifact(owner, strategy_config_id, as_of, artifact)?;
        let replay = match durable_replay(&mut tx, &queue_key, &replay_expectation).await {
            Ok(replay) => replay,
            Err(TenancyError::InvalidState(_)) => {
                return Err(OwnerBetaPriceRecommendationError::IdempotencyMismatch);
            }
            Err(error) => return Err(OwnerBetaPriceRecommendationError::from(error)),
        };
        if let Some(replay) = replay {
            tx.commit()
                .await
                .map_err(|_| OwnerBetaPriceRecommendationError::Internal)?;
            return Ok(replay);
        }

        // RLS makes a foreign config invisible. Keep the explicit owner
        // predicate as a second invariant, then require the active row lock.
        let config: Option<(Uuid, bool, String, String, Value)> = sqlx::query_as(
            "SELECT owner_user_id, is_active, strategy_id, strategy_version, config_json
               FROM user_strategy_configs
              WHERE id = $1
                AND owner_user_id = $2
              FOR SHARE",
        )
        .bind(strategy_config_id)
        .bind(owner)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| OwnerBetaPriceRecommendationError::Internal)?;
        let Some((config_owner, true, strategy_id, strategy_version, config_json)) = config else {
            return Err(OwnerBetaPriceRecommendationError::NotFound);
        };
        if config_owner != owner {
            return Err(OwnerBetaPriceRecommendationError::NotFound);
        }
        let resolved_config = ResolvedConfig {
            strategy_id,
            strategy_version,
            config: config_json,
        };
        if requirements_for(&resolved_config).is_err() {
            return Err(OwnerBetaPriceRecommendationError::StrategyUnsupported);
        }

        let active_jobs: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint
               FROM jobs
              WHERE owner_user_id = $1
                AND status IN ('QUEUED', 'RUNNING')",
        )
        .bind(owner)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| OwnerBetaPriceRecommendationError::Internal)?;
        if active_jobs >= max_jobs_per_owner as i64 {
            return Err(OwnerBetaPriceRecommendationError::CapacityExceeded);
        }

        let run_id = Uuid::new_v4();
        let job_id = Uuid::new_v4();
        let projection = SubmissionProjection::from_approved_artifact(
            run_id,
            strategy_config_id,
            as_of,
            artifact,
            &resolved_config,
        )?;

        // Keep this insert first: a failure in the run insert must roll back
        // the queue row together with it.
        sqlx::query(JOB_INSERT_SQL)
            .bind(job_id)
            .bind(owner)
            .bind(OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE)
            .bind(&queue_key)
            .bind(projection.payload_json)
            .execute(&mut *tx)
            .await
            .map_err(|_| OwnerBetaPriceRecommendationError::Internal)?;

        let pins = projection.pins;
        sqlx::query(RUN_INSERT_SQL)
            .bind(run_id)
            .bind(owner)
            .bind(strategy_config_id)
            .bind(projection.strategy_id)
            .bind(projection.strategy_version)
            .bind(projection.strategy_config_json)
            .bind(projection.strategy_config_sha256)
            .bind(job_id)
            .bind(as_of)
            .bind(pins.candidate_content_sha256)
            .bind(pins.artifact_manifest_sha256)
            .bind(pins.stage5_manifest_sha256)
            .bind(pins.action_manifest_sha256)
            .bind(pins.approval_registry_sha256)
            .execute(&mut *tx)
            .await
            .map_err(|_| OwnerBetaPriceRecommendationError::Internal)?;

        tx.commit()
            .await
            .map_err(|_| OwnerBetaPriceRecommendationError::Internal)?;
        Ok(OwnerBetaPriceRecommendationRun {
            run_id,
            job_id,
            status: "PENDING",
            replay: false,
        })
    }

    /// Read one owner-beta run and, only for a valid successful publication,
    /// its fixed ETF11 items. The transaction pins the actor GUC before any
    /// table read; explicit owner predicates are retained as a second
    /// defense-in-depth invariant.
    pub async fn get_price_only_run(
        &self,
        actor: &Actor,
        run_id: Uuid,
    ) -> crate::error::TenancyResult<(
        OwnerBetaPriceOnlyReadRunRow,
        Vec<OwnerBetaPriceOnlyReadItemRow>,
    )> {
        let owner = actor_uuid(actor)?;
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let row = sqlx::query_as::<_, OwnerBetaPriceOnlyReadRunRow>(READ_RUN_SQL)
            .bind(run_id)
            .bind(owner)
            .fetch_optional(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?;
        let Some(row) = row else {
            tx.commit().await.map_err(TenancyError::from_sqlx)?;
            return Err(TenancyError::NotFound);
        };

        let items = if row.status == "SUCCEEDED" {
            let item_limit = item_row_limit(1)?;
            sqlx::query_as::<_, OwnerBetaPriceOnlyReadItemRow>(READ_ITEMS_SQL)
                .bind(row.id)
                .bind(owner)
                .bind(item_limit)
                .fetch_all(&mut *tx)
                .await
                .map_err(TenancyError::from_sqlx)?
        } else {
            Vec::new()
        };
        validate_read_model(&row, &items)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok((row, items))
    }

    /// Fetch only the actor-owned run header. This is used by the detail
    /// handler to determine the exact entitlement date before it reads any
    /// result items. No item rows are selected by this method.
    pub async fn get_price_only_run_header(
        &self,
        actor: &Actor,
        run_id: Uuid,
    ) -> crate::error::TenancyResult<OwnerBetaPriceOnlyReadRunRow> {
        let owner = actor_uuid(actor)?;
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let row = sqlx::query_as::<_, OwnerBetaPriceOnlyReadRunRow>(READ_RUN_HEADER_SQL)
            .bind(run_id)
            .bind(owner)
            .fetch_optional(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        row.ok_or(TenancyError::NotFound)
    }

    /// Keyset-paginated actor-scoped list. Successful rows load their items
    /// solely to prove the durable result is still a valid ETF11 publication;
    /// those items are intentionally omitted from the returned list rows.
    pub async fn list_price_only_runs(
        &self,
        actor: &Actor,
        after: Option<&Cursor>,
        limit: usize,
    ) -> crate::error::TenancyResult<(Vec<OwnerBetaPriceOnlyReadRunRow>, Option<Cursor>)> {
        let owner = actor_uuid(actor)?;
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let rows = match after {
            Some(cursor) => {
                let cursor_id = Uuid::parse_str(&cursor.i).map_err(|_| TenancyError::NotFound)?;
                sqlx::query_as::<_, OwnerBetaPriceOnlyReadRunRow>(READ_RUNS_AFTER_SQL)
                    .bind(owner)
                    .bind(cursor.k.clone())
                    .bind(cursor_id)
                    .bind(limit as i64 + 1)
                    .fetch_all(&mut *tx)
                    .await
                    .map_err(TenancyError::from_sqlx)?
            }
            None => sqlx::query_as::<_, OwnerBetaPriceOnlyReadRunRow>(READ_RUNS_SQL)
                .bind(owner)
                .bind(limit as i64 + 1)
                .fetch_all(&mut *tx)
                .await
                .map_err(TenancyError::from_sqlx)?,
        };

        let successful_ids = rows
            .iter()
            .filter(|row| row.status == "SUCCEEDED")
            .map(|row| row.id)
            .collect::<Vec<_>>();
        let mut items_by_run = BTreeMap::<Uuid, Vec<OwnerBetaPriceOnlyReadItemRow>>::new();
        if !successful_ids.is_empty() {
            let item_limit = item_row_limit(successful_ids.len())?;
            let item_rows =
                sqlx::query_as::<_, OwnerBetaPriceOnlyReadItemRow>(READ_ITEMS_FOR_RUNS_SQL)
                    .bind(&successful_ids)
                    .bind(owner)
                    .bind(item_limit)
                    .fetch_all(&mut *tx)
                    .await
                    .map_err(TenancyError::from_sqlx)?;
            for item in item_rows {
                items_by_run
                    .entry(item.recommendation_run_id)
                    .or_default()
                    .push(item);
            }
        }
        for row in &rows {
            let items = items_by_run.remove(&row.id).unwrap_or_default();
            validate_read_model(row, &items)?;
        }

        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(crate::repos::split_page(rows, limit, |row| {
            (row.created_at.to_rfc3339(), row.id.to_string())
        }))
    }
}

const READ_RUN_SQL: &str = "
    SELECT run.id, run.job_id, run.strategy_config_id, run.strategy_id,
           run.strategy_version, run.as_of, run.status, run.input_kind,
           run.capability, run.audience, run.vendor_snapshot, run.strict_pit,
           run.strategy_config_sha256, run.candidate_content_sha256,
           run.artifact_manifest_sha256, run.stage5_manifest_sha256,
           run.action_manifest_sha256, run.approval_registry_sha256,
           run.factor_snapshot_sha256, run.target_snapshot_sha256,
           run.cash_weight::text AS cash_weight, run.error_code,
           run.created_at, run.started_at, run.finished_at, run.updated_at,
           (SELECT count(*)::bigint
              FROM owner_beta_recommendation_items AS item
             WHERE item.recommendation_run_id = run.id
               AND item.owner_user_id = run.owner_user_id) AS item_count
      FROM owner_beta_recommendation_runs AS run
     WHERE run.id = $1 AND run.owner_user_id = $2";
const READ_RUN_HEADER_SQL: &str =
    "\n    SELECT run.id, run.job_id, run.strategy_config_id, run.strategy_id,
           run.strategy_version, run.as_of, run.status, run.input_kind,
           run.capability, run.audience, run.vendor_snapshot, run.strict_pit,
           run.strategy_config_sha256, run.candidate_content_sha256,
           run.artifact_manifest_sha256, run.stage5_manifest_sha256,
           run.action_manifest_sha256, run.approval_registry_sha256,
           run.factor_snapshot_sha256, run.target_snapshot_sha256,
           run.cash_weight::text AS cash_weight, run.error_code,
           run.created_at, run.started_at, run.finished_at, run.updated_at,
           0::bigint AS item_count
      FROM owner_beta_recommendation_runs AS run
     WHERE run.id = $1 AND run.owner_user_id = $2";
const READ_RUNS_SQL: &str = "
    SELECT run.id, run.job_id, run.strategy_config_id, run.strategy_id,
           run.strategy_version, run.as_of, run.status, run.input_kind,
           run.capability, run.audience, run.vendor_snapshot, run.strict_pit,
           run.strategy_config_sha256, run.candidate_content_sha256,
           run.artifact_manifest_sha256, run.stage5_manifest_sha256,
           run.action_manifest_sha256, run.approval_registry_sha256,
           run.factor_snapshot_sha256, run.target_snapshot_sha256,
           run.cash_weight::text AS cash_weight, run.error_code,
           run.created_at, run.started_at, run.finished_at, run.updated_at,
           (SELECT count(*)::bigint
              FROM owner_beta_recommendation_items AS item
             WHERE item.recommendation_run_id = run.id
               AND item.owner_user_id = run.owner_user_id) AS item_count
      FROM owner_beta_recommendation_runs AS run
     WHERE run.owner_user_id = $1
       ORDER BY run.created_at DESC, run.id DESC
       LIMIT $2";
const READ_RUNS_AFTER_SQL: &str = "
    SELECT run.id, run.job_id, run.strategy_config_id, run.strategy_id,
           run.strategy_version, run.as_of, run.status, run.input_kind,
           run.capability, run.audience, run.vendor_snapshot, run.strict_pit,
           run.strategy_config_sha256, run.candidate_content_sha256,
           run.artifact_manifest_sha256, run.stage5_manifest_sha256,
           run.action_manifest_sha256, run.approval_registry_sha256,
           run.factor_snapshot_sha256, run.target_snapshot_sha256,
           run.cash_weight::text AS cash_weight, run.error_code,
           run.created_at, run.started_at, run.finished_at, run.updated_at,
           (SELECT count(*)::bigint
              FROM owner_beta_recommendation_items AS item
             WHERE item.recommendation_run_id = run.id
               AND item.owner_user_id = run.owner_user_id) AS item_count
      FROM owner_beta_recommendation_runs AS run
     WHERE run.owner_user_id = $1
       AND (run.created_at, run.id) < ($2::timestamptz, $3::uuid)
       ORDER BY run.created_at DESC, run.id DESC
       LIMIT $4";
const READ_ITEMS_SQL: &str = "
    SELECT item.recommendation_run_id, item.instrument_id,
           instrument.name AS instrument_name,
           instrument.asset_class AS instrument_asset_class,
           item.rank, item.target_weight::text AS target_weight,
           item.reason_codes, item.factors_json,
           item.excluded, item.exclusion_reason
      FROM public.owner_beta_recommendation_items AS item
      LEFT JOIN public.instruments AS instrument
        ON instrument.id = item.instrument_id
     WHERE item.recommendation_run_id = $1
       AND item.owner_user_id = $2
     ORDER BY item.instrument_id
       LIMIT $3";
const READ_ITEMS_FOR_RUNS_SQL: &str = "
    SELECT item.recommendation_run_id, item.instrument_id,
           instrument.name AS instrument_name,
           instrument.asset_class AS instrument_asset_class,
           item.rank, item.target_weight::text AS target_weight,
           item.reason_codes, item.factors_json,
           item.excluded, item.exclusion_reason
      FROM public.owner_beta_recommendation_items AS item
      LEFT JOIN public.instruments AS instrument
        ON instrument.id = item.instrument_id
     WHERE item.recommendation_run_id = ANY($1::uuid[])
       AND item.owner_user_id = $2
     ORDER BY item.recommendation_run_id, item.instrument_id
       LIMIT $3";

fn integrity_error() -> TenancyError {
    TenancyError::ResultIntegrity("owner-beta recommendation result integrity failed".to_owned())
}

fn item_row_limit(run_count: usize) -> Result<i64, TenancyError> {
    let per_run = KR_ETF_CORE_SYMBOLS
        .len()
        .checked_add(1)
        .ok_or_else(integrity_error)?;
    let total = run_count.checked_mul(per_run).ok_or_else(integrity_error)?;
    i64::try_from(total).map_err(|_| integrity_error())
}

/// Validate every durable field that crosses the owner-beta read boundary.
/// This function is intentionally pure and does not consult artifacts,
/// approval registries, queue payloads, or mutable strategy configuration.
fn validate_read_model(
    row: &OwnerBetaPriceOnlyReadRunRow,
    items: &[OwnerBetaPriceOnlyReadItemRow],
) -> Result<(), TenancyError> {
    if row.id.is_nil()
        || row.job_id.is_nil()
        || row.strategy_config_id.is_nil()
        || row.strategy_id.is_empty()
        || row.strategy_version.is_empty()
        || !OWNER_BETA_STATUSES.contains(&row.status.as_str())
        || row.input_kind != OWNER_BETA_PRICE_INPUT_KIND
        || row.capability != OWNER_BETA_PRICE_CAPABILITY
        || row.audience != OWNER_BETA_PRICE_AUDIENCE
        || !row.vendor_snapshot
        || row.strict_pit
        || !valid_sha256(&row.strategy_config_sha256)
        || !valid_sha256(&row.candidate_content_sha256)
        || !valid_sha256(&row.artifact_manifest_sha256)
        || !valid_sha256(&row.stage5_manifest_sha256)
        || !valid_sha256(&row.action_manifest_sha256)
        || !valid_sha256(&row.approval_registry_sha256)
        || !row
            .factor_snapshot_sha256
            .as_deref()
            .is_none_or(valid_sha256)
        || !row
            .target_snapshot_sha256
            .as_deref()
            .is_none_or(valid_sha256)
        || row.item_count < 0
        || row.item_count != items.len() as i64
        || row.updated_at < row.created_at
        || row
            .started_at
            .is_some_and(|started| started < row.created_at)
        || row
            .started_at
            .is_some_and(|started| row.updated_at < started)
        || row
            .finished_at
            .is_some_and(|finished| finished < row.created_at)
        || row
            .finished_at
            .is_some_and(|finished| row.started_at.is_some_and(|started| finished < started))
        || row
            .finished_at
            .is_some_and(|finished| row.updated_at < finished)
    {
        return Err(integrity_error());
    }

    match row.status.as_str() {
        "PENDING" => {
            if row.started_at.is_some()
                || row.finished_at.is_some()
                || row.error_code.is_some()
                || has_result_fields(row)
                || !items.is_empty()
            {
                return Err(integrity_error());
            }
        }
        "RUNNING" => {
            if row.started_at.is_none()
                || row.finished_at.is_some()
                || row.error_code.is_some()
                || has_result_fields(row)
                || !items.is_empty()
            {
                return Err(integrity_error());
            }
        }
        "SUCCEEDED" => {
            if row.started_at.is_none()
                || row.finished_at.is_none()
                || row.error_code.is_some()
                || row.factor_snapshot_sha256.is_none()
                || row.target_snapshot_sha256.is_none()
                || row.cash_weight.is_none()
                || !items_are_valid_success_publication(row, items)
            {
                return Err(integrity_error());
            }
        }
        "FAILED" => {
            if row.started_at.is_none()
                || row.finished_at.is_none()
                || !valid_error_code(row.error_code.as_deref(), false)
                || has_result_fields(row)
                || !items.is_empty()
            {
                return Err(integrity_error());
            }
        }
        "CANCELED" => {
            if row.finished_at.is_none()
                || !valid_error_code(row.error_code.as_deref(), true)
                || has_result_fields(row)
                || !items.is_empty()
            {
                return Err(integrity_error());
            }
        }
        _ => return Err(integrity_error()),
    }
    Ok(())
}

fn has_result_fields(row: &OwnerBetaPriceOnlyReadRunRow) -> bool {
    row.factor_snapshot_sha256.is_some()
        || row.target_snapshot_sha256.is_some()
        || row.cash_weight.is_some()
}

fn valid_sha256(value: &str) -> bool {
    let Some(digest) = value.strip_prefix("sha256:") else {
        return false;
    };
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_error_code(value: Option<&str>, canceled: bool) -> bool {
    let Some(value) = value else {
        return false;
    };
    if canceled {
        value == OWNER_BETA_CANCELED_CODE
    } else {
        OWNER_BETA_ERROR_CODES.contains(&value)
    }
}

fn items_are_valid_success_publication(
    row: &OwnerBetaPriceOnlyReadRunRow,
    items: &[OwnerBetaPriceOnlyReadItemRow],
) -> bool {
    let expected = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| format!("{symbol}.KRX"))
        .collect::<BTreeSet<_>>();
    if items.len() != KR_ETF_CORE_SYMBOLS.len()
        || items
            .iter()
            .any(|item| item.recommendation_run_id != row.id)
        || items
            .iter()
            .map(|item| item.instrument_id.as_str())
            .collect::<BTreeSet<_>>()
            != expected.iter().map(String::as_str).collect::<BTreeSet<_>>()
    {
        return false;
    }

    let Some(cash_weight) = row.cash_weight.as_deref().and_then(parse_fixed_six) else {
        return false;
    };
    let mut total = cash_weight;
    let mut seen_ranks = BTreeSet::new();
    for item in items {
        let Some(reason_codes) = valid_reason_codes(&item.reason_codes) else {
            return false;
        };
        if !valid_factors(&item.factors_json) {
            return false;
        }
        let valid_selected = !item.excluded
            && item.target_weight.is_some()
            && item.rank.is_some()
            && item.exclusion_reason.is_none();
        let valid_excluded = item.excluded
            && item.target_weight.is_none()
            && item.rank.is_none()
            && item
                .exclusion_reason
                .as_deref()
                .is_some_and(|reason| reason == reason_codes[0]);
        if !valid_selected && !valid_excluded {
            return false;
        }
        if let Some(rank) = item.rank
            && (!(1..=KR_ETF_CORE_SYMBOLS.len() as i32).contains(&rank) || !seen_ranks.insert(rank))
        {
            return false;
        }
        if let Some(weight) = item.target_weight.as_deref().and_then(parse_fixed_six) {
            total = match total.checked_add(weight) {
                Some(total) => total,
                None => return false,
            };
        } else if item.target_weight.is_some() {
            return false;
        }
    }
    total == OWNER_BETA_WEIGHT_SCALE
}

fn valid_reason_codes(value: &Value) -> Option<Vec<&str>> {
    let values = value.as_array()?;
    if values.is_empty() || values.len() > 16 {
        return None;
    }
    let mut seen = BTreeSet::new();
    let mut codes = Vec::with_capacity(values.len());
    for value in values {
        let code = value.as_str()?;
        if code.is_empty()
            || code.len() > 64
            || !OWNER_BETA_REASON_CODES.contains(&code)
            || !seen.insert(code)
        {
            return None;
        }
        codes.push(code);
    }
    Some(codes)
}

fn valid_factors(value: &Value) -> bool {
    let Some(values) = value.as_object() else {
        return false;
    };
    values.len() <= 64
        && values.iter().all(|(key, value)| {
            !key.is_empty()
                && key.len() <= 64
                && value
                    .as_str()
                    .is_some_and(|value| !value.is_empty() && value.len() <= 64)
        })
}

fn parse_fixed_six(value: &str) -> Option<i64> {
    let (whole, fraction) = value.split_once('.')?;
    if fraction.len() != 6
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || (whole != "0" && whole != "1")
    {
        return None;
    }
    let fractional = fraction.parse::<i64>().ok()?;
    if whole == "1" && fractional != 0 {
        return None;
    }
    let result = fractional;
    Some(if whole == "1" {
        OWNER_BETA_WEIGHT_SCALE + result
    } else {
        result
    })
}

async fn durable_replay(
    tx: &mut Transaction<'_, Postgres>,
    queue_key: &str,
    expected: &ReplayExpectation,
) -> Result<Option<OwnerBetaPriceRecommendationRun>, TenancyError> {
    let Some((job_id, job_type, payload_json)) =
        sqlx::query_as::<_, (Uuid, String, serde_json::Value)>(
            "SELECT id, job_type, payload_json
           FROM jobs
          WHERE owner_user_id = $1
            AND idempotency_key = $2
          FOR SHARE",
        )
        .bind(expected.owner_user_id)
        .bind(queue_key)
        .fetch_optional(&mut **tx)
        .await
        .map_err(TenancyError::from_sqlx)?
    else {
        return Ok(None);
    };

    let input = serde_json::from_value::<OwnerBetaPriceRecommendationInput>(payload_json).ok();
    let Some(input) = input else {
        return Err(TenancyError::InvalidState(
            "idempotency mismatch".to_owned(),
        ));
    };
    let row: Option<OwnerBetaRunBinding> = sqlx::query_as(
        "SELECT id, job_id, owner_user_id, strategy_config_id, as_of, status,
                strategy_id, strategy_version, strategy_config_json,
                strategy_config_sha256,
                candidate_content_sha256, artifact_manifest_sha256,
                stage5_manifest_sha256, action_manifest_sha256,
                approval_registry_sha256
           FROM owner_beta_recommendation_runs
          WHERE id = $1
            AND owner_user_id = $2
            AND job_id = $3
          FOR SHARE",
    )
    .bind(input.run_id())
    .bind(expected.owner_user_id)
    .bind(job_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(TenancyError::from_sqlx)?;
    let Some(binding) = row else {
        return Err(TenancyError::InvalidState(
            "idempotency mismatch".to_owned(),
        ));
    };
    if !replay_binding_matches(job_id, &job_type, &input, &binding, expected) {
        return Err(TenancyError::InvalidState(
            "idempotency mismatch".to_owned(),
        ));
    }
    Ok(Some(OwnerBetaPriceRecommendationRun {
        run_id: binding.id,
        job_id,
        // The enqueue contract always returns the fixed pending response,
        // including a durable replay after a worker has settled the run.
        status: "PENDING",
        replay: true,
    }))
}

fn replay_binding_matches(
    job_id: Uuid,
    job_type: &str,
    input: &OwnerBetaPriceRecommendationInput,
    binding: &OwnerBetaRunBinding,
    expected: &ReplayExpectation,
) -> bool {
    if input.validate_strategy_snapshot().is_err() {
        return false;
    }
    let input_pins = ApprovalPinStrings::from_input(input);
    let strategy = input.strategy_snapshot();
    job_type == OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE
        && input.strategy_config_id() == expected.strategy_config_id
        && input.as_of() == expected.as_of_trading
        && input_pins == expected.pins
        && binding.id == input.run_id()
        && binding.job_id == job_id
        && binding.owner_user_id == expected.owner_user_id
        && binding.strategy_config_id == expected.strategy_config_id
        && binding.strategy_id == strategy.strategy_id()
        && binding.strategy_version == strategy.strategy_version()
        && binding.strategy_config_json == *strategy.config_json()
        && binding.strategy_config_sha256 == strategy.config_sha256().as_str()
        && binding.as_of == expected.as_of
        && binding.candidate_content_sha256 == input_pins.candidate_content_sha256
        && binding.artifact_manifest_sha256 == input_pins.artifact_manifest_sha256
        && binding.stage5_manifest_sha256 == input_pins.stage5_manifest_sha256
        && binding.action_manifest_sha256 == input_pins.action_manifest_sha256
        && binding.approval_registry_sha256 == input_pins.approval_registry_sha256
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::collections::BTreeSet;

    fn hash(value: u8) -> String {
        format!("sha256:{value:064x}")
    }

    fn input_value() -> Value {
        let strategy = serde_json::to_value(
            job_queue::owner_beta::OwnerBetaStrategySnapshot::from_resolved_config(
                &job_queue::resolver::ResolvedConfig {
                    strategy_id: "buy_and_hold".to_owned(),
                    strategy_version: "1.0.0".to_owned(),
                    config: json!({}),
                },
            )
            .expect("strategy snapshot"),
        )
        .expect("serialize strategy snapshot");
        json!({
            "run_id": "00000000-0000-4000-8000-000000000001",
            "strategy_config_id": "00000000-0000-4000-8000-000000000002",
            "as_of": "2026-08-19",
            "pins": {
                "candidate_content_sha256": hash(1),
                "artifact_manifest_sha256": hash(2),
                "stage5_manifest_sha256": hash(3),
                "action_manifest_sha256": hash(4),
                "approval_registry_sha256": hash(5),
            },
            "strategy": strategy,
        })
    }

    fn input_from(value: Value) -> OwnerBetaPriceRecommendationInput {
        serde_json::from_value(value).expect("valid sealed input fixture")
    }

    fn replay_fixture() -> (
        Uuid,
        OwnerBetaPriceRecommendationInput,
        OwnerBetaRunBinding,
        ReplayExpectation,
    ) {
        let input = input_from(input_value());
        let job_id = Uuid::parse_str("00000000-0000-4000-8000-000000000003").unwrap();
        let owner = Uuid::parse_str("00000000-0000-4000-8000-000000000004").unwrap();
        let as_of = NaiveDate::from_ymd_opt(2026, 8, 19).unwrap();
        let pins = ApprovalPinStrings::from_input(&input);
        let binding = OwnerBetaRunBinding {
            id: input.run_id(),
            job_id,
            owner_user_id: owner,
            strategy_config_id: input.strategy_config_id(),
            strategy_id: input.strategy_snapshot().strategy_id().to_owned(),
            strategy_version: input.strategy_snapshot().strategy_version().to_owned(),
            strategy_config_json: input.strategy_snapshot().config_json().clone(),
            strategy_config_sha256: input.strategy_snapshot().config_sha256().to_string(),
            as_of,
            status: "PENDING".to_owned(),
            candidate_content_sha256: pins.candidate_content_sha256.clone(),
            artifact_manifest_sha256: pins.artifact_manifest_sha256.clone(),
            stage5_manifest_sha256: pins.stage5_manifest_sha256.clone(),
            action_manifest_sha256: pins.action_manifest_sha256.clone(),
            approval_registry_sha256: pins.approval_registry_sha256.clone(),
        };
        let expected = ReplayExpectation {
            owner_user_id: owner,
            strategy_config_id: input.strategy_config_id(),
            as_of,
            as_of_trading: domain::TradingDate::parse("2026-08-19").unwrap(),
            pins,
        };
        (job_id, input, binding, expected)
    }

    #[test]
    fn replay_requires_exact_job_request_row_and_all_five_pins() {
        let (job_id, input, binding, expected) = replay_fixture();
        assert!(replay_binding_matches(
            job_id,
            OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE,
            &input,
            &binding,
            &expected,
        ));

        assert!(!replay_binding_matches(
            job_id,
            "recommendation",
            &input,
            &binding,
            &expected,
        ));
        let mut changed = binding.clone();
        changed.job_id = Uuid::new_v4();
        assert!(!replay_binding_matches(
            job_id,
            OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE,
            &input,
            &changed,
            &expected,
        ));
        for mutate in [
            |row: &mut OwnerBetaRunBinding| row.id = Uuid::new_v4(),
            |row: &mut OwnerBetaRunBinding| row.owner_user_id = Uuid::new_v4(),
            |row: &mut OwnerBetaRunBinding| row.strategy_config_id = Uuid::new_v4(),
            |row: &mut OwnerBetaRunBinding| {
                row.as_of = NaiveDate::from_ymd_opt(2026, 8, 18).unwrap()
            },
        ] {
            let mut changed = binding.clone();
            mutate(&mut changed);
            assert!(!replay_binding_matches(
                job_id,
                OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE,
                &input,
                &changed,
                &expected,
            ));
        }

        for field in [
            "candidate_content_sha256",
            "artifact_manifest_sha256",
            "stage5_manifest_sha256",
            "action_manifest_sha256",
            "approval_registry_sha256",
        ] {
            let mut changed_value = input_value();
            changed_value["pins"][field] = json!(hash(9));
            let changed_input = input_from(changed_value);
            assert!(
                !replay_binding_matches(
                    job_id,
                    OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE,
                    &changed_input,
                    &binding,
                    &expected,
                ),
                "input {field} mismatch must fail"
            );

            let mut changed_row = binding.clone();
            match field {
                "candidate_content_sha256" => changed_row.candidate_content_sha256 = hash(9),
                "artifact_manifest_sha256" => changed_row.artifact_manifest_sha256 = hash(9),
                "stage5_manifest_sha256" => changed_row.stage5_manifest_sha256 = hash(9),
                "action_manifest_sha256" => changed_row.action_manifest_sha256 = hash(9),
                "approval_registry_sha256" => changed_row.approval_registry_sha256 = hash(9),
                _ => unreachable!(),
            }
            assert!(
                !replay_binding_matches(
                    job_id,
                    OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE,
                    &input,
                    &changed_row,
                    &expected,
                ),
                "run row {field} mismatch must fail"
            );
        }

        for (field, value) in [
            ("run_id", json!("00000000-0000-4000-8000-000000000098")),
            (
                "strategy_config_id",
                json!("00000000-0000-4000-8000-000000000099"),
            ),
            ("as_of", json!("2026-08-18")),
        ] {
            let mut changed_value = input_value();
            changed_value[field] = value;
            let changed_input = input_from(changed_value);
            assert!(!replay_binding_matches(
                job_id,
                OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE,
                &changed_input,
                &binding,
                &expected,
            ));
        }

        for (field, value) in [
            ("strategy_id", json!("trend_following")),
            ("strategy_version", json!("9.9.9")),
            ("config_json", json!({"changed": true})),
            ("config_sha256", json!(hash(9))),
        ] {
            let mut changed_value = input_value();
            changed_value["strategy"][field] = value;
            let changed_input = input_from(changed_value);
            assert!(
                !replay_binding_matches(
                    job_id,
                    OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE,
                    &changed_input,
                    &binding,
                    &expected,
                ),
                "input strategy {field} mismatch must fail"
            );

            let mut changed_row = binding.clone();
            match field {
                "strategy_id" => changed_row.strategy_id = "trend_following".to_owned(),
                "strategy_version" => changed_row.strategy_version = "9.9.9".to_owned(),
                "config_json" => changed_row.strategy_config_json = json!({"changed": true}),
                "config_sha256" => changed_row.strategy_config_sha256 = hash(9),
                _ => unreachable!(),
            }
            assert!(
                !replay_binding_matches(
                    job_id,
                    OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE,
                    &input,
                    &changed_row,
                    &expected,
                ),
                "run strategy {field} mismatch must fail"
            );
        }
    }

    #[test]
    fn submission_projection_is_exact_and_targets_only_dedicated_persistence() {
        let input = input_from(input_value());
        let projection = SubmissionProjection::from_input(input).expect("projection");
        let object = projection.payload_json.as_object().expect("payload object");
        assert_eq!(
            object.keys().cloned().collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "as_of".to_owned(),
                "pins".to_owned(),
                "run_id".to_owned(),
                "strategy_config_id".to_owned(),
                "strategy".to_owned(),
            ])
        );
        assert_eq!(
            object["pins"]
                .as_object()
                .expect("pins object")
                .keys()
                .cloned()
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "action_manifest_sha256".to_owned(),
                "approval_registry_sha256".to_owned(),
                "artifact_manifest_sha256".to_owned(),
                "candidate_content_sha256".to_owned(),
                "stage5_manifest_sha256".to_owned(),
            ])
        );
        assert_eq!(
            object["strategy"]
                .as_object()
                .expect("strategy object")
                .keys()
                .cloned()
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "config_json".to_owned(),
                "config_sha256".to_owned(),
                "strategy_id".to_owned(),
                "strategy_version".to_owned(),
            ])
        );
        assert!(JOB_INSERT_SQL.starts_with("INSERT INTO jobs"));
        assert!(RUN_INSERT_SQL.starts_with("INSERT INTO owner_beta_recommendation_runs"));
        assert!(!RUN_INSERT_SQL.contains("INSERT INTO recommendation_runs"));
        assert!(!RUN_INSERT_SQL.contains("target_portfolios"));
        assert!(!RUN_INSERT_SQL.contains("paper"));
    }

    #[test]
    fn owner_beta_job_type_is_excluded_from_all_existing_typed_workers() {
        for existing_worker_type in [
            "recommendation",
            "backtest",
            "candidate_compute",
            "paper_rebalance_preview",
        ] {
            assert_ne!(
                OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE, existing_worker_type,
                "existing typed worker must not claim owner-beta jobs"
            );
        }
    }

    #[test]
    fn owner_beta_item_reads_join_only_bounded_shared_instrument_metadata() {
        for query in [READ_ITEMS_SQL, READ_ITEMS_FOR_RUNS_SQL] {
            assert!(query.contains("LEFT JOIN public.instruments AS instrument"));
            assert!(query.contains("instrument.name AS instrument_name"));
            assert!(query.contains("instrument.asset_class AS instrument_asset_class"));
            assert!(query.contains("LIMIT $3"));
            assert!(!query.contains("strategy_config"));
        }
    }

    #[test]
    fn owner_beta_item_read_limit_is_checked_and_scaled_for_multi_run_reads() {
        let per_run = i64::try_from(KR_ETF_CORE_SYMBOLS.len() + 1).unwrap();
        assert_eq!(item_row_limit(1).expect("detail item limit"), per_run);
        assert_eq!(
            item_row_limit(3).expect("multi-run item limit"),
            per_run * 3
        );
        assert!(item_row_limit(usize::MAX).is_err());
    }

    fn read_row(status: &str) -> OwnerBetaPriceOnlyReadRunRow {
        let created_at = chrono::DateTime::parse_from_rfc3339("2026-08-19T00:00:00Z")
            .expect("created timestamp")
            .with_timezone(&chrono::Utc);
        let started_at = (status != "PENDING" && status != "CANCELED").then_some(created_at);
        let finished_at = matches!(status, "SUCCEEDED" | "FAILED" | "CANCELED").then_some(
            chrono::DateTime::parse_from_rfc3339("2026-08-19T00:01:00Z")
                .expect("finished timestamp")
                .with_timezone(&chrono::Utc),
        );
        OwnerBetaPriceOnlyReadRunRow {
            id: Uuid::parse_str("00000000-0000-4000-8000-000000000010").unwrap(),
            job_id: Uuid::parse_str("00000000-0000-4000-8000-000000000011").unwrap(),
            strategy_config_id: Uuid::parse_str("00000000-0000-4000-8000-000000000012").unwrap(),
            strategy_id: "buy_and_hold".to_owned(),
            strategy_version: "1.0.0".to_owned(),
            as_of: NaiveDate::from_ymd_opt(2026, 8, 19).unwrap(),
            status: status.to_owned(),
            input_kind: OWNER_BETA_PRICE_INPUT_KIND.to_owned(),
            capability: OWNER_BETA_PRICE_CAPABILITY.to_owned(),
            audience: OWNER_BETA_PRICE_AUDIENCE.to_owned(),
            vendor_snapshot: true,
            strict_pit: false,
            strategy_config_sha256: hash(1),
            candidate_content_sha256: hash(2),
            artifact_manifest_sha256: hash(3),
            stage5_manifest_sha256: hash(4),
            action_manifest_sha256: hash(5),
            approval_registry_sha256: hash(6),
            factor_snapshot_sha256: (status == "SUCCEEDED").then(|| hash(7)),
            target_snapshot_sha256: (status == "SUCCEEDED").then(|| hash(8)),
            cash_weight: (status == "SUCCEEDED").then(|| "0.000000".to_owned()),
            error_code: match status {
                "FAILED" => Some("OWNER_BETA_COMPUTATION_FAILED".to_owned()),
                "CANCELED" => Some(OWNER_BETA_CANCELED_CODE.to_owned()),
                _ => None,
            },
            created_at,
            started_at,
            finished_at,
            updated_at: finished_at.unwrap_or(created_at),
            item_count: if status == "SUCCEEDED" { 11 } else { 0 },
        }
    }

    fn read_item(index: usize, selected: bool) -> OwnerBetaPriceOnlyReadItemRow {
        let instrument_id = format!("{}.KRX", KR_ETF_CORE_SYMBOLS[index]);
        let reason = if selected {
            "SELECTED_TOP_N"
        } else {
            "NOT_SELECTED_BY_STRATEGY"
        };
        OwnerBetaPriceOnlyReadItemRow {
            recommendation_run_id: Uuid::parse_str("00000000-0000-4000-8000-000000000010").unwrap(),
            instrument_id,
            instrument_name: None,
            instrument_asset_class: None,
            rank: selected.then_some((index + 1) as i32),
            target_weight: selected.then(|| "0.500000".to_owned()),
            reason_codes: json!([reason]),
            factors_json: json!({"close": "100.0"}),
            excluded: !selected,
            exclusion_reason: (!selected).then(|| reason.to_owned()),
        }
    }

    #[test]
    fn owner_beta_read_model_accepts_fixed_success_and_canceled_without_start() {
        let mut row = read_row("SUCCEEDED");
        let mut items = (0..KR_ETF_CORE_SYMBOLS.len())
            .map(|index| read_item(index, index < 2))
            .collect::<Vec<_>>();
        row.cash_weight = Some("0.000000".to_owned());
        assert!(validate_read_model(&row, &items).is_ok());

        let canceled = read_row("CANCELED");
        assert!(canceled.started_at.is_none());
        assert!(validate_read_model(&canceled, &[]).is_ok());

        items[1].rank = Some(1);
        assert!(validate_read_model(&row, &items).is_err());
    }

    #[test]
    fn owner_beta_read_model_rejects_tampered_fixed_fields_hashes_and_state() {
        let mut row = read_row("SUCCEEDED");
        let items = (0..KR_ETF_CORE_SYMBOLS.len())
            .map(|index| read_item(index, index < 2))
            .collect::<Vec<_>>();
        assert!(validate_read_model(&row, &items).is_ok());

        row.input_kind = "other_input".to_owned();
        assert!(validate_read_model(&row, &items).is_err());
        row.input_kind = OWNER_BETA_PRICE_INPUT_KIND.to_owned();
        row.strategy_config_sha256 = format!("sha256:{}", "A".repeat(64));
        assert!(validate_read_model(&row, &items).is_err());
        row.strategy_config_sha256 = hash(1);
        row.cash_weight = Some("0.1".to_owned());
        assert!(validate_read_model(&row, &items).is_err());

        let mut failed = read_row("FAILED");
        failed.item_count = 1;
        assert!(validate_read_model(&failed, &[]).is_err());
        failed.item_count = 0;
        failed.error_code = Some("provider leaked detail".to_owned());
        assert!(validate_read_model(&failed, &[]).is_err());
    }

    #[test]
    fn owner_beta_read_model_rejects_impossible_lifecycle_timestamps() {
        let one_minute = chrono::TimeDelta::minutes(1);

        let mut canceled = read_row("CANCELED");
        canceled.finished_at = Some(canceled.created_at - one_minute);
        canceled.updated_at = canceled.created_at;
        assert!(validate_read_model(&canceled, &[]).is_err());

        let mut running = read_row("RUNNING");
        running.started_at = Some(running.created_at + one_minute);
        running.updated_at = running.created_at;
        assert!(validate_read_model(&running, &[]).is_err());

        let mut failed = read_row("FAILED");
        failed.updated_at = failed
            .finished_at
            .expect("failed fixture has a finish timestamp")
            - one_minute;
        assert!(validate_read_model(&failed, &[]).is_err());
    }
}
