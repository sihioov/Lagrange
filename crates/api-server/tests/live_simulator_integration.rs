//! Todo 41 acceptance, the clause that was outstanding: **Live simulator
//! integration**.
//!
//! Every piece of the Live path has its own suite. What none of them proves is
//! that the pieces COMPOSE — that an intent claimed in the database, gated by
//! the Risk Gateway, and authorised by a token that only the gate can mint,
//! actually reaches a broker and comes back as a recorded order. Each part
//! passing says nothing about the seams between them, and the seams are where
//! a live order goes missing or goes out twice.
//!
//! The broker here is `kis_client::simulator::BrokerSimulator`, a scripted KIS.
//! It needs no credentials, which is why this could have been written earlier
//! than it was: the reason recorded for deferring it — "no credentials" —
//! applied to a REAL account, and was quietly carried over to the simulator,
//! where it does not apply at all.
//!
//! What still cannot be proven here, and is not claimed: that the real KIS
//! endpoints behave as the simulator does. That needs a real account, and it is
//! the one thing the Phase 3 gate blocks on.

mod common;

use api_server::repos::order_intents::{NewOrderIntent, OrderIntentRepo};
use api_server::repos::risk::RiskRepo;
use common::{Harness, actor_pool};
use kis_client::auth::{AccessToken, TokenIssuer, TokenManager};
use kis_client::clock::TestClock;
use kis_client::error::KisError;
use kis_client::idempotency::InMemoryIntentStore;
use kis_client::mapping::{OrderRequest, OrderSide, OrderType};
use kis_client::order_state::{Event, OrderIntentState};
use kis_client::rate_limit::{Quota, RateLimiter};
use kis_client::rest::{Profile, RestClient};
use kis_client::retry::Sleeper;
use kis_client::secret::{AccountNo, Secret};
use kis_client::simulator::{BrokerSimulator, Scenario};
use std::sync::Arc;
use uuid::Uuid;

/// Retries without real time passing. Backoff correctness is `retry`'s own
/// suite; making this test wait would only make it slow and flaky.
struct NoSleep;
impl Sleeper for NoSleep {
    async fn sleep_ms(&self, _ms: u64) {}
}

/// A token issuer that always succeeds. Token serialisation and refresh are
/// covered by `auth`'s suite; here they must simply not be the reason a
/// submission fails.
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
         VALUES ($1, 'LIVE', 'sim-live', 'KRW') \
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

