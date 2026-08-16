//! Todo 32: what a settled Paper session TELLS the user.
//!
//! `paper_scheduler.rs` proved the target lifecycle; this file proves the
//! notification layer on top of it: one settled session yields exactly one
//! severity-graded notification with durable delivery outcomes, a divergent
//! or incomparable session is WARNING (design §15.3) rather than a quiet
//! completion notice, a blocked session is recorded rather than silent, and
//! a duplicate claim announces nothing a second time.
//!
//! The test ACTS AS the runner (the pattern established in Todos 29 and 31):
//! there is no daemon, and `settle_and_announce` is exactly the entry point
//! one would call.

mod common;

use axum::http::StatusCode;
use chrono::NaiveDate;
use common::{Harness, UserCtx};
use serde_json::json;
use uuid::Uuid;

use api_server::notify::AlertSeverity;
use api_server::paper_session::{SessionOutcome, settle_and_announce};
use api_server::repos::pending_targets::{NewPendingTarget, PaperSettlementAnnouncement};
use domain::DatasetVersionId;
use result_model::paper_parity::{ParityReport, ParityStatus};

fn status(resp: &axum::http::Response<axum::body::Body>) -> StatusCode {
    resp.status()
}

fn date(iso: &str) -> NaiveDate {
    NaiveDate::parse_from_str(iso, "%Y-%m-%d").expect("valid date")
}

const DATASET: &str = "2026-01-01";

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

/// Queues the target one close(T) produced, ready to settle at T+1.
async fn queue_target(h: &Harness, u: &UserCtx, account: &str, config: &str) -> Uuid {
    h.state_pending_targets()
        .queue(
            &u.actor(),
            NewPendingTarget {
                account_id: Uuid::parse_str(account).unwrap(),
                strategy_config_id: Uuid::parse_str(config).unwrap(),
                computed_on: date("2026-01-05"),
                effective_date: date("2026-01-06"),
                targets_json: targets_json(),
                dataset_version: Some(DATASET.to_owned()),
            },
        )
        .await
        .expect("target queues")
        .id
}

/// Seeds the LEDGER side of an executed session: one order on the target's
/// effective date.
///
/// `SessionOutcome::Executed` documents itself as "The session's orders and
/// fills are in the ledger", and `settle_and_announce` now checks that rather
/// than believing it. Before that check existed these tests settled EXECUTED
/// against an empty ledger and asserted the user was told the session had
/// completed -- which is precisely the false signal the check was added to
/// stop. The premise now has to be established, so the assertions mean what
/// they say.
async fn seed_executed_session(h: &Harness, u: &UserCtx, account: &str) {
    h.seed_tenant(
        u,
        &format!(
            "INSERT INTO orders \
             (account_id, owner_user_id, order_ref, instrument_id, side, quantity, price, \
              status, created_at) \
             VALUES ('{account}', '{owner}', 'order-{account}', '069500.KRX', 'BUY', \
                     10, 7250, 'FILLED', '2026-01-06T00:30:00Z')",
            owner = u.user_id
        ),
    )
    .await;
}

/// Seeds the backtest side of the parity comparison: a SUCCEEDED
/// recommendation run for the same config and close, with the weights the
/// caller passes.
async fn seed_backtest_side(
    h: &Harness,
    u: &UserCtx,
    config: &str,
    weights: &[(&str, &str)],
) -> Uuid {
    let run_id = Uuid::new_v4();
    h.seed_tenant(
        u,
        &format!(
            "INSERT INTO recommendation_runs \
             (id, owner_user_id, strategy_config_id, as_of, status, summary_json, \
              dataset_version_id, dataset_manifest_sha256) \
             VALUES ('{run_id}', '{owner}', '{config}', DATE '2026-01-05', 'SUCCEEDED', \
                     '{{\"dataset_version\": \"{DATASET}\"}}'::jsonb, \
                     (SELECT id FROM dataset_versions \
                       WHERE dataset_id = 'krx_eod_bars' AND version = '2026-01-01'), \
                     (SELECT manifest_sha256 FROM dataset_versions \
                       WHERE dataset_id = 'krx_eod_bars' AND version = '2026-01-01'))",
            owner = u.user_id,
        ),
    )
    .await;
    for (instrument, weight) in weights {
        h.seed_tenant(
            u,
            &format!(
                "INSERT INTO recommendation_items \
                 (recommendation_run_id, owner_user_id, instrument_id, rank, target_weight, \
                  excluded) \
                 VALUES ('{run_id}', '{owner}', '{instrument}', 1, {weight}, false)",
                owner = u.user_id,
            ),
        )
        .await;
    }
    run_id
}

