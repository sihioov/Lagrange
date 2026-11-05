//! One opt-in PostgreSQL witness for the real queue heartbeat/claim lock interaction.

#[path = "../../../../job-queue/tests/common/mod.rs"]
mod scratch_db;

use super::{ClaimHeartbeat, ClaimHeartbeatResult};
use chrono::{DateTime, SecondsFormat, Utc};
use job_queue::{HeartbeatStatus, JobQueue, QueueConfig, SubmitJob, error::QueueError};
use serde_json::{Value, json};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{PgPool, Postgres, Transaction};
use std::time::Duration;
use tokio::sync::mpsc;

const SUPERVISOR_URL: &str = "postgres://postgres:lagrange@127.0.0.1:55439/postgres";
const HEARTBEAT_PERIOD: Duration = Duration::from_secs(1);
const CLAIM_LEASE: Duration = Duration::from_secs(5);
const RETRY_BACKOFF: Duration = Duration::from_secs(1);

#[derive(Clone, Copy)]
struct WitnessFailure {
    primary: &'static str,
    cleanup: Option<&'static str>,
}

impl WitnessFailure {
    fn new(primary: &'static str) -> Self {
        Self {
            primary,
            cleanup: None,
        }
    }

    fn retain_cleanup(self, cleanup: &'static str) -> Self {
        Self {
            primary: self.primary,
            cleanup: Some(cleanup),
        }
    }
}

type WitnessResult<T> = Result<T, WitnessFailure>;

fn checked_nonce(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn checked_sqlstate(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    (bytes.len() == 5
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_digit() || byte.is_ascii_uppercase()))
    .then(|| value.to_owned())
}

fn safe_sqlstate(error: &QueueError) -> Option<String> {
    let QueueError::Database(sqlx::Error::Database(database)) = error else {
        return None;
    };
    let code = database.code()?;
    checked_sqlstate(&code)
}

fn stamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Nanos, true)
}

async fn connect_pool(
    url: &str,
    application_name: &str,
    max_connections: u32,
) -> WitnessResult<PgPool> {
    let options: PgConnectOptions = url
        .parse()
        .map_err(|_| WitnessFailure::new("connection_options"))?;
    PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(Duration::from_secs(5))
        .connect_with(options.application_name(application_name))
        .await
        .map_err(|_| WitnessFailure::new("connection_open"))
}

async fn verify_supervisor(pool: &PgPool, nonce: &str) -> WitnessResult<()> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT nonce::text, current_user::text FROM public.r5_claim_witness_identity",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| WitnessFailure::new("supervisor_identity_query"))?;
    if rows.len() != 1 || rows[0].0 != nonce || rows[0].1 != "postgres" {
        return Err(WitnessFailure::new("supervisor_identity_mismatch"));
    }
    Ok(())
}

async fn verify_direct_worker(pool: &PgPool) -> WitnessResult<()> {
    let (current_user, session_user): (String, String) =
        sqlx::query_as("SELECT current_user::text, session_user::text")
            .fetch_one(pool)
            .await
            .map_err(|_| WitnessFailure::new("worker_identity_query"))?;
    if current_user != "worker" || session_user != "worker" {
        return Err(WitnessFailure::new("worker_identity_mismatch"));
    }
    Ok(())
}

async fn backend_start(pool: &PgPool, pid: i32) -> WitnessResult<DateTime<Utc>> {
    sqlx::query_scalar(
        "SELECT backend_start FROM pg_stat_activity WHERE pid = $1 AND datname = current_database()",
    )
    .bind(pid)
    .fetch_one(pool)
    .await
    .map_err(|_| WitnessFailure::new("backend_identity_missing"))
}

async fn await_initial_extension(
    receiver: &mut mpsc::Receiver<ClaimHeartbeatResult>,
) -> WitnessResult<()> {
    match tokio::time::timeout(Duration::from_secs(4), receiver.recv())
        .await
        .map_err(|_| WitnessFailure::new("initial_heartbeat_timeout"))?
    {
        Some(result) if matches!(result.as_ref(), Ok(HeartbeatStatus::Extended)) => Ok(()),
        _ => Err(WitnessFailure::new("initial_heartbeat_not_extended")),
    }
}

