#![allow(dead_code)]

mod api_faults;

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use api_server::http::api_router;
use api_server::http::owner_market_stream_config::{MarketStreamReadConfig, MarketStreamReadPins};
use api_server::http::state::{
    ApiConfig, ApiState, OwnerBetaAccessMode, OwnerBetaEquitySignalsMode, OwnerBetaPaperMode,
    OwnerBetaPriceInputMode, OwnerIntradayQuoteReadConfig,
};
use auth::sessions::cookie;
use base64::Engine;
use chrono::{DateTime, FixedOffset, NaiveDate, SecondsFormat, Utc};
use collectors::intraday_quotes::IntradaySessionWindowContract;
use futures_util::FutureExt;
use job_queue::owner_equity_v2::{
    IntradaySessionProof, MarketStreamRuntimeError, MarketStreamRuntimeExit,
    OwnerIntradayQuoteRepository, OwnerMarketStreamRepository, OwnerMarketStreamRuntime,
    OwnerMarketStreamRuntimeConfig,
    StreamBufferProbeSummary, StreamPublicationProbe, StreamPublicationProbeOutcome,
    StreamPublicationProbeRecord, StreamPublicationProbeSnapshot, StreamPublicationProbeSummary,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UnixListener};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior, sleep_until, timeout_at};
use uuid::Uuid;

use super::boundary_support as boundary;
use super::owner_market_stream_http_support::{role_pool, seed_http_sessions};
use super::resource_measurements::{
    HeapObservation, HeapWindow, buffer_summary_is_valid, process_rss_bytes,
};

const CONTROL_PREFIX: &str = "/tmp/lagrange-kis-integrated-";
const MAX_CONFIG_BYTES: usize = 8 * 1024;
const MAX_HTTP_HEADER_BYTES: usize = 16 * 1024;
const MAX_HTTP_BODY_BYTES: usize = 16 * 1024;
const MAX_WS_FRAME_BYTES: usize = 64 * 1024;
const MAX_OBSERVATION_LINES: usize = 3_000;
const MAX_OBSERVATION_BYTES: usize = 4 * 1024 * 1024;
const MAX_COMMAND_EVENTS: usize = 256;
const MAX_PUBLICATION_RECORDS: usize = 12_000;
const MAX_PUBLICATION_BYTES: usize = 4 * 1024 * 1024;
const SYNTHETIC_APP_KEY: &str = "WP7_SYNTHETIC_APP_KEY";
const SYNTHETIC_APP_SECRET: &str = "WP7_SYNTHETIC_APP_SECRET";
const SYNTHETIC_APPROVAL_KEY: &str = "WP7_SYNTHETIC_APPROVAL_KEY";
const SETUP_BOUND: Duration = Duration::from_secs(60);
const CLEANUP_BOUND: Duration = Duration::from_secs(45);
const OBSERVATION_BOUND: Duration = Duration::from_secs(2);
const FIXED_CONTRACT_HASH: &str = boundary::NETWORK_HASH;

type FixtureResult<T = ()> = Result<T, FixtureFailure>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FixtureFailure {
    ControlInvalid,
    SetupFailed,
    SetupDeadline,
    DateMismatch,
    WindowInvalid,
    CalendarSeedFailed,
    InsufficientWindow,
    ApiSetupFailed,
    ListenerSetupFailed,
    RuntimeSetupFailed,
    NotInitiallyIdle,
    StopFileInvalid,
    WorkDeadline,
    ObservationFailed,
    ObservationLimit,
    DaemonExitedEarly,
    DaemonFailed,
    ServerFailed,
    ApprovalFailed,
    WebsocketFailed,
    TaskPanicked,
    TaskJoinFailed,
    ForcedAbort,
    CleanupFailed,
    ResultWriteFailed,
    MetricsPoisoned,
    PublicationProbeRegistrationFailed,
    PublicationProbeMissing,
    PublicationMeasurementFailed,
    PublicationEvidenceLimitExceeded,
    PublicationEvidenceFailed,
    ResourceProbeMissing,
    ResourceMeasurementFailed,
    UnexpectedPanic,
}

impl FixtureFailure {
    pub(super) const fn code(self) -> &'static str {
        match self {
            Self::ControlInvalid => "CONTROL_INVALID",
            Self::SetupFailed => "SETUP_FAILED",
            Self::SetupDeadline => "SETUP_DEADLINE",
            Self::DateMismatch => "DATE_MISMATCH",
            Self::WindowInvalid => "WINDOW_INVALID",
            Self::CalendarSeedFailed => "CALENDAR_SEED_FAILED",
            Self::InsufficientWindow => "INSUFFICIENT_WINDOW",
            Self::ApiSetupFailed => "API_SETUP_FAILED",
            Self::ListenerSetupFailed => "LISTENER_SETUP_FAILED",
            Self::RuntimeSetupFailed => "RUNTIME_SETUP_FAILED",
            Self::NotInitiallyIdle => "NOT_INITIALLY_IDLE",
            Self::StopFileInvalid => "STOP_FILE_INVALID",
            Self::WorkDeadline => "WORK_DEADLINE",
            Self::ObservationFailed => "OBSERVATION_FAILED",
            Self::ObservationLimit => "OBSERVATION_LIMIT",
            Self::DaemonExitedEarly => "DAEMON_EXITED_EARLY",
            Self::DaemonFailed => "DAEMON_FAILED",
            Self::ServerFailed => "SERVER_FAILED",
            Self::ApprovalFailed => "APPROVAL_FAILED",
            Self::WebsocketFailed => "WEBSOCKET_FAILED",
            Self::TaskPanicked => "TASK_PANICKED",
            Self::TaskJoinFailed => "TASK_JOIN_FAILED",
            Self::ForcedAbort => "FORCED_ABORT",
            Self::CleanupFailed => "CLEANUP_FAILED",
            Self::ResultWriteFailed => "RESULT_WRITE_FAILED",
            Self::MetricsPoisoned => "METRICS_POISONED",
            Self::PublicationProbeRegistrationFailed => "PUBLICATION_PROBE_REGISTRATION_FAILED",
            Self::PublicationProbeMissing => "PUBLICATION_PROBE_MISSING",
            Self::PublicationMeasurementFailed => "PUBLICATION_MEASUREMENT_FAILED",
            Self::PublicationEvidenceLimitExceeded => "PUBLICATION_EVIDENCE_LIMIT_EXCEEDED",
            Self::PublicationEvidenceFailed => "PUBLICATION_EVIDENCE_FAILED",
            Self::ResourceProbeMissing => "RESOURCE_PROBE_MISSING",
            Self::ResourceMeasurementFailed => "RESOURCE_MEASUREMENT_FAILED",
            Self::UnexpectedPanic => "UNEXPECTED_PANIC",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ControlConfig {
    schema_version: u32,
    origin: String,
    max_work_seconds: u64,
    #[serde(default)]
    fault_scenario: Option<api_faults::Scenario>,
}

struct ControlSettings {
    directory: PathBuf,
    origin: String,
    max_work: Duration,
    fault_scenario: Option<api_faults::Scenario>,
}

impl ControlSettings {
    fn read() -> FixtureResult<Self> {
        let directory = std::env::var_os("LAGRANGE_MARKET_STREAM_E2E_DIR")
            .and_then(|value| value.into_string().ok())
            .ok_or(FixtureFailure::ControlInvalid)?;
        let digest = std::env::var("LAGRANGE_MARKET_STREAM_E2E_CONFIG_SHA256")
            .map_err(|_| FixtureFailure::ControlInvalid)?;
        if !canonical_lower_hex(&digest, 64) {
            return Err(FixtureFailure::ControlInvalid);
        }
        let suffix = directory
            .strip_prefix(CONTROL_PREFIX)
            .ok_or(FixtureFailure::ControlInvalid)?;
        if suffix.len() != 32 || !canonical_lower_hex(suffix, 32) {
            return Err(FixtureFailure::ControlInvalid);
        }
        let directory = PathBuf::from(directory);
        let directory_metadata =
            fs::symlink_metadata(&directory).map_err(|_| FixtureFailure::ControlInvalid)?;
        if directory_metadata.file_type().is_symlink()
            || !directory_metadata.is_dir()
            || directory_metadata.uid() != effective_uid()
            || directory_metadata.mode() & 0o7777 != 0o700
        {
            return Err(FixtureFailure::ControlInvalid);
        }
        let config_path = directory.join("config.json");
        let config_bytes = read_private_file(&config_path, MAX_CONFIG_BYTES)?;
        let actual_digest = format!("{:x}", Sha256::digest(&config_bytes));
        if actual_digest != digest {
            return Err(FixtureFailure::ControlInvalid);
        }
        let config: ControlConfig =
            serde_json::from_slice(&config_bytes).map_err(|_| FixtureFailure::ControlInvalid)?;
        if config.schema_version != 1
            || !(60..=2_400).contains(&config.max_work_seconds)
            || !canonical_https_origin(&config.origin)
        {
            return Err(FixtureFailure::ControlInvalid);
        }
        for child in [
            "api.sock",
            "ready.json",
            "observations.jsonl",
            "stop",
            "result.json",
            "publication.json",
            "fault-01.json",
            "fault-01-result.json",
            "fault-02.json",
            "fault-02-result.json",
        ] {
            require_absent(&directory.join(child))?;
        }
        Ok(Self {
            directory,
            origin: config.origin,
            max_work: Duration::from_secs(config.max_work_seconds),
            fault_scenario: config.fault_scenario,
        })
    }
}

fn effective_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and returns the current process uid.
    unsafe { libc::geteuid() }
}

fn canonical_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn canonical_https_origin(origin: &str) -> bool {
    let Some(port) = origin.strip_prefix("https://127.0.0.1:") else {
        return false;
    };
    let Ok(port) = port.parse::<u16>() else {
        return false;
    };
    port != 0 && origin == format!("https://127.0.0.1:{port}")
}

fn require_absent(path: &Path) -> FixtureResult {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(FixtureFailure::ControlInvalid),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(FixtureFailure::ControlInvalid),
    }
}

fn read_private_file(path: &Path, maximum: usize) -> FixtureResult<Vec<u8>> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| FixtureFailure::ControlInvalid)?;
    let metadata = file
        .metadata()
        .map_err(|_| FixtureFailure::ControlInvalid)?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o7777 != 0o600
        || metadata.len() > maximum as u64
    {
        return Err(FixtureFailure::ControlInvalid);
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(maximum.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| FixtureFailure::ControlInvalid)?;
    if bytes.len() > maximum {
        return Err(FixtureFailure::ControlInvalid);
    }
    Ok(bytes)
}

