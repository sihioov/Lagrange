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
        if secret.len() >= 3 && !self.secrets.contains(&secret) {
            self.secrets.push(secret);
            self.secrets
                .sort_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
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
    for needle in [format!("{key}="), format!("{key}:")] {
        let mut from = 0;
        while let Some(rel) = text[from..].find(&needle) {
            let value_start = from + rel + needle.len();
            let rest = &text[value_start..];
            let (content_start, content_end) = value_bounds(rest);
            let value = &rest[content_start..content_end];
            if !value.is_empty() && value != REDACTED {
                return Some(value);
            }
            from = value_start + content_end.max(1);
        }
    }
    None
}

fn scrub_key_value(text: &str, key: &str) -> String {
    let mut out = text.to_owned();
    for needle in [format!("{key}="), format!("{key}:")] {
        let mut from = 0;
        while let Some(rel) = out[from..].find(&needle) {
            let value_start = from + rel + needle.len();
            let rest = &out[value_start..];
            let (content_start, content_end) = value_bounds(rest);
            let value = &rest[content_start..content_end];
            if !value.is_empty() && value != REDACTED {
                out.replace_range(
                    value_start + content_start..value_start + content_end,
                    REDACTED,
                );
                from = value_start + content_start + REDACTED.len();
            } else {
                from = value_start + content_end.max(1);
            }
        }
    }
    out
}

fn value_bounds(value: &str) -> (usize, usize) {
    match value.as_bytes().first().copied() {
        Some(quote @ (b'\'' | b'"')) => {
            let start = 1;
            let end = value[start..]
                .bytes()
                .position(|byte| byte == quote)
                .map_or(value.len(), |position| start + position);
            (start, end)
        }
        _ => {
            let end = value
                .find(|character: char| {
                    character.is_whitespace() || character == '"' || character == '\''
                })
                .unwrap_or(value.len());
            (0, end)
        }
    }
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

    #[test]
    fn overlapping_secrets_are_redacted_longest_first_and_deduplicated() {
        let password = "review_password";
        let database_url =
            "postgres://review_user:review_password@review-host.example/review_database";
        let mut redactor = Redactor::new();
        redactor.add_secret(password);
        redactor.add_secret(database_url);
        redactor.add_secret(password);

        let input = format!("connect {database_url} with password {password}");
        assert!(!redactor.is_clean(&input));
        let redacted = redactor.redact(&input);

        for secret_fragment in [
            database_url,
            password,
            "review_user",
            "review-host.example",
            "review_database",
        ] {
            assert!(
                !redacted.contains(secret_fragment),
                "redacted output leaked {secret_fragment:?}: {redacted}"
            );
        }
        assert!(redactor.is_clean(&redacted));
        assert_eq!(redactor.redact(&redacted), redacted);
        assert_eq!(redactor.secrets, vec![database_url, password]);
    }

    #[test]
    fn key_value_redaction_starts_after_the_complete_delimiter() {
        let redactor = Redactor::new();
        let input =
            "KRX_APP_SECRET=alpha KRX_BASE_URL:'https://secret.example' KRX_APP_SECRET=beta";

        assert_eq!(
            redactor.scan(input),
            vec![
                "KRX_APP_SECRET=alpha",
                "KRX_BASE_URL=https://secret.example"
            ]
        );
        let output = redactor.redact(input);
        assert_eq!(
            output,
            "KRX_APP_SECRET=[REDACTED] KRX_BASE_URL:'[REDACTED]' KRX_APP_SECRET=[REDACTED]"
        );
        assert!(redactor.is_clean(&output));
        assert_eq!(redactor.redact(&output), output);
    }
}
