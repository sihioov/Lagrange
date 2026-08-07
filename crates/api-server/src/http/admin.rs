//! Auth/session routes under `/api/v1` plus the Owner-only admin surface
//! (datasets, jobs, workers, audit logs) and the Phase 3 Live stubs.

use crate::http::dto::{
    AdminDatasetDto, AdminUserDto, AuditDto, DatasetVerdictDto, IssueDto, JobDto, PageDto,
    SessionDto, WorkerDto,
};
use crate::http::error::{api_error, code_error, request_id};
use crate::http::pagination::Cursor;
use crate::http::session::{PageParams, Session};
use crate::http::state::ApiState;
use crate::http::{JsonBody, audit, idempotent, tenancy_response};
use axum::Json;
use axum::extract::{OriginalUri, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Auth / session
// ---------------------------------------------------------------------------

pub async fn session_info(State(_state): State<ApiState>, session: Session) -> Response {
    (
        StatusCode::OK,
        Json(SessionDto {
            user_id: session.0.user_id.0.clone(),
            role: if session.actor().is_owner() {
                "owner"
            } else {
                "member"
            },
            expires_at_secs: session.0.expires_at_secs,
            auth_time_secs: session.0.auth_time_secs,
        }),
    )
        .into_response()
}

pub async fn logout(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    JsonBody(_body): JsonBody<crate::http::dto::EmptyBody>,
) -> Response {
    if let Err(r) = crate::http::session::require_csrf(&headers, &session.0) {
        return r;
    }
    let actor = session.actor();
    let cookie_value = headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|c| auth::sessions::cookie::parse(c, auth::sessions::cookie::NAME))
        .unwrap_or_default();
    let token_hash = auth::sessions::cookie::hash(&cookie_value);
    // Revoke under the actor GUC (web_sessions is FORCE RLS on user_id).
    if let Ok(mut tx) = crate::actor_tx::begin_actor_tx(&state.app_pool, &actor).await {
        let _ = sqlx::query(
            "UPDATE web_sessions SET revoked_at = now() \
             WHERE session_hash = $1 AND user_id = $2 AND revoked_at IS NULL",
        )
        .bind(&token_hash)
        .bind(crate::actor_tx::actor_uuid(&actor).unwrap_or_default())
        .execute(&mut *tx)
        .await;
        let _ = tx.commit().await;
    }
    audit(
        &state,
        &session,
        &headers,
        "auth.logout",
        "web_session",
        &token_hash.chars().take(12).collect::<String>(),
        None,
        None,
        Some("session revoked by user".to_string()),
    )
    .await;
    (
        StatusCode::NO_CONTENT,
        [("Set-Cookie", auth::sessions::cookie::clear_cookie())],
    )
        .into_response()
}

pub async fn csrf_token(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
) -> Response {
    let rid = request_id(&headers);
    let actor = session.actor();
    let cookie_value = headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|c| auth::sessions::cookie::parse(c, auth::sessions::cookie::NAME))
        .unwrap_or_default();
    let token_hash = auth::sessions::cookie::hash(&cookie_value);
    let token = auth::csrf::generate_token();
    let new_hash = auth::csrf::hash_token(&token);
    let updated = sqlx::query(
        "UPDATE web_sessions SET csrf_hash = $1 \
         WHERE session_hash = $2 AND user_id = $3 AND revoked_at IS NULL",
    )
    .bind(&new_hash)
    .bind(&token_hash)
    .bind(crate::actor_tx::actor_uuid(&actor).unwrap_or_default())
    .execute(&state.app_pool)
    .await;
    if let Err(e) = updated {
        return api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL",
            format!("csrf rotation failed: {e}"),
            &rid,
            None,
        );
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({ "csrf_token": token })),
    )
        .into_response()
}

pub async fn step_up_check(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
) -> Response {
    let rid = request_id(&headers);
    let now = chrono::Utc::now().timestamp();
    match auth::stepup::require_owner_step_up(&session.0, now, state.cfg.step_up_max_auth_age_secs)
    {
        Ok(()) => {
            audit(
                &state,
                &session,
                &headers,
                "auth.step_up",
                "owner_action",
                &session.0.user_id.0,
                None,
                None,
                Some("step-up allowed".to_string()),
            )
            .await;
            (
                StatusCode::OK,
                Json(serde_json::json!({ "step_up": "allowed" })),
            )
                .into_response()
        }
        Err(denial) => {
            audit(
                &state,
                &session,
                &headers,
                "auth.step_up",
                "owner_action",
                &session.0.user_id.0,
                None,
                None,
                Some(format!("denied: {}", denial.code())),
            )
            .await;
            code_error(denial.code(), denial.to_string(), &rid)
        }
    }
}

