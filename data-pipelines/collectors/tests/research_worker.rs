use std::collections::HashMap;
use std::process::Command;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use collectors::{
    FailureClass, PipelineError, PipelineStage, PublicationSink, PublicationState, PublishOutcome,
    SinkError, ingest_and_publish, recover_unpublished,
};
use domain::{BatchId, TradingDate, UtcTimestamp};
use market_data::contract::{MARKET_KR, PROVIDER_KRX, ResponseKind};
use market_data::ingest::{IngestError, IngestRequest, ingest_bundle};
use market_data::provider::{
    CredentialRef, EodProvider, FetchRequest, KrxProvider, ProviderError, RecordedBundle,
};
use market_data::publication::{PublicationBundle, PublicationError};
use market_data::storage::{RawStore, StoreError};

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
