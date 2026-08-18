use domain::{BatchId, DatasetId, TradingDate, UtcTimestamp};
use market_data::contract::{
    FetchMode, MARKET_KR, PROVIDER_KIS, PROVIDER_KIS_NORMALIZED, RawEnvelope, RequestMetadata,
    ResponseKind,
};
use market_data::curate::read_corporate_actions;
use market_data::normalize::{
    NormalizeError, deterministic_kis_normalized_batch_id, normalize_kis_batch,
    normalize_kis_envelopes,
};
use market_data::providers::kis::KR_ETF_CORE_SYMBOLS;
use market_data::storage::{BatchSpec, RawStore};
use market_data::validate::validate_response;
use market_data::{
    CurateError, CurateRequest, CurateStore, curate_batch, curation_inputs_from_raw,
};
use serde_json::{Value, json};
use std::sync::{Arc, Barrier};
use tempfile::TempDir;

const TARGET_DATE: &str = "2026-08-14";
const RETRIEVED_AT: &str = "2026-08-14T08:00:00Z";

#[derive(Debug, Clone)]
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
            "output1": {
                "hts_kor_isnm": format!("ETF {symbol}"),
                "stck_shrn_iscd": symbol
            },
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
        let bytes = br#"{"rt_cd":"0","output1":[]}"#.to_vec();
        wires.push(Wire {
            kind: ResponseKind::CorporateActions,
            file_name: format!("corporate-actions-{label}-page-01.json"),
            endpoint: endpoint.into(),
            query: vec![
                ("F_DT".into(), "20260814".into()),
                ("T_DT".into(), "20260814".into()),
            ],
            bytes,
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
fn normalized_reference_must_be_exact_fixed_etf_universe_at_curation_boundary() {
    let (_source_temp, source_store, source, _stored) = fixture(|_| {});
    let normalized = normalize_kis_batch(&source_store, &source).expect("normalize source");

    for mutation in ["extra", "missing", "non-etf"] {
        let temp = tempfile::tempdir().expect("normalized fixture root");
        let store = RawStore::new(temp.path().join("data"));
        let envelopes = normalized
            .entry
            .files
            .iter()
            .map(|metadata| {
                let stored = normalized
                    .files
                    .iter()
                    .find(|file| file.file_name == metadata.file_name)
                    .expect("normalized evidence");
                let bytes = if metadata.kind == ResponseKind::Reference {
                    let mut document: Value =
                        serde_json::from_slice(&stored.bytes).expect("reference document");
                    let instruments = document["instruments"]
                        .as_array_mut()
                        .expect("reference instruments");
                    match mutation {
                        "extra" => instruments.push(json!({
                            "symbol": "000001.KRX",
                            "name": "extra ETF",
                            "lot_size": 1,
                            "currency": "KRW",
                            "kind": "equity-etf"
                        })),
                        "missing" => {
                            instruments.pop();
                        }
                        "non-etf" => instruments[0]["kind"] = Value::String("equity".into()),
                        _ => unreachable!("mutation is fixed above"),
                    }
                    serde_json::to_vec(&document).expect("mutated reference")
                } else {
                    stored.bytes.clone()
                };
                RawEnvelope::new(
                    normalized.entry.batch_id,
                    metadata.kind,
                    metadata.file_name.clone(),
                    bytes,
                    normalized.entry.retrieved_at,
                    metadata.request.clone(),
                )
            })
            .collect::<Vec<_>>();
        let entry = store
            .store_batch(
                &BatchSpec {
                    provider: PROVIDER_KIS_NORMALIZED,
                    market: MARKET_KR,
                    date: &normalized.entry.date,
                    batch_id: normalized.entry.batch_id,
                    entitlement_reference: normalized.entry.entitlement_reference.as_deref(),
                    mode: FetchMode::Credentialed,
                },
                &envelopes,
            )
            .expect("handcrafted canonical normalized batch");
        let error = curation_inputs_from_raw(&store, &entry)
            .expect_err("curation must enforce exact fixed ETF reference universe");
        assert!(
            matches!(error, CurateError::NonCanonicalNormalizedBatch { .. }),
            "{mutation}: {error}"
        );
    }
}

#[test]
fn reference_name_comes_only_from_matching_daily_bars_output1() {
    let (_temp, _store, source, stored) = fixture(|wires| {
        let wire = wires
            .iter_mut()
            .find(|wire| wire.kind == ResponseKind::Reference)
            .expect("reference wire");
        let mut document: Value = serde_json::from_slice(&wire.bytes).expect("reference JSON");
        document["output"]["prdt_name"] = Value::String("must not be used".to_owned());
        wire.bytes = serde_json::to_vec(&document).expect("reference bytes");
    });
    let envelopes = normalize_kis_envelopes(&source, &stored).expect("official reference shape");
    let reference = envelopes
        .iter()
        .find(|envelope| envelope.kind == ResponseKind::Reference)
        .expect("reference envelope");
    let document: Value = serde_json::from_slice(&reference.bytes).expect("canonical reference");
    let first_symbol = format!("{}.KRX", KR_ETF_CORE_SYMBOLS[0]);
    let instrument = document["instruments"]
        .as_array()
        .expect("instruments")
        .iter()
        .find(|instrument| instrument["symbol"] == first_symbol)
        .expect("first instrument");
    assert_eq!(
        instrument["name"],
        format!("ETF {}", KR_ETF_CORE_SYMBOLS[0])
    );
}

#[test]
fn daily_bars_reference_fields_are_required_and_must_match_the_request() {
    let (_temp, _store, source, stored) = fixture(|wires| {
        let wire = wires
            .iter_mut()
            .find(|wire| wire.kind == ResponseKind::Bars)
            .expect("bars wire");
        let mut document: Value = serde_json::from_slice(&wire.bytes).expect("bars JSON");
        document["output1"]
            .as_object_mut()
            .expect("output1")
            .remove("hts_kor_isnm");
        wire.bytes = serde_json::to_vec(&document).expect("bars bytes");
    });
    let error = normalize_kis_envelopes(&source, &stored).expect_err("missing official name");
    assert!(matches!(
        error,
        NormalizeError::MissingField {
            kind: ResponseKind::Bars,
            field,
            ..
        } if field == "hts_kor_isnm"
    ));

    let (_temp, _store, source, stored) = fixture(|wires| {
        let wire = wires
            .iter_mut()
            .find(|wire| wire.kind == ResponseKind::Bars)
            .expect("bars wire");
        let mut document: Value = serde_json::from_slice(&wire.bytes).expect("bars JSON");
        document["output1"]["stck_shrn_iscd"] = Value::String("000001".to_owned());
        wire.bytes = serde_json::to_vec(&document).expect("bars bytes");
    });
    let error = normalize_kis_envelopes(&source, &stored).expect_err("provider symbol mismatch");
    assert!(matches!(
        error,
        NormalizeError::InvalidField {
            kind: ResponseKind::Bars,
            field,
            ..
        } if field == "stck_shrn_iscd"
    ));
}

#[test]
fn inquire_price_symbol_is_required_and_must_match_the_request() {
    for mutation in ["missing", "mismatch"] {
        let (_temp, _store, source, stored) = fixture(|wires| {
            let wire = wires
                .iter_mut()
                .find(|wire| wire.kind == ResponseKind::Reference)
                .expect("reference wire");
            let mut document: Value = serde_json::from_slice(&wire.bytes).expect("reference JSON");
            if mutation == "missing" {
                document["output"]
                    .as_object_mut()
                    .expect("output")
                    .remove("stck_shrn_iscd");
            } else {
                document["output"]["stck_shrn_iscd"] = Value::String("000001".to_owned());
            }
            wire.bytes = serde_json::to_vec(&document).expect("reference bytes");
        });
        let error = normalize_kis_envelopes(&source, &stored).expect_err("invalid provider symbol");
        match mutation {
            "missing" => assert!(matches!(
                error,
                NormalizeError::MissingField {
                    kind: ResponseKind::Reference,
                    field,
                    ..
                }
                if field == "stck_shrn_iscd"
            )),
            "mismatch" => assert!(matches!(
                error,
                NormalizeError::InvalidField {
                    kind: ResponseKind::Reference,
                    field,
                    ..
                }
                if field == "stck_shrn_iscd"
            )),
            _ => unreachable!("fixed mutation"),
        }
    }
}

#[test]
fn duplicate_and_conflicting_daily_bar_names_fail_closed() {
    for (name, conflict) in [
        (format!("ETF {}", KR_ETF_CORE_SYMBOLS[0]), false),
        ("other".to_owned(), true),
    ] {
        let (_temp, _store, source, stored) = fixture(|wires| {
            let mut duplicate = wires
                .iter()
                .find(|wire| wire.kind == ResponseKind::Bars)
                .expect("bars wire")
                .clone();
            duplicate.file_name = "daily-bars-duplicate-page-01.json".to_owned();
            let mut document: Value = serde_json::from_slice(&duplicate.bytes).expect("bars JSON");
            document["output1"]["hts_kor_isnm"] = Value::String(name.clone());
            document["output2"] = Value::Array(Vec::new());
            duplicate.bytes = serde_json::to_vec(&document).expect("duplicate bytes");
            wires.push(duplicate);
        });
        let error = normalize_kis_envelopes(&source, &stored).expect_err("duplicate name source");
        if conflict {
            assert!(matches!(
                error,
                NormalizeError::ConflictingRow {
                    kind: ResponseKind::Reference,
                    ..
                }
            ));
        } else {
            assert!(matches!(
                error,
                NormalizeError::DuplicateRow {
                    kind: ResponseKind::Reference,
                    ..
                }
            ));
        }
    }
}

#[test]
fn wire_kis_scope_cannot_enter_curation_directly() {
    let (_temp, store, source, _stored) = fixture(|_| {});
    let error = curation_inputs_from_raw(&store, &source)
        .expect_err("provider wire bytes must be normalized before curation");
    assert!(matches!(
        error,
        market_data::CurateError::UnsupportedScope {
            provider,
            market,
            ..
        } if provider == PROVIDER_KIS && market == MARKET_KR
    ));
}

#[test]
fn normalized_holiday_is_an_explicit_no_price_result() {
    let (temp, store, source, _stored) = fixture(|wires| {
        for wire in wires
            .iter_mut()
            .filter(|wire| wire.kind == ResponseKind::Bars)
        {
            let mut document: Value = serde_json::from_slice(&wire.bytes).expect("bars json");
            document["output2"] = Value::Array(
                document["output2"]
                    .as_array()
                    .expect("bars rows")
                    .iter()
                    .filter(|row| row["stck_bsop_date"] != TARGET_DATE.replace('-', ""))
                    .cloned()
                    .collect(),
            );
            wire.bytes = serde_json::to_vec(&document).expect("holiday bars");
        }
        let wire = wires
            .iter_mut()
            .find(|wire| wire.kind == ResponseKind::Calendar)
            .expect("calendar wire");
        let mut document: Value = serde_json::from_slice(&wire.bytes).expect("calendar json");
        let rows = document["output"].as_array_mut().expect("calendar rows");
        rows.iter_mut()
            .find(|row| row["bass_dt"] == TARGET_DATE.replace('-', ""))
            .expect("target calendar row")["opnd_yn"] = Value::String("N".to_owned());
        wire.bytes = serde_json::to_vec(&document).expect("holiday calendar");
    });
    let normalized = normalize_kis_batch(&store, &source).expect("holiday normalization");
    let (calendar, master) =
        curation_inputs_from_raw(&store, &normalized.entry).expect("holiday canonical inputs");
    assert!(!calendar.is_session(source.date));
    assert_eq!(master.instruments().count(), KR_ETF_CORE_SYMBOLS.len());
    let dataset_id = DatasetId::parse("kis-normalized-holiday").expect("dataset id");
    let error = curate_batch(
        &store,
        &normalized.entry,
        &calendar,
        &master,
        &CurateStore::new(temp.path().join("data")),
        &CurateRequest {
            dataset_id: &dataset_id,
            market: MARKET_KR,
            source: PROVIDER_KIS_NORMALIZED,
            now: normalized.entry.retrieved_at,
        },
    )
    .expect_err("holiday must not create a price dataset");
    assert!(matches!(
        error,
        market_data::CurateError::EodUnavailable {
            dataset_id: id,
            target_date,
        } if id == "kis-normalized-holiday" && target_date == source.date
    ));
}

#[test]
fn normalized_missing_open_session_is_eod_unavailable_not_malformed() {
    let (temp, store, source, _stored) = fixture(|wires| {
        for wire in wires
            .iter_mut()
            .filter(|wire| wire.kind == ResponseKind::Bars)
        {
            let mut document: Value = serde_json::from_slice(&wire.bytes).expect("bars json");
            document["output2"] = Value::Array(
                document["output2"]
                    .as_array()
                    .expect("bars rows")
                    .iter()
                    .filter(|row| row["stck_bsop_date"] != TARGET_DATE.replace('-', ""))
                    .cloned()
                    .collect(),
            );
            wire.bytes = serde_json::to_vec(&document).expect("unavailable bars");
        }
    });
    let normalized = normalize_kis_batch(&store, &source)
        .expect("empty target bars are a valid unavailable delivery");
    let (calendar, master) = curation_inputs_from_raw(&store, &normalized.entry)
        .expect("open unavailable canonical inputs");
    assert!(calendar.is_session(source.date));
    let dataset_id = DatasetId::parse("kis-normalized-open-unavailable").expect("dataset id");
    let error = curate_batch(
        &store,
        &normalized.entry,
        &calendar,
        &master,
        &CurateStore::new(temp.path().join("data")),
        &CurateRequest {
            dataset_id: &dataset_id,
            market: MARKET_KR,
            source: PROVIDER_KIS_NORMALIZED,
            now: normalized.entry.retrieved_at,
        },
    )
    .expect_err("open no-price delivery must not create a price dataset");
    assert!(matches!(
        error,
        market_data::CurateError::EodUnavailable {
            dataset_id: id,
            target_date,
        } if id == "kis-normalized-open-unavailable" && target_date == source.date
    ));
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
            endpoint: "kis.normalized/kis-wire-to-canonical-v2/bars".to_owned(),
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
            "output1": [{
                "sht_cd": "069500",
                "record_date": "20260814",
                "per_sto_divi_amt": "100",
                "divi_pay_dt": "20260828"
            }]
        }))
        .expect("action bytes");
    });
    let error = normalize_kis_envelopes(&source, &stored).expect_err("unsupported action");
    assert!(matches!(error, NormalizeError::UnsupportedAction { .. }));
}

