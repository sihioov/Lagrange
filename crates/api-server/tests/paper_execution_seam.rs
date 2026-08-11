//! The seam between a queued Paper target and the ledger it is supposed to
//! move (plan Todo 31/32; design §9.2, §10.2; requirements UC-04,
//! FR-PAPER-002/003).
//!
//! `portfolio-model` proves the session ARITHMETIC in memory and
//! `paper_notifications.rs` proves what a settled session TELLS the user. What
//! nothing proved is the connection: until this suite, no production code
//! wrote `orders`, `fills` or `positions` at all, and `cash_ledger` was written
//! exactly once, at account creation. Every component was green and the feature
//! did not exist.
//!
//! So these tests call the entry point a runner calls — `run_and_settle` — and
//! then read the ledger back THROUGH THE MEMBER'S OWN RLS context. That second
//! part is deliberate: the engine writes on a `worker` pool whose policies are
//! `USING (true)`, so a row stamped with the wrong `owner_user_id` would insert
//! happily and simply never be visible to the person who owns the account. A
//! count taken on the worker pool would not notice; these do.

mod common;

use std::path::PathBuf;

use axum::http::StatusCode;
use chrono::NaiveDate;
use common::{Harness, UserCtx};
use domain::{ContentHash, Currency, InstrumentId, Price, TradingDate, UtcTimestamp};
use market_data::CurateStore;
use market_data::curate::schema::{CuratedBar, write_bars};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use api_server::notify::AlertSeverity;
use api_server::paper_session::run_and_settle;
use api_server::repos::pending_targets::NewPendingTarget;
use job_queue::paper_execution::{
    ExecutionOutcome, SessionInput, execute_session, targets_from_json,
};
use job_queue::phase0::{CURATED_VERSION, DATASET_ID};

/// The close that produced the target.
const COMPUTED_ON: &str = "2020-01-20";
/// The session it executes at: opens 10,240 (069500) and 8,520 (229200).
const EFFECTIVE_DATE: &str = "2020-01-21";
/// A session the fixture deliberately has no bar for.
const UNPRICED_DATE: &str = "2020-01-22";

