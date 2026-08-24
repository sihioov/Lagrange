//! PostgreSQL transaction/RLS coverage for the dedicated owner-beta publisher.
//!
//! The fixture below is deliberately TEST-ONLY.  Its sentinel hashes are not
//! approval-registry pins and this file never calls the artifact approval or
//! computation boundary.  The public `Deserialize` implementation is used to
//! create the durable payload solely so this suite can exercise queue/run
//! persistence and cancellation/lease state transitions.

mod common;

use std::collections::BTreeMap;
use std::time::Duration;

use common::ScratchDb;
use domain::ContentHash;
use factor_engine::PriceOnlyFactorSnapshot;
use factor_engine::price_only::{PRICE_ONLY_CAPABILITY, PRICE_ONLY_INPUT_KIND};
use factor_engine::snapshot::{FactorRow, NormalizationMeta};
use job_queue::owner_beta::{
    OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE, OwnerBetaOutcome, OwnerBetaPriceRecommendationInput,
    OwnerBetaPublicationError, OwnerBetaPublicationOutcome, OwnerBetaRunnerConfig,
    OwnerBetaRunnerPaths, OwnerBetaStrategySnapshot, build_target_snapshot,
    publish_owner_beta_success, recover_owner_beta_claims, run_once,
};
use job_queue::resolver::ResolvedConfig;
use job_queue::{AttemptOutcome, AuditActor, JobQueue, JobStatus, QueueConfig, SubmitJob};
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use uuid::Uuid;

const RUN_ID: &str = "00000000-0000-4000-8000-000000000101";
const CONFIG_ID: &str = "00000000-0000-4000-8000-000000000102";
const AS_OF: &str = "2026-08-24";

struct Fixture {
    input: OwnerBetaPriceRecommendationInput,
    factor: PriceOnlyFactorSnapshot,
    target: job_queue::owner_beta::OwnerBetaTargetSnapshot,
}

type RecoveryRunFailureRow = (
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<chrono::DateTime<chrono::Utc>>,
);

type RejectedPublicationRunRow = (
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<chrono::DateTime<chrono::Utc>>,
    Option<chrono::DateTime<chrono::Utc>>,
    i64,
);

type PublishedRunRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<chrono::DateTime<chrono::Utc>>,
    Option<chrono::DateTime<chrono::Utc>>,
);

type PublishedItemRow = (
    String,
    Option<i32>,
    Option<String>,
    Value,
    Value,
    bool,
    Option<String>,
);

fn sentinel_hash(number: u8) -> String {
    // These values intentionally do not come from an approved artifact.
    format!("sha256:{number:064x}")
}

fn fixture() -> Fixture {
    let resolved = ResolvedConfig {
        strategy_id: "buy_and_hold".to_owned(),
        strategy_version: "1.0.0".to_owned(),
        config: json!({
            "benchmark_instrument": "069500.KRX",
            "target_weight": 1.0,
        }),
    };
    let strategy = OwnerBetaStrategySnapshot::from_resolved_config(&resolved)
        .expect("test strategy snapshot must be constructible");
    let input: OwnerBetaPriceRecommendationInput = serde_json::from_value(json!({
        "run_id": RUN_ID,
        "strategy_config_id": CONFIG_ID,
        "as_of": AS_OF,
        "pins": {
            "candidate_content_sha256": sentinel_hash(1),
            "artifact_manifest_sha256": sentinel_hash(2),
            "stage5_manifest_sha256": sentinel_hash(3),
            "action_manifest_sha256": sentinel_hash(4),
            "approval_registry_sha256": sentinel_hash(5),
        },
        "strategy": serde_json::to_value(&strategy).expect("strategy JSON"),
    }))
    .expect("public owner-beta payload deserializer");
    input
        .validate_strategy_snapshot()
        .expect("fixture strategy hash must validate");

    let mut factor = PriceOnlyFactorSnapshot {
        input_kind: PRICE_ONLY_INPUT_KIND.to_owned(),
        capability: PRICE_ONLY_CAPABILITY.to_owned(),
        as_of: input.as_of(),
        candidate_content_sha256: input.pins().candidate_content_sha256().to_string(),
        artifact_manifest_sha256: input.pins().artifact_manifest_sha256().to_string(),
        stage5_manifest_sha256: input.pins().stage5_manifest_sha256().to_string(),
        action_manifest_sha256: input.pins().action_manifest_sha256().to_string(),
        approval_registry_sha256: input.pins().approval_registry_sha256().to_string(),
        factor_versions: BTreeMap::new(),
        normalization: NormalizationMeta {
            id: "owner-beta-test".to_owned(),
            version: "1.0.0".to_owned(),
            params: BTreeMap::new(),
        },
        rows: Vec::<FactorRow>::new(),
        hash: ContentHash::from_bytes(b"owner-beta-test-factor-placeholder"),
    };
    input
        .validate_factor_snapshot(&factor)
        .expect("fixture factor provenance must validate");
    factor.hash = factor.compute_hash().expect("fixture factor hash");
    let target = build_target_snapshot(&input, &factor).expect("fixture target");
    target.validate_hash().expect("fixture target hash");

    Fixture {
        input,
        factor,
        target,
    }
}

