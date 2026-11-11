use std::{
    ffi::CString,
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    os::{
        fd::{AsRawFd, FromRawFd, RawFd},
        unix::fs::FileExt,
    },
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
    wire::{
        self, Artifact, ExactReadback, ExactRequest, Intent, ResetAnchor, ResetBoundary,
        ResetSelection,
    },
};
use crate::owner_beta::paper::evidence::{
    CheckedEnableSession, ClockTriple, financial::CheckedPaperAction,
};

mod archive;
mod archive_sandbox;
mod bound;
mod chain;

const PRIVATE_DIR: &str = "/run/owner-beta-paper-ops";
const REQUEST_PATH: &str = "/run/owner-beta-paper-ops/request.json";
const EVIDENCE_PATH: &str = "/run/owner-beta-paper-ops/evidence.json";
const SQL_REQUEST_PATH: &str = "/run/owner-beta-paper-ops/sql-request.json";
const RESET_SELECTION_PATH: &str = "/run/owner-beta-paper-ops/reset-selection.json";
const RESET_ANCHORS_PATH: &str = "/run/owner-beta-paper-ops/reset-anchors";
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
// Source implementation is not runtime acceptance. SQL/native hosted compiler
// and PG proof AND P4 local selection/anchor publication must pass together.
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
    gid: u32,
    links: u64,
    mtime: (i64, i64),
    ctime: (i64, i64),
}

struct GuardedFile {
    file: File,
    path: &'static str,
    identity: Identity,
    expected_mode: u32,
    // Keeps every opened directory ancestor alive while the file is consumed.
    _parents: Vec<File>,
}

struct GuardedDirectory {
    directory: File,
    path: &'static str,
    identity: Identity,
    _parents: Vec<File>,
}

struct RetainedPublishedOutput {
    file: File,
    identity: Identity,
    leaf: String,
    bytes: Vec<u8>,
    digest: String,
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
        gid: m.gid(),
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
        || id.gid != 0
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
        || id.gid != 0
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
    open_fixed_mode(path, want_dir, 0o400)
}

