//! One-shot, role-separated evidence helper. Never an admin or broker client.
use super::database_options;
use domain::ContentHash;
#[cfg(target_os = "linux")]
use job_queue::owner_beta::paper::evidence::checked_admission::{
    LocalObservedAdmission, LocalObservedAdmissionFailure, verify_admission_observed_in,
};
use job_queue::owner_beta::paper::source_evidence::{
    self as wire, CheckedProtectedSourceChain, CheckedSourceCompletion, CheckedSourceFailure,
    EvidenceError, ProtectedSourceChain,
};
use market_data::{
    checked_close::{Attempt, Failure, Gap},
    daily_price_only::DailyPricePinsV1Wire,
    raw_custody::RawInvocation,
};
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use std::{ffi::OsString, path::Path, process::ExitCode, time::Duration};
use uuid::Uuid;
type Result<T> = std::result::Result<T, EvidenceError>;
fn required(ok: bool) -> Result<()> {
    if ok { Ok(()) } else { Err(EvidenceError) }
}
fn string<'a>(v: &'a Value, k: &str) -> Result<&'a str> {
    v[k].as_str().ok_or(EvidenceError)
}
fn hash(v: &Value, k: &str) -> Result<ContentHash> {
    ContentHash::parse(string(v, k)?).map_err(|_| EvidenceError)
}
pub(super) async fn dispatch(values: Vec<OsString>) -> Option<ExitCode> {
    if values.get(1).is_none_or(|v| v != "paper-evidence") {
        return None;
    }
    let retained = execute(values).await;
    let exit = match &retained.result {
        Ok(value) if retained.publishable() => {
            println!("{}", value);
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("PAPER_SOURCE_EVIDENCE_REJECTED");
            ExitCode::FAILURE
        }
    };
    // Both successful bags and whole failure survive through this outer consumer.
    drop(retained);
    Some(exit)
}

fn last_stage<'a>(
    source: &'a CheckedProtectedSourceChain<'_, '_>,
) -> std::result::Result<&'a str, HelperFailure> {
    source
        .events()?
        .iter()
        .rev()
        .find(|e| !matches!(e["kind"].as_str(), Some("CONTAINER")))
        .and_then(|e| e["kind"].as_str())
        .ok_or(EvidenceError.into())
}
// Whole outcomes remain owned across the original async helper operation.
#[derive(Default)]
struct AdmissionRetention {
    #[cfg(target_os = "linux")]
    values: Vec<LocalObservedAdmission>,
}
enum HelperFailure {
    Database(Box<DatabaseFailure>),
    Source(Box<CheckedSourceFailure>),
    Policy(EvidenceError),
    #[cfg(target_os = "linux")]
    Observed(Box<LocalObservedAdmissionFailure>),
}
impl From<EvidenceError> for HelperFailure {
    fn from(source: EvidenceError) -> Self {
        Self::Policy(source)
    }
}
impl From<CheckedSourceFailure> for HelperFailure {
    fn from(source: CheckedSourceFailure) -> Self {
        Self::Source(Box::new(source))
    }
}
impl std::fmt::Debug for HelperFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Database(source) => f
                .debug_struct("Database")
                .field("phase", &source.phase)
                .finish_non_exhaustive(),
            Self::Source(source) => f
                .debug_tuple("Source")
                .field(&source.diagnostics())
                .finish(),
            Self::Policy(source) => f.debug_tuple("Policy").field(source).finish(),
            #[cfg(target_os = "linux")]
            Self::Observed(source) => f
                .debug_tuple("Observed")
                .field(&source.diagnostics())
                .finish(),
        }
    }
}
type UnitSettlement = std::result::Result<(), Failure<std::convert::Infallible>>;
struct RetainedExecution {
    result: std::result::Result<Value, HelperFailure>,
    _admissions: AdmissionRetention,
    database: HelperDbCompletion,
    source: Option<CheckedSourceCompletion>,
    local: UnitSettlement,
    terminal: UnitSettlement,
}
// The genuine body Result is owned before the same chain's consuming cleanup.
fn retain_execution_result<T, E, C>(
    result: std::result::Result<T, E>,
    finish: impl FnOnce() -> C,
) -> (std::result::Result<T, E>, C) {
    let completion = finish();
    (result, completion)
}
fn execution_publishable<T, E>(
    result: &std::result::Result<T, E>,
    source_clean: bool,
    local: &UnitSettlement,
    terminal: &UnitSettlement,
) -> bool {
    // Aggregate publication refusal, never an isolated native-close certificate.
    result.is_ok() && source_clean && local.is_ok() && terminal.is_ok()
}
impl RetainedExecution {
    fn publishable(&self) -> bool {
        execution_publishable(
            &self.result,
            self.source
                .as_ref()
                .is_none_or(CheckedSourceCompletion::is_clean),
            &self.local,
            &self.terminal,
        ) && self.database.api_allows_publication()
    }
}
struct ExecutionUnwind {
    attempt: Attempt,
    armed: bool,
}
impl Drop for ExecutionUnwind {
    fn drop(&mut self) {
        if self.armed {
            self.attempt.gap(Gap::Abandoned);
        }
    }
}