async fn role_pool(db: &ScratchDb, role: &str, actor: Option<Uuid>) -> PgPool {
    let mut options: PgConnectOptions = db
        .role_url(role)
        .parse()
        .unwrap_or_else(|_| panic!("valid {role} role URL"));
    if let Some(actor) = actor {
        options = options.options([("app.actor_user_id", actor.to_string())]);
    }
    PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .unwrap_or_else(|_| panic!("connect as {role}"))
}

async fn seed_fixture(db: &ScratchDb, app: &PgPool, owner_id: Uuid, fixture: &Fixture) -> Uuid {
    let config_id = fixture.input.strategy_config_id();

    sqlx::query(
        "INSERT INTO strategies (id, display_name, state) \
         VALUES ('buy_and_hold', 'Buy and hold', 'Paper')",
    )
    .execute(&db.pool)
    .await
    .expect("seed strategy registry");

    sqlx::query(
        "INSERT INTO user_strategy_configs \
         (id, owner_user_id, strategy_id, strategy_version, config_json) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(config_id)
    .bind(owner_id)
    .bind(fixture.input.strategy_snapshot().strategy_id())
    .bind(fixture.input.strategy_snapshot().strategy_version())
    .bind(fixture.input.strategy_snapshot().config_json())
    .execute(app)
    .await
    .expect("app role inserts its own strategy config");

    for item in fixture.target.items() {
        let instrument_id = item.instrument_id();
        let symbol = instrument_id
            .strip_suffix(".KRX")
            .expect("target instrument uses KRX suffix");
        sqlx::query(
            "INSERT INTO instruments \
             (id, symbol, venue, currency, name, asset_class, status, listed_at) \
             VALUES ($1, $2, 'KRX', 'KRW', $3, 'ETF', 'ACTIVE', DATE '2000-01-01')",
        )
        .bind(instrument_id)
        .bind(symbol)
        .bind(format!("Test ETF {symbol}"))
        .execute(&db.pool)
        .await
        .expect("seed target instrument master");
    }

    sqlx::query(
        "INSERT INTO data_entitlements \
         (contract_document_sha256, contract_reference, status, covered_datasets, \
          covered_uses, effective_from, effective_until, managed_by) \
         VALUES (repeat('e', 64), 'fixture://owner-beta-test', 'ACTIVE', \
                 '[\"krx_eod_bars\"]', '[\"recommendation\"]', \
                 DATE '2020-01-01', DATE '2030-12-31', $1)",
    )
    .bind(owner_id)
    .execute(&db.pool)
    .await
    .expect("seed active recommendation entitlement");

    let app_queue = JobQueue::new(app.clone(), None, QueueConfig::default());
    let job = app_queue
        .submit(SubmitJob {
            owner_user_id: owner_id,
            job_type: OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE.to_owned(),
            payload: serde_json::to_value(&fixture.input).expect("fixture payload JSON"),
            priority: 0,
            idempotency_key: Some(format!("owner-beta-publish-{}", Uuid::new_v4())),
            max_attempts: 2,
            available_at: None,
        })
        .await
        .expect("app role submits owner-beta job");

    sqlx::query(
        "INSERT INTO owner_beta_recommendation_runs \
         (id, owner_user_id, strategy_config_id, strategy_id, strategy_version, \
          strategy_config_json, strategy_config_sha256, job_id, as_of, \
          candidate_content_sha256, artifact_manifest_sha256, stage5_manifest_sha256, \
          action_manifest_sha256, approval_registry_sha256) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
    )
    .bind(fixture.input.run_id())
    .bind(owner_id)
    .bind(config_id)
    .bind(fixture.input.strategy_snapshot().strategy_id())
    .bind(fixture.input.strategy_snapshot().strategy_version())
    .bind(fixture.input.strategy_snapshot().config_json())
    .bind(fixture.input.strategy_snapshot().config_sha256().as_str())
    .bind(job.id)
    .bind(fixture.input.as_of().as_naive_date())
    .bind(fixture.input.pins().candidate_content_sha256().as_str())
    .bind(fixture.input.pins().artifact_manifest_sha256().as_str())
    .bind(fixture.input.pins().stage5_manifest_sha256().as_str())
    .bind(fixture.input.pins().action_manifest_sha256().as_str())
    .bind(fixture.input.pins().approval_registry_sha256().as_str())
    .execute(app)
    .await
    .expect("app role inserts owner-beta run projection");

    job.id
}

fn queue_config(lease: Duration) -> QueueConfig {
    QueueConfig {
        lease,
        backoff_base: Duration::from_millis(100),
    }
}

#[tokio::test]
async fn published_owner_beta_target_persists_exact_results_and_seals_claim() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let fixture = fixture();
    assert_eq!(fixture.target.items().len(), 11, "fixture must cover ETF11");
    let owner_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) VALUES ('owner-beta.test', $1, $2) RETURNING id",
    )
    .bind(format!("owner-beta-published-{}", Uuid::new_v4()))
    .bind(format!("owner-beta-published-{}@example.test", Uuid::new_v4()))
    .fetch_one(&db.pool)
    .await
    .expect("seed publication owner");
    let app = role_pool(&db, "app", Some(owner_id)).await;
    let job_id = seed_fixture(&db, &app, owner_id, &fixture).await;
    let worker = role_pool(&db, "worker", None).await;
    let worker_can_update_configs: bool = sqlx::query_scalar(
        "SELECT has_table_privilege(\
            current_user, 'public.user_strategy_configs', 'UPDATE'\
         )",
    )
    .fetch_one(&worker)
    .await
    .expect("worker checks its strategy-config privilege");
    assert!(
        !worker_can_update_configs,
        "publication must not grant worker strategy-config mutation"
    );
    let worker_can_update_any_config_column: bool = sqlx::query_scalar(
        "SELECT has_any_column_privilege(\
            current_user, 'public.user_strategy_configs', 'UPDATE'\
         )",
    )
    .fetch_one(&worker)
    .await
    .expect("worker checks its strategy-config column privileges");
    assert!(
        !worker_can_update_any_config_column,
        "publication must not grant worker any column-level strategy-config mutation"
    );
    let queue = JobQueue::new(worker.clone(), None, queue_config(Duration::from_secs(30)));
    let claim = queue
        .claim_next_for(
            "owner-beta-published-worker",
            OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE,
        )
        .await
        .expect("worker claim")
        .expect("owner-beta job is claimable");
    assert_eq!(claim.job.id, job_id);

    let outcome = publish_owner_beta_success(
        &worker,
        &queue,
        &claim,
        &fixture.input,
        &fixture.factor,
        &fixture.target,
    )
    .await
    .expect("owner-beta publication succeeds atomically");
    assert_eq!(outcome, OwnerBetaPublicationOutcome::Published);

    let run: PublishedRunRow = sqlx::query_as(
        "SELECT status, strategy_id, strategy_version, strategy_config_sha256, \
                candidate_content_sha256, artifact_manifest_sha256, stage5_manifest_sha256, \
                action_manifest_sha256, approval_registry_sha256, factor_snapshot_sha256, \
                target_snapshot_sha256, cash_weight::text, error_code, started_at, finished_at \
         FROM owner_beta_recommendation_runs WHERE id = $1",
    )
    .bind(fixture.input.run_id())
    .fetch_one(&worker)
    .await
    .expect("worker reads published run");
    assert_eq!(run.0, "SUCCEEDED");
    assert_eq!(run.1, fixture.input.strategy_snapshot().strategy_id());
    assert_eq!(run.2, fixture.input.strategy_snapshot().strategy_version());
    assert_eq!(
        run.3,
        fixture.input.strategy_snapshot().config_sha256().as_str()
    );
    assert_eq!(
        run.4,
        fixture.input.pins().candidate_content_sha256().as_str()
    );
    assert_eq!(
        run.5,
        fixture.input.pins().artifact_manifest_sha256().as_str()
    );
    assert_eq!(
        run.6,
        fixture.input.pins().stage5_manifest_sha256().as_str()
    );
    assert_eq!(
        run.7,
        fixture.input.pins().action_manifest_sha256().as_str()
    );
    assert_eq!(
        run.8,
        fixture.input.pins().approval_registry_sha256().as_str()
    );
    assert_eq!(run.9, fixture.factor.hash.as_str());
    assert_eq!(run.10, fixture.target.target_snapshot_sha256().as_str());
    assert_eq!(
        run.11.as_deref(),
        Some(fixture.target.cash_weight().as_str())
    );
    assert_eq!(run.12, None);
    assert!(run.13.is_some(), "published run has a start time");
    assert!(run.14.is_some(), "published run has a finish time");

    let result_nulls: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM owner_beta_recommendation_runs \
         WHERE id = $1 AND (factor_snapshot_sha256 IS NULL \
             OR target_snapshot_sha256 IS NULL OR cash_weight IS NULL \
             OR started_at IS NULL OR finished_at IS NULL OR error_code IS NOT NULL)",
    )
    .bind(fixture.input.run_id())
    .fetch_one(&worker)
    .await
    .expect("worker verifies result fields");
    assert_eq!(result_nulls, 0, "published run has complete result fields");

    let items: (i64, i64, String) = sqlx::query_as(
        "SELECT count(*), count(DISTINCT instrument_id), \
                (coalesce(sum(target_weight), 0) + $2::numeric)::numeric(18, 6)::text \
         FROM owner_beta_recommendation_items WHERE recommendation_run_id = $1",
    )
    .bind(fixture.input.run_id())
    .bind(fixture.target.cash_weight())
    .fetch_one(&worker)
    .await
    .expect("worker verifies exact target items and cash residual");
    assert_eq!(items, (11, 11, "1.000000".to_owned()));

    let persisted_items: Vec<PublishedItemRow> = sqlx::query_as(
        "SELECT instrument_id, rank, target_weight::text, reason_codes, factors_json, \
                excluded, exclusion_reason \
         FROM owner_beta_recommendation_items \
         WHERE recommendation_run_id = $1 \
         ORDER BY instrument_id ASC",
    )
    .bind(fixture.input.run_id())
    .fetch_all(&worker)
    .await
    .expect("worker reads canonical target rows");
    assert_eq!(persisted_items.len(), 11);
    for (persisted, expected) in persisted_items.iter().zip(fixture.target.items()) {
        let expected_weight = expected.target_weight();
        let expected_reason_codes = Value::Array(
            expected
                .reasons()
                .iter()
                .map(|reason| Value::String(reason.code().to_owned()))
                .collect(),
        );
        let expected_factors =
            serde_json::to_value(expected.factors()).expect("fixture factors serialize");
        let expected_excluded = expected_weight.is_none();
        let expected_exclusion_reason = expected_excluded
            .then(|| expected.reasons().first().map(|reason| reason.code()))
            .flatten();
        assert_eq!(persisted.0, expected.instrument_id());
        assert_eq!(
            persisted.1,
            expected
                .rank()
                .map(i32::try_from)
                .transpose()
                .expect("fixture ranks fit PostgreSQL integer")
        );
        assert_eq!(persisted.2, expected_weight);
        assert_eq!(persisted.3, expected_reason_codes);
        assert_eq!(persisted.4, expected_factors);
        assert_eq!(persisted.5, expected_excluded);
        assert_eq!(persisted.6.as_deref(), expected_exclusion_reason);
    }

    let job: (
        JobStatus,
        Option<String>,
        Option<String>,
        Option<chrono::DateTime<chrono::Utc>>,
    ) = sqlx::query_as(
        "SELECT status, error_code, error_message, finished_at FROM jobs WHERE id = $1",
    )
    .bind(job_id)
    .fetch_one(&worker)
    .await
    .expect("worker reads settled job");
    assert_eq!(job.0, JobStatus::Succeeded);
    assert_eq!(job.1, None);
    assert_eq!(job.2, None);
    assert!(job.3.is_some(), "succeeded job has a finish time");
    let attempt: (
        AttemptOutcome,
        Option<String>,
        Option<String>,
        Option<chrono::DateTime<chrono::Utc>>,
    ) = sqlx::query_as(
        "SELECT outcome, error_code, error_message, finished_at \
         FROM job_attempts WHERE job_id = $1 AND attempt_no = 1",
    )
    .bind(job_id)
    .fetch_one(&worker)
    .await
    .expect("worker reads settled attempt");
    assert_eq!(attempt.0, AttemptOutcome::Succeeded);
    assert_eq!(attempt.1, None);
    assert_eq!(attempt.2, None);
    assert!(attempt.3.is_some(), "succeeded attempt has a finish time");

    // Success settlement has no audit hook in the current queue contract;
    // only explicit cancellation writes audit_logs.  Keep this assertion as a
    // regression detector rather than inventing a test-only audit event.
    let success_audit_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE target_type = 'job' AND target_id = $1",
    )
    .bind(job_id.to_string())
    .fetch_one(&app)
    .await
    .expect("app reads owner-visible audit log");
    assert_eq!(success_audit_count, 0);

    let settled_before_replay: (Value, Value, Value) = sqlx::query_as(
        "SELECT \
           (SELECT to_jsonb(run) FROM owner_beta_recommendation_runs AS run WHERE run.id = $1), \
           (SELECT to_jsonb(job) FROM jobs AS job WHERE job.id = $2), \
           (SELECT to_jsonb(attempt) FROM job_attempts AS attempt \
            WHERE attempt.job_id = $2 AND attempt.attempt_no = 1)",
    )
    .bind(fixture.input.run_id())
    .bind(job_id)
    .fetch_one(&worker)
    .await
    .expect("worker snapshots complete settled rows");

    let replay = publish_owner_beta_success(
        &worker,
        &queue,
        &claim,
        &fixture.input,
        &fixture.factor,
        &fixture.target,
    )
    .await;
    assert_eq!(replay, Err(OwnerBetaPublicationError::QueueClaimLost));
    let item_count_after_replay: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM owner_beta_recommendation_items WHERE recommendation_run_id = $1",
    )
    .bind(fixture.input.run_id())
    .fetch_one(&worker)
    .await
    .expect("worker confirms replay did not duplicate target");
    assert_eq!(item_count_after_replay, 11);
    let settled_after_replay: (Value, Value, Value) = sqlx::query_as(
        "SELECT \
           (SELECT to_jsonb(run) FROM owner_beta_recommendation_runs AS run WHERE run.id = $1), \
           (SELECT to_jsonb(job) FROM jobs AS job WHERE job.id = $2), \
           (SELECT to_jsonb(attempt) FROM job_attempts AS attempt \
            WHERE attempt.job_id = $2 AND attempt.attempt_no = 1)",
    )
    .bind(fixture.input.run_id())
    .bind(job_id)
    .fetch_one(&worker)
    .await
    .expect("worker re-reads complete settled rows");
    assert_eq!(settled_after_replay, settled_before_replay);

    worker.close().await;
    app.close().await;
    db.drop_db().await;
}

