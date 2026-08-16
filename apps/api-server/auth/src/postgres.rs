//! Production persistence adapters for the auth router.
//!
//! The protocol crate deliberately knows nothing about SQLx.  This module is
//! the boundary between its typed store traits and the schema used by the
//! API process.  Session reads use the dedicated read-only `admin` pool so a
//! cookie can be resolved before an actor is known; session writes use the
//! `app` pool with an explicit actor GUC and therefore remain subject to the
//! same RLS policy as the versioned API.

use auth::audit::{AuthAudit, AuthAuditError, AuthAuditEvent};
use auth::entitlement::{Role, UserId};
use auth::invites::{InviteError, InviteRecord, InviteStore, UserRecord, UserStore};
use auth::sessions::{SessionError, SessionStore, StoredSession};
use sqlx::{PgPool, Row};
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const ACTOR_GUC: &str = "app.actor_user_id";
const MAX_CONSECUTIVE_FAILURES: u64 = 3;
const WORKER_STALE_SECS: u64 = 5;
const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(8);
/// Application-side deadline for one auth SQL operation.  The SQL functions
/// use a shorter function-local statement/lock timeout; this outer deadline
/// also covers pool acquisition, network transit, and commit.
pub const AUTH_SQL_OPERATION_DEADLINE: Duration = Duration::from_secs(6);
/// A session mutation remains one transaction from BEGIN through COMMIT.  A
/// timeout drops the SQLx transaction (which queues a rollback) and therefore
/// cannot leave a partially inserted/revoked session acknowledged.
pub const SESSION_TRANSACTION_DEADLINE: Duration = Duration::from_secs(7);
const SQL_STATEMENT_TIMEOUT_MS: &str = "5000";
const SQL_LOCK_TIMEOUT_MS: &str = "1000";
pub const AUTH_AUDIT_PENDING_SLA_SECS: i64 = 300;

fn epoch_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

tokio::task_local! {
    static INVITE_ACTOR: String;
    static INVITE_ACTOR_SESSION_HASH: String;
}

/// Run an invitation mutation with the authenticated Owner identity available
/// to the persistence adapter. The task-local avoids putting mutable actor
/// state on a process-wide pool or router state; missing context fails closed.
pub async fn with_actor_user_id<T, Fut>(user_id: &UserId, future: Fut) -> Result<T, InviteError>
where
    Fut: Future<Output = Result<T, InviteError>>,
{
    INVITE_ACTOR.scope(user_id.0.clone(), future).await
}

/// Run an invitation mutation with both the authenticated Owner identity and
/// the hash of that request's live session. The SQL adapter turns this hash
/// into a transaction-bound capability before permitting the mutation; an
/// actor GUC by itself is intentionally insufficient.
pub async fn with_authenticated_actor<T, Fut>(
    user_id: &UserId,
    session_hash: &str,
    future: Fut,
) -> Result<T, InviteError>
where
    Fut: Future<Output = Result<T, InviteError>>,
{
    INVITE_ACTOR
        .scope(
            user_id.0.clone(),
            INVITE_ACTOR_SESSION_HASH.scope(session_hash.to_owned(), future),
        )
        .await
}

fn invite_actor() -> Result<Uuid, InviteError> {
    INVITE_ACTOR
        .try_with(|actor| Uuid::parse_str(actor))
        .map_err(|_| InviteError::Store("actor context missing".to_string()))?
        .map_err(|_| InviteError::Store("invalid actor identity".to_string()))
}

fn invite_session_hash() -> Result<String, InviteError> {
    INVITE_ACTOR_SESSION_HASH
        .try_with(Clone::clone)
        .map_err(|_| InviteError::Store("authenticated actor capability missing".to_string()))
}

fn invite_hash_from_id(id: &str) -> Option<&str> {
    let hash = id.strip_prefix("inv-")?;
    (hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')))
    .then_some(hash)
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::Owner => "owner",
        Role::Member => "member",
    }
}

fn session_store_error<E>(_error: E) -> SessionError {
    // SQLx errors may include connection details.  Auth handlers must not
    // reflect them to a browser, and this adapter never logs the raw error.
    SessionError::Store("database operation failed".to_string())
}

fn session_store_timeout() -> SessionError {
    SessionError::Store("database operation timed out".to_string())
}

fn invite_store_error<E>(_error: E) -> InviteError {
    InviteError::Store("database operation failed".to_string())
}

fn invite_store_timeout() -> InviteError {
    InviteError::Store("database operation timed out".to_string())
}

fn parse_user_id(value: &UserId) -> Result<Uuid, SessionError> {
    Uuid::parse_str(&value.0).map_err(|_| SessionError::Store("invalid user id".to_string()))
}

/// The durable session store shared with `/api/v1` session extraction.
#[derive(Clone)]
pub struct PostgresSessionStore {
    app_pool: PgPool,
    admin_pool: PgPool,
}

impl PostgresSessionStore {
    pub fn new(app_pool: PgPool, admin_pool: PgPool) -> Self {
        Self {
            app_pool,
            admin_pool,
        }
    }

