use std::collections::HashMap;
use std::error::Error as _;
use std::os::unix::fs::PermissionsExt;
use std::sync::Mutex;

use async_trait::async_trait;
use collectors::{
    FailureClass, PipelineError, PipelineStage, PublicationSink, PublicationState, PublishOutcome,
    RecoveryScope, SinkError, ensure_kis_calendar_source, ingest_normalize_publish_kis,
    ingest_normalize_publish_kis_historical_range,
    ingest_normalize_publish_kis_with_calendar_source, recover_kis_normalization,
    recover_unpublished_normalized_for_date, recover_unpublished_scope,
};
use domain::{BatchId, TradingDate, UtcTimestamp};
use kis_client::{KisError, MarketDataReply};
use market_data::contract::{
    FetchMode, MARKET_KR, PROVIDER_KIS, PROVIDER_KIS_CALENDAR, PROVIDER_KIS_NORMALIZED,
    RawEnvelope, RequestMetadata, ResponseKind,
};
use market_data::ingest::{IngestRequest, ingest_kis_bundle, ingest_kis_calendar_with_batch_id};
use market_data::normalize::NormalizeError;
use market_data::providers::kis::KR_ETF_CORE_SYMBOLS;
use market_data::providers::kis::{KisProvider, KisRead};
use market_data::publication::PublicationBundle;
use market_data::storage::{BatchSpec, ManifestEntry, RawStore, StoreError};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;

const TARGET_DATE: &str = "2026-08-14";
const OTHER_DATE: &str = "2026-08-13";
const RANGE_SECOND_DATE: &str = "2026-08-15";
const RETRIEVED_AT: &str = "2026-08-14T08:00:00Z";

struct Wire {
    kind: ResponseKind,
    file_name: String,
    endpoint: String,
    query: Vec<(String, String)>,
    bytes: Vec<u8>,
}

fn seed_kis_store(mutate: impl FnOnce(&mut Vec<Wire>)) -> (TempDir, RawStore, ManifestEntry) {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RawStore::new(temp.path().join("data"));
    let entry = append_kis_batch(&store, TARGET_DATE, mutate);
    (temp, store, entry)
}

fn append_kis_batch(
    store: &RawStore,
    date: &str,
    mutate: impl FnOnce(&mut Vec<Wire>),
) -> ManifestEntry {
    let mut wires = valid_wires_for_date(date);
    mutate(&mut wires);
    let batch_id = BatchId::generate();
    let retrieved_at = UtcTimestamp::parse_rfc3339(RETRIEVED_AT).expect("timestamp");
    let envelopes = wires
        .into_iter()
        .map(|wire| {
            RawEnvelope::new(
                batch_id,
                wire.kind,
                wire.file_name,
                wire.bytes,
                retrieved_at,
                RequestMetadata {
                    endpoint: wire.endpoint,
                    query: wire.query,
                    headers: Vec::new(),
                    mode: FetchMode::Credentialed,
                },
            )
        })
        .collect::<Vec<_>>();
    let date = TradingDate::parse(date).expect("date");
    store
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
        .expect("KIS source batch")
}

fn valid_wires_for_date(date: &str) -> Vec<Wire> {
    let mut wires = Vec::new();
    for symbol in KR_ETF_CORE_SYMBOLS {
        wires.push(Wire {
            kind: ResponseKind::Bars,
            file_name: format!("daily-bars-{symbol}.json"),
            endpoint: "/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice".into(),
            query: vec![("FID_INPUT_ISCD".into(), symbol.into())],
            bytes: serde_json::to_vec(&json!({
                "rt_cd": "0",
                "output1": {
                    "hts_kor_isnm": format!("ETF {symbol}"),
                    "stck_shrn_iscd": symbol
                },
                "output2": [{
                    "stck_bsop_date": date,
                    "stck_oprc": "100.00",
                    "stck_hgpr": "102.00",
                    "stck_lwpr": "99.00",
                    "stck_clpr": "101.00",
                    "acml_vol": "1300",
                    "acml_tr_pbmn": "131300"
                }]
            }))
            .expect("bars"),
        });
        wires.push(Wire {
            kind: ResponseKind::Reference,
            file_name: format!("reference-{symbol}.json"),
            endpoint: "/uapi/domestic-stock/v1/quotations/inquire-price".into(),
            query: vec![("FID_INPUT_ISCD".into(), symbol.into())],
            bytes: serde_json::to_vec(&json!({
                "rt_cd": "0",
                "output": {
                    "stck_shrn_iscd": symbol
                }
            }))
            .expect("reference"),
        });
    }
    wires.push(Wire {
        kind: ResponseKind::Calendar,
        file_name: "calendar.json".into(),
        endpoint: "/uapi/domestic-stock/v1/quotations/chk-holiday".into(),
        query: vec![("BASS_DT".into(), date.into())],
        bytes: serde_json::to_vec(&json!({
            "rt_cd": "0",
            "output": [{"bass_dt": date, "opnd_yn": "Y"}]
        }))
        .expect("calendar"),
    });
    for (index, endpoint) in [
        "/uapi/domestic-stock/v1/ksdinfo/paidin-capin",
        "/uapi/domestic-stock/v1/ksdinfo/bonus-issue",
        "/uapi/domestic-stock/v1/ksdinfo/dividend",
        "/uapi/domestic-stock/v1/ksdinfo/merger-split",
        "/uapi/domestic-stock/v1/ksdinfo/rev-split",
        "/uapi/domestic-stock/v1/ksdinfo/cap-dcrs",
    ]
    .into_iter()
    .enumerate()
    {
        wires.push(Wire {
            kind: ResponseKind::CorporateActions,
            file_name: format!("corporate-actions-{index}.json"),
            endpoint: endpoint.into(),
            query: vec![("F_DT".into(), date.into()), ("T_DT".into(), date.into())],
            bytes: br#"{"rt_cd":"0","output1":[]}"#.to_vec(),
        });
    }
    wires
}

