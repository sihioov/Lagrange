//! Todo 39 acceptance, ledger half: partial fills update the ledger exactly
//! once.
//!
//! Named `live_order_state_*` so the plan's acceptance command
//! `cargo test -p kis-client -p portfolio-model live_order_state` selects
//! tests in BOTH crates. Cargo's positional argument is a test-NAME filter, so
//! a correctly-implemented feature whose tests are named otherwise reports
//! "0 passed" and the gate reads as green — the Todo 33 trap.
//!
//! `kis-client::order_state` decides WHETHER a fill report is new (its
//! `Applied::NoChange` means "the broker re-sent; do not move the ledger").
//! This file covers the other half of that contract: when a report IS new, the
//! ledger applies it once, and when the same fill arrives twice, the ledger
//! refuses it on its own account rather than trusting the caller to have
//! filtered it.
//!
//! Two independent defences, deliberately. A broker re-send that slipped past
//! the state machine would still be caught here by `fill_id`, and a caller
//! that ignored the state machine's `NoChange` would still not double-count.

use std::collections::BTreeMap;
use std::str::FromStr;

use domain::{
    Currency, FillId, FixedPoint, InstrumentId, Money, OrderId, Price, Quantity, TradingDate,
};

use portfolio_model::cost::CostProfile;
use portfolio_model::error::PortfolioError;
use portfolio_model::ledger::{LedgerEvent, LedgerState};
use portfolio_model::side::Side;

const A: &str = "069500.KRX";

fn instrument(s: &str) -> InstrumentId {
    InstrumentId::parse(s).expect("valid instrument id")
}

fn krw(amount: &str) -> Money {
    Money::parse(amount, Currency::KRW).expect("valid KRW money")
}

fn qty(n: u64) -> Quantity {
    Quantity::parse(&n.to_string()).expect("valid quantity")
}

fn price(p: &str) -> FixedPoint {
    FixedPoint::parse(p).expect("valid price")
}

/// The same profile the ledger contract suite uses: real KRX commission with
/// no slippage, so the cash assertions below are about fills rather than
/// about a slippage model.
fn cost_profile() -> CostProfile {
    CostProfile::custom("0.00015", "1000", "0", 0, "0", "0").expect("custom profile")
}

fn order_id(s: &str) -> OrderId {
    OrderId::from_str(s).expect("valid order id")
}

fn fill_id(s: &str) -> FillId {
    FillId::from_str(s).expect("valid fill id")
}

/// A funded ledger with one working buy order for 10 units.
fn ledger_with_open_order() -> LedgerState {
    let mut ledger = LedgerState::new(krw("10000000"), cost_profile());
    ledger
        .apply(LedgerEvent::OrderPlaced {
            seq: 1,
            order_id: order_id("11111111-1111-4111-8111-111111111111"),
            instrument_id: instrument(A),
            side: Side::Buy,
            quantity: qty(10),
        })
        .expect("order placed");
    ledger
}

fn partial(seq: u64, fill: &str, quantity: u64) -> LedgerEvent {
    LedgerEvent::Fill {
        seq,
        fill_id: fill_id(fill),
        order_id: order_id("11111111-1111-4111-8111-111111111111"),
        instrument_id: instrument(A),
        side: Side::Buy,
        quantity: qty(quantity),
        price: price("7250"),
    }
}

const F1: &str = "22222222-2222-4222-8222-000000000001";
const F2: &str = "22222222-2222-4222-8222-000000000002";
const F3: &str = "22222222-2222-4222-8222-000000000003";

#[test]
fn live_order_state_partial_fills_accumulate_to_the_full_position_once() {
    // Three partials of a 10-unit order: the position must end at exactly 10,
    // and cash must move exactly three times.
    let mut ledger = ledger_with_open_order();
    let cash_before = ledger.cash();

    ledger.apply(partial(2, F1, 4)).expect("first partial");
    ledger.apply(partial(3, F2, 3)).expect("second partial");
    ledger.apply(partial(4, F3, 3)).expect("third partial");

    assert_eq!(ledger.position(&instrument(A)), Some(&qty(10)));
    assert_eq!(ledger.fills().len(), 3);
    assert!(
        ledger.cash().amount() < cash_before.amount(),
        "three buys must have consumed cash"
    );

    // Every fill is explained by exactly one record, and the cash chain is
    // continuous: each fill's `cash_before` is the previous one's `cash_after`.
    let fills = ledger.fills();
    for pair in fills.windows(2) {
        assert_eq!(
            pair[0].cash_after, pair[1].cash_before,
            "the cash chain must have no gaps"
        );
    }
    assert_eq!(fills[2].cash_after, ledger.cash());
}

