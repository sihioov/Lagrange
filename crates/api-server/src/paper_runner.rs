//! One-cycle orchestration for the Paper worker daemon.
//!
//! The daemon is deliberately thin: target execution and settlement stay in
//! `paper_session`, while ledger valuation stays in `job_queue`. This module
//! only performs the trusted worker-wide scans and joins those two seams.

use std::future::Future;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use auth::entitlement::{Actor, Role};
use chrono::NaiveDate;
use domain::TradingDate;
use job_queue::paper_execution::set_paper_transaction_timeouts;
use job_queue::paper_preview::{PreviewRunOutcome, PreviewRunnerError, run_preview_once};
use job_queue::paper_valuation::{ValuationOutcome, value_account};
use job_queue::{JobQueue, QueueConfig};
use thiserror::Error;
use tokio::sync::watch;
use uuid::Uuid;

use crate::error::TenancyError;
use crate::http::state::ApiState;
use crate::paper_session::run_and_settle;
use crate::repos::pending_targets::{
    PaperSettlementBacklog, PendingTargetRepo, WorkerPendingTargetRow,
};

/// The maximum time a single Paper database/settlement/valuation stage may
/// occupy the runner.  PostgreSQL has a matching local statement timeout;
/// this application deadline also covers pool acquisition and filesystem
/// work around the query.
pub const DEFAULT_OPERATION_DEADLINE: Duration = Duration::from_secs(15);
/// A cycle is finite even when a database or a blocking preview task stops
/// making progress.  The daemon can therefore honor SIGTERM at a known
/// boundary instead of waiting for an unbounded cycle.
pub const DEFAULT_CYCLE_DEADLINE: Duration = Duration::from_secs(90);
/// The process-level shutdown budget is intentionally shorter than the
/// systemd/Docker stop windows documented by the deployment files.
pub const DEFAULT_SHUTDOWN_GRACE: Duration = Duration::from_secs(20);

/// Services the daemon needs for one cycle.
#[derive(Clone)]
pub struct RunnerServices {
    pub state: ApiState,
    pub worker_pool: sqlx::PgPool,
    pub dataset_root: PathBuf,
    preview_queue: JobQueue,
    preview_worker_id: String,
    preview_heartbeat: Duration,
    operation_deadline: Duration,
    cycle_deadline: Duration,
}

/// Parsed command-line controls shared by the daemon and its tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerArgs {
    pub once: bool,
    pub date: Option<NaiveDate>,
    pub preview_worker_id: String,
    pub preview_heartbeat: Duration,
    pub preview_lease: Duration,
    pub preview_backoff: Duration,
    pub operation_deadline: Duration,
    pub cycle_deadline: Duration,
    pub shutdown_grace: Duration,
}

/// Parses arguments after the executable name.
pub fn parse_args<I>(args: I) -> Result<RunnerArgs, String>
where
    I: IntoIterator<Item = String>,
{
    let mut parsed = RunnerArgs {
        once: false,
        date: None,
        preview_worker_id: format!("paper-preview-{}", std::process::id()),
        preview_heartbeat: Duration::from_secs(10),
        preview_lease: Duration::from_secs(60),
        preview_backoff: Duration::from_secs(30),
        operation_deadline: DEFAULT_OPERATION_DEADLINE,
        cycle_deadline: DEFAULT_CYCLE_DEADLINE,
        shutdown_grace: DEFAULT_SHUTDOWN_GRACE,
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
            "--preview-worker-id" => {
                parsed.preview_worker_id = iter
                    .next()
                    .ok_or_else(|| "--preview-worker-id requires a value".to_owned())?;
            }
            "--preview-heartbeat-ms" => {
                parsed.preview_heartbeat = parse_duration(&mut iter, &arg)?;
            }
            "--preview-lease-ms" => {
                parsed.preview_lease = parse_duration(&mut iter, &arg)?;
            }
            "--preview-backoff-ms" => {
                parsed.preview_backoff = parse_duration(&mut iter, &arg)?;
            }
            "--operation-timeout-ms" => {
                parsed.operation_deadline = parse_duration(&mut iter, &arg)?;
            }
            "--cycle-timeout-ms" => {
                parsed.cycle_deadline = parse_duration(&mut iter, &arg)?;
            }
            "--shutdown-grace-ms" => {
                parsed.shutdown_grace = parse_duration(&mut iter, &arg)?;
            }
            other => return Err(format!("unrecognised argument {other:?} (try --help)")),
        }
    }
    if parsed.preview_worker_id.trim().is_empty() {
        return Err("--preview-worker-id must not be empty".to_owned());
    }
    if parsed.preview_heartbeat >= parsed.preview_lease {
        return Err("preview heartbeat must be shorter than the preview lease".to_owned());
    }
    if parsed.cycle_deadline < parsed.operation_deadline {
        return Err("cycle timeout must not be shorter than operation timeout".to_owned());
    }
    Ok(parsed)
}

