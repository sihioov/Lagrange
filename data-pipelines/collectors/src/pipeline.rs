use domain::{BatchId, TradingDate};
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
    InvalidRecoveryCursor {
        cursor: BatchId,
    },
    InvalidRecoverySnapshotBoundary {
        batch_id: BatchId,
    },
    InvalidRecoverySnapshotPosition,
    InvalidRecoveryPageSize,
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
            Self::Manifest { .. }
            | Self::InvalidRecoveryCursor { .. }
            | Self::InvalidRecoverySnapshotBoundary { .. }
            | Self::InvalidRecoverySnapshotPosition
            | Self::InvalidRecoveryPageSize => None,
            Self::Publication { batch_id, .. }
            | Self::Sink { batch_id, .. }
            | Self::PartialPublication { batch_id }
            | Self::UnexpectedPublishOutcome { batch_id, .. } => Some(*batch_id),
        }
    }

    pub const fn stage(&self) -> PipelineStage {
        match self {
            Self::Ingest { .. } => PipelineStage::Ingest,
            Self::Manifest { .. }
            | Self::InvalidRecoveryCursor { .. }
            | Self::InvalidRecoverySnapshotBoundary { .. }
            | Self::InvalidRecoverySnapshotPosition
            | Self::InvalidRecoveryPageSize => PipelineStage::ReadManifest,
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
            Self::InvalidRecoveryCursor { .. }
            | Self::InvalidRecoverySnapshotBoundary { .. }
            | Self::InvalidRecoverySnapshotPosition
            | Self::InvalidRecoveryPageSize => false,
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
            Self::InvalidRecoveryCursor { cursor } => {
                write!(
                    formatter,
                    "recovery cursor {cursor} is not in the canonical Raw manifest"
                )
            }
            Self::InvalidRecoverySnapshotBoundary { batch_id } => write!(
                formatter,
                "recovery snapshot boundary {batch_id} is not in the canonical Raw manifest"
            ),
            Self::InvalidRecoverySnapshotPosition => {
                formatter.write_str("recovery cursor requires an immutable snapshot high-water")
            }
            Self::InvalidRecoveryPageSize => {
                formatter.write_str("recovery page size must be positive")
            }
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
            Self::InvalidRecoveryCursor { .. }
            | Self::InvalidRecoverySnapshotBoundary { .. }
            | Self::InvalidRecoverySnapshotPosition
            | Self::InvalidRecoveryPageSize => None,
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

/// Stable position within an append-order Raw manifest snapshot. `snapshot_after`
/// excludes the prefix consumed by earlier snapshots; `snapshot_high_water`
/// freezes this snapshot; and `cursor` resumes its canonical sorted page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RecoveryPosition {
    pub snapshot_after: Option<BatchId>,
    pub snapshot_high_water: Option<BatchId>,
    pub cursor: Option<BatchId>,
}

/// A bounded authoritative replay page. The high-water fixes the append-order
/// snapshot and the cursor is the last exact batch whose outcome was emitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryPage {
    pub snapshot_high_water: Option<BatchId>,
    pub cursor: Option<BatchId>,
    pub has_more: bool,
}

/// Conservative bound for one contained recovery helper invocation.
pub const RECOVERY_PAGE_SIZE: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryBatchOutcome {
    Recovered {
        batch_id: BatchId,
        date: TradingDate,
    },
    Skipped {
        batch_id: BatchId,
        date: TradingDate,
    },
}

impl RecoveryBatchOutcome {
    pub const fn batch_id(self) -> BatchId {
        match self {
            Self::Recovered { batch_id, .. } | Self::Skipped { batch_id, .. } => batch_id,
        }
    }

    pub const fn date(self) -> TradingDate {
        match self {
            Self::Recovered { date, .. } | Self::Skipped { date, .. } => date,
        }
    }
}

#[derive(Debug)]
pub enum RecoveryError<E> {
    Pipeline(PipelineError),
    Observer { batch_id: BatchId, source: E },
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
    let mut report = RecoveryReport::default();
    let result = recover_unpublished_with(store, sink, |outcome| {
        match outcome {
            RecoveryBatchOutcome::Recovered { batch_id, .. } => report.recovered.push(batch_id),
            RecoveryBatchOutcome::Skipped { batch_id, .. } => report.skipped.push(batch_id),
        }
        Ok::<_, std::convert::Infallible>(())
    })
    .await;
    match result {
        Ok(()) => Ok(report),
        Err(RecoveryError::Pipeline(error)) => Err(error),
        Err(RecoveryError::Observer { source, .. }) => match source {},
    }
}

pub async fn recover_unpublished_with<E, F>(
    store: &RawStore,
    sink: &dyn PublicationSink,
    mut observer: F,
) -> Result<(), RecoveryError<E>>
where
    F: FnMut(RecoveryBatchOutcome) -> Result<(), E>,
{
    let mut position = RecoveryPosition::default();
    loop {
        let page = recover_unpublished_page_with(
            store,
            sink,
            position,
            RECOVERY_PAGE_SIZE,
            |outcome, _snapshot_high_water| observer(outcome),
        )
        .await?;
        if page.has_more {
            position.snapshot_high_water = page.snapshot_high_water;
            position.cursor = page.cursor;
            continue;
        }
        if page.snapshot_high_water == position.snapshot_after && page.cursor.is_none() {
            return Ok(());
        }
        position = RecoveryPosition {
            snapshot_after: page.snapshot_high_water,
            snapshot_high_water: None,
            cursor: None,
        };
    }
}