/// A curated zone holding exactly the two sessions these tests execute.
///
/// # Why this remains tiny
///
/// The seam keeps only two sessions for speed. Its values match corrected
/// Phase0 v2 and are written through the production schema at the active
/// curated partition version.
struct Dataset {
    /// Kept alive: dropping it deletes the zone.
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Dataset {
    fn root(&self) -> &std::path::Path {
        &self.root
    }
}

/// `(date, open, high, low, close, volume, trading_value)`.
type FixtureBar = (&'static str, i64, i64, i64, i64, i64, i64);

const BARS_069500: &[FixtureBar] = &[
    (
        "2020-01-20",
        10150,
        10280,
        10100,
        10250,
        1_200_000,
        12_300_000_000,
    ),
    (
        "2020-01-21",
        10240,
        10320,
        10190,
        10290,
        1_150_000,
        11_833_500_000,
    ),
];
const BARS_229200: &[FixtureBar] = &[
    ("2020-01-20", 8480, 8560, 8450, 8530, 900_000, 7_677_000_000),
    ("2020-01-21", 8520, 8600, 8500, 8580, 870_000, 7_464_600_000),
];

fn curated_fixture() -> Dataset {
    let dir = tempfile::tempdir().expect("temp dataset root");
    let root = dir.path().to_path_buf();
    // The runner is given the dataset root and the curated zone sits one level
    // in, exactly as the engine reaches it.
    let store = CurateStore::new(root.join("curated"));
    for (symbol, bars) in [("069500.KRX", BARS_069500), ("229200.KRX", BARS_229200)] {
        let rows: Vec<CuratedBar> = bars.iter().map(|b| curated_bar(symbol, b)).collect();
        write_bars(&store.bars_path("kr", symbol, 2020, CURATED_VERSION), &rows)
            .expect("curated bars write");
    }
    Dataset { _dir: dir, root }
}

fn curated_bar(symbol: &str, bar: &FixtureBar) -> CuratedBar {
    let (date, open, high, low, close, volume, trading_value) = *bar;
    let price = |krw: i64| Price::parse(&krw.to_string()).expect("positive price");
    let instant = |suffix: &str| {
        UtcTimestamp::parse_rfc3339(&format!("{date}T{suffix}")).expect("session instant")
    };
    CuratedBar {
        instrument_id: InstrumentId::parse(symbol).expect("instrument id"),
        trading_date: TradingDate::parse(date).expect("trading date"),
        // KRX opens 09:00 and closes 15:30 Asia/Seoul (+09:00, no DST).
        market_open_ts: instant("00:00:00Z"),
        market_close_ts: instant("06:30:00Z"),
        open: price(open),
        high: price(high),
        low: price(low),
        close: price(close),
        volume,
        trading_value: Some(trading_value),
        currency: Currency::KRW,
        source: "test".to_owned(),
        ingested_at: UtcTimestamp::parse_rfc3339("2020-02-10T00:00:00Z").expect("ingest instant"),
        batch_id: "00000000-0000-0000-0000-000000000001"
            .parse()
            .expect("batch id"),
        raw_hash: ContentHash::from_bytes(b"paper-execution-seam"),
    }
}

fn date(iso: &str) -> NaiveDate {
    NaiveDate::parse_from_str(iso, "%Y-%m-%d").expect("valid date")
}

fn targets_json() -> serde_json::Value {
    json!([
        { "instrument_id": "069500.KRX", "weight": "0.600000" },
        { "instrument_id": "229200.KRX", "weight": "0.400000" }
    ])
}

async fn paper_account(h: &Harness, u: &UserCtx, name: &str, cash: &str) -> Uuid {
    let resp = h
        .post(
            "/api/v1/paper/accounts",
            Some(u),
            true,
            json!({ "name": name, "currency": "KRW", "initial_cash": cash }),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::CREATED, "account create");
    let id = Harness::body_json(resp).await["id"]
        .as_str()
        .expect("account id")
        .to_string();
    Uuid::parse_str(&id).expect("account id is a uuid")
}

async fn strategy_config(h: &Harness, u: &UserCtx, key: &str) -> Uuid {
    let resp = h
        .send(
            "POST",
            "/api/v1/strategies/buy_and_hold/configs",
            Some(u),
            true,
            Some("test-rid-1"),
            Some(key),
            Some(json!({
                "strategy_version": "1.0.0",
                "config": { "lookback": 200 },
                "is_active": true,
            })),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::CREATED, "config create");
    let id = Harness::body_json(resp).await["id"]
        .as_str()
        .expect("config id")
        .to_string();
    Uuid::parse_str(&id).expect("config id is a uuid")
}

async fn queue_target(
    h: &Harness,
    u: &UserCtx,
    account: Uuid,
    config: Uuid,
    computed_on: &str,
    effective_date: &str,
    targets: serde_json::Value,
) -> Uuid {
    h.state_pending_targets()
        .queue(
            &u.actor(),
            NewPendingTarget {
                account_id: account,
                strategy_config_id: config,
                computed_on: date(computed_on),
                effective_date: date(effective_date),
                targets_json: targets,
                dataset_version: Some(DATASET_ID.to_owned()),
            },
        )
        .await
        .expect("target queues")
        .id
}

/// What the OWNER can see of their own ledger, read under their RLS context.
struct LedgerView {
    orders: Vec<(String, String, String, String, NaiveDate)>,
    fills: Vec<(String, String, String, String)>,
    positions: Vec<(String, String)>,
    cash: Vec<(i64, String, String, String)>,
}

async fn ledger_view(h: &Harness, u: &UserCtx, account: Uuid) -> LedgerView {
    let pool = common::actor_pool(&h.app_url, &u.user_id.to_string(), 2).await;

    let orders = sqlx::query(
        "SELECT order_ref, instrument_id, side, status, created_at::date AS on_date \
         FROM orders WHERE account_id = $1 ORDER BY order_ref",
    )
    .bind(account)
    .fetch_all(&pool)
    .await
    .expect("orders read")
    .into_iter()
    .map(|r| {
        (
            r.get::<String, _>("order_ref"),
            r.get::<String, _>("instrument_id"),
            r.get::<String, _>("side"),
            r.get::<String, _>("status"),
            r.get::<NaiveDate, _>("on_date"),
        )
    })
    .collect();

    let fills = sqlx::query(
        "SELECT f.fill_ref, f.instrument_id, f.price::text AS price, o.order_ref \
         FROM fills f JOIN orders o ON o.id = f.order_id \
         WHERE f.account_id = $1 ORDER BY f.fill_ref",
    )
    .bind(account)
    .fetch_all(&pool)
    .await
    .expect("fills read")
    .into_iter()
    .map(|r| {
        (
            r.get::<String, _>("fill_ref"),
            r.get::<String, _>("instrument_id"),
            r.get::<String, _>("price"),
            r.get::<String, _>("order_ref"),
        )
    })
    .collect();

    let positions = sqlx::query(
        "SELECT instrument_id, quantity::text AS quantity FROM positions \
         WHERE account_id = $1 ORDER BY instrument_id",
    )
    .bind(account)
    .fetch_all(&pool)
    .await
    .expect("positions read")
    .into_iter()
    .map(|r| {
        (
            r.get::<String, _>("instrument_id"),
            r.get::<String, _>("quantity"),
        )
    })
    .collect();

    let cash = sqlx::query(
        "SELECT seq, event_type, amount::text AS amount, balance::text AS balance \
         FROM cash_ledger WHERE account_id = $1 ORDER BY seq",
    )
    .bind(account)
    .fetch_all(&pool)
    .await
    .expect("cash read")
    .into_iter()
    .map(|r| {
        (
            r.get::<i64, _>("seq"),
            r.get::<String, _>("event_type"),
            r.get::<String, _>("amount"),
            r.get::<String, _>("balance"),
        )
    })
    .collect();

    LedgerView {
        orders,
        fills,
        positions,
        cash,
    }
}

fn krw(s: &str) -> f64 {
    s.parse()
        .unwrap_or_else(|e| panic!("unreadable KRW {s:?}: {e}"))
}

/// The self-check `risk_snapshot::account_state` refuses an account for
/// failing: the running balance and the replayed events must agree.
fn assert_cash_ledger_agrees_with_itself(cash: &[(i64, String, String, String)]) {
    assert!(
        !cash.is_empty(),
        "an account always has its opening deposit"
    );
    let replayed: f64 = cash.iter().map(|(_, _, amount, _)| krw(amount)).sum();
    let running = krw(&cash.last().expect("at least one row").3);
    assert!(
        (replayed - running).abs() < 0.0001,
        "cash_ledger must not contradict itself: events replay to {replayed}, \
         the running balance says {running} ({cash:?})"
    );
    for (i, (seq, _, _, _)) in cash.iter().enumerate() {
        assert_eq!(
            *seq,
            i as i64 + 1,
            "per-account seq is strictly increasing with no holes: {cash:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// The session actually executes, and the ledger says so.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_queued_target_executes_into_the_ledger_and_settles_executed() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let m = h.member.clone();
    let account = paper_account(&h, &m, "exec-happy", "10000000").await;
    let config = strategy_config(&h, &m, "exec-happy-cfg").await;
    let target = queue_target(
        &h,
        &m,
        account,
        config,
        COMPUTED_ON,
        EFFECTIVE_DATE,
        targets_json(),
    )
    .await;

    let data = curated_fixture();
    let worker = h.worker_pool().await;
    let outcome = run_and_settle(&h.state(), &worker, data.root(), &m.actor(), target)
        .await
        .expect("the runner executes and settles the session");

    assert_eq!(
        outcome.target.status, "EXECUTED",
        "a session that placed real orders settles EXECUTED"
    );
    // WARNING, not INFO: no recommendation run is seeded for this close, so
    // parity is NOT_COMPARABLE. The grade is about the backtest comparison,
    // not about whether the session executed.
    assert_eq!(outcome.severity, AlertSeverity::Warning);

    let view = ledger_view(&h, &m, account).await;
    assert_eq!(
        view.orders.len(),
        2,
        "both target instruments were bought: {:?}",
        view.orders
    );
    for (order_ref, instrument, side, status, on_date) in &view.orders {
        assert_eq!(side, "BUY", "a funded account with no positions only buys");
        assert_eq!(status, "FILLED", "Paper fills the whole order at the open");
        assert_eq!(
            *on_date,
            date(EFFECTIVE_DATE),
            "created_at must fall on the SESSION date, not the runner's day -- \
             settlement looks for orders by exactly this predicate"
        );
        assert!(
            Uuid::parse_str(order_ref).is_ok(),
            "order_ref is the deterministic uuid5 id paper_flow mints: {order_ref}"
        );
        assert!(instrument.ends_with(".KRX"));
    }

    assert_eq!(view.fills.len(), 2, "one fill per order: {:?}", view.fills);
    for (fill_ref, _, price, order_ref) in &view.fills {
        assert!(
            view.orders.iter().any(|(r, _, _, _, _)| r == order_ref),
            "every fill points at an order of this session"
        );
        assert!(
            Uuid::parse_str(fill_ref).is_ok(),
            "fill_ref is the deterministic uuid5 id: {fill_ref}"
        );
        // The execution price embeds slippage over the raw open, so it is
        // strictly above it on a buy (10 bps of 10,240 / 8,520).
        let price = krw(price);
        assert!(
            price > 8520.0 && price < 10251.0,
            "fills price at the session's raw open plus slippage, not at a close: {price}"
        );
    }

    assert_eq!(
        view.positions.len(),
        2,
        "the account now holds both instruments: {:?}",
        view.positions
    );
    for (_, quantity) in &view.positions {
        assert!(krw(quantity) > 0.0, "a bought position is a real quantity");
    }

    assert_cash_ledger_agrees_with_itself(&view.cash);
    assert_eq!(
        view.cash.len(),
        3,
        "the opening deposit plus one movement per fill: {:?}",
        view.cash
    );
    let opening = krw(&view.cash[0].3);
    let closing = krw(&view.cash.last().expect("rows").3);
    assert_eq!(opening, 10_000_000.0);
    assert!(
        closing > 0.0 && closing < opening,
        "buying spent cash without ever crossing zero: {opening} -> {closing}"
    );
    for (_, event_type, amount, _) in view.cash.iter().skip(1) {
        assert_eq!(event_type, "BUY");
        assert!(krw(amount) < 0.0, "a buy is a cash DEBIT, signed: {amount}");
    }

    h.teardown().await;
}

// ---------------------------------------------------------------------------
// Running the same session twice never doubles the ledger.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_replayed_session_never_double_executes() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let m = h.member.clone();
    let account = paper_account(&h, &m, "exec-replay", "10000000").await;
    let config = strategy_config(&h, &m, "exec-replay-cfg").await;
    let target = queue_target(
        &h,
        &m,
        account,
        config,
        COMPUTED_ON,
        EFFECTIVE_DATE,
        targets_json(),
    )
    .await;

    let data = curated_fixture();
    let worker = h.worker_pool().await;
    run_and_settle(&h.state(), &worker, data.root(), &m.actor(), target)
        .await
        .expect("the first run executes");
    let first = ledger_view(&h, &m, account).await;

    // A second runner claiming the same target: the row is no longer PENDING,
    // so it is refused before anything is executed.
    let second = run_and_settle(&h.state(), &worker, data.root(), &m.actor(), target).await;
    assert!(
        second.is_err(),
        "a settled target must not be run a second time"
    );

    // And the crash window the status guard cannot see: the engine called
    // directly, as it would be after a runner died between its COMMIT and the
    // settle. It must recognise its own work rather than repeat it.
    let input = SessionInput {
        account_id: account,
        owner_user_id: m.user_id,
        effective_date: TradingDate::parse(EFFECTIVE_DATE).expect("valid date"),
        targets: targets_from_json(&targets_json()).expect("targets parse"),
    };
    let resumed = execute_session(&worker, data.root(), &input)
        .await
        .expect("a resumed session reports rather than repeats");
    assert!(
        matches!(resumed, ExecutionOutcome::AlreadyExecuted { orders: 2 }),
        "the resumed session must recognise the orders it already wrote: {resumed:?}"
    );

    let after = ledger_view(&h, &m, account).await;
    assert_eq!(after.orders.len(), first.orders.len(), "zero extra orders");
    assert_eq!(after.fills.len(), first.fills.len(), "zero extra fills");
    assert_eq!(
        after.cash.len(),
        first.cash.len(),
        "zero extra cash movements"
    );
    assert_cash_ledger_agrees_with_itself(&after.cash);

    h.teardown().await;
}

// ---------------------------------------------------------------------------
// Nothing worth trading is BLOCKED, never EXECUTED and never FAILED.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_session_with_nothing_worth_trading_settles_blocked() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let m = h.member.clone();
    // 120,000 KRW: the 0.6/0.4 split puts both order values under the
    // profile's 100,000 minimum trade, so the sizer plans nothing at all.
    let account = paper_account(&h, &m, "exec-notrade", "120000").await;
    let config = strategy_config(&h, &m, "exec-notrade-cfg").await;
    let target = queue_target(
        &h,
        &m,
        account,
        config,
        COMPUTED_ON,
        EFFECTIVE_DATE,
        targets_json(),
    )
    .await;

    let data = curated_fixture();
    let worker = h.worker_pool().await;
    let outcome = run_and_settle(&h.state(), &worker, data.root(), &m.actor(), target)
        .await
        .expect("a no-trade session still settles");

    assert_eq!(
        outcome.target.status, "SKIPPED",
        "a deliberate no-trade is auditable, not EXECUTED"
    );
    assert_eq!(
        outcome.severity,
        AlertSeverity::Warning,
        "blocked warns; it is not the CRITICAL a broken runner earns"
    );

    let view = ledger_view(&h, &m, account).await;
    assert!(
        view.orders.is_empty() && view.fills.is_empty(),
        "a no-trade session writes nothing: {:?} {:?}",
        view.orders,
        view.fills
    );
    assert_eq!(
        view.cash.len(),
        1,
        "only the opening deposit: {:?}",
        view.cash
    );

    h.teardown().await;
}

// ---------------------------------------------------------------------------
// A session without prices fails closed rather than trading on partial data.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_session_with_no_open_price_fails_closed_and_writes_nothing() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let m = h.member.clone();
    let account = paper_account(&h, &m, "exec-noprice", "10000000").await;
    let config = strategy_config(&h, &m, "exec-noprice-cfg").await;
    // The curated zone has no bar for this session, so there is no open to
    // execute at and no honest way to price the target.
    let target = queue_target(
        &h,
        &m,
        account,
        config,
        EFFECTIVE_DATE,
        UNPRICED_DATE,
        targets_json(),
    )
    .await;

    let data = curated_fixture();
    let worker = h.worker_pool().await;
    let outcome = run_and_settle(&h.state(), &worker, data.root(), &m.actor(), target)
        .await
        .expect("a failed session still settles rather than staying PENDING");

    assert_eq!(outcome.target.status, "SKIPPED");
    assert_eq!(
        outcome.severity,
        AlertSeverity::Critical,
        "a session that could not run is escalated, not quietly completed"
    );

    let view = ledger_view(&h, &m, account).await;
    assert!(
        view.orders.is_empty() && view.fills.is_empty() && view.positions.is_empty(),
        "a missing price fails the WHOLE session, never half of it"
    );
    assert_eq!(view.cash.len(), 1, "cash was never touched");

    h.teardown().await;
}

// ---------------------------------------------------------------------------
// AT-07: one member's runner can never move another member's ledger.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_session_never_moves_another_members_ledger() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let m1 = h.member.clone();
    let m2 = h
        .seed_user(
            auth::entitlement::Role::Member,
            "member2@lagrange.test",
            "member-iss",
            "member-sub-2",
        )
        .await;
    let acct1 = paper_account(&h, &m1, "exec-cross-1", "10000000").await;
    let acct2 = paper_account(&h, &m2, "exec-cross-2", "10000000").await;
    let cfg1 = strategy_config(&h, &m1, "exec-cross-cfg-1").await;
    let t1 = queue_target(
        &h,
        &m1,
        acct1,
        cfg1,
        COMPUTED_ON,
        EFFECTIVE_DATE,
        targets_json(),
    )
    .await;

