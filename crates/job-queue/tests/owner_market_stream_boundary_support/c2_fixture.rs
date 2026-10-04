//! Test-only binding for the one prepared C2 PostgreSQL cluster.
//!
//! The legacy WS-3A identity guard remains in the parent module. This module
//! accepts the current C2 endpoint only after validating its private binding
//! file and the exact SQLx URL that will be used for the supervisor.

use std::error::Error;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use uuid::Uuid;

pub(crate) type FixtureError = Box<dyn Error + Send + Sync>;

const BINDING_ENV: &str = "LAGRANGE_KIS_C2_BINDING_FILE";
const BINDING_SHA_ENV: &str = "LAGRANGE_KIS_C2_BINDING_SHA256";
const EXPECTED_RUN_ID: &str = "4b4fed71-6608-4d97-a6b6-7ae3ba294be0";
const EXPECTED_BINDING_PATH: &str =
    "/tmp/lagrange-kis-stream-completion-20261001-653ff5d84407/wp3c2-runtime-binding.json";
const EXPECTED_BINDING_SHA256: &str =
    "f173e4eed80a6543535f574fa003d6d77d521bfb41d6c532f6c44442a2f4f268";
const EXPECTED_RUNTIME_ROOT: &str = "/tmp/lagrange-kis-c2-4b4fed7166084d97a6b67ae3ba294be0";
const EXPECTED_SYSTEM_IDENTIFIER: &str = "7691641528624668685";
const LEGACY_SYSTEM_IDENTIFIER: &str = "7687959257259950090";
const EXPECTED_PORT: u16 = 55_471;
const EXPECTED_SERVER_VERSION_NUM: i32 = 180_006;
const EXPECTED_SUPERVISOR_USER: &str = "c2_supervisor";
const EXPECTED_SUPERVISOR_DATABASE: &str = "postgres";
const MAX_BINDING_BYTES: u64 = 4096;
const IDENTITY_QUERY_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(deny_unknown_fields)]
struct BindingDocument {
    schema: u32,
    run_id: String,
    owner_uid: u32,
    owner_gid: u32,
    data_directory: String,
    socket_directory: String,
    port: u16,
    system_identifier: String,
    server_version_num: i32,
    supervisor_user: String,
    supervisor_database: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct C2Endpoint {
    socket_directory: PathBuf,
    port: u16,
}

impl C2Endpoint {
    pub(crate) fn socket_directory(&self) -> &Path {
        &self.socket_directory
    }

    pub(crate) fn port(&self) -> u16 {
        self.port
    }
}

#[derive(Clone, Debug)]
struct ValidatedBinding {
    run_id: Uuid,
    owner_uid: u32,
    owner_gid: u32,
    runtime_root: PathBuf,
    data_directory: PathBuf,
    endpoint: C2Endpoint,
    system_identifier: String,
    server_version_num: i32,
    supervisor_user: String,
    supervisor_database: String,
}

#[derive(Clone)]
pub(crate) struct SupervisorTarget {
    options: PgConnectOptions,
    binding: Option<ValidatedBinding>,
}

impl SupervisorTarget {
    pub(super) fn from_environment() -> Result<Self, FixtureError> {
        Self::from_environment_with_inputs(
            std::env::var_os(BINDING_ENV),
            std::env::var_os(BINDING_SHA_ENV),
        )
    }

    fn from_environment_with_inputs(
        binding_path: Option<OsString>,
        binding_sha: Option<OsString>,
    ) -> Result<Self, FixtureError> {
        let binding = binding_from_inputs(binding_path, binding_sha)?;
        let supervisor_url = std::env::var(super::SUPERVISOR_ENV)
            .map_err(|_| closed_error("the explicit PostgreSQL supervisor URL is missing"))?;

        match binding {
            Some(binding) => {
                if has_foreign_pg_settings(std::env::vars_os().map(|(key, _)| key)) {
                    return Err(closed_error(
                        "C2 connection inherited unsupported PostgreSQL environment settings",
                    ));
                }
                let expected_url = canonical_supervisor_url(&binding.endpoint);
                if supervisor_url != expected_url {
                    return Err(closed_error(
                        "the C2 supervisor URL does not match the validated Unix endpoint",
                    ));
                }
                let options = parse_c2_supervisor_url(&supervisor_url, &binding.endpoint)?;
                Ok(Self {
                    options,
                    binding: Some(binding),
                })
            }
            None => {
                // Preserve the legacy URL parsing path when both C2 binding
                // variables are absent. The parent still applies the exact
                // legacy cluster identity checks before DDL.
                let options = PgConnectOptions::from_str(&supervisor_url)?;
                Ok(Self {
                    options,
                    binding: None,
                })
            }
        }
    }

