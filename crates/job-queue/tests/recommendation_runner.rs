mod common;
#[path = "../../../tests/support/curated_fixture.rs"]
mod curated_fixture;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use chrono::NaiveDate;
use curated_fixture::attest_curated_artifacts;
use domain::{BatchId, ContentHash, DatasetId, InstrumentId, UtcTimestamp};
use job_queue::recommendation::child::TargetChildPaths;
use job_queue::recommendation::input::{DatasetPin, RecommendationPayload};
use job_queue::recommendation::publish::PublicationError;
use job_queue::recommendation::{
    RecommendationOutcome, RecommendationRunnerConfig, RecommendationRunnerConfigError,
    RecommendationRunnerPaths, run_once,
};
use job_queue::{JobQueue, JobStatus, QueueConfig, SubmitJob};
use serde_json::json;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tempfile::TempDir;
use uuid::Uuid;

use market_data::curate::SourceBatchRef;
use market_data::curate::schema::{read_adjusted_bars, read_bars, write_adjusted_bars, write_bars};
use market_data::{Capability, CurateStore, DatasetManifest, dataset_manifest_hash};

use common::ScratchDb;

const MEMBERS: [&str; 11] = [
    "069500.KRX",
    "102110.KRX",
    "229200.KRX",
    "143850.KRX",
    "133690.KRX",
    "195930.KRX",
    "192090.KRX",
    "148070.KRX",
    "114260.KRX",
    "153130.KRX",
    "132030.KRX",
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn uv_bin() -> Option<PathBuf> {
    let output = if cfg!(windows) {
        Command::new("where.exe").arg("uv.exe").output().ok()?
    } else {
        Command::new("which").arg("uv").output().ok()?
    };
    output
        .status
        .success()
        .then_some(output.stdout)
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .and_then(|text| text.lines().next().map(str::trim).map(str::to_owned))
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn copy_python_project(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() == ".venv" || entry.file_name() == "__pycache__" {
            continue;
        }
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_python_project(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn clone_symbol(store_root: &Path, source: &str, destination: &str) {
    let market = store_root.join("curated/bars/market=kr");
    let target = market.join(format!("symbol={destination}"));
    copy_dir(&market.join(format!("symbol={source}")), &target);
    let store = CurateStore::new(store_root);
    let destination_id = InstrumentId::parse(destination).unwrap();
    for year in std::fs::read_dir(&target).unwrap().flatten() {
        let Some(year) = year
            .file_name()
            .to_string_lossy()
            .strip_prefix("year=")
            .and_then(|value| value.parse::<i32>().ok())
        else {
            continue;
        };
        let bars_path = store.bars_path("kr", destination, year, 2);
        if bars_path.is_file() {
            let mut rows = read_bars(&bars_path).unwrap();
            rows.iter_mut()
                .for_each(|row| row.instrument_id = destination_id.clone());
            write_bars(&bars_path, &rows).unwrap();
        }
        for adjusted in [
            store.adjusted_bars_path("kr", destination, year, 2),
            store.total_return_bars_path("kr", destination, year, 2),
        ] {
            if adjusted.is_file() {
                let mut rows = read_adjusted_bars(&adjusted).unwrap();
                rows.iter_mut()
                    .for_each(|row| row.instrument_id = destination_id.clone());
                write_adjusted_bars(&adjusted, &rows).unwrap();
            }
        }
    }
}

struct QaDataset {
    _temp: TempDir,
    root: PathBuf,
    hash: String,
}

async fn enqueue_run(
    pool: &PgPool,
    queue: &JobQueue,
    owner_id: Uuid,
    config_id: Uuid,
    dataset_version_id: Uuid,
    manifest_hash: &str,
    key: &str,
) -> (Uuid, Uuid) {
    let run_id = Uuid::new_v4();
    let payload = RecommendationPayload {
        run_id,
        strategy_config_id: config_id,
        as_of: NaiveDate::from_ymd_opt(2021, 1, 29).unwrap(),
        dataset: DatasetPin {
            id: dataset_version_id,
            dataset_id: "krx_eod_bars".into(),
            version: "qa-v2".into(),
            curated_version: 2,
            manifest_sha256: manifest_hash.to_owned(),
        },
    };
    let job = queue
        .submit(SubmitJob {
            owner_user_id: owner_id,
            job_type: "recommendation".into(),
            payload: serde_json::to_value(&payload).unwrap(),
            priority: 0,
            idempotency_key: Some(format!("{key}-{run_id}")),
            max_attempts: 2,
            available_at: None,
        })
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO recommendation_runs \
         (id, owner_user_id, strategy_config_id, as_of, status, job_id, trigger_kind, dataset_version_id, dataset_manifest_sha256) \
         VALUES ($1, $2, $3, $4, 'PENDING', $5, 'MANUAL', $6, $7)",
    )
    .bind(run_id)
    .bind(owner_id)
    .bind(config_id)
    .bind(payload.as_of)
    .bind(job.id)
    .bind(dataset_version_id)
    .bind(manifest_hash)
    .execute(pool)
    .await
    .unwrap();
    (run_id, job.id)
}

fn qa_dataset() -> QaDataset {
    let repo = repo_root();
    let temp = tempfile::Builder::new()
        .prefix(".recommendation-runner-qa-")
        .tempdir_in(&repo)
        .unwrap();
    let generated = temp.path().join("phase0");
    let output = Command::new(std::env::var_os("PYTHON").unwrap_or_else(|| "python".into()))
        .current_dir(&repo)
        .arg(repo.join("scripts/ci/prepare_phase0.py"))
        .arg("--root")
        .arg(&generated)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "QA generator failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let root = generated;
    let market = root.join("curated/bars/market=kr");
    for member in MEMBERS {
        if !market.join(format!("symbol={member}")).exists() {
            clone_symbol(&root, "069500.KRX", member);
        }
    }
    let store = CurateStore::new(&root);
    let manifest = DatasetManifest {
        dataset_id: DatasetId::parse("krx_eod_bars").unwrap(),
        version: 2,
        capability: Capability::PriceReturnOnly,
        created_at: UtcTimestamp::parse_rfc3339("2021-01-29T06:30:00Z").unwrap(),
        source_batches: Vec::new(),
        artifacts: attest_curated_artifacts(&store, 2),
        bar_count: 11 * 260,
        action_count: 0,
        content_hash: ContentHash::from_bytes(b"placeholder"),
    };
    let manifest = DatasetManifest {
        content_hash: dataset_manifest_hash(&manifest).unwrap(),
        ..manifest
    };
    store.write_dataset_manifest(&manifest).unwrap();
    let hash = manifest
        .content_hash
        .as_str()
        .strip_prefix("sha256:")
        .unwrap()
        .to_owned();
    QaDataset {
        _temp: temp,
        root,
        hash,
    }
}

#[test]
fn runner_config_rejects_unsafe_heartbeat_and_timeouts() {
    let error = RecommendationRunnerConfig::new(
        Duration::from_secs(10),
        Duration::from_secs(10),
        Duration::from_secs(30),
    )
    .expect_err("heartbeat must leave time before lease expiry");
    assert_eq!(
        error,
        RecommendationRunnerConfigError::HeartbeatNotBeforeLease
    );

    let error = RecommendationRunnerConfig::new(
        Duration::ZERO,
        Duration::from_secs(10),
        Duration::from_secs(30),
    )
    .expect_err("zero heartbeat is invalid");
    assert_eq!(error, RecommendationRunnerConfigError::ZeroDuration);

    let error = RecommendationRunnerConfig::new(
        Duration::from_secs(1),
        Duration::from_secs(10),
        Duration::ZERO,
    )
    .expect_err("zero child deadline is invalid");
    assert_eq!(error, RecommendationRunnerConfigError::ZeroDuration);
}

#[test]
fn runner_config_accepts_a_bounded_heartbeat() {
    let config = RecommendationRunnerConfig::new(
        Duration::from_secs(5),
        Duration::from_secs(30),
        Duration::from_secs(120),
    )
    .unwrap();
    assert_eq!(config.heartbeat_interval(), Duration::from_secs(5));
    assert_eq!(config.lease(), Duration::from_secs(30));
    assert_eq!(config.child_timeout(), Duration::from_secs(120));
    assert!(!config.is_production());
    assert!(config.with_production(true).is_production());
}

#[tokio::test]
async fn runner_refuses_a_lease_configuration_that_differs_from_the_queue() {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://worker:unused@127.0.0.1:1/unused")
        .unwrap();
    let queue = JobQueue::new(
        pool.clone(),
        None,
        QueueConfig {
            lease: Duration::from_secs(1),
            backoff_base: Duration::from_secs(1),
        },
    );
    let config = RecommendationRunnerConfig::new(
        Duration::from_millis(100),
        Duration::from_secs(2),
        Duration::from_secs(10),
    )
    .unwrap();
    let error = run_once(
        &pool,
        &queue,
        "lease-mismatch",
        &RecommendationRunnerPaths {
            data_root: PathBuf::from("unused"),
            universe_manifest: PathBuf::from("unused"),
            child: TargetChildPaths {
                uv_bin: PathBuf::from("unused"),
                repo_root: PathBuf::from("unused"),
                temp_root: PathBuf::from("unused"),
            },
        },
        &config,
    )
    .await
    .expect_err("mismatched leases must fail before claiming");
    assert!(error.to_string().contains("unavailable"));
}

#[test]
fn revoked_publication_entitlement_is_a_nonretryable_data_block() {
    let error = PublicationError::EntitlementDenied;
    assert_eq!(error.class(), job_queue::ErrorClass::DataBlocked);
    assert_eq!(error.code(), "DATA_ENTITLEMENT_REQUIRED");
}

#[tokio::test]
async fn typed_runner_leaves_backtests_queued() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let owner_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) VALUES ('https://issuer.test', $1, $2) RETURNING id",
    )
    .bind(format!("runner-mixed-{}", Uuid::new_v4()))
    .bind(format!("runner-mixed-{}@example.test", Uuid::new_v4()))
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let app_queue = JobQueue::new(db.pool.clone(), None, QueueConfig::default());
    let backtest = app_queue
        .submit(SubmitJob {
            owner_user_id: owner_id,
            job_type: "backtest".into(),
            payload: json!({}),
            priority: 0,
            idempotency_key: Some(format!("runner-mixed-{}", Uuid::new_v4())),
            max_attempts: 1,
            available_at: None,
        })
        .await
        .unwrap();
    let worker = PgPool::connect(&db.role_url("worker")).await.unwrap();
    let config = RecommendationRunnerConfig::new(
        Duration::from_secs(5),
        Duration::from_secs(30),
        Duration::from_secs(60),
    )
    .unwrap();
    let paths = RecommendationRunnerPaths {
        data_root: PathBuf::from("unused"),
        universe_manifest: PathBuf::from("unused"),
        child: TargetChildPaths {
            uv_bin: PathBuf::from("unused"),
            repo_root: PathBuf::from("unused"),
            temp_root: PathBuf::from("unused"),
        },
    };
    let queue = JobQueue::new(
        worker.clone(),
        None,
        QueueConfig {
            lease: config.lease(),
            backoff_base: Duration::from_millis(1),
        },
    );

    assert_eq!(
        run_once(&worker, &queue, "recommendation-test", &paths, &config)
            .await
            .unwrap(),
        RecommendationOutcome::Idle
    );
    assert_eq!(
        app_queue.get_by_id(backtest.id).await.unwrap().status,
        JobStatus::Queued
    );

    worker.close().await;
    db.drop_db().await;
}

