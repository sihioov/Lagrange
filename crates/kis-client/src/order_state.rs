//! The Live order state machine (plan Todo 39).
//!
//! Design §6.12 and §16; requirements FR-LIVE-003 and AT-09. The documented
//! path is
//!
//! ```text
//! INTENT_CREATED → RISK_APPROVED → SUBMITTING → SUBMITTED
//!                → ACCEPTED | REJECTED | UNKNOWN
//!                → PARTIALLY_FILLED → FILLED | CANCELED | EXPIRED
//! ```
//!
//! # The one rule the rest exists to protect
//!
//! **`Unknown` is not `Rejected`.** A rejection proves no order exists at the
//! broker; a timeout proves nothing at all. Resubmitting on a rejection is
//! safe, resubmitting on a timeout places a second real order against an
//! account that may already hold the first. So `Unknown` has no edge back to
//! `Submitting` — not a check, an absent transition — and is left only by
//! [`Event::BrokerLookupResolved`], which a caller can only produce by having
//! actually asked the broker.
//!
//! # Why the machine is here and pure
//!
//! [`OrderIntentState::apply`] is a total function of `(state, event)`. It
//! reads no clock, no database and no network, so the persisted event log
//! replays to the same state — the same property Todo 38 relies on, and what
//! makes "restart mid-submit" testable rather than merely argued.
//!
//! # Identity
//!
//! One `intent_ref` names one intent everywhere: `order_intents.intent_ref`,
//! `risk_events.intent_ref` (migration 0018), `orders.order_ref` (0007), and
//! the broker idempotency key are the same string. It is SERVER-generated and
//! globally unique, never composed from account plus client input — 0018's
//! unique index is global, so two accounts choosing the same ref would leave
//! the second unable to record a gate decision at all.

use crate::idempotency::IntentState;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Where a Live order intent has got to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE", tag = "state")]
pub enum OrderIntentState {
    /// Recorded, not yet gated.
    IntentCreated,
    /// The Risk Gateway approved it and the decision is durable.
    RiskApproved,
    /// The submission is in flight. The broker may or may not have it.
    Submitting,
    /// The request reached the broker; the outcome is not yet known.
    Submitted,
    /// The broker acknowledged, and gave the order number that identifies it
    /// from here on.
    Accepted { broker_order_no: String },
    /// The broker refused. Terminal, and safe: no order exists.
    ///
    /// Terminal even though `idempotency::IntentState::allows_submission`
    /// permits resubmission of a rejected key. In the Live path a resubmission
    /// could never be authorised anyway — 0018 allows exactly one gate
    /// decision per intent — so a retry is a NEW intent with a new ref and a
    /// new gate run. See [`OrderIntentState::as_broker_intent_state`].
    Rejected { reason: String },
    /// A mutation timed out. NOT a failure and NOT resubmittable; resolvable
    /// only by asking the broker (§16 "주문 조회로 해소 전 재제출 금지").
    Unknown,
    /// Some quantity has filled; the order is still working.
    ///
    /// Carries the broker's CUMULATIVE filled quantity, without which the
    /// machine cannot tell a re-sent report from a new one — and that
    /// distinction is what keeps a reconnect from moving the ledger twice.
    PartiallyFilled {
        broker_order_no: String,
        cumulative_filled: u64,
    },
    /// Fully filled. Terminal.
    Filled { broker_order_no: String },
    /// Canceled at the broker. Terminal.
    Canceled { broker_order_no: String },
    /// Expired at the broker (session end). Terminal.
    Expired { broker_order_no: String },
    /// The Risk Gateway denied it. Terminal.
    ///
    /// Not in the plan's diagram, which describes the path an APPROVED order
    /// takes. It is modelled explicitly so a denied intent is distinguishable
    /// from one that has not been gated yet — leaving both as
    /// `IntentCreated` would make "never gated" and "gated and refused" the
    /// same row, and only one of those may be retried by re-gating.
    Denied { reason: String },
}

/// Something that happened to an intent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE", tag = "event")]
pub enum Event {
    /// The gate approved and durably recorded its decision.
    RiskApproved,
    /// The gate denied.
    RiskDenied {
        reason: String,
    },
    /// The idempotency key is claimed and the request is about to go out.
    SubmissionStarted,
    /// The request reached the broker.
    SubmissionSent,
    BrokerAccepted {
        broker_order_no: String,
    },
    BrokerRejected {
        reason: String,
    },
    /// A mutation timed out; the outcome is genuinely unknown.
    SubmissionTimedOut,
    /// A fill report. `cumulative_filled` is the broker's running total for
    /// the order, NOT this report's increment — see [`Self::fill`].
    Fill {
        broker_order_no: String,
        cumulative_filled: u64,
        total_quantity: u64,
    },
    BrokerCanceled {
        broker_order_no: String,
    },
    BrokerExpired {
        broker_order_no: String,
    },
    /// The answer to a broker status lookup, which is the ONLY way out of
    /// `Unknown`. Carrying the resolved state means a caller cannot leave
    /// `Unknown` without having something concrete to move to.
    BrokerLookupResolved {
        resolved: Box<OrderIntentState>,
    },
}

