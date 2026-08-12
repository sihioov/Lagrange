//! Todo 24 recommendation routes: create (entitlement-gated, queued),
//! get, list, latest, items; idempotent replay; ownership isolation.

mod common;
use axum::http::StatusCode;
use common::{Harness, status};
use domain::{ContentHash, DatasetId, InstrumentId, UtcTimestamp};
use job_queue::recommendation::child::TargetChildPaths;
use job_queue::recommendation::compute::AttestedUniverse;
use job_queue::recommendation::input::DatasetPin;
use job_queue::recommendation::{
    RecommendationOutcome, RecommendationRunnerConfig, RecommendationRunnerPaths, run_once,
};
use job_queue::{JobQueue, QueueConfig};
use market_data::curate::schema::{read_adjusted_bars, read_bars, write_adjusted_bars, write_bars};
use market_data::{Capability, CurateStore, DatasetManifest, dataset_manifest_hash};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use uuid::Uuid;

const FIXED_UNIVERSE_MEMBERS: [&str; 11] = [
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

fn workspace_root() -> PathBuf {
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

fn copy_python_project(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("child repo creates");
    for entry in std::fs::read_dir(from).expect("read child repo") {
        let entry = entry.expect("child repo entry");
        if entry.file_name() == ".venv" || entry.file_name() == "__pycache__" {
            continue;
        }
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_python_project(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).expect("copy child repo file");
        }
    }
}

fn clone_symbol(store_root: &Path, source: &str, destination: &str) {
    let market = store_root.join("curated/bars/market=kr");
    let source_dir = market.join(format!("symbol={source}"));
    let target = market.join(format!("symbol={destination}"));
    copy_python_project(&source_dir, &target);
    let store = CurateStore::new(store_root);
    let destination_id = InstrumentId::parse(destination).expect("fixed instrument id");
    for year in std::fs::read_dir(&target)
        .expect("cloned market years")
        .flatten()
    {
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
            let mut rows = read_bars(&bars_path).expect("read cloned bars");
            rows.iter_mut()
                .for_each(|row| row.instrument_id = destination_id.clone());
            write_bars(&bars_path, &rows).expect("rewrite cloned bars");
        }
        for adjusted in [
            store.adjusted_bars_path("kr", destination, year, 2),
            store.total_return_bars_path("kr", destination, year, 2),
        ] {
            if adjusted.is_file() {
                let mut rows = read_adjusted_bars(&adjusted).expect("read cloned adjusted bars");
                rows.iter_mut()
                    .for_each(|row| row.instrument_id = destination_id.clone());
                write_adjusted_bars(&adjusted, &rows).expect("rewrite cloned adjusted bars");
            }
        }
    }
}

struct QaDataset {
    root: PathBuf,
    hash: String,
}

