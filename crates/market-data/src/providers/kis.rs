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
use crate::provider::{FetchRequest, ProviderError};
use crate::validate::ValidationError;

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
        output: &mut Vec<RawEnvelope>,
    ) -> Result<(), ProviderError> {
        let mut continuation = None;
        for page in 1..=MAX_PAGES {
            let sent_query = query.clone();
            let reply = self
                .reader
                .get(path, tr_id, &sent_query, continuation.as_deref())
                .await
                .map_err(|error| ProviderError::Remote {
                    provider: PROVIDER_KIS,
                    kind,
                    code: error.code(),
                    retryable: error.is_retryable(kis_client::RequestKind::Read),
                    detail: error.to_string(),
                })?;
            let file_name = match symbol {
                Some(symbol) => format!("{label}-{symbol}-page-{page:02}.json"),
                None => format!("{label}-page-{page:02}.json"),
            };
            update_continuation_query(&mut query, &reply.body);
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
            provider: PROVIDER_KIS,
            kind,
            code: "BROKER_PAGINATION_LIMIT",
            retryable: false,
            detail: format!("{path} exceeded the {MAX_PAGES}-page safety limit"),
        })
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

fn update_continuation_query(query: &mut [(String, String)], body: &[u8]) {
    let Ok(document) = serde_json::from_slice::<serde_json::Value>(body) else {
        return;
    };
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
            *current = value.to_owned();
        }
    }
}

pub(crate) fn validate_kis_response(
    kind: ResponseKind,
    endpoint: &str,
    bytes: &[u8],
) -> Result<(), ValidationError> {
    let document: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|error| ValidationError {
            kind,
            reason: format!("not valid KIS JSON: {error}"),
        })?;
    let object = document.as_object().ok_or_else(|| ValidationError {
        kind,
        reason: "KIS response must be a JSON object".to_owned(),
    })?;
    if object.get("rt_cd").and_then(serde_json::Value::as_str) != Some("0") {
        return Err(ValidationError {
            kind,
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
        (ResponseKind::CorporateActions, "/uapi/domestic-stock/v1/ksdinfo/paidin-capin") => object
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
        return Err(ValidationError {
            kind,
            reason: format!("unexpected KIS response shape for endpoint {endpoint:?}"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

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
                    br#"{"rt_cd":"0","output":[]}"#.to_vec()
                }
                _ => br#"{"rt_cd":"0","output1":[]}"#.to_vec(),
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

        assert!(matches!(error, IngestError::MalformedResponse { .. }));
        assert!(store.read_manifest(PROVIDER_KIS, "kr").unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_continuation_is_fetched_with_context_from_the_previous_body() {
        let provider = KisProvider::kr_etf_core(FixtureReader::with_one_continuation());
        let fetched = provider
            .fetch(&request(vec![ResponseKind::Calendar]))
            .await
            .expect("paginated calendar");
        assert_eq!(fetched.len(), 2);
        assert!(
            fetched[0]
                .request
                .query
                .contains(&("CTX_AREA_FK".to_owned(), String::new()))
        );
        assert!(
            fetched[1]
                .request
                .query
                .contains(&("CTX_AREA_FK".to_owned(), "next-fk".to_owned()))
        );
        let calls = provider.reader.calls.lock().unwrap();
        assert_eq!(calls[1].continuation.as_deref(), Some("N"));
        assert!(
            calls[1]
                .query
                .contains(&("CTX_AREA_FK".to_owned(), "next-fk".to_owned()))
        );
        assert!(
            calls[1]
                .query
                .contains(&("CTX_AREA_NK".to_owned(), "next-nk".to_owned()))
        );
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
                br#"{"rt_cd":"0","output":[]}"#.as_slice(),
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
    }
}
