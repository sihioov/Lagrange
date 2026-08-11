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

use job_queue::batch::{self, BatchItem, MAX_BATCH_SIZE};
use job_queue::{
    CancelResult, ErrorClass, HeartbeatStatus, JobQueue, JobStatus, QueueConfig, SettleResult,
    SubmitJob,
};
use sqlx::migrate::{MigrationType, Migrator};
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};
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
DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'research_writer') THEN
    CREATE ROLE research_writer LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD 'lagrange';
  END IF;
END $role$;
DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'admin') THEN
    CREATE ROLE admin LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD 'lagrange';
  END IF;
END $role$;
GRANT USAGE ON SCHEMA public TO migration_owner, app, worker, audit_writer, research_writer, admin;
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
/// intermittently (documented in learnings.md; the shared cluster is also
/// restarted by operators mid-run), so a fresh pool per test rides through
/// short outages instead of failing the whole suite.
///
/// sqlx pools connect lazily, so the retry performs a REAL round-trip
/// (`SELECT 1`) on each attempt; only a pool that answered a query counts.
async fn connect_with_retry(url: &str, max_conns: u32) -> Result<PgPool, Box<dyn Error>> {
    let mut opts: sqlx::postgres::PgConnectOptions = url.parse()?;
    opts = opts.options([("statement_timeout", "20s")]);
    connect_with_options(opts, max_conns).await
}

/// Connect with retries plus caller-supplied connection options (e.g. the
/// Todo 23 RLS actor GUC `app.actor_user_id`).
async fn connect_with_options(
    opts: PgConnectOptions,
    max_conns: u32,
) -> Result<PgPool, Box<dyn Error>> {
    let mut last: Option<sqlx::Error> = None;
    for attempt in 0..6u32 {
        let connect = PgPoolOptions::new()
            .max_connections(max_conns)
            .acquire_timeout(Duration::from_secs(8))
            .connect_with(opts.clone())
            .await;
        let pool = match connect {
            Ok(pool) => pool,
            Err(e) => {
                last = Some(e);
                tokio::time::sleep(Duration::from_millis(1200 * (attempt as u64 + 1))).await;
                continue;
            }
        };
        match sqlx::query_scalar::<_, i32>("SELECT 1")
            .fetch_one(&pool)
            .await
        {
            Ok(_) => return Ok(pool),
            Err(e) => {
                last = Some(e);
                pool.close().await;
                tokio::time::sleep(Duration::from_millis(1200 * (attempt as u64 + 1))).await;
            }
        }
    }
    Err(last
        .map(Into::into)
        .unwrap_or_else(|| "connect_with_retry exhausted attempts".into()))
}

