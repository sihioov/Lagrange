//! Private persistent minute gate and exact restart hints for daily processes.
//! Linux only. Existing installer provisioned state is mandatory; no reset,
//! provider access, approval constructor or database authority is supplied here.

use domain::{CodeCommit, ContentHash, TradingDate};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const MAX_BYTES: u64 = 16 * 1024;
const MINIMUM_INTERVAL_MILLIS: i64 = 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum Stage {
    Producer,
    Admitter,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub(crate) enum Pending {
    Candidate {
        content_sha256: ContentHash,
        manifest_sha256: ContentHash,
        parent_admission_sha256: Option<ContentHash>,
        session: TradingDate,
        producer_commit: CodeCommit,
    },
    Receipt {
        receipt_id: Uuid,
    },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    schema: String,
    stage: Stage,
    owner: Uuid,
    policy_sha256: ContentHash,
    generation: u64,
    last_started_unix_millis: Option<i64>,
    pending: Option<Pending>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum StateError {
    #[cfg(not(target_os = "linux"))]
    #[error("DAILY_STATE_UNSUPPORTED_PLATFORM")]
    UnsupportedPlatform,
    #[error("DAILY_STATE_INVALID")]
    Invalid,
    #[error("DAILY_STATE_UNSAFE_PATH")]
    UnsafePath,
    #[error("DAILY_STATE_CHANGED")]
    Changed,
    #[error("DAILY_STATE_IO")]
    Io,
    #[error("DAILY_STATE_CLOCK_ROLLBACK")]
    ClockRollback,
    #[error("DAILY_STATE_DURABILITY_UNKNOWN")]
    DurabilityUnknown,
}

impl Checkpoint {
    fn validate(&self, stage: Stage, owner: Uuid, policy: &ContentHash) -> Result<(), StateError> {
        let valid_pending = match (&self.pending, stage) {
            (None, _) => true,
            (Some(Pending::Receipt { receipt_id }), Stage::Admitter) => !receipt_id.is_nil(),
            (
                Some(Pending::Candidate {
                    content_sha256,
                    manifest_sha256,
                    parent_admission_sha256,
                    producer_commit,
                    ..
                }),
                Stage::Producer,
            ) => {
                canonical_hash(content_sha256)
                    && canonical_hash(manifest_sha256)
                    && parent_admission_sha256.as_ref().is_none_or(canonical_hash)
                    && CodeCommit::parse(producer_commit.as_str()).is_ok()
            }
            _ => false,
        };
        if self.schema != "owner-beta-daily-checkpoint-v1"
            || owner.is_nil()
            || self.owner != owner
            || self.stage != stage
            || &self.policy_sha256 != policy
            || !canonical_hash(policy)
            || !valid_pending
            || self.last_started_unix_millis.is_some_and(|value| value < 0)
            || (self.generation == 0
                && (self.last_started_unix_millis.is_some() || self.pending.is_some()))
            || (self.generation > 0 && self.last_started_unix_millis.is_none())
        {
            return Err(StateError::Invalid);
        }
        Ok(())
    }

    fn bytes(&self) -> Result<Vec<u8>, StateError> {
        let bytes = serde_json::to_vec(self).map_err(|_| StateError::Invalid)?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(StateError::Invalid);
        }
        Ok(bytes)
    }

    fn decode(
        bytes: &[u8],
        stage: Stage,
        owner: Uuid,
        policy: &ContentHash,
    ) -> Result<Self, StateError> {
        if bytes.len() as u64 > MAX_BYTES {
            return Err(StateError::Invalid);
        }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| StateError::Invalid)?;
        value.validate(stage, owner, policy)?;
        if value.bytes()? != bytes {
            return Err(StateError::Invalid);
        }
        Ok(value)
    }

    fn follows(&self, prior: &Self) -> bool {
        prior.generation.checked_add(1) == Some(self.generation)
            && self.owner == prior.owner
            && self.policy_sha256 == prior.policy_sha256
            && self.stage == prior.stage
            && self.last_started_unix_millis.is_some()
            && self.last_started_unix_millis >= prior.last_started_unix_millis
    }
}