#[test]
fn live_order_state_a_duplicate_fill_is_refused_by_the_ledger_itself() {
    // The state machine would have reported NoChange for a re-sent report, but
    // the ledger must not depend on that: a duplicate `fill_id` is refused
    // here regardless of what the caller believed.
    let mut ledger = ledger_with_open_order();
    ledger.apply(partial(2, F1, 4)).expect("first partial");

    let err = ledger
        .apply(partial(3, F1, 4))
        .expect_err("a duplicate fill must be refused");
    assert!(
        matches!(err, PortfolioError::DuplicateFill { .. }),
        "expected DuplicateFill, got {err:?}"
    );

    // And nothing moved: not the position, not the cash, not the fill list.
    assert_eq!(ledger.position(&instrument(A)), Some(&qty(4)));
    assert_eq!(ledger.fills().len(), 1);
}

#[test]
fn live_order_state_a_replayed_batch_reports_which_fills_were_already_applied() {
    // The restart case. After a crash the caller re-reads its event log and
    // replays it; `already_applied` is how it tells which entries the ledger
    // has seen, so a resumed session does not die on its first duplicate.
    let mut ledger = ledger_with_open_order();
    let batch = vec![partial(2, F1, 4), partial(3, F2, 3)];
    for event in &batch {
        ledger.apply(event.clone()).expect("applied");
    }

    // Replaying the same batch: every entry is recognised as already applied.
    for event in &batch {
        assert!(
            ledger.already_applied(event),
            "a replayed fill must be recognised, not re-applied"
        );
    }
    // A fill it has NOT seen is not falsely recognised.
    assert!(!ledger.already_applied(&partial(4, F3, 3)));

    // Filtering by that predicate, the resumed batch applies only the new one
    // and the position lands where it should.
    let resumed = vec![partial(2, F1, 4), partial(3, F2, 3), partial(4, F3, 3)];
    for event in resumed {
        if !ledger.already_applied(&event) {
            ledger.apply(event).expect("only the unseen one applies");
        }
    }
    assert_eq!(ledger.position(&instrument(A)), Some(&qty(10)));
    assert_eq!(ledger.fills().len(), 3);
}

#[test]
fn live_order_state_fills_beyond_the_order_quantity_are_refused() {
    // The state machine refuses a cumulative total above the order size; the
    // ledger refuses the same thing from its own accounting. Either alone
    // would be a single point of failure for a doubled position.
    let mut ledger = ledger_with_open_order();
    ledger.apply(partial(2, F1, 6)).expect("first partial");

    let err = ledger
        .apply(partial(3, F2, 5))
        .expect_err("11 units against a 10-unit order must be refused");
    assert!(
        matches!(err, PortfolioError::OverFill { .. }),
        "expected OverFill, got {err:?}"
    );
    assert_eq!(ledger.position(&instrument(A)), Some(&qty(6)));
}

#[test]
fn live_order_state_replaying_the_whole_log_reproduces_the_same_ledger() {
    // The same property the order machine has: state is a function of the log.
    // A Live node that restarts mid-order rebuilds from events alone.
    let events = vec![
        LedgerEvent::OrderPlaced {
            seq: 1,
            order_id: order_id("11111111-1111-4111-8111-111111111111"),
            instrument_id: instrument(A),
            side: Side::Buy,
            quantity: qty(10),
        },
        partial(2, F1, 4),
        partial(3, F2, 3),
        partial(4, F3, 3),
        LedgerEvent::MarkToMarket {
            seq: 5,
            date: TradingDate::from_str("2026-08-07").expect("valid date"),
            prices: BTreeMap::from([(instrument(A), Price::parse("7300").expect("valid price"))]),
        },
    ];

    let mut direct = LedgerState::new(krw("10000000"), cost_profile());
    for event in &events {
        direct.apply(event.clone()).expect("applied");
    }

    let replayed = LedgerState::new(krw("10000000"), cost_profile())
        .replay_onto(&events)
        .expect("replay succeeds");

    assert_eq!(
        direct.position(&instrument(A)),
        replayed.position(&instrument(A))
    );
    assert_eq!(direct.cash(), replayed.cash());
    assert_eq!(direct.fills().len(), replayed.fills().len());
    assert_eq!(
        direct.canonical_bytes().expect("canonical"),
        replayed.canonical_bytes().expect("canonical"),
        "a replayed ledger must be byte-identical, not merely equivalent"
    );
}
