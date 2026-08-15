mod common;

use std::collections::BTreeMap;
use std::time::Duration;

use chrono::NaiveDate;
use common::ScratchDb;
use domain::{
    ContentHash, Currency, DatasetId, InstrumentId, Money, Price, Quantity, TradingDate,
    UtcTimestamp, Weight,
};
use job_queue::paper_execution::{
    PAPER_LOCK_TIMEOUT, PAPER_STATEMENT_TIMEOUT, set_paper_transaction_timeouts,
};
use job_queue::paper_preview::{
    PaperPreviewError, PreviewCalculationInput, PreviewLineage, PreviewRunOutcome,
    calculate_preview, load_recommendation_closes, run_preview_once,
};
use job_queue::{JobQueue, QueueConfig};
use market_data::curate::schema::{CuratedBar, write_bars};
use market_data::{Capability, CurateStore, DatasetManifest, dataset_manifest_hash};
use portfolio_model::CostProfile;
use portfolio_model::sizing::TargetAllocation;
use uuid::Uuid;

fn instrument(value: &str) -> InstrumentId {
    InstrumentId::parse(value).unwrap()
}

fn target(value: &str, weight: &str) -> TargetAllocation {
    TargetAllocation {
        instrument_id: instrument(value),
        weight: Weight::parse(weight).unwrap(),
    }
}

fn lineage() -> PreviewLineage {
    PreviewLineage {
        account_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
        recommendation_run_id: Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap(),
        target_portfolio_id: Uuid::parse_str("00000000-0000-0000-0000-000000000003").unwrap(),
        strategy_config_id: Uuid::parse_str("00000000-0000-0000-0000-000000000004").unwrap(),
        dataset_version_id: Uuid::parse_str("00000000-0000-0000-0000-000000000005").unwrap(),
        curated_version: 7,
        dataset_manifest_sha256: "a".repeat(64),
        account_state_version: 7,
        account_state_sha256: "b".repeat(64),
        target_portfolio_sha256: "c".repeat(64),
    }
}

fn calculation_input() -> PreviewCalculationInput {
    PreviewCalculationInput {
        cash: Money::parse("1000000", Currency::KRW).unwrap(),
        positions: BTreeMap::from([(instrument("069500.KRX"), Quantity::parse("100").unwrap())]),
        close_prices: BTreeMap::from([
            (instrument("069500.KRX"), Price::parse("10000").unwrap()),
            (instrument("229200.KRX"), Price::parse("10000").unwrap()),
        ]),
        targets: vec![
            target("069500.KRX", "0.250000"),
            target("229200.KRX", "0.750000"),
        ],
        lot_sizes: BTreeMap::new(),
        profile: CostProfile::krx_etf_default().unwrap(),
        price_date: TradingDate::parse("2026-05-08").unwrap(),
        proposed_effective_date: TradingDate::parse("2026-05-12").unwrap(),
        lineage: lineage(),
    }
}

#[tokio::test]
async fn paper_transaction_limits_are_local_and_effective() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let mut tx = db.pool.begin().await.unwrap();
    set_paper_transaction_timeouts(&mut tx).await.unwrap();
    let statement: String = sqlx::query_scalar("SHOW statement_timeout")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    let lock: String = sqlx::query_scalar("SHOW lock_timeout")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(statement, PAPER_STATEMENT_TIMEOUT);
    assert_eq!(lock, PAPER_LOCK_TIMEOUT);
    tx.rollback().await.unwrap();
    db.drop_db().await;
}

#[test]
fn calculation_is_sell_first_explainable_and_deterministic() {
    let input = calculation_input();
    let (result, token) = calculate_preview(input.clone()).unwrap();

    assert_eq!(result.schema_version, 1);
    assert_eq!(result.price_basis, "RECOMMENDATION_CLOSE");
    assert_eq!(result.price_date, "2026-05-08");
    assert_eq!(result.proposed_effective_date, "2026-05-12");
    assert_eq!(result.equity, "2000000.0000");
    assert_eq!(result.cash_before, "1000000.0000");
    assert_eq!(result.warning_code, "INDICATIVE_NEXT_OPEN_REPLAN_REQUIRED");
    assert_eq!(result.orders.len(), 2);
    assert_eq!(result.orders[0].instrument_id, "069500.KRX");
    assert_eq!(result.orders[0].side, "SELL");
    assert_eq!(result.orders[1].instrument_id, "229200.KRX");
    assert_eq!(result.orders[1].side, "BUY");
    assert!(result.explicit_fees.parse::<f64>().unwrap() > 0.0);
    assert!(result.informational_slippage.parse::<f64>().unwrap() > 0.0);
    assert!(result.leftover_cash.parse::<f64>().unwrap() >= 0.0);
    assert_eq!(token.len(), 64);
    assert!(
        token
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    );

    let (same_result, same_token) = calculate_preview(input).unwrap();
    assert_eq!(same_result, result);
    assert_eq!(same_token, token);
}

