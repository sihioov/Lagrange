//! PostgreSQL contract coverage for R4-05 WP-3B storage, RLS, publication,
//! recovery, cancellation, and immutable pair revalidation.
//!
//! All simulation output here is synthetic but passes the production result
//! validator with production capital. No approved artifact or private result
//! root is opened. DATABASE_URL must name a disposable PostgreSQL cluster;
//! when it is absent this follows the existing ScratchDb harness skip pattern
//! and is not evidence of a passing database check.

mod common;

use std::{error::Error, time::Duration};

use chrono::NaiveDate;
use common::ScratchDb;
use job_queue::{
    AuditActor, JobQueue, JobStatus, QueueConfig,
    owner_beta::{
        simulation::{
            APPROVED_BARS_SHA256, APPROVED_CONTRACT_SHA256, CostProfile, EngineIdentity,
            EvaluationPriceRow, Price4, SignalSnapshot, SimulationResult, SimulationSchedule,
            StrategyIdentity, simulate_ledgers,
        },
        simulation_execution::{
            CheckedSimulationPair, OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE,
            SimulationCommitState, SimulationJobInput, SimulationPublicationError,
            SimulationPublicationFailure, SimulationPublicationOutcome,
            check_stored_simulation_pair, publish_simulation_success, reconcile_simulation_commit,
            recover_simulation_claims, settle_invalid_simulation_claim, settle_simulation_failure,
        },
    },
};
use market_data::KR_ETF_CORE_SYMBOLS;
use serde_json::json;
use sqlx::{PgPool, postgres::PgConnectOptions, postgres::PgPoolOptions};
use uuid::Uuid;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

struct SimulationFixture {
    owner_id: Uuid,
    input: SimulationJobInput,
    job_id: Uuid,
}

fn queue_config(lease: Duration) -> QueueConfig {
    QueueConfig {
        lease,
        backoff_base: Duration::from_millis(100),
    }
}

async fn new_owner(db: &ScratchDb, suffix: &str) -> TestResult<Uuid> {
    let id = sqlx::query_scalar(
        "INSERT INTO public.users (issuer, subject, email)
         VALUES ('r4-simulation.test', $1, $2) RETURNING id",
    )
    .bind(format!("{suffix}-{}", Uuid::new_v4()))
    .bind(format!("{suffix}-{}@example.test", Uuid::new_v4()))
    .fetch_one(&db.pool)
    .await?;
    sqlx::query("INSERT INTO public.roles (id) VALUES ('owner') ON CONFLICT (id) DO NOTHING")
        .execute(&db.pool)
        .await?;
    sqlx::query("INSERT INTO public.user_roles (user_id, role_id) VALUES ($1, 'owner')")
        .bind(id)
        .execute(&db.pool)
        .await?;
    Ok(id)
}

async fn role_pool(db: &ScratchDb, role: &str, actor: Option<Uuid>) -> PgPool {
    let actor = actor.map(|actor| actor.to_string());
    role_pool_with_actor_value(db, role, actor.as_deref()).await
}

async fn role_pool_with_actor_value(db: &ScratchDb, role: &str, actor: Option<&str>) -> PgPool {
    let options: PgConnectOptions = db
        .role_url(role)
        .parse()
        .unwrap_or_else(|_| panic!("valid {role} test role URL"));
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(options)
        .await
        .unwrap_or_else(|_| panic!("connect as {role}"));
    if let Some(actor) = actor {
        sqlx::query_scalar::<_, String>(
            "SELECT pg_catalog.set_config('app.actor_user_id', $1, false)",
        )
        .bind(actor)
        .fetch_one(&pool)
        .await
        .unwrap_or_else(|_| panic!("set test actor for {role}"));
    }
    pool
}

async fn assert_raw_actor(
    pool: &PgPool,
    expected_backend_pid: i32,
    expected: Option<&str>,
    context: &str,
) -> TestResult {
    let (backend_pid, observed): (i32, Option<String>) = sqlx::query_as(
        "SELECT pg_catalog.pg_backend_pid(),
                pg_catalog.current_setting('app.actor_user_id', true)",
    )
    .fetch_one(pool)
    .await?;
    assert_eq!(
        backend_pid, expected_backend_pid,
        "{context}: same connection"
    );
    assert_eq!(observed.as_deref(), expected, "{context}");
    Ok(())
}

async fn assert_raw_actor_in_transaction(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    expected: &str,
    context: &str,
) -> TestResult {
    let observed: String =
        sqlx::query_scalar("SELECT pg_catalog.current_setting('app.actor_user_id', true)")
            .fetch_one(&mut **transaction)
            .await?;
    assert_eq!(observed, expected, "{context}");
    Ok(())
}

async fn assert_actor_logically_absent(
    pool: &PgPool,
    expected_backend_pid: i32,
    context: &str,
) -> TestResult {
    let (backend_pid, absent): (i32, bool) = sqlx::query_as(
        "SELECT pg_catalog.pg_backend_pid(),
                NULLIF(pg_catalog.current_setting('app.actor_user_id', true), '') IS NULL",
    )
    .fetch_one(pool)
    .await?;
    assert_eq!(
        backend_pid, expected_backend_pid,
        "{context}: same connection"
    );
    assert!(absent, "{context}");
    Ok(())
}

fn assert_sqlstate(error: &sqlx::Error, expected: &str, context: &str) {
    let code = error
        .as_database_error()
        .and_then(|database_error| database_error.code());
    assert_eq!(code.as_deref(), Some(expected), "{context}");
}

async fn pin_inactive_candidate_control_for_test(db: &ScratchDb, worker: &PgPool) -> TestResult {
    let candidate: (bool, Uuid) = sqlx::query_as(
        "SELECT active, service_user_id
           FROM public.candidate_scheduler_control
          WHERE control_key = 'scheduler'",
    )
    .fetch_one(worker)
    .await?;
    assert_eq!(
        candidate,
        (
            false,
            Uuid::parse_str("00000000-0000-4000-8000-000000000042")?,
        ),
        "the default candidate service row remains present and inactive"
    );
    let daily_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM public.owner_beta_daily_control")
            .fetch_one(&db.pool)
            .await?;
    assert_eq!(
        daily_rows, 0,
        "the isolated test has no Daily control to activate"
    );
    Ok(())
}

#[derive(Debug, sqlx::FromRow, PartialEq, Eq)]
struct JobsOwnerPolicySnapshot {
    roles: Vec<String>,
    command: String,
    permissive: String,
    using_expression: Option<String>,
    check_expression: Option<String>,
}

#[derive(Debug, sqlx::FromRow, PartialEq, Eq)]
struct CandidateSettlementSnapshot {
    definition: String,
    body: String,
    owner: String,
    configuration: String,
    acl: String,
    security_definer: bool,
    language: String,
}

async fn jobs_owner_policy_snapshot(pool: &PgPool) -> TestResult<JobsOwnerPolicySnapshot> {
    Ok(sqlx::query_as(
        "SELECT roles::text[] AS roles, cmd AS command, permissive,
                qual AS using_expression, with_check AS check_expression
           FROM pg_catalog.pg_policies
          WHERE schemaname = 'public'
            AND tablename = 'jobs'
            AND policyname = 'tenant_all_owner_jobs'",
    )
    .fetch_one(pool)
    .await?)
}

async fn candidate_settlement_snapshot(pool: &PgPool) -> TestResult<CandidateSettlementSnapshot> {
    Ok(sqlx::query_as(
        "SELECT pg_catalog.pg_get_functiondef(p.oid) AS definition,
                p.prosrc AS body,
                owner.rolname::text AS owner,
                COALESCE(p.proconfig::text, '') AS configuration,
                COALESCE(p.proacl::text, '<default>') AS acl,
                p.prosecdef AS security_definer,
                lang.lanname::text AS language
           FROM pg_catalog.pg_proc AS p
           JOIN pg_catalog.pg_roles AS owner ON owner.oid = p.proowner
           JOIN pg_catalog.pg_language AS lang ON lang.oid = p.prolang
          WHERE p.oid =
                'public.assert_candidate_publication_settlement()'::regprocedure",
    )
    .fetch_one(pool)
    .await?)
}

fn legacy_candidate_settlement_body() -> &'static str {
    let source = include_str!("../../../migrations/0044_candidate_pipeline.up.sql");
    let function_start = source
        .find("CREATE FUNCTION public.assert_candidate_publication_settlement()")
        .expect("0044 candidate settlement function exists");
    let function_source = &source[function_start..];
    let body_start = function_source
        .find("AS $assert$")
        .expect("0044 candidate settlement body starts")
        + "AS $assert$".len();
    let body_end = function_source[body_start..]
        .find("$assert$;")
        .expect("0044 candidate settlement body ends")
        + body_start;
    &function_source[body_start..body_end]
}

async fn assert_generic_attempt_first_actor_scope(
    db: &ScratchDb,
    actor_setting: Option<&str>,
    expected_actor: Option<&str>,
    label: &str,
) -> TestResult {
    let owner_b = new_owner(db, &format!("r4-{label}-owner-b")).await?;
    let worker = role_pool_with_actor_value(db, "worker", actor_setting).await;
    let worker_backend_pid: i32 = sqlx::query_scalar("SELECT pg_catalog.pg_backend_pid()")
        .fetch_one(&worker)
        .await?;
    let queue = JobQueue::new(worker.clone(), None, queue_config(Duration::from_secs(1)));
    let fixture_queue = JobQueue::new(db.pool.clone(), None, queue_config(Duration::from_secs(1)));
    let recovering = fixture_queue
        .submit(job_queue::SubmitJob {
            owner_user_id: owner_b,
            job_type: "backtest".to_owned(),
            payload: json!({"kind": "actor-recovery", "case": label}),
            priority: 1,
            idempotency_key: None,
            max_attempts: 2,
            available_at: None,
        })
        .await?;
    let claim = queue
        .claim_next(&format!("r4-{label}-recovery-worker"))
        .await?
        .expect("generic recovery fixture is claimable");
    assert_eq!(claim.job.id, recovering.id);
    match expected_actor {
        Some(actor) => {
            assert_raw_actor(
                &worker,
                worker_backend_pid,
                Some(actor),
                "generic claim keeps actor A",
            )
            .await?
        }
        None => {
            assert_actor_logically_absent(
                &worker,
                worker_backend_pid,
                "generic claim keeps actor logically absent",
            )
            .await?
        }
    }
    sqlx::query(
        "UPDATE public.jobs
            SET locked_at = pg_catalog.now() - interval '10 seconds'
          WHERE id = $1",
    )
    .bind(recovering.id)
    .execute(&db.pool)
    .await?;
    let recovered = queue.sweep().await?;
    assert_eq!(
        (
            recovered.jobs_checked,
            recovered.attempts_orphaned,
            recovered.jobs_requeued
        ),
        (1, 1, 1),
        "{label} generic recovery performs its attempt-first transition"
    );
    match expected_actor {
        Some(actor) => {
            assert_raw_actor(
                &worker,
                worker_backend_pid,
                Some(actor),
                "generic recovery restores actor A",
            )
            .await?
        }
        None => {
            assert_actor_logically_absent(
                &worker,
                worker_backend_pid,
                "generic recovery keeps actor logically absent",
            )
            .await?
        }
    }
    let recovered_state: (JobStatus, String, i64) = sqlx::query_as(
        "SELECT job.status, attempt.outcome,
                (SELECT count(*) FROM public.job_attempts AS all_attempts
                  WHERE all_attempts.job_id = job.id)
           FROM public.jobs AS job
           JOIN public.job_attempts AS attempt
             ON attempt.job_id = job.id AND attempt.attempt_no = 1
          WHERE job.id = $1",
    )
    .bind(recovering.id)
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(
        recovered_state,
        (JobStatus::Queued, "ORPHANED".to_owned(), 1)
    );
    sqlx::query(
        "UPDATE public.jobs
            SET available_at = pg_catalog.now() + interval '1 hour'
          WHERE id = $1",
    )
    .bind(recovering.id)
    .execute(&db.pool)
    .await?;

    let canceled = fixture_queue
        .submit(job_queue::SubmitJob {
            owner_user_id: owner_b,
            job_type: "backtest".to_owned(),
            payload: json!({"kind": "actor-cancel-settlement", "case": label}),
            priority: 1,
            idempotency_key: None,
            max_attempts: 2,
            available_at: None,
        })
        .await?;
    let canceled_claim = queue
        .claim_next(&format!("r4-{label}-cancel-worker"))
        .await?
        .expect("generic cancellation fixture is claimable");
    assert_eq!(canceled_claim.job.id, canceled.id);
    let app = role_pool(db, "app", Some(owner_b)).await;
    let audit = role_pool(db, "audit_writer", None).await;
    let cancel_queue = JobQueue::new(app, Some(audit), queue_config(Duration::from_secs(1)));
    let mut actor = AuditActor::new("owner");
    actor.user_id = Some(owner_b);
    actor.correlation_id = Some(format!("r4-{label}-cancel-{}", Uuid::new_v4()));
    assert!(matches!(
        cancel_queue.request_cancel(canceled.id, &actor).await?,
        job_queue::CancelResult::Canceled(_)
    ));
    sqlx::query(
        "UPDATE public.jobs
            SET locked_at = pg_catalog.now() - interval '10 seconds'
          WHERE id = $1",
    )
    .bind(canceled.id)
    .execute(&db.pool)
    .await?;
    let canceled_recovery = queue.sweep().await?;
    assert_eq!(
        (
            canceled_recovery.jobs_checked,
            canceled_recovery.attempts_orphaned
        ),
        (1, 1),
        "{label} canceled settlement first terminalizes its attempt"
    );
    match expected_actor {
        Some(actor) => {
            assert_raw_actor(
                &worker,
                worker_backend_pid,
                Some(actor),
                "canceled settlement restores actor A",
            )
            .await?
        }
        None => {
            assert_actor_logically_absent(
                &worker,
                worker_backend_pid,
                "canceled settlement keeps actor logically absent",
            )
            .await?
        }
    }
    let canceled_state: (JobStatus, String) = sqlx::query_as(
        "SELECT job.status, attempt.outcome
           FROM public.jobs AS job
           JOIN public.job_attempts AS attempt
             ON attempt.job_id = job.id AND attempt.attempt_no = 1
          WHERE job.id = $1",
    )
    .bind(canceled.id)
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(canceled_state, (JobStatus::Canceled, "ORPHANED".to_owned()));
    sqlx::query("UPDATE public.jobs SET locked_at = NULL WHERE id = $1")
        .bind(canceled.id)
        .execute(&db.pool)
        .await?;
    Ok(())
}

async fn set_actor(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, owner: Uuid) -> TestResult {
    sqlx::query_scalar::<_, String>("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
        .bind(owner.to_string())
        .fetch_one(&mut **tx)
        .await?;
    Ok(())
}

fn input_json(run_id: Uuid, config_id: Uuid) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schema": "owner-beta-price-only-simulation-job-v1",
        "run_id": run_id,
        "strategy_config_id": config_id,
        "evaluation_start": "2026-08-27",
        "evaluation_end": "2026-08-28",
        "pins": {
            "candidate_content_sha256": "sha256:b0fb82f6a580f3def13a1e1e34bea68e30d95dc720306e7ee54b6c3199cf402d",
            "artifact_manifest_sha256": "sha256:53587f32ce67ae1a8488b9c00096465185d03e35b1b38a04f39ed5462da7f6b6",
            "stage5_manifest_sha256": "sha256:5b2900594e03e322d4e81adaa79fc6c40ae70035213786972ed5cb7a22870d2a",
            "action_manifest_sha256": "sha256:080d38142a6506f741114eb75a77a36c23c2554a0d39eba8843ff62bfb484550",
            "approval_registry_sha256": "sha256:5d3aa2b354d8c0c51d0d7d029e9fd3f92e0570fe074262bfc900829a5a2bb707"
        },
        "contract_document_sha256": APPROVED_CONTRACT_SHA256,
        "bars_sha256": APPROVED_BARS_SHA256,
        "strategy": {
            "strategy_id": "trend_following",
            "strategy_version": "1.0.0",
            "config_json": {
                "benchmark_instrument": "069500.KRX",
                "fast_ma": 50,
                "slow_ma": 200
            },
            "config_sha256": "sha256:90e5b4c6972678e9186d4655d76496676e7cfb8dbe473a249f062963873c04a0"
        }
    }))
    .expect("synthetic simulation payload JSON")
}

