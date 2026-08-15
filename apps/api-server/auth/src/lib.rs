//! Axum auth router: the HTTP surface of the confidential OIDC/session
//! authority. Every decision delegates to `crates/auth` (`AuthService`,
//! `PendingAuthStore`, `SessionStore`); this crate adds no second session
//! implementation. Routes:
//!
//! - `GET /auth/login`        -> 303 to the provider with PKCE S256 + state/nonce
//! - `GET /auth/callback`     -> validates, redeems invite, issues session cookie
//! - `POST /auth/logout`      -> revokes the session (CSRF-protected)
//! - `GET /auth/session`      -> current session (user id, role, expiry)
//! - `GET /auth/csrf`         -> rotates and returns the synchronizer token
//! - `POST /auth/invites`     -> Owner-only invite creation (CSRF-protected)
//! - `GET /auth/step-up-check`-> Owner step-up verdict (auth_time/amr)
//!
//! The browser only ever holds the opaque `__Host-lagrange_session` cookie and
//! a per-session CSRF token; provider tokens never cross this boundary. The
//! OIDC HTTP transport (token exchange + JWKS fetch) ships here as
//! [`HttpOidcTransport`].

pub mod config;
pub mod postgres;

use auth::audit::{AuthAudit, AuthAuditEvent, AuthAuditKind};
use auth::clock::SystemClock;
use auth::entitlement::Role;
use auth::invites::{InviteError, InviteService};
use auth::oidc::{
    DEFAULT_PENDING_TTL_SECS, InMemoryPendingAuthStore, OidcClient, OidcError, PendingAuth,
    PendingAuthStore, TransportError,
};
use auth::service::{AuthError, AuthService};
use auth::sessions::SessionInfo;
use auth::sessions::SessionService;
use auth::sessions::cookie;
use auth::stepup::require_owner_step_up;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;
const OIDC_TRANSACTION_COOKIE: &str = "__Host-lagrange_oidc_tx";
const OIDC_TRANSACTION_MAX_AGE: i64 = DEFAULT_PENDING_TTL_SECS;
/// Explicit transport limits for both the token exchange and JWKS fetch.
/// `reqwest::Client::timeout` is a total request deadline, including response
/// body consumption; connect timeout bounds DNS/TCP/TLS establishment.
pub const OIDC_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub const OIDC_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct RouterState {
    pub auth: Arc<AuthService>,
    pub pending: Arc<dyn PendingAuthStore>,
    pub audit: Arc<dyn AuthAudit>,
    pub step_up_max_auth_age_secs: i64,
    /// MAC key for the short-lived browser transaction cookie. It is derived
    /// from the confidential client secret in production and injected as a
    /// deterministic test key in simulator tests.
    pub transaction_cookie_key: Arc<[u8; 32]>,
    /// Production outbox worker for readiness/metrics/lifecycle wiring.
    /// Simulator state leaves this unset.
    pub durable_audit: Option<Arc<postgres::PostgresAuthAudit>>,
}

/// Build the router state used by the production API process.  The
/// confidential OIDC configuration is read from the environment and the
/// mounted client-secret file; no in-memory session fallback is available in
/// this path.
pub fn production_router_state_from_env(
    app_pool: PgPool,
    admin_pool: PgPool,
    audit_pool: PgPool,
    step_up_max_auth_age_secs: i64,
) -> Result<RouterState, ProductionAuthBuildError> {
    let config = config::ProductionAuthConfig::from_env()?;
    production_router_state(
        config,
        app_pool,
        admin_pool,
        audit_pool,
        step_up_max_auth_age_secs,
    )
}