// ---------------------------------------------------------------------------
// Admin (Owner-only, audited)
// ---------------------------------------------------------------------------

pub async fn list_datasets(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
) -> Response {
    let rid = request_id(&headers);
    match state.ops().list_datasets(&session.actor(), &rid).await {
        Ok(rows) => {
            let items: Vec<AdminDatasetDto> = rows
                .into_iter()
                .map(|(r, issues)| AdminDatasetDto {
                    id: r.id.to_string(),
                    dataset_id: r.dataset_id,
                    version: r.version,
                    status: r.status,
                    manifest_sha256: r.manifest_sha256,
                    created_at: r.created_at,
                    blocking_issues: issues
                        .into_iter()
                        .map(|i| IssueDto {
                            issue_code: i.issue_code,
                            severity: i.severity,
                            detail: i.detail_json,
                        })
                        .collect(),
                })
                .collect();
            (StatusCode::OK, Json(PageDto::new(items, None))).into_response()
        }
        Err(e) => tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    }
}

async fn dataset_verdict_route(
    state: ApiState,
    session: Session,
    headers: HeaderMap,
    dataset_id: String,
    action: &'static str,
) -> Response {
    let rid = request_id(&headers);
    if let Err(r) = crate::http::session::require_csrf(&headers, &session.0) {
        return r;
    }
    let id = match Uuid::parse_str(&dataset_id) {
        Ok(i) => i,
        Err(_) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "INVALID_PARAMETER",
                "dataset id must be a uuid",
                &rid,
                None,
            );
        }
    };
    let body_hash = crate::http::idempotency::body_hash(&serde_json::json!({}));
    idempotent(&state, &session, &headers, &body_hash, async {
        match state
            .ops()
            .dataset_verdict(&session.actor(), id, action, &rid)
            .await
        {
            Ok(v) => (
                StatusCode::OK,
                Json(DatasetVerdictDto {
                    dataset_id: v.dataset_id,
                    version: v.version,
                    status: v.status,
                    verdict: v.verdict,
                    reason: v.reason,
                }),
            )
                .into_response(),
            Err(e) => tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
        }
    })
    .await
}

pub async fn approve_dataset(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(dataset_id): Path<String>,
    JsonBody(_body): JsonBody<crate::http::dto::EmptyBody>,
) -> Response {
    dataset_verdict_route(state, session, headers, dataset_id, "approve").await
}

pub async fn block_dataset(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(dataset_id): Path<String>,
    JsonBody(_body): JsonBody<crate::http::dto::EmptyBody>,
) -> Response {
    dataset_verdict_route(state, session, headers, dataset_id, "block").await
}

pub async fn list_jobs(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Query(params): Query<PageParams>,
) -> Response {
    let rid = request_id(&headers);
    let cursor = match decode_cursor(&state, &rid, params.cursor.as_deref()) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let limit = params.limit_or(PageParams::DEFAULT_LIMIT);
    match state
        .ops()
        .list_jobs(&session.actor(), cursor.as_ref(), limit, &rid)
        .await
    {
        Ok((rows, next)) => {
            let next = next.map(|c| c.encode(&state.cfg.cursor_secret));
            let items = rows.into_iter().map(job_dto).collect();
            (StatusCode::OK, Json(PageDto::new(items, next))).into_response()
        }
        Err(e) => tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    }
}

pub async fn retry_job(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(job_id): Path<String>,
    JsonBody(_body): JsonBody<crate::http::dto::EmptyBody>,
) -> Response {
    let rid = request_id(&headers);
    if let Err(r) = crate::http::session::require_csrf(&headers, &session.0) {
        return r;
    }
    let id = match Uuid::parse_str(&job_id) {
        Ok(i) => i,
        Err(_) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "INVALID_PARAMETER",
                "job id must be a uuid",
                &rid,
                None,
            );
        }
    };
    let body_hash = crate::http::idempotency::body_hash(&serde_json::json!({}));
    idempotent(&state, &session, &headers, &body_hash, async {
        match state.ops().retry_job(&session.actor(), id, &rid).await {
            Ok(job) => (StatusCode::OK, Json(job_dto(job))).into_response(),
            Err(e) => tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
        }
    })
    .await
}

