//! Todo 38 acceptance: the twelve ordered checks of design §6.13.
//!
//! Three properties are proven here that a per-check unit test cannot reach:
//!
//! 1. **Every check can deny, and denies alone.** Each `break_*` below makes
//!    exactly one check fail against an otherwise all-green snapshot. If a
//!    check were unreachable — wired into the enum but never run — its case
//!    would report a different denier and fail.
//! 2. **Order is enforced, not incidental.** For every ordered pair of checks
//!    that can both be broken at once, the EARLIER one is reported. A gate
//!    that ran checks in hash order would pass every individual test and fail
//!    this one.
//! 3. **A decision survives a restart.** A persisted snapshot, round-tripped
//!    through JSON and re-evaluated, produces an identical decision.

use domain::Quantity;
use risk_gateway::decision::{CHECK_ORDER, CheckOutcome};
use risk_gateway::snapshot::{
    Allowlisted, DataFreshness, IntentConflict, KillSwitch, MarketSession, Reconciliation,
    RiskSnapshot, Side, StrategyPromotion,
};
use risk_gateway::testing::{self, FailingStore, RecordingStore, krw};
use risk_gateway::{Check, DenyReason, RiskApproval, checks, evaluate, evaluate_and_record};

/// The all-green baseline, sized so that every value-related break below can
/// be applied independently and in combination.
///
/// Equity and cash are deliberately large (10,000,000) so that breaking the
/// per-order value limit does not incidentally break the weight or funds
/// checks — a break that trips three checks would make the ordering property
/// vacuous rather than strict.
fn base() -> RiskSnapshot {
    let mut s = testing::snapshot_all_green();
    s.account.equity = krw("10000000");
    s.account.available_cash = krw("10000000");
    s.account.available_quantity = Quantity::parse("1000").unwrap();
    s
}

/// Makes exactly `check` fail. Verified by `each_break_denies_exactly_its_own_check`.
fn break_check(mut s: RiskSnapshot, check: Check) -> RiskSnapshot {
    match check {
        Check::KillSwitch => s.kill_switch = KillSwitch::Engaged,
        Check::MarketSession => s.market_session = MarketSession::Closed,
        // 301s against a 300s limit: one second past, not obviously stale.
        Check::DataFreshness => s.data_freshness = DataFreshness::Age(301),
        Check::StrategyPromotion => s.strategy_promotion = StrategyPromotion::NotPromoted,
        Check::Reconciliation => s.reconciliation = Reconciliation::NotGreen,
        Check::InstrumentAllowlist => s.instrument_allowed = Allowlisted::NotAllowed,
        // 50% of equity once this order lands, against a 30% cap.
        Check::SymbolMaxWeight => s.account.position_value = krw("5000000"),
        // 200 @ 7250 = 1,450,000, over the 1,000,000 per-order limit but only
        // 14.5% of equity and well inside the daily and cash limits.
        Check::OrderMaxValue => s.intent.quantity = Quantity::parse("200").unwrap(),
        Check::DailyOrderValue => s.account.daily_order_value = krw("4980000"),
        // Exactly at the limit: trading stops AT the loss limit.
        Check::DailyLoss => s.account.daily_loss = krw("500000"),
        Check::AvailableFunds => s.account.available_cash = krw("1000"),
        Check::DuplicateIntent => s.conflict = IntentConflict::Conflicting,
    }
    s
}

/// A check paired with a mutation that makes its input unknowable.
type UnknownCase = (Check, fn(&mut RiskSnapshot));

/// The reason each check gives when it is the one that denies.
fn expected_reason(check: Check) -> DenyReason {
    match check {
        Check::KillSwitch => DenyReason::LiveKillSwitchEngaged,
        Check::MarketSession => DenyReason::MarketSessionClosed,
        Check::DataFreshness => DenyReason::DataStale,
        Check::StrategyPromotion => DenyReason::StrategyNotLiveCandidate,
        Check::Reconciliation => DenyReason::LiveReconciliationRequired,
        Check::InstrumentAllowlist => DenyReason::InstrumentNotAllowed,
        Check::SymbolMaxWeight
        | Check::OrderMaxValue
        | Check::DailyOrderValue
        | Check::DailyLoss
        | Check::AvailableFunds => DenyReason::RiskLimitExceeded,
        Check::DuplicateIntent => DenyReason::DuplicateIntent,
    }
}