#[derive(Debug, Clone)]
struct FakeKisRead {
    calls: Arc<AtomicUsize>,
    calendar_calls: Arc<AtomicUsize>,
    malformed_first_bar: bool,
    calendar_range: bool,
}

impl FakeKisRead {
    fn new(malformed_first_bar: bool) -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
            calendar_calls: Arc::new(AtomicUsize::new(0)),
            malformed_first_bar,
            calendar_range: false,
        }
    }

    fn with_calendar_range() -> Self {
        Self {
            calendar_range: true,
            ..Self::new(false)
        }
    }

    fn query_value<'a>(query: &'a [(String, String)], key: &str) -> &'a str {
        query
            .iter()
            .find(|(query_key, _)| query_key == key)
            .map(|(_, value)| value.as_str())
            .unwrap_or_default()
    }
}

impl KisRead for FakeKisRead {
    async fn get(
        &self,
        path: &str,
        _tr_id: &str,
        query: &[(String, String)],
        _continuation: Option<&str>,
    ) -> Result<MarketDataReply, KisError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let symbol = Self::query_value(query, "FID_INPUT_ISCD");
        let bar_date = {
            let value = Self::query_value(query, "FID_INPUT_DATE_1");
            if value.is_empty() { "20260814" } else { value }
        };
        let body = if path.ends_with("inquire-daily-itemchartprice") {
            let mut row = json!({
                "stck_bsop_date": bar_date,
                "stck_oprc": "100.00",
                "stck_hgpr": "102.00",
                "stck_lwpr": "99.00",
                "stck_clpr": "101.00",
                "acml_vol": "1300",
                "acml_tr_pbmn": "131300"
            });
            if self.malformed_first_bar && symbol == KR_ETF_CORE_SYMBOLS[0] {
                row.as_object_mut().expect("bar row").remove("stck_clpr");
            }
            json!({
                "rt_cd": "0",
                "output1": {
                    "hts_kor_isnm": format!("ETF {symbol}"),
                    "stck_shrn_iscd": symbol
                },
                "output2": [row]
            })
        } else if path.ends_with("inquire-price") {
            json!({
                "rt_cd": "0",
                "output": {"stck_shrn_iscd": symbol}
            })
        } else if path.ends_with("chk-holiday") {
            self.calendar_calls.fetch_add(1, Ordering::SeqCst);
            if self.calendar_range {
                json!({
                    "rt_cd": "0",
                    "output": [
                        {"bass_dt": "20260814", "opnd_yn": "Y"},
                        {"bass_dt": "20260815", "opnd_yn": "Y"}
                    ]
                })
            } else {
                json!({
                    "rt_cd": "0",
                    "output": [{"bass_dt": "20260814", "opnd_yn": "Y"}]
                })
            }
        } else {
            json!({"rt_cd": "0", "output1": []})
        };
        // KSD schedule endpoints are documented as single-page.  A missing
        // terminal marker is therefore the expected fixture shape; a
        // non-empty marker is rejected by the provider instead of followed.
        Ok(MarketDataReply {
            body: serde_json::to_vec(&body).expect("fake KIS JSON"),
            continuation: None,
        })
    }
}

#[derive(Default)]
struct ReplaySink {
    states: Mutex<HashMap<BatchId, PublicationState>>,
    calls: Mutex<Vec<(BatchId, String, PublishOutcome)>>,
}

#[async_trait]
impl PublicationSink for ReplaySink {
    async fn publication_state(&self, batch_id: BatchId) -> Result<PublicationState, SinkError> {
        Ok(self
            .states
            .lock()
            .expect("states lock")
            .get(&batch_id)
            .copied()
            .unwrap_or(PublicationState::Missing))
    }