fn verify_admission(
    source: &CheckedProtectedSourceChain<'_, '_>,
    pins: &DailyPricePinsV1Wire,
    retained: &mut AdmissionRetention,
    invocation: &RawInvocation<'_>,
    expected: &Attempt,
    captured: &Attempt,
) -> std::result::Result<(), HelperFailure> {
    #[cfg(target_os = "linux")]
    {
        match verify_admission_observed_in(source, pins, invocation, expected, captured) {
            Ok(admission) => {
                retained.values.push(admission);
                Ok(())
            }
            Err(failure) => Err(HelperFailure::Observed(Box::new(failure))),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (source, pins, retained, invocation, expected, captured);
        Err(HelperFailure::Policy(EvidenceError))
    }
}
async fn execute(args: Vec<OsString>) -> RetainedExecution {
    let attempt = Attempt::default();
    let expected = attempt.clone();
    let captured = attempt.clone();
    let mut unwind = ExecutionUnwind {
        attempt: captured.clone(),
        armed: true,
    };
    let mut retained = AdmissionRetention::default();
    let mut db_state = HelperDbState::new();
    let (result, database, source, local) = {
        let invocation = RawInvocation::new(&attempt);
        let mut source_slot = None;
        let result = execute_body(
            args,
            &mut retained,
            &mut source_slot,
            &mut db_state,
            &invocation,
            &expected,
            &captured,
        )
        .await;
        // This can be committedOk. Save it before any later cleanup/settlement.
        if result.is_err() && captured.check() {
            captured.gap(Gap::Abandoned);
        }
        let (result, database) = retain_database_result(result, finish_database(db_state)).await;
        let (result, source) = retain_execution_result(result, || {
            source_slot
                .take()
                .map(CheckedProtectedSourceChain::finish_local)
        });
        let local = invocation.settle_local_in(&captured, Ok::<(), std::convert::Infallible>(()));
        (result, database, source, local)
    };
    let terminal = attempt.finish(Ok::<(), std::convert::Infallible>(()));
    unwind.armed = false;
    RetainedExecution {
        result,
        _admissions: retained,
        database,
        source,
        local,
        terminal,
    }
}
async fn execute_body<'scope, 'attempt>(
    args: Vec<OsString>,
    retained: &mut AdmissionRetention,
    source_slot: &mut Option<CheckedProtectedSourceChain<'scope, 'attempt>>,
    db: &mut HelperDbState,
    invocation: &'scope RawInvocation<'attempt>,
    expected: &'scope Attempt,
    captured: &'scope Attempt,
) -> std::result::Result<Value, HelperFailure> {
    let args = args
        .into_iter()
        .map(|s| s.into_string().map_err(|_| EvidenceError))
        .collect::<Result<Vec<_>>>()?;
    required(args.len() >= 4)?;
    let operation = args[2].as_str();
    let slot = &args[3];
    required(
        matches!(
            operation,
            "reserve" | "link-admission" | "link-run" | "read"
        ) && args.len()
            == if matches!(operation, "link-admission" | "link-run") {
                5
            } else {
                4
            },
    )?;
    let admitter = matches!(operation, "reserve" | "link-admission");
    #[cfg(target_os = "linux")]
    // SAFETY: argument-free process identity query.
    required(unsafe { libc::geteuid() } == if admitter { 10003 } else { 10001 })?;
    #[cfg(not(target_os = "linux"))]
    return Err(EvidenceError.into());
    required(
        std::env::var("APP_ENV").as_deref() == Ok("production")
            && std::env::var("DB_USER").as_deref()
                == Ok(if admitter {
                    "daily_input_admitter"
                } else {
                    "worker"
                }),
    )?;
    for name in [
        "DATABASE_URL",
        "DATABASE_URL_FILE",
        "DB_PASSWORD",
        "KIS_APP_KEY_FILE",
        "KIS_APP_SECRET_FILE",
    ] {
        required(std::env::var_os(name).is_none())?;
    }
    *source_slot = Some(ProtectedSourceChain::open_checked_in(
        Path::new("/data/owner-beta-paper-evidence/events"),
        slot,
        invocation,
        expected,
        captured,
    )?);
    let source = source_slot.as_ref().expect("actual opened source chain");
    required(
        std::env::var("LAGRANGE_CODE_COMMIT").ok().as_deref()
            == source.first()?["cohort"]["verifier_revision"].as_str(),
    )?;
    let options = database_options(true).map_err(|_| EvidenceError)?;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(5))
        .connect_with(options)
        .await
        .map_err(|source| database_failure(DatabasePhase::Connect, source))?;
    db.pool = Some(pool);
    let tx = db
        .pool
        .as_ref()
        .expect("actual returned pool")
        .begin()
        .await
        .map_err(|source| database_failure(DatabasePhase::Begin, source))?;
    db.transaction = Some(tx);
    db.phase = TransactionPhase::Open;
    {
        let tx = &mut **db
            .transaction
            .as_mut()
            .expect("actual returned transaction");
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .execute(&mut *tx)
            .await
            .map_err(|source| database_failure(DatabasePhase::Isolation, source))?;
        let first = source.first()?;
        let session = NaiveDate::parse_from_str(string(first, "session")?, "%Y-%m-%d")
            .map_err(|_| EvidenceError)?;
        let result: Value = match operation {
            "reserve" => {
                required(last_stage(source)? == "RESERVED")?;
                let reservation =
                    Uuid::parse_str(string(first, "reservation_id")?).map_err(|_| EvidenceError)?;
                let root_hash = wire::digest(&wire::canonical(source.event("RESERVED")?)?);
                sqlx::query_scalar(
                    "SELECT public.reserve_owner_beta_paper_source_slot($1,$2,$3,$4,$5)",
                )
                .bind(slot)
                .bind(reservation)
                .bind(root_hash)
                .bind(&first["cohort"])
                .bind(session)
                .fetch_one(&mut *tx)
                .await
                .map_err(|source| database_failure(DatabasePhase::Reserve, source))?
            }
            "link-admission" => {
                required(last_stage(source)? == "SOURCE_FINISHED")?;
                let pin: &str = &args[4];
                ContentHash::parse(pin).map_err(|_| EvidenceError)?;
                let row:(String,String,String,String)=sqlx::query_as("SELECT content_sha256,manifest_sha256,admission_sha256,policy_sha256 FROM public.read_owner_beta_daily_admitted_input($1) WHERE usable").bind(pin).fetch_one(&mut *tx).await.map_err(|source| database_failure(DatabasePhase::CandidateRead, source))?;
                let v = json!({"content_sha256":row.0,"manifest_sha256":row.1,"admission_sha256":row.2,"policy_sha256":row.3});
                let v = retain_returned_value(&mut db.returned.candidate, v);
                verify_admission(
                    source,
                    &DailyPricePinsV1Wire {
                        content_sha256: hash(v, "content_sha256")?,
                        manifest_sha256: hash(v, "manifest_sha256")?,
                        admission_sha256: hash(v, "admission_sha256")?,
                        policy_sha256: hash(v, "policy_sha256")?,
                    },
                    retained,
                    invocation,
                    expected,
                    captured,
                )?;
                let event = source.event("SOURCE_FINISHED")?;
                sqlx::query_scalar(
                    "SELECT public.link_owner_beta_paper_source_admission($1,$2,$3,$4)",
                )
                .bind(slot)
                .bind(wire::digest(&wire::canonical(event)?))
                .bind(string(&event["data"], "original_set_sha256")?)
                .bind(pin)
                .fetch_one(&mut *tx)
                .await
                .map_err(|source| database_failure(DatabasePhase::LinkAdmission, source))?
            }
            "link-run" => {
                required(last_stage(source)? == "ADMISSION_LINKED")?;
                let run = Uuid::parse_str(&args[4]).map_err(|_| EvidenceError)?;
                required(!run.is_nil())?;
                sqlx::query_scalar("SELECT public.link_owner_beta_paper_daily_run($1,$2)")
                    .bind(slot)
                    .bind(run)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(|source| database_failure(DatabasePhase::LinkRun, source))?
            }
            "read" => {
                sqlx::query_scalar("SELECT public.read_owner_beta_paper_session_evidence($1,$2)")
                    .bind(session)
                    .bind(&first["cohort"])
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(|source| database_failure(DatabasePhase::Read, source))?
            }
            _ => return Err(EvidenceError.into()),
        };
        let result = retain_returned_value(&mut db.returned.output, result);
        required(
            result["schema"] == "owner-beta-paper-db-v1"
                && result["slot"]["slot_id"] == first["slot_id"]
                && result["slot"]["cohort"] == first["cohort"]
                && result["slot"]["reservation_id"] == first["reservation_id"]
                && result["slot"]["root_event_sha256"]
                    == wire::digest(&wire::canonical(source.event("RESERVED")?)?),
        )?;
        required(wire::canonical(result)?.len() <= wire::SESSION_LIMIT)?;
        wire::validate_wire(result)?;
        if !result["admission"].is_null() {
            let admission = &result["admission"];
            let event = source.event("SOURCE_FINISHED")?;
            required(
                admission["source_event_sha256"] == wire::digest(&wire::canonical(event)?)
                    && admission["original_set_sha256"] == event["data"]["original_set_sha256"],
            )?;
            let pins = &result["candidate_pins"];
            required(admission["admission_sha256"] == pins["admission_sha256"])?;
            // A later read does not inherit yesterday's Raw/rights/file check.
            if operation != "link-admission" {
                verify_admission(
                    source,
                    &DailyPricePinsV1Wire {
                        content_sha256: hash(pins, "content_sha256")?,
                        manifest_sha256: hash(pins, "manifest_sha256")?,
                        admission_sha256: hash(pins, "admission_sha256")?,
                        policy_sha256: hash(pins, "policy_sha256")?,
                    },
                    retained,
                    invocation,
                    expected,
                    captured,
                )?;
            }
            if let Some(link) = source.optional_event("ADMISSION_LINKED")? {
                required(link["data"]["admission_sha256"] == admission["admission_sha256"])?;
            }
        }
        if let Some(link) = source.optional_event("RUN_LINKED")? {
            for key in ["run_id", "job_id", "attempt_id"] {
                required(link["data"][key] == result["run"][key])?;
            }
            required(result["terminal"] == "SUCCEEDED")?;
        }
        source.revalidate()?;
    }
    commit_returned(&mut db.transaction, &mut db.phase, |tx| async move {
        tx.commit().await
    })
    .await
    .map_err(|source| database_failure(DatabasePhase::Commit, source))?;
    Ok(db
        .returned
        .output
        .take()
        .expect("actual committed query value"))
}
use chrono::NaiveDate;

