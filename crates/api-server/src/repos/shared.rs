//! Shared system-owned data: instruments, dataset versions, factor snapshot
//! manifests. Read-only for serving roles (grant 0009 + RLS SELECT-only
//! policies). Methods still take the authenticated actor so the API layer has
//! one uniform signature and can audit downstream; no actor GUC is required
//! (the shared tables are not row-scoped).

use crate::actor_tx::begin_actor_tx;
use crate::error::{TenancyError, TenancyResult};
use auth::entitlement::Actor;
use chrono::{DateTime, NaiveDate, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// A row of `instruments`.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct InstrumentRow {
    pub id: String,
    pub symbol: String,
    pub venue: String,
    pub currency: String,
    pub name: Option<String>,
    pub asset_class: String,
    pub status: String,
}

/// A row of `dataset_versions`.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct DatasetVersionRow {
    pub id: Uuid,
    pub dataset_id: String,
    pub version: String,
    pub status: String,
    pub storage_path: String,
    pub created_at: DateTime<Utc>,
}

/// A row of `factor_snapshot_manifests`.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct SnapshotManifestRow {
    pub id: Uuid,
    pub factor_definition_id: String,
    pub snapshot_date: NaiveDate,
    pub storage_path: String,
    pub row_count: i64,
    pub created_at: DateTime<Utc>,
}

/// Typed read-only repository over shared system-owned tables.
#[derive(Debug, Clone)]
pub struct SharedDataRepo {
    pool: sqlx::PgPool,
}

impl SharedDataRepo {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    /// Read one instrument (shared, read-only).
    pub async fn get_instrument(&self, _actor: &Actor, id: &str) -> TenancyResult<InstrumentRow> {
        let mut tx = begin_actor_tx(&self.pool, _actor).await?;
        let row = sqlx::query_as::<_, InstrumentRow>(
            "SELECT id, symbol, venue, currency, name, asset_class, status \
             FROM instruments WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        crate::error::map_optional(row)
    }

    /// Read one dataset version (shared, read-only).
    pub async fn get_dataset_version(
        &self,
        _actor: &Actor,
        dataset_id: &str,
        version: &str,
    ) -> TenancyResult<DatasetVersionRow> {
        let mut tx = begin_actor_tx(&self.pool, _actor).await?;
        let row = sqlx::query_as::<_, DatasetVersionRow>(
            "SELECT id, dataset_id, version, status, storage_path, created_at \
             FROM dataset_versions WHERE dataset_id = $1 AND version = $2",
        )
        .bind(dataset_id)
        .bind(version)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        crate::error::map_optional(row)
    }

    /// List factor snapshot manifests (shared, read-only).
    pub async fn list_factor_manifests(
        &self,
        _actor: &Actor,
    ) -> TenancyResult<Vec<SnapshotManifestRow>> {
        let mut tx = begin_actor_tx(&self.pool, _actor).await?;
        let rows = sqlx::query_as::<_, SnapshotManifestRow>(
            "SELECT id, factor_definition_id, snapshot_date, storage_path, row_count, \
                    created_at \
             FROM factor_snapshot_manifests ORDER BY snapshot_date",
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(rows)
    }
}
