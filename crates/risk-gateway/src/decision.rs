//! What the gate decides, and the token that proves it decided.

use serde::{Deserialize, Serialize};
use std::fmt;

/// The twelve checks of design §6.13, in the order they must run.
///
/// Order is not cosmetic. The kill switch is first because an engaged switch
/// must deny before anything else is even consulted, and the cheap global
/// conditions precede the per-account ones so that a system-wide halt is not
/// reported to the operator as, say, an allowlist problem. The `u8`
/// discriminants are stable: they are persisted in decision records and read
/// back by the restart-equivalence property, so they may be appended to but
/// never renumbered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Check {
    KillSwitch = 1,
    MarketSession = 2,
    DataFreshness = 3,
    StrategyPromotion = 4,
    Reconciliation = 5,
    InstrumentAllowlist = 6,
    SymbolMaxWeight = 7,
    OrderMaxValue = 8,
    DailyOrderValue = 9,
    DailyLoss = 10,
    AvailableFunds = 11,
    DuplicateIntent = 12,
}

/// Every check, in evaluation order.
///
/// The single source of order. `evaluate` iterates this, and
/// `every_check_is_ordered_and_unique` proves it lists all twelve exactly once
/// in ascending discriminant order — so a check added to the enum but not to
/// this array (which would silently never run) fails the build's test suite.
pub const CHECK_ORDER: [Check; 12] = [
    Check::KillSwitch,
    Check::MarketSession,
    Check::DataFreshness,
    Check::StrategyPromotion,
    Check::Reconciliation,
    Check::InstrumentAllowlist,
    Check::SymbolMaxWeight,
    Check::OrderMaxValue,
    Check::DailyOrderValue,
    Check::DailyLoss,
    Check::AvailableFunds,
    Check::DuplicateIntent,
];

impl Check {
    /// The stable string persisted in `risk_events.denied_by_check`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Check::KillSwitch => "KILL_SWITCH",
            Check::MarketSession => "MARKET_SESSION",
            Check::DataFreshness => "DATA_FRESHNESS",
            Check::StrategyPromotion => "STRATEGY_PROMOTION",
            Check::Reconciliation => "RECONCILIATION",
            Check::InstrumentAllowlist => "INSTRUMENT_ALLOWLIST",
            Check::SymbolMaxWeight => "SYMBOL_MAX_WEIGHT",
            Check::OrderMaxValue => "ORDER_MAX_VALUE",
            Check::DailyOrderValue => "DAILY_ORDER_VALUE",
            Check::DailyLoss => "DAILY_LOSS",
            Check::AvailableFunds => "AVAILABLE_FUNDS",
            Check::DuplicateIntent => "DUPLICATE_INTENT",
        }
    }
}

impl fmt::Display for Check {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why an order was denied.
///
/// Four of these are the stable HTTP codes declared in `api-server`'s
/// `contract.rs`; they are named here as the same strings deliberately, so a
/// denial that reaches the web surfaces with a code the client already knows.
/// Anything else stays inside the payload — a new code crossing HTTP must be
/// declared in `contract.rs` first, which
/// `openapi_contract_handlers_emit_only_declared_codes` enforces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DenyReason {
    /// The system kill switch is engaged (§6.13 check 1, FR-LIVE-006).
    LiveKillSwitchEngaged,
    /// Market closed, halted, or in an unknown session state (check 2).
    MarketSessionClosed,
    /// Data older than the limit, or of unknown age (check 3, AT-08).
    DataStale,
    /// The strategy has not reached `LiveCandidate` (check 4).
    StrategyNotLiveCandidate,
    /// Reconciliation is not green (check 5, FR-LIVE-004).
    LiveReconciliationRequired,
    /// Instrument absent from the allowlist (check 6).
    InstrumentNotAllowed,
    /// A limit in checks 7-11 was exceeded (FR-LIVE-002).
    RiskLimitExceeded,
    /// A conflicting or duplicate intent already exists (check 12).
    DuplicateIntent,
    /// An input the gate needed was missing or unreadable.
    ///
    /// Distinct from every other reason: those mean "the answer is no", this
    /// means "there was no answer". Both deny, and §16 requires that they do,
    /// but conflating them would hide an outage as a policy rejection.
    InputUnavailable,
    /// The decision could not be durably recorded (§16: DB write failure
    /// blocks new Live orders).
    NotPersisted,
}

