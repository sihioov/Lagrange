//! Credential handling for the OpenDART `crtfc_key`.
//!
//! The key is a *query parameter*, not a header, so it leaks far more
//! easily than the KIS bearer token this crate's shape is modeled on: into
//! request logs, into `reqwest::Error`'s `Display`, into redirect targets.
//! This module's job is to get the key from disk into a [`Secret`] and no
//! further -- never through an environment variable or argv, and never
//! through a `Debug` implementation that prints it.

use std::env;
use std::fmt;
use std::path::PathBuf;

/// The default environment variable this crate reads for the credential
/// *path*. It names a file, never the key value itself.
pub const CRTFC_KEY_FILE_ENV_VAR: &str = "OPENDART_CRTFC_KEY_FILE";

/// A value that must never appear in a `Debug`/`Display` implementation.
///
/// `Debug` is hand-written (never `derive`d) so a future struct that embeds
/// a `Secret` field can't accidentally print the wrapped value just by
/// deriving `Debug` itself.
pub struct Secret<T>(T);

impl<T> Secret<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// The only way to get at the wrapped value. Named loudly so call sites
    /// stand out under review.
    pub fn expose(&self) -> &T {
        &self.0
    }
}

impl<T> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// Names *where* a credential comes from without holding or revealing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialRef {
    /// The credential file's path is read from this environment variable.
    EnvVarNamingFile(String),
    /// The credential file's path, already resolved.
    File(PathBuf),
}

impl CredentialRef {
    pub fn env(name: impl Into<String>) -> Self {
        Self::EnvVarNamingFile(name.into())
    }

    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self::File(path.into())
    }

    /// A human-readable description of the *source*, never the value.
    pub fn describe(&self) -> String {
        match self {
            Self::EnvVarNamingFile(name) => format!("path named by environment variable `{name}`"),
            Self::File(path) => format!("file `{}`", path.display()),
        }
    }
}

/// Why loading the credential failed. Distinct variants per failure mode so
/// callers (and tests) can tell "you forgot to set the env var" apart from
/// "the file is there but empty".
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CredentialError {
    #[error("credential file path environment variable is not set")]
    EnvVarMissing,
    #[error("credential file does not exist")]
    FileNotFound,
    #[error("credential file could not be read")]
    FileUnreadable,
    #[error("credential file is empty or contains only whitespace")]
    FileEmpty,
}

/// Reads the `crtfc_key` value. The only shipped implementation is
/// [`SystemCredentialSource`]; the trait exists so tests can substitute a
/// fixed value without touching the environment or filesystem.
pub trait CredentialSource {
    fn load(&self) -> Result<Secret<String>, CredentialError>;
}

/// Reads the key from the file named by an environment variable
/// (`OPENDART_CRTFC_KEY_FILE` by default). Never reads the key value itself
/// from an environment variable, and never from argv.
pub struct SystemCredentialSource {
    env_var_name: String,
}

impl SystemCredentialSource {
    pub fn new(env_var_name: impl Into<String>) -> Self {
        Self {
            env_var_name: env_var_name.into(),
        }
    }
}

impl Default for SystemCredentialSource {
    fn default() -> Self {
        Self::new(CRTFC_KEY_FILE_ENV_VAR)
    }
}