#[tokio::test]
async fn malformed_payload_fails_job_and_matching_run_without_leaking_payload() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let owner_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) VALUES ('https://issuer.test', $1, $2) RETURNING id",
    )
    .bind(format!("runner-malformed-{}", Uuid::new_v4()))
    .bind(format!("runner-malformed-{}@example.test", Uuid::new_v4()))
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let app_queue = JobQueue::new(db.pool.clone(), None, QueueConfig::default());
    let secret_marker = "DO_NOT_PERSIST_THIS_PAYLOAD";
    let job = app_queue
        .submit(SubmitJob {
            owner_user_id: owner_id,
            job_type: "recommendation".into(),
            payload: json!({"unexpected": secret_marker}),
            priority: 0,
            idempotency_key: Some(format!("runner-malformed-{}", Uuid::new_v4())),
            max_attempts: 2,
            available_at: None,
        })
        .await
        .unwrap();
    let run_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO recommendation_runs (id, owner_user_id, as_of, status, job_id, trigger_kind) \
         VALUES ($1, $2, DATE '2026-08-11', 'PENDING', $3, 'MANUAL')",
    )
    .bind(run_id)
    .bind(owner_id)
    .bind(job.id)
    .execute(&db.pool)
    .await
    .unwrap();
    let worker = PgPool::connect(&db.role_url("worker")).await.unwrap();
    let config = RecommendationRunnerConfig::new(
        Duration::from_millis(10),
        Duration::from_secs(5),
        Duration::from_secs(1),
    )
    .unwrap();
    let paths = RecommendationRunnerPaths {
        data_root: PathBuf::from("unused"),
        universe_manifest: PathBuf::from("unused"),
        child: TargetChildPaths {
            uv_bin: PathBuf::from("unused"),
            repo_root: PathBuf::from("unused"),
            temp_root: PathBuf::from("unused"),
        },
    };
    let queue = JobQueue::new(
        worker.clone(),
        None,
        QueueConfig {
            lease: config.lease(),
            backoff_base: Duration::from_millis(1),
        },
    );

    let outcome = run_once(&worker, &queue, "recommendation-test", &paths, &config)
        .await
        .unwrap();
    assert_eq!(
        outcome,
        RecommendationOutcome::Failed {
            job_id: job.id,
            code: "RECOMMENDATION_INPUT_MALFORMED".into(),
        }
    );
    let row: (String, Option<String>, Option<String>) =
        sqlx::query_as("SELECT status, error_code, error_message FROM jobs WHERE id = $1")
            .bind(job.id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(row.0, "FAILED");
    assert_eq!(row.1.as_deref(), Some("RECOMMENDATION_INPUT_MALFORMED"));
    assert!(!row.2.as_deref().unwrap_or_default().contains(secret_marker));
    let run: (String, serde_json::Value) =
        sqlx::query_as("SELECT status, summary_json FROM recommendation_runs WHERE id = $1")
            .bind(run_id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(run.0, "FAILED");
    assert_eq!(run.1["code"], "RECOMMENDATION_INPUT_MALFORMED");
    assert!(!run.1.to_string().contains(secret_marker));

    worker.close().await;
    db.drop_db().await;
}

#[tokio::test]
async fn entitlement_revoked_after_enqueue_blocks_linked_run_without_outputs() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let owner_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) VALUES ('https://issuer.test', $1, $2) RETURNING id",
    )
    .bind(format!("runner-revoked-{}", Uuid::new_v4()))
    .bind(format!("runner-revoked-{}@example.test", Uuid::new_v4()))
    .fetch_one(&db.pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO strategies (id, display_name, state) VALUES ('buy_and_hold', 'Buy and hold', 'Paper')")
        .execute(&db.pool)
        .await
        .unwrap();
    let config_id: Uuid = sqlx::query_scalar(
        "INSERT INTO user_strategy_configs (owner_user_id, strategy_id, strategy_version, config_json) \
         VALUES ($1, 'buy_and_hold', '1.0.0', $2) RETURNING id",
    )
    .bind(owner_id)
    .bind(json!({
        "benchmark_instrument": "069500.KRX",
        "target_weight": 1.0,
        "rebalance_cadence": "none"
    }))
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let dataset_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO dataset_versions (id, dataset_id, version, status, manifest_sha256, storage_path) \
         VALUES ($1, 'krx_eod_bars', 'qa-v2', 'READY', repeat('c', 64), 'never-read')",
    )
    .bind(dataset_id)
    .execute(&db.pool)
    .await
    .unwrap();
    let entitlement_id: Uuid = sqlx::query_scalar(
        "INSERT INTO data_entitlements \
         (contract_document_sha256, contract_reference, status, covered_datasets, covered_uses, effective_from, effective_until, managed_by) \
         VALUES (repeat('e', 64), 'vault://qa/revoked', 'ACTIVE', '[\"krx_eod_bars\"]', '[\"recommendation\"]', DATE '2020-01-01', DATE '2030-12-31', $1) \
         RETURNING id",
    )
    .bind(owner_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let run_id = Uuid::new_v4();
    let payload = RecommendationPayload {
        run_id,
        strategy_config_id: config_id,
        as_of: NaiveDate::from_ymd_opt(2026, 8, 11).unwrap(),
        dataset: DatasetPin {
            id: dataset_id,
            dataset_id: "krx_eod_bars".into(),
            version: "qa-v2".into(),
            curated_version: 2,
            manifest_sha256: "c".repeat(64),
        },
    };
    let app_queue = JobQueue::new(db.pool.clone(), None, QueueConfig::default());
    let job = app_queue
        .submit(SubmitJob {
            owner_user_id: owner_id,
            job_type: "recommendation".into(),
            payload: serde_json::to_value(&payload).unwrap(),
            priority: 0,
            idempotency_key: Some(format!("runner-revoked-{run_id}")),
            max_attempts: 2,
            available_at: None,
        })
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO recommendation_runs \
         (id, owner_user_id, strategy_config_id, as_of, status, job_id, trigger_kind, dataset_version_id, dataset_manifest_sha256) \
         VALUES ($1, $2, $3, $4, 'PENDING', $5, 'MANUAL', $6, repeat('c', 64))",
    )
    .bind(run_id)
    .bind(owner_id)
    .bind(config_id)
    .bind(payload.as_of)
    .bind(job.id)
    .bind(dataset_id)
    .execute(&db.pool)
    .await
    .unwrap();
    let queued_state: (String, String) = sqlx::query_as(
        "SELECT j.status, r.status FROM jobs j \
         JOIN recommendation_runs r ON r.job_id = j.id WHERE j.id = $1",
    )
    .bind(job.id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(queued_state, ("QUEUED".into(), "PENDING".into()));

    // The supervisor fixture represents the privileged contract-admin control
    // plane; the production app and worker roles cannot mutate entitlement rows.
    sqlx::query("UPDATE data_entitlements SET status = 'REVOKED' WHERE id = $1")
        .bind(entitlement_id)
        .execute(&db.pool)
        .await
        .unwrap();
    let worker = PgPool::connect(&db.role_url("worker")).await.unwrap();
    let config = RecommendationRunnerConfig::new(
        Duration::from_millis(10),
        Duration::from_secs(5),
        Duration::from_secs(1),
    )
    .unwrap();
    let paths = RecommendationRunnerPaths {
        data_root: PathBuf::from("unused"),
        universe_manifest: PathBuf::from("unused"),
        child: TargetChildPaths {
            uv_bin: PathBuf::from("unused"),
            repo_root: PathBuf::from("unused"),
            temp_root: PathBuf::from("unused"),
        },
    };
    let queue = JobQueue::new(
        worker.clone(),
        None,
        QueueConfig {
            lease: config.lease(),
            backoff_base: Duration::from_millis(1),
        },
    );
    assert_eq!(
        run_once(&worker, &queue, "recommendation-test", &paths, &config)
            .await
            .unwrap(),
        RecommendationOutcome::Blocked {
            job_id: job.id,
            code: "DATA_ENTITLEMENT_REQUIRED".into(),
        }
    );
    let job_state: (String, Option<String>, i32, i32, String, Option<String>) = sqlx::query_as(
        "SELECT j.status, j.error_code, j.attempt_count, j.max_attempts, \
                a.outcome, a.error_code \
         FROM jobs j JOIN job_attempts a ON a.job_id = j.id \
         WHERE j.id = $1 AND a.attempt_no = 1",
    )
    .bind(job.id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        job_state,
        (
            "FAILED".into(),
            Some("DATA_ENTITLEMENT_REQUIRED".into()),
            1,
            2,
            "FAILED".into(),
            Some("DATA_ENTITLEMENT_REQUIRED".into()),
        )
    );
    let status: String = sqlx::query_scalar("SELECT status FROM recommendation_runs WHERE id = $1")
        .bind(run_id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(status, "BLOCKED");
    let counts: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM recommendation_items WHERE recommendation_run_id = $1), \
                (SELECT count(*) FROM target_portfolios WHERE recommendation_run_id = $1)",
    )
    .bind(run_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(counts, (0, 0));

    sqlx::query("UPDATE data_entitlements SET status = 'ACTIVE' WHERE contract_reference = 'vault://qa/revoked'")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE dataset_versions SET status = 'BLOCKED' WHERE id = $1")
        .bind(dataset_id)
        .execute(&db.pool)
        .await
        .unwrap();
    let (blocked_run_id, blocked_job_id) = enqueue_run(
        &db.pool,
        &app_queue,
        owner_id,
        config_id,
        dataset_id,
        &"c".repeat(64),
        "runner-blocked-dataset",
    )
    .await;
    assert_eq!(
        run_once(&worker, &queue, "recommendation-test", &paths, &config)
            .await
            .unwrap(),
        RecommendationOutcome::Blocked {
            job_id: blocked_job_id,
            code: "RECOMMENDATION_DATA_BLOCKED".into(),
        }
    );
    let blocked_status: String =
        sqlx::query_scalar("SELECT status FROM recommendation_runs WHERE id = $1")
            .bind(blocked_run_id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(blocked_status, "BLOCKED");

    sqlx::query("UPDATE dataset_versions SET status = 'READY' WHERE id = $1")
        .bind(dataset_id)
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE user_strategy_configs SET is_active = false WHERE id = $1")
        .bind(config_id)
        .execute(&db.pool)
        .await
        .unwrap();
    let (invalid_run_id, invalid_job_id) = enqueue_run(
        &db.pool,
        &app_queue,
        owner_id,
        config_id,
        dataset_id,
        &"c".repeat(64),
        "runner-inactive-config",
    )
    .await;
    assert_eq!(
        run_once(&worker, &queue, "recommendation-test", &paths, &config)
            .await
            .unwrap(),
        RecommendationOutcome::Failed {
            job_id: invalid_job_id,
            code: "RECOMMENDATION_INPUT_NOT_FOUND".into(),
        }
    );
    let invalid_status: String =
        sqlx::query_scalar("SELECT status FROM recommendation_runs WHERE id = $1")
            .bind(invalid_run_id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(invalid_status, "FAILED");

    worker.close().await;
    db.drop_db().await;
}

#[tokio::test]
async fn publication_entitlement_lock_fences_concurrent_revocation_for_worker_role() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let owner_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) VALUES ('https://issuer.test', $1, $2) RETURNING id",
    )
    .bind(format!("runner-entitlement-lock-{}", Uuid::new_v4()))
    .bind(format!("runner-entitlement-lock-{}@example.test", Uuid::new_v4()))
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let entitlement_id: Uuid = sqlx::query_scalar(
        "INSERT INTO data_entitlements \
         (contract_document_sha256, contract_reference, status, covered_datasets, covered_uses, effective_from, effective_until, managed_by) \
         VALUES (repeat('e', 64), 'vault://qa/lock', 'ACTIVE', '[\"krx_eod_bars\"]', '[\"recommendation\"]', DATE '2020-01-01', DATE '2030-12-31', $1) RETURNING id",
    )
    .bind(owner_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let worker = PgPool::connect(&db.role_url("worker")).await.unwrap();
    let denied = sqlx::query("UPDATE data_entitlements SET status = 'REVOKED' WHERE id = $1")
        .bind(entitlement_id)
        .execute(&worker)
        .await
        .expect_err("worker remains SELECT-only on entitlement metadata");
    assert_eq!(
        denied
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("42501")
    );

    let mut publication = worker.begin().await.unwrap();
    let allowed: bool = sqlx::query_scalar(
        "SELECT public.lock_recommendation_entitlement($1, 'krx_eod_bars', DATE '2026-08-11')",
    )
    .bind(owner_id)
    .fetch_one(&mut *publication)
    .await
    .unwrap();
    assert!(allowed);
    let mut revocation = db.pool.begin().await.unwrap();
    sqlx::query("SET LOCAL lock_timeout = '100ms'")
        .execute(&mut *revocation)
        .await
        .unwrap();
    let blocked = sqlx::query("UPDATE data_entitlements SET status = 'REVOKED' WHERE id = $1")
        .bind(entitlement_id)
        .execute(&mut *revocation)
        .await
        .expect_err("revocation must wait for the publication transaction");
    assert_eq!(
        blocked
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("55P03")
    );
    revocation.rollback().await.unwrap();
    publication.commit().await.unwrap();
    sqlx::query("UPDATE data_entitlements SET status = 'REVOKED' WHERE id = $1")
        .bind(entitlement_id)
        .execute(&db.pool)
        .await
        .unwrap();

    worker.close().await;
    db.drop_db().await;
}