/// Run a test body on a fresh scratch database, retrying up to 3 times with a
/// NEW scratch database per attempt. The shared cluster is restarted by
/// operators without notice (every connection dies), so a whole-test retry is
/// the only way to ride through such a window.
async fn run_test<F>(label: &str, super_url: &str, body: F)
where
    F: for<'a> Fn(
        &'a str,
        &'a str,
        &'a PgPool,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), Box<dyn Error>>> + 'a>,
    >,
{
    let mut last: Option<String> = None;
    for attempt in 0..3u32 {
        let scratch = match create_scratch_db(super_url).await {
            Ok(v) => v,
            Err(e) => {
                last = Some(format!("setup: {e}"));
                tokio::time::sleep(Duration::from_secs(3)).await;
                continue;
            }
        };
        let result = body(super_url, &scratch.0, &scratch.1).await;
        let _ = drop_scratch_db(super_url, &scratch.0).await;
        match result {
            Ok(()) => return,
            Err(e) => {
                last = Some(format!("{e}"));
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
        eprintln!(
            "{label}: attempt {} failed ({last:?}); retrying with a fresh DB",
            attempt + 1
        );
    }
    panic!(
        "{label} FAILED after 3 attempts: {}",
        last.unwrap_or_default()
    );
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
    sqlx::raw_sql(ROLE_BOOTSTRAP_SQL)
        .execute(&super_new)
        .await?;
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

fn submit(owner: Uuid, key: Option<&str>, max_attempts: i32, tag: i64) -> SubmitJob {
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

async fn claim_and_settle_loop(queue: &JobQueue, worker_id: &str) -> Result<usize, Box<dyn Error>> {
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

#[tokio::test]
async fn typed_workers_claim_only_their_job_type() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    run_test("typed claims", &super_url, |_s, _d, p| {
        Box::pin(typed_claims_body(p))
    })
    .await;
}

async fn typed_claims_body(pool: &PgPool) -> Result<(), Box<dyn Error>> {
    let owner = insert_test_user(pool, "sub-typed-claims").await?;
    let q = JobQueue::new(pool.clone(), None, fast_config());

    let backtest = q.submit(submit(owner, None, 3, 1)).await?;
    let mut recommendation_request = submit(owner, None, 3, 2);
    recommendation_request.job_type = "recommendation".to_string();
    let recommendation = q.submit(recommendation_request).await?;

    let recommendation_claim = q
        .claim_next_for("rec-worker", "recommendation")
        .await?
        .expect("recommendation job must be claimable");
    assert_eq!(recommendation_claim.job.id, recommendation.id);
    assert_eq!(recommendation_claim.job.job_type, "recommendation");

    let backtest_claim = q
        .claim_next_for("bt-worker", "backtest")
        .await?
        .expect("backtest job must be claimable");
    assert_eq!(backtest_claim.job.id, backtest.id);
    assert_eq!(backtest_claim.job.job_type, "backtest");
    Ok(())
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
    run_test("two-worker drain", &super_url, |s, d, p| {
        Box::pin(two_workers_body(s, d, p))
    })
    .await;
}

async fn two_workers_body(super_url: &str, db: &str, pool: &PgPool) -> Result<(), Box<dyn Error>> {
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
    assert_eq!(
        a + b,
        100,
        "both workers together must claim exactly 100 jobs"
    );

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
        count(
            pool,
            "SELECT count(*) FROM jobs WHERE status <> 'SUCCEEDED'"
        )
        .await,
        0,
        "all jobs must be SUCCEEDED after the drain"
    );
    // Every attempt is terminal SUCCEEDED, owned by exactly one of the workers.
    assert_eq!(
        count(
            pool,
            "SELECT count(*) FROM job_attempts WHERE outcome <> 'SUCCEEDED'"
        )
        .await,
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
    assert!(
        a > 0 && b > 0,
        "both workers must claim at least one job: a={a} b={b}"
    );
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
    run_test("idempotency", &super_url, |_s, _d, p| {
        Box::pin(idempotency_body(p))
    })
    .await;
}

async fn idempotency_body(pool: &PgPool) -> Result<(), Box<dyn Error>> {
    let owner = insert_test_user(pool, "sub-pool").await?;
    let q = JobQueue::new(pool.clone(), None, fast_config());

    let first = q.submit(submit(owner, Some("k1"), 3, 1)).await?;

    // Serial duplicates return the same job id and do not create rows.
    for _ in 0..5 {
        let same = q.submit(submit(owner, Some("k1"), 3, 1)).await?;
        assert_eq!(
            same.id, first.id,
            "serial duplicate must return the same job"
        );
    }
    assert_eq!(
        count(
            pool,
            "SELECT count(*) FROM jobs WHERE idempotency_key = 'k1'"
        )
        .await,
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
        assert_eq!(
            job.id, first.id,
            "concurrent duplicate must return the same job"
        );
    }
    assert_eq!(
        count(
            pool,
            "SELECT count(*) FROM jobs WHERE idempotency_key = 'k1'"
        )
        .await,
        1,
        "concurrent duplicates must not create extra rows"
    );

    // The key is scoped per owner: another owner with the same key gets a new job.
    let other = insert_test_user(pool, "sub-other").await?;
    let other_job = q.submit(submit(other, Some("k1"), 3, 2)).await?;
    assert_ne!(other_job.id, first.id, "idempotency key is per-owner");

    // Re-submitting after completion returns the SAME job unchanged (AT-03:
    // "identical input twice -> existing run reused"), never a re-run.
    let claim = q
        .claim_next("worker-a")
        .await?
        .expect("job must be claimable");
    let _ = q.settle_success(&claim).await?;
    let done = q.submit(submit(owner, Some("k1"), 3, 1)).await?;
    assert_eq!(
        done.id, first.id,
        "post-completion re-submit returns the same job"
    );
    assert_eq!(
        done.status,
        JobStatus::Succeeded,
        "re-submit must not restart the job"
    );

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
    run_test("submit validation", &super_url, |_s, _d, p| {
        Box::pin(validate_body(p))
    })
    .await;
}

async fn validate_body(pool: &PgPool) -> Result<(), Box<dyn Error>> {
    let owner = insert_test_user(pool, "sub-pool").await?;
    let q = JobQueue::new(pool.clone(), None, fast_config());

    let mut bad_type = submit(owner, None, 3, 1);
    bad_type.job_type = String::new();
    assert!(
        q.submit(bad_type).await.is_err(),
        "empty job_type must be rejected"
    );

    let bad_attempts = submit(owner, None, 0, 1);
    assert!(
        q.submit(bad_attempts).await.is_err(),
        "max_attempts < 1 must be rejected"
    );

    let mut bad_payload = submit(owner, None, 3, 1);
    bad_payload.payload = serde_json::json!([1, 2, 3]);
    assert!(
        q.submit(bad_payload).await.is_err(),
        "non-object payload must be rejected"
    );

    assert_eq!(
        count(pool, "SELECT count(*) FROM jobs").await,
        0,
        "no rows may land"
    );
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
    run_test("api availability", &super_url, |s, d, p| {
        Box::pin(api_available_body(s, d, p))
    })
    .await;
}

async fn api_available_body(
    _super_url: &str,
    db: &str,
    pool: &PgPool,
) -> Result<(), Box<dyn Error>> {
    let owner = insert_test_user(pool, "sub-pool").await?;
    let q = JobQueue::new(pool.clone(), None, fast_config());
    let _job = q.submit(submit(owner, None, 3, 1)).await?;
    let claim = q
        .claim_next("worker-a")
        .await?
        .expect("job must be claimable");
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
    let c2 = q
        .claim_next("worker-a")
        .await?
        .expect("second job claimable");
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
    run_test("heartbeat", &super_url, |_s, _d, p| {
        Box::pin(heartbeat_body(p))
    })
    .await;
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
    run_test("production roles", &super_url, |s, d, p| {
        Box::pin(roles_body(s, d, p))
    })
    .await;
}

async fn roles_body(super_url: &str, db: &str, pool: &PgPool) -> Result<(), Box<dyn Error>> {
    let owner = insert_test_user(pool, "sub-pool").await?;

    // Todo 23 RLS: tenant reads are strict (owner = app.actor_user_id GUC), so
    // the app-role pool must carry the actor context to act "as the owner".
    let app_pool = {
        let opts: PgConnectOptions = conn_url(super_url, "app", db).parse()?;
        connect_with_options(opts.options([("app.actor_user_id", owner.to_string())]), 2).await?
    };
    let worker_pool = connect_with_retry(&conn_url(super_url, "worker", db), 2).await?;
    let api = JobQueue::new(app_pool.clone(), None, fast_config());
    let w = JobQueue::new(worker_pool.clone(), None, fast_config());

    // app submits; worker claims and settles the full lifecycle.
    let job = api.submit(submit(owner, Some("role-k"), 3, 1)).await?;
    let claim = w
        .claim_next("role-worker")
        .await?
        .expect("worker must claim");
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

// ---------------------------------------------------------------------------
// (c) Cancellation: audited requests, cooperative worker response, and the
//     settle-after-cancel race. CANCELED is a JOB status only; the attempt of
//     an aborted run is recorded FAILED(error_code='canceled').
// ---------------------------------------------------------------------------

async fn audit_rows(pool: &PgPool, job_id: Uuid) -> Vec<(String, String, String, String, String)> {
    sqlx::query_as(
        "SELECT action, actor_role, target_type, target_id, reason
         FROM audit_logs WHERE action = 'job.canceled' AND target_id = $1 ORDER BY created_at",
    )
    .bind(job_id.to_string())
    .fetch_all(pool)
    .await
    .expect("audit query")
}

#[tokio::test]
async fn cancel_queued_job_is_audited_and_terminal() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    run_test("queued cancel", &super_url, |_s, _d, p| {
        Box::pin(cancel_queued_body(p))
    })
    .await;
}

async fn cancel_queued_body(pool: &PgPool) -> Result<(), Box<dyn Error>> {
    let owner = insert_test_user(pool, "sub-cq").await?;
    let q = JobQueue::new(pool.clone(), Some(pool.clone()), fast_config());
    let actor = job_queue::AuditActor::new("owner");

    let job = q.submit(submit(owner, None, 3, 1)).await?;
    match q.request_cancel(job.id, &actor).await? {
        CancelResult::Canceled(j) => assert_eq!(j.status, JobStatus::Canceled),
        other => panic!("queued cancel must cancel, got {other:?}"),
    }
    // Never claimed: zero attempts.
    assert_eq!(
        count(pool, "SELECT count(*) FROM job_attempts").await,
        0,
        "canceled-before-claim job must have no attempts"
    );
    // Audited: exactly one job.canceled row naming this job.
    let rows = audit_rows(pool, job.id).await;
    assert_eq!(rows.len(), 1, "cancel must be audited exactly once");
    assert_eq!(rows[0].0, "job.canceled");
    assert_eq!(rows[0].1, "owner");
    assert_eq!(rows[0].2, "job");
    assert_eq!(rows[0].3, job.id.to_string());

    // A second cancel on the terminal job is a no-op, not a new audit row.
    match q.request_cancel(job.id, &actor).await? {
        CancelResult::AlreadyTerminal(j) => assert_eq!(j.status, JobStatus::Canceled),
        other => panic!("second cancel must be AlreadyTerminal, got {other:?}"),
    }
    assert_eq!(audit_rows(pool, job.id).await.len(), 1);
    // Canceling a nonexistent job is a typed error.
    let ghost = Uuid::new_v4();
    assert!(matches!(
        q.request_cancel(ghost, &actor).await,
        Err(job_queue::QueueError::JobNotFound(_))
    ));
    Ok(())
}

#[tokio::test]
async fn cancel_running_job_is_cooperative_and_audited() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    run_test("running cancel", &super_url, |_s, _d, p| {
        Box::pin(cancel_running_body(p))
    })
    .await;
}

async fn cancel_running_body(pool: &PgPool) -> Result<(), Box<dyn Error>> {
    let owner = insert_test_user(pool, "sub-cr").await?;
    let q = JobQueue::new(pool.clone(), Some(pool.clone()), fast_config());
    let actor = job_queue::AuditActor::new("owner");

    let job = q.submit(submit(owner, None, 3, 1)).await?;
    let claim = q.claim_next("worker-a").await?.expect("claimable");

    // Cancel while RUNNING: the job flips to CANCELED immediately (the worker
    // is not interrupted); the in-flight attempt stays RUNNING until the
    // worker observes the checkpoint cooperatively.
    match q.request_cancel(job.id, &actor).await? {
        CancelResult::Canceled(j) => assert_eq!(j.status, JobStatus::Canceled),
        other => panic!("running cancel must cancel, got {other:?}"),
    }
    assert_eq!(audit_rows(pool, job.id).await.len(), 1);
    assert!(
        q.check_canceled(job.id).await?,
        "checkpoint must observe the cancel"
    );

    // Cooperative response: the worker aborts and records its own attempt as
    // FAILED('canceled'); the JOB stays CANCELED, never FAILED.
    let aborted = q
        .settle_aborted(&claim, "observed cancel checkpoint")
        .await?;
    assert_eq!(aborted.status, JobStatus::Canceled);
    let attempt: (String, Option<String>) = sqlx::query_as(
        "SELECT outcome, error_code FROM job_attempts WHERE job_id = $1 AND attempt_no = 1",
    )
    .bind(job.id)
    .fetch_one(pool)
    .await?;
    assert_eq!(attempt.0, "FAILED");
    assert_eq!(attempt.1.as_deref(), Some("canceled"));
    // No retry may ever follow a cancel.
    assert!(
        q.claim_next("worker-a").await?.is_none(),
        "canceled job must never be claimable"
    );
    assert_eq!(q.get_by_id(job.id).await?.status, JobStatus::Canceled);
    Ok(())
}

#[tokio::test]
async fn settle_after_cancel_marks_attempt_canceled() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    run_test("settle/cancel race", &super_url, |_s, _d, p| {
        Box::pin(settle_cancel_race_body(p))
    })
    .await;
}

async fn settle_cancel_race_body(pool: &PgPool) -> Result<(), Box<dyn Error>> {
    let owner = insert_test_user(pool, "sub-sc").await?;
    let q = JobQueue::new(pool.clone(), Some(pool.clone()), fast_config());
    let actor = job_queue::AuditActor::new("owner");

    // Cancel wins the race: the late settle must NOT record SUCCEEDED.
    let job = q.submit(submit(owner, None, 3, 1)).await?;
    let claim = q.claim_next("worker-a").await?.expect("claimable");
    let _ = q.request_cancel(job.id, &actor).await?;
    match q.settle_success(&claim).await? {
        SettleResult::Canceled(j) => assert_eq!(j.status, JobStatus::Canceled),
        other => panic!("settle after cancel must yield Canceled, got {other:?}"),
    }
    let (outcome, code): (String, Option<String>) = sqlx::query_as(
        "SELECT outcome, error_code FROM job_attempts WHERE job_id = $1 AND attempt_no = 1",
    )
    .bind(job.id)
    .fetch_one(pool)
    .await?;
    assert_eq!(
        outcome, "FAILED",
        "canceled work must never settle SUCCEEDED"
    );
    assert_eq!(code.as_deref(), Some("canceled"));

    // Settle wins the race: the cancel is a no-op on the terminal job.
    let job2 = q.submit(submit(owner, None, 3, 2)).await?;
    let claim2 = q.claim_next("worker-a").await?.expect("claimable");
    match q.settle_success(&claim2).await? {
        SettleResult::Committed(j) => assert_eq!(j.status, JobStatus::Succeeded),
        other => panic!("settle must win when it lands first, got {other:?}"),
    }
    match q.request_cancel(job2.id, &actor).await? {
        CancelResult::AlreadyTerminal(j) => assert_eq!(j.status, JobStatus::Succeeded),
        other => panic!("cancel after success must be AlreadyTerminal, got {other:?}"),
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// (e)/(f) Retry classification (design 6.8): ONLY transient errors retry,
//     with exponential backoff; input/data/integrity/determinism errors fail
//     the job immediately no matter how many attempts remain. Exhaustion of
//     retries resolves the job FAILED.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn transient_failure_requeues_with_backoff_then_succeeds() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    run_test("transient retry", &super_url, |_s, _d, p| {
        Box::pin(transient_retry_body(p))
    })
    .await;
}

async fn transient_retry_body(pool: &PgPool) -> Result<(), Box<dyn Error>> {
    let owner = insert_test_user(pool, "sub-tr").await?;
    let q = JobQueue::new(pool.clone(), None, fast_config()); // backoff base 300ms

    let job = q.submit(submit(owner, None, 3, 1)).await?;
    let claim = q.claim_next("worker-a").await?.expect("claimable");
    match q
        .settle_failure(&claim, ErrorClass::Transient, "e_boom", "transient glitch")
        .await?
    {
        SettleResult::Committed(j) => {
            assert_eq!(
                j.status,
                JobStatus::Queued,
                "transient failure must requeue"
            );
            assert!(
                j.locked_by.is_none() && j.locked_at.is_none(),
                "requeue releases the claim"
            );
            assert!(j.available_at > j.created_at, "requeue applies a backoff");
        }
        other => panic!("transient failure must requeue, got {other:?}"),
    }
    // Attempt 1 is recorded FAILED with the error.
    let (outcome, code): (String, Option<String>) = sqlx::query_as(
        "SELECT outcome, error_code FROM job_attempts WHERE job_id = $1 AND attempt_no = 1",
    )
    .bind(job.id)
    .fetch_one(pool)
    .await?;
    assert_eq!(outcome, "FAILED");
    assert_eq!(code.as_deref(), Some("e_boom"));
    // Backoff: not claimable immediately...
    assert!(
        q.claim_next("worker-a").await?.is_none(),
        "backoff must delay the retry"
    );
    // ...then claimable after the backoff elapses.
    tokio::time::sleep(Duration::from_millis(700)).await;
    let claim2 = q
        .claim_next("worker-a")
        .await?
        .expect("retry claim after backoff");
    assert_eq!(claim2.attempt.attempt_no, 2, "retry must be attempt 2");
    match q.settle_success(&claim2).await? {
        SettleResult::Committed(j) => assert_eq!(j.status, JobStatus::Succeeded),
        other => panic!("retried job must succeed, got {other:?}"),
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM job_attempts WHERE job_id = $1")
            .bind(job.id)
            .fetch_one(pool)
            .await?,
        2,
        "exactly two attempts"
    );
    Ok(())
}

#[tokio::test]
async fn retry_exhaustion_ends_failed() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    run_test("retry exhaustion", &super_url, |_s, _d, p| {
        Box::pin(exhaustion_body(p))
    })
    .await;
}