/// Assemble a production auth authority from explicit settings.  This
/// constructor is also useful to credential-free tests that inject a local
/// simulator endpoint and a temporary client-secret file.
pub fn production_router_state(
    config: config::ProductionAuthConfig,
    app_pool: PgPool,
    admin_pool: PgPool,
    audit_pool: PgPool,
    step_up_max_auth_age_secs: i64,
) -> Result<RouterState, ProductionAuthBuildError> {
    let transaction_cookie_key = Arc::new(config.transaction_cookie_key());
    let transport = HttpOidcTransport::new(
        &config.provider.token_url,
        &config.provider.jwks_url,
        config.client_secret,
    )?;
    let audit = Arc::new(postgres::PostgresAuthAudit::new(audit_pool));
    let clock = Arc::new(SystemClock);
    let invites = InviteService::new(
        Arc::new(postgres::PostgresInviteStore::new(
            app_pool.clone(),
            admin_pool.clone(),
        )),
        Arc::new(postgres::PostgresUserStore::new(
            app_pool.clone(),
            admin_pool.clone(),
        )),
        clock.clone(),
        audit.clone(),
    );
    let sessions = SessionService::new(
        Arc::new(postgres::PostgresSessionStore::new(app_pool, admin_pool)),
        clock,
        audit.clone(),
    );
    let auth = AuthService::new(
        OidcClient {
            config: config.provider,
            transport: Arc::new(transport),
        },
        invites,
        sessions,
        audit.clone(),
    );
    Ok(RouterState {
        auth: Arc::new(auth),
        // This is deliberately process-local. A second API replica must not
        // be enabled until this seam is replaced with a shared transactional
        // pending store; the bounded local store still fails closed under
        // load rather than growing without limit.
        pending: Arc::new(InMemoryPendingAuthStore::with_capacity(
            auth::oidc::DEFAULT_PENDING_AUTH_CAPACITY,
        )),
        audit: audit.clone(),
        step_up_max_auth_age_secs,
        transaction_cookie_key,
        durable_audit: Some(audit.clone()),
    })
}

#[derive(Debug, thiserror::Error)]
pub enum ProductionAuthBuildError {
    #[error(transparent)]
    Config(#[from] config::ProductionAuthConfigError),
    #[error(transparent)]
    Transport(#[from] HttpOidcTransportConfigError),
}

pub fn router(state: RouterState) -> Router {
    Router::new()
        .route("/auth/login", get(login))
        .route("/auth/callback", get(callback))
        .route("/auth/logout", post(logout))
        .route("/auth/session", get(session_info))
        .route("/auth/csrf", get(csrf_token))
        .route("/auth/invites", post(create_invite))
        .route("/auth/step-up-check", get(step_up_check))
        .with_state(state)
}

#[derive(Debug, serde::Serialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Debug, serde::Serialize)]
struct ErrorDetail {
    code: &'static str,
    message: String,
}

fn error_response(status: StatusCode, code: &'static str, message: impl Into<String>) -> Response {
    (
        status,
        Json(ErrorBody {
            error: ErrorDetail {
                code,
                message: message.into(),
            },
        }),
    )
        .into_response()
}

fn bad_request(code: &'static str, message: impl Into<String>) -> Response {
    error_response(StatusCode::BAD_REQUEST, code, message)
}

fn forbidden(code: &'static str, message: impl Into<String>) -> Response {
    error_response(StatusCode::FORBIDDEN, code, message)
}

fn unauthorized(code: &'static str, message: impl Into<String>) -> Response {
    error_response(StatusCode::UNAUTHORIZED, code, message)
}

fn internal(message: impl Into<String>) -> Response {
    error_response(StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL", message)
}

fn transaction_cookie_mac(key: &[u8; 32], identifier: &str, state: &str) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts a 32-byte key");
    mac.update(identifier.as_bytes());
    mac.update(&[0]);
    mac.update(state.as_bytes());
    mac.finalize().into_bytes().into()
}

fn issue_transaction_cookie(key: &[u8; 32], state: &str) -> String {
    let identifier = auth::oidc::pkce::random_hex();
    let digest = hex::encode(transaction_cookie_mac(key, &identifier, state));
    format!("{identifier}.{digest}")
}

fn transaction_cookie_valid(key: &[u8; 32], cookie_value: &str, state: &str) -> bool {
    let Some((identifier, digest)) = cookie_value.split_once('.') else {
        return false;
    };
    if identifier.len() != 64
        || digest.len() != 64
        || !identifier.bytes().all(|byte| byte.is_ascii_hexdigit())
        || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return false;
    }
    let Ok(presented) = hex::decode(digest) else {
        return false;
    };
    let expected = transaction_cookie_mac(key, identifier, state);
    presented.as_slice().ct_eq(expected.as_slice()).into()
}

fn transaction_cookie_header(key: &[u8; 32], state: &str) -> String {
    format!(
        "{OIDC_TRANSACTION_COOKIE}={}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age={OIDC_TRANSACTION_MAX_AGE}",
        issue_transaction_cookie(key, state)
    )
}

fn clear_transaction_cookie_header() -> HeaderValue {
    HeaderValue::from_static(
        "__Host-lagrange_oidc_tx=; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT",
    )
}

fn clear_transaction_cookie(mut response: Response) -> Response {
    response
        .headers_mut()
        .append(header::SET_COOKIE, clear_transaction_cookie_header());
    response
}

async fn session_from_cookie(
    state: &RouterState,
    headers: &HeaderMap,
) -> Result<SessionInfo, Response> {
    let header_value = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| unauthorized("SESSION_UNKNOWN", "session required"))?;
    let cookie_value = cookie::parse(header_value, cookie::NAME)
        .ok_or_else(|| unauthorized("SESSION_UNKNOWN", "session cookie missing"))?;
    match state.auth.session_info(&cookie_value).await {
        Ok(info) => Ok(info),
        Err(e) => Err(match e {
            AuthError::Session(auth::sessions::SessionError::Expired) => {
                unauthorized("SESSION_EXPIRED", "session expired - re-login required")
            }
            _ => unauthorized("SESSION_UNKNOWN", "no session"),
        }),
    }
}

