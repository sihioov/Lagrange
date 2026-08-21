//! Ledger-derived Paper daily-equity seams.

mod common;

use std::path::{Path, PathBuf};

use axum::http::StatusCode;
use common::{Harness, UserCtx};
use domain::{ContentHash, Currency, InstrumentId, Price, TradingDate, UtcTimestamp};
use job_queue::paper_valuation::{ValuationError, ValuationOutcome, value_account};
use job_queue::phase0::CURATED_VERSION;
use market_data::CurateStore;
use market_data::curate::schema::{CuratedBar, write_bars};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

struct Dataset {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Dataset {
    fn root(&self) -> &std::path::Path {
        &self.root
    }
}

fn dataset(symbol: &str, date: &str, close: i64, close_at: &str) -> Dataset {
    let dir = tempfile::tempdir().expect("dataset tempdir");
    let root = dir.path().to_path_buf();
    let store = CurateStore::new(&root);
    let price = Price::parse(&close.to_string()).expect("positive close");
    let bar = CuratedBar {
        instrument_id: InstrumentId::parse(symbol).expect("instrument"),
        trading_date: TradingDate::parse(date).expect("trading date"),
        market_open_ts: UtcTimestamp::parse_rfc3339("2020-01-21T00:00:00Z").unwrap(),
        market_close_ts: UtcTimestamp::parse_rfc3339(close_at).expect("close timestamp"),
        open: price,
        high: price,
        low: price,
        close: price,
        volume: 1,
        trading_value: Some(close),
        currency: Currency::KRW,
        source: "test".to_owned(),
        ingested_at: UtcTimestamp::parse_rfc3339("2020-02-01T00:00:00Z").unwrap(),
        batch_id: "00000000-0000-0000-0000-000000000001"
            .parse()
            .expect("batch id"),
        raw_hash: ContentHash::from_bytes(b"paper-valuation"),
    };
    write_bars(
        &store.bars_path("kr", symbol, 2020, CURATED_VERSION),
        std::slice::from_ref(&bar),
    )
    .expect("write fixture bar");
    Dataset { _dir: dir, root }
}

async fn paper_account(h: &Harness, user: &UserCtx, name: &str, cash: &str) -> Uuid {
    let response = h
        .post(
            "/api/v1/paper/accounts",
            Some(user),
            true,
            json!({ "name": name, "currency": "KRW", "initial_cash": cash }),
        )
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    Uuid::parse_str(
        Harness::body_json(response).await["id"]
            .as_str()
            .expect("account id"),
    )
    .expect("account uuid")
}

async fn seed_position(
    h: &Harness,
    user: &UserCtx,
    account: Uuid,
    instrument: &str,
    quantity: i64,
) {
    h.seed_tenant(
        user,
        &format!(
            "INSERT INTO positions (account_id, owner_user_id, instrument_id, quantity) \
             VALUES ('{account}', '{owner}', '{instrument}', {quantity})",
            owner = user.user_id,
        ),
    )
    .await;
}

async fn equity_row(h: &Harness, account: Uuid) -> (String, String, String, String) {
    let row = sqlx::query(
        "SELECT equity::text, cash::text, positions_value::text, currency \
         FROM daily_equity WHERE account_id = $1 AND trading_date = DATE '2020-01-21'",
    )
    .bind(account)
    .fetch_one(&h.worker_pool().await)
    .await
    .expect("daily equity row");
    (
        row.get("equity"),
        row.get("cash"),
        row.get("positions_value"),
        row.get("currency"),
    )
}

#[tokio::test]
async fn valuation_writes_exact_ledger_values_and_is_idempotent() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let user = h.member.clone();
    let account = paper_account(&h, &user, "valuation-values", "9000000").await;
    seed_position(&h, &user, account, "069500.KRX", 100).await;
    let data = dataset("069500.KRX", "2020-01-21", 10290, "2020-01-21T06:30:00Z");
    let worker = h.worker_pool().await;
    let trading_date = TradingDate::parse("2020-01-21").unwrap();

    let first = value_account(&worker, data.root(), account, user.user_id, trading_date)
        .await
        .expect("first valuation");
    let ValuationOutcome::Valued {
        equity,
        cash,
        positions_value,
    } = first
    else {
        panic!("first valuation must insert a row");
    };
    assert_eq!(equity.amount().to_string(), "10029000.0000");
    assert_eq!(cash.amount().to_string(), "9000000.0000");
    assert_eq!(positions_value.amount().to_string(), "1029000.0000");
    assert_eq!(
        equity_row(&h, account).await,
        (
            "10029000.0000".to_owned(),
            "9000000.0000".to_owned(),
            "1029000.0000".to_owned(),
            "KRW".to_owned(),
        )
    );

