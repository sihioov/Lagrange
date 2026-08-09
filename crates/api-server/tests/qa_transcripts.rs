//! Todo 24 QA transcript: the full create -> queue -> result -> compare
//! happy path plus the stable error-code table, run over the real router
//! with correlation ids, and PRINTED (with `--nocapture`) for the evidence
//! bundle. Every assertion still holds; the printed lines are the HTTP
//! transcripts (status + body + X-Request-Id echo).

mod common;
use axum::http::StatusCode;
use common::{Harness, status};
use serde_json::{Value, json};

/// Print the exchange and return the parsed body.
async fn show(tag: &str, resp: axum::response::Response) -> Value {
    let rid = resp
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let status_code = resp.status();
    let body = Harness::body_json(resp).await;
    println!(
        "== {tag} => {status_code} X-Request-Id={rid}\n{}\n",
        serde_json::to_string_pretty(&body).unwrap()
    );
    body
}

#[tokio::test]
async fn http_qa_transcripts() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    println!(
        "### QA: Lagrange Station /api/v1 contract transcripts (correlation id = qa-rid-42)\n"
    );

    // --- happy: create -> queue -> result -> compare ------------------------
    let resp = h
        .send(
            "POST",
            "/api/v1/strategies/buy_and_hold/configs",
            Some(&h.member),
            true,
            Some("qa-rid-42"),
            Some("qa-config-1"),
            Some(json!({ "strategy_version": "1.0.0", "config": { "lookback": 200 }, "is_active": true })),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::CREATED);
    let cfg = show(
        "POST /api/v1/strategies/buy_and_hold/configs (create config)",
        resp,
    )
    .await;
    let cfg_id = cfg["id"].as_str().unwrap().to_string();

    let dataset: String =
        sqlx::query_scalar("SELECT id::text FROM dataset_versions WHERE status='READY' LIMIT 1")
            .fetch_one(&h.admin_pool)
            .await
            .unwrap();

    let backtest = json!({
        "strategy_config_id": cfg_id,
        "dataset_version_id": dataset,
        "start_date": "2026-01-05",
        "end_date": "2026-01-30",
        "initial_cash": { "currency": "KRW", "amount": "100000000" },
        "benchmark": "069500.KRX",
        "cost_profile_id": "KRX_ETF_DEFAULT",
        "execution_profile": "daily-close-next-open@1",
        "robustness": false
    });
    let resp = h
        .send(
            "POST",
            "/api/v1/backtests",
            Some(&h.member),
            true,
            Some("qa-rid-42"),
            Some("qa-backtest-1"),
            Some(backtest.clone()),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::CREATED);
    let run = show(
        "POST /api/v1/backtests (enqueue -> run PENDING + job QUEUED)",
        resp,
    )
    .await;
    let run_id = run["id"].as_str().unwrap().to_string();

    // The worker settles the run (simulated): SUCCEEDED + metrics + artifacts.
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
             (gen_random_uuid(), '{run_id}', '{owner}', 'EQUITY_CURVE', 'runs/{run_id}/equity.parquet', 20, repeat('a',64), 4096, '{{\"points\":[{{\"date\":\"2026-01-05\",\"equity\":\"100000000\"}},{{\"date\":\"2026-01-30\",\"equity\":\"101234000\"}}]}}'::jsonb)",
            owner = h.member.user_id
        ),
    )
    .await;

    let resp = h
        .send(
            "GET",
            &format!("/api/v1/backtests/{run_id}"),
            Some(&h.member),
            false,
            Some("qa-rid-42"),
            None,
            None,
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let _ = show("GET /api/v1/backtests/{run_id} (result)", resp).await;

    let resp = h
        .send(
            "GET",
            &format!("/api/v1/backtests/{run_id}/metrics"),
            Some(&h.member),
            false,
            Some("qa-rid-42"),
            None,
            None,
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let _ = show("GET /api/v1/backtests/{run_id}/metrics", resp).await;

    let resp = h
        .send(
            "GET",
            &format!("/api/v1/backtests/{run_id}/equity"),
            Some(&h.member),
            false,
            Some("qa-rid-42"),
            None,
            None,
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let _ = show("GET /api/v1/backtests/{run_id}/equity", resp).await;

    // A second run for compare.
    let mut backtest2 = backtest.clone();
    backtest2["start_date"] = json!("2026-01-06");
    let resp = h
        .send(
            "POST",
            "/api/v1/backtests",
            Some(&h.member),
            true,
            Some("qa-rid-42"),
            Some("qa-backtest-2"),
            Some(backtest2),
        )
        .await;
    let run2 = show("POST /api/v1/backtests (second run)", resp).await;
    let run2_id = run2["id"].as_str().unwrap().to_string();
    h.seed_tenant(
        &h.member,
        &format!(
            "UPDATE backtest_runs SET status='SUCCEEDED', summary_json='{{\"total_return\":\"0.1000\",\"cagr\":\"0.0400\",\"mdd\":\"-0.12\"}}'::jsonb, finished_at=now() WHERE id='{run2_id}'"
        ),
    )
    .await;

    let resp = h
        .send(
            "POST",
            "/api/v1/backtests/compare",
            Some(&h.member),
            true,
            Some("qa-rid-42"),
            None,
            Some(json!({ "run_ids": [run_id, run2_id] })),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let _ = show("POST /api/v1/backtests/compare (deltas)", resp).await;

    // --- stable error codes (typed 4xx, zero side effects) -------------------
    let blocked: String =
        sqlx::query_scalar("SELECT id::text FROM dataset_versions WHERE status='BLOCKED' LIMIT 1")
            .fetch_one(&h.admin_pool)
            .await
            .unwrap();
    let mut bad = backtest.clone();
    bad["dataset_version_id"] = json!(blocked);
    let resp = h
        .send(
            "POST",
            "/api/v1/backtests",
            Some(&h.member),
            true,
            Some("qa-rid-42"),
            Some("qa-blocked-1"),
            Some(bad),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::UNPROCESSABLE_ENTITY);
    let _ = show(
        "POST /api/v1/backtests (BLOCKED dataset -> DATASET_BLOCKED)",
        resp,
    )
    .await;

    let mut bad = backtest.clone();
    bad["initial_cash"] = json!({ "currency": "USD", "amount": "100000" });
    let resp = h
        .send(
            "POST",
            "/api/v1/backtests",
            Some(&h.member),
            true,
            Some("qa-rid-42"),
            Some("qa-ccy-1"),
            Some(bad),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::UNPROCESSABLE_ENTITY);
    let _ = show(
        "POST /api/v1/backtests (USD -> UNSUPPORTED_MARKET_CURRENCY)",
        resp,
    )
    .await;

    let mut bad = backtest.clone();
    bad["initial_cash"] = json!({ "currency": "KRW", "amount": "1e999" });
    let resp = h
        .send(
            "POST",
            "/api/v1/backtests",
            Some(&h.member),
            true,
            Some("qa-rid-42"),
            Some("qa-dec-1"),
            Some(bad),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::BAD_REQUEST);
    let _ = show("POST /api/v1/backtests (1e999 -> INVALID_DECIMAL)", resp).await;

    let resp = h
        .send(
            "POST",
            "/api/v1/backtests",
            Some(&h.member),
            true,
            Some("qa-rid-42"),
            Some("qa-cap-1"),
            Some(json!({ "blob": "x".repeat(300 * 1024) })),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::PAYLOAD_TOO_LARGE);
    let _ = show(
        "POST /api/v1/backtests (oversized -> PAYLOAD_TOO_LARGE)",
        resp,
    )
    .await;

    let resp = h
        .send(
            "POST",
            "/api/v1/backtests",
            Some(&h.member),
            true,
            Some("qa-rid-42"),
            Some("qa-idem-1"),
            Some(backtest.clone()),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::CREATED);
    let first = show(
        "POST /api/v1/backtests (idempotency key qa-idem-1, first call)",
        resp,
    )
    .await;
    let resp = h
        .send(
            "POST",
            "/api/v1/backtests",
            Some(&h.member),
            true,
            Some("qa-rid-42"),
            Some("qa-idem-1"),
            Some(backtest),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::CREATED);
    let is_replay = resp.headers().get("x-idempotent-replay").is_some();
    let second = show(
        "POST /api/v1/backtests (idempotency replay -> same run + X-Idempotent-Replay)",
        resp,
    )
    .await;
    assert!(is_replay, "replay header present");
    assert_eq!(first["id"], second["id"], "replay returns the same run");

    // --- ownership: the owner cannot touch the member's run ------------------
    let resp = h
        .send(
            "GET",
            &format!("/api/v1/backtests/{run_id}"),
            Some(&h.owner),
            false,
            Some("qa-rid-42"),
            None,
            None,
        )
        .await;
    assert_eq!(status(&resp), StatusCode::NOT_FOUND);
    let _ = show(
        "GET /api/v1/backtests/{run_id} as OWNER (ownership -> 404, indistinguishable)",
        resp,
    )
    .await;

    // --- entitlement gating (fail closed after revocation) --------------------
    h.seed_shared("UPDATE data_entitlements SET status='REVOKED' WHERE status='ACTIVE'")
        .await;
    let resp = h
        .send(
            "POST",
            "/api/v1/recommendations/runs",
            Some(&h.member),
            true,
            Some("qa-rid-42"),
            Some("qa-ent-1"),
            Some(json!({ "strategy_config_id": cfg_id, "as_of": "2026-01-31" })),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::FORBIDDEN);
    let _ = show(
        "POST /api/v1/recommendations/runs (REVOKED entitlement -> DATA_ENTITLEMENT_REQUIRED)",
        resp,
    )
    .await;

    // --- Phase 3 routes: Owner-only -------------------------------------------
    let resp = h
        .send(
            "POST",
            "/api/v1/admin/live/kill-switch/enable",
            Some(&h.member),
            true,
            Some("qa-rid-42"),
            Some("qa-live-1"),
            Some(json!({})),
        )
        .await;
    // Todo 37: a Member gets 404, not 403. A 403 would confirm the Live route
    // exists; to a Member it must be indistinguishable from a route that was
    // never built.
    assert_eq!(status(&resp), StatusCode::NOT_FOUND);
    let _ = show(
        "POST /api/v1/admin/live/kill-switch/enable as MEMBER (Owner-only -> NOT FOUND)",
        resp,
    )
    .await;

    let resp = h
        .send(
            "POST",
            "/api/v1/admin/live/kill-switch/enable",
            Some(&h.owner),
            true,
            Some("qa-rid-42"),
            Some("qa-live-2"),
            Some(json!({})),
        )
        .await;
    // Todo 37: the route IS implemented now, so an Owner is refused for the
    // real reason - this session carries no fresh MFA claim.
    assert_eq!(status(&resp), StatusCode::FORBIDDEN);
    let _ = show(
        "POST /api/v1/admin/live/kill-switch/enable as OWNER without fresh MFA (-> STEP_UP, audited)",
        resp,
    )
    .await;

    println!("### QA COMPLETE");
    h.teardown().await;
}
