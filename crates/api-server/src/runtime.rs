//! Production process wiring for the Axum API.
//!
//! The route modules intentionally remain usable with the database-backed
//! test harness (`http::api_router`).  This module owns only the process
//! boundary: fail-closed environment parsing, three least-privilege pools,
//! health/readiness probes, and graceful shutdown.

use crate::http::api_router;
use crate::http::state::{ApiConfig, ApiState, system_seoul_today};
use api_server_auth::RouterState as AuthRouterState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::get};
use base64::Engine;
use job_queue::recommendation::input::DatasetPin;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::ffi::OsString;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

const DEFAULT_HOST: &str = "0.0.0.0";
const DEFAULT_PORT: u16 = 8080;
const DEFAULT_POOL_SIZE: u32 = 16;
const DEFAULT_ADMIN_POOL_SIZE: u32 = 4;
const DEFAULT_AUDIT_POOL_SIZE: u32 = 4;
const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 10;
const DEFAULT_MAX_JOBS_PER_OWNER: u32 = 10;
const DEFAULT_STEP_UP_MAX_AUTH_AGE_SECS: i64 = 900;
const DEFAULT_ARTIFACT_ROOT: &str = "/data/artifacts";
const DEVELOPMENT_CODE_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
/// Maximum time allowed for in-flight HTTP requests after the shutdown signal
/// is observed. Dropping the server future after this deadline is the safe
/// forced-drain path: no request is allowed to hold the audit shutdown hostage.
pub const GRACEFUL_SHUTDOWN_DEADLINE: Duration = Duration::from_secs(30);
const AUDIT_DRAIN_DEADLINE: Duration = Duration::from_secs(10);

/// Runtime configuration after parsing and validating process inputs.
///
/// Database URLs are retained because `ApiState` uses the app URL to create
/// actor-GUC pools for queued work.  This type intentionally does not derive
/// `Debug`: a PostgreSQL URL can contain a password and must never be emitted
/// by an accidental config log.
#[derive(Clone)]
pub struct RuntimeConfig {
    pub listen_addr: SocketAddr,
    pub database: DatabaseConfig,
    pub cursor_secret: [u8; 32],
    pub max_jobs_per_owner: u32,
    pub step_up_max_auth_age_secs: i64,
    pub artifact_root: PathBuf,
    pub recommendation_dataset: DatasetPin,
    /// Immutable source revision baked into the API image and copied into
    /// every API-created backtest run.
    pub code_commit: String,
    pub acquire_timeout: Duration,
}

/// The three independent PostgreSQL connection strings and pool limits.
///
/// The app role serves tenant routes, the admin role serves owner-only
/// operational reads, and the audit role is the append-only writer.  Keeping
/// these values separate prevents a broad role from silently becoming the
/// fallback when one pool is misconfigured.
#[derive(Clone)]
pub struct DatabaseConfig {
    pub app_url: String,
    pub admin_url: String,
    pub audit_url: String,
    pub app_max_connections: u32,
    pub admin_max_connections: u32,
    pub audit_max_connections: u32,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("{key} is required")]
    Missing { key: String },
    #[error("{key} must not be empty")]
    Empty { key: String },
    #[error("{key} and {file_key} are mutually exclusive")]
    Ambiguous { key: String, file_key: String },
    #[error("{key} is invalid")]
    Invalid { key: String },
    #[error("{key} could not be read")]
    Unreadable { key: String },
    #[error("{key} must be a non-symlink regular file")]
    InvalidFile { key: String },
    #[error("{key} is not a valid UTF-8 value")]
    NonUnicode { key: String },
    #[error("{key} is not a valid path")]
    InvalidPath { key: String },
}

impl RuntimeConfig {
    /// Build the route state configuration without opening a socket or a
    /// database connection.
    pub fn api_config(&self) -> ApiConfig {
        ApiConfig {
            cursor_secret: self.cursor_secret,
            max_jobs_per_owner: self.max_jobs_per_owner,
            recommendation_dataset: self.recommendation_dataset.clone(),
            db_url: self.database.app_url.clone(),
            step_up_max_auth_age_secs: self.step_up_max_auth_age_secs,
            artifact_root: self.artifact_root.clone(),
            seoul_today: system_seoul_today,
            candidate_eod_ready: crate::http::state::system_candidate_eod_ready,
            code_commit: self.code_commit.clone(),
        }
    }
}

/// Parse the process environment.  The closure form keeps this function
/// deterministic in tests and avoids mutating process-global environment
/// variables while test binaries run in parallel.
pub fn load_config() -> Result<RuntimeConfig, ConfigError> {
    load_config_from(|key| std::env::var_os(key))
}

