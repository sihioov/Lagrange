use chrono::{DateTime, Utc};
use collectors::intraday_quotes::{
    INTRADAY_QUOTE_PATH, INTRADAY_QUOTE_TR_ID, IntradayMarketState, IntradayQuoteCaptureError,
    IntradaySessionDisposition, IntradaySessionWindowContract, intraday_quote_query,
    parse_intraday_attempt,
};
use kis_client::{IntradayAttemptError, IntradayAttemptOutcome};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn entry(
    date: &str,
    disposition: &str,
    open_local: Option<&str>,
    close_local: Option<&str>,
) -> Value {
    entry_with_evidence(
        date,
        disposition,
        open_local,
        close_local,
        &format!("{date}T00:00:00+09:00"),
    )
}

fn entry_with_evidence(
    date: &str,
    disposition: &str,
    open_local: Option<&str>,
    close_local: Option<&str>,
    evidence_retrieved_at: &str,
) -> Value {
    json!({
        "date": date,
        "disposition": disposition,
        "open_local": open_local,
        "close_local": close_local,
        "evidence_url": "https://global.krx.co.kr/contents/test",
        "evidence_retrieved_at": evidence_retrieved_at,
        "evidence_sha256": format!("sha256:{}", "c".repeat(64)),
    })
}

fn bytes(entries: Vec<Value>) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schema_version": 1,
        "exchange": "KRX",
        "timezone": "Asia/Seoul",
        "entries": entries,
    }))
    .expect("window fixture serializes")
}

fn pinned(entries: Vec<Value>) -> (Vec<u8>, String) {
    let bytes = bytes(entries);
    let hash = format!("sha256:{:x}", Sha256::digest(&bytes));
    (bytes, hash)
}

fn utc(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("RFC3339 fixture")
        .with_timezone(&Utc)
}

#[test]
fn regular_special_closed_and_rollover_states_use_only_exact_entries() {
    let (bytes, hash) = pinned(vec![
        entry("2026-09-07", "CLOSED", None, None),
        entry("2026-09-08", "REGULAR", Some("09:00:00"), Some("15:30:00")),
        entry("2026-09-09", "SPECIAL", Some("10:00:00"), Some("12:00:00")),
    ]);
    let contract = IntradaySessionWindowContract::from_bytes(&bytes, &hash).unwrap();

    assert_eq!(
        contract.state_at(utc("2026-09-08T00:00:00Z"), false),
        IntradayMarketState::Open
    );
    assert_eq!(
        contract.state_at(utc("2026-09-08T06:29:59Z"), false),
        IntradayMarketState::Open
    );
    assert_eq!(
        contract.state_at(utc("2026-09-08T06:30:00Z"), false),
        IntradayMarketState::Closed
    );
    assert_eq!(
        contract.state_at(utc("2026-09-09T01:00:00Z"), false),
        IntradayMarketState::Open
    );
    assert_eq!(
        contract.state_at(utc("2026-09-09T01:00:00Z"), true),
        IntradayMarketState::Halted
    );
    assert_eq!(
        contract.state_at(utc("2026-09-09T03:00:00Z"), false),
        IntradayMarketState::Closed
    );
    assert_eq!(
        contract.state_at(utc("2026-09-10T00:00:00Z"), false),
        IntradayMarketState::Unknown
    );
    assert_eq!(
        contract.state_at(utc("2026-09-07T03:00:00Z"), true),
        IntradayMarketState::Closed
    );
    assert_eq!(
        contract.state_at(utc("2026-09-08T00:00:00Z"), true),
        IntradayMarketState::Halted
    );
    assert_eq!(
        contract
            .entry(chrono::NaiveDate::from_ymd_opt(2026, 9, 9).unwrap())
            .unwrap()
            .disposition,
        IntradaySessionDisposition::Special
    );
}

