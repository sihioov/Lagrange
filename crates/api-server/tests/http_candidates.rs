#[path = "../../../tests/support/candidate_rolling_provider.rs"]
mod candidate_rolling_provider;
mod common;

use axum::http::{StatusCode, header};
use chrono::{FixedOffset, TimeZone};
use collectors::{
    CandidateInstrumentCatalog, CandidatePricePublication, PostgresCandidateSourceSink,
    PostgresPublicationSink, PublicationSink, PublishOutcome, candidate_raw_manifest_sha256,
    prepare_candidate_batch, publish_candidate_batch,
};
use common::{Harness, status};
use domain::{DatasetId, TradingDate, UtcTimestamp};
use job_queue::candidate::{
    CandidateOutcome, CandidateRunnerConfig, CandidateRunnerPaths, run_once,
    schedule_latest_candidate_run,
};
use job_queue::{JobQueue, QueueConfig};
use market_data::{
    CANDIDATE_RESPONSE_KINDS, CurateRequest, CurateStore, IngestRequest, MARKET_KR,
    PublicationBundle, RawStore, curate_batch, curation_inputs_from_raw, ingest_bundle,
    ingest_bundle_with_kinds, price_curation_evidence,
};
use serde_json::json;
use std::path::Path;
use std::time::Duration;
use uuid::Uuid;

use candidate_rolling_provider::CredentialedRollingCandidateProvider;

const ROLLING_ENTITLEMENT_ID: Uuid = Uuid::from_u128(0x00000000000040008000000000000991);
const ROLLING_LICENSE_REF: &str = "fixture://candidate-http-rolling";

const INSTRUMENTS: [&str; 5] = [
    "200001.KRX",
    "200002.KRX",
    "200003.KRX",
    "200004.KRX",
    "200005.KRX",
];