#[tokio::test]
async fn publication_config_lock_cannot_impersonate_another_owner() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let fixture = fixture();
    let owner_a: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) VALUES ('owner-beta.test', $1, $2) RETURNING id",
    )
    .bind(format!("owner-beta-lock-a-{}", Uuid::new_v4()))
    .bind(format!("owner-beta-lock-a-{}@example.test", Uuid::new_v4()))
    .fetch_one(&db.pool)
    .await
    .expect("seed publication owner A");
    let owner_b: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) VALUES ('owner-beta.test', $1, $2) RETURNING id",
    )
    .bind(format!("owner-beta-lock-b-{}", Uuid::new_v4()))
    .bind(format!("owner-beta-lock-b-{}@example.test", Uuid::new_v4()))
    .fetch_one(&db.pool)
    .await
    .expect("seed publication owner B");
    let app_a = role_pool(&db, "app", Some(owner_a)).await;
    let job_id = seed_fixture(&db, &app_a, owner_a, &fixture).await;
    let app_b = role_pool(&db, "app", Some(owner_b)).await;
    let other_config_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO user_strategy_configs \
         (id, owner_user_id, strategy_id, strategy_version, config_json) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(other_config_id)
    .bind(owner_b)
    .bind(fixture.input.strategy_snapshot().strategy_id())
    .bind(fixture.input.strategy_snapshot().strategy_version())
    .bind(fixture.input.strategy_snapshot().config_json())
    .execute(&app_b)
    .await
    .expect("owner B creates an exact but unrelated config");
    let mismatched_config = json!({
        "benchmark_instrument": "069500.KRX",
        "target_weight": 0.5,
    });
    let bound_config: (bool, Value) = sqlx::query_as(
        "UPDATE user_strategy_configs SET config_json = $2 \
         WHERE id = $1 RETURNING is_active, config_json",
    )
    .bind(fixture.input.strategy_config_id())
    .bind(&mismatched_config)
    .fetch_one(&app_a)
    .await
    .expect("owner A mutates its active bound config after enqueue");
    assert_eq!(
        bound_config,
        (true, mismatched_config),
        "same-owner bound config remains active but no longer matches sealed JSON"
    );

    let worker = role_pool(&db, "worker", None).await;
    let worker_can_invoke_trigger: bool = sqlx::query_scalar(
        "SELECT has_function_privilege(\
            current_user, \
            'public.owner_beta_recommendation_runs_lock_strategy_config_on_success()', \
            'EXECUTE'\
         )",
    )
    .fetch_one(&worker)
    .await
    .expect("worker checks trigger-function privilege");
    assert!(
        !worker_can_invoke_trigger,
        "worker must not invoke the owner-derived lock function directly"
    );
    let queue = JobQueue::new(worker.clone(), None, queue_config(Duration::from_secs(30)));
    let claim = queue
        .claim_next_for(
            "owner-beta-cross-owner-lock-worker",
            OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE,
        )
        .await
        .expect("worker claim")
        .expect("owner-beta job is claimable");
    assert_eq!(claim.job.id, job_id);

    let publication = publish_owner_beta_success(
        &worker,
        &queue,
        &claim,
        &fixture.input,
        &fixture.factor,
        &fixture.target,
    )
    .await;
    assert_eq!(
        publication,
        Err(OwnerBetaPublicationError::DatabaseIntegrity),
        "another owner's exact active config must not satisfy the bound run"
    );
    let state: RejectedPublicationRunRow = sqlx::query_as(
        "SELECT status, factor_snapshot_sha256, target_snapshot_sha256, cash_weight::text, \
                error_code, started_at, finished_at, \
                (SELECT count(*) FROM owner_beta_recommendation_items \
                  WHERE recommendation_run_id = $1) \
         FROM owner_beta_recommendation_runs WHERE id = $1",
    )
    .bind(fixture.input.run_id())
    .fetch_one(&worker)
    .await
    .expect("worker verifies failed publication rollback");
    assert_eq!(
        state,
        ("PENDING".to_owned(), None, None, None, None, None, None, 0,)
    );
    let job: (
        JobStatus,
        Option<String>,
        Option<String>,
        Option<chrono::DateTime<chrono::Utc>>,
    ) = sqlx::query_as(
        "SELECT status, error_code, error_message, finished_at FROM jobs WHERE id = $1",
    )
    .bind(job_id)
    .fetch_one(&worker)
    .await
    .expect("worker verifies claim remains unsettled after rollback");
    assert_eq!(job, (JobStatus::Running, None, None, None));
    let attempt: (
        AttemptOutcome,
        Option<String>,
        Option<String>,
        Option<chrono::DateTime<chrono::Utc>>,
    ) = sqlx::query_as(
        "SELECT outcome, error_code, error_message, finished_at \
         FROM job_attempts WHERE job_id = $1 AND attempt_no = $2",
    )
    .bind(job_id)
    .bind(claim.job.attempt_count)
    .fetch_one(&worker)
    .await
    .expect("worker verifies exact claimed attempt remains unsettled after rollback");
    assert_eq!(attempt, (AttemptOutcome::Running, None, None, None));

    worker.close().await;
    app_b.close().await;
    app_a.close().await;
    db.drop_db().await;
}

