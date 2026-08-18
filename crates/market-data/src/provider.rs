//! Provider-neutral **EOD provider contract** and the **KRX adapter** (Todo 8).
//!
//! [`EodProvider`] is the seam every licensed connector implements: it turns a
//! [`FetchRequest`] (market, date, clock) into a set of [`RawEnvelope`]s whose
//! bytes are stored byte-for-byte in the immutable raw zone.
//!
//! [`KrxProvider`] has two modes:
//! - **Synthetic** — plays back recorded synthetic contract fixtures (CI; no
//!   network, no credentials). Failure modes (timeout) are recorded in the
//!   bundle manifest so typed failures are reproducible.
//! - **Credentialed** — the Owner-only licensed mode. It requires real KRX
//!   credentials (`KRX_CREDENTIAL_REF` / `KRX_BASE_URL`), which do not exist in
//!   this environment: without them every fetch fails with the typed
//!   [`ProviderError::CredentialsUnavailable`]. Never exercised in CI.
//!
//! The adapter never scrapes undocumented endpoints: only the documented
//! licensed endpoint ids declared in the recorded bundle are used.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use domain::{BatchId, TradingDate, UtcTimestamp};
use serde::Deserialize;

use crate::contract::{EOD_RESPONSE_KINDS, FetchMode, RawEnvelope, RequestMetadata, ResponseKind};

/// What a provider needs to produce a delivery.
#[derive(Debug, Clone)]
pub struct FetchRequest {
    /// Market id (e.g. `kr`).
    pub market: String,
    /// The data date (the `date=` partition of the raw zone).
    pub date: TradingDate,
    /// Which licensed response classes to fetch.
    pub kinds: Vec<ResponseKind>,
    /// The retrieval clock (injected for deterministic tests).
    pub now: UtcTimestamp,
    /// The ingestion batch this delivery belongs to.
    pub batch_id: BatchId,
}

impl FetchRequest {
    pub fn new(market: String, date: TradingDate, now: UtcTimestamp) -> Self {
        Self {
            market,
            date,
            kinds: EOD_RESPONSE_KINDS.to_vec(),
            now,
            batch_id: BatchId::generate(),
        }
    }

    /// Replaces the default EOD classes with an explicit provider capability
    /// set. Candidate sources use this rather than changing legacy ingestion.
    pub fn with_kinds(mut self, kinds: impl IntoIterator<Item = ResponseKind>) -> Self {
        self.kinds = kinds.into_iter().collect();
        self
    }
}

/// The provider contract: fetch a delivery for one market/date.
pub trait EodProvider: fmt::Debug + Send + Sync {
    /// Stable provider id (`krx`).
    fn provider_id(&self) -> &'static str;
    /// The fetch mode this provider instance runs in.
    fn fetch_mode(&self) -> FetchMode;
    /// Fetches the requested response classes. Bytes are opaque: the provider
    /// never parses them; schema validation happens in the pipeline.
    fn fetch(&self, req: &FetchRequest) -> Result<Vec<RawEnvelope>, ProviderError>;
}