fn open_fixed_mode(
    path: &'static str,
    want_dir: bool,
    expected_mode: u32,
) -> Result<(File, Vec<File>), GuardError> {
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
                check_file_mode(&next, expected_mode)?;
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
    let fd = unsafe { libc::openat(dirfd, name.as_ptr(), flags | libc::O_NONBLOCK) };
    if fd < 0 {
        return Err(GuardError::Invalid);
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

impl GuardedFile {
    fn open(path: &'static str) -> Result<Self, GuardError> {
        Self::open_mode(path, 0o400)
    }

    fn open_mode(path: &'static str, expected_mode: u32) -> Result<Self, GuardError> {
        let (file, parents) = open_fixed_mode(path, false, expected_mode)?;
        let identity = check_file_mode(&file, expected_mode)?;
        Ok(Self {
            file,
            path,
            identity,
            expected_mode,
            _parents: parents,
        })
    }

    fn read_bounded(&mut self, max: usize) -> Result<Vec<u8>, GuardError> {
        self.revalidate()?;
        self.file
            .seek(SeekFrom::Start(0))
            .map_err(|_| GuardError::Read)?;
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
        let (named, parents) = open_fixed_mode(self.path, false, self.expected_mode)?;
        if check_file_mode(&named, self.expected_mode)? != self.identity
            || !same_parents(&self._parents, &parents)?
        {
            return Err(GuardError::Changed);
        }
        Ok(())
    }

    fn parent_matches(&self, directory: &GuardedDirectory) -> Result<bool, GuardError> {
        let Some(parent) = self._parents.last() else {
            return Ok(false);
        };
        Ok(same_object(check_dir(parent, false)?, directory.identity))
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
            || identity.gid != 0
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
        let (named, parents) = open_fixed(self.path, true)?;
        if !same_object(identity(&named)?, self.identity)
            || !same_parents(&self._parents, &parents)?
        {
            return Err(GuardError::Changed);
        }
        Ok(())
    }

    fn write_content_addressed(&self, bytes: &[u8]) -> Result<String, GuardError> {
        self.write_content_addressed_retained(bytes)
            .map(|published| published.digest)
    }

    fn write_content_addressed_retained(
        &self,
        bytes: &[u8],
    ) -> Result<RetainedPublishedOutput, GuardError> {
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
                // An existing output is not durable merely because read succeeds.
                existing.sync_all().map_err(|_| GuardError::Read)?;
                self.directory.sync_all().map_err(|_| GuardError::Read)?;
                if check_file(&existing)? != existing_id {
                    return Err(GuardError::Changed);
                }
                let named = openat_file(
                    self.directory.as_raw_fd(),
                    leaf.as_str(),
                    libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                )?;
                if check_file(&named)? != existing_id {
                    return Err(GuardError::Changed);
                }
                self.revalidate()?;
                let published = RetainedPublishedOutput {
                    file: existing,
                    identity: existing_id,
                    leaf,
                    bytes: bytes.to_vec(),
                    digest,
                };
                published.revalidate(self)?;
                return Ok(published);
            }
            return Err(GuardError::Invalid);
        }
        let mut file = unsafe { File::from_raw_fd(fd) };
        file.write_all(bytes).map_err(|_| GuardError::Read)?;
        file.sync_all().map_err(|_| GuardError::Read)?;
        let written_id = check_file(&file)?;
        self.directory.sync_all().map_err(|_| GuardError::Read)?;
        let retained = openat_file(
            self.directory.as_raw_fd(),
            leaf.as_str(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )?;
        if check_file(&file)? != written_id || check_file(&retained)? != written_id {
            return Err(GuardError::Changed);
        }
        let named = openat_file(
            self.directory.as_raw_fd(),
            leaf.as_str(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )?;
        if check_file(&named)? != written_id {
            return Err(GuardError::Changed);
        }
        self.revalidate()?;
        let published = RetainedPublishedOutput {
            file: retained,
            identity: written_id,
            leaf,
            bytes: bytes.to_vec(),
            digest,
        };
        published.revalidate(self)?;
        // Keep the O_WRONLY producer descriptor alive until the read descriptor
        // and the fixed name have both been matched to its original inode.
        drop(file);
        Ok(published)
    }
}

impl RetainedPublishedOutput {
    fn revalidate(&self, output: &GuardedDirectory) -> Result<(), GuardError> {
        output.revalidate()?;
        let expected_digest = digest(&self.bytes);
        if self.digest != expected_digest
            || self.leaf != format!("{}.json", &expected_digest[7..])
            || check_file(&self.file)? != self.identity
        {
            return Err(GuardError::Changed);
        }
        let mut current = vec![0; self.bytes.len()];
        self.file
            .read_exact_at(&mut current, 0)
            .map_err(|_| GuardError::Read)?;
        if current != self.bytes {
            return Err(GuardError::Changed);
        }
        let named = openat_file(
            output.directory.as_raw_fd(),
            &self.leaf,
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )?;
        if check_file(&named)? != self.identity {
            return Err(GuardError::Changed);
        }
        output.revalidate()
    }
}

fn same_object(left: Identity, right: Identity) -> bool {
    left.device == right.device
        && left.inode == right.inode
        && left.mode == right.mode
        && left.uid == right.uid
        && left.gid == right.gid
        && left.links == right.links
}

fn same_parents(left: &[File], right: &[File]) -> Result<bool, GuardError> {
    if left.len() != right.len() {
        return Ok(false);
    }
    for (left, right) in left.iter().zip(right) {
        if !same_object(check_dir(left, false)?, check_dir(right, false)?) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn check_file_mode(file: &File, expected_mode: u32) -> Result<Identity, GuardError> {
    let id = identity(file)?;
    if id.uid != 0
        || id.gid != 0
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
    let nanoseconds = i128::from(value.tv_sec)
        .checked_mul(1_000_000_000)
        .and_then(|sec| sec.checked_add(i128::from(value.tv_nsec)))
        .ok_or(())?;
    i64::try_from(nanoseconds).map_err(|_| ())
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
    (0..CLOCK_FRESHNESS_NS).contains(&realtime_age)
        && (0..CLOCK_FRESHNESS_NS).contains(&monotonic_age)
        && (0..CLOCK_FRESHNESS_NS).contains(&boottime_age)
        && realtime_age.abs_diff(monotonic_age) <= 1_000_000_000
        && boottime_age.abs_diff(monotonic_age) <= 1_000_000_000
}

fn finalize_after_recheck(
    mut result: ResultWire,
    recheck_succeeded: bool,
    verb: Verb,
) -> ResultWire {
    if recheck_succeeded {
        return result;
    }
    match result.outcome {
        Outcome::Ok | Outcome::Replay if matches!(verb, Verb::Inspect | Verb::ObserveReset) => {
            ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible)
        }
        Outcome::Ok | Outcome::Replay | Outcome::CommitUnknown => {
            mark_commit_unknown(&mut result);
            result
        }
        Outcome::Blocked => result,
    }
}

fn mark_commit_unknown(result: &mut ResultWire) {
    result.outcome = Outcome::CommitUnknown;
    result.error_code = ErrorCode::CommitUnknown;
}

async fn close_pool(pool: PgPool, result: ResultWire, verb: Verb) -> ResultWire {
    let close_completed = tokio::time::timeout(Duration::from_secs(5), pool.close())
        .await
        .is_ok();
    finalize_after_recheck(result, close_completed, verb)
}

macro_rules! finish_with_pool {
    ($pool:ident, $verb:ident, $body:block) => {{
        let pool_to_close = $pool.clone();
        let result = async move { $body }.await;
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
    saved_request: Option<(GuardedFile, ExactRequest)>,
    bound: Option<bound::BoundOriginals>,
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
    provenance: ProducerProvenance,
}

impl EnablePins {
    fn open(intent: &Intent, process: &Artifact) -> Result<Self, ErrorCode> {
        let (capability, capability_bytes) = read_fixed(CAPABILITY_PATH, wire::MAX_WIRE_BYTES)
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
        let provenance = ProducerProvenance::open(
            compiled_commit,
            &api_sha256,
            &worker_sha256,
            &executable.sha256,
        )?;
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
            provenance,
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
            .map_err(|_| ErrorCode::ReaderIncompatible)?;
        self.provenance.revalidate()
    }
}

struct ProducerProvenance {
    root: GuardedDirectory,
    directories: Vec<GuardedDirectory>,
    files: Vec<GuardedFile>,
}

fn fixed_directory_names(directory: &GuardedDirectory, expected: &[&str]) -> Result<(), ErrorCode> {
    directory
        .revalidate()
        .map_err(|_| ErrorCode::ReaderIncompatible)?;
    let mut actual =
        std::fs::read_dir(format!("/proc/self/fd/{}", directory.directory.as_raw_fd()))
            .map_err(|_| ErrorCode::EvidenceUnavailable)?
            .take(expected.len() + 1)
            .map(|entry| {
                entry
                    .map_err(|_| ErrorCode::EvidenceUnavailable)?
                    .file_name()
                    .into_string()
                    .map_err(|_| ErrorCode::WireInvalid)
            })
            .collect::<Result<Vec<_>, _>>()?;
    actual.sort();
    let mut expected = expected.to_vec();
    expected.sort();
    if actual.iter().map(String::as_str).collect::<Vec<_>>() != expected {
        return Err(ErrorCode::IdentityConflict);
    }
    Ok(())
}

fn validate_producer_receipt(
    bytes: &[u8],
    complete: &[u8],
    bin: &str,
    commit: &str,
    elf_sha256: &str,
) -> Result<(), ErrorCode> {
    // Official producer writes sorted compact JSON and one LF. Hash exactly
    // those retained bytes, not reserialized receipt data or ELF string literals.
    if bytes.is_empty() || bytes.len() > wire::MAX_WIRE_BYTES {
        return Err(ErrorCode::WireInvalid);
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|_| ErrorCode::WireInvalid)?;
    let mut expected =
        super::super::source_evidence::canonical(&value).map_err(|_| ErrorCode::WireInvalid)?;
    expected.push(b'\n');
    if expected != bytes {
        // Existing producer format, deliberately separate from operation wire:
        // exactly compact sorted JSON PLUS ONE LF; duplicate keys also fail.
        return Err(ErrorCode::WireInvalid);
    }
    if !exact_keys(
        &value,
        &[
            "binary_mode",
            "binary_sha256",
            "bin",
            "cache_key",
            "cargo_success",
            "compile_env",
            "features",
            "format",
            "h_sha256",
            "host_triple",
            "input_sha256",
            "package",
            "platform",
            "profile",
            "recipe_sha256",
            "source_commit",
        ],
    ) || value["format"] != "lagrange-rust-artifact-v1"
        || value["bin"] != bin
        || value["package"] != "api-server"
        || value["platform"] != "linux/amd64"
        || value["host_triple"] != "x86_64-unknown-linux-musl"
        || value["profile"] != "release"
        || value["binary_mode"] != "0755"
        || value["cargo_success"] != true
        || value["features"] != json!([])
        || value["source_commit"] != commit
        || value["compile_env"]
            != json!({
                "CARGO_BUILD_JOBS":"2", "CARGO_TARGET_DIR":"/cargo-target",
                "RUSTFLAGS":"<unset>", "LAGRANGE_CODE_COMMIT":commit
            })
        || value["binary_sha256"].as_str() != elf_sha256.strip_prefix("sha256:")
        || ![
            "binary_sha256",
            "cache_key",
            "h_sha256",
            "input_sha256",
            "recipe_sha256",
        ]
        .iter()
        .all(|key| sha256_hex(&value[*key]))
        || complete != format!("{}\n", &digest(bytes)[7..]).as_bytes()
    {
        return Err(ErrorCode::IdentityConflict);
    }
    Ok(())
}

impl ProducerProvenance {
    fn open(commit: &str, api: &str, worker: &str, native: &str) -> Result<Self, ErrorCode> {
        const ROOT: &str = "/run/owner-beta-paper-ops/provenance";
        const PAIRS: [(&str, &str, &str, &str); 3] = [
            (
                "api-server",
                "/run/owner-beta-paper-ops/provenance/api-server",
                "/run/owner-beta-paper-ops/provenance/api-server/artifact.json",
                "/run/owner-beta-paper-ops/provenance/api-server/COMPLETE",
            ),
            (
                "paper-runner",
                "/run/owner-beta-paper-ops/provenance/paper-runner",
                "/run/owner-beta-paper-ops/provenance/paper-runner/artifact.json",
                "/run/owner-beta-paper-ops/provenance/paper-runner/COMPLETE",
            ),
            (
                "owner-beta-paper-ops-native",
                "/run/owner-beta-paper-ops/provenance/owner-beta-paper-ops-native",
                "/run/owner-beta-paper-ops/provenance/owner-beta-paper-ops-native/artifact.json",
                "/run/owner-beta-paper-ops/provenance/owner-beta-paper-ops-native/COMPLETE",
            ),
        ];
        let root = GuardedDirectory::open(ROOT).map_err(|_| ErrorCode::EvidenceUnavailable)?;
        fixed_directory_names(
            &root,
            &["api-server", "paper-runner", "owner-beta-paper-ops-native"],
        )?;
        let mut directories = Vec::new();
        let mut files = Vec::new();
        for ((bin, directory, artifact, complete), elf) in
            PAIRS.into_iter().zip([api, worker, native])
        {
            let directory =
                GuardedDirectory::open(directory).map_err(|_| ErrorCode::EvidenceUnavailable)?;
            fixed_directory_names(&directory, &["artifact.json", "COMPLETE"])?;
            let (artifact_file, bytes) = read_fixed(artifact, wire::MAX_WIRE_BYTES)
                .map_err(|_| ErrorCode::EvidenceUnavailable)?;
            let (complete_file, complete_bytes) =
                read_fixed(complete, 65).map_err(|_| ErrorCode::EvidenceUnavailable)?;
            validate_producer_receipt(&bytes, &complete_bytes, bin, commit, elf)?;
            directories.push(directory);
            files.extend([artifact_file, complete_file]);
        }
        let result = Self {
            root,
            directories,
            files,
        };
        result.revalidate()?;
        Ok(result)
    }

    fn revalidate(&self) -> Result<(), ErrorCode> {
        fixed_directory_names(
            &self.root,
            &["api-server", "paper-runner", "owner-beta-paper-ops-native"],
        )?;
        for directory in &self.directories {
            fixed_directory_names(directory, &["artifact.json", "COMPLETE"])?;
        }
        for file in &self.files {
            file.revalidate()
                .map_err(|_| ErrorCode::ReaderIncompatible)?;
        }
        Ok(())
    }
}

struct NativeEnableGuard<'a> {
    inputs: &'a ProtectedInputs,
    output: &'a GuardedDirectory,
    pins: &'a EnablePins,
    anchors: &'a [CheckedResetAnchor],
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
        self.inputs.revalidate()?;
        self.output
            .revalidate()
            .map_err(|_| ErrorCode::ReaderIncompatible)?;
        self.pins.revalidate()?;
        for anchor in self.anchors {
            anchor.revalidate()?;
        }
        Ok(())
    }
}

struct CheckedResetAnchor {
    // No Deserialize/Clone or data-only constructor: each instance retains the
    // original selection, directory and file, not a report or a supplied flag.
    selection_file: GuardedFile,
    directory: GuardedDirectory,
    file: File,
    identity: Identity,
    names: Vec<String>,
    name: String,
    anchor: ResetAnchor,
}

fn anchor_names(directory: &GuardedDirectory) -> Result<Vec<String>, ErrorCode> {
    directory
        .revalidate()
        .map_err(|_| ErrorCode::ReaderIncompatible)?;
    let entries = std::fs::read_dir(format!("/proc/self/fd/{}", directory.directory.as_raw_fd()))
        .map_err(|_| ErrorCode::EvidenceUnavailable)?;
    let mut names = Vec::new();
    for entry in entries.take(4) {
        let entry = entry.map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| ErrorCode::WireInvalid)?;
        if name.len() != 69
            || !name.ends_with(".json")
            || !name[..64]
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(ErrorCode::WireInvalid);
        }
        names.push(name);
    }
    if names.is_empty() || names.len() > 3 {
        return Err(ErrorCode::EvidenceUnavailable);
    }
    names.sort();
    directory
        .revalidate()
        .map_err(|_| ErrorCode::ReaderIncompatible)?;
    Ok(names)
}

fn selection_matches(left: &ResetSelection, right: &ResetSelection) -> bool {
    left.installation_id == right.installation_id
        && left.reset == right.reset
        && left.release_manifest_sha256 == right.release_manifest_sha256
        && left.cohort_sha256 == right.cohort_sha256
}

fn read_selection(
    intent: &Intent,
    cohort: &str,
) -> Result<(GuardedFile, ResetSelection), ErrorCode> {
    let (file, bytes) = read_fixed(RESET_SELECTION_PATH, wire::MAX_WIRE_BYTES)
        .map_err(|_| ErrorCode::EvidenceUnavailable)?;
    let selection = ResetSelection::parse(&bytes).map_err(|_| ErrorCode::WireInvalid)?;
    if selection.installation_id != intent.installation_id
        || selection.release_manifest_sha256 != intent.release_manifest_sha256
        || selection.cohort_sha256 != cohort
        || Some(selection.reset.account_id) != intent.account_id
        || selection.reset.control_epoch > intent.expected_epoch
    {
        return Err(ErrorCode::IdentityConflict);
    }
    Ok((file, selection))
}

impl CheckedResetAnchor {
    fn open_all(intent: &Intent, cohort: &str) -> Result<Vec<Self>, ErrorCode> {
        let directory = GuardedDirectory::open(RESET_ANCHORS_PATH)
            .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let names = anchor_names(&directory)?;
        let mut anchors = Vec::new();
        let mut boots = std::collections::BTreeSet::new();
        for name in &names {
            let (selection_file, selection) = read_selection(intent, cohort)?;
            let directory = GuardedDirectory::open(RESET_ANCHORS_PATH)
                .map_err(|_| ErrorCode::EvidenceUnavailable)?;
            let mut file = openat_file(
                directory.directory.as_raw_fd(),
                name,
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
            .map_err(|_| ErrorCode::EvidenceUnavailable)?;
            let identity = check_file(&file).map_err(|_| ErrorCode::ReaderIncompatible)?;
            if identity.size > wire::MAX_WIRE_BYTES as u64 {
                return Err(ErrorCode::WireInvalid);
            }
            let mut bytes = Vec::new();
            (&mut file)
                .take(wire::MAX_WIRE_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| ErrorCode::EvidenceUnavailable)?;
            if name != &format!("{}.json", &digest(&bytes)[7..]) {
                return Err(ErrorCode::IdentityConflict);
            }
            let anchor = ResetAnchor::parse(&bytes).map_err(|_| ErrorCode::WireInvalid)?;
            if !selection_matches(&selection, &anchor.selection) || !boots.insert(anchor.boot_id) {
                return Err(ErrorCode::IdentityConflict);
            }
            let checked = Self {
                selection_file,
                directory,
                file,
                identity,
                name: name.clone(),
                names: names.clone(),
                anchor,
            };
            checked.revalidate()?;
            anchors.push(checked);
        }
        Ok(anchors)
    }

    fn revalidate(&self) -> Result<(), ErrorCode> {
        self.selection_file
            .revalidate()
            .map_err(|_| ErrorCode::ReaderIncompatible)?;
        self.directory
            .revalidate()
            .map_err(|_| ErrorCode::ReaderIncompatible)?;
        if identity(&self.directory.directory).map_err(|_| ErrorCode::ReaderIncompatible)?
            != self.directory.identity
            || anchor_names(&self.directory)? != self.names
            || check_file(&self.file).map_err(|_| ErrorCode::ReaderIncompatible)? != self.identity
        {
            return Err(ErrorCode::IdentityConflict);
        }
        let named = openat_file(
            self.directory.directory.as_raw_fd(),
            &self.name,
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
        .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        if check_file(&named).map_err(|_| ErrorCode::ReaderIncompatible)? != self.identity {
            return Err(ErrorCode::IdentityConflict);
        }
        // Historical anchors have no TTL or multi-day drift check. Only compare
        // kernel counters when their boot actually is this kernel's boot.
        if local_boot()? == self.anchor.boot_id {
            let now = sample_clocks()?;
            if !now.strictly_after(&self.anchor.observed) {
                return Err(ErrorCode::ClockInvalid);
            }
        }
        Ok(())
    }
}

fn local_boot() -> Result<Uuid, ErrorCode> {
    let text = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map_err(|_| ErrorCode::ClockInvalid)?;
    let text = text.trim_end_matches('\n');
    let boot = Uuid::parse_str(text).map_err(|_| ErrorCode::ClockInvalid)?;
    if boot.is_nil() || boot.to_string() != text {
        return Err(ErrorCode::ClockInvalid);
    }
    Ok(boot)
}

fn sample_clocks_value() -> Result<Value, ErrorCode> {
    Ok(json!({
        "realtime_ns":clock_ns(libc::CLOCK_REALTIME).map_err(|_| ErrorCode::ClockInvalid)?,
        "monotonic_ns":clock_ns(libc::CLOCK_MONOTONIC).map_err(|_| ErrorCode::ClockInvalid)?,
        "boottime_ns":clock_ns(libc::CLOCK_BOOTTIME).map_err(|_| ErrorCode::ClockInvalid)?,
    }))
}

fn sample_clocks() -> Result<ClockTriple, ErrorCode> {
    ClockTriple::from_value(&sample_clocks_value()?).map_err(|_| ErrorCode::ClockInvalid)
}

fn check_enable_reset(
    action: &CheckedPaperAction<'_>,
    reset: &ResetBoundary,
    anchors: &[CheckedResetAnchor],
) -> Result<(), ErrorCode> {
    let inputs = action.inputs();
    let sessions = inputs.native_enable_sessions();
    if !valid_native_enable_sessions(sessions) || anchors.is_empty() || anchors.len() > 3 {
        return Err(ErrorCode::EvidenceUnavailable);
    }
    let mut used = std::collections::BTreeSet::new();
    for session in sessions {
        let anchor = anchors
            .iter()
            .find(|anchor| anchor.anchor.boot_id == session.arm().boot_id())
            .ok_or(ErrorCode::EvidenceUnavailable)?;
        anchor.revalidate()?;
        if anchor.anchor.selection.reset != *reset
            || session.cohort_sha256() != anchor.anchor.selection.cohort_sha256
            || inputs.current_binding()["cohort"]["cohort_sha256"] != session.cohort_sha256()
        {
            return Err(ErrorCode::IdentityConflict);
        }
        let (intent, armed) = session.arm().observed();
        if !intent.strictly_after(&anchor.anchor.observed)
            || !armed.strictly_after(&anchor.anchor.observed)
        {
            return Err(ErrorCode::ClockInvalid);
        }
        used.insert(session.arm().boot_id());
    }
    if used.len() != anchors.len() {
        return Err(ErrorCode::IdentityConflict);
    }
    Ok(())
}

async fn read_exact_result(
    connection: &mut sqlx::PgConnection,
    request: &ExactRequest,
) -> Result<ExactReadback, ErrorCode> {
    // SET LOCAL must finish before SELECT is submitted. Function settings alone
    // do not bound an already executing statement. Outer bound includes setup.
    tokio::time::timeout(Duration::from_secs(15), async {
        sqlx::query("SET LOCAL lock_timeout = '5s'")
            .execute(&mut *connection)
            .await
            .map_err(|_| ErrorCode::CommitUnknown)?;
        sqlx::query("SET LOCAL statement_timeout = '15s'")
            .execute(&mut *connection)
            .await
            .map_err(|_| ErrorCode::CommitUnknown)?;
        let value = sqlx::query_scalar::<_, Value>(
            "SELECT public.read_owner_beta_paper_operation_result_v1($1::text,$2::text)",
        )
        .bind(&request.canonical)
        .bind(&request.sha256)
        .fetch_one(&mut *connection)
        .await
        .map_err(|error| {
            match error
                .as_database_error()
                .and_then(|db| db.code())
                .as_deref()
            {
                Some("22023" | "23505" | "23514" | "40001" | "42501" | "55000") => {
                    map_sql_error(&error, Verb::Readback)
                }
                // Includes cancellation/deadline/transport failure. None is absence.
                _ => ErrorCode::CommitUnknown,
            }
        })?;
        ExactReadback::parse(value, request).map_err(|_| ErrorCode::WireInvalid)
    })
    .await
    .map_err(|_| ErrorCode::CommitUnknown)?
}

async fn read_reset_boundary(
    connection: &mut sqlx::PgConnection,
    account_id: Uuid,
    expected_epoch: i64,
) -> Result<ResetBoundary, ErrorCode> {
    let request =
        ExactRequest::reset(account_id, expected_epoch).map_err(|_| ErrorCode::WireInvalid)?;
    let readback = read_exact_result(connection, &request).await?;
    ResetBoundary::parse(&readback.value["reset"]).map_err(|_| ErrorCode::WireInvalid)
}

async fn fresh_exact_read(
    pool: &PgPool,
    request: &ExactRequest,
) -> Result<ExactReadback, ErrorCode> {
    tokio::time::timeout(Duration::from_secs(15), async {
        let mut tx = pool.begin().await.map_err(|_| ErrorCode::CommitUnknown)?;
        if sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .is_err()
        {
            let _ = rollback_bounded(tx).await;
            return Err(ErrorCode::CommitUnknown);
        }
        let result = read_exact_result(&mut tx, request).await;
        // Always discard this snapshot. A later read must not reuse ABSENT from
        // before a surviving original backend committed.
        if !rollback_bounded(tx).await {
            return Err(ErrorCode::CommitUnknown);
        }
        result
    })
    .await
    .map_err(|_| ErrorCode::CommitUnknown)?
}

fn canonical_text(value: &Value) -> Result<String, ErrorCode> {
    String::from_utf8(
        super::super::source_evidence::canonical(value).map_err(|_| ErrorCode::WireInvalid)?,
    )
    .map_err(|_| ErrorCode::WireInvalid)
}

fn make_exact(kind: &str, intent: &Intent, args: Value) -> Result<ExactRequest, ErrorCode> {
    ExactRequest::from_value(json!({"schema":"owner-beta-paper-exact-request-v1",
        "kind":kind, "account_id":intent.account_id, "args":args}))
    .map_err(|_| ErrorCode::WireInvalid)
}

fn persist_dispatch(
    output: &GuardedDirectory,
    inputs: &ProtectedInputs,
    intent: &Intent,
    artifact: Option<&Artifact>,
    request: &ExactRequest,
) -> Result<(), ErrorCode> {
    inputs.revalidate()?;
    if let Some(originals) = &inputs.bound {
        return originals.persist_bound_dispatch(output, inputs, artifact, request);
    }
    request
        .correlate(intent, artifact)
        .map_err(|_| ErrorCode::IdentityConflict)?;
    let intent_bytes = canonical_text(&intent.value)?;
    let artifact_bytes = artifact.map(|a| canonical_text(&a.value)).transpose()?;
    // Files and the directory are fsynced by each write. The final descriptor is
    // a custody index, not an authorization token. Its embedded originals retain
    // all installation/release/account/generation/epoch and evaluation bindings.
    let intent_hash = output
        .write_content_addressed(intent_bytes.as_bytes())
        .map_err(|_| ErrorCode::EvidenceUnavailable)?;
    let artifact_hash = artifact_bytes
        .as_ref()
        .map(|bytes| output.write_content_addressed(bytes.as_bytes()))
        .transpose()
        .map_err(|_| ErrorCode::EvidenceUnavailable)?;
    let sql_hash = output
        .write_content_addressed(request.canonical.as_bytes())
        .map_err(|_| ErrorCode::EvidenceUnavailable)?;
    let custody = json!({
        "schema":"owner-beta-paper-native-dispatch-v1",
        "intent_canonical":intent_bytes, "intent_sha256":intent_hash,
        "artifact_canonical":artifact_bytes, "artifact_sha256":artifact_hash,
        "sql_request_canonical":request.canonical, "sql_request_sha256":sql_hash,
    });
    output
        .write_content_addressed(canonical_text(&custody)?.as_bytes())
        .map_err(|_| ErrorCode::EvidenceUnavailable)?;
    inputs.revalidate()?;
    output
        .revalidate()
        .map_err(|_| ErrorCode::ReaderIncompatible)?;
    Ok(())
}

impl ProtectedInputs {
    fn revalidate(&self) -> Result<(), ErrorCode> {
        self.request
            .revalidate()
            .map_err(|_| ErrorCode::ReaderIncompatible)?;
        self.database
            .revalidate()
            .map_err(|_| ErrorCode::ReaderIncompatible)?;
        if let Some(file) = &self.evidence {
            file.revalidate()
                .map_err(|_| ErrorCode::ReaderIncompatible)?;
        }
        if let Some((file, _)) = &self.saved_request {
            file.revalidate()
                .map_err(|_| ErrorCode::ReaderIncompatible)?;
        }
        if let Some(originals) = &self.bound {
            originals.revalidate(self)?;
        }
        Ok(())
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

async fn rollback_result(
    transaction: sqlx::Transaction<'_, sqlx::Postgres>,
    verb: Verb,
    intent: &Intent,
    code: ErrorCode,
) -> ResultWire {
    let mut result = if rollback_bounded(transaction).await {
        ResultWire::new(verb, Outcome::Blocked, code)
    } else {
        ResultWire::new(verb, Outcome::CommitUnknown, ErrorCode::CommitUnknown)
    };
    result.generation = Some(intent.generation.to_string());
    result
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
            Some("23505" | "23514") => ErrorCode::IdentityConflict,
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
    // Probe only the fixed protected name. A malformed/unreadable bound input
    // is never permission to downgrade to the legacy parser.
    if bound::has_action().map_err(|code| ResultWire::new(verb, Outcome::Blocked, code))? {
        return bound::load_bound_context(verb)
            .map_err(|code| ResultWire::new(verb, Outcome::Blocked, code));
    }
    if matches!(verb, Verb::Reconcile | Verb::Off) {
        // A stripped binding must never reopen the dormant legacy mutation path.
        return Err(ResultWire::new(
            verb,
            Outcome::Blocked,
            ErrorCode::EvidenceUnavailable,
        ));
    }
    if bound::has_dispatch().map_err(|code| ResultWire::new(verb, Outcome::Blocked, code))? {
        return Err(ResultWire::new(
            verb,
            Outcome::Blocked,
            ErrorCode::EvidenceUnavailable,
        ));
    }
    let (request_guard, request_bytes) = read_fixed(REQUEST_PATH, wire::MAX_WIRE_BYTES)
        .map_err(|_| ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible))?;
    let saved_request = if verb == Verb::Readback {
        let (file, bytes) = read_fixed(SQL_REQUEST_PATH, wire::MAX_EXACT_BYTES)
            .map_err(|_| ResultWire::new(verb, Outcome::Blocked, ErrorCode::EvidenceUnavailable))?;
        let request = ExactRequest::parse(&bytes)
            .map_err(|_| ResultWire::new(verb, Outcome::Blocked, ErrorCode::WireInvalid))?;
        Some((file, request))
    } else {
        None
    };
    let context_verb = if let Some((_, request)) = &saved_request {
        request
            .original_verb()
            .map_err(|_| ResultWire::new(verb, Outcome::Blocked, ErrorCode::WireInvalid))?
    } else {
        verb
    };
    let intent = wire::parse_intent(&request_bytes, context_verb)
        .map_err(|_| ResultWire::new(verb, Outcome::Blocked, ErrorCode::WireInvalid))?;
    request_guard
        .revalidate()
        .map_err(|_| ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible))?;
    let (evidence, evidence_guard) = match (intent.artifact_sha256.as_ref(), context_verb) {
        (None, Verb::Inspect | Verb::Begin) => (None, None),
        (Some(_), _) => {
            let (evidence_guard, bytes) =
                read_fixed(EVIDENCE_PATH, wire::MAX_WIRE_BYTES).map_err(|_| {
                    ResultWire::new(verb, Outcome::Blocked, ErrorCode::EvidenceUnavailable)
                })?;
            let schema = match context_verb {
                Verb::Reconcile => "owner-beta-paper-writer-inventory-v1",
                Verb::Off => "owner-beta-paper-drain-receipt-v1",
                Verb::RestoreOff => "owner-beta-paper-restore-receipt-v1",
                Verb::Recover => "owner-beta-paper-recovery-v2",
                Verb::Configure => "owner-beta-paper-configure-operation-v1",
                Verb::Enable | Verb::ObserveReset => "owner-beta-paper-process-v1",
                _ => {
                    return Err(ResultWire::new(
                        verb,
                        Outcome::Blocked,
                        ErrorCode::WireInvalid,
                    ));
                }
            };
            let artifact = if context_verb == Verb::Configure {
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
            } else if matches!(context_verb, Verb::Enable | Verb::ObserveReset) {
                wire::parse_process_binding(&bytes, &intent)
                    .map_err(|_| ResultWire::new(verb, Outcome::Blocked, ErrorCode::WireInvalid))?
            } else {
                wire::parse_artifact(&bytes, schema, &intent)
                    .map_err(|_| ResultWire::new(verb, Outcome::Blocked, ErrorCode::WireInvalid))?
            };
            if context_verb == Verb::Reconcile && verb != Verb::Readback {
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
            if verb != Verb::Readback
                && matches!(context_verb, Verb::Reconcile | Verb::Off | Verb::RestoreOff)
            {
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
    if let Some((file, request)) = &saved_request {
        file.revalidate()
            .map_err(|_| ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible))?;
        request
            .correlate(&intent, evidence.as_ref())
            .map_err(|_| ResultWire::new(verb, Outcome::Blocked, ErrorCode::IdentityConflict))?;
    }
    Ok((
        ProtectedInputs {
            request: request_guard,
            database: db_guard,
            evidence: evidence_guard,
            saved_request,
            bound: None,
        },
        intent,
        evidence,
        output,
        selector,
    ))
}

pub(super) async fn run(verb: Verb) -> ResultWire {
    let operation_started = std::time::Instant::now();
    if unsafe { libc::getuid() } != 0 || unsafe { libc::geteuid() } != 0 {
        return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible);
    }
    if !env_is_clean() {
        return ResultWire::new(verb, Outcome::Blocked, ErrorCode::WireInvalid);
    }
    if PRIVATE_DIR != "/run/owner-beta-paper-ops" {
        return ResultWire::new(verb, Outcome::Blocked, ErrorCode::InternalFailure);
    }
    let archive_sandbox = match archive_sandbox::install() {
        Ok(guard) => guard,
        Err(code) => return ResultWire::new(verb, Outcome::Blocked, code),
    };
    let (inputs, intent, evidence, output_dir, selector) = match load_context(verb).await {
        Ok(context) => context,
        Err(result) => return result,
    };
    if let Err(code) = archive_sandbox.check_invocation(verb, inputs.bound.as_ref()) {
        return ResultWire::new(verb, Outcome::Blocked, code);
    }
    // The existing RESTORE BEGIN wire selects read-only archive intake before
    // any destination mutation. Full pre-equality remains an independent gate.
    if verb == Verb::Begin
        && inputs
            .bound
            .as_ref()
            .is_some_and(|b| b.root.intent.purpose == "RESTORE" && b.action().verb == Verb::Begin)
    {
        return archive::with_verified_archive(
            &inputs,
            &intent,
            &selector,
            &output_dir,
            operation_started,
            &archive_sandbox,
            begin_verified_archive,
        )
        .await;
    }
    let mut live_begin = if verb == Verb::Begin
        && inputs
            .bound
            .as_ref()
            .is_some_and(|bound| bound.root.maintenance())
    {
        let Some(originals) = inputs.bound.as_ref() else {
            return ResultWire::new(verb, Outcome::Blocked, ErrorCode::EvidenceUnavailable);
        };
        match chain::LiveBeginInputs::open(originals, &inputs) {
            Ok(live) => Some(live),
            Err(code) => return ResultWire::new(verb, Outcome::Blocked, code),
        }
    } else {
        None
    };
    let mut live_maintenance = if matches!(verb, Verb::Reconcile | Verb::Off | Verb::RestoreOff) {
        let Some(originals) = inputs.bound.as_ref() else {
            return ResultWire::new(verb, Outcome::Blocked, ErrorCode::EvidenceUnavailable);
        };
        match chain::LiveMaintenanceInputs::open(originals, &inputs) {
            Ok(live) => Some(live),
            Err(code) => return dispatch_failure(verb, &intent, code),
        }
    } else {
        None
    };
    let result =
        if inputs.bound.as_ref().is_some_and(|b| b.root.maintenance()) && verb != Verb::Readback {
            if let Some(live) = live_begin.as_mut() {
                begin_bound(&intent, &selector, &inputs, &output_dir, live).await
            } else if let Some(live) = live_maintenance.as_mut() {
                maintenance_bound(
                    verb,
                    &selector,
                    &inputs,
                    evidence.as_ref(),
                    &output_dir,
                    live,
                )
                .await
            } else {
                // RESTORE/recovery still require their own protected consumers.
                ResultWire::new(verb, Outcome::Blocked, ErrorCode::EvidenceUnavailable)
            }
        } else if verb == Verb::Recover && inputs.bound.is_some() {
            recover(&intent, evidence.as_ref(), &selector, &inputs, &output_dir).await
        } else if let Some(error) = super::native_follow_up_block(verb) {
            ResultWire::new(verb, Outcome::Blocked, error)
        } else {
            match verb {
                Verb::Configure => {
                    configure(&intent, evidence.as_ref(), &selector, &inputs, &output_dir).await
                }
                Verb::Inspect => inspect(&intent, &selector, &output_dir).await,
                Verb::Enable => {
                    enable(&intent, evidence.as_ref(), &selector, &inputs, &output_dir).await
                }
                Verb::Begin => begin(&intent, &selector, &inputs, &output_dir).await,
                Verb::Readback => readback(&intent, &selector, &inputs, &output_dir).await,
                Verb::ObserveReset => {
                    observe_reset(&intent, evidence.as_ref(), &selector, &inputs, &output_dir).await
                }
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
    let recheck_succeeded = inputs.revalidate().is_ok()
        && output_dir.revalidate().is_ok()
        && live_begin.as_mut().is_none_or(|live| {
            inputs
                .bound
                .as_ref()
                .is_some_and(|originals| live.revalidate(originals, &inputs).is_ok())
        })
        && live_maintenance.as_mut().is_none_or(|live| {
            inputs
                .bound
                .as_ref()
                .is_some_and(|originals| live.revalidate(originals, &inputs).is_ok())
        })
        && !clock_expired;
    finalize_after_recheck(result, recheck_succeeded, verb)
}

async fn readback(
    intent: &Intent,
    selector: &DatabaseSelector,
    inputs: &ProtectedInputs,
    output: &GuardedDirectory,
) -> ResultWire {
    let verb = Verb::Readback;
    let Some((file, request)) = &inputs.saved_request else {
        return ResultWire::new(verb, Outcome::Blocked, ErrorCode::EvidenceUnavailable);
    };
    if file.revalidate().is_err() || inputs.revalidate().is_err() {
        return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible);
    }
    // This path deliberately opens neither Current, Daily, worker credentials,
    // source chains, release/ELFs nor eligibility. It only describes the original.
    let pool = match connect(selector, DatabaseRole::MigrationOwner).await {
        Ok(pool) => pool,
        Err(_) => return query_unknown(intent),
    };
    finish_with_pool!(pool, verb, {
        let readback = match fresh_exact_read(&pool, request).await {
            Ok(value) => value,
            Err(ErrorCode::CommitUnknown) => return query_unknown(intent),
            Err(code) => return ResultWire::new(verb, Outcome::Blocked, code),
        };
        if readback.correlate_original(intent).is_err() {
            return ResultWire::new(verb, Outcome::Blocked, ErrorCode::IdentityConflict);
        }
        if inputs.revalidate().is_err() {
            return query_unknown(intent);
        }
        let bytes = match canonical_text(&readback.value) {
            Ok(bytes) => bytes,
            Err(_) => return query_unknown(intent),
        };
        let hash = match output.write_content_addressed(bytes.as_bytes()) {
            Ok(hash) => hash,
            Err(_) => return query_unknown(intent),
        };
        let (outcome, code) = readback_outcome(&readback);
        let mut result = ResultWire::new(verb, outcome, code);
        result.generation = Some(intent.generation.to_string());
        result.event_id = readback.value["event_id"].as_str().map(str::to_owned);
        result.control_epoch = readback.value["current_epoch"].as_i64();
        result.artifact_sha256 = Some(hash);
        result
    })
}

fn readback_outcome(readback: &ExactReadback) -> (Outcome, ErrorCode) {
    // Called only after the complete existing ExactReadback parser. RESET
    // describes a boundary; it is never promoted to a committed mutation.
    match (
        readback.value["kind"].as_str(),
        readback.value["status"].as_str(),
    ) {
        (Some("RESET"), Some("RESET")) => (Outcome::Ok, ErrorCode::None),
        (Some("CONFIGURE" | "BEGIN" | "RECONCILE" | "TRANSITION"), Some("COMMITTED")) => {
            (Outcome::Replay, ErrorCode::None)
        }
        _ => (Outcome::CommitUnknown, ErrorCode::CommitUnknown),
    }
}

fn query_unknown(intent: &Intent) -> ResultWire {
    let mut result = ResultWire::new(
        Verb::Readback,
        Outcome::CommitUnknown,
        ErrorCode::CommitUnknown,
    );
    result.generation = Some(intent.generation.to_string());
    result
}

fn dispatch_failure(verb: Verb, intent: &Intent, code: ErrorCode) -> ResultWire {
    let mut result = ResultWire::new(
        verb,
        if code == ErrorCode::CommitUnknown {
            Outcome::CommitUnknown
        } else {
            Outcome::Blocked
        },
        code,
    );
    result.generation = Some(intent.generation.to_string());
    result
}

fn observe_current_matches(
    current: &mut super::super::source_evidence::CheckedProtectedCurrentBinding<'_, '_>,
    intent: &Intent,
    process: &Artifact,
) -> Result<(), ErrorCode> {
    let snapshot = current
        .snapshot()
        .map_err(|_| ErrorCode::ReaderIncompatible)?;
    if snapshot.status().map_err(|_| ErrorCode::WireInvalid)? != "CURRENT" {
        return Err(ErrorCode::EvidenceUnavailable);
    }
    let cohort = &snapshot.value()["binding"]["cohort"];
    let compiled = option_env!("LAGRANGE_CODE_COMMIT").ok_or(ErrorCode::EvidenceUnavailable)?;
    if !process_matches_current(&process.value, intent, cohort, compiled) {
        return Err(ErrorCode::IdentityConflict);
    }
    Ok(())
}

async fn observe_reset(
    intent: &Intent,
    process: Option<&Artifact>,
    selector: &DatabaseSelector,
    inputs: &ProtectedInputs,
    output: &GuardedDirectory,
) -> ResultWire {
    let verb = Verb::ObserveReset;
    let Some(process) = process else {
        return ResultWire::new(verb, Outcome::Blocked, ErrorCode::EvidenceUnavailable);
    };
    let pins = match EnablePins::open(intent, process) {
        Ok(pins) => pins,
        Err(code) => return ResultWire::new(verb, Outcome::Blocked, code),
    };
    let attempt = market_data::checked_close::Attempt::default();
    let invocation = market_data::raw_custody::RawInvocation::new(&attempt);
    let mut current = match super::super::source_evidence::ProtectedCurrentBinding::open_checked_in(
        &invocation,
        &attempt,
        &attempt,
    ) {
        Ok(current) => current,
        Err(_) => return ResultWire::new(verb, Outcome::Blocked, ErrorCode::EvidenceUnavailable),
    };
    if let Err(code) = observe_current_matches(&mut current, intent, process) {
        return ResultWire::new(verb, Outcome::Blocked, code);
    }
    let Some(cohort) = process.value["daily_cohort_sha256"].as_str() else {
        return ResultWire::new(verb, Outcome::Blocked, ErrorCode::WireInvalid);
    };
    let (selection_file, selection) = match read_selection(intent, cohort) {
        Ok(selection) => selection,
        Err(code) => return ResultWire::new(verb, Outcome::Blocked, code),
    };
    let Some(account) = intent.account_id else {
        return ResultWire::new(verb, Outcome::Blocked, ErrorCode::IdentityConflict);
    };
    let request = match ExactRequest::reset(account, intent.expected_epoch) {
        Ok(request) => request,
        Err(_) => return ResultWire::new(verb, Outcome::Blocked, ErrorCode::WireInvalid),
    };
    let pool = match connect(selector, DatabaseRole::MigrationOwner).await {
        Ok(pool) => pool,
        Err(code) => return ResultWire::new(verb, Outcome::Blocked, code),
    };
    let result = async {
        let original = match fresh_exact_read(&pool, &request).await {
            Ok(result) => result,
            Err(code) => return ResultWire::new(verb, Outcome::Blocked, code),
        };
        if !matches!(
            original.value["current_state"].as_str(),
            Some("OFF" | "RESTORED_OFF")
        ) || ResetBoundary::parse(&original.value["reset"]).ok().as_ref()
            != Some(&selection.reset)
        {
            return ResultWire::new(verb, Outcome::Blocked, ErrorCode::IdentityConflict);
        }
        // Sample real kernel clocks only after the committed-view RESET completed.
        // B already publishes Current before ARM; no three-session bootstrap path.
        let boot = match local_boot() {
            Ok(boot) => boot,
            Err(code) => return ResultWire::new(verb, Outcome::Blocked, code),
        };
        let observed = match sample_clocks_value() {
            Ok(value) => value,
            Err(code) => return ResultWire::new(verb, Outcome::Blocked, code),
        };
        if inputs.revalidate().is_err()
            || pins.revalidate().is_err()
            || selection_file.revalidate().is_err()
            || observe_current_matches(&mut current, intent, process).is_err()
            || local_boot().ok() != Some(boot)
        {
            return ResultWire::new(verb, Outcome::Blocked, ErrorCode::IdentityConflict);
        }
        // Discarded first snapshot; a distinct fresh snapshot detects a concurrent
        // reset or epoch change rather than rereading the old RR view.
        let repeated = match fresh_exact_read(&pool, &request).await {
            Ok(result) => result,
            Err(code) => return ResultWire::new(verb, Outcome::Blocked, code),
        };
        if repeated.value != original.value {
            return ResultWire::new(verb, Outcome::Blocked, ErrorCode::IdentityConflict);
        }
        let anchor = json!({"schema":"owner-beta-paper-reset-anchor-v1",
            "installation_id":intent.installation_id, "reset":original.value["reset"],
            "release_manifest_sha256":intent.release_manifest_sha256,
            "cohort_sha256":cohort, "boot_id":boot, "observed":observed});
        let bytes = match canonical_text(&anchor) {
            Ok(bytes) => bytes,
            Err(code) => return ResultWire::new(verb, Outcome::Blocked, code),
        };
        if ResetAnchor::parse(bytes.as_bytes()).is_err()
            || inputs.revalidate().is_err()
            || pins.revalidate().is_err()
            || selection_file.revalidate().is_err()
            || observe_current_matches(&mut current, intent, process).is_err()
            || check_clock(&anchor["observed"], &boot.to_string()).is_err()
        {
            return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ClockInvalid);
        }
        let hash = match output.write_content_addressed(bytes.as_bytes()) {
            Ok(hash) => hash,
            Err(_) => {
                return ResultWire::new(verb, Outcome::Blocked, ErrorCode::EvidenceUnavailable);
            }
        };
        if inputs.revalidate().is_err()
            || pins.revalidate().is_err()
            || selection_file.revalidate().is_err()
            || observe_current_matches(&mut current, intent, process).is_err()
            || check_clock(&anchor["observed"], &boot.to_string()).is_err()
        {
            // No successful anchor handoff. P4 must only publish the artifact of
            // an OK result under its release lock, never scan orphaned outputs.
            return ResultWire::new(verb, Outcome::Blocked, ErrorCode::IdentityConflict);
        }
        let mut result = ResultWire::new(verb, Outcome::Ok, ErrorCode::None);
        result.generation = Some(intent.generation.to_string());
        result.event_id = Some(selection.reset.event_id.to_string());
        result.control_epoch = Some(intent.expected_epoch);
        result.artifact_sha256 = Some(hash);
        result
    }
    .await;
    let result = close_pool(pool, result, verb).await;
    let valid = inputs.revalidate().is_ok()
        && pins.revalidate().is_ok()
        && selection_file.revalidate().is_ok()
        && observe_current_matches(&mut current, intent, process).is_ok();
    let completion = current.finish_local();
    let expected = invocation.settle_local_in(&attempt, Ok::<(), std::convert::Infallible>(()));
    let captured = invocation.settle_local_in(&attempt, Ok::<(), std::convert::Infallible>(()));
    let terminal = attempt.finish(Ok::<(), std::convert::Infallible>(()));
    finalize_after_recheck(
        result,
        valid && completion.is_clean() && expected.is_ok() && captured.is_ok() && terminal.is_ok(),
        verb,
    )
}

// The protected migration-owner connection selects originals; no caller DTO,
// copied process file, latest recommendation or grant enum supplies authority.
const RECOVERY_ORIGINAL_SQL: &str = r#"
SELECT jsonb_build_object(
 'lineage',to_jsonb(l)-'created_at',
 'session',jsonb_build_object('state',s.state,'settlement_sha256',s.settlement_sha256,
    'valuation_sha256',s.valuation_sha256,'original_receipt_sha256',
    COALESCE(s.settlement_sha256,public.daily_0060_digest(public.paper_0068_canonical_json(COALESCE(s.receipt_json,'null'::jsonb))))),
 'target_status',t.status,'pending_target_id',t.id,
 'current',jsonb_build_object('state',c.state,'control_epoch',c.control_epoch,
    'consent_event_id',c.consent_event_id,'process_binding',c.process_binding,
    'process_binding_sha256',c.process_binding_sha256,'cohort',c.daily_cohort),
 'original_process',ev.payload->'process_binding',
 'original_process_sha256',ev.payload->'process_binding_sha256',
 'signal_cohort',(SELECT sl.cohort FROM public.owner_beta_paper_source_slots sl
    JOIN public.owner_beta_paper_source_admissions sa USING(slot_id)
    JOIN public.owner_beta_paper_source_runs sr USING(slot_id)
    WHERE sl.owner_user_id=c.owner_user_id AND sl.session_date=l.signal_date
      AND sa.admission_sha256=l.signal_admission_sha256 AND sr.run_id=l.recommendation_run_id),
 'execution_cohort',(SELECT sl.cohort FROM public.owner_beta_paper_source_slots sl
    JOIN public.owner_beta_paper_source_admissions sa USING(slot_id)
    WHERE sl.owner_user_id=c.owner_user_id AND sl.session_date=l.effective_session
      AND sa.admission_sha256=$3::text))
FROM public.owner_beta_paper_control c
JOIN public.pending_targets t ON t.account_id=c.account_id AND t.owner_user_id=c.owner_user_id
JOIN public.owner_beta_paper_lineage l ON l.id=t.beta_lineage_id AND l.account_id=c.account_id
  AND l.owner_user_id=c.owner_user_id AND l.strategy_config_id=c.strategy_config_id
JOIN public.owner_beta_paper_sessions s ON s.pending_target_id=t.id AND s.account_id=c.account_id
  AND s.owner_user_id=c.owner_user_id AND s.effective_session=l.effective_session
JOIN public.owner_beta_paper_events ev ON ev.id=l.activation_event_id AND ev.account_id=l.account_id
  AND ev.owner_user_id=l.owner_user_id AND ev.kind='ENABLED' AND ev.control_epoch=l.control_epoch
WHERE c.id='etf11' AND c.account_id=$1::uuid AND t.id=$2::uuid
  AND t.source_kind IN ('OWNER_BETA_DAILY_MANUAL','OWNER_BETA_DAILY_AUTO')
  AND t.recommendation_run_id IS NULL AND t.strategy_config_id=l.strategy_config_id
  AND t.computed_on=l.signal_date AND t.effective_date=l.effective_session
  AND t.targets_json=l.target_weights_json
"#;

struct RecoveryOriginal {
    value: Value,
    process: Artifact,
    selector: crate::owner_beta::paper::evidence::financial::PaperActionInputRequest,
}

impl RecoveryOriginal {
    fn parse(value: Value, decision: &Artifact, intent: &Intent) -> Result<Self, ErrorCode> {
        use crate::owner_beta::paper::evidence::financial::PaperActionInputRequest;
        let reject = ErrorCode::RecoveryAuthorityUnavailable;
        let bytes = canonical_text(&value)?;
        wire::parse_canonical_limit(bytes.as_bytes(), wire::MAX_PAGE_BYTES).map_err(|_| reject)?;
        let l = &value["lineage"];
        let c = &value["current"];
        let d = &decision.value;
        let process = wire::recovery_process(c["process_binding"].clone()).map_err(|_| reject)?;
        let old = wire::recovery_process(value["original_process"].clone()).map_err(|_| reject)?;
        if intent.purpose != "RECOVERY"
            || d["schema"] != "owner-beta-paper-recovery-v2"
            || intent.account_id.map(|id| id.to_string()).as_deref() != d["account_id"].as_str()
            || intent.value["resume_target_id"] != d["pending_target_id"]
            || d["lineage_epoch"]
                .as_i64()
                .is_none_or(|epoch| epoch < 0 || epoch >= intent.expected_epoch)
            || c["state"] != "ENABLED"
            || c["control_epoch"].as_i64() != Some(intent.expected_epoch)
            || c["control_epoch"] != d["active_epoch"]
            || c["consent_event_id"] != d["consent_event_id"]
            || c["process_binding_sha256"] != process.sha256
            || d["process_config_sha256"] != process.sha256
            || value["original_process_sha256"] != old.sha256
            || value["pending_target_id"] != d["pending_target_id"]
            || l["id"] != d["lineage_id"]
            || l["account_id"] != d["account_id"]
            || l["effective_session"] != d["effective_session"]
            || l["control_epoch"] != d["lineage_epoch"]
            || l["signal_admission_sha256"] != d["signal_admission_sha256"]
            || value["session"]["original_receipt_sha256"] != d["original_receipt_sha256"]
            || !value["session"]["valuation_sha256"].is_null()
            || old.value["control_epoch"] != l["control_epoch"]
            || old.value["consent_event_id"] != l["consent_event_id"]
            || old.value["daily_cohort_sha256"] != l["cohort_sha256"]
            || process.value["control_epoch"] != c["control_epoch"]
            || process.value["consent_event_id"] != c["consent_event_id"]
            || process.value["daily_cohort_sha256"] != c["cohort"]["cohort_sha256"]
        {
            return Err(reject);
        }
        for key in ["owner_user_id", "account_id", "strategy_config_id"] {
            if old.value[key] != l[key] || process.value[key] != l[key] {
                return Err(reject);
            }
        }
        for key in ["financial_contract_sha256", "consent_version"] {
            if old.value[key] != process.value[key] {
                return Err(reject);
            }
        }
        let t = &value["signal_cohort"];
        if t["cohort_sha256"] != l["cohort_sha256"]
            || t["config_epoch"] != l["config_epoch"]
            || t["activation_epoch"] != l["daily_activation_epoch"]
        {
            return Err(reject);
        }
        for cohort in [t, &value["execution_cohort"]] {
            for key in [
                "owner_user_id",
                "policy_sha256",
                "strategy_config_id",
                "strategy_config_sha256",
                "entitlement_reference",
                "entitlement_sha256",
            ] {
                if cohort[key].is_null() || cohort[key] != c["cohort"][key] {
                    return Err(reject);
                }
            }
        }
        let signal = domain::TradingDate::parse(l["signal_date"].as_str().ok_or(reject)?)
            .map_err(|_| reject)?;
        let effective = domain::TradingDate::parse(l["effective_session"].as_str().ok_or(reject)?)
            .map_err(|_| reject)?;
        let run_id = l["recommendation_run_id"]
            .as_str()
            .and_then(|v| Uuid::parse_str(v).ok())
            .filter(|id| !id.is_nil())
            .ok_or(reject)?;
        if signal >= effective {
            return Err(reject);
        }
        let settled = !value["session"]["settlement_sha256"].is_null();
        let selector = match (d["step"].as_str(), settled) {
            (Some("SETTLE_THEN_VALUE"), false)
                if value["target_status"] == "PENDING"
                    && [
                        "PENDING",
                        "WAITING_PRICE",
                        "WAITING_ADMISSION",
                        "HELD",
                        "BLOCKED",
                        "RECOVERY_REQUIRED",
                    ]
                    .contains(&value["session"]["state"].as_str().unwrap_or_default()) =>
            {
                PaperActionInputRequest::Settle {
                    signal,
                    effective,
                    run_id,
                }
            }
            (Some("VALUE_ONLY"), true)
                if (value["target_status"] == "EXECUTED"
                    && value["session"]["state"] == "SETTLED_TRADE")
                    || (value["target_status"] == "SKIPPED"
                        && value["session"]["state"] == "SETTLED_NO_TRADE") =>
            {
                PaperActionInputRequest::Value {
                    signal,
                    effective,
                    run_id,
                }
            }
            _ => return Err(reject),
        };
        Ok(Self {
            value,
            process,
            selector,
        })
    }

    fn check_action(
        &self,
        action: &CheckedPaperAction<'_>,
        decision: &Artifact,
        intent: &Intent,
    ) -> Result<(), ErrorCode> {
        use crate::owner_beta::paper::calculation::{
            validate_next_trading_session, verify_t_e_prefix,
        };
        let reject = ErrorCode::RecoveryAuthorityUnavailable;
        let i = action.inputs();
        let l = &self.value["lineage"];
        let cohort = &i.current_binding()["cohort"];
        let signal = i.signal();
        let execution = i.execution().ok_or(reject)?;
        let t = signal.readback().as_untrusted_value();
        let e = execution.readback().as_untrusted_value();
        let now = chrono::Utc::now();
        let expiry = chrono::DateTime::parse_from_rfc3339(
            decision.value["expires_at"].as_str().ok_or(reject)?,
        )
        .map_err(|_| ErrorCode::ClockInvalid)?
        .with_timezone(&chrono::Utc);
        let compiled = option_env!("LAGRANGE_CODE_COMMIT").ok_or(ErrorCode::EvidenceUnavailable)?;
        if *cohort != self.value["current"]["cohort"]
            || self.process.value["code_commit"] != compiled
            || cohort["verifier_revision"] != compiled
            || self.process.value["release_manifest_sha256"] != intent.release_manifest_sha256
            || cohort["release_manifest_sha256"] != intent.release_manifest_sha256
            || t["candidate_admission_sha256"] != decision.value["signal_admission_sha256"]
            || e["candidate_admission_sha256"] != decision.value["execution_admission_sha256"]
            || t["candidate_run_id"] != l["recommendation_run_id"]
            || t["factor_snapshot_sha256"] != l["factor_snapshot_sha256"]
            || t["target_snapshot_sha256"] != l["target_snapshot_sha256"]
            || signal.snapshot().as_of().to_string() != l["signal_date"].as_str().ok_or(reject)?
            || execution.snapshot().as_of().to_string()
                != l["effective_session"].as_str().ok_or(reject)?
        {
            return Err(reject);
        }
        if i.evaluated_at() > now
            || now.signed_duration_since(i.evaluated_at()).num_seconds() >= 180
            || i.expires_at() <= now
            || expiry <= now
        {
            return Err(ErrorCode::ClockInvalid);
        }
        let owner = cohort["owner_user_id"]
            .as_str()
            .and_then(|v| Uuid::parse_str(v).ok())
            .ok_or(reject)?;
        validate_next_trading_session(
            signal.snapshot(),
            execution.snapshot(),
            i.calendar_range_proof().ok_or(reject)?,
            owner,
            signal.snapshot().policy_sha256(),
        )
        .map_err(|_| reject)?;
        verify_t_e_prefix(signal.snapshot(), execution.snapshot()).map_err(|_| reject)
    }
}

async fn recovery_original(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    intent: &Intent,
    decision: &Artifact,
    lock: bool,
) -> Result<RecoveryOriginal, ErrorCode> {
    let account = intent.account_id.ok_or(ErrorCode::IdentityConflict)?;
    let target = decision.value["pending_target_id"]
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or(ErrorCode::WireInvalid)?;
    let query = if lock {
        "SELECT owner_user_id FROM public.owner_beta_paper_control WHERE id='etf11' AND account_id=$1 FOR UPDATE"
    } else {
        "SELECT owner_user_id FROM public.owner_beta_paper_control WHERE id='etf11' AND account_id=$1"
    };
    let owner: Uuid = sqlx::query_scalar(query)
        .bind(account)
        .fetch_one(&mut **tx)
        .await
        .map_err(|_| ErrorCode::RecoveryAuthorityUnavailable)?;
    let old_actor: Option<String> =
        sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)")
            .fetch_one(&mut **tx)
            .await
            .map_err(|_| ErrorCode::ReaderIncompatible)?;
    sqlx::query("SELECT set_config('app.actor_user_id',$1,true)")
        .bind(owner.to_string())
        .execute(&mut **tx)
        .await
        .map_err(|_| ErrorCode::ReaderIncompatible)?;
    if lock {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text,381901))")
            .bind(account.to_string())
            .execute(&mut **tx)
            .await
            .map_err(|_| ErrorCode::RecoveryAuthorityUnavailable)?;
    }
    let value: Value = sqlx::query_scalar(RECOVERY_ORIGINAL_SQL)
        .bind(account)
        .bind(target)
        .bind(
            decision.value["execution_admission_sha256"]
                .as_str()
                .ok_or(ErrorCode::WireInvalid)?,
        )
        .fetch_one(&mut **tx)
        .await
        .map_err(|_| ErrorCode::RecoveryAuthorityUnavailable)?;
    sqlx::query("SELECT public.paper_0068_restore_actor($1::text)")
        .bind(old_actor)
        .execute(&mut **tx)
        .await
        .map_err(|_| ErrorCode::ReaderIncompatible)?;
    RecoveryOriginal::parse(value, decision, intent)
}

fn recovery_request(intent: &Intent, decision: &Artifact) -> Result<ExactRequest, ErrorCode> {
    make_exact(
        "TRANSITION",
        intent,
        json!({"expected_epoch":intent.expected_epoch,
        "transition":"RECOVERY", "evidence_canonical":canonical_text(&decision.value)?,
        "request_key":intent.request_key}),
    )
}

fn recovery_grant_matches(grant: &Value, response: &Value, decision: &Artifact) -> bool {
    exact_keys(
        grant,
        &[
            "grant_id",
            "active_epoch",
            "active_process_sha256",
            "active_consent_id",
            "step",
            "expires_at",
        ],
    ) && canonical_uuid(&grant["grant_id"])
        && grant["grant_id"] == response["data"]["grant_id"]
        && grant["active_epoch"] == decision.value["active_epoch"]
        && grant["active_process_sha256"] == decision.value["process_config_sha256"]
        && grant["active_consent_id"] == decision.value["consent_event_id"]
        && grant["step"] == decision.value["step"]
        && grant["expires_at"] == decision.value["expires_at"]
}

fn recovery_response_matches(response: &Value, intent: &Intent, decision: &Artifact) -> bool {
    sql_outcome(response, "TRANSITION").is_ok_and(|(outcome, event, epoch)| {
        outcome == Outcome::Ok && event.is_some() && epoch == Some(intent.expected_epoch)
    }) && response["account_id"] == decision.value["account_id"]
        && exact_keys(
            &response["data"],
            &["event_id", "state", "pending_target_id", "grant_id"],
        )
        && response["data"]["state"] == "ENABLED"
        && response["data"]["pending_target_id"] == decision.value["pending_target_id"]
        && canonical_uuid(&response["data"]["grant_id"])
}

async fn fresh_recovery_commit_readback(
    pool: &PgPool,
    request: &ExactRequest,
    decision: &Artifact,
    response: &Value,
) -> Result<ExactReadback, ErrorCode> {
    tokio::time::timeout(Duration::from_secs(15), async {
        let mut tx = pool.begin().await.map_err(|_| ErrorCode::CommitUnknown)?;
        let observed = async {
            sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
                .execute(&mut *tx)
                .await
                .map_err(|_| ErrorCode::CommitUnknown)?;
            let readback = read_exact_result(&mut tx, request).await?;
            // Same independent snapshot checks current control and the exact
            // grant. Legitimate post-COMMIT step consumption does not change its
            // immutable identity or permit another step; SQL owns that decision.
            // Later replay intentionally skips this current-authority check.
            let grant: Value = sqlx::query_scalar(
                r#"
SELECT jsonb_build_object('grant_id',g.grant_id,'active_epoch',g.active_epoch,
 'active_process_sha256',g.process_config_sha256,'active_consent_id',g.consent_event_id,
 'step',g.step,'expires_at',to_char(g.expires_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS"Z"'))
FROM public.owner_beta_paper_recovery_grants g JOIN public.owner_beta_paper_control c
 ON c.account_id=g.account_id AND c.control_epoch=g.active_epoch AND c.state='ENABLED'
 AND c.consent_event_id=g.consent_event_id AND c.process_binding_sha256=g.process_config_sha256
WHERE c.id='etf11' AND g.decision=$1::jsonb AND g.request_key=$2::text
 AND g.revoked_at IS NULL AND g.expires_at>clock_timestamp()
"#,
            )
            .bind(&decision.value)
            .bind(
                request.value["args"]["request_key"]
                    .as_str()
                    .ok_or(ErrorCode::CommitUnknown)?,
            )
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| ErrorCode::CommitUnknown)?;
            if !recovery_grant_matches(&grant, response, decision) {
                return Err(ErrorCode::CommitUnknown);
            }
            Ok(readback)
        }
        .await;
        if !rollback_bounded(tx).await {
            return Err(ErrorCode::CommitUnknown);
        }
        observed
    })
    .await
    .map_err(|_| ErrorCode::CommitUnknown)?
}

fn recovery_known_response(
    response: &Value,
    intent: &Intent,
    output: &GuardedDirectory,
) -> ResultWire {
    let mut result = ResultWire::new(
        Verb::Recover,
        Outcome::CommitUnknown,
        ErrorCode::CommitUnknown,
    );
    result.generation = Some(intent.generation.to_string());
    result.event_id = response["data"]["event_id"].as_str().map(str::to_owned);
    result.control_epoch = response["control_epoch"].as_i64();
    if let Ok(hash) = canonical_text(response).and_then(|bytes| {
        output
            .write_content_addressed(bytes.as_bytes())
            .map_err(|_| ErrorCode::CommitUnknown)
    }) {
        result.artifact_sha256 = Some(hash);
    }
    result
}

fn recovery_committed_matches(readback: &ExactReadback, response: &Value, intent: &Intent) -> bool {
    readback.value["status"] == "COMMITTED"
        && readback.value["current_epoch"].as_i64() == Some(intent.expected_epoch)
        && readback.value["current_state"] == "ENABLED"
        && readback.value["event_id"] == response["data"]["event_id"]
        && canonical_text(response).is_ok_and(|bytes| readback.value["result_canonical"] == bytes)
}

fn recovery_readback_result(
    readback: &ExactReadback,
    intent: &Intent,
    output: &GuardedDirectory,
    replay: bool,
) -> ResultWire {
    if readback.value["status"] != "COMMITTED" {
        return dispatch_failure(Verb::Recover, intent, ErrorCode::CommitUnknown);
    }
    let mut result = ResultWire::new(
        Verb::Recover,
        if replay { Outcome::Replay } else { Outcome::Ok },
        ErrorCode::None,
    );
    result.generation = Some(intent.generation.to_string());
    result.event_id = readback.value["event_id"].as_str().map(str::to_owned);
    result.control_epoch = readback.value["current_epoch"].as_i64();
    match canonical_text(&readback.value).and_then(|bytes| {
        output
            .write_content_addressed(bytes.as_bytes())
            .map_err(|_| ErrorCode::CommitUnknown)
    }) {
        Ok(hash) => result.artifact_sha256 = Some(hash),
        Err(_) => return mark_unknown(result),
    }
    result
}

struct NativeRecoveryGuard<'a> {
    inputs: &'a ProtectedInputs,
    output: &'a GuardedDirectory,
    pins: &'a EnablePins,
}
impl NativeRecoveryGuard<'_> {
    fn check(&self) -> Result<(), ErrorCode> {
        if !env_is_clean() {
            return Err(ErrorCode::WireInvalid);
        }
        self.inputs.revalidate()?;
        self.output
            .revalidate()
            .map_err(|_| ErrorCode::ReaderIncompatible)?;
        self.pins.revalidate()
    }
}
impl crate::owner_beta::paper::evidence::NativePaperActionGuard for NativeRecoveryGuard<'_> {
    fn revalidate(
        &mut self,
    ) -> Result<(), crate::owner_beta::paper::evidence::financial::PaperInputActionError> {
        self.check().map_err(|_| {
            crate::owner_beta::paper::evidence::financial::PaperInputActionError::Runtime
        })
    }
    fn checkpoint(
        &mut self,
    ) -> Result<(), crate::owner_beta::paper::evidence::financial::PaperInputActionError> {
        self.revalidate()
    }
}

