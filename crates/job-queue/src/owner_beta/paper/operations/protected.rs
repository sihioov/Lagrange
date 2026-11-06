use std::{
    ffi::CString,
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    os::fd::{AsRawFd, FromRawFd, RawFd},
    path::Path,
    time::Duration,
};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use uuid::Uuid;

use super::{
    ErrorCode, Outcome, ResultWire, Verb,
    wire::{self, Artifact, Intent},
};

const PRIVATE_DIR: &str = "/run/owner-beta-paper-ops";
const REQUEST_PATH: &str = "/run/owner-beta-paper-ops/request.json";
const EVIDENCE_PATH: &str = "/run/owner-beta-paper-ops/evidence.json";
const DATABASE_PATH: &str = "/run/owner-beta-paper-ops/database.json";
const OUTPUT_PATH: &str = "/run/owner-beta-paper-ops/output";
const CAPABILITY_PATH: &str = "/run/owner-beta-paper-ops/capability.json";
const API_IMAGE_ELF_PATH: &str = "/run/owner-beta-paper-ops/api-image.elf";
const WORKER_IMAGE_ELF_PATH: &str = "/run/owner-beta-paper-ops/worker-image.elf";
const OPERATIONS_SCHEMA_PATH: &str = "/run/owner-beta-paper-ops/operations.schema.json";
const ARCHIVE_SCHEMA_PATH: &str = "/run/owner-beta-paper-ops/archive.schema.json";
const MIGRATION_PATH: &str = "/run/owner-beta-paper-ops/migration-0068.up.sql";
const AUDIT_SQL_PATH: &str = "/run/owner-beta-paper-ops/audit.sql";
const MIGRATION_PASSWORD: &str = "/run/secrets/db_migration_owner_password";
const WORKER_PASSWORD: &str = "/run/secrets/db_worker_password";
const MAX_DB_SELECTOR: usize = 4096;
const MAX_PASSWORD_BYTES: usize = 4096;
const MAX_SOURCE_FILE_BYTES: usize = 1_048_576;
const MAX_ELF_BYTES: u64 = 512 * 1024 * 1024;
const MAX_INSPECT_PAGE_BYTES: usize = wire::MAX_PAGE_BYTES;
const INSPECT_PAGE_LIMIT: i32 = 64;
const CLOCK_FRESHNESS_NS: i64 = 30_000_000_000;
// The current 0068 wrapper delegates ENABLE to the 0067 transition and has no
// read-only event readback or post-reset ARM-origin fence. Keep the consuming
// checker path fail-closed until P1 supplies and tests that SQL contract.
const ENABLE_SQL_RESET_AND_READBACK_READY: bool = false;
const DB_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DB_COMMIT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Identity {
    device: u64,
    inode: u64,
    size: u64,
    mode: u32,
    uid: u32,
    links: u64,
    mtime: (i64, i64),
    ctime: (i64, i64),
}

struct GuardedFile {
    file: File,
    path: &'static str,
    identity: Identity,
    // Keeps every opened directory ancestor alive while the file is consumed.
    _parents: Vec<File>,
}

struct GuardedDirectory {
    directory: File,
    path: &'static str,
    identity: Identity,
    _parents: Vec<File>,
}

#[derive(Debug)]
enum GuardError {
    Invalid,
    Read,
    Changed,
}

fn identity(file: &File) -> Result<Identity, GuardError> {
    let m = file.metadata().map_err(|_| GuardError::Read)?;
    use std::os::unix::fs::MetadataExt;
    Ok(Identity {
        device: m.dev(),
        inode: m.ino(),
        size: m.size(),
        mode: m.mode(),
        uid: m.uid(),
        links: m.nlink(),
        mtime: (m.mtime(), m.mtime_nsec()),
        ctime: (m.ctime(), m.ctime_nsec()),
    })
}

fn has_acl(fd: RawFd, name: &str) -> Result<bool, GuardError> {
    let name = CString::new(name).map_err(|_| GuardError::Invalid)?;
    // fgetxattr with a null buffer asks for the size without reading or logging
    // ACL content. Any present ACL is rejected.
    let result = unsafe { libc::fgetxattr(fd, name.as_ptr(), std::ptr::null_mut(), 0) };
    if result >= 0 {
        return Ok(true);
    }
    let errno = std::io::Error::last_os_error()
        .raw_os_error()
        .unwrap_or_default();
    if errno == libc::ENODATA || errno == libc::ENOTSUP {
        Ok(false)
    } else {
        Err(GuardError::Invalid)
    }
}

fn check_dir(file: &File, private: bool) -> Result<Identity, GuardError> {
    let id = identity(file)?;
    let perms = id.mode & 0o7777;
    if id.uid != 0
        || id.links < 2
        || perms & 0o022 != 0
        || (private && perms != 0o700)
        || has_acl(file.as_raw_fd(), "system.posix_acl_access")?
        || has_acl(file.as_raw_fd(), "system.posix_acl_default")?
    {
        return Err(GuardError::Invalid);
    }
    Ok(id)
}

fn check_file(file: &File) -> Result<Identity, GuardError> {
    let id = identity(file)?;
    if id.uid != 0
        || (id.mode & libc::S_IFMT) != libc::S_IFREG
        || (id.mode & 0o7777) != 0o400
        || id.links != 1
        || has_acl(file.as_raw_fd(), "system.posix_acl_access")?
    {
        return Err(GuardError::Invalid);
    }
    Ok(id)
}

fn open_fixed(path: &'static str, want_dir: bool) -> Result<(File, Vec<File>), GuardError> {
    if !path.starts_with('/') || path.contains("..") || path.as_bytes().contains(&0) {
        return Err(GuardError::Invalid);
    }
    let root = openat_file(
        libc::AT_FDCWD,
        "/",
        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
    )?;
    let mut parents = vec![root];
    let mut parts = Path::new(path)
        .components()
        .filter_map(|part| match part {
            std::path::Component::RootDir => None,
            std::path::Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .peekable();
    let mut current_fd = parents.last().ok_or(GuardError::Invalid)?.as_raw_fd();
    while let Some(part) = parts.next() {
        let is_final = parts.peek().is_none();
        let flags = libc::O_RDONLY
            | libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | if !is_final || want_dir {
                libc::O_DIRECTORY
            } else {
                0
            };
        let next = openat_file(current_fd, part, flags)?;
        if is_final {
            if want_dir {
                check_dir(&next, true)?;
            } else {
                check_file(&next)?;
            }
            return Ok((next, parents));
        }
        // The operation directory itself is private. Container secret mounts
        // commonly expose a non-writable 0755 parent; the secret file remains
        // root-owned, single-link, ACL-free and exactly 0400 below.
        let private = part == "owner-beta-paper-ops";
        check_dir(&next, private)?;
        parents.push(next);
        current_fd = parents.last().ok_or(GuardError::Invalid)?.as_raw_fd();
    }
    Err(GuardError::Invalid)
}

fn openat_file(dirfd: RawFd, name: &str, flags: i32) -> Result<File, GuardError> {
    let name = CString::new(name).map_err(|_| GuardError::Invalid)?;
    let fd = unsafe { libc::openat(dirfd, name.as_ptr(), flags) };
    if fd < 0 {
        return Err(GuardError::Invalid);
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

impl GuardedFile {
    fn open(path: &'static str) -> Result<Self, GuardError> {
        let (file, parents) = open_fixed(path, false)?;
        let identity = check_file(&file)?;
        Ok(Self {
            file,
            path,
            identity,
            _parents: parents,
        })
    }

    fn read_bounded(&mut self, max: usize) -> Result<Vec<u8>, GuardError> {
        self.revalidate()?;
        let mut bytes = Vec::with_capacity(max.min(8192));
        (&mut self.file)
            .take((max + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| GuardError::Read)?;
        if bytes.len() > max {
            return Err(GuardError::Invalid);
        }
        self.revalidate()?;
        Ok(bytes)
    }

    fn digest_bounded(&mut self, max: u64) -> Result<String, GuardError> {
        self.revalidate()?;
        if self.identity.size > max {
            return Err(GuardError::Invalid);
        }
        let digest = digest_file(&mut self.file, max)?;
        self.revalidate()?;
        Ok(digest)
    }

    fn revalidate(&self) -> Result<(), GuardError> {
        if identity(&self.file)? != self.identity {
            return Err(GuardError::Changed);
        }
        let (named, _parents) = open_fixed(self.path, false)?;
        if identity(&named)? != self.identity {
            return Err(GuardError::Changed);
        }
        Ok(())
    }
}

struct GuardedExecutable {
    file: File,
    identity: Identity,
    sha256: String,
}

impl GuardedExecutable {
    fn open() -> Result<Self, GuardError> {
        let mut file = File::open("/proc/self/exe").map_err(|_| GuardError::Read)?;
        let identity = identity(&file)?;
        if identity.uid != 0
            || (identity.mode & libc::S_IFMT) != libc::S_IFREG
            || identity.mode & 0o022 != 0
            || identity.links != 1
            || identity.size > MAX_ELF_BYTES
            || has_acl(file.as_raw_fd(), "system.posix_acl_access")?
        {
            return Err(GuardError::Invalid);
        }
        if !is_elf(&mut file)? {
            return Err(GuardError::Invalid);
        }
        let sha256 = digest_file(&mut file, MAX_ELF_BYTES)?;
        Ok(Self {
            file,
            identity,
            sha256,
        })
    }

    fn revalidate(&self) -> Result<(), GuardError> {
        if identity(&self.file)? != self.identity {
            return Err(GuardError::Changed);
        }
        let named = File::open("/proc/self/exe").map_err(|_| GuardError::Read)?;
        if identity(&named)? != self.identity {
            return Err(GuardError::Changed);
        }
        Ok(())
    }
}

fn digest_file(file: &mut File, max: u64) -> Result<String, GuardError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|_| GuardError::Read)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let count = file.read(&mut buffer).map_err(|_| GuardError::Read)?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(count as u64)
            .filter(|length| *length <= max)
            .ok_or(GuardError::Invalid)?;
        hasher.update(&buffer[..count]);
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|_| GuardError::Read)?;
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

fn is_elf(file: &mut File) -> Result<bool, GuardError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|_| GuardError::Read)?;
    let mut magic = [0_u8; 4];
    let read = file.read(&mut magic).map_err(|_| GuardError::Read)?;
    file.seek(SeekFrom::Start(0))
        .map_err(|_| GuardError::Read)?;
    Ok(read == magic.len() && magic == *b"\x7fELF")
}

impl GuardedDirectory {
    fn open(path: &'static str) -> Result<Self, GuardError> {
        let (directory, parents) = open_fixed(path, true)?;
        let identity = check_dir(&directory, true)?;
        Ok(Self {
            directory,
            path,
            identity,
            _parents: parents,
        })
    }

    fn revalidate(&self) -> Result<(), GuardError> {
        if !same_object(identity(&self.directory)?, self.identity) {
            return Err(GuardError::Changed);
        }
        let (named, _parents) = open_fixed(self.path, true)?;
        if !same_object(identity(&named)?, self.identity) {
            return Err(GuardError::Changed);
        }
        Ok(())
    }

    fn write_content_addressed(&self, bytes: &[u8]) -> Result<String, GuardError> {
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(GuardError::Invalid);
        }
        self.revalidate()?;
        let digest = digest(bytes);
        let leaf = format!("{}.json", &digest[7..]);
        let c_leaf = CString::new(leaf.as_str()).map_err(|_| GuardError::Invalid)?;
        let fd = unsafe {
            libc::openat(
                self.directory.as_raw_fd(),
                c_leaf.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                0o400,
            )
        };
        if fd < 0 {
            let errno = std::io::Error::last_os_error()
                .raw_os_error()
                .unwrap_or_default();
            if errno == libc::EEXIST {
                let existing = openat_file(
                    self.directory.as_raw_fd(),
                    leaf.as_str(),
                    libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                )?;
                let existing_id = check_file_mode(&existing, 0o400)?;
                if existing_id.size != bytes.len() as u64 {
                    return Err(GuardError::Changed);
                }
                let mut existing_bytes = Vec::with_capacity(bytes.len());
                (&existing)
                    .take(bytes.len() as u64 + 1)
                    .read_to_end(&mut existing_bytes)
                    .map_err(|_| GuardError::Read)?;
                if existing_bytes != bytes {
                    return Err(GuardError::Changed);
                }
                self.revalidate()?;
                return Ok(digest);
            }
            return Err(GuardError::Invalid);
        }
        let mut file = unsafe { File::from_raw_fd(fd) };
        file.write_all(bytes).map_err(|_| GuardError::Read)?;
        file.sync_all().map_err(|_| GuardError::Read)?;
        self.directory.sync_all().map_err(|_| GuardError::Read)?;
        self.revalidate()?;
        Ok(digest)
    }
}

