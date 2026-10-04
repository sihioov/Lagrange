use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use chrono::{DateTime, FixedOffset, NaiveDate, SecondsFormat, Utc};
use kis_client::MarketStreamClient;
use kis_client::clock::{Clock, SystemClock};
use kis_client::market_stream::{MarketStreamCommandStateSnapshot, MarketStreamConfig};
use kis_client::market_stream_approval::ApprovalClient;
use kis_client::market_stream_state::MarketStreamDomain;
use kis_client::market_stream_wire::{FIELD_COUNT, MarketStreamSessionProof};
use kis_client::secret::Secret;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tempfile::TempDir;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant as TokioInstant;
use uuid::Uuid;

use crate::owner_equity_v2::intraday::{IntradaySessionProof, OwnerIntradayQuoteRepository};
use crate::owner_equity_v2::market_stream::{
    OwnerMarketStreamRepository, StreamLease, StreamLeaseRequest,
};
use crate::owner_equity_v2::market_stream_runtime::{
    MarketStreamRuntimeError, OwnerMarketStreamRuntime, OwnerMarketStreamRuntimeConfig,
    SnapshotWindowEvidence, snapshot_window_fixture,
};

#[path = "../../tests/owner_market_stream_boundary_support/mod.rs"]
pub(super) mod boundary;