fn canonical_hash(value: &ContentHash) -> bool {
    ContentHash::parse(value.as_str()).is_ok_and(|parsed| parsed == *value)
}

pub(crate) struct DailyState {
    #[cfg(target_os = "linux")]
    inner: linux::Guard,
}

impl DailyState {
    /// Acquires the full-cycle flock. Missing or invalid checkpoint is an error.
    /// The existing checkpoint belongs to exactly this stage, Owner and policy.
    pub(crate) fn acquire(
        path: &std::path::Path,
        expected_uid: u32,
        expected_gid: u32,
        stage: Stage,
        owner: Uuid,
        policy: &ContentHash,
    ) -> Result<Option<Self>, StateError> {
        #[cfg(target_os = "linux")]
        {
            linux::Guard::acquire(path, expected_uid, expected_gid, stage, owner, policy)
                .map(|guard| guard.map(|inner| Self { inner }))
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (path, expected_uid, expected_gid, stage, owner, policy);
            Err(StateError::UnsupportedPlatform)
        }
    }

    pub(crate) fn begin(&mut self, now: chrono::DateTime<chrono::Utc>) -> Result<bool, StateError> {
        #[cfg(target_os = "linux")]
        {
            self.inner.begin(now.timestamp_millis())
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = now;
            Err(StateError::UnsupportedPlatform)
        }
    }

    pub(crate) fn pending(&self) -> Option<&Pending> {
        #[cfg(target_os = "linux")]
        {
            self.inner.state.pending.as_ref()
        }
        #[cfg(not(target_os = "linux"))]
        {
            None
        }
    }

    pub(crate) fn set_pending(&mut self, pending: Pending) -> Result<(), StateError> {
        #[cfg(target_os = "linux")]
        {
            if self
                .inner
                .state
                .pending
                .as_ref()
                .is_some_and(|old| old != &pending)
            {
                return Err(StateError::Invalid);
            }
            self.inner.update_pending(Some(pending))
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = pending;
            Err(StateError::UnsupportedPlatform)
        }
    }

    /// Only after rereading the previous exact receipt under a new exclusive DB
    /// cycle. This is one atomic hint replacement, without a clear/write gap.
    pub(crate) fn replace_receipt(&mut self, receipt_id: Uuid) -> Result<(), StateError> {
        #[cfg(target_os = "linux")]
        {
            if self.inner.state.stage != Stage::Admitter || receipt_id.is_nil() {
                return Err(StateError::Invalid);
            }
            self.inner
                .update_pending(Some(Pending::Receipt { receipt_id }))
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = receipt_id;
            Err(StateError::UnsupportedPlatform)
        }
    }