fn qa_dataset(artifact_root: &Path) -> QaDataset {
    let repo = workspace_root();
    // The generator intentionally rejects output outside the repository.  It
    // builds in a temporary repository child, then this test copies the
    // resulting immutable store into its scratch artifact root.
    let temporary = tempfile::Builder::new()
        .prefix(".api-recommendation-phase0-")
        .tempdir_in(&repo)
        .expect("phase0 fixture temp dir");
    let generated = temporary.path().join("phase0");
    let output = Command::new(std::env::var_os("PYTHON").unwrap_or_else(|| "python".into()))
        .current_dir(&repo)
        .arg(repo.join("scripts/ci/prepare_phase0.py"))
        .arg("--root")
        .arg(&generated)
        .output()
        .expect("run phase0 fixture generator");
    assert!(
        output.status.success(),
        "phase0 fixture generator failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let root = artifact_root.join("recommendation-phase0/curated");
    copy_python_project(&generated.join("curated"), &root);
    let market = root.join("curated/bars/market=kr");
    for member in FIXED_UNIVERSE_MEMBERS {
        if !market.join(format!("symbol={member}")).exists() {
            clone_symbol(&root, "069500.KRX", member);
        }
    }
    let store = CurateStore::new(&root);
    let manifest = DatasetManifest {
        dataset_id: DatasetId::parse("krx_eod_bars").expect("dataset id"),
        version: 2,
        capability: Capability::PriceReturnOnly,
        created_at: UtcTimestamp::parse_rfc3339("2021-01-29T06:30:00Z").expect("timestamp"),
        source_batches: Vec::new(),
        bar_count: 11 * 260,
        action_count: 0,
        content_hash: ContentHash::from_bytes(b"placeholder"),
    };
    let manifest = DatasetManifest {
        content_hash: dataset_manifest_hash(&manifest).expect("manifest hash"),
        ..manifest
    };
    store
        .write_dataset_manifest(&manifest)
        .expect("write dataset manifest");
    QaDataset {
        root,
        hash: manifest
            .content_hash
            .as_str()
            .strip_prefix("sha256:")
            .expect("sha256 manifest hash")
            .to_owned(),
    }
}

/// Build the smallest real fixed-ETF world the worker accepts.  It writes
/// only source data/configuration; the runner itself is the sole writer of
/// recommendation reports, items, and target portfolios.
async fn prepare_real_runner_fixture(
    h: &mut Harness,
    strategy_config_id: &str,
) -> RecommendationRunnerPaths {
    let qa = qa_dataset(&h.artifact_root);
    let universe = AttestedUniverse::from_manifest_yaml(include_str!(
        "../../../configs/universes/kr-etf-core-v1.yaml"
    ))
    .expect("shipped universe parses");
    for member in FIXED_UNIVERSE_MEMBERS {
        sqlx::query(
            "INSERT INTO instruments (id, symbol, venue, currency, name, asset_class, status) \
             VALUES ($1, $2, 'KRX', 'KRW', $2, 'ETF', 'ACTIVE') ON CONFLICT (id) DO NOTHING",
        )
        .bind(member)
        .bind(member.trim_end_matches(".KRX"))
        .execute(&h.owner_pool)
        .await
        .expect("seed fixed universe instrument");
    }
    sqlx::query(
        "INSERT INTO universe_snapshots \
         (snapshot_id, universe_manifest_sha256, instruments_json, published_by) \
         VALUES ($1, repeat('d', 64), $2, $3)",
    )
    .bind(universe.snapshot_id())
    .bind(json!(universe.members()))
    .bind(h.member.user_id)
    .execute(&h.owner_pool)
    .await
    .expect("seed fixed universe snapshot");

    let dataset_id = h.state().cfg.recommendation_dataset.id;
    sqlx::query(
        "UPDATE dataset_versions SET dataset_id = 'krx_eod_bars', version = 'qa-v2', \
         status = 'READY', manifest_sha256 = $2, storage_path = $3 WHERE id = $1",
    )
    .bind(dataset_id)
    .bind(&qa.hash)
    .bind(qa.root.to_string_lossy().as_ref())
    .execute(&h.owner_pool)
    .await
    .expect("pin dataset fixture");
    h.seed_shared(
        "UPDATE data_entitlements SET effective_from = DATE '2020-01-01', \
         effective_until = DATE '2030-12-31' WHERE status = 'ACTIVE'",
    )
    .await;
    h.seed_tenant(
        &h.member,
        &format!(
            "UPDATE user_strategy_configs SET config_json = \
             '{{\"benchmark_instrument\":\"069500.KRX\",\"target_weight\":1.0,\"rebalance_cadence\":\"none\"}}'::jsonb \
             WHERE id = '{strategy_config_id}'"
        ),
    )
    .await;
    h.restart_api_with_recommendation_dataset(DatasetPin {
        id: dataset_id,
        dataset_id: "krx_eod_bars".into(),
        version: "qa-v2".into(),
        curated_version: 2,
        manifest_sha256: qa.hash,
    })
    .await;

    let repo = workspace_root();
    let uv = uv_bin().expect("the real runner test requires uv");
    let child_repo = h.artifact_root.join("recommendation-runner-repo");
    copy_python_project(&repo.join("nt"), &child_repo.join("nt"));
    let sync = Command::new(&uv)
        .arg("sync")
        .arg("--project")
        .arg(child_repo.join("nt"))
        .arg("--locked")
        .output()
        .expect("sync isolated runner environment");
    assert!(
        sync.status.success(),
        "isolated runner sync failed: {}",
        String::from_utf8_lossy(&sync.stderr)
    );
    let child_temp = h.artifact_root.join("recommendation-runner-child");
    std::fs::create_dir_all(&child_temp).expect("runner child temp creates");
    RecommendationRunnerPaths {
        data_root: qa.root,
        universe_manifest: repo.join("configs/universes/kr-etf-core-v1.yaml"),
        child: TargetChildPaths {
            uv_bin: uv,
            repo_root: child_repo,
            temp_root: child_temp,
        },
    }
}

/// Create a strategy config for `actor` and return its id.
async fn config_id(h: &Harness, actor: &common::UserCtx) -> String {
    let resp = h
        .send(
            "POST",
            "/api/v1/strategies/buy_and_hold/configs",
            Some(actor),
            true,
            Some("test-rid-1"),
            Some("seed-config-001"),
            Some(json!({ "strategy_version": "1.0.0", "config": { "lookback": 200 }, "is_active": true })),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    assert_eq!(resp.status(), StatusCode::CREATED);
    Harness::body_json(resp).await["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn http_recommendations_create_queue_result_happy() {
    let Some(mut h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let cfg = config_id(&h, &h.member).await;
    let paths = prepare_real_runner_fixture(&mut h, &cfg).await;

    // create -> PENDING run + QUEUED recommendation job.
    let resp = h
        .post(
            "/api/v1/recommendations/runs",
            Some(&h.member),
            true,
            json!({ "strategy_config_id": cfg, "as_of": "2021-01-29" }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::CREATED);
    h.assert_rid_echo(&resp);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["status"], "PENDING");
    assert_eq!(body["as_of"], "2021-01-29");
    assert_eq!(body["strategy_config_id"], cfg);
    let run_id = body["id"].as_str().unwrap().to_string();
    let job_id = body["job_id"].as_str().unwrap().to_string();
    assert_eq!(body["trigger_kind"], "MANUAL");
    assert!(body["provenance"]["dataset_version_id"].is_string());
    assert!(
        !body.to_string().contains("owner_user_id"),
        "no tenant column leak"
    );

    // The job is queued (visible through the API? recommendation job is the
    // actor's own -> assert via admin? no: assert through the queue table).
    let (job_status, payload, persisted_job_id, queue_key): (
        String,
        serde_json::Value,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT j.status, j.payload_json, r.job_id::text, j.idempotency_key \
             FROM jobs j JOIN recommendation_runs r ON r.id = $2::uuid WHERE j.id = $1::uuid",
    )
    .bind(&job_id)
    .bind(&run_id)
    .fetch_one(&h.member_pool().await)
    .await
    .unwrap();
    assert_eq!(job_status, "QUEUED");
    assert_eq!(persisted_job_id.as_deref(), Some(job_id.as_str()));
    assert!(
        queue_key.is_some_and(|key| key.starts_with("recommendation:")),
        "job idempotency is isolated from other queue job types"
    );
    assert_eq!(payload["run_id"], run_id);
    assert_eq!(payload["strategy_config_id"], cfg);
    assert!(
        payload["dataset"]["id"].is_string(),
        "runner requires a pinned dataset id"
    );
    assert!(payload["dataset"]["dataset_id"].is_string());
    assert!(payload["dataset"]["version"].is_string());
    assert!(payload["dataset"]["curated_version"].is_u64());
    assert!(payload["dataset"]["manifest_sha256"].is_string());

    // Drive the actual typed runner once against a faithful local fixture;
    // GET must expose the runner-persisted report, never test-written rows.
    let worker = h.worker_pool().await;
    let queue = JobQueue::new(
        worker.clone(),
        None,
        QueueConfig {
            lease: Duration::from_secs(60),
            backoff_base: Duration::from_millis(1),
        },
    );
    let runner = RecommendationRunnerConfig::new(
        Duration::from_millis(100),
        Duration::from_secs(60),
        Duration::from_secs(30),
    )
    .unwrap();
    let outcome = run_once(&worker, &queue, "http-recommendations", &paths, &runner)
        .await
        .unwrap();
    assert!(
        matches!(
            outcome,
            RecommendationOutcome::Succeeded { job_id: id, run_id: completed }
                if id.to_string() == job_id && completed.to_string() == run_id
        ),
        "real runner outcome: {outcome:?}"
    );

    // GET reads the runner-persisted report with no fabricated items.
    let resp = h
        .get(
            &format!("/api/v1/recommendations/runs/{run_id}"),
            Some(&h.member),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["status"], "SUCCEEDED");
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 11);
    assert!(
        items
            .iter()
            .any(|item| item["instrument_id"] == "069500.KRX")
    );
    assert!(
        body["summary"]["portfolio_snapshot_id"].is_string(),
        "GET exposes the runner-persisted recommendation report"
    );
    let portfolios: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM target_portfolios WHERE recommendation_run_id = $1::uuid",
    )
    .bind(&run_id)
    .fetch_one(&h.member_pool().await)
    .await
    .expect("runner-persisted target portfolio");
    assert_eq!(portfolios, 1);

    // latest by config.
    let resp = h
        .get(
            &format!("/api/v1/recommendations/latest?strategy_config_id={cfg}"),
            Some(&h.member),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["run"]["id"], run_id);
    assert_eq!(body["run"]["status"], "SUCCEEDED");
    assert_eq!(body["run"]["items"].as_array().unwrap().len(), 11);
    assert_eq!(body["latest_run"]["id"], run_id);
    assert_eq!(body["latest_run"]["status"], "SUCCEEDED");
    h.teardown().await;
}

#[tokio::test]
async fn http_recommendations_entitlement_gate_fails_closed() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let cfg = config_id(&h, &h.member).await;
    let existing = h
        .post(
            "/api/v1/recommendations/runs",
            Some(&h.member),
            true,
            json!({ "strategy_config_id": cfg, "as_of": "2026-01-30" }),
        )
        .await;
    assert_eq!(status(&existing), StatusCode::CREATED);
    let existing_id = Harness::body_json(existing).await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    // Revoke the ACTIVE entitlement (expire it) -> create is denied.
    h.seed_shared("UPDATE data_entitlements SET status='REVOKED' WHERE status='ACTIVE'")
        .await;
    let resp = h
        .post(
            "/api/v1/recommendations/runs",
            Some(&h.member),
            true,
            json!({ "strategy_config_id": cfg, "as_of": "2026-01-31" }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::FORBIDDEN);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "DATA_ENTITLEMENT_REQUIRED");

    // Revocation hides an already-persisted report as well as denying new
    // work; payloads are never exposed after the fresh read gate fails.
    let get = h
        .get(
            &format!("/api/v1/recommendations/runs/{existing_id}"),
            Some(&h.member),
        )
        .await;
    assert_eq!(status(&get), StatusCode::FORBIDDEN);
    assert_eq!(
        Harness::error_code(&Harness::body_json(get).await),
        "DATA_ENTITLEMENT_REQUIRED"
    );

    // No side effect: no recommendation_runs row, no job.
    let runs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM recommendation_runs WHERE strategy_config_id = $1::uuid",
    )
    .bind(&cfg)
    .fetch_one(&h.member_pool().await)
    .await
    .unwrap();
    assert_eq!(runs, 1, "denied create must not leave an additional row");
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE job_type='recommendation'")
        .fetch_one(&h.member_pool().await)
        .await
        .unwrap();
    assert_eq!(jobs, 1, "denied create must not enqueue an additional job");
    h.teardown().await;
}

