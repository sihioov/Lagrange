//! Todo 40 acceptance: startup and runtime reconciliation.
//!
//! Every test is named `reconciliation_*` because the plan's acceptance
//! command is `cargo test -p kis-client reconciliation` — a test-NAME filter.
//! Naming them so from the first one written is the standing fix for the
//! Todo 33 trap, where a filter selected zero tests and the gate read green.
//!
//! The rule the file defends: **the broker is the truth about the broker.**
//! An unexplained difference is a difference in a real account with real
//! money, and adopting our own number would hide it. Everything blocks except
//! a fill we simply had not applied yet.

use kis_client::order_state::OrderIntentState;
use kis_client::reconciliation::{
    BrokerAccountSnapshot, BrokerFill, BrokerOpenOrder, LocalAccountSnapshot, LocalIntent,
    Mismatch, PositionSnapshot, reconcile,
};

const NOW: i64 = 1_800_000_000;
const MAX_AGE: i64 = 300;

fn pos(id: &str, q: i64) -> PositionSnapshot {
    PositionSnapshot {
        instrument_id: id.into(),
        quantity: q,
    }
}

/// Two views that agree exactly.
fn agreeing() -> (LocalAccountSnapshot, BrokerAccountSnapshot) {
    let local = LocalAccountSnapshot {
        cash: "1000000.0000".into(),
        positions: vec![pos("069500.KRX", 10)],
        working_intents: vec![],
        known_execution_ids: vec!["E1".into()],
    };
    let broker = BrokerAccountSnapshot {
        account_no_masked: "****6-01".into(),
        // Deliberately a different rendering of the same amount: PostgreSQL
        // writes numeric(18,4) with its scale, a broker may not.
        cash: "1000000".into(),
        positions: vec![pos("069500.KRX", 10)],
        open_orders: vec![],
        day_fills: vec![BrokerFill {
            execution_id: "E1".into(),
            broker_order_no: "B1".into(),
            quantity: 10,
        }],
        as_of_secs: NOW - 30,
    };
    (local, broker)
}

#[test]
fn reconciliation_agreeing_views_are_green() {
    let (local, broker) = agreeing();
    let out = reconcile(&local, &broker, NOW, MAX_AGE);
    assert!(out.is_green(), "{:?}", out.mismatches);
    assert!(!out.requires_owner());
    assert!(out.lookups_required.is_empty());
}

#[test]
fn reconciliation_cash_formatting_alone_is_never_a_mismatch() {
    // If "1000" and "1000.0000" reported a difference, EVERY reconciliation
    // would raise a cash mismatch and the resulting noise would bury a real
    // one.
    let (mut local, mut broker) = agreeing();
    local.cash = "0".into();
    broker.cash = "0.0000".into();
    assert!(reconcile(&local, &broker, NOW, MAX_AGE).is_green());

    // ...but a real difference of one ten-thousandth still blocks.
    broker.cash = "0.0001".into();
    let out = reconcile(&local, &broker, NOW, MAX_AGE);
    assert!(!out.is_green());
    assert!(matches!(out.mismatches[0], Mismatch::Cash { .. }));
}

#[test]
fn reconciliation_a_position_the_broker_holds_and_we_do_not_is_found() {
    // The case iterating only our OWN positions would miss entirely, and the
    // most dangerous one: a real holding nobody is accounting for.
    let (local, mut broker) = agreeing();
    broker.positions.push(pos("229200.KRX", 5));

    let out = reconcile(&local, &broker, NOW, MAX_AGE);
    assert!(!out.is_green());
    assert!(out.mismatches.iter().any(|m| matches!(
        m,
        Mismatch::Position { instrument_id, ours: 0, brokers: 5 } if instrument_id == "229200.KRX"
    )));
    assert!(
        out.requires_owner(),
        "an unexplained position needs an Owner"
    );
}

#[test]
fn reconciliation_a_position_we_hold_and_the_broker_does_not_is_found() {
    let (mut local, broker) = agreeing();
    local.positions.push(pos("114260.KRX", 7));

    let out = reconcile(&local, &broker, NOW, MAX_AGE);
    assert!(out.mismatches.iter().any(|m| matches!(
        m,
        Mismatch::Position { instrument_id, ours: 7, brokers: 0 } if instrument_id == "114260.KRX"
    )));
}