    /// Call only after independently resolving the exact saved work. Clearing a
    /// hint never supplies admission, publication or source approval.
    pub(crate) fn clear_pending(&mut self) -> Result<(), StateError> {
        #[cfg(target_os = "linux")]
        {
            self.inner.update_pending(None)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(StateError::UnsupportedPlatform)
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::ffi::{CStr, CString};
    use std::fs::{File, Metadata};
    use std::io::{Read, Write};
    use std::os::fd::{AsRawFd, FromRawFd, RawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::{Component, Path};

    const DIRECTORY_FLAGS: i32 =
        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    const FILE_FLAGS: i32 = libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK;
    const CHECKPOINT: &CStr = c"checkpoint.json";
    const TEMPORARY: &CStr = c".checkpoint.tmp";
    const LOCK: &CStr = c"poll.lock";

    #[derive(Clone, PartialEq, Eq)]
    struct Identity {
        device: u64,
        inode: u64,
        mode: u32,
        uid: u32,
        gid: u32,
    }
    impl Identity {
        fn of(value: &Metadata) -> Self {
            Self {
                device: value.dev(),
                inode: value.ino(),
                mode: value.mode(),
                uid: value.uid(),
                gid: value.gid(),
            }
        }
    }
    #[derive(PartialEq, Eq)]
    struct Snapshot {
        identity: Identity,
        length: u64,
        links: u64,
        mtime: (i64, i64),
        ctime: (i64, i64),
    }
    impl Snapshot {
        fn of(value: &Metadata) -> Self {
            Self {
                identity: Identity::of(value),
                length: value.len(),
                links: value.nlink(),
                mtime: (value.mtime(), value.mtime_nsec()),
                ctime: (value.ctime(), value.ctime_nsec()),
            }
        }
    }
    fn own_file(fd: RawFd) -> Result<File, StateError> {
        if fd < 0 {
            return Err(StateError::Io);
        }
        // SAFETY: a successful open/openat transfers one new descriptor here.
        Ok(unsafe { File::from_raw_fd(fd) })
    }
    fn openat(parent: &File, name: &CStr, flags: i32) -> Result<File, StateError> {
        // SAFETY: parent is live and name is NUL-terminated; mode applies only to CREATE.
        own_file(unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags, 0o600) })
    }
    fn no_acl(file: &File, directory: bool) -> Result<(), StateError> {
        for name in [c"system.posix_acl_access", c"system.posix_acl_default"] {
            if !directory && name == c"system.posix_acl_default" {
                continue;
            }
            // SAFETY: size-zero xattr query does not dereference the null buffer.
            let length = unsafe {
                libc::fgetxattr(file.as_raw_fd(), name.as_ptr(), std::ptr::null_mut(), 0)
            };
            if length >= 0 {
                return Err(StateError::UnsafePath);
            }
            match std::io::Error::last_os_error().raw_os_error() {
                Some(libc::ENODATA) | Some(libc::ENOTSUP) => (),
                _ => return Err(StateError::Io),
            }
        }
        Ok(())
    }

    struct Directory {
        file: File,
        name: Option<CString>,
        identity: Identity,
    }
    struct Root {
        dirs: Vec<Directory>,
        uid: u32,
        gid: u32,
    }
    impl Root {
        fn open(path: &Path, uid: u32, gid: u32) -> Result<Self, StateError> {
            // SAFETY: get[e]uid/get[e]gid are argument-free process queries.
            if unsafe { libc::geteuid() } != uid || unsafe { libc::getegid() } != gid {
                return Err(StateError::UnsafePath);
            }
            let bytes = path.as_os_str().as_bytes();
            if !path.is_absolute()
                || bytes.len() > 4096
                || bytes.ends_with(b"/")
                || bytes.windows(2).any(|part| part == b"//")
                || bytes
                    .split(|byte| *byte == b'/')
                    .any(|part| part == b"." || part == b"..")
            {
                return Err(StateError::UnsafePath);
            }
            let parts: Vec<_> = path.components().collect();
            if parts.len() < 2
                || parts.len() > 65
                || parts
                    .iter()
                    .skip(1)
                    .any(|part| !matches!(part, Component::Normal(_)))
            {
                return Err(StateError::UnsafePath);
            }
            // SAFETY: constant slash is NUL-terminated and flags require a directory.
            let slash = own_file(unsafe { libc::open(c"/".as_ptr(), DIRECTORY_FLAGS) })?;
            let slash_metadata = slash.metadata().map_err(|_| StateError::Io)?;
            if slash_metadata.uid() != 0 || slash_metadata.mode() & 0o022 != 0 {
                return Err(StateError::UnsafePath);
            }
            no_acl(&slash, true)?;
            let mut dirs = vec![Directory {
                file: slash,
                name: None,
                identity: Identity::of(&slash_metadata),
            }];
            for part in parts.into_iter().skip(1) {
                let Component::Normal(name) = part else {
                    return Err(StateError::UnsafePath);
                };
                let name = CString::new(name.as_bytes()).map_err(|_| StateError::UnsafePath)?;
                let file = openat(
                    &dirs.last().ok_or(StateError::UnsafePath)?.file,
                    &name,
                    DIRECTORY_FLAGS,
                )?;
                let metadata = file.metadata().map_err(|_| StateError::Io)?;
                if !metadata.is_dir()
                    || (metadata.uid() != 0 && metadata.uid() != uid)
                    || metadata.mode() & 0o022 != 0
                {
                    return Err(StateError::UnsafePath);
                }
                no_acl(&file, true)?;
                dirs.push(Directory {
                    file,
                    name: Some(name),
                    identity: Identity::of(&metadata),
                });
            }
            let leaf = &dirs.last().ok_or(StateError::UnsafePath)?.identity;
            if leaf.uid != uid || leaf.gid != gid || leaf.mode & 0o7777 != 0o700 {
                return Err(StateError::UnsafePath);
            }
            let root = Self { dirs, uid, gid };
            root.revalidate()?;
            Ok(root)
        }
        fn fd(&self) -> &File {
            &self
                .dirs
                .last()
                .expect("validated nonempty directory chain")
                .file
        }
        fn revalidate(&self) -> Result<(), StateError> {
            for (index, directory) in self.dirs.iter().enumerate() {
                let metadata = directory.file.metadata().map_err(|_| StateError::Io)?;
                if Identity::of(&metadata) != directory.identity {
                    return Err(StateError::Changed);
                }
                no_acl(&directory.file, true)?;
                if index > 0 {
                    let named = openat(
                        &self.dirs[index - 1].file,
                        directory.name.as_deref().ok_or(StateError::Changed)?,
                        DIRECTORY_FLAGS,
                    )?;
                    if Identity::of(&named.metadata().map_err(|_| StateError::Io)?)
                        != directory.identity
                    {
                        return Err(StateError::Changed);
                    }
                }
            }
            Ok(())
        }
        fn validate_file(&self, file: &File) -> Result<Snapshot, StateError> {
            let metadata = file.metadata().map_err(|_| StateError::Io)?;
            if !metadata.is_file()
                || metadata.uid() != self.uid
                || metadata.gid() != self.gid
                || metadata.mode() & 0o7777 != 0o600
                || metadata.nlink() != 1
            {
                return Err(StateError::UnsafePath);
            }
            no_acl(file, false)?;
            Ok(Snapshot::of(&metadata))
        }
        fn read(&self, name: &CStr) -> Result<Vec<u8>, StateError> {
            self.revalidate()?;
            let mut file = openat(self.fd(), name, libc::O_RDONLY | FILE_FLAGS)?;
            let before = self.validate_file(&file)?;
            if before.length > MAX_BYTES {
                return Err(StateError::Invalid);
            }
            let mut bytes = Vec::new();
            Read::by_ref(&mut file)
                .take(MAX_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| StateError::Io)?;
            let named = openat(self.fd(), name, libc::O_RDONLY | FILE_FLAGS)?;
            if bytes.len() as u64 != before.length
                || before != self.validate_file(&file)?
                || before != self.validate_file(&named)?
            {
                return Err(StateError::Changed);
            }
            self.revalidate()?;
            Ok(bytes)
        }
        fn temporary_exists(&self) -> Result<bool, StateError> {
            // SAFETY: only lstat-like metadata is requested for the fixed entry.
            let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
            let result = unsafe {
                libc::fstatat(
                    self.fd().as_raw_fd(),
                    TEMPORARY.as_ptr(),
                    stat.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            };
            if result == 0 {
                Ok(true)
            } else if std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
                Ok(false)
            } else {
                Err(StateError::Io)
            }
        }
        fn promote(&self) -> Result<(), StateError> {
            self.revalidate()?;
            // SAFETY: both fixed names are relative to the same live guarded directory.
            if unsafe {
                libc::renameat(
                    self.fd().as_raw_fd(),
                    TEMPORARY.as_ptr(),
                    self.fd().as_raw_fd(),
                    CHECKPOINT.as_ptr(),
                )
            } != 0
            {
                return Err(StateError::DurabilityUnknown);
            }
            self.fd()
                .sync_all()
                .map_err(|_| StateError::DurabilityUnknown)?;
            self.revalidate()
        }
    }

    struct Construction {
        root: Root,
        lock_identity: Identity,
        lock: File,
    }

    /// The lock file is deliberately last, so every other owned descriptor drops
    /// before the full-cycle flock is released, including construction failures.
    pub(super) struct Guard {
        root: Root,
        pub(super) state: Checkpoint,
        begun: bool,
        lock_identity: Identity,
        lock: File,
    }
    impl Guard {
        pub(super) fn acquire(
            path: &Path,
            uid: u32,
            gid: u32,
            stage: Stage,
            owner: Uuid,
            policy: &ContentHash,
        ) -> Result<Option<Self>, StateError> {
            let root = Root::open(path, uid, gid)?;
            let lock = openat(root.fd(), LOCK, libc::O_RDWR | FILE_FLAGS)?;
            let stat = root.validate_file(&lock)?;
            if stat.length != 0 {
                return Err(StateError::Invalid);
            }
            // SAFETY: flock operates on our live descriptor; NONBLOCK prevents waiting.
            if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                return if std::io::Error::last_os_error().raw_os_error() == Some(libc::EWOULDBLOCK)
                {
                    Ok(None)
                } else {
                    Err(StateError::Io)
                };
            }
            let construction = Construction {
                root,
                lock_identity: stat.identity,
                lock,
            };
            let state =
                Checkpoint::decode(&construction.root.read(CHECKPOINT)?, stage, owner, policy)?;
            let Construction {
                root,
                lock_identity,
                lock,
            } = construction;
            let mut guard = Self {
                root,
                state,
                begun: false,
                lock_identity,
                lock,
            };
            guard.revalidate()?;
            if guard.root.temporary_exists()? {
                let bytes = guard.root.read(TEMPORARY)?;
                let next = Checkpoint::decode(&bytes, stage, owner, policy)?;
                if !next.follows(&guard.state) {
                    return Err(StateError::Invalid);
                }
                let temporary = openat(guard.root.fd(), TEMPORARY, libc::O_RDONLY | FILE_FLAGS)?;
                guard.root.validate_file(&temporary)?;
                temporary
                    .sync_all()
                    .map_err(|_| StateError::DurabilityUnknown)?;
                guard.revalidate()?;
                guard.root.promote()?;
                if guard.root.read(CHECKPOINT)? != bytes {
                    return Err(StateError::Changed);
                }
                guard.state = next;
            }
            guard.revalidate()?;
            Ok(Some(guard))
        }
        fn revalidate(&self) -> Result<(), StateError> {
            self.root.revalidate()?;
            let named = openat(self.root.fd(), LOCK, libc::O_RDONLY | FILE_FLAGS)?;
            if self.root.validate_file(&self.lock)?.identity != self.lock_identity
                || self.root.validate_file(&named)?.identity != self.lock_identity
                || named.metadata().map_err(|_| StateError::Io)?.len() != 0
            {
                return Err(StateError::Changed);
            }
            Ok(())
        }
        fn persist(&mut self, next: Checkpoint) -> Result<(), StateError> {
            self.revalidate()?;
            if !next.follows(&self.state) {
                return Err(StateError::Invalid);
            }
            next.validate(
                self.state.stage,
                self.state.owner,
                &self.state.policy_sha256,
            )?;
            if self.root.read(CHECKPOINT)? != self.state.bytes()? || self.root.temporary_exists()? {
                return Err(StateError::Changed);
            }
            let bytes = next.bytes()?;
            let mut file = openat(
                self.root.fd(),
                TEMPORARY,
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | FILE_FLAGS,
            )?;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|_| StateError::Io)?;
            self.root.validate_file(&file)?;
            file.write_all(&bytes).map_err(|_| StateError::Io)?;
            file.sync_all().map_err(|_| StateError::DurabilityUnknown)?;
            if self.root.read(TEMPORARY)? != bytes {
                return Err(StateError::Changed);
            }
            self.revalidate()?;
            self.root.promote()?;
            if self.root.read(CHECKPOINT)? != bytes {
                return Err(StateError::Changed);
            }
            self.state = next;
            Ok(())
        }
        pub(super) fn begin(&mut self, now: i64) -> Result<bool, StateError> {
            if self.begun || now < 0 {
                return Err(StateError::Invalid);
            }
            if let Some(last) = self.state.last_started_unix_millis {
                let elapsed = now.checked_sub(last).ok_or(StateError::ClockRollback)?;
                if elapsed < 0 {
                    return Err(StateError::ClockRollback);
                }
                if elapsed < MINIMUM_INTERVAL_MILLIS {
                    return Ok(false);
                }
            }
            let mut next = self.state.clone();
            next.generation = next.generation.checked_add(1).ok_or(StateError::Invalid)?;
            next.last_started_unix_millis = Some(now);
            self.persist(next)?;
            self.begun = true;
            Ok(true)
        }
        pub(super) fn update_pending(
            &mut self,
            pending: Option<Pending>,
        ) -> Result<(), StateError> {
            if !self.begun {
                return Err(StateError::Invalid);
            }
            if self.state.pending == pending {
                return self.revalidate();
            }
            let mut next = self.state.clone();
            next.generation = next.generation.checked_add(1).ok_or(StateError::Invalid)?;
            next.pending = pending;
            self.persist(next)
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink};
    use tempfile::{Builder, TempDir};

