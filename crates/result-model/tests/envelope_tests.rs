//! Contract tests for the `result-model` warning/error envelope.

use domain::{CorrelationId, Currency, DomainError, Money};
use result_model::{Envelope, ErrorEnvelope, Warning, WarningSeverity};

#[test]
fn error_envelope_matches_documented_shape() {
    let cid = CorrelationId::generate();
    let env = ErrorEnvelope::new("validation_failed", "negative price", cid);
    let json = serde_json::to_string(&env).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["code"], "validation_failed");
    assert_eq!(value["message"], "negative price");
    assert_eq!(value["correlation_id"], cid.to_string());
    // details is omitted when absent
    assert!(value.get("details").is_none());

    // round-trips exactly
    let back: ErrorEnvelope = serde_json::from_str(&json).unwrap();
    assert_eq!(back, env);
}

#[test]
fn from_domain_error_maps_code_and_message() {
    let err = DomainError::NonPositivePrice {
        value: "-100.0000".to_owned(),
    };
    let env = ErrorEnvelope::from_domain(&err, CorrelationId::generate());
    assert_eq!(env.code, "non_positive_price");
    assert!(env.message.contains("price"));
    // DomainError itself serializes with the same stable code tag.
    let json = serde_json::to_string(&err).unwrap();
    assert_eq!(json, r#"{"code":"non_positive_price","value":"-100.0000"}"#);
}

#[test]
fn envelope_ok_and_err() {
    let ok: Envelope<Money> = Envelope::ok(Money::parse("100.0000", Currency::KRW).unwrap());
    assert!(ok.is_success());
    assert_eq!(ok.into_result().unwrap().as_decimal_string(), "100.0000");

    let err_env = ErrorEnvelope::new("boom", "kaboom", CorrelationId::generate());
    let fail: Envelope<Money> = Envelope::err(err_env.clone());
    assert!(!fail.is_success());
    assert_eq!(fail.into_result().unwrap_err(), err_env);
}

#[test]
fn envelope_serialization_round_trip() {
    let ok: Envelope<Money> = Envelope::ok_with_warnings(
        Money::parse("100.0000", Currency::KRW).unwrap(),
        vec![Warning::info("lookback_short", "insufficient history")],
    );
    let json = serde_json::to_string(&ok).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["data"]["amount"], "100.0000");
    assert_eq!(value["data"]["currency"], "KRW");
    assert_eq!(value["warnings"][0]["severity"], "info");
    let back: Envelope<Money> = serde_json::from_str(&json).unwrap();
    assert_eq!(back, ok);

    // a failed envelope omits data and warnings
    let fail: Envelope<Money> =
        Envelope::err(ErrorEnvelope::new("x", "y", CorrelationId::generate()));
    let jf = serde_json::to_string(&fail).unwrap();
    let vf: serde_json::Value = serde_json::from_str(&jf).unwrap();
    assert!(vf.get("data").is_none());
    assert!(vf.get("warnings").is_none());
    assert_eq!(vf["error"]["code"], "x");
}

#[test]
fn warning_with_details_round_trip() {
    let w = Warning::critical("data_stale", "stale close")
        .with_details(serde_json::json!({ "as_of": "2026-08-04" }));
    let json = serde_json::to_string(&w).unwrap();
    assert_eq!(serde_json::from_str::<Warning>(&json).unwrap(), w);
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["severity"], "critical");
    assert_eq!(value["details"]["as_of"], "2026-08-04");
}

#[test]
fn warning_severity_round_trip() {
    for severity in [
        WarningSeverity::Info,
        WarningSeverity::Warning,
        WarningSeverity::Critical,
    ] {
        let json = serde_json::to_string(&severity).unwrap();
        assert_eq!(
            serde_json::from_str::<WarningSeverity>(&json).unwrap(),
            severity
        );
    }
    assert_eq!(
        serde_json::to_string(&WarningSeverity::Critical).unwrap(),
        "\"critical\""
    );
}