#[test]
fn official_alphanumeric_ksd_short_code_is_validated_then_filtered() {
    let (_temp, _store, source, stored) = fixture(|wires| {
        let wire = wires
            .iter_mut()
            .find(|wire| wire.file_name.contains("dividend"))
            .expect("dividend wire");
        wire.bytes = serde_json::to_vec(&json!({
            "rt_cd": "0",
            "output1": [{
                "sht_cd": "11138K",
                "record_date": "20260814",
                "per_sto_divi_amt": "100",
                "divi_pay_dt": "20260828"
            }]
        }))
        .expect("official alphanumeric action bytes");
    });
    let envelopes = normalize_kis_envelopes(&source, &stored).expect("valid non-core KSD row");
    let actions = envelopes
        .iter()
        .find(|envelope| envelope.kind == ResponseKind::CorporateActions)
        .expect("canonical actions");
    let document: Value = serde_json::from_slice(&actions.bytes).expect("canonical JSON");
    assert_eq!(document["actions"], json!([]));
}

#[test]
fn malformed_alphanumeric_ksd_short_codes_fail_closed() {
    for invalid_symbol in ["11138k", "11138-", "11138", "11138KK"] {
        let (_temp, _store, source, stored) = fixture(|wires| {
            let wire = wires
                .iter_mut()
                .find(|wire| wire.file_name.contains("dividend"))
                .expect("dividend wire");
            wire.bytes = serde_json::to_vec(&json!({
                "rt_cd": "0",
                "output1": [{
                    "sht_cd": invalid_symbol,
                    "record_date": "20260814",
                    "per_sto_divi_amt": "100",
                    "divi_pay_dt": "20260828"
                }]
            }))
            .expect("invalid action bytes");
        });
        let error = normalize_kis_envelopes(&source, &stored).expect_err("invalid KSD short code");
        assert!(matches!(
            error,
            NormalizeError::InvalidField {
                kind: ResponseKind::CorporateActions,
                field,
                ..
            } if field == "sht_cd"
        ));
    }
}

