//! Red-first suite for deterministic replay and the persistence seam.
//!
//! Replay contract: the same event stream applied to the same initial state
//! ALWAYS produces the same ledger state, byte for byte (canonical
//! serialization). Replay is idempotent (append-replay onto a snapshot is
//! identical to a full replay) and snapshot round-trips through the
//! `LedgerStore` seam exactly.
//!
//! Persistence seam (Todo 3 is BLOCKED): the ledger core is deliberately
//! DB-free. `LedgerStore` is the contract a Todo-3 PostgreSQL implementation
//! will provide; `InMemoryLedgerStore` is the tested in-memory
//! implementation used by Paper/backtest today (SessionStore precedent,
//! Todo 22).

use std::collections::BTreeMap;

use domain::{ContentHash, Currency, FixedPoint, InstrumentId, Money, OrderId, Quantity, TradingDate};
use std::str::FromStr;

use portfolio_model::cost::CostProfile;
use portfolio_model::ledger::{LedgerEvent, LedgerState};
use portfolio_model::persistence::{InMemoryLedgerStore, LedgerStore};
use portfolio_model::side::Side;

const A: &str = "069500.KRX";
const B: &str = "229200.KRX";
const C: &str = "114260.KRX";

fn instrument(s: &str) -> InstrumentId {
    InstrumentId::parse(s).expect("valid instrument id")
}

fn krw(amount: &str) -> Money {
    Money::parse(amount, Currency::KRW).expect("valid KRW money")
}

fn qty(n: u64) -> Quantity {
    Quantity::parse(&n.to_string()).expect("valid quantity")
}

fn profile() -> CostProfile {
    CostProfile::custom("0.00015", "1000", "0", 0, "0", "0").expect("custom profile")
}

fn parse_order(s: &str) -> OrderId {
    OrderId::from_str(s).expect("valid order id")
}

/// A fixed, realistic event stream: deposit, two buys, a partial sell, marks.
fn golden_stream() -> Vec<LedgerEvent> {
    let mut events = Vec::new();
    events.push(LedgerEvent::CashDeposit { seq: 1, amount: krw("10000000") });
    events.push(LedgerEvent::OrderPlaced {
        seq: 2,
        order_id: parse_order("00000000-0000-0000-0000-000000000001"),
        instrument_id: instrument(A),
        side: Side::Buy,
        quantity: qty(590),
    });
    events.push(LedgerEvent::Fill {
        seq: 3,
        fill_id: domain::FillId::from_str("00000000-0000-0000-0000-000000000011").expect("fill"),
        order_id: parse_order("00000000-0000-0000-0000-000000000001"),
        instrument_id: instrument(A),
        side: Side::Buy,
        quantity: qty(590),
        price: FixedPoint::parse("10160.1500").expect("price"),
    });
    events.push(LedgerEvent::OrderPlaced {
        seq: 4,
        order_id: parse_order("00000000-0000-0000-0000-000000000002"),
        instrument_id: instrument(B),
        side: Side::Buy,
        quantity: qty(160),
    });
    events.push(LedgerEvent::Fill {
        seq: 5,
        fill_id: domain::FillId::from_str("00000000-0000-0000-0000-000000000012").expect("fill"),
        order_id: parse_order("00000000-0000-0000-0000-000000000002"),
        instrument_id: instrument(B),
        side: Side::Buy,
        quantity: qty(160),
        price: FixedPoint::parse("24874.8500").expect("price"),
    });
    events.push(LedgerEvent::MarkToMarket {
        seq: 6,
        date: TradingDate::parse("2026-02-02").expect("date"),
        prices: BTreeMap::from([
            (instrument(A), domain::Price::parse("10200.0000").expect("mark")),
            (instrument(B), domain::Price::parse("24900.0000").expect("mark")),
            (instrument(C), domain::Price::parse("5900.0000").expect("mark")),
        ]),
    });
    events
}

#[test]
fn replay_twice_is_byte_identical() {
    let events = golden_stream();
    let s1 = LedgerState::replay(krw("10000000"), profile(), &events).expect("replay 1");
    let s2 = LedgerState::replay(krw("10000000"), profile(), &events).expect("replay 2");
    assert_eq!(s1, s2, "replay is deterministic");
    let b1 = s1.canonical_bytes().expect("canonical bytes");
    let b2 = s2.canonical_bytes().expect("canonical bytes");
    assert_eq!(b1, b2, "byte-identical ledger state across replays");
    assert_eq!(
        ContentHash::from_bytes(&b1).as_str(),
        ContentHash::from_bytes(&b2).as_str(),
        "identical content hash"
    );
}

#[test]
fn replay_from_snapshot_is_idempotent() {
    // Append-replay onto a snapshot equals a full replay of the whole stream.
    let events = golden_stream();
    let full = LedgerState::replay(krw("10000000"), profile(), &events).expect("full replay");
    let split = events.split_at(3);
    let head = LedgerState::replay(krw("10000000"), profile(), split.0).expect("head replay");
    let appended = head.replay_onto(split.1).expect("append replay");
    assert_eq!(appended, full, "replay_onto(snapshot, rest) == replay(all)");
    assert_eq!(
        appended.canonical_bytes().expect("bytes"),
        full.canonical_bytes().expect("bytes")
    );
}

#[test]
fn replay_rejects_the_same_stream_that_apply_rejects() {
    // Malformed streams fail identically whether replayed or applied live.
    let mut events = golden_stream();
    let mut bad = events.clone();
    bad[3] = LedgerEvent::CashDeposit { seq: 3, amount: krw("1") }; // duplicate seq
    let live = LedgerState::new(krw("10000000"), profile());
    let err_live = live.apply(bad[3].clone()).expect_err("out-of-order live");
    let err_replay = LedgerState::replay(krw("10000000"), profile(), &bad).expect_err("out-of-order replay");
    assert_eq!(format!("{err_live}"), format!("{err_replay}"), "identical typed reject");
    events[3] = bad[3].clone();
}

#[test]
fn snapshot_store_round_trips_exactly() {
    let store = InMemoryLedgerStore::new();
    let events = golden_stream();
    let state = LedgerState::replay(krw("10000000"), profile(), &events).expect("replay");
    assert!(store.load_snapshot("acct-1").expect("missing load").is_none(), "missing is None");
    store.save_snapshot("acct-1", &state).expect("save");
    let loaded = store.load_snapshot("acct-1").expect("load").expect("present");
    assert_eq!(loaded, state, "store round-trip is exact");
    assert_eq!(
        loaded.canonical_bytes().expect("bytes"),
        state.canonical_bytes().expect("bytes"),
        "byte-identical snapshot"
    );
    // Two accounts are isolated.
    assert!(store.load_snapshot("acct-2").expect("other account").is_none());
}

#[test]
fn ledger_state_json_round_trip_is_byte_stable() {
    let events = golden_stream();
    let state = LedgerState::replay(krw("10000000"), profile(), &events).expect("replay");
    let bytes = state.canonical_bytes().expect("canonical bytes");
    let back: LedgerState = serde_json::from_slice(&bytes).expect("deserialize");
    assert_eq!(back, state);
    assert_eq!(back.canonical_bytes().expect("bytes"), bytes, "byte-stable");
}