    pub(super) fn options(&self) -> PgConnectOptions {
        self.options.clone()
    }

    pub(super) fn c2_endpoint(&self) -> Option<C2Endpoint> {
        self.binding
            .as_ref()
            .map(|binding| binding.endpoint.clone())
    }

    pub(super) fn is_c2(&self) -> bool {
        self.binding.is_some()
    }

    pub(super) async fn verify_before_ddl(&self, pool: &PgPool) -> Result<(), FixtureError> {
        let Some(binding) = &self.binding else {
            return Ok(());
        };
        let observed = tokio::time::timeout(
            IDENTITY_QUERY_TIMEOUT,
            sqlx::query_as::<
                _,
                (
                    String,
                    String,
                    String,
                    String,
                    String,
                    Option<String>,
                    i32,
                    String,
                    String,
                ),
            >(
                "SELECT (SELECT system_identifier::text
                          FROM pg_catalog.pg_control_system()),
                        pg_catalog.current_setting('data_directory'),
                        pg_catalog.current_setting('listen_addresses'),
                        pg_catalog.current_setting('unix_socket_directories'),
                        pg_catalog.current_setting('port'),
                        pg_catalog.inet_server_addr()::text,
                        pg_catalog.current_setting('server_version_num')::integer,
                        current_user,
                        current_database()",
            )
            .fetch_one(pool),
        )
        .await
        .map_err(|_| closed_error("C2 PostgreSQL identity query exceeded two seconds"))??;

        let (
            system_identifier,
            data_directory,
            listen_addresses,
            socket_directories,
            port,
            server_address,
            server_version_num,
            current_user,
            current_database,
        ) = observed;
        if system_identifier != binding.system_identifier
            || data_directory != binding.data_directory.to_string_lossy()
            || !listen_addresses.is_empty()
            || socket_directories != binding.endpoint.socket_directory.to_string_lossy()
            || port != binding.endpoint.port.to_string()
            || server_address.is_some()
            || server_version_num != binding.server_version_num
            || current_user != binding.supervisor_user
            || current_database != binding.supervisor_database
            || binding.owner_uid != unsafe { libc::geteuid() }
            || binding.owner_gid != unsafe { libc::getegid() }
            || binding.runtime_root.as_os_str().is_empty()
            || binding.run_id.to_string() != EXPECTED_RUN_ID
        {
            return Err(closed_error(
                "C2 PostgreSQL identity does not match the validated binding",
            ));
        }
        Ok(())
    }
}

pub(super) async fn connect_supervisor(target: &SupervisorTarget) -> Result<PgPool, FixtureError> {
    let pool = tokio::time::timeout(
        Duration::from_secs(10),
        PgPoolOptions::new()
            .max_connections(16)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(target.options()),
    )
    .await
    .map_err(|_| closed_error("PostgreSQL supervisor connection exceeded ten seconds"))??;
    if let Err(error) = target.verify_before_ddl(&pool).await {
        pool.close().await;
        return Err(error);
    }
    Ok(pool)
}

