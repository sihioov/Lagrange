//! Todo 39, persistence half: intents, their event log, and claim-before-gate.
//!
//! Named `live_order_state_*` so the plan's acceptance filter reaches these
//! too. `kis-client`'s suite proves the machine; this proves that the database
//! agrees with it — that the log is gapless and append-only, that the cached
//! state matches a replay, and above all that a duplicate request is answered
//! from the existing intent instead of being sent back through the gate.

mod common;

use api_server::repos::order_intents::{Claim, NewOrderIntent, OrderIntentRepo, replay_state};
use common::{Harness, actor_pool};
use kis_client::order_state::{Event, OrderIntentState};
use uuid::Uuid;

/// A LIVE account for the owner. Intents are per-account, and 0019's FK means
/// an intent cannot exist without one.
async fn live_account(h: &Harness) -> Uuid {
    let pool = actor_pool(&h.app_url, &h.owner.user_id.to_string(), 2).await;
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO accounts (owner_user_id, account_type, name, currency) \
         VALUES ($1, 'LIVE', 'live-1', 'KRW') \
         ON CONFLICT (owner_user_id, name) DO UPDATE SET currency = EXCLUDED.currency \
         RETURNING id",
    )
    .bind(h.owner.user_id)
    .fetch_one(&pool)
    .await
    .expect("live account")
}

fn repo(h: &Harness) -> OrderIntentRepo {
    OrderIntentRepo::new(h.app_pool.clone(), h.owner.actor(), h.owner.user_id)
}

/// The harness already seeds the canonical KRX universe, so intents reference
/// `069500.KRX` rather than inventing an instrument.
async fn new_intent(_h: &Harness, account_id: Uuid) -> NewOrderIntent {
    NewOrderIntent {
        intent_ref: NewOrderIntent::mint_ref(),
        account_id,
        instrument_id: "069500.KRX".into(),
        side: "BUY".into(),
        quantity: "10".into(),
        price: Some("7250".into()),
        correlation_id: "corr-1".into(),
        client_key: format!("ck-{}", Uuid::new_v4()),
    }
}

#[tokio::test]
async fn live_order_state_a_minted_ref_is_globally_unique_and_server_generated() {
    // 0018's gate index is global, so a ref that could collide across accounts
    // would leave the second account unable to record a decision at all.
    let a = NewOrderIntent::mint_ref();
    let b = NewOrderIntent::mint_ref();
    assert_ne!(a, b);
    assert!(a.starts_with("oi_"));
    assert!(a.len() > 10, "a ref must not be guessable or sequential");
}

#[tokio::test]
async fn live_order_state_a_duplicate_claim_returns_the_existing_intent() {
    let Some(h) = Harness::new().await else {
        return;
    };
    let account = live_account(&h).await;
    let input = new_intent(&h, account).await;

    let first = repo(&h).claim(input.clone()).await.expect("first claim");
    assert!(
        first.is_new(),
        "the first claim owns the intent and proceeds to the gate"
    );

    // The retransmission, modelled the way it actually arrives: the SAME
    // client Idempotency-Key, but a fresh server-minted ref, because the
    // route mints one per request. Dedup keyed on the ref would miss this
    // entirely -- the client would get a second intent, a second legitimate
    // gate approval, and a SECOND REAL ORDER (FR-LIVE-003).
    let retransmission = NewOrderIntent {
        intent_ref: NewOrderIntent::mint_ref(),
        ..input.clone()
    };
    assert_ne!(retransmission.intent_ref, input.intent_ref);
    let second = repo(&h).claim(retransmission).await.expect("second claim");
    assert!(
        !second.is_new(),
        "a duplicate claim must not send the caller back through the gate"
    );
    assert!(matches!(second, Claim::Existing(_)));
    assert_eq!(
        second.row().intent_ref,
        input.intent_ref,
        "the retransmission must resolve to the ORIGINAL intent"
    );
    assert_eq!(second.row().state, "INTENT_CREATED");

    // Exactly one row exists.
    let pool = actor_pool(&h.app_url, &h.owner.user_id.to_string(), 2).await;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM order_intents WHERE client_key = $1")
        .bind(&input.client_key)
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(count, 1, "one client key yields exactly one intent, ever");
}

