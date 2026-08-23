//! Long-lived Paper worker daemon.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use api_server::http::state::{ApiConfig, ApiState};
use api_server::paper_runner::{RunnerArgs, RunnerServices, parse_args, run_cycle_with_shutdown};
use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPoolOptions;
use tokio::sync::{mpsc, watch};

const IDLE_SLEEP: Duration = Duration::from_secs(2);
const ERROR_SLEEP: Duration = Duration::from_secs(10);
const HEALTH_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const HEALTH_WRITE_DEADLINE: Duration = Duration::from_secs(2);

fn env_path(key: &str, fallback: PathBuf) -> PathBuf {
    std::env::var_os(key).map(PathBuf::from).unwrap_or(fallback)
}

fn required_env(key: &str) -> Result<String, String> {
    std::env::var(key).map_err(|_| format!("{key} is required"))
}

fn env_duration_ms(key: &str, default: Duration) -> Result<Duration, String> {
    let Some(raw) = std::env::var_os(key) else {
        return Ok(default);
    };
    let raw = raw
        .into_string()
        .map_err(|_| format!("{key} must be a positive millisecond value"))?;
    let millis = raw
        .parse::<u64>()
        .map_err(|_| format!("{key} must be a positive millisecond value"))?;
    if millis == 0 {
        return Err(format!("{key} must be a positive millisecond value"));
    }
    Ok(Duration::from_millis(millis))
}

fn env_duration_secs(key: &str, default: Duration) -> Result<Duration, String> {
    let Some(raw) = std::env::var_os(key) else {
        return Ok(default);
    };
    let raw = raw
        .into_string()
        .map_err(|_| format!("{key} must be a positive integer number of seconds"))?;
    let seconds = raw
        .parse::<u64>()
        .map_err(|_| format!("{key} must be a positive integer number of seconds"))?;
    if seconds == 0 {
        return Err(format!(
            "{key} must be a positive integer number of seconds"
        ));
    }
    Ok(Duration::from_secs(seconds))
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

#[cfg(any(unix, test))]
async fn wait_for_shutdown<C, T>(ctrl_c: C, terminate: T) -> std::io::Result<()>
where
    C: std::future::Future<Output = std::io::Result<()>>,
    T: std::future::Future<Output = std::io::Result<()>>,
{
    tokio::select! {
        result = ctrl_c => result,
        result = terminate => result,
    }
}

#[cfg(unix)]
async fn shutdown_signal() -> std::io::Result<()> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    wait_for_shutdown(tokio::signal::ctrl_c(), async move {
        terminate.recv().await.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "terminate signal stream closed",
            )
        })?;
        Ok(())
    })
    .await
}

#[cfg(not(unix))]
async fn shutdown_signal() -> std::io::Result<()> {
    tokio::signal::ctrl_c().await
}

/// Send a systemd service notification when the unit provides
/// `NOTIFY_SOCKET`.
///
/// Systemd accepts both filesystem pathname sockets and Linux abstract
/// namespace sockets. In the environment spelling, an abstract address starts
/// with `@` (systemd's convention); it must be converted to the leading NUL
/// byte used by `sockaddr_un`, rather than passed to a pathname-only API.
/// Notifications are tiny datagrams and do not carry credentials or dataset
/// contents.
#[cfg(unix)]
async fn systemd_notify(message: &str) -> Result<(), String> {
    let Some(socket_path) = std::env::var_os("NOTIFY_SOCKET") else {
        return Ok(());
    };
    if socket_path.is_empty() {
        return Ok(());
    }
    send_systemd_notify_to(&socket_path, message).await
}

