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
        while let Some(found) = find_ascii_case_insensitive(text, &needle, from) {
            let value_start = found + needle.len();
            let rest = &text[value_start..];
            let (content_start, content_end) = value_bounds_for_key(rest, key);
            let value = &rest[content_start..content_end];
            if !value.is_empty() && value != REDACTED {
                return Some(value);
            }
            from = next_search_start(value_start, rest, content_end);
        }
    }
    None
}

fn scrub_key_value(text: &str, key: &str) -> String {
    let mut out = text.to_owned();
    for needle in [format!("{key}="), format!("{key}:")] {
        let mut from = 0;
        while let Some(found) = find_ascii_case_insensitive(&out, &needle, from) {
            let value_start = found + needle.len();
            let rest = &out[value_start..];
            let (content_start, content_end) = value_bounds_for_key(rest, key);
            let value = &rest[content_start..content_end];
            if !value.is_empty() && value != REDACTED {
                out.replace_range(
                    value_start + content_start..value_start + content_end,
                    REDACTED,
                );
                from = value_start + content_start + REDACTED.len();
            } else {
                from = next_search_start(value_start, rest, content_end);
            }
        }
    }
    out
}

fn find_ascii_case_insensitive(text: &str, needle: &str, from: usize) -> Option<usize> {
    text.as_bytes()[from..]
        .windows(needle.len())
        .position(|candidate| candidate.eq_ignore_ascii_case(needle.as_bytes()))
        .map(|position| from + position)
}

fn value_bounds_for_key(value: &str, key: &str) -> (usize, usize) {
    if key.eq_ignore_ascii_case("Authorization") {
        authorization_value_bounds(value)
    } else {
        value_bounds(value)
    }
}

fn authorization_value_bounds(value: &str) -> (usize, usize) {
    let leading = value
        .find(|character: char| matches!(character, '\r' | '\n') || !character.is_whitespace())
        .unwrap_or(value.len());
    let credential = &value[leading..];
    if matches!(credential.as_bytes().first(), Some(b'\'' | b'"')) {
        let (start, end) = value_bounds(credential);
        return (leading + start, leading + end);
    }

    let line_end = credential.find(['\r', '\n']).unwrap_or(credential.len());
    (leading, leading + line_end)
}

fn value_bounds(value: &str) -> (usize, usize) {
    match value.as_bytes().first().copied() {
        Some(quote @ (b'\'' | b'"')) => {
            let start = 1;
            let mut escaped = false;
            let mut end = value.len();
            for (position, byte) in value.as_bytes()[start..].iter().copied().enumerate() {
                if byte == b'\\' {
                    escaped = !escaped;
                    continue;
                }
                if byte == quote && !escaped {
                    end = start + position;
                    break;
                }
                escaped = false;
            }
            (start, end)
        }
        _ => (0, unquoted_value_end(value)),
    }
}

fn unquoted_value_end(value: &str) -> usize {
    value
        .find(|character: char| character.is_whitespace() || character == '"' || character == '\'')
        .unwrap_or(value.len())
}

fn next_search_start(value_start: usize, value: &str, content_end: usize) -> usize {
    if content_end > 0 {
        value_start + content_end
    } else {
        value_start + value.chars().next().map_or(0, char::len_utf8)
    }
}

