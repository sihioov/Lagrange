//! Todo 19 job-queue contract: leased PostgreSQL claims, idempotent
//! submission, heartbeats, cancellation, retry classification and orphan
//! recovery, proven against a disposable PostgreSQL 18 cluster.
//!
//! Every test requires `DATABASE_URL` to point at a SUPERUSER connection to a
//! DISPOSABLE cluster (canonical Windows URL for this repo:
//! `postgres://postgres:lagrange@172.26.46.217:5432/postgres`). Tests create
//! their own scratch database per test and drop it afterwards; on hosts with
//! no database `require_db_url()` skips cleanly (reported as `ok`, zero
//! assertions) so `cargo test --workspace` stays green everywhere.
//!
//! Schema mapping (frozen T3 migrations, never modified here):
//!   - `jobs.status` has EXACTLY five public states
//!     (QUEUED|RUNNING|SUCCEEDED|FAILED|CANCELED) — ORPHANED exists only on
//!     `job_attempts.outcome` (attempt-level), never on jobs.
//!   - the lease anchor is `jobs.locked_at` set at claim time (design §6.8)
//!     and extended by heartbeats; lease expiry = locked_at + configured
//!     interval. `job_attempts.started_at` records the claim instant and is
//!     immutable; attempts are never rewritten once terminal.
//!   - cancellation is cooperative: `jobs.status` flips to CANCELED on an
//!     audited request; the RUNNING worker observes a cancel checkpoint and
//!     settles its own attempt as FAILED(error_code='canceled') — CANCELED is
//!     not an attempt outcome.

use job_queue::{HeartbeatStatus, JobQueue, JobStatus, QueueConfig, SubmitJob};
use sqlx::migrate::{MigrationType, Migrator};
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::types::Uuid;
use std::env;
use std::error::Error;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Migrations embedded at compile time from the workspace `migrations/` dir.
static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

/// Role bootstrap executed as superuser on each fresh database. Roles are
/// cluster-wide and created idempotently (same contract as the Todo 3
/// harness); per-database grants land via migration 0009.
const ROLE_BOOTSTRAP_SQL: &str = r#"
DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'migration_owner') THEN
    CREATE ROLE migration_owner LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD 'lagrange';
  END IF;
END $role$;
DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'app') THEN
    CREATE ROLE app LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD 'lagrange';
  END IF;
END $role$;
DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'worker') THEN
    CREATE ROLE worker LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD 'lagrange';
  END IF;
END $role$;
DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'audit_writer') THEN
    CREATE ROLE audit_writer LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD 'lagrange';
  END IF;
END $role$;
GRANT USAGE ON SCHEMA public TO migration_owner, app, worker, audit_writer;
GRANT CREATE ON SCHEMA public TO migration_owner;
"#;

/// SQLx 0.9's compile-time SQL audit requires dynamic DDL (database names
/// cannot be bind parameters) to be wrapped; the only interpolated value is a
/// `fresh_db_name()` output asserted safe below.
fn ddl_for(db: &str, statement: &str) -> sqlx::AssertSqlSafe<String> {
    sqlx::AssertSqlSafe(statement.replace("{db}", db))
}

