//! Korea Investment & Securities Open API market-data provider.
//!
//! The adapter owns endpoint selection and request metadata, while
//! `kis-client` owns authentication, secret redaction, rate limiting, retries,
//! and HTTP. Every successful KIS body becomes one immutable [`RawEnvelope`]
//! without parsing or rewriting its bytes.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::{Arc, Mutex};

use domain::{BatchId, InstrumentId, TradingDate, Venue};
use kis_client::{
    CredentialSource, KisError, KisMarketDataClient, MarketDataReply, Sleeper, Transport,
};
use serde_json::Value;

use crate::contract::{FetchMode, PROVIDER_KIS, RawEnvelope, RequestMetadata, ResponseKind};
use crate::provider::{FetchRequest, ProviderError, RemoteDiagnostic};

/// Fixed launch universe from `configs/universes/kr-etf-core-v1.yaml`.
pub const KR_ETF_CORE_SYMBOLS: [&str; 11] = [
    "069500", "102110", "229200", "143850", "133690", "195930", "192090", "148070", "114260",
    "153130", "132030",
];

/// Scope of the dedicated KSD action-range Raw collector.
///
/// `WholeMarket` preserves the Stage4B-v0-compatible blank `SHT_CD` request.
/// `FixedEtf11` sends one exact short-code query per member of the reviewed
/// fixed ETF11 universe. Both scopes retain every page in one immutable Raw
/// batch; neither scope claims historical point-in-time completeness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KisActionRangeScope {
    WholeMarket,
    FixedEtf11,
}

impl KisActionRangeScope {
    pub const fn initial_call_count(self) -> usize {
        match self {
            Self::WholeMarket => KIS_ACTION_CLASS_COUNT,
            Self::FixedEtf11 => KIS_ACTION_CLASS_COUNT * KR_ETF_CORE_SYMBOLS.len(),
        }
    }

    pub const fn is_symbol_scoped(self) -> bool {
        matches!(self, Self::FixedEtf11)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WholeMarket => "whole-market",
            Self::FixedEtf11 => "etf11",
        }
    }
}

const DAILY_BARS_PATH: &str = "/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice";
const DAILY_BARS_TR_ID: &str = "FHKST03010100";
const REFERENCE_PATH: &str = "/uapi/domestic-stock/v1/quotations/inquire-price";
const REFERENCE_TR_ID: &str = "FHKST01010100";
const CALENDAR_PATH: &str = "/uapi/domestic-stock/v1/quotations/chk-holiday";
const CALENDAR_TR_ID: &str = "CTCA0903R";
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaginationPolicy {
    SinglePage,
    /// The historical daily-range contract is terminal-page-only. Any
    /// non-empty response continuation marker or continuation-like body field
    /// is permanently rejected because the Raw contract does not preserve a
    /// resumable cursor for this endpoint.
    StrictSinglePage,
    /// KSD schedule responses follow the endpoint-specific official GitHub
    /// sample: an initial blank `tr_cont`, followed by `N` only when the
    /// response header is exactly `M`. `F` and every other marker are terminal.
    KsdGithubSample,
}

const KSD_MAX_PAGES: usize = 10;

/// The seven logical KSD action classes. Paid-in capital is one endpoint/TR
/// pair with two explicitly different `GB1` response classes.
pub const KIS_ACTION_CLASS_COUNT: usize = 7;
/// Maximum pages retained for one KSD action class.
pub const KIS_ACTION_MAX_PAGES: usize = KSD_MAX_PAGES;

/// The documented maximum number of daily observations returned by one
/// `FHKST03010100` request.
pub const MAX_DAILY_BAR_OBSERVATIONS: usize = 100;
/// Guard against a non-progressing or maliciously long date-window replay.
pub const MAX_DAILY_BAR_WINDOWS: usize = 1_024;

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
    calendar_snapshot_cache: bool,
    calendar_snapshot: Arc<Mutex<Option<CalendarSnapshot>>>,
}

