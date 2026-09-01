//! Owner-equity V2 capture and provider-free immutable-Raw verification.
//!
//! The only network-capable function accepts the existing `KisProvider`, so
//! authentication, token reuse, endpoint/TR rate channels, bounded retries,
//! and redacted request metadata remain owned by the reviewed provider seam.

use std::fs;

use domain::{
    BatchId, CodeCommit, ContentHash, OwnerEquityFailureCode, RetryDisposition, UtcTimestamp,
};
use market_data::owner_equity_v2::{
    OWNER_EQUITY_V2_MARKET, OWNER_EQUITY_V2_PROVIDER_SCOPE, OwnerEquityCaptureIdentity,
    OwnerEquityGenerationCandidate, OwnerEquityRawEvidence, OwnerEquityRawFile,
    materialize_owner_equity_candidate, materialize_owner_equity_candidate_allow_insufficient,
    validate_owner_equity_raw_evidence, verify_owner_equity_candidate,
};
use market_data::provider::{FetchRequest, ProviderError};
use market_data::providers::kis::{KisProvider, KisRead};
use market_data::storage::{BatchSpec, ManifestEntry, RawStore, StoreError};
use market_data::{FetchMode, RawEnvelope, ResponseKind};
use thiserror::Error;

pub mod artifact;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerEquityCaptureOutcome {
    pub batch_id: BatchId,
    pub entry: ManifestEntry,
    pub planned_gets: usize,
    pub actual_gets: usize,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerEquityMaterializeOutcome {
    pub candidate: OwnerEquityGenerationCandidate,
    pub canonical_bytes: Vec<u8>,
    pub content_sha256: ContentHash,
}

/// Sequentially captures one exact reference response and one exact daily
/// response per preplanned window.  No Raw path is touched until every wire
/// response and all cross-window invariants have passed validation.
pub async fn capture_owner_equity_raw<R: KisRead>(
    store: &RawStore,
    provider: &KisProvider<R>,
    identity: &OwnerEquityCaptureIdentity,
    retrieved_at: UtcTimestamp,
) -> Result<OwnerEquityCaptureOutcome, OwnerEquityCollectorError> {
    identity
        .validate()
        .map_err(OwnerEquityCollectorError::Engine)?;
    let batch_id = identity
        .batch_id()
        .map_err(OwnerEquityCollectorError::Engine)?;
    if let Some(entry) = find_existing_entry(store, identity, batch_id)? {
        load_owner_equity_raw_evidence(store, identity, &entry)?;
        return Ok(OwnerEquityCaptureOutcome {
            batch_id,
            entry,
            planned_gets: identity.plan.exact_get_ceiling,
            actual_gets: 0,
            replayed: true,
        });
    }

    let request = FetchRequest {
        market: OWNER_EQUITY_V2_MARKET.to_owned(),
        date: identity.plan.requested_end,
        kinds: vec![ResponseKind::Reference],
        now: retrieved_at,
        batch_id,
    };
    let mut envelopes = provider
        .fetch(&request)
        .await
        .map_err(OwnerEquityCollectorError::from_provider)?;
    if envelopes.len() != 1 || envelopes[0].kind != ResponseKind::Reference {
        return Err(OwnerEquityCollectorError::EvidenceShape);
    }
    let mut actual_gets = 1usize;
    for window in &identity.plan.windows {
        let mut daily = provider
            .fetch_daily_bars_range(
                OWNER_EQUITY_V2_MARKET,
                window.start,
                window.end,
                retrieved_at,
                batch_id,
            )
            .await
            .map_err(OwnerEquityCollectorError::from_provider)?;
        if daily.len() != 1 || daily[0].kind != ResponseKind::Bars {
            return Err(OwnerEquityCollectorError::EvidenceShape);
        }
        actual_gets = actual_gets
            .checked_add(1)
            .ok_or(OwnerEquityCollectorError::RequestBudget)?;
        if actual_gets > identity.plan.exact_get_ceiling {
            return Err(OwnerEquityCollectorError::RequestBudget);
        }
        daily[0].file_name = format!(
            "daily-bars-window-{:04}-{}-page-01.json",
            window.sequence,
            identity.plan.instrument_id.symbol()
        );
        envelopes.extend(daily);
    }
    if actual_gets != identity.plan.exact_get_ceiling
        || envelopes.len() != identity.plan.exact_get_ceiling
    {
        return Err(OwnerEquityCollectorError::EvidenceShape);
    }

    let pending = evidence_from_envelopes(identity, &envelopes)?;
    validate_owner_equity_raw_evidence(identity, &pending)
        .map_err(OwnerEquityCollectorError::Engine)?;

    let spec = BatchSpec {
        provider: OWNER_EQUITY_V2_PROVIDER_SCOPE,
        market: OWNER_EQUITY_V2_MARKET,
        date: &identity.plan.requested_start,
        batch_id,
        entitlement_reference: Some(&identity.entitlement_reference),
        mode: FetchMode::Credentialed,
    };
    let entry = match store.store_batch(&spec, &envelopes) {
        Ok(entry) => entry,
        Err(StoreError::FileExists { .. }) => find_existing_entry(store, identity, batch_id)?
            .ok_or(OwnerEquityCollectorError::RawStore)?,
        Err(_) => return Err(OwnerEquityCollectorError::RawStore),
    };
    load_owner_equity_raw_evidence(store, identity, &entry)?;
    Ok(OwnerEquityCaptureOutcome {
        batch_id,
        entry,
        planned_gets: identity.plan.exact_get_ceiling,
        actual_gets,
        replayed: false,
    })
}

pub fn materialize_owner_equity_from_raw(
    store: &RawStore,
    identity: &OwnerEquityCaptureIdentity,
    materializer_code_commit: CodeCommit,
) -> Result<OwnerEquityMaterializeOutcome, OwnerEquityCollectorError> {
    let batch_id = identity
        .batch_id()
        .map_err(OwnerEquityCollectorError::Engine)?;
    let entry = find_existing_entry(store, identity, batch_id)?
        .ok_or(OwnerEquityCollectorError::RawMissing)?;
    let evidence = load_owner_equity_raw_evidence(store, identity, &entry)?;
    let candidate =
        materialize_owner_equity_candidate(identity, &evidence, materializer_code_commit)
            .map_err(OwnerEquityCollectorError::Engine)?;
    let canonical_bytes = candidate
        .canonical_bytes()
        .map_err(OwnerEquityCollectorError::Engine)?;
    let content_sha256 = ContentHash::from_bytes(&canonical_bytes);
    Ok(OwnerEquityMaterializeOutcome {
        candidate,
        canonical_bytes,
        content_sha256,
    })
}

pub fn materialize_owner_equity_from_raw_allow_insufficient(
    store: &RawStore,
    identity: &OwnerEquityCaptureIdentity,
    materializer_code_commit: CodeCommit,
) -> Result<OwnerEquityMaterializeOutcome, OwnerEquityCollectorError> {
    let batch_id = identity
        .batch_id()
        .map_err(OwnerEquityCollectorError::Engine)?;
    let entry = find_existing_entry(store, identity, batch_id)?
        .ok_or(OwnerEquityCollectorError::RawMissing)?;
    let evidence = load_owner_equity_raw_evidence(store, identity, &entry)?;
    let candidate = materialize_owner_equity_candidate_allow_insufficient(
        identity,
        &evidence,
        materializer_code_commit,
    )
    .map_err(OwnerEquityCollectorError::Engine)?;
    let canonical_bytes = candidate
        .canonical_bytes()
        .map_err(OwnerEquityCollectorError::Engine)?;
    let content_sha256 = ContentHash::from_bytes(&canonical_bytes);
    Ok(OwnerEquityMaterializeOutcome {
        candidate,
        canonical_bytes,
        content_sha256,
    })
}

pub fn check_owner_equity_from_raw(
    store: &RawStore,
    identity: &OwnerEquityCaptureIdentity,
    materializer_code_commit: CodeCommit,
    expected_candidate_bytes: &[u8],
    expected_candidate_sha256: &ContentHash,
) -> Result<OwnerEquityGenerationCandidate, OwnerEquityCollectorError> {
    let batch_id = identity
        .batch_id()
        .map_err(OwnerEquityCollectorError::Engine)?;
    let entry = find_existing_entry(store, identity, batch_id)?
        .ok_or(OwnerEquityCollectorError::RawMissing)?;
    let evidence = load_owner_equity_raw_evidence(store, identity, &entry)?;
    verify_owner_equity_candidate(
        identity,
        &evidence,
        materializer_code_commit,
        expected_candidate_bytes,
        expected_candidate_sha256,
    )
    .map_err(OwnerEquityCollectorError::Engine)
}

pub fn load_owner_equity_raw_evidence(
    store: &RawStore,
    identity: &OwnerEquityCaptureIdentity,
    entry: &ManifestEntry,
) -> Result<OwnerEquityRawEvidence, OwnerEquityCollectorError> {
    validate_entry(identity, entry)?;
    let stored = store
        .read_batch_bytes(
            OWNER_EQUITY_V2_PROVIDER_SCOPE,
            OWNER_EQUITY_V2_MARKET,
            entry,
        )
        .map_err(|_| OwnerEquityCollectorError::RawStore)?;
    if stored.len() != entry.files.len() {
        return Err(OwnerEquityCollectorError::RawMissing);
    }
    let batch_path = store
        .batch_dir(
            OWNER_EQUITY_V2_PROVIDER_SCOPE,
            OWNER_EQUITY_V2_MARKET,
            &entry.date,
            &entry.batch_id,
        )
        .join(entry.batch_json_file_name());
    require_safe_metadata_file(&batch_path)?;
    let batch_bytes = fs::read(&batch_path).map_err(|_| OwnerEquityCollectorError::RawStore)?;
    let parsed: ManifestEntry =
        serde_json::from_slice(&batch_bytes).map_err(|_| OwnerEquityCollectorError::RawTamper)?;
    if &parsed != entry {
        return Err(OwnerEquityCollectorError::RawTamper);
    }
    let manifest_path = store.manifest_path(OWNER_EQUITY_V2_PROVIDER_SCOPE, OWNER_EQUITY_V2_MARKET);
    require_safe_metadata_file(&manifest_path)?;
    let manifest_bytes =
        fs::read(&manifest_path).map_err(|_| OwnerEquityCollectorError::RawStore)?;
    let matching_lines = manifest_bytes
        .split_inclusive(|byte| *byte == b'\n')
        .filter(|line| serde_json::from_slice::<ManifestEntry>(line).ok().as_ref() == Some(entry))
        .collect::<Vec<_>>();
    if matching_lines.len() != 1 {
        return Err(OwnerEquityCollectorError::RawTamper);
    }
    let files = entry
        .files
        .iter()
        .zip(stored)
        .map(|(metadata, stored)| {
            if metadata.file_name != stored.file_name
                || metadata.size_bytes != stored.bytes.len() as u64
                || metadata.content_hash != ContentHash::from_bytes(&stored.bytes)
            {
                return Err(OwnerEquityCollectorError::RawTamper);
            }
            Ok(OwnerEquityRawFile {
                kind: metadata.kind,
                file_name: metadata.file_name.clone(),
                content_hash: metadata.content_hash.clone(),
                request: metadata.request.clone(),
                response_continuation: metadata.response_continuation.clone(),
                bytes: stored.bytes,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let evidence = OwnerEquityRawEvidence {
        batch_id: entry.batch_id,
        raw_manifest_sha256: ContentHash::from_bytes(matching_lines[0]),
        batch_json_sha256: ContentHash::from_bytes(&batch_bytes),
        files,
    };
    validate_owner_equity_raw_evidence(identity, &evidence)
        .map_err(OwnerEquityCollectorError::Engine)?;
    Ok(evidence)
}

fn evidence_from_envelopes(
    identity: &OwnerEquityCaptureIdentity,
    envelopes: &[RawEnvelope],
) -> Result<OwnerEquityRawEvidence, OwnerEquityCollectorError> {
    let batch_id = identity
        .batch_id()
        .map_err(OwnerEquityCollectorError::Engine)?;
    let files = envelopes
        .iter()
        .map(|envelope| {
            if envelope.batch_id != batch_id {
                return Err(OwnerEquityCollectorError::EvidenceShape);
            }
            Ok(OwnerEquityRawFile {
                kind: envelope.kind,
                file_name: envelope.file_name.clone(),
                content_hash: envelope.content_hash.clone(),
                request: envelope.request.clone(),
                response_continuation: envelope.response_continuation.clone(),
                bytes: envelope.bytes.clone(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(OwnerEquityRawEvidence {
        batch_id,
        raw_manifest_sha256: ContentHash::from_bytes(b"not-visible-before-commit"),
        batch_json_sha256: ContentHash::from_bytes(b"not-visible-before-commit"),
        files,
    })
}

fn find_existing_entry(
    store: &RawStore,
    identity: &OwnerEquityCaptureIdentity,
    batch_id: BatchId,
) -> Result<Option<ManifestEntry>, OwnerEquityCollectorError> {
    if !store
        .manifest_path(OWNER_EQUITY_V2_PROVIDER_SCOPE, OWNER_EQUITY_V2_MARKET)
        .exists()
    {
        return Ok(None);
    }
    let entries = store
        .read_committed_manifest(OWNER_EQUITY_V2_PROVIDER_SCOPE, OWNER_EQUITY_V2_MARKET)
        .map_err(|_| OwnerEquityCollectorError::RawStore)?;
    let matches = entries
        .into_iter()
        .filter(|entry| entry.batch_id == batch_id)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Ok(None),
        [entry] => {
            validate_entry(identity, entry)?;
            Ok(Some(entry.clone()))
        }
        _ => Err(OwnerEquityCollectorError::RawTamper),
    }
}

fn validate_entry(
    identity: &OwnerEquityCaptureIdentity,
    entry: &ManifestEntry,
) -> Result<(), OwnerEquityCollectorError> {
    if entry.batch_id
        != identity
            .batch_id()
            .map_err(OwnerEquityCollectorError::Engine)?
        || entry.provider != OWNER_EQUITY_V2_PROVIDER_SCOPE
        || entry.market != OWNER_EQUITY_V2_MARKET
        || entry.date != identity.plan.requested_start
        || entry.mode != FetchMode::Credentialed
        || entry.entitlement_reference.as_deref() != Some(&identity.entitlement_reference)
        || entry.files.len() != identity.plan.exact_get_ceiling
    {
        return Err(OwnerEquityCollectorError::RawTamper);
    }
    Ok(())
}

fn require_safe_metadata_file(path: &std::path::Path) -> Result<(), OwnerEquityCollectorError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| OwnerEquityCollectorError::RawMissing)?;
    if !metadata.file_type().is_file() {
        return Err(OwnerEquityCollectorError::RawTamper);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum OwnerEquityCollectorError {
    #[error("owner equity engine rejected the input")]
    Engine(market_data::owner_equity_v2::OwnerEquityV2Error),
    #[error("owner equity provider request failed")]
    Provider {
        code: &'static str,
        retry: RetryDisposition,
    },
    #[error("owner equity provider evidence shape is invalid")]
    EvidenceShape,
    #[error("owner equity request budget was exceeded")]
    RequestBudget,
    #[error("owner equity immutable Raw store failed")]
    RawStore,
    #[error("owner equity immutable Raw evidence is missing")]
    RawMissing,
    #[error("owner equity immutable Raw evidence was tampered")]
    RawTamper,
}

impl OwnerEquityCollectorError {
    fn from_provider(error: ProviderError) -> Self {
        match error {
            ProviderError::Remote {
                code, retryable, ..
            } => Self::Provider {
                code: bounded_provider_code(code),
                retry: if retryable {
                    RetryDisposition::Retryable
                } else {
                    RetryDisposition::Terminal
                },
            },
            ProviderError::EndpointTimeout { .. } | ProviderError::Io { .. } => Self::Provider {
                code: "PROVIDER_RETRYABLE",
                retry: RetryDisposition::Retryable,
            },
            ProviderError::CredentialsUnavailable { .. } => Self::Provider {
                code: "PROVIDER_CREDENTIALS_UNAVAILABLE",
                retry: RetryDisposition::Terminal,
            },
            _ => Self::Provider {
                code: "PROVIDER_FAILURE",
                retry: RetryDisposition::Terminal,
            },
        }
    }

    pub const fn code(&self) -> &'static str {
        match self {
            Self::Engine(error) => error.code(),
            Self::Provider { code, .. } => code,
            Self::EvidenceShape => "EVIDENCE_SHAPE_INVALID",
            Self::RequestBudget => "REQUEST_BUDGET_EXCEEDED",
            Self::RawStore => "RAW_STORE_FAILURE",
            Self::RawMissing => "RAW_EVIDENCE_MISSING",
            Self::RawTamper => "RAW_TAMPERED",
        }
    }

    pub fn failure_code(&self) -> OwnerEquityFailureCode {
        OwnerEquityFailureCode::parse(self.code()).expect("bounded static collector code is valid")
    }

    pub const fn retry_disposition(&self) -> RetryDisposition {
        match self {
            Self::Provider { retry, .. } => *retry,
            Self::RawStore => RetryDisposition::Retryable,
            Self::Engine(error) => error.retry_disposition(),
            Self::EvidenceShape | Self::RequestBudget | Self::RawMissing | Self::RawTamper => {
                RetryDisposition::Terminal
            }
        }
    }
}

fn bounded_provider_code(code: &'static str) -> &'static str {
    if OwnerEquityFailureCode::parse(code).is_ok() {
        code
    } else {
        "PROVIDER_FAILURE"
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use domain::{OwnerEquityUniversePolicy, TradingDate};
    use kis_client::{KisError, MarketDataReply};
    use market_data::owner_equity_v2::{
        DAILY_BARS_PATH, DAILY_BARS_TR_ID, OwnerEquityCaptureKind, OwnerEquityCapturePlan,
        REFERENCE_PATH, REFERENCE_TR_ID,
    };
    use serde_json::json;
    use tempfile::tempdir;

    use super::*;

    const CAPTURE_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
    const MATERIALIZER_COMMIT: &str = "89abcdef0123456789abcdef0123456789abcdef";
    const SECRET_SENTINEL: &str = "sentinel-live-secret-value";

    #[derive(Debug, Clone)]
    struct FixtureCall {
        path: String,
        tr_id: String,
        query: Vec<(String, String)>,
        continuation: Option<String>,
    }

    #[derive(Debug, Clone)]
    struct FixtureReader {
        calls: Arc<Mutex<Vec<FixtureCall>>>,
        credential_that_must_not_escape: String,
    }

    impl FixtureReader {
        fn new() -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
                credential_that_must_not_escape: SECRET_SENTINEL.to_owned(),
            }
        }
    }

    impl KisRead for FixtureReader {
        async fn get(
            &self,
            path: &str,
            tr_id: &str,
            query: &[(String, String)],
            continuation: Option<&str>,
        ) -> Result<MarketDataReply, KisError> {
            assert_eq!(self.credential_that_must_not_escape, SECRET_SENTINEL);
            self.calls.lock().unwrap().push(FixtureCall {
                path: path.to_owned(),
                tr_id: tr_id.to_owned(),
                query: query.to_vec(),
                continuation: continuation.map(str::to_owned),
            });
            let symbol = query_value(query, "FID_INPUT_ISCD");
            let body = if path == REFERENCE_PATH && tr_id == REFERENCE_TR_ID {
                serde_json::to_vec(&json!({
                    "rt_cd": "0",
                    "output": {"stck_shrn_iscd": symbol}
                }))
                .unwrap()
            } else {
                assert_eq!((path, tr_id), (DAILY_BARS_PATH, DAILY_BARS_TR_ID));
                let start = parse_kis_query_date(query_value(query, "FID_INPUT_DATE_1"));
                let end = parse_kis_query_date(query_value(query, "FID_INPUT_DATE_2"));
                let mut dates = Vec::new();
                let mut date = start;
                while date <= end {
                    dates.push(date);
                    date = date.checked_add_days(1).unwrap();
                }
                dates.reverse();
                let rows = dates
                    .into_iter()
                    .map(|date| {
                        json!({
                            "stck_bsop_date": date.to_iso().replace('-', ""),
                            "stck_oprc": "100",
                            "stck_hgpr": "105",
                            "stck_lwpr": "95",
                            "stck_clpr": "101",
                            "acml_vol": "1000"
                        })
                    })
                    .collect::<Vec<_>>();
                serde_json::to_vec(&json!({
                    "rt_cd": "0",
                    "output1": {
                        "stck_shrn_iscd": symbol,
                        "hts_kor_isnm": "삼성전자"
                    },
                    "output2": rows
                }))
                .unwrap()
            };
            Ok(MarketDataReply {
                body,
                continuation: None,
            })
        }
    }

    fn identity() -> OwnerEquityCaptureIdentity {
        let plan = OwnerEquityCapturePlan::build(
            "005930.KRX",
            OwnerEquityUniversePolicy::default(),
            OwnerEquityCaptureKind::Initial,
            TradingDate::parse("2026-04-01").unwrap(),
            TradingDate::parse("2026-08-31").unwrap(),
        )
        .unwrap();
        OwnerEquityCaptureIdentity::new(
            plan,
            "vault://entitlements/kis-owner-equity-v2",
            ContentHash::from_bytes(b"entitlement"),
            CodeCommit::parse(CAPTURE_COMMIT).unwrap(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn fixture_capture_commits_only_valid_exact_requests_and_replays_idempotently() {
        let root = tempdir().unwrap();
        let store = RawStore::new(root.path());
        let identity = identity();
        let reader = FixtureReader::new();
        let calls = Arc::clone(&reader.calls);
        let provider = KisProvider::new(reader, vec![identity.plan.instrument_id.clone()]).unwrap();
        let now = UtcTimestamp::parse_rfc3339("2026-08-31T08:00:00Z").unwrap();
        let first = capture_owner_equity_raw(&store, &provider, &identity, now)
            .await
            .unwrap();
        assert!(!first.replayed);
        assert_eq!(first.actual_gets, identity.plan.exact_get_ceiling);
        let call_count = calls.lock().unwrap().len();
        assert_eq!(call_count, identity.plan.exact_get_ceiling);
        for call in calls.lock().unwrap().iter() {
            assert!(call.continuation.is_none());
            assert_eq!(query_value(&call.query, "FID_INPUT_ISCD"), "005930");
            assert!(matches!(
                (call.path.as_str(), call.tr_id.as_str()),
                (REFERENCE_PATH, REFERENCE_TR_ID) | (DAILY_BARS_PATH, DAILY_BARS_TR_ID)
            ));
        }

        let second = capture_owner_equity_raw(&store, &provider, &identity, now)
            .await
            .unwrap();
        assert!(second.replayed);
        assert_eq!(second.actual_gets, 0);
        assert_eq!(calls.lock().unwrap().len(), call_count);

        let materialized = materialize_owner_equity_from_raw(
            &store,
            &identity,
            CodeCommit::parse(MATERIALIZER_COMMIT).unwrap(),
        )
        .unwrap();
        assert!(materialized.candidate.observed_sessions >= 121);
        let checked = check_owner_equity_from_raw(
            &store,
            &identity,
            CodeCommit::parse(MATERIALIZER_COMMIT).unwrap(),
            &materialized.canonical_bytes,
            &materialized.content_sha256,
        )
        .unwrap();
        assert_eq!(checked, materialized.candidate);
    }

    #[tokio::test]
    async fn secret_sentinel_never_enters_raw_metadata_candidate_or_error() {
        let root = tempdir().unwrap();
        let store = RawStore::new(root.path());
        let identity = identity();
        let reader = FixtureReader::new();
        let provider = KisProvider::new(reader, vec![identity.plan.instrument_id.clone()]).unwrap();
        capture_owner_equity_raw(
            &store,
            &provider,
            &identity,
            UtcTimestamp::parse_rfc3339("2026-08-31T08:00:00Z").unwrap(),
        )
        .await
        .unwrap();
        let materialized = materialize_owner_equity_from_raw(
            &store,
            &identity,
            CodeCommit::parse(MATERIALIZER_COMMIT).unwrap(),
        )
        .unwrap();
        let manifest =
            fs::read(store.manifest_path(OWNER_EQUITY_V2_PROVIDER_SCOPE, OWNER_EQUITY_V2_MARKET))
                .unwrap();
        let batch = fs::read(
            store
                .batch_dir(
                    OWNER_EQUITY_V2_PROVIDER_SCOPE,
                    OWNER_EQUITY_V2_MARKET,
                    &identity.plan.requested_start,
                    &identity.batch_id().unwrap(),
                )
                .join("batch.json"),
        )
        .unwrap();
        assert!(!String::from_utf8_lossy(&manifest).contains(SECRET_SENTINEL));
        assert!(!String::from_utf8_lossy(&batch).contains(SECRET_SENTINEL));
        assert!(!String::from_utf8_lossy(&materialized.canonical_bytes).contains(SECRET_SENTINEL));
        assert!(
            !OwnerEquityCollectorError::RawTamper
                .to_string()
                .contains(SECRET_SENTINEL)
        );
    }

    #[tokio::test]
    async fn malformed_fixture_never_becomes_visible() {
        #[derive(Debug)]
        struct Malformed;
        impl KisRead for Malformed {
            async fn get(
                &self,
                _path: &str,
                _tr_id: &str,
                _query: &[(String, String)],
                _continuation: Option<&str>,
            ) -> Result<MarketDataReply, KisError> {
                Ok(MarketDataReply {
                    body: b"{".to_vec(),
                    continuation: None,
                })
            }
        }
        let root = tempdir().unwrap();
        let store = RawStore::new(root.path());
        let identity = identity();
        let provider =
            KisProvider::new(Malformed, vec![identity.plan.instrument_id.clone()]).unwrap();
        let error = capture_owner_equity_raw(
            &store,
            &provider,
            &identity,
            UtcTimestamp::parse_rfc3339("2026-08-31T08:00:00Z").unwrap(),
        )
        .await
        .unwrap_err();
        assert_eq!(
            error.failure_code().as_str(),
            "BROKER_PAGINATION_UNSUPPORTED"
        );
        assert!(
            !store
                .manifest_path(OWNER_EQUITY_V2_PROVIDER_SCOPE, OWNER_EQUITY_V2_MARKET)
                .exists()
        );
    }

    #[tokio::test]
    async fn tampered_missing_and_orphan_raw_sources_fail_closed() {
        let (tamper_root, tamper_store, tamper_identity, tamper_entry) = captured_fixture().await;
        let tamper_path = tamper_store
            .batch_dir(
                OWNER_EQUITY_V2_PROVIDER_SCOPE,
                OWNER_EQUITY_V2_MARKET,
                &tamper_entry.date,
                &tamper_entry.batch_id,
            )
            .join(&tamper_entry.files[1].file_name);
        let mut bytes = fs::read(&tamper_path).unwrap();
        bytes.push(b' ');
        fs::write(&tamper_path, bytes).unwrap();
        assert!(matches!(
            materialize_owner_equity_from_raw(
                &tamper_store,
                &tamper_identity,
                CodeCommit::parse(MATERIALIZER_COMMIT).unwrap(),
            ),
            Err(OwnerEquityCollectorError::RawStore)
        ));
        drop(tamper_root);

        let (missing_root, missing_store, missing_identity, missing_entry) =
            captured_fixture().await;
        let missing_path = missing_store
            .batch_dir(
                OWNER_EQUITY_V2_PROVIDER_SCOPE,
                OWNER_EQUITY_V2_MARKET,
                &missing_entry.date,
                &missing_entry.batch_id,
            )
            .join(&missing_entry.files[1].file_name);
        fs::remove_file(missing_path).unwrap();
        assert!(matches!(
            materialize_owner_equity_from_raw(
                &missing_store,
                &missing_identity,
                CodeCommit::parse(MATERIALIZER_COMMIT).unwrap(),
            ),
            Err(OwnerEquityCollectorError::RawStore)
        ));
        drop(missing_root);

        let (orphan_root, orphan_store, orphan_identity, _) = captured_fixture().await;
        fs::write(
            orphan_store.manifest_path(OWNER_EQUITY_V2_PROVIDER_SCOPE, OWNER_EQUITY_V2_MARKET),
            b"",
        )
        .unwrap();
        assert!(matches!(
            materialize_owner_equity_from_raw(
                &orphan_store,
                &orphan_identity,
                CodeCommit::parse(MATERIALIZER_COMMIT).unwrap(),
            ),
            Err(OwnerEquityCollectorError::RawMissing)
        ));
        drop(orphan_root);
    }

    async fn captured_fixture() -> (
        tempfile::TempDir,
        RawStore,
        OwnerEquityCaptureIdentity,
        ManifestEntry,
    ) {
        let root = tempdir().unwrap();
        let store = RawStore::new(root.path());
        let identity = identity();
        let reader = FixtureReader::new();
        let provider = KisProvider::new(reader, vec![identity.plan.instrument_id.clone()]).unwrap();
        let outcome = capture_owner_equity_raw(
            &store,
            &provider,
            &identity,
            UtcTimestamp::parse_rfc3339("2026-08-31T08:00:00Z").unwrap(),
        )
        .await
        .unwrap();
        (root, store, identity, outcome.entry)
    }

    fn query_value<'a>(query: &'a [(String, String)], field: &str) -> &'a str {
        query
            .iter()
            .find(|(key, _)| key == field)
            .map(|(_, value)| value.as_str())
            .unwrap()
    }

    fn parse_kis_query_date(value: &str) -> TradingDate {
        TradingDate::parse(&format!("{}-{}-{}", &value[..4], &value[4..6], &value[6..])).unwrap()
    }
}
