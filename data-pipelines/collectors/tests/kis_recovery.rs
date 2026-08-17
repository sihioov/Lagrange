use std::collections::HashMap;
use std::error::Error as _;
use std::sync::Mutex;

use async_trait::async_trait;
use collectors::{
    FailureClass, PipelineError, PipelineStage, PublicationSink, PublicationState, PublishOutcome,
    RecoveryScope, SinkError, recover_kis_normalization, recover_unpublished_scope,
};
use domain::{BatchId, TradingDate, UtcTimestamp};
use market_data::contract::{
    FetchMode, MARKET_KR, PROVIDER_KIS, PROVIDER_KIS_NORMALIZED, RawEnvelope, RequestMetadata,
    ResponseKind,
};
use market_data::normalize::NormalizeError;
use market_data::providers::kis::KR_ETF_CORE_SYMBOLS;
use market_data::publication::PublicationBundle;
use market_data::storage::{BatchSpec, ManifestEntry, RawStore, StoreError};
use serde_json::{Value, json};
use tempfile::TempDir;

const TARGET_DATE: &str = "2026-08-14";
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
    let mut wires = valid_wires();
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
        .expect("KIS source batch");
    (temp, store, entry)
}

fn valid_wires() -> Vec<Wire> {
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
                    "stck_bsop_date": "20260814",
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
        query: vec![("BASS_DT".into(), "20260814".into())],
        bytes: serde_json::to_vec(&json!({
            "rt_cd": "0",
            "output": [{"bass_dt": "20260814", "opnd_yn": "Y"}]
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
            query: vec![
                ("F_DT".into(), "20260814".into()),
                ("T_DT".into(), "20260814".into()),
            ],
            bytes: br#"{"rt_cd":"0","output1":[]}"#.to_vec(),
        });
    }
    wires
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
