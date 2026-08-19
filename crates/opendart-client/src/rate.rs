//! The 1-request-per-second ceiling.
//!
//! Single-flight and the rate ceiling share one `tokio::sync::Mutex` in
//! `client`: the lock is held for the full lifetime of a request, including
//! retries, so only one request is ever in flight at a time, and the same
//! lock guards the timestamp of the last request used by the gate below.
//! That keeps both invariants enforced by one mechanism instead of two that
//! could drift apart.

use std::time::{Duration, Instant};

/// How long to wait before the next request may go out, given when the
/// last one was sent. Pure and `Instant`-based so it is unit-testable
/// without sleeping: construct two `Instant`s (`Instant::now()` plus a
/// `Duration` offset) and call this directly, as the tests below do.
pub fn wait_duration(last_sent: Option<Instant>, now: Instant, min_interval: Duration) -> Duration {
    match last_sent {
        None => Duration::ZERO,
        Some(last) => {
            let elapsed = now.saturating_duration_since(last);
            min_interval.saturating_sub(elapsed)
        }
    }
}

/// Mutable state guarded by the client's single-flight mutex.
#[derive(Debug, Default)]
pub struct RateState {
    pub last_sent: Option<Instant>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_request_never_waits() {
        let now = Instant::now();
        assert_eq!(
            wait_duration(None, now, Duration::from_secs(1)),
            Duration::ZERO
        );
    }

    #[test]
    fn request_within_window_waits_the_remainder() {
        let last = Instant::now();
        let now = last + Duration::from_millis(300);
        let wait = wait_duration(Some(last), now, Duration::from_secs(1));
        assert_eq!(wait, Duration::from_millis(700));
    }

    #[test]
    fn request_exactly_at_the_boundary_does_not_wait() {
        let last = Instant::now();
        let now = last + Duration::from_secs(1);
        let wait = wait_duration(Some(last), now, Duration::from_secs(1));
        assert_eq!(wait, Duration::ZERO);
    }

    #[test]
    fn request_after_window_does_not_wait() {
        let last = Instant::now();
        let now = last + Duration::from_secs(2);
        let wait = wait_duration(Some(last), now, Duration::from_secs(1));
        assert_eq!(wait, Duration::ZERO);
    }
}