#[tokio::test]
async fn orphan_sweep_exhaustion_synchronizes_the_linked_run() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let owner_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) VALUES ('https://issuer.test', $1, $2) RETURNING id",
    )
    .bind(format!("runner-sweep-{}", Uuid::new_v4()))
    .bind(format!("runner-sweep-{}@example.test", Uuid::new_v4()))
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let app_queue = JobQueue::new(db.pool.clone(), None, QueueConfig::default());
    let job = app_queue
        .submit(SubmitJob {
            owner_user_id: owner_id,
            job_type: "recommendation".into(),
            payload: json!({"invalid": true}),
            priority: 0,
            idempotency_key: Some(format!("runner-sweep-{}", Uuid::new_v4())),
            max_attempts: 1,
            available_at: None,
        })
        .await
        .unwrap();
    let run_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO recommendation_runs (id, owner_user_id, as_of, status, job_id, trigger_kind) \
         VALUES ($1, $2, DATE '2026-08-11', 'PENDING', $3, 'MANUAL')",
    )
    .bind(run_id)
    .bind(owner_id)
    .bind(job.id)
    .execute(&db.pool)
    .await
    .unwrap();
    let worker = PgPool::connect(&db.role_url("worker")).await.unwrap();
    let queue = JobQueue::new(
        worker.clone(),
        None,
        QueueConfig {
            lease: Duration::from_millis(20),
            backoff_base: Duration::from_millis(1),
        },
    );
    queue
        .claim_next_for("recommendation-crashed", "recommendation")
        .await
        .unwrap()
        .unwrap();
    sqlx::query("UPDATE jobs SET locked_at = now() - interval '1 second' WHERE id = $1")
        .bind(job.id)
        .execute(&db.pool)
        .await
        .unwrap();
    let report = queue.sweep().await.unwrap();
    assert_eq!(report.jobs_failed, 1);
    let run: (String, serde_json::Value) =
        sqlx::query_as("SELECT status, summary_json FROM recommendation_runs WHERE id = $1")
            .bind(run_id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(run.0, "FAILED");
    assert_eq!(run.1["code"], "RECOMMENDATION_ATTEMPTS_EXHAUSTED");

    worker.close().await;
    db.drop_db().await;
}