#[tokio::test]
async fn canceled_before_owner_beta_publication_lock_has_no_outputs() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let fixture = fixture();
    let owner_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) VALUES ('owner-beta.test', $1, $2) RETURNING id",
    )
    .bind(format!("owner-beta-cancel-{}", Uuid::new_v4()))
    .bind(format!("owner-beta-cancel-{}@example.test", Uuid::new_v4()))
    .fetch_one(&db.pool)
    .await
    .expect("seed cancellation owner");
    let app = role_pool(&db, "app", Some(owner_id)).await;
    let job_id = seed_fixture(&db, &app, owner_id, &fixture).await;
    let audit = role_pool(&db, "audit_writer", None).await;
    let worker = role_pool(&db, "worker", None).await;
    let app_queue = JobQueue::new(
        app.clone(),
        Some(audit),
        queue_config(Duration::from_secs(30)),
    );
    let worker_queue = JobQueue::new(worker.clone(), None, queue_config(Duration::from_secs(30)));
    let claim = worker_queue
        .claim_next_for(
            "owner-beta-cancel-worker",
            OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE,
        )
        .await
        .expect("worker claim")
        .expect("owner-beta job is claimable");
    assert_eq!(claim.job.id, job_id);

    match app_queue
        .request_cancel(job_id, &AuditActor::new("owner"))
        .await
        .expect("audited app cancellation")
    {
        job_queue::CancelResult::Canceled(job) => assert_eq!(job.status, JobStatus::Canceled),
        other => panic!("expected cancellation, got {other:?}"),
    }

    let outcome = publish_owner_beta_success(
        &worker,
        &worker_queue,
        &claim,
        &fixture.input,
        &fixture.factor,
        &fixture.target,
    )
    .await
    .expect("canceled publication settles atomically");
    assert_eq!(outcome, OwnerBetaPublicationOutcome::Canceled);

    let job_status: JobStatus = sqlx::query_scalar("SELECT status FROM jobs WHERE id = $1")
        .bind(job_id)
        .fetch_one(&worker)
        .await
        .expect("worker reads job status");
    assert_eq!(job_status, JobStatus::Canceled);
    let attempt: (AttemptOutcome, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT outcome, error_code, error_message FROM job_attempts \
         WHERE job_id = $1 AND attempt_no = 1",
    )
    .bind(job_id)
    .fetch_one(&worker)
    .await
    .expect("worker reads finalized attempt");
    assert_eq!(attempt.0, AttemptOutcome::Failed);
    assert_eq!(attempt.1.as_deref(), Some("canceled"));
    assert!(
        attempt.2.is_some(),
        "canceled attempt carries a static reason"
    );

    let run: (
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT status, factor_snapshot_sha256, target_snapshot_sha256, \
                    cash_weight::text, error_code \
             FROM owner_beta_recommendation_runs WHERE id = $1",
    )
    .bind(fixture.input.run_id())
    .fetch_one(&worker)
    .await
    .expect("worker reads canceled run");
    assert_eq!(
        run,
        (
            "CANCELED".to_owned(),
            None,
            None,
            None,
            Some("CANCELED".to_owned())
        )
    );
    let item_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM owner_beta_recommendation_items WHERE recommendation_run_id = $1",
    )
    .bind(fixture.input.run_id())
    .fetch_one(&worker)
    .await
    .expect("worker reads owner-beta items");
    assert_eq!(item_count, 0);
    let audit_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE action = 'job.canceled' AND target_id = $1",
    )
    .bind(job_id.to_string())
    .fetch_one(&app)
    .await
    .expect("app reads cancellation audit");
    assert_eq!(audit_count, 1);

    worker.close().await;
    app.close().await;
    db.drop_db().await;
}