async fn exhaustion_body(pool: &PgPool) -> Result<(), Box<dyn Error>> {
    let owner = insert_test_user(pool, "sub-ex").await?;
    let q = JobQueue::new(pool.clone(), None, fast_config());

    let job = q.submit(submit(owner, None, 2, 1)).await?; // max_attempts = 2
    for attempt in 1..=2 {
        let claim = q
            .claim_next("worker-a")
            .await?
            .expect("claim for attempt {attempt}");
        match q
            .settle_failure(&claim, ErrorClass::Transient, "e_boom", "keeps failing")
            .await?
        {
            SettleResult::Committed(j) => {
                if attempt == 1 {
                    assert_eq!(j.status, JobStatus::Queued);
                } else {
                    assert_eq!(
                        j.status,
                        JobStatus::Failed,
                        "retry exhaustion must resolve FAILED"
                    );
                    assert_eq!(j.error_code.as_deref(), Some("e_boom"));
                }
            }
            other => panic!("unexpected settle result {other:?}"),
        }
        tokio::time::sleep(Duration::from_millis(700)).await; // past the backoff
    }
    let final_job = q.get_by_id(job.id).await?;
    assert_eq!(final_job.status, JobStatus::Failed);
    assert_eq!(final_job.attempt_count, 2);
    assert!(
        q.claim_next("worker-a").await?.is_none(),
        "exhausted job must not be claimable"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM job_attempts WHERE job_id = $1")
            .bind(job.id)
            .fetch_one(pool)
            .await?,
        2,
        "exactly two attempts"
    );
    Ok(())
}

