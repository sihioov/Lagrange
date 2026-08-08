//! Todo 39 acceptance: the Live order state machine.
//!
//! Every test is named `live_order_state_*` because the plan's acceptance
//! command is `cargo test -p kis-client -p portfolio-model live_order_state`,
//! and cargo's positional argument is a test-NAME filter, not a target
//! filter. Todo 33 shipped a gate whose filter silently selected zero tests;
//! the naming here is what stops that recurring.
//!
//! The property the whole file defends: **a timeout is not a rejection**. A
//! rejection proves no order exists; a timeout proves nothing. Every path that
//! could turn the second into a resubmission is closed by an absent
//! transition rather than by a check that someone could forget.

use kis_client::idempotency::IntentState;
use kis_client::order_state::{Applied, Event, OrderIntentState, TransitionError, replay};

/// The states, for exhaustiveness sweeps.
fn every_state() -> Vec<OrderIntentState> {
    vec![
        OrderIntentState::IntentCreated,
        OrderIntentState::RiskApproved,
        OrderIntentState::Submitting,
        OrderIntentState::Submitted,
        OrderIntentState::Accepted {
            broker_order_no: "B1".into(),
        },
        OrderIntentState::Rejected {
            reason: "no".into(),
        },
        OrderIntentState::Unknown,
        OrderIntentState::PartiallyFilled {
            broker_order_no: "B1".into(),
            cumulative_filled: 5,
        },
        OrderIntentState::Filled {
            broker_order_no: "B1".into(),
        },
        OrderIntentState::Canceled {
            broker_order_no: "B1".into(),
        },
        OrderIntentState::Expired {
            broker_order_no: "B1".into(),
        },
        OrderIntentState::Denied {
            reason: "gate".into(),
        },
    ]
}

fn every_event() -> Vec<Event> {
    vec![
        Event::RiskApproved,
        Event::RiskDenied {
            reason: "kill switch".into(),
        },
        Event::SubmissionStarted,
        Event::SubmissionSent,
        Event::BrokerAccepted {
            broker_order_no: "B1".into(),
        },
        Event::BrokerRejected {
            reason: "insufficient".into(),
        },
        Event::SubmissionTimedOut,
        Event::fill("B1", 5, 10),
        Event::BrokerCanceled {
            broker_order_no: "B1".into(),
        },
        Event::BrokerExpired {
            broker_order_no: "B1".into(),
        },
        Event::BrokerLookupResolved {
            resolved: Box::new(OrderIntentState::Accepted {
                broker_order_no: "B1".into(),
            }),
        },
    ]
}

fn moved(result: Result<Applied, TransitionError>) -> OrderIntentState {
    match result.expect("transition is legal") {
        Applied::Moved(s) => s,
        Applied::NoChange => panic!("expected a move, got NoChange"),
    }
}

#[test]
fn live_order_state_the_documented_happy_path_runs_end_to_end() {
    let s = OrderIntentState::IntentCreated;
    let s = moved(s.apply(&Event::RiskApproved));
    assert_eq!(s, OrderIntentState::RiskApproved);
    // Only here may a submission go out.
    assert!(s.may_submit());

    let s = moved(s.apply(&Event::SubmissionStarted));
    assert!(
        !s.may_submit(),
        "an in-flight intent must not be resubmitted"
    );
    let s = moved(s.apply(&Event::SubmissionSent));
    let s = moved(s.apply(&Event::BrokerAccepted {
        broker_order_no: "B-77".into(),
    }));
    assert_eq!(s.broker_order_no(), Some("B-77"));

    let s = moved(s.apply(&Event::fill("B-77", 4, 10)));
    assert_eq!(s.name(), "PARTIALLY_FILLED");
    let s = moved(s.apply(&Event::fill("B-77", 10, 10)));
    assert_eq!(s.name(), "FILLED");
    assert!(s.is_terminal());
}

#[test]
fn live_order_state_a_timeout_becomes_unknown_and_never_resubmits() {
    // AT-09 and §16. This is the test the module exists for.
    for start in [OrderIntentState::Submitting, OrderIntentState::Submitted] {
        let unknown = moved(start.apply(&Event::SubmissionTimedOut));
        assert_eq!(unknown, OrderIntentState::Unknown);
        assert!(
            !unknown.may_submit(),
            "UNKNOWN must never permit a resubmission"
        );

        // There is no event that takes UNKNOWN back into the submission path.
        // Not a check that could be bypassed — the transitions do not exist.
        for event in [
            Event::RiskApproved,
            Event::SubmissionStarted,
            Event::SubmissionSent,
        ] {
            assert!(
                unknown.apply(&event).is_err(),
                "UNKNOWN must not accept {}",
                event.name()
            );
        }
    }
}

