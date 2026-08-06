//! Append-only audit writer: INSERTs into `audit_logs` through the
//! `audit_writer` role. Serving roles hold no UPDATE/DELETE/TRUNCATE grant and
//! RLS exposes no mutation policy, so rows are immutable by privilege
//! (NFR-SEC-007). Captures actor / time / target / before-after / reason /
//! correlation id per FR-ADM-002.

use crate::error::{TenancyError, TenancyResult};
use auth::entitlement::Actor;
use serde_json::Value;
use uuid::Uuid;

/// One audit event to record (before/after optional; reason or correlation id
/// always present).
#[derive(Debug, Clone)]
pub struct AuditEntry {
    pub action: String,
    pub target_type: String,
    pub target_id: String,
    pub before_json: Option<Value>,
    pub after_json: Option<Value>,
    pub reason: Option<String>,
    pub correlation_id: Option<String>,
}

/// Typed append-only writer over `audit_logs` (audit_writer role pool).
#[derive(Debug, Clone)]
pub struct AuditWriter {
    pool: sqlx::PgPool,
}

impl AuditWriter {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    /// Record an event. `actor_role` is derived from the actor ("owner" /
    /// "member"); `actor_user_id` is bound when the actor id parses as a uuid.
    pub async fn record(&self, actor: &Actor, entry: &AuditEntry) -> TenancyResult<()> {
        let role = if actor.is_owner() { "owner" } else { "member" };
        let user_id = uuid_of(actor);
        sqlx::query(
            "INSERT INTO audit_logs \
             (action, actor_role, actor_user_id, target_type, target_id, \
              before_json, after_json, reason, correlation_id) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(&entry.action)
        .bind(role)
        .bind(user_id)
        .bind(&entry.target_type)
        .bind(&entry.target_id)
        .bind(&entry.before_json)
        .bind(&entry.after_json)
        .bind(&entry.reason)
        .bind(&entry.correlation_id)
        .execute(&self.pool)
        .await
        .map_err(TenancyError::from_sqlx)?;
        Ok(())
    }
}

fn uuid_of(actor: &Actor) -> Option<Uuid> {
    Uuid::parse_str(&actor.user_id.0).ok()
}