#[tokio::test]
async fn stale_swept_owner_beta_claim_cannot_publish_or_touch_run() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let fixture = fixture();
    let owner_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) VALUES ('owner-beta.test', $1, $2) RETURNING id",
    )
    .bind(format!("owner-beta-stale-{}", Uuid::new_v4()))
    .bind(format!("owner-beta-stale-{}@example.test", Uuid::new_v4()))
    .fetch_one(&db.pool)
    .await
    .expect("seed stale owner");
    let app = role_pool(&db, "app", Some(owner_id)).await;
    let job_id = seed_fixture(&db, &app, owner_id, &fixture).await;
    let worker = role_pool(&db, "worker", None).await;
    let queue = JobQueue::new(worker.clone(), None, queue_config(Duration::from_secs(1)));
    let claim = queue
        .claim_next_for(
            "owner-beta-stale-worker",
            OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE,
        )
        .await
        .expect("worker claim")
        .expect("owner-beta job is claimable");
    sqlx::query("UPDATE jobs SET locked_at = now() - interval '2 minutes' WHERE id = $1")
        .bind(job_id)
        .execute(&worker)
        .await
        .expect("expire old claim with database clock");
    let generic_report = queue.sweep().await.expect("generic worker sweep");
    assert_eq!(generic_report.jobs_checked, 0);
    assert_eq!(generic_report.attempts_orphaned, 0);
    assert_eq!(generic_report.jobs_requeued, 0);

    let recovery_report = recover_owner_beta_claims(&queue)
        .await
        .expect("dedicated owner-beta recovery");
    assert_eq!(recovery_report.attempts_orphaned, 1);
    assert_eq!(recovery_report.jobs_requeued, 1);

    let publication = publish_owner_beta_success(
        &worker,
        &queue,
        &claim,
        &fixture.input,
        &fixture.factor,
        &fixture.target,
    )
    .await;
    assert_eq!(publication, Err(OwnerBetaPublicationError::QueueClaimLost));

    let job: (JobStatus, Option<String>, Option<String>, i32) = sqlx::query_as(
        "SELECT status, locked_by, error_code, attempt_count FROM jobs WHERE id = $1",
    )
    .bind(job_id)
    .fetch_one(&worker)
    .await
    .expect("worker reads requeued job");
    assert_eq!(job, (JobStatus::Queued, None, None, 1));
    let attempt: (AttemptOutcome, Option<String>) = sqlx::query_as(
        "SELECT outcome, error_code FROM job_attempts \
         WHERE job_id = $1 AND attempt_no = 1",
    )
    .bind(job_id)
    .fetch_one(&worker)
    .await
    .expect("worker reads swept attempt");
    assert_eq!(attempt, (AttemptOutcome::Orphaned, None));
    let run: (
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT status, factor_snapshot_sha256, target_snapshot_sha256, \
                    cash_weight::text, error_code \
             FROM owner_beta_recommendation_runs WHERE id = $1",
    )
    .bind(fixture.input.run_id())
    .fetch_one(&worker)
    .await
    .expect("worker reads pending run");
    assert_eq!(run, ("PENDING".to_owned(), None, None, None, None));
    let item_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM owner_beta_recommendation_items WHERE recommendation_run_id = $1",
    )
    .bind(fixture.input.run_id())
    .fetch_one(&worker)
    .await
    .expect("worker reads owner-beta items");
    assert_eq!(item_count, 0);

    worker.close().await;
    app.close().await;
    db.drop_db().await;
}

