//! Deterministic clock seam for auth logic.
//!
//! All session/invite/step-up freshness decisions use an injected [`Clock`] so
//! tests freeze time instead of sleeping; production uses [`SystemClock`].

use std::time::{SystemTime, UNIX_EPOCH};

pub trait Clock: Send + Sync {
    fn now_epoch_secs(&self) -> i64;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_epoch_secs(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64
    }
}

/// Test double: a clock frozen at an explicit epoch second.
#[derive(Debug, Clone, Copy)]
pub struct FakeClock(pub i64);

impl Clock for FakeClock {
    fn now_epoch_secs(&self) -> i64 {
        self.0
    }
}

impl FakeClock {
    pub fn advance(&mut self, secs: i64) {
        self.0 += secs;
    }
}
