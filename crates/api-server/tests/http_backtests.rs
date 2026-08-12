//! Todo 24 backtest orchestration routes: create (dataset/entitlement/
//! capacity gates, queue enqueue), cancel, metrics, equity, trades,
//! robustness, compare; idempotent replay; ownership; fuzz.

mod common;
use axum::http::StatusCode;
use common::{Harness, status};
use serde_json::json;
use uuid::Uuid;

fn backtest_request(dataset: &str) -> serde_json::Value {
    json!({
        "strategy_config_id": null,
        "dataset_version_id": dataset,
        "start_date": "2026-01-05",
        "end_date": "2026-01-30",
        "initial_cash": { "currency": "KRW", "amount": "100000000" },
        "benchmark": "069500.KRX",
        "cost_profile_id": "KRX_ETF_DEFAULT",
        "execution_profile": "daily-close-next-open@1",
        "robustness": false
    })
}

/// Seed a strategy config for `actor`; returns its id and the config JSON.
async fn seed_config(h: &Harness, actor: &common::UserCtx) -> String {
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
    Harness::body_json(resp).await["id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// The READY dataset_version id from the baseline.
async fn ready_dataset(h: &Harness) -> String {
    let id: Uuid =
        sqlx::query_scalar("SELECT id FROM dataset_versions WHERE status='READY' LIMIT 1")
            .fetch_one(&h.member_pool().await)
            .await
            .unwrap();
    id.to_string()
}

#[tokio::test]
async fn http_backtests_create_queue_result_compare_happy() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let cfg = seed_config(&h, &h.member).await;
    let dataset = ready_dataset(&h).await;
    let mut req = backtest_request(&dataset);
    req["strategy_config_id"] = json!(cfg);

    // create -> 201 PENDING + job QUEUED.
    let resp = h
        .post("/api/v1/backtests", Some(&h.member), true, req.clone())
        .await;
    assert_eq!(status(&resp), StatusCode::CREATED);
    h.assert_rid_echo(&resp);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["status"], "PENDING");
    assert_eq!(body["strategy_id"], "buy_and_hold");
    assert_eq!(body["benchmark"], "069500.KRX");
    let run_id = body["id"].as_str().unwrap().to_string();
    let job_id = body["job_id"].as_str().unwrap().to_string();
    let config_sha = body["config_sha256"].as_str().unwrap().to_string();
    assert_eq!(config_sha.len(), 64, "config hash is a real sha256 hex");
    assert!(
        !body.to_string().contains("owner_user_id"),
        "no tenant column leak"
    );

    let job_status: String = sqlx::query_scalar("SELECT status FROM jobs WHERE id = $1::uuid")
        .bind(&job_id)
        .fetch_one(&h.member_pool().await)
        .await
        .unwrap();
    assert_eq!(job_status, "QUEUED");

    // Worker completes the run: SUCCEEDED + metrics + artifacts.
    h.seed_tenant(
        &h.member,
        &format!(
            "UPDATE backtest_runs SET status='SUCCEEDED', summary_json='{{\"total_return\":\"0.1234\",\"cagr\":\"0.0512\",\"mdd\":\"-0.08\"}}'::jsonb, finished_at=now() WHERE id='{run_id}'"
        ),
    )
    .await;
    h.seed_tenant(
        &h.member,
        &format!(
            "INSERT INTO backtest_metrics (id, backtest_run_id, owner_user_id, metric_key, metric_value) VALUES \
             (gen_random_uuid(), '{run_id}', '{owner}', 'total_return', 0.1234), \
             (gen_random_uuid(), '{run_id}', '{owner}', 'sharpe', 1.2345)",
            owner = h.member.user_id
        ),
    )
    .await;
    h.seed_tenant(
        &h.member,
        &format!(
            "INSERT INTO result_artifacts (id, backtest_run_id, owner_user_id, artifact_type, parquet_path, row_count, sha256, size_bytes, summary_json) VALUES \
             (gen_random_uuid(), '{run_id}', '{owner}', 'EQUITY_CURVE', 'runs/{run_id}/equity.parquet', 20, repeat('f',64), 4096, '{{\"points\":[{{\"date\":\"2026-01-05\",\"equity\":\"100000000\"}},{{\"date\":\"2026-01-30\",\"equity\":\"101234000\"}}]}}'::jsonb), \
             (gen_random_uuid(), '{run_id}', '{owner}', 'FILLS', 'runs/{run_id}/fills.parquet', 3, repeat('b',64), 1024, '{{\"trades\":2}}'::jsonb), \
             (gen_random_uuid(), '{run_id}', '{owner}', 'ORDERS', 'runs/{run_id}/orders.parquet', 3, repeat('c',64), 1024, '{{\"orders\":2}}'::jsonb)",
            owner = h.member.user_id
        ),
    )
    .await;

    // get -> SUCCEEDED.
    let resp = h
        .get(&format!("/api/v1/backtests/{run_id}"), Some(&h.member))
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["status"], "SUCCEEDED");
    assert_eq!(body["summary"]["total_return"], "0.1234");

    // metrics.
    let resp = h
        .get(
            &format!("/api/v1/backtests/{run_id}/metrics"),
            Some(&h.member),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    let metrics = body["items"].as_array().expect("metrics items");
    assert_eq!(metrics.len(), 2);
    let total: Vec<_> = metrics
        .iter()
        .filter(|m| m["metric_key"] == "total_return")
        .collect();
    assert_eq!(total[0]["metric_value"], "0.1234");

    // equity (curve manifest + summary points).
    let resp = h
        .get(
            &format!("/api/v1/backtests/{run_id}/equity"),
            Some(&h.member),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["artifact"]["artifact_type"], "EQUITY_CURVE");
    assert_eq!(body["artifact"]["row_count"], 20);
    assert_eq!(body["summary"]["points"][1]["equity"], "101234000");
    assert!(
        !body.to_string().contains("parquet_path"),
        "no filesystem path leak"
    );
    assert!(
        !body.to_string().contains("storage_path"),
        "no internal path leak"
    );

    // trades (fills + orders manifests).
    let resp = h
        .get(
            &format!("/api/v1/backtests/{run_id}/trades"),
            Some(&h.member),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    let items = body["items"].as_array().expect("trades items");
    assert_eq!(items.len(), 2);

    // A second run for compare.
    let mut req2 = req.clone();
    req2["start_date"] = json!("2026-01-06");
    let resp = h
        .post("/api/v1/backtests", Some(&h.member), true, req2)
        .await;
    assert_eq!(status(&resp), StatusCode::CREATED);
    let run2 = Harness::body_json(resp).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    h.seed_tenant(
        &h.member,
        &format!(
            "UPDATE backtest_runs SET status='SUCCEEDED', summary_json='{{\"total_return\":\"0.1000\",\"cagr\":\"0.0400\",\"mdd\":\"-0.12\"}}'::jsonb, finished_at=now() WHERE id='{run2}'"
        ),
    )
    .await;

    // compare.
    let resp = h
        .post(
            "/api/v1/backtests/compare",
            Some(&h.member),
            true,
            json!({ "run_ids": [run_id, run2] }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["run_ids"].as_array().unwrap().len(), 2);
    assert_eq!(body["deltas"]["total_return"], "-0.0234");
    assert_eq!(body["runs"][0]["strategy_id"], "buy_and_hold");
    h.teardown().await;
}

#[tokio::test]
async fn http_backtests_dataset_blocked_and_stale_gate() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let cfg = seed_config(&h, &h.member).await;
    let blocked: String =
        sqlx::query_scalar("SELECT id::text FROM dataset_versions WHERE status='BLOCKED' LIMIT 1")
            .fetch_one(&h.member_pool().await)
            .await
            .unwrap();
    let mut req = backtest_request(&blocked);
    req["strategy_config_id"] = json!(cfg);
    let resp = h
        .post("/api/v1/backtests", Some(&h.member), true, req)
        .await;
    assert_eq!(status(&resp), StatusCode::UNPROCESSABLE_ENTITY);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "DATASET_BLOCKED");
    assert!(body["error"]["details"]["issues"].is_array());
    // No side effect.
    let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM backtest_runs")
        .fetch_one(&h.member_pool().await)
        .await
        .unwrap();
    assert_eq!(runs, 0);

    // WARNING dataset -> DATA_STALE (stale quality issue present).
    let warning: String =
        sqlx::query_scalar("SELECT id::text FROM dataset_versions WHERE status='WARNING' LIMIT 1")
            .fetch_one(&h.member_pool().await)
            .await
            .unwrap();
    let mut req = backtest_request(&warning);
    req["strategy_config_id"] = json!(cfg);
    let resp = h
        .post("/api/v1/backtests", Some(&h.member), true, req)
        .await;
    assert_eq!(status(&resp), StatusCode::UNPROCESSABLE_ENTITY);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "DATA_STALE");
    let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM backtest_runs")
        .fetch_one(&h.member_pool().await)
        .await
        .unwrap();
    assert_eq!(runs, 0, "stale dataset must not create runs");
    h.teardown().await;
}