fn same_object(left: Identity, right: Identity) -> bool {
    left.device == right.device
        && left.inode == right.inode
        && left.mode == right.mode
        && left.uid == right.uid
        && left.links == right.links
}

fn check_file_mode(file: &File, expected_mode: u32) -> Result<Identity, GuardError> {
    let id = identity(file)?;
    if id.uid != 0
        || (id.mode & libc::S_IFMT) != libc::S_IFREG
        || (id.mode & 0o7777) != expected_mode
        || id.links != 1
        || has_acl(file.as_raw_fd(), "system.posix_acl_access")?
    {
        return Err(GuardError::Invalid);
    }
    Ok(id)
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn clock_ns(clock_id: libc::clockid_t) -> Result<i64, ()> {
    let mut value = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if unsafe { libc::clock_gettime(clock_id, &mut value) } != 0 {
        return Err(());
    }
    i64::try_from(value.tv_sec)
        .ok()
        .and_then(|sec| sec.checked_mul(1_000_000_000))
        .and_then(|sec| sec.checked_add(value.tv_nsec as i64))
        .ok_or(())
}

fn check_clock(value: &Value, boot_id: &str) -> Result<(), ErrorCode> {
    let boot_id = Uuid::parse_str(boot_id).map_err(|_| ErrorCode::ClockInvalid)?;
    let actual_boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map_err(|_| ErrorCode::ClockInvalid)?;
    if actual_boot.trim() != boot_id.to_string() {
        return Err(ErrorCode::ClockInvalid);
    }
    let realtime = value["realtime_ns"]
        .as_i64()
        .ok_or(ErrorCode::ClockInvalid)?;
    let monotonic = value["monotonic_ns"]
        .as_i64()
        .ok_or(ErrorCode::ClockInvalid)?;
    let boottime = value["boottime_ns"]
        .as_i64()
        .ok_or(ErrorCode::ClockInvalid)?;
    if realtime < 0 || monotonic < 0 || boottime < 0 {
        return Err(ErrorCode::ClockInvalid);
    }
    let now_realtime = clock_ns(libc::CLOCK_REALTIME).map_err(|_| ErrorCode::ClockInvalid)?;
    let now_monotonic = clock_ns(libc::CLOCK_MONOTONIC).map_err(|_| ErrorCode::ClockInvalid)?;
    let now_boottime = clock_ns(libc::CLOCK_BOOTTIME).map_err(|_| ErrorCode::ClockInvalid)?;
    let age = now_realtime
        .checked_sub(realtime)
        .ok_or(ErrorCode::ClockInvalid)?;
    let monotonic_age = now_monotonic
        .checked_sub(monotonic)
        .ok_or(ErrorCode::ClockInvalid)?;
    let boottime_age = now_boottime
        .checked_sub(boottime)
        .ok_or(ErrorCode::ClockInvalid)?;
    if !clock_samples_fresh(age, monotonic_age, boottime_age) {
        return Err(ErrorCode::ClockInvalid);
    }
    Ok(())
}

fn clock_samples_fresh(realtime_age: i64, monotonic_age: i64, boottime_age: i64) -> bool {
    realtime_age >= 0
        && realtime_age < CLOCK_FRESHNESS_NS
        && monotonic_age >= 0
        && boottime_age >= 0
        && boottime_age.abs_diff(monotonic_age) <= 1_000_000_000
}

fn finalize_after_recheck(result: ResultWire, recheck_succeeded: bool, verb: Verb) -> ResultWire {
    if recheck_succeeded {
        return result;
    }
    match result.outcome {
        Outcome::Ok | Outcome::Replay if verb == Verb::Inspect => {
            ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible)
        }
        Outcome::Ok | Outcome::Replay | Outcome::CommitUnknown => {
            ResultWire::new(verb, Outcome::CommitUnknown, ErrorCode::CommitUnknown)
        }
        Outcome::Blocked => result,
    }
}

async fn close_pool(pool: PgPool, result: ResultWire, verb: Verb) -> ResultWire {
    let close_completed = tokio::time::timeout(Duration::from_secs(5), pool.close())
        .await
        .is_ok();
    finalize_after_recheck(result, close_completed, verb)
}

macro_rules! finish_with_pool {
    ($pool:ident, $verb:ident, $body:block) => {{
        let pool_to_close = $pool;
        let pool_for_operation = pool_to_close.clone();
        let result = async move {
            let pool = pool_for_operation;
            $body
        }
        .await;
        close_pool(pool_to_close, result, $verb).await
    }};
}

fn env_is_clean() -> bool {
    std::env::vars_os().all(|(key, _)| {
        let Some(key) = key.to_str() else {
            return false;
        };
        // The native boundary consumes no configuration from the environment.
        // A tiny locale/timezone allowlist avoids path, identity, SQL and loader
        // inputs being smuggled through aliases unknown to a denylist.
        matches!(key, "LANG" | "LC_ALL" | "TZ")
    })
}

struct DatabaseSelector {
    host: String,
    database: String,
}

#[allow(
    dead_code,
    reason = "Worker is reserved for the joined enable checker callback."
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DatabaseRole {
    Worker,
    MigrationOwner,
}

impl DatabaseRole {
    fn name(self) -> &'static str {
        match self {
            Self::Worker => "worker",
            Self::MigrationOwner => "migration_owner",
        }
    }

    fn password_path(self) -> &'static str {
        match self {
            Self::Worker => WORKER_PASSWORD,
            Self::MigrationOwner => MIGRATION_PASSWORD,
        }
    }
}

struct ProtectedInputs {
    request: GuardedFile,
    database: GuardedFile,
    evidence: Option<GuardedFile>,
}

struct EnablePins {
    capability: GuardedFile,
    api_image: GuardedFile,
    worker_image: GuardedFile,
    operations_schema: GuardedFile,
    archive_schema: GuardedFile,
    migration: GuardedFile,
    audit_sql: GuardedFile,
    executable: GuardedExecutable,
}

impl EnablePins {
    fn open(intent: &Intent, process: &Artifact) -> Result<Self, ErrorCode> {
        let (mut capability, capability_bytes) = read_fixed(CAPABILITY_PATH, wire::MAX_WIRE_BYTES)
            .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let capability_value =
            wire::parse_capability(&capability_bytes).map_err(|_| ErrorCode::WireInvalid)?;
        let mut api_image =
            GuardedFile::open(API_IMAGE_ELF_PATH).map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let mut worker_image =
            GuardedFile::open(WORKER_IMAGE_ELF_PATH).map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let mut operations_schema = GuardedFile::open(OPERATIONS_SCHEMA_PATH)
            .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let mut archive_schema =
            GuardedFile::open(ARCHIVE_SCHEMA_PATH).map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let mut migration =
            GuardedFile::open(MIGRATION_PATH).map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let mut audit_sql =
            GuardedFile::open(AUDIT_SQL_PATH).map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let executable = GuardedExecutable::open().map_err(|_| ErrorCode::ReaderIncompatible)?;
        if !is_elf(&mut api_image.file).map_err(|_| ErrorCode::ReaderIncompatible)?
            || !is_elf(&mut worker_image.file).map_err(|_| ErrorCode::ReaderIncompatible)?
        {
            return Err(ErrorCode::ReaderIncompatible);
        }
        let source_matches = operations_schema
            .digest_bounded(MAX_SOURCE_FILE_BYTES as u64)
            .is_ok_and(|value| value == capability_value["operations_schema_sha256"])
            && archive_schema
                .digest_bounded(MAX_SOURCE_FILE_BYTES as u64)
                .is_ok_and(|value| value == capability_value["archive_schema_sha256"])
            && migration
                .digest_bounded(MAX_SOURCE_FILE_BYTES as u64)
                .is_ok_and(|value| value == capability_value["migration_0068_sha256"])
            && audit_sql
                .digest_bounded(MAX_SOURCE_FILE_BYTES as u64)
                .is_ok_and(|value| value == capability_value["audit_sql_sha256"]);
        let api_sha256 = api_image
            .digest_bounded(MAX_ELF_BYTES)
            .map_err(|_| ErrorCode::ReaderIncompatible)?;
        let worker_sha256 = worker_image
            .digest_bounded(MAX_ELF_BYTES)
            .map_err(|_| ErrorCode::ReaderIncompatible)?;
        let compiled_commit =
            option_env!("LAGRANGE_CODE_COMMIT").ok_or(ErrorCode::EvidenceUnavailable)?;
        if !source_matches
            || capability_value["release_manifest_sha256"] != intent.release_manifest_sha256
            || capability_value["code_commit"] != compiled_commit
            || capability_value["code_commit"] != process.value["code_commit"]
            || capability_value["api_binary_sha256"] != process.value["api_binary_sha256"]
            || capability_value["worker_binary_sha256"] != process.value["worker_binary_sha256"]
            || capability_value["native_binary_sha256"] != executable.sha256
            || api_sha256 != process.value["api_binary_sha256"]
            || worker_sha256 != process.value["worker_binary_sha256"]
            || process.value["release_manifest_sha256"] != intent.release_manifest_sha256
        {
            return Err(ErrorCode::IdentityConflict);
        }
        capability
            .revalidate()
            .map_err(|_| ErrorCode::ReaderIncompatible)?;
        Ok(Self {
            capability,
            api_image,
            worker_image,
            operations_schema,
            archive_schema,
            migration,
            audit_sql,
            executable,
        })
    }