#[tokio::test]
async fn live_order_state_events_are_gapless_and_the_cached_state_matches_a_replay() {
    let Some(h) = Harness::new().await else {
        return;
    };
    let account = live_account(&h).await;
    let input = new_intent(&h, account).await;
    let r = repo(&h);
    r.claim(input.clone()).await.expect("claim");

    let log = [
        Event::RiskApproved,
        Event::SubmissionStarted,
        Event::SubmissionSent,
        Event::BrokerAccepted {
            broker_order_no: "B-100".into(),
        },
        Event::fill("B-100", 4, 10),
    ];
    for (i, event) in log.iter().enumerate() {
        let out = r.append(&input.intent_ref, event).await.expect("append");
        assert_eq!(out.seq as usize, i + 1, "sequence must be gapless");
        assert!(out.moved, "each of these events moves the state");
    }

    // The cached column and a replay of the log must agree. They are stored
    // redundantly ON PURPOSE, so that a divergence is detectable rather than
    // invisible; the log is the truth.
    let cached = r.state(&input.intent_ref).await.expect("cached state");
    let events: Vec<Event> = r
        .events(&input.intent_ref)
        .await
        .expect("events")
        .into_iter()
        .map(|(_, e)| e)
        .collect();
    let replayed = replay_state(&events).expect("log replays");
    assert_eq!(cached, replayed);
    assert_eq!(
        replayed,
        OrderIntentState::PartiallyFilled {
            broker_order_no: "B-100".into(),
            cumulative_filled: 4,
        }
    );
}

#[tokio::test]
async fn live_order_state_a_broker_resend_is_recorded_but_moves_nothing() {
    let Some(h) = Harness::new().await else {
        return;
    };
    let account = live_account(&h).await;
    let input = new_intent(&h, account).await;
    let r = repo(&h);
    r.claim(input.clone()).await.expect("claim");
    for event in [
        Event::RiskApproved,
        Event::SubmissionStarted,
        Event::SubmissionSent,
        Event::BrokerAccepted {
            broker_order_no: "B-1".into(),
        },
        Event::fill("B-1", 6, 10),
    ] {
        r.append(&input.intent_ref, &event).await.expect("append");
    }

    let before = r.state(&input.intent_ref).await.expect("state");
    // The same cumulative total again, as a reconnect would deliver it.
    let out = r
        .append(&input.intent_ref, &Event::fill("B-1", 6, 10))
        .await
        .expect("a re-send is recorded, not rejected");
    assert!(
        !out.moved,
        "a re-sent fill must not move the state, and so must not move the ledger"
    );
    assert_eq!(r.state(&input.intent_ref).await.expect("state"), before);

    // It IS in the log, because it genuinely happened and reconciliation
    // should be able to see that the broker said it twice.
    let events = r.events(&input.intent_ref).await.expect("events");
    assert_eq!(events.len(), 6);
}

#[tokio::test]
async fn live_order_state_an_illegal_transition_writes_nothing_at_all() {
    let Some(h) = Harness::new().await else {
        return;
    };
    let account = live_account(&h).await;
    let input = new_intent(&h, account).await;
    let r = repo(&h);
    r.claim(input.clone()).await.expect("claim");

    // A fill for an intent that has not even been approved is not history --
    // it is a bug or a hostile message, and it must leave no trace.
    let before = r.events(&input.intent_ref).await.expect("events").len();
    let err = r
        .append(&input.intent_ref, &Event::fill("B-1", 1, 10))
        .await
        .expect_err("an illegal transition must be refused");
    assert!(format!("{err}").contains("not legal"), "{err}");
    assert_eq!(
        r.events(&input.intent_ref).await.expect("events").len(),
        before,
        "a refused transition must append nothing"
    );
    assert_eq!(
        r.state(&input.intent_ref).await.expect("state").name(),
        "INTENT_CREATED"
    );
}

