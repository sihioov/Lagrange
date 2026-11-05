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
        self.state.lock().unwrap().backend.push(backend);
        self.sql_entered.store(true, Ordering::Release);
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
                            with_checked_paper_action(
                                &self.pool,
                                context,
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
                            .await
                        } else {
                            with_checked_paper_action(
                                &self.pool,
                                context,
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
                            .await
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
                .log_statements(tracing::log::LevelFilter::Off),
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
