//! Bounded retry with backoff (plan Todo 36).
//!
//! The policy is deliberately asymmetric and the asymmetry is not a tuning
//! choice — it is the safety property. A read may be repeated freely because
//! repeating it cannot create state. A mutation may be repeated ONLY when the
//! transport can prove the request never reached the broker, because every
//! other case leaves open that an order already exists.
//!
//! [`execute`] therefore takes a [`RequestKind`] and consults
//! [`KisError::is_retryable`], which requires one. There is no "retry
//! everything transient" path that a mutation could accidentally take.
//!
//! Backoff is exponential with a cap, and the broker's own `retry_after_ms`
//! wins when it supplies one: guessing shorter than the broker asked is how a
//! throttle becomes a ban. Jitter is injected rather than random so tests stay
//! deterministic.

use std::future::Future;

use crate::error::{KisError, RequestKind};

/// Bounded retry parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Total attempts INCLUDING the first. 1 means "never retry".
    pub max_attempts: u32,
    pub initial_backoff_ms: u64,
    pub max_backoff_ms: u64,
}

impl RetryPolicy {
    /// Reads: a few quick retries absorb a transient 429/500.
    pub const fn reads() -> Self {
        Self {
            max_attempts: 4,
            initial_backoff_ms: 100,
            max_backoff_ms: 2_000,
        }
    }

    /// Mutations: at most one repeat, and only for a proven-unsent request.
    /// Kept tight on purpose — a long mutation retry chain is a long window in
    /// which an order the broker DID receive gets sent again.
    pub const fn mutations() -> Self {
        Self {
            max_attempts: 2,
            initial_backoff_ms: 50,
            max_backoff_ms: 200,
        }
    }

    /// Backoff before `attempt` (1-based: attempt 2 is the first retry).
    pub fn backoff_ms(&self, attempt: u32) -> u64 {
        if attempt <= 1 {
            return 0;
        }
        let exp = attempt.saturating_sub(2).min(16);
        let raw = self.initial_backoff_ms.saturating_mul(1u64 << exp);
        raw.min(self.max_backoff_ms)
    }
}

/// Where the executor sleeps. Injected so tests record delays instead of
/// waiting them out.
#[allow(async_fn_in_trait)]
pub trait Sleeper: Send + Sync {
    fn sleep_ms(&self, ms: u64) -> impl Future<Output = ()> + Send;
}

/// Real sleeping.
#[derive(Debug, Clone, Default)]
pub struct TokioSleeper;

impl Sleeper for TokioSleeper {
    fn sleep_ms(&self, ms: u64) -> impl Future<Output = ()> + Send {
        tokio::time::sleep(std::time::Duration::from_millis(ms))
    }
}