#[test]
fn reconciliation_an_unmapped_broker_order_blocks() {
    // A real order at the broker that no intent of ours manages. Nothing may
    // trade until someone explains it.
    let (local, mut broker) = agreeing();
    broker.open_orders.push(BrokerOpenOrder {
        broker_order_no: "B-ORPHAN".into(),
        instrument_id: "069500.KRX".into(),
        side: "BUY".into(),
        quantity: 10,
        filled_quantity: 0,
    });

    let out = reconcile(&local, &broker, NOW, MAX_AGE);
    assert!(!out.is_green());
    let m = out
        .mismatches
        .iter()
        .find(|m| matches!(m, Mismatch::UnmappedBrokerOrder { .. }))
        .expect("the orphan must be reported");
    assert!(
        !m.is_auto_resolvable(),
        "an orphan order is never auto-resolved"
    );
    assert!(out.requires_owner());
}

#[test]
fn reconciliation_an_order_we_think_is_working_but_the_broker_lost_blocks() {
    let (mut local, broker) = agreeing();
    local.working_intents.push(LocalIntent {
        intent_ref: "oi-1".into(),
        state: OrderIntentState::Accepted {
            broker_order_no: "B-GONE".into(),
        },
    });

    let out = reconcile(&local, &broker, NOW, MAX_AGE);
    assert!(out.mismatches.iter().any(|m| matches!(
        m,
        Mismatch::UnknownToBroker { intent_ref, .. } if intent_ref == "oi-1"
    )));
}

#[test]
fn reconciliation_an_unknown_intent_demands_a_lookup_and_blocks() {
    // AT-09's residue at startup: we do not know whether an order exists, and
    // no amount of comparing positions can tell us. Only a lookup can.
    let (mut local, broker) = agreeing();
    local.working_intents.push(LocalIntent {
        intent_ref: "oi-unknown".into(),
        state: OrderIntentState::Unknown,
    });

    let out = reconcile(&local, &broker, NOW, MAX_AGE);
    assert!(!out.is_green());
    assert_eq!(out.lookups_required, vec!["oi-unknown".to_string()]);
    assert!(out.requires_owner());
}

#[test]
fn reconciliation_a_missing_fill_is_the_only_thing_a_pass_may_resolve_itself() {
    // A fill we have not applied is not a disagreement about the world, it is
    // a message we missed -- and the ledger's fill_id handling makes applying
    // it idempotent. Everything else is a genuine difference in belief about
    // a real account.
    let (local, mut broker) = agreeing();
    broker.day_fills.push(BrokerFill {
        execution_id: "E-NEW".into(),
        broker_order_no: "B1".into(),
        quantity: 3,
    });

    let out = reconcile(&local, &broker, NOW, MAX_AGE);
    assert!(!out.is_green(), "an unapplied fill is still a mismatch");
    assert_eq!(out.fills_to_apply.len(), 1);
    assert_eq!(out.fills_to_apply[0].execution_id, "E-NEW");
    assert!(
        !out.requires_owner(),
        "a missed fill is resolvable without an Owner"
    );
    assert!(out.blocking().is_empty());

    // And every OTHER mismatch kind refuses to be auto-resolved.
    for m in [
        Mismatch::Position {
            instrument_id: "X".into(),
            ours: 1,
            brokers: 2,
        },
        Mismatch::Cash {
            ours: "1".into(),
            brokers: "2".into(),
        },
        Mismatch::UnmappedBrokerOrder {
            broker_order_no: "B".into(),
            instrument_id: "X".into(),
        },
        Mismatch::UnknownToBroker {
            intent_ref: "i".into(),
            broker_order_no: "B".into(),
        },
        Mismatch::UnresolvedIntent {
            intent_ref: "i".into(),
        },
        Mismatch::StaleSnapshot {
            age_secs: 1,
            max_age_secs: 0,
        },
    ] {
        assert!(
            !m.is_auto_resolvable(),
            "{} must never be auto-resolved",
            m.kind()
        );
    }
}

#[test]
fn reconciliation_is_not_green_until_the_work_is_actually_done() {
    // "Green" must not mean "green after some work nobody has done yet". The
    // Risk Gateway lets orders through on the strength of this, so a pass
    // that merely KNOWS how to fix itself is still not green.
    let (local, mut broker) = agreeing();
    broker.day_fills.push(BrokerFill {
        execution_id: "E-NEW".into(),
        broker_order_no: "B1".into(),
        quantity: 3,
    });
    let first = reconcile(&local, &broker, NOW, MAX_AGE);
    assert!(!first.is_green());

    // Only once the fill has actually been applied -- our known set now
    // contains it -- does a later pass go green.
    let mut applied = local.clone();
    applied.known_execution_ids.push("E-NEW".into());
    assert!(reconcile(&applied, &broker, NOW, MAX_AGE).is_green());
}

