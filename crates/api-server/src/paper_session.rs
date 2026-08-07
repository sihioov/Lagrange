//! Paper session settlement and its notifications (plan Todo 32).
//!
//! Todo 31 built the deterministic close→pending→next-open→close core and
//! the `pending_targets` claim/settle guard. This module is the seam where a
//! settled session becomes something a user is TOLD about: it settles the
//! target and routes exactly one severity-graded alert per session, recording
//! a durable delivery outcome for every channel it attempts.
//!
//! Grades follow design §15.3:
//!
//! - a session that executed and whose signals match its backtest → `INFO`
//!   (kind `job`, the completion notice);
//! - a session that executed but diverged from — or cannot be compared to —
//!   its backtest → `WARNING` (kind `alert`, the design's "Paper 불일치");
//! - a session the runner could not execute → `WARNING` when it was blocked
//!   (an entitlement pause, a missing close) and `CRITICAL` when it failed.
//!
//! The parity evaluation happens HERE, at settlement, and never on a read:
//! `GET .../parity` recomputes the report for display, but re-reading a
//! report must not manufacture new notifications.
//!
//! Announcing is deliberately separate from settling. `settle` is guarded on
//! `status = 'PENDING'`, so a second runner racing the same session gets
//! `NotFound` and never reaches the alert — one session yields one
//! notification even under a duplicate claim.

use auth::entitlement::Actor;
use result_model::paper_parity::{ParityReport, ParityStatus};
use uuid::Uuid;

use crate::error::TenancyResult;
use crate::http::state::ApiState;
use crate::notify::{AlertResult, AlertSeverity};
use crate::repos::pending_targets::PendingTargetRow;

/// What the runner observed for one session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionOutcome {
    /// The session's orders and fills are in the ledger.
    Executed,
    /// The runner deliberately did not trade this session (entitlement
    /// pause, missing close, every instrument below the rebalance
    /// threshold). The target settles `SKIPPED` and stays auditable.
    Blocked { reason: String },
    /// The runner could not complete the session. The target still settles
    /// `SKIPPED` — a PENDING row would be re-claimed forever — and the
    /// failure is escalated CRITICAL.
    Failed { reason: String },
}

impl SessionOutcome {
    /// The `pending_targets.status` this outcome settles to.
    fn settled_status(&self) -> &'static str {
        match self {
            Self::Executed => "EXECUTED",
            Self::Blocked { .. } | Self::Failed { .. } => "SKIPPED",
        }
    }
}

/// The result of settling and announcing one session.
#[derive(Debug, Clone)]
pub struct SettlementOutcome {
    pub target: PendingTargetRow,
    /// `None` only when the session never executed, so there are no Paper
    /// signals to compare.
    pub parity: Option<ParityReport>,
    pub severity: AlertSeverity,
    pub alerts: AlertResult,
}

/// Settles the target and routes its notification.
///
/// Returns `NotFound` when the target is not the actor's or is no longer
/// `PENDING`; in both cases nothing is announced.
pub async fn settle_and_announce(
    state: &ApiState,
    actor: &Actor,
    target_id: Uuid,
    outcome: SessionOutcome,
) -> TenancyResult<SettlementOutcome> {
    let target = state
        .pending_targets()
        .settle(actor, target_id, outcome.settled_status())
        .await?;

    let parity = match outcome {
        SessionOutcome::Executed => Some(
            state
                .parity_report(actor, target.account_id, &target.computed_on.to_string())
                .await?,
        ),
        _ => None,
    };

    let (severity, kind, title, body) = announcement(&target, &outcome, parity.as_ref());
    let alerts = state
        .notifier()
        .route_alert(actor, severity, kind, &title, &body)
        .await?;

    Ok(SettlementOutcome {
        target,
        parity,
        severity,
        alerts,
    })
}

