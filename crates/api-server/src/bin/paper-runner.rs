//! Long-lived Paper worker daemon.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use api_server::http::state::{ApiConfig, ApiState};
use api_server::paper_runner::{RunnerArgs, RunnerServices, parse_args, run_cycle};
use chrono::{FixedOffset, NaiveDate, Utc};
use sqlx::postgres::PgPoolOptions;

const IDLE_SLEEP: Duration = Duration::from_secs(2);
const ERROR_SLEEP: Duration = Duration::from_secs(10);

fn env_path(key: &str, fallback: PathBuf) -> PathBuf {
    std::env::var_os(key).map(PathBuf::from).unwrap_or(fallback)
}

fn required_env(key: &str) -> Result<String, String> {
    std::env::var(key).map_err(|_| format!("{key} is required"))
}

fn configured_date(args: &RunnerArgs) -> Result<NaiveDate, String> {
    if let Some(date) = args.date {
        return Ok(date);
    }
    if let Ok(value) = std::env::var("PAPER_DATE") {
        return parse_args(vec!["--date".to_owned(), value])
            .map(|parsed| parsed.date.expect("--date parser returns a date"))
            .map_err(|error| format!("PAPER_DATE: {error}"));
    }
    let seoul = FixedOffset::east_opt(9 * 60 * 60).expect("fixed Seoul offset");
    Ok(Utc::now().with_timezone(&seoul).date_naive())
}

async fn connect(url: &str, label: &str, max_connections: u32) -> Result<sqlx::PgPool, String> {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(Duration::from_secs(10))
        .connect(url)
        .await
        .map_err(|error| format!("cannot connect {label} database: {error}"))
}

fn print_help() {
    println!(
        "paper-runner [--once] [--date YYYY-MM-DD]\n\n\
         Executes due Paper targets and writes verified daily equity.\n\n\
         Environment:\n  \
           DATABASE_URL             app-role URL (RLS-scoped settlement)\n  \
           WORKER_DATABASE_URL      worker-role URL (cross-tenant engine)\n  \
           ADMIN_DATABASE_URL       admin-role URL (owner notification lookup)\n  \
           AUDIT_DATABASE_URL       audit pool URL\n  \
           PAPER_DATE               fallback processing date\n  \
           LAGRANGE_DATASET_ROOT    curated dataset root\n"
    );
}

async fn build_services() -> Result<RunnerServices, String> {
    let app_url = required_env("DATABASE_URL")?;
    let worker_url = required_env("WORKER_DATABASE_URL")?;
    let admin_url = required_env("ADMIN_DATABASE_URL")?;
    let audit_url = required_env("AUDIT_DATABASE_URL")?;
    let repo_root = env_path(
        "LAGRANGE_REPO_ROOT",
        std::env::current_dir().map_err(|error| format!("current directory: {error}"))?,
    );
    let dataset_root = env_path("LAGRANGE_DATASET_ROOT", repo_root.join("data/phase0"));
    let app_pool = connect(&app_url, "app", 4).await?;
    let worker_pool = connect(&worker_url, "worker", 4).await?;
    let admin_pool = connect(&admin_url, "admin", 2).await?;
    let audit_pool = connect(&audit_url, "audit", 2).await?;
    let state = ApiState::from_pools(
        ApiConfig {
            // The daemon never serves cursor-bearing HTTP routes; this field
            // is nevertheless required by the shared ApiState contract.
            cursor_secret: [0; 32],
            max_jobs_per_owner: 10,
            db_url: app_url,
            step_up_max_auth_age_secs: 900,
            artifact_root: repo_root.join("artifacts"),
        },
        app_pool,
        admin_pool,
        audit_pool,
    )
    .await
    .map_err(|error| format!("build API state: {error}"))?;
    Ok(RunnerServices::new(state, worker_pool, dataset_root))
}

#[tokio::main]
async fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        return ExitCode::SUCCESS;
    }
    let args = match parse_args(raw) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("paper-runner: {error}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = configured_date(&args) {
        eprintln!("paper-runner: {error}");
        return ExitCode::FAILURE;
    }
    let services = match build_services().await {
        Ok(services) => services,
        Err(error) => {
            eprintln!("paper-runner: {error}");
            return ExitCode::FAILURE;
        }
    };

    eprintln!("paper-runner: started");
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);
    loop {
        let date = match configured_date(&args) {
            Ok(date) => date,
            Err(error) => {
                eprintln!("paper-runner: {error}");
                return ExitCode::FAILURE;
            }
        };
        let cycle = run_cycle(&services, date).await;
        match cycle {
            Ok(report) => eprintln!(
                "paper-runner: date={date} targets={}/{} valuations={}/{} errors={}",
                report.targets_settled,
                report.targets_seen,
                report.valuations_written,
                report.valuations_seen,
                report.item_errors.len()
            ),
            Err(error) => {
                eprintln!("paper-runner: cycle failed: {error}");
                if args.once {
                    return ExitCode::FAILURE;
                }
                tokio::select! {
                    biased;
                    _ = &mut shutdown => break,
                    _ = tokio::time::sleep(ERROR_SLEEP) => continue,
                }
            }
        }
        if args.once {
            break;
        }
        tokio::select! {
            biased;
            _ = &mut shutdown => break,
            _ = tokio::time::sleep(IDLE_SLEEP) => {}
        }
    }
    services.worker_pool.close().await;
    ExitCode::SUCCESS
}
