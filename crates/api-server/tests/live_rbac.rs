//! Todo 37: the Owner-only Live boundary.
//!
//! Every test carries the `live_rbac_` prefix so the plan's literal acceptance
//! filter `cargo test -p api-server live_rbac` selects them.
//!
//! What is being proven is not "Members are hidden from Live" but "Live does
//! not exist for Members". A hidden route still answers; an absent one does
//! not. The distinction shows up as 404 rather than 403 — a 403 confirms the
//! path is real, which is the discovery the plan forbids.

mod common;

use axum::http::StatusCode;
use common::{Harness, UserCtx, status};
use serde_json::json;

fn conn_body(label: &str) -> serde_json::Value {
    json!({
        "label": label,
        "profile": "mock",
        "account_no_masked": "****6-01",
        "account_product_code": "01",
        "account_ref": "env:KIS_ACCOUNT_NO",
        "kis_app_key_ref": "env:KIS_APP_KEY",
        "kis_app_secret_ref": "file:/run/secrets/kis_app_secret",
    })
}

/// Owner WITH fresh MFA. The harness's owner is built without MFA claims, so
/// step-up-requiring routes need a session that carries them.
async fn owner_with_mfa(h: &Harness) -> UserCtx {
    h.seed_user_with_amr(
        auth::entitlement::Role::Owner,
        "owner-mfa@lagrange.test",
        "owner-iss",
        "owner-sub-mfa",
        &["pwd", "mfa"],
        chrono::Utc::now().timestamp(),
    )
    .await
}

// ---------------------------------------------------------------------------
// A Member must not be able to tell these routes exist.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn live_rbac_member_gets_404_not_403_on_every_live_route() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let m = h.member.clone();

    let reads = ["/api/v1/admin/live/connections"];
    for path in reads {
        let resp = h.get(path, Some(&m)).await;
        assert_eq!(
            status(&resp),
            StatusCode::NOT_FOUND,
            "{path} must be indistinguishable from a route that does not exist"
        );
        let body = Harness::body_json(resp).await;
        assert_eq!(Harness::error_code(&body), "RESOURCE_NOT_FOUND");
    }

    let writes = [
        ("/api/v1/admin/live/connections", conn_body("m")),
        (
            "/api/v1/admin/live/kill-switch/disable",
            json!({"reason": "member try"}),
        ),
    ];
    for (path, body) in writes {
        let resp = h.post(path, Some(&m), true, body).await;
        assert_eq!(
            status(&resp),
            StatusCode::NOT_FOUND,
            "{path} must 404 for a Member, never 403"
        );
    }
    h.teardown().await;
}

#[tokio::test]
async fn live_rbac_member_response_carries_no_live_fields() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let m = h.member.clone();
    let resp = h.get("/api/v1/admin/live/connections", Some(&m)).await;
    let body = Harness::body_json(resp).await;
    let rendered = body.to_string();
    // No connection, credential-reference, node, or kill-switch vocabulary may
    // leak through the refusal itself.
    for leak in [
        "kis_app_key_ref",
        "kis_app_secret_ref",
        "account_no_masked",
        "broker",
        "kill",
        "node",
    ] {
        assert!(
            !rendered.contains(leak),
            "a Member refusal leaked '{leak}': {rendered}"
        );
    }
    h.teardown().await;
}

// ---------------------------------------------------------------------------
// An Owner without fresh MFA is told precisely what is missing.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn live_rbac_owner_without_fresh_mfa_is_refused_with_a_step_up_code() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    // The baseline owner has no MFA claim.
    let resp = h
        .post(
            "/api/v1/admin/live/connections",
            Some(&h.owner),
            true,
            conn_body("no-mfa"),
        )
        .await;
    assert_eq!(
        status(&resp),
        StatusCode::FORBIDDEN,
        "an Owner may do this, just not without fresh MFA - so 403, not 404"
    );
    let code = Harness::error_code(&Harness::body_json(resp).await);
    assert!(
        code.starts_with("STEP_UP_"),
        "the refusal must name what is missing, got {code}"
    );
    h.teardown().await;
}

#[tokio::test]
async fn live_rbac_owner_with_stale_auth_is_refused() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    // MFA present but authenticated long ago: freshness is the point of
    // step-up, so an old MFA is not a fresh one.
    let stale = h
        .seed_user_with_amr(
            auth::entitlement::Role::Owner,
            "owner-stale@lagrange.test",
            "owner-iss",
            "owner-sub-stale",
            &["pwd", "mfa"],
            chrono::Utc::now().timestamp() - 86_400,
        )
        .await;
    let resp = h
        .post(
            "/api/v1/admin/live/connections",
            Some(&stale),
            true,
            conn_body("stale"),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::FORBIDDEN);
    assert_eq!(
        Harness::error_code(&Harness::body_json(resp).await),
        "STEP_UP_AUTH_TIME_STALE"
    );
    h.teardown().await;
}