#[tokio::test]
async fn input_data_integrity_errors_never_retry() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    run_test("no-retry classes", &super_url, |_s, _d, p| {
        Box::pin(no_retry_body(p))
    })
    .await;
}

async fn no_retry_body(pool: &PgPool) -> Result<(), Box<dyn Error>> {
    let owner = insert_test_user(pool, "sub-nr").await?;
    let q = JobQueue::new(pool.clone(), None, fast_config());

    for (i, class) in [
        ErrorClass::Input,
        ErrorClass::DataBlocked,
        ErrorClass::Integrity,
        ErrorClass::Determinism,
    ]
    .into_iter()
    .enumerate()
    {
        // max_attempts = 3: three retries would be available, but a
        // non-retryable class must fail the job on the FIRST attempt.
        let job = q.submit(submit(owner, None, 3, i as i64)).await?;
        let claim = q.claim_next("worker-a").await?.expect("claimable");
        match q
            .settle_failure(&claim, class, "e_noretry", "not transient")
            .await?
        {
            SettleResult::Committed(j) => {
                assert_eq!(
                    j.status,
                    JobStatus::Failed,
                    "{class:?} must fail immediately"
                );
                assert_eq!(j.error_code.as_deref(), Some("e_noretry"));
            }
            other => panic!("{class:?} must settle to FAILED, got {other:?}"),
        }
        assert_eq!(
            q.get_by_id(job.id).await?.attempt_count,
            1,
            "{class:?} must not retry"
        );
        assert!(
            q.claim_next("worker-a").await?.is_none(),
            "{class:?} failure must leave nothing claimable"
        );
    }
    assert_eq!(
        count(pool, "SELECT count(*) FROM jobs WHERE status <> 'FAILED'").await,
        0
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// (d) Orphan recovery: an expired lease is swept to an ORPHANED attempt
//     (attempt-level outcome ONLY — jobs.status never shows ORPHANED), the
//     job is requeued AT MOST ONCE per orphan, and retry exhaustion resolves
//     the job FAILED. A zombie worker can never settle after the sweep.
// ---------------------------------------------------------------------------

async fn attempt_rows(pool: &PgPool, job_id: Uuid) -> Vec<(i32, String, String)> {
    sqlx::query_as(
        "SELECT attempt_no, outcome, claimed_by FROM job_attempts WHERE job_id = $1 ORDER BY attempt_no",
    )
    .bind(job_id)
    .fetch_all(pool)
    .await
    .expect("attempt query")
}

#[tokio::test]
async fn worker_death_orphans_attempt_and_requeues_once() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    run_test("orphan requeue", &super_url, |_s, _d, p| {
        Box::pin(orphan_requeue_body(p))
    })
    .await;
}

async fn orphan_requeue_body(pool: &PgPool) -> Result<(), Box<dyn Error>> {
    let owner = insert_test_user(pool, "sub-or").await?;
    let q = JobQueue::new(pool.clone(), None, fast_config()); // 1s lease, 300ms backoff

    let job = q.submit(submit(owner, None, 3, 1)).await?;
    let _claim = q.claim_next("worker-a").await?.expect("claimable");
    tokio::time::sleep(Duration::from_millis(1500)).await; // lease expires: worker died

    let report = q.sweep().await?;
    assert_eq!(report.jobs_checked, 1);
    assert_eq!(report.attempts_orphaned, 1);
    assert_eq!(report.jobs_requeued, 1);
    assert_eq!(report.jobs_failed, 0);

    // Attempt 1 is ORPHANED (attempt-level); jobs.status is QUEUED, never
    // ORPHANED.
    let rows = attempt_rows(pool, job.id).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, 1);
    assert_eq!(rows[0].1, "ORPHANED");
    let swept = q.get_by_id(job.id).await?;
    assert_eq!(swept.status, JobStatus::Queued);
    assert!(swept.locked_by.is_none());
    assert_eq!(
        swept.attempt_count, 1,
        "orphan must not consume the next attempt"
    );
    assert!(
        count(pool, "SELECT count(*) FROM jobs WHERE status = 'ORPHANED'").await == 0,
        "ORPHANED must never appear as a job status"
    );

    // Sweeping again is a no-op: the same orphan is never requeued twice.
    let report2 = q.sweep().await?;
    assert_eq!(report2.jobs_checked, 0, "second sweep must find nothing");
    assert_eq!(report2.jobs_requeued, 0);

    // The requeued job is claimable by ANY worker (worker-b) as attempt 2.
    tokio::time::sleep(Duration::from_millis(700)).await; // past backoff
    let claim2 = q
        .claim_next("worker-b")
        .await?
        .expect("requeued job claimable");
    assert_eq!(claim2.attempt.attempt_no, 2);
    assert_eq!(claim2.job.status, JobStatus::Running);
    let _ = q.settle_success(&claim2).await?;
    assert_eq!(q.get_by_id(job.id).await?.status, JobStatus::Succeeded);
    Ok(())
}