async fn observe_blocker(
    monitor: &PgPool,
    heartbeat_pid: i32,
    heartbeat_start: DateTime<Utc>,
    owner_pid: i32,
    owner_start: DateTime<Utc>,
    heartbeat_application: &str,
    database_name: &str,
) -> WitnessResult<bool> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    loop {
        let row: Option<(Option<String>, bool)> = sqlx::query_as(
            "SELECT wait_event_type::text,
                    $1 = ANY(pg_blocking_pids(pid))
             FROM pg_stat_activity
             WHERE pid = $2 AND application_name = $3 AND datname = $4
               AND backend_start = $5
               AND EXISTS (
                   SELECT 1 FROM pg_stat_activity AS owner
                   WHERE owner.pid = $1 AND owner.backend_start = $6
                     AND owner.datname = $4
               )",
        )
        .bind(owner_pid)
        .bind(heartbeat_pid)
        .bind(heartbeat_application)
        .bind(database_name)
        .bind(heartbeat_start)
        .bind(owner_start)
        .fetch_optional(monitor)
        .await
        .map_err(|_| WitnessFailure::new("blocker_query"))?;
        if matches!(row, Some((Some(ref event), true)) if event == "Lock") {
            return Ok(true);
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(false);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn await_lost_outcome(
    receiver: &mut mpsc::Receiver<ClaimHeartbeatResult>,
) -> WitnessResult<(String, Option<String>)> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(WitnessFailure::new("blocked_heartbeat_timeout"));
        }
        let result = tokio::time::timeout(remaining, receiver.recv())
            .await
            .map_err(|_| WitnessFailure::new("blocked_heartbeat_timeout"))?
            .ok_or(WitnessFailure::new("heartbeat_observer_closed"))?;
        match result.as_ref() {
            Ok(HeartbeatStatus::Extended) => continue,
            Ok(HeartbeatStatus::Canceled) => return Ok(("canceled".to_owned(), None)),
            Ok(HeartbeatStatus::LeaseLost) => return Ok(("lease_lost".to_owned(), None)),
            Err(error) => {
                let outcome = match error {
                    QueueError::Database(_) => "database_error",
                    other => other.event_code(),
                };
                return Ok((outcome.to_owned(), safe_sqlstate(error)));
            }
        }
    }
}

async fn seed_job(
    monitor: &PgPool,
    owner_queue: &JobQueue,
    nonce: &str,
) -> WitnessResult<job_queue::ClaimedJob> {
    let subject = format!("r5-claim-{nonce}");
    let owner: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) \
         VALUES ('test-issuer', $1, $2) RETURNING id",
    )
    .bind(&subject)
    .bind(format!("{subject}@example.com"))
    .fetch_one(monitor)
    .await
    .map_err(|_| WitnessFailure::new("test_user_seed"))?;
    // Seeding uses the existing fixture administrator, as queue_contract does.
    // Both actual claim/lock and heartbeat still use direct worker logins.
    let submit_queue = JobQueue::new(monitor.clone(), None, *owner_queue.config());
    let submitted = submit_queue
        .submit(SubmitJob {
            owner_user_id: owner,
            job_type: "backtest".to_owned(),
            payload: json!({ "tag": 1 }),
            priority: 5,
            idempotency_key: Some(format!("r5-claim-witness-{nonce}")),
            max_attempts: 1,
            available_at: None,
        })
        .await
        .map_err(|_| WitnessFailure::new("queue_submit"))?;
    owner_queue
        .claim_next(&format!("r5-claim-{nonce}"))
        .await
        .map_err(|_| WitnessFailure::new("queue_claim"))?
        .filter(|claim| claim.job.id == submitted.id)
        .ok_or(WitnessFailure::new("queue_claim_missing"))
}

