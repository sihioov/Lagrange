//! KIS candidate-source adapter.
//!
//! The candidate vertical has a different raw contract from the fixed ETF
//! EOD path.  This module therefore has its own provider scope and never
//! returns a candidate response under `provider=kis`; doing so would make the
//! EOD recovery pass try to normalize it as bars/reference/calendar/actions.
//!
//! The two REST surfaces implemented here are the documented KIS
//! `investor-trade-by-stock-daily` and finance balance/income statement
//! endpoints.  KOSPI200/KOSDAQ150 membership and taxonomy/sector data come
//! from KIS master downloads rather than this JSON REST surface and are
//! intentionally reported as [`ProviderError::UnsupportedKind`] until their
//! binary download client and reviewed field mapping are wired.

use domain::{InstrumentId, Venue};
use kis_client::{KisError, RequestKind};

use crate::contract::{
    CANDIDATE_RESPONSE_KINDS, FetchMode, PROVIDER_KIS_CANDIDATE, RawEnvelope, RequestMetadata,
    ResponseKind,
};
use crate::provider::{FetchRequest, ProviderError};

/// KIS REST endpoint for per-stock daily investor flow.
pub const INVESTOR_FLOW_PATH: &str =
    "/uapi/domestic-stock/v1/quotations/investor-trade-by-stock-daily";
/// KIS transaction id for [`INVESTOR_FLOW_PATH`].
pub const INVESTOR_FLOW_TR_ID: &str = "FHPTJ04160001";
/// KIS REST endpoint for annual/quarterly balance-sheet observations.
pub const BALANCE_SHEET_PATH: &str = "/uapi/domestic-stock/v1/finance/balance-sheet";
/// KIS transaction id for [`BALANCE_SHEET_PATH`].
pub const BALANCE_SHEET_TR_ID: &str = "FHKST66430100";
/// KIS REST endpoint for annual/quarterly income-statement observations.
pub const INCOME_STATEMENT_PATH: &str = "/uapi/domestic-stock/v1/finance/income-statement";
/// KIS transaction id for [`INCOME_STATEMENT_PATH`].
pub const INCOME_STATEMENT_TR_ID: &str = "FHKST66430200";

/// Candidate classes that this adapter can fetch from the documented REST
/// API.  The response bytes remain provider wire bytes until candidate
/// normalization is explicitly requested.
pub const KIS_CANDIDATE_SUPPORTED_KINDS: [ResponseKind; 2] =
    [ResponseKind::InvestorFlow, ResponseKind::Fundamentals];

/// Candidate classes deliberately unsupported by this REST adapter.
///
/// Membership and sector are master-file capabilities; market-status needs a
/// reviewed mapping for all six canonical flags rather than a partial
/// `inquire-price` projection.  A caller requesting any of these receives a
/// permanent typed `UnsupportedKind` error.
pub const KIS_CANDIDATE_UNSUPPORTED_KINDS: [ResponseKind; 3] = [
    ResponseKind::MarketStatus,
    ResponseKind::IndexMembership,
    ResponseKind::SectorClassification,
];

const MAX_PAGES: usize = 32;

/// Async read seam shared with the production authenticated client and test
/// fixtures.  Re-exporting the trait from `providers::kis` keeps one token,
/// retry, rate-limit, and continuation implementation for both paths.
pub use super::kis::KisRead;

/// Credentialed provider for the REST-backed candidate source classes.
#[derive(Debug)]
pub struct KisCandidateProvider<R: KisRead> {
    reader: R,
    instruments: Vec<InstrumentId>,
}

impl<R: KisRead> KisCandidateProvider<R> {
    /// Builds a candidate provider for an explicit point-in-time instrument
    /// universe.  The caller must provide the KOSPI200/KOSDAQ150 master
    /// result; this adapter does not invent membership from a current quote.
    pub fn new(reader: R, instruments: Vec<InstrumentId>) -> Result<Self, ProviderError> {
        if instruments.is_empty() {
            return Err(ProviderError::InvalidConfiguration {
                detail: "KIS candidate provider requires at least one instrument".to_owned(),
            });
        }
        for instrument in &instruments {
            if instrument.venue() != Venue::Krx
                || instrument.symbol().len() != 6
                || !instrument
                    .symbol()
                    .bytes()
                    .all(|byte| byte.is_ascii_digit())
            {
                return Err(ProviderError::InvalidConfiguration {
                    detail: format!(
                        "KIS candidate data requires six-digit KRX instruments, got {instrument}"
                    ),
                });
            }
        }
        Ok(Self {
            reader,
            instruments,
        })
    }

