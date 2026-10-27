//! PostgreSQL regression source for the additive 0064 v2 binding migration.
//!
//! It uses synthetic fixed inputs and checked result constructors only. It
//! never opens the approved artifact. DATABASE_URL absence is a skip, not a
//! database pass; this source package deliberately does not execute it.

mod common;

use std::{error::Error, time::Duration};

use chrono::NaiveDate;
use common::ScratchDb;
use job_queue::owner_beta::{
    simulation::{
        APPROVED_BARS_SHA256, APPROVED_CONTRACT_SHA256, CostProfile, EngineIdentity,
        EvaluationPriceRow, Price4, SignalSnapshot, SimulationResult, StrategyIdentity,
        simulate_ledgers,
        v2::{
            V2Ledger, V2MarketSession, V2SimulationPair, V2SimulationResult, V2StrategyIdentity,
            V2TargetAllocation,
        },
    },
    simulation_execution::{
        CheckedSimulationPair, SimulationJobInput,
        v2::{V2CheckedSimulationPair, V2SimulationJobInput},
    },
};
use market_data::KR_ETF_CORE_SYMBOLS;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, postgres::PgConnectOptions, postgres::PgPoolOptions};
use uuid::Uuid;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const JOB_TYPE: &str = "owner_beta_price_only_simulation";
const V1_SCHEMA: &str = "owner-beta-price-only-simulation-job-v1";
const V2_SCHEMA: &str = "owner-beta-price-only-simulation-job-v2";
const V2_CONTRACT_SHA256: &str =
    "sha256:20dabf650deaff751d8d34a7fea98773e6843b879ed20e6002475fc70c243ee3";
const V1_PREFIX: &str = "owner-beta:price-only-simulation:v1:";
const V2_PREFIX: &str = "owner-beta:price-only-simulation:v2:";
const CANDIDATE_SHA256: &str =
    "sha256:b0fb82f6a580f3def13a1e1e34bea68e30d95dc720306e7ee54b6c3199cf402d";
const ARTIFACT_SHA256: &str =
    "sha256:53587f32ce67ae1a8488b9c00096465185d03e35b1b38a04f39ed5462da7f6b6";
const STAGE5_SHA256: &str =
    "sha256:5b2900594e03e322d4e81adaa79fc6c40ae70035213786972ed5cb7a22870d2a";
const ACTION_SHA256: &str =
    "sha256:080d38142a6506f741114eb75a77a36c23c2554a0d39eba8843ff62bfb484550";
const APPROVAL_SHA256: &str =
    "sha256:5d3aa2b354d8c0c51d0d7d029e9fd3f92e0570fe074262bfc900829a5a2bb707";
const RUN_START: &str = "2026-08-27";
const RUN_END: &str = "2026-08-28";

struct StrategyCase {
    id: &'static str,
    config_json: &'static str,
    config_sha256: &'static str,
}

const V2_CASES: [StrategyCase; 5] = [
    StrategyCase {
        id: "buy_and_hold",
        config_json: r#"{"benchmark_instrument":"069500.KRX","rebalance_cadence":"none","target_weight":1.0}"#,
        config_sha256: "sha256:4de76616ae1d72646bd695d0e5aa7df7d6544c2eef5bef7ca949536c1801fc06",
    },
    StrategyCase {
        id: "trend_following",
        config_json: r#"{"benchmark_instrument":"069500.KRX","fast_ma":50,"slow_ma":200}"#,
        config_sha256: "sha256:90e5b4c6972678e9186d4655d76496676e7cfb8dbe473a249f062963873c04a0",
    },
    StrategyCase {
        id: "relative_momentum",
        config_json: r#"{"lookback_months":12,"top_n":3}"#,
        config_sha256: "sha256:edace2510fb491084b142615a709f8ef689a5b207d7bae25ce7a886afceb8d07",
    },
    StrategyCase {
        id: "dual_momentum",
        config_json: r#"{"absolute_threshold":0.0,"lookback_months":12}"#,
        config_sha256: "sha256:ac6a1ac393e02952fe2db6308e5274436f766479f3e631ebe42e429a677426a6",
    },
    StrategyCase {
        id: "inverse_volatility",
        config_json: r#"{"max_weight":0.3,"vol_window":60}"#,
        config_sha256: "sha256:6252f27903f5ea0a7755ea7fc6c05a935518a2dbe44edf371a1e71f127d1aa38",
    },
];

#[derive(Clone)]
struct Fixture {
    owner_id: Uuid,
    run_id: Uuid,
    config_id: Uuid,
    job_id: Uuid,
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
         VALUES ('r4-v2-migration.test', $1, $2) RETURNING id",
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

async fn insert_strategy_config(
    db: &ScratchDb,
    owner: Uuid,
    strategy_id: &str,
    config_json: &Value,
) -> TestResult<Uuid> {
    sqlx::query(
        "INSERT INTO public.strategies (id, display_name, state)
         VALUES ($1, $1, 'Validated') ON CONFLICT (id) DO NOTHING",
    )
    .bind(strategy_id)
    .execute(&db.pool)
    .await?;
    let config_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO public.user_strategy_configs
            (id, owner_user_id, strategy_id, strategy_version, config_json, is_active)
         VALUES ($1, $2, $3, '1.0.0', $4, true)",
    )
    .bind(config_id)
    .bind(owner)
    .bind(strategy_id)
    .bind(config_json)
    .execute(&db.pool)
    .await?;
    Ok(config_id)
}

fn pins_json() -> Value {
    json!({
        "candidate_content_sha256": CANDIDATE_SHA256,
        "artifact_manifest_sha256": ARTIFACT_SHA256,
        "stage5_manifest_sha256": STAGE5_SHA256,
        "action_manifest_sha256": ACTION_SHA256,
        "approval_registry_sha256": APPROVAL_SHA256
    })
}

