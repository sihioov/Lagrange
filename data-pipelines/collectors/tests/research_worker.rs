use std::collections::{HashMap, VecDeque};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use collectors::{
    AppEnvironment, FailureClass, HealthcheckConfig, PipelineError, PipelineStage, PublicationSink,
    PublicationState, PublishOutcome, RECOVERY_PAGE_SIZE, RecoveryBatchOutcome, RecoveryError,
    RecoveryObserver, RecoveryPosition, ResearchBackend, ResearchWorker, ResearchWorkerConfig,
    SinkError, WaitOutcome, WorkerComponentFactory, WorkerControl, WorkerError, WorkerEvent,
    WorkerEventClass, WorkerEventKind, WorkerObserver, WorkerPhase, WorkerRunOutcome,
    bootstrap_worker_with, build_postgres_pool, healthcheck, ingest_and_publish, next_run_delay,
    publication_age, recover_unpublished, recover_unpublished_page_with, recover_unpublished_with,
    retry_delay, run_internal_recovery_stream, store_failure_class, validate_synthetic_policy,
};
use domain::{BatchId, TradingDate, UtcTimestamp};
use market_data::contract::{FetchMode, MARKET_KR, PROVIDER_KRX, ResponseKind};
use market_data::ingest::{IngestError, IngestRequest, ingest_bundle};
use market_data::provider::{
    CredentialRef, EodProvider, FetchRequest, KrxProvider, ProviderError, RecordedBundle,
};
use market_data::publication::{PublicationBundle, PublicationError};
use market_data::storage::{RawStore, StoreError};
use sqlx::PgPool;

#[allow(dead_code)]
mod common;
use common::ScratchDb;

fn provider() -> KrxProvider {
    KrxProvider::synthetic(
        RecordedBundle::open(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/kr-etf/contract"
        ))
        .unwrap(),
    )
}

fn request(at: &str) -> IngestRequest {
    IngestRequest::new(
        MARKET_KR.to_owned(),
        TradingDate::parse("2020-01-31").unwrap(),
        UtcTimestamp::parse_rfc3339(at).unwrap(),
    )
}

async fn seed_candidate_entitlement(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO data_entitlements
         (contract_document_sha256,contract_reference,status,covered_datasets,
          covered_uses,effective_from,effective_until,managed_by)
         VALUES (repeat('8',64),'fixture://candidate-license','ACTIVE',$1,
                 '[\"candidate\"]'::jsonb,DATE '2000-01-01',DATE '2030-12-31',
                 '00000000-0000-4000-8000-000000000042'::uuid)",
    )
    .bind(serde_json::json!([
        "krx_eod_bars",
        "krx_investor_flows",
        "krx_market_status",
        "krx_fundamentals",
        "krx_kospi200_membership",
        "krx_kosdaq150_membership",
        "krx_sector_classification"
    ]))
    .execute(pool)
    .await
    .expect("candidate-use fixture entitlement");
}

struct AlwaysFailWriter;

impl std::io::Write for AlwaysFailWriter {
    fn write(&mut self, _buffer: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("closed recovery stream"))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Err(std::io::Error::other("closed recovery stream"))
    }
}

type PublishCheck = Arc<dyn Fn(&PublicationBundle) + Send + Sync>;

#[derive(Default)]
struct FakeSink {
    states: Mutex<HashMap<BatchId, PublicationState>>,
    state_error: Mutex<Option<SinkError>>,
    publish_error: Mutex<Option<SinkError>>,
    outcome: Mutex<Option<PublishOutcome>>,
    state_calls: Mutex<Vec<BatchId>>,
    publish_calls: Mutex<Vec<BatchId>>,
    publish_check: Option<PublishCheck>,
}

impl FakeSink {
    fn with_state(self, batch_id: BatchId, state: PublicationState) -> Self {
        self.states.lock().unwrap().insert(batch_id, state);
        self
    }

    fn with_publish_error(self, error: SinkError) -> Self {
        *self.publish_error.lock().unwrap() = Some(error);
        self
    }

    fn with_state_error(self, error: SinkError) -> Self {
        *self.state_error.lock().unwrap() = Some(error);
        self
    }

    fn with_outcome(self, outcome: PublishOutcome) -> Self {
        *self.outcome.lock().unwrap() = Some(outcome);
        self
    }
}

#[async_trait]
impl PublicationSink for FakeSink {
    async fn publication_state(&self, batch_id: BatchId) -> Result<PublicationState, SinkError> {
        self.state_calls.lock().unwrap().push(batch_id);
        if let Some(error) = self.state_error.lock().unwrap().take() {
            return Err(error);
        }
        Ok(self
            .states
            .lock()
            .unwrap()
            .get(&batch_id)
            .copied()
            .unwrap_or(PublicationState::Missing))
    }

    async fn publish(&self, bundle: &PublicationBundle) -> Result<PublishOutcome, SinkError> {
        self.publish_calls
            .lock()
            .unwrap()
            .push(bundle.source_batch_id);
        if let Some(check) = &self.publish_check {
            check(bundle);
        }
        if let Some(error) = self.publish_error.lock().unwrap().take() {
            return Err(error);
        }
        Ok(self
            .outcome
            .lock()
            .unwrap()
            .unwrap_or(PublishOutcome::Published))
    }

    async fn has_eod(&self, _date: TradingDate) -> Result<bool, SinkError> {
        Ok(false)
    }
}

#[tokio::test]
async fn pipeline_ingest_and_publish_persists_raw_before_publishing() {
    let root = tempfile::tempdir().unwrap();
    let store = RawStore::new(root.path());
    let manifest_path = store.manifest_path(PROVIDER_KRX, MARKET_KR);
    let callback_store = store.clone();
    let sink = FakeSink {
        publish_check: Some(Arc::new(move |bundle| {
            assert!(
                manifest_path.is_file(),
                "manifest must predate DB publication"
            );
            let entries = callback_store
                .read_manifest(PROVIDER_KRX, MARKET_KR)
                .unwrap();
            assert_eq!(entries.last().unwrap().batch_id, bundle.source_batch_id);
            assert_eq!(bundle.files.len(), 4);
        })),
        ..FakeSink::default()
    };

    let result = ingest_and_publish(
        &store,
        &provider(),
        &request("2026-08-05T07:00:00Z"),
        None,
        &sink,
    )
    .await
    .unwrap();

    let durable = store.read_manifest(PROVIDER_KRX, MARKET_KR).unwrap();
    assert_eq!(durable, vec![result.manifest.clone()]);
    assert_eq!(result.published, PublishOutcome::Published);
    assert_eq!(
        *sink.publish_calls.lock().unwrap(),
        vec![result.manifest.batch_id]
    );
}

#[test]
fn pipeline_ingest_and_publish_future_is_send() {
    fn assert_send<T: Send>(_: T) {}

    let root = tempfile::tempdir().unwrap();
    let store = RawStore::new(root.path());
    let provider = provider();
    let request = request("2026-08-05T07:00:00Z");
    let sink = FakeSink::default();
    assert_send(ingest_and_publish(&store, &provider, &request, None, &sink));
}

#[derive(Debug)]
struct FailingProvider;

impl EodProvider for FailingProvider {
    fn provider_id(&self) -> &'static str {
        PROVIDER_KRX
    }

    fn fetch_mode(&self) -> market_data::FetchMode {
        market_data::FetchMode::Synthetic
    }

    fn fetch(
        &self,
        _request: &FetchRequest,
    ) -> Result<Vec<market_data::RawEnvelope>, ProviderError> {
        Err(ProviderError::EndpointTimeout {
            kind: ResponseKind::Bars,
            timeout_secs: 30,
        })
    }
}

#[tokio::test]
async fn pipeline_provider_failure_never_calls_sink() {
    let root = tempfile::tempdir().unwrap();
    let store = RawStore::new(root.path());
    let sink = FakeSink::default();

    let error = ingest_and_publish(
        &store,
        &FailingProvider,
        &request("2026-08-05T07:00:00Z"),
        None,
        &sink,
    )
    .await
    .unwrap_err();

    assert!(error.is_retryable());
    assert!(matches!(error, PipelineError::Ingest { .. }));
    assert!(sink.publish_calls.lock().unwrap().is_empty());
    assert!(sink.state_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn pipeline_raw_store_failure_never_calls_sink() {
    let root = tempfile::tempdir().unwrap();
    let blocked_root = root.path().join("not-a-directory");
    std::fs::write(&blocked_root, b"blocks Raw directory creation").unwrap();
    let store = RawStore::new(blocked_root);
    let sink = FakeSink::default();

    let error = ingest_and_publish(
        &store,
        &provider(),
        &request("2026-08-05T07:00:00Z"),
        None,
        &sink,
    )
    .await
    .unwrap_err();

    assert!(error.is_retryable());
    assert!(matches!(
        error,
        PipelineError::Ingest {
            source: IngestError::Store(StoreError::Io { .. })
        }
    ));
    assert!(sink.publish_calls.lock().unwrap().is_empty());
    assert!(sink.state_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn pipeline_durable_manifest_failure_exposes_recoverable_batch_id() {
    let root = tempfile::tempdir().unwrap();
    let store = RawStore::new(root.path());
    let manifest_path = store.manifest_path(PROVIDER_KRX, MARKET_KR);
    std::fs::create_dir_all(&manifest_path).unwrap();
    let sink = FakeSink::default();

    let error = ingest_and_publish(
        &store,
        &provider(),
        &request("2026-08-05T07:00:00Z"),
        None,
        &sink,
    )
    .await
    .unwrap_err();

    let batch_id = error.batch_id().expect("durable batch id");
    assert_eq!(error.failure_class(), FailureClass::Retryable);
    {
        use std::error::Error as _;
        let ingest = error.source().expect("pipeline ingest source");
        let durable = ingest.source().expect("durable store source");
        let manifest = durable.source().expect("manifest failure source");
        assert!(matches!(
            manifest.downcast_ref::<StoreError>(),
            Some(StoreError::Io { .. })
        ));
    }
    assert!(matches!(
        error,
        PipelineError::Ingest {
            source: IngestError::Store(StoreError::IndeterminateBatchCommit { .. })
        }
    ));
    assert!(sink.publish_calls.lock().unwrap().is_empty());
    std::fs::remove_dir(&manifest_path).unwrap();
    let entries = store.read_manifest(PROVIDER_KRX, MARKET_KR).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].batch_id, batch_id);
}

#[tokio::test]
async fn pipeline_retryable_publish_failure_retains_the_durable_batch() {
    let root = tempfile::tempdir().unwrap();
    let store = RawStore::new(root.path());
    let sink = FakeSink::default()
        .with_publish_error(SinkError::RetryableDatabase(sqlx::Error::PoolClosed));

    let error = ingest_and_publish(
        &store,
        &provider(),
        &request("2026-08-05T07:00:00Z"),
        None,
        &sink,
    )
    .await
    .unwrap_err();

    let batch_id = error
        .batch_id()
        .expect("publish error has durable batch ID");
    assert_eq!(error.failure_class(), FailureClass::Retryable);
    assert_eq!(error.stage(), PipelineStage::Publish);
    let entries = store.read_manifest(PROVIDER_KRX, MARKET_KR).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].batch_id, batch_id);
    PublicationBundle::from_raw(&store, &entries[0]).unwrap();
}

#[tokio::test]
async fn pipeline_recovery_reuses_the_failed_publication_batch_without_reingestion() {
    let root = tempfile::tempdir().unwrap();
    let store = RawStore::new(root.path());
    let first_sink = FakeSink::default()
        .with_publish_error(SinkError::RetryableDatabase(sqlx::Error::PoolClosed));
    let error = ingest_and_publish(
        &store,
        &provider(),
        &request("2026-08-05T07:00:00Z"),
        None,
        &first_sink,
    )
    .await
    .unwrap_err();
    let original_batch = error.batch_id().unwrap();
    let before = store.read_manifest(PROVIDER_KRX, MARKET_KR).unwrap();

    let recovery_sink = FakeSink::default().with_outcome(PublishOutcome::AlreadyPublished);
    let report = recover_unpublished(&store, &recovery_sink).await.unwrap();

    assert_eq!(report.recovered, vec![original_batch]);
    assert!(report.skipped.is_empty());
    assert_eq!(
        store.read_manifest(PROVIDER_KRX, MARKET_KR).unwrap(),
        before
    );
    assert_eq!(
        *recovery_sink.publish_calls.lock().unwrap(),
        vec![original_batch]
    );
}

#[tokio::test]
async fn pipeline_recovery_callback_is_after_db_outcome_and_failure_stops_next_batch() {
    let root = tempfile::tempdir().unwrap();
    let store = RawStore::new(root.path());
    let first = ingest_bundle(&store, &provider(), &request("2026-08-04T07:00:00Z"), None)
        .unwrap()
        .entry;
    let second = ingest_bundle(&store, &provider(), &request("2026-08-05T07:00:00Z"), None)
        .unwrap()
        .entry;
    let sink = FakeSink::default();
    let mut observed = Vec::new();

    let error = recover_unpublished_with(&store, &sink, |outcome| {
        observed.push(outcome);
        Err(std::io::Error::other("closed recovery event stream"))
    })
    .await
    .unwrap_err();

    assert!(matches!(
        error,
        RecoveryError::Observer {
            batch_id,
            source: _
        } if batch_id == first.batch_id
    ));
    assert_eq!(
        observed,
        vec![RecoveryBatchOutcome::Recovered {
            batch_id: first.batch_id,
            date: first.date,
        }]
    );
    assert_eq!(
        *sink.publish_calls.lock().unwrap(),
        vec![first.batch_id],
        "callback runs after first DB outcome and stops before the next batch"
    );
    assert_ne!(first.batch_id, second.batch_id);
}

#[tokio::test]
async fn pipeline_recovery_publishes_a_durable_orphan_batch() {
    let root = tempfile::tempdir().unwrap();
    let store = RawStore::new(root.path());
    let entry = ingest_bundle(&store, &provider(), &request("2026-08-05T07:00:00Z"), None)
        .unwrap()
        .entry;
    std::fs::remove_file(store.manifest_path(PROVIDER_KRX, MARKET_KR)).unwrap();
    let sink = FakeSink::default();

    let report = recover_unpublished(&store, &sink).await.unwrap();

    assert_eq!(report.recovered, vec![entry.batch_id]);
    assert!(report.skipped.is_empty());
    assert_eq!(*sink.publish_calls.lock().unwrap(), vec![entry.batch_id]);
}