    pub fn provider_id(&self) -> &'static str {
        PROVIDER_KIS_CANDIDATE
    }

    pub fn fetch_mode(&self) -> FetchMode {
        FetchMode::Credentialed
    }

    /// Fetches the requested candidate classes as immutable KIS wire bytes.
    ///
    /// This method refuses the unsupported classes before the first network
    /// call.  It also validates every successful response's `rt_cd` and
    /// endpoint-specific top-level shape before returning it to ingestion.
    pub async fn fetch(&self, req: &FetchRequest) -> Result<Vec<RawEnvelope>, ProviderError> {
        if req.market != "kr" {
            return Err(ProviderError::InvalidConfiguration {
                detail: format!(
                    "KIS candidate provider supports market kr, got {:?}",
                    req.market
                ),
            });
        }
        if req.kinds.is_empty()
            || req
                .kinds
                .iter()
                .any(|kind| !CANDIDATE_RESPONSE_KINDS.contains(kind))
        {
            return Err(ProviderError::InvalidConfiguration {
                detail: "candidate request must contain only nonempty candidate response kinds"
                    .to_owned(),
            });
        }
        for kind in &req.kinds {
            if !KIS_CANDIDATE_SUPPORTED_KINDS.contains(kind) {
                return Err(ProviderError::UnsupportedKind(*kind));
            }
        }

        let date = req.date.to_iso().replace('-', "");
        let mut envelopes = Vec::new();
        for kind in &req.kinds {
            match kind {
                ResponseKind::InvestorFlow => {
                    for instrument in &self.instruments {
                        self.fetch_pages(
                            req,
                            *kind,
                            "investor-flow",
                            instrument.symbol(),
                            INVESTOR_FLOW_PATH,
                            INVESTOR_FLOW_TR_ID,
                            vec![
                                ("FID_COND_MRKT_DIV_CODE".to_owned(), "J".to_owned()),
                                ("FID_INPUT_ISCD".to_owned(), instrument.symbol().to_owned()),
                                ("FID_INPUT_DATE_1".to_owned(), date.clone()),
                                ("FID_ORG_ADJ_PRC".to_owned(), String::new()),
                                ("FID_ETC_CLS_CODE".to_owned(), String::new()),
                            ],
                            &mut envelopes,
                        )
                        .await?;
                    }
                }
                ResponseKind::Fundamentals => {
                    for instrument in &self.instruments {
                        for (label, path, tr_id) in [
                            (
                                "fundamentals-balance",
                                BALANCE_SHEET_PATH,
                                BALANCE_SHEET_TR_ID,
                            ),
                            (
                                "fundamentals-income",
                                INCOME_STATEMENT_PATH,
                                INCOME_STATEMENT_TR_ID,
                            ),
                        ] {
                            self.fetch_pages(
                                req,
                                *kind,
                                label,
                                instrument.symbol(),
                                path,
                                tr_id,
                                vec![
                                    ("FID_DIV_CLS_CODE".to_owned(), "0".to_owned()),
                                    ("FID_COND_MRKT_DIV_CODE".to_owned(), "J".to_owned()),
                                    ("FID_INPUT_ISCD".to_owned(), instrument.symbol().to_owned()),
                                ],
                                &mut envelopes,
                            )
                            .await?;
                        }
                    }
                }
                // Checked above; keeping the arms explicit makes adding a
                // newly supported class require a reviewed mapping here.
                ResponseKind::MarketStatus
                | ResponseKind::IndexMembership
                | ResponseKind::SectorClassification => {
                    return Err(ProviderError::UnsupportedKind(*kind));
                }
                ResponseKind::Bars
                | ResponseKind::Reference
                | ResponseKind::Calendar
                | ResponseKind::CorporateActions
                | ResponseKind::CandidateMaster => {
                    return Err(ProviderError::UnsupportedKind(*kind));
                }
            }
        }
        Ok(envelopes)
    }

    #[allow(clippy::too_many_arguments)]
    async fn fetch_pages(
        &self,
        req: &FetchRequest,
        kind: ResponseKind,
        label: &str,
        symbol: &str,
        path: &str,
        tr_id: &str,
        query: Vec<(String, String)>,
        output: &mut Vec<RawEnvelope>,
    ) -> Result<(), ProviderError> {
        let mut continuation = None;
        for page in 1..=MAX_PAGES {
            let reply = self
                .reader
                .get(path, tr_id, &query, continuation.as_deref())
                .await
                .map_err(|error| remote_error(kind, error))?;
            validate_kis_candidate_response(kind, path, &reply.body).map_err(|reason| {
                ProviderError::Remote {
                    provider: PROVIDER_KIS_CANDIDATE,
                    kind,
                    code: "BROKER_SCHEMA_DRIFT",
                    retryable: false,
                    detail: reason,
                }
            })?;
            output.push(RawEnvelope::new(
                req.batch_id,
                kind,
                format!("{label}-{symbol}-page-{page:03}.json"),
                reply.body,
                req.now,
                RequestMetadata {
                    endpoint: path.to_owned(),
                    query: query.clone(),
                    headers: vec![
                        ("authorization".to_owned(), "[REDACTED]".to_owned()),
                        ("appkey".to_owned(), "[REDACTED]".to_owned()),
                        ("appsecret".to_owned(), "[REDACTED]".to_owned()),
                        ("tr_id".to_owned(), tr_id.to_owned()),
                    ],
                    mode: FetchMode::Credentialed,
                },
            ));
            if !reply
                .continuation
                .as_deref()
                .is_some_and(|value| matches!(value, "M" | "F"))
            {
                return Ok(());
            }
            continuation = Some("N".to_owned());
        }
        Err(ProviderError::Remote {
            provider: PROVIDER_KIS_CANDIDATE,
            kind,
            code: "BROKER_PAGINATION_LIMIT",
            retryable: false,
            detail: format!("{path} exceeded the {MAX_PAGES}-page safety limit"),
        })
    }
}

