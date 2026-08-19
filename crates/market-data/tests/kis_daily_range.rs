use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use domain::{BatchId, ContentHash, TradingDate, UtcTimestamp};
use kis_client::{KisError, MarketDataReply};
use market_data::contract::{FetchMode, MARKET_KR, PROVIDER_KIS_DAILY_RANGE, ResponseKind};
use market_data::providers::kis::{KR_ETF_CORE_SYMBOLS, KisProvider, KisRead};
use market_data::storage::RawStore;
use market_data::{MAX_DAILY_BAR_OBSERVATIONS, MAX_DAILY_BAR_WINDOWS, ingest_kis_daily_bars_range};
use serde_json::json;

const DAILY_PATH: &str = "/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice";
const DAILY_TR_ID: &str = "FHKST03010100";
const NOW: &str = "2026-08-14T08:00:00Z";

#[derive(Debug, Clone)]
struct Call {
    path: String,
    tr_id: String,
    query: Vec<(String, String)>,
    continuation: Option<String>,
}

#[derive(Debug, Clone)]
struct FixtureReader {
    calls: Arc<Mutex<Vec<Call>>>,
    out_of_scope_first_page: bool,
    first_page_rows: Option<usize>,
    reverse_first_page: bool,
    duplicate_first_page: bool,
}

impl FixtureReader {
    fn new(out_of_scope_first_page: bool) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            out_of_scope_first_page,
            first_page_rows: None,
            reverse_first_page: false,
            duplicate_first_page: false,
        }
    }

    fn with_first_page_rows(rows: usize) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            out_of_scope_first_page: false,
            first_page_rows: Some(rows),
            reverse_first_page: false,
            duplicate_first_page: false,
        }
    }

    fn with_reversed_first_page() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            out_of_scope_first_page: false,
            first_page_rows: Some(MAX_DAILY_BAR_OBSERVATIONS),
            reverse_first_page: true,
            duplicate_first_page: false,
        }
    }

    fn with_duplicate_first_page() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            out_of_scope_first_page: false,
            first_page_rows: None,
            reverse_first_page: false,
            duplicate_first_page: true,
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
        let call_index = self.calls.lock().expect("calls lock").len();
        self.calls.lock().expect("calls lock").push(Call {
            path: path.to_owned(),
            tr_id: tr_id.to_owned(),
            query: query.to_vec(),
            continuation: continuation.map(str::to_owned),
        });
        let symbol = query_value(query, "FID_INPUT_ISCD");
        let start = TradingDate::parse_digits(query_value(query, "FID_INPUT_DATE_1"))?;
        let end = TradingDate::parse_digits(query_value(query, "FID_INPUT_DATE_2"))?;
        let rows = if self.out_of_scope_first_page && call_index == 0 {
            vec![TradingDate::parse("2021-01-01").expect("fixture date")]
        } else if self.duplicate_first_page && call_index == 0 {
            vec![end, end]
        } else if let Some(first_page_rows) = self.first_page_rows
            && (end == TradingDate::parse("2020-04-10").expect("fixture date")
                || end == TradingDate::parse("2020-04-09").expect("fixture date"))
        {
            (0..first_page_rows)
                .map(|offset| {
                    end.checked_add_days(-(offset as i64))
                        .map_err(|_| KisError::SchemaDrift {
                            endpoint: path.to_owned(),
                            detail: "fixture date generation overflowed".to_owned(),
                        })
                })
                .collect::<Result<Vec<_>, _>>()?
        } else if end == TradingDate::parse("2020-04-10").expect("fixture date") {
            (0..MAX_DAILY_BAR_OBSERVATIONS)
                .map(|offset| {
                    end.checked_add_days(-(offset as i64))
                        .map_err(|_| KisError::SchemaDrift {
                            endpoint: path.to_owned(),
                            detail: "fixture date generation overflowed".to_owned(),
                        })
                })
                .collect::<Result<Vec<_>, _>>()?
        } else {
            vec![end]
        };
        let rows = if self.reverse_first_page && call_index == 0 {
            rows.into_iter().rev().collect()
        } else {
            rows
        };
        if !self.out_of_scope_first_page && rows.iter().any(|date| *date < start || *date > end) {
            return Err(KisError::SchemaDrift {
                endpoint: path.to_owned(),
                detail: "fixture date generation escaped query bounds".to_owned(),
            });
        }
        Ok(MarketDataReply {
            body: daily_body(symbol, &rows),
            continuation: Some("F".to_owned()),
        })
    }
}