#[test]
fn calculation_is_canonical_when_target_input_order_changes() {
    let input = calculation_input();
    let (expected, expected_token) = calculate_preview(input.clone()).unwrap();
    let mut reordered = input;
    reordered.targets.reverse();

    let (actual, actual_token) = calculate_preview(reordered).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual_token, expected_token);
}

#[test]
fn calculation_all_cash_sells_every_position_and_never_buys() {
    let mut input = calculation_input();
    input.targets.clear();

    let (result, _) = calculate_preview(input).unwrap();
    assert_eq!(result.orders.len(), 1);
    assert_eq!(result.orders[0].side, "SELL");
    assert_eq!(result.orders[0].quantity, "100");
    assert_eq!(result.buy_notional, "0.0000");
    assert!(result.sell_notional.parse::<f64>().unwrap() > 0.0);
}

#[test]
fn calculation_fails_closed_when_a_held_instrument_has_no_close() {
    let mut input = calculation_input();
    input.close_prices.remove(&instrument("069500.KRX"));

    let error = calculate_preview(input).unwrap_err();
    assert!(matches!(
        error,
        PaperPreviewError::MissingPrice { instrument_id }
            if instrument_id == "069500.KRX"
    ));
}

#[test]
fn calculation_explains_no_trade_and_minimum_trade_skips() {
    let mut no_trade = calculation_input();
    no_trade.cash = Money::zero(Currency::KRW);
    no_trade.targets = vec![target("069500.KRX", "1.000000")];
    let (no_trade_result, _) = calculate_preview(no_trade).unwrap();
    assert!(no_trade_result.orders.is_empty());
    assert_eq!(no_trade_result.decisions[0].action, "SKIP");
    assert_eq!(
        no_trade_result.decisions[0].skip_reason.as_deref(),
        Some("BELOW_REBALANCE_THRESHOLD")
    );

    let mut below_minimum = calculation_input();
    below_minimum.cash = Money::parse("50000", Currency::KRW).unwrap();
    below_minimum.positions.clear();
    below_minimum.targets = vec![target("229200.KRX", "1.000000")];
    let (minimum_result, _) = calculate_preview(below_minimum).unwrap();
    assert!(minimum_result.orders.is_empty());
    assert_eq!(minimum_result.decisions[0].action, "SKIP");
    assert_eq!(
        minimum_result.decisions[0].skip_reason.as_deref(),
        Some("BELOW_MIN_TRADE")
    );
}

#[test]
fn calculation_rejects_duplicate_target_identity() {
    let mut input = calculation_input();
    input.targets.push(target("069500.KRX", "0.100000"));
    assert!(matches!(
        calculate_preview(input),
        Err(PaperPreviewError::InvalidPayload(detail)) if detail.contains("duplicate target")
    ));
}

fn curated_bar(instrument_id: &str, date: &str, close: &str, close_at: &str) -> CuratedBar {
    let price = Price::parse(close).unwrap();
    CuratedBar {
        instrument_id: instrument(instrument_id),
        trading_date: TradingDate::parse(date).unwrap(),
        market_open_ts: UtcTimestamp::parse_rfc3339("2026-05-08T00:00:00Z").unwrap(),
        market_close_ts: UtcTimestamp::parse_rfc3339(close_at).unwrap(),
        open: price,
        high: price,
        low: price,
        close: price,
        volume: 1,
        trading_value: Some(1),
        currency: Currency::KRW,
        source: "qa".into(),
        ingested_at: UtcTimestamp::parse_rfc3339("2026-05-08T07:00:00Z").unwrap(),
        batch_id: "00000000-0000-0000-0000-000000000001".parse().unwrap(),
        raw_hash: ContentHash::from_bytes(b"paper-preview"),
    }
}

fn write_preview_bars(
    root: &std::path::Path,
    partition_instrument: &str,
    version: u32,
    rows: &[CuratedBar],
) {
    let store = CurateStore::new(root.join("curated"));
    let path = store.bars_path("kr", partition_instrument, 2026, version);
    write_bars(&path, rows).unwrap();
}

#[test]
fn close_loader_reads_exact_raw_close_from_the_attested_version() {
    let directory = tempfile::tempdir().unwrap();
    write_preview_bars(
        directory.path(),
        "069500.KRX",
        7,
        &[curated_bar(
            "069500.KRX",
            "2026-05-08",
            "12345.6700",
            "2026-05-08T06:30:00Z",
        )],
    );

    let closes = load_recommendation_closes(
        directory.path(),
        7,
        TradingDate::parse("2026-05-08").unwrap(),
        &[instrument("069500.KRX")],
    )
    .unwrap();
    assert_eq!(
        closes[&instrument("069500.KRX")].as_decimal_string(),
        "12345.6700"
    );
}

