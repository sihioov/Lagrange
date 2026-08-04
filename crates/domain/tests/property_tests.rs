//! Property tests for `lagrange-domain` checked arithmetic, timezone
//! normalization, finite-metric checks, and stable serialization.
//!
//! These are `proptest` properties: each runs hundreds of generated cases and
//! must never panic — invalid input surfaces as a typed `DomainError`.

use chrono::{DateTime, NaiveDate, Utc};

use domain::{
    Currency, DomainError, FixedPoint, InstrumentId, Money, Price, Quantity, ReportedStat,
    TradingDate, UtcTimestamp, Venue, VenueTimestamp, Weight,
};

use proptest::prelude::*;

/// Positive decimal strings (no sign, at least one non-zero integer digit).
fn positive_decimal() -> impl Strategy<Value = String> {
    "[1-9][0-9]{0,5}(\\.[0-9]{1,4})?"
}

/// Arbitrary decimal-ish strings (may be malformed; parse must not panic).
fn decimal_string() -> impl Strategy<Value = String> {
    prop_oneof![
        "[0-9]{1,6}",
        "[0-9]{1,6}\\.[0-9]{1,4}",
        "-?[0-9]{1,6}",
        "-?[0-9]{1,6}\\.[0-9]{1,4}",
    ]
}

fn venue() -> impl Strategy<Value = Venue> {
    prop::sample::select(vec![Venue::Krx, Venue::Nyse, Venue::Arca, Venue::Nasdaq])
}

