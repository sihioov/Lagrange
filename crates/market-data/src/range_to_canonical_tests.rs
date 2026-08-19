use super::load_with_approved_pin_for_test;
use std::fs;
use std::path::Path;

use crate::contract::{
    FetchMode, MARKET_KR, PROVIDER_KIS, PROVIDER_KIS_DAILY_RANGE,
    PROVIDER_KIS_DAILY_RANGE_NORMALIZED, RawEnvelope, RequestMetadata, ResponseKind,
};
use crate::range_normalize::{
    RANGE_NORMALIZER, RANGE_NORMALIZER_SCHEMA_VERSION, RangeNormalizationLineage,
    RangeNormalizationSourceFile, RangeNormalizationSourceRow,
};
use crate::range_to_canonical::{
    RANGE_CANONICAL_BRIDGE_VERSION, REQUIRED_ACTION_KINDS, RangeAction, RangeCanonicalError,
    build_range_canonical_candidate,
};
use crate::{BatchSpec, ManifestEntry, RawStore};
use domain::{BatchId, ContentHash, TradingDate, UtcTimestamp};
use serde_json::{Value, json};
use tempfile::TempDir;
use uuid::Uuid;

const DATE: &str = "2020-01-31";
const ACQUIRED: &str = "2026-08-19T00:00:00Z";

fn date() -> TradingDate {
    TradingDate::parse(DATE).unwrap()
}

fn timestamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::parse_rfc3339(value).unwrap()
}

fn etf_bars() -> Vec<Value> {
    crate::KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| {
            json!({
                "instrument": format!("{symbol}.KRX"),
                "date": DATE,
                "open": "100.25",
                "high": "101.5",
                "low": "99.75",
                "close": "100.75",
                "volume": "42",
            })
        })
        .collect()
}

fn source_request(symbol: &str) -> RequestMetadata {
    RequestMetadata {
        endpoint: "/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice".to_owned(),
        query: vec![
            ("FID_COND_MRKT_DIV_CODE".to_owned(), "J".to_owned()),
            ("FID_INPUT_ISCD".to_owned(), symbol.to_owned()),
            ("FID_INPUT_DATE_1".to_owned(), "20200131".to_owned()),
            ("FID_INPUT_DATE_2".to_owned(), "20200131".to_owned()),
            ("FID_PERIOD_DIV_CODE".to_owned(), "D".to_owned()),
            ("FID_ORG_ADJ_PRC".to_owned(), "1".to_owned()),
        ],
        headers: vec![("tr_cont".to_owned(), String::new())],
        mode: FetchMode::Credentialed,
    }
}

fn store_source(raw: &RawStore) -> ManifestEntry {
    let id = BatchId::from_uuid(Uuid::new_v4());
    let envelopes = crate::KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| {
            let bytes = serde_json::to_vec(&json!({
                "rt_cd": "0",
                "output1": {"stck_shrn_iscd": symbol},
                "output2": [{
                    "stck_bsop_date": "20200131",
                    "stck_oprc": "100.25",
                    "stck_hgpr": "101.5",
                    "stck_lwpr": "99.75",
                    "stck_clpr": "100.75",
                    "acml_vol": "42"
                }]
            }))
            .unwrap();
            RawEnvelope::new(
                id,
                ResponseKind::Bars,
                format!("daily-bars-range-window-01-{symbol}.json"),
                bytes,
                timestamp(ACQUIRED),
                source_request(symbol),
            )
        })
        .collect::<Vec<_>>();
    let date = date();
    raw.store_batch(
        &BatchSpec {
            provider: PROVIDER_KIS_DAILY_RANGE,
            market: MARKET_KR,
            date: &date,
            batch_id: id,
            entitlement_reference: Some("test-entitlement"),
            mode: FetchMode::Credentialed,
        },
        &envelopes,
    )
    .unwrap()
}