#[tokio::test]
async fn pipeline_recovery_orders_missing_oldest_first_and_skips_complete() {
    let root = tempfile::tempdir().unwrap();
    let store = RawStore::new(root.path());
    let mut entries = Vec::new();
    for at in [
        "2026-08-06T07:00:00Z",
        "2026-08-04T07:00:00Z",
        "2026-08-05T07:00:00Z",
    ] {
        entries.push(
            ingest_bundle(&store, &provider(), &request(at), None)
                .unwrap()
                .entry,
        );
    }
    let complete = entries[2].batch_id;
    let sink = FakeSink::default()
        .with_state(complete, PublicationState::Complete)
        .with_outcome(PublishOutcome::AlreadyPublished);

    let report = recover_unpublished(&store, &sink).await.unwrap();

    let expected_recovered = vec![entries[1].batch_id, entries[0].batch_id];
    assert_eq!(report.recovered, expected_recovered);
    assert_eq!(report.skipped, vec![complete]);
    assert_eq!(
        *sink.publish_calls.lock().unwrap(),
        vec![entries[1].batch_id, complete, entries[0].batch_id]
    );
    assert_eq!(
        *sink.state_calls.lock().unwrap(),
        vec![entries[1].batch_id, complete, entries[0].batch_id]
    );
}

#[tokio::test]
async fn pipeline_recovery_pages_resume_by_canonical_batch_cursor() {
    let root = tempfile::tempdir().unwrap();
    let store = RawStore::new(root.path());
    let mut entries = Vec::new();
    for day in 1..=5 {
        entries.push(
            ingest_bundle(
                &store,
                &provider(),
                &request(&format!("2026-08-{day:02}T07:00:00Z")),
                None,
            )
            .unwrap()
            .entry,
        );
    }
    let sink = FakeSink::default();
    let mut observed = Vec::new();

    let first = recover_unpublished_page_with(
        &store,
        &sink,
        RecoveryPosition::default(),
        2,
        |outcome, _snapshot_high_water| {
            observed.push(outcome);
            Ok::<_, std::convert::Infallible>(())
        },
    )
    .await
    .unwrap();
    assert_eq!(first.cursor, Some(entries[1].batch_id));
    assert_eq!(first.snapshot_high_water, Some(entries[4].batch_id));
    assert!(first.has_more);

    // An append after the fixed high-water belongs to the next snapshot even
    // when its retrieved_at sorts before the current cursor.
    let appended = ingest_bundle(&store, &provider(), &request("2026-07-31T07:00:00Z"), None)
        .unwrap()
        .entry;
    let second = recover_unpublished_page_with(
        &store,
        &sink,
        RecoveryPosition {
            snapshot_after: None,
            snapshot_high_water: first.snapshot_high_water,
            cursor: first.cursor,
        },
        2,
        |outcome, _snapshot_high_water| {
            observed.push(outcome);
            Ok::<_, std::convert::Infallible>(())
        },
    )
    .await
    .unwrap();
    assert_eq!(second.cursor, Some(entries[3].batch_id));
    assert!(second.has_more);
    let third = recover_unpublished_page_with(
        &store,
        &sink,
        RecoveryPosition {
            snapshot_after: None,
            snapshot_high_water: first.snapshot_high_water,
            cursor: second.cursor,
        },
        2,
        |outcome, _snapshot_high_water| {
            observed.push(outcome);
            Ok::<_, std::convert::Infallible>(())
        },
    )
    .await
    .unwrap();
    assert_eq!(third.cursor, Some(entries[4].batch_id));
    assert!(!third.has_more);

    let fourth = recover_unpublished_page_with(
        &store,
        &sink,
        RecoveryPosition {
            snapshot_after: first.snapshot_high_water,
            snapshot_high_water: None,
            cursor: None,
        },
        2,
        |outcome, _snapshot_high_water| {
            observed.push(outcome);
            Ok::<_, std::convert::Infallible>(())
        },
    )
    .await
    .unwrap();
    assert_eq!(fourth.snapshot_high_water, Some(appended.batch_id));
    assert_eq!(fourth.cursor, Some(appended.batch_id));
    assert!(!fourth.has_more);

    let stable = recover_unpublished_page_with(
        &store,
        &sink,
        RecoveryPosition {
            snapshot_after: fourth.snapshot_high_water,
            snapshot_high_water: None,
            cursor: None,
        },
        2,
        |_, _| -> Result<(), std::convert::Infallible> {
            panic!("an unchanged high-water completion check must be empty")
        },
    )
    .await
    .unwrap();
    assert_eq!(stable.snapshot_high_water, fourth.snapshot_high_water);
    assert_eq!(stable.cursor, None);
    assert!(!stable.has_more);

    assert_eq!(
        observed
            .iter()
            .map(|outcome| outcome.batch_id())
            .collect::<Vec<_>>(),
        entries
            .iter()
            .map(|entry| entry.batch_id)
            .chain(std::iter::once(appended.batch_id))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        *sink.state_calls.lock().unwrap(),
        observed
            .iter()
            .map(|outcome| outcome.batch_id())
            .collect::<Vec<_>>(),
        "resumed pages must never reprocess an emitted prefix"
    );
}

#[tokio::test]
async fn pipeline_recovery_reconciles_orphan_before_later_normal_append() {
    let root = tempfile::tempdir().unwrap();
    let store = RawStore::new(root.path());
    let orphan = ingest_bundle(&store, &provider(), &request("2026-08-05T07:00:00Z"), None)
        .unwrap()
        .entry;
    std::fs::remove_file(store.manifest_path(PROVIDER_KRX, MARKET_KR)).unwrap();

    let sink = FakeSink::default();
    let mut observed = Vec::new();
    let first = recover_unpublished_page_with(
        &store,
        &sink,
        RecoveryPosition::default(),
        1,
        |outcome, _snapshot_high_water| {
            observed.push(outcome.batch_id());
            Ok::<_, std::convert::Infallible>(())
        },
    )
    .await
    .unwrap();
    assert_eq!(first.snapshot_high_water, Some(orphan.batch_id));
    assert_eq!(first.cursor, Some(orphan.batch_id));
    assert!(!first.has_more);

    let normal = ingest_bundle(&store, &provider(), &request("2026-08-06T07:00:00Z"), None)
        .unwrap()
        .entry;
    let suffix = recover_unpublished_page_with(
        &store,
        &sink,
        RecoveryPosition {
            snapshot_after: first.snapshot_high_water,
            snapshot_high_water: None,
            cursor: None,
        },
        1,
        |outcome, _snapshot_high_water| {
            observed.push(outcome.batch_id());
            Ok::<_, std::convert::Infallible>(())
        },
    )
    .await
    .unwrap();

    assert_eq!(suffix.snapshot_high_water, Some(normal.batch_id));
    assert_eq!(suffix.cursor, Some(normal.batch_id));
    assert!(!suffix.has_more);
    assert_eq!(observed, vec![orphan.batch_id, normal.batch_id]);
    assert_eq!(
        store
            .read_manifest(PROVIDER_KRX, MARKET_KR)
            .unwrap()
            .into_iter()
            .map(|entry| entry.batch_id)
            .collect::<Vec<_>>(),
        vec![orphan.batch_id, normal.batch_id],
        "recovery high-water identities must be durable manifest lines"
    );
}

#[tokio::test]
async fn pipeline_recovery_drains_a_backdated_append_in_a_followup_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let store = RawStore::new(root.path());
    let mut original = Vec::new();
    for at in [
        "2026-08-03T07:00:00Z",
        "2026-08-04T07:00:00Z",
        "2026-08-05T07:00:00Z",
    ] {
        original.push(
            ingest_bundle(&store, &provider(), &request(at), None)
                .unwrap()
                .entry,
        );
    }
    let appended_id = Arc::new(Mutex::new(None));
    let appended_id_for_hook = appended_id.clone();
    let append_store = store.clone();
    let appended = Arc::new(AtomicBool::new(false));
    let appended_for_hook = appended.clone();
    let sink = FakeSink {
        publish_check: Some(Arc::new(move |_| {
            if !appended_for_hook.swap(true, Ordering::SeqCst) {
                let entry = ingest_bundle(
                    &append_store,
                    &provider(),
                    &request("2026-07-31T07:00:00Z"),
                    None,
                )
                .unwrap()
                .entry;
                *appended_id_for_hook.lock().unwrap() = Some(entry.batch_id);
            }
        })),
        ..FakeSink::default()
    };

    let report = recover_unpublished(&store, &sink).await.unwrap();
    let appended_id = appended_id.lock().unwrap().expect("hook appended a batch");

    assert_eq!(
        report.recovered,
        original
            .iter()
            .map(|entry| entry.batch_id)
            .chain(std::iter::once(appended_id))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        *sink.state_calls.lock().unwrap(),
        report.recovered,
        "the fixed prefix is not replayed and the new suffix is drained before completion"
    );
}

