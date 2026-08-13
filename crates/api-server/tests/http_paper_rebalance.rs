//! Actor-scoped recommendation-to-Paper rebalance preview HTTP contract.

mod common;

use axum::http::StatusCode;
use common::{Harness, actor_pool};
use domain::{ContentHash, Currency, InstrumentId, Price, TradingDate, UtcTimestamp};
use job_queue::paper_preview::{PreviewRunOutcome, run_preview_once};
use job_queue::{JobQueue, QueueConfig};
use market_data::CurateStore;
use market_data::curate::schema::{CuratedBar, write_bars};
use serde_json::{Value, json};
use std::time::Duration;
use uuid::Uuid;

struct PreviewInputs {
    account_id: Uuid,
    run_id: Uuid,
}

async fn ready_inputs(h: &Harness) -> PreviewInputs {
    let account = h
        .send(
            "POST",
            "/api/v1/paper/accounts",
            Some(&h.owner),
            true,
            Some("preview-account-rid"),
            Some("preview-account-key"),
            Some(json!({
                "name": "preview-account",
                "currency": "KRW",
                "initial_cash": "10000000"
            })),
        )
        .await;
    assert_eq!(account.status(), StatusCode::CREATED);
    let account_id =
        Uuid::parse_str(Harness::body_json(account).await["id"].as_str().unwrap()).unwrap();
    let config = h
        .send(
            "POST",
            "/api/v1/strategies/buy_and_hold/configs",
            Some(&h.owner),
            true,
            Some("preview-config-rid"),
            Some("preview-config-key"),
            Some(json!({
                "strategy_version": "1.0.0",
                "config": { "lookback": 200 },
                "is_active": true
            })),
        )
        .await;
    assert_eq!(config.status(), StatusCode::CREATED);
    let config_id =
        Uuid::parse_str(Harness::body_json(config).await["id"].as_str().unwrap()).unwrap();
    let binding = h
        .send(
            "POST",
            &format!("/api/v1/paper/accounts/{account_id}/bind-strategy"),
            Some(&h.owner),
            true,
            Some("preview-bind-rid"),
            Some("preview-bind-key"),
            Some(json!({
                "strategy_config_id": config_id,
                "auto_apply_recommendations": false
            })),
        )
        .await;
    assert_eq!(binding.status(), StatusCode::OK);

    h.seed_shared(
        "INSERT INTO trading_calendars \
         (exchange,session_date,session_type,timezone,source,source_version, \
          source_batch_id,content_sha256,retrieved_at) \
         VALUES ('KRX',DATE '2026-08-14','TRADING','Asia/Seoul','KRX','preview-http', \
                 gen_random_uuid(),repeat('9',64),now()) \
         ON CONFLICT (exchange,session_date) DO UPDATE SET \
           session_type='TRADING', timezone='Asia/Seoul', \
           source_batch_id=EXCLUDED.source_batch_id, \
           content_sha256=EXCLUDED.content_sha256, retrieved_at=EXCLUDED.retrieved_at",
    )
    .await;
    let pool = actor_pool(&h.app_url, &h.owner.user_id.to_string(), 2).await;
    let dataset = h.state().cfg.recommendation_dataset.clone();
    let source_job_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO jobs \
         (id,owner_user_id,job_type,status,idempotency_key,payload_json) \
         VALUES ($1,$2,'recommendation','SUCCEEDED',$3,$4)",
    )
    .bind(source_job_id)
    .bind(h.owner.user_id)
    .bind(format!("preview-source-{}", Uuid::new_v4()))
    .bind(json!({
        "dataset": {
            "id": dataset.id,
            "dataset_id": dataset.dataset_id,
            "version": dataset.version,
            "curated_version": dataset.curated_version,
            "manifest_sha256": dataset.manifest_sha256
        }
    }))
    .execute(&pool)
    .await
    .unwrap();
    let run_id: Uuid = sqlx::query_scalar(
        "INSERT INTO recommendation_runs \
         (owner_user_id,strategy_config_id,as_of,status,job_id,trigger_kind, \
          dataset_version_id,dataset_manifest_sha256) \
         VALUES ($1,$2,DATE '2026-08-12','SUCCEEDED',$3,'MANUAL',$4,$5) RETURNING id",
    )
    .bind(h.owner.user_id)
    .bind(config_id)
    .bind(source_job_id)
    .bind(dataset.id)
    .bind(&dataset.manifest_sha256)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO target_portfolios \
         (owner_user_id,recommendation_run_id,as_of,weights_json) \
         VALUES ($1,$2,DATE '2026-08-12','{\"069500.KRX\":\"1.000000\"}'::jsonb)",
    )
    .bind(h.owner.user_id)
    .bind(run_id)
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;
    PreviewInputs { account_id, run_id }
}

