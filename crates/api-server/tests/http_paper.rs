//! Todo 24 Paper account routes: create (PAPER only), get, bind-strategy,
//! orders/positions/equity views; shared reads and write ownership;
//! idempotency; fuzz.

mod common;
use axum::http::StatusCode;
use common::{Harness, status};
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
            // Matches the daily_equity.cash seeded below, so the fixture's
            // opening cash_ledger deposit agrees with the ledger authority
            // and cash_reconciled reads true for the ordinary happy path.
            json!({ "name": "member-paper-1", "currency": "KRW", "initial_cash": "40000000" }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::CREATED);
    h.assert_rid_echo(&resp);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["account_type"], "PAPER");
    assert_eq!(body["name"], "member-paper-1");
    assert_eq!(body["status"], "ACTIVE");
    assert_eq!(body["initial_cash"], "40000000.0000");
    assert_eq!(body["cost_profile_id"], "KRX_ETF_DEFAULT");
    let account_id = body["id"].as_str().unwrap().to_string();
    assert_eq!(body["owner_user_id"], h.member.user_id.to_string());
    assert_eq!(body["can_manage"], true);

    // get.
    let resp = h
        .get(
            &format!("/api/v1/paper/accounts/{account_id}"),
            Some(&h.member),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["id"].as_str().unwrap(), account_id);

    // bind-strategy: config owned by the actor.
    let cfg_resp = h
        .send(
            "POST",
            "/api/v1/strategies/buy_and_hold/configs",
            Some(&h.member),
            true,
            Some("test-rid-1"),
            Some("seed-config-002"),
            Some(json!({ "strategy_version": "1.0.0", "config": { "lookback": 200 }, "is_active": true })),
        )
        .await;
    let cfg_id = Harness::body_json(cfg_resp).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let resp = h
        .send(
            "POST",
            &format!("/api/v1/paper/accounts/{account_id}/bind-strategy"),
            Some(&h.member),
            true,
            Some("test-rid-1"),
            Some("bind-strategy-001"),
            Some(json!({
                "strategy_config_id": cfg_id,
                "auto_apply_recommendations": true
            })),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["account_id"], account_id);
    assert_eq!(body["strategy_config_id"], cfg_id);
    assert_eq!(body["strategy_id"], "buy_and_hold");
    assert_eq!(body["strategy_version"], "1.0.0");
    assert_eq!(body["auto_apply_recommendations"], true);

    let binding_enabled: bool = sqlx::query_scalar(
        "SELECT auto_apply_recommendations FROM account_strategy_bindings \
         WHERE account_id = $1::uuid AND unbound_at IS NULL",
    )
    .bind(&account_id)
    .fetch_one(&h.member_pool().await)
    .await
    .unwrap();
    assert!(binding_enabled, "explicit opt-in must be persisted");

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
    // Dated TODAY (CURRENT_DATE), not a fixed past date: the account's
    // opening cash_ledger deposit is stamped `now()` by AccountRepo::create,
    // so an as-of reconciliation against a date before the account existed
    // would correctly find no ledger event yet and report unreconciled --
    // truthfully, not as a bug in the check.
    h.seed_tenant(
        &h.member,
        &format!(
            "INSERT INTO daily_equity (id, account_id, owner_user_id, trading_date, equity, cash, positions_value, currency) VALUES \
             (gen_random_uuid(), '{account_id}', '{owner}', CURRENT_DATE, 100123456.7890, 40000000.0000, 60123456.7890, 'KRW')",
            owner = h.member.user_id
        ),
    )
    .await;

    let resp = h
        .get(
            &format!("/api/v1/paper/accounts/{account_id}/orders"),
            Some(&h.member),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    let orders = body["items"].as_array().expect("orders");
    assert_eq!(orders.len(), 2);
    let buy = orders
        .iter()
        .find(|o| o["side"] == "BUY")
        .expect("buy order");
    assert_eq!(buy["quantity"], "100.0000");

    let resp = h
        .get(
            &format!("/api/v1/paper/accounts/{account_id}/positions"),
            Some(&h.member),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    let positions = body["items"].as_array().expect("positions");
    assert_eq!(positions.len(), 1);
    assert_eq!(positions[0]["quantity"], "60.0000");

    let resp = h
        .get(
            &format!("/api/v1/paper/accounts/{account_id}/equity"),
            Some(&h.member),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    let points = body["items"].as_array().expect("equity points");
    assert_eq!(points[0]["equity"], "100123456.7890");
    assert_eq!(
        points[0]["cash_reconciled"], true,
        "the seeded daily_equity.cash matches the account's opening cash_ledger deposit"
    );
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
            json!({ "name": "member-paper-2", "currency": "KRW", "initial_cash": "10000000" }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::CREATED);
    let account_id = Harness::body_json(resp).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    h.seed_tenant(
        &h.member,
        &format!(
            "INSERT INTO orders (id, account_id, owner_user_id, order_ref, instrument_id, side, quantity, price, status, submitted_at) VALUES \
             (gen_random_uuid(), '{account_id}', '{owner}', 'shared-order-1', '069500.KRX', 'BUY', 10.0000, 10150.0000, 'FILLED', now())",
            owner = h.member.user_id
        ),
    )
    .await;

    // Every invited user can read the member's account.
    let resp = h
        .get(
            &format!("/api/v1/paper/accounts/{account_id}"),
            Some(&h.owner),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["owner_user_id"], h.member.user_id.to_string());
    assert_eq!(body["can_manage"], false);

    let resp = h.get("/api/v1/paper/accounts", Some(&h.owner)).await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    let shared = body["items"]
        .as_array()
        .expect("accounts")
        .iter()
        .find(|account| account["id"] == account_id)
        .expect("member account visible in invite-group list");
    assert_eq!(shared["can_manage"], false);

    let resp = h
        .get(
            &format!("/api/v1/paper/accounts/{account_id}/orders"),
            Some(&h.owner),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["items"][0]["order_ref"], "shared-order-1");

    // Shared visibility does not grant write access.
    let resp = h
        .post(
            &format!("/api/v1/paper/accounts/{account_id}/bind-strategy"),
            Some(&h.owner),
            true,
            json!({
                "strategy_config_id": "00000000-0000-4000-8000-000000000001",
                "auto_apply_recommendations": false
            }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::NOT_FOUND);

    // A LIVE account cannot be created through the API.
    let resp = h
        .post(
            "/api/v1/paper/accounts",
            Some(&h.owner),
            true,
            json!({ "name": "live-account", "currency": "KRW", "initial_cash": "10000000", "account_type": "LIVE" }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::BAD_REQUEST);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "INVALID_PARAMETER");

    // Idempotent creation replay.
    let key = "paper-acct-001";
    let b = json!({ "name": "member-paper-3", "currency": "KRW", "initial_cash": "10000000" });
    let r1 = h
        .send(
            "POST",
            "/api/v1/paper/accounts",
            Some(&h.member),
            true,
            Some("test-rid-1"),
            Some(key),
            Some(b.clone()),
        )
        .await;
    assert_eq!(r1.status(), StatusCode::CREATED);
    let id1 = Harness::body_json(r1).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let r2 = h
        .send(
            "POST",
            "/api/v1/paper/accounts",
            Some(&h.member),
            true,
            Some("test-rid-1"),
            Some(key),
            Some(b),
        )
        .await;
    assert_eq!(r2.status(), StatusCode::CREATED);
    assert_eq!(r2.headers()["x-idempotent-replay"], "true");
    assert_eq!(Harness::body_json(r2).await["id"].as_str().unwrap(), id1);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM accounts WHERE name='member-paper-3'")
            .fetch_one(&h.member_pool().await)
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
            json!({ "name": "x", "currency": "JPY", "initial_cash": "10000000" }),
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
            json!({ "name": "dup-name", "currency": "KRW", "initial_cash": "10000000" }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::CREATED);
    let resp = h
        .post(
            "/api/v1/paper/accounts",
            Some(&h.member),
            true,
            json!({ "name": "dup-name", "currency": "KRW", "initial_cash": "10000000" }),
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
            json!({ "name": "", "currency": "KRW", "initial_cash": "10000000" }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::BAD_REQUEST);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "INVALID_PARAMETER");
    h.teardown().await;
}

/// `daily_equity.cash` that disagrees with `cash_ledger` is served flagged,
/// not silently passed through as if it agreed.
///
/// `repos::accounts` states the rule this system runs on: "current cash is
/// never cached here -- it is always derived by replaying `cash_ledger`".
/// `daily_equity.cash` is exactly such a cache, and until now nothing checked
/// it against the authority before serving it to a caller.
/// The actual comparison arm: a ledger event EXISTS as of this date, and it
/// disagrees with the stored figure.
///
/// This must be dated `CURRENT_DATE`, not a fixed past date. `daily_equity`'s
/// own `cash_reconciled` is `(cl.balance IS NOT NULL AND cl.balance = de.cash)`
/// (`repos::paper.rs`), and `AccountRepo::create` stamps the opening deposit
/// with the column default `now()`. A row dated in the past would find NO
/// ledger event at all (`cl.balance IS NULL`) and read `false` for that
/// reason alone -- which would make this test pass even if the equality
/// comparison were deleted entirely. Dating it today is what makes the
/// inequality itself the thing under test.
#[tokio::test]
async fn http_paper_equity_flags_cash_that_disagrees_with_the_ledger() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let resp = h
        .post(
            "/api/v1/paper/accounts",
            Some(&h.member),
            true,
            json!({ "name": "paper-divergent", "currency": "KRW", "initial_cash": "10000000" }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::CREATED);
    let account_id = Harness::body_json(resp).await["id"]
        .as_str()
        .unwrap()
        .to_string();

    // The opening deposit is 10,000,000, stamped `now()`. This row claims
    // 40,000,000 for TODAY -- a real ledger event exists to compare against,
    // and it disagrees.
    h.seed_tenant(
        &h.member,
        &format!(
            "INSERT INTO daily_equity (id, account_id, owner_user_id, trading_date, equity, cash, positions_value, currency) VALUES \
             (gen_random_uuid(), '{account_id}', '{owner}', CURRENT_DATE, 40000000.0000, 40000000.0000, 0.0000, 'KRW')",
            owner = h.member.user_id
        ),
    )
    .await;

    let resp = h
        .get(
            &format!("/api/v1/paper/accounts/{account_id}/equity"),
            Some(&h.member),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    let points = body["items"].as_array().expect("equity points");
    assert_eq!(points.len(), 1);
    // The wrong number is still SERVED -- FR-PAPER-003 requires the curve be
    // viewable -- but flagged rather than presented as settled.
    assert_eq!(points[0]["cash"], "40000000.0000");
    assert_eq!(
        points[0]["cash_reconciled"], false,
        "a stored cash figure that disagrees with cash_ledger, when a ledger event exists to \
         compare against, must not be served silently"
    );

    // The same flag reaches the performance view.
    let resp = h
        .get(
            &format!("/api/v1/paper/accounts/{account_id}/performance"),
            Some(&h.member),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    let points = body["points"].as_array().expect("performance points");
    assert_eq!(points[0]["cash_reconciled"], false);

    h.teardown().await;
}

/// The OTHER way `cash_reconciled` can be `false`: no ledger event exists yet
/// as of the row's date at all. Distinguished from the test above so the two
/// arms of `(cl.balance IS NOT NULL AND cl.balance = de.cash)` are each
/// pinned by a case that isolates it.
#[tokio::test]
async fn http_paper_equity_flags_a_snapshot_dated_before_any_ledger_event() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let resp = h
        .post(
            "/api/v1/paper/accounts",
            Some(&h.member),
            true,
            json!({ "name": "paper-predates-ledger", "currency": "KRW", "initial_cash": "10000000" }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::CREATED);
    let account_id = Harness::body_json(resp).await["id"]
        .as_str()
        .unwrap()
        .to_string();

    // The account's only cash_ledger row is stamped `now()` (today). A row
    // dated in the past predates it, so there is no balance to compare
    // against yet -- the NULL arm, not the inequality arm.
    h.seed_tenant(
        &h.member,
        &format!(
            "INSERT INTO daily_equity (id, account_id, owner_user_id, trading_date, equity, cash, positions_value, currency) VALUES \
             (gen_random_uuid(), '{account_id}', '{owner}', '2026-01-30', 10000000.0000, 10000000.0000, 0.0000, 'KRW')",
            owner = h.member.user_id
        ),
    )
    .await;

    let resp = h
        .get(
            &format!("/api/v1/paper/accounts/{account_id}/equity"),
            Some(&h.member),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    let points = body["items"].as_array().expect("equity points");
    assert_eq!(
        points[0]["cash_reconciled"], false,
        "a snapshot dated before the account's own ledger history has nothing to agree with yet"
    );

    h.teardown().await;
}
