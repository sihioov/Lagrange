//! Real-role schedule/poll transactions. Synthetic SQL inputs are not approved
//! file artifacts. The dedicated PG lane must retain each named result and reject SKIP.

mod common;
mod owner_beta_daily_support;

use std::time::Duration;

use job_queue::owner_beta::daily_input::OwnerBetaDailyRecommendationInputV1;
use job_queue::owner_beta::runner::daily_admission as readback;
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
            .connect_with(options.disable_statement_logging())
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

type ReadbackResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn readback_hash(byte: char) -> domain::ContentHash {
    domain::ContentHash::parse(&format!("sha256:{}", byte.to_string().repeat(64))).unwrap()
}
async fn readback_cycle(
    f: &owner_beta_daily_support::DailyFixture,
) -> ReadbackResult<DailyAdmissionCycle> {
    match DailyAdmissionCycle::start(&f.admitter, ON).await? {
        DailyAdmissionStart::Started(cycle) => Ok(cycle),
        _ => Err("readback cycle did not start".into()),
    }
}
async fn readback_next(f: &owner_beta_daily_support::DailyFixture) -> ReadbackResult {
    sqlx::query("UPDATE owner_beta_daily_poll_state SET next_poll_at=clock_timestamp()-interval '1 second' WHERE stage='ADMISSION'")
        .execute(&f.db.pool).await?;
    Ok(())
}
async fn readback_existing(
    f: &owner_beta_daily_support::DailyFixture,
) -> ReadbackResult<(Uuid, domain::ContentHash)> {
    let (id,hash): (Uuid,String) = sqlx::query_as(
        "SELECT receipt_id,admission_sha256 FROM owner_beta_daily_poll_receipts WHERE stage='ADMISSION' AND result_code='VERIFIED'")
        .fetch_one(&f.db.pool).await?;
    Ok((id, domain::ContentHash::parse(&hash)?))
}
async fn readback_snapshot(f: &owner_beta_daily_support::DailyFixture) -> ReadbackResult<String> {
    Ok(sqlx::query_scalar(
        "SELECT public.daily_0060_digest(jsonb_build_object(\
         'admissions',(SELECT jsonb_agg(to_jsonb(a) ORDER BY a.admission_sha256) FROM owner_beta_daily_admissions a),\
         'receipts',(SELECT jsonb_agg(to_jsonb(r) ORDER BY r.receipt_id) FROM owner_beta_daily_poll_receipts r),\
         'state',(SELECT jsonb_agg(to_jsonb(s) ORDER BY s.stage) FROM owner_beta_daily_poll_state s),\
         'events',(SELECT jsonb_agg(to_jsonb(e) ORDER BY e.event_sha256) FROM owner_beta_daily_source_events e))::text)")
        .fetch_one(&f.db.pool).await?)
}

