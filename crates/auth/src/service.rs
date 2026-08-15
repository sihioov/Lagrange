//! Orchestration seam: the router-facing login/logout/session/CSRF API.
//!
//! Composes the protocol core ([`oidc::OidcClient`]), onboarding
//! ([`invites::InviteService`]) and sessions ([`sessions::SessionService`])
//! into the single authority that `apps/api-server/auth` mounts. Every
//! decision is audited; provider tokens never appear on any return type.

use crate::audit::{AuthAudit, AuthAuditEvent, AuthAuditKind};
use crate::invites::{InviteError, InviteService};
use crate::oidc::{AuthorizeRequest, OidcClient, OidcError, PendingAuthStore};
use crate::sessions::{IssuedSession, SessionError, SessionInfo, SessionService};
use std::sync::Arc;

pub struct AuthService {
    pub oidc: OidcClient,
    pub invites: InviteService,
    pub sessions: SessionService,
    pub audit: Arc<dyn AuthAudit>,
}

impl AuthService {
    pub fn new(
        oidc: OidcClient,
        invites: InviteService,
        sessions: SessionService,
        audit: Arc<dyn AuthAudit>,
    ) -> Self {
        Self {
            oidc,
            invites,
            sessions,
            audit,
        }
    }

    /// Step 1 of the login flow: authorize URL + the pending record to store
    /// server-side (single-use) before redirecting the browser.
    pub fn begin_login(&self) -> Result<AuthorizeRequest, AuthError> {
        self.oidc.begin_authorize().map_err(AuthError::Oidc)
    }

    /// Step 2: callback validation -> identity resolution -> session issue.
    /// The pending record is consumed from the store (single-use `state`); the
    /// caller only needs the opaque `state` it stored at `begin_login`.
    pub async fn complete_login(
        &self,
        code: &str,
        state: &str,
        pending_store: &dyn PendingAuthStore,
    ) -> Result<IssuedSession, AuthError> {
        let now = self.sessions.clock.now_epoch_secs();
        let consumed = match pending_store.take(state).await {
            Ok(Some(p)) => p,
            Ok(None) => {
                self.audit_login_denied(None, "PENDING_MISSING", state)
                    .await?;
                return Err(AuthError::Oidc(OidcError::StateMismatch));
            }
            Err(e) => return Err(AuthError::Oidc(e)),
        };
        let claims = match self
            .oidc
            .validate_callback(code, &consumed.state, &consumed, now)
            .await
        {
            Ok(c) => c,
            Err(e) => {
                self.audit_login_denied(None, e.code(), "callback validation")
                    .await?;
                return Err(AuthError::Oidc(e));
            }
        };
        let identity = match self.invites.resolve_identity(&claims).await {
            Ok(i) => i,
            Err(e) => {
                self.audit_login_denied(
                    Some(&claims.sub),
                    e.code(),
                    &claims.email.clone().unwrap_or_default(),
                )
                .await?;
                return Err(AuthError::Invite(e));
            }
        };
        let auth_time = claims.auth_time.unwrap_or(now);
        let issued = self
            .sessions
            .issue(&identity, auth_time, claims.amr.clone())
            .await
            .map_err(AuthError::Session)?;
        self.audit
            .record(AuthAuditEvent {
                at_secs: now,
                kind: AuthAuditKind::LoginSucceeded,
                user: Some(identity.user_id.0.clone()),
                reason: None,
                detail: format!("identity {}", identity.binding),
            })
            .map_err(|error| AuthError::Audit(format!("{error:?}")))?;
        Ok(issued)
    }

    pub async fn logout(&self, cookie_value: &str) -> Result<(), AuthError> {
        self.sessions
            .revoke(cookie_value)
            .await
            .map_err(AuthError::Session)
    }

    pub async fn session_info(&self, cookie_value: &str) -> Result<SessionInfo, AuthError> {
        match self.sessions.validate(cookie_value).await {
            Ok(info) => Ok(info),
            Err(SessionError::Expired) => {
                self.audit
                    .record_durable(AuthAuditEvent {
                        at_secs: self.sessions.clock.now_epoch_secs(),
                        kind: AuthAuditKind::SessionExpired,
                        user: None,
                        reason: Some("SESSION_EXPIRED".to_string()),
                        detail: "session expired, re-login required".to_string(),
                    })
                    .await
                    .map_err(|error| AuthError::Audit(format!("{error:?}")))?;
                Err(AuthError::Session(SessionError::Expired))
            }
            Err(e) => Err(AuthError::Session(e)),
        }
    }

    pub async fn rotate_csrf(&self, cookie_value: &str) -> Result<String, AuthError> {
        self.sessions
            .rotate_csrf(cookie_value)
            .await
            .map_err(AuthError::Session)
    }

    async fn audit_login_denied(
        &self,
        sub: Option<&str>,
        reason: &str,
        detail: &str,
    ) -> Result<(), AuthError> {
        self.audit
            .record_durable(AuthAuditEvent {
                at_secs: self.sessions.clock.now_epoch_secs(),
                kind: AuthAuditKind::LoginDenied,
                user: sub.map(str::to_string),
                reason: Some(reason.to_string()),
                detail: detail.to_string(),
            })
            .await
            .map_err(|error| AuthError::Audit(format!("{error:?}")))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error(transparent)]
    Oidc(#[from] OidcError),
    #[error(transparent)]
    Invite(#[from] InviteError),
    #[error(transparent)]
    Session(#[from] SessionError),
    #[error("auth audit delivery failure: {0}")]
    Audit(String),
}

impl AuthError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Oidc(e) => e.code(),
            Self::Invite(e) => e.code(),
            Self::Session(e) => e.code(),
            Self::Audit(_) => "AUTH_AUDIT",
        }
    }

    pub fn is_unauthenticated(&self) -> bool {
        matches!(
            self,
            Self::Session(SessionError::UnknownSession | SessionError::Expired)
        )
    }

    pub fn is_invite_denial(&self) -> bool {
        matches!(self, Self::Invite(_))
    }
}
