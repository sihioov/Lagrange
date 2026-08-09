//! Worker-wide Paper runner seams.

mod common;

use std::path::PathBuf;

use chrono::NaiveDate;
use common::{Harness, UserCtx};
use domain::{ContentHash, Currency, InstrumentId, Price, TradingDate, UtcTimestamp};
use market_data::CurateStore;
use market_data::curate::schema::{CuratedBar, write_bars};
use serde_json::json;
use uuid::Uuid;

use api_server::paper_runner::{RunnerServices, parse_args, run_cycle};
use api_server::repos::pending_targets::{NewPendingTarget, PendingTargetRepo};

#[test]
fn runner_cycle_api_exists() {
    let _ = run_cycle;
}

#[test]
fn runner_args_accept_once_and_a_padded_date() {
    let args = parse_args(vec![
        "--once".to_owned(),
        "--date".to_owned(),
        "2026-01-06".to_owned(),
    ])
    .expect("arguments parse");
    assert!(args.once);
    assert_eq!(args.date, Some(date("2026-01-06")));
}

#[test]
fn runner_args_refuse_unknown_or_malformed_values() {
    assert!(parse_args(vec!["--wat".to_owned()]).is_err());
    assert!(parse_args(vec!["--date".to_owned(), "2026-1-6".to_owned(),]).is_err());
}

fn date(value: &str) -> NaiveDate {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").expect("valid date")
}

struct Dataset {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Dataset {
    fn root(&self) -> &std::path::Path {
        &self.root
    }
}

fn runner_dataset() -> Dataset {
    let dir = tempfile::tempdir().expect("dataset tempdir");
    let root = dir.path().to_path_buf();
    let store = CurateStore::new(root.join("curated"));
    let open = Price::parse("10240").expect("open price");
    let close = Price::parse("10300").expect("close price");
    let bar = CuratedBar {
        instrument_id: InstrumentId::parse("069500.KRX").unwrap(),
        trading_date: TradingDate::parse("2020-01-21").unwrap(),
        market_open_ts: UtcTimestamp::parse_rfc3339("2020-01-21T00:00:00Z").unwrap(),
        market_close_ts: UtcTimestamp::parse_rfc3339("2020-01-21T06:30:00Z").unwrap(),
        open,
        high: close,
        low: open,
        close,
        volume: 1,
        trading_value: Some(1),
        currency: Currency::KRW,
        source: "test".to_owned(),
        ingested_at: UtcTimestamp::parse_rfc3339("2020-02-01T00:00:00Z").unwrap(),
        batch_id: "00000000-0000-0000-0000-000000000001".parse().unwrap(),
        raw_hash: ContentHash::from_bytes(b"paper-runner"),
    };
    write_bars(
        &store.bars_path("kr", "069500.KRX", 2020, 1),
        std::slice::from_ref(&bar),
    )
    .expect("write runner bar");
    Dataset { _dir: dir, root }
}

async fn paper_account(h: &Harness, user: &UserCtx, name: &str) -> Uuid {
    let response = h
        .post(
            "/api/v1/paper/accounts",
            Some(user),
            true,
            json!({ "name": name, "currency": "KRW", "initial_cash": "10000000" }),
        )
        .await;
    assert_eq!(response.status(), axum::http::StatusCode::CREATED);
    Uuid::parse_str(
        Harness::body_json(response).await["id"]
            .as_str()
            .expect("account id"),
    )
    .expect("uuid account id")
}

async fn strategy_config(h: &Harness, user: &UserCtx, key: &str) -> Uuid {
    let response = h
        .send(
            "POST",
            "/api/v1/strategies/buy_and_hold/configs",
            Some(user),
            true,
            Some("paper-runner-test-rid"),
            Some(key),
            Some(json!({
                "strategy_version": "1.0.0",
                "config": { "lookback": 200 },
                "is_active": true,
            })),
        )
        .await;
    assert_eq!(response.status(), axum::http::StatusCode::CREATED);
    Uuid::parse_str(
        Harness::body_json(response).await["id"]
            .as_str()
            .expect("strategy config id"),
    )
    .expect("uuid strategy config id")
}

fn target(account_id: Uuid, config_id: Uuid, effective_date: &str) -> NewPendingTarget {
    NewPendingTarget {
        account_id,
        strategy_config_id: config_id,
        computed_on: date("2020-01-20"),
        effective_date: date(effective_date),
        targets_json: json!([
            { "instrument_id": "069500.KRX", "weight": "1.000000" }
        ]),
        dataset_version: Some("krx_eod_bars@2026-01-01".to_owned()),
    }
}

#[tokio::test]
async fn worker_scan_returns_only_due_pending_targets_across_owners() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let member = h.member.clone();
    let other = h
        .seed_user(
            auth::entitlement::Role::Member,
            "paper-runner-other@lagrange.test",
            "paper-runner-other-iss",
            "paper-runner-other-sub",
        )
        .await;
    let account_a = paper_account(&h, &member, "runner-a").await;
    let account_b = paper_account(&h, &other, "runner-b").await;
    let config_a = strategy_config(&h, &member, "runner-config-a").await;
    let config_b = strategy_config(&h, &other, "runner-config-b").await;
    let repo = h.state_pending_targets();

