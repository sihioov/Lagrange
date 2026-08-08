//! Todo 34 Phase 2 failure injection: the two faults nothing else covers.
//!
//! Design §17's injection list and §16's fail-closed table are mostly already
//! proven elsewhere and the Phase 2 suite re-runs those binaries rather than
//! duplicating them:
//!
//!   worker kill / OOM      -> job-queue `worker_death_orphans_attempt_and_requeues_once`,
//!                             `zombie_worker_cannot_settle_after_sweep`, `retry_exhaustion_ends_failed`
//!   duplicate events       -> job-queue `duplicate_idempotency_key_returns_same_job`,
//!                             portfolio-model ledger replay rejection
//!   out-of-order fills     -> portfolio-model `OutOfOrderEvent`
//!   Paper scheduler break  -> portfolio-model `a_crash_between_sells_and_buys_resumes_without_double_filling`
//!   artifact corruption    -> api-server `artifact_hash_mismatch_fails_closed`
//!   result integrity       -> result-model `publication_is_refused_after_a_failed_validation`
//!   notification outage    -> api-server `observability_notification_email_outage_records_failed_delivery`
//!   restore failure        -> scripts/backup/tests/test-restore-failures.*
//!
//! What had NO coverage, and is therefore here:
//!
//!   F1  DB 일시 장애 (design §17) — a transient outage must fail CLOSED while
//!       it lasts, must not report false health, and must recover with no lost
//!       or duplicated rows.
//!   F6  disk-full — a manifest row whose bytes never landed (the shape a full
//!       disk leaves behind) must never be served, and the refusal must be
//!       audited with its correlation id.
//!
//! The outage is injected through PostgreSQL itself (`CONNECTION LIMIT 0` plus
//! `pg_terminate_backend`) rather than by stopping a container: it is the same
//! failure the pool actually sees, it needs no Docker inside a Rust test, and
//! it is deterministic on every host.
//!
//! All tests carry the `failure_` prefix so the Phase 2 suite can select them.

mod common;

use axum::http::StatusCode;
use common::{Harness, status};
use sqlx::{PgPool, postgres::PgPoolOptions};

/// Superuser pool on the *maintenance* database. Connection-limit changes and
/// backend termination cannot be issued from inside the database they target.
async fn super_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(20))
        .connect(&url)
        .await
        .ok()
}

/// Take the harness database offline: refuse new connections and kill the
/// existing ones. This is what the API's pool experiences during a DB outage.
async fn cut_database(sp: &PgPool, db: &str) {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "ALTER DATABASE \"{db}\" WITH CONNECTION LIMIT 0"
    )))
    .execute(sp)
    .await
    .expect("connection limit set to 0");
    sqlx::query(
        "SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
         WHERE datname = $1 AND pid <> pg_backend_pid()",
    )
    .bind(db)
    .execute(sp)
    .await
    .expect("existing backends terminated");
}

/// Restore connectivity. The pool is expected to recover on its own.
async fn restore_database(sp: &PgPool, db: &str) {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "ALTER DATABASE \"{db}\" WITH CONNECTION LIMIT -1"
    )))
    .execute(sp)
    .await
    .expect("connection limit restored");
}

