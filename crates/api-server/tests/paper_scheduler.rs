//! Todo 31: the Paper scheduler's DB-backed contract.
//!
//! `portfolio-model`'s `paper_flow.rs` already proves the pure session flow
//! (effective-date guard, byte-identical re-planning, replay rejection,
//! crash resume, AT-07 id isolation). This file proves the persistence
//! layer that a scheduler needs to survive a restart: a target is queued
//! uniquely per `(account, effective_date)`, re-queueing the same close is
//! a no-op, two runners racing one session cannot both claim it, and
//! entitlement loss pauses queueing without deleting history.
//!
//! The test ACTS AS the scheduler (the test-as-worker pattern from Todo
//! 29): there is no daemon to run, and one would have no consumer until
//! Todo 32's dashboards.

mod common;

use axum::http::StatusCode;
use chrono::NaiveDate;
use common::{Harness, UserCtx};
use serde_json::json;
use uuid::Uuid;

use api_server::repos::pending_targets::NewPendingTarget;

fn status(resp: &axum::http::Response<axum::body::Body>) -> StatusCode {
    resp.status()
}

fn date(iso: &str) -> NaiveDate {
    NaiveDate::parse_from_str(iso, "%Y-%m-%d").expect("valid date")
}

/// The weight vector a close(T) computation produced.
fn targets_json() -> serde_json::Value {
    json!([
        { "instrument_id": "069500.KRX", "weight": "0.600000" },
        { "instrument_id": "229200.KRX", "weight": "0.400000" }
    ])
}

async fn paper_account(h: &Harness, u: &UserCtx, name: &str) -> String {
    let resp = h
        .post(
            "/api/v1/paper/accounts",
            Some(u),
            true,
            json!({ "name": name, "currency": "KRW", "initial_cash": "10000000" }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::CREATED, "account create");
    Harness::body_json(resp).await["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn strategy_config(h: &Harness, u: &UserCtx, key: &str) -> String {
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
    assert_eq!(status(&resp), StatusCode::CREATED, "config create");
    Harness::body_json(resp).await["id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn new_target(account: &str, config: &str, computed: &str, effective: &str) -> NewPendingTarget {
    NewPendingTarget {
        account_id: Uuid::parse_str(account).unwrap(),
        strategy_config_id: Uuid::parse_str(config).unwrap(),
        computed_on: date(computed),
        effective_date: date(effective),
        targets_json: targets_json(),
    }
}

// ---------------------------------------------------------------------------
// Queueing at close(T) is unique and idempotent per session.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn requeueing_the_same_close_resolves_to_the_same_target() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let m = h.member.clone();
    let account = paper_account(&h, &m, "sched-acct").await;
    let config = strategy_config(&h, &m, "sched-cfg").await;
    let repo = h.state_pending_targets();

    let first = repo
        .queue(
            &m.actor(),
            new_target(&account, &config, "2026-01-05", "2026-01-06"),
        )
        .await
        .expect("first queue");
    // The scheduler restarts and recomputes the SAME close.
    let second = repo
        .queue(
            &m.actor(),
            new_target(&account, &config, "2026-01-05", "2026-01-06"),
        )
        .await
        .expect("re-queue is a no-op, not an error");

    assert_eq!(
        first.id, second.id,
        "re-queueing resolves to the same target"
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pending_targets WHERE account_id = $1::uuid AND effective_date = $2",
    )
    .bind(&account)
    .bind(date("2026-01-06"))
    .fetch_one(&h.member_pool().await)
    .await
    .unwrap();
    assert_eq!(count, 1, "one target per account per session, ever");
    h.teardown().await;
}

// ---------------------------------------------------------------------------
// The runner's claim: due scan, single settle, restart-safe.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_settled_target_can_never_be_claimed_twice() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let m = h.member.clone();
    let account = paper_account(&h, &m, "claim-acct").await;
    let config = strategy_config(&h, &m, "claim-cfg").await;
    let repo = h.state_pending_targets();

    let target = repo
        .queue(
            &m.actor(),
            new_target(&account, &config, "2026-01-05", "2026-01-06"),
        )
        .await
        .unwrap();

    let due = repo.due(&m.actor(), date("2026-01-06")).await.unwrap();
    assert_eq!(due.len(), 1, "the target is due at its effective session");
    assert_eq!(due[0].status, "PENDING");

    let settled = repo
        .settle(&m.actor(), target.id, "EXECUTED")
        .await
        .expect("the first runner settles it");
    assert_eq!(settled.status, "EXECUTED");
    assert!(settled.executed_at.is_some());

    // A second runner racing the same session finds nothing to claim.
    let second = repo.settle(&m.actor(), target.id, "EXECUTED").await;
    assert!(
        second.is_err(),
        "a settled target must never be claimed a second time"
    );
    let due_after = repo.due(&m.actor(), date("2026-01-06")).await.unwrap();
    assert!(due_after.is_empty(), "a settled target leaves the due scan");
    h.teardown().await;
}

#[tokio::test]
async fn a_target_is_not_due_before_its_effective_session() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let m = h.member.clone();
    let account = paper_account(&h, &m, "notdue-acct").await;
    let config = strategy_config(&h, &m, "notdue-cfg").await;
    let repo = h.state_pending_targets();

    repo.queue(
        &m.actor(),
        new_target(&account, &config, "2026-01-05", "2026-01-06"),
    )
    .await
    .unwrap();

    // Scanning at T (the close that produced it) must find nothing: the
    // target belongs to the NEXT session's open.
    let due_at_t = repo.due(&m.actor(), date("2026-01-05")).await.unwrap();
    assert!(
        due_at_t.is_empty(),
        "a T+1 target is never due at T -- the same-day-close error"
    );
    let due_at_t1 = repo.due(&m.actor(), date("2026-01-06")).await.unwrap();
    assert_eq!(due_at_t1.len(), 1);
    h.teardown().await;
}