async fn seed_published_candidate_feed(h: &Harness) -> Uuid {
    for instrument in INSTRUMENTS {
        sqlx::query(
            "INSERT INTO instruments
             (id, symbol, venue, currency, name, asset_class, status, listed_at)
             VALUES ($1,$2,'KRX','KRW',$3,'EQUITY','ACTIVE',DATE '2010-01-01')",
        )
        .bind(instrument)
        .bind(instrument.trim_end_matches(".KRX"))
        .bind(format!("Candidate {instrument}"))
        .execute(&h.owner_pool)
        .await
        .expect("seed candidate instrument");
    }
    let price: (Uuid, String, String) = sqlx::query_as(
        "SELECT id, version, manifest_sha256 FROM dataset_versions
         WHERE dataset_id='krx_eod_bars' AND status='READY' ORDER BY created_at LIMIT 1",
    )
    .fetch_one(&h.owner_pool)
    .await
    .expect("price dataset");
    let entitlement_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM data_entitlements WHERE status='ACTIVE' ORDER BY id LIMIT 1",
    )
    .fetch_one(&h.owner_pool)
    .await
    .expect("active candidate entitlement");
    sqlx::query(
        "UPDATE data_entitlements
            SET covered_datasets = $1,
                covered_uses = '[\"dataset\",\"factor\",\"recommendation\",\"candidate\",\"backtest\",\"report\",\"benchmark\",\"paper_view\",\"payload\",\"download\"]'::jsonb,
                updated_at = clock_timestamp()
          WHERE id = $2",
    )
    .bind(json!([
        "krx_eod_bars",
        "krx_market_status",
        "krx_investor_flows",
        "krx_fundamentals",
        "krx_kospi200_membership",
        "krx_sector_classification"
    ]))
    .bind(entitlement_id)
    .execute(&h.owner_pool)
    .await
    .expect("enable candidate entitlement before source inserts");
    let mut datasets = Vec::new();
    for (dataset_id, version, hash) in [
        ("krx_market_status", "candidate-http-status", "1".repeat(64)),
        ("krx_investor_flows", "candidate-http-flow", "2".repeat(64)),
        (
            "krx_fundamentals",
            "candidate-http-fundamental",
            "3".repeat(64),
        ),
        (
            "krx_kospi200_membership",
            "candidate-http-universe",
            "4".repeat(64),
        ),
        (
            "krx_sector_classification",
            "candidate-http-sector",
            "5".repeat(64),
        ),
    ] {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO dataset_versions
             (dataset_id, version, status, manifest_sha256, storage_path)
             VALUES ($1,$2,'READY',$3,$4) RETURNING id",
        )
        .bind(dataset_id)
        .bind(version)
        .bind(&hash)
        .bind(format!("db://candidate-http/{dataset_id}"))
        .fetch_one(&h.owner_pool)
        .await
        .expect("seed candidate dataset");
        datasets.push((dataset_id, id, hash));
    }
    let find = |dataset_id: &str| {
        datasets
            .iter()
            .find(|row| row.0 == dataset_id)
            .expect("fixture dataset")
    };
    let status_pin = find("krx_market_status").clone();
    let flow_pin = find("krx_investor_flows").clone();
    let fundamental_pin = find("krx_fundamentals").clone();
    let universe_pin = find("krx_kospi200_membership").clone();
    let sector_pin = find("krx_sector_classification").clone();

    let mut tx = h.owner_pool.begin().await.expect("candidate seed tx");
    let raw_batch_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO data_batches
         (provider, market, batch_date, kind, storage_path, content_sha256,
          bytes_size, retrieved_at, source_batch_id, source_file_name, fetch_mode)
         VALUES ('KRX','KR',DATE '2026-08-13','EOD','raw/candidate-http',repeat('9',64),
                 1,TIMESTAMPTZ '2026-08-13 07:00:00Z',$1,'candidate-bars.json','credentialed')",
    )
    .bind(raw_batch_id)
    .execute(&mut *tx)
    .await
    .expect("seed confirmed EOD batch");
    sqlx::query(
        "INSERT INTO trading_calendars
         (exchange, session_date, session_type, timezone, source, source_version,
          source_batch_id, content_sha256, retrieved_at)
         VALUES ('KRX',DATE '2026-08-13','TRADING','Asia/Seoul','synthetic',
                 'candidate-http-calendar-1',$1,repeat('8',64),
                 TIMESTAMPTZ '2026-08-13 07:00:00Z')",
    )
    .bind(raw_batch_id)
    .execute(&mut *tx)
    .await
    .expect("seed confirmed KRX session");
    sqlx::query(
        "INSERT INTO candidate_price_publications
         (dataset_version_id, dataset_version, manifest_sha256, market,
          curated_generation, first_session, last_session, provider,
          entitlement_id, license_ref, source_revision, available_at, retrieved_at)
         VALUES ($1,$2,$3,'kr',2,DATE '2026-01-01',DATE '2026-08-13','synthetic',
                 $4,'krx-2026-01','candidate-price-1',
                 TIMESTAMPTZ '2026-08-13 06:40:00Z',TIMESTAMPTZ '2026-08-13 07:00:00Z')",
    )
    .bind(price.0)
    .bind(&price.1)
    .bind(&price.2)
    .bind(entitlement_id)
    .execute(&mut *tx)
    .await
    .expect("seed candidate price publication");
    let universe_id: Uuid = sqlx::query_scalar(
        "INSERT INTO candidate_universe_snapshots
         (index_id, as_of_date, dataset_version_id, manifest_sha256, provider,
          entitlement_id, entitlement_date, license_ref, source_revision,
          available_at, retrieved_at, member_count)
         VALUES ('kospi200',DATE '2026-08-13',$1,$2,'synthetic',
                 $3,DATE '2026-08-13','krx-2026-01','fixture-universe-1',
                 TIMESTAMPTZ '2026-08-13 06:40:00Z',
                 TIMESTAMPTZ '2026-08-13 07:00:00Z',5) RETURNING id",
    )
    .bind(universe_pin.1)
    .bind(&universe_pin.2)
    .bind(entitlement_id)
    .fetch_one(&mut *tx)
    .await
    .expect("seed candidate universe");
    for instrument in INSTRUMENTS {
        sqlx::query(
            "INSERT INTO candidate_universe_members
             (universe_snapshot_id, instrument_id, announced_at, effective_from,
              available_at, source_revision)
             VALUES ($1,$2,TIMESTAMPTZ '2026-01-01 00:00:00Z',DATE '2026-01-01',
                     TIMESTAMPTZ '2026-01-01 00:01:00Z','fixture-universe-1')",
        )
        .bind(universe_id)
        .bind(instrument)
        .execute(&mut *tx)
        .await
        .expect("seed universe member");
    }
    let sector_id: Uuid = sqlx::query_scalar(
        "INSERT INTO candidate_sector_versions
          (taxonomy_id, taxonomy_version, effective_from, available_at, retrieved_at,
          provider, entitlement_id, entitlement_date, license_ref,
          source_revision, dataset_version_id, manifest_sha256)
         VALUES ('krx-sector','candidate-http-v1',DATE '2026-01-01',
                 TIMESTAMPTZ '2026-01-01 00:01:00Z',TIMESTAMPTZ '2026-08-13 07:00:00Z',
                 'synthetic',$1,DATE '2026-08-13','krx-2026-01','fixture-sector-1',$2,$3) RETURNING id",
    )
    .bind(entitlement_id)
    .bind(sector_pin.1)
    .bind(&sector_pin.2)
    .fetch_one(&mut *tx)
    .await
    .expect("seed sector version");
    for instrument in INSTRUMENTS {
        sqlx::query(
            "INSERT INTO candidate_sector_entries
             (sector_version_id, instrument_id, sector_code, sector_name,
              fundamental_profile, effective_from, available_at, source_revision)
             VALUES ($1,$2,'TECH','Technology','NON_FINANCIAL',DATE '2026-01-01',
                     TIMESTAMPTZ '2026-01-01 00:01:00Z','fixture-sector-1')",
        )
        .bind(sector_id)
        .bind(instrument)
        .execute(&mut *tx)
        .await
        .expect("seed sector entry");
        sqlx::query(
            "INSERT INTO candidate_market_status_observations
             (instrument_id, trade_date, provider, entitlement_id, entitlement_date,
              license_ref, source_revision,
              available_at, retrieved_at, dataset_version_id, manifest_sha256)
             VALUES ($1,DATE '2026-08-13','synthetic',$2,DATE '2026-08-13','krx-2026-01',
                     'fixture-status-1',TIMESTAMPTZ '2026-08-13 06:40:00Z',
                     TIMESTAMPTZ '2026-08-13 07:00:00Z',$3,$4)",
        )
        .bind(instrument)
        .bind(entitlement_id)
        .bind(status_pin.1)
        .bind(&status_pin.2)
        .execute(&mut *tx)
        .await
        .expect("seed status observation");
        let flow_observation_id: Uuid = sqlx::query_scalar(
            "INSERT INTO candidate_investor_flows
             (instrument_id, trade_date, investor_class, net_amount, net_volume,
              provider, source_revision, available_at)
             VALUES ($1,DATE '2026-08-13','FOREIGN',1000000,100,
                     'krx','fixture-flow-1',TIMESTAMPTZ '2026-08-13 06:40:00Z')
             RETURNING id",
        )
        .bind(instrument)
        .fetch_one(&mut *tx)
        .await
        .expect("seed flow observation");
        sqlx::query(
            "INSERT INTO candidate_investor_flow_snapshot_rows
             (dataset_version_id,flow_observation_id,entitlement_id,entitlement_date,
              license_ref,retrieved_at,manifest_sha256)
             VALUES ($1,$2,$3,DATE '2026-08-13','krx-2026-01',
                     TIMESTAMPTZ '2026-08-13 07:00:00Z',$4)",
        )
        .bind(flow_pin.1)
        .bind(flow_observation_id)
        .bind(entitlement_id)
        .bind(&flow_pin.2)
        .execute(&mut *tx)
        .await
        .expect("seed flow snapshot membership");
        sqlx::query(
            "INSERT INTO candidate_fundamental_observations
             (instrument_id, fiscal_period_start, fiscal_period_end, period_kind,
             statement_scope, metric, value, disclosed_at, available_at, retrieved_at,
              provider, entitlement_id, entitlement_date, license_ref, source_revision,
              dataset_version_id, manifest_sha256)
             VALUES ($1,DATE '2025-01-01',DATE '2025-12-31','ANNUAL','CONSOLIDATED',
                     'roe',0.15,TIMESTAMPTZ '2026-03-01 00:00:00Z',
                     TIMESTAMPTZ '2026-03-01 00:01:00Z',
                     TIMESTAMPTZ '2026-08-13 07:00:00Z','synthetic',$2,DATE '2026-08-13',
                     'krx-2026-01','fixture-fundamental-1',$3,$4)",
        )
        .bind(instrument)
        .bind(entitlement_id)
        .bind(fundamental_pin.1)
        .bind(&fundamental_pin.2)
        .execute(&mut *tx)
        .await
        .expect("seed fundamental observation");
    }

    let run_id: Uuid = sqlx::query_scalar(
        "INSERT INTO stock_analysis_runs
         (as_of_date, cutoff_at, computation_seq, status, scoring_config_version,
          scoring_config_sha256, universe_snapshot_id, universe_entitlement_id,
          price_dataset_version_id, price_entitlement_id, price_curated_version,
          price_manifest_sha256, status_dataset_version_id, status_entitlement_id,
          status_manifest_sha256, flow_dataset_version_id, flow_entitlement_id,
          flow_manifest_sha256, fundamental_dataset_version_id,
          fundamental_entitlement_id, fundamental_manifest_sha256,
          sector_version_id, sector_entitlement_id, input_identity_sha256,
          summary_json, published_at)
         VALUES (DATE '2026-08-13',TIMESTAMPTZ '2026-08-13 07:00:00Z',1,'SUCCEEDED',
                 'candidate-score-v1',
                 '1cd70f7a79af85896b015f265bea8ae931bbba29aef12a0b95f32c82ee056377',
                 $1,$2,$3,$4,2,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,
                 repeat('6',64),
                 '{\"eligible_count\":5}'::jsonb,TIMESTAMPTZ '2026-08-13 07:01:00Z')
         RETURNING id",
    )
    .bind(universe_id)
    .bind(entitlement_id)
    .bind(price.0)
    .bind(entitlement_id)
    .bind(&price.2)
    .bind(status_pin.1)
    .bind(entitlement_id)
    .bind(&status_pin.2)
    .bind(flow_pin.1)
    .bind(entitlement_id)
    .bind(&flow_pin.2)
    .bind(fundamental_pin.1)
    .bind(entitlement_id)
    .bind(&fundamental_pin.2)
    .bind(sector_id)
    .bind(entitlement_id)
    .fetch_one(&mut *tx)
    .await
    .expect("seed successful candidate run");
    let feed_id: Uuid = sqlx::query_scalar(
        "INSERT INTO candidate_feed_snapshots
         (run_id, as_of_date, computation_seq, status, published_at)
         VALUES ($1,DATE '2026-08-13',1,'PUBLISHED',TIMESTAMPTZ '2026-08-13 07:01:00Z')
         RETURNING id",
    )
    .bind(run_id)
    .fetch_one(&mut *tx)
    .await
    .expect("seed candidate feed");
    for (index, instrument) in INSTRUMENTS.iter().enumerate() {
        let rank = i32::try_from(index + 1).expect("rank fits i32");
        let score = 90.0 - index as f64;
        let snapshot_id: Uuid = sqlx::query_scalar(
            "INSERT INTO stock_analysis_snapshots
             (run_id, instrument_id, sector_code, fundamental_profile, eligible,
              exclusion_codes, flow_score, fundamental_score, technical_score,
              total_score, flow_coverage, fundamental_coverage, technical_coverage,
              evidence_strength, rank, normalization_scope, factors_json,
              scenarios_json, provenance_json, content_sha256)
             VALUES ($1,$2,'TECH','candidate-non-financial-v1',true,'[]'::jsonb,
                     $3,$3,$3,$3,1,1,1,'STRONG',$4,'SECTOR',
                     '{\"return_20\":{\"raw\":0.1,\"normalized\":1.2}}'::jsonb,
                     '{\"bullish\":{\"label\":\"BULLISH\"},
                       \"neutral\":{\"label\":\"NEUTRAL\"},
                       \"bearish\":{\"label\":\"BEARISH\"}}'::jsonb,
                     '{\"source\":\"fixture\"}'::jsonb,repeat($5,64))
             RETURNING id",
        )
        .bind(run_id)
        .bind(instrument)
        .bind(score)
        .bind(rank)
        .bind(char::from(b'a' + u8::try_from(index).unwrap()).to_string())
        .fetch_one(&mut *tx)
        .await
        .expect("seed analysis snapshot");
        sqlx::query(
            "INSERT INTO candidate_feed_items
             (feed_id, run_id, stock_analysis_snapshot_id, instrument_id, rank)
             VALUES ($1,$2,$3,$4,$5)",
        )
        .bind(feed_id)
        .bind(run_id)
        .bind(snapshot_id)
        .bind(instrument)
        .bind(rank)
        .execute(&mut *tx)
        .await
        .expect("seed feed item");
    }
    tx.commit().await.expect("candidate fixture commits");

    sqlx::query(
        "UPDATE data_entitlements
            SET covered_datasets=$1, covered_uses=$2, updated_at=clock_timestamp()
          WHERE status='ACTIVE'",
    )
    .bind(json!([
        "krx_eod_bars",
        "krx_market_status",
        "krx_investor_flows",
        "krx_fundamentals",
        "krx_kospi200_membership",
        "krx_sector_classification"
    ]))
    .bind(json!([
        "dataset",
        "factor",
        "recommendation",
        "candidate",
        "backtest",
        "report",
        "benchmark",
        "paper_view",
        "payload",
        "download"
    ]))
    .execute(&h.owner_pool)
    .await
    .expect("enable candidate entitlement");
    run_id
}

