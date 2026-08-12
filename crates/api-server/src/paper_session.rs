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

use std::path::Path;

use auth::entitlement::Actor;
use job_queue::paper_execution::{
    ExecutionOutcome, SessionInput, execute_session, execute_session_in_tx, targets_from_json,
};
use result_model::paper_parity::{ParityReport, ParityStatus};
use uuid::Uuid;

use crate::error::{TenancyError, TenancyResult};
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

/// Runs one queued session end to end: execute, then settle and announce.
///
/// This is the entry point a real caller uses. `settle_and_announce` below
/// records what a session DID; until now nothing produced that. The runner
/// daemon wraps this in a polling loop over `pending_targets::due`; a direct
/// call is the same path with the loop unrolled.
///
/// # Why the status guard is here and not in the engine
///
/// The target must still be `PENDING`. A `SKIPPED` target wrote nothing to the
/// ledger, so `execute_session`'s own "already executed?" check would let it
/// through and trade a terminally-settled session at the prices of whenever
/// this was called. The engine cannot make that judgement — it is handed a
/// session and executes it — and `plan_session_open`'s date guard cannot
/// either, since the session date it compares against is the target's own.
/// A non-`PENDING` target is reported as `NotFound`, exactly as a second
/// runner's `settle` reports it.
///
/// The engine's [`ExecutionOutcome::AlreadyExecuted`] covers the ONE case this
/// guard cannot: a crash between the engine's commit and the settle, which
/// leaves a `PENDING` target whose orders are already in the ledger.
///
/// `worker_pool` is a `worker`-role pool, not the API's `app` pool: the ledger
/// writes are the runner's, and this server has no worker connection of its
/// own to reach for.
pub async fn run_and_settle(
    state: &ApiState,
    worker_pool: &sqlx::PgPool,
    dataset_root: &Path,
    actor: &Actor,
    target_id: Uuid,
) -> TenancyResult<SettlementOutcome> {
    let target = state.pending_targets().get(actor, target_id).await?;
    if target.status != "PENDING" {
        return Err(TenancyError::NotFound);
    }

    // The owner is the ACTOR, never a column of the row: the read above only
    // returned it because RLS scoped it to this actor, so they are the same
    // user by construction — and the engine binds it as its tenancy predicate.
    let owner_user_id = crate::actor_tx::actor_uuid(actor)?;

    let outcome = match session_input(&target, owner_user_id) {
        Ok(input) => match execute_with_preflight(worker_pool, dataset_root, &target, &input).await
        {
            Ok(PreflightExecution::Skipped(reason)) => {
                return announce_preflight_skip(state, actor, target_id, reason).await;
            }
            Ok(PreflightExecution::NotPending) => return Err(TenancyError::NotFound),
            Ok(PreflightExecution::Outcome(
                ExecutionOutcome::Executed { .. } | ExecutionOutcome::AlreadyExecuted { .. },
            )) => SessionOutcome::Executed,
            Ok(PreflightExecution::Outcome(ExecutionOutcome::NoTrade)) => SessionOutcome::Blocked {
                reason: "no rebalance was needed: every instrument was inside the rebalance \
                         threshold or below the minimum trade size"
                    .to_owned(),
            },
            Err(e) => SessionOutcome::Failed { reason: e },
        },
        Err(reason) => SessionOutcome::Failed { reason },
    };

    settle_and_announce(state, actor, target_id, outcome).await
}

enum PreflightExecution {
    Outcome(ExecutionOutcome),
    Skipped(String),
    NotPending,
}

