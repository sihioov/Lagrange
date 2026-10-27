//! Versioned runner/publication/recovery regression source.
//!
//! All result bodies are synthetic and pass the checked v2 result boundary.
//! These fixtures reconstruct only the fixed queue calendar; they never open
//! an approved artifact or claim actual factor readiness. DATABASE_URL absence
//! follows the existing scratch database skip convention and is not a pass.

mod common;

use std::{error::Error, time::Duration};

use chrono::NaiveDate;
use common::ScratchDb;
use domain::TradingDate;
use job_queue::{
    AuditActor, JobQueue, JobStatus, QueueConfig, SubmitJob,
    owner_beta::{
        OwnerBetaStrategySnapshot,
        simulation::{
            APPROVED_BARS_SHA256, CostProfile, EngineIdentity,
            v2::{
                APPROVED_CONTRACT_SHA256 as APPROVED_V2_CONTRACT_SHA256, V2Ledger, V2MarketSession,
                V2SimulationPair, V2SimulationResult, V2Strategy, V2StrategyIdentity,
                V2TargetAllocation,
            },
        },
        simulation_execution::{
            OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE, SimulationCommitState, SimulationJobInput,
            SimulationPublicationError, SimulationPublicationFailure, SimulationPublicationOutcome,
            V2SimulationJobInput, VersionedCheckedSimulationPair, VersionedSimulationJobInput,
            check_stored_versioned_simulation_pair, publish_versioned_simulation_success,
            reconcile_versioned_simulation_commit, recover_simulation_claims,
            settle_invalid_simulation_claim, settle_versioned_simulation_failure,
        },
    },
};
use market_data::{KR_ETF_CORE_SYMBOLS, range_normalize::ExpectedRangeSessions};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions, postgres::PgPoolOptions};
use uuid::Uuid;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const JOB_TYPE: &str = OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE;
const V1_SCHEMA: &str = "owner-beta-price-only-simulation-job-v1";
const V2_SCHEMA: &str = "owner-beta-price-only-simulation-job-v2";
const V2_PREFIX: &str = "owner-beta:price-only-simulation:v2:";
const APPROVED_START: &str = "2016-08-29";
const START: &str = "2026-08-27";
const END: &str = "2026-08-28";

struct Fixture {
    run_id: Uuid,
    config_id: Uuid,
    job_id: Uuid,
    idempotency_key: String,
    payload: Value,
    input: Option<V2SimulationJobInput>,
}

fn queue_config(lease: Duration) -> QueueConfig {
    QueueConfig {
        lease,
        backoff_base: Duration::from_millis(100),
    }
}

fn approved_sessions() -> Vec<TradingDate> {
    ExpectedRangeSessions::approved_xkrx(
        TradingDate::parse("2016-08-29").expect("approved start"),
        TradingDate::parse("2026-08-28").expect("approved end"),
    )
    .expect("fixed approved XKRX calendar")
    .sessions
}

fn fixed_window() -> (usize, usize) {
    let sessions = approved_sessions();
    let start = TradingDate::parse(START).expect("evaluation start");
    let end = TradingDate::parse(END).expect("evaluation end");
    (
        sessions
            .iter()
            .position(|date| *date == start)
            .expect("start in approved calendar"),
        sessions
            .iter()
            .position(|date| *date == end)
            .expect("end in approved calendar"),
    )
}

fn snapshot(strategy: V2Strategy) -> OwnerBetaStrategySnapshot {
    let config = serde_json::from_str(strategy.default_config_json()).expect("default JSON");
    OwnerBetaStrategySnapshot::from_resolved_config(&job_queue::resolver::ResolvedConfig {
        strategy_id: strategy.as_str().to_owned(),
        strategy_version: "1.0.0".to_owned(),
        config,
    })
    .expect("exact default snapshot")
}

fn v2_payload(
    strategy: V2Strategy,
    run_id: Uuid,
    config_id: Uuid,
    start: &str,
    end: &str,
) -> Value {
    json!({
        "schema": V2_SCHEMA,
        "run_id": run_id,
        "strategy_config_id": config_id,
        "evaluation_start": start,
        "evaluation_end": end,
        "pins": {
            "candidate_content_sha256": "sha256:b0fb82f6a580f3def13a1e1e34bea68e30d95dc720306e7ee54b6c3199cf402d",
            "artifact_manifest_sha256": "sha256:53587f32ce67ae1a8488b9c00096465185d03e35b1b38a04f39ed5462da7f6b6",
            "stage5_manifest_sha256": "sha256:5b2900594e03e322d4e81adaa79fc6c40ae70035213786972ed5cb7a22870d2a",
            "action_manifest_sha256": "sha256:080d38142a6506f741114eb75a77a36c23c2554a0d39eba8843ff62bfb484550",
            "approval_registry_sha256": "sha256:5d3aa2b354d8c0c51d0d7d029e9fd3f92e0570fe074262bfc900829a5a2bb707"
        },
        "contract_document_sha256": APPROVED_V2_CONTRACT_SHA256,
        "bars_sha256": APPROVED_BARS_SHA256,
        "strategy": snapshot(strategy)
    })
}

