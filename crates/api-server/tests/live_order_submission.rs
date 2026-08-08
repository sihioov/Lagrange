//! The full Live submission path, end to end against a scripted broker.
//!
//! `live_simulator_integration` proved the pieces compose when a test drives
//! them by hand. This proves the ORCHESTRATION does it — the single function a
//! route will call — including the parts a hand-driven test cannot get wrong
//! because it never takes the wrong branch.
//!
//! The properties asserted here are the ones that cost money if they fail:
//! a retry never places a second order, a denial never reaches the broker, an
//! ambiguous timeout is never reported as a failure, and a dry run writes
//! nothing at all.

mod common;

use api_server::live_order::{Mode, Submission, SubmitRequest, submit};
use api_server::repos::order_intents::{NewOrderIntent, OrderIntentRepo};
use api_server::repos::risk::RiskRepo;
use common::{Harness, actor_pool};
use kis_client::auth::{AccessToken, TokenIssuer, TokenManager};
use kis_client::clock::TestClock;
use kis_client::error::KisError;
use kis_client::idempotency::InMemoryIntentStore;
use kis_client::rate_limit::{Quota, RateLimiter};
use kis_client::rest::{Profile, RestClient};
use kis_client::retry::Sleeper;
use kis_client::secret::{AccountNo, Secret};
use kis_client::simulator::{BrokerSimulator, Scenario};
use std::sync::Arc;
use uuid::Uuid;

const ORDER_PATH: &str = "/uapi/domestic-stock/v1/trading/order-cash";

struct NoSleep;
impl Sleeper for NoSleep {
    async fn sleep_ms(&self, _ms: u64) {}
}

struct AlwaysIssues;
#[async_trait::async_trait]
impl TokenIssuer for AlwaysIssues {
    async fn issue(&self) -> Result<AccessToken, KisError> {
        Ok(AccessToken {
            value: Secret::new("simulated-access-token".to_string()),
            expires_at_ms: i64::MAX,
        })
    }
}

fn broker(sim: BrokerSimulator) -> RestClient<BrokerSimulator, NoSleep> {
    let clock = TestClock::at(0);
    RestClient::new(
        Profile::Mock,
        sim,
        NoSleep,
        Arc::new(TokenManager::new(
            Arc::new(clock.clone()),
            Arc::new(AlwaysIssues),
        )),
        Arc::new(RateLimiter::new(Arc::new(clock), Quota::new(100, 100))),
        Arc::new(InMemoryIntentStore::new()),
        AccountNo::new("50123456"),
        "01",
    )
}

async fn live_account(h: &Harness) -> Uuid {
    let pool = actor_pool(&h.app_url, &h.owner.user_id.to_string(), 2).await;
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO accounts (owner_user_id, account_type, name, currency) \
         VALUES ($1, 'LIVE', 'submit-live', 'KRW') \
         ON CONFLICT (owner_user_id, name) DO UPDATE SET currency = EXCLUDED.currency \
         RETURNING id",
    )
    .bind(h.owner.user_id)
    .fetch_one(&pool)
    .await
    .expect("live account")
}

async fn publish_limits(h: &Harness) {
    sqlx::query(
        "INSERT INTO risk_limits (version, max_symbol_weight_bp, max_order_value, \
         max_daily_order_value, max_daily_loss, max_data_age_secs) \
         VALUES ('risk-limits-v1', 3000, 1000000, 5000000, 500000, 300) \
         ON CONFLICT (version) DO NOTHING",
    )
    .execute(&h.owner_pool)
    .await
    .expect("limits");
}

fn new_intent(account: Uuid, client_key: &str) -> NewOrderIntent {
    NewOrderIntent {
        intent_ref: NewOrderIntent::mint_ref(),
        account_id: account,
        instrument_id: "069500.KRX".into(),
        side: "BUY".into(),
        quantity: "10".into(),
        price: Some("7250".into()),
        correlation_id: "corr-submit".into(),
        client_key: client_key.to_string(),
    }
}

fn request(account: Uuid, client_key: &str, mode: Mode) -> SubmitRequest {
    SubmitRequest {
        intent: new_intent(account, client_key),
        snapshot: risk_gateway::testing::snapshot_all_green(),
        limits: risk_gateway::testing::limits(),
        mode,
    }
}