#[test]
fn close_loader_fails_closed_for_missing_version_or_date() {
    let directory = tempfile::tempdir().unwrap();
    write_preview_bars(
        directory.path(),
        "069500.KRX",
        6,
        &[curated_bar(
            "069500.KRX",
            "2026-05-07",
            "10000",
            "2026-05-07T06:30:00Z",
        )],
    );

    for (version, date) in [(7, "2026-05-07"), (6, "2026-05-08")] {
        let error = load_recommendation_closes(
            directory.path(),
            version,
            TradingDate::parse(date).unwrap(),
            &[instrument("069500.KRX")],
        )
        .unwrap_err();
        assert!(matches!(
            error,
            PaperPreviewError::MissingPrice { instrument_id }
                if instrument_id == "069500.KRX"
        ));
    }
}

#[test]
fn close_loader_rejects_partition_identity_mismatch_and_malformed_parquet() {
    let wrong_identity = tempfile::tempdir().unwrap();
    write_preview_bars(
        wrong_identity.path(),
        "069500.KRX",
        7,
        &[curated_bar(
            "229200.KRX",
            "2026-05-08",
            "10000",
            "2026-05-08T06:30:00Z",
        )],
    );
    let mismatch = load_recommendation_closes(
        wrong_identity.path(),
        7,
        TradingDate::parse("2026-05-08").unwrap(),
        &[instrument("069500.KRX")],
    )
    .unwrap_err();
    assert!(matches!(
        mismatch,
        PaperPreviewError::MalformedCuratedData(_)
    ));

    let malformed = tempfile::tempdir().unwrap();
    let store = CurateStore::new(malformed.path().join("curated"));
    let path = store.bars_path("kr", "069500.KRX", 2026, 7);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, b"not parquet").unwrap();
    let error = load_recommendation_closes(
        malformed.path(),
        7,
        TradingDate::parse("2026-05-08").unwrap(),
        &[instrument("069500.KRX")],
    )
    .unwrap_err();
    assert!(matches!(error, PaperPreviewError::MalformedCuratedData(_)));
}

#[test]
fn close_loader_rejects_a_close_that_is_not_yet_available() {
    let directory = tempfile::tempdir().unwrap();
    write_preview_bars(
        directory.path(),
        "069500.KRX",
        7,
        &[curated_bar(
            "069500.KRX",
            "2026-05-08",
            "10000",
            "2099-05-08T06:30:00Z",
        )],
    );

    let error = load_recommendation_closes(
        directory.path(),
        7,
        TradingDate::parse("2026-05-08").unwrap(),
        &[instrument("069500.KRX")],
    )
    .unwrap_err();
    assert!(matches!(error, PaperPreviewError::PreviewUnavailable(_)));
}

struct WorkerFixture {
    _directory: tempfile::TempDir,
    dataset_root: std::path::PathBuf,
    preview_id: Uuid,
    job_id: Uuid,
}