#[test]
fn live_order_state_unknown_resolves_only_to_a_concrete_broker_outcome() {
    let unknown = OrderIntentState::Unknown;

    // A lookup can resolve it to any real broker outcome...
    for resolved in [
        OrderIntentState::Accepted {
            broker_order_no: "B1".into(),
        },
        OrderIntentState::Rejected {
            reason: "refused".into(),
        },
        OrderIntentState::Filled {
            broker_order_no: "B1".into(),
        },
        OrderIntentState::Canceled {
            broker_order_no: "B1".into(),
        },
    ] {
        let out = moved(unknown.apply(&Event::BrokerLookupResolved {
            resolved: Box::new(resolved.clone()),
        }));
        assert_eq!(out, resolved);
    }

    // ...but never back into the submission path, which would be the
    // resubmission this module prevents wearing a different hat.
    for illegal in [
        OrderIntentState::RiskApproved,
        OrderIntentState::Submitting,
        OrderIntentState::Submitted,
        OrderIntentState::IntentCreated,
    ] {
        let err = unknown
            .apply(&Event::BrokerLookupResolved {
                resolved: Box::new(illegal.clone()),
            })
            .expect_err("must refuse");
        assert!(
            matches!(err, TransitionError::IllegalResolution { .. }),
            "{illegal} resolution should be refused, got {err}"
        );
    }
}

#[test]
fn live_order_state_a_late_broker_answer_resolves_unknown_by_itself() {
    // A WebSocket reconnect can deliver the ack for the order we timed out
    // on. That IS the answer; requiring a separate lookup would leave the
    // intent stuck.
    let unknown = OrderIntentState::Unknown;
    let accepted = moved(unknown.apply(&Event::BrokerAccepted {
        broker_order_no: "B-9".into(),
    }));
    assert_eq!(accepted.broker_order_no(), Some("B-9"));

    // A fill arriving while UNKNOWN likewise proves the order exists.
    let filled = moved(OrderIntentState::Unknown.apply(&Event::fill("B-9", 10, 10)));
    assert_eq!(filled.name(), "FILLED");
}

#[test]
fn live_order_state_one_intent_produces_at_most_one_order() {
    // The duplicate-order failure, expressed as a property: from every state,
    // no sequence of events can pass through SUBMITTING twice.
    let sequences: Vec<Vec<Event>> = vec![
        vec![
            Event::RiskApproved,
            Event::SubmissionStarted,
            Event::SubmissionSent,
            Event::SubmissionTimedOut,
        ],
        vec![
            Event::RiskApproved,
            Event::SubmissionStarted,
            Event::SubmissionTimedOut,
        ],
        vec![
            Event::RiskApproved,
            Event::SubmissionStarted,
            Event::SubmissionSent,
            Event::BrokerRejected {
                reason: "refused".into(),
            },
        ],
    ];

    for seq in sequences {
        let end = replay(OrderIntentState::IntentCreated, &seq).expect("legal sequence");
        // Whatever happened, a second submission is not available.
        assert!(
            !end.may_submit(),
            "{end} would permit a second submission for one intent"
        );
        assert!(
            end.apply(&Event::SubmissionStarted).is_err(),
            "{end} accepted a second SUBMISSION_STARTED"
        );
    }
}

#[test]
fn live_order_state_a_rejection_is_terminal_so_a_retry_is_a_new_intent() {
    // Under one-decision-per-intent (migration 0018) a resubmission of this
    // ref could never be authorised again, so the machine makes REJECTED
    // terminal rather than leaving a path that could not be walked.
    let rejected = OrderIntentState::Rejected {
        reason: "insufficient balance".into(),
    };
    assert!(rejected.is_terminal());
    assert!(!rejected.may_submit());
    assert!(matches!(
        rejected.apply(&Event::RiskApproved),
        Err(TransitionError::Terminal { .. })
    ));

    // The in-memory transport guard says a rejected key MAY be resubmitted.
    // That arm is unreachable through the Live path, and the two authorities
    // are reconciled here rather than left to disagree silently.
    let guard = rejected
        .as_broker_intent_state()
        .expect("has a guard state");
    assert!(guard.allows_submission());
    assert!(
        !rejected.may_submit(),
        "the durable machine, not the in-memory guard, is the authority"
    );
}