#[test]
fn bonus_issue_filters_by_record_date_and_preserves_later_right_date() {
    let (_temp, _store, source, stored) = fixture(|wires| {
        let wire = wires
            .iter_mut()
            .find(|wire| wire.file_name.contains("corporate-actions-bonus"))
            .expect("bonus wire");
        wire.bytes = serde_json::to_vec(&json!({
            "rt_cd": "0",
            "output1": [{
                "record_date": "20260814",
                "sht_cd": "069500",
                "isin_name": "KODEX 200",
                "fix_rate": "0.05",
                "right_dt": "20260818",
                "odd_pay_dt": "20260828",
                "list_date": "20260818",
                "tot_issue_stk_qty": "105000000",
                "issue_stk_qty": "5000000",
                "stk_kind": "보통주"
            }]
        }))
        .expect("bonus bytes");
    });
    let envelopes = normalize_kis_envelopes(&source, &stored).expect("bonus normalization");
    let document = envelopes
        .iter()
        .find(|envelope| envelope.kind == ResponseKind::CorporateActions)
        .expect("canonical actions");
    let document: Value = serde_json::from_slice(&document.bytes).expect("canonical json");
    let action = &document["actions"][0];
    assert_eq!(action["instrument"], "069500.KRX");
    assert_eq!(action["type"], "split");
    assert_eq!(action["ex_date"], "2026-08-18");
    assert_eq!(action["record_date"], "2026-08-14");
    assert_eq!(action["split_factor"], "1.05");
    assert_eq!(action["ratio"], "1.05:1");
    assert_eq!(action["available_at"], RETRIEVED_AT);
    assert!(action.get("announced_at").is_none());
    assert_eq!(
        document["_lineage"]["upstream_batch_id"],
        source.batch_id.to_string()
    );
}