    let due_a = repo
        .queue(&member.actor(), target(account_a, config_a, "2026-01-06"))
        .await
        .expect("queue member A");
    repo.queue(&other.actor(), target(account_b, config_b, "2026-01-06"))
        .await
        .expect("queue member B");
    repo.queue(&member.actor(), target(account_a, config_a, "2026-01-07"))
        .await
        .expect("queue future target");
    repo.settle(&member.actor(), due_a.id, "SKIPPED")
        .await
        .expect("settle one target");

    let rows = PendingTargetRepo::due_worker(&h.worker_pool().await, date("2026-01-06"))
        .await
        .expect("worker scan");
    assert_eq!(
        rows.len(),
        1,
        "only the other owner's target remains pending"
    );
    assert_eq!(rows[0].account_id, account_b);
    assert_eq!(rows[0].owner_user_id, other.user_id);
    assert_eq!(rows[0].effective_date, date("2026-01-06"));
    assert_eq!(rows[0].status, "PENDING");
    h.teardown().await;
}

#[tokio::test]
async fn one_cycle_executes_a_target_and_values_the_account_once() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let user = h.member.clone();
    let account = paper_account(&h, &user, "runner-cycle-account").await;
    let config = strategy_config(&h, &user, "runner-cycle-config").await;
    let target = h
        .state_pending_targets()
        .queue(&user.actor(), target(account, config, "2020-01-21"))
        .await
        .expect("queue target");
    let data = runner_dataset();
    let services = RunnerServices::new(h.state(), h.worker_pool().await, data.root().to_path_buf());

    let first = run_cycle(&services, date("2020-01-21"))
        .await
        .expect("first cycle");
    assert_eq!(first.targets_seen, 1);
    assert_eq!(first.targets_settled, 1);
    assert_eq!(first.valuations_seen, 1);
    assert_eq!(first.valuations_written, 1);
    assert!(
        first.item_errors.is_empty(),
        "cycle errors: {:?}",
        first.item_errors
    );

    let status: String = sqlx::query_scalar("SELECT status FROM pending_targets WHERE id = $1")
        .bind(target.id)
        .fetch_one(&services.worker_pool)
        .await
        .expect("target status");
    assert_eq!(status, "EXECUTED");
    let order_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM orders WHERE account_id = $1 AND created_at::date = DATE '2020-01-21'",
    )
    .bind(account)
    .fetch_one(&services.worker_pool)
    .await
    .expect("order count");
    assert!(order_count > 0);

    let second = run_cycle(&services, date("2020-01-21"))
        .await
        .expect("second cycle");
    assert_eq!(second.targets_seen, 0);
    assert_eq!(second.valuations_seen, 1);
    assert_eq!(second.valuations_written, 1, "same valuation is idempotent");
    let member_pool = common::actor_pool(&h.app_url, &user.user_id.to_string(), 2).await;
    let notification_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM notifications WHERE owner_user_id = $1")
            .bind(user.user_id)
            .fetch_one(&member_pool)
            .await
            .expect("notification count");
    assert_eq!(
        notification_count, 1,
        "settlement notification is not duplicated"
    );
    h.teardown().await;
}