fn source_lineage(source: &ManifestEntry) -> RangeNormalizationLineage {
    let source_manifest_hash = ContentHash::from_bytes(&serde_json::to_vec(source).unwrap());
    let source_files = source
        .files
        .iter()
        .map(|file| RangeNormalizationSourceFile {
            kind: file.kind,
            file_name: file.file_name.clone(),
            content_hash: file.content_hash.clone(),
            size_bytes: file.size_bytes,
            request: file.request.clone(),
        })
        .collect();
    let source_rows = crate::KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| {
            let file = source
                .files
                .iter()
                .find(|file| {
                    file.request
                        .query
                        .iter()
                        .any(|(key, value)| key == "FID_INPUT_ISCD" && value == *symbol)
                })
                .unwrap();
            let row_bytes = serde_json::to_vec(&json!({
                "stck_bsop_date": "20200131",
                "stck_oprc": "100.25",
                "stck_hgpr": "101.5",
                "stck_lwpr": "99.75",
                "stck_clpr": "100.75",
                "acml_vol": "42"
            }))
            .unwrap();
            RangeNormalizationSourceRow {
                source_file_name: file.file_name.clone(),
                source_file_hash: file.content_hash.clone(),
                source_file_size_bytes: file.size_bytes,
                row_content_hash: ContentHash::from_bytes(&row_bytes),
                row_size_bytes: row_bytes.len() as u64,
                source_query_start: date(),
                source_query_end: date(),
                symbol: (*symbol).to_owned(),
                row_date: date(),
            }
        })
        .collect();
    RangeNormalizationLineage {
        schema_version: RANGE_NORMALIZER_SCHEMA_VERSION,
        normalizer: RANGE_NORMALIZER.to_owned(),
        upstream_provider: PROVIDER_KIS_DAILY_RANGE.to_owned(),
        upstream_market: MARKET_KR.to_owned(),
        upstream_batch_id: source.batch_id,
        upstream_manifest_hash: source_manifest_hash,
        source_start: date(),
        source_end: date(),
        source_files,
        calendar_id: "xkrx-reviewed".to_owned(),
        calendar_hash: ContentHash::from_bytes(b"calendar"),
        listing_snapshot_id: "listing-v1".to_owned(),
        listing_snapshot_hash: ContentHash::from_bytes(b"listing-pinned"),
        selected_session: date(),
        source_rows,
        acquired_at: timestamp(ACQUIRED),
        availability_evidence: false,
        revision_evidence: false,
        knowledge_time_evidence: false,
    }
}

fn stage4a_entry(raw: &RawStore, source: &ManifestEntry) -> ManifestEntry {
    stage4a_entry_version(
        raw,
        source,
        crate::range_normalize::RANGE_NORMALIZED_SCHEMA_VERSION,
        crate::range_normalize::RANGE_NORMALIZER,
    )
}

fn stage4a_entry_version(
    raw: &RawStore,
    source: &ManifestEntry,
    schema_version: u32,
    normalizer: &str,
) -> ManifestEntry {
    let id = BatchId::from_uuid(Uuid::new_v4());
    let mut lineage = source_lineage(source);
    lineage.listing_snapshot_hash = listing_hash();
    lineage.schema_version = schema_version;
    lineage.normalizer = normalizer.to_owned();
    let document = json!({
        "schema_version": schema_version,
        "dataset_kind": "kis-daily-range-bars",
        "date": DATE,
        "bars": etf_bars(),
        "acquired_at": ACQUIRED,
        "pit": {
            "mode": "acquisition-time-vendor-snapshot",
            "strict": false,
            "availability_evidence": false,
            "revision_evidence": false,
            "knowledge_time_evidence": false,
        },
        "_lineage": lineage,
    });
    let bytes = serde_json::to_vec(&document).unwrap();
    let date = date();
    raw.store_batch(
        &BatchSpec {
            provider: PROVIDER_KIS_DAILY_RANGE_NORMALIZED,
            market: MARKET_KR,
            date: &date,
            batch_id: id,
            entitlement_reference: Some("test-entitlement"),
            mode: FetchMode::Credentialed,
        },
        &[RawEnvelope::new(
            id,
            ResponseKind::Bars,
            format!("bars-{DATE}.json"),
            bytes,
            timestamp(ACQUIRED),
            RequestMetadata {
                endpoint: format!("kis.range.normalized/{normalizer}/bars"),
                query: vec![
                    ("source_batch_id".to_owned(), source.batch_id.to_string()),
                    (
                        "source_manifest_hash".to_owned(),
                        ContentHash::from_bytes(&serde_json::to_vec(source).unwrap()).to_string(),
                    ),
                    ("session_date".to_owned(), DATE.to_owned()),
                ],
                headers: Vec::new(),
                mode: FetchMode::Credentialed,
            },
        )],
    )
    .unwrap()
}

