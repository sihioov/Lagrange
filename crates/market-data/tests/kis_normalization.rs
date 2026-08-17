use domain::{BatchId, DatasetId, TradingDate, UtcTimestamp};
use market_data::contract::{
    FetchMode, MARKET_KR, PROVIDER_KIS, PROVIDER_KIS_NORMALIZED, RawEnvelope, RequestMetadata,
    ResponseKind,
};
use market_data::normalize::{
    NormalizeError, deterministic_kis_normalized_batch_id, normalize_kis_batch,
    normalize_kis_envelopes,
};
use market_data::providers::kis::KR_ETF_CORE_SYMBOLS;
use market_data::storage::{BatchSpec, RawStore};
use market_data::validate::validate_response;
use market_data::{CurateRequest, CurateStore, curate_batch, curation_inputs_from_raw};
use serde_json::{Value, json};
use std::sync::{Arc, Barrier};
use tempfile::TempDir;

const TARGET_DATE: &str = "2026-08-14";
const RETRIEVED_AT: &str = "2026-08-14T08:00:00Z";

#[derive(Debug)]
struct Wire {
    kind: ResponseKind,
    file_name: String,
    endpoint: String,
    query: Vec<(String, String)>,
    bytes: Vec<u8>,
}

fn fixture(
    mutate: impl FnOnce(&mut Vec<Wire>),
) -> (
    TempDir,
    RawStore,
    market_data::ManifestEntry,
    Vec<market_data::StoredFile>,
) {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RawStore::new(temp.path().join("data"));
    let mut wires = valid_wires();
    mutate(&mut wires);
    let batch_id = BatchId::generate();
    let now = UtcTimestamp::parse_rfc3339(RETRIEVED_AT).expect("timestamp");
    let envelopes = wires
        .iter()
        .map(|wire| {
            RawEnvelope::new(
                batch_id,
                wire.kind,
                wire.file_name.clone(),
                wire.bytes.clone(),
                now,
                RequestMetadata {
                    endpoint: wire.endpoint.clone(),
                    query: wire.query.clone(),
                    headers: vec![("authorization".into(), "[REDACTED]".into())],
                    mode: FetchMode::Credentialed,
                },
            )
        })
        .collect::<Vec<_>>();
    let date = TradingDate::parse(TARGET_DATE).expect("date");
    let entry = store
        .store_batch(
            &BatchSpec {
                provider: PROVIDER_KIS,
                market: MARKET_KR,
                date: &date,
                batch_id,
                entitlement_reference: None,
                mode: FetchMode::Credentialed,
            },
            &envelopes,
        )
        .expect("wire batch");
    let stored = store
        .read_batch_bytes(PROVIDER_KIS, MARKET_KR, &entry)
        .expect("wire readback");
    (temp, store, entry, stored)
}

fn valid_wires() -> Vec<Wire> {
    let mut wires = Vec::new();
    for symbol in KR_ETF_CORE_SYMBOLS {
        let bar = json!({
            "rt_cd": "0",
            "msg_cd": "MCA00000",
            "msg1": "",
            "output1": {},
            "output2": [
                {
                    "stck_bsop_date": "20260813",
                    "stck_oprc": "99.00",
                    "stck_hgpr": "101.00",
                    "stck_lwpr": "98.00",
                    "stck_clpr": "100.00",
                    "acml_vol": "1200",
                    "acml_tr_pbmn": "120000"
                },
                {
                    "stck_bsop_date": "20260814",
                    "stck_oprc": "100.00",
                    "stck_hgpr": "102.00",
                    "stck_lwpr": "99.00",
                    "stck_clpr": "101.00",
                    "acml_vol": "1300",
                    "acml_tr_pbmn": "131300"
                }
            ]
        });
        wires.push(Wire {
            kind: ResponseKind::Bars,
            file_name: format!("daily-bars-{symbol}-page-01.json"),
            endpoint: "/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice".into(),
            query: vec![("FID_INPUT_ISCD".into(), symbol.into())],
            bytes: serde_json::to_vec(&bar).expect("bars"),
        });
        let reference = json!({
            "rt_cd": "0",
            "output": {
                "std_pdno": symbol,
                "prdt_name": format!("ETF {symbol}"),
                "stck_shrn_iscd": symbol
            }
        });
        wires.push(Wire {
            kind: ResponseKind::Reference,
            file_name: format!("reference-{symbol}-page-01.json"),
            endpoint: "/uapi/domestic-stock/v1/quotations/inquire-price".into(),
            query: vec![("FID_INPUT_ISCD".into(), symbol.into())],
            bytes: serde_json::to_vec(&reference).expect("reference"),
        });
    }
    wires.push(Wire {
        kind: ResponseKind::Calendar,
        file_name: "calendar-page-01.json".into(),
        endpoint: "/uapi/domestic-stock/v1/quotations/chk-holiday".into(),
        query: vec![("BASS_DT".into(), "20260814".into())],
        bytes: serde_json::to_vec(&json!({
            "rt_cd": "0",
            "output": [
                {"bass_dt": "20260814", "opnd_yn": "Y"},
                {"bass_dt": "20260815", "opnd_yn": "N"}
            ]
        }))
        .expect("calendar"),
    });
    for (label, endpoint) in [
        (
            "paidin-subscription",
            "/uapi/domestic-stock/v1/ksdinfo/paidin-capin",
        ),
        (
            "paidin-record",
            "/uapi/domestic-stock/v1/ksdinfo/paidin-capin",
        ),
        ("bonus", "/uapi/domestic-stock/v1/ksdinfo/bonus-issue"),
        ("dividend", "/uapi/domestic-stock/v1/ksdinfo/dividend"),
        (
            "merger-split",
            "/uapi/domestic-stock/v1/ksdinfo/merger-split",
        ),
        ("reverse-split", "/uapi/domestic-stock/v1/ksdinfo/rev-split"),
        (
            "capital-decrease",
            "/uapi/domestic-stock/v1/ksdinfo/cap-dcrs",
        ),
    ] {
        wires.push(Wire {
            kind: ResponseKind::CorporateActions,
            file_name: format!("corporate-actions-{label}-page-01.json"),
            endpoint: endpoint.into(),
            query: vec![
                ("F_DT".into(), "20260814".into()),
                ("T_DT".into(), "20260814".into()),
            ],
            bytes: br#"{"rt_cd":"0","output1":[]}"#.to_vec(),
        });
    }
    wires
}