fn parse_duration<I>(iter: &mut I, option: &str) -> Result<Duration, String>
where
    I: Iterator<Item = String>,
{
    let raw = iter
        .next()
        .ok_or_else(|| format!("{option} requires a positive millisecond value"))?;
    let millis = raw
        .parse::<u64>()
        .map_err(|_| format!("{option} requires a positive millisecond value"))?;
    if millis == 0 {
        return Err(format!("{option} requires a positive millisecond value"));
    }
    Ok(Duration::from_millis(millis))
}

impl RunnerServices {
    pub fn new(state: ApiState, worker_pool: sqlx::PgPool, dataset_root: PathBuf) -> Self {
        let preview_queue = JobQueue::new(worker_pool.clone(), None, QueueConfig::default());
        Self {
            state,
            worker_pool,
            dataset_root,
            preview_queue,
            preview_worker_id: format!("paper-preview-{}", std::process::id()),
            preview_heartbeat: Duration::from_secs(10),
            operation_deadline: DEFAULT_OPERATION_DEADLINE,
            cycle_deadline: DEFAULT_CYCLE_DEADLINE,
        }
    }

    pub fn with_preview_worker(
        mut self,
        worker_id: String,
        heartbeat: Duration,
        lease: Duration,
        backoff: Duration,
    ) -> Self {
        self.preview_queue = JobQueue::new(
            self.worker_pool.clone(),
            None,
            QueueConfig {
                lease,
                backoff_base: backoff,
            },
        );
        self.preview_worker_id = worker_id;
        self.preview_heartbeat = heartbeat;
        self
    }