#[tokio::test]
async fn pipeline_recovery_rejects_a_missing_cursor_permanently() {
    let root = tempfile::tempdir().unwrap();
    let store = RawStore::new(root.path());
    ingest_bundle(&store, &provider(), &request("2026-08-05T07:00:00Z"), None).unwrap();
    let sink = FakeSink::default();
    let missing = BatchId::generate();

    let entry = store.read_manifest(PROVIDER_KRX, MARKET_KR).unwrap()[0].clone();
    let error = recover_unpublished_page_with(
        &store,
        &sink,
        RecoveryPosition {
            snapshot_after: None,
            snapshot_high_water: Some(entry.batch_id),
            cursor: Some(missing),
        },
        2,
        |_, _| Ok::<_, std::convert::Infallible>(()),
    )
    .await
    .unwrap_err();

    assert!(matches!(
        error,
        RecoveryError::Pipeline(PipelineError::InvalidRecoveryCursor { cursor })
            if cursor == missing
    ));
    assert!(sink.state_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn pipeline_recovery_rejects_missing_snapshot_boundaries_permanently() {
    let root = tempfile::tempdir().unwrap();
    let store = RawStore::new(root.path());
    let entry = ingest_bundle(&store, &provider(), &request("2026-08-05T07:00:00Z"), None)
        .unwrap()
        .entry;
    let sink = FakeSink::default();
    let missing = BatchId::generate();

    for position in [
        RecoveryPosition {
            snapshot_after: None,
            snapshot_high_water: Some(missing),
            cursor: None,
        },
        RecoveryPosition {
            snapshot_after: Some(missing),
            snapshot_high_water: None,
            cursor: None,
        },
        RecoveryPosition {
            snapshot_after: None,
            snapshot_high_water: None,
            cursor: Some(entry.batch_id),
        },
    ] {
        let error = recover_unpublished_page_with(&store, &sink, position, 2, |_, _| {
            Ok::<_, std::convert::Infallible>(())
        })
        .await
        .unwrap_err();
        assert!(matches!(
            error,
            RecoveryError::Pipeline(ref source)
                if source.failure_class() == FailureClass::Permanent
                    && source.stage() == PipelineStage::ReadManifest
        ));
    }
    assert!(sink.state_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn pipeline_complete_state_requires_authoritative_exact_replay() {
    let root = tempfile::tempdir().unwrap();
    let store = RawStore::new(root.path());
    let entry = ingest_bundle(&store, &provider(), &request("2026-08-05T07:00:00Z"), None)
        .unwrap()
        .entry;
    let sink = FakeSink::default()
        .with_state(entry.batch_id, PublicationState::Complete)
        .with_publish_error(SinkError::Conflict("missing publication history".into()));

    let error = recover_unpublished(&store, &sink).await.unwrap_err();

    assert_eq!(error.batch_id(), Some(entry.batch_id));
    assert_eq!(error.failure_class(), FailureClass::Permanent);
    assert_eq!(error.stage(), PipelineStage::Publish);
    assert!(matches!(
        error,
        PipelineError::Sink {
            source: SinkError::Conflict(_),
            ..
        }
    ));
    assert_eq!(*sink.publish_calls.lock().unwrap(), vec![entry.batch_id]);
}

#[tokio::test]
async fn pipeline_complete_state_accepts_already_published_and_records_skipped() {
    let root = tempfile::tempdir().unwrap();
    let store = RawStore::new(root.path());
    let entry = ingest_bundle(&store, &provider(), &request("2026-08-05T07:00:00Z"), None)
        .unwrap()
        .entry;
    let sink = FakeSink::default()
        .with_state(entry.batch_id, PublicationState::Complete)
        .with_outcome(PublishOutcome::AlreadyPublished);

    let report = recover_unpublished(&store, &sink).await.unwrap();

    assert!(report.recovered.is_empty());
    assert_eq!(report.skipped, vec![entry.batch_id]);
    assert_eq!(*sink.publish_calls.lock().unwrap(), vec![entry.batch_id]);
}

#[tokio::test]
async fn pipeline_partial_state_is_permanent_and_is_not_repaired() {
    let root = tempfile::tempdir().unwrap();
    let store = RawStore::new(root.path());
    let entry = ingest_bundle(&store, &provider(), &request("2026-08-05T07:00:00Z"), None)
        .unwrap()
        .entry;
    let sink = FakeSink::default().with_state(entry.batch_id, PublicationState::Partial);

    let error = recover_unpublished(&store, &sink).await.unwrap_err();

    assert_eq!(error.batch_id(), Some(entry.batch_id));
    assert_eq!(error.failure_class(), FailureClass::Permanent);
    assert_eq!(error.stage(), PipelineStage::PublicationState);
    assert!(matches!(error, PipelineError::PartialPublication { .. }));
    assert!(sink.publish_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn pipeline_tampered_raw_stops_recovery_permanently_without_removing_evidence() {
    let root = tempfile::tempdir().unwrap();
    let store = RawStore::new(root.path());
    let entry = ingest_bundle(&store, &provider(), &request("2026-08-05T07:00:00Z"), None)
        .unwrap()
        .entry;
    let tampered = store
        .batch_dir(PROVIDER_KRX, MARKET_KR, &entry.date, &entry.batch_id)
        .join(&entry.files[0].file_name);
    std::fs::write(&tampered, b"tampered evidence").unwrap();
    let sink = FakeSink::default();

    let error = recover_unpublished(&store, &sink).await.unwrap_err();

    assert_eq!(error.batch_id(), Some(entry.batch_id));
    assert_eq!(error.failure_class(), FailureClass::Permanent);
    assert_eq!(error.stage(), PipelineStage::VerifyRaw);
    assert!(tampered.is_file());
    assert!(store.manifest_path(PROVIDER_KRX, MARKET_KR).is_file());
    assert!(sink.publish_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn pipeline_retryable_recovery_state_and_publish_failures_are_typed() {
    let root = tempfile::tempdir().unwrap();
    let store = RawStore::new(root.path());
    let entry = ingest_bundle(&store, &provider(), &request("2026-08-05T07:00:00Z"), None)
        .unwrap()
        .entry;

    let state_sink =
        FakeSink::default().with_state_error(SinkError::RetryableDatabase(sqlx::Error::PoolClosed));
    let state_error = recover_unpublished(&store, &state_sink).await.unwrap_err();
    assert_eq!(state_error.batch_id(), Some(entry.batch_id));
    assert_eq!(state_error.stage(), PipelineStage::PublicationState);
    assert!(state_error.is_retryable());

    let publish_sink = FakeSink::default()
        .with_publish_error(SinkError::RetryableDatabase(sqlx::Error::PoolClosed));
    let publish_error = recover_unpublished(&store, &publish_sink)
        .await
        .unwrap_err();
    assert_eq!(publish_error.batch_id(), Some(entry.batch_id));
    assert_eq!(publish_error.stage(), PipelineStage::Publish);
    assert!(publish_error.is_retryable());
}

#[test]
fn pipeline_error_classification_matrix_is_structural() {
    let provider_cases = [
        (
            ProviderError::EndpointTimeout {
                kind: ResponseKind::Bars,
                timeout_secs: 1,
            },
            true,
        ),
        (
            ProviderError::Io {
                context: "fixture".into(),
                source: std::io::Error::other("offline"),
            },
            true,
        ),
        (
            ProviderError::CredentialsUnavailable {
                credential_ref: CredentialRef::new("env:KRX_CREDENTIAL_REF").0,
                detail: "missing".into(),
            },
            false,
        ),
        (
            ProviderError::UnsafeFileName {
                kind: ResponseKind::Bars,
                file_name: "../x".into(),
            },
            false,
        ),
        (ProviderError::UnsupportedKind(ResponseKind::Bars), false),
        (
            ProviderError::InvalidConfiguration {
                detail: "invalid KIS universe".into(),
            },
            false,
        ),
        (
            ProviderError::Remote {
                provider: market_data::PROVIDER_KIS,
                kind: ResponseKind::Bars,
                code: "RATE_LIMITED",
                retryable: true,
                detail: "retry later".into(),
            },
            true,
        ),
        (
            ProviderError::Remote {
                provider: market_data::PROVIDER_KIS,
                kind: ResponseKind::Bars,
                code: "SCHEMA_DRIFT",
                retryable: false,
                detail: "unexpected response".into(),
            },
            false,
        ),
        (
            ProviderError::RecordedBundleMissing {
                path: "bundle.json".into(),
            },
            false,
        ),
        (
            ProviderError::RecordedBundleIo {
                context: "recorded response".into(),
                path: "missing.json".into(),
                source: std::io::Error::new(std::io::ErrorKind::NotFound, "missing"),
            },
            false,
        ),
        (
            ProviderError::RecordedBundleParse {
                path: "bundle.json".into(),
                source: serde_json::from_str::<serde_json::Value>("{").unwrap_err(),
            },
            false,
        ),
        (
            ProviderError::RecordedBundleInvalid {
                detail: "unknown response kind".into(),
            },
            false,
        ),
    ];
    for (source, retryable) in provider_cases {
        let error = PipelineError::Ingest {
            source: IngestError::Provider(source),
        };
        assert_eq!(error.is_retryable(), retryable, "{error:?}");
    }
    assert!(
        !PipelineError::Ingest {
            source: IngestError::MalformedResponse {
                kind: ResponseKind::Bars,
                reason: "invalid shape".into(),
            },
        }
        .is_retryable()
    );
    assert!(
        PipelineError::Ingest {
            source: IngestError::Store(StoreError::Io {
                context: "write".into(),
                source: std::io::Error::other("offline"),
            }),
        }
        .is_retryable()
    );

    let store_cases = [
        (
            StoreError::Io {
                context: "read".into(),
                source: std::io::Error::other("offline"),
            },
            true,
        ),
        (
            StoreError::CleanupFailed {
                path: "partial-batch".into(),
                original: Box::new(StoreError::Io {
                    context: "write".into(),
                    source: std::io::Error::other("offline"),
                }),
                cleanup: std::io::Error::other("cleanup unavailable"),
            },
            true,
        ),
        (StoreError::FileExists { path: "x".into() }, false),
        (
            StoreError::UnsafeFileName {
                file_name: "../x".into(),
                reason: "unsafe".into(),
            },
            false,
        ),
        (
            StoreError::UnsafeScope {
                component: "provider".into(),
                value: "../x".into(),
                reason: "unsafe".into(),
            },
            false,
        ),
        (
            StoreError::UnsafePath {
                path: "x".into(),
                reason: "escape".into(),
            },
            false,
        ),
        (
            StoreError::ScopeMismatch {
                expected_provider: PROVIDER_KRX.into(),
                expected_market: MARKET_KR.into(),
                actual_provider: "other".into(),
                actual_market: MARKET_KR.into(),
            },
            false,
        ),
        (
            StoreError::ContentHashMismatch {
                path: "x".into(),
                recorded: "a".into(),
                actual: "b".into(),
            },
            false,
        ),
        (
            StoreError::MissingEvidence {
                path: "missing.json".into(),
                source: std::io::Error::new(std::io::ErrorKind::NotFound, "missing"),
            },
            false,
        ),
        (
            StoreError::CorruptManifest {
                path: "manifest.jsonl".into(),
                line: 1,
                source: serde_json::from_str::<serde_json::Value>("not-json").unwrap_err(),
            },
            false,
        ),
        (
            StoreError::ManifestConflict {
                path: "manifest.jsonl".into(),
                batch_id: BatchId::generate(),
            },
            false,
        ),
    ];
    for (source, retryable) in store_cases {
        let error = PipelineError::Manifest { source };
        assert_eq!(error.is_retryable(), retryable, "{error:?}");
    }

    let batch_id = BatchId::generate();
    let io_publication = PipelineError::Publication {
        batch_id,
        source: PublicationError::Store(StoreError::Io {
            context: "read".into(),
            source: std::io::Error::other("offline"),
        }),
    };
    assert!(io_publication.is_retryable());
    let invalid_publication = PipelineError::Publication {
        batch_id,
        source: PublicationError::MalformedCalendar {
            file_name: "calendar.json".into(),
            reason: "bad JSON".into(),
        },
    };
    assert!(!invalid_publication.is_retryable());
    let retryable_sink = PipelineError::Sink {
        batch_id,
        stage: PipelineStage::Publish,
        source: SinkError::RetryableDatabase(sqlx::Error::PoolClosed),
    };
    assert!(retryable_sink.is_retryable());
    let permanent_sink = PipelineError::Sink {
        batch_id,
        stage: PipelineStage::Publish,
        source: SinkError::Conflict("different evidence".into()),
    };
    assert!(!permanent_sink.is_retryable());
}

#[test]
fn cleanup_failure_class_follows_the_original_error_recursively() {
    let permanent = StoreError::CleanupFailed {
        path: "partial-batch".into(),
        original: Box::new(StoreError::FileExists {
            path: "immutable.json".into(),
        }),
        cleanup: std::io::Error::other("cleanup io is secondary"),
    };
    assert_eq!(store_failure_class(&permanent), FailureClass::Permanent);

    let nested_permanent = StoreError::CleanupFailed {
        path: "outer-partial-batch".into(),
        original: Box::new(StoreError::CleanupFailed {
            path: "inner-partial-batch".into(),
            original: Box::new(StoreError::UnsafePath {
                path: "escaped".into(),
                reason: "outside Raw".into(),
            }),
            cleanup: std::io::Error::other("inner cleanup io is secondary"),
        }),
        cleanup: std::io::Error::other("outer cleanup io is secondary"),
    };
    assert_eq!(
        store_failure_class(&nested_permanent),
        FailureClass::Permanent
    );

    let retryable = StoreError::CleanupFailed {
        path: "partial-batch".into(),
        original: Box::new(StoreError::Io {
            context: "write".into(),
            source: std::io::Error::other("temporarily unavailable"),
        }),
        cleanup: std::io::Error::other("cleanup io is secondary"),
    };
    assert_eq!(store_failure_class(&retryable), FailureClass::Retryable);
}

#[test]
fn pipeline_error_sources_traverse_nested_provider_store_and_parse_causes() {
    use std::error::Error as _;

    let error = PipelineError::Ingest {
        source: IngestError::Provider(ProviderError::RecordedBundleParse {
            path: "bundle.json".into(),
            source: serde_json::from_str::<serde_json::Value>("{").unwrap_err(),
        }),
    };

    let ingest = error.source().expect("pipeline retains ingest source");
    let provider = ingest.source().expect("ingest retains provider source");
    let parse = provider.source().expect("provider retains JSON source");
    assert!(parse.downcast_ref::<serde_json::Error>().is_some());

    let store_error = PipelineError::Publication {
        batch_id: BatchId::generate(),
        source: PublicationError::Store(StoreError::MissingEvidence {
            path: "missing.json".into(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "missing"),
        }),
    };
    let publication = store_error
        .source()
        .expect("pipeline retains publication source");
    let store = publication
        .source()
        .expect("publication retains store source");
    let io = store.source().expect("store retains IO source");
    assert!(io.downcast_ref::<std::io::Error>().is_some());
}

#[test]
fn pipeline_post_store_readback_error_retains_batch_source_and_class() {
    use std::error::Error as _;

    let root = tempfile::tempdir().unwrap();
    let store = RawStore::new(root.path());
    let entry = ingest_bundle(&store, &provider(), &request("2026-08-05T07:00:00Z"), None)
        .unwrap()
        .entry;
    let batch_id = entry.batch_id;
    let error = PipelineError::Ingest {
        source: IngestError::Readback {
            entry: Box::new(entry),
            source: StoreError::Io {
                context: "post-store readback".to_owned(),
                source: std::io::Error::other("temporarily unavailable"),
            },
        },
    };

    assert_eq!(error.batch_id(), Some(batch_id));
    assert_eq!(error.failure_class(), FailureClass::Retryable);
    let ingest = error.source().unwrap();
    let store = ingest.source().unwrap();
    let io = store.source().unwrap();
    assert!(io.downcast_ref::<std::io::Error>().is_some());
}

#[test]
fn pipeline_manual_raw_command_needs_no_database_url() {
    let root = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_collectors"))
        .env_remove("DATABASE_URL")
        .args([
            "ingest-krx",
            "--root",
            root.path().to_str().unwrap(),
            "--date",
            "2020-01-31",
            "--mode",
            "synthetic",
            "--bundle",
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/kr-etf/contract"
            ),
            "--now",
            "2026-08-05T07:00:00Z",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "raw command failed");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["provider"], PROVIDER_KRX);
    assert_eq!(json["files"].as_array().unwrap().len(), 4);
}

#[test]
fn pipeline_manual_publish_command_reports_missing_database_url_safely() {
    let root = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_collectors"))
        .env_remove("DATABASE_URL")
        .args([
            "ingest-and-publish-krx",
            "--root",
            root.path().to_str().unwrap(),
            "--date",
            "2020-01-31",
            "--mode",
            "synthetic",
            "--bundle",
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/kr-etf/contract"
            ),
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["status"], "error");
    assert_eq!(json["error_code"], "DATABASE_URL_UNAVAILABLE");
    assert_eq!(json["class"], "permanent");
    assert!(json["message"].as_str().unwrap().contains("DATABASE_URL"));
}

#[test]
fn pipeline_manual_publish_error_redacts_overlapping_database_secrets() {
    let root = tempfile::tempdir().unwrap();
    let password = "review_password";
    let database_url = "postgres://review_user:review_password@review-host.example/review_database";
    let output = Command::new(env!("CARGO_BIN_EXE_collectors"))
        .env("KRX_CREDENTIAL_REF", password)
        .env("DATABASE_URL", database_url)
        .args([
            "ingest-and-publish-krx",
            "--root",
            root.path().to_str().unwrap(),
            "--date",
            "2020-01-31",
            "--mode",
            "synthetic",
            "--bundle",
            database_url,
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error_code"], "PROVIDER_UNAVAILABLE");
    assert_eq!(json["class"], "permanent");
    let visible_output = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for secret_fragment in [
        database_url,
        password,
        "review_user",
        "review-host.example",
        "review_database",
    ] {
        assert!(
            !visible_output.contains(secret_fragment),
            "CLI output leaked a database secret fragment"
        );
    }
}

#[test]
fn pipeline_manual_publish_error_redacts_digest_authorization_parameters() {
    let root = tempfile::tempdir().unwrap();
    let authorization =
        r#"authorization: Digest username="alice", realm="supersecret", response="hashvalue""#;
    let output = Command::new(env!("CARGO_BIN_EXE_collectors"))
        .env(
            "DATABASE_URL",
            "postgres://ignored:ignored@127.0.0.1:1/ignored",
        )
        .args([
            "ingest-and-publish-krx",
            "--root",
            root.path().to_str().unwrap(),
            "--date",
            "2020-01-31",
            "--mode",
            "synthetic",
            "--bundle",
            authorization,
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error_code"], "PROVIDER_UNAVAILABLE");
    assert_eq!(json["class"], "permanent");
    let visible_output = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(visible_output.contains("authorization: [REDACTED]"));
    for secret_fragment in ["alice", "supersecret", "hashvalue", "Digest username"] {
        assert!(
            !visible_output.contains(secret_fragment),
            "CLI output leaked an Authorization credential fragment"
        );
    }
}

#[test]
fn pipeline_manual_durable_manifest_failure_reports_recoverable_batch_id() {
    let root = tempfile::tempdir().unwrap();
    let store = RawStore::new(root.path());
    let manifest_path = store.manifest_path(PROVIDER_KRX, MARKET_KR);
    std::fs::create_dir_all(&manifest_path).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_collectors"))
        .env(
            "DATABASE_URL",
            "postgres://ignored:ignored@127.0.0.1:1/ignored",
        )
        .args([
            "ingest-and-publish-krx",
            "--root",
            root.path().to_str().unwrap(),
            "--date",
            "2020-01-31",
            "--mode",
            "synthetic",
            "--bundle",
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/kr-etf/contract"
            ),
            "--now",
            "2026-08-05T07:00:00Z",
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error_code"], "INGEST_FAILED");
    assert_eq!(json["class"], "retryable");
    let batch_id: BatchId = json["batch_id"].as_str().unwrap().parse().unwrap();
    std::fs::remove_dir(&manifest_path).unwrap();
    let recovered = store.read_manifest(PROVIDER_KRX, MARKET_KR).unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].batch_id, batch_id);
}

fn worker_config(overrides: &[(&str, &str)]) -> HashMap<String, String> {
    let mut values = HashMap::from([
        ("APP_ENV".to_owned(), "qa".to_owned()),
        ("RESEARCH_FETCH_MODE".to_owned(), "synthetic".to_owned()),
        ("RESEARCH_RAW_ROOT".to_owned(), "var/research".to_owned()),
        (
            "RESEARCH_CURATED_ROOT".to_owned(),
            "var/research".to_owned(),
        ),
        (
            "RESEARCH_ENTITLEMENT_REFERENCE".to_owned(),
            "fixture://candidate-license".to_owned(),
        ),
        ("DB_HOST".to_owned(), "127.0.0.1".to_owned()),
        ("DB_PORT".to_owned(), "55432".to_owned()),
        ("DB_NAME".to_owned(), "lagrange".to_owned()),
        ("DB_USER".to_owned(), "research_writer".to_owned()),
        ("DB_PASSWORD_FILE".to_owned(), "db-password".to_owned()),
    ]);
    for (key, value) in overrides {
        values.insert((*key).to_owned(), (*value).to_owned());
    }
    values
}

#[test]
fn worker_config_parses_defaults_and_trims_spaces_from_file_secret() {
    let values = worker_config(&[]);
    let config = ResearchWorkerConfig::from_map_with_reader(&values, |path| {
        assert_eq!(path.to_string_lossy(), "db-password");
        Ok("  password from file  ".to_owned())
    })
    .unwrap();

    assert_eq!(config.app_env, AppEnvironment::Qa);
    assert_eq!(config.fetch_mode, market_data::FetchMode::Synthetic);
    assert_eq!(config.run_at_kst.format("%H:%M").to_string(), "16:30");
    assert_eq!(config.max_publication_age.as_secs(), 345_600);
    assert_eq!(config.attempt_timeout.as_secs(), 900);
    assert_eq!(config.raw_root.to_string_lossy(), "var/research");
    assert_eq!(config.database.host, "127.0.0.1");
    assert_eq!(config.database.port, 55432);
    assert_eq!(config.database.name, "lagrange");
    assert_eq!(config.database.user, "research_writer");
    assert_eq!(config.database.password.expose(), "password from file");
    assert!(!config.candidate_sources_enabled);
    assert_eq!(
        config.candidate_raw_root.to_string_lossy(),
        "var/research/candidate"
    );
    assert!(!format!("{config:?}").contains("password from file"));
}

#[test]
fn worker_config_enables_separate_candidate_raw_and_fixture_paths() {
    let values = worker_config(&[
        ("RESEARCH_CANDIDATE_ENABLED", "true"),
        ("RESEARCH_CANDIDATE_RAW_ROOT", "var/candidate-raw"),
        ("RESEARCH_CANDIDATE_SYNTHETIC_BUNDLE", "fixtures/candidates"),
    ]);
    let config =
        ResearchWorkerConfig::from_map_with_reader(&values, |_| Ok("password".to_owned())).unwrap();
    assert!(config.candidate_sources_enabled);
    assert_eq!(
        config.candidate_raw_root.to_string_lossy(),
        "var/candidate-raw"
    );
    assert_eq!(
        config.candidate_synthetic_bundle.to_string_lossy(),
        "fixtures/candidates"
    );
}

#[test]
fn credentialed_candidate_sources_are_rejected_until_their_provider_exists() {
    let values = worker_config(&[
        ("APP_ENV", "development"),
        ("RESEARCH_FETCH_MODE", "credentialed"),
        ("RESEARCH_CANDIDATE_ENABLED", "true"),
        ("KIS_APP_KEY_FILE", "kis-app-key"),
        ("KIS_APP_SECRET_FILE", "kis-app-secret"),
    ]);
    let error =
        ResearchWorkerConfig::from_map_with_reader(&values, |_| Ok("valid-secret".to_owned()))
            .expect_err("credentialed candidate mode must fail closed");
    assert!(matches!(
        error,
        WorkerError::InvalidConfig {
            key: "RESEARCH_CANDIDATE_ENABLED"
        }
    ));
    assert!(!error.to_string().contains("valid-secret"));
}

#[test]
fn worker_config_parses_schedule_and_max_age_overrides() {
    let values = worker_config(&[
        ("APP_ENV", "development"),
        ("RESEARCH_FETCH_MODE", "credentialed"),
        ("RESEARCH_RUN_AT_KST", "07:05"),
        ("RESEARCH_MAX_PUBLICATION_AGE_SECS", "42"),
        ("RESEARCH_ATTEMPT_TIMEOUT_SECS", "600"),
        ("KIS_APP_KEY_FILE", "kis-app-key"),
        ("KIS_APP_SECRET_FILE", "kis-app-secret"),
    ]);
    let config = ResearchWorkerConfig::from_map_with_reader(&values, |path| {
        Ok(match path.to_string_lossy().as_ref() {
            "db-password" => "db-secret",
            "kis-app-key" => "app-key-value",
            "kis-app-secret" => "app-secret-value",
            other => panic!("unexpected secret path {other}"),
        }
        .to_owned())
    })
    .unwrap();

    assert_eq!(config.app_env, AppEnvironment::Development);
    assert_eq!(config.fetch_mode, market_data::FetchMode::Credentialed);
    assert_eq!(config.run_at_kst.format("%H:%M").to_string(), "07:05");
    assert_eq!(config.max_publication_age.as_secs(), 42);
    assert_eq!(config.attempt_timeout.as_secs(), 600);
    assert_eq!(
        config.kis_app_key_file.as_deref(),
        Some(std::path::Path::new("kis-app-key"))
    );
    assert_eq!(
        config.kis_app_secret_file.as_deref(),
        Some(std::path::Path::new("kis-app-secret"))
    );
}

#[test]
fn credentialed_worker_config_requires_both_kis_credentials() {
    for missing_key in ["KIS_APP_KEY_FILE", "KIS_APP_SECRET_FILE"] {
        let mut values = worker_config(&[
            ("APP_ENV", "development"),
            ("RESEARCH_FETCH_MODE", "credentialed"),
            ("KIS_APP_KEY_FILE", "kis-app-key"),
            ("KIS_APP_SECRET_FILE", "kis-app-secret"),
        ]);
        values.remove(missing_key);

        let error =
            ResearchWorkerConfig::from_map_with_reader(&values, |_| Ok("valid-secret".to_owned()))
                .unwrap_err();
        assert!(matches!(
            error,
            WorkerError::MissingConfig { key } if key == missing_key
        ));
    }
}

#[test]
fn worker_config_rejects_invalid_and_missing_values_without_secret_contents() {
    for (key, value) in [
        ("APP_ENV", "staging"),
        ("RESEARCH_FETCH_MODE", "auto"),
        ("RESEARCH_CANDIDATE_ENABLED", "yes"),
        ("RESEARCH_RUN_AT_KST", "25:00"),
        ("RESEARCH_MAX_PUBLICATION_AGE_SECS", "0"),
        ("RESEARCH_ATTEMPT_TIMEOUT_SECS", "59"),
        ("DB_PORT", "70000"),
        ("RESEARCH_RAW_ROOT", "  "),
        ("RESEARCH_CURATED_ROOT", "  "),
        ("RESEARCH_ENTITLEMENT_REFERENCE", "  "),
    ] {
        let values = worker_config(&[(key, value)]);
        let error = ResearchWorkerConfig::from_map_with_reader(&values, |_| {
            Ok("do-not-disclose".to_owned())
        })
        .unwrap_err();
        assert!(matches!(error, WorkerError::InvalidConfig { .. }));
        assert!(!error.to_string().contains("do-not-disclose"));
    }

    for failure in [
        std::io::Error::new(std::io::ErrorKind::NotFound, "secret-value"),
        std::io::Error::new(std::io::ErrorKind::PermissionDenied, "secret-value"),
    ] {
        let values = worker_config(&[]);
        let error = ResearchWorkerConfig::from_map_with_reader(&values, |_| {
            Err(std::io::Error::new(failure.kind(), "secret-value"))
        })
        .unwrap_err();
        assert!(matches!(error, WorkerError::SecretFile { .. }));
        assert!(!error.to_string().contains("secret-value"));
    }

    let error = ResearchWorkerConfig::from_map_with_reader(&worker_config(&[]), |_| {
        Ok(" \r\n\t".to_owned())
    })
    .unwrap_err();
    assert!(matches!(error, WorkerError::SecretFile { .. }));
}

#[test]
fn worker_config_rejects_any_line_break_in_secret_files() {
    for value in ["password\n", "password\r", "password\r\n", "pass\nword"] {
        let error = ResearchWorkerConfig::from_map_with_reader(&worker_config(&[]), |_| {
            Ok(value.to_owned())
        })
        .unwrap_err();
        assert!(matches!(error, WorkerError::SecretFile { .. }));
    }

    let values = worker_config(&[
        ("APP_ENV", "development"),
        ("RESEARCH_FETCH_MODE", "credentialed"),
        ("KIS_APP_KEY_FILE", "kis-app-key"),
        ("KIS_APP_SECRET_FILE", "kis-app-secret"),
    ]);
    for secret_path in ["kis-app-key", "kis-app-secret"] {
        for value in ["provider\nsecret", "provider\rsecret"] {
            let error = ResearchWorkerConfig::from_map_with_reader(&values, |path| {
                Ok(if path.to_string_lossy() == secret_path {
                    value.to_owned()
                } else {
                    "valid-secret".to_owned()
                })
            })
            .unwrap_err();
            assert!(matches!(error, WorkerError::SecretFile { .. }));
        }
    }
}

#[test]
fn worker_synthetic_policy_is_fail_closed_and_precedes_secret_reads() {
    for app_env in ["production", "staging", "", "prod"] {
        let values = worker_config(&[("APP_ENV", app_env)]);
        let reads = std::cell::Cell::new(0);
        let error = ResearchWorkerConfig::from_map_with_reader(&values, |_| {
            reads.set(reads.get() + 1);
            Ok("must-not-be-read".to_owned())
        })
        .unwrap_err();
        assert_eq!(reads.get(), 0, "policy must precede secret reads");
        if app_env == "production" {
            assert!(matches!(error, WorkerError::SyntheticForbidden { .. }));
        } else {
            assert!(matches!(error, WorkerError::InvalidConfig { .. }));
        }
    }

    assert!(
        validate_synthetic_policy(AppEnvironment::Qa, market_data::FetchMode::Synthetic).is_ok()
    );
    assert!(
        validate_synthetic_policy(
            AppEnvironment::Development,
            market_data::FetchMode::Synthetic
        )
        .is_ok()
    );
    assert!(matches!(
        validate_synthetic_policy(
            AppEnvironment::Production,
            market_data::FetchMode::Synthetic
        ),
        Err(WorkerError::SyntheticForbidden { .. })
    ));
}

#[derive(Default)]
struct ConstructionSpy {
    provider: AtomicUsize,
    store: AtomicUsize,
    pool: AtomicUsize,
}

impl WorkerComponentFactory for ConstructionSpy {
    fn build_provider(
        &self,
        _config: &ResearchWorkerConfig,
    ) -> Result<Arc<dyn EodProvider>, WorkerError> {
        self.provider.fetch_add(1, Ordering::SeqCst);
        panic!("provider construction must be fenced")
    }

    fn build_store(&self, _config: &ResearchWorkerConfig) -> Result<RawStore, WorkerError> {
        self.store.fetch_add(1, Ordering::SeqCst);
        panic!("Raw store construction must be fenced")
    }

    fn build_pool(&self, _config: &ResearchWorkerConfig) -> Result<PgPool, WorkerError> {
        self.pool.fetch_add(1, Ordering::SeqCst);
        panic!("pool construction must be fenced")
    }
}

#[test]
fn worker_production_fence_precedes_all_construction_and_filesystem_access() {
    let values = worker_config(&[("APP_ENV", "production")]);
    let reads = AtomicUsize::new(0);
    let factory = ConstructionSpy::default();

    let error = bootstrap_worker_with(
        &values,
        |_| {
            reads.fetch_add(1, Ordering::SeqCst);
            panic!("secret file must not be read")
        },
        &factory,
    )
    .err()
    .expect("production synthetic is rejected");

    assert!(matches!(error, WorkerError::SyntheticForbidden { .. }));
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    assert_eq!(factory.provider.load(Ordering::SeqCst), 0);
    assert_eq!(factory.store.load(Ordering::SeqCst), 0);
    assert_eq!(factory.pool.load(Ordering::SeqCst), 0);
}

#[test]
fn worker_schedule_uses_fixed_kst_civil_time_and_exponential_backoff() {
    let before = Utc.with_ymd_and_hms(2026, 8, 10, 7, 29, 30).unwrap();
    let at = chrono::NaiveTime::parse_from_str("16:30", "%H:%M").unwrap();
    assert_eq!(
        next_run_delay(before, at),
        std::time::Duration::from_secs(30)
    );

    let passed = Utc.with_ymd_and_hms(2026, 8, 10, 7, 30, 1).unwrap();
    assert_eq!(
        next_run_delay(passed, at),
        std::time::Duration::from_secs(86_399)
    );

    let delays: Vec<_> = (0..9)
        .map(|failure| retry_delay(failure).as_secs())
        .collect();
    assert_eq!(delays, vec![10, 20, 40, 80, 160, 320, 600, 600, 600]);

    assert_eq!(
        collectors::current_kst_date(Utc.with_ymd_and_hms(2026, 8, 9, 15, 30, 0).unwrap()).to_iso(),
        "2026-08-10"
    );
}

#[test]
fn worker_cli_help_and_argument_errors_are_stable_json() {
    let help = Command::new(env!("CARGO_BIN_EXE_research-worker"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    let help_text = String::from_utf8(help.stdout).unwrap();
    assert!(help_text.contains("--once --date YYYY-MM-DD"));
    assert!(help_text.contains("healthcheck"));

    let missing_date = Command::new(env!("CARGO_BIN_EXE_research-worker"))
        .arg("--once")
        .output()
        .unwrap();
    assert_eq!(missing_date.status.code(), Some(2));
    let error: serde_json::Value = serde_json::from_slice(&missing_date.stdout).unwrap();
    assert_eq!(error["phase"], "config");
    assert_eq!(error["class"], "permanent");
    assert!(error["batch_id"].is_null());
    assert_eq!(error["provider"], "KRX");
    assert_eq!(error["market"], "KR");
    assert!(error["target_date"].is_null());
}

#[test]
fn worker_cli_uses_discrete_db_keys_and_enforces_fence_before_secret_files() {
    let output = Command::new(env!("CARGO_BIN_EXE_research-worker"))
        .env_clear()
        .env("APP_ENV", "production")
        .env("RESEARCH_FETCH_MODE", "synthetic")
        .env(
            "DATABASE_URL",
            "postgres://leaked-user:leaked-password@leaked-host/db",
        )
        .args(["--once", "--date", "2020-01-31"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error_code"], "SYNTHETIC_FORBIDDEN");
    assert_eq!(json["phase"], "config");
    assert_eq!(json["provider"], "KRX");
    assert_eq!(json["market"], "KR");
    assert_eq!(json["target_date"], "2020-01-31");
    let visible = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for secret in ["leaked-user", "leaked-password", "leaked-host"] {
        assert!(!visible.contains(secret));
    }
}

#[test]
fn worker_cli_healthcheck_has_no_provider_or_raw_configuration_dependency() {
    let output = Command::new(env!("CARGO_BIN_EXE_research-worker"))
        .env_clear()
        .env(
            "DATABASE_URL",
            "postgres://ignored:ignored@127.0.0.1/ignored",
        )
        .arg("healthcheck")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error_code"], "MISSING_CONFIG");
    assert_eq!(json["phase"], "config");
    assert_eq!(json["provider"], "KRX");
    assert_eq!(json["market"], "KR");
    assert!(json["target_date"].is_null());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("DATABASE_URL"));
}

#[tokio::test]
async fn worker_cli_once_runs_collection_in_the_bounded_hidden_helper() {
    let qa_url = std::env::var("DATABASE_URL")
        .expect("required QA DATABASE_URL must be provided for worker process verification");
    assert_eq!(
        qa_url, "postgres://postgres:lagrange@127.0.0.1:55432/postgres",
        "worker process verification must use the reviewed local PostgreSQL endpoint"
    );
    let db = ScratchDb::create()
        .await
        .expect("required QA PostgreSQL must be reachable");
    seed_candidate_entitlement(&db.supervisor).await;
    let workspace = tempfile::tempdir().unwrap();
    let raw_root = workspace.path().join("raw");
    let password_file = workspace.path().join("db-password");
    let stdout_file = workspace.path().join("worker-stdout");
    let stderr_file = workspace.path().join("worker-stderr");
    std::fs::write(&password_file, "lagrange").unwrap();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/kr-etf/contract")
        .canonicalize()
        .unwrap();

    let mut command = Command::new(env!("CARGO_BIN_EXE_research-worker"));
    command
        .env_clear()
        .env("APP_ENV", "development")
        .env("RESEARCH_FETCH_MODE", "synthetic")
        .env("RESEARCH_RAW_ROOT", &raw_root)
        .env("RESEARCH_CURATED_ROOT", &raw_root)
        .env(
            "RESEARCH_ENTITLEMENT_REFERENCE",
            "fixture://candidate-license",
        )
        .env("RESEARCH_SYNTHETIC_BUNDLE", fixture)
        .env("DB_HOST", "127.0.0.1")
        .env("DB_PORT", "55432")
        .env("DB_NAME", db.database_name())
        .env("DB_USER", "research_writer")
        .env("DB_PASSWORD_FILE", &password_file)
        .args(["--once", "--date", "2020-01-31"])
        .stdout(std::fs::File::create(&stdout_file).unwrap())
        .stderr(std::fs::File::create(&stderr_file).unwrap());
    #[cfg(windows)]
    command.env("SYSTEMROOT", std::env::var("SYSTEMROOT").unwrap());
    let mut child = command.spawn().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!(
                "worker process exceeded watchdog; stdout events: {}; stderr: {}",
                std::fs::read_to_string(&stdout_file).unwrap(),
                std::fs::read_to_string(&stderr_file).unwrap()
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let stdout = std::fs::read(&stdout_file).unwrap();
    let stderr = std::fs::read(&stderr_file).unwrap();

    assert!(
        status.success(),
        "sanitized worker output: {}",
        String::from_utf8_lossy(&stdout)
    );
    assert!(stderr.is_empty());
    let records = String::from_utf8(stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 2, "completed event and final outcome");
    assert_eq!(records[0]["event"], "completed");
    assert_eq!(records[0]["provider"], "KRX");
    assert_eq!(records[0]["market"], "KR");
    assert_eq!(records[0]["target_date"], "2020-01-31");
    assert_eq!(records[1]["status"], "ok");
    assert_eq!(records[1]["outcome"], "published");
    assert_eq!(records[0]["batch_id"], records[1]["batch_id"]);

    let store = RawStore::new(raw_root);
    let entries = store.read_manifest(PROVIDER_KRX, MARKET_KR).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(records[1]["batch_id"], entries[0].batch_id.to_string());

    db.drop_db().await;
}

#[tokio::test]
async fn worker_cli_permanent_ingest_failure_streams_failed_then_contextual_error() {
    let qa_url = std::env::var("DATABASE_URL")
        .expect("required QA DATABASE_URL must be provided for worker error verification");
    assert_eq!(
        qa_url,
        "postgres://postgres:lagrange@127.0.0.1:55432/postgres"
    );
    let db = ScratchDb::create()
        .await
        .expect("required QA PostgreSQL must be reachable");
    seed_candidate_entitlement(&db.supervisor).await;
    let workspace = tempfile::tempdir().unwrap();
    let raw_root = workspace.path().join("raw");
    let password_file = workspace.path().join("db-password");
    std::fs::write(&password_file, "lagrange").unwrap();
    let malformed_fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/kr-etf/contract-variants/malformed-bars")
        .canonicalize()
        .unwrap();

    let mut command = Command::new(env!("CARGO_BIN_EXE_research-worker"));
    command
        .env_clear()
        .env("APP_ENV", "development")
        .env("RESEARCH_FETCH_MODE", "synthetic")
        .env("RESEARCH_RAW_ROOT", &raw_root)
        .env("RESEARCH_CURATED_ROOT", &raw_root)
        .env(
            "RESEARCH_ENTITLEMENT_REFERENCE",
            "fixture://candidate-license",
        )
        .env("RESEARCH_SYNTHETIC_BUNDLE", malformed_fixture)
        .env("DB_HOST", "127.0.0.1")
        .env("DB_PORT", "55432")
        .env("DB_NAME", db.database_name())
        .env("DB_USER", "research_writer")
        .env("DB_PASSWORD_FILE", &password_file)
        .args(["--once", "--date", "2020-01-31"]);
    #[cfg(windows)]
    command.env("SYSTEMROOT", std::env::var("SYSTEMROOT").unwrap());
    let output = command.output().unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stderr.is_empty());
    let records = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 2, "failed event followed by final error");
    assert_eq!(records[0]["event"], "failed");
    assert_eq!(records[0]["class"], "permanent");
    assert_eq!(records[1]["status"], "error");
    assert_eq!(records[1]["class"], "permanent");
    for record in &records {
        assert_eq!(record["provider"], "KRX");
        assert_eq!(record["market"], "KR");
        assert_eq!(record["target_date"], "2020-01-31");
        assert_eq!(record["phase"], "ingest");
    }
    assert_eq!(records[0]["batch_id"], records[1]["batch_id"]);

    db.drop_db().await;
}

#[tokio::test]
async fn worker_cli_streams_each_validated_recovery_batch_before_cycle_output() {
    let qa_url = std::env::var("DATABASE_URL")
        .expect("required QA DATABASE_URL must be provided for recovery stream verification");
    assert_eq!(
        qa_url, "postgres://postgres:lagrange@127.0.0.1:55432/postgres",
        "recovery stream verification must use the reviewed local PostgreSQL endpoint"
    );
    let db = ScratchDb::create()
        .await
        .expect("required QA PostgreSQL must be reachable");
    seed_candidate_entitlement(&db.supervisor).await;
    let workspace = tempfile::tempdir().unwrap();
    let raw_root = workspace.path().join("raw");
    let password_file = workspace.path().join("db-password");
    std::fs::write(&password_file, "lagrange").unwrap();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/kr-etf/contract")
        .canonicalize()
        .unwrap();
    let store = RawStore::new(&raw_root);
    let orphan_date = TradingDate::parse("2020-01-31").unwrap();
    let orphan_request = IngestRequest::new(
        MARKET_KR.to_owned(),
        orphan_date,
        UtcTimestamp::parse_rfc3339("2026-08-05T07:00:00Z").unwrap(),
    );
    let orphan = ingest_bundle(
        &store,
        &provider(),
        &orphan_request,
        Some("fixture://candidate-license"),
    )
    .unwrap()
    .entry;
    sqlx::query(
        "INSERT INTO data_batches
         (provider, market, batch_date, kind, fetch_mode, storage_path,
          content_sha256, bytes_size, retrieved_at, source_batch_id, source_file_name) VALUES
         ('KRX','KR','2020-02-03','EOD','synthetic','raw/already-published',
          repeat('a',64),1,$1,gen_random_uuid(),'already-published.json')",
    )
    .bind(Utc::now())
    .execute(&db.supervisor)
    .await
    .unwrap();

    for expected_recovery_event in ["recovered", "skipped"] {
        let stdout_file = workspace
            .path()
            .join(format!("{expected_recovery_event}-stdout"));
        let stderr_file = workspace
            .path()
            .join(format!("{expected_recovery_event}-stderr"));
        let mut command = Command::new(env!("CARGO_BIN_EXE_research-worker"));
        command
            .env_clear()
            .env("APP_ENV", "development")
            .env("RESEARCH_FETCH_MODE", "synthetic")
            .env("RESEARCH_RAW_ROOT", &raw_root)
            .env("RESEARCH_CURATED_ROOT", &raw_root)
            .env(
                "RESEARCH_ENTITLEMENT_REFERENCE",
                "fixture://candidate-license",
            )
            .env("RESEARCH_SYNTHETIC_BUNDLE", &fixture)
            .env("DB_HOST", "127.0.0.1")
            .env("DB_PORT", "55432")
            .env("DB_NAME", db.database_name())
            .env("DB_USER", "research_writer")
            .env("DB_PASSWORD_FILE", &password_file)
            .args(["--once", "--date", "2020-02-03"])
            .stdout(std::fs::File::create(&stdout_file).unwrap())
            .stderr(std::fs::File::create(&stderr_file).unwrap());
        #[cfg(windows)]
        command.env("SYSTEMROOT", std::env::var("SYSTEMROOT").unwrap());
        let mut child = command.spawn().unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if std::time::Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("recovery stream worker exceeded watchdog");
            }
            std::thread::sleep(Duration::from_millis(25));
        };
        assert!(
            status.success(),
            "worker failed: stdout={} stderr={}",
            std::fs::read_to_string(&stdout_file).unwrap(),
            std::fs::read_to_string(&stderr_file).unwrap()
        );
        assert!(std::fs::read(&stderr_file).unwrap().is_empty());
        let stdout = std::fs::read_to_string(&stdout_file).unwrap();
        let records = stdout
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(records.len(), 3, "recovery, cycle skip, final outcome");
        assert_eq!(records[0]["event"], expected_recovery_event);
        assert_eq!(records[0]["class"], "success");
        assert_eq!(records[0]["phase"], "recovery");
        assert_eq!(records[0]["target_date"], orphan_date.to_iso());
        assert_eq!(records[0]["batch_id"], orphan.batch_id.to_string());
        assert_eq!(records[1]["event"], "skipped");
        assert_eq!(records[1]["target_date"], "2020-02-03");
        assert!(records[1]["batch_id"].is_null());
        assert_eq!(records[2]["outcome"], "already_published");
    }

    db.drop_db().await;
}

#[tokio::test]
async fn worker_recovery_write_failure_is_retryable_and_exact_replay_is_skipped() {
    let qa_url = std::env::var("DATABASE_URL")
        .expect("required QA DATABASE_URL must be provided for recovery writer verification");
    assert_eq!(
        qa_url,
        "postgres://postgres:lagrange@127.0.0.1:55432/postgres"
    );
    let db = ScratchDb::create()
        .await
        .expect("required QA PostgreSQL must be reachable");
    seed_candidate_entitlement(&db.supervisor).await;
    let workspace = tempfile::tempdir().unwrap();
    let raw_root = workspace.path().join("raw");
    let password_file = workspace.path().join("db-password");
    std::fs::write(&password_file, "lagrange").unwrap();
    let store = RawStore::new(&raw_root);
    let orphan = ingest_bundle(
        &store,
        &provider(),
        &request("2026-08-05T07:00:00Z"),
        Some("fixture://candidate-license"),
    )
    .unwrap()
    .entry;
    let mut values = worker_config(&[("APP_ENV", "development")]);
    values.insert(
        "RESEARCH_RAW_ROOT".to_owned(),
        raw_root.to_string_lossy().into_owned(),
    );
    values.insert(
        "RESEARCH_CURATED_ROOT".to_owned(),
        raw_root.to_string_lossy().into_owned(),
    );
    values.insert("DB_HOST".to_owned(), "127.0.0.1".to_owned());
    values.insert("DB_PORT".to_owned(), "55432".to_owned());
    values.insert("DB_NAME".to_owned(), db.database_name().to_owned());
    values.insert("DB_USER".to_owned(), "research_writer".to_owned());
    values.insert(
        "DB_PASSWORD_FILE".to_owned(),
        password_file.to_string_lossy().into_owned(),
    );

    let error = run_internal_recovery_stream(&values, &mut AlwaysFailWriter)
        .await
        .unwrap_err();
    assert_eq!(error.phase(), WorkerPhase::Recovery);
    assert_eq!(error.failure_class(), FailureClass::Retryable);

    let mut replay_output = Vec::new();
    run_internal_recovery_stream(&values, &mut replay_output)
        .await
        .unwrap();
    let replay: serde_json::Value = serde_json::from_slice(&replay_output).unwrap();
    assert_eq!(replay["event"], "skipped");
    assert_eq!(replay["batch_id"], orphan.batch_id.to_string());
    assert_eq!(replay["target_date"], orphan.date.to_iso());

    db.drop_db().await;
}

#[tokio::test]
async fn worker_price_recovery_rejects_future_sessions_before_catalog_publication() {
    let qa_url = std::env::var("DATABASE_URL")
        .expect("required QA DATABASE_URL must be provided for no-lookahead verification");
    assert_eq!(
        qa_url,
        "postgres://postgres:lagrange@127.0.0.1:55432/postgres"
    );
    let db = ScratchDb::create()
        .await
        .expect("required QA PostgreSQL must be reachable");
    seed_candidate_entitlement(&db.supervisor).await;
    let workspace = tempfile::tempdir().unwrap();
    let raw_root = workspace.path().join("raw");
    let password_file = workspace.path().join("db-password");
    std::fs::write(&password_file, "lagrange").unwrap();
    let store = RawStore::new(&raw_root);
    let future_batch = ingest_bundle(
        &store,
        &provider(),
        &IngestRequest::new(
            MARKET_KR.to_owned(),
            TradingDate::parse("2020-01-30").unwrap(),
            UtcTimestamp::parse_rfc3339("2026-08-05T07:00:00Z").unwrap(),
        ),
        Some("fixture://candidate-license"),
    )
    .unwrap()
    .entry;
    let mut values = worker_config(&[("APP_ENV", "development")]);
    values.insert(
        "RESEARCH_RAW_ROOT".to_owned(),
        raw_root.to_string_lossy().into_owned(),
    );
    values.insert(
        "RESEARCH_CURATED_ROOT".to_owned(),
        raw_root.to_string_lossy().into_owned(),
    );
    values.insert("DB_NAME".to_owned(), db.database_name().to_owned());
    values.insert(
        "DB_PASSWORD_FILE".to_owned(),
        password_file.to_string_lossy().into_owned(),
    );

    let error = run_internal_recovery_stream(&values, &mut Vec::new())
        .await
        .expect_err("a target-date batch must not publish a future price session");
    assert_eq!(error.failure_class(), FailureClass::Permanent);
    let price_ledger: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM candidate_raw_batch_publications
          WHERE batch_id=$1 AND surface='price'",
    )
    .bind(future_batch.batch_id.as_uuid())
    .fetch_one(&db.supervisor)
    .await
    .expect("no-lookahead price ledger check");
    assert_eq!(price_ledger, 0);
    db.drop_db().await;
}

#[derive(Default)]
struct FakeResearchBackend {
    events: Mutex<Vec<String>>,
    recover: Mutex<VecDeque<Result<(), WorkerError>>>,
    recovered: Mutex<VecDeque<Vec<(BatchId, TradingDate)>>>,
    recovery_skipped: Mutex<VecDeque<Vec<(BatchId, TradingDate)>>>,
    eod: Mutex<VecDeque<Result<bool, WorkerError>>>,
    ingest: Mutex<VecDeque<Result<BatchId, WorkerError>>>,
}

#[async_trait]
impl ResearchBackend for FakeResearchBackend {
    async fn recover(
        &self,
        _control: &dyn WorkerControl,
        observer: &dyn RecoveryObserver,
    ) -> Result<(), WorkerError> {
        self.events.lock().unwrap().push("recover".to_owned());
        let result = self.recover.lock().unwrap().pop_front().unwrap_or(Ok(()));
        if result.is_ok() {
            for (batch_id, date) in self
                .recovered
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_default()
            {
                observer.recovered(batch_id, date);
            }
            for (batch_id, date) in self
                .recovery_skipped
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_default()
            {
                observer.skipped(batch_id, date);
            }
        }
        result
    }

    async fn has_eod(&self, date: TradingDate) -> Result<bool, WorkerError> {
        self.events
            .lock()
            .unwrap()
            .push(format!("has_eod:{}", date.to_iso()));
        self.eod.lock().unwrap().pop_front().unwrap_or(Ok(false))
    }

    async fn ingest(
        &self,
        date: TradingDate,
        _now: UtcTimestamp,
        _control: &dyn WorkerControl,
    ) -> Result<BatchId, WorkerError> {
        self.events
            .lock()
            .unwrap()
            .push(format!("ingest:{}", date.to_iso()));
        self.ingest
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Ok(BatchId::generate()))
    }
}

#[derive(Default)]
struct FakeControl {
    sleeps: Mutex<Vec<Duration>>,
    shutdown_on_sleep: bool,
}

struct ScheduleControl {
    now: chrono::DateTime<Utc>,
    sleeps: Mutex<Vec<Duration>>,
}

struct OneCycleControl {
    now: chrono::DateTime<Utc>,
}

struct OneScheduledCycleControl {
    now: chrono::DateTime<Utc>,
    scheduled_waits: AtomicUsize,
}

#[async_trait]
impl WorkerControl for OneCycleControl {
    fn now_utc(&self) -> chrono::DateTime<Utc> {
        self.now
    }

    async fn wait(&self, duration: Option<Duration>) -> WaitOutcome {
        if duration.is_some() {
            WaitOutcome::Elapsed
        } else {
            std::future::pending().await
        }
    }
}

#[async_trait]
impl WorkerControl for OneScheduledCycleControl {
    fn now_utc(&self) -> chrono::DateTime<Utc> {
        self.now
    }

    async fn wait(&self, duration: Option<Duration>) -> WaitOutcome {
        if duration.is_none() {
            return std::future::pending().await;
        }
        if self.scheduled_waits.fetch_add(1, Ordering::SeqCst) == 0 {
            WaitOutcome::Elapsed
        } else {
            WaitOutcome::Shutdown
        }
    }
}

#[async_trait]
impl WorkerControl for ScheduleControl {
    fn now_utc(&self) -> chrono::DateTime<Utc> {
        self.now
    }

    async fn wait(&self, duration: Option<Duration>) -> WaitOutcome {
        match duration {
            Some(duration) => {
                self.sleeps.lock().unwrap().push(duration);
                WaitOutcome::Shutdown
            }
            None => std::future::pending().await,
        }
    }
}

#[async_trait]
impl WorkerControl for FakeControl {
    fn now_utc(&self) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 10, 7, 30, 0).unwrap()
    }

    async fn wait(&self, duration: Option<Duration>) -> WaitOutcome {
        match duration {
            Some(duration) => {
                self.sleeps.lock().unwrap().push(duration);
                if self.shutdown_on_sleep {
                    WaitOutcome::Shutdown
                } else {
                    WaitOutcome::Elapsed
                }
            }
            None => std::future::pending().await,
        }
    }
}

fn configured_worker(backend: Arc<dyn ResearchBackend>) -> ResearchWorker {
    let config = ResearchWorkerConfig::from_map_with_reader(&worker_config(&[]), |_| {
        Ok("password".to_owned())
    })
    .unwrap();
    ResearchWorker::new(config, backend)
}

struct BudgetedCompleteRecoveryBackend {
    batches: Vec<BatchId>,
    cursor: AtomicUsize,
    observed: Mutex<Vec<BatchId>>,
    fetches: AtomicUsize,
}

#[async_trait]
impl ResearchBackend for BudgetedCompleteRecoveryBackend {
    async fn recover(
        &self,
        _control: &dyn WorkerControl,
        observer: &dyn RecoveryObserver,
    ) -> Result<(), WorkerError> {
        let start = self.cursor.load(Ordering::SeqCst);
        let end = start
            .saturating_add(RECOVERY_PAGE_SIZE)
            .min(self.batches.len());
        let date = TradingDate::parse("2020-01-31").unwrap();
        for batch_id in self.batches[start..end].iter().copied() {
            observer.skipped(batch_id, date);
            self.observed.lock().unwrap().push(batch_id);
            self.cursor
                .store(self.cursor.load(Ordering::SeqCst) + 1, Ordering::SeqCst);
        }
        if end < self.batches.len() {
            // Models one helper's fixed 60-second budget expiring after a
            // validated page. Restarting from the oldest batch could never
            // finish this history; retaining the cursor advances every retry.
            Err(WorkerError::Timeout {
                phase: WorkerPhase::Recovery,
            })
        } else {
            Ok(())
        }
    }

    async fn has_eod(&self, _date: TradingDate) -> Result<bool, WorkerError> {
        Ok(false)
    }

    async fn ingest(
        &self,
        _date: TradingDate,
        _now: UtcTimestamp,
        _control: &dyn WorkerControl,
    ) -> Result<BatchId, WorkerError> {
        self.fetches.fetch_add(1, Ordering::SeqCst);
        Ok("00000000-0000-4000-8000-000000000099".parse().unwrap())
    }
}

#[tokio::test]
async fn recovery_cursor_advances_across_multiple_helper_budgets_then_fetches() {
    let batches = (1..=(RECOVERY_PAGE_SIZE * 2 + 1))
        .map(|index| {
            format!("00000000-0000-4000-8000-{index:012}")
                .parse::<BatchId>()
                .unwrap()
        })
        .collect::<Vec<_>>();
    let backend = Arc::new(BudgetedCompleteRecoveryBackend {
        batches: batches.clone(),
        cursor: AtomicUsize::new(0),
        observed: Mutex::new(Vec::new()),
        fetches: AtomicUsize::new(0),
    });
    let control = FakeControl::default();

    let outcome = configured_worker(backend.clone())
        .run_once(TradingDate::parse("2020-01-31").unwrap(), &control)
        .await
        .unwrap();

    assert!(matches!(outcome, WorkerRunOutcome::Published(_)));
    assert_eq!(*backend.observed.lock().unwrap(), batches);
    assert_eq!(
        backend.cursor.load(Ordering::SeqCst),
        RECOVERY_PAGE_SIZE * 2 + 1
    );
    assert_eq!(backend.fetches.load(Ordering::SeqCst), 1);
    assert_eq!(
        *control.sleeps.lock().unwrap(),
        vec![Duration::from_secs(10), Duration::from_secs(20)]
    );
}

#[tokio::test]
async fn worker_once_recovers_first_and_skips_an_existing_eod() {
    let backend = Arc::new(FakeResearchBackend::default());
    backend.eod.lock().unwrap().push_back(Ok(true));
    let worker = configured_worker(backend.clone());
    let date = TradingDate::parse("2020-01-31").unwrap();

    let result = worker
        .run_once(date, &FakeControl::default())
        .await
        .unwrap();

    assert_eq!(result, WorkerRunOutcome::AlreadyPublished);
    assert_eq!(
        *backend.events.lock().unwrap(),
        vec!["recover", "has_eod:2020-01-31"]
    );
}

#[tokio::test]
async fn worker_daemon_uses_injected_clock_and_default_or_override_schedule() {
    for (override_time, now) in [
        (None, Utc.with_ymd_and_hms(2026, 8, 10, 7, 29, 30).unwrap()),
        (
            Some("07:05"),
            Utc.with_ymd_and_hms(2026, 8, 9, 22, 4, 30).unwrap(),
        ),
    ] {
        let backend = Arc::new(FakeResearchBackend::default());
        let mut values = worker_config(&[]);
        if let Some(run_at) = override_time {
            values.insert("RESEARCH_RUN_AT_KST".to_owned(), run_at.to_owned());
        }
        let config =
            ResearchWorkerConfig::from_map_with_reader(&values, |_| Ok("password".to_owned()))
                .unwrap();
        let worker = ResearchWorker::new(config, backend.clone());
        let control = ScheduleControl {
            now,
            sleeps: Mutex::new(Vec::new()),
        };

        assert_eq!(
            worker.run_daemon(&control).await.unwrap(),
            WorkerRunOutcome::Shutdown
        );
        assert_eq!(*backend.events.lock().unwrap(), vec!["recover"]);
        assert_eq!(
            *control.sleeps.lock().unwrap(),
            vec![Duration::from_secs(30)]
        );
    }
}

#[tokio::test]
async fn worker_daemon_catches_up_immediately_at_or_after_schedule() {
    for now in [
        Utc.with_ymd_and_hms(2026, 8, 10, 7, 30, 0).unwrap(),
        Utc.with_ymd_and_hms(2026, 8, 10, 9, 0, 0).unwrap(),
    ] {
        let backend = Arc::new(FakeResearchBackend::default());
        backend.eod.lock().unwrap().push_back(Ok(true));
        let control = ScheduleControl {
            now,
            sleeps: Mutex::new(Vec::new()),
        };

        assert_eq!(
            configured_worker(backend.clone())
                .run_daemon(&control)
                .await
                .unwrap(),
            WorkerRunOutcome::Shutdown
        );
        assert_eq!(
            *backend.events.lock().unwrap(),
            vec!["recover", "recover", "has_eod:2026-08-10"],
            "startup after the configured time performs one immediate catch-up"
        );
        assert_eq!(control.sleeps.lock().unwrap().len(), 1);
        assert!(control.sleeps.lock().unwrap()[0] >= Duration::from_secs(22 * 60 * 60));
    }
}

#[tokio::test]
async fn worker_daemon_recovers_immediately_before_each_scheduled_cycle() {
    let backend = Arc::new(FakeResearchBackend::default());
    backend.eod.lock().unwrap().push_back(Ok(true));
    let control = OneScheduledCycleControl {
        now: Utc.with_ymd_and_hms(2026, 8, 10, 7, 29, 30).unwrap(),
        scheduled_waits: AtomicUsize::new(0),
    };

    assert_eq!(
        configured_worker(backend.clone())
            .run_daemon(&control)
            .await
            .unwrap(),
        WorkerRunOutcome::Shutdown
    );
    assert_eq!(
        *backend.events.lock().unwrap(),
        vec!["recover", "recover", "has_eod:2026-08-10"],
        "a post-completion-check append is recovered before the next fetch cycle"
    );
}

#[tokio::test]
async fn worker_daemon_error_retains_the_current_kst_cycle_date() {
    let backend = Arc::new(FakeResearchBackend::default());
    backend
        .eod
        .lock()
        .unwrap()
        .push_back(Err(WorkerError::Database {
            phase: WorkerPhase::DuplicateCheck,
            source: SinkError::PermanentDatabase(sqlx::Error::RowNotFound),
        }));
    let observer = Arc::new(RecordingObserver::default());
    let error = configured_worker(backend)
        .with_observer(observer.clone())
        .run_daemon(&OneCycleControl {
            now: Utc.with_ymd_and_hms(2026, 8, 10, 7, 30, 0).unwrap(),
        })
        .await
        .unwrap_err();

    assert_eq!(
        error.target_date(),
        Some(TradingDate::parse("2026-08-10").unwrap())
    );
    assert_eq!(error.phase(), WorkerPhase::DuplicateCheck);
    let failed = observer.0.lock().unwrap().last().cloned().unwrap();
    assert_eq!(failed.kind, WorkerEventKind::Failed);
    assert_eq!(failed.target_date, error.target_date());
}

#[tokio::test]
async fn worker_retries_startup_recovery_before_target_work() {
    let backend = Arc::new(FakeResearchBackend::default());
    backend.recover.lock().unwrap().extend([
        Err(WorkerError::Io {
            phase: WorkerPhase::Recovery,
        }),
        Ok(()),
    ]);
    backend.eod.lock().unwrap().push_back(Ok(true));
    let control = FakeControl::default();

    assert_eq!(
        configured_worker(backend.clone())
            .run_once(TradingDate::parse("2020-01-31").unwrap(), &control)
            .await
            .unwrap(),
        WorkerRunOutcome::AlreadyPublished
    );
    assert_eq!(
        *backend.events.lock().unwrap(),
        vec!["recover", "recover", "has_eod:2020-01-31"]
    );
    assert_eq!(
        *control.sleeps.lock().unwrap(),
        vec![Duration::from_secs(10)]
    );
}

#[tokio::test]
async fn worker_eod_unavailable_does_not_suppress_fetch_and_retry_resets_per_run() {
    let backend = Arc::new(FakeResearchBackend::default());
    backend.eod.lock().unwrap().extend([
        Err(WorkerError::Io {
            phase: WorkerPhase::DuplicateCheck,
        }),
        Err(WorkerError::Io {
            phase: WorkerPhase::DuplicateCheck,
        }),
        Ok(false),
        Err(WorkerError::Io {
            phase: WorkerPhase::DuplicateCheck,
        }),
        Ok(false),
    ]);
    let first_batch = BatchId::generate();
    let second_batch = BatchId::generate();
    backend
        .ingest
        .lock()
        .unwrap()
        .extend([Ok(first_batch), Ok(second_batch)]);
    let worker = configured_worker(backend);

    let first_control = FakeControl::default();
    assert_eq!(
        worker
            .run_once(TradingDate::parse("2020-01-31").unwrap(), &first_control)
            .await
            .unwrap(),
        WorkerRunOutcome::Published(first_batch)
    );
    assert_eq!(
        *first_control.sleeps.lock().unwrap(),
        vec![Duration::from_secs(10), Duration::from_secs(20)]
    );

    let second_control = FakeControl::default();
    assert_eq!(
        worker
            .run_once(TradingDate::parse("2020-02-01").unwrap(), &second_control)
            .await
            .unwrap(),
        WorkerRunOutcome::Published(second_batch)
    );
    assert_eq!(
        *second_control.sleeps.lock().unwrap(),
        vec![Duration::from_secs(10)],
        "retry counters reset after a successful cycle"
    );
}

#[tokio::test]
async fn worker_retryable_ingest_requires_recovery_before_any_fresh_fetch() {
    let backend = Arc::new(FakeResearchBackend::default());
    backend.recover.lock().unwrap().extend([
        Ok(()),
        Err(WorkerError::Io {
            phase: WorkerPhase::Recovery,
        }),
        Err(WorkerError::Io {
            phase: WorkerPhase::Recovery,
        }),
        Ok(()),
    ]);
    backend.eod.lock().unwrap().extend([Ok(false), Ok(true)]);
    backend
        .ingest
        .lock()
        .unwrap()
        .push_back(Err(WorkerError::Database {
            phase: WorkerPhase::Publication,
            source: SinkError::RetryableDatabase(sqlx::Error::PoolClosed),
        }));
    let control = FakeControl::default();

    assert_eq!(
        configured_worker(backend.clone())
            .run_once(TradingDate::parse("2020-01-31").unwrap(), &control)
            .await
            .unwrap(),
        WorkerRunOutcome::AlreadyPublished
    );
    assert_eq!(
        *backend.events.lock().unwrap(),
        vec![
            "recover",
            "has_eod:2020-01-31",
            "ingest:2020-01-31",
            "recover",
            "recover",
            "recover",
            "has_eod:2020-01-31",
        ],
        "no second provider fetch may begin until recovery succeeds"
    );
    assert_eq!(
        *control.sleeps.lock().unwrap(),
        vec![
            Duration::from_secs(10),
            Duration::from_secs(20),
            Duration::from_secs(40),
        ],
        "exactly one backoff follows each retryable failure"
    );
}

#[derive(Debug)]
struct CountingProvider {
    inner: KrxProvider,
    calls: AtomicUsize,
}

impl EodProvider for CountingProvider {
    fn provider_id(&self) -> &'static str {
        self.inner.provider_id()
    }

    fn fetch_mode(&self) -> market_data::FetchMode {
        self.inner.fetch_mode()
    }

    fn fetch(
        &self,
        request: &FetchRequest,
    ) -> Result<Vec<market_data::RawEnvelope>, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.fetch(request)
    }
}

#[derive(Default)]
struct RetryPublicationSink;

#[async_trait]
impl PublicationSink for RetryPublicationSink {
    async fn publication_state(&self, _batch_id: BatchId) -> Result<PublicationState, SinkError> {
        unreachable!("fresh ingestion does not query publication state")
    }

    async fn publish(&self, _bundle: &PublicationBundle) -> Result<PublishOutcome, SinkError> {
        Err(SinkError::RetryableDatabase(sqlx::Error::PoolClosed))
    }

    async fn has_eod(&self, _date: TradingDate) -> Result<bool, SinkError> {
        Ok(false)
    }
}

struct RecoveryPublicationSink {
    failures_remaining: AtomicUsize,
    state_batches: Mutex<Vec<BatchId>>,
    publish_batches: Mutex<Vec<BatchId>>,
    published: AtomicBool,
}

#[async_trait]
impl PublicationSink for RecoveryPublicationSink {
    async fn publication_state(&self, batch_id: BatchId) -> Result<PublicationState, SinkError> {
        self.state_batches.lock().unwrap().push(batch_id);
        if self
            .failures_remaining
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok()
        {
            return Err(SinkError::RetryableDatabase(sqlx::Error::PoolClosed));
        }
        Ok(PublicationState::Missing)
    }

    async fn publish(&self, bundle: &PublicationBundle) -> Result<PublishOutcome, SinkError> {
        self.publish_batches
            .lock()
            .unwrap()
            .push(bundle.source_batch_id);
        self.published.store(true, Ordering::SeqCst);
        Ok(PublishOutcome::Published)
    }

    async fn has_eod(&self, _date: TradingDate) -> Result<bool, SinkError> {
        Ok(self.published.load(Ordering::SeqCst))
    }
}

struct DurableRetryBackend {
    store: RawStore,
    provider: CountingProvider,
    ingest_sink: RetryPublicationSink,
    recovery_sink: RecoveryPublicationSink,
}

#[async_trait]
impl ResearchBackend for DurableRetryBackend {
    async fn recover(
        &self,
        _control: &dyn WorkerControl,
        observer: &dyn RecoveryObserver,
    ) -> Result<(), WorkerError> {
        let result = recover_unpublished_with(&self.store, &self.recovery_sink, |outcome| {
            match outcome {
                RecoveryBatchOutcome::Recovered { batch_id, date } => {
                    observer.recovered(batch_id, date);
                }
                RecoveryBatchOutcome::Skipped { batch_id, date } => {
                    observer.skipped(batch_id, date);
                }
            }
            Ok::<_, std::convert::Infallible>(())
        })
        .await;
        match result {
            Ok(()) => Ok(()),
            Err(RecoveryError::Pipeline(error)) => Err(WorkerError::Pipeline(error)),
            Err(RecoveryError::Observer { source, .. }) => match source {},
        }
    }

    async fn has_eod(&self, date: TradingDate) -> Result<bool, WorkerError> {
        self.recovery_sink
            .has_eod(date)
            .await
            .map_err(|source| WorkerError::Database {
                phase: WorkerPhase::DuplicateCheck,
                source,
            })
    }

    async fn ingest(
        &self,
        date: TradingDate,
        now: UtcTimestamp,
        _control: &dyn WorkerControl,
    ) -> Result<BatchId, WorkerError> {
        let request = IngestRequest::new(MARKET_KR.to_owned(), date, now);
        ingest_and_publish(
            &self.store,
            &self.provider,
            &request,
            None,
            &self.ingest_sink,
        )
        .await
        .map(|outcome| outcome.manifest.batch_id)
        .map_err(WorkerError::Pipeline)
    }
}

#[tokio::test]
async fn worker_db_outage_recovers_exact_durable_batch_without_raw_growth() {
    let root = tempfile::tempdir().unwrap();
    let backend = Arc::new(DurableRetryBackend {
        store: RawStore::new(root.path()),
        provider: CountingProvider {
            inner: provider(),
            calls: AtomicUsize::new(0),
        },
        ingest_sink: RetryPublicationSink,
        recovery_sink: RecoveryPublicationSink {
            failures_remaining: AtomicUsize::new(2),
            state_batches: Mutex::new(Vec::new()),
            publish_batches: Mutex::new(Vec::new()),
            published: AtomicBool::new(false),
        },
    });
    let control = FakeControl::default();
    let date = TradingDate::parse("2020-01-31").unwrap();

    assert_eq!(
        configured_worker(backend.clone())
            .run_once(date, &control)
            .await
            .unwrap(),
        WorkerRunOutcome::AlreadyPublished
    );

    let manifest = backend
        .store
        .read_manifest(PROVIDER_KRX, MARKET_KR)
        .unwrap();
    assert_eq!(manifest.len(), 1, "retries must not create new Raw batches");
    let original_batch = manifest[0].batch_id;
    assert_eq!(backend.provider.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        *backend.recovery_sink.state_batches.lock().unwrap(),
        vec![original_batch, original_batch, original_batch]
    );
    assert_eq!(
        *backend.recovery_sink.publish_batches.lock().unwrap(),
        vec![original_batch]
    );
    assert_eq!(
        *control.sleeps.lock().unwrap(),
        vec![
            Duration::from_secs(10),
            Duration::from_secs(20),
            Duration::from_secs(40),
        ]
    );
}

#[derive(Default)]
struct RecordingObserver(Mutex<Vec<WorkerEvent>>);

impl WorkerObserver for RecordingObserver {
    fn emit(&self, event: WorkerEvent) {
        self.0.lock().unwrap().push(event);
    }
}

#[tokio::test]
async fn worker_streams_sanitized_retry_completed_and_skipped_events() {
    let date = TradingDate::parse("2020-01-31").unwrap();
    let published_batch = BatchId::generate();
    let published_backend = Arc::new(FakeResearchBackend::default());
    published_backend.eod.lock().unwrap().extend([
        Err(WorkerError::Io {
            phase: WorkerPhase::DuplicateCheck,
        }),
        Ok(false),
    ]);
    published_backend
        .ingest
        .lock()
        .unwrap()
        .push_back(Ok(published_batch));
    let published_observer = Arc::new(RecordingObserver::default());
    let worker = configured_worker(published_backend).with_observer(published_observer.clone());

    assert_eq!(
        worker
            .run_once(date, &FakeControl::default())
            .await
            .unwrap(),
        WorkerRunOutcome::Published(published_batch)
    );
    let events = published_observer.0.lock().unwrap().clone();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].kind, WorkerEventKind::Retrying);
    assert_eq!(events[0].provider, "KRX");
    assert_eq!(events[0].market, "KR");
    assert_eq!(events[0].target_date, Some(date));
    assert_eq!(events[0].phase, WorkerPhase::DuplicateCheck);
    assert_eq!(events[0].class, WorkerEventClass::Retryable);
    assert_eq!(events[0].batch_id, None);
    assert_eq!(events[1].kind, WorkerEventKind::Completed);
    assert_eq!(events[1].class, WorkerEventClass::Success);
    assert_eq!(events[1].batch_id, Some(published_batch));

    let skipped_backend = Arc::new(FakeResearchBackend::default());
    skipped_backend.eod.lock().unwrap().push_back(Ok(true));
    let skipped_observer = Arc::new(RecordingObserver::default());
    configured_worker(skipped_backend)
        .with_observer(skipped_observer.clone())
        .run_once(date, &FakeControl::default())
        .await
        .unwrap();
    let skipped = skipped_observer.0.lock().unwrap().clone();
    assert_eq!(skipped.last().unwrap().kind, WorkerEventKind::Skipped);
    assert_eq!(skipped.last().unwrap().class, WorkerEventClass::Success);
    assert_eq!(skipped.last().unwrap().target_date, Some(date));
}

#[tokio::test]
async fn worker_emits_recovery_batches_and_all_permanent_failures_with_context() {
    let date = TradingDate::parse("2020-01-31").unwrap();
    let recovered_date = TradingDate::parse("2020-01-30").unwrap();
    let replayed_date = TradingDate::parse("2020-01-29").unwrap();
    let recovered = BatchId::generate();
    let replayed = BatchId::generate();
    let recovery_backend = Arc::new(FakeResearchBackend::default());
    recovery_backend
        .recovered
        .lock()
        .unwrap()
        .push_back(vec![(recovered, recovered_date)]);
    recovery_backend
        .recovery_skipped
        .lock()
        .unwrap()
        .push_back(vec![(replayed, replayed_date)]);
    recovery_backend.eod.lock().unwrap().push_back(Ok(true));
    let recovery_observer = Arc::new(RecordingObserver::default());
    configured_worker(recovery_backend)
        .with_observer(recovery_observer.clone())
        .run_once(date, &FakeControl::default())
        .await
        .unwrap();
    let recovery_events = recovery_observer.0.lock().unwrap().clone();
    assert_eq!(
        recovery_events
            .iter()
            .map(|event| (event.kind, event.class, event.batch_id, event.target_date))
            .collect::<Vec<_>>(),
        vec![
            (
                WorkerEventKind::Recovered,
                WorkerEventClass::Success,
                Some(recovered),
                Some(recovered_date),
            ),
            (
                WorkerEventKind::Skipped,
                WorkerEventClass::Success,
                Some(replayed),
                Some(replayed_date),
            ),
            (
                WorkerEventKind::Skipped,
                WorkerEventClass::Success,
                None,
                Some(date),
            ),
        ]
    );

    let daemon_backend = Arc::new(FakeResearchBackend::default());
    daemon_backend
        .recovered
        .lock()
        .unwrap()
        .push_back(vec![(recovered, recovered_date)]);
    let daemon_observer = Arc::new(RecordingObserver::default());
    let daemon_control = ScheduleControl {
        now: Utc.with_ymd_and_hms(2020, 1, 31, 7, 0, 0).unwrap(),
        sleeps: Mutex::new(Vec::new()),
    };
    configured_worker(daemon_backend)
        .with_observer(daemon_observer.clone())
        .run_daemon(&daemon_control)
        .await
        .unwrap();
    let daemon_events = daemon_observer.0.lock().unwrap().clone();
    assert_eq!(daemon_events.len(), 1);
    assert_eq!(daemon_events[0].kind, WorkerEventKind::Recovered);
    assert_eq!(daemon_events[0].target_date, Some(recovered_date));

    for (phase, setup) in [
        (WorkerPhase::Recovery, "recovery"),
        (WorkerPhase::DuplicateCheck, "duplicate"),
        (WorkerPhase::Publication, "ingest"),
    ] {
        let backend = Arc::new(FakeResearchBackend::default());
        let batch_id = BatchId::generate();
        let error = match phase {
            WorkerPhase::Recovery => WorkerError::InvalidConfig { key: "recovery" },
            WorkerPhase::DuplicateCheck => WorkerError::Database {
                phase,
                source: SinkError::PermanentDatabase(sqlx::Error::RowNotFound),
            },
            WorkerPhase::Publication => WorkerError::Pipeline(PipelineError::Sink {
                batch_id,
                stage: PipelineStage::Publish,
                source: SinkError::PermanentDatabase(sqlx::Error::RowNotFound),
            }),
            _ => unreachable!(),
        };
        match setup {
            "recovery" => backend.recover.lock().unwrap().push_back(Err(error)),
            "duplicate" => backend.eod.lock().unwrap().push_back(Err(error)),
            "ingest" => {
                backend.eod.lock().unwrap().push_back(Ok(false));
                backend.ingest.lock().unwrap().push_back(Err(error));
            }
            _ => unreachable!(),
        }
        let observer = Arc::new(RecordingObserver::default());
        let returned = configured_worker(backend)
            .with_observer(observer.clone())
            .run_once(date, &FakeControl::default())
            .await
            .unwrap_err();
        let failed = observer.0.lock().unwrap().last().cloned().unwrap();
        assert_eq!(failed.kind, WorkerEventKind::Failed);
        assert_eq!(failed.class, WorkerEventClass::Permanent);
        assert_eq!(failed.provider, "KRX");
        assert_eq!(failed.market, "KR");
        assert_eq!(failed.target_date, Some(date));
        assert_eq!(failed.phase, returned.phase());
        assert_eq!(failed.batch_id, returned.batch_id());
    }
}

#[tokio::test]
async fn worker_permanent_errors_are_not_retried_and_shutdown_interrupts_backoff() {
    let permanent_backend = Arc::new(FakeResearchBackend::default());
    permanent_backend
        .eod
        .lock()
        .unwrap()
        .push_back(Err(WorkerError::InvalidConfig {
            key: "test-permanent",
        }));
    let permanent_control = FakeControl::default();
    let error = configured_worker(permanent_backend)
        .run_once(
            TradingDate::parse("2020-01-31").unwrap(),
            &permanent_control,
        )
        .await
        .unwrap_err();
    assert_eq!(error.failure_class(), FailureClass::Permanent);
    assert!(permanent_control.sleeps.lock().unwrap().is_empty());

    let retryable_backend = Arc::new(FakeResearchBackend::default());
    retryable_backend
        .eod
        .lock()
        .unwrap()
        .push_back(Err(WorkerError::Io {
            phase: WorkerPhase::DuplicateCheck,
        }));
    let shutdown_control = FakeControl {
        shutdown_on_sleep: true,
        ..FakeControl::default()
    };
    let result = configured_worker(retryable_backend)
        .run_once(TradingDate::parse("2020-01-31").unwrap(), &shutdown_control)
        .await
        .unwrap();
    assert_eq!(result, WorkerRunOutcome::Shutdown);
    assert_eq!(
        *shutdown_control.sleeps.lock().unwrap(),
        vec![Duration::from_secs(10)]
    );
}

#[tokio::test]
async fn worker_health_requires_db_round_trip_and_fresh_real_eod() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let now = Utc.with_ymd_and_hms(2026, 8, 10, 0, 0, 0).unwrap();
    let max_age = Duration::from_secs(345_600);

    let missing = healthcheck(&db.writer, now, max_age).await.unwrap_err();
    assert_eq!(missing.phase(), WorkerPhase::Health);

    sqlx::query(
        "INSERT INTO data_batches (provider, market, batch_date, kind, storage_path, \
         content_sha256, bytes_size, retrieved_at) VALUES \
         ('KRX','KR','2026-08-10','EOD_UNAVAILABLE','raw/unavailable',repeat('a',64),1,$1)",
    )
    .bind(now)
    .execute(&db.supervisor)
    .await
    .unwrap();
    assert!(healthcheck(&db.writer, now, max_age).await.is_err());

    sqlx::query(
        "INSERT INTO data_batches (provider, market, batch_date, kind, storage_path, \
         content_sha256, bytes_size, retrieved_at) VALUES \
         ('KRX','KR','2026-08-01','EOD','raw/stale',repeat('b',64),1,$1)",
    )
    .bind(now - chrono::Duration::days(5))
    .execute(&db.supervisor)
    .await
    .unwrap();
    assert!(healthcheck(&db.writer, now, max_age).await.is_err());

    sqlx::query(
        "INSERT INTO data_batches (provider, market, batch_date, kind, storage_path, \
         content_sha256, bytes_size, retrieved_at) VALUES \
         ('KRX','KR','2026-08-09','EOD','raw/fresh',repeat('c',64),1,$1)",
    )
    .bind(now - chrono::Duration::hours(12))
    .execute(&db.supervisor)
    .await
    .unwrap();
    let healthy = healthcheck(&db.writer, now, max_age).await.unwrap();
    assert_eq!(healthy.age.as_secs(), 12 * 60 * 60);

    db.writer.close().await;
    assert!(healthcheck(&db.writer, now, max_age).await.is_err());
    db.drop_db().await;
}

#[tokio::test]
async fn worker_health_uses_batch_date_and_ignores_future_batches() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let now = Utc.with_ymd_and_hms(2026, 8, 10, 0, 0, 0).unwrap();
    let max_age = Duration::from_secs(345_600);

    sqlx::query(
        "INSERT INTO data_batches (provider, market, batch_date, kind, storage_path, \
         content_sha256, bytes_size, retrieved_at) VALUES \
         ('KRX','KR','2020-01-31','EOD','raw/historical-backfill',repeat('a',64),1,$1)",
    )
    .bind(now)
    .execute(&db.supervisor)
    .await
    .unwrap();
    let historical = healthcheck(&db.writer, now, max_age).await.unwrap_err();
    assert!(matches!(
        historical,
        WorkerError::Unhealthy {
            reason: collectors::HealthFailure::StaleEodPublication
        }
    ));
    let historical_with_large_limit =
        healthcheck(&db.writer, now, Duration::from_secs(500_000_000))
            .await
            .unwrap();
    assert_eq!(
        historical_with_large_limit.newest_eod_at,
        Utc.with_ymd_and_hms(2020, 1, 31, 15, 0, 0).unwrap(),
        "health output must report the effective KST batch-date boundary, not backfill retrieval"
    );

    sqlx::query(
        "INSERT INTO data_batches (provider, market, batch_date, kind, storage_path, \
         content_sha256, bytes_size, retrieved_at) VALUES \
         ('KRX','KR','2026-08-09','EOD','raw/applicable',repeat('b',64),1,$1), \
         ('KRX','KR','2026-08-11','EOD','raw/future-date',repeat('c',64),1,$2)",
    )
    .bind(now - chrono::Duration::hours(12))
    .bind(now)
    .execute(&db.supervisor)
    .await
    .unwrap();
    let healthy = healthcheck(&db.writer, now, max_age).await.unwrap();
    assert_eq!(healthy.age, Duration::from_secs(12 * 60 * 60));

    sqlx::query(
        "INSERT INTO data_batches (provider, market, batch_date, kind, storage_path, \
         content_sha256, bytes_size, retrieved_at) VALUES \
         ('KRX','KR','2026-08-10','EOD','raw/current',repeat('d',64),1,$1)",
    )
    .bind(now - chrono::Duration::seconds(60))
    .execute(&db.supervisor)
    .await
    .unwrap();
    let current = healthcheck(&db.writer, now, max_age).await.unwrap();
    assert_eq!(current.age, Duration::from_secs(60));
    assert_eq!(current.newest_eod_at, now - chrono::Duration::seconds(60));

    db.drop_db().await;
}

