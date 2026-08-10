use domain::BatchId;
use market_data::contract::{MARKET_KR, PROVIDER_KRX};
use market_data::ingest::{IngestError, IngestRequest, ingest_bundle};
use market_data::provider::{EodProvider, ProviderError};
use market_data::publication::{PublicationBundle, PublicationError};
use market_data::storage::{ManifestEntry, RawStore, StoreError};

use crate::sink::{PublicationSink, PublicationState, PublishOutcome, SinkError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureClass {
    Retryable,
    Permanent,
}

impl FailureClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Retryable => "retryable",
            Self::Permanent => "permanent",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineStage {
    Ingest,
    ReadManifest,
    PublicationState,
    VerifyRaw,
    Publish,
}

#[derive(Debug)]
pub enum PipelineError {
    Ingest {
        source: IngestError,
    },
    Manifest {
        source: StoreError,
    },
    Publication {
        batch_id: BatchId,
        source: PublicationError,
    },
    Sink {
        batch_id: BatchId,
        stage: PipelineStage,
        source: SinkError,
    },
    PartialPublication {
        batch_id: BatchId,
    },
    UnexpectedPublishOutcome {
        batch_id: BatchId,
        state: PublicationState,
        outcome: PublishOutcome,
    },
}

impl PipelineError {
    pub fn batch_id(&self) -> Option<BatchId> {
        match self {
            Self::Ingest { source } => source.batch_id(),
            Self::Manifest { .. } => None,
            Self::Publication { batch_id, .. }
            | Self::Sink { batch_id, .. }
            | Self::PartialPublication { batch_id }
            | Self::UnexpectedPublishOutcome { batch_id, .. } => Some(*batch_id),
        }
    }

    pub const fn stage(&self) -> PipelineStage {
        match self {
            Self::Ingest { .. } => PipelineStage::Ingest,
            Self::Manifest { .. } => PipelineStage::ReadManifest,
            Self::Publication { .. } => PipelineStage::VerifyRaw,
            Self::Sink { stage, .. } => *stage,
            Self::PartialPublication { .. } => PipelineStage::PublicationState,
            Self::UnexpectedPublishOutcome { .. } => PipelineStage::Publish,
        }
    }

    pub fn failure_class(&self) -> FailureClass {
        let retryable = match self {
            Self::Ingest { source } => ingest_error_is_retryable(source),
            Self::Manifest { source } => store_failure_class(source) == FailureClass::Retryable,
            Self::Publication { source, .. } => publication_error_is_retryable(source),
            Self::Sink { source, .. } => source.is_retryable(),
            Self::PartialPublication { .. } => false,
            Self::UnexpectedPublishOutcome { .. } => false,
        };
        if retryable {
            FailureClass::Retryable
        } else {
            FailureClass::Permanent
        }
    }

    pub fn is_retryable(&self) -> bool {
        self.failure_class() == FailureClass::Retryable
    }
}

impl std::fmt::Display for PipelineError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ingest { source } => write!(formatter, "{source}"),
            Self::Manifest { source } => write!(formatter, "read Raw manifest failed: {source}"),
            Self::Publication { batch_id, source } => {
                write!(formatter, "verify Raw batch {batch_id} failed: {source}")
            }
            Self::Sink {
                batch_id,
                stage,
                source,
            } => write!(
                formatter,
                "publication stage {stage:?} failed for batch {batch_id}: {source}"
            ),
            Self::PartialPublication { batch_id } => {
                write!(
                    formatter,
                    "publication state is partial for batch {batch_id}"
                )
            }
            Self::UnexpectedPublishOutcome {
                batch_id,
                state,
                outcome,
            } => write!(
                formatter,
                "publication state {state:?} returned unexpected outcome {outcome:?} for batch {batch_id}"
            ),
        }
    }
}

impl std::error::Error for PipelineError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Ingest { source } => Some(source),
            Self::Manifest { source } => Some(source),
            Self::Publication { source, .. } => Some(source),
            Self::Sink { source, .. } => Some(source),
            Self::PartialPublication { .. } => None,
            Self::UnexpectedPublishOutcome { .. } => None,
        }
    }
}

pub fn provider_failure_class(error: &ProviderError) -> FailureClass {
    match error {
        ProviderError::EndpointTimeout { .. } | ProviderError::Io { .. } => FailureClass::Retryable,
        ProviderError::CredentialsUnavailable { .. }
        | ProviderError::UnsafeFileName { .. }
        | ProviderError::UnsupportedKind(_)
        | ProviderError::RecordedBundleMissing { .. }
        | ProviderError::RecordedBundleIo { .. }
        | ProviderError::RecordedBundleParse { .. }
        | ProviderError::RecordedBundleInvalid { .. } => FailureClass::Permanent,
    }
}

pub fn store_failure_class(error: &StoreError) -> FailureClass {
    match error {
        StoreError::Io { .. } => FailureClass::Retryable,
        // Cleanup I/O is secondary: callers act on the original operation's class.
        StoreError::CleanupFailed { original, .. } => store_failure_class(original),
        StoreError::IndeterminateBatchCommit { source, .. } => store_failure_class(source),
        StoreError::FileExists { .. }
        | StoreError::UnsafeFileName { .. }
        | StoreError::UnsafeScope { .. }
        | StoreError::ScopeMismatch { .. }
        | StoreError::UnsafePath { .. }
        | StoreError::ContentHashMismatch { .. }
        | StoreError::CorruptManifest { .. }
        | StoreError::CorruptBatchMetadata { .. }
        | StoreError::InvalidBatchMetadata { .. }
        | StoreError::MissingEvidence { .. }
        | StoreError::Serialization { .. }
        | StoreError::ManifestConflict { .. } => FailureClass::Permanent,
    }
}