fn listing_document() -> Value {
    let instruments = crate::KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| {
            json!({
                "instrument_id": format!("{symbol}.KRX"),
                "name": format!("ETF {symbol}"),
                "kind": "etf",
                "lot_size": "1",
                "listed_at": "2019-01-01",
                "delisted_at": null,
                "acquired_at": ACQUIRED
            })
        })
        .collect::<Vec<_>>();
    let view = json!({
        "schema_version": 1,
        "snapshot_id": "listing-v1",
        "source": "reviewed-listing-source",
        "captured_at": ACQUIRED,
        "instruments": instruments
    });
    let snapshot_hash = listing_hash();
    let mut object = view.as_object().unwrap().clone();
    object.insert("snapshot_hash".to_owned(), json!(snapshot_hash));
    Value::Object(object)
}

fn listing_hash() -> ContentHash {
    ContentHash::parse("sha256:267dc7aa065c6647ce634218fb8514fa49547a110ffc3d30f3bc00819ef7e992")
        .unwrap()
}

fn action_request(kind: &str, range_start: TradingDate, range_end: TradingDate) -> RequestMetadata {
    let (path, tr_id, extra) = match kind {
        "paidin-subscription" => (
            "/uapi/domestic-stock/v1/ksdinfo/paidin-capin",
            "HHKDB669100C0",
            vec![("GB1", "1")],
        ),
        "paidin-record" => (
            "/uapi/domestic-stock/v1/ksdinfo/paidin-capin",
            "HHKDB669100C0",
            vec![("GB1", "2")],
        ),
        "bonus-issue" => (
            "/uapi/domestic-stock/v1/ksdinfo/bonus-issue",
            "HHKDB669101C0",
            vec![],
        ),
        "dividend" => (
            "/uapi/domestic-stock/v1/ksdinfo/dividend",
            "HHKDB669102C0",
            vec![("GB1", "0"), ("HIGH_GB", "")],
        ),
        "merger-split" => (
            "/uapi/domestic-stock/v1/ksdinfo/merger-split",
            "HHKDB669104C0",
            vec![],
        ),
        "reverse-split" => (
            "/uapi/domestic-stock/v1/ksdinfo/rev-split",
            "HHKDB669105C0",
            vec![("MARKET_GB", "0")],
        ),
        "capital-decrease" => (
            "/uapi/domestic-stock/v1/ksdinfo/cap-dcrs",
            "HHKDB669106C0",
            vec![],
        ),
        _ => panic!("unknown kind"),
    };
    let mut query = vec![
        ("CTS".to_owned(), String::new()),
        ("F_DT".to_owned(), range_start.to_iso().replace('-', "")),
        ("T_DT".to_owned(), range_end.to_iso().replace('-', "")),
        ("SHT_CD".to_owned(), String::new()),
    ];
    query.extend(
        extra
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned())),
    );
    RequestMetadata {
        endpoint: path.to_owned(),
        query,
        headers: vec![
            ("tr_id".to_owned(), tr_id.to_owned()),
            ("tr_cont".to_owned(), String::new()),
        ],
        mode: FetchMode::Credentialed,
    }
}

fn action_entry(raw: &RawStore, bonus: bool) -> ManifestEntry {
    action_entry_with_response_marker(raw, bonus, None, None)
}