fn remote_error(kind: ResponseKind, error: KisError) -> ProviderError {
    ProviderError::Remote {
        provider: PROVIDER_KIS_CANDIDATE,
        kind,
        code: error.code(),
        retryable: error.is_retryable(RequestKind::Read),
        detail: error.to_string(),
    }
}

/// Endpoint-specific structural validation performed before immutable Raw
/// persistence.  Semantic field conversion is intentionally left to the
/// candidate normalizer so malformed evidence remains diagnosable without
/// ever becoming a curated observation.
pub fn validate_kis_candidate_response(
    kind: ResponseKind,
    endpoint: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let document: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|error| format!("not valid KIS JSON: {error}"))?;
    let object = document
        .as_object()
        .ok_or_else(|| "KIS response must be a JSON object".to_owned())?;
    if object.get("rt_cd").and_then(serde_json::Value::as_str) != Some("0") {
        return Err("KIS response rt_cd must equal 0".to_owned());
    }
    let valid = match (kind, endpoint) {
        (ResponseKind::InvestorFlow, INVESTOR_FLOW_PATH) => {
            is_object_or_array(object.get("output1")) && is_object_or_array(object.get("output2"))
        }
        (ResponseKind::Fundamentals, BALANCE_SHEET_PATH)
        | (ResponseKind::Fundamentals, INCOME_STATEMENT_PATH) => {
            is_object_or_array(object.get("output"))
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(format!(
            "unexpected KIS candidate response shape for endpoint {endpoint:?}"
        ))
    }
}