fn read_stop_file(directory: &Path) -> FixtureResult<bool> {
    let path = directory.join("stop");
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(FixtureFailure::StopFileInvalid),
        Ok(_) => {
            let bytes = read_private_file(&path, 5).map_err(|_| FixtureFailure::StopFileInvalid)?;
            if bytes == b"STOP\n" {
                Ok(true)
            } else {
                Err(FixtureFailure::StopFileInvalid)
            }
        }
    }
}

#[derive(Clone, Serialize, Default)]
struct CommandEvent {
    elapsed_ms: u64,
    symbol: String,
    operation: &'static str,
}

#[derive(Clone, Serialize, Default)]
struct Metrics {
    approval_accepts: u64,
    websocket_accepts: u64,
    active_websockets: u64,
    peak_websockets: u64,
    subscribe_commands: u64,
    unsubscribe_commands: u64,
    command_events: Vec<CommandEvent>,
    quote_frames: u64,
    quote_records: u64,
    unexpected_commands_or_requests: u64,
    server_errors: u64,
    event_overflow: bool,
    client_close_seen: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct TransportSummary {
    approval_accepts: u64,
    websocket_accepts: u64,
    active_websockets: u64,
    peak_websockets: u64,
    subscribe_commands: u64,
    unsubscribe_commands: u64,
    quote_frames: u64,
    quote_records: u64,
    unexpected_commands_or_requests: u64,
    server_errors: u64,
    event_overflow: bool,
    client_close_seen: bool,
}

#[derive(Clone)]
struct MetricsHandle(Arc<Mutex<Metrics>>);

impl MetricsHandle {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(Metrics::default())))
    }

    fn update(&self, update: impl FnOnce(&mut Metrics)) -> FixtureResult {
        let mut metrics = self.0.lock().map_err(|_| FixtureFailure::MetricsPoisoned)?;
        update(&mut metrics);
        Ok(())
    }

    fn snapshot(&self) -> FixtureResult<Metrics> {
        self.0
            .lock()
            .map(|value| value.clone())
            .map_err(|_| FixtureFailure::MetricsPoisoned)
    }

    fn aggregate(&self) -> FixtureResult<TransportSummary> {
        let metrics = self.0.lock().map_err(|_| FixtureFailure::MetricsPoisoned)?;
        Ok(TransportSummary {
            approval_accepts: metrics.approval_accepts,
            websocket_accepts: metrics.websocket_accepts,
            active_websockets: metrics.active_websockets,
            peak_websockets: metrics.peak_websockets,
            subscribe_commands: metrics.subscribe_commands,
            unsubscribe_commands: metrics.unsubscribe_commands,
            quote_frames: metrics.quote_frames,
            quote_records: metrics.quote_records,
            unexpected_commands_or_requests: metrics.unexpected_commands_or_requests,
            server_errors: metrics.server_errors,
            event_overflow: metrics.event_overflow,
            client_close_seen: metrics.client_close_seen,
        })
    }
}

struct ActiveTask(Arc<AtomicUsize>);

impl ActiveTask {
    fn enter(counter: Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::AcqRel);
        Self(counter)
    }
}

impl Drop for ActiveTask {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Serialize)]
struct ReadyDocument {
    schema_version: u32,
    status: &'static str,
    origin: String,
    api_socket: String,
    generated_database: String,
    owner_user_id: Uuid,
    credential_slot_id: Uuid,
    grant_id: Uuid,
    session_date: NaiveDate,
    window_contract_sha256: String,
    cookie_name: &'static str,
    cookie_value: String,
    identities: Vec<ReadyIdentity>,
}

#[derive(Serialize)]
struct ReadyIdentity {
    membership_id: Uuid,
    instrument_id: String,
    generation: u64,
}

#[derive(Serialize)]
struct Observation {
    elapsed_ms: u64,
    active_leases: i64,
    desired_count: i64,
    desired_reference_sum: i64,
    cache_rows: i64,
    quote_version_sum: i64,
    producer_state: Option<String>,
    daemon_finished: bool,
    transport: TransportSummary,
    publication: StreamPublicationProbeSummary,
    buffer: StreamBufferProbeSummary,
    heap: HeapObservation,
    process_rss_bytes: u64,
}

#[derive(Serialize)]
struct ResultDocument {
    schema_version: u32,
    status: &'static str,
    primary_failure: Option<&'static str>,
    cleanup_failure: Option<&'static str>,
    forced_abort: bool,
    daemon_exit: &'static str,
    elapsed_ms: u64,
    transport: Metrics,
    active_fixture_tasks: usize,
    publication_summary: Option<StreamPublicationProbeSummary>,
    publication_evidence_written: bool,
    buffer_summary: Option<StreamBufferProbeSummary>,
    heap_summary: Option<HeapObservation>,
    process_rss_bytes: Option<u64>,
    resource_failure: Option<&'static str>,
}

struct OwnedFixture {
    settings: ControlSettings,
    started: Instant,
    setup_deadline: Instant,
    cleanup_deadline: Instant,
    work_deadline: Option<Instant>,
    database: Option<boundary::DisposableDatabase>,
    fixture: Option<boundary::Fixture>,
    audit_pool: Option<PgPool>,
    window: Option<Arc<IntradaySessionWindowContract>>,
    window_sha256: Option<String>,
    database_name: Option<String>,
    state_directory: Option<tempfile::TempDir>,
    observations: Option<File>,
    observation_lines: usize,
    observation_bytes: usize,
    metrics: MetricsHandle,
    publication_probe: Option<StreamPublicationProbe>,
    heap_window: Option<HeapWindow>,
    active_tasks: Arc<AtomicUsize>,
    api_socket: Option<PathBuf>,
    restart_config: Option<ApiConfig>,
    api_shutdown: Option<watch::Sender<bool>>,
    daemon_shutdown: Option<watch::Sender<bool>>,
    fixture_shutdown: Option<watch::Sender<bool>>,
    window_watch: Option<watch::Sender<Option<IntradaySessionWindowContract>>>,
    api_task: Option<JoinHandle<FixtureResult>>,
    daemon_task: Option<JoinHandle<Result<MarketStreamRuntimeExit, MarketStreamRuntimeError>>>,
    approval_task: Option<JoinHandle<FixtureResult>>,
    websocket_task: Option<JoinHandle<FixtureResult>>,
    api_fault_step: u8,
    slow_listener_baseline: Option<i32>,
    rights_quote_version: Option<i64>,
    generation_quote_version: Option<i64>,
    forced_abort: bool,
    daemon_exit: &'static str,
}

impl OwnedFixture {
    fn new(settings: ControlSettings) -> Self {
        let started = Instant::now();
        let setup_deadline = started + SETUP_BOUND;
        let cleanup_deadline = setup_deadline + settings.max_work + CLEANUP_BOUND;
        Self {
            settings,
            started,
            setup_deadline,
            cleanup_deadline,
            work_deadline: None,
            database: None,
            fixture: None,
            audit_pool: None,
            window: None,
            window_sha256: None,
            database_name: None,
            state_directory: None,
            observations: None,
            observation_lines: 0,
            observation_bytes: 0,
            metrics: MetricsHandle::new(),
            publication_probe: None,
            heap_window: None,
            active_tasks: Arc::new(AtomicUsize::new(0)),
            api_socket: None,
            restart_config: None,
            api_shutdown: None,
            daemon_shutdown: None,
            fixture_shutdown: None,
            window_watch: None,
            api_task: None,
            daemon_task: None,
            approval_task: None,
            websocket_task: None,
            api_fault_step: 0,
            slow_listener_baseline: None,
            rights_quote_version: None,
            generation_quote_version: None,
            forced_abort: false,
            daemon_exit: "NOT_STARTED",
        }
    }

