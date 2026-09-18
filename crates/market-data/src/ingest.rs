//! The collector pipeline: fetch -> validate -> store -> manifest (Todo 8).
//!
//! [`ingest_bundle`] drives one delivery end-to-end: the provider returns raw
//! envelopes, the pipeline validates their structure, persists every byte
//! unchanged into a fresh immutable batch, and appends one manifest row. Any
//! failure is a typed [`IngestError`]. A failed pre-visible cleanup can leave a
//! non-committed directory that recovery ignores; once final metadata is visible,
//! any failure leaves an exact-identity batch for [`RawStore::read_manifest`]
//! to re-sync before recovery.

use std::collections::{BTreeMap, BTreeSet};

use domain::{BatchId, TradingDate, UtcTimestamp};
use serde_json::Value;

use crate::contract::{
    FetchMode, MARKET_KR, PROVIDER_KIS, PROVIDER_KIS_CALENDAR, PROVIDER_KIS_DAILY_RANGE,
    RawEnvelope, ResponseKind, SourceFileReference, StoredFile,
};
use crate::provider::{EodProvider, ProviderError};
use crate::providers::kis::{
    CALENDAR_FILE_NAME, CALENDAR_PATH, KisActionRangeScope, KisProvider, KisRead, calendar_query,
    calendar_request_headers, validate_kis_response,
};
use crate::providers::kis_candidate::{
    KIS_CANDIDATE_SUPPORTED_KINDS, KisCandidateProvider, validate_kis_candidate_response,
};
use crate::storage::{BatchSpec, ManifestEntry, RawStore, StoreError};
use crate::validate::{ValidationError, validate_response};

/// A typed failure of the whole ingestion pipeline.
#[derive(Debug)]
pub enum IngestError {
    /// The provider failed (timeout, credentials, unsafe file name, ...).
    Provider(ProviderError),
    /// The response bytes failed structural schema validation.
    MalformedResponse {
        kind: ResponseKind,
        reason: String,
        diagnostic: Option<ResponseValidationDiagnostic>,
    },
    /// The provider returned a partial or out-of-scope response-class set.
    ResponseShape { detail: String },
    /// The immutable store rejected the batch.
    Store(StoreError),
    /// The batch was stored, but its mandatory post-store verification failed.
    Readback {
        entry: Box<ManifestEntry>,
        source: StoreError,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseValidationDiagnostic {
    pub code: &'static str,
    pub endpoint: String,
    pub file_name: String,
}

impl std::fmt::Display for IngestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Provider(e) => write!(f, "ingest provider failure: {e}"),
            Self::MalformedResponse { kind, reason, .. } => {
                write!(f, "malformed {kind} response: {reason}")
            }
            Self::ResponseShape { detail } => {
                write!(f, "invalid provider response shape: {detail}")
            }
            Self::Store(e) => write!(f, "ingest store failure: {e}"),
            Self::Readback { entry, source } => write!(
                f,
                "ingest readback failed for stored batch {}: {source}",
                entry.batch_id
            ),
        }
    }
}

impl std::error::Error for IngestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Provider(source) => Some(source),
            Self::Store(source) => Some(source),
            Self::Readback { source, .. } => Some(source),
            Self::MalformedResponse { .. } | Self::ResponseShape { .. } => None,
        }
    }
}

impl IngestError {
    pub fn batch_id(&self) -> Option<BatchId> {
        match self {
            Self::Store(source) => source.batch_id(),
            Self::Readback { entry, .. } => Some(entry.batch_id),
            Self::Provider(_) | Self::MalformedResponse { .. } | Self::ResponseShape { .. } => None,
        }
    }
}

impl From<ProviderError> for IngestError {
    fn from(e: ProviderError) -> Self {
        Self::Provider(e)
    }
}

impl From<StoreError> for IngestError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

impl From<ValidationError> for IngestError {
    fn from(e: ValidationError) -> Self {
        Self::MalformedResponse {
            kind: e.kind,
            reason: e.reason,
            diagnostic: None,
        }
    }
}

/// One delivery request: market, data date, and the retrieval clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestRequest {
    pub market: String,
    pub date: TradingDate,
    pub now: UtcTimestamp,
}

impl IngestRequest {
    pub fn new(market: String, date: TradingDate, now: UtcTimestamp) -> Self {
        Self { market, date, now }
    }
}

/// The outcome of a successful delivery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestOutcome {
    pub batch_id: BatchId,
    pub entry: ManifestEntry,
    pub files: Vec<StoredFile>,
}

trait IngestStore {
    fn store_batch(
        &self,
        spec: &BatchSpec<'_>,
        envelopes: &[crate::contract::RawEnvelope],
    ) -> Result<ManifestEntry, StoreError>;

    fn read_batch_bytes(
        &self,
        provider: &str,
        market: &str,
        entry: &ManifestEntry,
    ) -> Result<Vec<StoredFile>, StoreError>;
}

