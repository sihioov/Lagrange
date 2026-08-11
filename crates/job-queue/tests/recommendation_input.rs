mod common;

use chrono::NaiveDate;
use common::ScratchDb;
use job_queue::recommendation::input::{
    AttestedDatasetStatus, RecommendationInputError, RecommendationPayload,
    attest_recommendation_input,
};
use job_queue::types::ErrorClass;
use serde_json::json;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;
use uuid::Uuid;

const RUN_ID: &str = "11111111-1111-4111-8111-111111111111";
const CONFIG_ID: &str = "22222222-2222-4222-8222-222222222222";
const DATASET_VERSION_ID: &str = "33333333-3333-4333-8333-333333333333";

fn scheduled_payload() -> serde_json::Value {
    json!({
        "run_id": RUN_ID,
        "strategy_config_id": CONFIG_ID,
        "as_of": "2026-08-11",
        "dataset": {
            "id": DATASET_VERSION_ID,
            "dataset_id": "kr-etf-core",
            "version": "2026-08-11",
            "curated_version": 7,
            "manifest_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        }
    })
}

#[test]
fn exact_scheduled_function_payload_parses_into_typed_fields() {
    let parsed = RecommendationPayload::try_from(scheduled_payload()).expect("valid payload");

    assert_eq!(parsed.run_id, Uuid::parse_str(RUN_ID).unwrap());
    assert_eq!(
        parsed.strategy_config_id,
        Uuid::parse_str(CONFIG_ID).unwrap()
    );
    assert_eq!(parsed.as_of, NaiveDate::from_ymd_opt(2026, 8, 11).unwrap());
    assert_eq!(
        parsed.dataset.id,
        Uuid::parse_str(DATASET_VERSION_ID).unwrap()
    );
    assert_eq!(parsed.dataset.dataset_id, "kr-etf-core");
    assert_eq!(parsed.dataset.version, "2026-08-11");
    assert_eq!(parsed.dataset.curated_version, 7);
    assert_eq!(parsed.dataset.manifest_sha256, "a".repeat(64));
}

#[test]
fn payload_rejects_missing_unknown_flat_and_storage_path_fields() {
    let mut unknown = scheduled_payload();
    unknown["unexpected"] = json!(true);

    let mut flat = scheduled_payload();
    flat.as_object_mut().unwrap().remove("dataset");
    flat["dataset_id"] = json!(DATASET_VERSION_ID);

    let mut storage_path = scheduled_payload();
    storage_path["dataset"]["storage_path"] = json!("untrusted/latest");

    let mut cases = vec![
        ("unknown".to_string(), unknown),
        ("flat".to_string(), flat),
        ("storage path".to_string(), storage_path),
    ];
    for field in ["run_id", "strategy_config_id", "as_of", "dataset"] {
        let mut missing = scheduled_payload();
        missing.as_object_mut().unwrap().remove(field);
        cases.push((format!("missing {field}"), missing));
    }
    for field in [
        "id",
        "dataset_id",
        "version",
        "curated_version",
        "manifest_sha256",
    ] {
        let mut missing = scheduled_payload();
        missing["dataset"].as_object_mut().unwrap().remove(field);
        cases.push((format!("missing dataset {field}"), missing));
    }

    for (case, payload) in cases {
        assert!(
            RecommendationPayload::try_from(payload).is_err(),
            "{case} payload must be rejected"
        );
    }
}

#[test]
fn payload_rejects_invalid_uuid_and_noncanonical_date() {
    let mut bad_run = scheduled_payload();
    bad_run["run_id"] = json!("not-a-uuid");

    let mut bad_config = scheduled_payload();
    bad_config["strategy_config_id"] = json!("not-a-uuid");

    let mut bad_dataset = scheduled_payload();
    bad_dataset["dataset"]["id"] = json!("not-a-uuid");

    let mut bad_date = scheduled_payload();
    bad_date["as_of"] = json!("2026-02-30");

    let mut noncanonical_date = scheduled_payload();
    noncanonical_date["as_of"] = json!("2026-8-11");

    for (case, payload) in [
        ("run uuid", bad_run),
        ("config uuid", bad_config),
        ("dataset uuid", bad_dataset),
        ("invalid date", bad_date),
        ("noncanonical date", noncanonical_date),
    ] {
        assert!(
            RecommendationPayload::try_from(payload).is_err(),
            "{case} must be rejected"
        );
    }
}

#[test]
fn payload_rejects_empty_dataset_names_invalid_hashes_and_versions() {
    let mut empty_dataset_id = scheduled_payload();
    empty_dataset_id["dataset"]["dataset_id"] = json!("  ");

    let mut empty_version = scheduled_payload();
    empty_version["dataset"]["version"] = json!("");

    let mut zero = scheduled_payload();
    zero["dataset"]["curated_version"] = json!(0);

    let mut overflow = scheduled_payload();
    overflow["dataset"]["curated_version"] = json!(u64::from(u32::MAX) + 1);

    let mut uppercase_hash = scheduled_payload();
    uppercase_hash["dataset"]["manifest_sha256"] = json!("A".repeat(64));

    let mut short_hash = scheduled_payload();
    short_hash["dataset"]["manifest_sha256"] = json!("a".repeat(63));

    let mut non_hex_hash = scheduled_payload();
    non_hex_hash["dataset"]["manifest_sha256"] = json!("g".repeat(64));

    for (case, payload) in [
        ("empty dataset id", empty_dataset_id),
        ("empty version", empty_version),
        ("zero version", zero),
        ("overflow version", overflow),
        ("uppercase hash", uppercase_hash),
        ("short hash", short_hash),
        ("non-hex hash", non_hex_hash),
    ] {
        assert!(
            RecommendationPayload::try_from(payload).is_err(),
            "{case} must be rejected"
        );
    }
}

#[test]
fn malformed_payload_error_is_permanent_input() {
    let mut malformed = scheduled_payload();
    malformed["dataset"]["curated_version"] = json!(0);

    let error = RecommendationPayload::try_from(malformed).expect_err("must reject malformed");
    assert_eq!(error.class(), ErrorClass::Input);
    assert_eq!(error.code(), "RECOMMENDATION_INPUT_MALFORMED");
}

#[tokio::test]
async fn unavailable_database_is_transient() {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_millis(100))
        .connect_lazy("postgresql://127.0.0.1:1/unavailable")
        .expect("lazy pool");
    let payload = RecommendationPayload::try_from(scheduled_payload()).unwrap();

    let error = attest_recommendation_input(
        &pool,
        Uuid::parse_str(RUN_ID).unwrap(),
        Uuid::new_v4(),
        payload,
    )
    .await
    .expect_err("database must be unavailable");

    assert_eq!(error.class(), ErrorClass::Transient);
    assert_eq!(error.code(), "RECOMMENDATION_INPUT_UNAVAILABLE");
}