#[allow(clippy::too_many_arguments)]
async fn publish_credentialed_rolling_day(
    h: &Harness,
    raw: &RawStore,
    curated: &CurateStore,
    data_root: &Path,
    sink: &PostgresCandidateSourceSink,
    publication_sink: &PostgresPublicationSink,
    as_of: TradingDate,
    retrieved_at: UtcTimestamp,
) {
    let provider = CredentialedRollingCandidateProvider;
    let source = ingest_bundle_with_kinds(
        raw,
        &provider,
        &IngestRequest::new(MARKET_KR.to_owned(), as_of, retrieved_at),
        Some(ROLLING_LICENSE_REF),
        &CANDIDATE_RESPONSE_KINDS,
    )
    .expect("credentialed rolling candidate Raw");
    let price = ingest_bundle(
        raw,
        &provider,
        &IngestRequest::new(MARKET_KR.to_owned(), as_of, retrieved_at),
        Some(ROLLING_LICENSE_REF),
    )
    .expect("credentialed rolling price Raw");
    let price_bundle = PublicationBundle::from_raw(raw, &price.entry)
        .expect("credentialed rolling publication bundle");
    publication_sink
        .publish(&price_bundle)
        .await
        .expect("credentialed EOD/calendar publication");
    let (calendar, master) =
        curation_inputs_from_raw(raw, &price.entry).expect("rolling curation inputs");
    let curated_outcome = curate_batch(
        raw,
        &price.entry,
        &calendar,
        &master,
        curated,
        &CurateRequest {
            dataset_id: &DatasetId::parse("krx_eod_bars").expect("price dataset id"),
            market: MARKET_KR,
            source: "krx",
            now: retrieved_at,
        },
    )
    .expect("credentialed rolling price curation");
    let evidence = price_curation_evidence(raw, &price.entry, &curated_outcome.manifest)
        .expect("credentialed rolling price evidence");
    let reference_sha256 = price
        .entry
        .files
        .iter()
        .find(|file| file.kind == market_data::ResponseKind::Reference)
        .and_then(|file| file.content_hash.as_str().strip_prefix("sha256:"))
        .expect("credentialed rolling reference hash");
    sink.register_candidate_instruments(&CandidateInstrumentCatalog {
        master: &master,
        entitlement_id: ROLLING_ENTITLEMENT_ID,
        contract_reference: ROLLING_LICENSE_REF,
        entitlement_date: as_of,
        reference_sha256,
        source_revision: &price.batch_id.to_string(),
        retrieved_at,
    })
    .await
    .expect("credentialed rolling instrument catalog");
    let bindings = sink
        .catalog_candidate_batch(&source)
        .await
        .expect("credentialed rolling source catalog");
    let batch = prepare_candidate_batch(&source, as_of, retrieved_at, &bindings)
        .expect("credentialed rolling typed batch");
    assert_eq!(
        publish_candidate_batch(sink, &batch)
            .await
            .expect("credentialed rolling source seal"),
        PublishOutcome::Published
    );
    let price_version = curated_outcome.manifest.version.to_string();
    let raw_manifest_sha256 =
        candidate_raw_manifest_sha256(&price.entry).expect("credentialed price Raw hash");
    assert_eq!(
        sink.publish_price(&CandidatePricePublication {
            raw_batch_id: price.batch_id.as_uuid(),
            raw_manifest_sha256: &raw_manifest_sha256,
            fetch_mode: price.entry.mode,
            entitlement_date: price.entry.date,
            evidence: &evidence,
            dataset_version: &price_version,
            storage_path: data_root.to_str().expect("UTF-8 rolling root"),
            provider: "krx",
            entitlement_id: ROLLING_ENTITLEMENT_ID,
            license_ref: ROLLING_LICENSE_REF,
            available_at: retrieved_at,
            retrieved_at,
        })
        .await
        .expect("credentialed rolling price seal")
        .1,
        PublishOutcome::Published
    );
    let _: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM candidate_raw_batch_publications
          WHERE batch_id IN ($1,$2) AND state='PUBLISHED' AND fetch_mode='credentialed'",
    )
    .bind(source.batch_id.as_uuid())
    .bind(price.batch_id.as_uuid())
    .fetch_one(&h.owner_pool)
    .await
    .expect("credentialed rolling seals are queryable");
}