struct RecoveryConsumer<'a> {
    intent: &'a Intent,
    decision: &'a Artifact,
    original: &'a RecoveryOriginal,
    request: &'a ExactRequest,
    pool: &'a PgPool,
    output: &'a GuardedDirectory,
    inputs: &'a ProtectedInputs,
    result: Option<ResultWire>,
    commit_started: bool,
    committed: bool,
}
impl crate::owner_beta::paper::evidence::NativePaperActionConsumer for RecoveryConsumer<'_> {
    fn consume<'a, 'hold>(
        &'a mut self,
        action: &'a mut CheckedPaperAction<'hold>,
    ) -> crate::owner_beta::paper::evidence::financial::LocalInputFuture<
        'a,
        Result<(), crate::owner_beta::paper::evidence::financial::PaperInputActionError>,
    >
    where
        'hold: 'a,
    {
        Box::pin(async move {
            use crate::owner_beta::paper::evidence::financial::PaperInputActionError;
            let mut result = self.recover(action).await;
            // This worker transaction is the retained rights/readback fence,
            // not the migration-owner mutation. Finish it explicitly after the
            // independent readback; finish_scope rejects an abandoned Active tx.
            if action.rollback().await.is_err() {
                result = mark_unknown(result);
            }
            if action.revalidate_files().is_err() {
                result = if self.commit_started
                    || self.committed
                    || result.outcome == Outcome::CommitUnknown
                {
                    mark_unknown(result)
                } else {
                    dispatch_failure(
                        Verb::Recover,
                        self.intent,
                        ErrorCode::RecoveryAuthorityUnavailable,
                    )
                };
            }
            let error = match result.outcome {
                Outcome::Ok | Outcome::Replay => None,
                Outcome::Blocked => Some(PaperInputActionError::Runtime),
                Outcome::CommitUnknown => Some(PaperInputActionError::CommitUnknown),
            };
            self.result = Some(result);
            error.map_or(Ok(()), Err)
        })
    }
}
impl RecoveryConsumer<'_> {
    async fn recover(&mut self, action: &mut CheckedPaperAction<'_>) -> ResultWire {
        let verb = Verb::Recover;
        if let Err(code) = self
            .original
            .check_action(action, self.decision, self.intent)
        {
            return dispatch_failure(verb, self.intent, code);
        }
        if action.revalidate_files().is_err()
            || action.begin_transaction().await.is_err()
            || action.revalidate_before_commit().await.is_err()
        {
            return dispatch_failure(verb, self.intent, ErrorCode::RecoveryAuthorityUnavailable);
        }
        let mut tx = match tokio::time::timeout(DB_CONNECT_TIMEOUT, self.pool.begin()).await {
            Ok(Ok(tx)) => tx,
            _ => return dispatch_failure(verb, self.intent, ErrorCode::ReaderIncompatible),
        };
        // Same account/control serialization as frozen 0068; original bytes are
        // selected again after locks and compared before any durable dispatch.
        let original = match tokio::time::timeout(
            Duration::from_secs(15),
            recovery_original(&mut tx, self.intent, self.decision, true),
        )
        .await
        {
            Ok(Ok(original)) if original.value == self.original.value => original,
            _ => {
                return rollback_result(
                    tx,
                    verb,
                    self.intent,
                    ErrorCode::RecoveryAuthorityUnavailable,
                )
                .await;
            }
        };
        if original
            .check_action(action, self.decision, self.intent)
            .is_err()
            || action.revalidate_before_commit().await.is_err()
            || self.inputs.revalidate().is_err()
        {
            return rollback_result(
                tx,
                verb,
                self.intent,
                ErrorCode::RecoveryAuthorityUnavailable,
            )
            .await;
        }
        if let Err(code) = persist_dispatch(
            self.output,
            self.inputs,
            self.intent,
            Some(self.decision),
            self.request,
        ) {
            let result = rollback_result(tx, verb, self.intent, code).await;
            // Acknowledging this unused transaction's rollback does not settle
            // an already-published dispatch owned by a previous invocation.
            return if code == ErrorCode::CommitUnknown {
                mark_unknown(result)
            } else {
                result
            };
        }
        if action.revalidate_files().is_err() {
            return rollback_result(tx, verb, self.intent, ErrorCode::ReaderIncompatible).await;
        }
        let response = tokio::time::timeout(Duration::from_secs(15),
            sqlx::query_scalar::<_, Value>("SELECT public.transition_owner_beta_paper_v1($1::bigint,'RECOVERY'::text,$2::jsonb,$3::text)")
                .bind(self.intent.expected_epoch).bind(&self.decision.value).bind(&self.intent.request_key)
                .fetch_one(&mut *tx)).await;
        let response = match response {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                return rollback_result(tx, verb, self.intent, map_sql_error(&error, verb)).await;
            }
            Err(_) => {
                let _ = rollback_bounded(tx).await;
                return dispatch_failure(verb, self.intent, ErrorCode::CommitUnknown);
            }
        };
        action.retain_value(response.clone());
        if !recovery_response_matches(&response, self.intent, self.decision) {
            return rollback_result(tx, verb, self.intent, ErrorCode::IdentityConflict).await;
        }
        let target = Uuid::parse_str(
            self.decision.value["pending_target_id"]
                .as_str()
                .unwrap_or_default(),
        );
        let Ok(target) = target else {
            return rollback_result(tx, verb, self.intent, ErrorCode::WireInvalid).await;
        };
        let grant = tokio::time::timeout(Duration::from_secs(15),
            sqlx::query_scalar::<_, Value>("SELECT public.paper_0068_recovery_authority($1::uuid,$2::bigint,$3::text,$4::text)")
                .bind(target).bind(self.intent.expected_epoch).bind(&self.original.process.sha256)
                .bind(if self.decision.value["step"] == "VALUE_ONLY" { "VALUE" } else { "SETTLE" })
                .fetch_one(&mut *tx)).await;
        let grant = match grant {
            Ok(Ok(grant)) if recovery_grant_matches(&grant, &response, self.decision) => grant,
            _ => {
                return rollback_result(
                    tx,
                    verb,
                    self.intent,
                    ErrorCode::RecoveryAuthorityUnavailable,
                )
                .await;
            }
        };
        action.retain_value(grant);
        if action.revalidate_before_commit().await.is_err()
            || original
                .check_action(action, self.decision, self.intent)
                .is_err()
            || self.inputs.revalidate().is_err()
            || self.output.revalidate().is_err()
        {
            return rollback_result(
                tx,
                verb,
                self.intent,
                ErrorCode::RecoveryAuthorityUnavailable,
            )
            .await;
        }
        // No await between this last retained-source check and starting COMMIT.
        if action.revalidate_files().is_err() {
            return rollback_result(tx, verb, self.intent, ErrorCode::ReaderIncompatible).await;
        }
        self.commit_started = true;
        if commit_bounded(tx).await.is_err() {
            return recovery_known_response(&response, self.intent, self.output);
        }
        self.committed = true;
        let files_valid = action.revalidate_files().is_ok() && self.inputs.revalidate().is_ok();
        let readback =
            match fresh_recovery_commit_readback(self.pool, self.request, self.decision, &response)
                .await
            {
                Ok(readback) => readback,
                Err(_) => return recovery_known_response(&response, self.intent, self.output),
            };
        let exact = recovery_committed_matches(&readback, &response, self.intent);
        let mut result = recovery_readback_result(&readback, self.intent, self.output, false);
        // Worker/source/rights/current and root custody remain owned through
        // independent readback; their later cleanup can still force UNKNOWN.
        if !files_valid
            || !exact
            || action.revalidate_before_commit().await.is_err()
            || original
                .check_action(action, self.decision, self.intent)
                .is_err()
            || action.revalidate_files().is_err()
            || self.inputs.revalidate().is_err()
        {
            result = mark_unknown(result);
        }
        result
    }
}

