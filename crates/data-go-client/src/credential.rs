//! Protected-file handling for the FSC portal service key.
//!
//! The value is intentionally available only to the private client transport.
//! Callers can inspect the source path and safe metadata, but cannot obtain a
//! plain `String` through this crate's public API except via the redacted
//! [`Secret`] wrapper's explicit `expose` method at the transport seam.

use std::env;
use std::fmt;
use std::path::{Path, PathBuf};

/// Names a path, never the service-key value itself.
pub const SERVICE_KEY_FILE_ENV_VAR: &str = "FSC_KRX_LISTED_KEY_FILE";
const DEFAULT_SERVICE_KEY_FILE: &str = "/etc/lagrange/secrets/fsc_krx_listed_service_key";
/// Maximum number of bytes accepted from the protected service-key file.
///
/// The service key is a short opaque value. Keeping this bound small makes a
/// malformed file unable to cause an unbounded allocation at the credential
/// boundary.
pub const MAX_SERVICE_KEY_BYTES: usize = 4096;

/// A value that must never be formatted with its contents.
pub struct Secret<T>(T);

impl<T> Secret<T> {
    pub(crate) fn new(value: T) -> Self {
        Self(value)
    }

    /// The only value access at the transport boundary.
    pub(crate) fn expose(&self) -> &T {
        &self.0
    }
}

impl<T> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// Safe source metadata for the key file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialRef {
    File(PathBuf),
    EnvVarNamingFile(String),
}

impl CredentialRef {
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self::File(path.into())
    }

    pub fn env(name: impl Into<String>) -> Self {
        Self::EnvVarNamingFile(name.into())
    }

    pub fn describe(&self) -> String {
        match self {
            Self::File(path) => format!("file `{}`", path.display()),
            Self::EnvVarNamingFile(name) => {
                format!("path named by environment variable `{name}`")
            }
        }
    }
}

/// Typed configuration errors. None carry a path or value, so they remain
/// safe to print from an operator CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CredentialError {
    #[error("service-key file path environment variable is empty")]
    EnvVarEmpty,
    #[error("service-key file is missing")]
    FileNotFound,
    #[error("service-key file is not a regular non-symlink file")]
    FileUnreadable,
    #[error("service-key file is empty or whitespace-only")]
    FileEmpty,
    #[error("service-key file is larger than the permitted bound")]
    FileTooLarge,
    #[error("service-key file must have mode 0600")]
    InvalidMode,
    #[error("protected service-key files are unsupported on this platform")]
    UnsupportedPlatform,
}

/// A credential source reads only a protected file. Implementations used by
/// tests can supply a synthetic value without any network or filesystem call.
pub trait CredentialSource: Send + Sync {
    fn load(&self) -> Result<Secret<String>, CredentialError>;
}

/// Production source: a fixed path by default, optionally overridden by an
/// environment variable containing a *path*, never the key value.
pub struct SystemCredentialSource {
    path: PathBuf,
    reference: CredentialRef,
}

impl SystemCredentialSource {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        Self {
            reference: CredentialRef::file(path.clone()),
            path,
        }
    }

    pub fn from_env_or_default() -> Result<Self, CredentialError> {
        match env::var(SERVICE_KEY_FILE_ENV_VAR) {
            Ok(path) if !path.trim().is_empty() => Ok(Self {
                reference: CredentialRef::env(SERVICE_KEY_FILE_ENV_VAR),
                path: PathBuf::from(path),
            }),
            Ok(_) => Err(CredentialError::EnvVarEmpty),
            Err(_) => Ok(Self::new(DEFAULT_SERVICE_KEY_FILE)),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn credential_ref(&self) -> &CredentialRef {
        &self.reference
    }

    /// Checks file shape and permissions without reading its contents.
    pub fn check_metadata(&self) -> Result<(), CredentialError> {
        let metadata = std::fs::symlink_metadata(&self.path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                CredentialError::FileNotFound
            } else {
                CredentialError::FileUnreadable
            }
        })?;
        if !metadata.file_type().is_file() {
            return Err(CredentialError::FileUnreadable);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o777 != 0o600 {
                return Err(CredentialError::InvalidMode);
            }
        }
        Ok(())
    }
}

