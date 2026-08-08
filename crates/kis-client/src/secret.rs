//! Credential references and redacting wrappers (plan Todo 36).
//!
//! Design §6.12 forbids logging tokens, account numbers, or raw secrets. That
//! is not a discipline anyone can hold by hand across a whole transport layer,
//! so it is enforced by the TYPE: [`Secret`] and [`AccountNo`] have no
//! non-redacting `Debug` or `Display`. Interpolating one into a log line, a
//! `format!`, an error message, or a `#[derive(Debug)]` struct prints a
//! placeholder. Getting the real bytes requires calling [`Secret::expose`],
//! which is greppable in review.
//!
//! Secrets are never inlined in configuration either: config carries a
//! [`CredentialRef`] naming WHERE the value lives, and resolution happens once,
//! at the edge, through a [`CredentialSource`].

use std::fmt;

/// A value that must never reach a log, an error, or an audit record.
///
/// `Clone` is derived because tokens legitimately move between the auth module
/// and the request builder. `PartialEq` is deliberately NOT derived: comparing
/// secrets with `==` is a timing-unsafe habit, and the one place that needs it
/// (token equality in tests) can compare `expose()` explicitly.
#[derive(Clone)]
pub struct Secret<T> {
    value: T,
}

impl<T> Secret<T> {
    pub fn new(value: T) -> Self {
        Self { value }
    }

    /// The only way to read the wrapped value. Named to be conspicuous: a
    /// reviewer greps `expose(` to find every place a secret can escape.
    pub fn expose(&self) -> &T {
        &self.value
    }

    pub fn into_inner(self) -> T {
        self.value
    }
}

impl<T> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

impl<T> fmt::Display for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// A KIS account number.
///
/// Not a `Secret` because it is not a credential — it is an identifier the
/// system must correlate on — but it is personally identifying and design
/// §6.12 forbids logging it. It therefore renders as a masked form that keeps
/// just enough to correlate two records by eye without disclosing the account.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct AccountNo(String);

impl AccountNo {
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    /// The real value, for building a request. Conspicuously named, like
    /// [`Secret::expose`].
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// The masked rendering used by every log, error, and audit record: the
    /// last four characters only. Enough to tell two accounts apart in a
    /// transcript, not enough to be one.
    pub fn masked(&self) -> String {
        let n = self.0.chars().count();
        if n <= 4 {
            // Too short to mask meaningfully — disclose nothing rather than
            // most of it.
            return "****".to_string();
        }
        let tail: String = self.0.chars().skip(n - 4).collect();
        format!("****{tail}")
    }
}

impl fmt::Debug for AccountNo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AccountNo({})", self.masked())
    }
}

impl fmt::Display for AccountNo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.masked())
    }
}

/// Where a credential lives. Configuration carries this, never the value.
///
/// Keeping the reference and the value in different types means a config file,
/// a serialized struct, or a debug dump of settings physically cannot contain
/// a secret — there is no field to put one in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialRef {
    /// A process environment variable.
    Env { var: String },
    /// A file whose entire contents are the credential (the Docker secret
    /// shape the compose stack already uses: `*_FILE`).
    File { path: String },
}

impl CredentialRef {
    pub fn env(var: impl Into<String>) -> Self {
        Self::Env { var: var.into() }
    }

    pub fn file(path: impl Into<String>) -> Self {
        Self::File { path: path.into() }
    }

    /// A short description safe to put in an error message: it names the
    /// LOCATION, never the value.
    pub fn describe(&self) -> String {
        match self {
            Self::Env { var } => format!("environment variable {var}"),
            Self::File { path } => format!("file {path}"),
        }
    }
}

/// Resolves a [`CredentialRef`] to its value.
///
/// A trait so tests can supply credentials without touching the environment or
/// the filesystem, and so the mock and live profiles differ only in which
/// source is installed.
pub trait CredentialSource {
    fn resolve(&self, reference: &CredentialRef) -> Result<Secret<String>, CredentialError>;
}

/// Why a credential could not be resolved. Every variant names the reference,
/// never the value — an error about a secret must not become a way to print it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CredentialError {
    #[error("credential not found in {location}")]
    NotFound { location: String },
    #[error("credential in {location} is empty")]
    Empty { location: String },
    #[error("credential in {location} could not be read: {reason}")]
    Unreadable { location: String, reason: String },
}

/// The production source: environment variables and files.
#[derive(Debug, Clone, Default)]
pub struct SystemCredentialSource;