/// A single reviewed `chk-holiday` response shared by one bounded range
/// process.  KIS explicitly asks clients to call this service at most once per
/// day; a range must therefore reuse one exact response until its coverage is
/// exhausted, then stop rather than issue a second request.
#[derive(Debug, Clone)]
struct CalendarSnapshot {
    bytes: Vec<u8>,
    request: RequestMetadata,
    retrieved_at: domain::UtcTimestamp,
    covered_dates: BTreeMap<TradingDate, bool>,
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
            calendar_snapshot_cache: false,
            calendar_snapshot: Arc::new(Mutex::new(None)),
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
            calendar_snapshot_cache: false,
            calendar_snapshot: Arc::new(Mutex::new(None)),
        }
    }

    /// Enables the range-only `chk-holiday` snapshot contract.
    ///
    /// Ordinary one-date daemon/once calls retain their one request per
    /// invocation behavior.  A bounded historical range opts into this mode
    /// so one provider instance can reuse the exact first calendar response;
    /// a target outside that response fails closed instead of violating KIS's
    /// one-call-per-day operational guidance.
    pub fn with_calendar_snapshot_cache(mut self) -> Self {
        self.calendar_snapshot_cache = true;
        self
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
                            PaginationPolicy::SinglePage,
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
                            PaginationPolicy::SinglePage,
                            &mut envelopes,
                        )
                        .await?;
                    }
                }
                ResponseKind::Calendar => {
                    let query = vec![
                        ("BASS_DT".to_owned(), date.clone()),
                        ("CTX_AREA_FK".to_owned(), String::new()),
                        ("CTX_AREA_NK".to_owned(), String::new()),
                    ];
                    if self.calendar_snapshot_cache {
                        self.fetch_calendar_with_snapshot(req, query, &mut envelopes)
                            .await?;
                    } else {
                        self.fetch_pages(
                            req,
                            *kind,
                            "calendar",
                            None,
                            CALENDAR_PATH,
                            CALENDAR_TR_ID,
                            query,
                            PaginationPolicy::SinglePage,
                            &mut envelopes,
                        )
                        .await?;
                    }
                }
                ResponseKind::CorporateActions => {
                    for endpoint in corporate_action_endpoints(&date, &date, None) {
                        self.fetch_pages(
                            req,
                            *kind,
                            endpoint.label,
                            None,
                            endpoint.path,
                            endpoint.tr_id,
                            endpoint.query,
                            PaginationPolicy::KsdGithubSample,
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

    /// Fetches the complete KSD action-range request matrix as immutable Raw
    /// envelopes. The calls are strictly sequential. Every endpoint/class
    /// accumulates pages locally, and the complete output is returned only
    /// after all expected symbol/class groups have passed validation; callers
    /// can therefore commit it as one all-or-nothing Raw batch.
    pub async fn fetch_corporate_actions_range(
        &self,
        market: &str,
        start: TradingDate,
        end: TradingDate,
        now: domain::UtcTimestamp,
        batch_id: BatchId,
        scope: KisActionRangeScope,
    ) -> Result<Vec<RawEnvelope>, ProviderError> {
        if market != "kr" {
            return Err(ProviderError::InvalidConfiguration {
                detail: format!("KIS provider supports market kr, got {market:?}"),
            });
        }
        if end < start {
            return Err(ProviderError::InvalidConfiguration {
                detail: "KSD action range end precedes start".to_owned(),
            });
        }

        let request = FetchRequest {
            market: market.to_owned(),
            date: start,
            kinds: vec![ResponseKind::CorporateActions],
            now,
            batch_id,
        };
        let start_text = kis_date_text(start);
        let end_text = kis_date_text(end);
        let symbols: Vec<Option<&str>> = match scope {
            KisActionRangeScope::WholeMarket => vec![None],
            KisActionRangeScope::FixedEtf11 => {
                KR_ETF_CORE_SYMBOLS.iter().copied().map(Some).collect()
            }
        };
        let mut envelopes = Vec::new();

        for symbol in symbols {
            for endpoint in corporate_action_endpoints(&start_text, &end_text, symbol) {
                let mut pages = Vec::new();
                self.fetch_pages(
                    &request,
                    ResponseKind::CorporateActions,
                    endpoint.label,
                    symbol,
                    endpoint.path,
                    endpoint.tr_id,
                    endpoint.query,
                    PaginationPolicy::KsdGithubSample,
                    &mut pages,
                )
                .await?;

                for envelope in &pages {
                    validate_kis_response(
                        ResponseKind::CorporateActions,
                        endpoint.path,
                        &envelope.bytes,
                    )
                    .map_err(|error| action_range_validation_error(error.code))?;
                    if let Some(symbol) = symbol {
                        validate_action_response_symbol(&envelope.bytes, symbol)?;
                    }
                }
                envelopes.extend(pages);
            }
        }

        validate_action_range_envelopes(scope, start, end, &envelopes)?;
        Ok(envelopes)
    }

    /// Fetches a bounded historical daily-bar range as exact KIS wire bytes.
    ///
    /// This is intentionally a Raw-only capability.  The existing EOD path
    /// remains one target date per `FetchRequest`; callers that use this
    /// method must persist the result under
    /// [`crate::contract::PROVIDER_KIS_DAILY_RANGE`] and must not pass it to
    /// `normalize_kis_batch` until a range-aware canonical contract exists.
    /// No `chk-holiday`, current-price, account, or order request is made.
    pub async fn fetch_daily_bars_range(
        &self,
        market: &str,
        start: TradingDate,
        end: TradingDate,
        now: domain::UtcTimestamp,
        batch_id: BatchId,
    ) -> Result<Vec<RawEnvelope>, ProviderError> {
        if market != "kr" {
            return Err(ProviderError::InvalidConfiguration {
                detail: format!("KIS provider supports market kr, got {market:?}"),
            });
        }
        if end < start {
            return Err(ProviderError::InvalidConfiguration {
                detail: "historical daily-bar range end precedes start".to_owned(),
            });
        }

        let request = FetchRequest {
            market: market.to_owned(),
            date: start,
            kinds: vec![ResponseKind::Bars],
            now,
            batch_id,
        };
        let mut envelopes = Vec::new();
        for instrument in &self.instruments {
            // KIS returns the newest rows first.  Keep the requested start
            // fixed and move the next window's end to oldest_date - 1 day;
            // this avoids both gaps and broker continuation semantics (which
            // this endpoint explicitly does not support).
            let mut current_end = end;
            let mut seen_dates = BTreeSet::new();
            for window in 1..=MAX_DAILY_BAR_WINDOWS {
                if current_end < start {
                    break;
                }
                let start_text = start.to_iso().replace('-', "");
                let end_text = current_end.to_iso().replace('-', "");
                let query = vec![
                    ("FID_COND_MRKT_DIV_CODE".to_owned(), "J".to_owned()),
                    ("FID_INPUT_ISCD".to_owned(), instrument.symbol().to_owned()),
                    ("FID_INPUT_DATE_1".to_owned(), start_text.clone()),
                    ("FID_INPUT_DATE_2".to_owned(), end_text.clone()),
                    ("FID_PERIOD_DIV_CODE".to_owned(), "D".to_owned()),
                    // Preserve execution/original prices; adjustment is a
                    // separate curation concern and is never backfilled from
                    // current reference data.
                    ("FID_ORG_ADJ_PRC".to_owned(), "1".to_owned()),
                ];
                let label = format!("daily-bars-range-window-{window}");
                let before = envelopes.len();
                self.fetch_pages(
                    &request,
                    ResponseKind::Bars,
                    &label,
                    Some(instrument.symbol()),
                    DAILY_BARS_PATH,
                    DAILY_BARS_TR_ID,
                    query,
                    PaginationPolicy::StrictSinglePage,
                    &mut envelopes,
                )
                .await?;
                let envelope = envelopes.last().ok_or_else(|| {
                    pagination_error(
                        ResponseKind::Bars,
                        DAILY_BARS_PATH,
                        "KIS_DAILY_RANGE_EMPTY_RESPONSE",
                        "daily-bars range request returned no envelope",
                    )
                })?;
                debug_assert_eq!(envelopes.len(), before + 1);
                let summary = validate_daily_bars_range_envelope(
                    envelope,
                    instrument.symbol(),
                    start,
                    current_end,
                )?;
                for date in summary.dates {
                    if !seen_dates.insert(date) {
                        return Err(range_validation_error(
                            "KIS_DAILY_RANGE_OVERLAP",
                            "daily-bars windows overlap on a business date",
                        ));
                    }
                }
                if summary.row_count < MAX_DAILY_BAR_OBSERVATIONS || summary.oldest <= Some(start) {
                    break;
                }
                let oldest = summary.oldest.ok_or_else(|| {
                    range_validation_error(
                        "KIS_DAILY_RANGE_WINDOW_PROGRESS",
                        "full daily-bars window had no oldest date",
                    )
                })?;
                current_end = oldest.previous_day();
                if window == MAX_DAILY_BAR_WINDOWS {
                    return Err(pagination_error(
                        ResponseKind::Bars,
                        DAILY_BARS_PATH,
                        "KIS_DAILY_RANGE_WINDOW_LIMIT",
                        "daily-bars range exceeded the bounded window limit",
                    ));
                }
            }
        }
        Ok(envelopes)
    }

    async fn fetch_calendar_with_snapshot(
        &self,
        req: &FetchRequest,
        query: Vec<(String, String)>,
        output: &mut Vec<RawEnvelope>,
    ) -> Result<(), ProviderError> {
        let cached = self
            .calendar_snapshot
            .lock()
            .map_err(|_| {
                calendar_snapshot_error("KIS_CALENDAR_SNAPSHOT_LOCK", "snapshot lock poisoned")
            })?
            .clone();
        if let Some(snapshot) = cached {
            return self.push_cached_calendar(req, &snapshot, output);
        }

        // The range provider is deliberately driven sequentially.  Do not
        // add continuation handling or a retry loop around this endpoint.
        let mut fetched = Vec::new();
        self.fetch_pages(
            req,
            ResponseKind::Calendar,
            "calendar",
            None,
            CALENDAR_PATH,
            CALENDAR_TR_ID,
            query,
            PaginationPolicy::SinglePage,
            &mut fetched,
        )
        .await?;
        let envelope = fetched.pop().ok_or_else(|| {
            calendar_snapshot_error(
                "KIS_CALENDAR_SNAPSHOT_EMPTY",
                "chk-holiday returned no calendar envelope",
            )
        })?;
        let snapshot = CalendarSnapshot::from_envelope(&envelope)?;
        if !snapshot.covered_dates.contains_key(&req.date) {
            return Err(calendar_snapshot_miss(req.date));
        }
        let mut guard = self.calendar_snapshot.lock().map_err(|_| {
            calendar_snapshot_error("KIS_CALENDAR_SNAPSHOT_LOCK", "snapshot lock poisoned")
        })?;
        if guard.is_none() {
            *guard = Some(snapshot);
        }
        output.push(envelope);
        Ok(())
    }

    fn push_cached_calendar(
        &self,
        req: &FetchRequest,
        snapshot: &CalendarSnapshot,
        output: &mut Vec<RawEnvelope>,
    ) -> Result<(), ProviderError> {
        if !snapshot.covered_dates.contains_key(&req.date) {
            return Err(calendar_snapshot_miss(req.date));
        }
        output.push(RawEnvelope::new(
            req.batch_id,
            ResponseKind::Calendar,
            "calendar-page-01.json",
            snapshot.bytes.clone(),
            snapshot.retrieved_at,
            snapshot.request.clone(),
        ));
        Ok(())
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
        query: Vec<(String, String)>,
        pagination: PaginationPolicy,
        output: &mut Vec<RawEnvelope>,
    ) -> Result<(), ProviderError> {
        let mut continuation: Option<String> = None;
        let mut previous_bodies: Vec<Vec<u8>> = Vec::new();
        let mut pages = Vec::new();

        for page in 1..=KSD_MAX_PAGES {
            // The official KSD sample keeps CTS unchanged (blank for this
            // adapter) and changes only the request continuation header to
            // `N` after an `M` response. Do not carry broker CTS values.
            let sent_query = query.clone();
            let request_continuation = continuation.clone();
            let reply = self
                .reader
                .get(path, tr_id, &sent_query, request_continuation.as_deref())
                .await
                .map_err(|error| remote_error(kind, path, error))?;
            let MarketDataReply {
                body,
                continuation: marker,
            } = reply;

            if pagination == PaginationPolicy::StrictSinglePage
                && (marker.as_deref().is_some_and(|value| !value.is_empty())
                    || body_has_continuation_marker(&body))
            {
                return Err(pagination_error(
                    kind,
                    path,
                    "BROKER_PAGINATION_UNSUPPORTED",
                    "strict single-page response carried continuation metadata",
                ));
            }

            // A repeated opaque body is the broker returning the same page
            // again. Stop before retaining either page so a bad continuation
            // cannot become visible as a duplicate Raw response.
            if previous_bodies.iter().any(|previous| previous == &body) {
                return Err(pagination_error(
                    kind,
                    path,
                    "BROKER_PAGINATION_STALLED",
                    "KSD continuation returned repeated response bytes",
                ));
            }

            let file_name = match symbol {
                Some(symbol) => format!("{label}-{symbol}-page-{page:02}.json"),
                None => format!("{label}-page-{page:02}.json"),
            };
            pages.push(RawEnvelope::new(
                req.batch_id,
                kind,
                file_name,
                body.clone(),
                req.now,
                RequestMetadata {
                    endpoint: path.to_owned(),
                    query: sent_query,
                    headers: vec![
                        ("authorization".to_owned(), "[REDACTED]".to_owned()),
                        ("appkey".to_owned(), "[REDACTED]".to_owned()),
                        ("appsecret".to_owned(), "[REDACTED]".to_owned()),
                        ("tr_id".to_owned(), tr_id.to_owned()),
                        (
                            "tr_cont".to_owned(),
                            request_continuation.unwrap_or_default(),
                        ),
                    ],
                    mode: FetchMode::Credentialed,
                },
            ));

            let follows =
                pagination == PaginationPolicy::KsdGithubSample && marker.as_deref() == Some("M");
            if !follows {
                output.extend(pages);
                return Ok(());
            }

            previous_bodies.push(body);
            if page == KSD_MAX_PAGES {
                return Err(pagination_error(
                    kind,
                    path,
                    "BROKER_PAGINATION_LIMIT",
                    "KSD continuation exceeded the bounded page limit",
                ));
            }
            continuation = Some("N".to_owned());
        }

        unreachable!("the bounded KSD pagination loop returns on every page")
    }
}

impl CalendarSnapshot {
    fn from_envelope(envelope: &RawEnvelope) -> Result<Self, ProviderError> {
        validate_kis_response(ResponseKind::Calendar, CALENDAR_PATH, &envelope.bytes)
            .map_err(|error| calendar_snapshot_error(error.code, &error.reason))?;
        let document: Value = serde_json::from_slice(&envelope.bytes).map_err(|_| {
            calendar_snapshot_error(
                "KIS_CALENDAR_SNAPSHOT_SCHEMA",
                "chk-holiday response was not valid JSON",
            )
        })?;
        let output = document.get("output").ok_or_else(|| {
            calendar_snapshot_error(
                "KIS_CALENDAR_SNAPSHOT_SCHEMA",
                "chk-holiday response has no output",
            )
        })?;
        let rows = match output {
            Value::Array(rows) => rows.iter().collect::<Vec<_>>(),
            Value::Object(_) => vec![output],
            _ => {
                return Err(calendar_snapshot_error(
                    "KIS_CALENDAR_SNAPSHOT_SCHEMA",
                    "chk-holiday output is not an object or array",
                ));
            }
        };
        let mut covered_dates = BTreeMap::new();
        for row in rows {
            let object = row.as_object().ok_or_else(|| {
                calendar_snapshot_error(
                    "KIS_CALENDAR_SNAPSHOT_SCHEMA",
                    "chk-holiday output row is not an object",
                )
            })?;
            let date = object
                .get("bass_dt")
                .and_then(Value::as_str)
                .and_then(parse_kis_date)
                .ok_or_else(|| {
                    calendar_snapshot_error(
                        "KIS_CALENDAR_SNAPSHOT_SCHEMA",
                        "chk-holiday output row has an invalid bass_dt",
                    )
                })?;
            let is_open = match object.get("opnd_yn").and_then(Value::as_str) {
                Some("Y") => true,
                Some("N") => false,
                _ => {
                    return Err(calendar_snapshot_error(
                        "KIS_CALENDAR_SNAPSHOT_SCHEMA",
                        "chk-holiday output row has an invalid opnd_yn",
                    ));
                }
            };
            if let Some(previous) = covered_dates.insert(date, is_open)
                && previous != is_open
            {
                return Err(calendar_snapshot_error(
                    "KIS_CALENDAR_SNAPSHOT_SCHEMA",
                    "chk-holiday output contains conflicting duplicate dates",
                ));
            }
        }
        Ok(Self {
            bytes: envelope.bytes.clone(),
            request: envelope.request.clone(),
            retrieved_at: envelope.retrieved_at,
            covered_dates,
        })
    }
}

fn parse_kis_date(value: &str) -> Option<TradingDate> {
    if value.len() != 8 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    TradingDate::parse(&format!("{}-{}-{}", &value[..4], &value[4..6], &value[6..])).ok()
}

struct DailyBarsRangePage {
    row_count: usize,
    oldest: Option<TradingDate>,
    dates: BTreeSet<TradingDate>,
}

fn validate_daily_bars_range_envelope(
    envelope: &RawEnvelope,
    symbol: &str,
    start: TradingDate,
    end: TradingDate,
) -> Result<DailyBarsRangePage, ProviderError> {
    validate_kis_response(ResponseKind::Bars, DAILY_BARS_PATH, &envelope.bytes).map_err(
        |error| range_validation_error(error.code, "daily-bars response schema invalid"),
    )?;
    let document: Value = serde_json::from_slice(&envelope.bytes)
        .map_err(|_| range_validation_error("KIS_DAILY_RANGE_SCHEMA", "daily-bars JSON invalid"))?;
    let output1 = document
        .get("output1")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            range_validation_error("KIS_DAILY_RANGE_SCHEMA", "output1 is not an object")
        })?;
    let returned_symbol = output1
        .get("stck_shrn_iscd")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            range_validation_error("KIS_DAILY_RANGE_SCHEMA", "output1 has no stck_shrn_iscd")
        })?;
    if returned_symbol != symbol {
        return Err(range_validation_error(
            "KIS_DAILY_RANGE_SYMBOL_MISMATCH",
            "daily-bars output1 symbol differs from requested symbol",
        ));
    }
    let rows = document
        .get("output2")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            range_validation_error("KIS_DAILY_RANGE_SCHEMA", "output2 is not an array")
        })?;
    if rows.len() > MAX_DAILY_BAR_OBSERVATIONS {
        return Err(range_validation_error(
            "KIS_DAILY_RANGE_PAGE_LIMIT",
            "daily-bars response exceeded the documented 100-observation limit",
        ));
    }
    let mut dates = BTreeSet::new();
    let mut previous_date = None;
    for row in rows {
        let object = row.as_object().ok_or_else(|| {
            range_validation_error("KIS_DAILY_RANGE_SCHEMA", "output2 row is not an object")
        })?;
        let date_text = object
            .get("stck_bsop_date")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                range_validation_error(
                    "KIS_DAILY_RANGE_SCHEMA",
                    "output2 row has no stck_bsop_date",
                )
            })?;
        let date = parse_kis_date(date_text).ok_or_else(|| {
            range_validation_error("KIS_DAILY_RANGE_DATE_INVALID", "output2 date is invalid")
        })?;
        if date < start || date > end {
            return Err(range_validation_error(
                "KIS_DAILY_RANGE_DATE_OUT_OF_SCOPE",
                "output2 date is outside the requested range",
            ));
        }
        if let Some(previous) = previous_date
            && date >= previous
        {
            return Err(range_validation_error(
                "KIS_DAILY_RANGE_OVERLAP",
                "output2 dates are not strictly newest-to-oldest",
            ));
        }
        previous_date = Some(date);
        if !dates.insert(date) {
            return Err(range_validation_error(
                "KIS_DAILY_RANGE_OVERLAP",
                "output2 contains an overlapping business date",
            ));
        }
    }
    Ok(DailyBarsRangePage {
        row_count: dates.len(),
        oldest: dates.iter().next().copied(),
        dates,
    })
}

