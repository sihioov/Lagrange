//! Artifact ownership: `result_artifacts` rows are only reachable through an
//! owned parent `backtest_run`. RLS on both tables plus an explicit join keeps
//! a foreign artifact (direct ID guess / replay) invisible.

use crate::actor_tx::{actor_uuid, begin_actor_tx};
use crate::error::{TenancyError, TenancyResult};
use auth::entitlement::Actor;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

/// A row of `result_artifacts` as the actor may see it (ownership derived from
/// the parent run, per schema 0006).
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct ArtifactRow {
    pub id: Uuid,
    pub backtest_run_id: Uuid,
    pub owner_user_id: Option<Uuid>,
    pub artifact_type: String,
    pub parquet_path: String,
    pub row_count: i64,
    pub sha256: String,
    pub size_bytes: i64,
    pub summary_json: Value,
    pub created_at: DateTime<Utc>,
}

/// Typed repository over `result_artifacts` (read path; writes are the
/// worker's, per grants 0009).
#[derive(Debug, Clone)]
pub struct ArtifactRepo {
    pool: sqlx::PgPool,
}

impl ArtifactRepo {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    /// Fetch one of `actor`'s artifacts by id, requiring the PARENT RUN to be
    /// owned by the actor. Both tables are RLS-scoped, so a foreign artifact
    /// (or a direct id guess) yields zero rows => NotFound.
    pub async fn get_owned(&self, actor: &Actor, artifact_id: Uuid) -> TenancyResult<ArtifactRow> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let row = sqlx::query_as::<_, ArtifactRow>(
            "SELECT a.id, a.backtest_run_id, a.owner_user_id, a.artifact_type, \
                    a.parquet_path, a.row_count, a.sha256, a.size_bytes, \
                    a.summary_json, a.created_at \
             FROM result_artifacts a \
             JOIN backtest_runs r ON r.id = a.backtest_run_id \
             WHERE a.id = $1 AND r.owner_user_id = $2",
        )
        .bind(artifact_id)
        .bind(actor_uuid(actor)?)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        crate::error::map_optional(row)
    }
}

/// The path prefix the artifact tree is mounted under (compose
/// `LAGRANGE_ARTIFACTS_DIR`). Stored `parquet_path` values may or may not
/// carry it; it is stripped (never trusted) before the internal path is
/// validated. Nginx serves `/internal-artifacts/<rel>` from the same root.
pub const ARTIFACT_URL_PREFIX: &str = "data/artifacts/";

/// Derive the INTERNAL alias path (`/internal-artifacts/<rel>`) from a stored
/// `parquet_path`. Fail-closed: absolute paths, backslashes, drive/UNC
/// markers, and `..`/`.` segments are rejected so a poisoned manifest can
/// never escape the artifact root or reach an internal Nginx target outside
/// the alias (symlink escapes are additionally blocked by `disable_symlinks`).
pub fn derive_internal_path(parquet_path: &str) -> TenancyResult<String> {
    let rel = match parquet_path.strip_prefix(ARTIFACT_URL_PREFIX) {
        Some(rest) => rest,
        None => parquet_path,
    };
    if rel.is_empty()
        || rel.starts_with('/')
        || rel.starts_with('\\')
        || rel.contains('\\')
        || rel.contains(':')
    {
        return Err(TenancyError::ResultIntegrity(
            "artifact path is not a safe relative path".to_string(),
        ));
    }
    for segment in rel.split('/') {
        if segment.is_empty() || segment == ".." || segment == "." {
            return Err(TenancyError::ResultIntegrity(
                "artifact path contains unsafe segments".to_string(),
            ));
        }
    }
    Ok(rel.to_string())
}

/// The media type served for an artifact: derived from the constrained
/// `artifact_type` (all result artifacts are Parquet files), never from the
/// client or the file extension. Unknown types fail closed to a binary type.
pub fn media_type(artifact_type: &str) -> &'static str {
    match artifact_type {
        "EQUITY_CURVE" | "DRAWDOWN_CURVE" | "MONTHLY_RETURNS" | "ORDERS" | "FILLS"
        | "POSITIONS" | "CASH_LEDGER" | "FEES" | "BENCHMARK" => "application/vnd.apache.parquet",
        _ => "application/octet-stream",
    }
}