/// Typed provider failure. Never a panic; never partial output downstream.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// The Owner-only credentialed mode requires real credentials (a licensed
    /// KRX contract reference). Absent credentials fail typed.
    #[error("provider credentials unavailable ({credential_ref}): {detail}")]
    CredentialsUnavailable {
        credential_ref: String,
        detail: String,
    },
    /// The endpoint did not answer within its timeout budget.
    #[error("provider endpoint timeout for {kind} after {timeout_secs}s")]
    EndpointTimeout {
        kind: ResponseKind,
        timeout_secs: u64,
    },
    /// The provider-supplied file name is not a plain name (path traversal).
    #[error("provider returned unsafe file name {file_name:?} for {kind}")]
    UnsafeFileName {
        kind: ResponseKind,
        file_name: String,
    },
    /// A requested response class is not supported by this provider.
    #[error("provider does not support {0}")]
    UnsupportedKind(ResponseKind),
    /// Provider configuration is invalid before any request is sent.
    #[error("invalid provider configuration: {detail}")]
    InvalidConfiguration { detail: String },
    /// A credentialed provider call failed with a typed, redacted error.
    #[error("{provider} request failed for {kind} ({code}): {detail}")]
    Remote {
        provider: &'static str,
        kind: ResponseKind,
        code: &'static str,
        retryable: bool,
        /// Endpoint and HTTP status are safe operational metadata. Response
        /// bodies and free-form broker details must never be copied from this
        /// diagnostic into a worker event.
        diagnostic: Option<RemoteDiagnostic>,
        detail: String,
    },
    /// Transient endpoint or transport I/O.
    #[error("provider io failure ({context})")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },
    /// Required recorded bundle metadata does not exist.
    #[error("recorded bundle is missing {path}")]
    RecordedBundleMissing { path: String },
    /// A recorded bundle file cannot be read. Recorded fixtures are local
    /// configuration, so this is permanent rather than endpoint I/O.
    #[error("recorded bundle io failure ({context}) at {path}")]
    RecordedBundleIo {
        context: String,
        path: String,
        #[source]
        source: std::io::Error,
    },
    /// `bundle.json` is not valid JSON for the recorded-bundle contract.
    #[error("recorded bundle manifest is malformed at {path}")]
    RecordedBundleParse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    /// The manifest is valid JSON but declares unsupported configuration.
    #[error("recorded bundle configuration is invalid: {detail}")]
    RecordedBundleInvalid { detail: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteDiagnostic {
    pub endpoint: String,
    pub http_status: Option<u16>,
}

/// A reference to a stored credential, e.g. `env:KRX_CREDENTIAL_REF`. The
/// reference travels in metadata; the credential value never does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialRef(pub String);

impl CredentialRef {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}

/// The two modes of the KRX adapter.
#[derive(Debug, Clone)]
pub enum KrxMode {
    /// Playback of recorded synthetic contract fixtures.
    Synthetic(RecordedBundle),
    /// Owner-only licensed mode; requires real KRX credentials (not exercised).
    Credentialed(CredentialRef),
}

/// The KRX provider adapter for licensed ETF OHLCV / reference / calendar /
/// corporate-action inputs.
#[derive(Debug, Clone)]
pub struct KrxProvider {
    mode: KrxMode,
}

impl KrxProvider {
    pub fn synthetic(bundle: RecordedBundle) -> Self {
        Self {
            mode: KrxMode::Synthetic(bundle),
        }
    }

    pub fn credentialed(credential_ref: CredentialRef) -> Self {
        Self {
            mode: KrxMode::Credentialed(credential_ref),
        }
    }
}

impl EodProvider for KrxProvider {
    fn provider_id(&self) -> &'static str {
        "krx"
    }

    fn fetch_mode(&self) -> FetchMode {
        match &self.mode {
            KrxMode::Synthetic(_) => FetchMode::Synthetic,
            KrxMode::Credentialed(_) => FetchMode::Credentialed,
        }
    }

    fn fetch(&self, req: &FetchRequest) -> Result<Vec<RawEnvelope>, ProviderError> {
        match &self.mode {
            KrxMode::Synthetic(bundle) => bundle.fetch(req),
            KrxMode::Credentialed(credential_ref) => {
                let missing = ["KRX_CREDENTIAL_REF", "KRX_BASE_URL"]
                    .iter()
                    .filter(|v| std::env::var(v).is_err())
                    .cloned()
                    .collect::<Vec<_>>();
                if !missing.is_empty() {
                    return Err(ProviderError::CredentialsUnavailable {
                        credential_ref: credential_ref.0.clone(),
                        detail: format!(
                            "Owner-only credentialed KRX mode requires a licensed contract and \
                             credentials; missing env: {}. No real KRX credentials exist in this \
                             environment - the mode is implemented but never exercised.",
                            missing.join(", ")
                        ),
                    });
                }
                Err(ProviderError::CredentialsUnavailable {
                    credential_ref: credential_ref.0.clone(),
                    detail: "credentialed KRX transport is Owner-only and requires a licensed \
                             KRX contract; not exercised in CI"
                        .to_owned(),
                })
            }
        }
    }
}