#[test]
fn reconciliation_a_stale_snapshot_blocks_however_well_it_agrees() {
    // A snapshot too old to trust is not evidence of agreement. Everything
    // else here matches exactly, and it still must not go green.
    let (local, broker) = agreeing();
    let mut old = broker.clone();
    old.as_of_secs = NOW - (MAX_AGE + 1);

    let out = reconcile(&local, &old, NOW, MAX_AGE);
    assert!(!out.is_green());
    assert!(matches!(out.mismatches[0], Mismatch::StaleSnapshot { .. }));

    // Exactly at the limit is still fresh, matching the gate's freshness rule.
    let mut edge = broker.clone();
    edge.as_of_secs = NOW - MAX_AGE;
    assert!(reconcile(&local, &edge, NOW, MAX_AGE).is_green());

    // A snapshot from the FUTURE is a clock fault, not very fresh data.
    let mut future = broker.clone();
    future.as_of_secs = NOW + 10;
    assert!(!reconcile(&local, &future, NOW, MAX_AGE).is_green());
}

#[test]
fn reconciliation_is_pure_so_a_stored_pair_replays_to_the_same_verdict() {
    // Restored systems re-run reconciliation from persisted snapshots; if the
    // verdict could drift, "blocked until green" would be unstable.
    let (local, mut broker) = agreeing();
    broker.positions.push(pos("229200.KRX", 5));

    let a = reconcile(&local, &broker, NOW, MAX_AGE);
    let local_json = serde_json::to_string(&local).expect("serialises");
    let broker_json = serde_json::to_string(&broker).expect("serialises");
    let local2: LocalAccountSnapshot = serde_json::from_str(&local_json).expect("round trips");
    let broker2: BrokerAccountSnapshot = serde_json::from_str(&broker_json).expect("round trips");
    let b = reconcile(&local2, &broker2, NOW, MAX_AGE);

    assert_eq!(a, b);
}

#[test]
fn reconciliation_reports_every_difference_not_merely_the_first() {
    // An operator fixing one mismatch at a time, discovering the next only
    // after another restart, is a much worse outage than one report listing
    // all of them.
    let (mut local, mut broker) = agreeing();
    broker.positions.push(pos("229200.KRX", 5));
    broker.cash = "999999".into();
    broker.open_orders.push(BrokerOpenOrder {
        broker_order_no: "B-ORPHAN".into(),
        instrument_id: "069500.KRX".into(),
        side: "BUY".into(),
        quantity: 1,
        filled_quantity: 0,
    });
    local.working_intents.push(LocalIntent {
        intent_ref: "oi-unknown".into(),
        state: OrderIntentState::Unknown,
    });

    let out = reconcile(&local, &broker, NOW, MAX_AGE);
    let kinds: Vec<&str> = out.mismatches.iter().map(Mismatch::kind).collect();
    for expected in [
        "POSITION",
        "CASH",
        "UNMAPPED_BROKER_ORDER",
        "UNRESOLVED_INTENT",
    ] {
        assert!(
            kinds.contains(&expected),
            "{expected} missing from {kinds:?}"
        );
    }
    assert!(out.requires_owner());
}

#[test]
fn reconciliation_a_terminal_intent_is_not_expected_at_the_broker() {
    // A filled or canceled order is gone from the broker's working list by
    // definition; reporting that as "the broker lost our order" would make
    // every completed order a mismatch.
    let (mut local, broker) = agreeing();
    local.working_intents.push(LocalIntent {
        intent_ref: "oi-done".into(),
        state: OrderIntentState::Filled {
            broker_order_no: "B-DONE".into(),
        },
    });

    let out = reconcile(&local, &broker, NOW, MAX_AGE);
    assert!(
        !out.mismatches
            .iter()
            .any(|m| matches!(m, Mismatch::UnknownToBroker { .. })),
        "a completed order must not be reported as lost: {:?}",
        out.mismatches
    );
    assert!(out.is_green(), "{:?}", out.mismatches);
}