fn binding_from_inputs(
    path: Option<OsString>,
    digest: Option<OsString>,
) -> Result<Option<ValidatedBinding>, FixtureError> {
    match (path, digest) {
        (None, None) => Ok(None),
        (Some(path), Some(digest)) => {
            let path = PathBuf::from(path);
            let digest = digest
                .into_string()
                .map_err(|_| closed_error("C2 binding digest is not valid UTF-8"))?;
            if path != Path::new(EXPECTED_BINDING_PATH) || digest != EXPECTED_BINDING_SHA256 {
                return Err(closed_error(
                    "C2 binding input does not match the prepared run",
                ));
            }
            Ok(Some(read_binding_file(&path, &digest)?))
        }
        _ => Err(closed_error(
            "C2 binding path and SHA-256 must be supplied together",
        )),
    }
}

fn read_binding_file(path: &Path, expected_sha256: &str) -> Result<ValidatedBinding, FixtureError> {
    validate_binding(read_binding_document(path, expected_sha256)?)
}

fn read_binding_document(
    path: &Path,
    expected_sha256: &str,
) -> Result<BindingDocument, FixtureError> {
    if !is_lower_hex_digest(expected_sha256) || path.file_name().is_none() {
        return Err(closed_error("C2 binding path or digest is malformed"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| closed_error("C2 binding has no containing directory"))?;
    validate_private_directory(parent, unsafe { libc::geteuid() }, unsafe {
        libc::getegid()
    })?;
    if path.file_name().and_then(|name| name.to_str()) != Some("wp3c2-runtime-binding.json")
        || fs::canonicalize(parent)
            .map_err(|_| closed_error("C2 binding directory is unavailable"))?
            != parent
    {
        return Err(closed_error("C2 binding path is not canonical"));
    }

    let path_before =
        fs::symlink_metadata(path).map_err(|_| closed_error("C2 binding file is unavailable"))?;
    validate_binding_file_metadata(&path_before)?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| closed_error("C2 binding file could not be opened safely"))?;
    let fd_before = file
        .metadata()
        .map_err(|_| closed_error("C2 binding descriptor metadata is unavailable"))?;
    validate_binding_file_metadata(&fd_before)?;
    if !same_file_identity(&path_before, &fd_before) || fd_before.len() > MAX_BINDING_BYTES {
        return Err(closed_error("C2 binding file identity or size is invalid"));
    }
    let mut bytes = Vec::with_capacity(fd_before.len() as usize);
    (&mut file)
        .take(MAX_BINDING_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| closed_error("C2 binding file could not be read"))?;
    if bytes.len() as u64 > MAX_BINDING_BYTES {
        return Err(closed_error("C2 binding file exceeds its size bound"));
    }
    let fd_after = file
        .metadata()
        .map_err(|_| closed_error("C2 binding descriptor metadata is unavailable"))?;
    let path_after = fs::symlink_metadata(path)
        .map_err(|_| closed_error("C2 binding file changed during read"))?;
    if !same_file_identity(&fd_before, &fd_after)
        || !same_file_identity(&fd_after, &path_after)
        || Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
            != expected_sha256
    {
        return Err(closed_error(
            "C2 binding changed or failed its SHA-256 check",
        ));
    }
    let document: BindingDocument = serde_json::from_slice(&bytes)
        .map_err(|_| closed_error("C2 binding JSON is invalid or has unknown fields"))?;
    Ok(document)
}

fn validate_binding(document: BindingDocument) -> Result<ValidatedBinding, FixtureError> {
    validate_binding_at(document, PathBuf::from(EXPECTED_RUNTIME_ROOT), true)
}

fn validate_binding_at(
    document: BindingDocument,
    expected_root: PathBuf,
    require_prepared_root: bool,
) -> Result<ValidatedBinding, FixtureError> {
    if document.schema != 1
        || document.run_id != EXPECTED_RUN_ID
        || document.owner_uid != unsafe { libc::geteuid() }
        || document.owner_gid != unsafe { libc::getegid() }
        || document.port != EXPECTED_PORT
        || document.system_identifier != EXPECTED_SYSTEM_IDENTIFIER
        || document.system_identifier == LEGACY_SYSTEM_IDENTIFIER
        || document.server_version_num != EXPECTED_SERVER_VERSION_NUM
        || document.supervisor_user != EXPECTED_SUPERVISOR_USER
        || document.supervisor_database != EXPECTED_SUPERVISOR_DATABASE
    {
        return Err(closed_error(
            "C2 binding values do not match the prepared identity",
        ));
    }
    let run_id = Uuid::parse_str(&document.run_id)
        .map_err(|_| closed_error("C2 binding run ID is invalid"))?;
    if run_id.get_version_num() != 4 || run_id.to_string() != document.run_id {
        return Err(closed_error("C2 binding run ID is not canonical UUIDv4"));
    }
    let run_root = PathBuf::from(format!("/tmp/lagrange-kis-c2-{}", run_id.simple()));
    if require_prepared_root && expected_root != run_root {
        return Err(closed_error("C2 binding run root is not the prepared root"));
    }
    let expected_data = expected_root.join("data");
    let expected_socket = expected_root.join("socket");
    if document.data_directory != expected_data.to_string_lossy()
        || document.socket_directory != expected_socket.to_string_lossy()
        || (require_prepared_root && expected_root.to_string_lossy() != EXPECTED_RUNTIME_ROOT)
        || expected_socket.as_os_str().as_encoded_bytes().len() + 16 >= 108
    {
        return Err(closed_error(
            "C2 binding paths are not the canonical prepared paths",
        ));
    }
    for directory in [&expected_root, &expected_data, &expected_socket] {
        validate_private_directory(directory, document.owner_uid, document.owner_gid)?;
    }
    Ok(ValidatedBinding {
        run_id,
        owner_uid: document.owner_uid,
        owner_gid: document.owner_gid,
        runtime_root: expected_root,
        data_directory: expected_data,
        endpoint: C2Endpoint {
            socket_directory: expected_socket,
            port: document.port,
        },
        system_identifier: document.system_identifier,
        server_version_num: document.server_version_num,
        supervisor_user: document.supervisor_user,
        supervisor_database: document.supervisor_database,
    })
}

fn parse_c2_supervisor_url(
    url: &str,
    endpoint: &C2Endpoint,
) -> Result<PgConnectOptions, FixtureError> {
    if has_foreign_pg_settings(std::env::vars_os().map(|(key, _)| key))
        || url != canonical_supervisor_url(endpoint)
    {
        return Err(closed_error(
            "C2 supervisor URL contains unsupported options",
        ));
    }
    let options = PgConnectOptions::from_str(url)
        .map_err(|_| closed_error("C2 supervisor URL was rejected by SQLx"))?
        // PgConnectOptions applies .pgpass while parsing. The private C2 URL
        // intentionally has no password/service inputs, so clear that
        // implicit lookup before the options can be used.
        .password("")
        .options([("statement_timeout", "15s"), ("lock_timeout", "5s")]);
    if options.get_socket() != Some(&endpoint.socket_directory)
        || options.get_port() != endpoint.port
        || options.get_username() != EXPECTED_SUPERVISOR_USER
        || options.get_database() != Some(EXPECTED_SUPERVISOR_DATABASE)
        || !matches!(options.get_ssl_mode(), PgSslMode::Disable)
    {
        return Err(closed_error(
            "SQLx normalized the C2 URL to a different endpoint",
        ));
    }
    Ok(options)
}

fn canonical_supervisor_url(endpoint: &C2Endpoint) -> String {
    format!(
        "postgresql:///postgres?host={}&port={}&user=c2_supervisor&sslmode=disable",
        endpoint.socket_directory.display(),
        endpoint.port
    )
}

fn validate_private_directory(path: &Path, uid: u32, gid: u32) -> Result<(), FixtureError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| closed_error("C2 private directory is unavailable"))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || metadata.uid() != uid
        || metadata.gid() != gid
        || metadata.mode() & 0o7777 != 0o700
        || fs::canonicalize(path).map_err(|_| closed_error("C2 directory path is invalid"))? != path
    {
        return Err(closed_error(
            "C2 directory ownership, mode, or path is invalid",
        ));
    }
    Ok(())
}