#[cfg(unix)]
fn systemd_notify_addr(socket_path: &std::ffi::OsStr) -> Result<socket2::SockAddr, String> {
    use std::os::unix::ffi::OsStrExt;

    let bytes = socket_path.as_bytes();
    if bytes.is_empty() {
        return Err("systemd notify socket path is empty".to_owned());
    }
    let address = if let Some(name) = bytes.strip_prefix(b"@") {
        #[cfg(target_os = "linux")]
        {
            use std::os::linux::net::SocketAddrExt as _;
            use std::os::unix::net::SocketAddr;

            if name.is_empty() {
                return Err("systemd abstract notify socket name is empty".to_owned());
            }
            let _ = SocketAddr::from_abstract_name(name)
                .map_err(|error| format!("systemd abstract notify socket: {error}"))?;
            abstract_notify_addr(name)?
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = name;
            return Err("systemd abstract notify sockets are supported only on Linux".to_owned());
        }
    } else {
        socket2::SockAddr::unix(std::path::Path::new(socket_path))
            .map_err(|error| format!("systemd notify socket path: {error}"))?
    };
    Ok(address)
}

/// Build the socket2 address for a Linux abstract namespace. `socket2` offers
/// the portable pathname constructor but intentionally does not convert the
/// standard library's Linux-only `SocketAddr` type, so the short raw address
/// layout is filled here after `SocketAddrExt` has validated the name.
#[cfg(target_os = "linux")]
fn abstract_notify_addr(name: &[u8]) -> Result<socket2::SockAddr, String> {
    // SAFETY: `try_init` gives us writable, properly aligned storage; this
    // closure zeroes the complete sockaddr and sets its family, abstract name,
    // and exact length before returning it to socket2.
    let (_, address) = unsafe {
        socket2::SockAddr::try_init(|storage, length| {
            let address = storage.cast::<libc::sockaddr_un>();
            let path_offset = std::mem::offset_of!(libc::sockaddr_un, sun_path);
            let path_capacity = (*address).sun_path.len();
            if name.len() + 1 > path_capacity {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "abstract notify socket name is too long",
                ));
            }
            std::ptr::write_bytes(address, 0, 1);
            (*address).sun_family = libc::AF_UNIX as libc::sa_family_t;
            std::ptr::copy_nonoverlapping(
                name.as_ptr(),
                (*address).sun_path.as_mut_ptr().cast::<u8>().add(1),
                name.len(),
            );
            *length = (path_offset + 1 + name.len()) as socket2::socklen_t;
            Ok(())
        })
    }
    .map_err(|error| format!("systemd abstract notify socket: {error}"))?;
    Ok(address)
}

