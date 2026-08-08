//! WebSocket session lifecycle and reconnect (plan Todo 36, design §6.12).
//!
//! Design rule: "재연결 후 미체결·체결·잔고를 전체 조회해 대사한다." A
//! reconnect is NOT a resume. The stream offers no replay guarantee, so
//! anything that happened while the socket was down is simply absent — and a
//! session that came back and carried on would silently miss fills it never
//! saw. The only sound recovery is to re-query open orders, executions, and
//! balances in full and reconcile.
//!
//! [`ReconnectPolicy`] therefore produces a [`ResyncRequired`] marker on every
//! successful reconnect, and the marker is not optional or advisory: the
//! session refuses to be considered live until the caller acknowledges the
//! resync. A "reconnected, resync pending" state that could be read as healthy
//! is the failure mode this exists to prevent.
//!
//! Backoff is exponential with a cap so a broker outage does not turn into a
//! reconnect storm, and attempts are counted so an operator can see a flapping
//! link rather than only its current state.

use crate::error::KisError;

/// Where a subscription session is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionState {
    /// Never connected, or deliberately closed.
    Disconnected,
    /// Socket is up AND the post-reconnect reconciliation is done.
    Live,
    /// Socket is up but the full re-query has not been acknowledged. NOT
    /// healthy: execution reports received now cannot be trusted as complete.
    ResyncPending,
    /// Backing off before the next attempt.
    Reconnecting { attempt: u32, wait_ms: u64 },
}

impl SessionState {
    /// Whether execution data from this session may be treated as complete.
    ///
    /// `ResyncPending` deliberately answers false. That is the whole point of
    /// distinguishing it from `Live`.
    pub fn is_trustworthy(&self) -> bool {
        matches!(self, Self::Live)
    }
}

/// Returned on every successful reconnect. Must be discharged by a full
/// re-query before the session counts as live.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use = "a reconnect requires a FULL re-query of open orders, executions, and balances; \
              ignoring this silently drops everything that happened while the socket was down"]
pub struct ResyncRequired {
    /// Which reconnect produced it, for the audit trail.
    pub attempt: u32,
}

/// Reconnect timing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReconnectPolicy {
    pub initial_backoff_ms: u64,
    pub max_backoff_ms: u64,
    /// `None` means keep trying. A market session should not give up on its
    /// own during the trading day; giving up is an operator decision.
    pub max_attempts: Option<u32>,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            initial_backoff_ms: 500,
            max_backoff_ms: 30_000,
            max_attempts: None,
        }
    }
}

impl ReconnectPolicy {
    pub fn backoff_ms(&self, attempt: u32) -> u64 {
        if attempt == 0 {
            return 0;
        }
        let exp = attempt.saturating_sub(1).min(16);
        self.initial_backoff_ms
            .saturating_mul(1u64 << exp)
            .min(self.max_backoff_ms)
    }
}

/// Tracks one subscription session across disconnects.
#[derive(Debug, Clone)]
pub struct WebSocketSession {
    policy: ReconnectPolicy,
    state: SessionState,
    attempt: u32,
    /// Total reconnects since construction, so a flapping link is visible
    /// rather than only its current state.
    total_reconnects: u64,
}

impl WebSocketSession {
    pub fn new(policy: ReconnectPolicy) -> Self {
        Self {
            policy,
            state: SessionState::Disconnected,
            attempt: 0,
            total_reconnects: 0,
        }
    }

    pub fn state(&self) -> &SessionState {
        &self.state
    }

    pub fn total_reconnects(&self) -> u64 {
        self.total_reconnects
    }

    /// The first connection. No resync is required: there is no gap behind it.
    pub fn connected_first_time(&mut self) {
        self.attempt = 0;
        self.state = SessionState::Live;
    }

    /// The socket dropped.
    pub fn disconnected(&mut self) {
        self.attempt = 0;
        self.state = SessionState::Reconnecting {
            attempt: 1,
            wait_ms: self.policy.backoff_ms(1),
        };
        self.attempt = 1;
    }

    /// A reconnect attempt failed; schedule the next.
    pub fn attempt_failed(&mut self) -> Result<u64, KisError> {
        if let Some(max) = self.policy.max_attempts
            && self.attempt >= max
        {
            self.state = SessionState::Disconnected;
            return Err(KisError::Connect {
                reason: format!("gave up after {} reconnect attempts", self.attempt),
            });
        }
        self.attempt = self.attempt.saturating_add(1);
        let wait_ms = self.policy.backoff_ms(self.attempt);
        self.state = SessionState::Reconnecting {
            attempt: self.attempt,
            wait_ms,
        };
        Ok(wait_ms)
    }