#[tokio::test]
async fn live_order_state_a_timeout_then_lookup_resolves_without_a_second_order() {
    // AT-09 end to end through the database.
    let Some(h) = Harness::new().await else {
        return;
    };
    let account = live_account(&h).await;
    let input = new_intent(&h, account).await;
    let r = repo(&h);
    r.claim(input.clone()).await.expect("claim");

    for event in [
        Event::RiskApproved,
        Event::SubmissionStarted,
        Event::SubmissionSent,
        Event::SubmissionTimedOut,
    ] {
        r.append(&input.intent_ref, &event).await.expect("append");
    }
    assert_eq!(
        r.state(&input.intent_ref).await.expect("state"),
        OrderIntentState::Unknown
    );

    // Resubmitting is not merely discouraged, it is refused by the machine.
    assert!(
        r.append(&input.intent_ref, &Event::SubmissionStarted)
            .await
            .is_err(),
        "an UNKNOWN intent must never be resubmitted"
    );

    // The lookup resolves it, and the intent binds to exactly one broker order.
    r.append(
        &input.intent_ref,
        &Event::BrokerLookupResolved {
            resolved: Box::new(OrderIntentState::Accepted {
                broker_order_no: "B-777".into(),
            }),
        },
    )
    .await
    .expect("lookup resolves");

    let state = r.state(&input.intent_ref).await.expect("state");
    assert_eq!(state.broker_order_no(), Some("B-777"));
}

#[tokio::test]
async fn live_order_state_two_intents_cannot_claim_one_broker_order() {
    // Reconciliation (Todo 40) reads this mapping; two intents pointing at one
    // broker order would make the account's true position unknowable.
    let Some(h) = Harness::new().await else {
        return;
    };
    let account = live_account(&h).await;
    let r = repo(&h);

    let first = new_intent(&h, account).await;
    r.claim(first.clone()).await.expect("claim");
    for event in [
        Event::RiskApproved,
        Event::SubmissionStarted,
        Event::SubmissionSent,
        Event::BrokerAccepted {
            broker_order_no: "B-SHARED".into(),
        },
    ] {
        r.append(&first.intent_ref, &event).await.expect("append");
    }

    let second = new_intent(&h, account).await;
    r.claim(second.clone()).await.expect("claim");
    for event in [
        Event::RiskApproved,
        Event::SubmissionStarted,
        Event::SubmissionSent,
    ] {
        r.append(&second.intent_ref, &event).await.expect("append");
    }
    let err = r
        .append(
            &second.intent_ref,
            &Event::BrokerAccepted {
                broker_order_no: "B-SHARED".into(),
            },
        )
        .await
        .expect_err("one broker order belongs to one intent");
    let _ = err;

    // The refusal must have left the second intent unbound rather than
    // half-applied: an intent claiming a broker order it does not own would
    // make the account's true position unknowable at reconciliation.
    let second_row = r.get(&second.intent_ref).await.expect("second intent");
    assert_eq!(second_row.broker_order_no, None);
    assert_eq!(second_row.state, "SUBMITTED");

    // And the first intent still owns it.
    let first_row = r.get(&first.intent_ref).await.expect("first intent");
    assert_eq!(first_row.broker_order_no.as_deref(), Some("B-SHARED"));
}

#[tokio::test]
async fn live_order_state_the_event_log_cannot_be_edited_or_deleted() {
    let Some(h) = Harness::new().await else {
        return;
    };
    let account = live_account(&h).await;
    let input = new_intent(&h, account).await;
    let r = repo(&h);
    r.claim(input.clone()).await.expect("claim");
    r.append(&input.intent_ref, &Event::RiskApproved)
        .await
        .expect("append");

    // History does not change. `order_intents` itself is mutable -- its state
    // legitimately moves -- but the events that produced it are fixed.
    let pool = actor_pool(&h.app_url, &h.owner.user_id.to_string(), 2).await;
    for statement in [
        "UPDATE order_intent_events SET resulting_state = 'FILLED'",
        "DELETE FROM order_intent_events",
    ] {
        let err = sqlx::query(sqlx::AssertSqlSafe(statement.to_string()))
            .execute(&pool)
            .await
            .expect_err("the event log must be append-only");
        let code = match &err {
            sqlx::Error::Database(e) => e.code().map(|c| c.into_owned()),
            _ => None,
        };
        assert_eq!(code.as_deref(), Some("42501"), "{statement}");
    }
}