    async fn publish(&self, bundle: &PublicationBundle) -> Result<PublishOutcome, SinkError> {
        assert_eq!(bundle.provider, PROVIDER_KIS_NORMALIZED);
        assert_eq!(bundle.market, MARKET_KR);
        let mut states = self.states.lock().expect("states lock");
        let outcome = if states.insert(bundle.source_batch_id, PublicationState::Complete)
            == Some(PublicationState::Complete)
        {
            PublishOutcome::AlreadyPublished
        } else {
            PublishOutcome::Published
        };
        self.calls.lock().expect("calls lock").push((
            bundle.source_batch_id,
            bundle.provider.clone(),
            outcome,
        ));
        Ok(outcome)
    }

    async fn has_eod(&self, _date: TradingDate) -> Result<bool, SinkError> {
        Ok(false)
    }
}

#[test]
fn kis_recovery_reconciles_wire_source_and_is_idempotent() {
    let (_temp, store, source) = seed_kis_store(|_| {});
    let manifest_path = store.manifest_path(PROVIDER_KIS, MARKET_KR);
    std::fs::remove_file(&manifest_path).expect("simulate manifest crash tail");

    let first = recover_kis_normalization(&store).expect("normalize durable KIS source");
    assert_eq!(first.outcomes.len(), 1);
    assert_eq!(first.outcomes[0].source_batch_id, source.batch_id);
    assert_eq!(first.outcomes[0].entry.provider, PROVIDER_KIS_NORMALIZED);
    assert_eq!(first.outcomes[0].entry.files.len(), 4);
    let second = recover_kis_normalization(&store).expect("idempotent normalization");
    assert_eq!(second.outcomes, first.outcomes);
    assert_eq!(
        store
            .read_reconciled_manifest(PROVIDER_KIS_NORMALIZED, MARKET_KR)
            .expect("normalized manifest")
            .len(),
        1
    );
    assert_eq!(
        store
            .read_reconciled_manifest(PROVIDER_KIS, MARKET_KR)
            .expect("wire manifest")
            .len(),
        1
    );

    let wire = store
        .read_manifest(PROVIDER_KIS, MARKET_KR)
        .expect("wire manifest")
        .pop()
        .expect("wire source");
    assert!(matches!(
        PublicationBundle::from_raw(&store, &wire),
        Err(market_data::PublicationError::UnsupportedManifestScope { .. })
    ));
}

#[test]
fn kis_recovery_ignores_corporate_action_evidence_only_batches() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RawStore::new(temp.path().join("data"));
    let evidence = append_kis_batch(&store, TARGET_DATE, |wires| {
        wires.retain(|wire| wire.kind == ResponseKind::CorporateActions);
    });
    let eod = append_kis_batch(&store, OTHER_DATE, |_| {});

    let normalized = recover_kis_normalization(&store).expect("normalize EOD sources only");
    assert_eq!(normalized.outcomes.len(), 1);
    assert_eq!(normalized.outcomes[0].source_batch_id, eod.batch_id);
    assert_ne!(normalized.outcomes[0].source_batch_id, evidence.batch_id);
    assert_eq!(
        store
            .read_reconciled_manifest(PROVIDER_KIS, MARKET_KR)
            .expect("both immutable KIS sources remain visible")
            .len(),
        2
    );
    assert_eq!(
        store
            .read_reconciled_manifest(PROVIDER_KIS_NORMALIZED, MARKET_KR)
            .expect("only the EOD source is normalized")
            .len(),
        1
    );
}

#[tokio::test]
async fn normalized_scope_recovery_replays_already_published_without_new_manifest() {
    let (_temp, store, _source) = seed_kis_store(|_| {});
    let normalized = recover_kis_normalization(&store)
        .expect("normalize")
        .outcomes
        .pop()
        .expect("normalized outcome");
    let sink = ReplaySink::default();

    let first = recover_unpublished_scope(&store, &sink, RecoveryScope::KisNormalized)
        .await
        .expect("first normalized publication recovery");
    let second = recover_unpublished_scope(&store, &sink, RecoveryScope::KisNormalized)
        .await
        .expect("second normalized publication recovery");

    assert_eq!(first.recovered, vec![normalized.entry.batch_id]);
    assert!(first.skipped.is_empty());
    assert!(second.recovered.is_empty());
    assert_eq!(second.skipped, vec![normalized.entry.batch_id]);
    assert_eq!(
        store
            .read_manifest(PROVIDER_KIS_NORMALIZED, MARKET_KR)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        sink.calls.lock().unwrap().as_slice(),
        &[
            (
                normalized.entry.batch_id,
                PROVIDER_KIS_NORMALIZED.into(),
                PublishOutcome::Published
            ),
            (
                normalized.entry.batch_id,
                PROVIDER_KIS_NORMALIZED.into(),
                PublishOutcome::AlreadyPublished
            ),
        ]
    );
}