async fn bind_target_backtest_lineage(h: &Harness, u: &UserCtx, target: Uuid, run_id: Uuid) {
    h.seed_migration_owner(
        u,
        &format!(
            "UPDATE pending_targets \
                SET source_kind = 'MANUAL_RECOMMENDATION', \
                    recommendation_run_id = '{run_id}', \
                    dataset_version_id = (SELECT id FROM dataset_versions \
                                           WHERE dataset_id = 'krx_eod_bars' \
                                             AND version = '2026-01-01'), \
                    dataset_manifest_sha256 = (SELECT manifest_sha256 FROM dataset_versions \
                                                 WHERE dataset_id = 'krx_eod_bars' \
                                                   AND version = '2026-01-01') \
              WHERE id = '{target}'",
        ),
    )
    .await;
}

async fn feed(h: &Harness, u: &UserCtx) -> Vec<serde_json::Value> {
    let resp = h.get("/api/v1/notifications", Some(u)).await;
    assert_eq!(status(&resp), StatusCode::OK, "feed reads");
    Harness::body_json(resp).await["items"]
        .as_array()
        .expect("items")
        .clone()
}

fn assert_exact_dataset_lineage(parity: &ParityReport) {
    let dataset = parity
        .lineage
        .fields
        .iter()
        .find(|field| field.field == "dataset_version")
        .expect("parity report carries dataset lineage");
    assert_eq!(dataset.backtest, DATASET);
    assert_eq!(dataset.paper, DATASET);
    assert_ne!(dataset.backtest, "unknown-dataset");
    assert_eq!(
        DatasetVersionId::parse(&dataset.backtest)
            .expect("production dataset version parses")
            .as_str(),
        DATASET
    );
}

// ---------------------------------------------------------------------------
// A session whose signals match its backtest is a plain completion notice.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_matching_session_completes_with_an_info_notice() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let m = h.member.clone();
    let account = paper_account(&h, &m, "notify-match").await;
    let config = strategy_config(&h, &m, "notify-match-cfg").await;
    let run_id = seed_backtest_side(
        &h,
        &m,
        &config,
        &[
            ("069500.KRX", "0.600000"),
            ("229200.KRX", "0.400000"),
            ("114260.KRX", "0.000000"),
        ],
    )
    .await;
    let target = queue_target(&h, &m, &account, &config).await;
    bind_target_backtest_lineage(&h, &m, target, run_id).await;

    seed_executed_session(&h, &m, &account).await;
    let outcome = settle_and_announce(&h.state(), &m.actor(), target, SessionOutcome::Executed)
        .await
        .expect("the runner settles and announces");

    assert_eq!(outcome.target.status, "EXECUTED");
    let parity = outcome.parity.as_ref().expect("parity report");
    assert_exact_dataset_lineage(parity);
    assert_eq!(parity.status, ParityStatus::Match);
    assert_eq!(outcome.severity, AlertSeverity::Info);
    assert_eq!(
        outcome.alerts.deliveries.len(),
        1,
        "INFO routes to the web feed only"
    );
    assert_eq!(outcome.alerts.deliveries[0].channel, "web");
    assert_eq!(outcome.alerts.deliveries[0].status, "SUCCESS");

    // The feed carries the notice AND its delivery outcome.
    let items = feed(&h, &m).await;
    assert_eq!(items.len(), 1, "one session, one notification");
    assert_eq!(items[0]["kind"], "job");
    let deliveries = items[0]["deliveries"].as_array().expect("deliveries");
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0]["status"], "SUCCESS");
    h.teardown().await;
}

