use market_data::intraday_quotes::{
    IntradayQuote, IntradayQuoteDirection, IntradayQuoteError, parse_intraday_quote,
};

const SYMBOL: &str = "005930";
const PROVIDER_PROSE_SENTINEL: &str = "SYNTHETIC_PROVIDER_PROSE_SENTINEL";

#[allow(clippy::too_many_arguments)]
fn fixture(
    symbol: &str,
    price: &str,
    change: &str,
    percent: &str,
    sign: &str,
    base_price: &str,
    status: &str,
    temporary_stop: &str,
) -> Vec<u8> {
    format!(
        r#"{{"rt_cd":"0","msg_cd":"M000","msg1":"{}","output":{{"stck_shrn_iscd":"{}","stck_prpr":"{}","prdy_vrss":"{}","prdy_ctrt":"{}","prdy_vrss_sign":"{}","stck_sdpr":"{}","iscd_stat_cls_code":"{}","temp_stop_yn":"{}","unrelated_field":{{"discard":"{}"}}}}}}"#,
        PROVIDER_PROSE_SENTINEL,
        symbol,
        price,
        change,
        percent,
        sign,
        base_price,
        status,
        temporary_stop,
        PROVIDER_PROSE_SENTINEL,
    )
    .into_bytes()
}

fn valid_fixture() -> Vec<u8> {
    fixture(SYMBOL, "72500", "1500", "2.11", "2", "71000", "00", "N")
}

fn replace_once(bytes: Vec<u8>, from: &str, to: &str) -> Vec<u8> {
    let text = String::from_utf8(bytes).expect("synthetic fixture is UTF-8");
    assert_eq!(
        text.matches(from).count(),
        1,
        "fixture replacement is unique"
    );
    text.replacen(from, to, 1).into_bytes()
}

fn assert_code(result: Result<IntradayQuote, IntradayQuoteError>, expected: &str) {
    let error = result.expect_err("fixture should be rejected");
    assert_eq!(error.code(), expected);
    let display = error.to_string();
    let debug = format!("{error:?}");
    assert!(display == "PROVIDER_RESPONSE_INVALID" || display == "QUOTE_VALUE_INVALID");
    assert!(!display.contains(PROVIDER_PROSE_SENTINEL));
    assert!(!debug.contains(PROVIDER_PROSE_SENTINEL));
}

#[test]
fn accepts_all_five_direction_codes_and_preserves_signed_values() {
    let cases = [
        ("1", "100", "1.00", IntradayQuoteDirection::LimitUp),
        ("2", "100", "1.00", IntradayQuoteDirection::Up),
        ("3", "0", "0.00", IntradayQuoteDirection::Flat),
        ("4", "-100", "-1.00", IntradayQuoteDirection::LimitDown),
        ("5", "-100", "-1.00", IntradayQuoteDirection::Down),
    ];

    for (sign, change, percent, direction) in cases {
        let quote = parse_intraday_quote(
            SYMBOL,
            &fixture(SYMBOL, "72500", change, percent, sign, "71000", "00", "N"),
        )
        .unwrap_or_else(|error| panic!("valid direction fixture for sign {sign}: {error:?}"));
        assert_eq!(quote.direction, direction);
        assert_eq!(quote.direction.as_str(), direction.as_str());
        assert_eq!(quote.change_from_previous_day, change);
        assert_eq!(quote.change_percent_from_previous_day, percent);
        assert!(!quote.halted);
    }
}

#[test]
fn preserves_identity_price_and_base_price_without_previous_close_claim() {
    let quote = parse_intraday_quote(SYMBOL, &valid_fixture()).expect("valid fixture");
    assert_eq!(
        quote,
        IntradayQuote {
            symbol: SYMBOL.to_owned(),
            price: "72500".to_owned(),
            change_from_previous_day: "1500".to_owned(),
            change_percent_from_previous_day: "2.11".to_owned(),
            direction: IntradayQuoteDirection::Up,
            base_price: "71000".to_owned(),
            halted: false,
        }
    );
    assert_eq!(quote.base_price, "71000");
    assert_ne!(quote.price, quote.base_price);
    assert!(!format!("{quote:?}").contains("previous_close"));
    assert!(!format!("{quote:?}").contains("received_at"));
}