fn ingest_error_is_retryable(error: &IngestError) -> bool {
    match error {
        IngestError::Provider(source) => provider_failure_class(source) == FailureClass::Retryable,
        IngestError::Store(source) => store_failure_class(source) == FailureClass::Retryable,
        IngestError::Readback { source, .. } => {
            store_failure_class(source) == FailureClass::Retryable
        }
        IngestError::MalformedResponse { .. } => false,
    }
}

fn publication_error_is_retryable(error: &PublicationError) -> bool {
    match error {
        PublicationError::Store(source) => store_failure_class(source) == FailureClass::Retryable,
        PublicationError::UnsupportedManifestScope { .. }
        | PublicationError::SizeMismatch { .. }
        | PublicationError::SizeExceedsPostgresBigint { .. }
        | PublicationError::UnexpectedContentHash { .. }
        | PublicationError::NonUtf8StoragePath { .. }
        | PublicationError::ReadbackFileCountMismatch { .. }
        | PublicationError::ReadbackFileNameMismatch { .. }
        | PublicationError::MalformedBars { .. }
        | PublicationError::InvalidBarDate { .. }
        | PublicationError::MalformedCalendar { .. }
        | PublicationError::UnsupportedCalendarTimezone { .. }
        | PublicationError::InvalidCalendarSessionTimes { .. }
        | PublicationError::InvalidCalendarDate { .. }
        | PublicationError::InvalidCalendarTimestamp { .. }
        | PublicationError::InconsistentCalendarInstant { .. }
        | PublicationError::CalendarDateBothSessionAndHoliday { .. }
        | PublicationError::ConflictingCalendarFact { .. }
        | PublicationError::ConflictingCalendarProvenance { .. } => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutcome {
    pub manifest: ManifestEntry,
    pub published: PublishOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RecoveryReport {
    pub recovered: Vec<BatchId>,
    pub skipped: Vec<BatchId>,
}

pub async fn ingest_and_publish(
    store: &RawStore,
    provider: &dyn EodProvider,
    request: &IngestRequest,
    entitlement_reference: Option<&str>,
    sink: &dyn PublicationSink,
) -> Result<RunOutcome, PipelineError> {
    let ingested = ingest_bundle(store, provider, request, entitlement_reference)
        .map_err(|source| PipelineError::Ingest { source })?;
    let manifest = ingested.entry;
    let batch_id = manifest.batch_id;
    let bundle = PublicationBundle::from_raw(store, &manifest)
        .map_err(|source| PipelineError::Publication { batch_id, source })?;
    let published = sink
        .publish(&bundle)
        .await
        .map_err(|source| PipelineError::Sink {
            batch_id,
            stage: PipelineStage::Publish,
            source,
        })?;
    Ok(RunOutcome {
        manifest,
        published,
    })
}

pub async fn recover_unpublished(
    store: &RawStore,
    sink: &dyn PublicationSink,
) -> Result<RecoveryReport, PipelineError> {
    let mut entries = store
        .read_manifest(PROVIDER_KRX, MARKET_KR)
        .map_err(|source| PipelineError::Manifest { source })?;
    entries.sort_by(|left, right| {
        left.retrieved_at
            .cmp(&right.retrieved_at)
            .then_with(|| left.batch_id.cmp(&right.batch_id))
    });

    let mut report = RecoveryReport::default();
    for manifest in entries {
        let batch_id = manifest.batch_id;
        let state =
            sink.publication_state(batch_id)
                .await
                .map_err(|source| PipelineError::Sink {
                    batch_id,
                    stage: PipelineStage::PublicationState,
                    source,
                })?;
        match state {
            PublicationState::Missing => {
                let bundle = PublicationBundle::from_raw(store, &manifest)
                    .map_err(|source| PipelineError::Publication { batch_id, source })?;
                sink.publish(&bundle)
                    .await
                    .map_err(|source| PipelineError::Sink {
                        batch_id,
                        stage: PipelineStage::Publish,
                        source,
                    })?;
                report.recovered.push(batch_id);
            }
            PublicationState::Complete => {
                let bundle = PublicationBundle::from_raw(store, &manifest)
                    .map_err(|source| PipelineError::Publication { batch_id, source })?;
                let outcome =
                    sink.publish(&bundle)
                        .await
                        .map_err(|source| PipelineError::Sink {
                            batch_id,
                            stage: PipelineStage::Publish,
                            source,
                        })?;
                if outcome != PublishOutcome::AlreadyPublished {
                    return Err(PipelineError::UnexpectedPublishOutcome {
                        batch_id,
                        state,
                        outcome,
                    });
                }
                report.skipped.push(batch_id);
            }
            PublicationState::Partial => {
                return Err(PipelineError::PartialPublication { batch_id });
            }
        }
    }
    Ok(report)
}