#[cfg(unix)]
async fn send_systemd_notify_to(
    socket_path: &std::ffi::OsStr,
    message: &str,
) -> Result<(), String> {
    let socket_path = socket_path.to_os_string();
    let message = message.to_owned();
    tokio::task::spawn_blocking(move || {
        use socket2::{Domain, Socket, Type};

        let address = systemd_notify_addr(&socket_path)?;
        let socket = Socket::new(Domain::UNIX, Type::DGRAM, None)
            .map_err(|error| format!("systemd notify socket: {error}"))?;
        socket
            .send_to(message.as_bytes(), &address)
            .map_err(|error| format!("systemd notify datagram: {error}"))?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|error| format!("systemd notify task: {error}"))?
}

#[cfg(not(unix))]
async fn systemd_notify(_message: &str) -> Result<(), String> {
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HealthState {
    pid: u32,
    heartbeat_at: DateTime<Utc>,
    last_progress_at: DateTime<Utc>,
    phase: String,
    cycle_id: u64,
    cycle_in_progress: bool,
    cycle_started_at: Option<DateTime<Utc>>,
    cycle_deadline_at: Option<DateTime<Utc>>,
    last_cycle_completed_at: Option<DateTime<Utc>>,
    last_cycle_outcome: Option<String>,
}

#[derive(Debug, Clone)]
struct HealthProgress {
    last_progress_at: DateTime<Utc>,
    phase: String,
    cycle_id: u64,
    cycle_in_progress: bool,
    cycle_started_at: Option<DateTime<Utc>>,
    cycle_deadline_at: Option<DateTime<Utc>>,
    last_cycle_completed_at: Option<DateTime<Utc>>,
    last_cycle_outcome: Option<String>,
}

impl HealthProgress {
    fn new(now: DateTime<Utc>) -> Self {
        Self {
            last_progress_at: now,
            phase: "starting".to_owned(),
            cycle_id: 0,
            cycle_in_progress: false,
            cycle_started_at: None,
            cycle_deadline_at: None,
            last_cycle_completed_at: None,
            last_cycle_outcome: None,
        }
    }

    fn state(&self, now: DateTime<Utc>) -> HealthState {
        HealthState {
            pid: std::process::id(),
            heartbeat_at: now,
            last_progress_at: self.last_progress_at,
            phase: self.phase.clone(),
            cycle_id: self.cycle_id,
            cycle_in_progress: self.cycle_in_progress,
            cycle_started_at: self.cycle_started_at,
            cycle_deadline_at: self.cycle_deadline_at,
            last_cycle_completed_at: self.last_cycle_completed_at,
            last_cycle_outcome: self.last_cycle_outcome.clone(),
        }
    }
}

fn health_state_path() -> Option<PathBuf> {
    std::env::var_os("PAPER_HEALTH_STATE_PATH")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

fn write_health_state(path: &std::path::Path, state: &HealthState) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| "health state path has no parent".to_owned())?;
    let metadata = std::fs::symlink_metadata(parent)
        .map_err(|_| "health state directory is inaccessible".to_owned())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("health state directory must be a non-symlink directory".to_owned());
    }
    let temporary = parent.join(format!(
        ".paper-health-{}-{}.tmp",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let result = (|| {
        let bytes = serde_json::to_vec(state).map_err(|_| "health state serialization failed")?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|_| "health state temporary file is inaccessible")?;
        use std::io::Write as _;
        file.write_all(&bytes)
            .map_err(|_| "health state write failed")?;
        file.write_all(b"\n")
            .map_err(|_| "health state write failed")?;
        file.sync_all().map_err(|_| "health state sync failed")?;
        drop(file);
        std::fs::rename(&temporary, path).map_err(|_| "health state replacement failed")?;
        Ok::<(), &str>(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(str::to_owned)
}

async fn maintain_health_state(
    path: PathBuf,
    mut progress: watch::Receiver<HealthProgress>,
    mut shutdown: watch::Receiver<bool>,
    error_tx: mpsc::Sender<String>,
    max_age: Duration,
) -> Result<(), String> {
    let mut ticker = tokio::time::interval(HEALTH_HEARTBEAT_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker.tick().await;
    loop {
        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return Ok(());
                }
            }
            changed = progress.changed() => {
                if changed.is_err() {
                    return Ok(());
                }
            }
            _ = ticker.tick() => {}
        }
        let state = progress.borrow().state(Utc::now());
        let now = Utc::now();
        let write_path = path.clone();
        let write_state = state.clone();
        let write =
            tokio::task::spawn_blocking(move || write_health_state(&write_path, &write_state));
        match tokio::time::timeout(HEALTH_WRITE_DEADLINE, write).await {
            Ok(Ok(Ok(()))) => {}
            Ok(Ok(Err(error))) => {
                let _ = error_tx.send(error.clone()).await;
                return Err(error);
            }
            Ok(Err(_)) => {
                let error = "health state writer task failed".to_owned();
                let _ = error_tx.send(error.clone()).await;
                return Err(error);
            }
            Err(_) => {
                let error = "health state write exceeded its deadline".to_owned();
                let _ = error_tx.send(error.clone()).await;
                return Err(error);
            }
        }
        if !progress_is_live(&state, now, max_age) {
            let error = "Paper loop progress exceeded its supervision deadline".to_owned();
            let _ = error_tx.send(error.clone()).await;
            return Err(error);
        }
        let status = format!(
            "WATCHDOG=1\nSTATUS=phase={} cycle={}",
            state.phase, state.cycle_id
        );
        if let Err(error) = systemd_notify(&status).await {
            let _ = error_tx.send(error.clone()).await;
            return Err(error);
        }
    }
}

fn progress_is_live(state: &HealthState, now: DateTime<Utc>, max_age: Duration) -> bool {
    if state.heartbeat_at > now || state.last_progress_at > now {
        return false;
    }
    let max_age = chrono::Duration::from_std(max_age).unwrap_or(chrono::Duration::MAX);
    if state.cycle_in_progress {
        state
            .cycle_deadline_at
            .is_some_and(|deadline| now <= deadline + max_age)
    } else {
        now.signed_duration_since(state.last_progress_at) <= max_age
    }
}

fn print_help() {
    println!(
        "paper-runner [--once] [--date YYYY-MM-DD] [PREVIEW OPTIONS]\n\n\
         Executes due Paper targets and writes verified daily equity.\n\n\
         Preview options:\n  \
           --preview-worker-id ID       queue lease owner\n  \
           --preview-heartbeat-ms N     heartbeat interval (default 10000)\n  \
           --preview-lease-ms N         queue lease (default 60000)\n  \
           --preview-backoff-ms N       first retry backoff (default 30000)\n\n\
         Environment:\n  \
           DATABASE_URL             app-role URL (RLS-scoped settlement)\n  \
           WORKER_DATABASE_URL      worker-role URL (cross-tenant engine)\n  \
           ADMIN_DATABASE_URL       admin-role URL (owner notification lookup)\n  \
           AUDIT_DATABASE_URL       audit pool URL\n  \
           PAPER_DATE               fallback processing date\n  \
           LAGRANGE_DATASET_ROOT    curated dataset root\n"
    );
    println!(
        "Timing controls: --operation-timeout-ms N (default 15000), \\
         --cycle-timeout-ms N (default 90000), \\
         --shutdown-grace-ms N (default 20000). \\
         PAPER_HEALTH_STATE_PATH records non-secret loop progress."
    );
}

async fn build_services(args: &RunnerArgs) -> Result<RunnerServices, String> {
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
            recommendation_dataset: job_queue::recommendation::input::DatasetPin {
                id: uuid::Uuid::nil(),
                dataset_id: "not-configured".to_owned(),
                version: "not-configured".to_owned(),
                curated_version: 1,
                manifest_sha256: "0".repeat(64),
            },
            db_url: app_url,
            step_up_max_auth_age_secs: 900,
            artifact_root: repo_root.join("artifacts"),
            seoul_today: api_server::http::state::system_seoul_today,
            candidate_eod_ready: || true,
            code_commit: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            // This daemon does not expose HTTP routes.  Its direct internal
            // state still carries the API configuration shape, so preserve
            // the normal non-beta default here.
            owner_beta_access: api_server::http::state::OwnerBetaAccessMode::Disabled,
        },
        app_pool,
        admin_pool,
        audit_pool,
    )
    .await
    .map_err(|error| format!("build API state: {error}"))?;
    Ok(RunnerServices::new(state, worker_pool, dataset_root)
        .with_preview_worker(
            args.preview_worker_id.clone(),
            args.preview_heartbeat,
            args.preview_lease,
            args.preview_backoff,
        )
        .with_deadlines(args.operation_deadline, args.cycle_deadline))
}