impl Event {
    /// A fill report keyed by the broker's cumulative total.
    ///
    /// Cumulative rather than incremental on purpose: broker fill reports
    /// arrive out of order and are re-sent after a reconnect, and adding
    /// increments would double-count both. With a cumulative total, a
    /// duplicate or stale report is recognisably not-newer and is ignored.
    pub fn fill(broker_order_no: impl Into<String>, cumulative_filled: u64, total: u64) -> Self {
        Event::Fill {
            broker_order_no: broker_order_no.into(),
            cumulative_filled,
            total_quantity: total,
        }
    }
}

/// Why a transition was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionError {
    /// The event has no meaning in this state.
    Illegal {
        from: &'static str,
        event: &'static str,
    },
    /// The state is terminal; nothing follows it.
    Terminal { state: &'static str },
    /// An event arrived for a different broker order than the one this intent
    /// is bound to. Never applied: it belongs to someone else's order.
    BrokerOrderMismatch { expected: String, got: String },
    /// A fill claiming more than the order's total quantity.
    OverFill { cumulative: u64, total: u64 },
    /// `Unknown` may only be left for a concrete resolved state, and never
    /// back into the submission path.
    IllegalResolution { resolved: &'static str },
}

impl fmt::Display for TransitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransitionError::Illegal { from, event } => {
                write!(f, "event {event} is not legal in state {from}")
            }
            TransitionError::Terminal { state } => write!(f, "state {state} is terminal"),
            TransitionError::BrokerOrderMismatch { expected, got } => write!(
                f,
                "event names broker order {got}, but this intent is bound to {expected}"
            ),
            TransitionError::OverFill { cumulative, total } => {
                write!(
                    f,
                    "cumulative fill {cumulative} exceeds order quantity {total}"
                )
            }
            TransitionError::IllegalResolution { resolved } => {
                write!(f, "UNKNOWN cannot resolve to {resolved}")
            }
        }
    }
}

impl std::error::Error for TransitionError {}

/// What applying an event did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applied {
    /// The state moved.
    Moved(OrderIntentState),
    /// The event was valid but carried nothing new — a duplicate or
    /// out-of-order fill report. Explicitly NOT an error: brokers re-send, and
    /// treating a re-send as a fault would turn a reconnect into an incident.
    /// The ledger must not be touched for one of these.
    NoChange,
}

