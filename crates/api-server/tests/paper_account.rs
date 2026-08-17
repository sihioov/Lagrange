//! Todo 30: per-user Paper ownership with invite-group read access.
//!
//! `http_paper.rs` already proves account CRUD/ownership/idempotency; this
//! file proves the acceptance items unique to Todo 30: one ACTIVE binding
//! per account with immutable history, branching on strategy change
//! (FR-PAPER-004), a retired strategy refusing new bindings, invalid
//! currency/cash rejection, and entitlement loss pausing new bindings
//! without deleting history.

mod common;

use axum::http::StatusCode;
use common::{Harness, UserCtx};
use serde_json::json;

fn status(resp: &axum::http::Response<axum::body::Body>) -> StatusCode {
    resp.status()
}

async fn create_paper_account(h: &Harness, u: &UserCtx, name: &str, cash: &str) -> String {
    let resp = h
        .post(
            "/api/v1/paper/accounts",
            Some(u),
            true,
            json!({ "name": name, "currency": "KRW", "initial_cash": cash }),
        )
        .await;
    assert_eq!(
        status(&resp),
        StatusCode::CREATED,
        "account create must succeed"
    );
    Harness::body_json(resp).await["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn create_config(h: &Harness, u: &UserCtx, strategy_id: &str, key: &str) -> String {
    let config = match strategy_id {
        "trend_following" => json!({ "fast_ma": 20, "slow_ma": 60 }),
        _ => json!({ "lookback": 200 }),
    };
    let resp = h
        .send(
            "POST",
            &format!("/api/v1/strategies/{strategy_id}/configs"),
            Some(u),
            true,
            Some("test-rid-1"),
            Some(key),
            Some(json!({
                "strategy_version": "1.0.0",
                "config": config,
                "is_active": true,
            })),
        )
        .await;
    assert_eq!(
        status(&resp),
        StatusCode::CREATED,
        "config create must succeed for {key}"
    );
    Harness::body_json(resp).await["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn bind(
    h: &Harness,
    u: &UserCtx,
    account_id: &str,
    cfg_id: &str,
    key: &str,
) -> axum::response::Response {
    h.send(
        "POST",
        &format!("/api/v1/paper/accounts/{account_id}/bind-strategy"),
        Some(u),
        true,
        Some("test-rid-1"),
        Some(key),
        Some(json!({ "strategy_config_id": cfg_id })),
    )
    .await
}

// ---------------------------------------------------------------------------
// Two Members open separately managed accounts from the same shape.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn two_members_open_separate_accounts_with_the_same_strategy_and_cash() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let member2 = h
        .seed_user(
            auth::entitlement::Role::Member,
            "member2@lagrange.test",
            "member-iss",
            "member-sub-2",
        )
        .await;

    let acct1 = create_paper_account(&h, &h.member, "shared-shape", "10000000").await;
    let acct2 = create_paper_account(&h, &member2, "shared-shape", "10000000").await;
    assert_ne!(acct1, acct2);

    let cfg1 = create_config(&h, &h.member, "buy_and_hold", "m1-cfg").await;
    let cfg2 = create_config(&h, &member2, "buy_and_hold", "m2-cfg").await;
    let resp1 = bind(&h, &h.member, &acct1, &cfg1, "m1-bind").await;
    let resp2 = bind(&h, &member2, &acct2, &cfg2, "m2-bind").await;
    assert_eq!(status(&resp1), StatusCode::OK);
    assert_eq!(status(&resp2), StatusCode::OK);

    // member2 can inspect member1's account but cannot manage it.
    let resp = h
        .get(&format!("/api/v1/paper/accounts/{acct1}"), Some(&member2))
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["can_manage"], false);

    let resp = bind(&h, &member2, &acct1, &cfg2, "foreign-bind").await;
    assert_eq!(status(&resp), StatusCode::NOT_FOUND);

    let cash1: String =
        sqlx::query_scalar("SELECT amount::text FROM cash_ledger WHERE account_id = $1::uuid")
            .bind(&acct1)
            .fetch_one(&h.member_pool().await)
            .await
            .unwrap();
    assert_eq!(
        cash1, "10000000.0000",
        "the opening deposit seeds the shared ledger exactly"
    );
    h.teardown().await;
}