async fn csrf_guard(
    state: &RouterState,
    headers: &HeaderMap,
    session: &SessionInfo,
) -> Result<(), Response> {
    let presented = headers
        .get("x-csrf-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if !auth::csrf::verify(&session.csrf_token_hash, presented) {
        if state
            .audit
            .record_durable(AuthAuditEvent {
                at_secs: state.auth.sessions.clock.now_epoch_secs(),
                kind: AuthAuditKind::CsrfDenied,
                user: Some(session.user_id.0.clone()),
                reason: Some("CSRF_DENIED".to_string()),
                detail: "synchronizer token missing or wrong".to_string(),
            })
            .await
            .is_err()
        {
            return Err(internal("audit delivery unavailable"));
        }
        return Err(forbidden("CSRF_DENIED", "missing or invalid CSRF token"));
    }
    Ok(())
}

async fn login(State(state): State<RouterState>) -> Response {
    let request = match state.auth.begin_login() {
        Ok(r) => r,
        Err(e) => return internal(format!("login setup failed: {e}")),
    };
    let pending = PendingAuth {
        state: request.state.clone(),
        nonce: request.nonce.clone(),
        code_verifier: request.pkce.verifier.clone(),
        created_at_secs: state.auth.sessions.clock.now_epoch_secs(),
        ttl_secs: DEFAULT_PENDING_TTL_SECS,
    };
    if let Err(e) = state.pending.insert(request.state.clone(), pending).await {
        return internal(format!("pending store failed: {e}"));
    }
    let mut response = (
        StatusCode::SEE_OTHER,
        [
            ("Location", request.url.to_string()),
            (
                "Set-Cookie",
                transaction_cookie_header(&state.transaction_cookie_key, &request.state),
            ),
        ],
    )
        .into_response();
    response
        .headers_mut()
        .insert("Cache-Control", HeaderValue::from_static("no-store"));
    response
}

#[derive(Deserialize)]
struct CallbackParams {
    code: Option<String>,
    state: Option<String>,
}

