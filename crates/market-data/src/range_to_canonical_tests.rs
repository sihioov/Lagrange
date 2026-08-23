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
    HistoricalBetaVerificationScope, RANGE_CANONICAL_BRIDGE_VERSION, REQUIRED_ACTION_KINDS,
    RangeAction, RangeCanonicalError, build_range_canonical_candidate,
    verify_historical_price_only_beta_input_for_scope, write_evidence_package,
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
                format!("daily-bars-range-window-1-{symbol}-page-01.json"),
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
    let mut lineage = source_lineage(source);
    lineage.listing_snapshot_hash = listing_hash();
    lineage.schema_version = schema_version;
    lineage.normalizer = normalizer.to_owned();
    let source_manifest_hash = ContentHash::from_bytes(&serde_json::to_vec(source).unwrap());
    let id = crate::deterministic_range_normalized_batch_id_with_identity(
        source,
        &source_manifest_hash,
        lineage.selected_session,
        &lineage.calendar_hash,
        &lineage.listing_snapshot_hash,
    );
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
    let envelopes = action_envelopes(id, bonus, body_marker, tr_cont, None);
    store_action_batch(raw, id, envelopes)
}

/// The seven initial-page KSD responses where `kind` (never `bonus-issue`)
/// carries one nonempty row.  `load_action_evidence` is deliberately
/// permissive about content and maps such a row to
/// `RangeAction::Unsupported`; only `validate_actions` rejects it.
fn action_entry_with_nonempty_unsupported(raw: &RawStore, kind: &str) -> ManifestEntry {
    let id = BatchId::from_uuid(Uuid::new_v4());
    let envelopes = action_envelopes(id, false, None, None, Some(kind));
    store_action_batch(raw, id, envelopes)
}

/// The seven KSD responses plus one unrelated bars file, exactly as the
/// daily EOD bundle stores them.  `load_action_evidence` requires the
/// pinned Raw batch to hold exactly seven files, so this batch can never be
/// loaded and must therefore never be selected as action evidence.
fn action_entry_with_extra_file(raw: &RawStore) -> ManifestEntry {
    let id = BatchId::from_uuid(Uuid::new_v4());
    let mut envelopes = action_envelopes(id, false, None, None, None);
    let bytes = serde_json::to_vec(&json!({
        "rt_cd": "0",
        "output1": {"stck_shrn_iscd": "069500"},
        "output2": []
    }))
    .unwrap();
    envelopes.push(RawEnvelope::new(
        id,
        ResponseKind::Bars,
        "daily-bars-eod-069500.json".to_owned(),
        bytes,
        timestamp(ACQUIRED),
        source_request("069500"),
    ));
    store_action_batch(raw, id, envelopes)
}

