//! Typed, redacted broker errors and the retry classification (plan Todo 36).
//!
//! Two rules from design §6.12 are encoded here rather than left to callers:
//!
//! 1. **"주문 제출 타임아웃을 주문 실패로 단정하지 않는다."** A mutation that
//!    times out did not fail — the broker may well have accepted it. It
//!    produces [`KisError::Ambiguous`], which is neither success nor failure
//!    and is resolvable ONLY by querying order state. There is no code path
//!    that turns it into a failure, and none that retries it.
//!
//! 2. **Never blindly retry a mutation.** Retryability is not a property of the
//!    error alone; it is a property of the error AND what was attempted. The
//!    same 500 that is safe to retry on a balance query is not safe to retry on
//!    an order submission. [`RequestKind`] therefore participates in the
//!    decision, so "is this retryable?" cannot be asked without saying what
//!    "this" was.
//!
//! Every variant is redacted by construction: broker payloads reach an error
//! only through [`redact_payload`], and credentials cannot reach one at all
//! because [`crate::secret::Secret`] has no non-redacting rendering.

use crate::secret::CredentialError;

/// What the caller was attempting. Retry decisions require it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    /// Safe to repeat: quotes, balances, order lookups. Repeating a read
    /// cannot create state.
    Read,
    /// Creates or changes broker state: submit, amend, cancel. Repeating one
    /// can duplicate an order, so it is retried ONLY when the transport can
    /// prove the request never reached the broker.
    Mutation,
}

/// A broker interaction that did not produce a typed success.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KisError {
    /// The request never left this process, so the broker cannot have seen it.
    /// This is the one transport failure a mutation may safely retry.
    #[error("could not connect to the broker: {reason}")]
    Connect { reason: String },

    /// A mutation whose outcome is genuinely unknown: the request was sent and
    /// no usable response came back.
    ///
    /// NOT a failure. The order may exist at the broker. Callers must resolve
    /// it by querying order state (design §16 `UNKNOWN` 상태, 주문 조회로 해소
    /// 전 재제출 금지) and must never resubmit on the strength of this error.
    #[error(
        "order state is UNKNOWN after {operation}: the broker may have accepted it; \
         resolve by querying order state before any resubmission"
    )]
    Ambiguous {
        operation: String,
        /// Correlates this attempt with the audit record and with the eventual
        /// resolving query.
        client_order_id: String,
    },

    /// The broker throttled us.
    #[error("broker rate limit hit on {endpoint} (retry after {retry_after_ms}ms)")]
    RateLimited {
        endpoint: String,
        retry_after_ms: u64,
    },

    /// The broker answered with an error status. `body` is already redacted.
    #[error("broker returned {status} for {endpoint}: {body}")]
    Broker {
        status: u16,
        endpoint: String,
        body: String,
    },

    /// The response did not match the schema we compiled against — a schema
    /// drift, not a transport problem. Never retried: repeating a request the
    /// code cannot parse just produces the same unparseable answer.
    #[error("broker response for {endpoint} did not match the expected schema: {detail}")]
    SchemaDrift { endpoint: String, detail: String },

    /// Authentication could not be established.
    #[error("broker authentication failed: {reason}")]
    Auth { reason: String },

    /// A credential could not be resolved. Carries the location, never a value.
    #[error(transparent)]
    Credential(#[from] CredentialError),

    /// The local clock is too far from the broker's for signed requests to be
    /// accepted. Retrying cannot fix a wrong clock.
    #[error("local clock differs from the broker by {skew_secs}s (limit {limit_secs}s)")]
    ClockSkew { skew_secs: i64, limit_secs: i64 },

    /// An instrument could not be mapped in either direction.
    #[error("no KIS mapping for instrument {instrument}")]
    UnknownInstrument { instrument: String },
}

impl KisError {
    /// Whether this error may be retried for `kind`.
    ///
    /// The asymmetry is the point. A read may retry anything transient. A
    /// mutation may retry ONLY [`KisError::Connect`], where the request
    /// provably never left the process — everything else leaves open the
    /// possibility that the broker already acted on it.
    pub fn is_retryable(&self, kind: RequestKind) -> bool {
        match kind {
            RequestKind::Read => matches!(
                self,
                Self::Connect { .. }
                    | Self::RateLimited { .. }
                    | Self::Broker {
                        status: 429 | 500..=599,
                        ..
                    }
            ),
            // A mutation retries only when the broker cannot have seen it.
            RequestKind::Mutation => matches!(self, Self::Connect { .. }),
        }
    }

