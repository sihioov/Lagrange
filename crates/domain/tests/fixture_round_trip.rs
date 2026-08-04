//! JSON fixture round-trip coverage (Todo 2 manual QA channel).
//!
//!   - `valid_contracts.json` deserializes into the typed contracts and
//!     serializes back to byte-equivalent JSON after canonicalization.
//!   - `invalid_values.json` is deliberately invalid; each value must be
//!     rejected with a TYPED `DomainError` (no panic, no silent acceptance).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use domain::{
    ContentHash, Currency, DomainError, FactorId, FactorVersion, InstrumentId, JobStatus, Money,
    Price, Quantity, ReportedStat, RunId, StrategyId, StrategyVersion, TradingDate, UtcTimestamp,
    VenueTimestamp, Weight,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ValidContracts {
    price: Price,
    quantity: Quantity,
    weight: Weight,
    money: Money,
    instrument_id: InstrumentId,
    trading_date: TradingDate,
    utc_timestamp: UtcTimestamp,
    venue_timestamp: VenueTimestamp,
    strategy_id: StrategyId,
    strategy_version: StrategyVersion,
    factor_id: FactorId,
    factor_version: FactorVersion,
    run_id: RunId,
    job_status: JobStatus,
    content_hash: ContentHash,
    reported_stat: ReportedStat,
}

/// Canonicalize a JSON document: parse and re-emit with deterministic
/// whitespace and key ordering.
fn canonical(json: &str) -> String {
    let value: Value = serde_json::from_str(json).expect("fixture must be valid JSON");
    serde_json::to_string(&value).expect("re-serialize canonical JSON")
}

#[test]
fn valid_fixture_round_trips_byte_equivalently() {
    let raw = include_str!("fixtures/valid_contracts.json");

    // Structural identity: every field survives the typed round-trip.
    let original: Value = serde_json::from_str(raw).expect("parse fixture");
    let typed: ValidContracts = serde_json::from_str(raw).expect("deserialize typed contracts");
    let re_serialized = serde_json::to_string(&typed).expect("serialize typed contracts");
    let re_parsed: Value = serde_json::from_str(&re_serialized).expect("parse re-serialized");

    assert_eq!(original, re_parsed, "typed round-trip must preserve every field");

    // Byte equivalence after canonicalization.
    assert_eq!(
        canonical(raw),
        canonical(&re_serialized),
        "canonicalized JSON bytes must be identical across the round-trip"
    );
}

#[test]
fn invalid_fixture_is_rejected_as_typed_errors() {
    let raw = include_str!("fixtures/invalid_values.json");
    let value: Value = serde_json::from_str(raw).expect("fixture must be valid JSON");

    let err = Price::parse(value["negative_price"].as_str().unwrap())
        .expect_err("negative price must be rejected");
    assert!(matches!(err, DomainError::NonPositivePrice { .. }), "got: {err}");

    let err = Quantity::parse(value["negative_quantity"].as_str().unwrap())
        .expect_err("negative quantity must be rejected");
    assert!(matches!(err, DomainError::NegativeQuantity { .. }), "got: {err}");

    let err = Weight::parse(value["weight_over_one"].as_str().unwrap())
        .expect_err("weight over 1 must be rejected");
    assert!(matches!(err, DomainError::WeightOutOfRange { .. }), "got: {err}");

    let negative_money = &value["money_negative"];
    let err = Money::parse(
        negative_money["amount"].as_str().unwrap(),
        Currency::from_code(negative_money["currency"].as_str().unwrap()).expect("valid currency"),
    )
    .expect_err("negative money must be rejected");
    assert!(matches!(err, DomainError::NegativeMoney { .. }), "got: {err}");

    let err = Currency::from_code(value["bad_currency"].as_str().unwrap())
        .expect_err("bad currency must be rejected");
    assert!(matches!(err, DomainError::InvalidCurrency { .. }), "got: {err}");

    let err = InstrumentId::parse(value["bad_instrument"].as_str().unwrap())
        .expect_err("bad instrument must be rejected");
    assert!(matches!(err, DomainError::InvalidId { .. }), "got: {err}");
}
