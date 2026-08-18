//! Korea Investment & Securities Open API market-data provider.
//!
//! The adapter owns endpoint selection and request metadata, while
//! `kis-client` owns authentication, secret redaction, rate limiting, retries,
//! and HTTP. Every successful KIS body becomes one immutable [`RawEnvelope`]
//! without parsing or rewriting its bytes.

use std::future::Future;

use domain::{InstrumentId, Venue};
use kis_client::{
    CredentialSource, KisError, KisMarketDataClient, MarketDataReply, Sleeper, Transport,
};

use crate::contract::{FetchMode, PROVIDER_KIS, RawEnvelope, RequestMetadata, ResponseKind};
use crate::provider::{FetchRequest, ProviderError, RemoteDiagnostic};

/// Fixed launch universe from `configs/universes/kr-etf-core-v1.yaml`.
pub const KR_ETF_CORE_SYMBOLS: [&str; 11] = [
    "069500", "102110", "229200", "143850", "133690", "195930", "192090", "148070", "114260",
    "153130", "132030",
];

const DAILY_BARS_PATH: &str = "/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice";
const DAILY_BARS_TR_ID: &str = "FHKST03010100";
const REFERENCE_PATH: &str = "/uapi/domestic-stock/v1/quotations/inquire-price";
const REFERENCE_TR_ID: &str = "FHKST01010100";
const CALENDAR_PATH: &str = "/uapi/domestic-stock/v1/quotations/chk-holiday";
const CALENDAR_TR_ID: &str = "CTCA0903R";
const MAX_PAGES: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaginationPolicy {
    BodyCursor,
    SinglePage,
}

/// Async read seam implemented by the production KIS client and by fixtures.
#[allow(async_fn_in_trait)]
pub trait KisRead: std::fmt::Debug + Send + Sync {
    fn get(
        &self,
        path: &str,
        tr_id: &str,
        query: &[(String, String)],
        continuation: Option<&str>,
    ) -> impl Future<Output = Result<MarketDataReply, KisError>> + Send;
}

impl<T, S, C> KisRead for KisMarketDataClient<T, S, C>
where
    T: Transport,
    S: Sleeper,
    C: CredentialSource + Send + Sync,
{
    async fn get(
        &self,
        path: &str,
        tr_id: &str,
        query: &[(String, String)],
        continuation: Option<&str>,
    ) -> Result<MarketDataReply, KisError> {
        KisMarketDataClient::get(self, path, tr_id, query, continuation).await
    }
}

/// Credentialed KIS adapter for the fixed Korean ETF launch universe.
#[derive(Debug)]
pub struct KisProvider<R: KisRead> {
    reader: R,
    instruments: Vec<InstrumentId>,
}