    async fn prepare(&mut self) -> FixtureResult {
        if Instant::now() >= self.setup_deadline {
            return Err(FixtureFailure::SetupDeadline);
        }
        let database = boundary::DisposableDatabase::create_until(
            self.setup_deadline,
            self.setup_deadline + CLEANUP_BOUND,
        )
        .await
        .map_err(|_| FixtureFailure::SetupFailed)?;
        self.database = Some(database);

        let fixture = timeout_at(
            self.setup_deadline,
            boundary::seed_fixture(self.database.as_ref().ok_or(FixtureFailure::SetupFailed)?),
        )
        .await
        .map_err(|_| FixtureFailure::SetupDeadline)?
        .map_err(|_| FixtureFailure::SetupFailed)?;
        self.fixture = Some(fixture);

        let database = self.database.as_ref().ok_or(FixtureFailure::SetupFailed)?;
        let fixture = self.fixture.as_ref().ok_or(FixtureFailure::SetupFailed)?;
        let now = timeout_at(
            self.setup_deadline,
            sqlx::query_scalar::<_, DateTime<Utc>>("SELECT pg_catalog.clock_timestamp()")
                .fetch_one(&database.migration_owner),
        )
        .await
        .map_err(|_| FixtureFailure::SetupDeadline)?
        .map_err(|_| FixtureFailure::SetupFailed)?;
        let kst = FixedOffset::east_opt(9 * 60 * 60).ok_or(FixtureFailure::DateMismatch)?;
        if now.with_timezone(&kst).date_naive() != fixture.session_date {
            return Err(FixtureFailure::DateMismatch);
        }
        let (contract, digest) = make_window_contract(fixture.session_date, now)?;
        timeout_at(
            self.setup_deadline,
            seed_calendar_lineage(database, fixture, &digest, now),
        )
        .await
        .map_err(|_| FixtureFailure::SetupDeadline)?
        .map_err(|_| FixtureFailure::CalendarSeedFailed)?;
        let (_, close_at) = window_bounds(fixture.session_date)?;
        let remaining_setup = self
            .setup_deadline
            .saturating_duration_since(Instant::now());
        let required = remaining_setup + self.settings.max_work + CLEANUP_BOUND;
        if close_at
            .signed_duration_since(now)
            .to_std()
            .ok()
            .is_none_or(|left| left < required)
        {
            return Err(FixtureFailure::InsufficientWindow);
        }
        self.window = Some(Arc::new(contract));
        self.window_sha256 = Some(digest);

        let users = timeout_at(self.setup_deadline, seed_http_sessions(database, fixture))
            .await
            .map_err(|_| FixtureFailure::SetupDeadline)?
            .map_err(|_| FixtureFailure::SetupFailed)?;
        let audit_pool = timeout_at(
            self.setup_deadline,
            role_pool(database, "audit_writer", 8, "wp7i-audit"),
        )
        .await
        .map_err(|_| FixtureFailure::SetupDeadline)?
        .map_err(|_| FixtureFailure::SetupFailed)?;
        self.audit_pool = Some(audit_pool.clone());

        let window = self.window.as_ref().ok_or(FixtureFailure::WindowInvalid)?;
        let origin = self.settings.origin.clone();
        let stream_pins = MarketStreamReadPins::new(
            origin.clone(),
            fixture.credential_slot_id,
            fixture.grant_id,
            FIXED_CONTRACT_HASH.to_owned(),
            Some(Arc::clone(window)),
        )
        .map_err(|_| FixtureFailure::ApiSetupFailed)?;
        let api_config = ApiConfig {
            cursor_secret: [0x57; 32],
            max_jobs_per_owner: 10,
            recommendation_dataset: job_queue::recommendation::input::DatasetPin {
                id: Uuid::from_u128(0x7701),
                dataset_id: "wp7i-synthetic".to_owned(),
                version: "fixture".to_owned(),
                curated_version: 1,
                manifest_sha256: "0".repeat(64),
            },
            // The selected router only uses the role pools supplied below.
            db_url: "postgres://unused:unused@fixture.invalid/unused".to_owned(),
            step_up_max_auth_age_secs: 900,
            artifact_root: PathBuf::from("/unused"),
            seoul_today: api_server::http::state::system_seoul_today,
            candidate_eod_ready: api_server::http::state::system_candidate_eod_ready,
            code_commit: boundary::CODE_COMMIT.to_owned(),
            owner_beta_access: OwnerBetaAccessMode::Disabled,
            owner_beta_paper: OwnerBetaPaperMode::Disabled,
            owner_beta_price_input: OwnerBetaPriceInputMode::Disabled,
            owner_beta_equity_signals: OwnerBetaEquitySignalsMode::Disabled,
            stock_price_beta_artifact_root: PathBuf::from("/unused"),
            owner_equity_v2_pins: None,
            owner_equity_v2_api_artifact_root: None,
            owner_intraday_quotes: OwnerIntradayQuoteReadConfig::OwnerOnly {
                window: Some(Arc::clone(window)),
            },
            owner_market_stream: MarketStreamReadConfig::OwnerOnly(stream_pins),
            intraday_now: api_server::http::state::system_intraday_now,
        };
        if self.settings.fault_scenario == Some(api_faults::Scenario::ApiRestart) {
            self.restart_config = Some(api_config.clone());
        }
        let state = timeout_at(
            self.setup_deadline,
            ApiState::from_pools(
                api_config,
                database.app.clone(),
                database.admin.clone(),
                audit_pool,
            ),
        )
        .await
        .map_err(|_| FixtureFailure::SetupDeadline)?
        .map_err(|_| FixtureFailure::ApiSetupFailed)?;
        timeout_at(self.setup_deadline, state.check_readiness())
            .await
            .map_err(|_| FixtureFailure::SetupDeadline)?
            .map_err(|_| FixtureFailure::ApiSetupFailed)?;
        let router = api_router(state);

        let directory = self.settings.directory.clone();
        let socket_path = directory.join("api.sock");
        let unix_listener =
            UnixListener::bind(&socket_path).map_err(|_| FixtureFailure::ListenerSetupFailed)?;
        self.api_socket = Some(socket_path.clone());
        fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))
            .map_err(|_| FixtureFailure::ListenerSetupFailed)?;
        let socket_metadata =
            fs::symlink_metadata(&socket_path).map_err(|_| FixtureFailure::ListenerSetupFailed)?;
        if !socket_metadata.file_type().is_socket()
            || socket_metadata.uid() != effective_uid()
            || socket_metadata.mode() & 0o7777 != 0o600
        {
            return Err(FixtureFailure::ListenerSetupFailed);
        }
        let approval_listener = timeout_at(self.setup_deadline, TcpListener::bind("127.0.0.1:0"))
            .await
            .map_err(|_| FixtureFailure::SetupDeadline)?
            .map_err(|_| FixtureFailure::ListenerSetupFailed)?;
        let approval_addr = approval_listener
            .local_addr()
            .map_err(|_| FixtureFailure::ListenerSetupFailed)?;
        let websocket_listener = timeout_at(self.setup_deadline, TcpListener::bind("127.0.0.1:0"))
            .await
            .map_err(|_| FixtureFailure::SetupDeadline)?
            .map_err(|_| FixtureFailure::ListenerSetupFailed)?;
        let websocket_addr = websocket_listener
            .local_addr()
            .map_err(|_| FixtureFailure::ListenerSetupFailed)?;
        let state_directory = tempfile::tempdir().map_err(|_| FixtureFailure::SetupFailed)?;
        let domain = kis_client::market_stream_state::MarketStreamDomain::for_test(
            state_directory.path(),
            fixture.credential_slot_id,
        )
        .map_err(|_| FixtureFailure::RuntimeSetupFailed)?;
        self.state_directory = Some(state_directory);

        let (fixture_stop, fixture_stop_rx) = watch::channel(false);
        self.fixture_shutdown = Some(fixture_stop);
        let max_sessions = if matches!(
            self.settings.fault_scenario,
            Some(api_faults::Scenario::WindowCycle | api_faults::Scenario::GenerationChange)
        ) {
            2
        } else {
            1
        };
        let metrics = self.metrics.clone();
        let active = Arc::clone(&self.active_tasks);
        self.approval_task = Some(tokio::spawn(async move {
            let _active = ActiveTask::enter(active);
            approval_accept_loop(approval_listener, fixture_stop_rx, metrics, max_sessions).await
        }));

        let websocket_stop_rx = self
            .fixture_shutdown
            .as_ref()
            .ok_or(FixtureFailure::RuntimeSetupFailed)?
            .subscribe();
        let metrics = self.metrics.clone();
        let active = Arc::clone(&self.active_tasks);
        let date = fixture.session_date;
        let started = self.started;
        let websocket_lifetime_deadline =
            self.started + SETUP_BOUND + self.settings.max_work + CLEANUP_BOUND;
        let websocket_task = tokio::spawn(async move {
            let _active = ActiveTask::enter(active);
            websocket_accept_loop(
                websocket_listener,
                websocket_stop_rx,
                metrics,
                date,
                started,
                websocket_lifetime_deadline,
                max_sessions,
            )
            .await
        });
        self.websocket_task = Some(websocket_task);

        let (window_watch, window_rx) = watch::channel(Some(window.as_ref().clone()));
        self.window_watch = Some(window_watch);
        let (daemon_shutdown, daemon_rx) = watch::channel(false);
        self.daemon_shutdown = Some(daemon_shutdown);
        let runtime_config = OwnerMarketStreamRuntimeConfig::from_values(
            fixture.credential_slot_id,
            fixture.grant_id,
            FIXED_CONTRACT_HASH,
            7,
            Uuid::new_v4(),
        )
        .map_err(|_| FixtureFailure::RuntimeSetupFailed)?;
        let worker_pool = database.worker.clone();
        let approval_origin = format!("http://{approval_addr}");
        let websocket_endpoint = format!("ws://{websocket_addr}/tryitout");
        let active = Arc::clone(&self.active_tasks);
        self.publication_probe = Some(
            StreamPublicationProbe::register(fixture.credential_slot_id)
                .map_err(|_| FixtureFailure::PublicationProbeRegistrationFailed)?,
        );
        self.heap_window = Some(
            HeapWindow::begin(&super::TEST_HEAP)
                .map_err(|_| FixtureFailure::ResourceMeasurementFailed)?,
        );
        self.daemon_task = Some(tokio::spawn(async move {
            let _active = ActiveTask::enter(active);
            OwnerMarketStreamRuntime::run_loopback_fixture(
                OwnerMarketStreamRepository::new(worker_pool.clone()),
                OwnerIntradayQuoteRepository::new(worker_pool),
                runtime_config,
                domain,
                &approval_origin,
                &websocket_endpoint,
                window_rx,
                daemon_rx,
            )
            .await
            .inspect_err(|error| {
                eprintln!("KIS_TEST_DIAG fixture_daemon error={error:?}");
            })
        }));

        let (api_shutdown, api_shutdown_rx) = watch::channel(false);
        self.api_shutdown = Some(api_shutdown);
        let active = Arc::clone(&self.active_tasks);
        self.api_task = Some(tokio::spawn(async move {
            let _active = ActiveTask::enter(active);
            axum::serve(unix_listener, router)
                .with_graceful_shutdown(wait_for_true(api_shutdown_rx))
                .await
                .map_err(|_| FixtureFailure::ServerFailed)
        }));

        let database_name: String = timeout_at(
            self.setup_deadline,
            sqlx::query_scalar("SELECT current_database()").fetch_one(&database.migration_owner),
        )
        .await
        .map_err(|_| FixtureFailure::SetupDeadline)?
        .map_err(|_| FixtureFailure::SetupFailed)?;
        self.database_name = Some(database_name);
        let initial = timeout_at(
            self.setup_deadline,
            query_observation(database, fixture.owner_user_id, fixture.credential_slot_id),
        )
        .await
        .map_err(|_| FixtureFailure::SetupDeadline)??;
        let metrics = self.metrics.snapshot()?;
        if initial.active_leases != 0
            || initial.desired_count != 0
            || metrics.approval_accepts != 0
            || metrics.websocket_accepts != 0
            || metrics.active_websockets != 0
        {
            return Err(FixtureFailure::NotInitiallyIdle);
        }

        let observations_path = directory.join("observations.jsonl");
        let observations = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&observations_path)
            .map_err(|_| FixtureFailure::ControlInvalid)?;
        self.observations = Some(observations);

        let ready = ReadyDocument {
            schema_version: 1,
            status: "READY",
            origin,
            api_socket: socket_path.to_string_lossy().into_owned(),
            generated_database: self
                .database_name
                .as_ref()
                .ok_or(FixtureFailure::SetupFailed)?
                .clone(),
            owner_user_id: fixture.owner_user_id,
            credential_slot_id: fixture.credential_slot_id,
            grant_id: fixture.grant_id,
            session_date: fixture.session_date,
            window_contract_sha256: self
                .window_sha256
                .as_ref()
                .ok_or(FixtureFailure::WindowInvalid)?
                .clone(),
            cookie_name: cookie::NAME,
            cookie_value: users.owner.cookie_value,
            identities: fixture
                .identities
                .iter()
                .map(|identity| ReadyIdentity {
                    membership_id: identity.membership_id,
                    instrument_id: identity.instrument_id.clone(),
                    generation: identity.generation,
                })
                .collect(),
        };
        atomic_json_once(&directory.join("ready.json"), &ready)
            .map_err(|_| FixtureFailure::ControlInvalid)?;
        let work_deadline = Instant::now() + self.settings.max_work;
        self.work_deadline = Some(work_deadline);
        self.cleanup_deadline = work_deadline + CLEANUP_BOUND;
        Ok(())
    }

    async fn work_until_stop(&mut self) -> FixtureResult {
        let deadline = self.work_deadline.ok_or(FixtureFailure::SetupFailed)?;
        let mut next_observation = Instant::now();
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Err(FixtureFailure::WorkDeadline);
            }
            if self
                .daemon_task
                .as_ref()
                .is_some_and(JoinHandle::is_finished)
            {
                return Err(FixtureFailure::DaemonExitedEarly);
            }
            if read_stop_file(&self.settings.directory)? {
                return Ok(());
            }
            self.poll_api_fault().await?;
            if now >= next_observation {
                let database = self.database.as_ref().ok_or(FixtureFailure::SetupFailed)?;
                let fixture = self.fixture.as_ref().ok_or(FixtureFailure::SetupFailed)?;
                let observation_deadline = (Instant::now() + OBSERVATION_BOUND).min(deadline);
                let row = timeout_at(
                    observation_deadline,
                    query_observation(database, fixture.owner_user_id, fixture.credential_slot_id),
                )
                .await
                .map_err(|_| FixtureFailure::ObservationFailed)??;
                let elapsed_ms = Instant::now()
                    .saturating_duration_since(self.started)
                    .as_millis()
                    .min(u128::from(u64::MAX)) as u64;
                let transport = self.metrics.aggregate()?;
                let publication = self
                    .publication_probe
                    .as_ref()
                    .ok_or(FixtureFailure::PublicationProbeMissing)?
                    .summary()
                    .map_err(|_| FixtureFailure::PublicationMeasurementFailed)?;
                if !publication_observation_is_valid(&publication) {
                    return Err(FixtureFailure::PublicationMeasurementFailed);
                }
                let buffer = self
                    .publication_probe
                    .as_ref()
                    .ok_or(FixtureFailure::ResourceProbeMissing)?
                    .buffer_summary()
                    .map_err(|_| FixtureFailure::ResourceMeasurementFailed)?;
                if !buffer_summary_is_valid(&buffer) {
                    return Err(FixtureFailure::ResourceMeasurementFailed);
                }
                let heap = self
                    .heap_window
                    .as_ref()
                    .ok_or(FixtureFailure::ResourceProbeMissing)?
                    .snapshot()
                    .map_err(|_| FixtureFailure::ResourceMeasurementFailed)?;
                let process_rss_bytes =
                    process_rss_bytes().map_err(|_| FixtureFailure::ResourceMeasurementFailed)?;
                let observation = Observation {
                    elapsed_ms,
                    active_leases: row.active_leases,
                    desired_count: row.desired_count,
                    desired_reference_sum: row.desired_reference_sum,
                    cache_rows: row.cache_rows,
                    quote_version_sum: row.quote_version_sum,
                    producer_state: row.producer_state,
                    daemon_finished: self
                        .daemon_task
                        .as_ref()
                        .is_some_and(JoinHandle::is_finished),
                    transport,
                    publication,
                    buffer,
                    heap,
                    process_rss_bytes,
                };
                self.append_observation(&observation)?;
                // Schedule from completion; delayed work never catches up.
                next_observation = Instant::now() + Duration::from_secs(1);
            }
            let poll = (Instant::now() + Duration::from_millis(100))
                .min(next_observation)
                .min(deadline);
            sleep_until(poll).await;
        }
    }

    fn append_observation(&mut self, observation: &Observation) -> FixtureResult {
        if self.observation_lines >= MAX_OBSERVATION_LINES {
            return Err(FixtureFailure::ObservationLimit);
        }
        let mut bytes =
            serde_json::to_vec(observation).map_err(|_| FixtureFailure::ObservationFailed)?;
        bytes.push(b'\n');
        if self.observation_bytes.saturating_add(bytes.len()) > MAX_OBSERVATION_BYTES {
            return Err(FixtureFailure::ObservationLimit);
        }
        self.observations
            .as_mut()
            .ok_or(FixtureFailure::ObservationFailed)?
            .write_all(&bytes)
            .map_err(|_| FixtureFailure::ObservationFailed)?;
        self.observations
            .as_ref()
            .ok_or(FixtureFailure::ObservationFailed)?
            .sync_data()
            .map_err(|_| FixtureFailure::ObservationFailed)?;
        self.observation_lines += 1;
        self.observation_bytes += bytes.len();
        Ok(())
    }

    async fn cleanup(&mut self) -> Option<FixtureFailure> {
        self.cleanup_deadline = self.cleanup_deadline.min(Instant::now() + CLEANUP_BOUND);
        let mut first_failure = None;
        if let Some(stop) = self.daemon_shutdown.as_ref() {
            stop.send_replace(true);
        }
        if let Some(task) = self.daemon_task.take() {
            match join_daemon(task, self.cleanup_deadline, &mut self.forced_abort).await {
                Ok(MarketStreamRuntimeExit::Shutdown) => self.daemon_exit = "SHUTDOWN",
                Ok(_) => {
                    self.daemon_exit = "UNEXPECTED_EXIT";
                    first_failure.get_or_insert(FixtureFailure::DaemonFailed);
                }
                Err(error) => {
                    self.daemon_exit = "ERROR";
                    first_failure.get_or_insert(error);
                }
            }
        }

        if let Some(stop) = self.api_shutdown.as_ref() {
            stop.send_replace(true);
        }
        if let Some(task) = self.api_task.take() {
            if let Err(error) = join_fixture_task(
                task,
                self.cleanup_deadline,
                FixtureFailure::ServerFailed,
                &mut self.forced_abort,
            )
            .await
            {
                first_failure.get_or_insert(error);
            }
        }

        if let Some(stop) = self.fixture_shutdown.as_ref() {
            stop.send_replace(true);
        }
        if let Some(task) = self.approval_task.take() {
            if let Err(error) = join_fixture_task(
                task,
                self.cleanup_deadline,
                FixtureFailure::ApprovalFailed,
                &mut self.forced_abort,
            )
            .await
            {
                first_failure.get_or_insert(error);
            }
        }
        if let Some(task) = self.websocket_task.take() {
            if let Err(error) = join_fixture_task(
                task,
                self.cleanup_deadline,
                FixtureFailure::WebsocketFailed,
                &mut self.forced_abort,
            )
            .await
            {
                first_failure.get_or_insert(error);
            }
        }

        if let Some(socket) = self.api_socket.take() {
            match fs::symlink_metadata(&socket) {
                Ok(metadata) if metadata.file_type().is_socket() => {
                    if fs::remove_file(&socket).is_err() {
                        first_failure.get_or_insert(FixtureFailure::CleanupFailed);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                _ => {
                    first_failure.get_or_insert(FixtureFailure::CleanupFailed);
                }
            }
        }

        self.observations.take();
        self.restart_config.take();
        self.window_watch.take();
        self.window.take();
        self.window_sha256.take();
        self.state_directory.take();
        self.fixture.take();

        if let Some(pool) = self.audit_pool.take() {
            if timeout_at(self.cleanup_deadline, pool.close())
                .await
                .is_err()
            {
                first_failure.get_or_insert(FixtureFailure::CleanupFailed);
            }
        }
        if let Some(database) = self.database.take() {
            if database.cleanup_until(self.cleanup_deadline).await.is_err() {
                first_failure.get_or_insert(FixtureFailure::CleanupFailed);
            }
        }
        if self.active_tasks.load(Ordering::Acquire) != 0 {
            first_failure.get_or_insert(FixtureFailure::TaskJoinFailed);
        }
        if self.forced_abort {
            first_failure.get_or_insert(FixtureFailure::ForcedAbort);
        }
        first_failure
    }

    fn write_result(
        &mut self,
        primary_failure: Option<FixtureFailure>,
        cleanup_failure: Option<FixtureFailure>,
    ) -> FixtureResult {
        let active_fixture_tasks = self.active_tasks.load(Ordering::Acquire);
        let task_handles_empty = self.api_task.is_none()
            && self.daemon_task.is_none()
            && self.approval_task.is_none()
            && self.websocket_task.is_none();
        // End the acquisition window only after the unchanged owned cleanup.
        // Final publication trace cloning/JSON encoding is outside this window.
        let mut resource_failure = None;
        let heap_summary = match self.heap_window.take() {
            Some(window) => match window.finish() {
                Ok(value) => Some(value),
                Err(_) => {
                    resource_failure = Some(FixtureFailure::ResourceMeasurementFailed);
                    None
                }
            },
            None => {
                resource_failure = Some(FixtureFailure::ResourceProbeMissing);
                None
            }
        };
        let process_rss_bytes = match process_rss_bytes() {
            Ok(value) => Some(value),
            Err(_) => {
                resource_failure.get_or_insert(FixtureFailure::ResourceMeasurementFailed);
                None
            }
        };
        let buffer_summary = match self.publication_probe.as_ref() {
            Some(probe) => match probe.buffer_summary() {
                Ok(value) if buffer_summary_is_valid(&value) => Some(value),
                _ => {
                    resource_failure.get_or_insert(FixtureFailure::ResourceMeasurementFailed);
                    None
                }
            },
            None => {
                resource_failure.get_or_insert(FixtureFailure::ResourceProbeMissing);
                None
            }
        };
        let mut publication_summary = None;
        let mut publication_evidence_written = false;
        let mut publication_failure = None;
        match self.publication_probe.as_ref() {
            None => publication_failure = Some(FixtureFailure::PublicationProbeMissing),
            Some(probe) => match probe.snapshot() {
                Err(_) => {
                    publication_failure = Some(FixtureFailure::PublicationMeasurementFailed)
                }
                Ok(snapshot) => {
                    publication_summary = Some(snapshot.summary.clone());
                    publication_failure = final_publication_failure(
                        true,
                        Some(&snapshot.summary),
                        task_handles_empty,
                        active_fixture_tasks,
                    );
                    match encode_publication_snapshot(snapshot) {
                        Ok(bytes) => match atomic_publication_bytes_once(
                            &self.settings.directory,
                            &bytes,
                        ) {
                            Ok(()) => publication_evidence_written = true,
                            Err(error) => {
                                publication_failure.get_or_insert(error);
                            }
                        },
                        Err(error) => {
                            publication_failure.get_or_insert(error);
                        }
                    }
                }
            },
        }
        if publication_failure.is_none() {
            publication_failure = final_publication_failure(
                self.publication_probe.is_some(),
                publication_summary.as_ref(),
                task_handles_empty,
                active_fixture_tasks,
            );
        }
        let effective_primary_failure =
            primary_failure.or(publication_failure).or(resource_failure);
        let elapsed_ms = Instant::now()
            .saturating_duration_since(self.started)
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        let result = ResultDocument {
            schema_version: 1,
            status: if effective_primary_failure.is_none() && cleanup_failure.is_none() {
                "PASS"
            } else {
                "FAIL"
            },
            primary_failure: effective_primary_failure.map(FixtureFailure::code),
            cleanup_failure: cleanup_failure.map(FixtureFailure::code),
            forced_abort: self.forced_abort,
            daemon_exit: self.daemon_exit,
            elapsed_ms,
            transport: self.metrics.snapshot()?,
            active_fixture_tasks,
            publication_summary,
            publication_evidence_written,
            buffer_summary,
            heap_summary,
            process_rss_bytes,
            resource_failure: resource_failure.map(FixtureFailure::code),
        };
        atomic_json_once(&self.settings.directory.join("result.json"), &result)
            .map_err(|_| FixtureFailure::ResultWriteFailed)?;
        if primary_failure.is_none() && cleanup_failure.is_none() {
            if let Some(error) = publication_failure.or(resource_failure) {
                return Err(error);
            }
        }
        Ok(())
    }
}

impl Drop for OwnedFixture {
    fn drop(&mut self) {
        if let Some(task) = self.api_task.as_ref() {
            task.abort();
        }
        if let Some(task) = self.daemon_task.as_ref() {
            task.abort();
        }
        if let Some(task) = self.approval_task.as_ref() {
            task.abort();
        }
        if let Some(task) = self.websocket_task.as_ref() {
            task.abort();
        }
    }
}

pub(super) async fn run_fixture() -> FixtureResult {
    let settings = ControlSettings::read()?;
    let mut owner = OwnedFixture::new(settings);
    let setup_result = std::panic::AssertUnwindSafe(owner.prepare())
        .catch_unwind()
        .await;
    let mut primary_failure = match setup_result {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(error),
        Err(_) => Some(FixtureFailure::UnexpectedPanic),
    };

    if primary_failure.is_none() {
        let work_result = std::panic::AssertUnwindSafe(owner.work_until_stop())
            .catch_unwind()
            .await;
        primary_failure = match work_result {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(error),
            Err(_) => Some(FixtureFailure::UnexpectedPanic),
        };
    }

    let cleanup_result = std::panic::AssertUnwindSafe(owner.cleanup())
        .catch_unwind()
        .await;
    let cleanup_failure = match cleanup_result {
        Ok(value) => value,
        Err(_) => Some(FixtureFailure::UnexpectedPanic),
    };
    let write_result = owner.write_result(primary_failure, cleanup_failure);
    match (primary_failure, cleanup_failure, write_result) {
        (None, None, Ok(())) => Ok(()),
        (Some(error), _, _) => Err(error),
        (_, Some(error), _) => Err(error),
        (_, _, Err(error)) => Err(error),
    }
}

async fn join_fixture_task(
    mut task: JoinHandle<FixtureResult>,
    deadline: Instant,
    task_failure: FixtureFailure,
    forced_abort: &mut bool,
) -> FixtureResult {
    match timeout_at(deadline, &mut task).await {
        Ok(Ok(Ok(()))) => Ok(()),
        Ok(Ok(Err(_))) => Err(task_failure),
        Ok(Err(error)) => Err(if error.is_panic() {
            FixtureFailure::TaskPanicked
        } else {
            FixtureFailure::TaskJoinFailed
        }),
        Err(_) => {
            task.abort();
            *forced_abort = true;
            let joined = task.await;
            match joined {
                Ok(_) | Err(_) => Err(FixtureFailure::ForcedAbort),
            }
        }
    }
}

async fn join_daemon(
    mut task: JoinHandle<Result<MarketStreamRuntimeExit, MarketStreamRuntimeError>>,
    deadline: Instant,
    forced_abort: &mut bool,
) -> FixtureResult<MarketStreamRuntimeExit> {
    match timeout_at(deadline, &mut task).await {
        Ok(Ok(Ok(exit))) => Ok(exit),
        Ok(Ok(Err(_))) => Err(FixtureFailure::DaemonFailed),
        Ok(Err(error)) => Err(if error.is_panic() {
            FixtureFailure::TaskPanicked
        } else {
            FixtureFailure::TaskJoinFailed
        }),
        Err(_) => {
            task.abort();
            *forced_abort = true;
            let joined = task.await;
            match joined {
                Ok(_) | Err(_) => Err(FixtureFailure::ForcedAbort),
            }
        }
    }
}

async fn wait_for_true(mut receiver: watch::Receiver<bool>) {
    loop {
        if *receiver.borrow_and_update() {
            return;
        }
        if receiver.changed().await.is_err() {
            return;
        }
    }
}

#[derive(Serialize)]
struct ObservationRow {
    active_leases: i64,
    desired_count: i64,
    desired_reference_sum: i64,
    cache_rows: i64,
    quote_version_sum: i64,
    producer_state: Option<String>,
}

async fn query_observation(
    database: &boundary::DisposableDatabase,
    owner_id: Uuid,
    slot_id: Uuid,
) -> FixtureResult<ObservationRow> {
    let row = sqlx::query(
        "SELECT
            (SELECT count(*)::bigint FROM public.owner_market_stream_leases
              WHERE owner_user_id = $1 AND state = 'ACTIVE'
                AND lease_expires_at > pg_catalog.clock_timestamp()) AS active_leases,
            (SELECT count(*)::bigint
               FROM public.owner_market_stream_subscriptions
              WHERE credential_slot_id = $2 AND desired_reference_count > 0) AS desired_count,
            (SELECT COALESCE(sum(desired_reference_count), 0)::bigint
               FROM public.owner_market_stream_subscriptions
              WHERE credential_slot_id = $2) AS desired_reference_sum,
            (SELECT count(*)::bigint FROM public.owner_market_stream_cache
              WHERE owner_user_id = $1) AS cache_rows,
            (SELECT COALESCE(sum(quote_version), 0)::bigint
               FROM public.owner_market_stream_cache
              WHERE owner_user_id = $1) AS quote_version_sum,
            (SELECT connection_state::text FROM public.owner_market_stream_producers
              WHERE credential_slot_id = $2 LIMIT 1) AS producer_state",
    )
    .bind(owner_id)
    .bind(slot_id)
    .fetch_one(&database.migration_owner)
    .await
    .map_err(|_| FixtureFailure::ObservationFailed)?;
    Ok(ObservationRow {
        active_leases: row
            .try_get("active_leases")
            .map_err(|_| FixtureFailure::ObservationFailed)?,
        desired_count: row
            .try_get("desired_count")
            .map_err(|_| FixtureFailure::ObservationFailed)?,
        desired_reference_sum: row
            .try_get("desired_reference_sum")
            .map_err(|_| FixtureFailure::ObservationFailed)?,
        cache_rows: row
            .try_get("cache_rows")
            .map_err(|_| FixtureFailure::ObservationFailed)?,
        quote_version_sum: row
            .try_get("quote_version_sum")
            .map_err(|_| FixtureFailure::ObservationFailed)?,
        producer_state: row
            .try_get("producer_state")
            .map_err(|_| FixtureFailure::ObservationFailed)?,
    })
}

fn make_window_contract(
    date: NaiveDate,
    retrieved_at: DateTime<Utc>,
) -> FixtureResult<(IntradaySessionWindowContract, String)> {
    let bytes = serde_json::to_vec(&json!({
        "schema_version": 1,
        "exchange": "KRX",
        "timezone": "Asia/Seoul",
        "entries": [{
            "date": date.format("%Y-%m-%d").to_string(),
            "disposition": "SPECIAL",
            "open_local": "00:00:00",
            "close_local": "23:59:59",
            "evidence_url": "https://global.krx.co.kr/synthetic/wp7i-window",
            "evidence_retrieved_at": retrieved_at.to_rfc3339_opts(SecondsFormat::Millis, true),
            "evidence_sha256": format!("sha256:{}", "7".repeat(64))
        }]
    }))
    .map_err(|_| FixtureFailure::WindowInvalid)?;
    let digest = format!("sha256:{:x}", Sha256::digest(&bytes));
    let contract = IntradaySessionWindowContract::from_bytes(&bytes, &digest)
        .map_err(|_| FixtureFailure::WindowInvalid)?;
    Ok((contract, digest))
}

fn window_bounds(date: NaiveDate) -> FixtureResult<(DateTime<Utc>, DateTime<Utc>)> {
    let kst = FixedOffset::east_opt(9 * 60 * 60).ok_or(FixtureFailure::WindowInvalid)?;
    let open = date
        .and_hms_opt(0, 0, 0)
        .ok_or(FixtureFailure::WindowInvalid)?
        .and_local_timezone(kst)
        .single()
        .ok_or(FixtureFailure::WindowInvalid)?
        .with_timezone(&Utc);
    let close = date
        .and_hms_opt(23, 59, 59)
        .ok_or(FixtureFailure::WindowInvalid)?
        .and_local_timezone(kst)
        .single()
        .ok_or(FixtureFailure::WindowInvalid)?
        .with_timezone(&Utc);
    Ok((open, close))
}

async fn seed_calendar_lineage(
    database: &boundary::DisposableDatabase,
    fixture: &boundary::Fixture,
    window_sha256: &str,
    retrieved_at: DateTime<Utc>,
) -> FixtureResult {
    let calendar = IntradaySessionProof::new(
        fixture.session_date,
        fixture.session.calendar_source_batch_id,
        fixture.session.calendar_content_sha256.clone(),
        window_sha256.to_owned(),
    )
    .map_err(|_| FixtureFailure::CalendarSeedFailed)?;
    let batch_id = calendar.calendar_source_batch_id;
    let content_sha256 = calendar.calendar_content_sha256.clone();
    sqlx::query(
        "INSERT INTO public.data_batches
            (provider, market, batch_date, kind, storage_path, content_sha256,
             bytes_size, retrieved_at, source_batch_id, source_file_name, fetch_mode)
         VALUES ('KRX', 'KR', $1, 'CALENDAR', $2, $3, 1, $4, $5,
                 'calendar.json', 'credentialed')",
    )
    .bind(calendar.session_date)
    .bind(format!("synthetic-wp7i-calendar/{batch_id}/calendar.json"))
    .bind(&content_sha256)
    .bind(retrieved_at)
    .bind(batch_id)
    .execute(&database.migration_owner)
    .await
    .map_err(|_| FixtureFailure::CalendarSeedFailed)?;
    for query in [
        "INSERT INTO public.trading_calendar_versions
            (exchange, session_date, session_type, timezone, source, source_version,
             source_batch_id, content_sha256, retrieved_at)
         VALUES ('KRX', $1, 'TRADING', 'Asia/Seoul', $2, $3, $4, $5, $6)",
        "INSERT INTO public.trading_calendars
            (exchange, session_date, session_type, timezone, source, source_version,
             source_batch_id, content_sha256, retrieved_at)
         VALUES ('KRX', $1, 'TRADING', 'Asia/Seoul', $2, $3, $4, $5, $6)",
    ] {
        sqlx::query(query)
            .bind(calendar.session_date)
            .bind(calendar.calendar_source())
            .bind(calendar.calendar_source_version())
            .bind(batch_id)
            .bind(&content_sha256)
            .bind(retrieved_at)
            .execute(&database.migration_owner)
            .await
            .map_err(|_| FixtureFailure::CalendarSeedFailed)?;
    }
    Ok(())
}

async fn approval_accept_loop(
    listener: TcpListener,
    mut stop: watch::Receiver<bool>,
    metrics: MetricsHandle,
    max_sessions: u64,
) -> FixtureResult {
    loop {
        let accepted = tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow_and_update() { return Ok(()); }
                continue;
            }
            accepted = listener.accept() => accepted.map_err(|_| FixtureFailure::ApprovalFailed)?,
        };
        let (mut stream, _) = accepted;
        metrics
            .update(|value| value.approval_accepts = value.approval_accepts.saturating_add(1))?;
        if metrics.snapshot()?.approval_accepts > max_sessions {
            metrics.update(|value| {
                value.unexpected_commands_or_requests =
                    value.unexpected_commands_or_requests.saturating_add(1)
            })?;
            return Err(FixtureFailure::ApprovalFailed);
        }
        match tokio::time::timeout(Duration::from_secs(5), handle_approval(&mut stream)).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) | Err(_) => {
                metrics
                    .update(|value| value.server_errors = value.server_errors.saturating_add(1))?;
                return Err(FixtureFailure::ApprovalFailed);
            }
        }
    }
}