#[tokio::test]
async fn eod_first_acquisition_claims_one_calendar_and_reuses_it_for_normalization() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RawStore::new(temp.path().join("data"));
    std::fs::create_dir_all(store.root().join("raw")).expect("raw root");
    let reader = FakeKisRead::new(false);
    let provider = KisProvider::kr_etf_core(reader.clone());
    let request = IngestRequest::new(
        MARKET_KR.to_owned(),
        TradingDate::parse(TARGET_DATE).expect("date"),
        UtcTimestamp::parse_rfc3339("2026-08-14T08:00:00Z").expect("now"),
    );
    let source =
        ensure_kis_calendar_source(&store, &provider, &request, Some("entitlement://kis-live"))
            .await
            .expect("calendar claim and capture")
            .expect("dedicated calendar source");
    let sink = ReplaySink::default();
    let outcome = ingest_normalize_publish_kis_with_calendar_source(
        &store,
        &provider,
        &request,
        Some("entitlement://kis-live"),
        &sink,
        Some(&source),
    )
    .await
    .expect("EOD normalization and publication");

    // One calendar call for the durable daily source, then exactly the 29
    // non-calendar KIS EOD calls. A second chk-holiday request would make 31.
    assert_eq!(reader.calls.load(Ordering::SeqCst), 30);
    assert_eq!(
        store
            .read_reconciled_manifest("kis-calendar", MARKET_KR)
            .expect("calendar manifest")
            .len(),
        1
    );
    assert_eq!(outcome.manifest.files.len(), 4);
    assert_eq!(sink.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn bootstrap_first_duplicate_execution_reuses_the_exact_calendar_batch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RawStore::new(temp.path().join("data"));
    std::fs::create_dir_all(store.root().join("raw")).expect("raw root");
    let reader = FakeKisRead::new(false);
    let provider = KisProvider::kr_etf_core(reader.clone());
    let request = IngestRequest::new(
        MARKET_KR.to_owned(),
        TradingDate::parse(TARGET_DATE).expect("date"),
        UtcTimestamp::parse_rfc3339("2026-08-14T08:00:00Z").expect("now"),
    );

    let first =
        ensure_kis_calendar_source(&store, &provider, &request, Some("entitlement://kis-live"))
            .await
            .expect("first bootstrap")
            .expect("dedicated source");
    let second =
        ensure_kis_calendar_source(&store, &provider, &request, Some("entitlement://kis-live"))
            .await
            .expect("duplicate bootstrap")
            .expect("same dedicated source");

    assert_eq!(second, first);
    assert_eq!(reader.calendar_calls.load(Ordering::SeqCst), 1);
    assert_eq!(reader.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        store
            .read_reconciled_manifest(PROVIDER_KIS_CALENDAR, MARKET_KR)
            .expect("calendar manifest")
            .len(),
        1
    );
}

#[tokio::test]
async fn concurrent_calendar_contenders_share_one_claim_or_fail_busy_without_recapture() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RawStore::new(temp.path().join("data"));
    std::fs::create_dir_all(store.root().join("raw")).expect("raw root");
    let reader = FakeKisRead::new(false);
    let provider = KisProvider::kr_etf_core(reader.clone());
    let request = IngestRequest::new(
        MARKET_KR.to_owned(),
        TradingDate::parse(TARGET_DATE).expect("date"),
        UtcTimestamp::parse_rfc3339("2026-08-14T08:00:00Z").expect("now"),
    );

    let (first, second) = tokio::join!(
        ensure_kis_calendar_source(&store, &provider, &request, Some("entitlement://kis-live"),),
        ensure_kis_calendar_source(&store, &provider, &request, Some("entitlement://kis-live"),)
    );
    let successful = [first, second]
        .into_iter()
        .filter_map(|result| result.ok().flatten())
        .collect::<Vec<_>>();
    assert!(
        !successful.is_empty(),
        "one contender must acquire the source"
    );
    assert!(successful.iter().all(|source| *source == successful[0]));
    assert_eq!(reader.calendar_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        store
            .read_reconciled_manifest(PROVIDER_KIS_CALENDAR, MARKET_KR)
            .expect("calendar manifest")
            .len(),
        1
    );
}