#[tokio::test]
async fn orphan_exhaustion_resolves_failed() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    run_test("orphan exhaustion", &super_url, |_s, _d, p| {
        Box::pin(orphan_exhaust_body(p))
    })
    .await;
}

async fn orphan_exhaust_body(pool: &PgPool) -> Result<(), Box<dyn Error>> {
    let owner = insert_test_user(pool, "sub-oe").await?;
    let q = JobQueue::new(pool.clone(), None, fast_config());

    let job = q.submit(submit(owner, None, 2, 1)).await?; // max_attempts = 2
    // Death 1: orphan + requeue.
    let _claim = q.claim_next("worker-a").await?.expect("claimable");
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let r1 = q.sweep().await?;
    assert_eq!((r1.jobs_requeued, r1.jobs_failed), (1, 0));
    // Death 2: attempts exhausted -> FAILED, no requeue.
    tokio::time::sleep(Duration::from_millis(700)).await;
    let claim2 = q.claim_next("worker-b").await?.expect("claimable");
    assert_eq!(claim2.attempt.attempt_no, 2);
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let r2 = q.sweep().await?;
    assert_eq!(r2.attempts_orphaned, 1);
    assert_eq!(r2.jobs_requeued, 0, "no requeue after retry exhaustion");
    assert_eq!(r2.jobs_failed, 1);

    let final_job = q.get_by_id(job.id).await?;
    assert_eq!(final_job.status, JobStatus::Failed);
    assert_eq!(final_job.error_code.as_deref(), Some("attempts_exhausted"));
    let rows = attempt_rows(pool, job.id).await;
    assert_eq!(
        rows.iter().map(|r| (r.0, r.1.as_str())).collect::<Vec<_>>(),
        vec![(1, "ORPHANED"), (2, "ORPHANED")],
        "both dead attempts are ORPHANED"
    );
    Ok(())
}