async fn seed_fixture(
    db: &ScratchDb,
    owner_id: Uuid,
    app: &PgPool,
    suffix: &str,
) -> TestResult<SimulationFixture> {
    seed_fixture_with_identity_window(db, owner_id, app, suffix, None, None).await
}

async fn seed_invalid_fixture(
    db: &ScratchDb,
    owner_id: Uuid,
    app: &PgPool,
    suffix: &str,
    evaluation_start: &str,
    evaluation_end: &str,
) -> TestResult<SimulationFixture> {
    seed_fixture_with_identity_window(
        db,
        owner_id,
        app,
        suffix,
        Some((evaluation_start, evaluation_end)),
        None,
    )
    .await
}

async fn seed_nil_identity_fixture(
    db: &ScratchDb,
    owner_id: Uuid,
    app: &PgPool,
    suffix: &str,
) -> TestResult<SimulationFixture> {
    seed_fixture_with_identity_window(
        db,
        owner_id,
        app,
        suffix,
        None,
        Some((Uuid::from_u128(0), Uuid::from_u128(0))),
    )
    .await
}

async fn seed_fixture_with_identity_window(
    db: &ScratchDb,
    owner_id: Uuid,
    app: &PgPool,
    suffix: &str,
    evaluation_window: Option<(&str, &str)>,
    explicit_ids: Option<(Uuid, Uuid)>,
) -> TestResult<SimulationFixture> {
    sqlx::query(
        "INSERT INTO public.strategies (id, display_name, state)
         VALUES ('trend_following', 'Trend following', 'Validated')
         ON CONFLICT (id) DO NOTHING",
    )
    .execute(&db.pool)
    .await?;

    let (run_id, config_id) = explicit_ids.unwrap_or_else(|| (Uuid::new_v4(), Uuid::new_v4()));
    let input = SimulationJobInput::from_json(&input_json(run_id, config_id))
        .map_err(|_| "synthetic fixed-window input must validate")?;

    // Explicit config IDs (including the nil-ID regression) are privileged
    // fixture preparation. The app's production column grant excludes id.
    sqlx::query(
        "INSERT INTO public.user_strategy_configs
            (id, owner_user_id, strategy_id, strategy_version, config_json, is_active)
         VALUES ($1, $2, 'trend_following', '1.0.0', $3, true)",
    )
    .bind(config_id)
    .bind(owner_id)
    .bind(input.strategy().config_json())
    .execute(&db.pool)
    .await?;

    let mut tx = app.begin().await?;
    set_actor(&mut tx, owner_id).await?;
    let app_identity: (String, String, Uuid) = sqlx::query_as(
        "SELECT current_user::text, session_user::text,
                current_setting('app.actor_user_id')::uuid",
    )
    .fetch_one(&mut *tx)
    .await?;
    assert_eq!(app_identity, ("app".to_owned(), "app".to_owned(), owner_id));
    let config_matches: bool = sqlx::query_scalar(
        "SELECT owner_user_id = $2 AND strategy_id = 'trend_following'
                AND strategy_version = '1.0.0' AND config_json = $3 AND is_active
           FROM public.user_strategy_configs WHERE id = $1",
    )
    .bind(config_id)
    .bind(owner_id)
    .bind(input.strategy().config_json())
    .fetch_one(&mut *tx)
    .await?;
    assert!(
        config_matches,
        "app sees the exact owner-bound fixture config"
    );

    let mut job_payload = serde_json::to_value(&input)?;
    let (evaluation_start, evaluation_end, session_count) = match evaluation_window {
        Some((start, end)) => {
            job_payload["evaluation_start"] = json!(start);
            job_payload["evaluation_end"] = json!(end);
            (
                NaiveDate::parse_from_str(start, "%Y-%m-%d")?,
                NaiveDate::parse_from_str(end, "%Y-%m-%d")?,
                202,
            )
        }
        None => (
            input.evaluation_start().as_naive_date(),
            input.evaluation_end().as_naive_date(),
            (input.schedule().warmup_sessions().len()
                + input.schedule().evaluation_sessions().len()) as i32,
        ),
    };
    let job_id: Uuid = sqlx::query_scalar(
        "INSERT INTO public.jobs
            (owner_user_id, job_type, status, priority, idempotency_key,
             payload_json, max_attempts)
         VALUES ($1, $2, 'QUEUED', 10, $3, $4, 3)
         RETURNING id",
    )
    .bind(owner_id)
    .bind(OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE)
    .bind(format!(
        "owner-beta:price-only-simulation:v1:{suffix}-{}",
        Uuid::new_v4()
    ))
    .bind(job_payload)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO public.owner_beta_simulation_runs
            (id, owner_user_id, strategy_config_id, job_id,
             strategy_id, strategy_version, strategy_config_json, strategy_config_sha256,
             evaluation_start, evaluation_end, session_count,
             candidate_content_sha256, artifact_manifest_sha256, stage5_manifest_sha256,
             action_manifest_sha256, approval_registry_sha256,
             contract_document_sha256, bars_sha256)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11,
                 $12, $13, $14, $15, $16, $17, $18)",
    )
    .bind(run_id)
    .bind(owner_id)
    .bind(config_id)
    .bind(job_id)
    .bind(input.strategy().strategy_id())
    .bind(input.strategy().strategy_version())
    .bind(input.strategy().config_json())
    .bind(input.strategy().config_sha256().as_str())
    .bind(evaluation_start)
    .bind(evaluation_end)
    .bind(session_count)
    .bind(input.pins().candidate_content_sha256().as_str())
    .bind(input.pins().artifact_manifest_sha256().as_str())
    .bind(input.pins().stage5_manifest_sha256().as_str())
    .bind(input.pins().action_manifest_sha256().as_str())
    .bind(input.pins().approval_registry_sha256().as_str())
    .bind(input.contract_document_sha256().as_str())
    .bind(input.bars_sha256().as_str())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    sqlx::query(
        "INSERT INTO public.data_entitlements
            (contract_document_sha256, contract_reference, status,
             covered_datasets, covered_uses, effective_from, effective_until, managed_by)
         VALUES (repeat('e', 64), $1, 'ACTIVE', '[\"krx_eod_bars\"]'::jsonb,
                 '[\"backtest\"]'::jsonb, DATE '2016-08-29', DATE '2026-08-28', $2)",
    )
    .bind(format!("fixture://r4-simulation-{suffix}"))
    .bind(owner_id)
    .execute(&db.pool)
    .await?;
    Ok(SimulationFixture {
        owner_id,
        input,
        job_id,
    })
}

async fn mutate_config_after_enqueue(db: &ScratchDb, fixture: &SimulationFixture) -> TestResult {
    let updated = sqlx::query(
        "UPDATE public.user_strategy_configs
            SET config_json = pg_catalog.jsonb_set(config_json, '{fast_ma}', '40'::jsonb),
                is_active = false
          WHERE id = $1 AND owner_user_id = $2",
    )
    .bind(fixture.input.strategy_config_id())
    .bind(fixture.owner_id)
    .execute(&db.pool)
    .await?;
    assert_eq!(updated.rows_affected(), 1);
    Ok(())
}

fn synthetic_engine(marker: char) -> EngineIdentity {
    EngineIdentity::new(
        &marker.to_string().repeat(40),
        &format!("sha256:{}", marker.to_string().repeat(64)),
        "rustc-test",
        "x86_64-unknown-linux-gnu",
        "test",
    )
    .expect("synthetic content-bound engine identity")
}

fn signal_targets() -> Vec<(String, u32)> {
    let mut targets = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| {
            let instrument = format!("{symbol}.KRX");
            let weight = u32::from(instrument == "069500.KRX") * 1_000_000;
            (instrument, weight)
        })
        .collect::<Vec<_>>();
    targets.sort_by(|left, right| left.0.cmp(&right.0));
    targets
}

fn synthetic_result_bytes(
    schedule: &SimulationSchedule,
    engine: &EngineIdentity,
    strategy_hash: &str,
    profile: CostProfile,
) -> Vec<u8> {
    let factor_hash = format!("sha256:{}", "a".repeat(64));
    let target_hash = format!("sha256:{}", "b".repeat(64));
    let signals = schedule
        .signal_execution_pairs()
        .into_iter()
        .map(|(signal_date, _)| {
            SignalSnapshot::new(
                &signal_date.to_iso(),
                &factor_hash,
                &target_hash,
                signal_targets(),
                0,
            )
            .expect("synthetic signal snapshot")
        })
        .collect::<Vec<_>>();
    let prices = schedule
        .evaluation_sessions()
        .iter()
        .map(|session| {
            EvaluationPriceRow::new(
                &session.to_iso(),
                Price4::parse("100.0000").expect("open price"),
                Price4::parse("101.0000").expect("close price"),
                1_000_000,
            )
            .expect("synthetic price")
        })
        .collect::<Vec<_>>();
    let ledgers = simulate_ledgers(schedule, &signals, &prices)
        .expect("production-capital synthetic ledgers");
    let strategy = StrategyIdentity::trend_following_v1(
        strategy_hash,
        "1.0.0",
        &format!("sha256:{}", "d".repeat(64)),
    )
    .expect("fixed trend strategy identity");
    let profile_ledgers = match profile {
        CostProfile::Illustrative10BpPerSideV1 => ledgers.illustrative_10bp_per_side(),
        CostProfile::ZeroCostReferenceV1 => ledgers.zero_cost_reference(),
    };
    SimulationResult::new(
        engine.clone(),
        strategy,
        schedule.clone(),
        profile,
        signals,
        profile_ledgers.strategy().clone(),
        profile_ledgers.benchmark().clone(),
    )
    .expect("checked synthetic result")
    .canonical_json()
    .expect("canonical result bytes")
}

fn synthetic_pair(fixture: &SimulationFixture, engine: &EngineIdentity) -> CheckedSimulationPair {
    let illustrative = synthetic_result_bytes(
        fixture.input.schedule(),
        engine,
        fixture.input.strategy().config_sha256().as_str(),
        CostProfile::Illustrative10BpPerSideV1,
    );
    let zero_cost = synthetic_result_bytes(
        fixture.input.schedule(),
        engine,
        fixture.input.strategy().config_sha256().as_str(),
        CostProfile::ZeroCostReferenceV1,
    );
    CheckedSimulationPair::from_canonical(&illustrative, &zero_cost, &fixture.input, engine)
        .expect("whole checked simulation pair")
}

async fn publish_fixture(
    fixture: &SimulationFixture,
    _worker: &PgPool,
    queue: &JobQueue,
    engine: &EngineIdentity,
) -> TestResult<CheckedSimulationPair> {
    let claim = queue
        .claim_next_for("r4-test-worker", OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE)
        .await?
        .expect("dedicated simulation claim");
    assert_eq!(claim.job.id, fixture.job_id);
    let pair = synthetic_pair(fixture, engine);
    let outcome = publish_simulation_success(queue, &claim, &fixture.input, &pair, engine).await?;
    assert_eq!(outcome, SimulationPublicationOutcome::Published);
    Ok(pair)
}

async fn require_scratch_db() -> Option<ScratchDb> {
    let db = ScratchDb::create().await;
    if db.is_none() {
        eprintln!("SKIP: DATABASE_URL not set; this is not a database test pass");
    }
    db
}

#[tokio::test]
async fn r4_actor_rls_and_hash_isolation() {
    let Some(db) = require_scratch_db().await else {
        return;
    };
    r4_actor_rls_and_hash_isolation_body(&db).await.unwrap();
    db.drop_db().await;
}

async fn r4_actor_rls_and_hash_isolation_body(db: &ScratchDb) -> TestResult {
    let owner_a = new_owner(db, "r4-isolation-a").await?;
    let owner_b = new_owner(db, "r4-isolation-b").await?;
    let app = role_pool(db, "app", None).await;
    let fixture_a = seed_fixture(db, owner_a, &app, "same-content-a").await?;
    let fixture_b = seed_fixture(db, owner_b, &app, "same-content-b").await?;

    let mut forged_owner_tx = app.begin().await?;
    set_actor(&mut forged_owner_tx, owner_a).await?;
    let forged_insert = sqlx::query(
        "INSERT INTO public.jobs
            (owner_user_id, job_type, status, priority, idempotency_key,
             payload_json, max_attempts)
         VALUES ($1, $2, 'QUEUED', 1, $3, $4, 3)",
    )
    .bind(owner_b)
    .bind(OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE)
    .bind(format!(
        "owner-beta:price-only-simulation:v1:forged-{}",
        Uuid::new_v4()
    ))
    .bind(serde_json::to_value(&fixture_b.input)?)
    .execute(&mut *forged_owner_tx)
    .await;
    assert!(
        forged_insert.is_err(),
        "actor A cannot create a job for owner B"
    );
    forged_owner_tx.rollback().await?;

    let worker = role_pool(db, "worker", None).await;
    let queue = JobQueue::new(worker.clone(), None, queue_config(Duration::from_secs(30)));
    let engine = synthetic_engine('a');
    let pair_a = publish_fixture(&fixture_a, &worker, &queue, &engine).await?;
    let pair_b = publish_fixture(&fixture_b, &worker, &queue, &engine).await?;
    assert_eq!(
        pair_a.illustrative_10bp_per_side_sha256(),
        pair_b.illustrative_10bp_per_side_sha256(),
        "equal canonical bytes remain separately owner-linked"
    );
    assert_eq!(
        pair_a.zero_cost_reference_sha256(),
        pair_b.zero_cost_reference_sha256()
    );
    let owner_pair_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.owner_beta_simulation_result_pairs
          WHERE illustrative_sha256 = $1 AND zero_cost_sha256 = $2",
    )
    .bind(pair_a.illustrative_10bp_per_side_sha256().as_str())
    .bind(pair_a.zero_cost_reference_sha256().as_str())
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(owner_pair_count, 2, "no global hash uniqueness or reuse");

    let tx_a = async {
        let mut tx = app.begin().await?;
        set_actor(&mut tx, owner_a).await?;
        let own: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM public.owner_beta_simulation_result_pairs WHERE run_id = $1",
        )
        .bind(fixture_a.input.run_id())
        .fetch_one(&mut *tx)
        .await?;
        let foreign: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM public.owner_beta_simulation_result_pairs WHERE run_id = $1",
        )
        .bind(fixture_b.input.run_id())
        .fetch_one(&mut *tx)
        .await?;
        assert_eq!((own, foreign), (1, 0));
        tx.commit().await?;
        Ok::<_, Box<dyn Error>>(())
    };
    tx_a.await?;
    let actor_cleared: bool = sqlx::query_scalar(
        "SELECT NULLIF(pg_catalog.current_setting('app.actor_user_id', true), '') IS NULL",
    )
    .fetch_one(&app)
    .await?;
    assert!(
        actor_cleared,
        "transaction-local actor must reset in the pooled app connection"
    );

    let mut tx_b = app.begin().await?;
    set_actor(&mut tx_b, owner_b).await?;
    let own_b: i64 =
        sqlx::query_scalar("SELECT count(*) FROM public.owner_beta_simulation_runs WHERE id = $1")
            .bind(fixture_b.input.run_id())
            .fetch_one(&mut *tx_b)
            .await?;
    let foreign_b: i64 =
        sqlx::query_scalar("SELECT count(*) FROM public.owner_beta_simulation_runs WHERE id = $1")
            .bind(fixture_a.input.run_id())
            .fetch_one(&mut *tx_b)
            .await?;
    assert_eq!((own_b, foreign_b), (1, 0));
    let own_b_pair: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.owner_beta_simulation_result_pairs WHERE run_id = $1",
    )
    .bind(fixture_b.input.run_id())
    .fetch_one(&mut *tx_b)
    .await?;
    let foreign_b_pair: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.owner_beta_simulation_result_pairs WHERE run_id = $1",
    )
    .bind(fixture_a.input.run_id())
    .fetch_one(&mut *tx_b)
    .await?;
    assert_eq!((own_b_pair, foreign_b_pair), (1, 0));
    tx_b.commit().await?;

    let foreign_queued = seed_fixture(db, owner_b, &app, "foreign-cancel").await?;
    let actor_a_pool = role_pool(db, "app", Some(owner_a)).await;
    let audit = role_pool(db, "audit_writer", None).await;
    let actor_a_queue = JobQueue::new(
        actor_a_pool,
        Some(audit),
        queue_config(Duration::from_secs(30)),
    );
    let mut actor_a = AuditActor::new("owner");
    actor_a.user_id = Some(owner_a);
    actor_a.correlation_id = Some(format!("r4-cross-owner-cancel-{}", Uuid::new_v4()));
    assert!(matches!(
        actor_a_queue
            .request_cancel(foreign_queued.job_id, &actor_a)
            .await,
        Err(job_queue::QueueError::JobNotFound(id)) if id == foreign_queued.job_id
    ));
    let foreign_status: JobStatus =
        sqlx::query_scalar("SELECT status FROM public.jobs WHERE id = $1")
            .bind(foreign_queued.job_id)
            .fetch_one(&db.pool)
            .await?;
    assert_eq!(foreign_status, JobStatus::Queued);

    let unset_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM public.owner_beta_simulation_result_pairs")
            .fetch_one(&app)
            .await?;
    assert_eq!(unset_count, 0, "unset actor sees no pair rows");
    Ok(())
}