#[test]
fn reverse_split_allows_official_blank_list_dt_and_remains_typed_unsupported() {
    let (_temp, _store, source, stored) = fixture(|wires| {
        let wire = wires
            .iter_mut()
            .find(|wire| wire.file_name.contains("reverse-split"))
            .expect("reverse split wire");
        wire.bytes = serde_json::to_vec(&json!({
            "rt_cd": "0",
            "output1": [{
                "sht_cd": "069500",
                "record_date": "20260814",
                "list_dt": "",
                "inter_bf_face_amt": "100",
                "inter_af_face_amt": "1000"
            }]
        }))
        .expect("reverse split bytes");
    });
    let error = normalize_kis_envelopes(&source, &stored).expect_err("unsupported action");
    assert!(matches!(error, NormalizeError::UnsupportedAction { .. }));
}

#[test]
fn documented_blank_secondary_action_dates_are_validated_then_filtered() {
    let (_temp, _store, source, stored) = fixture(|wires| {
        for (label, row) in [
            (
                "dividend",
                json!({
                    "sht_cd": "000001",
                    "record_date": "20260814",
                    "divi_pay_dt": "",
                    "per_sto_divi_amt": "100"
                }),
            ),
            (
                "merger-split",
                json!({
                    "sht_cd": "000001",
                    "record_date": "20260814",
                    "list_dt": "",
                    "merge_rate": "1.00"
                }),
            ),
            (
                "reverse-split",
                json!({
                    "sht_cd": "000001",
                    "record_date": "20260814",
                    "list_dt": "",
                    "inter_bf_face_amt": "100",
                    "inter_af_face_amt": "1000"
                }),
            ),
            (
                "capital-decrease",
                json!({
                    "sht_cd": "000001",
                    "record_date": "20260814",
                    "list_dt": "",
                    "reduce_cap_rate": "1.00"
                }),
            ),
        ] {
            let wire = wires
                .iter_mut()
                .find(|wire| wire.file_name.contains(label))
                .expect("corporate action wire");
            wire.bytes =
                serde_json::to_vec(&json!({"rt_cd": "0", "output1": [row]})).expect("action bytes");
        }
    });
    let envelopes = normalize_kis_envelopes(&source, &stored).expect("unrelated actions filtered");
    let document = envelopes
        .iter()
        .find(|envelope| envelope.kind == ResponseKind::CorporateActions)
        .expect("canonical actions");
    let document: Value = serde_json::from_slice(&document.bytes).expect("canonical json");
    assert_eq!(document["actions"], json!([]));
}