    /// Whether the outcome is unknown rather than failed. Callers must branch
    /// on this before treating an error as "the order did not happen".
    pub fn is_ambiguous(&self) -> bool {
        matches!(self, Self::Ambiguous { .. })
    }

    /// The stable code recorded in audit and surfaced upstream. Matches the
    /// API error-code table so a Live failure reads the same at every layer.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Ambiguous { .. } => "ORDER_STATE_UNKNOWN",
            Self::RateLimited { .. } => "BROKER_RATE_LIMITED",
            Self::Broker { .. } => "BROKER_REJECTED",
            Self::SchemaDrift { .. } => "BROKER_SCHEMA_DRIFT",
            Self::Auth { .. } | Self::Credential(_) => "BROKER_AUTH_FAILED",
            Self::ClockSkew { .. } => "BROKER_CLOCK_SKEW",
            Self::UnknownInstrument { .. } => "UNKNOWN_INSTRUMENT",
            Self::Connect { .. } => "BROKER_UNREACHABLE",
        }
    }
}

/// Keys whose values are stripped from any payload before it can be stored or
/// logged. Matched case-insensitively against JSON keys and header names.
const SENSITIVE_KEYS: &[&str] = &[
    "appkey",
    "appsecret",
    "access_token",
    "refresh_token",
    "authorization",
    "token",
    "cano",         // KIS: account number
    "acnt_prdt_cd", // KIS: account product code
    "hts_id",
    "personalseckey",
];

/// Redact a broker payload so it is safe to store in an audit record.
///
/// Design §6.12: "응답 원문에서 민감정보를 제거한 감사 기록을 보관한다" — keep
/// the audit record, remove the sensitive content. This is a textual pass
/// deliberately: it must work on a body that failed to parse, which is exactly
/// the case (schema drift, broker error page) where a raw payload would
/// otherwise be logged verbatim.
///
/// Truncation is applied last so an enormous error page cannot fill the audit
/// log; the marker makes the truncation visible rather than silent.
pub fn redact_payload(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len().min(MAX_PAYLOAD));
    let mut rest = raw;

    while let Some(colon) = rest.find(':') {
        let (head, tail) = rest.split_at(colon);
        out.push_str(head);
        out.push(':');
        let after = &tail[1..];

        // Is the key immediately before this colon sensitive?
        let key = head
            .rsplit(['{', ',', '\n'])
            .next()
            .unwrap_or("")
            .trim()
            .trim_matches('"')
            .to_ascii_lowercase();

        if SENSITIVE_KEYS.contains(&key.as_str()) {
            // Consume this value and replace it.
            let end = after.find([',', '}', '\n']).unwrap_or(after.len());
            out.push_str("\"<redacted>\"");
            rest = &after[end..];
        } else {
            rest = after;
        }
    }
    out.push_str(rest);

    if out.len() > MAX_PAYLOAD {
        out.truncate(MAX_PAYLOAD);
        out.push_str("...<truncated>");
    }
    out
}