fn rolling_http_today() -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(2026, 8, 17).unwrap()
}

fn rolling_http_post_close() -> bool {
    true
}

#[tokio::test]
async fn credentialed_rolling_raw_reaches_real_http_and_exact_entitlement_gate() {
    let Some(mut h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    sqlx::query(
        "INSERT INTO data_entitlements
         (id,contract_document_sha256,contract_reference,status,covered_datasets,
          covered_uses,effective_from,effective_until,managed_by)
         VALUES ($1,repeat('9',64),$2,'ACTIVE',$3,'[\"candidate\"]'::jsonb,
                 DATE '2020-01-01',DATE '2030-12-31',$4)",
    )
    .bind(ROLLING_ENTITLEMENT_ID)
    .bind(ROLLING_LICENSE_REF)
    .bind(json!([
        "krx_eod_bars",
        "krx_investor_flows",
        "krx_market_status",
        "krx_fundamentals",
        "krx_kospi200_membership",
        "krx_sector_classification"
    ]))
    .bind(h.owner.user_id)
    .execute(&h.owner_pool)
    .await
    .expect("credentialed rolling entitlement");
    let data_root = tempfile::tempdir().expect("rolling HTTP Raw/curated root");
    let raw = RawStore::new(data_root.path());
    let curated = CurateStore::new(data_root.path());
    let publisher_pool = h.research_writer_pool().await;
    let sink = PostgresCandidateSourceSink::new(publisher_pool.clone());
    let publication_sink = PostgresPublicationSink::new(publisher_pool.clone());
    let day1 = TradingDate::parse("2026-08-14").unwrap();
    let day1_retrieved = UtcTimestamp::parse_rfc3339("2026-08-14T07:30:00Z").unwrap();
    let day2 = TradingDate::parse("2026-08-17").unwrap();
    let day2_retrieved = UtcTimestamp::parse_rfc3339("2026-08-17T07:30:00Z").unwrap();
    sqlx::query("UPDATE candidate_scoring_configs SET created_at=$1")
        .bind(day1_retrieved.as_datetime())
        .execute(&h.owner_pool)
        .await
        .expect("rolling HTTP scoring clock");
    sqlx::query(
        "UPDATE candidate_scheduler_control
            SET active=true,required_fetch_mode='credentialed'
          WHERE control_key='scheduler'",
    )
    .execute(&h.owner_pool)
    .await
    .expect("credentialed scheduler mode");
    let worker_pool = h.worker_pool().await;
    let queue_config = QueueConfig {
        lease: Duration::from_secs(30),
        backoff_base: Duration::from_millis(10),
    };
    let queue = JobQueue::new(worker_pool.clone(), None, queue_config);
    let runner_config = CandidateRunnerConfig::new(Duration::from_millis(100), queue_config.lease)
        .expect("rolling HTTP runner config");
    let seoul = FixedOffset::east_opt(9 * 60 * 60).unwrap();

    publish_credentialed_rolling_day(
        &h,
        &raw,
        &curated,
        data_root.path(),
        &sink,
        &publication_sink,
        day1,
        day1_retrieved,
    )
    .await;
    let scheduled_day1 = schedule_latest_candidate_run(
        &worker_pool,
        seoul.with_ymd_and_hms(2026, 8, 14, 17, 0, 0).unwrap(),
    )
    .await
    .expect("schedule credentialed rolling day one");
    assert_eq!(
        run_once(
            &worker_pool,
            &queue,
            "candidate-http-rolling-day-one",
            &CandidateRunnerPaths {
                data_root: data_root.path().to_path_buf(),
            },
            &runner_config,
        )
        .await
        .expect("run credentialed rolling day one"),
        CandidateOutcome::Succeeded {
            job_id: scheduled_day1.job_id,
            run_id: scheduled_day1.run_id,
        }
    );

    publish_credentialed_rolling_day(
        &h,
        &raw,
        &curated,
        data_root.path(),
        &sink,
        &publication_sink,
        day2,
        day2_retrieved,
    )
    .await;
    let scheduled_day2 = schedule_latest_candidate_run(
        &worker_pool,
        seoul.with_ymd_and_hms(2026, 8, 17, 17, 0, 0).unwrap(),
    )
    .await
    .expect("schedule credentialed rolling day two");
    assert_eq!(
        run_once(
            &worker_pool,
            &queue,
            "candidate-http-rolling-day-two",
            &CandidateRunnerPaths {
                data_root: data_root.path().to_path_buf(),
            },
            &runner_config,
        )
        .await
        .expect("run credentialed rolling day two"),
        CandidateOutcome::Succeeded {
            job_id: scheduled_day2.job_id,
            run_id: scheduled_day2.run_id,
        }
    );
    h.restart_api_with_candidate_clock(rolling_http_today, rolling_http_post_close)
        .await;
    let response = h
        .get("/api/v1/candidates/feed/latest", Some(&h.member))
        .await;
    assert_eq!(status(&response), StatusCode::OK);
    let feed = Harness::body_json(response).await;
    assert_eq!(feed["as_of"], "2026-08-17");
    let day2_identity: String =
        sqlx::query_scalar("SELECT input_identity_sha256 FROM stock_analysis_runs WHERE id=$1")
            .bind(scheduled_day2.run_id)
            .fetch_one(&h.owner_pool)
            .await
            .expect("day-two rolling input identity");
    assert_eq!(feed["dataset_pins"]["input_identity_sha256"], day2_identity);
    assert_eq!(feed["items"].as_array().unwrap().len(), 5);
    assert_eq!(feed["license_attributions"].as_array().unwrap().len(), 6);

    let recent_listing: (bool, bool) = sqlx::query_as(
        "SELECT eligible,
                exclusion_codes @> '[\"INSUFFICIENT_PRICE_HISTORY\"]'::jsonb
           FROM stock_analysis_snapshots
          WHERE run_id=$1 AND instrument_id='100007.KRX'",
    )
    .bind(scheduled_day2.run_id)
    .fetch_one(&h.owner_pool)
    .await
    .expect("typed recent-listing exclusion");
    assert_eq!(recent_listing, (false, true));

    let response = h
        .send(
            "POST",
            "/api/v1/screener/query",
            Some(&h.member),
            false,
            Some("rolling-http-screener"),
            None,
            Some(json!({
                "run_id": scheduled_day2.run_id,
                "criteria": {
                    "sectors": [],
                    "evidence_strength": [],
                    "min_total_score": null,
                    "min_flow_score": null,
                    "min_fundamental_score": null,
                    "min_technical_score": null
                },
                "limit": 10
            })),
        )
        .await;
    assert_eq!(status(&response), StatusCode::OK);
    let screener = Harness::body_json(response).await;
    let screener_ids = screener["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["instrument_id"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(screener_ids.len(), 6);
    assert!(!screener_ids.contains("100007.KRX"));

    sqlx::query(
        "INSERT INTO data_entitlements
         (contract_document_sha256,contract_reference,status,covered_datasets,
          covered_uses,effective_from,effective_until,managed_by)
         SELECT repeat('a',64),'fixture://candidate-http-unrelated','ACTIVE',
                covered_datasets,covered_uses,effective_from,effective_until,managed_by
           FROM data_entitlements WHERE id=$1",
    )
    .bind(ROLLING_ENTITLEMENT_ID)
    .execute(&h.owner_pool)
    .await
    .expect("unrelated active all-source entitlement");
    sqlx::query("UPDATE data_entitlements SET status='REVOKED' WHERE id=$1")
        .bind(ROLLING_ENTITLEMENT_ID)
        .execute(&h.owner_pool)
        .await
        .expect("revoke exact rolling entitlement");
    let response = h
        .get("/api/v1/candidates/feed/latest", Some(&h.member))
        .await;
    assert_eq!(status(&response), StatusCode::FORBIDDEN);
    let denied = Harness::body_json(response).await;
    assert_eq!(Harness::error_code(&denied), "DATA_ENTITLEMENT_REQUIRED");
    assert!(denied.get("items").is_none());

    publisher_pool.close().await;
    worker_pool.close().await;
    h.teardown().await;
}

#[tokio::test]
async fn candidate_feed_analysis_screener_and_saved_screen_are_one_secure_surface() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let run_id = seed_published_candidate_feed(&h).await;

    let response = h
        .get("/api/v1/candidates/feed/latest", Some(&h.member))
        .await;
    assert_eq!(status(&response), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );
    let feed = Harness::body_json(response).await;
    assert_eq!(feed["state"], "READY");
    assert_eq!(feed["items"].as_array().unwrap().len(), 5);
    assert!(feed["disclaimer"].as_str().unwrap().contains("확률"));
    assert_eq!(
        feed["dataset_pins"]["input_identity_sha256"],
        "6".repeat(64)
    );
    assert_eq!(feed["license_attributions"].as_array().unwrap().len(), 6);
    assert_eq!(feed["license_attributions"][0]["source"], "flow");
    assert!(
        feed["license_attributions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["source"] == "price" && row["dataset_id"] == "krx_eod_bars")
    );
    assert!(
        feed["license_attributions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| {
                row["license_ref"].is_string()
                    && row["entitlement_id"].is_string()
                    && row["contract_reference"].is_string()
            })
    );

    let response = h
        .get(
            "/api/v1/stocks/200001.KRX/analysis?date=2026-08-13",
            Some(&h.member),
        )
        .await;
    assert_eq!(status(&response), StatusCode::OK);
    let analysis = Harness::body_json(response).await;
    assert_eq!(analysis["analysis"]["instrument_id"], "200001.KRX");
    assert_eq!(
        analysis["analysis"]["fundamental_profile"],
        "candidate-non-financial-v1"
    );
    assert!(analysis["analysis"]["scenarios"]["bullish"].is_object());

    let response = h
        .send(
            "POST",
            "/api/v1/screener/query",
            Some(&h.member),
            false,
            Some("candidate-screen-query"),
            None,
            Some(json!({
                "run_id": run_id,
                "criteria": {
                    "sectors": ["TECH"],
                    "evidence_strength": ["STRONG"],
                    "min_total_score": 87,
                    "min_flow_score": null,
                    "min_fundamental_score": null,
                    "min_technical_score": null
                },
                "limit": 2
            })),
        )
        .await;
    assert_eq!(status(&response), StatusCode::OK);
    let screened = Harness::body_json(response).await;
    assert_eq!(screened["run_id"], run_id.to_string());
    assert_eq!(screened["items"].as_array().unwrap().len(), 2);
    assert!(screened["next_cursor"].is_string());

    let cursor = screened["next_cursor"].as_str().unwrap().to_owned();
    let response = h
        .send(
            "POST",
            "/api/v1/screener/query",
            Some(&h.member),
            false,
            Some("candidate-screen-page-2"),
            None,
            Some(json!({
                "run_id": run_id,
                "criteria": {
                    "sectors": ["TECH"],
                    "evidence_strength": ["STRONG"],
                    "min_total_score": 87,
                    "min_flow_score": null,
                    "min_fundamental_score": null,
                    "min_technical_score": null
                },
                "cursor": cursor,
                "limit": 2
            })),
        )
        .await;
    assert_eq!(status(&response), StatusCode::OK);

    let response = h
        .send(
            "POST",
            "/api/v1/screener/query",
            Some(&h.member),
            false,
            Some("candidate-screen-filter-replay"),
            None,
            Some(json!({
                "run_id": run_id,
                "criteria": {
                    "sectors": ["TECH"],
                    "evidence_strength": ["STRONG"],
                    "min_total_score": 88,
                    "min_flow_score": null,
                    "min_fundamental_score": null,
                    "min_technical_score": null
                },
                "cursor": cursor,
                "limit": 2
            })),
        )
        .await;
    assert_eq!(status(&response), StatusCode::BAD_REQUEST);
    assert_eq!(
        Harness::error_code(&Harness::body_json(response).await),
        "INVALID_CURSOR"
    );

    let response = h
        .send(
            "POST",
            "/api/v1/screener/query",
            Some(&h.member),
            false,
            Some("candidate-screen-as-of-only"),
            None,
            Some(json!({
                "as_of": "2026-08-13",
                "criteria": {"sectors": [], "evidence_strength": []}
            })),
        )
        .await;
    assert_eq!(status(&response), StatusCode::BAD_REQUEST);
    assert_eq!(
        Harness::error_code(&Harness::body_json(response).await),
        "INVALID_PARAMETER"
    );

    let response = h
        .post(
            "/api/v1/screener/screens",
            Some(&h.member),
            true,
            json!({
                "name": "Strong technology",
                "criteria": {
                    "sectors": ["TECH"],
                    "evidence_strength": ["STRONG"],
                    "min_total_score": 80,
                    "min_flow_score": null,
                    "min_fundamental_score": null,
                    "min_technical_score": null
                }
            }),
        )
        .await;
    assert_eq!(status(&response), StatusCode::CREATED);
    let saved = Harness::body_json(response).await;
    let screen_id = saved["id"].as_str().expect("screen id");
    let response = h
        .get(
            &format!("/api/v1/screener/screens/{screen_id}"),
            Some(&h.owner),
        )
        .await;
    assert_eq!(status(&response), StatusCode::NOT_FOUND);

    sqlx::query("UPDATE data_entitlements SET status='REVOKED' WHERE status='ACTIVE'")
        .execute(&h.owner_pool)
        .await
        .expect("revoke candidate entitlement");
    let response = h
        .get("/api/v1/candidates/feed/latest", Some(&h.member))
        .await;
    assert_eq!(status(&response), StatusCode::FORBIDDEN);
    let denied = Harness::body_json(response).await;
    assert_eq!(Harness::error_code(&denied), "DATA_ENTITLEMENT_REQUIRED");

    h.teardown().await;
}

#[tokio::test]
async fn candidate_gate_requires_the_run_pinned_entitlement_not_just_any_active_grant() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let run_id = seed_published_candidate_feed(&h).await;
    let pinned_entitlement: Uuid = sqlx::query_scalar(
        "SELECT price_entitlement_id
           FROM stock_analysis_runs
          WHERE id = $1",
    )
    .bind(run_id)
    .fetch_one(&h.owner_pool)
    .await
    .expect("read pinned candidate entitlement");

    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO data_entitlements (
             contract_document_sha256, contract_reference, status,
             covered_datasets, covered_uses, effective_from, effective_until,
             managed_by
         )
         SELECT contract_document_sha256, 'candidate-duplicate-all-sources', 'ACTIVE',
                covered_datasets, covered_uses, effective_from,
                effective_until, managed_by
           FROM data_entitlements
          WHERE id = $1
         RETURNING id",
    )
    .bind(pinned_entitlement)
    .fetch_one(&h.owner_pool)
    .await
    .expect("seed duplicate active EOD entitlement");
    let active_eod_entitlements: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM data_entitlements
          WHERE status = 'ACTIVE'
            AND covered_datasets @> '[\"krx_eod_bars\"]'::jsonb",
    )
    .fetch_one(&h.owner_pool)
    .await
    .expect("count active EOD entitlements");
    assert_eq!(active_eod_entitlements, 2);

    // A second ACTIVE grant for the same dataset must not displace the exact
    // entitlement pinned onto the published run.
    let response = h
        .get("/api/v1/candidates/feed/latest", Some(&h.member))
        .await;
    assert_eq!(status(&response), StatusCode::OK);

    sqlx::query(
        "UPDATE data_entitlements SET status = 'REVOKED'
          WHERE id = $1",
    )
    .bind(pinned_entitlement)
    .execute(&h.owner_pool)
    .await
    .expect("revoke run-pinned entitlement");
    let response = h
        .get("/api/v1/candidates/feed/latest", Some(&h.member))
        .await;
    assert_eq!(status(&response), StatusCode::FORBIDDEN);
    assert_eq!(
        Harness::error_code(&Harness::body_json(response).await),
        "DATA_ENTITLEMENT_REQUIRED"
    );

    h.teardown().await;
}