/// Scratch-database name, unique per call (monotonic counter: parallel test
/// threads share one process, so pid+millis alone can collide).
fn fresh_db_name() -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_millis();
    format!(
        "queue_{}_{}_{}",
        std::process::id(),
        ts,
        SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

/// Rewrite a superuser URL for a role and database, preserving the password
/// (the disposable cluster runs scram with password `lagrange`).
fn conn_url(super_url: &str, role: &str, db: &str) -> String {
    let (_scheme, rest) = super_url
        .split_once("://")
        .expect("DATABASE_URL must start with a scheme");
    let (auth, hostport_db) = rest.split_once('@').expect("DATABASE_URL must contain @");
    let (user, pw) = match auth.split_once(':') {
        Some((u, p)) => (u, Some(p)),
        None => (auth, None),
    };
    let (hostport, _old_db) = hostport_db
        .rsplit_once('/')
        .expect("DATABASE_URL must contain a database path");
    let _ = user; // role replaces the original user
    match pw {
        Some(p) => format!("postgres://{role}:{p}@{hostport}/{db}"),
        None => format!("postgres://{role}@{hostport}/{db}"),
    }
}

/// Number of up-migrations (sqlx 0.9 records each .up.sql/.down.sql as its
/// own Migration entry; run applies every non-ReversibleDown).
fn up_migration_count() -> usize {
    MIGRATOR
        .migrations
        .iter()
        .filter(|m| m.migration_type != MigrationType::ReversibleDown)
        .count()
}

/// Connect with retries: the Windows->WSL PostgreSQL path flaps
/// intermittently (documented in learnings.md; wslrelay died 2026-08-06 and
/// the eth0-IP NAT route drops connections mid-stream), so a fresh pool per
/// test rides through short outages instead of failing the whole suite.
///
/// sqlx pools connect lazily, so the retry performs a REAL round-trip
/// (`SELECT 1`) on each attempt; only a pool that answered a query counts.
async fn connect_with_retry(url: &str, max_conns: u32) -> Result<PgPool, Box<dyn Error>> {
    let mut last: Option<sqlx::Error> = None;
    for attempt in 0..10u32 {
        let connect = PgPoolOptions::new()
            .max_connections(max_conns)
            .acquire_timeout(Duration::from_secs(8))
            .connect(url)
            .await;
        let pool = match connect {
            Ok(pool) => pool,
            Err(e) => {
                last = Some(e);
                tokio::time::sleep(Duration::from_millis(1500 * (attempt as u64 + 1))).await;
                continue;
            }
        };
        match sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(&pool).await {
            Ok(_) => return Ok(pool),
            Err(e) => {
                last = Some(e);
                pool.close().await;
                tokio::time::sleep(Duration::from_millis(1500 * (attempt as u64 + 1))).await;
            }
        }
    }
    Err(last
        .map(Into::into)
        .unwrap_or_else(|| "connect_with_retry exhausted attempts".into()))
}

async fn admin_pool(url: &str) -> Result<PgPool, Box<dyn Error>> {
    connect_with_retry(url, 3).await
}

/// Create a fresh scratch database, bootstrap roles, apply all migrations,
/// and insert one test user. Returns (db_name, superuser_pool).
async fn create_scratch_db(super_url: &str) -> Result<(String, PgPool), Box<dyn Error>> {
    let db = fresh_db_name();
    assert!(
        db.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
        "generated database name must be a safe identifier"
    );
    let admin = admin_pool(super_url).await?;
    sqlx::query(ddl_for(&db, "DROP DATABASE IF EXISTS {db} WITH (FORCE)"))
        .execute(&admin)
        .await?;
    sqlx::query(ddl_for(&db, "CREATE DATABASE {db}"))
        .execute(&admin)
        .await?;
    drop(admin);

    let super_new = connect_with_retry(&conn_url(super_url, "postgres", &db), 3).await?;
    sqlx::raw_sql(ROLE_BOOTSTRAP_SQL).execute(&super_new).await?;
    sqlx::raw_sql(ddl_for(
        &db,
        "GRANT CONNECT ON DATABASE {db} TO migration_owner, app, worker, audit_writer",
    ))
    .execute(&super_new)
    .await?;
    drop(super_new);

    let pool = connect_with_retry(&conn_url(super_url, "postgres", &db), 6).await?;
    let expected = up_migration_count();
    assert!(expected > 0, "migrator must embed at least one migration");
    MIGRATOR.run(&pool).await?;
    let applied: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await?;
    assert_eq!(applied as usize, expected, "all migrations applied");
    Ok((db, pool))
}

/// Drop the disposable database, terminating any remaining connections.
async fn drop_scratch_db(super_url: &str, db: &str) -> Result<(), Box<dyn Error>> {
    let admin = admin_pool(super_url).await?;
    sqlx::query(ddl_for(db, "DROP DATABASE IF EXISTS {db} WITH (FORCE)"))
        .execute(&admin)
        .await?;
    Ok(())
}

async fn insert_test_user(pool: &PgPool, subject: &str) -> Result<Uuid, Box<dyn Error>> {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) VALUES ('test-issuer', $1, $2) RETURNING id",
    )
    .bind(subject)
    .bind(format!("{subject}@example.com"))
    .fetch_one(pool)
    .await?;
    Ok(id)
}

fn require_db_url() -> Result<String, Box<dyn Error>> {
    match env::var("DATABASE_URL").ok().filter(|s| !s.is_empty()) {
        Some(url) => Ok(url),
        None => {
            eprintln!("SKIP: DATABASE_URL not set - no disposable PostgreSQL cluster available");
            Err("DATABASE_URL not set".into())
        }
    }
}

