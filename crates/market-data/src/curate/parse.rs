//! Parsing of the raw provider response bytes into typed curation inputs
//! (Todo 10). The raw bytes stay immutable in the raw zone; parsing is
//! read-only and fails typed on any structural surprise. Fixture bytes are
//! DATA, never code — they are only ever deserialized, never executed.

use domain::{Currency, FixedPoint, TradingDate, UtcTimestamp};
use serde::Deserialize;
use serde_json::Value;

use super::CurateError;

/// The raw bars response shape (Todo 8 contract fixture).
#[derive(Debug, Clone, Deserialize)]
pub struct RawBarsDoc {
    pub dataset_id: String,
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub currency: Option<Currency>,
    #[serde(default)]
    pub instruments: Vec<RawInstrumentRef>,
    #[serde(default)]
    pub bars: Vec<RawBarRow>,
}

/// Instrument reference inside the bars document.
#[derive(Debug, Clone, Deserialize)]
pub struct RawInstrumentRef {
    pub symbol: String,
    #[serde(default)]
    pub currency: Option<Currency>,
}

/// One raw bar row (numbers stay JSON numbers until validated).
#[derive(Debug, Clone, Deserialize)]
pub struct RawBarRow {
    #[serde(alias = "instrument_id")]
    pub instrument: String,
    pub date: String,
    pub open: Value,
    pub high: Value,
    pub low: Value,
    pub close: Value,
    pub volume: Value,
    #[serde(default)]
    pub value: Option<Value>,
}

impl RawBarRow {
    /// The optional trading value (거래대금) as i64.
    pub fn trading_value(&self) -> Result<Option<i64>, CurateError> {
        match &self.value {
            None => Ok(None),
            Some(v) => number_i64(v).map(Some).map_err(|e| CurateError::MalformedBars {
                reason: format!("bar value: {e}"),
            }),
        }
    }
}

/// The raw corporate-actions response shape. Field names follow the Todo 6
/// split-dividend fixture; `instrument_id` is accepted as an alias.
#[derive(Debug, Clone, Deserialize)]
pub struct RawActionsDoc {
    #[serde(default)]
    pub actions: Vec<RawActionRow>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawActionRow {
    #[serde(alias = "instrument_id")]
    pub instrument: String,
    #[serde(rename = "type", alias = "event_type")]
    pub event_type: String,
    #[serde(default)]
    pub ex_date: Option<String>,
    #[serde(default)]
    pub record_date: Option<String>,
    #[serde(default)]
    pub pay_date: Option<String>,
    #[serde(default)]
    pub ratio: Option<String>,
    #[serde(default)]
    pub split_factor: Option<Value>,
    #[serde(default)]
    pub amount_per_share: Option<String>,
    #[serde(default)]
    pub tax_withholding_pct: Option<String>,
    #[serde(default)]
    pub currency: Option<Currency>,
    #[serde(default)]
    pub announced_at: Option<String>,
}

/// Parses the raw bars response bytes.
pub fn parse_bars(bytes: &[u8]) -> Result<RawBarsDoc, CurateError> {
    serde_json::from_slice(bytes).map_err(|e| CurateError::MalformedBars {
        reason: format!("not a valid bars document: {e}"),
    })
}

/// Parses the raw corporate-actions response bytes.
pub fn parse_actions(bytes: &[u8]) -> Result<RawActionsDoc, CurateError> {
    serde_json::from_slice(bytes).map_err(|e| CurateError::MalformedAction {
        reason: format!("not a valid corporate-actions document: {e}"),
    })
}

/// A JSON number as a fixed-point decimal; non-finite floats are rejected.
pub fn number_to_fixed(value: &Value) -> Result<FixedPoint, CurateError> {
    let Some(number) = value.as_number() else {
        return Err(CurateError::MalformedBars {
            reason: format!("expected a JSON number, got {value}"),
        });
    };
    if let Some(integer) = number.as_i64() {
        return FixedPoint::parse(&integer.to_string()).map_err(|e| CurateError::MalformedBars {
            reason: format!("invalid integer price {integer}: {e}"),
        });
    }
    let float = number.as_f64().ok_or_else(|| CurateError::MalformedBars {
        reason: format!("number {value} is out of range"),
    })?;
    if !float.is_finite() {
        return Err(CurateError::MalformedBars {
            reason: format!("non-finite number {value} is not a valid price"),
        });
    }
    FixedPoint::parse(&float.to_string()).map_err(|e| CurateError::MalformedBars {
        reason: format!("invalid number {float}: {e}"),
    })
}

/// A JSON number as i64.
fn number_i64(value: &Value) -> Result<i64, CurateError> {
    let Some(number) = value.as_number() else {
        return Err(CurateError::MalformedBars {
            reason: format!("expected a JSON number, got {value}"),
        });
    };
    if let Some(integer) = number.as_i64() {
        return Ok(integer);
    }
    let float = number.as_f64().ok_or_else(|| CurateError::MalformedBars {
        reason: format!("number {value} is out of range"),
    })?;
    if !float.is_finite() {
        return Err(CurateError::MalformedBars {
            reason: format!("non-finite number {value}"),
        });
    }
    Ok(float as i64)
}

/// Parses a date field, requiring the field when `required` is true.
pub fn parse_date(
    raw: Option<&String>,
    what: &str,
    required: bool,
) -> Result<Option<TradingDate>, CurateError> {
    match raw {
        None if required => Err(CurateError::MalformedAction {
            reason: format!("missing required field {what:?}"),
        }),
        None => Ok(None),
        Some(s) => TradingDate::parse(s)
            .map(Some)
            .map_err(|e| CurateError::MalformedAction {
                reason: format!("invalid {what} {s:?}: {e}"),
            }),
    }
}

/// Parses a timestamp field, requiring the field when `required` is true.
pub fn parse_timestamp(
    raw: Option<&String>,
    what: &str,
    required: bool,
) -> Result<Option<UtcTimestamp>, CurateError> {
    match raw {
        None if required => Err(CurateError::MalformedAction {
            reason: format!("missing required field {what:?}"),
        }),
        None => Ok(None),
        Some(s) => UtcTimestamp::parse_rfc3339(s)
            .map(Some)
            .map_err(|e| CurateError::MalformedAction {
                reason: format!("invalid {what} {s:?}: {e}"),
            }),
    }
}

/// Parses a fixed-point string field.
pub fn parse_fixed(
    raw: Option<&String>,
    what: &str,
) -> Result<Option<FixedPoint>, CurateError> {
    match raw {
        None => Ok(None),
        Some(s) => FixedPoint::parse(s)
            .map(Some)
            .map_err(|e| CurateError::MalformedAction {
                reason: format!("invalid {what} {s:?}: {e}"),
            }),
    }
}

/// Parses a fixed-point field that may arrive as a JSON number or a decimal
/// string (e.g. the split-dividend fixture's `"split_factor": 2`).
pub fn parse_fixed_value(
    raw: Option<&Value>,
    what: &str,
) -> Result<Option<FixedPoint>, CurateError> {
    match raw {
        None => Ok(None),
        Some(Value::String(s)) => FixedPoint::parse(s)
            .map(Some)
            .map_err(|e| CurateError::MalformedAction {
                reason: format!("invalid {what} {s:?}: {e}"),
            }),
        Some(value) => number_to_fixed(value).map(Some).map_err(|e| CurateError::MalformedAction {
            reason: format!("invalid {what} {value}: {e}"),
        }),
    }
}