#[test]
fn optional_action_date_is_required_as_a_field_and_validated_when_nonblank() {
    let (_temp, _store, source, stored) = fixture(|wires| {
        let wire = wires
            .iter_mut()
            .find(|wire| wire.file_name.contains("reverse-split"))
            .expect("reverse split wire");
        wire.bytes = serde_json::to_vec(&json!({
            "rt_cd": "0",
            "output1": [{
                "sht_cd": "000001",
                "record_date": "20260814",
                "inter_bf_face_amt": "100",
                "inter_af_face_amt": "1000"
            }]
        }))
        .expect("reverse split bytes");
    });
    let error = normalize_kis_envelopes(&source, &stored).expect_err("missing list date field");
    assert!(matches!(error, NormalizeError::MissingField { field, .. } if field == "list_dt"));

    let (_temp, _store, source, stored) = fixture(|wires| {
        let wire = wires
            .iter_mut()
            .find(|wire| wire.file_name.contains("reverse-split"))
            .expect("reverse split wire");
        wire.bytes = serde_json::to_vec(&json!({
            "rt_cd": "0",
            "output1": [{
                "sht_cd": "000001",
                "record_date": "20260814",
                "list_dt": "not-a-date",
                "inter_bf_face_amt": "100",
                "inter_af_face_amt": "1000"
            }]
        }))
        .expect("reverse split bytes");
    });
    let error = normalize_kis_envelopes(&source, &stored).expect_err("invalid list date");
    assert!(matches!(
        error,
        NormalizeError::InvalidField { field, .. } if field == "list_dt"
    ));

    let (_temp, _store, source, stored) = fixture(|wires| {
        let wire = wires
            .iter_mut()
            .find(|wire| wire.file_name.contains("reverse-split"))
            .expect("reverse split wire");
        wire.bytes = serde_json::to_vec(&json!({
            "rt_cd": "0",
            "output1": [{
                "sht_cd": "000001",
                "list_dt": "",
                "inter_bf_face_amt": "100",
                "inter_af_face_amt": "1000"
            }]
        }))
        .expect("reverse split bytes");
    });
    let error = normalize_kis_envelopes(&source, &stored).expect_err("missing record date");
    assert!(matches!(error, NormalizeError::MissingField { field, .. } if field == "record_date"));
}

