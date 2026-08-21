use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use domain::{BatchId, TradingDate, UtcTimestamp};
use kis_client::{KisError, MarketDataReply};
use market_data::contract::PROVIDER_KIS;
use market_data::ingest::ingest_kis_action_range;
use market_data::storage::RawStore;
use market_data::{
    KIS_ACTION_MAX_PAGES, KR_ETF_CORE_SYMBOLS, KisActionRangeScope, KisProvider, KisRead,
};

#[derive(Debug, Clone)]
struct Call {
    path: String,
    tr_id: String,
    query: Vec<(String, String)>,
    continuation: Option<String>,
}

#[derive(Debug)]
struct FakeReader {
    replies: Mutex<VecDeque<Result<MarketDataReply, KisError>>>,
    calls: Arc<Mutex<Vec<Call>>>,
}

impl FakeReader {
    fn new(
        replies: impl IntoIterator<Item = Result<MarketDataReply, KisError>>,
    ) -> (Self, Arc<Mutex<Vec<Call>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                replies: Mutex::new(replies.into_iter().collect()),
                calls: Arc::clone(&calls),
            },
            calls,
        )
    }
}

impl KisRead for FakeReader {
    async fn get(
        &self,
        path: &str,
        tr_id: &str,
        query: &[(String, String)],
        continuation: Option<&str>,
    ) -> Result<MarketDataReply, KisError> {
        self.calls.lock().unwrap().push(Call {
            path: path.to_owned(),
            tr_id: tr_id.to_owned(),
            query: query.to_vec(),
            continuation: continuation.map(str::to_owned),
        });
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Ok(terminal_body(0)))
    }
}

fn terminal_body(page: usize) -> MarketDataReply {
    MarketDataReply {
        body: format!(r#"{{"rt_cd":"0","output1":[],"fixture_page":{page}}}"#).into_bytes(),
        continuation: None,
    }
}

fn replies(count: usize) -> Vec<Result<MarketDataReply, KisError>> {
    (0..count).map(|page| Ok(terminal_body(page))).collect()
}

fn date(value: &str) -> TradingDate {
    TradingDate::parse(value).unwrap()
}

fn now() -> UtcTimestamp {
    UtcTimestamp::parse_rfc3339("2026-08-21T00:00:00Z").unwrap()
}

fn expected_query(
    start: &str,
    end: &str,
    symbol: &str,
    extra: &[(&str, &str)],
) -> Vec<(String, String)> {
    let mut query = vec![
        ("CTS".to_owned(), String::new()),
        ("F_DT".to_owned(), start.replace('-', "")),
        ("T_DT".to_owned(), end.replace('-', "")),
        ("SHT_CD".to_owned(), symbol.to_owned()),
    ];
    query.extend(
        extra
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned())),
    );
    query
}

type ActionQuery = &'static [(&'static str, &'static str)];
type ActionContract = (&'static str, &'static str, &'static str, ActionQuery);

const ACTIONS: [ActionContract; 7] = [
    (
        "/uapi/domestic-stock/v1/ksdinfo/paidin-capin",
        "HHKDB669100C0",
        "corporate-actions-paidin-subscription",
        &[("GB1", "1")],
    ),
    (
        "/uapi/domestic-stock/v1/ksdinfo/paidin-capin",
        "HHKDB669100C0",
        "corporate-actions-paidin-record",
        &[("GB1", "2")],
    ),
    (
        "/uapi/domestic-stock/v1/ksdinfo/bonus-issue",
        "HHKDB669101C0",
        "corporate-actions-bonus",
        &[],
    ),
    (
        "/uapi/domestic-stock/v1/ksdinfo/dividend",
        "HHKDB669102C0",
        "corporate-actions-dividend",
        &[("GB1", "0"), ("HIGH_GB", "")],
    ),
    (
        "/uapi/domestic-stock/v1/ksdinfo/merger-split",
        "HHKDB669104C0",
        "corporate-actions-merger-split",
        &[],
    ),
    (
        "/uapi/domestic-stock/v1/ksdinfo/rev-split",
        "HHKDB669105C0",
        "corporate-actions-reverse-split",
        &[("MARKET_GB", "0")],
    ),
    (
        "/uapi/domestic-stock/v1/ksdinfo/cap-dcrs",
        "HHKDB669106C0",
        "corporate-actions-capital-decrease",
        &[],
    ),
];

#[tokio::test]
async fn whole_market_has_exact_seven_query_contracts() {
    let (reader, calls) = FakeReader::new(replies(7));
    let provider = KisProvider::kr_etf_core(reader);
    let envelopes = provider
        .fetch_corporate_actions_range(
            "kr",
            date("2020-01-01"),
            date("2026-08-21"),
            now(),
            BatchId::generate(),
            KisActionRangeScope::WholeMarket,
        )
        .await
        .unwrap();
    assert_eq!(envelopes.len(), 7);
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 7);
    for (call, (path, tr_id, _, extra)) in calls.iter().zip(ACTIONS) {
        assert_eq!(call.path, path);
        assert_eq!(call.tr_id, tr_id);
        assert_eq!(
            call.query,
            expected_query("2020-01-01", "2026-08-21", "", extra)
        );
        assert_eq!(call.continuation, None);
    }
}