    async fn user_id_for_hash(&self, token_hash: &str) -> Result<Option<Uuid>, SessionError> {
        tokio::time::timeout(
            AUTH_SQL_OPERATION_DEADLINE,
            sqlx::query_scalar::<_, Uuid>(
                "SELECT s.user_id FROM public.web_sessions s \
                 JOIN public.users u ON u.id = s.user_id \
                 WHERE s.session_hash = $1 \
                   AND s.revoked_at IS NULL \
                   AND u.provisioned_by_user_id IS NULL",
            )
            .bind(token_hash)
            .fetch_optional(&self.admin_pool),
        )
        .await
        .map_err(|_| session_store_timeout())?
        .map_err(session_store_error)
    }

    async fn set_actor(
        transaction: &mut sqlx::Transaction<'static, sqlx::Postgres>,
        user_id: Uuid,
    ) -> Result<(), SessionError> {
        sqlx::query("SELECT set_config($1, $2, true)")
            .bind(ACTOR_GUC)
            .bind(user_id.to_string())
            .execute(&mut **transaction)
            .await
            .map(|_| ())
            .map_err(session_store_error)
    }

    async fn set_transaction_timeouts(
        transaction: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    ) -> Result<(), SessionError> {
        // These are transaction-local settings, not connection-wide state.
        // They bound direct session DML while preserving the single
        // insert/update + audit enqueue + commit transaction.
        sqlx::query("SELECT set_config('statement_timeout', $1, true), set_config('lock_timeout', $2, true)")
            .bind(SQL_STATEMENT_TIMEOUT_MS)
            .bind(SQL_LOCK_TIMEOUT_MS)
            .execute(&mut **transaction)
            .await
            .map(|_| ())
            .map_err(session_store_error)
    }

    async fn insert_inner(&self, session: StoredSession) -> Result<(), SessionError> {
        let user_id = parse_user_id(&session.user_id)?;
        let mut transaction = self.app_pool.begin().await.map_err(session_store_error)?;
        Self::set_transaction_timeouts(&mut transaction).await?;
        Self::set_actor(&mut transaction, user_id).await?;
        sqlx::query(
            "INSERT INTO public.web_sessions \
             (user_id, session_hash, csrf_hash, expires_at, created_at, amr, auth_time) \
             VALUES ($1, $2, $3, to_timestamp($4::double precision), \
                     to_timestamp($5::double precision), $6, \
                     to_timestamp($7::double precision))",
        )
        .bind(user_id)
        .bind(&session.token_hash)
        .bind(&session.csrf_token_hash)
        .bind(session.expires_at_secs)
        .bind(session.created_at_secs)
        .bind(&session.amr)
        .bind(session.auth_time_secs)
        .execute(&mut *transaction)
        .await
        .map_err(session_store_error)?;
        sqlx::query(
            "SELECT public.enqueue_auth_audit($1, 'auth.login_succeeded', $2, 'session', $3, NULL, $4)",
        )
        .bind(format!("session:{}:issued", session.token_hash))
        .bind(user_id)
        .bind(&session.token_hash)
        .bind(session.created_at_secs)
        .execute(&mut *transaction)
        .await
        .map_err(session_store_error)?;
        transaction.commit().await.map_err(session_store_error)
    }

    async fn revoke_inner(&self, token_hash: &str, user_id: Uuid) -> Result<(), SessionError> {
        let mut transaction = self.app_pool.begin().await.map_err(session_store_error)?;
        Self::set_transaction_timeouts(&mut transaction).await?;
        Self::set_actor(&mut transaction, user_id).await?;
        let result = sqlx::query(
            "UPDATE public.web_sessions SET revoked_at = now() \
             WHERE session_hash = $1 AND user_id = $2 AND revoked_at IS NULL",
        )
        .bind(token_hash)
        .bind(user_id)
        .execute(&mut *transaction)
        .await
        .map_err(session_store_error)?;
        if result.rows_affected() > 0 {
            sqlx::query(
                "SELECT public.enqueue_auth_audit($1, 'auth.session_revoked', $2, 'session', $3, NULL, $4)",
            )
            .bind(format!("session:{}:revoked", token_hash))
            .bind(user_id)
            .bind(token_hash)
            .bind(chrono::Utc::now().timestamp())
            .execute(&mut *transaction)
            .await
            .map_err(session_store_error)?;
        }
        transaction.commit().await.map_err(session_store_error)
    }