async fn seed_worker_fixture(db: &ScratchDb) -> WorkerFixture {
    let directory = tempfile::tempdir().unwrap();
    write_preview_bars(
        directory.path(),
        "069500.KRX",
        7,
        &[curated_bar(
            "069500.KRX",
            "2026-05-08",
            "10000",
            "2026-05-08T06:30:00Z",
        )],
    );
    let store = CurateStore::new(directory.path().join("curated"));
    let manifest = DatasetManifest {
        dataset_id: DatasetId::parse("krx_eod_bars").unwrap(),
        version: 7,
        capability: Capability::PriceReturnOnly,
        created_at: UtcTimestamp::parse_rfc3339("2026-05-08T07:00:00Z").unwrap(),
        source_batches: Vec::new(),
        bar_count: 1,
        action_count: 0,
        content_hash: ContentHash::from_bytes(b"placeholder"),
    };
    let manifest = DatasetManifest {
        content_hash: dataset_manifest_hash(&manifest).unwrap(),
        ..manifest
    };
    store.write_dataset_manifest(&manifest).unwrap();
    let manifest_sha256 = manifest
        .content_hash
        .as_str()
        .strip_prefix("sha256:")
        .unwrap()
        .to_owned();
    let owner_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) \
         VALUES ('https://issuer.test', $1, $2) RETURNING id",
    )
    .bind(format!("preview-worker-{}", Uuid::new_v4()))
    .bind(format!("preview-worker-{}@example.test", Uuid::new_v4()))
    .fetch_one(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO strategies (id, display_name, state) \
         VALUES ('paper_preview_worker', 'Paper Preview Worker', 'Paper')",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO instruments (id, symbol, venue, currency) \
         VALUES ('069500.KRX', '069500', 'KRX', 'KRW')",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let config_id: Uuid = sqlx::query_scalar(
        "INSERT INTO user_strategy_configs \
         (owner_user_id, strategy_id, strategy_version, config_json) \
         VALUES ($1, 'paper_preview_worker', '1.0.0', '{}'::jsonb) RETURNING id",
    )
    .bind(owner_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let account_id: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts \
         (owner_user_id, account_type, name, status, initial_cash) \
         VALUES ($1, 'PAPER', 'preview-worker', 'ACTIVE', 1000000) RETURNING id",
    )
    .bind(owner_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO account_strategy_bindings \
         (account_id, owner_user_id, strategy_config_id, strategy_id, strategy_version) \
         VALUES ($1, $2, $3, 'paper_preview_worker', '1.0.0')",
    )
    .bind(account_id)
    .bind(owner_id)
    .bind(config_id)
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO cash_ledger \
         (account_id, owner_user_id, seq, event_type, amount, balance) \
         VALUES ($1, $2, 1, 'DEPOSIT', 1000000, 1000000)",
    )
    .bind(account_id)
    .bind(owner_id)
    .execute(&db.pool)
    .await
    .unwrap();
    let dataset_version_id: Uuid = sqlx::query_scalar(
        "INSERT INTO dataset_versions \
         (dataset_id, version, status, manifest_sha256, storage_path) \
         VALUES ('krx_eod_bars', 'preview-v1', 'READY', $1, \
                 'curated/preview-v1') RETURNING id",
    )
    .bind(&manifest_sha256)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO data_entitlements \
         (contract_document_sha256, contract_reference, status, covered_datasets, \
          covered_uses, effective_from, effective_until, managed_by) \
         VALUES (repeat('e',64), 'vault://qa/paper-preview', 'ACTIVE', \
                 '[\"krx_eod_bars\"]', '[\"recommendation\"]', \
                 DATE '2020-01-01', DATE '2030-12-31', $1)",
    )
    .bind(owner_id)
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO trading_calendars \
         (exchange, session_date, session_type, timezone, source, source_version, \
          source_batch_id, content_sha256, retrieved_at) \
         VALUES ('KRX', DATE '2026-05-12', 'TRADING', 'Asia/Seoul', 'KRX', \
                 'preview-v1', $1, repeat('c',64), now())",
    )
    .bind(Uuid::new_v4())
    .execute(&db.pool)
    .await
    .unwrap();
    let recommendation_job_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO jobs \
         (id, owner_user_id, job_type, status, idempotency_key, payload_json) \
         VALUES ($1, $2, 'recommendation', 'SUCCEEDED', $3, \
                 jsonb_build_object('dataset', jsonb_build_object( \
                   'id', $4::uuid, 'dataset_id', 'krx_eod_bars', \
                   'version', 'preview-v1', 'curated_version', 7, \
                   'manifest_sha256', $5::text)))",
    )
    .bind(recommendation_job_id)
    .bind(owner_id)
    .bind(format!("preview-source-{}", Uuid::new_v4()))
    .bind(dataset_version_id)
    .bind(&manifest_sha256)
    .execute(&db.pool)
    .await
    .unwrap();
    let recommendation_run_id: Uuid = sqlx::query_scalar(
        "INSERT INTO recommendation_runs \
         (owner_user_id, strategy_config_id, as_of, status, job_id, trigger_kind, \
          dataset_version_id, dataset_manifest_sha256) \
         VALUES ($1, $2, DATE '2026-05-08', 'SUCCEEDED', $3, 'MANUAL', \
                 $4, $5) RETURNING id",
    )
    .bind(owner_id)
    .bind(config_id)
    .bind(recommendation_job_id)
    .bind(dataset_version_id)
    .bind(&manifest_sha256)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let portfolio_id: Uuid = sqlx::query_scalar(
        "INSERT INTO target_portfolios \
         (owner_user_id, recommendation_run_id, as_of, weights_json) \
         VALUES ($1, $2, DATE '2026-05-08', \
                 '{\"069500.KRX\":\"1.000000\"}'::jsonb) RETURNING id",
    )
    .bind(owner_id)
    .bind(recommendation_run_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let target_portfolio_sha256 = ContentHash::from_bytes(br#"{"069500.KRX":"1.000000"}"#)
        .as_str()
        .strip_prefix("sha256:")
        .unwrap()
        .to_owned();
    let preview_id = Uuid::new_v4();
    let job_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO jobs \
         (id, owner_user_id, job_type, status, idempotency_key, payload_json, max_attempts) \
         VALUES ($1, $2, 'paper_rebalance_preview', 'QUEUED', $3, \
                 jsonb_build_object('preview_id', $4::uuid), 2)",
    )
    .bind(job_id)
    .bind(owner_id)
    .bind(format!("paper-preview-{}", preview_id))
    .bind(preview_id)
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO paper_rebalance_previews \
         (id, owner_user_id, account_id, recommendation_run_id, target_portfolio_id, \
          strategy_config_id, job_id, price_date, dataset_version_id, \
          dataset_manifest_sha256, target_portfolio_sha256) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, DATE '2026-05-08', $8, \
                 $9, $10)",
    )
    .bind(preview_id)
    .bind(owner_id)
    .bind(account_id)
    .bind(recommendation_run_id)
    .bind(portfolio_id)
    .bind(config_id)
    .bind(job_id)
    .bind(dataset_version_id)
    .bind(&manifest_sha256)
    .bind(target_portfolio_sha256)
    .execute(&db.pool)
    .await
    .unwrap();

    WorkerFixture {
        dataset_root: directory.path().to_path_buf(),
        _directory: directory,
        preview_id,
        job_id,
    }
}