#[tokio::test]
async fn zombie_worker_cannot_settle_after_sweep() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    run_test("zombie settle", &super_url, |_s, _d, p| {
        Box::pin(zombie_body(p))
    })
    .await;
}

async fn zombie_body(pool: &PgPool) -> Result<(), Box<dyn Error>> {
    let owner = insert_test_user(pool, "sub-zb").await?;
    let q = JobQueue::new(pool.clone(), None, fast_config());

    let job = q.submit(submit(owner, None, 3, 1)).await?;
    let claim = q.claim_next("worker-a").await?.expect("claimable");
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let _ = q.sweep().await?; // attempt 1 -> ORPHANED, job requeued

    // The zombie's settle and heartbeat are both rejected; the ORPHANED
    // attempt is never rewritten.
    assert!(
        matches!(
            q.settle_success(&claim).await,
            Err(job_queue::QueueError::StaleClaim(_))
        ),
        "zombie settle must be rejected as StaleClaim"
    );
    assert_eq!(q.heartbeat(&claim).await?, HeartbeatStatus::LeaseLost);
    let rows = attempt_rows(pool, job.id).await;
    assert_eq!(rows[0].1, "ORPHANED", "orphaned attempt stays immutable");
    assert_eq!(q.get_by_id(job.id).await?.status, JobStatus::Queued);

    // A fresh worker takes the requeued job and settles it normally.
    tokio::time::sleep(Duration::from_millis(700)).await;
    let claim2 = q.claim_next("worker-b").await?.expect("reclaimable");
    let _ = q.settle_success(&claim2).await?;
    assert_eq!(q.get_by_id(job.id).await?.status, JobStatus::Succeeded);
    Ok(())
}