pub fn load_config_from<F>(get: F) -> Result<RuntimeConfig, ConfigError>
where
    F: Fn(&str) -> Option<OsString>,
{
    let app_env = optional_text(&get, "APP_ENV")?
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_else(|| "production".to_owned());
    if !matches!(app_env.as_str(), "production" | "development" | "test") {
        return Err(invalid("APP_ENV"));
    }
    let production = app_env == "production";
    let recommendation_dataset = match recommendation_dataset(&get) {
        Ok(pin) => pin,
        Err(error) if !production && is_missing_dataset_error(&error) => development_dataset_pin(),
        Err(error) => return Err(error),
    };
    let code_commit = code_commit_from(&get, production)?;

    let listen_addr = listen_addr_from(&get)?;
    let database = DatabaseConfig {
        app_url: role_database_url(&get, DatabaseRole::App, production)?,
        admin_url: role_database_url(&get, DatabaseRole::Admin, production)?,
        audit_url: role_database_url(&get, DatabaseRole::Audit, production)?,
        app_max_connections: positive_u32(&get, "DB_APP_MAX_CONNECTIONS", DEFAULT_POOL_SIZE)?,
        admin_max_connections: positive_u32(
            &get,
            "DB_ADMIN_MAX_CONNECTIONS",
            DEFAULT_ADMIN_POOL_SIZE,
        )?,
        audit_max_connections: positive_u32(
            &get,
            "DB_AUDIT_MAX_CONNECTIONS",
            DEFAULT_AUDIT_POOL_SIZE,
        )?,
    };

    let cursor_secret = secret_32(&get, "CURSOR_SECRET", production)?;
    let max_jobs_per_owner = positive_u32(&get, "MAX_JOBS_PER_OWNER", DEFAULT_MAX_JOBS_PER_OWNER)?;
    let step_up_max_auth_age_secs = positive_i64(
        &get,
        "STEP_UP_MAX_AUTH_AGE_SECS",
        DEFAULT_STEP_UP_MAX_AUTH_AGE_SECS,
    )?;
    let artifact_root = artifact_root_from(&get)?;
    if artifact_root.as_os_str().is_empty() {
        return Err(ConfigError::InvalidPath {
            key: "ARTIFACT_ROOT".to_owned(),
        });
    }

    let acquire_timeout_secs = positive_u64(
        &get,
        "DB_ACQUIRE_TIMEOUT_SECS",
        DEFAULT_CONNECT_TIMEOUT_SECS,
    )?;

    Ok(RuntimeConfig {
        listen_addr,
        database,
        cursor_secret,
        max_jobs_per_owner,
        step_up_max_auth_age_secs,
        artifact_root,
        recommendation_dataset,
        code_commit,
        acquire_timeout: Duration::from_secs(acquire_timeout_secs),
    })
}

/// Parse the image revision once at startup. Production receives the exact
/// lowercase Git object name baked into the image; development/test retain a
/// deterministic value so the database-backed harness needs no build env.
fn code_commit_from<F>(get: &F, production: bool) -> Result<String, ConfigError>
where
    F: Fn(&str) -> Option<OsString>,
{
    match optional_text(get, "LAGRANGE_CODE_COMMIT")? {
        Some(value)
            if value.len() == 40
                && value.bytes().all(|byte| byte.is_ascii_hexdigit())
                && value.bytes().all(|byte| !byte.is_ascii_uppercase())
                && value.bytes().any(|byte| byte != b'0') =>
        {
            Ok(value)
        }
        Some(_) => Err(invalid("LAGRANGE_CODE_COMMIT")),
        None if !production => Ok(DEVELOPMENT_CODE_COMMIT.to_owned()),
        None => Err(ConfigError::Missing {
            key: "LAGRANGE_CODE_COMMIT".to_owned(),
        }),
    }
}

fn is_missing_dataset_error(error: &ConfigError) -> bool {
    matches!(error, ConfigError::Missing { key } if key.starts_with("RECOMMENDATION_DATASET_"))
}

fn development_dataset_pin() -> DatasetPin {
    DatasetPin {
        id: uuid::Uuid::nil(),
        dataset_id: "not-configured".to_owned(),
        version: "not-configured".to_owned(),
        curated_version: 1,
        manifest_sha256: "0".repeat(64),
    }
}

#[derive(Clone, Copy)]
enum DatabaseRole {
    App,
    Admin,
    Audit,
}

impl DatabaseRole {
    const fn label(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::Admin => "admin",
            Self::Audit => "audit",
        }
    }

    const fn url_key(self) -> &'static str {
        match self {
            Self::App => "DATABASE_URL",
            Self::Admin => "ADMIN_DATABASE_URL",
            Self::Audit => "AUDIT_DATABASE_URL",
        }
    }

    const fn default_user(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::Admin => "admin",
            Self::Audit => "audit_writer",
        }
    }

    const fn host_key(self) -> &'static str {
        match self {
            Self::App => "DB_HOST",
            Self::Admin => "ADMIN_DB_HOST",
            Self::Audit => "AUDIT_DB_HOST",
        }
    }

    const fn port_key(self) -> &'static str {
        match self {
            Self::App => "DB_PORT",
            Self::Admin => "ADMIN_DB_PORT",
            Self::Audit => "AUDIT_DB_PORT",
        }
    }

    const fn name_key(self) -> &'static str {
        match self {
            Self::App => "DB_NAME",
            Self::Admin => "ADMIN_DB_NAME",
            Self::Audit => "AUDIT_DB_NAME",
        }
    }

    const fn user_key(self) -> &'static str {
        match self {
            Self::App => "DB_USER",
            Self::Admin => "ADMIN_DB_USER",
            Self::Audit => "AUDIT_DB_USER",
        }
    }

    const fn password_key(self) -> &'static str {
        match self {
            Self::App => "DB_PASSWORD",
            Self::Admin => "ADMIN_DB_PASSWORD",
            Self::Audit => "AUDIT_DB_PASSWORD",
        }
    }

    const fn legacy_password_key(self) -> Option<&'static str> {
        match self {
            Self::App => None,
            Self::Admin => Some("DB_ADMIN_PASSWORD"),
            Self::Audit => Some("DB_AUDIT_PASSWORD"),
        }
    }

    const fn alternate_password_key(self) -> Option<&'static str> {
        match self {
            Self::Admin => Some("DB_ADMIN_ROLE_PASSWORD"),
            Self::App | Self::Audit => None,
        }
    }
}