fn action_entry_with_response_marker(
    raw: &RawStore,
    bonus: bool,
    body_marker: Option<(&str, &str)>,
    tr_cont: Option<Option<&str>>,
) -> ManifestEntry {
    let id = BatchId::from_uuid(Uuid::new_v4());
    let date = date();
    let envelopes = REQUIRED_ACTION_KINDS
        .iter()
        .map(|kind| {
            let output = if bonus && *kind == "bonus-issue" {
                json!([{
                    "sht_cd": "069500",
                    "record_date": DATE,
                    "right_dt": DATE,
                    "fix_rate": "0.05"
                }])
            } else {
                json!([])
            };
            let mut response = json!({"rt_cd":"0", "output1": output});
            if let Some((marker_kind, marker_field)) = body_marker
                && marker_kind == *kind
            {
                response[marker_field] = json!("next-page");
            }
            let mut request = action_request(kind, date, date);
            if let Some(tr_cont) = tr_cont
                && *kind == REQUIRED_ACTION_KINDS[0]
            {
                request
                    .headers
                    .retain(|(key, _)| !key.eq_ignore_ascii_case("tr_cont"));
                if let Some(value) = tr_cont {
                    request
                        .headers
                        .push(("tr_cont".to_owned(), value.to_owned()));
                }
            }
            RawEnvelope::new(
                id,
                ResponseKind::CorporateActions,
                format!("{kind}.json"),
                serde_json::to_vec(&response).unwrap(),
                timestamp(ACQUIRED),
                request,
            )
        })
        .collect::<Vec<_>>();
    raw.store_batch(
        &BatchSpec {
            provider: PROVIDER_KIS,
            market: MARKET_KR,
            date: &date,
            batch_id: id,
            entitlement_reference: Some("test-entitlement"),
            mode: FetchMode::Credentialed,
        },
        &envelopes,
    )
    .unwrap()
}

fn write_package(
    root: &Path,
    source: &ManifestEntry,
    normalized: &ManifestEntry,
    actions: &ManifestEntry,
    tamper_manifest: bool,
) -> ContentHash {
    fs::create_dir_all(root).unwrap();
    let schedule = json!({
        "schema_version": 1,
        "calendar_id": "xkrx-reviewed",
        "calendar_hash": ContentHash::from_bytes(b"calendar"),
        "session_date": DATE,
        "open_utc": "2020-01-31T00:00:00Z",
        "close_utc": "2020-01-31T06:30:00Z",
        "break_start_utc": null,
        "break_end_utc": null,
        "authority": {"Reviewed": {"source": "reviewed-schedule", "version": "v1"}}
    });
    let pit = json!({
        "schema_version": 1,
        "policy_id": "kis-historical-vendor-snapshot-v1",
        "approved": true,
        "approved_by": "operator",
        "approved_at": ACQUIRED,
        "rationale": "KIS endpoint is an acquisition-time vendor snapshot; strict PIT is unavailable"
    });
    let artifacts = [
        ("schedule.json", serde_json::to_vec(&schedule).unwrap()),
        (
            "listing.json",
            serde_json::to_vec(&listing_document()).unwrap(),
        ),
        ("pit.json", serde_json::to_vec(&pit).unwrap()),
    ];
    for (name, bytes) in &artifacts {
        fs::write(root.join(name), bytes).unwrap();
    }
    let artifact_ref = |name: &str, bytes: &[u8]| {
        json!({
            "path": name,
            "sha256": ContentHash::from_bytes(bytes),
            "size_bytes": bytes.len(),
            "schema_version": 1
        })
    };
    let action_manifest_hash = ContentHash::from_bytes(&serde_json::to_vec(actions).unwrap());
    let action_refs = actions
        .files
        .iter()
        .map(|file| {
            let kind = file.file_name.trim_end_matches(".json");
            json!({
                "kind": kind,
                "raw_batch_id": actions.batch_id,
                "raw_manifest_hash": action_manifest_hash,
                "raw_file_name": file.file_name,
                "content_hash": file.content_hash,
                "size_bytes": file.size_bytes
            })
        })
        .collect::<Vec<_>>();
    let manifest = json!({
        "schema_version": 1,
        "session_date": DATE,
        "range_start": DATE,
        "range_end": DATE,
        "calendar": artifact_ref("schedule.json", &artifacts[0].1),
        "listing": artifact_ref("listing.json", &artifacts[1].1),
        "pit_policy": artifact_ref("pit.json", &artifacts[2].1),
        "actions": action_refs,
        "bridge_version": RANGE_CANONICAL_BRIDGE_VERSION,
        "source_batch_id": source.batch_id,
        "normalized_batch_id": normalized.batch_id
    });
    let mut manifest_bytes = serde_json::to_vec(&manifest).unwrap();
    if tamper_manifest {
        manifest_bytes.extend_from_slice(b"tampered");
    }
    fs::write(root.join("manifest.json"), &manifest_bytes).unwrap();
    ContentHash::from_bytes(&manifest_bytes)
}

