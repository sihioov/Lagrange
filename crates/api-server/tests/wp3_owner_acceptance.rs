//! WP-3 evidence for the browser-equivalent Owner request prefix.
//!
//! The checked-in approval registry intentionally contains only the immutable
//! contract metadata. The corresponding 17,688-bar artifact is not a test
//! fixture, and the production approval type is deliberately nonconstructible
//! outside its verifier. This test therefore proves only the truthful
//! pre-publication journey: authenticated session -> CSRF -> active strategy
//! config -> CSRF rejection -> fail-closed artifact admission -> safe retry.
//! It must not be read as evidence of a successful runner publication.

mod common;

use axum::http::StatusCode;
use common::{Harness, UserCtx, status};
use serde_json::json;

const AS_OF: &str = "2026-08-19";

fn wp3_seoul_today() -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(2026, 8, 19).expect("WP-3 acceptance date")
}

fn wp3_candidate_eod_ready() -> bool {
    true
}

async fn fresh_csrf(h: &Harness, user: &UserCtx) -> UserCtx {
    let response = h.get("/api/v1/auth/csrf", Some(user)).await;
    assert_eq!(status(&response), StatusCode::OK);
    let body = Harness::body_json(response).await;
    let token = body["csrf_token"]
        .as_str()
        .expect("CSRF response token")
        .to_owned();
    let mut fresh = user.clone();
    fresh.csrf_token = token;
    fresh
}

async fn owner_beta_counts(h: &Harness, owner: &UserCtx) -> (i64, i64) {
    let runs: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint
           FROM owner_beta_recommendation_runs
          WHERE owner_user_id = $1",
    )
    .bind(owner.user_id)
    .fetch_one(&h.admin_pool)
    .await
    .expect("owner-beta run count");
    let jobs: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint
           FROM jobs
          WHERE owner_user_id = $1
            AND job_type = 'owner_beta_price_recommendation'",
    )
    .bind(owner.user_id)
    .fetch_one(&h.admin_pool)
    .await
    .expect("owner-beta job count");
    (runs, jobs)
}

#[tokio::test]
async fn wp3_owner_prefix_is_ordered_and_unavailable_artifact_retries_safely() {
    let Some(mut h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    h.restart_api_with_candidate_clock(wp3_seoul_today, wp3_candidate_eod_ready)
        .await;
    h.restart_api_with_owner_beta_price_input().await;

    // 1. An authenticated browser session is the starting point.
    let session = h.get("/api/v1/auth/session", Some(&h.owner)).await;
    assert_eq!(status(&session), StatusCode::OK);
    let session_body = Harness::body_json(session).await;
    assert_eq!(session_body["role"], "owner");
    assert_eq!(session_body["user_id"], h.owner.user_id.to_string());

    // 2. The browser fetches CSRF before the first mutation.
    let config_user = fresh_csrf(&h, &h.owner).await;
    let config_response = h
        .send(
            "POST",
            "/api/v1/strategies/buy_and_hold/configs",
            Some(&config_user),
            true,
            Some("wp3-owner-config-rid"),
            Some("wp3-owner-config-idempotency"),
            Some(json!({
                "strategy_version": "1.0.0",
                "config": { "lookback": 200 },
                "is_active": true
            })),
        )
        .await;
    assert_eq!(status(&config_response), StatusCode::CREATED);
    let config_body = Harness::body_json(config_response).await;
    let strategy_config_id = config_body["id"]
        .as_str()
        .expect("created strategy config id")
        .to_owned();
    assert_eq!(config_body["is_active"], true);

    // The saved configuration is durable and active before recommendation
    // admission is attempted.
    let saved = h
        .get(
            &format!("/api/v1/strategy-configs/{strategy_config_id}"),
            Some(&config_user),
        )
        .await;
    assert_eq!(status(&saved), StatusCode::OK);
    let saved_body = Harness::body_json(saved).await;
    assert_eq!(saved_body["id"], strategy_config_id);
    assert_eq!(saved_body["is_active"], true);
    assert_eq!(saved_body["config"]["lookback"], 200);

    let request_body = json!({
        "strategy_config_id": strategy_config_id,
        "as_of": AS_OF
    });
    let before = owner_beta_counts(&h, &h.owner).await;
    assert_eq!(before, (0, 0));
    assert!(
        !h.artifact_root.join("historical-price-beta-root").exists(),
        "this evidence must not create or copy an approved artifact"
    );

    // 3. Missing CSRF is rejected before approval or durable enqueue.
    let denied = h
        .send(
            "POST",
            "/api/v1/recommendations/owner-beta/price-only/runs",
            Some(&config_user),
            false,
            Some("wp3-owner-beta-csrf-denied"),
            Some("wp3-owner-beta-csrf-denied-key"),
            Some(request_body.clone()),
        )
        .await;
    assert_eq!(status(&denied), StatusCode::FORBIDDEN);
    let denied_body = Harness::body_json(denied).await;
    assert_eq!(Harness::error_code(&denied_body), "CSRF_DENIED");
    assert_eq!(owner_beta_counts(&h, &h.owner).await, before);

    // 4. A browser retry obtains a fresh token. With no checked-in approved
    // bytes available, the real API fails closed before it can enqueue.
    let first_attempt_user = fresh_csrf(&h, &config_user).await;
    let first_attempt = h
        .send(
            "POST",
            "/api/v1/recommendations/owner-beta/price-only/runs",
            Some(&first_attempt_user),
            true,
            Some("wp3-owner-beta-first"),
            Some("wp3-owner-beta-safe-retry-key"),
            Some(request_body.clone()),
        )
        .await;
    assert_eq!(status(&first_attempt), StatusCode::SERVICE_UNAVAILABLE);
    let first_body = Harness::body_json(first_attempt).await;
    assert_eq!(
        Harness::error_code(&first_body),
        "OWNER_BETA_PRICE_INPUT_UNAVAILABLE"
    );
    assert_eq!(
        first_body["error"]["message"],
        "owner-beta price input unavailable"
    );
    assert_eq!(owner_beta_counts(&h, &h.owner).await, before);

    // The same client key/body is safe to retry after another browser CSRF
    // fetch: no run, queue job, or partial publication exists to replay.
    let retry_user = fresh_csrf(&h, &first_attempt_user).await;
    let retry = h
        .send(
            "POST",
            "/api/v1/recommendations/owner-beta/price-only/runs",
            Some(&retry_user),
            true,
            Some("wp3-owner-beta-retry"),
            Some("wp3-owner-beta-safe-retry-key"),
            Some(request_body),
        )
        .await;
    assert_eq!(status(&retry), StatusCode::SERVICE_UNAVAILABLE);
    let retry_body = Harness::body_json(retry).await;
    assert_eq!(
        Harness::error_code(&retry_body),
        "OWNER_BETA_PRICE_INPUT_UNAVAILABLE"
    );
    assert_eq!(owner_beta_counts(&h, &h.owner).await, before);

    let history = h
        .get(
            "/api/v1/recommendations/owner-beta/price-only/runs",
            Some(&retry_user),
        )
        .await;
    assert_eq!(status(&history), StatusCode::OK);
    let history_body = Harness::body_json(history).await;
    assert_eq!(history_body["items"], json!([]));

    h.teardown().await;
}