fn bearer_token(text: &str) -> Option<&str> {
    let needle = "Bearer";
    let mut from = 0;
    while let Some(found) = find_ascii_case_insensitive(text, needle, from) {
        let separator_start = found + needle.len();
        let separator = &text[separator_start..];
        let separator_len =
            separator.len() - separator.trim_start_matches(char::is_whitespace).len();
        if separator_len == 0 {
            from = separator_start;
            continue;
        }
        let token_start = separator_start + separator_len;
        let rest = &text[token_start..];
        let token_end = unquoted_value_end(rest);
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
    let needle = "Bearer";
    let mut from = 0;
    while let Some(found) = find_ascii_case_insensitive(&out, needle, from) {
        let separator_start = found + needle.len();
        let separator = &out[separator_start..];
        let separator_len =
            separator.len() - separator.trim_start_matches(char::is_whitespace).len();
        if separator_len == 0 {
            from = separator_start;
            continue;
        }
        let token_start = separator_start + separator_len;
        let rest = &out[token_start..];
        let token_end = unquoted_value_end(rest);
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

    #[test]
    fn empty_terminal_and_repeated_empty_values_are_stable() {
        let redactor = Redactor::new();
        for input in [
            "KRX_APP_SECRET=",
            "KRX_APP_SECRET= KRX_APP_SECRET:",
            "앞 KRX_APP_SECRET= 뒤 KRX_APP_SECRET:",
        ] {
            assert!(redactor.scan(input).is_empty(), "input: {input:?}");
            assert_eq!(redactor.redact(input), input);
            assert!(redactor.is_clean(input));
        }
    }

    #[test]
    fn multibyte_whitespace_between_empty_and_nonempty_values_is_utf8_safe() {
        let redactor = Redactor::new();
        let input = "KRX_APP_SECRET=\u{3000}KRX_APP_SECRET:비밀";

        assert_eq!(redactor.scan(input), vec!["KRX_APP_SECRET=비밀"]);
        let output = redactor.redact(input);
        assert_eq!(output, "KRX_APP_SECRET=\u{3000}KRX_APP_SECRET:[REDACTED]");
        assert!(redactor.is_clean(&output));
        assert_eq!(redactor.redact(&output), output);
    }

    #[test]
    fn authorization_redaction_is_case_insensitive_and_escape_aware() {
        let redactor = Redactor::new();
        let cases = [
            (
                r#"Authorization:"top\"secret""#,
                r#"Authorization:"[REDACTED]""#,
            ),
            ("Authorization:Bearer secret", "Authorization:[REDACTED]"),
            ("authorization: bearer secret", "authorization: [REDACTED]"),
            ("Authorization:Basic abc123", "Authorization:[REDACTED]"),
            (
                r#"Authorization:"top\"secret" authorization: bearer second"#,
                r#"Authorization:"[REDACTED]" authorization: [REDACTED]"#,
            ),
            ("auth=bEaReR secret", "auth=bEaReR [REDACTED]"),
        ];

        for (input, expected) in cases {
            assert!(!redactor.scan(input).is_empty(), "input: {input:?}");
            let output = redactor.redact(input);
            assert_eq!(output, expected, "input: {input:?}");
            assert!(redactor.is_clean(&output), "output: {output:?}");
            assert_eq!(redactor.redact(&output), output);
        }
    }

    #[test]
    fn authorization_redacts_multi_parameter_values_through_the_header_line() {
        let redactor = Redactor::new();
        let cases = [
            (
                r#"authorization: Digest username="alice", realm="supersecret", response="hashvalue""#,
                "authorization: [REDACTED]",
            ),
            (
                "AUTHORIZATION: AWS4-HMAC-SHA256 Credential=alice/20260810/ap-northeast-2/service/aws4_request, SignedHeaders=host;x-amz-date, Signature=deadbeef\r\nX-Public: visible",
                "AUTHORIZATION: [REDACTED]\r\nX-Public: visible",
            ),
            (
                "Authorization:\"top\\\"secret\"\r\nX-Public: quoted-visible",
                "Authorization:\"[REDACTED]\"\r\nX-Public: quoted-visible",
            ),
            (
                "Authorization: Digest username=first, response=one\r\nX-Public: keep\r\naUtHoRiZaTiOn: aws4-hmac-sha256 Credential=second, Signature=two\r\nX-Tail: keep-too",
                "Authorization: [REDACTED]\r\nX-Public: keep\r\naUtHoRiZaTiOn: [REDACTED]\r\nX-Tail: keep-too",
            ),
        ];

        for (input, expected) in cases {
            assert!(!redactor.is_clean(input), "input: {input:?}");
            let output = redactor.redact(input);
            assert_eq!(output, expected, "input: {input:?}");
            assert!(redactor.scan(&output).is_empty(), "output: {output:?}");
            assert_eq!(redactor.redact(&output), output);
        }

        let empty_value = "Authorization:   \r\nX-Public: must-remain";
        assert!(redactor.is_clean(empty_value));
        assert_eq!(redactor.redact(empty_value), empty_value);
    }
}