#[tokio::test]
async fn http_recommendations_ownership_and_idempotency() {
    let Some(mut h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let cfg = config_id(&h, &h.member).await;
    let resp = h
        .post(
            "/api/v1/recommendations/runs",
            Some(&h.member),
            true,
            json!({ "strategy_config_id": cfg, "as_of": "2026-01-31" }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::CREATED);
    let run_id = Harness::body_json(resp).await["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Member B (owner) cannot read member A's run.
    let resp = h
        .get(
            &format!("/api/v1/recommendations/runs/{run_id}"),
            Some(&h.owner),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::NOT_FOUND);

    // Idempotent replay: same key -> same run id, no second run/job.
    // (The first create above used a different auto key, so the fixed-key
    // create below is a distinct run; its OWN replay must dedup.)
    let key = "rec-run-001";
    let b = json!({ "strategy_config_id": cfg, "as_of": "2026-01-31" });
    let r1 = h
        .send(
            "POST",
            "/api/v1/recommendations/runs",
            Some(&h.member),
            true,
            Some("test-rid-1"),
            Some(key),
            Some(b.clone()),
        )
        .await;
    assert_eq!(r1.status(), StatusCode::CREATED);
    let body1 = Harness::body_json(r1).await;
    let id1 = body1["id"].as_str().unwrap().to_string();
    let job1 = body1["job_id"].as_str().unwrap().to_string();
    assert_ne!(
        id1, run_id,
        "different idempotency keys create distinct runs"
    );

    // A process restart clears the HTTP layer's in-memory store.  This
    // replay must therefore come from the durable job idempotency record.
    h.restart_api().await;
    let r2 = h
        .send(
            "POST",
            "/api/v1/recommendations/runs",
            Some(&h.member),
            true,
            Some("test-rid-1"),
            Some(key),
            Some(b.clone()),
        )
        .await;
    assert_eq!(r2.status(), StatusCode::CREATED);
    let body2 = Harness::body_json(r2).await;
    let id2 = body2["id"].as_str().unwrap().to_string();
    assert_eq!(id1, id2);
    assert_eq!(body2["job_id"], job1);

    // Deployment may advance the hidden dataset pin between public retries.
    // The public key + body must still replay the run originally committed.
    let replacement_id: Uuid = sqlx::query_scalar(
        "INSERT INTO dataset_versions \
         (dataset_id, version, status, manifest_sha256, storage_path) \
         VALUES ('krx_eod_bars', 'replacement-v3', 'READY', repeat('9',64), \
                 'data/curated/krx_eod_bars/replacement-v3') RETURNING id",
    )
    .fetch_one(&h.owner_pool)
    .await
    .unwrap();
    h.restart_api_with_recommendation_dataset(DatasetPin {
        id: replacement_id,
        dataset_id: "krx_eod_bars".into(),
        version: "replacement-v3".into(),
        curated_version: 3,
        manifest_sha256: "9".repeat(64),
    })
    .await;
    let pin_changed_replay = h
        .send(
            "POST",
            "/api/v1/recommendations/runs",
            Some(&h.member),
            true,
            Some("test-rid-1"),
            Some(key),
            Some(b.clone()),
        )
        .await;
    assert_eq!(pin_changed_replay.status(), StatusCode::CREATED);
    let pin_changed_body = Harness::body_json(pin_changed_replay).await;
    assert_eq!(pin_changed_body["id"], id1);
    assert_eq!(pin_changed_body["job_id"], job1);

    h.restart_api().await;
    let mismatch = h
        .send(
            "POST",
            "/api/v1/recommendations/runs",
            Some(&h.member),
            true,
            Some("test-rid-1"),
            Some(key),
            Some(json!({ "strategy_config_id": cfg, "as_of": "2026-02-01" })),
        )
        .await;
    assert_eq!(mismatch.status(), StatusCode::CONFLICT);
    assert_eq!(
        Harness::error_code(&Harness::body_json(mismatch).await),
        "IDEMPOTENCY_KEY_MISMATCH"
    );

    // Two DISTINCT keys created two runs; the replay did not create a third.
    let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM recommendation_runs")
        .fetch_one(&h.member_pool().await)
        .await
        .unwrap();
    assert_eq!(runs, 2, "replay must not double-create for the same key");
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE job_type='recommendation'")
        .fetch_one(&h.member_pool().await)
        .await
        .unwrap();
    assert_eq!(jobs, 2, "no double enqueue for the same key");
    h.teardown().await;
}

#[tokio::test]
async fn http_recommendation_manual_keys_cannot_alias_scheduled_namespace() {
    let Some(mut h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let cfg = config_id(&h, &h.member).await;
    let body = json!({ "strategy_config_id": cfg, "as_of": "2026-01-31" });
    let public_key = "scheduled:user-controlled";
    let response = h
        .send(
            "POST",
            "/api/v1/recommendations/runs",
            Some(&h.member),
            true,
            Some("test-rid-1"),
            Some(public_key),
            Some(body.clone()),
        )
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let created = Harness::body_json(response).await;
    let queue_key: String =
        sqlx::query_scalar("SELECT idempotency_key FROM jobs WHERE id = $1::uuid")
            .bind(created["job_id"].as_str().unwrap())
            .fetch_one(&h.member_pool().await)
            .await
            .unwrap();
    assert_eq!(queue_key, "recommendation:manual:scheduled:user-controlled");

    // A real reserved scheduled identity remains separate from the manual
    // public key even when the user deliberately starts with `scheduled:`.
    let scheduled_key = format!("recommendation:scheduled:{}", "1".repeat(32));
    assert_ne!(queue_key, scheduled_key);
    let reserved_insert = sqlx::query(
        "INSERT INTO jobs (owner_user_id, job_type, idempotency_key, payload_json) \
         VALUES ($1, 'recommendation', $2, '{}'::jsonb)",
    )
    .bind(h.member.user_id)
    .bind(&scheduled_key)
    .execute(&h.member_pool().await)
    .await;
    assert_eq!(
        reserved_insert
            .as_ref()
            .err()
            .and_then(|e| e.as_database_error())
            .and_then(|e| e.code())
            .as_deref(),
        Some("42501")
    );

    h.restart_api().await;
    let oversized = "x".repeat(256);
    let rejected = h
        .send(
            "POST",
            "/api/v1/recommendations/runs",
            Some(&h.member),
            true,
            Some("test-rid-1"),
            Some(&oversized),
            Some(body),
        )
        .await;
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        Harness::error_code(&Harness::body_json(rejected).await),
        "INVALID_PARAMETER"
    );
    h.teardown().await;
}

#[tokio::test]
async fn http_recommendations_queue_failure_rolls_back_run_and_job() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let cfg = config_id(&h, &h.member).await;
    h.seed_shared(
        "CREATE FUNCTION fail_recommendation_queue_insert() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION 'queue intentionally unavailable'; END $$",
    )
    .await;
    h.seed_shared(
        "CREATE TRIGGER fail_recommendation_queue_insert BEFORE INSERT ON jobs \
         FOR EACH ROW WHEN (NEW.job_type = 'recommendation') \
         EXECUTE FUNCTION fail_recommendation_queue_insert()",
    )
    .await;

    let resp = h
        .post(
            "/api/v1/recommendations/runs",
            Some(&h.member),
            true,
            json!({ "strategy_config_id": cfg, "as_of": "2026-01-31" }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::INTERNAL_SERVER_ERROR);
    let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM recommendation_runs")
        .fetch_one(&h.member_pool().await)
        .await
        .unwrap();
    let jobs: i64 =
        sqlx::query_scalar("SELECT count(*) FROM jobs WHERE job_type = 'recommendation'")
            .fetch_one(&h.member_pool().await)
            .await
            .unwrap();
    assert_eq!(runs, 0, "queue failure may not leave a PENDING run");
    assert_eq!(jobs, 0, "queue failure rolls back the job too");
    h.teardown().await;
}

#[tokio::test]
async fn configured_ready_dataset_attestation_is_app_only_and_locks_mutation() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let pin = h.state().cfg.recommendation_dataset.clone();
    let app_pool = h.member_pool().await;
    let mut attestation_tx = app_pool.begin().await.unwrap();
    let attested: bool =
        sqlx::query_scalar("SELECT public.lock_recommendation_submission_dataset($1, $2, $3, $4)")
            .bind(pin.id)
            .bind(&pin.dataset_id)
            .bind(&pin.version)
            .bind(&pin.manifest_sha256)
            .fetch_one(&mut *attestation_tx)
            .await
            .unwrap();
    assert!(attested);
    let mismatched: bool = sqlx::query_scalar(
        "SELECT public.lock_recommendation_submission_dataset($1, $2, $3, repeat('0', 64))",
    )
    .bind(pin.id)
    .bind(&pin.dataset_id)
    .bind(&pin.version)
    .fetch_one(&mut *attestation_tx)
    .await
    .unwrap();
    assert!(!mismatched);

    let grants: (bool, bool, bool, bool) = sqlx::query_as(
        "SELECT has_function_privilege('app', 'public.lock_recommendation_submission_dataset(uuid,text,text,text)', 'EXECUTE'), \
                has_function_privilege('worker', 'public.lock_recommendation_submission_dataset(uuid,text,text,text)', 'EXECUTE'), \
                has_function_privilege('admin', 'public.lock_recommendation_submission_dataset(uuid,text,text,text)', 'EXECUTE'), \
                EXISTS (SELECT 1 FROM pg_proc p, LATERAL aclexplode(p.proacl) acl \
                         WHERE p.oid='public.lock_recommendation_submission_dataset(uuid,text,text,text)'::regprocedure \
                           AND acl.grantee=0 AND acl.privilege_type='EXECUTE')",
    )
    .fetch_one(&h.owner_pool)
    .await
    .unwrap();
    assert_eq!(grants, (true, false, false, false));

    let mut mutation_tx = h.owner_pool.begin().await.unwrap();
    sqlx::query("SET LOCAL lock_timeout = '150ms'")
        .execute(&mut *mutation_tx)
        .await
        .unwrap();
    let blocked = sqlx::query("UPDATE dataset_versions SET status = 'BLOCKED' WHERE id = $1")
        .bind(pin.id)
        .execute(&mut *mutation_tx)
        .await;
    assert_eq!(
        blocked
            .as_ref()
            .err()
            .and_then(|e| e.as_database_error())
            .and_then(|e| e.code())
            .as_deref(),
        Some("55P03"),
        "FOR SHARE attestation must close the READY-check mutation race"
    );
    mutation_tx.rollback().await.unwrap();
    attestation_tx.commit().await.unwrap();

    let mut mutation_tx = h.owner_pool.begin().await.unwrap();
    sqlx::query("UPDATE dataset_versions SET status = 'BLOCKED' WHERE id = $1")
        .bind(pin.id)
        .execute(&mut *mutation_tx)
        .await
        .unwrap();
    mutation_tx.rollback().await.unwrap();
    h.teardown().await;
}

#[tokio::test]
async fn http_recommendations_capacity_is_per_owner_and_typed() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let cfg = config_id(&h, &h.member).await;
    h.seed_tenant(
        &h.member,
        &format!(
            "INSERT INTO jobs (owner_user_id, job_type, idempotency_key, payload_json) \
             SELECT '{owner}', 'other', 'occupied-' || gs::text, '{{}}'::jsonb \
             FROM generate_series(1, 10) AS gs",
            owner = h.member.user_id
        ),
    )
    .await;
    let resp = h
        .post(
            "/api/v1/recommendations/runs",
            Some(&h.member),
            true,
            json!({ "strategy_config_id": cfg, "as_of": "2026-01-31" }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        Harness::error_code(&Harness::body_json(resp).await),
        "RECOMMENDATION_CAPACITY_EXCEEDED"
    );
    let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM recommendation_runs")
        .fetch_one(&h.member_pool().await)
        .await
        .unwrap();
    assert_eq!(runs, 0);
    h.teardown().await;
}

#[tokio::test]
async fn concurrent_backtest_and_recommendation_share_one_owner_capacity_slot() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let cfg = config_id(&h, &h.member).await;
    let dataset_id = h.state().cfg.recommendation_dataset.id;
    h.seed_tenant(
        &h.member,
        &format!(
            "INSERT INTO jobs (owner_user_id, job_type, idempotency_key, payload_json) \
             SELECT '{owner}', 'occupied', 'capacity-race-' || gs::text, '{{}}'::jsonb \
             FROM generate_series(1, 9) AS gs",
            owner = h.member.user_id
        ),
    )
    .await;

    let recommendation = h.send(
        "POST",
        "/api/v1/recommendations/runs",
        Some(&h.member),
        true,
        Some("capacity-race-rec"),
        Some("capacity-race-rec"),
        Some(json!({ "strategy_config_id": cfg, "as_of": "2026-01-31" })),
    );
    let backtest = h.send(
        "POST",
        "/api/v1/backtests",
        Some(&h.member),
        true,
        Some("capacity-race-backtest"),
        Some("capacity-race-backtest"),
        Some(json!({
            "strategy_config_id": cfg,
            "dataset_version_id": dataset_id,
            "start_date": "2026-01-05",
            "end_date": "2026-01-30",
            "initial_cash": { "currency": "KRW", "amount": "100000000" },
            "benchmark": "069500.KRX",
            "cost_profile_id": "KRX_ETF_DEFAULT",
            "execution_profile": "daily-close-next-open@1",
            "robustness": false
        })),
    );
    let (recommendation, backtest) = tokio::join!(recommendation, backtest);
    let statuses = [recommendation.status(), backtest.status()];
    assert_eq!(
        statuses
            .iter()
            .filter(|&&s| s == StatusCode::CREATED)
            .count(),
        1,
        "exactly one producer may reserve the final global owner slot"
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|&&s| s == StatusCode::TOO_MANY_REQUESTS)
            .count(),
        1
    );
    let active: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM jobs WHERE owner_user_id = $1 AND status IN ('QUEUED', 'RUNNING')",
    )
    .bind(h.member.user_id)
    .fetch_one(&h.member_pool().await)
    .await
    .unwrap();
    assert_eq!(active, 10);
    h.teardown().await;
}

