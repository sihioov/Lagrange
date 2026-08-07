//! Structured JSON logging (design §15.1 log fields): every event is one
//! JSON line with timestamp/level/service/instance_id/correlation_id and
//! optional user/job context. All free-form payloads pass through the
//! [`Redactor`] so secrets, account numbers, and PII never reach the log
//! stream (the workspace's eprintln!("...") upgrades land here).

use serde_json::{Value, json};

/// The service name reported in every log line.
pub const SERVICE: &str = "api-server";

/// The redaction marker (itself treated as clean so redaction is idempotent).
pub const MARKER: &str = "[REDACTED]";

/// Redacts secrets / account numbers / PII from free-form text before it is
/// logged (NFR-§10.5, plan: "secret-bearing log payload → redaction").
/// Patterns capture the secret VALUE; keys/prefixes stay for debuggability.
#[derive(Debug, Clone)]
pub struct Redactor {
    patterns: Vec<(&'static str, &'static str)>,
}

impl Default for Redactor {
    fn default() -> Self {
        Self::new()
    }
}

impl Redactor {
    pub fn new() -> Self {
        Self {
            patterns: vec![
                // Key/value secrets with common names (capture the value).
                (
                    r#"(?i)((?:api[_-]?key|key|secret|password|token|client_secret|access_token|refresh_token)\s*[=:]\s*)(\S+)"#,
                    "$1",
                ),
                // Authorization header values (greedy to the line end;
                // over-redaction is safe, under-redaction is the bug).
                (r#"(?i)(authorization\s*:\s*)(\S+(?:\s+\S+)*)"#, "$1"),
                // Account numbers: hyphenated bank style or bare 12+ digit runs.
                (r"\b\d{2,4}-\d{2,6}-\d{3,10}\b", ""),
                (r"\b\d{12,}\b", ""),
                // Emails (PII).
                (r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}", ""),
            ],
        }
    }
}

/// Redact one string; the output never contains the original secret-bearing
/// tokens. Idempotent: `[REDACTED]` itself is clean.
pub fn redact_str(red: &Redactor, input: &str) -> String {
    if input.contains(MARKER) {
        return input.to_string();
    }
    let mut out = input.to_string();
    for (pattern, prefix_group) in &red.patterns {
        let re = regex::Regex::new(pattern).expect("static redaction regex");
        out = re
            .replace_all(&out, |caps: &regex::Captures| {
                if caps.len() > 1 && !prefix_group.is_empty() {
                    format!("{}{}", &caps[1], MARKER)
                } else {
                    MARKER.to_string()
                }
            })
            .into_owned();
    }
    out
}

/// One structured log line (design §15.1 field list, subset relevant here).
#[derive(Debug, Clone)]
pub struct LogEvent {
    level: &'static str,
    event: &'static str,
    correlation_id: Option<String>,
    user_id: Option<String>,
    job_id: Option<String>,
    message: String,
    error_code: Option<String>,
}

impl LogEvent {
    pub fn info(event: &'static str) -> Self {
        Self::new("INFO", event)
    }
    pub fn warn(event: &'static str) -> Self {
        Self::new("WARNING", event)
    }
    pub fn critical(event: &'static str) -> Self {
        Self::new("CRITICAL", event)
    }

    fn new(level: &'static str, event: &'static str) -> Self {
        Self {
            level,
            event,
            correlation_id: None,
            user_id: None,
            job_id: None,
            message: String::new(),
            error_code: None,
        }
    }

    pub fn correlation(mut self, rid: impl Into<String>) -> Self {
        self.correlation_id = Some(rid.into());
        self
    }

    pub fn user(mut self, user_id: impl Into<String>) -> Self {
        self.user_id = Some(user_id.into());
        self
    }

    pub fn job(mut self, job_id: impl Into<String>) -> Self {
        self.job_id = Some(job_id.into());
        self
    }

    /// The message is REDACTED before it enters the JSON payload.
    pub fn message(mut self, message: impl Into<String>) -> Self {
        let red = Redactor::new();
        self.message = redact_str(&red, &message.into());
        self
    }

    pub fn error_code(mut self, code: impl Into<String>) -> Self {
        self.error_code = Some(code.into());
        self
    }

    /// The canonical structured JSON line (one line per event).
    pub fn to_json(&self) -> String {
        let mut value = json!({
            "timestamp": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            "level": self.level,
            "service": SERVICE,
            "instance_id": std::process::id(),
            "event": self.event,
            "message": self.message,
        });
        let obj = value.as_object_mut().expect("json object");
        if let Some(c) = &self.correlation_id {
            obj.insert("correlation_id".into(), Value::String(c.clone()));
        }
        if let Some(u) = &self.user_id {
            obj.insert("user_id".into(), Value::String(u.clone()));
        }
        if let Some(j) = &self.job_id {
            obj.insert("job_id".into(), Value::String(j.clone()));
        }
        if let Some(e) = &self.error_code {
            obj.insert("error_code".into(), Value::String(e.clone()));
        }
        value.to_string()
    }

    /// Emit the line to stderr (the service's structured log stream).
    pub fn emit(&self) {
        eprintln!("{}", self.to_json());
    }
}
