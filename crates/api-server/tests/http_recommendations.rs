//! Todo 24 recommendation routes: create (entitlement-gated, queued),
//! get, list, latest, items; idempotent replay; ownership isolation.

mod common;
use axum::http::StatusCode;
use common::{Harness, status};
use job_queue::recommendation::child::TargetChildPaths;
use job_queue::recommendation::{
    RecommendationOutcome, RecommendationRunnerConfig, RecommendationRunnerPaths, run_once,
};
use job_queue::{JobQueue, QueueConfig};
use serde_json::json;
use std::time::Duration;

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
    assert_eq!(body["trigger_kind"], "MANUAL");
    assert!(body["provenance"]["dataset_version_id"].is_string());
    assert!(
        !body.to_string().contains("owner_user_id"),
        "no tenant column leak"
    );

    // The job is queued (visible through the API? recommendation job is the
    // actor's own -> assert via admin? no: assert through the queue table).
    let (job_status, payload, persisted_job_id, queue_key): (
        String,
        serde_json::Value,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT j.status, j.payload_json, r.job_id::text, j.idempotency_key \
             FROM jobs j JOIN recommendation_runs r ON r.id = $2::uuid WHERE j.id = $1::uuid",
    )
    .bind(&job_id)
    .bind(&run_id)
    .fetch_one(&h.member_pool().await)
    .await
    .unwrap();
    assert_eq!(job_status, "QUEUED");
    assert_eq!(persisted_job_id.as_deref(), Some(job_id.as_str()));
    assert!(
        queue_key.is_some_and(|key| key.starts_with("recommendation:")),
        "job idempotency is isolated from other queue job types"
    );
    assert_eq!(payload["run_id"], run_id);
    assert_eq!(payload["strategy_config_id"], cfg);
    assert!(
        payload["dataset"]["id"].is_string(),
        "runner requires a pinned dataset id"
    );
    assert!(payload["dataset"]["dataset_id"].is_string());
    assert!(payload["dataset"]["version"].is_string());
    assert!(payload["dataset"]["curated_version"].is_u64());
    assert!(payload["dataset"]["manifest_sha256"].is_string());

    // Drive the actual typed runner once. The shared HTTP harness intentionally
    // has no deployment curated-store mount, so the real runner records a
    // retryable path failure rather than the test mutating a result row by
    // hand. GET observes the runner's persisted PENDING state.
    let worker = h.worker_pool().await;
    let queue = JobQueue::new(
        worker.clone(),
        None,
        QueueConfig {
            lease: Duration::from_secs(60),
            backoff_base: Duration::from_millis(1),
        },
    );
    let runner = RecommendationRunnerConfig::new(
        Duration::from_secs(1),
        Duration::from_secs(60),
        Duration::from_secs(1),
    )
    .unwrap();
    let paths = RecommendationRunnerPaths {
        data_root: h.artifact_root.clone(),
        universe_manifest: h.artifact_root.join("universe.yaml"),
        child: TargetChildPaths {
            uv_bin: "uv".into(),
            repo_root: h.artifact_root.clone(),
            temp_root: h.artifact_root.join("tmp"),
        },
    };
    assert!(matches!(
        run_once(&worker, &queue, "http-recommendations", &paths, &runner)
            .await
            .unwrap(),
        RecommendationOutcome::Retrying { job_id: id, .. } if id.to_string() == job_id
    ));

    // GET reads the runner-persisted in-flight run with no fabricated items.
    let resp = h
        .get(
            &format!("/api/v1/recommendations/runs/{run_id}"),
            Some(&h.member),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["status"], "PENDING");
    let items = body["items"].as_array().expect("items");
    assert!(items.is_empty());

    // latest by config.
    let resp = h
        .get(
            &format!("/api/v1/recommendations/latest?strategy_config_id={cfg}"),
            Some(&h.member),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    assert!(body["run"].is_null(), "pending run is not a usable report");
    assert_eq!(body["latest_run"]["id"], run_id);
    assert_eq!(body["latest_run"]["status"], "PENDING");
    h.teardown().await;
}