#[tokio::test]
async fn r4_role_grants_and_forced_rls() {
    let Some(db) = require_scratch_db().await else {
        return;
    };
    r4_role_grants_and_forced_rls_body(&db).await.unwrap();
    db.drop_db().await;
}

async fn r4_role_grants_and_forced_rls_body(db: &ScratchDb) -> TestResult {
    let owner = new_owner(db, "r4-grants").await?;
    let app = role_pool(db, "app", None).await;
    let fixture = seed_fixture(db, owner, &app, "grants").await?;
    let worker = role_pool(db, "worker", None).await;
    let admin = role_pool(db, "admin", None).await;
    let research = role_pool(db, "research_writer", None).await;
    let audit = role_pool(db, "audit_writer", None).await;

    let role_identity: (String, String, bool, bool) = sqlx::query_as(
        "SELECT current_user::text, session_user::text,
                r.rolsuper, r.rolbypassrls
           FROM pg_catalog.pg_roles AS r
          WHERE r.rolname = current_user",
    )
    .fetch_one(&app)
    .await?;
    assert_eq!(
        (role_identity.0.as_str(), role_identity.1.as_str()),
        ("app", "app")
    );
    assert!(
        !role_identity.2 && !role_identity.3,
        "app is not privileged around RLS"
    );

    let table_flags: Vec<(String, bool, bool, String)> = sqlx::query_as(
        "SELECT c.relname::text, c.relrowsecurity, c.relforcerowsecurity,
                pg_catalog.pg_get_userbyid(c.relowner)
           FROM pg_catalog.pg_class AS c
          WHERE c.relname IN ('owner_beta_simulation_runs',
                              'owner_beta_simulation_result_pairs')
          ORDER BY c.relname",
    )
    .fetch_all(&db.pool)
    .await?;
    assert_eq!(table_flags.len(), 2);
    assert!(
        table_flags
            .iter()
            .all(|row| row.1 && row.2 && row.3 == "migration_owner")
    );

    let worker_config_update: bool = sqlx::query_scalar(
        "SELECT pg_catalog.has_any_column_privilege(
            'worker', 'public.user_strategy_configs', 'UPDATE')",
    )
    .fetch_one(&db.pool)
    .await?;
    assert!(!worker_config_update);
    let fixture_only_privileges: (bool, bool) = sqlx::query_as(
        "SELECT pg_catalog.has_column_privilege(
            'app', 'public.user_strategy_configs', 'id', 'INSERT'),
                pg_catalog.has_table_privilege('worker', 'public.jobs', 'INSERT')",
    )
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(fixture_only_privileges, (false, false));

    let mut allowed_config = app.begin().await?;
    set_actor(&mut allowed_config, owner).await?;
    let created: (Uuid, Uuid) = sqlx::query_as(
        "INSERT INTO public.user_strategy_configs
            (owner_user_id, strategy_id, strategy_version, config_json, is_active)
         VALUES ($1, 'trend_following', '1.0.0', $2, true)
         RETURNING id, owner_user_id",
    )
    .bind(owner)
    .bind(fixture.input.strategy().config_json())
    .fetch_one(&mut *allowed_config)
    .await?;
    assert_ne!(created.0, fixture.input.strategy_config_id());
    assert_eq!(created.1, owner);
    allowed_config.rollback().await?;

    let mut forbidden_config = app.begin().await?;
    set_actor(&mut forbidden_config, owner).await?;
    let denied = sqlx::query(
        "INSERT INTO public.user_strategy_configs
            (id, owner_user_id, strategy_id, strategy_version, config_json, is_active)
         VALUES ($1, $2, 'trend_following', '1.0.0', $3, true)",
    )
    .bind(Uuid::new_v4())
    .bind(owner)
    .bind(fixture.input.strategy().config_json())
    .execute(&mut *forbidden_config)
    .await
    .expect_err("app cannot choose a config ID");
    assert_eq!(
        denied
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("42501")
    );
    forbidden_config.rollback().await?;
    let app_pair_insert: bool = sqlx::query_scalar(
        "SELECT pg_catalog.has_any_column_privilege(
            'app', 'public.owner_beta_simulation_result_pairs', 'INSERT')",
    )
    .fetch_one(&db.pool)
    .await?;
    let app_pair_update: bool = sqlx::query_scalar(
        "SELECT pg_catalog.has_table_privilege(
            'app', 'public.owner_beta_simulation_result_pairs', 'UPDATE')",
    )
    .fetch_one(&db.pool)
    .await?;
    let app_pair_truncate: bool = sqlx::query_scalar(
        "SELECT pg_catalog.has_table_privilege(
            'app', 'public.owner_beta_simulation_result_pairs', 'TRUNCATE')",
    )
    .fetch_one(&db.pool)
    .await?;
    let app_run_terminal_insert: bool = sqlx::query_scalar(
        "SELECT pg_catalog.has_column_privilege(
            'app', 'public.owner_beta_simulation_runs', 'engine_json', 'INSERT')
             OR pg_catalog.has_column_privilege(
            'app', 'public.owner_beta_simulation_runs', 'published_attempt_id', 'INSERT')
             OR pg_catalog.has_column_privilege(
            'app', 'public.owner_beta_simulation_runs', 'status', 'INSERT')",
    )
    .fetch_one(&db.pool)
    .await?;
    let admin_pair_select: bool = sqlx::query_scalar(
        "SELECT pg_catalog.has_table_privilege(
            'admin', 'public.owner_beta_simulation_result_pairs', 'SELECT')",
    )
    .fetch_one(&db.pool)
    .await?;
    let research_pair_select: bool = sqlx::query_scalar(
        "SELECT pg_catalog.has_table_privilege(
            'research_writer', 'public.owner_beta_simulation_result_pairs', 'SELECT')",
    )
    .fetch_one(&db.pool)
    .await?;
    let audit_pair_select: bool = sqlx::query_scalar(
        "SELECT pg_catalog.has_table_privilege(
            'audit_writer', 'public.owner_beta_simulation_result_pairs', 'SELECT')",
    )
    .fetch_one(&db.pool)
    .await?;
    assert!(
        !app_pair_insert && !app_pair_update && !app_pair_truncate,
        "app cannot insert, update, or truncate canonical pairs"
    );
    assert!(
        !app_run_terminal_insert,
        "app cannot insert terminal metadata"
    );
    assert!(!admin_pair_select && !research_pair_select && !audit_pair_select);

    assert!(
        sqlx::query("DELETE FROM public.owner_beta_simulation_result_pairs WHERE run_id = $1")
            .bind(fixture.input.run_id())
            .execute(&app)
            .await
            .is_err(),
        "app cannot directly mutate pair rows"
    );
    assert!(
        sqlx::query(
            "INSERT INTO public.owner_beta_simulation_result_pairs
                (run_id, owner_user_id, illustrative_canonical, illustrative_sha256,
                 zero_cost_canonical, zero_cost_sha256)
             VALUES ($1, $2, '\\x7b', $3, '\\x7b', $4)",
        )
        .bind(fixture.input.run_id())
        .bind(owner)
        .bind(format!("sha256:{}", "0".repeat(64)))
        .bind(format!("sha256:{}", "1".repeat(64)))
        .execute(&app)
        .await
        .is_err(),
        "app cannot directly insert canonical pairs"
    );
    assert!(
        sqlx::query(
            "UPDATE public.owner_beta_simulation_result_pairs
                SET illustrative_sha256 = $2 WHERE run_id = $1",
        )
        .bind(fixture.input.run_id())
        .bind(format!("sha256:{}", "0".repeat(64)))
        .execute(&app)
        .await
        .is_err(),
        "app cannot directly update canonical pairs"
    );
    assert!(
        sqlx::query("TRUNCATE public.owner_beta_simulation_result_pairs")
            .execute(&app)
            .await
            .is_err(),
        "app cannot truncate canonical pairs"
    );
    assert!(
        sqlx::query(
            "UPDATE public.owner_beta_simulation_runs SET strategy_config_id = $2 WHERE id = $1",
        )
        .bind(fixture.input.run_id())
        .bind(Uuid::new_v4())
        .execute(&worker)
        .await
        .is_err(),
        "worker has no immutable configuration update privilege"
    );
    for column in [
        "owner_user_id",
        "strategy_config_id",
        "evaluation_start",
        "evaluation_end",
        "candidate_content_sha256",
        "action_manifest_sha256",
    ] {
        let permitted: bool = sqlx::query_scalar(
            "SELECT pg_catalog.has_column_privilege(
                'worker', 'public.owner_beta_simulation_runs', $1, 'UPDATE')",
        )
        .bind(column)
        .fetch_one(&db.pool)
        .await?;
        assert!(
            !permitted,
            "worker cannot update immutable run column {column}"
        );
    }
    let mut actor_job_update = app.begin().await?;
    set_actor(&mut actor_job_update, owner).await?;
    let retagged = sqlx::query(
        "UPDATE public.jobs SET payload_json = payload_json || '{\"extra\":true}'::jsonb
          WHERE id = $1",
    )
    .bind(fixture.job_id)
    .execute(&mut *actor_job_update)
    .await;
    assert!(
        retagged.is_err(),
        "app cannot mutate the frozen simulation payload"
    );
    actor_job_update.rollback().await?;
    assert!(
        sqlx::query("UPDATE public.jobs SET job_type = 'backtest' WHERE id = $1")
            .bind(fixture.job_id)
            .execute(&worker)
            .await
            .is_err(),
        "worker cannot retag a simulation job"
    );
    let foreign_owner = new_owner(db, "r4-grants-retag-owner").await?;
    let owner_retag = sqlx::query("UPDATE public.jobs SET owner_user_id = $2 WHERE id = $1")
        .bind(fixture.job_id)
        .bind(foreign_owner)
        .execute(&worker)
        .await
        .expect_err("worker cannot retag a simulation job owner");
    assert_sqlstate(
        &owner_retag,
        "42501",
        "worker owner retag remains forbidden",
    );
    for role_pool in [&admin, &research, &audit] {
        assert!(
            sqlx::query(
                "SELECT illustrative_canonical FROM public.owner_beta_simulation_result_pairs"
            )
            .fetch_all(role_pool)
            .await
            .is_err(),
            "admin and non-serving writers cannot read canonical bytes"
        );
    }
    let migration_owner = role_pool(db, "migration_owner", None).await;
    let hidden_without_actor: i64 =
        sqlx::query_scalar("SELECT count(*) FROM public.owner_beta_simulation_runs WHERE id = $1")
            .bind(fixture.input.run_id())
            .fetch_one(&migration_owner)
            .await?;
    assert_eq!(hidden_without_actor, 0);
    Ok(())
}

#[tokio::test]
async fn r4_success_pair_is_one_commit() {
    let Some(db) = require_scratch_db().await else {
        return;
    };
    r4_success_pair_is_one_commit_body(&db).await.unwrap();
    db.drop_db().await;
}

