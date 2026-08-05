//! The collector pipeline: fetch -> validate -> store -> manifest (Todo 8).
//!
//! [`ingest_bundle`] drives one delivery end-to-end: the provider returns raw
//! envelopes, the pipeline validates their structure, persists every byte
//! unchanged into a fresh immutable batch, and appends one manifest row. Any
//! failure is a typed [`IngestError`] and leaves no partial output.

use domain::{BatchId, TradingDate, UtcTimestamp};

use crate::contract::{ResponseKind, StoredFile};
use crate::provider::{EodProvider, ProviderError};
use crate::storage::{BatchSpec, ManifestEntry, RawStore, StoreError};
use crate::validate::{ValidationError, validate_response};

/// A typed failure of the whole ingestion pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestError {
    /// The provider failed (timeout, credentials, unsafe file name, ...).
    Provider(ProviderError),
    /// The response bytes failed structural schema validation.
    MalformedResponse { kind: ResponseKind, reason: String },
    /// The immutable store rejected the batch.
    Store(StoreError),
}

impl std::fmt::Display for IngestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Provider(e) => write!(f, "ingest provider failure: {e}"),
            Self::MalformedResponse { kind, reason } => {
                write!(f, "malformed {kind} response: {reason}")
            }
            Self::Store(e) => write!(f, "ingest store failure: {e}"),
        }
    }
}

impl std::error::Error for IngestError {}

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

/// Runs one delivery: fetch all licensed response classes, validate, persist
/// as a new immutable batch, append the manifest row.
///
/// `entitlements` supplies the Todo 5 gate snapshot: when the governing
/// entitlement is ACTIVE on the batch date its contract reference is recorded
/// on the manifest row (`None` otherwise).
pub fn ingest_bundle(
    store: &RawStore,
    provider: &dyn EodProvider,
    req: &IngestRequest,
    entitlements: Option<&crate::entitlement::EntitlementService>,
) -> Result<IngestOutcome, IngestError> {
    let batch_id = BatchId::generate();
    let fetch_req = crate::provider::FetchRequest {
        market: req.market.clone(),
        date: req.date,
        kinds: crate::contract::ALL_RESPONSE_KINDS.to_vec(),
        now: req.now,
        batch_id,
    };
    let envelopes = provider.fetch(&fetch_req)?;

    for env in &envelopes {
        validate_response(env.kind, &env.bytes)?;
    }

    let entitlement_reference = entitlements
        .and_then(|service| crate::entitlement::governing_entitlement_reference(service, req.date));

    let spec = BatchSpec {
        provider: provider.provider_id(),
        market: &req.market,
        date: &req.date,
        batch_id,
        entitlement_reference: entitlement_reference.as_deref(),
        mode: provider.fetch_mode(),
    };
    let entry = store.store_batch(&spec, &envelopes)?;

    let files = envelopes
        .iter()
        .map(|e| StoredFile {
            file_name: e.file_name.clone(),
            bytes: e.bytes.clone(),
        })
        .collect();

    Ok(IngestOutcome {
        batch_id,
        entry,
        files,
    })
}
