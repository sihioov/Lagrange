//! Structural schema validation of provider responses (Todo 8 QA: malformed
//! schema must fail typed with no partial curated output).
//!
//! The validation here is deliberately *structural* — the raw bytes are stored
//! unchanged regardless; a malformed response simply never reaches the store.
//! Full point-in-time quality checks are Todo 11.

use serde_json::Value;

use crate::contract::ResponseKind;

/// Why a response failed structural validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub kind: ResponseKind,
    pub reason: String,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "malformed {} response: {}", self.kind, self.reason)
    }
}

impl std::error::Error for ValidationError {}

/// Validates the recorded response shape for `kind`. Responses must be JSON
/// objects carrying the documented top-level array for their class.
pub fn validate_response(kind: ResponseKind, bytes: &[u8]) -> Result<(), ValidationError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|e| ValidationError {
        kind,
        reason: format!("not valid JSON: {e}"),
    })?;
    let obj = value.as_object().ok_or_else(|| ValidationError {
        kind,
        reason: "response must be a JSON object".to_owned(),
    })?;

    match kind {
        ResponseKind::Bars => {
            let bars = obj.get("bars").ok_or_else(|| missing(kind, "bars"))?;
            let arr = bars
                .as_array()
                .ok_or_else(|| invalid_type(kind, "bars", "array"))?;
            for (i, bar) in arr.iter().enumerate() {
                let b = bar.as_object().ok_or_else(|| ValidationError {
                    kind,
                    reason: format!("bars[{i}] is not an object"),
                })?;
                for field in ["instrument", "date"] {
                    if !b.get(field).is_some_and(|v| v.is_string()) {
                        return Err(ValidationError {
                            kind,
                            reason: format!("bars[{i}].{field} must be a string"),
                        });
                    }
                }
                for field in ["open", "high", "low", "close", "volume"] {
                    if !b.get(field).is_some_and(|v| v.is_number()) {
                        return Err(ValidationError {
                            kind,
                            reason: format!("bars[{i}].{field} must be a number"),
                        });
                    }
                }
            }
        }
        ResponseKind::Reference => {
            let instruments = obj
                .get("instruments")
                .ok_or_else(|| missing(kind, "instruments"))?;
            let arr = instruments
                .as_array()
                .ok_or_else(|| invalid_type(kind, "instruments", "array"))?;
            for (i, inst) in arr.iter().enumerate() {
                let i_obj = inst.as_object().ok_or_else(|| ValidationError {
                    kind,
                    reason: format!("instruments[{i}] is not an object"),
                })?;
                if !i_obj.get("symbol").is_some_and(|v| v.is_string()) {
                    return Err(ValidationError {
                        kind,
                        reason: format!("instruments[{i}].symbol must be a string"),
                    });
                }
            }
        }
        ResponseKind::Calendar => {
            let sessions = obj
                .get("sessions")
                .ok_or_else(|| missing(kind, "sessions"))?;
            let arr = sessions
                .as_array()
                .ok_or_else(|| invalid_type(kind, "sessions", "array"))?;
            for (i, s) in arr.iter().enumerate() {
                let s_obj = s.as_object().ok_or_else(|| ValidationError {
                    kind,
                    reason: format!("sessions[{i}] is not an object"),
                })?;
                for field in ["date", "open_utc", "close_utc"] {
                    if !s_obj.get(field).is_some_and(|v| v.is_string()) {
                        return Err(ValidationError {
                            kind,
                            reason: format!("sessions[{i}].{field} must be a string"),
                        });
                    }
                }
            }
        }
        ResponseKind::CorporateActions => {
            // The actions array may legitimately be empty (canonical dataset).
            let actions = obj.get("actions").ok_or_else(|| missing(kind, "actions"))?;
            if !actions.is_array() {
                return Err(invalid_type(kind, "actions", "array"));
            }
        }
    }
    Ok(())
}

fn missing(kind: ResponseKind, field: &str) -> ValidationError {
    ValidationError {
        kind,
        reason: format!("missing required field {field:?}"),
    }
}

fn invalid_type(kind: ResponseKind, field: &str, expected: &str) -> ValidationError {
    ValidationError {
        kind,
        reason: format!("field {field:?} must be a JSON {expected}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_fixture_shapes_pass() {
        let bars = br#"{"bars":[{"instrument":"069500.KRX","date":"2020-01-31","open":1,"high":2,"low":1,"close":2,"volume":10}]}"#;
        assert!(validate_response(ResponseKind::Bars, bars).is_ok());
        let refr = br#"{"instruments":[{"symbol":"069500.KRX"}]}"#;
        assert!(validate_response(ResponseKind::Reference, refr).is_ok());
        let cal = br#"{"sessions":[{"date":"2020-01-31","open_utc":"2020-01-31T00:00:00Z","close_utc":"2020-01-31T06:30:00Z"}]}"#;
        assert!(validate_response(ResponseKind::Calendar, cal).is_ok());
        let actions = br#"{"actions":[]}"#;
        assert!(validate_response(ResponseKind::CorporateActions, actions).is_ok());
    }

    #[test]
    fn malformed_shapes_fail_typed() {
        let cases: &[(&[u8], ResponseKind)] = &[
            (br#"{"bars":{"not":"an array"}}"#, ResponseKind::Bars),
            (
                br#"{"bars":[{"instrument":"X","date":"d","open":"nope"}]}"#,
                ResponseKind::Bars,
            ),
            (
                br#"{"instruments":[{"symbol":123}]}"#,
                ResponseKind::Reference,
            ),
            (br#"{"sessions":"nope"}"#, ResponseKind::Calendar),
            (br#"{"actions":{}}"#, ResponseKind::CorporateActions),
            (b"not json", ResponseKind::Bars),
        ];
        for (bytes, kind) in cases {
            let err = validate_response(*kind, bytes).expect_err("must fail");
            assert_eq!(err.kind, *kind);
            assert!(!err.reason.is_empty());
        }
    }
}