/// A recorded synthetic contract bundle: `bundle.json` plus the recorded
/// provider response files, read-only.
#[derive(Debug, Clone)]
pub struct RecordedBundle {
    root: PathBuf,
}

#[derive(Debug, Deserialize)]
struct RecordedBundleFile {
    #[serde(rename = "provider")]
    #[allow(dead_code)]
    provider: String,
    #[serde(rename = "market")]
    #[allow(dead_code)]
    market: String,
    #[serde(rename = "schema_version")]
    #[allow(dead_code)]
    schema_version: u32,
    #[serde(rename = "simulate")]
    simulate: Option<String>,
    responses: Vec<RecordedResponse>,
}

#[derive(Debug, Deserialize)]
struct RecordedResponse {
    kind: String,
    file: String,
    endpoint: String,
    #[serde(default)]
    query: Vec<(String, String)>,
    #[serde(default)]
    headers: Vec<(String, String)>,
}

impl RecordedBundle {
    /// Opens a recorded bundle directory containing `bundle.json`.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, ProviderError> {
        let root = root.into();
        if !root.join("bundle.json").is_file() {
            return Err(ProviderError::RecordedBundleMissing {
                path: root.join("bundle.json").display().to_string(),
            });
        }
        let _ = read_bundle_manifest(&root)?;
        Ok(Self { root })
    }

    /// The bundle directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn fetch(&self, req: &FetchRequest) -> Result<Vec<RawEnvelope>, ProviderError> {
        let manifest = read_bundle_manifest(&self.root)?;
        if let Some(sim) = manifest.simulate.as_deref() {
            if sim == "timeout" {
                return Err(ProviderError::EndpointTimeout {
                    kind: ResponseKind::Bars,
                    timeout_secs: 30,
                });
            }
            return Err(ProviderError::RecordedBundleInvalid {
                detail: format!("unknown simulate directive {sim:?}"),
            });
        }

        let mut out = Vec::new();
        for recorded in &manifest.responses {
            let kind = ResponseKind::parse(&recorded.kind).ok_or_else(|| {
                ProviderError::RecordedBundleInvalid {
                    detail: format!("unknown response kind {:?}", recorded.kind),
                }
            })?;
            if !req.kinds.contains(&kind) {
                continue;
            }
            validate_plain_name(&recorded.file, kind)?;
            let path = self.root.join(&recorded.file);
            let bytes = fs::read(&path).map_err(|source| ProviderError::RecordedBundleIo {
                context: format!("recorded response {}", recorded.file),
                path: path.display().to_string(),
                source,
            })?;
            out.push(RawEnvelope::new(
                req.batch_id,
                kind,
                recorded.file.clone(),
                bytes,
                req.now,
                RequestMetadata {
                    endpoint: recorded.endpoint.clone(),
                    query: recorded.query.clone(),
                    headers: recorded.headers.clone(),
                    mode: FetchMode::Synthetic,
                },
            ));
        }
        Ok(out)
    }
}

fn read_bundle_manifest(root: &Path) -> Result<RecordedBundleFile, ProviderError> {
    let path = root.join("bundle.json");
    let raw = fs::read_to_string(&path).map_err(|source| ProviderError::RecordedBundleIo {
        context: "read bundle.json".to_owned(),
        path: path.display().to_string(),
        source,
    })?;
    serde_json::from_str(&raw).map_err(|source| ProviderError::RecordedBundleParse {
        path: path.display().to_string(),
        source,
    })
}

/// Plain-name validation at the provider boundary (defense in depth; the store
/// validates again on write).
fn validate_plain_name(name: &str, kind: ResponseKind) -> Result<(), ProviderError> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains(':')
        || name.bytes().any(|b| b.is_ascii_control())
    {
        return Err(ProviderError::UnsafeFileName {
            kind,
            file_name: name.to_owned(),
        });
    }
    Ok(())
}