#[tokio::test]
async fn sweep_finalizes_orphan_of_canceled_job_without_requeue() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    run_test("sweep canceled", &super_url, |_s, _d, p| {
        Box::pin(sweep_canceled_body(p))
    })
    .await;
}

async fn sweep_canceled_body(pool: &PgPool) -> Result<(), Box<dyn Error>> {
    let owner = insert_test_user(pool, "sub-sc2").await?;
    let q = JobQueue::new(pool.clone(), Some(pool.clone()), fast_config());
    let actor = job_queue::AuditActor::new("owner");

    // Cancel a RUNNING job, then let the worker's lease expire without a
    // cooperative abort: the sweeper finalizes the attempt as ORPHANED and
    // never requeues — cancellation is honored even across worker death.
    let job = q.submit(submit(owner, None, 3, 1)).await?;
    let _claim = q.claim_next("worker-a").await?.expect("claimable");
    let _ = q.request_cancel(job.id, &actor).await?;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let report = q.sweep().await?;
    assert_eq!(report.attempts_orphaned, 1);
    assert_eq!(
        report.jobs_requeued, 0,
        "canceled job must never be requeued"
    );

    let rows = attempt_rows(pool, job.id).await;
    assert_eq!(rows[0].1, "ORPHANED");
    let job = q.get_by_id(job.id).await?;
    assert_eq!(job.status, JobStatus::Canceled);
    assert!(q.claim_next("worker-a").await?.is_none());
    Ok(())
}

// ---------------------------------------------------------------------------
// Todo 29: bounded batch fan-out and cascade cancellation — the job-queue
// side of robustness suite orchestration. `job_queue::batch` is
// domain-agnostic: it knows nothing about robustness/derived axes, only "N
// related jobs, one owner, each with a caller-supplied idempotency key."
// Domain-specific fan-out planning (grid limits, one-axis children, holdout
// guards) lives in result-model, which owns that domain.
// ---------------------------------------------------------------------------

fn batch_item(tag: usize, key: &str) -> BatchItem {
    BatchItem {
        job_type: "backtest".to_string(),
        payload: serde_json::json!({ "kind": "robustness_child", "tag": tag }),
        idempotency_key: key.to_string(),
    }
}

#[tokio::test]
async fn robustness_orchestration_rejects_oversized_batch() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    run_test("robustness oversized batch", &super_url, |_s, _d, p| {
        Box::pin(oversized_batch_body(p))
    })
    .await;
}

