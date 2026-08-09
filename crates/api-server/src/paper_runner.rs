//! One-cycle orchestration for the Paper worker daemon.
//!
//! The daemon is deliberately thin: target execution and settlement stay in
//! `paper_session`, while ledger valuation stays in `job_queue`. This module
//! only performs the trusted worker-wide scans and joins those two seams.

use std::path::PathBuf;

use auth::entitlement::{Actor, Role};
use chrono::NaiveDate;
use domain::TradingDate;
use job_queue::paper_valuation::{ValuationOutcome, value_account};
use thiserror::Error;
use uuid::Uuid;

use crate::error::TenancyError;
use crate::http::state::ApiState;
use crate::paper_session::run_and_settle;
use crate::repos::pending_targets::{PendingTargetRepo, WorkerPendingTargetRow};

/// Services the daemon needs for one cycle.
#[derive(Clone)]
pub struct RunnerServices {
    pub state: ApiState,
    pub worker_pool: sqlx::PgPool,
    pub dataset_root: PathBuf,
}

/// Parsed command-line controls shared by the daemon and its tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerArgs {
    pub once: bool,
    pub date: Option<NaiveDate>,
}

/// Parses arguments after the executable name.
pub fn parse_args<I>(args: I) -> Result<RunnerArgs, String>
where
    I: IntoIterator<Item = String>,
{
    let mut parsed = RunnerArgs {
        once: false,
        date: None,
    };
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--once" => {
                if parsed.once {
                    return Err("--once may be provided only once".to_owned());
                }
                parsed.once = true;
            }
            "--date" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--date requires YYYY-MM-DD".to_owned())?;
                if value.len() != 10 || value.as_bytes()[4] != b'-' || value.as_bytes()[7] != b'-' {
                    return Err(format!("invalid date {value:?}; expected YYYY-MM-DD"));
                }
                let date = NaiveDate::parse_from_str(&value, "%Y-%m-%d")
                    .map_err(|e| format!("invalid date {value:?}: {e}"))?;
                if parsed.date.replace(date).is_some() {
                    return Err("--date may be provided only once".to_owned());
                }
            }
            other => return Err(format!("unrecognised argument {other:?} (try --help)")),
        }
    }
    Ok(parsed)
}

impl RunnerServices {
    pub fn new(state: ApiState, worker_pool: sqlx::PgPool, dataset_root: PathBuf) -> Self {
        Self {
            state,
            worker_pool,
            dataset_root,
        }
    }
}

/// Per-item error recorded while the rest of a cycle continues.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerItemError {
    pub kind: &'static str,
    pub resource_id: Uuid,
    pub detail: String,
}

/// What one worker cycle observed and completed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CycleReport {
    pub targets_seen: usize,
    pub targets_settled: usize,
    pub valuations_seen: usize,
    pub valuations_written: usize,
    pub item_errors: Vec<RunnerItemError>,
}

/// Errors that prevent a cycle-wide scan from being completed.
#[derive(Debug, Error)]
pub enum RunnerError {
    #[error("pending target scan failed: {0}")]
    PendingScan(#[from] TenancyError),

    #[error("Paper account scan failed: {0}")]
    AccountScan(#[from] sqlx::Error),

    #[error("invalid Paper processing date: {0}")]
    InvalidDate(String),
}

/// Runs one deterministic processing date for every due target and active
/// Paper account.
pub async fn run_cycle(
    services: &RunnerServices,
    process_date: NaiveDate,
) -> Result<CycleReport, RunnerError> {
    let targets = PendingTargetRepo::due_worker(&services.worker_pool, process_date).await?;
    let accounts = active_paper_accounts(&services.worker_pool).await?;
    let date = TradingDate::parse(&process_date.to_string())
        .map_err(|e| RunnerError::InvalidDate(format!("{process_date}: {e}")))?;

    let mut report = CycleReport {
        targets_seen: targets.len(),
        valuations_seen: accounts.len(),
        ..CycleReport::default()
    };

    for target in targets {
        settle_target(services, target, &mut report).await;
    }

    for account in accounts {
        match value_account(
            &services.worker_pool,
            &services.dataset_root,
            account.id,
            account.owner_user_id,
            date,
        )
        .await
        {
            Ok(ValuationOutcome::Valued { .. } | ValuationOutcome::AlreadyValued) => {
                report.valuations_written += 1;
            }
            Err(error) => report.item_errors.push(RunnerItemError {
                kind: "valuation",
                resource_id: account.id,
                detail: error.to_string(),
            }),
        }
    }

    Ok(report)
}

async fn settle_target(
    services: &RunnerServices,
    target: WorkerPendingTargetRow,
    report: &mut CycleReport,
) {
    let actor = match owner_actor(&services.worker_pool, target.owner_user_id).await {
        Ok(actor) => actor,
        Err(error) => {
            report.item_errors.push(RunnerItemError {
                kind: "target",
                resource_id: target.id,
                detail: format!("owner role lookup failed: {error}"),
            });
            return;
        }
    };
    match run_and_settle(
        &services.state,
        &services.worker_pool,
        &services.dataset_root,
        &actor,
        target.id,
    )
    .await
    {
        Ok(_) => report.targets_settled += 1,
        Err(TenancyError::NotFound) => {
            // Another replica won the status guard. This is a normal restart
            // race and must not create a second alert.
        }
        Err(error) => report.item_errors.push(RunnerItemError {
            kind: "target",
            resource_id: target.id,
            detail: error.to_string(),
        }),
    }
}

#[derive(Debug, Clone, Copy, sqlx::FromRow)]
struct WorkerPaperAccount {
    id: Uuid,
    owner_user_id: Uuid,
}

async fn active_paper_accounts(
    pool: &sqlx::PgPool,
) -> Result<Vec<WorkerPaperAccount>, sqlx::Error> {
    sqlx::query_as::<_, WorkerPaperAccount>(
        "SELECT id, owner_user_id FROM accounts \
         WHERE account_type = 'PAPER' AND status = 'ACTIVE' \
         ORDER BY owner_user_id, id",
    )
    .fetch_all(pool)
    .await
}

async fn owner_actor(pool: &sqlx::PgPool, user_id: Uuid) -> Result<Actor, sqlx::Error> {
    let is_owner: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM user_roles WHERE user_id = $1 AND role_id = 'owner')",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    Ok(Actor::new(
        user_id.to_string(),
        if is_owner { Role::Owner } else { Role::Member },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_report_starts_empty() {
        let report = CycleReport::default();
        assert_eq!(report.targets_seen, 0);
        assert_eq!(report.valuations_written, 0);
        assert!(report.item_errors.is_empty());
    }
}