#[test]
fn the_baseline_passes_every_check() {
    let decision = evaluate(&base(), &testing::limits());
    assert!(decision.is_approved(), "{decision:?}");
    assert_eq!(decision.records.len(), 12);
}

#[test]
fn each_break_denies_exactly_its_own_check() {
    // The table test the acceptance criteria names. Each row breaks one input
    // and asserts BOTH that the right check denied and that no earlier check
    // did -- the second half is what keeps the other rows meaningful.
    for check in CHECK_ORDER {
        let snap = break_check(base(), check);
        let decision = evaluate(&snap, &testing::limits());

        assert_eq!(
            decision.denied_by,
            Some(check),
            "breaking {check} should be denied by {check}, got {:?}",
            decision.denied_by
        );
        assert_eq!(decision.reason, Some(expected_reason(check)));

        // Exactly one check ran and denied; every earlier one passed.
        for record in &decision.records {
            match record.check.cmp(&check) {
                std::cmp::Ordering::Less => assert_eq!(
                    record.outcome,
                    CheckOutcome::Passed,
                    "{} should have passed while testing {check}",
                    record.check
                ),
                std::cmp::Ordering::Equal => {
                    assert_eq!(record.outcome, CheckOutcome::Denied(expected_reason(check)))
                }
                std::cmp::Ordering::Greater => assert_eq!(
                    record.outcome,
                    CheckOutcome::NotEvaluated,
                    "{} ran after the short circuit at {check}",
                    record.check
                ),
            }
        }
    }
}

#[test]
fn the_earlier_check_always_wins() {
    // For every ordered pair, break both and require the earlier to be the
    // one reported. The composition is asserted rather than assumed: if
    // applying the second break undid the first, the pair would silently stop
    // testing anything, so both are confirmed to deny in isolation first.
    let limits = testing::limits();
    let mut pairs_tested = 0;

    for (i, earlier) in CHECK_ORDER.iter().enumerate() {
        for later in &CHECK_ORDER[i + 1..] {
            let both = break_check(break_check(base(), *earlier), *later);

            assert!(
                checks::run(*earlier, &both, &limits).is_some(),
                "composing {earlier} with {later} undid the {earlier} break"
            );
            assert!(
                checks::run(*later, &both, &limits).is_some(),
                "composing {earlier} with {later} undid the {later} break"
            );

            let decision = evaluate(&both, &limits);
            assert_eq!(
                decision.denied_by,
                Some(*earlier),
                "{earlier} precedes {later} in §6.13 and must be the reported denier"
            );
            pairs_tested += 1;
        }
    }

    assert_eq!(pairs_tested, 66, "every ordered pair of twelve checks");
}

#[test]
fn every_unknown_input_denies_as_unavailable_not_as_a_policy_rejection() {
    // §16 requires missing state to block. Conflating "we could not tell" with
    // "the answer is no" would hide an outage as a routine rejection, so the
    // reason must be InputUnavailable and the grade CRITICAL.
    let unknowns: [UnknownCase; 8] = [
        (Check::KillSwitch, |s| s.kill_switch = KillSwitch::Unknown),
        (Check::MarketSession, |s| {
            s.market_session = MarketSession::Unknown
        }),
        (Check::DataFreshness, |s| {
            s.data_freshness = DataFreshness::Unknown
        }),
        (Check::StrategyPromotion, |s| {
            s.strategy_promotion = StrategyPromotion::Unknown
        }),
        (Check::Reconciliation, |s| {
            s.reconciliation = Reconciliation::Unknown
        }),
        (Check::InstrumentAllowlist, |s| {
            s.instrument_allowed = Allowlisted::Unknown
        }),
        (Check::DuplicateIntent, |s| {
            s.conflict = IntentConflict::Unknown
        }),
        // A market order has no price, so its value cannot be established.
        (Check::SymbolMaxWeight, |s| s.intent.price = None),
    ];

    for (check, apply) in unknowns {
        let mut snap = base();
        apply(&mut snap);
        let decision = evaluate(&snap, &testing::limits());
        assert_eq!(decision.denied_by, Some(check), "unknown input at {check}");
        assert_eq!(
            decision.reason,
            Some(DenyReason::InputUnavailable),
            "{check} must deny as unavailable, not as a policy rejection"
        );
        assert_eq!(decision.severity(), "CRITICAL");
    }
}