#[tokio::test]
async fn http_backtests_capacity_limit_is_typed_429() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let cfg = seed_config(&h, &h.member).await;
    let dataset = ready_dataset(&h).await;
    // Fill the owner's queue up to the (default 10) limit.
    let owner_id = h.member.user_id;
    h.seed_tenant(
        &h.member,
        &format!(
            "INSERT INTO jobs (id, owner_user_id, job_type, status, payload_json) \
             SELECT gen_random_uuid(), '{owner_id}', 'backtest', 'QUEUED', '{{}}'::jsonb \
             FROM generate_series(1, 10)"
        ),
    )
    .await;
    let mut req = backtest_request(&dataset);
    req["strategy_config_id"] = json!(cfg);
    let resp = h
        .post("/api/v1/backtests", Some(&h.member), true, req)
        .await;
    assert_eq!(status(&resp), StatusCode::TOO_MANY_REQUESTS);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "BACKTEST_CAPACITY_EXCEEDED");
    let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM backtest_runs")
        .fetch_one(&h.member_pool().await)
        .await
        .unwrap();
    assert_eq!(runs, 0, "capacity denial must not create runs");
    h.teardown().await;
}

#[tokio::test]
async fn http_backtests_durable_idempotency_survives_api_restart() {
    let Some(mut h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let cfg = seed_config(&h, &h.member).await;
    let dataset = ready_dataset(&h).await;
    let mut request = backtest_request(&dataset);
    request["strategy_config_id"] = json!(cfg);
    let key = "backtest-durable-replay-001";

    let first = h
        .send(
            "POST",
            "/api/v1/backtests",
            Some(&h.member),
            true,
            Some("test-rid-1"),
            Some(key),
            Some(request.clone()),
        )
        .await;
    assert_eq!(first.status(), StatusCode::CREATED);
    let first_body = Harness::body_json(first).await;

    h.restart_api().await;
    let replay = h
        .send(
            "POST",
            "/api/v1/backtests",
            Some(&h.member),
            true,
            Some("test-rid-1"),
            Some(key),
            Some(request.clone()),
        )
        .await;
    assert_eq!(replay.status(), StatusCode::CREATED);
    let replay_body = Harness::body_json(replay).await;
    assert_eq!(replay_body["id"], first_body["id"]);
    assert_eq!(replay_body["job_id"], first_body["job_id"]);

    h.restart_api().await;
    request["start_date"] = json!("2026-01-06");
    let mismatch = h
        .send(
            "POST",
            "/api/v1/backtests",
            Some(&h.member),
            true,
            Some("test-rid-1"),
            Some(key),
            Some(request),
        )
        .await;
    assert_eq!(mismatch.status(), StatusCode::CONFLICT);
    assert_eq!(
        Harness::error_code(&Harness::body_json(mismatch).await),
        "IDEMPOTENCY_KEY_MISMATCH"
    );

    let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM backtest_runs")
        .fetch_one(&h.member_pool().await)
        .await
        .unwrap();
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE job_type='backtest'")
        .fetch_one(&h.member_pool().await)
        .await
        .unwrap();
    assert_eq!(runs, 1, "durable replay must not create another run");
    assert_eq!(jobs, 1, "durable replay must not enqueue another job");
    h.teardown().await;
}

#[tokio::test]
async fn http_backtests_validation_and_fuzz() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let cfg = seed_config(&h, &h.member).await;
    let dataset = ready_dataset(&h).await;

    // start > end -> INVALID_PARAMETER.
    let mut req = backtest_request(&dataset);
    req["strategy_config_id"] = json!(cfg);
    req["start_date"] = json!("2026-02-01");
    req["end_date"] = json!("2026-01-01");
    let resp = h
        .post("/api/v1/backtests", Some(&h.member), true, req)
        .await;
    assert_eq!(status(&resp), StatusCode::BAD_REQUEST);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "INVALID_PARAMETER");

    // Bad decimal string in initial_cash.
    let mut req = backtest_request(&dataset);
    req["strategy_config_id"] = json!(cfg);
    req["initial_cash"] = json!({ "currency": "KRW", "amount": "1e999" });
    let resp = h
        .post("/api/v1/backtests", Some(&h.member), true, req)
        .await;
    assert_eq!(status(&resp), StatusCode::BAD_REQUEST);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "INVALID_DECIMAL");

    // Unsupported currency.
    let mut req = backtest_request(&dataset);
    req["strategy_config_id"] = json!(cfg);
    req["initial_cash"] = json!({ "currency": "USD", "amount": "100000" });
    let resp = h
        .post("/api/v1/backtests", Some(&h.member), true, req)
        .await;
    assert_eq!(status(&resp), StatusCode::UNPROCESSABLE_ENTITY);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "UNSUPPORTED_MARKET_CURRENCY");

    // Foreign strategy config (owned by someone else) -> 404.
    let mut req = backtest_request(&dataset);
    req["strategy_config_id"] = json!(cfg);
    req["strategy_config_id"] = json!("00000000-0000-0000-0000-000000000001");
    let resp = h
        .post("/api/v1/backtests", Some(&h.member), true, req)
        .await;
    assert_eq!(status(&resp), StatusCode::NOT_FOUND);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "RESOURCE_NOT_FOUND");

    let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM backtest_runs")
        .fetch_one(&h.member_pool().await)
        .await
        .unwrap();
    assert_eq!(runs, 0, "fuzz must not create runs");
    h.teardown().await;
}