// ---------------------------------------------------------------------------
// A divergence is a WARNING, never a quiet completion (design 15.3).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_divergent_session_warns_and_reaches_the_owner() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let m = h.member.clone();
    let account = paper_account(&h, &m, "notify-diverge").await;
    let config = strategy_config(&h, &m, "notify-diverge-cfg").await;
    // Same lineage, DIFFERENT weights: the comparison is meaningful and it
    // fails, which is precisely the case a user must not miss.
    let run_id = seed_backtest_side(
        &h,
        &m,
        &config,
        &[("069500.KRX", "0.900000"), ("229200.KRX", "0.100000")],
    )
    .await;
    let target = queue_target(&h, &m, &account, &config).await;
    bind_target_backtest_lineage(&h, &m, target, run_id).await;

    seed_executed_session(&h, &m, &account).await;
    let outcome = settle_and_announce(&h.state(), &m.actor(), target, SessionOutcome::Executed)
        .await
        .expect("settles and announces");

    let parity = outcome.parity.as_ref().expect("parity report");
    assert_exact_dataset_lineage(parity);
    assert_eq!(parity.status, ParityStatus::Divergent);
    assert_eq!(outcome.severity, AlertSeverity::Warning);
    let items = feed(&h, &m).await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["kind"], "alert");
    assert!(
        items[0]["title"]
            .as_str()
            .is_some_and(|t| t.contains("diverged")),
        "the title names the divergence: {}",
        items[0]["title"]
    );

    // WARNING also raises an immediate admin alert to the Owner.
    let owner_items = feed(&h, &h.owner).await;
    assert_eq!(
        owner_items.len(),
        1,
        "a member's WARNING reaches the Owner's admin feed"
    );
    assert_eq!(
        owner_items[0]["deliveries"][0]["channel"], "admin",
        "the Owner's leg is the admin channel"
    );
    h.teardown().await;
}

// ---------------------------------------------------------------------------
// Stale/absent lineage cannot be compared, and says so.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_session_without_a_backtest_warns_not_comparable() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let m = h.member.clone();
    let account = paper_account(&h, &m, "notify-nocmp").await;
    let config = strategy_config(&h, &m, "notify-nocmp-cfg").await;
    // No recommendation run seeded: there is no backtest side at all.
    let target = queue_target(&h, &m, &account, &config).await;

    seed_executed_session(&h, &m, &account).await;
    let outcome = settle_and_announce(&h.state(), &m.actor(), target, SessionOutcome::Executed)
        .await
        .expect("settles and announces");

    assert_eq!(
        outcome.parity.as_ref().map(|p| p.status),
        Some(ParityStatus::NotComparable),
        "a missing side can never read as a match"
    );
    assert_eq!(outcome.severity, AlertSeverity::Warning);
    let items = feed(&h, &m).await;
    assert!(
        items[0]["title"]
            .as_str()
            .is_some_and(|t| t.contains("cannot be compared")),
        "the notice states the limit rather than claiming parity: {}",
        items[0]["title"]
    );
    h.teardown().await;
}

// ---------------------------------------------------------------------------
// Blocked and failed sessions are recorded, never silent.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_blocked_session_is_recorded_and_the_target_stays_auditable() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let m = h.member.clone();
    let account = paper_account(&h, &m, "notify-blocked").await;
    let config = strategy_config(&h, &m, "notify-blocked-cfg").await;
    let target = queue_target(&h, &m, &account, &config).await;

    let outcome = settle_and_announce(
        &h.state(),
        &m.actor(),
        target,
        SessionOutcome::Blocked {
            reason: "the data entitlement is paused".to_owned(),
        },
    )
    .await
    .expect("settles and announces");

    assert_eq!(outcome.target.status, "SKIPPED");
    assert!(
        outcome.parity.is_none(),
        "a session that never traded has no Paper signals to compare"
    );
    assert_eq!(outcome.severity, AlertSeverity::Warning);
    let items = feed(&h, &m).await;
    assert!(
        items[0]["body"]
            .as_str()
            .is_some_and(|b| b.contains("the data entitlement is paused")),
        "the block reason is carried to the user: {}",
        items[0]["body"]
    );

    // The target survives as an auditable SKIPPED row, not a hole.
    let history = h
        .state_pending_targets()
        .history(&m.actor(), Uuid::parse_str(&account).unwrap())
        .await
        .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].status, "SKIPPED");
    h.teardown().await;
}