impl<R: KisRead> KisProvider<R> {
    pub fn new(reader: R, instruments: Vec<InstrumentId>) -> Result<Self, ProviderError> {
        if instruments.is_empty() {
            return Err(ProviderError::InvalidConfiguration {
                detail: "KIS provider requires at least one instrument".to_owned(),
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
                        "KIS domestic market data requires a six-digit KRX instrument, got {instrument}"
                    ),
                });
            }
        }
        Ok(Self {
            reader,
            instruments,
        })
    }

    pub fn kr_etf_core(reader: R) -> Self {
        let instruments = KR_ETF_CORE_SYMBOLS
            .iter()
            .map(|symbol| {
                InstrumentId::from_parts(symbol, Venue::Krx)
                    .expect("the checked-in KR ETF universe is valid")
            })
            .collect();
        Self {
            reader,
            instruments,
        }
    }

    pub fn provider_id(&self) -> &'static str {
        PROVIDER_KIS
    }

    pub fn fetch_mode(&self) -> FetchMode {
        FetchMode::Credentialed
    }

    /// Fetch all requested EOD response classes as exact KIS response bytes.
    pub async fn fetch(&self, req: &FetchRequest) -> Result<Vec<RawEnvelope>, ProviderError> {
        if req.market != "kr" {
            return Err(ProviderError::InvalidConfiguration {
                detail: format!("KIS provider supports market kr, got {:?}", req.market),
            });
        }

        let date = req.date.to_iso().replace('-', "");
        let mut envelopes = Vec::new();
        for kind in &req.kinds {
            match kind {
                ResponseKind::Bars => {
                    for instrument in &self.instruments {
                        let query = vec![
                            ("FID_COND_MRKT_DIV_CODE".to_owned(), "J".to_owned()),
                            ("FID_INPUT_ISCD".to_owned(), instrument.symbol().to_owned()),
                            ("FID_INPUT_DATE_1".to_owned(), date.clone()),
                            ("FID_INPUT_DATE_2".to_owned(), date.clone()),
                            ("FID_PERIOD_DIV_CODE".to_owned(), "D".to_owned()),
                            // Preserve unadjusted execution prices; corporate actions are
                            // curated separately for adjusted/total-return series.
                            ("FID_ORG_ADJ_PRC".to_owned(), "1".to_owned()),
                        ];
                        self.fetch_pages(
                            req,
                            *kind,
                            "daily-bars",
                            Some(instrument.symbol()),
                            DAILY_BARS_PATH,
                            DAILY_BARS_TR_ID,
                            query,
                            PaginationPolicy::BodyCursor,
                            &mut envelopes,
                        )
                        .await?;
                    }
                }
                ResponseKind::Reference => {
                    for instrument in &self.instruments {
                        let query = vec![
                            ("FID_COND_MRKT_DIV_CODE".to_owned(), "J".to_owned()),
                            ("FID_INPUT_ISCD".to_owned(), instrument.symbol().to_owned()),
                        ];
                        self.fetch_pages(
                            req,
                            *kind,
                            "reference",
                            Some(instrument.symbol()),
                            REFERENCE_PATH,
                            REFERENCE_TR_ID,
                            query,
                            PaginationPolicy::BodyCursor,
                            &mut envelopes,
                        )
                        .await?;
                    }
                }
                ResponseKind::Calendar => {
                    self.fetch_pages(
                        req,
                        *kind,
                        "calendar",
                        None,
                        CALENDAR_PATH,
                        CALENDAR_TR_ID,
                        vec![
                            ("BASS_DT".to_owned(), date.clone()),
                            ("CTX_AREA_FK".to_owned(), String::new()),
                            ("CTX_AREA_NK".to_owned(), String::new()),
                        ],
                        PaginationPolicy::SinglePage,
                        &mut envelopes,
                    )
                    .await?;
                }
                ResponseKind::CorporateActions => {
                    for endpoint in corporate_action_endpoints(&date) {
                        self.fetch_pages(
                            req,
                            *kind,
                            endpoint.label,
                            None,
                            endpoint.path,
                            endpoint.tr_id,
                            endpoint.query,
                            PaginationPolicy::BodyCursor,
                            &mut envelopes,
                        )
                        .await?;
                    }
                }
                unsupported => return Err(ProviderError::UnsupportedKind(*unsupported)),
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
        symbol: Option<&str>,
        path: &str,
        tr_id: &str,
        mut query: Vec<(String, String)>,
        pagination: PaginationPolicy,
        output: &mut Vec<RawEnvelope>,
    ) -> Result<(), ProviderError> {
        let mut continuation = None;
        for page in 1..=MAX_PAGES {
            let sent_query = query.clone();
            let reply = self
                .reader
                .get(path, tr_id, &sent_query, continuation.as_deref())
                .await
                .map_err(|error| remote_error(kind, path, error))?;
            let file_name = match symbol {
                Some(symbol) => format!("{label}-{symbol}-page-{page:02}.json"),
                None => format!("{label}-page-{page:02}.json"),
            };
            let should_continue = reply
                .continuation
                .as_deref()
                .is_some_and(|value| matches!(value, "M" | "F"));
            let cursor_advanced = pagination == PaginationPolicy::SinglePage
                || !should_continue
                || update_continuation_query(&mut query, &reply.body);
            output.push(RawEnvelope::new(
                req.batch_id,
                kind,
                file_name,
                reply.body,
                req.now,
                RequestMetadata {
                    endpoint: path.to_owned(),
                    query: sent_query,
                    headers: vec![
                        ("authorization".to_owned(), "[REDACTED]".to_owned()),
                        ("appkey".to_owned(), "[REDACTED]".to_owned()),
                        ("appsecret".to_owned(), "[REDACTED]".to_owned()),
                        ("tr_id".to_owned(), tr_id.to_owned()),
                    ],
                    mode: FetchMode::Credentialed,
                },
            ));

            // The current KIS layout marks chk-holiday as not supporting
            // tr_cont pagination and requires both CTX_AREA fields to remain
            // blank. Its response can nevertheless carry continuation-like
            // header/body values, which must not turn one daily lookup into a
            // repeated request loop.
            if pagination == PaginationPolicy::SinglePage {
                return Ok(());
            }

            if !should_continue {
                return Ok(());
            }
            if !cursor_advanced {
                return Err(pagination_error(
                    kind,
                    path,
                    "BROKER_PAGINATION_STALLED",
                    "continuation cursor did not advance",
                ));
            }
            continuation = Some("N".to_owned());
        }
        Err(pagination_error(
            kind,
            path,
            "BROKER_PAGINATION_LIMIT",
            "page count exceeded the safety limit",
        ))
    }
}

fn pagination_error(
    kind: ResponseKind,
    path: &str,
    code: &'static str,
    detail: &'static str,
) -> ProviderError {
    ProviderError::Remote {
        provider: PROVIDER_KIS,
        kind,
        code,
        retryable: false,
        diagnostic: Some(RemoteDiagnostic {
            endpoint: path.to_owned(),
            http_status: None,
        }),
        detail: detail.to_owned(),
    }
}

fn remote_error(kind: ResponseKind, requested_endpoint: &str, error: KisError) -> ProviderError {
    let http_status = match &error {
        KisError::Broker { status, .. } => Some(*status),
        _ => None,
    };
    ProviderError::Remote {
        provider: PROVIDER_KIS,
        kind,
        code: error.code(),
        retryable: error.is_retryable(kis_client::RequestKind::Read),
        diagnostic: Some(RemoteDiagnostic {
            endpoint: requested_endpoint.to_owned(),
            http_status,
        }),
        detail: error.to_string(),
    }
}

#[derive(Debug)]
struct EndpointSpec {
    label: &'static str,
    path: &'static str,
    tr_id: &'static str,
    query: Vec<(String, String)>,
}

fn corporate_action_endpoints(date: &str) -> Vec<EndpointSpec> {
    let common = |extra: &[(&str, &str)]| {
        let mut query = vec![
            ("CTS".to_owned(), String::new()),
            ("F_DT".to_owned(), date.to_owned()),
            ("T_DT".to_owned(), date.to_owned()),
            ("SHT_CD".to_owned(), String::new()),
        ];
        query.extend(
            extra
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned())),
        );
        query
    };
    vec![
        EndpointSpec {
            label: "corporate-actions-paidin-subscription",
            path: "/uapi/domestic-stock/v1/ksdinfo/paidin-capin",
            tr_id: "HHKDB669100C0",
            query: common(&[("GB1", "1")]),
        },
        EndpointSpec {
            label: "corporate-actions-paidin-record",
            path: "/uapi/domestic-stock/v1/ksdinfo/paidin-capin",
            tr_id: "HHKDB669100C0",
            query: common(&[("GB1", "2")]),
        },
        EndpointSpec {
            label: "corporate-actions-bonus",
            path: "/uapi/domestic-stock/v1/ksdinfo/bonus-issue",
            tr_id: "HHKDB669101C0",
            query: common(&[]),
        },
        EndpointSpec {
            label: "corporate-actions-dividend",
            path: "/uapi/domestic-stock/v1/ksdinfo/dividend",
            tr_id: "HHKDB669102C0",
            query: common(&[("GB1", "0"), ("HIGH_GB", "")]),
        },
        EndpointSpec {
            label: "corporate-actions-merger-split",
            path: "/uapi/domestic-stock/v1/ksdinfo/merger-split",
            tr_id: "HHKDB669104C0",
            query: common(&[]),
        },
        EndpointSpec {
            label: "corporate-actions-reverse-split",
            path: "/uapi/domestic-stock/v1/ksdinfo/rev-split",
            tr_id: "HHKDB669105C0",
            query: common(&[("MARKET_GB", "0")]),
        },
        EndpointSpec {
            label: "corporate-actions-capital-decrease",
            path: "/uapi/domestic-stock/v1/ksdinfo/cap-dcrs",
            tr_id: "HHKDB669106C0",
            query: common(&[]),
        },
    ]
}