async fn async_main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        return ExitCode::SUCCESS;
    }
    let mut args = match parse_args(raw) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("paper-runner: {error}");
            return ExitCode::FAILURE;
        }
    };
    args.operation_deadline =
        match env_duration_ms("PAPER_OPERATION_TIMEOUT_MS", args.operation_deadline) {
            Ok(value) => value,
            Err(error) => {
                eprintln!("paper-runner: {error}");
                return ExitCode::FAILURE;
            }
        };
    args.cycle_deadline = match env_duration_ms("PAPER_CYCLE_TIMEOUT_MS", args.cycle_deadline) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("paper-runner: {error}");
            return ExitCode::FAILURE;
        }
    };
    args.shutdown_grace = match env_duration_ms("PAPER_SHUTDOWN_GRACE_MS", args.shutdown_grace) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("paper-runner: {error}");
            return ExitCode::FAILURE;
        }
    };
    if args.cycle_deadline < args.operation_deadline {
        eprintln!("paper-runner: cycle timeout must not be shorter than operation timeout");
        return ExitCode::FAILURE;
    }
    if let Err(error) = configured_date(&args) {
        eprintln!("paper-runner: {error}");
        return ExitCode::FAILURE;
    }
    let services = match build_services(&args).await {
        Ok(services) => services,
        Err(error) => {
            eprintln!("paper-runner: {error}");
            return ExitCode::FAILURE;
        }
    };

    eprintln!("paper-runner: started");
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let shutdown_started = Arc::new(OnceLock::new());
    // Keep the listener active for `--once` too: a one-shot run can still be
    // in a database/preview operation when systemd or a test harness sends
    // SIGTERM, and it must receive the same bounded cancellation path.
    let signal_tx = shutdown_tx.clone();
    let signal_started = shutdown_started.clone();
    tokio::spawn(async move {
        if shutdown_signal().await.is_ok() {
            signal_started.get_or_init(std::time::Instant::now);
            let _ = signal_tx.send(true);
        }
    });

    let state_path = health_state_path();
    if std::env::var("APP_ENV").as_deref() == Ok("production") && state_path.is_none() {
        eprintln!("paper-runner: PAPER_HEALTH_STATE_PATH is required in production");
        return ExitCode::FAILURE;
    }
    let initial_progress = HealthProgress::new(Utc::now());
    if let Some(path) = state_path.as_deref()
        && let Err(error) = write_health_state(path, &initial_progress.state(Utc::now()))
    {
        eprintln!("paper-runner: health state unavailable: {error}");
        return ExitCode::FAILURE;
    }
    let (progress_tx, progress_rx) = watch::channel(initial_progress);
    let (health_error_tx, mut health_error_rx) = mpsc::channel::<String>(1);
    let health_max_age =
        match env_duration_secs("PAPER_HEALTH_MAX_AGE_SECS", Duration::from_secs(30)) {
            Ok(value) => value,
            Err(error) => {
                eprintln!("paper-runner: {error}");
                return ExitCode::FAILURE;
            }
        };
    if health_max_age.is_zero() {
        eprintln!("paper-runner: PAPER_HEALTH_MAX_AGE_SECS must be positive");
        return ExitCode::FAILURE;
    }
    let health_task = state_path.clone().map(|path| {
        tokio::spawn(maintain_health_state(
            path,
            progress_rx,
            shutdown_rx.clone(),
            health_error_tx,
            health_max_age,
        ))
    });

    if let Err(error) = systemd_notify("READY=1\nSTATUS=Paper runner is polling").await {
        eprintln!("paper-runner: {error}");
        return ExitCode::FAILURE;
    }

    let mut cycle_id = 0_u64;
    let mut exit = ExitCode::SUCCESS;
    loop {
        if *shutdown_rx.borrow() {
            break;
        }
        if health_error_rx.try_recv().is_ok() {
            eprintln!("paper-runner: health state writer stopped");
            exit = ExitCode::FAILURE;
            break;
        }
        let date = match configured_date(&args) {
            Ok(date) => date,
            Err(error) => {
                eprintln!("paper-runner: {error}");
                exit = ExitCode::FAILURE;
                break;
            }
        };
        cycle_id = cycle_id.saturating_add(1);
        let started_at = Utc::now();
        let prior_progress = progress_tx.borrow().clone();
        let _ = progress_tx.send(HealthProgress {
            last_progress_at: started_at,
            phase: "cycle_started".to_owned(),
            cycle_id,
            cycle_in_progress: true,
            cycle_started_at: Some(started_at),
            cycle_deadline_at: Some(
                started_at
                    + chrono::Duration::from_std(args.cycle_deadline)
                        .expect("finite cycle timeout"),
            ),
            last_cycle_completed_at: prior_progress.last_cycle_completed_at,
            last_cycle_outcome: prior_progress.last_cycle_outcome,
        });
        let cycle = run_cycle_with_shutdown(&services, date, Some(shutdown_rx.clone())).await;
        match cycle {
            Ok(report) => eprintln!(
                "paper-runner: date={date} preview_outcome={} preview_compute_ms={} previews={}/{}/{} targets={}/{} valuations={}/{} outbox_backlog={} outbox_oldest_age_secs={} outbox_failed={} outbox_exhausted={} outbox_ready={} errors={}",
                report.preview_outcome,
                report.preview_compute_ms,
                report.previews_published,
                report.previews_failed,
                report.previews_seen,
                report.targets_settled,
                report.targets_seen,
                report.valuations_written,
                report.valuations_seen,
                report.notification_backlog,
                report.notification_oldest_age_secs,
                report.notification_failed,
                report.notification_exhausted,
                report.notification_ready,
                report.item_errors.len()
            ),
            Err(api_server::paper_runner::RunnerError::Shutdown) => {
                eprintln!("paper-runner: shutdown requested during cycle");
                break;
            }
            Err(error) => {
                eprintln!("paper-runner: cycle failed: {error}");
                if args.once {
                    let _ = progress_tx.send(HealthProgress {
                        last_progress_at: Utc::now(),
                        phase: "cycle_failed".to_owned(),
                        cycle_id,
                        cycle_in_progress: false,
                        cycle_started_at: None,
                        cycle_deadline_at: None,
                        last_cycle_completed_at: Some(Utc::now()),
                        last_cycle_outcome: Some("failed".to_owned()),
                    });
                    exit = ExitCode::FAILURE;
                    break;
                }
                let _ = progress_tx.send(HealthProgress {
                    last_progress_at: Utc::now(),
                    phase: "cycle_failed".to_owned(),
                    cycle_id,
                    cycle_in_progress: false,
                    cycle_started_at: None,
                    cycle_deadline_at: None,
                    last_cycle_completed_at: Some(Utc::now()),
                    last_cycle_outcome: Some("failed".to_owned()),
                });
                let mut stop = shutdown_rx.clone();
                tokio::select! {
                    biased;
                    _ = stop.changed() => break,
                    _ = tokio::time::sleep(ERROR_SLEEP) => continue,
                }
            }
        }
        let now = Utc::now();
        let _ = progress_tx.send(HealthProgress {
            last_progress_at: now,
            phase: "cycle_completed".to_owned(),
            cycle_id,
            cycle_in_progress: false,
            cycle_started_at: None,
            cycle_deadline_at: None,
            last_cycle_completed_at: Some(now),
            last_cycle_outcome: Some("succeeded".to_owned()),
        });
        if args.once {
            break;
        }
        let mut stop = shutdown_rx.clone();
        tokio::select! {
            biased;
            _ = stop.changed() => break,
            _ = tokio::time::sleep(IDLE_SLEEP) => {}
        }
    }
    shutdown_started.get_or_init(std::time::Instant::now);
    let _ = shutdown_tx.send(true);
    drop(progress_tx);
    let shutdown_remaining = || {
        shutdown_started
            .get()
            .map(|started| args.shutdown_grace.saturating_sub(started.elapsed()))
            .unwrap_or(args.shutdown_grace)
    };
    if let Some(task) = health_task {
        let remaining = shutdown_remaining();
        if !remaining.is_zero() {
            let _ = tokio::time::timeout(remaining, task).await;
        }
    }
    let remaining = shutdown_remaining();
    if !remaining.is_zero() {
        let _ = tokio::time::timeout(remaining, services.worker_pool.close()).await;
    }
    exit
}