#[tokio::test]
async fn a_failed_session_escalates_critical() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let m = h.member.clone();
    let account = paper_account(&h, &m, "notify-failed").await;
    let config = strategy_config(&h, &m, "notify-failed-cfg").await;
    let target = queue_target(&h, &m, &account, &config).await;

    let outcome = settle_and_announce(
        &h.state(),
        &m.actor(),
        target,
        SessionOutcome::Failed {
            reason: "the session open price was never published".to_owned(),
        },
    )
    .await
    .expect("settles and announces");

    assert_eq!(outcome.severity, AlertSeverity::Critical);
    assert_eq!(
        outcome.target.status, "SKIPPED",
        "a failed target must not stay PENDING and be re-claimed forever"
    );
    let owner_items = feed(&h, &h.owner).await;
    assert_eq!(owner_items.len(), 1, "CRITICAL reaches the Owner");
    h.teardown().await;
}

// ---------------------------------------------------------------------------
// A duplicate claim announces nothing: one session, one notification.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_second_runner_claiming_the_same_session_announces_nothing() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let m = h.member.clone();
    let account = paper_account(&h, &m, "notify-dup").await;
    let config = strategy_config(&h, &m, "notify-dup-cfg").await;
    let target = queue_target(&h, &m, &account, &config).await;

    seed_executed_session(&h, &m, &account).await;
    settle_and_announce(&h.state(), &m.actor(), target, SessionOutcome::Executed)
        .await
        .expect("the first runner settles");
    let second =
        settle_and_announce(&h.state(), &m.actor(), target, SessionOutcome::Executed).await;
    assert!(second.is_err(), "the second claim finds nothing to settle");

    let items = feed(&h, &m).await;
    assert_eq!(
        items.len(),
        1,
        "a duplicate claim must not duplicate the notification"
    );
    h.teardown().await;
}

// ---------------------------------------------------------------------------
// A notification outage is recorded on the recipient's own feed.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_email_outage_is_visible_in_the_recipients_own_feed() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let m = h.member.clone();
    // Configurable per kind: the member subscribes email for `alert`, which
    // is the kind a divergence routes.
    let resp = h
        .send(
            "PUT",
            "/api/v1/notifications/subscriptions",
            Some(&m),
            true,
            Some("test-rid-1"),
            Some("paper-sub-key"),
            Some(json!({ "kind": "alert", "channel": "email", "enabled": true })),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);

    let account = paper_account(&h, &m, "notify-outage").await;
    let config = strategy_config(&h, &m, "notify-outage-cfg").await;
    let target = queue_target(&h, &m, &account, &config).await;
    let settlement_error = settle_and_announce(
        &h.state(),
        &m.actor(),
        target,
        SessionOutcome::Blocked {
            reason: "no close was published for this session".to_owned(),
        },
    )
    .await
    .expect_err("email outage is surfaced after durable settlement");
    assert!(
        settlement_error
            .to_string()
            .contains("email delivery not configured"),
        "unexpected durable email failure: {settlement_error}"
    );

    let items = feed(&h, &m).await;
    let deliveries = items[0]["deliveries"].as_array().expect("deliveries");
    let email = deliveries
        .iter()
        .find(|d| d["channel"] == "email")
        .expect("the subscribed email leg was attempted");
    assert_eq!(
        email["status"], "FAILED",
        "an outage is recorded, never silent"
    );
    assert!(
        email["error_detail"]
            .as_str()
            .is_some_and(|e| !e.is_empty()),
        "a failed delivery carries its reason"
    );
    // The web leg still succeeded, so the user is not left with nothing.
    assert!(
        deliveries
            .iter()
            .any(|d| d["channel"] == "web" && d["status"] == "SUCCESS")
    );
    h.teardown().await;
}