#[tokio::test]
async fn claim_only_state_is_indeterminate_and_never_recaptures() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RawStore::new(temp.path().join("data"));
    let raw_root = store.root().join("raw");
    let claim_root = raw_root.join(".calendar-bootstrap");
    std::fs::create_dir_all(&claim_root).expect("claim root");
    std::fs::set_permissions(&claim_root, std::fs::Permissions::from_mode(0o700))
        .expect("claim permissions");
    let claim_path = claim_root.join(format!("{TARGET_DATE}.json"));
    std::fs::write(
        &claim_path,
        serde_json::to_vec(&json!({
            "schema_version": 1,
            "date": TARGET_DATE,
            "source_batch_id": BatchId::generate(),
        }))
        .expect("claim json"),
    )
    .expect("claim file");
    std::fs::set_permissions(&claim_path, std::fs::Permissions::from_mode(0o600))
        .expect("claim file permissions");

    let reader = FakeKisRead::new(false);
    let provider = KisProvider::kr_etf_core(reader.clone());
    let request = IngestRequest::new(
        MARKET_KR.to_owned(),
        TradingDate::parse(TARGET_DATE).expect("date"),
        UtcTimestamp::parse_rfc3339("2026-08-14T08:00:00Z").expect("now"),
    );
    let error =
        ensure_kis_calendar_source(&store, &provider, &request, Some("entitlement://kis-live"))
            .await
            .expect_err("claim without committed source must stop");
    assert!(
        error
            .to_string()
            .contains("KIS_CALENDAR_SOURCE_INDETERMINATE")
    );
    assert_eq!(reader.calls.load(Ordering::SeqCst), 0);
    assert_eq!(reader.calendar_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn committed_calendar_raw_reconciles_after_manifest_interrupt_without_network() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RawStore::new(temp.path().join("data"));
    let reader = FakeKisRead::new(false);
    let provider = KisProvider::kr_etf_core(reader.clone());
    let request = IngestRequest::new(
        MARKET_KR.to_owned(),
        TradingDate::parse(TARGET_DATE).expect("date"),
        UtcTimestamp::parse_rfc3339("2026-08-14T08:00:00Z").expect("now"),
    );
    let batch_id = BatchId::generate();
    let captured = ingest_kis_calendar_with_batch_id(
        &store,
        &provider,
        &request,
        Some("entitlement://kis-live"),
        batch_id,
    )
    .await
    .expect("committed calendar raw");
    std::fs::remove_file(store.manifest_path(PROVIDER_KIS_CALENDAR, MARKET_KR))
        .expect("interrupt manifest publication");

    let recovered =
        ensure_kis_calendar_source(&store, &provider, &request, Some("entitlement://kis-live"))
            .await
            .expect("reconcile committed raw")
            .expect("calendar source");
    assert_eq!(recovered.batch_id, captured.entry.batch_id);
    assert_eq!(reader.calendar_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        store
            .read_reconciled_manifest(PROVIDER_KIS_CALENDAR, MARKET_KR)
            .expect("reconciled calendar manifest"),
        vec![captured.entry]
    );
}

#[tokio::test]
async fn retained_claim_uuid_is_reused_without_inventing_a_second_calendar_source() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RawStore::new(temp.path().join("data"));
    let reader = FakeKisRead::new(false);
    let provider = KisProvider::kr_etf_core(reader.clone());
    let request = IngestRequest::new(
        MARKET_KR.to_owned(),
        TradingDate::parse(TARGET_DATE).expect("date"),
        UtcTimestamp::parse_rfc3339("2026-08-14T08:00:00Z").expect("now"),
    );
    let retained_batch_id = BatchId::generate();
    let captured = ingest_kis_calendar_with_batch_id(
        &store,
        &provider,
        &request,
        Some("entitlement://kis-live"),
        retained_batch_id,
    )
    .await
    .expect("calendar source capture");
    let claim_root = store.root().join("raw/.calendar-bootstrap");
    std::fs::create_dir_all(&claim_root).expect("claim root");
    std::fs::set_permissions(&claim_root, std::fs::Permissions::from_mode(0o700))
        .expect("claim permissions");
    let claim_path = claim_root.join(format!("{TARGET_DATE}.json"));
    std::fs::write(
        &claim_path,
        serde_json::to_vec(&json!({
            "schema_version": 1,
            "date": TARGET_DATE,
            "source_batch_id": retained_batch_id,
        }))
        .expect("claim JSON"),
    )
    .expect("claim file");
    std::fs::set_permissions(&claim_path, std::fs::Permissions::from_mode(0o600))
        .expect("claim file permissions");

    let source =
        ensure_kis_calendar_source(&store, &provider, &request, Some("entitlement://kis-live"))
            .await
            .expect("retained claim recovery")
            .expect("retained source");
    assert_eq!(source.batch_id, retained_batch_id);
    assert_eq!(source, captured.entry);
    assert_eq!(reader.calendar_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        store
            .read_reconciled_manifest(PROVIDER_KIS_CALENDAR, MARKET_KR)
            .expect("calendar manifest")
            .len(),
        1
    );
}

