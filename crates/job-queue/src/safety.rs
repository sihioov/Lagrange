//! Fail-closed capacity and readiness gates for the backtest runner.
//!
//! A backtest is allowed to produce a large immutable generation before its
//! queue transaction commits.  Claiming work while the artifact volume is
//! nearly full, or while the database cannot be consulted, turns a recoverable
//! outage into a publication outage.  [`ClaimGate`] keeps that decision in one
//! small, testable object shared by the daemon and its readiness probe.

use crate::queue::JobQueue;
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// Stable diagnostic/event code emitted when claims are blocked by capacity or
/// an unavailable authoritative database read.
pub const BACKTEST_BACKPRESSURE_EVENT_CODE: &str = "BACKTEST_CLAIM_BACKPRESSURE";

/// Conservative non-production default. Production must provide the value
/// explicitly through the daemon's configuration parser.
pub const DEFAULT_MIN_FREE_BYTES: u64 = 1_073_741_824;

/// Conservative non-production queued-job high-water mark. Production must
/// provide the value explicitly through the daemon's configuration parser.
pub const DEFAULT_MAX_QUEUED_BACKTESTS: i64 = 1_000;

/// How much capacity must remain before a new backtest may be claimed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackpressureConfig {
    pub min_free_bytes: u64,
    pub max_queued_backtests: i64,
}

impl BackpressureConfig {
    pub fn validate(self) -> Result<Self, String> {
        if self.min_free_bytes == 0 {
            return Err("minimum free artifact bytes must be greater than zero".to_owned());
        }
        if self.max_queued_backtests <= 0 {
            return Err("maximum queued backtests must be greater than zero".to_owned());
        }
        Ok(self)
    }
}

/// A single capacity observation. Missing observations are intentionally not
/// treated as zero or infinity: an unavailable disk or DB read is not safe to
/// claim against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapacitySnapshot {
    pub database_available: bool,
    pub free_bytes: Option<u64>,
    pub queued_backtests: Option<i64>,
    pub ready: bool,
    pub reason: String,
}

impl CapacitySnapshot {
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            database_available: false,
            free_bytes: None,
            queued_backtests: None,
            ready: false,
            reason: reason.into(),
        }
    }

    pub fn from_observations(
        config: BackpressureConfig,
        free_bytes: Option<u64>,
        queued_backtests: Option<i64>,
    ) -> Self {
        let config_valid = config.validate().is_ok();
        let database_available = queued_backtests.is_some();
        let disk_ok = free_bytes.is_some_and(|free| free >= config.min_free_bytes);
        let backlog_ok = queued_backtests
            .is_some_and(|queued| queued >= 0 && queued < config.max_queued_backtests);
        let ready = config_valid && database_available && disk_ok && backlog_ok;
        let reason = if !config_valid {
            "invalid backpressure configuration"
        } else if !database_available {
            "authoritative queue read unavailable"
        } else if !disk_ok {
            "artifact volume is below the free-space threshold"
        } else if !backlog_ok {
            "queued backtest backlog is at or above the threshold"
        } else {
            "ready"
        };
        Self {
            database_available,
            free_bytes,
            queued_backtests,
            ready,
            reason: reason.to_owned(),
        }
    }

    pub fn allows_claim(&self) -> bool {
        self.ready
    }
}

/// Shared gate consulted immediately before a queue claim.
#[derive(Clone)]
pub struct ClaimGate {
    config: BackpressureConfig,
    snapshot: Arc<RwLock<CapacitySnapshot>>,
}

impl ClaimGate {
    pub fn new(config: BackpressureConfig) -> Result<Self, String> {
        let config = config.validate()?;
        Ok(Self {
            config,
            snapshot: Arc::new(RwLock::new(CapacitySnapshot::unavailable(
                "capacity has not been measured yet",
            ))),
        })
    }

    pub fn config(&self) -> BackpressureConfig {
        self.config
    }

    pub fn snapshot(&self) -> CapacitySnapshot {
        self.snapshot
            .read()
            .expect("claim gate lock must not be poisoned")
            .clone()
    }