async fn create(
    h: &Harness,
    input: &PreviewInputs,
    key: &str,
    run_id: Uuid,
) -> axum::response::Response {
    h.send(
        "POST",
        &format!(
            "/api/v1/paper/accounts/{}/recommendation-previews",
            input.account_id
        ),
        Some(&h.owner),
        true,
        Some("preview-create-rid"),
        Some(key),
        Some(json!({ "recommendation_run_id": run_id })),
    )
    .await
}

struct ReadyPreview {
    id: Uuid,
    token: String,
    body: Value,
}

async fn finish_preview(h: &Harness, input: &PreviewInputs, key: &str) -> ReadyPreview {
    let created = create(h, input, key, input.run_id).await;
    assert_eq!(created.status(), StatusCode::ACCEPTED);
    let created = Harness::body_json(created).await;
    let preview_id = Uuid::parse_str(created["id"].as_str().unwrap()).unwrap();

    let data = tempfile::tempdir().unwrap();
    let store = CurateStore::new(data.path().join("curated"));
    let instrument = InstrumentId::parse("069500.KRX").unwrap();
    let price = Price::parse("10000").unwrap();
    write_bars(
        &store.bars_path("kr", "069500.KRX", 2026, 2),
        &[CuratedBar {
            instrument_id: instrument,
            trading_date: TradingDate::parse("2026-08-12").unwrap(),
            market_open_ts: UtcTimestamp::parse_rfc3339("2026-08-12T00:00:00Z").unwrap(),
            market_close_ts: UtcTimestamp::parse_rfc3339("2026-08-12T06:30:00Z").unwrap(),
            open: price,
            high: price,
            low: price,
            close: price,
            volume: 1,
            trading_value: Some(10_000),
            currency: Currency::KRW,
            source: "test".into(),
            ingested_at: UtcTimestamp::parse_rfc3339("2026-08-12T07:00:00Z").unwrap(),
            batch_id: Uuid::new_v4().to_string().parse().unwrap(),
            raw_hash: ContentHash::from_bytes(b"preview-http-bar"),
        }],
    )
    .unwrap();
    let worker = h.worker_pool().await;
    let queue = JobQueue::new(
        worker.clone(),
        None,
        QueueConfig {
            lease: Duration::from_secs(10),
            backoff_base: Duration::from_millis(1),
        },
    );
    let outcome = run_preview_once(
        &worker,
        &queue,
        data.path(),
        "http-preview-worker",
        chrono::NaiveDate::from_ymd_opt(2026, 8, 13).unwrap(),
        Duration::from_millis(100),
    )
    .await
    .unwrap();
    assert!(matches!(outcome, PreviewRunOutcome::Published { .. }));
    worker.close().await;

    let read = h
        .get(
            &format!(
                "/api/v1/paper/accounts/{}/recommendation-previews/{preview_id}",
                input.account_id
            ),
            Some(&h.owner),
        )
        .await;
    assert_eq!(read.status(), StatusCode::OK);
    let body = Harness::body_json(read).await;
    ReadyPreview {
        id: preview_id,
        token: body["preview_token"].as_str().unwrap().to_owned(),
        body,
    }
}

async fn apply_preview(
    h: &Harness,
    input: &PreviewInputs,
    ready: &ReadyPreview,
    key: &str,
    token: &str,
) -> axum::response::Response {
    h.send(
        "POST",
        &format!(
            "/api/v1/paper/accounts/{}/recommendation-previews/{}/apply",
            input.account_id, ready.id
        ),
        Some(&h.owner),
        true,
        Some("preview-apply-rid"),
        Some(key),
        Some(json!({ "preview_token": token })),
    )
    .await
}