fn v1_input(run_id: Uuid, config_id: Uuid) -> SimulationJobInput {
    let payload = json!({
        "schema": V1_SCHEMA,
        "run_id": run_id,
        "strategy_config_id": config_id,
        "evaluation_start": START,
        "evaluation_end": END,
        "pins": {
            "candidate_content_sha256": "sha256:b0fb82f6a580f3def13a1e1e34bea68e30d95dc720306e7ee54b6c3199cf402d",
            "artifact_manifest_sha256": "sha256:53587f32ce67ae1a8488b9c00096465185d03e35b1b38a04f39ed5462da7f6b6",
            "stage5_manifest_sha256": "sha256:5b2900594e03e322d4e81adaa79fc6c40ae70035213786972ed5cb7a22870d2a",
            "action_manifest_sha256": "sha256:080d38142a6506f741114eb75a77a36c23c2554a0d39eba8843ff62bfb484550",
            "approval_registry_sha256": "sha256:5d3aa2b354d8c0c51d0d7d029e9fd3f92e0570fe074262bfc900829a5a2bb707"
        },
        "contract_document_sha256": job_queue::owner_beta::simulation::APPROVED_CONTRACT_SHA256,
        "bars_sha256": APPROVED_BARS_SHA256,
        "strategy": {
            "strategy_id": "trend_following",
            "strategy_version": "1.0.0",
            "config_json": {"benchmark_instrument":"069500.KRX","fast_ma":50,"slow_ma":200},
            "config_sha256": "sha256:90e5b4c6972678e9186d4655d76496676e7cfb8dbe473a249f062963873c04a0"
        }
    });
    SimulationJobInput::from_json(&serde_json::to_vec(&payload).expect("v1 fixture JSON"))
        .expect("fixed v1 input remains accepted")
}

async fn require_scratch_db() -> Option<ScratchDb> {
    let db = ScratchDb::create().await;
    if db.is_none() {
        eprintln!("SKIP: DATABASE_URL not set; this is not a database test pass");
    }
    db
}

async fn new_owner(db: &ScratchDb, label: &str) -> TestResult<Uuid> {
    let id = sqlx::query_scalar(
        "INSERT INTO public.users (issuer, subject, email)
         VALUES ('r4-v2-execution.test', $1, $2) RETURNING id",
    )
    .bind(format!("{label}-{}", Uuid::new_v4()))
    .bind(format!("{label}-{}@example.test", Uuid::new_v4()))
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
        .bind(actor.to_string())
        .fetch_one(&pool)
        .await
        .unwrap_or_else(|_| panic!("set test actor for {role}"));
    }
    pool
}

async fn insert_strategy_config(
    db: &ScratchDb,
    owner: Uuid,
    strategy: V2Strategy,
) -> TestResult<Uuid> {
    sqlx::query(
        "INSERT INTO public.strategies (id, display_name, state)
         VALUES ($1, $1, 'Validated') ON CONFLICT (id) DO NOTHING",
    )
    .bind(strategy.as_str())
    .execute(&db.pool)
    .await?;
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO public.user_strategy_configs
            (id, owner_user_id, strategy_id, strategy_version, config_json, is_active)
         VALUES ($1, $2, $3, '1.0.0', $4, true)",
    )
    .bind(id)
    .bind(owner)
    .bind(strategy.as_str())
    .bind(serde_json::from_str::<Value>(
        strategy.default_config_json(),
    )?)
    .execute(&db.pool)
    .await?;
    Ok(id)
}

