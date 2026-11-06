//! Read-only process binding for the Owner-beta Paper runtime.
//!
//! A decoded binding is only a selector. `TrustedPaperRuntime` is constructed
//! only after the fixed root-protected config and the executable running in
//! this process have both been opened, checked, and hashed. This module does
//! not grant an economic action permit; callers still need the protected
//! current-binding reader and the existing fresh source/SQL checks.

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt;
use uuid::Uuid;

use super::source_evidence as wire;

const CONFIG_SCHEMA: &str = "owner-beta-paper-process-v1";
const CONSENT_VERSION: &str = "owner-beta-paper-v1-draft-1";
const FINANCIAL_CONTRACT_SHA256: &str =
    "sha256:18cde9fa7f15e19c9ec272abb780d06ae4be672068ed3f9208c39dc2297ab8d0";
const CONFIG_LIMIT: usize = 65_536;
const MAX_EXECUTABLE_BYTES: usize = 256 * 1024 * 1024;

const CONFIG_FIELDS: [&str; 12] = [
    "schema",
    "owner_user_id",
    "account_id",
    "strategy_config_id",
    "consent_event_id",
    "consent_version",
    "control_epoch",
    "daily_cohort_sha256",
    "financial_contract_sha256",
    "release_manifest_sha256",
    "api_binary_sha256",
    "worker_binary_sha256",
    // The revision is kept separate from the repeated field array below to
    // make accidental omission from the closed schema harder to overlook.
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaperProcessRole {
    Api,
    Worker,
}

/// Static, value-free denial codes for the runtime binding.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PaperRuntimeError {
    Unbound,
    Unsupported,
    InvalidConfig,
    UnsafePath,
    Changed,
    ExecutableMismatch,
    CurrentBindingMismatch,
    Io,
}

impl fmt::Debug for PaperRuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl fmt::Display for PaperRuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for PaperRuntimeError {}

impl PaperRuntimeError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Unbound => "PAPER_RUNTIME_UNBOUND",
            Self::Unsupported => "PAPER_RUNTIME_UNSUPPORTED",
            Self::InvalidConfig => "PAPER_RUNTIME_INVALID_CONFIG",
            Self::UnsafePath => "PAPER_RUNTIME_UNSAFE_PATH",
            Self::Changed => "PAPER_RUNTIME_CHANGED",
            Self::ExecutableMismatch => "PAPER_RUNTIME_EXECUTABLE_MISMATCH",
            Self::CurrentBindingMismatch => "PAPER_RUNTIME_CURRENT_BINDING_MISMATCH",
            Self::Io => "PAPER_RUNTIME_IO",
        }
    }
}

/// Closed Paper process selectors. This value is not an action permit.
#[derive(Clone, PartialEq, Eq)]
pub struct PaperProcessBinding {
    owner_user_id: String,
    account_id: String,
    strategy_config_id: String,
    consent_event_id: String,
    consent_version: String,
    control_epoch: i64,
    daily_cohort_sha256: String,
    financial_contract_sha256: String,
    release_manifest_sha256: String,
    api_binary_sha256: String,
    worker_binary_sha256: String,
    code_commit: String,
}

impl fmt::Debug for PaperProcessBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PaperProcessBinding { .. }")
    }
}

impl PaperProcessBinding {
    pub fn owner_user_id(&self) -> &str {
        &self.owner_user_id
    }

    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub fn strategy_config_id(&self) -> &str {
        &self.strategy_config_id
    }

    pub fn consent_event_id(&self) -> &str {
        &self.consent_event_id
    }

    pub fn consent_version(&self) -> &str {
        &self.consent_version
    }

    pub fn control_epoch(&self) -> i64 {
        self.control_epoch
    }

    pub fn daily_cohort_sha256(&self) -> &str {
        &self.daily_cohort_sha256
    }

    pub fn financial_contract_sha256(&self) -> &str {
        &self.financial_contract_sha256
    }

    pub fn release_manifest_sha256(&self) -> &str {
        &self.release_manifest_sha256
    }

    pub fn code_commit(&self) -> &str {
        &self.code_commit
    }
}

/// Opaque read-only proof that the fixed config and this process executable
/// matched the root-selected Paper binding at construction and last recheck.
pub struct TrustedPaperRuntime {
    binding: PaperProcessBinding,
    config_sha256: String,
    role: PaperProcessRole,
    #[cfg(target_os = "linux")]
    files: linux::RuntimeFiles,
}

impl fmt::Debug for TrustedPaperRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TrustedPaperRuntime { .. }")
    }
}

