//! Todo 24 admin / live / licensing-status / authorized-artifact routes:
//! Owner-only gating with audited denials, stable codes, no member exposure
//! of Phase 3 surfaces.

mod common;
use axum::http::StatusCode;
use common::{Harness, status};
use serde_json::json;

/// Seed an owner-owned artifact row + parent run; returns (run_id, artifact_id).
async fn seed_artifact(h: &Harness, actor: &common::UserCtx) -> (String, String) {
    h.seed_tenant(
        actor,
        &format!(
            "INSERT INTO backtest_runs (id, owner_user_id, strategy_id, strategy_version, dataset_version, engine_version, config_sha256, code_commit, status, summary_json) VALUES \
             (gen_random_uuid(), '{owner}', 'buy_and_hold', '1.0.0', 'krx_eod_bars@2026-01-01', '1.231.0', repeat('1',64), 'PENDING', 'SUCCEEDED', '{{}}'::jsonb)",
            owner = actor.user_id
        ),
    )
    .await;
    let run_id: String = sqlx::query_scalar(
        "SELECT id::text FROM backtest_runs WHERE owner_user_id = $1::uuid ORDER BY created_at DESC LIMIT 1",
    )
    .bind(actor.user_id.to_string())
    .fetch_one(&h.member_pool().await)
    .await
    .unwrap();
    h.seed_tenant(
        actor,
        &format!(
            "INSERT INTO result_artifacts (id, backtest_run_id, owner_user_id, artifact_type, parquet_path, row_count, sha256, size_bytes, summary_json) VALUES \
             (gen_random_uuid(), '{run_id}', '{owner}', 'EQUITY_CURVE', 'runs/{run_id}/equity.parquet', 5, repeat('2',64), 128, '{{\"points\":[{{\"date\":\"2026-01-05\",\"equity\":\"100000000\"}}]}}'::jsonb)",
            owner = actor.user_id
        ),
    )
    .await;
    let artifact_id: String = sqlx::query_scalar(
        "SELECT a.id::text FROM result_artifacts a WHERE a.backtest_run_id = $1::uuid LIMIT 1",
    )
    .bind(&run_id)
    .fetch_one(&h.member_pool().await)
    .await
    .unwrap();
    (run_id, artifact_id)
}

#[tokio::test]
async fn http_admin_jobs_and_workers_owner_only() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    // Member -> 403 FORBIDDEN (audited denial).
    let resp = h.get("/api/v1/admin/jobs", Some(&h.member)).await;
    assert_eq!(status(&resp), StatusCode::FORBIDDEN);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "FORBIDDEN");

    // Owner sees the (empty) queue.
    let resp = h.get("/api/v1/admin/jobs", Some(&h.owner)).await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    assert!(body["items"].is_array());

    // Seed a running claim and check the derived worker view.
    h.seed_tenant(
        &h.member,
        &format!(
            "INSERT INTO jobs (id, owner_user_id, job_type, status, locked_by, locked_at, payload_json) VALUES \
             (gen_random_uuid(), '{owner}', 'backtest', 'RUNNING', 'worker-1', now(), '{{}}'::jsonb)",
            owner = h.member.user_id
        ),
    )
    .await;
    let resp = h.get("/api/v1/admin/workers", Some(&h.owner)).await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    let workers = body["items"].as_array().expect("workers");
    assert_eq!(workers.len(), 1);
    assert_eq!(workers[0]["worker_id"], "worker-1");
    assert_eq!(workers[0]["active_job_count"], 1);
    h.teardown().await;
}