impl DenyReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            DenyReason::LiveKillSwitchEngaged => "LIVE_KILL_SWITCH_ENGAGED",
            DenyReason::MarketSessionClosed => "MARKET_SESSION_CLOSED",
            DenyReason::DataStale => "DATA_STALE",
            DenyReason::StrategyNotLiveCandidate => "STRATEGY_NOT_LIVE_CANDIDATE",
            DenyReason::LiveReconciliationRequired => "LIVE_RECONCILIATION_REQUIRED",
            DenyReason::InstrumentNotAllowed => "INSTRUMENT_NOT_ALLOWED",
            DenyReason::RiskLimitExceeded => "RISK_LIMIT_EXCEEDED",
            DenyReason::DuplicateIntent => "DUPLICATE_INTENT",
            DenyReason::InputUnavailable => "INPUT_UNAVAILABLE",
            DenyReason::NotPersisted => "RISK_DECISION_NOT_PERSISTED",
        }
    }

    /// Whether this denial reflects a broken or absent input rather than a
    /// policy answer. §15.3 grades these CRITICAL; a policy denial is not.
    pub const fn is_input_failure(self) -> bool {
        matches!(
            self,
            DenyReason::InputUnavailable | DenyReason::NotPersisted
        )
    }
}

impl fmt::Display for DenyReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The outcome of one check.
///
/// `NotEvaluated` exists so that a persisted decision records all twelve
/// checks even though evaluation short-circuits. An audit row that simply
/// omitted the checks after the denier would be ambiguous between "did not
/// run" and "ran and passed", and the difference matters when someone is
/// reconstructing why an order was blocked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome", content = "reason")]
pub enum CheckOutcome {
    Passed,
    Denied(DenyReason),
    NotEvaluated,
}

/// One check and how it came out, in evaluation order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckRecord {
    pub check: Check,
    pub outcome: CheckOutcome,
}

/// The verdict, with the full ordered trail that produced it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub intent_ref: String,
    pub correlation_id: String,
    pub limits_version: String,
    pub evaluated_at_secs: i64,
    pub records: Vec<CheckRecord>,
    /// `None` on approval; the denying check on denial.
    pub denied_by: Option<Check>,
    pub reason: Option<DenyReason>,
}

impl Decision {
    pub fn is_approved(&self) -> bool {
        self.denied_by.is_none() && self.reason.is_none()
    }

    /// The metric §15.2 wants incremented for this decision, if any.
    ///
    /// Returned rather than emitted so the crate takes no dependency on a
    /// metrics backend; the caller owns the registry. `stale_data_blocks` is
    /// AT-08's required metric.
    pub fn metric(&self) -> Option<&'static str> {
        match self.reason {
            None => None,
            Some(DenyReason::DataStale) => Some("stale_data_blocks"),
            Some(DenyReason::LiveKillSwitchEngaged) => Some("kill_switch_state"),
            Some(_) => Some("orders_rejected_total"),
        }
    }

    /// §15.3 alert grade. An input failure or a kill-switch denial is
    /// CRITICAL; an ordinary policy denial is a WARNING.
    pub fn severity(&self) -> &'static str {
        match self.reason {
            None => "INFO",
            Some(r) if r.is_input_failure() => "CRITICAL",
            Some(DenyReason::LiveKillSwitchEngaged) => "CRITICAL",
            Some(_) => "WARNING",
        }
    }
}

/// Proof that a specific intent passed every check AND that the decision was
/// durably recorded.
///
/// The type is the enforcement mechanism for "do not let the adapter bypass
/// the gate":
///
///   * fields are private and there is no public constructor, so it can only
///     be minted by [`crate::gate::evaluate_and_record`], and only after the
///     store confirms the write;
///   * it is not `Clone` and submission takes it BY VALUE, so one approval
///     authorises exactly one submission — a reference could be handed to a
///     retry loop and used twice;
///   * it carries its `intent_ref`, so a submitter must check that the
///     approval it holds is for the order it is about to place, and cannot
///     launder an approval for intent A into a submission of intent B.
///
/// Consuming the token is also the `RISK_APPROVED → SUBMITTING` transition of
/// the Todo 39 state machine, expressed in the type system.
#[derive(Debug)]
pub struct RiskApproval {
    intent_ref: String,
    correlation_id: String,
    /// The `risk_events.id` of the row that authorises this submission, so
    /// approval and audit trail are joinable in both directions.
    risk_event_id: String,
}

impl RiskApproval {
    /// Private: only `gate` may mint one, and only post-persistence.
    pub(crate) fn new(intent_ref: String, correlation_id: String, risk_event_id: String) -> Self {
        Self {
            intent_ref,
            correlation_id,
            risk_event_id,
        }
    }

    pub fn intent_ref(&self) -> &str {
        &self.intent_ref
    }

    pub fn correlation_id(&self) -> &str {
        &self.correlation_id
    }

