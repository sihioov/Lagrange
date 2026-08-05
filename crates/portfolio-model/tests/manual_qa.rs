//! Manual QA channel: one full rebalance (cash -> sells -> buys) with the
//! KRX_ETF_DEFAULT profile, an exact cash/position/equity trace after every
//! fill, byte-identical replay of the same stream, and the golden
//! "higher costs never improve terminal equity" assertion (design §9.3,
//! acceptance: `cargo test -p portfolio-model` with `-- --nocapture`).

use std::collections::BTreeMap;
use std::str::FromStr;

use domain::{
    ContentHash, Currency, FillId, FixedPoint, InstrumentId, Money, OrderId, Price, TradingDate,
};

use portfolio_model::cost::CostProfile;
use portfolio_model::ledger::{LedgerEvent, LedgerState};
use portfolio_model::sizing::{OrderRequest, SizingInput, TargetAllocation, plan_rebalance};

const A: &str = "069500.KRX";
const B: &str = "229200.KRX";

fn instrument(s: &str) -> InstrumentId {
    InstrumentId::parse(s).expect("valid instrument id")
}

fn krw(amount: &str) -> Money {
    Money::parse(amount, Currency::KRW).expect("valid KRW money")
}

fn opens() -> BTreeMap<InstrumentId, Price> {
    BTreeMap::from([
        (instrument(A), Price::parse("10150.0000").expect("open")),
        (instrument(B), Price::parse("24850.0000").expect("open")),
    ])
}

fn closes() -> BTreeMap<InstrumentId, Price> {
    BTreeMap::from([
        (instrument(A), Price::parse("10200.0000").expect("close")),
        (instrument(B), Price::parse("24900.0000").expect("close")),
    ])
}

fn targets(a: &str, b: &str) -> Vec<TargetAllocation> {
    vec![
        TargetAllocation {
            instrument_id: instrument(A),
            weight: portfolio_model::sizing::weight_from_ratio(a.parse().expect("a weight"))
                .expect("weight"),
        },
        TargetAllocation {
            instrument_id: instrument(B),
            weight: portfolio_model::sizing::weight_from_ratio(b.parse().expect("b weight"))
                .expect("weight"),
        },
    ]
}

fn equity_marked_at(state: &LedgerState, marks: &BTreeMap<InstrumentId, Price>) -> Money {
    let mut total = state.cash();
    for (id, qty) in state.positions() {
        let mark = marks.get(id).expect("mark");
        total = total
            .checked_add(&qty.checked_mul_price(mark, Currency::KRW).expect("value"))
            .expect("sum");
    }
    total
}

/// Applies one planned order (OrderPlaced + Fill at the exec price) and
/// prints the exact cash/position trace. Returns the fill event for replay.
fn execute_order(
    state: &mut LedgerState,
    events: &mut Vec<LedgerEvent>,
    order: &OrderRequest,
    opens: &BTreeMap<InstrumentId, Price>,
    counter: &mut u64,
    seq: &mut u64,
) -> LedgerEvent {
    *seq += 1;
    *counter += 1;
    let order_id =
        OrderId::from_str(&format!("00000000-0000-0000-0000-{counter:012}")).expect("id");
    let placed = LedgerEvent::OrderPlaced {
        seq: *seq,
        order_id,
        instrument_id: order.instrument_id.clone(),
        side: order.side,
        quantity: order.quantity,
    };
    state.apply(placed.clone()).expect("order places");
    events.push(placed);

    *seq += 1;
    *counter += 1;
    let exec = state
        .cost_profile
        .execution_price(opens.get(&order.instrument_id).expect("open"), order.side)
        .expect("exec price");
    let fill = LedgerEvent::Fill {
        seq: *seq,
        fill_id: FillId::from_str(&format!("00000000-0000-0000-0000-{counter:012}")).expect("id"),
        order_id,
        instrument_id: order.instrument_id.clone(),
        side: order.side,
        quantity: order.quantity,
        price: exec.amount(),
    };
    let effect = state.apply(fill.clone()).expect("fill applies");
    let fees = effect.fees.expect("fees");
    let position = state
        .position(&order.instrument_id)
        .map(|q| q.to_u64().expect("qty"))
        .unwrap_or(0);
    println!(
        "  fill #{counter:02} {:>4} {:>9} x {} @ {} | notional {} | commission {} tax {} | cash {} -> {} | position {}",
        order.side,
        order.instrument_id,
        order.quantity,
        exec.as_decimal_string(),
        order.order_value.as_decimal_string(),
        fees.commission.as_decimal_string(),
        fees.tax.as_decimal_string(),
        effect.cash_before.as_decimal_string(),
        effect.cash_after.as_decimal_string(),
        position,
    );
    events.push(fill.clone());
    fill
}

