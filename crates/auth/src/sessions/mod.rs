//! First-party sessions: opaque cookie, hashed storage, revocation, expiry.
//!
//! **Session persistence seam (Todo 3 is BLOCKED):** `web_sessions` is a
//! Todo-3 migration table that does not exist yet. [`SessionStore`] is the
//! typed async trait contract - opaque cookie value hashed (SHA-256) before
//! storage, lookup/revoke/expiry, ownership binding to the internal user -
//! with the tested in-memory implementation shipping now. The PostgreSQL
//! implementation lands with Todo 3; nothing above the trait changes.
//!
//! Sessions are SHORT (30 min, no sliding renewal, no browser refresh tokens):
//! expiry means re-login at the provider, so `auth_time`/`amr` captured at
//! login stay meaningful for the Owner step-up gate.

pub mod cookie;
use crate::audit::{AuthAudit, AuthAuditEvent, AuthAuditKind};
use crate::clock::Clock;
use crate::entitlement::{Actor, Role, UserId};
use crate::invites::RedeemedIdentity;
use crate::oidc::pkce::random_hex;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSession {
    pub token_hash: String,
    pub user_id: UserId,
    pub role: Role,
    pub auth_time_secs: i64,
    pub amr: Vec<String>,
    pub csrf_token_hash: String,
    pub created_at_secs: i64,
    pub expires_at_secs: i64,
}

/// Typed async contract for the `web_sessions` persistence layer (Todo 3).
/// The token hash is the primary key; raw opaque values never enter the store.
#[async_trait::async_trait]
pub trait SessionStore: Send + Sync {
    async fn insert(&self, session: StoredSession) -> Result<(), SessionError>;
    async fn lookup(&self, token_hash: &str) -> Result<Option<StoredSession>, SessionError>;
    async fn revoke(&self, token_hash: &str) -> Result<(), SessionError>;
    async fn update_csrf(
        &self,
        token_hash: &str,
        csrf_token_hash: &str,
    ) -> Result<(), SessionError>;
}

/// Todo 22 in-memory `SessionStore` (tested; PostgreSQL impl lands with Todo 3).
#[derive(Debug, Default)]
pub struct InMemorySessionStore {
    inner: RwLock<HashMap<String, StoredSession>>,
}

#[async_trait::async_trait]
impl SessionStore for InMemorySessionStore {
    async fn insert(&self, session: StoredSession) -> Result<(), SessionError> {
        let mut map = self
            .inner
            .write()
            .map_err(|_| SessionError::Store("lock".into()))?;
        map.insert(session.token_hash.clone(), session);
        Ok(())
    }

    async fn lookup(&self, token_hash: &str) -> Result<Option<StoredSession>, SessionError> {
        let map = self
            .inner
            .read()
            .map_err(|_| SessionError::Store("lock".into()))?;
        Ok(map.get(token_hash).cloned())
    }

    async fn revoke(&self, token_hash: &str) -> Result<(), SessionError> {
        let mut map = self
            .inner
            .write()
            .map_err(|_| SessionError::Store("lock".into()))?;
        map.remove(token_hash);
        Ok(())
    }

    async fn update_csrf(
        &self,
        token_hash: &str,
        csrf_token_hash: &str,
    ) -> Result<(), SessionError> {
        let mut map = self
            .inner
            .write()
            .map_err(|_| SessionError::Store("lock".into()))?;
        let session = map
            .get_mut(token_hash)
            .ok_or(SessionError::UnknownSession)?;
        session.csrf_token_hash = csrf_token_hash.to_string();
        Ok(())
    }
}

/// Authenticated view of a validated session (no provider tokens, ever).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    pub user_id: UserId,
    pub role: Role,
    pub auth_time_secs: i64,
    pub amr: Vec<String>,
    pub expires_at_secs: i64,
    pub csrf_token_hash: String,
}

