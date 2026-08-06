//! Todo 24 recommendation routes: create (entitlement-gated, queued),
//! get, list, latest, items; idempotent replay; ownership isolation.

mod common;
use axum::http::StatusCode;
use common::{Harness, status};
use serde_json::json;

/// Create a strategy config for `actor` and return its id.
async fn config_id(h: &Harness, actor: &common::UserCtx) -> String {
    let resp = h
        .send(
            "POST",
            "/api/v1/strategies/buy_and_hold/configs",
            Some(actor),
            true,
            Some("test-rid-1"),
            Some("seed-config-001"),
            Some(json!({ "strategy_version": "1.0.0", "config": { "lookback": 200 }, "is_active": true })),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    assert_eq!(resp.status(), StatusCode::CREATED);
    Harness::body_json(resp).await["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn http_recommendations_create_queue_result_happy() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let cfg = config_id(&h, &h.member).await;

    // create -> PENDING run + QUEUED recommendation job.
    let resp = h
        .post(
            "/api/v1/recommendations/runs",
            Some(&h.member),
            true,
            json!({ "strategy_config_id": cfg, "as_of": "2026-01-31" }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::CREATED);
    h.assert_rid_echo(&resp);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["status"], "PENDING");
    assert_eq!(body["as_of"], "2026-01-31");
    assert_eq!(body["strategy_config_id"], cfg);
    let run_id = body["id"].as_str().unwrap().to_string();
    let job_id = body["job_id"].as_str().unwrap().to_string();
    assert!(
        !body.to_string().contains("owner_user_id"),
        "no tenant column leak"
    );

    // The job is queued (visible through the API? recommendation job is the
    // actor's own -> assert via admin? no: assert through the queue table).
    let job_status: String = sqlx::query_scalar("SELECT status FROM jobs WHERE id = $1::uuid")
        .bind(&job_id)
        .fetch_one(&h.member_pool().await)
        .await
        .unwrap();
    assert_eq!(job_status, "QUEUED");

    // Worker completes the run: seed SUCCEEDED + items (the research worker's
    // side effect; the API reads it back).
    h.seed_tenant(
        &h.member,
        &format!(
            "UPDATE recommendation_runs SET status='SUCCEEDED', summary_json='{{\"selected\":2}}'::jsonb \
             WHERE id='{run_id}'"
        ),
    )
    .await;
    h.seed_tenant(
        &h.member,
        &format!(
            "INSERT INTO recommendation_items (id, recommendation_run_id, owner_user_id, instrument_id, rank, target_weight, reason_codes, factors_json, excluded, exclusion_reason) VALUES \
             (gen_random_uuid(), '{run_id}', '{owner}', '069500.KRX', 1, 0.600000, '[\"momentum_12_1\"]'::jsonb, '{{\"momentum_12_1\":0.8123}}'::jsonb, false, NULL), \
             (gen_random_uuid(), '{run_id}', '{owner}', '229200.KRX', 2, 0.400000, '[\"trend_50\"]'::jsonb, '{{\"trend_50\":0.71}}'::jsonb, false, NULL)",
            owner = h.member.user_id
        ),
    )
    .await;

    // get -> SUCCEEDED with items.
    let resp = h
        .get(
            &format!("/api/v1/recommendations/runs/{run_id}"),
            Some(&h.member),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["status"], "SUCCEEDED");
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["instrument_id"], "069500.KRX");
    assert_eq!(items[0]["target_weight"], "0.600000");
    assert_eq!(items[1]["excluded"], false);

    // latest by config.
    let resp = h
        .get(
            &format!("/api/v1/recommendations/latest?strategy_config_id={cfg}"),
            Some(&h.member),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["run"]["id"], run_id);
    assert_eq!(body["run"]["items"].as_array().unwrap().len(), 2);
    h.teardown().await;
}

#[tokio::test]
async fn http_recommendations_entitlement_gate_fails_closed() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let cfg = config_id(&h, &h.member).await;
    // Revoke the ACTIVE entitlement (expire it) -> create is denied.
    h.seed_shared("UPDATE data_entitlements SET status='REVOKED' WHERE status='ACTIVE'")
        .await;
    let resp = h
        .post(
            "/api/v1/recommendations/runs",
            Some(&h.member),
            true,
            json!({ "strategy_config_id": cfg, "as_of": "2026-01-31" }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::FORBIDDEN);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "DATA_ENTITLEMENT_REQUIRED");

    // No side effect: no recommendation_runs row, no job.
    let runs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM recommendation_runs WHERE strategy_config_id = $1::uuid",
    )
    .bind(&cfg)
    .fetch_one(&h.member_pool().await)
    .await
    .unwrap();
    assert_eq!(runs, 0, "denied create must not leave rows");
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE job_type='recommendation'")
        .fetch_one(&h.member_pool().await)
        .await
        .unwrap();
    assert_eq!(jobs, 0, "denied create must not enqueue");
    h.teardown().await;
}

