use domain::{BatchId, ContentHash, DatasetId, TradingDate, UtcTimestamp};
use market_data::contract::{FetchMode, RequestMetadata, ResponseKind};
use market_data::{
    BatchSpec, CurateRequest, CurateStore, IngestRequest, KrxProvider, MARKET_KR, RawEnvelope,
    RawStore, RecordedBundle, curate_batch, curate_generation, curation_inputs_from_raw,
    curation_inputs_from_raw_entries, ingest_bundle, price_curation_evidence,
    price_curation_evidence_for_generation,
};
use serde_json::Value;

fn fixture_entry(
    raw: &RawStore,
    date: TradingDate,
    retrieved_at: UtcTimestamp,
    bars: Vec<u8>,
) -> market_data::ManifestEntry {
    let batch_id = BatchId::generate();
    let request = RequestMetadata {
        endpoint: "krx.eod.bars.v1".to_owned(),
        query: Vec::new(),
        headers: Vec::new(),
        mode: FetchMode::Synthetic,
    };
    raw.store_batch(
        &BatchSpec {
            provider: "krx",
            market: MARKET_KR,
            date: &date,
            batch_id,
            entitlement_reference: Some("fixture://candidate-license"),
            mode: FetchMode::Synthetic,
        },
        &[
            RawEnvelope::new(
                batch_id,
                ResponseKind::Bars,
                "bars.json",
                bars,
                retrieved_at,
                request.clone(),
            ),
            RawEnvelope::new(
                batch_id,
                ResponseKind::Reference,
                "reference.json",
                include_bytes!("../../../tests/fixtures/kr-etf/contract/reference-response.json")
                    .to_vec(),
                retrieved_at,
                request.clone(),
            ),
            RawEnvelope::new(
                batch_id,
                ResponseKind::Calendar,
                "calendar.json",
                include_bytes!("../../../tests/fixtures/kr-etf/2020-01-31/calendar.json").to_vec(),
                retrieved_at,
                request.clone(),
            ),
            RawEnvelope::new(
                batch_id,
                ResponseKind::CorporateActions,
                "corporate-actions.json",
                include_bytes!(
                    "../../../tests/fixtures/kr-etf/contract/corporate-actions-response.json"
                )
                .to_vec(),
                retrieved_at,
                request,
            ),
        ],
    )
    .expect("immutable fixture Raw delivery")
}

fn artifact_fixture() -> (tempfile::TempDir, CurateStore, market_data::DatasetManifest) {
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
    (root, curated, result.manifest)
}

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

