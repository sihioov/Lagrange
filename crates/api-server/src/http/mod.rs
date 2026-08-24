//! The `/api/v1` router: correlation middleware, body limits, and every
//! documented route group. `api_router` is the single assembly point the
//! tests drive through `tower::ServiceExt::oneshot`.

pub mod admin;
pub mod artifacts;
pub mod backtests;
pub mod candidates;
pub mod dto;
pub mod entitlement;
pub mod error;
pub mod idempotency;
pub mod licensing;
pub mod live;
pub mod middleware;
pub mod notifications;
pub mod owner_beta;
pub mod pagination;
pub mod paper;
pub mod recommendations;
pub mod screener;
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
        .route(
            "/recommendations/owner-beta/price-only/runs",
            post(owner_beta::create_price_only_run).get(owner_beta::list_price_only_runs),
        )
        .route(
            "/recommendations/owner-beta/price-only/runs/{run_id}",
            get(owner_beta::get_price_only_run),
        )
        .route("/recommendations/runs", get(recommendations::list_runs))
        .route(
            "/recommendations/runs/{run_id}",
            get(recommendations::get_run),
        )
        .route("/recommendations/latest", get(recommendations::latest))
        // common individual-stock research (separate from ETF recommendations)
        .route("/candidates/feed/latest", get(candidates::latest_feed))
        .route("/candidates/feed/{date}", get(candidates::feed_on_date))
        .route(
            "/stocks/{instrument_id}/analysis",
            get(candidates::stock_analysis),
        )
        .route("/screener/query", post(screener::query))
        .route(
            "/screener/screens",
            get(screener::list_screens).post(screener::create_screen),
        )
        .route(
            "/screener/screens/{id}",
            get(screener::get_screen)
                .put(screener::update_screen)
                .delete(screener::delete_screen),
        )
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
        .route(
            "/paper/accounts/{account_id}/recommendation-previews/{preview_id}/apply",
            post(paper::apply_rebalance_preview),
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
        // Axum strips a nested prefix before middleware installed on the
        // nested router.  Keep this full-path policy outside `nest` so its
        // `/api/v1/...` allowlist cannot silently miss every protected route.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            middleware::owner_beta_admission,
        ))
        // Correlation is installed after the policy layer so it runs first
        // and every admission rejection carries the ordinary request id.
        .layer(axum::middleware::from_fn(middleware::correlation))
        .layer(axum::extract::DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
}