#[tokio::test]
async fn daily_admitter_context_is_pending_scoped_and_default_off() {
    run("readback_empty_context", |f| async move {
        f.control(false).await?;
        let mut cycle = readback_cycle(&f).await?;
        let before = readback_snapshot(&f).await?;
        let context = cycle.context().await?;
        assert_eq!(context.owner(), f.owner_id);
        assert_eq!(context.policy_sha256(), &readback_hash('b'));
        assert_eq!(context.activation_epoch(), 0);
        assert!(!context.enabled());
        assert_eq!(context.entitlement_reference(), "daily-fixture");
        assert_eq!(context.entitlement_sha256(), &readback_hash('a'));
        assert_eq!(context.first_auto_session().to_string(), "2026-08-29");
        assert!(context.latest_admission_sha256().is_none());
        assert!(!context.source_blocked());
        let receipt = readback::read_daily_admission_receipt(&f.admitter, cycle.receipt_id())
            .await?
            .ok_or("missing receipt")?;
        assert_eq!(receipt.receipt_id(), cycle.receipt_id());
        assert!(receipt.matches_current_cycle());
        assert_eq!(receipt.owner(), context.owner());
        assert_eq!(receipt.policy_sha256(), context.policy_sha256());
        assert!(context.observed_at() >= receipt.started_at());
        assert_eq!(
            receipt.outcome(),
            &readback::RecordedAdmissionOutcome::Started
        );
        assert_eq!(readback_snapshot(&f).await?, before);
        cycle
            .finish_failure(AdmissionPollFailure::InputPending)
            .await?;
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_admitter_context_returns_exact_committed_tip() {
    run("readback_committed_context", |f| async move {
        f.ready().await?;
        let (_, hash) = readback_existing(&f).await?;
        readback_next(&f).await?;
        let mut cycle = readback_cycle(&f).await?;
        let context = cycle.context().await?;
        assert_eq!(context.latest_admission_sha256(), Some(&hash));
        let input = readback::read_committed_daily_input(&f.admitter, &hash)
            .await?
            .ok_or("missing committed input")?;
        assert_eq!(input.owner(), context.owner());
        assert_eq!(input.pins().policy_sha256, *context.policy_sha256());
        assert_eq!(input.pins().admission_sha256, hash);
        assert!(input.parent_admission_sha256().is_none());
        assert_eq!(input.range_start().to_string(), "2016-08-29");
        assert!(input.range_end() > input.range_start());
        assert_eq!(input.session_count(), 2453);
        assert!(input.unblocked_at_read());
        cycle
            .finish_failure(AdmissionPollFailure::InputPending)
            .await?;
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_admitter_context_rejects_completed_and_superseded_receipts() {
    for completed in [true, false] {
        run("readback_context_lifecycle", move |f| async move {
            f.control(false).await?;
            let mut cycle = readback_cycle(&f).await?;
            let id = cycle.receipt_id();
            if completed {
                sqlx::query("SELECT finish_owner_beta_daily_admission_poll($1,'IO_FAILED',NULL)")
                    .bind(id)
                    .execute(&f.admitter)
                    .await?;
            } else {
                readback_next(&f).await?;
                let next: Uuid = sqlx::query_scalar(
                    "SELECT receipt_id FROM take_owner_beta_daily_admission_poll_slot()",
                )
                .fetch_one(&f.admitter)
                .await?;
                assert_ne!(next, id);
            }
            assert!(matches!(
                cycle.context().await,
                Err(DailyRuntimeError::DatabaseIntegrity)
            ));
            let record = readback::read_daily_admission_receipt(&f.admitter, id)
                .await?
                .ok_or("missing old receipt")?;
            assert_eq!(record.completed_at().is_some(), completed);
            assert_eq!(record.matches_current_cycle(), completed);
            drop(cycle);
            Ok(())
        })
        .await;
    }
}

#[tokio::test]
async fn daily_admitter_readback_requires_direct_principal_and_no_table_grants() {
    run("readback_direct_principal", |f| async move {
        f.ready().await?;
        let (id,hash)=readback_existing(&f).await?;
        for pool in [&f.app,&f.worker,&f.db.pool] {
            for query in [
                "SELECT count(*) FROM public.read_owner_beta_daily_admission_context($1)",
                "SELECT count(*) FROM public.read_owner_beta_daily_admission_receipt($1)",
            ] {
                let error=sqlx::query_scalar::<_,i64>(query).bind(id).fetch_one(pool).await.unwrap_err();
                assert_eq!(error.as_database_error().and_then(|e|e.code()).as_deref(),Some("42501"));
            }
            let error=sqlx::query_scalar::<_,i64>("SELECT count(*) FROM read_owner_beta_daily_admitted_input($1)")
                .bind(hash.as_str()).fetch_one(pool).await.unwrap_err();
            assert_eq!(error.as_database_error().and_then(|e|e.code()).as_deref(),Some("42501"));
        }
        let mut elevated=f.db.pool.acquire().await?;
        elevated.close_on_drop();
        sqlx::query("SET ROLE daily_input_admitter").execute(&mut *elevated).await?;
        let error=sqlx::query_scalar::<_,i64>("SELECT count(*) FROM read_owner_beta_daily_admission_receipt($1)")
            .bind(id).fetch_one(&mut *elevated).await.unwrap_err();
        assert_eq!(error.as_database_error().and_then(|e|e.code()).as_deref(),Some("42501"));
        elevated.close().await?;
        for table in ["owner_beta_daily_control","owner_beta_daily_admissions","owner_beta_daily_poll_receipts","owner_beta_daily_source_events"] {
            let allowed:bool=sqlx::query_scalar(
                "SELECT has_table_privilege('daily_input_admitter',$1,'SELECT,INSERT,UPDATE,DELETE')")
                .bind(format!("public.{table}")).fetch_one(&f.db.pool).await?;
            assert!(!allowed);
        }
        Ok(())
    }).await;
}

#[tokio::test]
async fn daily_admitter_readback_rechecks_owner_and_exact_rights() {
    for revoke_owner in [true, false] {
        run("readback_current_authority", move |f| async move {
            f.ready().await?;
            let (id, hash) = readback_existing(&f).await?;
            readback_next(&f).await?;
            let mut cycle = readback_cycle(&f).await?;
            if revoke_owner {
                sqlx::query("DELETE FROM user_roles WHERE user_id=$1 AND role_id='owner'")
                    .bind(f.owner_id)
                    .execute(&f.db.pool)
                    .await?;
            } else {
                sqlx::query("UPDATE data_entitlements SET status='REVOKED'")
                    .execute(&f.db.pool)
                    .await?;
            }
            assert!(matches!(
                cycle.context().await,
                Err(DailyRuntimeError::DatabaseIntegrity)
            ));
            assert!(matches!(
                readback::read_committed_daily_input(&f.admitter, &hash).await,
                Err(DailyRuntimeError::DatabaseIntegrity)
            ));
            assert!(matches!(
                readback::read_daily_admission_receipt(&f.admitter, id).await,
                Err(DailyRuntimeError::DatabaseIntegrity)
            ));
            drop(cycle);
            Ok(())
        })
        .await;
    }
}

#[tokio::test]
async fn daily_admitter_readback_filters_foreign_owner_policy_and_stage() {
    run("readback_scoped_rows", |f| async move {
        f.ready().await?;
        let (old_id,old_hash)=readback_existing(&f).await?;
        let foreign=Uuid::new_v4();
        sqlx::query("INSERT INTO users(id,issuer,subject,email) VALUES($1,'https://daily.invalid','readback-foreign','readback-foreign@example.invalid')")
            .bind(foreign).execute(&f.db.pool).await?;
        for (owner,policy,stage) in [
            (foreign,readback_hash('b'),"ADMISSION"),
            (f.owner_id,readback_hash('c'),"ADMISSION"),
            (f.owner_id,readback_hash('b'),"SCHEDULE"),
        ] {
            let id:Uuid=sqlx::query_scalar(
                "INSERT INTO owner_beta_daily_poll_receipts(stage,owner_user_id,policy_sha256,activation_epoch) VALUES($1,$2,$3,1) RETURNING receipt_id")
                .bind(stage).bind(owner).bind(policy.as_str()).fetch_one(&f.db.pool).await?;
            assert!(readback::read_daily_admission_receipt(&f.admitter,id).await?.is_none());
            let error=sqlx::query_scalar::<_,i64>("SELECT count(*) FROM read_owner_beta_daily_admission_context($1)")
                .bind(id).fetch_one(&f.admitter).await.unwrap_err();
            assert_eq!(error.as_database_error().and_then(|e|e.code()).as_deref(),Some("23514"));
        }
        sqlx::query(
            "INSERT INTO owner_beta_daily_admissions(admission_sha256,owner_user_id,policy_sha256,content_sha256,manifest_sha256,\
             parent_admission_sha256,range_start,range_end,session_count,bar_count,evidence_json)\
             SELECT $1,$2,policy_sha256,content_sha256,manifest_sha256,parent_admission_sha256,range_start,range_end,session_count,bar_count,evidence_json \
             FROM owner_beta_daily_admissions WHERE admission_sha256=$3")
            .bind(readback_hash('c').as_str()).bind(foreign).bind(old_hash.as_str()).execute(&f.db.pool).await?;
        assert!(readback::read_committed_daily_input(&f.admitter,&readback_hash('c')).await?.is_none());
        sqlx::query("UPDATE owner_beta_daily_control SET policy_sha256=$1")
            .bind(readback_hash('d').as_str()).execute(&f.db.pool).await?;
        assert!(readback::read_daily_admission_receipt(&f.admitter,old_id).await?.is_none());
        assert!(readback::read_committed_daily_input(&f.admitter,&old_hash).await?.is_none());
        readback_next(&f).await?;
        let mut cycle=readback_cycle(&f).await?;
        assert!(matches!(cycle.context().await,Err(DailyRuntimeError::DatabaseIntegrity)));
        drop(cycle);
        Ok(())
    }).await;
}

#[tokio::test]
async fn daily_admitter_receipt_recovery_distinguishes_started_and_completed_failure() {
    for failure in [
        AdmissionPollFailure::InputPending,
        AdmissionPollFailure::InputBlocked,
        AdmissionPollFailure::IoFailed,
    ] {
        run("readback_failure_reply", move |f| async move {
            f.control(false).await?;
            let cycle = readback_cycle(&f).await?;
            let id = cycle.receipt_id();
            let pending = readback::read_daily_admission_receipt(&f.admitter, id)
                .await?
                .ok_or("missing receipt")?;
            assert_eq!(
                pending.outcome(),
                &readback::RecordedAdmissionOutcome::Started
            );
            cycle.finish_failure(failure).await?;
            // Ignore the original finish response; a new query recovers exact committed state.
            let before = readback_snapshot(&f).await?;
            let complete = readback::read_daily_admission_receipt(&f.admitter, id)
                .await?
                .ok_or("missing completion")?;
            assert_eq!(
                complete.outcome(),
                &readback::RecordedAdmissionOutcome::Failed {
                    failure,
                    source_frontier: None,
                    event: None
                }
            );
            assert!(complete.completed_at().is_some());
            assert!(complete.matches_current_cycle());
            assert_eq!(readback_snapshot(&f).await?, before);
            assert!(matches!(
                DailyAdmissionCycle::start(&f.admitter, ON).await?,
                DailyAdmissionStart::NotDue
            ));
            Ok(())
        })
        .await;
    }
}

#[tokio::test]
async fn daily_admitter_receipt_recovery_matches_verified_commit_without_mutation() {
    run("readback_verified_reply", |f| async move {
        f.ready().await?;
        let (id, hash) = readback_existing(&f).await?;
        let before = readback_snapshot(&f).await?;
        let receipt = readback::read_daily_admission_receipt(&f.admitter, id)
            .await?
            .ok_or("missing receipt")?;
        let input = readback::read_committed_daily_input(&f.admitter, &hash)
            .await?
            .ok_or("missing input")?;
        assert_eq!(
            receipt.outcome(),
            &readback::RecordedAdmissionOutcome::Verified {
                admission: hash.clone(),
                source_frontier: input.source_frontier_sha256().clone(),
            }
        );
        assert_eq!(receipt.owner(), input.owner());
        assert_eq!(receipt.policy_sha256(), &input.pins().policy_sha256);
        assert!(receipt.completed_at().is_some());
        assert!(receipt.matches_current_cycle());
        for _ in 0..2 {
            assert!(
                readback::read_daily_admission_receipt(&f.admitter, id)
                    .await?
                    .is_some()
            );
            assert!(
                readback::read_committed_daily_input(&f.admitter, &hash)
                    .await?
                    .is_some()
            );
        }
        assert_eq!(readback_snapshot(&f).await?, before);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_admitter_receipt_keeps_old_epoch_results_historical() {
    run("readback_old_epoch", |f| async move {
        f.ready().await?;
        let (id, _) = readback_existing(&f).await?;
        let old = readback::read_daily_admission_receipt(&f.admitter, id)
            .await?
            .ok_or("missing original")?;
        assert!(old.matches_current_cycle());
        sqlx::query("UPDATE owner_beta_daily_control SET enabled=false")
            .execute(&f.db.pool)
            .await?;
        let prior = readback::read_daily_admission_receipt(&f.admitter, id)
            .await?
            .ok_or("missing old epoch")?;
        assert_eq!(prior.activation_epoch(), old.activation_epoch());
        assert_eq!(prior.outcome(), old.outcome());
        assert!(!prior.matches_current_cycle());
        readback_next(&f).await?;
        let mut cycle = readback_cycle(&f).await?;
        let context = cycle.context().await?;
        assert!(context.activation_epoch() > prior.activation_epoch());
        let new = readback::read_daily_admission_receipt(&f.admitter, cycle.receipt_id())
            .await?
            .ok_or("missing new epoch")?;
        assert!(new.matches_current_cycle());
        assert_eq!(new.outcome(), &readback::RecordedAdmissionOutcome::Started);
        cycle
            .finish_failure(AdmissionPollFailure::InputPending)
            .await?;
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_admitter_receipt_rejects_corrupt_verified_binding() {
    run("readback_invalid_binding", |f| async move {
        f.ready().await?;
        let (id,_)=readback_existing(&f).await?;
        // Release-authority fault injection; the admitter has no table UPDATE.
        sqlx::query("UPDATE owner_beta_daily_poll_receipts SET completion_binding_sha256=$2 WHERE receipt_id=$1")
            .bind(id).bind(readback_hash('e').as_str()).execute(&f.db.pool).await?;
        assert!(matches!(readback::read_daily_admission_receipt(&f.admitter,id).await,
            Err(DailyRuntimeError::DatabaseIntegrity)));
        Ok(())
    }).await;
}

#[tokio::test]
async fn daily_admitter_readback_retains_blocked_event_identity() {
    run("readback_blocked_event", |f| async move {
        f.control(false).await?;
        let cycle=readback_cycle(&f).await?;
        let id=cycle.receipt_id();
        let event=serde_json::json!({
            "source_kind":"BAR","range_start":"2026-08-29","range_end":"2026-08-29",
            "code":"SEALED_SOURCE_CHANGED","evidence_sha256":readback_hash('e'),
        });
        // Synthetic SQL transport only, not a source observation or file proof.
        let result:(String,Option<String>,String,String)=sqlx::query_as(
            "SELECT disposition,admission_sha256,reason_code,event_sha256 FROM block_owner_beta_daily_input($1,$2)")
            .bind(id).bind(event).fetch_one(&f.admitter).await?;
        assert_eq!(result.0,"BLOCKED");
        assert!(result.1.is_none());
        assert_eq!(result.2,"SEALED_SOURCE_CHANGED");
        let expected_event=domain::ContentHash::parse(&result.3)?;
        let receipt=readback::read_daily_admission_receipt(&f.admitter,id).await?.ok_or("missing blocked receipt")?;
        assert_eq!(receipt.outcome(),&readback::RecordedAdmissionOutcome::Failed {
            failure:AdmissionPollFailure::InputBlocked,source_frontier:Some(readback_hash('e')),
            event:Some((market_data::daily_price_only::observation::DailyBlockCode::SealedSourceChanged,expected_event)),
        });
        drop(cycle);
        readback_next(&f).await?;
        let mut cycle=readback_cycle(&f).await?;
        assert!(cycle.context().await?.source_blocked());
        cycle.finish_failure(AdmissionPollFailure::InputBlocked).await?;
        Ok(())
    }).await;
}

#[tokio::test]
async fn daily_admitter_exact_input_absence_and_revocation_are_explicit() {
    run("readback_exact_input", |f| async move {
        f.ready().await?;
        let (_, hash) = readback_existing(&f).await?;
        assert!(
            readback::read_committed_daily_input(&f.admitter, &readback_hash('e'))
                .await?
                .is_none()
        );
        assert!(
            readback::read_daily_admission_receipt(&f.admitter, Uuid::new_v4())
                .await?
                .is_none()
        );
        assert!(matches!(
            readback::read_daily_admission_receipt(&f.admitter, Uuid::nil()).await,
            Err(DailyRuntimeError::InvalidInput)
        ));
        let error = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM read_owner_beta_daily_admitted_input('bad')",
        )
        .fetch_one(&f.admitter)
        .await
        .unwrap_err();
        assert_eq!(
            error.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("23514")
        );
        let prior = readback::read_committed_daily_input(&f.admitter, &hash)
            .await?
            .ok_or("missing input")?;
        assert!(prior.unblocked_at_read());
        sqlx::query("SELECT revoke_owner_beta_daily_input($1,$2)")
            .bind(hash.as_str())
            .bind(readback_hash('e').as_str())
            .execute(&f.db.pool)
            .await?;
        let blocked = readback::read_committed_daily_input(&f.admitter, &hash)
            .await?
            .ok_or("missing retained input")?;
        assert!(!blocked.unblocked_at_read());
        assert_eq!(blocked.pins(), prior.pins());
        assert_eq!(
            blocked.source_frontier_sha256(),
            prior.source_frontier_sha256()
        );
        readback_next(&f).await?;
        let mut cycle = readback_cycle(&f).await?;
        assert!(cycle.context().await?.source_blocked());
        cycle
            .finish_failure(AdmissionPollFailure::InputBlocked)
            .await?;
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_admitter_readback_security_settings_are_fixed() {
    run("readback_security", |f| async move {
        for signature in [
            "public.read_owner_beta_daily_admission_context(uuid)",
            "public.read_owner_beta_daily_admitted_input(text)",
            "public.read_owner_beta_daily_admission_receipt(uuid)",
        ] {
            let (definer,owner,settings):(bool,String,bool)=sqlx::query_as(
                "SELECT p.prosecdef,r.rolname::text,p.proconfig @> ARRAY[\
                 'search_path=pg_catalog, pg_temp','lock_timeout=5s','statement_timeout=30s'] \
                 FROM pg_proc p JOIN pg_roles r ON r.oid=p.proowner WHERE p.oid=$1::text::regprocedure")
                .bind(signature).fetch_one(&f.db.pool).await?;
            assert!(definer);
            assert_eq!(owner,"migration_owner");
            assert!(settings);
            for role in ["app","worker","admin","audit_writer","research_writer","daily_input_admitter"] {
                let allowed:bool=sqlx::query_scalar("SELECT has_function_privilege($1,$2,'EXECUTE')")
                    .bind(role).bind(signature).fetch_one(&f.db.pool).await?;
                assert_eq!(allowed,role=="daily_input_admitter");
            }
            let public_grant:bool=sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_proc p,aclexplode(p.proacl) a \
                 WHERE p.oid=$1::text::regprocedure AND a.grantee=0 AND a.privilege_type='EXECUTE')")
                .bind(signature).fetch_one(&f.db.pool).await?;
            assert!(!public_grant);
        }
        Ok(())
    }).await;
}

#[tokio::test]
async fn daily_admitter_readback_down_preserves_existing_daily_evidence() {
    run("readback_additive_down", |f| async move {
        f.ready().await?;
        let (id,hash)=readback_existing(&f).await?;
        let before=readback_snapshot(&f).await?;
        let mut transaction=f.db.pool.begin().await?;
        sqlx::raw_sql(include_str!("../../../migrations/0062_owner_beta_daily_admitter_readback.down.sql"))
            .execute(&mut *transaction).await?;
        let absent:bool=sqlx::query_scalar(
            "SELECT to_regprocedure('public.read_owner_beta_daily_admission_context(uuid)') IS NULL \
             AND to_regprocedure('public.read_owner_beta_daily_admitted_input(text)') IS NULL \
             AND to_regprocedure('public.read_owner_beta_daily_admission_receipt(uuid)') IS NULL \
             AND to_regprocedure('public.admit_owner_beta_daily_input(uuid,jsonb)') IS NOT NULL \
             AND to_regprocedure('public.cancel_superseded_owner_beta_daily_claim(uuid,uuid,uuid,integer,text)') IS NOT NULL")
            .fetch_one(&mut *transaction).await?;
        assert!(absent);
        transaction.commit().await?;
        assert_eq!(readback_snapshot(&f).await?,before);
        let mut transaction=f.db.pool.begin().await?;
        sqlx::raw_sql(include_str!("../../../migrations/0062_owner_beta_daily_admitter_readback.up.sql"))
            .execute(&mut *transaction).await?;
        transaction.commit().await?;
        assert!(readback::read_daily_admission_receipt(&f.admitter,id).await?.is_some());
        assert!(readback::read_committed_daily_input(&f.admitter,&hash).await?.is_some());
        assert_eq!(readback_snapshot(&f).await?,before);
        Ok(())
    }).await;
}
