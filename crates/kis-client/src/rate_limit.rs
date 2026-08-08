//! Per-endpoint / per-TR token buckets (plan Todo 36, design §6.12).
//!
//! "API 호출 제한은 endpoint/TR별 token bucket으로 관리한다." The bucket is
//! keyed by `(endpoint, tr_id)` rather than a single global limiter, and that
//! distinction is the whole point: KIS meters different TR ids separately, so
//! one endpoint exhausting its allowance must not starve another. A global
//! limiter would be simpler and would convert a busy quote feed into an order
//! that cannot be submitted.
//!
//! The bucket is time-driven off an injected [`Clock`], so tests advance time
//! by hand instead of sleeping. Refill is computed lazily on acquire — no
//! background task, nothing to shut down, and no drift between a timer and the
//! clock the rest of the adapter uses.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::clock::Clock;

/// Identifies one metered channel. KIS meters per transaction id, so the TR is
/// part of the key, not a label.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BucketKey {
    pub endpoint: String,
    pub tr_id: String,
}

impl BucketKey {
    pub fn new(endpoint: impl Into<String>, tr_id: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            tr_id: tr_id.into(),
        }
    }
}

/// How fast one channel may be called.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quota {
    /// Maximum burst.
    pub capacity: u32,
    /// Tokens restored per second.
    pub refill_per_sec: u32,
}

impl Quota {
    pub fn new(capacity: u32, refill_per_sec: u32) -> Self {
        Self {
            capacity,
            refill_per_sec,
        }
    }
}

/// The outcome of asking to make a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permit {
    /// Proceed now.
    Granted,
    /// Wait this long, then ask again. The caller decides whether to wait or
    /// give up — the limiter never sleeps on the caller's behalf, because a
    /// hidden sleep inside an order path is how a submit silently misses its
    /// window.
    Throttled { retry_after_ms: u64 },
}

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    last_refill_ms: i64,
}

/// Token buckets keyed by `(endpoint, tr_id)`.
pub struct RateLimiter {
    clock: Arc<dyn Clock>,
    default_quota: Quota,
    quotas: HashMap<BucketKey, Quota>,
    buckets: Mutex<HashMap<BucketKey, Bucket>>,
}