#[tokio::test]
async fn create_preview_is_atomic_readable_and_durably_idempotent() {
    let Some(mut h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let input = ready_inputs(&h).await;
    let created = create(&h, &input, "preview-create-key", input.run_id).await;
    let created_status = created.status();
    let created = Harness::body_json(created).await;
    assert_eq!(created_status, StatusCode::ACCEPTED, "{created}");
    assert_eq!(created["account_id"], input.account_id.to_string());
    assert_eq!(created["recommendation_run_id"], input.run_id.to_string());
    assert_eq!(created["status"], "PENDING");
    assert!(created.get("result").is_none());
    let preview_id = Uuid::parse_str(created["id"].as_str().unwrap()).unwrap();
    let job_id = Uuid::parse_str(created["job_id"].as_str().unwrap()).unwrap();
    let owner_pool = actor_pool(&h.app_url, &h.owner.user_id.to_string(), 2).await;
    let row: (String, Value, i64) = sqlx::query_as(
        "SELECT job.status,job.payload_json, \
                (SELECT count(*) FROM paper_rebalance_previews WHERE job_id=job.id) \
         FROM jobs AS job WHERE job.id=$1",
    )
    .bind(job_id)
    .fetch_one(&owner_pool)
    .await
    .unwrap();
    assert_eq!(row.0, "QUEUED");
    assert_eq!(row.1, json!({ "preview_id": preview_id }));
    assert_eq!(row.2, 1);
    owner_pool.close().await;

    let read = h
        .get(
            &format!(
                "/api/v1/paper/accounts/{}/recommendation-previews/{preview_id}",
                input.account_id
            ),
            Some(&h.owner),
        )
        .await;
    assert_eq!(read.status(), StatusCode::OK);
    assert_eq!(Harness::body_json(read).await["status"], "PENDING");

    let other = h
        .seed_user(
            auth::entitlement::Role::Owner,
            "preview-other@lagrange.test",
            "preview-other-iss",
            "preview-other-sub",
        )
        .await;
    let hidden = h
        .get(
            &format!(
                "/api/v1/paper/accounts/{}/recommendation-previews/{preview_id}",
                input.account_id
            ),
            Some(&other),
        )
        .await;
    assert_eq!(hidden.status(), StatusCode::NOT_FOUND);

    h.restart_api().await;
    let replay = create(&h, &input, "preview-create-key", input.run_id).await;
    assert_eq!(replay.status(), StatusCode::OK);
    let replay = Harness::body_json(replay).await;
    assert_eq!(replay["id"], preview_id.to_string());
    assert_eq!(replay["job_id"], job_id.to_string());

    let mismatch = create(&h, &input, "preview-create-key", Uuid::new_v4()).await;
    assert_eq!(mismatch.status(), StatusCode::CONFLICT);
    assert_eq!(
        Harness::body_json(mismatch).await["error"]["code"],
        "IDEMPOTENCY_KEY_MISMATCH"
    );
    let correct = create(&h, &input, "preview-create-key", input.run_id).await;
    assert_eq!(correct.status(), StatusCode::OK);
    assert_eq!(
        Harness::body_json(correct).await["id"],
        preview_id.to_string()
    );
    h.teardown().await;
}

#[tokio::test]
async fn create_preview_rejects_invalid_lifecycle_and_global_capacity_without_rows() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let input = ready_inputs(&h).await;
    let pool = actor_pool(&h.app_url, &h.owner.user_id.to_string(), 2).await;

    sqlx::query("UPDATE recommendation_runs SET status='FAILED' WHERE id=$1")
        .bind(input.run_id)
        .execute(&pool)
        .await
        .unwrap();
    let failed = create(&h, &input, "preview-failed-run", input.run_id).await;
    assert_eq!(failed.status(), StatusCode::CONFLICT);
    assert_eq!(
        Harness::body_json(failed).await["error"]["code"],
        "REBALANCE_PREVIEW_NOT_READY"
    );
    sqlx::query("UPDATE recommendation_runs SET status='SUCCEEDED' WHERE id=$1")
        .bind(input.run_id)
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query(
        "UPDATE account_strategy_bindings SET unbound_at=now() \
         WHERE account_id=$1 AND unbound_at IS NULL",
    )
    .bind(input.account_id)
    .execute(&pool)
    .await
    .unwrap();
    let unbound = create(&h, &input, "preview-unbound", input.run_id).await;
    assert_eq!(unbound.status(), StatusCode::CONFLICT);
    assert_eq!(
        Harness::body_json(unbound).await["error"]["code"],
        "REBALANCE_PREVIEW_BINDING_REQUIRED"
    );
    let config_id: Uuid =
        sqlx::query_scalar("SELECT strategy_config_id FROM recommendation_runs WHERE id=$1")
            .bind(input.run_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let rebound = h
        .send(
            "POST",
            &format!("/api/v1/paper/accounts/{}/bind-strategy", input.account_id),
            Some(&h.owner),
            true,
            Some("preview-rebind-rid"),
            Some("preview-rebind-key"),
            Some(json!({
                "strategy_config_id": config_id,
                "auto_apply_recommendations": false
            })),
        )
        .await;
    assert_eq!(rebound.status(), StatusCode::OK);

    sqlx::query("UPDATE dataset_versions SET status='BLOCKED' WHERE id=$1")
        .bind(h.state().cfg.recommendation_dataset.id)
        .execute(&h.owner_pool)
        .await
        .unwrap();
    let blocked = create(&h, &input, "preview-blocked", input.run_id).await;
    assert_eq!(blocked.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        Harness::body_json(blocked).await["error"]["code"],
        "REBALANCE_PREVIEW_DATA_BLOCKED"
    );
    sqlx::query("UPDATE dataset_versions SET status='READY' WHERE id=$1")
        .bind(h.state().cfg.recommendation_dataset.id)
        .execute(&h.owner_pool)
        .await
        .unwrap();

    sqlx::query("UPDATE data_entitlements SET status='REVOKED' WHERE status='ACTIVE'")
        .execute(&h.owner_pool)
        .await
        .unwrap();
    let revoked = create(&h, &input, "preview-revoked", input.run_id).await;
    assert_eq!(revoked.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        Harness::body_json(revoked).await["error"]["code"],
        "DATA_ENTITLEMENT_REQUIRED"
    );
    sqlx::query("UPDATE data_entitlements SET status='ACTIVE' WHERE status='REVOKED'")
        .execute(&h.owner_pool)
        .await
        .unwrap();

    for index in 0..10 {
        sqlx::query(
            "INSERT INTO jobs \
             (owner_user_id,job_type,status,idempotency_key,payload_json) \
             VALUES ($1,'backtest','QUEUED',$2,'{}'::jsonb)",
        )
        .bind(h.owner.user_id)
        .bind(format!("preview-capacity-{index}"))
        .execute(&pool)
        .await
        .unwrap();
    }
    let capacity = create(&h, &input, "preview-capacity", input.run_id).await;
    assert_eq!(capacity.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        Harness::body_json(capacity).await["error"]["code"],
        "REBALANCE_PREVIEW_CAPACITY_EXCEEDED"
    );
    let previews: i64 = sqlx::query_scalar("SELECT count(*) FROM paper_rebalance_previews")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(previews, 0);
    pool.close().await;
    h.teardown().await;
}

#[tokio::test]
async fn concurrent_preview_submissions_share_the_global_owner_capacity() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let input = ready_inputs(&h).await;
    let pool = actor_pool(&h.app_url, &h.owner.user_id.to_string(), 2).await;
    for index in 0..9 {
        sqlx::query(
            "INSERT INTO jobs \
             (owner_user_id,job_type,status,idempotency_key,payload_json) \
             VALUES ($1,'backtest','QUEUED',$2,'{}'::jsonb)",
        )
        .bind(h.owner.user_id)
        .bind(format!("preview-concurrent-capacity-{index}"))
        .execute(&pool)
        .await
        .unwrap();
    }
    let (first, second) = tokio::join!(
        create(&h, &input, "preview-concurrent-a", input.run_id),
        create(&h, &input, "preview-concurrent-b", input.run_id),
    );
    let mut statuses = [first.status(), second.status()];
    statuses.sort();
    assert_eq!(
        statuses,
        [StatusCode::ACCEPTED, StatusCode::TOO_MANY_REQUESTS]
    );
    let active: i64 =
        sqlx::query_scalar("SELECT count(*) FROM jobs WHERE status IN ('QUEUED','RUNNING')")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(active, 10);
    let previews: i64 = sqlx::query_scalar("SELECT count(*) FROM paper_rebalance_previews")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(previews, 1);
    pool.close().await;
    h.teardown().await;
}

