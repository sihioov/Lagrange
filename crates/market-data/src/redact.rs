//! Secret/redaction scan for collector logs (Todo 8 acceptance: logs pass a
//! secret/redaction scan; provider keys/data never appear in logs).
//!
//! A [`Redactor`] holds the known secret values of a run; [`Redactor::redact`]
//! scrubs any log line before it is emitted, and [`Redactor::scan`] detects
//! leftovers (secrets, `KEY=value` pairs for known credential keys, `Bearer`
//! tokens). [`Redactor::is_clean`] is the acceptance predicate.

/// Known credential key names whose values must never reach a log.
pub const SECRET_KEYS: [&str; 5] = [
    "KRX_CREDENTIAL_REF",
    "KRX_APP_SECRET",
    "KRX_BASE_URL",
    "X-Api-Key",
    "Authorization",
];

const REDACTED: &str = "[REDACTED]";

/// Scrubber for one run's log stream.
#[derive(Debug, Clone, Default)]
pub struct Redactor {
    secrets: Vec<String>,
}

impl Redactor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers secret values that must never appear in logs.
    pub fn add_secret(&mut self, secret: impl Into<String>) {
        let secret = secret.into();
        if secret.len() >= 3 {
            self.secrets.push(secret);
        }
    }

    pub fn with_secrets<I, S>(mut self, secrets: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for s in secrets {
            self.add_secret(s);
        }
        self
    }

    /// Scans `text` for secret material. Returns every hit (the secret value,
    /// or the offending `KEY=value`/`Bearer` span).
    pub fn scan(&self, text: &str) -> Vec<String> {
        let mut hits = Vec::new();
        for secret in &self.secrets {
            if text.contains(secret.as_str()) {
                hits.push(secret.clone());
            }
        }
        for key in SECRET_KEYS {
            if let Some(value) = value_after_key(text, key) {
                hits.push(format!("{key}={value}"));
            }
        }
        if let Some(token) = bearer_token(text) {
            hits.push(format!("Bearer {token}"));
        }
        hits
    }

    /// Whether `text` is free of secret material.
    pub fn is_clean(&self, text: &str) -> bool {
        self.scan(text).is_empty()
    }

    /// Returns `text` with every known secret scrubbed.
    pub fn redact(&self, text: &str) -> String {
        let mut out = text.to_owned();
        for secret in &self.secrets {
            out = out.replace(secret.as_str(), REDACTED);
        }
        for key in SECRET_KEYS {
            out = scrub_key_value(&out, key);
        }
        out = scrub_bearer(&out);
        out
    }
}

/// Finds the value bound to `key` via `key=` or `key:` (up to whitespace).
fn value_after_key<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    for (needle, start) in [(format!("{key}="), 1), (format!("{key}:"), 1)] {
        let mut from = 0;
        while let Some(rel) = text[from..].find(&needle) {
            let value_start = from + rel + start;
            let rest = &text[value_start..];
            let value_end = rest
                .find(|c: char| c.is_whitespace() || c == '"' || c == '\'')
                .unwrap_or(rest.len());
            let value = &rest[..value_end];
            if !value.is_empty() && value != REDACTED {
                return Some(value);
            }
            from = value_start + value_end;
        }
    }
    None
}

fn scrub_key_value(text: &str, key: &str) -> String {
    let mut out = text.to_owned();
    for (needle, start) in [(format!("{key}="), 1), (format!("{key}:"), 1)] {
        let mut from = 0;
        while let Some(rel) = out[from..].find(&needle) {
            let value_start = from + rel + start;
            let rest = &out[value_start..];
            let value_end = rest
                .find(|c: char| c.is_whitespace() || c == '"' || c == '\'')
                .unwrap_or(rest.len());
            let value = &rest[..value_end];
            if !value.is_empty() && value != REDACTED {
                out.replace_range(value_start..value_start + value_end, REDACTED);
            }
            from = value_start + REDACTED.len();
        }
    }
    out
}

fn bearer_token(text: &str) -> Option<&str> {
    let needle = "Bearer ";
    let mut from = 0;
    while let Some(rel) = text[from..].find(needle) {
        let token_start = from + rel + needle.len();
        let rest = &text[token_start..];
        let token_end = rest
            .find(|c: char| c.is_whitespace() || c == '"' || c == '\'')
            .unwrap_or(rest.len());
        let token = &rest[..token_end];
        if !token.is_empty() && token != REDACTED {
            return Some(token);
        }
        from = token_start + token_end;
    }
    None
}

fn scrub_bearer(text: &str) -> String {
    let mut out = text.to_owned();
    let needle = "Bearer ";
    let mut from = 0;
    while let Some(rel) = out[from..].find(needle) {
        let token_start = from + rel + needle.len();
        let rest = &out[token_start..];
        let token_end = rest
            .find(|c: char| c.is_whitespace() || c == '"' || c == '\'')
            .unwrap_or(rest.len());
        let token = &rest[..token_end];
        if !token.is_empty() && token != REDACTED {
            out.replace_range(token_start..token_start + token_end, REDACTED);
        }
        from = token_start + REDACTED.len();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_known_key_values_and_bearer_tokens() {
        let redactor = Redactor::new();
        let line = "KRX_CREDENTIAL_REF=sk-live-x KRX_APP_SECRET:secret2 auth=Bearer tok123";
        assert!(!redactor.is_clean(line));
        let out = redactor.redact(line);
        assert!(redactor.is_clean(&out), "still dirty: {out}");
        assert!(!out.contains("sk-live-x"));
        assert!(!out.contains("secret2"));
        assert!(!out.contains("tok123"));
    }

    #[test]
    fn redaction_is_idempotent() {
        let redactor = Redactor::new().with_secrets(["sk-live-x"]);
        let once = redactor.redact("KRX_CREDENTIAL_REF=sk-live-x");
        let twice = redactor.redact(&once);
        assert_eq!(once, twice);
    }
}