impl TrustedPaperRuntime {
    /// Load only `/run/owner-beta-paper/config.json`; callers cannot select a
    /// path or construct trusted state from decoded or test data.
    pub fn load(role: PaperProcessRole) -> Result<Self, PaperRuntimeError> {
        #[cfg(target_os = "linux")]
        {
            let compiled_commit =
                option_env!("LAGRANGE_CODE_COMMIT").ok_or(PaperRuntimeError::InvalidConfig)?;
            let (files, bytes) = linux::RuntimeFiles::open(role, compiled_commit)?;
            let binding = parse_binding(&bytes, compiled_commit)?;
            let config_sha256 = digest(&bytes);
            Ok(Self {
                binding,
                config_sha256,
                role,
                files,
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = role;
            Err(PaperRuntimeError::Unsupported)
        }
    }

    /// Re-open the fixed config name, rehash both pinned descriptors, and
    /// confirm `/proc/self/exe` still identifies the retained executable.
    pub fn revalidate(&self) -> Result<(), PaperRuntimeError> {
        #[cfg(target_os = "linux")]
        {
            let compiled_commit =
                option_env!("LAGRANGE_CODE_COMMIT").ok_or(PaperRuntimeError::InvalidConfig)?;
            self.files.revalidate(
                &self.binding,
                &self.config_sha256,
                self.role,
                compiled_commit,
            )
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(PaperRuntimeError::Unsupported)
        }
    }

    pub fn binding(&self) -> &PaperProcessBinding {
        &self.binding
    }

    pub fn config_sha256(&self) -> &str {
        &self.config_sha256
    }

    pub fn owner_user_id(&self) -> &str {
        self.binding.owner_user_id()
    }

    pub fn account_id(&self) -> &str {
        self.binding.account_id()
    }

    pub fn strategy_config_id(&self) -> &str {
        self.binding.strategy_config_id()
    }

    pub fn control_epoch(&self) -> i64 {
        self.binding.control_epoch()
    }

    /// Compare a selector obtained from the actual ProtectedCurrentBinding
    /// snapshot. This check is only a comparison; it does not create current
    /// authority or replace the protected reader's fresh checker.
    pub fn validate_current_binding(
        &self,
        current_binding: &Value,
    ) -> Result<(), PaperRuntimeError> {
        self.revalidate()?;
        validate_current_binding_value(&self.binding, current_binding)
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn valid_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_hash(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| valid_lower_hex(hex, 64))
}

fn valid_uuid(value: &str) -> bool {
    Uuid::parse_str(value).is_ok_and(|parsed| !parsed.is_nil() && parsed.to_string() == value)
}

fn valid_revision(value: &str) -> bool {
    valid_lower_hex(value, 40) && value.bytes().any(|byte| byte != b'0')
}

fn field<'a>(value: &'a Value, key: &str) -> Result<&'a str, PaperRuntimeError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or(PaperRuntimeError::InvalidConfig)
}

fn parse_binding(
    bytes: &[u8],
    compiled_commit: &str,
) -> Result<PaperProcessBinding, PaperRuntimeError> {
    if bytes.len() > CONFIG_LIMIT || !valid_revision(compiled_commit) {
        return Err(PaperRuntimeError::InvalidConfig);
    }
    let value = wire::decode_canonical(bytes, CONFIG_LIMIT)
        .map_err(|_| PaperRuntimeError::InvalidConfig)?;
    let object = value.as_object().ok_or(PaperRuntimeError::InvalidConfig)?;
    // There are 13 required fields; keep the fixed array and the compiled
    // revision selector explicit so schema drift cannot silently open keys.
    if object.len() != 13
        || CONFIG_FIELDS.iter().any(|key| !object.contains_key(*key))
        || !object.contains_key("code_commit")
        || object
            .keys()
            .any(|key| !CONFIG_FIELDS.contains(&key.as_str()) && key.as_str() != "code_commit")
        || field(&value, "schema")? != CONFIG_SCHEMA
        || field(&value, "consent_version")? != CONSENT_VERSION
    {
        return Err(PaperRuntimeError::InvalidConfig);
    }

    let owner_user_id = field(&value, "owner_user_id")?.to_owned();
    let account_id = field(&value, "account_id")?.to_owned();
    let strategy_config_id = field(&value, "strategy_config_id")?.to_owned();
    let consent_event_id = field(&value, "consent_event_id")?.to_owned();
    let daily_cohort_sha256 = field(&value, "daily_cohort_sha256")?.to_owned();
    let financial_contract_sha256 = field(&value, "financial_contract_sha256")?.to_owned();
    let release_manifest_sha256 = field(&value, "release_manifest_sha256")?.to_owned();
    let api_binary_sha256 = field(&value, "api_binary_sha256")?.to_owned();
    let worker_binary_sha256 = field(&value, "worker_binary_sha256")?.to_owned();
    let code_commit = field(&value, "code_commit")?.to_owned();
    let control_epoch = value["control_epoch"]
        .as_i64()
        .filter(|epoch| *epoch >= 0)
        .ok_or(PaperRuntimeError::InvalidConfig)?;

    if !valid_uuid(&owner_user_id)
        || !valid_uuid(&account_id)
        || !valid_uuid(&strategy_config_id)
        || !valid_uuid(&consent_event_id)
        || !valid_hash(&daily_cohort_sha256)
        || financial_contract_sha256 != FINANCIAL_CONTRACT_SHA256
        || !valid_hash(&release_manifest_sha256)
        || !valid_hash(&api_binary_sha256)
        || !valid_hash(&worker_binary_sha256)
        || !valid_revision(&code_commit)
        || code_commit != compiled_commit
    {
        return Err(PaperRuntimeError::InvalidConfig);
    }

    Ok(PaperProcessBinding {
        owner_user_id,
        account_id,
        strategy_config_id,
        consent_event_id,
        consent_version: CONSENT_VERSION.to_owned(),
        control_epoch,
        daily_cohort_sha256,
        financial_contract_sha256,
        release_manifest_sha256,
        api_binary_sha256,
        worker_binary_sha256,
        code_commit,
    })
}

fn selected_binary_sha256(binding: &PaperProcessBinding, role: PaperProcessRole) -> &str {
    match role {
        PaperProcessRole::Api => &binding.api_binary_sha256,
        PaperProcessRole::Worker => &binding.worker_binary_sha256,
    }
}

fn require_executable_digest(actual: &str, selected: &str) -> Result<(), PaperRuntimeError> {
    if actual == selected {
        Ok(())
    } else {
        Err(PaperRuntimeError::ExecutableMismatch)
    }
}

fn validate_current_binding_value(
    binding: &PaperProcessBinding,
    current: &Value,
) -> Result<(), PaperRuntimeError> {
    let cohort = current
        .get("cohort")
        .and_then(Value::as_object)
        .ok_or(PaperRuntimeError::CurrentBindingMismatch)?;
    let matches = current.get("schema").and_then(Value::as_str)
        == Some("owner-beta-paper-witness-binding-v1")
        && cohort.get("cohort_sha256").and_then(Value::as_str)
            == Some(binding.daily_cohort_sha256())
        && cohort.get("owner_user_id").and_then(Value::as_str) == Some(binding.owner_user_id())
        && cohort.get("strategy_config_id").and_then(Value::as_str)
            == Some(binding.strategy_config_id())
        && cohort.get("verifier_revision").and_then(Value::as_str) == Some(binding.code_commit())
        && cohort
            .get("release_manifest_sha256")
            .and_then(Value::as_str)
            == Some(binding.release_manifest_sha256());
    if matches {
        Ok(())
    } else {
        Err(PaperRuntimeError::CurrentBindingMismatch)
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{
        CONFIG_LIMIT, MAX_EXECUTABLE_BYTES, PaperProcessBinding, PaperProcessRole,
        PaperRuntimeError as Error, parse_binding, require_executable_digest,
        selected_binary_sha256, valid_revision,
    };
    use std::{
        ffi::CStr,
        fs::{File, Metadata},
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::fs::{FileExt, MetadataExt},
        },
    };

    const CONFIG_UID: u32 = 0;
    const CONFIG_GID: u32 = 10_001;
    const CONFIG_MODE: u32 = 0o440;
    const CONFIG_ROOT: u32 = 0;
    const DIRECTORY_FLAGS: libc::c_int =
        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    const CONFIG_FLAGS: libc::c_int =
        libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK;
    const EXECUTABLE_FLAGS: libc::c_int = libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NONBLOCK;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct Identity {
        device: u64,
        inode: u64,
        uid: u32,
        gid: u32,
        mode: u32,
        links: u64,
        length: u64,
        mtime_seconds: i64,
        mtime_nanoseconds: i64,
        ctime_seconds: i64,
        ctime_nanoseconds: i64,
    }

    impl Identity {
        fn of(metadata: &Metadata) -> Self {
            Self {
                device: metadata.dev(),
                inode: metadata.ino(),
                uid: metadata.uid(),
                gid: metadata.gid(),
                mode: metadata.mode(),
                links: metadata.nlink(),
                length: metadata.len(),
                mtime_seconds: metadata.mtime(),
                mtime_nanoseconds: metadata.mtime_nsec(),
                ctime_seconds: metadata.ctime(),
                ctime_nanoseconds: metadata.ctime_nsec(),
            }
        }

        fn same_inode(self, other: Self) -> bool {
            self.device == other.device && self.inode == other.inode
        }

        fn same_directory(self, other: Self) -> bool {
            self.device == other.device
                && self.inode == other.inode
                && self.uid == other.uid
                && self.gid == other.gid
                && self.mode == other.mode
                && self.links == other.links
        }
    }

    #[derive(Clone, Copy)]
    struct FileAttributes {
        regular: bool,
        uid: u32,
        gid: u32,
        mode: u32,
        links: u64,
        has_acl: bool,
    }

    fn file_attributes_are_protected(value: FileAttributes) -> bool {
        value.regular
            && value.uid == CONFIG_UID
            && value.gid == CONFIG_GID
            && value.mode & 0o7777 == CONFIG_MODE
            && value.links == 1
            && !value.has_acl
    }

    fn validate_config_file(file: &File) -> Result<Identity, Error> {
        let metadata = file.metadata().map_err(|_| Error::Io)?;
        let identity = Identity::of(&metadata);
        let attrs = FileAttributes {
            regular: metadata.is_file(),
            uid: identity.uid,
            gid: identity.gid,
            mode: identity.mode,
            links: identity.links,
            has_acl: file_has_acl(file, false)?,
        };
        if !file_attributes_are_protected(attrs) {
            return Err(Error::UnsafePath);
        }
        if identity.length > CONFIG_LIMIT as u64 {
            return Err(Error::InvalidConfig);
        }
        Ok(identity)
    }

    fn validate_executable_file(file: &File) -> Result<Identity, Error> {
        let metadata = file.metadata().map_err(|_| Error::Io)?;
        let identity = Identity::of(&metadata);
        if !metadata.is_file()
            || identity.uid != 0
            || identity.mode & 0o022 != 0
            || identity.mode & 0o7000 != 0
            || file_has_acl(file, false)?
            || identity.length == 0
            || identity.length > MAX_EXECUTABLE_BYTES as u64
        {
            return Err(Error::UnsafePath);
        }
        Ok(identity)
    }

    struct DirectoryHandle {
        file: File,
        name: Option<&'static CStr>,
        identity: Identity,
    }

    struct ProtectedConfig {
        directories: Vec<DirectoryHandle>,
        file: File,
        identity: Identity,
    }

    impl ProtectedConfig {
        fn open() -> Result<(Self, Vec<u8>), Error> {
            let root = open_root()?;
            let mut directories = vec![directory_handle(root, None)?];
            for name in [c"run", c"owner-beta-paper"] {
                let parent = directories.last().ok_or(Error::UnsafePath)?;
                let file = open_directory_at(&parent.file, name)?;
                directories.push(directory_handle(file, Some(name))?);
            }
            let parent = directories.last().ok_or(Error::UnsafePath)?;
            let file = open_readonly_at(&parent.file, c"config.json").map_err(|error| {
                if error.raw_os_error() == Some(libc::ENOENT) {
                    Error::Unbound
                } else {
                    Error::UnsafePath
                }
            })?;
            let identity = validate_config_file(&file)?;
            let config = Self {
                directories,
                file,
                identity,
            };
            let bytes = config.read_and_validate()?;
            Ok((config, bytes))
        }

        fn read_and_validate(&self) -> Result<Vec<u8>, Error> {
            self.revalidate_directories()?;
            let before = validate_config_file(&self.file)?;
            if before != self.identity {
                return Err(Error::Changed);
            }
            let named_before = self.open_named_config()?;
            let named_before_identity = validate_config_file(&named_before)?;
            if named_before_identity != self.identity {
                return Err(Error::Changed);
            }
            let bytes = read_at_bounded(&self.file, CONFIG_LIMIT)?;
            let after = validate_config_file(&self.file)?;
            let named_after = self.open_named_config()?;
            let named_after_identity = validate_config_file(&named_after)?;
            if after != before
                || named_after_identity != self.identity
                || bytes.len() as u64 != after.length
            {
                return Err(Error::Changed);
            }
            Ok(bytes)
        }

        fn open_named_config(&self) -> Result<File, Error> {
            let parent = self.directories.last().ok_or(Error::UnsafePath)?;
            open_readonly_at(&parent.file, c"config.json").map_err(|_| Error::Changed)
        }

        fn revalidate_directories(&self) -> Result<(), Error> {
            for (index, directory) in self.directories.iter().enumerate() {
                validate_directory(&directory.file)?;
                let observed = Identity::of(&directory.file.metadata().map_err(|_| Error::Io)?);
                if !observed.same_directory(directory.identity) {
                    return Err(Error::Changed);
                }
                if index > 0 {
                    let parent = &self.directories[index - 1];
                    let name = directory.name.ok_or(Error::Changed)?;
                    let named =
                        open_directory_at(&parent.file, name).map_err(|_| Error::Changed)?;
                    let named_identity = Identity::of(&named.metadata().map_err(|_| Error::Io)?);
                    if !named_identity.same_directory(directory.identity) {
                        return Err(Error::Changed);
                    }
                }
            }
            Ok(())
        }
    }

    struct Executable {
        file: File,
        identity: Identity,
        sha256: String,
    }

    impl Executable {
        fn open(role: PaperProcessRole, binding: &PaperProcessBinding) -> Result<Self, Error> {
            let file = open_current_executable()?;
            let identity = validate_executable_file(&file)?;
            let bytes = read_at_bounded(&file, MAX_EXECUTABLE_BYTES)?;
            if bytes.len() < 4 || !bytes.starts_with(b"\x7fELF") {
                return Err(Error::ExecutableMismatch);
            }
            let after = validate_executable_file(&file)?;
            if identity != after || bytes.len() as u64 != identity.length {
                return Err(Error::Changed);
            }
            let sha256 = super::digest(&bytes);
            require_executable_digest(&sha256, selected_binary_sha256(binding, role))?;
            Ok(Self {
                file,
                identity,
                sha256,
            })
        }

        fn revalidate(
            &self,
            role: PaperProcessRole,
            binding: &PaperProcessBinding,
        ) -> Result<(), Error> {
            let before = validate_executable_file(&self.file)?;
            let current = open_current_executable()?;
            let current_identity = validate_executable_file(&current)?;
            if !current_identity.same_inode(self.identity) || before != self.identity {
                return Err(Error::Changed);
            }
            let bytes = read_at_bounded(&self.file, MAX_EXECUTABLE_BYTES)?;
            let after = validate_executable_file(&self.file)?;
            if after != before || bytes.len() as u64 != after.length {
                return Err(Error::Changed);
            }
            let actual = super::digest(&bytes);
            require_executable_digest(&actual, &self.sha256)?;
            require_executable_digest(&actual, selected_binary_sha256(binding, role))?;
            if bytes.len() < 4 || !bytes.starts_with(b"\x7fELF") {
                return Err(Error::ExecutableMismatch);
            }
            Ok(())
        }
    }

    pub(super) struct RuntimeFiles {
        config: ProtectedConfig,
        executable: Executable,
    }

    impl RuntimeFiles {
        pub(super) fn open(
            role: PaperProcessRole,
            compiled_commit: &str,
        ) -> Result<(Self, Vec<u8>), Error> {
            if !valid_revision(compiled_commit) {
                return Err(Error::InvalidConfig);
            }
            let (config, bytes) = ProtectedConfig::open()?;
            let binding = parse_binding(&bytes, compiled_commit)?;
            let executable = Executable::open(role, &binding)?;
            Ok((Self { config, executable }, bytes))
        }

        pub(super) fn revalidate(
            &self,
            binding: &PaperProcessBinding,
            expected_config_sha256: &str,
            role: PaperProcessRole,
            compiled_commit: &str,
        ) -> Result<(), Error> {
            if !valid_revision(compiled_commit) || compiled_commit != binding.code_commit() {
                return Err(Error::InvalidConfig);
            }
            let bytes = self.config.read_and_validate()?;
            if super::digest(&bytes) != expected_config_sha256 {
                return Err(Error::Changed);
            }
            let current = parse_binding(&bytes, compiled_commit)?;
            if &current != binding {
                return Err(Error::Changed);
            }
            self.executable.revalidate(role, binding)
        }
    }

    fn open_root() -> Result<File, Error> {
        // SAFETY: constant path and flags request a read-only directory fd.
        own_file(unsafe { libc::open(c"/".as_ptr(), DIRECTORY_FLAGS) }).map_err(|_| Error::Io)
    }

    fn open_directory_at(parent: &File, name: &CStr) -> Result<File, Error> {
        // SAFETY: parent is a live directory fd, name is NUL-terminated, and
        // O_NOFOLLOW prevents following a user-controlled symbolic link.
        own_file(unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), DIRECTORY_FLAGS) })
            .map_err(|error| {
                if error.raw_os_error() == Some(libc::ENOENT) {
                    Error::Unbound
                } else {
                    Error::UnsafePath
                }
            })
    }

    fn open_readonly_at(parent: &File, name: &CStr) -> std::io::Result<File> {
        // SAFETY: parent is a live directory fd, name is NUL-terminated, and
        // O_NOFOLLOW rejects a symbolic-link leaf. O_NONBLOCK avoids hanging
        // if the leaf has been replaced with a FIFO before fstat rejects it.
        own_file(unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), CONFIG_FLAGS) })
    }

    fn own_file(fd: libc::c_int) -> std::io::Result<File> {
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: successful open/openat transfers a new owned descriptor.
        Ok(unsafe { File::from_raw_fd(fd) })
    }

    fn directory_handle(file: File, name: Option<&'static CStr>) -> Result<DirectoryHandle, Error> {
        validate_directory(&file)?;
        let identity = Identity::of(&file.metadata().map_err(|_| Error::Io)?);
        Ok(DirectoryHandle {
            file,
            name,
            identity,
        })
    }

    fn validate_directory(file: &File) -> Result<(), Error> {
        let metadata = file.metadata().map_err(|_| Error::Io)?;
        if !metadata.is_dir()
            || metadata.uid() != CONFIG_ROOT
            || metadata.mode() & 0o022 != 0
            || file_has_acl(file, true)?
        {
            return Err(Error::UnsafePath);
        }
        Ok(())
    }

    fn file_has_acl(file: &File, directory: bool) -> Result<bool, Error> {
        let names: &[&CStr] = if directory {
            &[c"system.posix_acl_access", c"system.posix_acl_default"]
        } else {
            &[c"system.posix_acl_access"]
        };
        for name in names {
            // SAFETY: fgetxattr with a null buffer and zero size queries only
            // the attribute length and does not dereference a caller buffer.
            let length = unsafe {
                libc::fgetxattr(file.as_raw_fd(), name.as_ptr(), std::ptr::null_mut(), 0)
            };
            if length >= 0 {
                return Ok(true);
            }
            if std::io::Error::last_os_error().raw_os_error() != Some(libc::ENODATA) {
                return Err(Error::Unsupported);
            }
        }
        Ok(false)
    }

    fn read_at_bounded(file: &File, limit: usize) -> Result<Vec<u8>, Error> {
        let probe_limit = limit.checked_add(1).ok_or(Error::InvalidConfig)?;
        let mut bytes = Vec::new();
        let mut offset = 0_u64;
        let mut buffer = [0_u8; 8192];
        loop {
            let remaining = probe_limit
                .checked_sub(bytes.len())
                .ok_or(Error::InvalidConfig)?;
            if remaining == 0 {
                return Err(Error::InvalidConfig);
            }
            let count = remaining.min(buffer.len());
            let read = file
                .read_at(&mut buffer[..count], offset)
                .map_err(|_| Error::Io)?;
            if read == 0 {
                break;
            }
            offset = offset
                .checked_add(u64::try_from(read).map_err(|_| Error::InvalidConfig)?)
                .ok_or(Error::InvalidConfig)?;
            bytes.extend_from_slice(&buffer[..read]);
            if bytes.len() > limit {
                return Err(Error::InvalidConfig);
            }
        }
        Ok(bytes)
    }

    fn open_current_executable() -> Result<File, Error> {
        // This is the one intentional use of procfs's kernel magic link. It
        // identifies the executable of this process; config paths never use it.
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(EXECUTABLE_FLAGS)
            .open("/proc/self/exe")
            .map_err(|_| Error::Unsupported)?;
        Ok(file)
    }

    use std::os::unix::fs::OpenOptionsExt;

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::owner_beta::paper::runtime::{
            CONFIG_SCHEMA, CONSENT_VERSION, FINANCIAL_CONTRACT_SHA256, PaperRuntimeError,
            parse_binding, require_executable_digest, selected_binary_sha256,
            validate_current_binding_value,
        };
        use serde_json::{Value, json};

        const REVISION: &str = "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1";

        fn fixture() -> Value {
            json!({
                "schema": CONFIG_SCHEMA,
                "owner_user_id": "00000000-0000-4000-8000-000000000001",
                "account_id": "00000000-0000-4000-8000-000000000002",
                "strategy_config_id": "00000000-0000-4000-8000-000000000003",
                "consent_event_id": "00000000-0000-4000-8000-000000000004",
                "consent_version": CONSENT_VERSION,
                "control_epoch": 0,
                "daily_cohort_sha256": format!("sha256:{}", "b".repeat(64)),
                "financial_contract_sha256": FINANCIAL_CONTRACT_SHA256,
                "release_manifest_sha256": format!("sha256:{}", "c".repeat(64)),
                "api_binary_sha256": format!("sha256:{}", "d".repeat(64)),
                "worker_binary_sha256": format!("sha256:{}", "e".repeat(64)),
                "code_commit": REVISION,
            })
        }

        fn encode(value: &Value) -> Vec<u8> {
            crate::owner_beta::paper::source_evidence::canonical(value).expect("canonical JSON")
        }

        fn binding() -> PaperProcessBinding {
            parse_binding(&encode(&fixture()), REVISION).expect("valid synthetic binding")
        }

        #[test]
        fn accepts_closed_synthetic_binding_and_selects_role_digest() {
            let parsed = binding();
            assert_eq!(
                parsed.owner_user_id(),
                "00000000-0000-4000-8000-000000000001"
            );
            assert_eq!(parsed.control_epoch(), 0);
            assert_eq!(
                selected_binary_sha256(&parsed, PaperProcessRole::Api),
                format!("sha256:{}", "d".repeat(64))
            );
            assert_eq!(
                selected_binary_sha256(&parsed, PaperProcessRole::Worker),
                format!("sha256:{}", "e".repeat(64))
            );
        }

        #[test]
        fn rejects_unknown_missing_duplicate_float_boolean_and_contract_mismatch() {
            let mut unknown = fixture();
            unknown["enable"] = json!(true);
            assert_eq!(
                parse_binding(&encode(&unknown), REVISION),
                Err(PaperRuntimeError::InvalidConfig)
            );

            let mut missing = fixture();
            missing
                .as_object_mut()
                .expect("object")
                .remove("account_id");
            assert_eq!(
                parse_binding(&encode(&missing), REVISION),
                Err(PaperRuntimeError::InvalidConfig)
            );

            let canonical = String::from_utf8(encode(&fixture())).expect("canonical UTF-8");
            let duplicate = canonical.replace(
                ",\"api_binary_sha256\":",
                ",\"account_id\":\"00000000-0000-4000-8000-000000000002\",\"api_binary_sha256\":",
            );
            assert_eq!(
                parse_binding(duplicate.as_bytes(), REVISION),
                Err(PaperRuntimeError::InvalidConfig)
            );

            let mut float_epoch = fixture();
            float_epoch["control_epoch"] = json!(1.0);
            assert_eq!(
                parse_binding(&encode(&float_epoch), REVISION),
                Err(PaperRuntimeError::InvalidConfig)
            );

            let mut bool_epoch = fixture();
            bool_epoch["control_epoch"] = json!(true);
            assert_eq!(
                parse_binding(&encode(&bool_epoch), REVISION),
                Err(PaperRuntimeError::InvalidConfig)
            );

            let mut wrong_financial = fixture();
            wrong_financial["financial_contract_sha256"] =
                json!(format!("sha256:{}", "f".repeat(64)));
            assert_eq!(
                parse_binding(&encode(&wrong_financial), REVISION),
                Err(PaperRuntimeError::InvalidConfig)
            );
        }

        #[test]
        fn rejects_uuid_revision_digest_and_compiled_revision_mismatches() {
            let mut invalid_uuid = fixture();
            invalid_uuid["account_id"] = json!("00000000-0000-0000-0000-000000000000");
            assert_eq!(
                parse_binding(&encode(&invalid_uuid), REVISION),
                Err(PaperRuntimeError::InvalidConfig)
            );

            let mut invalid_digest = fixture();
            invalid_digest["daily_cohort_sha256"] = json!(format!("sha256:{}", "A".repeat(64)));
            assert_eq!(
                parse_binding(&encode(&invalid_digest), REVISION),
                Err(PaperRuntimeError::InvalidConfig)
            );

            assert_eq!(
                parse_binding(
                    &encode(&fixture()),
                    "b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2"
                ),
                Err(PaperRuntimeError::InvalidConfig)
            );
            assert!(!super::super::super::runtime::valid_revision(
                &"0".repeat(40)
            ));
        }

        #[test]
        fn protected_file_policy_rejects_wrong_owner_mode_links_and_acl() {
            let good = FileAttributes {
                regular: true,
                uid: 0,
                gid: 10_001,
                mode: 0o100440,
                links: 1,
                has_acl: false,
            };
            assert!(file_attributes_are_protected(good));
            assert!(!file_attributes_are_protected(FileAttributes {
                uid: 1000,
                ..good
            }));
            assert!(!file_attributes_are_protected(FileAttributes {
                mode: 0o100444,
                ..good
            }));
            assert!(!file_attributes_are_protected(FileAttributes {
                links: 2,
                ..good
            }));
            assert!(!file_attributes_are_protected(FileAttributes {
                has_acl: true,
                ..good
            }));
            assert!(!file_attributes_are_protected(FileAttributes {
                regular: false,
                ..good
            }));
        }

        #[test]
        fn rejects_symbolic_link_for_fixed_leaf_open_primitive() {
            let directory = tempfile::tempdir().expect("temporary directory");
            std::fs::write(directory.path().join("target"), b"not an authority").expect("target");
            std::os::unix::fs::symlink("target", directory.path().join("link")).expect("symlink");
            std::fs::create_dir(directory.path().join("real-dir")).expect("real directory");
            std::os::unix::fs::symlink("real-dir", directory.path().join("dir-link"))
                .expect("directory symlink");
            let parent = File::open(directory.path()).expect("parent fd");
            let name = std::ffi::CString::new("link").expect("static leaf");
            assert!(open_readonly_at(&parent, name.as_c_str()).is_err());
            assert!(open_directory_at(&parent, c"dir-link").is_err());
        }

        #[test]
        fn detects_config_replacement_by_inode_identity() {
            let first = tempfile::NamedTempFile::new().expect("first file");
            let second = tempfile::NamedTempFile::new().expect("second file");
            let first_identity = Identity::of(&first.as_file().metadata().expect("first metadata"));
            let second_identity =
                Identity::of(&second.as_file().metadata().expect("second metadata"));
            assert!(!first_identity.same_inode(second_identity));
        }

        #[test]
        fn enforces_bounded_reads_and_current_cohort_comparison() {
            use std::io::Write;
            let mut oversized = tempfile::NamedTempFile::new().expect("temporary file");
            oversized
                .write_all(&[7_u8; 64])
                .expect("write bounded-read fixture");
            assert!(read_at_bounded(oversized.as_file(), 32).is_err());

            let binding = binding();
            let current = json!({
                "schema": "owner-beta-paper-witness-binding-v1",
                "cohort": {
                    "cohort_sha256": binding.daily_cohort_sha256(),
                    "owner_user_id": binding.owner_user_id(),
                    "strategy_config_id": binding.strategy_config_id(),
                    "verifier_revision": binding.code_commit(),
                    "release_manifest_sha256": binding.release_manifest_sha256(),
                }
            });
            assert!(validate_current_binding_value(&binding, &current).is_ok());
            let mut changed = current.clone();
            changed["cohort"]["cohort_sha256"] = json!(format!("sha256:{}", "9".repeat(64)));
            assert_eq!(
                validate_current_binding_value(&binding, &changed),
                Err(PaperRuntimeError::CurrentBindingMismatch)
            );
        }

        #[test]
        fn executable_hash_must_match_role_selection() {
            let binding = binding();
            let selected_api = selected_binary_sha256(&binding, PaperProcessRole::Api);
            let selected_worker = selected_binary_sha256(&binding, PaperProcessRole::Worker);
            assert_eq!(
                require_executable_digest(selected_api, selected_api),
                Ok(())
            );
            assert_eq!(
                require_executable_digest(&super::super::digest(b"synthetic ELF"), selected_api),
                Err(PaperRuntimeError::ExecutableMismatch)
            );
            assert_ne!(selected_api, selected_worker);
        }

        #[test]
        fn rejects_config_bytes_over_the_fixed_limit() {
            let oversized = vec![b' '; CONFIG_LIMIT + 1];
            assert_eq!(
                parse_binding(&oversized, REVISION),
                Err(PaperRuntimeError::InvalidConfig)
            );
        }
    }
}