impl OrderIntentState {
    pub const fn name(&self) -> &'static str {
        match self {
            Self::IntentCreated => "INTENT_CREATED",
            Self::RiskApproved => "RISK_APPROVED",
            Self::Submitting => "SUBMITTING",
            Self::Submitted => "SUBMITTED",
            Self::Accepted { .. } => "ACCEPTED",
            Self::Rejected { .. } => "REJECTED",
            Self::Unknown => "UNKNOWN",
            Self::PartiallyFilled { .. } => "PARTIALLY_FILLED",
            Self::Filled { .. } => "FILLED",
            Self::Canceled { .. } => "CANCELED",
            Self::Expired { .. } => "EXPIRED",
            Self::Denied { .. } => "DENIED",
        }
    }

    /// Whether nothing can follow this state.
    pub const fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Rejected { .. }
                | Self::Filled { .. }
                | Self::Canceled { .. }
                | Self::Expired { .. }
                | Self::Denied { .. }
        )
    }

    /// The broker's order number, once one exists.
    pub fn broker_order_no(&self) -> Option<&str> {
        match self {
            Self::Accepted { broker_order_no }
            | Self::Filled { broker_order_no }
            | Self::Canceled { broker_order_no }
            | Self::Expired { broker_order_no }
            | Self::PartiallyFilled {
                broker_order_no, ..
            } => Some(broker_order_no),
            _ => None,
        }
    }

    /// Whether this intent may still be submitted to the broker.
    ///
    /// True in exactly one state. `Unknown` is false, which is the point of
    /// the whole module.
    pub const fn may_submit(&self) -> bool {
        matches!(self, Self::RiskApproved)
    }

    /// How the transport-level guard should see this intent.
    ///
    /// `idempotency::IntentState` is `kis-client`'s in-memory, per-process
    /// guard from Todo 36; this machine is the durable authority. They are
    /// reconciled here rather than left to disagree. Note that the guard's
    /// `Rejected → may resubmit` arm is unreachable through the Live path:
    /// a rejection is terminal in this machine, and 0018 would refuse a second
    /// gate decision for the ref anyway, so a retry is necessarily a new
    /// intent.
    pub fn as_broker_intent_state(&self) -> Option<IntentState> {
        match self {
            Self::Submitting | Self::Submitted => Some(IntentState::Submitting),
            Self::Accepted { broker_order_no }
            | Self::Filled { broker_order_no }
            | Self::Canceled { broker_order_no }
            | Self::Expired { broker_order_no }
            | Self::PartiallyFilled {
                broker_order_no, ..
            } => Some(IntentState::Acknowledged {
                broker_order_no: broker_order_no.clone(),
            }),
            Self::Rejected { reason } => Some(IntentState::Rejected {
                reason: reason.clone(),
            }),
            Self::Unknown => Some(IntentState::Unknown),
            Self::IntentCreated | Self::RiskApproved | Self::Denied { .. } => None,
        }
    }

    /// Applies an event, or explains why it cannot be applied.
    ///
    /// Total and pure. Every `(state, event)` pair is either a move, an
    /// explicit no-change, or a named error — there is no fallthrough that
    /// silently keeps the old state.
    pub fn apply(&self, event: &Event) -> Result<Applied, TransitionError> {
        // A terminal state absorbs nothing. Checked first so that a late
        // broker event after a cancel cannot revive an order.
        if self.is_terminal() {
            // ...except a repeat of the terminal fact itself, which a
            // reconnect will re-send.
            if self.is_repeat_of_terminal(event) {
                return Ok(Applied::NoChange);
            }
            return Err(TransitionError::Terminal { state: self.name() });
        }

        match (self, event) {
            (Self::IntentCreated, Event::RiskApproved) => Ok(Applied::Moved(Self::RiskApproved)),
            (Self::IntentCreated, Event::RiskDenied { reason }) => {
                Ok(Applied::Moved(Self::Denied {
                    reason: reason.clone(),
                }))
            }
            (Self::RiskApproved, Event::SubmissionStarted) => Ok(Applied::Moved(Self::Submitting)),
            (Self::Submitting, Event::SubmissionSent) => Ok(Applied::Moved(Self::Submitted)),

            // A timeout may strike once the request is in flight, and from
            // either side of "sent" — that ambiguity is exactly what UNKNOWN
            // records.
            (Self::Submitting | Self::Submitted, Event::SubmissionTimedOut) => {
                Ok(Applied::Moved(Self::Unknown))
            }

            // The broker may answer while we still think we are submitting:
            // the ack can beat our own bookkeeping.
            (Self::Submitting | Self::Submitted, Event::BrokerAccepted { broker_order_no }) => {
                Ok(Applied::Moved(Self::Accepted {
                    broker_order_no: broker_order_no.clone(),
                }))
            }
            (Self::Submitting | Self::Submitted, Event::BrokerRejected { reason }) => {
                Ok(Applied::Moved(Self::Rejected {
                    reason: reason.clone(),
                }))
            }

            // UNKNOWN leaves only by lookup.
            (Self::Unknown, Event::BrokerLookupResolved { resolved }) => {
                self.resolve_unknown(resolved)
            }
            // A late broker event that arrives while UNKNOWN is itself an
            // answer: it proves the order reached the broker.
            (Self::Unknown, Event::BrokerAccepted { broker_order_no }) => {
                Ok(Applied::Moved(Self::Accepted {
                    broker_order_no: broker_order_no.clone(),
                }))
            }
            (Self::Unknown, Event::BrokerRejected { reason }) => {
                Ok(Applied::Moved(Self::Rejected {
                    reason: reason.clone(),
                }))
            }

            (
                Self::Accepted { .. } | Self::PartiallyFilled { .. } | Self::Unknown,
                Event::Fill {
                    broker_order_no,
                    cumulative_filled,
                    total_quantity,
                },
            ) => self.apply_fill(broker_order_no, *cumulative_filled, *total_quantity),

            (
                Self::Accepted { .. } | Self::PartiallyFilled { .. } | Self::Unknown,
                Event::BrokerCanceled { broker_order_no },
            ) => {
                self.check_broker_order(broker_order_no)?;
                Ok(Applied::Moved(Self::Canceled {
                    broker_order_no: broker_order_no.clone(),
                }))
            }
            (
                Self::Accepted { .. } | Self::PartiallyFilled { .. } | Self::Unknown,
                Event::BrokerExpired { broker_order_no },
            ) => {
                self.check_broker_order(broker_order_no)?;
                Ok(Applied::Moved(Self::Expired {
                    broker_order_no: broker_order_no.clone(),
                }))
            }

            // Re-sent lifecycle events for the state we are already in.
            (Self::Accepted { broker_order_no }, Event::BrokerAccepted { broker_order_no: n })
                if broker_order_no == n =>
            {
                Ok(Applied::NoChange)
            }

            (state, event) => Err(TransitionError::Illegal {
                from: state.name(),
                event: event.name(),
            }),
        }
    }

    /// Whether `event` merely restates the terminal fact already recorded.
    fn is_repeat_of_terminal(&self, event: &Event) -> bool {
        match (self, event) {
            (Self::Rejected { .. }, Event::BrokerRejected { .. }) => true,
            (Self::Canceled { broker_order_no }, Event::BrokerCanceled { broker_order_no: n })
            | (Self::Expired { broker_order_no }, Event::BrokerExpired { broker_order_no: n }) => {
                broker_order_no == n
            }
            (
                Self::Filled { broker_order_no },
                Event::Fill {
                    broker_order_no: n,
                    cumulative_filled,
                    total_quantity,
                },
            ) => broker_order_no == n && cumulative_filled == total_quantity,
            _ => false,
        }
    }

    /// `Unknown` may resolve to any concrete broker outcome, and to nothing
    /// else. Resolving back into the submission path is refused: that is the
    /// resubmission this module exists to prevent, wearing a different hat.
    fn resolve_unknown(&self, resolved: &OrderIntentState) -> Result<Applied, TransitionError> {
        match resolved {
            Self::Accepted { .. }
            | Self::Rejected { .. }
            | Self::PartiallyFilled { .. }
            | Self::Filled { .. }
            | Self::Canceled { .. }
            | Self::Expired { .. } => Ok(Applied::Moved(resolved.clone())),
            other => Err(TransitionError::IllegalResolution {
                resolved: other.name(),
            }),
        }
    }

    /// The event must name the order this intent is bound to.
    fn check_broker_order(&self, got: &str) -> Result<(), TransitionError> {
        match self.broker_order_no() {
            Some(expected) if expected != got => Err(TransitionError::BrokerOrderMismatch {
                expected: expected.to_string(),
                got: got.to_string(),
            }),
            _ => Ok(()),
        }
    }

    fn apply_fill(
        &self,
        broker_order_no: &str,
        cumulative: u64,
        total: u64,
    ) -> Result<Applied, TransitionError> {
        self.check_broker_order(broker_order_no)?;
        if cumulative > total {
            return Err(TransitionError::OverFill { cumulative, total });
        }
        // How much this intent already knows to be filled. `Unknown` has no
        // record, so it is treated as nothing filled: any positive report
        // advances it, which is the conservative direction because it moves
        // the intent toward a resolved state rather than leaving it stuck.
        let known = match self {
            Self::PartiallyFilled {
                cumulative_filled, ..
            } => *cumulative_filled,
            _ => 0,
        };

        // A report that does not advance the cumulative total is a duplicate
        // or an out-of-order re-send. Not an error -- brokers re-send after
        // every reconnect -- and it must NOT move the ledger.
        if cumulative <= known {
            return Ok(Applied::NoChange);
        }

        if cumulative == total {
            Ok(Applied::Moved(Self::Filled {
                broker_order_no: broker_order_no.to_string(),
            }))
        } else {
            Ok(Applied::Moved(Self::PartiallyFilled {
                broker_order_no: broker_order_no.to_string(),
                cumulative_filled: cumulative,
            }))
        }
    }
}