impl Default for SystemCredentialSource {
    fn default() -> Self {
        Self::from_env_or_default().unwrap_or_else(|_| Self::new(DEFAULT_SERVICE_KEY_FILE))
    }
}

impl fmt::Debug for SystemCredentialSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SystemCredentialSource")
            .field("reference", &self.reference)
            .finish_non_exhaustive()
    }
}

impl CredentialSource for SystemCredentialSource {
    fn load(&self) -> Result<Secret<String>, CredentialError> {
        #[cfg(unix)]
        {
            use rustix::fs::{FileType, Mode, OFlags, fstat, open};
            use std::fs::File;
            use std::io::Read;

            // Open once with O_NOFOLLOW. All subsequent checks and reads use
            // this descriptor, so a path replacement cannot swap the object
            // between a metadata check and the credential read.
            let fd = open(
                &self.path,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(map_open_error)?;
            let stat = fstat(&fd).map_err(|_| CredentialError::FileUnreadable)?;
            if !FileType::from_raw_mode(stat.st_mode).is_file() {
                return Err(CredentialError::FileUnreadable);
            }
            if Mode::from_raw_mode(stat.st_mode) != (Mode::RUSR | Mode::WUSR) {
                return Err(CredentialError::InvalidMode);
            }

            let mut file = File::from(fd);
            let mut bytes = Vec::with_capacity(MAX_SERVICE_KEY_BYTES + 1);
            (&mut file)
                .take((MAX_SERVICE_KEY_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|_| CredentialError::FileUnreadable)?;
            if bytes.len() > MAX_SERVICE_KEY_BYTES {
                return Err(CredentialError::FileTooLarge);
            }
            let value = String::from_utf8(bytes).map_err(|_| CredentialError::FileUnreadable)?;
            let trimmed = value.trim();
            if trimmed.is_empty() {
                return Err(CredentialError::FileEmpty);
            }
            Ok(Secret::new(trimmed.to_owned()))
        }

        #[cfg(not(unix))]
        {
            // There is no equivalent single-FD O_NOFOLLOW contract here.
            // Refuse to read rather than silently reintroduce a path race.
            let _ = &self.path;
            Err(CredentialError::UnsupportedPlatform)
        }
    }
}

#[cfg(unix)]
fn map_open_error(error: rustix::io::Errno) -> CredentialError {
    if std::io::Error::from_raw_os_error(error.raw_os_error()).kind()
        == std::io::ErrorKind::NotFound
    {
        CredentialError::FileNotFound
    } else {
        CredentialError::FileUnreadable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn protected_file(path: &Path, contents: &[u8]) {
        use std::os::unix::fs::PermissionsExt;

        std::fs::write(path, contents).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlink_is_rejected_without_following_it() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        let link = temp.path().join("service-key");
        protected_file(&target, b"service-key-sentinel-test-only");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let error = SystemCredentialSource::new(link).load().unwrap_err();
        assert_eq!(error, CredentialError::FileUnreadable);
    }

    #[cfg(unix)]
    #[test]
    fn oversized_service_key_is_rejected_at_the_read_boundary() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("service-key");
        protected_file(&path, &vec![b'k'; MAX_SERVICE_KEY_BYTES + 1]);

        let error = SystemCredentialSource::new(path).load().unwrap_err();
        assert_eq!(error, CredentialError::FileTooLarge);
    }

    #[cfg(unix)]
    #[test]
    fn exact_service_key_bound_is_accepted_without_unbounded_read() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("service-key");
        protected_file(&path, &vec![b'k'; MAX_SERVICE_KEY_BYTES]);

        let secret = SystemCredentialSource::new(path).load().unwrap();
        assert_eq!(secret.expose().len(), MAX_SERVICE_KEY_BYTES);
    }
}