async fn seed_schedulable_inputs(pool: &PgPool) -> (Uuid, Uuid, Uuid) {
    let owner_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) \
         VALUES ('https://issuer.test', 'recommendation-owner', 'recommendation@example.test') \
         RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("seed owner");
    sqlx::query(
        "INSERT INTO strategies (id, display_name, state) \
         VALUES ('recommendation_test', 'Recommendation Test', 'Paper')",
    )
    .execute(pool)
    .await
    .expect("seed strategy");
    let config_id: Uuid = sqlx::query_scalar(
        "INSERT INTO user_strategy_configs \
         (owner_user_id, strategy_id, strategy_version, config_json) \
         VALUES ($1, 'recommendation_test', '1.0.0', '{\"lookback\":12}'::jsonb) \
         RETURNING id",
    )
    .bind(owner_id)
    .fetch_one(pool)
    .await
    .expect("seed config");
    let dataset_id: Uuid = sqlx::query_scalar(
        "INSERT INTO dataset_versions \
         (dataset_id, version, status, manifest_sha256, storage_path) \
         VALUES ('kr-etf-core', '2026-08-11', 'READY', repeat('a', 64), \
                 'curated/authoritative/2026-08-11') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("seed dataset");
    let account_id: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (owner_user_id, account_type, name, status) \
         VALUES ($1, 'PAPER', 'recommendation-paper', 'ACTIVE') RETURNING id",
    )
    .bind(owner_id)
    .fetch_one(pool)
    .await
    .expect("seed account");
    sqlx::query(
        "INSERT INTO account_strategy_bindings \
         (account_id, owner_user_id, strategy_config_id, strategy_id, strategy_version, \
          auto_apply_recommendations) \
         VALUES ($1, $2, $3, 'recommendation_test', '1.0.0', true)",
    )
    .bind(account_id)
    .bind(owner_id)
    .bind(config_id)
    .execute(pool)
    .await
    .expect("seed binding");
    (owner_id, config_id, dataset_id)
}