fn mutate_first_bar(wires: &mut [Wire], mutate: impl FnOnce(&mut Vec<Value>)) {
    let wire = wires
        .iter_mut()
        .find(|wire| wire.kind == ResponseKind::Bars)
        .expect("bar wire");
    let mut document: Value = serde_json::from_slice(&wire.bytes).expect("bar json");
    let rows = document["output2"].as_array_mut().expect("rows");
    mutate(rows);
    wire.bytes = serde_json::to_vec(&document).expect("bar bytes");
}

#[test]
fn successful_normalization_is_four_canonical_documents_and_preserves_wire_bytes() {
    let (temp, store, source, before) = fixture(|_| {});
    let envelopes = normalize_kis_envelopes(
        &source,
        &store
            .read_batch_bytes(PROVIDER_KIS, MARKET_KR, &source)
            .unwrap(),
    )
    .expect("normalize envelopes");
    assert_eq!(envelopes.len(), 4);
    assert!(
        envelopes
            .iter()
            .all(|envelope| envelope.batch_id == envelopes[0].batch_id)
    );
    let outcome = normalize_kis_batch(&store, &source).expect("normalize");
    assert_eq!(outcome.entry.provider, PROVIDER_KIS_NORMALIZED);
    assert_eq!(outcome.entry.files.len(), 4);
    assert_eq!(outcome.files.len(), 4);
    assert_eq!(
        outcome
            .entry
            .files
            .iter()
            .map(|file| file.kind)
            .collect::<Vec<_>>(),
        vec![
            ResponseKind::Bars,
            ResponseKind::Reference,
            ResponseKind::Calendar,
            ResponseKind::CorporateActions
        ]
    );
    for file in &outcome.files {
        validate_response(file_kind(&outcome.entry, &file.file_name), &file.bytes)
            .expect("canonical contract");
        let value: Value = serde_json::from_slice(&file.bytes).expect("canonical json");
        assert_eq!(
            value["_lineage"]["upstream_batch_id"],
            source.batch_id.to_string()
        );
        assert_eq!(
            value["_lineage"]["upstream_files"]
                .as_array()
                .unwrap()
                .len(),
            source.files.len()
        );
    }
    let bars = outcome
        .files
        .iter()
        .find(|file| file.file_name == "bars.json")
        .expect("bars");
    let bars: Value = serde_json::from_slice(&bars.bytes).expect("bars json");
    assert_eq!(bars["bars"].as_array().unwrap().len(), 11);
    assert!(
        bars["bars"]
            .as_array()
            .unwrap()
            .iter()
            .all(|bar| bar["date"] == TARGET_DATE)
    );
    let reference = outcome
        .files
        .iter()
        .find(|file| file.file_name == "reference.json")
        .expect("reference");
    let reference: Value = serde_json::from_slice(&reference.bytes).expect("reference json");
    assert_eq!(reference["instruments"].as_array().unwrap().len(), 11);
    assert_eq!(
        serde_json::from_slice::<Value>(&outcome.files[2].bytes).unwrap()["sessions"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store
            .read_batch_bytes(PROVIDER_KIS, MARKET_KR, &source)
            .unwrap(),
        before,
        "wire bytes and hashes remain untouched"
    );

    let (calendar, master) = curation_inputs_from_raw(&store, &outcome.entry)
        .expect("canonical reference and calendar are curation inputs");
    let dataset_id = DatasetId::parse("kis-normalized-test").expect("dataset id");
    let curated = CurateStore::new(temp.path().join("data"));
    let curated_outcome = curate_batch(
        &store,
        &outcome.entry,
        &calendar,
        &master,
        &curated,
        &CurateRequest {
            dataset_id: &dataset_id,
            market: MARKET_KR,
            source: PROVIDER_KIS_NORMALIZED,
            now: UtcTimestamp::parse_rfc3339("2026-08-14T09:00:00Z").unwrap(),
        },
    )
    .expect("canonical batch is curation-compatible");
    assert_eq!(curated_outcome.bars_written, 11);
}

#[test]
fn stored_normalization_is_idempotent_for_one_source_batch() {
    let (_temp, store, source, _stored) = fixture(|_| {});
    let first = normalize_kis_batch(&store, &source).expect("first normalization");
    let first_files = first.files.clone();
    let second = normalize_kis_batch(&store, &source).expect("idempotent retry");

    assert_eq!(
        first.entry.batch_id,
        deterministic_kis_normalized_batch_id(source.batch_id)
    );
    assert_eq!(first.entry, second.entry);
    assert_eq!(first_files, second.files);
    let manifest = store
        .read_manifest(PROVIDER_KIS_NORMALIZED, MARKET_KR)
        .expect("normalized manifest");
    assert_eq!(manifest, vec![first.entry]);
}

#[test]
fn concurrent_normalization_converges_to_one_batch() {
    let (_temp, store, source, _stored) = fixture(|_| {});
    let barrier = Arc::new(Barrier::new(4));
    let handles = (0..4)
        .map(|_| {
            let barrier = Arc::clone(&barrier);
            let store = store.clone();
            let source = source.clone();
            std::thread::spawn(move || {
                barrier.wait();
                normalize_kis_batch(&store, &source)
            })
        })
        .collect::<Vec<_>>();
    let outcomes = handles
        .into_iter()
        .map(|handle| {
            handle
                .join()
                .expect("normalizer thread")
                .expect("concurrent normalization")
        })
        .collect::<Vec<_>>();

    let first = &outcomes[0];
    assert_eq!(
        first.entry.batch_id,
        deterministic_kis_normalized_batch_id(source.batch_id)
    );
    for outcome in &outcomes[1..] {
        assert_eq!(outcome.entry, first.entry);
        assert_eq!(outcome.files, first.files);
    }
    assert_eq!(
        store
            .read_manifest(PROVIDER_KIS_NORMALIZED, MARKET_KR)
            .expect("normalized manifest")
            .len(),
        1
    );
}

#[test]
fn existing_deterministic_batch_conflict_fails_closed() {
    let (_temp, store, source, _stored) = fixture(|_| {});
    let batch_id = deterministic_kis_normalized_batch_id(source.batch_id);
    let wrong = RawEnvelope::new(
        batch_id,
        ResponseKind::Bars,
        "bars.json",
        b"not the canonical bars document".to_vec(),
        source.retrieved_at,
        RequestMetadata {
            endpoint: "kis.normalized/kis-wire-to-canonical-v1/bars".to_owned(),
            query: Vec::new(),
            headers: Vec::new(),
            mode: FetchMode::Credentialed,
        },
    );
    store
        .store_batch(
            &BatchSpec {
                provider: PROVIDER_KIS_NORMALIZED,
                market: MARKET_KR,
                date: &source.date,
                batch_id,
                entitlement_reference: source.entitlement_reference.as_deref(),
                mode: FetchMode::Credentialed,
            },
            &[wrong],
        )
        .expect("conflicting deterministic batch fixture");

    let error = normalize_kis_batch(&store, &source).expect_err("conflicting batch");
    assert!(matches!(
        error,
        NormalizeError::ExistingBatchConflict {
            batch_id: actual,
            ..
        } if actual == batch_id
    ));
}

#[test]
fn malformed_official_field_fails_closed() {
    let (_temp, _store, source, stored) = fixture(|wires| {
        mutate_first_bar(wires, |rows| {
            rows[1].as_object_mut().unwrap().remove("stck_clpr");
        });
    });
    let error = normalize_kis_envelopes(&source, &stored).expect_err("missing close");
    assert!(matches!(error, NormalizeError::MissingField { field, .. } if field == "stck_clpr"));
}

#[test]
fn duplicate_and_conflicting_rows_are_rejected() {
    let (_temp, _store, source, stored) = fixture(|wires| {
        mutate_first_bar(wires, |rows| rows.push(rows[1].clone()));
    });
    let duplicate = normalize_kis_envelopes(&source, &stored).expect_err("duplicate");
    assert!(matches!(
        duplicate,
        NormalizeError::DuplicateRow {
            kind: ResponseKind::Bars,
            ..
        }
    ));

    let (_temp, _store, source, stored) = fixture(|wires| {
        mutate_first_bar(wires, |rows| {
            rows.push(rows[1].clone());
            rows.last_mut().unwrap()["stck_clpr"] = json!("102.00");
        });
    });
    let conflict = normalize_kis_envelopes(&source, &stored).expect_err("conflict");
    assert!(matches!(
        conflict,
        NormalizeError::ConflictingRow {
            kind: ResponseKind::Bars,
            ..
        }
    ));
}

#[test]
fn nonempty_corporate_actions_fail_closed_without_inventing_dates_or_values() {
    let (_temp, _store, source, stored) = fixture(|wires| {
        let wire = wires
            .iter_mut()
            .find(|wire| wire.file_name.contains("dividend"))
            .expect("dividend wire");
        wire.bytes = serde_json::to_vec(&json!({
            "rt_cd": "0",
            "output1": [{"sht_cd": "069500", "ex_date": "20260814", "divi_amt": "100"}]
        }))
        .expect("action bytes");
    });
    let error = normalize_kis_envelopes(&source, &stored).expect_err("unsupported action");
    assert!(matches!(error, NormalizeError::UnsupportedAction { .. }));
}

#[test]
fn evidence_must_match_manifest_exactly() {
    let (_temp, _store, source, stored) = fixture(|_| {});
    let missing = &stored[..stored.len() - 1];
    let error = normalize_kis_envelopes(&source, missing).expect_err("missing evidence");
    assert!(matches!(error, NormalizeError::EvidenceMissing { .. }));

    let mut extra = stored.clone();
    let mut unmanifested = extra[0].clone();
    unmanifested.file_name = "unmanifested.json".to_owned();
    extra.push(unmanifested);
    let error = normalize_kis_envelopes(&source, &extra).expect_err("extra evidence");
    assert!(matches!(error, NormalizeError::EvidenceUnexpected { .. }));
}

#[test]
fn evidence_bytes_must_match_manifest_hash() {
    let (_temp, _store, source, mut stored) = fixture(|_| {});
    stored[0].bytes[0] ^= 1;
    let error = normalize_kis_envelopes(&source, &stored).expect_err("tampered evidence");
    assert!(matches!(error, NormalizeError::EvidenceHashMismatch { .. }));
}

#[test]
fn evidence_bytes_must_match_manifest_size() {
    let (_temp, _store, mut source, stored) = fixture(|_| {});
    source.files[0].size_bytes += 1;
    let error = normalize_kis_envelopes(&source, &stored).expect_err("wrong manifest size");
    assert!(matches!(error, NormalizeError::EvidenceSizeMismatch { .. }));
}

#[test]
fn calendar_requires_one_target_observation() {
    let (_temp, _store, source, stored) = fixture(|wires| {
        let wire = wires
            .iter_mut()
            .find(|wire| wire.kind == ResponseKind::Calendar)
            .expect("calendar wire");
        wire.bytes = serde_json::to_vec(&json!({
            "rt_cd": "0",
            "output": [{"bass_dt": "20260815", "opnd_yn": "N"}]
        }))
        .expect("calendar bytes");
    });
    let error = normalize_kis_envelopes(&source, &stored).expect_err("missing target date");
    assert!(matches!(
        error,
        NormalizeError::MissingTargetObservation { .. }
    ));
}

#[test]
fn corporate_action_endpoint_must_be_one_of_the_reviewed_ksd_paths() {
    let (_temp, _store, source, stored) = fixture(|wires| {
        let wire = wires
            .iter_mut()
            .find(|wire| wire.kind == ResponseKind::CorporateActions)
            .expect("corporate action wire");
        wire.endpoint = "/uapi/domestic-stock/v1/ksdinfo/not-reviewed".to_owned();
    });
    let error = normalize_kis_envelopes(&source, &stored).expect_err("unknown endpoint");
    assert!(matches!(error, NormalizeError::UnexpectedEndpoint { .. }));
}

#[test]
fn open_target_day_requires_one_bar_for_each_fixed_symbol() {
    let (_temp, _store, source, stored) = fixture(|wires| {
        mutate_first_bar(wires, |rows| {
            rows.retain(|row| row["stck_bsop_date"] != "20260814");
        });
    });
    let error = normalize_kis_envelopes(&source, &stored).expect_err("missing target bar");
    assert!(matches!(error, NormalizeError::TargetBarCoverage { .. }));
}

fn file_kind(entry: &market_data::ManifestEntry, file_name: &str) -> ResponseKind {
    entry
        .files
        .iter()
        .find(|file| file.file_name == file_name)
        .expect("file metadata")
        .kind
}
