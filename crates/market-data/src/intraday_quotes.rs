//! Fixture-backed parsing for the allowlisted KIS current-price response.
//!
//! This module deliberately has no transport, clock, database, Raw, or EOD
//! dependencies. The caller is responsible for the HTTP success and request
//! allowlist preconditions. This parser only turns one response body into a
//! validated, ephemeral quote value.

use std::fmt;

use serde::Deserialize;
use serde::de::{self, Deserializer, IgnoredAny, MapAccess, Visitor};

/// The direction classification carried by KIS prdy_vrss_sign.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntradayQuoteDirection {
    Up,
    Down,
    Flat,
    LimitUp,
    LimitDown,
}

impl IntradayQuoteDirection {
    /// The stable application spelling used by the later cache/API producer.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Up => "UP",
            Self::Down => "DOWN",
            Self::Flat => "FLAT",
            Self::LimitUp => "LIMIT_UP",
            Self::LimitDown => "LIMIT_DOWN",
        }
    }
}

/// A validated KIS intraday quote with no fabricated time or previous-close
/// metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntradayQuote {
    /// The six-digit KIS short code returned by the provider.
    pub symbol: String,
    /// KIS stck_prpr, preserved as the canonical provider string.
    pub price: String,
    /// KIS prdy_vrss, preserved as supplied.
    pub change_from_previous_day: String,
    /// KIS prdy_ctrt, preserved as supplied.
    pub change_percent_from_previous_day: String,
    /// KIS prdy_vrss_sign, mapped without applying a second sign.
    pub direction: IntradayQuoteDirection,
    /// KIS stck_sdpr (stock base price), not a previous-close claim.
    pub base_price: String,
    /// Halted when KIS status is exactly 58 or temp_stop_yn is Y.
    pub halted: bool,
}

/// The only two stable parser failure categories exposed to later producer
/// code. Neither variant carries provider values, response bytes, or prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum IntradayQuoteError {
    /// The body is malformed, structurally invalid, duplicated, or has an
    /// identity/status envelope that cannot be accepted as a provider reply.
    #[error("PROVIDER_RESPONSE_INVALID")]
    ProviderResponseInvalid,
    /// A structurally valid string has an invalid quote value or inconsistent
    /// direction/stop/status semantics.
    #[error("QUOTE_VALUE_INVALID")]
    QuoteValueInvalid,
}

impl IntradayQuoteError {
    /// Stable error code for API/cache layers.
    pub const fn code(self) -> &'static str {
        match self {
            Self::ProviderResponseInvalid => "PROVIDER_RESPONSE_INVALID",
            Self::QuoteValueInvalid => "QUOTE_VALUE_INVALID",
        }
    }
}

/// Parse one already-received, successful KIS current-price response.
///
/// The requested symbol must be exactly six ASCII digits. Only the approved
/// identity and seven quote fields are read; unrelated output fields are
/// consumed and discarded. No network call is possible from this function.
pub fn parse_intraday_quote(
    requested_symbol: &str,
    response_bytes: &[u8],
) -> Result<IntradayQuote, IntradayQuoteError> {
    if !is_six_ascii_digits(requested_symbol) {
        return Err(IntradayQuoteError::ProviderResponseInvalid);
    }

    let mut deserializer = serde_json::Deserializer::from_slice(response_bytes);
    let envelope = WireEnvelope::deserialize(&mut deserializer)
        .map_err(|_| IntradayQuoteError::ProviderResponseInvalid)?;
    deserializer
        .end()
        .map_err(|_| IntradayQuoteError::ProviderResponseInvalid)?;

    if envelope.rt_cd.as_deref() != Some("0") {
        return Err(IntradayQuoteError::ProviderResponseInvalid);
    }
    let output = envelope
        .output
        .ok_or(IntradayQuoteError::ProviderResponseInvalid)?;

    let symbol = output
        .stck_shrn_iscd
        .ok_or(IntradayQuoteError::ProviderResponseInvalid)?;
    if !is_six_ascii_digits(&symbol) || symbol != requested_symbol {
        return Err(IntradayQuoteError::ProviderResponseInvalid);
    }

    let price = parse_decimal(
        output
            .stck_prpr
            .ok_or(IntradayQuoteError::ProviderResponseInvalid)?,
        true,
    )?
    .0;
    let (change_from_previous_day, change_kind) = parse_decimal(
        output
            .prdy_vrss
            .ok_or(IntradayQuoteError::ProviderResponseInvalid)?,
        false,
    )?;
    let (change_percent_from_previous_day, percent_kind) = parse_decimal(
        output
            .prdy_ctrt
            .ok_or(IntradayQuoteError::ProviderResponseInvalid)?,
        false,
    )?;
    let direction = parse_direction(
        output
            .prdy_vrss_sign
            .ok_or(IntradayQuoteError::ProviderResponseInvalid)?,
    )?;
    if !direction_matches_values(direction, change_kind, percent_kind) {
        return Err(IntradayQuoteError::QuoteValueInvalid);
    }

    let base_price = parse_decimal(
        output
            .stck_sdpr
            .ok_or(IntradayQuoteError::ProviderResponseInvalid)?,
        true,
    )?
    .0;
    let status = output
        .iscd_stat_cls_code
        .ok_or(IntradayQuoteError::ProviderResponseInvalid)?;
    if !is_well_formed_status(&status) {
        return Err(IntradayQuoteError::QuoteValueInvalid);
    }
    let temporary_stop = output
        .temp_stop_yn
        .ok_or(IntradayQuoteError::ProviderResponseInvalid)?;
    let temporary_stop = match temporary_stop.as_str() {
        "Y" => true,
        "N" => false,
        _ => return Err(IntradayQuoteError::QuoteValueInvalid),
    };

    Ok(IntradayQuote {
        symbol,
        price,
        change_from_previous_day,
        change_percent_from_previous_day,
        direction,
        base_price,
        halted: status == "58" || temporary_stop,
    })
}