#[tokio::test]
async fn http_recommendations_ownership_and_idempotency() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let cfg = config_id(&h, &h.member).await;
    let resp = h
        .post(
            "/api/v1/recommendations/runs",
            Some(&h.member),
            true,
            json!({ "strategy_config_id": cfg, "as_of": "2026-01-31" }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::CREATED);
    let run_id = Harness::body_json(resp).await["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Member B (owner) cannot read member A's run.
    let resp = h
        .get(
            &format!("/api/v1/recommendations/runs/{run_id}"),
            Some(&h.owner),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::NOT_FOUND);

    // Idempotent replay: same key -> same run id, no second run/job.
    // (The first create above used a different auto key, so the fixed-key
    // create below is a distinct run; its OWN replay must dedup.)
    let key = "rec-run-001";
    let make = |body: &serde_json::Value| {
        h.send(
            "POST",
            "/api/v1/recommendations/runs",
            Some(&h.member),
            true,
            Some("test-rid-1"),
            Some(key),
            Some(body.clone()),
        )
    };
    let b = json!({ "strategy_config_id": cfg, "as_of": "2026-01-31" });
    let r1 = make(&b).await;
    assert_eq!(r1.status(), StatusCode::CREATED);
    let id1 = Harness::body_json(r1).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(
        id1, run_id,
        "different idempotency keys create distinct runs"
    );

    let r2 = make(&b).await;
    assert_eq!(r2.status(), StatusCode::CREATED);
    assert_eq!(r2.headers()["x-idempotent-replay"], "true");
    let id2 = Harness::body_json(r2).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(id1, id2);

    // Two DISTINCT keys created two runs; the replay did not create a third.
    let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM recommendation_runs")
        .fetch_one(&h.member_pool().await)
        .await
        .unwrap();
    assert_eq!(runs, 2, "replay must not double-create for the same key");
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE job_type='recommendation'")
        .fetch_one(&h.member_pool().await)
        .await
        .unwrap();
    assert_eq!(jobs, 2, "no double enqueue for the same key");
    h.teardown().await;
}

#[tokio::test]
async fn http_recommendations_fuzz_rejects_invalid_input() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let cfg = config_id(&h, &h.member).await;
    // Invalid date.
    let resp = h
        .post(
            "/api/v1/recommendations/runs",
            Some(&h.member),
            true,
            json!({ "strategy_config_id": cfg, "as_of": "2026-13-45" }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::BAD_REQUEST);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "INVALID_DATE");

    // Invalid uuid.
    let resp = h
        .post(
            "/api/v1/recommendations/runs",
            Some(&h.member),
            true,
            json!({ "strategy_config_id": "not-a-uuid", "as_of": "2026-01-31" }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::BAD_REQUEST);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "INVALID_PARAMETER");

    // No rows created by any fuzz input.
    let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM recommendation_runs")
        .fetch_one(&h.member_pool().await)
        .await
        .unwrap();
    assert_eq!(runs, 0, "fuzz must not create rows");
    h.teardown().await;
}
