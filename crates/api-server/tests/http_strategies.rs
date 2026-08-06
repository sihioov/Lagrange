//! Todo 24 strategy config routes: schema-bound parameter validation, actor
//! ownership, idempotent creation, audit, and fuzz rejection.

mod common;
use common::{Harness, status};
use axum::http::StatusCode;
use serde_json::json;

#[tokio::test]
async fn http_strategies_config_create_happy() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let resp = h
        .post(
            "/api/v1/strategies/buy_and_hold/configs",
            Some(&h.member),
            true,
            json!({ "strategy_version": "1.0.0", "config": { "lookback": 200 }, "is_active": true }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::CREATED);
    h.assert_rid_echo(&resp);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["strategy_id"], "buy_and_hold");
    assert_eq!(body["config"]["lookback"], 200);
    assert!(!body.to_string().contains("owner_user_id"), "no tenant column leak");

    // The row is readable back through the API (ownership round-trip).
    let id = body["id"].as_str().unwrap();
    let resp = h
        .get(&format!("/api/v1/strategy-configs/{id}"), Some(&h.member))
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["id"].as_str().unwrap(), id);
    h.teardown().await;
}

#[tokio::test]
async fn http_strategies_config_invalid_parameters_rejected() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    // Non-integer lookback against the published JSON schema.
    let resp = h
        .post(
            "/api/v1/strategies/buy_and_hold/configs",
            Some(&h.member),
            true,
            json!({ "strategy_version": "1.0.0", "config": { "lookback": "two-hundred" }, "is_active": true }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::UNPROCESSABLE_ENTITY);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "INVALID_STRATEGY_PARAMETER");

    // Unknown parameter (schema additionalProperties:false) -> same code.
    let resp = h
        .post(
            "/api/v1/strategies/buy_and_hold/configs",
            Some(&h.member),
            true,
            json!({ "strategy_version": "1.0.0", "config": { "lookback": 200, "python_code": "import os" }, "is_active": true }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::UNPROCESSABLE_ENTITY);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "INVALID_STRATEGY_PARAMETER");

    // Unknown strategy version.
    let resp = h
        .post(
            "/api/v1/strategies/buy_and_hold/configs",
            Some(&h.member),
            true,
            json!({ "strategy_version": "99.0.0", "config": { "lookback": 200 }, "is_active": true }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::BAD_REQUEST);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "INVALID_PARAMETER");
    h.teardown().await;
}

#[tokio::test]
async fn http_strategies_config_ownership_isolation() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    // Member A creates a config.
    let resp = h
        .post(
            "/api/v1/strategies/buy_and_hold/configs",
            Some(&h.member),
            true,
            json!({ "strategy_version": "1.0.0", "config": { "lookback": 100 }, "is_active": true }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::CREATED);
    let id = Harness::body_json(resp).await["id"].as_str().unwrap().to_string();

    // Member B (owner) reading A's config by direct id -> 404, no leak.
    let resp = h
        .get(&format!("/api/v1/strategy-configs/{id}"), Some(&h.owner))
        .await;
    assert_eq!(status(&resp), StatusCode::NOT_FOUND);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "RESOURCE_NOT_FOUND");
    h.teardown().await;
}

#[tokio::test]
async fn http_strategies_config_idempotent_replay() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let body = json!({ "strategy_version": "1.0.0", "config": { "lookback": 250 }, "is_active": true });
    let key = "config-create-001";
    // Without the Idempotency-Key header the mutation is rejected.
    let r0 = h
        .send("POST", "/api/v1/strategies/buy_and_hold/configs", Some(&h.member), true, Some("test-rid-1"), None, Some(body.clone()))
        .await;
    assert_eq!(r0.status(), StatusCode::BAD_REQUEST);
    let b0 = Harness::body_json(r0).await;
    assert_eq!(Harness::error_code(&b0), "IDEMPOTENCY_KEY_REQUIRED");

    // First call with the key -> 201.
    let r1 = h
        .send("POST", "/api/v1/strategies/buy_and_hold/configs", Some(&h.member), true, Some("test-rid-1"), Some(key), Some(body.clone()))
        .await;
    assert_eq!(r1.status(), StatusCode::CREATED);
    let b1 = Harness::body_json(r1).await;
    let id1 = b1["id"].as_str().unwrap().to_string();

    // Replay with the SAME key + SAME body -> same id, replay header, no double row.
    let r2 = h
        .send("POST", "/api/v1/strategies/buy_and_hold/configs", Some(&h.member), true, Some("test-rid-1"), Some(key), Some(body.clone()))
        .await;
    assert_eq!(r2.status(), StatusCode::CREATED);
    assert_eq!(r2.headers()["x-idempotent-replay"], "true");
    let b2 = Harness::body_json(r2).await;
    assert_eq!(b2["id"].as_str().unwrap(), id1);

    // Replay with the SAME key + DIFFERENT body -> 409 IDEMPOTENCY_KEY_MISMATCH.
    let other = json!({ "strategy_version": "1.0.0", "config": { "lookback": 251 }, "is_active": true });
    let r3 = h
        .send("POST", "/api/v1/strategies/buy_and_hold/configs", Some(&h.member), true, Some("test-rid-1"), Some(key), Some(other))
        .await;
    assert_eq!(r3.status(), StatusCode::CONFLICT);
    let b3 = Harness::body_json(r3).await;
    assert_eq!(Harness::error_code(&b3), "IDEMPOTENCY_KEY_MISMATCH");

    // Exactly one row exists for the actor (no double side effect).
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM user_strategy_configs WHERE strategy_id = 'buy_and_hold' AND config_json->>'lookback' = '250'",
    )
    .fetch_one(&h.app_pool)
    .await
    .unwrap();
    assert_eq!(count, 1, "idempotent replay must not double-create");
    h.teardown().await;
}
