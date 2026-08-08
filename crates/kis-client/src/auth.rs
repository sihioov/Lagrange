//! Serialized token issue and refresh (plan Todo 36, design §6.12).
//!
//! "토큰 발급·갱신은 단일 책임 모듈에서 직렬화한다." Serialization is the
//! entire contract. KIS rate-limits token issuance hard and invalidates the
//! previous token when a new one is issued, so N concurrent callers that each
//! notice an expired token and each request a new one do not merely waste
//! calls — they invalidate each other's tokens and can lock the account out.
//!
//! The guard is a single mutex held across the *issue*, plus a re-check inside
//! the critical section. The re-check is what makes it correct: without it,
//! every caller that queued on the mutex would still issue after the winner
//! released it, which is the same stampede one lock later.
//!
//! The token itself is a [`Secret`], so it cannot reach a log, an error, or a
//! derived `Debug`.

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::clock::Clock;
use crate::error::KisError;
use crate::secret::Secret;

/// An access token and when it stops being usable.
#[derive(Clone)]
pub struct AccessToken {
    pub value: Secret<String>,
    /// UNIX ms after which the token must not be used.
    pub expires_at_ms: i64,
}

impl std::fmt::Debug for AccessToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never render the value; the expiry is safe and is the only field
        // anyone debugging token behaviour actually needs.
        f.debug_struct("AccessToken")
            .field("value", &self.value)
            .field("expires_at_ms", &self.expires_at_ms)
            .finish()
    }
}

/// Issues a fresh token. Implemented by the REST client in production and by a
/// counting stub in tests, so the serialization contract can be asserted
/// without a network.
#[async_trait::async_trait]
pub trait TokenIssuer: Send + Sync {
    async fn issue(&self) -> Result<AccessToken, KisError>;
}

/// How long before real expiry a token is treated as stale.
///
/// Without a margin a token can pass the check here and expire in flight,
/// producing an auth failure on a request that looked fine — and on an order
/// path that failure is ambiguous, not clean. Renewing early converts a
/// hard-to-diagnose race into an ordinary refresh.
pub const DEFAULT_REFRESH_MARGIN_MS: i64 = 60_000;

/// The single owner of the access token.
pub struct TokenManager {
    clock: Arc<dyn Clock>,
    issuer: Arc<dyn TokenIssuer>,
    refresh_margin_ms: i64,
    /// The mutex IS the serialization contract; it is held across issue().
    state: Mutex<Option<AccessToken>>,
}

impl TokenManager {
    pub fn new(clock: Arc<dyn Clock>, issuer: Arc<dyn TokenIssuer>) -> Self {
        Self {
            clock,
            issuer,
            refresh_margin_ms: DEFAULT_REFRESH_MARGIN_MS,
            state: Mutex::new(None),
        }
    }

    pub fn with_refresh_margin_ms(mut self, margin: i64) -> Self {
        self.refresh_margin_ms = margin;
        self
    }

    fn is_usable(&self, token: &AccessToken, now: i64) -> bool {
        now + self.refresh_margin_ms < token.expires_at_ms
    }

    /// The current token, issuing one only if necessary.
    ///
    /// Concurrent callers serialize here and exactly ONE issue happens: the
    /// winner stores the token, and everyone queued behind it re-checks and
    /// finds a usable one.
    pub async fn token(&self) -> Result<AccessToken, KisError> {
        let mut guard = self.state.lock().await;
        let now = self.clock.now_ms();

        // Re-check INSIDE the lock. Checking only before acquiring would let
        // every queued caller issue in turn - a stampede one lock later.
        if let Some(existing) = guard.as_ref()
            && self.is_usable(existing, now)
        {
            return Ok(existing.clone());
        }

        let fresh = self.issuer.issue().await?;
        *guard = Some(fresh.clone());
        Ok(fresh)
    }

    /// Force the next [`token`](Self::token) call to issue.
    ///
    /// For the 401 path: the broker is the authority on whether a token is
    /// still good, and it can revoke one before its stated expiry.
    pub async fn invalidate(&self) {
        *self.state.lock().await = None;
    }
}