fn repos(h: &Harness, account: Uuid) -> (OrderIntentRepo, RiskRepo) {
    (
        OrderIntentRepo::new(h.app_pool.clone(), h.owner.actor(), h.owner.user_id),
        RiskRepo::new(
            h.app_pool.clone(),
            h.owner.actor(),
            h.owner.user_id,
            Some(account),
        ),
    )
}

#[tokio::test]
async fn a_dry_run_writes_nothing_and_sends_nothing() {
    // A prediction that mutates the world is not a prediction. Recording a
    // decision would consume this order's ONE permitted gate decision (0018),
    // so the real submission could never be authorised afterwards -- the
    // rehearsal would have destroyed the performance.
    let Some(h) = Harness::new().await else {
        return;
    };
    publish_limits(&h).await;
    let account = live_account(&h).await;
    let (intents, risk) = repos(&h, account);
    let sim = BrokerSimulator::new();
    let client = broker(sim);

    let out = submit(
        &intents,
        &risk,
        &client,
        request(account, "ck-dry", Mode::DryRun),
    )
    .await
    .expect("dry run");

    match out {
        Submission::Rehearsed { would_submit, .. } => assert!(would_submit),
        other => panic!("expected a rehearsal, got {other:?}"),
    }

    let pool = actor_pool(&h.app_url, &h.owner.user_id.to_string(), 2).await;
    let intents_n: i64 = sqlx::query_scalar("SELECT count(*) FROM order_intents")
        .fetch_one(&pool)
        .await
        .expect("count");
    let decisions_n: i64 = sqlx::query_scalar("SELECT count(*) FROM risk_events")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(intents_n, 0, "a dry run must create no intent");
    assert_eq!(decisions_n, 0, "a dry run must record no gate decision");
}

#[tokio::test]
async fn a_dry_run_reports_a_refusal_without_pretending_it_was_submitted() {
    let Some(h) = Harness::new().await else {
        return;
    };
    publish_limits(&h).await;
    let account = live_account(&h).await;
    let (intents, risk) = repos(&h, account);
    let client = broker(BrokerSimulator::new());

    let mut req = request(account, "ck-dry-blocked", Mode::DryRun);
    req.snapshot.kill_switch = risk_gateway::snapshot::KillSwitch::Engaged;

    match submit(&intents, &risk, &client, req)
        .await
        .expect("dry run")
    {
        Submission::Rehearsed {
            would_submit,
            decision,
        } => {
            assert!(!would_submit);
            assert_eq!(
                decision.reason.map(|r| r.as_str()),
                Some("LIVE_KILL_SWITCH_ENGAGED"),
                "a rehearsal must name the same reason a real run would"
            );
        }
        other => panic!("expected a rehearsal, got {other:?}"),
    }
}

#[tokio::test]
async fn a_live_order_reaches_the_broker_and_records_its_order_number() {
    let Some(h) = Harness::new().await else {
        return;
    };
    publish_limits(&h).await;
    let account = live_account(&h).await;
    let (intents, risk) = repos(&h, account);
    let client = broker(BrokerSimulator::new().script(
        "POST",
        ORDER_PATH,
        vec![Scenario::Ok {
            body: BrokerSimulator::order_ack("0000200001"),
        }],
    ));

    match submit(
        &intents,
        &risk,
        &client,
        request(account, "ck-live-1", Mode::Live),
    )
    .await
    .expect("submit")
    {
        Submission::Accepted {
            intent_ref,
            broker_order_no,
        } => {
            assert_eq!(broker_order_no, "0000200001");
            let row = intents.get(&intent_ref).await.expect("row");
            assert_eq!(row.state, "ACCEPTED");
            assert_eq!(row.broker_order_no.as_deref(), Some("0000200001"));
        }
        other => panic!("expected Accepted, got {other:?}"),
    }
}