fn execute_plan(
    state: &mut LedgerState,
    events: &mut Vec<LedgerEvent>,
    input: &SizingInput,
    closes: &BTreeMap<InstrumentId, Price>,
    date: &str,
    counter: &mut u64,
    seq: &mut u64,
) {
    let report = plan_rebalance(input).expect("rebalance plan");
    for order in &report.orders {
        let fill = execute_order(state, events, order, &input.open_prices, counter, seq);
        let _ = fill;
    }
    *seq += 1;
    let mark = LedgerEvent::MarkToMarket {
        seq: *seq,
        date: TradingDate::parse(date).expect("date"),
        prices: closes.clone(),
    };
    let effect = state.apply(mark.clone()).expect("mark applies");
    events.push(mark);
    println!(
        "  mark {date}: daily equity = {} (cash {} + marked positions {})",
        effect.equity_after.expect("equity").as_decimal_string(),
        state.cash().as_decimal_string(),
        equity_marked_at(state, closes).as_decimal_string(),
    );
}

/// The golden two-cycle scenario under one profile -> terminal equity.
fn run_golden_scenario(profile: &CostProfile) -> Money {
    let open = opens();
    let close = closes();
    let mut state = LedgerState::new(krw("10000000"), profile.clone());
    let mut events: Vec<LedgerEvent> = Vec::new();
    let mut counter = 0u64;
    let mut seq = 0u64;

    let cycle1 = SizingInput {
        cash: state.cash(),
        positions: state.positions().clone(),
        open_prices: open.clone(),
        targets: targets("0.6", "0.4"),
        lot_sizes: BTreeMap::new(),
        profile: profile.clone(),
    };
    execute_plan(
        &mut state,
        &mut events,
        &cycle1,
        &close,
        "2026-02-02",
        &mut counter,
        &mut seq,
    );

    let cycle2 = SizingInput {
        cash: state.cash(),
        positions: state.positions().clone(),
        open_prices: open.clone(),
        targets: targets("0.4", "0.6"),
        lot_sizes: BTreeMap::new(),
        profile: profile.clone(),
    };
    execute_plan(
        &mut state,
        &mut events,
        &cycle2,
        &close,
        "2026-02-03",
        &mut counter,
        &mut seq,
    );

    let terminal = state.equity().expect("terminal equity");
    assert!(
        state.cash().amount() >= FixedPoint::ZERO,
        "cash is never negative"
    );
    assert_eq!(
        terminal,
        equity_marked_at(&state, &close),
        "cash + marked positions == daily equity at every point"
    );
    // Replay the WHOLE stream twice: byte-identical ledger state.
    let r1 = LedgerState::replay(krw("10000000"), profile.clone(), &events).expect("replay 1");
    let r2 = LedgerState::replay(krw("10000000"), profile.clone(), &events).expect("replay 2");
    assert_eq!(r1, state, "replay reproduces the live ledger");
    assert_eq!(r1, r2, "replay is deterministic");
    assert_eq!(
        r1.canonical_bytes().expect("bytes"),
        r2.canonical_bytes().expect("bytes"),
        "byte-identical ledger state"
    );
    terminal
}