fn query_value<'a>(query: &'a [(String, String)], key: &str) -> &'a str {
    query
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
        .expect("query field")
}

trait ParseDigits {
    fn parse_digits(value: &str) -> Result<TradingDate, KisError>;
}

impl ParseDigits for TradingDate {
    fn parse_digits(value: &str) -> Result<TradingDate, KisError> {
        if value.len() != 8 {
            return Err(KisError::SchemaDrift {
                endpoint: DAILY_PATH.to_owned(),
                detail: "fixture date is not YYYYMMDD".to_owned(),
            });
        }
        TradingDate::parse(&format!("{}-{}-{}", &value[..4], &value[4..6], &value[6..])).map_err(
            |_| KisError::SchemaDrift {
                endpoint: DAILY_PATH.to_owned(),
                detail: "fixture date is invalid".to_owned(),
            },
        )
    }
}

fn daily_body(symbol: &str, dates: &[TradingDate]) -> Vec<u8> {
    let rows = dates
        .iter()
        .map(|date| {
            json!({
                "stck_bsop_date": date.to_iso().replace('-', ""),
                "stck_clpr": "100",
                "stck_oprc": "99",
                "stck_hgpr": "101",
                "stck_lwpr": "98",
                "acml_vol": "100",
                "acml_tr_pbmn": "10000"
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_vec(&json!({
        "rt_cd": "0",
        "msg_cd": "MCA00000",
        "msg1": "",
        "output1": {"stck_shrn_iscd": symbol},
        "output2": rows
    }))
    .expect("fixture JSON")
}

#[tokio::test]
async fn range_uses_oldest_date_minus_one_without_tr_continuation() {
    let reader = FixtureReader::new(false);
    let calls = reader.calls.clone();
    let provider = KisProvider::kr_etf_core(reader);
    let fetched = provider
        .fetch_daily_bars_range(
            MARKET_KR,
            TradingDate::parse("2020-01-01").expect("start"),
            TradingDate::parse("2020-04-10").expect("end"),
            UtcTimestamp::parse_rfc3339(NOW).expect("timestamp"),
            BatchId::generate(),
        )
        .await
        .expect("range fetch");

    assert_eq!(fetched.len(), KR_ETF_CORE_SYMBOLS.len() * 2);
    let calls = calls.lock().expect("calls lock");
    assert_eq!(calls.len(), KR_ETF_CORE_SYMBOLS.len() * 2);
    assert!(calls.iter().all(|call| call.path == DAILY_PATH));
    assert!(calls.iter().all(|call| call.tr_id == DAILY_TR_ID));
    assert!(calls.iter().all(|call| call.continuation.is_none()));
    assert_eq!(query_value(&calls[0].query, "FID_INPUT_DATE_1"), "20200101");
    assert_eq!(query_value(&calls[0].query, "FID_INPUT_DATE_2"), "20200410");
    assert_eq!(
        query_value(&calls[11].query, "FID_INPUT_DATE_1"),
        "20200101"
    );
    assert_eq!(
        query_value(&calls[11].query, "FID_INPUT_DATE_2"),
        "20200101"
    );
    assert_eq!(MAX_DAILY_BAR_OBSERVATIONS, 100);
    assert_eq!(MAX_DAILY_BAR_WINDOWS, 1_024);
}

#[tokio::test]
async fn range_stops_after_99_observations() {
    let reader = FixtureReader::with_first_page_rows(99);
    let calls = reader.calls.clone();
    let provider = KisProvider::kr_etf_core(reader);
    let fetched = provider
        .fetch_daily_bars_range(
            MARKET_KR,
            TradingDate::parse("2020-01-01").expect("start"),
            TradingDate::parse("2020-04-10").expect("end"),
            UtcTimestamp::parse_rfc3339(NOW).expect("timestamp"),
            BatchId::generate(),
        )
        .await
        .expect("range fetch");
    assert_eq!(fetched.len(), KR_ETF_CORE_SYMBOLS.len());
    assert_eq!(
        calls.lock().expect("calls lock").len(),
        KR_ETF_CORE_SYMBOLS.len()
    );
}

#[tokio::test]
async fn range_stops_after_100_observations_when_oldest_reaches_start() {
    let reader = FixtureReader::with_first_page_rows(100);
    let calls = reader.calls.clone();
    let provider = KisProvider::kr_etf_core(reader);
    let fetched = provider
        .fetch_daily_bars_range(
            MARKET_KR,
            TradingDate::parse("2020-01-01").expect("start"),
            TradingDate::parse("2020-04-09").expect("end"),
            UtcTimestamp::parse_rfc3339(NOW).expect("timestamp"),
            BatchId::generate(),
        )
        .await
        .expect("range fetch");
    assert_eq!(fetched.len(), KR_ETF_CORE_SYMBOLS.len());
    assert_eq!(
        calls.lock().expect("calls lock").len(),
        KR_ETF_CORE_SYMBOLS.len()
    );
}

#[tokio::test]
async fn range_rejects_101_observations_from_one_response() {
    let provider = KisProvider::kr_etf_core(FixtureReader::with_first_page_rows(101));
    let error = provider
        .fetch_daily_bars_range(
            MARKET_KR,
            TradingDate::parse("2020-01-01").expect("start"),
            TradingDate::parse("2020-04-10").expect("end"),
            UtcTimestamp::parse_rfc3339(NOW).expect("timestamp"),
            BatchId::generate(),
        )
        .await
        .expect_err("more than 100 observations must fail closed");
    assert!(error.to_string().contains("KIS_DAILY_RANGE_PAGE_LIMIT"));
}

#[tokio::test]
async fn range_rejects_reversed_output2_order() {
    let provider = KisProvider::kr_etf_core(FixtureReader::with_reversed_first_page());
    let error = provider
        .fetch_daily_bars_range(
            MARKET_KR,
            TradingDate::parse("2020-01-01").expect("start"),
            TradingDate::parse("2020-04-10").expect("end"),
            UtcTimestamp::parse_rfc3339(NOW).expect("timestamp"),
            BatchId::generate(),
        )
        .await
        .expect_err("reversed output2 must fail closed");
    assert!(error.to_string().contains("KIS_DAILY_RANGE_OVERLAP"));
}

#[tokio::test]
async fn range_rejects_overlapping_output2_dates() {
    let provider = KisProvider::kr_etf_core(FixtureReader::with_duplicate_first_page());
    let error = provider
        .fetch_daily_bars_range(
            MARKET_KR,
            TradingDate::parse("2020-01-01").expect("start"),
            TradingDate::parse("2020-01-31").expect("end"),
            UtcTimestamp::parse_rfc3339(NOW).expect("timestamp"),
            BatchId::generate(),
        )
        .await
        .expect_err("overlapping output2 must fail closed");
    assert!(error.to_string().contains("KIS_DAILY_RANGE_OVERLAP"));
}

#[tokio::test]
async fn range_rejects_a_response_date_outside_the_requested_window() {
    let provider = KisProvider::kr_etf_core(FixtureReader::new(true));
    let error = provider
        .fetch_daily_bars_range(
            MARKET_KR,
            TradingDate::parse("2020-01-01").expect("start"),
            TradingDate::parse("2020-01-31").expect("end"),
            UtcTimestamp::parse_rfc3339(NOW).expect("timestamp"),
            BatchId::generate(),
        )
        .await
        .expect_err("out-of-scope date must fail closed");
    assert!(
        error
            .to_string()
            .contains("KIS_DAILY_RANGE_DATE_OUT_OF_SCOPE")
    );
}

#[tokio::test]
async fn range_ingest_is_stored_in_a_separate_raw_scope() {
    let reader = FixtureReader::new(false);
    let calls = reader.calls.clone();
    let provider = KisProvider::kr_etf_core(reader);
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RawStore::new(temp.path());
    let outcome = ingest_kis_daily_bars_range(
        &store,
        &provider,
        MARKET_KR,
        TradingDate::parse("2020-01-01").expect("start"),
        TradingDate::parse("2020-01-31").expect("end"),
        UtcTimestamp::parse_rfc3339(NOW).expect("timestamp"),
        Some("fixture-entitlement"),
    )
    .await
    .expect("raw range ingest");

    assert_eq!(outcome.entry.provider, PROVIDER_KIS_DAILY_RANGE);
    assert_eq!(outcome.entry.market, MARKET_KR);
    assert_eq!(outcome.entry.mode, FetchMode::Credentialed);
    assert_eq!(outcome.entry.files.len(), KR_ETF_CORE_SYMBOLS.len());
    let manifest_entries = store
        .read_manifest(PROVIDER_KIS_DAILY_RANGE, MARKET_KR)
        .expect("read range manifest");
    assert_eq!(manifest_entries, vec![outcome.entry.clone()]);
    let manifest = &manifest_entries[0];
    let readback = store
        .read_batch_bytes(PROVIDER_KIS_DAILY_RANGE, MARKET_KR, manifest)
        .expect("read range evidence");
    assert_eq!(readback.len(), manifest.files.len());

    let mut names = HashSet::new();
    let mut symbols = HashSet::new();
    for file in &manifest.files {
        assert_eq!(file.kind, ResponseKind::Bars);
        assert!(names.insert(file.file_name.clone()));
        assert_eq!(file.request.endpoint, DAILY_PATH);
        assert_eq!(file.request.mode, FetchMode::Credentialed);
        assert_eq!(
            query_value(&file.request.query, "FID_INPUT_DATE_1"),
            "20200101"
        );
        assert_eq!(
            query_value(&file.request.query, "FID_INPUT_DATE_2"),
            "20200131"
        );
        assert_eq!(query_value(&file.request.query, "FID_PERIOD_DIV_CODE"), "D");
        assert_eq!(query_value(&file.request.query, "FID_ORG_ADJ_PRC"), "1");
        assert_eq!(
            query_value(&file.request.query, "FID_COND_MRKT_DIV_CODE"),
            "J"
        );
        let symbol = query_value(&file.request.query, "FID_INPUT_ISCD");
        assert!(KR_ETF_CORE_SYMBOLS.contains(&symbol));
        assert!(symbols.insert(symbol.to_owned()));
        assert_eq!(
            file.request
                .headers
                .iter()
                .find(|(key, _)| key == "tr_cont")
                .map(|(_, value)| value.as_str()),
            Some("")
        );
        let evidence = readback
            .iter()
            .find(|stored| stored.file_name == file.file_name)
            .expect("manifest evidence file");
        assert_eq!(evidence.bytes.len() as u64, file.size_bytes);
        assert_eq!(ContentHash::from_bytes(&evidence.bytes), file.content_hash);
    }
    assert_eq!(symbols.len(), KR_ETF_CORE_SYMBOLS.len());
    assert!(
        calls
            .lock()
            .expect("calls lock")
            .iter()
            .all(|call| call.continuation.is_none())
    );
}

#[tokio::test]
async fn range_rejects_reversed_dates_before_reader_access() {
    let reader = FixtureReader::new(false);
    let calls = reader.calls.clone();
    let provider = KisProvider::kr_etf_core(reader);
    let error = provider
        .fetch_daily_bars_range(
            MARKET_KR,
            TradingDate::parse("2020-02-01").expect("start"),
            TradingDate::parse("2020-01-31").expect("end"),
            UtcTimestamp::parse_rfc3339(NOW).expect("timestamp"),
            BatchId::generate(),
        )
        .await
        .expect_err("reversed range");
    assert!(error.to_string().contains("end precedes start"));
    assert!(calls.lock().expect("calls lock").is_empty());
}