    async fn update_csrf_inner(
        &self,
        token_hash: &str,
        csrf_token_hash: &str,
        user_id: Uuid,
    ) -> Result<(), SessionError> {
        let mut transaction = self.app_pool.begin().await.map_err(session_store_error)?;
        Self::set_transaction_timeouts(&mut transaction).await?;
        Self::set_actor(&mut transaction, user_id).await?;
        let result = sqlx::query(
            "UPDATE public.web_sessions SET csrf_hash = $1 \
             WHERE session_hash = $2 AND user_id = $3 AND revoked_at IS NULL",
        )
        .bind(csrf_token_hash)
        .bind(token_hash)
        .bind(user_id)
        .execute(&mut *transaction)
        .await
        .map_err(session_store_error)?;
        if result.rows_affected() == 0 {
            transaction.rollback().await.map_err(session_store_error)?;
            return Err(SessionError::UnknownSession);
        }
        sqlx::query(
            "SELECT public.enqueue_auth_audit($1, 'auth.csrf_rotated', $2, 'session', $3, NULL, $4)",
        )
        .bind(format!("session:{}:csrf:{}", token_hash, csrf_token_hash))
        .bind(user_id)
        .bind(token_hash)
        .bind(chrono::Utc::now().timestamp())
        .execute(&mut *transaction)
        .await
        .map_err(session_store_error)?;
        transaction.commit().await.map_err(session_store_error)
    }
}

#[async_trait::async_trait]
impl SessionStore for PostgresSessionStore {
    async fn insert(&self, session: StoredSession) -> Result<(), SessionError> {
        tokio::time::timeout(SESSION_TRANSACTION_DEADLINE, self.insert_inner(session))
            .await
            .map_err(|_| session_store_timeout())?
    }