async fn recover(
    intent: &Intent,
    decision: Option<&Artifact>,
    selector: &DatabaseSelector,
    inputs: &ProtectedInputs,
    output: &GuardedDirectory,
) -> ResultWire {
    let verb = Verb::Recover;
    let (Some(decision), Some(originals)) = (decision, inputs.bound.as_ref()) else {
        return dispatch_failure(verb, intent, ErrorCode::EvidenceUnavailable);
    };
    let request = match recovery_request(intent, decision) {
        Ok(request) => request,
        Err(code) => return dispatch_failure(verb, intent, code),
    };
    let prior_dispatch =
        match originals.retain_existing_dispatch(output, inputs, decision, &request) {
            Ok(prior) => prior,
            Err(code) => return dispatch_failure(verb, intent, code),
        };
    let pool = match connect(selector, DatabaseRole::MigrationOwner).await {
        Ok(pool) => pool,
        Err(code) => {
            return dispatch_failure(
                verb,
                intent,
                if prior_dispatch {
                    ErrorCode::CommitUnknown
                } else {
                    code
                },
            );
        }
    };
    let mut retained_pins: Option<EnablePins> = None;
    let result = async {
        // Descriptive replay precedes every live/expiry check. ExactReadback's
        // closed parser binds original Q/result/event; no serving permission follows.
        let prior = match fresh_exact_read(&pool, &request).await {
            Ok(prior) => prior,
            Err(code) => {
                return dispatch_failure(
                    verb,
                    intent,
                    if prior_dispatch && code != ErrorCode::IdentityConflict {
                        ErrorCode::CommitUnknown
                    } else {
                        code
                    },
                );
            }
        };
        if prior_dispatch {
            return recovery_readback_result(&prior, intent, output, true);
        }
        if prior.value["status"] == "COMMITTED" {
            // Missing local R-A-E-Q custody is not reconstructed from a SQL receipt.
            return dispatch_failure(verb, intent, ErrorCode::IdentityConflict);
        }
        if prior.value["status"] != "ABSENT" {
            return dispatch_failure(verb, intent, ErrorCode::CommitUnknown);
        }
        let mut tx = match pool.begin().await {
            Ok(tx) => tx,
            Err(_) => return dispatch_failure(verb, intent, ErrorCode::ReaderIncompatible),
        };
        if sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .is_err()
        {
            return rollback_result(tx, verb, intent, ErrorCode::ReaderIncompatible).await;
        }
        let original = tokio::time::timeout(
            Duration::from_secs(15),
            recovery_original(&mut tx, intent, decision, false),
        )
        .await;
        let closed = rollback_bounded(tx).await;
        let original = match original {
            Ok(Ok(original)) if closed => original,
            Ok(Err(code)) if closed => return dispatch_failure(verb, intent, code),
            _ => return dispatch_failure(verb, intent, ErrorCode::ReaderIncompatible),
        };
        let pins = match EnablePins::open(intent, &original.process) {
            Ok(pins) => pins,
            Err(code) => return dispatch_failure(verb, intent, code),
        };
        retained_pins = Some(pins);
        let pins = retained_pins.as_ref().expect("retained recovery pins");
        // One mutation connection plus one retained read connection stays
        // within admin2. Worker remains worker-only, including financial jobs.
        let reader = match connect(selector, DatabaseRole::MigrationOwner).await {
            Ok(reader) => reader,
            Err(code) => return dispatch_failure(verb, intent, code),
        };
        let worker = match connect(selector, DatabaseRole::Worker).await {
            Ok(worker) => worker,
            Err(code) => {
                return close_pool(reader, dispatch_failure(verb, intent, code), verb).await;
            }
        };
        let mut guard = NativeRecoveryGuard {
            inputs,
            output,
            pins,
        };
        let mut consumer = RecoveryConsumer {
            intent,
            decision,
            original: &original,
            request: &request,
            pool: &pool,
            output,
            inputs,
            result: None,
            commit_started: false,
            committed: false,
        };
        let completion = crate::owner_beta::paper::evidence::with_native_checked_recovery_action(
            &worker,
            &reader,
            &decision.value,
            &original.selector,
            original.value["signal_cohort"].clone(),
            original.value["execution_cohort"].clone(),
            &mut guard,
            &mut consumer,
        )
        .await;
        let mut result = consumer.result.take().unwrap_or_else(|| {
            dispatch_failure(verb, intent, ErrorCode::RecoveryAuthorityUnavailable)
        });
        let recheck = guard.check().is_ok();
        let closed = tokio::time::timeout(Duration::from_secs(5), worker.close())
            .await
            .is_ok();
        let reader_closed = tokio::time::timeout(Duration::from_secs(5), reader.close())
            .await
            .is_ok();
        if completion.first.is_some()
            || !completion.secondary.is_empty()
            || !recheck
            || !closed
            || !reader_closed
            || guard.check().is_err()
        {
            result = if consumer.commit_started
                || consumer.committed
                || completion.commit_started
                || completion.committed
                || result.outcome == Outcome::CommitUnknown
            {
                mark_unknown(result)
            } else {
                result
            }; // Keep the first precise pre-dispatch/rollback rejection.
        }
        result
    }
    .await;
    let result = close_pool(pool, result, verb).await;
    finalize_after_recheck(
        result,
        inputs.revalidate().is_ok()
            && output.revalidate().is_ok()
            && retained_pins
                .as_ref()
                .is_none_or(|pins| pins.revalidate().is_ok()),
        verb,
    )
}

