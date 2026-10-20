//! Explicit EOD source acquisition for the independent R2 price-only consumer.
//!
//! A committed source is not canonical publication, a verified daily input, or
//! recommendation authority. Legacy normalization remains a separate contract.

use domain::BatchId;
use market_data::{
    contract::{MARKET_KR, PROVIDER_KIS},
    ingest::{
        IngestError, IngestRequest, ingest_kis_raw_eod_with_calendar_source,
        verified_existing_kis_eod_source,
    },
    providers::kis::{KisProvider, KisRead},
    storage::{ManifestEntry, RawStore},
};

use crate::{
    PipelineError, calendar_claim::CalendarDayLock, pipeline::ensure_kis_calendar_source_locked,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EodRawSource {
    pub batch_id: BatchId,
    pub reused: bool,
}

fn invalid(detail: &'static str) -> PipelineError {
    PipelineError::Ingest {
        source: IngestError::ResponseShape {
            detail: detail.to_owned(),
        },
    }
}

/// Read and verify an existing committed source before constructing any
/// new delivery. Action-only range evidence is not an EOD source. A conflicting
/// or malformed price-bearing entry is an error, never permission to refetch.
pub fn existing_eod_source(
    store: &RawStore,
    request: &IngestRequest,
    entitlement: &str,
) -> Result<Option<ManifestEntry>, PipelineError> {
    verified_existing_kis_eod_source(store, request, entitlement)
        .map_err(|source| PipelineError::Ingest { source })
}

/// Reuse exact immutable Raw, or capture once through the existing fixed ETF11
/// provider. New acquisition shares the durable calendar claim.
/// No canonical normalizer, publication sink, or database is constructed here.
pub async fn capture_eod_source<R: KisRead>(
    store: &RawStore,
    provider: &KisProvider<R>,
    request: &IngestRequest,
    entitlement: &str,
    current_day: bool,
) -> Result<EodRawSource, PipelineError> {
    capture_eod_source_with_calendar(store, provider, request, entitlement, current_day, None)
        .await
        .map(|(source, _)| source)
}

/// Historical multi-date callers retain the returned committed calendar
/// original. Its coverage is checked before any non-calendar request; a claim
/// already reserved for this target takes precedence and cannot be bypassed.
pub async fn capture_eod_source_with_calendar<R: KisRead>(
    store: &RawStore,
    provider: &KisProvider<R>,
    request: &IngestRequest,
    entitlement: &str,
    current_day: bool,
    historical_calendar: Option<&ManifestEntry>,
) -> Result<(EodRawSource, Option<ManifestEntry>), PipelineError> {
    if request.market != MARKET_KR || entitlement.trim().is_empty() {
        return Err(invalid("EOD_RAW_REQUEST_INVALID"));
    }
    if current_day && historical_calendar.is_some() {
        return Err(invalid("EOD_RAW_CURRENT_DAY_REQUIRES_OWN_CALENDAR"));
    }
    let guard = CalendarDayLock::acquire(&store.root().join("raw"), request.date)
        .map_err(|_| invalid("EOD_RAW_SOURCE_STATE_INVALID_OR_BUSY"))?;
    // Only the executing path reconciles crash residue. The check command
    // requires existing committed evidence and never creates a lock/manifest.
    store
        .read_reconciled_manifest(PROVIDER_KIS, MARKET_KR)
        .map_err(|source| PipelineError::Manifest { source })?;
    let existing = existing_eod_source(store, request, entitlement)?;
    let has_claim = guard
        .existing()
        .map_err(|_| invalid("EOD_RAW_CALENDAR_STATE_INVALID"))?
        .is_some();
    let calendar =
        if current_day || has_claim || (existing.is_none() && historical_calendar.is_none()) {
            ensure_kis_calendar_source_locked(store, provider, request, Some(entitlement), &guard)
                .await?
        } else {
            historical_calendar.cloned()
        };
    if let Some(source) = existing {
        return Ok((
            EodRawSource {
                batch_id: source.batch_id,
                reused: true,
            },
            calendar,
        ));
    }
    let calendar = calendar.ok_or_else(|| invalid("EOD_RAW_CALENDAR_ORIGINAL_REQUIRED"))?;
    let captured = ingest_kis_raw_eod_with_calendar_source(
        store,
        provider,
        request,
        Some(entitlement),
        &calendar,
    )
    .await
    .map_err(|source| PipelineError::Ingest { source })?;
    Ok((
        EodRawSource {
            batch_id: captured.batch_id,
            reused: false,
        },
        Some(calendar),
    ))
}
