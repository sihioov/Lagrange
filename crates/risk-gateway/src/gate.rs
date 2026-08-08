//! Ordered evaluation, then persistence, then — only then — an approval.

use crate::checks;
use crate::decision::{
    CHECK_ORDER, CheckOutcome, CheckRecord, Decision, DenyReason, GateOutcome, RiskApproval,
};
use crate::limits::RiskLimits;
use crate::snapshot::RiskSnapshot;
use crate::store::RiskEventStore;

/// Evaluates the twelve checks in order, short-circuiting at the first denial.
///
/// Pure, and separate from persistence so that a persisted snapshot can be
/// replayed through it and compared. Checks after the denier are recorded as
/// `NotEvaluated` rather than omitted: an audit row must distinguish "did not
/// run" from "ran and passed".
pub fn evaluate(snapshot: &RiskSnapshot, limits: &RiskLimits) -> Decision {
    let mut records = Vec::with_capacity(CHECK_ORDER.len());
    let mut denied_by = None;
    let mut reason = None;

    for check in CHECK_ORDER {
        if denied_by.is_some() {
            records.push(CheckRecord {
                check,
                outcome: CheckOutcome::NotEvaluated,
            });
            continue;
        }
        match checks::run(check, snapshot, limits) {
            None => records.push(CheckRecord {
                check,
                outcome: CheckOutcome::Passed,
            }),
            Some(r) => {
                records.push(CheckRecord {
                    check,
                    outcome: CheckOutcome::Denied(r),
                });
                denied_by = Some(check);
                reason = Some(r);
            }
        }
    }

    Decision {
        intent_ref: snapshot.intent.intent_ref.clone(),
        correlation_id: snapshot.correlation_id.clone(),
        limits_version: limits.version.clone(),
        evaluated_at_secs: snapshot.evaluated_at_secs,
        records,
        denied_by,
        reason,
    }
}

