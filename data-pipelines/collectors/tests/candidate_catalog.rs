#[path = "../../../tests/support/candidate_rolling_provider.rs"]
#[allow(dead_code)]
mod candidate_rolling_provider;
#[allow(dead_code)]
mod common;

use chrono::{DateTime, NaiveTime, Utc};
use collectors::{
    CandidateInstrumentCatalog, CandidatePricePublication, HealthFailure,
    PostgresCandidateSourceSink, PostgresPublicationSink, PublishOutcome, WorkerError,
    candidate_healthcheck, prepare_candidate_batch, publish_candidate_batch,
    recover_candidate_batches,
};
use domain::{DatasetId, TradingDate, UtcTimestamp};
use market_data::{
    CANDIDATE_RESPONSE_KINDS, CurateRequest, CurateStore, FetchMode, IngestRequest, MARKET_KR,
    RawStore, curate_batch, curation_inputs_from_raw, ingest_bundle, ingest_bundle_with_kinds,
    price_curation_evidence,
};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use candidate_rolling_provider::RollingCandidateProvider;
use common::ScratchDb;

async fn open_status_origin(
    pool: &PgPool,
    batch_id: Uuid,
    version: &str,
    manifest_sha256: &str,
    entitlement_id: Uuid,
    contract_reference: &str,
    as_of: TradingDate,
) -> Uuid {
    sqlx::query("SELECT public.begin_candidate_raw_batch($1,'source',$2,'synthetic',$3,$4)")
        .bind(batch_id)
        .bind(manifest_sha256)
        .bind(contract_reference)
        .bind(as_of.as_naive_date())
        .execute(pool)
        .await
        .expect("begin concurrent status batch");
    let dataset_id: Uuid = sqlx::query_scalar(
        "SELECT public.register_candidate_source_dataset(
             'krx_market_status',$1,$2,$3,$4,$5)",
    )
    .bind(version)
    .bind(manifest_sha256)
    .bind(entitlement_id)
    .bind(contract_reference)
    .bind(as_of.as_naive_date())
    .fetch_one(pool)
    .await
    .expect("register concurrent status dataset");
    sqlx::query("SELECT public.bind_candidate_raw_dataset($1,'source','market_status',$2,false)")
        .bind(batch_id)
        .bind(dataset_id)
        .execute(pool)
        .await
        .expect("bind concurrent status dataset");
    dataset_id
}

#[allow(clippy::too_many_arguments)]
async fn insert_status_in_own_transaction(
    pool: PgPool,
    batch_id: Uuid,
    dataset_version_id: Uuid,
    manifest_sha256: String,
    entitlement_id: Uuid,
    contract_reference: String,
    as_of: TradingDate,
    source_revision: String,
    suspended: bool,
) -> Result<bool, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.candidate_raw_batch_id',$1,true)")
        .bind(batch_id.to_string())
        .execute(&mut *tx)
        .await?;
    let published = sqlx::query_scalar(
        "SELECT public.insert_candidate_market_status(
             '100001.KRX',$1,$2,false,false,false,false,false,'krx',
             $3,$1,$4,$5,$6,$6,$7,$8)",
    )
    .bind(as_of.as_naive_date())
    .bind(suspended)
    .bind(entitlement_id)
    .bind(&contract_reference)
    .bind(&source_revision)
    .bind(
        UtcTimestamp::parse_rfc3339("2026-08-14T07:00:00Z")
            .expect("status time")
            .as_datetime(),
    )
    .bind(dataset_version_id)
    .bind(&manifest_sha256)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(published)
}