#[tokio::test]
async fn real_worker_and_uv_publish_all_five_shipped_strategies() {
    let Some(uv) = uv_bin() else {
        eprintln!("skipping: uv unavailable");
        return;
    };
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let qa = qa_dataset();
    let repo = repo_root();
    let child_repo = tempfile::Builder::new()
        .prefix(".recommendation-runner-repo-")
        .tempdir_in(&repo)
        .unwrap();
    copy_python_project(&repo.join("nt"), &child_repo.path().join("nt"));
    let sync = Command::new(&uv)
        .arg("sync")
        .arg("--project")
        .arg(child_repo.path().join("nt"))
        .arg("--locked")
        .output()
        .unwrap();
    assert!(
        sync.status.success(),
        "isolated uv sync failed: {}",
        String::from_utf8_lossy(&sync.stderr)
    );
    let child_temp = tempfile::Builder::new()
        .prefix(".recommendation-runner-child-")
        .tempdir_in(&repo)
        .unwrap();
    let owner_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) VALUES ('https://issuer.test', $1, $2) RETURNING id",
    )
    .bind(format!("runner-five-{}", Uuid::new_v4()))
    .bind(format!("runner-five-{}@example.test", Uuid::new_v4()))
    .fetch_one(&db.pool)
    .await
    .unwrap();
    for member in MEMBERS {
        sqlx::query(
            "INSERT INTO instruments (id, symbol, venue, currency) VALUES ($1, $2, 'KRX', 'KRW')",
        )
        .bind(member)
        .bind(member.trim_end_matches(".KRX"))
        .execute(&db.pool)
        .await
        .unwrap();
    }
    let universe = job_queue::recommendation::compute::AttestedUniverse::from_manifest_yaml(
        include_str!("../../../configs/universes/kr-etf-core-v1.yaml"),
    )
    .unwrap();
    sqlx::query(
        "INSERT INTO universe_snapshots \
         (snapshot_id, universe_manifest_sha256, instruments_json, published_by) \
         VALUES ($1, repeat('d', 64), $2, $3)",
    )
    .bind(universe.snapshot_id())
    .bind(json!(universe.members()))
    .bind(owner_id)
    .execute(&db.pool)
    .await
    .unwrap();
    let dataset_version_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO dataset_versions \
         (id, dataset_id, version, status, manifest_sha256, storage_path) \
         VALUES ($1, 'krx_eod_bars', 'qa-v2', 'READY', $2, $3)",
    )
    .bind(dataset_version_id)
    .bind(&qa.hash)
    .bind(qa.root.to_string_lossy().as_ref())
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO data_entitlements \
         (contract_document_sha256, contract_reference, status, covered_datasets, covered_uses, effective_from, effective_until, managed_by) \
         VALUES (repeat('e', 64), 'vault://qa/five', 'ACTIVE', '[\"krx_eod_bars\"]', '[\"recommendation\"]', DATE '2020-01-01', DATE '2030-12-31', $1)",
    )
    .bind(owner_id)
    .execute(&db.pool)
    .await
    .unwrap();
    let cases = [
        (
            "buy_and_hold",
            json!({
                "benchmark_instrument": "069500.KRX",
                "target_weight": 1.0,
                "rebalance_cadence": "none"
            }),
        ),
        (
            "trend_following",
            json!({
                "benchmark_instrument": "069500.KRX",
                "fast_ma": 50,
                "slow_ma": 200
            }),
        ),
        (
            "relative_momentum",
            json!({"top_n": 3, "lookback_months": 12}),
        ),
        (
            "dual_momentum",
            json!({"absolute_threshold": 0.0, "lookback_months": 6}),
        ),
        (
            "inverse_volatility",
            json!({"vol_window": 60, "max_weight": 0.3}),
        ),
    ];
    for (strategy, _) in &cases {
        sqlx::query("INSERT INTO strategies (id, display_name, state) VALUES ($1, $1, 'Paper')")
            .bind(strategy)
            .execute(&db.pool)
            .await
            .unwrap();
    }
    let worker = PgPool::connect(&db.role_url("worker")).await.unwrap();
    let config = RecommendationRunnerConfig::new(
        Duration::from_millis(100),
        Duration::from_secs(10),
        Duration::from_secs(30),
    )
    .unwrap();
    let paths = RecommendationRunnerPaths {
        data_root: qa.root.clone(),
        universe_manifest: repo.join("configs/universes/kr-etf-core-v1.yaml"),
        child: TargetChildPaths {
            uv_bin: uv,
            repo_root: child_repo.path().to_path_buf(),
            temp_root: child_temp.path().to_path_buf(),
        },
    };
    let queue = JobQueue::new(
        worker.clone(),
        None,
        QueueConfig {
            lease: config.lease(),
            backoff_base: Duration::from_millis(1),
        },
    );
    let app_queue = JobQueue::new(db.pool.clone(), None, QueueConfig::default());

    let health_root = tempfile::tempdir().unwrap();
    let health_path = health_root.path().join("health.json");
    let worker_url = db.role_url("worker");
    let mut daemon = Command::new(env!("CARGO_BIN_EXE_recommendation-runner"))
        .env("APP_ENV", "qa")
        .env("DATABASE_URL", &worker_url)
        .env("RECOMMENDATION_HEALTH_STATE_PATH", &health_path)
        .args(["--worker-id", "recommendation-health-smoke", "--repo-root"])
        .arg(child_repo.path())
        .arg("--data-root")
        .arg(&qa.root)
        .arg("--universe-manifest")
        .arg(repo.join("configs/universes/kr-etf-core-v1.yaml"))
        .arg("--uv-bin")
        .arg(&paths.child.uv_bin)
        .arg("--temp-root")
        .arg(child_temp.path())
        .args([
            "--poll-ms",
            "60000",
            "--sweep-ms",
            "60000",
            "--heartbeat-ms",
            "100",
            "--lease-ms",
            "10000",
            "--backoff-ms",
            "1",
            "--child-timeout-ms",
            "30000",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !health_path.is_file() && std::time::Instant::now() < deadline {
        assert!(
            daemon.try_wait().unwrap().is_none(),
            "health daemon exited early"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(health_path.is_file(), "health daemon did not publish state");
    let health_output = Command::new(env!("CARGO_BIN_EXE_recommendation-runner"))
        .arg("healthcheck")
        .env("APP_ENV", "qa")
        .env("DATABASE_URL", &worker_url)
        .env("RECOMMENDATION_HEALTH_STATE_PATH", &health_path)
        .output()
        .unwrap();
    let _ = daemon.kill();
    let _ = daemon.wait();
    assert!(
        health_output.status.success(),
        "healthcheck failed: {}",
        String::from_utf8_lossy(&health_output.stderr)
    );
    let health: serde_json::Value = serde_json::from_slice(&health_output.stdout).unwrap();
    assert_eq!(health["status"], "ok");
    assert_eq!(health["database"], "reachable");
    assert!(health["process"]["pid"].is_number());
    assert!(health.get("last_schedule").is_some());
    assert!(health.get("queue_age_seconds").is_some());
    assert_eq!(health["blocked_recommendation_runs"], 0);
    let health_stdout = String::from_utf8_lossy(&health_output.stdout);
    let health_stderr = String::from_utf8_lossy(&health_output.stderr);
    assert!(!health_stdout.contains(&worker_url));
    assert!(!health_stderr.contains(&worker_url));

    for (strategy_id, parameters) in cases {
        let config_id: Uuid = sqlx::query_scalar(
            "INSERT INTO user_strategy_configs \
             (owner_user_id, strategy_id, strategy_version, config_json) \
             VALUES ($1, $2, '1.0.0', $3) RETURNING id",
        )
        .bind(owner_id)
        .bind(strategy_id)
        .bind(parameters)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        let run_id = Uuid::new_v4();
        let payload = RecommendationPayload {
            run_id,
            strategy_config_id: config_id,
            as_of: NaiveDate::from_ymd_opt(2021, 1, 29).unwrap(),
            dataset: DatasetPin {
                id: dataset_version_id,
                dataset_id: "krx_eod_bars".into(),
                version: "qa-v2".into(),
                curated_version: 2,
                manifest_sha256: qa.hash.clone(),
            },
        };
        let job = app_queue
            .submit(SubmitJob {
                owner_user_id: owner_id,
                job_type: "recommendation".into(),
                payload: serde_json::to_value(&payload).unwrap(),
                priority: 0,
                idempotency_key: Some(format!("runner-five-{run_id}")),
                max_attempts: 2,
                available_at: None,
            })
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO recommendation_runs \
             (id, owner_user_id, strategy_config_id, as_of, status, job_id, trigger_kind, dataset_version_id, dataset_manifest_sha256) \
             VALUES ($1, $2, $3, $4, 'PENDING', $5, 'MANUAL', $6, $7)",
        )
        .bind(run_id)
        .bind(owner_id)
        .bind(config_id)
        .bind(payload.as_of)
        .bind(job.id)
        .bind(dataset_version_id)
        .bind(&qa.hash)
        .execute(&db.pool)
        .await
        .unwrap();

        if strategy_id == "buy_and_hold" {
            let output = Command::new(env!("CARGO_BIN_EXE_recommendation-runner"))
                .env("APP_ENV", "qa")
                .env("DATABASE_URL", &worker_url)
                .args([
                    "--once",
                    "--worker-id",
                    "recommendation-smoke-cli",
                    "--repo-root",
                ])
                .arg(child_repo.path())
                .arg("--data-root")
                .arg(&qa.root)
                .arg("--universe-manifest")
                .arg(repo.join("configs/universes/kr-etf-core-v1.yaml"))
                .arg("--uv-bin")
                .arg(&paths.child.uv_bin)
                .arg("--temp-root")
                .arg(child_temp.path())
                .args([
                    "--poll-ms",
                    "10",
                    "--sweep-ms",
                    "1000",
                    "--heartbeat-ms",
                    "100",
                    "--lease-ms",
                    "10000",
                    "--backoff-ms",
                    "1",
                    "--child-timeout-ms",
                    "30000",
                ])
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                output.status.success(),
                "recommendation-runner --once failed: stdout={stdout} stderr={stderr}"
            );
            assert!(!stdout.contains(&worker_url));
            assert!(!stderr.contains(&worker_url));
        } else {
            let outcome = run_once(&worker, &queue, "recommendation-five", &paths, &config)
                .await
                .unwrap();
            assert_eq!(
                outcome,
                RecommendationOutcome::Succeeded {
                    job_id: job.id,
                    run_id,
                },
                "{strategy_id}"
            );
        }
        let state: (String, String, i64, i64) = sqlx::query_as(
            "SELECT r.status, j.status, \
                    (SELECT count(*) FROM recommendation_items WHERE recommendation_run_id = r.id), \
                    (SELECT count(*) FROM target_portfolios WHERE recommendation_run_id = r.id) \
             FROM recommendation_runs r JOIN jobs j ON j.id = r.job_id WHERE r.id = $1",
        )
        .bind(run_id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(state, ("SUCCEEDED".into(), "SUCCEEDED".into(), 11, 1));
    }

    let buy_hold_config_id: Uuid = sqlx::query_scalar(
        "INSERT INTO user_strategy_configs \
         (owner_user_id, strategy_id, strategy_version, config_json) \
         VALUES ($1, 'buy_and_hold', '1.0.0', $2) RETURNING id",
    )
    .bind(owner_id)
    .bind(json!({
        "benchmark_instrument": "069500.KRX",
        "target_weight": 1.0,
        "rebalance_cadence": "none"
    }))
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let mismatched_manifest = child_repo.path().join("mismatched-universe.yaml");
    std::fs::write(
        &mismatched_manifest,
        include_str!("../../../configs/universes/kr-etf-core-v1.yaml")
            .replace("132030.KRX", "999999.KRX"),
    )
    .unwrap();
    let mut mismatched_paths = paths.clone();
    mismatched_paths.universe_manifest = mismatched_manifest;
    let (mismatched_run_id, mismatched_job_id) = enqueue_run(
        &db.pool,
        &app_queue,
        owner_id,
        buy_hold_config_id,
        dataset_version_id,
        &qa.hash,
        "runner-universe-mismatch",
    )
    .await;
    assert_eq!(
        run_once(
            &worker,
            &queue,
            "recommendation-universe-mismatch",
            &mismatched_paths,
            &config,
        )
        .await
        .unwrap(),
        RecommendationOutcome::Blocked {
            job_id: mismatched_job_id,
            code: "RECOMMENDATION_UNIVERSE_INTEGRITY".into(),
        }
    );
    let mismatched_status: String =
        sqlx::query_scalar("SELECT status FROM recommendation_runs WHERE id = $1")
            .bind(mismatched_run_id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(mismatched_status, "BLOCKED");

    let trend_config_id: Uuid = sqlx::query_scalar(
        "INSERT INTO user_strategy_configs \
         (owner_user_id, strategy_id, strategy_version, config_json) \
         VALUES ($1, 'trend_following', '1.0.0', $2) RETURNING id",
    )
    .bind(owner_id)
    .bind(json!({
        "benchmark_instrument": "069500.KRX",
        "fast_ma": 50,
        "slow_ma": 500
    }))
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let (history_run_id, history_job_id) = enqueue_run(
        &db.pool,
        &app_queue,
        owner_id,
        trend_config_id,
        dataset_version_id,
        &qa.hash,
        "runner-insufficient-history",
    )
    .await;
    assert_eq!(
        run_once(
            &worker,
            &queue,
            "recommendation-insufficient-history",
            &paths,
            &config,
        )
        .await
        .unwrap(),
        RecommendationOutcome::Blocked {
            job_id: history_job_id,
            code: "RECOMMENDATION_DATA_BLOCKED".into(),
        }
    );
    let history_status: String =
        sqlx::query_scalar("SELECT status FROM recommendation_runs WHERE id = $1")
            .bind(history_run_id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(history_status, "BLOCKED");

    let target_path = child_repo
        .path()
        .join("nt/strategies/buy_and_hold/target.py");
    let target = std::fs::read_to_string(&target_path).unwrap();
    std::fs::write(
        &target_path,
        target.replace(
            "    validate_params(params, PACKAGE[\"parameter_schema\"])",
            "    raise TargetError(\"INJECTED\", \"injected deterministic failure\")",
        ),
    )
    .unwrap();
    let (deterministic_run_id, deterministic_job_id) = enqueue_run(
        &db.pool,
        &app_queue,
        owner_id,
        buy_hold_config_id,
        dataset_version_id,
        &qa.hash,
        "runner-deterministic-child",
    )
    .await;
    assert_eq!(
        run_once(
            &worker,
            &queue,
            "recommendation-deterministic-child",
            &paths,
            &config,
        )
        .await
        .unwrap(),
        RecommendationOutcome::Failed {
            job_id: deterministic_job_id,
            code: "TARGET_GENERATION_FAILED".into(),
        }
    );
    let deterministic_status: String =
        sqlx::query_scalar("SELECT status FROM recommendation_runs WHERE id = $1")
            .bind(deterministic_run_id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(deterministic_status, "FAILED");
    std::fs::write(&target_path, target).unwrap();

    let config_id: Uuid = sqlx::query_scalar(
        "INSERT INTO user_strategy_configs \
         (owner_user_id, strategy_id, strategy_version, config_json) \
         VALUES ($1, 'buy_and_hold', '1.0.0', $2) RETURNING id",
    )
    .bind(owner_id)
    .bind(json!({
        "benchmark_instrument": "069500.KRX",
        "target_weight": 1.0,
        "rebalance_cadence": "none"
    }))
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let run_id = Uuid::new_v4();
    let payload = RecommendationPayload {
        run_id,
        strategy_config_id: config_id,
        as_of: NaiveDate::from_ymd_opt(2021, 1, 29).unwrap(),
        dataset: DatasetPin {
            id: dataset_version_id,
            dataset_id: "krx_eod_bars".into(),
            version: "qa-v2".into(),
            curated_version: 2,
            manifest_sha256: qa.hash.clone(),
        },
    };
    let job = app_queue
        .submit(SubmitJob {
            owner_user_id: owner_id,
            job_type: "recommendation".into(),
            payload: serde_json::to_value(&payload).unwrap(),
            priority: 0,
            idempotency_key: Some(format!("runner-exhaust-{run_id}")),
            max_attempts: 2,
            available_at: None,
        })
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO recommendation_runs \
         (id, owner_user_id, strategy_config_id, as_of, status, job_id, trigger_kind, dataset_version_id, dataset_manifest_sha256) \
         VALUES ($1, $2, $3, $4, 'PENDING', $5, 'MANUAL', $6, $7)",
    )
    .bind(run_id)
    .bind(owner_id)
    .bind(config_id)
    .bind(payload.as_of)
    .bind(job.id)
    .bind(dataset_version_id)
    .bind(&qa.hash)
    .execute(&db.pool)
    .await
    .unwrap();
    // The production child prefers the prebuilt virtualenv over uv. Exercise
    // the uv launch-failure path in a separate project without `.venv` so the
    // injected executable remains the process that is actually selected.
    let broken_repo = tempfile::Builder::new()
        .prefix(".recommendation-runner-broken-repo-")
        .tempdir_in(&repo)
        .unwrap();
    copy_python_project(
        &child_repo.path().join("nt"),
        &broken_repo.path().join("nt"),
    );
    let invalid_uv = broken_repo.path().join("not-an-executable");
    std::fs::write(&invalid_uv, b"not an executable").unwrap();
    let mut broken_paths = paths.clone();
    broken_paths.child.repo_root = broken_repo.path().to_path_buf();
    broken_paths.child.uv_bin = invalid_uv;
    let first = run_once(
        &worker,
        &queue,
        "recommendation-exhaust",
        &broken_paths,
        &config,
    )
    .await
    .unwrap();
    assert_eq!(
        first,
        RecommendationOutcome::Retrying {
            job_id: job.id,
            code: "TARGET_CHILD_LAUNCH_FAILED".into(),
        }
    );
    let pending: (String, String) = sqlx::query_as(
        "SELECT r.status, j.status FROM recommendation_runs r \
         JOIN jobs j ON j.id = r.job_id WHERE r.id = $1",
    )
    .bind(run_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(pending, ("PENDING".into(), "QUEUED".into()));
    sqlx::query("UPDATE jobs SET available_at = now() WHERE id = $1")
        .bind(job.id)
        .execute(&db.pool)
        .await
        .unwrap();
    let exhausted = run_once(
        &worker,
        &queue,
        "recommendation-exhaust",
        &broken_paths,
        &config,
    )
    .await
    .unwrap();
    assert_eq!(
        exhausted,
        RecommendationOutcome::Failed {
            job_id: job.id,
            code: "TARGET_CHILD_LAUNCH_FAILED".into(),
        }
    );
    let final_state: (String, String, i64, i64) = sqlx::query_as(
        "SELECT r.status, j.status, \
                (SELECT count(*) FROM recommendation_items WHERE recommendation_run_id = r.id), \
                (SELECT count(*) FROM target_portfolios WHERE recommendation_run_id = r.id) \
         FROM recommendation_runs r JOIN jobs j ON j.id = r.job_id WHERE r.id = $1",
    )
    .bind(run_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(final_state, ("FAILED".into(), "FAILED".into(), 0, 0));

    let cli_path = child_repo
        .path()
        .join("nt/strategies/recommendation_cli.py");
    let cli = std::fs::read_to_string(&cli_path).unwrap();
    std::fs::write(
        &cli_path,
        cli.replace(
            "from __future__ import annotations",
            "from __future__ import annotations\n\nimport time\ntime.sleep(2)",
        ),
    )
    .unwrap();
    let canceled_run_id = Uuid::new_v4();
    let canceled_payload = RecommendationPayload {
        run_id: canceled_run_id,
        strategy_config_id: config_id,
        as_of: NaiveDate::from_ymd_opt(2021, 1, 29).unwrap(),
        dataset: DatasetPin {
            id: dataset_version_id,
            dataset_id: "krx_eod_bars".into(),
            version: "qa-v2".into(),
            curated_version: 2,
            manifest_sha256: qa.hash.clone(),
        },
    };
    let canceled_job = app_queue
        .submit(SubmitJob {
            owner_user_id: owner_id,
            job_type: "recommendation".into(),
            payload: serde_json::to_value(&canceled_payload).unwrap(),
            priority: 0,
            idempotency_key: Some(format!("runner-cancel-{canceled_run_id}")),
            max_attempts: 2,
            available_at: None,
        })
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO recommendation_runs \
         (id, owner_user_id, strategy_config_id, as_of, status, job_id, trigger_kind, dataset_version_id, dataset_manifest_sha256) \
         VALUES ($1, $2, $3, $4, 'PENDING', $5, 'MANUAL', $6, $7)",
    )
    .bind(canceled_run_id)
    .bind(owner_id)
    .bind(config_id)
    .bind(canceled_payload.as_of)
    .bind(canceled_job.id)
    .bind(dataset_version_id)
    .bind(&qa.hash)
    .execute(&db.pool)
    .await
    .unwrap();
    let runner_pool = worker.clone();
    let runner_queue = queue.clone();
    let runner_paths = paths.clone();
    let running = tokio::spawn(async move {
        run_once(
            &runner_pool,
            &runner_queue,
            "recommendation-cancel",
            &runner_paths,
            &config,
        )
        .await
    });
    for _ in 0..100 {
        let status: String = sqlx::query_scalar("SELECT status FROM jobs WHERE id = $1")
            .bind(canceled_job.id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
        if status == "RUNNING" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let cancel_queue = JobQueue::new(
        db.pool.clone(),
        Some(db.pool.clone()),
        QueueConfig::default(),
    );
    cancel_queue
        .request_cancel(canceled_job.id, &job_queue::AuditActor::new("owner"))
        .await
        .unwrap();
    assert_eq!(
        running.await.unwrap().unwrap(),
        RecommendationOutcome::Failed {
            job_id: canceled_job.id,
            code: "RECOMMENDATION_CANCELED".into(),
        }
    );
    let canceled_state: (String, String, i64, i64, String, Option<String>) = sqlx::query_as(
        "SELECT r.status, j.status, \
                (SELECT count(*) FROM recommendation_items WHERE recommendation_run_id = r.id), \
                (SELECT count(*) FROM target_portfolios WHERE recommendation_run_id = r.id), \
                a.outcome, a.error_code \
         FROM recommendation_runs r JOIN jobs j ON j.id = r.job_id \
         JOIN job_attempts a ON a.job_id = j.id WHERE r.id = $1",
    )
    .bind(canceled_run_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        canceled_state,
        (
            "FAILED".into(),
            "CANCELED".into(),
            0,
            0,
            "FAILED".into(),
            Some("canceled".into())
        )
    );
    for _ in 0..120 {
        if child_temp.path().read_dir().unwrap().next().is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        child_temp.path().read_dir().unwrap().next().is_none(),
        "cancel must not abandon child scratch cleanup"
    );
    std::fs::write(&cli_path, cli).unwrap();

    let first_source = BatchId::generate();
    let second_source = BatchId::generate();
    let source_batches = vec![
        SourceBatchRef {
            batch_id: first_source,
            bars_file: "first-bars.json".into(),
            bars_hash: ContentHash::from_bytes(b"first bars"),
            actions_file: "first-actions.json".into(),
            actions_hash: ContentHash::from_bytes(b"first actions"),
        },
        SourceBatchRef {
            batch_id: second_source,
            bars_file: "second-bars.json".into(),
            bars_hash: ContentHash::from_bytes(b"second bars"),
            actions_file: "second-actions.json".into(),
            actions_hash: ContentHash::from_bytes(b"second actions"),
        },
    ];
    let production_manifest = DatasetManifest {
        dataset_id: DatasetId::parse("krx_eod_bars").unwrap(),
        version: 2,
        capability: Capability::PriceReturnOnly,
        created_at: UtcTimestamp::parse_rfc3339("2021-01-29T06:30:00Z").unwrap(),
        source_batches,
        artifacts: attest_curated_artifacts(&CurateStore::new(&qa.root), 2),
        bar_count: 11 * 260,
        action_count: 0,
        content_hash: ContentHash::from_bytes(b"placeholder"),
    };
    let production_manifest = DatasetManifest {
        content_hash: dataset_manifest_hash(&production_manifest).unwrap(),
        ..production_manifest
    };
    CurateStore::new(&qa.root)
        .write_dataset_manifest(&production_manifest)
        .unwrap();
    let production_hash = production_manifest
        .content_hash
        .as_str()
        .strip_prefix("sha256:")
        .unwrap();
    sqlx::query("UPDATE dataset_versions SET manifest_sha256 = $2 WHERE id = $1")
        .bind(dataset_version_id)
        .bind(production_hash)
        .execute(&db.pool)
        .await
        .unwrap();
    for (batch, file, kind, hash) in [
        (
            first_source,
            "first-bars.json",
            "EOD",
            production_manifest.source_batches[0].bars_hash.clone(),
        ),
        (
            first_source,
            "first-actions.json",
            "CORPORATE_ACTIONS",
            production_manifest.source_batches[0].actions_hash.clone(),
        ),
        (
            second_source,
            "second-bars.json",
            "EOD",
            production_manifest.source_batches[1].bars_hash.clone(),
        ),
    ] {
        sqlx::query(
            "INSERT INTO data_batches \
             (provider, market, batch_date, kind, storage_path, content_sha256, bytes_size, retrieved_at, source_batch_id, source_file_name, fetch_mode) \
             VALUES ('KRX', 'KR', DATE '2021-01-29', $1, $2, $3, 1, now(), $4, $5, 'credentialed')",
        )
        .bind(kind)
        .bind(format!("raw/{batch}/{file}"))
        .bind(hash.as_str().trim_start_matches("sha256:"))
        .bind(batch.as_uuid())
        .bind(file)
        .execute(&db.pool)
        .await
        .unwrap();
    }
    let (missing_pin_run_id, missing_pin_job_id) = enqueue_run(
        &db.pool,
        &app_queue,
        owner_id,
        config_id,
        dataset_version_id,
        production_hash,
        "runner-production-missing-source-file",
    )
    .await;
    let production_config = config.with_production(true);
    assert_eq!(
        run_once(
            &worker,
            &queue,
            "recommendation-production",
            &paths,
            &production_config,
        )
        .await
        .unwrap(),
        RecommendationOutcome::Blocked {
            job_id: missing_pin_job_id,
            code: "RECOMMENDATION_CREDENTIALLED_DATA_REQUIRED".into(),
        }
    );
    let missing_pin_state: (String, i64, i64) = sqlx::query_as(
        "SELECT status, \
                (SELECT count(*) FROM recommendation_items WHERE recommendation_run_id = $1), \
                (SELECT count(*) FROM target_portfolios WHERE recommendation_run_id = $1) \
         FROM recommendation_runs WHERE id = $1",
    )
    .bind(missing_pin_run_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(missing_pin_state, ("BLOCKED".into(), 0, 0));

    sqlx::query(
        "INSERT INTO data_batches \
         (provider, market, batch_date, kind, storage_path, content_sha256, bytes_size, retrieved_at, source_batch_id, source_file_name, fetch_mode) \
         VALUES ('KRX', 'KR', DATE '2021-01-29', 'CORPORATE_ACTIONS', $1, $2, 1, now(), $3, 'second-actions.json', 'credentialed')",
    )
    .bind(format!("raw/{second_source}/second-actions.json"))
    .bind(
        production_manifest.source_batches[1]
            .actions_hash
            .as_str()
            .trim_start_matches("sha256:"),
    )
    .bind(second_source.as_uuid())
    .execute(&db.pool)
    .await
    .unwrap();
    let (production_run_id, production_job_id) = enqueue_run(
        &db.pool,
        &app_queue,
        owner_id,
        config_id,
        dataset_version_id,
        production_hash,
        "runner-production",
    )
    .await;
    assert_eq!(
        run_once(
            &worker,
            &queue,
            "recommendation-production",
            &paths,
            &production_config,
        )
        .await
        .unwrap(),
        RecommendationOutcome::Succeeded {
            job_id: production_job_id,
            run_id: production_run_id,
        }
    );

    sqlx::query(
        "UPDATE data_batches SET fetch_mode = 'synthetic' \
         WHERE source_batch_id = $1 AND source_file_name = 'first-actions.json'",
    )
    .bind(first_source.as_uuid())
    .execute(&db.pool)
    .await
    .unwrap();
    let (blocked_run_id, blocked_job_id) = enqueue_run(
        &db.pool,
        &app_queue,
        owner_id,
        config_id,
        dataset_version_id,
        production_hash,
        "runner-production-mixed",
    )
    .await;
    assert_eq!(
        run_once(
            &worker,
            &queue,
            "recommendation-production",
            &paths,
            &production_config,
        )
        .await
        .unwrap(),
        RecommendationOutcome::Blocked {
            job_id: blocked_job_id,
            code: "RECOMMENDATION_CREDENTIALLED_DATA_REQUIRED".into(),
        }
    );
    let blocked: (String, i64, i64) = sqlx::query_as(
        "SELECT status, \
                (SELECT count(*) FROM recommendation_items WHERE recommendation_run_id = $1), \
                (SELECT count(*) FROM target_portfolios WHERE recommendation_run_id = $1) \
         FROM recommendation_runs WHERE id = $1",
    )
    .bind(blocked_run_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(blocked, ("BLOCKED".into(), 0, 0));

    sqlx::query("UPDATE data_batches SET fetch_mode = 'credentialed' WHERE source_batch_id = $1")
        .bind(first_source.as_uuid())
        .execute(&db.pool)
        .await
        .unwrap();
    let cli = std::fs::read_to_string(&cli_path).unwrap();
    std::fs::write(
        &cli_path,
        cli.replace(
            "from __future__ import annotations",
            "from __future__ import annotations\n\nimport time\ntime.sleep(2)",
        ),
    )
    .unwrap();
    sqlx::raw_sql(
        "CREATE FUNCTION test_fail_recommendation_heartbeat() RETURNS trigger \
         LANGUAGE plpgsql AS $$ BEGIN \
           IF OLD.status = 'RUNNING' AND NEW.status = 'RUNNING' \
              AND NEW.locked_at IS DISTINCT FROM OLD.locked_at THEN \
             RAISE EXCEPTION USING ERRCODE = '40001', MESSAGE = 'injected heartbeat failure'; \
           END IF; \
           RETURN NEW; \
         END $$; \
         CREATE TRIGGER test_fail_recommendation_heartbeat \
         BEFORE UPDATE ON jobs FOR EACH ROW \
         EXECUTE FUNCTION test_fail_recommendation_heartbeat();",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let (heartbeat_run_id, heartbeat_job_id) = enqueue_run(
        &db.pool,
        &app_queue,
        owner_id,
        config_id,
        dataset_version_id,
        production_hash,
        "runner-heartbeat-error",
    )
    .await;
    assert_eq!(
        run_once(
            &worker,
            &queue,
            "recommendation-heartbeat-error",
            &paths,
            &production_config,
        )
        .await
        .unwrap(),
        RecommendationOutcome::Retrying {
            job_id: heartbeat_job_id,
            code: "RECOMMENDATION_HEARTBEAT_UNAVAILABLE".into(),
        }
    );
    let heartbeat_state: (String, String) = sqlx::query_as(
        "SELECT r.status, j.status FROM recommendation_runs r \
         JOIN jobs j ON j.id = r.job_id WHERE r.id = $1",
    )
    .bind(heartbeat_run_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(heartbeat_state, ("PENDING".into(), "QUEUED".into()));
    sqlx::raw_sql(
        "DROP TRIGGER test_fail_recommendation_heartbeat ON jobs; \
         DROP FUNCTION test_fail_recommendation_heartbeat();",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query("UPDATE jobs SET available_at = now() + interval '1 hour' WHERE id = $1")
        .bind(heartbeat_job_id)
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::raw_sql(
        "CREATE FUNCTION test_reject_recommendation_heartbeat() RETURNS trigger \
         LANGUAGE plpgsql AS $$ BEGIN \
           IF OLD.status = 'RUNNING' AND NEW.status = 'RUNNING' \
              AND NEW.locked_at IS DISTINCT FROM OLD.locked_at THEN \
             RAISE EXCEPTION USING ERRCODE = '23514', MESSAGE = 'injected permanent heartbeat failure'; \
           END IF; \
           RETURN NEW; \
         END $$; \
         CREATE TRIGGER test_reject_recommendation_heartbeat \
         BEFORE UPDATE ON jobs FOR EACH ROW \
         EXECUTE FUNCTION test_reject_recommendation_heartbeat();",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let (permanent_heartbeat_run_id, permanent_heartbeat_job_id) = enqueue_run(
        &db.pool,
        &app_queue,
        owner_id,
        config_id,
        dataset_version_id,
        production_hash,
        "runner-heartbeat-integrity",
    )
    .await;
    assert_eq!(
        run_once(
            &worker,
            &queue,
            "recommendation-heartbeat-integrity",
            &paths,
            &production_config,
        )
        .await
        .unwrap(),
        RecommendationOutcome::Failed {
            job_id: permanent_heartbeat_job_id,
            code: "RECOMMENDATION_HEARTBEAT_INTEGRITY".into(),
        }
    );
    let permanent_heartbeat_state: (String, String, i64, i64) = sqlx::query_as(
        "SELECT r.status, j.status, \
                (SELECT count(*) FROM recommendation_items WHERE recommendation_run_id = r.id), \
                (SELECT count(*) FROM target_portfolios WHERE recommendation_run_id = r.id) \
         FROM recommendation_runs r JOIN jobs j ON j.id = r.job_id WHERE r.id = $1",
    )
    .bind(permanent_heartbeat_run_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        permanent_heartbeat_state,
        ("FAILED".into(), "FAILED".into(), 0, 0)
    );
    sqlx::raw_sql(
        "DROP TRIGGER test_reject_recommendation_heartbeat ON jobs; \
         DROP FUNCTION test_reject_recommendation_heartbeat();",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query("UPDATE jobs SET available_at = now() WHERE id = $1")
        .bind(heartbeat_job_id)
        .execute(&db.pool)
        .await
        .unwrap();
    let stale_pool = worker.clone();
    let stale_queue = queue.clone();
    let stale_paths = paths.clone();
    let stale_config = RecommendationRunnerConfig::new(
        Duration::from_secs(5),
        Duration::from_secs(10),
        Duration::from_secs(30),
    )
    .unwrap()
    .with_production(true);
    let stale = tokio::spawn(async move {
        run_once(
            &stale_pool,
            &stale_queue,
            "recommendation-stale",
            &stale_paths,
            &stale_config,
        )
        .await
    });
    let mut claimed_second_attempt = false;
    for _ in 0..100 {
        let state: (String, i32) =
            sqlx::query_as("SELECT status, attempt_count FROM jobs WHERE id = $1")
                .bind(heartbeat_job_id)
                .fetch_one(&db.pool)
                .await
                .unwrap();
        if state == ("RUNNING".into(), 2) {
            claimed_second_attempt = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(claimed_second_attempt);
    sqlx::query("UPDATE jobs SET locked_at = now() - interval '20 seconds' WHERE id = $1")
        .bind(heartbeat_job_id)
        .execute(&db.pool)
        .await
        .unwrap();
    let sweep = queue.sweep().await.unwrap();
    assert_eq!(sweep.jobs_failed, 1);
    assert_eq!(
        stale.await.unwrap().unwrap(),
        RecommendationOutcome::Failed {
            job_id: heartbeat_job_id,
            code: "RECOMMENDATION_ATTEMPTS_EXHAUSTED".into(),
        }
    );
    let stale_state: (String, String, i64, i64) = sqlx::query_as(
        "SELECT r.status, j.status, \
                (SELECT count(*) FROM recommendation_items WHERE recommendation_run_id = r.id), \
                (SELECT count(*) FROM target_portfolios WHERE recommendation_run_id = r.id) \
         FROM recommendation_runs r JOIN jobs j ON j.id = r.job_id WHERE r.id = $1",
    )
    .bind(heartbeat_run_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(stale_state, ("FAILED".into(), "FAILED".into(), 0, 0));
    for _ in 0..120 {
        if child_temp.path().read_dir().unwrap().next().is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(child_temp.path().read_dir().unwrap().next().is_none());
    std::fs::write(&cli_path, cli).unwrap();
    sqlx::raw_sql(
        "CREATE FUNCTION test_fail_final_recommendation_heartbeat() RETURNS trigger \
         LANGUAGE plpgsql AS $$ BEGIN \
           IF OLD.status = 'RUNNING' AND NEW.status = 'RUNNING' \
              AND NEW.locked_at IS DISTINCT FROM OLD.locked_at THEN \
             RAISE EXCEPTION USING ERRCODE = '40001', MESSAGE = 'injected final heartbeat failure'; \
           END IF; \
           RETURN NEW; \
         END $$; \
         CREATE TRIGGER test_fail_final_recommendation_heartbeat \
         BEFORE UPDATE ON jobs FOR EACH ROW \
         EXECUTE FUNCTION test_fail_final_recommendation_heartbeat();",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let (final_heartbeat_run_id, final_heartbeat_job_id) = enqueue_run(
        &db.pool,
        &app_queue,
        owner_id,
        config_id,
        dataset_version_id,
        production_hash,
        "runner-final-heartbeat-error",
    )
    .await;
    let final_heartbeat_config = RecommendationRunnerConfig::new(
        Duration::from_secs(5),
        Duration::from_secs(10),
        Duration::from_secs(30),
    )
    .unwrap()
    .with_production(true);
    assert_eq!(
        run_once(
            &worker,
            &queue,
            "recommendation-final-heartbeat",
            &paths,
            &final_heartbeat_config,
        )
        .await
        .unwrap(),
        RecommendationOutcome::Retrying {
            job_id: final_heartbeat_job_id,
            code: "RECOMMENDATION_HEARTBEAT_UNAVAILABLE".into(),
        }
    );
    let final_heartbeat_state: (String, String) = sqlx::query_as(
        "SELECT r.status, j.status FROM recommendation_runs r \
         JOIN jobs j ON j.id = r.job_id WHERE r.id = $1",
    )
    .bind(final_heartbeat_run_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(final_heartbeat_state, ("PENDING".into(), "QUEUED".into()));
    sqlx::raw_sql(
        "DROP TRIGGER test_fail_final_recommendation_heartbeat ON jobs; \
         DROP FUNCTION test_fail_final_recommendation_heartbeat();",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query("UPDATE jobs SET available_at = now() + interval '1 hour' WHERE id = $1")
        .bind(final_heartbeat_job_id)
        .execute(&db.pool)
        .await
        .unwrap();

    sqlx::raw_sql(
        "CREATE FUNCTION test_reject_final_recommendation_heartbeat() RETURNS trigger \
         LANGUAGE plpgsql AS $$ BEGIN \
           IF OLD.status = 'RUNNING' AND NEW.status = 'RUNNING' \
              AND NEW.locked_at IS DISTINCT FROM OLD.locked_at THEN \
             RAISE EXCEPTION USING ERRCODE = '23514', MESSAGE = 'injected permanent final heartbeat failure'; \
           END IF; \
           RETURN NEW; \
         END $$; \
         CREATE TRIGGER test_reject_final_recommendation_heartbeat \
         BEFORE UPDATE ON jobs FOR EACH ROW \
         EXECUTE FUNCTION test_reject_final_recommendation_heartbeat();",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let (permanent_final_run_id, permanent_final_job_id) = enqueue_run(
        &db.pool,
        &app_queue,
        owner_id,
        config_id,
        dataset_version_id,
        production_hash,
        "runner-final-heartbeat-integrity",
    )
    .await;
    assert_eq!(
        run_once(
            &worker,
            &queue,
            "recommendation-final-heartbeat-integrity",
            &paths,
            &final_heartbeat_config,
        )
        .await
        .unwrap(),
        RecommendationOutcome::Failed {
            job_id: permanent_final_job_id,
            code: "RECOMMENDATION_HEARTBEAT_INTEGRITY".into(),
        }
    );
    let permanent_final_state: (String, String, i64, i64) = sqlx::query_as(
        "SELECT r.status, j.status, \
                (SELECT count(*) FROM recommendation_items WHERE recommendation_run_id = r.id), \
                (SELECT count(*) FROM target_portfolios WHERE recommendation_run_id = r.id) \
         FROM recommendation_runs r JOIN jobs j ON j.id = r.job_id WHERE r.id = $1",
    )
    .bind(permanent_final_run_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        permanent_final_state,
        ("FAILED".into(), "FAILED".into(), 0, 0)
    );
    sqlx::raw_sql(
        "DROP TRIGGER test_reject_final_recommendation_heartbeat ON jobs; \
         DROP FUNCTION test_reject_final_recommendation_heartbeat();",
    )
    .execute(&db.pool)
    .await
    .unwrap();

    worker.close().await;
    db.drop_db().await;
}