#[test]
fn exact_fee_rebalance_trace_and_byte_identical_replay() {
    let profile = CostProfile::krx_etf_default().expect("default profile");
    println!("\n== Lagrange Station exact-fee rebalance (KRX_ETF_DEFAULT v1) ==");
    println!("initial cash 10,000,000.0000 KRW; opens A 10150.0000 B 24850.0000; slippage 10 bps");
    let open = opens();
    let close = closes();
    let mut state = LedgerState::new(krw("10000000"), profile.clone());
    let mut events: Vec<LedgerEvent> = Vec::new();
    let mut counter = 0u64;
    let mut seq = 0u64;

    println!("cycle 1: targets A 0.6 B 0.4 (from cash)");
    let cycle1 = SizingInput {
        cash: state.cash(),
        positions: state.positions().clone(),
        open_prices: open.clone(),
        targets: targets("0.6", "0.4"),
        lot_sizes: BTreeMap::new(),
        profile: profile.clone(),
    };
    execute_plan(
        &mut state,
        &mut events,
        &cycle1,
        &close,
        "2026-02-02",
        &mut counter,
        &mut seq,
    );
    assert_eq!(
        state.cash(),
        krw("23535.5000"),
        "exact leftover after cycle 1 buys"
    );
    assert_eq!(
        state
            .position(&instrument(A))
            .map(|q| q.to_u64().expect("qty")),
        Some(590)
    );
    assert_eq!(
        state
            .position(&instrument(B))
            .map(|q| q.to_u64().expect("qty")),
        Some(160)
    );
    assert_eq!(
        state.equity().expect("equity"),
        krw("10025535.5000"),
        "equity after cycle 1 mark"
    );

    println!("cycle 2: targets A 0.4 B 0.6 (sells first, then buys)");
    let cycle2 = SizingInput {
        cash: state.cash(),
        positions: state.positions().clone(),
        open_prices: open.clone(),
        targets: targets("0.4", "0.6"),
        lot_sizes: BTreeMap::new(),
        profile: profile.clone(),
    };
    execute_plan(
        &mut state,
        &mut events,
        &cycle2,
        &close,
        "2026-02-03",
        &mut counter,
        &mut seq,
    );
    assert_eq!(
        state.cash(),
        krw("18958.1000"),
        "exact leftover after cycle 2"
    );
    assert_eq!(
        state
            .position(&instrument(A))
            .map(|q| q.to_u64().expect("qty")),
        Some(394)
    );
    assert_eq!(
        state
            .position(&instrument(B))
            .map(|q| q.to_u64().expect("qty")),
        Some(240)
    );
    assert_eq!(
        state.equity().expect("equity"),
        krw("10013758.1000"),
        "terminal equity after cycle 2 mark"
    );

    // Replay the SAME stream twice and assert byte-identical ledger state.
    let r1 = LedgerState::replay(krw("10000000"), profile.clone(), &events).expect("replay 1");
    let r2 = LedgerState::replay(krw("10000000"), profile.clone(), &events).expect("replay 2");
    assert_eq!(r1, state, "replay reproduces the live ledger");
    assert_eq!(r1, r2, "replay is deterministic");
    let h1 = ContentHash::from_bytes(&r1.canonical_bytes().expect("bytes"));
    let h2 = ContentHash::from_bytes(&r2.canonical_bytes().expect("bytes"));
    assert_eq!(h1, h2, "byte-identical ledger state");
    println!("\nreplay #1 ledger sha256: {}", h1);
    println!("replay #2 ledger sha256: {}", h2);
    println!(
        "byte-identical replay: OK ({} events, {} fills)",
        events.len(),
        state.fills().len()
    );
    println!("== end trace ==");
}

#[test]
fn higher_costs_never_improve_terminal_equity_in_the_golden_scenario() {
    // The same golden scenario under monotonically higher costs: terminal
    // equity must never improve (acceptance criterion).
    let profiles = [
        CostProfile::krx_etf_default().expect("default"),
        CostProfile::custom("0.0003", "1000", "0", 20, "100000", "0.005").expect("2x"),
        CostProfile::custom("0.003", "5000", "0", 20, "100000", "0.005").expect("10x"),
        CostProfile::custom("0.003", "5000", "0.002", 20, "100000", "0.005").expect("10x+tax"),
    ];
    let mut terminals = Vec::new();
    for (i, profile) in profiles.iter().enumerate() {
        let terminal = run_golden_scenario(profile);
        println!(
            "profile {i}: terminal equity {}",
            terminal.as_decimal_string()
        );
        terminals.push(terminal);
    }
    for w in terminals.windows(2) {
        assert!(
            w[0].amount() >= w[1].amount(),
            "higher costs improved terminal equity: {} < {}",
            w[1].as_decimal_string(),
            w[0].as_decimal_string()
        );
    }
    // Determinism: the same profile twice yields the same terminal equity.
    assert_eq!(
        run_golden_scenario(&profiles[0]),
        run_golden_scenario(&profiles[0]),
        "golden scenario is deterministic"
    );
}