async fn callback(
    State(state): State<RouterState>,
    Query(params): Query<CallbackParams>,
    headers: HeaderMap,
) -> Response {
    let (Some(code), Some(state_value)) = (params.code.as_deref(), params.state.as_deref()) else {
        return clear_transaction_cookie(bad_request(
            "MISSING_STATE",
            "code and state query parameters are required",
        ));
    };
    let transaction_cookie = headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| cookie::parse(value, OIDC_TRANSACTION_COOKIE));
    let Some(transaction_cookie) = transaction_cookie else {
        let audit_ok = state
            .audit
            .record_durable(AuthAuditEvent {
                at_secs: state.auth.sessions.clock.now_epoch_secs(),
                kind: AuthAuditKind::LoginDenied,
                user: None,
                reason: Some("OIDC_TRANSACTION_MISSING".to_string()),
                detail: "callback did not present the initiating browser transaction cookie"
                    .to_string(),
            })
            .await;
        if audit_ok.is_err() {
            return clear_transaction_cookie(internal("audit delivery unavailable"));
        }
        return clear_transaction_cookie(forbidden(
            "OIDC_TRANSACTION_MISSING",
            "login transaction is missing or invalid",
        ));
    };
    if !transaction_cookie_valid(
        &state.transaction_cookie_key,
        &transaction_cookie,
        state_value,
    ) {
        let audit_ok = state
            .audit
            .record_durable(AuthAuditEvent {
                at_secs: state.auth.sessions.clock.now_epoch_secs(),
                kind: AuthAuditKind::LoginDenied,
                user: None,
                reason: Some("OIDC_TRANSACTION_MISMATCH".to_string()),
                detail: "callback transaction did not match the initiating browser".to_string(),
            })
            .await;
        if audit_ok.is_err() {
            return clear_transaction_cookie(internal("audit delivery unavailable"));
        }
        return clear_transaction_cookie(forbidden(
            "OIDC_TRANSACTION_MISMATCH",
            "login transaction is missing or invalid",
        ));
    };
    match state
        .auth
        .complete_login(code, state_value, &*state.pending)
        .await
    {
        Ok(issued) => {
            let mut response = (
                StatusCode::SEE_OTHER,
                [
                    ("Location", "/"),
                    ("Set-Cookie", issued.set_cookie_header.as_str()),
                ],
            )
                .into_response();
            response.headers_mut().insert(
                "X-CSRF-Token",
                HeaderValue::from_str(&issued.csrf_token)
                    .unwrap_or_else(|_| HeaderValue::from_static("")),
            );
            response
                .headers_mut()
                .insert("Cache-Control", HeaderValue::from_static("no-store"));
            clear_transaction_cookie(response)
        }
        Err(e) => {
            let code = e.code();
            let generic = match e {
                AuthError::Invite(
                    InviteError::InviteNotFound
                    | InviteError::InviteExpired
                    | InviteError::AlreadyRedeemed
                    | InviteError::EmailNotVerified
                    | InviteError::EmailRequired
                    | InviteError::RoleUnknown,
                ) => "login denied: access is by invitation only",
                AuthError::Oidc(OidcError::StateMismatch) => {
                    "login denied: invalid or replayed login state"
                }
                _ => "login denied",
            };
            clear_transaction_cookie(forbidden(code, generic))
        }
    }
}