fn role_database_url<F>(
    get: &F,
    role: DatabaseRole,
    production: bool,
) -> Result<String, ConfigError>
where
    F: Fn(&str) -> Option<OsString>,
{
    let url_key = role.url_key();
    let direct = optional_value(get, url_key)?;
    let components = role_components(get, role)?;
    if direct.is_some() && components.any {
        return Err(ConfigError::Ambiguous {
            key: url_key.to_owned(),
            file_key: format!("{} component settings", role.label()),
        });
    }
    if let Some(url) = direct {
        if production && get(url_key).is_some() {
            return Err(ConfigError::Invalid {
                key: format!("{url_key}_FILE (plaintext URL is forbidden in production)"),
            });
        }
        validate_database_url(&url, url_key)?;
        return Ok(url);
    }
    if !components.any {
        return Err(ConfigError::Missing {
            key: format!(
                "{url_key} or {}_* database settings",
                role.label().to_uppercase()
            ),
        });
    }

    let host = component_or_shared(get, role.host_key(), "DB_HOST", role)?;
    let port = component_or_shared(get, role.port_key(), "DB_PORT", role)?;
    let name = component_or_shared(get, role.name_key(), "DB_NAME", role)?;
    let user =
        optional_text(get, role.user_key())?.unwrap_or_else(|| role.default_user().to_owned());
    let password = password_value(get, role, production)?;
    let port = port
        .parse::<u16>()
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| invalid(role.port_key()))?;
    let url = component_url(&host, port, &name, &user, &password, url_key)?;
    validate_database_url(&url, url_key)?;
    Ok(url)
}

struct RoleComponents {
    any: bool,
}

fn role_components<F>(get: &F, role: DatabaseRole) -> Result<RoleComponents, ConfigError>
where
    F: Fn(&str) -> Option<OsString>,
{
    let mut any = false;
    for key in [
        role.host_key(),
        role.port_key(),
        role.name_key(),
        role.user_key(),
        role.password_key(),
    ] {
        if optional_value(get, key)?.is_some() {
            any = true;
        }
    }
    if let Some(key) = role.legacy_password_key()
        && optional_value(get, key)?.is_some()
    {
        any = true;
    }
    if let Some(key) = role.alternate_password_key()
        && optional_value(get, key)?.is_some()
    {
        any = true;
    }
    Ok(RoleComponents { any })
}

fn component_or_shared<F>(
    get: &F,
    role_key: &str,
    shared_key: &str,
    role: DatabaseRole,
) -> Result<String, ConfigError>
where
    F: Fn(&str) -> Option<OsString>,
{
    optional_text(get, role_key)?
        .or(optional_text(get, shared_key)?)
        .ok_or_else(|| ConfigError::Missing {
            key: format!("{role_key} or {shared_key} for {} database", role.label()),
        })
}

fn password_value<F>(get: &F, role: DatabaseRole, production: bool) -> Result<String, ConfigError>
where
    F: Fn(&str) -> Option<OsString>,
{
    let primary = optional_value(get, role.password_key())?;
    let legacy = match role.legacy_password_key() {
        Some(key) => optional_value(get, key)?,
        None => None,
    };
    let alternate = match role.alternate_password_key() {
        Some(key) => optional_value(get, key)?,
        None => None,
    };
    if [primary.as_ref(), legacy.as_ref(), alternate.as_ref()]
        .into_iter()
        .flatten()
        .count()
        > 1
    {
        return Err(ConfigError::Ambiguous {
            key: role.password_key().to_owned(),
            file_key: "multiple password settings".to_owned(),
        });
    }
    let primary_plaintext = get(role.password_key()).is_some();
    let legacy_plaintext = role
        .legacy_password_key()
        .is_some_and(|key| get(key).is_some());
    let alternate_plaintext = role
        .alternate_password_key()
        .is_some_and(|key| get(key).is_some());
    if production && (primary_plaintext || legacy_plaintext || alternate_plaintext) {
        return Err(ConfigError::Invalid {
            key: format!(
                "{}_FILE (plaintext password is forbidden in production)",
                role.password_key()
            ),
        });
    }
    primary
        .or(legacy)
        .or(alternate)
        .ok_or_else(|| ConfigError::Missing {
            key: format!(
                "{} or {}_FILE for {} database",
                role.password_key(),
                role.password_key(),
                role.label()
            ),
        })
}

fn component_url(
    host: &str,
    port: u16,
    database: &str,
    username: &str,
    password: &str,
    key: &str,
) -> Result<String, ConfigError> {
    if host.trim().is_empty()
        || database.trim().is_empty()
        || username.trim().is_empty()
        || host.chars().any(char::is_whitespace)
        || host.contains('/')
    {
        return Err(invalid(key));
    }
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    Ok(format!(
        "postgres://{}:{}@{}:{}/{}",
        percent_encode(username),
        percent_encode(password),
        host,
        port,
        percent_encode(database),
    ))
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push(char::from(b"0123456789ABCDEF"[(byte >> 4) as usize]));
            encoded.push(char::from(b"0123456789ABCDEF"[(byte & 0x0f) as usize]));
        }
    }
    encoded
}

fn validate_database_url(url: &str, key: &str) -> Result<(), ConfigError> {
    let options = url
        .parse::<sqlx::postgres::PgConnectOptions>()
        .map_err(|_| invalid(key))?;
    // A URL with no database name can make a role accidentally connect to the
    // server's default database.  Component mode always supplies one; keep
    // direct URL mode equally explicit.
    if options.get_database().is_none() {
        return Err(invalid(key));
    }
    Ok(())
}

fn recommendation_dataset<F>(get: &F) -> Result<DatasetPin, ConfigError>
where
    F: Fn(&str) -> Option<OsString>,
{
    let id = required_text(get, "RECOMMENDATION_DATASET_VERSION_ID")?
        .parse::<uuid::Uuid>()
        .map_err(|_| invalid("RECOMMENDATION_DATASET_VERSION_ID"))?;
    let dataset_id = required_text(get, "RECOMMENDATION_DATASET_ID")?;
    let version = required_text(get, "RECOMMENDATION_DATASET_VERSION")?;
    let curated_version = required_text(get, "RECOMMENDATION_CURATED_VERSION")?
        .parse::<u32>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| invalid("RECOMMENDATION_CURATED_VERSION"))?;
    let manifest_sha256 = required_text(get, "RECOMMENDATION_DATASET_MANIFEST_SHA256")?;
    if manifest_sha256.len() != 64
        || !manifest_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(invalid("RECOMMENDATION_DATASET_MANIFEST_SHA256"));
    }
    Ok(DatasetPin {
        id,
        dataset_id,
        version,
        curated_version,
        manifest_sha256,
    })
}

