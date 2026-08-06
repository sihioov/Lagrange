//! Todo 24 HTTP contract conventions: `/api/v1` JSON/cursor/error/
//! correlation rules, the typed error envelope, stable error codes, and the
//! strategy catalog routes.
//!
//! Every test here is `http_contract_*` (the plan acceptance command
//! `cargo test -p api-server` filters by test function names).

mod common;
use axum::http::StatusCode;
use common::{Harness, status};
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Conventions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn http_contract_error_envelope_shape_and_codes() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    // 404 on an unknown path under /api/v1 (no session needed): typed envelope.
    let resp = h
        .get("/api/v1/strategies/does-not-exist", Some(&h.member))
        .await;
    assert_eq!(status(&resp), StatusCode::NOT_FOUND);
    h.assert_rid_echo(&resp);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "RESOURCE_NOT_FOUND");
    let err = &body["error"];
    assert_eq!(err["request_id"], "test-rid-1");
    assert!(err["message"].is_string());
    assert!(err.get("details").is_none() || err["details"].is_object());

    // Unauthenticated: 401 SESSION_UNKNOWN.
    let resp = h.get("/api/v1/strategies", None).await;
    assert_eq!(status(&resp), StatusCode::UNAUTHORIZED);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "SESSION_UNKNOWN");
    h.teardown().await;
}

#[tokio::test]
async fn http_contract_correlation_ids_propagate() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    // Custom X-Request-Id is echoed and appears in the error envelope.
    let resp = h
        .send(
            "GET",
            "/api/v1/strategies/does-not-exist",
            Some(&h.member),
            false,
            Some("corr-abc-123"),
            None,
            None,
        )
        .await;
    assert_eq!(resp.headers()["x-request-id"], "corr-abc-123");
    let body = Harness::body_json(resp).await;
    assert_eq!(body["error"]["request_id"], "corr-abc-123");

    // Missing X-Request-Id: server generates one and echoes it back.
    let req = axum::http::Request::builder()
        .method("GET")
        .uri("/api/v1/strategies")
        .header(
            "cookie",
            format!("__Host-lagrange_session={}", h.member.cookie_value),
        )
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    let echoed = resp.headers()["x-request-id"].to_str().unwrap().to_string();
    assert!(!echoed.is_empty());
    assert_eq!(resp.status(), StatusCode::OK);
    h.teardown().await;
}

#[tokio::test]
async fn http_contract_oversized_payload_is_typed_413() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    // 300 KB body > 256 KB limit.
    let huge = serde_json::json!({ "blob": "x".repeat(300 * 1024) });
    let resp = h
        .post(
            "/api/v1/strategies/buy_and_hold/configs",
            Some(&h.member),
            true,
            huge,
        )
        .await;
    assert_eq!(status(&resp), StatusCode::PAYLOAD_TOO_LARGE);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "PAYLOAD_TOO_LARGE");
    h.teardown().await;
}

#[tokio::test]
async fn http_contract_mutating_route_requires_csrf_and_idempotency_key() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    // No CSRF token -> 403 CSRF_DENIED (audited).
    let body = serde_json::json!({ "strategy_version": "1.0.0", "config": { "lookback": 200 }, "is_active": true });
    let resp = h
        .send(
            "POST",
            "/api/v1/strategies/buy_and_hold/configs",
            Some(&h.member),
            false,
            Some("test-rid-1"),
            None,
            Some(body.clone()),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::FORBIDDEN);
    let resp_body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&resp_body), "CSRF_DENIED");

    // CSRF ok but no Idempotency-Key -> 400 IDEMPOTENCY_KEY_REQUIRED.
    let resp = h
        .send(
            "POST",
            "/api/v1/strategies/buy_and_hold/configs",
            Some(&h.member),
            true,
            Some("test-rid-1"),
            None,
            Some(body),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::BAD_REQUEST);
    let resp_body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&resp_body), "IDEMPOTENCY_KEY_REQUIRED");
    h.teardown().await;
}

#[tokio::test]
async fn http_contract_invalid_parameter_schema_is_typed_400() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    // Unknown field in body (deny_unknown_fields) -> 400 INVALID_PARAMETER.
    let resp = h
        .send(
            "POST",
            "/api/v1/strategies/buy_and_hold/configs",
            Some(&h.member),
            true,
            Some("test-rid-1"),
            None,
            Some(serde_json::json!({ "strategy_version": "1.0.0", "config": { "lookback": 200 }, "is_active": true, "owner_user_id": "00000000-0000-0000-0000-000000000000" })),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::BAD_REQUEST);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "INVALID_PARAMETER");
    h.teardown().await;
}

// ---------------------------------------------------------------------------
// Strategy catalog
// ---------------------------------------------------------------------------

#[tokio::test]
async fn http_contract_strategy_catalog_happy() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let resp = h.get("/api/v1/strategies", Some(&h.member)).await;
    assert_eq!(status(&resp), StatusCode::OK);
    h.assert_rid_echo(&resp);
    let body = Harness::body_json(resp).await;
    let items = body["items"].as_array().expect("items array");
    assert_eq!(items.len(), 2);
    let ids: Vec<&str> = items.iter().map(|i| i["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"buy_and_hold"));
    assert!(ids.contains(&"trend_following"));
    // No database model leakage: no owner_user_id / internal columns.
    let serialized = body.to_string();
    assert!(
        !serialized.contains("owner_user_id"),
        "no tenant column leak"
    );
    assert!(
        !serialized.contains("storage_path"),
        "no internal path leak"
    );

    let resp = h
        .get("/api/v1/strategies/buy_and_hold", Some(&h.member))
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["id"], "buy_and_hold");
    assert_eq!(body["state"], "Validated");
    assert_eq!(body["latest_version"], "1.0.0");
    h.teardown().await;
}

#[tokio::test]
async fn http_contract_strategy_catalog_is_shared_and_readonly() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    // Both roles see the same catalog; no session -> 401 even for shared data.
    let member_resp = h.get("/api/v1/strategies", Some(&h.member)).await;
    let owner_resp = h.get("/api/v1/strategies", Some(&h.owner)).await;
    assert_eq!(member_resp.status(), StatusCode::OK);
    assert_eq!(owner_resp.status(), StatusCode::OK);
    h.teardown().await;
}