    fn revalidate(&self) -> Result<(), ErrorCode> {
        for file in [
            &self.capability,
            &self.api_image,
            &self.worker_image,
            &self.operations_schema,
            &self.archive_schema,
            &self.migration,
            &self.audit_sql,
        ] {
            file.revalidate()
                .map_err(|_| ErrorCode::ReaderIncompatible)?;
        }
        self.executable
            .revalidate()
            .map_err(|_| ErrorCode::ReaderIncompatible)
    }
}

struct NativeEnableGuard<'a> {
    inputs: &'a ProtectedInputs,
    output: &'a GuardedDirectory,
    pins: &'a EnablePins,
}

#[cfg(target_os = "linux")]
impl crate::owner_beta::paper::evidence::NativePaperActionGuard for NativeEnableGuard<'_> {
    fn revalidate(
        &mut self,
    ) -> Result<(), crate::owner_beta::paper::evidence::financial::PaperInputActionError> {
        let result = self.revalidate_native();
        result.map_err(|_| {
            crate::owner_beta::paper::evidence::financial::PaperInputActionError::Runtime
        })
    }

    fn checkpoint(
        &mut self,
    ) -> Result<(), crate::owner_beta::paper::evidence::financial::PaperInputActionError> {
        self.revalidate()
    }
}

impl NativeEnableGuard<'_> {
    fn revalidate_native(&self) -> Result<(), ErrorCode> {
        if !env_is_clean() {
            return Err(ErrorCode::WireInvalid);
        }
        self.inputs
            .request
            .revalidate()
            .map_err(|_| ErrorCode::ReaderIncompatible)?;
        self.inputs
            .database
            .revalidate()
            .map_err(|_| ErrorCode::ReaderIncompatible)?;
        if let Some(evidence) = &self.inputs.evidence {
            evidence
                .revalidate()
                .map_err(|_| ErrorCode::ReaderIncompatible)?;
        }
        self.output
            .revalidate()
            .map_err(|_| ErrorCode::ReaderIncompatible)?;
        self.pins.revalidate()
    }
}

fn parse_database(bytes: &[u8]) -> Result<DatabaseSelector, ()> {
    let value = wire::parse_canonical(bytes)?;
    let object = value.as_object().ok_or(())?;
    let expected = ["schema", "host", "port", "database"];
    if object.len() != expected.len()
        || expected.iter().any(|key| !object.contains_key(*key))
        || value["schema"] != "owner-beta-paper-ops-database-v1"
        || value["port"].as_i64() != Some(5432)
    {
        return Err(());
    }
    let host = value["host"].as_str().ok_or(())?;
    if host != "postgres" && host != "/run/postgresql" {
        return Err(());
    }
    let database = value["database"].as_str().ok_or(())?;
    let mut chars = database.bytes();
    if database.is_empty()
        || database.len() > 63
        || !chars
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        || !chars.all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err(());
    }
    Ok(DatabaseSelector {
        host: host.to_owned(),
        database: database.to_owned(),
    })
}

async fn connect(selector: &DatabaseSelector, role: DatabaseRole) -> Result<PgPool, ErrorCode> {
    let mut password_file =
        GuardedFile::open(role.password_path()).map_err(|_| ErrorCode::ReaderIncompatible)?;
    let password_bytes = password_file
        .read_bounded(MAX_PASSWORD_BYTES)
        .map_err(|_| ErrorCode::ReaderIncompatible)?;
    password_file
        .revalidate()
        .map_err(|_| ErrorCode::ReaderIncompatible)?;
    let password =
        std::str::from_utf8(&password_bytes).map_err(|_| ErrorCode::ReaderIncompatible)?;
    if password.is_empty() || password.bytes().any(|b| b == 0 || b == b'\n' || b == b'\r') {
        return Err(ErrorCode::ReaderIncompatible);
    }
    let options = PgConnectOptions::new_without_pgpass()
        .host(&selector.host)
        .port(5432)
        .database(&selector.database)
        .username(role.name())
        .password(password)
        .application_name("owner-beta-paper-ops-native")
        .options([("statement_timeout", "15000"), ("lock_timeout", "5000")]);
    tokio::time::timeout(
        DB_CONNECT_TIMEOUT,
        PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options),
    )
    .await
    .map_err(|_| ErrorCode::ReaderIncompatible)?
    .map_err(|_| ErrorCode::ReaderIncompatible)
}

async fn commit_bounded(transaction: sqlx::Transaction<'_, sqlx::Postgres>) -> Result<(), ()> {
    tokio::time::timeout(DB_COMMIT_TIMEOUT, transaction.commit())
        .await
        .map_err(|_| ())?
        .map_err(|_| ())
}

async fn rollback_bounded(transaction: sqlx::Transaction<'_, sqlx::Postgres>) -> bool {
    tokio::time::timeout(Duration::from_secs(5), transaction.rollback())
        .await
        .is_ok_and(|result| result.is_ok())
}

fn read_fixed(path: &'static str, limit: usize) -> Result<(GuardedFile, Vec<u8>), GuardError> {
    let mut file = GuardedFile::open(path)?;
    let bytes = file.read_bounded(limit)?;
    Ok((file, bytes))
}

fn map_sql_error(error: &sqlx::Error, verb: Verb) -> ErrorCode {
    if let sqlx::Error::Database(db) = error {
        match db.code().as_deref() {
            Some("22023") => ErrorCode::WireInvalid,
            Some("23505") => ErrorCode::IdentityConflict,
            Some("40001") => ErrorCode::StaleEpoch,
            Some("42501") if verb == Verb::Recover => ErrorCode::RecoveryAuthorityUnavailable,
            Some("42501") => ErrorCode::ReaderIncompatible,
            Some("55000") => ErrorCode::EvidenceUnavailable,
            _ => ErrorCode::InternalFailure,
        }
    } else {
        ErrorCode::InternalFailure
    }
}

fn sql_outcome(
    value: &Value,
    action: &str,
) -> Result<(Outcome, Option<String>, Option<i64>), ErrorCode> {
    if !exact_keys(
        value,
        &[
            "schema",
            "action",
            "outcome",
            "control_epoch",
            "account_id",
            "data",
        ],
    ) || value["schema"] != "owner-beta-paper-action-v1"
        || value["action"] != action
    {
        return Err(ErrorCode::InternalFailure);
    }
    let outcome = match value["outcome"].as_str() {
        Some("OK") => Outcome::Ok,
        Some("REPLAY") => Outcome::Replay,
        Some("NOT_FOUND") => return Err(ErrorCode::EvidenceUnavailable),
        _ => return Err(ErrorCode::EvidenceUnavailable),
    };
    let data = value.get("data").ok_or(ErrorCode::InternalFailure)?;
    if !(value["control_epoch"].is_null() || nonnegative_i64(&value["control_epoch"]))
        || !optional_canonical_uuid(&value["account_id"])
        || !data.is_object()
    {
        return Err(ErrorCode::InternalFailure);
    }
    let event_id = data
        .get("event_id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let epoch = value.get("control_epoch").and_then(Value::as_i64);
    if event_id
        .as_ref()
        .is_some_and(|id| !canonical_uuid(&Value::String(id.clone())))
    {
        return Err(ErrorCode::InternalFailure);
    }
    Ok((outcome, event_id, epoch))
}

fn configure_outcome(value: &Value) -> Result<(Outcome, String, i64), ErrorCode> {
    let (outcome, event_id, epoch) = sql_outcome(value, "CONFIGURE")?;
    let event_id = event_id.ok_or(ErrorCode::InternalFailure)?;
    let epoch = epoch.ok_or(ErrorCode::InternalFailure)?;
    let data = &value["data"];
    let state = data
        .get("state")
        .and_then(Value::as_str)
        .ok_or(ErrorCode::InternalFailure)?;
    if !["OFF", "ENABLED", "DRAINING", "RESTORED_OFF", "BLOCKED"].contains(&state) {
        return Err(ErrorCode::InternalFailure);
    }
    match outcome {
        Outcome::Ok => {
            if !exact_keys(
                data,
                &[
                    "account_id",
                    "event_id",
                    "state",
                    "initial_cash",
                    "currency",
                ],
            ) || !canonical_uuid(&data["account_id"])
                || data["account_id"] != value["account_id"]
                || data["event_id"].as_str() != Some(event_id.as_str())
                || data["initial_cash"] != "10000000.0000"
                || data["currency"] != "KRW"
                || epoch != 0
            {
                return Err(ErrorCode::InternalFailure);
            }
        }
        Outcome::Replay => {
            if !exact_keys(data, &["event_id", "state"])
                || data["event_id"].as_str() != Some(event_id.as_str())
                || !canonical_uuid(&value["account_id"])
            {
                return Err(ErrorCode::InternalFailure);
            }
        }
        _ => return Err(ErrorCode::InternalFailure),
    }
    Ok((outcome, event_id, epoch))
}

fn begin_outcome(value: &Value, intent: &Intent) -> Result<(Outcome, String, i64), ErrorCode> {
    if !exact_keys(
        value,
        &[
            "schema",
            "action",
            "outcome",
            "control_epoch",
            "account_id",
            "data",
        ],
    ) || value["schema"] != "owner-beta-paper-action-v1"
        || value["action"] != "BEGIN_OPERATION"
    {
        return Err(ErrorCode::InternalFailure);
    }
    let outcome = match value["outcome"].as_str() {
        Some("OK") => Outcome::Ok,
        Some("REPLAY") => Outcome::Replay,
        _ => return Err(ErrorCode::EvidenceUnavailable),
    };
    let data = &value["data"];
    if !exact_keys(
        data,
        &[
            "generation",
            "operation_event_id",
            "intent_sha256",
            "fenced_epoch",
        ],
    ) {
        return Err(ErrorCode::InternalFailure);
    }
    let generation = intent.generation.to_string();
    let account = intent.account_id.map(|id| id.to_string());
    let expected_floor = intent.expected_epoch.max(intent.local_epoch_floor);
    let next_epoch = expected_floor.checked_add(1).ok_or(ErrorCode::StaleEpoch)?;
    let epoch = value["control_epoch"]
        .as_i64()
        .ok_or(ErrorCode::InternalFailure)?;
    let canonical_intent = super::super::source_evidence::canonical(&intent.value)
        .map_err(|_| ErrorCode::WireInvalid)?;
    if !canonical_uuid(&data["generation"])
        || data["generation"].as_str() != Some(generation.as_str())
        || !canonical_uuid(&data["operation_event_id"])
        || data["intent_sha256"] != super::super::source_evidence::digest(&canonical_intent)
        || data["fenced_epoch"].as_i64() != Some(next_epoch)
        || (outcome == Outcome::Ok && epoch != next_epoch)
        || (outcome == Outcome::Replay && epoch < next_epoch)
        || value["account_id"].as_str() != account.as_deref()
    {
        return Err(ErrorCode::IdentityConflict);
    }
    Ok((
        outcome,
        data["operation_event_id"]
            .as_str()
            .ok_or(ErrorCode::InternalFailure)?
            .to_owned(),
        epoch,
    ))
}

fn exact_keys(value: &Value, expected: &[&str]) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.len() == expected.len() && expected.iter().all(|key| object.contains_key(*key))
}