// ---------------------------------------------------------------------------
// One binding per account, immutable history, branch on strategy change.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rebinding_closes_the_old_binding_and_never_mixes_two_active_strategies() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let m = h.member.clone();
    let account_id = create_paper_account(&h, &m, "rebind-acct", "10000000").await;
    let cfg_a = create_config(&h, &m, "buy_and_hold", "cfg-a").await;
    let cfg_b = create_config(&h, &m, "trend_following", "cfg-b").await;

    let resp = bind(&h, &m, &account_id, &cfg_a, "bind-a").await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["strategy_id"], "buy_and_hold");

    // Branch: rebind to a different strategy on the SAME account.
    let resp = bind(&h, &m, &account_id, &cfg_b, "bind-b").await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["strategy_id"], "trend_following");

    // Exactly one active binding at any time; the history preserves both.
    let active: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM account_strategy_bindings WHERE account_id = $1::uuid AND unbound_at IS NULL",
    )
    .bind(&account_id)
    .fetch_one(&h.member_pool().await)
    .await
    .unwrap();
    assert_eq!(active, 1, "at most one ACTIVE binding per account, ever");

    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM account_strategy_bindings WHERE account_id = $1::uuid",
    )
    .bind(&account_id)
    .fetch_one(&h.member_pool().await)
    .await
    .unwrap();
    assert_eq!(
        total, 2,
        "the closed binding is never deleted -- immutable history"
    );

    let closed_strategy: String = sqlx::query_scalar(
        "SELECT strategy_id FROM account_strategy_bindings \
         WHERE account_id = $1::uuid AND unbound_at IS NOT NULL",
    )
    .bind(&account_id)
    .fetch_one(&h.member_pool().await)
    .await
    .unwrap();
    assert_eq!(
        closed_strategy, "buy_and_hold",
        "the ORIGINAL binding's strategy identity is never rewritten, only closed"
    );
    h.teardown().await;
}

// ---------------------------------------------------------------------------
// A Retired strategy can never gain a new binding.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_retired_strategy_refuses_a_new_binding() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let m = h.member.clone();
    let account_id = create_paper_account(&h, &m, "retired-acct", "10000000").await;
    // The config is created while the strategy is still Validated; it is
    // retired AFTERWARD, matching the real scenario a binding gate must
    // catch (a Member's stale config outliving the strategy's lifecycle).
    let cfg = create_config(&h, &m, "trend_following", "cfg-retired").await;
    h.seed_shared("UPDATE strategies SET state='Retired' WHERE id='trend_following'")
        .await;

    let resp = bind(&h, &m, &account_id, &cfg, "bind-retired").await;
    assert_eq!(status(&resp), StatusCode::UNPROCESSABLE_ENTITY);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "INVALID_PARAMETER");

    let bound: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM account_strategy_bindings WHERE account_id = $1::uuid",
    )
    .bind(&account_id)
    .fetch_one(&h.member_pool().await)
    .await
    .unwrap();
    assert_eq!(bound, 0, "a refused bind must create no history row");
    h.teardown().await;
}

// ---------------------------------------------------------------------------
// Invalid currency/cash amounts are typed rejections.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn zero_and_negative_initial_cash_are_rejected() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    for (name, cash) in [("zero-cash", "0"), ("negative-cash", "-1000")] {
        let resp = h
            .post(
                "/api/v1/paper/accounts",
                Some(&h.member),
                true,
                json!({ "name": name, "currency": "KRW", "initial_cash": cash }),
            )
            .await;
        assert!(
            status(&resp) == StatusCode::BAD_REQUEST
                || status(&resp) == StatusCode::UNPROCESSABLE_ENTITY,
            "{name} must be rejected, got {}",
            status(&resp)
        );
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM accounts WHERE name = $1")
            .bind(name)
            .fetch_one(&h.member_pool().await)
            .await
            .unwrap();
        assert_eq!(count, 0, "{name} must create nothing");
    }
    h.teardown().await;
}

// ---------------------------------------------------------------------------
// Entitlement loss pauses NEW bindings without deleting existing history.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn expired_entitlement_blocks_new_account_and_binding_but_keeps_existing_history() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let m = h.member.clone();
    let account_id = create_paper_account(&h, &m, "pre-expiry-acct", "10000000").await;
    let cfg = create_config(&h, &m, "buy_and_hold", "cfg-pre-expiry").await;
    let resp = bind(&h, &m, &account_id, &cfg, "bind-pre-expiry").await;
    assert_eq!(status(&resp), StatusCode::OK);

    h.seed_shared("UPDATE data_entitlements SET status='REVOKED' WHERE status='ACTIVE'")
        .await;

    // A brand-new account cannot be opened once entitlement is gone.
    let resp = h
        .post(
            "/api/v1/paper/accounts",
            Some(&m),
            true,
            json!({ "name": "post-expiry-acct", "currency": "KRW", "initial_cash": "10000000" }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::FORBIDDEN);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "DATA_ENTITLEMENT_REQUIRED");

    // A new binding on the EXISTING account is also paused.
    let cfg2 = create_config(&h, &m, "trend_following", "cfg-post-expiry").await;
    let resp = bind(&h, &m, &account_id, &cfg2, "bind-post-expiry").await;
    assert_eq!(status(&resp), StatusCode::FORBIDDEN);

    // The account row and the pre-expiry binding are both still intact --
    // "pause", never delete.
    let still_there: i64 = sqlx::query_scalar("SELECT count(*) FROM accounts WHERE id = $1::uuid")
        .bind(&account_id)
        .fetch_one(&h.member_pool().await)
        .await
        .unwrap();
    assert_eq!(still_there, 1, "the account row survives entitlement loss");
    let bindings: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM account_strategy_bindings WHERE account_id = $1::uuid",
    )
    .bind(&account_id)
    .fetch_one(&h.member_pool().await)
    .await
    .unwrap();
    assert_eq!(bindings, 1, "the pre-expiry binding is never deleted");
    h.teardown().await;
}