const FIXTURE_JOIN_BOUND: Duration = Duration::from_secs(8);
const SYNTHETIC_APP_KEY: &str = "D4S_SYNTHETIC_APP_KEY";
const SYNTHETIC_APP_SECRET: &str = "D4S_SYNTHETIC_APP_SECRET";
const SYNTHETIC_APPROVAL_KEY: &str = "D4S_SYNTHETIC_APPROVAL_KEY";
const MAX_HTTP_HEADER_BYTES: usize = 16 * 1024;
const MAX_HTTP_BODY_BYTES: usize = 8 * 1024;
const MAX_FRAME_BYTES: usize = 64 * 1024;
const MAX_RECORDED_COMMANDS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TestFailure(pub(super) &'static str);

impl fmt::Display for TestFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for TestFailure {}

pub(super) type TestResult<T> = Result<T, TestFailure>;
pub(super) type SetupResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub(super) fn combine_setup_cleanup_failure(
    primary: TestFailure,
    cleanup: Box<dyn std::error::Error + Send + Sync>,
) -> Box<dyn std::error::Error + Send + Sync> {
    boundary::combine_primary_cleanup(Box::new(primary), cleanup)
}

pub(super) fn combine_cleanup_failures(
    first: Box<dyn std::error::Error + Send + Sync>,
    second: Box<dyn std::error::Error + Send + Sync>,
) -> Box<dyn std::error::Error + Send + Sync> {
    boundary::combine_primary_cleanup(first, second)
}

pub(super) fn cleanup_test_failure(error: &(dyn std::error::Error + Send + Sync)) -> TestFailure {
    let safe_message = error.to_string();
    let pool_close = safe_message.contains("owned C2 fixture pool close");
    let database_cleanup = safe_message.contains("generated C2 database cleanup");
    match (pool_close, database_cleanup) {
        (true, true) => {
            TestFailure("owned fixture pool close and generated database drop both failed")
        }
        (true, false) => TestFailure("owned C2 fixture pool close failed"),
        (false, true) => TestFailure("generated C2 database cleanup failed"),
        (false, false) => TestFailure("bounded fixture cleanup failed with an unknown safe error"),
    }
}

#[derive(Clone, Copy)]
pub(super) struct CaseDeadlines {
    pub(super) setup: TokioInstant,
    pub(super) work: TokioInstant,
    pub(super) cleanup: TokioInstant,
}

impl CaseDeadlines {
    pub(super) fn from_test_entry() -> Self {
        let started_at = TokioInstant::now();
        Self {
            setup: started_at + Duration::from_secs(35),
            work: started_at + Duration::from_secs(120),
            cleanup: started_at + Duration::from_secs(150),
        }
    }
}

fn failure<T>(message: &'static str) -> TestResult<T> {
    Err(TestFailure(message))
}

async fn observe_sql<T>(
    observation: impl std::future::Future<Output = Result<T, sqlx::Error>>,
    failure_message: &'static str,
) -> TestResult<T> {
    tokio::time::timeout(Duration::from_secs(2), observation)
        .await
        .map_err(|_| TestFailure("database observation exceeded two seconds"))?
        .map_err(|_| TestFailure(failure_message))
}

#[derive(Clone)]
struct MetricsHandle(Arc<Mutex<Metrics>>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CommandKind {
    Subscribe,
    Unsubscribe,
}

#[derive(Clone, Debug)]
pub(super) struct CommandEvent {
    pub(super) symbol: String,
    pub(super) kind: CommandKind,
    pub(super) arrived_at: Instant,
}

#[derive(Clone, Debug)]
pub(super) struct MetricsSnapshot {
    pub(super) approval_accepts: usize,
    pub(super) websocket_accepts: usize,
    pub(super) current_websockets: usize,
    pub(super) maximum_websockets: usize,
    pub(super) subscribe_commands: usize,
    pub(super) unsubscribe_commands: usize,
    pub(super) command_events: Vec<CommandEvent>,
    pub(super) acknowledgement_frames: usize,
    pub(super) quote_frames: usize,
    pub(super) quote_records: usize,
    pub(super) client_close_seen: bool,
    pub(super) unexpected_requests_or_commands: usize,
    pub(super) server_errors: usize,
    pub(super) event_overflow: bool,
    pub(super) active_fixture_tasks: usize,
}

#[derive(Default)]
struct Metrics {
    approval_accepts: usize,
    websocket_accepts: usize,
    current_websockets: usize,
    maximum_websockets: usize,
    subscribe_commands: usize,
    unsubscribe_commands: usize,
    command_events: Vec<CommandEvent>,
    acknowledgement_frames: usize,
    quote_frames: usize,
    quote_records: usize,
    client_close_seen: bool,
    unexpected_requests_or_commands: usize,
    server_errors: usize,
    event_overflow: bool,
}

impl MetricsHandle {
    fn update(&self, update: impl FnOnce(&mut Metrics)) -> TestResult<()> {
        let mut metrics = self
            .0
            .lock()
            .map_err(|_| TestFailure("fixture metrics poisoned"))?;
        update(&mut metrics);
        Ok(())
    }

    fn snapshot(&self, active: &AtomicUsize) -> TestResult<MetricsSnapshot> {
        let metrics = self
            .0
            .lock()
            .map_err(|_| TestFailure("fixture metrics poisoned"))?;
        Ok(MetricsSnapshot {
            approval_accepts: metrics.approval_accepts,
            websocket_accepts: metrics.websocket_accepts,
            current_websockets: metrics.current_websockets,
            maximum_websockets: metrics.maximum_websockets,
            subscribe_commands: metrics.subscribe_commands,
            unsubscribe_commands: metrics.unsubscribe_commands,
            command_events: metrics.command_events.clone(),
            acknowledgement_frames: metrics.acknowledgement_frames,
            quote_frames: metrics.quote_frames,
            quote_records: metrics.quote_records,
            client_close_seen: metrics.client_close_seen,
            unexpected_requests_or_commands: metrics.unexpected_requests_or_commands,
            server_errors: metrics.server_errors,
            event_overflow: metrics.event_overflow,
            active_fixture_tasks: active.load(Ordering::Acquire),
        })
    }
}

pub(super) struct ActiveTask(Arc<AtomicUsize>);

impl ActiveTask {
    pub(super) fn new(active: Arc<AtomicUsize>) -> Self {
        active.fetch_add(1, Ordering::AcqRel);
        Self(active)
    }
}

impl Drop for ActiveTask {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(super) struct TestDatabase {
    pub(super) database: boundary::DisposableDatabase,
    pub(super) fixture: boundary::Fixture,
    pub(super) window_contract: collectors::intraday_quotes::IntradaySessionWindowContract,
    pub(super) window_sha256: String,
    pub(super) snapshot_window: SnapshotWindowEvidence,
    holder_id: Uuid,
}

impl TestDatabase {
    pub(super) async fn create(deadlines: CaseDeadlines) -> SetupResult<Self> {
        let database =
            boundary::DisposableDatabase::create_until(deadlines.setup, deadlines.cleanup).await?;
        let prepared = tokio::time::timeout_at(deadlines.setup, async {
            let fixture = boundary::seed_fixture(&database)
                .await
                .map_err(|_| TestFailure("synthetic owner fixture setup failed"))?;
            let now_deadline = (TokioInstant::now() + Duration::from_secs(2)).min(deadlines.setup);
            let now = tokio::time::timeout_at(now_deadline, boundary::now(&database))
                .await
                .map_err(|_| TestFailure("database clock observation exceeded its bound"))?
                .map_err(|_| TestFailure("database clock observation failed"))?;
            if now
                .with_timezone(
                    &FixedOffset::east_opt(9 * 60 * 60)
                        .ok_or(TestFailure("KST offset unavailable"))?,
                )
                .date_naive()
                != fixture.session_date
            {
                return failure("fixture date differs from the current database KST date");
            }
            let (contract, window_sha256) = make_window_contract(fixture.session_date, now)?;
            let (open_at, close_at) = window_bounds(fixture.session_date)?;
            if close_at.signed_duration_since(now).num_seconds() < 120 {
                return failure("current synthetic window has less than 120 seconds remaining");
            }
            seed_calendar_lineage(&database, &fixture, &window_sha256, now).await?;
            let snapshot_window = snapshot_window_fixture(
                fixture.session_date,
                open_at,
                close_at,
                window_sha256.clone(),
            );
            Ok((
                fixture,
                contract,
                window_sha256,
                snapshot_window,
                Uuid::new_v4(),
            ))
        })
        .await;
        match prepared {
            Ok(Ok((fixture, window_contract, window_sha256, snapshot_window, holder_id)))
                if TokioInstant::now() < deadlines.setup =>
            {
                Ok(Self {
                    database,
                    fixture,
                    window_contract,
                    window_sha256,
                    snapshot_window,
                    holder_id,
                })
            }
            Ok(Err(primary)) => match database.cleanup_until(deadlines.cleanup).await {
                Ok(()) => Err(Box::new(primary)),
                Err(cleanup) => Err(combine_setup_cleanup_failure(primary, cleanup)),
            },
            Err(_) | Ok(Ok(_)) => {
                let primary = TestFailure("fixture setup exceeded its absolute deadline");
                match database.cleanup_until(deadlines.cleanup).await {
                    Ok(()) => Err(Box::new(primary)),
                    Err(cleanup) => Err(combine_setup_cleanup_failure(primary, cleanup)),
                }
            }
        }
    }

    pub(super) async fn cleanup_until(
        self,
        deadline: TokioInstant,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.database.cleanup_until(deadline).await
    }

    pub(super) fn runtime(
        &self,
        transport: &LoopbackTransport,
    ) -> TestResult<OwnerMarketStreamRuntime> {
        let config = OwnerMarketStreamRuntimeConfig::from_values(
            self.fixture.credential_slot_id,
            self.fixture.grant_id,
            boundary::NETWORK_HASH,
            7,
            self.holder_id,
        )
        .map_err(|_| TestFailure("runtime configuration construction failed"))?;
        if !transport.matches_runtime_binding(self.fixture.credential_slot_id, 7) {
            return failure("loopback approval metadata does not match runtime binding");
        }
        Ok(OwnerMarketStreamRuntime {
            client: transport.client.clone(),
            repository: OwnerMarketStreamRepository::new(self.database.worker.clone())
                .runtime_repository(),
            calendar: OwnerIntradayQuoteRepository::new(self.database.worker.clone()),
            windows_source: collectors::intraday_quotes::IntradaySessionWindowSource::ReleaseV1,
            config,
        })
    }

    pub(super) async fn create_lease(
        &self,
        consumer_id: Uuid,
        identities: Vec<crate::owner_equity_v2::market_stream::StreamLeaseIdentity>,
    ) -> TestResult<StreamLease> {
        let request = StreamLeaseRequest::new(
            consumer_id,
            0,
            identities,
            format!("d4s-{}-0", consumer_id.simple()),
        )
        .map_err(|_| TestFailure("synthetic browser lease request was rejected"))?;
        OwnerMarketStreamRepository::new(self.database.app.clone())
            .replace_stream_lease(
                self.fixture.owner_user_id,
                &self.fixture.owner_session_hash,
                &request,
            )
            .await
            .map_err(|_| TestFailure("synthetic browser lease creation failed"))
    }

    pub(super) async fn snapshot(
        &self,
        lease_id: Uuid,
        window: Option<SnapshotWindowEvidence>,
    ) -> TestResult<crate::owner_equity_v2::market_stream::StreamSnapshot> {
        tokio::time::timeout(
            Duration::from_secs(2),
            OwnerMarketStreamRepository::new(self.database.app.clone())
                .read_stream_snapshot_with_window_fixture(
                    self.fixture.owner_user_id,
                    &self.fixture.owner_session_hash,
                    lease_id,
                    window,
                ),
        )
        .await
        .map_err(|_| TestFailure("authorized snapshot exceeded its observation bound"))?
        .map_err(|_| TestFailure("authorized snapshot read failed"))
    }

    pub(super) async fn reference_counts(&self) -> TestResult<Vec<(String, i32)>> {
        observe_sql(
            sqlx::query_as(
                "SELECT symbol, desired_reference_count
               FROM public.owner_market_stream_subscriptions
              WHERE credential_slot_id = $1
              ORDER BY symbol",
            )
            .bind(self.fixture.credential_slot_id)
            .fetch_all(&self.database.migration_owner),
            "subscription count observation failed",
        )
        .await
    }

    pub(super) async fn cache_versions(&self) -> TestResult<Vec<(Uuid, i64)>> {
        observe_sql(
            sqlx::query_as(
                "SELECT membership_id, quote_version
               FROM public.owner_market_stream_cache
              WHERE owner_user_id = $1
              ORDER BY membership_id",
            )
            .bind(self.fixture.owner_user_id)
            .fetch_all(&self.database.migration_owner),
            "cache version observation failed",
        )
        .await
    }

    pub(super) async fn producer_state(&self) -> TestResult<Option<(Option<Uuid>, String, i64)>> {
        observe_sql(
            sqlx::query_as(
                "SELECT current_epoch, connection_state, fencing_token
               FROM public.owner_market_stream_producers
              WHERE credential_slot_id = $1",
            )
            .bind(self.fixture.credential_slot_id)
            .fetch_optional(&self.database.migration_owner),
            "producer state observation failed",
        )
        .await
    }

    pub(super) async fn quote_version_sum(&self) -> TestResult<i64> {
        observe_sql(
            sqlx::query_scalar(
                "SELECT COALESCE(sum(quote_version), 0)::bigint
               FROM public.owner_market_stream_cache
              WHERE owner_user_id = $1",
            )
            .bind(self.fixture.owner_user_id)
            .fetch_one(&self.database.migration_owner),
            "cache quote version observation failed",
        )
        .await
    }

    pub(super) fn command_state(
        &self,
        transport: &LoopbackTransport,
    ) -> TestResult<MarketStreamCommandStateSnapshot> {
        transport
            .domain
            .test_command_state_snapshot()
            .map_err(|_| TestFailure("durable command pacing observation failed"))
    }
}

fn make_window_contract(
    date: NaiveDate,
    now: DateTime<Utc>,
) -> TestResult<(
    collectors::intraday_quotes::IntradaySessionWindowContract,
    String,
)> {
    let bytes = serde_json::to_vec(&serde_json::json!({
        "schema_version": 1,
        "exchange": "KRX",
        "timezone": "Asia/Seoul",
        "entries": [{
            "date": date.format("%Y-%m-%d").to_string(),
            "disposition": "SPECIAL",
            "open_local": "00:00:00",
            "close_local": "23:59:59",
            "evidence_url": "https://global.krx.co.kr/synthetic/wp3d4s-window",
            "evidence_retrieved_at": now.to_rfc3339_opts(SecondsFormat::Millis, true),
            "evidence_sha256": format!("sha256:{}", "a".repeat(64))
        }]
    }))
    .map_err(|_| TestFailure("synthetic window serialization failed"))?;
    let digest = format!("sha256:{:x}", Sha256::digest(&bytes));
    let contract =
        collectors::intraday_quotes::IntradaySessionWindowContract::from_bytes(&bytes, &digest)
            .map_err(|_| TestFailure("synthetic window contract validation failed"))?;
    Ok((contract, digest))
}

fn window_bounds(date: NaiveDate) -> TestResult<(DateTime<Utc>, DateTime<Utc>)> {
    let kst = FixedOffset::east_opt(9 * 60 * 60).ok_or(TestFailure("KST offset unavailable"))?;
    let open = date
        .and_hms_opt(0, 0, 0)
        .ok_or(TestFailure("synthetic window opening is invalid"))?
        .and_local_timezone(kst)
        .single()
        .ok_or(TestFailure("synthetic window opening is ambiguous"))?
        .with_timezone(&Utc);
    let close = date
        .and_hms_opt(23, 59, 59)
        .ok_or(TestFailure("synthetic window close is invalid"))?
        .and_local_timezone(kst)
        .single()
        .ok_or(TestFailure("synthetic window close is ambiguous"))?
        .with_timezone(&Utc);
    Ok((open, close))
}

fn snapshot_calendar(
    fixture: &boundary::Fixture,
    window_sha256: &str,
) -> TestResult<IntradaySessionProof> {
    IntradaySessionProof::new(
        fixture.session_date,
        fixture.session.calendar_source_batch_id,
        fixture.session.calendar_content_sha256.clone(),
        window_sha256.to_owned(),
    )
    .map_err(|_| TestFailure("synthetic calendar proof setup failed"))
}

async fn seed_calendar_lineage(
    database: &boundary::DisposableDatabase,
    fixture: &boundary::Fixture,
    window_sha256: &str,
    retrieved_at: DateTime<Utc>,
) -> TestResult<()> {
    let calendar = snapshot_calendar(fixture, window_sha256)?;
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
    .bind(format!(
        "synthetic-wp3d4s-calendar/{batch_id}/calendar.json"
    ))
    .bind(&content_sha256)
    .bind(retrieved_at)
    .bind(batch_id)
    .execute(&database.migration_owner)
    .await
    .map_err(|_| TestFailure("synthetic calendar batch seeding failed"))?;
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
            .map_err(|_| TestFailure("synthetic calendar lineage seeding failed"))?;
    }
    Ok(())
}

pub(super) struct LeaseRenewer {
    sequence: Arc<std::sync::atomic::AtomicU64>,
    stop: watch::Sender<bool>,
    task: Option<JoinHandle<TestResult<()>>>,
}

impl LeaseRenewer {
    pub(super) fn start(
        app: PgPool,
        owner: Uuid,
        session_hash: String,
        consumer_id: Uuid,
        identities: Vec<crate::owner_equity_v2::market_stream::StreamLeaseIdentity>,
        initial_sequence: u64,
        active: Arc<AtomicUsize>,
    ) -> Self {
        let sequence = Arc::new(std::sync::atomic::AtomicU64::new(initial_sequence));
        let worker_sequence = Arc::clone(&sequence);
        let (stop, mut stop_rx) = watch::channel(false);
        let task = tokio::spawn(async move {
            let _active = ActiveTask::new(active);
            let repository = OwnerMarketStreamRepository::new(app);
            let first = tokio::time::Instant::now() + Duration::from_secs(5);
            let mut ticker = tokio::time::interval_at(first, Duration::from_secs(5));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    changed = stop_rx.changed() => {
                        if changed.is_err() || *stop_rx.borrow_and_update() {
                            return Ok(());
                        }
                    }
                    _ = ticker.tick() => {}
                }
                if *stop_rx.borrow() {
                    return Ok(());
                }
                let next_sequence = worker_sequence.load(Ordering::Acquire).saturating_add(1);
                let request = StreamLeaseRequest::new(
                    consumer_id,
                    next_sequence,
                    identities.clone(),
                    format!("d4s-{}-{next_sequence}", consumer_id.simple()),
                )
                .map_err(|_| TestFailure("synthetic lease renewal request was rejected"))?;
                let renewed = repository
                    .replace_stream_lease(owner, &session_hash, &request)
                    .await
                    .map_err(|_| TestFailure("synthetic lease renewal failed"))?;
                worker_sequence.store(renewed.renewal_sequence, Ordering::Release);
            }
        });
        Self {
            sequence,
            stop,
            task: Some(task),
        }
    }

    pub(super) fn is_running(&self) -> bool {
        self.task.as_ref().is_some_and(|task| !task.is_finished())
    }

    pub(super) async fn stop_and_join(
        &mut self,
        absolute_deadline: TokioInstant,
    ) -> TestResult<u64> {
        self.request_stop();
        let phase_deadline = (TokioInstant::now() + FIXTURE_JOIN_BOUND).min(absolute_deadline);
        self.join_until(phase_deadline).await
    }

    pub(super) fn request_stop(&self) {
        self.stop.send_replace(true);
    }

    pub(super) async fn join_until(&mut self, deadline: tokio::time::Instant) -> TestResult<u64> {
        let Some(task) = self.task.as_mut() else {
            return Ok(self.sequence.load(Ordering::Acquire));
        };
        let outcome = match tokio::time::timeout_at(deadline, &mut *task).await {
            Ok(Ok(Ok(()))) => Ok(self.sequence.load(Ordering::Acquire)),
            Ok(Ok(Err(error))) => Err(error),
            Ok(Err(_)) => failure("lease renewer task failed to join"),
            Err(_) => {
                if let Some(task) = self.task.as_mut() {
                    task.abort();
                    let _ = task.await;
                }
                failure("lease renewer exceeded its owned cleanup bound")
            }
        };
        self.task.take();
        if TokioInstant::now() > deadline {
            return failure("lease renewer joined beyond its absolute cleanup deadline");
        }
        outcome
    }
}