/// FHKST03010100 is accepted only as one terminal response.  The transport
/// header is checked above; this body scan rejects the common KIS cursor
/// spellings as well, without treating arbitrary output2 row fields as
/// pagination metadata. Empty/missing markers are terminal and allowed.
fn body_has_continuation_marker(bytes: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return true;
    };
    let Some(object) = value.as_object() else {
        return true;
    };
    object.iter().any(|(key, value)| {
        let normalized = key.to_ascii_lowercase();
        let looks_like_cursor = normalized.contains("ctx")
            || normalized.contains("cts")
            || normalized.contains("continu")
            || normalized == "next"
            || normalized == "has_more"
            || normalized == "more";
        looks_like_cursor
            && match value {
                Value::Null => false,
                Value::String(text) => !text.is_empty(),
                Value::Bool(flag) => *flag,
                Value::Number(number) => number.as_u64().is_none_or(|value| value != 0),
                Value::Array(items) => !items.is_empty(),
                Value::Object(map) => !map.is_empty(),
            }
    })
}

fn range_validation_error(code: &'static str, detail: &'static str) -> ProviderError {
    ProviderError::Remote {
        provider: PROVIDER_KIS,
        kind: ResponseKind::Bars,
        code,
        retryable: false,
        diagnostic: Some(RemoteDiagnostic {
            endpoint: DAILY_BARS_PATH.to_owned(),
            http_status: None,
        }),
        detail: detail.to_owned(),
    }
}