    let data = curated_fixture();
    let worker = h.worker_pool().await;
    run_and_settle(&h.state(), &worker, data.root(), &m1.actor(), t1)
        .await
        .expect("member 1's session runs");

    let theirs = ledger_view(&h, &m1, acct1).await;
    assert_eq!(theirs.orders.len(), 2);
    let others = ledger_view(&h, &m2, acct2).await;
    assert!(
        others.orders.is_empty() && others.fills.is_empty() && others.positions.is_empty(),
        "another member's account is untouched by a session that is not theirs"
    );

    // The engine binds `owner_user_id` as a predicate, so a session whose
    // owner does not own the account executes nothing even on a worker pool
    // that RLS never filters.
    let input = SessionInput {
        account_id: acct1,
        owner_user_id: m2.user_id,
        effective_date: TradingDate::parse(EFFECTIVE_DATE).expect("valid date"),
        targets: targets_from_json(&targets_json()).expect("targets parse"),
    };
    let stolen = execute_session(&worker, data.root(), &input).await;
    assert!(
        stolen.is_err(),
        "an account is not executable by someone who does not own it"
    );

    h.teardown().await;
}

// ---------------------------------------------------------------------------
// A target queued against a LIVE account executes nothing.
// ---------------------------------------------------------------------------

/// A LIVE account, seeded directly (no HTTP route creates one as PAPER-typed
/// data would require -- this is the point: nothing gates this at the API
/// today, so the engine itself must).
async fn live_account(h: &Harness, u: &UserCtx, name: &str, cash: &str) -> Uuid {
    let pool = common::actor_pool(&h.app_url, &u.user_id.to_string(), 2).await;
    let account: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (owner_user_id, account_type, name, currency) \
         VALUES ($1, 'LIVE', $2, 'KRW') RETURNING id",
    )
    .bind(u.user_id)
    .bind(name)
    .fetch_one(&pool)
    .await
    .expect("live account");
    sqlx::query(
        "INSERT INTO cash_ledger (account_id, owner_user_id, seq, event_type, amount, balance, currency) \
         VALUES ($1, $2, 1, 'DEPOSIT', $3::numeric, $3::numeric, 'KRW')",
    )
    .bind(account)
    .bind(u.user_id)
    .bind(cash)
    .execute(&pool)
    .await
    .expect("live account funded");
    account
}