fn v2_payload(
    case: &StrategyCase,
    run_id: Uuid,
    config_id: Uuid,
    schema: &str,
    contract_sha256: &str,
    config_override: Option<Value>,
    hash_override: Option<&str>,
) -> Value {
    let config: Value = config_override.unwrap_or_else(|| {
        serde_json::from_str(case.config_json).expect("fixed strategy config JSON")
    });
    json!({
        "schema": schema,
        "run_id": run_id.to_string(),
        "strategy_config_id": config_id.to_string(),
        "evaluation_start": RUN_START,
        "evaluation_end": RUN_END,
        "pins": pins_json(),
        "contract_document_sha256": contract_sha256,
        "bars_sha256": APPROVED_BARS_SHA256,
        "strategy": {
            "strategy_id": case.id,
            "strategy_version": "1.0.0",
            "config_json": config,
            "config_sha256": hash_override.unwrap_or(case.config_sha256)
        }
    })
}

async fn insert_v2_fixture(
    db: &ScratchDb,
    app: &PgPool,
    owner: Uuid,
    case: &StrategyCase,
) -> TestResult<Fixture> {
    let config: Value = serde_json::from_str(case.config_json)?;
    let config_id = insert_strategy_config(db, owner, case.id, &config).await?;
    let run_id = Uuid::new_v4();
    let payload = v2_payload(
        case,
        run_id,
        config_id,
        V2_SCHEMA,
        V2_CONTRACT_SHA256,
        None,
        None,
    );
    let input = V2SimulationJobInput::from_json(&serde_json::to_vec(&payload)?)
        .expect("fixed v2 wire reconstructs the complete preparation prefix");
    let session_count = i32::try_from(input.required_dates().len())?;

    let mut transaction = app.begin().await?;
    set_actor(&mut transaction, owner).await?;
    let job_id: Uuid = sqlx::query_scalar(
        "INSERT INTO public.jobs
            (owner_user_id, job_type, status, priority, idempotency_key,
             payload_json, max_attempts)
         VALUES ($1, $2, 'QUEUED', 10, $3, $4, 3)
         RETURNING id",
    )
    .bind(owner)
    .bind(JOB_TYPE)
    .bind(format!("{V2_PREFIX}{}", Uuid::new_v4()))
    .bind(payload)
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
    .bind(case.id)
    .bind(config)
    .bind(case.config_sha256)
    .bind(NaiveDate::parse_from_str(RUN_START, "%Y-%m-%d")?)
    .bind(NaiveDate::parse_from_str(RUN_END, "%Y-%m-%d")?)
    .bind(session_count)
    .bind(CANDIDATE_SHA256)
    .bind(ARTIFACT_SHA256)
    .bind(STAGE5_SHA256)
    .bind(ACTION_SHA256)
    .bind(APPROVAL_SHA256)
    .bind(V2_CONTRACT_SHA256)
    .bind(APPROVED_BARS_SHA256)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(Fixture {
        owner_id: owner,
        run_id,
        config_id,
        job_id,
    })
}

fn v1_input(run_id: Uuid, config_id: Uuid) -> SimulationJobInput {
    let config = json!({
        "benchmark_instrument": "069500.KRX",
        "fast_ma": 50,
        "slow_ma": 200
    });
    let bytes = serde_json::to_vec(&json!({
        "schema": V1_SCHEMA,
        "run_id": run_id,
        "strategy_config_id": config_id,
        "evaluation_start": RUN_START,
        "evaluation_end": RUN_END,
        "pins": pins_json(),
        "contract_document_sha256": APPROVED_CONTRACT_SHA256,
        "bars_sha256": APPROVED_BARS_SHA256,
        "strategy": {
            "strategy_id": "trend_following",
            "strategy_version": "1.0.0",
            "config_json": config,
            "config_sha256": "sha256:90e5b4c6972678e9186d4655d76496676e7cfb8dbe473a249f062963873c04a0"
        }
    }))
    .expect("fixed v1 input JSON");
    SimulationJobInput::from_json(&bytes).expect("original checked v1 input constructor")
}