const MAX_PAYLOAD: usize = 2048;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_submit_timeout_is_ambiguous_and_is_never_a_failure() {
        let e = KisError::Ambiguous {
            operation: "order.submit".to_string(),
            client_order_id: "coid-1".to_string(),
        };
        assert!(e.is_ambiguous());
        assert_eq!(e.code(), "ORDER_STATE_UNKNOWN");
        // The message must tell the operator what to do, not just what broke.
        let msg = e.to_string();
        assert!(msg.contains("UNKNOWN"));
        assert!(msg.contains("resolve by querying order state"));
    }

    #[test]
    fn an_ambiguous_mutation_is_never_retried() {
        let e = KisError::Ambiguous {
            operation: "order.submit".to_string(),
            client_order_id: "coid-1".to_string(),
        };
        assert!(
            !e.is_retryable(RequestKind::Mutation),
            "retrying an ambiguous submit is how one order becomes two"
        );
    }

    #[test]
    fn a_mutation_retries_only_when_the_broker_cannot_have_seen_it() {
        let connect = KisError::Connect {
            reason: "connection refused".to_string(),
        };
        assert!(
            connect.is_retryable(RequestKind::Mutation),
            "a request that never left the process is safe to repeat"
        );

        // Everything else leaves open that the broker already acted.
        for e in [
            KisError::RateLimited {
                endpoint: "/order".to_string(),
                retry_after_ms: 200,
            },
            KisError::Broker {
                status: 500,
                endpoint: "/order".to_string(),
                body: "{}".to_string(),
            },
            KisError::Broker {
                status: 429,
                endpoint: "/order".to_string(),
                body: "{}".to_string(),
            },
        ] {
            assert!(
                !e.is_retryable(RequestKind::Mutation),
                "a mutation must not retry {e:?}"
            );
        }
    }

    #[test]
    fn a_read_retries_transient_status_codes() {
        let throttled = KisError::Broker {
            status: 429,
            endpoint: "/quote".to_string(),
            body: "{}".to_string(),
        };
        let server = KisError::Broker {
            status: 500,
            endpoint: "/quote".to_string(),
            body: "{}".to_string(),
        };
        assert!(throttled.is_retryable(RequestKind::Read));
        assert!(server.is_retryable(RequestKind::Read));

        // A client error is the caller's fault; repeating it changes nothing.
        let bad = KisError::Broker {
            status: 400,
            endpoint: "/quote".to_string(),
            body: "{}".to_string(),
        };
        assert!(!bad.is_retryable(RequestKind::Read));
    }

    #[test]
    fn schema_drift_and_clock_skew_are_never_retried() {
        let drift = KisError::SchemaDrift {
            endpoint: "/balance".to_string(),
            detail: "missing field output".to_string(),
        };
        let skew = KisError::ClockSkew {
            skew_secs: 90,
            limit_secs: 30,
        };
        for e in [drift, skew] {
            assert!(!e.is_retryable(RequestKind::Read));
            assert!(!e.is_retryable(RequestKind::Mutation));
        }
    }

    #[test]
    fn payload_redaction_strips_credentials_and_account_numbers() {
        let raw = r#"{"appkey":"PSabc123","appsecret":"sec-xyz","access_token":"eyJhbGci","CANO":"50123456","output":{"odno":"0000117057"}}"#;
        let red = redact_payload(raw);
        for leaked in ["PSabc123", "sec-xyz", "eyJhbGci", "50123456"] {
            assert!(!red.contains(leaked), "payload leaked {leaked}: {red}");
        }
        // Non-sensitive fields survive, or the audit record would be useless.
        assert!(red.contains("0000117057"), "{red}");
        assert!(red.contains("output"), "{red}");
    }

    #[test]
    fn redaction_works_on_a_body_that_is_not_json() {
        // The case that matters most: a broker error page or a drifted schema,
        // where a parse-first redactor would fall back to logging the raw text.
        let raw = "error: authorization: Bearer eyJhbGciOiJIUzI1NiJ9 rejected";
        let red = redact_payload(raw);
        assert!(
            !red.contains("eyJhbGciOiJIUzI1NiJ9"),
            "non-JSON payload leaked a token: {red}"
        );
    }

    #[test]
    fn redaction_truncates_an_enormous_payload_visibly() {
        let raw = "x".repeat(MAX_PAYLOAD * 2);
        let red = redact_payload(&raw);
        assert!(red.len() <= MAX_PAYLOAD + 16);
        assert!(
            red.ends_with("...<truncated>"),
            "truncation must be visible, not silent"
        );
    }

    #[test]
    fn every_variant_has_a_stable_audit_code() {
        let all = [
            KisError::Connect { reason: "x".into() },
            KisError::Ambiguous {
                operation: "o".into(),
                client_order_id: "c".into(),
            },
            KisError::RateLimited {
                endpoint: "e".into(),
                retry_after_ms: 1,
            },
            KisError::Broker {
                status: 400,
                endpoint: "e".into(),
                body: "b".into(),
            },
            KisError::SchemaDrift {
                endpoint: "e".into(),
                detail: "d".into(),
            },
            KisError::Auth { reason: "r".into() },
            KisError::ClockSkew {
                skew_secs: 1,
                limit_secs: 0,
            },
            KisError::UnknownInstrument {
                instrument: "i".into(),
            },
        ];
        for e in all {
            assert!(!e.code().is_empty(), "{e:?} has no audit code");
        }
    }
}
