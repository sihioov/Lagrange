//! Order intent idempotency and broker order-number mapping (plan Todo 36).
//!
//! Two design §6.12 rules meet here, and they are the same rule seen from two
//! sides:
//!
//! * "같은 idempotency key의 주문 의도는 한 번만 제출한다."
//! * "내부 주문 ID와 KIS 주문번호를 영속 매핑한다."
//!
//! The mapping has to be **persisted before the request is sent**, not after
//! the response arrives. If it were written on success, a crash between send
//! and response would leave no record that anything was submitted — and the
//! restarted process would submit again. Claiming the key first means the
//! worst case is a claimed intent whose outcome is unknown, which is
//! recoverable by querying the broker; the alternative's worst case is a
//! duplicate order, which is not recoverable at all.
//!
//! That is why [`IntentState::Submitting`] exists. It is not bookkeeping: it is
//! the state that says "this may or may not have reached the broker", and it
//! resolves to [`IntentState::Unknown`] rather than to failure when a submit
//! times out — the same ambiguity [`crate::error::KisError::Ambiguous`]
//! carries at the transport layer.

use std::collections::HashMap;
use std::sync::Mutex;

/// Where an order intent has got to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntentState {
    /// Claimed and about to be sent. The request may or may not reach the
    /// broker from here.
    Submitting,
    /// The broker acknowledged and gave us its order number.
    Acknowledged { broker_order_no: String },
    /// The broker explicitly refused. Terminal, and safe: a rejection means no
    /// order exists.
    Rejected { reason: String },
    /// The outcome is genuinely unknown — a timeout after the request was
    /// sent. NOT a failure and NOT resubmittable; resolve by querying the
    /// broker (design §16 "주문 조회로 해소 전 재제출 금지").
    Unknown,
}

impl IntentState {
    /// Whether a new submission may be made for this key.
    ///
    /// Only a `Rejected` intent may be re-submitted, because only a rejection
    /// proves no order exists. `Unknown` deliberately returns false: that is
    /// the whole point of tracking it separately from `Rejected`.
    pub fn allows_submission(&self) -> bool {
        matches!(self, Self::Rejected { .. })
    }
}

/// The outcome of trying to claim an idempotency key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    /// The caller owns this key and must proceed to submit.
    Granted,
    /// Someone already claimed it. The existing state says what to do next —
    /// never resubmit on the strength of this alone.
    AlreadyClaimed { state: IntentState },
}

/// Durable storage for order intents.
///
/// A trait so the production implementation can be the database (surviving the
/// crash this design exists to handle) while tests use memory. The in-memory
/// implementation is explicitly NOT sufficient for production: a process
/// restart would forget every claim.
pub trait IntentStore: Send + Sync {
    /// Atomically claim `key` if unclaimed. MUST be atomic — a check-then-write
    /// race here submits the same order twice, which is the exact failure this
    /// module exists to prevent.
    fn claim(&self, key: &str) -> Claim;
    fn get(&self, key: &str) -> Option<IntentState>;
    fn set(&self, key: &str, state: IntentState);
    /// Resolve an internal order id from a broker order number, for
    /// reconciliation after a reconnect.
    fn key_for_broker_order(&self, broker_order_no: &str) -> Option<String>;
}

/// In-memory store for tests and the mock profile.
#[derive(Debug, Default)]
pub struct InMemoryIntentStore {
    intents: Mutex<HashMap<String, IntentState>>,
    by_broker_no: Mutex<HashMap<String, String>>,
}

impl InMemoryIntentStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl IntentStore for InMemoryIntentStore {
    fn claim(&self, key: &str) -> Claim {
        let mut intents = self.intents.lock().expect("intent store mutex");
        // Claim and insert under ONE lock: releasing between the check and the
        // insert is precisely the race that duplicates an order.
        match intents.get(key) {
            Some(state) => Claim::AlreadyClaimed {
                state: state.clone(),
            },
            None => {
                intents.insert(key.to_string(), IntentState::Submitting);
                Claim::Granted
            }
        }
    }

    fn get(&self, key: &str) -> Option<IntentState> {
        self.intents
            .lock()
            .expect("intent store mutex")
            .get(key)
            .cloned()
    }

    fn set(&self, key: &str, state: IntentState) {
        if let IntentState::Acknowledged { broker_order_no } = &state {
            self.by_broker_no
                .lock()
                .expect("broker map mutex")
                .insert(broker_order_no.clone(), key.to_string());
        }
        self.intents
            .lock()
            .expect("intent store mutex")
            .insert(key.to_string(), state);
    }

    fn key_for_broker_order(&self, broker_order_no: &str) -> Option<String> {
        self.by_broker_no
            .lock()
            .expect("broker map mutex")
            .get(broker_order_no)
            .cloned()
    }
}

/// Why a submission was refused before any request was built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SubmitGuardError {
    #[error("order intent {key} was already submitted (broker order {broker_order_no})")]
    AlreadyAcknowledged {
        key: String,
        broker_order_no: String,
    },
    #[error("order intent {key} is in flight; wait for it to resolve")]
    InFlight { key: String },
    #[error(
        "order intent {key} has an UNKNOWN outcome: query the broker before any \
         resubmission (design 16: 주문 조회로 해소 전 재제출 금지)"
    )]
    UnresolvedUnknown { key: String },
}

