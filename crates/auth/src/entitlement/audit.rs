//! Authorization audit events.
//!
//! In production these records are written to the append-only `audit_logs` table
//! (Todo 3/23). This module defines the event shape and an in-memory log so the
//! gate's deny paths produce auditable evidence today.

use crate::entitlement::date::CalendarDate;
use crate::entitlement::error::DenialCode;
use crate::entitlement::identity::{DatasetId, EntitlementId, UserId};
use crate::entitlement::service::{AccessRequest, Grant};
use crate::entitlement::use_registry::KrUse;

/// Whether an authorization outcome was allowed or denied (with its stable code).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditDecision {
    Allowed,
    Denied(DenialCode),
}

/// One authorization decision, captured for the append-only audit trail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEvent {
    pub occurred_on: CalendarDate,
    pub actor: UserId,
    pub dataset: DatasetId,
    pub use_kind: KrUse,
    pub decision: AuditDecision,
    pub entitlement_id: Option<EntitlementId>,
}

/// Append-only in-memory audit log (production replaces this with the DB sink).
#[derive(Debug, Clone, Default)]
pub struct AuditLog {
    events: Vec<AuditEvent>,
}

impl AuditLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, event: AuditEvent) {
        self.events.push(event);
    }

    pub fn events(&self) -> &[AuditEvent] {
        &self.events
    }
}

/// Build the audit event for an authorization outcome.
pub fn audit_event_for(
    req: &AccessRequest,
    use_kind: KrUse,
    outcome: &Result<Grant, crate::entitlement::error::EntitlementDenied>,
) -> AuditEvent {
    let (decision, entitlement_id) = match outcome {
        Ok(grant) => (AuditDecision::Allowed, Some(grant.entitlement_id.clone())),
        Err(denied) => (AuditDecision::Denied(denied.code), None),
    };
    AuditEvent {
        occurred_on: req.as_of,
        actor: req.actor.user_id.clone(),
        dataset: req.dataset.clone(),
        use_kind,
        decision,
        entitlement_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entitlement::identity::Actor;
    use crate::entitlement::service::AccessRequest;
    use crate::entitlement::{CalendarDate, DatasetId};

    #[test]
    fn audit_log_is_append_only() {
        let mut log = AuditLog::new();
        let req = AccessRequest {
            actor: Actor::member("usr_a"),
            dataset: DatasetId::krx_eod_bars(),
            as_of: CalendarDate::parse("2026-06-15").unwrap(),
        };
        let denied = Err(crate::entitlement::error::EntitlementDenied {
            code: DenialCode::DataEntitlementRequired,
            dataset: req.dataset.clone(),
            use_kind: KrUse::Recommendation,
            state: None,
            reason: crate::entitlement::error::DenialReason::NoEntitlementRecord,
        });
        log.record(audit_event_for(&req, KrUse::Recommendation, &denied));
        assert_eq!(log.events().len(), 1);
        assert_eq!(
            log.events()[0].decision,
            AuditDecision::Denied(DenialCode::DataEntitlementRequired)
        );
        assert_eq!(log.events()[0].actor, UserId::new("usr_a"));
    }
}