struct EnableConsumer<'a> {
    intent: &'a Intent,
    process: &'a Artifact,
    pool: &'a PgPool,
    output: &'a GuardedDirectory,
    result: Option<ResultWire>,
    committed: bool,
    commit_started: bool,
    inputs: &'a ProtectedInputs,
    anchors: &'a [CheckedResetAnchor],
    original_request: Option<ExactRequest>,
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
            "session_evidence": sessions.iter().map(|s| s.sql_projection()).collect::<Vec<_>>(),
            "expected_epoch": self.intent.expected_epoch,
            "new_epoch": new_epoch,
        });
        match super::super::source_evidence::canonical(&evidence) {
            Ok(bytes) if bytes.len() <= wire::MAX_WIRE_BYTES => {}
            _ => return ResultWire::new(verb, Outcome::Blocked, ErrorCode::WireInvalid),
        };
        let request = match canonical_text(&evidence).and_then(|canonical| {
            make_exact(
                "TRANSITION",
                self.intent,
                json!({
                    "expected_epoch":self.intent.expected_epoch, "transition":"ENABLE",
                    "evidence_canonical":canonical, "request_key":self.intent.request_key,
                }),
            )
        }) {
            Ok(request) => request,
            Err(code) => return ResultWire::new(verb, Outcome::Blocked, code),
        };
        if !ENABLE_SQL_RESET_AND_READBACK_READY {
            return ResultWire::new(verb, Outcome::Blocked, ErrorCode::EvidenceUnavailable);
        }
        if action.revalidate_files().is_err() {
            return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible);
        }
        // Retain the actual worker checker/rights transaction while the
        // migration_owner transaction waits, commits and is cleaned up.
        if action.begin_transaction().await.is_err()
            || action.revalidate_before_commit().await.is_err()
        {
            return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible);
        }
        let mut transaction =
            match tokio::time::timeout(Duration::from_secs(10), self.pool.begin()).await {
                Ok(Ok(transaction)) => transaction,
                _ => return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible),
            };
        let reset =
            match read_reset_boundary(&mut transaction, account_id, self.intent.expected_epoch)
                .await
            {
                Ok(reset) => reset,
                Err(code) => return rollback_result(transaction, verb, self.intent, code).await,
            };
        if let Err(code) = check_enable_reset(action, &reset, self.anchors) {
            return rollback_result(transaction, verb, self.intent, code).await;
        }
        if let Err(code) = persist_dispatch(
            self.output,
            self.inputs,
            self.intent,
            Some(self.process),
            &request,
        ) {
            let result = rollback_result(transaction, verb, self.intent, code).await;
            return if code == ErrorCode::CommitUnknown {
                mark_unknown(result)
            } else {
                result
            };
        }
        self.original_request = Some(request);
        if action.revalidate_files().is_err() {
            return rollback_result(
                transaction,
                verb,
                self.intent,
                ErrorCode::ReaderIncompatible,
            )
            .await;
        }
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
                return rollback_result(
                    transaction,
                    verb,
                    self.intent,
                    map_sql_error(&error, verb),
                )
                .await;
            }
            Err(_) => {
                return rollback_result(
                    transaction,
                    verb,
                    self.intent,
                    ErrorCode::ReaderIncompatible,
                )
                .await;
            }
        };
        let parsed = match enable_outcome(&response, account_id, new_epoch) {
            Ok(parsed) => parsed,
            Err(code) => {
                return rollback_result(transaction, verb, self.intent, code).await;
            }
        };
        // The function has acquired and retained its SQL row/advisory locks;
        // this is the post-wait source/current check immediately before commit.
        let locked_reset = match read_reset_boundary(&mut transaction, account_id, parsed.2).await {
            Ok(reset) => reset,
            Err(code) => return rollback_result(transaction, verb, self.intent, code).await,
        };
        if locked_reset != reset || check_enable_reset(action, &reset, self.anchors).is_err() {
            return rollback_result(transaction, verb, self.intent, ErrorCode::IdentityConflict)
                .await;
        }
        if action.revalidate_before_commit().await.is_err()
            || action.revalidate_files().is_err()
            || self.inputs.revalidate().is_err()
            || check_enable_reset(action, &reset, self.anchors).is_err()
        {
            return rollback_result(
                transaction,
                verb,
                self.intent,
                ErrorCode::ReaderIncompatible,
            )
            .await;
        }
        self.commit_started = true;
        if commit_bounded(transaction).await.is_err() {
            return ResultWire::new(verb, Outcome::CommitUnknown, ErrorCode::CommitUnknown);
        }
        self.committed = true;
        // First action after acknowledged COMMIT: recheck the still-borrowed
        // Source/Current/readers/anchors, before hashing or persisting a result.
        let files_after_commit = action.revalidate_files().is_ok()
            && self.inputs.revalidate().is_ok()
            && check_enable_reset(action, &reset, self.anchors).is_ok();
        let reset_after_commit = match ExactRequest::reset(account_id, parsed.2) {
            Ok(request) => fresh_exact_read(self.pool, &request)
                .await
                .is_ok_and(|r| ResetBoundary::parse(&r.value["reset"]).is_ok_and(|r| r == reset)),
            Err(_) => false,
        };
        let rights_after_commit = action.revalidate_before_commit().await.is_ok();
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
        // Check again while every borrowed Source/Current/rights/anchor owner is
        // still alive. A post-COMMIT failure preserves the original event and
        // error/unknown handoff; it never becomes an acknowledged rollback.
        if !files_after_commit
            || !reset_after_commit
            || !rights_after_commit
            || action.revalidate_files().is_err()
            || self.inputs.revalidate().is_err()
            || check_enable_reset(action, &reset, self.anchors).is_err()
        {
            return mark_unknown(result);
        }
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
fn valid_sql_session_projection(sessions: &[Value]) -> bool {
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

fn valid_native_enable_sessions(sessions: &[&CheckedEnableSession]) -> bool {
    if sessions.len() != 3 {
        return false;
    }
    let mut identities = std::collections::BTreeSet::new();
    let mut slots = std::collections::BTreeSet::new();
    let mut arms = std::collections::BTreeSet::new();
    let mut events = std::collections::BTreeSet::new();
    let mut previous = None;
    for session in sessions {
        if previous.is_some_and(|date| date >= session.session_date())
            || !slots.insert(session.slot_id())
            || !arms.insert(session.arm().identities())
        {
            return false;
        }
        previous = Some(session.session_date());
        let (run, job, attempt, reservation) = session.identities();
        if [run, job, attempt, reservation]
            .into_iter()
            .any(|id| !identities.insert(id))
        {
            return false;
        }
        let (intent, armed) = session.arm().digests();
        if !events.insert(intent) || !events.insert(armed) {
            return false;
        }
    }
    valid_sql_session_projection(
        &sessions
            .iter()
            .map(|s| s.sql_projection())
            .collect::<Vec<_>>(),
    )
}

fn mark_unknown(mut result: ResultWire) -> ResultWire {
    result.outcome = Outcome::CommitUnknown;
    result.error_code = ErrorCode::CommitUnknown;
    result
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
    let cohort = process.value["daily_cohort_sha256"]
        .as_str()
        .unwrap_or_default();
    let anchors = match CheckedResetAnchor::open_all(intent, cohort) {
        Ok(anchors) => anchors,
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
        anchors: &anchors,
    };
    let mut consumer = EnableConsumer {
        intent,
        process,
        pool: &migration_pool,
        output,
        result: None,
        committed: false,
        commit_started: false,
        inputs,
        anchors: &anchors,
        original_request: None,
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
        || completion.commit_started
        || result.outcome == Outcome::CommitUnknown;
    if committed_or_unknown
        && (completion.first.is_some()
            || !completion.secondary.is_empty()
            || !recheck
            || !worker_closed
            || !migration_closed)
    {
        result = mark_unknown(result);
        if let Some(request) = &consumer.original_request {
            // Preserve a closed cleanup failure alongside the already-fsynced
            // original. COMMITTED readback is descriptive; startup stays denied
            // and P4 must hand this ENABLE to explicit DRAIN.
            let handoff = json!({"schema":"owner-beta-paper-native-cleanup-v1",
                "generation":intent.generation, "sql_request_sha256":request.sha256,
                "commit_started":consumer.commit_started, "commit_acknowledged":consumer.committed,
                "requires_drain":true, "result":serde_json::to_value(&result).unwrap_or(Value::Null)});
            if let Ok(bytes) = canonical_text(&handoff) {
                let _ = output.write_content_addressed(bytes.as_bytes());
            }
        }
    } else if !committed_or_unknown && (!worker_closed || !migration_closed) {
        result = ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible);
    }
    finalize_after_recheck(result, recheck && worker_closed && migration_closed, verb)
}

async fn configure(
    intent: &Intent,
    artifact: Option<&Artifact>,
    selector: &DatabaseSelector,
    inputs: &ProtectedInputs,
    output: &GuardedDirectory,
) -> ResultWire {
    let verb = Verb::Configure;
    let Some(artifact) = artifact else {
        return ResultWire::new(verb, Outcome::Blocked, ErrorCode::EvidenceUnavailable);
    };
    let request = match make_exact(
        "CONFIGURE",
        intent,
        json!({
            "strategy_config_id":artifact.value["strategy_config_id"],
            "account_name":artifact.value["account_name"],
            "contract_canonical": match canonical_text(&artifact.value["contract"]) {
                Ok(text) => text, Err(code) => return ResultWire::new(verb, Outcome::Blocked, code),
            },
            "process_binding":null, "request_key":intent.request_key,
        }),
    ) {
        Ok(request) => request,
        Err(code) => return ResultWire::new(verb, Outcome::Blocked, code),
    };
    if let Err(code) = persist_dispatch(output, inputs, intent, Some(artifact), &request) {
        return if inputs.bound.is_some() {
            dispatch_failure(verb, intent, code)
        } else {
            ResultWire::new(verb, Outcome::Blocked, code)
        };
    }
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
        if inputs.revalidate().is_err() {
            return rollback_result(tx, verb, intent, ErrorCode::ReaderIncompatible).await;
        }
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
                return rollback_result(tx, verb, intent, map_sql_error(&error, verb)).await;
            }
        };
        let (outcome, event_id, epoch) = match configure_outcome(&response) {
            Ok(v) => v,
            Err(code) => {
                return rollback_result(tx, verb, intent, code).await;
            }
        };
        if inputs.revalidate().is_err() {
            return rollback_result(tx, verb, intent, ErrorCode::ReaderIncompatible).await;
        }
        if commit_bounded(tx).await.is_err() {
            return ResultWire::new(verb, Outcome::CommitUnknown, ErrorCode::CommitUnknown);
        }
        let mut result = ResultWire::new(verb, outcome, ErrorCode::None);
        result.event_id = Some(event_id);
        result.control_epoch = Some(epoch);
        result.artifact_sha256 = Some(artifact.sha256.clone());
        result
    })
}

async fn begin(
    intent: &Intent,
    selector: &DatabaseSelector,
    inputs: &ProtectedInputs,
    output: &GuardedDirectory,
) -> ResultWire {
    let verb = Verb::Begin;
    let request = match canonical_text(&intent.value).and_then(|canonical| {
        make_exact(
            "BEGIN",
            intent,
            json!({"expected_epoch":intent.expected_epoch,
            "intent_canonical":canonical, "request_key":intent.request_key}),
        )
    }) {
        Ok(request) => request,
        Err(code) => return ResultWire::new(verb, Outcome::Blocked, code),
    };
    if let Err(code) = persist_dispatch(output, inputs, intent, None, &request) {
        return if inputs.bound.is_some() {
            dispatch_failure(verb, intent, code)
        } else {
            ResultWire::new(verb, Outcome::Blocked, code)
        };
    }
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
                return rollback_result(tx, verb, intent, map_sql_error(&error, verb)).await;
            }
        };
        let (outcome, event_id, epoch) = match begin_outcome(&response, intent) {
            Ok(value) => value,
            Err(code) => {
                return rollback_result(tx, verb, intent, code).await;
            }
        };
        let mut result = ResultWire::new(verb, outcome, ErrorCode::None);
        result.generation = Some(intent.generation.to_string());
        result.event_id = Some(event_id);
        result.control_epoch = Some(epoch);
        if inputs.revalidate().is_err() {
            return rollback_result(tx, verb, intent, ErrorCode::ReaderIncompatible).await;
        }
        if commit_bounded(tx).await.is_err() {
            return ResultWire::new(verb, Outcome::CommitUnknown, ErrorCode::CommitUnknown);
        }
        result
    })
}

/// Both consumers keep their actual original owners alive. The archive guard
/// can only be constructed with A's borrowed, privately constructed view.
trait BeginGuard {
    fn revalidate(
        &mut self,
        originals: &bound::BoundOriginals,
        inputs: &ProtectedInputs,
    ) -> Result<(), ErrorCode>;
    fn retain_result(&mut self, result: RetainedPublishedOutput) -> Result<(), ErrorCode>;
}
impl BeginGuard for chain::LiveBeginInputs {
    fn revalidate(
        &mut self,
        originals: &bound::BoundOriginals,
        inputs: &ProtectedInputs,
    ) -> Result<(), ErrorCode> {
        chain::LiveBeginInputs::revalidate(self, originals, inputs)
    }
    fn retain_result(&mut self, result: RetainedPublishedOutput) -> Result<(), ErrorCode> {
        chain::LiveBeginInputs::retain_result(self, result)
    }
}
struct ArchiveBeginGuard<'a> {
    view: archive::CheckedArchive<'a>,
    result: Option<RetainedPublishedOutput>,
}
impl BeginGuard for ArchiveBeginGuard<'_> {
    fn revalidate(
        &mut self,
        _originals: &bound::BoundOriginals,
        _inputs: &ProtectedInputs,
    ) -> Result<(), ErrorCode> {
        self.view.revalidate()?;
        if let Some(result) = &self.result {
            result
                .revalidate(self.view.output)
                .map_err(|_| ErrorCode::CommitUnknown)?;
        }
        Ok(())
    }
    fn retain_result(&mut self, result: RetainedPublishedOutput) -> Result<(), ErrorCode> {
        if self.result.is_some() {
            return Err(ErrorCode::IdentityConflict);
        }
        result
            .revalidate(self.view.output)
            .map_err(|_| ErrorCode::CommitUnknown)?;
        self.result = Some(result);
        Ok(())
    }
}
fn begin_verified_archive<'a>(
    view: archive::CheckedArchive<'a>,
) -> archive::ArchiveBeginFuture<'a> {
    Box::pin(async move {
        let (inputs, intent, selector, output, started) = (
            view.inputs,
            view.intent,
            view.selector,
            view.output,
            view.operation_started,
        );
        let mut live = ArchiveBeginGuard { view, result: None };
        if started.elapsed() >= Duration::from_secs(15) || live.view.revalidate().is_err() {
            return ResultWire::new(
                Verb::Begin,
                Outcome::Blocked,
                ErrorCode::EvidenceUnavailable,
            );
        }
        // A's encompassing deadline stays active through dispatch and cleanup.
        // This is the ordinary durable dispatch/SQL path, not a second executor.
        begin_bound(intent, selector, inputs, output, &mut live).await
    })
}

async fn begin_bound(
    intent: &Intent,
    selector: &DatabaseSelector,
    inputs: &ProtectedInputs,
    output: &GuardedDirectory,
    live: &mut impl BeginGuard,
) -> ResultWire {
    let verb = Verb::Begin;
    let Some(originals) = inputs.bound.as_ref() else {
        return ResultWire::new(verb, Outcome::Blocked, ErrorCode::EvidenceUnavailable);
    };
    let request = match canonical_text(&intent.value).and_then(|canonical| {
        make_exact(
            "BEGIN",
            intent,
            json!({"expected_epoch":intent.expected_epoch,
            "intent_canonical":canonical, "request_key":intent.request_key}),
        )
    }) {
        Ok(request) => request,
        Err(code) => return ResultWire::new(verb, Outcome::Blocked, code),
    };
    if live.revalidate(originals, inputs).is_err() {
        return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible);
    }
    if let Err(code) = originals.persist_bound_dispatch(output, inputs, None, &request) {
        return dispatch_failure(verb, intent, code);
    }
    if live.revalidate(originals, inputs).is_err() {
        return ResultWire::new(verb, Outcome::CommitUnknown, ErrorCode::CommitUnknown);
    }
    let Ok(pool) = connect(selector, DatabaseRole::MigrationOwner).await else {
        return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible);
    };
    let pool_to_close = pool.clone();
    let mut result = async {
        let mut tx = match pool.begin().await {
            Ok(tx) => tx,
            Err(_) => {
                return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible);
            }
        };
        if sqlx::query("SET LOCAL lock_timeout = '5s'")
            .execute(&mut *tx)
            .await
            .is_err()
            || sqlx::query("SET LOCAL statement_timeout = '15s'")
                .execute(&mut *tx)
                .await
                .is_err()
        {
            let _ = rollback_bounded(tx).await;
            return ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible);
        }
        if live.revalidate(originals, inputs).is_err() {
            return rollback_result(tx, verb, intent, ErrorCode::ReaderIncompatible).await;
        }
        let response = tokio::time::timeout(
            Duration::from_secs(15),
            sqlx::query_scalar::<_, Value>(
                "SELECT public.begin_owner_beta_paper_operation_v1($1::bigint,$2::jsonb,$3::text)",
            )
            .bind(intent.expected_epoch)
            .bind(intent.value.clone())
            .bind(&intent.request_key)
            .fetch_one(&mut *tx),
        )
        .await;
        let response = match response {
            Ok(Ok(value)) => value,
            Ok(Err(error)) => {
                return rollback_result(tx, verb, intent, map_sql_error(&error, verb)).await;
            }
            Err(_) => {
                let _ = rollback_bounded(tx).await;
                return ResultWire::new(verb, Outcome::CommitUnknown, ErrorCode::CommitUnknown);
            }
        };
        let (outcome, event_id, epoch) = match begin_outcome(&response, intent) {
            Ok(value) => value,
            Err(code) => return rollback_result(tx, verb, intent, code).await,
        };
        // The function returns only after its internal account/advisory locks
        // have fenced the generation. Recheck the original chain at that point.
        if live.revalidate(originals, inputs).is_err() {
            return rollback_result(tx, verb, intent, ErrorCode::ReaderIncompatible).await;
        }
        if inputs.revalidate().is_err() || output.revalidate().is_err() {
            return rollback_result(tx, verb, intent, ErrorCode::ReaderIncompatible).await;
        }
        if live.revalidate(originals, inputs).is_err() {
            return rollback_result(tx, verb, intent, ErrorCode::ReaderIncompatible).await;
        }
        if commit_bounded(tx).await.is_err() {
            return ResultWire::new(verb, Outcome::CommitUnknown, ErrorCode::CommitUnknown);
        }
        let mut committed_result = ResultWire::new(verb, outcome, ErrorCode::None);
        committed_result.generation = Some(intent.generation.to_string());
        committed_result.event_id = Some(event_id);
        committed_result.control_epoch = Some(epoch);
        let original_result = match super::super::source_evidence::canonical(&response) {
            Ok(bytes) => bytes,
            Err(_) => {
                mark_commit_unknown(&mut committed_result);
                return committed_result;
            }
        };
        let result_sha256 = digest(&original_result);
        committed_result.artifact_sha256 = Some(result_sha256.clone());
        let retained_result = match output.write_content_addressed_retained(&original_result) {
            Ok(published) if published.digest == result_sha256 => published,
            Err(_) => {
                mark_commit_unknown(&mut committed_result);
                return committed_result;
            }
            _ => {
                mark_commit_unknown(&mut committed_result);
                return committed_result;
            }
        };
        if live.retain_result(retained_result).is_err() {
            mark_commit_unknown(&mut committed_result);
        }
        committed_result
    }
    .await;
    let cleanup = tokio::time::timeout(Duration::from_secs(5), pool_to_close.close())
        .await
        .is_ok();
    if (!cleanup || live.revalidate(originals, inputs).is_err())
        && matches!(
            result.outcome,
            Outcome::Ok | Outcome::Replay | Outcome::CommitUnknown
        )
    {
        mark_commit_unknown(&mut result);
    }
    result
}