#[tokio::test]
async fn target_portfolio_change_after_submission_never_publishes_mislabelled_result() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let fixture = seed_worker_fixture(&db).await;
    sqlx::query(
        "UPDATE target_portfolios \
            SET weights_json='{\"069500.KRX\":\"0.500000\"}'::jsonb \
          WHERE id=(SELECT target_portfolio_id FROM paper_rebalance_previews WHERE id=$1)",
    )
    .bind(fixture.preview_id)
    .execute(&db.pool)
    .await
    .unwrap();
    let worker = sqlx::PgPool::connect(&db.role_url("worker")).await.unwrap();
    let queue = preview_queue(worker.clone());
    let outcome = run_preview_once(
        &worker,
        &queue,
        &fixture.dataset_root,
        "target-change-worker",
        NaiveDate::from_ymd_opt(2026, 5, 9).unwrap(),
        Duration::from_millis(100),
    )
    .await
    .unwrap();
    assert!(matches!(
        outcome,
        PreviewRunOutcome::Failed { ref code, .. } if code == "PAPER_PREVIEW_TARGET_CHANGED"
    ));
    let state: (String, String, bool) = sqlx::query_as(
        "SELECT job.status, preview.status, preview.result_json IS NULL \
         FROM jobs AS job JOIN paper_rebalance_previews AS preview ON preview.job_id=job.id \
         WHERE job.id=$1",
    )
    .bind(fixture.job_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(state, ("FAILED".into(), "FAILED".into(), true));
    worker.close().await;
    db.drop_db().await;
}

