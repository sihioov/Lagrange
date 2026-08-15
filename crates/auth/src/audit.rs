//! Append-only auth audit trail (in-memory, typed).
//!
//! Every login denial/allowance, invite lifecycle step, session revocation,
//! CSRF rejection, and step-up verdict is recorded here. The in-memory
//! implementation remains the simulator/test store; production PostgreSQL
//! adapters enqueue through the transactional audit outbox before a worker
//! copies committed rows to append-only `audit_logs` under RLS.

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthAuditError {
    Saturated,
    Closed,
    Unavailable,
}

#[async_trait::async_trait]
pub trait AuthAudit: Send + Sync {
    /// Enqueue an event without blocking the caller. Production sinks must
    /// return an explicit error when durable delivery cannot be admitted.
    fn record(&self, event: AuthAuditEvent) -> Result<(), AuthAuditError>;

    /// Enqueue before the protected operation succeeds. In-memory sinks do
    /// this synchronously; the production sink writes the transactional
    /// PostgreSQL outbox and returns only after commit.
    async fn record_durable(&self, event: AuthAuditEvent) -> Result<(), AuthAuditError> {
        self.record(event)
    }
}

#[derive(Debug, Default)]
pub struct InMemoryAuthAudit {
    events: Mutex<Vec<AuthAuditEvent>>,
}

#[async_trait::async_trait]
impl AuthAudit for InMemoryAuthAudit {
    fn record(&self, event: AuthAuditEvent) -> Result<(), AuthAuditError> {
        self.events
            .lock()
            .map_err(|_| AuthAuditError::Unavailable)?
            .push(event);
        Ok(())
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

/// Explicit no-op sink for simulator/tests. Production uses the durable
/// bounded Postgres sink; callers retain the same admission result contract.
#[derive(Debug, Default)]
pub struct NoopAudit;

#[async_trait::async_trait]
impl AuthAudit for NoopAudit {
    fn record(&self, _event: AuthAuditEvent) -> Result<(), AuthAuditError> {
        Ok(())
    }
}