#[cfg(test)]
mod platform_tests {
    use super::*;

    #[test]
    fn errors_and_debug_output_are_value_free() {
        let error = PaperRuntimeError::UnsafePath;
        assert_eq!(format!("{error:?}"), "PAPER_RUNTIME_UNSAFE_PATH");
        assert_eq!(format!("{error}"), "PAPER_RUNTIME_UNSAFE_PATH");
        let binding = PaperProcessBinding {
            owner_user_id: "private-id".to_owned(),
            account_id: "account-id".to_owned(),
            strategy_config_id: "strategy-id".to_owned(),
            consent_event_id: "consent-id".to_owned(),
            consent_version: CONSENT_VERSION.to_owned(),
            control_epoch: 0,
            daily_cohort_sha256: "private-hash".to_owned(),
            financial_contract_sha256: FINANCIAL_CONTRACT_SHA256.to_owned(),
            release_manifest_sha256: "private-release".to_owned(),
            api_binary_sha256: "api".to_owned(),
            worker_binary_sha256: "worker".to_owned(),
            code_commit: "a".repeat(40),
        };
        let debug = format!("{binding:?}");
        assert!(!debug.contains("private-id"));
        assert!(!debug.contains("account-id"));
        assert!(!debug.contains("private-hash"));
    }
}