#[test]
fn normalized_bonus_action_curates_with_retrieval_availability_only() {
    let (temp, store, source, _stored) = fixture(|wires| {
        let wire = wires
            .iter_mut()
            .find(|wire| wire.file_name.contains("corporate-actions-bonus"))
            .expect("bonus wire");
        wire.bytes = serde_json::to_vec(&json!({
            "rt_cd": "0",
            "output1": [{
                "record_date": "20260814",
                "sht_cd": "069500",
                "fix_rate": "0.05",
                "right_dt": "20260814"
            }]
        }))
        .expect("bonus bytes");
    });
    let normalized = normalize_kis_batch(&store, &source).expect("normalize bonus");
    let (calendar, master) =
        curation_inputs_from_raw(&store, &normalized.entry).expect("canonical inputs");
    let curated = CurateStore::new(temp.path().join("data"));
    let dataset_id = DatasetId::parse("kis-normalized-bonus").expect("dataset id");
    let outcome = curate_batch(
        &store,
        &normalized.entry,
        &calendar,
        &master,
        &curated,
        &CurateRequest {
            dataset_id: &dataset_id,
            market: MARKET_KR,
            source: PROVIDER_KIS_NORMALIZED,
            now: UtcTimestamp::parse_rfc3339("2026-08-14T09:00:00Z").expect("now"),
        },
    )
    .expect("bonus action is curation-compatible");
    assert_eq!(outcome.actions_written, 1);
    let actions = read_corporate_actions(&curated.corporate_actions_path(
        MARKET_KR,
        "069500.KRX",
        2026,
        outcome.dataset_version,
    ))
    .expect("read curated action");
    assert_eq!(actions.len(), 1);
    assert!(actions[0].announced_at.is_none());
    assert_eq!(actions[0].available_at, source.retrieved_at);
}

#[test]
fn malformed_bonus_row_is_not_dropped_by_universe_filter() {
    let (_temp, _store, source, stored) = fixture(|wires| {
        let wire = wires
            .iter_mut()
            .find(|wire| wire.file_name.contains("corporate-actions-bonus"))
            .expect("bonus wire");
        wire.bytes = serde_json::to_vec(&json!({
            "rt_cd": "0",
            "output1": [{
                "record_date": "not-a-date",
                "sht_cd": "000001",
                "fix_rate": "0.05",
                "right_dt": "20260814"
            }]
        }))
        .expect("bonus bytes");
    });
    let error = normalize_kis_envelopes(&source, &stored).expect_err("malformed row");
    assert!(matches!(
        error,
        NormalizeError::InvalidField {
            kind: ResponseKind::CorporateActions,
            field,
            ..
        } if field == "record_date"
    ));
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
