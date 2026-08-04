//! Contract tests for `lagrange-domain` typed financial and time contracts.
//!
//! These are the named QA tests for Todo 2:
//!   - `cargo test -p domain rejects_invalid_financial_values`
//!   - `cargo test -p domain round_trip -- --exact`
//!
//! Every financial/time/validation behavior is expected to reject invalid
//! input with a TYPED `DomainError` (never a panic).

use chrono::{DateTime, NaiveDate, Utc};

use domain::{
    Currency, DomainError, FixedPoint, InstrumentId, Money, Price, Quantity, ReportedStat, RunId,
    StrategyId, TradingDate, UtcTimestamp, Venue, VenueTimestamp, Weight, MONEY_SCALE, WEIGHT_SCALE,
};

fn naive_utc(year: i32, month: u32, day: u32, h: u32, m: u32, s: u32) -> chrono::NaiveDateTime {
    NaiveDate::from_ymd_opt(year, month, day)
        .expect("valid date")
        .and_hms_opt(h, m, s)
        .expect("valid time")
}

#[test]
fn rejects_invalid_financial_values() {
    // Negative price -> typed rejection, no panic.
    let err = Price::parse("-100.0000").expect_err("negative price must be rejected");
    assert!(
        matches!(err, DomainError::NonPositivePrice { .. }),
        "unexpected error: {err}"
    );
    let err = Price::parse("0.0000").expect_err("zero price must be rejected");
    assert!(matches!(err, DomainError::NonPositivePrice { .. }));

    // NaN / infinite reported statistics -> typed rejection.
    let err = ReportedStat::from_f64(f64::NAN).expect_err("NaN metric must be rejected");
    assert!(matches!(err, DomainError::NonFiniteMetric { .. }));
    let err = ReportedStat::from_f64(f64::INFINITY).expect_err("infinite metric must be rejected");
    assert!(matches!(err, DomainError::NonFiniteMetric { .. }));
    let err = ReportedStat::from_f64(f64::NEG_INFINITY).expect_err("infinite metric must be rejected");
    assert!(matches!(err, DomainError::NonFiniteMetric { .. }));

    // Currency mismatch -> typed rejection on add and sub.
    let krw = Money::parse("100.0000", Currency::KRW).expect("valid money");
    let usd = Money::parse("1.0000", Currency::USD).expect("valid money");
    assert!(matches!(krw.checked_add(&usd).expect_err("currency mismatch"), DomainError::CurrencyMismatch { .. }));
    assert!(matches!(krw.checked_sub(&usd).expect_err("currency mismatch"), DomainError::CurrencyMismatch { .. }));

    // Money cannot go negative -> typed rejection.
    let five = Money::parse("5.0000", Currency::KRW).expect("valid money");
    let err = five.checked_sub(&krw).expect_err("5 - 100 must be rejected");
    assert!(matches!(err, DomainError::NegativeMoney { .. }));

    // Negative quantity -> typed rejection.
    let err = Quantity::parse("-1").expect_err("negative quantity must be rejected");
    assert!(matches!(err, DomainError::NegativeQuantity { .. }));

    // Weight outside [0, 1] -> typed rejection.
    let err = Weight::parse("1.000001").expect_err("weight over 1 must be rejected");
    assert!(matches!(err, DomainError::WeightOutOfRange { .. }));
    let err = Weight::parse("-0.000001").expect_err("negative weight must be rejected");
    assert!(matches!(err, DomainError::WeightOutOfRange { .. }));

    // Ambiguous local timestamp (US DST fall-back 2026-11-01 01:30 occurs twice)
    // -> typed rejection, no panic.
    let ambiguous = naive_utc(2026, 11, 1, 1, 30, 0);
    let err = VenueTimestamp::from_naive_local(Venue::Nyse, ambiguous)
        .expect_err("ambiguous local time must be rejected");
    assert!(matches!(err, DomainError::AmbiguousLocalTime { .. }));

    // Nonexistent local timestamp (US DST spring-forward 2026-03-08 02:30)
    // -> typed rejection, no panic.
    let nonexistent = naive_utc(2026, 3, 8, 2, 30, 0);
    let err = VenueTimestamp::from_naive_local(Venue::Nyse, nonexistent)
        .expect_err("nonexistent local time must be rejected");
    assert!(matches!(err, DomainError::NonexistentLocalTime { .. }));

    // Checked arithmetic overflow -> typed rejection.
    let huge = Money::from_fixed(
        FixedPoint::from_i128(i128::MAX, MONEY_SCALE).expect("max value"),
        Currency::KRW,
    )
    .expect("max money");
    let err = huge.checked_add(&huge).expect_err("overflow must be rejected");
    assert!(matches!(err, DomainError::Overflow { .. }));

    // Division by zero -> typed rejection.
    let one = FixedPoint::from_i128(1, 0).expect("one");
    let zero = FixedPoint::from_i128(0, 0).expect("zero");
    let err = one.checked_div(&zero, 4).expect_err("division by zero must be rejected");
    assert!(matches!(err, DomainError::DivisionByZero));

    // Invalid branded identifiers -> typed rejection.
    let err = InstrumentId::parse("lower.krx").expect_err("lowercase symbol must be rejected");
    assert!(matches!(err, DomainError::InvalidId { .. }));
    let err = InstrumentId::parse("069500.NOPE").expect_err("unknown venue must be rejected");
    assert!(matches!(err, DomainError::InvalidId { .. }));
    let err = Currency::from_code("kRw").expect_err("lowercase currency must be rejected");
    assert!(matches!(err, DomainError::InvalidCurrency { .. }));
    let err = StrategyId::parse("Dual Momentum!").expect_err("invalid slug must be rejected");
    assert!(matches!(err, DomainError::InvalidId { .. }));
    let err = TradingDate::new(2026, 2, 30).expect_err("invalid calendar day must be rejected");
    assert!(matches!(err, DomainError::InvalidTradingDate { .. }));
    let err = UtcTimestamp::parse_rfc3339("not-a-timestamp")
        .expect_err("malformed timestamp must be rejected");
    assert!(matches!(err, DomainError::InvalidId { .. }));

    // Invalid decimal strings -> typed rejection.
    let err = FixedPoint::parse("12.34.56").expect_err("double dot must be rejected");
    assert!(matches!(err, DomainError::InvalidDecimalString { .. }));
    let err = FixedPoint::parse("abc").expect_err("non-numeric must be rejected");
    assert!(matches!(err, DomainError::InvalidDecimalString { .. }));

    // Sanity: valid values construct without error (no panic anywhere above).
    let _ = Price::parse("34570.0000").expect("valid price");
    let _ = Money::parse("3457000.0000", Currency::KRW).expect("valid money");
}

