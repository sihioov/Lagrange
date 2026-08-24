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
    OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE, OwnerBetaPriceRecommendationInput,
    OwnerBetaPublicationError, OwnerBetaPublicationOutcome, OwnerBetaStrategySnapshot,
    build_target_snapshot, publish_owner_beta_success,
};
use job_queue::resolver::ResolvedConfig;
use job_queue::{AttemptOutcome, AuditActor, JobQueue, JobStatus, QueueConfig, SubmitJob};
use serde_json::json;
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
    let report = queue.sweep().await.expect("worker sweep");
    assert_eq!(report.attempts_orphaned, 1);
    assert_eq!(report.jobs_requeued, 1);

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
