//! The PostgreSQL-backed `RiskEventStore` (plan Todo 38).
//!
//! `risk-gateway` owns the decision; this owns the row. The split matters:
//! the gate mints an approval only after `record` returns `Ok`, so this
//! implementation's contract is the whole durability half of the guarantee.
//! It must therefore COMMIT before returning — a buffered or fire-and-forget
//! write would let an approval outlive the evidence for it, which is exactly
//! what §16 blocks Live orders to prevent.
//!
//! `risk_events` is a 0007 TENANT table with FORCE RLS on the actor GUC, so
//! every write runs inside an actor transaction. Todo 37 learned this the
//! painful way: a repository using a bare pool has its inserts silently
//! refused by the policy rather than failing loudly at review time.

use crate::actor_tx::begin_actor_tx;
use auth::entitlement::Actor;
use risk_gateway::{Decision, RiskEventStore, RiskSnapshot, StoreError};
use sqlx::PgPool;
use uuid::Uuid;

/// The `event_type` under which gate decisions are filed.
///
/// Migration 0018's CHECK and partial unique index both key off this exact
/// string: it is what separates a gate decision (which must carry the full
/// column set and is unique per intent) from any other risk event.
pub const GATE_EVENT_TYPE: &str = "LIVE_ORDER_GATE";

/// Persists Risk Gateway decisions for one actor.
pub struct RiskRepo {
    pool: PgPool,
    actor: Actor,
    owner_user_id: Uuid,
    account_id: Option<Uuid>,
}

impl RiskRepo {
    pub fn new(pool: PgPool, actor: Actor, owner_user_id: Uuid, account_id: Option<Uuid>) -> Self {
        Self {
            pool,
            actor,
            owner_user_id,
            account_id,
        }
    }
}

impl RiskEventStore for RiskRepo {
    async fn record(
        &self,
        decision: &Decision,
        snapshot: &RiskSnapshot,
    ) -> Result<String, StoreError> {
        // The payload carries BOTH the ordered per-check trail and the inputs
        // it was derived from. The snapshot is what makes a decision
        // re-derivable after limits change or the process restarts; without
        // it the row records a verdict nobody can check.
        let payload = serde_json::json!({
            "checks": decision.records,
            "snapshot": snapshot,
        });

        let mut tx = begin_actor_tx(&self.pool, &self.actor)
            .await
            .map_err(|e| StoreError::new(format!("actor transaction failed: {e:?}")))?;

        let id: Uuid = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO risk_events \
             (owner_user_id, account_id, event_type, severity, payload_json, created_by, \
              intent_ref, correlation_id, limits_version, decision, denied_by_check, \
              reason_code, evaluated_at) \
             VALUES ($1, $2, $3, $4, $5, 'risk-gateway', $6, $7, $8, $9, $10, $11, \
                     to_timestamp($12)) \
             RETURNING id",
        )
        .bind(self.owner_user_id)
        .bind(self.account_id)
        .bind(GATE_EVENT_TYPE)
        .bind(decision.severity())
        .bind(&payload)
        .bind(&decision.intent_ref)
        .bind(&decision.correlation_id)
        .bind(&decision.limits_version)
        .bind(if decision.is_approved() {
            "APPROVED"
        } else {
            "DENIED"
        })
        .bind(decision.denied_by.map(|c| c.as_str()))
        .bind(decision.reason.map_or("APPROVED", |r| r.as_str()))
        .bind(decision.evaluated_at_secs as f64)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| {
            // A unique violation is not an infrastructure fault: it means this
            // intent already has a decision. It still denies, because the
            // caller must not proceed on a decision it did not just record,
            // but the message distinguishes the two for whoever reads it.
            if matches!(&e, sqlx::Error::Database(db) if db.code().as_deref() == Some("23505")) {
                StoreError::new(format!(
                    "intent {} already has a gate decision",
                    decision.intent_ref
                ))
            } else {
                StoreError::new(format!("insert failed: {e}"))
            }
        })?;

        // Commit BEFORE returning. The approval the gate is about to mint is
        // only sound if this row is durable.
        tx.commit()
            .await
            .map_err(|e| StoreError::new(format!("commit failed: {e}")))?;

        Ok(id.to_string())
    }
}