async fn run_claim_witness<'queue>(
    nonce: &str,
    monitor: &PgPool,
    owner_queue: &'queue JobQueue,
    heartbeat_queue: &JobQueue,
    heartbeat: &mut Option<ClaimHeartbeat>,
    transaction: &mut Option<Transaction<'queue, Postgres>>,
) -> WitnessResult<Value> {
    let claim = seed_job(monitor, owner_queue, nonce).await?;
    let worker_id = claim.worker_id.clone();
    let (observer_tx, mut observer_rx) = mpsc::channel(8);
    *heartbeat = Some(ClaimHeartbeat::start_observed(
        heartbeat_queue.clone(),
        claim.clone(),
        HEARTBEAT_PERIOD,
        observer_tx,
    ));
    await_initial_extension(&mut observer_rx).await?;

    let initial_locked_at: DateTime<Utc> = sqlx::query_scalar(
        "SELECT locked_at FROM jobs WHERE id = $1 AND status = 'RUNNING' AND locked_by = $2",
    )
    .bind(claim.job.id)
    .bind(&worker_id)
    .fetch_one(monitor)
    .await
    .map_err(|_| WitnessFailure::new("initial_claim_readback"))?;

    let heartbeat_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&heartbeat_queue.pool())
        .await
        .map_err(|_| WitnessFailure::new("heartbeat_backend_identity"))?;
    let heartbeat_start = backend_start(monitor, heartbeat_pid).await?;
    let heartbeat_application = format!("r5-claim-heartbeat-{nonce}");
    let database_name: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(monitor)
        .await
        .map_err(|_| WitnessFailure::new("scratch_database_identity"))?;

    *transaction = Some(
        owner_queue
            .begin()
            .await
            .map_err(|_| WitnessFailure::new("owner_transaction_begin"))?,
    );
    let tx = transaction
        .as_mut()
        .ok_or(WitnessFailure::new("owner_transaction_missing"))?;
    let (owner_pid, transaction_time, wall_time): (i32, DateTime<Utc>, DateTime<Utc>) =
        sqlx::query_as("SELECT pg_backend_pid(), transaction_timestamp(), clock_timestamp()")
            .fetch_one(&mut **tx)
            .await
            .map_err(|_| WitnessFailure::new("owner_transaction_clock"))?;
    let owner_start = backend_start(monitor, owner_pid).await?;
    owner_queue
        .lock_claim_in(tx, &claim)
        .await
        .map_err(|_| WitnessFailure::new("exact_attempt_lock"))?;
    let (transaction_time_after_lock, wall_time_after_lock): (DateTime<Utc>, DateTime<Utc>) =
        sqlx::query_as("SELECT transaction_timestamp(), clock_timestamp()")
            .fetch_one(&mut **tx)
            .await
            .map_err(|_| WitnessFailure::new("owner_transaction_clock_after_lock"))?;

    let blocker_observed = observe_blocker(
        monitor,
        heartbeat_pid,
        heartbeat_start,
        owner_pid,
        owner_start,
        &heartbeat_application,
        &database_name,
    )
    .await?;
    if !blocker_observed {
        return Err(WitnessFailure::new("heartbeat_blocker_not_observed"));
    }
    let (heartbeat_outcome, heartbeat_sqlstate) = await_lost_outcome(&mut observer_rx).await?;
    let lost = heartbeat
        .as_ref()
        .is_some_and(|active| *active.lost.borrow());
    if !lost || heartbeat_outcome == "extended" {
        return Err(WitnessFailure::new("sticky_loss_mapping_mismatch"));
    }

    let (owner_transaction_time_after_wait, owner_clock_time_after_wait): (
        DateTime<Utc>,
        DateTime<Utc>,
    ) = sqlx::query_as("SELECT transaction_timestamp(), clock_timestamp()")
        .fetch_one(&mut **tx)
        .await
        .map_err(|_| WitnessFailure::new("owner_clock_after_wait"))?;

    let (last_locked_at, database_transaction_time, database_clock_time, status, attempt_outcome): (
        DateTime<Utc>,
        DateTime<Utc>,
        DateTime<Utc>,
        String,
        String,
    ) = sqlx::query_as(
        "SELECT jobs.locked_at, transaction_timestamp(), clock_timestamp(), jobs.status::text, \
                job_attempts.outcome::text \
         FROM jobs JOIN job_attempts ON job_attempts.job_id = jobs.id \
         WHERE jobs.id = $1 AND job_attempts.id = $2",
    )
    .bind(claim.job.id)
    .bind(claim.attempt.id)
    .fetch_one(monitor)
    .await
    .map_err(|_| WitnessFailure::new("post_wait_claim_readback"))?;
    if status != "RUNNING" || attempt_outcome != "RUNNING" || last_locked_at != initial_locked_at {
        return Err(WitnessFailure::new("claim_changed_during_witness"));
    }

    Ok(json!({
        "nonce": nonce,
        "job_id": claim.job.id,
        "attempt_id": claim.attempt.id,
        "attempt_no": claim.attempt.attempt_no,
        "worker_id": worker_id,
        "heartbeat_backend_pid": heartbeat_pid,
        "heartbeat_backend_start": stamp(heartbeat_start),
        "owner_backend_pid": owner_pid,
        "owner_backend_start": stamp(owner_start),
        "transaction_timestamp": stamp(transaction_time),
        "owner_clock_timestamp": stamp(wall_time),
        "transaction_timestamp_after_lock": stamp(transaction_time_after_lock),
        "owner_clock_timestamp_after_lock": stamp(wall_time_after_lock),
        "transaction_timestamp_after_wait": stamp(owner_transaction_time_after_wait),
        "owner_clock_timestamp_after_wait": stamp(owner_clock_time_after_wait),
        "transaction_timestamp_stable": transaction_time == transaction_time_after_lock
            && transaction_time == owner_transaction_time_after_wait,
        "last_locked_at_before_wait": stamp(initial_locked_at),
        "last_locked_at_after_wait": stamp(last_locked_at),
        "monitor_transaction_timestamp": stamp(database_transaction_time),
        "monitor_clock_timestamp": stamp(database_clock_time),
        "heartbeat_outcome": heartbeat_outcome,
        "heartbeat_sqlstate": heartbeat_sqlstate,
        "sticky_loss": lost,
        "blocker_observed": blocker_observed,
        "queue_status_after_wait": status,
        "attempt_outcome_after_wait": attempt_outcome,
        "lease_seconds": CLAIM_LEASE.as_secs(),
        "lease_elapsed_at_final_readback": database_clock_time
            >= last_locked_at + chrono::Duration::seconds(CLAIM_LEASE.as_secs() as i64),
        "transaction_stable_clock_is_owner_transaction":
            owner_transaction_time_after_wait == transaction_time,
        "lease_live_at_end_claimed": false,
        "supervisor_pool_closed": true,
        "heartbeat_pool_closed": true,
        "owner_pool_closed": true,
        "heartbeat_stopped_and_joined": true,
        "owning_transaction_rolled_back": true,
        "scratch_database_dropped": true
    }))
}