#[tokio::test]
async fn scheduled_function_payload_parses_and_attests_under_worker_role() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let (owner_id, config_id, dataset_id) = seed_schedulable_inputs(&db.pool).await;
    let worker = PgPool::connect(&db.role_url("worker"))
        .await
        .expect("connect worker");
    let identity = format!("{owner_id}|{config_id}|2026-08-11|{dataset_id}");
    let key: String = sqlx::query_scalar("SELECT 'recommendation:scheduled:' || md5($1)")
        .bind(identity)
        .fetch_one(&worker)
        .await
        .expect("idempotency key");
    let (run_id, job_id): (Uuid, Uuid) = sqlx::query_as(
        "SELECT run_id, job_id FROM schedule_recommendation_run( \
         $1, $2, $3::date, $4, $5, $6, $7)",
    )
    .bind(owner_id)
    .bind(config_id)
    .bind("2026-08-11")
    .bind(dataset_id)
    .bind("a".repeat(64))
    .bind(7_i32)
    .bind(key)
    .fetch_one(&worker)
    .await
    .expect("schedule recommendation");
    let stored: serde_json::Value =
        sqlx::query_scalar("SELECT payload_json FROM jobs WHERE id = $1")
            .bind(job_id)
            .fetch_one(&worker)
            .await
            .expect("read scheduled payload");
    let payload = RecommendationPayload::try_from(stored).expect("parse scheduled payload");

    let attested = attest_recommendation_input(&worker, job_id, owner_id, payload)
        .await
        .expect("attest scheduled payload");

    assert_eq!(attested.payload.run_id, run_id);
    assert_eq!(attested.resolved_config.strategy_id, "recommendation_test");
    assert_eq!(attested.resolved_config.strategy_version, "1.0.0");
    assert_eq!(attested.resolved_config.config, json!({"lookback": 12}));
    assert_eq!(
        attested.dataset.storage_path,
        "curated/authoritative/2026-08-11"
    );
    let serialized_payload = serde_json::to_string(&attested.payload).expect("serialize payload");
    assert!(
        !serialized_payload.contains("storage_path"),
        "storage path must come only from the attested DB row"
    );

    worker.close().await;
    db.drop_db().await;
}

#[derive(Clone)]
struct ManualFixture {
    owner_id: Uuid,
    config_id: Uuid,
    dataset_id: Uuid,
    run_id: Uuid,
    job_id: Uuid,
    payload: RecommendationPayload,
}