impl IngestStore for RawStore {
    fn store_batch(
        &self,
        spec: &BatchSpec<'_>,
        envelopes: &[crate::contract::RawEnvelope],
    ) -> Result<ManifestEntry, StoreError> {
        Self::store_batch(self, spec, envelopes)
    }

    fn read_batch_bytes(
        &self,
        provider: &str,
        market: &str,
        entry: &ManifestEntry,
    ) -> Result<Vec<StoredFile>, StoreError> {
        Self::read_batch_bytes(self, provider, market, entry)
    }
}

/// Runs one delivery: fetch all licensed response classes, validate, persist
/// as a new immutable batch, append the manifest row.
///
/// `entitlement_reference` is the governing licensed-data contract reference
/// recorded on the manifest row (see
/// [`crate::entitlement::governing_entitlement_reference`]); `None` records an
/// unlicensed batch.
pub fn ingest_bundle(
    store: &RawStore,
    provider: &dyn EodProvider,
    req: &IngestRequest,
    entitlement_reference: Option<&str>,
) -> Result<IngestOutcome, IngestError> {
    ingest_bundle_with_store(store, provider, req, entitlement_reference)
}

/// Runs one immutable delivery for an explicit provider capability set.
///
/// Candidate-source collection uses this entry point with
/// [`crate::CANDIDATE_RESPONSE_KINDS`]. Every requested response class must be
/// present at least once and the provider may not smuggle an unrequested class
/// into the batch. Multiple files of one class remain valid for paginated
/// licensed deliveries.
pub fn ingest_bundle_with_kinds(
    store: &RawStore,
    provider: &dyn EodProvider,
    req: &IngestRequest,
    entitlement_reference: Option<&str>,
    kinds: &[ResponseKind],
) -> Result<IngestOutcome, IngestError> {
    ingest_bundle_with_kinds_and_store(store, provider, req, entitlement_reference, kinds)
}

/// Fetches one credentialed KIS EOD delivery and persists every broker reply
/// byte-for-byte in one immutable Raw batch.
///
/// KIS is async because token issuance, rate limiting, retry sleeps, and HTTP
/// are async. Its wire documents are validated against the endpoint-specific
/// KIS shape rather than the recorded provider-neutral fixture shape.
pub async fn ingest_kis_bundle<R: KisRead>(
    store: &RawStore,
    provider: &KisProvider<R>,
    req: &IngestRequest,
    entitlement_reference: Option<&str>,
) -> Result<IngestOutcome, IngestError> {
    if let Some(calendar_source) = committed_calendar_source(store, req, entitlement_reference)? {
        return ingest_kis_bundle_with_calendar_source(
            store,
            provider,
            req,
            entitlement_reference,
            &calendar_source,
        )
        .await;
    }
    reject_indeterminate_calendar_claim(store, req)?;
    if let Some(legacy) = committed_complete_eod_source(store, req, entitlement_reference)? {
        return reuse_committed_outcome(store, legacy);
    }
    let batch_id = BatchId::generate();
    let fetch_req = crate::provider::FetchRequest {
        market: req.market.clone(),
        date: req.date,
        kinds: crate::contract::EOD_RESPONSE_KINDS.to_vec(),
        now: req.now,
        batch_id,
    };
    let envelopes = provider.fetch(&fetch_req).await?;
    validate_returned_kinds(&crate::contract::EOD_RESPONSE_KINDS, &envelopes)?;
    for envelope in &envelopes {
        validate_kis_response(envelope.kind, &envelope.request.endpoint, &envelope.bytes).map_err(
            |error| IngestError::MalformedResponse {
                kind: error.kind,
                reason: error.reason,
                diagnostic: Some(ResponseValidationDiagnostic {
                    code: error.code,
                    endpoint: envelope.request.endpoint.clone(),
                    file_name: envelope.file_name.clone(),
                }),
            },
        )?;
    }
    persist_bundle(
        store,
        provider.provider_id(),
        provider.fetch_mode(),
        req,
        entitlement_reference,
        batch_id,
        &envelopes,
    )
}

