//! The `/api/v1` router: correlation middleware, body limits, and every
//! documented route group. `api_router` is the single assembly point the
//! tests drive through `tower::ServiceExt::oneshot`.

pub mod admin;
pub mod artifacts;
pub mod backtests;
pub mod dto;
pub mod entitlement;
pub mod error;
pub mod idempotency;
pub mod licensing;
pub mod live;
pub mod middleware;
pub mod notifications;
pub mod pagination;
pub mod paper;
pub mod recommendations;
pub mod session;
pub mod state;
pub mod strategies;
pub mod validation;

use crate::contract::CONTRACT_ROUTES;
use crate::http::state::ApiState;
use axum::extract::{FromRequest, Request};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::de::DeserializeOwned;

/// The max JSON body size accepted by `/api/v1` (256 KiB).
pub const MAX_BODY_BYTES: usize = 256 * 1024;

/// A validated JSON body with typed rejections: oversized -> 413
/// `PAYLOAD_TOO_LARGE`, malformed/unknown-field -> 400 `INVALID_PARAMETER`.
pub struct JsonBody<T>(pub T);

impl<T> FromRequest<ApiState> for JsonBody<T>
where
    T: DeserializeOwned,
{
    type Rejection = Response;

    async fn from_request(req: Request, _state: &ApiState) -> Result<Self, Self::Rejection> {
        let rid = error::request_id(req.headers());
        let bytes = match axum::body::to_bytes(req.into_body(), MAX_BODY_BYTES).await {
            Ok(b) => b,
            Err(_) => {
                return Err(error::api_error(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "PAYLOAD_TOO_LARGE",
                    "request body exceeds 256 KiB",
                    &rid,
                    None,
                ));
            }
        };
        match serde_json::from_slice::<T>(&bytes) {
            Ok(v) => Ok(JsonBody(v)),
            Err(e) => Err(error::api_error(
                StatusCode::BAD_REQUEST,
                "INVALID_PARAMETER",
                format!("malformed JSON body: {e}"),
                &rid,
                None,
            )),
        }
    }
}

/// Run a mutating handler under the idempotency contract: the route requires
/// an `Idempotency-Key`, replays return the cached result (same body) or
/// `IDEMPOTENCY_KEY_MISMATCH` (different body), and the first result is
/// cached for replay. `body_hash` is the canonical hash of the request body.
pub async fn idempotent(
    state: &ApiState,
    session: &session::Session,
    headers: &HeaderMap,
    body_hash: &str,
    run: impl std::future::Future<Output = Response>,
) -> Response {
    let rid = error::request_id(headers);
    let key = match idempotency::key_from(headers) {
        Some(k) => k,
        None => {
            return error::code_error(
                "IDEMPOTENCY_KEY_REQUIRED",
                "mutating routes require an Idempotency-Key header",
                &rid,
            );
        }
    };
    let store_key = format!("{}:{key}", session.0.user_id.0);
    if let Some(cached) = state.idempotency.get(&store_key) {
        if cached.body_hash != body_hash {
            return error::code_error(
                "IDEMPOTENCY_KEY_MISMATCH",
                "the same Idempotency-Key was already used with a different request body",
                &rid,
            );
        }
        let mut resp = (cached.status, Json(cached.body)).into_response();
        resp.headers_mut().insert(
            "X-Idempotent-Replay",
            axum::http::HeaderValue::from_static("true"),
        );
        return resp;
    }
    // Single-flight each public actor/key pair. A concurrent retry waits for
    // the first request to commit and populate the cache, then replays that
    // exact result instead of executing a second side effect.
    let gate = state.idempotency.gate(&store_key);
    let _guard = gate.lock().await;
    if let Some(cached) = state.idempotency.get(&store_key) {
        if cached.body_hash != body_hash {
            return error::code_error(
                "IDEMPOTENCY_KEY_MISMATCH",
                "the same Idempotency-Key was already used with a different request body",
                &rid,
            );
        }
        let mut resp = (cached.status, Json(cached.body)).into_response();
        resp.headers_mut().insert(
            "X-Idempotent-Replay",
            axum::http::HeaderValue::from_static("true"),
        );
        return resp;
    }
    let resp = run.await;
    let status = resp.status();
    let body = match axum::body::to_bytes(resp.into_body(), 16 * 1024 * 1024).await {
        Ok(bytes) => {
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap_or(serde_json::Value::Null)
        }
        Err(_) => serde_json::Value::Null,
    };
    // A durable repository may discover that this fresh process received the
    // wrong body for a key whose canonical request already committed.  That
    // mismatch is not the key's result: caching it under the wrong body hash
    // would prevent the canonical body from hydrating/replaying its success.
    let is_non_authoritative_mismatch = status == StatusCode::CONFLICT
        && body.pointer("/error/code").and_then(|code| code.as_str())
            == Some("IDEMPOTENCY_KEY_MISMATCH");
    if !is_non_authoritative_mismatch {
        state.idempotency.insert(
            &store_key,
            idempotency::CachedResult {
                body_hash: body_hash.to_string(),
                status,
                body: body.clone(),
            },
        );
    }
    (status, Json(body)).into_response()
}