async fn handle_approval(stream: &mut TcpStream) -> FixtureResult {
    let (headers, body) = read_http_request(stream)
        .await
        .map_err(|_| FixtureFailure::ApprovalFailed)?;
    let request_line = headers
        .split(|byte| *byte == b'\n')
        .next()
        .ok_or(FixtureFailure::ApprovalFailed)?;
    if request_line != b"POST /oauth2/Approval HTTP/1.1\r" {
        return Err(FixtureFailure::ApprovalFailed);
    }
    let request: Value =
        serde_json::from_slice(&body).map_err(|_| FixtureFailure::ApprovalFailed)?;
    if request.as_object().is_none_or(|object| object.len() != 3)
        || request.get("grant_type").and_then(Value::as_str) != Some("client_credentials")
        || request.get("appkey").and_then(Value::as_str) != Some(SYNTHETIC_APP_KEY)
        || request.get("secretkey").and_then(Value::as_str) != Some(SYNTHETIC_APP_SECRET)
    {
        return Err(FixtureFailure::ApprovalFailed);
    }
    let response_body = br#"{"approval_key":"WP7_SYNTHETIC_APPROVAL_KEY"}"#;
    let response_header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response_body.len()
    );
    stream
        .write_all(response_header.as_bytes())
        .await
        .map_err(|_| FixtureFailure::ApprovalFailed)?;
    stream
        .write_all(response_body)
        .await
        .map_err(|_| FixtureFailure::ApprovalFailed)
}

