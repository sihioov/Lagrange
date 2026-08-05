//! Append-only auth audit trail (in-memory, typed).
//!
//! Every login denial/allowance, invite lifecycle step, session revocation,
//! CSRF rejection, and step-up verdict is recorded here. The in-memory
//! implementation is the Todo 22 store; the append-only PostgreSQL `audit_logs`
//! table lands with Todo 23 (tenancy/RLS) and the admin surface (Todo 27) -
//! the trait is the contract, so the swap is drop-in.

use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuthAuditKind {
    LoginSucceeded,
    LoginDenied,
    InviteCreated,
    InviteRedeemed,
    InviteDenied,
    SessionExpired,
    SessionRevoked,
    CsrfDenied,
    StepUpDenied,
    StepUpAllowed,
}

impl AuthAuditKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LoginSucceeded => "login_succeeded",
            Self::LoginDenied => "login_denied",
            Self::InviteCreated => "invite_created",
            Self::InviteRedeemed => "invite_redeemed",
            Self::InviteDenied => "invite_denied",
            Self::SessionExpired => "session_expired",
            Self::SessionRevoked => "session_revoked",
            Self::CsrfDenied => "csrf_denied",
            Self::StepUpDenied => "step_up_denied",
            Self::StepUpAllowed => "step_up_allowed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthAuditEvent {
    pub at_secs: i64,
    pub kind: AuthAuditKind,
    pub user: Option<String>,
    pub reason: Option<String>,
    pub detail: String,
}

pub trait AuthAudit: Send + Sync {
    fn record(&self, event: AuthAuditEvent);
}

#[derive(Debug, Default)]
pub struct InMemoryAuthAudit {
    events: Mutex<Vec<AuthAuditEvent>>,
}

impl AuthAudit for InMemoryAuthAudit {
    fn record(&self, event: AuthAuditEvent) {
        self.events.lock().unwrap().push(event);
    }
}

impl InMemoryAuthAudit {
    pub fn events(&self) -> Vec<AuthAuditEvent> {
        self.events.lock().unwrap().clone()
    }

    pub fn has(&self, kind: AuthAuditKind, reason: Option<&str>) -> bool {
        self.events()
            .iter()
            .any(|e| e.kind == kind && e.reason.as_deref() == reason)
    }

    pub fn count(&self, kind: AuthAuditKind) -> usize {
        self.events().iter().filter(|e| e.kind == kind).count()
    }
}

/// Discard sink for production until the append-only store lands (Todo 23).
#[derive(Debug, Default)]
pub struct NoopAudit;

impl AuthAudit for NoopAudit {
    fn record(&self, _event: AuthAuditEvent) {}
}