fn listen_addr_from<F>(get: &F) -> Result<SocketAddr, ConfigError>
where
    F: Fn(&str) -> Option<OsString>,
{
    if let Some(addr) = optional_text(get, "APP_LISTEN_ADDR")? {
        return addr
            .parse::<SocketAddr>()
            .map_err(|_| invalid("APP_LISTEN_ADDR"));
    }
    let host = optional_text(get, "APP_HOST")?.unwrap_or_else(|| DEFAULT_HOST.to_owned());
    let port = optional_text(get, "APP_PORT")?
        .map(|value| value.parse::<u16>().map_err(|_| invalid("APP_PORT")))
        .transpose()?
        .unwrap_or(DEFAULT_PORT);
    if port == 0 || host.trim().is_empty() {
        return Err(invalid("APP_HOST/APP_PORT"));
    }
    let ip = host.parse::<IpAddr>().map_err(|_| invalid("APP_HOST"))?;
    Ok(SocketAddr::new(ip, port))
}

fn artifact_root_from<F>(get: &F) -> Result<PathBuf, ConfigError>
where
    F: Fn(&str) -> Option<OsString>,
{
    let mut configured: Option<(String, String)> = None;
    for key in [
        "ARTIFACT_ROOT",
        "LAGRANGE_ARTIFACTS_ROOT",
        "LAGRANGE_ARTIFACT_ROOT",
    ] {
        if let Some(value) = optional_text(get, key)? {
            if let Some((previous, _)) = configured.as_ref() {
                return Err(ConfigError::Ambiguous {
                    key: previous.clone(),
                    file_key: key.to_owned(),
                });
            }
            configured = Some((key.to_owned(), value));
        }
    }
    Ok(configured
        .map(|(_, value)| PathBuf::from(value))
        .unwrap_or_else(|| PathBuf::from(DEFAULT_ARTIFACT_ROOT)))
}

fn positive_u32<F>(get: &F, key: &str, default: u32) -> Result<u32, ConfigError>
where
    F: Fn(&str) -> Option<OsString>,
{
    match optional_text(get, key)? {
        Some(value) => value
            .parse::<u32>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| invalid(key)),
        None => Ok(default),
    }
}

fn positive_i64<F>(get: &F, key: &str, default: i64) -> Result<i64, ConfigError>
where
    F: Fn(&str) -> Option<OsString>,
{
    match optional_text(get, key)? {
        Some(value) => value
            .parse::<i64>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| invalid(key)),
        None => Ok(default),
    }
}

fn positive_u64<F>(get: &F, key: &str, default: u64) -> Result<u64, ConfigError>
where
    F: Fn(&str) -> Option<OsString>,
{
    match optional_text(get, key)? {
        Some(value) => value
            .parse::<u64>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| invalid(key)),
        None => Ok(default),
    }
}

fn secret_32<F>(get: &F, key: &str, production: bool) -> Result<[u8; 32], ConfigError>
where
    F: Fn(&str) -> Option<OsString>,
{
    let value = required_value(get, key)?;
    if production && get(key).is_some() {
        return Err(ConfigError::Invalid {
            key: format!("{key}_FILE (plaintext secret is forbidden in production)"),
        });
    }
    let bytes = value.as_bytes();
    if bytes.len() == 32 {
        let mut secret = [0; 32];
        secret.copy_from_slice(bytes);
        return Ok(secret);
    }
    if bytes.len() == 64
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f' | b'A'..=b'F'))
    {
        let decoded = hex::decode(bytes).map_err(|_| invalid(key))?;
        let mut secret = [0; 32];
        secret.copy_from_slice(&decoded);
        return Ok(secret);
    }
    for engine in [
        base64::engine::general_purpose::URL_SAFE_NO_PAD,
        base64::engine::general_purpose::URL_SAFE,
        base64::engine::general_purpose::STANDARD,
    ] {
        if let Ok(decoded) = engine.decode(bytes)
            && decoded.len() == 32
        {
            let mut secret = [0; 32];
            secret.copy_from_slice(&decoded);
            return Ok(secret);
        }
    }
    Err(invalid(key))
}

fn required_text<F>(get: &F, key: &str) -> Result<String, ConfigError>
where
    F: Fn(&str) -> Option<OsString>,
{
    required_value(get, key)
}

fn optional_text<F>(get: &F, key: &str) -> Result<Option<String>, ConfigError>
where
    F: Fn(&str) -> Option<OsString>,
{
    optional_value(get, key)
}

fn required_value<F>(get: &F, key: &str) -> Result<String, ConfigError>
where
    F: Fn(&str) -> Option<OsString>,
{
    optional_value(get, key)?.ok_or_else(|| ConfigError::Missing {
        key: key.to_owned(),
    })
}

fn optional_value<F>(get: &F, key: &str) -> Result<Option<String>, ConfigError>
where
    F: Fn(&str) -> Option<OsString>,
{
    let file_key = format!("{key}_FILE");
    let direct = get(key);
    let file = get(&file_key);
    if direct.is_some() && file.is_some() {
        return Err(ConfigError::Ambiguous {
            key: key.to_owned(),
            file_key,
        });
    }
    if let Some(value) = direct {
        return text_value(value, key).map(Some);
    }
    let Some(path) = file else {
        return Ok(None);
    };
    let path = text_value(path, &file_key)?;
    read_secret_file(Path::new(&path), &file_key).map(Some)
}