pub(super) struct LoopbackTransport {
    pub(super) client: MarketStreamClient,
    pub(super) domain: MarketStreamDomain,
    stop: watch::Sender<bool>,
    tasks: Vec<JoinHandle<TestResult<()>>>,
    metrics: MetricsHandle,
    active: Arc<AtomicUsize>,
    binding_slot: Uuid,
    binding_generation: u64,
    _state_dir: TempDir,
}

impl LoopbackTransport {
    pub(super) async fn bind(slot: Uuid, date: NaiveDate) -> TestResult<Self> {
        let approval_listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|_| TestFailure("synthetic approval listener bind failed"))?;
        let approval_port = approval_listener
            .local_addr()
            .map_err(|_| TestFailure("synthetic approval listener address failed"))?
            .port();
        let websocket_listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|_| TestFailure("synthetic WebSocket listener bind failed"))?;
        let websocket_port = websocket_listener
            .local_addr()
            .map_err(|_| TestFailure("synthetic WebSocket listener address failed"))?
            .port();
        let state_dir = tempfile::tempdir()
            .map_err(|_| TestFailure("synthetic state directory creation failed"))?;
        let domain = MarketStreamDomain::for_test(state_dir.path(), slot)
            .map_err(|_| TestFailure("synthetic market-stream domain setup failed"))?;
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let approval = ApprovalClient::for_loopback(
            &format!("http://127.0.0.1:{approval_port}"),
            Secret::new(SYNTHETIC_APP_KEY.to_owned()),
            Secret::new(SYNTHETIC_APP_SECRET.to_owned()),
            domain.clone(),
            "7",
        )
        .map_err(|_| TestFailure("synthetic approval client setup failed"))?
        .with_clock(Arc::clone(&clock));
        if !approval.matches_runtime_binding(slot, 7) {
            return failure("synthetic approval binding mismatch");
        }
        let date_wire = date
            .format("%Y%m%d")
            .to_string()
            .parse::<u32>()
            .map_err(|_| TestFailure("synthetic session date conversion failed"))?;
        let kst =
            FixedOffset::east_opt(9 * 60 * 60).ok_or(TestFailure("KST offset unavailable"))?;
        let midnight_ms = date
            .and_hms_opt(0, 0, 0)
            .ok_or(TestFailure("synthetic session midnight invalid"))?
            .and_local_timezone(kst)
            .single()
            .ok_or(TestFailure("synthetic session midnight ambiguous"))?
            .timestamp_millis();
        let proof = MarketStreamSessionProof::new(date_wire, 0, 235_959, midnight_ms)
            .map_err(|_| TestFailure("synthetic transport session proof invalid"))?;
        let config = MarketStreamConfig::loopback(
            approval,
            &format!("ws://127.0.0.1:{websocket_port}/tryitout"),
        )
        .map_err(|_| TestFailure("synthetic loopback client setup failed"))?
        .with_session_proof(proof)
        .with_ack_timeout(Duration::from_secs(5))
        .with_clock(clock);
        let client = MarketStreamClient::new(config);
        let metrics = MetricsHandle(Arc::new(Mutex::new(Metrics::default())));
        let active = Arc::new(AtomicUsize::new(0));
        let (stop, stop_rx) = watch::channel(false);
        let tasks = vec![
            tokio::spawn(approval_accept_loop(
                approval_listener,
                stop_rx.clone(),
                metrics.clone(),
                Arc::clone(&active),
            )),
            tokio::spawn(websocket_accept_loop(
                websocket_listener,
                stop_rx,
                metrics.clone(),
                Arc::clone(&active),
                date,
            )),
        ];
        Ok(Self {
            client,
            domain,
            stop,
            tasks,
            metrics,
            active,
            binding_slot: slot,
            binding_generation: 7,
            _state_dir: state_dir,
        })
    }

    pub(super) fn matches_runtime_binding(&self, slot: Uuid, generation: u64) -> bool {
        self.binding_slot == slot && self.binding_generation == generation
    }

    pub(super) fn metrics(&self) -> TestResult<MetricsSnapshot> {
        self.metrics.snapshot(&self.active)
    }

    pub(super) fn active_counter(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.active)
    }

    pub(super) fn task_guard(&self) -> ActiveTask {
        ActiveTask::new(Arc::clone(&self.active))
    }

    pub(super) async fn stop_and_join(
        &mut self,
        absolute_deadline: TokioInstant,
    ) -> TestResult<()> {
        self.stop.send_replace(true);
        let mut first_error = None;
        let deadline = (TokioInstant::now() + FIXTURE_JOIN_BOUND).min(absolute_deadline);
        while let Some(task) = self.tasks.last_mut() {
            match tokio::time::timeout_at(deadline, &mut *task).await {
                Ok(Ok(Ok(()))) => {}
                Ok(Ok(Err(error))) => {
                    first_error.get_or_insert(error);
                }
                Ok(Err(_)) => {
                    first_error.get_or_insert(TestFailure("synthetic listener task failed"));
                }
                Err(_) => {
                    if let Some(task) = self.tasks.last_mut() {
                        task.abort();
                        let _ = task.await;
                    }
                    first_error.get_or_insert(TestFailure("synthetic listener join timed out"));
                }
            };
            self.tasks.pop();
        }
        if self.active.load(Ordering::Acquire) != 0 {
            first_error.get_or_insert(TestFailure("synthetic fixture child remains active"));
        }
        if TokioInstant::now() > absolute_deadline {
            first_error.get_or_insert(TestFailure(
                "synthetic listener cleanup exceeded its aggregate deadline",
            ));
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

async fn approval_accept_loop(
    listener: TcpListener,
    mut stop: watch::Receiver<bool>,
    metrics: MetricsHandle,
    active: Arc<AtomicUsize>,
) -> TestResult<()> {
    let _active = ActiveTask::new(active);
    loop {
        let accepted = tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow_and_update() { return Ok(()); }
                continue;
            }
            accepted = listener.accept() => accepted,
        };
        let (mut stream, _) =
            accepted.map_err(|_| TestFailure("synthetic approval accept failed"))?;
        metrics
            .update(|value| value.approval_accepts = value.approval_accepts.saturating_add(1))?;
        let result = handle_approval(&mut stream).await;
        if result.is_err() {
            metrics.update(|value| {
                value.unexpected_requests_or_commands =
                    value.unexpected_requests_or_commands.saturating_add(1)
            })?;
            metrics.update(|value| value.server_errors = value.server_errors.saturating_add(1))?;
            return result;
        }
    }
}