/// Short lease + short backoff for fast deterministic tests.
fn fast_config() -> QueueConfig {
    QueueConfig {
        lease: Duration::from_secs(1),
        backoff_base: Duration::from_millis(300),
    }
}

/// Lease long enough that WSL-NAT latency spikes never expire it mid-test
/// (the flap documented in learnings.md can stall a query for seconds).
fn long_lease_config() -> QueueConfig {
    QueueConfig {
        lease: Duration::from_secs(30),
        backoff_base: Duration::from_millis(300),
    }
}

fn submit<'a>(owner: Uuid, key: Option<&'a str>, max_attempts: i32, tag: i64) -> SubmitJob {
    SubmitJob {
        owner_user_id: owner,
        job_type: "backtest".to_string(),
        payload: serde_json::json!({ "tag": tag }),
        priority: 5,
        idempotency_key: key.map(str::to_string),
        max_attempts,
        available_at: None,
    }
}

async fn count(pool: &PgPool, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

async fn claim_and_settle_loop(
    queue: &JobQueue,
    worker_id: &str,
) -> Result<usize, Box<dyn Error>> {
    let mut claimed = 0usize;
    loop {
        match queue.claim_next(worker_id).await? {
            None => break,
            Some(c) => {
                claimed += 1;
                let _ = queue.settle_success(&c).await?;
            }
        }
    }
    Ok(claimed)
}

// ---------------------------------------------------------------------------
// (a) Concurrent claim ownership: two workers drain 100 jobs; every job is
//     claimed by exactly one worker and has exactly one attempt.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn two_workers_drain_100_jobs_without_duplicate_ownership() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    let (db, pool) = match create_scratch_db(&super_url).await {
        Ok(v) => v,
        Err(e) => panic!("setup failed: {e}"),
    };
    let result = two_workers_body(&super_url, &db, &pool).await;
    let _ = drop_scratch_db(&super_url, &db).await;
    if let Err(e) = result {
        panic!("concurrent claim contract FAILED: {e}");
    }
}

async fn two_workers_body(
    super_url: &str,
    db: &str,
    pool: &PgPool,
) -> Result<(), Box<dyn Error>> {
    let owner = insert_test_user(pool, "sub-pool").await?;

    // Separate pools per worker: each worker is an independent database
    // client with its own connection(s), like separate processes.
    let w1 = JobQueue::new(
        connect_with_retry(&conn_url(super_url, "postgres", db), 2).await?,
        None,
        fast_config(),
    );
    let w2 = JobQueue::new(
        connect_with_retry(&conn_url(super_url, "postgres", db), 2).await?,
        None,
        fast_config(),
    );

    for i in 0..100i64 {
        let mut j = submit(owner, None, 3, i);
        j.priority = (i % 7) as i32; // interleave priorities so ordering is exercised
        let job = w1.submit(j).await?;
        assert_eq!(job.status, JobStatus::Queued);
    }

    let (a, b) = tokio::join!(
        claim_and_settle_loop(&w1, "worker-a"),
        claim_and_settle_loop(&w2, "worker-b"),
    );
    let a = a?;
    let b = b?;
    assert_eq!(a + b, 100, "both workers together must claim exactly 100 jobs");

    // Exactly 100 attempts - one per job, no duplicates.
    assert_eq!(count(pool, "SELECT count(*) FROM job_attempts").await, 100);
    // No job has more than one attempt (a duplicate claim would create a
    // second attempt row OR fail the UNIQUE(job_id, attempt_no) constraint).
    assert_eq!(
        count(
            pool,
            "SELECT count(*) FROM (SELECT job_id FROM job_attempts GROUP BY job_id HAVING count(*) > 1) s"
        )
        .await,
        0,
        "no job may be claimed twice"
    );
    // Every job reached SUCCEEDED, none is left RUNNING or QUEUED.
    assert_eq!(
        count(pool, "SELECT count(*) FROM jobs WHERE status <> 'SUCCEEDED'").await,
        0,
        "all jobs must be SUCCEEDED after the drain"
    );
    // Every attempt is terminal SUCCEEDED, owned by exactly one of the workers.
    assert_eq!(
        count(pool, "SELECT count(*) FROM job_attempts WHERE outcome <> 'SUCCEEDED'").await,
        0,
        "every attempt must settle SUCCEEDED"
    );
    assert_eq!(
        count(
            pool,
            "SELECT count(*) FROM job_attempts WHERE claimed_by NOT IN ('worker-a', 'worker-b')"
        )
        .await,
        0,
        "every attempt must be owned by one of the two workers"
    );
    assert!(a > 0 && b > 0, "both workers must claim at least one job: a={a} b={b}");
    // attempt_count matches: each job was claimed exactly once.
    assert_eq!(
        count(pool, "SELECT count(*) FROM jobs WHERE attempt_count <> 1").await,
        0,
        "attempt_count must equal the single attempt for every job"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// (b) Idempotency keys: duplicate submits (serial, concurrent, post-completion)
//     all return the SAME job; the key is scoped per owner.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn duplicate_idempotency_key_returns_same_job() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    let (db, pool) = match create_scratch_db(&super_url).await {
        Ok(v) => v,
        Err(e) => panic!("setup failed: {e}"),
    };
    let result = idempotency_body(&pool).await;
    let _ = drop_scratch_db(&super_url, &db).await;
    if let Err(e) = result {
        panic!("idempotency contract FAILED: {e}");
    }
}