#[tokio::test]
async fn target_portfolio_change_after_snapshot_never_publishes_stale_calculation() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let fixture = seed_worker_fixture(&db).await;
    sqlx::raw_sql(
        "CREATE FUNCTION test_preview_mutate_target() RETURNS trigger \
         LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp AS $$ \
         BEGIN \
           IF OLD.status='PENDING' AND NEW.status='RUNNING' THEN \
             UPDATE public.target_portfolios \
                SET weights_json='{\"069500.KRX\":\"0.500000\"}'::jsonb \
              WHERE id=NEW.target_portfolio_id; \
           END IF; \
           RETURN NEW; \
         END $$; \
         ALTER FUNCTION test_preview_mutate_target() OWNER TO migration_owner; \
         CREATE TRIGGER test_preview_mutate_target \
           AFTER UPDATE ON paper_rebalance_previews \
           FOR EACH ROW EXECUTE FUNCTION test_preview_mutate_target();",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let worker = sqlx::PgPool::connect(&db.role_url("worker")).await.unwrap();
    let queue = preview_queue(worker.clone());
    let outcome = run_preview_once(
        &worker,
        &queue,
        &fixture.dataset_root,
        "target-race-worker",
        NaiveDate::from_ymd_opt(2026, 5, 9).unwrap(),
        Duration::from_millis(100),
    )
    .await
    .unwrap();
    assert!(matches!(
        outcome,
        PreviewRunOutcome::Failed { ref code, .. }
            if code == "PAPER_PREVIEW_PUBLICATION_INTEGRITY"
    ));
    let state: (String, String, bool) = sqlx::query_as(
        "SELECT job.status, preview.status, preview.result_json IS NULL \
         FROM jobs AS job JOIN paper_rebalance_previews AS preview ON preview.job_id=job.id \
         WHERE job.id=$1",
    )
    .bind(fixture.job_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(state, ("FAILED".into(), "FAILED".into(), true));
    worker.close().await;
    db.drop_db().await;
}

#[tokio::test]
async fn corrupted_attested_manifest_never_publishes_preview() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let fixture = seed_worker_fixture(&db).await;
    let store = CurateStore::new(fixture.dataset_root.join("curated"));
    let manifest_path = store
        .dataset_dir(&DatasetId::parse("krx_eod_bars").unwrap(), 7)
        .join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    manifest["bar_count"] = serde_json::json!(2);
    std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();

    let worker = sqlx::PgPool::connect(&db.role_url("worker")).await.unwrap();
    let queue = preview_queue(worker.clone());
    let outcome = run_preview_once(
        &worker,
        &queue,
        &fixture.dataset_root,
        "manifest-corruption-worker",
        NaiveDate::from_ymd_opt(2026, 5, 9).unwrap(),
        Duration::from_millis(100),
    )
    .await
    .unwrap();
    assert!(matches!(
        outcome,
        PreviewRunOutcome::Failed { ref code, .. } if code == "PAPER_PREVIEW_CURATED_INTEGRITY"
    ));
    let state: (String, String, bool) = sqlx::query_as(
        "SELECT job.status, preview.status, preview.result_json IS NULL \
         FROM jobs AS job JOIN paper_rebalance_previews AS preview ON preview.job_id=job.id \
         WHERE job.id=$1",
    )
    .bind(fixture.job_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(state, ("FAILED".into(), "FAILED".into(), true));
    worker.close().await;
    db.drop_db().await;
}

#[tokio::test]
async fn worker_publishes_preview_and_queue_state_atomically_without_trading() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let fixture = seed_worker_fixture(&db).await;
    let worker = sqlx::PgPool::connect(&db.role_url("worker")).await.unwrap();
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
        &fixture.dataset_root,
        "paper-preview-test",
        NaiveDate::from_ymd_opt(2026, 5, 9).unwrap(),
        Duration::from_millis(100),
    )
    .await
    .unwrap();
    assert!(matches!(
        outcome,
        PreviewRunOutcome::Published { job_id, preview_id }
            if job_id == fixture.job_id && preview_id == fixture.preview_id
    ));
    let state: (String, String, bool, bool) = sqlx::query_as(
        "SELECT job.status, preview.status, preview.result_json IS NOT NULL, \
                preview.preview_token ~ '^[0-9a-f]{64}$' \
         FROM jobs AS job JOIN paper_rebalance_previews AS preview ON preview.job_id=job.id \
         WHERE job.id=$1",
    )
    .bind(fixture.job_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(state, ("SUCCEEDED".into(), "READY".into(), true, true));
    let attempt: String = sqlx::query_scalar("SELECT outcome FROM job_attempts WHERE job_id=$1")
        .bind(fixture.job_id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(attempt, "SUCCEEDED");
    let side_effects: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM pending_targets), \
                (SELECT count(*) FROM orders), (SELECT count(*) FROM fills)",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(side_effects, (0, 0, 0));

    worker.close().await;
    db.drop_db().await;
}

fn preview_queue(pool: sqlx::PgPool) -> JobQueue {
    JobQueue::new(
        pool,
        None,
        QueueConfig {
            lease: Duration::from_secs(10),
            backoff_base: Duration::from_millis(1),
        },
    )
}

async fn delay_preview_snapshot(db: &ScratchDb) {
    sqlx::raw_sql(
        "CREATE FUNCTION test_delay_preview_snapshot() RETURNS trigger \
         LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_sleep(1); RETURN NEW; END $$; \
         CREATE TRIGGER test_delay_preview_snapshot \
           AFTER UPDATE OF status ON paper_rebalance_previews \
           FOR EACH ROW WHEN (OLD.status='PENDING' AND NEW.status='RUNNING') \
           EXECUTE FUNCTION test_delay_preview_snapshot();",
    )
    .execute(&db.pool)
    .await
    .unwrap();
}

async fn wait_for_running_job(db: &ScratchDb, job_id: Uuid) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let running: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM jobs WHERE id=$1 AND status='RUNNING')",
            )
            .bind(job_id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
            if running {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("preview job reached RUNNING");
}

#[tokio::test]
async fn two_workers_publish_one_preview_once() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let fixture = seed_worker_fixture(&db).await;
    let worker_one = sqlx::PgPool::connect(&db.role_url("worker")).await.unwrap();
    let worker_two = sqlx::PgPool::connect(&db.role_url("worker")).await.unwrap();
    let queue_one = preview_queue(worker_one.clone());
    let queue_two = preview_queue(worker_two.clone());
    let date = NaiveDate::from_ymd_opt(2026, 5, 9).unwrap();
    let first = run_preview_once(
        &worker_one,
        &queue_one,
        &fixture.dataset_root,
        "preview-worker-one",
        date,
        Duration::from_millis(100),
    );
    let second = run_preview_once(
        &worker_two,
        &queue_two,
        &fixture.dataset_root,
        "preview-worker-two",
        date,
        Duration::from_millis(100),
    );
    let (first, second) = tokio::join!(first, second);
    let outcomes = [first.unwrap(), second.unwrap()];
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, PreviewRunOutcome::Published { .. }))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, PreviewRunOutcome::Idle))
            .count(),
        1
    );
    let counts: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM job_attempts WHERE job_id=$1), \
                (SELECT count(*) FROM paper_rebalance_previews \
                  WHERE id=$2 AND status='READY')",
    )
    .bind(fixture.job_id)
    .bind(fixture.preview_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(counts, (1, 1));
    worker_one.close().await;
    worker_two.close().await;
    db.drop_db().await;
}