#[tokio::test]
async fn worker_completion_is_returned_as_a_strict_ready_preview() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let input = ready_inputs(&h).await;
    let ready = finish_preview(&h, &input, "preview-worker-key").await;
    let read = ready.body;
    assert_eq!(read["status"], "READY");
    assert_eq!(read["result"]["schema_version"], 1);
    assert_eq!(read["result"]["price_basis"], "RECOMMENDATION_CLOSE");
    assert_eq!(
        read["result"]["lineage"]["recommendation_run_id"],
        input.run_id.to_string()
    );
    assert_eq!(read["result"]["orders"].as_array().unwrap().len(), 1);
    assert!(read["preview_token"].as_str().is_some());
    h.teardown().await;
}

#[tokio::test]
async fn apply_ready_preview_creates_one_manual_target_without_ledger_writes_and_replays() {
    let Some(mut h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let input = ready_inputs(&h).await;
    let ready = finish_preview(&h, &input, "preview-apply-ready").await;
    let path = format!(
        "/api/v1/paper/accounts/{}/recommendation-previews/{}/apply",
        input.account_id, ready.id
    );
    let applied = h
        .send(
            "POST",
            &path,
            Some(&h.owner),
            true,
            Some("preview-apply-rid"),
            Some("preview-apply-key"),
            Some(json!({ "preview_token": ready.token })),
        )
        .await;
    let applied_status = applied.status();
    let applied = Harness::body_json(applied).await;
    assert_eq!(applied_status, StatusCode::OK, "{applied}");
    assert_eq!(applied["preview_id"], ready.id.to_string());
    assert_eq!(applied["status"], "APPLIED");
    assert_eq!(applied["effective_date"], "2026-08-14");
    assert_eq!(applied["source_kind"], "MANUAL_RECOMMENDATION");
    let pending_target_id = applied["pending_target_id"].as_str().unwrap().to_owned();

    let pool = actor_pool(&h.app_url, &h.owner.user_id.to_string(), 2).await;
    let persisted: (String, String, String, Uuid, Value, Uuid, String, i64, i64) = sqlx::query_as(
        "SELECT preview.status,target.source_kind,target.status,target.recommendation_run_id, \
                target.targets_json,target.dataset_version_id,target.dataset_manifest_sha256, \
                (SELECT count(*) FROM orders WHERE account_id=preview.account_id), \
                (SELECT count(*) FROM fills WHERE account_id=preview.account_id) \
         FROM paper_rebalance_previews AS preview \
         JOIN pending_targets AS target ON target.id=preview.pending_target_id \
         WHERE preview.id=$1",
    )
    .bind(ready.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(persisted.0, "APPLIED");
    assert_eq!(persisted.1, "MANUAL_RECOMMENDATION");
    assert_eq!(persisted.2, "PENDING");
    assert_eq!(persisted.3, input.run_id);
    assert_eq!(
        persisted.4,
        json!([{"instrument_id":"069500.KRX","weight":"1.000000"}])
    );
    assert_eq!(persisted.5, h.state().cfg.recommendation_dataset.id);
    assert_eq!(
        persisted.6,
        h.state().cfg.recommendation_dataset.manifest_sha256
    );
    assert_eq!((persisted.7, persisted.8), (0, 0));
    pool.close().await;

    h.restart_api().await;
    let replay = h
        .send(
            "POST",
            &path,
            Some(&h.owner),
            true,
            Some("preview-apply-replay-rid"),
            Some("preview-apply-key"),
            Some(json!({ "preview_token": ready.token })),
        )
        .await;
    assert_eq!(replay.status(), StatusCode::OK);
    assert_eq!(
        Harness::body_json(replay).await["pending_target_id"],
        pending_target_id
    );
    h.teardown().await;
}

#[tokio::test]
async fn apply_preview_rejects_not_ready_wrong_token_and_foreign_tenant() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let input = ready_inputs(&h).await;
    let pending = create(&h, &input, "preview-not-ready", input.run_id).await;
    assert_eq!(pending.status(), StatusCode::ACCEPTED);
    let pending = Harness::body_json(pending).await;
    let not_ready = ReadyPreview {
        id: Uuid::parse_str(pending["id"].as_str().unwrap()).unwrap(),
        token: "a".repeat(64),
        body: pending,
    };
    let response = apply_preview(
        &h,
        &input,
        &not_ready,
        "preview-not-ready-apply",
        &not_ready.token,
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        Harness::body_json(response).await["error"]["code"],
        "REBALANCE_PREVIEW_NOT_READY"
    );
    let pool = actor_pool(&h.app_url, &h.owner.user_id.to_string(), 1).await;
    sqlx::query("UPDATE jobs SET status='CANCELED' WHERE id=$1")
        .bind(Uuid::parse_str(not_ready.body["job_id"].as_str().unwrap()).unwrap())
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let ready = finish_preview(&h, &input, "preview-invalid-apply").await;
    let wrong = apply_preview(&h, &input, &ready, "preview-wrong-token", &"b".repeat(64)).await;
    assert_eq!(wrong.status(), StatusCode::CONFLICT);
    assert_eq!(
        Harness::body_json(wrong).await["error"]["code"],
        "REBALANCE_PREVIEW_STALE"
    );

    let other = h
        .seed_user(
            auth::entitlement::Role::Owner,
            "preview-apply-other@lagrange.test",
            "preview-apply-other-iss",
            "preview-apply-other-sub",
        )
        .await;
    let foreign = h
        .send(
            "POST",
            &format!(
                "/api/v1/paper/accounts/{}/recommendation-previews/{}/apply",
                input.account_id, ready.id
            ),
            Some(&other),
            true,
            Some("preview-foreign-apply-rid"),
            Some("preview-foreign-apply-key"),
            Some(json!({ "preview_token": ready.token })),
        )
        .await;
    assert_eq!(foreign.status(), StatusCode::NOT_FOUND);
    h.teardown().await;
}

#[tokio::test]
async fn apply_preview_fails_closed_after_account_target_or_shared_input_changes() {
    async fn assert_stale(h: &Harness, input: &PreviewInputs, ready: &ReadyPreview, key: &str) {
        let response = apply_preview(h, input, ready, key, &ready.token).await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(
            Harness::body_json(response).await["error"]["code"],
            "REBALANCE_PREVIEW_STALE"
        );
    }

    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let input = ready_inputs(&h).await;
    let pool = actor_pool(&h.app_url, &h.owner.user_id.to_string(), 2).await;

    let cash = finish_preview(&h, &input, "preview-stale-cash").await;
    sqlx::query("UPDATE cash_ledger SET ts=ts+interval '1 second' WHERE account_id=$1")
        .bind(input.account_id)
        .execute(&pool)
        .await
        .unwrap();
    assert_stale(&h, &input, &cash, "preview-apply-stale-cash").await;

    let position = finish_preview(&h, &input, "preview-stale-position").await;
    sqlx::query(
        "INSERT INTO positions \
         (account_id,owner_user_id,instrument_id,quantity) VALUES ($1,$2,'069500.KRX',1)",
    )
    .bind(input.account_id)
    .bind(h.owner.user_id)
    .execute(&pool)
    .await
    .unwrap();
    assert_stale(&h, &input, &position, "preview-apply-stale-position").await;

    let account = finish_preview(&h, &input, "preview-stale-account").await;
    sqlx::query("UPDATE accounts SET status='SUSPENDED' WHERE id=$1")
        .bind(input.account_id)
        .execute(&pool)
        .await
        .unwrap();
    assert_stale(&h, &input, &account, "preview-apply-stale-account").await;
    sqlx::query("UPDATE accounts SET status='ACTIVE' WHERE id=$1")
        .bind(input.account_id)
        .execute(&pool)
        .await
        .unwrap();

    let target = finish_preview(&h, &input, "preview-stale-target").await;
    let target_id = Uuid::parse_str(target.body["target_portfolio_id"].as_str().unwrap()).unwrap();
    sqlx::query(
        "UPDATE target_portfolios SET weights_json='{\"069500.KRX\":\"0.500000\"}'::jsonb \
         WHERE id=$1",
    )
    .bind(target_id)
    .execute(&pool)
    .await
    .unwrap();
    assert_stale(&h, &input, &target, "preview-apply-stale-target").await;
    sqlx::query(
        "UPDATE target_portfolios SET weights_json='{\"069500.KRX\":\"1.000000\"}'::jsonb \
         WHERE id=$1",
    )
    .bind(target_id)
    .execute(&pool)
    .await
    .unwrap();

    let binding = finish_preview(&h, &input, "preview-stale-binding").await;
    sqlx::query(
        "UPDATE account_strategy_bindings SET unbound_at=now() \
         WHERE account_id=$1 AND unbound_at IS NULL",
    )
    .bind(input.account_id)
    .execute(&pool)
    .await
    .unwrap();
    assert_stale(&h, &input, &binding, "preview-apply-stale-binding").await;
    let config_id = Uuid::parse_str(binding.body["strategy_config_id"].as_str().unwrap()).unwrap();
    sqlx::query(
        "INSERT INTO account_strategy_bindings \
         (account_id,owner_user_id,strategy_config_id,strategy_id,strategy_version) \
         VALUES ($1,$2,$3,'buy_and_hold','1.0.0')",
    )
    .bind(input.account_id)
    .bind(h.owner.user_id)
    .bind(config_id)
    .execute(&pool)
    .await
    .unwrap();

    let dataset = finish_preview(&h, &input, "preview-stale-dataset").await;
    sqlx::query("UPDATE dataset_versions SET status='BLOCKED' WHERE id=$1")
        .bind(h.state().cfg.recommendation_dataset.id)
        .execute(&h.owner_pool)
        .await
        .unwrap();
    assert_stale(&h, &input, &dataset, "preview-apply-stale-dataset").await;
    sqlx::query("UPDATE dataset_versions SET status='READY' WHERE id=$1")
        .bind(h.state().cfg.recommendation_dataset.id)
        .execute(&h.owner_pool)
        .await
        .unwrap();

    let entitlement = finish_preview(&h, &input, "preview-stale-entitlement").await;
    sqlx::query("UPDATE data_entitlements SET status='REVOKED' WHERE status='ACTIVE'")
        .execute(&h.owner_pool)
        .await
        .unwrap();
    let response = apply_preview(
        &h,
        &input,
        &entitlement,
        "preview-apply-stale-entitlement",
        &entitlement.token,
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        Harness::body_json(response).await["error"]["code"],
        "DATA_ENTITLEMENT_REQUIRED"
    );
    pool.close().await;
    h.teardown().await;
}

#[tokio::test]
async fn apply_preview_rejects_arrived_or_conflicting_session_and_serializes_replays() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let input = ready_inputs(&h).await;
    let arrived = finish_preview(&h, &input, "preview-arrived").await;
    let arrived_result = h
        .state()
        .rebalance_previews()
        .apply(
            &h.owner.actor(),
            input.account_id,
            arrived.id,
            &arrived.token,
            chrono::NaiveDate::from_ymd_opt(2026, 8, 14).unwrap(),
        )
        .await;
    assert!(matches!(
        arrived_result,
        Err(api_server::repos::rebalance_previews::ApplyRebalancePreviewError::Stale)
    ));

    let conflict = finish_preview(&h, &input, "preview-conflict").await;
    let pool = actor_pool(&h.app_url, &h.owner.user_id.to_string(), 2).await;
    let config_id = Uuid::parse_str(conflict.body["strategy_config_id"].as_str().unwrap()).unwrap();
    sqlx::query(
        "INSERT INTO pending_targets \
         (account_id,owner_user_id,strategy_config_id,computed_on,effective_date,targets_json) \
         VALUES ($1,$2,$3,DATE '2026-08-12',DATE '2026-08-14','[]'::jsonb)",
    )
    .bind(input.account_id)
    .bind(h.owner.user_id)
    .bind(config_id)
    .execute(&pool)
    .await
    .unwrap();
    let conflict_response = apply_preview(
        &h,
        &input,
        &conflict,
        "preview-apply-conflict",
        &conflict.token,
    )
    .await;
    assert_eq!(conflict_response.status(), StatusCode::CONFLICT);
    assert_eq!(
        Harness::body_json(conflict_response).await["error"]["code"],
        "REBALANCE_PREVIEW_CONFLICT"
    );
    sqlx::query("DELETE FROM pending_targets WHERE account_id=$1")
        .bind(input.account_id)
        .execute(&pool)
        .await
        .unwrap();

    let concurrent = finish_preview(&h, &input, "preview-concurrent-apply").await;
    let (first, second) = tokio::join!(
        apply_preview(
            &h,
            &input,
            &concurrent,
            "preview-concurrent-apply-a",
            &concurrent.token,
        ),
        apply_preview(
            &h,
            &input,
            &concurrent,
            "preview-concurrent-apply-b",
            &concurrent.token,
        ),
    );
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(second.status(), StatusCode::OK);
    let first = Harness::body_json(first).await;
    let second = Harness::body_json(second).await;
    assert_eq!(first["pending_target_id"], second["pending_target_id"]);
    let target_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pending_targets WHERE account_id=$1 AND source_kind='MANUAL_RECOMMENDATION'",
    )
    .bind(input.account_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(target_count, 1);
    pool.close().await;
    h.teardown().await;
}

