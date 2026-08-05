//! Red-first contract suite for the canonical ledger (design §9.2, §10).
//!
//! The ONE ledger shared by backtest, Paper, and Live. Covers the canonical
//! order/fill/cash/position/daily-equity transitions and every documented
//! failure mode as a typed reject (never a panic, never silent): cash
//! shortage, duplicate fill, unknown/duplicate order, overfill, side and
//! instrument mismatch, sell without position, out-of-order events,
//! impossible precision, zero quantity, and the minimum-fee edge.

use std::collections::BTreeMap;
use std::str::FromStr;

use domain::{
    ContentHash, Currency, FillId, FixedPoint, InstrumentId, Money, OrderId, Price, Quantity,
    TradingDate,
};
use proptest::prelude::*;
use proptest::test_runner::Config as PropConfig;

use portfolio_model::cost::CostProfile;
use portfolio_model::error::PortfolioError;
use portfolio_model::ledger::{LedgerEvent, LedgerState};
use portfolio_model::side::Side;
use portfolio_model::sizing::{plan_rebalance, SizingInput, TargetAllocation};

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

fn price(amount: &str) -> Price {
    Price::parse(amount).expect("valid price")
}

fn parse_order(s: &str) -> OrderId {
    OrderId::from_str(s).expect("valid order id")
}

fn parse_fill(s: &str) -> FillId {
    FillId::from_str(s).expect("valid fill id")
}

fn zero_slippage_profile() -> CostProfile {
    CostProfile::custom("0.00015", "1000", "0", 0, "0", "0").expect("custom profile")
}

fn default_profile() -> CostProfile {
    CostProfile::krx_etf_default().expect("default profile")
}

fn deposit(seq: u64, amount: &str) -> LedgerEvent {
    LedgerEvent::CashDeposit { seq, amount: krw(amount) }
}

fn place(seq: u64, order: &str, id: &str, side: Side, n: u64) -> LedgerEvent {
    LedgerEvent::OrderPlaced {
        seq,
        order_id: parse_order(order),
        instrument_id: instrument(id),
        side,
        quantity: qty(n),
    }
}

fn fill(seq: u64, fill_id: &str, order: &str, id: &str, side: Side, n: u64, px: &str) -> LedgerEvent {
    LedgerEvent::Fill {
        seq,
        fill_id: parse_fill(fill_id),
        order_id: parse_order(order),
        instrument_id: instrument(id),
        side,
        quantity: qty(n),
        price: FixedPoint::parse(px).expect("fill price"),
    }
}

fn mark(seq: u64, date: &str, prices: &[(&str, &str)]) -> LedgerEvent {
    LedgerEvent::MarkToMarket {
        seq,
        date: TradingDate::parse(date).expect("date"),
        prices: prices.iter().map(|(id, px)| (instrument(id), price(px))).collect(),
    }
}

fn new_state(initial: &str) -> LedgerState {
    LedgerState::new(krw(initial), zero_slippage_profile())
}

fn apply_ok(state: &mut LedgerState, event: LedgerEvent) {
    state.apply(event).expect("event applies");
}

#[test]
fn deposit_sets_cash_and_deposits_accumulate() {
    let mut state = new_state("0");
    apply_ok(&mut state, deposit(1, "5000000"));
    assert_eq!(state.cash(), krw("5000000.0000"));
    // A mid-stream deposit (Paper top-up) is allowed and deterministic.
    apply_ok(&mut state, place(2, "00000000-0000-0000-0000-000000000001", A, Side::Buy, 10));
    apply_ok(&mut state, fill(3, "00000000-0000-0000-0000-000000000011", "00000000-0000-0000-0000-000000000001", A, Side::Buy, 10, "10150.0000"));
    apply_ok(&mut state, deposit(4, "2500000"));
    assert_eq!(state.cash(), krw("7446005.0000"), "5,000,000 - 10x10150 - 1000 fee + 2,500,000");
}