async fn handle_approval(stream: &mut TcpStream) -> TestResult<()> {
    let (headers, body) = read_http_request(stream)
        .await
        .map_err(|_| TestFailure("synthetic approval request was malformed"))?;
    let first = headers
        .split(|byte| *byte == b'\n')
        .next()
        .ok_or(TestFailure("synthetic approval request line missing"))?;
    if !first.starts_with(b"POST /oauth2/Approval HTTP/1.1\r") {
        return failure("unexpected synthetic approval request kind");
    }
    let request: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|_| TestFailure("synthetic approval request body malformed"))?;
    if request.get("grant_type").and_then(|value| value.as_str()) != Some("client_credentials")
        || request.get("appkey").and_then(|value| value.as_str()) != Some(SYNTHETIC_APP_KEY)
        || request.get("secretkey").and_then(|value| value.as_str()) != Some(SYNTHETIC_APP_SECRET)
    {
        return failure("unexpected synthetic approval request metadata");
    }
    let response_body = br#"{"approval_key":"D4S_SYNTHETIC_APPROVAL_KEY"}"#;
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response_body.len()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|_| TestFailure("synthetic approval response write failed"))?;
    stream
        .write_all(response_body)
        .await
        .map_err(|_| TestFailure("synthetic approval response body write failed"))?;
    Ok(())
}