async fn execute_witness() -> WitnessResult<Value> {
    execute_witness_case(false).await
}

async fn execute_witness_case(handoff: bool) -> WitnessResult<Value> {
    let nonce = std::env::var("R5_CLAIM_WITNESS_NONCE")
        .map_err(|_| WitnessFailure::new("nonce_required"))?;
    if !checked_nonce(&nonce) {
        return Err(WitnessFailure::new("nonce_invalid"));
    }
    let database_url =
        std::env::var("DATABASE_URL").map_err(|_| WitnessFailure::new("database_url_required"))?;
    if database_url != SUPERVISOR_URL {
        return Err(WitnessFailure::new("database_url_not_admitted"));
    }

    let supervisor = connect_pool(&database_url, "r5-claim-supervisor-preflight", 1).await?;
    if let Err(failure) = verify_supervisor(&supervisor, &nonce).await {
        supervisor.close().await;
        return Err(failure);
    }

    let Some(scratch) = scratch_db::ScratchDb::create().await else {
        supervisor.close().await;
        return Err(WitnessFailure::new("scratch_database_not_created"));
    };
    let scratch_url = scratch.role_url("postgres");
    let database_name: String = match sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&scratch.pool)
        .await
    {
        Ok(name) => name,
        Err(_) => {
            scratch.drop_db().await;
            supervisor.close().await;
            return Err(WitnessFailure::new("scratch_database_name"));
        }
    };
    let monitor = match connect_pool(&scratch_url, &format!("r5-claim-monitor-{nonce}"), 2).await {
        Ok(pool) => pool,
        Err(failure) => {
            scratch.drop_db().await;
            supervisor.close().await;
            return Err(failure);
        }
    };
    let heartbeat_application = format!("r5-claim-heartbeat-{nonce}");
    let heartbeat_pool =
        match connect_pool(&scratch.role_url("worker"), &heartbeat_application, 1).await {
            Ok(pool) => pool,
            Err(failure) => {
                monitor.close().await;
                scratch.drop_db().await;
                supervisor.close().await;
                return Err(failure);
            }
        };
    let owner_pool = match connect_pool(
        &scratch.role_url("worker"),
        &format!("r5-claim-owner-{nonce}"),
        1,
    )
    .await
    {
        Ok(pool) => pool,
        Err(failure) => {
            monitor.close().await;
            heartbeat_pool.close().await;
            scratch.drop_db().await;
            supervisor.close().await;
            return Err(failure);
        }
    };
    let worker_identity_result = match verify_direct_worker(&heartbeat_pool).await {
        Ok(()) => verify_direct_worker(&owner_pool).await,
        Err(failure) => Err(failure),
    };
    if let Err(failure) = worker_identity_result {
        monitor.close().await;
        heartbeat_pool.close().await;
        owner_pool.close().await;
        scratch.drop_db().await;
        supervisor.close().await;
        return Err(failure);
    }

    let config = QueueConfig {
        lease: CLAIM_LEASE,
        backoff_base: RETRY_BACKOFF,
    };
    let heartbeat_queue = JobQueue::new(heartbeat_pool.clone(), None, config);
    let owner_queue = JobQueue::new(owner_pool.clone(), None, config);
    let mut active_heartbeat = None;
    let mut owning_transaction: Option<Transaction<'_, Postgres>> = None;
    let body = if handoff {
        run_transaction_handoff_witness(
            &nonce,
            &monitor,
            &owner_queue,
            &heartbeat_queue,
            &mut active_heartbeat,
            &mut owning_transaction,
        )
        .await
    } else {
        run_claim_witness(
            &nonce,
            &monitor,
            &owner_queue,
            &heartbeat_queue,
            &mut active_heartbeat,
            &mut owning_transaction,
        )
        .await
    };

    let mut cleanup_failure: Option<WitnessFailure> = None;
    let rollback_ok = if let Some(transaction) = owning_transaction.take() {
        transaction.rollback().await.is_ok()
    } else {
        true
    };
    if !rollback_ok {
        cleanup_failure = Some(WitnessFailure::new("owner_transaction_rollback"));
    }
    if let Some(heartbeat) = active_heartbeat.take() {
        heartbeat.stop().await;
        if !heartbeat.joined().await {
            cleanup_failure.get_or_insert(WitnessFailure::new("heartbeat_join"));
        }
    }
    monitor.close().await;
    heartbeat_pool.close().await;
    owner_pool.close().await;
    scratch.drop_db().await;
    let remaining_database: WitnessResult<i64> =
        sqlx::query_scalar("SELECT count(*) FROM pg_database WHERE datname = $1")
            .bind(&database_name)
            .fetch_one(&supervisor)
            .await
            .map_err(|_| WitnessFailure::new("scratch_drop_verification"));
    supervisor.close().await;
    let remaining_database = match remaining_database {
        Ok(0) => true,
        Ok(_) => {
            cleanup_failure.get_or_insert(WitnessFailure::new("scratch_database_not_dropped"));
            false
        }
        Err(failure) => {
            cleanup_failure.get_or_insert(failure);
            false
        }
    };
    let mut report = match (body, cleanup_failure) {
        (Ok(report), None) => report,
        (Err(primary), Some(cleanup)) => {
            return Err(primary.retain_cleanup(cleanup.primary));
        }
        (Err(primary), None) => return Err(primary),
        (Ok(_), Some(cleanup)) => return Err(cleanup),
    };
    report["supervisor_pool_closed"] = json!(true);
    report["heartbeat_pool_closed"] = json!(true);
    report["owner_pool_closed"] = json!(true);
    report["heartbeat_stopped_and_joined"] = json!(active_heartbeat.is_none());
    report["owning_transaction_rolled_back"] = json!(rollback_ok);
    report["scratch_database_dropped"] = json!(remaining_database);
    Ok(report)
}