#[tokio::test]
async fn http_recommendations_latest_keeps_success_visible_behind_new_pending_run() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let cfg = config_id(&h, &h.member).await;
    let first = h
        .send(
            "POST",
            "/api/v1/recommendations/runs",
            Some(&h.member),
            true,
            Some("test-rid-1"),
            Some("successful-rec"),
            Some(json!({ "strategy_config_id": cfg, "as_of": "2026-01-30" })),
        )
        .await;
    let successful_id = Harness::body_json(first).await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    h.seed_tenant(
        &h.member,
        &format!(
            "UPDATE recommendation_runs SET status = 'SUCCEEDED', summary_json = '{{\"selected\":1}}'::jsonb \
             WHERE id = '{successful_id}'"
        ),
    )
    .await;
    let newest = h
        .send(
            "POST",
            "/api/v1/recommendations/runs",
            Some(&h.member),
            true,
            Some("test-rid-1"),
            Some("pending-rec"),
            Some(json!({ "strategy_config_id": cfg, "as_of": "2026-01-31" })),
        )
        .await;
    let newest_id = Harness::body_json(newest).await["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let response = h
        .get(
            &format!("/api/v1/recommendations/latest?strategy_config_id={cfg}"),
            Some(&h.member),
        )
        .await;
    assert_eq!(status(&response), StatusCode::OK);
    let body = Harness::body_json(response).await;
    assert_eq!(body["run"]["id"], successful_id);
    assert_eq!(body["run"]["status"], "SUCCEEDED");
    assert_eq!(body["latest_run"]["id"], newest_id);
    assert_eq!(body["latest_run"]["status"], "PENDING");

    let first_page = h
        .get("/api/v1/recommendations/runs?limit=1", Some(&h.member))
        .await;
    let first_page = Harness::body_json(first_page).await;
    assert_eq!(
        first_page["items"][0]["id"], newest_id,
        "history is newest first"
    );
    let cursor = first_page["next_cursor"]
        .as_str()
        .expect("second page cursor");
    let second_page = h
        .get(
            &format!("/api/v1/recommendations/runs?limit=1&cursor={cursor}"),
            Some(&h.member),
        )
        .await;
    let second_page = Harness::body_json(second_page).await;
    assert_eq!(second_page["items"][0]["id"], successful_id);
    h.teardown().await;
}