async fn read_http_request(stream: &mut TcpStream) -> io::Result<(Vec<u8>, Vec<u8>)> {
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
        return Err(io::Error::new(io::ErrorKind::InvalidData, "header bound"));
    }
    let length = header_value(&headers, b"content-length")
        .and_then(|value| std::str::from_utf8(value).ok())
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    if length > MAX_HTTP_BODY_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "body bound"));
    }
    let mut body = vec![0_u8; length];
    stream.read_exact(&mut body).await?;
    Ok((headers, body))
}

fn header_value<'a>(headers: &'a [u8], name: &[u8]) -> Option<&'a [u8]> {
    headers
        .split(|byte| *byte == b'\n')
        .skip(1)
        .find_map(|line| {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            let separator = line.iter().position(|byte| *byte == b':')?;
            let (key, suffix) = line.split_at(separator);
            let value = &suffix[1..];
            key.eq_ignore_ascii_case(name).then(|| {
                let value = value.strip_prefix(b" ").unwrap_or(value);
                value
            })
        })
}

async fn websocket_accept_loop(
    listener: TcpListener,
    mut stop: watch::Receiver<bool>,
    metrics: MetricsHandle,
    active: Arc<AtomicUsize>,
    date: NaiveDate,
) -> TestResult<()> {
    let _active = ActiveTask::new(Arc::clone(&active));
    loop {
        let accepted = tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow_and_update() { return Ok(()); }
                continue;
            }
            accepted = listener.accept() => accepted,
        };
        let (stream, _) = accepted.map_err(|_| TestFailure("synthetic WebSocket accept failed"))?;
        metrics.update(|value| {
            value.websocket_accepts = value.websocket_accepts.saturating_add(1);
            value.current_websockets = value.current_websockets.saturating_add(1);
            value.maximum_websockets = value.maximum_websockets.max(value.current_websockets);
        })?;
        let result = serve_websocket(stream, metrics.clone(), Arc::clone(&active), date).await;
        metrics.update(|value| {
            value.current_websockets = value.current_websockets.saturating_sub(1)
        })?;
        if result.is_err() {
            metrics.update(|value| value.server_errors = value.server_errors.saturating_add(1))?;
            return result;
        }
    }
}