#[tokio::test]
async fn live_order_state_unresolved_lists_exactly_what_needs_attention() {
    let Some(h) = Harness::new().await else {
        return;
    };
    let account = live_account(&h).await;
    let r = repo(&h);

    // One that finished, and one stuck in UNKNOWN.
    let done = new_intent(&h, account).await;
    r.claim(done.clone()).await.expect("claim");
    for event in [
        Event::RiskApproved,
        Event::SubmissionStarted,
        Event::SubmissionSent,
        Event::BrokerRejected {
            reason: "refused".into(),
        },
    ] {
        r.append(&done.intent_ref, &event).await.expect("append");
    }

    let stuck = new_intent(&h, account).await;
    r.claim(stuck.clone()).await.expect("claim");
    for event in [
        Event::RiskApproved,
        Event::SubmissionStarted,
        Event::SubmissionTimedOut,
    ] {
        r.append(&stuck.intent_ref, &event).await.expect("append");
    }

    let unresolved = r.unresolved().await.expect("unresolved");
    let refs: Vec<&str> = unresolved.iter().map(|r| r.intent_ref.as_str()).collect();
    assert!(refs.contains(&stuck.intent_ref.as_str()));
    assert!(
        !refs.contains(&done.intent_ref.as_str()),
        "a terminated intent needs no attention"
    );
}

#[tokio::test]
async fn live_order_state_a_different_client_key_is_a_different_intent() {
    // The dedup must not be so eager that two genuinely distinct orders
    // collapse into one. Same account, same instrument, same size -- a client
    // legitimately placing the same order twice -- but different keys.
    let Some(h) = Harness::new().await else {
        return;
    };
    let account = live_account(&h).await;
    let r = repo(&h);

    let first = new_intent(&h, account).await;
    let second = NewOrderIntent {
        intent_ref: NewOrderIntent::mint_ref(),
        client_key: format!("ck-{}", Uuid::new_v4()),
        ..first.clone()
    };

    let a = r.claim(first).await.expect("first");
    let b = r.claim(second).await.expect("second");
    assert!(a.is_new());
    assert!(b.is_new(), "a distinct client key is a distinct intent");
    assert_ne!(a.row().intent_ref, b.row().intent_ref);
}

#[tokio::test]
async fn live_order_state_an_approval_for_another_intent_is_refused() {
    // `record_approval` consumes the RiskApproval token, so an intent cannot
    // be approved without a gate run that both approved AND persisted. This
    // asserts the other half: the token must name THIS intent, or it would
    // authorise an order the gate never assessed.
    let Some(h) = Harness::new().await else {
        return;
    };
    let account = live_account(&h).await;
    let r = repo(&h);

    let target = new_intent(&h, account).await;
    r.claim(target.clone()).await.expect("claim");

    // Publish limits and run the gate for a DIFFERENT intent, then try to
    // spend its approval here.
    sqlx::query(
        "INSERT INTO risk_limits (version, max_symbol_weight_bp, max_order_value, \
         max_daily_order_value, max_daily_loss, max_data_age_secs) \
         VALUES ('risk-limits-v1', 3000, 1000000, 5000000, 500000, 300) \
         ON CONFLICT (version) DO NOTHING",
    )
    .execute(&h.owner_pool)
    .await
    .expect("limits");

    let mut snapshot = risk_gateway::testing::snapshot_all_green();
    snapshot.intent.intent_ref = "some-other-intent".into();
    let store = api_server::repos::risk::RiskRepo::new(
        h.app_pool.clone(),
        h.owner.actor(),
        h.owner.user_id,
        None,
    );
    let approval =
        risk_gateway::evaluate_and_record(&snapshot, &risk_gateway::testing::limits(), &store)
            .await
            .into_approval()
            .expect("approved");

    let err = r
        .record_approval(&target.intent_ref, approval)
        .await
        .expect_err("an approval for another intent must be refused");
    assert!(format!("{err}").contains("some-other-intent"), "{err}");

    // The target intent is untouched: still ungated.
    assert_eq!(
        r.state(&target.intent_ref).await.expect("state").name(),
        "INTENT_CREATED"
    );
}