/// Run `op` under `policy`, retrying only what `kind` permits.
///
/// `op` is called with the 1-based attempt number so callers can log or vary
/// per attempt. The final error is returned unchanged — in particular an
/// [`KisError::Ambiguous`] is never converted into a failure on the way out.
pub async fn execute<T, F, Fut, S>(
    policy: RetryPolicy,
    kind: RequestKind,
    sleeper: &S,
    mut op: F,
) -> Result<T, KisError>
where
    F: FnMut(u32) -> Fut,
    Fut: Future<Output = Result<T, KisError>>,
    S: Sleeper,
{
    let mut attempt = 1;
    loop {
        match op(attempt).await {
            Ok(v) => return Ok(v),
            Err(e) => {
                let last = attempt >= policy.max_attempts;
                if last || !e.is_retryable(kind) {
                    return Err(e);
                }
                // The broker knows better than our curve when it says so;
                // guessing shorter than it asked turns a throttle into a ban.
                let wait = match &e {
                    KisError::RateLimited { retry_after_ms, .. } => {
                        (*retry_after_ms).max(policy.backoff_ms(attempt + 1))
                    }
                    _ => policy.backoff_ms(attempt + 1),
                };
                sleeper.sleep_ms(wait).await;
                attempt += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingSleeper {
        waits: Mutex<Vec<u64>>,
    }

    impl RecordingSleeper {
        fn waits(&self) -> Vec<u64> {
            self.waits.lock().expect("sleeper mutex").clone()
        }
    }

    impl Sleeper for RecordingSleeper {
        fn sleep_ms(&self, ms: u64) -> impl Future<Output = ()> + Send {
            self.waits.lock().expect("sleeper mutex").push(ms);
            std::future::ready(())
        }
    }

    fn broker(status: u16) -> KisError {
        KisError::Broker {
            status,
            endpoint: "/quote".to_string(),
            body: "{}".to_string(),
        }
    }

    #[tokio::test]
    async fn a_read_retries_a_429_and_then_succeeds() {
        let sleeper = RecordingSleeper::default();
        let got = execute(
            RetryPolicy::reads(),
            RequestKind::Read,
            &sleeper,
            |attempt| async move {
                if attempt < 3 {
                    Err(broker(429))
                } else {
                    Ok("ok")
                }
            },
        )
        .await
        .expect("succeeds on the third attempt");
        assert_eq!(got, "ok");
        assert_eq!(
            sleeper.waits().len(),
            2,
            "two backoffs before the third try"
        );
    }

    #[tokio::test]
    async fn a_read_retries_a_500_up_to_the_bound_then_gives_up() {
        let sleeper = RecordingSleeper::default();
        let calls = Mutex::new(0u32);
        let err = execute(RetryPolicy::reads(), RequestKind::Read, &sleeper, |_| {
            *calls.lock().unwrap() += 1;
            async { Err::<(), _>(broker(500)) }
        })
        .await
        .expect_err("always fails");
        assert_eq!(*calls.lock().unwrap(), RetryPolicy::reads().max_attempts);
        assert!(matches!(err, KisError::Broker { status: 500, .. }));
    }

    #[tokio::test]
    async fn a_mutation_never_retries_an_ambiguous_result() {
        // The single most important property in this crate: a timed-out
        // submit must be attempted exactly once.
        let sleeper = RecordingSleeper::default();
        let calls = Mutex::new(0u32);
        let err = execute(
            RetryPolicy::mutations(),
            RequestKind::Mutation,
            &sleeper,
            |_| {
                *calls.lock().unwrap() += 1;
                async {
                    Err::<(), _>(KisError::Ambiguous {
                        operation: "order.submit".to_string(),
                        client_order_id: "coid-1".to_string(),
                    })
                }
            },
        )
        .await
        .expect_err("ambiguous");
        assert_eq!(*calls.lock().unwrap(), 1, "exactly one submission attempt");
        assert!(sleeper.waits().is_empty(), "no backoff, because no retry");
        // And it must still be ambiguous coming out, not downgraded to failed.
        assert!(err.is_ambiguous());
        assert_eq!(err.code(), "ORDER_STATE_UNKNOWN");
    }

    #[tokio::test]
    async fn a_mutation_never_retries_a_500() {
        // A 500 after sending leaves open that the broker acted on it.
        let sleeper = RecordingSleeper::default();
        let calls = Mutex::new(0u32);
        let _ = execute(
            RetryPolicy::mutations(),
            RequestKind::Mutation,
            &sleeper,
            |_| {
                *calls.lock().unwrap() += 1;
                async { Err::<(), _>(broker(500)) }
            },
        )
        .await;
        assert_eq!(*calls.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn a_mutation_retries_only_a_proven_unsent_request() {
        let sleeper = RecordingSleeper::default();
        let calls = Mutex::new(0u32);
        let got = execute(
            RetryPolicy::mutations(),
            RequestKind::Mutation,
            &sleeper,
            |attempt| {
                *calls.lock().unwrap() += 1;
                async move {
                    if attempt == 1 {
                        Err(KisError::Connect {
                            reason: "connection refused".to_string(),
                        })
                    } else {
                        Ok("submitted")
                    }
                }
            },
        )
        .await
        .expect("a request that never left may be repeated");
        assert_eq!(got, "submitted");
        assert_eq!(*calls.lock().unwrap(), 2);
    }

    #[tokio::test]
    async fn the_brokers_retry_after_wins_over_our_curve() {
        // Guessing shorter than the broker asked turns a throttle into a ban.
        let sleeper = RecordingSleeper::default();
        let _ = execute(
            RetryPolicy::reads(),
            RequestKind::Read,
            &sleeper,
            |_| async {
                Err::<(), _>(KisError::RateLimited {
                    endpoint: "/quote".to_string(),
                    retry_after_ms: 5_000,
                })
            },
        )
        .await;
        assert!(
            sleeper.waits().iter().all(|w| *w >= 5_000),
            "every wait must honour the broker: {:?}",
            sleeper.waits()
        );
    }

    #[test]
    fn backoff_grows_then_caps() {
        let p = RetryPolicy::reads();
        assert_eq!(p.backoff_ms(1), 0, "no wait before the first attempt");
        assert_eq!(p.backoff_ms(2), 100);
        assert_eq!(p.backoff_ms(3), 200);
        assert_eq!(p.backoff_ms(4), 400);
        // Cap holds no matter how far it is pushed.
        assert_eq!(p.backoff_ms(50), p.max_backoff_ms);
    }

    #[tokio::test]
    async fn schema_drift_is_not_retried_even_for_a_read() {
        let sleeper = RecordingSleeper::default();
        let calls = Mutex::new(0u32);
        let _ = execute(RetryPolicy::reads(), RequestKind::Read, &sleeper, |_| {
            *calls.lock().unwrap() += 1;
            async {
                Err::<(), _>(KisError::SchemaDrift {
                    endpoint: "/balance".to_string(),
                    detail: "missing output".to_string(),
                })
            }
        })
        .await;
        assert_eq!(
            *calls.lock().unwrap(),
            1,
            "repeating a request the code cannot parse just repeats the problem"
        );
    }
}