async fn execute_with_preflight(
    worker_pool: &sqlx::PgPool,
    dataset_root: &Path,
    target: &PendingTargetRow,
    input: &SessionInput,
) -> Result<PreflightExecution, String> {
    if target.dataset_version_id.is_none() || target.dataset_manifest_sha256.is_none() {
        return execute_session(worker_pool, dataset_root, input)
            .await
            .map(PreflightExecution::Outcome)
            .map_err(|error| error.to_string());
    }

    let mut tx = worker_pool
        .begin()
        .await
        .map_err(|error| error.to_string())?;
    let (authorized, reason): (bool, Option<serde_json::Value>) =
        sqlx::query_as("SELECT authorized, reason FROM public.preflight_paper_target($1, $2)")
            .bind(target.id)
            .bind(input.owner_user_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|error| error.to_string())?;
    if !authorized {
        tx.commit().await.map_err(|error| error.to_string())?;
        let code = reason
            .as_ref()
            .and_then(|value| value.get("code"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("PAPER_PREFLIGHT_DENIED");
        if code == "PAPER_TARGET_NOT_PENDING" {
            return Ok(PreflightExecution::NotPending);
        }
        let message = reason
            .as_ref()
            .and_then(|value| value.get("message"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("Paper execution preflight denied execution");
        return Ok(PreflightExecution::Skipped(format!("{code}: {message}")));
    }

    let execution = execute_session_in_tx(&mut tx, dataset_root, input)
        .await
        .map_err(|error| error.to_string());
    match execution {
        Ok(outcome @ ExecutionOutcome::Executed { .. }) => {
            tx.commit().await.map_err(|error| error.to_string())?;
            Ok(PreflightExecution::Outcome(outcome))
        }
        Ok(other) => {
            tx.rollback().await.map_err(|error| error.to_string())?;
            Ok(PreflightExecution::Outcome(other))
        }
        Err(error) => {
            tx.rollback()
                .await
                .map_err(|rollback| rollback.to_string())?;
            Err(error)
        }
    }
}

async fn announce_preflight_skip(
    state: &ApiState,
    actor: &Actor,
    target_id: Uuid,
    reason: String,
) -> TenancyResult<SettlementOutcome> {
    let target = state.pending_targets().get(actor, target_id).await?;
    if target.status != "SKIPPED" {
        return Err(TenancyError::NotFound);
    }
    let outcome = SessionOutcome::Blocked { reason };
    let (severity, kind, title, body) = announcement(&target, &outcome, None);
    let alerts = state
        .notifier()
        .route_alert(actor, severity, kind, &title, &body)
        .await?;
    Ok(SettlementOutcome {
        target,
        parity: None,
        severity,
        alerts,
    })
}

/// Turns a queued row into the engine's input, or says why it cannot.
fn session_input(target: &PendingTargetRow, owner_user_id: Uuid) -> Result<SessionInput, String> {
    let effective_date = domain::TradingDate::parse(&target.effective_date.to_string())
        .map_err(|e| format!("unreadable effective_date: {e}"))?;
    let targets = targets_from_json(&target.targets_json).map_err(|e| e.to_string())?;
    Ok(SessionInput {
        account_id: target.account_id,
        owner_user_id,
        effective_date,
        targets,
    })
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
    // `Executed` is a claim about the ledger, so it is checked against the
    // ledger.
    //
    // Its own documentation says "The session's orders and fills are in the
    // ledger", and nothing verified that. A caller could settle a session
    // EXECUTED and the user would be told INFO -- the completion notice --
    // while no order, no fill and no cash movement had ever been recorded. As
    // of this writing nothing in this server writes `orders`, `fills` or
    // `positions` at all, so every `Executed` in production would have been
    // exactly that: a completion notice for a session that did nothing.
    //
    // The floor is ONE ORDER, not one fill. A session that placed orders and
    // filled none still executed -- the runner did its job and the market did
    // not. A session that placed nothing did not execute, and the enum already
    // has the right word for it: `Blocked` covers the deliberate no-trade
    // cases (an entitlement pause, every instrument inside the rebalance
    // threshold), so `Executed` with zero orders is not a legitimate state.
    let target_peek = state.pending_targets().get(actor, target_id).await?;
    let outcome = match &outcome {
        SessionOutcome::Executed => {
            match ledger_evidence(state, actor, &target_peek).await? {
                true => outcome,
                // Downgraded rather than refused. Returning an error would
                // leave the row PENDING, and the enum's own comment explains
                // why that is the worst option: "a PENDING row would be
                // re-claimed forever". A runner claiming execution it cannot
                // evidence is broken, which is what CRITICAL is for.
                false => SessionOutcome::Failed {
                    reason: format!(
                        "settled EXECUTED but no order exists for account {} on {}; \
                         the session recorded nothing",
                        target_peek.account_id, target_peek.effective_date
                    ),
                },
            }
        }
        _ => outcome,
    };

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

/// Whether the ledger holds anything for this session.
///
/// Runs inside an ACTOR transaction: `orders` is under FORCE RLS (migration
/// 0010), so a bare pool sees zero rows and this would report "no evidence"
/// for every session including the ones that genuinely traded.
///
/// The session is identified by its account and its EFFECTIVE date -- the
/// trading date the target was for. `computed_on` is when the target was
/// calculated, which is the previous close, so matching on it would look for
/// orders a day before they could exist.
async fn ledger_evidence(
    state: &ApiState,
    actor: &Actor,
    target: &PendingTargetRow,
) -> TenancyResult<bool> {
    let mut tx = crate::actor_tx::begin_actor_tx(&state.app_pool, actor).await?;
    let exists: Option<(bool,)> = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM orders \
         WHERE account_id = $1 AND created_at::date = $2::date)",
    )
    .bind(target.account_id)
    .bind(target.effective_date)
    .fetch_optional(&mut *tx)
    .await
    .map_err(crate::error::TenancyError::from_sqlx)?;
    tx.commit()
        .await
        .map_err(crate::error::TenancyError::from_sqlx)?;
    Ok(exists.map(|(b,)| b).unwrap_or(false))
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
            dataset_version_id: None,
            dataset_manifest_sha256: None,
            non_execution_reason: None,
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