fn calendar_snapshot_miss(date: TradingDate) -> ProviderError {
    calendar_snapshot_error(
        "KIS_CALENDAR_SNAPSHOT_MISS",
        &format!(
            "chk-holiday snapshot does not cover target date {}",
            date.to_iso()
        ),
    )
}

fn calendar_snapshot_error(code: &'static str, detail: &str) -> ProviderError {
    ProviderError::Remote {
        provider: PROVIDER_KIS,
        kind: ResponseKind::Calendar,
        code,
        retryable: false,
        diagnostic: Some(RemoteDiagnostic {
            endpoint: CALENDAR_PATH.to_owned(),
            http_status: None,
        }),
        detail: detail.to_owned(),
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
        // `KisError::Broker` and schema errors can carry redacted broker
        // prose. Keep that detail out of the provider error because this
        // error may cross a worker boundary or be rendered by an operator.
        detail: error.code().to_owned(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KisActionSpec {
    pub(crate) kind: &'static str,
    pub(crate) label: &'static str,
    pub(crate) path: &'static str,
    pub(crate) tr_id: &'static str,
    pub(crate) extra: &'static [(&'static str, &'static str)],
}

pub(crate) const KIS_ACTION_SPECS: [KisActionSpec; KIS_ACTION_CLASS_COUNT] = [
    KisActionSpec {
        kind: "paidin-subscription",
        label: "corporate-actions-paidin-subscription",
        path: "/uapi/domestic-stock/v1/ksdinfo/paidin-capin",
        tr_id: "HHKDB669100C0",
        extra: &[("GB1", "1")],
    },
    KisActionSpec {
        kind: "paidin-record",
        label: "corporate-actions-paidin-record",
        path: "/uapi/domestic-stock/v1/ksdinfo/paidin-capin",
        tr_id: "HHKDB669100C0",
        extra: &[("GB1", "2")],
    },
    KisActionSpec {
        kind: "bonus-issue",
        label: "corporate-actions-bonus",
        path: "/uapi/domestic-stock/v1/ksdinfo/bonus-issue",
        tr_id: "HHKDB669101C0",
        extra: &[],
    },
    KisActionSpec {
        kind: "dividend",
        label: "corporate-actions-dividend",
        path: "/uapi/domestic-stock/v1/ksdinfo/dividend",
        tr_id: "HHKDB669102C0",
        extra: &[("GB1", "0"), ("HIGH_GB", "")],
    },
    KisActionSpec {
        kind: "merger-split",
        label: "corporate-actions-merger-split",
        path: "/uapi/domestic-stock/v1/ksdinfo/merger-split",
        tr_id: "HHKDB669104C0",
        extra: &[],
    },
    KisActionSpec {
        kind: "reverse-split",
        label: "corporate-actions-reverse-split",
        path: "/uapi/domestic-stock/v1/ksdinfo/rev-split",
        tr_id: "HHKDB669105C0",
        extra: &[("MARKET_GB", "0")],
    },
    KisActionSpec {
        kind: "capital-decrease",
        label: "corporate-actions-capital-decrease",
        path: "/uapi/domestic-stock/v1/ksdinfo/cap-dcrs",
        tr_id: "HHKDB669106C0",
        extra: &[],
    },
];

pub(crate) fn kis_action_spec(kind: &str) -> Option<KisActionSpec> {
    KIS_ACTION_SPECS
        .iter()
        .copied()
        .find(|spec| spec.kind == kind)
}

#[derive(Debug)]
struct EndpointSpec {
    label: &'static str,
    path: &'static str,
    tr_id: &'static str,
    query: Vec<(String, String)>,
}

fn corporate_action_endpoints(start: &str, end: &str, symbol: Option<&str>) -> Vec<EndpointSpec> {
    let common = |extra: &[(&str, &str)]| {
        let mut query = vec![
            ("CTS".to_owned(), String::new()),
            ("F_DT".to_owned(), start.to_owned()),
            ("T_DT".to_owned(), end.to_owned()),
            ("SHT_CD".to_owned(), symbol.unwrap_or_default().to_owned()),
        ];
        query.extend(
            extra
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned())),
        );
        query
    };
    KIS_ACTION_SPECS
        .iter()
        .map(|spec| EndpointSpec {
            label: spec.label,
            path: spec.path,
            tr_id: spec.tr_id,
            query: common(spec.extra),
        })
        .collect()
}

fn kis_date_text(date: TradingDate) -> String {
    date.to_iso().replace('-', "")
}

fn action_range_validation_error(code: &'static str) -> ProviderError {
    ProviderError::Remote {
        provider: PROVIDER_KIS,
        kind: ResponseKind::CorporateActions,
        code,
        retryable: false,
        diagnostic: None,
        detail: code.to_owned(),
    }
}

fn expected_action_query(
    spec: KisActionSpec,
    start: TradingDate,
    end: TradingDate,
    symbol: Option<&str>,
) -> Vec<(String, String)> {
    let mut query = vec![
        ("CTS".to_owned(), String::new()),
        ("F_DT".to_owned(), kis_date_text(start)),
        ("T_DT".to_owned(), kis_date_text(end)),
        ("SHT_CD".to_owned(), symbol.unwrap_or_default().to_owned()),
    ];
    query.extend(
        spec.extra
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned())),
    );
    query
}

