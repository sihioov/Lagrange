use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use url::Url;
use zeroize::Zeroizing;

pub const AUTH0_CLIENT_SECRET_FILE: &str = "AUTH0_CLIENT_SECRET_FILE";
pub const DEFAULT_AUTH0_REDIRECT_URI: &str = "https://app.lagrange.local/auth/callback";
pub const MAX_AUTH0_CLOCK_SKEW_SECS: i64 = auth::oidc::MAX_CLOCK_SKEW_SECS;

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
        let metadata = fs::symlink_metadata(&path).map_err(|source| ClientSecretError::Read {
            path: path.clone(),
            source,
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(ClientSecretError::InvalidFile { path });
        }
        let value = Zeroizing::new(fs::read_to_string(&path).map_err(|source| {
            ClientSecretError::Read {
                path: path.clone(),
                source,
            }
        })?);

        if value.contains('\r') || value.contains('\n') {
            return Err(ClientSecretError::MultipleLines { path });
        }

        if value.trim().is_empty() {
            return Err(ClientSecretError::Empty { path });
        }

        Ok(Self { value })
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
    #[error("{AUTH0_CLIENT_SECRET_FILE} must contain exactly one line at {path}")]
    MultipleLines { path: PathBuf },
    #[error("{AUTH0_CLIENT_SECRET_FILE} must be a regular non-symlink file at {path}")]
    InvalidFile { path: PathBuf },
}

/// Non-secret OIDC settings plus the confidential client secret loaded from
/// its mounted file. The secret is intentionally not exposed through a
/// getter or a Debug implementation; the transport consumes it directly.
pub struct ProductionAuthConfig {
    pub provider: auth::oidc::OidcProviderConfig,
    pub(crate) client_secret: ClientSecret,
}

#[derive(Debug, thiserror::Error)]
pub enum ProductionAuthConfigError {
    #[error("{key} is required")]
    Missing { key: &'static str },
    #[error("{key} must not be empty")]
    Empty { key: &'static str },
    #[error("AUTH0_DOMAIN must be an HTTPS host without a path")]
    InvalidDomain,
    #[error("AUTH0_REDIRECT_URI must be an absolute HTTPS URL without a query or fragment")]
    InvalidRedirectUri,
    #[error("AUTH0_CLOCK_SKEW_SECS must be an integer from 0 through 300")]
    InvalidClockSkew,
    #[error(transparent)]
    ClientSecret(#[from] ClientSecretError),
}

impl ProductionAuthConfig {
    /// Derive a process-local transaction-cookie MAC key without exposing the
    /// client secret to the router or to any serialized configuration. A
    /// restart invalidates outstanding browser transactions, which is safe
    /// and preferable to accepting a transaction minted by an older process.
    pub(crate) fn transaction_cookie_key(&self) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update(b"lagrange-station/oidc-transaction-cookie/v1\0");
        digest.update(self.client_secret.expose().as_bytes());
        digest.finalize().into()
    }

    /// Load the production Auth0 contract from process environment and the
    /// mounted client-secret file. No plaintext secret fallback exists.
    pub fn from_env() -> Result<Self, ProductionAuthConfigError> {
        Self::from_values(
            env::var("AUTH0_DOMAIN").ok(),
            env::var("AUTH0_CLIENT_ID").ok(),
            env::var("AUTH0_REDIRECT_URI").ok(),
            env::var("AUTH0_AUDIENCE").ok(),
            env::var("AUTH0_CLOCK_SKEW_SECS").ok(),
            ClientSecret::from_env()?,
        )
    }

    /// Credential-free constructor used by simulator/router tests. The
    /// transport still receives a ClientSecret, but tests can use a local
    /// temporary file and never need a tenant credential.
    pub fn from_values(
        domain: Option<String>,
        client_id: Option<String>,
        redirect_uri: Option<String>,
        audience: Option<String>,
        clock_skew_secs: Option<String>,
        client_secret: ClientSecret,
    ) -> Result<Self, ProductionAuthConfigError> {
        let domain = required("AUTH0_DOMAIN", domain)?;
        let client_id = required("AUTH0_CLIENT_ID", client_id)?;
        let issuer = normalize_issuer(&domain)?;
        let redirect_uri = redirect_uri.unwrap_or_else(|| DEFAULT_AUTH0_REDIRECT_URI.to_string());
        validate_redirect_uri(&redirect_uri)?;
        let clock_skew_secs = match clock_skew_secs {
            None => auth::oidc::DEFAULT_CLOCK_SKEW_SECS,
            Some(value) => value
                .parse::<i64>()
                .ok()
                .filter(|value| (0..=MAX_AUTH0_CLOCK_SKEW_SECS).contains(value))
                .ok_or(ProductionAuthConfigError::InvalidClockSkew)?,
        };
        let audience = audience.and_then(|value| {
            let value = value.trim().to_owned();
            (!value.is_empty()).then_some(value)
        });
        Ok(Self {
            provider: auth::oidc::OidcProviderConfig {
                issuer: issuer.clone(),
                client_id,
                redirect_uri,
                authorize_url: format!("{issuer}authorize"),
                token_url: format!("{issuer}oauth/token"),
                jwks_url: format!("{issuer}.well-known/jwks.json"),
                audience,
                clock_skew_secs,
            },
            client_secret,
        })
    }
}

fn required(key: &'static str, value: Option<String>) -> Result<String, ProductionAuthConfigError> {
    let value = value.ok_or(ProductionAuthConfigError::Missing { key })?;
    let value = value.trim().to_owned();
    if value.is_empty() {
        return Err(ProductionAuthConfigError::Empty { key });
    }
    Ok(value)
}

fn normalize_issuer(domain: &str) -> Result<String, ProductionAuthConfigError> {
    let candidate = if domain.starts_with("https://") {
        domain.to_owned()
    } else {
        format!("https://{domain}")
    };
    let url = Url::parse(&candidate).map_err(|_| ProductionAuthConfigError::InvalidDomain)?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || (url.path() != "" && url.path() != "/")
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ProductionAuthConfigError::InvalidDomain);
    }
    Ok(format!("{}/", candidate.trim_end_matches('/')))
}

fn validate_redirect_uri(value: &str) -> Result<(), ProductionAuthConfigError> {
    let url = Url::parse(value).map_err(|_| ProductionAuthConfigError::InvalidRedirectUri)?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path().is_empty()
    {
        return Err(ProductionAuthConfigError::InvalidRedirectUri);
    }
    Ok(())
}