#[tokio::test]
async fn worker_production_pool_uses_discrete_fields_role_limits_and_timeouts() {
    let qa_url = std::env::var("DATABASE_URL")
        .expect("required QA DATABASE_URL must be provided for production-pool verification");
    assert_eq!(
        qa_url, "postgres://postgres:lagrange@127.0.0.1:55432/postgres",
        "final QA must use the reviewed local PostgreSQL endpoint"
    );
    let db = ScratchDb::create()
        .await
        .expect("required QA PostgreSQL must be reachable");
    let values = HashMap::from([
        ("APP_ENV".to_owned(), "qa".to_owned()),
        ("RESEARCH_FETCH_MODE".to_owned(), "synthetic".to_owned()),
        ("DB_HOST".to_owned(), "127.0.0.1".to_owned()),
        ("DB_PORT".to_owned(), "55432".to_owned()),
        ("DB_NAME".to_owned(), db.database_name().to_owned()),
        ("DB_USER".to_owned(), "research_writer".to_owned()),
        ("DB_PASSWORD_FILE".to_owned(), "qa-password".to_owned()),
        (
            "RESEARCH_CURATED_ROOT".to_owned(),
            "var/research".to_owned(),
        ),
    ]);
    let config =
        HealthcheckConfig::from_map_with_reader(&values, |_| Ok("lagrange".to_owned())).unwrap();
    assert_eq!(config.expected_fetch_mode, FetchMode::Synthetic);
    let pool = build_postgres_pool(&config.database);

    let (one, current_user): (i32, String) = sqlx::query_as("SELECT 1, current_user::text")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(one, 1);
    assert_eq!(current_user, "research_writer");
    let statement_timeout: String = sqlx::query_scalar("SHOW statement_timeout")
        .fetch_one(&pool)
        .await
        .unwrap();
    let lock_timeout: String = sqlx::query_scalar("SHOW lock_timeout")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(statement_timeout, "15s");
    assert_eq!(lock_timeout, "5s");
    assert_eq!(pool.options().get_max_connections(), 4);
    assert_eq!(
        pool.options().get_acquire_timeout(),
        Duration::from_secs(10)
    );

    let mut timed_connection = pool.acquire().await.unwrap();
    sqlx::query("SET statement_timeout = '50ms'")
        .execute(&mut *timed_connection)
        .await
        .unwrap();
    let cancellation = sqlx::query("SELECT pg_sleep(0.2)")
        .execute(&mut *timed_connection)
        .await
        .unwrap_err();
    let cancellation = SinkError::from_sqlx(cancellation);
    assert!(matches!(cancellation, SinkError::RetryableDatabase(_)));
    assert_eq!(cancellation.to_string(), "retryable database failure");
    drop(timed_connection);

    pool.close().await;
    db.drop_db().await;
}

#[test]
fn worker_health_rejects_future_publications_and_accepts_exact_boundaries() {
    let now = Utc.with_ymd_and_hms(2026, 8, 10, 0, 0, 0).unwrap();
    let max_age = Duration::from_secs(345_600);

    assert_eq!(publication_age(now, now, max_age).unwrap(), Duration::ZERO);
    assert_eq!(
        publication_age(
            now,
            now - chrono::Duration::seconds(max_age.as_secs() as i64),
            max_age,
        )
        .unwrap(),
        max_age
    );
    for future in [
        now + chrono::Duration::seconds(1),
        now + chrono::Duration::days(3650),
    ] {
        assert!(matches!(
            publication_age(now, future, max_age),
            Err(collectors::HealthFailure::FutureEodPublication)
        ));
    }
}
