//! Todo 24 Paper account routes: create (PAPER only), get, bind-strategy,
//! orders/positions/equity views; ownership; idempotency; fuzz.

mod common;
use common::{Harness, status};
use axum::http::StatusCode;
use serde_json::json;

#[tokio::test]
async fn http_paper_accounts_happy() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    // Create a paper account (member).
    let resp = h
        .post(
            "/api/v1/paper/accounts",
            Some(&h.member),
            true,
            json!({ "name": "member-paper-1", "currency": "KRW" }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::CREATED);
    h.assert_rid_echo(&resp);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["account_type"], "PAPER");
    assert_eq!(body["name"], "member-paper-1");
    assert_eq!(body["status"], "ACTIVE");
    let account_id = body["id"].as_str().unwrap().to_string();
    assert!(!body.to_string().contains("owner_user_id"), "no tenant column leak");

    // get.
    let resp = h
        .get(&format!("/api/v1/paper/accounts/{account_id}"), Some(&h.member))
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["id"].as_str().unwrap(), account_id);

    // bind-strategy: config owned by the actor.
    let cfg_resp = h
        .post(
            "/api/v1/strategies/buy_and_hold/configs",
            Some(&h.member),
            true,
            json!({ "strategy_version": "1.0.0", "config": { "lookback": 200 }, "is_active": true }),
        )
        .await;
    let cfg_id = Harness::body_json(cfg_resp).await["id"].as_str().unwrap().to_string();
    let resp = h
        .post(
            &format!("/api/v1/paper/accounts/{account_id}/bind-strategy"),
            Some(&h.member),
            true,
            json!({ "strategy_config_id": cfg_id }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["account_id"], account_id);
    assert_eq!(body["strategy_config_id"], cfg_id);

    // Ledger views (seeded as the paper engine would write them).
    h.seed_tenant(
        &h.member,
        &format!(
            "INSERT INTO orders (id, account_id, owner_user_id, order_ref, instrument_id, side, quantity, price, status, submitted_at) VALUES \
             (gen_random_uuid(), '{account_id}', '{owner}', 'ord-1', '069500.KRX', 'BUY', 100.0000, 10150.0000, 'FILLED', now()), \
             (gen_random_uuid(), '{account_id}', '{owner}', 'ord-2', '069500.KRX', 'SELL', 40.0000, 10200.0000, 'FILLED', now())",
            owner = h.member.user_id
        ),
    )
    .await;
    h.seed_tenant(
        &h.member,
        &format!(
            "INSERT INTO positions (id, account_id, owner_user_id, instrument_id, quantity, avg_price, updated_at) VALUES \
             (gen_random_uuid(), '{account_id}', '{owner}', '069500.KRX', 60.0000, 10166.6667, now())",
            owner = h.member.user_id
        ),
    )
    .await;
    h.seed_tenant(
        &h.member,
        &format!(
            "INSERT INTO daily_equity (id, account_id, owner_user_id, trading_date, equity, cash, positions_value, currency) VALUES \
             (gen_random_uuid(), '{account_id}', '{owner}', '2026-01-30', 100123456.7890, 40000000.0000, 60123456.7890, 'KRW')",
            owner = h.member.user_id
        ),
    )
    .await;

    let resp = h
        .get(&format!("/api/v1/paper/accounts/{account_id}/orders"), Some(&h.member))
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    let orders = body["items"].as_array().expect("orders");
    assert_eq!(orders.len(), 2);
    assert_eq!(orders[0]["side"], "BUY");
    assert_eq!(orders[0]["quantity"], "100.0000");

    let resp = h
        .get(&format!("/api/v1/paper/accounts/{account_id}/positions"), Some(&h.member))
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    let positions = body["items"].as_array().expect("positions");
    assert_eq!(positions.len(), 1);
    assert_eq!(positions[0]["quantity"], "60.0000");

    let resp = h
        .get(&format!("/api/v1/paper/accounts/{account_id}/equity"), Some(&h.member))
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    let points = body["items"].as_array().expect("equity points");
    assert_eq!(points[0]["equity"], "100123456.7890");
    h.teardown().await;
}

#[tokio::test]
async fn http_paper_accounts_ownership_and_gating() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let resp = h
        .post(
            "/api/v1/paper/accounts",
            Some(&h.member),
            true,
            json!({ "name": "member-paper-2", "currency": "KRW" }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::CREATED);
    let account_id = Harness::body_json(resp).await["id"].as_str().unwrap().to_string();

    // The owner cannot read the member's account.
    let resp = h
        .get(&format!("/api/v1/paper/accounts/{account_id}"), Some(&h.owner))
        .await;
    assert_eq!(status(&resp), StatusCode::NOT_FOUND);

    // A LIVE account cannot be created through the API.
    let resp = h
        .post(
            "/api/v1/paper/accounts",
            Some(&h.owner),
            true,
            json!({ "name": "live-account", "currency": "KRW", "account_type": "LIVE" }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::BAD_REQUEST);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "INVALID_PARAMETER");

    // Idempotent creation replay.
    let key = "paper-acct-001";
    let b = json!({ "name": "member-paper-3", "currency": "KRW" });
    let r1 = h
        .send("POST", "/api/v1/paper/accounts", Some(&h.member), true, Some("test-rid-1"), Some(key), Some(b.clone()))
        .await;
    assert_eq!(r1.status(), StatusCode::CREATED);
    let id1 = Harness::body_json(r1).await["id"].as_str().unwrap().to_string();
    let r2 = h
        .send("POST", "/api/v1/paper/accounts", Some(&h.member), true, Some("test-rid-1"), Some(key), Some(b))
        .await;
    assert_eq!(r2.status(), StatusCode::CREATED);
    assert_eq!(r2.headers()["x-idempotent-replay"], "true");
    assert_eq!(Harness::body_json(r2).await["id"].as_str().unwrap(), id1);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM accounts WHERE name='member-paper-3'")
        .fetch_one(&h.app_pool)
        .await
        .unwrap();
    assert_eq!(count, 1, "idempotent replay must not double-create");
    h.teardown().await;
}

#[tokio::test]
async fn http_paper_accounts_fuzz_and_duplicate() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    // Bad currency -> UNSUPPORTED_MARKET_CURRENCY.
    let resp = h
        .post(
            "/api/v1/paper/accounts",
            Some(&h.member),
            true,
            json!({ "name": "x", "currency": "JPY" }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::UNPROCESSABLE_ENTITY);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "UNSUPPORTED_MARKET_CURRENCY");

    // Duplicate (owner, name) -> 409 DUPLICATE_RESOURCE (accounts_owner_name_key).
    let resp = h
        .post(
            "/api/v1/paper/accounts",
            Some(&h.member),
            true,
            json!({ "name": "dup-name", "currency": "KRW" }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::CREATED);
    let resp = h
        .post(
            "/api/v1/paper/accounts",
            Some(&h.member),
            true,
            json!({ "name": "dup-name", "currency": "KRW" }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::CONFLICT);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "DUPLICATE_RESOURCE");

    // Empty name -> INVALID_PARAMETER.
    let resp = h
        .post(
            "/api/v1/paper/accounts",
            Some(&h.member),
            true,
            json!({ "name": "", "currency": "KRW" }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::BAD_REQUEST);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "INVALID_PARAMETER");
    h.teardown().await;
}