/// Fetches the three non-calendar KIS EOD classes and incorporates the exact
/// committed calendar source supplied by the shared daily calendar path.
///
/// The calendar envelope is copied with its original request metadata, bytes,
/// hash, and retrieval time. The immutable store verifies the reference again
/// before any destination metadata becomes visible.
pub async fn ingest_kis_bundle_with_calendar_source<R: KisRead>(
    store: &RawStore,
    provider: &KisProvider<R>,
    req: &IngestRequest,
    entitlement_reference: Option<&str>,
    calendar_source: &ManifestEntry,
) -> Result<IngestOutcome, IngestError> {
    if calendar_source.provider != PROVIDER_KIS_CALENDAR
        || calendar_source.market != MARKET_KR
        || calendar_source.date != req.date
        || calendar_source.mode != FetchMode::Credentialed
        || calendar_source.entitlement_reference.as_deref() != entitlement_reference
        || calendar_source.retrieved_at > req.now
        || calendar_source.retrieved_at > UtcTimestamp::now()
    {
        return Err(IngestError::ResponseShape {
            detail: "KIS calendar source does not exactly match the EOD request".to_owned(),
        });
    }
    let calendar_stored = store.read_batch_bytes(
        &calendar_source.provider,
        &calendar_source.market,
        calendar_source,
    )?;
    crate::normalize::validate_kis_calendar_source_manifest(calendar_source, &calendar_stored)
        .map_err(|error| IngestError::ResponseShape {
            detail: format!("KIS calendar source contract failed: {error}"),
        })?;
    let calendar_file = &calendar_source.files[0];
    let calendar_bytes = calendar_stored
        .iter()
        .find(|file| file.file_name == calendar_file.file_name)
        .ok_or_else(|| IngestError::ResponseShape {
            detail: "KIS calendar source file is missing from verified Raw".to_owned(),
        })?;
    let batch_id = BatchId::generate();
    let fetch_req = crate::provider::FetchRequest {
        market: req.market.clone(),
        date: req.date,
        kinds: vec![
            ResponseKind::Bars,
            ResponseKind::Reference,
            ResponseKind::CorporateActions,
        ],
        now: req.now,
        batch_id,
    };
    let fetched = provider.fetch(&fetch_req).await?;
    validate_kis_eod_envelopes(&fetched)?;
    let calendar = RawEnvelope::new(
        batch_id,
        ResponseKind::Calendar,
        calendar_file.file_name.clone(),
        calendar_bytes.bytes.clone(),
        calendar_source.retrieved_at,
        calendar_file.request.clone(),
    )
    .with_response_continuation(calendar_file.response_continuation.clone())
    .with_copied_from(Some(SourceFileReference {
        provider: calendar_source.provider.clone(),
        market: calendar_source.market.clone(),
        batch_id: calendar_source.batch_id,
        file_name: calendar_file.file_name.clone(),
        content_hash: calendar_file.content_hash.clone(),
        retrieved_at: calendar_source.retrieved_at,
    }));
    let mut envelopes = Vec::with_capacity(fetched.len() + 1);
    for kind in [ResponseKind::Bars, ResponseKind::Reference] {
        envelopes.extend(
            fetched
                .iter()
                .filter(|envelope| envelope.kind == kind)
                .cloned(),
        );
    }
    envelopes.push(calendar);
    envelopes.extend(
        fetched
            .into_iter()
            .filter(|envelope| envelope.kind == ResponseKind::CorporateActions),
    );
    persist_bundle(
        store,
        provider.provider_id(),
        provider.fetch_mode(),
        req,
        entitlement_reference,
        batch_id,
        &envelopes,
    )
}

fn validate_kis_eod_envelopes(envelopes: &[RawEnvelope]) -> Result<(), IngestError> {
    validate_returned_kinds(
        &[
            ResponseKind::Bars,
            ResponseKind::Reference,
            ResponseKind::CorporateActions,
        ],
        envelopes,
    )?;
    for envelope in envelopes {
        validate_kis_response(envelope.kind, &envelope.request.endpoint, &envelope.bytes).map_err(
            |error| IngestError::MalformedResponse {
                kind: error.kind,
                reason: error.reason,
                diagnostic: Some(ResponseValidationDiagnostic {
                    code: error.code,
                    endpoint: envelope.request.endpoint.clone(),
                    file_name: envelope.file_name.clone(),
                }),
            },
        )?;
    }
    Ok(())
}

/// A committed calendar source is preferred even when the claim artifact was
/// written by a previous process. A claim without that exact source remains
/// indeterminate and cannot trigger a recapture.
fn committed_calendar_source(
    store: &RawStore,
    req: &IngestRequest,
    entitlement_reference: Option<&str>,
) -> Result<Option<ManifestEntry>, IngestError> {
    let path = store.manifest_path(PROVIDER_KIS_CALENDAR, MARKET_KR);
    if !path.exists() {
        return Ok(None);
    }
    let entries = store.read_committed_manifest(PROVIDER_KIS_CALENDAR, MARKET_KR)?;
    let matches: Vec<_> = entries
        .into_iter()
        .filter(|entry| entry.date == req.date)
        .collect();
    if matches.is_empty() {
        return Ok(None);
    }
    if matches.len() != 1 {
        return Err(IngestError::ResponseShape {
            detail: "multiple committed KIS calendar sources exist for the target date".to_owned(),
        });
    }
    let source = matches.into_iter().next().expect("one source validated");
    if source.mode != FetchMode::Credentialed
        || source.entitlement_reference.as_deref() != entitlement_reference
    {
        return Err(IngestError::ResponseShape {
            detail: "committed KIS calendar source has incompatible mode or entitlement".to_owned(),
        });
    }
    Ok(Some(source))
}

