//! Real-role queue recovery against synthetic SQL admission fixtures. These
//! cases never confer approval on filesystem input or execute provider calls.

mod common;
mod owner_beta_daily_support;

use std::{error::Error, time::Duration};

use job_queue::owner_beta::{
    OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE, recover_owner_beta_claims,
    runner::{daily_runtime::DailyProcessMode, daily_schedule::schedule_daily_once},
};
use job_queue::{AuditActor, ClaimedJob, JobQueue, QueueConfig};
use owner_beta_daily_support::{DailyFixture, run};
use serde_json::Value;
use sqlx::{
    ConnectOptions,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use uuid::Uuid;

type TestResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

fn queue(f: &DailyFixture, lease: u64) -> JobQueue {
    JobQueue::new(
        f.worker.clone(),
        None,
        QueueConfig {
            lease: Duration::from_secs(lease),
            backoff_base: Duration::from_secs(1),
        },
    )
}

async fn reserve(f: &DailyFixture) -> TestResult<Uuid> {
    f.ready().await?;
    schedule_daily_once(&f.worker, DailyProcessMode::OwnerOnly).await?;
    Ok(
        sqlx::query_scalar("SELECT job_id FROM owner_beta_daily_reservations")
            .fetch_one(&f.db.pool)
            .await?,
    )
}

async fn claim(queue: &JobQueue) -> TestResult<ClaimedJob> {
    Ok(queue
        .claim_next_for(
            "daily-recovery-fixture",
            OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE,
        )
        .await?
        .ok_or("daily claim missing")?)
}

async fn rows(f: &DailyFixture) -> TestResult<(Value, Value)> {
    Ok(sqlx::query_as(
        "SELECT to_jsonb(r),to_jsonb(d) FROM owner_beta_recommendation_runs r \
        JOIN owner_beta_daily_reservations d ON d.run_id=r.id",
    )
    .fetch_one(&f.db.pool)
    .await?)
}

async fn short_lease(f: &DailyFixture) -> TestResult<JobQueue> {
    // This changes the fixture's control epoch through its real guard. Failure
    // cleanup remains required after authority changes; success stays fenced.
    sqlx::query("UPDATE owner_beta_daily_control SET lease_seconds=1")
        .execute(&f.db.pool)
        .await?;
    Ok(queue(f, 1))
}

async fn wait_db_clock(f: &DailyFixture, job: Uuid, expired: bool) -> TestResult<()> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let ready: bool = sqlx::query_scalar(
                "SELECT CASE WHEN $2 THEN locked_at+interval '1 second'<=clock_timestamp() \
                ELSE available_at<=clock_timestamp() END FROM jobs WHERE id=$1",
            )
            .bind(job)
            .bind(expired)
            .fetch_one(&f.worker)
            .await?;
            if ready {
                return Ok::<(), sqlx::Error>(());
            }
            tokio::task::yield_now().await;
        }
    })
    .await??;
    Ok(())
}

async fn cancel(f: &DailyFixture, job: Uuid) -> TestResult<()> {
    let options: PgConnectOptions = f.db.role_url("audit_writer").parse()?;
    let audit = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options.disable_statement_logging())
        .await?;
    let app = JobQueue::new(f.app.clone(), Some(audit.clone()), QueueConfig::default());
    let result = app.request_cancel(job, &AuditActor::new("owner")).await;
    audit.close().await;
    assert!(matches!(result?, job_queue::CancelResult::Canceled(_)));
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE action='job.canceled' AND target_id=$1",
    )
    .bind(job.to_string())
    .fetch_one(&f.db.pool)
    .await?;
    assert_eq!(count, 1);
    Ok(())
}

#[tokio::test]
async fn daily_recovery_does_not_steal_a_live_claim() {
    run("daily_recovery_live",|f| async move {
        let job=reserve(&f).await?;
        let q=queue(&f,60);
        let claimed=claim(&q).await?;
        assert_eq!(claimed.job.id,job);
        let before=rows(&f).await?;
        let report=recover_owner_beta_claims(&q).await?;
        assert_eq!(report.jobs_checked,0);
        assert_eq!(rows(&f).await?,before);
        let state:(String,String)=sqlx::query_as("SELECT j.status::text,a.outcome::text FROM jobs j JOIN job_attempts a ON a.job_id=j.id WHERE j.id=$1")
            .bind(job).fetch_one(&f.worker).await?;
        assert_eq!(state,("RUNNING".into(),"RUNNING".into()));
        Ok(())
    }).await;
}