// ---------------------------------------------------------------------------
// The happy path, and what it must never disclose.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn live_rbac_owner_with_fresh_mfa_configures_a_connection_holding_no_secret() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let o = owner_with_mfa(&h).await;
    let resp = h
        .post(
            "/api/v1/admin/live/connections",
            Some(&o),
            true,
            conn_body("simulator"),
        )
        .await;
    let st = status(&resp);
    let body = Harness::body_json(resp).await;
    assert_eq!(st, StatusCode::CREATED, "body={body}");

    // References are disclosed; values cannot be, because none are stored.
    assert_eq!(body["kis_app_key_ref"], "env:KIS_APP_KEY");
    assert_eq!(body["account_no_masked"], "****6-01");
    let rendered = body.to_string();
    assert!(!rendered.contains("50123456"), "{rendered}");

    // ...and the stored row genuinely holds no credential material.
    let stored: (String, String) =
        sqlx::query_as("SELECT app_key_ref, secret_ref FROM broker_connections LIMIT 1")
            .fetch_one(&h.admin_pool)
            .await
            .unwrap();
    assert!(stored.0.starts_with("env:"));
    assert!(stored.1.starts_with("file:/"));
    h.teardown().await;
}

#[tokio::test]
async fn live_rbac_a_pasted_secret_is_refused_before_it_is_stored() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let o = owner_with_mfa(&h).await;
    let mut body = conn_body("pasted");
    body["kis_app_secret_ref"] = json!("PSabcdef0123456789");

    let resp = h
        .post("/api/v1/admin/live/connections", Some(&o), true, body)
        .await;
    assert_eq!(
        status(&resp),
        StatusCode::BAD_REQUEST,
        "a raw secret must never be accepted, even by an authorised Owner"
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM broker_connections")
        .fetch_one(&h.admin_pool)
        .await
        .unwrap();
    assert_eq!(count, 0, "nothing may be stored when the shape is refused");
    h.teardown().await;
}

// ---------------------------------------------------------------------------
// The kill switch and one-node-per-account.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn live_rbac_the_kill_switch_is_engaged_by_default_and_blocks_start() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let o = owner_with_mfa(&h).await;
    let created = Harness::body_json(
        h.post(
            "/api/v1/admin/live/connections",
            Some(&o),
            true,
            conn_body("sim"),
        )
        .await,
    )
    .await;
    let cid = created["id"].as_str().expect("id").to_string();

    // A fresh install, a restored backup, and a failed migration all land in
    // the safe state: Live is OFF until switched on deliberately.
    let resp = h
        .post(
            &format!("/api/v1/admin/live/connections/{cid}/start"),
            Some(&o),
            true,
            json!({}),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::CONFLICT);
    assert_eq!(
        Harness::error_code(&Harness::body_json(resp).await),
        "LIVE_KILL_SWITCH_ENGAGED"
    );

    let nodes: i64 = sqlx::query_scalar("SELECT count(*) FROM broker_nodes")
        .fetch_one(&h.admin_pool)
        .await
        .unwrap();
    assert_eq!(nodes, 0, "a blocked start must create no node");
    h.teardown().await;
}

#[tokio::test]
async fn live_rbac_only_one_node_per_connection_can_run() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let o = owner_with_mfa(&h).await;
    let created = Harness::body_json(
        h.post(
            "/api/v1/admin/live/connections",
            Some(&o),
            true,
            conn_body("sim"),
        )
        .await,
    )
    .await;
    let cid = created["id"].as_str().expect("id").to_string();

    // Disengage the kill switch first — itself an Owner + fresh-MFA action.
    let resp = h
        .post(
            "/api/v1/admin/live/kill-switch/disable",
            Some(&o),
            true,
            json!({"reason": "test drill"}),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);

    let first = h
        .post(
            &format!("/api/v1/admin/live/connections/{cid}/start"),
            Some(&o),
            true,
            json!({}),
        )
        .await;
    assert_eq!(status(&first), StatusCode::CREATED);

    // Two nodes on one account double every order it places.
    let second = h
        .post(
            &format!("/api/v1/admin/live/connections/{cid}/start"),
            Some(&o),
            true,
            json!({}),
        )
        .await;
    assert_eq!(status(&second), StatusCode::CONFLICT);
    let running: i64 =
        sqlx::query_scalar("SELECT count(*) FROM broker_nodes WHERE status <> 'STOPPED'")
            .fetch_one(&h.admin_pool)
            .await
            .unwrap();
    assert_eq!(running, 1, "exactly one active node per connection");
    h.teardown().await;
}

// ---------------------------------------------------------------------------
// Refusals are audited.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn live_rbac_a_refused_live_request_is_audited() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let m = h.member.clone();
    let rid = "live-rbac-audit-1";
    let resp = h
        .send(
            "POST",
            "/api/v1/admin/live/connections",
            Some(&m),
            true,
            Some(rid),
            Some("live-key-1"),
            Some(conn_body("denied")),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::NOT_FOUND);

    // A refused Live request is exactly the event an operator must be able to
    // see, so the refusal is audited even though the caller learns nothing.
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE correlation_id = $1 \
           AND action LIKE 'admin.live.%' AND reason = 'FORBIDDEN_NOT_OWNER'",
    )
    .bind(rid)
    .fetch_one(&h.admin_pool)
    .await
    .unwrap_or(0);
    assert_eq!(audited, 1, "the refusal must be on the audit trail");
    h.teardown().await;
}
