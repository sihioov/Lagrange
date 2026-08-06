//! Session extraction for `/api/v1`: the opaque `__Host-lagrange_session`
//! cookie -> sha256 -> `web_sessions` lookup. `web_sessions` is FORCE RLS on
//! `user_id` with the actor GUC, but the actor is unknown until the session
//! resolves, so the lookup runs over the dedicated read-only `admin` role
//! (its `USING (true)` SELECT policy exists exactly for this), and all
//! subsequent row access uses the `app` role with the actor GUC.
//!
//! Fail-closed: unknown hash, revoked, or expired session => 401
//! `SESSION_UNKNOWN` / `SESSION_EXPIRED` (T22 codes).

use crate::http::error::{api_error, request_id};
use crate::http::state::SessionBackend;
use auth::entitlement::Role;
use auth::sessions::SessionInfo;
use auth::sessions::cookie;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use serde::Deserialize;
use uuid::Uuid;

/// An authenticated session, extracted from the cookie header. Implemented
/// as a `FromRequestParts` extractor (it only reads headers), so handlers
/// may combine it with the body-consuming [`crate::http::JsonBody`].
#[derive(Debug, Clone)]
pub struct Session(pub SessionInfo);

impl Session {
    pub fn actor(&self) -> auth::entitlement::Actor {
        self.0.actor()
    }
}

impl<S: SessionBackend> FromRequestParts<S> for Session {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let headers = &parts.headers;
        let rid = request_id(headers);
        let cookie_header = headers
            .get(header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                api_error(
                    StatusCode::UNAUTHORIZED,
                    "SESSION_UNKNOWN",
                    "session required",
                    &rid,
                    None,
                )
            })?;
        let value = cookie::parse(cookie_header, cookie::NAME).ok_or_else(|| {
            api_error(
                StatusCode::UNAUTHORIZED,
                "SESSION_UNKNOWN",
                "session cookie missing",
                &rid,
                None,
            )
        })?;
        let info = resolve_session(state, &value)
            .await
            .map_err(|e| api_error(e.status(), e.code(), e.message(), &rid, None))?;
        Ok(Session(info))
    }
}

#[derive(Debug)]
pub(crate) struct SessionRejection {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
}

impl SessionRejection {
    fn status(&self) -> StatusCode {
        self.status
    }
    fn code(&self) -> &'static str {
        self.code
    }
    fn message(&self) -> &'static str {
        self.message
    }
}

/// Resolve a session cookie value to [`SessionInfo`] via the admin pool.
///
/// `web_sessions` has no `auth_time`/`amr` columns, so the login instant
/// (`created_at`) is the best available authentication timestamp and `amr` is
/// empty — the Owner step-up gate therefore fails closed (no MFA claim).
pub(crate) async fn resolve_session(
    backend: &impl SessionBackend,
    cookie_value: &str,
) -> Result<SessionInfo, SessionRejection> {
    let token_hash = cookie::hash(cookie_value);
    let row: Option<(Uuid, String, String, i64, i64)> = sqlx::query_as(
        "SELECT s.user_id, r.id, s.csrf_hash, \
                EXTRACT(EPOCH FROM s.expires_at)::bigint, \
                EXTRACT(EPOCH FROM s.created_at)::bigint \
         FROM web_sessions s \
         JOIN users u ON u.id = s.user_id \
         JOIN user_roles ur ON ur.user_id = u.id \
         JOIN roles r ON r.id = ur.role_id \
         WHERE s.session_hash = $1 AND s.revoked_at IS NULL AND s.expires_at > now()",
    )
    .bind(&token_hash)
    .fetch_optional(backend.admin_pool())
    .await
    .map_err(|_| SessionRejection {
        status: StatusCode::UNAUTHORIZED,
        code: "SESSION_UNKNOWN",
        message: "no session",
    })?;
    let Some((user_id, role_id, csrf_hash, expires_at_secs, auth_time_secs)) = row else {
        return Err(SessionRejection {
            status: StatusCode::UNAUTHORIZED,
            code: "SESSION_UNKNOWN",
            message: "no session",
        });
    };
    let role = match role_id.as_str() {
        "owner" => Role::Owner,
        "member" => Role::Member,
        _ => {
            return Err(SessionRejection {
                status: StatusCode::UNAUTHORIZED,
                code: "SESSION_UNKNOWN",
                message: "unknown role",
            });
        }
    };
    Ok(SessionInfo {
        user_id: auth::entitlement::UserId::new(user_id.to_string()),
        role,
        auth_time_secs,
        amr: Vec::new(),
        expires_at_secs,
        csrf_token_hash: csrf_hash,
    })
}

/// CSRF synchronizer-token guard for mutating routes (T22 pattern).
#[allow(clippy::result_large_err)]
pub fn require_csrf(headers: &HeaderMap, session: &SessionInfo) -> Result<(), Response> {
    let rid = request_id(headers);
    let presented = headers
        .get("x-csrf-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if auth::csrf::verify(&session.csrf_token_hash, presented) {
        Ok(())
    } else {
        Err(api_error(
            StatusCode::FORBIDDEN,
            "CSRF_DENIED",
            "missing or invalid CSRF token",
            &rid,
            None,
        ))
    }
}

/// Path/query parameter struct for cursor pagination.
#[derive(Debug, Deserialize)]
pub struct PageParams {
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

impl PageParams {
    pub const MAX_LIMIT: u32 = 100;
    pub const DEFAULT_LIMIT: u32 = 20;

    /// Normalized limit (capped, never zero).
    pub fn limit_or(&self, fallback: u32) -> usize {
        let l = self.limit.unwrap_or(fallback).clamp(1, Self::MAX_LIMIT);
        l as usize
    }
}