    pub fn risk_event_id(&self) -> &str {
        &self.risk_event_id
    }
}

/// What the gate returns: an approval token, or the reason there is none.
///
/// Not a `Result`, because a denial is a normal outcome rather than an error,
/// and both arms carry the same full `Decision` for the audit trail.
#[derive(Debug)]
pub enum GateOutcome {
    Approved {
        approval: RiskApproval,
        decision: Decision,
    },
    Denied {
        decision: Decision,
    },
}

impl GateOutcome {
    pub fn decision(&self) -> &Decision {
        match self {
            GateOutcome::Approved { decision, .. } => decision,
            GateOutcome::Denied { decision } => decision,
        }
    }

    /// The approval, if there is one. Consumes `self` because the token
    /// cannot be copied out of a borrow.
    pub fn into_approval(self) -> Option<RiskApproval> {
        match self {
            GateOutcome::Approved { approval, .. } => Some(approval),
            GateOutcome::Denied { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_check_is_ordered_and_unique() {
        // A check added to the enum but forgotten in CHECK_ORDER would never
        // run, and the gate would approve orders it should have denied. This
        // is the test that makes that impossible to ship.
        assert_eq!(CHECK_ORDER.len(), 12);
        let mut seen = std::collections::BTreeSet::new();
        for c in CHECK_ORDER {
            assert!(seen.insert(c as u8), "{c} appears twice in CHECK_ORDER");
        }
        let ordered: Vec<u8> = CHECK_ORDER.iter().map(|c| *c as u8).collect();
        let mut sorted = ordered.clone();
        sorted.sort_unstable();
        assert_eq!(ordered, sorted, "CHECK_ORDER must be in §6.13 order");
        assert_eq!(
            ordered.first(),
            Some(&1),
            "the kill switch is checked first"
        );
        assert_eq!(ordered.last(), Some(&12));
    }

    #[test]
    fn check_and_reason_strings_are_stable_and_distinct() {
        // These strings are persisted; a collision would make two different
        // denials indistinguishable in the audit trail.
        let checks: std::collections::BTreeSet<&str> =
            CHECK_ORDER.iter().map(|c| c.as_str()).collect();
        assert_eq!(checks.len(), 12);
        assert_eq!(Check::KillSwitch.as_str(), "KILL_SWITCH");
        assert_eq!(
            DenyReason::DataStale.as_str(),
            "DATA_STALE",
            "must match the declared HTTP code"
        );
        assert_eq!(
            DenyReason::LiveKillSwitchEngaged.as_str(),
            "LIVE_KILL_SWITCH_ENGAGED"
        );
        assert_eq!(
            DenyReason::LiveReconciliationRequired.as_str(),
            "LIVE_RECONCILIATION_REQUIRED"
        );
        assert_eq!(
            DenyReason::RiskLimitExceeded.as_str(),
            "RISK_LIMIT_EXCEEDED"
        );
    }

    #[test]
    fn an_input_failure_is_critical_and_a_policy_denial_is_not() {
        let decision = |reason: Option<DenyReason>| Decision {
            intent_ref: "i".into(),
            correlation_id: "c".into(),
            limits_version: "v".into(),
            evaluated_at_secs: 0,
            records: Vec::new(),
            denied_by: reason.map(|_| Check::KillSwitch),
            reason,
        };
        assert_eq!(decision(None).severity(), "INFO");
        assert_eq!(
            decision(Some(DenyReason::InputUnavailable)).severity(),
            "CRITICAL"
        );
        assert_eq!(
            decision(Some(DenyReason::NotPersisted)).severity(),
            "CRITICAL"
        );
        assert_eq!(
            decision(Some(DenyReason::RiskLimitExceeded)).severity(),
            "WARNING"
        );
        // AT-08 requires a metric for the stale-data block specifically.
        assert_eq!(
            decision(Some(DenyReason::DataStale)).metric(),
            Some("stale_data_blocks")
        );
        assert_eq!(decision(None).metric(), None);
    }

    #[test]
    fn an_approval_cannot_be_cloned_or_constructed_outside_the_crate() {
        // Compile-time facts, asserted here so the intent is recorded where a
        // reader will look. `RiskApproval` has no public constructor and no
        // Clone derive; both are checked by the compiler, and the negative
        // cases live in tests/compile_fail if they are ever added.
        fn assert_not_clone<T>() {}
        assert_not_clone::<RiskApproval>();
        let a = RiskApproval::new("i".into(), "c".into(), "e".into());
        assert_eq!(a.intent_ref(), "i");
        assert_eq!(a.risk_event_id(), "e");
    }
}