// BEGIN C17 DB API completion
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DatabasePhase {
    Connect,
    Begin,
    Isolation,
    Reserve,
    CandidateRead,
    LinkAdmission,
    LinkRun,
    Read,
    Commit,
}
struct DatabaseFailure {
    phase: DatabasePhase,
    // Intentionally retained whole, never formatted as server/credential prose.
    _source: sqlx::Error,
}
fn database_failure(phase: DatabasePhase, source: sqlx::Error) -> HelperFailure {
    HelperFailure::Database(Box::new(DatabaseFailure {
        phase,
        _source: source,
    }))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TransactionPhase {
    NotAcquired,
    Open,
    CommitStarted,
    CommitReturnedOk,
    CommitReturnedErr,
    RollbackStarted,
    RollbackReturnedOk,
    RollbackReturnedErr,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PoolCloseObservation {
    // No application handle returned; startup internals are NOT attested absent.
    NotAcquired,
    // Unit-returning public API completed, not a native-close success receipt.
    AwaitReturned,
}
struct ReturnedDatabaseValues<V> {
    candidate: Option<V>,
    output: Option<V>,
}
impl<V> ReturnedDatabaseValues<V> {
    fn new() -> Self {
        Self {
            candidate: None,
            output: None,
        }
    }
}
struct DatabaseState<P, T, V> {
    pool: Option<P>,
    transaction: Option<T>,
    phase: TransactionPhase,
    returned: ReturnedDatabaseValues<V>,
}
impl<P, T, V> DatabaseState<P, T, V> {
    fn new() -> Self {
        Self {
            pool: None,
            transaction: None,
            phase: TransactionPhase::NotAcquired,
            returned: ReturnedDatabaseValues::new(),
        }
    }
}
type HelperDbState = DatabaseState<sqlx::PgPool, sqlx::Transaction<'static, sqlx::Postgres>, Value>;
struct DatabaseCompletion<E, V> {
    phase: TransactionPhase,
    rollback: Option<std::result::Result<(), E>>,
    pool_close: PoolCloseObservation,
    // Returned data is diagnostic only; no bytes-to-authority constructor.
    returned: ReturnedDatabaseValues<V>,
}
impl<E, V> DatabaseCompletion<E, V> {
    fn api_allows_publication(&self) -> bool {
        self.phase == TransactionPhase::CommitReturnedOk
            && self.rollback.is_none()
            && self.pool_close == PoolCloseObservation::AwaitReturned
    }
}
impl<E, V> std::fmt::Debug for DatabaseCompletion<E, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DatabaseApiCompletion")
            .field("phase", &self.phase)
            .field("rollback_returned", &self.rollback.is_some())
            .field(
                "rollback_failed",
                &self.rollback.as_ref().map(|r| r.is_err()),
            )
            .field("pool_close", &self.pool_close)
            .field("candidate_returned", &self.returned.candidate.is_some())
            .field("output_returned", &self.returned.output.is_some())
            .finish_non_exhaustive()
    }
}
type HelperDbCompletion = DatabaseCompletion<sqlx::Error, Value>;

// This shared seam owns the first Result while cleanup is pending. It never
// places JSON or an error body in a canonical unit settlement.
async fn retain_database_result<T, E, C>(
    original: std::result::Result<T, E>,
    completion: impl std::future::Future<Output = C>,
) -> (std::result::Result<T, E>, C) {
    let completion = completion.await;
    (original, completion)
}
fn retain_returned_value<V>(slot: &mut Option<V>, value: V) -> &V {
    *slot = Some(value);
    slot.as_ref().expect("actual returned query value")
}
async fn commit_returned<T, E, F>(
    transaction: &mut Option<T>,
    phase: &mut TransactionPhase,
    commit: impl FnOnce(T) -> F,
) -> std::result::Result<(), E>
where
    F: std::future::Future<Output = std::result::Result<(), E>>,
{
    // Taking the actual handle BEFORE await is essential: Err/cancellation does
    // not give an application transaction back for rollback/retry.
    let transaction = transaction.take().expect("actual returned transaction");
    *phase = TransactionPhase::CommitStarted;
    let original = commit(transaction).await;
    *phase = if original.is_ok() {
        TransactionPhase::CommitReturnedOk
    } else {
        TransactionPhase::CommitReturnedErr
    };
    original
}
async fn finish_database_with<P, T, E, V, R, C>(
    mut state: DatabaseState<P, T, V>,
    rollback: impl FnOnce(T) -> R,
    close: impl FnOnce(P) -> C,
) -> DatabaseCompletion<E, V>
where
    R: std::future::Future<Output = std::result::Result<(), E>>,
    C: std::future::Future<Output = ()>,
{
    let rollback = if let Some(transaction) = state.transaction.take() {
        state.phase = TransactionPhase::RollbackStarted;
        let original = rollback(transaction).await;
        state.phase = if original.is_ok() {
            TransactionPhase::RollbackReturnedOk
        } else {
            TransactionPhase::RollbackReturnedErr
        };
        Some(original)
    } else {
        None
    };
    // No transaction or connection borrow is held while the same pool waits.
    let pool_close = if let Some(pool) = state.pool.take() {
        close(pool).await;
        PoolCloseObservation::AwaitReturned
    } else {
        PoolCloseObservation::NotAcquired
    };
    DatabaseCompletion {
        phase: state.phase,
        rollback,
        pool_close,
        returned: state.returned,
    }
}
async fn finish_database(state: HelperDbState) -> HelperDbCompletion {
    finish_database_with(
        state,
        |transaction| async move { transaction.rollback().await },
        |pool| async move {
            pool.close().await;
            drop(pool);
        },
    )
    .await
}
// END C17 DB API completion

