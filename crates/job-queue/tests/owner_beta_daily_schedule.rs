//! Real-role schedule/poll transactions. Synthetic SQL inputs are not approved
//! file artifacts. The dedicated PG lane must retain each named result and reject SKIP.

mod common;
mod owner_beta_daily_support;

use std::time::Duration;

use job_queue::owner_beta::daily_input::OwnerBetaDailyRecommendationInputV1;
use job_queue::owner_beta::runner::{
    daily_admission::{AdmissionPollFailure, DailyAdmissionCycle, DailyAdmissionStart},
    daily_runtime::{DailyProcessMode, DailyRuntimeError},
    daily_schedule::{DailyScheduleOutcome, DailyScheduleReport, schedule_daily_once},
};
use owner_beta_daily_support::run;
use sqlx::{
    ConnectOptions,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use uuid::Uuid;

const ON: DailyProcessMode = DailyProcessMode::OwnerOnly;

#[test]
fn daily_mode_defaults_off_and_rejects_ambiguous_configuration() {
    assert_eq!(DailyProcessMode::default(), DailyProcessMode::Disabled);
    assert_eq!(
        DailyProcessMode::parse(None),
        Ok(DailyProcessMode::Disabled)
    );
    assert_eq!(
        DailyProcessMode::parse(Some("disabled")),
        Ok(DailyProcessMode::Disabled)
    );
    assert_eq!(DailyProcessMode::parse(Some("owner_only")), Ok(ON));
    for text in ["", "true", "enabled", "OWNER_ONLY", "owner_only ", "all"] {
        assert_eq!(
            DailyProcessMode::parse(Some(text)),
            Err(DailyRuntimeError::InvalidConfiguration)
        );
    }
}

#[tokio::test]
async fn daily_disabled_mode_does_not_acquire_a_database_connection() {
    let pool = PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(1))
        .connect_lazy("postgres://unused@127.0.0.1:1/unused")
        .unwrap();
    assert_eq!(
        schedule_daily_once(&pool, DailyProcessMode::Disabled).await,
        Ok(DailyScheduleOutcome::Disabled)
    );
    assert!(matches!(
        DailyAdmissionCycle::start(&pool, DailyProcessMode::Disabled).await,
        Ok(DailyAdmissionStart::Disabled)
    ));
    assert_eq!(pool.size(), 0);
    pool.close().await;
}

