//! The process that drains the backtest queue.
//!
//! `run_once` is a library function, and a library function nobody runs leaves
//! the queue exactly as full as it was. This binary is the deployment unit:
//! one process, one job at a time, until it is asked to stop.
//!
//! # Two responsibilities, not one
//!
//! It runs jobs, and it SWEEPS. `runner.rs` says a crash mid-backtest is safe
//! because "the sweeper requeues it" — and until this binary existed, nothing
//! in the repository ever called [`JobQueue::sweep`]. The recovery story was
//! prose. A job whose runner died stayed RUNNING behind an expired lease with
//! no process left that would ever look at it, so the crash-safety the lease
//! design buys was never actually collected.
//!
//! Sweeping happens on the IDLE path. A busy runner is a runner whose leases
//! are being heartbeated; the moment worth spending on recovery is the moment
//! there is nothing else to do. Several runners sweeping concurrently is safe
//! — the sweep is a conditional UPDATE — and is the normal case with more than
//! one replica.
//!
//! # What it deliberately does not do
//!
//! **Cooperative cancel is not observed mid-run.** [`JobQueue::request_cancel`]
//! flips the row to CANCELED and expects the running worker to notice at a
//! checkpoint; this runner hands the whole backtest to a child process and
//! learns nothing until it exits. A cancel therefore takes effect when the
//! current backtest finishes rather than promptly, bounded by the worker's own
//! `wall_seconds` limit. Settling a claim whose row was canceled underneath it
//! is handled by the queue, so the outcome is correct — only the latency is
//! wrong. Fixing it means a checkpoint protocol with the child, which is a
//! change to the worker, not to this loop.
//!
//! **One job at a time.** A NautilusTrader run is given a 2 GiB limit; two in
//! one container is how a host that was sized for one runner starts swapping.
//! Concurrency here is `--replicas`, not threads.
//!
//! **No audit connection.** [`JobQueue`] takes an optional second pool for
//! `audit_logs`, and this process passes `None` — a decision rather than an
//! omission. `request_cancel` is the only operation that writes an audit row,
//! it is the API server's to call, and the daemon calls `claim_next_for`
//! for `backtest` jobs,
//! `settle_*` and `sweep` exclusively. `worker` is not granted INSERT on
//! `audit_logs` (the migration contract asserts it), so handing this process
//! an audit pool would mean giving the role a privilege it has no use for.
//!
//! **`--once` does not sweep.** The one-shot path exits on an empty queue
//! before reaching the recovery step, so it will not pick up an orphan left by
//! a previous crash. That is right for the gates it exists for, which start
//! from a fresh database; a long-lived deployment must run without `--once`.

use job_queue::queue::{JobQueue, QueueConfig};
use job_queue::resolver::DbStrategyResolver;
use job_queue::runner::{Outcome, RunnerPaths, run_once};
use sqlx::postgres::PgPoolOptions;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

/// How long to wait after finding nothing.
///
/// A backtest takes seconds to minutes, so polling faster buys latency nobody
/// perceives and costs a query per runner per interval. `LISTEN/NOTIFY` would
/// remove the poll entirely; it is not worth its failure modes until the idle
/// query shows up in a profile.
const IDLE_SLEEP: Duration = Duration::from_secs(2);

/// Backoff after a queue error, so a database outage does not become a hot
/// loop hammering a server that is already unwell.
const ERROR_SLEEP: Duration = Duration::from_secs(10);

struct Args {
    /// Claim at most one job, then exit. For scripts and gates, which need a
    /// deterministic end rather than a daemon they have to kill.
    once: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args { once: false };
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--once" => args.once = true,
            "--help" | "-h" => {
                println!(
                    "backtest-runner [--once]\n\n\
                     Drains the `backtest` job queue.\n\n\
                     Environment:\n  \
                       DATABASE_URL           required; connect as the `worker` role\n  \
                       LAGRANGE_REPO_ROOT     defaults to the current directory\n  \
                       LAGRANGE_DATASET_ROOT  defaults to <repo>/data/phase0\n  \
                       LAGRANGE_ARTIFACTS_ROOT defaults to <repo>/artifacts\n  \
                       LAGRANGE_UV_BIN        defaults to `uv` on PATH\n"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unrecognised argument {other:?} (try --help)")),
        }
    }
    Ok(args)
}