#[tokio::test]
async fn fixed_etf11_has_sequential_77_symbol_scoped_initial_calls() {
    let (reader, calls) = FakeReader::new(replies(77));
    let provider = KisProvider::kr_etf_core(reader);
    let envelopes = provider
        .fetch_corporate_actions_range(
            "kr",
            date("2020-01-01"),
            date("2026-08-21"),
            now(),
            BatchId::generate(),
            KisActionRangeScope::FixedEtf11,
        )
        .await
        .unwrap();
    assert_eq!(envelopes.len(), 77);
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 77);
    for (symbol_index, symbol) in KR_ETF_CORE_SYMBOLS.iter().enumerate() {
        for (class_index, (path, tr_id, label, extra)) in ACTIONS.iter().enumerate() {
            let call = &calls[symbol_index * 7 + class_index];
            assert_eq!(&call.path, path);
            assert_eq!(&call.tr_id, tr_id);
            assert_eq!(
                call.query,
                expected_query("2020-01-01", "2026-08-21", symbol, extra)
            );
            assert_eq!(call.continuation, None);
            assert_eq!(
                envelopes[symbol_index * 7 + class_index].file_name,
                format!("{label}-{symbol}-page-01.json")
            );
        }
    }
}

#[tokio::test]
async fn exact_m_then_n_pagination_preserves_query_and_chain() {
    let first = MarketDataReply {
        body: br#"{"rt_cd":"0","output1":[{"page":"one"}]}"#.to_vec(),
        continuation: Some("M".to_owned()),
    };
    let second = MarketDataReply {
        body: br#"{"rt_cd":"0","output1":[{"page":"two"}]}"#.to_vec(),
        continuation: Some("F".to_owned()),
    };
    let mut scripted = vec![Ok(first), Ok(second)];
    scripted.extend(replies(6));
    let (reader, calls) = FakeReader::new(scripted);
    let provider = KisProvider::kr_etf_core(reader);
    let envelopes = provider
        .fetch_corporate_actions_range(
            "kr",
            date("2020-01-01"),
            date("2026-08-21"),
            now(),
            BatchId::generate(),
            KisActionRangeScope::WholeMarket,
        )
        .await
        .unwrap();
    assert_eq!(envelopes.len(), 8);
    assert_eq!(
        envelopes[0].file_name,
        "corporate-actions-paidin-subscription-page-01.json"
    );
    assert_eq!(
        envelopes[1].file_name,
        "corporate-actions-paidin-subscription-page-02.json"
    );
    let calls = calls.lock().unwrap();
    assert_eq!(calls[0].continuation, None);
    assert_eq!(calls[1].continuation.as_deref(), Some("N"));
    assert_eq!(calls[0].query, calls[1].query);
}

#[tokio::test]
async fn terminal_f_blank_and_other_markers_are_terminal() {
    for marker in [Some("F"), Some(""), Some("other")] {
        let scripted = (0..7)
            .map(|_| {
                Ok(MarketDataReply {
                    body: br#"{"rt_cd":"0","output1":[]}"#.to_vec(),
                    continuation: marker.map(str::to_owned),
                })
            })
            .collect::<Vec<_>>();
        let (reader, calls) = FakeReader::new(scripted);
        let provider = KisProvider::kr_etf_core(reader);
        assert_eq!(
            provider
                .fetch_corporate_actions_range(
                    "kr",
                    date("2020-01-01"),
                    date("2026-08-21"),
                    now(),
                    BatchId::generate(),
                    KisActionRangeScope::WholeMarket,
                )
                .await
                .unwrap()
                .len(),
            7
        );
        assert_eq!(calls.lock().unwrap().len(), 7);
    }
}

#[tokio::test]
async fn reversed_range_and_pagination_limits_fail_before_raw_output() {
    let (reader, calls) = FakeReader::new(Vec::<Result<MarketDataReply, KisError>>::new());
    let provider = KisProvider::kr_etf_core(reader);
    let error = provider
        .fetch_corporate_actions_range(
            "kr",
            date("2026-08-21"),
            date("2020-01-01"),
            now(),
            BatchId::generate(),
            KisActionRangeScope::WholeMarket,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        market_data::ProviderError::InvalidConfiguration { .. }
    ));
    assert!(calls.lock().unwrap().is_empty());

    let cap_replies = (0..KIS_ACTION_MAX_PAGES)
        .map(|page| {
            Ok(MarketDataReply {
                body: format!(r#"{{"rt_cd":"0","output1":[{{"page":{page}}}]}}"#).into_bytes(),
                continuation: Some("M".to_owned()),
            })
        })
        .collect::<Vec<_>>();
    let (reader, calls) = FakeReader::new(cap_replies);
    let provider = KisProvider::kr_etf_core(reader);
    let error = provider
        .fetch_corporate_actions_range(
            "kr",
            date("2020-01-01"),
            date("2026-08-21"),
            now(),
            BatchId::generate(),
            KisActionRangeScope::WholeMarket,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        market_data::ProviderError::Remote {
            code: "BROKER_PAGINATION_LIMIT",
            ..
        }
    ));
    assert_eq!(calls.lock().unwrap().len(), KIS_ACTION_MAX_PAGES);
}

