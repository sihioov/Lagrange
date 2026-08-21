//! Offline contract tests for the owner-approved FSC KRX listing source.
//! Every response is a synthetic fixture; this test never constructs a live
//! client and never makes a provider/network call.

use std::sync::{Arc, Mutex};

use data_go_client::ItemInfoQuery;
use domain::{TradingDate, UtcTimestamp};
use market_data::contract::{FetchMode, MARKET_KR, ResponseKind};
use market_data::storage::RawStore;
use market_data::{
    FIXED_ETF11, FSC_KRX_LISTED_ENTITLEMENT_REFERENCE, FSC_KRX_LISTED_PROVIDER,
    FscKrxListedAvailability, FscKrxListedError, FscKrxListedOutcome, FscKrxListedProvider,
    FscKrxListedRead, ITEM_INFO_MAX_PAGES, ITEM_INFO_PAGE_SIZE,
};
use serde_json::{Value, json};

const SENTINEL_SERVICE_KEY: &str = "service-key-sentinel-test-only";

#[derive(Debug, Clone)]
struct FixtureReader {
    calls: Arc<Mutex<Vec<ItemInfoQuery>>>,
    pages: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl FixtureReader {
    fn new(pages: Vec<Vec<u8>>) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            pages: Arc::new(Mutex::new(pages)),
        }
    }

    fn calls(&self) -> Vec<ItemInfoQuery> {
        self.calls.lock().unwrap().clone()
    }
}

impl FscKrxListedRead for FixtureReader {
    async fn get_item_info(&self, query: &ItemInfoQuery) -> Result<Vec<u8>, FscKrxListedError> {
        self.calls.lock().unwrap().push(query.clone());
        self.pages
            .lock()
            .unwrap()
            .pop()
            .ok_or(FscKrxListedError::MissingTargetObservation {
                short_code: "fixture-exhausted".to_owned(),
            })
    }
}

fn date() -> TradingDate {
    TradingDate::new(2026, 8, 20).unwrap()
}

fn retrieved_at() -> UtcTimestamp {
    UtcTimestamp::parse_rfc3339("2026-08-21T06:00:00Z").unwrap()
}

fn response_page(page_no: u32, total_count: u64, item: Option<Value>) -> Vec<u8> {
    let items = item.map_or_else(Vec::new, |item| vec![item]);
    json!({
        "response": {
            "header": {"resultCode": "00", "resultMsg": "SYNTHETIC"},
            "body": {
                "numOfRows": ITEM_INFO_PAGE_SIZE,
                "pageNo": page_no,
                "totalCount": total_count,
                "items": {"item": items}
            }
        }
    })
    .to_string()
    .into_bytes()
}

fn item(short_code: &str, isin_cd: &str, bas_dt: &str) -> Value {
    json!({
        "basDt": bas_dt,
        "srtnCd": format!("A{short_code}"),
        "isinCd": isin_cd,
        "mrktCtg": "KOSPI",
        "itmsNm": "SYNTHETIC ETF",
        "crno": "",
        "corpNm": "SYNTHETIC CORP"
    })
}

#[tokio::test]
async fn exact_identity_probe_is_single_page_and_never_writes_raw() {
    let (_temp, store) = new_store();
    let available = response_page(1, 1, Some(item("069500", "KR7069500007", "20260820")));
    let reader = FixtureReader::new(vec![available]);
    let provider = FscKrxListedProvider::new(reader.clone());
    assert_eq!(
        provider
            .probe_fixed_identity(&date(), FIXED_ETF11[0])
            .await
            .unwrap(),
        FscKrxListedAvailability::Available
    );
    assert_eq!(reader.calls().len(), 1);
    assert!(
        !store
            .manifest_path(FSC_KRX_LISTED_PROVIDER, MARKET_KR)
            .exists()
    );

    let unavailable = response_page(1, 0, None);
    assert_eq!(
        FscKrxListedProvider::new(FixtureReader::new(vec![unavailable]))
            .probe_fixed_identity(&date(), FIXED_ETF11[0])
            .await
            .unwrap(),
        FscKrxListedAvailability::Unavailable
    );

    let mut unprefixed = item("069500", "KR7069500007", "20260820");
    unprefixed["srtnCd"] = json!("069500");
    let error = FscKrxListedProvider::new(FixtureReader::new(vec![response_page(
        1,
        1,
        Some(unprefixed),
    )]))
    .probe_fixed_identity(&date(), FIXED_ETF11[0])
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        FscKrxListedError::UnexpectedObservation { .. }
    ));
}