#[test]
fn live_order_state_duplicate_and_stale_fills_move_nothing() {
    // Brokers re-send fill reports after a reconnect. Counting them twice
    // would double the position; erroring on them would turn a normal
    // reconnect into an incident.
    let accepted = OrderIntentState::Accepted {
        broker_order_no: "B1".into(),
    };
    let partial = moved(accepted.apply(&Event::fill("B1", 6, 10)));
    assert_eq!(partial.name(), "PARTIALLY_FILLED");

    // The same cumulative total again: valid, and NOTHING happens. This is
    // the assertion the exhaustive sweep corrected -- it originally expected
    // a "move" to the identical state, which would have told a caller to
    // apply the fill to the ledger a second time.
    assert_eq!(
        partial.apply(&Event::fill("B1", 6, 10)),
        Ok(Applied::NoChange),
        "a re-sent partial must not move the ledger"
    );
    // A report going BACKWARDS (an out-of-order older report) likewise.
    assert_eq!(
        partial.apply(&Event::fill("B1", 3, 10)),
        Ok(Applied::NoChange),
        "a stale partial must not move the ledger"
    );

    // A fully-filled order re-sending its final report is a no-op, not an
    // error, and must not move the ledger.
    let filled = moved(partial.apply(&Event::fill("B1", 10, 10)));
    assert_eq!(
        filled.apply(&Event::fill("B1", 10, 10)),
        Ok(Applied::NoChange)
    );
}

#[test]
fn live_order_state_fills_are_cumulative_so_order_of_arrival_does_not_matter() {
    // Reports keyed by the broker's running total are order-insensitive by
    // construction; an incremental model would double-count a re-send and
    // under-count a lost one.
    let start = OrderIntentState::Accepted {
        broker_order_no: "B1".into(),
    };
    let in_order = replay(
        start.clone(),
        &[
            Event::fill("B1", 3, 10),
            Event::fill("B1", 7, 10),
            Event::fill("B1", 10, 10),
        ],
    )
    .expect("legal");
    let with_resend = replay(
        start.clone(),
        &[
            Event::fill("B1", 3, 10),
            Event::fill("B1", 3, 10),
            Event::fill("B1", 7, 10),
            Event::fill("B1", 10, 10),
            Event::fill("B1", 10, 10),
        ],
    )
    .expect("legal");
    assert_eq!(in_order, with_resend);
    assert_eq!(in_order.name(), "FILLED");
}

#[test]
fn live_order_state_an_event_for_another_broker_order_is_refused() {
    // Applying someone else's fill to this intent would corrupt both.
    let accepted = OrderIntentState::Accepted {
        broker_order_no: "B1".into(),
    };
    let err = accepted
        .apply(&Event::fill("B2", 5, 10))
        .expect_err("must refuse");
    assert!(matches!(err, TransitionError::BrokerOrderMismatch { .. }));
    assert!(
        accepted
            .apply(&Event::BrokerCanceled {
                broker_order_no: "B2".into()
            })
            .is_err()
    );
}

#[test]
fn live_order_state_a_fill_beyond_the_order_quantity_is_refused() {
    let accepted = OrderIntentState::Accepted {
        broker_order_no: "B1".into(),
    };
    assert!(matches!(
        accepted.apply(&Event::fill("B1", 11, 10)),
        Err(TransitionError::OverFill {
            cumulative: 11,
            total: 10
        })
    ));
}

#[test]
fn live_order_state_a_terminal_order_cannot_be_revived_by_a_late_event() {
    // A cancel confirmation arriving after a fill, or a fill arriving after a
    // cancel, must not resurrect the order.
    let canceled = OrderIntentState::Canceled {
        broker_order_no: "B1".into(),
    };
    assert!(matches!(
        canceled.apply(&Event::fill("B1", 5, 10)),
        Err(TransitionError::Terminal { .. })
    ));

    // But the terminal fact being RE-SENT is a no-op, since brokers re-send.
    assert_eq!(
        canceled.apply(&Event::BrokerCanceled {
            broker_order_no: "B1".into()
        }),
        Ok(Applied::NoChange)
    );
}