impl Event {
    pub const fn name(&self) -> &'static str {
        match self {
            Self::RiskApproved => "RISK_APPROVED",
            Self::RiskDenied { .. } => "RISK_DENIED",
            Self::SubmissionStarted => "SUBMISSION_STARTED",
            Self::SubmissionSent => "SUBMISSION_SENT",
            Self::BrokerAccepted { .. } => "BROKER_ACCEPTED",
            Self::BrokerRejected { .. } => "BROKER_REJECTED",
            Self::SubmissionTimedOut => "SUBMISSION_TIMED_OUT",
            Self::Fill { .. } => "FILL",
            Self::BrokerCanceled { .. } => "BROKER_CANCELED",
            Self::BrokerExpired { .. } => "BROKER_EXPIRED",
            Self::BrokerLookupResolved { .. } => "BROKER_LOOKUP_RESOLVED",
        }
    }
}

impl fmt::Display for OrderIntentState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Replays an event log onto a starting state.
///
/// This is what a restart does: the durable log is the truth, and the state
/// is derived from it. `NoChange` events are applied and simply do not move
/// the state, so a log containing broker re-sends replays identically.
pub fn replay(
    start: OrderIntentState,
    events: &[Event],
) -> Result<OrderIntentState, TransitionError> {
    let mut state = start;
    for event in events {
        if let Applied::Moved(next) = state.apply(event)? {
            state = next;
        }
    }
    Ok(state)
}
