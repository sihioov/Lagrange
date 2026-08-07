//! Observability (design §15, NFR-§10.5): structured JSON logs with
//! correlation propagation and secret/account/PII redaction, plus the
//! documented Prometheus counters (no PII in labels — §15.2).

pub mod log;
pub mod metrics;