    async fn lookup(&self, token_hash: &str) -> Result<Option<StoredSession>, SessionError> {
        let row = tokio::time::timeout(
            AUTH_SQL_OPERATION_DEADLINE,
            sqlx::query(
                "SELECT s.user_id, r.id AS role_id, s.csrf_hash, \
                    EXTRACT(EPOCH FROM s.expires_at)::bigint AS expires_at_secs, \
                    EXTRACT(EPOCH FROM COALESCE(s.auth_time, s.created_at))::bigint \
                        AS auth_time_secs, \
                    EXTRACT(EPOCH FROM s.created_at)::bigint AS created_at_secs, s.amr \
             FROM public.web_sessions s \
             JOIN public.users u ON u.id = s.user_id \
             JOIN public.user_roles ur ON ur.user_id = s.user_id \
             JOIN public.roles r ON r.id = ur.role_id \
             WHERE s.session_hash = $1 AND s.revoked_at IS NULL \
               AND u.provisioned_by_user_id IS NULL \
             ORDER BY CASE WHEN r.id = 'owner' THEN 0 ELSE 1 END \
             LIMIT 1",
            )
            .bind(token_hash)
            .fetch_optional(&self.admin_pool),
        )
        .await
        .map_err(|_| session_store_timeout())?
        .map_err(session_store_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let user_id: Uuid = row.try_get("user_id").map_err(session_store_error)?;
        let role_id: String = row.try_get("role_id").map_err(session_store_error)?;
        let role = match role_id.as_str() {
            "owner" => Role::Owner,
            "member" => Role::Member,
            _ => return Ok(None),
        };
        let csrf_hash: String = row.try_get("csrf_hash").map_err(session_store_error)?;
        let expires_at_secs: i64 = row
            .try_get("expires_at_secs")
            .map_err(session_store_error)?;
        let auth_time_secs: i64 = row.try_get("auth_time_secs").map_err(session_store_error)?;
        let created_at_secs: i64 = row
            .try_get("created_at_secs")
            .map_err(session_store_error)?;
        let amr: Vec<String> = row.try_get("amr").map_err(session_store_error)?;
        Ok(Some(StoredSession {
            token_hash: token_hash.to_string(),
            user_id: UserId::new(user_id.to_string()),
            role,
            auth_time_secs,
            amr,
            csrf_token_hash: csrf_hash,
            created_at_secs,
            expires_at_secs,
        }))
    }

    async fn revoke(&self, token_hash: &str) -> Result<(), SessionError> {
        tokio::time::timeout(SESSION_TRANSACTION_DEADLINE, async {
            let Some(user_id) = self.user_id_for_hash(token_hash).await? else {
                return Ok(());
            };
            self.revoke_inner(token_hash, user_id).await
        })
        .await
        .map_err(|_| session_store_timeout())?
    }

    async fn update_csrf(
        &self,
        token_hash: &str,
        csrf_token_hash: &str,
    ) -> Result<(), SessionError> {
        tokio::time::timeout(SESSION_TRANSACTION_DEADLINE, async {
            let Some(user_id) = self.user_id_for_hash(token_hash).await? else {
                return Err(SessionError::UnknownSession);
            };
            self.update_csrf_inner(token_hash, csrf_token_hash, user_id)
                .await
        })
        .await
        .map_err(|_| session_store_timeout())?
    }
}

/// Identity adapter. Reads use the dedicated `admin` pool; first-time binding
/// uses only the migration-owned `bind_redeemed_identity` capability exposed
/// to the `app` role. No table write grant is needed by the serving role.
#[derive(Clone)]
pub struct PostgresUserStore {
    app_pool: PgPool,
    admin_pool: PgPool,
}

impl PostgresUserStore {
    pub fn new(app_pool: PgPool, admin_pool: PgPool) -> Self {
        Self {
            app_pool,
            admin_pool,
        }
    }
}

#[async_trait::async_trait]
impl UserStore for PostgresUserStore {
    async fn find_by_binding(
        &self,
        issuer: &str,
        subject: &str,
    ) -> Result<Option<UserRecord>, InviteError> {
        let row = tokio::time::timeout(
            AUTH_SQL_OPERATION_DEADLINE,
            sqlx::query(
                "SELECT u.id, u.issuer, u.subject, u.email, \
                        COALESCE(MAX(CASE WHEN ur.role_id = 'owner' THEN 'owner' ELSE ur.role_id END), '') \
                            AS role_id, \
                        EXTRACT(EPOCH FROM u.created_at)::bigint AS created_at_secs \
                 FROM public.users u \
                 LEFT JOIN public.user_roles ur ON ur.user_id = u.id \
                 WHERE u.issuer = $1 AND u.subject = $2 \
                   AND u.provisioned_by_user_id IS NULL \
                 GROUP BY u.id, u.issuer, u.subject, u.email, u.created_at \
                 LIMIT 1",
            )
            .bind(issuer)
            .bind(subject)
            .fetch_optional(&self.admin_pool),
        )
        .await
        .map_err(|_| invite_store_timeout())?
        .map_err(invite_store_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let role_id: String = row.try_get("role_id").map_err(invite_store_error)?;
        let role = match role_id.as_str() {
            "owner" => Role::Owner,
            "member" => Role::Member,
            _ => return Ok(None),
        };
        let id: Uuid = row.try_get("id").map_err(invite_store_error)?;
        Ok(Some(UserRecord {
            binding_issuer: row.try_get("issuer").map_err(invite_store_error)?,
            binding_subject: row.try_get("subject").map_err(invite_store_error)?,
            user_id: UserId::new(id.to_string()),
            role,
            email: row.try_get("email").map_err(invite_store_error)?,
            created_at_secs: row.try_get("created_at_secs").map_err(invite_store_error)?,
        }))
    }

    async fn insert_user(&self, user: UserRecord) -> Result<UserId, InviteError> {
        tokio::time::timeout(
            AUTH_SQL_OPERATION_DEADLINE,
            sqlx::query_scalar::<_, Uuid>("SELECT public.bind_redeemed_identity($1, $2, $3, $4)")
                .bind(user.binding_issuer)
                .bind(user.binding_subject)
                .bind(user.email)
                .bind(role_name(user.role))
                .fetch_one(&self.app_pool),
        )
        .await
        .map_err(|_| invite_store_timeout())?
        .map(|id| UserId::new(id.to_string()))
        .map_err(invite_store_error)
    }

    async fn update_profile(&self, _user_id: &str, _email: &str) -> Result<(), InviteError> {
        // `users` is intentionally read-only to serving roles.  Keeping the
        // immutable issuer/subject binding usable is safer than attempting an
        // unprivileged UPDATE and turning every returning user into a 500.
        Ok(())
    }
}

/// Invitation adapter. Reads use `admin`; mutations call narrowly scoped
/// migration-owned functions through `app`, preserving the serving role's
/// read-only table grants and the invitation RLS boundary.
#[derive(Clone)]
pub struct PostgresInviteStore {
    app_pool: PgPool,
    admin_pool: PgPool,
}

impl PostgresInviteStore {
    pub fn new(app_pool: PgPool, admin_pool: PgPool) -> Self {
        Self {
            app_pool,
            admin_pool,
        }
    }
}

#[async_trait::async_trait]
impl InviteStore for PostgresInviteStore {
    async fn insert(&self, invite: InviteRecord) -> Result<(), InviteError> {
        let owner_id = invite_actor()?;
        let session_hash = invite_session_hash()?;
        let invite_hash = invite_hash_from_id(&invite.id)
            .ok_or_else(|| InviteError::Store("invalid invitation identifier".to_string()))?;
        tokio::time::timeout(AUTH_SQL_OPERATION_DEADLINE, async {
            let mut transaction = self.app_pool.begin().await.map_err(invite_store_error)?;
            let capability: Uuid =
                sqlx::query_scalar("SELECT public.authenticate_identity_actor($1, $2)")
                    .bind(owner_id)
                    .bind(session_hash)
                    .fetch_one(&mut *transaction)
                    .await
                    .map_err(invite_store_error)?;
            sqlx::query_scalar::<_, Uuid>(
                "SELECT public.create_invitation($1, $2, $3, $4, $5, $6)",
            )
            .bind(owner_id)
            .bind(invite.email)
            .bind(role_name(invite.role))
            .bind(invite_hash)
            .bind(invite.expires_at_secs)
            .bind(capability)
            .fetch_one(&mut *transaction)
            .await
            .map_err(invite_store_error)?;
            transaction.commit().await.map_err(invite_store_error)
        })
        .await
        .map_err(|_| invite_store_timeout())?
    }