fn update_continuation_query(query: &mut [(String, String)], body: &[u8]) -> bool {
    let Ok(document) = serde_json::from_slice::<serde_json::Value>(body) else {
        return false;
    };
    let mut advanced = false;
    for (query_key, response_key) in [
        ("CTX_AREA_FK", "ctx_area_fk"),
        ("CTX_AREA_NK", "ctx_area_nk"),
        ("CTS", "cts"),
    ] {
        let Some(value) = document
            .get(response_key)
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        if let Some((_, current)) = query.iter_mut().find(|(key, _)| key == query_key) {
            if value.trim().is_empty() || current == value {
                continue;
            }
            *current = value.to_owned();
            advanced = true;
        }
    }
    advanced
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KisResponseValidationError {
    pub(crate) kind: ResponseKind,
    pub(crate) code: &'static str,
    pub(crate) reason: String,
}

pub(crate) fn validate_kis_response(
    kind: ResponseKind,
    endpoint: &str,
    bytes: &[u8],
) -> Result<(), KisResponseValidationError> {
    let document: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|error| KisResponseValidationError {
            kind,
            code: "KIS_RESPONSE_MALFORMED_JSON",
            reason: format!("not valid KIS JSON: {error}"),
        })?;
    let object = document
        .as_object()
        .ok_or_else(|| KisResponseValidationError {
            kind,
            code: "KIS_RESPONSE_SCHEMA_INVALID",
            reason: "KIS response must be a JSON object".to_owned(),
        })?;
    if object.get("rt_cd").and_then(serde_json::Value::as_str) != Some("0") {
        return Err(KisResponseValidationError {
            kind,
            code: "KIS_RESPONSE_SCHEMA_INVALID",
            reason: "KIS response rt_cd must equal 0".to_owned(),
        });
    }

    let valid = match (kind, endpoint) {
        (ResponseKind::Bars, DAILY_BARS_PATH) => {
            object
                .get("output1")
                .is_some_and(serde_json::Value::is_object)
                && object
                    .get("output2")
                    .is_some_and(serde_json::Value::is_array)
        }
        (ResponseKind::Reference, REFERENCE_PATH) => object
            .get("output")
            .is_some_and(serde_json::Value::is_object),
        (ResponseKind::Calendar, CALENDAR_PATH) => object
            .get("output")
            .is_some_and(|value| value.is_object() || value.is_array()),
        (ResponseKind::CorporateActions, path)
            if path.starts_with("/uapi/domestic-stock/v1/ksdinfo/") =>
        {
            object
                .get("output1")
                .is_some_and(|value| value.is_object() || value.is_array())
        }
        _ => false,
    };
    if !valid {
        return Err(KisResponseValidationError {
            kind,
            code: "KIS_RESPONSE_SCHEMA_INVALID",
            reason: format!("unexpected KIS response shape for endpoint {endpoint:?}"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use domain::{BatchId, TradingDate, UtcTimestamp};
    use kis_client::MarketDataReply;

    use super::*;
    use crate::ingest::{IngestError, IngestRequest, ingest_kis_bundle};
    use crate::storage::RawStore;

    #[derive(Debug)]
    struct FixtureReader {
        calls: Mutex<Vec<RecordedCall>>,
        continuation_once: bool,
    }

    #[derive(Debug)]
    struct RecordedCall {
        query: Vec<(String, String)>,
        continuation: Option<String>,
    }

    impl FixtureReader {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                continuation_once: false,
            }
        }

        fn with_one_continuation() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                continuation_once: true,
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
            self.calls.lock().unwrap().push(RecordedCall {
                query: query.to_vec(),
                continuation: continuation.map(str::to_owned),
            });
            let should_continue = self.continuation_once && continuation.is_none();
            let body = match path {
                DAILY_BARS_PATH => br#"{"rt_cd":"0","output1":{},"output2":[]}"#.to_vec(),
                REFERENCE_PATH => br#"{"rt_cd":"0","output":{}}"#.to_vec(),
                CALENDAR_PATH => {
                    br#"{"rt_cd":"0","ctx_area_fk":"next-fk","ctx_area_nk":"next-nk","output":[]}"#
                        .to_vec()
                }
                "/uapi/domestic-stock/v1/ksdinfo/paidin-capin" => {
                    br#"{"rt_cd":"0","cts":"next-cts","output1":[]}"#.to_vec()
                }
                _ => br#"{"rt_cd":"0","cts":"next-cts","output1":[]}"#.to_vec(),
            };
            Ok(MarketDataReply {
                body,
                continuation: should_continue.then(|| "F".to_owned()),
            })
        }
    }

    #[derive(Debug)]
    struct MalformedReader;

    impl KisRead for MalformedReader {
        async fn get(
            &self,
            _path: &str,
            _tr_id: &str,
            _query: &[(String, String)],
            _continuation: Option<&str>,
        ) -> Result<MarketDataReply, KisError> {
            Ok(MarketDataReply {
                body: br#"{"rt_cd":"0","wrong":[]}"#.to_vec(),
                continuation: None,
            })
        }
    }

    #[derive(Debug)]
    struct AlwaysContinuationReader {
        calls: AtomicUsize,
        advancing: bool,
    }

    impl KisRead for AlwaysContinuationReader {
        async fn get(
            &self,
            _path: &str,
            _tr_id: &str,
            _query: &[(String, String)],
            _continuation: Option<&str>,
        ) -> Result<MarketDataReply, KisError> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            let cursor = if self.advancing {
                format!("cursor-{call}")
            } else {
                "same-cursor".to_owned()
            };
            Ok(MarketDataReply {
                body: format!(r#"{{"rt_cd":"0","cts":"{cursor}","output1":[]}}"#).into_bytes(),
                continuation: Some("F".to_owned()),
            })
        }
    }

    #[test]
    fn remote_error_keeps_status_and_requested_endpoint_but_not_body_in_diagnostic() {
        let error = remote_error(
            ResponseKind::Bars,
            DAILY_BARS_PATH,
            KisError::Broker {
                status: 403,
                endpoint: DAILY_BARS_PATH.to_owned(),
                body: "appsecret=fixture-secret".to_owned(),
            },
        );
        match error {
            ProviderError::Remote {
                code,
                diagnostic: Some(diagnostic),
                ..
            } => {
                assert_eq!(code, "BROKER_REJECTED");
                assert_eq!(diagnostic.endpoint, DAILY_BARS_PATH);
                assert_eq!(diagnostic.http_status, Some(403));
                assert!(!diagnostic.endpoint.contains("fixture-secret"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    fn request(kinds: Vec<ResponseKind>) -> FetchRequest {
        FetchRequest {
            market: "kr".to_owned(),
            date: TradingDate::parse("2026-08-14").unwrap(),
            kinds,
            now: UtcTimestamp::parse_rfc3339("2026-08-14T07:00:00Z").unwrap(),
            batch_id: BatchId::generate(),
        }
    }

    #[tokio::test]
    async fn fixed_universe_fetches_exact_raw_eod_responses() {
        let provider = KisProvider::kr_etf_core(FixtureReader::new());
        let fetched = provider
            .fetch(&request(vec![
                ResponseKind::Bars,
                ResponseKind::Reference,
                ResponseKind::Calendar,
                ResponseKind::CorporateActions,
            ]))
            .await
            .expect("KIS EOD fetch");

        assert_eq!(provider.provider_id(), "kis");
        assert_eq!(provider.fetch_mode(), FetchMode::Credentialed);
        assert_eq!(
            fetched
                .iter()
                .filter(|item| item.kind == ResponseKind::Bars)
                .count(),
            KR_ETF_CORE_SYMBOLS.len()
        );
        assert_eq!(
            fetched
                .iter()
                .filter(|item| item.kind == ResponseKind::Reference)
                .count(),
            KR_ETF_CORE_SYMBOLS.len()
        );
        assert_eq!(
            fetched
                .iter()
                .filter(|item| item.kind == ResponseKind::Calendar)
                .count(),
            1
        );
        assert_eq!(
            fetched
                .iter()
                .filter(|item| item.kind == ResponseKind::CorporateActions)
                .count(),
            7
        );
        assert!(fetched.iter().all(|item| {
            item.request.mode == FetchMode::Credentialed
                && item
                    .request
                    .headers
                    .iter()
                    .filter(|(name, _)| {
                        matches!(name.as_str(), "authorization" | "appkey" | "appsecret")
                    })
                    .all(|(_, value)| value == "[REDACTED]")
        }));
        assert_eq!(
            fetched[0].bytes,
            br#"{"rt_cd":"0","output1":{},"output2":[]}"#
        );
        assert_eq!(
            fetched[0].content_hash,
            domain::ContentHash::from_bytes(&fetched[0].bytes)
        );
    }

    #[tokio::test]
    async fn credentialed_ingest_commits_one_exact_kis_raw_batch() {
        let root = tempfile::tempdir().unwrap();
        let store = RawStore::new(root.path());
        let provider = KisProvider::kr_etf_core(FixtureReader::new());
        let req = IngestRequest::new(
            "kr".to_owned(),
            TradingDate::parse("2026-08-14").unwrap(),
            UtcTimestamp::parse_rfc3339("2026-08-14T07:00:00Z").unwrap(),
        );

        let outcome =
            ingest_kis_bundle(&store, &provider, &req, Some("contract://kis-market-data"))
                .await
                .unwrap();

        assert_eq!(outcome.entry.provider, PROVIDER_KIS);
        assert_eq!(outcome.entry.mode, FetchMode::Credentialed);
        assert_eq!(
            outcome.entry.entitlement_reference.as_deref(),
            Some("contract://kis-market-data")
        );
        assert_eq!(outcome.files.len(), 30);
        assert_eq!(
            outcome.files[0].bytes,
            br#"{"rt_cd":"0","output1":{},"output2":[]}"#
        );
        assert_eq!(
            store.read_manifest(PROVIDER_KIS, "kr").unwrap(),
            vec![outcome.entry]
        );
    }

    #[tokio::test]
    async fn malformed_kis_wire_is_rejected_before_raw_visibility() {
        let root = tempfile::tempdir().unwrap();
        let store = RawStore::new(root.path());
        let provider = KisProvider::kr_etf_core(MalformedReader);
        let req = IngestRequest::new(
            "kr".to_owned(),
            TradingDate::parse("2026-08-14").unwrap(),
            UtcTimestamp::parse_rfc3339("2026-08-14T07:00:00Z").unwrap(),
        );

        let error = ingest_kis_bundle(&store, &provider, &req, None)
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            IngestError::MalformedResponse {
                kind: ResponseKind::Bars,
                diagnostic: Some(crate::ingest::ResponseValidationDiagnostic {
                    code: "KIS_RESPONSE_SCHEMA_INVALID",
                    ref endpoint,
                    ref file_name,
                }),
                ..
            } if endpoint == DAILY_BARS_PATH
                && file_name == "daily-bars-069500-page-01.json"
        ));
        assert!(store.read_manifest(PROVIDER_KIS, "kr").unwrap().is_empty());
    }

    #[tokio::test]
    async fn calendar_ignores_continuation_like_metadata_by_contract() {
        let provider = KisProvider::kr_etf_core(FixtureReader::with_one_continuation());
        let fetched = provider
            .fetch(&request(vec![ResponseKind::Calendar]))
            .await
            .expect("single-page calendar");
        assert_eq!(fetched.len(), 1);
        assert!(
            fetched[0]
                .request
                .query
                .contains(&("CTX_AREA_FK".to_owned(), String::new()))
        );
        let calls = provider.reader.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].continuation.is_none());
        assert!(
            calls[0]
                .query
                .contains(&("CTX_AREA_NK".to_owned(), String::new()))
        );
    }

    #[tokio::test]
    async fn other_endpoints_keep_body_cursor_continuation() {
        let provider = KisProvider::kr_etf_core(FixtureReader::with_one_continuation());
        let fetched = provider
            .fetch(&request(vec![ResponseKind::CorporateActions]))
            .await
            .expect("paginated corporate actions");
        assert_eq!(fetched.len(), 14);
        assert!(
            fetched[1]
                .request
                .query
                .contains(&("CTS".to_owned(), "next-cts".to_owned()))
        );
        let calls = provider.reader.calls.lock().unwrap();
        assert_eq!(calls[1].continuation.as_deref(), Some("N"));
        assert!(
            calls[1]
                .query
                .contains(&("CTS".to_owned(), "next-cts".to_owned()))
        );
    }

    #[tokio::test]
    async fn body_cursor_pagination_fails_on_stall_and_at_the_page_limit() {
        for (advancing, expected_code, expected_calls) in [
            (false, "BROKER_PAGINATION_STALLED", 2),
            (true, "BROKER_PAGINATION_LIMIT", MAX_PAGES),
        ] {
            let reader = AlwaysContinuationReader {
                calls: AtomicUsize::new(0),
                advancing,
            };
            let provider = KisProvider::kr_etf_core(reader);
            let error = provider
                .fetch_pages(
                    &request(vec![ResponseKind::CorporateActions]),
                    ResponseKind::CorporateActions,
                    "fixture",
                    None,
                    "/uapi/domestic-stock/v1/ksdinfo/bonus-issue",
                    "HHKDB669101C0",
                    vec![("CTS".to_owned(), String::new())],
                    PaginationPolicy::BodyCursor,
                    &mut Vec::new(),
                )
                .await
                .expect_err("unbounded continuation must fail closed");
            assert!(matches!(
                error,
                ProviderError::Remote {
                    code,
                    diagnostic: Some(RemoteDiagnostic { ref endpoint, .. }),
                    ..
                } if code == expected_code
                    && endpoint == "/uapi/domestic-stock/v1/ksdinfo/bonus-issue"
            ));
            assert_eq!(provider.reader.calls.load(Ordering::SeqCst), expected_calls);
        }
    }

    #[tokio::test]
    async fn candidate_kinds_fail_closed_until_their_adapter_exists() {
        let provider = KisProvider::kr_etf_core(FixtureReader::new());
        let error = provider
            .fetch(&request(vec![ResponseKind::Fundamentals]))
            .await
            .expect_err("candidate source is a separate adapter");
        assert!(matches!(
            error,
            ProviderError::UnsupportedKind(ResponseKind::Fundamentals)
        ));
    }

    #[test]
    fn non_krx_or_non_six_digit_instruments_are_rejected() {
        for id in ["SPY.ARCA", "A.KRX"] {
            let error =
                KisProvider::new(FixtureReader::new(), vec![InstrumentId::parse(id).unwrap()])
                    .expect_err("invalid KIS domestic instrument");
            assert!(matches!(error, ProviderError::InvalidConfiguration { .. }));
        }
    }

    #[test]
    fn kis_wire_validation_is_endpoint_specific_and_fail_closed() {
        for (kind, endpoint, body) in [
            (
                ResponseKind::Bars,
                DAILY_BARS_PATH,
                br#"{"rt_cd":"0","output1":{},"output2":[]}"#.as_slice(),
            ),
            (
                ResponseKind::Reference,
                REFERENCE_PATH,
                br#"{"rt_cd":"0","output":{}}"#.as_slice(),
            ),
            (
                ResponseKind::Calendar,
                CALENDAR_PATH,
                br#"{"rt_cd":"0","output":[]}"#.as_slice(),
            ),
            (
                ResponseKind::CorporateActions,
                "/uapi/domestic-stock/v1/ksdinfo/dividend",
                br#"{"rt_cd":"0","output1":[]}"#.as_slice(),
            ),
            (
                ResponseKind::CorporateActions,
                "/uapi/domestic-stock/v1/ksdinfo/paidin-capin",
                br#"{"rt_cd":"0","output1":[]}"#.as_slice(),
            ),
        ] {
            validate_kis_response(kind, endpoint, body).expect("official KIS wire shape");
        }

        for body in [
            br#"{"rt_cd":"1","output2":[]}"#.as_slice(),
            br#"{"rt_cd":"0","bars":[]}"#.as_slice(),
        ] {
            assert!(validate_kis_response(ResponseKind::Bars, DAILY_BARS_PATH, body).is_err());
        }
        assert!(
            validate_kis_response(
                ResponseKind::Bars,
                "/uapi/domestic-stock/v1/quotations/unknown",
                br#"{"rt_cd":"0","output1":{},"output2":[]}"#,
            )
            .is_err()
        );
        assert_eq!(
            validate_kis_response(ResponseKind::Bars, DAILY_BARS_PATH, b"not-json")
                .unwrap_err()
                .code,
            "KIS_RESPONSE_MALFORMED_JSON"
        );
        assert_eq!(
            validate_kis_response(
                ResponseKind::Bars,
                DAILY_BARS_PATH,
                br#"{"rt_cd":"0","output1":{},"output2":{}}"#,
            )
            .unwrap_err()
            .code,
            "KIS_RESPONSE_SCHEMA_INVALID"
        );
        assert_eq!(
            validate_kis_response(
                ResponseKind::CorporateActions,
                "/uapi/domestic-stock/v1/ksdinfo/paidin-capin",
                // Sheet 105's layout says `output`, but its response example
                // and the official generated client both require `output1`.
                br#"{"rt_cd":"0","output":[]}"#,
            )
            .unwrap_err()
            .code,
            "KIS_RESPONSE_SCHEMA_INVALID"
        );
    }
}