/// Explicit KIS-named entry point for callers that prefer the provider in the
/// function name. It has the same provider-free behavior as
/// parse_intraday_quote.
pub fn parse_kis_intraday_quote(
    requested_symbol: &str,
    response_bytes: &[u8],
) -> Result<IntradayQuote, IntradayQuoteError> {
    parse_intraday_quote(requested_symbol, response_bytes)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecimalKind {
    Positive,
    Zero,
    Negative,
}

fn parse_decimal(
    value: String,
    require_positive: bool,
) -> Result<(String, DecimalKind), IntradayQuoteError> {
    let kind = classify_decimal(&value).ok_or(IntradayQuoteError::QuoteValueInvalid)?;
    if require_positive && kind != DecimalKind::Positive {
        return Err(IntradayQuoteError::QuoteValueInvalid);
    }
    Ok((value, kind))
}

fn classify_decimal(value: &str) -> Option<DecimalKind> {
    let bytes = value.as_bytes();
    if bytes.is_empty() {
        return None;
    }

    let negative = bytes[0] == b'-';
    let integer_start = usize::from(negative);
    if integer_start == bytes.len() {
        return None;
    }

    let mut integer_end = integer_start;
    match bytes[integer_start] {
        b'0' => {
            integer_end += 1;
            if integer_end < bytes.len() && bytes[integer_end].is_ascii_digit() {
                return None;
            }
        }
        byte if (b'1'..=b'9').contains(&byte) => {
            integer_end += 1;
            while integer_end < bytes.len() && bytes[integer_end].is_ascii_digit() {
                integer_end += 1;
            }
        }
        _ => return None,
    }

    let integer_digits = integer_end - integer_start;
    if integer_digits > 12 {
        return None;
    }

    let mut fraction_start = integer_end;
    if integer_end < bytes.len() {
        if bytes[integer_end] != b'.' {
            return None;
        }
        fraction_start += 1;
        let mut fraction_end = fraction_start;
        while fraction_end < bytes.len() && bytes[fraction_end].is_ascii_digit() {
            fraction_end += 1;
        }
        let count = fraction_end - fraction_start;
        if !(1..=8).contains(&count) || fraction_end != bytes.len() {
            return None;
        }
    }

    let integer_is_zero = bytes[integer_start] == b'0';
    let fraction_is_zero = bytes[fraction_start..].iter().all(|byte| *byte == b'0');
    if negative && integer_is_zero && fraction_is_zero {
        return None;
    }

    if integer_is_zero && fraction_is_zero {
        Some(DecimalKind::Zero)
    } else if negative {
        Some(DecimalKind::Negative)
    } else {
        Some(DecimalKind::Positive)
    }
}

fn parse_direction(value: String) -> Result<IntradayQuoteDirection, IntradayQuoteError> {
    match value.as_str() {
        "1" => Ok(IntradayQuoteDirection::LimitUp),
        "2" => Ok(IntradayQuoteDirection::Up),
        "3" => Ok(IntradayQuoteDirection::Flat),
        "4" => Ok(IntradayQuoteDirection::LimitDown),
        "5" => Ok(IntradayQuoteDirection::Down),
        _ => Err(IntradayQuoteError::QuoteValueInvalid),
    }
}

fn direction_matches_values(
    direction: IntradayQuoteDirection,
    amount: DecimalKind,
    percent: DecimalKind,
) -> bool {
    match direction {
        IntradayQuoteDirection::LimitUp | IntradayQuoteDirection::Up => {
            amount == DecimalKind::Positive && percent == DecimalKind::Positive
        }
        IntradayQuoteDirection::Flat => amount == DecimalKind::Zero && percent == DecimalKind::Zero,
        IntradayQuoteDirection::LimitDown | IntradayQuoteDirection::Down => {
            amount == DecimalKind::Negative && percent == DecimalKind::Negative
        }
    }
}

fn is_six_ascii_digits(value: &str) -> bool {
    value.len() == 6 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_well_formed_status(value: &str) -> bool {
    matches!(value.len(), 2 | 3) && value.bytes().all(|byte| byte.is_ascii_digit())
}

struct WireEnvelope {
    rt_cd: Option<String>,
    output: Option<WireOutput>,
}

struct WireOutput {
    stck_shrn_iscd: Option<String>,
    stck_prpr: Option<String>,
    prdy_vrss: Option<String>,
    prdy_ctrt: Option<String>,
    prdy_vrss_sign: Option<String>,
    stck_sdpr: Option<String>,
    iscd_stat_cls_code: Option<String>,
    temp_stop_yn: Option<String>,
}

fn duplicate_field<E: de::Error>() -> E {
    E::custom("duplicate critical field")
}

impl<'de> Deserialize<'de> for WireEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct EnvelopeVisitor;

        impl<'de> Visitor<'de> for EnvelopeVisitor {
            type Value = WireEnvelope;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a JSON object")
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut rt_cd = None;
                let mut output = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "rt_cd" => {
                            if rt_cd.is_some() {
                                return Err(duplicate_field());
                            }
                            rt_cd = Some(map.next_value::<String>()?);
                        }
                        "output" => {
                            if output.is_some() {
                                return Err(duplicate_field());
                            }
                            output = Some(map.next_value::<WireOutput>()?);
                        }
                        _ => {
                            let _: IgnoredAny = map.next_value()?;
                        }
                    }
                }
                Ok(WireEnvelope { rt_cd, output })
            }
        }

        deserializer.deserialize_map(EnvelopeVisitor)
    }
}