#[tokio::test]
async fn http_backtests_ownership_cancel_robustness_integrity() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let cfg = seed_config(&h, &h.member).await;
    let dataset = ready_dataset(&h).await;
    let mut req = backtest_request(&dataset);
    req["strategy_config_id"] = json!(cfg);
    let resp = h
        .post("/api/v1/backtests", Some(&h.member), true, req.clone())
        .await;
    assert_eq!(status(&resp), StatusCode::CREATED);
    let run_id = Harness::body_json(resp).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let job_id: String =
        sqlx::query_scalar("SELECT job_id::text FROM backtest_runs WHERE id=$1::uuid")
            .bind(&run_id)
            .fetch_one(&h.member_pool().await)
            .await
            .unwrap();

    // Ownership: the owner cannot read member's run (direct id guess).
    let resp = h
        .get(&format!("/api/v1/backtests/{run_id}"), Some(&h.owner))
        .await;
    assert_eq!(status(&resp), StatusCode::NOT_FOUND);
    let resp = h
        .post(
            &format!("/api/v1/backtests/{run_id}/cancel"),
            Some(&h.owner),
            true,
            json!({}),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::NOT_FOUND);

    // Cancel own run -> CANCEL_REQUESTED; job status flips to CANCELED.
    let resp = h
        .post(
            &format!("/api/v1/backtests/{run_id}/cancel"),
            Some(&h.member),
            true,
            json!({}),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["status"], "CANCEL_REQUESTED");
    let job_status: String = sqlx::query_scalar("SELECT status FROM jobs WHERE id = $1::uuid")
        .bind(&job_id)
        .fetch_one(&h.member_pool().await)
        .await
        .unwrap();
    assert_eq!(job_status, "CANCELED");

    // Robustness enqueue on a non-succeeded run -> INVALID_PARAMETER.
    let resp = h
        .post(
            &format!("/api/v1/backtests/{run_id}/robustness"),
            Some(&h.member),
            true,
            json!({}),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::BAD_REQUEST);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "INVALID_PARAMETER");

    // Result integrity: a SUCCEEDED run with NO equity artifact -> 422.
    let mut req2 = req.clone();
    req2["start_date"] = json!("2026-01-07");
    let resp = h
        .post("/api/v1/backtests", Some(&h.member), true, req2)
        .await;
    assert_eq!(status(&resp), StatusCode::CREATED);
    let run2 = Harness::body_json(resp).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    h.seed_tenant(
        &h.member,
        &format!("UPDATE backtest_runs SET status='SUCCEEDED' WHERE id='{run2}'"),
    )
    .await;
    let resp = h
        .get(&format!("/api/v1/backtests/{run2}/equity"), Some(&h.member))
        .await;
    assert_eq!(status(&resp), StatusCode::UNPROCESSABLE_ENTITY);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "RESULT_INTEGRITY_FAILED");
    h.teardown().await;
}