    async fn find_by_email(&self, email: &str) -> Result<Option<InviteRecord>, InviteError> {
        let row = tokio::time::timeout(
            AUTH_SQL_OPERATION_DEADLINE,
            sqlx::query(
                "SELECT i.id, i.email, i.status, \
                        EXTRACT(EPOCH FROM i.created_at)::bigint AS created_at_secs, \
                        EXTRACT(EPOCH FROM i.expires_at)::bigint AS expires_at_secs, \
                        i.role_id \
                 FROM public.invitations i \
                 WHERE pg_catalog.lower(pg_catalog.btrim(i.email)) = \
                           pg_catalog.lower(pg_catalog.btrim($1)) \
                   AND (\
                       i.status = 'PENDING' \
                       OR (\
                           i.status = 'REDEEMED' \
                           AND EXISTS (\
                               SELECT 1 FROM public.users provisional_user \
                               WHERE provisional_user.id = i.redeemed_by_user_id \
                                 AND provisional_user.provisioned_by_user_id IS NOT NULL \
                           )\
                       )\
                   ) \
                 ORDER BY i.created_at DESC LIMIT 1",
            )
            .bind(email)
            .fetch_optional(&self.admin_pool),
        )
        .await
        .map_err(|_| invite_store_timeout())?
        .map_err(invite_store_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let role_id: String = row.try_get("role_id").map_err(invite_store_error)?;
        let role = match role_id.as_str() {
            "owner" => Role::Owner,
            "member" => Role::Member,
            _ => return Ok(None),
        };
        let status: String = row.try_get("status").map_err(invite_store_error)?;
        let id: Uuid = row.try_get("id").map_err(invite_store_error)?;
        Ok(Some(InviteRecord {
            id: id.to_string(),
            email: row.try_get("email").map_err(invite_store_error)?,
            role,
            created_at_secs: row.try_get("created_at_secs").map_err(invite_store_error)?,
            expires_at_secs: row.try_get("expires_at_secs").map_err(invite_store_error)?,
            redeemed_by: (status == "REDEEMED").then(|| ("database".to_string(), id.to_string())),
            redeemed_at_secs: None,
        }))
    }

    async fn claim(
        &self,
        id: &str,
        issuer: &str,
        subject: &str,
        _at_secs: i64,
    ) -> Result<bool, InviteError> {
        tokio::time::timeout(AUTH_SQL_OPERATION_DEADLINE, async {
            let invitation_id = Uuid::parse_str(id)
                .map_err(|_| InviteError::Store("invalid invitation identifier".to_string()))?;
            let invitation: Option<(Uuid, String)> = tokio::time::timeout(
                AUTH_SQL_OPERATION_DEADLINE,
                sqlx::query_as("SELECT user_id, invite_hash FROM public.invitations WHERE id = $1")
                    .bind(invitation_id)
                    .fetch_optional(&self.admin_pool),
            )
            .await
            .map_err(|_| invite_store_timeout())?
            .map_err(invite_store_error)?;
            let Some((owner_id, invite_hash)) = invitation else {
                return Ok(false);
            };
            sqlx::query_scalar::<_, bool>("SELECT public.claim_invitation($1, $2, $3, $4, $5)")
                .bind(owner_id)
                .bind(invitation_id)
                .bind(invite_hash)
                .bind(issuer)
                .bind(subject)
                .fetch_one(&self.app_pool)
                .await
                .map_err(invite_store_error)
        })
        .await
        .map_err(|_| invite_store_timeout())?
    }
}

/// Durable auth audit outbox worker. State-changing adapters enqueue in their
/// existing PostgreSQL transaction; this worker only copies committed rows to
/// the append-only audit log. The worker never acknowledges a state mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthAuditReadiness {
    pub backlog: i64,
    pub oldest_pending_age_secs: i64,
    pub worker_alive: bool,
    pub worker_stale: bool,
    pub consecutive_failures: u64,
    pub failures: u64,
}

impl AuthAuditReadiness {
    pub fn is_ready(self) -> bool {
        self.worker_alive
            && !self.worker_stale
            && self.consecutive_failures < MAX_CONSECUTIVE_FAILURES
            && self.oldest_pending_age_secs <= AUTH_AUDIT_PENDING_SLA_SECS
    }
}

/// Advance the writer's consecutive-failure state for one poll. An empty poll
/// is not a successful delivery: rows may simply be in backoff, or a runner
/// may have raced another worker. Only a poll that actually delivered at least
/// one row clears an unresolved failure streak.
fn next_consecutive_failures(current: u64, delivered: usize, failed: usize) -> u64 {
    if failed > 0 {
        current.saturating_add(1)
    } else if delivered > 0 {
        0
    } else {
        current
    }
}

pub struct PostgresAuthAudit {
    pool: PgPool,
    worker: Mutex<Option<JoinHandle<()>>>,
    closed: Arc<AtomicBool>,
    worker_alive: Arc<AtomicBool>,
    last_heartbeat: Arc<AtomicU64>,
    consecutive_failures: Arc<AtomicU64>,
    failures: Arc<AtomicU64>,
    backlog: Arc<AtomicUsize>,
}

impl PostgresAuthAudit {
    pub fn new(pool: PgPool) -> Self {
        let closed = Arc::new(AtomicBool::new(false));
        let worker_alive = Arc::new(AtomicBool::new(false));
        let last_heartbeat = Arc::new(AtomicU64::new(epoch_now()));
        let consecutive_failures = Arc::new(AtomicU64::new(0));
        let failures = Arc::new(AtomicU64::new(0));
        let backlog = Arc::new(AtomicUsize::new(0));
        let worker_closed = Arc::clone(&closed);
        let worker_alive_flag = Arc::clone(&worker_alive);
        let worker_heartbeat = Arc::clone(&last_heartbeat);
        let worker_consecutive_failures = Arc::clone(&consecutive_failures);
        let worker_failures = Arc::clone(&failures);
        let worker_backlog = Arc::clone(&backlog);
        let worker_pool = pool.clone();
        let worker = thread::Builder::new()
            .name("auth-audit-writer".to_string())
            .spawn(move || {
                worker_alive_flag.store(true, Ordering::Release);
                worker_heartbeat.store(epoch_now(), Ordering::Release);
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(_) => {
                        worker_failures.fetch_add(1, Ordering::Relaxed);
                        worker_alive_flag.store(false, Ordering::Release);
                        return;
                    }
                };
                let mut consecutive_failures = 0_u64;
                let mut next_cleanup = Instant::now() + Duration::from_secs(60);
                loop {
                    worker_heartbeat.store(epoch_now(), Ordering::Release);
                    match runtime.block_on(deliver_outbox_batch(&worker_pool)) {
                        Ok((delivered, failed)) => {
                            if failed > 0 {
                                worker_failures.fetch_add(failed as u64, Ordering::Relaxed);
                            }
                            consecutive_failures =
                                next_consecutive_failures(consecutive_failures, delivered, failed);
                            worker_consecutive_failures
                                .store(consecutive_failures, Ordering::Release);
                            let _ = worker_backlog.fetch_update(
                                Ordering::AcqRel,
                                Ordering::Acquire,
                                |current| Some(current.saturating_sub(delivered)),
                            );
                            if Instant::now() >= next_cleanup {
                                if runtime
                                    .block_on(prune_outbox(&worker_pool, 604_800, 256))
                                    .is_err()
                                {
                                    worker_failures.fetch_add(1, Ordering::Relaxed);
                                }
                                next_cleanup = Instant::now() + Duration::from_secs(60);
                            }
                            if worker_closed.load(Ordering::Acquire)
                                && delivered == 0
                                && failed == 0
                            {
                                break;
                            }
                            if delivered == 0 {
                                thread::sleep(std::time::Duration::from_millis(100));
                            }
                        }
                        Err(_) => {
                            worker_failures.fetch_add(1, Ordering::Relaxed);
                            consecutive_failures = consecutive_failures.saturating_add(1);
                            worker_consecutive_failures
                                .store(consecutive_failures, Ordering::Release);
                            if worker_closed.load(Ordering::Acquire)
                                && consecutive_failures >= MAX_CONSECUTIVE_FAILURES
                            {
                                break;
                            }
                            thread::sleep(std::time::Duration::from_millis(100));
                        }
                    }
                }
                worker_alive_flag.store(false, Ordering::Release);
            })
            .ok();
        let sink = Self {
            pool,
            worker: Mutex::new(worker),
            closed,
            worker_alive,
            last_heartbeat,
            consecutive_failures,
            failures,
            backlog,
        };
        // A thread creation failure is observable through the same metric and
        // causes subsequent records to fail closed rather than disappear.
        if sink.worker.lock().unwrap().is_none() {
            sink.closed.store(true, Ordering::Release);
            sink.worker_alive.store(false, Ordering::Release);
            sink.failures.fetch_add(1, Ordering::Relaxed);
        }
        sink
    }