#[tokio::test]
async fn candidate_freshness_uses_confirmed_db_close_not_calendar_or_wall_date() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let _run_id = seed_published_candidate_feed(&h).await;
    let repo = h.state().candidates();
    assert_eq!(
        repo.latest_confirmed_krx_close().await.unwrap(),
        Some(chrono::NaiveDate::from_ymd_opt(2026, 8, 13).unwrap())
    );

    // A current/future calendar projection is not a close. This models the
    // pre-close state; holiday and weekend projections must not advance it.
    let preclose_batch = Uuid::new_v4();
    for (date, session_type) in [
        ("2026-08-14", "TRADING"),
        ("2026-08-15", "CLOSED"),
        ("2026-08-16", "TRADING"),
    ] {
        sqlx::query(
            "INSERT INTO trading_calendars
             (exchange, session_date, session_type, timezone, source, source_version,
              source_batch_id, content_sha256, retrieved_at)
             VALUES ('KRX',$1::date,$2,'Asia/Seoul','synthetic','freshness-test',$3,repeat('7',64),now())",
        )
        .bind(date)
        .bind(session_type)
        .bind(preclose_batch)
        .execute(&h.owner_pool)
        .await
        .unwrap();
    }
    let preclose_repo = api_server::repos::candidates::CandidateRepo::with_close_clock(
        h.app_pool.clone(),
        || chrono::NaiveDate::from_ymd_opt(2026, 8, 14).unwrap(),
        || false,
    );
    assert_eq!(
        preclose_repo.latest_confirmed_krx_close().await.unwrap(),
        Some(chrono::NaiveDate::from_ymd_opt(2026, 8, 13).unwrap())
    );
    assert_eq!(
        preclose_repo
            .freshness_state(chrono::NaiveDate::from_ymd_opt(2026, 8, 13).unwrap())
            .await
            .unwrap(),
        "READY"
    );

    let postclose_repo = api_server::repos::candidates::CandidateRepo::with_close_clock(
        h.app_pool.clone(),
        || chrono::NaiveDate::from_ymd_opt(2026, 8, 14).unwrap(),
        || true,
    );
    assert_eq!(
        postclose_repo.latest_confirmed_krx_close().await.unwrap(),
        None,
        "after the close threshold an absent current EOD must not fall back"
    );
    assert_eq!(
        postclose_repo
            .freshness_state(chrono::NaiveDate::from_ymd_opt(2026, 8, 13).unwrap())
            .await
            .unwrap(),
        "STALE"
    );

    // Once the credentialed EOD batch arrives, the DB-confirmed close moves
    // forward even though the API process's configured Seoul date is fixed.
    sqlx::query(
        "INSERT INTO data_batches
         (provider, market, batch_date, kind, storage_path, content_sha256,
          bytes_size, retrieved_at, source_batch_id, source_file_name, fetch_mode)
         VALUES ('KRX','KR',DATE '2026-08-14','EOD','raw/freshness-test',repeat('6',64),
                 1,now(),$1,'bars-2026-08-14.json','credentialed')",
    )
    .bind(preclose_batch)
    .execute(&h.owner_pool)
    .await
    .unwrap();
    assert_eq!(
        postclose_repo.latest_confirmed_krx_close().await.unwrap(),
        Some(chrono::NaiveDate::from_ymd_opt(2026, 8, 14).unwrap())
    );
    assert_eq!(
        postclose_repo
            .freshness_state(chrono::NaiveDate::from_ymd_opt(2026, 8, 13).unwrap())
            .await
            .unwrap(),
        "STALE"
    );

    h.teardown().await;
}