#[tokio::test]
async fn tampered_calendar_source_fails_closed_before_non_calendar_fetches() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RawStore::new(temp.path().join("data"));
    std::fs::create_dir_all(store.root().join("raw")).expect("raw root");
    let reader = FakeKisRead::new(false);
    let provider = KisProvider::kr_etf_core(reader.clone());
    let request = IngestRequest::new(
        MARKET_KR.to_owned(),
        TradingDate::parse(TARGET_DATE).expect("date"),
        UtcTimestamp::parse_rfc3339("2026-08-14T08:00:00Z").expect("now"),
    );
    let source =
        ensure_kis_calendar_source(&store, &provider, &request, Some("entitlement://kis-live"))
            .await
            .expect("capture calendar")
            .expect("dedicated source");
    let source_path = store
        .batch_dir(
            PROVIDER_KIS_CALENDAR,
            MARKET_KR,
            &source.date,
            &source.batch_id,
        )
        .join(&source.files[0].file_name);
    std::fs::write(&source_path, b"tampered calendar bytes").expect("tamper source");

    let sink = ReplaySink::default();
    let error = ingest_normalize_publish_kis_with_calendar_source(
        &store,
        &provider,
        &request,
        Some("entitlement://kis-live"),
        &sink,
        Some(&source),
    )
    .await
    .expect_err("tampered source must fail closed");
    assert!(matches!(error, PipelineError::Ingest { .. }));
    assert_eq!(reader.calendar_calls.load(Ordering::SeqCst), 1);
    assert_eq!(reader.calls.load(Ordering::SeqCst), 1);
    assert!(sink.calls.lock().expect("sink calls").is_empty());
}

#[tokio::test]
async fn historical_range_snapshot_keeps_one_calendar_request_for_covered_targets() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RawStore::new(temp.path().join("data"));
    let reader = FakeKisRead::with_calendar_range();
    let provider = KisProvider::kr_etf_core(reader.clone()).with_calendar_snapshot_cache();
    let sink = ReplaySink::default();

    for (date, retrieved_at) in [
        (TARGET_DATE, "2026-08-14T08:00:00Z"),
        (RANGE_SECOND_DATE, "2026-08-15T08:00:00Z"),
    ] {
        let request = IngestRequest::new(
            MARKET_KR.to_owned(),
            TradingDate::parse(date).expect("range date"),
            UtcTimestamp::parse_rfc3339(retrieved_at).expect("range retrieval"),
        );
        let outcome = ingest_normalize_publish_kis_historical_range(
            &store,
            &provider,
            &request,
            Some("entitlement://kis-live"),
            &sink,
        )
        .await
        .expect("historical range EOD publication");
        assert_eq!(outcome.manifest.files.len(), 4);
    }

    assert_eq!(reader.calendar_calls.load(Ordering::SeqCst), 1);
    assert_eq!(reader.calls.load(Ordering::SeqCst), 59);
}

#[tokio::test]
async fn composite_raw_normalization_and_publication_recovery_is_idempotent() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RawStore::new(temp.path().join("data"));
    let reader = FakeKisRead::new(false);
    let calls = Arc::clone(&reader.calls);
    let provider = KisProvider::kr_etf_core(reader);
    let request = IngestRequest::new(
        MARKET_KR.to_owned(),
        TradingDate::parse(TARGET_DATE).expect("date"),
        UtcTimestamp::parse_rfc3339(RETRIEVED_AT).expect("retrieved at"),
    );
    let sink = ReplaySink::default();
    let ingested = ingest_kis_bundle(&store, &provider, &request, None)
        .await
        .expect("durable four-file raw EOD");
    std::fs::remove_file(store.manifest_path(PROVIDER_KIS, MARKET_KR))
        .expect("interrupt raw manifest tail");

    let normalized = recover_kis_normalization(&store).expect("recover normalization");
    assert_eq!(normalized.outcomes.len(), 1);
    assert_eq!(
        normalized.outcomes[0].source_batch_id,
        ingested.entry.batch_id
    );
    let first = recover_unpublished_scope(&store, &sink, RecoveryScope::KisNormalized)
        .await
        .expect("recover publication");
    let second = recover_unpublished_scope(&store, &sink, RecoveryScope::KisNormalized)
        .await
        .expect("replay publication");

    assert_eq!(first.recovered.len(), 1);
    assert!(first.skipped.is_empty());
    assert!(second.recovered.is_empty());
    assert_eq!(second.skipped.len(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 30);
    assert_eq!(
        store
            .read_reconciled_manifest(PROVIDER_KIS, MARKET_KR)
            .expect("reconciled raw manifest")
            .len(),
        1
    );
    assert_eq!(
        store
            .read_reconciled_manifest(PROVIDER_KIS_NORMALIZED, MARKET_KR)
            .expect("reconciled normalized manifest")
            .len(),
        1
    );
}