// ---------------------------------------------------------------------------
// F1 — DB 일시 장애: fail closed, no false health, clean recovery.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn failure_db_outage_fails_closed_then_recovers() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let Some(sp) = super_pool().await else {
        eprintln!("SKIP: superuser pool unavailable");
        h.teardown().await;
        return;
    };
    let member = h.member.clone();

    // --- healthy baseline ---------------------------------------------------
    let resp = h.get("/api/v1/strategies", Some(&member)).await;
    assert_eq!(status(&resp), StatusCode::OK, "baseline read succeeds");
    let baseline = Harness::body_json(resp).await["items"]
        .as_array()
        .expect("items")
        .len();

    // --- outage -------------------------------------------------------------
    cut_database(&sp, &h.db_name).await;

    let resp = h.get("/api/v1/strategies", Some(&member)).await;
    let code = status(&resp);
    assert!(
        code.is_server_error(),
        "a DB-backed read during an outage must fail closed, got {code}"
    );
    // Fail closed means NO payload, not an empty-but-successful one: an empty
    // 200 would read to a client as "you own no strategies", which is a
    // different and much worse claim than "the server could not answer".
    let body = Harness::body_json(resp).await;
    assert!(
        body.get("items").is_none(),
        "an outage response must not carry a result set: {body}"
    );

    // The process itself is alive. `/metrics` is deliberately DB-free, so
    // liveness stays truthful while readiness does not — reporting the whole
    // service down here would be as wrong as reporting it healthy.
    let resp = h.get("/api/v1/metrics", None).await;
    assert_eq!(
        status(&resp),
        StatusCode::OK,
        "the DB-free metrics surface stays up during a DB outage"
    );

    // --- recovery -----------------------------------------------------------
    restore_database(&sp, &h.db_name).await;

    // The pool reconnects on its own; allow a bounded number of attempts so a
    // slow reconnect does not read as a permanent failure.
    let mut recovered = false;
    for _ in 0..20 {
        let resp = h.get("/api/v1/strategies", Some(&member)).await;
        if status(&resp) == StatusCode::OK {
            let after = Harness::body_json(resp).await["items"]
                .as_array()
                .expect("items")
                .len();
            assert_eq!(
                after, baseline,
                "recovery must restore the SAME rows: no loss, no duplication"
            );
            recovered = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    assert!(recovered, "the API never recovered after the DB came back");

    h.teardown().await;
}

#[tokio::test]
async fn failure_db_outage_never_half_writes_a_mutation() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let Some(sp) = super_pool().await else {
        eprintln!("SKIP: superuser pool unavailable");
        h.teardown().await;
        return;
    };
    let member = h.member.clone();

    cut_database(&sp, &h.db_name).await;

    // A create attempted mid-outage must not appear to succeed.
    let resp = h
        .post(
            "/api/v1/paper/accounts",
            Some(&member),
            true,
            serde_json::json!({ "name": "outage-acct", "currency": "KRW", "initial_cash": "10000000" }),
        )
        .await;
    assert!(
        !status(&resp).is_success(),
        "a mutation during a DB outage must never report success"
    );

    restore_database(&sp, &h.db_name).await;

    // ...and must leave nothing behind once the DB is back. A partially
    // applied account (row without its opening cash_ledger entry) is exactly
    // the corruption design §16 forbids.
    let mut leftover = -1i64;
    for _ in 0..20 {
        match sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM accounts WHERE name = 'outage-acct'",
        )
        .fetch_one(&h.admin_pool)
        .await
        {
            Ok(n) => {
                leftover = n;
                break;
            }
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(250)).await,
        }
    }
    assert_eq!(
        leftover, 0,
        "a mutation refused during an outage must leave no row behind"
    );

    h.teardown().await;
}

// ---------------------------------------------------------------------------
// F6 — disk-full: a manifest row whose bytes never landed.
// ---------------------------------------------------------------------------