    /// A reconnect succeeded. The session is NOT live yet.
    pub fn reconnected(&mut self) -> ResyncRequired {
        let attempt = self.attempt;
        self.total_reconnects = self.total_reconnects.saturating_add(1);
        self.attempt = 0;
        self.state = SessionState::ResyncPending;
        ResyncRequired { attempt }
    }

    /// Acknowledge that the full re-query completed. Only now is the session
    /// live, and only now may its execution data be treated as complete.
    pub fn resync_completed(&mut self, _proof: ResyncRequired) {
        self.state = SessionState::Live;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_first_connection_needs_no_resync() {
        let mut s = WebSocketSession::new(ReconnectPolicy::default());
        s.connected_first_time();
        assert_eq!(s.state(), &SessionState::Live);
        assert!(s.state().is_trustworthy());
        assert_eq!(s.total_reconnects(), 0);
    }

    #[test]
    fn a_reconnect_is_not_live_until_the_resync_is_acknowledged() {
        // The core property: a session that came back and carried on would
        // silently miss every fill that happened while it was down.
        let mut s = WebSocketSession::new(ReconnectPolicy::default());
        s.connected_first_time();
        s.disconnected();
        assert!(!s.state().is_trustworthy());

        let resync = s.reconnected();
        assert_eq!(s.state(), &SessionState::ResyncPending);
        assert!(
            !s.state().is_trustworthy(),
            "reports received before reconciliation cannot be trusted as complete"
        );

        s.resync_completed(resync);
        assert_eq!(s.state(), &SessionState::Live);
        assert!(s.state().is_trustworthy());
        assert_eq!(s.total_reconnects(), 1);
    }

    #[test]
    fn backoff_grows_and_caps() {
        let p = ReconnectPolicy::default();
        assert_eq!(p.backoff_ms(0), 0);
        assert_eq!(p.backoff_ms(1), 500);
        assert_eq!(p.backoff_ms(2), 1_000);
        assert_eq!(p.backoff_ms(3), 2_000);
        // A broker outage must not become a reconnect storm.
        assert_eq!(p.backoff_ms(99), p.max_backoff_ms);
    }

    #[test]
    fn failed_attempts_back_off_further_each_time() {
        let mut s = WebSocketSession::new(ReconnectPolicy::default());
        s.connected_first_time();
        s.disconnected();

        let first = s.attempt_failed().expect("keeps trying");
        let second = s.attempt_failed().expect("keeps trying");
        let third = s.attempt_failed().expect("keeps trying");
        assert!(first < second && second < third, "{first} {second} {third}");
    }

    #[test]
    fn an_unbounded_policy_never_gives_up_on_its_own() {
        // Abandoning a market session mid-day is an operator decision.
        let mut s = WebSocketSession::new(ReconnectPolicy::default());
        s.disconnected();
        for _ in 0..100 {
            assert!(s.attempt_failed().is_ok());
        }
    }

    #[test]
    fn a_bounded_policy_reports_giving_up_as_a_typed_error() {
        let mut s = WebSocketSession::new(ReconnectPolicy {
            max_attempts: Some(2),
            ..ReconnectPolicy::default()
        });
        s.disconnected();
        s.attempt_failed().expect("second attempt");
        let err = s.attempt_failed().expect_err("gives up");
        assert!(matches!(err, KisError::Connect { .. }));
        assert_eq!(s.state(), &SessionState::Disconnected);
    }

    #[test]
    fn a_flapping_link_is_visible_in_the_reconnect_count() {
        // Current state alone cannot distinguish "stable" from "reconnected
        // forty times in ten minutes".
        let mut s = WebSocketSession::new(ReconnectPolicy::default());
        s.connected_first_time();
        for _ in 0..5 {
            s.disconnected();
            let r = s.reconnected();
            s.resync_completed(r);
        }
        assert_eq!(s.total_reconnects(), 5);
        assert!(s.state().is_trustworthy());
    }

    #[test]
    fn the_resync_marker_records_which_attempt_produced_it() {
        let mut s = WebSocketSession::new(ReconnectPolicy::default());
        s.disconnected();
        s.attempt_failed().expect("retry");
        let r = s.reconnected();
        assert!(r.attempt >= 1, "the audit trail needs the attempt number");
    }
}