fn env_path(key: &str, fallback: PathBuf) -> PathBuf {
    std::env::var_os(key).map(PathBuf::from).unwrap_or(fallback)
}

/// Identifies this process in `jobs.locked_by`.
///
/// The pid is included because the host name alone cannot distinguish two
/// replicas on one machine, and `locked_by` is what an operator reads when
/// asking which process is holding a stuck job.
fn worker_id() -> String {
    let host = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "unknown-host".to_string());
    format!("backtest-runner@{host}/{}", std::process::id())
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("backtest-runner: {e}");
            return ExitCode::FAILURE;
        }
    };

    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("backtest-runner: DATABASE_URL is required");
        return ExitCode::FAILURE;
    };

    let repo_root = env_path(
        "LAGRANGE_REPO_ROOT",
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    );
    let paths = RunnerPaths {
        dataset_root: env_path("LAGRANGE_DATASET_ROOT", repo_root.join("data/phase0")),
        artifacts_root: env_path("LAGRANGE_ARTIFACTS_ROOT", repo_root.join("artifacts")),
        repo_root,
        uv_bin: RunnerPaths::default_uv_bin(),
    };

    // Small on purpose: this process runs one job at a time, so a large pool
    // would reserve connections it can never use while starving the API.
    let pool = match PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&database_url)
        .await
    {
        Ok(p) => p,
        Err(e) => {
            eprintln!("backtest-runner: cannot reach the database: {e}");
            return ExitCode::FAILURE;
        }
    };

    let id = worker_id();
    let queue = JobQueue::new(pool.clone(), None, QueueConfig::default());
    let resolver = DbStrategyResolver::new(pool.clone());
    eprintln!("backtest-runner: {id} draining the queue");

    // Ctrl-C sets a flag rather than aborting: a claimed job must be settled
    // before this process leaves, or it stays RUNNING until a sweep. The
    // signal is observed BETWEEN jobs, so a shutdown waits out the backtest in
    // flight instead of orphaning it.
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);

    loop {
        let outcome = tokio::select! {
            biased;
            // Checked first, but only ever observed here -- between jobs.
            _ = &mut shutdown => {
                eprintln!("backtest-runner: shutting down");
                break;
            }
            result = run_once(&queue, &id, &paths, &resolver) => result,
        };

        match outcome {
            Ok(Outcome::Succeeded { job_id }) => eprintln!("job {job_id}: succeeded"),
            Ok(Outcome::Failed { job_id, reason }) => eprintln!("job {job_id}: failed: {reason}"),
            Ok(Outcome::Errored { job_id, reason }) => {
                eprintln!("job {job_id}: runner error, will retry: {reason}");
            }
            Ok(Outcome::Idle) => {
                if args.once {
                    break;
                }
                // Nothing to run is the moment to recover what a dead runner
                // left behind. See the module docs.
                match queue.sweep().await {
                    // Logged only when it did something. A line every two
                    // seconds saying nothing happened is how the line that
                    // matters gets missed.
                    Ok(r)
                        if r.attempts_orphaned > 0 || r.jobs_requeued > 0 || r.jobs_failed > 0 =>
                    {
                        eprintln!(
                            "swept: {} attempts orphaned, {} jobs requeued, {} exhausted",
                            r.attempts_orphaned, r.jobs_requeued, r.jobs_failed
                        );
                    }
                    Ok(_) => {}
                    Err(e) => eprintln!("sweep failed: {e}"),
                }
                tokio::time::sleep(IDLE_SLEEP).await;
                continue;
            }
            Err(e) => {
                // The queue itself is unreachable. Every claimed job was
                // settled before this returned, so backing off loses nothing
                // and stops a sick database being hammered.
                eprintln!("backtest-runner: queue error: {e}");
                if args.once {
                    return ExitCode::FAILURE;
                }
                tokio::time::sleep(ERROR_SLEEP).await;
                continue;
            }
        }

        if args.once {
            break;
        }
    }

    // Closing returns the connections rather than leaving the server to time
    // them out, which matters when a deploy restarts every replica at once.
    pool.close().await;
    ExitCode::SUCCESS
}
