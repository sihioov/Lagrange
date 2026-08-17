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
        ResponseKind::InvestorFlow => validate_rows(
            kind,
            obj,
            "flows",
            &[
                "instrument",
                "trade_date",
                "investor_class",
                "source_revision",
                "available_at",
            ],
            &["net_amount", "net_volume"],
        )?,
        ResponseKind::MarketStatus => {
            validate_rows(
                kind,
                obj,
                "statuses",
                &[
                    "instrument",
                    "trade_date",
                    "source_revision",
                    "available_at",
                ],
                &[],
            )?;
            validate_boolean_fields(
                kind,
                obj,
                "statuses",
                &[
                    "suspended",
                    "administrative",
                    "liquidation",
                    "inactive",
                    "disqualifying_audit_opinion",
                    "complete_capital_impairment",
                ],
            )?;
        }
        ResponseKind::Fundamentals => validate_rows(
            kind,
            obj,
            "fundamentals",
            &[
                "instrument",
                "fiscal_period_start",
                "fiscal_period_end",
                "period_kind",
                "statement_scope",
                "metric",
                "disclosed_at",
                "available_at",
                "source_revision",
            ],
            &["value", "unit_scale"],
        )?,
        ResponseKind::IndexMembership => validate_rows(
            kind,
            obj,
            "memberships",
            &[
                "index_id",
                "instrument",
                "announced_at",
                "effective_from",
                "available_at",
                "source_revision",
                "source_revision",
            ],
            &[],
        )?,
        ResponseKind::SectorClassification => validate_rows(
            kind,
            obj,
            "sectors",
            &[
                "taxonomy_id",
                "taxonomy_version",
                "instrument",
                "sector_code",
                "sector_name",
                "fundamental_profile",
                "effective_from",
                "available_at",
            ],
            &[],
        )?,
        ResponseKind::CandidateMaster => {
            return Err(ValidationError {
                kind,
                reason: "candidate master is a ZIP source; use the strict candidate-master parser"
                    .to_owned(),
            });
        }
    }
    Ok(())
}

fn validate_rows(
    kind: ResponseKind,
    obj: &serde_json::Map<String, Value>,
    collection: &str,
    string_fields: &[&str],
    number_fields: &[&str],
) -> Result<(), ValidationError> {
    let rows = obj
        .get(collection)
        .ok_or_else(|| missing(kind, collection))?
        .as_array()
        .ok_or_else(|| invalid_type(kind, collection, "array"))?;
    for (index, row) in rows.iter().enumerate() {
        let row = row.as_object().ok_or_else(|| ValidationError {
            kind,
            reason: format!("{collection}[{index}] is not an object"),
        })?;
        for field in string_fields {
            if !row.get(*field).is_some_and(Value::is_string) {
                return Err(ValidationError {
                    kind,
                    reason: format!("{collection}[{index}].{field} must be a string"),
                });
            }
        }
        for field in number_fields {
            if !row.get(*field).is_some_and(Value::is_number) {
                return Err(ValidationError {
                    kind,
                    reason: format!("{collection}[{index}].{field} must be a number"),
                });
            }
        }
    }
    Ok(())
}

fn validate_boolean_fields(
    kind: ResponseKind,
    obj: &serde_json::Map<String, Value>,
    collection: &str,
    fields: &[&str],
) -> Result<(), ValidationError> {
    let rows = obj
        .get(collection)
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_type(kind, collection, "array"))?;
    for (index, row) in rows.iter().enumerate() {
        let row = row.as_object().ok_or_else(|| ValidationError {
            kind,
            reason: format!("{collection}[{index}] is not an object"),
        })?;
        for field in fields {
            if !row.get(*field).is_some_and(Value::is_boolean) {
                return Err(ValidationError {
                    kind,
                    reason: format!("{collection}[{index}].{field} must be a boolean"),
                });
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
        let flow = br#"{"flows":[{"instrument":"005930.KRX","trade_date":"2026-08-14","investor_class":"FOREIGN","net_amount":1,"net_volume":2,"source_revision":"1","available_at":"2026-08-14T07:00:00Z"}]}"#;
        assert!(validate_response(ResponseKind::InvestorFlow, flow).is_ok());
        let status = br#"{"statuses":[{"instrument":"005930.KRX","trade_date":"2026-08-14","suspended":false,"administrative":false,"liquidation":false,"inactive":false,"disqualifying_audit_opinion":false,"complete_capital_impairment":false,"source_revision":"1","available_at":"2026-08-14T07:00:00Z"}]}"#;
        assert!(validate_response(ResponseKind::MarketStatus, status).is_ok());
        let fundamentals = br#"{"fundamentals":[{"instrument":"005930.KRX","fiscal_period_start":"2026-01-01","fiscal_period_end":"2026-03-31","period_kind":"QUARTER","statement_scope":"CONSOLIDATED","metric":"revenue","value":1,"unit_scale":1,"disclosed_at":"2026-05-01T00:00:00Z","available_at":"2026-05-01T00:01:00Z","source_revision":"1"}]}"#;
        assert!(validate_response(ResponseKind::Fundamentals, fundamentals).is_ok());
        let membership = br#"{"memberships":[{"index_id":"kospi200","instrument":"005930.KRX","announced_at":"2026-06-01T00:00:00Z","effective_from":"2026-06-12","available_at":"2026-06-01T00:01:00Z","source_revision":"1"}]}"#;
        assert!(validate_response(ResponseKind::IndexMembership, membership).is_ok());
        let sectors = br#"{"sectors":[{"taxonomy_id":"krx","taxonomy_version":"2026","instrument":"005930.KRX","sector_code":"IT","sector_name":"Information Technology","fundamental_profile":"NON_FINANCIAL","effective_from":"2026-01-01","available_at":"2026-01-01T00:00:00Z","source_revision":"1"}]}"#;
        assert!(validate_response(ResponseKind::SectorClassification, sectors).is_ok());
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
            (
                br#"{"flows":[{"instrument":1}]}"#,
                ResponseKind::InvestorFlow,
            ),
            (br#"{"statuses":{}}"#, ResponseKind::MarketStatus),
            (br#"{"fundamentals":{}}"#, ResponseKind::Fundamentals),
            (br#"{"memberships":[1]}"#, ResponseKind::IndexMembership),
            (br#"{"sectors":"nope"}"#, ResponseKind::SectorClassification),
            (b"not json", ResponseKind::Bars),
        ];
        for (bytes, kind) in cases {
            let err = validate_response(*kind, bytes).expect_err("must fail");
            assert_eq!(err.kind, *kind);
            assert!(!err.reason.is_empty());
        }
    }
}
