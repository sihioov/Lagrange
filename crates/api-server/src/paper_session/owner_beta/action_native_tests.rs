//! Private real-consumer fixture entry. Every packet is untrusted selection.
//! Root owns protected Current/ELF binding, latches, mutations and independent
//! economic/wait witnesses. A marker/local assertion is never native acceptance.
use super::*;
use futures_util::FutureExt;
use job_queue::types::{Job, SweepReport};
use sqlx::{
    ConnectOptions,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{
    any::Any,
    cell::RefCell,
    collections::BTreeSet,
    ffi::CString,
    fs::{File, Metadata, OpenOptions},
    io::Read,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::Path,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use tokio::sync::{Mutex as AsyncMutex, oneshot};

const ROOT: &str = "/run/r5-paper-checker-integration";
const LIMIT: u64 = 65_536;
const CLASSIFICATION: &str = "SYNTHETIC_TEST_AUTHORITY_NOT_ORIGIN";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Consumer {
    Queue,
    Preview,
    Settlement,
    Valuation,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Variant {
    Clean,
    RetainedChange,
    CalendarRow,
    CalendarTerminal,
    CalendarOwnerDirect,
    CalendarPolicyDirect,
    ShortHeartbeat,
    LostLease,
    ReadbackTimeout,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum ExpectedGate {
    Committed,
    EvidenceTampered,
    InputPinMismatch,
    ReaderIncompatible,
    CohortChanged,
    OriginUnproven,
    Runtime,
    Calendar,
    LeaseLost,
    ReadbackTimeout,
    ActionReadbackTimeout,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CaseSpec {
    case_id: String,
    scope_id: String,
    process_date: NaiveDate,
    target_effective_session: NaiveDate,
    target_id: Option<Uuid>,
    t_run: Option<Uuid>,
    consumer: Consumer,
    variant: Variant,
    expected_gate: ExpectedGate,
    absolute_deadline: chrono::DateTime<Utc>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseBatch {
    schema: String,
    nonce: String,
    cases: Vec<CaseSpec>,
}
fn hex(value: &str, n: usize) -> bool {
    value.len() == n
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn label(value: &str) -> bool {
    (1..=48).contains(&value.len())
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
}
fn stamp(m: &Metadata) -> (u64, u64, u64, i64, i64, i64, i64) {
    (
        m.dev(),
        m.ino(),
        m.len(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec(),
    )
}
fn private_dir(m: &Metadata, uid: u32) -> bool {
    m.is_dir() && m.uid() == uid && m.mode() & 0o7777 == 0o700
}
fn leaf(m: &Metadata, uid: u32) -> bool {
    m.is_file()
        && m.uid() == uid
        && m.mode() & 0o7777 == 0o600
        && m.nlink() == 1
        && (1..=LIMIT).contains(&m.len())
}
fn open_at(parent: &File, name: &str, flags: i32) -> Result<File, &'static str> {
    let name = CString::new(name).map_err(|_| "CASE_NAME")?;
    // SAFETY: fixed validated component, borrowed parent FD; the successful FD
    // is adopted exactly once. No borrowed checked capability is reconstructed.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return Err("CASE_OPEN");
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn read_packet() -> Result<CaseBatch, &'static str> {
    for (name, _) in std::env::vars_os() {
        if name.to_str().is_some_and(|s| {
            s.starts_with("PG")
                || s.starts_with("DB_")
                || matches!(s, "DATABASE_URL" | "DATABASE_URL_FILE")
        }) {
            return Err("CASE_AMBIENT_DB");
        }
    }
    let nonce = std::env::var("R5_INTEG_NONCE").map_err(|_| "CASE_NONCE")?;
    let pin = std::env::var("R5_PAPER_ACTION_CASE_SHA256").map_err(|_| "CASE_PIN")?;
    if !hex(&nonce, 32) || !hex(&pin, 64) {
        return Err("CASE_PIN_FORMAT");
    }
    let uid = unsafe { libc::geteuid() };
    let run = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open("/run")
        .map_err(|_| "CASE_RUN")?;
    let root = open_at(
        &run,
        "r5-paper-checker-integration",
        libc::O_RDONLY | libc::O_DIRECTORY,
    )?;
    let directory = open_at(&root, &nonce, libc::O_RDONLY | libc::O_DIRECTORY)?;
    let root_before = root.metadata().map_err(|_| "CASE_ROOT_STAT")?;
    let directory_before = directory.metadata().map_err(|_| "CASE_DIR_STAT")?;
    if !private_dir(&root_before, uid) || !private_dir(&directory_before, uid) {
        return Err("CASE_PRIVATE_PARENTS");
    }
    let path = format!("{ROOT}/{nonce}/action-case.json");
    let named = std::fs::symlink_metadata(&path).map_err(|_| "CASE_NAMED")?;
    let mut file = open_at(&directory, "action-case.json", libc::O_RDONLY)?;
    let before = file.metadata().map_err(|_| "CASE_FD_STAT")?;
    if !leaf(&named, uid) || !leaf(&before, uid) || stamp(&named) != stamp(&before) {
        return Err("CASE_IDENTITY");
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "CASE_READ")?;
    let after = file.metadata().map_err(|_| "CASE_AFTER")?;
    let named_after = std::fs::symlink_metadata(&path).map_err(|_| "CASE_AFTER_NAME")?;
    let root_after = std::fs::symlink_metadata(ROOT).map_err(|_| "CASE_ROOT_NAME")?;
    let dir_after =
        std::fs::symlink_metadata(format!("{ROOT}/{nonce}")).map_err(|_| "CASE_DIR_NAME")?;
    if !leaf(&after, uid)
        || !leaf(&named_after, uid)
        || bytes.len() as u64 > LIMIT
        || stamp(&before) != stamp(&after)
        || stamp(&after) != stamp(&named_after)
        || !private_dir(&root_after, uid)
        || !private_dir(&dir_after, uid)
        || (root_before.dev(), root_before.ino()) != (root_after.dev(), root_after.ino())
        || (directory_before.dev(), directory_before.ino()) != (dir_after.dev(), dir_after.ino())
        || format!("{:x}", Sha256::digest(&bytes)) != pin
    {
        return Err("CASE_CHANGED");
    }
    let batch: CaseBatch = serde_json::from_slice(&bytes).map_err(|_| "CASE_SCHEMA")?;
    validate_packet(&batch, &nonce, Utc::now())?;
    Ok(batch)
}
fn validate_packet(
    batch: &CaseBatch,
    nonce: &str,
    now: chrono::DateTime<Utc>,
) -> Result<(), &'static str> {
    if batch.schema != "r5-action-real-consumer-v1"
        || batch.nonce != nonce
        || !(1..=6).contains(&batch.cases.len())
    {
        return Err("CASE_BATCH");
    }
    let today = now
        .with_timezone(&FixedOffset::east_opt(9 * 60 * 60).unwrap())
        .date_naive();
    let mut names = BTreeSet::new();
    for case in &batch.cases {
        if !label(&case.case_id)
            || !label(&case.scope_id)
            || !names.insert((&case.case_id, &case.scope_id))
            || case.process_date != today
            || case.target_effective_session > today + chrono::Days::new(369)
            || case.absolute_deadline <= now
            || case.absolute_deadline > now + chrono::Duration::seconds(600)
            || case.target_id.is_some_and(|v| v.is_nil())
            || case.t_run.is_some_and(|v| v.is_nil())
        {
            return Err("CASE_ID_DATE_DEADLINE");
        }
        if matches!(case.consumer, Consumer::Settlement | Consumer::Valuation)
            && case.target_id.is_none()
        {
            return Err("CASE_TARGET_REQUIRED");
        }
        if case.consumer != Consumer::Preview
            && matches!(case.variant, Variant::ShortHeartbeat | Variant::LostLease)
        {
            return Err("CASE_PREVIEW_ONLY");
        }
        if case.consumer == Consumer::Queue
            && matches!(
                case.variant,
                Variant::CalendarRow
                    | Variant::CalendarTerminal
                    | Variant::CalendarOwnerDirect
                    | Variant::CalendarPolicyDirect
            )
        {
            return Err("CASE_CALENDAR_CONSUMER");
        }
        let gate_ok = match case.variant {
            Variant::Clean | Variant::ShortHeartbeat => {
                case.expected_gate == ExpectedGate::Committed
            }
            Variant::CalendarRow
            | Variant::CalendarTerminal
            | Variant::CalendarOwnerDirect
            | Variant::CalendarPolicyDirect => case.expected_gate == ExpectedGate::Calendar,
            Variant::LostLease => case.expected_gate == ExpectedGate::LeaseLost,
            Variant::ReadbackTimeout => matches!(
                case.expected_gate,
                ExpectedGate::ReadbackTimeout | ExpectedGate::ActionReadbackTimeout
            ),
            Variant::RetainedChange => matches!(
                case.expected_gate,
                ExpectedGate::EvidenceTampered
                    | ExpectedGate::InputPinMismatch
                    | ExpectedGate::ReaderIncompatible
                    | ExpectedGate::CohortChanged
                    | ExpectedGate::OriginUnproven
                    | ExpectedGate::Runtime
            ),
        };
        if !gate_ok {
            return Err("CASE_GATE_VARIANT");
        }
    }
    if batch.cases.iter().any(|c| c.consumer == Consumer::Preview) {
        if batch.cases.iter().any(|c| c.consumer != Consumer::Preview) {
            return Err("CASE_ONE_REAL_PREVIEW_CLAIM");
        }
        if !matches!(
            batch.cases.last().unwrap().variant,
            Variant::Clean | Variant::ShortHeartbeat | Variant::LostLease
        ) {
            return Err("CASE_PREVIEW_FINAL_REQUIRED");
        }
        if batch.cases[..batch.cases.len() - 1].iter().any(|c| {
            matches!(
                c.variant,
                Variant::Clean | Variant::ShortHeartbeat | Variant::LostLease
            )
        }) {
            return Err("CASE_PREVIEW_FINAL_ONLY");
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct Backend {
    pid: i32,
    backend_start: chrono::DateTime<Utc>,
    database: String,
    role: String,
    application_name: String,
}
#[derive(Default)]
struct ProbeState {
    backend: Vec<Backend>,
    lease_samples: Vec<Job>,
    sweeps: Vec<SweepReport>,
    // Owned auxiliary SQL/queue failures are retained, never string-dumped.
    lower: Vec<Box<dyn Any + Send>>,
    // Unserialized fault observation; absent from every ordinary fixture.
    ack_fault: Option<Arc<recovery_commit_ack_loss::Fault>>,
}
pub(super) struct Probe {
    case: CaseSpec,
    queue: JobQueue,
    claim: Option<ClaimedJob>,
    monitor: Arc<AsyncMutex<Option<Arc<ClaimHeartbeat>>>>,
    entered: AtomicBool,
    consumer_returned: AtomicBool,
    sql_entered: AtomicBool,
    direct_calendar: AtomicBool,
    calendar_checked: AtomicBool,
    state: Mutex<ProbeState>,
}
impl Probe {
    fn sql_error(&self, error: sqlx::Error) -> OwnerBetaPaperError {
        self.state.lock().unwrap().lower.push(Box::new(error));
        OwnerBetaPaperError::Sql
    }
    fn queue_error(&self, error: job_queue::QueueError) -> OwnerBetaPaperError {
        self.state.lock().unwrap().lower.push(Box::new(error));
        OwnerBetaPaperError::Sql
    }
    pub(super) fn queue_reply(&self, value: &Value) -> Result<(), OwnerBetaPaperError> {
        if matches!(value["outcome"].as_str(), Some("QUEUED" | "REPLAY"))
            && value["data"]["effective_session"] != self.case.target_effective_session.to_string()
        {
            return Err(OwnerBetaPaperError::Snapshot);
        }
        Ok(())
    }
    pub(super) fn calendar_entered(&self) {
        self.calendar_checked.store(true, Ordering::Release);
        self.mark("ACTUAL_SHARED_CALENDAR_VALIDATOR_ENTERED");
    }
    fn mark(&self, stage: &str) {
        eprintln!(
            "R5_ACTION_CASE {}",
            json!({"classification":CLASSIFICATION,"case_id":self.case.case_id,"scope_id":self.case.scope_id,"consumer":self.case.consumer,"variant":self.case.variant,"stage":stage,"external_lock_mutation_witness_required":true})
        );
    }
    async fn capture_connection(&self, pool: &PgPool) -> Result<(), sqlx::Error> {
        let row:(i32,chrono::DateTime<Utc>,String,String,String)=sqlx::query_as("SELECT pg_backend_pid(),backend_start,current_database()::text,current_user::text,current_setting('application_name') FROM pg_stat_activity WHERE pid=pg_backend_pid()")
            .fetch_one(pool).await?;
        let backend = Backend {
            pid: row.0,
            backend_start: row.1,
            database: row.2,
            role: row.3,
            application_name: row.4,
        };
        eprintln!(
            "R5_ACTION_BACKEND {}",
            json!({"case_id":self.case.case_id,"scope_id":self.case.scope_id,"stage":"CAPTURE_CONNECTION","backend":&backend})
        );
        self.state.lock().unwrap().backend.push(backend);
        Ok(())
    }
    pub(super) async fn before_sql(
        &self,
        action: &mut CheckedPaperAction<'_>,
    ) -> Result<(), OwnerBetaPaperError> {
        let identity:Result<(i32,chrono::DateTime<Utc>,String,String,String),sqlx::Error>=sqlx::query_as(
            "SELECT pg_backend_pid(),backend_start,current_database()::text,current_user::text,current_setting('application_name') FROM pg_stat_activity WHERE pid=pg_backend_pid()")
            .fetch_one(&mut **action.transaction().map_err(map_action_input_error)?).await;
        let row = match identity {
            Ok(row) => row,
            Err(error) => {
                action.retain_sql_error(error);
                return Err(OwnerBetaPaperError::Sql);
            }
        };
        let backend = Backend {
            pid: row.0,
            backend_start: row.1,
            database: row.2,
            role: row.3,
            application_name: row.4,
        };
        eprintln!(
            "R5_ACTION_BACKEND {}",
            json!({"case_id":self.case.case_id,"scope_id":self.case.scope_id,"stage":"SQL_ENTERED","backend":backend})
        );
        let ack_pid = backend.pid;
        self.state.lock().unwrap().backend.push(backend);
        self.sql_entered.store(true, Ordering::Release);
        let ack_fault = self.state.lock().unwrap().ack_fault.clone();
        if let Some(fault) = ack_fault {
            fault
                .arm(ack_pid)
                .map_err(|_| OwnerBetaPaperError::Runtime)?;
        }
        if self.case.variant == Variant::ShortHeartbeat {
            let claim = self.claim.as_ref().ok_or(OwnerBetaPaperError::Snapshot)?;
            let baseline = self
                .queue
                .get_by_id(claim.job.id)
                .await
                .map_err(|error| self.queue_error(error))?;
            let initial = baseline.locked_at.ok_or(OwnerBetaPaperError::LeaseLost)?;
            self.state.lock().unwrap().lease_samples.push(baseline);
            let total = Instant::now();
            let mut previous = initial;
            for gate in 0..3_i32 {
                let key = latch_key(&self.case, "heartbeat");
                eprintln!(
                    "R5_ACTION_GATE {}",
                    json!({"case_id":self.case.case_id,"scope_id":self.case.scope_id,"key1":key,"key2":gate,"stage":"HEARTBEAT_GATE_BEGIN"})
                );
                let started = Instant::now();
                let original =
                    sqlx::query_scalar::<_, i32>("SELECT 1 FROM pg_advisory_xact_lock($1,$2)")
                        .bind(key)
                        .bind(gate)
                        .fetch_one(&mut **action.transaction().map_err(map_action_input_error)?)
                        .await;
                match original {
                    Ok(value) => action.retain_value(json!(value)),
                    Err(error) => {
                        action.retain_sql_error(error);
                        return Err(OwnerBetaPaperError::Sql);
                    }
                }
                if started.elapsed() >= Duration::from_secs(4) {
                    return Err(OwnerBetaPaperError::Runtime);
                }
                let sample = self
                    .queue
                    .get_by_id(claim.job.id)
                    .await
                    .map_err(|error| self.queue_error(error))?;
                if sample.status != job_queue::types::JobStatus::Running
                    || sample.attempt_count != claim.attempt.attempt_no
                    || sample.locked_by.as_deref() != Some(&claim.worker_id)
                    || sample.locked_at.is_none_or(|at| at <= previous)
                {
                    return Err(OwnerBetaPaperError::LeaseLost);
                }
                let same_attempt:Uuid=sqlx::query_scalar("SELECT id FROM public.job_attempts WHERE job_id=$1 AND attempt_no=$2 AND outcome='RUNNING'")
                    .bind(claim.job.id).bind(claim.attempt.attempt_no).fetch_one(&self.queue.pool()).await.map_err(|error|self.sql_error(error))?;
                if same_attempt != claim.attempt.id {
                    return Err(OwnerBetaPaperError::LeaseLost);
                }
                previous = sample.locked_at.unwrap();
                self.state.lock().unwrap().lease_samples.push(sample);
            }
            if total.elapsed() <= Duration::from_secs(10) {
                return Err(OwnerBetaPaperError::Runtime);
            }
            self.mark("THREE_ACTUAL_HEARTBEAT_SAMPLES");
        }
        Ok(())
    }
    async fn expire_actual_claim(&self) -> Result<(), OwnerBetaPaperError> {
        if self.case.variant != Variant::LostLease {
            return Ok(());
        }
        if let Some(monitor) = self.monitor.lock().await.as_mut() {
            monitor.stop().await;
        }
        let claim = self.claim.as_ref().ok_or(OwnerBetaPaperError::Snapshot)?;
        loop {
            if Utc::now() >= self.case.absolute_deadline {
                return Err(OwnerBetaPaperError::Runtime);
            }
            let now: chrono::DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
                .fetch_one(&self.queue.pool())
                .await
                .map_err(|error| self.sql_error(error))?;
            let job = self
                .queue
                .get_by_id(claim.job.id)
                .await
                .map_err(|error| self.queue_error(error))?;
            let expiry =
                job.locked_at.ok_or(OwnerBetaPaperError::LeaseLost)? + chrono::Duration::seconds(5);
            self.state.lock().unwrap().lease_samples.push(job);
            if now >= expiry {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let sweep = self
            .queue
            .sweep_paper_for_owner(claim.job.owner_user_id)
            .await
            .map_err(|error| self.queue_error(error))?;
        self.state.lock().unwrap().sweeps.push(sweep);
        self.mark("ACTUAL_EXPIRED_CLAIM_SWEPT");
        Ok(())
    }
}
fn latch_key(case: &CaseSpec, label: &str) -> i32 {
    let digest = Sha256::digest(format!("{}:{}:{label}", case.case_id, case.scope_id).as_bytes());
    i32::from_be_bytes(digest[..4].try_into().unwrap())
}
type LowerHolds = Rc<RefCell<Vec<Box<dyn Any>>>>;
fn hold<T: 'static>(holds: &LowerHolds, value: T) {
    holds.borrow_mut().push(Box::new(value));
}
struct DirectCalendar<'c, 's, 'a> {
    context: &'c PaperActionContext<'c, 's, 'a>,
    signal: TradingDate,
    effective: TradingDate,
    evidence: String,
    link: Value,
}
struct Entered<'c, 's, 'a, C> {
    real: C,
    probe: Arc<Probe>,
    direct: Option<DirectCalendar<'c, 's, 'a>>,
    lower: LowerHolds,
}
impl<T: 'static, C: PaperFinancialConsumer<T, OwnerBetaPaperError>>
    PaperFinancialConsumer<T, OwnerBetaPaperError> for Entered<'_, '_, '_, C>
{
    fn run<'a, 'hold>(
        self,
        action: &'a mut CheckedPaperAction<'hold>,
    ) -> LocalInputFuture<'a, Result<T, OwnerBetaPaperError>>
    where
        Self: 'a,
        'hold: 'a,
    {
        Box::pin(async move {
            self.probe.entered.store(true, Ordering::Release);
            self.probe.mark("ENTERED");
            if self
                .probe
                .case
                .t_run
                .is_some_and(|expected| expected != action.inputs().signal().input().run_id())
            {
                return Err(OwnerBetaPaperError::Snapshot);
            }
            self.probe.expire_actual_claim().await?;
            if matches!(
                self.probe.case.variant,
                Variant::CalendarOwnerDirect | Variant::CalendarPolicyDirect
            ) {
                self.probe.direct_calendar.store(true, Ordering::Release);
                let direct = self.direct.as_ref().ok_or(OwnerBetaPaperError::Snapshot)?;
                let original =
                    direct_calendar_refusal(action, direct, self.probe.case.variant, &self.lower);
                self.probe
                    .mark("SHARED_VALIDATOR_DIRECT_RETURNED_NOT_WHOLE_CONSUMER");
                return original.and(Err(OwnerBetaPaperError::Capture));
            }
            let original = self.real.run(action).await;
            self.probe.consumer_returned.store(true, Ordering::Release);
            self.probe.mark("REAL_CONSUMER_RETURNED");
            let _ = &self.lower;
            original
        })
    }
}

enum HeldScope {
    Unit(
        Result<
            (),
            job_queue::owner_beta::paper::evidence::financial::CheckedPaperActionFailure<
                (),
                OwnerBetaPaperError,
            >,
        >,
    ),
    Flag(
        Result<
            bool,
            job_queue::owner_beta::paper::evidence::financial::CheckedPaperActionFailure<
                bool,
                OwnerBetaPaperError,
            >,
        >,
    ),
}
#[derive(Clone, Debug, Serialize)]
struct CaseReport {
    case_id: String,
    scope_id: String,
    entered: bool,
    consumer_returned: bool,
    sql_entered: bool,
    scope_finished: bool,
    committed: bool,
    commit_started: bool,
    primary: Option<String>,
    secondary: usize,
    original_error: Option<String>,
    success: bool,
    direct_calendar: bool,
    local_gate_matched: bool,
    refusal_stage: &'static str,
}
impl HeldScope {
    fn report(&self, probe: &Probe, committed: bool, commit_started: bool) -> CaseReport {
        let (primary, secondary, original_error, success) = match self {
            Self::Unit(Ok(())) => (None, 0, None, true),
            Self::Flag(Ok(value)) => (None, 0, None, *value),
            Self::Unit(Err(f)) => (
                f.first_error().map(|e| format!("{e:?}")),
                f.secondary_error_count(),
                f.original()
                    .and_then(|r| r.as_ref().err())
                    .map(ToString::to_string),
                false,
            ),
            Self::Flag(Err(f)) => (
                f.first_error().map(|e| format!("{e:?}")),
                f.secondary_error_count(),
                f.original()
                    .and_then(|r| r.as_ref().err())
                    .map(ToString::to_string),
                false,
            ),
        };
        let entered = probe.entered.load(Ordering::Acquire);
        let sql_entered = probe.sql_entered.load(Ordering::Acquire);
        let original_kind = original_error.as_deref();
        let local_gate_matched = match probe.case.expected_gate {
            ExpectedGate::Committed => entered && success && committed,
            ExpectedGate::LeaseLost => {
                entered && !commit_started && original_kind == Some("PAPER_BETA_LEASE_LOST")
            }
            ExpectedGate::Calendar => {
                entered
                    && (probe.calendar_checked.load(Ordering::Acquire)
                        || probe.direct_calendar.load(Ordering::Acquire))
                    && !commit_started
                    && original_kind == Some("PAPER_BETA_CAPTURE_REJECTED")
            }
            ExpectedGate::Runtime => {
                entered
                    && sql_entered
                    && !commit_started
                    && (original_kind == Some("PAPER_BETA_RUNTIME_REJECTED")
                        || primary.as_deref() == Some("Runtime"))
            }
            ExpectedGate::EvidenceTampered => {
                entered
                    && sql_entered
                    && !commit_started
                    && original_kind == Some("PAPER_BETA_CAPTURE_REJECTED")
                    && primary.as_deref() == Some("Input(EvidenceTampered)")
            }
            ExpectedGate::InputPinMismatch => {
                entered
                    && sql_entered
                    && !commit_started
                    && original_kind == Some("PAPER_BETA_CAPTURE_REJECTED")
                    && primary.as_deref() == Some("Input(InputPinMismatch)")
            }
            // This proves only local refusal+scope completion. Root must prove
            // actual T lock/55P03 independently; it is never a native verdict.
            ExpectedGate::ReaderIncompatible => {
                entered
                    && sql_entered
                    && !commit_started
                    && original_kind == Some("PAPER_BETA_CAPTURE_REJECTED")
                    && primary.as_deref() == Some("Input(ReaderIncompatible)")
            }
            ExpectedGate::CohortChanged => {
                entered
                    && sql_entered
                    && !commit_started
                    && original_kind == Some("PAPER_BETA_CAPTURE_REJECTED")
                    && primary.as_deref() == Some("Input(CohortChanged)")
            }
            ExpectedGate::OriginUnproven => {
                entered
                    && sql_entered
                    && !commit_started
                    && original_kind == Some("PAPER_BETA_CAPTURE_REJECTED")
                    && primary.as_deref() == Some("Input(OriginUnproven)")
            }
            ExpectedGate::ActionReadbackTimeout => {
                entered
                    && sql_entered
                    && !commit_started
                    && original_kind == Some("PAPER_BETA_CAPTURE_REJECTED")
                    && primary.as_deref() == Some("Input(ReaderIncompatible)")
            }
            ExpectedGate::ReadbackTimeout => {
                !entered
                    && !commit_started
                    && primary.as_deref() == Some("Input(ReaderIncompatible)")
            }
        };
        let refusal_stage = if !entered {
            "CAPTURE_BEFORE_ENTRY"
        } else if probe.direct_calendar.load(Ordering::Acquire) {
            "DIRECT_SHARED_CALENDAR"
        } else if probe.calendar_checked.load(Ordering::Acquire)
            && original_kind == Some("PAPER_BETA_CAPTURE_REJECTED")
        {
            "CONSUMER_SHARED_CALENDAR"
        } else if !sql_entered {
            "CONSUMER_BEFORE_SQL"
        } else if !commit_started {
            "CONSUMER_SQL_PRECOMMIT"
        } else {
            "COMMIT_OR_POSTCHECK"
        };
        CaseReport {
            case_id: probe.case.case_id.clone(),
            scope_id: probe.case.scope_id.clone(),
            entered,
            consumer_returned: probe.consumer_returned.load(Ordering::Acquire),
            sql_entered,
            scope_finished: true,
            committed,
            commit_started,
            primary,
            secondary,
            original_error,
            success,
            direct_calendar: probe.direct_calendar.load(Ordering::Acquire),
            local_gate_matched,
            refusal_stage,
        }
    }
}

// Exact rows, never seven counts. All selectors are compiled literals. Wait/job
// transitions are stored separately, without hiding any financial row change.
struct EconomicRows {
    immutable: Value,
    allowed_lifecycle: Value,
    hash: String,
}
async fn economic_rows(
    pool: &PgPool,
    account: Uuid,
    owner: Uuid,
) -> (
    Result<EconomicRows, sqlx::Error>,
    Option<Result<(), sqlx::Error>>,
) {
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(error) => return (Err(error), None),
    };
    let original=async{
        sqlx::raw_sql("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY").execute(&mut *tx).await?;
        set_paper_transaction_timeouts(&mut tx).await?;
        let mut immutable=serde_json::Map::new();
        for (name,query) in [
            ("accounts","SELECT to_jsonb(x)::text FROM public.accounts x WHERE id=$1"),
            ("positions","SELECT to_jsonb(x)::text FROM public.positions x WHERE account_id=$1"),
            ("cash_ledger","SELECT to_jsonb(x)::text FROM public.cash_ledger x WHERE account_id=$1"),
            ("orders","SELECT to_jsonb(x)::text FROM public.orders x WHERE account_id=$1"),
            ("fills","SELECT to_jsonb(x)::text FROM public.fills x WHERE account_id=$1"),
            ("equity","SELECT to_jsonb(x)::text FROM public.daily_equity x WHERE account_id=$1"),
            ("targets","SELECT to_jsonb(x)::text FROM public.pending_targets x WHERE account_id=$1"),
            ("previews","SELECT to_jsonb(x)::text FROM public.paper_rebalance_previews x WHERE account_id=$1"),
            ("lineage","SELECT to_jsonb(x)::text FROM public.owner_beta_paper_lineage x WHERE account_id=$1"),
            ("sessions","SELECT (to_jsonb(x)-'state'-'reason_code'-'updated_at')::text FROM public.owner_beta_paper_sessions x WHERE account_id=$1"),
            ("outbox","SELECT to_jsonb(x)::text FROM public.paper_settlement_outbox x WHERE pending_target_id IN (SELECT id FROM public.pending_targets WHERE account_id=$1)"),
        ]{
            let mut rows:Vec<String>=sqlx::query_scalar(query).bind(account).fetch_all(&mut *tx).await?;
            rows.sort();
            immutable.insert(name.to_owned(),json!(rows));
        }
        let lifecycle:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('session',to_jsonb(s)::text,'events',COALESCE((SELECT jsonb_agg(to_jsonb(e)::text ORDER BY to_jsonb(e)::text) FROM public.owner_beta_paper_session_events e WHERE e.pending_target_id=s.pending_target_id),'[]')) FROM public.owner_beta_paper_sessions s WHERE account_id=$1 ORDER BY to_jsonb(s)::text")
            .bind(account).fetch_all(&mut *tx).await?;
        let jobs:Vec<String>=sqlx::query_scalar("SELECT to_jsonb(j)::text FROM public.jobs j WHERE j.owner_user_id=$1 AND job_type='owner_beta_paper_preview' ORDER BY id")
            .bind(owner).fetch_all(&mut *tx).await?;
        let attempts:Vec<String>=sqlx::query_scalar("SELECT to_jsonb(a)::text FROM public.job_attempts a JOIN public.jobs j ON j.id=a.job_id WHERE j.owner_user_id=$1 AND j.job_type='owner_beta_paper_preview' ORDER BY a.job_id,a.attempt_no")
            .bind(owner).fetch_all(&mut *tx).await?;
        let immutable=Value::Object(immutable);let hash=prefixed_sha256(&serde_json::to_vec(&immutable).unwrap());
        Ok::<_,sqlx::Error>(EconomicRows{immutable,allowed_lifecycle:json!({"sessions_and_wait_events":lifecycle,"jobs":jobs,"attempts":attempts}),hash})
    }.await;
    let rollback = tx.rollback().await;
    (original, Some(rollback))
}
fn economic_value(
    value: &(
        Result<EconomicRows, sqlx::Error>,
        Option<Result<(), sqlx::Error>>,
    ),
) -> &EconomicRows {
    match value {
        (Ok(rows), Some(Ok(()))) => rows,
        _ => panic!("WHOLE_ECONOMIC_SNAPSHOT_OR_CLEANUP_FAILED"),
    }
}

struct RealWork {
    pool: PgPool,
    queue: JobQueue,
    probe: Arc<Probe>,
    claim: Option<ClaimedJob>,
    heartbeat: Option<Arc<ClaimHeartbeat>>,
    report: oneshot::Sender<CaseReport>,
}
impl PaperActionWork<CaseReport, OwnerBetaPaperError> for RealWork {
    fn run<'ctx, 'scope, 'attempt>(
        self: Box<Self>,
        context: &'ctx PaperActionContext<'ctx, 'scope, 'attempt>,
    ) -> PaperActionFuture<'ctx, CaseReport, OwnerBetaPaperError> {
        Box::pin(async move {
            let result = async {
                if context.runtime().owner_user_id()
                    != std::env::var("R5_PAPER_OWNER_USER_ID")
                        .map_err(|_| OwnerBetaPaperError::Runtime)?
                    || context.runtime().account_id()
                        != std::env::var("R5_PAPER_ACCOUNT_ID")
                            .map_err(|_| OwnerBetaPaperError::Runtime)?
                {
                    return Err(OwnerBetaPaperError::Runtime);
                }
                let case = &self.probe.case;
                let lower: LowerHolds = Rc::new(RefCell::new(Vec::new()));
                let work = OwnerBetaPreviewWork {
                    pool: self.pool.clone(),
                    queue: self.queue.clone(),
                    worker_id: format!("action-{}", case.scope_id),
                    heartbeat_period: Duration::from_secs(1),
                    native_probe: Some(Arc::clone(&self.probe)),
                };
                let (held, committed, started) = match case.consumer {
                    Consumer::Queue => {
                        let selector = PaperActionInputRequest::LatestSignal;
                        let outcome = with_checked_paper_action(
                            &self.pool,
                            context,
                            &selector,
                            Entered {
                                real: QueueSignalConsumer {
                                    work: &work,
                                    context,
                                },
                                probe: Arc::clone(&self.probe),
                                direct: None,
                                lower: Rc::clone(&lower),
                            },
                        )
                        .await;
                        let flags = (outcome.committed(), outcome.commit_started());
                        (HeldScope::Unit(outcome.into_result()), flags.0, flags.1)
                    }
                    Consumer::Preview => {
                        let claim = self.claim.as_ref().ok_or(OwnerBetaPaperError::Snapshot)?;
                        let value: Value = sqlx::query_scalar(
                            "SELECT public.snapshot_owner_beta_paper_preview_v1($1,$2,$3)",
                        )
                        .bind(
                            claim.job.payload_json["preview_id"]
                                .as_str()
                                .and_then(|s| Uuid::parse_str(s).ok())
                                .ok_or(OwnerBetaPaperError::Snapshot)?,
                        )
                        .bind(claim.job.id)
                        .bind(context.runtime().config_sha256())
                        .fetch_one(&self.pool)
                        .await
                        .map_err(|error| self.probe.sql_error(error))?;
                        let envelope = decode_action(value, "SNAPSHOT_PREVIEW")?;
                        if envelope.outcome != "OK" {
                            return Err(OwnerBetaPaperError::Snapshot);
                        }
                        let mut snapshot: PreviewSnapshot =
                            serde_json::from_value(envelope.data)
                                .map_err(|_| OwnerBetaPaperError::Snapshot)?;
                        validate_snapshot_identity(context, claim, &snapshot)?;
                        verify_selectors(
                            case,
                            snapshot.recommendation_run_id,
                            TradingDate::parse(&snapshot.effective_session)
                                .map_err(|_| OwnerBetaPaperError::Snapshot)?
                                .as_naive_date(),
                        )?;
                        mutate_calendar(
                            case,
                            &mut snapshot.calendar_link,
                            &mut snapshot.calendar_evidence_sha256,
                        )?;
                        let direct = Some(DirectCalendar {
                            context,
                            signal: TradingDate::parse(&snapshot.signal_date)
                                .map_err(|_| OwnerBetaPaperError::Snapshot)?,
                            effective: TradingDate::parse(&snapshot.effective_session)
                                .map_err(|_| OwnerBetaPaperError::Snapshot)?,
                            evidence: snapshot.calendar_evidence_sha256.clone(),
                            link: snapshot.calendar_link.clone(),
                        });
                        let selector = PaperActionInputRequest::Preview {
                            signal: TradingDate::parse(&snapshot.signal_date)
                                .map_err(|_| OwnerBetaPaperError::Snapshot)?,
                            effective: TradingDate::parse(&snapshot.effective_session)
                                .map_err(|_| OwnerBetaPaperError::Snapshot)?,
                            run_id: snapshot.recommendation_run_id,
                        };
                        let outcome = with_checked_paper_action(
                            &self.pool,
                            context,
                            &selector,
                            Entered {
                                real: PreviewConsumer {
                                    work: &work,
                                    context,
                                    claim,
                                    heartbeat: self
                                        .heartbeat
                                        .as_deref()
                                        .ok_or(OwnerBetaPaperError::LeaseLost)?,
                                    snapshot,
                                },
                                probe: Arc::clone(&self.probe),
                                direct,
                                lower: Rc::clone(&lower),
                            },
                        )
                        .await;
                        let flags = (outcome.committed(), outcome.commit_started());
                        (HeldScope::Unit(outcome.into_result()), flags.0, flags.1)
                    }
                    Consumer::Settlement | Consumer::Valuation => {
                        let mut snapshot = work
                            .snapshot_target(
                                context,
                                case.target_id.ok_or(OwnerBetaPaperError::Snapshot)?,
                            )
                            .await?;
                        validate_target_snapshot(context, &snapshot)?;
                        verify_selectors(
                            case,
                            snapshot.recommendation_run_id,
                            snapshot.effective_session,
                        )?;
                        mutate_calendar(
                            case,
                            &mut snapshot.calendar_link,
                            &mut snapshot.calendar_evidence_sha256,
                        )?;
                        let signal = TradingDate::parse(&snapshot.signal_date.to_string())
                            .map_err(|_| OwnerBetaPaperError::Snapshot)?;
                        let effective = TradingDate::parse(&snapshot.effective_session.to_string())
                            .map_err(|_| OwnerBetaPaperError::Snapshot)?;
                        let direct = || {
                            Some(DirectCalendar {
                                context,
                                signal,
                                effective,
                                evidence: snapshot.calendar_evidence_sha256.clone(),
                                link: snapshot.calendar_link.clone(),
                            })
                        };
                        let selector = if case.consumer == Consumer::Settlement {
                            PaperActionInputRequest::Settle {
                                signal,
                                effective,
                                run_id: snapshot.recommendation_run_id,
                            }
                        } else {
                            PaperActionInputRequest::Value {
                                signal,
                                effective,
                                run_id: snapshot.recommendation_run_id,
                            }
                        };
                        let outcome = if case.consumer == Consumer::Settlement {
                            with_checked_target_action(
                                &self.pool,
                                context,
                                &snapshot,
                                &selector,
                                Entered {
                                    real: SettlementConsumer {
                                        work: &work,
                                        context,
                                        target: &snapshot,
                                    },
                                    probe: Arc::clone(&self.probe),
                                    direct: direct(),
                                    lower: Rc::clone(&lower),
                                },
                            )
                            .await?
                        } else {
                            with_checked_target_action(
                                &self.pool,
                                context,
                                &snapshot,
                                &selector,
                                Entered {
                                    real: ValuationConsumer {
                                        work: &work,
                                        context,
                                        target: &snapshot,
                                    },
                                    probe: Arc::clone(&self.probe),
                                    direct: direct(),
                                    lower: Rc::clone(&lower),
                                },
                            )
                            .await?
                        };
                        let flags = (outcome.committed(), outcome.commit_started());
                        (HeldScope::Flag(outcome.into_result()), flags.0, flags.1)
                    }
                };
                self.probe.mark("SCOPE_FINISHED");
                let report = held.report(&self.probe, committed, started);
                // Whole failures + all auxiliary lower completions are kept until
                // this final local mapping, after actual checked-scope retirement.
                let _ = (&held, &lower);
                let ack_fault = self.probe.state.lock().unwrap().ack_fault.clone();
                if let Some(fault) = ack_fault {
                    // Keep the actual whole failure and all lower SQL outcomes
                    // alive until independent readback, without sending them
                    // across threads or manufacturing a checked value.
                    fault.hold_failure(&held, committed, started).await?;
                }
                Ok(report)
            }
            .await;
            let diagnostic = match &result {
                Ok(report) => report.clone(),
                Err(error) => CaseReport {
                    case_id: self.probe.case.case_id.clone(),
                    scope_id: self.probe.case.scope_id.clone(),
                    entered: self.probe.entered.load(Ordering::Acquire),
                    consumer_returned: self.probe.consumer_returned.load(Ordering::Acquire),
                    sql_entered: self.probe.sql_entered.load(Ordering::Acquire),
                    scope_finished: false,
                    committed: false,
                    commit_started: false,
                    primary: None,
                    secondary: 0,
                    original_error: Some(error.to_string()),
                    success: false,
                    direct_calendar: false,
                    local_gate_matched: false,
                    refusal_stage: "PRELUDE_OR_CAPTURE_NOT_REACHED",
                },
            };
            let _ = self.report.send(diagnostic);
            result
        })
    }
}
fn verify_selectors(
    case: &CaseSpec,
    run: Uuid,
    effective: NaiveDate,
) -> Result<(), OwnerBetaPaperError> {
    if case.target_effective_session != effective
        || case.t_run.is_some_and(|expected| expected != run)
    {
        return Err(OwnerBetaPaperError::Snapshot);
    }
    Ok(())
}
fn mutate_calendar(
    case: &CaseSpec,
    link: &mut Value,
    terminal: &mut String,
) -> Result<(), OwnerBetaPaperError> {
    match case.variant {
        Variant::CalendarRow => {
            let rows = link["covered_days"]
                .as_array_mut()
                .ok_or(OwnerBetaPaperError::Snapshot)?;
            let row = rows.first_mut().ok_or(OwnerBetaPaperError::Snapshot)?;
            let previous = row["source_row_sha256"]
                .as_str()
                .ok_or(OwnerBetaPaperError::Snapshot)?;
            row["source_row_sha256"] = json!(prefixed_sha256(previous.as_bytes()));
        }
        Variant::CalendarTerminal => *terminal = prefixed_sha256(terminal.as_bytes()),
        _ => {}
    }
    Ok(())
}
type PoolIdentity = (String, String, String, String, i64, Option<String>);
async fn pool(
    env: &str,
    application: &str,
    max: u32,
    inspection: bool,
    lower: &LowerHolds,
) -> Result<PgPool, &'static str> {
    let options: PgConnectOptions = std::env::var(env)
        .expect("explicit isolated role URL")
        .parse()
        .map_err(|error| {
            hold(lower, error);
            "ROLE_URL_FORMAT"
        })?;
    let owner = std::env::var("R5_PAPER_OWNER_USER_ID").expect("real inspection Owner");
    let pool = PgPoolOptions::new()
        .max_connections(max)
        .acquire_timeout(Duration::from_secs(5))
        .after_connect(move |connection, _| {
            let owner = owner.clone();
            Box::pin(async move {
                sqlx::raw_sql("SET statement_timeout='15s'; SET lock_timeout='5s'")
                    .execute(&mut *connection)
                    .await?;
                if inspection {
                    sqlx::query("SELECT set_config('app.actor_user_id',$1,false)")
                        .bind(owner)
                        .execute(&mut *connection)
                        .await?;
                    sqlx::query("SET default_transaction_read_only=on")
                        .execute(&mut *connection)
                        .await?;
                }
                Ok(())
            })
        })
        .connect_with(
            options
                .application_name(application)
                .log_statements(tracing::log::LevelFilter::Off)
                .log_slow_statements(tracing::log::LevelFilter::Off, Duration::from_secs(1)),
        )
        .await
        .map_err(|error| {
            hold(lower, error);
            "ROLE_POOL_CONNECT"
        })?;
    let nonce = std::env::var("R5_INTEG_NONCE").expect("isolated fixture nonce");
    let returned:Result<PoolIdentity,sqlx::Error>=sqlx::query_as(
        "SELECT current_database()::text,current_user::text,session_user::text,current_setting('role'),(SELECT count(*) FROM public.r5_checker_fixture_identity),(SELECT min(nonce) FROM public.r5_checker_fixture_identity)")
        .fetch_one(&pool).await;
    let identity = match &returned {
        Ok(identity) => identity,
        Err(_) => {
            hold(lower, returned);
            pool.close().await;
            return Err("ROLE_IDENTITY_READ");
        }
    };
    let role = if inspection {
        "migration_owner"
    } else {
        "worker"
    };
    let valid = identity.0 == format!("r5_checker_{nonce}")
        && identity.1 == role
        && identity.2 == role
        && identity.3 == "none"
        && identity.4 == 1
        && identity.5.as_deref() == Some(&nonce);
    hold(lower, returned);
    if !valid {
        pool.close().await;
        return Err("ROLE_IDENTITY_MISMATCH");
    }

    Ok(pool)
}
struct NextWork;
impl PaperActionWork<(), OwnerBetaPaperError> for NextWork {
    fn run<'ctx, 'scope, 'attempt>(
        self: Box<Self>,
        context: &'ctx PaperActionContext<'ctx, 'scope, 'attempt>,
    ) -> PaperActionFuture<'ctx, (), OwnerBetaPaperError> {
        Box::pin(async move {
            context
                .revalidate()
                .map_err(|_| OwnerBetaPaperError::Runtime)?;
            eprintln!("R5_ACTION_NEXT_ACTION_ENTERED actual_bounded_slot=true");
            Ok(())
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Root-owned exact libtest ELF/cohort before ENABLE; real protected four-consumer cases, never ordinary/host execution"]
async fn checked_action_real_consumer_fixture() {
    let packet = read_packet().expect("closed pinned private action case packet");
    let observer_lower: LowerHolds = Rc::new(RefCell::new(Vec::new()));
    let worker = pool(
        "R5_PAPER_WORKER_URL",
        &format!("r5-action-queue-{}", packet.nonce),
        3,
        false,
        &observer_lower,
    )
    .await
    .unwrap_or_else(|_| panic!("ISOLATED_WORKER_POOL"));
    let inspection = pool(
        "R5_PAPER_MIGRATION_OWNER_URL",
        &format!("r5-action-observer-{}", packet.nonce),
        2,
        true,
        &observer_lower,
    )
    .await;
    let inspection = match inspection {
        Ok(pool) => pool,
        Err(_) => {
            worker.close().await;
            panic!("ISOLATED_INSPECTION_POOL");
        }
    };
    let queue = JobQueue::new(
        worker.clone(),
        None,
        job_queue::QueueConfig {
            lease: Duration::from_secs(5),
            backoff_base: Duration::from_secs(1),
        },
    );
    let monitor: Arc<AsyncMutex<Option<Arc<ClaimHeartbeat>>>> = Arc::new(AsyncMutex::new(None));
    let acquired: Arc<AsyncMutex<Option<ClaimedJob>>> = Arc::new(AsyncMutex::new(None));
    let original=std::panic::AssertUnwindSafe(async{
        let owner=Uuid::parse_str(&std::env::var("R5_PAPER_OWNER_USER_ID").expect("real Owner ID")).unwrap();
        let account=Uuid::parse_str(&std::env::var("R5_PAPER_ACCOUNT_ID").expect("real account ID")).unwrap();
        let claim=if packet.cases[0].consumer==Consumer::Preview{
            let returned=queue.claim_next_paper_for_owner(&format!("action-{}",packet.nonce),owner).await;
            let claim=match returned { Ok(Some(claim))=>claim, original=>{hold(&observer_lower,original);panic!("REAL_PAPER_CLAIM_UNAVAILABLE");} };
            *acquired.lock().await=Some(claim.clone());
            *monitor.lock().await=Some(Arc::new(ClaimHeartbeat::start(queue.clone(),claim.clone(),Duration::from_secs(1))));Some(claim)
        }else{None};
        let heartbeat=monitor.lock().await.as_ref().cloned();
        let lost=if let Some(monitor)=monitor.lock().await.as_ref(){monitor.lost.clone()}else{watch::channel(false).1};
        let mut observations=Vec::new();
        for case in packet.cases{
            if claim.is_some(){assert!(!*lost.borrow(),"a genuinely lost claim aborts the group before fresh capture");}
            let app=format!("r5-act-{}-{:x}",&packet.nonce[..8],Sha256::digest(format!("{}:{}",case.case_id,case.scope_id).as_bytes()));
            let economic=pool("R5_PAPER_WORKER_URL",&app[..60],1,false,&observer_lower).await.unwrap_or_else(|_|panic!("CASE_ECONOMIC_POOL"));
            let before_owned=Rc::new(economic_rows(&inspection,account,owner).await);hold(&observer_lower,Rc::clone(&before_owned));let before=economic_value(&before_owned);
            let probe=Arc::new(Probe{case:case.clone(),queue:queue.clone(),claim:claim.clone(),monitor:Arc::clone(&monitor),entered:AtomicBool::new(false),consumer_returned:AtomicBool::new(false),sql_entered:AtomicBool::new(false),direct_calendar:AtomicBool::new(false),calendar_checked:AtomicBool::new(false),state:Mutex::new(ProbeState::default())});
            hold(&observer_lower,Arc::clone(&probe));
            probe.mark("CASE_BEGIN_BEFORE_CAPTURE");
            let identity=probe.capture_connection(&economic).await;
            let identity_ok=identity.is_ok();hold(&observer_lower,identity);assert!(identity_ok,"same actual capture connection identity");
            let (sender,receiver)=oneshot::channel();
            let remaining=(case.absolute_deadline-Utc::now()).to_std().expect("case deadline remains").min(ACTION_TIMEOUT);
            let outer_owned=Rc::new(run_bound_paper_action(PaperProcessRole::Worker,remaining,None,RealWork{pool:economic.clone(),queue:queue.clone(),probe:Arc::clone(&probe),claim:claim.clone(),heartbeat:heartbeat.clone(),report:sender}).await);
            hold(&observer_lower,Rc::clone(&outer_owned));let outer=outer_owned.as_ref();
            probe.mark("OUTER_RETURNED");
            let local=tokio::time::timeout(Duration::from_secs(90),receiver).await;
            let connection=tokio::time::timeout(Duration::from_secs(5),economic.acquire()).await;
            let released=match connection {
                Ok(Ok(connection))=>{drop(connection);true},
                Ok(Err(error))=>{hold(&observer_lower,error);false},
                Err(elapsed)=>{hold(&observer_lower,elapsed);false},
            };
            let after_owned=Rc::new(economic_rows(&inspection,account,owner).await);hold(&observer_lower,Rc::clone(&after_owned));
            economic.close().await;
            let report=local.expect("actual callback completion, not outer timeout").expect("scope report");
            let after=economic_value(&after_owned);
            if case.expected_gate!=ExpectedGate::Committed{
                assert!(before.immutable==after.immutable,"every economic row must be unchanged on refusal; row values remain private");
            }
            let lifecycle_changed=before.allowed_lifecycle!=after.allowed_lifecycle;
            eprintln!("R5_ACTION_SCOPE {}",json!({"report":report,"economic_before_sha256":before.hash,"economic_after_sha256":after.hash,"allowed_lifecycle_changed":lifecycle_changed,"worker_connection_released":released,"outer_kind":outer.as_ref().err().map(|e|format!("{:?}",e.kind())),"native_acceptance":false,"Root_independent_wait_mutation_economic_witness_required":true}));
            assert!(released&&report.local_gate_matched&&(case.expected_gate!=ExpectedGate::Committed||outer.is_ok()),"intended entered gate and outer completion must match; early generic Capture is not a pass");
            // Root releases this derived gate after restoring the selected input.
            // No command or root path is accepted from the case packet.
            let returned=queue.begin().await;
            let mut restore=match returned { Ok(tx)=>tx,Err(error)=>{hold(&observer_lower,error);panic!("RESTORE_RENDEZVOUS_BEGIN");} };
            let restored=async {
                set_paper_transaction_timeouts(&mut restore).await?;
                sqlx::query_scalar::<_,i32>("SELECT 1 FROM pg_advisory_xact_lock($1,$2)").bind(latch_key(&case,"restore")).bind(0_i32).fetch_one(&mut *restore).await
            }.await;
            let retired=restore.rollback().await;
            let restored_ok=restored.is_ok()&&retired.is_ok();hold(&observer_lower,(restored,retired));
            assert!(restored_ok,"external restore rendezvous");
            let remaining=(case.absolute_deadline-Utc::now()).to_std().expect("same absolute deadline must still permit the next slot witness").min(ACTION_TIMEOUT);
            let next=run_bound_paper_action(PaperProcessRole::Worker,remaining,None,NextWork).await;
            let next_ok=next.is_ok();hold(&observer_lower,next);
            assert!(next_ok,"following real action must acquire/release its slot after restoration");
            observations.push((outer_owned,report,before_owned,after_owned,probe));
        }
        if let Some(claim)=&claim{
            let state=queue.get_by_id(claim.job.id).await.expect("same real final claim");
            assert!(matches!(state.status,job_queue::types::JobStatus::Succeeded|job_queue::types::JobStatus::Queued|job_queue::types::JobStatus::Failed),"no abandoned live claim");
        }
        observations
    }).catch_unwind().await;
    if let Some(heartbeat) = monitor.lock().await.take() {
        heartbeat.stop().await;
    }
    if original.is_err()
        && let Some(claim) = acquired.lock().await.as_ref()
    {
        let cleanup = retire_failed_claim(&queue, claim).await;
        hold(&observer_lower, cleanup);
    }
    drop(queue);
    worker.close().await;
    inspection.close().await;
    if let Err(payload) = original {
        std::panic::resume_unwind(payload);
    }
}

#[test]
fn case_packet_rejects_wrong_gate_unknown_fields_and_oversized_groups() {
    let now = Utc::now();
    let today = now
        .with_timezone(&FixedOffset::east_opt(9 * 60 * 60).unwrap())
        .date_naive();
    let mut case = CaseSpec {
        case_id: "c1".into(),
        scope_id: "s1".into(),
        process_date: today,
        target_effective_session: today,
        target_id: None,
        t_run: None,
        consumer: Consumer::Queue,
        variant: Variant::RetainedChange,
        expected_gate: ExpectedGate::Committed,
        absolute_deadline: now + chrono::Duration::seconds(90),
    };
    let packet = CaseBatch {
        schema: "r5-action-real-consumer-v1".into(),
        nonce: "a".repeat(32),
        cases: vec![case.clone()],
    };
    assert!(validate_packet(&packet, &packet.nonce, now).is_err());
    case.expected_gate = ExpectedGate::EvidenceTampered;
    let accepted = CaseBatch {
        cases: vec![case.clone()],
        schema: packet.schema.clone(),
        nonce: packet.nonce.clone(),
    };
    assert!(validate_packet(&accepted, &accepted.nonce, now).is_ok());
    let mut valid = serde_json::to_value(&case).unwrap();
    assert!(serde_json::from_value::<CaseSpec>(valid.clone()).is_ok());
    valid["arbitrary_sql"] = json!("SELECT 1");
    assert!(serde_json::from_value::<CaseSpec>(valid).is_err());
    let valid = CaseBatch {
        cases: vec![case; 7],
        ..packet
    };
    assert!(validate_packet(&valid, &valid.nonce, now).is_err());
}

// Explicit fallback: real checked Raw originals -> real alternate projection ->
// unchanged shared validator. This is distinctly NOT whole-consumer coverage.
fn direct_calendar_refusal(
    action: &mut CheckedPaperAction<'_>,
    direct: &DirectCalendar<'_, '_, '_>,
    variant: Variant,
    holds: &LowerHolds,
) -> Result<(), OwnerBetaPaperError> {
    use market_data::{
        ResponseKind, SourceFileReference,
        checked_close::Attempt,
        daily_price_only::{
            calendar::project_calendar_candidates_checked,
            source::{ApprovedRawScope, CommittedRawReader, ScanLimitValues, ScanLimits},
            verify::{CheckedRawEvidenceSnapshots, resolve_original_checked},
        },
        raw_custody::RawInvocation,
    };
    action.revalidate_files().map_err(map_action_input_error)?;
    let path = Path::new("/run/owner-beta-daily/config.json");
    if direct.context.binding()["daily_config_file"] != path.to_str().unwrap() {
        return Err(OwnerBetaPaperError::Snapshot);
    }
    let mut config_file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| OwnerBetaPaperError::Snapshot)?;
    let before = config_file
        .metadata()
        .map_err(|_| OwnerBetaPaperError::Snapshot)?;
    if !before.is_file()
        || before.nlink() != 1
        || before.mode() & 0o022 != 0
        || before.len() > 16384
    {
        return Err(OwnerBetaPaperError::Snapshot);
    }
    let mut bytes = Vec::new();
    (&mut config_file)
        .take(16385)
        .read_to_end(&mut bytes)
        .map_err(|_| OwnerBetaPaperError::Snapshot)?;
    let after = config_file
        .metadata()
        .map_err(|_| OwnerBetaPaperError::Snapshot)?;
    let named = std::fs::symlink_metadata(path).map_err(|_| OwnerBetaPaperError::Snapshot)?;
    if bytes.len() > 16384
        || stamp(&before) != stamp(&after)
        || stamp(&after) != stamp(&named)
        || prefixed_sha256(&bytes)
            != direct.context.binding()["daily_config_sha256"]
                .as_str()
                .ok_or(OwnerBetaPaperError::Snapshot)?
    {
        return Err(OwnerBetaPaperError::Snapshot);
    }
    // Settings are selectors from the identical bytes already checked by the
    // enclosing real action/config scope, never a config/approval constructor.
    let settings: Value =
        serde_json::from_slice(&bytes).map_err(|_| OwnerBetaPaperError::Snapshot)?;
    let size = |key: &str| -> Result<usize, OwnerBetaPaperError> {
        usize::try_from(
            settings["scan"][key]
                .as_u64()
                .ok_or(OwnerBetaPaperError::Snapshot)?,
        )
        .map_err(|_| OwnerBetaPaperError::Snapshot)
    };
    let limits = ScanLimitValues {
        max_manifest_bytes: size("max_manifest_bytes")?,
        max_manifest_records: size("max_manifest_records")?,
        max_batches: size("max_batches")?,
        max_files_per_batch: size("max_files_per_batch")?,
        max_files: size("max_files")?,
        max_batch_metadata_bytes: size("max_batch_metadata_bytes")?,
        max_source_file_bytes: size("max_source_file_bytes")?,
        max_retained_bytes: size("max_retained_bytes")?,
        max_open_descriptors: size("max_open_descriptors")?,
        max_data_root_bytes: size("max_data_root_bytes")?,
        max_data_root_components: size("max_data_root_components")?,
        max_metadata_json_depth: size("max_metadata_json_depth")?,
    };
    let raw_uid = u32::try_from(
        settings["raw_owner_uid"]
            .as_u64()
            .ok_or(OwnerBetaPaperError::Snapshot)?,
    )
    .map_err(|_| OwnerBetaPaperError::Snapshot)?;
    if direct.context.binding()["raw_owner_uid"].as_u64() != Some(u64::from(raw_uid)) {
        return Err(OwnerBetaPaperError::Snapshot);
    }
    let entitlement = settings["entitlement_reference"]
        .as_str()
        .ok_or(OwnerBetaPaperError::Snapshot)?;
    let owner = Uuid::parse_str(
        direct.context.binding()["cohort"]["owner_user_id"]
            .as_str()
            .ok_or(OwnerBetaPaperError::Snapshot)?,
    )
    .map_err(|_| OwnerBetaPaperError::Snapshot)?;
    let policy = ContentHash::parse(
        direct.context.binding()["cohort"]["policy_sha256"]
            .as_str()
            .ok_or(OwnerBetaPaperError::Snapshot)?,
    )
    .map_err(|_| OwnerBetaPaperError::Snapshot)?;
    let alternate_owner = if variant == Variant::CalendarOwnerDirect {
        let value = owner.as_u128() ^ 1;
        Uuid::from_u128(if value == 0 {
            owner.as_u128() ^ 2
        } else {
            value
        })
    } else {
        owner
    };
    let alternate_policy = if variant == Variant::CalendarPolicyDirect {
        ContentHash::parse(&prefixed_sha256(
            &serde_json::to_vec(
                &json!({"schema":"r5-calendar-policy-negative-v1","base_policy":policy.as_str()}),
            )
            .unwrap(),
        ))
        .map_err(|_| OwnerBetaPaperError::Snapshot)?
    } else {
        policy
    };
    let attempt = Attempt::default();
    let captured = attempt.clone();
    let config_owner = attempt.adopt(config_file, market_data::checked_close::Role::Source);
    let invocation = RawInvocation::new(&attempt);
    let mut raw = Vec::new();
    let mut clean = true;
    let original = (|| {
        for provider in ["kis", "kis-calendar"] {
            let scope = ApprovedRawScope::new(provider, "kr", raw_uid).map_err(|error| {
                hold(holds, error);
                OwnerBetaPaperError::Snapshot
            })?;
            let scan = ScanLimits::new(limits).map_err(|error| {
                hold(holds, error);
                OwnerBetaPaperError::Snapshot
            })?;
            match CommittedRawReader::new(Path::new("/data")).snapshot_checked_in(
                &invocation,
                &attempt,
                &captured,
                scope,
                scan,
            ) {
                Ok(snapshot) => raw.push(snapshot),
                Err(error) => {
                    hold(holds, error);
                    return Err(OwnerBetaPaperError::Snapshot);
                }
            }
        }
        let references_raw = raw.iter().collect::<Vec<_>>();
        let snapshots = CheckedRawEvidenceSnapshots::new_in(
            &invocation,
            &attempt,
            &captured,
            &references_raw,
            entitlement,
            domain::UtcTimestamp::from_datetime(Utc::now()),
        )
        .map_err(|error| {
            hold(holds, error);
            OwnerBetaPaperError::Snapshot
        })?;
        let mut originals = Vec::new();
        let inner = (|| {
            let mut references = Vec::new();
            for provider in ["kis", "kis-calendar"] {
                let (view, local) = snapshots
                    .snapshot_observed_view_in(provider, "kr")
                    .map_err(|error| {
                        hold(holds, error);
                        OwnerBetaPaperError::Snapshot
                    })?;
                let local_clean = local.is_clean();
                hold(holds, local);
                if !local_clean {
                    return Err(OwnerBetaPaperError::Runtime);
                }
                let scope = view.scope().ok_or(OwnerBetaPaperError::Snapshot)?;
                for batch in view.batches() {
                    let matching = snapshots.matches_entitlement(&batch);
                    if !matching {
                        continue;
                    }
                    for file in batch
                        .files()
                        .filter(|file| file.kind() == ResponseKind::Calendar)
                    {
                        references.push(SourceFileReference {
                            provider: scope.provider().to_owned(),
                            market: scope.market().to_owned(),
                            batch_id: file.batch().batch_id(),
                            file_name: file.file_name().to_owned(),
                            content_hash: file.content_hash().clone(),
                            retrieved_at: file.batch().retrieved_at(),
                        });
                    }
                }
            }
            for reference in &references {
                match resolve_original_checked(&snapshots, reference) {
                    Ok(original) => originals.push(original),
                    Err(error) => {
                        hold(holds, error);
                        return Err(OwnerBetaPaperError::Snapshot);
                    }
                }
            }
            let candidates =
                project_calendar_candidates_checked(&originals, alternate_owner, &alternate_policy)
                    .map_err(|error| {
                        hold(holds, error);
                        OwnerBetaPaperError::Snapshot
                    })?;
            let validation = match candidates.view_in() {
                Err(error) => {
                    hold(holds, error);
                    Err(OwnerBetaPaperError::Snapshot)
                }
                Ok(view) => {
                    let proved = view.prove_range(direct.signal, direct.effective);
                    let result = match proved {
                        Err(error) => {
                            hold(holds, error);
                            Err(OwnerBetaPaperError::Snapshot)
                        }
                        Ok(proof) => {
                            let original = validate_calendar_link(
                                direct.signal,
                                direct.effective,
                                &direct.evidence,
                                &direct.link,
                                &proof,
                                direct.context,
                                None,
                            );
                            hold(holds, proof);
                            if matches!(original, Err(OwnerBetaPaperError::Capture)) {
                                Ok(())
                            } else {
                                Err(OwnerBetaPaperError::Snapshot)
                            }
                        }
                    };
                    for local in view.into_diagnostics() {
                        clean &= local.is_clean();
                        hold(holds, local);
                    }
                    result
                }
            };
            let completion = candidates.finish_local();
            clean &= completion.is_clean();
            hold(holds, completion);
            validation
        })();
        for original in originals {
            hold(holds, original.retain_diagnostics());
        }
        let local = snapshots.into_diagnostics();
        clean &= local.is_clean();
        hold(holds, local);
        inner
    })();
    // Actual calendar then KIS resource order, including partial acquisition.
    while let Some(snapshot) = raw.pop() {
        let completion = snapshot.finish_local();
        clean &= completion.is_clean();
        hold(holds, completion);
    }
    clean &= config_owner.finish();
    let expected = invocation.settle_local_in(&attempt, Ok::<(), std::convert::Infallible>(()));
    clean &= expected.is_ok();
    hold(holds, expected);
    let actual = invocation.settle_local_in(&captured, Ok::<(), std::convert::Infallible>(()));
    clean &= actual.is_ok();
    hold(holds, actual);
    let finished = attempt
        .clone()
        .finish(Ok::<(), std::convert::Infallible>(()));
    clean &= finished.is_ok();
    hold(holds, finished);
    // The extra config FD was adopted and actually closed once; all native
    // reports/results remain in lower until the outer callback's local mapping.
    action.revalidate_files().map_err(map_action_input_error)?;
    if !clean {
        return Err(OwnerBetaPaperError::Runtime);
    }
    original
}

// Failure cleanup never invents terminal success or revives a claim. Stop has
// already been awaited; only actual expiry and the canonical sweep may retire
// this attempt. Root remains owner of overdue/native process cleanup.
async fn retire_failed_claim(
    queue: &JobQueue,
    claim: &ClaimedJob,
) -> Result<(Vec<Job>, Option<SweepReport>), job_queue::QueueError> {
    let mut samples = Vec::new();
    loop {
        let job = queue.get_by_id(claim.job.id).await?;
        if job.status != job_queue::types::JobStatus::Running
            || job.attempt_count != claim.attempt.attempt_no
        {
            samples.push(job);
            return Ok((samples, None));
        }
        let expired = match job.locked_at {
            Some(at) => {
                let now: chrono::DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
                    .fetch_one(&queue.pool())
                    .await?;
                now >= at + chrono::Duration::seconds(5)
            }
            None => true,
        };
        samples.push(job);
        if expired {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let swept = queue.sweep_paper_for_owner(claim.job.owner_user_id).await?;
    let job = queue.get_by_id(claim.job.id).await?;
    samples.push(job);
    Ok((samples, Some(swept)))
}

// These cases require the existing Root packet and actual mounted originals.
// The packet describes selectors; it is not staged here and cannot prove origin.
async fn real_retained_recovery_cases(step: &str, refuse: bool) {
    let packet =
        read_packet().expect("existing protected case packet plus real retained grant/originals");
    let holds: LowerHolds = Rc::new(RefCell::new(Vec::new()));
    let worker = pool(
        "R5_PAPER_WORKER_URL",
        "r5-recovery-worker",
        1,
        false,
        &holds,
    )
    .await
    .unwrap();
    let inspection = pool(
        "R5_PAPER_MIGRATION_OWNER_URL",
        "r5-recovery-observer",
        1,
        true,
        &holds,
    )
    .await
    .unwrap();
    // Only the observer owns this pool; RealWork receives the worker pool only.
    let queue = JobQueue::new(worker.clone(), None, job_queue::QueueConfig::default());
    let owner = Uuid::parse_str(&std::env::var("R5_PAPER_OWNER_USER_ID").unwrap()).unwrap();
    let account = Uuid::parse_str(&std::env::var("R5_PAPER_ACCOUNT_ID").unwrap()).unwrap();
    let original=std::panic::AssertUnwindSafe(async {
        if !refuse {
            let expected=if step=="SETTLE_THEN_VALUE" {vec![Consumer::Settlement,Consumer::Valuation]} else {vec![Consumer::Valuation]};
            assert_eq!(packet.cases.iter().map(|case|case.consumer).collect::<Vec<_>>(),expected);
        }
        let mut original_settlement=None;
        let mut selected_target=None;
        for case in packet.cases {
            assert!(matches!(case.consumer,Consumer::Settlement|Consumer::Valuation));
            assert_eq!(case.expected_gate==ExpectedGate::Committed,!refuse);
            let target=case.target_id.expect("actual retained target selected by Root");
            assert!(selected_target.is_none_or(|selected|selected==target)); selected_target=Some(target);
            let rows:(String,i64,i64,i64,i64,bool,Option<String>)=sqlx::query_as(
                "SELECT g.step,l.control_epoch,c.control_epoch,l.config_epoch,(c.daily_cohort->>'config_epoch')::bigint,d.first_auto_session>s.effective_session,s.settlement_sha256 FROM owner_beta_paper_recovery_grants g JOIN owner_beta_paper_sessions s ON s.pending_target_id=g.pending_target_id JOIN owner_beta_paper_lineage l ON l.id=g.lineage_id JOIN owner_beta_paper_control c ON c.account_id=g.account_id JOIN owner_beta_daily_control d ON d.owner_user_id=c.owner_user_id WHERE g.pending_target_id=$1 AND g.active_epoch=c.control_epoch AND g.revoked_at IS NULL AND g.valuation_receipt_sha256 IS NULL",
            ).bind(target).fetch_one(&inspection).await.expect("actual retained grant rows; no fabricated identities");
            assert_eq!(rows.0,step);assert!(rows.1<rows.2);assert_ne!(rows.3,rows.4);assert!(rows.5);
            if original_settlement.is_none(){original_settlement=rows.6.clone();}
            let before=Rc::new(economic_rows(&inspection,account,owner).await);hold(&holds,Rc::clone(&before));
            let monitor=Arc::new(AsyncMutex::new(None));
            let probe=Arc::new(Probe{case:case.clone(),queue:queue.clone(),claim:None,monitor,
                entered:AtomicBool::new(false),consumer_returned:AtomicBool::new(false),sql_entered:AtomicBool::new(false),
                direct_calendar:AtomicBool::new(false),calendar_checked:AtomicBool::new(false),state:Mutex::new(ProbeState::default())});
            let (sender,receiver)=oneshot::channel();
            let remaining=(case.absolute_deadline-Utc::now()).to_std().expect("original absolute case deadline").min(ACTION_TIMEOUT);
            let result=Rc::new(run_bound_paper_action(PaperProcessRole::Worker,remaining,None,
                RealWork{pool:worker.clone(),queue:queue.clone(),probe:Arc::clone(&probe),claim:None,heartbeat:None,report:sender}).await);
            hold(&holds,Rc::clone(&result));
            let report=receiver.await.expect("actual completed consumer callback");
            let after=Rc::new(economic_rows(&inspection,account,owner).await);hold(&holds,Rc::clone(&after));
            if refuse {
                assert!(result.is_err() || !report.success);
                assert!(report.local_gate_matched,"selected genuine failure must be reached, not an unrelated early refusal");
                assert_eq!(economic_value(&before).immutable,economic_value(&after).immutable);
            } else {
                assert!(result.is_ok() && report.success && report.committed && report.commit_started);
                assert!(probe.entered.load(Ordering::Acquire) && probe.sql_entered.load(Ordering::Acquire));
                assert!(probe.calendar_checked.load(Ordering::Acquire));
            }
        }
        if !refuse {
            let target=selected_target.unwrap();
            let (settlement,valuation,outbox,equity):(String,String,i64,i64)=sqlx::query_as(
                "SELECT s.settlement_sha256,s.valuation_sha256,(SELECT count(*) FROM paper_settlement_outbox o WHERE o.pending_target_id=s.pending_target_id),(SELECT count(*) FROM daily_equity e WHERE e.account_id=s.account_id AND e.trading_date=s.effective_session) FROM owner_beta_paper_sessions s WHERE s.pending_target_id=$1 AND s.state='VALUED'",
            ).bind(target).fetch_one(&inspection).await.unwrap();
            assert!(!valuation.is_empty());assert_eq!((outbox,equity),(1,1));
            if step=="VALUE_ONLY" {assert_eq!(Some(settlement),original_settlement,"no second settlement");}
        }
    }).catch_unwind().await;
    drop(queue);
    worker.close().await;
    inspection.close().await;
    if let Err(payload) = original {
        std::panic::resume_unwind(payload);
    }
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "NOT_STAGED: genuine retained T/run/E Source/Raw/sealed Daily + current protected ELF/Current + committed SETTLE_THEN_VALUE grant and existing Root case packet"]
async fn checked_recovery_real_settlement_then_valuation_consumer() {
    real_retained_recovery_cases("SETTLE_THEN_VALUE", false).await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "NOT_STAGED: genuine retained settled T/run/E Source and current protected ELF/Current + committed VALUE_ONLY grant and existing Root case packet"]
async fn checked_recovery_real_value_only_preserves_original_settlement() {
    real_retained_recovery_cases("VALUE_ONLY", false).await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "NOT_STAGED: genuine retained Source plus Root independently witnessed input/FD/rights/grant mutation, cancellation/readback barriers in existing case packet"]
async fn checked_recovery_real_refusal_rolls_back_economics() {
    real_retained_recovery_cases("VALUE_ONLY", true).await;
}

// Private source fixture only. The existing packet is untrusted selection and
// CLASSIFICATION remains SYNTHETIC_TEST_AUTHORITY_NOT_ORIGIN. This module adds
// no serialized field/enum, public protocol, credential surface or production hook.
mod recovery_commit_ack_loss {
    use super::*;
    use std::{
        net::{IpAddr, Ipv4Addr, SocketAddr},
        sync::atomic::{AtomicI32, AtomicUsize},
    };
    use tokio::{
        net::{TcpListener, TcpStream},
        sync::Notify,
        time::{Instant as AsyncInstant, timeout_at},
    };

    const FRAME_CAP: usize = 1_048_576;
    const STARTUP_CAP: usize = 16_384;
    const STREAM_CAP: usize = 64 * 1_048_576;
    const FRAME_COUNT_CAP: usize = 16_384;
    const TRAILING_CAP: usize = 16;
    const CLEANUP_RESERVE: Duration = Duration::from_secs(2);

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum FaultError {
        Selection,
        ConnectionOptions,
        EncryptedTransport,
        Endpoint,
        Database,
        Deadline,
        Io,
        EarlyEof,
        FrameLength,
        FrameTag,
        StreamLimit,
        Startup,
        BackendRejected,
        BackendIdentity,
        TransactionBoundary,
        PostCommitFrame,
        MissingBoundary,
        ClientOutcome,
        IndependentEffect,
        Cleanup,
        Panic,
    }
    type Result<T> = std::result::Result<T, FaultError>;

    // Only transient I/O storage. No Debug/Serialize, persistence or diagnostics.
    // Authentication, backend key and SQL/result bytes are forwarded unchanged,
    // then discarded; none becomes a retained observation or log.
    struct Buffer(Vec<u8>);
    impl Drop for Buffer {
        fn drop(&mut self) {
            self.0.fill(0);
        }
    }
    struct Frame {
        tag: u8,
        length: [u8; 4],
        body: Buffer,
    }
    #[derive(Default)]
    struct WireBudget {
        bytes: usize,
        frames: usize,
    }
    impl WireBudget {
        fn charge(&mut self, bytes: usize) -> Result<()> {
            self.bytes = self
                .bytes
                .checked_add(bytes)
                .ok_or(FaultError::StreamLimit)?;
            self.frames += 1;
            if self.bytes > STREAM_CAP || self.frames > FRAME_COUNT_CAP {
                return Err(FaultError::StreamLimit);
            }
            Ok(())
        }
    }
    fn body_length(length: [u8; 4], cap: usize) -> Result<usize> {
        let length = u32::from_be_bytes(length) as usize;
        if !(4..=cap).contains(&length) {
            return Err(FaultError::FrameLength);
        }
        Ok(length - 4)
    }
    fn startup_plain(body: &[u8]) -> Result<()> {
        if body.len() < 4 {
            return Err(FaultError::Startup);
        }
        let code = u32::from_be_bytes(body[..4].try_into().map_err(|_| FaultError::Startup)?);
        // Never answer SSL/GSS negotiation with N or change TLS settings.
        if matches!(code, 80_877_103 | 80_877_104) {
            return Err(FaultError::EncryptedTransport);
        }
        if code != 196_608 || body.len() < 9 || body.last() != Some(&0) {
            return Err(FaultError::Startup);
        }
        let mut fields = body[4..body.len() - 1].split(|byte| *byte == 0);
        loop {
            let key = fields.next().ok_or(FaultError::Startup)?;
            if key.is_empty() {
                if fields.next().is_none() {
                    break;
                }
                return Err(FaultError::Startup);
            }
            fields.next().ok_or(FaultError::Startup)?;
        }
        Ok(())
    }
    async fn read_exact(socket: &TcpStream, bytes: &mut [u8]) -> Result<()> {
        let mut used = 0;
        while used < bytes.len() {
            socket.readable().await.map_err(|_| FaultError::Io)?;
            match socket.try_read(&mut bytes[used..]) {
                Ok(0) => return Err(FaultError::EarlyEof),
                Ok(n) => used += n,
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) => {}
                Err(_) => return Err(FaultError::Io),
            }
        }
        Ok(())
    }
    async fn write_all(socket: &TcpStream, bytes: &[u8]) -> Result<()> {
        let mut used = 0;
        while used < bytes.len() {
            socket.writable().await.map_err(|_| FaultError::Io)?;
            match socket.try_write(&bytes[used..]) {
                Ok(0) => return Err(FaultError::Io),
                Ok(n) => used += n,
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) => {}
                Err(_) => return Err(FaultError::Io),
            }
        }
        Ok(())
    }
    async fn read_frame(
        socket: &TcpStream,
        budget: &mut WireBudget,
        backend: bool,
    ) -> Result<Frame> {
        let mut header = [0_u8; 5];
        read_exact(socket, &mut header).await?;
        let tag = header[0];
        let allowed = if backend {
            b"RSKZTDCEN123nstIAV".as_slice()
        } else {
            b"pQPBDEHSCX".as_slice()
        };
        if !allowed.contains(&tag) {
            return Err(FaultError::FrameTag);
        }
        let length: [u8; 4] = header[1..]
            .try_into()
            .map_err(|_| FaultError::FrameLength)?;
        let size = body_length(length, FRAME_CAP)?;
        budget.charge(size + 5)?;
        let mut body = Buffer(vec![0; size]);
        read_exact(socket, &mut body.0).await?;
        Ok(Frame { tag, length, body })
    }
    async fn forward(socket: &TcpStream, frame: &Frame) -> Result<()> {
        write_all(socket, &[frame.tag]).await?;
        write_all(socket, &frame.length).await?;
        write_all(socket, &frame.body.0).await
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Forward {
        Pass,
        Withhold,
        LostAcknowledgment,
    }
    #[derive(Default)]
    struct Backend {
        authenticated: bool,
        pid: i32,
        ready: u8,
        withheld_commit: bool,
        trailing: usize,
    }
    impl Backend {
        // This classifier is not Origin. The genuine test calls it only on
        // frames read directly from the configured, role-checked PG backend.
        fn inspect(&mut self, tag: u8, body: &[u8], armed_pid: i32) -> Result<Forward> {
            if matches!(tag, b'1' | b'2' | b'3' | b'n' | b'I' | b's') && !body.is_empty() {
                return Err(FaultError::FrameLength);
            }
            if tag == b'C'
                && (body.len() < 2
                    || body.last() != Some(&0)
                    || body[..body.len() - 1].contains(&0)
                    || !body[..body.len() - 1].is_ascii())
            {
                return Err(FaultError::FrameLength);
            }
            if tag == b'N' {
                // Notice fields are one tag + a C string, terminated by zero.
                // Their contents remain opaque and are never diagnostic output.
                let mut rest = body;
                loop {
                    let Some((&field, tail)) = rest.split_first() else {
                        return Err(FaultError::FrameLength);
                    };
                    if field == 0 {
                        if !tail.is_empty() {
                            return Err(FaultError::FrameLength);
                        }
                        break;
                    }
                    let end = tail
                        .iter()
                        .position(|byte| *byte == 0)
                        .ok_or(FaultError::FrameLength)?;
                    rest = &tail[end + 1..];
                }
            }
            if tag == b'S' {
                let mut fields = body.split(|byte| *byte == 0);
                if fields.next().is_none_or(|key| key.is_empty())
                    || fields.next().is_none()
                    || fields.next() != Some([].as_slice())
                    || fields.next().is_some()
                {
                    return Err(FaultError::FrameLength);
                }
            }
            if self.withheld_commit {
                self.trailing += 1;
                if self.trailing > TRAILING_CAP {
                    return Err(FaultError::PostCommitFrame);
                }
                return match (tag, body) {
                    (b'Z', b"I") => Ok(Forward::LostAcknowledgment),
                    (b'N' | b'S', _) => Ok(Forward::Withhold),
                    _ => Err(FaultError::PostCommitFrame),
                };
            }
            match tag {
                b'R' => {
                    if self.authenticated || body.len() < 4 {
                        return Err(FaultError::BackendIdentity);
                    }
                    let code = u32::from_be_bytes(
                        body[..4]
                            .try_into()
                            .map_err(|_| FaultError::BackendIdentity)?,
                    );
                    if !matches!(code, 0 | 3 | 5 | 10 | 11 | 12) {
                        return Err(FaultError::BackendIdentity);
                    }
                    if code == 0 {
                        if body.len() != 4 {
                            return Err(FaultError::BackendIdentity);
                        }
                        self.authenticated = true;
                    }
                }
                b'K' => {
                    if !self.authenticated || self.pid != 0 || body.len() != 8 {
                        return Err(FaultError::BackendIdentity);
                    }
                    self.pid = i32::from_be_bytes(
                        body[..4]
                            .try_into()
                            .map_err(|_| FaultError::BackendIdentity)?,
                    );
                    // The secret half of BackendKeyData is never retained.
                    if self.pid <= 0 {
                        return Err(FaultError::BackendIdentity);
                    }
                }
                b'Z' => {
                    if !self.authenticated
                        || self.pid <= 0
                        || body.len() != 1
                        || !b"ITE".contains(&body[0])
                    {
                        return Err(FaultError::TransactionBoundary);
                    }
                    self.ready = body[0];
                }
                b'E' => return Err(FaultError::BackendRejected),
                b'C' if armed_pid != 0 && body == b"ROLLBACK\0" => {
                    return Err(FaultError::TransactionBoundary);
                }
                b'C' if armed_pid != 0 && body == b"COMMIT\0" => {
                    if !self.authenticated || self.pid != armed_pid || self.ready != b'T' {
                        return Err(FaultError::TransactionBoundary);
                    }
                    self.withheld_commit = true;
                    return Ok(Forward::Withhold);
                }
                _ => {}
            }
            Ok(Forward::Pass)
        }
    }

    #[derive(Clone, Copy)]
    struct ClientObservation {
        first: Option<PaperInputActionError>,
        commit_started: bool,
        committed: bool,
        original_error: bool,
        retained: usize,
    }
    pub(super) struct Fault {
        backend_pid: AtomicI32,
        armed_pid: AtomicI32,
        arms: AtomicUsize,
        accepts: AtomicUsize,
        live_sockets: AtomicUsize,
        live_listeners: AtomicUsize,
        lost_ack: AtomicBool,
        callbacks: AtomicUsize,
        observation: Mutex<Option<ClientObservation>>,
        observed: Notify,
        release: Notify,
        socket_changed: Notify,
        stop_at: AsyncInstant,
    }
    impl Fault {
        fn new(stop_at: AsyncInstant) -> Self {
            Self {
                backend_pid: AtomicI32::new(0),
                armed_pid: AtomicI32::new(0),
                arms: AtomicUsize::new(0),
                accepts: AtomicUsize::new(0),
                live_sockets: AtomicUsize::new(0),
                live_listeners: AtomicUsize::new(0),
                lost_ack: AtomicBool::new(false),
                callbacks: AtomicUsize::new(0),
                observation: Mutex::new(None),
                observed: Notify::new(),
                release: Notify::new(),
                socket_changed: Notify::new(),
                stop_at,
            }
        }
        pub(super) fn arm(&self, actual_backend: i32) -> Result<()> {
            if actual_backend <= 0
                || actual_backend != self.backend_pid.load(Ordering::Acquire)
                || self.arms.fetch_add(1, Ordering::AcqRel) != 0
                || self
                    .armed_pid
                    .compare_exchange(0, actual_backend, Ordering::AcqRel, Ordering::Acquire)
                    .is_err()
            {
                return Err(FaultError::BackendIdentity);
            }
            Ok(())
        }
        pub(super) async fn hold_failure(
            &self,
            held: &HeldScope,
            committed: bool,
            started: bool,
        ) -> std::result::Result<(), OwnerBetaPaperError> {
            let observation = match held {
                HeldScope::Flag(Err(f)) => ClientObservation {
                    first: f.first_error(),
                    commit_started: f.commit_started(),
                    committed: f.committed(),
                    original_error: f.original().is_some_and(|result| result.is_err()),
                    retained: f.retained_outcome_count(),
                },
                _ => ClientObservation {
                    first: None,
                    commit_started: started,
                    committed,
                    original_error: false,
                    retained: 0,
                },
            };
            self.callbacks.fetch_add(1, Ordering::AcqRel);
            *self
                .observation
                .lock()
                .map_err(|_| OwnerBetaPaperError::Runtime)? = Some(observation);
            self.observed.notify_one();
            timeout_at(self.stop_at, self.release.notified())
                .await
                .map_err(|_| OwnerBetaPaperError::Runtime)?;
            // held is still borrowed here: original failure/SQL outcomes have
            // remained owned throughout independent inspection.
            let _ = held;
            Ok(())
        }
    }

    struct Socket {
        inner: Option<TcpStream>,
        fault: Arc<Fault>,
    }
    impl Socket {
        fn new(inner: TcpStream, fault: &Arc<Fault>) -> Self {
            fault.live_sockets.fetch_add(1, Ordering::AcqRel);
            fault.socket_changed.notify_one();
            Self {
                inner: Some(inner),
                fault: Arc::clone(fault),
            }
        }
        fn get(&self) -> &TcpStream {
            self.inner.as_ref().expect("owned socket")
        }
    }
    impl Drop for Socket {
        fn drop(&mut self) {
            drop(self.inner.take()); // close the exact descriptor before accounting
            self.fault.live_sockets.fetch_sub(1, Ordering::AcqRel);
            self.fault.socket_changed.notify_one();
        }
    }
    struct Listener {
        inner: Option<TcpListener>,
        fault: Arc<Fault>,
    }
    impl Listener {
        fn new(inner: TcpListener, fault: &Arc<Fault>) -> Self {
            fault.live_listeners.fetch_add(1, Ordering::AcqRel);
            Self {
                inner: Some(inner),
                fault: Arc::clone(fault),
            }
        }
    }
    impl Drop for Listener {
        fn drop(&mut self) {
            drop(self.inner.take());
            self.fault.live_listeners.fetch_sub(1, Ordering::AcqRel);
        }
    }
    async fn upstream(client: &TcpStream, backend: &TcpStream) -> Result<()> {
        let mut budget = WireBudget::default();
        loop {
            let frame = read_frame(client, &mut budget, false).await?;
            forward(backend, &frame).await?;
            if frame.tag == b'X' {
                return Err(FaultError::EarlyEof);
            }
        }
    }
    async fn downstream(backend: &TcpStream, client: &TcpStream, fault: &Fault) -> Result<()> {
        let mut budget = WireBudget::default();
        let mut state = Backend::default();
        loop {
            let frame = read_frame(backend, &mut budget, true).await?;
            let decision = state.inspect(
                frame.tag,
                &frame.body.0,
                fault.armed_pid.load(Ordering::Acquire),
            );
            if state.pid > 0 {
                fault.backend_pid.store(state.pid, Ordering::Release);
            }
            match decision {
                Ok(Forward::Pass) => forward(client, &frame).await?,
                Ok(Forward::Withhold) => {}
                Ok(Forward::LostAcknowledgment) => {
                    // Neither CommandComplete(COMMIT) nor ReadyForQuery(I)
                    // was sent to the worker. Dropping this relay closes both
                    // sockets, after the real backend's successful COMMIT.
                    fault.lost_ack.store(true, Ordering::Release);
                    return Ok(());
                }
                Err(error) => {
                    if frame.tag == b'E' && !state.withheld_commit {
                        forward(client, &frame).await?;
                    }
                    return Err(error);
                }
            }
        }
    }
    async fn relay(listener: Listener, endpoint: SocketAddr, fault: Arc<Fault>) -> Result<()> {
        let (stream, peer) = listener
            .inner
            .as_ref()
            .ok_or(FaultError::Cleanup)?
            .accept()
            .await
            .map_err(|_| FaultError::Io)?;
        if !peer.ip().is_loopback() || fault.accepts.fetch_add(1, Ordering::AcqRel) != 0 {
            return Err(FaultError::Endpoint);
        }
        drop(listener); // single connection, never an alternate retry backend
        let client = Socket::new(stream, &fault);
        let server = Socket::new(
            TcpStream::connect(endpoint)
                .await
                .map_err(|_| FaultError::Io)?,
            &fault,
        );
        let mut length = [0; 4];
        read_exact(client.get(), &mut length).await?;
        let size = body_length(length, STARTUP_CAP)?;
        let mut startup = Buffer(vec![0; size]);
        read_exact(client.get(), &mut startup.0).await?;
        startup_plain(&startup.0)?;
        write_all(server.get(), &length).await?;
        write_all(server.get(), &startup.0).await?;
        drop(startup);
        // Both pumps are borrowed futures in this one owner, not spawned tasks.
        // Selecting/canceling one drops the other before the owned sockets.
        tokio::select! {
            result = upstream(client.get(), server.get()) => result,
            result = downstream(server.get(), client.get(), &fault) => result,
            _ = tokio::time::sleep_until(fault.stop_at) => Err(FaultError::Deadline),
        }
    }

    #[derive(Default)]
    struct Pools {
        worker: Option<PgPool>,
        inspection: Option<PgPool>,
    }
    impl Pools {
        async fn close(&mut self) {
            if let Some(pool) = self.worker.take() {
                pool.close().await;
            }
            if let Some(pool) = self.inspection.take() {
                pool.close().await;
            }
        }
    }
    fn connection() -> Result<(PgConnectOptions, SocketAddr)> {
        let options: PgConnectOptions = std::env::var("R5_PAPER_WORKER_URL")
            .map_err(|_| FaultError::ConnectionOptions)?
            .parse()
            .map_err(|_| FaultError::ConnectionOptions)?;
        if options.get_ssl_mode() != sqlx::postgres::PgSslMode::Disable {
            return Err(FaultError::EncryptedTransport);
        }
        let address: IpAddr = options
            .get_host()
            .parse()
            .map_err(|_| FaultError::Endpoint)?;
        if !address.is_loopback() || options.get_socket().is_some() {
            return Err(FaultError::Endpoint);
        }
        let endpoint = SocketAddr::new(address, options.get_port());
        Ok((options, endpoint))
    }
    fn worker_pool(options: PgConnectOptions) -> PgPool {
        PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(5))
            .after_connect(|connection, _| {
                Box::pin(async move {
                    sqlx::raw_sql("SET statement_timeout='15s'; SET lock_timeout='5s'")
                        .execute(connection)
                        .await?;
                    Ok(())
                })
            })
            .connect_lazy_with(
                options
                    .log_statements(tracing::log::LevelFilter::Off)
                    .log_slow_statements(tracing::log::LevelFilter::Off, Duration::from_secs(1)),
            )
    }
    async fn worker_identity(worker: &PgPool, nonce: &str) -> Result<()> {
        let row: PoolIdentity = sqlx::query_as(
            "SELECT current_database()::text,current_user::text,session_user::text,current_setting('role'),(SELECT count(*) FROM public.r5_checker_fixture_identity),(SELECT min(nonce) FROM public.r5_checker_fixture_identity)",
        ).fetch_one(worker).await.map_err(|_| FaultError::Database)?;
        if row.0 != format!("r5_checker_{nonce}")
            || row.1 != "worker"
            || row.2 != "worker"
            || row.3 != "none"
            || row.4 != 1
            || row.5.as_deref() != Some(nonce)
        {
            return Err(FaultError::BackendIdentity);
        }
        Ok(())
    }
    async fn rows(inspection: &PgPool, target: Uuid, grant: Uuid) -> Result<Value> {
        sqlx::query_scalar(r#"SELECT jsonb_build_object(
          'grant',to_jsonb(g),'session',to_jsonb(s),'lineage',to_jsonb(l),'target',to_jsonb(t),
          'dispatch',(SELECT to_jsonb(e) FROM owner_beta_paper_events e WHERE e.account_id=g.account_id AND e.request_key=g.request_key),
          'events',COALESCE((SELECT jsonb_agg(to_jsonb(e) ORDER BY event_seq) FROM owner_beta_paper_session_events e WHERE e.pending_target_id=t.id),'[]'::jsonb),
          'outbox',COALESCE((SELECT jsonb_agg(to_jsonb(o) ORDER BY o.id) FROM paper_settlement_outbox o WHERE o.pending_target_id=t.id),'[]'::jsonb),
          'equity',COALESCE((SELECT jsonb_agg(to_jsonb(e) ORDER BY e.id) FROM daily_equity e WHERE e.account_id=s.account_id AND e.trading_date=s.effective_session),'[]'::jsonb))
          FROM owner_beta_paper_recovery_grants g JOIN owner_beta_paper_sessions s ON s.pending_target_id=g.pending_target_id
          JOIN owner_beta_paper_lineage l ON l.id=g.lineage_id JOIN pending_targets t ON t.id=g.pending_target_id
          WHERE g.grant_id=$1 AND g.pending_target_id=$2"#)
            .bind(grant).bind(target).fetch_one(inspection).await.map_err(|_| FaultError::Database)
    }
    fn array<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>> {
        value[key].as_array().ok_or(FaultError::IndependentEffect)
    }
    fn same_except(a: &Value, b: &Value, keys: &[&str]) -> bool {
        let (Some(mut a), Some(mut b)) = (a.as_object().cloned(), b.as_object().cloned()) else {
            return false;
        };
        for key in keys {
            a.remove(*key);
            b.remove(*key);
        }
        a == b
    }
    fn economic_ok(
        value: &(
            std::result::Result<EconomicRows, sqlx::Error>,
            Option<std::result::Result<(), sqlx::Error>>,
        ),
    ) -> Result<&EconomicRows> {
        match value {
            (Ok(rows), Some(Ok(()))) => Ok(rows),
            _ => Err(FaultError::Database),
        }
    }
    fn append_only_delta(
        before: &EconomicRows,
        after: &EconomicRows,
        table: &str,
        count: usize,
    ) -> Result<()> {
        let old = before.immutable[table]
            .as_array()
            .ok_or(FaultError::IndependentEffect)?;
        let new = after.immutable[table]
            .as_array()
            .ok_or(FaultError::IndependentEffect)?;
        if new.len() != old.len() + count || old.iter().any(|row| !new.contains(row)) {
            return Err(FaultError::IndependentEffect);
        }
        Ok(())
    }
    fn verify_effect(
        before: &Value,
        after: &Value,
        before_all: &EconomicRows,
        after_all: &EconomicRows,
        consumer: Consumer,
    ) -> Result<()> {
        let key = if consumer == Consumer::Settlement {
            "settlement_receipt_sha256"
        } else {
            "valuation_receipt_sha256"
        };
        let receipt = if consumer == Consumer::Settlement {
            "settlement"
        } else {
            "valuation"
        };
        let digest_key = if consumer == Consumer::Settlement {
            "settlement_sha256"
        } else {
            "valuation_sha256"
        };
        if before["lineage"] != after["lineage"]
            || before["dispatch"] != after["dispatch"]
            || !same_except(&before["grant"], &after["grant"], &[key])
            || !before["grant"][key].is_null()
            || !after["grant"][key].as_str().is_some_and(is_prefixed_hash)
            || after["grant"][key] != after["session"][digest_key]
            || after["session"]["receipt_json"][receipt]["calculation_sha256"]
                != after["grant"][key]
        {
            return Err(FaultError::IndependentEffect);
        }
        for key in [
            "pending_target_id",
            "account_id",
            "owner_user_id",
            "effective_session",
            "account_state_version",
            "account_state_sha256",
            "processing_origin",
        ] {
            if before["session"][key].is_null() || before["session"][key] != after["session"][key] {
                return Err(FaultError::IndependentEffect);
            }
        }
        let old_events = array(before, "events")?;
        let events = array(after, "events")?;
        if events.len() != old_events.len() + 1 || events[..old_events.len()] != old_events[..] {
            return Err(FaultError::IndependentEffect);
        }
        for key in ["previews", "lineage"] {
            if before_all.immutable[key] != after_all.immutable[key] {
                return Err(FaultError::IndependentEffect);
            }
        }
        let event = events.last().ok_or(FaultError::IndependentEffect)?;
        if event["payload"][digest_key] != after["grant"][key] {
            return Err(FaultError::IndependentEffect);
        }
        if consumer == Consumer::Settlement {
            let outbox = array(after, "outbox")?;
            if !array(before, "outbox")?.is_empty()
                || outbox.len() != 1
                || before["equity"] != after["equity"]
                || before_all.immutable["equity"] != after_all.immutable["equity"]
                || !same_except(
                    &before["target"],
                    &after["target"],
                    &[
                        "status",
                        "executed_at",
                        "non_execution_reason",
                        "updated_at",
                    ],
                )
                || after["target"]["status"]
                    != (if after["session"]["state"] == "SETTLED_TRADE" {
                        "EXECUTED"
                    } else {
                        "SKIPPED"
                    })
                || !after["session"]["valuation_sha256"].is_null()
                || !matches!(
                    after["session"]["state"].as_str(),
                    Some("SETTLED_TRADE" | "SETTLED_NO_TRADE")
                )
                || event["kind"] != after["session"]["state"]
                || after["session"]["receipt_json"]["outbox_id"] != outbox[0]["id"]
                || event["payload"]["outbox_id"] != outbox[0]["id"]
            {
                return Err(FaultError::IndependentEffect);
            }
            let ids = after["session"]["receipt_json"]["order_fill_ids"]
                .as_array()
                .ok_or(FaultError::IndependentEffect)?;
            append_only_delta(before_all, after_all, "outbox", 1)?;
            append_only_delta(before_all, after_all, "cash_ledger", ids.len())?;
            for (table, id_key) in [("orders", "order_id"), ("fills", "fill_id")] {
                let old = before_all.immutable[table]
                    .as_array()
                    .ok_or(FaultError::IndependentEffect)?;
                let new = after_all.immutable[table]
                    .as_array()
                    .ok_or(FaultError::IndependentEffect)?;
                if new.len() != old.len() + ids.len() || old.iter().any(|row| !new.contains(row)) {
                    return Err(FaultError::IndependentEffect);
                }
                for pair in ids {
                    let id = pair[id_key]
                        .as_str()
                        .and_then(|v| Uuid::parse_str(v).ok())
                        .filter(|id| !id.is_nil())
                        .ok_or(FaultError::IndependentEffect)?;
                    if !new.iter().any(|row| {
                        row.as_str()
                            .and_then(|s| serde_json::from_str::<Value>(s).ok())
                            .is_some_and(|r| r["id"] == id.to_string())
                    }) {
                        return Err(FaultError::IndependentEffect);
                    }
                }
            }
        } else {
            append_only_delta(before_all, after_all, "equity", 1)?;
            let equity = array(after, "equity")?;
            if before["target"] != after["target"]
                || before["outbox"] != after["outbox"]
                || before["session"]["settlement_sha256"] != after["session"]["settlement_sha256"]
                || before["grant"]["original_receipt_sha256"]
                    != after["session"]["settlement_sha256"]
                || !same_except(
                    &before["session"]["receipt_json"],
                    &after["session"]["receipt_json"],
                    &["valuation", "valuation_processed_at"],
                )
                || !array(before, "equity")?.is_empty()
                || equity.len() != 1
                || event["kind"] != "VALUED"
                || after["session"]["state"] != "VALUED"
                || event["payload"]["daily_equity_id"] != equity[0]["id"]
            {
                return Err(FaultError::IndependentEffect);
            }
            for key in [
                "accounts",
                "cash_ledger",
                "positions",
                "orders",
                "fills",
                "outbox",
            ] {
                if before_all.immutable[key] != after_all.immutable[key] {
                    return Err(FaultError::IndependentEffect);
                }
            }
        }
        Ok(())
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Exact original identities and independently retained row owners stay explicit."
    )]
    async fn prove(
        inspection: &PgPool,
        fault: &Fault,
        before: &Value,
        before_all: &EconomicRows,
        target: Uuid,
        grant: Uuid,
        account: Uuid,
        owner: Uuid,
        consumer: Consumer,
    ) -> Result<(Value, EconomicRows)> {
        fault.observed.notified().await;
        let observation = fault
            .observation
            .lock()
            .map_err(|_| FaultError::Panic)?
            .as_ref()
            .copied()
            .ok_or(FaultError::ClientOutcome)?;
        if observation.first != Some(PaperInputActionError::CommitUnknown)
            || !observation.commit_started
            || observation.committed
            || !observation.original_error
            || observation.retained == 0
            || fault.callbacks.load(Ordering::Acquire) != 1
        {
            return Err(FaultError::ClientOutcome);
        }
        let after = rows(inspection, target, grant).await?;
        let after_all = economic_rows(inspection, account, owner).await;
        let actual = economic_ok(&after_all)?;
        verify_effect(before, &after, before_all, actual, consumer)?;
        // Retain full actual rows privately; no body is logged or emitted.
        let (Ok(after_all), Some(Ok(()))) = after_all else {
            return Err(FaultError::Database);
        };
        Ok((after, after_all))
    }

    async fn case(
        pools: &mut Pools,
        packet: CaseBatch,
        consumer: Consumer,
        deadline: AsyncInstant,
    ) -> Result<()> {
        if packet.cases.len() != 1 {
            return Err(FaultError::Selection);
        }
        let case = packet
            .cases
            .into_iter()
            .next()
            .ok_or(FaultError::Selection)?;
        if case.consumer != consumer
            || case.variant != Variant::Clean
            || case.expected_gate != ExpectedGate::Committed
        {
            return Err(FaultError::Selection);
        }
        let target = case.target_id.ok_or(FaultError::Selection)?;
        let run = case.t_run.ok_or(FaultError::Selection)?;
        let owner = std::env::var("R5_PAPER_OWNER_USER_ID")
            .ok()
            .and_then(|v| Uuid::parse_str(&v).ok())
            .ok_or(FaultError::Selection)?;
        let account = std::env::var("R5_PAPER_ACCOUNT_ID")
            .ok()
            .and_then(|v| Uuid::parse_str(&v).ok())
            .ok_or(FaultError::Selection)?;
        let stop_at = deadline
            .checked_sub(CLEANUP_RESERVE)
            .ok_or(FaultError::Deadline)?;
        let lower: LowerHolds = Rc::new(RefCell::new(Vec::new()));
        pools.inspection = Some(
            timeout_at(
                stop_at,
                pool(
                    "R5_PAPER_MIGRATION_OWNER_URL",
                    "r5-ack-observer",
                    1,
                    true,
                    &lower,
                ),
            )
            .await
            .map_err(|_| FaultError::Deadline)?
            .map_err(|_| FaultError::Database)?,
        );
        let inspection = pools
            .inspection
            .as_ref()
            .ok_or(FaultError::Cleanup)?
            .clone();
        let step = if consumer == Consumer::Settlement {
            "SETTLE_THEN_VALUE"
        } else {
            "VALUE_ONLY"
        };
        let (grant,before,before_owned)=timeout_at(stop_at,async {
        let grants:Vec<Uuid>=sqlx::query_scalar(
            "SELECT g.grant_id FROM owner_beta_paper_recovery_grants g JOIN owner_beta_paper_control c ON c.account_id=g.account_id JOIN owner_beta_paper_sessions s ON s.pending_target_id=g.pending_target_id JOIN owner_beta_paper_lineage l ON l.id=g.lineage_id WHERE g.pending_target_id=$1 AND c.owner_user_id=$2 AND c.account_id=$3 AND g.step=$4 AND l.recommendation_run_id=$5 AND l.control_epoch<g.active_epoch AND g.active_epoch=c.control_epoch AND g.process_config_sha256=c.process_binding_sha256 AND g.consent_event_id=c.consent_event_id AND g.revoked_at IS NULL AND g.expires_at>clock_timestamp() AND g.valuation_receipt_sha256 IS NULL AND s.effective_session=$6 AND s.effective_session<(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date ORDER BY g.created_at DESC LIMIT 2",
        ).bind(target).bind(owner).bind(account).bind(step).bind(run).bind(case.target_effective_session)
            .fetch_all(&inspection).await.map_err(|_|FaultError::Database)?;
        if grants.len()!=1 {return Err(FaultError::Selection);}
        let grant=grants[0];let before=rows(&inspection,target,grant).await?;
        if (consumer==Consumer::Settlement)!=before["session"]["settlement_sha256"].is_null()
            || before["dispatch"].is_null() {return Err(FaultError::Selection);}
        let before_owned=economic_rows(&inspection,account,owner).await;
        economic_ok(&before_owned)?;
        Ok::<_,FaultError>((grant,before,before_owned))
        }).await.map_err(|_|FaultError::Deadline)??;
        let before_all = economic_ok(&before_owned)?;
        let (options, endpoint) = connection()?;
        let fault = Arc::new(Fault::new(stop_at));
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|_| FaultError::Endpoint)?;
        let local = listener.local_addr().map_err(|_| FaultError::Endpoint)?;
        let listener = Listener::new(listener, &fault);
        // Same credentials/database/options, only a private loopback transport
        // destination changes. Explicit original sslmode=disable is mandatory.
        let worker = worker_pool(options.host("127.0.0.1").port(local.port()));
        pools.worker = Some(worker.clone());
        // Catch proxy panics inside the owner so an already running bounded
        // consumer is still canceled cooperatively and awaited below.
        let mut relay = Box::pin(async {
            std::panic::AssertUnwindSafe(relay(listener, endpoint, Arc::clone(&fault)))
                .catch_unwind()
                .await
                .unwrap_or(Err(FaultError::Panic))
        });
        let prepared = tokio::select! {
            r=worker_identity(&worker,&packet.nonce)=>r,
            _=&mut relay=>Err(FaultError::MissingBoundary),
            _=tokio::time::sleep_until(stop_at)=>Err(FaultError::Deadline),
        };
        prepared?;
        let queue = JobQueue::new(worker.clone(), None, job_queue::QueueConfig::default());
        let probe = Arc::new(Probe {
            case: case.clone(),
            queue: queue.clone(),
            claim: None,
            monitor: Arc::new(AsyncMutex::new(None)),
            entered: AtomicBool::new(false),
            consumer_returned: AtomicBool::new(false),
            sql_entered: AtomicBool::new(false),
            direct_calendar: AtomicBool::new(false),
            calendar_checked: AtomicBool::new(false),
            state: Mutex::new(ProbeState {
                ack_fault: Some(Arc::clone(&fault)),
                ..ProbeState::default()
            }),
        });
        let (sender, receiver) = oneshot::channel();
        let (cancel, shutdown) = watch::channel(false);
        let remaining = stop_at
            .checked_duration_since(AsyncInstant::now())
            .ok_or(FaultError::Deadline)?;
        let mut action = Box::pin(run_bound_paper_action(
            PaperProcessRole::Worker,
            remaining,
            Some(shutdown),
            RealWork {
                pool: worker.clone(),
                queue,
                probe: Arc::clone(&probe),
                claim: None,
                heartbeat: None,
                report: sender,
            },
        ));
        let mut early = None;
        let cut = tokio::select! {
            r=&mut relay=>r,
            r=&mut action=>{early=Some(r);Err(FaultError::MissingBoundary)},
            _=tokio::time::sleep_until(stop_at)=>Err(FaultError::Deadline),
        };
        drop(relay); // no spawned proxy task; both borrowed pumps/sockets are gone
        let proof = if cut.is_ok() {
            timeout_at(
                stop_at,
                std::panic::AssertUnwindSafe(prove(
                    &inspection,
                    &fault,
                    &before,
                    before_all,
                    target,
                    grant,
                    account,
                    owner,
                    consumer,
                ))
                .catch_unwind(),
            )
            .await
            .map_err(|_| FaultError::Deadline)
            .and_then(|r| r.unwrap_or(Err(FaultError::Panic)))
        } else {
            Err(FaultError::MissingBoundary)
        };
        // Always release the actual held result and join the original bounded
        // action. Never cancel/drop its future merely because the proxy failed.
        fault.release.notify_one();
        if cut.is_err() || proof.is_err() {
            let _ = cancel.send(true);
        }
        let outer = match early {
            Some(r) => r,
            None => action.await,
        };
        let diagnostic = timeout_at(deadline, receiver)
            .await
            .map_err(|_| FaultError::Deadline)?
            .map_err(|_| FaultError::ClientOutcome)?;
        cut?;
        let (after, after_all) = proof?;
        let report = outer.map_err(|_| FaultError::ClientOutcome)?;
        if !fault.lost_ack.load(Ordering::Acquire)
            || fault.accepts.load(Ordering::Acquire) != 1
            || fault.arms.load(Ordering::Acquire) != 1
            || fault.callbacks.load(Ordering::Acquire) != 1
            || fault.live_sockets.load(Ordering::Acquire) != 0
            || fault.live_listeners.load(Ordering::Acquire) != 0
            || report.primary.as_deref() != Some("CommitUnknown")
            || !report.commit_started
            || report.committed
            || report.success
            || diagnostic.primary != report.primary
            || diagnostic.commit_started != report.commit_started
            || report.local_gate_matched
            || !report.scope_finished
            || !probe.entered.load(Ordering::Acquire)
            || !probe.sql_entered.load(Ordering::Acquire)
        {
            return Err(FaultError::ClientOutcome);
        }
        // No subsequent consumer, queue dispatch or economic retry. This is
        // independent readback after the original callback/scope has closed.
        timeout_at(deadline, async {
            let final_rows = rows(&inspection, target, grant).await?;
            let final_all = economic_rows(&inspection, account, owner).await;
            if final_rows != after
                || economic_ok(&final_all)?.immutable != after_all.immutable
                || economic_ok(&final_all)?.allowed_lifecycle != after_all.allowed_lifecycle
            {
                return Err(FaultError::IndependentEffect);
            }
            Ok::<_, FaultError>(())
        })
        .await
        .map_err(|_| FaultError::Deadline)??;
        if AsyncInstant::now() >= deadline {
            return Err(FaultError::Deadline);
        }
        Ok(())
    }

    fn fixture_deadline(
        started: AsyncInstant,
        packet_clock: AsyncInstant,
        remaining: Duration,
    ) -> AsyncInstant {
        (packet_clock + remaining.min(ACTION_TIMEOUT)).min(started + ACTION_TIMEOUT)
    }

    fn fixture(consumer: Consumer) {
        let started = AsyncInstant::now();
        let packet = read_packet().unwrap_or_else(|_| panic!("ACK_FIXTURE_PACKET_REQUIRED"));
        let earliest = packet
            .cases
            .iter()
            .map(|c| c.absolute_deadline)
            .min()
            .unwrap_or_else(|| panic!("ACK_FIXTURE_CASE_REQUIRED"));
        // Sample monotonic time before the wall-clock conversion so elapsed
        // conversion/setup time is never added back to the packet's budget.
        let packet_clock = AsyncInstant::now();
        let remaining = (earliest - Utc::now())
            .to_std()
            .unwrap_or_else(|_| panic!("ACK_FIXTURE_DEADLINE"))
            .min(ACTION_TIMEOUT);
        let deadline = fixture_deadline(started, packet_clock, remaining);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap_or_else(|_| panic!("ACK_FIXTURE_RUNTIME"));
        let result = runtime.block_on(async {
            let mut pools = Pools::default();
            // Assertions and panic propagation occur only after owned pools
            // close. The relay owns no child task and all socket guards drop
            // on timeout/cancellation/panic, including partial setup.
            let result = std::panic::AssertUnwindSafe(case(&mut pools, packet, consumer, deadline))
                .catch_unwind()
                .await;
            pools.close().await;
            if AsyncInstant::now() >= deadline {
                return Err(FaultError::Deadline);
            }
            result.unwrap_or(Err(FaultError::Panic))
        });
        drop(runtime);
        assert!(result.is_ok(), "ACK_FIXTURE_FAILED: {:?}", result.err());
    }

    #[test]
    #[ignore = "NOT_STAGED/NOT_RUN: real SETTLE_THEN_VALUE grant, original T/run/E Source/Raw/sealed Daily, live protected Current/ELF and unchanged private one-case Clean/Committed packet; explicit plaintext loopback isolated worker PG connection"]
    fn retained_recovery_settlement_commit_acknowledgment_loss() {
        fixture(Consumer::Settlement);
    }

    #[test]
    #[ignore = "NOT_STAGED/NOT_RUN: real VALUE_ONLY grant and original settlement/T/run/E, live protected Current/ELF and unchanged private one-case Clean/Committed packet; explicit plaintext loopback isolated worker PG connection"]
    fn retained_recovery_valuation_commit_acknowledgment_loss() {
        fixture(Consumer::Valuation);
    }

    // Transport cleanup only; no fabricated PG/Source authority. These tests
    // require actual loopback I/O when explicitly staged, never this source pass.
    async fn socket_cleanup_case(unwind_after_startup: bool, deadline: AsyncInstant) -> Result<()> {
        let backend_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|_| FaultError::Io)?;
        let endpoint = backend_listener.local_addr().map_err(|_| FaultError::Io)?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|_| FaultError::Io)?;
        let local = listener.local_addr().map_err(|_| FaultError::Io)?;
        let fault = Arc::new(Fault::new(deadline));
        let mut proxy = Box::pin(relay(
            Listener::new(listener, &fault),
            endpoint,
            Arc::clone(&fault),
        ));
        let peers = async {
            let client = TcpStream::connect(local)
                .await
                .map_err(|_| FaultError::Io)?;
            let (backend, _) = backend_listener
                .accept()
                .await
                .map_err(|_| FaultError::Io)?;
            Ok::<_, FaultError>((client, backend))
        };
        let (client, backend) = tokio::select! {
            result=peers=>result?,
            _=&mut proxy=>return Err(FaultError::Cleanup),
        };
        drop(backend_listener);
        let sockets_ready = async {
            while fault.live_sockets.load(Ordering::Acquire) != 2 {
                fault.socket_changed.notified().await;
            }
        };
        tokio::select! { _=sockets_ready=>{}, _=&mut proxy=>return Err(FaultError::Cleanup) }
        if unwind_after_startup {
            // Minimal protocol framing sample only; no auth/request is supplied
            // and no backend success is simulated. Require byte preservation.
            let startup = b"\0\0\0\x0d\0\x03\0\0u\0v\0\0";
            let exchange = async {
                write_all(&client, startup).await?;
                let mut received = Buffer(vec![0; startup.len()]);
                read_exact(&backend, &mut received.0).await?;
                if received.0.as_slice() != startup.as_slice() {
                    return Err(FaultError::Startup);
                }
                Ok::<_, FaultError>(())
            };
            tokio::select! { result=exchange=>result?, _=&mut proxy=>return Err(FaultError::Cleanup) }
            let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                let _owned_proxy = proxy;
                panic!("ACK_PROXY_TEST_UNWIND");
            }));
            if panic.is_ok() {
                return Err(FaultError::Cleanup);
            }
        } else {
            drop(proxy); // cancellation while waiting for the startup header
        }
        if fault.live_sockets.load(Ordering::Acquire) != 0
            || fault.live_listeners.load(Ordering::Acquire) != 0
        {
            return Err(FaultError::Cleanup);
        }
        let mut one = [0; 1];
        if read_exact(&client, &mut one).await != Err(FaultError::EarlyEof)
            || read_exact(&backend, &mut one).await != Err(FaultError::EarlyEof)
        {
            return Err(FaultError::Cleanup);
        }
        // The exact single-accept listener was dropped, so no reconnect is possible.
        if TcpStream::connect(local).await.is_ok() {
            return Err(FaultError::Cleanup);
        }
        Ok(())
    }
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "NOT_RUN: explicit isolated loopback transport cleanup fixture; not a PG/Source Origin or economic acceptance test"]
    async fn proxy_cancellation_closes_exact_owned_descriptors() {
        let deadline = AsyncInstant::now() + Duration::from_secs(5);
        assert!(matches!(
            timeout_at(deadline, socket_cleanup_case(false, deadline)).await,
            Ok(Ok(()))
        ));
    }
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "NOT_RUN: explicit isolated loopback transport unwind fixture; not a PG/Source Origin or economic acceptance test"]
    async fn proxy_panic_closes_exact_owned_descriptors_after_startup_forwarding() {
        let deadline = AsyncInstant::now() + Duration::from_secs(5);
        assert!(matches!(
            timeout_at(deadline, socket_cleanup_case(true, deadline)).await,
            Ok(Ok(()))
        ));
    }

    #[test]
    fn commit_frame_requires_exact_tag_transaction_and_backend() {
        // Parser-only examples: these bytes are not authenticated Original/PG proof.
        let mut s = Backend {
            authenticated: true,
            pid: 7,
            ready: b'T',
            ..Backend::default()
        };
        assert_eq!(
            s.inspect(b'D', b"\0\x01\0\0\0\x06COMMIT", 7),
            Ok(Forward::Pass)
        );
        assert_eq!(s.inspect(b'C', b"SELECT 1\0", 7), Ok(Forward::Pass));
        assert_eq!(s.inspect(b'C', b"COMMIT extra\0", 7), Ok(Forward::Pass));
        assert_eq!(
            s.inspect(b'C', b"COMMIT\0", 8),
            Err(FaultError::TransactionBoundary)
        );
        assert_eq!(s.inspect(b'C', b"COMMIT\0", 7), Ok(Forward::Withhold));
        assert_eq!(s.inspect(b'Z', b"I", 7), Ok(Forward::LostAcknowledgment));
        let mut idle = Backend {
            authenticated: true,
            pid: 7,
            ready: b'I',
            ..Backend::default()
        };
        assert_eq!(
            idle.inspect(b'C', b"COMMIT\0", 7),
            Err(FaultError::TransactionBoundary)
        );
        assert_eq!(
            Backend::default().inspect(b'C', b"COMMIT\0", 7),
            Err(FaultError::TransactionBoundary)
        );
    }
    #[test]
    fn rollback_error_and_incomplete_commit_cannot_prove_loss() {
        for (tag, body) in [(b'C', b"ROLLBACK\0".as_slice()), (b'E', b"".as_slice())] {
            let mut s = Backend {
                authenticated: true,
                pid: 7,
                ready: b'T',
                ..Backend::default()
            };
            assert!(s.inspect(tag, body, 7).is_err());
        }
        for (tag, body) in [
            (b'Z', b"T".as_slice()),
            (b'D', b"COMMIT\0".as_slice()),
            (b'C', b"COMMIT\0".as_slice()),
            (b'E', b"".as_slice()),
        ] {
            let mut s = Backend {
                authenticated: true,
                pid: 7,
                ready: b'T',
                withheld_commit: true,
                ..Backend::default()
            };
            assert!(s.inspect(tag, body, 7).is_err());
        }
        let mut s = Backend {
            withheld_commit: true,
            ..Backend::default()
        };
        for _ in 0..TRAILING_CAP {
            assert_eq!(s.inspect(b'N', b"Mnotice\0\0", 7), Ok(Forward::Withhold));
        }
        assert_eq!(s.inspect(b'Z', b"I", 7), Err(FaultError::PostCommitFrame));
    }
    #[test]
    fn original_budget_charges_packet_parsing_and_runtime_setup() {
        let started = AsyncInstant::now();
        let packet_clock = started + Duration::from_secs(12);
        let runtime_ready = started + Duration::from_secs(19);
        let deadline = fixture_deadline(started, packet_clock, Duration::from_secs(120));
        assert_eq!(deadline, started + ACTION_TIMEOUT);
        assert_eq!(
            deadline.duration_since(runtime_ready),
            Duration::from_secs(71)
        );
        let packet_deadline = fixture_deadline(started, packet_clock, Duration::from_secs(10));
        assert_eq!(packet_deadline, started + Duration::from_secs(22));
        assert_eq!(
            packet_deadline.duration_since(runtime_ready),
            Duration::from_secs(3)
        );
        assert!(fixture_deadline(started, runtime_ready, Duration::ZERO) <= runtime_ready);
    }

    #[test]
    fn wire_bounds_and_tls_negotiation_fail_closed() {
        assert!(body_length(3_u32.to_be_bytes(), FRAME_CAP).is_err());
        assert!(body_length((FRAME_CAP as u32 + 1).to_be_bytes(), FRAME_CAP).is_err());
        assert_eq!(body_length(4_u32.to_be_bytes(), FRAME_CAP), Ok(0));
        for code in [80_877_103_u32, 80_877_104] {
            assert_eq!(
                startup_plain(&code.to_be_bytes()),
                Err(FaultError::EncryptedTransport)
            );
        }
        let mut backend = Backend::default();
        assert_eq!(
            backend.inspect(b'C', b"COMMIT\0extra", 0),
            Err(FaultError::FrameLength)
        );
        assert_eq!(backend.inspect(b'N', b"", 0), Err(FaultError::FrameLength));
        assert_eq!(
            backend.inspect(b'S', b"key\0value", 0),
            Err(FaultError::FrameLength)
        );
        assert!(startup_plain(b"").is_err());
        assert_eq!(startup_plain(b"\0\x03\0\0u\0v\0\0"), Ok(()));
        assert!(startup_plain(b"\0\x03\0\0u\0v\0").is_err());
        assert!(startup_plain(b"\0\x03\0\0u\0v\0\0extra\0").is_err());
        let mut budget = WireBudget {
            bytes: STREAM_CAP,
            frames: 0,
        };
        assert_eq!(budget.charge(1), Err(FaultError::StreamLimit));
        let mut budget = WireBudget {
            bytes: 0,
            frames: FRAME_COUNT_CAP,
        };
        assert_eq!(budget.charge(0), Err(FaultError::StreamLimit));
    }
}