    pub fn allows_claim(&self) -> bool {
        self.snapshot().allows_claim()
    }

    pub fn set_snapshot(&self, snapshot: CapacitySnapshot) {
        *self
            .snapshot
            .write()
            .expect("claim gate lock must not be poisoned") = snapshot;
    }

    /// Probe both authorities. Any failure leaves the gate unready. This is
    /// deliberately best-effort: callers keep the diagnostic snapshot and
    /// retry on the next periodic tick rather than claiming with stale data.
    pub async fn refresh(&self, queue: &JobQueue, artifact_root: &Path) -> CapacitySnapshot {
        let free = available_bytes(artifact_root).ok();
        let queued = queue.backtest_queued_count().await.ok();
        let snapshot = CapacitySnapshot::from_observations(self.config, free, queued);
        self.set_snapshot(snapshot.clone());
        snapshot
    }

    /// Mark a database or filesystem probe as unavailable without exposing a
    /// stale ready snapshot to the claim loop.
    pub fn fail_closed(&self, reason: impl Into<String>) -> CapacitySnapshot {
        let snapshot = CapacitySnapshot::unavailable(reason);
        self.set_snapshot(snapshot.clone());
        snapshot
    }
}

/// Return bytes available to an unprivileged process on the configured
/// artifact filesystem. `f_bavail`, rather than total free blocks, matches
/// what the runner can actually allocate under its UID.
pub fn available_bytes(path: &Path) -> std::io::Result<u64> {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let bytes = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "artifact path contains NUL",
            )
        })?;
        // SAFETY: `statvfs` writes only to the initialized local struct and
        // the C string is NUL terminated for the duration of the call.
        let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        let result = unsafe { libc::statvfs(bytes.as_ptr(), stats.as_mut_ptr()) };
        if result != 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: a zero return from statvfs initialized the struct.
        let stats = unsafe { stats.assume_init() };
        stats
            .f_bavail
            .checked_mul(stats.f_frsize)
            .ok_or_else(|| std::io::Error::other("filesystem free-space value overflowed"))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "filesystem free-space probing is unsupported on this platform",
        ))
    }
}

/// How often the daemon refreshes capacity and runs retention.
pub const DEFAULT_RECONCILE_INTERVAL: Duration = Duration::from_secs(60);

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> BackpressureConfig {
        BackpressureConfig {
            min_free_bytes: 100,
            max_queued_backtests: 10,
        }
    }

    #[test]
    fn claims_are_blocked_until_both_authorities_are_healthy() {
        let gate = ClaimGate::new(config()).unwrap();
        assert!(!gate.allows_claim());
        let ready = CapacitySnapshot::from_observations(config(), Some(101), Some(9));
        gate.set_snapshot(ready);
        assert!(gate.allows_claim());
        gate.set_snapshot(CapacitySnapshot::from_observations(
            config(),
            Some(99),
            Some(9),
        ));
        assert!(!gate.allows_claim());
        gate.set_snapshot(CapacitySnapshot::from_observations(
            config(),
            Some(101),
            Some(10),
        ));
        assert!(!gate.allows_claim());
        gate.fail_closed("database unavailable");
        assert!(!gate.allows_claim());
    }

    #[test]
    fn invalid_configuration_fails_closed() {
        assert!(
            BackpressureConfig {
                min_free_bytes: 0,
                max_queued_backtests: 1,
            }
            .validate()
            .is_err()
        );
        assert!(
            BackpressureConfig {
                min_free_bytes: 1,
                max_queued_backtests: 0,
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn database_unavailable_is_unready_and_stops_claims() {
        let gate = ClaimGate::new(config()).unwrap();
        let snapshot = CapacitySnapshot::from_observations(config(), Some(u64::MAX), None);
        gate.set_snapshot(snapshot.clone());
        assert!(!snapshot.ready);
        assert!(!gate.allows_claim());
        assert!(snapshot.reason.contains("authoritative queue"));
    }
}