    struct Fixture {
        directory: TempDir,
        owner: Uuid,
        policy: ContentHash,
        uid: u32,
        gid: u32,
    }
    impl Fixture {
        fn new(stage: Stage) -> Self {
            let parent =
                std::env::var_os("LAGRANGE_DAILY_ARTIFACT_TEST_ROOT").unwrap_or_else(|| {
                    std::env::var_os("HOME").expect("protected fixture parent required")
                });
            let directory = Builder::new()
                .prefix(".daily-state-")
                .tempdir_in(std::path::PathBuf::from(parent))
                .unwrap();
            fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
            // SAFETY: argument-free process identity queries.
            let (uid, gid) = unsafe { (libc::geteuid(), libc::getegid()) };
            let value = Self {
                directory,
                owner: Uuid::new_v4(),
                policy: ContentHash::from_bytes(b"policy"),
                uid,
                gid,
            };
            let state = Checkpoint {
                schema: "owner-beta-daily-checkpoint-v1".into(),
                stage,
                owner: value.owner,
                policy_sha256: value.policy.clone(),
                generation: 0,
                last_started_unix_millis: None,
                pending: None,
            };
            value.write("checkpoint.json", &state.bytes().unwrap());
            value.write("poll.lock", b"");
            value
        }
        fn write(&self, name: &str, bytes: &[u8]) {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(self.directory.path().join(name))
                .unwrap();
            file.write_all(bytes).unwrap();
        }
        fn acquire(&self, stage: Stage) -> Result<Option<DailyState>, StateError> {
            DailyState::acquire(
                self.directory.path(),
                self.uid,
                self.gid,
                stage,
                self.owner,
                &self.policy,
            )
        }
        fn bytes(&self) -> Vec<u8> {
            fs::read(self.directory.path().join("checkpoint.json")).unwrap()
        }
        fn decoded(&self, stage: Stage) -> Checkpoint {
            Checkpoint::decode(&self.bytes(), stage, self.owner, &self.policy).unwrap()
        }
    }
    fn now(millis: i64) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp_millis(millis).unwrap()
    }
    fn candidate() -> Pending {
        Pending::Candidate {
            content_sha256: ContentHash::from_bytes(b"bars"),
            manifest_sha256: ContentHash::from_bytes(b"manifest"),
            parent_admission_sha256: None,
            session: TradingDate::parse("2026-08-31").unwrap(),
            producer_commit: CodeCommit::parse(&"a".repeat(40)).unwrap(),
        }
    }

    #[test]
    fn daily_state_restart_retains_minute_gate_pending_and_full_cycle_exclusion() {
        let f = Fixture::new(Stage::Producer);
        let mut first = f.acquire(Stage::Producer).unwrap().unwrap();
        assert!(first.begin(now(1_000_000)).unwrap());
        first.set_pending(candidate()).unwrap();
        assert!(f.acquire(Stage::Producer).unwrap().is_none());
        assert_eq!(first.begin(now(1_060_000)), Err(StateError::Invalid));
        drop(first);
        let mut restarted = f.acquire(Stage::Producer).unwrap().unwrap();
        assert!(restarted.pending() == Some(&candidate()));
        let before = f.bytes();
        assert!(!restarted.begin(now(1_059_999)).unwrap());
        assert_eq!(before, f.bytes());
        assert!(restarted.begin(now(1_060_000)).unwrap());
        assert!(f.acquire(Stage::Producer).unwrap().is_none());
        restarted.clear_pending().unwrap();
        drop(restarted);
        assert!(
            f.acquire(Stage::Producer)
                .unwrap()
                .unwrap()
                .pending()
                .is_none()
        );
    }
    #[test]
    fn daily_state_clock_rollback_cannot_reset_the_durable_slot() {
        let f = Fixture::new(Stage::Producer);
        let mut state = f.acquire(Stage::Producer).unwrap().unwrap();
        state.begin(now(1_000_000)).unwrap();
        drop(state);
        let before = f.bytes();
        let mut state = f.acquire(Stage::Producer).unwrap().unwrap();
        assert_eq!(state.begin(now(999_999)), Err(StateError::ClockRollback));
        assert_eq!(before, f.bytes());
        assert!(state.begin(now(1_060_000)).unwrap());
    }
    #[test]
    fn daily_state_only_exact_next_complete_temporary_generation_is_recovered() {
        let f = Fixture::new(Stage::Producer);
        let mut prior = f.decoded(Stage::Producer);
        prior.generation = 1;
        prior.last_started_unix_millis = Some(1_000_000);
        prior.pending = Some(candidate());
        f.write(".checkpoint.tmp", &prior.bytes().unwrap());
        let mut state = f.acquire(Stage::Producer).unwrap().unwrap();
        assert!(state.pending() == Some(&candidate()));
        assert_eq!(f.bytes(), prior.bytes().unwrap());
        assert!(!f.directory.path().join(".checkpoint.tmp").exists());
        assert!(!state.begin(now(1_059_999)).unwrap());
        assert!(state.begin(now(1_060_000)).unwrap());
    }
    #[test]
    fn daily_state_partial_or_disconnected_temporary_state_is_preserved_and_rejected() {
        for complete in [false, true] {
            let f = Fixture::new(Stage::Producer);
            let before = f.bytes();
            let mut next = f.decoded(Stage::Producer);
            next.generation = 2;
            next.last_started_unix_millis = Some(1_000_000);
            let bytes = if complete {
                next.bytes().unwrap()
            } else {
                b"{".to_vec()
            };
            f.write(".checkpoint.tmp", &bytes);
            assert!(f.acquire(Stage::Producer).is_err());
            assert_eq!(f.bytes(), before);
            assert_eq!(
                fs::read(f.directory.path().join(".checkpoint.tmp")).unwrap(),
                bytes
            );
            fs::remove_file(f.directory.path().join(".checkpoint.tmp")).unwrap();
            assert!(f.acquire(Stage::Producer).unwrap().is_some());
        }
    }
    #[test]
    fn daily_state_missing_corrupt_or_foreign_checkpoint_never_becomes_first_use() {
        for invalid in [0, 1, 2, 3] {
            let f = Fixture::new(Stage::Producer);
            let valid = f.bytes();
            let path = f.directory.path().join("checkpoint.json");
            fs::remove_file(&path).unwrap();
            match invalid {
                0 => (),
                1 => f.write("checkpoint.json", b"{}"),
                2 => {
                    let mut foreign =
                        Checkpoint::decode(&valid, Stage::Producer, f.owner, &f.policy).unwrap();
                    foreign.owner = Uuid::new_v4();
                    f.write("checkpoint.json", &foreign.bytes().unwrap());
                }
                _ => {
                    let mut bytes = valid.clone();
                    bytes.push(b'\n');
                    f.write("checkpoint.json", &bytes);
                }
            }
            assert!(f.acquire(Stage::Producer).is_err());
            if path.exists() {
                fs::remove_file(&path).unwrap();
            }
            f.write("checkpoint.json", &valid);
            assert!(f.acquire(Stage::Producer).unwrap().is_some());
        }
    }
    #[test]
    fn daily_state_rejects_symlinks_links_modes_and_replaced_live_lock() {
        for name in ["checkpoint.json", "poll.lock"] {
            let f = Fixture::new(Stage::Producer);
            let path = f.directory.path().join(name);
            let outside = f.directory.path().join("outside");
            fs::rename(&path, &outside).unwrap();
            symlink(&outside, &path).unwrap();
            assert!(f.acquire(Stage::Producer).is_err());
            fs::remove_file(&path).unwrap();
            fs::hard_link(&outside, &path).unwrap();
            assert!(f.acquire(Stage::Producer).is_err());
            fs::remove_file(&path).unwrap();
            fs::rename(&outside, &path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
            assert!(f.acquire(Stage::Producer).is_err());
        }
        let f = Fixture::new(Stage::Producer);
        let mut state = f.acquire(Stage::Producer).unwrap().unwrap();
        fs::rename(
            f.directory.path().join("poll.lock"),
            f.directory.path().join("old.lock"),
        )
        .unwrap();
        f.write("poll.lock", b"");
        assert_eq!(state.begin(now(1_000_000)), Err(StateError::Changed));
        assert_eq!(f.decoded(Stage::Producer).generation, 0);
    }
    #[test]
    fn daily_state_symlink_temporary_does_not_overwrite_its_target() {
        let f = Fixture::new(Stage::Producer);
        let mut state = f.acquire(Stage::Producer).unwrap().unwrap();
        f.write("victim", b"do not replace");
        symlink("victim", f.directory.path().join(".checkpoint.tmp")).unwrap();
        assert!(state.begin(now(1_000_000)).is_err());
        assert_eq!(
            fs::read(f.directory.path().join("victim")).unwrap(),
            b"do not replace"
        );
        assert_eq!(f.decoded(Stage::Producer).generation, 0);
    }
    #[test]
    fn daily_state_receipt_replacement_is_atomic_and_wrong_stage_pending_is_rejected() {
        let f = Fixture::new(Stage::Admitter);
        let mut state = f.acquire(Stage::Admitter).unwrap().unwrap();
        assert!(state.replace_receipt(Uuid::new_v4()).is_err());
        state.begin(now(1_000_000)).unwrap();
        assert!(state.set_pending(candidate()).is_err());
        let old = Uuid::new_v4();
        state.replace_receipt(old).unwrap();
        let next = Uuid::new_v4();
        state.replace_receipt(next).unwrap();
        assert!(state.pending() == Some(&Pending::Receipt { receipt_id: next }));
        drop(state);
        let mut restarted = f.acquire(Stage::Admitter).unwrap().unwrap();
        assert!(restarted.pending() == Some(&Pending::Receipt { receipt_id: next }));
        assert!(restarted.clear_pending().is_err());
        restarted.begin(now(1_060_000)).unwrap();
        restarted.clear_pending().unwrap();
        assert!(f.decoded(Stage::Admitter).pending.is_none());
    }
    #[test]
    fn daily_state_candidate_identity_cannot_be_silently_replaced() {
        let f = Fixture::new(Stage::Producer);
        let mut state = f.acquire(Stage::Producer).unwrap().unwrap();
        state.begin(now(1_000_000)).unwrap();
        state.set_pending(candidate()).unwrap();
        let before = f.bytes();
        let mut changed = candidate();
        if let Pending::Candidate {
            producer_commit, ..
        } = &mut changed
        {
            *producer_commit = CodeCommit::parse(&"b".repeat(40)).unwrap();
        }
        assert_eq!(state.set_pending(changed), Err(StateError::Invalid));
        assert_eq!(before, f.bytes());
        assert!(state.set_pending(candidate()).is_ok());
        assert_eq!(before, f.bytes());
    }
}