#[tokio::test]
async fn daily_expired_recovery_requeues_the_same_job_and_preserves_all_sealed_rows() {
    run("daily_recovery_requeue", |f| async move {
        let job = reserve(&f).await?;
        let q = queue(&f, 60);
        let claimed = claim(&q).await?;
        let q = short_lease(&f).await?;
        let before = rows(&f).await?;
        wait_db_clock(&f, job, true).await?;
        let generic = q.sweep().await?;
        assert_eq!(generic.attempts_orphaned, 0);
        let report = recover_owner_beta_claims(&q).await?;
        assert_eq!(report.jobs_requeued, 1);
        assert_eq!(report.attempts_orphaned, 1);
        assert_eq!(rows(&f).await?, before);
        let state: (String, String, bool, Value) = sqlx::query_as(
            "SELECT j.status::text,a.outcome::text, \
            j.locked_at IS NULL AND j.locked_by IS NULL,j.payload_json FROM jobs j \
            JOIN job_attempts a ON a.job_id=j.id AND a.attempt_no=j.attempt_count WHERE j.id=$1",
        )
        .bind(job)
        .fetch_one(&f.worker)
        .await?;
        assert_eq!(
            state,
            (
                "QUEUED".into(),
                "ORPHANED".into(),
                true,
                claimed.job.payload_json
            )
        );
        wait_db_clock(&f, job, false).await?;
        let next = claim(&q).await?;
        assert_eq!(next.job.id, job);
        assert_eq!(next.attempt.attempt_no, 2);
        assert_eq!(rows(&f).await?, before);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_recovery_exhausts_three_attempts_without_replacing_the_reservation() {
    run("daily_recovery_exhausted", |f| async move {
        let job = reserve(&f).await?;
        let q = queue(&f, 60);
        let first = claim(&q).await?;
        let q = short_lease(&f).await?;
        let before = rows(&f).await?;
        for number in 1..=3 {
            wait_db_clock(&f, job, true).await?;
            let report = recover_owner_beta_claims(&q).await?;
            assert_eq!(report.attempts_orphaned, 1);
            if number < 3 {
                assert_eq!(report.jobs_requeued, 1);
                assert_eq!(rows(&f).await?, before);
                wait_db_clock(&f, job, false).await?;
                let next = claim(&q).await?;
                assert_eq!(next.job.id, job);
                assert_eq!(next.attempt.attempt_no, number + 1);
                assert_eq!(next.job.payload_json, first.job.payload_json);
            } else {
                assert_eq!(report.jobs_failed, 1);
            }
        }
        let (run, reservation) = rows(&f).await?;
        assert_eq!(reservation, before.1);
        assert_eq!(run["status"], "FAILED");
        assert_eq!(run["error_code"], "OWNER_BETA_ATTEMPTS_EXHAUSTED");
        let state: (String, i32, i64, i64) = sqlx::query_as(
            "SELECT status::text,attempt_count, \
            (SELECT count(*) FROM job_attempts WHERE outcome='ORPHANED'), \
            (SELECT count(*) FROM owner_beta_recommendation_items) FROM jobs WHERE id=$1",
        )
        .bind(job)
        .fetch_one(&f.worker)
        .await?;
        assert_eq!(state, ("FAILED".into(), 3, 3, 0));
        let after = rows(&f).await?;
        let report = recover_owner_beta_claims(&q).await?;
        assert_eq!(report.jobs_checked, 0);
        assert_eq!(rows(&f).await?, after);
        f.next_schedule().await?;
        schedule_daily_once(&f.worker, DailyProcessMode::OwnerOnly).await?;
        assert_eq!(f.job_payloads().await?, vec![first.job.payload_json]);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_racing_recoveries_orphan_and_requeue_only_once() {
    run("daily_recovery_race", |f| async move {
        let job = reserve(&f).await?;
        let q = queue(&f, 60);
        claim(&q).await?;
        let q = short_lease(&f).await?;
        wait_db_clock(&f, job, true).await?;
        let before = rows(&f).await?;
        let (a, b) = tokio::join!(recover_owner_beta_claims(&q), recover_owner_beta_claims(&q));
        let (a, b) = (a?, b?);
        assert_eq!(a.jobs_requeued + b.jobs_requeued, 1);
        assert_eq!(a.attempts_orphaned + b.attempts_orphaned, 1);
        assert_eq!(rows(&f).await?, before);
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM job_attempts WHERE job_id=$1 AND outcome='ORPHANED'",
        )
        .bind(job)
        .fetch_one(&f.worker)
        .await?;
        assert_eq!(count, 1);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_recovery_mirrors_unclaimed_audited_cancellation() {
    run("daily_recovery_unclaimed_cancel", |f| async move {
        let job = reserve(&f).await?;
        let reservation = rows(&f).await?.1;
        cancel(&f, job).await?;
        let q = queue(&f, 60);
        let report = recover_owner_beta_claims(&q).await?;
        assert_eq!(report.runs_canceled, 1);
        assert_eq!(report.attempts_orphaned, 0);
        let (run, after) = rows(&f).await?;
        assert_eq!(run["status"], "CANCELED");
        assert_eq!(run["error_code"], "CANCELED");
        assert_eq!(after, reservation);
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM job_attempts WHERE job_id=$1")
            .bind(job)
            .fetch_one(&f.worker)
            .await?;
        assert_eq!(count, 0);
        let after = rows(&f).await?;
        let report = recover_owner_beta_claims(&q).await?;
        assert_eq!(report.runs_canceled, 0);
        assert_eq!(rows(&f).await?, after);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_recovery_orphans_expired_canceled_attempt_and_releases_its_lease() {
    run("daily_recovery_running_cancel", |f| async move {
        let job = reserve(&f).await?;
        let q = queue(&f, 60);
        claim(&q).await?;
        let reservation = rows(&f).await?.1;
        cancel(&f, job).await?;
        let q = short_lease(&f).await?;
        wait_db_clock(&f, job, true).await?;
        let report = recover_owner_beta_claims(&q).await?;
        assert_eq!(report.runs_canceled, 1);
        assert_eq!(report.attempts_orphaned, 1);
        let (run, after) = rows(&f).await?;
        assert_eq!(run["status"], "CANCELED");
        assert_eq!(after, reservation);
        let state: (String, String, bool) = sqlx::query_as(
            "SELECT j.status::text,a.outcome::text,j.locked_at IS NULL AND j.locked_by IS NULL \
            FROM jobs j JOIN job_attempts a ON a.job_id=j.id WHERE j.id=$1",
        )
        .bind(job)
        .fetch_one(&f.worker)
        .await?;
        assert_eq!(state, ("CANCELED".into(), "ORPHANED".into(), true));
        Ok(())
    })
    .await;
}