#[tokio::test]
async fn owner_beta_recovery_exhaustion_mirrors_failed_run() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let fixture = fixture();
    let owner_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) VALUES ('owner-beta.test', $1, $2) RETURNING id",
    )
    .bind(format!("owner-beta-exhausted-{}", Uuid::new_v4()))
    .bind(format!("owner-beta-exhausted-{}@example.test", Uuid::new_v4()))
    .fetch_one(&db.pool)
    .await
    .expect("seed exhausted owner");
    let app = role_pool(&db, "app", Some(owner_id)).await;
    let job_id = seed_fixture(&db, &app, owner_id, &fixture).await;
    let worker = role_pool(&db, "worker", None).await;
    let queue = JobQueue::new(worker.clone(), None, queue_config(Duration::from_secs(1)));
    queue
        .claim_next_for(
            "owner-beta-exhausted-worker",
            OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE,
        )
        .await
        .expect("worker claim")
        .expect("owner-beta job is claimable");
    sqlx::query(
        "UPDATE jobs
         SET max_attempts = 1, locked_at = now() - interval '2 minutes'
         WHERE id = $1",
    )
    .bind(job_id)
    .execute(&worker)
    .await
    .expect("expire exhausted claim");

    let generic_report = queue.sweep().await.expect("generic worker sweep");
    assert_eq!(generic_report.jobs_checked, 0);
    let report = recover_owner_beta_claims(&queue)
        .await
        .expect("dedicated owner-beta recovery");
    assert_eq!(report.attempts_orphaned, 1);
    assert_eq!(report.jobs_failed, 1);

    let job: (
        JobStatus,
        Option<String>,
        Option<String>,
        Option<chrono::DateTime<chrono::Utc>>,
    ) = sqlx::query_as(
        "SELECT status, error_code, error_message, started_at FROM jobs WHERE id = $1",
    )
    .bind(job_id)
    .fetch_one(&worker)
    .await
    .expect("worker reads exhausted job");
    assert_eq!(job.0, JobStatus::Failed);
    assert_eq!(job.1.as_deref(), Some("attempts_exhausted"));
    assert_eq!(
        job.2.as_deref(),
        Some("owner-beta worker crash exhausted retries")
    );
    assert!(job.3.is_some(), "claimed job has a start time");

    let attempt: AttemptOutcome =
        sqlx::query_scalar("SELECT outcome FROM job_attempts WHERE job_id = $1 AND attempt_no = 1")
            .bind(job_id)
            .fetch_one(&worker)
            .await
            .expect("worker reads orphaned attempt");
    assert_eq!(attempt, AttemptOutcome::Orphaned);

    let run: RecoveryRunFailureRow = sqlx::query_as(
        "SELECT status, factor_snapshot_sha256, target_snapshot_sha256,
                cash_weight::text, error_code, started_at
         FROM owner_beta_recommendation_runs WHERE id = $1",
    )
    .bind(fixture.input.run_id())
    .fetch_one(&worker)
    .await
    .expect("worker reads exhausted run");
    assert_eq!(run.0, "FAILED");
    assert_eq!(run.1, None);
    assert_eq!(run.2, None);
    assert_eq!(run.3, None);
    assert_eq!(run.4.as_deref(), Some("OWNER_BETA_ATTEMPTS_EXHAUSTED"));
    assert_eq!(run.5, job.3);
    let item_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM owner_beta_recommendation_items WHERE recommendation_run_id = $1",
    )
    .bind(fixture.input.run_id())
    .fetch_one(&worker)
    .await
    .expect("worker reads owner-beta items");
    assert_eq!(item_count, 0);

    worker.close().await;
    app.close().await;
    db.drop_db().await;
}