#[test]
fn evidence_must_be_nonfuture_and_same_kst_civil_date_before_state_classification() {
    let now = utc("2026-09-08T00:00:00Z");

    let (previous_day_bytes, previous_day_hash) = pinned(vec![entry_with_evidence(
        "2026-09-08",
        "REGULAR",
        Some("09:00:00"),
        Some("15:30:00"),
        "2026-09-07T14:59:59Z",
    )]);
    let previous_day =
        IntradaySessionWindowContract::from_bytes(&previous_day_bytes, &previous_day_hash).unwrap();
    assert_eq!(
        previous_day.state_at(now, false),
        IntradayMarketState::Unknown
    );

    let (future_bytes, future_hash) = pinned(vec![entry_with_evidence(
        "2026-09-08",
        "REGULAR",
        Some("09:00:00"),
        Some("15:30:00"),
        "2026-09-08T00:00:01Z",
    )]);
    let future = IntradaySessionWindowContract::from_bytes(&future_bytes, &future_hash).unwrap();
    assert_eq!(future.state_at(now, false), IntradayMarketState::Unknown);

    let (exact_now_bytes, exact_now_hash) = pinned(vec![entry_with_evidence(
        "2026-09-08",
        "REGULAR",
        Some("09:00:00"),
        Some("15:30:00"),
        "2026-09-08T00:00:00Z",
    )]);
    let exact_now =
        IntradaySessionWindowContract::from_bytes(&exact_now_bytes, &exact_now_hash).unwrap();
    assert_eq!(exact_now.state_at(now, false), IntradayMarketState::Open);

    let (same_kst_bytes, same_kst_hash) = pinned(vec![entry_with_evidence(
        "2026-09-08",
        "REGULAR",
        Some("09:00:00"),
        Some("15:30:00"),
        "2026-09-07T15:00:00Z",
    )]);
    let same_kst =
        IntradaySessionWindowContract::from_bytes(&same_kst_bytes, &same_kst_hash).unwrap();
    assert_eq!(
        same_kst.state_at(utc("2026-09-08T00:00:01Z"), false),
        IntradayMarketState::Open
    );

    let (rollover_bytes, rollover_hash) = pinned(vec![entry_with_evidence(
        "2026-09-07",
        "REGULAR",
        Some("09:00:00"),
        Some("15:30:00"),
        "2026-09-07T14:59:59Z",
    )]);
    let rollover =
        IntradaySessionWindowContract::from_bytes(&rollover_bytes, &rollover_hash).unwrap();
    assert_eq!(rollover.state_at(now, false), IntradayMarketState::Unknown);

    let (missing_bytes, missing_hash) = pinned(Vec::new());
    let missing = IntradaySessionWindowContract::from_bytes(&missing_bytes, &missing_hash).unwrap();
    assert_eq!(missing.state_at(now, false), IntradayMarketState::Unknown);

    let (stale_closed_bytes, stale_closed_hash) = pinned(vec![entry_with_evidence(
        "2026-09-08",
        "CLOSED",
        None,
        None,
        "2026-09-07T14:59:59Z",
    )]);
    let stale_closed =
        IntradaySessionWindowContract::from_bytes(&stale_closed_bytes, &stale_closed_hash).unwrap();
    assert_eq!(
        stale_closed.state_at(now, true),
        IntradayMarketState::Unknown
    );
}

#[test]
fn hash_and_closed_schema_are_fail_closed() {
    let (bytes, hash) = pinned(vec![entry(
        "2026-09-08",
        "REGULAR",
        Some("09:00:00"),
        Some("15:30:00"),
    )]);
    assert!(IntradaySessionWindowContract::from_bytes(&bytes, &hash).is_ok());
    assert_eq!(
        IntradaySessionWindowContract::from_bytes(&bytes, &format!("sha256:{}", "d".repeat(64)))
            .unwrap_err()
            .to_string(),
        "INTRADAY_SESSION_WINDOW_HASH_MISMATCH"
    );
    assert!(IntradaySessionWindowContract::from_bytes(&bytes, &hash.to_ascii_uppercase()).is_err());

    let mut unknown: Value = serde_json::from_slice(&bytes).unwrap();
    unknown["unexpected"] = json!(true);
    let unknown_bytes = serde_json::to_vec(&unknown).unwrap();
    let unknown_hash = format!("sha256:{:x}", Sha256::digest(&unknown_bytes));
    assert!(IntradaySessionWindowContract::from_bytes(&unknown_bytes, &unknown_hash).is_err());

    let (closed_bytes, closed_hash) =
        pinned(vec![entry("2026-09-08", "CLOSED", Some("09:00:00"), None)]);
    assert!(IntradaySessionWindowContract::from_bytes(&closed_bytes, &closed_hash).is_err());

    let (unsorted_bytes, unsorted_hash) = pinned(vec![
        entry("2026-09-09", "REGULAR", Some("09:00:00"), Some("15:30:00")),
        entry("2026-09-08", "REGULAR", Some("09:00:00"), Some("15:30:00")),
    ]);
    assert!(IntradaySessionWindowContract::from_bytes(&unsorted_bytes, &unsorted_hash).is_err());
}

#[test]
fn exact_query_and_parser_failure_preserve_the_provider_free_boundary() {
    assert_eq!(
        INTRADAY_QUOTE_PATH,
        "/uapi/domestic-stock/v1/quotations/inquire-price"
    );
    assert_eq!(INTRADAY_QUOTE_TR_ID, "FHKST01010100");
    assert_eq!(
        intraday_quote_query("005930").unwrap(),
        vec![
            ("FID_COND_MRKT_DIV_CODE".to_owned(), "J".to_owned()),
            ("FID_INPUT_ISCD".to_owned(), "005930".to_owned()),
        ]
    );
    assert!(intraday_quote_query("00593").is_err());
    assert!(intraday_quote_query("005930.KRX").is_err());

    let error = parse_intraday_attempt(
        "005930",
        IntradayAttemptOutcome::Failed {
            error: IntradayAttemptError::Transport,
            reservation: None,
        },
    )
    .unwrap_err();
    assert!(matches!(
        error,
        IntradayQuoteCaptureError::Attempt {
            error: IntradayAttemptError::Transport,
            reservation: None,
        }
    ));
    assert!(!format!("{error:?}").contains("provider"));
}
