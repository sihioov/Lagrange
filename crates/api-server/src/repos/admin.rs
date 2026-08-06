//! Explicit, audited admin pathway.
//!
//! Admin operations are the ONLY cross-user reads in the system and run over
//! the dedicated `admin` database role (SELECT-only, no BYPASSRLS — migration
//! 0010 grants it `USING (true)` SELECT policies). Repository-level gating:
//! every call requires `actor.role == Owner`; Members are denied (403) and the
//! denial itself is audited. Every successful call writes an audit row
//! (actor/time/target/reason/correlation id) through the append-only
//! `audit_writer` pool.

use crate::actor_tx::begin_actor_tx;
use crate::error::{TenancyError, TenancyResult};
use crate::repos::audit::{AuditEntry, AuditWriter};
use auth::entitlement::Actor;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// A row of `jobs` as the admin may see it (cross-user queue view).
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct JobRow {
    pub id: Uuid,
    pub owner_user_id: Uuid,
    pub job_type: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

/// Admin views (cross-user, Owner-only, audited).
#[derive(Debug, Clone)]
pub struct AdminRepo {
    admin_pool: sqlx::PgPool,
    audit: AuditWriter,
}

impl AdminRepo {
    pub fn new(admin_pool: sqlx::PgPool, audit: AuditWriter) -> Self {
        Self { admin_pool, audit }
    }

    /// Owner-only: list every job across all users (FR-ADM-001 queue view).
    /// Audited with the caller's actor/time/target/reason/correlation id.
    pub async fn list_all_jobs(
        &self,
        actor: &Actor,
        correlation_id: &str,
    ) -> TenancyResult<Vec<JobRow>> {
        if !actor.is_owner() {
            self.audit
                .record(
                    actor,
                    &AuditEntry {
                        action: "admin.jobs.list_all".to_string(),
                        target_type: "job".to_string(),
                        target_id: "all".to_string(),
                        before_json: None,
                        after_json: None,
                        reason: Some("FORBIDDEN_MEMBER".to_string()),
                        correlation_id: Some(correlation_id.to_string()),
                    },
                )
                .await?;
            return Err(TenancyError::Forbidden);
        }
        let mut tx = begin_actor_tx(&self.admin_pool, actor).await?;
        let rows = sqlx::query_as::<_, JobRow>(
            "SELECT id, owner_user_id, job_type, status, created_at \
             FROM jobs ORDER BY created_at",
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        self.audit
            .record(
                actor,
                &AuditEntry {
                    action: "admin.jobs.list_all".to_string(),
                    target_type: "job".to_string(),
                    target_id: "all".to_string(),
                    before_json: None,
                    after_json: None,
                    reason: None,
                    correlation_id: Some(correlation_id.to_string()),
                },
            )
            .await?;
        Ok(rows)
    }
}