async fn r4_success_pair_is_one_commit_body(db: &ScratchDb) -> TestResult {
    let owner_a = new_owner(db, "r4-actor-a").await?;
    let owner_b = new_owner(db, "r4-atomic-owner-b").await?;
    let app = role_pool(db, "app", None).await;
    let fixture = seed_fixture(db, owner_b, &app, "atomic").await?;
    let actor_a = owner_a.to_string();
    let worker = role_pool_with_actor_value(db, "worker", Some(&actor_a)).await;
    let worker_backend_pid: i32 = sqlx::query_scalar("SELECT pg_catalog.pg_backend_pid()")
        .fetch_one(&worker)
        .await?;
    pin_inactive_candidate_control_for_test(db, &worker).await?;
    let queue = JobQueue::new(worker.clone(), None, queue_config(Duration::from_secs(60)));
    let generic = JobQueue::new(db.pool.clone(), None, queue_config(Duration::from_secs(60)))
        .submit(job_queue::SubmitJob {
            owner_user_id: owner_b,
            job_type: "backtest".to_owned(),
            payload: json!({"kind": "actor-preservation-generic"}),
            priority: 1,
            idempotency_key: None,
            max_attempts: 1,
            available_at: None,
        })
        .await?;
    let generic_claim = queue
        .claim_next("r4-actor-a-generic")
        .await?
        .expect("generic owner B claim");
    assert_eq!(generic_claim.job.id, generic.id);
    assert_raw_actor(
        &worker,
        worker_backend_pid,
        Some(&actor_a),
        "generic claim preserves worker actor A",
    )
    .await?;
    queue.settle_success(&generic_claim).await?;
    assert_raw_actor(
        &worker,
        worker_backend_pid,
        Some(&actor_a),
        "generic attempt settlement preserves worker actor A",
    )
    .await?;

    let claim = queue
        .claim_next_for(
            "r4-atomic-worker",
            OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE,
        )
        .await?
        .expect("typed R4 claim");
    assert_raw_actor(
        &worker,
        worker_backend_pid,
        Some(&actor_a),
        "R4 claim and attempt insertion preserve worker actor A",
    )
    .await?;
    let engine = synthetic_engine('a');
    let pair = synthetic_pair(&fixture, &engine);

    let mut uncommitted = worker.begin().await?;
    sqlx::query(
        "INSERT INTO public.owner_beta_simulation_result_pairs
            (run_id, owner_user_id, illustrative_canonical, illustrative_sha256,
             zero_cost_canonical, zero_cost_sha256)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(fixture.input.run_id())
    .bind(owner_b)
    .bind(pair.illustrative_10bp_per_side_bytes())
    .bind(pair.illustrative_10bp_per_side_sha256().as_str())
    .bind(pair.zero_cost_reference_bytes())
    .bind(pair.zero_cost_reference_sha256().as_str())
    .execute(&mut *uncommitted)
    .await?;
    assert_raw_actor_in_transaction(
        &mut uncommitted,
        &actor_a,
        "R4 pair validation restores actor A in its writing transaction",
    )
    .await?;
    let app_pair_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.owner_beta_simulation_result_pairs WHERE run_id = $1",
    )
    .bind(fixture.input.run_id())
    .fetch_one(&app)
    .await?;
    assert_eq!(
        app_pair_count, 0,
        "uncommitted pair is invisible on a separate app connection"
    );
    let mut app_read = app.begin().await?;
    set_actor(&mut app_read, owner_b).await?;
    let actor_pair_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.owner_beta_simulation_result_pairs WHERE run_id = $1",
    )
    .bind(fixture.input.run_id())
    .fetch_one(&mut *app_read)
    .await?;
    assert_eq!(
        actor_pair_count, 0,
        "owner actor cannot see an uncommitted pair"
    );
    app_read.rollback().await?;
    uncommitted.rollback().await?;

    let published =
        publish_simulation_success(&queue, &claim, &fixture.input, &pair, &engine).await?;
    assert_eq!(published, SimulationPublicationOutcome::Published);
    assert_raw_actor(
        &worker,
        worker_backend_pid,
        Some(&actor_a),
        "nested run, pair, attempt and deferred callbacks preserve actor A",
    )
    .await?;
    let stored: (Vec<u8>, String, Vec<u8>, String) = sqlx::query_as(
        "SELECT illustrative_canonical, illustrative_sha256,
                zero_cost_canonical, zero_cost_sha256
           FROM public.owner_beta_simulation_result_pairs WHERE run_id = $1",
    )
    .bind(fixture.input.run_id())
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(stored.0, pair.illustrative_10bp_per_side_bytes());
    assert_eq!(stored.1, pair.illustrative_10bp_per_side_sha256().as_str());
    assert_eq!(stored.2, pair.zero_cost_reference_bytes());
    assert_eq!(stored.3, pair.zero_cost_reference_sha256().as_str());
    let lifecycle: (String, String, String, Uuid) = sqlx::query_as(
        "SELECT run.status, job.status, attempt.outcome, run.published_attempt_id
           FROM public.owner_beta_simulation_runs AS run
           JOIN public.jobs AS job ON job.id = run.job_id
           JOIN public.job_attempts AS attempt ON attempt.id = run.published_attempt_id
          WHERE run.id = $1",
    )
    .bind(fixture.input.run_id())
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(lifecycle.0, "SUCCEEDED");
    assert_eq!(lifecycle.1, "SUCCEEDED");
    assert_eq!(lifecycle.2, "SUCCEEDED");
    let app_pair_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.owner_beta_simulation_result_pairs WHERE run_id = $1",
    )
    .bind(fixture.input.run_id())
    .fetch_one(&app)
    .await?;
    assert_eq!(
        app_pair_count, 0,
        "unset actor remains invisible after publication"
    );
    let mut actor_tx = app.begin().await?;
    set_actor(&mut actor_tx, owner_b).await?;
    let visible: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.owner_beta_simulation_result_pairs WHERE run_id = $1",
    )
    .bind(fixture.input.run_id())
    .fetch_one(&mut *actor_tx)
    .await?;
    assert_eq!(
        visible, 1,
        "app sees both profiles only after the full commit"
    );
    actor_tx.commit().await?;

    let mut deferred = worker.begin().await?;
    let noop_job_update = sqlx::query("UPDATE public.jobs SET status = status WHERE id = $1")
        .bind(fixture.job_id)
        .execute(&mut *deferred)
        .await?;
    assert_eq!(noop_job_update.rows_affected(), 1);
    assert_raw_actor_in_transaction(
        &mut deferred,
        &actor_a,
        "the jobs status trigger restores actor A before deferred checks",
    )
    .await?;
    sqlx::query("SET CONSTRAINTS ALL IMMEDIATE")
        .execute(&mut *deferred)
        .await?;
    assert_raw_actor_in_transaction(
        &mut deferred,
        &actor_a,
        "forced candidate and R4 deferred callbacks preserve actor A",
    )
    .await?;
    deferred.commit().await?;
    assert_raw_actor(
        &worker,
        worker_backend_pid,
        Some(&actor_a),
        "worker pool retains actor A after forced deferred checks",
    )
    .await?;

    let savepoint_fixture = seed_fixture(db, owner_b, &app, "actor-savepoint").await?;
    let savepoint_claim = queue
        .claim_next_for(
            "r4-actor-savepoint-worker",
            OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE,
        )
        .await?
        .expect("savepoint fixture claim");
    let mut scoped_write = worker.begin().await?;
    assert_raw_actor_in_transaction(
        &mut scoped_write,
        &actor_a,
        "actor A is set before the savepoint",
    )
    .await?;
    let before_attempt: (Uuid, i32, String, Option<String>) = sqlx::query_as(
        "SELECT job_id, attempt_no, outcome, claimed_by
           FROM public.job_attempts WHERE id = $1",
    )
    .bind(savepoint_claim.attempt.id)
    .fetch_one(&mut *scoped_write)
    .await?;
    let before_job_status: JobStatus =
        sqlx::query_scalar("SELECT status FROM public.jobs WHERE id = $1")
            .bind(savepoint_fixture.job_id)
            .fetch_one(&mut *scoped_write)
            .await?;
    sqlx::query("SAVEPOINT actor_scope_rollback")
        .execute(&mut *scoped_write)
        .await?;
    sqlx::query(
        "UPDATE public.job_attempts
            SET outcome = 'ORPHANED', finished_at = pg_catalog.now()
          WHERE id = $1",
    )
    .bind(savepoint_claim.attempt.id)
    .execute(&mut *scoped_write)
    .await?;
    assert_raw_actor_in_transaction(
        &mut scoped_write,
        &actor_a,
        "successful attempt guard write restores actor A before savepoint rollback",
    )
    .await?;
    let late_validation =
        sqlx::query("UPDATE public.job_attempts SET attempt_no = attempt_no + 1 WHERE id = $1")
            .bind(savepoint_claim.attempt.id)
            .execute(&mut *scoped_write)
            .await
            .expect_err("attempt identity remains immutable after scoped actor work");
    assert_sqlstate(
        &late_validation,
        "42501",
        "late attempt validation rejects immutable identity",
    );
    sqlx::query("ROLLBACK TO SAVEPOINT actor_scope_rollback")
        .execute(&mut *scoped_write)
        .await?;
    assert_raw_actor_in_transaction(
        &mut scoped_write,
        &actor_a,
        "savepoint rollback preserves the original raw actor A",
    )
    .await?;
    let after_attempt: (Uuid, i32, String, Option<String>) = sqlx::query_as(
        "SELECT job_id, attempt_no, outcome, claimed_by
           FROM public.job_attempts WHERE id = $1",
    )
    .bind(savepoint_claim.attempt.id)
    .fetch_one(&mut *scoped_write)
    .await?;
    let after_job_status: JobStatus =
        sqlx::query_scalar("SELECT status FROM public.jobs WHERE id = $1")
            .bind(savepoint_fixture.job_id)
            .fetch_one(&mut *scoped_write)
            .await?;
    assert_eq!(after_attempt, before_attempt);
    assert_eq!(after_job_status, before_job_status);
    scoped_write.rollback().await?;

    assert_eq!(
        reconcile_simulation_commit(&queue, &claim, &fixture.input, &pair, &engine).await?,
        SimulationCommitState::Published
    );
    Ok(())
}

#[tokio::test]
async fn r4_partial_pair_and_transaction_failure_hidden() {
    let Some(db) = require_scratch_db().await else {
        return;
    };
    r4_partial_pair_and_transaction_failure_hidden_body(&db)
        .await
        .unwrap();
    db.drop_db().await;
}

async fn r4_partial_pair_and_transaction_failure_hidden_body(db: &ScratchDb) -> TestResult {
    let owner = new_owner(db, "r4-partial").await?;
    let app = role_pool(db, "app", None).await;
    let fixture = seed_fixture(db, owner, &app, "partial").await?;
    let worker = role_pool(db, "worker", None).await;
    pin_inactive_candidate_control_for_test(db, &worker).await?;
    let queue = JobQueue::new(worker.clone(), None, queue_config(Duration::from_secs(60)));
    let claim = queue
        .claim_next_for(
            "r4-partial-worker",
            OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE,
        )
        .await?
        .expect("typed R4 claim");
    let engine = synthetic_engine('b');
    let pair = synthetic_pair(&fixture, &engine);

    let mut success_without_pair = worker.begin().await?;
    let failed_success_commit = sqlx::query(
        "UPDATE public.owner_beta_simulation_runs
            SET status = 'SUCCEEDED', engine_json = $2, published_attempt_id = $3,
                started_at = now(), finished_at = now(), updated_at = now()
          WHERE id = $1",
    )
    .bind(fixture.input.run_id())
    .bind(serde_json::to_value(&engine)?)
    .bind(claim.attempt.id)
    .execute(&mut *success_without_pair)
    .await;
    assert!(
        failed_success_commit.is_ok(),
        "deferred guard waits until commit"
    );
    assert!(success_without_pair.commit().await.is_err());

    let mut pair_under_failure = worker.begin().await?;
    sqlx::query(
        "INSERT INTO public.owner_beta_simulation_result_pairs
            (run_id, owner_user_id, illustrative_canonical, illustrative_sha256,
             zero_cost_canonical, zero_cost_sha256)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(fixture.input.run_id())
    .bind(owner)
    .bind(pair.illustrative_10bp_per_side_bytes())
    .bind(pair.illustrative_10bp_per_side_sha256().as_str())
    .bind(pair.zero_cost_reference_bytes())
    .bind(pair.zero_cost_reference_sha256().as_str())
    .execute(&mut *pair_under_failure)
    .await?;
    sqlx::query(
        "UPDATE public.owner_beta_simulation_runs
            SET status = 'FAILED', error_code = 'SIMULATION_RESULT_INVALID',
                finished_at = now(), updated_at = now()
          WHERE id = $1",
    )
    .bind(fixture.input.run_id())
    .execute(&mut *pair_under_failure)
    .await?;
    assert!(pair_under_failure.commit().await.is_err());

    sqlx::raw_sql(
        "CREATE FUNCTION public.r4_test_fail_simulation_success() RETURNS trigger
         LANGUAGE plpgsql AS $r4_test$
         BEGIN
             IF NEW.status = 'SUCCEEDED' THEN
                 RAISE EXCEPTION 'test-only post-pair publication failure';
             END IF;
             RETURN NEW;
         END
         $r4_test$;
         CREATE TRIGGER r4_test_fail_simulation_success
         BEFORE UPDATE OF status ON public.owner_beta_simulation_runs
         FOR EACH ROW EXECUTE FUNCTION public.r4_test_fail_simulation_success();",
    )
    .execute(&db.pool)
    .await?;
    let failed = publish_simulation_success(&queue, &claim, &fixture.input, &pair, &engine).await;
    assert!(
        failed.is_err(),
        "post-insert run failure rejects the transaction"
    );
    sqlx::raw_sql(
        "DROP TRIGGER r4_test_fail_simulation_success ON public.owner_beta_simulation_runs;
         DROP FUNCTION public.r4_test_fail_simulation_success();",
    )
    .execute(&db.pool)
    .await?;
    let rollback_state: (i64, String, JobStatus) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM public.owner_beta_simulation_result_pairs
                  WHERE run_id = $1), run.status, job.status
           FROM public.owner_beta_simulation_runs AS run
           JOIN public.jobs AS job ON job.id = run.job_id
          WHERE run.id = $1",
    )
    .bind(fixture.input.run_id())
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(
        rollback_state,
        (0, "PENDING".to_owned(), JobStatus::Running)
    );

    let outcome =
        publish_simulation_success(&queue, &claim, &fixture.input, &pair, &engine).await?;
    assert_eq!(outcome, SimulationPublicationOutcome::Published);
    let stored: (Vec<u8>, String, Vec<u8>, String) = sqlx::query_as(
        "SELECT illustrative_canonical, illustrative_sha256,
                zero_cost_canonical, zero_cost_sha256
           FROM public.owner_beta_simulation_result_pairs WHERE run_id = $1",
    )
    .bind(fixture.input.run_id())
    .fetch_one(&db.pool)
    .await?;
    let checked = check_stored_simulation_pair(
        &stored.0,
        &stored.1,
        &stored.2,
        &stored.3,
        &fixture.input,
        &engine,
    )?;
    assert_eq!(
        checked.illustrative_10bp_per_side_sha256(),
        pair.illustrative_10bp_per_side_sha256()
    );
    assert!(matches!(
        check_stored_simulation_pair(
            b"tampered illustrative result",
            &stored.1,
            &stored.2,
            &stored.3,
            &fixture.input,
            &engine
        ),
        Err(SimulationPublicationError::Integrity)
    ));
    assert!(matches!(
        check_stored_simulation_pair(
            &stored.0,
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            &stored.2,
            &stored.3,
            &fixture.input,
            &engine
        ),
        Err(SimulationPublicationError::Integrity)
    ));
    assert!(matches!(
        check_stored_simulation_pair(
            &stored.0,
            &stored.1,
            b"tampered zero-cost result",
            &stored.3,
            &fixture.input,
            &engine
        ),
        Err(SimulationPublicationError::Integrity)
    ));
    assert!(matches!(
        check_stored_simulation_pair(
            &stored.0,
            &stored.1,
            &stored.2,
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            &fixture.input,
            &engine
        ),
        Err(SimulationPublicationError::Integrity)
    ));
    assert!(matches!(
        check_stored_simulation_pair(
            b"",
            &stored.1,
            &stored.2,
            &stored.3,
            &fixture.input,
            &engine
        ),
        Err(SimulationPublicationError::Integrity)
    ));
    assert!(matches!(
        check_stored_simulation_pair(
            &stored.0,
            &stored.1,
            b"",
            &stored.3,
            &fixture.input,
            &engine
        ),
        Err(SimulationPublicationError::Integrity)
    ));
    Ok(())
}

#[tokio::test]
async fn r4_typed_claim_and_generic_exclusion() {
    let Some(db) = require_scratch_db().await else {
        return;
    };
    r4_typed_claim_and_generic_exclusion_body(&db)
        .await
        .unwrap();
    db.drop_db().await;
}

