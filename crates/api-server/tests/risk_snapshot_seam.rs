//! The seam between the submission route and the Risk Gateway.
//!
//! This file exists because that seam had no test at all, and a defect lived
//! there: the route handed the gate `risk_gateway::testing::snapshot_all_green()`
//! -- a fixture whose own documentation is *"A snapshot that passes all twelve
//! checks"* -- with `instrument_id` and `correlation_id` overwritten, while the
//! order sent to the broker was built separately from the caller's request.
//!
//! Every gate suite passed throughout. They exercise the gate directly with
//! real snapshots, so they prove the gate works; none of them proves the
//! server ASKS IT ABOUT THE RIGHT ORDER. That is the only property here.

mod common;

use api_server::repos::reconciliation::ReconciliationRepo;
use api_server::risk_snapshot::{GateOrder, for_submission, limits_for, parse_side, side_str};
use common::{Harness, actor_pool};
use domain::{Price, Quantity};
use risk_gateway::snapshot::{
    Allowlisted, DataFreshness, IntentConflict, MarketSession, Side, StrategyPromotion,
};
use uuid::Uuid;

async fn seeded_account(h: &Harness) -> Uuid {
    let pool = actor_pool(&h.app_url, &h.owner.user_id.to_string(), 2).await;
    let account: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (owner_user_id, account_type, name, currency) \
         VALUES ($1, 'LIVE', 'seam-live', 'KRW') \
         ON CONFLICT (owner_user_id, name) DO UPDATE SET currency = EXCLUDED.currency \
         RETURNING id",
    )
    .bind(h.owner.user_id)
    .fetch_one(&pool)
    .await
    .expect("account");

    sqlx::query(
        "INSERT INTO daily_equity \
         (id, account_id, owner_user_id, trading_date, equity, cash, positions_value, currency) \
         VALUES (gen_random_uuid(), $1, $2, CURRENT_DATE, 10000000.0000, 4000000.0000, \
                 6000000.0000, 'KRW') \
         ON CONFLICT (account_id, trading_date) DO NOTHING",
    )
    .bind(account)
    .bind(h.owner.user_id)
    .execute(&pool)
    .await
    .expect("equity");

    sqlx::query(
        "INSERT INTO risk_limits (version, max_symbol_weight_bp, max_order_value, \
         max_daily_order_value, max_daily_loss, max_data_age_secs) \
         VALUES ('risk-limits-seam', 3000, 1000000, 5000000, 500000, 300) \
         ON CONFLICT (version) DO NOTHING",
    )
    .execute(&h.owner_pool)
    .await
    .expect("limits");

    account
}

fn order(account: Uuid, side: Side, qty: &str, price: &str) -> GateOrder {
    GateOrder {
        intent_ref: format!("intent-seam-{qty}-{price}"),
        account_id: account,
        instrument_id: "069500.KRX".into(),
        side,
        quantity: Quantity::parse(qty).expect("quantity"),
        price: Some(Price::parse(price).expect("price")),
        correlation_id: "corr-seam".into(),
    }
}

/// THE regression lock: the gate is asked about the order that was submitted.
///
/// A 9,700-unit sell at 10,250 shares nothing with the old fixture's 10-unit
/// buy at 7,250 except the instrument. Before this was fixed, every assertion
/// below failed -- and nothing in the repository noticed.
#[tokio::test]
async fn the_snapshot_describes_the_submitted_order() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let account = seeded_account(&h).await;
    let recon = ReconciliationRepo::new(h.app_pool.clone(), h.owner.actor(), h.owner.user_id);
    let submitted = order(account, Side::Sell, "9700", "10250");

    let snap = for_submission(
        &h.app_pool,
        &h.owner.actor(),
        &recon,
        None,
        Some(false),
        &submitted,
        1_800_000_000,
    )
    .await
    .expect("snapshot builds");

    assert_eq!(snap.intent.side, Side::Sell, "a SELL was checked as a BUY");
    assert_eq!(snap.intent.quantity, submitted.quantity);
    assert_eq!(snap.intent.price, submitted.price);
    assert_eq!(snap.intent.instrument_id, submitted.instrument_id);
    assert_eq!(snap.intent.intent_ref, submitted.intent_ref);
    assert_eq!(snap.intent.account_id, account.to_string());

    // The account the checks divide by is this account, not a fabricated one.
    // The fixture's equity was 1,000,000; the seeded account holds 10,000,000.
    assert_eq!(snap.account.equity.amount().to_string(), "10000000.0000");
    assert_eq!(snap.account.available_cash.amount().to_string(), "4000000.0000");

    h.teardown().await;
}

/// Unwired inputs deny. They are not quietly assumed green.
///
/// `checks.rs`: "Every `Unknown` input denies with `InputUnavailable`. §16
/// requires missing inputs to deny." Four inputs have no source yet, and this
/// pins that they stay closed until one exists -- the fixture asserted the
/// opposite for all four.
#[tokio::test]
async fn inputs_without_a_source_stay_closed() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let account = seeded_account(&h).await;
    let recon = ReconciliationRepo::new(h.app_pool.clone(), h.owner.actor(), h.owner.user_id);

    let snap = for_submission(
        &h.app_pool,
        &h.owner.actor(),
        &recon,
        None,
        Some(false),
        &order(account, Side::Buy, "10", "7250"),
        1_800_000_000,
    )
    .await
    .expect("snapshot builds");

    assert_eq!(snap.market_session, MarketSession::Unknown);
    assert_eq!(snap.data_freshness, DataFreshness::Unknown);
    assert_eq!(snap.strategy_promotion, StrategyPromotion::Unknown);
    assert_eq!(snap.instrument_allowed, Allowlisted::Unknown);
    assert_eq!(snap.conflict, IntentConflict::Unknown);

    // Which means even the fixture's own order is now refused.
    let decision = risk_gateway::evaluate(&snap, &limits_for(&h.app_pool, &h.owner.actor(), h.owner.user_id).await.expect("limits"));
    assert!(
        !decision.is_approved(),
        "an order with unsourced inputs must not be approved"
    );

    h.teardown().await;
}

/// The limits come from `risk_limits`, not from the test fixture.
#[tokio::test]
async fn limits_come_from_the_configured_table() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    seeded_account(&h).await;

    let limits = limits_for(&h.app_pool, &h.owner.actor(), h.owner.user_id)
        .await
        .expect("configured limits");
    assert_eq!(
        limits.version, "risk-limits-seam",
        "the version stamped on every recorded decision must name a row somebody configured"
    );
    assert_eq!(limits.max_symbol_weight_bp, 3000);

    h.teardown().await;
}

/// An unrecognised side has no third answer.
///
/// The old code resolved this with `eq_ignore_ascii_case("SELL")` and a bare
/// `else`, so each of these became a BUY.
#[test]
fn only_buy_and_sell_are_sides() {
    assert_eq!(parse_side("BUY"), Some(Side::Buy));
    assert_eq!(parse_side("sell"), Some(Side::Sell));
    assert_eq!(parse_side(" SELL "), Some(Side::Sell));
    for bad in ["SEL", "", "매도", "B", "SELLL", "0"] {
        assert_eq!(parse_side(bad), None, "{bad:?} must not resolve to a side");
    }
    assert_eq!(side_str(Side::Buy), "BUY");
    assert_eq!(side_str(Side::Sell), "SELL");
}
