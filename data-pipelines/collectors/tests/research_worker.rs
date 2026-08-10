use std::collections::{HashMap, VecDeque};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use collectors::{
    AppEnvironment, FailureClass, PipelineError, PipelineStage, PublicationSink, PublicationState,
    PublishOutcome, ResearchBackend, ResearchWorker, ResearchWorkerConfig, SinkError, WaitOutcome,
    WorkerComponentFactory, WorkerControl, WorkerError, WorkerPhase, WorkerRunOutcome,
    bootstrap_worker_with, healthcheck, ingest_and_publish, next_run_delay, recover_unpublished,
    retry_delay, store_failure_class, validate_synthetic_policy,
};
use domain::{BatchId, TradingDate, UtcTimestamp};
use market_data::contract::{MARKET_KR, PROVIDER_KRX, ResponseKind};
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
fn worker_config_parses_defaults_and_trims_file_secret() {
    let values = worker_config(&[]);
    let config = ResearchWorkerConfig::from_map_with_reader(&values, |path| {
        assert_eq!(path.to_string_lossy(), "db-password");
        Ok("  password from file\r\n".to_owned())
    })
    .unwrap();

    assert_eq!(config.app_env, AppEnvironment::Qa);
    assert_eq!(config.fetch_mode, market_data::FetchMode::Synthetic);
    assert_eq!(config.run_at_kst.format("%H:%M").to_string(), "16:30");
    assert_eq!(config.max_publication_age.as_secs(), 345_600);
    assert_eq!(config.raw_root.to_string_lossy(), "var/research");
    assert_eq!(config.database.host, "127.0.0.1");
    assert_eq!(config.database.port, 55432);
    assert_eq!(config.database.name, "lagrange");
    assert_eq!(config.database.user, "research_writer");
    assert_eq!(config.database.password.expose(), "password from file");
    assert!(!format!("{config:?}").contains("password from file"));
}

#[test]
fn worker_config_parses_schedule_and_max_age_overrides() {
    let values = worker_config(&[
        ("APP_ENV", "development"),
        ("RESEARCH_FETCH_MODE", "credentialed"),
        ("RESEARCH_RUN_AT_KST", "07:05"),
        ("RESEARCH_MAX_PUBLICATION_AGE_SECS", "42"),
        ("KRX_CREDENTIAL_FILE", "provider-secret"),
    ]);
    let config = ResearchWorkerConfig::from_map_with_reader(&values, |path| {
        Ok(match path.to_string_lossy().as_ref() {
            "db-password" => "db-secret\n",
            "provider-secret" => "provider-secret-value\n",
            other => panic!("unexpected secret path {other}"),
        }
        .to_owned())
    })
    .unwrap();

    assert_eq!(config.app_env, AppEnvironment::Development);
    assert_eq!(config.fetch_mode, market_data::FetchMode::Credentialed);
    assert_eq!(config.run_at_kst.format("%H:%M").to_string(), "07:05");
    assert_eq!(config.max_publication_age.as_secs(), 42);
}

#[test]
fn worker_config_rejects_invalid_and_missing_values_without_secret_contents() {
    for (key, value) in [
        ("APP_ENV", "staging"),
        ("RESEARCH_FETCH_MODE", "auto"),
        ("RESEARCH_RUN_AT_KST", "25:00"),
        ("RESEARCH_MAX_PUBLICATION_AGE_SECS", "0"),
        ("DB_PORT", "70000"),
        ("RESEARCH_RAW_ROOT", "  "),
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
    assert!(!String::from_utf8_lossy(&output.stdout).contains("DATABASE_URL"));
}

#[derive(Default)]
struct FakeResearchBackend {
    events: Mutex<Vec<String>>,
    recover: Mutex<VecDeque<Result<(), WorkerError>>>,
    eod: Mutex<VecDeque<Result<bool, WorkerError>>>,
    ingest: Mutex<VecDeque<Result<BatchId, WorkerError>>>,
}

#[async_trait]
impl ResearchBackend for FakeResearchBackend {
    async fn recover(&self) -> Result<(), WorkerError> {
        self.events.lock().unwrap().push("recover".to_owned());
        self.recover.lock().unwrap().pop_front().unwrap_or(Ok(()))
    }

    async fn has_eod(&self, date: TradingDate) -> Result<bool, WorkerError> {
        self.events
            .lock()
            .unwrap()
            .push(format!("has_eod:{}", date.to_iso()));
        self.eod.lock().unwrap().pop_front().unwrap_or(Ok(false))
    }

    async fn ingest(&self, date: TradingDate, _now: UtcTimestamp) -> Result<BatchId, WorkerError> {
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