fn canonical_uuid(value: &Value) -> bool {
    value.as_str().is_some_and(|text| {
        Uuid::parse_str(text).is_ok_and(|id| !id.is_nil() && id.to_string() == text)
    })
}

fn optional_canonical_uuid(value: &Value) -> bool {
    value.is_null() || canonical_uuid(value)
}

fn nonnegative_i64(value: &Value) -> bool {
    value.as_i64().is_some_and(|number| number >= 0)
}

fn sha256_hex(value: &Value) -> bool {
    value.as_str().is_some_and(|text| {
        text.len() == 64
            && text
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn sha256_wire(value: &Value) -> bool {
    value.as_str().is_some_and(|text| {
        text.len() == 71
            && text.starts_with("sha256:")
            && text[7..]
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn inspect_page_next(page: &Value, intent: &Intent) -> Result<Option<Uuid>, ErrorCode> {
    if !exact_keys(
        page,
        &[
            "schema",
            "action",
            "outcome",
            "control_epoch",
            "account_id",
            "data",
        ],
    ) || page["schema"] != "owner-beta-paper-action-v1"
        || page["action"] != "INSPECT_OPERATION"
        || page["outcome"] != "OK"
    {
        return Err(ErrorCode::ReaderIncompatible);
    }
    let data = page.get("data").ok_or(ErrorCode::ReaderIncompatible)?;
    let expected_account = intent.account_id.map(|id| id.to_string());
    let expected_generation = intent.generation.to_string();
    let expected_installation = intent.installation_id.to_string();
    if !exact_keys(
        data,
        &[
            "generation",
            "state",
            "control_epoch",
            "history_present",
            "operation_event_id",
            "reconciliation",
            "targets",
            "next_target",
        ],
    ) {
        return Err(ErrorCode::ReaderIncompatible);
    }
    if !optional_canonical_uuid(&page["account_id"])
        || (intent.account_id.is_some()
            && page["account_id"].as_str() != expected_account.as_deref())
        || !(page["control_epoch"].is_null() || nonnegative_i64(&page["control_epoch"]))
        || !(data["control_epoch"].is_null() || nonnegative_i64(&data["control_epoch"]))
        || data["control_epoch"] != page["control_epoch"]
        || !(data["generation"].is_null() || canonical_uuid(&data["generation"]))
        || (intent.account_id.is_some()
            && data["generation"].as_str() != Some(expected_generation.as_str()))
        || !(data["state"].is_null()
            || ["OFF", "ENABLED", "DRAINING", "RESTORED_OFF", "BLOCKED"]
                .contains(&data["state"].as_str().unwrap_or_default()))
        || !data["history_present"].is_boolean()
        || !optional_canonical_uuid(&data["operation_event_id"])
        || (intent.account_id.is_some()
            && (data["history_present"] != true
                || data["operation_event_id"].is_null()
                || data["state"].is_null()))
    {
        return Err(ErrorCode::IdentityConflict);
    }
    if !data["reconciliation"].is_null() {
        let summary = &data["reconciliation"];
        if !exact_keys(
            summary,
            &[
                "schema",
                "installation_id",
                "generation",
                "account_id",
                "observation_id",
                "control_epoch",
                "inventory_sha256",
                "observed_at",
                "database_sha256",
                "ledger_sha256",
                "pending_sha256",
                "outbox_sha256",
                "jobs_sha256",
                "unresolved_commits",
                "live_claims",
                "held_targets",
                "quarantined_deliveries",
            ],
        ) || summary["schema"] != "owner-beta-paper-reconciliation-v1"
            || !canonical_uuid(&summary["installation_id"])
            || !canonical_uuid(&summary["generation"])
            || !canonical_uuid(&summary["account_id"])
            || !canonical_uuid(&summary["observation_id"])
            || summary["installation_id"].as_str() != Some(expected_installation.as_str())
            || (intent.account_id.is_some()
                && summary["account_id"].as_str() != expected_account.as_deref())
            || (intent.account_id.is_some()
                && summary["generation"].as_str() != Some(expected_generation.as_str()))
            || !nonnegative_i64(&summary["control_epoch"])
            || ![
                "inventory_sha256",
                "database_sha256",
                "ledger_sha256",
                "pending_sha256",
                "outbox_sha256",
                "jobs_sha256",
            ]
            .iter()
            .all(|key| sha256_wire(&summary[*key]))
            || !summary["observed_at"].as_str().is_some_and(|value| {
                chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%SZ").is_ok()
            })
            || ![
                "unresolved_commits",
                "live_claims",
                "held_targets",
                "quarantined_deliveries",
            ]
            .iter()
            .all(|key| nonnegative_i64(&summary[*key]))
        {
            return Err(ErrorCode::ReaderIncompatible);
        }
    }
    let targets = data["targets"]
        .as_array()
        .ok_or(ErrorCode::ReaderIncompatible)?;
    if targets.len() > INSPECT_PAGE_LIMIT as usize {
        return Err(ErrorCode::ReaderIncompatible);
    }
    for target in targets {
        if !exact_keys(
            target,
            &[
                "target_id",
                "lineage_id",
                "effective_session",
                "lineage_epoch",
                "target_status",
                "session_state",
                "settlement_sha256",
                "valuation_sha256",
                "ledger_before_sha256",
                "ledger_after_sha256",
                "account_state_version",
                "outbox_id",
            ],
        ) || !canonical_uuid(&target["target_id"])
            || !canonical_uuid(&target["lineage_id"])
            || !target["effective_session"]
                .as_str()
                .is_some_and(|date| chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_ok())
            || !nonnegative_i64(&target["lineage_epoch"])
            || !["PENDING", "EXECUTED", "SKIPPED"]
                .contains(&target["target_status"].as_str().unwrap_or_default())
            || ![
                "PENDING",
                "WAITING_PRICE",
                "WAITING_ADMISSION",
                "HELD",
                "BLOCKED",
                "RECOVERY_REQUIRED",
                "SETTLED_TRADE",
                "SETTLED_NO_TRADE",
                "VALUED",
            ]
            .contains(&target["session_state"].as_str().unwrap_or_default())
            || ![
                "settlement_sha256",
                "valuation_sha256",
                "ledger_before_sha256",
                "ledger_after_sha256",
            ]
            .iter()
            .all(|key| target[*key].is_null() || sha256_hex(&target[*key]))
            || !(target["account_state_version"].is_null()
                || nonnegative_i64(&target["account_state_version"]))
            || !optional_canonical_uuid(&target["outbox_id"])
        {
            return Err(ErrorCode::ReaderIncompatible);
        }
    }
    match &data["next_target"] {
        Value::Null => Ok(None),
        Value::String(value) if canonical_uuid(&data["next_target"]) => {
            let last = targets
                .last()
                .and_then(|target| target["target_id"].as_str())
                .ok_or(ErrorCode::ReaderIncompatible)?;
            if value != last || targets.len() != INSPECT_PAGE_LIMIT as usize {
                return Err(ErrorCode::ReaderIncompatible);
            }
            Ok(Some(
                Uuid::parse_str(value).map_err(|_| ErrorCode::ReaderIncompatible)?,
            ))
        }
        _ => Err(ErrorCode::ReaderIncompatible),
    }
}

async fn load_context(
    verb: Verb,
) -> Result<
    (
        ProtectedInputs,
        Intent,
        Option<Artifact>,
        GuardedDirectory,
        DatabaseSelector,
    ),
    ResultWire,
> {
    let (request_guard, request_bytes) = read_fixed(REQUEST_PATH, wire::MAX_WIRE_BYTES)
        .map_err(|_| ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible))?;
    let intent = wire::parse_intent(&request_bytes, verb)
        .map_err(|_| ResultWire::new(verb, Outcome::Blocked, ErrorCode::WireInvalid))?;
    request_guard
        .revalidate()
        .map_err(|_| ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible))?;
    let (evidence, evidence_guard) = match (intent.artifact_sha256.as_ref(), verb) {
        (None, Verb::Inspect | Verb::Begin) => (None, None),
        (Some(_), _) => {
            let (evidence_guard, bytes) =
                read_fixed(EVIDENCE_PATH, wire::MAX_WIRE_BYTES).map_err(|_| {
                    ResultWire::new(verb, Outcome::Blocked, ErrorCode::EvidenceUnavailable)
                })?;
            let schema = match verb {
                Verb::Reconcile => "owner-beta-paper-writer-inventory-v1",
                Verb::Off => "owner-beta-paper-drain-receipt-v1",
                Verb::RestoreOff => "owner-beta-paper-restore-receipt-v1",
                Verb::Recover => "owner-beta-paper-recovery-v2",
                Verb::Configure => "owner-beta-paper-configure-operation-v1",
                Verb::Enable => "owner-beta-paper-process-v1",
                _ => {
                    return Err(ResultWire::new(
                        verb,
                        Outcome::Blocked,
                        ErrorCode::WireInvalid,
                    ));
                }
            };
            let artifact = if verb == Verb::Configure {
                let value = wire::parse_configure(&bytes)
                    .map_err(|_| ResultWire::new(verb, Outcome::Blocked, ErrorCode::WireInvalid))?;
                let sha256 = digest(&bytes);
                if intent.artifact_sha256.as_deref() != Some(sha256.as_str()) {
                    return Err(ResultWire::new(
                        verb,
                        Outcome::Blocked,
                        ErrorCode::WireInvalid,
                    ));
                }
                Artifact { value, sha256 }
            } else if verb == Verb::Enable {
                wire::parse_process_binding(&bytes, &intent)
                    .map_err(|_| ResultWire::new(verb, Outcome::Blocked, ErrorCode::WireInvalid))?
            } else {
                wire::parse_artifact(&bytes, schema, &intent)
                    .map_err(|_| ResultWire::new(verb, Outcome::Blocked, ErrorCode::WireInvalid))?
            };
            if verb == Verb::Reconcile {
                let writers = artifact
                    .value
                    .get("writers")
                    .and_then(Value::as_array)
                    .ok_or_else(|| {
                        ResultWire::new(verb, Outcome::Blocked, ErrorCode::WireInvalid)
                    })?;
                if writers.iter().any(|writer| {
                    writer.get("writer_class").and_then(Value::as_str) == Some("OTHER_DB_WRITER")
                }) {
                    return Err(ResultWire::new(
                        verb,
                        Outcome::Blocked,
                        ErrorCode::WriterUnknown,
                    ));
                }
                let covered = writers
                    .iter()
                    .filter_map(|writer| writer.get("writer_class").and_then(Value::as_str))
                    .collect::<std::collections::BTreeSet<_>>();
                let mandatory = ["API", "PAPER_RUNNER", "PAPER_DELIVERY"];
                if mandatory.iter().any(|class| !covered.contains(*class)) {
                    return Err(ResultWire::new(
                        verb,
                        Outcome::Blocked,
                        ErrorCode::EvidenceUnavailable,
                    ));
                }
            }
            if matches!(verb, Verb::Reconcile | Verb::Off | Verb::RestoreOff) {
                check_clock(
                    &artifact.value["observed"],
                    artifact.value["boot_id"].as_str().unwrap_or_default(),
                )
                .map_err(|code| ResultWire::new(verb, Outcome::Blocked, code))?;
            }
            evidence_guard.revalidate().map_err(|_| {
                ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible)
            })?;
            (Some(artifact), Some(evidence_guard))
        }
        _ => {
            return Err(ResultWire::new(
                verb,
                Outcome::Blocked,
                ErrorCode::WireInvalid,
            ));
        }
    };
    let (db_guard, db_bytes) = read_fixed(DATABASE_PATH, MAX_DB_SELECTOR)
        .map_err(|_| ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible))?;
    let selector = parse_database(&db_bytes)
        .map_err(|_| ResultWire::new(verb, Outcome::Blocked, ErrorCode::WireInvalid))?;
    db_guard
        .revalidate()
        .map_err(|_| ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible))?;
    let output = GuardedDirectory::open(OUTPUT_PATH)
        .map_err(|_| ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible))?;
    if !env_is_clean() {
        return Err(ResultWire::new(
            verb,
            Outcome::Blocked,
            ErrorCode::WireInvalid,
        ));
    }
    Ok((
        ProtectedInputs {
            request: request_guard,
            database: db_guard,
            evidence: evidence_guard,
        },
        intent,
        evidence,
        output,
        selector,
    ))
}