#[tokio::test]
async fn http_recommendations_entitlement_gate_fails_closed() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let cfg = config_id(&h, &h.member).await;
    let existing = h
        .post(
            "/api/v1/recommendations/runs",
            Some(&h.member),
            true,
            json!({ "strategy_config_id": cfg, "as_of": "2026-01-30" }),
        )
        .await;
    assert_eq!(status(&existing), StatusCode::CREATED);
    let existing_id = Harness::body_json(existing).await["id"]
        .as_str()
        .unwrap()
        .to_owned();
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

    // Revocation hides an already-persisted report as well as denying new
    // work; payloads are never exposed after the fresh read gate fails.
    let get = h
        .get(
            &format!("/api/v1/recommendations/runs/{existing_id}"),
            Some(&h.member),
        )
        .await;
    assert_eq!(status(&get), StatusCode::FORBIDDEN);
    assert_eq!(
        Harness::error_code(&Harness::body_json(get).await),
        "DATA_ENTITLEMENT_REQUIRED"
    );

    // No side effect: no recommendation_runs row, no job.
    let runs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM recommendation_runs WHERE strategy_config_id = $1::uuid",
    )
    .bind(&cfg)
    .fetch_one(&h.member_pool().await)
    .await
    .unwrap();
    assert_eq!(runs, 1, "denied create must not leave an additional row");
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE job_type='recommendation'")
        .fetch_one(&h.member_pool().await)
        .await
        .unwrap();
    assert_eq!(jobs, 1, "denied create must not enqueue an additional job");
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
async fn http_recommendations_queue_failure_rolls_back_run_and_job() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let cfg = config_id(&h, &h.member).await;
    h.seed_shared(
        "CREATE FUNCTION fail_recommendation_queue_insert() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION 'queue intentionally unavailable'; END $$; \
         CREATE TRIGGER fail_recommendation_queue_insert BEFORE INSERT ON jobs \
         FOR EACH ROW WHEN (NEW.job_type = 'recommendation') \
         EXECUTE FUNCTION fail_recommendation_queue_insert()",
    )
    .await;

    let resp = h
        .post(
            "/api/v1/recommendations/runs",
            Some(&h.member),
            true,
            json!({ "strategy_config_id": cfg, "as_of": "2026-01-31" }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::INTERNAL_SERVER_ERROR);
    let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM recommendation_runs")
        .fetch_one(&h.member_pool().await)
        .await
        .unwrap();
    let jobs: i64 =
        sqlx::query_scalar("SELECT count(*) FROM jobs WHERE job_type = 'recommendation'")
            .fetch_one(&h.member_pool().await)
            .await
            .unwrap();
    assert_eq!(runs, 0, "queue failure may not leave a PENDING run");
    assert_eq!(jobs, 0, "queue failure rolls back the job too");
    h.teardown().await;
}

#[tokio::test]
async fn http_recommendations_capacity_is_per_owner_and_typed() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let cfg = config_id(&h, &h.member).await;
    h.seed_tenant(
        &h.member,
        &format!(
            "INSERT INTO jobs (owner_user_id, job_type, idempotency_key, payload_json) \
             SELECT '{owner}', 'other', 'occupied-' || gs::text, '{{}}'::jsonb \
             FROM generate_series(1, 10) AS gs",
            owner = h.member.user_id
        ),
    )
    .await;
    let resp = h
        .post(
            "/api/v1/recommendations/runs",
            Some(&h.member),
            true,
            json!({ "strategy_config_id": cfg, "as_of": "2026-01-31" }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        Harness::error_code(&Harness::body_json(resp).await),
        "RECOMMENDATION_CAPACITY_EXCEEDED"
    );
    let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM recommendation_runs")
        .fetch_one(&h.member_pool().await)
        .await
        .unwrap();
    assert_eq!(runs, 0);
    h.teardown().await;
}

#[tokio::test]
async fn http_recommendations_latest_keeps_success_visible_behind_new_pending_run() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let cfg = config_id(&h, &h.member).await;
    let first = h
        .send(
            "POST",
            "/api/v1/recommendations/runs",
            Some(&h.member),
            true,
            Some("test-rid-1"),
            Some("successful-rec"),
            Some(json!({ "strategy_config_id": cfg, "as_of": "2026-01-30" })),
        )
        .await;
    let successful_id = Harness::body_json(first).await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    h.seed_tenant(
        &h.member,
        &format!(
            "UPDATE recommendation_runs SET status = 'SUCCEEDED', summary_json = '{{\"selected\":1}}'::jsonb \
             WHERE id = '{successful_id}'"
        ),
    )
    .await;
    let newest = h
        .send(
            "POST",
            "/api/v1/recommendations/runs",
            Some(&h.member),
            true,
            Some("test-rid-1"),
            Some("pending-rec"),
            Some(json!({ "strategy_config_id": cfg, "as_of": "2026-01-31" })),
        )
        .await;
    let newest_id = Harness::body_json(newest).await["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let response = h
        .get(
            &format!("/api/v1/recommendations/latest?strategy_config_id={cfg}"),
            Some(&h.member),
        )
        .await;
    assert_eq!(status(&response), StatusCode::OK);
    let body = Harness::body_json(response).await;
    assert_eq!(body["run"]["id"], successful_id);
    assert_eq!(body["run"]["status"], "SUCCEEDED");
    assert_eq!(body["latest_run"]["id"], newest_id);
    assert_eq!(body["latest_run"]["status"], "PENDING");

    let first_page = h
        .get("/api/v1/recommendations/runs?limit=1", Some(&h.member))
        .await;
    let first_page = Harness::body_json(first_page).await;
    assert_eq!(
        first_page["items"][0]["id"], newest_id,
        "history is newest first"
    );
    let cursor = first_page["next_cursor"]
        .as_str()
        .expect("second page cursor");
    let second_page = h
        .get(
            &format!("/api/v1/recommendations/runs?limit=1&cursor={cursor}"),
            Some(&h.member),
        )
        .await;
    let second_page = Harness::body_json(second_page).await;
    assert_eq!(second_page["items"][0]["id"], successful_id);
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