#[tokio::test]
async fn typed_status_natural_key_is_serialized_across_datasets() {
    let Some(db) = ScratchDb::create().await else {
        eprintln!("SKIP: DATABASE_URL is not set");
        return;
    };
    let contract_reference = "fixture://candidate-concurrency-license";
    sqlx::query(
        "INSERT INTO data_entitlements
         (contract_document_sha256,contract_reference,status,covered_datasets,
          covered_uses,effective_from,effective_until,managed_by)
         VALUES (repeat('7',64),$1,'ACTIVE',$2,'[\"candidate\"]'::jsonb,
                 DATE '2020-01-01',DATE '2030-12-31',
                 '00000000-0000-4000-8000-000000000042'::uuid)",
    )
    .bind(contract_reference)
    .bind(json!([
        "krx_eod_bars",
        "krx_investor_flows",
        "krx_market_status",
        "krx_fundamentals",
        "krx_kospi200_membership",
        "krx_sector_classification"
    ]))
    .execute(&db.supervisor)
    .await
    .expect("candidate concurrency entitlement");
    let entitlement_id: Uuid =
        sqlx::query_scalar("SELECT id FROM data_entitlements WHERE contract_reference=$1")
            .bind(contract_reference)
            .fetch_one(&db.supervisor)
            .await
            .expect("candidate concurrency entitlement id");
    let as_of = TradingDate::parse("2026-08-14").expect("status as-of");
    sqlx::query(
        "SELECT public.register_candidate_instrument(
             '100001.KRX','100001','Concurrent candidate','EQUITY',DATE '2020-01-02',
             $1,$2,$3,repeat('9',64),'concurrency-reference-v1',$4)",
    )
    .bind(entitlement_id)
    .bind(contract_reference)
    .bind(as_of.as_naive_date())
    .bind(
        UtcTimestamp::parse_rfc3339("2026-08-14T07:00:00Z")
            .expect("registration time")
            .as_datetime(),
    )
    .execute(&db.writer)
    .await
    .expect("register concurrency instrument");

    let replay_batch = Uuid::new_v4();
    let replay_hash = "1".repeat(64);
    let replay_dataset = open_status_origin(
        &db.writer,
        replay_batch,
        "concurrent-replay-v1",
        &replay_hash,
        entitlement_id,
        contract_reference,
        as_of,
    )
    .await;
    let replay_a = insert_status_in_own_transaction(
        db.writer.clone(),
        replay_batch,
        replay_dataset,
        replay_hash.clone(),
        entitlement_id,
        contract_reference.to_owned(),
        as_of,
        "concurrent-exact-v1".to_owned(),
        false,
    );
    let replay_b = insert_status_in_own_transaction(
        db.writer.clone(),
        replay_batch,
        replay_dataset,
        replay_hash,
        entitlement_id,
        contract_reference.to_owned(),
        as_of,
        "concurrent-exact-v1".to_owned(),
        false,
    );
    let (replay_a, replay_b) = tokio::join!(replay_a, replay_b);
    let mut replay_results = [
        replay_a.expect("first exact concurrent replay"),
        replay_b.expect("second exact concurrent replay"),
    ];
    replay_results.sort_unstable();
    assert_eq!(replay_results, [false, true]);

    let conflict_batch_a = Uuid::new_v4();
    let conflict_batch_b = Uuid::new_v4();
    let conflict_hash_a = "2".repeat(64);
    let conflict_hash_b = "3".repeat(64);
    let conflict_dataset_a = open_status_origin(
        &db.writer,
        conflict_batch_a,
        "concurrent-conflict-a",
        &conflict_hash_a,
        entitlement_id,
        contract_reference,
        as_of,
    )
    .await;
    let conflict_dataset_b = open_status_origin(
        &db.writer,
        conflict_batch_b,
        "concurrent-conflict-b",
        &conflict_hash_b,
        entitlement_id,
        contract_reference,
        as_of,
    )
    .await;
    let conflict_a = insert_status_in_own_transaction(
        db.writer.clone(),
        conflict_batch_a,
        conflict_dataset_a,
        conflict_hash_a,
        entitlement_id,
        contract_reference.to_owned(),
        as_of,
        "concurrent-conflict-v1".to_owned(),
        false,
    );
    let conflict_b = insert_status_in_own_transaction(
        db.writer.clone(),
        conflict_batch_b,
        conflict_dataset_b,
        conflict_hash_b,
        entitlement_id,
        contract_reference.to_owned(),
        as_of,
        "concurrent-conflict-v1".to_owned(),
        true,
    );
    let (conflict_a, conflict_b) = tokio::join!(conflict_a, conflict_b);
    let outcomes = [conflict_a, conflict_b];
    assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
    let loser = outcomes
        .iter()
        .find_map(|outcome| outcome.as_ref().err())
        .expect("one contradictory natural key loses");
    assert_eq!(
        loser
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("23514")
    );
    let conflict_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM candidate_market_status_observations
          WHERE instrument_id='100001.KRX' AND trade_date=$1
            AND source_revision='concurrent-conflict-v1'",
    )
    .bind(as_of.as_naive_date())
    .fetch_one(&db.supervisor)
    .await
    .expect("one immutable concurrent status fact");
    assert_eq!(conflict_rows, 1);
    let cataloged_batches: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM candidate_raw_batch_publications
          WHERE batch_id=ANY($1) AND surface='source' AND state='CATALOGED'",
    )
    .bind(vec![conflict_batch_a, conflict_batch_b])
    .fetch_one(&db.supervisor)
    .await
    .expect("conflicting batches remain unsealed");
    assert_eq!(cataloged_batches, 2);
    db.drop_db().await;
}