#[test]
fn live_order_state_a_gate_denial_is_distinguishable_from_never_gated() {
    // Both would otherwise sit in INTENT_CREATED, and only one of them may be
    // gated again.
    let created = OrderIntentState::IntentCreated;
    let denied = moved(created.apply(&Event::RiskDenied {
        reason: "LIVE_KILL_SWITCH_ENGAGED".into(),
    }));
    assert_eq!(denied.name(), "DENIED");
    assert!(denied.is_terminal());
    assert_ne!(denied, OrderIntentState::IntentCreated);
    assert!(created.apply(&Event::RiskApproved).is_ok());
    assert!(denied.apply(&Event::RiskApproved).is_err());
}

#[test]
fn live_order_state_every_state_event_pair_is_decided_and_never_silently_ignored() {
    // Exhaustive sweep: 12 states x 11 events. Every pair must be a move, an
    // explicit NoChange, or a named error. A `_ => Ok(self.clone())` fallthrough
    // would pass every other test in this file and fail here, because an event
    // that is silently dropped is indistinguishable from one that was handled.
    let mut pairs = 0;
    for state in every_state() {
        for event in every_event() {
            match state.apply(&event) {
                Ok(Applied::Moved(next)) => {
                    assert_ne!(
                        next,
                        state,
                        "{state} + {} reported a move to the same state",
                        event.name()
                    );
                }
                Ok(Applied::NoChange) | Err(_) => {}
            }
            pairs += 1;
        }
    }
    assert_eq!(pairs, 12 * 11);
}

#[test]
fn live_order_state_replaying_the_log_reproduces_the_state_after_a_restart() {
    // A restart mid-submit has nothing but the durable event log. The machine
    // is pure, so the log is sufficient.
    let log = vec![
        Event::RiskApproved,
        Event::SubmissionStarted,
        Event::SubmissionSent,
        Event::SubmissionTimedOut,
        Event::BrokerLookupResolved {
            resolved: Box::new(OrderIntentState::Accepted {
                broker_order_no: "B-42".into(),
            }),
        },
        Event::fill("B-42", 5, 10),
    ];
    let end = replay(OrderIntentState::IntentCreated, &log).expect("legal log");
    assert_eq!(end.name(), "PARTIALLY_FILLED");

    // Replaying a prefix then the remainder gives the same answer, which is
    // what a crash halfway through actually looks like.
    let mid = replay(OrderIntentState::IntentCreated, &log[..4]).expect("legal prefix");
    assert_eq!(mid, OrderIntentState::Unknown);
    let rest = replay(mid, &log[4..]).expect("legal suffix");
    assert_eq!(rest, end);
}

#[test]
fn live_order_state_serialises_to_stable_names_for_the_database() {
    // These strings are persisted in `order_intents.state`, so a rename would
    // silently orphan every existing row.
    let json = serde_json::to_string(&OrderIntentState::Unknown).expect("serialises");
    assert_eq!(json, r#"{"state":"UNKNOWN"}"#);
    let json = serde_json::to_string(&OrderIntentState::PartiallyFilled {
        broker_order_no: "B1".into(),
        cumulative_filled: 4,
    })
    .expect("serialises");
    assert_eq!(
        json,
        r#"{"state":"PARTIALLY_FILLED","broker_order_no":"B1","cumulative_filled":4}"#
    );
    for state in every_state() {
        let round: OrderIntentState =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        assert_eq!(round, state);
    }
}

#[test]
fn live_order_state_maps_onto_the_transport_guard_without_contradiction() {
    // The in-memory guard (Todo 36) and this machine must not disagree about
    // whether a submission may go out.
    assert_eq!(
        OrderIntentState::Submitting.as_broker_intent_state(),
        Some(IntentState::Submitting)
    );
    assert_eq!(
        OrderIntentState::Unknown.as_broker_intent_state(),
        Some(IntentState::Unknown)
    );
    assert_eq!(
        OrderIntentState::Filled {
            broker_order_no: "B1".into()
        }
        .as_broker_intent_state(),
        Some(IntentState::Acknowledged {
            broker_order_no: "B1".into()
        })
    );
    // Pre-submission states have no transport-level meaning yet.
    assert_eq!(
        OrderIntentState::IntentCreated.as_broker_intent_state(),
        None
    );
    assert_eq!(
        OrderIntentState::RiskApproved.as_broker_intent_state(),
        None
    );
}