#[tokio::test]
async fn owner_beta_recovery_mirrors_unclaimed_cancellation() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let fixture = fixture();
    let owner_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) VALUES ('owner-beta.test', $1, $2) RETURNING id",
    )
    .bind(format!("owner-beta-unclaimed-cancel-{}", Uuid::new_v4()))
    .bind(format!(
        "owner-beta-unclaimed-cancel-{}@example.test",
        Uuid::new_v4()
    ))
    .fetch_one(&db.pool)
    .await
    .expect("seed unclaimed cancellation owner");
    let app = role_pool(&db, "app", Some(owner_id)).await;
    let job_id = seed_fixture(&db, &app, owner_id, &fixture).await;
    let audit = role_pool(&db, "audit_writer", None).await;
    let app_queue = JobQueue::new(
        app.clone(),
        Some(audit),
        queue_config(Duration::from_secs(30)),
    );
    match app_queue
        .request_cancel(job_id, &AuditActor::new("owner"))
        .await
        .expect("audited unclaimed cancellation")
    {
        job_queue::CancelResult::Canceled(job) => assert_eq!(job.status, JobStatus::Canceled),
        other => panic!("expected cancellation, got {other:?}"),
    }
    let worker = role_pool(&db, "worker", None).await;
    let queue = JobQueue::new(worker.clone(), None, queue_config(Duration::from_secs(1)));
    let report = recover_owner_beta_claims(&queue)
        .await
        .expect("dedicated owner-beta cancellation recovery");
    assert_eq!(report.runs_canceled, 1);
    assert_eq!(report.attempts_orphaned, 0);

    let job: (JobStatus, Option<String>, Option<String>) =
        sqlx::query_as("SELECT status, locked_by, locked_at::text FROM jobs WHERE id = $1")
            .bind(job_id)
            .fetch_one(&worker)
            .await
            .expect("worker reads canceled job");
    assert_eq!(job, (JobStatus::Canceled, None, None));
    let run: (
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT status, factor_snapshot_sha256, target_snapshot_sha256,
                cash_weight::text, error_code
         FROM owner_beta_recommendation_runs WHERE id = $1",
    )
    .bind(fixture.input.run_id())
    .fetch_one(&worker)
    .await
    .expect("worker reads canceled run");
    assert_eq!(
        run,
        (
            "CANCELED".to_owned(),
            None,
            None,
            None,
            Some("CANCELED".to_owned())
        )
    );
    let started_at: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT started_at FROM owner_beta_recommendation_runs WHERE id = $1")
            .bind(fixture.input.run_id())
            .fetch_one(&worker)
            .await
            .expect("worker reads canceled run start");
    assert!(
        started_at.is_none(),
        "unclaimed cancellation has no fabricated start"
    );
    let item_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM owner_beta_recommendation_items WHERE recommendation_run_id = $1",
    )
    .bind(fixture.input.run_id())
    .fetch_one(&worker)
    .await
    .expect("worker reads owner-beta items");
    assert_eq!(item_count, 0);

    worker.close().await;
    app.close().await;
    db.drop_db().await;
}