async fn insert_v1_fixture(db: &ScratchDb, app: &PgPool, owner: Uuid) -> TestResult<Fixture> {
    let run_id = Uuid::new_v4();
    let config_id = Uuid::new_v4();
    let input = v1_input(run_id, config_id);
    let config = input.strategy().config_json().clone();
    sqlx::query(
        "INSERT INTO public.strategies (id, display_name, state)
         VALUES ('trend_following', 'Trend following', 'Validated')
         ON CONFLICT (id) DO NOTHING",
    )
    .execute(&db.pool)
    .await?;
    sqlx::query(
        "INSERT INTO public.user_strategy_configs
            (id, owner_user_id, strategy_id, strategy_version, config_json, is_active)
         VALUES ($1, $2, 'trend_following', '1.0.0', $3, true)",
    )
    .bind(config_id)
    .bind(owner)
    .bind(&config)
    .execute(&db.pool)
    .await?;

    let mut transaction = app.begin().await?;
    set_actor(&mut transaction, owner).await?;
    let job_id: Uuid = sqlx::query_scalar(
        "INSERT INTO public.jobs
            (owner_user_id, job_type, status, priority, idempotency_key,
             payload_json, max_attempts)
         VALUES ($1, $2, 'QUEUED', 10, $3, $4, 3)
         RETURNING id",
    )
    .bind(owner)
    .bind(JOB_TYPE)
    .bind(format!("{V1_PREFIX}{}", Uuid::new_v4()))
    .bind(serde_json::to_value(&input)?)
    .fetch_one(&mut *transaction)
    .await?;
    let session_count = (input.schedule().warmup_sessions().len()
        + input.schedule().evaluation_sessions().len()) as i32;
    sqlx::query(
        "INSERT INTO public.owner_beta_simulation_runs
            (id, owner_user_id, strategy_config_id, job_id,
             strategy_id, strategy_version, strategy_config_json, strategy_config_sha256,
             evaluation_start, evaluation_end, session_count,
             candidate_content_sha256, artifact_manifest_sha256, stage5_manifest_sha256,
             action_manifest_sha256, approval_registry_sha256,
             contract_document_sha256, bars_sha256)
         VALUES ($1, $2, $3, $4, 'trend_following', '1.0.0', $5, $6, $7, $8, $9,
                 $10, $11, $12, $13, $14, $15, $16)",
    )
    .bind(run_id)
    .bind(owner)
    .bind(config_id)
    .bind(job_id)
    .bind(config)
    .bind(input.strategy().config_sha256().as_str())
    .bind(NaiveDate::parse_from_str(RUN_START, "%Y-%m-%d")?)
    .bind(NaiveDate::parse_from_str(RUN_END, "%Y-%m-%d")?)
    .bind(session_count)
    .bind(input.pins().candidate_content_sha256().as_str())
    .bind(input.pins().artifact_manifest_sha256().as_str())
    .bind(input.pins().stage5_manifest_sha256().as_str())
    .bind(input.pins().action_manifest_sha256().as_str())
    .bind(input.pins().approval_registry_sha256().as_str())
    .bind(input.contract_document_sha256().as_str())
    .bind(input.bars_sha256().as_str())
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(Fixture {
        owner_id: owner,
        run_id,
        config_id,
        job_id,
    })
}

fn v1_signal_targets() -> Vec<(String, u32)> {
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

fn v1_pair(fixture: &Fixture) -> CheckedSimulationPair {
    let input = v1_input(fixture.run_id, fixture.config_id);
    let engine = EngineIdentity::new(
        &"a".repeat(40),
        &format!("sha256:{}", "b".repeat(64)),
        "rustc-source-fixture",
        "x86_64-unknown-linux-gnu",
        "test",
    )
    .expect("synthetic engine identity");
    // Both profiles bind the same signal path; only their transaction costs differ.
    let make_result = |profile: CostProfile| {
        let factor_hash = format!("sha256:{}", "c".repeat(64));
        let target_hash = format!("sha256:{}", "d".repeat(64));
        let signals = input
            .schedule()
            .signal_execution_pairs()
            .into_iter()
            .map(|(signal_date, _)| {
                SignalSnapshot::new(
                    &signal_date.to_iso(),
                    &factor_hash,
                    &target_hash,
                    v1_signal_targets(),
                    0,
                )
                .expect("synthetic checked v1 signal")
            })
            .collect::<Vec<_>>();
        let prices = input
            .schedule()
            .evaluation_sessions()
            .iter()
            .map(|session| {
                EvaluationPriceRow::new(
                    &session.to_iso(),
                    Price4::parse("100.0000").expect("open price"),
                    Price4::parse("101.0000").expect("close price"),
                    1_000_000,
                )
                .expect("synthetic checked v1 price")
            })
            .collect::<Vec<_>>();
        let ledgers = simulate_ledgers(input.schedule(), &signals, &prices)
            .expect("synthetic production-capital v1 ledger");
        let identity = StrategyIdentity::trend_following_v1(
            input.strategy().config_sha256().as_str(),
            "1.0.0",
            &format!("sha256:{}", "e".repeat(64)),
        )
        .expect("fixed v1 strategy identity");
        let profile_ledgers = match profile {
            CostProfile::Illustrative10BpPerSideV1 => ledgers.illustrative_10bp_per_side(),
            CostProfile::ZeroCostReferenceV1 => ledgers.zero_cost_reference(),
        };
        SimulationResult::new(
            engine.clone(),
            identity,
            input.schedule().clone(),
            profile,
            signals,
            profile_ledgers.strategy().clone(),
            profile_ledgers.benchmark().clone(),
        )
        .expect("checked synthetic v1 result")
        .canonical_json()
        .expect("canonical v1 result bytes")
    };
    let illustrative = make_result(CostProfile::Illustrative10BpPerSideV1);
    let zero_cost = make_result(CostProfile::ZeroCostReferenceV1);
    CheckedSimulationPair::from_canonical(&illustrative, &zero_cost, &input, &engine)
        .expect("whole checked synthetic v1 pair")
}

fn v2_pair(fixture: &Fixture) -> (Vec<u8>, Vec<u8>, EngineIdentity) {
    let payload = v2_payload(
        &V2_CASES[0],
        fixture.run_id,
        fixture.config_id,
        V2_SCHEMA,
        V2_CONTRACT_SHA256,
        None,
        None,
    );
    let input = V2SimulationJobInput::from_json(
        &serde_json::to_vec(&payload).expect("fixed v2 input JSON"),
    )
    .expect("checked v2 wire and full calendar prefix");
    let schedule = input.schedule().clone();
    let strategy = V2StrategyIdentity::from_default_snapshot(
        "buy_and_hold",
        "1.0.0",
        r#"{"benchmark_instrument":"069500.KRX","rebalance_cadence":"none","target_weight":1.0}"#,
        "sha256:4de76616ae1d72646bd695d0e5aa7df7d6544c2eef5bef7ca949536c1801fc06",
        Vec::new(),
    )
    .expect("frozen v2 default strategy identity");
    let engine = EngineIdentity::new(
        &"a".repeat(40),
        &format!("sha256:{}", "b".repeat(64)),
        "rustc-source-fixture",
        "x86_64-unknown-linux-gnu",
        "test",
    )
    .expect("synthetic engine identity");
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
        .expect("synthetic checked v2 target");

    let make_run = |profile| {
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
                    .expect("scheduled buy-and-hold allocation");
            }
            ledger.close_session().expect("close synthetic session");
        }
        ledger
            .finish(&schedule)
            .expect("checked synthetic v2 ledger")
    };
    let illustrative_ledger = make_run(CostProfile::Illustrative10BpPerSideV1);
    let zero_cost_ledger = make_run(CostProfile::ZeroCostReferenceV1);
    let illustrative = V2SimulationResult::new(
        strategy.clone(),
        engine.clone(),
        schedule.clone(),
        CostProfile::Illustrative10BpPerSideV1,
        illustrative_ledger.clone(),
        illustrative_ledger,
    )
    .expect("checked synthetic illustrative v2 result");
    let zero_cost = V2SimulationResult::new(
        strategy,
        engine.clone(),
        schedule,
        CostProfile::ZeroCostReferenceV1,
        zero_cost_ledger.clone(),
        zero_cost_ledger,
    )
    .expect("checked synthetic zero-cost v2 result");
    let pair =
        V2SimulationPair::new(illustrative, zero_cost).expect("whole checked synthetic v2 pair");
    let (illustrative, zero_cost) = pair.canonical_json().expect("canonical v2 pair bodies");
    V2CheckedSimulationPair::from_canonical(&illustrative, &zero_cost, &input, &engine)
        .expect("synthetic v2 pair binds to the queued input and complete schedule");
    (illustrative, zero_cost, engine)
}