pub async fn recover_unpublished_page_with<E, F>(
    store: &RawStore,
    sink: &dyn PublicationSink,
    position: RecoveryPosition,
    page_size: usize,
    mut observer: F,
) -> Result<RecoveryPage, RecoveryError<E>>
where
    F: FnMut(RecoveryBatchOutcome, BatchId) -> Result<(), E>,
{
    if page_size == 0 {
        return Err(RecoveryError::Pipeline(
            PipelineError::InvalidRecoveryPageSize,
        ));
    }
    if position.cursor.is_some() && position.snapshot_high_water.is_none() {
        return Err(RecoveryError::Pipeline(
            PipelineError::InvalidRecoverySnapshotPosition,
        ));
    }
    let mut entries = store
        .read_reconciled_manifest(PROVIDER_KRX, MARKET_KR)
        .map_err(|source| RecoveryError::Pipeline(PipelineError::Manifest { source }))?;
    let boundary_index = position
        .snapshot_after
        .map(|boundary| {
            entries
                .iter()
                .position(|entry| entry.batch_id == boundary)
                .ok_or(RecoveryError::Pipeline(
                    PipelineError::InvalidRecoverySnapshotBoundary { batch_id: boundary },
                ))
        })
        .transpose()?;
    let start_of_snapshot = boundary_index.map_or(0, |index| index + 1);
    let high_water_index = match position.snapshot_high_water {
        Some(high_water) => Some(
            entries
                .iter()
                .position(|entry| entry.batch_id == high_water)
                .ok_or(RecoveryError::Pipeline(
                    PipelineError::InvalidRecoverySnapshotBoundary {
                        batch_id: high_water,
                    },
                ))?,
        ),
        None => entries.len().checked_sub(1),
    };
    if position.snapshot_high_water.is_some()
        && high_water_index.is_some_and(|index| index < start_of_snapshot)
    {
        return Err(RecoveryError::Pipeline(
            PipelineError::InvalidRecoverySnapshotPosition,
        ));
    }
    let snapshot_high_water = high_water_index
        .map(|index| entries[index].batch_id)
        .or(position.snapshot_after);
    let end_of_snapshot = high_water_index
        .map_or(start_of_snapshot, |index| index + 1)
        .max(start_of_snapshot);
    entries[start_of_snapshot..end_of_snapshot].sort_by(|left, right| {
        left.retrieved_at
            .cmp(&right.retrieved_at)
            .then_with(|| left.batch_id.cmp(&right.batch_id))
    });

    let start = match position.cursor {
        Some(cursor) => entries
            .iter()
            .position(|entry| entry.batch_id == cursor)
            .filter(|manifest_position| {
                *manifest_position >= start_of_snapshot && *manifest_position < end_of_snapshot
            })
            .and_then(|_| {
                entries[start_of_snapshot..end_of_snapshot]
                    .iter()
                    .position(|entry| entry.batch_id == cursor)
            })
            .map(|cursor_position| cursor_position + 1)
            .ok_or(RecoveryError::Pipeline(
                PipelineError::InvalidRecoveryCursor { cursor },
            ))?,
        None => 0,
    };
    let snapshot_len = end_of_snapshot - start_of_snapshot;
    let end = start.saturating_add(page_size).min(snapshot_len);
    let has_more = end < snapshot_len;
    let mut cursor = position.cursor;

    for manifest in entries
        .into_iter()
        .skip(start_of_snapshot + start)
        .take(end - start)
    {
        let batch_id = manifest.batch_id;
        let state = sink.publication_state(batch_id).await.map_err(|source| {
            RecoveryError::Pipeline(PipelineError::Sink {
                batch_id,
                stage: PipelineStage::PublicationState,
                source,
            })
        })?;
        match state {
            PublicationState::Missing => {
                let bundle = PublicationBundle::from_raw(store, &manifest).map_err(|source| {
                    RecoveryError::Pipeline(PipelineError::Publication { batch_id, source })
                })?;
                sink.publish(&bundle).await.map_err(|source| {
                    RecoveryError::Pipeline(PipelineError::Sink {
                        batch_id,
                        stage: PipelineStage::Publish,
                        source,
                    })
                })?;
                observer(
                    RecoveryBatchOutcome::Recovered {
                        batch_id,
                        date: manifest.date,
                    },
                    snapshot_high_water.expect("a non-empty snapshot has a high-water"),
                )
                .map_err(|source| RecoveryError::Observer { batch_id, source })?;
                cursor = Some(batch_id);
            }
            PublicationState::Complete => {
                let bundle = PublicationBundle::from_raw(store, &manifest).map_err(|source| {
                    RecoveryError::Pipeline(PipelineError::Publication { batch_id, source })
                })?;
                let outcome = sink.publish(&bundle).await.map_err(|source| {
                    RecoveryError::Pipeline(PipelineError::Sink {
                        batch_id,
                        stage: PipelineStage::Publish,
                        source,
                    })
                })?;
                if outcome != PublishOutcome::AlreadyPublished {
                    return Err(RecoveryError::Pipeline(
                        PipelineError::UnexpectedPublishOutcome {
                            batch_id,
                            state,
                            outcome,
                        },
                    ));
                }
                observer(
                    RecoveryBatchOutcome::Skipped {
                        batch_id,
                        date: manifest.date,
                    },
                    snapshot_high_water.expect("a non-empty snapshot has a high-water"),
                )
                .map_err(|source| RecoveryError::Observer { batch_id, source })?;
                cursor = Some(batch_id);
            }
            PublicationState::Partial => {
                return Err(RecoveryError::Pipeline(PipelineError::PartialPublication {
                    batch_id,
                }));
            }
        }
    }
    Ok(RecoveryPage {
        snapshot_high_water,
        cursor,
        has_more,
    })
}