/// Evaluates, records, and mints an approval only if both succeeded.
///
/// The ordering is the entire safety property, so it is worth being explicit
/// about why it is this way round:
///
/// * The decision is recorded **before** the approval exists. A crash between
///   the two loses the approval, never the record — so the worst case is an
///   order that was authorised and not placed, rather than one that was placed
///   with no evidence of why.
/// * A failed write **denies** (§16: "DB 쓰기 실패 → 신규 Live 주문 차단").
///   The returned decision carries `NotPersisted`, which is graded CRITICAL,
///   because an unrecordable decision means the audit trail is broken and that
///   is an incident rather than a policy outcome.
/// * A denial is recorded too. A denied order is exactly the thing someone
///   will later need to explain.
pub async fn evaluate_and_record<S: RiskEventStore>(
    snapshot: &RiskSnapshot,
    limits: &RiskLimits,
    store: &S,
) -> GateOutcome {
    let decision = evaluate(snapshot, limits);

    match store.record(&decision, snapshot).await {
        Ok(risk_event_id) => {
            if decision.is_approved() {
                let approval = RiskApproval::new(
                    decision.intent_ref.clone(),
                    decision.correlation_id.clone(),
                    risk_event_id,
                );
                GateOutcome::Approved { approval, decision }
            } else {
                GateOutcome::Denied { decision }
            }
        }
        Err(_) => {
            // The evaluation may have said yes; it does not matter. Without a
            // durable record there is nothing to reconcile against after a
            // restart, so the order does not go out.
            let mut decision = decision;

            // `denied_by` stays as evaluation left it: the denying check if a
            // check denied, and NONE if the checks all passed and only the
            // write failed. Naming a check here — the earlier version blamed
            // the last one so that "every denial names a check" — would tell
            // an operator reading the log during a database outage that the
            // order was denied by DUPLICATE_INTENT. The consistency it bought
            // was worthless: a `NotPersisted` decision is by definition never
            // a row, so 0018's CHECK constraint never sees it, while the log
            // that does see it is read at exactly the wrong moment.
            //
            // When a check HAD already denied, `reason` is overwritten while
            // `records` keep the original denial. That is deliberate: both
            // facts are true, and unrecordability is the dominant one because
            // it is the one that makes the audit trail unreliable. The trail
            // in `records` still says which check objected.
            decision.reason = Some(DenyReason::NotPersisted);
            GateOutcome::Denied { decision }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::Check;
    use crate::store::StoreError;
    use crate::testing::{self, FailingStore, RecordingStore};

    #[tokio::test]
    async fn an_all_green_snapshot_is_approved_and_records_twelve_passes() {
        let store = RecordingStore::default();
        let outcome =
            evaluate_and_record(&testing::snapshot_all_green(), &testing::limits(), &store).await;
        let decision = outcome.decision().clone();
        assert!(decision.is_approved(), "{decision:?}");
        assert_eq!(decision.records.len(), 12);
        assert!(
            decision
                .records
                .iter()
                .all(|r| r.outcome == CheckOutcome::Passed)
        );
        assert_eq!(
            store.records().len(),
            1,
            "exactly one risk_event per intent"
        );
        let approval = outcome.into_approval().expect("approved");
        assert_eq!(approval.intent_ref(), "intent-1");
        assert!(!approval.risk_event_id().is_empty());
    }

    #[test]
    fn evaluation_short_circuits_and_marks_the_rest_not_evaluated() {
        // Two checks fail; only the FIRST may be reported, and the later one
        // must not even be consulted -- otherwise the operator is told about
        // an allowlist problem when the real story is that trading is halted.
        let mut snap = testing::snapshot_all_green();
        snap.market_session = crate::snapshot::MarketSession::Closed;
        snap.instrument_allowed = crate::snapshot::Allowlisted::NotAllowed;

        let decision = evaluate(&snap, &testing::limits());
        assert_eq!(decision.denied_by, Some(Check::MarketSession));
        assert_eq!(decision.reason, Some(DenyReason::MarketSessionClosed));

        assert_eq!(decision.records[0].outcome, CheckOutcome::Passed);
        assert_eq!(
            decision.records[1].outcome,
            CheckOutcome::Denied(DenyReason::MarketSessionClosed)
        );
        // Everything after the denier, including the allowlist check that
        // would also have failed, is explicitly not evaluated.
        for record in &decision.records[2..] {
            assert_eq!(
                record.outcome,
                CheckOutcome::NotEvaluated,
                "{} ran after the short circuit",
                record.check
            );
        }
    }

    #[tokio::test]
    async fn a_failed_write_denies_an_otherwise_approved_order() {
        // §16: a DB write failure blocks new Live orders. The evaluation here
        // is all-green, so the ONLY thing standing between this order and the
        // broker is the persistence requirement.
        let store = FailingStore::new("disk full");
        let outcome =
            evaluate_and_record(&testing::snapshot_all_green(), &testing::limits(), &store).await;
        let decision = outcome.decision().clone();
        assert!(!decision.is_approved());
        assert_eq!(decision.reason, Some(DenyReason::NotPersisted));
        assert_eq!(decision.severity(), "CRITICAL");
        assert!(
            outcome.into_approval().is_none(),
            "no approval may exist without a durable record"
        );
    }

    #[tokio::test]
    async fn a_write_failure_blames_the_write_and_not_a_check() {
        // The log written during a database outage is read by an operator
        // trying to find out what is wrong. If an all-green evaluation whose
        // write failed reported `denied_by: DUPLICATE_INTENT`, it would send
        // that reader looking for a duplicate order that does not exist.
        let outcome = evaluate_and_record(
            &testing::snapshot_all_green(),
            &testing::limits(),
            &FailingStore::new("connection reset"),
        )
        .await;
        let decision = outcome.decision();
        assert_eq!(decision.reason, Some(DenyReason::NotPersisted));
        assert_eq!(
            decision.denied_by, None,
            "no check denied this order; the write did"
        );
        assert!(
            decision
                .records
                .iter()
                .all(|r| r.outcome == CheckOutcome::Passed),
            "the trail must still show that every check passed"
        );
    }

    #[tokio::test]
    async fn a_write_failure_after_a_real_denial_keeps_both_facts() {
        // Unrecordability is the dominant reason, because it is the one that
        // makes the audit trail unreliable -- but the trail must still say
        // which check objected.
        let mut snap = testing::snapshot_all_green();
        snap.market_session = crate::snapshot::MarketSession::Closed;
        let outcome =
            evaluate_and_record(&snap, &testing::limits(), &FailingStore::new("db down")).await;
        let decision = outcome.decision();
        assert_eq!(decision.reason, Some(DenyReason::NotPersisted));
        assert_eq!(decision.denied_by, Some(Check::MarketSession));
        assert_eq!(
            decision.records[1].outcome,
            CheckOutcome::Denied(DenyReason::MarketSessionClosed)
        );
    }

    #[tokio::test]
    async fn a_denial_is_recorded_too() {
        // The rows someone will need most are the denials.
        let mut snap = testing::snapshot_all_green();
        snap.kill_switch = crate::snapshot::KillSwitch::Engaged;
        let store = RecordingStore::default();
        let outcome = evaluate_and_record(&snap, &testing::limits(), &store).await;
        assert!(outcome.into_approval().is_none());
        assert_eq!(store.records().len(), 1);
        let (decision, _) = store.records().into_iter().next().unwrap();
        assert_eq!(decision.reason, Some(DenyReason::LiveKillSwitchEngaged));
        assert_eq!(decision.denied_by, Some(Check::KillSwitch));
    }

    #[tokio::test]
    async fn a_second_decision_for_the_same_intent_is_refused_by_the_store() {
        // The DB's unique index is modelled here: re-deciding an intent must
        // fail rather than produce a second, possibly contradictory approval.
        let store = RecordingStore::default();
        let snap = testing::snapshot_all_green();
        let first = evaluate_and_record(&snap, &testing::limits(), &store).await;
        assert!(first.into_approval().is_some());

        let second = evaluate_and_record(&snap, &testing::limits(), &store).await;
        assert!(
            second.into_approval().is_none(),
            "an intent may be approved once"
        );
        assert_eq!(store.records().len(), 1);
    }

    #[tokio::test]
    async fn the_store_sees_the_snapshot_the_decision_was_made_from() {
        // Without the snapshot, the decision cannot be re-derived later, and
        // the restart property has nothing to replay.
        let store = RecordingStore::default();
        let snap = testing::snapshot_all_green();
        let _ = evaluate_and_record(&snap, &testing::limits(), &store).await;
        let (_, recorded) = store.records().into_iter().next().unwrap();
        assert_eq!(recorded, snap);
    }

    #[test]
    fn store_errors_are_reported_without_leaking_into_the_approval_path() {
        let e = StoreError::new("pool timed out");
        assert!(e.to_string().contains("pool timed out"));
    }
}