#[tokio::test]
async fn http_admin_retry_and_datasets() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    // Seed a FAILED job for the member.
    h.seed_tenant(
        &h.member,
        &format!(
            "INSERT INTO jobs (id, owner_user_id, job_type, status, error_code, error_message, payload_json) VALUES \
             (gen_random_uuid(), '{owner}', 'backtest', 'FAILED', 'RESULT_INTEGRITY_FAILED', 'integrity', '{{}}'::jsonb)",
            owner = h.member.user_id
        ),
    )
    .await;
    let job_id: String = sqlx::query_scalar(
        "SELECT id::text FROM jobs WHERE owner_user_id=$1::uuid AND status='FAILED' LIMIT 1",
    )
    .bind(h.member.user_id.to_string())
    .fetch_one(&h.member_pool().await)
    .await
    .unwrap();

    // Member cannot retry.
    let resp = h
        .send(
            "POST",
            &format!("/api/v1/admin/jobs/{job_id}/retry"),
            Some(&h.member),
            true,
            Some("test-rid-1"),
            Some("retry-member-denied"),
            Some(json!({})),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::FORBIDDEN);

    // Owner retries (cross-user, audited) -> job returns to QUEUED.
    let resp = h
        .send(
            "POST",
            &format!("/api/v1/admin/jobs/{job_id}/retry"),
            Some(&h.owner),
            true,
            Some("test-rid-1"),
            Some("retry-owner-001"),
            Some(json!({})),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["status"], "QUEUED");
    let state: String = sqlx::query_scalar("SELECT status FROM jobs WHERE id=$1::uuid")
        .bind(&job_id)
        .fetch_one(&h.member_pool().await)
        .await
        .unwrap();
    assert_eq!(state, "QUEUED");

    // Datasets view (owner).
    let resp = h.get("/api/v1/admin/datasets", Some(&h.owner)).await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    let items = body["items"].as_array().expect("datasets");
    assert_eq!(items.len(), 3);
    let blocked = items
        .iter()
        .find(|d| d["status"] == "BLOCKED")
        .expect("blocked dataset present");
    assert!(!blocked["blocking_issues"].as_array().unwrap().is_empty());

    // Approve a BLOCKED dataset -> 422 DATASET_BLOCKED (needs a new version).
    let blocked_id = blocked["id"].as_str().unwrap();
    let resp = h
        .send(
            "POST",
            &format!("/api/v1/admin/datasets/{blocked_id}/approve"),
            Some(&h.owner),
            true,
            Some("test-rid-1"),
            Some("approve-blocked-001"),
            Some(json!({})),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::UNPROCESSABLE_ENTITY);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "DATASET_BLOCKED");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("NEW dataset version")
    );

    // Approve a WARNING dataset -> approved verdict + audit row.
    let warning_id: String =
        sqlx::query_scalar("SELECT id::text FROM dataset_versions WHERE status='WARNING' LIMIT 1")
            .fetch_one(&h.member_pool().await)
            .await
            .unwrap();
    let resp = h
        .send(
            "POST",
            &format!("/api/v1/admin/datasets/{warning_id}/approve"),
            Some(&h.owner),
            true,
            Some("test-rid-1"),
            Some("approve-warning-001"),
            Some(json!({})),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["verdict"], "APPROVED");
    let audit: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE action='admin.dataset.approve' AND target_id=$1",
    )
    .bind(&warning_id)
    .fetch_one(&h.admin_pool)
    .await
    .unwrap();
    assert_eq!(audit, 1, "approval is audited");

    // Member approve -> 403.
    let resp = h
        .send(
            "POST",
            &format!("/api/v1/admin/datasets/{warning_id}/approve"),
            Some(&h.member),
            true,
            Some("test-rid-1"),
            Some("approve-member-denied"),
            Some(json!({})),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::FORBIDDEN);
    h.teardown().await;
}

#[tokio::test]
async fn http_admin_audit_logs_and_phase3_live_gating() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    // Member -> 403 on audit logs.
    let resp = h.get("/api/v1/admin/audit-logs", Some(&h.member)).await;
    assert_eq!(status(&resp), StatusCode::FORBIDDEN);
    // Owner -> 200 (empty list ok).
    let resp = h.get("/api/v1/admin/audit-logs", Some(&h.owner)).await;
    assert_eq!(status(&resp), StatusCode::OK);

    // Phase 3 live routes: Member -> 403 FORBIDDEN (never exposed).
    let resp = h
        .post(
            "/api/v1/admin/live/kill-switch/enable",
            Some(&h.member),
            true,
            json!({}),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::FORBIDDEN);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "FORBIDDEN");

    // Owner -> 501 NOT_IMPLEMENTED (Phase 3), audited.
    let resp = h
        .post(
            "/api/v1/admin/live/kill-switch/enable",
            Some(&h.owner),
            true,
            json!({}),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::NOT_IMPLEMENTED);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "NOT_IMPLEMENTED");
    assert!(body["error"]["details"]["phase"].is_string());
    let audit: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action LIKE 'admin.live.%'")
            .fetch_one(&h.admin_pool)
            .await
            .unwrap();
    assert!(audit >= 1, "Phase 3 attempts are audited");
    h.teardown().await;
}