pub(super) async fn run(verb: Verb) -> ResultWire {
    if unsafe { libc::getuid() } != 0 || unsafe { libc::geteuid() } != 0 {
        return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible);
    }
    if PRIVATE_DIR != "/run/owner-beta-paper-ops" {
        return ResultWire::new(verb, Outcome::Blocked, ErrorCode::InternalFailure);
    }
    let (inputs, intent, evidence, output_dir, selector) = match load_context(verb).await {
        Ok(context) => context,
        Err(result) => return result,
    };
    let result = if let Some(error) = super::native_follow_up_block(verb) {
        ResultWire::new(verb, Outcome::Blocked, error)
    } else {
        match verb {
            Verb::Configure => configure(&intent, evidence.as_ref(), &selector).await,
            Verb::Inspect => inspect(&intent, &selector, &output_dir).await,
            Verb::Enable => {
                enable(&intent, evidence.as_ref(), &selector, &inputs, &output_dir).await
            }
            Verb::Begin => begin(&intent, &selector).await,
            Verb::Reconcile | Verb::Off | Verb::RestoreOff | Verb::Recover => {
                ResultWire::new(verb, Outcome::Blocked, ErrorCode::InternalFailure)
            }
        }
    };
    let clock_expired = matches!(verb, Verb::Reconcile | Verb::Off | Verb::RestoreOff)
        && evidence.as_ref().is_some_and(|artifact| {
            check_clock(
                &artifact.value["observed"],
                artifact.value["boot_id"].as_str().unwrap_or_default(),
            )
            .is_err()
        });
    let recheck_succeeded = inputs.request.revalidate().is_ok()
        && inputs.database.revalidate().is_ok()
        && !inputs
            .evidence
            .as_ref()
            .is_some_and(|file| file.revalidate().is_err())
        && output_dir.revalidate().is_ok()
        && !clock_expired;
    finalize_after_recheck(result, recheck_succeeded, verb)
}

struct EnableConsumer<'a> {
    intent: &'a Intent,
    process: &'a Artifact,
    pool: &'a PgPool,
    output: &'a GuardedDirectory,
    result: Option<ResultWire>,
    committed: bool,
    commit_started: bool,
}

#[cfg(target_os = "linux")]
impl crate::owner_beta::paper::evidence::NativePaperActionConsumer for EnableConsumer<'_> {
    fn consume<'a, 'hold>(
        &'a mut self,
        action: &'a mut crate::owner_beta::paper::evidence::financial::CheckedPaperAction<'hold>,
    ) -> crate::owner_beta::paper::evidence::financial::LocalInputFuture<
        'a,
        Result<(), crate::owner_beta::paper::evidence::financial::PaperInputActionError>,
    >
    where
        'hold: 'a,
    {
        Box::pin(async move {
            let result = self.enable(action).await;
            let code = match result.outcome {
                Outcome::Ok | Outcome::Replay => None,
                Outcome::CommitUnknown => {
                    Some(crate::owner_beta::paper::evidence::financial::PaperInputActionError::CommitUnknown)
                }
                Outcome::Blocked => Some(
                    crate::owner_beta::paper::evidence::financial::PaperInputActionError::Runtime,
                ),
            };
            self.result = Some(result);
            match code {
                None => Ok(()),
                Some(error) => Err(error),
            }
        })
    }
}

#[cfg(target_os = "linux")]
impl EnableConsumer<'_> {
    async fn enable(
        &mut self,
        action: &mut crate::owner_beta::paper::evidence::financial::CheckedPaperAction<'_>,
    ) -> ResultWire {
        let verb = Verb::Enable;
        if action.revalidate_files().is_err() {
            return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible);
        }
        let inputs = action.inputs();
        let current_binding = inputs.current_binding();
        let cohort = &current_binding["cohort"];
        let Some(compiled_commit) = option_env!("LAGRANGE_CODE_COMMIT") else {
            return ResultWire::new(verb, Outcome::Blocked, ErrorCode::EvidenceUnavailable);
        };
        if !process_matches_current(&self.process.value, self.intent, cohort, compiled_commit) {
            return ResultWire::new(verb, Outcome::Blocked, ErrorCode::IdentityConflict);
        }
        let sessions = inputs.native_enable_sessions();
        if !valid_native_enable_sessions(sessions) {
            return ResultWire::new(verb, Outcome::Blocked, ErrorCode::EvidenceUnavailable);
        }
        let now = chrono::Utc::now();
        if inputs.evaluated_at() > now
            || now
                .signed_duration_since(inputs.evaluated_at())
                .num_seconds()
                >= 180
            || inputs.expires_at() <= now
        {
            return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ClockInvalid);
        }
        let Some(account_id) = self.intent.account_id else {
            return ResultWire::new(verb, Outcome::Blocked, ErrorCode::IdentityConflict);
        };
        let Some(new_epoch) = self.intent.expected_epoch.checked_add(1) else {
            return ResultWire::new(verb, Outcome::Blocked, ErrorCode::StaleEpoch);
        };
        let evidence = json!({
            "current_cohort": cohort.clone(),
            "process_binding": self.process.value.clone(),
            "process_binding_sha256": self.process.sha256.clone(),
            "evaluated_at": inputs.evaluated_at().to_rfc3339_opts(
                chrono::SecondsFormat::Nanos,
                true,
            ),
            "expires_at": inputs.expires_at().to_rfc3339_opts(
                chrono::SecondsFormat::Nanos,
                true,
            ),
            "session_evidence": sessions.to_vec(),
            "expected_epoch": self.intent.expected_epoch,
            "new_epoch": new_epoch,
        });
        let evidence_bytes = match super::super::source_evidence::canonical(&evidence) {
            Ok(bytes) if bytes.len() <= wire::MAX_WIRE_BYTES => bytes,
            _ => return ResultWire::new(verb, Outcome::Blocked, ErrorCode::WireInvalid),
        };
        if !ENABLE_SQL_RESET_AND_READBACK_READY {
            return ResultWire::new(verb, Outcome::Blocked, ErrorCode::EvidenceUnavailable);
        }
        if action.revalidate_files().is_err() {
            return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible);
        }
        let mut transaction =
            match tokio::time::timeout(Duration::from_secs(10), self.pool.begin()).await {
                Ok(Ok(transaction)) => transaction,
                _ => return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible),
            };
        let response = tokio::time::timeout(
            Duration::from_secs(20),
            sqlx::query_scalar::<_, Value>(
                "SELECT public.transition_owner_beta_paper_v1($1::bigint,'ENABLE',$2::jsonb,$3::text)",
            )
            .bind(self.intent.expected_epoch)
            .bind(evidence.clone())
            .bind(&self.intent.request_key)
            .fetch_one(&mut *transaction),
        )
        .await;
        let response = match response {
            Ok(Ok(value)) => value,
            Ok(Err(error)) => {
                let _ = rollback_bounded(transaction).await;
                return ResultWire::new(verb, Outcome::Blocked, map_sql_error(&error, verb));
            }
            Err(_) => {
                let _ = rollback_bounded(transaction).await;
                return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible);
            }
        };
        let parsed = match enable_outcome(&response, account_id, new_epoch) {
            Ok(parsed) => parsed,
            Err(code) => {
                let _ = rollback_bounded(transaction).await;
                return ResultWire::new(verb, Outcome::Blocked, code);
            }
        };
        // The function has acquired and retained its SQL row/advisory locks;
        // this is the post-wait source/current check immediately before commit.
        if action.revalidate_files().is_err() {
            let _ = rollback_bounded(transaction).await;
            return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible);
        }
        if action.revalidate_files().is_err() {
            let _ = rollback_bounded(transaction).await;
            return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible);
        }
        self.commit_started = true;
        if commit_bounded(transaction).await.is_err() {
            return ResultWire::new(verb, Outcome::CommitUnknown, ErrorCode::CommitUnknown);
        }
        self.committed = true;
        if action.revalidate_files().is_err() {
            // The committed event is not treated as a clean rollback. This
            // exact SQL ABI has no read-only ENABLE event reader, so the
            // durable readback remains an explicit integration seam.
            return ResultWire::new(verb, Outcome::CommitUnknown, ErrorCode::CommitUnknown);
        }
        let response_bytes = match super::super::source_evidence::canonical(&response) {
            Ok(bytes) if bytes.len() <= wire::MAX_WIRE_BYTES => bytes,
            _ => return ResultWire::new(verb, Outcome::CommitUnknown, ErrorCode::CommitUnknown),
        };
        let response_hash = match self.output.write_content_addressed(&response_bytes) {
            Ok(hash) => hash,
            Err(_) => {
                return ResultWire::new(verb, Outcome::CommitUnknown, ErrorCode::CommitUnknown);
            }
        };
        let mut result = ResultWire::new(verb, parsed.0, ErrorCode::None);
        result.generation = Some(self.intent.generation.to_string());
        result.event_id = Some(parsed.1);
        result.control_epoch = Some(parsed.2);
        result.artifact_sha256 = Some(response_hash);
        result
    }
}

#[cfg(target_os = "linux")]
fn process_matches_current(
    process: &Value,
    intent: &Intent,
    cohort: &Value,
    compiled_commit: &str,
) -> bool {
    let Some(account_id) = intent.account_id else {
        return false;
    };
    process["account_id"] == account_id.to_string()
        && process["owner_user_id"] == cohort["owner_user_id"]
        && process["strategy_config_id"] == cohort["strategy_config_id"]
        && process["daily_cohort_sha256"] == cohort["cohort_sha256"]
        && process["release_manifest_sha256"] == intent.release_manifest_sha256
        && cohort["release_manifest_sha256"] == intent.release_manifest_sha256
        && process["code_commit"] == compiled_commit
        && cohort["verifier_revision"] == compiled_commit
        && process["control_epoch"].as_i64() == intent.expected_epoch.checked_add(1)
}

