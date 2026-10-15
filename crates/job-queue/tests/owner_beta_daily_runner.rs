//! Real-principal preflight and mixed-dispatch cases. SQL fixtures are synthetic;
//! they cannot approve filesystem data or exercise the positive sealed chain.

mod common;
mod owner_beta_daily_support;

use std::{error::Error, path::Path, time::Duration};

use job_queue::{
    AuditActor, ClaimedJob, JobQueue, QueueConfig,
    owner_beta::{
        OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE, OwnerBetaOutcome, OwnerBetaPublicationError,
        OwnerBetaPublicationFailure, OwnerBetaPublicationOutcome, OwnerBetaRunnerConfig,
        OwnerBetaRunnerError, OwnerBetaRunnerPaths,
        daily_input::OwnerBetaDailyRecommendationInputV1,
        publish::{
            DailyClaimPreflight, preflight_owner_beta_daily_claim, settle_owner_beta_daily_failure,
        },
        runner::{
            daily_execution::{DailyExecutionContext, DailyExecutionMode, run_once_with_daily},
            daily_runtime::DailyProcessMode,
            daily_schedule::{DailyScheduleOutcome, DailyScheduleReport, schedule_daily_once},
        },
    },
};
use market_data::daily_price_only::{
    admission::PinnedDailyPolicy, encode_daily_price_policy_v1, replay::DailyReplayLimits,
};
use owner_beta_daily_support::{DailyFixture, run};
use serde_json::Value;
use sqlx::{
    ConnectOptions,
    postgres::{PgConnectOptions, PgPoolOptions},
};

type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

fn queue(f: &DailyFixture) -> JobQueue {
    JobQueue::new(
        f.worker.clone(),
        None,
        QueueConfig {
            lease: Duration::from_secs(60),
            backoff_base: Duration::from_secs(1),
        },
    )
}
fn config() -> OwnerBetaRunnerConfig {
    OwnerBetaRunnerConfig::new(
        Duration::from_secs(10),
        Duration::from_secs(60),
        Duration::from_secs(20),
    )
    .unwrap()
}
async fn reserve(f: &DailyFixture) -> TestResult<JobQueue> {
    f.ready().await?;
    schedule_daily_once(&f.worker, DailyProcessMode::OwnerOnly).await?;
    Ok(queue(f))
}
async fn claimed(
    f: &DailyFixture,
) -> TestResult<(JobQueue, ClaimedJob, OwnerBetaDailyRecommendationInputV1)> {
    let queue = reserve(f).await?;
    let claim = queue
        .claim_next_for(
            "daily-runner-fixture",
            OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE,
        )
        .await?
        .ok_or("daily claim missing")?;
    let input = serde_json::from_value(claim.job.payload_json.clone())?;
    Ok((queue, claim, input))
}
async fn state(f: &DailyFixture) -> TestResult<Value> {
    Ok(sqlx::query_scalar(
        "SELECT jsonb_build_object('jobs',(SELECT jsonb_agg(to_jsonb(j) ORDER BY j.id) FROM jobs j), \
         'attempts',(SELECT jsonb_agg(to_jsonb(a) ORDER BY a.id) FROM job_attempts a), \
         'runs',(SELECT jsonb_agg(to_jsonb(r) ORDER BY r.id) FROM owner_beta_recommendation_runs r), \
         'reservations',(SELECT jsonb_agg(to_jsonb(d) ORDER BY d.run_id) FROM owner_beta_daily_reservations d), \
         'items',(SELECT count(*) FROM owner_beta_recommendation_items))"
    ).fetch_one(&f.db.pool).await?)
}
async fn policy(f: &DailyFixture) -> TestResult<PinnedDailyPolicy> {
    let value: Value = sqlx::query_scalar("SELECT policy_json FROM owner_beta_daily_control")
        .fetch_one(&f.db.pool)
        .await?;
    let wire = serde_json::from_value(value)?;
    let bytes = encode_daily_price_policy_v1(&wire)?;
    let hash = domain::ContentHash::from_bytes(&bytes);
    Ok(PinnedDailyPolicy::from_pinned_bytes(&bytes, &hash)?)
}
fn mode<'a>(policy: &'a PinnedDailyPolicy, path: &'a Path) -> DailyExecutionMode<'a> {
    DailyExecutionMode::OwnerOnly(DailyExecutionContext {
        admission_root: path,
        trusted_owner: 0,
        reader_group: 0,
        policy,
        replay_limits: DailyReplayLimits::new(4).unwrap(),
    })
}
async fn audited_cancel(f: &DailyFixture, claim: &ClaimedJob) -> TestResult {
    let options: PgConnectOptions = f.db.role_url("audit_writer").parse()?;
    let audit = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options.disable_statement_logging())
        .await?;
    let app = JobQueue::new(f.app.clone(), Some(audit.clone()), QueueConfig::default());
    let canceled = app
        .request_cancel(claim.job.id, &AuditActor::new("owner"))
        .await;
    audit.close().await;
    assert!(matches!(canceled?, job_queue::CancelResult::Canceled(_)));
    Ok(())
}