#[tokio::test]
async fn latest_metadata_and_items_are_read_from_one_database_snapshot() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let cfg = config_id(&h, &h.member).await;
    let created = h
        .send(
            "POST",
            "/api/v1/recommendations/runs",
            Some(&h.member),
            true,
            Some("snapshot-seed"),
            Some("snapshot-seed"),
            Some(json!({ "strategy_config_id": cfg, "as_of": "2026-01-31" })),
        )
        .await;
    let run_id = Harness::body_json(created).await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    h.seed_tenant(
        &h.member,
        &format!(
            "UPDATE recommendation_runs SET status='SUCCEEDED', summary_json='{{\"generation\":0}}'::jsonb WHERE id='{run_id}'"
        ),
    )
    .await;
    h.seed_tenant(
        &h.member,
        &format!(
            "INSERT INTO recommendation_items \
                 (recommendation_run_id, owner_user_id, instrument_id, rank, target_weight, factors_json) \
             VALUES ('{run_id}', '{owner}', '069500.KRX', 1, 1, '{{\"generation\":0}}'::jsonb)",
            owner = h.member.user_id
        ),
    )
    .await;

    let writer_pool = h.member_pool().await;
    let writer_run = Uuid::parse_str(&run_id).unwrap();
    let writer = tokio::spawn(async move {
        for generation in 1..=200_i32 {
            sqlx::query(
                "WITH changed AS ( \
                    UPDATE recommendation_runs SET summary_json=jsonb_build_object('generation', $2) \
                    WHERE id=$1 RETURNING id \
                 ) \
                 UPDATE recommendation_items SET factors_json=jsonb_build_object('generation', $2) \
                 WHERE recommendation_run_id=(SELECT id FROM changed)",
            )
            .bind(writer_run)
            .bind(generation)
            .execute(&writer_pool)
            .await
            .unwrap();
            tokio::task::yield_now().await;
        }
    });
    for _ in 0..100 {
        let response = h
            .get(
                &format!("/api/v1/recommendations/latest?strategy_config_id={cfg}"),
                Some(&h.member),
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = Harness::body_json(response).await;
        assert_eq!(
            body["run"]["summary"]["generation"], body["run"]["items"][0]["factors"]["generation"],
            "metadata and items must never straddle a publisher commit"
        );
    }
    writer.await.unwrap();
    h.teardown().await;
}

#[tokio::test]
async fn http_recommendations_fuzz_rejects_invalid_input() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let cfg = config_id(&h, &h.member).await;
    // Invalid date.
    let resp = h
        .post(
            "/api/v1/recommendations/runs",
            Some(&h.member),
            true,
            json!({ "strategy_config_id": cfg, "as_of": "2026-13-45" }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::BAD_REQUEST);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "INVALID_DATE");

    // Invalid uuid.
    let resp = h
        .post(
            "/api/v1/recommendations/runs",
            Some(&h.member),
            true,
            json!({ "strategy_config_id": "not-a-uuid", "as_of": "2026-01-31" }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::BAD_REQUEST);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "INVALID_PARAMETER");

    // No rows created by any fuzz input.
    let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM recommendation_runs")
        .fetch_one(&h.member_pool().await)
        .await
        .unwrap();
    assert_eq!(runs, 0, "fuzz must not create rows");
    h.teardown().await;
}