#[cfg(target_os = "linux")]
fn valid_native_enable_sessions(sessions: &[Value]) -> bool {
    if sessions.len() != 3 {
        return false;
    }
    let keys = [
        "session_date",
        "run_id",
        "admission_sha256",
        "source_event_sha256",
        "root_event_sha256",
    ];
    let mut previous: Option<chrono::NaiveDate> = None;
    for session in sessions {
        if !exact_keys(session, &keys)
            || !canonical_uuid(&session["run_id"])
            || !sha256_wire(&session["admission_sha256"])
            || !sha256_wire(&session["source_event_sha256"])
            || !sha256_wire(&session["root_event_sha256"])
        {
            return false;
        }
        let Some(text) = session["session_date"].as_str() else {
            return false;
        };
        let Ok(date) = chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d") else {
            return false;
        };
        if date.to_string() != text || previous.is_some_and(|value| value >= date) {
            return false;
        }
        previous = Some(date);
    }
    true
}

fn enable_outcome(
    response: &Value,
    account_id: Uuid,
    expected_epoch: i64,
) -> Result<(Outcome, String, i64), ErrorCode> {
    let (outcome, event_id, epoch) = sql_outcome(response, "TRANSITION")?;
    let event_id = event_id.ok_or(ErrorCode::InternalFailure)?;
    let epoch = epoch.ok_or(ErrorCode::InternalFailure)?;
    let data = &response["data"];
    if !(exact_keys(data, &["event_id", "state"])
        || exact_keys(data, &["event_id", "state", "pending_target_id"]))
        || data["event_id"].as_str() != Some(event_id.as_str())
        || !matches!(data["state"].as_str(), Some("ENABLED"))
        || response["account_id"] != account_id.to_string()
        || epoch != expected_epoch
    {
        return Err(ErrorCode::IdentityConflict);
    }
    if data
        .get("pending_target_id")
        .is_some_and(|value| !value.is_null())
    {
        return Err(ErrorCode::InternalFailure);
    }
    Ok((outcome, event_id, epoch))
}

#[cfg(target_os = "linux")]
async fn enable(
    intent: &Intent,
    process: Option<&Artifact>,
    selector: &DatabaseSelector,
    inputs: &ProtectedInputs,
    output: &GuardedDirectory,
) -> ResultWire {
    let verb = Verb::Enable;
    let Some(process) = process else {
        return ResultWire::new(verb, Outcome::Blocked, ErrorCode::EvidenceUnavailable);
    };
    if intent.purpose != "ENABLE" {
        return ResultWire::new(verb, Outcome::Blocked, ErrorCode::WireInvalid);
    }
    let pins = match EnablePins::open(intent, process) {
        Ok(pins) => pins,
        Err(code) => return ResultWire::new(verb, Outcome::Blocked, code),
    };
    let worker_pool = match connect(selector, DatabaseRole::Worker).await {
        Ok(pool) => pool,
        Err(code) => return ResultWire::new(verb, Outcome::Blocked, code),
    };
    let migration_pool = match connect(selector, DatabaseRole::MigrationOwner).await {
        Ok(pool) => pool,
        Err(code) => {
            let _ = tokio::time::timeout(Duration::from_secs(5), worker_pool.close()).await;
            return ResultWire::new(verb, Outcome::Blocked, code);
        }
    };
    let mut guard = NativeEnableGuard {
        inputs,
        output,
        pins: &pins,
    };
    let mut consumer = EnableConsumer {
        intent,
        process,
        pool: &migration_pool,
        output,
        result: None,
        committed: false,
        commit_started: false,
    };
    let completion = crate::owner_beta::paper::evidence::with_native_checked_paper_action(
        &worker_pool,
        &mut guard,
        &mut consumer,
    )
    .await;
    let mut result = consumer
        .result
        .take()
        .unwrap_or_else(|| ResultWire::new(verb, Outcome::Blocked, ErrorCode::EvidenceUnavailable));
    let recheck_before_close = guard.revalidate_native().is_ok()
        && pins.revalidate().is_ok()
        && inputs.request.revalidate().is_ok()
        && inputs.database.revalidate().is_ok()
        && inputs
            .evidence
            .as_ref()
            .is_none_or(|file| file.revalidate().is_ok())
        && output.revalidate().is_ok();
    let worker_closed = tokio::time::timeout(Duration::from_secs(5), worker_pool.close())
        .await
        .is_ok();
    let migration_closed = tokio::time::timeout(Duration::from_secs(5), migration_pool.close())
        .await
        .is_ok();
    let recheck = recheck_before_close
        && guard.revalidate_native().is_ok()
        && pins.revalidate().is_ok()
        && inputs.request.revalidate().is_ok()
        && inputs.database.revalidate().is_ok()
        && inputs
            .evidence
            .as_ref()
            .is_none_or(|file| file.revalidate().is_ok())
        && output.revalidate().is_ok();
    let consumer_committed = consumer.committed;
    let committed_or_unknown = consumer_committed
        || consumer.commit_started
        || completion.committed
        || completion.commit_started;
    if committed_or_unknown
        && (completion.first.is_some()
            || !completion.secondary.is_empty()
            || !recheck
            || !worker_closed
            || !migration_closed)
    {
        result = ResultWire::new(verb, Outcome::CommitUnknown, ErrorCode::CommitUnknown);
    } else if !committed_or_unknown && (!worker_closed || !migration_closed) {
        result = ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible);
    }
    finalize_after_recheck(result, recheck && worker_closed && migration_closed, verb)
}

async fn configure(
    intent: &Intent,
    artifact: Option<&Artifact>,
    selector: &DatabaseSelector,
) -> ResultWire {
    let verb = Verb::Configure;
    let Some(artifact) = artifact else {
        return ResultWire::new(verb, Outcome::Blocked, ErrorCode::EvidenceUnavailable);
    };
    let Ok(pool) = connect(selector, DatabaseRole::MigrationOwner).await else {
        return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible);
    };
    finish_with_pool!(pool, verb, {
        let mut tx = match pool.begin().await {
            Ok(tx) => tx,
            Err(_) => {
                return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible);
            }
        };
        let response = sqlx::query_scalar::<_, Value>(
        "SELECT public.configure_owner_beta_paper_v1($1::uuid,$2::text,$3::jsonb,NULL,$4::text)",
    )
    .bind(
        artifact.value["strategy_config_id"]
            .as_str()
            .unwrap_or_default(),
    )
    .bind("Owner-beta Paper")
    .bind(artifact.value["contract"].clone())
    .bind(&intent.request_key)
    .fetch_one(&mut *tx)
    .await;
        let response = match response {
            Ok(value) => value,
            Err(error) => {
                let _ = rollback_bounded(tx).await;
                return ResultWire::new(verb, Outcome::Blocked, map_sql_error(&error, verb));
            }
        };
        let (outcome, event_id, epoch) = match configure_outcome(&response) {
            Ok(v) => v,
            Err(code) => {
                let _ = rollback_bounded(tx).await;
                return ResultWire::new(verb, Outcome::Blocked, code);
            }
        };
        if commit_bounded(tx).await.is_err() {
            return ResultWire::new(verb, Outcome::CommitUnknown, ErrorCode::CommitUnknown);
        }
        let mut result = ResultWire::new(verb, outcome, ErrorCode::None);
        result.event_id = event_id;
        result.control_epoch = epoch;
        result.artifact_sha256 = Some(artifact.sha256.clone());
        result
    })
}

async fn begin(intent: &Intent, selector: &DatabaseSelector) -> ResultWire {
    let verb = Verb::Begin;
    let Ok(pool) = connect(selector, DatabaseRole::MigrationOwner).await else {
        return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible);
    };
    finish_with_pool!(pool, verb, {
        let mut tx = match pool.begin().await {
            Ok(tx) => tx,
            Err(_) => {
                return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible);
            }
        };
        let response = sqlx::query_scalar::<_, Value>(
            "SELECT public.begin_owner_beta_paper_operation_v1($1::bigint,$2::jsonb,$3::text)",
        )
        .bind(intent.expected_epoch)
        .bind(intent.value.clone())
        .bind(&intent.request_key)
        .fetch_one(&mut *tx)
        .await;
        let response = match response {
            Ok(value) => value,
            Err(error) => {
                let _ = rollback_bounded(tx).await;
                return ResultWire::new(verb, Outcome::Blocked, map_sql_error(&error, verb));
            }
        };
        let (outcome, event_id, epoch) = match begin_outcome(&response, intent) {
            Ok(value) => value,
            Err(code) => {
                let _ = rollback_bounded(tx).await;
                return ResultWire::new(verb, Outcome::Blocked, code);
            }
        };
        let mut result = ResultWire::new(verb, outcome, ErrorCode::None);
        result.generation = Some(intent.generation.to_string());
        result.event_id = event_id;
        result.control_epoch = Some(epoch);
        if commit_bounded(tx).await.is_err() {
            return ResultWire::new(verb, Outcome::CommitUnknown, ErrorCode::CommitUnknown);
        }
        result
    })
}