    /// Override the finite operation/cycle budgets in deterministic tests or
    /// an explicitly configured deployment.  The defaults are production-safe
    /// and remain in force for every caller that does not opt in.
    pub fn with_deadlines(mut self, operation: Duration, cycle: Duration) -> Self {
        self.operation_deadline = operation;
        self.cycle_deadline = cycle;
        self
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
    pub previews_seen: usize,
    pub previews_published: usize,
    pub previews_failed: usize,
    pub preview_outcome: &'static str,
    pub preview_compute_ms: u128,
    pub targets_seen: usize,
    pub targets_settled: usize,
    pub valuations_seen: usize,
    pub valuations_written: usize,
    /// Durable settlement intents observed by the recovery scan.
    pub notifications_seen: usize,
    /// Intents whose recipient rows were persisted during this cycle.
    pub notifications_delivered: usize,
    /// Intents left pending after a bounded dispatch failure.
    pub notifications_pending: usize,
    /// Queue health snapshot after the recovery pass.
    pub notification_backlog: i64,
    pub notification_oldest_age_secs: i64,
    pub notification_failed: i64,
    pub notification_exhausted: i64,
    pub notification_ready: bool,
    pub item_errors: Vec<RunnerItemError>,
}

/// Errors that prevent a cycle-wide scan from being completed.
#[derive(Debug, Error)]
pub enum RunnerError {
    #[error("Paper preview queue failed: {0}")]
    Preview(#[from] PreviewRunnerError),

    #[error("pending target scan failed: {0}")]
    PendingScan(#[from] TenancyError),

    #[error("Paper account scan failed: {0}")]
    AccountScan(#[from] sqlx::Error),

    #[error("Paper settlement notification scan failed: {0}")]
    AnnouncementScan(TenancyError),

    #[error("invalid Paper processing date: {0}")]
    InvalidDate(String),

    #[error("Paper cycle canceled during shutdown")]
    Shutdown,

    #[error("Paper {operation} exceeded its {timeout:?} application deadline")]
    Deadline {
        operation: &'static str,
        timeout: Duration,
    },
}

#[derive(Debug)]
enum StageError<E> {
    Canceled,
    TimedOut,
    Failed(E),
}

async fn wait_for_shutdown(mut shutdown: watch::Receiver<bool>) {
    loop {
        if *shutdown.borrow() {
            return;
        }
        if shutdown.changed().await.is_err() {
            return;
        }
    }
}

/// Await one stage with both an application deadline and cooperative shutdown
/// cancellation. Dropping the SQLx future on either boundary releases its
/// transaction; the transaction's local PostgreSQL limits provide the server
/// side backstop for a query that has already reached the database.
async fn await_stage<F, T, E>(
    future: F,
    timeout: Duration,
    shutdown: Option<watch::Receiver<bool>>,
) -> Result<T, StageError<E>>
where
    F: Future<Output = Result<T, E>>,
{
    if timeout.is_zero() {
        return Err(StageError::TimedOut);
    }
    tokio::pin!(future);
    let timer = tokio::time::sleep(timeout);
    tokio::pin!(timer);
    match shutdown {
        Some(shutdown) => {
            tokio::select! {
                biased;
                result = &mut future => result.map_err(StageError::Failed),
                _ = &mut timer => Err(StageError::TimedOut),
                _ = wait_for_shutdown(shutdown) => Err(StageError::Canceled),
            }
        }
        None => tokio::select! {
            biased;
            result = &mut future => result.map_err(StageError::Failed),
            _ = &mut timer => Err(StageError::TimedOut),
        },
    }
}

#[derive(Clone)]
struct CycleBudget {
    cycle_deadline: Instant,
    operation_deadline: Duration,
    shutdown: Option<watch::Receiver<bool>>,
}

impl CycleBudget {
    fn new(services: &RunnerServices, shutdown: Option<watch::Receiver<bool>>) -> Self {
        Self {
            cycle_deadline: Instant::now() + services.cycle_deadline,
            operation_deadline: services.operation_deadline,
            shutdown,
        }
    }

    fn remaining(&self) -> Duration {
        self.cycle_deadline
            .saturating_duration_since(Instant::now())
    }

    fn stage_timeout(&self) -> Duration {
        self.operation_deadline.min(self.remaining())
    }

    fn shutdown(&self) -> Option<watch::Receiver<bool>> {
        self.shutdown.clone()
    }
}

/// Runs one deterministic processing date for every due target and active
/// Paper account without allowing any individual stage or the whole cycle to
/// become unbounded.
pub async fn run_cycle(
    services: &RunnerServices,
    process_date: NaiveDate,
) -> Result<CycleReport, RunnerError> {
    run_cycle_with_shutdown(services, process_date, None).await
}

/// Variant used by the daemon. A signal is observed while preview, target,
/// and valuation work is in flight, not only between polling iterations.
pub async fn run_cycle_with_shutdown(
    services: &RunnerServices,
    process_date: NaiveDate,
    shutdown: Option<watch::Receiver<bool>>,
) -> Result<CycleReport, RunnerError> {
    let budget = CycleBudget::new(services, shutdown);
    let mut report = CycleReport::default();

    // Recover terminal targets whose outbox dispatch was interrupted by a
    // deadline or SIGTERM before doing more work.  This scan is intentionally
    // first in the cycle so a repeatedly failing preview cannot starve alert
    // recovery forever.
    match await_stage(
        drain_announcements(services, &mut report),
        budget.stage_timeout(),
        budget.shutdown(),
    )
    .await
    {
        Ok(()) => {}
        Err(StageError::Failed(error)) => return Err(RunnerError::AnnouncementScan(error)),
        Err(StageError::Canceled) => return Err(RunnerError::Shutdown),
        Err(StageError::TimedOut) => {
            return Err(RunnerError::Deadline {
                operation: "Paper settlement notification scan",
                timeout: services.operation_deadline,
            });
        }
    }
    if let Err(error) = PendingTargetRepo::prune_settlement_outbox_worker(
        &services.worker_pool,
        7 * 24 * 60 * 60,
        256,
    )
    .await
    {
        report.item_errors.push(RunnerItemError {
            kind: "notification",
            resource_id: Uuid::nil(),
            detail: format!("settlement outbox prune failed: {error}"),
        });
    }
    match PendingTargetRepo::settlement_backlog_worker(&services.worker_pool).await {
        Ok(backlog) => record_backlog(&mut report, backlog),
        Err(error) => return Err(RunnerError::AnnouncementScan(error)),
    }
    let preview_started = Instant::now();
    let preview = match await_stage(
        run_preview_once(
            &services.worker_pool,
            &services.preview_queue,
            &services.dataset_root,
            &services.preview_worker_id,
            process_date,
            services.preview_heartbeat,
        ),
        budget.stage_timeout(),
        budget.shutdown(),
    )
    .await
    {
        Ok(preview) => preview,
        Err(StageError::Canceled) => return Err(RunnerError::Shutdown),
        Err(StageError::TimedOut) => {
            return Err(RunnerError::Deadline {
                operation: "preview",
                timeout: services.operation_deadline,
            });
        }
        Err(StageError::Failed(error)) => return Err(error.into()),
    };
    let targets = match await_stage(
        PendingTargetRepo::due_worker(&services.worker_pool, process_date),
        budget.stage_timeout(),
        budget.shutdown(),
    )
    .await
    {
        Ok(targets) => targets,
        Err(StageError::Canceled) => return Err(RunnerError::Shutdown),
        Err(StageError::TimedOut) => {
            return Err(RunnerError::Deadline {
                operation: "pending-target scan",
                timeout: services.operation_deadline,
            });
        }
        Err(StageError::Failed(error)) => return Err(error.into()),
    };
    let accounts = match await_stage(
        active_paper_accounts(&services.worker_pool),
        budget.stage_timeout(),
        budget.shutdown(),
    )
    .await
    {
        Ok(accounts) => accounts,
        Err(StageError::Canceled) => return Err(RunnerError::Shutdown),
        Err(StageError::TimedOut) => {
            return Err(RunnerError::Deadline {
                operation: "Paper account scan",
                timeout: services.operation_deadline,
            });
        }
        Err(StageError::Failed(error)) => return Err(error.into()),
    };
    let date = TradingDate::parse(&process_date.to_string())
        .map_err(|e| RunnerError::InvalidDate(format!("{process_date}: {e}")))?;

    report.preview_compute_ms = preview_started.elapsed().as_millis();
    report.targets_seen = targets.len();
    report.valuations_seen = accounts.len();
    match preview {
        PreviewRunOutcome::Idle => report.preview_outcome = "idle",
        PreviewRunOutcome::Published { .. } => {
            report.previews_seen = 1;
            report.previews_published = 1;
            report.preview_outcome = "published";
        }
        PreviewRunOutcome::Retrying { .. } => {
            report.previews_seen = 1;
            report.preview_outcome = "retrying";
        }
        PreviewRunOutcome::Failed { .. } => {
            report.previews_seen = 1;
            report.previews_failed = 1;
            report.preview_outcome = "failed";
        }
        PreviewRunOutcome::Canceled { .. } => {
            report.previews_seen = 1;
            report.preview_outcome = "canceled";
        }
        PreviewRunOutcome::LeaseLost { .. } => {
            report.previews_seen = 1;
            report.preview_outcome = "lease_lost";
        }
    }

    for target in targets {
        if budget.remaining().is_zero() {
            return Err(RunnerError::Deadline {
                operation: "Paper cycle",
                timeout: services.cycle_deadline,
            });
        }
        let target_id = target.id;
        match await_stage(
            settle_target(services, target, &mut report),
            budget.stage_timeout(),
            budget.shutdown(),
        )
        .await
        {
            Ok(()) => {}
            Err(StageError::Canceled) => return Err(RunnerError::Shutdown),
            Err(StageError::TimedOut) => report.item_errors.push(RunnerItemError {
                kind: "target",
                resource_id: target_id,
                detail: format!(
                    "target settlement/announcement exceeded {:?}; terminal state and its durable notification outbox will be retried",
                    services.operation_deadline
                ),
            }),
            Err(StageError::Failed(error)) => match error {},
        }
    }

    // A target may have committed its terminal row and outbox just before the
    // per-target application deadline canceled its dispatch future.  Drain it
    // again before valuation so this same cycle can make the normal path
    // truthful without relying solely on the next process iteration.
    match await_stage(
        drain_announcements(services, &mut report),
        budget.stage_timeout(),
        budget.shutdown(),
    )
    .await
    {
        Ok(()) => {}
        Err(StageError::Failed(error)) => report.item_errors.push(RunnerItemError {
            kind: "notification",
            resource_id: Uuid::nil(),
            detail: error.to_string(),
        }),
        Err(StageError::Canceled) => return Err(RunnerError::Shutdown),
        Err(StageError::TimedOut) => report.item_errors.push(RunnerItemError {
            kind: "notification",
            resource_id: Uuid::nil(),
            detail: format!(
                "settlement notification scan exceeded {:?}; pending intents remain durable",
                services.operation_deadline
            ),
        }),
    }
    match PendingTargetRepo::settlement_backlog_worker(&services.worker_pool).await {
        Ok(backlog) => record_backlog(&mut report, backlog),
        Err(error) => report.item_errors.push(RunnerItemError {
            kind: "notification",
            resource_id: Uuid::nil(),
            detail: format!("settlement backlog stats failed: {error}"),
        }),
    }

    for account in accounts {
        if budget.remaining().is_zero() {
            return Err(RunnerError::Deadline {
                operation: "Paper cycle",
                timeout: services.cycle_deadline,
            });
        }
        let account_id = account.id;
        match await_stage(
            value_account(
                &services.worker_pool,
                &services.dataset_root,
                account.id,
                account.owner_user_id,
                date,
            ),
            budget.stage_timeout(),
            budget.shutdown(),
        )
        .await
        {
            Ok(ValuationOutcome::Valued { .. } | ValuationOutcome::AlreadyValued) => {
                report.valuations_written += 1;
            }
            Err(StageError::Failed(error)) => report.item_errors.push(RunnerItemError {
                kind: "valuation",
                resource_id: account_id,
                detail: error.to_string(),
            }),
            Err(StageError::Canceled) => return Err(RunnerError::Shutdown),
            Err(StageError::TimedOut) => report.item_errors.push(RunnerItemError {
                kind: "valuation",
                resource_id: account_id,
                detail: format!(
                    "valuation exceeded {:?}; transaction was canceled",
                    services.operation_deadline
                ),
            }),
        }
    }

    Ok(report)
}

async fn settle_target(
    services: &RunnerServices,
    target: WorkerPendingTargetRow,
    report: &mut CycleReport,
) -> Result<(), std::convert::Infallible> {
    let actor = match owner_actor(&services.worker_pool, target.owner_user_id).await {
        Ok(actor) => actor,
        Err(error) => {
            report.item_errors.push(RunnerItemError {
                kind: "target",
                resource_id: target.id,
                detail: format!("owner role lookup failed: {error}"),
            });
            return Ok(());
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
        Ok(_) => {
            report.targets_settled += 1;
            report.notifications_seen += 1;
            report.notifications_delivered += 1;
        }
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
    Ok(())
}

/// Dispatch committed settlement intents.  Every row is keyed by its target,
/// and [`Notifier::dispatch_paper_settlement`] keys each recipient/channel by
/// the same outbox id, so a process kill at any point is safe to retry.
async fn drain_announcements(
    services: &RunnerServices,
    report: &mut CycleReport,
) -> Result<(), TenancyError> {
    let rows = PendingTargetRepo::due_announcements_worker(&services.worker_pool, 128).await?;
    report.notifications_seen += rows.len();
    for outbox in rows {
        let actor = match owner_actor(&services.worker_pool, outbox.owner_user_id).await {
            Ok(actor) => actor,
            Err(error) => {
                report.notifications_pending += 1;
                report.item_errors.push(RunnerItemError {
                    kind: "notification",
                    resource_id: outbox.id,
                    detail: format!("owner role lookup failed: {error}"),
                });
                continue;
            }
        };
        match services
            .state
            .notifier()
            .dispatch_paper_settlement(&actor, &outbox)
            .await
        {
            Ok(result)
                if result
                    .deliveries
                    .iter()
                    .all(|delivery| delivery.status == "SUCCESS") =>
            {
                match services
                    .state
                    .pending_targets()
                    .mark_announcement_delivered(&actor, outbox.id)
                    .await
                {
                    Ok(_) => report.notifications_delivered += 1,
                    Err(error) => {
                        report.notifications_pending += 1;
                        report.item_errors.push(RunnerItemError {
                            kind: "notification",
                            resource_id: outbox.id,
                            detail: format!("announcement mark failed: {error}"),
                        });
                    }
                }
            }
            Ok(result) => {
                report.notifications_pending += 1;
                let detail = result
                    .deliveries
                    .iter()
                    .filter(|delivery| delivery.status == "FAILED")
                    .filter_map(|delivery| delivery.error_detail.clone())
                    .collect::<Vec<_>>()
                    .join("; ");
                let _ = services
                    .state
                    .pending_targets()
                    .record_announcement_failure(&actor, outbox.id, &detail)
                    .await;
                crate::observability::metrics::record_paper_settlement_retry("transport_failed");
                report.item_errors.push(RunnerItemError {
                    kind: "notification",
                    resource_id: outbox.id,
                    detail: if detail.is_empty() {
                        "notification transport failed".to_owned()
                    } else {
                        detail
                    },
                });
            }
            Err(error) => {
                report.notifications_pending += 1;
                let _ = services
                    .state
                    .pending_targets()
                    .record_announcement_failure(&actor, outbox.id, &error.to_string())
                    .await;
                report.item_errors.push(RunnerItemError {
                    kind: "notification",
                    resource_id: outbox.id,
                    detail: error.to_string(),
                });
            }
        }
    }
    Ok(())
}

fn record_backlog(report: &mut CycleReport, backlog: PaperSettlementBacklog) {
    report.notification_backlog = backlog.pending_count;
    report.notification_oldest_age_secs = backlog.oldest_pending_age_secs;
    report.notification_failed = backlog.failed_count;
    report.notification_exhausted = backlog.exhausted_count;
    report.notification_ready = backlog.ready;
    crate::observability::metrics::record_paper_settlement_backlog(
        backlog.pending_count,
        backlog.oldest_pending_age_secs,
        backlog.ready,
    );
    if backlog.exhausted_count > 0 {
        crate::observability::metrics::record_paper_settlement_retry("exhausted");
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
    let mut tx = pool.begin().await?;
    set_paper_transaction_timeouts(&mut tx).await?;
    let rows = sqlx::query_as::<_, WorkerPaperAccount>(
        "SELECT id, owner_user_id FROM accounts \
         WHERE account_type = 'PAPER' AND status = 'ACTIVE' \
         ORDER BY owner_user_id, id",
    )
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}

async fn owner_actor(pool: &sqlx::PgPool, user_id: Uuid) -> Result<Actor, sqlx::Error> {
    let mut tx = pool.begin().await?;
    set_paper_transaction_timeouts(&mut tx).await?;
    let is_owner: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM user_roles WHERE user_id = $1 AND role_id = 'owner')",
    )
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Actor::new(
        user_id.to_string(),
        if is_owner { Role::Owner } else { Role::Member },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::Infallible;

    #[test]
    fn cycle_report_starts_empty() {
        let report = CycleReport::default();
        assert_eq!(report.previews_seen, 0);
        assert_eq!(report.previews_published, 0);
        assert_eq!(report.previews_failed, 0);
        assert_eq!(report.targets_seen, 0);
        assert_eq!(report.valuations_written, 0);
        assert!(report.item_errors.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn stuck_paper_stage_is_canceled_by_sigterm_boundary() {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let stage = await_stage::<_, (), Infallible>(
            std::future::pending::<Result<(), Infallible>>(),
            Duration::from_secs(20),
            Some(shutdown_rx),
        );
        tokio::pin!(stage);
        tokio::task::yield_now().await;
        shutdown_tx.send(true).unwrap();
        assert!(matches!(stage.await, Err(StageError::Canceled)));
    }

    #[tokio::test(start_paused = true)]
    async fn preview_timeout_path_is_finite() {
        let result = await_stage::<_, (), Infallible>(
            std::future::pending::<Result<(), Infallible>>(),
            Duration::from_secs(3),
            None,
        );
        tokio::pin!(result);
        tokio::time::advance(Duration::from_secs(3)).await;
        assert!(matches!(result.await, Err(StageError::TimedOut)));
    }

    #[tokio::test(start_paused = true)]
    async fn target_settlement_timeout_path_is_finite() {
        let result = await_stage::<_, (), Infallible>(
            std::future::pending::<Result<(), Infallible>>(),
            Duration::from_secs(4),
            None,
        );
        tokio::pin!(result);
        tokio::time::advance(Duration::from_secs(4)).await;
        assert!(matches!(result.await, Err(StageError::TimedOut)));
    }

    #[tokio::test(start_paused = true)]
    async fn valuation_timeout_path_is_finite() {
        let result = await_stage::<_, (), Infallible>(
            std::future::pending::<Result<(), Infallible>>(),
            Duration::from_secs(5),
            None,
        );
        tokio::pin!(result);
        tokio::time::advance(Duration::from_secs(5)).await;
        assert!(matches!(result.await, Err(StageError::TimedOut)));
    }
}