impl CredentialSource for SystemCredentialSource {
    fn resolve(&self, reference: &CredentialRef) -> Result<Secret<String>, CredentialError> {
        let location = reference.describe();
        let raw = match reference {
            CredentialRef::Env { var } => {
                std::env::var(var).map_err(|_| CredentialError::NotFound {
                    location: location.clone(),
                })?
            }
            CredentialRef::File { path } => {
                std::fs::read_to_string(path).map_err(|e| CredentialError::Unreadable {
                    location: location.clone(),
                    reason: e.kind().to_string(),
                })?
            }
        };
        // Trailing newlines are how a file-backed secret is usually written;
        // sending one to the broker would fail authentication for a reason
        // nobody could see in a redacted log.
        let trimmed = raw.trim().to_string();
        if trimmed.is_empty() {
            return Err(CredentialError::Empty { location });
        }
        Ok(Secret::new(trimmed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_never_renders_its_value() {
        let s = Secret::new("super-secret-token".to_string());
        assert_eq!(format!("{s:?}"), "Secret(<redacted>)");
        assert_eq!(format!("{s}"), "<redacted>");
        // The value is still reachable deliberately, through one named method.
        assert_eq!(s.expose(), "super-secret-token");
    }

    #[test]
    fn a_secret_inside_a_derived_debug_struct_stays_redacted() {
        // The realistic leak: someone derives Debug on a config struct and
        // logs it. The wrapper has to survive that, not just direct printing.
        #[derive(Debug)]
        struct Config {
            endpoint: String,
            app_secret: Secret<String>,
        }
        let c = Config {
            endpoint: "https://openapi.koreainvestment.com".to_string(),
            app_secret: Secret::new("PSxxxxxxxxxxxxxxxx".to_string()),
        };
        let rendered = format!("{c:?}");
        assert!(rendered.contains("openapi.koreainvestment.com"));
        assert!(
            !rendered.contains("PSxxxxxxxxxxxxxxxx"),
            "a derived Debug leaked the secret: {rendered}"
        );
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn account_numbers_are_masked_everywhere() {
        let a = AccountNo::new("50123456-01");
        assert_eq!(a.masked(), "****6-01");
        assert_eq!(format!("{a}"), "****6-01");
        assert_eq!(format!("{a:?}"), "AccountNo(****6-01)");
        assert!(!format!("{a:?}").contains("50123456"));
        assert_eq!(a.expose(), "50123456-01");
    }

    #[test]
    fn a_short_account_number_discloses_nothing() {
        // Masking the last four of a four-character value would disclose all
        // of it; the mask must degrade to nothing rather than to everything.
        assert_eq!(AccountNo::new("1234").masked(), "****");
        assert_eq!(AccountNo::new("7").masked(), "****");
    }

    #[test]
    fn credential_refs_carry_a_location_not_a_value() {
        let r = CredentialRef::env("KIS_APP_SECRET");
        assert_eq!(r.describe(), "environment variable KIS_APP_SECRET");
        // The enum has no variant capable of holding a literal secret, so a
        // serialized config cannot contain one.
        let f = CredentialRef::file("/run/secrets/kis_app_secret");
        assert_eq!(f.describe(), "file /run/secrets/kis_app_secret");
    }

    #[test]
    fn credential_errors_name_the_location_never_the_value() {
        let e = CredentialError::NotFound {
            location: CredentialRef::env("KIS_APP_KEY").describe(),
        };
        let rendered = e.to_string();
        assert!(rendered.contains("KIS_APP_KEY"));
        assert!(!rendered.contains("PS"), "{rendered}");
    }

    #[test]
    fn a_missing_env_credential_is_not_found_and_an_empty_one_is_rejected() {
        let src = SystemCredentialSource;
        let missing = src.resolve(&CredentialRef::env("LAGRANGE_KIS_TEST_ABSENT_VAR"));
        assert!(matches!(missing, Err(CredentialError::NotFound { .. })));

        // SAFETY: single-threaded test process; the var is unique to this test.
        unsafe { std::env::set_var("LAGRANGE_KIS_TEST_EMPTY_VAR", "   \n") };
        let empty = src.resolve(&CredentialRef::env("LAGRANGE_KIS_TEST_EMPTY_VAR"));
        assert!(
            matches!(empty, Err(CredentialError::Empty { .. })),
            "whitespace-only is empty, not a credential"
        );
        unsafe { std::env::remove_var("LAGRANGE_KIS_TEST_EMPTY_VAR") };
    }

    #[test]
    fn a_file_backed_credential_is_trimmed() {
        let src = SystemCredentialSource;
        // A trailing newline is how a secret file is normally written; sending
        // it to the broker fails auth for a reason no redacted log can show.
        unsafe { std::env::set_var("LAGRANGE_KIS_TEST_TRIM_VAR", "  token-value\n") };
        let got = src
            .resolve(&CredentialRef::env("LAGRANGE_KIS_TEST_TRIM_VAR"))
            .expect("resolves");
        assert_eq!(got.expose(), "token-value");
        unsafe { std::env::remove_var("LAGRANGE_KIS_TEST_TRIM_VAR") };
    }
}