proptest! {
    // ---- checked arithmetic -------------------------------------------------

    #[test]
    fn fixed_point_add_commutes(a in decimal_string(), b in decimal_string()) {
        let fa = FixedPoint::parse(&a).unwrap();
        let fb = FixedPoint::parse(&b).unwrap();
        let left = fa.checked_add(&fb).unwrap();
        let right = fb.checked_add(&fa).unwrap();
        prop_assert_eq!(left, right, "addition must commute");
    }

    #[test]
    fn fixed_point_add_sub_is_inverse(a in decimal_string(), b in decimal_string()) {
        let fa = FixedPoint::parse(&a).unwrap();
        let fb = FixedPoint::parse(&b).unwrap();
        let sum = fa.checked_add(&fb).unwrap();
        let back = sum.checked_sub(&fb).unwrap();
        prop_assert_eq!(back, fa, "(a + b) - b must equal a");
    }

    #[test]
    fn money_add_commutes_within_currency(a in positive_decimal(), b in positive_decimal()) {
        let ma = Money::parse(&a, Currency::KRW).unwrap();
        let mb = Money::parse(&b, Currency::KRW).unwrap();
        prop_assert_eq!(
            ma.checked_add(&mb).unwrap(),
            mb.checked_add(&ma).unwrap(),
            "money addition must commute within a currency"
        );
    }

    #[test]
    fn money_sub_is_checked_and_typed(a in positive_decimal(), b in positive_decimal()) {
        let ma = Money::parse(&a, Currency::KRW).unwrap();
        let mb = Money::parse(&b, Currency::KRW).unwrap();
        match ma.checked_sub(&mb) {
            Ok(diff) => prop_assert!(!diff.amount().is_negative(), "money may not be negative"),
            Err(err) => {
                let is_negative_money = matches!(err, DomainError::NegativeMoney { .. });
                prop_assert!(is_negative_money, "only a typed NegativeMoney rejection is allowed");
            }
        }
    }

    // ---- currency mismatch --------------------------------------------------

    #[test]
    fn money_currency_mismatch_rejected(a in positive_decimal(), b in positive_decimal()) {
        let krw = Money::parse(&a, Currency::KRW).unwrap();
        let usd = Money::parse(&b, Currency::USD).unwrap();
        let err = krw.checked_add(&usd).unwrap_err();
        let is_mismatch = matches!(err, DomainError::CurrencyMismatch { .. });
        prop_assert!(is_mismatch, "currency mismatch must be a typed error");
    }

    // ---- timezone normalization ---------------------------------------------

    #[test]
    fn utc_to_venue_normalization_is_invertible(
        year in 2000..2030i32,
        month in 1..=12u32,
        day in 1..=28u32,
        h in 0..24u32,
        m in 0..60u32,
        s in 0..60u32,
        venue in venue(),
    ) {
        let naive = NaiveDate::from_ymd_opt(year, month, day)
            .unwrap()
            .and_hms_opt(h, m, s)
            .unwrap();
        let utc = UtcTimestamp::from_datetime(DateTime::from_naive_utc_and_offset(naive, Utc));
        let venue_local = VenueTimestamp::from_utc(venue, utc);
        prop_assert_eq!(
            venue_local.to_utc(),
            utc,
            "venue-local normalization must preserve the UTC instant"
        );
    }

    #[test]
    fn unambiguous_venue_local_round_trips_through_utc(
        year in 2000..2030i32,
        month in 1..=12u32,
        day in 1..=28u32,
        h in 0..24u32,
        m in 0..60u32,
        s in 0..60u32,
        venue in venue(),
    ) {
        let naive = NaiveDate::from_ymd_opt(year, month, day)
            .unwrap()
            .and_hms_opt(h, m, s)
            .unwrap();
        if let Ok(ts) = VenueTimestamp::from_naive_local(venue, naive) {
            // local -> utc -> local must recover the identical venue-local wall clock
            let utc = ts.to_utc();
            let back = VenueTimestamp::from_utc(venue, utc);
            prop_assert_eq!(back, ts, "venue-local wall clock must round-trip through UTC");
        }
    }

    // ---- finite-metric checks -----------------------------------------------

    #[test]
    fn reported_stat_accepts_only_finite(bits in any::<u64>()) {
        let v = f64::from_bits(bits);
        let res = ReportedStat::from_f64(v);
        if v.is_finite() {
            prop_assert!(res.is_ok(), "finite value {v} must be accepted");
        } else {
            let is_rejected = matches!(res, Err(DomainError::NonFiniteMetric { .. }));
            prop_assert!(is_rejected, "non-finite value {v} must be rejected as a typed error");
        }
    }

    // ---- stable serialization (canonical round-trip) ------------------------

    #[test]
    fn price_serialization_is_idempotent(s in positive_decimal()) {
        let p = Price::parse(&s).unwrap();
        let j1 = serde_json::to_string(&p).unwrap();
        let p2: Price = serde_json::from_str(&j1).unwrap();
        let j2 = serde_json::to_string(&p2).unwrap();
        prop_assert_eq!(j1, j2, "Price JSON round-trip must be byte-stable");
    }

    #[test]
    fn money_serialization_is_idempotent(a in positive_decimal()) {
        let m = Money::parse(&a, Currency::KRW).unwrap();
        let j1 = serde_json::to_string(&m).unwrap();
        let m2: Money = serde_json::from_str(&j1).unwrap();
        let j2 = serde_json::to_string(&m2).unwrap();
        prop_assert_eq!(j1, j2, "Money JSON round-trip must be byte-stable");
    }

    #[test]
    fn weight_serialization_is_idempotent_and_bounded(s in "0\\.[0-9]{1,6}|0|1") {
        let w = Weight::parse(&s).unwrap();
        prop_assert!(w.amount().bits() >= 0, "weight must be non-negative");
        prop_assert!(w.amount().bits() <= 1_000_000, "weight must not exceed 1");
        let j1 = serde_json::to_string(&w).unwrap();
        let w2: Weight = serde_json::from_str(&j1).unwrap();
        let j2 = serde_json::to_string(&w2).unwrap();
        prop_assert_eq!(j1, j2, "Weight JSON round-trip must be byte-stable");
    }

    #[test]
    fn instrument_id_round_trips(s in "[A-Z0-9]{1,8}\\.(KRX|ARCA|NYSE|NASDAQ|KIS)") {
        let id = InstrumentId::parse(&s).unwrap();
        prop_assert_eq!(id.to_string(), s, "instrument id must round-trip its canonical string");
        let j = serde_json::to_string(&id).unwrap();
        let back: InstrumentId = serde_json::from_str(&j).unwrap();
        prop_assert_eq!(back, id);
    }

    // ---- invalid-value rejection (no panic on garbage) ----------------------

    #[test]
    fn parsing_garbage_never_panics(s in "\\PC*") {
        let _ = FixedPoint::parse(&s);
        let _ = Price::parse(&s);
        let _ = Quantity::parse(&s);
        let _ = Weight::parse(&s);
        let _ = Money::parse(&s, Currency::KRW);
        let _ = InstrumentId::parse(&s);
        let _ = Currency::from_code(&s);
        let _ = TradingDate::parse(&s);
        let _ = UtcTimestamp::parse_rfc3339(&s);
    }
}