#[tokio::test]
async fn a_retry_with_the_same_idempotency_key_never_places_a_second_order() {
    // FR-LIVE-003, at the level a route actually calls. The retransmission
    // carries a FRESH minted ref -- as a real route would -- and must still
    // resolve to the first intent.
    let Some(h) = Harness::new().await else {
        return;
    };
    publish_limits(&h).await;
    let account = live_account(&h).await;
    let (intents, risk) = repos(&h, account);
    let sim = BrokerSimulator::new().script(
        "POST",
        ORDER_PATH,
        vec![Scenario::Ok {
            body: BrokerSimulator::order_ack("0000200002"),
        }],
    );
    let client = broker(sim);

    let first = submit(
        &intents,
        &risk,
        &client,
        request(account, "ck-retry", Mode::Live),
    )
    .await
    .expect("first");
    let original_ref = match &first {
        Submission::Accepted { intent_ref, .. } => intent_ref.clone(),
        other => panic!("expected Accepted, got {other:?}"),
    };

    let second = submit(
        &intents,
        &risk,
        &client,
        request(account, "ck-retry", Mode::Live),
    )
    .await
    .expect("second");

    match second {
        Submission::AlreadySubmitted { intent_ref, state } => {
            assert_eq!(
                intent_ref, original_ref,
                "the retry resolves to the FIRST intent"
            );
            assert_eq!(state, "ACCEPTED");
        }
        other => panic!("a retry must not re-submit; got {other:?}"),
    }

    let pool = actor_pool(&h.app_url, &h.owner.user_id.to_string(), 2).await;
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM order_intents WHERE client_key = $1")
        .bind("ck-retry")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(n, 1, "one client key yields exactly one intent, ever");
}

#[tokio::test]
async fn a_denied_order_never_reaches_the_broker() {
    let Some(h) = Harness::new().await else {
        return;
    };
    publish_limits(&h).await;
    let account = live_account(&h).await;
    let (intents, risk) = repos(&h, account);
    // Scripted to ACCEPT, so if the gate leaked an order through, this test
    // would report success rather than silence.
    let sim = BrokerSimulator::new().script(
        "POST",
        ORDER_PATH,
        vec![Scenario::Ok {
            body: BrokerSimulator::order_ack("0000200003"),
        }],
    );
    let client = broker(sim);

    let mut req = request(account, "ck-denied", Mode::Live);
    req.snapshot.kill_switch = risk_gateway::snapshot::KillSwitch::Engaged;

    match submit(&intents, &risk, &client, req).await.expect("submit") {
        Submission::Denied {
            intent_ref,
            reason,
            severity,
        } => {
            assert_eq!(reason, "LIVE_KILL_SWITCH_ENGAGED");
            assert_eq!(severity, "CRITICAL");
            assert_eq!(
                intents.get(&intent_ref).await.expect("row").state,
                "DENIED",
                "the denial is recorded on the intent, not merely returned"
            );
        }
        other => panic!("expected Denied, got {other:?}"),
    }
}

#[tokio::test]
async fn an_ambiguous_timeout_is_unresolved_rather_than_failed() {
    // The single most dangerous outcome. Reporting it as a failure invites a
    // retry, and the broker may already hold the order.
    let Some(h) = Harness::new().await else {
        return;
    };
    publish_limits(&h).await;
    let account = live_account(&h).await;
    let (intents, risk) = repos(&h, account);
    let client = broker(BrokerSimulator::new().script("POST", ORDER_PATH, vec![Scenario::Timeout]));

    match submit(
        &intents,
        &risk,
        &client,
        request(account, "ck-timeout", Mode::Live),
    )
    .await
    .expect("submit")
    {
        Submission::Unresolved {
            intent_ref,
            client_order_id,
        } => {
            assert_eq!(client_order_id, intent_ref);
            assert_eq!(
                intents.get(&intent_ref).await.expect("row").state,
                "UNKNOWN",
                "an ambiguous submission must leave the intent UNKNOWN, never REJECTED"
            );
        }
        other => panic!("a timeout must be Unresolved, not {other:?}"),
    }
}

#[tokio::test]
async fn a_request_that_never_left_terminates_the_intent_as_rejected() {
    // The complement of the case above, and what makes it meaningful: nothing
    // reached the broker, so no order exists and the intent is safely closed.
    // A retry is then a NEW intent with a new gate run.
    let Some(h) = Harness::new().await else {
        return;
    };
    publish_limits(&h).await;
    let account = live_account(&h).await;
    let (intents, risk) = repos(&h, account);
    let client = broker(BrokerSimulator::new().script(
        "POST",
        ORDER_PATH,
        vec![Scenario::Unreachable {
            reason: "connection refused".into(),
        }],
    ));

    match submit(
        &intents,
        &risk,
        &client,
        request(account, "ck-unreachable", Mode::Live),
    )
    .await
    .expect("submit")
    {
        Submission::Denied { intent_ref, .. } => {
            assert_eq!(
                intents.get(&intent_ref).await.expect("row").state,
                "REJECTED",
                "a request that never left leaves no order, so the intent closes"
            );
        }
        other => panic!("expected Denied, got {other:?}"),
    }
}
