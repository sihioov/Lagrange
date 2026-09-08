use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::intraday_producer_pipeline_support::{
    PipelineClient, RequestRecord, ResponseBarrier, ScriptedTransport,
};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use collectors::intraday_quotes::{INTRADAY_QUOTE_PATH, INTRADAY_QUOTE_TR_ID};
use job_queue::owner_equity_v2::{EligibilityCheck, IntradayQuoteReader};
use kis_client::{IntradayAttemptError, IntradayAttemptOutcome};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tokio::sync::{Notify, oneshot, watch};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::intraday_quotes_support::{IntradayTestDb, MembershipFixture};

#[derive(Clone)]
pub struct OutcomeBarrier {
    captured: Arc<Notify>,
    release: Arc<Notify>,
    outcome: Arc<std::sync::Mutex<Option<AttemptOutcomeClass>>>,
}

impl OutcomeBarrier {
    pub fn new() -> Self {
        Self {
            captured: Arc::new(Notify::new()),
            release: Arc::new(Notify::new()),
            outcome: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    pub async fn wait_for_capture(&self) {
        self.captured.notified().await;
    }

    pub fn release(&self) {
        self.release.notify_one();
    }

    fn record_outcome(&self, outcome: &IntradayAttemptOutcome) {
        let class = match outcome {
            IntradayAttemptOutcome::Success(_) => AttemptOutcomeClass::Success,
            IntradayAttemptOutcome::Failed { error, .. } => match error {
                IntradayAttemptError::CallerIneligible => AttemptOutcomeClass::CallerIneligible,
                IntradayAttemptError::Timeout => AttemptOutcomeClass::Timeout,
                IntradayAttemptError::Busy => AttemptOutcomeClass::Busy,
                _ => AttemptOutcomeClass::Other,
            },
        };
        *self.outcome.lock().expect("outcome observation") = Some(class);
    }

    pub fn outcome(&self) -> Option<AttemptOutcomeClass> {
        *self.outcome.lock().expect("outcome observation")
    }
}

pub struct OutcomeHoldingReader {
    client: Arc<PipelineClient>,
    barrier: OutcomeBarrier,
}

impl OutcomeHoldingReader {
    pub fn new(client: Arc<PipelineClient>, barrier: OutcomeBarrier) -> Arc<Self> {
        Arc::new(Self { client, barrier })
    }
}

#[async_trait]
impl IntradayQuoteReader for OutcomeHoldingReader {
    async fn get_intraday_attempt(
        &self,
        query: &[(String, String)],
        eligibility_check: EligibilityCheck,
    ) -> IntradayAttemptOutcome {
        let outcome = self
            .client
            .get_intraday_attempt_outcome(INTRADAY_QUOTE_PATH, INTRADAY_QUOTE_TR_ID, query, || {
                eligibility_check()
            })
            .await;
        self.barrier.record_outcome(&outcome);
        self.barrier.captured.notify_one();
        self.barrier.release.notified().await;
        outcome
    }
}

#[derive(Clone)]
pub struct EligibilityBarrier {
    entered: Arc<Notify>,
    proceed: Arc<Notify>,
}

impl EligibilityBarrier {
    pub fn new() -> Self {
        Self {
            entered: Arc::new(Notify::new()),
            proceed: Arc::new(Notify::new()),
        }
    }

    pub async fn wait_for_entry(&self) {
        self.entered.notified().await;
    }

    pub fn release(&self) {
        self.proceed.notify_one();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EligibilityReturn {
    True,
    False,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptOutcomeClass {
    Success,
    CallerIneligible,
    Timeout,
    Busy,
    Other,
}

#[derive(Clone)]
pub struct EligibilityObservation {
    callback_entered: Arc<Notify>,
    callback_proceed: Arc<Notify>,
    callback_returned: Arc<Notify>,
    returned: Arc<std::sync::Mutex<Option<EligibilityReturn>>>,
    outcome: Arc<std::sync::Mutex<Option<AttemptOutcomeClass>>>,
}

impl EligibilityObservation {
    pub fn new() -> Self {
        Self {
            callback_entered: Arc::new(Notify::new()),
            callback_proceed: Arc::new(Notify::new()),
            callback_returned: Arc::new(Notify::new()),
            returned: Arc::new(std::sync::Mutex::new(None)),
            outcome: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    pub async fn wait_for_callback_entry(&self) {
        self.callback_entered.notified().await;
    }

    pub async fn wait_for_callback_return(&self) {
        self.callback_returned.notified().await;
    }

    pub fn release_callback(&self) {
        self.callback_proceed.notify_one();
    }

    fn record_return(&self, value: &Result<bool, ()>) {
        *self.returned.lock().expect("eligibility observation") = Some(match value {
            Ok(true) => EligibilityReturn::True,
            Ok(false) => EligibilityReturn::False,
            Err(()) => EligibilityReturn::Error,
        });
        self.callback_returned.notify_one();
    }

    fn record_outcome(&self, outcome: &IntradayAttemptOutcome) {
        let class = match outcome {
            IntradayAttemptOutcome::Success(_) => AttemptOutcomeClass::Success,
            IntradayAttemptOutcome::Failed { error, .. } => match error {
                IntradayAttemptError::CallerIneligible => AttemptOutcomeClass::CallerIneligible,
                IntradayAttemptError::Timeout => AttemptOutcomeClass::Timeout,
                IntradayAttemptError::Busy => AttemptOutcomeClass::Busy,
                _ => AttemptOutcomeClass::Other,
            },
        };
        *self.outcome.lock().expect("eligibility observation") = Some(class);
    }

    pub fn returned(&self) -> Option<EligibilityReturn> {
        *self.returned.lock().expect("eligibility observation")
    }

    pub fn outcome(&self) -> Option<AttemptOutcomeClass> {
        *self.outcome.lock().expect("eligibility observation")
    }
}

pub struct EligibilityHoldingReader {
    client: Arc<PipelineClient>,
    barrier: EligibilityBarrier,
    observation: EligibilityObservation,
}

impl EligibilityHoldingReader {
    pub fn new(
        client: Arc<PipelineClient>,
        barrier: EligibilityBarrier,
        observation: EligibilityObservation,
    ) -> Arc<Self> {
        Arc::new(Self {
            client,
            barrier,
            observation,
        })
    }
}

#[async_trait]
impl IntradayQuoteReader for EligibilityHoldingReader {
    async fn get_intraday_attempt(
        &self,
        query: &[(String, String)],
        eligibility_check: EligibilityCheck,
    ) -> IntradayAttemptOutcome {
        self.barrier.entered.notify_one();
        self.barrier.proceed.notified().await;
        let observation = self.observation.clone();
        let outcome = self
            .client
            .get_intraday_attempt_outcome(INTRADAY_QUOTE_PATH, INTRADAY_QUOTE_TR_ID, query, || {
                let observation = observation.clone();
                Box::pin(async move {
                    observation.callback_entered.notify_one();
                    observation.callback_proceed.notified().await;
                    let result = eligibility_check().await;
                    observation.record_return(&result);
                    result
                })
            })
            .await;
        self.observation.record_outcome(&outcome);
        outcome
    }
}

pub struct LifecycleTask<T> {
    shutdown_tx: watch::Sender<bool>,
    handle: Option<JoinHandle<T>>,
    outcome_barrier: Option<OutcomeBarrier>,
}

impl<T> LifecycleTask<T> {
    pub fn new(
        shutdown_tx: watch::Sender<bool>,
        handle: JoinHandle<T>,
        outcome_barrier: Option<OutcomeBarrier>,
    ) -> Self {
        Self {
            shutdown_tx,
            handle: Some(handle),
            outcome_barrier,
        }
    }

    pub fn release_outcome(&mut self) {
        if let Some(barrier) = self.outcome_barrier.take() {
            barrier.release();
        }
    }

    pub async fn join(mut self, timeout: Duration) -> Result<T, String> {
        self.release_outcome();
        self.join_inner(timeout).await
    }

    pub async fn shutdown(mut self, timeout: Duration) -> Result<T, String> {
        let _ = self.shutdown_tx.send(true);
        self.release_outcome();
        self.join_inner(timeout).await
    }

    async fn join_inner(&mut self, timeout: Duration) -> Result<T, String> {
        if self.handle.is_none() {
            return Err("lifecycle task was already joined".to_owned());
        }
        let result = {
            let handle = self.handle.as_mut().expect("lifecycle handle");
            tokio::time::timeout(timeout, handle).await
        };
        match result {
            Ok(result) => result.map_err(|_| "lifecycle task panicked or was cancelled".to_owned()),
            Err(_) => {
                if let Some(handle) = self.handle.as_ref() {
                    handle.abort();
                }
                if let Some(handle) = self.handle.as_mut() {
                    let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
                }
                Err("lifecycle task did not clean up within its bounded timeout".to_owned())
            }
        }
    }
}

impl<T> Drop for LifecycleTask<T> {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(true);
        if let Some(barrier) = self.outcome_barrier.take() {
            barrier.release();
        }
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

pub struct ResponseTask<T> {
    handle: Option<JoinHandle<T>>,
    barrier: ResponseBarrier,
}

impl<T> ResponseTask<T> {
    pub fn new(handle: JoinHandle<T>, barrier: ResponseBarrier) -> Self {
        Self {
            handle: Some(handle),
            barrier,
        }
    }

    pub async fn join(mut self, timeout: Duration) -> Result<T, String> {
        self.barrier.release();
        if self.handle.is_none() {
            return Err("response task was already joined".to_owned());
        }
        let result = {
            let handle = self.handle.as_mut().expect("response handle");
            tokio::time::timeout(timeout, handle).await
        };
        match result {
            Ok(result) => result.map_err(|_| "response task panicked or was cancelled".to_owned()),
            Err(_) => {
                if let Some(handle) = self.handle.as_ref() {
                    handle.abort();
                }
                if let Some(handle) = self.handle.as_mut() {
                    let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
                }
                Err("response task did not clean up within its bounded timeout".to_owned())
            }
        }
    }
}

impl<T> Drop for ResponseTask<T> {
    fn drop(&mut self) {
        self.barrier.release();
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

pub async fn run_lifecycle_body<F, Fut>(body: F)
where
    F: FnOnce(IntradayTestDb) -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), String>> + Send + 'static,
{
    let db = IntradayTestDb::create()
        .await
        .unwrap_or_else(|error| panic!("intraday lifecycle DB setup failed: {error}"));
    let mut body_task = tokio::spawn(body(db.clone()));
    let result = match tokio::time::timeout(Duration::from_secs(90), &mut body_task).await {
        Ok(Ok(result)) => Ok(result),
        Ok(Err(_)) => Err("lifecycle body panicked or was cancelled".to_owned()),
        Err(_) => {
            body_task.abort();
            let _ = tokio::time::timeout(Duration::from_secs(2), &mut body_task).await;
            Err("lifecycle body exceeded its bounded timeout".to_owned())
        }
    };
    let cleanup = tokio::time::timeout(Duration::from_secs(10), db.drop_database()).await;
    match (result, cleanup) {
        (Ok(Ok(())), Ok(Ok(()))) => {}
        (Ok(Err(error)), Ok(Ok(()))) => panic!("intraday lifecycle body failed: {error}"),
        (Err(error), Ok(Ok(()))) => panic!("intraday lifecycle body failed: {error}"),
        (Ok(Ok(())), Ok(Err(error))) => panic!("intraday lifecycle cleanup failed: {error}"),
        (Ok(Err(error)), Ok(Err(cleanup_error))) => {
            panic!("intraday lifecycle body failed: {error}; cleanup failed: {cleanup_error}")
        }
        (Err(_), Ok(Err(cleanup_error))) => {
            panic!("intraday lifecycle body panicked; cleanup failed: {cleanup_error}")
        }
        (_, Err(_)) => panic!("intraday lifecycle cleanup exceeded its bounded timeout"),
    }
}

pub async fn wait_for_request_count(
    transport: &ScriptedTransport,
    expected: usize,
    timeout: Duration,
) -> Result<Vec<RequestRecord>, String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let requests = transport.requests();
        if requests.len() >= expected {
            return Ok(requests);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for {expected} synthetic requests; observed {}",
                requests.len()
            ));
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

pub async fn wait_for_heartbeat_advance(
    pool: &PgPool,
    owner_user_id: Uuid,
    previous: DateTime<Utc>,
    timeout: Duration,
) -> Result<DateTime<Utc>, String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let heartbeat: DateTime<Utc> = sqlx::query_scalar(
            "SELECT heartbeat_at
               FROM public.owner_intraday_quote_producers
              WHERE owner_user_id = $1",
        )
        .bind(owner_user_id)
        .fetch_one(pool)
        .await
        .map_err(|_| "could not read producer heartbeat fixture".to_owned())?;
        if heartbeat > previous {
            return Ok(heartbeat);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("timed out waiting for an independent producer heartbeat".to_owned());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

pub async fn wait_for_database_time(
    pool: &PgPool,
    target: DateTime<Utc>,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let reached: bool =
            sqlx::query_scalar("SELECT pg_catalog.clock_timestamp() >= $1::timestamptz")
                .bind(target)
                .fetch_one(pool)
                .await
                .map_err(|_| "could not observe the lifecycle database clock".to_owned())?;
        if reached {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("timed out waiting for the lifecycle database clock".to_owned());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

pub async fn wait_for_quote_version(
    pool: &PgPool,
    owner_user_id: Uuid,
    membership_id: Uuid,
    expected: i64,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let version: Option<i64> = sqlx::query_scalar(
            "SELECT quote_version
               FROM public.owner_intraday_quote_cache
              WHERE owner_user_id = $1 AND membership_id = $2",
        )
        .bind(owner_user_id)
        .bind(membership_id)
        .fetch_optional(pool)
        .await
        .map_err(|_| "could not read the intraday quote version".to_owned())?;
        if version == Some(expected) {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for quote version {expected}; observed {version:?}"
            ));
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

pub async fn wait_for_blocked_backend(
    observer: &PgPool,
    role: &str,
    query_fragment: &str,
    timeout: Duration,
) -> Result<(i32, Vec<i32>), String> {
    wait_for_blocked_backend_inner(observer, role, query_fragment, None, timeout).await
}

pub async fn wait_for_blocked_backend_with_blocker(
    observer: &PgPool,
    role: &str,
    query_fragment: &str,
    blocker_pid: i32,
    timeout: Duration,
) -> Result<(i32, Vec<i32>), String> {
    wait_for_blocked_backend_inner(observer, role, query_fragment, Some(blocker_pid), timeout).await
}

async fn wait_for_blocked_backend_inner(
    observer: &PgPool,
    role: &str,
    query_fragment: &str,
    required_blocker: Option<i32>,
    timeout: Duration,
) -> Result<(i32, Vec<i32>), String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let pid: Option<i32> = sqlx::query_scalar(
            "SELECT activity.pid
               FROM pg_catalog.pg_stat_activity AS activity
              WHERE activity.datname = pg_catalog.current_database()
                AND activity.usename = $1
                AND activity.state = 'active'
                AND activity.wait_event_type = 'Lock'
                AND activity.query LIKE '%' || $2 || '%'
                AND activity.pid <> pg_catalog.pg_backend_pid()
                AND ($3::integer IS NULL
                     OR $3::integer = ANY(pg_catalog.pg_blocking_pids(activity.pid)))
              ORDER BY activity.pid
              LIMIT 1",
        )
        .bind(role)
        .bind(query_fragment)
        .bind(required_blocker)
        .fetch_optional(observer)
        .await
        .map_err(|_| "could not observe a lifecycle lock wait".to_owned())?;
        if let Some(pid) = pid {
            let waiting_lock: bool = sqlx::query_scalar(
                "SELECT EXISTS (
                       SELECT 1
                         FROM pg_catalog.pg_locks
                        WHERE pid = $1 AND NOT granted
                   )",
            )
            .bind(pid)
            .fetch_one(observer)
            .await
            .map_err(|_| "could not inspect the lifecycle lock wait".to_owned())?;
            let blockers: Vec<i32> = sqlx::query_scalar("SELECT pg_catalog.pg_blocking_pids($1)")
                .bind(pid)
                .fetch_one(observer)
                .await
                .map_err(|_| "could not inspect lifecycle blocking pids".to_owned())?;
            if waiting_lock && !blockers.is_empty() {
                return Ok((pid, blockers));
            }
        }
        if tokio::time::Instant::now() >= deadline {
            let active: Vec<String> = sqlx::query_scalar(
                "SELECT format(
                         'pid=%s wait=%s/%s query=%s',
                         activity.pid,
                         COALESCE(activity.wait_event_type, 'none'),
                         COALESCE(activity.wait_event, 'none'),
                         activity.query
                       )
                  FROM pg_catalog.pg_stat_activity AS activity
                 WHERE activity.datname = pg_catalog.current_database()
                   AND activity.usename = $1
                   AND activity.state = 'active'
                 ORDER BY activity.pid",
            )
            .bind(role)
            .fetch_all(observer)
            .await
            .unwrap_or_default();
            return Err(format!(
                "timed out observing lifecycle role {role} waiting for {query_fragment}; active={active:?}"
            ));
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

pub async fn add_new_generation(
    db: &IntradayTestDb,
    fixture: &MembershipFixture,
) -> Result<Uuid, String> {
    let generation_id = Uuid::new_v4();
    let first_session = db
        .session_date
        .checked_sub_days(chrono::Days::new(120))
        .ok_or_else(|| "generation fixture date underflowed".to_owned())?;
    sqlx::query(
        "INSERT INTO public.owner_equity_instrument_generations
            (id, membership_id, owner_user_id, instrument_id, generation,
             target_observed_sessions, minimum_observed_sessions,
             observed_sessions, first_session, last_session)
         VALUES ($1, $2, $3, $4, 2, 261, 121, 121, $5, $6)",
    )
    .bind(generation_id)
    .bind(fixture.membership_id)
    .bind(fixture.owner_user_id)
    .bind(&fixture.instrument_id)
    .bind(first_session)
    .bind(db.session_date)
    .execute(&db.superuser)
    .await
    .map_err(|_| "could not install newer generation fixture".to_owned())?;
    sqlx::query(
        "INSERT INTO public.owner_equity_generation_admissions
            (generation_id, owner_user_id, membership_id, instrument_id,
             generation, raw_manifest_sha256, artifact_manifest_sha256,
             entitlement_sha256, capture_code_commit, materializer_code_commit)
         VALUES ($1, $2, $3, $4, 2, $5, $6, $7, $8, $8)",
    )
    .bind(generation_id)
    .bind(fixture.owner_user_id)
    .bind(fixture.membership_id)
    .bind(&fixture.instrument_id)
    .bind(format!("sha256:{}", "1".repeat(64)))
    .bind(format!("sha256:{}", "2".repeat(64)))
    .bind(format!("sha256:{}", "3".repeat(64)))
    .bind("c".repeat(40))
    .execute(&db.superuser)
    .await
    .map_err(|_| "could not install newer admission fixture".to_owned())?;
    Ok(generation_id)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableSignalRowFingerprint {
    pub xmin: String,
    pub snapshot_id: Uuid,
    pub owner_user_id: Uuid,
    pub instrument_id: String,
    pub generation_id: Uuid,
    pub generation: i64,
    pub rank: i32,
    pub signals_json: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheRowFingerprint {
    pub xmin: String,
    pub owner_user_id: Uuid,
    pub membership_id: Uuid,
    pub generation_id: Uuid,
    pub instrument_id: String,
    pub generation: i64,
    pub session_date: Option<NaiveDate>,
    pub calendar_source: Option<String>,
    pub calendar_source_version: Option<String>,
    pub calendar_source_batch_id: Option<Uuid>,
    pub calendar_content_sha256: Option<String>,
    pub window_contract_sha256: Option<String>,
    pub price: Option<String>,
    pub base_price: Option<String>,
    pub change_amount: Option<String>,
    pub change_percent: Option<String>,
    pub direction: Option<String>,
    pub halted: Option<bool>,
    pub received_at: Option<DateTime<Utc>>,
    pub last_success_at: Option<DateTime<Utc>>,
    pub quote_version: i64,
    pub last_attempt_at: DateTime<Utc>,
    pub last_failure_code: Option<String>,
    pub last_failure_at: Option<DateTime<Utc>>,
    pub producer_fence: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub async fn raw_cache_row_fingerprint(
    pool: &PgPool,
    owner_user_id: Uuid,
    membership_id: Uuid,
) -> Result<CacheRowFingerprint, String> {
    sqlx::query_as::<_, CacheRowFingerprint>(
        "SELECT xmin::text AS xmin, owner_user_id, membership_id, generation_id,
                instrument_id, generation, session_date, calendar_source,
                calendar_source_version, calendar_source_batch_id,
                calendar_content_sha256, window_contract_sha256,
                price::text AS price, base_price::text AS base_price,
                change_amount::text AS change_amount,
                change_percent::text AS change_percent, direction, halted,
                received_at, last_success_at, quote_version, last_attempt_at,
                last_failure_code, last_failure_at, producer_fence, created_at,
                updated_at
           FROM public.owner_intraday_quote_cache
          WHERE owner_user_id = $1 AND membership_id = $2",
    )
    .bind(owner_user_id)
    .bind(membership_id)
    .fetch_one(pool)
    .await
    .map_err(|_| "could not read the raw lifecycle cache row".to_owned())
}

impl sqlx::FromRow<'_, sqlx::postgres::PgRow> for CacheRowFingerprint {
    fn from_row(row: &sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        Ok(Self {
            xmin: row.try_get("xmin")?,
            owner_user_id: row.try_get("owner_user_id")?,
            membership_id: row.try_get("membership_id")?,
            generation_id: row.try_get("generation_id")?,
            instrument_id: row.try_get("instrument_id")?,
            generation: row.try_get("generation")?,
            session_date: row.try_get("session_date")?,
            calendar_source: row.try_get("calendar_source")?,
            calendar_source_version: row.try_get("calendar_source_version")?,
            calendar_source_batch_id: row.try_get("calendar_source_batch_id")?,
            calendar_content_sha256: row.try_get("calendar_content_sha256")?,
            window_contract_sha256: row.try_get("window_contract_sha256")?,
            price: row.try_get("price")?,
            base_price: row.try_get("base_price")?,
            change_amount: row.try_get("change_amount")?,
            change_percent: row.try_get("change_percent")?,
            direction: row.try_get("direction")?,
            halted: row.try_get("halted")?,
            received_at: row.try_get("received_at")?,
            last_success_at: row.try_get("last_success_at")?,
            quote_version: row.try_get("quote_version")?,
            last_attempt_at: row.try_get("last_attempt_at")?,
            last_failure_code: row.try_get("last_failure_code")?,
            last_failure_at: row.try_get("last_failure_at")?,
            producer_fence: row.try_get("producer_fence")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

pub struct ChildDropSignal(Option<oneshot::Sender<()>>);

impl ChildDropSignal {
    pub fn new(sender: oneshot::Sender<()>) -> Self {
        Self(Some(sender))
    }
}

impl Drop for ChildDropSignal {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

pub async fn seed_published_signal_row(
    db: &IntradayTestDb,
    fixture: &MembershipFixture,
) -> Result<(Uuid, DurableSignalRowFingerprint), String> {
    let snapshot_id = Uuid::new_v4();
    let universe_sha256 = format!(
        "sha256:{:x}",
        Sha256::digest(fixture.instrument_id.as_bytes())
    );
    sqlx::query(
        "INSERT INTO public.owner_equity_signal_snapshots
            (id, owner_user_id, as_of_session, universe_sha256, row_count,
             signal_code_commit)
         VALUES ($1, $2, $3, $4, 1, $5)",
    )
    .bind(snapshot_id)
    .bind(fixture.owner_user_id)
    .bind(db.session_date)
    .bind(universe_sha256)
    .bind("c".repeat(40))
    .execute(&db.superuser)
    .await
    .map_err(|_| "could not seed the durable signal snapshot".to_owned())?;
    sqlx::query(
        "INSERT INTO public.owner_equity_signal_snapshot_rows
            (snapshot_id, owner_user_id, instrument_id, membership_id,
             generation_id, generation, rank, signals_json)
         VALUES ($1, $2, $3, $4, $5, 1, 1, $6::jsonb)",
    )
    .bind(snapshot_id)
    .bind(fixture.owner_user_id)
    .bind(&fixture.instrument_id)
    .bind(fixture.membership_id)
    .bind(fixture.generation_id)
    .bind(r#"{"score":"fixture","rank":"1"}"#)
    .execute(&db.superuser)
    .await
    .map_err(|_| "could not seed the durable signal snapshot row".to_owned())?;
    sqlx::query(
        "UPDATE public.owner_equity_signal_snapshots
            SET published_at = pg_catalog.clock_timestamp()
          WHERE id = $1",
    )
    .bind(snapshot_id)
    .execute(&db.superuser)
    .await
    .map_err(|_| "could not publish the durable signal snapshot fixture".to_owned())?;
    let fingerprint = durable_signal_row_fingerprint(&db.superuser, snapshot_id).await?;
    Ok((snapshot_id, fingerprint))
}

pub async fn durable_signal_row_fingerprint(
    pool: &PgPool,
    snapshot_id: Uuid,
) -> Result<DurableSignalRowFingerprint, String> {
    sqlx::query_as::<_, DurableSignalRowFingerprint>(
        "SELECT xmin::text AS xmin, snapshot_id, owner_user_id, instrument_id,
                generation_id, generation, rank, signals_json::text AS signals_json, created_at
           FROM public.owner_equity_signal_snapshot_rows
          WHERE snapshot_id = $1",
    )
    .bind(snapshot_id)
    .fetch_one(pool)
    .await
    .map_err(|_| "could not read the durable signal snapshot fingerprint".to_owned())
}

impl sqlx::FromRow<'_, sqlx::postgres::PgRow> for DurableSignalRowFingerprint {
    fn from_row(row: &sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        Ok(Self {
            xmin: row.try_get("xmin")?,
            snapshot_id: row.try_get("snapshot_id")?,
            owner_user_id: row.try_get("owner_user_id")?,
            instrument_id: row.try_get("instrument_id")?,
            generation_id: row.try_get("generation_id")?,
            generation: row.try_get("generation")?,
            rank: row.try_get("rank")?,
            signals_json: row.try_get("signals_json")?,
            created_at: row.try_get("created_at")?,
        })
    }
}