// ---------------------------------------------------------------------------
// AT-07: two members' sessions never cross feeds.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn two_members_settlements_never_cross_feeds() {
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
    let acct1 = paper_account(&h, &m1, "cross-1").await;
    let acct2 = paper_account(&h, &m2, "cross-2").await;
    let cfg1 = strategy_config(&h, &m1, "cross-cfg-1").await;
    let cfg2 = strategy_config(&h, &m2, "cross-cfg-2").await;
    let t1 = queue_target(&h, &m1, &acct1, &cfg1).await;
    queue_target(&h, &m2, &acct2, &cfg2).await;

    seed_executed_session(&h, &m1, &acct1).await;
    settle_and_announce(&h.state(), &m1.actor(), t1, SessionOutcome::Executed)
        .await
        .expect("member 1 settles their own session");

    assert_eq!(feed(&h, &m1).await.len(), 1, "member 1 sees their notice");
    assert!(
        feed(&h, &m2).await.is_empty(),
        "member 2 never sees another member's Paper session"
    );

    // Member 2 cannot settle — and therefore cannot announce — member 1's.
    let stolen = settle_and_announce(&h.state(), &m2.actor(), t1, SessionOutcome::Executed).await;
    assert!(
        stolen.is_err(),
        "a foreign target is indistinguishable from missing"
    );
    h.teardown().await;
}

/// A session claimed EXECUTED with nothing in the ledger is not announced as
/// a completion.
///
/// `SessionOutcome::Executed` documents itself as "The session's orders and
/// fills are in the ledger", and nothing used to check it. As of this writing
/// no code in this server writes `orders`, `fills` or `positions` at all, so
/// every `Executed` a runner could produce would have been this case: the
/// target flipped to EXECUTED and the user told INFO -- the completion notice
/// -- for a session that recorded nothing.
///
/// The downgrade is to `Failed`, not an error. Returning an error would leave
/// the row PENDING, and `SessionOutcome`'s own comment explains why that is
/// worse: "a PENDING row would be re-claimed forever". A runner claiming
/// execution it cannot evidence is broken, which is what CRITICAL is for.
#[tokio::test]
async fn a_session_that_recorded_nothing_is_not_announced_as_complete() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let m = h.member.clone();
    let account = paper_account(&h, &m, "notify-hollow").await;
    let config = strategy_config(&h, &m, "notify-hollow-cfg").await;
    let target = queue_target(&h, &m, &account, &config).await;

    // No seed_executed_session: the ledger holds nothing for this session.
    let outcome = settle_and_announce(&h.state(), &m.actor(), target, SessionOutcome::Executed)
        .await
        .expect("the session still settles rather than staying PENDING forever");

    assert_eq!(
        outcome.target.status, "SKIPPED",
        "a session with no orders must not settle EXECUTED"
    );
    assert_eq!(
        outcome.severity,
        AlertSeverity::Critical,
        "a runner that claims execution it cannot evidence is broken, not merely blocked"
    );
    assert!(
        outcome.parity.is_none(),
        "there is nothing to compare a session that traded nothing against"
    );

    let items = feed(&h, &m).await;
    assert_eq!(items.len(), 1, "exactly one notice per session");
    let body = format!("{items:?}");
    assert!(
        body.contains("recorded nothing"),
        "the notice must say what was missing: {body}"
    );

    h.teardown().await;
}