async fn read_http_request(stream: &mut TcpStream) -> std::io::Result<(Vec<u8>, Vec<u8>)> {
    let headers = read_http_headers(stream).await?;
    let length = header_value(&headers, b"content-length")
        .and_then(|value| std::str::from_utf8(value).ok())
        .and_then(|value| value.parse::<usize>().ok())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "content length"))?;
    if length > MAX_HTTP_BODY_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "body bound",
        ));
    }
    let mut body = vec![0_u8; length];
    stream.read_exact(&mut body).await?;
    Ok((headers, body))
}

async fn read_http_headers(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut headers = Vec::with_capacity(1024);
    let mut byte = [0_u8; 1];
    while headers.len() < MAX_HTTP_HEADER_BYTES {
        stream.read_exact(&mut byte).await?;
        headers.push(byte[0]);
        if headers.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    if !headers.ends_with(b"\r\n\r\n") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "header bound",
        ));
    }
    Ok(headers)
}

fn header_value<'a>(headers: &'a [u8], name: &[u8]) -> Option<&'a [u8]> {
    headers
        .split(|byte| *byte == b'\n')
        .skip(1)
        .find_map(|line| {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            let index = line.iter().position(|byte| *byte == b':')?;
            let (key, suffix) = line.split_at(index);
            key.eq_ignore_ascii_case(name)
                .then(|| suffix[1..].strip_prefix(b" ").unwrap_or(&suffix[1..]))
        })
}

