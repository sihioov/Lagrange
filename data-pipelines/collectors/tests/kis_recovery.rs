use std::collections::HashMap;
use std::error::Error as _;
use std::sync::Mutex;

use async_trait::async_trait;
use collectors::{
    FailureClass, PipelineError, PipelineStage, PublicationSink, PublicationState, PublishOutcome,
    RecoveryScope, SinkError, ingest_normalize_publish_kis, recover_kis_normalization,
    recover_unpublished_normalized_for_date, recover_unpublished_scope,
};
use domain::{BatchId, TradingDate, UtcTimestamp};
use kis_client::{KisError, MarketDataReply};
use market_data::contract::{
    FetchMode, MARKET_KR, PROVIDER_KIS, PROVIDER_KIS_NORMALIZED, RawEnvelope, RequestMetadata,
    ResponseKind,
};
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
                    "std_pdno": symbol,
                    "prdt_name": format!("ETF {symbol}")
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
            bytes: if endpoint.ends_with("/paidin-capin") {
                br#"{"rt_cd":"0","output":[]}"#.to_vec()
            } else {
                br#"{"rt_cd":"0","output1":[]}"#.to_vec()
            },
        });
    }
    wires
}

#[derive(Debug, Clone)]
struct FakeKisRead {
    calls: Arc<AtomicUsize>,
    malformed_first_bar: bool,
}

impl FakeKisRead {
    fn new(malformed_first_bar: bool) -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
            malformed_first_bar,
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
        let body = if path.ends_with("inquire-daily-itemchartprice") {
            let mut row = json!({
                "stck_bsop_date": "20260814",
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
            json!({"rt_cd": "0", "output1": {}, "output2": [row]})
        } else if path.ends_with("inquire-price") {
            json!({
                "rt_cd": "0",
                "output": {"std_pdno": symbol, "prdt_name": format!("ETF {symbol}")}
            })
        } else if path.ends_with("chk-holiday") {
            json!({
                "rt_cd": "0",
                "output": [{"bass_dt": "20260814", "opnd_yn": "Y"}]
            })
        } else if path.ends_with("paidin-capin") {
            json!({"rt_cd": "0", "output": []})
        } else {
            json!({"rt_cd": "0", "output1": []})
        };
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