#[tokio::test]
async fn cancellation_during_compute_never_publishes() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let fixture = seed_worker_fixture(&db).await;
    delay_preview_snapshot(&db).await;
    let worker = sqlx::PgPool::connect(&db.role_url("worker")).await.unwrap();
    let queue = JobQueue::new(
        worker.clone(),
        None,
        QueueConfig {
            lease: Duration::from_secs(2),
            backoff_base: Duration::from_millis(1),
        },
    );
    let running = tokio::spawn({
        let worker = worker.clone();
        let queue = queue.clone();
        let root = fixture.dataset_root.clone();
        async move {
            run_preview_once(
                &worker,
                &queue,
                &root,
                "cancel-preview-worker",
                NaiveDate::from_ymd_opt(2026, 5, 9).unwrap(),
                Duration::from_millis(50),
            )
            .await
        }
    });
    wait_for_running_job(&db, fixture.job_id).await;
    sqlx::query(
        "UPDATE jobs SET status='CANCELED', finished_at=now(), updated_at=now() WHERE id=$1",
    )
    .bind(fixture.job_id)
    .execute(&db.pool)
    .await
    .unwrap();
    let outcome = running.await.unwrap().unwrap();
    assert!(matches!(outcome, PreviewRunOutcome::Canceled { .. }));
    let state: (String, String, bool) = sqlx::query_as(
        "SELECT job.status, preview.status, preview.result_json IS NULL \
         FROM jobs AS job JOIN paper_rebalance_previews AS preview ON preview.job_id=job.id \
         WHERE job.id=$1",
    )
    .bind(fixture.job_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(state, ("CANCELED".into(), "FAILED".into(), true));
    worker.close().await;
    db.drop_db().await;
}

#[tokio::test]
async fn swept_lease_during_compute_never_publishes() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let fixture = seed_worker_fixture(&db).await;
    delay_preview_snapshot(&db).await;
    let worker = sqlx::PgPool::connect(&db.role_url("worker")).await.unwrap();
    let queue = JobQueue::new(
        worker.clone(),
        None,
        QueueConfig {
            lease: Duration::from_secs(2),
            backoff_base: Duration::from_millis(1),
        },
    );
    let running = tokio::spawn({
        let worker = worker.clone();
        let queue = queue.clone();
        let root = fixture.dataset_root.clone();
        async move {
            run_preview_once(
                &worker,
                &queue,
                &root,
                "lease-preview-worker",
                NaiveDate::from_ymd_opt(2026, 5, 9).unwrap(),
                Duration::from_millis(500),
            )
            .await
        }
    });
    wait_for_running_job(&db, fixture.job_id).await;
    sqlx::query("UPDATE jobs SET locked_at=now()-interval '1 hour' WHERE id=$1")
        .bind(fixture.job_id)
        .execute(&db.pool)
        .await
        .unwrap();
    let swept = queue.sweep().await.unwrap();
    assert_eq!(swept.jobs_requeued, 1);
    let outcome = running.await.unwrap().unwrap();
    assert!(matches!(outcome, PreviewRunOutcome::LeaseLost { .. }));
    let state: (String, String, bool) = sqlx::query_as(
        "SELECT job.status, preview.status, preview.result_json IS NULL \
         FROM jobs AS job JOIN paper_rebalance_previews AS preview ON preview.job_id=job.id \
         WHERE job.id=$1",
    )
    .bind(fixture.job_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(state, ("QUEUED".into(), "PENDING".into(), true));
    worker.close().await;
    db.drop_db().await;
}