async fn financial_row_counts(pool: &PgPool) -> WitnessResult<(i64, i64, i64, i64)> {
    sqlx::query_as(
        "SELECT (SELECT count(*) FROM public.owner_beta_paper_lineage),
                (SELECT count(*) FROM public.owner_beta_paper_sessions),
                (SELECT count(*) FROM public.paper_rebalance_previews),
                (SELECT count(*) FROM public.cash_ledger)",
    )
    .fetch_one(pool)
    .await
    .map_err(|_| WitnessFailure::new("financial_row_counts"))
}

async fn run_transaction_handoff_witness<'queue>(
    nonce: &str,
    monitor: &PgPool,
    owner_queue: &'queue JobQueue,
    heartbeat_queue: &JobQueue,
    heartbeat: &mut Option<ClaimHeartbeat>,
    transaction: &mut Option<Transaction<'queue, Postgres>>,
) -> WitnessResult<Value> {
    let financial_before = financial_row_counts(monitor).await?;
    let claim = seed_job(monitor, owner_queue, nonce).await?;
    let (observer_tx, mut observer_rx) = mpsc::channel(8);
    *heartbeat = Some(ClaimHeartbeat::start_observed(
        heartbeat_queue.clone(),
        claim.clone(),
        HEARTBEAT_PERIOD,
        observer_tx,
    ));
    await_initial_extension(&mut observer_rx).await?;
    let active = heartbeat
        .as_ref()
        .ok_or(WitnessFailure::new("heartbeat_missing"))?;
    active
        .quiesce()
        .await
        .map_err(|_| WitnessFailure::new("handoff_quiesce_refused"))?;
    if !active.joined().await || *active.lost.borrow() {
        return Err(WitnessFailure::new("handoff_join_not_acknowledged"));
    }
    let persisted_anchor: DateTime<Utc> =
        sqlx::query_scalar("SELECT locked_at FROM jobs WHERE id = $1")
            .bind(claim.job.id)
            .fetch_one(monitor)
            .await
            .map_err(|_| WitnessFailure::new("handoff_persisted_anchor"))?;
    *transaction = Some(
        owner_queue
            .begin()
            .await
            .map_err(|_| WitnessFailure::new("handoff_begin"))?,
    );
    let tx = transaction
        .as_mut()
        .ok_or(WitnessFailure::new("handoff_transaction_missing"))?;
    let mut lease = owner_queue
        .lock_claim_transaction_lease_in(tx, &claim)
        .await
        .map_err(|_| WitnessFailure::new("handoff_live_acquisition"))?;
    let (owner_pid, owner_tx_time, started): (i32, DateTime<Utc>, DateTime<Utc>) =
        sqlx::query_as("SELECT pg_backend_pid(), transaction_timestamp(), clock_timestamp()")
            .fetch_one(&mut **tx)
            .await
            .map_err(|_| WitnessFailure::new("handoff_owner_identity"))?;
    let owner_start = backend_start(monitor, owner_pid).await?;
    let anchor_before_foreign: DateTime<Utc> =
        sqlx::query_scalar("SELECT locked_at FROM jobs WHERE id = $1")
            .bind(claim.job.id)
            .fetch_one(&mut **tx)
            .await
            .map_err(|_| WitnessFailure::new("handoff_anchor_before_foreign"))?;

    // This other backend must refuse BEFORE attempting the held job row lock.
    let mut foreign = heartbeat_queue
        .begin()
        .await
        .map_err(|_| WitnessFailure::new("foreign_begin"))?;
    let foreign_pid: Result<i32, sqlx::Error> = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *foreign)
        .await;
    let foreign_refusal = owner_queue
        .checkpoint_claim_transaction_lease_in(&mut foreign, &mut lease)
        .await;
    let foreign_rollback = foreign.rollback().await;
    let foreign_pid = foreign_pid.map_err(|_| WitnessFailure::new("foreign_backend"))?;
    if foreign_rollback.is_err() {
        return Err(WitnessFailure::new("foreign_rollback"));
    }
    if foreign_pid == owner_pid
        || !matches!(foreign_refusal, Err(QueueError::StaleClaim(id)) if id == claim.job.id)
    {
        return Err(WitnessFailure::new("foreign_transaction_not_refused"));
    }
    let anchor_after_foreign: DateTime<Utc> =
        sqlx::query_scalar("SELECT locked_at FROM jobs WHERE id = $1")
            .bind(claim.job.id)
            .fetch_one(&mut **tx)
            .await
            .map_err(|_| WitnessFailure::new("handoff_anchor_after_foreign"))?;
    if anchor_before_foreign != anchor_after_foreign {
        return Err(WitnessFailure::new("foreign_transaction_mutated_claim"));
    }

    let mut samples = Vec::new();
    for _ in 0..2 {
        tokio::time::sleep(Duration::from_secs(3)).await;
        owner_queue
            .checkpoint_claim_transaction_lease_in(tx, &mut lease)
            .await
            .map_err(|_| WitnessFailure::new("live_transaction_checkpoint"))?;
        let (anchor, now, tx_time): (DateTime<Utc>, DateTime<Utc>, DateTime<Utc>) = sqlx::query_as(
            "SELECT locked_at, clock_timestamp(), transaction_timestamp() FROM jobs WHERE id = $1",
        )
        .bind(claim.job.id)
        .fetch_one(&mut **tx)
        .await
        .map_err(|_| WitnessFailure::new("handoff_checkpoint_readback"))?;
        if tx_time != owner_tx_time || now >= anchor + chrono::Duration::seconds(5) {
            return Err(WitnessFailure::new("handoff_checkpoint_clock_mismatch"));
        }
        samples.push(json!({"locked_at": stamp(anchor), "clock_timestamp": stamp(now), "transaction_timestamp": stamp(tx_time)}));
    }
    let finished: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut **tx)
        .await
        .map_err(|_| WitnessFailure::new("handoff_final_clock"))?;
    if finished - started <= chrono::Duration::seconds(5) {
        return Err(WitnessFailure::new("handoff_total_not_beyond_lease"));
    }
    transaction
        .take()
        .ok_or(WitnessFailure::new("handoff_rollback_missing"))?
        .rollback()
        .await
        .map_err(|_| WitnessFailure::new("handoff_live_rollback"))?;

    // Reusing the same backend for a NEW transaction must also refuse the
    // old opaque token, independently of the backend-only refusal above.
    *transaction = Some(
        owner_queue
            .begin()
            .await
            .map_err(|_| WitnessFailure::new("reused_backend_begin"))?,
    );
    let next = transaction
        .as_mut()
        .ok_or(WitnessFailure::new("reused_backend_missing"))?;
    let next_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut **next)
        .await
        .map_err(|_| WitnessFailure::new("reused_backend_identity"))?;
    let reused = owner_queue
        .checkpoint_claim_transaction_lease_in(next, &mut lease)
        .await;
    if next_pid != owner_pid
        || !matches!(reused, Err(QueueError::StaleClaim(id)) if id == claim.job.id)
    {
        return Err(WitnessFailure::new("new_transaction_not_refused"));
    }
    let expired = owner_queue
        .lock_claim_transaction_lease_in(next, &claim)
        .await;
    if !matches!(expired, Err(QueueError::StaleClaim(id)) if id == claim.job.id) {
        return Err(WitnessFailure::new("expired_claim_not_refused"));
    }
    let (last_anchor, now, status, outcome): (DateTime<Utc>, DateTime<Utc>, String, String) =
        sqlx::query_as("SELECT jobs.locked_at, clock_timestamp(), jobs.status::text, job_attempts.outcome::text FROM jobs JOIN job_attempts ON job_attempts.job_id = jobs.id WHERE jobs.id = $1 AND job_attempts.id = $2")
            .bind(claim.job.id).bind(claim.attempt.id).fetch_one(&mut **next).await
            .map_err(|_| WitnessFailure::new("expired_guard_readback"))?;
    if last_anchor != persisted_anchor
        || now < last_anchor + chrono::Duration::seconds(5)
        || status != "RUNNING"
        || outcome != "RUNNING"
    {
        return Err(WitnessFailure::new("expired_claim_was_mutated"));
    }
    transaction
        .take()
        .ok_or(WitnessFailure::new("handoff_final_rollback_missing"))?
        .rollback()
        .await
        .map_err(|_| WitnessFailure::new("handoff_final_rollback"))?;
    if financial_row_counts(monitor).await? != financial_before {
        return Err(WitnessFailure::new("handoff_financial_rows_changed"));
    }
    Ok(json!({
        "nonce": nonce, "job_id": claim.job.id, "attempt_id": claim.attempt.id,
        "attempt_no": claim.attempt.attempt_no, "worker_id": claim.worker_id,
        "owner_backend_pid": owner_pid, "owner_backend_start": stamp(owner_start),
        "foreign_backend_pid": foreign_pid, "same_backend_new_transaction_pid": next_pid,
        "heartbeat_quiesced_and_joined_before_lock": true, "sticky_loss": *active.lost.borrow(),
        "transaction_timestamp": stamp(owner_tx_time), "started_clock_timestamp": stamp(started),
        "finished_clock_timestamp": stamp(finished), "renewal_samples": samples,
        "total_elapsed_exceeds_lease": true, "foreign_transaction_refused_without_mutation": true,
        "same_backend_new_transaction_refused": true, "expired_claim_refused_without_renewal": true,
        "persisted_locked_at": stamp(last_anchor), "expiry_check_clock_timestamp": stamp(now),
        "queue_status_after_wait": status, "attempt_outcome_after_wait": outcome,
        "lease_seconds": CLAIM_LEASE.as_secs(), "financial_rows_unchanged": true,
        "terminal_success_rows_created": false
    }))
}