#[test]
fn cumulative_generation_covers_multiple_raw_dates_for_one_pin() {
    let root = tempfile::tempdir().expect("data root");
    let raw = RawStore::new(root.path());
    let first_date = TradingDate::parse("2020-01-31").expect("date");
    let last_date = TradingDate::parse("2020-02-03").expect("date");
    let first_at = UtcTimestamp::parse_rfc3339("2020-02-01T07:00:00Z").expect("timestamp");
    let last_at = UtcTimestamp::parse_rfc3339("2020-02-04T07:00:00Z").expect("timestamp");
    let mut first_bars: Value = serde_json::from_slice(include_bytes!(
        "../../../tests/fixtures/kr-etf/2020-01-31/bars.json"
    ))
    .expect("fixture bars");
    first_bars["bars"] = Value::Array(
        first_bars["bars"]
            .as_array()
            .expect("bars array")
            .iter()
            .filter(|row| {
                row["date"]
                    .as_str()
                    .is_some_and(|date| date <= "2020-01-31")
                    && matches!(
                        row["instrument"].as_str(),
                        Some("069500.KRX" | "229200.KRX")
                    )
            })
            .cloned()
            .collect(),
    );
    let first = fixture_entry(
        &raw,
        first_date,
        first_at,
        serde_json::to_vec(&first_bars).expect("first bars fixture"),
    );
    let mut all_bars: Value = serde_json::from_slice(include_bytes!(
        "../../../tests/fixtures/kr-etf/2020-01-31/bars.json"
    ))
    .expect("fixture bars");
    all_bars["bars"] = Value::Array(
        all_bars["bars"]
            .as_array()
            .expect("bars array")
            .iter()
            .filter(|row| {
                row["date"] == "2020-02-03"
                    && matches!(
                        row["instrument"].as_str(),
                        Some("069500.KRX" | "229200.KRX")
                    )
            })
            .cloned()
            .collect(),
    );
    let last = fixture_entry(
        &raw,
        last_date,
        last_at,
        serde_json::to_vec(&all_bars).expect("second bars fixture"),
    );
    let entries = vec![first.clone(), last.clone()];
    let (calendar, master) =
        curation_inputs_from_raw_entries(&raw, &entries).expect("merged curation inputs");
    let curated = CurateStore::new(root.path());
    let dataset_id = DatasetId::parse("krx_eod_bars").expect("dataset id");
    let outcome = curate_generation(
        &raw,
        &entries,
        &calendar,
        &master,
        &curated,
        &CurateRequest {
            dataset_id: &dataset_id,
            market: MARKET_KR,
            source: "krx",
            now: last_at,
        },
    )
    .expect("one cumulative generation");
    assert_eq!(outcome.dataset_version, 1);
    assert_eq!(outcome.manifest.source_batches.len(), 2);
    assert_eq!(outcome.manifest.bar_count, 18);
    assert_eq!(outcome.first_session.to_iso(), "2020-01-20");
    assert_eq!(outcome.last_session.to_iso(), "2020-02-03");

    let evidence = price_curation_evidence_for_generation(&raw, &entries, &outcome.manifest, &last)
        .expect("cumulative evidence");
    assert_eq!(evidence.first_session.to_iso(), "2020-01-20");
    assert_eq!(evidence.last_session.to_iso(), "2020-02-03");
    assert_eq!(evidence.source_revision, last.batch_id.to_string());
    assert_eq!(evidence.instrument_coverage.len(), 2);
    assert!(
        evidence
            .instrument_coverage
            .iter()
            .all(|coverage| coverage.last_session == last_date)
    );
    assert!(!outcome.manifest.artifacts.is_empty());
    curated
        .verify_artifacts(&outcome.manifest)
        .expect("exact cumulative artifacts");

    let artifact = outcome.manifest.artifacts[0].clone();
    let artifact_path = root.path().join("curated").join(&artifact.path);
    let original = std::fs::read(&artifact_path).expect("artifact bytes");
    std::fs::write(&artifact_path, b"tampered").expect("tamper fixture");
    assert!(curated.verify_artifacts(&outcome.manifest).is_err());
    std::fs::write(&artifact_path, original).expect("restore artifact");

    let extra_path = artifact_path
        .parent()
        .expect("artifact parent")
        .join("unexpected.parquet");
    std::fs::write(&extra_path, b"extra").expect("extra fixture");
    assert!(curated.verify_artifacts(&outcome.manifest).is_err());
    std::fs::remove_file(extra_path).expect("remove extra fixture");
}

#[test]
fn artifact_attestation_rejects_legacy_and_each_declared_integrity_gap() {
    let (_root, curated, manifest) = artifact_fixture();
    curated
        .verify_artifacts(&manifest)
        .expect("fixture artifact set is attested");

    let mut legacy = manifest.clone();
    legacy.artifacts.clear();
    assert!(curated.verify_artifacts(&legacy).is_err());

    let bars_index = manifest
        .artifacts
        .iter()
        .position(|artifact| artifact.schema == market_data::BARS_SCHEMA_ID)
        .expect("bars artifact");

    let mut missing = manifest.clone();
    missing.artifacts[bars_index].path =
        "bars/market=kr/symbol=missing/year=2020/version=1/bars.parquet".to_owned();
    assert!(curated.verify_artifacts(&missing).is_err());

    let mut unsafe_path = manifest.clone();
    unsafe_path.artifacts[bars_index].path = "../outside/bars.parquet".to_owned();
    assert!(curated.verify_artifacts(&unsafe_path).is_err());

    let mut schema_mismatch = manifest.clone();
    schema_mismatch.artifacts[bars_index].schema = market_data::ADJUSTED_BARS_SCHEMA_ID.to_owned();
    assert!(curated.verify_artifacts(&schema_mismatch).is_err());

    let mut size_mismatch = manifest.clone();
    size_mismatch.artifacts[bars_index].size_bytes = size_mismatch.artifacts[bars_index]
        .size_bytes
        .saturating_add(1);
    assert_ne!(
        market_data::dataset_manifest_hash(&manifest).expect("manifest hash"),
        market_data::dataset_manifest_hash(&size_mismatch).expect("changed manifest hash")
    );
    assert!(curated.verify_artifacts(&size_mismatch).is_err());

    let mut hash_mismatch = manifest;
    hash_mismatch.artifacts[bars_index].sha256 = ContentHash::from_bytes(b"wrong");
    assert!(curated.verify_artifacts(&hash_mismatch).is_err());
}