impl SessionInfo {
    pub fn actor(&self) -> Actor {
        Actor::new(self.user_id.0.clone(), self.role)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedSession {
    pub cookie_value: String,
    pub set_cookie_header: String,
    pub csrf_token: String,
    pub session: SessionInfo,
}

pub struct SessionService {
    pub store: Arc<dyn SessionStore>,
    pub clock: Arc<dyn Clock>,
    pub audit: Arc<dyn AuthAudit>,
}

impl SessionService {
    pub fn new(
        store: Arc<dyn SessionStore>,
        clock: Arc<dyn Clock>,
        audit: Arc<dyn AuthAudit>,
    ) -> Self {
        Self {
            store,
            clock,
            audit,
        }
    }

    /// Mints a fresh opaque session: new random cookie value, hashed at rest,
    /// per-session CSRF token, short TTL. Session fixation is impossible by
    /// construction - every login gets a brand-new value.
    pub async fn issue(
        &self,
        identity: &RedeemedIdentity,
        auth_time_secs: i64,
        amr: Vec<String>,
    ) -> Result<IssuedSession, SessionError> {
        let now = self.clock.now_epoch_secs();
        let cookie_value = cookie::generate_value();
        let token_hash = cookie::hash(&cookie_value);
        let csrf_token = random_hex();
        let csrf_token_hash = cookie::hash(&csrf_token);
        let stored_amr = amr;
        let expires_at_secs = now + cookie::TTL_SECS;
        let stored = StoredSession {
            token_hash: token_hash.clone(),
            user_id: identity.user_id.clone(),
            role: identity.role,
            auth_time_secs,
            amr: stored_amr.clone(),
            csrf_token_hash: csrf_token_hash.clone(),
            created_at_secs: now,
            expires_at_secs,
        };
        self.store.insert(stored).await?;
        let session = SessionInfo {
            user_id: identity.user_id.clone(),
            role: identity.role,
            auth_time_secs,
            amr: stored_amr,
            expires_at_secs,
            csrf_token_hash,
        };
        Ok(IssuedSession {
            cookie_value: cookie_value.clone(),
            set_cookie_header: cookie::set_cookie(&cookie_value, expires_at_secs),
            csrf_token,
            session,
        })
    }

    /// Validates a presented cookie value: hash -> lookup -> expiry. Denials
    /// are typed (unknown vs expired) and audited.
    pub async fn validate(&self, cookie_value: &str) -> Result<SessionInfo, SessionError> {
        let token_hash = cookie::hash(cookie_value);
        let stored = self
            .store
            .lookup(&token_hash)
            .await?
            .ok_or(SessionError::UnknownSession)?;
        let now = self.clock.now_epoch_secs();
        if now >= stored.expires_at_secs {
            self.audit.record(AuthAuditEvent {
                at_secs: now,
                kind: AuthAuditKind::SessionExpired,
                user: Some(stored.user_id.0.clone()),
                reason: Some("SESSION_EXPIRED".to_string()),
                detail: format!("expired at {}", stored.expires_at_secs),
            });
            let _ = self.store.revoke(&token_hash).await;
            return Err(SessionError::Expired);
        }
        Ok(SessionInfo {
            user_id: stored.user_id,
            role: stored.role,
            auth_time_secs: stored.auth_time_secs,
            amr: stored.amr,
            expires_at_secs: stored.expires_at_secs,
            csrf_token_hash: stored.csrf_token_hash,
        })
    }

    pub async fn revoke(&self, cookie_value: &str) -> Result<(), SessionError> {
        let token_hash = cookie::hash(cookie_value);
        let now = self.clock.now_epoch_secs();
        if let Ok(Some(stored)) = self.store.lookup(&token_hash).await {
            self.audit.record(AuthAuditEvent {
                at_secs: now,
                kind: AuthAuditKind::SessionRevoked,
                user: Some(stored.user_id.0.clone()),
                reason: None,
                detail: "logout".to_string(),
            });
        }
        self.store.revoke(&token_hash).await
    }

    /// Rotates the per-session CSRF synchronizer token; returns the plaintext.
    pub async fn rotate_csrf(&self, cookie_value: &str) -> Result<String, SessionError> {
        let token_hash = cookie::hash(cookie_value);
        let new_token = random_hex();
        let new_hash = cookie::hash(&new_token);
        self.store.update_csrf(&token_hash, &new_hash).await?;
        Ok(new_token)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("no session for this cookie")]
    UnknownSession,
    #[error("session expired - re-login required")]
    Expired,
    #[error("session store failure: {0}")]
    Store(String),
    #[error("session revoked")]
    Revoked,
}

impl SessionError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnknownSession => "SESSION_UNKNOWN",
            Self::Expired => "SESSION_EXPIRED",
            Self::Store(_) => "SESSION_STORE",
            Self::Revoked => "SESSION_REVOKED",
        }
    }
}