/// A cost_profile_id nobody can resolve is refused at submission.
///
/// This route used to write `cost_profile_id` into the job payload without
/// looking at it, so an unresolvable profile became a job that failed in a
/// worker minutes later -- and every backtest test in this repository had
/// settled on `krx-etf-default@2026-01`, a spelling that resolved nowhere,
/// precisely because nothing ever rejected it.
///
/// The absences are the point. A 400 that still left a `backtest_runs` row
/// behind would leak a PENDING run that no job will ever advance.
#[tokio::test]
async fn http_backtests_unknown_cost_profile_is_refused_with_no_side_effects() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let cfg = seed_config(&h, &h.member).await;
    let dataset = ready_dataset(&h).await;
    let pool = h.member_pool().await;

    let runs_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM backtest_runs")
        .fetch_one(&pool)
        .await
        .unwrap();
    let jobs_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs")
        .fetch_one(&pool)
        .await
        .unwrap();

    for bad in ["krx-etf-default@2026-01", "krx-etf-default", "NOPE"] {
        let mut req = backtest_request(&dataset);
        req["strategy_config_id"] = json!(cfg);
        req["cost_profile_id"] = json!(bad);
        let resp = h
            .post("/api/v1/backtests", Some(&h.member), true, req)
            .await;
        assert_eq!(
            status(&resp),
            StatusCode::BAD_REQUEST,
            "cost_profile_id {bad:?} should be refused"
        );
        let body = Harness::body_json(resp).await;
        assert_eq!(Harness::error_code(&body), "INVALID_PARAMETER");
    }

    let runs_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM backtest_runs")
        .fetch_one(&pool)
        .await
        .unwrap();
    let jobs_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        runs_before, runs_after,
        "a refused submission wrote a run row"
    );
    assert_eq!(jobs_before, jobs_after, "a refused submission queued a job");
    h.teardown().await;
}

/// `CUSTOM` is refused with its own code, not the generic unknown-id error.
///
/// "Never heard of it" and "known, not available yet" are different answers.
/// Collapsing them sends someone hunting for a typo that is not there.
#[tokio::test]
async fn http_backtests_custom_cost_profile_is_refused_as_unsupported() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let cfg = seed_config(&h, &h.member).await;
    let dataset = ready_dataset(&h).await;
    let mut req = backtest_request(&dataset);
    req["strategy_config_id"] = json!(cfg);
    req["cost_profile_id"] = json!("CUSTOM");

    let resp = h
        .post("/api/v1/backtests", Some(&h.member), true, req)
        .await;
    assert_eq!(status(&resp), StatusCode::BAD_REQUEST);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "UNSUPPORTED_COST_PROFILE");
    h.teardown().await;
}