async fn inspect(
    intent: &Intent,
    selector: &DatabaseSelector,
    output_dir: &GuardedDirectory,
) -> ResultWire {
    let verb = Verb::Inspect;
    let Ok(pool) = connect(selector, DatabaseRole::MigrationOwner).await else {
        return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible);
    };
    finish_with_pool!(pool, verb, {
        let mut tx = match pool.begin().await {
            Ok(tx) => tx,
            Err(_) => {
                return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible);
            }
        };
        if sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .is_err()
        {
            let _ = rollback_bounded(tx).await;
            return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible);
        }
        let generation = intent.account_id.map(|_| intent.generation);
        let mut cursor: Option<Uuid> = None;
        let mut page_hashes = Vec::new();
        let mut last_id: Option<String> = None;
        let mut seen_cursors = std::collections::BTreeSet::new();
        let mut seen_page_hashes = std::collections::BTreeSet::new();
        let mut epoch: Option<i64> = None;
        let mut final_event = None;
        let mut observed_state: Option<String> = None;
        for _ in 0..wire::MAX_PAGES {
            let page = sqlx::query_scalar::<_, Value>(
            "SELECT public.inspect_owner_beta_paper_operation_v1($1::uuid,$2::uuid,$3::integer)",
        )
        .bind(generation)
        .bind(cursor)
        .bind(INSPECT_PAGE_LIMIT)
        .fetch_one(&mut *tx)
        .await;
            let page = match page {
                Ok(value) => value,
                Err(error) => {
                    let _ = rollback_bounded(tx).await;
                    return ResultWire::new(verb, Outcome::Blocked, map_sql_error(&error, verb));
                }
            };
            let next_cursor = match inspect_page_next(&page, intent) {
                Ok(value) => value,
                Err(code) => {
                    let _ = rollback_bounded(tx).await;
                    return ResultWire::new(verb, Outcome::Blocked, code);
                }
            };
            let data = &page["data"];
            let Some(targets) = data.get("targets").and_then(Value::as_array) else {
                let _ = rollback_bounded(tx).await;
                return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible);
            };
            let current_epoch = page["control_epoch"].as_i64();
            if epoch.is_some() && epoch != current_epoch {
                let _ = rollback_bounded(tx).await;
                return ResultWire::new(verb, Outcome::Blocked, ErrorCode::IdentityConflict);
            }
            epoch = current_epoch;
            observed_state = data.get("state").and_then(Value::as_str).map(str::to_owned);
            final_event = data
                .get("operation_event_id")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let bytes = match super::super::source_evidence::canonical(&page) {
                Ok(bytes) if bytes.len() <= MAX_INSPECT_PAGE_BYTES => bytes,
                _ => {
                    let _ = rollback_bounded(tx).await;
                    return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible);
                }
            };
            let calculated_hash = digest(&bytes);
            if !seen_page_hashes.insert(calculated_hash) {
                let _ = rollback_bounded(tx).await;
                return ResultWire::new(verb, Outcome::Blocked, ErrorCode::IdentityConflict);
            }
            let page_hash = match output_dir.write_content_addressed(&bytes) {
                Ok(hash) => hash,
                Err(_) => {
                    let _ = rollback_bounded(tx).await;
                    return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible);
                }
            };
            page_hashes.push(page_hash);
            for target in targets {
                let Some(id) = target.get("target_id").and_then(Value::as_str) else {
                    let _ = rollback_bounded(tx).await;
                    return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible);
                };
                if !canonical_uuid(&target["target_id"])
                    || last_id.as_deref().is_some_and(|previous| previous >= id)
                {
                    let _ = rollback_bounded(tx).await;
                    return ResultWire::new(verb, Outcome::Blocked, ErrorCode::IdentityConflict);
                }
                last_id = Some(id.to_owned());
            }
            cursor = next_cursor;
            if let Some(cursor_id) = cursor {
                if !seen_cursors.insert(cursor_id) {
                    let _ = rollback_bounded(tx).await;
                    return ResultWire::new(verb, Outcome::Blocked, ErrorCode::IdentityConflict);
                }
            } else {
                break;
            }
            if page_hashes.len() == wire::MAX_PAGES {
                let _ = rollback_bounded(tx).await;
                return ResultWire::new(verb, Outcome::Blocked, ErrorCode::EvidenceUnavailable);
            }
        }
        if commit_bounded(tx).await.is_err() {
            return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible);
        }
        let pages_json = Value::Array(
            page_hashes
                .iter()
                .map(|hash| Value::String(hash.clone()))
                .collect(),
        );
        let page_root = match super::super::source_evidence::canonical(&pages_json) {
            Ok(bytes) => digest(&bytes),
            Err(_) => return ResultWire::new(verb, Outcome::Blocked, ErrorCode::InternalFailure),
        };
        let index = json!({"schema":"owner-beta-paper-operations-inspect-index-v1","generation":generation.map(|id| id.to_string()),"state":observed_state,"control_epoch":epoch,"operation_event_id":final_event,"root_sha256":page_root,"pages":pages_json});
        let index_bytes = match super::super::source_evidence::canonical(&index) {
            Ok(v) if v.len() <= 65_536 => v,
            _ => return ResultWire::new(verb, Outcome::Blocked, ErrorCode::EvidenceUnavailable),
        };
        let index_hash = match output_dir.write_content_addressed(&index_bytes) {
            Ok(value) => value,
            Err(_) => {
                return ResultWire::new(verb, Outcome::Blocked, ErrorCode::EvidenceUnavailable);
            }
        };
        let mut result = ResultWire::new(verb, Outcome::Ok, ErrorCode::None);
        result.generation = generation.map(|id| id.to_string());
        result.control_epoch = epoch;
        result.event_id = final_event;
        result.artifact_sha256 = Some(index_hash);
        result
    })
}

#[allow(
    dead_code,
    reason = "Held until the protected stage-chain mount is integrated."
)]
async fn reconcile(
    intent: &Intent,
    artifact: Option<&Artifact>,
    selector: &DatabaseSelector,
    output_dir: &GuardedDirectory,
) -> ResultWire {
    let verb = Verb::Reconcile;
    let Some(inventory) = artifact else {
        return ResultWire::new(verb, Outcome::Blocked, ErrorCode::EvidenceUnavailable);
    };
    let Ok(pool) = connect(selector, DatabaseRole::MigrationOwner).await else {
        return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible);
    };
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(_) => return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible),
    };
    let response = sqlx::query_scalar::<_, Value>("SELECT public.reconcile_owner_beta_paper_operation_v1($1::uuid,$2::bigint,$3::text,$4::text)")
        .bind(intent.generation).bind(intent.expected_epoch).bind(&inventory.sha256).bind(&intent.request_key).fetch_one(&mut *tx).await;
    let response = match response {
        Ok(value) => value,
        Err(error) => {
            let _ = rollback_bounded(tx).await;
            return ResultWire::new(verb, Outcome::Blocked, map_sql_error(&error, verb));
        }
    };
    if response["schema"] != "owner-beta-paper-action-v1"
        || response["action"] != "RECONCILE_OPERATION"
    {
        let _ = rollback_bounded(tx).await;
        return ResultWire::new(verb, Outcome::Blocked, ErrorCode::InternalFailure);
    }
    let outcome = match response["outcome"].as_str() {
        Some("OK") => Outcome::Ok,
        Some("REPLAY") => Outcome::Replay,
        _ => {
            let _ = rollback_bounded(tx).await;
            return ResultWire::new(verb, Outcome::Blocked, ErrorCode::EvidenceUnavailable);
        }
    };
    let summary = response["data"]["summary"].clone();
    let summary_bytes = match super::super::source_evidence::canonical(&summary) {
        Ok(bytes) if bytes.len() <= MAX_INSPECT_PAGE_BYTES => bytes,
        _ => {
            let _ = rollback_bounded(tx).await;
            return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible);
        }
    };
    if digest(&summary_bytes)
        != response["data"]["summary_sha256"]
            .as_str()
            .unwrap_or_default()
    {
        let _ = rollback_bounded(tx).await;
        return ResultWire::new(verb, Outcome::IdentityConflict, ErrorCode::IdentityConflict);
    }
    let summary_hash = match output_dir.write_content_addressed(&summary_bytes) {
        Ok(v) => v,
        Err(_) => {
            let _ = rollback_bounded(tx).await;
            return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible);
        }
    };
    if commit_bounded(tx).await.is_err() {
        return ResultWire::new(verb, Outcome::CommitUnknown, ErrorCode::CommitUnknown);
    }
    let mut result = ResultWire::new(verb, outcome, ErrorCode::None);
    result.generation = Some(intent.generation.to_string());
    result.control_epoch = response["control_epoch"].as_i64();
    result.event_id = response["data"]["observation_id"]
        .as_str()
        .map(str::to_owned);
    result.artifact_sha256 = Some(summary_hash);
    result
}

#[allow(
    dead_code,
    reason = "Held until the protected stage-chain mount is integrated."
)]
async fn transition(
    intent: &Intent,
    artifact: Option<&Artifact>,
    transition: &str,
    selector: &DatabaseSelector,
) -> ResultWire {
    let verb = match transition {
        "OFF" => Verb::Off,
        "RESTORED_OFF" => Verb::RestoreOff,
        _ => Verb::Recover,
    };
    let Some(artifact) = artifact else {
        return ResultWire::new(verb, Outcome::Blocked, ErrorCode::EvidenceUnavailable);
    };
    let Ok(pool) = connect(selector, DatabaseRole::MigrationOwner).await else {
        return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible);
    };
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(_) => return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible),
    };
    let response = sqlx::query_scalar::<_, Value>(
        "SELECT public.transition_owner_beta_paper_v1($1::bigint,$2::text,$3::jsonb,$4::text)",
    )
    .bind(intent.expected_epoch)
    .bind(transition)
    .bind(artifact.value.clone())
    .bind(&intent.request_key)
    .fetch_one(&mut *tx)
    .await;
    let response = match response {
        Ok(value) => value,
        Err(error) => {
            let _ = rollback_bounded(tx).await;
            return ResultWire::new(verb, Outcome::Blocked, map_sql_error(&error, verb));
        }
    };
    let (outcome, event_id, epoch) = match sql_outcome(&response, "TRANSITION") {
        Ok(value) => value,
        Err(code) => {
            let _ = rollback_bounded(tx).await;
            return ResultWire::new(verb, Outcome::Blocked, code);
        }
    };
    if commit_bounded(tx).await.is_err() {
        return ResultWire::new(verb, Outcome::CommitUnknown, ErrorCode::CommitUnknown);
    }
    let mut result = ResultWire::new(verb, outcome, ErrorCode::None);
    result.generation = Some(intent.generation.to_string());
    result.event_id = event_id;
    result.control_epoch = epoch;
    result.artifact_sha256 = Some(artifact.sha256.clone());
    result
}

#[cfg(test)]
mod tests {
    use super::{
        CLOCK_FRESHNESS_NS, DatabaseRole, Outcome, begin_outcome, clock_samples_fresh,
        configure_outcome, digest, enable_outcome, finalize_after_recheck, inspect_page_next,
        parse_database, process_matches_current, same_object, valid_native_enable_sessions,
    };
    use crate::owner_beta::paper::operations::wire::{parse_canonical, parse_intent};
    use crate::owner_beta::paper::operations::{ErrorCode, ResultWire, Verb};
    use uuid::Uuid;