fn sha256(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

async fn begin_worker_attempt(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    fixture: &Fixture,
    worker_id: &str,
) -> TestResult<Uuid> {
    sqlx::query(
        "UPDATE public.jobs
            SET status = 'RUNNING', attempt_count = 1, locked_by = $2,
                locked_at = pg_catalog.now(), started_at = pg_catalog.now(),
                updated_at = pg_catalog.now()
          WHERE id = $1",
    )
    .bind(fixture.job_id)
    .bind(worker_id)
    .execute(&mut **transaction)
    .await?;
    let attempt_id: Uuid = sqlx::query_scalar(
        "INSERT INTO public.job_attempts
            (job_id, attempt_no, outcome, claimed_by, started_at)
         VALUES ($1, 1, 'RUNNING', $2, pg_catalog.now())
         RETURNING id",
    )
    .bind(fixture.job_id)
    .bind(worker_id)
    .fetch_one(&mut **transaction)
    .await?;
    sqlx::query(
        "UPDATE public.owner_beta_simulation_runs
            SET started_at = pg_catalog.now(), updated_at = pg_catalog.now()
          WHERE id = $1",
    )
    .bind(fixture.run_id)
    .execute(&mut **transaction)
    .await?;
    Ok(attempt_id)
}

async fn finish_v2_success(
    fixture: &Fixture,
    worker: &PgPool,
    illustrative: &[u8],
    zero_cost: &[u8],
    engine: &EngineIdentity,
) -> TestResult {
    let mut transaction = worker.begin().await?;
    let actor_before = current_actor(&mut transaction).await?;
    let attempt_id = begin_worker_attempt(&mut transaction, fixture, "r4-v2-source-worker").await?;
    sqlx::query(
        "INSERT INTO public.owner_beta_simulation_result_pairs
            (run_id, owner_user_id, illustrative_canonical, illustrative_sha256,
             zero_cost_canonical, zero_cost_sha256)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(fixture.run_id)
    .bind(fixture.owner_id)
    .bind(illustrative)
    .bind(sha256(illustrative))
    .bind(zero_cost)
    .bind(sha256(zero_cost))
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "UPDATE public.owner_beta_simulation_runs
            SET status = 'SUCCEEDED', engine_json = $2,
                published_attempt_id = $3, finished_at = pg_catalog.now(),
                updated_at = pg_catalog.now()
          WHERE id = $1",
    )
    .bind(fixture.run_id)
    .bind(serde_json::to_value(engine)?)
    .bind(attempt_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "UPDATE public.job_attempts
            SET outcome = 'SUCCEEDED', finished_at = pg_catalog.now()
          WHERE id = $1",
    )
    .bind(attempt_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "UPDATE public.jobs
            SET status = 'SUCCEEDED', locked_by = NULL, locked_at = NULL,
                finished_at = pg_catalog.now(), updated_at = pg_catalog.now()
          WHERE id = $1",
    )
    .bind(fixture.job_id)
    .execute(&mut *transaction)
    .await?;
    assert_terminal_actor_preserved(&mut transaction, &actor_before).await?;
    transaction.commit().await?;
    Ok(())
}

async fn current_actor(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> TestResult<Option<String>> {
    Ok(
        sqlx::query_scalar("SELECT pg_catalog.current_setting('app.actor_user_id', true)")
            .fetch_one(&mut **transaction)
            .await?,
    )
}

async fn assert_terminal_actor_preserved(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    before: &Option<String>,
) -> TestResult {
    assert_eq!(
        &current_actor(transaction).await?,
        before,
        "immediate v2 terminal callbacks preserve the caller's raw actor"
    );
    sqlx::query("SET CONSTRAINTS ALL IMMEDIATE")
        .execute(&mut **transaction)
        .await?;
    assert_eq!(
        &current_actor(transaction).await?,
        before,
        "deferred v2 terminal callbacks preserve the caller's raw actor"
    );
    Ok(())
}

async fn finish_v2_failure(fixture: &Fixture, worker: &PgPool, canceled: bool) -> TestResult {
    let mut transaction = worker.begin().await?;
    let actor_before = current_actor(&mut transaction).await?;
    let attempt_id =
        begin_worker_attempt(&mut transaction, fixture, "r4-v2-terminal-worker").await?;
    let (job_status, run_status, attempt_outcome) = if canceled {
        ("CANCELED", "CANCELED", "ORPHANED")
    } else {
        ("FAILED", "FAILED", "FAILED")
    };
    sqlx::query(
        "UPDATE public.owner_beta_simulation_runs
            SET status = $2, error_code = 'SYNTHETIC_FAILURE',
                finished_at = pg_catalog.now(), updated_at = pg_catalog.now()
          WHERE id = $1",
    )
    .bind(fixture.run_id)
    .bind(run_status)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "UPDATE public.job_attempts
            SET outcome = $2, error_code = 'SYNTHETIC_FAILURE',
                finished_at = pg_catalog.now()
          WHERE id = $1",
    )
    .bind(attempt_id)
    .bind(attempt_outcome)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "UPDATE public.jobs
            SET status = $2, error_code = 'SYNTHETIC_FAILURE',
                locked_by = NULL, locked_at = NULL,
                finished_at = pg_catalog.now(), updated_at = pg_catalog.now()
          WHERE id = $1",
    )
    .bind(fixture.job_id)
    .bind(job_status)
    .execute(&mut *transaction)
    .await?;
    assert_terminal_actor_preserved(&mut transaction, &actor_before).await?;
    transaction.commit().await?;
    Ok(())
}