/// Audit one mutation through the append-only writer (actor/time/target/
/// before-after/reason/correlation - FR-ADM-002).
#[allow(clippy::too_many_arguments)]
pub async fn audit(
    state: &ApiState,
    session: &session::Session,
    headers: &HeaderMap,
    action: &str,
    target_type: &str,
    target_id: &str,
    before: Option<serde_json::Value>,
    after: Option<serde_json::Value>,
    reason: Option<String>,
) {
    let _ = state
        .audit_writer()
        .record(
            &session.actor(),
            &crate::repos::audit::AuditEntry {
                action: action.to_string(),
                target_type: target_type.to_string(),
                target_id: target_id.to_string(),
                before_json: before,
                after_json: after,
                reason,
                correlation_id: Some(error::request_id(headers)),
            },
        )
        .await;
}

/// Map a tenancy error to the typed envelope (ownership -> 404/403; a
/// foreign resource is indistinguishable from a missing one).
pub fn tenancy_response(
    err: crate::error::TenancyError,
    rid: &str,
    not_found_code: &'static str,
) -> Response {
    match err {
        crate::error::TenancyError::NotFound => {
            error::code_error(not_found_code, "resource not found", rid)
        }
        crate::error::TenancyError::Forbidden => error::code_error("FORBIDDEN", "forbidden", rid),
        crate::error::TenancyError::DatasetBlocked(msg) => error::api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "DATASET_BLOCKED",
            msg,
            rid,
            None,
        ),
        crate::error::TenancyError::InvalidState(msg) => {
            error::api_error(StatusCode::BAD_REQUEST, "INVALID_PARAMETER", msg, rid, None)
        }
        crate::error::TenancyError::ResultIntegrity(msg) => error::api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "RESULT_INTEGRITY_FAILED",
            msg,
            rid,
            None,
        ),
        crate::error::TenancyError::NotImplemented => {
            error::code_error("NOT_IMPLEMENTED", "not implemented", rid)
        }
        crate::error::TenancyError::Database(e) => {
            // Unique violation -> DUPLICATE_RESOURCE (409).
            if let Some(code) = sqlstate_of(&e)
                && code == "23505"
            {
                return error::code_error("DUPLICATE_RESOURCE", "resource already exists", rid);
            }
            crate::observability::log::LogEvent::critical("db.error")
                .correlation(rid)
                .message(format!("database: {e}"))
                .error_code("INTERNAL")
                .emit();
            error::api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                format!("database: {e}"),
                rid,
                None,
            )
        }
    }
}

fn sqlstate_of(e: &sqlx::Error) -> Option<String> {
    match e {
        sqlx::Error::Database(db) => db.code().map(|c| c.to_string()),
        _ => None,
    }
}

/// Serve the Prometheus text exposition format (design §15.2). No session
/// is required: the endpoint is scraped over the internal backend network
/// and carries no PII (labels come from fixed sets only).
pub async fn metrics() -> axum::response::Response {
    let body = crate::observability::metrics::render();
    (
        StatusCode::OK,
        [
            ("Content-Type", "text/plain; version=0.0.4"),
            ("Cache-Control", "no-store"),
        ],
        body,
    )
        .into_response()
}