    #[test]
    fn database_selector_has_a_closed_nonsecret_shape() {
        let good = br#"{"database":"paper","host":"postgres","port":5432,"schema":"owner-beta-paper-ops-database-v1"}"#;
        let selector = parse_database(good).unwrap();
        assert_eq!(selector.host, "postgres");
        assert_eq!(selector.database, "paper");
        assert!(parse_database(br#"{"database":"paper","host":"evil","port":5432,"schema":"owner-beta-paper-ops-database-v1"}"#).is_err());
        assert!(parse_database(br#"{"database":"paper","host":"postgres","port":5432,"password":"x","schema":"owner-beta-paper-ops-database-v1"}"#).is_err());
        assert!(parse_database(br#"{"database":"../paper","host":"postgres","port":5432,"schema":"owner-beta-paper-ops-database-v1"}"#).is_err());
    }

    #[test]
    fn digests_are_stable_and_only_hash_material_is_returned() {
        let bytes = b"private observation";
        let hash = digest(bytes);
        assert!(hash.starts_with("sha256:") && hash.len() == 71);
        assert_ne!(hash.as_bytes(), bytes);
        assert!(parse_canonical(br#"{"schema":"owner-beta-paper-ops-database-v1"}"#).is_ok());
    }

    #[test]
    fn three_clock_ttl_and_divergence_are_bounded() {
        assert!(clock_samples_fresh(CLOCK_FRESHNESS_NS - 1, 4, 4));
        assert!(!clock_samples_fresh(CLOCK_FRESHNESS_NS, 4, 4));
        assert!(!clock_samples_fresh(-1, 4, 4));
        assert!(!clock_samples_fresh(4, 4, 1_000_000_005));
    }

    #[test]
    fn post_commit_recheck_failure_is_unknown_and_never_rollback() {
        let committed = ResultWire::new(Verb::Off, Outcome::Ok, ErrorCode::None);
        let finalized = finalize_after_recheck(committed, false, Verb::Off);
        assert_eq!(finalized.exit_code(), 4);
        let value: serde_json::Value = serde_json::from_slice(&finalized.encode()).unwrap();
        assert_eq!(value["outcome"], "COMMIT_UNKNOWN");
        assert_eq!(value["error_code"], "COMMIT_UNKNOWN");
        let blocked = ResultWire::new(Verb::Off, Outcome::Blocked, ErrorCode::EvidenceUnavailable);
        let finalized = finalize_after_recheck(blocked, false, Verb::Off);
        let value: serde_json::Value = serde_json::from_slice(&finalized.encode()).unwrap();
        assert_eq!(value["outcome"], "BLOCKED");
    }

    #[test]
    fn protected_directory_identity_rejects_swap_or_metadata_change() {
        let pinned = super::Identity {
            device: 1,
            inode: 2,
            size: 4096,
            mode: libc::S_IFDIR | 0o700,
            uid: 0,
            links: 2,
            mtime: (1, 0),
            ctime: (1, 0),
        };
        let same_entry = pinned;
        let swapped_inode = super::Identity { inode: 3, ..pinned };
        let changed_mode = super::Identity {
            mode: libc::S_IFDIR | 0o755,
            ..pinned
        };
        assert!(same_object(pinned, same_entry));
        assert!(!same_object(pinned, swapped_inode));
        assert!(!same_object(pinned, changed_mode));
    }

    #[test]
    fn database_roles_are_closed_and_use_distinct_root_secret_files() {
        assert_eq!(DatabaseRole::Worker.name(), "worker");
        assert_eq!(DatabaseRole::MigrationOwner.name(), "migration_owner");
        assert_eq!(DatabaseRole::Worker.password_path(), WORKER_PASSWORD);
        assert_eq!(
            DatabaseRole::MigrationOwner.password_path(),
            MIGRATION_PASSWORD
        );
    }

    #[test]
    fn native_enable_facts_are_exact_sorted_and_identity_bound() {
        let row = |date: &str, run: &str| {
            serde_json::json!({
                "session_date":date,
                "run_id":run,
                "admission_sha256":format!("sha256:{}", "a".repeat(64)),
                "source_event_sha256":format!("sha256:{}", "b".repeat(64)),
                "root_event_sha256":format!("sha256:{}", "c".repeat(64))
            })
        };
        let valid = vec![
            row("2026-11-02", "11111111-1111-4111-8111-111111111111"),
            row("2026-11-03", "22222222-2222-4222-8222-222222222222"),
            row("2026-11-04", "33333333-3333-4333-8333-333333333333"),
        ];
        assert!(valid_native_enable_sessions(&valid));
        let mut duplicate_date = valid.clone();
        duplicate_date[2]["session_date"] = serde_json::json!("2026-11-03");
        assert!(!valid_native_enable_sessions(&duplicate_date));
        let mut widened = valid.clone();
        widened[0]["manual"] = serde_json::json!(true);
        assert!(!valid_native_enable_sessions(&widened));
        let mut malformed_run = valid;
        malformed_run[1]["run_id"] = serde_json::json!("not-a-uuid");
        assert!(!valid_native_enable_sessions(&malformed_run));
    }

    #[test]
    fn enable_process_must_match_the_protected_current_and_sql_epoch() {
        let request = serde_json::json!({
            "schema":"owner-beta-paper-operation-intent-v1",
            "installation_id":"11111111-1111-4111-8111-111111111111",
            "generation":"22222222-2222-4222-8222-222222222222",
            "account_id":"33333333-3333-4333-8333-333333333333",
            "predecessor_generation":null,
            "request_key":"22222222-2222-4222-8222-222222222222:enable",
            "purpose":"ENABLE",
            "scope":"PAPER_STOP",
            "expected_epoch":0,
            "local_epoch_floor":0,
            "release_manifest_sha256":format!("sha256:{}", "a".repeat(64)),
            "artifact_sha256":null,
            "resume_target_id":null
        });
        let request = crate::owner_beta::paper::source_evidence::canonical(&request).unwrap();
        let intent = parse_intent(&request, Verb::Enable).unwrap();
        let cohort = serde_json::json!({
            "owner_user_id":"44444444-4444-4444-8444-444444444444",
            "strategy_config_id":"55555555-5555-4555-8555-555555555555",
            "cohort_sha256":format!("sha256:{}", "b".repeat(64)),
            "release_manifest_sha256":format!("sha256:{}", "a".repeat(64)),
            "verifier_revision":"d".repeat(40)
        });
        let process = serde_json::json!({
            "owner_user_id":cohort["owner_user_id"],
            "account_id":"33333333-3333-4333-8333-333333333333",
            "strategy_config_id":cohort["strategy_config_id"],
            "daily_cohort_sha256":cohort["cohort_sha256"],
            "release_manifest_sha256":cohort["release_manifest_sha256"],
            "code_commit":"d".repeat(40),
            "control_epoch":1
        });
        assert!(process_matches_current(
            &process,
            &intent,
            &cohort,
            &"d".repeat(40)
        ));
        let mut wrong_epoch = process.clone();
        wrong_epoch["control_epoch"] = serde_json::json!(2);
        assert!(!process_matches_current(
            &wrong_epoch,
            &intent,
            &cohort,
            &"d".repeat(40)
        ));
        let mut wrong_cohort = cohort.clone();
        wrong_cohort["cohort_sha256"] = serde_json::json!(format!("sha256:{}", "c".repeat(64)));
        assert!(!process_matches_current(
            &process,
            &intent,
            &wrong_cohort,
            &"d".repeat(40)
        ));
    }

    #[test]
    fn enable_sql_envelope_requires_exact_durable_state_and_epoch() {
        let response = serde_json::json!({
            "schema":"owner-beta-paper-action-v1",
            "action":"TRANSITION",
            "outcome":"OK",
            "control_epoch":1,
            "account_id":"33333333-3333-4333-8333-333333333333",
            "data":{
                "event_id":"44444444-4444-4444-8444-444444444444",
                "state":"ENABLED"
            }
        });
        assert!(
            enable_outcome(
                &response,
                Uuid::parse_str("33333333-3333-4333-8333-333333333333").unwrap(),
                1
            )
            .is_ok()
        );
        let mut wrong_epoch = response.clone();
        wrong_epoch["control_epoch"] = serde_json::json!(2);
        assert!(
            enable_outcome(
                &wrong_epoch,
                Uuid::parse_str("33333333-3333-4333-8333-333333333333").unwrap(),
                1
            )
            .is_err()
        );
        let mut widened = response;
        widened["data"]["permission"] = serde_json::json!(true);
        assert!(
            enable_outcome(
                &widened,
                Uuid::parse_str("33333333-3333-4333-8333-333333333333").unwrap(),
                1
            )
            .is_err()
        );
    }

    #[test]
    fn native_inspect_result_requires_the_exact_sql_page_shape() {
        let request = serde_json::json!({
            "schema":"owner-beta-paper-operation-intent-v1",
            "installation_id":"11111111-1111-4111-8111-111111111111",
            "generation":"22222222-2222-4222-8222-222222222222",
            "account_id":null,
            "predecessor_generation":null,
            "request_key":"inspect",
            "purpose":"INSPECT",
            "scope":"PAPER_STOP",
            "expected_epoch":0,
            "local_epoch_floor":0,
            "release_manifest_sha256":format!("sha256:{}", "a".repeat(64)),
            "artifact_sha256":null,
            "resume_target_id":null
        });
        let request = crate::owner_beta::paper::source_evidence::canonical(&request).unwrap();
        let intent = parse_intent(&request, Verb::Inspect).unwrap();
        let mut page = serde_json::json!({
            "schema":"owner-beta-paper-action-v1",
            "action":"INSPECT_OPERATION",
            "outcome":"OK",
            "control_epoch":null,
            "account_id":null,
            "data":{
                "generation":null,
                "state":null,
                "control_epoch":null,
                "history_present":false,
                "operation_event_id":null,
                "reconciliation":null,
                "targets":[],
                "next_target":null
            }
        });
        assert_eq!(inspect_page_next(&page, &intent).unwrap(), None);
        page["data"]["unexpected"] = serde_json::json!(false);
        assert!(inspect_page_next(&page, &intent).is_err());
    }

    #[test]
    fn sql_configure_and_begin_envelopes_bind_replay_identities() {
        let configure = serde_json::json!({
            "schema":"owner-beta-paper-action-v1",
            "action":"CONFIGURE",
            "outcome":"OK",
            "control_epoch":0,
            "account_id":"33333333-3333-4333-8333-333333333333",
            "data":{
                "account_id":"33333333-3333-4333-8333-333333333333",
                "event_id":"44444444-4444-4444-8444-444444444444",
                "state":"OFF",
                "initial_cash":"10000000.0000",
                "currency":"KRW"
            }
        });
        assert!(configure_outcome(&configure).is_ok());
        let mut widened = configure;
        widened["unexpected"] = serde_json::json!(true);
        assert!(configure_outcome(&widened).is_err());

        let request = serde_json::json!({
            "schema":"owner-beta-paper-operation-intent-v1",
            "installation_id":"11111111-1111-4111-8111-111111111111",
            "generation":"22222222-2222-4222-8222-222222222222",
            "account_id":"33333333-3333-4333-8333-333333333333",
            "predecessor_generation":null,
            "request_key":"22222222-2222-4222-8222-222222222222:begin",
            "purpose":"STOP",
            "scope":"PAPER_STOP",
            "expected_epoch":0,
            "local_epoch_floor":0,
            "release_manifest_sha256":format!("sha256:{}", "a".repeat(64)),
            "artifact_sha256":null,
            "resume_target_id":null
        });
        let request = crate::owner_beta::paper::source_evidence::canonical(&request).unwrap();
        let intent = parse_intent(&request, Verb::Begin).unwrap();
        let intent_sha = crate::owner_beta::paper::source_evidence::digest(&request);
        let begin = serde_json::json!({
            "schema":"owner-beta-paper-action-v1",
            "action":"BEGIN_OPERATION",
            "outcome":"REPLAY",
            "control_epoch":3,
            "account_id":"33333333-3333-4333-8333-333333333333",
            "data":{
                "generation":"22222222-2222-4222-8222-222222222222",
                "operation_event_id":"55555555-5555-4555-8555-555555555555",
                "intent_sha256":intent_sha,
                "fenced_epoch":1
            }
        });
        assert_eq!(begin_outcome(&begin, &intent).unwrap().2, 3);
    }
}