// BEGIN C17 shared async completion tests
#[cfg(test)]
mod database_completion_tests {
    use super::*;
    use market_data::checked_close::{Close, Errno, Owner, Role};
    use std::{
        cell::{Cell, RefCell},
        future::Future,
        pin::Pin,
        rc::Rc,
        task::{Context, Poll, Waker},
    };
    struct Body(Box<str>);
    struct Row(Box<str>);
    struct Counted {
        name: &'static str,
        log: Rc<RefCell<Vec<&'static str>>>,
    }
    impl Close for Counted {
        fn close(self) -> std::result::Result<(), Errno> {
            self.log.borrow_mut().push(self.name);
            Ok(())
        }
    }
    struct Resource {
        owner: Owner<Counted>,
    }
    fn resource(
        scope: &Attempt,
        log: &Rc<RefCell<Vec<&'static str>>>,
        name: &'static str,
    ) -> Resource {
        Resource {
            owner: scope.adopt(
                Counted {
                    name,
                    log: Rc::clone(log),
                },
                Role::Source,
            ),
        }
    }
    fn release(value: Resource) {
        value.owner.finish();
    }
    fn state() -> DatabaseState<Resource, Resource, Row> {
        DatabaseState::new()
    }
    fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
        let mut context = Context::from_waker(Waker::noop());
        future.poll(&mut context)
    }
    struct Gate(Rc<Cell<bool>>);
    impl Future for Gate {
        type Output = ();
        fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
            if self.0.get() {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }
    }

    #[test]
    fn original_nonclone_error_precedes_whole_later_rollback_error() {
        let scope = Attempt::default();
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut state = state();
        state.pool = Some(resource(&scope, &log, "pool"));
        state.transaction = Some(resource(&scope, &log, "transaction"));
        state.phase = TransactionPhase::Open;
        let first = Body("original query failure".into());
        let first_address = first.0.as_ptr();
        let later = Body("actual rollback API failure".into());
        let later_address = later.0.as_ptr();
        let first = Err::<Row, _>(first);
        scope.gap(Gap::Abandoned);
        let future = retain_database_result(
            first,
            finish_database_with(
                state,
                |tx| async move {
                    release(tx);
                    Err::<(), _>(later)
                },
                |pool| async move {
                    release(pool);
                },
            ),
        );
        let mut future = std::pin::pin!(future);
        let (first, completion) = match poll_once(future.as_mut()) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("ordinary ready completion"),
        };
        match first {
            Err(first) => assert_eq!(first.0.as_ptr(), first_address),
            Ok(_) => panic!("first failure is retained"),
        }
        assert_eq!(
            completion
                .rollback
                .as_ref()
                .unwrap()
                .as_ref()
                .err()
                .unwrap()
                .0
                .as_ptr(),
            later_address
        );
        assert_eq!(completion.phase, TransactionPhase::RollbackReturnedErr);
        assert_eq!(completion.pool_close, PoolCloseObservation::AwaitReturned);
        assert!(!completion.api_allows_publication());
        assert!(!scope.check());
        assert_eq!(&*log.borrow(), &["transaction", "pool"]);
    }

    #[test]
    fn pending_rollback_consumes_transaction_before_pool_and_all_later_settlement() {
        let scope = Attempt::default();
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut state = state();
        state.pool = Some(resource(&scope, &log, "pool"));
        state.transaction = Some(resource(&scope, &log, "transaction"));
        state.phase = TransactionPhase::Open;
        let ready = Rc::new(Cell::new(false));
        let tx_done = Cell::new(false);
        let pool_calls = Cell::new(0);
        let rollback_ready = Rc::clone(&ready);
        let future = async {
            let original = Err::<Row, _>(Body("first body".into()));
            let (original, completion) = retain_database_result(
                original,
                finish_database_with(
                    state,
                    |tx| async {
                        Gate(rollback_ready).await;
                        release(tx);
                        tx_done.set(true);
                        Ok::<(), Body>(())
                    },
                    |pool| async {
                        pool_calls.set(pool_calls.get() + 1);
                        assert!(tx_done.get());
                        release(pool);
                    },
                ),
            )
            .await;
            log.borrow_mut().push("source");
            log.borrow_mut().push("local");
            log.borrow_mut().push("terminal");
            (original, completion)
        };
        let mut future = std::pin::pin!(future);
        assert!(poll_once(future.as_mut()).is_pending());
        assert_eq!(pool_calls.get(), 0);
        assert!(log.borrow().is_empty());
        ready.set(true);
        let (original, completion) = match poll_once(future.as_mut()) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("gate released"),
        };
        assert!(original.is_err());
        assert!(completion.rollback.as_ref().unwrap().is_ok());
        assert_eq!(
            &*log.borrow(),
            &["transaction", "pool", "source", "local", "terminal"]
        );
    }

    #[test]
    fn pending_pool_close_keeps_same_success_allocation_before_source_local_terminal() {
        let scope = Attempt::default();
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut state = state();
        state.pool = Some(resource(&scope, &log, "pool"));
        state.phase = TransactionPhase::CommitReturnedOk;
        let returned = Row("actually committed data".into());
        let address = returned.0.as_ptr();
        let ready = Rc::new(Cell::new(false));
        let pool_ready = Rc::clone(&ready);
        let future = async {
            let (original, completion) = retain_database_result(
                Ok::<_, Body>(returned),
                finish_database_with(
                    state,
                    |_| {
                        std::future::ready(Err(Body("must not rollback a committed handle".into())))
                    },
                    |pool| async {
                        Gate(pool_ready).await;
                        release(pool);
                    },
                ),
            )
            .await;
            log.borrow_mut().push("source");
            log.borrow_mut().push("local");
            log.borrow_mut().push("terminal");
            (original, completion)
        };
        let mut future = std::pin::pin!(future);
        assert!(poll_once(future.as_mut()).is_pending());
        assert!(log.borrow().is_empty());
        ready.set(true);
        let (original, completion) = match poll_once(future.as_mut()) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("gate released"),
        };
        assert_eq!(original.as_ref().ok().unwrap().0.as_ptr(), address);
        assert!(completion.api_allows_publication());
        assert_eq!(&*log.borrow(), &["pool", "source", "local", "terminal"]);
    }

    #[test]
    fn no_returned_pool_and_begin_failure_do_not_fabricate_cleanup_handles() {
        for returned_pool in [false, true] {
            let scope = Attempt::default();
            let log = Rc::new(RefCell::new(Vec::new()));
            let mut state = state();
            if returned_pool {
                state.pool = Some(resource(&scope, &log, "pool"));
            }
            let rollback_calls = Cell::new(0);
            let close_calls = Cell::new(0);
            let future = finish_database_with(
                state,
                |tx| {
                    rollback_calls.set(rollback_calls.get() + 1);
                    release(tx);
                    std::future::ready(Ok::<(), Body>(()))
                },
                |pool| {
                    close_calls.set(close_calls.get() + 1);
                    release(pool);
                    std::future::ready(())
                },
            );
            let mut future = std::pin::pin!(future);
            let completion = match poll_once(future.as_mut()) {
                Poll::Ready(value) => value,
                Poll::Pending => panic!("ordinary ready completion"),
            };
            assert_eq!(rollback_calls.get(), 0);
            assert_eq!(close_calls.get(), usize::from(returned_pool));
            assert!(completion.rollback.is_none());
            assert_eq!(completion.phase, TransactionPhase::NotAcquired);
            assert_eq!(
                completion.pool_close,
                if returned_pool {
                    PoolCloseObservation::AwaitReturned
                } else {
                    PoolCloseObservation::NotAcquired
                }
            );
            // NotAcquired concerns returned handles only; no SDK-internals claim.
            assert!(!completion.api_allows_publication());
        }
    }

    #[test]
    fn commit_ok_consumes_once_and_cleanup_never_rolls_back() {
        let scope = Attempt::default();
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut state = state();
        state.pool = Some(resource(&scope, &log, "pool"));
        state.transaction = Some(resource(&scope, &log, "transaction"));
        state.phase = TransactionPhase::Open;
        let commit_calls = Cell::new(0);
        let rollback_calls = Cell::new(0);
        let future = async {
            let original = commit_returned(&mut state.transaction, &mut state.phase, |tx| {
                commit_calls.set(commit_calls.get() + 1);
                release(tx);
                std::future::ready(Ok::<(), Body>(()))
            })
            .await;
            assert!(state.transaction.is_none());
            let completion = finish_database_with(
                state,
                |tx| {
                    rollback_calls.set(rollback_calls.get() + 1);
                    release(tx);
                    std::future::ready(Err(Body("unexpected rollback".into())))
                },
                |pool| async {
                    release(pool);
                },
            )
            .await;
            (original, completion)
        };
        let mut future = std::pin::pin!(future);
        let (original, completion) = match poll_once(future.as_mut()) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("ordinary ready completion"),
        };
        assert!(original.is_ok() && completion.api_allows_publication());
        assert_eq!((commit_calls.get(), rollback_calls.get()), (1, 0));
        assert_eq!(&*log.borrow(), &["transaction", "pool"]);
    }

    #[test]
    fn commit_error_keeps_returned_data_and_has_no_application_rollback_or_retry() {
        let scope = Attempt::default();
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut state = state();
        state.pool = Some(resource(&scope, &log, "pool"));
        state.transaction = Some(resource(&scope, &log, "transaction"));
        state.phase = TransactionPhase::Open;
        let row = Row("returned before commit".into());
        let address = row.0.as_ptr();
        retain_returned_value(&mut state.returned.output, row);
        let error = Body("actual returned commit error".into());
        let error_address = error.0.as_ptr();
        let commit_calls = Cell::new(0);
        let rollback_calls = Cell::new(0);
        let future = async {
            let original = commit_returned(&mut state.transaction, &mut state.phase, |tx| {
                commit_calls.set(commit_calls.get() + 1);
                release(tx);
                std::future::ready(Err::<(), _>(error))
            })
            .await;
            assert!(state.transaction.is_none());
            retain_database_result(
                original,
                finish_database_with(
                    state,
                    |tx| {
                        rollback_calls.set(rollback_calls.get() + 1);
                        release(tx);
                        std::future::ready(Ok::<(), Body>(()))
                    },
                    |pool| async {
                        release(pool);
                    },
                ),
            )
            .await
        };
        let mut future = std::pin::pin!(future);
        let (original, completion) = match poll_once(future.as_mut()) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("ordinary ready completion"),
        };
        assert_eq!(original.as_ref().err().unwrap().0.as_ptr(), error_address);
        assert_eq!(
            completion.returned.output.as_ref().unwrap().0.as_ptr(),
            address
        );
        assert_eq!(completion.phase, TransactionPhase::CommitReturnedErr);
        assert!(completion.rollback.is_none());
        assert_eq!((commit_calls.get(), rollback_calls.get()), (1, 0));
        assert!(!completion.api_allows_publication());
    }

    #[test]
    fn post_query_policy_error_keeps_both_actual_values_before_rollback() {
        let scope = Attempt::default();
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut state = state();
        state.pool = Some(resource(&scope, &log, "pool"));
        state.transaction = Some(resource(&scope, &log, "transaction"));
        state.phase = TransactionPhase::Open;
        let candidate = Row("actual first query allocation".into());
        let candidate_address = candidate.0.as_ptr();
        let output = Row("actual output query allocation".into());
        let output_address = output.0.as_ptr();
        retain_returned_value(&mut state.returned.candidate, candidate);
        retain_returned_value(&mut state.returned.output, output);
        let policy = Body("actual later policy source".into());
        let policy_address = policy.0.as_ptr();
        let future = retain_database_result(
            Err::<Row, _>(policy),
            finish_database_with(
                state,
                |tx| async {
                    release(tx);
                    Ok::<(), Body>(())
                },
                |pool| async {
                    release(pool);
                },
            ),
        );
        let mut future = std::pin::pin!(future);
        let (original, completion) = match poll_once(future.as_mut()) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("ordinary ready completion"),
        };
        assert_eq!(original.as_ref().err().unwrap().0.as_ptr(), policy_address);
        assert_eq!(
            completion.returned.candidate.as_ref().unwrap().0.as_ptr(),
            candidate_address
        );
        assert_eq!(
            completion.returned.output.as_ref().unwrap().0.as_ptr(),
            output_address
        );
        assert_eq!(completion.phase, TransactionPhase::RollbackReturnedOk);
        assert!(completion.rollback.as_ref().unwrap().is_ok());
        assert_eq!(&*log.borrow(), &["transaction", "pool"]);
    }

    #[test]
    fn committed_data_stays_ok_when_later_source_and_scope_refuse_publication() {
        let scope = Attempt::default();
        let log = Rc::new(RefCell::new(Vec::new()));
        let outer = resource(&scope, &log, "outer");
        let mut state = state();
        state.pool = Some(resource(&scope, &log, "pool"));
        state.phase = TransactionPhase::CommitReturnedOk;
        let returned = Row("committed output".into());
        let address = returned.0.as_ptr();
        let future = retain_database_result(
            Ok::<_, Body>(returned),
            finish_database_with(
                state,
                |_| std::future::ready(Ok::<(), Body>(())),
                |pool| async {
                    release(pool);
                },
            ),
        );
        let mut future = std::pin::pin!(future);
        let (original, completion) = match poll_once(future.as_mut()) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("ordinary ready completion"),
        };
        assert!(completion.api_allows_publication());
        assert_eq!(scope.report().live(), 1);
        scope.gap(Gap::Abandoned);
        release(outer);
        let invocation = RawInvocation::new(&scope);
        let local = invocation.settle_local_in(&scope, Ok::<(), std::convert::Infallible>(()));
        let terminal = scope.finish(Ok::<(), std::convert::Infallible>(()));
        assert_eq!(original.as_ref().ok().unwrap().0.as_ptr(), address);
        assert!(!execution_publishable(&original, false, &local, &terminal));
        assert_eq!(&*log.borrow(), &["pool", "outer"]);
    }

    #[test]
    fn cleanup_cancellation_marks_gap_without_normal_api_receipt_or_outer_finish() {
        let scope = Attempt::default();
        let captured = scope.clone();
        let log = Rc::new(RefCell::new(Vec::new()));
        let outer = resource(&scope, &log, "outer");
        let mut state = state();
        state.pool = Some(resource(&scope, &log, "pool-fallback"));
        state.phase = TransactionPhase::CommitReturnedOk;
        let ready = Rc::new(Cell::new(false));
        let close_ready = Rc::clone(&ready);
        let normal = Cell::new(0);
        let future = async {
            let mut unwind = ExecutionUnwind {
                attempt: captured.clone(),
                armed: true,
            };
            let completed = retain_database_result(
                Ok::<_, Body>(Row("returned".into())),
                finish_database_with(
                    state,
                    |_| std::future::ready(Ok::<(), Body>(())),
                    |pool| async {
                        Gate(close_ready).await;
                        release(pool);
                    },
                ),
            )
            .await;
            normal.set(normal.get() + 1);
            unwind.armed = false;
            completed
        };
        let mut future = Box::pin(future);
        assert!(poll_once(future.as_mut()).is_pending());
        drop(future);
        assert_eq!(normal.get(), 0);
        assert!(!scope.check());
        assert_eq!(scope.report().live(), 1);
        // Counted fallback is a model close, NOT a pool API await receipt.
        assert_eq!(&*log.borrow(), &["pool-fallback"]);
        release(outer);
        assert_eq!(&*log.borrow(), &["pool-fallback", "outer"]);
    }

    #[test]
    fn cancelled_commit_has_no_returned_outcome_or_returned_transaction() {
        let scope = Attempt::default();
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut transaction = Some(resource(&scope, &log, "transaction-fallback"));
        let mut phase = TransactionPhase::Open;
        let ready = Rc::new(Cell::new(false));
        let returned = Cell::new(0);
        let future = commit_returned(&mut transaction, &mut phase, |tx| async {
            Gate(ready).await;
            release(tx);
            returned.set(returned.get() + 1);
            Ok::<(), Body>(())
        });
        let mut future = Box::pin(future);
        assert!(poll_once(future.as_mut()).is_pending());
        drop(future);
        assert!(transaction.is_none());
        assert_eq!(phase, TransactionPhase::CommitStarted);
        assert_eq!(returned.get(), 0);
        assert_eq!(&*log.borrow(), &["transaction-fallback"]);
        // Neither commitOk nor commitErr nor application rollback was observed.
    }

    #[test]
    fn api_debug_and_nonclone_error_data_remain_bounded() {
        let completion = DatabaseCompletion {
            phase: TransactionPhase::RollbackReturnedErr,
            rollback: Some(Err(Body("private rollback prose".into()))),
            pool_close: PoolCloseObservation::AwaitReturned,
            returned: ReturnedDatabaseValues {
                candidate: Some(Row("private candidate".into())),
                output: Some(Row("private output".into())),
            },
        };
        let text = format!("{completion:?}");
        assert!(text.contains("AwaitReturned"));
        assert!(!text.contains("private"));
        assert!(text.len() < 300);
        assert!(!completion.api_allows_publication());
    }
}
// END C17 shared async completion tests