async fn logout(State(state): State<RouterState>, headers: HeaderMap) -> Response {
    let session = match session_from_cookie(&state, &headers).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    if let Err(r) = csrf_guard(&state, &headers, &session).await {
        return r;
    }
    let cookie_value = cookie::parse(
        headers
            .get(header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default(),
        cookie::NAME,
    )
    .unwrap_or_default();
    if state.auth.logout(&cookie_value).await.is_err() {
        // Keep the bearer cookie when durable revocation failed. Clearing it
        // would make the browser appear logged out while the server may still
        // accept the session; the caller must retry after the 5xx response.
        return internal("logout could not be durably completed");
    }
    (
        StatusCode::NO_CONTENT,
        [("Set-Cookie", cookie::clear_cookie())],
    )
        .into_response()
}

#[derive(serde::Serialize)]
struct SessionBody {
    user_id: String,
    role: &'static str,
    expires_at_secs: i64,
    auth_time_secs: i64,
}

async fn session_info(State(state): State<RouterState>, headers: HeaderMap) -> Response {
    match session_from_cookie(&state, &headers).await {
        Ok(session) => {
            let body = SessionBody {
                user_id: session.user_id.0.clone(),
                role: match session.role {
                    Role::Owner => "owner",
                    Role::Member => "member",
                },
                expires_at_secs: session.expires_at_secs,
                auth_time_secs: session.auth_time_secs,
            };
            let mut response = (StatusCode::OK, Json(body)).into_response();
            response
                .headers_mut()
                .insert("Cache-Control", HeaderValue::from_static("no-store"));
            response
        }
        Err(r) => r,
    }
}

async fn csrf_token(State(state): State<RouterState>, headers: HeaderMap) -> Response {
    let session = match session_from_cookie(&state, &headers).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let _ = &session;
    let cookie_value = cookie::parse(
        headers
            .get(header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default(),
        cookie::NAME,
    )
    .unwrap_or_default();
    match state.auth.rotate_csrf(&cookie_value).await {
        Ok(token) => (
            StatusCode::OK,
            Json(serde_json::json!({ "csrf_token": token })),
        )
            .into_response(),
        Err(_) => unauthorized("SESSION_UNKNOWN", "no session"),
    }
}

#[derive(Deserialize)]
struct InviteBody {
    email: String,
    role: String,
}

async fn create_invite(
    State(state): State<RouterState>,
    headers: HeaderMap,
    Json(body): Json<InviteBody>,
) -> Response {
    let session = match session_from_cookie(&state, &headers).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    if let Err(r) = csrf_guard(&state, &headers, &session).await {
        return r;
    }
    if session.role != Role::Owner {
        return forbidden("INVITE_NOT_OWNER", "only the Owner may invite users");
    }
    let role = match body.role.as_str() {
        "owner" => Role::Owner,
        "member" => Role::Member,
        _ => return bad_request("INVITE_INVALID_ROLE", "role must be owner or member"),
    };
    let result = postgres::with_actor_user_id(
        &session.user_id,
        state.auth.invites.create_invite_as(
            &body.email,
            role,
            auth::invites::DEFAULT_INVITE_TTL_SECS,
            Some(&session.user_id),
        ),
    )
    .await;
    match result {
        Ok(invite) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "id": invite.id,
                "email": invite.email,
                "role": match invite.role { Role::Owner => "owner", Role::Member => "member" },
                "expires_at_secs": invite.expires_at_secs,
            })),
        )
            .into_response(),
        Err(InviteError::InvalidEmail(_)) => {
            bad_request("INVITE_INVALID_EMAIL", "email address is invalid")
        }
        Err(e) => internal(format!("invite store failed: {e}")),
    }
}

async fn step_up_check(State(state): State<RouterState>, headers: HeaderMap) -> Response {
    let session = match session_from_cookie(&state, &headers).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let now = state.auth.sessions.clock.now_epoch_secs();
    match require_owner_step_up(&session, now, state.step_up_max_auth_age_secs) {
        Ok(()) => {
            if state
                .audit
                .record_durable(AuthAuditEvent {
                    at_secs: now,
                    kind: AuthAuditKind::StepUpAllowed,
                    user: Some(session.user_id.0.clone()),
                    reason: None,
                    detail: "owner step-up allowed".to_string(),
                })
                .await
                .is_err()
            {
                return internal("audit delivery unavailable");
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({ "step_up": "allowed" })),
            )
                .into_response()
        }
        Err(denial) => {
            let audit_ok = state
                .audit
                .record_durable(AuthAuditEvent {
                    at_secs: now,
                    kind: AuthAuditKind::StepUpDenied,
                    user: Some(session.user_id.0.clone()),
                    reason: Some(denial.code().to_string()),
                    detail: denial.to_string(),
                })
                .await;
            if audit_ok.is_err() {
                return internal("audit delivery unavailable");
            }
            forbidden(denial.code(), denial.to_string())
        }
    }
}

/// Production OIDC transport: token exchange + JWKS fetch over HTTPS.
/// PKCE S256 protects the authorization code; the client secret authenticates
/// the confidential server-side application.
pub struct HttpOidcTransport {
    token_url: url::Url,
    jwks_url: url::Url,
    client_secret: config::ClientSecret,
    client: reqwest::Client,
}