#[tokio::test]
async fn daily_absent_or_disabled_control_does_not_reserve_work() {
    run("absent_or_disabled", |f| async move {
        assert_eq!(
            schedule_daily_once(&f.worker, ON).await?,
            DailyScheduleOutcome::NotDue
        );
        f.control(false).await?;
        assert_eq!(
            schedule_daily_once(&f.worker, ON).await?,
            DailyScheduleOutcome::NotDue
        );
        assert!(f.job_payloads().await?.is_empty());
        let receipts: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM owner_beta_daily_poll_receipts WHERE stage='SCHEDULE'",
        )
        .fetch_one(&f.db.pool)
        .await?;
        assert_eq!(receipts, 0);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_scheduler_reserves_canonical_snapshot_and_persistent_minute_gate() {
    run("reservation_and_minute", |f| async move {
        let config = f.ready().await?;
        assert_eq!(schedule_daily_once(&f.worker, ON).await?, DailyScheduleOutcome::Completed(
            DailyScheduleReport { checked: 1, reserved: 1, ..Default::default() }));
        let payloads = f.job_payloads().await?;
        assert_eq!(payloads.len(), 1);
        let input: OwnerBetaDailyRecommendationInputV1 = serde_json::from_value(payloads[0].clone())?;
        input.validate()?;
        assert_eq!(input.strategy_config_id(), config);
        let saved: (Uuid, String, i64, i64) = sqlx::query_as(
            "SELECT run_id, strategy_config_sha256, config_epoch, activation_epoch FROM owner_beta_daily_reservations")
            .fetch_one(&f.db.pool).await?;
        assert_eq!(saved.0, input.run_id());
        assert_eq!(saved.1, input.strategy_snapshot().config_sha256().as_str());
        assert_eq!(saved.2, input.config_epoch());
        assert_eq!(saved.3, input.activation_epoch());
        let gate: (String, bool, bool) = sqlx::query_as(
            "SELECT r.result_code, r.completed_at IS NOT NULL, s.next_poll_at >= r.started_at + interval '60 seconds' \
             FROM owner_beta_daily_poll_receipts r JOIN owner_beta_daily_poll_state s ON s.latest_receipt_id=r.receipt_id WHERE s.stage='SCHEDULE'")
            .fetch_one(&f.db.pool).await?;
        assert_eq!(gate, ("CHECKED".into(), true, true));
        assert_eq!(schedule_daily_once(&f.worker, ON).await?, DailyScheduleOutcome::NotDue);
        assert_eq!(f.job_payloads().await?, payloads);
        Ok(())
    }).await;
}

#[tokio::test]
async fn daily_scheduler_new_session_replays_permanent_reservation_without_duplicate_job() {
    run("permanent_replay", |f| async move {
        f.ready().await?;
        schedule_daily_once(&f.worker, ON).await?;
        let before = f.job_payloads().await?;
        f.next_schedule().await?;
        // Every cycle closes its session; this invocation cannot use an in-memory dedup cache.
        assert_eq!(
            schedule_daily_once(&f.worker, ON).await?,
            DailyScheduleOutcome::Completed(DailyScheduleReport {
                checked: 1,
                replayed: 1,
                ..Default::default()
            })
        );
        assert_eq!(f.job_payloads().await?, before);
        let reservations: i64 =
            sqlx::query_scalar("SELECT count(*) FROM owner_beta_daily_reservations")
                .fetch_one(&f.db.pool)
                .await?;
        assert_eq!(reservations, 1);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_scheduler_two_sessions_have_one_reservation_and_one_poll_receipt() {
    run("two_schedulers", |f| async move {
        f.ready().await?;
        let (a, b) = tokio::join!(
            schedule_daily_once(&f.worker, ON),
            schedule_daily_once(&f.worker, ON)
        );
        let results = [a?, b?];
        assert_eq!(
            results
                .iter()
                .filter(|v| matches!(v, DailyScheduleOutcome::Completed(_)))
                .count(),
            1
        );
        assert_eq!(
            results
                .iter()
                .filter(|v| matches!(v, DailyScheduleOutcome::Busy | DailyScheduleOutcome::NotDue))
                .count(),
            1
        );
        assert_eq!(f.job_payloads().await?.len(), 1);
        let receipts: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM owner_beta_daily_poll_receipts WHERE stage='SCHEDULE'",
        )
        .fetch_one(&f.db.pool)
        .await?;
        assert_eq!(receipts, 1);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_scheduler_stage_lock_precedes_the_persistent_poll_gate() {
    run("schedule_stage_lock", |f| async move {
        f.ready().await?;
        let mut holder = f.worker.acquire().await?;
        holder.close_on_drop();
        let acquired: bool = sqlx::query_scalar(
            "SELECT pg_try_advisory_lock(hashtextextended('daily-cycle:SCHEDULE',7922))",
        )
        .fetch_one(&mut *holder)
        .await?;
        assert!(acquired);
        assert_eq!(
            schedule_daily_once(&f.worker, ON).await?,
            DailyScheduleOutcome::Busy
        );
        let receipts: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM owner_beta_daily_poll_receipts WHERE stage='SCHEDULE'",
        )
        .fetch_one(&f.db.pool)
        .await?;
        assert_eq!(receipts, 0);
        holder.close().await?;
        assert!(matches!(
            schedule_daily_once(&f.worker, ON).await?,
            DailyScheduleOutcome::Completed(_)
        ));
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_poll_roles_are_direct_and_cannot_be_interchanged() {
    run("direct_roles", |f| async move {
        f.control(false).await?;
        assert_eq!(
            schedule_daily_once(&f.admitter, ON).await,
            Err(DailyRuntimeError::InvalidRole)
        );
        assert!(matches!(
            DailyAdmissionCycle::start(&f.worker, ON).await,
            Err(DailyRuntimeError::InvalidRole)
        ));
        // The scratch fixture's supervisor is a superuser, never a serving role.
        // An administrative connection must not pass the runtime boundary.
        assert!(matches!(
            DailyAdmissionCycle::start(&f.db.pool, ON).await,
            Err(DailyRuntimeError::InvalidRole)
        ));
        // A matching current_user still cannot turn a supervisor session into
        // the direct worker principal required by the runtime.
        let options: PgConnectOptions = f.db.role_url("postgres").parse()?;
        let impersonated = PgPoolOptions::new()
            .max_connections(1)
            .after_connect(|connection, _| {
                Box::pin(async move {
                    sqlx::query("SET ROLE worker").execute(connection).await?;
                    Ok(())
                })
            })
            .connect_with(options.log_statements(tracing::log::LevelFilter::Off))
            .await?;
        let identities: (String, String) = sqlx::query_as("SELECT current_user,session_user")
            .fetch_one(&impersonated)
            .await?;
        assert_eq!(identities.0, "worker");
        assert_ne!(identities.0, identities.1);
        let rejected = schedule_daily_once(&impersonated, ON).await;
        impersonated.close().await;
        assert_eq!(rejected, Err(DailyRuntimeError::InvalidRole));
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_admission_cycle_holds_lock_past_minute_and_commits_failure_receipt() {
    run("admission_cycle_lock", |f| async move {
        f.control(false).await?;
        let DailyAdmissionStart::Started(cycle) = DailyAdmissionCycle::start(&f.admitter, ON).await? else { panic!("first cycle starts"); };
        let receipt = cycle.receipt_id();
        assert!(matches!(DailyAdmissionCycle::start(&f.admitter, ON).await?, DailyAdmissionStart::Busy));
        sqlx::query("UPDATE owner_beta_daily_poll_state SET next_poll_at=clock_timestamp()-interval '1 second' WHERE stage='ADMISSION'")
            .execute(&f.db.pool).await?;
        assert!(matches!(DailyAdmissionCycle::start(&f.admitter, ON).await?, DailyAdmissionStart::Busy));
        cycle.finish_failure(AdmissionPollFailure::InputPending).await?;
        let completed: (String, bool, bool) = sqlx::query_as(
            "SELECT result_code, completed_at IS NOT NULL, admission_sha256 IS NULL AND event_sha256 IS NULL AND source_frontier_sha256 IS NULL \
             FROM owner_beta_daily_poll_receipts WHERE receipt_id=$1")
            .bind(receipt).fetch_one(&f.db.pool).await?;
        assert_eq!(completed, ("INPUT_PENDING".into(), true, true));
        let DailyAdmissionStart::Started(next) = DailyAdmissionCycle::start(&f.admitter, ON).await? else { panic!("next cycle starts after release"); };
        assert_ne!(next.receipt_id(), receipt);
        next.finish_failure(AdmissionPollFailure::IoFailed).await?;
        Ok(())
    }).await;
}

#[tokio::test]
async fn daily_abandoned_admission_leaves_started_receipt_and_releases_session_lock() {
    run("abandoned_admission", |f| async move {
        f.control(false).await?;
        let DailyAdmissionStart::Started(cycle) = DailyAdmissionCycle::start(&f.admitter, ON).await? else { panic!("cycle starts"); };
        let receipt = cycle.receipt_id();
        drop(cycle);
        let mut probe = f.admitter.acquire().await?;
        probe.close_on_drop();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock(hashtextextended('daily-cycle:ADMISSION',7922))")
                    .fetch_one(&mut *probe).await?;
                if locked { break Ok::<(),sqlx::Error>(()); }
                tokio::task::yield_now().await;
            }
        }).await??;
        probe.close().await?;
        assert!(matches!(DailyAdmissionCycle::start(&f.admitter, ON).await?, DailyAdmissionStart::NotDue));
        let started_only: bool = sqlx::query_scalar("SELECT completed_at IS NULL AND result_code IS NULL FROM owner_beta_daily_poll_receipts WHERE receipt_id=$1")
            .bind(receipt).fetch_one(&f.db.pool).await?;
        assert!(started_only);
        Ok(())
    }).await;
}

#[tokio::test]
async fn daily_schedule_missing_fresh_input_commits_checked_without_job_or_fallback() {
    run("no_fresh_input", |f| async move {
        f.control(true).await?;
        assert_eq!(
            schedule_daily_once(&f.worker, ON).await?,
            DailyScheduleOutcome::Completed(DailyScheduleReport::default())
        );
        assert!(f.job_payloads().await?.is_empty());
        let result: String = sqlx::query_scalar(
            "SELECT last_result_code FROM owner_beta_daily_poll_state WHERE stage='SCHEDULE'",
        )
        .fetch_one(&f.db.pool)
        .await?;
        assert_eq!(result, "CHECKED");
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_schedule_capacity_includes_manual_jobs_without_reserving_on_deferral() {
    run("manual_capacity", |f| async move {
        f.ready().await?;
        for _ in 0..4 {
            sqlx::query("INSERT INTO jobs(owner_user_id,job_type) VALUES($1,'fixture-capacity')")
                .bind(f.owner_id).execute(&f.app).await?;
        }
        let before: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM jobs ORDER BY id")
            .fetch_all(&f.worker).await?;
        assert_eq!(before.len(), 4);
        assert_eq!(schedule_daily_once(&f.worker, ON).await?, DailyScheduleOutcome::Completed(
            DailyScheduleReport { checked: 1, deferred: 1, ..Default::default() }));
        assert!(f.job_payloads().await?.is_empty());
        let counts: (i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM owner_beta_daily_reservations), (SELECT count(*) FROM job_attempts)")
            .fetch_one(&f.db.pool).await?;
        assert_eq!(counts, (0,0));
        let after: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM jobs ORDER BY id")
            .fetch_all(&f.worker).await?;
        assert_eq!(before, after);
        let code: String = sqlx::query_scalar("SELECT last_result_code FROM owner_beta_daily_poll_state WHERE stage='SCHEDULE'")
            .fetch_one(&f.db.pool).await?;
        assert_eq!(code, "DEFERRED");
        Ok(())
    }).await;
}

#[tokio::test]
async fn daily_schedule_rechecks_optin_and_uses_the_current_config_epoch() {
    run("current_config_epoch", |f| async move {
        let config = f.ready().await?;
        sqlx::query("UPDATE user_strategy_configs SET daily_enabled=false WHERE id=$1")
            .bind(config)
            .execute(&f.app)
            .await?;
        assert_eq!(
            schedule_daily_once(&f.worker, ON).await?,
            DailyScheduleOutcome::Completed(DailyScheduleReport::default())
        );
        assert!(f.job_payloads().await?.is_empty());
        sqlx::query("UPDATE user_strategy_configs SET daily_enabled=true WHERE id=$1")
            .bind(config)
            .execute(&f.app)
            .await?;
        let epoch: i64 =
            sqlx::query_scalar("SELECT daily_epoch FROM user_strategy_configs WHERE id=$1")
                .bind(config)
                .fetch_one(&f.app)
                .await?;
        assert!(epoch >= 2);
        f.next_schedule().await?;
        assert_eq!(
            schedule_daily_once(&f.worker, ON).await?,
            DailyScheduleOutcome::Completed(DailyScheduleReport {
                checked: 1,
                reserved: 1,
                ..Default::default()
            })
        );
        let payloads = f.job_payloads().await?;
        assert_eq!(payloads.len(), 1);
        let input: OwnerBetaDailyRecommendationInputV1 =
            serde_json::from_value(payloads[0].clone())?;
        assert_eq!(input.config_epoch(), epoch);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_schedule_latest_failed_admission_cannot_fall_back_to_previous_verified_scan() {
    run("latest_scan_failure", |f| async move {
        f.ready().await?;
        sqlx::query("UPDATE owner_beta_daily_poll_state SET next_poll_at=clock_timestamp()-interval '1 second' WHERE stage='ADMISSION'")
            .execute(&f.db.pool).await?;
        let DailyAdmissionStart::Started(cycle)=DailyAdmissionCycle::start(&f.admitter, ON).await? else { panic!("new admission cycle starts"); };
        cycle.finish_failure(AdmissionPollFailure::InputPending).await?;
        let outcomes: Vec<String> = sqlx::query_scalar("SELECT result_code FROM owner_beta_daily_poll_receipts WHERE stage='ADMISSION' ORDER BY started_at")
            .fetch_all(&f.db.pool).await?;
        assert_eq!(outcomes,vec!["VERIFIED","INPUT_PENDING"]);
        assert_eq!(schedule_daily_once(&f.worker,ON).await?,
            DailyScheduleOutcome::Completed(DailyScheduleReport::default()));
        assert!(f.job_payloads().await?.is_empty());
        Ok(())
    }).await;
}
