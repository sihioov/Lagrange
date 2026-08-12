mod common;

use chrono::{FixedOffset, TimeZone};
use common::ScratchDb;
use job_queue::recommendation::input::DatasetPin;
use job_queue::recommendation::schedule::{ScheduleError, run_schedule_cycle};
use sqlx::PgPool;
use uuid::Uuid;

async fn seed_candidate(db: &ScratchDb, opted_in: bool) -> (Uuid, Uuid, DatasetPin) {
    let owner: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) VALUES ('scheduler.test', $1, $2) RETURNING id",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(format!("{}@scheduler.test", Uuid::new_v4()))
    .fetch_one(&db.pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO strategies (id, display_name, state) VALUES ('scheduled_strategy', 'Scheduled', 'Paper') ON CONFLICT DO NOTHING")
        .execute(&db.pool).await.unwrap();
    let config: Uuid = sqlx::query_scalar(
        "INSERT INTO user_strategy_configs (owner_user_id, strategy_id, strategy_version, config_json) \
         VALUES ($1, 'scheduled_strategy', '1.0.0', '{}'::jsonb) RETURNING id",
    )
    .bind(owner)
    .fetch_one(&db.pool).await.unwrap();
    let account: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (owner_user_id, account_type, name, currency, status) \
         VALUES ($1, 'PAPER', $2, 'KRW', 'ACTIVE') RETURNING id",
    )
    .bind(owner)
    .bind(format!("paper-{config}"))
    .fetch_one(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO account_strategy_bindings \
         (account_id, owner_user_id, strategy_config_id, strategy_id, strategy_version, auto_apply_recommendations) \
         VALUES ($1, $2, $3, 'scheduled_strategy', '1.0.0', $4)",
    )
    .bind(account).bind(owner).bind(config).bind(opted_in)
    .execute(&db.pool).await.unwrap();
    sqlx::query(
        "INSERT INTO data_entitlements \
         (contract_document_sha256, contract_reference, status, covered_datasets, covered_uses, effective_from, effective_until, managed_by) \
         VALUES (repeat('e',64), $2, 'ACTIVE', '[\"krx_eod_bars\"]', '[\"recommendation\"]', '2026-01-01', '2026-12-31', $1)",
    )
    .bind(owner).bind(format!("scheduler-{owner}"))
    .execute(&db.pool).await.unwrap();
    let dataset_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO dataset_versions (id, dataset_id, version, status, manifest_sha256, storage_path) \
         VALUES ($1, 'krx_eod_bars', $2, 'READY', repeat('c',64), $3)",
    )
    .bind(dataset_id).bind(format!("v-{dataset_id}")).bind(format!("curated/{dataset_id}"))
    .execute(&db.pool).await.unwrap();
    (
        owner,
        config,
        DatasetPin {
            id: dataset_id,
            dataset_id: "krx_eod_bars".into(),
            version: format!("v-{dataset_id}"),
            curated_version: 2,
            manifest_sha256: "c".repeat(64),
        },
    )
}

async fn publish_close(db: &ScratchDb, date: &str) {
    let batch = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO data_batches \
         (provider, market, batch_date, kind, storage_path, content_sha256, bytes_size, retrieved_at, source_batch_id, source_file_name, fetch_mode) \
         VALUES ('KRX','KR',$1::date,'EOD',$2,repeat('a',64),1,now(),$3,'bars.json','credentialed')",
    )
    .bind(date).bind(format!("raw/{date}")).bind(batch)
    .execute(&db.pool).await.unwrap();
    sqlx::query(
        "INSERT INTO trading_calendars \
         (exchange, session_date, session_type, timezone, source, source_version, source_batch_id, content_sha256, retrieved_at) \
         VALUES ('KRX',$1::date,'TRADING','Asia/Seoul','KRX',$2,$3,repeat('b',64),now())",
    )
    .bind(date).bind(format!("calendar-{date}")).bind(batch)
    .execute(&db.pool).await.unwrap();
}

async fn publish_trading_session(db: &ScratchDb, date: &str) {
    sqlx::query(
        "INSERT INTO trading_calendars \
         (exchange, session_date, session_type, timezone, source, source_version, \
          source_batch_id, content_sha256, retrieved_at) \
         VALUES ('KRX',$1::date,'TRADING','Asia/Seoul','KRX',$2,$3,repeat('b',64),now())",
    )
    .bind(date)
    .bind(format!("calendar-{date}"))
    .bind(Uuid::new_v4())
    .execute(&db.pool)
    .await
    .unwrap();
}