impl<'de> Deserialize<'de> for WireOutput {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct OutputVisitor;

        impl<'de> Visitor<'de> for OutputVisitor {
            type Value = WireOutput;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a JSON object")
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut stck_shrn_iscd = None;
                let mut stck_prpr = None;
                let mut prdy_vrss = None;
                let mut prdy_ctrt = None;
                let mut prdy_vrss_sign = None;
                let mut stck_sdpr = None;
                let mut iscd_stat_cls_code = None;
                let mut temp_stop_yn = None;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "stck_shrn_iscd" => {
                            if stck_shrn_iscd.is_some() {
                                return Err(duplicate_field());
                            }
                            stck_shrn_iscd = Some(map.next_value::<String>()?);
                        }
                        "stck_prpr" => {
                            if stck_prpr.is_some() {
                                return Err(duplicate_field());
                            }
                            stck_prpr = Some(map.next_value::<String>()?);
                        }
                        "prdy_vrss" => {
                            if prdy_vrss.is_some() {
                                return Err(duplicate_field());
                            }
                            prdy_vrss = Some(map.next_value::<String>()?);
                        }
                        "prdy_ctrt" => {
                            if prdy_ctrt.is_some() {
                                return Err(duplicate_field());
                            }
                            prdy_ctrt = Some(map.next_value::<String>()?);
                        }
                        "prdy_vrss_sign" => {
                            if prdy_vrss_sign.is_some() {
                                return Err(duplicate_field());
                            }
                            prdy_vrss_sign = Some(map.next_value::<String>()?);
                        }
                        "stck_sdpr" => {
                            if stck_sdpr.is_some() {
                                return Err(duplicate_field());
                            }
                            stck_sdpr = Some(map.next_value::<String>()?);
                        }
                        "iscd_stat_cls_code" => {
                            if iscd_stat_cls_code.is_some() {
                                return Err(duplicate_field());
                            }
                            iscd_stat_cls_code = Some(map.next_value::<String>()?);
                        }
                        "temp_stop_yn" => {
                            if temp_stop_yn.is_some() {
                                return Err(duplicate_field());
                            }
                            temp_stop_yn = Some(map.next_value::<String>()?);
                        }
                        _ => {
                            let _: IgnoredAny = map.next_value()?;
                        }
                    }
                }

                Ok(WireOutput {
                    stck_shrn_iscd,
                    stck_prpr,
                    prdy_vrss,
                    prdy_ctrt,
                    prdy_vrss_sign,
                    stck_sdpr,
                    iscd_stat_cls_code,
                    temp_stop_yn,
                })
            }
        }

        deserializer.deserialize_map(OutputVisitor)
    }
}