async fn insert_rejected_job(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    owner: Uuid,
    payload: Value,
    idempotency_key: String,
) -> TestResult {
    let payload_json = serde_json::to_string(&payload)?;
    insert_rejected_job_json(transaction, owner, &payload_json, idempotency_key).await
}

async fn insert_rejected_job_json(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    owner: Uuid,
    payload_json: &str,
    idempotency_key: String,
) -> TestResult {
    sqlx::query("SAVEPOINT rejected_v2_insert")
        .execute(&mut **transaction)
        .await?;
    let result = sqlx::query(
        "INSERT INTO public.jobs
            (owner_user_id, job_type, status, priority, idempotency_key,
             payload_json, max_attempts)
         VALUES ($1, $2, 'QUEUED', 10, $3, $4::jsonb, 3)",
    )
    .bind(owner)
    .bind(JOB_TYPE)
    .bind(idempotency_key)
    .bind(payload_json)
    .execute(&mut **transaction)
    .await;
    let error = result.expect_err("invalid version/default cross-product is rejected");
    assert_eq!(
        error
            .as_database_error()
            .and_then(|database| database.code())
            .as_deref(),
        Some("23514")
    );
    sqlx::query("ROLLBACK TO SAVEPOINT rejected_v2_insert")
        .execute(&mut **transaction)
        .await?;
    sqlx::query("RELEASE SAVEPOINT rejected_v2_insert")
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

async fn reject_cross_products(app: &PgPool, owner: Uuid) -> TestResult {
    let mut transaction = app.begin().await?;
    set_actor(&mut transaction, owner).await?;
    let base = v2_payload(
        &V2_CASES[0],
        Uuid::new_v4(),
        Uuid::new_v4(),
        V2_SCHEMA,
        V2_CONTRACT_SHA256,
        None,
        None,
    );
    let mut invalid = Vec::new();

    let mut wrong_schema = base.clone();
    wrong_schema["schema"] = json!("unknown-owner-beta-schema");
    invalid.push((wrong_schema, format!("{V2_PREFIX}{}", Uuid::new_v4())));

    let mut v2_schema_v1_contract = base.clone();
    v2_schema_v1_contract["contract_document_sha256"] = json!(APPROVED_CONTRACT_SHA256);
    invalid.push((
        v2_schema_v1_contract,
        format!("{V2_PREFIX}{}", Uuid::new_v4()),
    ));

    let mut v1_schema_v2_contract = base.clone();
    v1_schema_v2_contract["schema"] = json!(V1_SCHEMA);
    invalid.push((
        v1_schema_v2_contract,
        format!("{V1_PREFIX}{}", Uuid::new_v4()),
    ));

    let mut wrong_contract = base.clone();
    wrong_contract["contract_document_sha256"] =
        json!("sha256:0000000000000000000000000000000000000000000000000000000000000000");
    invalid.push((wrong_contract, format!("{V2_PREFIX}{}", Uuid::new_v4())));

    let mut wrong_config = base.clone();
    wrong_config["strategy"]["config_json"]["target_weight"] = json!(0.5);
    invalid.push((wrong_config, format!("{V2_PREFIX}{}", Uuid::new_v4())));

    let mut wrong_hash = base.clone();
    wrong_hash["strategy"]["config_sha256"] =
        json!("sha256:0000000000000000000000000000000000000000000000000000000000000000");
    invalid.push((wrong_hash, format!("{V2_PREFIX}{}", Uuid::new_v4())));

    let mut wrong_namespace = base.clone();
    invalid.push((
        wrong_namespace.clone(),
        format!("{V1_PREFIX}{}", Uuid::new_v4()),
    ));
    wrong_namespace["strategy"]["strategy_id"] = json!("unsupported_strategy");
    invalid.push((wrong_namespace, format!("{V2_PREFIX}{}", Uuid::new_v4())));

    let mut integer_target_weight = base.clone();
    integer_target_weight["strategy"]["config_json"]["target_weight"] = json!(1);
    invalid.push((
        integer_target_weight,
        format!("{V2_PREFIX}{}", Uuid::new_v4()),
    ));

    let mut integer_zero_threshold = v2_payload(
        &V2_CASES[3],
        Uuid::new_v4(),
        Uuid::new_v4(),
        V2_SCHEMA,
        V2_CONTRACT_SHA256,
        None,
        None,
    );
    integer_zero_threshold["strategy"]["config_json"]["absolute_threshold"] = json!(0);
    invalid.push((
        integer_zero_threshold,
        format!("{V2_PREFIX}{}", Uuid::new_v4()),
    ));

    let mut integer_max_weight = v2_payload(
        &V2_CASES[4],
        Uuid::new_v4(),
        Uuid::new_v4(),
        V2_SCHEMA,
        V2_CONTRACT_SHA256,
        None,
        None,
    );
    integer_max_weight["strategy"]["config_json"]["max_weight"] = json!(0);
    invalid.push((integer_max_weight, format!("{V2_PREFIX}{}", Uuid::new_v4())));

    for (payload, key) in invalid {
        insert_rejected_job(&mut transaction, owner, payload, key).await?;
    }
    let mut decimal_scale_variant = serde_json::to_string(&v2_payload(
        &V2_CASES[4],
        Uuid::new_v4(),
        Uuid::new_v4(),
        V2_SCHEMA,
        V2_CONTRACT_SHA256,
        None,
        None,
    ))?;
    decimal_scale_variant =
        decimal_scale_variant.replace("\"max_weight\":0.3", "\"max_weight\":0.30");
    insert_rejected_job_json(
        &mut transaction,
        owner,
        &decimal_scale_variant,
        format!("{V2_PREFIX}{}", Uuid::new_v4()),
    )
    .await?;
    transaction.commit().await?;

    // A v1 numeric spelling remains governed by the original jsonb/hash checks;
    // this rejected mutation changes the actual default and its payload pair.
    let v1 = v1_input(Uuid::new_v4(), Uuid::new_v4());
    let mut invalid_v1 = serde_json::to_value(&v1)?;
    invalid_v1["strategy"]["config_json"]["fast_ma"] = json!(51);
    let mut transaction = app.begin().await?;
    set_actor(&mut transaction, owner).await?;
    insert_rejected_job(
        &mut transaction,
        owner,
        invalid_v1,
        format!("{V1_PREFIX}{}", Uuid::new_v4()),
    )
    .await?;
    transaction.commit().await?;
    Ok(())
}

async fn catalog_snapshot(db: &ScratchDb) -> TestResult<Value> {
    Ok(sqlx::query_scalar(
        "SELECT pg_catalog.jsonb_build_object(
            'rls', (
                SELECT pg_catalog.jsonb_agg(
                    pg_catalog.jsonb_build_array(c.relname, c.relrowsecurity, c.relforcerowsecurity)
                    ORDER BY c.relname
                )
                  FROM pg_catalog.pg_class AS c
                 WHERE c.oid IN (
                    'public.jobs'::regclass,
                    'public.owner_beta_simulation_runs'::regclass,
                    'public.owner_beta_simulation_result_pairs'::regclass,
                    'public.job_attempts'::regclass
                 )
            ),
            'run_constraints', (
                SELECT pg_catalog.jsonb_agg(
                    pg_catalog.jsonb_build_array(conname, pg_catalog.pg_get_constraintdef(oid), convalidated)
                    ORDER BY conname
                ) FROM pg_catalog.pg_constraint
                 WHERE conrelid = 'public.owner_beta_simulation_runs'::regclass
            ),
            'functions', (
                SELECT pg_catalog.jsonb_agg(
                    pg_catalog.jsonb_build_array(
                        p.proname,
                        pg_catalog.pg_get_functiondef(p.oid),
                        owner.rolname,
                        COALESCE(p.proconfig::text, ''),
                        COALESCE(p.proacl::text, '')
                    ) ORDER BY p.proname
                )
                  FROM pg_catalog.pg_proc AS p
                  JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace
                  JOIN pg_catalog.pg_roles AS owner ON owner.oid = p.proowner
                 WHERE n.nspname = 'public'
                   AND p.proname IN (
                    'owner_beta_simulation_version_binding_matches',
                    'owner_beta_simulation_version_namespace_matches',
                    'owner_beta_simulation_payload_matches',
                    'owner_beta_simulation_jobs_insert_guard',
                    'owner_beta_simulation_runs_validate_binding'
                   )
            ),
            'policies', (
                SELECT pg_catalog.jsonb_agg(
                    pg_catalog.jsonb_build_array(tablename, policyname, roles, cmd, qual, with_check)
                    ORDER BY tablename, policyname
                ) FROM pg_catalog.pg_policies
                 WHERE schemaname = 'public'
                   AND (tablename = 'jobs' OR tablename LIKE 'owner_beta_simulation_%')
            )
        )",
    )
    .fetch_one(&db.pool)
    .await?)
}