fn maintenance_request(
    originals: &bound::BoundOriginals,
    artifact: &Artifact,
) -> Result<ExactRequest, ErrorCode> {
    maintenance_request_parts(
        &originals.root,
        originals.action(),
        artifact,
        originals
            .artifact_bytes()
            .ok_or(ErrorCode::EvidenceUnavailable)?,
    )
}

// Produces Q only. It cannot construct a live chain or authorize SQL.
fn maintenance_request_parts(
    root: &wire::RootIntent,
    action: &wire::ActionBinding,
    artifact: &Artifact,
    original_evidence: &[u8],
) -> Result<ExactRequest, ErrorCode> {
    if digest(original_evidence) != artifact.sha256
        || action.expected_epoch != root.fence_epoch().map_err(|_| ErrorCode::StaleEpoch)?
    {
        return Err(ErrorCode::IdentityConflict);
    }
    let (kind, args) = match action.verb {
        Verb::Reconcile => (
            "RECONCILE",
            json!({"generation":root.intent.generation,
            "expected_epoch":action.expected_epoch,"inventory_sha256":artifact.sha256,
            "request_key":action.request_key}),
        ),
        Verb::Off | Verb::RestoreOff => (
            "TRANSITION",
            json!({"expected_epoch":action.expected_epoch,
            "transition":if action.verb == Verb::RestoreOff { "RESTORED_OFF" } else { "OFF" },"evidence_canonical":std::str::from_utf8(
                original_evidence).map_err(|_| ErrorCode::WireInvalid)?,"request_key":action.request_key}),
        ),
        _ => return Err(ErrorCode::EvidenceUnavailable),
    };
    let bytes = canonical_text(&json!({"schema":"owner-beta-paper-exact-request-v1",
        "kind":kind,"account_id":root.intent.account_id,"args":args}))?;
    let request = wire::parse_exact_bound(bytes.as_bytes(), root, action)
        .map_err(|_| ErrorCode::IdentityConflict)?;
    wire::correlate_bound(root, action, Some(artifact), &request)
        .map_err(|_| ErrorCode::IdentityConflict)?;
    Ok(request)
}

fn maintenance_outcome(
    value: &Value,
    verb: Verb,
    root: &wire::RootIntent,
    artifact: &Artifact,
) -> Result<(Outcome, String, i64), ErrorCode> {
    let (outcome, event, observed_epoch) = sql_outcome(
        value,
        if verb == Verb::Reconcile {
            "RECONCILE_OPERATION"
        } else {
            "TRANSITION"
        },
    )?;
    let fence = root.fence_epoch().map_err(|_| ErrorCode::StaleEpoch)?;
    let result_epoch = if matches!(verb, Verb::Off | Verb::RestoreOff) {
        fence.checked_add(1).ok_or(ErrorCode::StaleEpoch)?
    } else {
        fence
    };
    if value["account_id"] != root.intent.value["account_id"]
        || !observed_epoch.is_some_and(|epoch| {
            if outcome == Outcome::Replay {
                epoch >= result_epoch
            } else {
                epoch == result_epoch
            }
        })
    {
        return Err(ErrorCode::IdentityConflict);
    }
    let data = &value["data"];
    let event = if verb == Verb::Reconcile {
        if !exact_keys(data, &["observation_id", "summary", "summary_sha256"]) {
            return Err(ErrorCode::WireInvalid);
        }
        wire::validate_summary(&data["summary"]).map_err(|_| ErrorCode::WireInvalid)?;
        let summary = &data["summary"];
        if !canonical_uuid(&data["observation_id"])
            || data["observation_id"] != summary["observation_id"]
            || summary["installation_id"] != root.intent.installation_id.to_string()
            || summary["generation"] != root.intent.generation.to_string()
            || summary["account_id"] != root.intent.value["account_id"]
            || summary["control_epoch"] != fence
            || summary["inventory_sha256"] != artifact.sha256
            || data["summary_sha256"] != digest(canonical_text(summary)?.as_bytes())
        {
            return Err(ErrorCode::IdentityConflict);
        }
        data["observation_id"]
            .as_str()
            .ok_or(ErrorCode::WireInvalid)?
            .to_owned()
    } else if matches!(verb, Verb::Off | Verb::RestoreOff) {
        if !exact_keys(data, &["event_id", "state"])
            || (outcome == Outcome::Ok
                && data["state"]
                    != if verb == Verb::RestoreOff {
                        "RESTORED_OFF"
                    } else {
                        "OFF"
                    })
        {
            return Err(ErrorCode::IdentityConflict);
        }
        event.ok_or(ErrorCode::WireInvalid)?
    } else {
        return Err(ErrorCode::EvidenceUnavailable);
    };
    Ok((outcome, event, result_epoch))
}

async fn maintenance_backends_absent(
    connection: &mut sqlx::PgConnection,
    captured: &[i32],
) -> Result<(), ErrorCode> {
    // PID is visible to an ordinary direct login even when another role's query
    // text/start fields are masked. We require total client absence (except this
    // actual connection), not an application-name/role-selected approximation.
    // A reused captured PID also fails closed; no signaling happens in native.
    sqlx::query("SELECT pg_catalog.pg_stat_clear_snapshot()")
        .execute(&mut *connection)
        .await
        .map_err(|_| ErrorCode::WriterUnknown)?;
    let clean = sqlx::query_scalar::<_, bool>(
        "SELECT current_user='migration_owner' AND session_user='migration_owner' \
         AND current_setting('role')='none' \
         AND EXISTS(SELECT 1 FROM pg_catalog.pg_stat_activity WHERE pid=pg_backend_pid() \
                    AND backend_start IS NOT NULL AND datname=current_database()) \
         AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_stat_activity WHERE pid=ANY($1::int[]) \
                        OR (datname=current_database() AND pid<>pg_backend_pid() \
                            AND backend_type IS DISTINCT FROM 'autovacuum worker')) \
         AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_prepared_xacts WHERE database=current_database())",
    ).bind(captured).fetch_one(connection).await.map_err(|_| ErrorCode::WriterUnknown)?;
    if clean {
        Ok(())
    } else {
        Err(ErrorCode::WriterUnknown)
    }
}

fn publish_maintenance_result(
    output: &GuardedDirectory,
    live: &mut chain::LiveMaintenanceInputs,
    bytes: &[u8],
    value: &Value,
    result: &mut ResultWire,
) -> Result<(), ErrorCode> {
    result.artifact_sha256 = Some(digest(bytes));
    let retained = output
        .write_content_addressed_retained(bytes)
        .map_err(|_| ErrorCode::CommitUnknown)?;
    live.retain_result(retained)?;
    if value["action"] == "RECONCILE_OPERATION" {
        let summary = canonical_text(&value["data"]["summary"])?;
        let retained = output
            .write_content_addressed_retained(summary.as_bytes())
            .map_err(|_| ErrorCode::CommitUnknown)?;
        live.retain_result(retained)?;
    }
    Ok(())
}