#[tokio::test]
async fn http_licensing_status_fail_closed() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let resp = h.get("/api/v1/licensing-status", Some(&h.member)).await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["as_of"].as_str().unwrap().len(), 10);
    let datasets = body["datasets"].as_array().expect("datasets");
    let krx = datasets
        .iter()
        .find(|d| d["dataset_id"] == "krx_eod_bars" && d["use_kind"] == "backtest")
        .expect("krx backtest licensing row");
    assert_eq!(krx["state"], "ACTIVE");
    assert_eq!(krx["covered"], true);
    assert!(krx["effective_until"].is_string());

    // After revocation, the same surface reports fail-closed state.
    h.seed_shared("UPDATE data_entitlements SET status='REVOKED' WHERE status='ACTIVE'")
        .await;
    let resp = h.get("/api/v1/licensing-status", Some(&h.member)).await;
    let body = Harness::body_json(resp).await;
    let krx = body["datasets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["dataset_id"] == "krx_eod_bars" && d["use_kind"] == "backtest")
        .unwrap();
    assert_eq!(krx["state"], "REVOKED");
    assert_eq!(krx["covered"], false);
    h.teardown().await;
}

#[tokio::test]
async fn http_artifacts_authorized_download() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let (_run_id, artifact_id) = seed_artifact(&h, &h.member).await;

    // Metadata (owner of the parent run only).
    let resp = h
        .get(&format!("/api/v1/artifacts/{artifact_id}"), Some(&h.member))
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["artifact_type"], "EQUITY_CURVE");
    assert_eq!(body["row_count"], 5);
    assert_eq!(
        body["download_path"],
        format!("/api/v1/artifacts/{artifact_id}/download")
    );
    assert!(
        !body.to_string().contains("parquet_path"),
        "no filesystem path leak"
    );

    // The owner (foreign actor) cannot see it: 404.
    let resp = h
        .get(&format!("/api/v1/artifacts/{artifact_id}"), Some(&h.owner))
        .await;
    assert_eq!(status(&resp), StatusCode::NOT_FOUND);

    // Authorized download returns the inline payload.
    let resp = h
        .get(
            &format!("/api/v1/artifacts/{artifact_id}/download"),
            Some(&h.member),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["payload"]["points"][0]["equity"], "100000000");

    // Foreign download -> 404 (no bytes).
    let resp = h
        .get(
            &format!("/api/v1/artifacts/{artifact_id}/download"),
            Some(&h.owner),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::NOT_FOUND);
    h.teardown().await;
}

#[tokio::test]
async fn http_auth_session_routes() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    // GET /api/v1/auth/session -> actor identity (no DB model leak).
    let resp = h.get("/api/v1/auth/session", Some(&h.member)).await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["role"], "member");
    assert!(body["expires_at_secs"].is_number());

    // GET /api/v1/auth/csrf -> rotates the synchronizer token.
    let resp = h.get("/api/v1/auth/csrf", Some(&h.member)).await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    assert!(body["csrf_token"].as_str().unwrap().len() >= 32);

    // Owner step-up is fail-closed for DB-resolved sessions (no amr column).
    let resp = h.get("/api/v1/auth/step-up-check", Some(&h.owner)).await;
    assert_eq!(status(&resp), StatusCode::FORBIDDEN);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "STEP_UP_MFA_REQUIRED");

    // Logout (CSRF) revokes the session; the cookie no longer resolves.
    let resp = h
        .post("/api/v1/auth/logout", Some(&h.member), true, json!({}))
        .await;
    assert_eq!(status(&resp), StatusCode::NO_CONTENT);
    let resp = h.get("/api/v1/auth/session", Some(&h.member)).await;
    assert_eq!(status(&resp), StatusCode::UNAUTHORIZED);
    h.teardown().await;
}