fn committed_complete_eod_source(
    store: &RawStore,
    req: &IngestRequest,
    entitlement_reference: Option<&str>,
) -> Result<Option<ManifestEntry>, IngestError> {
    let path = store.manifest_path(PROVIDER_KIS, MARKET_KR);
    if !path.exists() {
        return Ok(None);
    }
    let entries = store.read_committed_manifest(PROVIDER_KIS, MARKET_KR)?;
    let mut matches = entries.into_iter().filter(|entry| {
        entry.date == req.date
            && entry.mode == FetchMode::Credentialed
            && entry.entitlement_reference.as_deref() == entitlement_reference
            && entry.retrieved_at <= req.now
            && entry.retrieved_at <= UtcTimestamp::now()
            && entry.files.len() >= crate::contract::EOD_RESPONSE_KINDS.len()
            && crate::contract::EOD_RESPONSE_KINDS
                .iter()
                .all(|kind| entry.files.iter().any(|file| file.kind == *kind))
            && entry
                .files
                .iter()
                .filter(|file| file.kind == ResponseKind::Calendar)
                .count()
                == 1
    });
    let first = matches.next();
    if matches.next().is_some() {
        return Err(IngestError::ResponseShape {
            detail: "multiple committed KIS EOD sources exist for the target date".to_owned(),
        });
    }
    Ok(first)
}

fn reuse_committed_outcome(
    store: &RawStore,
    entry: ManifestEntry,
) -> Result<IngestOutcome, IngestError> {
    let files = store.read_batch_bytes(&entry.provider, &entry.market, &entry)?;
    Ok(IngestOutcome {
        batch_id: entry.batch_id,
        entry,
        files,
    })
}