pub async fn list_workers(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
) -> Response {
    let rid = request_id(&headers);
    match state.ops().list_workers(&session.actor(), &rid).await {
        Ok(rows) => {
            let items: Vec<WorkerDto> = rows
                .into_iter()
                .map(|w| WorkerDto {
                    worker_id: w.worker_id,
                    last_heartbeat_at: w.last_heartbeat_at,
                    active_job_count: w.active_job_count,
                })
                .collect();
            (StatusCode::OK, Json(PageDto::new(items, None))).into_response()
        }
        Err(e) => tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    }
}

pub async fn list_audit_logs(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Query(params): Query<PageParams>,
) -> Response {
    let rid = request_id(&headers);
    let cursor = match decode_cursor(&state, &rid, params.cursor.as_deref()) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let limit = params.limit_or(PageParams::DEFAULT_LIMIT);
    match state
        .ops()
        .list_audit_logs(&session.actor(), cursor.as_ref(), limit, &rid)
        .await
    {
        Ok((rows, next)) => {
            let next = next.map(|c| c.encode(&state.cfg.cursor_secret));
            let items: Vec<AuditDto> = rows
                .into_iter()
                .map(|a| AuditDto {
                    id: a.id.to_string(),
                    action: a.action,
                    actor_role: a.actor_role,
                    actor_user_id: a.actor_user_id.map(|u| u.to_string()),
                    target_type: a.target_type,
                    target_id: a.target_id,
                    reason: a.reason,
                    correlation_id: a.correlation_id,
                    created_at: a.created_at,
                })
                .collect();
            (StatusCode::OK, Json(PageDto::new(items, next))).into_response()
        }
        Err(e) => tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    }
}

pub async fn list_users(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
) -> Response {
    let rid = request_id(&headers);
    match state.ops().list_users(&session.actor(), &rid).await {
        Ok(rows) => {
            let items: Vec<AdminUserDto> = rows
                .into_iter()
                .map(|u| AdminUserDto {
                    id: u.id.to_string(),
                    email: u.email,
                    roles: u.roles,
                    created_at: u.created_at,
                })
                .collect();
            (StatusCode::OK, Json(PageDto::new(items, None))).into_response()
        }
        Err(e) => tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    }
}

// ---------------------------------------------------------------------------
// Phase 3 Live (Owner-only; NOT_IMPLEMENTED until Todo 36+)
// ---------------------------------------------------------------------------

pub async fn live_not_available(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    uri: OriginalUri,
    JsonBody(_body): JsonBody<serde_json::Value>,
) -> Response {
    let rid = request_id(&headers);
    let route = uri.path().to_string();
    let live_action = route
        .strip_prefix("/api/v1/admin/live/")
        .unwrap_or(route.trim_start_matches("/api/v1"))
        .replace('/', ".");
    let action = format!("admin.live.{live_action}");
    if !session.actor().is_owner() {
        audit(
            &state,
            &session,
            &headers,
            &action,
            "live",
            &route,
            None,
            None,
            Some("FORBIDDEN_MEMBER: Phase 3 Live is Owner-only".to_string()),
        )
        .await;
        return code_error("FORBIDDEN", "Phase 3 Live routes are Owner-only", &rid);
    }
    audit(
        &state,
        &session,
        &headers,
        &action,
        "live",
        &route,
        None,
        None,
        Some("NOT_IMPLEMENTED: Phase 3".to_string()),
    )
    .await;
    api_error(
        StatusCode::NOT_IMPLEMENTED,
        "NOT_IMPLEMENTED",
        "Phase 3 Live is not available in this release",
        &rid,
        Some(serde_json::json!({ "phase": "3" })),
    )
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn job_dto(j: crate::repos::ops::AdminJobRow) -> JobDto {
    JobDto {
        id: j.id.to_string(),
        job_type: j.job_type,
        status: j.status,
        priority: j.priority,
        idempotency_key: j.idempotency_key,
        attempt_count: j.attempt_count,
        created_at: j.created_at,
        started_at: j.started_at,
        finished_at: j.finished_at,
        error_code: j.error_code,
        error_message: j.error_message,
    }
}

#[allow(clippy::result_large_err)]
fn decode_cursor(
    state: &ApiState,
    rid: &str,
    raw: Option<&str>,
) -> Result<Option<Cursor>, Response> {
    match raw {
        None => Ok(None),
        Some(r) => match Cursor::decode(r, &state.cfg.cursor_secret) {
            Ok(c) => Ok(Some(c)),
            Err(_) => Err(code_error(
                "INVALID_CURSOR",
                "pagination cursor is invalid",
                rid,
            )),
        },
    }
}