fn is_object_or_array(value: Option<&serde_json::Value>) -> bool {
    value.is_some_and(|value| value.is_object() || value.is_array())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use domain::{BatchId, TradingDate, UtcTimestamp};
    use kis_client::MarketDataReply;

    use super::*;
    use crate::candidate_normalize::normalize_kis_candidate_batch;
    use crate::ingest::{IngestRequest, ingest_kis_candidate_bundle_with_kinds};
    use crate::storage::RawStore;

    #[derive(Debug, Clone, Copy)]
    enum FixtureMode {
        Healthy,
        Malformed,
        RateLimited,
    }

    #[derive(Debug, Clone)]
    struct Call {
        path: String,
        continuation: Option<String>,
        query: Vec<(String, String)>,
    }

    #[derive(Debug, Clone)]
    struct FixtureReader {
        mode: FixtureMode,
        calls: Arc<Mutex<Vec<Call>>>,
    }

    impl FixtureReader {
        fn new(mode: FixtureMode) -> Self {
            Self {
                mode,
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn body(path: &str, continuation: Option<&str>) -> Vec<u8> {
            if path == INVESTOR_FLOW_PATH {
                let date = if continuation.is_some() {
                    "20260813"
                } else {
                    "20260814"
                };
                format!(
                    r#"{{"rt_cd":"0","output1":{{}},"output2":[{{"stck_bsop_date":"{date}","frgn_ntby_qty":"10","frgn_ntby_tr_pbmn":"1000","orgn_ntby_qty":"-4","orgn_ntby_tr_pbmn":"-500"}}]}}"#
                )
                .into_bytes()
            } else if path == BALANCE_SHEET_PATH {
                br#"{"rt_cd":"0","output":[{"stac_yymm":"202412","cras":"99.99","total_aset":"200","total_lblt":"80","total_cptl":"120"}]}"#
                    .to_vec()
            } else {
                br#"{"rt_cd":"0","output":[{"stac_yymm":"202412","sale_account":"300","bsop_prti":"40","thtr_ntin":"30"}]}"#
                    .to_vec()
            }
        }
    }

    impl KisRead for FixtureReader {
        async fn get(
            &self,
            path: &str,
            _tr_id: &str,
            query: &[(String, String)],
            continuation: Option<&str>,
        ) -> Result<MarketDataReply, KisError> {
            self.calls.lock().expect("calls lock").push(Call {
                path: path.to_owned(),
                continuation: continuation.map(str::to_owned),
                query: query.to_vec(),
            });
            match self.mode {
                FixtureMode::Malformed => Ok(MarketDataReply {
                    body: br#"{"rt_cd":"0","unexpected":[]}"#.to_vec(),
                    continuation: None,
                }),
                FixtureMode::RateLimited => Err(KisError::RateLimited {
                    endpoint: path.to_owned(),
                    retry_after_ms: 250,
                }),
                FixtureMode::Healthy => Ok(MarketDataReply {
                    body: Self::body(path, continuation),
                    continuation: (path == INVESTOR_FLOW_PATH && continuation.is_none())
                        .then(|| "M".to_owned()),
                }),
            }
        }
    }

    fn instrument() -> InstrumentId {
        InstrumentId::parse("005930.KRX").expect("valid KRX instrument")
    }

    fn request(kinds: Vec<ResponseKind>) -> FetchRequest {
        FetchRequest {
            market: "kr".to_owned(),
            date: TradingDate::parse("2026-08-14").expect("valid date"),
            kinds,
            now: UtcTimestamp::parse_rfc3339("2026-08-14T07:00:00Z").expect("valid timestamp"),
            batch_id: BatchId::generate(),
        }
    }

    fn ingest_request() -> IngestRequest {
        IngestRequest::new(
            "kr".to_owned(),
            TradingDate::parse("2026-08-14").expect("valid date"),
            UtcTimestamp::parse_rfc3339("2026-08-14T07:00:00Z").expect("valid timestamp"),
        )
    }

    #[tokio::test]
    async fn fetches_supported_candidate_kinds_and_follows_continuation() {
        let reader = FixtureReader::new(FixtureMode::Healthy);
        let calls = Arc::clone(&reader.calls);
        let provider =
            KisCandidateProvider::new(reader, vec![instrument()]).expect("candidate provider");
        let envelopes = provider
            .fetch(&request(KIS_CANDIDATE_SUPPORTED_KINDS.to_vec()))
            .await
            .expect("candidate wire fetch");

        assert_eq!(provider.provider_id(), PROVIDER_KIS_CANDIDATE);
        assert_eq!(provider.fetch_mode(), FetchMode::Credentialed);
        assert_eq!(
            envelopes
                .iter()
                .filter(|envelope| envelope.kind == ResponseKind::InvestorFlow)
                .count(),
            2
        );
        assert_eq!(
            envelopes
                .iter()
                .filter(|envelope| envelope.kind == ResponseKind::Fundamentals)
                .count(),
            2
        );
        let calls = calls.lock().expect("calls lock");
        assert_eq!(calls[0].path, INVESTOR_FLOW_PATH);
        assert!(calls[0].continuation.is_none());
        assert_eq!(calls[1].continuation.as_deref(), Some("N"));
        assert!(
            calls[0]
                .query
                .contains(&("FID_INPUT_ISCD".to_owned(), "005930".to_owned()))
        );
        assert!(
            calls[2]
                .query
                .contains(&("FID_DIV_CLS_CODE".to_owned(), "0".to_owned()))
        );
    }

    #[tokio::test]
    async fn malformed_wire_response_is_permanent_schema_error() {
        let provider = KisCandidateProvider::new(
            FixtureReader::new(FixtureMode::Malformed),
            vec![instrument()],
        )
        .expect("candidate provider");
        let error = provider
            .fetch(&request(vec![ResponseKind::InvestorFlow]))
            .await
            .expect_err("malformed KIS wire must fail");
        assert!(matches!(
            error,
            ProviderError::Remote {
                code: "BROKER_SCHEMA_DRIFT",
                retryable: false,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn rate_limit_is_classified_retryable_without_persisting_bytes() {
        let provider = KisCandidateProvider::new(
            FixtureReader::new(FixtureMode::RateLimited),
            vec![instrument()],
        )
        .expect("candidate provider");
        let error = provider
            .fetch(&request(vec![ResponseKind::InvestorFlow]))
            .await
            .expect_err("rate limit must fail typed");
        assert!(matches!(
            error,
            ProviderError::Remote {
                code: "BROKER_RATE_LIMITED",
                retryable: true,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn ingest_and_normalize_candidate_wire_is_idempotent() {
        let root = tempfile::tempdir().expect("temp raw root");
        let store = RawStore::new(root.path());
        let provider =
            KisCandidateProvider::new(FixtureReader::new(FixtureMode::Healthy), vec![instrument()])
                .expect("candidate provider");
        let raw = ingest_kis_candidate_bundle_with_kinds(
            &store,
            &provider,
            &ingest_request(),
            None,
            &[ResponseKind::InvestorFlow],
        )
        .await
        .expect("candidate raw ingest");
        assert_eq!(raw.entry.provider, PROVIDER_KIS_CANDIDATE);
        assert_eq!(raw.entry.files.len(), 2);

        let first =
            normalize_kis_candidate_batch(&store, &raw.entry).expect("candidate normalization");
        assert_eq!(
            first.entry.provider,
            crate::contract::PROVIDER_KIS_CANDIDATE_NORMALIZED
        );
        assert_eq!(first.entry.files.len(), 1);
        let replay = normalize_kis_candidate_batch(&store, &raw.entry)
            .expect("deterministic candidate replay");
        assert_eq!(first.entry, replay.entry);
        assert_eq!(
            store
                .read_manifest(crate::contract::PROVIDER_KIS_CANDIDATE_NORMALIZED, "kr")
                .expect("normalized manifest")
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn finance_raw_is_immutable_but_normalization_fails_closed() {
        let root = tempfile::tempdir().expect("temp raw root");
        let store = RawStore::new(root.path());
        let provider =
            KisCandidateProvider::new(FixtureReader::new(FixtureMode::Healthy), vec![instrument()])
                .expect("candidate provider");
        let raw = ingest_kis_candidate_bundle_with_kinds(
            &store,
            &provider,
            &ingest_request(),
            None,
            &[ResponseKind::Fundamentals],
        )
        .await
        .expect("finance raw ingest");
        let raw_manifest = store
            .read_manifest(PROVIDER_KIS_CANDIDATE, "kr")
            .expect("raw manifest");
        assert_eq!(raw_manifest, vec![raw.entry.clone()]);

        let error = normalize_kis_candidate_batch(&store, &raw.entry)
            .expect_err("unverified finance semantics must be blocked");
        match error {
            crate::candidate_normalize::CandidateNormalizeError::UnverifiedFinanceSemantics {
                file_name,
                reason,
            } => {
                assert!(file_name.contains("fundamentals"));
                assert!(reason.contains("99.99"));
            }
            other => panic!("expected typed finance gate, got {other:?}"),
        }
        assert!(
            store
                .read_manifest(crate::contract::PROVIDER_KIS_CANDIDATE_NORMALIZED, "kr")
                .expect("normalized manifest")
                .is_empty()
        );
        assert_eq!(
            store
                .read_manifest(PROVIDER_KIS_CANDIDATE, "kr")
                .expect("raw manifest remains immutable"),
            raw_manifest
        );
    }

    #[tokio::test]
    async fn membership_sector_and_market_status_fail_before_network() {
        let reader = FixtureReader::new(FixtureMode::Healthy);
        let calls = Arc::clone(&reader.calls);
        let provider =
            KisCandidateProvider::new(reader, vec![instrument()]).expect("candidate provider");
        for kind in KIS_CANDIDATE_UNSUPPORTED_KINDS {
            let error = provider
                .fetch(&request(vec![kind]))
                .await
                .expect_err("unsupported candidate source");
            assert!(matches!(error, ProviderError::UnsupportedKind(actual) if actual == kind));
        }
        assert!(calls.lock().expect("calls lock").is_empty());
    }
}