/// The Paper engine is the only writer of `orders`/`fills`/`cash_ledger` in
/// this repository, and those tables are what the LIVE Risk Gateway reads for
/// its cash and position inputs (`risk_snapshot::account_state`). Nothing
/// upstream constrains `pending_targets.account_id` to a PAPER account, so
/// the engine's own guard is what stands between a queued target and writing
/// simulated fills into a real account's ledger.
#[tokio::test]
async fn a_target_queued_against_a_live_account_executes_nothing() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let m = h.member.clone();
    let account = live_account(&h, &m, "exec-live-guard", "10000000").await;
    let config = strategy_config(&h, &m, "exec-live-guard-cfg").await;
    let target = queue_target(
        &h,
        &m,
        account,
        config,
        COMPUTED_ON,
        EFFECTIVE_DATE,
        targets_json(),
    )
    .await;

    let data = curated_fixture();
    let worker = h.worker_pool().await;
    let outcome = run_and_settle(&h.state(), &worker, data.root(), &m.actor(), target)
        .await
        .expect(
            "a session against the wrong account type still settles rather than hanging PENDING",
        );

    assert_eq!(
        outcome.target.status, "SKIPPED",
        "a LIVE account must never settle EXECUTED via the Paper engine"
    );
    assert_eq!(
        outcome.severity,
        AlertSeverity::Critical,
        "queuing a target against a LIVE account is a caller error, escalated loudly"
    );

    let pool = common::actor_pool(&h.app_url, &m.user_id.to_string(), 2).await;
    let orders: i64 = sqlx::query_scalar("SELECT count(*) FROM orders WHERE account_id = $1")
        .bind(account)
        .fetch_one(&pool)
        .await
        .expect("orders count");
    let cash_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM cash_ledger WHERE account_id = $1")
            .bind(account)
            .fetch_one(&pool)
            .await
            .expect("cash_ledger count");
    assert_eq!(orders, 0, "no simulated order may land in a LIVE account");
    assert_eq!(
        cash_rows, 1,
        "only the real opening deposit -- no simulated cash movement"
    );

    h.teardown().await;
}
