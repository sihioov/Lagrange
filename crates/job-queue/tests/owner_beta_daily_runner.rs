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

// Synthetic calendar/clock only inside a disposable database. No provider call,
// approved filesystem capability, or production clock override is created.
async fn future_calendar(
    f: &DailyFixture,
    input: &OwnerBetaDailyRecommendationInputV1,
    project: bool,
) -> TestResult {
    let future = input
        .as_of()
        .as_naive_date()
        .succ_opt()
        .ok_or("future date")?;
    if project {
        let mut file: Value = sqlx::query_scalar(
            "SELECT evidence_json FROM owner_beta_daily_calendar_sources WHERE owner_user_id=$1 ORDER BY calendar_evidence_sha256 LIMIT 1",
        ).bind(f.owner_id).fetch_one(&f.db.pool).await?;
        file["batch_id"] = serde_json::json!(uuid::Uuid::new_v4().to_string());
        file["batch_date"] = serde_json::json!(future.to_string());
        file["file_name"] = serde_json::json!("calendar-supersession.json");
        file["request"]["query"] = serde_json::json!([
            ["BASS_DT", future.format("%Y%m%d").to_string()],
            ["CTX_AREA_FK", ""],
            ["CTX_AREA_NK", ""]
        ]);
        let days = serde_json::json!([{
            "session_date":future.to_string(),"session_type":"TRADING","evidence_kind":"REQUESTED_TARGET",
            "source_row_ordinal":0,"source_row_sha256":format!("sha256:{}","4".repeat(64))
        }]);
        assert_eq!(project_calendar(f, &file, &days).await?, "PROJECTED");
    }
    sqlx::raw_sql(
        "CREATE TABLE public.validation_daily_clock(wall_time timestamp NOT NULL); \
         REVOKE ALL ON public.validation_daily_clock FROM PUBLIC; \
         GRANT SELECT ON public.validation_daily_clock TO migration_owner",
    )
    .execute(&f.db.pool)
    .await?;
    sqlx::query("INSERT INTO validation_daily_clock VALUES($1::date + time '16:29:59')")
        .bind(future)
        .execute(&f.db.pool)
        .await?;
    sqlx::raw_sql(
        "CREATE OR REPLACE FUNCTION public.daily_0060_expected(c public.owner_beta_daily_control) \
         RETURNS date LANGUAGE sql SET search_path=pg_catalog,pg_temp AS $$ \
         SELECT public.daily_0060_expected_at(c,(SELECT wall_time FROM public.validation_daily_clock)) $$",
    ).execute(&f.db.pool).await?;
    Ok(())
}

async fn project_calendar(f: &DailyFixture, file: &Value, days: &Value) -> TestResult<String> {
    sqlx::query(
        "UPDATE owner_beta_daily_poll_state SET next_poll_at=clock_timestamp()-interval '1 second' WHERE stage='ADMISSION'",
    ).execute(&f.db.pool).await?;
    let receipt: uuid::Uuid = sqlx::query_scalar(
        "SELECT receipt_id FROM public.take_owner_beta_daily_admission_poll_slot()",
    )
    .fetch_one(&f.admitter)
    .await?;
    let key: String = sqlx::query_scalar(
        "SELECT daily_0060_digest('owner-beta-daily-calendar-source-v1'||chr(10)||owner_user_id::text||chr(10)||policy_sha256||chr(10)||'daily-raw-reconstruction-v1'||chr(10)||daily_0060_file_bytes($1)) FROM owner_beta_daily_control",
    ).bind(file).fetch_one(&f.db.pool).await?;
    let result: String = sqlx::query_scalar(
        "SELECT disposition FROM public.project_owner_beta_daily_calendar($1,$2,$3,$4)",
    )
    .bind(receipt)
    .bind(key)
    .bind(file)
    .bind(days)
    .fetch_one(&f.admitter)
    .await?;
    if result == "PROJECTED" {
        let admitted: Value = sqlx::query_scalar(
            "SELECT evidence_json FROM owner_beta_daily_admissions WHERE owner_user_id=$1 ORDER BY range_end LIMIT 1",
        ).bind(f.owner_id).fetch_one(&f.db.pool).await?;
        let replay: String = sqlx::query_scalar(
            "SELECT disposition FROM public.admit_owner_beta_daily_input($1,$2)",
        )
        .bind(receipt)
        .bind(admitted)
        .fetch_one(&f.admitter)
        .await?;
        assert_eq!(replay, "REPLAY");
    }
    Ok(result)
}