async fn websocket_accept_loop(
    listener: TcpListener,
    mut stop: watch::Receiver<bool>,
    metrics: MetricsHandle,
    session_date: NaiveDate,
    started: Instant,
    session_deadline: Instant,
    max_sessions: u64,
) -> FixtureResult {
    loop {
        let accepted = tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow_and_update() { return Ok(()); }
                continue;
            }
            accepted = listener.accept() => accepted.map_err(|_| FixtureFailure::WebsocketFailed)?,
            _ = sleep_until(session_deadline) => return Err(FixtureFailure::WebsocketFailed),
        };
        let (stream, _) = accepted;
        metrics
            .update(|value| value.websocket_accepts = value.websocket_accepts.saturating_add(1))?;
        if metrics.snapshot()?.websocket_accepts > max_sessions {
            metrics.update(|value| {
                value.unexpected_commands_or_requests =
                    value.unexpected_commands_or_requests.saturating_add(1)
            })?;
            return Err(FixtureFailure::WebsocketFailed);
        }
        let handshake_deadline = (Instant::now() + Duration::from_secs(5)).min(session_deadline);
        metrics.update(|value| value.client_close_seen = false)?;
        match timeout_at(
            session_deadline,
            serve_websocket(
                stream,
                metrics.clone(),
                session_date,
                started,
                stop.clone(),
                handshake_deadline,
                session_deadline,
            ),
        )
        .await
        {
            Ok(Ok(())) if metrics.snapshot()?.client_close_seen => {
                if metrics.snapshot()?.websocket_accepts == max_sessions {
                    return Ok(());
                }
            }
            Ok(Ok(())) => return Err(FixtureFailure::WebsocketFailed),
            Ok(Err(error)) => {
                metrics
                    .update(|value| value.server_errors = value.server_errors.saturating_add(1))?;
                return Err(error);
            }
            Err(_) => {
                metrics
                    .update(|value| value.server_errors = value.server_errors.saturating_add(1))?;
                return Err(FixtureFailure::WebsocketFailed);
            }
        }
    }
}

async fn serve_websocket(
    mut stream: TcpStream,
    metrics: MetricsHandle,
    session_date: NaiveDate,
    started: Instant,
    mut stop: watch::Receiver<bool>,
    handshake_deadline: Instant,
    session_deadline: Instant,
) -> FixtureResult {
    {
        let handshake = async {
            let headers = read_http_headers(&mut stream)
                .await
                .map_err(|_| FixtureFailure::WebsocketFailed)?;
            let request_line = headers
                .split(|byte| *byte == b'\n')
                .next()
                .ok_or(FixtureFailure::WebsocketFailed)?;
            if request_line != b"GET /tryitout HTTP/1.1\r" {
                return Err(FixtureFailure::WebsocketFailed);
            }
            let key = header_value(&headers, b"sec-websocket-key")
                .ok_or(FixtureFailure::WebsocketFailed)?;
            let key = std::str::from_utf8(key).map_err(|_| FixtureFailure::WebsocketFailed)?;
            let mut digest = sha1_smol::Sha1::new();
            digest.update(format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes());
            let accept = base64::engine::general_purpose::STANDARD.encode(digest.digest().bytes());
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .await
                .map_err(|_| FixtureFailure::WebsocketFailed)?;
            Ok::<(), FixtureFailure>(())
        };
        tokio::pin!(handshake);
        let handshake_timeout = timeout_at(handshake_deadline, &mut handshake);
        tokio::pin!(handshake_timeout);
        loop {
            tokio::select! {
                result = &mut handshake_timeout => {
                    result.map_err(|_| FixtureFailure::WebsocketFailed)??;
                    break;
                }
                changed = stop.changed() => {
                    if changed.is_err() || *stop.borrow_and_update() {
                        return Err(FixtureFailure::WebsocketFailed);
                    }
                }
            }
        }
    }

    metrics.update(|value| {
        value.active_websockets = value.active_websockets.saturating_add(1);
        value.peak_websockets = value.peak_websockets.max(value.active_websockets);
    })?;
    let _active = ActiveWebsocket(metrics.clone());

    let (mut reader, mut writer) = tokio::io::split(stream);
    let first_tick = Instant::now() + Duration::from_millis(50);
    let mut ticker = tokio::time::interval_at(first_tick, Duration::from_millis(50));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut subscriptions = BTreeMap::<String, i64>::new();
    let mut sequence = 0_u64;
    loop {
        if Instant::now() >= session_deadline {
            return Err(FixtureFailure::WebsocketFailed);
        }
        let frame = {
            let read_frame = read_client_frame(&mut reader);
            tokio::pin!(read_frame);
            loop {
                tokio::select! {
                    frame = &mut read_frame => break frame.map_err(|_| FixtureFailure::WebsocketFailed)?,
                    _ = ticker.tick() => {
                        if subscriptions.is_empty() {
                            continue;
                        }
                        let now = Utc::now();
                        let kst = FixedOffset::east_opt(9 * 60 * 60)
                            .ok_or(FixtureFailure::WindowInvalid)?;
                        let trade_time = now.with_timezone(&kst).format("%H%M%S").to_string();
                        let mut records = Vec::with_capacity(subscriptions.len());
                        for (symbol, first_whole_second) in &subscriptions {
                            if now.timestamp() < *first_whole_second {
                                continue;
                            }
                            records.push(market_fields(session_date, &trade_time, symbol, sequence).join("^"));
                            sequence = sequence.saturating_add(1);
                        }
                        if records.is_empty() {
                            continue;
                        }
                        if records.len() > 30 {
                            return Err(FixtureFailure::WebsocketFailed);
                        }
                        let frame = format!("0|H0STCNT0|{:03}|{}", records.len(), records.join("^"));
                        if frame.len() > MAX_WS_FRAME_BYTES {
                            return Err(FixtureFailure::WebsocketFailed);
                        }
                        write_server_frame(&mut writer, 0x1, frame.as_bytes())
                            .await.map_err(|_| FixtureFailure::WebsocketFailed)?;
                        metrics.update(|value| {
                            value.quote_frames = value.quote_frames.saturating_add(1);
                            value.quote_records = value.quote_records.saturating_add(records.len() as u64);
                        })?;
                    }
                    _ = sleep_until(session_deadline) => return Err(FixtureFailure::WebsocketFailed),
                    changed = stop.changed() => {
                        if changed.is_err() || *stop.borrow_and_update() {
                            return Err(FixtureFailure::WebsocketFailed);
                        }
                    }
                }
            }
        };
        match frame.opcode {
            0x1 => {
                handle_command_frame(
                    &mut writer,
                    &mut subscriptions,
                    frame.payload,
                    &metrics,
                    started,
                )
                .await?
            }
            0x8 => {
                write_server_frame(&mut writer, 0x8, &frame.payload)
                    .await
                    .map_err(|_| FixtureFailure::WebsocketFailed)?;
                metrics.update(|value| value.client_close_seen = true)?;
                return Ok(());
            }
            0x9 => write_server_frame(&mut writer, 0xa, &frame.payload)
                .await
                .map_err(|_| FixtureFailure::WebsocketFailed)?,
            0xa => {}
            _ => return Err(FixtureFailure::WebsocketFailed),
        }
    }
}