fn validate_binding_file_metadata(metadata: &fs::Metadata) -> Result<(), FixtureError> {
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.mode() & 0o7777 != 0o600
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.gid() != unsafe { libc::getegid() }
        || metadata.nlink() != 1
        || metadata.len() == 0
        || metadata.len() > MAX_BINDING_BYTES
    {
        return Err(closed_error(
            "C2 binding file ownership, mode, or type is invalid",
        ));
    }
    Ok(())
}

fn same_file_identity(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.dev() == right.dev()
        && left.ino() == right.ino()
        && left.mode() == right.mode()
        && left.uid() == right.uid()
        && left.gid() == right.gid()
        && left.nlink() == right.nlink()
        && left.len() == right.len()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
        && left.ctime() == right.ctime()
        && left.ctime_nsec() == right.ctime_nsec()
}

fn is_lower_hex_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn closed_error(message: &'static str) -> FixtureError {
    Box::new(std::io::Error::other(message))
}

pub(crate) fn relay_endpoint_from_environment() -> Result<Option<C2Endpoint>, FixtureError> {
    let binding_path = std::env::var_os(BINDING_ENV);
    let binding_sha = std::env::var_os(BINDING_SHA_ENV);
    match (binding_path, binding_sha) {
        (None, None) => Ok(None),
        (Some(path), Some(sha)) => {
            let target = SupervisorTarget::from_environment_with_inputs(Some(path), Some(sha))?;
            Ok(target.c2_endpoint())
        }
        _ => Err(closed_error(
            "C2 binding path and SHA-256 must be supplied together",
        )),
    }
}