#[test]
fn buy_then_sell_round_trip_keeps_exact_fees() {
    let mut state = new_state("10000000");
    apply_ok(&mut state, place(1, "00000000-0000-0000-0000-000000000001", A, Side::Buy, 100));
    apply_ok(&mut state, fill(2, "00000000-0000-0000-0000-000000000011", "00000000-0000-0000-0000-000000000001", A, Side::Buy, 100, "10160.1500"));
    // buy: 100 x 10160.15 = 1,016,015 + 1,000 min commission
    assert_eq!(state.cash(), krw("8982985.0000"));
    assert_eq!(state.position(&instrument(A)).copied(), Some(qty(100)));

    apply_ok(&mut state, place(3, "00000000-0000-0000-0000-000000000002", A, Side::Sell, 100));
    apply_ok(&mut state, fill(4, "00000000-0000-0000-0000-000000000012", "00000000-0000-0000-0000-000000000002", A, Side::Sell, 100, "10139.8500"));
    // sell: 100 x 10139.85 = 1,013,985 - 1,000 min commission
    assert_eq!(state.cash(), krw("9995970.0000"));
    assert_eq!(state.position(&instrument(A)).copied(), Some(qty(0)));
    assert_eq!(state.fills().len(), 2);
    // Every KRW accounted: 10,000,000 - 1,016,015 - 1000 + 1,013,985 - 1000
    assert_eq!(state.equity().expect("equity"), krw("9995970.0000"), "all cash, no positions");
}

#[test]
fn cash_shortage_is_a_typed_reject_and_state_is_unchanged() {
    let mut state = new_state("1000000");
    let before = state.clone();
    apply_ok(&mut state, place(1, "00000000-0000-0000-0000-000000000001", A, Side::Buy, 100));
    let err = state
        .apply(fill(2, "00000000-0000-0000-0000-000000000011", "00000000-0000-0000-0000-000000000001", A, Side::Buy, 100, "10150.0000"))
        .expect_err("buy of 1,016,000 + 1,000 > 1,000,000");
    assert!(matches!(err, PortfolioError::InsufficientCash { .. }));
    assert_eq!(&*state, &before, "rejected event must not mutate the ledger");
}

#[test]
fn duplicate_fill_is_rejected() {
    let mut state = new_state("10000000");
    apply_ok(&mut state, place(1, "00000000-0000-0000-0000-000000000001", A, Side::Buy, 100));
    apply_ok(&mut state, fill(2, "00000000-0000-0000-0000-000000000011", "00000000-0000-0000-0000-000000000001", A, Side::Buy, 50, "10150.0000"));
    let err = state
        .apply(fill(3, "00000000-0000-0000-0000-000000000011", "00000000-0000-0000-0000-000000000001", A, Side::Buy, 50, "10150.0000"))
        .expect_err("same fill id twice");
    assert!(matches!(err, PortfolioError::DuplicateFill { .. }));
    assert_eq!(state.position(&instrument(A)).copied(), Some(qty(50)));
}

#[test]
fn unknown_order_is_rejected() {
    let mut state = new_state("10000000");
    let err = state
        .apply(fill(1, "00000000-0000-0000-0000-000000000011", "00000000-0000-0000-0000-000000000099", A, Side::Buy, 10, "10150.0000"))
        .expect_err("fill without a placed order");
    assert!(matches!(err, PortfolioError::UnknownOrder { .. }));
}

#[test]
fn duplicate_order_is_rejected() {
    let mut state = new_state("10000000");
    apply_ok(&mut state, place(1, "00000000-0000-0000-0000-000000000001", A, Side::Buy, 100));
    let err = state
        .apply(place(2, "00000000-0000-0000-0000-000000000001", A, Side::Buy, 50))
        .expect_err("same order id twice");
    assert!(matches!(err, PortfolioError::DuplicateOrder { .. }));
}

#[test]
fn overfill_is_rejected() {
    let mut state = new_state("10000000");
    apply_ok(&mut state, place(1, "00000000-0000-0000-0000-000000000001", A, Side::Buy, 100));
    apply_ok(&mut state, fill(2, "00000000-0000-0000-0000-000000000011", "00000000-0000-0000-0000-000000000001", A, Side::Buy, 60, "10150.0000"));
    let err = state
        .apply(fill(3, "00000000-0000-0000-0000-000000000012", "00000000-0000-0000-0000-000000000001", A, Side::Buy, 60, "10150.0000"))
        .expect_err("60 > remaining 40");
    assert!(matches!(err, PortfolioError::OverFill { .. }));
}

#[test]
fn side_mismatch_is_rejected() {
    let mut state = new_state("10000000");
    apply_ok(&mut state, place(1, "00000000-0000-0000-0000-000000000001", A, Side::Buy, 100));
    let err = state
        .apply(fill(2, "00000000-0000-0000-0000-000000000011", "00000000-0000-0000-0000-000000000001", A, Side::Sell, 10, "10150.0000"))
        .expect_err("sell fill on a buy order");
    assert!(matches!(err, PortfolioError::SideMismatch { .. }));
}

