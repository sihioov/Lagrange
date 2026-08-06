//! Actor-scoped reads of `backtest_metrics`, `backtest_warnings`, and the
//! `result_artifacts` manifests of one run (tenant tables, FORCE RLS; the
//! artifact read requires the parent run to be owned - same join as
//! [`crate::repos::artifacts::ArtifactRepo`]).

use crate::actor_tx::{actor_uuid, begin_actor_tx};
use crate::error::{TenancyError, TenancyResult};
use crate::repos::artifacts::ArtifactRow;
use auth::entitlement::Actor;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct MetricRow {
    pub metric_key: String,
    pub metric_value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct WarningRow {
    pub warning_code: String,
    pub message: String,
}

/// Read-only repository over per-run result tables.
#[derive(Debug, Clone)]
pub struct MetricsRepo {
    pool: sqlx::PgPool,
}

impl MetricsRepo {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    pub async fn metrics(&self, actor: &Actor, run_id: Uuid) -> TenancyResult<Vec<MetricRow>> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let rows = sqlx::query_as::<_, MetricRow>(
            "SELECT metric_key, metric_value::text FROM backtest_metrics \
             WHERE backtest_run_id = $1 ORDER BY metric_key",
        )
        .bind(run_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(rows)
    }

    pub async fn warnings(&self, actor: &Actor, run_id: Uuid) -> TenancyResult<Vec<WarningRow>> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let rows = sqlx::query_as::<_, WarningRow>(
            "SELECT warning_code, message FROM backtest_warnings \
             WHERE backtest_run_id = $1 ORDER BY created_at",
        )
        .bind(run_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(rows)
    }

    /// Artifact manifests of one owned run (RLS + explicit owner join).
    pub async fn artifacts(&self, actor: &Actor, run_id: Uuid) -> TenancyResult<Vec<ArtifactRow>> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let rows = sqlx::query_as::<_, ArtifactRow>(
            "SELECT a.id, a.backtest_run_id, a.owner_user_id, a.artifact_type, \
                    a.parquet_path, a.row_count, a.sha256, a.size_bytes, \
                    a.summary_json, a.created_at \
             FROM result_artifacts a \
             JOIN backtest_runs r ON r.id = a.backtest_run_id \
             WHERE a.backtest_run_id = $1 AND r.owner_user_id = $2 \
             ORDER BY a.artifact_type",
        )
        .bind(run_id)
        .bind(actor_uuid(actor)?)
        .fetch_all(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(rows)
    }
}
