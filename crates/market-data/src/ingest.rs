//! The collector pipeline: fetch -> validate -> store -> manifest (Todo 8).
//!
//! [`ingest_bundle`] drives one delivery end-to-end: the provider returns raw
//! envelopes, the pipeline validates their structure, persists every byte
//! unchanged into a fresh immutable batch, and appends one manifest row. Any
//! failure is a typed [`IngestError`]. A failed pre-visible cleanup can leave a
//! non-committed directory that recovery ignores; once final metadata is visible,
//! any failure leaves an exact-identity batch for [`RawStore::read_manifest`]
//! to re-sync before recovery.

use std::collections::BTreeSet;

use domain::{BatchId, TradingDate, UtcTimestamp};

use crate::contract::{ResponseKind, StoredFile};
use crate::provider::{EodProvider, ProviderError};
use crate::providers::kis::{KisProvider, KisRead, validate_kis_response};
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
    MalformedResponse { kind: ResponseKind, reason: String },
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

impl std::fmt::Display for IngestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Provider(e) => write!(f, "ingest provider failure: {e}"),
            Self::MalformedResponse { kind, reason } => {
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
        validate_kis_response(envelope.kind, &envelope.request.endpoint, &envelope.bytes)?;
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
