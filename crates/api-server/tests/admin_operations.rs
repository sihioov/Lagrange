//! Todo 27 admin operations: eligibility-limited job retries (only FAILED
//! jobs with retry budget and unblocked inputs), the cross-user users view,
//! and the append-only audit guarantee (admins can VIEW audit history but
//! never MUTATE it — FR-ADM-001/002/003, NFR-SEC-007).
//!
//! All tests carry the `admin_` prefix so the plan acceptance filter
//! `cargo test -p api-server artifact admin observability` selects them.

mod common;

use axum::http::StatusCode;
use common::{Harness, status};
use serde_json::json;
use sqlx::Row;

/// Seed a job for `actor` with the given state; returns its id.
async fn seed_job(
    h: &Harness,
    actor: &common::UserCtx,
    status: &str,
    attempt_count: i32,
    max_attempts: i32,
    error_code: Option<&str>,
    payload: &str,
) -> String {
    h.seed_tenant(
        actor,
        &format!(
            "INSERT INTO jobs (id, owner_user_id, job_type, status, attempt_count, max_attempts, error_code, payload_json) VALUES \
             (gen_random_uuid(), '{owner}', 'backtest', '{status}', {attempt_count}, {max_attempts}, {err}, '{payload}'::jsonb)",
            owner = actor.user_id,
            err = error_code.map(|c| format!("'{c}'")).unwrap_or_else(|| "NULL".to_string()),
        ),
    )
    .await;
    sqlx::query_scalar::<_, String>(
        "SELECT id::text FROM jobs WHERE owner_user_id = $1::uuid AND status = $2 \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(actor.user_id.to_string())
    .bind(status)
    .fetch_one(&h.member_pool().await)
    .await
    .unwrap()
}

async fn owner_retry(h: &Harness, job_id: &str) -> axum::http::Response<axum::body::Body> {
    let key = format!("retry-{}", uuid::Uuid::new_v4());
    h.send(
        "POST",
        &format!("/api/v1/admin/jobs/{job_id}/retry"),
        Some(&h.owner),
        true,
        Some("test-rid-1"),
        Some(&key),
        Some(json!({})),
    )
    .await
}

async fn job_state(h: &Harness, job_id: &str) -> (String, i32, String) {
    let row = sqlx::query(
        "SELECT status, attempt_count, coalesce(error_code,'') FROM jobs WHERE id=$1::uuid",
    )
    .bind(job_id)
    .fetch_one(&h.member_pool().await)
    .await
    .unwrap();
    (
        row.get::<String, _>(0),
        row.get::<i32, _>(1),
        row.get::<String, _>(2),
    )
}

#[tokio::test]
async fn admin_retry_eligible_failed_job_requeued_once() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let job_id = seed_job(&h, &h.member, "FAILED", 1, 3, Some("timeout"), "{}").await;
    let resp = owner_retry(&h, &job_id).await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["status"], "QUEUED");
    assert_eq!(body["attempt_count"], 0);
    let (state, attempts, _) = job_state(&h, &job_id).await;
    assert_eq!(state, "QUEUED");
    assert_eq!(attempts, 0, "retry resets the attempt budget");
    h.teardown().await;
}

#[tokio::test]
async fn admin_retry_ineligible_not_failed_denied() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    for state in ["QUEUED", "RUNNING", "SUCCEEDED", "CANCELED"] {
        let job_id = seed_job(&h, &h.member, state, 0, 3, None, "{}").await;
        let resp = owner_retry(&h, &job_id).await;
        assert_eq!(
            status(&resp),
            StatusCode::BAD_REQUEST,
            "job in state {state} must not be retryable"
        );
        let body = Harness::body_json(resp).await;
        assert_eq!(Harness::error_code(&body), "INVALID_PARAMETER");
        let (after, _, _) = job_state(&h, &job_id).await;
        assert_eq!(after, state, "state must not change");
    }
    h.teardown().await;
}

#[tokio::test]
async fn admin_retry_ineligible_exhausted_denied() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let job_id = seed_job(
        &h,
        &h.member,
        "FAILED",
        3,
        3,
        Some("attempts_exhausted"),
        "{}",
    )
    .await;
    let resp = owner_retry(&h, &job_id).await;
    assert_eq!(
        status(&resp),
        StatusCode::BAD_REQUEST,
        "an exhausted job must not be admin-retryable"
    );
    let (state, attempts, _) = job_state(&h, &job_id).await;
    assert_eq!(state, "FAILED");
    assert_eq!(attempts, 3, "attempts must stay exhausted");
    h.teardown().await;
}