async fn seed_manual_fixture(pool: &PgPool, suffix: &str, dataset_status: &str) -> ManualFixture {
    let owner_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) VALUES ('https://issuer.test', $1, $2) \
         RETURNING id",
    )
    .bind(format!("manual-{suffix}"))
    .bind(format!("manual-{suffix}@example.test"))
    .fetch_one(pool)
    .await
    .expect("seed manual owner");
    let strategy_id = format!("manual_strategy_{suffix}");
    sqlx::query("INSERT INTO strategies (id, display_name, state) VALUES ($1, $2, 'Paper')")
        .bind(&strategy_id)
        .bind(format!("Manual {suffix}"))
        .execute(pool)
        .await
        .expect("seed manual strategy");
    let config_id: Uuid = sqlx::query_scalar(
        "INSERT INTO user_strategy_configs \
         (owner_user_id, strategy_id, strategy_version, config_json) \
         VALUES ($1, $2, '1.0.0', $3) RETURNING id",
    )
    .bind(owner_id)
    .bind(&strategy_id)
    .bind(json!({"fixture": suffix}))
    .fetch_one(pool)
    .await
    .expect("seed manual config");
    let logical_dataset_id = format!("manual-dataset-{suffix}");
    let version = format!("version-{suffix}");
    let storage_path = format!("curated/manual/{suffix}");
    let dataset_id: Uuid = sqlx::query_scalar(
        "INSERT INTO dataset_versions \
         (dataset_id, version, status, manifest_sha256, storage_path) \
         VALUES ($1, $2, $3, repeat('b', 64), $4) RETURNING id",
    )
    .bind(&logical_dataset_id)
    .bind(&version)
    .bind(dataset_status)
    .bind(&storage_path)
    .fetch_one(pool)
    .await
    .expect("seed manual dataset");
    let run_id = Uuid::new_v4();
    let payload = RecommendationPayload::try_from(json!({
        "run_id": run_id,
        "strategy_config_id": config_id,
        "as_of": "2026-08-11",
        "dataset": {
            "id": dataset_id,
            "dataset_id": logical_dataset_id,
            "version": version,
            "curated_version": 2,
            "manifest_sha256": "b".repeat(64)
        }
    }))
    .expect("manual payload");
    let job_id: Uuid = sqlx::query_scalar(
        "INSERT INTO jobs (owner_user_id, job_type, payload_json) \
         VALUES ($1, 'recommendation', $2) RETURNING id",
    )
    .bind(owner_id)
    .bind(serde_json::to_value(&payload).unwrap())
    .fetch_one(pool)
    .await
    .expect("seed manual job");
    sqlx::query(
        "INSERT INTO recommendation_runs \
         (id, owner_user_id, strategy_config_id, as_of, status, job_id, trigger_kind, \
          dataset_version_id, dataset_manifest_sha256) \
         VALUES ($1, $2, $3, '2026-08-11', 'PENDING', $4, 'MANUAL', $5, repeat('b', 64))",
    )
    .bind(run_id)
    .bind(owner_id)
    .bind(config_id)
    .bind(job_id)
    .bind(dataset_id)
    .execute(pool)
    .await
    .expect("seed manual run");
    ManualFixture {
        owner_id,
        config_id,
        dataset_id,
        run_id,
        job_id,
        payload,
    }
}

async fn attest_error(worker: &PgPool, fixture: &ManualFixture) -> RecommendationInputError {
    attest_recommendation_input(
        worker,
        fixture.job_id,
        fixture.owner_id,
        fixture.payload.clone(),
    )
    .await
    .expect_err("attestation must fail")
}

