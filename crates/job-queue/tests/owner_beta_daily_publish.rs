//! Daily queue/run settlement with real database principals. Synthetic SQL
//! fixtures exercise persistence only; they confer no filesystem approval.

mod common;
mod owner_beta_daily_support;

use std::{error::Error, time::Duration};

use job_queue::owner_beta::{
    OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE, OwnerBetaPublicationError,
    OwnerBetaPublicationFailure, OwnerBetaPublicationOutcome,
    daily_input::OwnerBetaDailyRecommendationInputV1,
    publish::settle_owner_beta_daily_failure,
    runner::{
        daily_runtime::DailyProcessMode,
        daily_schedule::{DailyScheduleOutcome, schedule_daily_once},
    },
};
use job_queue::{ClaimedJob, JobQueue, QueueConfig};
use owner_beta_daily_support::{DailyFixture, run};
use serde_json::Value;
use sqlx::{
    ConnectOptions,
    postgres::{PgConnectOptions, PgPoolOptions},
};

type TestResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

async fn claim(
    f: &DailyFixture,
) -> TestResult<(JobQueue, ClaimedJob, OwnerBetaDailyRecommendationInputV1)> {
    f.ready().await?;
    assert!(matches!(
        schedule_daily_once(&f.worker, DailyProcessMode::OwnerOnly).await?,
        DailyScheduleOutcome::Completed(_)
    ));
    let queue = JobQueue::new(
        f.worker.clone(),
        None,
        QueueConfig {
            lease: Duration::from_secs(60),
            backoff_base: Duration::from_secs(1),
        },
    );
    let claim = queue
        .claim_next_for(
            "daily-settlement-fixture",
            OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE,
        )
        .await?
        .ok_or("daily claim missing")?;
    let input = serde_json::from_value(claim.job.payload_json.clone())?;
    Ok((queue, claim, input))
}

async fn saved_rows(f: &DailyFixture) -> TestResult<(Value, Value)> {
    Ok(sqlx::query_as(
        "SELECT to_jsonb(r),to_jsonb(d) FROM owner_beta_recommendation_runs r \
        JOIN owner_beta_daily_reservations d ON d.run_id=r.id",
    )
    .fetch_one(&f.db.pool)
    .await?)
}