async fn v2_states(db: &ScratchDb) -> TestResult<Vec<(String, String, i32)>> {
    Ok(sqlx::query_as(
        "SELECT run.strategy_id, run.status, job.attempt_count
           FROM public.owner_beta_simulation_runs AS run
           JOIN public.jobs AS job ON job.id = run.job_id
          WHERE run.contract_document_sha256 = $1
          ORDER BY run.strategy_id",
    )
    .bind(V2_CONTRACT_SHA256)
    .fetch_all(&db.pool)
    .await?)
}

async fn v1_pair_snapshot(
    db: &ScratchDb,
    fixture: &Fixture,
) -> TestResult<(Vec<u8>, String, Vec<u8>, String, Value)> {
    Ok(sqlx::query_as(
        "SELECT pair.illustrative_canonical, pair.illustrative_sha256,
                pair.zero_cost_canonical, pair.zero_cost_sha256, run.engine_json
           FROM public.owner_beta_simulation_result_pairs AS pair
           JOIN public.owner_beta_simulation_runs AS run ON run.id = pair.run_id
          WHERE pair.run_id = $1 AND pair.owner_user_id = $2",
    )
    .bind(fixture.run_id)
    .bind(fixture.owner_id)
    .fetch_one(&db.pool)
    .await?)
}