#[test]
fn instrument_mismatch_is_rejected() {
    let mut state = new_state("10000000");
    apply_ok(&mut state, place(1, "00000000-0000-0000-0000-000000000001", A, Side::Buy, 100));
    let err = state
        .apply(fill(2, "00000000-0000-0000-0000-000000000011", "00000000-0000-0000-0000-000000000001", B, Side::Buy, 10, "10150.0000"))
        .expect_err("fill on a different instrument than the order");
    assert!(matches!(err, PortfolioError::InstrumentMismatch { .. }));
}

#[test]
fn sell_without_position_is_rejected() {
    let mut state = new_state("10000000");
    apply_ok(&mut state, place(1, "00000000-0000-0000-0000-000000000001", A, Side::Sell, 10));
    let err = state
        .apply(fill(2, "00000000-0000-0000-0000-000000000011", "00000000-0000-0000-0000-000000000001", A, Side::Sell, 10, "10150.0000"))
        .expect_err("short selling is unsupported");
    assert!(matches!(err, PortfolioError::SellWithoutPosition { .. }));
}

#[test]
fn sell_beyond_position_is_rejected() {
    let mut state = new_state("10000000");
    apply_ok(&mut state, place(1, "00000000-0000-0000-0000-000000000001", A, Side::Buy, 100));
    apply_ok(&mut state, fill(2, "00000000-0000-0000-0000-000000000011", "00000000-0000-0000-0000-000000000001", A, Side::Buy, 40, "10150.0000"));
    apply_ok(&mut state, place(3, "00000000-0000-0000-0000-000000000002", A, Side::Sell, 60));
    let err = state
        .apply(fill(4, "00000000-0000-0000-0000-000000000012", "00000000-0000-0000-0000-000000000002", A, Side::Sell, 60, "10150.0000"))
        .expect_err("sell 60 > held 40");
    assert!(matches!(err, PortfolioError::SellWithoutPosition { .. }));
}

#[test]
fn out_of_order_and_equal_seqs_are_rejected() {
    let mut state = new_state("10000000");
    apply_ok(&mut state, deposit(5, "1000000"));
    let err = state
        .apply(deposit(3, "1000000"))
        .expect_err("seq 3 after seq 5");
    assert!(matches!(err, PortfolioError::OutOfOrderEvent { .. }));
    let err = state.apply(deposit(5, "1000000")).expect_err("seq 5 == last seq 5");
    assert!(matches!(err, PortfolioError::OutOfOrderEvent { .. }));
}

#[test]
fn impossible_precision_is_a_typed_error() {
    let mut state = new_state("10000000");
    apply_ok(&mut state, place(1, "00000000-0000-0000-0000-000000000001", A, Side::Buy, 10));
    let err = state
        .apply(fill(2, "00000000-0000-0000-0000-000000000011", "00000000-0000-0000-0000-000000000001", A, Side::Buy, 10, "123.45678"))
        .expect_err("5dp fill price is impossible for KRW scale-4");
    assert!(matches!(err, PortfolioError::PrecisionExceeded { .. }));
    // A trailing-zero higher-scale value is lossless and accepted.
    apply_ok(&mut state, fill(3, "00000000-0000-0000-0000-000000000012", "00000000-0000-0000-0000-000000000001", A, Side::Buy, 10, "123.45000"));
    let record = state.fills().last().expect("fill record");
    assert_eq!(record.price, price("123.4500"));
}

#[test]
fn zero_quantity_events_are_rejected() {
    let mut state = new_state("10000000");
    let err = state
        .apply(place(1, "00000000-0000-0000-0000-000000000001", A, Side::Buy, 0))
        .expect_err("zero-quantity order");
    assert!(matches!(err, PortfolioError::ZeroQuantity { .. }));
    apply_ok(&mut state, place(2, "00000000-0000-0000-0000-000000000002", A, Side::Buy, 10));
    let err = state
        .apply(fill(3, "00000000-0000-0000-0000-000000000011", "00000000-0000-0000-0000-000000000002", A, Side::Buy, 0, "10150.0000"))
        .expect_err("zero-quantity fill");
    assert!(matches!(err, PortfolioError::ZeroQuantity { .. }));
}