/// Grades one settled session and writes its user-facing message.
///
/// The body always names the session and the strategy lineage the target
/// carried, so a reader of the feed alone can tell WHICH session and WHICH
/// data version the notice is about.
fn announcement(
    target: &PendingTargetRow,
    outcome: &SessionOutcome,
    parity: Option<&ParityReport>,
) -> (AlertSeverity, &'static str, String, String) {
    let session = target.effective_date;
    let dataset = target
        .dataset_version
        .clone()
        .unwrap_or_else(|| "unrecorded".to_owned());
    match outcome {
        SessionOutcome::Failed { reason } => (
            AlertSeverity::Critical,
            "alert",
            format!("Paper session {session} failed"),
            format!(
                "The paper session for {session} could not be executed: {reason}. No orders were \
                 placed and the target is settled SKIPPED."
            ),
        ),
        SessionOutcome::Blocked { reason } => (
            AlertSeverity::Warning,
            "alert",
            format!("Paper session {session} blocked"),
            format!(
                "The paper session for {session} did not trade: {reason}. The target is settled \
                 SKIPPED and remains auditable."
            ),
        ),
        SessionOutcome::Executed => match parity.map(|p| p.status) {
            Some(ParityStatus::Match) => (
                AlertSeverity::Info,
                "job",
                format!("Paper session {session} completed"),
                format!(
                    "The paper session for {session} executed its target from dataset {dataset}, \
                     and its signals match the backtest for the same close."
                ),
            ),
            Some(ParityStatus::Divergent) => (
                AlertSeverity::Warning,
                "alert",
                format!("Paper session {session} diverged from its backtest"),
                format!(
                    "The paper session for {session} executed, but its target weights differ from \
                     the backtest computed for the same close on dataset {dataset}. Review the \
                     parity report before acting on this account."
                ),
            ),
            _ => (
                AlertSeverity::Warning,
                "alert",
                format!("Paper session {session} cannot be compared to a backtest"),
                format!(
                    "The paper session for {session} executed, but its lineage does not match any \
                     backtest for the same close (dataset {dataset}). Parity cannot be claimed."
                ),
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use result_model::paper_parity::{LineageComparison, ParityReport};

    fn target() -> PendingTargetRow {
        PendingTargetRow {
            id: Uuid::nil(),
            account_id: Uuid::nil(),
            strategy_config_id: Uuid::nil(),
            computed_on: NaiveDate::from_ymd_opt(2026, 1, 30).expect("valid date"),
            effective_date: NaiveDate::from_ymd_opt(2026, 2, 2).expect("valid date"),
            targets_json: serde_json::json!([]),
            dataset_version: Some("krx-eod.2026-01-30".to_owned()),
            status: "EXECUTED".to_owned(),
            executed_at: None,
            created_at: chrono::Utc::now(),
        }
    }

    fn report(status: ParityStatus) -> ParityReport {
        ParityReport {
            status,
            lineage: LineageComparison { fields: Vec::new() },
            divergences: Vec::new(),
            fill_model_difference: String::new(),
        }
    }

    #[test]
    fn matching_session_is_an_info_completion_notice() {
        let (severity, kind, title, _) = announcement(
            &target(),
            &SessionOutcome::Executed,
            Some(&report(ParityStatus::Match)),
        );
        assert_eq!(severity, AlertSeverity::Info);
        assert_eq!(kind, "job");
        assert!(
            title.contains("2026-02-02"),
            "the session is named: {title}"
        );
    }

    #[test]
    fn divergent_session_is_a_warning() {
        let (severity, kind, _, body) = announcement(
            &target(),
            &SessionOutcome::Executed,
            Some(&report(ParityStatus::Divergent)),
        );
        assert_eq!(severity, AlertSeverity::Warning);
        assert_eq!(kind, "alert");
        assert!(
            body.contains("krx-eod.2026-01-30"),
            "the body names the dataset: {body}"
        );
    }

    #[test]
    fn incomparable_session_is_a_warning_not_a_completion() {
        let (severity, _, title, _) = announcement(
            &target(),
            &SessionOutcome::Executed,
            Some(&report(ParityStatus::NotComparable)),
        );
        assert_eq!(severity, AlertSeverity::Warning);
        assert!(
            title.contains("cannot be compared"),
            "an incomparable session never reads as a match: {title}"
        );
    }

    #[test]
    fn blocked_warns_and_failed_escalates() {
        let blocked = SessionOutcome::Blocked {
            reason: "entitlement paused".to_owned(),
        };
        let (severity, _, _, body) = announcement(&target(), &blocked, None);
        assert_eq!(severity, AlertSeverity::Warning);
        assert!(body.contains("entitlement paused"), "reason is carried");
        assert_eq!(blocked.settled_status(), "SKIPPED");

        let failed = SessionOutcome::Failed {
            reason: "open price missing".to_owned(),
        };
        let (severity, _, _, _) = announcement(&target(), &failed, None);
        assert_eq!(severity, AlertSeverity::Critical);
        assert_eq!(failed.settled_status(), "SKIPPED");
        assert_eq!(SessionOutcome::Executed.settled_status(), "EXECUTED");
    }
}