async fn apply_sql(pool: &PgPool, source: &'static str) -> TestResult {
    let mut transaction = pool.begin().await?;
    sqlx::raw_sql(source).execute(&mut *transaction).await?;
    transaction.commit().await?;
    Ok(())
}

#[tokio::test]
async fn v2_defaults_are_admitted_and_any_v2_history_refuses_down() {
    let Some(db) = require_scratch_db().await else {
        return;
    };
    let owner = new_owner(&db, "v2-owner").await.unwrap();
    let foreign_owner = new_owner(&db, "v2-foreign").await.unwrap();
    let app = role_pool(&db, "app", None).await;
    let fixtures = V2_CASES
        .iter()
        .map(|case| insert_v2_fixture(&db, &app, owner, case));
    let mut admitted = Vec::new();
    for result in fixtures {
        admitted.push(result.await.unwrap());
    }
    reject_cross_products(&app, owner).await.unwrap();

    // Keep v1 admission/rejection covered after 0064 is present.
    let v1 = insert_v1_fixture(&db, &app, owner).await.unwrap();
    let mut invalid_v1 = serde_json::to_value(&v1_input(Uuid::new_v4(), Uuid::new_v4())).unwrap();
    invalid_v1["contract_document_sha256"] = json!(V2_CONTRACT_SHA256);
    let mut reject_tx = app.begin().await.unwrap();
    set_actor(&mut reject_tx, owner).await.unwrap();
    insert_rejected_job(
        &mut reject_tx,
        owner,
        invalid_v1,
        format!("{V1_PREFIX}{}", Uuid::new_v4()),
    )
    .await
    .unwrap();
    reject_tx.commit().await.unwrap();

    let (illustrative, zero_cost, engine) = v2_pair(&admitted[0]);
    // A different valid actor makes accidental owner leakage observable exactly.
    let worker = role_pool(&db, "worker", Some(foreign_owner)).await;
    finish_v2_success(&admitted[0], &worker, &illustrative, &zero_cost, &engine)
        .await
        .unwrap();
    finish_v2_failure(&admitted[1], &worker, false)
        .await
        .unwrap();
    finish_v2_failure(&admitted[2], &worker, true)
        .await
        .unwrap();

    assert_eq!(
        v2_states(&db).await.unwrap(),
        vec![
            ("buy_and_hold".to_owned(), "SUCCEEDED".to_owned(), 1),
            ("dual_momentum".to_owned(), "PENDING".to_owned(), 0),
            ("inverse_volatility".to_owned(), "PENDING".to_owned(), 0),
            ("relative_momentum".to_owned(), "CANCELED".to_owned(), 1),
            ("trend_following".to_owned(), "FAILED".to_owned(), 1),
        ]
    );

    let owner_app = role_pool(&db, "app", Some(owner)).await;
    let own_runs: i64 =
        sqlx::query_scalar("SELECT count(*) FROM public.owner_beta_simulation_runs")
            .fetch_one(&owner_app)
            .await
            .unwrap();
    let own_pairs: i64 =
        sqlx::query_scalar("SELECT count(*) FROM public.owner_beta_simulation_result_pairs")
            .fetch_one(&owner_app)
            .await
            .unwrap();
    let pending_pair_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.owner_beta_simulation_result_pairs WHERE run_id = $1",
    )
    .bind(admitted[3].run_id)
    .fetch_one(&owner_app)
    .await
    .unwrap();
    let failed_pair_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.owner_beta_simulation_result_pairs WHERE run_id = $1",
    )
    .bind(admitted[1].run_id)
    .fetch_one(&owner_app)
    .await
    .unwrap();
    let canceled_pair_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.owner_beta_simulation_result_pairs WHERE run_id = $1",
    )
    .bind(admitted[2].run_id)
    .fetch_one(&owner_app)
    .await
    .unwrap();
    assert_eq!(
        own_runs, 6,
        "owner sees its five v2 runs and retained v1 run"
    );
    assert_eq!(own_pairs, 1);
    assert_eq!(pending_pair_count, 0, "pending v2 results remain hidden");
    assert_eq!(failed_pair_count, 0, "failed runs have no visible pair");
    assert_eq!(canceled_pair_count, 0, "canceled runs have no visible pair");

    let foreign_app = role_pool(&db, "app", Some(foreign_owner)).await;
    let foreign_runs: i64 =
        sqlx::query_scalar("SELECT count(*) FROM public.owner_beta_simulation_runs")
            .fetch_one(&foreign_app)
            .await
            .unwrap();
    let foreign_pairs: i64 =
        sqlx::query_scalar("SELECT count(*) FROM public.owner_beta_simulation_result_pairs")
            .fetch_one(&foreign_app)
            .await
            .unwrap();
    assert_eq!(foreign_runs, 0, "another owner sees no simulation runs");
    assert_eq!(foreign_pairs, 0, "another owner sees no v2 pair");
    let no_actor_app = role_pool(&db, "app", None).await;
    let no_actor_runs: i64 =
        sqlx::query_scalar("SELECT count(*) FROM public.owner_beta_simulation_runs")
            .fetch_one(&no_actor_app)
            .await
            .unwrap();
    let no_actor_pairs: i64 =
        sqlx::query_scalar("SELECT count(*) FROM public.owner_beta_simulation_result_pairs")
            .fetch_one(&no_actor_app)
            .await
            .unwrap();
    assert_eq!(no_actor_runs, 0, "missing actor sees no simulation runs");
    assert_eq!(no_actor_pairs, 0, "missing actor sees no v2 pair");

    let before = catalog_snapshot(&db).await.unwrap();
    let migration_owner = role_pool(&db, "migration_owner", None).await;
    let mut down = migration_owner.begin().await.unwrap();
    let refusal = sqlx::raw_sql(include_str!(
        "../../../migrations/0064_owner_beta_simulation_v2.down.sql"
    ))
    .execute(&mut *down)
    .await
    .expect_err("v2 run/job/attempt history prevents downgrade");
    assert_eq!(
        refusal
            .as_database_error()
            .and_then(|database| database.code())
            .as_deref(),
        Some("55000")
    );
    down.rollback().await.unwrap();
    assert_eq!(catalog_snapshot(&db).await.unwrap(), before);
    assert_eq!(v2_states(&db).await.unwrap().len(), 5);

    owner_app.close().await;
    foreign_app.close().await;
    no_actor_app.close().await;
    migration_owner.close().await;
    worker.close().await;
    app.close().await;
    let _ = v1;
    db.drop_db().await;
}