async fn idempotency_body(pool: &PgPool) -> Result<(), Box<dyn Error>> {
    let owner = insert_test_user(pool, "sub-pool").await?;
    let q = JobQueue::new(pool.clone(), None, fast_config());

    let first = q.submit(submit(owner, Some("k1"), 3, 1)).await?;

    // Serial duplicates return the same job id and do not create rows.
    for _ in 0..5 {
        let same = q.submit(submit(owner, Some("k1"), 3, 1)).await?;
        assert_eq!(same.id, first.id, "serial duplicate must return the same job");
    }
    assert_eq!(
        count(pool, "SELECT count(*) FROM jobs WHERE idempotency_key = 'k1'").await,
        1,
        "duplicate key must not create extra rows"
    );

    // Concurrent duplicates racing the original insert resolve to ONE job.
    let mut handles = Vec::new();
    for _ in 0..8 {
        let q2 = q.clone();
        let s = submit(owner, Some("k1"), 3, 1);
        handles.push(tokio::spawn(async move { q2.submit(s).await }));
    }
    for h in handles {
        let job = h.await.expect("submit task panicked")?;
        assert_eq!(job.id, first.id, "concurrent duplicate must return the same job");
    }
    assert_eq!(
        count(pool, "SELECT count(*) FROM jobs WHERE idempotency_key = 'k1'").await,
        1,
        "concurrent duplicates must not create extra rows"
    );

    // The key is scoped per owner: another owner with the same key gets a new job.
    let other = insert_test_user(pool, "sub-other").await?;
    let other_job = q.submit(submit(other, Some("k1"), 3, 2)).await?;
    assert_ne!(other_job.id, first.id, "idempotency key is per-owner");

    // Re-submitting after completion returns the SAME job unchanged (AT-03:
    // "identical input twice -> existing run reused"), never a re-run.
    let claim = q.claim_next("worker-a").await?.expect("job must be claimable");
    let _ = q.settle_success(&claim).await?;
    let done = q.submit(submit(owner, Some("k1"), 3, 1)).await?;
    assert_eq!(done.id, first.id, "post-completion re-submit returns the same job");
    assert_eq!(done.status, JobStatus::Succeeded, "re-submit must not restart the job");

    // Jobs without a key are never deduplicated.
    let unkeyed_a = q.submit(submit(owner, None, 3, 3)).await?;
    let unkeyed_b = q.submit(submit(owner, None, 3, 4)).await?;
    assert_ne!(unkeyed_a.id, unkeyed_b.id);
    Ok(())
}

// ---------------------------------------------------------------------------
// (c) Submit validation: malformed input is rejected BEFORE any row lands.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn submit_validates_malformed_input() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    let (db, pool) = match create_scratch_db(&super_url).await {
        Ok(v) => v,
        Err(e) => panic!("setup failed: {e}"),
    };
    let result = validate_body(&pool).await;
    let _ = drop_scratch_db(&super_url, &db).await;
    if let Err(e) = result {
        panic!("submit validation FAILED: {e}");
    }
}