#[tokio::test]
async fn legacy_complete_eod_recovery_requires_authentic_source_metadata() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RawStore::new(temp.path().join("data"));
    let source = append_kis_batch(&store, TARGET_DATE, |_| {});
    let reader = FakeKisRead::new(false);
    let provider = KisProvider::kr_etf_core(reader.clone());
    let request = IngestRequest::new(
        MARKET_KR.to_owned(),
        TradingDate::parse(TARGET_DATE).expect("date"),
        UtcTimestamp::parse_rfc3339(RETRIEVED_AT).expect("retrieved at"),
    );

    let no_new_claim = ensure_kis_calendar_source(&store, &provider, &request, None)
        .await
        .expect("legacy source inspection");
    assert!(no_new_claim.is_none());
    assert_eq!(reader.calls.load(Ordering::SeqCst), 0);

    let metadata_path = store
        .batch_dir(PROVIDER_KIS, MARKET_KR, &source.date, &source.batch_id)
        .join(source.batch_json_file_name());
    let mut metadata: Value =
        serde_json::from_slice(&std::fs::read(&metadata_path).expect("read source metadata"))
            .expect("source metadata JSON");
    metadata["retrieved_at"] = json!("2026-08-14T08:01:00Z");
    std::fs::write(
        &metadata_path,
        serde_json::to_vec(&metadata).expect("tampered metadata JSON"),
    )
    .expect("tamper source metadata");

    let sink = ReplaySink::default();
    let error = ingest_normalize_publish_kis_with_calendar_source(
        &store, &provider, &request, None, &sink, None,
    )
    .await
    .expect_err("legacy source metadata drift must fail closed");
    assert!(matches!(error, PipelineError::Ingest { .. }));
    assert_eq!(reader.calls.load(Ordering::SeqCst), 0);
    assert!(sink.calls.lock().expect("sink calls").is_empty());
}

#[tokio::test]
async fn target_date_normalized_replay_does_not_publish_other_backlog_or_refetch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RawStore::new(temp.path().join("data"));
    let _target_source = append_kis_batch(&store, TARGET_DATE, |_| {});
    let _other_source = append_kis_batch(&store, OTHER_DATE, |_| {});
    let normalized = recover_kis_normalization(&store).expect("normalize durable KIS sources");
    assert_eq!(normalized.outcomes.len(), 2);
    let target_batch_id = normalized
        .outcomes
        .iter()
        .find(|outcome| outcome.entry.date == TradingDate::parse(TARGET_DATE).unwrap())
        .expect("target normalized entry")
        .entry
        .batch_id;
    let other_batch_id = normalized
        .outcomes
        .iter()
        .find(|outcome| outcome.entry.date == TradingDate::parse(OTHER_DATE).unwrap())
        .expect("other normalized entry")
        .entry
        .batch_id;
    let sink = ReplaySink::default();

    let first = recover_unpublished_normalized_for_date(
        &store,
        &sink,
        &normalized,
        TradingDate::parse(TARGET_DATE).expect("target date"),
    )
    .await
    .expect("target-date recovery");
    assert_eq!(first.recovered, vec![target_batch_id]);
    assert!(first.skipped.is_empty());
    assert_eq!(
        sink.calls
            .lock()
            .expect("sink calls")
            .iter()
            .map(|(batch_id, _, _)| *batch_id)
            .collect::<Vec<_>>(),
        vec![target_batch_id],
        "ingest retry must not publish another date or refetch the target"
    );

    let second = recover_unpublished_normalized_for_date(
        &store,
        &sink,
        &normalized,
        TradingDate::parse(TARGET_DATE).expect("target date"),
    )
    .await
    .expect("idempotent target-date recovery");
    assert!(second.recovered.is_empty());
    assert_eq!(second.skipped, vec![target_batch_id]);
    assert!(
        !sink
            .calls
            .lock()
            .expect("sink calls")
            .iter()
            .any(|(batch_id, _, _)| *batch_id == other_batch_id)
    );
}

#[tokio::test]
async fn kis_ingest_normalize_publish_is_canonical_and_recovery_is_idempotent() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RawStore::new(temp.path().join("data"));
    let reader = FakeKisRead::new(false);
    let calls = Arc::clone(&reader.calls);
    let provider = KisProvider::kr_etf_core(reader);
    let request = market_data::IngestRequest::new(
        MARKET_KR.to_owned(),
        TradingDate::parse(TARGET_DATE).expect("target date"),
        UtcTimestamp::parse_rfc3339(RETRIEVED_AT).expect("retrieved at"),
    );
    let sink = ReplaySink::default();

    let first = ingest_normalize_publish_kis(&store, &provider, &request, None, &sink)
        .await
        .expect("KIS fake ingest and publication");
    assert_eq!(first.manifest.provider, PROVIDER_KIS_NORMALIZED);
    assert_eq!(first.manifest.mode, FetchMode::Credentialed);
    assert_eq!(first.published, PublishOutcome::Published);
    assert_eq!(calls.load(Ordering::SeqCst), 30);
    assert_eq!(
        store
            .read_reconciled_manifest(PROVIDER_KIS, MARKET_KR)
            .expect("wire manifest")
            .len(),
        1
    );
    assert_eq!(
        store
            .read_reconciled_manifest(PROVIDER_KIS_NORMALIZED, MARKET_KR)
            .expect("normalized manifest")
            .len(),
        1
    );

    let normalized = recover_kis_normalization(&store).expect("replay normalization");
    assert_eq!(normalized.outcomes.len(), 1);
    let recovery = recover_unpublished_scope(&store, &sink, RecoveryScope::KisNormalized)
        .await
        .expect("replay publication");
    assert!(recovery.recovered.is_empty());
    assert_eq!(recovery.skipped, vec![first.manifest.batch_id]);
    assert_eq!(
        store
            .read_reconciled_manifest(PROVIDER_KIS_NORMALIZED, MARKET_KR)
            .expect("normalized manifest after replay")
            .len(),
        1
    );
    assert_eq!(calls.load(Ordering::SeqCst), 30);
}