fn fixture() -> (TempDir, RawStore, ManifestEntry, ContentHash, TempDir) {
    let raw_root = TempDir::new().unwrap();
    let raw = RawStore::new(raw_root.path());
    let source = store_source(&raw);
    let normalized = stage4a_entry(&raw, &source);
    let actions = action_entry(&raw, false);
    let package_root = TempDir::new().unwrap();
    let pin = write_package(package_root.path(), &source, &normalized, &actions, false);
    (raw_root, raw, normalized, pin, package_root)
}

#[test]
fn loader_produces_verified_candidate_and_replay_is_deterministic() {
    let (_raw_root, raw, normalized, pin, package_root) = fixture();
    let evidence =
        load_with_approved_pin_for_test(&raw, &normalized, package_root.path(), &pin).unwrap();
    let first = build_range_canonical_candidate(&raw, &normalized, &evidence).unwrap();
    let second = build_range_canonical_candidate(&raw, &normalized, &evidence).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.bars.len(), 11);
    assert_eq!(first.acquired_at, timestamp(ACQUIRED));
    assert!(first.actions.is_empty());
}

#[test]
fn fake_self_hash_and_manifest_or_artifact_tamper_block() {
    let (_raw_root, raw, normalized, pin, package_root) = fixture();
    let fake_pin = ContentHash::from_bytes(b"not-the-manifest");
    assert!(matches!(
        load_with_approved_pin_for_test(&raw, &normalized, package_root.path(), &fake_pin),
        Err(RangeCanonicalError::EvidencePackage { .. })
    ));
    fs::write(package_root.path().join("listing.json"), b"tampered").unwrap();
    assert!(matches!(
        load_with_approved_pin_for_test(&raw, &normalized, package_root.path(), &pin),
        Err(RangeCanonicalError::MissingListingMasterEvidence { .. })
    ));
}

#[test]
fn symlink_artifact_and_unsafe_manifest_path_block() {
    let (_raw_root, raw, normalized, pin, package_root) = fixture();
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("secret.json"), b"not evidence").unwrap();
    fs::remove_file(package_root.path().join("listing.json")).unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("secret.json"),
        package_root.path().join("listing.json"),
    )
    .unwrap();
    assert!(matches!(
        load_with_approved_pin_for_test(&raw, &normalized, package_root.path(), &pin),
        Err(RangeCanonicalError::UnsafeEvidencePath { .. })
            | Err(RangeCanonicalError::EvidenceArtifact { .. })
    ));
}