#[test]
fn round_trip() {
    // Price serializes as a canonical decimal STRING (scale 4).
    let p = Price::parse("34570.0000").expect("valid price");
    let json = serde_json::to_string(&p).expect("serialize");
    assert_eq!(json, "\"34570.0000\"");
    let back: Price = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, p);
    assert_eq!(serde_json::to_string(&back).expect("re-serialize"), json);

    // Non-canonical input is canonicalized on parse (34570.5 -> 34570.5000).
    assert_eq!(Price::parse("34570.5").expect("valid").as_decimal_string(), "34570.5000");

    // Money serializes as { amount: string, currency: string }.
    let m = Money::parse("3457000.0000", Currency::KRW).expect("valid money");
    let jm = serde_json::to_string(&m).expect("serialize");
    assert_eq!(jm, r#"{"amount":"3457000.0000","currency":"KRW"}"#);
    let back: Money = serde_json::from_str(&jm).expect("deserialize");
    assert_eq!(back, m);
    assert_eq!(serde_json::to_string(&back).expect("re-serialize"), jm);

    // Quantity and Weight serialize as canonical decimal strings.
    let q = Quantity::parse("100").expect("valid quantity");
    assert_eq!(serde_json::to_string(&q).expect("serialize"), "\"100\"");
    let w = Weight::parse("0.400000").expect("valid weight");
    assert_eq!(serde_json::to_string(&w).expect("serialize"), "\"0.400000\"");

    // UtcTimestamp serializes as RFC3339 UTC.
    let utc = DateTime::from_naive_utc_and_offset(naive_utc(2026, 8, 4, 15, 0, 0), Utc);
    let ts = UtcTimestamp::from_datetime(utc);
    let jt = serde_json::to_string(&ts).expect("serialize");
    assert_eq!(jt, "\"2026-08-04T15:00:00Z\"");
    let back: UtcTimestamp = serde_json::from_str(&jt).expect("deserialize");
    assert_eq!(back, ts);

    // VenueTimestamp serializes as { venue, local RFC3339 } and round-trips.
    let vt = VenueTimestamp::from_naive_local(Venue::Krx, naive_utc(2026, 8, 5, 0, 0, 0))
        .expect("valid venue timestamp");
    assert_eq!(vt.to_rfc3339(), "2026-08-05T00:00:00+09:00");
    let jv = serde_json::to_string(&vt).expect("serialize");
    assert_eq!(jv, r#"{"venue":"krx","local":"2026-08-05T00:00:00+09:00"}"#);
    let back: VenueTimestamp = serde_json::from_str(&jv).expect("deserialize");
    assert_eq!(back, vt);
    assert_eq!(serde_json::to_string(&back).expect("re-serialize"), jv);

    // TradingDate serializes as an ISO calendar date (no time component).
    let td = TradingDate::new(2026, 8, 5).expect("valid date");
    assert_eq!(serde_json::to_string(&td).expect("serialize"), "\"2026-08-05\"");

    // Branded IDs round-trip through JSON as strings.
    let sid = StrategyId::parse("dual-momentum").expect("valid slug");
    assert_eq!(serde_json::to_string(&sid).expect("serialize"), "\"dual-momentum\"");
    let uid = RunId::generate();
    let ju = serde_json::to_string(&uid).expect("serialize");
    assert_eq!(serde_json::from_str::<RunId>(&ju).expect("deserialize"), uid);

    // Typed DomainError round-trips with its tagged code (typed, not a bare string).
    let err = DomainError::CurrencyMismatch {
        left: Currency::KRW,
        right: Currency::USD,
    };
    let je = serde_json::to_string(&err).expect("serialize");
    assert_eq!(je, r#"{"code":"currency_mismatch","left":"KRW","right":"USD"}"#);
    let back: DomainError = serde_json::from_str(&je).expect("deserialize");
    assert_eq!(back, err);

    // Venue-local time normalized from UTC round-trips to the same UTC instant.
    let utc_ts = UtcTimestamp::from_datetime(utc);
    let venue_local = VenueTimestamp::from_utc(Venue::Krx, utc_ts);
    assert_eq!(venue_local.to_utc(), utc_ts);

    // Weight scale constant sanity.
    let one = Weight::parse("1.000000").expect("valid weight");
    assert_eq!(one.amount().bits(), 1_000_000i128);
    assert_eq!(one.amount().scale(), WEIGHT_SCALE);
}