    /// Number of records rejected by the closed/disconnected sink or failed
    /// by the durable writer. This is intended for readiness/metrics wiring.
    pub fn failure_count(&self) -> u64 {
        self.failures.load(Ordering::Acquire)
    }

    pub fn backlog(&self) -> usize {
        self.backlog.load(Ordering::Acquire)
    }

    pub async fn readiness(&self) -> Result<AuthAuditReadiness, AuthAuditError> {
        let stats = tokio::time::timeout(
            Duration::from_secs(4),
            sqlx::query_as::<_, (i64, i64)>(
                "SELECT pending_count, oldest_pending_age_secs \
                 FROM public.auth_audit_outbox_stats()",
            )
            .fetch_one(&self.pool),
        )
        .await
        .map_err(|_| AuthAuditError::Unavailable)?
        .map_err(|_| AuthAuditError::Unavailable)?;
        let (backlog, oldest_pending_age_secs) = stats;
        let heartbeat = self.last_heartbeat.load(Ordering::Acquire);
        Ok(AuthAuditReadiness {
            backlog,
            oldest_pending_age_secs,
            worker_alive: self.worker_alive.load(Ordering::Acquire),
            worker_stale: epoch_now().saturating_sub(heartbeat) > WORKER_STALE_SECS,
            consecutive_failures: self.consecutive_failures.load(Ordering::Acquire),
            failures: self.failure_count(),
        })
    }