#[tokio::test]
async fn admin_retry_ineligible_structural_blocked_denied() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    // A backtest job whose input dataset version is quality-BLOCKED cannot be
    // retried: the retry would fail identically (DATASET_BLOCKED).
    let blocked_id: String = sqlx::query_scalar(
        "SELECT id::text FROM dataset_versions WHERE status = 'BLOCKED' LIMIT 1",
    )
    .fetch_one(&h.member_pool().await)
    .await
    .unwrap();
    let payload = format!("{{\"dataset_version_id\": \"{blocked_id}\"}}");
    let job_id = seed_job(&h, &h.member, "FAILED", 1, 3, Some("timeout"), &payload).await;
    let resp = owner_retry(&h, &job_id).await;
    assert_eq!(status(&resp), StatusCode::UNPROCESSABLE_ENTITY);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "DATASET_BLOCKED");
    let (state, _, _) = job_state(&h, &job_id).await;
    assert_eq!(state, "FAILED", "blocked job must stay FAILED");
    h.teardown().await;
}

#[tokio::test]
async fn admin_retry_retryable_when_dataset_ready() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    // The same job shape with a READY input dataset is eligible.
    let ready_id: String =
        sqlx::query_scalar("SELECT id::text FROM dataset_versions WHERE status = 'READY' LIMIT 1")
            .fetch_one(&h.member_pool().await)
            .await
            .unwrap();
    let payload = format!("{{\"dataset_version_id\": \"{ready_id}\"}}");
    let job_id = seed_job(&h, &h.member, "FAILED", 1, 3, Some("timeout"), &payload).await;
    let resp = owner_retry(&h, &job_id).await;
    assert_eq!(status(&resp), StatusCode::OK);
    h.teardown().await;
}

#[tokio::test]
async fn admin_retry_success_and_denial_are_audited() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let job_id = seed_job(&h, &h.member, "FAILED", 1, 3, Some("timeout"), "{}").await;
    let resp = owner_retry(&h, &job_id).await;
    assert_eq!(status(&resp), StatusCode::OK);
    let denied_id = seed_job(&h, &h.member, "SUCCEEDED", 0, 3, None, "{}").await;
    let resp = owner_retry(&h, &denied_id).await;
    assert_eq!(status(&resp), StatusCode::BAD_REQUEST);

    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs \
         WHERE action = 'admin.job.retry' AND target_id = $1::text",
    )
    .bind(&job_id)
    .fetch_one(&h.admin_pool)
    .await
    .unwrap();
    assert!(count >= 1, "the successful retry must be audited");
    let denials: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs \
         WHERE action = 'admin.job.retry' AND target_id = $1::text AND reason IS NOT NULL",
    )
    .bind(&denied_id)
    .fetch_one(&h.admin_pool)
    .await
    .unwrap();
    assert!(
        denials >= 1,
        "the denied retry must be audited with a reason"
    );
    h.teardown().await;
}

#[tokio::test]
async fn admin_users_list_owner_only_with_audit() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    // Member -> 403 FORBIDDEN (audited denial).
    let resp = h.get("/api/v1/admin/users", Some(&h.member)).await;
    assert_eq!(status(&resp), StatusCode::FORBIDDEN);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "FORBIDDEN");

    // Owner sees every user (cross-user read-only view).
    let resp = h.get("/api/v1/admin/users", Some(&h.owner)).await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    let items = body["items"].as_array().expect("users items");
    let emails: Vec<String> = items
        .iter()
        .map(|u| u["email"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(
        emails.contains(&"owner@lagrange.test".to_string())
            && emails.contains(&"member@lagrange.test".to_string()),
        "admin users view must list both seeded users: {emails:?}"
    );
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE action = 'admin.users.list' AND actor_role = 'owner'",
    )
    .fetch_one(&h.admin_pool)
    .await
    .unwrap();
    assert!(audits >= 1, "the users list must be audited");
    h.teardown().await;
}

#[tokio::test]
async fn admin_audit_history_append_only_immutable() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    // Seed one audit row through the append-only writer role.
    sqlx::query(
        "INSERT INTO audit_logs (action, actor_role, reason, correlation_id) VALUES \
         ('admin.audit.probe', 'system', 'immutability probe', 'probe-1')",
    )
    .execute(&h.audit_pool)
    .await
    .expect("audit_writer inserts the probe row");
    // UPDATE/DELETE/TRUNCATE must fail with SQLSTATE 42501 for the app role
    // (actor-GUC'd) AND for the read-only admin role.
    for sql in [
        "UPDATE audit_logs SET reason = 'tampered'",
        "DELETE FROM audit_logs",
        "TRUNCATE audit_logs",
    ] {
        for pool in [&h.app_pool, &h.admin_pool] {
            let err = sqlx::query(sqlx::AssertSqlSafe(sql.to_string()))
                .execute(pool)
                .await
                .expect_err("audit mutation must be denied");
            let db = match err {
                sqlx::Error::Database(d) => d,
                _ => panic!("expected a database error for {sql}"),
            };
            assert_eq!(
                db.code().map(|c| c.to_string()),
                Some("42501".to_string()),
                "{sql} must be denied by privilege"
            );
        }
    }
    // The row is still intact and visible to the owner (read-only view).
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE correlation_id = 'probe-1'")
            .fetch_one(&h.admin_pool)
            .await
            .unwrap();
    assert_eq!(count, 1, "audit rows must survive every mutation attempt");
    h.teardown().await;
}