async fn r4_typed_claim_and_generic_exclusion_body(db: &ScratchDb) -> TestResult {
    let owner = new_owner(db, "r4-typed-only").await?;
    let app = role_pool(db, "app", None).await;
    let fixture = seed_fixture(db, owner, &app, "typed-only").await?;
    let worker = role_pool(db, "worker", None).await;
    let queue = JobQueue::new(worker, None, queue_config(Duration::from_secs(30)));
    let fixture_queue = JobQueue::new(db.pool.clone(), None, queue_config(Duration::from_secs(30)));
    let worker_role = queue.pool();
    let worker_identity: (String, String) =
        sqlx::query_as("SELECT current_user::text, session_user::text")
            .fetch_one(&worker_role)
            .await?;
    assert_eq!(worker_identity, ("worker".to_owned(), "worker".to_owned()));
    let ordinary = fixture_queue
        .submit(job_queue::SubmitJob {
            owner_user_id: owner,
            job_type: "backtest".to_owned(),
            payload: json!({"kind": "ordinary-test"}),
            priority: 1,
            idempotency_key: None,
            max_attempts: 1,
            available_at: None,
        })
        .await?;
    let untyped = queue
        .claim_next("r4-generic-claim")
        .await?
        .expect("untyped queue still selects an ordinary eligible job");
    assert_eq!(untyped.job.id, ordinary.id);
    queue.settle_success(&untyped).await?;
    let worker_a = role_pool(db, "worker", None).await;
    let worker_b = role_pool(db, "worker", None).await;
    let queue_a = JobQueue::new(worker_a, None, queue_config(Duration::from_secs(30)));
    let queue_b = JobQueue::new(worker_b, None, queue_config(Duration::from_secs(30)));
    let (claim_a, claim_b) = tokio::join!(
        queue_a.claim_next_for(
            "r4-exact-claim-a",
            OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE
        ),
        queue_b.claim_next_for(
            "r4-exact-claim-b",
            OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE
        ),
    );
    let claim_a = claim_a?;
    let claim_b = claim_b?;
    assert_ne!(claim_a.is_some(), claim_b.is_some());
    let typed = claim_a
        .or(claim_b)
        .expect("one exact R4 worker claims the simulation");
    assert_eq!(typed.job.id, fixture.job_id);
    assert_eq!(typed.attempt.attempt_no, 1);

    let generic_target = fixture_queue
        .submit(job_queue::SubmitJob {
            owner_user_id: owner,
            job_type: "backtest".to_owned(),
            payload: json!({"kind": "r4-attempt-reassignment-target"}),
            priority: 1,
            idempotency_key: None,
            max_attempts: 2,
            available_at: None,
        })
        .await?;
    let target_slot_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.job_attempts
          WHERE job_id = $1 AND attempt_no = 1",
    )
    .bind(generic_target.id)
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(target_slot_count, 0, "R4-to-generic target slot is unused");
    let r4_to_generic = sqlx::query("UPDATE public.job_attempts SET job_id = $2 WHERE id = $1")
        .bind(typed.attempt.id)
        .bind(generic_target.id)
        .execute(&worker_role)
        .await
        .expect_err("an R4 attempt cannot be reassigned to a generic job");
    assert_sqlstate(
        &r4_to_generic,
        "42501",
        "R4-to-generic reassignment reaches the immutable-parent guard",
    );
    let retained_r4_link: (Uuid, i32) =
        sqlx::query_as("SELECT job_id, attempt_no FROM public.job_attempts WHERE id = $1")
            .bind(typed.attempt.id)
            .fetch_one(&db.pool)
            .await?;
    assert_eq!(retained_r4_link, (fixture.job_id, 1));

    let generic_source = fixture_queue
        .submit(job_queue::SubmitJob {
            owner_user_id: owner,
            job_type: "backtest".to_owned(),
            payload: json!({"kind": "generic-attempt-reassignment-source"}),
            priority: 1,
            idempotency_key: None,
            max_attempts: 2,
            available_at: None,
        })
        .await?;
    let reassigned_attempt_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO public.job_attempts
            (id, job_id, attempt_no, outcome, claimed_by, started_at)
         VALUES ($1, $2, 1, 'RUNNING', 'r4-parent-reassignment-test', pg_catalog.now())",
    )
    .bind(reassigned_attempt_id)
    .bind(generic_source.id)
    .execute(&db.pool)
    .await?;
    let unused_r4 = seed_fixture(db, owner, &app, "attempt-reassignment-target").await?;
    let reverse_target_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.job_attempts
          WHERE job_id = $1 AND attempt_no = 1",
    )
    .bind(unused_r4.job_id)
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(
        reverse_target_count, 0,
        "generic-to-R4 target slot is unused"
    );
    let generic_to_r4 = sqlx::query("UPDATE public.job_attempts SET job_id = $2 WHERE id = $1")
        .bind(reassigned_attempt_id)
        .bind(unused_r4.job_id)
        .execute(&worker_role)
        .await
        .expect_err("a generic attempt cannot be reassigned to an R4 job");
    assert_sqlstate(
        &generic_to_r4,
        "42501",
        "generic-to-R4 reassignment reaches the immutable-parent guard",
    );
    let retained_generic_link: (Uuid, i32) =
        sqlx::query_as("SELECT job_id, attempt_no FROM public.job_attempts WHERE id = $1")
            .bind(reassigned_attempt_id)
            .fetch_one(&db.pool)
            .await?;
    assert_eq!(retained_generic_link, (generic_source.id, 1));

    sqlx::query(
        "UPDATE public.jobs SET locked_at = pg_catalog.now() - interval '10 seconds' WHERE id = $1",
    )
    .bind(fixture.job_id)
    .execute(&db.pool)
    .await?;
    assert_eq!(queue.sweep().await?.jobs_checked, 0);
    assert_eq!(
        queue.get_by_id(fixture.job_id).await?.status,
        JobStatus::Running
    );
    Ok(())
}

#[tokio::test]
async fn r4_lease_retry_exhaustion_and_zombie() {
    let Some(db) = require_scratch_db().await else {
        return;
    };
    r4_lease_retry_exhaustion_and_zombie_body(&db)
        .await
        .unwrap();
    db.drop_db().await;
}

async fn r4_lease_retry_exhaustion_and_zombie_body(db: &ScratchDb) -> TestResult {
    let owner = new_owner(db, "r4-recovery").await?;
    let app = role_pool(db, "app", None).await;
    let fixture = seed_fixture(db, owner, &app, "recovery").await?;
    let worker = role_pool(db, "worker", None).await;
    pin_inactive_candidate_control_for_test(db, &worker).await?;
    let queue = JobQueue::new(worker.clone(), None, queue_config(Duration::from_secs(60)));
    let claim = queue
        .claim_next_for(
            "r4-recovery-worker-1",
            OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE,
        )
        .await?
        .expect("typed R4 claim");
    assert_eq!(claim.job.id, fixture.job_id);
    assert!(
        queue.claim_next("generic-worker").await?.is_none(),
        "generic untyped claim cannot consume the R4 job"
    );
    sqlx::query("UPDATE public.jobs SET locked_at = now() - interval '10 seconds' WHERE id = $1")
        .bind(fixture.job_id)
        .execute(&db.pool)
        .await?;
    assert_eq!(
        queue.sweep().await?.jobs_checked,
        0,
        "generic sweep skips R4"
    );
    let first = recover_simulation_claims(&queue).await?;
    assert_eq!((first.jobs_requeued, first.attempts_orphaned), (1, 1));
    let first_outcome: String =
        sqlx::query_scalar("SELECT outcome FROM public.job_attempts WHERE id = $1")
            .bind(claim.attempt.id)
            .fetch_one(&db.pool)
            .await?;
    assert_eq!(first_outcome, "ORPHANED");

    tokio::time::sleep(Duration::from_millis(150)).await;
    let claim_two = queue
        .claim_next_for(
            "r4-recovery-worker-2",
            OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE,
        )
        .await?
        .expect("dedicated recovery requeues attempt 2");
    assert_eq!(claim_two.attempt.attempt_no, 2);
    assert!(matches!(
        settle_simulation_failure(
            &queue,
            &claim,
            &fixture.input,
            SimulationPublicationFailure::ResultInvalid,
        )
        .await,
        Err(SimulationPublicationError::QueueClaimLost)
    ));
    let still_running_attempt_two: (JobStatus, String) = sqlx::query_as(
        "SELECT job.status, attempt.outcome
           FROM public.jobs job
           JOIN public.job_attempts attempt
             ON attempt.job_id = job.id AND attempt.attempt_no = job.attempt_count
          WHERE job.id = $1",
    )
    .bind(fixture.job_id)
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(
        still_running_attempt_two,
        (JobStatus::Running, "RUNNING".to_owned())
    );
    let heartbeat = queue.heartbeat(&claim_two).await?;
    assert_eq!(heartbeat, job_queue::HeartbeatStatus::Extended);
    let no_op = recover_simulation_claims(&queue).await?;
    assert_eq!(no_op.jobs_checked, 0, "heartbeat wins database-clock race");
    sqlx::query("UPDATE public.jobs SET locked_at = now() - interval '10 seconds' WHERE id = $1")
        .bind(fixture.job_id)
        .execute(&db.pool)
        .await?;
    let second = recover_simulation_claims(&queue).await?;
    assert_eq!((second.jobs_requeued, second.attempts_orphaned), (1, 1));

    tokio::time::sleep(Duration::from_millis(150)).await;
    let claim_three = queue
        .claim_next_for(
            "r4-recovery-worker-3",
            OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE,
        )
        .await?
        .expect("dedicated recovery requeues attempt 3");
    assert_eq!(claim_three.attempt.attempt_no, 3);
    sqlx::query("UPDATE public.jobs SET locked_at = now() - interval '10 seconds' WHERE id = $1")
        .bind(fixture.job_id)
        .execute(&db.pool)
        .await?;
    let exhausted = recover_simulation_claims(&queue).await?;
    assert_eq!((exhausted.jobs_failed, exhausted.attempts_orphaned), (1, 1));
    let terminal: (JobStatus, String, String) = sqlx::query_as(
        "SELECT job.status, run.status, run.error_code
           FROM public.jobs AS job
           JOIN public.owner_beta_simulation_runs AS run ON run.job_id = job.id
          WHERE job.id = $1",
    )
    .bind(fixture.job_id)
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(
        terminal,
        (
            JobStatus::Failed,
            "FAILED".to_owned(),
            "SIMULATION_ATTEMPTS_EXHAUSTED".to_owned()
        )
    );
    assert_eq!(recover_simulation_claims(&queue).await?.jobs_checked, 0);

    let cancel_owner = new_owner(db, "r4-recovery-cancel").await?;
    let cancel_app = role_pool(db, "app", None).await;
    let cancel_fixture = seed_fixture(db, cancel_owner, &cancel_app, "recovery-cancel").await?;
    let cancel_claim = queue
        .claim_next_for(
            "r4-recovery-cancel-worker",
            OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE,
        )
        .await?
        .expect("running cancellation claim");
    let actor_pool = role_pool(db, "app", Some(cancel_owner)).await;
    let audit_pool = role_pool(db, "audit_writer", None).await;
    let cancel_queue = JobQueue::new(
        actor_pool,
        Some(audit_pool),
        queue_config(Duration::from_secs(60)),
    );
    let mut cancel_actor = AuditActor::new("owner");
    cancel_actor.user_id = Some(cancel_owner);
    cancel_actor.correlation_id = Some(format!("r4-recovery-cancel-{}", Uuid::new_v4()));
    cancel_queue
        .request_cancel(cancel_fixture.job_id, &cancel_actor)
        .await?;
    sqlx::query(
        "UPDATE public.jobs
            SET locked_at = pg_catalog.now() - interval '10 seconds'
          WHERE id = $1",
    )
    .bind(cancel_fixture.job_id)
    .execute(&db.pool)
    .await?;
    assert_eq!(queue.sweep().await?.jobs_checked, 0);
    let canceled_recovery = recover_simulation_claims(&queue).await?;
    assert_eq!(
        (
            canceled_recovery.runs_canceled,
            canceled_recovery.attempts_orphaned
        ),
        (1, 1)
    );
    let orphaned_outcome: String =
        sqlx::query_scalar("SELECT outcome FROM public.job_attempts WHERE id = $1")
            .bind(cancel_claim.attempt.id)
            .fetch_one(&db.pool)
            .await?;
    assert_eq!(orphaned_outcome, "ORPHANED");
    assert_eq!(
        settle_simulation_failure(
            &queue,
            &cancel_claim,
            &cancel_fixture.input,
            SimulationPublicationFailure::ResultInvalid,
        )
        .await?,
        SimulationPublicationOutcome::Canceled,
        "recovered cancellation wins over a late zombie failure"
    );

    let actor_a = new_owner(db, "r4-recovery-actor-a").await?.to_string();
    assert_generic_attempt_first_actor_scope(db, None, None, "missing-actor").await?;
    assert_generic_attempt_first_actor_scope(db, Some(""), None, "empty-actor").await?;
    assert_generic_attempt_first_actor_scope(db, Some(&actor_a), Some(&actor_a), "valid-actor-a")
        .await?;

    for actor_setting in [None, Some("")] {
        let migration_owner =
            role_pool_with_actor_value(db, "migration_owner", actor_setting).await;
        let visible_jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM public.jobs")
            .fetch_one(&migration_owner)
            .await?;
        assert_eq!(visible_jobs, 0, "NULL and empty actors are no identity");
    }
    let malformed_actor =
        role_pool_with_actor_value(db, "migration_owner", Some("not-a-uuid")).await;
    let malformed_read = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM public.jobs")
        .fetch_one(&malformed_actor)
        .await
        .expect_err("a malformed nonempty actor is rejected by the scoped owner reader");
    assert_sqlstate(
        &malformed_read,
        "22P02",
        "malformed nonempty actor is not treated as missing",
    );
    Ok(())
}

#[tokio::test]
async fn r4_cancel_wins_and_late_publish_denied() {
    let Some(db) = require_scratch_db().await else {
        return;
    };
    r4_cancel_wins_and_late_publish_denied_body(&db)
        .await
        .unwrap();
    db.drop_db().await;
}