async fn set_actor(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    owner: Uuid,
) -> TestResult {
    sqlx::query_scalar::<_, String>("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
        .bind(owner.to_string())
        .fetch_one(&mut **transaction)
        .await?;
    Ok(())
}

async fn insert_v2_fixture(
    db: &ScratchDb,
    app: &PgPool,
    owner: Uuid,
    strategy: V2Strategy,
    start_index: usize,
    end_index: usize,
    entitlement_window: (&str, &str),
) -> TestResult<Fixture> {
    let (entitlement_start, entitlement_end) = entitlement_window;
    let sessions = approved_sessions();
    let run_id = Uuid::new_v4();
    let config_id = insert_strategy_config(db, owner, strategy).await?;
    let start = sessions[start_index].to_iso();
    let end = sessions[end_index].to_iso();
    let payload = v2_payload(strategy, run_id, config_id, &start, &end);
    let input = V2SimulationJobInput::from_json(&serde_json::to_vec(&payload)?).ok();
    let session_count = i32::try_from(
        input
            .as_ref()
            .map_or(end_index + 1, |input| input.required_dates().len()),
    )?;
    let config: Value = serde_json::from_str(strategy.default_config_json())?;
    let strategy_snapshot = snapshot(strategy);
    let idempotency_key = format!("{V2_PREFIX}{}", Uuid::new_v4());

    let mut transaction = app.begin().await?;
    set_actor(&mut transaction, owner).await?;
    let job_id: Uuid = sqlx::query_scalar(
        "INSERT INTO public.jobs
            (owner_user_id, job_type, status, priority, idempotency_key,
             payload_json, max_attempts)
         VALUES ($1, $2, 'QUEUED', 10, $3, $4, 3) RETURNING id",
    )
    .bind(owner)
    .bind(JOB_TYPE)
    .bind(&idempotency_key)
    .bind(&payload)
    .fetch_one(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT INTO public.owner_beta_simulation_runs
            (id, owner_user_id, strategy_config_id, job_id,
             strategy_id, strategy_version, strategy_config_json, strategy_config_sha256,
             evaluation_start, evaluation_end, session_count,
             candidate_content_sha256, artifact_manifest_sha256, stage5_manifest_sha256,
             action_manifest_sha256, approval_registry_sha256,
             contract_document_sha256, bars_sha256)
         VALUES ($1, $2, $3, $4, $5, '1.0.0', $6, $7, $8, $9, $10,
                 $11, $12, $13, $14, $15, $16, $17)",
    )
    .bind(run_id)
    .bind(owner)
    .bind(config_id)
    .bind(job_id)
    .bind(strategy.as_str())
    .bind(config)
    .bind(strategy_snapshot.config_sha256().as_str())
    .bind(NaiveDate::parse_from_str(&start, "%Y-%m-%d")?)
    .bind(NaiveDate::parse_from_str(&end, "%Y-%m-%d")?)
    .bind(session_count)
    .bind("sha256:b0fb82f6a580f3def13a1e1e34bea68e30d95dc720306e7ee54b6c3199cf402d")
    .bind("sha256:53587f32ce67ae1a8488b9c00096465185d03e35b1b38a04f39ed5462da7f6b6")
    .bind("sha256:5b2900594e03e322d4e81adaa79fc6c40ae70035213786972ed5cb7a22870d2a")
    .bind("sha256:080d38142a6506f741114eb75a77a36c23c2554a0d39eba8843ff62bfb484550")
    .bind("sha256:5d3aa2b354d8c0c51d0d7d029e9fd3f92e0570fe074262bfc900829a5a2bb707")
    .bind(APPROVED_V2_CONTRACT_SHA256)
    .bind(APPROVED_BARS_SHA256)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    sqlx::query(
        "INSERT INTO public.data_entitlements
            (contract_document_sha256, contract_reference, status,
             covered_datasets, covered_uses, effective_from, effective_until, managed_by)
         VALUES (repeat('e', 64), $1, 'ACTIVE', '[\"krx_eod_bars\"]'::jsonb,
                 '[\"backtest\"]'::jsonb, $2, $3, $4)",
    )
    .bind(format!("fixture://r4-v2-execution-{}", Uuid::new_v4()))
    .bind(NaiveDate::parse_from_str(entitlement_start, "%Y-%m-%d")?)
    .bind(NaiveDate::parse_from_str(entitlement_end, "%Y-%m-%d")?)
    .bind(owner)
    .execute(&db.pool)
    .await?;

    Ok(Fixture {
        run_id,
        config_id,
        job_id,
        idempotency_key,
        payload,
        input,
    })
}

fn engine(marker: char) -> EngineIdentity {
    EngineIdentity::new(
        &marker.to_string().repeat(40),
        &format!("sha256:{}", marker.to_string().repeat(64)),
        "rustc-source-fixture",
        "x86_64-unknown-linux-gnu",
        "test",
    )
    .expect("synthetic declared engine")
}

fn checked_v2_pair(
    input: &V2SimulationJobInput,
    expected_engine: &EngineIdentity,
) -> (VersionedSimulationJobInput, VersionedCheckedSimulationPair) {
    assert_eq!(input.strategy().strategy_id(), "buy_and_hold");
    let schedule = input.schedule().clone();
    let strategy = V2StrategyIdentity::from_default_snapshot(
        "buy_and_hold",
        "1.0.0",
        V2Strategy::BuyAndHold.default_config_json(),
        input.strategy().config_sha256().as_str(),
        Vec::new(),
    )
    .expect("checked frozen buy-and-hold identity");
    let mut weights = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| {
            let instrument = format!("{symbol}.KRX");
            let weight = u32::from(instrument == "069500.KRX") * 1_000_000;
            (instrument, weight)
        })
        .collect::<Vec<_>>();
    weights.sort_by(|left, right| left.0.cmp(&right.0));
    let target = V2TargetAllocation::new(&format!("sha256:{}", "c".repeat(64)), weights, 0)
        .expect("synthetic checked target");

    let ledger_for = |profile| {
        let mut ledger = V2Ledger::new(profile).expect("fixed production v2 capital");
        for date in schedule.evaluation_sessions() {
            let mut prices = KR_ETF_CORE_SYMBOLS
                .iter()
                .map(|symbol| {
                    (
                        format!("{symbol}.KRX"),
                        "100.0000".to_owned(),
                        "101.0000".to_owned(),
                        1_000_000,
                    )
                })
                .collect::<Vec<_>>();
            prices.sort_by(|left, right| left.0.cmp(&right.0));
            let session = V2MarketSession::new(&date.to_iso(), prices)
                .expect("complete synthetic ETF11 session");
            ledger
                .open_session(session)
                .expect("open synthetic session");
            if let Some(decision) = schedule.decision_for_execution(*date) {
                ledger
                    .rebalance(decision, target.clone())
                    .expect("scheduled synthetic allocation");
            }
            ledger.close_session().expect("close synthetic session");
        }
        ledger.finish(&schedule).expect("checked synthetic ledger")
    };
    let illustrative_ledger = ledger_for(CostProfile::Illustrative10BpPerSideV1);
    let zero_cost_ledger = ledger_for(CostProfile::ZeroCostReferenceV1);
    let illustrative = V2SimulationResult::new(
        strategy.clone(),
        expected_engine.clone(),
        schedule.clone(),
        CostProfile::Illustrative10BpPerSideV1,
        illustrative_ledger.clone(),
        illustrative_ledger,
    )
    .expect("checked illustrative result");
    let zero_cost = V2SimulationResult::new(
        strategy,
        expected_engine.clone(),
        schedule,
        CostProfile::ZeroCostReferenceV1,
        zero_cost_ledger.clone(),
        zero_cost_ledger,
    )
    .expect("checked zero-cost result");
    let pair = V2SimulationPair::new(illustrative, zero_cost).expect("complete v2 pair");
    let (illustrative_bytes, zero_cost_bytes) = pair.canonical_json().expect("canonical bodies");
    let versioned_input = VersionedSimulationJobInput::V2(input.clone());
    let checked = VersionedCheckedSimulationPair::from_canonical(
        &illustrative_bytes,
        &zero_cost_bytes,
        &versioned_input,
        expected_engine,
    )
    .expect("checked versioned pair");
    (versioned_input, checked)
}