fn text_value(value: OsString, key: &str) -> Result<String, ConfigError> {
    let value = value.into_string().map_err(|_| ConfigError::NonUnicode {
        key: key.to_owned(),
    })?;
    let value = value.trim().to_owned();
    if value.is_empty() {
        Err(ConfigError::Empty {
            key: key.to_owned(),
        })
    } else {
        Ok(value)
    }
}

/// Read a Docker/systemd secret file without ever returning its contents in
/// an error.  A symlink is rejected so a deployment cannot be redirected to a
/// mutable path after configuration validation.
pub fn read_secret_file(path: &Path, key: &str) -> Result<String, ConfigError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| ConfigError::Unreadable {
        key: key.to_owned(),
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ConfigError::InvalidFile {
            key: key.to_owned(),
        });
    }
    let value = std::fs::read_to_string(path).map_err(|_| ConfigError::Unreadable {
        key: key.to_owned(),
    })?;
    if value.contains('\n') || value.contains('\r') {
        return Err(ConfigError::Invalid {
            key: key.to_owned(),
        });
    }
    let value = value.trim().to_owned();
    if value.is_empty() {
        return Err(ConfigError::Empty {
            key: key.to_owned(),
        });
    }
    Ok(value)
}

fn invalid(key: &str) -> ConfigError {
    ConfigError::Invalid {
        key: key.to_owned(),
    }
}

/// Build the app/admin/audit pools and load entitlement state before serving
/// any socket.  A failure in any role is fatal; the API never starts in a
/// partially-connected mode.
pub async fn build_state(config: &RuntimeConfig) -> Result<ApiState, String> {
    validate_artifact_root(&config.artifact_root).map_err(|error| error.to_string())?;
    let app_pool = connect_pool(
        &config.database.app_url,
        config.database.app_max_connections,
        config.acquire_timeout,
        "app",
    )
    .await?;
    let admin_pool = connect_pool(
        &config.database.admin_url,
        config.database.admin_max_connections,
        config.acquire_timeout,
        "admin",
    )
    .await?;
    let audit_pool = connect_pool(
        &config.database.audit_url,
        config.database.audit_max_connections,
        config.acquire_timeout,
        "audit",
    )
    .await?;
    ApiState::from_pools(config.api_config(), app_pool, admin_pool, audit_pool)
        .await
        .map_err(|error| format!("load API state: {error}"))
}

/// Build the confidential OIDC/session authority only after all database
/// pools are ready. Missing production Auth0 settings or the mounted client
/// secret therefore prevents the process from binding its public socket.
pub fn build_auth_router_state(state: &ApiState) -> Result<AuthRouterState, String> {
    api_server_auth::production_router_state_from_env(
        state.app_pool.clone(),
        state.admin_pool.clone(),
        state.audit_pool.clone(),
        state.cfg.step_up_max_auth_age_secs,
    )
    .map_err(|error| format!("auth configuration rejected: {error}"))
}

async fn connect_pool(
    url: &str,
    max_connections: u32,
    acquire_timeout: Duration,
    label: &str,
) -> Result<sqlx::PgPool, String> {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(acquire_timeout)
        .connect(url)
        .await
        .map_err(|error| format!("connect {label} database: {error}"))
}

fn validate_artifact_root(path: &Path) -> Result<(), ConfigError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| ConfigError::Unreadable {
        key: "ARTIFACT_ROOT".to_owned(),
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ConfigError::InvalidPath {
            key: "ARTIFACT_ROOT".to_owned(),
        });
    }
    Ok(())
}

/// Assemble health routes alongside the existing versioned API router.
pub fn app_router(state: ApiState) -> Router {
    app_router_base(state)
}

fn app_router_base(state: ApiState) -> Router {
    let health = health_router(state.clone(), None);
    api_router(state).merge(health)
}

fn health_router(
    state: ApiState,
    audit: Option<Arc<api_server_auth::postgres::PostgresAuthAudit>>,
) -> Router {
    let audit_for_ready = audit.clone();
    Router::<ApiState>::new()
        .route("/healthz", get(healthz))
        .route(
            "/readyz",
            get(move |State(state): State<ApiState>| {
                let audit = audit_for_ready.clone();
                async move { readyz_with_audit(state, audit).await }
            }),
        )
        .with_state(state)
}

/// Assemble the production API plus the unversioned confidential auth
/// endpoints. The auth router owns /auth/*; the existing API router owns
/// /api/v1/auth/*, so no path or state type is duplicated.
pub fn app_router_with_auth(state: ApiState, auth_state: AuthRouterState) -> Router {
    let health = health_router(state.clone(), auth_state.durable_audit.clone());
    api_router(state.clone())
        .merge(health)
        .merge(api_server_auth::router(auth_state))
}

/// Process liveness: no database or filesystem dependency by design.
pub async fn healthz() -> Response {
    (
        StatusCode::OK,
        [("Cache-Control", "no-store")],
        Json(json!({ "status": "ok" })),
    )
        .into_response()
}

/// Process readiness: all three role-scoped database pools must respond.
pub async fn readyz(State(state): State<ApiState>) -> Response {
    readyz_with_audit(state, None).await
}