    pub async fn durable_backlog(&self) -> Result<i64, AuthAuditError> {
        Ok(self.readiness().await?.backlog)
    }

    /// Stop polling and drain currently available durable rows. Repeated calls
    /// are harmless and do not block; repeated delivery failures are surfaced
    /// through `failure_count` and terminate the bounded shutdown retry loop.
    pub fn shutdown(&self) {
        self.closed.store(true, Ordering::Release);
    }

    /// Join the drained writer from an explicit process shutdown hook. This
    /// is intentionally separate from `shutdown` so dropping router state on
    /// an async executor never blocks that executor.
    pub fn shutdown_and_wait(&self) {
        self.shutdown();
        if let Some(worker) = self.worker.lock().unwrap().take() {
            // `JoinHandle::join` has no timeout. Move it to a short-lived
            // reaper and bound the caller's wait so HTTP/runtime shutdown can
            // never hang behind a stuck database socket. The worker's SQL
            // calls have their own deadline and cooperative closed flag.
            let (done_tx, done_rx) = std::sync::mpsc::sync_channel(1);
            thread::spawn(move || {
                let _ = worker.join();
                let _ = done_tx.send(());
            });
            let _ = done_rx.recv_timeout(SHUTDOWN_DEADLINE);
        }
    }
}

#[async_trait::async_trait]
impl AuthAudit for PostgresAuthAudit {
    fn record(&self, event: AuthAuditEvent) -> Result<(), AuthAuditError> {
        // Mutation adapters enqueue in their transaction. This method is
        // retained for simulator-compatible service calls and intentionally
        // does not acknowledge a production event as durable.
        let _ = event;
        Ok(())
    }

    async fn record_durable(&self, event: AuthAuditEvent) -> Result<(), AuthAuditError> {
        if self.closed.load(Ordering::Acquire) {
            self.failures.fetch_add(1, Ordering::Relaxed);
            return Err(AuthAuditError::Closed);
        }
        let event_id = Uuid::new_v4();
        let enqueue = tokio::time::timeout(
            AUTH_SQL_OPERATION_DEADLINE,
            sqlx::query("SELECT public.enqueue_auth_audit($1, $2, $3, $4, $5, $6, $7)")
                .bind(format!("event:{}", event_id))
                .bind(format!("auth.{}", event.kind.as_str()))
                .bind(
                    event
                        .user
                        .as_deref()
                        .and_then(|id| Uuid::parse_str(id).ok()),
                )
                .bind("auth")
                .bind(event.user.as_deref().unwrap_or_default())
                .bind(event.reason.or(Some(event.detail)))
                .bind(event.at_secs)
                .execute(&self.pool),
        )
        .await;
        if !matches!(enqueue, Ok(Ok(_))) {
            self.failures.fetch_add(1, Ordering::Relaxed);
            return Err(AuthAuditError::Unavailable);
        }
        self.backlog.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }
}

impl Drop for PostgresAuthAudit {
    fn drop(&mut self) {
        self.shutdown();
    }
}

async fn deliver_outbox_batch(pool: &PgPool) -> Result<(usize, usize), sqlx::Error> {
    tokio::time::timeout(
        Duration::from_secs(6),
        sqlx::query_as::<_, (i32, i32)>(
            "SELECT delivered_count, failed_count \
             FROM public.deliver_auth_audit_batch($1)",
        )
        .bind(64_i32)
        .fetch_one(pool),
    )
    .await
    .map_err(|_| sqlx::Error::PoolTimedOut)?
    .map(|(delivered, failed)| (delivered as usize, failed as usize))
}

async fn prune_outbox(pool: &PgPool, keep_seconds: i64, limit: i32) -> Result<i64, sqlx::Error> {
    tokio::time::timeout(
        Duration::from_secs(6),
        sqlx::query_scalar("SELECT public.prune_auth_audit_outbox($1, $2)")
            .bind(keep_seconds)
            .bind(limit)
            .fetch_one(pool),
    )
    .await
    .map_err(|_| sqlx::Error::PoolTimedOut)?
}

#[cfg(test)]
mod tests {
    use super::*;
    use auth::audit::AuthAuditKind;
    use sqlx::postgres::PgPoolOptions;

    fn audit_event() -> AuthAuditEvent {
        AuthAuditEvent {
            at_secs: 1,
            kind: AuthAuditKind::LoginDenied,
            user: None,
            reason: Some("TEST".to_string()),
            detail: "test event".to_string(),
        }
    }