#[tokio::test]
async fn at_08_stale_data_blocks_with_a_reason_a_metric_and_an_audited_record() {
    // AT-08: 오래된 데이터로 Live 주문 시도 → 주문이 차단되고 사유와 감사 로그 생성.
    let store = RecordingStore::default();
    let snap = break_check(base(), Check::DataFreshness);
    let outcome = evaluate_and_record(&snap, &testing::limits(), &store).await;

    let decision = outcome.decision().clone();
    assert_eq!(decision.reason, Some(DenyReason::DataStale));
    assert_eq!(decision.reason.unwrap().as_str(), "DATA_STALE");
    // The metric §15.2 names for exactly this case.
    assert_eq!(decision.metric(), Some("stale_data_blocks"));
    // The audit record exists, carries the snapshot, and no approval was made.
    assert_eq!(store.records().len(), 1);
    let (recorded, recorded_snap) = store.records().into_iter().next().unwrap();
    assert_eq!(recorded.reason, Some(DenyReason::DataStale));
    assert_eq!(recorded_snap, snap);
    assert!(outcome.into_approval().is_none());
}

#[test]
fn freshness_is_inclusive_at_the_limit_and_a_future_timestamp_is_a_fault() {
    let limits = testing::limits(); // 300s
    let with_age = |secs| {
        let mut s = base();
        s.data_freshness = DataFreshness::Age(secs);
        evaluate(&s, &limits)
    };
    // Exactly at the limit is still fresh; one second past is not.
    assert!(with_age(300).is_approved());
    assert_eq!(with_age(301).reason, Some(DenyReason::DataStale));
    // Data stamped in the future is a clock or feed fault, not fresh data.
    assert_eq!(with_age(-1).reason, Some(DenyReason::InputUnavailable));
}

#[test]
fn limit_boundaries_admit_the_limit_and_refuse_a_hairs_breadth_past_it() {
    let limits = testing::limits();

    // Order value exactly 1,000,000 (the limit) is allowed; 1,000,000.0001 is
    // not. A float comparison cannot reliably tell these apart.
    let at_limit = {
        let mut s = base();
        s.intent.quantity = Quantity::parse("100").unwrap();
        s.intent.price = Some(domain::Price::parse("10000").unwrap());
        s
    };
    assert!(evaluate(&at_limit, &limits).is_approved());

    let over = {
        let mut s = at_limit.clone();
        s.intent.price = Some(domain::Price::parse("10000.0001").unwrap());
        s
    };
    assert_eq!(
        evaluate(&over, &limits).denied_by,
        Some(Check::OrderMaxValue)
    );

    // The daily loss limit stops trading AT the limit, not one won past it.
    let mut at_loss = base();
    at_loss.account.daily_loss = krw("500000");
    assert_eq!(
        evaluate(&at_loss, &limits).denied_by,
        Some(Check::DailyLoss)
    );
    let mut under_loss = base();
    under_loss.account.daily_loss = krw("499999.9999");
    assert!(evaluate(&under_loss, &limits).is_approved());
}

#[test]
fn a_sell_is_checked_against_units_held_not_cash() {
    let limits = testing::limits();
    let mut sell = base();
    sell.intent.side = Side::Sell;
    sell.account.available_cash = krw("0");
    sell.account.available_quantity = Quantity::parse("10").unwrap();
    // Selling exactly what is held, with no cash at all, is fine.
    assert!(evaluate(&sell, &limits).is_approved());

    // Selling one more unit than is available is not.
    sell.intent.quantity = Quantity::parse("11").unwrap();
    assert_eq!(
        evaluate(&sell, &limits).denied_by,
        Some(Check::AvailableFunds)
    );
}

#[test]
fn the_weight_check_measures_the_position_this_order_would_create() {
    // A check against the CURRENT position would approve the order that
    // itself creates the breach. 2,900,000 existing + 1,450,000 new = 43.5%
    // of a 10,000,000 account, over the 30% cap, even though the existing
    // position alone (29%) is inside it.
    let mut snap = base();
    snap.account.position_value = krw("2900000");
    snap.intent.quantity = Quantity::parse("200").unwrap();
    let decision = evaluate(&snap, &testing::limits());
    assert_eq!(decision.denied_by, Some(Check::SymbolMaxWeight));

    // The existing position on its own does not breach, confirming the
    // denial above came from including the new order.
    let mut existing_only = base();
    existing_only.account.position_value = krw("2900000");
    assert!(evaluate(&existing_only, &testing::limits()).is_approved());
}

