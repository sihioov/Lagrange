//! Actor-scoped repository over `recommendation_runs` / `recommendation_items`
//! (tenant tables, FORCE RLS). Creation writes a PENDING run; the research
//! worker settles status + items. Reads are owner-scoped via the actor GUC.

use crate::actor_tx::{actor_uuid, begin_actor_tx};
use crate::error::{TenancyError, TenancyResult};
use crate::http::pagination::Cursor;
use auth::entitlement::Actor;
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct RecommendationRunRow {
    pub id: Uuid,
    pub strategy_config_id: Option<Uuid>,
    pub as_of: NaiveDate,
    pub status: String,
    pub summary_json: Value,
    pub created_at: DateTime<Utc>,
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

/// Repository over the recommendation tenant tables.
#[derive(Debug, Clone)]
pub struct RecommendationRepo {
    pool: sqlx::PgPool,
}

impl RecommendationRepo {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    /// Create a PENDING run for `actor` (job enqueued by the caller).
    pub async fn create_run(
        &self,
        actor: &Actor,
        strategy_config_id: Uuid,
        as_of: NaiveDate,
    ) -> TenancyResult<RecommendationRunRow> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let row = sqlx::query_as::<_, RecommendationRunRow>(
            "INSERT INTO recommendation_runs (owner_user_id, strategy_config_id, as_of, status) \
             VALUES ($1, $2, $3, 'PENDING') \
             RETURNING id, strategy_config_id, as_of, status, summary_json, created_at",
        )
        .bind(actor_uuid(actor)?)
        .bind(strategy_config_id)
        .bind(as_of)
        .fetch_one(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(row)
    }

    pub async fn get_run(&self, actor: &Actor, id: Uuid) -> TenancyResult<RecommendationRunRow> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let row = sqlx::query_as::<_, RecommendationRunRow>(
            "SELECT id, strategy_config_id, as_of, status, summary_json, created_at \
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
                "SELECT id, strategy_config_id, as_of, status, summary_json, created_at \
                 FROM recommendation_runs WHERE (created_at, id) > ($1::timestamptz, $2::uuid) \
                 ORDER BY created_at, id LIMIT $3"
            }
            None => {
                "SELECT id, strategy_config_id, as_of, status, summary_json, created_at \
                 FROM recommendation_runs ORDER BY created_at, id LIMIT $1"
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

    /// The latest run for `actor` (optionally for one strategy config).
    pub async fn latest_run(
        &self,
        actor: &Actor,
        config_id: Option<Uuid>,
    ) -> TenancyResult<Option<RecommendationRunRow>> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let row = match config_id {
            Some(cfg) => sqlx::query_as::<_, RecommendationRunRow>(
                "SELECT id, strategy_config_id, as_of, status, summary_json, created_at \
                     FROM recommendation_runs WHERE strategy_config_id = $1 \
                     ORDER BY created_at DESC, id DESC LIMIT 1",
            )
            .bind(cfg)
            .fetch_optional(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?,
            None => sqlx::query_as::<_, RecommendationRunRow>(
                "SELECT id, strategy_config_id, as_of, status, summary_json, created_at \
                     FROM recommendation_runs ORDER BY created_at DESC, id DESC LIMIT 1",
            )
            .fetch_optional(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?,
        };
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(row)
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