fn reject_indeterminate_calendar_claim(
    store: &RawStore,
    req: &IngestRequest,
) -> Result<(), IngestError> {
    let blocked = || IngestError::ResponseShape {
        detail: "KIS_CALENDAR_SOURCE_INDETERMINATE".to_owned(),
    };
    let claim = store
        .root()
        .join("raw/.calendar-bootstrap")
        .join(format!("{}.json", req.date.to_iso()));
    match std::fs::symlink_metadata(claim) {
        Ok(_) => return Err(blocked()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(blocked()),
    }
    Ok(())
}

/// Fetches one credentialed KIS `chk-holiday` proof and persists it under the
/// dedicated calendar-only Raw scope. This seam intentionally does not call
/// the four-file EOD collector: the calendar proof is independently useful to
/// the current-price path and must not make a partial EOD batch visible.
pub async fn ingest_kis_calendar<R: KisRead>(
    store: &RawStore,
    provider: &KisProvider<R>,
    req: &IngestRequest,
    entitlement_reference: Option<&str>,
) -> Result<IngestOutcome, IngestError> {
    ingest_kis_calendar_with_batch_id(
        store,
        provider,
        req,
        entitlement_reference,
        BatchId::generate(),
    )
    .await
}

/// Same calendar-only proof capture with a caller-owned immutable Raw batch id.
/// The caller may persist this id in an external attempt artifact and retry
/// the exact operation without inventing a second lineage identity.
pub async fn ingest_kis_calendar_with_batch_id<R: KisRead>(
    store: &RawStore,
    provider: &KisProvider<R>,
    req: &IngestRequest,
    entitlement_reference: Option<&str>,
    batch_id: BatchId,
) -> Result<IngestOutcome, IngestError> {
    if req.market != MARKET_KR {
        return Err(IngestError::ResponseShape {
            detail: "KIS calendar capture supports market kr only".to_owned(),
        });
    }
    let fetch_req = crate::provider::FetchRequest {
        market: req.market.clone(),
        date: req.date,
        kinds: vec![ResponseKind::Calendar],
        now: req.now,
        batch_id,
    };
    let envelopes = provider.fetch(&fetch_req).await?;
    validate_kis_calendar_envelopes(req, batch_id, &envelopes)?;
    persist_bundle(
        store,
        PROVIDER_KIS_CALENDAR,
        provider.fetch_mode(),
        req,
        entitlement_reference,
        batch_id,
        &envelopes,
    )
}

fn validate_kis_calendar_envelopes(
    req: &IngestRequest,
    batch_id: BatchId,
    envelopes: &[crate::contract::RawEnvelope],
) -> Result<(), IngestError> {
    if envelopes.len() != 1 {
        return Err(IngestError::ResponseShape {
            detail: format!(
                "KIS calendar capture expected exactly one envelope, got {}",
                envelopes.len()
            ),
        });
    }
    let envelope = &envelopes[0];
    if envelope.kind != ResponseKind::Calendar {
        return Err(IngestError::ResponseShape {
            detail: "KIS calendar capture returned an unexpected response kind".to_owned(),
        });
    }
    if envelope.batch_id != batch_id {
        return Err(IngestError::ResponseShape {
            detail: "KIS calendar envelope batch identity differs from the caller-owned id"
                .to_owned(),
        });
    }
    if envelope.file_name != CALENDAR_FILE_NAME {
        return Err(calendar_validation_error(
            &envelope.file_name,
            "KIS_CALENDAR_FILE_NAME",
            "KIS calendar response used a noncanonical file name",
        ));
    }
    if envelope.retrieved_at != req.now {
        return Err(calendar_validation_error(
            &envelope.file_name,
            "KIS_CALENDAR_RETRIEVED_AT",
            "KIS calendar envelope retrieval time differs from the request clock",
        ));
    }
    if envelope.request.mode != FetchMode::Credentialed
        || envelope.request.endpoint != CALENDAR_PATH
        || envelope.request.query != calendar_query(req.date)
        || envelope.request.headers != calendar_request_headers()
    {
        return Err(calendar_validation_error(
            &envelope.file_name,
            "KIS_CALENDAR_REQUEST_SHAPE",
            "KIS calendar request metadata is not the exact single-page contract",
        ));
    }
    if envelope.response_continuation.is_some() {
        return Err(calendar_validation_error(
            &envelope.file_name,
            "KIS_CALENDAR_CONTINUATION",
            "KIS calendar response carried continuation metadata",
        ));
    }
    validate_kis_response(ResponseKind::Calendar, CALENDAR_PATH, &envelope.bytes).map_err(
        |error| IngestError::MalformedResponse {
            kind: error.kind,
            reason: error.reason,
            diagnostic: Some(ResponseValidationDiagnostic {
                code: error.code,
                endpoint: envelope.request.endpoint.clone(),
                file_name: envelope.file_name.clone(),
            }),
        },
    )?;
    validate_kis_calendar_rows(&envelope.file_name, &envelope.bytes, req.date)
}

fn validate_kis_calendar_rows(
    file_name: &str,
    bytes: &[u8],
    target_date: TradingDate,
) -> Result<(), IngestError> {
    let document: Value = serde_json::from_slice(bytes).map_err(|_| {
        calendar_validation_error(
            file_name,
            "KIS_CALENDAR_SCHEMA",
            "KIS calendar response was not valid JSON",
        )
    })?;
    let object = document.as_object().ok_or_else(|| {
        calendar_validation_error(
            file_name,
            "KIS_CALENDAR_SCHEMA",
            "KIS calendar response was not a JSON object",
        )
    })?;
    let output = object.get("output").ok_or_else(|| {
        calendar_validation_error(
            file_name,
            "KIS_CALENDAR_SCHEMA",
            "KIS calendar response had no output",
        )
    })?;
    let rows: Vec<&Value> = match output {
        Value::Array(rows) => rows.iter().collect(),
        Value::Object(_) => vec![output],
        _ => {
            return Err(calendar_validation_error(
                file_name,
                "KIS_CALENDAR_SCHEMA",
                "KIS calendar output was not an object or array",
            ));
        }
    };
    let mut dates = BTreeMap::new();
    for row in rows {
        let row = row.as_object().ok_or_else(|| {
            calendar_validation_error(
                file_name,
                "KIS_CALENDAR_SCHEMA",
                "KIS calendar output row was not an object",
            )
        })?;
        let date_text = row.get("bass_dt").and_then(Value::as_str).ok_or_else(|| {
            calendar_validation_error(
                file_name,
                "KIS_CALENDAR_SCHEMA",
                "KIS calendar output row had no bass_dt",
            )
        })?;
        let date = parse_kis_calendar_date(date_text).ok_or_else(|| {
            calendar_validation_error(
                file_name,
                "KIS_CALENDAR_DATE",
                "KIS calendar output row had an invalid bass_dt",
            )
        })?;
        let is_open = match row.get("opnd_yn").and_then(Value::as_str) {
            Some("Y") => true,
            Some("N") => false,
            _ => {
                return Err(calendar_validation_error(
                    file_name,
                    "KIS_CALENDAR_SESSION_TYPE",
                    "KIS calendar output row had an invalid opnd_yn",
                ));
            }
        };
        if dates.insert(date, is_open).is_some() {
            return Err(calendar_validation_error(
                file_name,
                "KIS_CALENDAR_DUPLICATE_DATE",
                "KIS calendar output contained a duplicate date",
            ));
        }
    }
    if !dates.contains_key(&target_date) {
        return Err(calendar_validation_error(
            file_name,
            "KIS_CALENDAR_TARGET_MISSING",
            "KIS calendar output did not contain the requested date",
        ));
    }
    Ok(())
}

fn parse_kis_calendar_date(value: &str) -> Option<TradingDate> {
    if value.len() != 8 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    TradingDate::parse(&format!("{}-{}-{}", &value[..4], &value[4..6], &value[6..])).ok()
}

fn calendar_validation_error(file_name: &str, code: &'static str, reason: &str) -> IngestError {
    IngestError::MalformedResponse {
        kind: ResponseKind::Calendar,
        reason: reason.to_owned(),
        diagnostic: Some(ResponseValidationDiagnostic {
            code,
            endpoint: CALENDAR_PATH.to_owned(),
            file_name: file_name.to_owned(),
        }),
    }
}

/// Captures a bounded historical KIS daily-bar range as immutable Raw only.
///
/// The range provider deliberately uses a separate `kis-daily-range` scope:
/// the existing normalizer/publication contract is target-date based and must
/// not silently reinterpret a multi-date wire response.  This function is a
/// safe acquisition seam for the later range-aware normalizer; it performs no
/// calendar, current-price, account, or order request.
pub async fn ingest_kis_daily_bars_range<R: KisRead>(
    store: &RawStore,
    provider: &KisProvider<R>,
    market: &str,
    start: TradingDate,
    end: TradingDate,
    now: UtcTimestamp,
    entitlement_reference: Option<&str>,
) -> Result<IngestOutcome, IngestError> {
    ingest_kis_daily_bars_range_with_batch_id(
        store,
        provider,
        market,
        start,
        end,
        now,
        entitlement_reference,
        BatchId::generate(),
    )
    .await
}

/// Same Raw-only range capture with an explicit caller-owned batch identity.
///
/// A guarded operator wrapper may persist this identity before starting a
/// process and reuse it after interruption. Raw remains immutable: a retry
/// with different bytes returns the normal store conflict rather than
/// overwriting the original evidence.
#[allow(clippy::too_many_arguments)]
pub async fn ingest_kis_daily_bars_range_with_batch_id<R: KisRead>(
    store: &RawStore,
    provider: &KisProvider<R>,
    market: &str,
    start: TradingDate,
    end: TradingDate,
    now: UtcTimestamp,
    entitlement_reference: Option<&str>,
    batch_id: BatchId,
) -> Result<IngestOutcome, IngestError> {
    let envelopes = provider
        .fetch_daily_bars_range(market, start, end, now, batch_id)
        .await?;
    validate_returned_kinds(&[ResponseKind::Bars], &envelopes)?;
    for envelope in &envelopes {
        validate_kis_response(envelope.kind, &envelope.request.endpoint, &envelope.bytes).map_err(
            |error| IngestError::MalformedResponse {
                kind: error.kind,
                reason: error.reason,
                diagnostic: Some(ResponseValidationDiagnostic {
                    code: error.code,
                    endpoint: envelope.request.endpoint.clone(),
                    file_name: envelope.file_name.clone(),
                }),
            },
        )?;
    }
    let request = IngestRequest::new(market.to_owned(), start, now);
    persist_bundle(
        store,
        PROVIDER_KIS_DAILY_RANGE,
        provider.fetch_mode(),
        &request,
        entitlement_reference,
        batch_id,
        &envelopes,
    )
}

/// Fetches a complete KIS KSD action-range matrix and persists it as one
/// immutable `provider=kis` Raw batch. The provider returns no envelopes on
/// a failed/incomplete class or pagination chain, and RawStore publishes the
/// batch only after every class/page has been written and hashed.
#[allow(clippy::too_many_arguments)]
pub async fn ingest_kis_action_range<R: KisRead>(
    store: &RawStore,
    provider: &KisProvider<R>,
    market: &str,
    start: TradingDate,
    end: TradingDate,
    now: UtcTimestamp,
    scope: KisActionRangeScope,
    entitlement_reference: Option<&str>,
) -> Result<IngestOutcome, IngestError> {
    ingest_kis_action_range_with_batch_id(
        store,
        provider,
        market,
        start,
        end,
        now,
        scope,
        entitlement_reference,
        BatchId::generate(),
    )
    .await
}

/// Same action-range Raw capture with an explicit caller-owned batch id.
/// Whole-market and symbol-scoped captures both remain one atomic batch; a
/// failed symbol/class never permits a partial manifest row.
#[allow(clippy::too_many_arguments)]
pub async fn ingest_kis_action_range_with_batch_id<R: KisRead>(
    store: &RawStore,
    provider: &KisProvider<R>,
    market: &str,
    start: TradingDate,
    end: TradingDate,
    now: UtcTimestamp,
    scope: KisActionRangeScope,
    entitlement_reference: Option<&str>,
    batch_id: BatchId,
) -> Result<IngestOutcome, IngestError> {
    let envelopes = provider
        .fetch_corporate_actions_range(market, start, end, now, batch_id, scope)
        .await?;
    if envelopes.is_empty() {
        return Err(IngestError::ResponseShape {
            detail: "KIS action-range capture returned no response pages".to_owned(),
        });
    }
    for envelope in &envelopes {
        validate_kis_response(envelope.kind, &envelope.request.endpoint, &envelope.bytes).map_err(
            |error| IngestError::MalformedResponse {
                kind: error.kind,
                reason: error.reason,
                diagnostic: Some(ResponseValidationDiagnostic {
                    code: error.code,
                    endpoint: envelope.request.endpoint.clone(),
                    file_name: envelope.file_name.clone(),
                }),
            },
        )?;
    }
    let request = IngestRequest::new(market.to_owned(), start, now);
    persist_bundle(
        store,
        PROVIDER_KIS,
        provider.fetch_mode(),
        &request,
        entitlement_reference,
        batch_id,
        &envelopes,
    )
}

/// Fetches one candidate-source delivery through the authenticated KIS REST
/// adapter and persists the exact broker responses under the dedicated
/// `provider=kis-candidate` scope.
///
/// The provider intentionally supports only the REST-backed investor-flow and
/// fundamentals classes.  Membership/sector master files and complete market
/// status are rejected before any request is sent; callers must not treat a
/// partial candidate delivery as publishable.
pub async fn ingest_kis_candidate_bundle<R: KisRead>(
    store: &RawStore,
    provider: &KisCandidateProvider<R>,
    req: &IngestRequest,
    entitlement_reference: Option<&str>,
) -> Result<IngestOutcome, IngestError> {
    ingest_kis_candidate_bundle_with_kinds(
        store,
        provider,
        req,
        entitlement_reference,
        &KIS_CANDIDATE_SUPPORTED_KINDS,
    )
    .await
}

/// Fetches and persists an explicit subset of the REST-backed candidate
/// classes. Finance responses are intentionally accepted here as immutable
/// Raw evidence, while candidate normalization fails closed until their
/// semantics have a reviewed mapping.
pub async fn ingest_kis_candidate_bundle_with_kinds<R: KisRead>(
    store: &RawStore,
    provider: &KisCandidateProvider<R>,
    req: &IngestRequest,
    entitlement_reference: Option<&str>,
    kinds: &[ResponseKind],
) -> Result<IngestOutcome, IngestError> {
    let batch_id = BatchId::generate();
    let fetch_req = crate::provider::FetchRequest {
        market: req.market.clone(),
        date: req.date,
        kinds: kinds.to_vec(),
        now: req.now,
        batch_id,
    };
    let envelopes = provider.fetch(&fetch_req).await?;
    validate_returned_kinds(kinds, &envelopes)?;
    for envelope in &envelopes {
        validate_kis_candidate_response(envelope.kind, &envelope.request.endpoint, &envelope.bytes)
            .map_err(|reason| IngestError::MalformedResponse {
                kind: envelope.kind,
                reason,
                diagnostic: None,
            })?;
    }
    persist_bundle(
        store,
        provider.provider_id(),
        provider.fetch_mode(),
        req,
        entitlement_reference,
        batch_id,
        &envelopes,
    )
}

fn ingest_bundle_with_store<S: IngestStore + ?Sized>(
    store: &S,
    provider: &dyn EodProvider,
    req: &IngestRequest,
    entitlement_reference: Option<&str>,
) -> Result<IngestOutcome, IngestError> {
    ingest_bundle_with_kinds_and_store(
        store,
        provider,
        req,
        entitlement_reference,
        &crate::contract::EOD_RESPONSE_KINDS,
    )
}

fn ingest_bundle_with_kinds_and_store<S: IngestStore + ?Sized>(
    store: &S,
    provider: &dyn EodProvider,
    req: &IngestRequest,
    entitlement_reference: Option<&str>,
    kinds: &[ResponseKind],
) -> Result<IngestOutcome, IngestError> {
    let requested: BTreeSet<_> = kinds.iter().copied().collect();
    if requested.is_empty() || requested.len() != kinds.len() {
        return Err(IngestError::ResponseShape {
            detail: "requested response classes must be nonempty and unique".to_owned(),
        });
    }
    let batch_id = BatchId::generate();
    let fetch_req = crate::provider::FetchRequest {
        market: req.market.clone(),
        date: req.date,
        kinds: kinds.to_vec(),
        now: req.now,
        batch_id,
    };
    let envelopes = provider.fetch(&fetch_req)?;

    validate_returned_kinds(kinds, &envelopes)?;

    for env in &envelopes {
        validate_response(env.kind, &env.bytes)?;
    }

    persist_bundle(
        store,
        provider.provider_id(),
        provider.fetch_mode(),
        req,
        entitlement_reference,
        batch_id,
        &envelopes,
    )
}

fn validate_returned_kinds(
    kinds: &[ResponseKind],
    envelopes: &[crate::contract::RawEnvelope],
) -> Result<(), IngestError> {
    let requested: BTreeSet<_> = kinds.iter().copied().collect();
    if requested.is_empty() || requested.len() != kinds.len() {
        return Err(IngestError::ResponseShape {
            detail: "requested response classes must be nonempty and unique".to_owned(),
        });
    }
    let returned: BTreeSet<_> = envelopes.iter().map(|envelope| envelope.kind).collect();
    let missing: Vec<_> = requested
        .difference(&returned)
        .map(ToString::to_string)
        .collect();
    let unexpected: Vec<_> = returned
        .difference(&requested)
        .map(ToString::to_string)
        .collect();
    if !missing.is_empty() || !unexpected.is_empty() {
        return Err(IngestError::ResponseShape {
            detail: format!(
                "missing [{}], unexpected [{}]",
                missing.join(","),
                unexpected.join(",")
            ),
        });
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn persist_bundle<S: IngestStore + ?Sized>(
    store: &S,
    provider_id: &str,
    fetch_mode: crate::contract::FetchMode,
    req: &IngestRequest,
    entitlement_reference: Option<&str>,
    batch_id: BatchId,
    envelopes: &[crate::contract::RawEnvelope],
) -> Result<IngestOutcome, IngestError> {
    let spec = BatchSpec {
        provider: provider_id,
        market: &req.market,
        date: &req.date,
        batch_id,
        entitlement_reference,
        mode: fetch_mode,
    };
    let entry = store.store_batch(&spec, envelopes)?;

    let files = store
        .read_batch_bytes(&entry.provider, &entry.market, &entry)
        .map_err(|source| IngestError::Readback {
            entry: Box::new(entry.clone()),
            source,
        })?;

    Ok(IngestOutcome {
        batch_id,
        entry,
        files,
    })
}

/// Candidate-master's explicit ingest path uses the same immutable Raw
/// transaction as the EOD/candidate JSON paths, but is kept crate-private so
/// callers cannot accidentally widen the generic EOD provider surface.
pub(crate) fn persist_candidate_master_bundle(
    store: &RawStore,
    provider_id: &str,
    fetch_mode: crate::contract::FetchMode,
    req: &IngestRequest,
    entitlement_reference: Option<&str>,
    batch_id: BatchId,
    envelopes: &[crate::contract::RawEnvelope],
) -> Result<IngestOutcome, IngestError> {
    persist_bundle(
        store,
        provider_id,
        fetch_mode,
        req,
        entitlement_reference,
        batch_id,
        envelopes,
    )
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use crate::contract::{MARKET_KR, PROVIDER_KRX};
    use crate::provider::{KrxProvider, RecordedBundle};

    use super::*;

    #[derive(Debug)]
    struct FailingReadbackStore {
        raw: RawStore,
        fail_readback: AtomicBool,
    }

    impl IngestStore for FailingReadbackStore {
        fn store_batch(
            &self,
            spec: &BatchSpec<'_>,
            envelopes: &[crate::contract::RawEnvelope],
        ) -> Result<ManifestEntry, StoreError> {
            self.raw.store_batch(spec, envelopes)
        }

        fn read_batch_bytes(
            &self,
            provider: &str,
            market: &str,
            entry: &ManifestEntry,
        ) -> Result<Vec<StoredFile>, StoreError> {
            if self.fail_readback.swap(false, Ordering::SeqCst) {
                Err(StoreError::Io {
                    context: "injected post-store readback".to_owned(),
                    source: std::io::Error::other("injected readback failure"),
                })
            } else {
                self.raw.read_batch_bytes(provider, market, entry)
            }
        }
    }

    #[test]
    fn post_store_readback_failure_retains_exact_manifest_identity_and_source() {
        let root = tempfile::tempdir().unwrap();
        let store = FailingReadbackStore {
            raw: RawStore::new(root.path()),
            fail_readback: AtomicBool::new(true),
        };
        let provider = KrxProvider::synthetic(
            RecordedBundle::open(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/kr-etf/contract"
            ))
            .unwrap(),
        );
        let request = IngestRequest::new(
            MARKET_KR.to_owned(),
            TradingDate::parse("2020-01-31").unwrap(),
            UtcTimestamp::parse_rfc3339("2026-08-10T00:00:00Z").unwrap(),
        );

        let error = ingest_bundle_with_store(&store, &provider, &request, None).unwrap_err();
        let error_batch_id = error.batch_id().unwrap();
        let entry = match error {
            IngestError::Readback { entry, source } => {
                assert!(matches!(source, StoreError::Io { .. }));
                *entry
            }
            other => panic!("expected typed readback error, got {other:?}"),
        };
        assert_eq!(entry.provider, PROVIDER_KRX);
        assert_eq!(entry.batch_id, error_batch_id);
        assert_eq!(
            store.raw.read_manifest(PROVIDER_KRX, MARKET_KR).unwrap(),
            vec![entry.clone()]
        );
        assert_eq!(
            store
                .raw
                .read_batch_bytes(PROVIDER_KRX, MARKET_KR, &entry)
                .unwrap()
                .len(),
            4
        );
    }
}