struct ActiveWebsocket(MetricsHandle);

impl Drop for ActiveWebsocket {
    fn drop(&mut self) {
        if let Ok(mut metrics) = self.0.0.lock() {
            metrics.active_websockets = metrics.active_websockets.saturating_sub(1);
        }
    }
}

async fn handle_command_frame<W>(
    writer: &mut W,
    subscriptions: &mut BTreeMap<String, i64>,
    payload: Vec<u8>,
    metrics: &MetricsHandle,
    started: Instant,
) -> FixtureResult
where
    W: AsyncWrite + Unpin,
{
    if payload.len() > MAX_WS_FRAME_BYTES {
        return Err(FixtureFailure::WebsocketFailed);
    }
    let command: Value =
        serde_json::from_slice(&payload).map_err(|_| FixtureFailure::WebsocketFailed)?;
    let symbol = command
        .pointer("/body/input/tr_key")
        .and_then(Value::as_str)
        .ok_or(FixtureFailure::WebsocketFailed)?;
    let operation = command
        .pointer("/header/tr_type")
        .and_then(Value::as_str)
        .ok_or(FixtureFailure::WebsocketFailed)?;
    let approval_key = command
        .pointer("/header/approval_key")
        .and_then(Value::as_str);
    if command.pointer("/body/input/tr_id").and_then(Value::as_str) != Some("H0STCNT0")
        || !boundary::MARKET_SYMBOLS.contains(&symbol)
        || approval_key != Some(SYNTHETIC_APPROVAL_KEY)
    {
        metrics.update(|value| {
            value.unexpected_commands_or_requests =
                value.unexpected_commands_or_requests.saturating_add(1)
        })?;
        return Err(FixtureFailure::WebsocketFailed);
    }
    let (name, acknowledgement) = match operation {
        "1" if !subscriptions.contains_key(symbol) => ("subscribe", "SUBSCRIBE SUCCESS"),
        "2" if subscriptions.contains_key(symbol) => ("unsubscribe", "UNSUBSCRIBE SUCCESS"),
        _ => {
            metrics.update(|value| {
                value.unexpected_commands_or_requests =
                    value.unexpected_commands_or_requests.saturating_add(1)
            })?;
            return Err(FixtureFailure::WebsocketFailed);
        }
    };
    let ack = serde_json::to_vec(&json!({
        "header": {"tr_id": "H0STCNT0", "tr_key": symbol, "encrypt": "N"},
        "body": {"rt_cd": "0", "msg_cd": "OPSP0000", "msg1": acknowledgement}
    }))
    .map_err(|_| FixtureFailure::WebsocketFailed)?;
    write_server_frame(writer, 0x1, &ack)
        .await
        .map_err(|_| FixtureFailure::WebsocketFailed)?;
    let symbol = symbol.to_owned();
    if operation == "1" {
        // HHMMSS is second-granular: emit a real later second instead of a
        // rounded timestamp preceding this connection's fresh-epoch boundary.
        let first_whole_second = Utc::now().timestamp().saturating_add(1);
        subscriptions.insert(symbol.clone(), first_whole_second);
    } else {
        subscriptions.remove(&symbol);
    }
    let elapsed_ms = Instant::now()
        .saturating_duration_since(started)
        .as_millis()
        .min(u128::from(u64::MAX)) as u64;
    metrics.update(|value| {
        if operation == "1" {
            value.subscribe_commands = value.subscribe_commands.saturating_add(1);
        } else {
            value.unsubscribe_commands = value.unsubscribe_commands.saturating_add(1);
        }
        if value.command_events.len() >= MAX_COMMAND_EVENTS {
            value.event_overflow = true;
        } else {
            value.command_events.push(CommandEvent {
                elapsed_ms,
                symbol,
                operation: name,
            });
        }
    })?;
    if metrics.snapshot()?.event_overflow {
        return Err(FixtureFailure::ObservationLimit);
    }
    Ok(())
}

fn market_fields(date: NaiveDate, trade_time: &str, symbol: &str, sequence: u64) -> Vec<String> {
    let mut fields = vec![String::new(); 47];
    fields[0] = symbol.to_owned();
    fields[1] = trade_time.to_owned();
    fields[2] = (70_000_u64.saturating_add(sequence % 10_000)).to_string();
    fields[3] = "2".to_owned();
    fields[4] = "100".to_owned();
    fields[5] = "0.14".to_owned();
    fields[12] = "10".to_owned();
    fields[13] = (20_u64.saturating_add(sequence)).to_string();
    fields[33] = date.format("%Y%m%d").to_string();
    fields[34] = "20".to_owned();
    fields[35] = "N".to_owned();
    fields[43] = "0".to_owned();
    fields[46] = "2".to_owned();
    fields
}

struct ClientFrame {
    opcode: u8,
    payload: Vec<u8>,
}

async fn read_client_frame<R>(reader: &mut R) -> std::io::Result<ClientFrame>
where
    R: AsyncRead + Unpin,
{
    let mut header = [0_u8; 2];
    reader.read_exact(&mut header).await?;
    let fin = header[0] & 0x80 != 0;
    let reserved = header[0] & 0x70;
    let opcode = header[0] & 0x0f;
    let masked = header[1] & 0x80 != 0;
    let marker = header[1] & 0x7f;
    if !fin || reserved != 0 || !masked || !matches!(opcode, 0x1 | 0x8 | 0x9 | 0xa) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame shape",
        ));
    }
    let length = match marker {
        0..=125 => usize::from(marker),
        126 => {
            let mut bytes = [0_u8; 2];
            reader.read_exact(&mut bytes).await?;
            usize::from(u16::from_be_bytes(bytes))
        }
        127 => {
            let mut bytes = [0_u8; 8];
            reader.read_exact(&mut bytes).await?;
            usize::try_from(u64::from_be_bytes(bytes))
                .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "frame length"))?
        }
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "frame marker",
            ));
        }
    };
    if length > MAX_WS_FRAME_BYTES || (opcode & 0x8 != 0 && length > 125) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame bound",
        ));
    }
    let mut mask = [0_u8; 4];
    reader.read_exact(&mut mask).await?;
    let mut payload = vec![0_u8; length];
    reader.read_exact(&mut payload).await?;
    for (index, byte) in payload.iter_mut().enumerate() {
        *byte ^= mask[index % mask.len()];
    }
    Ok(ClientFrame { opcode, payload })
}

async fn write_server_frame<W>(writer: &mut W, opcode: u8, payload: &[u8]) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    if payload.len() > MAX_WS_FRAME_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "frame bound",
        ));
    }
    let mut frame = Vec::with_capacity(payload.len() + 10);
    frame.push(0x80 | opcode);
    match payload.len() {
        0..=125 => frame.push(payload.len() as u8),
        126..=65_535 => {
            frame.push(126);
            frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        }
        _ => {
            frame.push(127);
            frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
        }
    }
    frame.extend_from_slice(payload);
    writer.write_all(&frame).await
}

fn atomic_json_once(path: &Path, value: &impl Serialize) -> FixtureResult {
    let parent = path.parent().ok_or(FixtureFailure::ControlInvalid)?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|_| FixtureFailure::ControlInvalid)?;
    temporary
        .as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|_| FixtureFailure::ControlInvalid)?;
    serde_json::to_writer(temporary.as_file_mut(), value)
        .map_err(|_| FixtureFailure::ControlInvalid)?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|_| FixtureFailure::ControlInvalid)?;
    temporary
        .persist_noclobber(path)
        .map_err(|_| FixtureFailure::ControlInvalid)?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| FixtureFailure::ControlInvalid)?;
    Ok(())
}


// Reuse the checked-in parser source only inside this integration test target.
// No production API or transport receipt constructor is widened for this check.
#[cfg(test)]
#[path = "../../../kis-client/src/market_stream_wire.rs"]
mod reviewed_wire_contract;

#[cfg(test)]
mod synthetic_wire_tests {
    use super::market_fields;
    use super::reviewed_wire_contract::{ParseContext, ParsedMarketRecord, parse_market_message};
    use chrono::NaiveDate;

    #[test]
    fn thirty_symbol_repeated_frames_are_regular_and_advance_volume() {
        let date = NaiveDate::from_ymd_opt(2026, 10, 4).unwrap();
        let mut prior_volumes = [0_u64; 30];
        for frame in 0..3_u64 {
            let mut records = Vec::new();
            for index in 0..30_u64 {
                let symbol = format!("{:06}", 100_000 + index);
                records.extend(market_fields(date, "090001", &symbol, frame * 30 + index));
            }
            let message = format!("0|H0STCNT0|030|{}", records.join("^"));
            let parsed = parse_market_message(message.as_bytes(), ParseContext::default())
                .expect("positive synthetic frame must satisfy the wire contract");
            assert_eq!(parsed.record_count, 30);
            assert_eq!(parsed.records.len(), 30);
            for (index, record) in parsed.records.iter().enumerate() {
                let ParsedMarketRecord::Regular(observation) = record else {
                    panic!("positive synthetic quote was classified non-regular");
                };
                assert_eq!(observation.symbol, format!("{:06}", 100_000 + index));
                assert!(observation.is_regular());
                assert_eq!(observation.trade_volume, 10);
                assert!(observation.cumulative_volume > prior_volumes[index]);
                prior_volumes[index] = observation.cumulative_volume;
            }
        }
    }

    #[test]
    fn nonregular_opening_class_remains_excluded() {
        let date = NaiveDate::from_ymd_opt(2026, 10, 4).unwrap();
        let mut fields = market_fields(date, "090001", "005930", 1);
        fields[34] = "21".to_owned();
        let message = format!("0|H0STCNT0|001|{}", fields.join("^"));
        let parsed = parse_market_message(message.as_bytes(), ParseContext::default())
            .expect("nonregular record still has a valid wire shape");
        assert_eq!(parsed.record_count, 1);
        assert!(matches!(
            parsed.records.as_slice(),
            [ParsedMarketRecord::NonRegular(_)]
        ));
    }
}