#[tokio::test]
async fn malformed_kis_normalization_is_permanent_after_wire_commit_and_never_publishes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RawStore::new(temp.path().join("data"));
    let provider = KisProvider::kr_etf_core(FakeKisRead::new(true));
    let request = market_data::IngestRequest::new(
        MARKET_KR.to_owned(),
        TradingDate::parse(TARGET_DATE).expect("target date"),
        UtcTimestamp::parse_rfc3339(RETRIEVED_AT).expect("retrieved at"),
    );
    let sink = ReplaySink::default();

    let error = ingest_normalize_publish_kis(&store, &provider, &request, None, &sink)
        .await
        .expect_err("malformed canonical field");
    assert_eq!(error.failure_class(), FailureClass::Permanent);
    assert_eq!(error.stage(), PipelineStage::VerifyRaw);
    assert!(matches!(error, PipelineError::Normalize { .. }));
    assert_eq!(
        store
            .read_reconciled_manifest(PROVIDER_KIS, MARKET_KR)
            .expect("durable wire batch")
            .len(),
        1
    );
    assert!(
        store
            .read_reconciled_manifest(PROVIDER_KIS_NORMALIZED, MARKET_KR)
            .expect("normalized manifest")
            .is_empty()
    );
    assert!(sink.calls.lock().expect("sink calls").is_empty());
}

#[test]
fn malformed_normalization_is_permanent_and_preserves_source() {
    let (_temp, store, source) = seed_kis_store(|wires| {
        let bar = wires
            .iter_mut()
            .find(|wire| wire.kind == ResponseKind::Bars)
            .expect("bar");
        let mut document: Value = serde_json::from_slice(&bar.bytes).expect("bar json");
        document["output2"][0]
            .as_object_mut()
            .expect("bar row")
            .remove("stck_clpr");
        bar.bytes = serde_json::to_vec(&document).expect("malformed bar");
    });
    let error = recover_kis_normalization(&store).expect_err("malformed KIS source");
    assert_eq!(error.batch_id(), Some(source.batch_id));
    assert_eq!(error.stage(), PipelineStage::VerifyRaw);
    assert_eq!(error.failure_class(), FailureClass::Permanent);
    assert!(matches!(error, PipelineError::Normalize { .. }));
    assert!(store.manifest_path(PROVIDER_KIS, MARKET_KR).is_file());
    assert!(
        store
            .read_manifest(PROVIDER_KIS_NORMALIZED, MARKET_KR)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn normalization_store_io_and_collision_are_retryable_but_schema_is_not() {
    let batch_id = BatchId::generate();
    let io = PipelineError::Normalize {
        batch_id,
        source: Box::new(NormalizeError::Store(StoreError::Io {
            context: "test".into(),
            source: std::io::Error::other("offline"),
        })),
    };
    assert_eq!(io.failure_class(), FailureClass::Retryable);
    assert!(io.is_retryable());
    assert!(io.source().is_some());
    assert!(io.to_string().contains(&batch_id.to_string()));

    let collision = PipelineError::Normalize {
        batch_id,
        source: Box::new(NormalizeError::Store(StoreError::FileExists {
            path: "batch".into(),
        })),
    };
    assert!(collision.is_retryable());

    let malformed = PipelineError::Normalize {
        batch_id,
        source: Box::new(NormalizeError::Malformed {
            kind: ResponseKind::Bars,
            file_name: "bars.json".into(),
            reason: "schema drift".into(),
        }),
    };
    assert_eq!(malformed.failure_class(), FailureClass::Permanent);
    assert!(!malformed.is_retryable());
}

#[test]
fn recovery_scope_owns_the_only_publishable_provider_pairs() {
    assert_eq!(RecoveryScope::Krx.provider(), "krx");
    assert_eq!(RecoveryScope::Krx.market(), MARKET_KR);
    assert_eq!(
        RecoveryScope::KisNormalized.provider(),
        PROVIDER_KIS_NORMALIZED
    );
    assert_eq!(RecoveryScope::KisNormalized.market(), MARKET_KR);
    assert!(RecoveryScope::KisNormalized.is_kis_normalized());
}