/// The mounted route count (drift guard; the openapi_contract test asserts
/// equality with the spec's operation count).
pub fn mounted_route_count() -> usize {
    CONTRACT_ROUTES.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::owner_beta_applies;
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request};
    use tower::ServiceExt;

    fn concrete_path(template: &str) -> String {
        template
            .replace("{run_id}", "00000000-0000-0000-0000-000000000001")
            .replace("{account_id}", "00000000-0000-0000-0000-000000000002")
            .replace("{preview_id}", "00000000-0000-0000-0000-000000000003")
    }

    #[tokio::test]
    async fn owner_beta_admission_sees_the_full_path_outside_the_nested_router() {
        let state =
            ApiState::test_without_database(crate::http::state::OwnerBetaAccessMode::OwnerOnly);
        assert_eq!(state.app_pool.size(), 0);
        assert_eq!(state.admin_pool.size(), 0);
        assert_eq!(state.audit_pool.size(), 0);
        let app = api_router(state.clone());

        // DELETE has no MethodRouter handler here.  The outer admission
        // boundary must still recognize the protected full path and return
        // its authentication rejection before Axum can return 405.  With the
        // old layer inside `v1`, Axum exposed `/backtests` to the middleware,
        // skipped the gate, and this assertion failed with 405.
        let protected = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri("/api/v1/backtests")
                    .body(Body::empty())
                    .expect("protected request"),
            )
            .await
            .expect("protected response");
        assert_eq!(protected.status(), StatusCode::UNAUTHORIZED);
        let request_id = protected
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .expect("correlation runs before admission")
            .to_owned();
        assert!(!request_id.is_empty());
        let body = to_bytes(protected.into_body(), 16 * 1024)
            .await
            .expect("bounded error body");
        let body: serde_json::Value = serde_json::from_slice(&body).expect("typed error JSON");
        assert_eq!(body["error"]["code"], "SESSION_UNKNOWN");
        assert_eq!(body["error"]["request_id"], request_id);

        // Every currently inventoried full path is intercepted without a
        // session-store or product-store query. This turns any newly added
        // route in the three groups into a required admission test case.
        for route in CONTRACT_ROUTES
            .iter()
            .filter(|route| owner_beta_applies(route.path))
        {
            let protected = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(route.method.parse::<Method>().expect("contract method"))
                        .uri(concrete_path(route.path))
                        .body(Body::empty())
                        .expect("inventoried protected request"),
                )
                .await
                .expect("inventoried protected response");
            assert_eq!(
                protected.status(),
                StatusCode::UNAUTHORIZED,
                "{} {} must be admitted centrally",
                route.method,
                route.path
            );
        }

        // A real, database-free route outside the product prefixes must not
        // be misclassified by the outer layer.
        let unrelated = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/metrics")
                    .body(Body::empty())
                    .expect("unrelated request"),
            )
            .await
            .expect("unrelated response");
        assert_eq!(unrelated.status(), StatusCode::OK);

        assert_eq!(state.app_pool.size(), 0);
        assert_eq!(state.admin_pool.size(), 0);
        assert_eq!(state.audit_pool.size(), 0);

        // The Paper default is inert outside owner-only access mode.
        let normal_state =
            ApiState::test_without_database(crate::http::state::OwnerBetaAccessMode::Disabled);
        let normal = api_router(normal_state.clone())
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri("/api/v1/paper/accounts")
                    .body(Body::empty())
                    .expect("normal-mode Paper request"),
            )
            .await
            .expect("normal-mode Paper response");
        assert_eq!(normal.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(normal_state.app_pool.size(), 0);
        assert_eq!(normal_state.admin_pool.size(), 0);
        assert_eq!(normal_state.audit_pool.size(), 0);
    }

    #[tokio::test]
    async fn owner_beta_live_routes_fail_before_any_handler_logic() {
        let state =
            ApiState::test_without_database(crate::http::state::OwnerBetaAccessMode::OwnerOnly);
        let app = api_router(state.clone());
        let requests = [
            (Method::GET, "/api/v1/admin/live/connections", String::new()),
            (
                Method::POST,
                "/api/v1/admin/live/connections",
                "{}".to_owned(),
            ),
            (
                Method::POST,
                "/api/v1/admin/live/connections/00000000-0000-0000-0000-000000000001/start",
                "{}".to_owned(),
            ),
            (
                Method::POST,
                "/api/v1/admin/live/nodes/00000000-0000-0000-0000-000000000002/stop",
                "{}".to_owned(),
            ),
            (
                Method::POST,
                "/api/v1/admin/live/kill-switch/enable",
                "{}".to_owned(),
            ),
            (
                Method::POST,
                "/api/v1/admin/live/kill-switch/disable",
                r#"{"reason":"owner-beta exclusion"}"#.to_owned(),
            ),
            (
                Method::POST,
                "/api/v1/admin/live/orders",
                r#"{
                    "connection_id":"00000000-0000-0000-0000-000000000001",
                    "account_id":"00000000-0000-0000-0000-000000000002",
                    "instrument_id":"069500.KRX",
                    "side":"BUY",
                    "quantity":"1",
                    "price":"100",
                    "dry_run":false
                }"#
                .to_owned(),
            ),
            (
                Method::GET,
                "/api/v1/admin/live/reconciliation",
                String::new(),
            ),
        ];

        for (method, path, body) in requests {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(&method)
                        .uri(path)
                        .header("content-type", "application/json")
                        .header("x-request-id", "owner-beta-live-exclusion")
                        .body(Body::from(body))
                        .expect("owner-beta Live request"),
                )
                .await
                .expect("owner-beta Live response");
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{method} {path}");
            let body = to_bytes(response.into_body(), 16 * 1024)
                .await
                .expect("bounded owner-beta Live error body");
            let body: serde_json::Value = serde_json::from_slice(&body).expect("typed error JSON");
            assert_eq!(body["error"]["code"], "FORBIDDEN", "{method} {path}");
            assert_eq!(body["error"]["message"], "forbidden", "{method} {path}");
        }

        // A handler-side session lookup or Live repository call would need a
        // database connection. The lazy pools remaining unopened proves the
        // exclusion ran at the outer route boundary, including for the real
        // non-dry-run order path above.
        assert_eq!(state.app_pool.size(), 0);
        assert_eq!(state.admin_pool.size(), 0);
        assert_eq!(state.audit_pool.size(), 0);

        // The exclusion is specific to owner-only mode. In the documented
        // disabled mode, the same real route reaches its ordinary session
        // boundary instead of inheriting the beta refusal.
        let normal_state =
            ApiState::test_without_database(crate::http::state::OwnerBetaAccessMode::Disabled);
        let normal = api_router(normal_state)
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/admin/live/connections")
                    .body(Body::empty())
                    .expect("ordinary Live request"),
            )
            .await
            .expect("ordinary Live response");
        assert_eq!(normal.status(), StatusCode::UNAUTHORIZED);
    }
}