async fn claim_fixture(
    queue: &JobQueue,
    fixture: &Fixture,
    worker_id: &str,
) -> TestResult<job_queue::types::ClaimedJob> {
    let claim = queue
        .claim_next_for(worker_id, JOB_TYPE)
        .await?
        .expect("versioned simulation claim");
    assert_eq!(claim.job.id, fixture.job_id);
    Ok(claim)
}

async fn pair_count(db: &ScratchDb, run_id: Uuid) -> TestResult<i64> {
    Ok(sqlx::query_scalar(
        "SELECT count(*) FROM public.owner_beta_simulation_result_pairs WHERE run_id = $1",
    )
    .bind(run_id)
    .fetch_one(&db.pool)
    .await?)
}

async fn expire_job(db: &ScratchDb, job_id: Uuid) -> TestResult {
    sqlx::query(
        "UPDATE public.jobs SET locked_at = pg_catalog.now() - interval '2 minutes' WHERE id = $1",
    )
    .bind(job_id)
    .execute(&db.pool)
    .await?;
    Ok(())
}

#[test]
fn all_five_exact_defaults_have_strict_versioned_queue_bindings() {
    let (start_index, end_index) = fixed_window();
    let sessions = approved_sessions();
    for (index, strategy) in V2Strategy::ALL.into_iter().enumerate() {
        let payload = v2_payload(
            strategy,
            Uuid::from_u128(index as u128 + 1),
            Uuid::from_u128(index as u128 + 101),
            &sessions[start_index].to_iso(),
            &sessions[end_index].to_iso(),
        );
        let bytes = serde_json::to_vec(&payload).expect("queue fixture serializes");
        let checked = VersionedSimulationJobInput::from_json(&bytes)
            .expect("one of the five frozen defaults binds to v2");
        assert_eq!(checked.schema(), V2_SCHEMA);
        assert_eq!(checked.strategy().strategy_id(), strategy.as_str());
        assert!(checked.required_dates().len() >= 202);

        let mut unknown = payload.clone();
        unknown["schema"] = json!("owner-beta-price-only-simulation-v99");
        assert!(
            VersionedSimulationJobInput::from_json(
                &serde_json::to_vec(&unknown).expect("unknown schema fixture")
            )
            .is_err()
        );
        let mut mixed = payload;
        mixed["contract_document_sha256"] =
            json!(job_queue::owner_beta::simulation::APPROVED_CONTRACT_SHA256);
        assert!(
            VersionedSimulationJobInput::from_json(
                &serde_json::to_vec(&mixed).expect("mixed contract fixture")
            )
            .is_err()
        );
    }
}