async fn publish_eod_batch(db: &ScratchDb, date: &str) {
    sqlx::query(
        "INSERT INTO data_batches \
         (provider, market, batch_date, kind, storage_path, content_sha256, bytes_size, \
          retrieved_at, source_batch_id, source_file_name, fetch_mode) \
         VALUES ('KRX','KR',$1::date,'EOD',$2,repeat('a',64),1,now(),$3,'bars.json','credentialed')",
    )
    .bind(date)
    .bind(format!("raw/{date}"))
    .bind(Uuid::new_v4())
    .execute(&db.pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn concurrent_close_and_startup_cycles_create_one_scheduled_identity() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let (owner, config, pin) = seed_candidate(&db, true).await;
    publish_close(&db, "2026-05-08").await;
    let worker = PgPool::connect(&db.role_url("worker")).await.unwrap();
    let seoul = FixedOffset::east_opt(9 * 3600).unwrap();
    let now = seoul.with_ymd_and_hms(2026, 5, 11, 9, 0, 0).unwrap();
    let first = tokio::spawn(run_schedule_cycle(worker.clone(), pin.clone(), now));
    let second = tokio::spawn(run_schedule_cycle(worker.clone(), pin, now));
    let (first, second) = tokio::join!(first, second);
    assert_eq!(first.unwrap().unwrap().as_of.to_string(), "2026-05-08");
    assert_eq!(second.unwrap().unwrap().as_of.to_string(), "2026-05-08");
    let identities: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM recommendation_runs WHERE owner_user_id=$1 AND strategy_config_id=$2 \
         AND as_of='2026-05-08' AND trigger_kind='SCHEDULED'",
    )
    .bind(owner)
    .bind(config)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(identities, 1);
    let jobs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM jobs WHERE owner_user_id=$1 AND job_type='recommendation'",
    )
    .bind(owner)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(jobs, 1);
    worker.close().await;
    db.drop_db().await;
}

#[tokio::test]
async fn only_explicit_opt_in_is_eligible_for_a_confirmed_published_close() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let (enabled_owner, enabled_config, pin) = seed_candidate(&db, true).await;
    let (disabled_owner, _, _) = seed_candidate(&db, false).await;
    publish_close(&db, "2026-05-11").await;
    let worker = PgPool::connect(&db.role_url("worker")).await.unwrap();
    let seoul = FixedOffset::east_opt(9 * 3600).unwrap();
    let now = seoul.with_ymd_and_hms(2026, 5, 11, 16, 30, 0).unwrap();
    let report = run_schedule_cycle(worker.clone(), pin, now).await.unwrap();
    assert_eq!(report.scheduled, 1);
    let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT owner_user_id, strategy_config_id FROM recommendation_runs WHERE trigger_kind='SCHEDULED'",
    ).fetch_all(&db.pool).await.unwrap();
    assert_eq!(rows, vec![(enabled_owner, enabled_config)]);
    assert!(!rows.iter().any(|(owner, _)| *owner == disabled_owner));
    worker.close().await;
    db.drop_db().await;
}

#[tokio::test]
async fn current_trading_session_waits_for_its_own_confirmed_close() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let (_, _, pin) = seed_candidate(&db, true).await;
    publish_close(&db, "2026-05-08").await;
    let worker = PgPool::connect(&db.role_url("worker")).await.unwrap();
    let seoul = FixedOffset::east_opt(9 * 3600).unwrap();
    let now = seoul.with_ymd_and_hms(2026, 5, 11, 16, 30, 0).unwrap();

    let missing_calendar = run_schedule_cycle(worker.clone(), pin.clone(), now)
        .await
        .unwrap_err();
    assert!(
        matches!(missing_calendar, ScheduleError::NoConfirmedClose),
        "the scheduler must retry today's key instead of completing Friday: {missing_calendar}"
    );

    publish_trading_session(&db, "2026-05-11").await;
    let blocked = run_schedule_cycle(worker.clone(), pin.clone(), now)
        .await
        .unwrap_err();
    assert!(
        matches!(blocked, ScheduleError::DatasetUnavailable),
        "the scheduler must retry today's close instead of accepting Friday: {blocked}"
    );
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM recommendation_runs")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(before, 0);

    publish_eod_batch(&db, "2026-05-11").await;
    let report = run_schedule_cycle(worker.clone(), pin, now).await.unwrap();
    assert_eq!(report.as_of.to_string(), "2026-05-11");
    assert_eq!(report.scheduled, 1);
    worker.close().await;
    db.drop_db().await;
}