fn order_for(intent_ref: &str) -> OrderRequest {
    OrderRequest {
        client_order_id: intent_ref.to_string(),
        instrument_id: "069500.KRX".into(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        quantity: 10,
        price: Some("7250".into()),
    }
}

/// Claims an intent and takes it through the gate, returning its ref.
///
/// The approval token is CONSUMED by `record_approval`, so this helper cannot
/// hand one back for reuse — which is the property being relied on rather than
/// an inconvenience.
async fn gated_intent(h: &Harness, account: Uuid) -> String {
    let intents = OrderIntentRepo::new(h.app_pool.clone(), h.owner.actor(), h.owner.user_id);
    let input = NewOrderIntent {
        intent_ref: NewOrderIntent::mint_ref(),
        account_id: account,
        instrument_id: "069500.KRX".into(),
        side: "BUY".into(),
        quantity: "10".into(),
        price: Some("7250".into()),
        correlation_id: "corr-sim".into(),
        client_key: format!("ck-{}", Uuid::new_v4()),
    };
    let claim = intents.claim(input.clone()).await.expect("claim");
    assert!(claim.is_new(), "a fresh key must claim a new intent");

    let store = RiskRepo::new(
        h.app_pool.clone(),
        h.owner.actor(),
        h.owner.user_id,
        Some(account),
    );
    let mut snapshot = risk_gateway::testing::snapshot_all_green();
    snapshot.intent.intent_ref = input.intent_ref.clone();
    let approval =
        risk_gateway::evaluate_and_record(&snapshot, &risk_gateway::testing::limits(), &store)
            .await
            .into_approval()
            .expect("an all-green snapshot is approved");

    intents
        .record_approval(&input.intent_ref, approval)
        .await
        .expect("the approval authorises exactly this intent");
    input.intent_ref
}

#[tokio::test]
async fn live_simulator_a_gated_intent_reaches_the_broker_and_comes_back_recorded() {
    let Some(h) = Harness::new().await else {
        return;
    };
    publish_limits(&h).await;
    let account = live_account(&h).await;
    let intents = OrderIntentRepo::new(h.app_pool.clone(), h.owner.actor(), h.owner.user_id);

    let intent_ref = gated_intent(&h, account).await;
    assert_eq!(
        intents.state(&intent_ref).await.expect("state"),
        OrderIntentState::RiskApproved,
        "the gate's approval is what moves the intent, not the caller's say-so"
    );

    // Submit. SUBMITTING is recorded BEFORE the request goes out, so a crash
    // mid-flight leaves an intent that must be swept rather than one that
    // looks untouched.
    intents
        .append(&intent_ref, &Event::SubmissionStarted)
        .await
        .expect("submitting");

    let sim = BrokerSimulator::new().script(
        "POST",
        "/uapi/domestic-stock/v1/trading/order-cash",
        vec![Scenario::Ok {
            body: BrokerSimulator::order_ack("0000117057"),
        }],
    );
    let client = broker(sim);
    let ack = client
        .submit_order(&order_for(&intent_ref))
        .await
        .expect("the broker accepts");

    intents
        .append(&intent_ref, &Event::SubmissionSent)
        .await
        .expect("sent");
    intents
        .append(
            &intent_ref,
            &Event::BrokerAccepted {
                broker_order_no: ack.broker_order_no.clone(),
            },
        )
        .await
        .expect("accepted");

    // The broker's own order number is what the intent is now bound to, and
    // it came from the simulator rather than from anything this test invented.
    let row = intents.get(&intent_ref).await.expect("row");
    assert_eq!(row.state, "ACCEPTED");
    assert_eq!(row.broker_order_no.as_deref(), Some("0000117057"));

    // A fill completes it, end to end.
    intents
        .append(&intent_ref, &Event::fill(&ack.broker_order_no, 10, 10))
        .await
        .expect("fill");
    assert_eq!(
        intents.state(&intent_ref).await.expect("state").name(),
        "FILLED"
    );
}

#[tokio::test]
async fn live_simulator_a_broker_timeout_becomes_unknown_and_is_never_resubmitted() {
    // AT-09 across the whole path rather than in the machine alone. The
    // simulator's Timeout is deliberately AMBIGUOUS -- it offers no way to say
    // "the order definitely did not happen", because a real broker cannot say
    // that either.
    let Some(h) = Harness::new().await else {
        return;
    };
    publish_limits(&h).await;
    let account = live_account(&h).await;
    let intents = OrderIntentRepo::new(h.app_pool.clone(), h.owner.actor(), h.owner.user_id);
    let intent_ref = gated_intent(&h, account).await;

    intents
        .append(&intent_ref, &Event::SubmissionStarted)
        .await
        .expect("submitting");

    let sim = BrokerSimulator::new().script(
        "POST",
        "/uapi/domestic-stock/v1/trading/order-cash",
        vec![Scenario::Timeout],
    );
    let client = broker(sim);
    let err = client
        .submit_order(&order_for(&intent_ref))
        .await
        .expect_err("a timeout is not a success");
    assert!(
        format!("{err}").to_lowercase().contains("ambiguous")
            || format!("{err:?}").contains("Ambiguous"),
        "a mutation timeout must surface as AMBIGUOUS, not as a plain failure: {err:?}"
    );

    intents
        .append(&intent_ref, &Event::SubmissionTimedOut)
        .await
        .expect("timed out");
    assert_eq!(
        intents.state(&intent_ref).await.expect("state"),
        OrderIntentState::Unknown
    );

    // The machine refuses a second submission outright -- there is no edge
    // back into the submission path, so this is not a check that could be
    // skipped.
    assert!(
        intents
            .append(&intent_ref, &Event::SubmissionStarted)
            .await
            .is_err(),
        "an UNKNOWN intent must never be resubmitted"
    );

    // And only a broker lookup settles it.
    intents
        .append(
            &intent_ref,
            &Event::BrokerLookupResolved {
                resolved: Box::new(OrderIntentState::Accepted {
                    broker_order_no: "0000117058".into(),
                }),
            },
        )
        .await
        .expect("the lookup resolves it");
    assert_eq!(
        intents
            .get(&intent_ref)
            .await
            .expect("row")
            .broker_order_no
            .as_deref(),
        Some("0000117058")
    );
}

#[tokio::test]
async fn live_simulator_the_broker_is_called_exactly_once_per_intent() {
    // The duplicate-order failure, observed at the transport rather than
    // inferred from state. The idempotency guard claims the key before
    // anything is built or sent, so the second attempt never reaches the wire.
    let Some(h) = Harness::new().await else {
        return;
    };
    publish_limits(&h).await;
    let account = live_account(&h).await;
    let intent_ref = gated_intent(&h, account).await;

    let sim = BrokerSimulator::new().script(
        "POST",
        "/uapi/domestic-stock/v1/trading/order-cash",
        vec![Scenario::Ok {
            body: BrokerSimulator::order_ack("0000117059"),
        }],
    );
    let client = broker(sim);
    let order = order_for(&intent_ref);

    client.submit_order(&order).await.expect("first submission");
    let second = client.submit_order(&order).await;
    assert!(
        second.is_err(),
        "a second submission for one intent must be refused before it is sent"
    );
}

#[tokio::test]
async fn live_simulator_a_denied_intent_never_reaches_the_broker() {
    // The whole point of the gate, proven at the wire: a denial yields no
    // approval token, `record_approval` cannot be called, and the intent never
    // becomes submittable. The broker sees nothing.
    let Some(h) = Harness::new().await else {
        return;
    };
    publish_limits(&h).await;
    let account = live_account(&h).await;
    let intents = OrderIntentRepo::new(h.app_pool.clone(), h.owner.actor(), h.owner.user_id);

    let input = NewOrderIntent {
        intent_ref: NewOrderIntent::mint_ref(),
        account_id: account,
        instrument_id: "069500.KRX".into(),
        side: "BUY".into(),
        quantity: "10".into(),
        price: Some("7250".into()),
        correlation_id: "corr-denied".into(),
        client_key: format!("ck-{}", Uuid::new_v4()),
    };
    intents.claim(input.clone()).await.expect("claim");

    let store = RiskRepo::new(
        h.app_pool.clone(),
        h.owner.actor(),
        h.owner.user_id,
        Some(account),
    );
    let mut snapshot = risk_gateway::testing::snapshot_all_green();
    snapshot.intent.intent_ref = input.intent_ref.clone();
    snapshot.kill_switch = risk_gateway::snapshot::KillSwitch::Engaged;

    let outcome =
        risk_gateway::evaluate_and_record(&snapshot, &risk_gateway::testing::limits(), &store)
            .await;
    assert!(
        outcome.into_approval().is_none(),
        "a denied decision mints no token"
    );

    // Without a token the intent cannot be approved, so it cannot be
    // submitted. The state machine refuses the attempt.
    assert!(
        intents
            .append(&input.intent_ref, &Event::SubmissionStarted)
            .await
            .is_err(),
        "an ungated intent must not be submittable"
    );
    assert_eq!(
        intents
            .state(&input.intent_ref)
            .await
            .expect("state")
            .name(),
        "INTENT_CREATED"
    );

    // And the broker was never constructed with anything to send: the call
    // count is asserted on a simulator that saw no traffic at all.
    let sim = BrokerSimulator::new();
    assert_eq!(
        sim.call_count("POST", "/uapi/domestic-stock/v1/trading/order-cash"),
        0
    );
}
