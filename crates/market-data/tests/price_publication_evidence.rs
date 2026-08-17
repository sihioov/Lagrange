use domain::{DatasetId, TradingDate, UtcTimestamp};
use market_data::{
    CurateRequest, CurateStore, IngestRequest, KrxProvider, MARKET_KR, RawStore, RecordedBundle,
    curate_batch, curation_inputs_from_raw, ingest_bundle, price_curation_evidence,
};

#[test]
fn raw_delivery_curates_and_rebuilds_exact_price_publication_evidence() {
    let root = tempfile::tempdir().expect("data root");
    let raw = RawStore::new(root.path());
    let provider = KrxProvider::synthetic(
        RecordedBundle::open(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/kr-etf/contract"
        ))
        .expect("fixture bundle"),
    );
    let target = TradingDate::parse("2020-01-31").expect("target date");
    let retrieved_at =
        UtcTimestamp::parse_rfc3339("2020-01-31T07:00:00Z").expect("retrieval instant");
    let outcome = ingest_bundle(
        &raw,
        &provider,
        &IngestRequest::new(MARKET_KR.to_owned(), target, retrieved_at),
        Some("fixture://candidate-license"),
    )
    .expect("immutable Raw delivery");
    let (calendar, master) =
        curation_inputs_from_raw(&raw, &outcome.entry).expect("typed Raw curation inputs");
    let dataset_id = DatasetId::parse("krx_eod_bars").expect("dataset id");
    let curated = CurateStore::new(root.path());
    let result = curate_batch(
        &raw,
        &outcome.entry,
        &calendar,
        &master,
        &curated,
        &CurateRequest {
            dataset_id: &dataset_id,
            market: MARKET_KR,
            source: "synthetic",
            now: retrieved_at,
        },
    )
    .expect("curation");
    assert_eq!(result.first_session.to_iso(), "2020-01-30");
    assert_eq!(result.last_session.to_iso(), "2020-01-31");

    let recovered = curated
        .manifest_for_source_batch(&dataset_id, outcome.batch_id)
        .expect("manifest lookup")
        .expect("published generation");
    assert_eq!(recovered, result.manifest);
    let evidence = price_curation_evidence(&raw, &outcome.entry, &recovered)
        .expect("replayable publication evidence");
    assert_eq!(evidence.curated_generation, 1);
    assert_eq!(evidence.first_session, result.first_session);
    assert_eq!(evidence.last_session, result.last_session);
    assert_eq!(evidence.source_revision, outcome.batch_id.to_string());
    assert_eq!(evidence.manifest_sha256.len(), 64);
    assert_eq!(evidence.instrument_coverage.len(), 2);
    assert_eq!(evidence.instrument_coverage[0].instrument_id, "069500.KRX");
    assert_eq!(
        evidence.instrument_coverage[0].first_session,
        result.first_session
    );
    assert_eq!(
        evidence.instrument_coverage[0].last_session,
        result.last_session
    );
    assert_eq!(evidence.instrument_coverage[0].session_count, 2);
    assert_eq!(evidence.instrument_coverage[1].instrument_id, "229200.KRX");
    assert_eq!(evidence.instrument_coverage[1].session_count, 2);
}