/// Assemble the versioned router.
pub fn api_router(state: ApiState) -> Router {
    let v1 = Router::new()
        // metrics (Prometheus scrape; no PII, fixed label sets)
        .route("/metrics", get(metrics))
        // auth / session
        .route("/auth/session", get(admin::session_info))
        .route("/auth/logout", post(admin::logout))
        .route("/auth/csrf", get(admin::csrf_token))
        .route("/auth/step-up-check", get(admin::step_up_check))
        // strategies
        .route("/strategies", get(strategies::list))
        .route("/strategies/{strategy_id}", get(strategies::get))
        .route(
            "/strategies/{strategy_id}/configs",
            post(strategies::create_config),
        )
        .route("/strategy-configs", get(strategies::list_configs))
        .route("/strategy-configs/{config_id}", get(strategies::get_config))
        // recommendations
        .route("/recommendations/runs", post(recommendations::create_run))
        .route("/recommendations/runs", get(recommendations::list_runs))
        .route(
            "/recommendations/runs/{run_id}",
            get(recommendations::get_run),
        )
        .route("/recommendations/latest", get(recommendations::latest))
        // backtests
        .route("/backtests", post(backtests::create))
        .route("/backtests", get(backtests::list))
        .route("/backtests/compare", post(backtests::compare))
        .route("/backtests/{run_id}", get(backtests::get))
        .route("/backtests/{run_id}/cancel", post(backtests::cancel))
        .route("/backtests/{run_id}/metrics", get(backtests::metrics))
        .route("/backtests/{run_id}/equity", get(backtests::equity))
        .route("/backtests/{run_id}/trades", get(backtests::trades))
        .route(
            "/backtests/{run_id}/robustness",
            post(backtests::robustness),
        )
        // paper
        .route(
            "/paper/accounts",
            get(paper::list_accounts).post(paper::create_account),
        )
        .route("/paper/accounts/{account_id}", get(paper::get_account))
        .route(
            "/paper/accounts/{account_id}/bind-strategy",
            post(paper::bind_strategy),
        )
        .route(
            "/paper/accounts/{account_id}/recommendation-previews",
            post(paper::create_rebalance_preview),
        )
        .route(
            "/paper/accounts/{account_id}/recommendation-previews/{preview_id}",
            get(paper::get_rebalance_preview),
        )
        .route("/paper/accounts/{account_id}/orders", get(paper::orders))
        .route(
            "/paper/accounts/{account_id}/positions",
            get(paper::positions),
        )
        .route("/paper/accounts/{account_id}/equity", get(paper::equity))
        .route(
            "/paper/accounts/{account_id}/performance",
            get(paper::performance),
        )
        .route("/paper/accounts/{account_id}/lineage", get(paper::lineage))
        .route("/paper/accounts/{account_id}/parity", get(paper::parity))
        // admin
        .route("/admin/datasets", get(admin::list_datasets))
        .route(
            "/admin/datasets/{dataset_id}/approve",
            post(admin::approve_dataset),
        )
        .route(
            "/admin/datasets/{dataset_id}/block",
            post(admin::block_dataset),
        )
        .route("/admin/jobs", get(admin::list_jobs))
        .route("/admin/jobs/{job_id}/retry", post(admin::retry_job))
        .route("/admin/workers", get(admin::list_workers))
        .route("/admin/users", get(admin::list_users))
        .route("/admin/audit-logs", get(admin::list_audit_logs))
        .route(
            "/admin/notifications/deliveries",
            get(notifications::list_deliveries),
        )
        // notifications
        .route("/notifications", get(notifications::list))
        .route(
            "/notifications/subscriptions",
            get(notifications::list_subscriptions),
        )
        .route(
            "/notifications/subscriptions",
            axum::routing::put(notifications::upsert_subscription),
        )
        .route(
            "/notifications/test",
            post(notifications::test_notification),
        )
        // live (Phase 3)
        .route(
            "/admin/live/connections",
            get(live::list_connections).post(live::create_connection),
        )
        .route(
            "/admin/live/connections/{connection_id}/start",
            post(live::start_node),
        )
        .route("/admin/live/orders", post(live::submit_order))
        .route("/admin/live/nodes/{node_id}/stop", post(live::stop_node))
        .route(
            "/admin/live/kill-switch/enable",
            post(live::enable_kill_switch),
        )
        .route(
            "/admin/live/kill-switch/disable",
            post(live::disable_kill_switch),
        )
        .route("/admin/live/reconciliation", get(admin::live_not_available))
        // licensing / artifacts
        .route("/licensing-status", get(licensing::status))
        .route("/artifacts/{artifact_id}", get(artifacts::get))
        .route(
            "/artifacts/{artifact_id}/download",
            get(artifacts::download),
        );

    Router::new()
        .nest("/api/v1", v1)
        .layer(axum::middleware::from_fn(middleware::correlation))
        .layer(axum::extract::DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
}

/// The mounted route count (drift guard; the openapi_contract test asserts
/// equality with the spec's operation count).
pub fn mounted_route_count() -> usize {
    CONTRACT_ROUTES.len()
}