fn store_action_batch(raw: &RawStore, id: BatchId, envelopes: Vec<RawEnvelope>) -> ManifestEntry {
    let date = date();
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

fn action_envelopes(
    id: BatchId,
    bonus: bool,
    body_marker: Option<(&str, &str)>,
    tr_cont: Option<Option<&str>>,
    nonempty_unsupported: Option<&str>,
) -> Vec<RawEnvelope> {
    let date = date();
    REQUIRED_ACTION_KINDS
        .iter()
        .map(|kind| {
            let output = if bonus && *kind == "bonus-issue" {
                json!([{
                    "sht_cd": "069500",
                    "record_date": DATE,
                    "right_dt": DATE,
                    "fix_rate": "5"
                }])
            } else if nonempty_unsupported == Some(*kind) {
                json!([{
                    "sht_cd": "069500",
                    "record_date": DATE,
                    "right_dt": DATE
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
        .collect::<Vec<_>>()
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
fn raw_bound_historical_input_reauthenticates_exact_source_and_action_pins() {
    let raw_root = TempDir::new().unwrap();
    let raw = RawStore::new(raw_root.path());
    let source = store_source(&raw);
    let normalized = stage4a_entry(&raw, &source);
    let actions = action_entry(&raw, false);
    let source_pin = ContentHash::from_bytes(&serde_json::to_vec(&source).unwrap());
    let action_pin = ContentHash::from_bytes(&serde_json::to_vec(&actions).unwrap());
    let scope = HistoricalBetaVerificationScope {
        source_batch_id: source.batch_id,
        range_start: date(),
        range_end: date(),
        sessions: vec![date()],
        calendar_id: "xkrx-reviewed".to_owned(),
        calendar_hash: ContentHash::from_bytes(b"calendar"),
        listing_snapshot_id: "listing-v1".to_owned(),
        listing_snapshot_hash: listing_hash(),
    };

    let verified =
        verify_historical_price_only_beta_input_for_scope(&raw, &source_pin, &action_pin, &scope)
            .unwrap();
    assert_eq!(verified.source_batch_id(), source.batch_id);
    assert_eq!(verified.source_manifest_hash(), &source_pin);
    assert_eq!(verified.source_files().len(), 11);
    assert_eq!(verified.action_batch_id(), actions.batch_id);
    assert_eq!(verified.action_manifest_hash(), &action_pin);
    assert_eq!(verified.action_file_count(), 7);
    assert_eq!(verified.sessions().len(), 1);
    assert_eq!(
        verified.sessions()[0].normalized_batch_id(),
        normalized.batch_id
    );
    assert_eq!(verified.bars().len(), 11);
    assert!(verified.actions().is_empty());

    let first = crate::materialize_historical_price_only_beta(&verified).unwrap();
    let second = crate::materialize_historical_price_only_beta(&verified).unwrap();
    assert_eq!(first.source_batch_id(), verified.source_batch_id());
    assert_eq!(
        first.source_manifest_hash(),
        verified.source_manifest_hash()
    );
    assert_eq!(first.source_files(), verified.source_files());
    assert_eq!(first.action_batch_id(), verified.action_batch_id());
    assert_eq!(
        first.action_manifest_hash(),
        verified.action_manifest_hash()
    );
    assert_eq!(first.action_file_count(), verified.action_file_count());
    assert_eq!(first.row_count(), 11);
    assert_eq!(first.session_count(), 1);
    assert!(first.bonus_evidence().is_empty());
    assert_eq!(
        first.metadata(),
        crate::HistoricalPriceOnlyMetadata {
            audience: crate::HistoricalPriceOnlyAudience::OwnerOnly,
            vendor_snapshot: true,
            strict_pit: false,
            capability: crate::Capability::PriceReturnOnly,
            materialized: false,
            in_memory: true,
            ready: false,
        }
    );
    assert_eq!(first, second);
    assert_eq!(first.content_hash(), second.content_hash());

    let wrong_source_pin = ContentHash::from_bytes(b"wrong-stage5-pin");
    assert!(matches!(
        verify_historical_price_only_beta_input_for_scope(
            &raw,
            &wrong_source_pin,
            &action_pin,
            &scope,
        ),
        Err(RangeCanonicalError::HistoricalBetaContract { .. })
    ));
    let wrong_action_pin = ContentHash::from_bytes(b"wrong-action-pin");
    assert!(matches!(
        verify_historical_price_only_beta_input_for_scope(
            &raw,
            &source_pin,
            &wrong_action_pin,
            &scope,
        ),
        Err(RangeCanonicalError::MissingActionEvidence { .. })
    ));
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

fn schedule_bytes(calendar_hash: ContentHash) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schema_version": 1,
        "calendar_id": "xkrx-reviewed",
        "calendar_hash": calendar_hash,
        "session_date": DATE,
        "open_utc": "2020-01-31T00:00:00Z",
        "close_utc": "2020-01-31T06:30:00Z",
        "break_start_utc": null,
        "break_end_utc": null,
        "authority": {"Reviewed": {"source": "reviewed-schedule", "version": "v1"}}
    }))
    .unwrap()
}

fn pit_policy_bytes() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schema_version": 1,
        "policy_id": "kis-historical-vendor-snapshot-v1",
        "approved": true,
        "approved_by": "operator",
        "approved_at": ACQUIRED,
        "rationale": "KIS endpoint is an acquisition-time vendor snapshot; strict PIT is unavailable"
    }))
    .unwrap()
}

fn listing_bytes_with_snapshot_hash(snapshot_hash: ContentHash) -> Vec<u8> {
    let mut document = listing_document();
    document["snapshot_hash"] = json!(snapshot_hash);
    serde_json::to_vec(&document).unwrap()
}

/// The acceptance test: `write_evidence_package` output must round-trip
/// through the same pin loader production callers use. A schema mismatch
/// between the writer and `EvidencePackageManifest`'s `deny_unknown_fields`
/// deserializer would fail here even though it can't be caught by any
/// self-test inside the writer/CLI itself.
#[test]
fn write_evidence_package_round_trips_through_the_pin_loader() {
    let raw_root = TempDir::new().unwrap();
    let raw = RawStore::new(raw_root.path());
    let source = store_source(&raw);
    let normalized = stage4a_entry(&raw, &source);
    let _actions = action_entry(&raw, false);

    let package_root = TempDir::new().unwrap();
    let out_dir = package_root.path().join("pkg");
    let manifest_hash = write_evidence_package(
        &raw,
        &normalized,
        &schedule_bytes(ContentHash::from_bytes(b"calendar")),
        &listing_bytes_with_snapshot_hash(listing_hash()),
        &pit_policy_bytes(),
        &out_dir,
    )
    .unwrap();

    let evidence =
        load_with_approved_pin_for_test(&raw, &normalized, &out_dir, &manifest_hash).unwrap();
    let candidate = build_range_canonical_candidate(&raw, &normalized, &evidence).unwrap();
    assert_eq!(candidate.action_coverage_file_count(), 7);
    assert!(candidate.action_coverage_is_zero_result());
    assert_eq!(candidate.bars.len(), 11);
}

#[test]
fn write_evidence_package_fails_closed_on_schedule_lineage_mismatch() {
    let raw_root = TempDir::new().unwrap();
    let raw = RawStore::new(raw_root.path());
    let source = store_source(&raw);
    let normalized = stage4a_entry(&raw, &source);
    let _actions = action_entry(&raw, false);

    let package_root = TempDir::new().unwrap();
    let out_dir = package_root.path().join("pkg");
    let result = write_evidence_package(
        &raw,
        &normalized,
        &schedule_bytes(ContentHash::from_bytes(b"not-the-lineage-calendar")),
        &listing_bytes_with_snapshot_hash(listing_hash()),
        &pit_policy_bytes(),
        &out_dir,
    );
    assert!(matches!(
        result,
        Err(RangeCanonicalError::UnsupportedHistoricalSessionSchedule { .. })
    ));
    assert!(!out_dir.exists() || fs::read_dir(&out_dir).unwrap().next().is_none());
}

#[test]
fn write_evidence_package_fails_closed_on_listing_lineage_mismatch() {
    let raw_root = TempDir::new().unwrap();
    let raw = RawStore::new(raw_root.path());
    let source = store_source(&raw);
    let normalized = stage4a_entry(&raw, &source);
    let _actions = action_entry(&raw, false);

    let package_root = TempDir::new().unwrap();
    let out_dir = package_root.path().join("pkg");
    let result = write_evidence_package(
        &raw,
        &normalized,
        &schedule_bytes(ContentHash::from_bytes(b"calendar")),
        &listing_bytes_with_snapshot_hash(ContentHash::from_bytes(b"not-the-lineage-listing")),
        &pit_policy_bytes(),
        &out_dir,
    );
    assert!(matches!(
        result,
        Err(RangeCanonicalError::MissingListingMasterEvidence { .. })
    ));
    assert!(!out_dir.exists() || fs::read_dir(&out_dir).unwrap().next().is_none());
}

/// `load_action_evidence` accepts a nonempty non-`bonus-issue` KSD response
/// by mapping it to `RangeAction::Unsupported`; only `validate_actions`
/// rejects it, and the pin loader runs both.  A writer that ran only the
/// first would print a `manifest_sha256` for a package the loader can never
/// accept, and that hash — once reviewed and committed to the approved
/// registry — would be permanently dead gate evidence.  A whole-market KSD
/// dividend response is nonempty for almost any multi-day range, so this is
/// the expected shape of real data, not an edge case.
#[test]
fn write_evidence_package_refuses_a_nonempty_unsupported_action_response() {
    let raw_root = TempDir::new().unwrap();
    let raw = RawStore::new(raw_root.path());
    let source = store_source(&raw);
    let normalized = stage4a_entry(&raw, &source);
    let _actions = action_entry_with_nonempty_unsupported(&raw, "dividend");

    let package_root = TempDir::new().unwrap();
    let out_dir = package_root.path().join("pkg");
    let result = write_evidence_package(
        &raw,
        &normalized,
        &schedule_bytes(ContentHash::from_bytes(b"calendar")),
        &listing_bytes_with_snapshot_hash(listing_hash()),
        &pit_policy_bytes(),
        &out_dir,
    );
    assert!(
        matches!(&result, Err(RangeCanonicalError::UnsupportedAction { kind }) if kind == "dividend"),
        "unexpected writer outcome: {result:?}"
    );
    assert!(!out_dir.exists() || fs::read_dir(&out_dir).unwrap().next().is_none());
}

/// Two Raw batches that both cover the range exactly must never be resolved
/// silently: the writer has no authority to choose which one a reviewer will
/// approve.
#[test]
fn write_evidence_package_refuses_an_ambiguous_action_batch_pin() {
    let raw_root = TempDir::new().unwrap();
    let raw = RawStore::new(raw_root.path());
    let source = store_source(&raw);
    let normalized = stage4a_entry(&raw, &source);
    let _first = action_entry(&raw, false);
    let _second = action_entry(&raw, false);

    let package_root = TempDir::new().unwrap();
    let out_dir = package_root.path().join("pkg");
    let result = write_evidence_package(
        &raw,
        &normalized,
        &schedule_bytes(ContentHash::from_bytes(b"calendar")),
        &listing_bytes_with_snapshot_hash(listing_hash()),
        &pit_policy_bytes(),
        &out_dir,
    );
    assert!(
        matches!(
            &result,
            Err(RangeCanonicalError::MissingActionEvidence { .. })
        ),
        "unexpected writer outcome: {result:?}"
    );
    assert!(!out_dir.exists() || fs::read_dir(&out_dir).unwrap().next().is_none());
}

/// A batch holding the seven KSD responses plus anything else — the daily
/// EOD bundle, or a paginated KSD range batch — can never be loaded, because
/// `load_action_evidence` requires the pinned batch to hold exactly seven
/// files.  The writer must therefore not treat it as a candidate at all.
#[test]
fn write_evidence_package_ignores_an_action_batch_with_an_extra_file() {
    let raw_root = TempDir::new().unwrap();
    let raw = RawStore::new(raw_root.path());
    let source = store_source(&raw);
    let normalized = stage4a_entry(&raw, &source);
    let bundle = action_entry_with_extra_file(&raw);
    assert_eq!(bundle.files.len(), REQUIRED_ACTION_KINDS.len() + 1);

    let package_root = TempDir::new().unwrap();
    let out_dir = package_root.path().join("pkg");
    let result = write_evidence_package(
        &raw,
        &normalized,
        &schedule_bytes(ContentHash::from_bytes(b"calendar")),
        &listing_bytes_with_snapshot_hash(listing_hash()),
        &pit_policy_bytes(),
        &out_dir,
    );
    assert!(
        matches!(
            &result,
            Err(RangeCanonicalError::MissingActionEvidence { .. })
        ),
        "unexpected writer outcome: {result:?}"
    );
    assert!(!out_dir.exists() || fs::read_dir(&out_dir).unwrap().next().is_none());
}

/// Directory creation follows symlinks while `safe_package_root` rejects
/// them, so the parent chain must be verified before anything is created.
/// Checking afterwards would already have created a directory outside the
/// intended tree.
#[test]
fn write_evidence_package_creates_nothing_under_a_symlinked_out_parent() {
    let raw_root = TempDir::new().unwrap();
    let raw = RawStore::new(raw_root.path());
    let source = store_source(&raw);
    let normalized = stage4a_entry(&raw, &source);
    let _actions = action_entry(&raw, false);

    let package_root = TempDir::new().unwrap();
    let elsewhere = TempDir::new().unwrap();
    let staging_link = package_root.path().join("staging");
    std::os::unix::fs::symlink(elsewhere.path(), &staging_link).unwrap();
    let out_dir = staging_link.join("pkg");

    let result = write_evidence_package(
        &raw,
        &normalized,
        &schedule_bytes(ContentHash::from_bytes(b"calendar")),
        &listing_bytes_with_snapshot_hash(listing_hash()),
        &pit_policy_bytes(),
        &out_dir,
    );
    assert!(
        matches!(&result, Err(RangeCanonicalError::UnsafeEvidencePath { .. })),
        "unexpected writer outcome: {result:?}"
    );
    assert!(fs::read_dir(elsewhere.path()).unwrap().next().is_none());
}

/// The package directory is published by renaming the staging directory onto
/// it, so an `--out` the operator has already created (empty) must still
/// work, while a non-empty one must fail closed rather than be merged into.
#[test]
fn an_existing_empty_out_dir_is_published_and_a_nonempty_one_is_refused() {
    let raw_root = TempDir::new().unwrap();
    let raw = RawStore::new(raw_root.path());
    let source = store_source(&raw);
    let normalized = stage4a_entry(&raw, &source);
    let _actions = action_entry(&raw, false);

    let package_root = TempDir::new().unwrap();
    let out_dir = package_root.path().join("pkg");
    fs::create_dir(&out_dir).unwrap();
    let manifest_hash = write_evidence_package(
        &raw,
        &normalized,
        &schedule_bytes(ContentHash::from_bytes(b"calendar")),
        &listing_bytes_with_snapshot_hash(listing_hash()),
        &pit_policy_bytes(),
        &out_dir,
    )
    .unwrap();
    load_with_approved_pin_for_test(&raw, &normalized, &out_dir, &manifest_hash).unwrap();

    // The same `--out` is now non-empty, so a second run must refuse it
    // instead of writing beside or over the first package.
    let result = write_evidence_package(
        &raw,
        &normalized,
        &schedule_bytes(ContentHash::from_bytes(b"calendar")),
        &listing_bytes_with_snapshot_hash(listing_hash()),
        &pit_policy_bytes(),
        &out_dir,
    );
    assert!(
        matches!(&result, Err(RangeCanonicalError::EvidencePackage { .. })),
        "unexpected writer outcome: {result:?}"
    );
    let mut names = fs::read_dir(&out_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(
        names,
        ["listing.json", "manifest.json", "pit.json", "schedule.json"]
    );
}

/// A failure after the first artifact is written must leave no partial
/// package at `--out`, and no residue that would make an otherwise valid
/// retry into the same `--out` fail on `create_new`.
#[test]
fn a_failed_write_leaves_no_residue_and_a_retry_into_the_same_out_dir_succeeds() {
    let raw_root = TempDir::new().unwrap();
    let raw = RawStore::new(raw_root.path());
    let source = store_source(&raw);
    let normalized = stage4a_entry(&raw, &source);
    let _actions = action_entry(&raw, false);

    let package_root = TempDir::new().unwrap();
    let out_dir = package_root.path().join("pkg");
    // Fails only at the PIT approval, i.e. after the schedule and listing
    // evidence has already been fully validated.
    let mut rejected_pit: Value = serde_json::from_slice(&pit_policy_bytes()).unwrap();
    rejected_pit["approved"] = json!(false);
    let result = write_evidence_package(
        &raw,
        &normalized,
        &schedule_bytes(ContentHash::from_bytes(b"calendar")),
        &listing_bytes_with_snapshot_hash(listing_hash()),
        &serde_json::to_vec(&rejected_pit).unwrap(),
        &out_dir,
    );
    assert!(
        matches!(
            &result,
            Err(RangeCanonicalError::NonStrictPitNotApproved { .. })
        ),
        "unexpected writer outcome: {result:?}"
    );
    assert!(!out_dir.exists());
    assert!(fs::read_dir(package_root.path()).unwrap().next().is_none());

    let manifest_hash = write_evidence_package(
        &raw,
        &normalized,
        &schedule_bytes(ContentHash::from_bytes(b"calendar")),
        &listing_bytes_with_snapshot_hash(listing_hash()),
        &pit_policy_bytes(),
        &out_dir,
    )
    .unwrap();
    load_with_approved_pin_for_test(&raw, &normalized, &out_dir, &manifest_hash).unwrap();
}

/// The likely real layout: one dedicated seven-file KSD range batch beside a
/// daily EOD bundle whose single-day KSD calls match the same range.  Only
/// the seven-file batch is loadable, so exactly one candidate exists and the
/// pin must resolve to it rather than being reported as ambiguous.
#[test]
fn write_evidence_package_pins_the_seven_file_batch_beside_an_eod_bundle() {
    let raw_root = TempDir::new().unwrap();
    let raw = RawStore::new(raw_root.path());
    let source = store_source(&raw);
    let normalized = stage4a_entry(&raw, &source);
    let seven = action_entry(&raw, false);
    let bundle = action_entry_with_extra_file(&raw);
    assert_ne!(seven.batch_id, bundle.batch_id);

    let package_root = TempDir::new().unwrap();
    let out_dir = package_root.path().join("pkg");
    let manifest_hash = write_evidence_package(
        &raw,
        &normalized,
        &schedule_bytes(ContentHash::from_bytes(b"calendar")),
        &listing_bytes_with_snapshot_hash(listing_hash()),
        &pit_policy_bytes(),
        &out_dir,
    )
    .unwrap();

    let manifest: Value =
        serde_json::from_slice(&fs::read(out_dir.join("manifest.json")).unwrap()).unwrap();
    let actions = manifest["actions"].as_array().unwrap();
    assert_eq!(actions.len(), REQUIRED_ACTION_KINDS.len());
    let expected = serde_json::to_value(seven.batch_id).unwrap();
    for action in actions {
        assert_eq!(action["raw_batch_id"], expected);
    }
    load_with_approved_pin_for_test(&raw, &normalized, &out_dir, &manifest_hash).unwrap();
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