    #[test]
    fn invitation_ids_carry_only_a_lowercase_hash() {
        let hash = "a1".repeat(32);
        assert_eq!(
            invite_hash_from_id(&format!("inv-{hash}")),
            Some(hash.as_str())
        );
        assert!(invite_hash_from_id("inv-too-short").is_none());
        assert!(invite_hash_from_id(&format!("inv-{}", "A".repeat(64))).is_none());
        assert!(invite_hash_from_id("invite-a1").is_none());
    }

    #[test]
    fn auth_sql_deadlines_bound_inner_work_before_transaction_budget() {
        assert!(AUTH_SQL_OPERATION_DEADLINE < SESSION_TRANSACTION_DEADLINE);
        assert!(AUTH_SQL_OPERATION_DEADLINE <= Duration::from_secs(6));
        assert!(SESSION_TRANSACTION_DEADLINE <= Duration::from_secs(7));
    }

    #[tokio::test]
    async fn invitation_actor_context_is_required_and_scoped() {
        let missing = async { invite_actor() }.await;
        assert!(
            matches!(missing, Err(InviteError::Store(message)) if message == "actor context missing")
        );

        let user = UserId::new(Uuid::new_v4().to_string());
        let observed = with_actor_user_id(&user, async { invite_actor() })
            .await
            .expect("scoped actor");
        assert_eq!(observed, Uuid::parse_str(&user.0).expect("uuid actor"));

        let invalid =
            with_actor_user_id(&UserId::new("usr_not_a_uuid"), async { invite_actor() }).await;
        assert!(
            matches!(invalid, Err(InviteError::Store(message)) if message == "invalid actor identity")
        );
    }

    #[tokio::test]
    async fn audit_shutdown_is_nonblocking_and_rejects_durable_events() {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://127.0.0.1:1/credential_free_test")
            .expect("lazy pool construction");
        let audit = PostgresAuthAudit::new(pool);
        audit.shutdown();
        assert_eq!(audit.backlog(), 0);
        assert_eq!(audit.record(audit_event()), Ok(()));
        assert_eq!(
            audit.record_durable(audit_event()).await,
            Err(AuthAuditError::Closed)
        );
        audit.shutdown_and_wait();
    }

    #[tokio::test]
    async fn audit_write_failures_are_observable_after_retries() {
        let pool = PgPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_millis(25))
            .connect_lazy("postgres://127.0.0.1:1/credential_free_test")
            .expect("lazy pool construction");
        let audit = PostgresAuthAudit::new(pool);
        assert_eq!(
            audit.record_durable(audit_event()).await,
            Err(AuthAuditError::Unavailable)
        );
        let shutdown_started = std::time::Instant::now();
        audit.shutdown_and_wait();
        assert!(
            shutdown_started.elapsed() < SHUTDOWN_DEADLINE + Duration::from_secs(1),
            "audit shutdown exceeded its deadline"
        );
        assert!(audit.failure_count() >= 1);
    }

    #[test]
    fn audit_readiness_rejects_dead_worker_failures_and_stale_backlog() {
        let healthy = AuthAuditReadiness {
            backlog: 1,
            oldest_pending_age_secs: AUTH_AUDIT_PENDING_SLA_SECS,
            worker_alive: true,
            worker_stale: false,
            consecutive_failures: 0,
            failures: 0,
        };
        assert!(healthy.is_ready());
        assert!(
            !AuthAuditReadiness {
                worker_alive: false,
                ..healthy
            }
            .is_ready()
        );
        assert!(
            !AuthAuditReadiness {
                worker_stale: true,
                ..healthy
            }
            .is_ready()
        );
        assert!(
            !AuthAuditReadiness {
                oldest_pending_age_secs: AUTH_AUDIT_PENDING_SLA_SECS + 1,
                ..healthy
            }
            .is_ready()
        );
        assert!(
            !AuthAuditReadiness {
                consecutive_failures: 3,
                ..healthy
            }
            .is_ready()
        );
    }

    #[test]
    fn audit_empty_backoff_polls_preserve_unresolved_failure_streak() {
        let mut consecutive = 0;
        consecutive = next_consecutive_failures(consecutive, 0, 2);
        assert_eq!(consecutive, 1);
        consecutive = next_consecutive_failures(consecutive, 0, 1);
        assert_eq!(consecutive, 2);
        consecutive = next_consecutive_failures(consecutive, 0, 1);
        assert_eq!(consecutive, 3);
        // The failed rows are unavailable during exponential backoff. Empty
        // polls must not make readiness look healthy again.
        consecutive = next_consecutive_failures(consecutive, 0, 0);
        assert_eq!(consecutive, 3);
        consecutive = next_consecutive_failures(consecutive, 0, 0);
        assert_eq!(consecutive, 3);
        assert!(
            !AuthAuditReadiness {
                backlog: 1,
                oldest_pending_age_secs: 1,
                worker_alive: true,
                worker_stale: false,
                consecutive_failures: consecutive,
                failures: 2,
            }
            .is_ready()
        );
        // A real delivery, rather than an empty poll, resolves the streak.
        consecutive = next_consecutive_failures(consecutive, 1, 0);
        assert_eq!(consecutive, 0);
    }
}