/// Guard a submission behind its idempotency key.
///
/// Returns `Ok(())` only when the caller may proceed to build and send the
/// request; the key is claimed as a side effect, BEFORE anything is sent.
pub fn guard_submission(store: &dyn IntentStore, key: &str) -> Result<(), SubmitGuardError> {
    match store.claim(key) {
        Claim::Granted => Ok(()),
        Claim::AlreadyClaimed { state } => match state {
            IntentState::Acknowledged { broker_order_no } => {
                Err(SubmitGuardError::AlreadyAcknowledged {
                    key: key.to_string(),
                    broker_order_no,
                })
            }
            IntentState::Submitting => Err(SubmitGuardError::InFlight {
                key: key.to_string(),
            }),
            IntentState::Unknown => Err(SubmitGuardError::UnresolvedUnknown {
                key: key.to_string(),
            }),
            // Only a rejection proves no order exists, so only a rejection may
            // be retried. Re-claim it for the new attempt.
            IntentState::Rejected { .. } => {
                store.set(key, IntentState::Submitting);
                Ok(())
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_claim_is_granted_and_the_second_is_refused() {
        let store = InMemoryIntentStore::new();
        assert!(guard_submission(&store, "coid-1").is_ok());
        // The intent is claimed BEFORE any request is built, so a crash here
        // leaves a record rather than a silent gap.
        assert_eq!(store.get("coid-1"), Some(IntentState::Submitting));
        assert!(matches!(
            guard_submission(&store, "coid-1"),
            Err(SubmitGuardError::InFlight { .. })
        ));
    }

    #[test]
    fn an_acknowledged_intent_is_never_submitted_again() {
        let store = InMemoryIntentStore::new();
        guard_submission(&store, "coid-1").expect("first");
        store.set(
            "coid-1",
            IntentState::Acknowledged {
                broker_order_no: "0000117057".to_string(),
            },
        );
        let err = guard_submission(&store, "coid-1").expect_err("already submitted");
        assert!(matches!(err, SubmitGuardError::AlreadyAcknowledged { .. }));
        // The error names the broker order, so an operator can look it up.
        assert!(err.to_string().contains("0000117057"));
    }

    #[test]
    fn an_unknown_outcome_blocks_resubmission() {
        // The core safety property: a timed-out submit must not be retried
        // just because it did not succeed.
        let store = InMemoryIntentStore::new();
        guard_submission(&store, "coid-1").expect("first");
        store.set("coid-1", IntentState::Unknown);

        let err = guard_submission(&store, "coid-1").expect_err("must not resubmit");
        assert!(matches!(err, SubmitGuardError::UnresolvedUnknown { .. }));
        assert!(!IntentState::Unknown.allows_submission());
        assert!(
            err.to_string().contains("query the broker"),
            "the error must say how to resolve it: {err}"
        );
    }

    #[test]
    fn only_a_rejection_permits_a_fresh_submission() {
        // A rejection is the ONLY state that proves no order exists.
        let store = InMemoryIntentStore::new();
        guard_submission(&store, "coid-1").expect("first");
        store.set(
            "coid-1",
            IntentState::Rejected {
                reason: "insufficient balance".to_string(),
            },
        );
        assert!(
            guard_submission(&store, "coid-1").is_ok(),
            "a rejected intent may be corrected and resubmitted"
        );
        // ...and re-claiming puts it back in flight, so a third caller waits.
        assert!(matches!(
            guard_submission(&store, "coid-1"),
            Err(SubmitGuardError::InFlight { .. })
        ));
    }

    #[test]
    fn state_transitions_agree_with_allows_submission() {
        assert!(!IntentState::Submitting.allows_submission());
        assert!(!IntentState::Unknown.allows_submission());
        assert!(
            !IntentState::Acknowledged {
                broker_order_no: "1".into()
            }
            .allows_submission()
        );
        assert!(IntentState::Rejected { reason: "x".into() }.allows_submission());
    }

    #[test]
    fn a_broker_order_number_resolves_back_to_its_internal_key() {
        // Needed after a reconnect: the broker reports order numbers, and
        // reconciliation has to map them back to our own intents.
        let store = InMemoryIntentStore::new();
        guard_submission(&store, "coid-42").expect("claim");
        store.set(
            "coid-42",
            IntentState::Acknowledged {
                broker_order_no: "0000117057".to_string(),
            },
        );
        assert_eq!(
            store.key_for_broker_order("0000117057").as_deref(),
            Some("coid-42")
        );
        assert_eq!(store.key_for_broker_order("9999999999"), None);
    }

    #[test]
    fn concurrent_claims_grant_exactly_one() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let store: Arc<dyn IntentStore> = Arc::new(InMemoryIntentStore::new());
        let granted = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..16 {
            let s = Arc::clone(&store);
            let g = Arc::clone(&granted);
            handles.push(std::thread::spawn(move || {
                if guard_submission(s.as_ref(), "coid-race").is_ok() {
                    g.fetch_add(1, Ordering::SeqCst);
                }
            }));
        }
        for h in handles {
            h.join().expect("thread");
        }
        assert_eq!(
            granted.load(Ordering::SeqCst),
            1,
            "a check-then-write race here submits the same order twice"
        );
    }
}