async fn r4_cancel_wins_and_late_publish_denied_body(db: &ScratchDb) -> TestResult {
    let owner = new_owner(db, "r4-cancel").await?;
    let app = role_pool(db, "app", Some(owner)).await;
    let audit = role_pool(db, "audit_writer", None).await;
    let fixture = seed_fixture(db, owner, &role_pool(db, "app", None).await, "cancel").await?;
    let worker = role_pool(db, "worker", None).await;
    let queue = JobQueue::new(worker.clone(), None, queue_config(Duration::from_secs(60)));
    let claim = queue
        .claim_next_for(
            "r4-cancel-worker",
            OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE,
        )
        .await?
        .expect("typed R4 claim");
    let app_queue = JobQueue::new(
        app.clone(),
        Some(audit),
        queue_config(Duration::from_secs(60)),
    );
    let mut actor = AuditActor::new("owner");
    actor.user_id = Some(owner);
    actor.correlation_id = Some(format!("r4-cancel-{}", Uuid::new_v4()));

    let queued_owner = new_owner(db, "r4-queued-cancel").await?;
    let queued_app = role_pool(db, "app", None).await;
    let queued_fixture = seed_fixture(db, queued_owner, &queued_app, "queued-cancel").await?;
    let queued_owner_app = role_pool(db, "app", Some(queued_owner)).await;
    let unaudited_queue = JobQueue::new(
        queued_owner_app.clone(),
        None,
        queue_config(Duration::from_secs(60)),
    );
    let mut queued_actor = AuditActor::new("owner");
    queued_actor.user_id = Some(queued_owner);
    queued_actor.correlation_id = Some(format!("r4-queued-cancel-{}", Uuid::new_v4()));
    assert!(matches!(
        unaudited_queue
            .request_cancel(queued_fixture.job_id, &queued_actor)
            .await,
        Err(job_queue::QueueError::AuditUnavailable)
    ));
    let unchanged_queued: JobStatus =
        sqlx::query_scalar("SELECT status FROM public.jobs WHERE id = $1")
            .bind(queued_fixture.job_id)
            .fetch_one(&db.pool)
            .await?;
    assert_eq!(unchanged_queued, JobStatus::Queued);
    let queued_audit = role_pool(db, "audit_writer", None).await;
    let queued_app_queue = JobQueue::new(
        queued_owner_app,
        Some(queued_audit),
        queue_config(Duration::from_secs(60)),
    );
    assert!(matches!(
        queued_app_queue
            .request_cancel(queued_fixture.job_id, &queued_actor)
            .await?,
        job_queue::CancelResult::Canceled(_)
    ));
    let queued_recovery = recover_simulation_claims(&queue).await?;
    assert_eq!(queued_recovery.runs_canceled, 1);
    let queued_state: (JobStatus, String, i64) = sqlx::query_as(
        "SELECT job.status, run.status,
                (SELECT count(*) FROM public.owner_beta_simulation_result_pairs pair
                  WHERE pair.run_id = run.id)
           FROM public.jobs job
           JOIN public.owner_beta_simulation_runs run ON run.job_id = job.id
          WHERE job.id = $1",
    )
    .bind(queued_fixture.job_id)
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(
        queued_state,
        (JobStatus::Canceled, "CANCELED".to_owned(), 0)
    );

    let canceled = app_queue.request_cancel(fixture.job_id, &actor).await?;
    assert!(matches!(canceled, job_queue::CancelResult::Canceled(_)));

    let engine = synthetic_engine('c');
    let pair = synthetic_pair(&fixture, &engine);
    let late = publish_simulation_success(&queue, &claim, &fixture.input, &pair, &engine).await?;
    assert_eq!(late, SimulationPublicationOutcome::Canceled);
    assert_eq!(
        settle_simulation_failure(
            &queue,
            &claim,
            &fixture.input,
            SimulationPublicationFailure::ResultInvalid,
        )
        .await?,
        SimulationPublicationOutcome::Canceled,
        "an already-canceled stale worker cannot convert the run to a failure"
    );
    let state: (JobStatus, String, String, i64) = sqlx::query_as(
        "SELECT job.status, run.status, attempt.outcome,
                (SELECT count(*) FROM public.owner_beta_simulation_result_pairs p
                  WHERE p.run_id = run.id)
           FROM public.jobs AS job
           JOIN public.owner_beta_simulation_runs AS run ON run.job_id = job.id
           JOIN public.job_attempts AS attempt ON attempt.id = $2
          WHERE job.id = $1",
    )
    .bind(fixture.job_id)
    .bind(claim.attempt.id)
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(
        state,
        (
            JobStatus::Canceled,
            "CANCELED".to_owned(),
            "FAILED".to_owned(),
            0
        )
    );

    let success_owner = new_owner(db, "r4-success-before-cancel").await?;
    let success_app = role_pool(db, "app", None).await;
    let successful = seed_fixture(db, success_owner, &success_app, "success-before-cancel").await?;
    let success_claim = queue
        .claim_next_for(
            "r4-success-first-worker",
            OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE,
        )
        .await?
        .expect("success-first typed claim");
    let success_pair = synthetic_pair(&successful, &engine);
    assert_eq!(
        publish_simulation_success(
            &queue,
            &success_claim,
            &successful.input,
            &success_pair,
            &engine
        )
        .await?,
        SimulationPublicationOutcome::Published
    );
    let success_owner_app = role_pool(db, "app", Some(success_owner)).await;
    let success_audit = role_pool(db, "audit_writer", None).await;
    let success_app_queue = JobQueue::new(
        success_owner_app,
        Some(success_audit),
        queue_config(Duration::from_secs(60)),
    );
    let mut success_actor = AuditActor::new("owner");
    success_actor.user_id = Some(success_owner);
    success_actor.correlation_id = Some(format!("r4-success-cancel-{}", Uuid::new_v4()));
    let already_terminal = success_app_queue
        .request_cancel(successful.job_id, &success_actor)
        .await?;
    assert!(matches!(
        already_terminal,
        job_queue::CancelResult::AlreadyTerminal(_)
    ));
    let retained: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.owner_beta_simulation_result_pairs WHERE run_id = $1",
    )
    .bind(successful.input.run_id())
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(retained, 1, "terminal success keeps its immutable pair");
    Ok(())
}

#[tokio::test]
async fn r4_migration_up_down_guards() {
    let Some(db) = require_scratch_db().await else {
        return;
    };
    r4_migration_up_down_guards_body(&db).await.unwrap();
    db.drop_db().await;
}

async fn r4_migration_up_down_guards_body(db: &ScratchDb) -> TestResult {
    let migration_owner = role_pool(db, "migration_owner", None).await;
    let before_policy = jobs_owner_policy_snapshot(&migration_owner).await?;
    let before_callback = candidate_settlement_snapshot(&migration_owner).await?;
    let mut down = migration_owner.begin().await?;
    sqlx::raw_sql(include_str!(
        "../../../migrations/0063_owner_beta_price_only_simulations.down.sql"
    ))
    .execute(&mut *down)
    .await?;
    down.commit().await?;
    let down_policy = jobs_owner_policy_snapshot(&migration_owner).await?;
    let down_callback = candidate_settlement_snapshot(&migration_owner).await?;
    assert_eq!(down_policy.roles, vec!["migration_owner".to_owned()]);
    assert_eq!(down_policy.command, "ALL");
    assert_eq!(down_policy.permissive, "PERMISSIVE");
    let legacy_using = down_policy
        .using_expression
        .as_deref()
        .expect("legacy jobs owner USING expression");
    let legacy_check = down_policy
        .check_expression
        .as_deref()
        .expect("legacy jobs owner WITH CHECK expression");
    assert_ne!(
        before_policy.using_expression, down_policy.using_expression,
        "down restores the historical owner expression before up reinstalls the repair"
    );
    assert_eq!(legacy_using, legacy_check);
    assert!(legacy_using.contains("current_setting"));
    assert!(legacy_using.contains("uuid"));
    assert!(
        !legacy_using.contains("NULLIF"),
        "down restores the historical raw current_setting cast"
    );
    assert_ne!(before_callback.definition, down_callback.definition);
    assert!(down_callback.definition.contains(&down_callback.body));
    assert!(
        down_callback.definition.contains(
            "CREATE OR REPLACE FUNCTION public.assert_candidate_publication_settlement()"
        )
    );
    assert_eq!(down_callback.body, legacy_candidate_settlement_body());
    assert_eq!(down_callback.owner, "migration_owner");
    assert_eq!(down_callback.configuration, "{search_path=pg_catalog}");
    assert_eq!(down_callback.acl, before_callback.acl);
    assert!(down_callback.security_definer);
    assert_eq!(down_callback.language, "plpgsql");

    let mut up = migration_owner.begin().await?;
    sqlx::raw_sql(include_str!(
        "../../../migrations/0063_owner_beta_price_only_simulations.up.sql"
    ))
    .execute(&mut *up)
    .await?;
    up.commit().await?;
    let after_policy = jobs_owner_policy_snapshot(&migration_owner).await?;
    let after_callback = candidate_settlement_snapshot(&migration_owner).await?;
    assert_eq!(
        before_policy, after_policy,
        "empty down/up restores jobs owner policy"
    );
    assert_eq!(
        before_callback, after_callback,
        "empty down/up restores the complete scoped candidate callback"
    );

    let actor = new_owner(db, "r4-down-actor").await?;
    let hidden_owner = new_owner(db, "r4-down-hidden-owner").await?;
    let app = role_pool(db, "app", None).await;
    let hidden = seed_fixture(db, hidden_owner, &app, "hidden-history").await?;
    drop(migration_owner);
    let foreign_scoped_migration_owner = role_pool(db, "migration_owner", Some(actor)).await;
    let mut populated_down = foreign_scoped_migration_owner.begin().await?;
    let refusal = sqlx::raw_sql(include_str!(
        "../../../migrations/0063_owner_beta_price_only_simulations.down.sql"
    ))
    .execute(&mut *populated_down)
    .await;
    assert!(
        refusal.is_err(),
        "a hidden owner's history prevents rollback"
    );
    populated_down.rollback().await?;
    let still_forced: bool = sqlx::query_scalar(
        "SELECT c.relforcerowsecurity FROM pg_catalog.pg_class c
          WHERE c.oid = 'public.owner_beta_simulation_runs'::regclass",
    )
    .fetch_one(&db.pool)
    .await?;
    let retained: i64 =
        sqlx::query_scalar("SELECT count(*) FROM public.owner_beta_simulation_runs WHERE id = $1")
            .bind(hidden.input.run_id())
            .fetch_one(&db.pool)
            .await?;
    assert!(still_forced);
    assert_eq!(retained, 1, "refused down preserves foreign history");
    Ok(())
}

#[tokio::test]
async fn r4_read_revalidates_both_profiles() {
    let Some(db) = require_scratch_db().await else {
        return;
    };
    r4_read_revalidates_both_profiles_body(&db).await.unwrap();
    db.drop_db().await;
}

async fn r4_read_revalidates_both_profiles_body(db: &ScratchDb) -> TestResult {
    let owner = new_owner(db, "r4-read-check").await?;
    let app = role_pool(db, "app", None).await;
    let fixture = seed_fixture(db, owner, &app, "read-check").await?;
    let worker = role_pool(db, "worker", None).await;
    let queue = JobQueue::new(worker.clone(), None, queue_config(Duration::from_secs(60)));
    let engine = synthetic_engine('d');
    let pair = publish_fixture(&fixture, &worker, &queue, &engine).await?;
    let stored: (Vec<u8>, String, Vec<u8>, String) = sqlx::query_as(
        "SELECT illustrative_canonical, illustrative_sha256,
                zero_cost_canonical, zero_cost_sha256
           FROM public.owner_beta_simulation_result_pairs WHERE run_id = $1",
    )
    .bind(fixture.input.run_id())
    .fetch_one(&db.pool)
    .await?;
    let checked = check_stored_simulation_pair(
        &stored.0,
        &stored.1,
        &stored.2,
        &stored.3,
        &fixture.input,
        &engine,
    )?;
    assert_eq!(
        checked.illustrative_10bp_per_side_bytes(),
        pair.illustrative_10bp_per_side_bytes()
    );
    assert_eq!(
        checked.zero_cost_reference_bytes(),
        pair.zero_cost_reference_bytes()
    );
    let mut altered_illustrative = stored.0.clone();
    altered_illustrative[0] ^= 1;
    assert!(matches!(
        check_stored_simulation_pair(
            &altered_illustrative,
            &stored.1,
            &stored.2,
            &stored.3,
            &fixture.input,
            &engine
        ),
        Err(SimulationPublicationError::Integrity)
    ));
    let mut altered_zero_cost = stored.2.clone();
    altered_zero_cost[0] ^= 1;
    assert!(matches!(
        check_stored_simulation_pair(
            &stored.0,
            &stored.1,
            &altered_zero_cost,
            &stored.3,
            &fixture.input,
            &engine
        ),
        Err(SimulationPublicationError::Integrity)
    ));
    assert!(matches!(
        check_stored_simulation_pair(
            b"",
            &stored.1,
            &stored.2,
            &stored.3,
            &fixture.input,
            &engine
        ),
        Err(SimulationPublicationError::Integrity)
    ));
    assert!(matches!(
        check_stored_simulation_pair(
            &stored.0,
            &stored.1,
            b"",
            &stored.3,
            &fixture.input,
            &engine
        ),
        Err(SimulationPublicationError::Integrity)
    ));

    let mut corrupted_illustrative = stored.0.clone();
    corrupted_illustrative[0] ^= 1;
    let mut corrupt = db.pool.begin().await?;
    sqlx::query("SET LOCAL session_replication_role = 'replica'")
        .execute(&mut *corrupt)
        .await?;
    sqlx::query(
        "UPDATE public.owner_beta_simulation_result_pairs
            SET illustrative_canonical = $2 WHERE run_id = $1",
    )
    .bind(fixture.input.run_id())
    .bind(&corrupted_illustrative)
    .execute(&mut *corrupt)
    .await?;
    sqlx::query(
        "UPDATE public.owner_beta_simulation_runs
            SET status = 'FAILED', error_code = 'SIMULATION_RESULT_INVALID',
                engine_json = NULL, published_attempt_id = NULL,
                finished_at = pg_catalog.now(), updated_at = pg_catalog.now()
          WHERE id = $1",
    )
    .bind(fixture.input.run_id())
    .execute(&mut *corrupt)
    .await?;
    corrupt.commit().await?;

    let owner_app = role_pool(db, "app", None).await;
    let mut actor_tx = owner_app.begin().await?;
    set_actor(&mut actor_tx, owner).await?;
    let hidden_pair_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.owner_beta_simulation_result_pairs WHERE run_id = $1",
    )
    .bind(fixture.input.run_id())
    .fetch_one(&mut *actor_tx)
    .await?;
    assert_eq!(
        hidden_pair_count, 0,
        "app cannot see a corrupt non-success pair"
    );
    actor_tx.rollback().await?;
    let corrupted: (Vec<u8>, String, Vec<u8>, String) = sqlx::query_as(
        "SELECT illustrative_canonical, illustrative_sha256,
                zero_cost_canonical, zero_cost_sha256
           FROM public.owner_beta_simulation_result_pairs WHERE run_id = $1",
    )
    .bind(fixture.input.run_id())
    .fetch_one(&db.pool)
    .await?;
    assert!(matches!(
        check_stored_simulation_pair(
            &corrupted.0,
            &corrupted.1,
            &corrupted.2,
            &corrupted.3,
            &fixture.input,
            &engine
        ),
        Err(SimulationPublicationError::Integrity)
    ));
    Ok(())
}

#[tokio::test]
async fn r4_entitlement_covers_every_consumed_session() {
    let Some(db) = require_scratch_db().await else {
        return;
    };
    r4_entitlement_covers_every_consumed_session_body(&db)
        .await
        .unwrap();
    db.drop_db().await;
}

async fn r4_entitlement_covers_every_consumed_session_body(db: &ScratchDb) -> TestResult {
    let owner = new_owner(db, "r4-rights-gap").await?;
    let app = role_pool(db, "app", None).await;
    let fixture = seed_fixture(db, owner, &app, "rights-gap").await?;
    let worker = role_pool(db, "worker", None).await;
    let queue = JobQueue::new(worker, None, queue_config(Duration::from_secs(60)));
    let claim = queue
        .claim_next_for(
            "r4-rights-worker",
            OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE,
        )
        .await?
        .expect("typed simulation claim");
    let engine = synthetic_engine('g');
    let pair = synthetic_pair(&fixture, &engine);
    let dates = fixture
        .input
        .schedule()
        .warmup_sessions()
        .iter()
        .chain(fixture.input.schedule().evaluation_sessions())
        .map(|date| date.as_naive_date())
        .collect::<Vec<_>>();
    let gap = dates.len() / 2;
    let entitlement_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM public.data_entitlements
          WHERE contract_reference = 'fixture://r4-simulation-rights-gap'",
    )
    .fetch_one(&db.pool)
    .await?;
    sqlx::query("UPDATE public.data_entitlements SET effective_until = $2 WHERE id = $1")
        .bind(entitlement_id)
        .bind(dates[gap - 1])
        .execute(&db.pool)
        .await?;
    let second_entitlement: Uuid = sqlx::query_scalar(
        "INSERT INTO public.data_entitlements
            (contract_document_sha256, contract_reference, status,
             covered_datasets, covered_uses, effective_from, effective_until, managed_by)
         VALUES (repeat('f', 64), 'fixture://r4-simulation-rights-gap-second', 'ACTIVE',
                 '[\"krx_eod_bars\"]'::jsonb, '[\"backtest\"]'::jsonb,
                 $1, DATE '2026-08-28', $2)
         RETURNING id",
    )
    .bind(dates[gap + 1])
    .bind(owner)
    .fetch_one(&db.pool)
    .await?;

    assert!(matches!(
        publish_simulation_success(&queue, &claim, &fixture.input, &pair, &engine).await,
        Err(SimulationPublicationError::EntitlementDenied)
    ));
    let rejected: (JobStatus, String, i64) = sqlx::query_as(
        "SELECT job.status, run.status,
                (SELECT count(*) FROM public.owner_beta_simulation_result_pairs pair
                  WHERE pair.run_id = run.id)
           FROM public.jobs job
           JOIN public.owner_beta_simulation_runs run ON run.job_id = job.id
          WHERE job.id = $1",
    )
    .bind(fixture.job_id)
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(rejected, (JobStatus::Running, "PENDING".to_owned(), 0));

    sqlx::query("UPDATE public.data_entitlements SET effective_from = $2 WHERE id = $1")
        .bind(second_entitlement)
        .bind(dates[gap])
        .execute(&db.pool)
        .await?;
    assert_eq!(
        publish_simulation_success(&queue, &claim, &fixture.input, &pair, &engine).await?,
        SimulationPublicationOutcome::Published,
        "two active rights may cover the complete schedule as a union"
    );
    Ok(())
}

