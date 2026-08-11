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

use auth::audit::{AuthAudit, AuthAuditEvent, AuthAuditKind};
use auth::entitlement::Role;
use auth::invites::InviteError;
use auth::oidc::{
    DEFAULT_PENDING_TTL_SECS, OidcError, PendingAuth, PendingAuthStore, TransportError,
};
use auth::service::{AuthError, AuthService};
use auth::sessions::SessionInfo;
use auth::sessions::cookie;
use auth::stepup::require_owner_step_up;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Clone)]
pub struct RouterState {
    pub auth: Arc<AuthService>,
    pub pending: Arc<dyn PendingAuthStore>,
    pub audit: Arc<dyn AuthAudit>,
    pub step_up_max_auth_age_secs: i64,
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
        state.audit.record(AuthAuditEvent {
            at_secs: state.auth.sessions.clock.now_epoch_secs(),
            kind: AuthAuditKind::CsrfDenied,
            user: Some(session.user_id.0.clone()),
            reason: Some("CSRF_DENIED".to_string()),
            detail: "synchronizer token missing or wrong".to_string(),
        });
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
        [("Location", request.url.to_string())],
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
) -> Response {
    let (Some(code), Some(state_value)) = (params.code.as_deref(), params.state.as_deref()) else {
        return bad_request(
            "MISSING_STATE",
            "code and state query parameters are required",
        );
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
            response
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
            forbidden(code, generic)
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
    let _ = state.auth.logout(&cookie_value).await;
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
    match state
        .auth
        .invites
        .create_invite(&body.email, role, auth::invites::DEFAULT_INVITE_TTL_SECS)
        .await
    {
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
            state.audit.record(AuthAuditEvent {
                at_secs: now,
                kind: AuthAuditKind::StepUpAllowed,
                user: Some(session.user_id.0.clone()),
                reason: None,
                detail: "owner step-up allowed".to_string(),
            });
            (
                StatusCode::OK,
                Json(serde_json::json!({ "step_up": "allowed" })),
            )
                .into_response()
        }
        Err(denial) => {
            state.audit.record(AuthAuditEvent {
                at_secs: now,
                kind: AuthAuditKind::StepUpDenied,
                user: Some(session.user_id.0.clone()),
                reason: Some(denial.code().to_string()),
                detail: denial.to_string(),
            });
            forbidden(denial.code(), denial.to_string())
        }
    }
}

/// Production OIDC transport: token exchange + JWKS fetch over HTTPS.
/// PKCE S256 replaces the client secret (Auth0 confidential-app best
/// practice); `client_secret` is deliberately absent.
pub struct HttpOidcTransport {
    pub token_url: String,
    pub jwks_url: String,
    client: reqwest::Client,
}

impl HttpOidcTransport {
    pub fn new(token_url: impl Into<String>, jwks_url: impl Into<String>) -> Self {
        Self {
            token_url: token_url.into(),
            jwks_url: jwks_url.into(),
            client: reqwest::Client::builder()
                .user_agent("lagrange-station-api-server")
                .build()
                .expect("reqwest client builds"),
        }
    }
}

#[async_trait::async_trait]
impl auth::oidc::OidcTransport for HttpOidcTransport {
    async fn exchange_code(
        &self,
        request: &auth::oidc::TokenRequest,
    ) -> Result<auth::oidc::TokenResponse, TransportError> {
        let body = self
            .client
            .post(&self.token_url)
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", &request.code),
                ("redirect_uri", &request.redirect_uri),
                ("client_id", &request.client_id),
                ("code_verifier", &request.code_verifier),
            ])
            .send()
            .await
            .map_err(|e| TransportError(format!("token exchange: {e}")))?;
        let status = body.status();
        let text = body
            .text()
            .await
            .map_err(|e| TransportError(format!("token exchange body: {e}")))?;
        if !status.is_success() {
            return Err(TransportError(format!(
                "token exchange http {status}: {text}"
            )));
        }
        auth::oidc::TokenResponse::from_json(&text).map_err(|e| TransportError(e.to_string()))
    }

    async fn fetch_jwks(&self) -> Result<auth::oidc::jwks::Jwks, TransportError> {
        let body = self
            .client
            .get(&self.jwks_url)
            .send()
            .await
            .map_err(|e| TransportError(format!("jwks fetch: {e}")))?;
        let status = body.status();
        let text = body
            .text()
            .await
            .map_err(|e| TransportError(format!("jwks body: {e}")))?;
        if !status.is_success() {
            return Err(TransportError(format!("jwks http {status}: {text}")));
        }
        auth::oidc::jwks::Jwks::parse(&text).map_err(|e| TransportError(e.to_string()))
    }
}