impl std::fmt::Debug for TokenManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenManager")
            .field("refresh_margin_ms", &self.refresh_margin_ms)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::TestClock;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingIssuer {
        clock: TestClock,
        calls: AtomicUsize,
        ttl_ms: i64,
        fail: bool,
    }

    impl CountingIssuer {
        fn new(clock: TestClock, ttl_ms: i64) -> Arc<Self> {
            Arc::new(Self {
                clock,
                calls: AtomicUsize::new(0),
                ttl_ms,
                fail: false,
            })
        }
        fn failing(clock: TestClock) -> Arc<Self> {
            Arc::new(Self {
                clock,
                calls: AtomicUsize::new(0),
                ttl_ms: 0,
                fail: true,
            })
        }
        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl TokenIssuer for CountingIssuer {
        async fn issue(&self) -> Result<AccessToken, KisError> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if self.fail {
                return Err(KisError::Auth {
                    reason: "issuer refused".to_string(),
                });
            }
            // Yield so concurrent callers genuinely interleave; without this a
            // race test can pass on scheduling luck alone.
            tokio::task::yield_now().await;
            Ok(AccessToken {
                value: Secret::new(format!("token-{n}")),
                expires_at_ms: self.clock.now_ms() + self.ttl_ms,
            })
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_token_race_issues_exactly_one_token() {
        // The defect this prevents: KIS invalidates the previous token when a
        // new one is issued, so N concurrent refreshes invalidate each other.
        let clock = TestClock::at(0);
        let issuer = CountingIssuer::new(clock.clone(), 3_600_000);
        let mgr = Arc::new(TokenManager::new(
            Arc::new(clock.clone()),
            issuer.clone() as Arc<dyn TokenIssuer>,
        ));

        let mut handles = Vec::new();
        for _ in 0..32 {
            let m = Arc::clone(&mgr);
            handles.push(tokio::spawn(async move { m.token().await }));
        }
        let mut values = Vec::new();
        for h in handles {
            values.push(
                h.await
                    .expect("task")
                    .expect("token")
                    .value
                    .expose()
                    .clone(),
            );
        }

        assert_eq!(issuer.calls(), 1, "32 concurrent callers must issue once");
        assert!(
            values.iter().all(|v| v == &values[0]),
            "every caller must get the SAME token"
        );
    }

    #[tokio::test]
    async fn a_usable_token_is_reused_without_issuing() {
        let clock = TestClock::at(0);
        let issuer = CountingIssuer::new(clock.clone(), 3_600_000);
        let mgr = TokenManager::new(
            Arc::new(clock.clone()),
            issuer.clone() as Arc<dyn TokenIssuer>,
        );

        mgr.token().await.expect("first");
        mgr.token().await.expect("second");
        mgr.token().await.expect("third");
        assert_eq!(issuer.calls(), 1);
    }

    #[tokio::test]
    async fn a_token_is_refreshed_before_it_actually_expires() {
        // Renewing exactly at expiry lets a token die in flight; on an order
        // path that surfaces as an ambiguous failure rather than a clean one.
        let clock = TestClock::at(0);
        let issuer = CountingIssuer::new(clock.clone(), 100_000);
        let mgr = TokenManager::new(
            Arc::new(clock.clone()),
            issuer.clone() as Arc<dyn TokenIssuer>,
        )
        .with_refresh_margin_ms(60_000);

        mgr.token().await.expect("first");
        assert_eq!(issuer.calls(), 1);

        // 50s in: 50s of life left, which is inside the 60s margin.
        clock.advance_ms(50_000);
        mgr.token().await.expect("refreshed");
        assert_eq!(
            issuer.calls(),
            2,
            "a token inside the refresh margin must be renewed early"
        );
    }

    #[tokio::test]
    async fn invalidate_forces_the_next_call_to_issue() {
        let clock = TestClock::at(0);
        let issuer = CountingIssuer::new(clock.clone(), 3_600_000);
        let mgr = TokenManager::new(
            Arc::new(clock.clone()),
            issuer.clone() as Arc<dyn TokenIssuer>,
        );

        mgr.token().await.expect("first");
        assert_eq!(issuer.calls(), 1);
        // The broker is the authority: it may revoke before stated expiry.
        mgr.invalidate().await;
        mgr.token().await.expect("reissued");
        assert_eq!(issuer.calls(), 2);
    }

    #[tokio::test]
    async fn an_issue_failure_is_typed_and_caches_nothing() {
        let clock = TestClock::at(0);
        let issuer = CountingIssuer::failing(clock.clone());
        let mgr = TokenManager::new(
            Arc::new(clock.clone()),
            issuer.clone() as Arc<dyn TokenIssuer>,
        );

        let err = mgr.token().await.expect_err("issuer refuses");
        assert_eq!(err.code(), "BROKER_AUTH_FAILED");
        // A failed issue must not poison the manager into never retrying.
        let _ = mgr.token().await;
        assert_eq!(issuer.calls(), 2);
    }

    #[tokio::test]
    async fn a_token_never_renders_its_value() {
        let clock = TestClock::at(0);
        let issuer = CountingIssuer::new(clock.clone(), 3_600_000);
        let mgr = TokenManager::new(
            Arc::new(clock.clone()),
            issuer.clone() as Arc<dyn TokenIssuer>,
        );
        let t = mgr.token().await.expect("token");
        let rendered = format!("{t:?}");
        assert!(!rendered.contains("token-1"), "{rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
        // The manager itself must not leak either.
        assert!(!format!("{mgr:?}").contains("token-1"));
    }
}