#[test]
fn a_decision_is_reproduced_exactly_after_a_restart() {
    // The approved gatekeeper decision requires gate state to survive a
    // restart and remain blocking. A restart has nothing but the persisted
    // snapshot, so replaying it must give the identical verdict -- including
    // the per-check trail, not merely the same yes/no.
    let limits = testing::limits();
    for check in CHECK_ORDER {
        let snap = break_check(base(), check);
        let before = evaluate(&snap, &limits);

        let json = serde_json::to_string(&snap).expect("snapshot serializes");
        let restored: RiskSnapshot = serde_json::from_str(&json).expect("snapshot deserializes");
        let after = evaluate(&restored, &limits);

        assert_eq!(before, after, "replaying the {check} denial changed it");
    }

    // And the approval case, which is the one where a divergence would matter
    // most: a restart must not turn a denial into an approval.
    let green = base();
    let json = serde_json::to_string(&green).unwrap();
    let restored: RiskSnapshot = serde_json::from_str(&json).unwrap();
    assert_eq!(evaluate(&green, &limits), evaluate(&restored, &limits));
}

/// A submission path, shaped like the one Todo 39 will build.
///
/// It takes the approval BY VALUE. There is no way to call it without one,
/// and no way to call it twice with the same one: `RiskApproval` is not
/// `Clone` and has no public constructor, so the only source is a gate run
/// that both approved and persisted.
fn submit_to_broker(approval: RiskApproval, intent_ref: &str) -> Result<String, &'static str> {
    // A submitter must confirm the approval it holds is for the order it is
    // about to place; the token carries the intent so this is checkable.
    if approval.intent_ref() != intent_ref {
        return Err("approval is for a different intent");
    }
    Ok(format!("submitted:{}", approval.risk_event_id()))
}

#[tokio::test]
async fn no_submission_is_possible_after_a_denial() {
    // "no simulated KIS submission occurs after any denial or failed
    // persistence" -- here there is no approval to pass, so the call cannot
    // be written at all.
    let store = RecordingStore::default();
    let denied = evaluate_and_record(
        &break_check(base(), Check::KillSwitch),
        &testing::limits(),
        &store,
    )
    .await;
    assert!(denied.into_approval().is_none());

    let failed =
        evaluate_and_record(&base(), &testing::limits(), &FailingStore::new("db down")).await;
    assert!(
        failed.into_approval().is_none(),
        "a failed write must not yield a token"
    );
}

#[tokio::test]
async fn an_approval_authorises_exactly_the_intent_it_names() {
    let store = RecordingStore::default();
    let approval = evaluate_and_record(&base(), &testing::limits(), &store)
        .await
        .into_approval()
        .expect("approved");

    // Laundering an approval into a different order is refused.
    assert_eq!(
        submit_to_broker(approval, "some-other-intent"),
        Err("approval is for a different intent")
    );

    // A fresh approval for the right intent submits once. It is moved by the
    // call, so a second submission with it does not compile.
    let store2 = RecordingStore::default();
    let mut snap = base();
    snap.intent.intent_ref = "intent-2".into();
    let approval2 = evaluate_and_record(&snap, &testing::limits(), &store2)
        .await
        .into_approval()
        .expect("approved");
    assert!(submit_to_broker(approval2, "intent-2").is_ok());
}

#[tokio::test]
async fn every_decision_names_its_limits_version_and_correlation_id() {
    // Without these a decision cannot be re-derived once limits change, nor
    // joined to the audit log and the alert it raised.
    let store = RecordingStore::default();
    let outcome = evaluate_and_record(&base(), &testing::limits(), &store).await;
    let decision = outcome.decision();
    assert_eq!(decision.limits_version, "risk-limits-v1");
    assert_eq!(decision.correlation_id, "correlation-1");
    assert_eq!(decision.intent_ref, "intent-1");
    assert_eq!(decision.evaluated_at_secs, 1_800_000_000);
}