fn expected_action_headers(spec: KisActionSpec, page: usize) -> Vec<(String, String)> {
    vec![
        ("authorization".to_owned(), "[REDACTED]".to_owned()),
        ("appkey".to_owned(), "[REDACTED]".to_owned()),
        ("appsecret".to_owned(), "[REDACTED]".to_owned()),
        ("tr_id".to_owned(), spec.tr_id.to_owned()),
        (
            "tr_cont".to_owned(),
            if page == 1 {
                String::new()
            } else {
                "N".to_owned()
            },
        ),
    ]
}

/// Validate the symbol identity carried by a KSD response when the endpoint
/// exposes one. Some KSD classes legitimately omit a short-code field, so a
/// missing identity is retained as unverifiable rather than fabricated; an
/// explicit conflicting identity fails closed.
fn validate_action_response_symbol(bytes: &[u8], expected: &str) -> Result<(), ProviderError> {
    let document: Value = serde_json::from_slice(bytes)
        .map_err(|_| action_range_validation_error("KIS_ACTION_RANGE_SCHEMA_INVALID"))?;
    let output1 = document
        .get("output1")
        .ok_or_else(|| action_range_validation_error("KIS_ACTION_RANGE_SCHEMA_INVALID"))?;
    let rows: Vec<&Value> = match output1 {
        Value::Array(rows) => rows.iter().collect(),
        Value::Object(_) => vec![output1],
        _ => {
            return Err(action_range_validation_error(
                "KIS_ACTION_RANGE_SCHEMA_INVALID",
            ));
        }
    };
    const SYMBOL_KEYS: [&str; 3] = ["sht_cd", "stck_shrn_iscd", "isu_srt_cd"];
    for row in rows {
        let object = row
            .as_object()
            .ok_or_else(|| action_range_validation_error("KIS_ACTION_RANGE_SCHEMA_INVALID"))?;
        for key in SYMBOL_KEYS {
            let Some((_, value)) = object
                .iter()
                .find(|(actual, _)| actual.eq_ignore_ascii_case(key))
            else {
                continue;
            };
            let Some(actual) = value.as_str() else {
                return Err(action_range_validation_error(
                    "KIS_ACTION_RANGE_SYMBOL_SHAPE_INVALID",
                ));
            };
            if !actual.is_empty() && actual != expected {
                return Err(action_range_validation_error(
                    "KIS_ACTION_RANGE_SYMBOL_MISMATCH",
                ));
            }
        }
    }
    Ok(())
}