#[derive(Debug)]
struct FixtureFrame {
    opcode: u8,
    payload: Vec<u8>,
}

async fn serve_websocket(
    mut stream: TcpStream,
    metrics: MetricsHandle,
    active: Arc<AtomicUsize>,
    date: NaiveDate,
) -> TestResult<()> {
    let (headers, _) = read_http_request(&mut stream)
        .await
        .map_err(|_| TestFailure("synthetic WebSocket handshake malformed"))?;
    let request_line = headers
        .split(|byte| *byte == b'\n')
        .next()
        .ok_or(TestFailure("synthetic WebSocket request line missing"))?;
    if !request_line.starts_with(b"GET /tryitout HTTP/1.1\r") {
        return failure("unexpected synthetic WebSocket request path");
    }
    let key = header_value(&headers, b"sec-websocket-key")
        .ok_or(TestFailure("synthetic WebSocket key missing"))?;
    let key = std::str::from_utf8(key)
        .map_err(|_| TestFailure("synthetic WebSocket key encoding invalid"))?;
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
        .map_err(|_| TestFailure("synthetic WebSocket handshake response failed"))?;
    let (read_half, write_half) = tokio::io::split(stream);
    let (sender, receiver) = mpsc::channel(4);
    let reader_active = Arc::clone(&active);
    let reader = async move {
        let _active = ActiveTask::new(reader_active);
        read_client_frames(read_half, sender).await
    };
    let writer = websocket_writer(write_half, receiver, metrics, active, date);
    tokio::pin!(reader, writer);
    // Both halves live inside their registered listener task. Cancelling that
    // task drops both futures and sockets; no nested JoinHandle can detach.
    tokio::select! {
        reader_result = &mut reader => {
            let writer_result = writer.await;
            reader_result.and(writer_result)
        }
        writer_result = &mut writer => {
            writer_result?;
            reader.await
        }
    }
}