    let second = value_account(&worker, data.root(), account, user.user_id, trading_date)
        .await
        .expect("second valuation");
    assert_eq!(second, ValuationOutcome::AlreadyValued);
    h.teardown().await;
}

#[tokio::test]
async fn valuation_refuses_missing_close_without_writing_a_row() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let user = h.member.clone();
    let account = paper_account(&h, &user, "valuation-missing", "9000000").await;
    seed_position(&h, &user, account, "069500.KRX", 100).await;
    let data = dataset("069500.KRX", "2020-01-20", 10290, "2020-01-20T06:30:00Z");
    let worker = h.worker_pool().await;
    let error = value_account(
        &worker,
        data.root(),
        account,
        user.user_id,
        TradingDate::parse("2020-01-21").unwrap(),
    )
    .await
    .expect_err("missing close must fail closed");
    assert!(matches!(error, ValuationError::MissingMark { .. }));
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM daily_equity WHERE account_id = $1 AND trading_date = DATE '2020-01-21'",
    )
    .bind(account)
    .fetch_one(&worker)
    .await
    .unwrap();
    assert_eq!(count, 0);
    h.teardown().await;
}

#[tokio::test]
async fn valuation_refuses_a_close_that_is_still_in_the_future() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let user = h.member.clone();
    let account = paper_account(&h, &user, "valuation-future", "9000000").await;
    seed_position(&h, &user, account, "069500.KRX", 100).await;
    let data = dataset("069500.KRX", "2020-01-21", 10290, "2999-01-21T06:30:00Z");
    let error = value_account(
        &h.worker_pool().await,
        data.root(),
        account,
        user.user_id,
        TradingDate::parse("2020-01-21").unwrap(),
    )
    .await
    .expect_err("future close must fail closed");
    assert!(matches!(error, ValuationError::CloseNotYetAvailable { .. }));
    h.teardown().await;
}

#[tokio::test]
async fn valuation_refuses_a_conflicting_existing_point() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let user = h.member.clone();
    let account = paper_account(&h, &user, "valuation-conflict", "9000000").await;
    seed_position(&h, &user, account, "069500.KRX", 100).await;
    let data = dataset("069500.KRX", "2020-01-21", 10290, "2020-01-21T06:30:00Z");
    let worker = h.worker_pool().await;
    let trading_date = TradingDate::parse("2020-01-21").unwrap();
    value_account(&worker, data.root(), account, user.user_id, trading_date)
        .await
        .expect("initial point");
    sqlx::query("UPDATE daily_equity SET equity = 1 WHERE account_id = $1 AND trading_date = $2")
        .bind(account)
        .bind(trading_date.as_naive_date())
        .execute(&worker)
        .await
        .expect("corrupt point for conflict test");

    let error = value_account(&worker, data.root(), account, user.user_id, trading_date)
        .await
        .expect_err("a conflicting point must not be overwritten");
    assert!(matches!(error, ValuationError::DailyEquityConflict { .. }));
    h.teardown().await;
}

#[tokio::test]
async fn valuation_cannot_cross_tenants_or_touch_a_live_account() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let member = h.member.clone();
    let other = h
        .seed_user(
            auth::entitlement::Role::Member,
            "paper-valuation-other@lagrange.test",
            "paper-valuation-other-iss",
            "paper-valuation-other-sub",
        )
        .await;
    let paper = paper_account(&h, &member, "valuation-tenant", "9000000").await;
    let worker = h.worker_pool().await;
    let date = TradingDate::parse("2020-01-21").unwrap();
    let cross_tenant = value_account(&worker, Path::new("."), paper, other.user_id, date)
        .await
        .expect_err("wrong owner must be unavailable");
    assert!(matches!(
        cross_tenant,
        ValuationError::AccountUnavailable { .. }
    ));

    let live = Uuid::new_v4();
    h.seed_tenant(
        &member,
        &format!(
            "INSERT INTO accounts (id, owner_user_id, account_type, name, currency, status, initial_cash, cost_profile_id, cost_profile_version) \
             VALUES ('{live}', '{owner}', 'LIVE', 'valuation-live', 'KRW', 'ACTIVE', 9000000, 'KRX_ETF_DEFAULT', 1)",
            owner = member.user_id,
        ),
    )
    .await;
    let live_error = value_account(&worker, Path::new("."), live, member.user_id, date)
        .await
        .expect_err("LIVE account must be refused");
    assert!(matches!(
        live_error,
        ValuationError::AccountUnavailable { .. }
    ));
    h.teardown().await;
}