#[tokio::test]
async fn daily_transient_failure_preserves_pending_row_and_permanent_reservation() {
    run("daily_transient_settlement", |f| async move {
        let (queue, claim, input) = claim(&f).await?;
        let before = saved_rows(&f).await?;
        assert_eq!(
            settle_owner_beta_daily_failure(
                &f.worker,
                &queue,
                &claim,
                &input,
                OwnerBetaPublicationFailure::ComputationUnavailable
            )
            .await?,
            OwnerBetaPublicationOutcome::Retrying
        );
        assert_eq!(saved_rows(&f).await?, before);
        let state: (String, String, i64) = sqlx::query_as(
            "SELECT j.status::text,a.outcome::text, \
            (SELECT count(*) FROM owner_beta_recommendation_items) FROM jobs j \
            JOIN job_attempts a ON a.job_id=j.id AND a.attempt_no=j.attempt_count WHERE j.id=$1",
        )
        .bind(claim.job.id)
        .fetch_one(&f.worker)
        .await?;
        assert_eq!(state, ("QUEUED".into(), "FAILED".into(), 0));
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_terminal_failure_releases_capacity_but_preserves_reservation() {
    run("daily_terminal_settlement", |f| async move {
        let (queue, claim, input) = claim(&f).await?;
        let reservation = saved_rows(&f).await?.1;
        assert_eq!(
            settle_owner_beta_daily_failure(
                &f.worker,
                &queue,
                &claim,
                &input,
                OwnerBetaPublicationFailure::InputInvalid
            )
            .await?,
            OwnerBetaPublicationOutcome::Failed
        );
        let (row, after_reservation) = saved_rows(&f).await?;
        assert_eq!(reservation, after_reservation);
        assert_eq!(row["status"], "FAILED");
        assert_eq!(row["error_code"], "OWNER_BETA_INPUT_INVALID");
        let active: i64 =
            sqlx::query_scalar("SELECT count(*) FROM jobs WHERE status IN ('QUEUED','RUNNING')")
                .fetch_one(&f.worker)
                .await?;
        assert_eq!(active, 0);
        let before = f.job_payloads().await?;
        f.next_schedule().await?;
        let DailyScheduleOutcome::Completed(report) =
            schedule_daily_once(&f.worker, DailyProcessMode::OwnerOnly).await?
        else {
            panic!("repeat scheduling completes");
        };
        assert_eq!(report.replayed, 1);
        assert_eq!(report.reserved, 0);
        assert_eq!(f.job_payloads().await?, before);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_expired_claim_cannot_settle_or_touch_the_pending_run() {
    run("daily_stale_settlement", |f| async move {
        let (_, claim, input) = claim(&f).await?;
        // Real DB time expiry with a short fixture lease. No trigger or clock
        // bypass: activation changes are allowed to fence success but not cleanup.
        sqlx::query("UPDATE owner_beta_daily_control SET lease_seconds=1")
            .execute(&f.db.pool)
            .await?;
        let queue = JobQueue::new(
            f.worker.clone(),
            None,
            QueueConfig {
                lease: Duration::from_secs(1),
                backoff_base: Duration::from_secs(1),
            },
        );
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let expired: bool = sqlx::query_scalar(
                    "SELECT locked_at+interval '1 second'<=clock_timestamp() FROM jobs WHERE id=$1",
                )
                .bind(claim.job.id)
                .fetch_one(&f.worker)
                .await?;
                if expired {
                    return Ok::<(), sqlx::Error>(());
                }
                tokio::task::yield_now().await;
            }
        })
        .await??;
        let before = saved_rows(&f).await?;
        let attempt: Value =
            sqlx::query_scalar("SELECT to_jsonb(a) FROM job_attempts a WHERE job_id=$1")
                .bind(claim.job.id)
                .fetch_one(&f.worker)
                .await?;
        assert_eq!(
            settle_owner_beta_daily_failure(
                &f.worker,
                &queue,
                &claim,
                &input,
                OwnerBetaPublicationFailure::InputInvalid
            )
            .await,
            Err(OwnerBetaPublicationError::QueueClaimLost)
        );
        assert_eq!(saved_rows(&f).await?, before);
        let after: Value =
            sqlx::query_scalar("SELECT to_jsonb(a) FROM job_attempts a WHERE job_id=$1")
                .bind(claim.job.id)
                .fetch_one(&f.worker)
                .await?;
        assert_eq!(attempt, after);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_failure_still_settles_after_control_is_disabled() {
    run("daily_disabled_settlement", |f| async move {
        let (queue, claim, input) = claim(&f).await?;
        let reservation = saved_rows(&f).await?.1;
        sqlx::query("UPDATE owner_beta_daily_control SET enabled=false")
            .execute(&f.db.pool)
            .await?;
        assert_eq!(
            settle_owner_beta_daily_failure(
                &f.worker,
                &queue,
                &claim,
                &input,
                OwnerBetaPublicationFailure::InputInvalid
            )
            .await?,
            OwnerBetaPublicationOutcome::Failed
        );
        let (row, after) = saved_rows(&f).await?;
        assert_eq!(row["status"], "FAILED");
        assert_eq!(reservation, after);
        let outputs: i64 =
            sqlx::query_scalar("SELECT count(*) FROM owner_beta_recommendation_items")
                .fetch_one(&f.worker)
                .await?;
        assert_eq!(outputs, 0);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_mismatched_payload_cannot_settle_the_valid_claim() {
    run("daily_payload_mismatch", |f| async move {
        let (queue, claim, input) = claim(&f).await?;
        let before = saved_rows(&f).await?;
        let mut changed = serde_json::to_value(&input)?;
        changed["input"]["policy_sha256"] = serde_json::json!(format!("sha256:{}", "f".repeat(64)));
        let changed = serde_json::from_value(changed)?;
        assert_eq!(settle_owner_beta_daily_failure(&f.worker,&queue,&claim,&changed,
            OwnerBetaPublicationFailure::InputInvalid).await,
            Err(OwnerBetaPublicationError::Integrity));
        assert_eq!(saved_rows(&f).await?,before);
        let state: (String,String) = sqlx::query_as(
            "SELECT j.status::text,a.outcome::text FROM jobs j JOIN job_attempts a ON a.job_id=j.id WHERE j.id=$1")
            .bind(claim.job.id).fetch_one(&f.worker).await?;
        assert_eq!(state,("RUNNING".into(),"RUNNING".into()));
        Ok(())
    }).await;
}

async fn audited_cancel(f: &DailyFixture, job: uuid::Uuid) -> TestResult<()> {
    let options: PgConnectOptions = f.db.role_url("audit_writer").parse()?;
    let audit = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options.disable_statement_logging())
        .await?;
    let app = JobQueue::new(f.app.clone(), Some(audit.clone()), QueueConfig::default());
    let result = app
        .request_cancel(job, &job_queue::AuditActor::new("owner"))
        .await;
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
async fn daily_cancellation_wins_failure_and_clears_claim_without_outputs() {
    run("daily_cancel_settlement", |f| async move {
        let (queue, claim, input) = claim(&f).await?;
        let reservation = saved_rows(&f).await?.1;
        audited_cancel(&f, claim.job.id).await?;
        assert_eq!(
            settle_owner_beta_daily_failure(
                &f.worker,
                &queue,
                &claim,
                &input,
                OwnerBetaPublicationFailure::ComputationUnavailable
            )
            .await?,
            OwnerBetaPublicationOutcome::Canceled
        );
        let (row, after) = saved_rows(&f).await?;
        assert_eq!(row["status"], "CANCELED");
        assert_eq!(row["error_code"], "CANCELED");
        assert_eq!(after, reservation);
        let state: (String, String, bool, i64) = sqlx::query_as(
            "SELECT j.status::text,a.outcome::text,j.locked_at IS NULL AND j.locked_by IS NULL, \
             (SELECT count(*) FROM owner_beta_recommendation_items) FROM jobs j \
             JOIN job_attempts a ON a.job_id=j.id WHERE j.id=$1",
        )
        .bind(claim.job.id)
        .fetch_one(&f.worker)
        .await?;
        assert_eq!(state, ("CANCELED".into(), "FAILED".into(), true, 0));
        Ok(())
    })
    .await;
}