fn main() -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("paper-runner: runtime initialization failed: {error}");
            return ExitCode::FAILURE;
        }
    };
    let exit = runtime.block_on(async_main());
    // `spawn_blocking` cannot be force-aborted after its closure starts.  The
    // runtime shutdown timeout is the final process-level bound for a stuck
    // preview read/calculation; the semaphore in paper_preview prevents it
    // from multiplying while this process drains.
    runtime.shutdown_timeout(Duration::from_secs(2));
    exit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    async fn receive_notification(socket: std::os::unix::net::UnixDatagram) -> String {
        tokio::task::spawn_blocking(move || {
            let mut bytes = [0_u8; 128];
            let count = socket.recv(&mut bytes).expect("receive systemd datagram");
            String::from_utf8(bytes[..count].to_vec()).expect("UTF-8 systemd datagram")
        })
        .await
        .expect("notification receiver task")
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn systemd_notify_sends_ready_and_watchdog_to_pathname_socket() {
        let directory = tempfile::tempdir().expect("notification socket directory");
        let path = directory.path().join("notify.sock");
        let receiver = std::os::unix::net::UnixDatagram::bind(&path)
            .expect("bind pathname notification socket");

        send_systemd_notify_to(path.as_os_str(), "READY=1")
            .await
            .expect("send READY notification");
        assert_eq!(receive_notification(receiver).await, "READY=1");
        std::fs::remove_file(&path).expect("remove pathname notification socket");

        let receiver = std::os::unix::net::UnixDatagram::bind(&path)
            .expect("rebind pathname notification socket");
        send_systemd_notify_to(path.as_os_str(), "WATCHDOG=1")
            .await
            .expect("send WATCHDOG notification");
        assert_eq!(receive_notification(receiver).await, "WATCHDOG=1");
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn systemd_notify_sends_ready_and_watchdog_to_abstract_socket() {
        use socket2::{Domain, Socket, Type};

        let name = format!(
            "lagrange-paper-notify-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        );
        let address = abstract_notify_addr(name.as_bytes()).expect("abstract notification address");
        let receiver =
            Socket::new(Domain::UNIX, Type::DGRAM, None).expect("abstract notification socket");
        receiver
            .bind(&address)
            .expect("bind abstract notification socket");
        let receiver: std::os::unix::net::UnixDatagram = receiver.into();
        let environment_name = std::ffi::OsString::from(format!("@{name}"));

        send_systemd_notify_to(&environment_name, "READY=1")
            .await
            .expect("send READY notification");
        assert_eq!(receive_notification(receiver).await, "READY=1");

        let address = abstract_notify_addr(name.as_bytes()).expect("abstract notification address");
        let receiver =
            Socket::new(Domain::UNIX, Type::DGRAM, None).expect("abstract notification socket");
        receiver
            .bind(&address)
            .expect("rebind abstract notification socket");
        let receiver: std::os::unix::net::UnixDatagram = receiver.into();
        send_systemd_notify_to(&environment_name, "WATCHDOG=1")
            .await
            .expect("send WATCHDOG notification");
        assert_eq!(receive_notification(receiver).await, "WATCHDOG=1");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn systemd_notify_rejects_an_empty_abstract_name() {
        let error = send_systemd_notify_to(std::ffi::OsStr::new("@"), "READY=1")
            .await
            .expect_err("empty abstract name must fail startup notification");
        assert!(
            error.contains("abstract notify socket name is empty"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn shutdown_selection_observes_sigterm_future() {
        let result = wait_for_shutdown(
            std::future::pending::<std::io::Result<()>>(),
            std::future::ready(Ok(())),
        )
        .await;
        assert!(result.is_ok());
    }

    #[test]
    fn stale_progress_state_is_not_silently_accepted_by_contract() {
        // The runtime wrapper owns the cross-process parser. Keep the marker
        // names asserted here so a refactor cannot accidentally remove the
        // bounded-cycle fields that healthcheck consumes.
        let source = include_str!("../../../../deploy/runtime/paper-runner-entrypoint");
        assert!(source.contains("cycle_in_progress"));
        assert!(source.contains("cycle_deadline_at"));
        assert!(source.contains("loop progress is stale"));
    }
}