#[tokio::test]
async fn daily_preflight_current_claim_is_read_only_and_releases_locks() {
    run("daily_preflight_readonly", |f| async move {
        let (q, claim, input) = claimed(&f).await?;
        let before = state(&f).await?;
        assert_eq!(
            preflight_owner_beta_daily_claim(&f.worker, &q, &claim, &input).await?,
            DailyClaimPreflight::Ready
        );
        assert_eq!(state(&f).await?, before);
        let payloads = f.job_payloads().await?;
        assert_eq!(payloads.len(), 1);
        f.next_schedule().await?;
        // A later cycle must replay the claimed permanent reservation. Preflight
        // may neither keep its locks nor make the scheduler create another job.
        assert_eq!(
            tokio::time::timeout(
                Duration::from_secs(5),
                schedule_daily_once(&f.worker, DailyProcessMode::OwnerOnly),
            )
            .await??,
            DailyScheduleOutcome::Completed(DailyScheduleReport {
                checked: 1,
                replayed: 1,
                ..Default::default()
            })
        );
        assert_eq!(f.job_payloads().await?, payloads);
        assert_eq!(state(&f).await?, before);
        // A control writer needs the authority lock that preflight took.
        tokio::time::timeout(
            Duration::from_secs(5),
            sqlx::query("UPDATE owner_beta_daily_control SET enabled=false").execute(&f.db.pool),
        )
        .await??;
        assert_eq!(state(&f).await?, before);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_preflight_denies_disabled_authority_and_settles_without_outputs() {
    run("daily_preflight_disabled", |f| async move {
        let (q, claim, input) = claimed(&f).await?;
        let before = state(&f).await?;
        sqlx::query("UPDATE owner_beta_daily_control SET enabled=false")
            .execute(&f.db.pool)
            .await?;
        assert_eq!(
            preflight_owner_beta_daily_claim(&f.worker, &q, &claim, &input).await,
            Err(OwnerBetaPublicationError::DailyAuthorityDenied)
        );
        assert_eq!(state(&f).await?, before);
        assert_eq!(
            settle_owner_beta_daily_failure(
                &f.worker,
                &q,
                &claim,
                &input,
                OwnerBetaPublicationFailure::DailyAuthorityDenied
            )
            .await?,
            OwnerBetaPublicationOutcome::Failed
        );
        let after = state(&f).await?;
        assert_eq!(after["reservations"], before["reservations"]);
        assert_eq!(after["runs"][0]["status"], "FAILED");
        assert_eq!(
            after["runs"][0]["error_code"],
            "OWNER_BETA_DAILY_AUTHORITY_DENIED"
        );
        assert_eq!(after["items"], 0);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_preflight_rejects_edit_restore_epoch_without_resealing() {
    run("daily_preflight_epoch", |f| async move {
        let (q, claim, input) = claimed(&f).await?;
        let before = state(&f).await?;
        sqlx::query("UPDATE user_strategy_configs SET daily_enabled=false WHERE id=$1")
            .bind(input.strategy_config_id())
            .execute(&f.app)
            .await?;
        sqlx::query("UPDATE user_strategy_configs SET daily_enabled=true WHERE id=$1")
            .bind(input.strategy_config_id())
            .execute(&f.app)
            .await?;
        assert_eq!(
            preflight_owner_beta_daily_claim(&f.worker, &q, &claim, &input).await,
            Err(OwnerBetaPublicationError::DailyAuthorityDenied)
        );
        assert_eq!(state(&f).await?, before);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_preflight_rejects_changed_payload_and_foreign_attempt() {
    run("daily_preflight_fences", |f| async move {
        let (q, claim, input) = claimed(&f).await?;
        let before = state(&f).await?;
        let mut value = serde_json::to_value(&input)?;
        value["input"]["content_sha256"] =
            serde_json::json!(domain::ContentHash::from_bytes(b"foreign"));
        let foreign = serde_json::from_value(value)?;
        assert_eq!(
            preflight_owner_beta_daily_claim(&f.worker, &q, &claim, &foreign).await,
            Err(OwnerBetaPublicationError::Integrity)
        );
        let mut other = claim.clone();
        other.attempt.attempt_no += 1;
        assert_eq!(
            preflight_owner_beta_daily_claim(&f.worker, &q, &other, &input).await,
            Err(OwnerBetaPublicationError::QueueClaimLost)
        );
        assert_eq!(state(&f).await?, before);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_preflight_finishes_audited_cancellation_exactly_once() {
    run("daily_preflight_canceled", |f| async move {
        let (q, claim, input) = claimed(&f).await?;
        let reservation = state(&f).await?["reservations"].clone();
        audited_cancel(&f, &claim).await?;
        assert_eq!(
            preflight_owner_beta_daily_claim(&f.worker, &q, &claim, &input).await?,
            DailyClaimPreflight::Canceled
        );
        let after = state(&f).await?;
        assert_eq!(after["reservations"], reservation);
        assert_eq!(after["runs"][0]["status"], "CANCELED");
        assert_eq!(after["jobs"][0]["locked_by"], Value::Null);
        assert_eq!(after["attempts"][0]["outcome"], "FAILED");
        assert_eq!(after["items"], 0);
        assert_eq!(
            preflight_owner_beta_daily_claim(&f.worker, &q, &claim, &input).await,
            Err(OwnerBetaPublicationError::QueueClaimLost)
        );
        assert_eq!(state(&f).await?, after);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_mixed_runner_disabled_mode_blocks_without_file_approval() {
    run("daily_runner_disabled", |f| async move {
        let q = reserve(&f).await?;
        let before = state(&f).await?;
        let root = tempfile::tempdir()?;
        let paths = OwnerBetaRunnerPaths {
            artifact_root: root.path().to_path_buf(),
        };
        let outcome = run_once_with_daily(
            &f.worker,
            &q,
            "daily-disabled-fixture",
            &paths,
            &config(),
            &DailyExecutionMode::Disabled,
        )
        .await?;
        assert!(matches!(
            outcome,
            OwnerBetaOutcome::Blocked {
                code: "OWNER_BETA_DAILY_EXECUTION_DISABLED",
                ..
            }
        ));
        let after = state(&f).await?;
        assert_eq!(after["reservations"], before["reservations"]);
        assert_eq!(after["runs"][0]["status"], "FAILED");
        assert_eq!(
            after["runs"][0]["error_code"],
            "OWNER_BETA_DAILY_EXECUTION_DISABLED"
        );
        assert_eq!(after["items"], 0);
        assert_eq!(
            run_once_with_daily(
                &f.worker,
                &q,
                "daily-disabled-fixture",
                &paths,
                &config(),
                &DailyExecutionMode::Disabled
            )
            .await?,
            OwnerBetaOutcome::Idle
        );
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_mixed_runner_current_thread_rejects_before_claim() {
    run("daily_runner_runtime", |f| async move {
        let q = reserve(&f).await?;
        let before = state(&f).await?;
        let policy = policy(&f).await?;
        let root = tempfile::tempdir()?;
        let paths = OwnerBetaRunnerPaths {
            artifact_root: root.path().to_path_buf(),
        };
        assert_eq!(
            run_once_with_daily(
                &f.worker,
                &q,
                "daily-runtime-fixture",
                &paths,
                &config(),
                &mode(&policy, root.path())
            )
            .await,
            Err(OwnerBetaRunnerError::UnsupportedDailyRuntime)
        );
        assert_eq!(state(&f).await?, before);
        Ok(())
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn daily_mixed_runner_foreign_policy_stops_before_base_or_admission_io() {
    run("daily_runner_policy", |f| async move {
        let q = reserve(&f).await?;
        let before = state(&f).await?;
        let policy = policy(&f).await?;
        assert_ne!(
            policy.sha256().as_str(),
            before["jobs"][0]["payload_json"]["input"]["policy_sha256"]
                .as_str()
                .unwrap()
        );
        let root = tempfile::tempdir()?;
        let missing = root.path().join("missing-admissions");
        let paths = OwnerBetaRunnerPaths {
            artifact_root: root.path().to_path_buf(),
        };
        let outcome = run_once_with_daily(
            &f.worker,
            &q,
            "daily-policy-fixture",
            &paths,
            &config(),
            &mode(&policy, &missing),
        )
        .await?;
        // An attempted base approval would yield ENTITLEMENT_DENIED instead.
        assert!(matches!(
            outcome,
            OwnerBetaOutcome::Failed {
                code: "OWNER_BETA_INPUT_INVALID",
                ..
            }
        ));
        let after = state(&f).await?;
        assert_eq!(after["reservations"], before["reservations"]);
        assert_eq!(after["items"], 0);
        assert_eq!(std::fs::read_dir(root.path())?.count(), 0);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_mixed_runner_lease_mismatch_preserves_unclaimed_job() {
    run("daily_runner_lease", |f| async move {
        let q = reserve(&f).await?;
        let before = state(&f).await?;
        let root = tempfile::tempdir()?;
        let paths = OwnerBetaRunnerPaths {
            artifact_root: root.path().to_path_buf(),
        };
        let mismatched = OwnerBetaRunnerConfig::new(
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(3),
        )?;
        assert_eq!(
            run_once_with_daily(
                &f.worker,
                &q,
                "daily-lease-fixture",
                &paths,
                &mismatched,
                &DailyExecutionMode::Disabled
            )
            .await,
            Err(OwnerBetaRunnerError::QueueUnavailable)
        );
        assert_eq!(state(&f).await?, before);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_preflight_rejects_app_admitter_and_elevated_pools() {
    run("daily_preflight_roles", |f| async move {
        let (q, claim, input) = claimed(&f).await?;
        let before = state(&f).await?;
        for pool in [&f.app, &f.admitter, &f.db.pool] {
            assert_eq!(
                preflight_owner_beta_daily_claim(pool, &q, &claim, &input).await,
                Err(OwnerBetaPublicationError::DailyAuthorityDenied)
            );
        }
        assert_eq!(state(&f).await?, before);
        Ok(())
    })
    .await;
}