impl CredentialSource for SystemCredentialSource {
    fn load(&self) -> Result<Secret<String>, CredentialError> {
        let path = env::var(&self.env_var_name).map_err(|_| CredentialError::EnvVarMissing)?;
        let contents = std::fs::read_to_string(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                CredentialError::FileNotFound
            } else {
                CredentialError::FileUnreadable
            }
        })?;
        // "Trim trailing whitespace and newlines" -- `trim_end` covers both,
        // and a whitespace-only file collapses to "" here (every byte in it
        // is trailing whitespace), so the emptiness check below also
        // catches "whitespace-only".
        let trimmed = contents.trim_end();
        if trimmed.is_empty() {
            return Err(CredentialError::FileEmpty);
        }
        Ok(Secret::new(trimmed.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // `std::env::set_var`/`remove_var` are process-global and, since
    // Rust 1.82, `unsafe` for exactly that reason. Serialize every test
    // that touches the environment so concurrent test threads cannot
    // interleave `set_var` calls, even though each test uses its own
    // uniquely named variable.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn set_env(name: &str, value: &str) {
        // SAFETY: serialized by `ENV_LOCK` for the duration of the call site.
        unsafe { env::set_var(name, value) };
    }

    fn remove_env(name: &str) {
        // SAFETY: serialized by `ENV_LOCK` for the duration of the call site.
        unsafe { env::remove_var(name) };
    }

    #[test]
    fn missing_env_var_fails_closed() {
        let _guard = ENV_LOCK.lock().unwrap();
        let var = "OPENDART_TEST_MISSING_VAR";
        remove_env(var);
        let source = SystemCredentialSource::new(var);
        assert_eq!(source.load().unwrap_err(), CredentialError::EnvVarMissing);
    }

    #[test]
    fn nonexistent_path_fails_closed() {
        let _guard = ENV_LOCK.lock().unwrap();
        let var = "OPENDART_TEST_NONEXISTENT_PATH";
        set_env(var, "/nonexistent/lagrange-opendart-test-path/key.txt");
        let source = SystemCredentialSource::new(var);
        assert_eq!(source.load().unwrap_err(), CredentialError::FileNotFound);
        remove_env(var);
    }

    #[test]
    fn empty_file_fails_closed() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key.txt");
        std::fs::write(&path, "").unwrap();
        let var = "OPENDART_TEST_EMPTY_FILE";
        set_env(var, path.to_str().unwrap());
        let source = SystemCredentialSource::new(var);
        assert_eq!(source.load().unwrap_err(), CredentialError::FileEmpty);
        remove_env(var);
    }

    #[test]
    fn whitespace_only_file_fails_closed() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key.txt");
        std::fs::write(&path, "   \n\t \n").unwrap();
        let var = "OPENDART_TEST_WHITESPACE_FILE";
        set_env(var, path.to_str().unwrap());
        let source = SystemCredentialSource::new(var);
        assert_eq!(source.load().unwrap_err(), CredentialError::FileEmpty);
        remove_env(var);
    }

    #[test]
    fn unreadable_path_fails_closed() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        // A directory is not a valid credential file. Reading it via
        // `read_to_string` yields an IO error distinct from "not found",
        // which is what we're asserting classifies as `FileUnreadable`.
        let var = "OPENDART_TEST_UNREADABLE_PATH";
        set_env(var, dir.path().to_str().unwrap());
        let source = SystemCredentialSource::new(var);
        assert_eq!(source.load().unwrap_err(), CredentialError::FileUnreadable);
        remove_env(var);
    }

    #[test]
    fn trailing_newline_is_trimmed() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key.txt");
        const SENTINEL: &str = "sk-lagrange-test-sentinel-0f9c";
        std::fs::write(&path, format!("{SENTINEL}\n")).unwrap();
        let var = "OPENDART_TEST_TRAILING_NEWLINE";
        set_env(var, path.to_str().unwrap());
        let source = SystemCredentialSource::new(var);
        let secret = source.load().expect("file has a value");
        // Assert equality without ever printing the value.
        assert_eq!(secret.expose().as_str(), SENTINEL);
        assert_eq!(secret.expose().len(), SENTINEL.len());
        remove_env(var);
    }

    #[test]
    fn secret_debug_is_redacted() {
        let secret = Secret::new("sk-must-not-appear-in-debug".to_string());
        let debug_output = format!("{secret:?}");
        assert_eq!(debug_output, "Secret(<redacted>)");
        assert!(!debug_output.contains("sk-must-not-appear-in-debug"));
    }

    #[test]
    fn credential_ref_describe_never_holds_a_value() {
        let by_env = CredentialRef::env("SOME_VAR");
        let by_file = CredentialRef::file("/some/path/key.txt");
        assert_eq!(
            by_env.describe(),
            "path named by environment variable `SOME_VAR`"
        );
        assert_eq!(by_file.describe(), "file `/some/path/key.txt`");
    }
}