#[tokio::test]
async fn duplicate_bytes_and_symbol_mismatch_fail_closed() {
    let duplicate = br#"{"rt_cd":"0","output1":[]}"#.to_vec();
    let (reader, _) = FakeReader::new([
        Ok(MarketDataReply {
            body: duplicate.clone(),
            continuation: Some("M".to_owned()),
        }),
        Ok(MarketDataReply {
            body: duplicate,
            continuation: Some("F".to_owned()),
        }),
    ]);
    let provider = KisProvider::kr_etf_core(reader);
    let error = provider
        .fetch_corporate_actions_range(
            "kr",
            date("2020-01-01"),
            date("2026-08-21"),
            now(),
            BatchId::generate(),
            KisActionRangeScope::WholeMarket,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        market_data::ProviderError::Remote {
            code: "BROKER_PAGINATION_STALLED",
            ..
        }
    ));

    let (reader, _) = FakeReader::new([Ok(MarketDataReply {
        body: br#"{"rt_cd":"0","output1":[{"sht_cd":"999999"}]}"#.to_vec(),
        continuation: None,
    })]);
    let provider = KisProvider::kr_etf_core(reader);
    let error = provider
        .fetch_corporate_actions_range(
            "kr",
            date("2020-01-01"),
            date("2026-08-21"),
            now(),
            BatchId::generate(),
            KisActionRangeScope::FixedEtf11,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        market_data::ProviderError::Remote {
            code: "KIS_ACTION_RANGE_SYMBOL_MISMATCH",
            ..
        }
    ));
}

#[tokio::test]
async fn action_range_ingest_is_one_atomic_batch_and_failure_has_no_manifest() {
    let root = tempfile::tempdir().unwrap();
    let (reader, _) = FakeReader::new(replies(77));
    let provider = KisProvider::kr_etf_core(reader);
    let outcome = ingest_kis_action_range(
        &RawStore::new(root.path()),
        &provider,
        "kr",
        date("2020-01-01"),
        date("2026-08-21"),
        now(),
        KisActionRangeScope::FixedEtf11,
        Some("entitlement://test"),
    )
    .await
    .unwrap();
    assert_eq!(outcome.entry.provider, PROVIDER_KIS);
    assert_eq!(outcome.entry.files.len(), 77);
    assert_eq!(
        RawStore::new(root.path())
            .read_manifest(PROVIDER_KIS, "kr")
            .unwrap()
            .len(),
        1
    );

    let failed_root = tempfile::tempdir().unwrap();
    let failure = (0..77).map(|index| {
        if index == 7 {
            Err(KisError::SchemaDrift {
                endpoint: "/uapi/domestic-stock/v1/ksdinfo/bonus-issue".to_owned(),
                detail: "fixture failure".to_owned(),
            })
        } else {
            Ok(terminal_body(index))
        }
    });
    let (reader, _) = FakeReader::new(failure);
    let provider = KisProvider::kr_etf_core(reader);
    assert!(
        ingest_kis_action_range(
            &RawStore::new(failed_root.path()),
            &provider,
            "kr",
            date("2020-01-01"),
            date("2026-08-21"),
            now(),
            KisActionRangeScope::FixedEtf11,
            Some("entitlement://test"),
        )
        .await
        .is_err()
    );
    assert!(!failed_root.path().join("raw").exists());
}

#[tokio::test]
async fn provider_errors_never_render_secret_or_broker_prose() {
    let (reader, _) = FakeReader::new([Err(KisError::Broker {
        status: 403,
        endpoint: "/uapi/domestic-stock/v1/ksdinfo/bonus-issue".to_owned(),
        body: "appsecret=secret-sentinel broker-prose-sentinel".to_owned(),
    })]);
    let provider = KisProvider::kr_etf_core(reader);
    let error = provider
        .fetch_corporate_actions_range(
            "kr",
            date("2020-01-01"),
            date("2026-08-21"),
            now(),
            BatchId::generate(),
            KisActionRangeScope::WholeMarket,
        )
        .await
        .unwrap_err();
    let rendered = error.to_string();
    assert!(!rendered.contains("secret-sentinel"));
    assert!(!rendered.contains("broker-prose-sentinel"));
}