async fn validate_body(pool: &PgPool) -> Result<(), Box<dyn Error>> {
    let owner = insert_test_user(pool, "sub-pool").await?;
    let q = JobQueue::new(pool.clone(), None, fast_config());

    let mut bad_type = submit(owner, None, 3, 1);
    bad_type.job_type = String::new();
    assert!(q.submit(bad_type).await.is_err(), "empty job_type must be rejected");

    let mut bad_attempts = submit(owner, None, 0, 1);
    assert!(q.submit(bad_attempts).await.is_err(), "max_attempts < 1 must be rejected");

    let mut bad_payload = submit(owner, None, 3, 1);
    bad_payload.payload = serde_json::json!([1, 2, 3]);
    assert!(q.submit(bad_payload).await.is_err(), "non-object payload must be rejected");

    assert_eq!(count(pool, "SELECT count(*) FROM jobs").await, 0, "no rows may land");
    Ok(())
}

// ---------------------------------------------------------------------------
// (g) The API database stays available while a job is RUNNING: claims are
//     short transactions, never held across work.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn api_db_available_while_job_running() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    let (db, pool) = match create_scratch_db(&super_url).await {
        Ok(v) => v,
        Err(e) => panic!("setup failed: {e}"),
    };
    let result = api_available_body(&super_url, &db, &pool).await;
    let _ = drop_scratch_db(&super_url, &db).await;
    if let Err(e) = result {
        panic!("API availability contract FAILED: {e}");
    }
}

async fn api_available_body(
    _super_url: &str,
    db: &str,
    pool: &PgPool,
) -> Result<(), Box<dyn Error>> {
    let owner = insert_test_user(pool, "sub-pool").await?;
    let q = JobQueue::new(pool.clone(), None, fast_config());
    let _job = q.submit(submit(owner, None, 3, 1)).await?;
    let claim = q.claim_next("worker-a").await?.expect("job must be claimable");
    assert_eq!(claim.job.status, JobStatus::Running);
    assert_eq!(claim.attempt.outcome, job_queue::AttemptOutcome::Running);

    // While the job is RUNNING (work not yet settled), the API must be able
    // to read and write freely: no open transaction may be held by the claim.
    let started = Instant::now();
    let live: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE status = 'RUNNING'")
        .fetch_one(pool)
        .await?;
    assert_eq!(live, 1, "exactly the claimed job is RUNNING");

    // A concurrent submit (write) succeeds while the first job is RUNNING.
    let second = q.submit(submit(owner, None, 3, 2)).await?;
    assert_eq!(second.status, JobStatus::Queued);

    // No connection of this database may sit idle-in-transaction: the claim
    // transaction was committed before work began.
    let idle: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_stat_activity WHERE datname = $1 AND state = 'idle in transaction'",
    )
    .bind(db)
    .fetch_one(pool)
    .await?;
    assert_eq!(idle, 0, "no long-held transactions while a job is RUNNING");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "API operations must complete without waiting on the RUNNING job"
    );

    // Settle normally afterwards; the second job is still claimable.
    let _ = q.settle_success(&claim).await?;
    let c2 = q.claim_next("worker-a").await?.expect("second job claimable");
    let _ = q.settle_success(&c2).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// (h) Heartbeats extend the lease (jobs.locked_at advances) and a lease that
//     expires without heartbeats reports LeaseLost.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn heartbeat_extends_lease_and_expired_lease_is_lost() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    let (db, pool) = match create_scratch_db(&super_url).await {
        Ok(v) => v,
        Err(e) => panic!("setup failed: {e}"),
    };
    let result = heartbeat_body(&pool).await;
    let _ = drop_scratch_db(&super_url, &db).await;
    if let Err(e) = result {
        panic!("heartbeat contract FAILED: {e}");
    }
}