#[test]
fn partial_fills_aggregate_and_equity_is_consistent() {
    let mut state = new_state("10000000");
    apply_ok(&mut state, place(1, "00000000-0000-0000-0000-000000000001", A, Side::Buy, 100));
    apply_ok(&mut state, fill(2, "00000000-0000-0000-0000-000000000011", "00000000-0000-0000-0000-000000000001", A, Side::Buy, 30, "10160.1500"));
    apply_ok(&mut state, fill(3, "00000000-0000-0000-0000-000000000012", "00000000-0000-0000-0000-000000000001", A, Side::Buy, 70, "10160.1500"));
    assert_eq!(state.position(&instrument(A)).copied(), Some(qty(100)));
    assert_eq!(state.cash(), krw("8981985.0000"), "10M - 1,016,015 - 2,000 in fees");
    // 30 x 10160.15 = 304,804.50; 70 x 10160.15 = 711,210.50; fees 2 x 1000.
    apply_ok(&mut state, mark(4, "2026-02-02", &[(A, "10200.0000")]));
    assert_eq!(state.equity().expect("equity"), krw("10001985.0000"));
    assert_eq!(
        state.equity().expect("equity"),
        state
            .cash()
            .checked_add(&qty(100).checked_mul_price(&price("10200.0000"), Currency::KRW).expect("mark"))
            .expect("sum"),
        "cash + marked positions == daily equity"
    );
    assert_eq!(state.equity_curve().get(&TradingDate::parse("2026-02-02").expect("date")), Some(&krw("10001985.0000")));
}

#[test]
fn mark_requires_every_position_to_be_priced() {
    let mut state = new_state("10000000");
    apply_ok(&mut state, place(1, "00000000-0000-0000-0000-000000000001", A, Side::Buy, 10));
    apply_ok(&mut state, fill(2, "00000000-0000-0000-0000-000000000011", "00000000-0000-0000-0000-000000000001", A, Side::Buy, 10, "10150.0000"));
    apply_ok(&mut state, place(3, "00000000-0000-0000-0000-000000000002", B, Side::Buy, 10));
    apply_ok(&mut state, fill(4, "00000000-0000-0000-0000-000000000012", "00000000-0000-0000-0000-000000000002", B, Side::Buy, 10, "24850.0000"));
    let err = state
        .apply(mark(5, "2026-02-02", &[(A, "10200.0000")]))
        .expect_err("B is held but not marked");
    assert!(matches!(err, PortfolioError::MissingMark { .. }));
}

#[test]
fn remarking_the_same_date_overwrites_deterministically() {
    let mut state = new_state("10000000");
    apply_ok(&mut state, place(1, "00000000-0000-0000-0000-000000000001", A, Side::Buy, 10));
    apply_ok(&mut state, fill(2, "00000000-0000-0000-0000-000000000011", "00000000-0000-0000-0000-000000000001", A, Side::Buy, 10, "10150.0000"));
    apply_ok(&mut state, mark(3, "2026-02-02", &[(A, "10000.0000")]));
    apply_ok(&mut state, mark(4, "2026-02-02", &[(A, "10200.0000")]));
    let curve = state.equity_curve();
    assert_eq!(curve.len(), 1, "same date overwrites (deterministic)");
    assert_eq!(curve.get(&TradingDate::parse("2026-02-02").expect("date")), Some(&krw("10001985.0000")));
}

#[test]
fn minimum_fee_edge_is_deterministic_across_ledgers() {
    // The same tiny fill in two fresh ledgers produces IDENTICAL records.
    let mut s1 = new_state("10000000");
    let mut s2 = new_state("10000000");
    for s in [&mut s1, &mut s2] {
        apply_ok(s, place(1, "00000000-0000-0000-0000-000000000001", A, Side::Buy, 1));
        apply_ok(s, fill(2, "00000000-0000-0000-0000-000000000011", "00000000-0000-0000-0000-000000000001", A, Side::Buy, 1, "1000.0000"));
    }
    assert_eq!(s1, s2);
    let f1 = &s1.fills()[0];
    let f2 = &s2.fills()[0];
    assert_eq!(f1.commission, krw("1000.0000"), "minimum commission applies exactly");
    assert_eq!(f1.tax, krw("0.0000"));
    assert_eq!(f1.notional, krw("1000.0000"));
    assert_eq!(f1.cash_before, krw("10000000.0000"));
    assert_eq!(f1.cash_after, krw("9998000.0000"));
    assert_eq!(f1.cash_after, f2.cash_after, "deterministic across ledgers");
    // fee identity: cash_before - cash_after == notional + commission
    let debit = f1
        .cash_before
        .checked_sub(&f1.cash_after)
        .expect("debit");
    let expected = f1
        .notional
        .checked_add(&f1.commission)
        .expect("notional + commission")
        .checked_add(&f1.tax)
        .expect("+ tax");
    assert_eq!(debit, expected, "fees always balanced against cash");
}