#[test]
fn rejects_direction_value_mismatches() {
    for (sign, change, percent) in [
        ("1", "100", "-1"),
        ("2", "0", "1"),
        ("3", "0", "1"),
        ("4", "-100", "1"),
        ("5", "-100", "0"),
    ] {
        assert_code(
            parse_intraday_quote(
                SYMBOL,
                &fixture(SYMBOL, "72500", change, percent, sign, "71000", "00", "N"),
            ),
            "QUOTE_VALUE_INVALID",
        );
    }
}

#[test]
fn rejects_malformed_json_and_non_object_envelopes() {
    assert_code(
        parse_intraday_quote(SYMBOL, br#"{"rt_cd":"0","output":{"#),
        "PROVIDER_RESPONSE_INVALID",
    );
    assert_code(
        parse_intraday_quote(SYMBOL, br#"null"#),
        "PROVIDER_RESPONSE_INVALID",
    );
    assert_code(
        parse_intraday_quote(SYMBOL, br#"[]"#),
        "PROVIDER_RESPONSE_INVALID",
    );
    assert_code(
        parse_intraday_quote(SYMBOL, br#"{}{}"#),
        "PROVIDER_RESPONSE_INVALID",
    );
}

#[test]
fn rejects_missing_or_wrong_type_envelope_fields() {
    let missing_rt_cd = replace_once(valid_fixture(), r#""rt_cd":"0","#, "");
    assert_code(
        parse_intraday_quote(SYMBOL, &missing_rt_cd),
        "PROVIDER_RESPONSE_INVALID",
    );

    let numeric_rt_cd = replace_once(valid_fixture(), r#""rt_cd":"0""#, r#""rt_cd":0"#);
    assert_code(
        parse_intraday_quote(SYMBOL, &numeric_rt_cd),
        "PROVIDER_RESPONSE_INVALID",
    );

    let nonzero_rt_cd = replace_once(valid_fixture(), r#""rt_cd":"0""#, r#""rt_cd":"1""#);
    assert_code(
        parse_intraday_quote(SYMBOL, &nonzero_rt_cd),
        "PROVIDER_RESPONSE_INVALID",
    );

    let missing_output = replace_once(valid_fixture(), r#","output":{"#, "");
    assert_code(
        parse_intraday_quote(SYMBOL, &missing_output),
        "PROVIDER_RESPONSE_INVALID",
    );

    let array_output = replace_once(valid_fixture(), r#""output":{"#, r#""output":[],"#);
    assert_code(
        parse_intraday_quote(SYMBOL, &array_output),
        "PROVIDER_RESPONSE_INVALID",
    );
}

#[test]
fn rejects_missing_and_wrong_type_required_output_fields() {
    let missing_price = replace_once(valid_fixture(), r#","stck_prpr":"72500""#, "");
    assert_code(
        parse_intraday_quote(SYMBOL, &missing_price),
        "PROVIDER_RESPONSE_INVALID",
    );

    let numeric_price = replace_once(
        valid_fixture(),
        r#""stck_prpr":"72500""#,
        r#""stck_prpr":72500"#,
    );
    assert_code(
        parse_intraday_quote(SYMBOL, &numeric_price),
        "PROVIDER_RESPONSE_INVALID",
    );

    let numeric_status = replace_once(
        valid_fixture(),
        r#""iscd_stat_cls_code":"00""#,
        r#""iscd_stat_cls_code":0"#,
    );
    assert_code(
        parse_intraday_quote(SYMBOL, &numeric_status),
        "PROVIDER_RESPONSE_INVALID",
    );

    let missing_stop = replace_once(valid_fixture(), r#","temp_stop_yn":"N""#, "");
    assert_code(
        parse_intraday_quote(SYMBOL, &missing_stop),
        "PROVIDER_RESPONSE_INVALID",
    );
}

#[test]
fn rejects_duplicate_envelope_and_critical_output_fields() {
    let duplicate_rt_cd = replace_once(
        valid_fixture(),
        r#""rt_cd":"0""#,
        r#""rt_cd":"0","rt_cd":"0""#,
    );
    assert_code(
        parse_intraday_quote(SYMBOL, &duplicate_rt_cd),
        "PROVIDER_RESPONSE_INVALID",
    );

    let duplicate_output = replace_once(
        valid_fixture(),
        r#""output":{"#,
        r#""output":{},"output":{"#,
    );
    assert_code(
        parse_intraday_quote(SYMBOL, &duplicate_output),
        "PROVIDER_RESPONSE_INVALID",
    );

    let duplicate_price = replace_once(
        valid_fixture(),
        r#""stck_prpr":"72500""#,
        r#""stck_prpr":"72500","stck_prpr":"72500""#,
    );
    assert_code(
        parse_intraday_quote(SYMBOL, &duplicate_price),
        "PROVIDER_RESPONSE_INVALID",
    );
}

#[test]
fn rejects_identity_mismatch_and_invalid_requested_symbol() {
    let wrong_response_symbol = replace_once(
        fixture(SYMBOL, "72500", "1500", "2.11", "2", "71000", "00", "N"),
        SYMBOL,
        "005931",
    );
    assert_code(
        parse_intraday_quote(SYMBOL, &wrong_response_symbol),
        "PROVIDER_RESPONSE_INVALID",
    );

    assert_code(
        parse_intraday_quote("00593", &valid_fixture()),
        "PROVIDER_RESPONSE_INVALID",
    );
    assert_code(
        parse_intraday_quote("00593A", &valid_fixture()),
        "PROVIDER_RESPONSE_INVALID",
    );

    let non_ascii_response_symbol = fixture(
        "００５９３０",
        "72500",
        "1500",
        "2.11",
        "2",
        "71000",
        "00",
        "N",
    );
    assert_code(
        parse_intraday_quote(SYMBOL, &non_ascii_response_symbol),
        "PROVIDER_RESPONSE_INVALID",
    );
}

#[test]
fn accepts_numeric_boundaries_and_preserves_all_digits() {
    let max = "999999999999.99999999";
    let quote = parse_intraday_quote(
        SYMBOL,
        &fixture(SYMBOL, max, max, "1.00000000", "2", max, "00", "N"),
    )
    .expect("numeric(20,8) boundary should be accepted");
    assert_eq!(quote.price, max);
    assert_eq!(quote.change_from_previous_day, max);
    assert_eq!(quote.base_price, max);

    let eight_fraction_digits = fixture(
        SYMBOL,
        "1.12345678",
        "1.12345678",
        "1.12345678",
        "2",
        "1.12345678",
        "00",
        "N",
    );
    assert!(parse_intraday_quote(SYMBOL, &eight_fraction_digits).is_ok());
}

#[test]
fn rejects_decimal_overflow_excess_fraction_and_noncanonical_strings() {
    for value in [
        "1000000000000",
        "1.123456789",
        "+1",
        "1,000",
        " 1",
        "1 ",
        "1e3",
        "01",
        "-0",
        "-0.00",
        "-0.00000000",
        "１２",
    ] {
        let bytes = fixture(SYMBOL, value, "1500", "2.11", "2", "71000", "00", "N");
        assert_code(parse_intraday_quote(SYMBOL, &bytes), "QUOTE_VALUE_INVALID");
    }

    let json_number = replace_once(
        valid_fixture(),
        r#""stck_prpr":"72500""#,
        r#""stck_prpr":72500"#,
    );
    assert_code(
        parse_intraday_quote(SYMBOL, &json_number),
        "PROVIDER_RESPONSE_INVALID",
    );
}

#[test]
fn rejects_negative_zero_in_changes_and_requires_positive_price_and_base() {
    for (price, base_price) in [
        ("0", "71000"),
        ("-1", "71000"),
        ("72500", "0"),
        ("72500", "-1"),
    ] {
        assert_code(
            parse_intraday_quote(
                SYMBOL,
                &fixture(SYMBOL, price, "1500", "2.11", "2", base_price, "00", "N"),
            ),
            "QUOTE_VALUE_INVALID",
        );
    }

    for (change, percent) in [("-0", "0"), ("-0.00", "0"), ("0", "-0.00000000")] {
        assert_code(
            parse_intraday_quote(
                SYMBOL,
                &fixture(SYMBOL, "72500", change, percent, "3", "71000", "00", "N"),
            ),
            "QUOTE_VALUE_INVALID",
        );
    }
}

#[test]
fn status_58_or_temporary_stop_y_marks_quote_halted() {
    let status_halted = parse_intraday_quote(
        SYMBOL,
        &fixture(SYMBOL, "72500", "1500", "2.11", "2", "71000", "58", "N"),
    )
    .expect("status 58 fixture");
    assert!(status_halted.halted);

    let temporary_stop_halted = parse_intraday_quote(
        SYMBOL,
        &fixture(SYMBOL, "72500", "1500", "2.11", "2", "71000", "00", "Y"),
    )
    .expect("temporary stop fixture");
    assert!(temporary_stop_halted.halted);

    let other_status = parse_intraday_quote(
        SYMBOL,
        &fixture(SYMBOL, "72500", "1500", "2.11", "2", "71000", "123", "N"),
    )
    .expect("well-formed uninterpreted status");
    assert!(!other_status.halted);
    assert!(!format!("{other_status:?}").contains("123"));
}

#[test]
fn rejects_invalid_status_stop_and_sign_values() {
    for status in ["", "5", "1234", "Ａ０", "AB"] {
        assert_code(
            parse_intraday_quote(
                SYMBOL,
                &fixture(SYMBOL, "72500", "1500", "2.11", "2", "71000", status, "N"),
            ),
            "QUOTE_VALUE_INVALID",
        );
    }
    for temporary_stop in ["", "X", "y", "0", "예"] {
        assert_code(
            parse_intraday_quote(
                SYMBOL,
                &fixture(
                    SYMBOL,
                    "72500",
                    "1500",
                    "2.11",
                    "2",
                    "71000",
                    "00",
                    temporary_stop,
                ),
            ),
            "QUOTE_VALUE_INVALID",
        );
    }
    assert_code(
        parse_intraday_quote(
            SYMBOL,
            &fixture(SYMBOL, "72500", "1500", "2.11", "0", "71000", "00", "N"),
        ),
        "QUOTE_VALUE_INVALID",
    );
}

#[test]
fn tolerates_unrelated_output_fields_but_discards_provider_prose() {
    let quote = parse_intraday_quote(SYMBOL, &valid_fixture()).expect("extra fields are allowed");
    let debug = format!("{quote:?}");
    assert!(!debug.contains(PROVIDER_PROSE_SENTINEL));
    assert!(!debug.contains("msg1"));
    assert!(!debug.contains("unrelated_field"));
}

#[test]
fn parser_errors_never_echo_body_or_provider_prose() {
    let body = format!(
        r#"{{"rt_cd":"0","msg1":"{}","output":["BODY_SENTINEL"]}} trailing"#,
        PROVIDER_PROSE_SENTINEL,
    );
    let result = parse_intraday_quote(SYMBOL, body.as_bytes());
    let error = result.expect_err("malformed synthetic body");
    assert_eq!(error, IntradayQuoteError::ProviderResponseInvalid);
    assert!(!error.to_string().contains(PROVIDER_PROSE_SENTINEL));
    assert!(!format!("{error:?}").contains(PROVIDER_PROSE_SENTINEL));
    assert!(!error.to_string().contains("BODY_SENTINEL"));
    assert!(!format!("{error:?}").contains("BODY_SENTINEL"));
}