#[cfg(test)]
mod source_completion_tests {
    use super::*;
    use market_data::checked_close::{Close, Errno, Role};
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };
    struct Counted(Rc<Cell<usize>>, Rc<RefCell<Vec<&'static str>>>);
    impl Close for Counted {
        fn close(self) -> std::result::Result<(), Errno> {
            self.0.set(self.0.get() + 1);
            self.1.borrow_mut().push("close");
            Err(Errno::Interrupted)
        }
    }
    struct Body(Box<str>);
    struct Row(Box<str>);

    #[test]
    fn helper_committed_ok_survives_late_source_close() {
        let attempt = Attempt::default();
        let count = Rc::new(Cell::new(0));
        let log = Rc::new(RefCell::new(Vec::new()));
        let owner = attempt.adopt(Counted(count.clone(), log.clone()), Role::Source);
        // Ordinary effect log, not a fabricated DB transaction or native reader.
        log.borrow_mut().push("committed");
        let original = Ok::<_, Body>(Row("committed value".into()));
        let (original, _) = retain_execution_result(original, || owner.finish());
        let invocation = RawInvocation::new(&attempt);
        let local = invocation.settle_local_in(&attempt, Ok::<(), std::convert::Infallible>(()));
        let terminal = attempt.finish(Ok::<(), std::convert::Infallible>(()));
        assert_eq!(&*original.as_ref().ok().unwrap().0, "committed value");
        assert_eq!(&*log.borrow(), &["committed", "close"]);
        assert_eq!(count.get(), 1);
        assert!(!execution_publishable(&original, false, &local, &terminal));
        assert!(local.is_err() && terminal.is_err());
    }

    #[test]
    fn helper_failure_before_or_after_admission_retains_whole_objects() {
        for failed in [false, true] {
            let attempt = Attempt::default();
            let count = Rc::new(Cell::new(0));
            let log = Rc::new(RefCell::new(Vec::new()));
            let owner = attempt.adopt(Counted(count.clone(), log.clone()), Role::Source);
            let retained = Row("ordinary prior channel".into());
            let original = {
                let body = async {
                    std::future::ready(()).await;
                    assert_eq!(attempt.report().live(), 1);
                    if failed {
                        Err(Body("original error".into()))
                    } else {
                        Ok(Row("original success".into()))
                    }
                };
                let mut body = std::pin::pin!(body);
                let mut context = std::task::Context::from_waker(std::task::Waker::noop());
                match std::future::Future::poll(body.as_mut(), &mut context) {
                    std::task::Poll::Ready(value) => value,
                    std::task::Poll::Pending => panic!("ordinary ready future"),
                }
            };
            let (original, _) = retain_execution_result(original, || owner.finish());
            assert_eq!(&*retained.0, "ordinary prior channel");
            match original {
                Err(body) => {
                    assert!(failed);
                    assert_eq!(&*body.0, "original error");
                }
                Ok(row) => {
                    assert!(!failed);
                    assert_eq!(&*row.0, "original success");
                }
            }
            assert_eq!(count.get(), 1);
            assert!(
                attempt
                    .finish(Ok::<(), std::convert::Infallible>(()))
                    .is_err()
            );
        }
    }
}
#[cfg(test)]
#[cfg(target_os = "linux")]
mod database_native_component_tests {
    use super::*;
    use sqlx::postgres::{PgConnectOptions, PgSslMode};
    use std::{
        fs::{Metadata, OpenOptions},
        os::unix::fs::{MetadataExt, OpenOptionsExt},
    };