async fn oversized_batch_body(pool: &PgPool) -> Result<(), Box<dyn Error>> {
    let owner = insert_test_user(pool, "sub-batch-oversized").await?;
    let q = JobQueue::new(pool.clone(), Some(pool.clone()), fast_config());
    let items: Vec<BatchItem> = (0..=MAX_BATCH_SIZE)
        .map(|i| batch_item(i, &format!("oversized-{i}")))
        .collect();
    let err = batch::submit_batch(&q, owner, items, 5, 3)
        .await
        .expect_err("a batch larger than MAX_BATCH_SIZE must be rejected before any insert");
    assert!(matches!(err, job_queue::QueueError::InvalidInput(_)));
    let queued: i64 = count(pool, "SELECT count(*) FROM jobs").await;
    assert_eq!(
        queued, 0,
        "an oversized batch must submit NOTHING, not a truncated prefix"
    );
    Ok(())
}

#[tokio::test]
async fn robustness_orchestration_fan_out_is_crash_safe_on_resubmit() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    run_test("robustness crash-safe fan-out", &super_url, |_s, _d, p| {
        Box::pin(crash_safe_fan_out_body(p))
    })
    .await;
}

async fn crash_safe_fan_out_body(pool: &PgPool) -> Result<(), Box<dyn Error>> {
    let owner = insert_test_user(pool, "sub-batch-crashsafe").await?;
    let q = JobQueue::new(pool.clone(), Some(pool.clone()), fast_config());
    let items = || {
        vec![
            batch_item(0, "suite-1-child-cost-stress"),
            batch_item(1, "suite-1-child-execution-delay"),
            batch_item(2, "suite-1-child-benchmark"),
        ]
    };
    let first = batch::submit_batch(&q, owner, items(), 5, 3).await?;
    assert_eq!(first.len(), 3);

    // The orchestrator "dies" after submitting and re-plans the SAME suite
    // from scratch: re-submission with the same keys must resolve to the
    // SAME jobs, never duplicate rows (AT-03 semantics at suite level).
    let second = batch::submit_batch(&q, owner, items(), 5, 3).await?;
    let first_ids: Vec<Uuid> = first.iter().map(|j| j.id).collect();
    let second_ids: Vec<Uuid> = second.iter().map(|j| j.id).collect();
    assert_eq!(
        first_ids, second_ids,
        "re-submission must return the identical jobs"
    );

    let total: i64 = count(pool, "SELECT count(*) FROM jobs").await;
    assert_eq!(
        total, 3,
        "crash-safe re-submission must never create duplicate rows"
    );
    Ok(())
}

#[tokio::test]
async fn robustness_orchestration_cascade_cancel_stops_pending_children_only() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    run_test("robustness cascade cancel", &super_url, |_s, _d, p| {
        Box::pin(cascade_cancel_body(p))
    })
    .await;
}

async fn cascade_cancel_body(pool: &PgPool) -> Result<(), Box<dyn Error>> {
    let owner = insert_test_user(pool, "sub-batch-cascadecancel").await?;
    let q = JobQueue::new(pool.clone(), Some(pool.clone()), fast_config());
    let items = vec![
        batch_item(0, "suite-2-child-a"),
        batch_item(1, "suite-2-child-b"),
        batch_item(2, "suite-2-child-c"),
    ];
    let jobs = batch::submit_batch(&q, owner, items, 5, 3).await?;
    let job_ids: Vec<Uuid> = jobs.iter().map(|j| j.id).collect();

    // One child already finished successfully before the cascade fires.
    let claim = q.claim_next("worker-cascade").await?.expect("claimable");
    assert_eq!(claim.job.id, job_ids[0]);
    let _ = q.settle_success(&claim).await?;

    let actor = job_queue::AuditActor::new("owner");
    let results = batch::cancel_batch(&q, &job_ids, &actor).await;
    assert_eq!(results.len(), 3);

    let mut canceled = 0usize;
    let mut already_terminal = 0usize;
    for (_, outcome) in &results {
        match outcome.as_ref().expect("cancel request must not error") {
            CancelResult::Canceled(_) => canceled += 1,
            CancelResult::AlreadyTerminal(job) => {
                already_terminal += 1;
                assert_eq!(job.status, JobStatus::Succeeded);
            }
        }
    }
    assert_eq!(
        canceled, 2,
        "the two still-pending children must be canceled"
    );
    assert_eq!(
        already_terminal, 1,
        "the already-succeeded child stays untouched"
    );

    let mut statuses = Vec::new();
    for id in &job_ids {
        statuses.push(q.get_by_id(*id).await?.status);
    }
    assert_eq!(statuses[0], JobStatus::Succeeded);
    assert_eq!(statuses[1], JobStatus::Canceled);
    assert_eq!(statuses[2], JobStatus::Canceled);
    Ok(())
}