async fn read_client_frames<R>(mut read: R, sender: mpsc::Sender<FixtureFrame>) -> TestResult<()>
where
    R: AsyncRead + Unpin,
{
    loop {
        let frame = match read_client_frame(&mut read).await {
            Ok(frame) => frame,
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(_) => return failure("synthetic WebSocket frame read failed"),
        };
        let is_close = frame.opcode == 0x8;
        sender
            .send(frame)
            .await
            .map_err(|_| TestFailure("synthetic WebSocket writer channel closed"))?;
        if is_close {
            return Ok(());
        }
    }
}

async fn read_client_frame<R>(read: &mut R) -> io::Result<FixtureFrame>
where
    R: AsyncRead + Unpin,
{
    let mut header = [0_u8; 2];
    read.read_exact(&mut header).await?;
    let fin = header[0] & 0x80 != 0;
    let reserved = header[0] & 0x70;
    let opcode = header[0] & 0x0f;
    let masked = header[1] & 0x80 != 0;
    let marker = header[1] & 0x7f;
    if !fin || reserved != 0 || !masked || !matches!(opcode, 0x1 | 0x8 | 0x9 | 0xa) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame shape"));
    }
    let length = match marker {
        0..=125 => usize::from(marker),
        126 => {
            let mut bytes = [0_u8; 2];
            read.read_exact(&mut bytes).await?;
            usize::from(u16::from_be_bytes(bytes))
        }
        127 => {
            let mut bytes = [0_u8; 8];
            read.read_exact(&mut bytes).await?;
            usize::try_from(u64::from_be_bytes(bytes))
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "frame length"))?
        }
        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "frame length")),
    };
    if length > MAX_FRAME_BYTES || (opcode & 0x8 != 0 && length > 125) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame bound"));
    }
    let mut mask = [0_u8; 4];
    read.read_exact(&mut mask).await?;
    let mut payload = vec![0_u8; length];
    read.read_exact(&mut payload).await?;
    for (index, byte) in payload.iter_mut().enumerate() {
        *byte ^= mask[index % mask.len()];
    }
    Ok(FixtureFrame { opcode, payload })
}

async fn websocket_writer<W>(
    mut write: W,
    mut receiver: mpsc::Receiver<FixtureFrame>,
    metrics: MetricsHandle,
    active: Arc<AtomicUsize>,
    date: NaiveDate,
) -> TestResult<()>
where
    W: AsyncWrite + Unpin,
{
    let _active = ActiveTask::new(active);
    let first_tick = tokio::time::Instant::now() + Duration::from_millis(250);
    let mut ticker = tokio::time::interval_at(first_tick, Duration::from_millis(250));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut subscriptions = BTreeMap::<String, i64>::new();
    loop {
        tokio::select! {
            frame = receiver.recv() => {
                let Some(frame) = frame else { return Ok(()); };
                match frame.opcode {
                    0x1 => handle_command_frame(&mut write, &mut subscriptions, frame.payload, &metrics).await?,
                    0x8 => {
                        metrics.update(|value| value.client_close_seen = true)?;
                        write_server_frame(&mut write, 0x8, &frame.payload).await
                            .map_err(|_| TestFailure("synthetic WebSocket close response failed"))?;
                        return Ok(());
                    }
                    0x9 => write_server_frame(&mut write, 0xa, &frame.payload).await
                        .map_err(|_| TestFailure("synthetic WebSocket pong failed"))?,
                    0xa => {}
                    _ => {
                        metrics.update(|value| value.unexpected_requests_or_commands =
                            value.unexpected_requests_or_commands.saturating_add(1))?;
                        return failure("unexpected synthetic WebSocket frame");
                    }
                }
            }
            _ = ticker.tick() => {
                let now = Utc::now();
                let trade_time = now.with_timezone(&FixedOffset::east_opt(9 * 60 * 60)
                    .ok_or(TestFailure("KST offset unavailable"))?).format("%H%M%S").to_string();
                let eligible = subscriptions.iter()
                    .filter(|(_, first_whole_second)| now.timestamp() >= **first_whole_second)
                    .map(|(symbol, _)| symbol.clone())
                    .collect::<Vec<_>>();
                if !eligible.is_empty() {
                    let mut records = Vec::with_capacity(eligible.len());
                    for symbol in &eligible {
                        let fields = market_fields(date, &trade_time, symbol);
                        records.push(fields.join("^"));
                    }
                    let frame = format!("0|H0STCNT0|{:03}|{}", records.len(), records.join("^"));
                    if frame.len() > MAX_FRAME_BYTES || records.len() > 30 {
                        return failure("synthetic packed market frame exceeded its bound");
                    }
                    write_server_frame(&mut write, 0x1, frame.as_bytes()).await
                        .map_err(|_| TestFailure("synthetic market frame write failed"))?;
                    metrics.update(|value| {
                        value.quote_frames = value.quote_frames.saturating_add(1);
                        value.quote_records = value.quote_records.saturating_add(records.len());
                    })?;
                }
            }
        }
    }
}