#[tokio::test]
async fn v2_publication_stores_both_original_members_and_reconciles_the_exact_pair() -> TestResult {
    let Some(db) = require_scratch_db().await else {
        return Ok(());
    };
    let result =
        v2_publication_stores_both_original_members_and_reconciles_the_exact_pair_body(&db).await;
    db.drop_db().await;
    result
}

async fn v2_publication_stores_both_original_members_and_reconciles_the_exact_pair_body(
    db: &ScratchDb,
) -> TestResult {
    let owner = new_owner(db, "v2-publish").await?;
    let app = role_pool(db, "app", Some(owner)).await;
    let (start_index, end_index) = fixed_window();
    let fixture = insert_v2_fixture(
        db,
        &app,
        owner,
        V2Strategy::BuyAndHold,
        start_index,
        end_index,
        (APPROVED_START, END),
    )
    .await?;
    let app_queue = JobQueue::new(app.clone(), None, queue_config(Duration::from_secs(60)));
    let duplicate = app_queue
        .submit(SubmitJob {
            owner_user_id: owner,
            job_type: JOB_TYPE.to_owned(),
            payload: fixture.payload.clone(),
            priority: 10,
            idempotency_key: Some(fixture.idempotency_key.clone()),
            max_attempts: 3,
            available_at: None,
        })
        .await?;
    assert_eq!(
        duplicate.id, fixture.job_id,
        "same key replays the frozen job"
    );

    // The worker must use the immutable queued snapshot after mutable config changes.
    let changed = sqlx::query(
        "UPDATE public.user_strategy_configs
            SET config_json = '{\"mutated_after_enqueue\":true}'::jsonb, is_active = false
          WHERE id = $1 AND owner_user_id = $2",
    )
    .bind(fixture.config_id)
    .bind(owner)
    .execute(&db.pool)
    .await?;
    assert_eq!(changed.rows_affected(), 1);

    let worker = role_pool(db, "worker", None).await;
    let queue = JobQueue::new(worker, None, queue_config(Duration::from_secs(60)));
    let claim = claim_fixture(&queue, &fixture, "v2-publish-worker").await?;
    let input = fixture.input.as_ref().expect("valid checked v2 input");
    let (versioned_input, pair) = checked_v2_pair(input, &engine('a'));

    let v1_mismatch = VersionedSimulationJobInput::V1(v1_input(fixture.run_id, fixture.config_id));
    assert!(matches!(
        settle_versioned_simulation_failure(
            &queue,
            &claim,
            &v1_mismatch,
            SimulationPublicationFailure::InputInvalid,
        )
        .await,
        Err(SimulationPublicationError::QueueIntegrity)
    ));
    assert!(matches!(
        publish_versioned_simulation_success(&queue, &claim, &versioned_input, &pair, &engine('b'))
            .await,
        Err(SimulationPublicationError::Integrity)
    ));
    assert_eq!(pair_count(db, fixture.run_id).await?, 0);

    let illustrative = pair.illustrative_bytes();
    assert!(
        VersionedCheckedSimulationPair::from_canonical(
            illustrative,
            illustrative,
            &versioned_input,
            &engine('a'),
        )
        .is_err(),
        "a duplicate profile cannot form a two-member pair"
    );
    assert!(
        check_stored_versioned_simulation_pair(
            pair.illustrative_bytes(),
            "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            pair.zero_cost_bytes(),
            pair.zero_cost_sha256().as_str(),
            &versioned_input,
            &engine('a'),
        )
        .is_err(),
        "both stored byte hashes are required"
    );

    assert_eq!(
        reconcile_versioned_simulation_commit(
            &queue,
            &claim,
            &versioned_input,
            &pair,
            &engine('a')
        )
        .await?,
        SimulationCommitState::NotPublished
    );
    assert_eq!(
        publish_versioned_simulation_success(&queue, &claim, &versioned_input, &pair, &engine('a'))
            .await?,
        SimulationPublicationOutcome::Published
    );
    let stored: (Vec<u8>, String, Vec<u8>, String) = sqlx::query_as(
        "SELECT illustrative_canonical, illustrative_sha256,
                zero_cost_canonical, zero_cost_sha256
           FROM public.owner_beta_simulation_result_pairs WHERE run_id = $1",
    )
    .bind(fixture.run_id)
    .fetch_one(&db.pool)
    .await?;
    assert!(!stored.0.is_empty() && !stored.2.is_empty());
    let checked = check_stored_versioned_simulation_pair(
        &stored.0,
        &stored.1,
        &stored.2,
        &stored.3,
        &versioned_input,
        &engine('a'),
    )?;
    assert_eq!(checked.illustrative_bytes(), pair.illustrative_bytes());
    assert_eq!(checked.zero_cost_bytes(), pair.zero_cost_bytes());
    assert_eq!(pair_count(db, fixture.run_id).await?, 1);
    assert_eq!(
        reconcile_versioned_simulation_commit(
            &queue,
            &claim,
            &versioned_input,
            &pair,
            &engine('a')
        )
        .await?,
        SimulationCommitState::Published
    );
    Ok(())
}