#[tokio::test]
async fn apply_and_ledger_mutation_have_a_single_account_locked_order() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let input = ready_inputs(&h).await;
    let actor_pool = actor_pool(&h.app_url, &h.owner.user_id.to_string(), 4).await;

    let mutation_first = finish_preview(&h, &input, "preview-mutation-first").await;
    let mut mutation = actor_pool.begin().await.unwrap();
    sqlx::query("UPDATE cash_ledger SET ts=ts+interval '1 second' WHERE account_id=$1")
        .bind(input.account_id)
        .execute(&mut *mutation)
        .await
        .unwrap();
    let repo = h.state().rebalance_previews();
    let actor = h.owner.actor();
    let mutation_first_token = mutation_first.token.clone();
    let account_id = input.account_id;
    let preview_id = mutation_first.id;
    let applying = tokio::spawn(async move {
        repo.apply(
            &actor,
            account_id,
            preview_id,
            &mutation_first_token,
            chrono::NaiveDate::from_ymd_opt(2026, 8, 13).unwrap(),
        )
        .await
    });
    let mut observed_wait = false;
    for _ in 0..200 {
        observed_wait = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pg_stat_activity \
             WHERE datname=current_database() AND usename='app' \
               AND wait_event_type='Lock' \
               AND query LIKE '%paper_rebalance_previews AS preview%')",
        )
        .fetch_one(&h.owner_pool)
        .await
        .unwrap();
        if observed_wait {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        observed_wait || !applying.is_finished(),
        "apply must wait for the in-flight ledger mutation"
    );
    mutation.commit().await.unwrap();
    let result = applying.await.unwrap();
    assert!(matches!(
        result,
        Err(api_server::repos::rebalance_previews::ApplyRebalancePreviewError::Stale)
    ));

    let apply_first = finish_preview(&h, &input, "preview-apply-first").await;
    h.seed_shared(
        "CREATE FUNCTION block_manual_preview_apply() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN PERFORM pg_advisory_xact_lock(813038); RETURN NEW; END $$",
    )
    .await;
    h.seed_shared(
        "CREATE TRIGGER block_manual_preview_apply BEFORE INSERT ON pending_targets \
         FOR EACH ROW WHEN (NEW.source_kind='MANUAL_RECOMMENDATION') \
         EXECUTE FUNCTION block_manual_preview_apply()",
    )
    .await;
    let mut latch = h.owner_pool.acquire().await.unwrap();
    sqlx::query("SELECT pg_advisory_lock(813038)")
        .execute(&mut *latch)
        .await
        .unwrap();
    let repo = h.state().rebalance_previews();
    let actor = h.owner.actor();
    let apply_first_token = apply_first.token.clone();
    let preview_id = apply_first.id;
    let applying = tokio::spawn(async move {
        repo.apply(
            &actor,
            account_id,
            preview_id,
            &apply_first_token,
            chrono::NaiveDate::from_ymd_opt(2026, 8, 13).unwrap(),
        )
        .await
    });
    let mut apply_reached_insert = false;
    for _ in 0..200 {
        apply_reached_insert = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pg_locks \
             WHERE locktype='advisory' AND NOT granted)",
        )
        .fetch_one(&h.owner_pool)
        .await
        .unwrap();
        if apply_reached_insert {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        apply_reached_insert,
        "apply must reach the target insertion seam"
    );
    let mutation_pool = actor_pool.clone();
    let mutating = tokio::spawn(async move {
        sqlx::query("UPDATE cash_ledger SET ts=ts+interval '1 second' WHERE account_id=$1")
            .bind(account_id)
            .execute(&mutation_pool)
            .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !mutating.is_finished(),
        "ledger mutation must wait while apply holds account validity locks"
    );
    sqlx::query("SELECT pg_advisory_unlock(813038)")
        .execute(&mut *latch)
        .await
        .unwrap();
    let applied = applying.await.unwrap().unwrap();
    assert!(!applied.replayed);
    assert_eq!(applied.source_kind, "MANUAL_RECOMMENDATION");
    assert_eq!(mutating.await.unwrap().unwrap().rows_affected(), 1);
    actor_pool.close().await;
    h.teardown().await;
}