#[derive(Debug, thiserror::Error)]
pub enum HttpOidcTransportConfigError {
    #[error("invalid OIDC {endpoint} URL: {source}")]
    InvalidUrl {
        endpoint: &'static str,
        #[source]
        source: url::ParseError,
    },
    #[error("OIDC {endpoint} URL must use HTTPS unless its host is explicit loopback")]
    InsecureUrl { endpoint: &'static str },
    #[error("cannot build OIDC HTTP client: {0}")]
    Client(#[source] reqwest::Error),
}

impl HttpOidcTransport {
    pub fn new(
        token_url: impl AsRef<str>,
        jwks_url: impl AsRef<str>,
        client_secret: config::ClientSecret,
    ) -> Result<Self, HttpOidcTransportConfigError> {
        Self::with_timeouts(
            token_url,
            jwks_url,
            client_secret,
            OIDC_CONNECT_TIMEOUT,
            OIDC_REQUEST_TIMEOUT,
        )
    }

    /// Construct a transport with explicit limits.  Production uses `new`
    /// and the conservative constants above; the override keeps timeout tests
    /// fast and lets callers choose a stricter deployment-specific budget.
    pub fn with_timeouts(
        token_url: impl AsRef<str>,
        jwks_url: impl AsRef<str>,
        client_secret: config::ClientSecret,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, HttpOidcTransportConfigError> {
        let token_url = secure_endpoint_url(token_url.as_ref(), "token endpoint")?;
        let jwks_url = secure_endpoint_url(jwks_url.as_ref(), "JWKS endpoint")?;
        Ok(Self {
            token_url,
            jwks_url,
            client_secret,
            client: reqwest::Client::builder()
                .user_agent("lagrange-station-api-server")
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(connect_timeout)
                .timeout(request_timeout)
                .build()
                .map_err(HttpOidcTransportConfigError::Client)?,
        })
    }
}

fn secure_endpoint_url(
    value: &str,
    endpoint: &'static str,
) -> Result<url::Url, HttpOidcTransportConfigError> {
    let url = url::Url::parse(value)
        .map_err(|source| HttpOidcTransportConfigError::InvalidUrl { endpoint, source })?;
    let explicit_loopback = match url.host() {
        Some(url::Host::Ipv4(address)) => address == std::net::Ipv4Addr::LOCALHOST,
        Some(url::Host::Ipv6(address)) => address == std::net::Ipv6Addr::LOCALHOST,
        Some(url::Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
        None => false,
    };
    if url.scheme() != "https" && !(url.scheme() == "http" && explicit_loopback) {
        return Err(HttpOidcTransportConfigError::InsecureUrl { endpoint });
    }
    Ok(url)
}

#[async_trait::async_trait]
impl auth::oidc::OidcTransport for HttpOidcTransport {
    async fn exchange_code(
        &self,
        request: &auth::oidc::TokenRequest,
    ) -> Result<auth::oidc::TokenResponse, TransportError> {
        let body = self
            .client
            .post(self.token_url.clone())
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", &request.code),
                ("redirect_uri", &request.redirect_uri),
                ("client_id", &request.client_id),
                ("code_verifier", &request.code_verifier),
                ("client_secret", self.client_secret.expose()),
            ])
            .send()
            .await
            .map_err(|e| TransportError(format!("token exchange: {e}")))?;
        let status = body.status();
        if !status.is_success() {
            return Err(TransportError(format!("token exchange http {status}")));
        }
        let text = body
            .text()
            .await
            .map_err(|e| TransportError(format!("token exchange body: {e}")))?;
        auth::oidc::TokenResponse::from_json(&text).map_err(|e| TransportError(e.to_string()))
    }

    async fn fetch_jwks(&self) -> Result<auth::oidc::jwks::Jwks, TransportError> {
        let body = self
            .client
            .get(self.jwks_url.clone())
            .send()
            .await
            .map_err(|e| TransportError(format!("jwks fetch: {e}")))?;
        let status = body.status();
        if !status.is_success() {
            return Err(TransportError(format!("jwks http {status}")));
        }
        let text = body
            .text()
            .await
            .map_err(|e| TransportError(format!("jwks body: {e}")))?;
        auth::oidc::jwks::Jwks::parse(&text).map_err(|e| TransportError(e.to_string()))
    }
}