#[test]
fn each_missing_evidence_class_is_typed_and_fail_closed() {
    let (_raw_root, raw, normalized, pin, package_root) = fixture();
    fs::remove_file(package_root.path().join("schedule.json")).unwrap();
    assert!(matches!(
        load_with_approved_pin_for_test(&raw, &normalized, package_root.path(), &pin),
        Err(RangeCanonicalError::UnsupportedHistoricalSessionSchedule { .. })
    ));

    let (_raw_root, raw, normalized, pin, package_root) = fixture();
    fs::remove_file(package_root.path().join("listing.json")).unwrap();
    assert!(matches!(
        load_with_approved_pin_for_test(&raw, &normalized, package_root.path(), &pin),
        Err(RangeCanonicalError::MissingListingMasterEvidence { .. })
    ));

    let (_raw_root, raw, normalized, pin, package_root) = fixture();
    fs::remove_file(package_root.path().join("pit.json")).unwrap();
    assert!(matches!(
        load_with_approved_pin_for_test(&raw, &normalized, package_root.path(), &pin),
        Err(RangeCanonicalError::NonStrictPitNotApproved { .. })
    ));

    let (_raw_root, raw, normalized, _pin, package_root) = fixture();
    let manifest_path = package_root.path().join("manifest.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["actions"].as_array_mut().unwrap().pop();
    let bytes = serde_json::to_vec(&manifest).unwrap();
    fs::write(&manifest_path, &bytes).unwrap();
    let pin = ContentHash::from_bytes(&bytes);
    assert!(matches!(
        load_with_approved_pin_for_test(&raw, &normalized, package_root.path(), &pin),
        Err(RangeCanonicalError::MissingActionEvidence { .. })
    ));
}

#[test]
fn action_zero_result_is_verified_from_all_seven_raw_responses() {
    let (_raw_root, raw, normalized, pin, package_root) = fixture();
    let evidence =
        load_with_approved_pin_for_test(&raw, &normalized, package_root.path(), &pin).unwrap();
    let candidate = build_range_canonical_candidate(&raw, &normalized, &evidence).unwrap();
    assert_eq!(candidate.action_coverage_file_count(), 7);
    assert!(candidate.action_coverage_is_zero_result());
}

#[test]
fn action_nonterminal_body_markers_are_rejected_for_each_endpoint() {
    // Exercise every allowlisted endpoint while covering each continuation
    // spelling that can appear in the KIS response family.  Stage4B has no
    // persisted continuation chain, so every non-empty marker is permanent.
    let marker_fields = [
        "cts",
        "ctx_area_fk",
        "ctx_area_nk",
        "ctx_area_fk200",
        "ctx_area_nk200",
        "cts",
        "cts",
    ];
    for (kind, marker) in REQUIRED_ACTION_KINDS.iter().zip(marker_fields) {
        let raw_root = TempDir::new().unwrap();
        let raw = RawStore::new(raw_root.path());
        let source = store_source(&raw);
        let normalized = stage4a_entry(&raw, &source);
        let actions = action_entry_with_response_marker(&raw, false, Some((kind, marker)), None);
        let package_root = TempDir::new().unwrap();
        let pin = write_package(package_root.path(), &source, &normalized, &actions, false);
        let result = load_with_approved_pin_for_test(&raw, &normalized, package_root.path(), &pin);
        assert!(matches!(
            result,
            Err(RangeCanonicalError::IncompleteActionPagination {
                kind: actual,
                marker: actual_marker,
            }) if actual == *kind && actual_marker == marker
        ));
    }
}

#[test]
fn action_request_requires_explicit_empty_terminal_tr_cont() {
    for continuation in [Some(Some("N")), Some(None)] {
        let raw_root = TempDir::new().unwrap();
        let raw = RawStore::new(raw_root.path());
        let source = store_source(&raw);
        let normalized = stage4a_entry(&raw, &source);
        let actions = action_entry_with_response_marker(&raw, false, None, continuation);
        let package_root = TempDir::new().unwrap();
        let pin = write_package(package_root.path(), &source, &normalized, &actions, false);
        assert!(matches!(
            load_with_approved_pin_for_test(&raw, &normalized, package_root.path(), &pin),
            Err(RangeCanonicalError::IncompleteActionPagination {
                marker,
                ..
            }) if marker == "tr_cont"
        ));
    }
}

#[test]
fn candidate_identity_binds_the_pinned_artifact_bytes() {
    let (_raw_root, raw, normalized, pin, package_root) = fixture();
    let first_evidence =
        load_with_approved_pin_for_test(&raw, &normalized, package_root.path(), &pin).unwrap();
    let first = build_range_canonical_candidate(&raw, &normalized, &first_evidence).unwrap();

    let second_root = TempDir::new().unwrap();
    for name in ["manifest.json", "schedule.json", "listing.json", "pit.json"] {
        fs::copy(
            package_root.path().join(name),
            second_root.path().join(name),
        )
        .unwrap();
    }
    let schedule_path = second_root.path().join("schedule.json");
    let mut schedule: Value = serde_json::from_slice(&fs::read(&schedule_path).unwrap()).unwrap();
    schedule["authority"]["Reviewed"]["version"] = json!("v2");
    let schedule_bytes = serde_json::to_vec(&schedule).unwrap();
    fs::write(&schedule_path, &schedule_bytes).unwrap();
    let manifest_path = second_root.path().join("manifest.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["calendar"]["sha256"] = json!(ContentHash::from_bytes(&schedule_bytes));
    manifest["calendar"]["size_bytes"] = json!(schedule_bytes.len());
    let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
    fs::write(&manifest_path, &manifest_bytes).unwrap();
    let second_pin = ContentHash::from_bytes(&manifest_bytes);
    let second_evidence =
        load_with_approved_pin_for_test(&raw, &normalized, second_root.path(), &second_pin)
            .unwrap();
    let second = build_range_canonical_candidate(&raw, &normalized, &second_evidence).unwrap();
    assert_ne!(first.candidate_id, second.candidate_id);
}

#[test]
fn bonus_action_is_target_session_only_and_factor_is_greater_than_one() {
    let raw_root = TempDir::new().unwrap();
    let raw = RawStore::new(raw_root.path());
    let source = store_source(&raw);
    let normalized = stage4a_entry(&raw, &source);
    let actions = action_entry(&raw, true);
    let package_root = TempDir::new().unwrap();
    let pin = write_package(package_root.path(), &source, &normalized, &actions, false);
    let evidence =
        load_with_approved_pin_for_test(&raw, &normalized, package_root.path(), &pin).unwrap();
    let candidate = build_range_canonical_candidate(&raw, &normalized, &evidence).unwrap();
    assert_eq!(candidate.actions.len(), 1);
    match &candidate.actions[0] {
        RangeAction::BonusIssue {
            split_factor,
            record_date,
            ..
        } => {
            assert!(*split_factor > domain::FixedPoint::parse("1").unwrap());
            assert_eq!(*record_date, date());
        }
        RangeAction::Unsupported { .. } => panic!("bonus should be mapped"),
    }
}

#[test]
fn non_normalized_scope_is_rejected_before_any_evidence_load() {
    let (_raw_root, raw, mut normalized, pin, package_root) = fixture();
    normalized.provider = PROVIDER_KIS_DAILY_RANGE.to_owned();
    assert!(matches!(
        load_with_approved_pin_for_test(&raw, &normalized, package_root.path(), &pin),
        Err(RangeCanonicalError::UnsupportedScope { .. })
    ));
}

#[test]
fn public_loader_rejects_self_created_package_pin_when_registry_is_empty() {
    let (_raw_root, raw, normalized, _pin, package_root) = fixture();
    assert!(matches!(
        crate::range_to_canonical::load_verified_range_canonical_evidence(
            &raw,
            &normalized,
            package_root.path(),
        ),
        Err(RangeCanonicalError::EvidencePackage { .. })
    ));
}

#[test]
fn legacy_stage4a_v1_is_rejected_before_deserialization() {
    let raw_root = TempDir::new().unwrap();
    let raw = RawStore::new(raw_root.path());
    let source = store_source(&raw);
    let normalized = stage4a_entry_version(&raw, &source, 1, "kis-daily-range-to-session-bars-v1");
    let actions = action_entry(&raw, false);
    let package_root = TempDir::new().unwrap();
    let pin = write_package(package_root.path(), &source, &normalized, &actions, false);
    assert!(matches!(
        load_with_approved_pin_for_test(&raw, &normalized, package_root.path(), &pin),
        Err(RangeCanonicalError::UnsupportedLegacyStage4A {
            schema_version: 1,
            ..
        })
    ));
}