async fn after_cutoff(f: &DailyFixture) -> TestResult {
    sqlx::query("UPDATE validation_daily_clock SET wall_time=wall_time::date+time '16:30:00'")
        .execute(&f.db.pool)
        .await?;
    Ok(())
}

async fn supersede_sql(
    connection: &mut sqlx::PgConnection,
    claim: &ClaimedJob,
    input: &OwnerBetaDailyRecommendationInputV1,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT public.cancel_superseded_owner_beta_daily_claim($1,$2,$3,$4,$5)")
        .bind(input.run_id())
        .bind(claim.job.id)
        .bind(claim.attempt.id)
        .bind(claim.attempt.attempt_no)
        .bind(&claim.worker_id)
        .fetch_one(connection)
        .await
}

async fn assert_superseded(f: &DailyFixture, before: &Value) -> TestResult {
    let after = state(f).await?;
    assert_eq!(after["reservations"], before["reservations"]);
    assert_eq!(after["items"], 0);
    assert_eq!(after["jobs"][0]["status"], "CANCELED");
    assert!(after["jobs"][0]["locked_by"].is_null());
    assert!(after["jobs"][0]["locked_at"].is_null());
    assert_eq!(after["attempts"][0]["outcome"], "FAILED");
    assert_eq!(
        after["attempts"][0]["error_code"],
        "DAILY_SESSION_SUPERSEDED"
    );
    assert_eq!(after["runs"][0]["status"], "CANCELED");
    assert_eq!(after["runs"][0]["error_code"], "DAILY_SESSION_SUPERSEDED");
    assert!(after["runs"][0]["factor_snapshot_sha256"].is_null());
    assert!(after["runs"][0]["target_snapshot_sha256"].is_null());
    assert!(after["runs"][0]["cash_weight"].is_null());
    let receipts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM owner_beta_daily_publication_claims")
            .fetch_one(&f.db.pool)
            .await?;
    assert_eq!(receipts, 0);
    Ok(())
}