#[test]
fn full_state_json_round_trip_is_stable() {
    let mut state = new_state("10000000");
    apply_ok(&mut state, place(1, "00000000-0000-0000-0000-000000000001", A, Side::Buy, 10));
    apply_ok(&mut state, fill(2, "00000000-0000-0000-0000-000000000011", "00000000-0000-0000-0000-000000000001", A, Side::Buy, 10, "10150.0000"));
    apply_ok(&mut state, mark(3, "2026-02-02", &[(A, "10200.0000")]));
    let bytes = state.canonical_bytes().expect("canonical bytes");
    let back: LedgerState = serde_json::from_slice(&bytes).expect("deserialize");
    assert_eq!(back, state);
    assert_eq!(back.canonical_bytes().expect("canonical bytes"), bytes, "byte-stable");
    let hash = ContentHash::from_bytes(&bytes);
    assert!(hash.as_str().starts_with("sha256:"), "hashed canonical state");
}

/// A deterministic fill stream (buys then sells <= half position) plus the
/// initial cash that provably covers it: seed -> stream, replayable forever.
fn seeded_stream(seed: u64) -> (Money, Vec<LedgerEvent>, CostProfile) {
    let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(0xD1B5_4A32_D192_ED03);
    let mut next = move || {
        s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        s >> 33
    };
    let instruments = [instrument(A), instrument(B), instrument(C)];
    let exec_prices = ["10150.0000", "24850.0000", "5850.0000"];
    let profile = zero_slippage_profile();
    let rate = profile.commission_rate;
    let mut events = Vec::new();
    let mut held = [0u64; 3];
    let mut seq = 0u64;
    let mut total_needed = FixedPoint::ZERO;

    for i in 0..6 {
        let idx = i % 3;
        let px = FixedPoint::parse(exec_prices[idx]).expect("price");
        let buy = i < 3;
        let n = 1 + (next() % 40) as u64;
        if buy {
            held[idx] += n;
            let notional = FixedPoint::from_i128(n as i128, 0)
                .expect("qty")
                .checked_mul(&px)
                .expect("notional");
            let raw_fee = notional.checked_mul(&rate).expect("fee").with_scale(4).expect("fee scale");
            let min = profile.min_commission.amount();
            let fee = if raw_fee < min { min } else { raw_fee };
            total_needed = total_needed.checked_add(&notional.checked_add(&fee).expect("sum")).expect("total");
        } else {
            // sell <= half of what remains held: never short, never overfill.
            let n = n.min(held[idx] / 2);
            held[idx] -= n;
        }
        let order = format!("00000000-0000-0000-0000-{:012}", i + 1);
        let fill_id = format!("00000000-0000-0000-0000-{:012}", i + 100);
        seq += 1;
        events.push(LedgerEvent::OrderPlaced {
            seq,
            order_id: parse_order(&order),
            instrument_id: instruments[idx].clone(),
            side: if buy { Side::Buy } else { Side::Sell },
            quantity: qty(n),
        });
        seq += 1;
        events.push(LedgerEvent::Fill {
            seq,
            fill_id: parse_fill(&fill_id),
            order_id: parse_order(&order),
            instrument_id: instruments[idx].clone(),
            side: if buy { Side::Buy } else { Side::Sell },
            quantity: qty(n),
            price: px,
        });
    }
    seq += 1;
    events.push(LedgerEvent::MarkToMarket {
        seq,
        date: TradingDate::parse("2026-02-02").expect("date"),
        prices: BTreeMap::from([
            (instrument(A), price("10200.0000")),
            (instrument(B), price("24900.0000")),
            (instrument(C), price("5900.0000")),
        ]),
    });
    // initial cash = everything the buys need + a fixed margin.
    let margin = FixedPoint::from_i128(1_000_000_000, 4).expect("margin"); // 100,000 KRW
    let initial = Money::from_fixed(total_needed.checked_add(&margin).expect("initial"), Currency::KRW)
        .expect("initial money");
    (initial, events, profile)
}