/// Seed a SUCCEEDED run plus an artifact manifest row, then remove the file so
/// the DB claims bytes the disk does not have. That is precisely the state a
/// full disk leaves behind: the manifest INSERT committed, the payload write
/// did not.
async fn seed_artifact_without_bytes(h: &Harness, actor: &common::UserCtx) -> String {
    let rel = "runs/disk-full/equity.parquet";
    let sha = h.write_artifact(rel, b"bytes-that-will-vanish");
    h.seed_tenant(
        actor,
        &format!(
            "INSERT INTO backtest_runs (id, owner_user_id, strategy_id, strategy_version, \
             dataset_version, engine_version, config_sha256, code_commit, status, summary_json) \
             VALUES (gen_random_uuid(), '{owner}', 'buy_and_hold', '1.0.0', \
             'krx_eod_bars@2026-01-01', '1.231.0', repeat('1',64), 'PENDING', 'SUCCEEDED', '{{}}'::jsonb)",
            owner = actor.user_id
        ),
    )
    .await;
    let run_id: String = sqlx::query_scalar(
        "SELECT id::text FROM backtest_runs WHERE owner_user_id = $1::uuid \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(actor.user_id.to_string())
    .fetch_one(&h.member_pool().await)
    .await
    .unwrap();
    h.seed_tenant(
        actor,
        &format!(
            "INSERT INTO result_artifacts (id, backtest_run_id, owner_user_id, artifact_type, \
             parquet_path, row_count, sha256, size_bytes, summary_json) \
             VALUES (gen_random_uuid(), '{run_id}', '{owner}', 'EQUITY_CURVE', '{rel}', 5, \
             '{sha}', 22, '{{}}'::jsonb)",
            owner = actor.user_id
        ),
    )
    .await;
    let artifact_id: String = sqlx::query_scalar(
        "SELECT id::text FROM result_artifacts WHERE backtest_run_id = $1::uuid LIMIT 1",
    )
    .bind(&run_id)
    .fetch_one(&h.member_pool().await)
    .await
    .unwrap();

    // The write that never completed.
    let _ = std::fs::remove_file(h.artifact_root.join(rel));
    artifact_id
}

#[tokio::test]
async fn failure_disk_full_artifact_is_never_served() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let member = h.member.clone();
    let artifact_id = seed_artifact_without_bytes(&h, &member).await;

    let resp = h
        .get(
            &format!("/api/v1/artifacts/{artifact_id}/download"),
            Some(&member),
        )
        .await;

    assert!(
        !status(&resp).is_success(),
        "an artifact whose bytes never landed must never be served"
    );
    // No internal redirect: issuing one would hand Nginx a path that does not
    // exist and turn a caught integrity failure into a confusing 404 from a
    // component that never checked anything.
    assert!(
        resp.headers().get("x-accel-redirect").is_none(),
        "a failed integrity check must not issue the internal redirect"
    );
    let body = Harness::body_json(resp).await;
    let code = Harness::error_code(&body);
    assert!(
        code == "RESULT_INTEGRITY_FAILED" || code == "RESOURCE_NOT_FOUND",
        "expected a typed fail-closed code, got {code}: {body}"
    );
    // The filesystem layout is never disclosed, even in the failure path.
    let rendered = body.to_string();
    assert!(
        !rendered.contains("/data/artifacts") && !rendered.contains("runs/disk-full"),
        "the failure response leaked an internal path: {rendered}"
    );

    h.teardown().await;
}

// ---------------------------------------------------------------------------
// Correlation-linked audit: a refusal must be traceable to its request.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn failure_refusal_is_audited_with_its_correlation_id() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let member = h.member.clone();
    let owner = h.owner.clone();

    // An Owner-only surface refused to a member: a real, audited refusal.
    let rid = "test-rid-correlation-1";
    let resp = h
        .send(
            "GET",
            "/api/v1/admin/notifications/deliveries",
            Some(&member),
            false,
            Some(rid),
            None,
            None,
        )
        .await;
    assert_eq!(status(&resp), StatusCode::FORBIDDEN, "member is refused");

    // The audit row must exist AND carry the request's correlation id, so an
    // operator can join the refusal to the request that caused it. Counting
    // audit rows alone would pass even if every one of them were unlinkable.
    let linked: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs \
         WHERE action = 'admin.notifications.deliveries.list' \
           AND correlation_id = $1",
    )
    .bind(rid)
    .fetch_one(&h.admin_pool)
    .await
    .unwrap_or(0);
    assert_eq!(
        linked, 1,
        "the refusal must be audited with the request's correlation id ({rid})"
    );

    // And the refusal reason is recorded, not just the fact of a denial.
    let reason: Option<String> =
        sqlx::query_scalar("SELECT reason FROM audit_logs WHERE correlation_id = $1 LIMIT 1")
            .bind(rid)
            .fetch_one(&h.admin_pool)
            .await
            .unwrap_or(None);
    assert!(
        reason.is_some_and(|r| !r.is_empty()),
        "an audited refusal must record why it was refused"
    );

    let _ = owner;
    h.teardown().await;
}
