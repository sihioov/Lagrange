//! Actor-scoped repository over `recommendation_runs` / `recommendation_items`
//! (tenant tables, FORCE RLS). Creation writes a PENDING run; the research
//! worker settles status + items. Reads are owner-scoped via the actor GUC.

use crate::actor_tx::{actor_uuid, begin_actor_tx};
use crate::error::{TenancyError, TenancyResult};
use crate::http::pagination::Cursor;
use auth::entitlement::Actor;
use chrono::{DateTime, NaiveDate, Utc};
use job_queue::recommendation::input::{DatasetPin, RecommendationPayload};
use serde_json::Value;
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct RecommendationRunRow {
    pub id: Uuid,
    pub strategy_config_id: Option<Uuid>,
    pub as_of: NaiveDate,
    pub status: String,
    pub summary_json: Value,
    pub created_at: DateTime<Utc>,
    pub job_id: Option<Uuid>,
    pub trigger_kind: String,
    pub dataset_version_id: Option<Uuid>,
    pub dataset_manifest_sha256: Option<String>,
}

/// The submission boundary keeps the run and its queue row in one actor
/// transaction. The caller cannot supply any owner or lineage fields.
#[derive(Debug, Clone)]
pub struct SubmitRecommendation {
    pub strategy_config_id: Uuid,
    pub as_of: NaiveDate,
    pub dataset: DatasetPin,
    pub idempotency_key: Option<String>,
    pub max_jobs_per_owner: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum SubmitRecommendationError {
    #[error(transparent)]
    Tenancy(#[from] TenancyError),
    #[error("per-owner recommendation capacity exceeded")]
    CapacityExceeded,
    #[error("idempotency key was already used with different recommendation input")]
    IdempotencyMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct RecommendationItemRow {
    pub instrument_id: String,
    pub rank: Option<i32>,
    pub target_weight: Option<String>,
    pub reason_codes: Value,
    pub factors_json: Value,
    pub excluded: bool,
    pub exclusion_reason: Option<String>,
}

#[derive(Debug, FromRow)]
struct TaggedRecommendationRunRow {
    kind: String,
    id: Uuid,
    strategy_config_id: Option<Uuid>,
    as_of: NaiveDate,
    status: String,
    summary_json: Value,
    created_at: DateTime<Utc>,
    job_id: Option<Uuid>,
    trigger_kind: String,
    dataset_version_id: Option<Uuid>,
    dataset_manifest_sha256: Option<String>,
}

impl From<TaggedRecommendationRunRow> for RecommendationRunRow {
    fn from(row: TaggedRecommendationRunRow) -> Self {
        Self {
            id: row.id,
            strategy_config_id: row.strategy_config_id,
            as_of: row.as_of,
            status: row.status,
            summary_json: row.summary_json,
            created_at: row.created_at,
            job_id: row.job_id,
            trigger_kind: row.trigger_kind,
            dataset_version_id: row.dataset_version_id,
            dataset_manifest_sha256: row.dataset_manifest_sha256,
        }
    }
}

/// Repository over the recommendation tenant tables.
#[derive(Debug, Clone)]
pub struct RecommendationRepo {
    pool: sqlx::PgPool,
}

impl RecommendationRepo {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    /// Insert the PENDING run and its QUEUED recommendation job atomically.
    /// The owner advisory lock makes the capacity check serializable per
    /// tenant; the namespaced queue key supplies durable duplicate protection
    /// across API instances.
    pub async fn submit(
        &self,
        actor: &Actor,
        input: SubmitRecommendation,
    ) -> Result<RecommendationRunRow, SubmitRecommendationError> {
        let owner = actor_uuid(actor)?;
        let mut tx = begin_actor_tx(&self.pool, actor).await?;

        // Serialize submissions for one owner so a concurrent capacity check
        // cannot admit more than the configured number of active jobs.
        crate::repos::lock_owner_job_capacity(&mut tx, owner).await?;

        let queue_key = input
            .idempotency_key
            .as_deref()
            .map(|key| format!("recommendation:manual:{key}"));
        if let Some(key) = queue_key.as_deref() {
            let existing: Option<(Uuid, String, Value)> = sqlx::query_as(
                "SELECT id, job_type, payload_json FROM jobs \
                 WHERE owner_user_id = $1 AND idempotency_key = $2 \
                 FOR SHARE",
            )
            .bind(owner)
            .bind(key)
            .fetch_optional(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?;
            if let Some((job_id, job_type, payload)) = existing {
                let matches = RecommendationPayload::try_from(payload)
                    .map(|payload| {
                        job_type == "recommendation"
                            && payload.strategy_config_id == input.strategy_config_id
                            && payload.as_of == input.as_of
                    })
                    .unwrap_or(false);
                if !matches {
                    return Err(SubmitRecommendationError::IdempotencyMismatch);
                }
                let row = select_run_by_job(&mut tx, job_id).await?;
                tx.commit().await.map_err(TenancyError::from_sqlx)?;
                return Ok(row);
            }
        }

        let config_active: Option<bool> = sqlx::query_scalar(
            "SELECT is_active FROM user_strategy_configs WHERE id = $1 FOR SHARE",
        )
        .bind(input.strategy_config_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        if config_active != Some(true) {
            return Err(TenancyError::NotFound.into());
        }

        let dataset_ready: bool = sqlx::query_scalar(
            "SELECT public.lock_recommendation_submission_dataset($1, $2, $3, $4)",
        )
        .bind(input.dataset.id)
        .bind(&input.dataset.dataset_id)
        .bind(&input.dataset.version)
        .bind(&input.dataset.manifest_sha256)
        .fetch_one(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        if !dataset_ready {
            return Err(TenancyError::DatasetBlocked(
                "configured dataset pin is missing, mismatched, or not READY".into(),
            )
            .into());
        }

        let active_jobs: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM jobs WHERE owner_user_id = $1 AND status IN ('QUEUED', 'RUNNING')",
        )
        .bind(owner)
        .fetch_one(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        if active_jobs >= input.max_jobs_per_owner as i64 {
            return Err(SubmitRecommendationError::CapacityExceeded);
        }

        let run_id = Uuid::new_v4();
        let job_id = Uuid::new_v4();
        let payload = serde_json::to_value(RecommendationPayload {
            run_id,
            strategy_config_id: input.strategy_config_id,
            as_of: input.as_of,
            dataset: input.dataset.clone(),
        })
        .expect("recommendation payload serializes");
        sqlx::query(
            "INSERT INTO jobs \
             (id, owner_user_id, job_type, status, priority, idempotency_key, payload_json, max_attempts, available_at) \
             VALUES ($1, $2, 'recommendation', 'QUEUED', 10, $3, $4, 3, now())",
        )
        .bind(job_id)
        .bind(owner)
        .bind(&queue_key)
        .bind(payload)
        .execute(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;

        let row = sqlx::query_as::<_, RecommendationRunRow>(
            "INSERT INTO recommendation_runs \
             (id, owner_user_id, strategy_config_id, as_of, status, job_id, trigger_kind, dataset_version_id, dataset_manifest_sha256) \
             VALUES ($1, $2, $3, $4, 'PENDING', $5, 'MANUAL', $6, $7) \
             RETURNING id, strategy_config_id, as_of, status, summary_json, created_at, \
                       job_id, trigger_kind, dataset_version_id, dataset_manifest_sha256",
        )
        .bind(run_id)
        .bind(owner)
        .bind(input.strategy_config_id)
        .bind(input.as_of)
        .bind(job_id)
        .bind(input.dataset.id)
        .bind(&input.dataset.manifest_sha256)
        .fetch_one(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(row)
    }

    pub async fn get_run(&self, actor: &Actor, id: Uuid) -> TenancyResult<RecommendationRunRow> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let row = sqlx::query_as::<_, RecommendationRunRow>(
            "SELECT id, strategy_config_id, as_of, status, summary_json, created_at, \
                    job_id, trigger_kind, dataset_version_id, dataset_manifest_sha256 \
             FROM recommendation_runs WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        crate::error::map_optional(row)
    }

    /// Keyset-paginated list of the actor's runs (stable `created_at, id`).
    pub async fn list_runs(
        &self,
        actor: &Actor,
        after: Option<&Cursor>,
        limit: usize,
    ) -> TenancyResult<(Vec<RecommendationRunRow>, Option<Cursor>)> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let sql = match after {
            Some(_) => {
                "SELECT id, strategy_config_id, as_of, status, summary_json, created_at, \
                         job_id, trigger_kind, dataset_version_id, dataset_manifest_sha256 \
                 FROM recommendation_runs WHERE (created_at, id) < ($1::timestamptz, $2::uuid) \
                 ORDER BY created_at DESC, id DESC LIMIT $3"
            }
            None => {
                "SELECT id, strategy_config_id, as_of, status, summary_json, created_at, \
                        job_id, trigger_kind, dataset_version_id, dataset_manifest_sha256 \
                 FROM recommendation_runs ORDER BY created_at DESC, id DESC LIMIT $1"
            }
        };
        let mut q = sqlx::query_as::<_, RecommendationRunRow>(sql);
        if let Some(c) = after {
            q = q
                .bind(c.k.clone())
                .bind(uuid::Uuid::parse_str(&c.i).map_err(|_| TenancyError::NotFound)?);
        }
        let rows = q
            .bind(limit as i64 + 1)
            .fetch_all(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(crate::repos::split_page(rows, limit, |r| {
            (r.created_at.to_rfc3339(), r.id.to_string())
        }))
    }

    /// The latest successful report and newest run metadata are deliberately
    /// independent: a pending/failed submission must not hide usable advice.
    pub async fn latest_snapshot(
        &self,
        actor: &Actor,
        config_id: Option<Uuid>,
    ) -> TenancyResult<(
        Option<RecommendationRunRow>,
        Option<RecommendationRunRow>,
        Vec<RecommendationItemRow>,
    )> {
        let mut tx = self.pool.begin().await.map_err(TenancyError::from_sqlx)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .execute(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?;
        crate::actor_tx::set_actor_guc(&mut tx, actor).await?;
        // Both metadata rows come from one statement, and REPEATABLE READ
        // keeps the following item query on that exact PostgreSQL snapshot.
        let rows = sqlx::query_as::<_, TaggedRecommendationRunRow>(
            "(SELECT 'successful'::text AS kind, id, strategy_config_id, as_of, status, \
                     summary_json, created_at, job_id, trigger_kind, dataset_version_id, \
                     dataset_manifest_sha256 \
              FROM recommendation_runs \
              WHERE ($1::uuid IS NULL OR strategy_config_id = $1) AND status = 'SUCCEEDED' \
              ORDER BY created_at DESC, id DESC LIMIT 1) \
             UNION ALL \
             (SELECT 'newest'::text AS kind, id, strategy_config_id, as_of, status, \
                     summary_json, created_at, job_id, trigger_kind, dataset_version_id, \
                     dataset_manifest_sha256 \
              FROM recommendation_runs \
              WHERE ($1::uuid IS NULL OR strategy_config_id = $1) \
              ORDER BY created_at DESC, id DESC LIMIT 1)",
        )
        .bind(config_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        let mut successful: Option<RecommendationRunRow> = None;
        let mut newest: Option<RecommendationRunRow> = None;
        for row in rows {
            match row.kind.as_str() {
                "successful" => successful = Some(row.into()),
                "newest" => newest = Some(row.into()),
                _ => unreachable!("latest query only emits fixed kind values"),
            }
        }
        let items = match successful.as_ref() {
            Some(run) => sqlx::query_as::<_, RecommendationItemRow>(
                "SELECT instrument_id, rank, target_weight::text, reason_codes, factors_json, \
                        excluded, exclusion_reason \
                 FROM recommendation_items WHERE recommendation_run_id = $1 \
                 ORDER BY rank NULLS LAST, instrument_id",
            )
            .bind(run.id)
            .fetch_all(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?,
            None => Vec::new(),
        };
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok((successful, newest, items))
    }

    /// Items of one of the actor's runs (empty when not settled yet).
    pub async fn items(
        &self,
        actor: &Actor,
        run_id: Uuid,
    ) -> TenancyResult<Vec<RecommendationItemRow>> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let rows = sqlx::query_as::<_, RecommendationItemRow>(
            "SELECT instrument_id, rank, target_weight::text, reason_codes, factors_json, \
                    excluded, exclusion_reason \
             FROM recommendation_items WHERE recommendation_run_id = $1 \
             ORDER BY rank NULLS LAST, instrument_id",
        )
        .bind(run_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(rows)
    }
}

async fn select_run_by_job(
    tx: &mut Transaction<'_, Postgres>,
    job_id: Uuid,
) -> TenancyResult<RecommendationRunRow> {
    let row = sqlx::query_as::<_, RecommendationRunRow>(
        "SELECT id, strategy_config_id, as_of, status, summary_json, created_at, \
                job_id, trigger_kind, dataset_version_id, dataset_manifest_sha256 \
         FROM recommendation_runs WHERE job_id = $1",
    )
    .bind(job_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(TenancyError::from_sqlx)?;
    crate::error::map_optional(row)
}
