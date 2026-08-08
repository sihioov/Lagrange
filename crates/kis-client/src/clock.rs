//! Injectable time (plan Todo 36).
//!
//! Token expiry, bucket refill, and clock-skew detection are all time-driven.
//! Testing them against the wall clock would mean sleeping — slow, and flaky on
//! a loaded host — so time is a dependency here, exactly as the ledger treats
//! trading dates. [`TestClock`] advances by hand, so a token-expiry or
//! rate-limit test is deterministic and instant.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// A source of the current UNIX time in milliseconds.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}

/// The real clock.
#[derive(Debug, Clone, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            // A clock before the epoch is not something to paper over with a
            // default; it is exactly the condition ClockSkew exists to catch,
            // and 0 makes the skew obvious rather than plausible.
            .unwrap_or(0)
    }
}

/// A clock the test drives.
#[derive(Debug, Clone, Default)]
pub struct TestClock {
    now: Arc<AtomicI64>,
}

impl TestClock {
    pub fn at(ms: i64) -> Self {
        Self {
            now: Arc::new(AtomicI64::new(ms)),
        }
    }

    pub fn advance_ms(&self, delta: i64) {
        self.now.fetch_add(delta, Ordering::SeqCst);
    }
}

impl Clock for TestClock {
    fn now_ms(&self) -> i64 {
        self.now.load(Ordering::SeqCst)
    }
}

/// How far the local clock may differ from the broker's before signed requests
/// stop being accepted. KIS rejects badly skewed requests, and retrying cannot
/// fix a wrong clock — so the adapter detects it and says so.
pub const DEFAULT_SKEW_LIMIT_SECS: i64 = 30;

/// Compare local time against a broker-supplied timestamp.
///
/// Returns `Ok(skew_secs)` when within the limit, so callers can record the
/// drift even on the healthy path; the value is signed (positive = local clock
/// ahead) because "which way" is the first question during an incident.
pub fn check_skew(
    local_ms: i64,
    broker_ms: i64,
    limit_secs: i64,
) -> Result<i64, crate::error::KisError> {
    let skew_secs = (local_ms - broker_ms) / 1000;
    if skew_secs.abs() > limit_secs {
        return Err(crate::error::KisError::ClockSkew {
            skew_secs,
            limit_secs,
        });
    }
    Ok(skew_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_test_clock_only_moves_when_told_to() {
        let c = TestClock::at(1_000);
        assert_eq!(c.now_ms(), 1_000);
        c.advance_ms(500);
        assert_eq!(c.now_ms(), 1_500);
    }

    #[test]
    fn skew_within_the_limit_reports_the_drift_rather_than_failing() {
        // Recording drift on the healthy path is what makes a later failure
        // diagnosable instead of surprising.
        let ok = check_skew(10_000, 5_000, 30).expect("5s is within 30s");
        assert_eq!(ok, 5);
        let behind = check_skew(5_000, 10_000, 30).expect("-5s is within 30s");
        assert_eq!(behind, -5, "the sign says which way the clock is wrong");
    }

    #[test]
    fn skew_beyond_the_limit_is_a_typed_error_in_both_directions() {
        for (local, broker) in [(100_000_i64, 0_i64), (0, 100_000)] {
            let err = check_skew(local, broker, 30).expect_err("100s exceeds 30s");
            assert!(matches!(err, crate::error::KisError::ClockSkew { .. }));
            // Never retried: repeating a request cannot correct a clock.
            assert!(!err.is_retryable(crate::error::RequestKind::Read));
        }
    }
}