#[tokio::test]
async fn v2_failures_cancellation_lease_loss_expiry_and_recovery_never_publish_partial_pairs()
-> TestResult {
    let Some(db) = require_scratch_db().await else {
        return Ok(());
    };
    let result =
        v2_failures_cancellation_lease_loss_expiry_and_recovery_never_publish_partial_pairs_body(
            &db,
        )
        .await;
    db.drop_db().await;
    result
}

async fn v2_failures_cancellation_lease_loss_expiry_and_recovery_never_publish_partial_pairs_body(
    db: &ScratchDb,
) -> TestResult {
    let (start_index, end_index) = fixed_window();
    // Lease expiry is driven explicitly by expire_job, not test wall-clock speed.
    let lease = Duration::from_secs(60);
    let worker = role_pool(db, "worker", None).await;
    let queue = JobQueue::new(worker, None, queue_config(lease));

    let failed_owner = new_owner(db, "v2-failed").await?;
    let failed_app = role_pool(db, "app", Some(failed_owner)).await;
    let failed = insert_v2_fixture(
        db,
        &failed_app,
        failed_owner,
        V2Strategy::BuyAndHold,
        start_index,
        end_index,
        (APPROVED_START, END),
    )
    .await?;
    let failed_claim = claim_fixture(&queue, &failed, "v2-failed-worker").await?;
    let failed_input =
        VersionedSimulationJobInput::V2(failed.input.clone().expect("valid v2 input"));
    assert_eq!(
        settle_versioned_simulation_failure(
            &queue,
            &failed_claim,
            &failed_input,
            SimulationPublicationFailure::ResultInvalid,
        )
        .await?,
        SimulationPublicationOutcome::Failed
    );
    assert_eq!(pair_count(db, failed.run_id).await?, 0);

    let recovery_owner = new_owner(db, "v2-recovery").await?;
    let recovery_app = role_pool(db, "app", Some(recovery_owner)).await;
    let recovery = insert_v2_fixture(
        db,
        &recovery_app,
        recovery_owner,
        V2Strategy::BuyAndHold,
        start_index,
        end_index,
        (APPROVED_START, END),
    )
    .await?;
    let expired_claim = claim_fixture(&queue, &recovery, "v2-expired-worker").await?;
    expire_job(db, recovery.job_id).await?;
    let recovered = recover_simulation_claims(&queue).await?;
    assert_eq!(recovered.jobs_requeued, 1);
    assert_eq!(pair_count(db, recovery.run_id).await?, 0);
    tokio::time::sleep(Duration::from_millis(150)).await;
    let new_claim = claim_fixture(&queue, &recovery, "v2-reclaimed-worker").await?;
    assert!(matches!(
        settle_versioned_simulation_failure(
            &queue,
            &expired_claim,
            &VersionedSimulationJobInput::V2(recovery.input.clone().expect("valid v2 input"),),
            SimulationPublicationFailure::InputInvalid,
        )
        .await,
        Err(SimulationPublicationError::QueueClaimLost)
    ));
    assert_eq!(pair_count(db, recovery.run_id).await?, 0);
    settle_versioned_simulation_failure(
        &queue,
        &new_claim,
        &VersionedSimulationJobInput::V2(recovery.input.clone().expect("valid v2 input")),
        SimulationPublicationFailure::InputInvalid,
    )
    .await?;

    let canceled_owner = new_owner(db, "v2-canceled").await?;
    let canceled_app = role_pool(db, "app", Some(canceled_owner)).await;
    let canceled = insert_v2_fixture(
        db,
        &canceled_app,
        canceled_owner,
        V2Strategy::BuyAndHold,
        start_index,
        end_index,
        (APPROVED_START, END),
    )
    .await?;
    let canceled_claim = claim_fixture(&queue, &canceled, "v2-cancel-worker").await?;
    let cancel_pool = role_pool(db, "app", Some(canceled_owner)).await;
    let audit_pool = role_pool(db, "audit_writer", None).await;
    let cancel_queue = JobQueue::new(cancel_pool, Some(audit_pool), queue_config(lease));
    let mut actor = AuditActor::new("owner");
    actor.user_id = Some(canceled_owner);
    actor.correlation_id = Some(format!("v2-cancel-{}", Uuid::new_v4()));
    cancel_queue.request_cancel(canceled.job_id, &actor).await?;
    expire_job(db, canceled.job_id).await?;
    let cancellation_recovery = recover_simulation_claims(&queue).await?;
    assert_eq!(cancellation_recovery.runs_canceled, 1);
    let (cancel_input, cancel_pair) = checked_v2_pair(
        canceled.input.as_ref().expect("valid v2 input"),
        &engine('d'),
    );
    assert_eq!(
        publish_versioned_simulation_success(
            &queue,
            &canceled_claim,
            &cancel_input,
            &cancel_pair,
            &engine('d'),
        )
        .await?,
        SimulationPublicationOutcome::Canceled
    );
    assert_eq!(pair_count(db, canceled.run_id).await?, 0);

    // Entitlements cover a global union of dates; managed_by is not an owner filter.
    // Remove earlier fixtures' grants from that union before testing denial.
    sqlx::query("UPDATE public.data_entitlements SET status = 'REVOKED' WHERE status = 'ACTIVE'")
        .execute(&db.pool)
        .await?;
    let entitlement_owner = new_owner(db, "v2-entitlement").await?;
    let entitlement_app = role_pool(db, "app", Some(entitlement_owner)).await;
    let entitlement = insert_v2_fixture(
        db,
        &entitlement_app,
        entitlement_owner,
        V2Strategy::BuyAndHold,
        start_index,
        end_index,
        (START, END),
    )
    .await?;
    let entitlement_claim = claim_fixture(&queue, &entitlement, "v2-entitlement-worker").await?;
    let (entitlement_input, entitlement_pair) = checked_v2_pair(
        entitlement.input.as_ref().expect("valid v2 input"),
        &engine('e'),
    );
    let entitlement_required_dates = entitlement_input.required_dates();
    assert!(
        entitlement_required_dates[0] < TradingDate::parse(START).expect("evaluation start"),
        "v2 entitlement must begin before evaluation to cover preparation"
    );
    assert!(matches!(
        publish_versioned_simulation_success(
            &queue,
            &entitlement_claim,
            &entitlement_input,
            &entitlement_pair,
            &engine('e'),
        )
        .await,
        Err(SimulationPublicationError::EntitlementDenied)
    ));
    assert_eq!(pair_count(db, entitlement.run_id).await?, 0);
    assert_eq!(
        settle_versioned_simulation_failure(
            &queue,
            &entitlement_claim,
            &entitlement_input,
            SimulationPublicationFailure::EntitlementDenied,
        )
        .await?,
        SimulationPublicationOutcome::Failed
    );
    assert_eq!(pair_count(db, entitlement.run_id).await?, 0);

    // Entitlements cover a global union of dates; managed_by is not an owner filter.
    // Remove earlier fixtures' grants from that union before testing denial.
    sqlx::query("UPDATE public.data_entitlements SET status = 'REVOKED' WHERE status = 'ACTIVE'")
        .execute(&db.pool)
        .await?;
    let expired_owner = new_owner(db, "v2-expired-entitlement").await?;
    let expired_app = role_pool(db, "app", Some(expired_owner)).await;
    let expired = insert_v2_fixture(
        db,
        &expired_app,
        expired_owner,
        V2Strategy::BuyAndHold,
        start_index,
        end_index,
        (APPROVED_START, "2020-01-01"),
    )
    .await?;
    let expired_claim = claim_fixture(&queue, &expired, "v2-expired-entitlement-worker").await?;
    let (expired_input, expired_pair) = checked_v2_pair(
        expired.input.as_ref().expect("valid v2 input"),
        &engine('f'),
    );
    assert!(matches!(
        publish_versioned_simulation_success(
            &queue,
            &expired_claim,
            &expired_input,
            &expired_pair,
            &engine('f'),
        )
        .await,
        Err(SimulationPublicationError::EntitlementDenied)
    ));
    assert_eq!(pair_count(db, expired.run_id).await?, 0);
    assert_eq!(
        settle_versioned_simulation_failure(
            &queue,
            &expired_claim,
            &expired_input,
            SimulationPublicationFailure::EntitlementDenied,
        )
        .await?,
        SimulationPublicationOutcome::Failed
    );
    assert_eq!(pair_count(db, expired.run_id).await?, 0);
    Ok(())
}