async fn wait_blocked(pool: &sqlx::PgPool, pid: i32) -> TestResult {
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let blocked: bool = sqlx::query_scalar("SELECT cardinality(pg_blocking_pids($1))>0")
                .bind(pid)
                .fetch_one(pool)
                .await?;
            if blocked {
                return Ok::<_, sqlx::Error>(());
            }
            tokio::task::yield_now().await;
        }
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn daily_supersession_preflight_cancels_and_replays_only_exact_identity() {
    run("daily_supersession_preflight", |f| async move {
        let (q, claim, input) = claimed(&f).await?;
        future_calendar(&f, &input, true).await?;
        let before = state(&f).await?;
        assert_eq!(
            preflight_owner_beta_daily_claim(&f.worker, &q, &claim, &input).await?,
            DailyClaimPreflight::Ready
        );
        assert_eq!(state(&f).await?, before);
        after_cutoff(&f).await?;
        assert_eq!(
            preflight_owner_beta_daily_claim(&f.worker, &q, &claim, &input).await?,
            DailyClaimPreflight::Canceled
        );
        assert_superseded(&f, &before).await?;
        let completed = state(&f).await?;
        let mut connection = f.worker.acquire().await?;
        assert!(supersede_sql(&mut connection, &claim, &input).await?);
        drop(connection);
        assert_eq!(state(&f).await?, completed);
        let mut wrong = claim.clone();
        wrong.attempt.id = uuid::Uuid::new_v4();
        let mut connection = f.worker.acquire().await?;
        let error = supersede_sql(&mut connection, &wrong, &input)
            .await
            .unwrap_err();
        assert_eq!(
            error.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("23514")
        );
        drop(connection);
        assert_eq!(state(&f).await?, completed);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_supersession_failure_cleanup_cancels_instead_of_retrying() {
    run("daily_supersession_failure", |f| async move {
        let (q, claim, input) = claimed(&f).await?;
        future_calendar(&f, &input, true).await?;
        after_cutoff(&f).await?;
        let before = state(&f).await?;
        assert_eq!(
            settle_owner_beta_daily_failure(
                &f.worker,
                &q,
                &claim,
                &input,
                OwnerBetaPublicationFailure::ComputationUnavailable
            )
            .await?,
            OwnerBetaPublicationOutcome::Canceled
        );
        assert_superseded(&f, &before).await?;
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_supersession_cleanup_remains_possible_after_authority_revocation() {
    run("daily_supersession_revoked", |f| async move {
        let (q, claim, input) = claimed(&f).await?;
        future_calendar(&f, &input, true).await?;
        after_cutoff(&f).await?;
        sqlx::raw_sql(
            "UPDATE owner_beta_daily_control SET enabled=false; \
             UPDATE data_entitlements SET status='REVOKED'; \
             UPDATE user_strategy_configs SET daily_enabled=false",
        )
        .execute(&f.db.pool)
        .await?;
        let before = state(&f).await?;
        assert_eq!(
            preflight_owner_beta_daily_claim(&f.worker, &q, &claim, &input).await?,
            DailyClaimPreflight::Canceled
        );
        assert_superseded(&f, &before).await?;
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_supersession_missing_calendar_does_not_invent_a_newer_session() {
    run("daily_supersession_missing", |f| async move {
        let (q, claim, input) = claimed(&f).await?;
        future_calendar(&f, &input, false).await?;
        after_cutoff(&f).await?;
        let before = state(&f).await?;
        let mut connection = f.worker.acquire().await?;
        assert!(!supersede_sql(&mut connection, &claim, &input).await?);
        drop(connection);
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
async fn daily_supersession_conflicting_calendar_does_not_cancel() {
    run("daily_supersession_conflict", |f| async move {
        let (q,claim,input)=claimed(&f).await?;
        future_calendar(&f,&input,true).await?;
        let future=input.as_of().as_naive_date().succ_opt().ok_or("future")?;
        let mut file:Value=sqlx::query_scalar(
            "SELECT evidence_json FROM owner_beta_daily_calendar_sources WHERE requested_target=$1",
        ).bind(future).fetch_one(&f.db.pool).await?;
        file["batch_id"]=serde_json::json!(uuid::Uuid::new_v4().to_string());
        let days=serde_json::json!([{
            "session_date":future.to_string(),"session_type":"CLOSED","evidence_kind":"REQUESTED_TARGET",
            "source_row_ordinal":0,"source_row_sha256":format!("sha256:{}","4".repeat(64))
        }]);
        assert_eq!(project_calendar(&f,&file,&days).await?,"BLOCKED");
        after_cutoff(&f).await?;
        let before=state(&f).await?;
        let mut connection=f.worker.acquire().await?;
        assert!(!supersede_sql(&mut connection,&claim,&input).await?);
        drop(connection);
        assert_eq!(preflight_owner_beta_daily_claim(
            &f.worker,&q,&claim,&input
        ).await,Err(OwnerBetaPublicationError::DailyAuthorityDenied));
        assert_eq!(state(&f).await?,before);
        Ok(())
    }).await;
}

#[tokio::test]
async fn daily_supersession_transaction_rollback_restores_every_output_and_lease() {
    run("daily_supersession_rollback", |f| async move {
        let (q, claim, input) = claimed(&f).await?;
        future_calendar(&f, &input, true).await?;
        after_cutoff(&f).await?;
        let before = state(&f).await?;
        let mut transaction = f.worker.begin().await?;
        assert!(supersede_sql(&mut transaction, &claim, &input).await?);
        sqlx::query("SET CONSTRAINTS ALL IMMEDIATE")
            .execute(&mut *transaction)
            .await?;
        transaction.rollback().await?;
        assert_eq!(state(&f).await?, before);
        assert_eq!(
            preflight_owner_beta_daily_claim(&f.worker, &q, &claim, &input).await?,
            DailyClaimPreflight::Canceled
        );
        assert_superseded(&f, &before).await?;
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_supersession_samples_cutoff_after_waiting_for_control() {
    run("daily_supersession_cutoff_wait", |f| async move {
        let (_q, claim, input) = claimed(&f).await?;
        future_calendar(&f, &input, true).await?;
        let before = state(&f).await?;
        let mut blocker = f.db.pool.begin().await?;
        sqlx::query("SELECT 1 FROM owner_beta_daily_control FOR UPDATE")
            .execute(&mut *blocker)
            .await?;
        let mut connection = f.worker.acquire().await?;
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *connection)
            .await?;
        let waiting =
            tokio::spawn(async move { supersede_sql(&mut connection, &claim, &input).await });
        let observed = wait_blocked(&f.db.pool, pid).await;
        if observed.is_err() {
            blocker.rollback().await?;
            waiting.abort();
            let _ = waiting.await;
            observed?;
            unreachable!();
        }
        sqlx::query("UPDATE validation_daily_clock SET wall_time=wall_time::date+time '16:30:00'")
            .execute(&mut *blocker)
            .await?;
        blocker.commit().await?;
        assert!(tokio::time::timeout(Duration::from_secs(8), waiting).await???);
        assert_superseded(&f, &before).await?;
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_supersession_foreign_worker_attempt_and_payload_cannot_cancel() {
    run("daily_supersession_identity", |f| async move {
        let (q, claim, input) = claimed(&f).await?;
        future_calendar(&f, &input, true).await?;
        after_cutoff(&f).await?;
        let before = state(&f).await?;
        for wrong in [
            {
                let mut x = claim.clone();
                x.worker_id = "foreign-worker".into();
                x
            },
            {
                let mut x = claim.clone();
                x.attempt.attempt_no += 1;
                x
            },
            {
                let mut x = claim.clone();
                x.attempt.id = uuid::Uuid::new_v4();
                x
            },
        ] {
            let mut connection = f.worker.acquire().await?;
            let error = supersede_sql(&mut connection, &wrong, &input)
                .await
                .unwrap_err();
            assert_eq!(
                error.as_database_error().and_then(|e| e.code()).as_deref(),
                Some("23514")
            );
            drop(connection);
            assert_eq!(state(&f).await?, before);
        }
        let mut foreign = serde_json::to_value(&input)?;
        foreign["input"]["content_sha256"] =
            serde_json::json!(domain::ContentHash::from_bytes(b"foreign"));
        let foreign = serde_json::from_value(foreign)?;
        assert_eq!(
            preflight_owner_beta_daily_claim(&f.worker, &q, &claim, &foreign).await,
            Err(OwnerBetaPublicationError::Integrity)
        );
        assert_eq!(state(&f).await?, before);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_supersession_expired_real_lease_cannot_settle() {
    run("daily_supersession_expired", |f| async move {
        let (_q,claim,input)=claimed(&f).await?;
        future_calendar(&f,&input,true).await?;
        after_cutoff(&f).await?;
        sqlx::query("UPDATE owner_beta_daily_control SET lease_seconds=1")
            .execute(&f.db.pool).await?;
        tokio::time::timeout(Duration::from_secs(5),async {
            loop {
                let expired:bool=sqlx::query_scalar(
                    "SELECT locked_at <= clock_timestamp()-interval '1 second' FROM jobs WHERE id=$1",
                ).bind(claim.job.id).fetch_one(&f.worker).await?;
                if expired {return Ok::<_,sqlx::Error>(());}
                tokio::task::yield_now().await;
            }
        }).await??;
        let before=state(&f).await?;
        let mut connection=f.worker.acquire().await?;
        let error=supersede_sql(&mut connection,&claim,&input).await.unwrap_err();
        assert_eq!(error.as_database_error().and_then(|e|e.code()).as_deref(),Some("55000"));
        drop(connection);
        assert_eq!(state(&f).await?,before);
        Ok(())
    }).await;
}

#[tokio::test]
async fn daily_supersession_rejects_other_real_principals_and_restores_actor() {
    run("daily_supersession_principals", |f| async move {
        let (_q, claim, input) = claimed(&f).await?;
        future_calendar(&f, &input, true).await?;
        after_cutoff(&f).await?;
        let before = state(&f).await?;
        for pool in [&f.app, &f.admitter, &f.db.pool] {
            let mut connection = pool.acquire().await?;
            let error = supersede_sql(&mut connection, &claim, &input)
                .await
                .unwrap_err();
            assert_eq!(
                error.as_database_error().and_then(|e| e.code()).as_deref(),
                Some("42501")
            );
        }
        let options: PgConnectOptions = f.db.role_url("postgres").parse()?;
        let elevated = PgPoolOptions::new()
            .max_connections(1)
            .after_connect(|connection, _| {
                Box::pin(async move {
                    sqlx::query("SET ROLE worker").execute(connection).await?;
                    Ok(())
                })
            })
            .connect_with(options.disable_statement_logging())
            .await?;
        let mut impersonated = elevated.acquire().await?;
        let identities: (String, String) = sqlx::query_as("SELECT current_user,session_user")
            .fetch_one(&mut *impersonated)
            .await?;
        assert_eq!(identities.0, "worker");
        assert_ne!(identities.0, identities.1);
        let error = supersede_sql(&mut impersonated, &claim, &input)
            .await
            .unwrap_err();
        drop(impersonated);
        elevated.close().await;
        assert_eq!(
            error.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("42501")
        );
        assert_eq!(state(&f).await?, before);
        let mut tx = f.worker.begin().await?;
        let actor = uuid::Uuid::new_v4().to_string();
        sqlx::query("SELECT set_config('app.actor_user_id',$1,true)")
            .bind(&actor)
            .execute(&mut *tx)
            .await?;
        assert!(supersede_sql(&mut tx, &claim, &input).await?);
        let after_actor: String = sqlx::query_scalar("SELECT current_setting('app.actor_user_id')")
            .fetch_one(&mut *tx)
            .await?;
        assert_eq!(after_actor, actor);
        tx.commit().await?;
        assert_superseded(&f, &before).await?;
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_supersession_additive_migration_down_up_preserves_populated_rows() {
    static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");
    run("daily_supersession_migration", |f| async move {
        let (q,claim,input)=claimed(&f).await?;
        future_calendar(&f,&input,true).await?;
        after_cutoff(&f).await?;
        assert_eq!(preflight_owner_beta_daily_claim(
            &f.worker,&q,&claim,&input
        ).await?,DailyClaimPreflight::Canceled);
        let before=state(&f).await?;
        let signature="public.cancel_superseded_owner_beta_daily_claim(uuid,uuid,uuid,integer,text)";
        let properties:(String,bool,Vec<String>)=sqlx::query_as(
            "SELECT pg_get_userbyid(proowner),prosecdef,proconfig FROM pg_proc WHERE oid=$1::regprocedure",
        ).bind(signature).fetch_one(&f.db.pool).await?;
        assert_eq!(properties.0,"migration_owner");
        assert!(properties.1);
        assert!(properties.2.iter().any(|x|x=="search_path=pg_catalog, pg_temp"));
        assert!(properties.2.iter().any(|x|x=="lock_timeout=5s"));
        assert!(properties.2.iter().any(|x|x=="statement_timeout=30s"));
        for role in ["app","worker","admin","audit_writer","research_writer","daily_input_admitter"] {
            let permitted:bool=sqlx::query_scalar("SELECT has_function_privilege($1,$2,'EXECUTE')")
                .bind(role).bind(signature).fetch_one(&f.db.pool).await?;
            assert_eq!(permitted,role=="worker");
        }
        let public_grants:i64=sqlx::query_scalar(
            "SELECT count(*) FROM pg_proc p, LATERAL aclexplode(COALESCE(p.proacl,acldefault('f',p.proowner))) a WHERE p.oid=$1::regprocedure AND a.grantee=0",
        ).bind(signature).fetch_one(&f.db.pool).await?;
        assert_eq!(public_grants,0);
        MIGRATOR.undo(&f.db.pool,60).await?;
        let absent:bool=sqlx::query_scalar("SELECT to_regprocedure($1) IS NULL")
            .bind(signature).fetch_one(&f.db.pool).await?;
        assert!(absent);
        assert_eq!(state(&f).await?,before);
        MIGRATOR.run(&f.db.pool).await?;
        let mut connection=f.worker.acquire().await?;
        assert!(supersede_sql(&mut connection,&claim,&input).await?);
        drop(connection);
        assert_eq!(state(&f).await?,before);
        let ledger:(i64,i64)=sqlx::query_as(
            "SELECT count(*),max(version) FROM _sqlx_migrations WHERE success",
        ).fetch_one(&f.db.pool).await?;
        assert_eq!(ledger,(61,61));
        Ok(())
    }).await;
}

#[tokio::test]
async fn daily_supersession_does_not_replace_an_audited_manual_cancel_reason() {
    run("daily_supersession_manual_cancel", |f| async move {
        let (q, claim, input) = claimed(&f).await?;
        future_calendar(&f, &input, true).await?;
        after_cutoff(&f).await?;
        audited_cancel(&f, &claim).await?;
        assert_eq!(
            preflight_owner_beta_daily_claim(&f.worker, &q, &claim, &input).await?,
            DailyClaimPreflight::Canceled
        );
        let after = state(&f).await?;
        assert_eq!(after["runs"][0]["error_code"], "CANCELED");
        assert_eq!(after["jobs"][0]["status"], "CANCELED");
        assert!(after["jobs"][0]["locked_by"].is_null());
        assert!(after["jobs"][0]["locked_at"].is_null());
        assert_eq!(after["items"], 0);
        Ok(())
    })
    .await;
}

async fn future_source_conflict(
    f: &DailyFixture,
    input: &OwnerBetaDailyRecommendationInputV1,
) -> TestResult<(uuid::Uuid, Value)> {
    sqlx::query(
        "UPDATE owner_beta_daily_poll_state SET next_poll_at=clock_timestamp()-interval '1 second' WHERE stage='ADMISSION'",
    ).execute(&f.db.pool).await?;
    let receipt: uuid::Uuid = sqlx::query_scalar(
        "SELECT receipt_id FROM public.take_owner_beta_daily_admission_poll_slot()",
    )
    .fetch_one(&f.admitter)
    .await?;
    let future = input
        .as_of()
        .as_naive_date()
        .succ_opt()
        .ok_or("future date")?;
    let event = serde_json::json!({
        "source_kind":"CALENDAR","range_start":future.to_string(),"range_end":future.to_string(),
        "code":"SOURCE_CONFLICT","evidence_sha256":domain::ContentHash::from_bytes(b"supersession-source-race")
    });
    Ok((receipt, event))
}

#[tokio::test]
async fn daily_supersession_observes_source_conflict_committed_while_waiting() {
    run("daily_supersession_source_before", |f| async move {
        let (_q, claim, input) = claimed(&f).await?;
        future_calendar(&f, &input, true).await?;
        after_cutoff(&f).await?;
        let (receipt, event) = future_source_conflict(&f, &input).await?;
        let mut blocker = f.admitter.begin().await?;
        let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *blocker)
            .await?;
        let result: String = sqlx::query_scalar(
            "SELECT disposition FROM public.block_owner_beta_daily_input($1,$2)",
        )
        .bind(receipt)
        .bind(event)
        .fetch_one(&mut *blocker)
        .await?;
        assert_eq!(result, "BLOCKED");
        let before = state(&f).await?;
        let mut connection = f.worker.acquire().await?;
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *connection)
            .await?;
        let waiting =
            tokio::spawn(async move { supersede_sql(&mut connection, &claim, &input).await });
        let observed = wait_blocked(&f.db.pool, pid).await;
        if observed.is_err() {
            blocker.rollback().await?;
            waiting.abort();
            let _ = waiting.await;
            observed?;
            unreachable!();
        }
        let blockers: Vec<i32> = sqlx::query_scalar("SELECT pg_blocking_pids($1)")
            .bind(pid)
            .fetch_one(&f.db.pool)
            .await?;
        assert!(blockers.contains(&blocker_pid));
        blocker.commit().await?;
        assert!(!tokio::time::timeout(Duration::from_secs(8), waiting).await???);
        assert_eq!(state(&f).await?, before);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_supersession_holds_source_fence_until_atomic_commit() {
    run("daily_supersession_source_after", |f| async move {
        let (_q, claim, input) = claimed(&f).await?;
        future_calendar(&f, &input, true).await?;
        after_cutoff(&f).await?;
        let (receipt, event) = future_source_conflict(&f, &input).await?;
        let before = state(&f).await?;
        let mut cancel = f.worker.begin().await?;
        let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *cancel)
            .await?;
        assert!(supersede_sql(&mut cancel, &claim, &input).await?);
        let mut event_connection = f.admitter.acquire().await?;
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *event_connection)
            .await?;
        let waiting = tokio::spawn(async move {
            sqlx::query_scalar::<_, String>(
                "SELECT disposition FROM public.block_owner_beta_daily_input($1,$2)",
            )
            .bind(receipt)
            .bind(event)
            .fetch_one(&mut *event_connection)
            .await
        });
        let observed = wait_blocked(&f.db.pool, pid).await;
        if observed.is_err() {
            cancel.rollback().await?;
            waiting.abort();
            let _ = waiting.await;
            observed?;
            unreachable!();
        }
        let blockers: Vec<i32> = sqlx::query_scalar("SELECT pg_blocking_pids($1)")
            .bind(pid)
            .fetch_one(&f.db.pool)
            .await?;
        assert!(blockers.contains(&blocker_pid));
        cancel.commit().await?;
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(8), waiting).await???,
            "BLOCKED"
        );
        assert_superseded(&f, &before).await?;
        let completed = state(&f).await?;
        let mut connection = f.worker.acquire().await?;
        assert!(supersede_sql(&mut connection, &claim, &input).await?);
        drop(connection);
        assert_eq!(state(&f).await?, completed);
        Ok(())
    })
    .await;
}