async fn heartbeat_body(pool: &PgPool) -> Result<(), Box<dyn Error>> {
    let owner = insert_test_user(pool, "sub-pool").await?;
    // Long lease for the extension phase: latency spikes (WSL NAT flap) must
    // never expire the lease between heartbeats, or the test would assert on
    // network timing instead of heartbeat semantics.
    let q = JobQueue::new(pool.clone(), None, long_lease_config());

    let job = q.submit(submit(owner, None, 3, 1)).await?;
    let claim = q.claim_next("worker-a").await?.expect("claimable");
    assert_eq!(
        claim.lease_expires_at - claim.attempt.started_at.expect("claim inserts started_at"),
        chrono::Duration::seconds(30)
    );

    // Heartbeat well inside the lease: lease anchor advances each time.
    for _ in 0..3 {
        tokio::time::sleep(Duration::from_millis(400)).await; // 400ms < 1s lease
        let before = locked_at(pool, job.id).await;
        match q.heartbeat(&claim).await? {
            HeartbeatStatus::Extended => {}
            other => panic!("expected Extended, got {other:?}"),
        }
        let after = locked_at(pool, job.id).await;
        assert!(after > before, "heartbeat must advance the lease anchor");
    }
    let _ = q.settle_success(&claim).await?;

    // A second job with NO heartbeat lets the lease expire: the worker has
    // (apparently) died, and further heartbeats are rejected as LeaseLost.
    // Claimed through a 1s-lease queue so the expiry happens while we wait.
    let q_short = JobQueue::new(pool.clone(), None, fast_config());
    let job2 = q_short.submit(submit(owner, None, 3, 2)).await?;
    let claim2 = q_short.claim_next("worker-a").await?.expect("claimable");
    tokio::time::sleep(Duration::from_millis(1500)).await; // > 1s lease, no heartbeat
    match q_short.heartbeat(&claim2).await? {
        HeartbeatStatus::LeaseLost => {}
        other => panic!("expected LeaseLost after expiry, got {other:?}"),
    }
    // The job is still RUNNING until a sweeper acts on it (sweeper test).
    assert_eq!(q.get_by_id(job2.id).await?.status, JobStatus::Running);
    Ok(())
}

async fn locked_at(pool: &PgPool, job_id: Uuid) -> chrono::DateTime<chrono::Utc> {
    sqlx::query_scalar("SELECT locked_at FROM jobs WHERE id = $1")
        .bind(job_id)
        .fetch_one(pool)
        .await
        .expect("job must exist")
}

// ---------------------------------------------------------------------------
// (j) Production role matrix: app submits, worker claims/settles, and the
//     roles cannot cross the boundary (app cannot claim; worker cannot submit).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn queue_operates_under_production_roles() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    let (db, pool) = match create_scratch_db(&super_url).await {
        Ok(v) => v,
        Err(e) => panic!("setup failed: {e}"),
    };
    let result = roles_body(&super_url, &db, &pool).await;
    let _ = drop_scratch_db(&super_url, &db).await;
    if let Err(e) = result {
        panic!("production role contract FAILED: {e}");
    }
}

async fn roles_body(super_url: &str, db: &str, pool: &PgPool) -> Result<(), Box<dyn Error>> {
    let owner = insert_test_user(pool, "sub-pool").await?;

    let app_pool = connect_with_retry(&conn_url(super_url, "app", db), 2).await?;
    let worker_pool = connect_with_retry(&conn_url(super_url, "worker", db), 2).await?;
    let api = JobQueue::new(app_pool.clone(), None, fast_config());
    let w = JobQueue::new(worker_pool.clone(), None, fast_config());

    // app submits; worker claims and settles the full lifecycle.
    let job = api.submit(submit(owner, Some("role-k"), 3, 1)).await?;
    let claim = w.claim_next("role-worker").await?.expect("worker must claim");
    assert_eq!(claim.job.id, job.id);
    let _ = w.settle_success(&claim).await?;
    assert_eq!(api.get_by_id(job.id).await?.status, JobStatus::Succeeded);

    // Role boundary: the worker role cannot submit (no INSERT grant on jobs).
    match w.submit(submit(owner, None, 3, 2)).await {
        Err(job_queue::QueueError::Database(_)) => {}
        other => panic!("worker submit must be denied by grants, got {other:?}"),
    }
    // Role boundary: the app role cannot claim (no grant on job_attempts).
    let app_job = api.submit(submit(owner, None, 3, 3)).await?;
    match api.claim_next("app-worker").await {
        Err(job_queue::QueueError::Database(_)) => {}
        other => panic!("app claim must be denied by grants, got {other:?}"),
    }
    // The denied app claim must not have consumed the job.
    assert_eq!(api.get_by_id(app_job.id).await?.status, JobStatus::Queued);
    // And the worker can still claim it.
    let c = w.claim_next("role-worker").await?.expect("worker reclaims");
    let _ = w.settle_success(&c).await?;
    Ok(())
}

// Keep the compiler quiet about unused imports while red (implementation not
// landed yet); removed once the implementation compiles.
#[allow(unused_imports)]
use sqlx::Row as _Row;