    const FIXTURE_RUNTIME: &str = "/data/worktrees/3puw275b/modern-anaconda/target/r5-paper/coordinator/r5-canonical-reader-integration-20261102/C18-db-api-component/runtime";
    const FIXTURE_IMAGE: &str =
        "postgres@sha256:3a82e1f56c8f0f5616a11103ac3d47e632c3938698946a7ad26da0df1334744a";

    // This private input identifies only the separately admitted scratch fixture.
    // It cannot construct protected Source/Context/Raw authority.
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Fixture {
        schema: String,
        run_nonce: String,
        port: u16,
        database: String,
        username: String,
        password: String,
        image_digest: String,
        server_version_num: i32,
    }
    type NativeResult<T> = std::result::Result<T, HelperFailure>;

    fn lower_hex(value: &str, length: usize) -> bool {
        value.len() == length
            && value
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    }
    fn private_file(metadata: &Metadata, uid: u32) -> bool {
        metadata.is_file() && metadata.uid() == uid && metadata.mode() & 0o7777 == 0o600
    }
    fn fixture_parent(path: &Path, uid: u32) -> std::result::Result<(), ()> {
        let parent = path.parent().ok_or(())?;
        // Check each ancestor before any descendant metadata/open can follow it.
        for ancestor in parent.ancestors().collect::<Vec<_>>().into_iter().rev() {
            let metadata = std::fs::symlink_metadata(ancestor).map_err(|_| ())?;
            if !metadata.file_type().is_dir()
                || (ancestor == parent
                    && (metadata.uid() != uid || metadata.mode() & 0o7777 != 0o700))
            {
                return Err(());
            }
        }
        Ok(())
    }
    fn fixture() -> std::result::Result<Fixture, ()> {
        // These names are exclusions, never alternate connection inputs.
        for (name, _) in std::env::vars_os() {
            if name.to_str().is_some_and(|name| {
                name.starts_with("PG")
                    || name.starts_with("DB_")
                    || matches!(name, "DATABASE_URL" | "DATABASE_URL_FILE")
            }) {
                return Err(());
            }
        }
        let nonce = std::env::var("R5_C18_DB_RUN_NONCE").map_err(|_| ())?;
        if !lower_hex(&nonce, 32) {
            return Err(());
        }
        let supplied = std::env::var("R5_C18_DB_ADMISSION_FILE").map_err(|_| ())?;
        let expected = format!("{FIXTURE_RUNTIME}/{nonce}/fixture.json");
        if supplied != expected || !supplied.is_ascii() {
            return Err(());
        }
        let path = Path::new(&supplied);
        // SAFETY: argument-free identity query for this fixture's actual owner.
        let uid = unsafe { libc::geteuid() };
        fixture_parent(path, uid)?;
        let named_before = std::fs::symlink_metadata(path).map_err(|_| ())?;
        if !private_file(&named_before, uid) {
            return Err(());
        }
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(path)
            .map_err(|_| ())?;
        let before = file.metadata().map_err(|_| ())?;
        if !private_file(&before, uid)
            || (before.dev(), before.ino()) != (named_before.dev(), named_before.ino())
        {
            return Err(());
        }
        let fixture: Fixture = serde_json::from_reader(&mut file).map_err(|_| ())?;
        let after = file.metadata().map_err(|_| ())?;
        let named_after = std::fs::symlink_metadata(path).map_err(|_| ())?;
        fixture_parent(path, uid)?;
        if !private_file(&after, uid)
            || !private_file(&named_after, uid)
            || (
                before.dev(),
                before.ino(),
                before.len(),
                before.mtime(),
                before.mtime_nsec(),
                before.ctime(),
                before.ctime_nsec(),
            ) != (
                after.dev(),
                after.ino(),
                after.len(),
                after.mtime(),
                after.mtime_nsec(),
                after.ctime(),
                after.ctime_nsec(),
            )
            || (after.dev(), after.ino()) != (named_after.dev(), named_after.ino())
            || fixture.schema != "r5-c18-db-fixture-v1"
            || fixture.run_nonce != nonce
            || fixture.port == 0
            || fixture.database != format!("r5_c18_{nonce}")
            || fixture.username != fixture.database
            || !lower_hex(&fixture.password, 64)
            || fixture.image_digest != FIXTURE_IMAGE
            || fixture.server_version_num != 180004
        {
            return Err(());
        }
        // File Drop is not reported as an observed native-close success.
        Ok(fixture)
    }