#[tokio::test]
async fn v1_pair_bytes_survive_v2_up_down_and_reupgrade() {
    let Some(db) = require_scratch_db().await else {
        return;
    };
    let migration_owner = role_pool(&db, "migration_owner", None).await;
    apply_sql(
        &migration_owner,
        include_str!("../../../migrations/0064_owner_beta_simulation_v2.down.sql"),
    )
    .await
    .unwrap();

    let owner = new_owner(&db, "retained-v1").await.unwrap();
    let app = role_pool(&db, "app", None).await;
    let fixture = insert_v1_fixture(&db, &app, owner).await.unwrap();
    let pair = v1_pair(&fixture);
    let worker = role_pool(&db, "worker", None).await;
    let mut transaction = worker.begin().await.unwrap();
    let worker_id = "r4-v1-migration-source-worker";
    sqlx::query(
        "UPDATE public.jobs
            SET status = 'RUNNING', attempt_count = 1, locked_by = $2,
                locked_at = pg_catalog.now(), started_at = pg_catalog.now(),
                updated_at = pg_catalog.now()
          WHERE id = $1",
    )
    .bind(fixture.job_id)
    .bind(worker_id)
    .execute(&mut *transaction)
    .await
    .unwrap();
    let attempt_id: Uuid = sqlx::query_scalar(
        "INSERT INTO public.job_attempts
            (job_id, attempt_no, outcome, claimed_by, started_at)
         VALUES ($1, 1, 'RUNNING', $2, pg_catalog.now())
         RETURNING id",
    )
    .bind(fixture.job_id)
    .bind(worker_id)
    .fetch_one(&mut *transaction)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE public.owner_beta_simulation_runs
            SET started_at = pg_catalog.now(), updated_at = pg_catalog.now()
          WHERE id = $1",
    )
    .bind(fixture.run_id)
    .execute(&mut *transaction)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO public.owner_beta_simulation_result_pairs
            (run_id, owner_user_id, illustrative_canonical, illustrative_sha256,
             zero_cost_canonical, zero_cost_sha256)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(fixture.run_id)
    .bind(owner)
    .bind(pair.illustrative_10bp_per_side_bytes())
    .bind(pair.illustrative_10bp_per_side_sha256().as_str())
    .bind(pair.zero_cost_reference_bytes())
    .bind(pair.zero_cost_reference_sha256().as_str())
    .execute(&mut *transaction)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE public.owner_beta_simulation_runs
            SET status = 'SUCCEEDED', engine_json = $2,
                published_attempt_id = $3, finished_at = pg_catalog.now(),
                updated_at = pg_catalog.now()
          WHERE id = $1",
    )
    .bind(fixture.run_id)
    .bind(serde_json::to_value(pair.illustrative_10bp_per_side_result().engine()).unwrap())
    .bind(attempt_id)
    .execute(&mut *transaction)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE public.job_attempts
            SET outcome = 'SUCCEEDED', finished_at = pg_catalog.now()
          WHERE id = $1",
    )
    .bind(attempt_id)
    .execute(&mut *transaction)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE public.jobs
            SET status = 'SUCCEEDED', locked_by = NULL, locked_at = NULL,
                finished_at = pg_catalog.now(), updated_at = pg_catalog.now()
          WHERE id = $1",
    )
    .bind(fixture.job_id)
    .execute(&mut *transaction)
    .await
    .unwrap();
    transaction.commit().await.unwrap();
    let before = v1_pair_snapshot(&db, &fixture).await.unwrap();

    apply_sql(
        &migration_owner,
        include_str!("../../../migrations/0064_owner_beta_simulation_v2.up.sql"),
    )
    .await
    .unwrap();
    assert_eq!(v1_pair_snapshot(&db, &fixture).await.unwrap(), before);

    // A retained v1 run and its whole pair are not a reason to refuse 0064 down.
    apply_sql(
        &migration_owner,
        include_str!("../../../migrations/0064_owner_beta_simulation_v2.down.sql"),
    )
    .await
    .unwrap();
    assert_eq!(v1_pair_snapshot(&db, &fixture).await.unwrap(), before);
    apply_sql(
        &migration_owner,
        include_str!("../../../migrations/0064_owner_beta_simulation_v2.up.sql"),
    )
    .await
    .unwrap();
    assert_eq!(v1_pair_snapshot(&db, &fixture).await.unwrap(), before);

    let owner_app = role_pool(&db, "app", Some(owner)).await;
    let visible: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.owner_beta_simulation_result_pairs WHERE run_id = $1",
    )
    .bind(fixture.run_id)
    .fetch_one(&owner_app)
    .await
    .unwrap();
    assert_eq!(
        visible, 1,
        "retained v1 pair remains owner-visible after re-upgrade"
    );

    owner_app.close().await;
    worker.close().await;
    app.close().await;
    migration_owner.close().await;
    db.drop_db().await;
}
