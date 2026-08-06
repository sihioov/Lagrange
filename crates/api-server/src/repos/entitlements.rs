//! Loads the fail-closed entitlement service from `data_entitlements`
//! (shared table, SELECT to app). The DB mirror has no `covered_users`
//! column, so the contract-level user list (the redacted krx.schema.json
//! side) is represented as "every platform user": the lifecycle state
//! machine (ACTIVE/PENDING/EXPIRED/REVOKED + effective window) remains the
//! real gate. An empty table or a malformed row fails closed.

use crate::error::{TenancyError, TenancyResult};
use auth::entitlement::{
    CalendarDate, ContractRef, DataProvider, DatasetId, DocumentHash, Entitlement,
    EntitlementBuilder, EntitlementId, EntitlementState, KrUse, UserId,
};
use sqlx::Row;

#[derive(Debug, Clone, FromRow)]
struct EntitlementRow {
    id: Uuid,
    contract_document_sha256: String,
    contract_reference: String,
    status: String,
    covered_datasets: serde_json::Value,
    covered_uses: serde_json::Value,
    effective_from: chrono::NaiveDate,
    effective_until: chrono::NaiveDate,
}

/// Repository over `data_entitlements` (shared, read-only).
#[derive(Debug, Clone)]
pub struct EntitlementRepo {
    pool: sqlx::PgPool,
}

impl EntitlementRepo {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    /// Load every entitlement row as an [`Entitlement`]; covered users are
    /// all users known to the platform (see module docs).
    pub async fn load(&self) -> TenancyResult<Vec<Entitlement>> {
        let users: Vec<String> = sqlx::query("SELECT id::text AS id FROM users")
            .fetch_all(&self.pool)
            .await
            .map_err(TenancyError::from_sqlx)?
            .iter()
            .map(|r| r.get::<String, _>("id"))
            .collect();
        let rows: Vec<EntitlementRow> = sqlx::query_as(
            "SELECT id, contract_document_sha256, contract_reference, status, \
                    covered_datasets, covered_uses, effective_from, effective_until \
             FROM data_entitlements",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(TenancyError::from_sqlx)?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let lifecycle = match r.status.as_str() {
                "PENDING" => EntitlementState::Pending,
                "ACTIVE" => EntitlementState::Active,
                "EXPIRED" => EntitlementState::Expired,
                "REVOKED" => EntitlementState::Revoked,
                _ => continue,
            };
            let from = match CalendarDate::parse(&r.effective_from.format("%Y-%m-%d").to_string()) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let until = match CalendarDate::parse(&r.effective_until.format("%Y-%m-%d").to_string())
            {
                Ok(d) => d,
                Err(_) => continue,
            };
            let datasets: Vec<DatasetId> = r
                .covered_datasets
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str())
                .map(DatasetId::new)
                .collect();
            let uses: Vec<KrUse> = r
                .covered_uses
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str())
                .filter_map(crate::http::entitlement::use_from_name)
                .collect();
            if datasets.is_empty() || uses.is_empty() {
                continue;
            }
            out.push(
                EntitlementBuilder::new()
                    .id(EntitlementId::new(r.id.to_string()))
                    .provider(DataProvider::Krx)
                    .contract(ContractRef::new(
                        DocumentHash::sha256(r.contract_document_sha256),
                        r.contract_reference,
                    ))
                    .lifecycle(lifecycle)
                    .effective(from, until)
                    .covered_datasets(datasets)
                    .covered_uses(uses)
                    .covered_users(users.iter().cloned().map(UserId::new))
                    .build(),
            );
        }
        Ok(out)
    }
}

use sqlx::FromRow;
use uuid::Uuid;