async fn readyz_with_audit(
    state: ApiState,
    audit: Option<Arc<api_server_auth::postgres::PostgresAuthAudit>>,
) -> Response {
    match state.check_readiness().await {
        Ok(()) => {
            let audit_status = match audit {
                Some(ref audit) => match audit.readiness().await {
                    Ok(status) => {
                        if !status.is_ready() {
                            return (
                                StatusCode::SERVICE_UNAVAILABLE,
                                [("Cache-Control", "no-store")],
                                Json(json!({
                                    "status": "not_ready",
                                    "audit_backlog": status.backlog,
                                    "audit_oldest_pending_age_secs": status.oldest_pending_age_secs,
                                    "audit_worker_alive": status.worker_alive,
                                    "audit_worker_stale": status.worker_stale,
                                    "audit_consecutive_failures": status.consecutive_failures,
                                    "audit_failures": status.failures,
                                    "audit_pending_sla_secs": api_server_auth::postgres::AUTH_AUDIT_PENDING_SLA_SECS
                                })),
                            )
                                .into_response();
                        }
                        status
                    }
                    Err(_) => {
                        return (
                            StatusCode::SERVICE_UNAVAILABLE,
                            [("Cache-Control", "no-store")],
                            Json(json!({ "status": "not_ready", "audit_outbox": "unavailable" })),
                        )
                            .into_response();
                    }
                },
                None => api_server_auth::postgres::AuthAuditReadiness {
                    backlog: 0,
                    oldest_pending_age_secs: 0,
                    worker_alive: true,
                    worker_stale: false,
                    consecutive_failures: 0,
                    failures: 0,
                },
            };
            (
                StatusCode::OK,
                [("Cache-Control", "no-store")],
                Json(json!({
                    "status": "ready",
                    "audit_backlog": audit_status.backlog,
                    "audit_oldest_pending_age_secs": audit_status.oldest_pending_age_secs,
                    "audit_worker_alive": audit_status.worker_alive,
                    "audit_worker_stale": audit_status.worker_stale,
                    "audit_consecutive_failures": audit_status.consecutive_failures,
                    "audit_failures": audit_status.failures,
                    "audit_pending_sla_secs": api_server_auth::postgres::AUTH_AUDIT_PENDING_SLA_SECS
                })),
            )
                .into_response()
        }
        Err(error) => {
            crate::observability::log::LogEvent::critical("api.readiness_failed")
                .message(format!("database readiness check failed: {error}"))
                .emit();
            (
                StatusCode::SERVICE_UNAVAILABLE,
                [("Cache-Control", "no-store")],
                Json(json!({ "status": "not_ready" })),
            )
                .into_response()
        }
    }
}

/// Serve until the supplied shutdown future resolves.  Keeping the future
/// injectable makes graceful-shutdown behavior testable without sending
/// process signals from a unit test.
pub async fn serve<F>(listener: TcpListener, state: ApiState, shutdown: F) -> Result<(), String>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    serve_router_with_deadline(
        listener,
        app_router(state),
        shutdown,
        GRACEFUL_SHUTDOWN_DEADLINE,
    )
    .await
}

/// Run an Axum server with a bounded post-signal drain.  The notification
/// channel is separate from the future supplied to Hyper so the deadline
/// starts when that future resolves rather than when the process starts.
async fn serve_router_with_deadline<F>(
    listener: TcpListener,
    app: Router,
    shutdown: F,
    graceful_deadline: Duration,
) -> Result<(), String>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    let (shutdown_started_tx, shutdown_started_rx) = oneshot::channel();
    let shutdown = async move {
        shutdown.await;
        let _ = shutdown_started_tx.send(());
    };
    // Axum exposes `WithGracefulShutdown` through `IntoFuture`; materialize
    // that future so it can be selected and timed out more than once below.
    let server = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .into_future();
    tokio::pin!(server);
    tokio::pin!(shutdown_started_rx);

    tokio::select! {
        result = &mut server => result.map_err(|error| format!("HTTP server failed: {error}")),
        _ = &mut shutdown_started_rx => {
            match tokio::time::timeout(graceful_deadline, &mut server).await {
                Ok(result) => result.map_err(|error| format!("HTTP server failed: {error}")),
                Err(_) => {
                    crate::observability::log::LogEvent::critical(
                        "api.graceful_shutdown_timeout",
                    )
                    .message(format!(
                        "in-flight HTTP requests exceeded {:?}; forcing server drain",
                        graceful_deadline
                    ))
                    .emit();
                    // Dropping the pinned server closes the listener and
                    // cancels Hyper's graceful driver. The outbox drain is
                    // handled by the caller independently of this branch.
                    Ok(())
                }
            }
        }
    }
}

/// Production serving entrypoint with the confidential auth authority
/// mounted before the socket is exposed.
pub async fn serve_with_auth<F>(
    listener: TcpListener,
    state: ApiState,
    auth_state: AuthRouterState,
    shutdown: F,
) -> Result<(), String>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    let app = app_router_with_auth(state, auth_state.clone());
    let audit = auth_state.durable_audit.clone();
    let audit_for_signal = audit.clone();
    let shutdown = async move {
        shutdown.await;
        // Stop admitting new durable events as soon as shutdown starts; this
        // does not block the HTTP drain and lets the writer finish committed
        // rows while requests are winding down.
        if let Some(audit) = audit_for_signal {
            audit.shutdown();
        }
    };
    let result =
        serve_router_with_deadline(listener, app, shutdown, GRACEFUL_SHUTDOWN_DEADLINE).await;
    if let Some(audit) = audit {
        // Joining a std::thread is blocking, so keep it off the async runtime.
        // The audit worker has its own SQL deadlines and reaper; this outer
        // timeout bounds the process hook even if the database is wedged.
        let drain = tokio::task::spawn_blocking(move || audit.shutdown_and_wait());
        let _ = tokio::time::timeout(AUDIT_DRAIN_DEADLINE, drain).await;
    }
    result
}