#[tokio::test]
async fn research_writer_catalogs_exact_raw_sources_without_broad_dataset_dml() {
    let Some(db) = ScratchDb::create().await else {
        eprintln!("SKIP: DATABASE_URL is not set");
        return;
    };
    let contract_reference = "fixture://candidate-license";
    sqlx::query(
        "INSERT INTO data_entitlements
         (contract_document_sha256, contract_reference, status, covered_datasets,
          covered_uses, effective_from, effective_until, managed_by)
         VALUES (repeat('8',64),$1,'ACTIVE',$2,'[\"candidate\"]'::jsonb,
                 DATE '2020-01-01',DATE '2030-12-31',
                 '00000000-0000-4000-8000-000000000042'::uuid)",
    )
    .bind(contract_reference)
    .bind(json!([
        "krx_eod_bars",
        "krx_investor_flows",
        "krx_market_status",
        "krx_fundamentals",
        "krx_kospi200_membership",
        "krx_sector_classification"
    ]))
    .execute(&db.supervisor)
    .await
    .expect("candidate entitlement");
    let raw_root = tempfile::tempdir().expect("candidate Raw root");
    let raw = RawStore::new(raw_root.path());
    let provider = RollingCandidateProvider;
    let as_of = TradingDate::parse("2026-08-14").expect("as-of date");
    let retrieved_at = UtcTimestamp::parse_rfc3339("2026-08-14T07:00:00Z").expect("retrieved at");
    let outcome = ingest_bundle_with_kinds(
        &raw,
        &provider,
        &IngestRequest::new(MARKET_KR.to_owned(), as_of, retrieved_at),
        Some(contract_reference),
        &CANDIDATE_RESPONSE_KINDS,
    )
    .expect("immutable candidate Raw batch");
    let sink = PostgresCandidateSourceSink::new(db.writer.clone());
    let bindings = sink
        .catalog_candidate_batch(&outcome)
        .await
        .expect("narrow source catalog");
    assert_eq!(bindings.len(), 5);
    let logical_paths: Vec<String> = sqlx::query_scalar(
        "SELECT storage_path FROM dataset_versions
          WHERE dataset_id LIKE 'krx_%' AND dataset_id <> 'krx_eod_bars'
          ORDER BY dataset_id",
    )
    .fetch_all(&db.supervisor)
    .await
    .expect("logical candidate catalog paths");
    assert_eq!(logical_paths.len(), 5);
    assert!(
        logical_paths
            .iter()
            .all(|path| path.starts_with("db://candidate/"))
    );
    assert_eq!(
        sink.catalog_candidate_batch(&outcome)
            .await
            .expect("exact catalog replay"),
        bindings
    );
    let batch = prepare_candidate_batch(&outcome, as_of, retrieved_at, &bindings)
        .expect("typed candidate batch");
    let direct = sqlx::query(
        "INSERT INTO dataset_versions
         (dataset_id,version,status,manifest_sha256,storage_path)
         VALUES ('escape','v1','READY',repeat('1',64),'db://escape')",
    )
    .execute(&db.writer)
    .await
    .expect_err("research_writer must not have broad dataset catalog DML");
    assert_eq!(
        direct
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("42501")
    );
    let direct_instrument = sqlx::query(
        "INSERT INTO instruments (id,symbol,venue,currency)
         VALUES ('999999.KRX','999999','KRX','KRW')",
    )
    .execute(&db.writer)
    .await
    .expect_err("research_writer must not have broad instrument DML");
    assert_eq!(
        direct_instrument
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("42501")
    );

    let price_root = tempfile::tempdir().expect("price data root");
    let price_raw = RawStore::new(price_root.path());
    let price_provider = RollingCandidateProvider;
    let price_date = as_of;
    let price_retrieved = retrieved_at;
    let price_raw_outcome = ingest_bundle(
        &price_raw,
        &price_provider,
        &IngestRequest::new(MARKET_KR.to_owned(), price_date, price_retrieved),
        Some(contract_reference),
    )
    .expect("price Raw batch");
    let (calendar, master) = curation_inputs_from_raw(&price_raw, &price_raw_outcome.entry)
        .expect("price curation inputs");
    let dataset_id = DatasetId::parse("krx_eod_bars").expect("price dataset id");
    let curated = CurateStore::new(price_root.path());
    let curated_outcome = curate_batch(
        &price_raw,
        &price_raw_outcome.entry,
        &calendar,
        &master,
        &curated,
        &CurateRequest {
            dataset_id: &dataset_id,
            market: MARKET_KR,
            source: "synthetic",
            now: price_retrieved,
        },
    )
    .expect("price curation");
    let evidence = price_curation_evidence(
        &price_raw,
        &price_raw_outcome.entry,
        &curated_outcome.manifest,
    )
    .expect("price publication evidence");
    let health_sessions = RollingCandidateProvider::sessions(as_of);
    let entitlement_id = sink
        .resolve_contract_entitlement(
            contract_reference,
            evidence.first_session,
            evidence.last_session,
        )
        .await
        .expect("exact six-source entitlement");
    let reference_sha256 = price_raw_outcome
        .entry
        .files
        .iter()
        .find(|file| file.kind == market_data::ResponseKind::Reference)
        .and_then(|file| file.content_hash.as_str().strip_prefix("sha256:"))
        .expect("reference hash");
    let source_revision = price_raw_outcome.batch_id.to_string();
    assert_eq!(
        sink.register_candidate_instruments(&CandidateInstrumentCatalog {
            master: &master,
            entitlement_id,
            contract_reference,
            entitlement_date: price_date,
            reference_sha256,
            source_revision: &source_revision,
            retrieved_at: price_retrieved,
        })
        .await
        .expect("Raw reference instrument catalog"),
        7
    );
    assert_eq!(
        sink.register_candidate_instruments(&CandidateInstrumentCatalog {
            master: &master,
            entitlement_id,
            contract_reference,
            entitlement_date: price_date,
            reference_sha256,
            source_revision: &source_revision,
            retrieved_at: price_retrieved,
        })
        .await
        .expect("exact instrument catalog replay"),
        0
    );
    let flow_source = batch
        .sources
        .iter()
        .find(|source| {
            matches!(
                source.document,
                market_data::CandidateDocument::InvestorFlow(_)
            )
        })
        .expect("prepared rolling flow source");
    let market_data::CandidateDocument::InvestorFlow(flow_document) = &flow_source.document else {
        unreachable!("flow source kind was selected")
    };
    let flow = flow_document.flows.first().expect("rolling flow row");
    let mut invalid_time = db.writer.begin().await.expect("invalid flow time tx");
    sqlx::query("SELECT set_config('app.candidate_raw_batch_id',$1,true)")
        .bind(batch.batch_id.to_string())
        .execute(&mut *invalid_time)
        .await
        .expect("scope invalid flow time attempt");
    let invalid_time_error = sqlx::query(
        "SELECT public.insert_candidate_investor_flow(
            $1,$2,$3,$4::numeric(28,4),$5::numeric(28,4),$6,$7,$8,
            $9,$10,$11,$12,$13,$14,$15,$16)",
    )
    .bind(flow.instrument.to_string())
    .bind(flow.trade_date.as_naive_date())
    .bind(match flow.investor_class {
        market_data::InvestorClass::Foreign => "FOREIGN",
        market_data::InvestorClass::Institution => "INSTITUTION",
    })
    .bind(flow.net_amount)
    .bind(flow.net_volume)
    .bind(&flow.currency)
    .bind(&flow.volume_unit)
    .bind(&flow_source.pin.provider)
    .bind(flow_source.pin.entitlement_id)
    .bind(batch.as_of.as_naive_date())
    .bind(&flow_source.pin.license_ref)
    .bind(&flow.source_revision)
    .bind(flow.available_at.as_datetime())
    .bind(flow.available_at.as_datetime() - chrono::Duration::seconds(1))
    .bind(flow_source.dataset_version_id)
    .bind(&flow_source.pin.manifest_sha256)
    .execute(&mut *invalid_time)
    .await
    .expect_err("database boundary rejects retrieved_at before available_at");
    assert_eq!(
        invalid_time_error
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("42501")
    );
    invalid_time
        .rollback()
        .await
        .expect("rollback invalid flow time attempt");
    assert_eq!(
        publish_candidate_batch(&sink, &batch)
            .await
            .expect("candidate publication"),
        PublishOutcome::Published
    );
    assert_eq!(
        publish_candidate_batch(&sink, &batch)
            .await
            .expect("candidate exact replay"),
        PublishOutcome::AlreadyPublished
    );
    let expected_credentialed_missing = CANDIDATE_RESPONSE_KINDS
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    let credentialed_missing = sink
        .missing_source_kinds(as_of, retrieved_at.as_datetime(), FetchMode::Credentialed)
        .await
        .expect("fetch-mode-aware source discovery")
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(credentialed_missing, expected_credentialed_missing);
    let mut tampered_replay = batch.clone();
    tampered_replay.raw_manifest_sha256 = "e".repeat(64);
    assert!(matches!(
        publish_candidate_batch(&sink, &tampered_replay).await,
        Err(collectors::CandidatePipelineError::Publish(
            collectors::SinkError::Conflict(_)
        ))
    ));
    let sealed_fundamental = bindings
        .iter()
        .find(|binding| binding.kind == market_data::ResponseKind::Fundamentals)
        .expect("sealed fundamental origin");
    let stolen_batch_id = uuid::Uuid::new_v4();
    sqlx::query(
        "SELECT public.begin_candidate_raw_batch($1,'source',repeat('c',64),'synthetic',$2,$3)",
    )
    .bind(stolen_batch_id)
    .bind(contract_reference)
    .bind(as_of.as_naive_date())
    .execute(&db.writer)
    .await
    .expect("begin origin-stealing attack batch");
    let origin_steal = sqlx::query(
        "SELECT public.bind_candidate_raw_dataset($1,'source','fundamentals',$2,false)",
    )
    .bind(stolen_batch_id)
    .bind(sealed_fundamental.dataset_version_id)
    .execute(&db.writer)
    .await
    .expect_err("a sealed dataset cannot be rebound as a new writable origin");
    assert_eq!(
        origin_steal
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("23514")
    );
    let sealed_universe = bindings
        .iter()
        .find(|binding| binding.kind == market_data::ResponseKind::IndexMembership)
        .expect("sealed universe origin");
    let reused_batch_id = uuid::Uuid::new_v4();
    let mut reused_tx = db.writer.begin().await.expect("reused attack tx");
    sqlx::query(
        "SELECT public.begin_candidate_raw_batch($1,'source',repeat('d',64),'synthetic',$2,$3)",
    )
    .bind(reused_batch_id)
    .bind(contract_reference)
    .bind(as_of.as_naive_date())
    .execute(&mut *reused_tx)
    .await
    .expect("begin reused attack batch");
    sqlx::query("SELECT public.bind_candidate_raw_dataset($1,'source','index_membership',$2,true)")
        .bind(reused_batch_id)
        .bind(sealed_universe.dataset_version_id)
        .execute(&mut *reused_tx)
        .await
        .expect("bind a legal read-only reused PIT dataset");
    sqlx::query("SELECT set_config('app.candidate_raw_batch_id',$1,true)")
        .bind(reused_batch_id.to_string())
        .execute(&mut *reused_tx)
        .await
        .expect("scope reused attack batch");
    let snapshot_id: uuid::Uuid = sqlx::query_scalar(
        "SELECT id FROM candidate_universe_snapshots WHERE dataset_version_id=$1 LIMIT 1",
    )
    .bind(sealed_universe.dataset_version_id)
    .fetch_one(&mut *reused_tx)
    .await
    .expect("sealed universe snapshot");
    let reused_append = sqlx::query(
        "SELECT public.insert_candidate_universe_member(
             $1,'100001.KRX',$2,$3,NULL,$2,'rolling-membership-v1')",
    )
    .bind(snapshot_id)
    .bind(retrieved_at.as_datetime())
    .bind(as_of.as_naive_date())
    .execute(&mut *reused_tx)
    .await
    .expect_err("a reused PIT binding must remain read-only");
    assert_eq!(
        reused_append
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("42501")
    );
    reused_tx.rollback().await.expect("rollback reused attack");
    let renewed_contract = "fixture://candidate-license-renewed";
    sqlx::query(
        "INSERT INTO data_entitlements
         (contract_document_sha256, contract_reference, status, covered_datasets,
          covered_uses, effective_from, effective_until, managed_by)
         SELECT repeat('a',64),$1,'ACTIVE',covered_datasets,covered_uses,
                effective_from,effective_until,managed_by
           FROM data_entitlements WHERE contract_reference=$2",
    )
    .bind(renewed_contract)
    .bind(contract_reference)
    .execute(&db.supervisor)
    .await
    .expect("renewed candidate entitlement");
    let renewed_outcome = ingest_bundle_with_kinds(
        &raw,
        &provider,
        &IngestRequest::new(MARKET_KR.to_owned(), as_of, retrieved_at),
        Some(renewed_contract),
        &CANDIDATE_RESPONSE_KINDS,
    )
    .expect("same PIT bytes under a renewed exact contract");
    let renewed_bindings = sink
        .catalog_candidate_batch(&renewed_outcome)
        .await
        .expect("renewed contract creates a distinct immutable origin");
    assert!(
        renewed_bindings
            .iter()
            .all(|binding| !binding.reused_existing)
    );
    for pit_kind in [
        market_data::ResponseKind::Fundamentals,
        market_data::ResponseKind::IndexMembership,
        market_data::ResponseKind::SectorClassification,
    ] {
        let original = bindings
            .iter()
            .find(|binding| binding.kind == pit_kind)
            .expect("original PIT binding");
        let renewed = renewed_bindings
            .iter()
            .find(|binding| binding.kind == pit_kind)
            .expect("renewed PIT binding");
        assert_ne!(original.dataset_version_id, renewed.dataset_version_id);
        assert_eq!(renewed.license_ref, renewed_contract);
    }
    let renewed_batch =
        prepare_candidate_batch(&renewed_outcome, as_of, retrieved_at, &renewed_bindings)
            .expect("renewed typed candidate batch");

    let original_fundamental = bindings
        .iter()
        .find(|binding| binding.kind == market_data::ResponseKind::Fundamentals)
        .expect("original fundamental binding");
    let renewed_fundamental = renewed_bindings
        .iter()
        .find(|binding| binding.kind == market_data::ResponseKind::Fundamentals)
        .expect("renewed fundamental binding");
    let mut fundamental_attack = db.writer.begin().await.expect("fundamental attack tx");
    sqlx::query("SELECT set_config('app.candidate_raw_batch_id',$1,true)")
        .bind(renewed_outcome.batch_id.to_string())
        .execute(&mut *fundamental_attack)
        .await
        .expect("scope fundamental attack");
    let fundamental_conflict = sqlx::query(
        "SELECT public.insert_candidate_fundamental(
             source.instrument_id,source.fiscal_period_start,source.fiscal_period_end,
             source.period_kind,source.statement_scope,source.metric,source.value+1,
             source.currency,source.unit_scale,source.audited,source.disclosed_at,
             source.available_at,$1,source.provider,$2,$3,$4,source.source_revision,
             source.restates_observation_id,$5,$6)
           FROM candidate_fundamental_observations AS source
          WHERE source.dataset_version_id=$7 LIMIT 1",
    )
    .bind(retrieved_at.as_datetime())
    .bind(renewed_fundamental.entitlement_id)
    .bind(as_of.as_naive_date())
    .bind(renewed_contract)
    .bind(renewed_fundamental.dataset_version_id)
    .bind(&renewed_fundamental.manifest_sha256)
    .bind(original_fundamental.dataset_version_id)
    .execute(&mut *fundamental_attack)
    .await
    .expect_err("same fundamental revision cannot contradict immutable content");
    assert_eq!(
        fundamental_conflict
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("23514")
    );
    fundamental_attack
        .rollback()
        .await
        .expect("rollback fundamental attack");

    let mut lineage_attack = db.writer.begin().await.expect("restatement attack tx");
    sqlx::query("SELECT set_config('app.candidate_raw_batch_id',$1,true)")
        .bind(renewed_outcome.batch_id.to_string())
        .execute(&mut *lineage_attack)
        .await
        .expect("scope restatement attack");
    let invalid_lineage = sqlx::query(
        "SELECT public.insert_candidate_fundamental(
             source.instrument_id,source.fiscal_period_start,source.fiscal_period_end,
             source.period_kind,source.statement_scope,source.metric,source.value,
             source.currency,source.unit_scale,source.audited,
             source.disclosed_at+interval '1 day',source.available_at+interval '1 day',
             $1,source.provider,$2,$3,$4,'malicious-restatement-v2',foreign_prior.id,$5,$6)
           FROM candidate_fundamental_observations AS source
           CROSS JOIN LATERAL (
               SELECT other.id FROM candidate_fundamental_observations AS other
                WHERE other.dataset_version_id=$7
                  AND other.instrument_id<>source.instrument_id
                  AND other.metric=source.metric LIMIT 1
           ) AS foreign_prior
          WHERE source.dataset_version_id=$7 LIMIT 1",
    )
    .bind(retrieved_at.as_datetime())
    .bind(renewed_fundamental.entitlement_id)
    .bind(as_of.as_naive_date())
    .bind(renewed_contract)
    .bind(renewed_fundamental.dataset_version_id)
    .bind(&renewed_fundamental.manifest_sha256)
    .bind(original_fundamental.dataset_version_id)
    .execute(&mut *lineage_attack)
    .await
    .expect_err("restatement cannot point at another instrument");
    assert_eq!(
        invalid_lineage
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("23514")
    );
    lineage_attack
        .rollback()
        .await
        .expect("rollback restatement attack");

    let original_universe = bindings
        .iter()
        .find(|binding| binding.kind == market_data::ResponseKind::IndexMembership)
        .expect("original universe binding");
    let renewed_universe = renewed_bindings
        .iter()
        .find(|binding| binding.kind == market_data::ResponseKind::IndexMembership)
        .expect("renewed universe binding");
    let mut universe_attack = db.writer.begin().await.expect("universe attack tx");
    sqlx::query("SELECT set_config('app.candidate_raw_batch_id',$1,true)")
        .bind(renewed_outcome.batch_id.to_string())
        .execute(&mut *universe_attack)
        .await
        .expect("scope universe attack");
    let renewed_snapshot_id: Uuid = sqlx::query_scalar(
        "SELECT public.insert_candidate_universe_snapshot(
             source.as_of_date,$1,$2,source.provider,$3,$4,$5,
             source.source_revision,source.available_at,$6,source.member_count)
           FROM candidate_universe_snapshots AS source
          WHERE source.dataset_version_id=$7 LIMIT 1",
    )
    .bind(renewed_universe.dataset_version_id)
    .bind(&renewed_universe.manifest_sha256)
    .bind(renewed_universe.entitlement_id)
    .bind(as_of.as_naive_date())
    .bind(renewed_contract)
    .bind(retrieved_at.as_datetime())
    .bind(original_universe.dataset_version_id)
    .fetch_one(&mut *universe_attack)
    .await
    .expect("identical universe snapshot may be reacquired under renewed rights");
    let universe_conflict = sqlx::query(
        "SELECT public.insert_candidate_universe_member(
             $1,member.instrument_id,member.announced_at,member.effective_from,
             $2,member.available_at,member.source_revision)
           FROM candidate_universe_members AS member
           JOIN candidate_universe_snapshots AS source
             ON source.id=member.universe_snapshot_id
          WHERE source.dataset_version_id=$3 LIMIT 1",
    )
    .bind(renewed_snapshot_id)
    .bind(as_of.as_naive_date())
    .bind(original_universe.dataset_version_id)
    .execute(&mut *universe_attack)
    .await
    .expect_err("same universe-member revision cannot change its effective window");
    assert_eq!(
        universe_conflict
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("23514")
    );
    universe_attack
        .rollback()
        .await
        .expect("rollback universe attack");

    let original_sector = bindings
        .iter()
        .find(|binding| binding.kind == market_data::ResponseKind::SectorClassification)
        .expect("original sector binding");
    let renewed_sector = renewed_bindings
        .iter()
        .find(|binding| binding.kind == market_data::ResponseKind::SectorClassification)
        .expect("renewed sector binding");
    let mut sector_attack = db.writer.begin().await.expect("sector attack tx");
    sqlx::query("SELECT set_config('app.candidate_raw_batch_id',$1,true)")
        .bind(renewed_outcome.batch_id.to_string())
        .execute(&mut *sector_attack)
        .await
        .expect("scope sector attack");
    let renewed_sector_version: Uuid = sqlx::query_scalar(
        "SELECT public.insert_candidate_sector_version(
             source.taxonomy_id,source.taxonomy_version,source.effective_from,
             source.available_at,$1,source.provider,$2,$3,$4,
             source.source_revision,$5,$6)
           FROM candidate_sector_versions AS source
          WHERE source.dataset_version_id=$7 LIMIT 1",
    )
    .bind(retrieved_at.as_datetime())
    .bind(renewed_sector.entitlement_id)
    .bind(as_of.as_naive_date())
    .bind(renewed_contract)
    .bind(renewed_sector.dataset_version_id)
    .bind(&renewed_sector.manifest_sha256)
    .bind(original_sector.dataset_version_id)
    .fetch_one(&mut *sector_attack)
    .await
    .expect("identical sector version may be reacquired under renewed rights");
    let sector_conflict = sqlx::query(
        "SELECT public.insert_candidate_sector_entry(
             $1,entry.instrument_id,entry.sector_code,entry.sector_name || ' conflict',
             entry.fundamental_profile,entry.effective_from,entry.effective_until,
             entry.available_at,entry.source_revision)
           FROM candidate_sector_entries AS entry
           JOIN candidate_sector_versions AS source
             ON source.id=entry.sector_version_id
          WHERE source.dataset_version_id=$2 LIMIT 1",
    )
    .bind(renewed_sector_version)
    .bind(original_sector.dataset_version_id)
    .execute(&mut *sector_attack)
    .await
    .expect_err("same sector-entry revision cannot change classification content");
    assert_eq!(
        sector_conflict
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("23514")
    );
    sector_attack
        .rollback()
        .await
        .expect("rollback sector attack");

    assert_eq!(
        publish_candidate_batch(&sink, &renewed_batch)
            .await
            .expect("renewed candidate publication"),
        PublishOutcome::Published
    );
    let short_contract = "fixture://candidate-license-short-window";
    sqlx::query(
        "INSERT INTO data_entitlements
         (contract_document_sha256, contract_reference, status, covered_datasets,
          covered_uses, effective_from, effective_until, managed_by)
         SELECT repeat('b',64),$1,'ACTIVE',covered_datasets,covered_uses,
                $2,effective_until,managed_by
           FROM data_entitlements WHERE contract_reference=$3",
    )
    .bind(short_contract)
    .bind(as_of.as_naive_date())
    .bind(contract_reference)
    .execute(&db.supervisor)
    .await
    .expect("short-window candidate entitlement");
    let short_outcome = ingest_bundle_with_kinds(
        &raw,
        &provider,
        &IngestRequest::new(MARKET_KR.to_owned(), as_of, retrieved_at),
        Some(short_contract),
        &CANDIDATE_RESPONSE_KINDS,
    )
    .expect("rolling flow under insufficient historical rights");
    assert_eq!(
        recover_candidate_batches(&raw, &sink)
            .await
            .expect("insufficient rolling rights become a durable terminal record"),
        0
    );
    let short_state: String = sqlx::query_scalar(
        "SELECT state FROM candidate_raw_batch_publications
          WHERE batch_id=$1 AND surface='source'",
    )
    .bind(short_outcome.batch_id.as_uuid())
    .fetch_one(&db.supervisor)
    .await
    .expect("short rights durable state");
    assert_eq!(short_state, "BLOCKED");
    let short_price_outcome = ingest_bundle(
        &price_raw,
        &price_provider,
        &IngestRequest::new(MARKET_KR.to_owned(), price_date, price_retrieved),
        Some(short_contract),
    )
    .expect("price Raw delivery under insufficient historical rights");
    sink.block_raw_batch_for_inactive_rights(
        &short_price_outcome.entry,
        "price",
        evidence.first_session,
        evidence.last_session,
    )
    .await
    .expect("price historical rights failure becomes durable terminal state");
    let short_price_state: String = sqlx::query_scalar(
        "SELECT state FROM candidate_raw_batch_publications
          WHERE batch_id=$1 AND surface='price'",
    )
    .bind(short_price_outcome.batch_id.as_uuid())
    .fetch_one(&db.supervisor)
    .await
    .expect("short price rights durable state");
    assert_eq!(short_price_state, "BLOCKED");
    let mut tampered_terminal = outcome.entry.clone();
    tampered_terminal.mode = FetchMode::Credentialed;
    assert!(matches!(
        sink.raw_batch_is_terminal(&tampered_terminal, "source")
            .await,
        Err(collectors::SinkError::Conflict(_))
    ));
    for omitted in CANDIDATE_RESPONSE_KINDS {
        let incomplete_batch_id = uuid::Uuid::new_v4();
        let incomplete_hash = format!("{:064x}", omitted as u8 + 1);
        sqlx::query("SELECT public.begin_candidate_raw_batch($1,'source',$2,'synthetic',$3,$4)")
            .bind(incomplete_batch_id)
            .bind(&incomplete_hash)
            .bind(contract_reference)
            .bind(as_of.as_naive_date())
            .execute(&db.writer)
            .await
            .expect("begin incomplete source ledger");
        for binding in bindings.iter().filter(|binding| {
            binding.kind != omitted
                && matches!(
                    binding.kind,
                    market_data::ResponseKind::Fundamentals
                        | market_data::ResponseKind::IndexMembership
                        | market_data::ResponseKind::SectorClassification
                )
        }) {
            sqlx::query("SELECT public.bind_candidate_raw_dataset($1,'source',$2,$3,true)")
                .bind(incomplete_batch_id)
                .bind(binding.kind.as_str())
                .bind(binding.dataset_version_id)
                .execute(&db.writer)
                .await
                .expect("bind incomplete source dataset");
        }
        let incomplete =
            sqlx::query("SELECT public.seal_candidate_raw_batch($1,'source',$2,'synthetic')")
                .bind(incomplete_batch_id)
                .bind(&incomplete_hash)
                .execute(&db.writer)
                .await
                .expect_err("every one of the five typed source kinds is seal-mandatory");
        assert_eq!(
            incomplete
                .as_database_error()
                .and_then(|error| error.code())
                .as_deref(),
            Some("23514")
        );
    }
    let sealed_append = sqlx::query(
        "INSERT INTO candidate_investor_flows
         (instrument_id,trade_date,investor_class,net_amount,net_volume,currency,volume_unit,
          provider,source_revision,available_at)
         SELECT instrument_id,trade_date,investor_class,net_amount,net_volume,currency,volume_unit,
                provider,'forged-after-seal',available_at
           FROM candidate_investor_flows LIMIT 1",
    )
    .execute(&db.writer)
    .await
    .expect_err("sealed candidate Raw batch rejects later source-row append");
    assert_eq!(
        sealed_append
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("42501")
    );
    let blocked_outcome = ingest_bundle_with_kinds(
        &raw,
        &provider,
        &IngestRequest::new(MARKET_KR.to_owned(), as_of, retrieved_at),
        Some(contract_reference),
        &CANDIDATE_RESPONSE_KINDS,
    )
    .expect("catalog-only crash fixture Raw batch");
    sink.catalog_candidate_batch(&blocked_outcome)
        .await
        .expect("catalog-only fixture");
    sqlx::query("UPDATE data_entitlements SET status='REVOKED' WHERE contract_reference=$1")
        .bind(contract_reference)
        .execute(&db.supervisor)
        .await
        .expect("revoke historical source entitlement");
    assert_eq!(
        recover_candidate_batches(&raw, &sink)
            .await
            .expect("revoked historical catalog is terminally recorded, not fatal"),
        0
    );
    let blocked_state: String = sqlx::query_scalar(
        "SELECT state FROM candidate_raw_batch_publications
          WHERE batch_id=$1 AND surface='source'",
    )
    .bind(blocked_outcome.batch_id.as_uuid())
    .fetch_one(&db.supervisor)
    .await
    .expect("durable recovery state");
    assert_eq!(blocked_state, "BLOCKED");
    sqlx::query("UPDATE data_entitlements SET status='ACTIVE' WHERE contract_reference=$1")
        .bind(contract_reference)
        .execute(&db.supervisor)
        .await
        .expect("restore active source entitlement");
    let price_version = curated_outcome.manifest.version.to_string();
    let price_publication = CandidatePricePublication {
        raw_batch_id: price_raw_outcome.batch_id.as_uuid(),
        raw_manifest_sha256: &collectors::candidate_raw_manifest_sha256(&price_raw_outcome.entry)
            .expect("price Raw manifest digest"),
        fetch_mode: price_raw_outcome.entry.mode,
        entitlement_date: price_raw_outcome.entry.date,
        evidence: &evidence,
        dataset_version: &price_version,
        storage_path: price_root.path().to_str().expect("UTF-8 price root"),
        provider: "synthetic",
        entitlement_id,
        license_ref: contract_reference,
        available_at: retrieved_at,
        retrieved_at,
    };
    let invalid_price_date = CandidatePricePublication {
        raw_batch_id: price_raw_outcome.batch_id.as_uuid(),
        raw_manifest_sha256: &collectors::candidate_raw_manifest_sha256(&price_raw_outcome.entry)
            .expect("price Raw manifest digest"),
        fetch_mode: price_raw_outcome.entry.mode,
        entitlement_date: TradingDate::parse("2026-08-17").expect("future Raw date"),
        evidence: &evidence,
        dataset_version: &price_version,
        storage_path: price_root.path().to_str().expect("UTF-8 price root"),
        provider: "synthetic",
        entitlement_id,
        license_ref: contract_reference,
        available_at: retrieved_at,
        retrieved_at,
    };
    let invalid_price_error = sink
        .publish_price(&invalid_price_date)
        .await
        .expect_err("DB publisher rejects a Raw date beyond its last price session");
    match invalid_price_error {
        collectors::SinkError::PermanentDatabase(error) => assert_eq!(
            error
                .as_database_error()
                .and_then(|database| database.code())
                .as_deref(),
            Some("23514")
        ),
        other => panic!("unexpected future-price error: {other:?}"),
    }
    let first = sink
        .publish_price(&price_publication)
        .await
        .expect("price catalog publication");
    assert_eq!(first.1, PublishOutcome::Published);
    let replay = sink
        .publish_price(&price_publication)
        .await
        .expect("price exact replay");
    assert_eq!(replay, (first.0, PublishOutcome::AlreadyPublished));
    assert!(sink.has_price(as_of).await.expect("price readiness"));
    for session in &health_sessions {
        sqlx::query(
            "INSERT INTO trading_calendars
             (exchange,session_date,session_type,timezone,source,source_version,
              source_batch_id,content_sha256,retrieved_at)
             VALUES ('KRX',$1,'TRADING','Asia/Seoul','synthetic',
                     'candidate-health-v1',$2,repeat('9',64),$3)",
        )
        .bind(session.as_naive_date())
        .bind(uuid::Uuid::new_v4())
        .bind(retrieved_at.as_datetime())
        .execute(&db.supervisor)
        .await
        .expect("confirmed KRX coverage session");
    }
    sqlx::query(
        "INSERT INTO data_batches
         (provider,market,batch_date,kind,storage_path,content_sha256,bytes_size,retrieved_at,
          source_batch_id,source_file_name,fetch_mode)
         VALUES ('KRX','KR',$1,'EOD','raw/candidate-health-v1',repeat('7',64),1,$2,
                 $3,'bars.json','synthetic')",
    )
    .bind(as_of.as_naive_date())
    .bind(retrieved_at.as_datetime())
    .bind(uuid::Uuid::new_v4())
    .execute(&db.supervisor)
    .await
    .expect("confirmed prior EOD publication");
    assert!(
        !PostgresPublicationSink::new(db.writer.clone())
            .has_eod_for_mode(as_of, FetchMode::Credentialed)
            .await
            .expect("fetch-mode-aware EOD discovery")
    );
    let now = retrieved_at.as_datetime() + chrono::Duration::hours(1);
    candidate_healthcheck(
        &db.writer,
        price_root.path(),
        now,
        std::time::Duration::from_secs(4 * 24 * 60 * 60),
        FetchMode::Synthetic,
        NaiveTime::from_hms_opt(16, 30, 0).unwrap(),
    )
    .await
    .expect("coherent same-session sources and exact disk manifest are healthy");
    let next_session = TradingDate::parse("2026-08-17").expect("next trading session");
    let pre_close = DateTime::parse_from_rfc3339("2026-08-17T01:00:00Z")
        .expect("pre-close instant")
        .with_timezone(&Utc);
    sqlx::query(
        "INSERT INTO trading_calendars
         (exchange,session_date,session_type,timezone,source,source_version,
          source_batch_id,content_sha256,retrieved_at)
         VALUES ('KRX',$1,'TRADING','Asia/Seoul','synthetic','candidate-health-v2',
                 $2,repeat('8',64),$3)",
    )
    .bind(next_session.as_naive_date())
    .bind(uuid::Uuid::new_v4())
    .bind(pre_close)
    .execute(&db.supervisor)
    .await
    .expect("pre-announced current KRX session");
    candidate_healthcheck(
        &db.writer,
        price_root.path(),
        pre_close,
        std::time::Duration::from_secs(4 * 24 * 60 * 60),
        FetchMode::Synthetic,
        NaiveTime::from_hms_opt(16, 30, 0).unwrap(),
    )
    .await
    .expect("pre-close calendar must retain the prior confirmed EOD session");
    let post_close = DateTime::parse_from_rfc3339("2026-08-17T08:00:00Z")
        .expect("post-close instant")
        .with_timezone(&Utc);
    let missing_eod = candidate_healthcheck(
        &db.writer,
        price_root.path(),
        post_close,
        std::time::Duration::from_secs(4 * 24 * 60 * 60),
        FetchMode::Synthetic,
        NaiveTime::from_hms_opt(16, 30, 0).unwrap(),
    )
    .await
    .expect_err("post-close current session without EOD must fail closed");
    assert!(matches!(
        missing_eod,
        WorkerError::Unhealthy {
            reason: HealthFailure::NoEodPublication
        }
    ));
    sqlx::query(
        "INSERT INTO data_batches
         (provider,market,batch_date,kind,storage_path,content_sha256,bytes_size,retrieved_at,
          source_batch_id,source_file_name,fetch_mode)
         VALUES ('KRX','KR',$1,'EOD','raw/candidate-health-v2',repeat('6',64),1,$2,
                 $3,'bars.json','synthetic')",
    )
    .bind(next_session.as_naive_date())
    .bind(post_close)
    .bind(uuid::Uuid::new_v4())
    .execute(&db.supervisor)
    .await
    .expect("current EOD publication without candidate sources");
    let current_missing = candidate_healthcheck(
        &db.writer,
        price_root.path(),
        post_close,
        std::time::Duration::from_secs(4 * 24 * 60 * 60),
        FetchMode::Synthetic,
        NaiveTime::from_hms_opt(16, 30, 0).unwrap(),
    )
    .await
    .expect_err("a confirmed current EOD must not fall back to stale candidate sources");
    assert!(matches!(
        current_missing,
        WorkerError::Unhealthy {
            reason: HealthFailure::NoCandidatePublication
        }
    ));
    let wrong_root = tempfile::tempdir().expect("wrong curated root");
    let mismatch = candidate_healthcheck(
        &db.writer,
        wrong_root.path(),
        now,
        std::time::Duration::from_secs(4 * 24 * 60 * 60),
        FetchMode::Synthetic,
        NaiveTime::from_hms_opt(16, 30, 0).unwrap(),
    )
    .await
    .expect_err("DB-only price readiness must fail without its exact disk manifest");
    assert!(matches!(
        mismatch,
        WorkerError::Unhealthy {
            reason: HealthFailure::PriceManifestMismatch
        }
    ));
    db.drop_db().await;
}