#[tokio::test]
async fn v2_invalid_claim_settlement_requires_exact_supported_identity_and_live_claim() -> TestResult
{
    let Some(db) = require_scratch_db().await else {
        return Ok(());
    };
    let result =
        v2_invalid_claim_settlement_requires_exact_supported_identity_and_live_claim_body(&db)
            .await;
    db.drop_db().await;
    result
}

async fn v2_invalid_claim_settlement_requires_exact_supported_identity_and_live_claim_body(
    db: &ScratchDb,
) -> TestResult {
    let owner = new_owner(db, "v2-invalid").await?;
    let app = role_pool(db, "app", Some(owner)).await;
    let invalid = insert_v2_fixture(
        db,
        &app,
        owner,
        V2Strategy::BuyAndHold,
        199,
        201,
        (APPROVED_START, END),
    )
    .await?;
    assert!(
        invalid.input.is_none(),
        "wire calendar alone rejects early start"
    );
    let worker = role_pool(db, "worker", None).await;
    let queue = JobQueue::new(worker, None, queue_config(Duration::from_secs(60)));
    let claim = claim_fixture(&queue, &invalid, "v2-invalid-worker").await?;
    assert_eq!(
        settle_invalid_simulation_claim(&queue, &claim).await?,
        SimulationPublicationOutcome::Failed
    );
    assert_eq!(pair_count(db, invalid.run_id).await?, 0);
    let status: (JobStatus, String) = sqlx::query_as(
        "SELECT job.status, run.status
           FROM public.jobs AS job
           JOIN public.owner_beta_simulation_runs AS run ON run.job_id = job.id
          WHERE job.id = $1",
    )
    .bind(invalid.job_id)
    .fetch_one(&db.pool)
    .await?;
    assert_eq!(status, (JobStatus::Failed, "FAILED".to_owned()));

    let valid_owner = new_owner(db, "v2-valid-invalid-settle").await?;
    let valid_app = role_pool(db, "app", Some(valid_owner)).await;
    let (start_index, end_index) = fixed_window();
    let valid = insert_v2_fixture(
        db,
        &valid_app,
        valid_owner,
        V2Strategy::BuyAndHold,
        start_index,
        end_index,
        (APPROVED_START, END),
    )
    .await?;
    let valid_claim = claim_fixture(&queue, &valid, "v2-valid-invalid-worker").await?;
    assert!(
        matches!(
            settle_invalid_simulation_claim(&queue, &valid_claim).await,
            Err(SimulationPublicationError::Integrity)
        ),
        "valid v2 input is never accepted by invalid-claim settlement"
    );
    let mut foreign_claim = valid_claim.clone();
    foreign_claim.job.owner_user_id = Uuid::new_v4();
    assert!(
        settle_invalid_simulation_claim(&queue, &foreign_claim)
            .await
            .is_err()
    );
    assert_eq!(pair_count(db, valid.run_id).await?, 0);
    Ok(())
}