/// Wait for SIGINT/Ctrl-C or SIGTERM, whichever arrives first.
pub async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            crate::observability::log::LogEvent::critical("api.shutdown_signal_failed")
                .message(format!("failed to install Ctrl-C handler: {error}"))
                .emit();
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    {
        let terminate = async {
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(mut signal) => {
                    signal.recv().await;
                }
                Err(error) => {
                    crate::observability::log::LogEvent::critical("api.shutdown_signal_failed")
                        .message(format!("failed to install SIGTERM handler: {error}"))
                        .emit();
                    std::future::pending::<()>().await;
                }
            }
        };
        tokio::select! {
            _ = ctrl_c => {},
            _ = terminate => {},
        }
    }

    #[cfg(not(unix))]
    ctrl_c.await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    use tempfile::tempdir;

    fn base_env() -> HashMap<String, OsString> {
        let mut env = HashMap::new();
        env.insert("APP_ENV".to_owned(), "test".into());
        env.insert("APP_HOST".to_owned(), "127.0.0.1".into());
        env.insert("APP_PORT".to_owned(), "18080".into());
        env.insert(
            "DATABASE_URL".to_owned(),
            "postgres://app:secret@localhost:5432/lagrange".into(),
        );
        env.insert(
            "ADMIN_DATABASE_URL".to_owned(),
            "postgres://admin:secret@localhost:5432/lagrange".into(),
        );
        env.insert(
            "AUDIT_DATABASE_URL".to_owned(),
            "postgres://audit_writer:secret@localhost:5432/lagrange".into(),
        );
        env.insert("CURSOR_SECRET".to_owned(), "a".repeat(64).into());
        env
    }

    fn config(env: &HashMap<String, OsString>) -> Result<RuntimeConfig, ConfigError> {
        load_config_from(|key| env.get(key).cloned())
    }

    #[test]
    fn config_requires_all_three_role_urls() {
        let mut env = base_env();
        env.remove("AUDIT_DATABASE_URL");
        let error = match config(&env) {
            Ok(_) => panic!("audit URL must be required"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            ConfigError::Missing {
                key: "AUDIT_DATABASE_URL or AUDIT_* database settings".to_owned()
            }
        );
    }

    #[test]
    fn config_reads_database_urls_and_cursor_from_files() {
        let root = tempdir().expect("tempdir");
        let app = root.path().join("app-url");
        let admin = root.path().join("admin-url");
        let audit = root.path().join("audit-url");
        let cursor = root.path().join("cursor");
        std::fs::write(&app, "postgres://app:file@localhost:5432/lagrange").unwrap();
        std::fs::write(&admin, "postgres://admin:file@localhost:5432/lagrange").unwrap();
        std::fs::write(
            &audit,
            "postgres://audit_writer:file@localhost:5432/lagrange",
        )
        .unwrap();
        std::fs::write(&cursor, "b".repeat(64)).unwrap();
        let mut env = base_env();
        for key in ["DATABASE_URL", "ADMIN_DATABASE_URL", "AUDIT_DATABASE_URL"] {
            env.remove(key);
        }
        env.insert("DATABASE_URL_FILE".to_owned(), app.into_os_string());
        env.insert("ADMIN_DATABASE_URL_FILE".to_owned(), admin.into_os_string());
        env.insert("AUDIT_DATABASE_URL_FILE".to_owned(), audit.into_os_string());
        env.remove("CURSOR_SECRET");
        env.insert("CURSOR_SECRET_FILE".to_owned(), cursor.into_os_string());
        let loaded = config(&env).expect("file-backed config");
        assert!(loaded.database.app_url.contains("app:file"));
        assert_eq!(loaded.cursor_secret, [0xbb; 32]);

        env.insert("APP_ENV".to_owned(), "production".into());
        env.insert(
            "LAGRANGE_CODE_COMMIT".to_owned(),
            DEVELOPMENT_CODE_COMMIT.into(),
        );
        env.insert(
            "RECOMMENDATION_DATASET_VERSION_ID".to_owned(),
            uuid::Uuid::new_v4().to_string().into(),
        );
        env.insert(
            "RECOMMENDATION_DATASET_ID".to_owned(),
            "krx_eod_bars".into(),
        );
        env.insert(
            "RECOMMENDATION_DATASET_VERSION".to_owned(),
            "2026-01".into(),
        );
        env.insert("RECOMMENDATION_CURATED_VERSION".to_owned(), "1".into());
        env.insert(
            "RECOMMENDATION_DATASET_MANIFEST_SHA256".to_owned(),
            "c".repeat(64).into(),
        );
        config(&env).expect("production accepts file-backed secrets");
    }

    #[test]
    fn config_builds_component_urls_with_role_specific_password_files() {
        let root = tempdir().expect("tempdir");
        let app_password = root.path().join("app-password");
        let admin_password = root.path().join("admin-password");
        let audit_password = root.path().join("audit-password");
        std::fs::write(&app_password, "app pw").unwrap();
        std::fs::write(&admin_password, "admin pw").unwrap();
        std::fs::write(&audit_password, "audit pw").unwrap();
        let mut env = base_env();
        for key in ["DATABASE_URL", "ADMIN_DATABASE_URL", "AUDIT_DATABASE_URL"] {
            env.remove(key);
        }
        env.insert("DB_HOST".to_owned(), "localhost".into());
        env.insert("DB_PORT".to_owned(), "5432".into());
        env.insert("DB_NAME".to_owned(), "lagrange".into());
        env.insert("DB_USER".to_owned(), "app".into());
        env.insert("DB_PASSWORD_FILE".to_owned(), app_password.into_os_string());
        env.insert("ADMIN_DB_USER".to_owned(), "admin".into());
        env.insert(
            "ADMIN_DB_PASSWORD_FILE".to_owned(),
            admin_password.into_os_string(),
        );
        env.insert("AUDIT_DB_USER".to_owned(), "audit_writer".into());
        env.insert(
            "AUDIT_DB_PASSWORD_FILE".to_owned(),
            audit_password.into_os_string(),
        );
        let loaded = config(&env).expect("component config");
        assert!(loaded.database.app_url.contains("app%20pw"));
        assert!(loaded.database.admin_url.contains("admin%20pw"));
        assert!(loaded.database.audit_url.contains("audit%20pw"));
        let app_url = role_database_url(&|key| env.get(key).cloned(), DatabaseRole::App, true)
            .expect("production component password file");
        assert!(app_url.contains("app%20pw"));
    }

    #[test]
    fn config_rejects_direct_and_file_values_together() {
        let root = tempdir().expect("tempdir");
        let path = root.path().join("cursor");
        std::fs::write(&path, "c".repeat(64)).unwrap();
        let mut env = base_env();
        env.insert("CURSOR_SECRET_FILE".to_owned(), path.into_os_string());
        let error = match config(&env) {
            Ok(_) => panic!("ambiguous cursor values"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            ConfigError::Ambiguous {
                key: "CURSOR_SECRET".to_owned(),
                file_key: "CURSOR_SECRET_FILE".to_owned()
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn config_rejects_secret_symlink_and_empty_file() {
        let root = tempdir().expect("tempdir");
        let target = root.path().join("target");
        let link = root.path().join("link");
        std::fs::write(&target, "d".repeat(64)).unwrap();
        symlink(&target, &link).unwrap();
        let mut env = base_env();
        env.remove("CURSOR_SECRET");
        env.insert("CURSOR_SECRET_FILE".to_owned(), link.into_os_string());
        assert!(matches!(
            config(&env),
            Err(ConfigError::InvalidFile { key }) if key == "CURSOR_SECRET_FILE"
        ));

        let empty = root.path().join("empty");
        std::fs::write(&empty, "").unwrap();
        env.insert("CURSOR_SECRET_FILE".to_owned(), empty.into_os_string());
        assert!(matches!(
            config(&env),
            Err(ConfigError::Empty { key }) if key == "CURSOR_SECRET_FILE"
        ));
    }

    #[test]
    fn config_rejects_any_secret_file_line_ending_before_trimming() {
        let root = tempdir().expect("tempdir");
        let mut env = base_env();
        env.remove("CURSOR_SECRET");

        for (suffix, contents) in [
            ("lf", "secret\n"),
            ("cr", "secret\r"),
            ("crlf", "secret\r\n"),
        ] {
            let path = root.path().join(format!("cursor-{suffix}"));
            std::fs::write(&path, contents).unwrap();
            env.insert("CURSOR_SECRET_FILE".to_owned(), path.into_os_string());
            assert!(
                matches!(
                    config(&env),
                    Err(ConfigError::Invalid { key }) if key == "CURSOR_SECRET_FILE"
                ),
                "secret file with {suffix} must fail before trimming"
            );
        }
    }

    #[test]
    fn config_defaults_to_production_and_requires_dataset_pin() {
        let mut env = base_env();
        env.remove("APP_ENV");
        let error = match config(&env) {
            Ok(_) => panic!("production requires dataset pin"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            ConfigError::Missing { key } if key == "RECOMMENDATION_DATASET_VERSION_ID"
        ));
    }

    #[test]
    fn config_accepts_development_without_dataset_pin_but_keeps_db_requirements() {
        let env = base_env();
        let loaded = config(&env).expect("test environment config");
        assert_eq!(loaded.recommendation_dataset.id, uuid::Uuid::nil());
        assert_eq!(loaded.code_commit, DEVELOPMENT_CODE_COMMIT);
        assert_eq!(loaded.listen_addr, "127.0.0.1:18080".parse().unwrap());
    }

    #[test]
    fn production_code_commit_is_exact_lowercase_nonzero_40_hex() {
        let mut env = base_env();
        env.insert("APP_ENV".to_owned(), "production".into());
        for value in [
            "0123456789abcdef0123456789abcdef01234567",
            "0123456789ABCDEF0123456789abcdef01234567",
            "0123456789abcdef0123456789abcdef0123456",
            "0000000000000000000000000000000000000000",
            "not-a-commit",
        ] {
            env.insert("LAGRANGE_CODE_COMMIT".to_owned(), value.into());
            let result = code_commit_from(&|key| env.get(key).cloned(), true);
            if value == DEVELOPMENT_CODE_COMMIT {
                assert!(result.is_ok());
            } else {
                assert!(
                    matches!(result, Err(ConfigError::Invalid { ref key }) if key == "LAGRANGE_CODE_COMMIT")
                );
            }
        }

        env.remove("LAGRANGE_CODE_COMMIT");
        assert!(matches!(
            code_commit_from(&|key| env.get(key).cloned(), true),
            Err(ConfigError::Missing { ref key }) if key == "LAGRANGE_CODE_COMMIT"
        ));
    }

    #[tokio::test]
    async fn healthz_is_liveness_only() {
        let response = healthz().await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 64)
            .await
            .expect("health body");
        assert_eq!(body.as_ref(), br#"{"status":"ok"}"#);
    }

    #[tokio::test]
    async fn graceful_shutdown_forces_a_stuck_request_after_deadline() {
        use axum::routing::get;
        use std::io::Write;
        use std::sync::Arc;
        use tokio::sync::{Notify, oneshot};

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let address = listener.local_addr().expect("test listener address");
        let request_started = Arc::new(Notify::new());
        let handler_started = Arc::clone(&request_started);
        let app = Router::new().route(
            "/hang",
            get(move || {
                let handler_started = Arc::clone(&handler_started);
                async move {
                    handler_started.notify_one();
                    std::future::pending::<()>().await;
                }
            }),
        );
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let server = tokio::spawn(serve_router_with_deadline(
            listener,
            app,
            async move {
                let _ = shutdown_rx.await;
            },
            Duration::from_millis(50),
        ));

        // A small synchronous write is deterministic here: the notify barrier
        // below proves Hyper accepted and dispatched the request before the
        // shutdown signal is sent.
        let mut socket = std::net::TcpStream::connect(address).expect("connect test request");
        socket
            .write_all(b"GET /hang HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .expect("write test request");
        tokio::time::timeout(Duration::from_secs(1), request_started.notified())
            .await
            .expect("request dispatched");
        shutdown_tx.send(()).expect("send shutdown");

        let result = tokio::time::timeout(Duration::from_secs(1), server)
            .await
            .expect("forced shutdown completed")
            .expect("server task joined");
        assert_eq!(result, Ok(()));
    }
}