/// An INDEPENDENT naive fold over the same event stream: positions as raw
/// u64 sums, cash via a hand-rolled commission formula.
fn naive_fold(
    initial: Money,
    events: &[LedgerEvent],
    profile: &CostProfile,
) -> (BTreeMap<InstrumentId, u64>, FixedPoint) {
    let rate = profile.commission_rate;
    let min = profile.min_commission.amount();
    let mut positions: BTreeMap<InstrumentId, i64> = BTreeMap::new();
    let mut cash = initial.amount();
    for event in events {
        if let LedgerEvent::Fill { instrument_id, side, quantity, price, .. } = event {
            let n = quantity.to_u64().expect("qty");
            let notional = FixedPoint::from_i128(n as i128, 0)
                .expect("qty")
                .checked_mul(price)
                .expect("notional");
            let raw_fee = notional.checked_mul(&rate).expect("fee").with_scale(4).expect("fee");
            let fee = if raw_fee < min { min } else { raw_fee };
            let delta = if side.is_buy() {
                cash = cash.checked_sub(&notional.checked_add(&fee).expect("sum")).expect("debit");
                n as i64
            } else {
                cash = cash.checked_add(&notional.checked_sub(&fee).expect("proceeds")).expect("credit");
                -(n as i64)
            };
            let entry = positions.entry(instrument_id.clone()).or_insert(0i64);
            *entry += delta;
        }
    }
    let positions = positions
        .into_iter()
        .map(|(id, v)| (id, u64::try_from(v).expect("non-negative position")))
        .collect();
    (positions, cash)
}

proptest! {
    #![proptest_config(PropConfig::with_cases(64))]

    /// Property: fills aggregate to positions and fees to cash debits -
    /// the ledger matches an independent naive fold exactly.
    #[test]
    fn fills_aggregate_to_positions_and_fees_debit_cash(seed in 0u64..1_000_000u64) {
        let (initial, events, profile) = seeded_stream(seed);
        let state = LedgerState::replay(initial, profile.clone(), &events).expect("replay");
        let (ref_pos, ref_cash) = naive_fold(initial, &events, &profile);
        for (id, expected) in &ref_pos {
            let id_str = id.to_string();
            prop_assert_eq!(
                state.position(id).map(|q| q.to_u64().expect("qty")),
                Some(*expected),
                "position aggregation for {}",
                id_str
            );
        }
        prop_assert_eq!(state.cash().amount(), ref_cash, "cash == independent fold");
        let rate = profile.commission_rate;
        let min = profile.min_commission.amount();
        // The fee identity per fill record: cash_before - cash_after ==
        // +/-notional -/+ (commission + tax), recomputed independently.
        for record in state.fills() {
            let n = record.quantity.to_u64().expect("qty");
            let notional = FixedPoint::from_i128(n as i128, 0)
                .expect("qty")
                .checked_mul(&record.price.amount())
                .expect("notional");
            let raw_fee = notional.checked_mul(&rate).expect("fee").with_scale(4).expect("fee");
            let fee = if raw_fee < min { min } else { raw_fee };
            prop_assert_eq!(record.commission.amount(), fee, "commission recomputation");
            prop_assert_eq!(record.tax, Money::zero(Currency::KRW), "no tax in this profile");
            let delta = if record.side.is_buy() {
                notional.checked_add(&fee).expect("debit")
            } else {
                record
                    .notional
                    .amount()
                    .checked_sub(&fee)
                    .expect("proceeds")
            };
            let expected = if record.side.is_buy() {
                record.cash_before.amount().checked_sub(&delta).expect("after")
            } else {
                record.cash_before.amount().checked_add(&delta).expect("after")
            };
            prop_assert_eq!(record.cash_after.amount(), expected, "fee-to-cash debit identity");
        }
        // Equity identity after the mark: cash + marked positions.
        let mut marked = state.cash();
        for (id, q) in &ref_pos {
            let mark = state.marks().get(id).expect("mark");
            marked = marked
                .checked_add(&qty(*q).checked_mul_price(mark, Currency::KRW).expect("value"))
                .expect("sum");
        }
        prop_assert_eq!(state.equity().expect("equity"), marked, "cash + marked == equity");
    }
}
