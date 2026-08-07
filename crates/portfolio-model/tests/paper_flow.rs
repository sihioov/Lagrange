//! Todo 31: the deterministic Paper session flow (design §9.2 processing
//! order, §10.2; requirements UC-04, FR-PAPER-002/003, AT-07).
//!
//! The documented order is structural, not incidental:
//!   `DailyBarClosedEvent(T)` -> `PendingTarget(effective_date = T+1)`
//!   -> `SessionOpenEvent(T+1)` -> fills -> `DailyBarClosedEvent(T+1)`
//! ("이 설계는 시가 시점에 당일 고가·저가·종가를 참조하는 오류를 구조적으로
//! 방지한다"). This suite proves the flow core enforces exactly that, that
//! re-planning after a crash is byte-identical, that a replayed session is
//! REJECTED rather than double-filled, and that two accounts running the
//! same strategy on the same date never share ids or state (AT-07).

use std::collections::BTreeMap;

use domain::{Currency, InstrumentId, Money, Price, TradingDate};
use uuid::Uuid;

use portfolio_model::cost::CostProfile;
use portfolio_model::error::PortfolioError;
use portfolio_model::ledger::{LedgerEvent, LedgerState};
use portfolio_model::paper_flow::{PendingTarget, close_valuation_event, plan_session_open};
use portfolio_model::sizing::TargetAllocation;
use portfolio_model::sizing::weight_from_ratio;

fn krw(amount: &str) -> Money {
    Money::parse(amount, Currency::KRW).expect("valid KRW money")
}

fn price(amount: &str) -> Price {
    Price::parse(amount).expect("valid price")
}

fn instrument(symbol: &str) -> InstrumentId {
    InstrumentId::parse(symbol).expect("valid instrument")
}

fn date(iso: &str) -> TradingDate {
    TradingDate::parse(iso).expect("valid trading date")
}

fn profile() -> CostProfile {
    CostProfile::krx_etf_default().expect("default profile builds")
}

/// A funded, empty account (Todo 30's opening state).
fn opening_state() -> LedgerState {
    LedgerState::new(krw("10000000"), profile())
}

fn opens() -> BTreeMap<InstrumentId, Price> {
    BTreeMap::from([
        (instrument("069500.KRX"), price("10000")),
        (instrument("229200.KRX"), price("20000")),
    ])
}

fn closes() -> BTreeMap<InstrumentId, Price> {
    BTreeMap::from([
        (instrument("069500.KRX"), price("10100")),
        (instrument("229200.KRX"), price("19800")),
    ])
}

fn lots() -> BTreeMap<InstrumentId, u64> {
    BTreeMap::new()
}

fn target_for(account: Uuid, effective: &str) -> PendingTarget {
    PendingTarget {
        account_id: account,
        effective_date: date(effective),
        targets: vec![
            TargetAllocation {
                instrument_id: instrument("069500.KRX"),
                weight: weight_from_ratio(0.6).unwrap(),
            },
            TargetAllocation {
                instrument_id: instrument("229200.KRX"),
                weight: weight_from_ratio(0.4).unwrap(),
            },
        ],
    }
}