#[tokio::test]
async fn missing_close_fails_preview_permanently_without_outputs() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let fixture = seed_worker_fixture(&db).await;
    let path = CurateStore::new(fixture.dataset_root.join("curated")).bars_path(
        "kr",
        "069500.KRX",
        2026,
        7,
    );
    std::fs::remove_file(path).unwrap();
    let worker = sqlx::PgPool::connect(&db.role_url("worker")).await.unwrap();
    let queue = preview_queue(worker.clone());
    let outcome = run_preview_once(
        &worker,
        &queue,
        &fixture.dataset_root,
        "missing-close-worker",
        NaiveDate::from_ymd_opt(2026, 5, 9).unwrap(),
        Duration::from_millis(100),
    )
    .await
    .unwrap();
    assert!(matches!(
        outcome,
        PreviewRunOutcome::Failed { ref code, .. } if code == "PAPER_PREVIEW_CLOSE_MISSING"
    ));
    let state: (String, String, Option<String>, i64) = sqlx::query_as(
        "SELECT job.status, preview.status, job.error_code, \
                (SELECT count(*) FROM orders) + (SELECT count(*) FROM fills) \
         FROM jobs AS job JOIN paper_rebalance_previews AS preview ON preview.job_id=job.id \
         WHERE job.id=$1",
    )
    .bind(fixture.job_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        state,
        (
            "FAILED".into(),
            "FAILED".into(),
            Some("PAPER_PREVIEW_CLOSE_MISSING".into()),
            0,
        )
    );
    worker.close().await;
    db.drop_db().await;
}

#[tokio::test]
async fn account_change_after_snapshot_requeues_without_publishing() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let fixture = seed_worker_fixture(&db).await;
    sqlx::raw_sql(
        "CREATE FUNCTION test_preview_mutate_cash() RETURNS trigger \
         LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp AS $$ \
         BEGIN \
           IF OLD.status='PENDING' AND NEW.status='RUNNING' THEN \
             INSERT INTO public.cash_ledger \
               (account_id,owner_user_id,seq,event_type,amount,balance) \
             SELECT NEW.account_id,NEW.owner_user_id, \
                    COALESCE(max(seq),0)+1,'TEST_MUTATION',0, \
                    (SELECT balance FROM public.cash_ledger \
                      WHERE account_id=NEW.account_id ORDER BY seq DESC LIMIT 1) \
               FROM public.cash_ledger WHERE account_id=NEW.account_id; \
           END IF; \
           RETURN NEW; \
         END $$; \
         ALTER FUNCTION test_preview_mutate_cash() OWNER TO migration_owner; \
         CREATE TRIGGER test_preview_mutate_cash \
           AFTER UPDATE ON paper_rebalance_previews \
           FOR EACH ROW EXECUTE FUNCTION test_preview_mutate_cash();",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let worker = sqlx::PgPool::connect(&db.role_url("worker")).await.unwrap();
    let queue = preview_queue(worker.clone());
    let outcome = run_preview_once(
        &worker,
        &queue,
        &fixture.dataset_root,
        "account-change-worker",
        NaiveDate::from_ymd_opt(2026, 5, 9).unwrap(),
        Duration::from_millis(100),
    )
    .await
    .unwrap();
    assert!(matches!(
        outcome,
        PreviewRunOutcome::Retrying { ref code, .. } if code == "PAPER_PREVIEW_ACCOUNT_CHANGED"
    ));
    let state: (String, String, bool) = sqlx::query_as(
        "SELECT job.status, preview.status, preview.result_json IS NULL \
         FROM jobs AS job JOIN paper_rebalance_previews AS preview ON preview.job_id=job.id \
         WHERE job.id=$1",
    )
    .bind(fixture.job_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(state, ("QUEUED".into(), "RUNNING".into(), true));
    worker.close().await;
    db.drop_db().await;
}

#[tokio::test]
async fn result_write_failure_rolls_back_preview_and_settles_job() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let fixture = seed_worker_fixture(&db).await;
    sqlx::raw_sql(
        "CREATE FUNCTION test_preview_reject_ready() RETURNS trigger \
         LANGUAGE plpgsql AS $$ BEGIN \
           IF NEW.status='READY' THEN \
             RAISE EXCEPTION 'reject preview publication' USING ERRCODE='23514'; \
           END IF; \
           RETURN NEW; \
         END $$; \
         CREATE TRIGGER test_preview_reject_ready \
           BEFORE UPDATE ON paper_rebalance_previews \
           FOR EACH ROW EXECUTE FUNCTION test_preview_reject_ready();",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let worker = sqlx::PgPool::connect(&db.role_url("worker")).await.unwrap();
    let queue = preview_queue(worker.clone());
    let outcome = run_preview_once(
        &worker,
        &queue,
        &fixture.dataset_root,
        "result-write-worker",
        NaiveDate::from_ymd_opt(2026, 5, 9).unwrap(),
        Duration::from_millis(100),
    )
    .await
    .unwrap();
    assert!(matches!(outcome, PreviewRunOutcome::Failed { .. }));
    let state: (String, String, bool) = sqlx::query_as(
        "SELECT job.status, preview.status, preview.result_json IS NULL \
         FROM jobs AS job JOIN paper_rebalance_previews AS preview ON preview.job_id=job.id \
         WHERE job.id=$1",
    )
    .bind(fixture.job_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(state, ("FAILED".into(), "FAILED".into(), true));
    worker.close().await;
    db.drop_db().await;
}
