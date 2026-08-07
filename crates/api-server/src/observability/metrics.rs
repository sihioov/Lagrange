//! Prometheus counters (design §15.2 core metrics) rendered in the text
//! format for the internal `/api/v1/metrics` scrape. Label values come from
//! FIXED sets (status classes, outcomes, severities) — never user ids,
//! emails, request ids, or artifact ids, so the endpoint carries no PII.

use prometheus::{Encoder, Histogram, HistogramOpts, IntCounterVec, Opts, Registry, TextEncoder};
use std::sync::OnceLock;

/// The documented counter base names (the drift guard for tests).
pub const METRIC_NAMES: &[&str] = &[
    "api_requests_total",
    "api_errors_total",
    "api_latency_seconds",
    "artifact_downloads_total",
    "job_retries_total",
    "alerts_raised_total",
    "notification_deliveries_total",
];

struct CounterHandles {
    requests: IntCounterVec,
    errors: IntCounterVec,
    latency: Histogram,
    artifacts: IntCounterVec,
    retries: IntCounterVec,
    alerts: IntCounterVec,
    deliveries: IntCounterVec,
}

fn counters() -> &'static CounterHandles {
    static C: OnceLock<CounterHandles> = OnceLock::new();
    C.get_or_init(|| {
        let registry = Registry::new();
        let requests = IntCounterVec::new(
            Opts::new("api_requests_total", "API requests by status class"),
            &["status"],
        )
        .expect("api_requests_total registers");
        let errors = IntCounterVec::new(
            Opts::new("api_errors_total", "API error responses by status class"),
            &["status"],
        )
        .expect("api_errors_total registers");
        let latency = Histogram::with_opts(HistogramOpts::new(
            "api_latency_seconds",
            "API request latency",
        ))
        .expect("api_latency_seconds registers");
        let artifacts = IntCounterVec::new(
            Opts::new(
                "artifact_downloads_total",
                "Artifact download outcomes (authorized/denied/integrity_failed)",
            ),
            &["outcome"],
        )
        .expect("artifact_downloads_total registers");
        let retries = IntCounterVec::new(
            Opts::new(
                "job_retries_total",
                "Admin job retry outcomes (requeued/denied)",
            ),
            &["outcome"],
        )
        .expect("job_retries_total registers");
        let alerts = IntCounterVec::new(
            Opts::new(
                "alerts_raised_total",
                "Alerts routed by severity (INFO/WARNING/CRITICAL)",
            ),
            &["severity"],
        )
        .expect("alerts_raised_total registers");
        let deliveries = IntCounterVec::new(
            Opts::new(
                "notification_deliveries_total",
                "Notification delivery outcomes (SUCCESS/FAILED)",
            ),
            &["status"],
        )
        .expect("notification_deliveries_total registers");
        for c in [
            Box::new(requests.clone()) as Box<dyn prometheus::core::Collector>,
            Box::new(errors.clone()),
            Box::new(latency.clone()),
            Box::new(artifacts.clone()),
            Box::new(retries.clone()),
            Box::new(alerts.clone()),
            Box::new(deliveries.clone()),
        ] {
            registry.register(c).expect("collector registers once");
        }
        // Seed every FIXED label value so the documented counters are always
        // present in the exposition (value 0 until first observation).
        for v in ["2", "4", "5"] {
            requests.with_label_values(&[v]).inc_by(0);
            errors.with_label_values(&[v]).inc_by(0);
        }
        for v in ["authorized", "denied", "integrity_failed"] {
            artifacts.with_label_values(&[v]).inc_by(0);
        }
        for v in ["requeued", "denied"] {
            retries.with_label_values(&[v]).inc_by(0);
        }
        for v in ["INFO", "WARNING", "CRITICAL"] {
            alerts.with_label_values(&[v]).inc_by(0);
        }
        for v in ["SUCCESS", "FAILED"] {
            deliveries.with_label_values(&[v]).inc_by(0);
        }
        std::mem::forget(registry);
        CounterHandles {
            requests,
            errors,
            latency,
            artifacts,
            retries,
            alerts,
            deliveries,
        }
    })
}

/// Record one API response by status class (`2`/`4`/`5` — no PII).
pub fn record_response(status_class: &str) {
    counters().requests.with_label_values(&[status_class]).inc();
}

/// Record one API error by status class.
pub fn record_error(status_class: &str) {
    counters().errors.with_label_values(&[status_class]).inc();
}

/// Record one request latency observation (seconds).
pub fn record_latency(seconds: f64) {
    counters().latency.observe(seconds);
}

/// Record one artifact download outcome (fixed set, no PII).
pub fn record_artifact_outcome(outcome: &str) {
    counters().artifacts.with_label_values(&[outcome]).inc();
}

/// Record one admin retry outcome (fixed set, no PII).
pub fn record_retry_outcome(outcome: &str) {
    counters().retries.with_label_values(&[outcome]).inc();
}

/// Record one routed alert (severity label from the fixed grade set).
pub fn record_alert(severity: &str) {
    counters().alerts.with_label_values(&[severity]).inc();
}

/// Record one delivery outcome (SUCCESS/FAILED).
pub fn record_delivery(status: &str) {
    counters().deliveries.with_label_values(&[status]).inc();
}

/// Render every counter in the Prometheus text exposition format.
pub fn render() -> String {
    let snapshot = Registry::new();
    for c in [
        Box::new(counters().requests.clone()) as Box<dyn prometheus::core::Collector>,
        Box::new(counters().errors.clone()),
        Box::new(counters().latency.clone()),
        Box::new(counters().artifacts.clone()),
        Box::new(counters().retries.clone()),
        Box::new(counters().alerts.clone()),
        Box::new(counters().deliveries.clone()),
    ] {
        let _ = snapshot.register(c);
    }
    let encoder = TextEncoder::new();
    let mut buf = Vec::new();
    encoder
        .encode(&snapshot.gather(), &mut buf)
        .expect("metrics encode");
    String::from_utf8(buf).expect("metrics are utf8")
}