// ---------------------------------------------------------------------------
// AT-07 at the persistence layer: two accounts, one session, no crossing.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn two_members_targets_for_the_same_session_stay_isolated() {
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
    let acct1 = paper_account(&h, &m1, "iso-acct").await;
    let acct2 = paper_account(&h, &m2, "iso-acct").await;
    let cfg1 = strategy_config(&h, &m1, "iso-cfg-1").await;
    let cfg2 = strategy_config(&h, &m2, "iso-cfg-2").await;
    let repo = h.state_pending_targets();

    repo.queue(
        &m1.actor(),
        new_target(&acct1, &cfg1, "2026-01-05", "2026-01-06"),
    )
    .await
    .unwrap();
    repo.queue(
        &m2.actor(),
        new_target(&acct2, &cfg2, "2026-01-05", "2026-01-06"),
    )
    .await
    .unwrap();

    // Each member's due scan sees exactly their own target.
    let due1 = repo.due(&m1.actor(), date("2026-01-06")).await.unwrap();
    let due2 = repo.due(&m2.actor(), date("2026-01-06")).await.unwrap();
    assert_eq!(due1.len(), 1);
    assert_eq!(due2.len(), 1);
    assert_ne!(due1[0].id, due2[0].id);
    assert_eq!(due1[0].account_id.to_string(), acct1);
    assert_eq!(due2[0].account_id.to_string(), acct2);

    // Member 2 cannot settle member 1's target (RLS => zero rows).
    let stolen = repo.settle(&m2.actor(), due1[0].id, "EXECUTED").await;
    assert!(
        stolen.is_err(),
        "a member must never settle another member's target"
    );
    h.teardown().await;
}

// ---------------------------------------------------------------------------
// Entitlement loss pauses queueing without deleting history.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn revoked_entitlement_pauses_new_sessions_but_keeps_prior_targets() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let m = h.member.clone();
    let account = paper_account(&h, &m, "pause-acct").await;
    let config = strategy_config(&h, &m, "pause-cfg").await;
    let repo = h.state_pending_targets();

    repo.queue(
        &m.actor(),
        new_target(&account, &config, "2026-01-05", "2026-01-06"),
    )
    .await
    .unwrap();

    h.seed_shared("UPDATE data_entitlements SET status='REVOKED' WHERE status='ACTIVE'")
        .await;

    // The member-facing surface is now closed (the gate the scheduler
    // consults before computing a new close).
    let resp = h
        .post(
            "/api/v1/paper/accounts",
            Some(&m),
            true,
            json!({ "name": "post-revoke", "currency": "KRW", "initial_cash": "10000000" }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::FORBIDDEN);
    assert_eq!(
        Harness::error_code(&Harness::body_json(resp).await),
        "DATA_ENTITLEMENT_REQUIRED"
    );

    // The already-queued target is untouched -- paused, never deleted.
    let history = repo
        .history(&m.actor(), Uuid::parse_str(&account).unwrap())
        .await
        .unwrap();
    assert_eq!(history.len(), 1, "prior targets survive entitlement loss");
    assert_eq!(history[0].status, "PENDING");
    h.teardown().await;
}