/// Validate the complete request/file matrix before the caller can commit a
/// Raw batch. This rejects missing logical classes, unexpected classes,
/// duplicate file identities, altered common queries, and altered page
/// continuation metadata.
fn validate_action_range_envelopes(
    scope: KisActionRangeScope,
    start: TradingDate,
    end: TradingDate,
    envelopes: &[RawEnvelope],
) -> Result<(), ProviderError> {
    let symbols: Vec<Option<&str>> = match scope {
        KisActionRangeScope::WholeMarket => vec![None],
        KisActionRangeScope::FixedEtf11 => KR_ETF_CORE_SYMBOLS.iter().copied().map(Some).collect(),
    };
    let mut groups = BTreeSet::new();
    let mut file_names = BTreeSet::new();

    for envelope in envelopes {
        if envelope.kind != ResponseKind::CorporateActions
            || envelope.request.mode != FetchMode::Credentialed
        {
            return Err(action_range_validation_error(
                "KIS_ACTION_RANGE_UNEXPECTED_CLASS",
            ));
        }
        if !file_names.insert(envelope.file_name.clone()) {
            return Err(action_range_validation_error(
                "KIS_ACTION_RANGE_DUPLICATE_FILE",
            ));
        }

        let matches: Vec<(KisActionSpec, Option<&str>)> = symbols
            .iter()
            .flat_map(|symbol| {
                KIS_ACTION_SPECS.iter().copied().filter_map(move |spec| {
                    (envelope.request.endpoint == spec.path
                        && envelope.request.query
                            == expected_action_query(spec, start, end, *symbol))
                    .then_some((spec, *symbol))
                })
            })
            .collect();
        if matches.len() != 1 {
            return Err(action_range_validation_error(
                "KIS_ACTION_RANGE_REQUEST_CONTRACT_INVALID",
            ));
        }
        let (spec, symbol) = matches[0];
        if envelope.request.headers != expected_action_headers(spec, 1)
            && envelope.request.headers != expected_action_headers(spec, 2)
        {
            return Err(action_range_validation_error(
                "KIS_ACTION_RANGE_METADATA_INVALID",
            ));
        }
        let prefix = match symbol {
            Some(symbol) => format!("{}-{symbol}-page-", spec.label),
            None => format!("{}-page-", spec.label),
        };
        let Some(page_text) = envelope
            .file_name
            .strip_prefix(&prefix)
            .and_then(|name| name.strip_suffix(".json"))
        else {
            return Err(action_range_validation_error(
                "KIS_ACTION_RANGE_FILENAME_INVALID",
            ));
        };
        if page_text.len() != 2 || !page_text.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(action_range_validation_error(
                "KIS_ACTION_RANGE_FILENAME_INVALID",
            ));
        }
        let page = page_text
            .parse::<usize>()
            .ok()
            .filter(|page| (1..=KSD_MAX_PAGES).contains(page))
            .ok_or_else(|| action_range_validation_error("KIS_ACTION_RANGE_FILENAME_INVALID"))?;
        let continuation = envelope
            .request
            .headers
            .iter()
            .find(|(name, _)| name == "tr_cont")
            .map(|(_, value)| value.as_str());
        let expected_continuation = if page == 1 { "" } else { "N" };
        if continuation != Some(expected_continuation) {
            return Err(action_range_validation_error(
                "KIS_ACTION_RANGE_METADATA_INVALID",
            ));
        }
        if !groups.insert((spec.kind, symbol.unwrap_or_default(), page)) {
            return Err(action_range_validation_error(
                "KIS_ACTION_RANGE_DUPLICATE_PAGE",
            ));
        }
        validate_kis_response(ResponseKind::CorporateActions, spec.path, &envelope.bytes)
            .map_err(|error| action_range_validation_error(error.code))?;
    }

    let expected_groups = KIS_ACTION_SPECS
        .iter()
        .flat_map(|spec| {
            symbols
                .iter()
                .map(move |symbol| (spec.kind, symbol.unwrap_or_default()))
        })
        .collect::<BTreeSet<_>>();
    let actual_groups = groups
        .iter()
        .map(|(kind, symbol, _)| (*kind, *symbol))
        .collect::<BTreeSet<_>>();
    if actual_groups != expected_groups {
        return Err(action_range_validation_error("KIS_ACTION_RANGE_INCOMPLETE"));
    }
    Ok(())
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
    use std::collections::VecDeque;
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
        calendar_snapshot: bool,
    }

    #[derive(Debug)]
    struct RecordedCall {
        path: String,
        tr_id: String,
        query: Vec<(String, String)>,
        continuation: Option<String>,
    }

    impl FixtureReader {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                continuation_once: false,
                calendar_snapshot: false,
            }
        }

        fn with_calendar_snapshot() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                continuation_once: false,
                calendar_snapshot: true,
            }
        }

        fn with_one_continuation() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                continuation_once: true,
                calendar_snapshot: false,
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
            self.calls.lock().unwrap().push(RecordedCall {
                path: path.to_owned(),
                tr_id: tr_id.to_owned(),
                query: query.to_vec(),
                continuation: continuation.map(str::to_owned),
            });
            let marker = self.continuation_once.then(|| {
                if continuation.is_none() {
                    "M".to_owned()
                } else {
                    "F".to_owned()
                }
            });
            let body = match path {
                DAILY_BARS_PATH => br#"{"rt_cd":"0","output1":{},"output2":[]}"#.to_vec(),
                REFERENCE_PATH => br#"{"rt_cd":"0","output":{}}"#.to_vec(),
                CALENDAR_PATH if self.calendar_snapshot => {
                    let date = query
                        .iter()
                        .find(|(key, _)| key == "BASS_DT")
                        .map(|(_, value)| value.as_str())
                        .and_then(parse_kis_date)
                        .expect("fixture calendar date");
                    let next = date.next_day();
                    format!(
                        r#"{{"rt_cd":"0","ctx_area_fk":"next-fk","ctx_area_nk":"next-nk","output":[{{"bass_dt":"{}","opnd_yn":"Y"}},{{"bass_dt":"{}","opnd_yn":"N"}}]}}"#,
                        date.to_iso().replace('-', ""),
                        next.to_iso().replace('-', "")
                    )
                    .into_bytes()
                }
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
                continuation: marker,
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
    struct ScriptedReader {
        replies: Mutex<VecDeque<MarketDataReply>>,
        calls: Mutex<Vec<RecordedCall>>,
    }

    impl ScriptedReader {
        fn new(replies: impl IntoIterator<Item = MarketDataReply>) -> Self {
            Self {
                replies: Mutex::new(replies.into_iter().collect()),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl KisRead for ScriptedReader {
        async fn get(
            &self,
            path: &str,
            tr_id: &str,
            query: &[(String, String)],
            continuation: Option<&str>,
        ) -> Result<MarketDataReply, KisError> {
            self.calls.lock().unwrap().push(RecordedCall {
                path: path.to_owned(),
                tr_id: tr_id.to_owned(),
                query: query.to_vec(),
                continuation: continuation.map(str::to_owned),
            });
            self.replies
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| KisError::SchemaDrift {
                    endpoint: path.to_owned(),
                    detail: "fixture exhausted".to_owned(),
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
        assert!(!error.to_string().contains("fixture-secret"));
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
        request_at("2026-08-14", kinds)
    }

    fn request_at(date: &str, kinds: Vec<ResponseKind>) -> FetchRequest {
        FetchRequest {
            market: "kr".to_owned(),
            date: TradingDate::parse(date).unwrap(),
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
        let calls = provider.reader.calls.lock().unwrap();
        assert!(
            calls
                .iter()
                .filter(|call| { call.query.iter().any(|(key, _)| key == "CTS") })
                .all(|call| {
                    call.query
                        .iter()
                        .any(|(key, value)| key == "CTS" && value.is_empty())
                })
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
                .expect("marker-free KSD terminal pages are single-page success");

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
        // The XLSX labels chk-holiday as a non-paginated API, while generic
        // official samples show M/F continuation handling.  The endpoint
        // specific contract wins: retain the first response and never follow
        // the contradictory metadata.
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
    async fn range_calendar_snapshot_reuses_one_call_and_fails_closed_outside_coverage() {
        let provider = KisProvider::kr_etf_core(FixtureReader::with_calendar_snapshot())
            .with_calendar_snapshot_cache();
        let first = provider
            .fetch(&request_at("2026-08-14", vec![ResponseKind::Calendar]))
            .await
            .expect("first range calendar snapshot");
        let second = provider
            .fetch(&request_at("2026-08-15", vec![ResponseKind::Calendar]))
            .await
            .expect("covered date reuses range calendar snapshot");
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert_eq!(first[0].bytes, second[0].bytes);
        assert_eq!(first[0].request.query, second[0].request.query);

        let error = provider
            .fetch(&request_at("2026-08-16", vec![ResponseKind::Calendar]))
            .await
            .expect_err("range calendar miss must not issue a second call");
        assert!(matches!(
            error,
            ProviderError::Remote {
                kind: ResponseKind::Calendar,
                code: "KIS_CALENDAR_SNAPSHOT_MISS",
                retryable: false,
                ..
            }
        ));
        let calls = provider.reader.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
    }

    #[tokio::test]
    async fn bars_and_reference_ignore_continuation_like_metadata_by_contract() {
        let provider = KisProvider::kr_etf_core(FixtureReader::with_one_continuation());
        let fetched = provider
            .fetch(&request(vec![ResponseKind::Bars, ResponseKind::Reference]))
            .await
            .expect("single-page bars and reference");
        // The official repository's generic continuation helper must not
        // override the XLSX endpoint contract for these single-page reads.
        assert_eq!(fetched.len(), KR_ETF_CORE_SYMBOLS.len() * 2);
        let calls = provider.reader.calls.lock().unwrap();
        assert_eq!(calls.len(), KR_ETF_CORE_SYMBOLS.len() * 2);
        assert!(calls.iter().all(|call| call.continuation.is_none()));
    }

    #[tokio::test]
    async fn ksd_action_follows_exact_m_then_terminal_marker() {
        let reader = ScriptedReader::new([
            MarketDataReply {
                body: br#"{"rt_cd":"0","output1":[{"page":"one"}]}"#.to_vec(),
                continuation: Some("M".to_owned()),
            },
            MarketDataReply {
                body: br#"{"rt_cd":"0","output1":[{"page":"two"}]}"#.to_vec(),
                continuation: Some("F".to_owned()),
            },
        ]);
        let provider = KisProvider::kr_etf_core(reader);
        let mut output = Vec::new();
        provider
            .fetch_pages(
                &request(vec![ResponseKind::CorporateActions]),
                ResponseKind::CorporateActions,
                "corporate-actions-bonus",
                None,
                "/uapi/domestic-stock/v1/ksdinfo/bonus-issue",
                "HHKDB669101C0",
                vec![
                    ("CTS".to_owned(), String::new()),
                    ("F_DT".to_owned(), "20260818".to_owned()),
                    ("T_DT".to_owned(), "20260818".to_owned()),
                    ("SHT_CD".to_owned(), String::new()),
                ],
                PaginationPolicy::KsdGithubSample,
                &mut output,
            )
            .await
            .expect("official M then terminal flow");

        assert_eq!(
            output
                .iter()
                .map(|envelope| envelope.file_name.as_str())
                .collect::<Vec<_>>(),
            vec![
                "corporate-actions-bonus-page-01.json",
                "corporate-actions-bonus-page-02.json"
            ]
        );
        let calls = provider.reader.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].path, "/uapi/domestic-stock/v1/ksdinfo/bonus-issue");
        assert_eq!(calls[0].tr_id, "HHKDB669101C0");
        assert_eq!(calls[0].continuation, None);
        assert_eq!(calls[1].continuation.as_deref(), Some("N"));
        assert_eq!(calls[0].query, calls[1].query);
        assert_eq!(
            calls[0].query,
            vec![
                ("CTS".to_owned(), String::new()),
                ("F_DT".to_owned(), "20260818".to_owned()),
                ("T_DT".to_owned(), "20260818".to_owned()),
                ("SHT_CD".to_owned(), String::new()),
            ]
        );
        assert_eq!(
            output[0]
                .request
                .headers
                .iter()
                .find(|(name, _)| name == "tr_cont")
                .map(|(_, value)| value.as_str()),
            Some("")
        );
        assert_eq!(
            output[1]
                .request
                .headers
                .iter()
                .find(|(name, _)| name == "tr_cont")
                .map(|(_, value)| value.as_str()),
            Some("N")
        );
    }

    #[tokio::test]
    async fn ksd_action_repeated_response_bytes_fail_closed_without_partial_output() {
        let body = br#"{"rt_cd":"0","output1":[{"page":"same"}]}"#.to_vec();
        let reader = ScriptedReader::new([
            MarketDataReply {
                body: body.clone(),
                continuation: Some("M".to_owned()),
            },
            MarketDataReply {
                body,
                continuation: Some("M".to_owned()),
            },
        ]);
        let provider = KisProvider::kr_etf_core(reader);
        let mut output = Vec::new();
        let error = provider
            .fetch_pages(
                &request(vec![ResponseKind::CorporateActions]),
                ResponseKind::CorporateActions,
                "fixture",
                None,
                "/uapi/domestic-stock/v1/ksdinfo/bonus-issue",
                "HHKDB669101C0",
                vec![("CTS".to_owned(), String::new())],
                PaginationPolicy::KsdGithubSample,
                &mut output,
            )
            .await
            .expect_err("repeated continuation payload must stop");
        assert!(matches!(
            error,
            ProviderError::Remote {
                code: "BROKER_PAGINATION_STALLED",
                ..
            }
        ));
        assert!(output.is_empty());
        assert_eq!(provider.reader.calls.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn ksd_action_page_cap_fails_closed_without_partial_output() {
        let reader = ScriptedReader::new((0..KSD_MAX_PAGES).map(|page| MarketDataReply {
            body: format!(r#"{{"rt_cd":"0","output1":[{{"page":"{page}"}}]}}"#).into_bytes(),
            continuation: Some("M".to_owned()),
        }));
        let provider = KisProvider::kr_etf_core(reader);
        let mut output = Vec::new();
        let error = provider
            .fetch_pages(
                &request(vec![ResponseKind::CorporateActions]),
                ResponseKind::CorporateActions,
                "fixture",
                None,
                "/uapi/domestic-stock/v1/ksdinfo/bonus-issue",
                "HHKDB669101C0",
                vec![("CTS".to_owned(), String::new())],
                PaginationPolicy::KsdGithubSample,
                &mut output,
            )
            .await
            .expect_err("continuation must not exceed the page cap");
        assert!(matches!(
            error,
            ProviderError::Remote {
                code: "BROKER_PAGINATION_LIMIT",
                ..
            }
        ));
        assert!(output.is_empty());
        assert_eq!(provider.reader.calls.lock().unwrap().len(), KSD_MAX_PAGES);
    }

    #[tokio::test]
    async fn ksd_action_non_m_markers_are_terminal_without_other_endpoint_calls() {
        for marker in ["F", "unknown"] {
            let reader = ScriptedReader::new([MarketDataReply {
                body: br#"{"rt_cd":"0","output1":[]}"#.to_vec(),
                continuation: Some(marker.to_owned()),
            }]);
            let provider = KisProvider::kr_etf_core(reader);
            let mut output = Vec::new();
            provider
                .fetch_pages(
                    &request(vec![ResponseKind::CorporateActions]),
                    ResponseKind::CorporateActions,
                    "fixture",
                    None,
                    "/uapi/domestic-stock/v1/ksdinfo/paidin-capin",
                    "HHKDB669100C0",
                    vec![
                        ("CTS".to_owned(), String::new()),
                        ("GB1".to_owned(), "1".to_owned()),
                    ],
                    PaginationPolicy::KsdGithubSample,
                    &mut output,
                )
                .await
                .expect("non-M is terminal per exact GitHub sample");
            assert_eq!(output.len(), 1);
            let calls = provider.reader.calls.lock().unwrap();
            assert_eq!(calls.len(), 1);
            assert_eq!(
                calls[0].path,
                "/uapi/domestic-stock/v1/ksdinfo/paidin-capin"
            );
            assert_eq!(calls[0].tr_id, "HHKDB669100C0");
            assert!(calls[0].continuation.is_none());
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