#[tokio::test]
async fn r4_failure_settlement_retries_only_infrastructure_errors() {
    let Some(db) = require_scratch_db().await else {
        return;
    };
    r4_failure_settlement_retries_only_infrastructure_errors_body(&db)
        .await
        .unwrap();
    db.drop_db().await;
}

async fn r4_failure_settlement_retries_only_infrastructure_errors_body(
    db: &ScratchDb,
) -> TestResult {
    let owner = new_owner(db, "r4-failure").await?;
    let app = role_pool(db, "app", None).await;
    let fixture = seed_fixture(db, owner, &app, "failure").await?;
    let worker = role_pool(db, "worker", None).await;
    let queue = JobQueue::new(worker.clone(), None, queue_config(Duration::from_secs(60)));
    let claim = queue
        .claim_next_for(
            "r4-failure-worker",
            OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE,
        )
        .await?
        .expect("typed R4 claim");
    let outcome = settle_simulation_failure(
        &queue,
        &claim,
        &fixture.input,
        SimulationPublicationFailure::InfrastructureUnavailable,
    )
    .await?;
    assert_eq!(outcome, SimulationPublicationOutcome::Retrying);
    let retry_state: (JobStatus, String, i64) = sqlx::query_as(
        "SELECT job.status, run.status,
                (SELECT count(*) FROM public.owner_beta_simulation_result_pairs p
                  WHERE p.run_id = run.id)
           FROM public.jobs job
           JOIN public.owner_beta_simulation_runs run ON run.job_id = job.id
          WHERE job.id = $1",
    )
    .bind(fixture.job_id)
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(retry_state, (JobStatus::Queued, "PENDING".to_owned(), 0));

    tokio::time::sleep(Duration::from_millis(150)).await;
    let retry_claim = queue
        .claim_next_for(
            "r4-failure-worker-2",
            OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE,
        )
        .await?
        .expect("infrastructure retry is queued");
    let failed = settle_simulation_failure(
        &queue,
        &retry_claim,
        &fixture.input,
        SimulationPublicationFailure::ResultInvalid,
    )
    .await?;
    assert_eq!(failed, SimulationPublicationOutcome::Failed);
    let final_state: (JobStatus, String, String, i64) = sqlx::query_as(
        "SELECT job.status, run.status, run.error_code,
                (SELECT count(*) FROM public.owner_beta_simulation_result_pairs p
                  WHERE p.run_id = run.id)
           FROM public.jobs job
           JOIN public.owner_beta_simulation_runs run ON run.job_id = job.id
          WHERE job.id = $1",
    )
    .bind(fixture.job_id)
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(
        final_state,
        (
            JobStatus::Failed,
            "FAILED".to_owned(),
            "SIMULATION_RESULT_INVALID".to_owned(),
            0
        )
    );
    Ok(())
}

#[tokio::test]
async fn r4_commit_unknown_reconciliation() {
    let Some(db) = require_scratch_db().await else {
        return;
    };
    r4_commit_unknown_reconciliation_body(&db).await.unwrap();
    db.drop_db().await;
}

async fn r4_commit_unknown_reconciliation_body(db: &ScratchDb) -> TestResult {
    let owner = new_owner(db, "r4-reconcile").await?;
    let app = role_pool(db, "app", None).await;
    let fixture = seed_fixture(db, owner, &app, "reconcile").await?;
    let worker = role_pool(db, "worker", None).await;
    let queue = JobQueue::new(worker.clone(), None, queue_config(Duration::from_secs(60)));
    let claim = queue
        .claim_next_for(
            "r4-reconcile-worker",
            OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE,
        )
        .await?
        .expect("typed R4 claim");
    let engine = synthetic_engine('e');
    let pair = synthetic_pair(&fixture, &engine);
    assert_eq!(
        reconcile_simulation_commit(&queue, &claim, &fixture.input, &pair, &engine).await?,
        SimulationCommitState::NotPublished
    );
    assert_eq!(
        publish_simulation_success(&queue, &claim, &fixture.input, &pair, &engine).await?,
        SimulationPublicationOutcome::Published
    );
    assert_eq!(
        reconcile_simulation_commit(&queue, &claim, &fixture.input, &pair, &engine).await?,
        SimulationCommitState::Published
    );
    let other_engine = synthetic_engine('f');
    assert_eq!(
        reconcile_simulation_commit(&queue, &claim, &fixture.input, &pair, &other_engine).await?,
        SimulationCommitState::Unresolved
    );
    Ok(())
}

#[tokio::test]
async fn r4_invalid_claim_is_terminal_and_attempt_fenced() {
    let Some(db) = require_scratch_db().await else {
        return;
    };
    r4_invalid_claim_is_terminal_and_attempt_fenced_body(&db)
        .await
        .unwrap();
    db.drop_db().await;
}

async fn r4_invalid_claim_is_terminal_and_attempt_fenced_body(db: &ScratchDb) -> TestResult {
    let owner = new_owner(db, "r4-invalid-claim").await?;
    let app = role_pool(db, "app", None).await;
    let fixture = seed_invalid_fixture(
        db,
        owner,
        &app,
        "invalid-calendar",
        "2026-08-23",
        "2026-08-28",
    )
    .await?;
    let worker = role_pool(db, "worker", None).await;
    let queue = JobQueue::new(worker, None, queue_config(Duration::from_secs(60)));
    let claim_one = queue
        .claim_next_for(
            "r4-invalid-worker-1",
            OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE,
        )
        .await?
        .expect("SQL-admitted non-calendar schedule claim");
    assert_eq!(claim_one.job.id, fixture.job_id);
    assert!(matches!(
        queue
            .settle_failure(
                &claim_one,
                job_queue::ErrorClass::Transient,
                "FIXTURE_TRANSIENT",
                "fixture transient settlement",
            )
            .await?,
        job_queue::SettleResult::Committed(job) if job.status == JobStatus::Queued
    ));
    tokio::time::sleep(Duration::from_millis(150)).await;
    let claim_two = queue
        .claim_next_for(
            "r4-invalid-worker-2",
            OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE,
        )
        .await?
        .expect("fixture has a newer attempt for fencing");
    assert_eq!(claim_two.attempt.attempt_no, 2);

    let mut wrong_owner_claim = claim_two.clone();
    wrong_owner_claim.job.owner_user_id = Uuid::new_v4();
    assert!(matches!(
        settle_invalid_simulation_claim(&queue, &wrong_owner_claim).await,
        Err(SimulationPublicationError::QueueClaimLost)
    ));
    assert!(matches!(
        settle_invalid_simulation_claim(&queue, &claim_one).await,
        Err(SimulationPublicationError::QueueClaimLost)
    ));
    let before_current_settlement: (JobStatus, i32, String, String, Option<String>, i64) =
        sqlx::query_as(
            "SELECT job.status, job.attempt_count, run.status, attempt.outcome,
                    attempt.error_code,
                    (SELECT count(*) FROM public.owner_beta_simulation_result_pairs pair
                      WHERE pair.run_id = run.id)
               FROM public.jobs job
               JOIN public.owner_beta_simulation_runs run ON run.job_id = job.id
               JOIN public.job_attempts attempt
                 ON attempt.job_id = job.id AND attempt.attempt_no = job.attempt_count
              WHERE job.id = $1",
        )
        .bind(fixture.job_id)
        .fetch_one(&db.pool)
        .await?;
    assert_eq!(
        before_current_settlement,
        (
            JobStatus::Running,
            2,
            "PENDING".to_owned(),
            "RUNNING".to_owned(),
            None,
            0,
        )
    );
    assert_eq!(
        settle_invalid_simulation_claim(&queue, &claim_two).await?,
        SimulationPublicationOutcome::Failed
    );
    type FinalSimulationState = (
        JobStatus,
        i32,
        Option<String>,
        String,
        Option<String>,
        String,
        Option<String>,
        i64,
    );
    let final_state: FinalSimulationState = sqlx::query_as(
        "SELECT job.status, job.attempt_count, job.error_code,
                    run.status, run.error_code, attempt.outcome, attempt.error_code,
                    (SELECT count(*) FROM public.owner_beta_simulation_result_pairs pair
                      WHERE pair.run_id = run.id)
               FROM public.jobs job
               JOIN public.owner_beta_simulation_runs run ON run.job_id = job.id
               JOIN public.job_attempts attempt
                 ON attempt.job_id = job.id AND attempt.attempt_no = job.attempt_count
              WHERE job.id = $1",
    )
    .bind(fixture.job_id)
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(
        final_state,
        (
            JobStatus::Failed,
            2,
            Some("SIMULATION_INPUT_INVALID".to_owned()),
            "FAILED".to_owned(),
            Some("SIMULATION_INPUT_INVALID".to_owned()),
            "FAILED".to_owned(),
            Some("SIMULATION_INPUT_INVALID".to_owned()),
            0,
        )
    );

    let valid_owner = new_owner(db, "r4-valid-api-reject").await?;
    let valid_fixture = seed_fixture(
        db,
        valid_owner,
        &role_pool(db, "app", None).await,
        "valid-api-reject",
    )
    .await?;
    let valid_claim = queue
        .claim_next_for(
            "r4-valid-api-reject-worker",
            OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE,
        )
        .await?
        .expect("valid simulation claim");
    assert_eq!(
        settle_invalid_simulation_claim(&queue, &valid_claim).await,
        Err(SimulationPublicationError::Integrity),
        "the invalid-input API cannot settle a valid payload"
    );
    let valid_unchanged: (JobStatus, String, String, i64) = sqlx::query_as(
        "SELECT job.status, run.status, attempt.outcome,
                (SELECT count(*) FROM public.owner_beta_simulation_result_pairs pair
                  WHERE pair.run_id = run.id)
           FROM public.jobs job
           JOIN public.owner_beta_simulation_runs run ON run.job_id = job.id
           JOIN public.job_attempts attempt
             ON attempt.job_id = job.id AND attempt.attempt_no = job.attempt_count
          WHERE job.id = $1",
    )
    .bind(valid_fixture.job_id)
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(
        valid_unchanged,
        (
            JobStatus::Running,
            "PENDING".to_owned(),
            "RUNNING".to_owned(),
            0
        )
    );

    let nil_owner = new_owner(db, "r4-nil-identity").await?;
    let nil_fixture = seed_nil_identity_fixture(
        db,
        nil_owner,
        &role_pool(db, "app", None).await,
        "nil-identity",
    )
    .await?;
    let nil_claim = queue
        .claim_next_for(
            "r4-nil-identity-worker",
            OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE,
        )
        .await?
        .expect("SQL-admitted nil identity payload claim");
    let nil_payload = serde_json::to_vec(&nil_claim.job.payload_json)?;
    assert!(
        SimulationJobInput::from_json(&nil_payload).is_ok(),
        "current WP-3A parser treats nil UUID fields as structurally valid"
    );
    assert_eq!(
        settle_invalid_simulation_claim(&queue, &nil_claim).await,
        Err(SimulationPublicationError::Integrity),
        "nil UUID payloads cannot be relabeled parser-invalid by this API"
    );
    let nil_unchanged: (JobStatus, String, String, i64) = sqlx::query_as(
        "SELECT job.status, run.status, attempt.outcome,
                (SELECT count(*) FROM public.owner_beta_simulation_result_pairs pair
                  WHERE pair.run_id = run.id)
           FROM public.jobs job
           JOIN public.owner_beta_simulation_runs run ON run.job_id = job.id
           JOIN public.job_attempts attempt
             ON attempt.job_id = job.id AND attempt.attempt_no = job.attempt_count
          WHERE job.id = $1",
    )
    .bind(nil_fixture.job_id)
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(
        nil_unchanged,
        (
            JobStatus::Running,
            "PENDING".to_owned(),
            "RUNNING".to_owned(),
            0,
        )
    );

    let cancel_owner = new_owner(db, "r4-invalid-cancel").await?;
    let cancel_fixture = seed_invalid_fixture(
        db,
        cancel_owner,
        &role_pool(db, "app", None).await,
        "invalid-cancel",
        "2026-08-23",
        "2026-08-28",
    )
    .await?;
    let cancel_claim = queue
        .claim_next_for(
            "r4-invalid-cancel-worker",
            OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE,
        )
        .await?
        .expect("invalid simulation cancellation claim");
    let app_cancel = role_pool(db, "app", Some(cancel_owner)).await;
    let audit_cancel = role_pool(db, "audit_writer", None).await;
    let cancel_queue = JobQueue::new(
        app_cancel,
        Some(audit_cancel),
        queue_config(Duration::from_secs(60)),
    );
    let mut actor = AuditActor::new("owner");
    actor.user_id = Some(cancel_owner);
    actor.correlation_id = Some(format!("r4-invalid-cancel-{}", Uuid::new_v4()));
    assert!(matches!(
        cancel_queue
            .request_cancel(cancel_fixture.job_id, &actor)
            .await?,
        job_queue::CancelResult::Canceled(_)
    ));
    assert_eq!(
        settle_invalid_simulation_claim(&queue, &cancel_claim).await?,
        SimulationPublicationOutcome::Canceled
    );
    assert_eq!(
        settle_invalid_simulation_claim(&queue, &cancel_claim).await?,
        SimulationPublicationOutcome::Canceled,
        "a fully mirrored cancellation remains idempotent"
    );
    let canceled_state: (
        JobStatus,
        String,
        Option<String>,
        String,
        Option<String>,
        i64,
    ) = sqlx::query_as(
        "SELECT job.status, run.status, run.error_code, attempt.outcome,
                    attempt.error_code,
                    (SELECT count(*) FROM public.owner_beta_simulation_result_pairs pair
                      WHERE pair.run_id = run.id)
               FROM public.jobs job
               JOIN public.owner_beta_simulation_runs run ON run.job_id = job.id
               JOIN public.job_attempts attempt
                 ON attempt.job_id = job.id AND attempt.attempt_no = job.attempt_count
              WHERE job.id = $1",
    )
    .bind(cancel_fixture.job_id)
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(
        canceled_state,
        (
            JobStatus::Canceled,
            "CANCELED".to_owned(),
            Some("CANCELED".to_owned()),
            "FAILED".to_owned(),
            Some("canceled".to_owned()),
            0,
        )
    );
    Ok(())
}