impl RateLimiter {
    pub fn new(clock: Arc<dyn Clock>, default_quota: Quota) -> Self {
        Self {
            clock,
            default_quota,
            quotas: HashMap::new(),
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Override the quota for one channel (KIS publishes different limits per
    /// TR; order endpoints are tighter than quote endpoints).
    pub fn with_quota(mut self, key: BucketKey, quota: Quota) -> Self {
        self.quotas.insert(key, quota);
        self
    }

    fn quota_for(&self, key: &BucketKey) -> Quota {
        self.quotas.get(key).copied().unwrap_or(self.default_quota)
    }

    /// Try to consume one token for `key`.
    pub fn acquire(&self, key: &BucketKey) -> Permit {
        let quota = self.quota_for(key);
        let now = self.clock.now_ms();
        let mut buckets = self.buckets.lock().expect("rate limiter mutex");
        let bucket = buckets.entry(key.clone()).or_insert(Bucket {
            tokens: quota.capacity as f64,
            last_refill_ms: now,
        });

        // Lazy refill: no background task, and no drift against the clock the
        // rest of the adapter reads.
        let elapsed_ms = (now - bucket.last_refill_ms).max(0) as f64;
        if elapsed_ms > 0.0 {
            let refilled = elapsed_ms / 1000.0 * quota.refill_per_sec as f64;
            bucket.tokens = (bucket.tokens + refilled).min(quota.capacity as f64);
            bucket.last_refill_ms = now;
        }

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            return Permit::Granted;
        }

        // How long until one whole token exists.
        let missing = 1.0 - bucket.tokens;
        let secs = if quota.refill_per_sec == 0 {
            f64::INFINITY
        } else {
            missing / quota.refill_per_sec as f64
        };
        let retry_after_ms = if secs.is_finite() {
            (secs * 1000.0).ceil() as u64
        } else {
            u64::MAX
        };
        Permit::Throttled { retry_after_ms }
    }
}

impl std::fmt::Debug for RateLimiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RateLimiter")
            .field("default_quota", &self.default_quota)
            .field("overrides", &self.quotas.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::TestClock;

    fn limiter(clock: TestClock, quota: Quota) -> RateLimiter {
        RateLimiter::new(Arc::new(clock), quota)
    }

    #[test]
    fn a_burst_is_allowed_up_to_capacity_then_throttled() {
        let clock = TestClock::at(0);
        let rl = limiter(clock.clone(), Quota::new(3, 1));
        let key = BucketKey::new("/quote", "FHKST01010100");

        for i in 0..3 {
            assert_eq!(rl.acquire(&key), Permit::Granted, "burst call {i}");
        }
        match rl.acquire(&key) {
            Permit::Throttled { retry_after_ms } => {
                assert!(
                    retry_after_ms > 0 && retry_after_ms <= 1000,
                    "{retry_after_ms}"
                )
            }
            Permit::Granted => panic!("capacity was 3; the fourth call must throttle"),
        }
    }

    #[test]
    fn tokens_refill_with_elapsed_time() {
        let clock = TestClock::at(0);
        let rl = limiter(clock.clone(), Quota::new(2, 2));
        let key = BucketKey::new("/quote", "TR1");

        assert_eq!(rl.acquire(&key), Permit::Granted);
        assert_eq!(rl.acquire(&key), Permit::Granted);
        assert!(matches!(rl.acquire(&key), Permit::Throttled { .. }));

        clock.advance_ms(500); // 2/sec for 0.5s = 1 token
        assert_eq!(rl.acquire(&key), Permit::Granted);
    }

    #[test]
    fn refill_never_exceeds_capacity() {
        // Otherwise a long idle period would bank an unbounded burst and the
        // first busy moment would blow straight through the broker's limit.
        let clock = TestClock::at(0);
        let rl = limiter(clock.clone(), Quota::new(2, 10));
        let key = BucketKey::new("/quote", "TR1");
        clock.advance_ms(60_000);

        assert_eq!(rl.acquire(&key), Permit::Granted);
        assert_eq!(rl.acquire(&key), Permit::Granted);
        assert!(
            matches!(rl.acquire(&key), Permit::Throttled { .. }),
            "an idle hour must not bank more than capacity"
        );
    }

    #[test]
    fn exhausting_one_channel_never_starves_another() {
        // The reason the bucket is keyed by (endpoint, tr_id) at all: a busy
        // quote feed must not make an order unsubmittable.
        let clock = TestClock::at(0);
        let rl = limiter(clock.clone(), Quota::new(2, 1));
        let quotes = BucketKey::new("/quote", "FHKST01010100");
        let orders = BucketKey::new("/order", "TTTC0802U");

        assert_eq!(rl.acquire(&quotes), Permit::Granted);
        assert_eq!(rl.acquire(&quotes), Permit::Granted);
        assert!(matches!(rl.acquire(&quotes), Permit::Throttled { .. }));

        assert_eq!(
            rl.acquire(&orders),
            Permit::Granted,
            "the order channel has its own allowance"
        );
    }

    #[test]
    fn the_same_endpoint_under_different_tr_ids_is_metered_separately() {
        // KIS meters per TR, so two TRs on one path are two channels.
        let clock = TestClock::at(0);
        let rl = limiter(clock.clone(), Quota::new(1, 1));
        let a = BucketKey::new("/order", "TTTC0802U");
        let b = BucketKey::new("/order", "TTTC0801U");

        assert_eq!(rl.acquire(&a), Permit::Granted);
        assert!(matches!(rl.acquire(&a), Permit::Throttled { .. }));
        assert_eq!(rl.acquire(&b), Permit::Granted);
    }

    #[test]
    fn a_per_channel_quota_overrides_the_default() {
        let clock = TestClock::at(0);
        let orders = BucketKey::new("/order", "TTTC0802U");
        let rl =
            limiter(clock.clone(), Quota::new(10, 10)).with_quota(orders.clone(), Quota::new(1, 1));

        assert_eq!(rl.acquire(&orders), Permit::Granted);
        assert!(
            matches!(rl.acquire(&orders), Permit::Throttled { .. }),
            "the tighter order quota must win over the default"
        );
        // The default still applies elsewhere.
        assert_eq!(
            rl.acquire(&BucketKey::new("/quote", "TR1")),
            Permit::Granted
        );
    }

    #[test]
    fn a_zero_refill_channel_reports_an_unbounded_wait_rather_than_dividing_by_zero() {
        let clock = TestClock::at(0);
        let rl = limiter(clock.clone(), Quota::new(1, 0));
        let key = BucketKey::new("/order", "TR1");
        assert_eq!(rl.acquire(&key), Permit::Granted);
        assert_eq!(
            rl.acquire(&key),
            Permit::Throttled {
                retry_after_ms: u64::MAX
            }
        );
    }

    #[test]
    fn the_limiter_never_renders_anything_sensitive() {
        let rl = limiter(TestClock::at(0), Quota::new(1, 1));
        let rendered = format!("{rl:?}");
        assert!(rendered.contains("RateLimiter"));
        assert!(!rendered.contains("token"), "{rendered}");
    }
}