fn valid_pages() -> Vec<Vec<u8>> {
    FIXED_ETF11
        .iter()
        .rev()
        .map(|identity| {
            response_page(
                1,
                1,
                Some(item(identity.short_code, identity.isin_cd, "20260820")),
            )
        })
        .collect()
}

fn new_store() -> (tempfile::TempDir, RawStore) {
    let temp = tempfile::tempdir().unwrap();
    let store = RawStore::new(temp.path());
    (temp, store)
}

#[tokio::test]
async fn exact_fixed_etf11_queries_commit_one_immutable_raw_batch() {
    let (_temp, store) = new_store();
    let reader = FixtureReader::new(valid_pages());
    let provider = FscKrxListedProvider::new(reader.clone());
    let outcome = provider
        .ingest_fixed_etf11(
            &store,
            &date(),
            retrieved_at(),
            FetchMode::Credentialed,
            FSC_KRX_LISTED_ENTITLEMENT_REFERENCE,
        )
        .await
        .unwrap();

    let FscKrxListedOutcome::Stored(entry) = outcome;
    assert_eq!(entry.provider, FSC_KRX_LISTED_PROVIDER);
    assert_eq!(entry.market, MARKET_KR);
    assert_eq!(entry.files.len(), FIXED_ETF11.len());
    assert!(
        entry
            .files
            .iter()
            .all(|file| file.kind == ResponseKind::CandidateMaster)
    );

    let calls = reader.calls();
    assert_eq!(calls.len(), FIXED_ETF11.len());
    for (call, identity) in calls.iter().zip(FIXED_ETF11) {
        assert_eq!(call.num_of_rows, ITEM_INFO_PAGE_SIZE);
        assert_eq!(call.page_no, 1);
        assert_eq!(call.bas_dt, "20260820");
        assert_eq!(call.isin_cd, identity.isin_cd);
        assert!(
            call.visible_pairs()
                .iter()
                .any(|(key, value)| key == "resultType" && value == "json")
        );
        assert!(
            !call
                .visible_pairs()
                .iter()
                .any(|(key, _)| key == "serviceKey")
        );
    }

    let manifest =
        std::fs::read_to_string(store.manifest_path(FSC_KRX_LISTED_PROVIDER, MARKET_KR)).unwrap();
    let batch_dir = store.batch_dir(FSC_KRX_LISTED_PROVIDER, MARKET_KR, &date(), &entry.batch_id);
    let batch_json = std::fs::read_to_string(batch_dir.join("batch.json")).unwrap();
    let batch_value: Value = serde_json::from_str(&batch_json).unwrap();
    let query = &batch_value["files"][0]["request"]["query"];
    assert!(query.as_array().unwrap().iter().any(|pair| {
        pair.as_array().is_some_and(|pair| {
            pair.first().and_then(Value::as_str) == Some("resultType")
                && pair.get(1).and_then(Value::as_str) == Some("json")
        })
    }));
    let mut stored_bytes = Vec::new();
    for file in std::fs::read_dir(&batch_dir).unwrap() {
        stored_bytes.extend(std::fs::read(file.unwrap().path()).unwrap());
    }
    let all_stored = format!("{manifest}{batch_json}");
    assert!(!all_stored.contains(SENTINEL_SERVICE_KEY));
    assert!(!all_stored.contains("serviceKey"));
    assert!(
        !stored_bytes
            .windows(SENTINEL_SERVICE_KEY.len())
            .any(|window| window == SENTINEL_SERVICE_KEY.as_bytes())
    );
}

#[tokio::test]
async fn malformed_non_success_missing_and_wrong_identity_responses_fail_closed() {
    let (_temp, store) = new_store();
    let reader = FixtureReader::new(vec![b"not-json".to_vec()]);
    let error = FscKrxListedProvider::new(reader)
        .ingest_fixed_etf11(
            &store,
            &date(),
            retrieved_at(),
            FetchMode::Credentialed,
            FSC_KRX_LISTED_ENTITLEMENT_REFERENCE,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, FscKrxListedError::MalformedJson));
    assert!(
        !store
            .manifest_path(FSC_KRX_LISTED_PROVIDER, MARKET_KR)
            .exists()
    );

    let non_success = json!({
        "response": {
            "header": {"resultCode": "99", "resultMsg": SENTINEL_SERVICE_KEY},
            "body": {"numOfRows": ITEM_INFO_PAGE_SIZE, "pageNo": 1, "totalCount": 0, "items": {"item": []}}
        }
    })
    .to_string()
    .into_bytes();
    let error = FscKrxListedProvider::new(FixtureReader::new(vec![non_success]))
        .ingest_fixed_etf11(
            &store,
            &date(),
            retrieved_at(),
            FetchMode::Credentialed,
            FSC_KRX_LISTED_ENTITLEMENT_REFERENCE,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, FscKrxListedError::NonSuccessResult { .. }));
    assert!(!error.to_string().contains(SENTINEL_SERVICE_KEY));

    let missing = response_page(1, 0, None);
    let error = FscKrxListedProvider::new(FixtureReader::new(vec![missing]))
        .ingest_fixed_etf11(
            &store,
            &date(),
            retrieved_at(),
            FetchMode::Credentialed,
            FSC_KRX_LISTED_ENTITLEMENT_REFERENCE,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        FscKrxListedError::MissingTargetObservation { .. }
    ));

    let wrong = response_page(1, 1, Some(item("999999", "KR7999990000", "20260820")));
    let error = FscKrxListedProvider::new(FixtureReader::new(vec![wrong]))
        .ingest_fixed_etf11(
            &store,
            &date(),
            retrieved_at(),
            FetchMode::Credentialed,
            FSC_KRX_LISTED_ENTITLEMENT_REFERENCE,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        FscKrxListedError::UnexpectedObservation { .. }
    ));
}