#[derive(Serialize)]
struct PublicationEvidenceDocument {
    schema_version: u32,
    summary: StreamPublicationProbeSummary,
    records: Vec<StreamPublicationProbeRecord>,
}

fn publication_observation_is_valid(summary: &StreamPublicationProbeSummary) -> bool {
    !summary.overflowed && !summary.invalid_record && !summary.diagnostic_error
}

fn final_publication_failure(
    probe_present: bool,
    summary: Option<&StreamPublicationProbeSummary>,
    task_handles_empty: bool,
    active_fixture_tasks: usize,
) -> Option<FixtureFailure> {
    if !probe_present {
        return Some(FixtureFailure::PublicationProbeMissing);
    }
    if !task_handles_empty || active_fixture_tasks != 0 {
        return Some(FixtureFailure::TaskJoinFailed);
    }
    let Some(summary) = summary else {
        return Some(FixtureFailure::PublicationMeasurementFailed);
    };
    if !publication_observation_is_valid(summary) || summary.in_flight != 0 {
        return Some(FixtureFailure::PublicationMeasurementFailed);
    }
    None
}

fn encode_publication_snapshot(snapshot: StreamPublicationProbeSnapshot) -> FixtureResult<Vec<u8>> {
    if snapshot.records.len() > MAX_PUBLICATION_RECORDS {
        return Err(FixtureFailure::PublicationEvidenceLimitExceeded);
    }
    let document = PublicationEvidenceDocument {
        schema_version: 1,
        summary: snapshot.summary,
        records: snapshot.records,
    };
    let bytes =
        serde_json::to_vec(&document).map_err(|_| FixtureFailure::PublicationEvidenceFailed)?;
    if bytes.len() > MAX_PUBLICATION_BYTES {
        return Err(FixtureFailure::PublicationEvidenceLimitExceeded);
    }
    Ok(bytes)
}

fn atomic_publication_bytes_once(directory: &Path, bytes: &[u8]) -> FixtureResult {
    if bytes.len() > MAX_PUBLICATION_BYTES {
        return Err(FixtureFailure::PublicationEvidenceLimitExceeded);
    }
    let metadata =
        fs::symlink_metadata(directory).map_err(|_| FixtureFailure::PublicationEvidenceFailed)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o7777 != 0o700
    {
        return Err(FixtureFailure::PublicationEvidenceFailed);
    }
    let path = directory.join("publication.json");
    let mut temporary = tempfile::NamedTempFile::new_in(directory)
        .map_err(|_| FixtureFailure::PublicationEvidenceFailed)?;
    temporary
        .as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|_| FixtureFailure::PublicationEvidenceFailed)?;
    temporary
        .as_file_mut()
        .write_all(bytes)
        .map_err(|_| FixtureFailure::PublicationEvidenceFailed)?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|_| FixtureFailure::PublicationEvidenceFailed)?;
    temporary
        .persist_noclobber(&path)
        .map_err(|_| FixtureFailure::PublicationEvidenceFailed)?;
    File::open(directory)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| FixtureFailure::PublicationEvidenceFailed)?;
    Ok(())
}

#[cfg(test)]
mod measurement_tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn measurement_transport_summary_omits_event_history() {
        let metrics = MetricsHandle::new();
        metrics
            .update(|value| {
                *value = Metrics {
                    approval_accepts: 11,
                    websocket_accepts: 12,
                    active_websockets: 13,
                    peak_websockets: 14,
                    subscribe_commands: 15,
                    unsubscribe_commands: 16,
                    command_events: (0..256)
                        .map(|index| CommandEvent {
                            elapsed_ms: index,
                            symbol: "SYNTHETIC_EVENT_MARKER".to_owned(),
                            operation: "subscribe",
                        })
                        .collect(),
                    quote_frames: 17,
                    quote_records: 18,
                    unexpected_commands_or_requests: 19,
                    server_errors: 20,
                    event_overflow: true,
                    client_close_seen: false,
                };
            })
            .unwrap();

        let aggregate = metrics.aggregate().unwrap();
        assert_eq!(aggregate.approval_accepts, 11);
        assert_eq!(aggregate.websocket_accepts, 12);
        assert_eq!(aggregate.active_websockets, 13);
        assert_eq!(aggregate.peak_websockets, 14);
        assert_eq!(aggregate.subscribe_commands, 15);
        assert_eq!(aggregate.unsubscribe_commands, 16);
        assert_eq!(aggregate.quote_frames, 17);
        assert_eq!(aggregate.quote_records, 18);
        assert_eq!(aggregate.unexpected_commands_or_requests, 19);
        assert_eq!(aggregate.server_errors, 20);
        assert!(aggregate.event_overflow);
        assert!(!aggregate.client_close_seen);

        let first = serde_json::to_vec(&aggregate).unwrap();
        assert!(first.len() < 2_048);
        assert!(
            !first
                .windows(b"command_events".len())
                .any(|part| part == b"command_events")
        );
        assert!(
            !first
                .windows(b"SYNTHETIC_EVENT_MARKER".len())
                .any(|part| part == b"SYNTHETIC_EVENT_MARKER")
        );
        for _ in 0..4 {
            let repeated = serde_json::to_vec(&metrics.aggregate().unwrap()).unwrap();
            assert_eq!(repeated, first);
            assert!(repeated.len() < 2_048);
        }
        assert_eq!(metrics.snapshot().unwrap().command_events.len(), 256);
    }

    #[test]
    fn measurement_final_acceptance_rejects_missing_or_unfinished_probe() {
        let healthy = StreamPublicationProbeSummary::default();
        assert_eq!(
            final_publication_failure(false, Some(&healthy), true, 0),
            Some(FixtureFailure::PublicationProbeMissing)
        );
        assert_eq!(
            final_publication_failure(true, None, true, 0),
            Some(FixtureFailure::PublicationMeasurementFailed)
        );

        for flag in 0..3 {
            let mut summary = healthy.clone();
            match flag {
                0 => summary.overflowed = true,
                1 => summary.invalid_record = true,
                _ => summary.diagnostic_error = true,
            }
            assert_eq!(
                final_publication_failure(true, Some(&summary), true, 0),
                Some(FixtureFailure::PublicationMeasurementFailed)
            );
        }

        let mut in_flight = healthy.clone();
        in_flight.in_flight = 1;
        assert_eq!(
            final_publication_failure(true, Some(&in_flight), true, 0),
            Some(FixtureFailure::PublicationMeasurementFailed)
        );
        assert!(publication_observation_is_valid(&in_flight));

        assert_eq!(
            final_publication_failure(true, Some(&healthy), false, 0),
            Some(FixtureFailure::TaskJoinFailed)
        );
        assert_eq!(
            final_publication_failure(true, Some(&healthy), true, 1),
            Some(FixtureFailure::TaskJoinFailed)
        );
        assert_eq!(
            final_publication_failure(true, Some(&healthy), true, 0),
            None
        );
    }

    #[test]
    fn measurement_publication_document_is_bounded_metadata() {
        let summary = StreamPublicationProbeSummary {
            record_count: 1,
            committed: 1,
            attempted_changed_rows: 1,
            known_committed_changed_rows: 1,
            ..StreamPublicationProbeSummary::default()
        };
        let record = StreamPublicationProbeRecord {
            sequence: 3,
            started_elapsed_ns: 10,
            finished_elapsed_ns: Some(20),
            planned_rows: 1,
            changed_rows: 1,
            outcome: StreamPublicationProbeOutcome::Committed,
        };
        let bytes = encode_publication_snapshot(StreamPublicationProbeSnapshot {
            summary: summary.clone(),
            records: vec![record],
        })
        .unwrap();
        let document: Value = serde_json::from_slice(&bytes).unwrap();
        let object = document.as_object().unwrap();
        let mut top_level_keys: Vec<_> = object.keys().map(String::as_str).collect();
        top_level_keys.sort_unstable();
        assert_eq!(top_level_keys, ["records", "schema_version", "summary"]);
        assert_eq!(document["schema_version"], 1);
        assert_eq!(document["summary"], serde_json::to_value(&summary).unwrap());
        assert_eq!(
            document["records"][0],
            json!({
                "sequence": 3,
                "started_elapsed_ns": 10,
                "finished_elapsed_ns": 20,
                "planned_rows": 1,
                "changed_rows": 1,
                "outcome": "committed"
            })
        );
        let mut record_keys: Vec<_> = document["records"][0]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        record_keys.sort_unstable();
        assert_eq!(
            record_keys,
            [
                "changed_rows",
                "finished_elapsed_ns",
                "outcome",
                "planned_rows",
                "sequence",
                "started_elapsed_ns"
            ]
        );

        let records = (0..MAX_PUBLICATION_RECORDS)
            .map(|sequence| StreamPublicationProbeRecord {
                sequence: sequence as u64,
                started_elapsed_ns: sequence as u64,
                finished_elapsed_ns: Some(sequence as u64 + 1),
                planned_rows: 1,
                changed_rows: 1,
                outcome: StreamPublicationProbeOutcome::Committed,
            })
            .collect();
        let summary = StreamPublicationProbeSummary {
            record_count: MAX_PUBLICATION_RECORDS as u64,
            committed: MAX_PUBLICATION_RECORDS as u64,
            attempted_changed_rows: MAX_PUBLICATION_RECORDS as u64,
            known_committed_changed_rows: MAX_PUBLICATION_RECORDS as u64,
            ..StreamPublicationProbeSummary::default()
        };
        let bounded = encode_publication_snapshot(StreamPublicationProbeSnapshot {
            summary: summary.clone(),
            records,
        })
        .unwrap();
        assert!(bounded.len() < MAX_PUBLICATION_BYTES);

        let oversized = StreamPublicationProbeSnapshot {
            summary: StreamPublicationProbeSummary {
                record_count: (MAX_PUBLICATION_RECORDS + 1) as u64,
                ..summary
            },
            records: (0..=MAX_PUBLICATION_RECORDS)
                .map(|sequence| StreamPublicationProbeRecord {
                    sequence: sequence as u64,
                    started_elapsed_ns: sequence as u64,
                    finished_elapsed_ns: Some(sequence as u64 + 1),
                    planned_rows: 1,
                    changed_rows: 1,
                    outcome: StreamPublicationProbeOutcome::Committed,
                })
                .collect(),
        };
        assert!(matches!(
            encode_publication_snapshot(oversized),
            Err(FixtureFailure::PublicationEvidenceLimitExceeded)
        ));
    }
}
