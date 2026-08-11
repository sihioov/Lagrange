use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

pub const AUTH0_CLIENT_SECRET_FILE: &str = "AUTH0_CLIENT_SECRET_FILE";

pub struct ClientSecret {
    value: Zeroizing<String>,
}

impl ClientSecret {
    pub fn from_env() -> Result<Self, ClientSecretError> {
        let path = env::var_os(AUTH0_CLIENT_SECRET_FILE).ok_or(ClientSecretError::MissingPath)?;
        Self::from_file(path)
    }

    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, ClientSecretError> {
        let path = path.as_ref().to_path_buf();
        let mut value = fs::read_to_string(&path).map_err(|source| ClientSecretError::Read {
            path: path.clone(),
            source,
        })?;

        if value.ends_with("\r\n") {
            value.truncate(value.len() - 2);
        } else if value.ends_with('\n') {
            value.pop();
        }

        if value.trim().is_empty() {
            return Err(ClientSecretError::Empty { path });
        }

        Ok(Self {
            value: Zeroizing::new(value),
        })
    }

    pub(crate) fn expose(&self) -> &str {
        self.value.as_str()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClientSecretError {
    #[error("{AUTH0_CLIENT_SECRET_FILE} is required")]
    MissingPath,
    #[error("{AUTH0_CLIENT_SECRET_FILE} cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{AUTH0_CLIENT_SECRET_FILE} contains an empty secret at {path}")]
    Empty { path: PathBuf },
}
