//! Todo 27 observability: structured JSON logs with correlation-id
//! propagation and secret/account-number/PII redaction (NFR-MAINT/§10.5),
//! and Prometheus counters with NO PII in labels (design §15.2). All tests
//! carry the `observability_` prefix for the plan acceptance filter
//! `cargo test -p api-server artifact admin observability`.

mod common;

use api_server::observability::log::{LogEvent, Redactor, redact_str};
use api_server::observability::metrics::METRIC_NAMES;
use axum::http::StatusCode;
use common::{Harness, status};
use serde_json::json;

// ---------------------------------------------------------------------------
// Log redaction
// ---------------------------------------------------------------------------

#[test]
fn observability_log_redaction_secrets_accounts_and_pii() {
    let red = Redactor::default();
    // API keys / secrets / bearer tokens.
    assert_eq!(
        redact_str(&red, "client key=sk_live_abcdef123456"),
        "client key=[REDACTED]"
    );
    assert_eq!(
        redact_str(&red, "Authorization: Bearer eyJhbGciOiJSUzI1NiJ9.abc.def"),
        "Authorization: [REDACTED]"
    );
    assert_eq!(
        redact_str(&red, "password=correct-horse"),
        "password=[REDACTED]"
    );
    assert_eq!(
        redact_str(&red, "api_key=0123456789abcdef"),
        "api_key=[REDACTED]"
    );
    // Account numbers (KIS: bank-style digits).
    assert_eq!(
        redact_str(&red, "account 1002-123456-789 transfer"),
        "account [REDACTED] transfer"
    );
    assert_eq!(
        redact_str(&red, "account=123456789012345"),
        "account=[REDACTED]"
    );
    // PII: emails.
    assert_eq!(
        redact_str(&red, "user owner@lagrange.test failed"),
        "user [REDACTED] failed"
    );
    // Redaction is idempotent: [REDACTED] itself must stay clean.
    let once = redact_str(&red, "key=sk_live_abc token=1234567890123456");
    assert_eq!(redact_str(&red, &once), once);
}

#[test]
fn observability_log_event_carries_correlation_and_redacts_payload() {
    let event = LogEvent::info("http.request")
        .correlation("rid-42")
        .user("u-1")
        .message("GET /api/v1/backtests failed with token=abc123")
        .error_code("INTERNAL");
    let json = event.to_json();
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("structured JSON log");
    assert_eq!(parsed["level"], "INFO");
    assert_eq!(parsed["service"], "api-server");
    assert_eq!(parsed["correlation_id"], "rid-42");
    assert_eq!(parsed["event"], "http.request");
    assert_eq!(parsed["error_code"], "INTERNAL");
    let msg = parsed["message"].as_str().expect("message");
    assert!(
        !msg.contains("abc123"),
        "secrets must be redacted from the message: {msg}"
    );
    assert!(msg.contains("[REDACTED]"));
    assert!(parsed["timestamp"].is_string(), "RFC3339 timestamp");
}

#[test]
fn observability_log_event_warning_and_critical_levels() {
    let w = LogEvent::warn("job.degraded").message("missing bars");
    assert_eq!(serde_json::from_str::<serde_json::Value>(&w.to_json()).unwrap()["level"], "WARNING");
    let c = LogEvent::critical("db.down").message("pool exhausted");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&c.to_json()).unwrap()["level"],
        "CRITICAL"
    );
}

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

#[tokio::test]
async fn observability_metrics_exposed_without_pii() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    // No session required (Prometheus scrape over the internal network).
    let resp = h.get("/api/v1/metrics", None).await;
    assert_eq!(status(&resp), StatusCode::OK);
    let text = Harness::body_text(resp).await;
    for name in METRIC_NAMES {
        assert!(
            text.contains(name),
            "documented counter {name} must be exposed"
        );
    }
    // No PII: no emails, no request ids, no uuid-shaped label values.
    assert!(!text.contains('@'), "metrics must not contain emails: {text}");
    assert!(
        !text.contains("test-rid"),
        "metrics must not contain request ids"
    );
    for line in text.lines() {
        for quoted in line.split('"').skip(1).step_by(2) {
            assert_ne!(
                quoted.len(),
                36,
                "label values must not be uuids (line: {line})"
            );
            assert!(
                !quoted.starts_with("00000000-"),
                "label values must not be uuids (line: {line})"
            );
        }
    }
    // A request increments the API counter.
    let before = count_metric(&text, "api_requests_total");
    let _ = h.get("/api/v1/auth/session", Some(&h.member)).await;
    let after = count_metric(&Harness::body_text(h.get("/api/v1/metrics", None).await).await, "api_requests_total");
    assert!(
        after > before,
        "api_requests_total must increment per request ({before} -> {after})"
    );
    h.teardown().await;
}

#[tokio::test]
async fn observability_metrics_artifact_and_alert_counters() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    // A denied artifact download increments the denied outcome (no PII label).
    let resp = h
        .get("/api/v1/artifacts/00000000-0000-0000-0000-000000000000/download", Some(&h.member))
        .await;
    assert_eq!(status(&resp), StatusCode::NOT_FOUND);
    let text = Harness::body_text(h.get("/api/v1/metrics", None).await).await;
    assert!(
        text.contains("artifact_downloads_total"),
        "artifact counter must be documented"
    );
    // An alert routes and increments alerts + delivery counters.
    let key = format!("obs-{}", uuid::Uuid::new_v4());
    let resp = h
        .send(
            "POST",
            "/api/v1/notifications/test",
            Some(&h.member),
            true,
            Some("test-rid-1"),
            Some(&key),
            Some(json!({ "severity": "WARNING", "kind": "job", "title": "obs alert" })),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let text = Harness::body_text(h.get("/api/v1/metrics", None).await).await;
    assert!(
        text.contains("alerts_raised_total{severity=\"WARNING\"}"),
        "alert routing must expose the severity counter"
    );
    assert!(
        text.contains("notification_deliveries_total{status=\"SUCCESS\"}"),
        "delivery outcomes must be counted"
    );
    h.teardown().await;
}

fn count_metric(text: &str, name: &str) -> u64 {
    text.lines()
        .filter(|l| l.starts_with(&format!("{name}{{")) || l.starts_with(&format!("{name} ")))
        .map(|l| {
            l.rsplit_once(' ')
                .and_then(|(_, v)| v.parse::<u64>().ok())
                .unwrap_or(0)
        })
        .sum()
}