fn has_foreign_pg_settings<I>(keys: I) -> bool
where
    I: IntoIterator<Item = OsString>,
{
    keys.into_iter()
        .any(|key| key.to_string_lossy().starts_with("PG"))
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn synthetic_binding(root: &Path) -> BindingDocument {
        let run_id = Uuid::parse_str(EXPECTED_RUN_ID).expect("canonical run ID");
        BindingDocument {
            schema: 1,
            run_id: run_id.to_string(),
            owner_uid: unsafe { libc::geteuid() },
            owner_gid: unsafe { libc::getegid() },
            data_directory: root.join("data").to_string_lossy().into_owned(),
            socket_directory: root.join("socket").to_string_lossy().into_owned(),
            port: EXPECTED_PORT,
            system_identifier: EXPECTED_SYSTEM_IDENTIFIER.to_owned(),
            server_version_num: EXPECTED_SERVER_VERSION_NUM,
            supervisor_user: EXPECTED_SUPERVISOR_USER.to_owned(),
            supervisor_database: EXPECTED_SUPERVISOR_DATABASE.to_owned(),
        }
    }

    fn serialize_document(document: &BindingDocument) -> Vec<u8> {
        serde_json::to_vec(document).expect("synthetic binding serializes")
    }

    fn temp_binding_dir() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let private = tempfile::tempdir().expect("temporary private directory");
        fs::set_permissions(private.path(), fs::Permissions::from_mode(0o700))
            .expect("private mode");
        let root = private.path().join("runtime");
        fs::create_dir(&root).expect("runtime directory");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("runtime mode");
        for child in ["data", "socket"] {
            let path = root.join(child);
            fs::create_dir(&path).expect("runtime child");
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).expect("child mode");
        }
        (private, root.clone(), root)
    }

    #[test]
    fn c2_binding_requires_both_explicit_inputs_and_rejects_legacy_or_other_runs() {
        assert!(
            binding_from_inputs(None, None)
                .expect("legacy selection")
                .is_none()
        );
        assert!(binding_from_inputs(Some("binding".into()), None).is_err());
        assert!(binding_from_inputs(None, Some("a".repeat(64).into())).is_err());
        assert!(
            binding_from_inputs(
                Some(EXPECTED_BINDING_PATH.into()),
                Some("A".repeat(64).into())
            )
            .is_err()
        );
        assert!(has_foreign_pg_settings([OsString::from("PGPASSWORD")]));
        assert!(!has_foreign_pg_settings([
            OsString::from("LANG"),
            OsString::from("PATH")
        ]));
    }

    #[test]
    fn c2_binding_parser_rejects_unknown_fields_wrong_identity_and_bad_paths() {
        let (_private, root, _) = temp_binding_dir();
        let valid = synthetic_binding(&root);
        assert!(validate_binding_at(valid, root.clone(), false).is_ok());

        let mut legacy = synthetic_binding(&root);
        legacy.system_identifier = LEGACY_SYSTEM_IDENTIFIER.to_owned();
        assert!(validate_binding_at(legacy, root.clone(), false).is_err());

        let mut mismatched_path = synthetic_binding(&root);
        mismatched_path.socket_directory = "/tmp/other/socket".to_owned();
        assert!(validate_binding_at(mismatched_path, root.clone(), false).is_err());

        let mut unknown: serde_json::Value =
            serde_json::from_slice(&serialize_document(&synthetic_binding(&root)))
                .expect("valid JSON");
        unknown["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<BindingDocument>(unknown).is_err());
    }

    #[test]
    fn c2_binding_file_checks_private_owner_mode_link_count_and_digest() {
        let (private, root, binding_root) = temp_binding_dir();
        let path = private.path().join("wp3c2-runtime-binding.json");
        let bytes = serialize_document(&synthetic_binding(&root));
        fs::write(&path, &bytes).expect("write synthetic binding");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("binding mode");
        let digest = format!("{:x}", Sha256::digest(&bytes));
        assert!(read_binding_file_for_test(&path, &digest, &binding_root).is_ok());
        assert!(read_binding_file_for_test(&path, &"0".repeat(64), &binding_root).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).expect("loosen mode");
        assert!(read_binding_file_for_test(&path, &digest, &binding_root).is_err());
    }

    fn read_binding_file_for_test(
        path: &Path,
        digest: &str,
        private_parent: &Path,
    ) -> Result<ValidatedBinding, FixtureError> {
        validate_private_directory(private_parent, unsafe { libc::geteuid() }, unsafe {
            libc::getegid()
        })?;
        let document = read_binding_document(path, digest)?;
        validate_binding_at(document, private_parent.to_path_buf(), false)
    }

    #[test]
    fn canonical_c2_url_is_parsed_by_sqlx_as_the_bound_unix_socket() {
        let endpoint = C2Endpoint {
            socket_directory: PathBuf::from(EXPECTED_RUNTIME_ROOT).join("socket"),
            port: EXPECTED_PORT,
        };
        let url = canonical_supervisor_url(&endpoint);
        let options = parse_c2_supervisor_url(&url, &endpoint).expect("SQLx endpoint parse");
        // SQLx can retain its parsed default host while the Unix socket is authoritative.
        assert_eq!(options.get_socket(), Some(&endpoint.socket_directory));
        assert_eq!(options.get_port(), EXPECTED_PORT);
        assert_eq!(options.get_username(), EXPECTED_SUPERVISOR_USER);
        assert_eq!(options.get_database(), Some(EXPECTED_SUPERVISOR_DATABASE));
        assert!(matches!(options.get_ssl_mode(), PgSslMode::Disable));
        for unsupported in [
            url.replace("user=c2_supervisor", "user=postgres"),
            url.replace("sslmode=disable", "sslmode=prefer"),
            url.replace("postgresql:///", "postgresql://127.0.0.1/"),
            format!("{url}&password=not-allowed"),
            format!("{url}&service=foreign"),
        ] {
            assert!(parse_c2_supervisor_url(&unsupported, &endpoint).is_err());
        }
    }

    #[test]
    fn c2_binding_parser_rejects_symlink_input() {
        let (private, root, _) = temp_binding_dir();
        let real = private.path().join("real-binding.json");
        fs::write(&real, serialize_document(&synthetic_binding(&root))).expect("synthetic binding");
        fs::set_permissions(&real, fs::Permissions::from_mode(0o600)).expect("mode");
        let link = private.path().join("wp3c2-runtime-binding.json");
        std::os::unix::fs::symlink(&real, &link).expect("symlink");
        let digest = "0".repeat(64);
        assert!(read_binding_file(&link, &digest).is_err());
    }
}