#[test]
fn witness_nonce_and_sqlstate_classifiers_reject_unbounded_values() {
    assert!(checked_nonce("0123456789abcdef0123456789abcdef"));
    assert!(!checked_nonce("0123456789ABCDEF0123456789abcdef"));
    assert_eq!(checked_sqlstate("55P03").as_deref(), Some("55P03"));
    assert_eq!(checked_sqlstate("55p03"), None);
    assert_eq!(checked_sqlstate("55P03;message"), None);
}

#[tokio::test]
#[ignore = "requires the explicitly admitted disposable PostgreSQL witness cluster"]
async fn job_lock_blocks_heartbeat_and_maps_loss() {
    match execute_witness().await {
        Ok(report) => println!("R5_CLAIM_WITNESS {}", report),
        Err(WitnessFailure {
            primary,
            cleanup: Some(cleanup),
        }) => panic!("R5_CLAIM_WITNESS failed: {primary}; cleanup: {cleanup}"),
        Err(WitnessFailure {
            primary,
            cleanup: None,
        }) => panic!("R5_CLAIM_WITNESS failed: {primary}"),
    }
}

#[tokio::test]
#[ignore = "requires the explicitly admitted disposable PostgreSQL witness cluster"]
async fn transaction_handoff_renews_live_claim_and_refuses_expired() {
    match execute_witness_case(true).await {
        Ok(report) => println!("R5_CLAIM_WITNESS {}", report),
        Err(WitnessFailure {
            primary,
            cleanup: Some(cleanup),
        }) => panic!("R5_CLAIM_WITNESS failed: {primary}; cleanup: {cleanup}"),
        Err(WitnessFailure {
            primary,
            cleanup: None,
        }) => panic!("R5_CLAIM_WITNESS failed: {primary}"),
    }
}