async fn handle_command_frame<W>(
    write: &mut W,
    subscriptions: &mut BTreeMap<String, i64>,
    payload: Vec<u8>,
    metrics: &MetricsHandle,
) -> TestResult<()>
where
    W: AsyncWrite + Unpin,
{
    if payload.len() > MAX_FRAME_BYTES {
        return failure("synthetic command frame exceeded its bound");
    }
    let command: serde_json::Value = serde_json::from_slice(&payload)
        .map_err(|_| TestFailure("synthetic subscription command was malformed"))?;
    let symbol = command
        .pointer("/body/input/tr_key")
        .and_then(|value| value.as_str())
        .ok_or(TestFailure("synthetic subscription symbol missing"))?;
    let tr_id = command
        .pointer("/body/input/tr_id")
        .and_then(|value| value.as_str());
    let operation = command
        .pointer("/header/tr_type")
        .and_then(|value| value.as_str());
    let approval_key = command
        .pointer("/header/approval_key")
        .and_then(|value| value.as_str());
    if tr_id != Some("H0STCNT0")
        || !boundary::MARKET_SYMBOLS.contains(&symbol)
        || approval_key != Some(SYNTHETIC_APPROVAL_KEY)
    {
        metrics.update(|value| {
            value.unexpected_requests_or_commands =
                value.unexpected_requests_or_commands.saturating_add(1)
        })?;
        return failure("synthetic subscription command binding mismatch");
    }
    let (kind, acknowledgement) = match operation {
        Some("1") => (CommandKind::Subscribe, "SUBSCRIBE SUCCESS"),
        Some("2") => (CommandKind::Unsubscribe, "UNSUBSCRIBE SUCCESS"),
        _ => {
            metrics.update(|value| {
                value.unexpected_requests_or_commands =
                    value.unexpected_requests_or_commands.saturating_add(1)
            })?;
            return failure("synthetic subscription operation invalid");
        }
    };
    let symbol = symbol.to_owned();
    let arrived_at = Instant::now();
    match kind {
        CommandKind::Subscribe => {
            let first_whole_second = Utc::now().timestamp().saturating_add(1);
            subscriptions.insert(symbol.clone(), first_whole_second);
        }
        CommandKind::Unsubscribe => {
            subscriptions.remove(&symbol);
        }
    }
    metrics.update(|value| {
        match kind {
            CommandKind::Subscribe => {
                value.subscribe_commands = value.subscribe_commands.saturating_add(1)
            }
            CommandKind::Unsubscribe => {
                value.unsubscribe_commands = value.unsubscribe_commands.saturating_add(1)
            }
        }
        if value.command_events.len() >= MAX_RECORDED_COMMANDS {
            value.event_overflow = true;
        } else {
            value.command_events.push(CommandEvent {
                symbol: symbol.clone(),
                kind,
                arrived_at,
            });
        }
    })?;
    let ack = serde_json::to_vec(&serde_json::json!({
        "header": {"tr_id": "H0STCNT0", "tr_key": symbol, "encrypt": "N"},
        "body": {"rt_cd": "0", "msg_cd": "OPSP0000", "msg1": acknowledgement}
    }))
    .map_err(|_| TestFailure("synthetic acknowledgement serialization failed"))?;
    write_server_frame(write, 0x1, &ack)
        .await
        .map_err(|_| TestFailure("synthetic acknowledgement write failed"))?;
    metrics.update(|value| {
        value.acknowledgement_frames = value.acknowledgement_frames.saturating_add(1)
    })?;
    Ok(())
}

fn market_fields(date: NaiveDate, trade_time: &str, symbol: &str) -> Vec<String> {
    let mut fields = vec![String::new(); FIELD_COUNT];
    fields[0] = symbol.to_owned();
    fields[1] = trade_time.to_owned();
    fields[2] = "70000".to_owned();
    fields[3] = "2".to_owned();
    fields[4] = "100".to_owned();
    fields[5] = "0.14".to_owned();
    fields[12] = "10".to_owned();
    fields[13] = "20".to_owned();
    fields[33] = date.format("%Y%m%d").to_string();
    fields[34] = "20".to_owned();
    fields[35] = "N".to_owned();
    fields[43] = "0".to_owned();
    fields[46] = "2".to_owned();
    fields
}

async fn write_server_frame<W>(write: &mut W, opcode: u8, payload: &[u8]) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    if payload.len() > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "server frame bound",
        ));
    }
    let mut header = Vec::with_capacity(10);
    header.push(0x80 | (opcode & 0x0f));
    match payload.len() {
        0..=125 => header.push(payload.len() as u8),
        126..=65535 => {
            header.push(126);
            header.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        }
        _ => {
            header.push(127);
            header.extend_from_slice(&(payload.len() as u64).to_be_bytes());
        }
    }
    write.write_all(&header).await?;
    write.write_all(payload).await
}