#[cfg(unix)]
#[test]
fn artifact_attestation_rejects_symlink_leaf_and_ancestor() {
    use std::os::unix::fs::symlink;

    let (root, curated, manifest) = artifact_fixture();
    let artifact = manifest
        .artifacts
        .iter()
        .find(|artifact| artifact.schema == market_data::BARS_SCHEMA_ID)
        .expect("bars artifact");
    let artifact_path = root.path().join("curated").join(&artifact.path);
    let original = std::fs::read(&artifact_path).expect("artifact bytes");
    let outside = root.path().join("outside.parquet");
    std::fs::write(&outside, &original).expect("outside bytes");

    std::fs::remove_file(&artifact_path).expect("remove artifact");
    symlink(&outside, &artifact_path).expect("leaf symlink");
    assert!(curated.verify_artifacts(&manifest).is_err());
    std::fs::remove_file(&artifact_path).expect("remove leaf symlink");
    std::fs::write(&artifact_path, &original).expect("restore artifact");

    let version_dir = artifact_path.parent().expect("version directory");
    let version_backup = root.path().join("version-directory-backup");
    std::fs::rename(version_dir, &version_backup).expect("move version directory");
    symlink(&version_backup, version_dir).expect("ancestor symlink");
    assert!(curated.verify_artifacts(&manifest).is_err());
    std::fs::remove_file(version_dir).expect("remove ancestor symlink");
    std::fs::rename(version_backup, version_dir).expect("restore version directory");
}

#[cfg(unix)]
#[test]
fn dataset_manifest_read_rejects_symlink_leaf_and_ancestor() {
    use std::os::unix::fs::symlink;

    let (root, curated, manifest) = artifact_fixture();
    let manifest_path = curated
        .dataset_dir(&manifest.dataset_id, manifest.version)
        .join("manifest.json");
    let original = std::fs::read(&manifest_path).expect("manifest bytes");
    let outside = root.path().join("outside-manifest.json");
    std::fs::write(&outside, &original).expect("outside manifest");

    std::fs::remove_file(&manifest_path).expect("remove manifest");
    symlink(&outside, &manifest_path).expect("manifest leaf symlink");
    assert!(
        curated
            .read_dataset_manifest(&manifest.dataset_id, manifest.version)
            .is_err()
    );
    std::fs::remove_file(&manifest_path).expect("remove manifest symlink");
    std::fs::write(&manifest_path, &original).expect("restore manifest");

    let dataset_dir = curated.dataset_dir(&manifest.dataset_id, manifest.version);
    let dataset_backup = root.path().join("dataset-directory-backup");
    std::fs::rename(&dataset_dir, &dataset_backup).expect("move dataset directory");
    symlink(&dataset_backup, &dataset_dir).expect("manifest ancestor symlink");
    assert!(
        curated
            .read_dataset_manifest(&manifest.dataset_id, manifest.version)
            .is_err()
    );
    std::fs::remove_file(&dataset_dir).expect("remove ancestor symlink");
    std::fs::rename(dataset_backup, dataset_dir).expect("restore dataset directory");
}

#[test]
fn legacy_manifest_deserializes_but_is_not_production_attestable() {
    let (_root, curated, manifest) = artifact_fixture();
    let mut json = serde_json::to_value(&manifest).expect("manifest json");
    json.as_object_mut()
        .expect("manifest object")
        .remove("artifacts");
    let decoded: market_data::DatasetManifest =
        serde_json::from_value(json).expect("legacy manifest remains readable");
    assert!(decoded.artifacts.is_empty());
    assert!(curated.verify_artifacts(&decoded).is_err());
}