#[tokio::test]
async fn r4_invalid_recovery_is_terminal_and_continues_to_valid_jobs() {
    let Some(db) = require_scratch_db().await else {
        return;
    };
    r4_invalid_recovery_is_terminal_and_continues_to_valid_jobs_body(&db)
        .await
        .unwrap();
    db.drop_db().await;
}

async fn r4_invalid_recovery_is_terminal_and_continues_to_valid_jobs_body(
    db: &ScratchDb,
) -> TestResult {
    let calendar_owner = new_owner(db, "r4-invalid-calendar-recovery").await?;
    let calendar_fixture = seed_invalid_fixture(
        db,
        calendar_owner,
        &role_pool(db, "app", None).await,
        "calendar-recovery",
        "2026-08-23",
        "2026-08-28",
    )
    .await?;
    tokio::time::sleep(Duration::from_millis(10)).await;
    let warmup_owner = new_owner(db, "r4-invalid-warmup-recovery").await?;
    let warmup_fixture = seed_invalid_fixture(
        db,
        warmup_owner,
        &role_pool(db, "app", None).await,
        "warmup-recovery",
        "2016-08-29",
        "2016-08-31",
    )
    .await?;
    tokio::time::sleep(Duration::from_millis(10)).await;
    let valid_owner = new_owner(db, "r4-invalid-recovery-valid").await?;
    let valid_fixture = seed_fixture(
        db,
        valid_owner,
        &role_pool(db, "app", None).await,
        "recovery-valid-followup",
    )
    .await?;
    tokio::time::sleep(Duration::from_millis(10)).await;
    let queued_cancel_owner = new_owner(db, "r4-invalid-queued-cancel").await?;
    let queued_cancel_fixture = seed_invalid_fixture(
        db,
        queued_cancel_owner,
        &role_pool(db, "app", None).await,
        "queued-cancel",
        "2026-08-23",
        "2026-08-28",
    )
    .await?;
    tokio::time::sleep(Duration::from_millis(10)).await;
    let live_cancel_owner = new_owner(db, "r4-invalid-live-cancel").await?;
    let live_cancel_fixture = seed_invalid_fixture(
        db,
        live_cancel_owner,
        &role_pool(db, "app", None).await,
        "live-cancel",
        "2026-08-23",
        "2026-08-28",
    )
    .await?;

    let worker = role_pool(db, "worker", None).await;
    let queue = JobQueue::new(worker, None, queue_config(Duration::from_secs(60)));
    let queued_app = role_pool(db, "app", Some(queued_cancel_owner)).await;
    let queued_audit = role_pool(db, "audit_writer", None).await;
    let queued_cancel_queue = JobQueue::new(
        queued_app,
        Some(queued_audit),
        queue_config(Duration::from_secs(60)),
    );
    let mut queued_actor = AuditActor::new("owner");
    queued_actor.user_id = Some(queued_cancel_owner);
    queued_actor.correlation_id = Some(format!("r4-invalid-queued-cancel-{}", Uuid::new_v4()));
    assert!(matches!(
        queued_cancel_queue
            .request_cancel(queued_cancel_fixture.job_id, &queued_actor)
            .await?,
        job_queue::CancelResult::Canceled(_)
    ));

    for (worker_id, expected_job_id) in [
        ("r4-calendar-expired-worker", calendar_fixture.job_id),
        ("r4-warmup-expired-worker", warmup_fixture.job_id),
        ("r4-valid-followup-worker", valid_fixture.job_id),
    ] {
        let claim = queue
            .claim_next_for(worker_id, OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE)
            .await?
            .expect("the next exact simulation claim");
        assert_eq!(claim.job.id, expected_job_id);
    }
    let live_claim = queue
        .claim_next_for(
            "r4-live-cancel-worker",
            OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE,
        )
        .await?
        .expect("live cancellation claim");
    assert_eq!(live_claim.job.id, live_cancel_fixture.job_id);

    for job_id in [
        calendar_fixture.job_id,
        warmup_fixture.job_id,
        valid_fixture.job_id,
    ] {
        sqlx::query(
            "UPDATE public.jobs
                SET locked_at = pg_catalog.now() - interval '10 seconds'
              WHERE id = $1",
        )
        .bind(job_id)
        .execute(&db.pool)
        .await?;
    }
    let live_app = role_pool(db, "app", Some(live_cancel_owner)).await;
    let live_audit = role_pool(db, "audit_writer", None).await;
    let live_cancel_queue = JobQueue::new(
        live_app,
        Some(live_audit),
        queue_config(Duration::from_secs(60)),
    );
    let mut live_actor = AuditActor::new("owner");
    live_actor.user_id = Some(live_cancel_owner);
    live_actor.correlation_id = Some(format!("r4-invalid-live-cancel-{}", Uuid::new_v4()));
    assert!(matches!(
        live_cancel_queue
            .request_cancel(live_cancel_fixture.job_id, &live_actor)
            .await?,
        job_queue::CancelResult::Canceled(_)
    ));

    let first = recover_simulation_claims(&queue).await?;
    assert_eq!(first.jobs_checked, 4);
    assert_eq!(first.attempts_orphaned, 3);
    assert_eq!(first.jobs_failed, 2);
    assert_eq!(first.jobs_requeued, 1);
    assert_eq!(first.runs_canceled, 1);
    assert_eq!(first.runs_failed_healed, 0);
    assert_eq!(first.jobs_skipped, 0);

    for fixture in [&calendar_fixture, &warmup_fixture] {
        let failed: (
            JobStatus,
            i32,
            Option<String>,
            String,
            Option<String>,
            String,
            i64,
        ) = sqlx::query_as(
            "SELECT job.status, job.attempt_count, job.error_code,
                        run.status, run.error_code, attempt.outcome,
                        (SELECT count(*) FROM public.owner_beta_simulation_result_pairs pair
                          WHERE pair.run_id = run.id)
                   FROM public.jobs job
                   JOIN public.owner_beta_simulation_runs run ON run.job_id = job.id
                   JOIN public.job_attempts attempt
                     ON attempt.job_id = job.id AND attempt.attempt_no = job.attempt_count
                  WHERE job.id = $1",
        )
        .bind(fixture.job_id)
        .fetch_one(&db.pool)
        .await?;
        assert_eq!(
            failed,
            (
                JobStatus::Failed,
                1,
                Some("SIMULATION_INPUT_INVALID".to_owned()),
                "FAILED".to_owned(),
                Some("SIMULATION_INPUT_INVALID".to_owned()),
                "ORPHANED".to_owned(),
                0,
            )
        );
    }
    let valid_requeued: (JobStatus, i32, String, String, i64) = sqlx::query_as(
        "SELECT job.status, job.attempt_count, run.status, attempt.outcome,
                (SELECT count(*) FROM public.owner_beta_simulation_result_pairs pair
                  WHERE pair.run_id = run.id)
           FROM public.jobs job
           JOIN public.owner_beta_simulation_runs run ON run.job_id = job.id
           JOIN public.job_attempts attempt
             ON attempt.job_id = job.id AND attempt.attempt_no = job.attempt_count
          WHERE job.id = $1",
    )
    .bind(valid_fixture.job_id)
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(
        valid_requeued,
        (
            JobStatus::Queued,
            1,
            "PENDING".to_owned(),
            "ORPHANED".to_owned(),
            0,
        )
    );
    let queued_canceled: (JobStatus, String, i64) = sqlx::query_as(
        "SELECT job.status, run.status,
                (SELECT count(*) FROM public.job_attempts attempt
                  WHERE attempt.job_id = job.id)
           FROM public.jobs job
           JOIN public.owner_beta_simulation_runs run ON run.job_id = job.id
          WHERE job.id = $1",
    )
    .bind(queued_cancel_fixture.job_id)
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(
        queued_canceled,
        (JobStatus::Canceled, "CANCELED".to_owned(), 0)
    );
    let still_live: (JobStatus, String, String, i64) = sqlx::query_as(
        "SELECT job.status, run.status, attempt.outcome,
                (SELECT count(*) FROM public.owner_beta_simulation_result_pairs pair
                  WHERE pair.run_id = run.id)
           FROM public.jobs job
           JOIN public.owner_beta_simulation_runs run ON run.job_id = job.id
           JOIN public.job_attempts attempt
             ON attempt.job_id = job.id AND attempt.attempt_no = job.attempt_count
          WHERE job.id = $1",
    )
    .bind(live_cancel_fixture.job_id)
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(
        still_live,
        (
            JobStatus::Canceled,
            "PENDING".to_owned(),
            "RUNNING".to_owned(),
            0,
        )
    );

    sqlx::query(
        "UPDATE public.jobs
            SET locked_at = pg_catalog.now() - interval '10 seconds'
          WHERE id = $1",
    )
    .bind(live_cancel_fixture.job_id)
    .execute(&db.pool)
    .await?;
    let live_expired = recover_simulation_claims(&queue).await?;
    assert_eq!(
        (
            live_expired.jobs_checked,
            live_expired.runs_canceled,
            live_expired.attempts_orphaned,
        ),
        (1, 1, 1)
    );
    let canceled_attempt: (String, Option<String>) =
        sqlx::query_as("SELECT outcome, error_code FROM public.job_attempts WHERE id = $1")
            .bind(live_claim.attempt.id)
            .fetch_one(&db.pool)
            .await?;
    assert_eq!(canceled_attempt, ("ORPHANED".to_owned(), None));
    Ok(())
}

#[tokio::test]
async fn r4_frozen_config_edits_allow_failure_cancel_recovery_and_publication() {
    let Some(db) = require_scratch_db().await else {
        return;
    };
    r4_frozen_config_edits_allow_failure_cancel_recovery_and_publication_body(&db)
        .await
        .unwrap();
    db.drop_db().await;
}

async fn r4_frozen_config_edits_allow_failure_cancel_recovery_and_publication_body(
    db: &ScratchDb,
) -> TestResult {
    let worker = role_pool(db, "worker", None).await;
    let queue = JobQueue::new(worker, None, queue_config(Duration::from_secs(60)));

    let success_owner = new_owner(db, "r4-frozen-success").await?;
    let success_fixture = seed_fixture(
        db,
        success_owner,
        &role_pool(db, "app", None).await,
        "frozen-success",
    )
    .await?;
    mutate_config_after_enqueue(db, &success_fixture).await?;
    let config_after_edit: (serde_json::Value, bool) = sqlx::query_as(
        "SELECT config_json, is_active FROM public.user_strategy_configs WHERE id = $1",
    )
    .bind(success_fixture.input.strategy_config_id())
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(
        config_after_edit,
        (
            json!({
                "benchmark_instrument": "069500.KRX",
                "fast_ma": 40,
                "slow_ma": 200
            }),
            false,
        )
    );
    let frozen_snapshot: serde_json::Value = sqlx::query_scalar(
        "SELECT strategy_config_json FROM public.owner_beta_simulation_runs WHERE job_id = $1",
    )
    .bind(success_fixture.job_id)
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(
        frozen_snapshot,
        *success_fixture.input.strategy().config_json()
    );
    let success_claim = queue
        .claim_next_for(
            "r4-frozen-success-worker",
            OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE,
        )
        .await?
        .expect("frozen config success claim");
    let engine = synthetic_engine('g');
    let pair = synthetic_pair(&success_fixture, &engine);
    assert_eq!(
        publish_simulation_success(
            &queue,
            &success_claim,
            &success_fixture.input,
            &pair,
            &engine,
        )
        .await?,
        SimulationPublicationOutcome::Published,
        "success uses the immutable queued/run snapshot after config edit and deactivation"
    );
    let published_snapshot: (String, serde_json::Value, bool) = sqlx::query_as(
        "SELECT run.status, run.strategy_config_json, config.is_active
           FROM public.owner_beta_simulation_runs run
           JOIN public.user_strategy_configs config
             ON config.id = run.strategy_config_id
          WHERE run.job_id = $1",
    )
    .bind(success_fixture.job_id)
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(
        published_snapshot,
        (
            "SUCCEEDED".to_owned(),
            success_fixture.input.strategy().config_json().clone(),
            false,
        )
    );

    let failed_owner = new_owner(db, "r4-frozen-failure").await?;
    let failed_fixture = seed_fixture(
        db,
        failed_owner,
        &role_pool(db, "app", None).await,
        "frozen-failure",
    )
    .await?;
    mutate_config_after_enqueue(db, &failed_fixture).await?;
    let failed_claim = queue
        .claim_next_for(
            "r4-frozen-failure-worker",
            OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE,
        )
        .await?
        .expect("frozen config failure claim");
    assert_eq!(
        settle_simulation_failure(
            &queue,
            &failed_claim,
            &failed_fixture.input,
            SimulationPublicationFailure::ResultInvalid,
        )
        .await?,
        SimulationPublicationOutcome::Failed
    );
    let failed_run: (String, Option<String>, serde_json::Value) = sqlx::query_as(
        "SELECT status, error_code, strategy_config_json
           FROM public.owner_beta_simulation_runs WHERE job_id = $1",
    )
    .bind(failed_fixture.job_id)
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(
        failed_run,
        (
            "FAILED".to_owned(),
            Some("SIMULATION_RESULT_INVALID".to_owned()),
            failed_fixture.input.strategy().config_json().clone(),
        )
    );

    let cancel_owner = new_owner(db, "r4-frozen-cancel").await?;
    let cancel_fixture = seed_fixture(
        db,
        cancel_owner,
        &role_pool(db, "app", None).await,
        "frozen-cancel",
    )
    .await?;
    mutate_config_after_enqueue(db, &cancel_fixture).await?;
    let cancel_claim = queue
        .claim_next_for(
            "r4-frozen-cancel-worker",
            OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE,
        )
        .await?
        .expect("frozen config cancellation claim");
    let app_cancel = role_pool(db, "app", Some(cancel_owner)).await;
    let audit_cancel = role_pool(db, "audit_writer", None).await;
    let cancel_queue = JobQueue::new(
        app_cancel,
        Some(audit_cancel),
        queue_config(Duration::from_secs(60)),
    );
    let mut actor = AuditActor::new("owner");
    actor.user_id = Some(cancel_owner);
    actor.correlation_id = Some(format!("r4-frozen-cancel-{}", Uuid::new_v4()));
    assert!(matches!(
        cancel_queue
            .request_cancel(cancel_fixture.job_id, &actor)
            .await?,
        job_queue::CancelResult::Canceled(_)
    ));
    sqlx::query(
        "UPDATE public.jobs
            SET locked_at = pg_catalog.now() - interval '10 seconds'
          WHERE id = $1",
    )
    .bind(cancel_fixture.job_id)
    .execute(&db.pool)
    .await?;
    let recovery = recover_simulation_claims(&queue).await?;
    assert_eq!((recovery.runs_canceled, recovery.attempts_orphaned), (1, 1));
    let canceled_snapshot: (String, Option<String>, serde_json::Value, bool) = sqlx::query_as(
        "SELECT run.status, run.error_code, run.strategy_config_json, config.is_active
           FROM public.owner_beta_simulation_runs run
           JOIN public.user_strategy_configs config
             ON config.id = run.strategy_config_id
          WHERE run.job_id = $1",
    )
    .bind(cancel_fixture.job_id)
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(
        canceled_snapshot,
        (
            "CANCELED".to_owned(),
            Some("CANCELED".to_owned()),
            cancel_fixture.input.strategy().config_json().clone(),
            false,
        )
    );
    assert_eq!(cancel_claim.attempt.attempt_no, 1);
    Ok(())
}