#[tokio::test]
async fn malformed_owner_beta_payload_is_terminal_without_inventing_a_run() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let owner_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) VALUES ('owner-beta.test', $1, $2) RETURNING id",
    )
    .bind(format!("owner-beta-malformed-{}", Uuid::new_v4()))
    .bind(format!(
        "owner-beta-malformed-{}@example.test",
        Uuid::new_v4()
    ))
    .fetch_one(&db.pool)
    .await
    .expect("seed malformed owner");
    let app = role_pool(&db, "app", Some(owner_id)).await;
    let worker = role_pool(&db, "worker", None).await;
    let lease = Duration::from_secs(30);
    let app_queue = JobQueue::new(app.clone(), None, queue_config(lease));
    let job = app_queue
        .submit(SubmitJob {
            owner_user_id: owner_id,
            job_type: OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE.to_owned(),
            payload: json!({"malformed_test_payload": true}),
            priority: 0,
            idempotency_key: Some(format!("owner-beta-malformed-{}", Uuid::new_v4())),
            max_attempts: 2,
            available_at: None,
        })
        .await
        .expect("app submits malformed owner-beta fixture job");
    let worker_queue = JobQueue::new(worker.clone(), None, queue_config(lease));
    let artifact_root = tempfile::tempdir().expect("absolute test artifact root");
    let outcome = run_once(
        &worker,
        &worker_queue,
        "owner-beta-malformed-worker",
        &OwnerBetaRunnerPaths {
            artifact_root: artifact_root.path().to_path_buf(),
        },
        &OwnerBetaRunnerConfig::new(Duration::from_secs(1), lease, Duration::from_secs(2))
            .expect("valid runner timing"),
    )
    .await
    .expect("malformed payload is sealed-terminal");
    assert_eq!(outcome, OwnerBetaOutcome::Rejected { job_id: job.id });

    let stored: (JobStatus, Option<String>, Option<String>) =
        sqlx::query_as("SELECT status, error_code, error_message FROM jobs WHERE id = $1")
            .bind(job.id)
            .fetch_one(&worker)
            .await
            .expect("worker reads rejected job");
    assert_eq!(stored.0, JobStatus::Failed);
    assert_eq!(stored.1.as_deref(), Some("OWNER_BETA_INPUT_INVALID"));
    assert_eq!(stored.2.as_deref(), Some("owner-beta input rejected"));
    let attempt: (AttemptOutcome, Option<String>) = sqlx::query_as(
        "SELECT outcome, error_code FROM job_attempts WHERE job_id = $1 AND attempt_no = 1",
    )
    .bind(job.id)
    .fetch_one(&worker)
    .await
    .expect("worker reads rejected attempt");
    assert_eq!(
        attempt,
        (
            AttemptOutcome::Failed,
            Some("OWNER_BETA_INPUT_INVALID".to_owned())
        )
    );
    let run_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM owner_beta_recommendation_runs WHERE job_id = $1")
            .bind(job.id)
            .fetch_one(&worker)
            .await
            .expect("worker checks absent fabricated run");
    assert_eq!(run_count, 0);

    worker.close().await;
    app.close().await;
    db.drop_db().await;
}