async fn maintenance_bound(
    verb: Verb,
    selector: &DatabaseSelector,
    inputs: &ProtectedInputs,
    artifact: Option<&Artifact>,
    output: &GuardedDirectory,
    live: &mut chain::LiveMaintenanceInputs,
) -> ResultWire {
    let (Some(originals), Some(artifact)) = (inputs.bound.as_ref(), artifact) else {
        return ResultWire::new(verb, Outcome::Blocked, ErrorCode::EvidenceUnavailable);
    };
    let root = &originals.root;
    let request = match maintenance_request(originals, artifact) {
        Ok(request) => request,
        Err(code) => return dispatch_failure(verb, &root.intent, code),
    };
    // Fixed dispatch publication precedes even the read-only prior checks.
    // An existing index returns UNKNOWN without opening a second SQL connection.
    if let Err(code) = originals.persist_bound_dispatch(output, inputs, Some(artifact), &request) {
        return dispatch_failure(verb, &root.intent, code);
    }
    let pool = match connect(selector, DatabaseRole::MigrationOwner).await {
        Ok(pool) => pool,
        Err(code) => return dispatch_failure(verb, &root.intent, code),
    };
    let result = async {
        // An independent committed BEGIN and every preceding reconciliation are
        // mandatory. A proposed fence/floor or protected summary alone is not SQL proof.
        for index in 0..live.prior.len() {
            let readback = match fresh_exact_read(&pool, &live.prior[index].request).await {
                Ok(readback) => readback,
                Err(code) => return dispatch_failure(verb, &root.intent, code),
            };
            if let Err(code) = live.prior[index].verify_readback(&readback, live.fence) {
                return dispatch_failure(verb, &root.intent, code);
            }
            if let Err(code) = live.revalidate(originals, inputs) {
                return dispatch_failure(verb, &root.intent, code);
            }
        }
        let mut tx = match pool.begin().await {
            Ok(tx) => tx,
            Err(_) => return dispatch_failure(verb, &root.intent, ErrorCode::CommitUnknown),
        };
        if sqlx::query("SET LOCAL lock_timeout = '5s'").execute(&mut *tx).await.is_err()
            || sqlx::query("SET LOCAL statement_timeout = '15s'").execute(&mut *tx).await.is_err() {
            return rollback_result(tx, verb, &root.intent, ErrorCode::ReaderIncompatible).await;
        }
        if let Err(code) = live.revalidate(originals, inputs) {
            return rollback_result(tx, verb, &root.intent, code).await;
        }
        if let Err(code) = maintenance_backends_absent(&mut tx, &live.captured_pids).await {
            return rollback_result(tx, verb, &root.intent, code).await;
        }
        let response = tokio::time::timeout(Duration::from_secs(15), async {
            if verb == Verb::Reconcile {
                sqlx::query_scalar::<_, Value>(
                    "SELECT public.reconcile_owner_beta_paper_operation_v1($1::uuid,$2::bigint,$3::text,$4::text)")
                    .bind(root.intent.generation).bind(live.fence).bind(&artifact.sha256)
                    .bind(&originals.action().request_key).fetch_one(&mut *tx).await
            } else {
                sqlx::query_scalar::<_, Value>(
                    "SELECT public.transition_owner_beta_paper_v1($1::bigint,$2::text,$3::jsonb,$4::text)")
                    .bind(live.fence).bind(if verb == Verb::RestoreOff { "RESTORED_OFF" } else { "OFF" }).bind(artifact.value.clone())
                    .bind(&originals.action().request_key).fetch_one(&mut *tx).await
            }
        }).await;
        let response = match response {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => return rollback_result(tx, verb, &root.intent, map_sql_error(&error, verb)).await,
            Err(_) => {
                let _ = rollback_bounded(tx).await;
                return dispatch_failure(verb, &root.intent, ErrorCode::CommitUnknown);
            }
        };
        let (outcome, event, epoch) = match maintenance_outcome(&response, verb, root, artifact) {
            Ok(value) => value,
            Err(code) => return rollback_result(tx, verb, &root.intent, code).await,
        };
        // SQL has now taken the control/account locks. Recheck source-owned
        // originals and native backend absence under that same transaction.
        if let Err(code) = live.revalidate(originals, inputs) {
            return rollback_result(tx, verb, &root.intent, code).await;
        }
        if let Err(code) = maintenance_backends_absent(&mut tx, &live.captured_pids).await {
            return rollback_result(tx, verb, &root.intent, code).await;
        }
        if inputs.revalidate().is_err() || output.revalidate().is_err() {
            return rollback_result(tx, verb, &root.intent, ErrorCode::ReaderIncompatible).await;
        }
        let mut result = ResultWire::new(verb, outcome, ErrorCode::None);
        result.generation = Some(root.intent.generation.to_string());
        result.event_id = Some(event);
        result.control_epoch = Some(epoch);
        let response_bytes = match canonical_text(&response) {
            Ok(bytes) => bytes,
            Err(code) => return rollback_result(tx, verb, &root.intent, code).await,
        };
        result.artifact_sha256 = Some(digest(response_bytes.as_bytes()));
        if commit_bounded(tx).await.is_err() {
            // Preserve known response identities/bytes, but only a later separate
            // exact readback can decide COMMITTED. Never resubmit this dispatch.
            let _ = publish_maintenance_result(output, live, response_bytes.as_bytes(), &response, &mut result);
            mark_commit_unknown(&mut result);
            return result;
        }
        if live.revalidate(originals, inputs).is_err() {
            let _ = publish_maintenance_result(output, live, response_bytes.as_bytes(), &response, &mut result);
            mark_commit_unknown(&mut result);
            return result;
        }
        let readback = match fresh_exact_read(&pool, &request).await {
            Ok(readback) => readback,
            Err(_) => {
                let _ = publish_maintenance_result(output, live, response_bytes.as_bytes(), &response, &mut result);
                mark_commit_unknown(&mut result);
                return result;
            }
        };
        let original = &readback.value;
        if original["status"] != "COMMITTED" || original["event_id"].as_str() != result.event_id.as_deref()
            || original["event_epoch"] != epoch || original["current_epoch"] != epoch
            || original["current_state"] != if verb == Verb::RestoreOff { "RESTORED_OFF" } else if verb == Verb::Off { "OFF" } else { "DRAINING" }
            || (outcome == Outcome::Ok && original["result_canonical"] != response_bytes) {
            let _ = publish_maintenance_result(output, live, response_bytes.as_bytes(), &response, &mut result);
            mark_commit_unknown(&mut result);
            return result;
        }
        let Some(bytes) = original["result_canonical"].as_str() else {
            mark_commit_unknown(&mut result);
            return result;
        };
        let value = match wire::parse_canonical_limit(bytes.as_bytes(), wire::MAX_PAGE_BYTES) {
            Ok(value) => value,
            Err(_) => { mark_commit_unknown(&mut result); return result; }
        };
        if publish_maintenance_result(output, live, bytes.as_bytes(), &value, &mut result).is_err()
            || live.revalidate(originals, inputs).is_err() {
            mark_commit_unknown(&mut result);
        }
        result
    }.await;
    let result = close_pool(pool, result, verb).await;
    finalize_after_recheck(result, live.revalidate(originals, inputs).is_ok(), verb)
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
        .bind(if generation.is_none() { 1 } else { INSPECT_PAGE_LIMIT })
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
            if generation.is_none() && next_cursor.is_some() {
                let _ = rollback_bounded(tx).await;
                return ResultWire::new(verb, Outcome::Blocked, ErrorCode::EvidenceUnavailable);
            }
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

// Maintenance mutations remain blocked by native_follow_up_block. No dormant
// SQL dispatch is retained without the future protected stage-chain consumer.

#[cfg(test)]
mod tests {
    use super::{
        CLOCK_FRESHNESS_NS, DatabaseRole, MIGRATION_PASSWORD, Outcome, WORKER_PASSWORD,
        begin_outcome, clock_samples_fresh, configure_outcome, digest, enable_outcome,
        finalize_after_recheck, inspect_page_next, parse_database, process_matches_current,
        same_object, valid_sql_session_projection,
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
    fn readback_classifies_reset_separately_from_committed_mutations() {
        for (kind, status, expected) in [
            ("RESET", "RESET", "OK"),
            ("TRANSITION", "COMMITTED", "REPLAY"),
            ("BEGIN", "ABSENT", "COMMIT_UNKNOWN"),
            ("CONFIGURE", "UNPROVEN", "COMMIT_UNKNOWN"),
            ("RESET", "COMMITTED", "COMMIT_UNKNOWN"),
            ("TRANSITION", "RESET", "COMMIT_UNKNOWN"),
        ] {
            let readback = super::ExactReadback {
                value: serde_json::json!({"kind":kind,"status":status}),
            };
            let (outcome, code) = super::readback_outcome(&readback);
            let wire = ResultWire::new(Verb::Readback, outcome, code);
            let result: serde_json::Value = serde_json::from_slice(&wire.encode()).unwrap();
            assert_eq!(result["outcome"], expected, "{kind}/{status}");
            assert_eq!(
                result["error_code"],
                if expected == "COMMIT_UNKNOWN" {
                    "COMMIT_UNKNOWN"
                } else {
                    "NONE"
                }
            );
        }
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
        assert!(clock_samples_fresh(
            CLOCK_FRESHNESS_NS - 1,
            CLOCK_FRESHNESS_NS - 1,
            CLOCK_FRESHNESS_NS - 1
        ));
        assert!(!clock_samples_fresh(CLOCK_FRESHNESS_NS, 4, 4));
        assert!(!clock_samples_fresh(-1, 4, 4));
        assert!(!clock_samples_fresh(4, 4, 1_000_000_005));
        assert!(!clock_samples_fresh(
            4,
            CLOCK_FRESHNESS_NS,
            CLOCK_FRESHNESS_NS
        ));
        assert!(!clock_samples_fresh(CLOCK_FRESHNESS_NS - 1, 4, 4));
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
            gid: 0,
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
    fn cleanup_unknown_preserves_original_event_and_durable_artifact_identity() {
        let mut original = ResultWire::new(Verb::Enable, Outcome::Ok, ErrorCode::None);
        original.generation = Some("11111111-1111-4111-8111-111111111111".to_owned());
        original.event_id = Some("22222222-2222-4222-8222-222222222222".to_owned());
        original.control_epoch = Some(1);
        original.artifact_sha256 = Some(format!("sha256:{}", "a".repeat(64)));
        let before = serde_json::to_value(&original).unwrap();
        let unknown = finalize_after_recheck(original, false, Verb::Enable);
        let after = serde_json::to_value(unknown).unwrap();
        for key in ["event_id", "generation", "control_epoch", "artifact_sha256"] {
            assert_eq!(before[key], after[key], "{key}");
        }
        assert_eq!(after["outcome"], "COMMIT_UNKNOWN");
    }

    #[test]
    fn official_producer_proof_requires_exact_complete_env_bin_and_actual_elf_hash() {
        let h = "a".repeat(64);
        let revision = "b".repeat(40);
        let receipt = serde_json::json!({
            "format":"lagrange-rust-artifact-v1","binary_mode":"0755","binary_sha256":h,
            "bin":"owner-beta-paper-ops-native","cache_key":h,"cargo_success":true,
            "compile_env":{"CARGO_BUILD_JOBS":"2","CARGO_TARGET_DIR":"/cargo-target",
                "RUSTFLAGS":"<unset>","LAGRANGE_CODE_COMMIT":revision},
            "features":[],"h_sha256":h,"host_triple":"x86_64-unknown-linux-musl",
            "input_sha256":h,"package":"api-server","platform":"linux/amd64",
            "profile":"release","recipe_sha256":h,"source_commit":revision
        });
        let encode = |value: &serde_json::Value| {
            let mut bytes = crate::owner_beta::paper::source_evidence::canonical(value).unwrap();
            bytes.push(b'\n');
            bytes
        };
        let bytes = encode(&receipt);
        let complete = format!("{}\n", &digest(&bytes)[7..]);
        let elf = format!("sha256:{h}");
        assert!(
            super::validate_producer_receipt(
                &bytes,
                complete.as_bytes(),
                "owner-beta-paper-ops-native",
                &revision,
                &elf
            )
            .is_ok()
        );
        for (key, value) in [
            ("bin", serde_json::json!("paper-runner")),
            ("cargo_success", serde_json::json!(false)),
            ("package", serde_json::json!("job-queue")),
            ("platform", serde_json::json!("linux/arm64")),
            ("binary_mode", serde_json::json!("0500")),
            ("extra", serde_json::json!(true)),
            ("binary_sha256", serde_json::json!("c".repeat(64))),
            ("source_commit", serde_json::json!("c".repeat(40))),
            (
                "compile_env",
                serde_json::json!({"LAGRANGE_CODE_COMMIT":revision}),
            ),
        ] {
            let mut bad = receipt.clone();
            bad[key] = value;
            let bytes = encode(&bad);
            let complete = format!("{}\n", &digest(&bytes)[7..]);
            assert!(
                super::validate_producer_receipt(
                    &bytes,
                    complete.as_bytes(),
                    "owner-beta-paper-ops-native",
                    &revision,
                    &elf
                )
                .is_err(),
                "{key}"
            );
        }
        assert!(
            super::validate_producer_receipt(
                &bytes,
                b"wrong",
                "owner-beta-paper-ops-native",
                &revision,
                &elf
            )
            .is_err()
        );
        for malformed in [
            bytes[..bytes.len() - 1].to_vec(),
            [bytes.as_slice(), b"\n"].concat(),
        ] {
            let matching_complete = format!("{}\n", &digest(&malformed)[7..]);
            assert!(
                super::validate_producer_receipt(
                    &malformed,
                    matching_complete.as_bytes(),
                    "owner-beta-paper-ops-native",
                    &revision,
                    &elf
                )
                .is_err()
            );
        }
    }

    #[test]
    fn reset_selection_requires_all_original_fields_release_cohort_and_installation() {
        let h = format!("sha256:{}", "a".repeat(64));
        let value = serde_json::json!({"schema":"owner-beta-paper-reset-selection-v1",
            "installation_id":"11111111-1111-4111-8111-111111111111",
            "release_manifest_sha256":h,"cohort_sha256":h,
            "reset":{"account_id":"22222222-2222-4222-8222-222222222222",
                "event_id":"33333333-3333-4333-8333-333333333333","kind":"OFF","control_epoch":4,
                "occurred_at_us":1234567,"payload_sha256":h}});
        let parse = |value: &serde_json::Value| {
            super::ResetSelection::parse(super::canonical_text(value).unwrap().as_bytes()).unwrap()
        };
        let original = parse(&value);
        assert!(super::selection_matches(&original, &parse(&value)));
        for (key, replacement) in [
            (
                "installation_id",
                serde_json::json!("44444444-4444-4444-8444-444444444444"),
            ),
            (
                "release_manifest_sha256",
                serde_json::json!(format!("sha256:{}", "b".repeat(64))),
            ),
            (
                "cohort_sha256",
                serde_json::json!(format!("sha256:{}", "b".repeat(64))),
            ),
        ] {
            let mut changed = value.clone();
            changed[key] = replacement;
            assert!(
                !super::selection_matches(&original, &parse(&changed)),
                "{key}"
            );
        }
        for (key, replacement) in [
            (
                "account_id",
                serde_json::json!("44444444-4444-4444-8444-444444444444"),
            ),
            (
                "event_id",
                serde_json::json!("44444444-4444-4444-8444-444444444444"),
            ),
            ("kind", serde_json::json!("RESTORED_OFF")),
            ("control_epoch", serde_json::json!(5)),
            ("occurred_at_us", serde_json::json!(1234568)),
            (
                "payload_sha256",
                serde_json::json!(format!("sha256:{}", "b".repeat(64))),
            ),
        ] {
            let mut changed = value.clone();
            changed["reset"][key] = replacement;
            assert!(
                !super::selection_matches(&original, &parse(&changed)),
                "reset.{key}"
            );
        }
    }

    // Existing hosted Root fixture only. These tests do not create genuine
    // Source/ARM authority and must never be counted as an Origin test.
    struct OsTree {
        path: &'static str,
    }
    impl OsTree {
        fn new() -> Self {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                unsafe { libc::geteuid() },
                0,
                "hosted disposable Root fixture required"
            );
            assert!(
                std::path::Path::new("/run/r5-paper-checker-integration").is_dir(),
                "existing R5 fixture required"
            );
            let path: String = format!(
                "/run/r5-paper-checker-integration/native-readback-{}",
                uuid::Uuid::new_v4()
            );
            std::fs::create_dir(&path).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            Self {
                path: Box::leak(path.into_boxed_str()),
            }
        }
        fn child(&self, name: &str) -> &'static str {
            Box::leak(format!("{}/{name}", self.path).into_boxed_str())
        }
        fn file(&self, name: &str, bytes: &[u8]) -> &'static str {
            use std::{io::Write, os::unix::fs::OpenOptionsExt};
            let path = self.child(name);
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o400)
                .open(path)
                .unwrap();
            file.write_all(bytes).unwrap();
            file.sync_all().unwrap();
            path
        }
    }
    impl Drop for OsTree {
        fn drop(&mut self) {
            std::fs::remove_dir_all(self.path).unwrap();
        }
    }

    #[test]
    #[ignore = "Existing hosted disposable Root R5 fixture; real FD/metadata/named identity checks"]
    fn actual_native_protected_original_rejects_replace_link_symlink_and_mode_change() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let tree = OsTree::new();
        assert!(super::GuardedFile::open(tree.child("missing.json")).is_err());
        let path = tree.file("original.json", b"original");
        let mut original = super::GuardedFile::open(path).unwrap();
        assert_eq!(original.read_bounded(64).unwrap(), b"original");
        std::fs::rename(path, tree.child("retained.json")).unwrap();
        tree.file("original.json", b"original");
        assert!(original.revalidate().is_err());
        std::fs::remove_file(path).unwrap();
        std::fs::rename(tree.child("retained.json"), path).unwrap();
        let original = super::GuardedFile::open(path).unwrap();
        std::fs::hard_link(path, tree.child("link.json")).unwrap();
        assert!(original.revalidate().is_err());
        std::fs::remove_file(tree.child("link.json")).unwrap();
        let original = super::GuardedFile::open(path).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(original.revalidate().is_err());
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o400)).unwrap();
        let original = super::GuardedFile::open(path).unwrap();
        std::fs::rename(path, tree.child("retained.json")).unwrap();
        symlink(tree.child("retained.json"), path).unwrap();
        assert!(original.revalidate().is_err());
        drop(original);
    }

    #[test]
    #[ignore = "Existing hosted disposable Root R5 fixture; actual fsync/no-replace custody, no SQL mutation"]
    fn actual_native_dispatch_custody_is_durable_before_dispatch_and_survives_unknown() {
        let tree = OsTree::new();
        let h = format!("sha256:{}", "a".repeat(64));
        let artifact = serde_json::json!({"schema":"owner-beta-paper-configure-operation-v1",
            "strategy_config_id":"33333333-3333-4333-8333-333333333333","account_name":"Owner-beta Paper",
            "contract":{"contract_id":"owner-beta-paper-v1-draft-1",
                "contract_sha256":"sha256:18cde9fa7f15e19c9ec272abb780d06ae4be672068ed3f9208c39dc2297ab8d0",
                "decision_sha256":"sha256:8ef602a35586d3e2ca532c5fbb5efc6f2e727eb25ebeb45a44dad0ea0d0ca394",
                "cost_profile_id":"KRX_ETF_DEFAULT","cost_profile_version":1}});
        let artifact_bytes = super::canonical_text(&artifact).unwrap();
        let artifact = super::Artifact {
            value: artifact,
            sha256: digest(artifact_bytes.as_bytes()),
        };
        let intent_value = serde_json::json!({"schema":"owner-beta-paper-operation-intent-v1",
            "installation_id":"11111111-1111-4111-8111-111111111111",
            "generation":"22222222-2222-4222-8222-222222222222","account_id":null,"predecessor_generation":null,
            "request_key":"original","purpose":"CONFIGURE","scope":"PAPER_STOP","expected_epoch":0,"local_epoch_floor":0,
            "release_manifest_sha256":h,"artifact_sha256":artifact.sha256,"resume_target_id":null});
        let intent_bytes = super::canonical_text(&intent_value).unwrap();
        let intent = parse_intent(intent_bytes.as_bytes(), Verb::Configure).unwrap();
        let inputs = super::ProtectedInputs {
            request: super::GuardedFile::open(tree.file("request.json", intent_bytes.as_bytes()))
                .unwrap(),
            evidence: Some(
                super::GuardedFile::open(tree.file("evidence.json", artifact_bytes.as_bytes()))
                    .unwrap(),
            ),
            database: super::GuardedFile::open(tree.file("database.json", b"selector")).unwrap(),
            saved_request: None,
            bound: None,
        };
        let output = super::GuardedDirectory::open(tree.path).unwrap();
        let request=super::make_exact("CONFIGURE",&intent,serde_json::json!({
            "strategy_config_id":artifact.value["strategy_config_id"],"account_name":"Owner-beta Paper",
            "contract_canonical":super::canonical_text(&artifact.value["contract"]).unwrap(),
            "process_binding":null,"request_key":"original"})).unwrap();
        super::persist_dispatch(&output, &inputs, &intent, Some(&artifact), &request).unwrap();
        let saved = tree.child(&format!("{}.json", &request.sha256[7..]));
        assert_eq!(std::fs::read(saved).unwrap(), request.canonical.as_bytes());
        assert!(request.value["account_id"].is_null());
        let _unknown = super::mark_unknown(ResultWire::new(
            Verb::Configure,
            Outcome::Ok,
            ErrorCode::None,
        ));
        assert_eq!(std::fs::read(saved).unwrap(), request.canonical.as_bytes());
        // Failure of the durable handoff prevents reaching the submission point.
        std::fs::rename(tree.child("request.json"), tree.child("moved.json")).unwrap();
        let mut submissions = 0;
        if super::persist_dispatch(&output, &inputs, &intent, Some(&artifact), &request).is_ok() {
            submissions += 1;
        }
        assert_eq!(submissions, 0);
    }

    struct SourceProbeGuard<'a> {
        inputs: &'a super::ProtectedInputs,
        anchors: &'a [super::CheckedResetAnchor],
    }
    impl crate::owner_beta::paper::evidence::NativePaperActionGuard for SourceProbeGuard<'_> {
        fn revalidate(
            &mut self,
        ) -> Result<(), crate::owner_beta::paper::evidence::financial::PaperInputActionError>
        {
            self.inputs
                .revalidate()
                .and_then(|_| {
                    self.anchors
                        .iter()
                        .try_for_each(|anchor| anchor.revalidate())
                })
                .map_err(|_| {
                    crate::owner_beta::paper::evidence::financial::PaperInputActionError::Runtime
                })
        }
        fn checkpoint(
            &mut self,
        ) -> Result<(), crate::owner_beta::paper::evidence::financial::PaperInputActionError>
        {
            self.revalidate()
        }
    }
    struct SourceResetProbe<'a> {
        pool: &'a sqlx::PgPool,
        intent: &'a super::Intent,
        anchors: &'a [super::CheckedResetAnchor],
        result: Option<Result<(), ErrorCode>>,
    }
    impl crate::owner_beta::paper::evidence::NativePaperActionConsumer for SourceResetProbe<'_> {
        fn consume<'a, 'hold>(
            &'a mut self,
            action: &'a mut crate::owner_beta::paper::evidence::financial::CheckedPaperAction<
                'hold,
            >,
        ) -> crate::owner_beta::paper::evidence::financial::LocalInputFuture<
            'a,
            Result<(), crate::owner_beta::paper::evidence::financial::PaperInputActionError>,
        >
        where
            'hold: 'a,
        {
            Box::pin(async move {
                action.revalidate_files()?;
                let request = super::ExactRequest::reset(
                    self.intent.account_id.unwrap(),
                    self.intent.expected_epoch,
                )
                .unwrap();
                let readback = super::fresh_exact_read(self.pool, &request).await;
                self.result = Some(match readback {
                    Ok(readback) => super::ResetBoundary::parse(&readback.value["reset"])
                        .map_err(|_| ErrorCode::WireInvalid)
                        .and_then(|reset| super::check_enable_reset(action, &reset, self.anchors)),
                    Err(code) => Err(code),
                });
                action.revalidate_files()?;
                // Read-only probe records refusal; it never authorizes SQL mutation.
                Ok(())
            })
        }
    }

    async fn run_actual_source_reset_probe(case: &[u8], expected: Result<(), ErrorCode>) {
        // Existing Root R5 fixture plus P4 fixed local selection/anchor originals.
        // Marker selects only a test expectation, never production authority.
        let (_, marker) =
            super::read_fixed("/run/r5-paper-checker-integration/native-reset-case", 128).unwrap();
        assert_eq!(marker, case, "wrong staged fixture");
        let (request_file, request) =
            super::read_fixed(super::REQUEST_PATH, super::wire::MAX_WIRE_BYTES).unwrap();
        let intent = parse_intent(&request, Verb::Enable).unwrap();
        let (process_file, process) =
            super::read_fixed(super::EVIDENCE_PATH, super::wire::MAX_WIRE_BYTES).unwrap();
        let process = super::wire::parse_process_binding(&process, &intent).unwrap();
        let (database_file, database) = super::read_fixed(super::DATABASE_PATH, 4096).unwrap();
        let selector = parse_database(&database).unwrap();
        let inputs = super::ProtectedInputs {
            request: request_file,
            evidence: Some(process_file),
            database: database_file,
            saved_request: None,
            bound: None,
        };
        let anchors = super::CheckedResetAnchor::open_all(
            &intent,
            process.value["daily_cohort_sha256"].as_str().unwrap(),
        )
        .unwrap();
        let worker = super::connect(&selector, DatabaseRole::Worker)
            .await
            .unwrap();
        let migration = super::connect(&selector, DatabaseRole::MigrationOwner)
            .await
            .unwrap();
        let mut guard = SourceProbeGuard {
            inputs: &inputs,
            anchors: &anchors,
        };
        let mut probe = SourceResetProbe {
            pool: &migration,
            intent: &intent,
            anchors: &anchors,
            result: None,
        };
        let completion = crate::owner_beta::paper::evidence::with_native_checked_paper_action(
            &worker, &mut guard, &mut probe,
        )
        .await;
        assert!(
            completion.first.is_none() && completion.secondary.is_empty(),
            "genuine checker/cleanup refused"
        );
        assert!(
            !completion.committed && !completion.commit_started,
            "probe must not mutate"
        );
        assert_eq!(
            probe.result,
            Some(expected),
            "not the required ARM/reset result"
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(5), worker.close())
                .await
                .is_ok()
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(5), migration.close())
                .await
                .is_ok()
        );
    }

    #[tokio::test(flavor = "current_thread")]
    #[ignore = "Existing hosted Root PG/Source fixture plus P4 local originals; genuine consuming join, no mutation"]
    async fn actual_native_source_join_requires_post_anchor_arm() {
        run_actual_source_reset_probe(b"post-anchor\n", Ok(())).await;
    }

    #[tokio::test(flavor = "current_thread")]
    #[ignore = "Existing hosted Root PG fixture stages genuine pre-anchor ARM and all five post-reset DB writes"]
    async fn actual_native_source_join_rejects_pre_reset_arm_despite_later_db_writes() {
        run_actual_source_reset_probe(
            b"pre-reset-arm-post-reset-db\n",
            Err(ErrorCode::ClockInvalid),
        )
        .await;
    }

    async fn staged_native_readback(case: &[u8]) -> (ResultWire, super::ExactReadback) {
        use std::{io::Read, os::fd::AsRawFd};
        let (_, marker) =
            super::read_fixed("/run/r5-paper-checker-integration/native-reset-case", 128).unwrap();
        assert_eq!(marker, case, "wrong staged fixture");
        let (inputs, intent, _, output, selector) =
            super::load_context(Verb::Readback).await.unwrap();
        let result = super::readback(&intent, &selector, &inputs, &output).await;
        let hash = result
            .artifact_sha256
            .as_ref()
            .expect("complete readback artifact required");
        let name = format!("{}.json", &hash[7..]);
        let mut file = super::openat_file(
            output.directory.as_raw_fd(),
            &name,
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
        .unwrap();
        super::check_file(&file).unwrap();
        let mut bytes = Vec::new();
        (&mut file)
            .take(super::wire::MAX_EXACT_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(&digest(&bytes), hash);
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let (_, request) = inputs.saved_request.as_ref().unwrap();
        let readback = super::ExactReadback::parse(value, request).unwrap();
        assert!(inputs.revalidate().is_ok());
        (result, readback)
    }

    #[tokio::test(flavor = "current_thread")]
    #[ignore = "Existing hosted disposable PG/OS fixture; stored ENABLE, later OFF, invalid Current, expired original, no worker secret"]
    async fn actual_native_readback_invalid_current_daily_off_expired_without_worker() {
        use std::io::Read;
        let missing = std::fs::symlink_metadata(super::WORKER_PASSWORD).unwrap_err();
        assert_eq!(missing.kind(), std::io::ErrorKind::NotFound);
        let read = |path: &str| {
            let mut bytes = Vec::new();
            std::fs::File::open(path)
                .unwrap()
                .take(65_537)
                .read_to_end(&mut bytes)
                .unwrap();
            assert!(bytes.len() <= 65_536);
            bytes
        };
        let current = crate::owner_beta::paper::source_evidence::UntrustedCurrentEnvelope::decode(
            &read("/data/owner-beta-paper-evidence/current/current-binding.json"),
        )
        .unwrap();
        assert_eq!(current.status().unwrap(), "INVALID");
        let daily: serde_json::Value =
            serde_json::from_slice(&read("/run/owner-beta-daily/config.json")).unwrap();
        assert_eq!(daily["enabled"], false);
        let (_, bytes) =
            super::read_fixed(super::SQL_REQUEST_PATH, super::wire::MAX_EXACT_BYTES).unwrap();
        let original = super::ExactRequest::parse(&bytes).unwrap();
        assert_eq!(original.value["args"]["transition"], "ENABLE");
        let evidence: serde_json::Value = serde_json::from_str(
            original.value["args"]["evidence_canonical"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        assert!(
            chrono::DateTime::parse_from_rfc3339(evidence["expires_at"].as_str().unwrap()).unwrap()
                < chrono::Utc::now()
        );
        let (result, readback) = staged_native_readback(b"readback-offline-committed\n").await;
        assert_eq!(result.outcome, Outcome::Replay);
        assert_eq!(result.exit_code(), 0);
        assert_eq!(readback.value["status"], "COMMITTED");
        assert_eq!(readback.value["current_state"], "OFF");
        assert!(
            readback.value["current_epoch"].as_i64().unwrap()
                > readback.value["event_epoch"].as_i64().unwrap()
        );
        let historical: serde_json::Value =
            serde_json::from_str(readback.value["result_canonical"].as_str().unwrap()).unwrap();
        assert_eq!(historical["data"]["state"], "ENABLED");
    }

    #[tokio::test(flavor = "current_thread")]
    #[ignore = "Existing hosted disposable PG/OS fixture; original request has ABSENT or legacy UNPROVEN receipt"]
    async fn actual_native_absent_or_unproven_readback_is_commit_unknown() {
        let (result, readback) = staged_native_readback(b"readback-unresolved\n").await;
        assert_eq!(result.outcome, Outcome::CommitUnknown);
        assert_eq!(result.exit_code(), 4);
        assert!(matches!(
            readback.value["status"].as_str(),
            Some("ABSENT" | "UNPROVEN")
        ));
        assert!(readback.value["event_id"].is_null());
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
        assert!(valid_sql_session_projection(&valid));
        let mut duplicate_date = valid.clone();
        duplicate_date[2]["session_date"] = serde_json::json!("2026-11-03");
        assert!(!valid_sql_session_projection(&duplicate_date));
        let mut widened = valid.clone();
        widened[0]["manual"] = serde_json::json!(true);
        assert!(!valid_sql_session_projection(&widened));
        let mut malformed_run = valid;
        malformed_run[1]["run_id"] = serde_json::json!("not-a-uuid");
        assert!(!valid_sql_session_projection(&malformed_run));
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

#[cfg(test)]
mod recovery_tests {
    use super::*;

    // Pure decoder fixtures do not construct CheckedPaperAction or authority.
    fn projection_fixture() -> (Value, Artifact, Intent) {
        let fixtures: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/owner-beta-paper-operations/action-bindings-v1.json"
        )))
        .unwrap();
        let dispatch = wire::BoundDispatch::parse(
            fixtures["objects"]["recover.action0.dispatch"]["canonical"]
                .as_str()
                .unwrap()
                .as_bytes(),
        )
        .unwrap();
        let mut decision = dispatch.artifact.unwrap();
        let mut intent = dispatch.root.intent;
        let d = decision.value.clone();
        let hash = |c: char| format!("sha256:{}", c.to_string().repeat(64));
        let owner = "11111111-1111-4111-8111-111111111111";
        let strategy = "22222222-2222-4222-8222-222222222222";
        let run = "33333333-3333-4333-8333-333333333333";
        let old_consent = "44444444-4444-4444-8444-444444444444";
        let mut current_process = json!({"schema":"owner-beta-paper-process-v1",
            "owner_user_id":owner,"account_id":d["account_id"],"strategy_config_id":strategy,
            "consent_event_id":d["consent_event_id"],"consent_version":"owner-beta-paper-v1-draft-1",
            "control_epoch":d["active_epoch"],"daily_cohort_sha256":hash('a'),
            "financial_contract_sha256":"sha256:18cde9fa7f15e19c9ec272abb780d06ae4be672068ed3f9208c39dc2297ab8d0",
            "code_commit":"a".repeat(40),"release_manifest_sha256":intent.release_manifest_sha256,
            "api_binary_sha256":hash('b'),"worker_binary_sha256":hash('c')});
        let mut old_process = current_process.clone();
        old_process["control_epoch"] = d["lineage_epoch"].clone();
        old_process["consent_event_id"] = json!(old_consent);
        old_process["daily_cohort_sha256"] = json!(hash('d'));
        old_process["release_manifest_sha256"] = json!(hash('e'));
        let old_process = wire::recovery_process(old_process).unwrap();
        let active = wire::recovery_process(current_process.take()).unwrap();
        decision.value["process_config_sha256"] = json!(active.sha256);
        decision.value["step"] = json!("SETTLE_THEN_VALUE");
        decision.value["decision_sha256"] = json!(digest(canonical_text(&json!({
            "effective_session":decision.value["effective_session"],
            "pending_target_id":decision.value["pending_target_id"], "step":decision.value["step"]
        })).unwrap().as_bytes()));
        decision.sha256 = digest(canonical_text(&decision.value).unwrap().as_bytes());
        intent.artifact_sha256 = Some(decision.sha256.clone());
        intent.value["artifact_sha256"] = json!(decision.sha256);
        let current_cohort = json!({"owner_user_id":owner,"strategy_config_id":strategy,
            "policy_sha256":hash('f'),"strategy_config_sha256":hash('1'),
            "entitlement_reference":"retained-test-rights","entitlement_sha256":hash('2'),
            "config_epoch":3,"activation_epoch":5,"cohort_sha256":hash('a')});
        let mut signal_cohort = current_cohort.clone();
        signal_cohort["cohort_sha256"] = json!(hash('d'));
        let effective =
            chrono::NaiveDate::parse_from_str(d["effective_session"].as_str().unwrap(), "%Y-%m-%d")
                .unwrap();
        let signal = (effective - chrono::Days::new(1)).to_string();
        let value = json!({"pending_target_id":d["pending_target_id"],"target_status":"PENDING",
            "lineage":{"id":d["lineage_id"],"owner_user_id":owner,"account_id":d["account_id"],
                "strategy_config_id":strategy,"recommendation_run_id":run,"signal_date":signal,
                "effective_session":d["effective_session"],"signal_admission_sha256":d["signal_admission_sha256"],
                "control_epoch":d["lineage_epoch"],"consent_event_id":old_consent,"cohort_sha256":hash('d'),
                "config_epoch":3,"daily_activation_epoch":5,"factor_snapshot_sha256":hash('3'),"target_snapshot_sha256":hash('4')},
            "session":{"state":"HELD","settlement_sha256":null,"valuation_sha256":null,
                "original_receipt_sha256":d["original_receipt_sha256"]},
            "current":{"state":"ENABLED","control_epoch":d["active_epoch"],
                "consent_event_id":d["consent_event_id"],"process_binding":active.value,
                "process_binding_sha256":active.sha256,"cohort":current_cohort},
            "original_process":old_process.value,"original_process_sha256":old_process.sha256,
            "signal_cohort":signal_cohort,"execution_cohort":signal_cohort});
        (value, decision, intent)
    }

    #[test]
    fn current_daily_epoch_changes_keep_original_recovery_identities_separate() {
        let (mut value, decision, intent) = projection_fixture();
        let old_t = value["signal_cohort"].clone();
        let old_e = value["execution_cohort"].clone();
        let old_lineage = value["lineage"].clone();
        value["current"]["cohort"]["activation_epoch"] = json!(91);
        value["current"]["cohort"]["config_epoch"] = json!(73);
        let original = RecoveryOriginal::parse(value.clone(), &decision, &intent).unwrap();
        assert_eq!(original.value["signal_cohort"], old_t);
        assert_eq!(original.value["execution_cohort"], old_e);
        assert_eq!(original.value["lineage"], old_lineage);
        assert_eq!(original.value["current"]["cohort"]["activation_epoch"], 91);
        value["signal_cohort"]["activation_epoch"] = json!(91);
        assert!(
            RecoveryOriginal::parse(value, &decision, &intent).is_err(),
            "current epochs must never overwrite the original T epoch"
        );
    }

    #[test]
    fn changed_current_economics_cannot_borrow_historical_selection() {
        let (value, decision, intent) = projection_fixture();
        for key in [
            "owner_user_id",
            "strategy_config_id",
            "strategy_config_sha256",
            "policy_sha256",
            "entitlement_reference",
            "entitlement_sha256",
        ] {
            let mut changed = value.clone();
            changed["current"]["cohort"][key] = json!("changed");
            assert!(
                RecoveryOriginal::parse(changed, &decision, &intent).is_err(),
                "{key}"
            );
        }
        let mut changed = value;
        changed["original_process"]["financial_contract_sha256"] =
            json!(format!("sha256:{}", "0".repeat(64)));
        let process = wire::recovery_process(changed["original_process"].clone());
        if let Ok(process) = process {
            changed["original_process_sha256"] = json!(process.sha256);
        }
        assert!(RecoveryOriginal::parse(changed, &decision, &intent).is_err());
    }

    #[test]
    fn original_recovery_selects_historical_t_run_and_e_without_relabelling_epochs() {
        use crate::owner_beta::paper::evidence::financial::PaperActionInputRequest;
        let (value, decision, intent) = projection_fixture();
        let before = value.clone();
        let original = RecoveryOriginal::parse(value, &decision, &intent).unwrap();
        let PaperActionInputRequest::Settle {
            signal,
            effective,
            run_id,
        } = original.selector
        else {
            panic!("original unsettled target must select SETTLE T/E")
        };
        assert_eq!(signal.to_string(), before["lineage"]["signal_date"]);
        assert_eq!(
            effective.to_string(),
            before["lineage"]["effective_session"]
        );
        assert_eq!(
            run_id.to_string(),
            before["lineage"]["recommendation_run_id"]
        );
        assert_eq!(original.value, before);
        assert_ne!(
            original.value["lineage"]["control_epoch"],
            original.process.value["control_epoch"]
        );
        assert_ne!(
            original.value["signal_cohort"]["cohort_sha256"],
            original.value["current"]["cohort"]["cohort_sha256"]
        );
    }

    #[test]
    fn settled_recovery_selects_value_only_and_cannot_create_a_second_settlement() {
        use crate::owner_beta::paper::evidence::financial::PaperActionInputRequest;
        let (mut value, mut decision, intent) = projection_fixture();
        value["session"]["settlement_sha256"] = decision.value["original_receipt_sha256"].clone();
        value["session"]["state"] = json!("SETTLED_TRADE");
        value["target_status"] = json!("EXECUTED");
        assert!(RecoveryOriginal::parse(value.clone(), &decision, &intent).is_err());
        decision.value["step"] = json!("VALUE_ONLY");
        let original = RecoveryOriginal::parse(value.clone(), &decision, &intent).unwrap();
        assert!(matches!(
            original.selector,
            PaperActionInputRequest::Value { .. }
        ));
        value["session"]["valuation_sha256"] = json!(format!("sha256:{}", "f".repeat(64)));
        assert!(RecoveryOriginal::parse(value, &decision, &intent).is_err());
    }

    #[test]
    fn wrong_target_owner_lineage_process_epoch_and_semantics_cannot_select_recovery() {
        let (value, decision, intent) = projection_fixture();
        for pointer in [
            "/pending_target_id",
            "/lineage/id",
            "/lineage/owner_user_id",
            "/lineage/account_id",
            "/lineage/strategy_config_id",
            "/lineage/control_epoch",
            "/lineage/consent_event_id",
            "/current/control_epoch",
            "/current/consent_event_id",
            "/current/process_binding_sha256",
            "/original_process_sha256",
            "/session/original_receipt_sha256",
            "/signal_cohort/policy_sha256",
            "/execution_cohort/entitlement_sha256",
            "/signal_cohort/config_epoch",
        ] {
            let mut changed = value.clone();
            *changed.pointer_mut(pointer).unwrap() = Value::Null;
            assert!(
                RecoveryOriginal::parse(changed, &decision, &intent).is_err(),
                "{pointer}"
            );
        }
        let mut wrong_intent = intent.clone();
        wrong_intent.value["resume_target_id"] = Value::Null;
        assert!(RecoveryOriginal::parse(value, &decision, &wrong_intent).is_err());
    }

    #[test]
    fn grant_identity_is_complete_and_changed_grant_or_decision_is_rejected() {
        let (_, decision, intent) = projection_fixture();
        let id = "55555555-5555-4555-8555-555555555555";
        let response = json!({"schema":"owner-beta-paper-action-v1","action":"TRANSITION","outcome":"OK",
            "control_epoch":intent.expected_epoch,"account_id":decision.value["account_id"],
            "data":{"event_id":"66666666-6666-4666-8666-666666666666","state":"ENABLED",
                "pending_target_id":decision.value["pending_target_id"],"grant_id":id}});
        let grant = json!({"grant_id":id,"active_epoch":decision.value["active_epoch"],
            "active_process_sha256":decision.value["process_config_sha256"],
            "active_consent_id":decision.value["consent_event_id"],"step":decision.value["step"],
            "expires_at":decision.value["expires_at"]});
        assert!(recovery_response_matches(&response, &intent, &decision));
        assert!(recovery_grant_matches(&grant, &response, &decision));
        for key in [
            "grant_id",
            "active_epoch",
            "active_process_sha256",
            "active_consent_id",
            "step",
            "expires_at",
        ] {
            let mut changed = grant.clone();
            changed[key] = Value::Null;
            assert!(
                !recovery_grant_matches(&changed, &response, &decision),
                "{key}"
            );
        }
        let mut other_grant = grant.clone();
        other_grant["grant_id"] = json!("77777777-7777-4777-8777-777777777777");
        assert!(!recovery_grant_matches(&other_grant, &response, &decision));
        let mut widened = grant;
        widened["authority"] = json!(true);
        assert!(!recovery_grant_matches(&widened, &response, &decision));
        let mut wrong = response;
        wrong["data"]["pending_target_id"] = json!(id);
        assert!(!recovery_response_matches(&wrong, &intent, &decision));
    }

    #[test]
    fn independent_commit_readback_must_match_the_exact_grant_and_current_epoch() {
        let fixtures: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/owner-beta-paper-operations/action-bindings-v1.json"
        )))
        .unwrap();
        let dispatch = wire::BoundDispatch::parse(
            fixtures["objects"]["recover.action0.dispatch"]["canonical"]
                .as_str()
                .unwrap()
                .as_bytes(),
        )
        .unwrap();
        let response: Value = serde_json::from_str(
            fixtures["objects"]["recover.result"]["canonical"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        let text = canonical_text(&response).unwrap();
        let value = json!({"schema":"owner-beta-paper-exact-readback-v1","kind":"TRANSITION",
            "request_sha256":dispatch.request.sha256,"status":"COMMITTED","account_id":response["account_id"],
            "current_epoch":response["control_epoch"],"current_state":"ENABLED","event_id":response["data"]["event_id"],
            "event_epoch":response["control_epoch"],"result_canonical":text,"result_sha256":digest(text.as_bytes()),"reset":null});
        let exact = ExactReadback::parse(value.clone(), &dispatch.request).unwrap();
        assert!(recovery_committed_matches(
            &exact,
            &response,
            &dispatch.root.intent
        ));
        let mut different = response.clone();
        different["data"]["grant_id"] = json!("77777777-7777-4777-8777-777777777777");
        let text = canonical_text(&different).unwrap();
        let mut changed = value.clone();
        changed["result_canonical"] = json!(text);
        changed["result_sha256"] = json!(digest(text.as_bytes()));
        let other = ExactReadback::parse(changed, &dispatch.request).unwrap();
        assert!(!recovery_committed_matches(
            &other,
            &response,
            &dispatch.root.intent
        ));
        let mut later = value;
        later["current_epoch"] = json!(dispatch.root.intent.expected_epoch + 1);
        later["current_state"] = json!("OFF");
        let later = ExactReadback::parse(later, &dispatch.request).unwrap();
        assert!(!recovery_committed_matches(
            &later,
            &response,
            &dispatch.root.intent
        ));
        // Exact historical replay remains decodable but supplies no current grant.
        assert_eq!(later.value["status"], "COMMITTED");
    }

    #[test]
    fn recover_cleanup_failure_preserves_original_event_and_requires_exact_readback() {
        let mut result = ResultWire::new(Verb::Recover, Outcome::Ok, ErrorCode::None);
        result.event_id = Some("11111111-1111-4111-8111-111111111111".to_owned());
        result.generation = Some("22222222-2222-4222-8222-222222222222".to_owned());
        result.control_epoch = Some(9);
        result.artifact_sha256 = Some(format!("sha256:{}", "a".repeat(64)));
        let before = serde_json::to_value(&result).unwrap();
        let after =
            serde_json::to_value(finalize_after_recheck(result, false, Verb::Recover)).unwrap();
        assert_eq!(after["outcome"], "COMMIT_UNKNOWN");
        for key in ["event_id", "generation", "control_epoch", "artifact_sha256"] {
            assert_eq!(after[key], before[key]);
        }
        assert!(!std::hint::black_box(ENABLE_SQL_RESET_AND_READBACK_READY));
        assert_eq!(
            super::super::native_follow_up_block(Verb::Recover),
            Some(ErrorCode::EvidenceUnavailable)
        );
        assert_eq!(
            super::super::native_follow_up_block(Verb::RestoreOff),
            Some(ErrorCode::EvidenceUnavailable)
        );
    }

    async fn staged_recovery(
        case: &[u8],
    ) -> (
        ProtectedInputs,
        Intent,
        Artifact,
        GuardedDirectory,
        DatabaseSelector,
    ) {
        assert_eq!(
            unsafe { libc::geteuid() },
            0,
            "disposable hosted Root fixture only"
        );
        assert!(
            env_is_clean(),
            "native environment must be scrubbed by fixture launcher"
        );
        let (_, marker) =
            read_fixed("/run/r5-paper-checker-integration/native-bound-case", 128).unwrap();
        assert_eq!(marker, case);
        let (inputs, intent, decision, output, selector) =
            load_context(Verb::Recover).await.unwrap();
        (inputs, intent, decision.unwrap(), output, selector)
    }

    async fn actual_recovery_roundtrip(case: &[u8], expected_step: &str) {
        let (inputs, intent, decision, output, selector) = staged_recovery(case).await;
        assert_eq!(decision.value["step"], expected_step);
        let pool = connect(&selector, DatabaseRole::MigrationOwner)
            .await
            .unwrap();
        let mut tx = pool.begin().await.unwrap();
        let before = recovery_original(&mut tx, &intent, &decision, false)
            .await
            .unwrap();
        assert!(rollback_bounded(tx).await);
        assert!(before.value["lineage"]["control_epoch"].as_i64().unwrap() < intent.expected_epoch);
        let first = recover(&intent, Some(&decision), &selector, &inputs, &output).await;
        assert_eq!(
            first.outcome,
            Outcome::Ok,
            "real protected checker/SQL/COMMIT/readback must pass"
        );
        let replay = recover(&intent, Some(&decision), &selector, &inputs, &output).await;
        assert_eq!(replay.outcome, Outcome::Replay);
        assert_eq!(first.event_id, replay.event_id);
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM public.owner_beta_paper_recovery_grants WHERE account_id=$1 AND request_key=$2")
            .bind(intent.account_id.unwrap()).bind(&intent.request_key).fetch_one(&pool).await.unwrap();
        assert_eq!(count, 1, "exact replay cannot mint another grant");
        let mut tx = pool.begin().await.unwrap();
        let after = recovery_original(&mut tx, &intent, &decision, false)
            .await
            .unwrap();
        assert!(rollback_bounded(tx).await);
        for key in [
            "lineage",
            "original_process",
            "original_process_sha256",
            "signal_cohort",
            "execution_cohort",
        ] {
            assert_eq!(after.value[key], before.value[key], "{key}");
        }
        for key in [
            "original_receipt_sha256",
            "settlement_sha256",
            "valuation_sha256",
        ] {
            assert_eq!(
                after.value["session"][key], before.value["session"][key],
                "{key}"
            );
        }
        assert!(
            tokio::time::timeout(Duration::from_secs(5), pool.close())
                .await
                .is_ok()
        );
        inputs.revalidate().unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    #[ignore = "Hosted isolated Root/PG actual historical T/E Source + Current + bound R-A-E-Q + image/provenance fixture; NOT_RUN locally"]
    async fn actual_native_recover_pre_settlement_original_t_e_and_exact_idempotency() {
        actual_recovery_roundtrip(b"recover-retained-settle\n", "SETTLE_THEN_VALUE").await;
    }

    #[tokio::test(flavor = "current_thread")]
    #[ignore = "Hosted isolated Root/PG actual settled historical T/E Source + Current + bound R-A-E-Q + image/provenance fixture; NOT_RUN locally"]
    async fn actual_native_recover_post_settlement_preserves_receipt_and_selects_value_only() {
        actual_recovery_roundtrip(b"recover-retained-value\n", "VALUE_ONLY").await;
    }

    #[tokio::test(flavor = "current_thread")]
    #[ignore = "Hosted isolated Root/PG existing committed RECOVER dispatch, expired decision/later OFF, no worker secret/live mounts; NOT_RUN locally"]
    async fn actual_native_recover_expired_replay_is_descriptive_without_current_or_worker() {
        let (inputs, intent, decision, output, selector) =
            staged_recovery(b"recover-expired-replay\n").await;
        assert!(
            chrono::DateTime::parse_from_rfc3339(decision.value["expires_at"].as_str().unwrap())
                .unwrap()
                < chrono::Utc::now()
        );
        assert_eq!(
            std::fs::symlink_metadata(WORKER_PASSWORD)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::NotFound
        );
        let result = recover(&intent, Some(&decision), &selector, &inputs, &output).await;
        assert_eq!(result.outcome, Outcome::Replay);
        assert!(result.control_epoch.unwrap() > intent.expected_epoch);
        inputs.revalidate().unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    #[ignore = "Hosted isolated Root/PG complete original dispatch whose transaction did not commit; no live mounts; NOT_RUN locally"]
    async fn actual_native_recover_absent_prior_dispatch_stays_unknown_and_never_retries() {
        let (inputs, intent, decision, output, selector) =
            staged_recovery(b"recover-absent-dispatch\n").await;
        let result = recover(&intent, Some(&decision), &selector, &inputs, &output).await;
        assert_eq!(result.outcome, Outcome::CommitUnknown);
        let pool = connect(&selector, DatabaseRole::MigrationOwner)
            .await
            .unwrap();
        let request = recovery_request(&intent, &decision).unwrap();
        assert_eq!(
            fresh_exact_read(&pool, &request).await.unwrap().value["status"],
            "ABSENT"
        );
        assert!(
            tokio::time::timeout(Duration::from_secs(5), pool.close())
                .await
                .is_ok()
        );
        inputs.revalidate().unwrap();
    }
}