#[tokio::test]
async fn attestation_rejects_foreign_stale_and_mismatched_rows_with_typed_classes() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let ready = seed_manual_fixture(&db.pool, "ready", "READY").await;
    let warning = seed_manual_fixture(&db.pool, "warning", "WARNING").await;
    let foreign = seed_manual_fixture(&db.pool, "foreign", "READY").await;
    let blocked = seed_manual_fixture(&db.pool, "blocked", "BLOCKED").await;
    let worker = PgPool::connect(&db.role_url("worker"))
        .await
        .expect("connect worker");

    let ready_attested =
        attest_recommendation_input(&worker, ready.job_id, ready.owner_id, ready.payload.clone())
            .await
            .expect("READY accepted");
    assert_eq!(ready_attested.dataset.status, AttestedDatasetStatus::Ready);
    assert_eq!(ready_attested.dataset.id, ready.dataset_id);
    assert_eq!(ready_attested.dataset.storage_path, "curated/manual/ready");
    let warning_attested = attest_recommendation_input(
        &worker,
        warning.job_id,
        warning.owner_id,
        warning.payload.clone(),
    )
    .await
    .expect("WARNING accepted");
    assert_eq!(
        warning_attested.dataset.status,
        AttestedDatasetStatus::Warning
    );

    let mut missing_run = ready.clone();
    missing_run.payload.run_id = Uuid::new_v4();
    assert_eq!(
        attest_error(&worker, &missing_run).await,
        RecommendationInputError::NotFound
    );
    let mut foreign_run = ready.clone();
    foreign_run.payload.run_id = foreign.run_id;
    assert_eq!(
        attest_error(&worker, &foreign_run).await,
        RecommendationInputError::NotFound
    );
    let mut foreign_owner = ready.clone();
    foreign_owner.owner_id = foreign.owner_id;
    assert_eq!(
        attest_error(&worker, &foreign_owner).await,
        RecommendationInputError::NotFound
    );

    let mut wrong_job = ready.clone();
    wrong_job.job_id = Uuid::new_v4();
    assert_eq!(
        attest_error(&worker, &wrong_job).await.class(),
        ErrorClass::Integrity
    );
    let mut wrong_config = ready.clone();
    wrong_config.payload.strategy_config_id = Uuid::new_v4();
    assert_eq!(
        attest_error(&worker, &wrong_config).await.class(),
        ErrorClass::Integrity
    );
    let mut wrong_as_of = ready.clone();
    wrong_as_of.payload.as_of = NaiveDate::from_ymd_opt(2026, 8, 12).unwrap();
    assert_eq!(
        attest_error(&worker, &wrong_as_of).await.class(),
        ErrorClass::Integrity
    );
    let mut wrong_dataset = ready.clone();
    wrong_dataset.payload.dataset.id = warning.dataset_id;
    wrong_dataset.payload.dataset.dataset_id = warning.payload.dataset.dataset_id.clone();
    wrong_dataset.payload.dataset.version = warning.payload.dataset.version.clone();
    assert_eq!(
        attest_error(&worker, &wrong_dataset).await.class(),
        ErrorClass::Integrity
    );
    let mut wrong_logical_id = ready.clone();
    wrong_logical_id.payload.dataset.dataset_id = "other-logical-dataset".into();
    assert_eq!(
        attest_error(&worker, &wrong_logical_id).await.class(),
        ErrorClass::Integrity
    );
    let mut wrong_version = ready.clone();
    wrong_version.payload.dataset.version = "other-version".into();
    assert_eq!(
        attest_error(&worker, &wrong_version).await.class(),
        ErrorClass::Integrity
    );
    let mut wrong_hash = ready.clone();
    wrong_hash.payload.dataset.manifest_sha256 = "c".repeat(64);
    assert_eq!(
        attest_error(&worker, &wrong_hash).await.class(),
        ErrorClass::Integrity
    );

    assert_eq!(
        attest_error(&worker, &blocked).await.class(),
        ErrorClass::DataBlocked
    );
    let mut missing_dataset = ready.clone();
    missing_dataset.payload.dataset.id = Uuid::new_v4();
    assert_eq!(
        attest_error(&worker, &missing_dataset).await.class(),
        ErrorClass::DataBlocked
    );

    sqlx::query("UPDATE user_strategy_configs SET is_active = false WHERE id = $1")
        .bind(ready.config_id)
        .execute(&db.pool)
        .await
        .expect("deactivate config");
    assert_eq!(
        attest_error(&worker, &ready).await,
        RecommendationInputError::NotFound
    );
    sqlx::query("UPDATE user_strategy_configs SET is_active = true WHERE id = $1")
        .bind(ready.config_id)
        .execute(&db.pool)
        .await
        .expect("reactivate config");

    let mut foreign_config = warning.clone();
    foreign_config.payload.strategy_config_id = foreign.config_id;
    sqlx::query("UPDATE recommendation_runs SET strategy_config_id = $1 WHERE id = $2")
        .bind(foreign.config_id)
        .bind(warning.run_id)
        .execute(&db.pool)
        .await
        .expect("seed foreign config reference");
    sqlx::query("UPDATE jobs SET payload_json = $1 WHERE id = $2")
        .bind(serde_json::to_value(&foreign_config.payload).unwrap())
        .bind(warning.job_id)
        .execute(&db.pool)
        .await
        .expect("seed foreign config payload");
    assert_eq!(
        attest_error(&worker, &foreign_config).await,
        RecommendationInputError::NotFound
    );

    sqlx::query("UPDATE recommendation_runs SET status = 'FAILED' WHERE id = $1")
        .bind(foreign.run_id)
        .execute(&db.pool)
        .await
        .expect("make run non-pending");
    assert_eq!(
        attest_error(&worker, &foreign).await.class(),
        ErrorClass::Integrity
    );

    worker.close().await;
    db.drop_db().await;
}