    async fn begin_fixture(db: &mut HelperDbState, fixture: &Fixture) -> NativeResult<()> {
        let options = PgConnectOptions::new()
            .host("127.0.0.1")
            .port(fixture.port)
            .database(&fixture.database)
            .username(&fixture.username)
            .password(&fixture.password)
            .ssl_mode(PgSslMode::Disable);
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(5))
            .connect_with(options)
            .await
            .map_err(|source| database_failure(DatabasePhase::Connect, source))?;
        db.pool = Some(pool);
        let transaction = db
            .pool
            .as_ref()
            .expect("actual returned fixture pool")
            .begin()
            .await
            .map_err(|source| database_failure(DatabasePhase::Begin, source))?;
        db.transaction = Some(transaction);
        db.phase = TransactionPhase::Open;
        let connection = &mut **db
            .transaction
            .as_mut()
            .expect("actual returned fixture transaction");
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .execute(&mut *connection)
            .await
            .map_err(|source| database_failure(DatabasePhase::Isolation, source))?;
        let identity: (
            String,
            String,
            i32,
            i64,
            Option<String>,
            bool,
            bool,
            bool,
            bool,
            bool,
        ) = sqlx::query_as(
            "SELECT current_database()::text, current_user::text, \
             current_setting('server_version_num')::integer, \
             (SELECT count(*) FROM public.r5_c18_fixture_identity), \
             (SELECT min(nonce) FROM public.r5_c18_fixture_identity), \
             rolsuper, rolcreatedb, rolcreaterole, rolreplication, rolbypassrls \
             FROM pg_roles WHERE rolname = current_user",
        )
        .fetch_one(&mut *connection)
        .await
        .map_err(|source| database_failure(DatabasePhase::Read, source))?;
        required(
            identity.0 == fixture.database
                && identity.1 == fixture.username
                && identity.2 == fixture.server_version_num
                && identity.3 == 1
                && identity.4.as_deref() == Some(fixture.run_nonce.as_str())
                && !identity.5
                && !identity.6
                && !identity.7
                && !identity.8
                && !identity.9,
        )?;
        Ok(())
    }

    async fn capture_value(db: &mut HelperDbState, query: &'static str) -> NativeResult<()> {
        let value: Value = sqlx::query_scalar(query)
            .fetch_one(
                &mut **db
                    .transaction
                    .as_mut()
                    .expect("actual returned fixture transaction"),
            )
            .await
            .map_err(|source| database_failure(DatabasePhase::Read, source))?;
        retain_returned_value(&mut db.returned.output, value);
        Ok(())
    }
    fn marker_allocation(value: Option<&Value>) -> Option<*const u8> {
        value.and_then(|v| v["marker"].as_str()).map(str::as_ptr)
    }
    fn database_error_allocation(
        failure: &HelperFailure,
        phase: DatabasePhase,
    ) -> Option<*const DatabaseFailure> {
        match failure {
            HelperFailure::Database(source)
                if source.phase == phase && matches!(&source._source, sqlx::Error::Database(_)) =>
            {
                Some(std::ptr::from_ref(source.as_ref()))
            }
            _ => None,
        }
    }
    fn admitted_fixture() -> Fixture {
        match fixture() {
            Ok(fixture) => fixture,
            Err(()) => panic!("R5_C18_FIXTURE_REJECTED"),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    #[ignore = "requires separately admitted private C18 loopback fixture"]
    async fn genuine_commit_ok_and_pool_await() {
        let fixture = admitted_fixture();
        let mut db = HelperDbState::new();
        let mut returned_allocation = None;
        let mut commit_called = false;
        let original: NativeResult<Value> = async {
            begin_fixture(&mut db, &fixture).await?;
            capture_value(
                &mut db,
                "SELECT jsonb_build_object('marker', 'c18-commit-ok', 'count', 1)",
            )
            .await?;
            returned_allocation = marker_allocation(db.returned.output.as_ref());
            commit_called = true;
            commit_returned(&mut db.transaction, &mut db.phase, |tx| async move {
                tx.commit().await
            })
            .await
            .map_err(|source| database_failure(DatabasePhase::Commit, source))?;
            Ok(db
                .returned
                .output
                .take()
                .expect("actual returned fixture query value"))
        }
        .await;
        let transaction_absent = db.transaction.is_none();
        let (original, completion) = retain_database_result(original, finish_database(db)).await;
        // All ordinary setup/body/commit error paths have completed DB cleanup.
        assert!(commit_called && transaction_absent, "R5_C18_COMMIT_STATE");
        assert!(original.is_ok(), "R5_C18_COMMIT_NOT_OK");
        let value = original.as_ref().ok();
        assert!(
            value.is_some_and(|v| v == &json!({"marker": "c18-commit-ok", "count": 1})),
            "R5_C18_RETURNED_VALUE"
        );
        assert!(
            returned_allocation.is_some() && marker_allocation(value) == returned_allocation,
            "R5_C18_VALUE_MOVED"
        );
        assert!(
            completion.phase == TransactionPhase::CommitReturnedOk,
            "R5_C18_COMMIT_PHASE"
        );
        assert!(completion.rollback.is_none(), "R5_C18_UNEXPECTED_ROLLBACK");
        assert!(
            completion.returned.output.is_none(),
            "R5_C18_VALUE_DUPLICATED"
        );
        assert!(
            completion.pool_close == PoolCloseObservation::AwaitReturned,
            "R5_C18_POOL_AWAIT"
        );
        assert!(completion.api_allows_publication(), "R5_C18_API_REFUSAL");
        // This predicate is API-only, never full helper/Source/native-FD proof.
    }

    #[tokio::test(flavor = "current_thread")]
    #[ignore = "requires separately admitted private C18 loopback fixture"]
    async fn genuine_body_error_and_rollback() {
        let fixture = admitted_fixture();
        let mut db = HelperDbState::new();
        let mut body_query_started = false;
        let original: NativeResult<()> = async {
            begin_fixture(&mut db, &fixture).await?;
            capture_value(
                &mut db,
                "SELECT jsonb_build_object('marker', 'c18-body-error', 'count', 1)",
            )
            .await?;
            body_query_started = true;
            sqlx::query("SELECT 1/0")
                .execute(
                    &mut **db
                        .transaction
                        .as_mut()
                        .expect("actual returned fixture transaction"),
                )
                .await
                .map_err(|source| database_failure(DatabasePhase::Read, source))?;
            Ok(())
        }
        .await;
        let before_error = original
            .as_ref()
            .err()
            .and_then(|f| database_error_allocation(f, DatabasePhase::Read));
        let before_value = marker_allocation(db.returned.output.as_ref());
        let transaction_owned = db.transaction.is_some();
        let (original, completion) = retain_database_result(original, finish_database(db)).await;
        assert!(
            body_query_started && transaction_owned,
            "R5_C18_BODY_NOT_REACHED"
        );
        let after_error = original
            .as_ref()
            .err()
            .and_then(|f| database_error_allocation(f, DatabasePhase::Read));
        assert!(
            before_error.is_some() && after_error == before_error,
            "R5_C18_ORIGINAL_DATABASE_ERROR"
        );
        assert!(
            completion.phase == TransactionPhase::RollbackReturnedOk,
            "R5_C18_ROLLBACK_PHASE"
        );
        assert!(
            matches!(completion.rollback.as_ref(), Some(Ok(()))),
            "R5_C18_ROLLBACK_NOT_OK"
        );
        assert!(
            completion.pool_close == PoolCloseObservation::AwaitReturned,
            "R5_C18_POOL_AWAIT"
        );
        assert!(
            completion
                .returned
                .output
                .as_ref()
                .is_some_and(|v| v == &json!({"marker": "c18-body-error", "count": 1})),
            "R5_C18_BODY_VALUE"
        );
        assert!(
            before_value.is_some()
                && marker_allocation(completion.returned.output.as_ref()) == before_value,
            "R5_C18_VALUE_MOVED"
        );
        assert!(
            !completion.api_allows_publication(),
            "R5_C18_ERROR_PUBLISHED"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    #[ignore = "requires separately admitted private C18 loopback fixture"]
    async fn genuine_commit_error_consumes_transaction() {
        let fixture = admitted_fixture();
        let mut db = HelperDbState::new();
        let mut insert_succeeded = false;
        let mut commit_called = false;
        let original: NativeResult<()> = async {
            begin_fixture(&mut db, &fixture).await?;
            {
                let connection = &mut **db.transaction.as_mut().expect("actual returned fixture transaction");
                sqlx::query("CREATE TEMP TABLE r5_c18_deferred_unique(v integer, CONSTRAINT r5_c18_deferred UNIQUE(v) DEFERRABLE INITIALLY DEFERRED) ON COMMIT DROP")
                    .execute(&mut *connection)
                    .await
                    .map_err(|source| database_failure(DatabasePhase::Read, source))?;
                let inserted = sqlx::query("INSERT INTO r5_c18_deferred_unique(v) VALUES (1), (1)")
                    .execute(&mut *connection)
                    .await
                    .map_err(|source| database_failure(DatabasePhase::Read, source))?;
                insert_succeeded = inserted.rows_affected() == 2;
                required(insert_succeeded)?;
            }
            capture_value(&mut db, "SELECT jsonb_build_object('marker', 'c18-commit-error', 'rowcount', count(*)) FROM r5_c18_deferred_unique").await?;
            commit_called = true;
            commit_returned(&mut db.transaction, &mut db.phase, |tx| async move { tx.commit().await })
                .await
                .map_err(|source| database_failure(DatabasePhase::Commit, source))?;
            Ok(())
        }.await;
        let before_error = original
            .as_ref()
            .err()
            .and_then(|f| database_error_allocation(f, DatabasePhase::Commit));
        let before_value = marker_allocation(db.returned.output.as_ref());
        let transaction_absent = db.transaction.is_none();
        let (original, completion) = retain_database_result(original, finish_database(db)).await;
        assert!(
            insert_succeeded && commit_called && transaction_absent,
            "R5_C18_DEFERRED_COMMIT_STATE"
        );
        let after_error = original
            .as_ref()
            .err()
            .and_then(|f| database_error_allocation(f, DatabasePhase::Commit));
        assert!(
            before_error.is_some() && after_error == before_error,
            "R5_C18_ORIGINAL_COMMIT_ERROR"
        );
        assert!(
            completion.phase == TransactionPhase::CommitReturnedErr,
            "R5_C18_COMMIT_ERROR_PHASE"
        );
        assert!(
            completion.rollback.is_none(),
            "R5_C18_APPLICATION_ROLLBACK_AFTER_COMMIT"
        );
        assert!(
            completion.pool_close == PoolCloseObservation::AwaitReturned,
            "R5_C18_POOL_AWAIT"
        );
        assert!(
            completion
                .returned
                .output
                .as_ref()
                .is_some_and(|v| v == &json!({"marker": "c18-commit-error", "rowcount": 2})),
            "R5_C18_COMMIT_ERROR_VALUE"
        );
        assert!(
            before_value.is_some()
                && marker_allocation(completion.returned.output.as_ref()) == before_value,
            "R5_C18_VALUE_MOVED"
        );
        assert!(
            !completion.api_allows_publication(),
            "R5_C18_COMMIT_ERROR_PUBLISHED"
        );
        // No application retry/rollback or inference about SDK/server rollback.
    }
}