/// A deterministic stand-in for a runner cancellation immediately after the
/// terminal target write: leave the committed outbox row untouched, then let
/// the recovery scan dispatch it.  Replaying the same row proves the
/// notification source key and per-channel uniqueness are idempotent.  The
/// rollback half proves a failed transaction cannot leave either side behind.
#[tokio::test]
async fn failure_settlement_outbox_recovers_after_cancel_and_retries_once() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let member = h.member.clone();
    let account = paper_account(&h, &member, "notify-outbox-recovery").await;
    let config = strategy_config(&h, &member, "notify-outbox-recovery-cfg").await;
    let target = queue_target(&h, &member, &account, &config).await;
    let announcement = PaperSettlementAnnouncement {
        severity: "WARNING".to_owned(),
        kind: "alert".to_owned(),
        title: "Paper settlement recovery test".to_owned(),
        body: "the dispatcher was canceled after settlement".to_owned(),
        parity_json: Some(json!({ "status": "MATCH" })),
        non_execution_reason: None,
    };

    // The target/outbox transaction commits, then this test intentionally
    // pauses before dispatch — the exact crash boundary that used to lose an
    // alert forever.
    let (settled, outbox) = h
        .state_pending_targets()
        .settle_with_announcement(&member.actor(), target, "SKIPPED", &announcement)
        .await
        .expect("terminal target and outbox commit together");
    assert_eq!(settled.status, "SKIPPED");
    let worker = h.worker_pool().await;
    let pending: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM paper_settlement_outbox \
         WHERE id=$1 AND delivered_at IS NULL",
    )
    .bind(outbox.id)
    .fetch_one(&worker)
    .await
    .expect("pending outbox row");
    assert_eq!(pending, 1, "cancellation leaves a recoverable intent");

    let notifier = h.state().notifier();
    let first_notifier = notifier.clone();
    let second_notifier = notifier.clone();
    let first_actor = member.actor();
    let second_actor = member.actor();
    let first_outbox = outbox.clone();
    let second_outbox = outbox.clone();
    let (first, second) = tokio::join!(
        async move {
            first_notifier
                .dispatch_paper_settlement(&first_actor, &first_outbox)
                .await
        },
        async move {
            second_notifier
                .dispatch_paper_settlement(&second_actor, &second_outbox)
                .await
        }
    );
    let first = first.expect("first recovery dispatch");
    let second = second.expect("second recovery dispatch");
    assert!(
        [first, second]
            .iter()
            .any(|alerts| alerts.deliveries.iter().any(|d| d.status == "SUCCESS")),
        "one concurrent runner must own the durable delivery lease"
    );
    h.state_pending_targets()
        .mark_announcement_delivered(&member.actor(), outbox.id, outbox.claim_token)
        .await
        .expect("mark dispatched");

    // A retry after a worker restart sees the same immutable source key.  It
    // must return the same rows rather than append another notification.
    notifier
        .dispatch_paper_settlement(&member.actor(), &outbox)
        .await
        .expect("idempotent retry");
    let member_pool = h.member_pool().await;
    let member_counts: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM notifications WHERE owner_user_id=$1), \
                (SELECT count(*) FROM notification_deliveries WHERE owner_user_id=$1)",
    )
    .bind(member.user_id)
    .fetch_one(&member_pool)
    .await
    .expect("member notification counts");
    assert_eq!(member_counts, (1, 1));
    let owner_counts: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM notifications WHERE owner_user_id=$1), \
                (SELECT count(*) FROM notification_deliveries WHERE owner_user_id=$1)",
    )
    .bind(h.owner.user_id)
    .fetch_one(&h.admin_pool)
    .await
    .expect("owner notification counts");
    assert_eq!(owner_counts, (1, 1));
    let parity: serde_json::Value =
        sqlx::query_scalar("SELECT parity_json FROM paper_settlement_outbox WHERE id=$1")
            .bind(outbox.id)
            .fetch_one(&worker)
            .await
            .expect("durable parity snapshot");
    assert_eq!(parity["status"], "MATCH");

    // Rollback injection: neither a terminal target nor an outbox intent is
    // visible when the transaction is canceled before commit.
    let rollback_account = paper_account(&h, &member, "notify-outbox-rollback").await;
    let rollback_config = strategy_config(&h, &member, "notify-outbox-rollback-cfg").await;
    let rollback_target = queue_target(&h, &member, &rollback_account, &rollback_config).await;
    let mut tx = api_server::actor_tx::begin_actor_tx(&h.app_pool, &member.actor())
        .await
        .expect("rollback transaction");
    sqlx::query("UPDATE pending_targets SET status='SKIPPED', executed_at=now() WHERE id=$1")
        .bind(rollback_target)
        .execute(&mut *tx)
        .await
        .expect("terminal update before injected rollback");
    sqlx::query_scalar::<_, Uuid>(
        "SELECT public.enqueue_paper_settlement_outbox(\
                 $1, 'WARNING', 'alert', 'rollback', 'rollback', NULL)",
    )
    .bind(rollback_target)
    .fetch_one(&mut *tx)
    .await
    .expect("outbox enqueue before injected rollback");
    tx.rollback().await.expect("injected cancellation rollback");
    let status: String = sqlx::query_scalar("SELECT status FROM pending_targets WHERE id=$1")
        .bind(rollback_target)
        .fetch_one(&member_pool)
        .await
        .expect("rolled back target");
    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM paper_settlement_outbox WHERE pending_target_id=$1",
    )
    .bind(rollback_target)
    .fetch_one(&worker)
    .await
    .expect("rolled back outbox");
    assert_eq!(status, "PENDING");
    assert_eq!(outbox_count, 0);
    h.teardown().await;
}