fn apply_all(state: &mut LedgerState, events: &[LedgerEvent]) -> Result<(), PortfolioError> {
    for event in events {
        state.apply(event.clone())?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// close(T) -> pending(T+1) -> open(T+1) -> close(T+1)
// ---------------------------------------------------------------------------

#[test]
fn a_target_only_executes_on_its_own_effective_session() {
    let state = opening_state();
    let target = target_for(Uuid::from_u128(1), "2026-01-06");

    // Executing the T+1 target at T's session (the same-day-close error the
    // design structurally prevents) is a typed rejection.
    let err = plan_session_open(&state, &target, &date("2026-01-05"), &opens(), &lots())
        .expect_err("a target must never execute before its effective session");
    assert!(matches!(err, PortfolioError::TargetNotEffective { .. }));

    // A later session is equally wrong: the target is stale, not "close
    // enough" -- executing it would fill at the wrong session's prices.
    let err = plan_session_open(&state, &target, &date("2026-01-07"), &opens(), &lots())
        .expect_err("a stale target must never execute at a later session");
    assert!(matches!(err, PortfolioError::TargetNotEffective { .. }));

    plan_session_open(&state, &target, &date("2026-01-06"), &opens(), &lots())
        .expect("the target executes at exactly its effective session");
}

#[test]
fn one_session_produces_sells_before_buys_and_a_close_valuation() {
    let mut state = opening_state();
    let target = target_for(Uuid::from_u128(1), "2026-01-06");
    let plan = plan_session_open(&state, &target, &date("2026-01-06"), &opens(), &lots()).unwrap();

    // Every order is placed then filled, and no buy is sequenced before a
    // sell (the sells fund the buys; the ledger's cash guard depends on it).
    let mut seen_buy = false;
    for event in &plan.events {
        if let LedgerEvent::OrderPlaced { side, .. } = event {
            match side {
                portfolio_model::Side::Buy => seen_buy = true,
                portfolio_model::Side::Sell => {
                    assert!(!seen_buy, "a sell must never be sequenced after a buy");
                }
            }
        }
    }

    apply_all(&mut state, &plan.events).expect("the whole session applies cleanly");
    assert!(
        !state.positions.is_empty(),
        "the session opened real positions"
    );
    assert!(
        state.cash.amount().bits() >= 0,
        "cash is never negative after a session"
    );

    // The close valuation is a separate, later event -- prices at close are
    // never available to the open.
    let valuation =
        close_valuation_event(&state, date("2026-01-06"), &closes()).expect("close valuation");
    state.apply(valuation).expect("valuation applies");
    assert_eq!(
        state.equity_curve.len(),
        1,
        "the session's close valuation is recorded exactly once"
    );
}

// ---------------------------------------------------------------------------
// Restart / replay determinism
// ---------------------------------------------------------------------------

#[test]
fn replanning_the_same_session_is_byte_identical() {
    let state = opening_state();
    let target = target_for(Uuid::from_u128(1), "2026-01-06");

    // The scheduler dies before applying anything and re-plans from the
    // unchanged persisted state: the SAME events, ids and sequence.
    let first = plan_session_open(&state, &target, &date("2026-01-06"), &opens(), &lots()).unwrap();
    let second =
        plan_session_open(&state, &target, &date("2026-01-06"), &opens(), &lots()).unwrap();
    assert_eq!(
        first.events, second.events,
        "re-planning an unapplied session must be identical, ids included"
    );
}

#[test]
fn replaying_an_applied_session_is_rejected_never_double_filled() {
    let mut state = opening_state();
    let target = target_for(Uuid::from_u128(1), "2026-01-06");
    let plan = plan_session_open(&state, &target, &date("2026-01-06"), &opens(), &lots()).unwrap();
    apply_all(&mut state, &plan.events).expect("first application succeeds");

    let fills_after_first = state.fills.len();
    let cash_after_first = state.cash;

    // A duplicate SessionOpenEvent replay must be refused by the ledger,
    // not silently absorbed -- and must leave the state untouched.
    let err = apply_all(&mut state, &plan.events)
        .expect_err("replaying an applied session must be rejected");
    assert!(
        matches!(
            err,
            PortfolioError::OutOfOrderEvent { .. } | PortfolioError::DuplicateOrder { .. }
        ),
        "unexpected replay error: {err}"
    );
    assert_eq!(state.fills.len(), fills_after_first, "zero duplicate fills");
    assert_eq!(
        state.cash, cash_after_first,
        "cash is untouched by the replay"
    );
}

#[test]
fn ledger_hashes_are_stable_across_a_full_replay() {
    let target = target_for(Uuid::from_u128(1), "2026-01-06");

    let mut a = opening_state();
    let plan_a = plan_session_open(&a, &target, &date("2026-01-06"), &opens(), &lots()).unwrap();
    apply_all(&mut a, &plan_a.events).unwrap();
    a.apply(close_valuation_event(&a.clone(), date("2026-01-06"), &closes()).unwrap())
        .unwrap();

    let mut b = opening_state();
    let plan_b = plan_session_open(&b, &target, &date("2026-01-06"), &opens(), &lots()).unwrap();
    apply_all(&mut b, &plan_b.events).unwrap();
    b.apply(close_valuation_event(&b.clone(), date("2026-01-06"), &closes()).unwrap())
        .unwrap();

    assert_eq!(
        a.canonical_bytes().unwrap(),
        b.canonical_bytes().unwrap(),
        "the same session replayed from scratch is byte-identical"
    );
}

#[test]
fn a_crash_between_sells_and_buys_resumes_without_double_filling() {
    let mut state = opening_state();
    // Seed a position so the rebalance has something to sell.
    let seed = target_for(Uuid::from_u128(1), "2026-01-06");
    let seed_plan =
        plan_session_open(&state, &seed, &date("2026-01-06"), &opens(), &lots()).unwrap();
    apply_all(&mut state, &seed_plan.events).unwrap();

    // A new target that forces both a sell and a buy.
    let rebalance = PendingTarget {
        account_id: Uuid::from_u128(1),
        effective_date: date("2026-01-07"),
        targets: vec![
            TargetAllocation {
                instrument_id: instrument("069500.KRX"),
                weight: weight_from_ratio(0.2).unwrap(),
            },
            TargetAllocation {
                instrument_id: instrument("229200.KRX"),
                weight: weight_from_ratio(0.8).unwrap(),
            },
        ],
    };
    let plan =
        plan_session_open(&state, &rebalance, &date("2026-01-07"), &opens(), &lots()).unwrap();
    assert!(
        plan.events.len() >= 4,
        "the rebalance produces at least one sell and one buy (placed + filled)"
    );

    // The runner crashes after applying only the first order's events.
    let split = 2;
    apply_all(&mut state, &plan.events[..split]).unwrap();
    let fills_before_resume = state.fills.len();

    // On restart it re-plans from the PERSISTED state; the already-applied
    // prefix is filtered by id, and only the remainder is applied.
    let resumed =
        plan_session_open(&state, &rebalance, &date("2026-01-07"), &opens(), &lots()).unwrap();
    let remaining: Vec<LedgerEvent> = resumed
        .events
        .into_iter()
        .filter(|e| !state.already_applied(e))
        .collect();
    apply_all(&mut state, &remaining).expect("the remainder applies cleanly");

    assert!(
        state.fills.len() > fills_before_resume,
        "the resumed half actually executed"
    );
    let mut fill_ids: Vec<String> = state.fills.iter().map(|f| f.fill_id.to_string()).collect();
    let total = fill_ids.len();
    fill_ids.sort();
    fill_ids.dedup();
    assert_eq!(fill_ids.len(), total, "zero duplicate fills after recovery");
    assert!(state.cash.amount().bits() >= 0, "cash never went negative");
}

// ---------------------------------------------------------------------------
// AT-07: two accounts, same strategy, same date, fully independent.
// ---------------------------------------------------------------------------

#[test]
fn two_accounts_running_the_same_target_never_share_ids_or_state() {
    let a_id = Uuid::from_u128(1);
    let b_id = Uuid::from_u128(2);
    let target_a = target_for(a_id, "2026-01-06");
    let target_b = target_for(b_id, "2026-01-06");

    let mut a = opening_state();
    let mut b = LedgerState::new(krw("50000000"), profile());

    let plan_a = plan_session_open(&a, &target_a, &date("2026-01-06"), &opens(), &lots()).unwrap();
    let plan_b = plan_session_open(&b, &target_b, &date("2026-01-06"), &opens(), &lots()).unwrap();

    // Same strategy, same date -- but every id is account-scoped.
    let ids_a: Vec<String> = plan_a
        .events
        .iter()
        .filter_map(|e| match e {
            LedgerEvent::OrderPlaced { order_id, .. } => Some(order_id.to_string()),
            _ => None,
        })
        .collect();
    let ids_b: Vec<String> = plan_b
        .events
        .iter()
        .filter_map(|e| match e {
            LedgerEvent::OrderPlaced { order_id, .. } => Some(order_id.to_string()),
            _ => None,
        })
        .collect();
    assert!(!ids_a.is_empty());
    for id in &ids_a {
        assert!(
            !ids_b.contains(id),
            "two accounts must never mint the same order id"
        );
    }

    apply_all(&mut a, &plan_a.events).unwrap();
    apply_all(&mut b, &plan_b.events).unwrap();
    assert_ne!(a.cash, b.cash, "the larger account holds different cash");
    assert_ne!(
        a.canonical_bytes().unwrap(),
        b.canonical_bytes().unwrap(),
        "accounts never share state"
    );
}

// ---------------------------------------------------------------------------
// Missing data fails closed (never a partial session).
// ---------------------------------------------------------------------------

#[test]
fn a_missing_open_price_fails_the_whole_session_closed() {
    let state = opening_state();
    let target = target_for(Uuid::from_u128(1), "2026-01-06");
    let partial = BTreeMap::from([(instrument("069500.KRX"), price("10000"))]);

    let err = plan_session_open(&state, &target, &date("2026-01-06"), &partial, &lots())
        .expect_err("an omitted symbol must fail the session, not partially execute it");
    assert!(matches!(err, PortfolioError::MissingPrice { .. }));
}

#[test]
fn a_missing_close_price_fails_the_valuation_closed() {
    let mut state = opening_state();
    let target = target_for(Uuid::from_u128(1), "2026-01-06");
    let plan = plan_session_open(&state, &target, &date("2026-01-06"), &opens(), &lots()).unwrap();
    apply_all(&mut state, &plan.events).unwrap();

    let partial = BTreeMap::from([(instrument("069500.KRX"), price("10100"))]);
    let err = close_valuation_event(&state, date("2026-01-06"), &partial)
        .expect_err("a held position without a close price must fail the valuation");
    assert!(matches!(err, PortfolioError::MissingMark { .. }));
}
