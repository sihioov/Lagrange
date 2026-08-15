//! Invite-only onboarding (FR-AUTH-001) with immutable `(iss, sub)` binding.
//!
//! An invite is single-use, addressed to a NORMALIZED email, and can only be
//! redeemed by a verified email at the provider. Identity is then keyed by the
//! immutable `(issuer, subject)` pair: an email-profile change at the provider
//! keeps the same internal user, and email alone never grants or changes
//! access. The initial role comes from the ID-token `roles` claim (member /
//! owner), falling back to the invite's role; unknown claim values deny
//! fail-closed. The invite/user stores are the same Todo-3 seam as
//! `web_sessions`: in-memory now, PostgreSQL with RLS later.

use crate::audit::{AuthAudit, AuthAuditEvent, AuthAuditKind};
use crate::clock::Clock;
use crate::entitlement::{Role, UserId};
use crate::oidc::claims::IdTokenClaims;
use crate::oidc::pkce::random_hex;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

pub const DEFAULT_INVITE_TTL_SECS: i64 = 7 * 24 * 3600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteRecord {
    pub id: String,
    pub email: String,
    pub role: Role,
    pub created_at_secs: i64,
    pub expires_at_secs: i64,
    pub redeemed_by: Option<(String, String)>,
    pub redeemed_at_secs: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserRecord {
    pub binding_issuer: String,
    pub binding_subject: String,
    pub user_id: UserId,
    pub role: Role,
    pub email: String,
    pub created_at_secs: i64,
}

#[async_trait::async_trait]
pub trait InviteStore: Send + Sync {
    async fn insert(&self, invite: InviteRecord) -> Result<(), InviteError>;
    async fn find_by_email(&self, email: &str) -> Result<Option<InviteRecord>, InviteError>;
    /// Atomic single-use claim: succeeds only when still unredeemed.
    async fn claim(
        &self,
        id: &str,
        issuer: &str,
        subject: &str,
        at_secs: i64,
    ) -> Result<bool, InviteError>;
}

#[async_trait::async_trait]
pub trait UserStore: Send + Sync {
    async fn find_by_binding(
        &self,
        issuer: &str,
        subject: &str,
    ) -> Result<Option<UserRecord>, InviteError>;
    /// Persist a first-time identity and return the store's canonical ID.
    ///
    /// In-memory implementations may preserve the caller's provisional ID,
    /// while a durable store can allocate a database-native UUID. Callers
    /// must use the returned value for sessions and audit records; the ID on
    /// `user` is only a simulator/provisional candidate.
    async fn insert_user(&self, user: UserRecord) -> Result<UserId, InviteError>;
    async fn update_profile(&self, user_id: &str, email: &str) -> Result<(), InviteError>;
}

/// Todo 22 in-memory stores (tested; PostgreSQL impls land with Todo 3/23).
#[derive(Debug, Default)]
pub struct InMemoryInviteStore {
    inner: RwLock<HashMap<String, InviteRecord>>,
}

#[async_trait::async_trait]
impl InviteStore for InMemoryInviteStore {
    async fn insert(&self, invite: InviteRecord) -> Result<(), InviteError> {
        let mut map = self
            .inner
            .write()
            .map_err(|_| InviteError::Store("lock".into()))?;
        map.insert(invite.id.clone(), invite);
        Ok(())
    }

    async fn find_by_email(&self, email: &str) -> Result<Option<InviteRecord>, InviteError> {
        let map = self
            .inner
            .read()
            .map_err(|_| InviteError::Store("lock".into()))?;
        Ok(map.values().find(|i| i.email == email).cloned())
    }

    async fn claim(
        &self,
        id: &str,
        issuer: &str,
        subject: &str,
        at_secs: i64,
    ) -> Result<bool, InviteError> {
        let mut map = self
            .inner
            .write()
            .map_err(|_| InviteError::Store("lock".into()))?;
        let invite = map.get_mut(id).ok_or(InviteError::InviteNotFound)?;
        if invite.redeemed_by.is_some() {
            return Ok(false);
        }
        invite.redeemed_by = Some((issuer.to_string(), subject.to_string()));
        invite.redeemed_at_secs = Some(at_secs);
        Ok(true)
    }
}

#[derive(Debug, Default)]
pub struct InMemoryUserStore {
    inner: RwLock<Vec<UserRecord>>,
}

#[async_trait::async_trait]
impl UserStore for InMemoryUserStore {
    async fn find_by_binding(
        &self,
        issuer: &str,
        subject: &str,
    ) -> Result<Option<UserRecord>, InviteError> {
        let map = self
            .inner
            .read()
            .map_err(|_| InviteError::Store("lock".into()))?;
        Ok(map
            .iter()
            .find(|u| u.binding_issuer == issuer && u.binding_subject == subject)
            .cloned())
    }

    async fn insert_user(&self, user: UserRecord) -> Result<UserId, InviteError> {
        let mut map = self
            .inner
            .write()
            .map_err(|_| InviteError::Store("lock".into()))?;
        let user_id = user.user_id.clone();
        map.push(user);
        Ok(user_id)
    }

    async fn update_profile(&self, user_id: &str, email: &str) -> Result<(), InviteError> {
        let mut map = self
            .inner
            .write()
            .map_err(|_| InviteError::Store("lock".into()))?;
        if let Some(user) = map.iter_mut().find(|u| u.user_id.0 == user_id) {
            user.email = email.to_string();
        }
        Ok(())
    }
}

pub struct InviteService {
    pub invites: Arc<dyn InviteStore>,
    pub users: Arc<dyn UserStore>,
    pub clock: Arc<dyn Clock>,
    pub audit: Arc<dyn AuthAudit>,
}

impl InviteService {
    pub fn new(
        invites: Arc<dyn InviteStore>,
        users: Arc<dyn UserStore>,
        clock: Arc<dyn Clock>,
        audit: Arc<dyn AuthAudit>,
    ) -> Self {
        Self {
            invites,
            users,
            clock,
            audit,
        }
    }

    pub fn normalize_email(raw: &str) -> Result<String, InviteError> {
        let normalized = raw.trim().to_lowercase();
        if !email_address::EmailAddress::is_valid(&normalized) {
            return Err(InviteError::InvalidEmail(normalized));
        }
        Ok(normalized)
    }

    pub async fn create_invite(
        &self,
        email: &str,
        role: Role,
        ttl_secs: i64,
    ) -> Result<InviteRecord, InviteError> {
        self.create_invite_as(email, role, ttl_secs, None).await
    }

    /// Create an invitation on behalf of an authenticated actor.
    ///
    /// The legacy convenience method above remains useful for the simulator,
    /// but the HTTP Owner route must call this method so the audit row names
    /// the authenticated Owner rather than silently recording an unattributed
    /// mutation.
    pub async fn create_invite_as(
        &self,
        email: &str,
        role: Role,
        ttl_secs: i64,
        actor: Option<&UserId>,
    ) -> Result<InviteRecord, InviteError> {
        let email = Self::normalize_email(email)?;
        let now = self.clock.now_epoch_secs();
        let invite = InviteRecord {
            id: format!("inv-{}", random_hex()),
            email,
            role,
            created_at_secs: now,
            expires_at_secs: now + ttl_secs,
            redeemed_by: None,
            redeemed_at_secs: None,
        };
        self.invites.insert(invite.clone()).await?;
        self.audit
            .record(AuthAuditEvent {
                at_secs: now,
                kind: AuthAuditKind::InviteCreated,
                user: actor.map(|user| user.0.clone()),
                reason: None,
                detail: format!(
                    "invite {} for {} as {}",
                    invite.id,
                    invite.email,
                    role_name(invite.role)
                ),
            })
            .map_err(|error| InviteError::Audit(format!("{error:?}")))?;
        Ok(invite)
    }

    /// Resolves an authenticated subject to an internal identity.
    ///
    /// 1. Existing `(iss, sub)` binding -> same user (email profile may change).
    /// 2. Otherwise: `email_verified` required, single-use invite on the
    ///    normalized email, role = claims `roles` (owner/member) else invite
    ///    role else deny.
    pub async fn resolve_identity(
        &self,
        claims: &IdTokenClaims,
    ) -> Result<RedeemedIdentity, InviteError> {
        let now = self.clock.now_epoch_secs();
        let issuer = claims.iss.clone();
        let subject = claims.sub.clone();
        let binding_key = format!("{issuer}|{subject}");

        if let Some(user) = self.users.find_by_binding(&issuer, &subject).await? {
            if let Some(email) = &claims.email
                && let Ok(email) = Self::normalize_email(email)
            {
                let _ = self.users.update_profile(&user.user_id.0, &email).await;
            }
            let user = self
                .users
                .find_by_binding(&issuer, &subject)
                .await?
                .expect("user still bound after profile update");
            return Ok(RedeemedIdentity {
                user_id: user.user_id,
                role: user.role,
                email: user.email,
                binding: binding_key,
            });
        }

        let email = claims
            .email
            .as_deref()
            .map(Self::normalize_email)
            .transpose()?
            .ok_or(InviteError::EmailRequired)?;
        if !claims.is_email_verified() {
            return Err(InviteError::EmailNotVerified);
        }
        let invite = self
            .invites
            .find_by_email(&email)
            .await?
            .ok_or(InviteError::InviteNotFound)?;
        if now > invite.expires_at_secs {
            return Err(InviteError::InviteExpired);
        }
        if !self
            .invites
            .claim(&invite.id, &issuer, &subject, now)
            .await?
        {
            return Err(InviteError::AlreadyRedeemed);
        }
        // Role resolution is fail-closed: explicit claims that name no known
        // role deny; only a SILENT roles claim falls back to the invite role.
        let role = if claims.roles.is_empty() {
            invite.role
        } else {
            claims.mapped_role().ok_or(InviteError::RoleUnknown)?
        };
        // The in-memory store keeps this simulator ID, while a durable store
        // may allocate a database-native UUID. The insertion result below is
        // the only ID that may cross into sessions or audit records.
        let provisional_user_id = UserId::new(format!("usr_{}", random_hex()));
        let user = UserRecord {
            binding_issuer: issuer,
            binding_subject: subject,
            user_id: provisional_user_id,
            role,
            email: email.clone(),
            created_at_secs: now,
        };
        let user_id = self.users.insert_user(user).await?;
        self.audit
            .record(AuthAuditEvent {
                at_secs: now,
                kind: AuthAuditKind::InviteRedeemed,
                user: Some(user_id.0.clone()),
                reason: None,
                detail: format!("invite {} redeemed as {}", invite.id, role_name(role)),
            })
            .map_err(|error| InviteError::Audit(format!("{error:?}")))?;
        Ok(RedeemedIdentity {
            user_id,
            role,
            email,
            binding: binding_key,
        })
    }
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::Owner => "owner",
        Role::Member => "member",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedeemedIdentity {
    pub user_id: UserId,
    pub role: Role,
    pub email: String,
    /// Immutable `(issuer, subject)` binding key (`iss|sub`).
    pub binding: String,
}

#[derive(Debug, thiserror::Error)]
pub enum InviteError {
    #[error("invalid email address")]
    InvalidEmail(String),
    #[error("email claim is required")]
    EmailRequired,
    #[error("email is not verified at the provider")]
    EmailNotVerified,
    #[error("no invite for this email")]
    InviteNotFound,
    #[error("invite expired")]
    InviteExpired,
    #[error("invite already redeemed")]
    AlreadyRedeemed,
    #[error("no known role for this identity")]
    RoleUnknown,
    #[error("invite/user store failure: {0}")]
    Store(String),
    #[error("auth audit delivery failure: {0}")]
    Audit(String),
}

impl InviteError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidEmail(_) => "INVITE_INVALID_EMAIL",
            Self::EmailRequired => "INVITE_EMAIL_REQUIRED",
            Self::EmailNotVerified => "INVITE_EMAIL_NOT_VERIFIED",
            Self::InviteNotFound => "INVITE_NOT_FOUND",
            Self::InviteExpired => "INVITE_EXPIRED",
            Self::AlreadyRedeemed => "INVITE_ALREADY_REDEEMED",
            Self::RoleUnknown => "INVITE_ROLE_UNKNOWN",
            Self::Store(_) => "INVITE_STORE",
            Self::Audit(_) => "INVITE_AUDIT",
        }
    }
}