#[tokio::test]
async fn page_identity_total_count_and_duplicate_bytes_are_checked() {
    let (_temp, store) = new_store();
    let page_mismatch = response_page(2, 1, Some(item("069500", "KR7069500007", "20260820")));
    let error = FscKrxListedProvider::new(FixtureReader::new(vec![page_mismatch]))
        .ingest_fixed_etf11(
            &store,
            &date(),
            retrieved_at(),
            FetchMode::Credentialed,
            FSC_KRX_LISTED_ENTITLEMENT_REFERENCE,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        FscKrxListedError::ResponsePageMismatch { .. }
    ));

    let first = response_page(1, 101, Some(item("069500", "KR7069500007", "20260820")));
    let changed = response_page(2, 101, Some(item("069500", "KR7069500007", "20260820")));
    let changed_value: Value = serde_json::from_slice(&changed).unwrap();
    let mut changed_value = changed_value;
    changed_value["response"]["body"]["totalCount"] = json!(102);
    let changed = changed_value.to_string().into_bytes();
    let error = FscKrxListedProvider::new(FixtureReader::new(vec![changed, first]))
        .ingest_fixed_etf11(
            &store,
            &date(),
            retrieved_at(),
            FetchMode::Credentialed,
            FSC_KRX_LISTED_ENTITLEMENT_REFERENCE,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, FscKrxListedError::InconsistentTotalCount));

    let first_identity = response_page(1, 1, Some(item("069500", "KR7069500007", "20260820")));
    let mut pages = valid_pages();
    pages[9] = first_identity;
    let error = FscKrxListedProvider::new(FixtureReader::new(pages))
        .ingest_fixed_etf11(
            &store,
            &date(),
            retrieved_at(),
            FetchMode::Credentialed,
            FSC_KRX_LISTED_ENTITLEMENT_REFERENCE,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        FscKrxListedError::DuplicateResponseBytes { .. }
    ));
}

#[tokio::test]
async fn pagination_is_bounded_at_ten_pages() {
    let (_temp, store) = new_store();
    let mut pages = Vec::new();
    for page_no in (1..=ITEM_INFO_MAX_PAGES as u32).rev() {
        pages.push(response_page(page_no, 1001, None));
    }
    let error = FscKrxListedProvider::new(FixtureReader::new(pages))
        .ingest_fixed_etf11(
            &store,
            &date(),
            retrieved_at(),
            FetchMode::Credentialed,
            FSC_KRX_LISTED_ENTITLEMENT_REFERENCE,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        FscKrxListedError::PaginationBoundExceeded {
            max_pages: ITEM_INFO_MAX_PAGES
        }
    ));
    assert!(
        !store
            .manifest_path(FSC_KRX_LISTED_PROVIDER, MARKET_KR)
            .exists()
    );
}

#[tokio::test]
async fn oversized_response_body_is_rejected_before_raw_commit() {
    let (_temp, store) = new_store();
    let oversized = vec![b'x'; data_go_client::MAX_RESPONSE_BODY_BYTES + 1];
    let error = FscKrxListedProvider::new(FixtureReader::new(vec![oversized]))
        .ingest_fixed_etf11(
            &store,
            &date(),
            retrieved_at(),
            FetchMode::Credentialed,
            FSC_KRX_LISTED_ENTITLEMENT_REFERENCE,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        FscKrxListedError::ResponseBodyTooLarge {
            max_bytes: data_go_client::MAX_RESPONSE_BODY_BYTES
        }
    ));
    assert!(
        !store
            .manifest_path(FSC_KRX_LISTED_PROVIDER, MARKET_KR)
            .exists()
    );
}
