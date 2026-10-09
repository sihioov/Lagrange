//! Read-only Owner service status contract, strict host observation reader,
//! and fixed-clock projection. This module has no provider or process-control
//! surface; it consumes only the mounted safe snapshot and bounded DB reads.

use chrono::{
    DateTime, Duration as ChronoDuration, FixedOffset, NaiveDate, NaiveTime, SecondsFormat,
    TimeZone, Utc,
};
use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::ffi::CString;
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::mem::MaybeUninit;
use std::os::fd::FromRawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const CATALOG_JSON: &str = include_str!("../../../configs/operations/service-status-catalog.json");
pub const HOST_ROOT: &str = "/run/lagrange/service-status";
pub const HOST_FILE: &str = "snapshot.json";
pub const HOST_MAX_BYTES: usize = 256 * 1024;
const ACTOR_ADMISSION_INTERVAL: Duration = Duration::from_secs(5);
const ACTOR_SLOT_TTL: Duration = Duration::from_secs(5 * 60);
const ACTOR_SLOT_LIMIT: usize = 128;
const CACHE_TTL: Duration = Duration::from_secs(5);
const HOST_TTL_SECONDS: i64 = 90;
const DB_TTL_SECONDS: i64 = 30;
const RECENT_FAILURE_SECONDS: i64 = 24 * 60 * 60;
const KST_SECONDS: i32 = 9 * 60 * 60;

/// Stable reader failures. Provider and operating-system prose never crosses
/// this boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostReadError {
    Missing,
    Invalid,
    PolicyMismatch,
    Future,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionError {
    RateLimited,
    SlotCapacity,
}

#[derive(Clone)]
struct CachedEvidence {
    inserted_at: Instant,
    input: Value,
    session_hash: String,
    policy_revision: String,
}

#[derive(Clone)]
struct CachedSharedEvidence {
    inserted_at: Instant,
    input: Value,
    policy_revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HostFileIdentity {
    device: u64,
    inode: u64,
    size: i64,
    modified: (i64, i64),
    changed: (i64, i64),
    owner: (u32, u32),
    mode: u32,
    links: u64,
}

impl HostFileIdentity {
    fn from_stat(stat: &libc::stat) -> Self {
        Self {
            device: stat.st_dev,
            inode: stat.st_ino,
            size: stat.st_size,
            modified: (stat.st_mtime, stat.st_mtime_nsec),
            changed: (stat.st_ctime, stat.st_ctime_nsec),
            owner: (stat.st_uid, stat.st_gid),
            mode: stat.st_mode,
            links: stat.st_nlink,
        }
    }
}

#[derive(Clone)]
struct CachedHostSnapshot {
    inserted_at: Instant,
    identity: HostFileIdentity,
    release: String,
    evidence: HostEvidence,
}

/// Safe evidence from the fixed host reader. The revision binds both the
/// reopened file identity and the accepted embedded policy, never private
/// installer state. Production constructors do not accept a path or owner.
#[derive(Clone)]
pub struct HostEvidence {
    pub(crate) read: Value,
    pub(crate) error: Option<HostReadError>,
    revision: String,
}

impl HostEvidence {
    pub(crate) fn revision(&self) -> &str {
        &self.revision
    }

    fn attach(&self, input: &mut Value) {
        input["host_read"] = self.read.clone();
        input["host_error"] = self
            .error
            .map(|error| json!(error.reason()))
            .unwrap_or(Value::Null);
    }
}

struct ActorSlot {
    last_touch: Instant,
    last_admitted: Option<Instant>,
    session_hash: String,
    cache: Option<CachedEvidence>,
}

#[derive(Default)]
struct AdmissionState {
    actors: HashMap<String, ActorSlot>,
    global: VecDeque<Instant>,
}

/// Per-process admission and source-evidence cache. Cache entries are keyed by
/// authenticated actor, canonical session hash, and immutable release policy.
/// The caller always projects them again against the current serve clock.
pub struct ServiceStatusRuntime {
    state: Mutex<AdmissionState>,
    refreshes: Arc<Semaphore>,
    shared_evidence: Mutex<Option<CachedSharedEvidence>>,
    shared_refresh: Arc<Semaphore>,
    host_snapshot: Mutex<Option<CachedHostSnapshot>>,
}

impl Default for ServiceStatusRuntime {
    fn default() -> Self {
        Self {
            state: Mutex::new(AdmissionState::default()),
            refreshes: Arc::new(Semaphore::new(2)),
            shared_evidence: Mutex::new(None),
            shared_refresh: Arc::new(Semaphore::new(1)),
            host_snapshot: Mutex::new(None),
        }
    }
}

impl ServiceStatusRuntime {
    pub fn admit(
        &self,
        actor_id: &str,
        session_hash: &str,
        now: Instant,
    ) -> Result<(), AdmissionError> {
        let Ok(mut state) = self.state.lock() else {
            return Err(AdmissionError::RateLimited);
        };
        state
            .actors
            .retain(|_, slot| now.saturating_duration_since(slot.last_touch) < ACTOR_SLOT_TTL);
        state
            .global
            .retain(|at| now.saturating_duration_since(*at) < Duration::from_secs(1));
        if !state.actors.contains_key(actor_id) {
            if state.actors.len() >= ACTOR_SLOT_LIMIT {
                return Err(AdmissionError::SlotCapacity);
            }
            state.actors.insert(
                actor_id.to_owned(),
                ActorSlot {
                    last_touch: now,
                    last_admitted: None,
                    session_hash: session_hash.to_owned(),
                    cache: None,
                },
            );
        }
        let global_at_cap = state.global.len() >= 10;
        let slot = state.actors.get_mut(actor_id).expect("actor slot inserted");
        if slot.session_hash != session_hash {
            slot.session_hash = session_hash.to_owned();
            slot.cache = None;
        }
        slot.last_touch = now;
        if slot
            .last_admitted
            .is_some_and(|at| now.saturating_duration_since(at) < ACTOR_ADMISSION_INTERVAL)
            || global_at_cap
        {
            return Err(AdmissionError::RateLimited);
        }
        state
            .actors
            .get_mut(actor_id)
            .expect("actor slot present")
            .last_admitted = Some(now);
        state.global.push_back(now);
        Ok(())
    }

    pub fn cached_input(
        &self,
        actor_id: &str,
        session_hash: &str,
        policy_revision: &str,
        now: Instant,
    ) -> Option<Value> {
        let state = self.state.lock().ok()?;
        let slot = state.actors.get(actor_id)?;
        let cache = slot.cache.as_ref()?;
        (cache.session_hash == session_hash
            && cache.policy_revision == policy_revision
            && now.saturating_duration_since(cache.inserted_at) < CACHE_TTL)
            .then(|| cache.input.clone())
    }

    pub fn cache_input(
        &self,
        actor_id: &str,
        session_hash: &str,
        policy_revision: &str,
        input: Value,
        now: Instant,
    ) {
        if let Ok(mut state) = self.state.lock()
            && let Some(slot) = state.actors.get_mut(actor_id)
            && slot.session_hash == session_hash
        {
            slot.cache = Some(CachedEvidence {
                inserted_at: now,
                input,
                session_hash: session_hash.to_owned(),
                policy_revision: policy_revision.to_owned(),
            });
        }
    }

    pub fn try_refresh(&self) -> Option<OwnedSemaphorePermit> {
        self.refreshes.clone().try_acquire_owned().ok()
    }

    pub fn cached_shared_evidence(&self, policy_revision: &str, now: Instant) -> Option<Value> {
        let cached = self.shared_evidence.lock().ok()?.clone()?;
        (cached.policy_revision == policy_revision
            && now.saturating_duration_since(cached.inserted_at) < CACHE_TTL)
            .then_some(cached.input)
    }

    pub fn cache_shared_evidence(&self, policy_revision: &str, input: Value, now: Instant) {
        if let Ok(mut cache) = self.shared_evidence.lock() {
            *cache = Some(CachedSharedEvidence {
                inserted_at: now,
                input,
                policy_revision: policy_revision.to_owned(),
            });
        }
    }

    pub fn try_shared_refresh(&self) -> Option<OwnedSemaphorePermit> {
        self.shared_refresh.clone().try_acquire_owned().ok()
    }

    pub(crate) fn observe_host(&self, now: DateTime<Utc>, release: &str) -> HostEvidence {
        self.observe_host_at(Path::new(HOST_ROOT), (0, 10001), now, release, false)
    }

    /// Uses the same validation and cache path as serving; only this isolated
    /// test helper can supply synthetic filesystem ownership.
    #[doc(hidden)]
    pub fn observe_host_for_test(
        &self,
        root: &Path,
        owner: (u32, u32),
        now: DateTime<Utc>,
        release: &str,
    ) -> HostEvidence {
        self.observe_host_at(root, owner, now, release, true)
    }

    fn observe_host_at(
        &self,
        root: &Path,
        owner: (u32, u32),
        now: DateTime<Utc>,
        release: &str,
        test_mode: bool,
    ) -> HostEvidence {
        let result = (|| {
            let opened = open_snapshot(
                root,
                Path::new(HOST_FILE),
                owner.0,
                owner.1,
                0o750,
                0o640,
                release,
                test_mode,
            )?;
            let identity = HostFileIdentity::from_stat(&opened.before);
            let mut cache = self
                .host_snapshot
                .lock()
                .map_err(|_| HostReadError::Invalid)?;
            if let Some(cached) = cache.as_ref()
                && cached.identity == identity
                && cached.release == release
                && cached.inserted_at.elapsed() < CACHE_TTL
            {
                opened.validate_unchanged()?;
                if cached
                    .evidence
                    .read
                    .pointer("/snapshot/generated_at")
                    .and_then(optional_instant)
                    .is_some_and(|at| at > now)
                    || cached
                        .evidence
                        .read
                        .pointer("/snapshot/policy/policy_installed_at")
                        .and_then(optional_instant)
                        .is_some_and(|at| at > now)
                {
                    return Err(HostReadError::Future);
                }
                return Ok(cached.evidence.clone());
            }
            let snapshot = opened.read(now, release)?;
            let policy =
                serde_json::to_vec(&snapshot["policy"]).map_err(|_| HostReadError::Invalid)?;
            let revision = format!("{release}:{identity:?}:{:x}", Sha256::digest(&policy));
            let evidence = HostEvidence {
                read: json!({"result":"ok","snapshot":snapshot}),
                error: None,
                revision,
            };
            *cache = Some(CachedHostSnapshot {
                inserted_at: Instant::now(),
                identity,
                release: release.to_owned(),
                evidence: evidence.clone(),
            });
            Ok(evidence)
        })();
        result.unwrap_or_else(|error: HostReadError| {
            if let Ok(mut cache) = self.host_snapshot.lock() {
                *cache = None;
            }
            HostEvidence {
                read: json!({"result":error.result(),"snapshot":null}),
                error: Some(error),
                revision: format!("{release}:{}", error.reason()),
            }
        })
    }

    /// The serving cache lookup binds the actual current trusted file/policy
    /// and refuses a queue summary after its first count-changing boundary.
    #[doc(hidden)]
    pub fn cached_owner_evidence(
        &self,
        actor: &str,
        session: &str,
        host: &HostEvidence,
        at: Instant,
        now: DateTime<Utc>,
    ) -> Option<Value> {
        let mut input = self.cached_input(actor, session, host.revision(), at)?;
        if queue_summary_requires_refresh(&input, now) {
            return None;
        }
        host.attach(&mut input);
        Some(input)
    }

    #[doc(hidden)]
    pub fn cache_owner_evidence(
        &self,
        actor: &str,
        session: &str,
        host: &HostEvidence,
        mut input: Value,
        at: Instant,
    ) {
        host.attach(&mut input);
        self.cache_input(actor, session, host.revision(), input, at);
    }

    #[doc(hidden)]
    pub fn cached_shared_for_host(&self, host: &HostEvidence, at: Instant) -> Option<Value> {
        self.cached_shared_evidence(host.revision(), at)
    }

    #[doc(hidden)]
    pub fn cache_shared_for_host(&self, host: &HostEvidence, input: Value, at: Instant) {
        self.cache_shared_evidence(host.revision(), input, at);
    }
}

impl HostReadError {
    pub const fn result(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Invalid | Self::PolicyMismatch | Self::Future => "invalid",
        }
    }

    pub const fn reason(self) -> &'static str {
        match self {
            Self::Missing => "HOST_MISSING",
            Self::Invalid => "HOST_INVALID",
            Self::PolicyMismatch => "POLICY_MISMATCH",
            Self::Future => "HOST_FUTURE",
        }
    }
}

/// Open the fixed snapshot through directory file descriptors, rejecting
/// symlinks, writable/untrusted parents, hardlinks and replacement races.
pub fn read_host_snapshot(
    now: DateTime<Utc>,
    expected_release: &str,
) -> Result<Value, HostReadError> {
    read_host_snapshot_at(
        Path::new(HOST_ROOT),
        Path::new(HOST_FILE),
        0,
        10001,
        0o750,
        0o640,
        now,
        expected_release,
        false,
    )
}

/// Isolated filesystem seam used by the API tests. Production always uses
/// [`read_host_snapshot`] with the immutable release and mounted path.
#[doc(hidden)]
pub fn read_host_snapshot_for_test(
    root: &Path,
    now: DateTime<Utc>,
    expected_release: &str,
    owner: (u32, u32),
) -> Result<Value, HostReadError> {
    read_host_snapshot_at(
        root,
        Path::new(HOST_FILE),
        owner.0,
        owner.1,
        0o750,
        0o640,
        now,
        expected_release,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
fn read_host_snapshot_at(
    root: &Path,
    file_name: &Path,
    expected_uid: u32,
    expected_gid: u32,
    directory_mode: u32,
    file_mode: u32,
    now: DateTime<Utc>,
    expected_release: &str,
    test_mode: bool,
) -> Result<Value, HostReadError> {
    open_snapshot(
        root,
        file_name,
        expected_uid,
        expected_gid,
        directory_mode,
        file_mode,
        expected_release,
        test_mode,
    )?
    .read(now, expected_release)
}

struct OpenSnapshot {
    directory: File,
    file: File,
    name: CString,
    before: libc::stat,
}

impl OpenSnapshot {
    fn validate_unchanged(&self) -> Result<(), HostReadError> {
        let after = stat_fd(self.file.as_raw_fd())?;
        let mut named = MaybeUninit::<libc::stat>::zeroed();
        if unsafe {
            libc::fstatat(
                self.directory.as_raw_fd(),
                self.name.as_ptr(),
                named.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(HostReadError::Invalid);
        }
        let named = unsafe { named.assume_init() };
        if !same_file_observation(&self.before, &after)
            || !same_file_observation(&self.before, &named)
        {
            return Err(HostReadError::Invalid);
        }
        Ok(())
    }

    fn read(mut self, now: DateTime<Utc>, expected_release: &str) -> Result<Value, HostReadError> {
        let mut bytes = Vec::with_capacity(self.before.st_size as usize);
        (&mut self.file)
            .take((HOST_MAX_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| HostReadError::Invalid)?;
        self.validate_unchanged()?;
        if bytes.len() != self.before.st_size as usize || bytes.len() > HOST_MAX_BYTES {
            return Err(HostReadError::Invalid);
        }
        let value = strict_json(&bytes).map_err(|_| HostReadError::Invalid)?;
        validate_host_snapshot(&value, now, expected_release)?;
        Ok(value)
    }
}

#[allow(clippy::too_many_arguments)]
fn open_snapshot(
    root: &Path,
    file_name: &Path,
    expected_uid: u32,
    expected_gid: u32,
    directory_mode: u32,
    file_mode: u32,
    expected_release: &str,
    test_mode: bool,
) -> Result<OpenSnapshot, HostReadError> {
    if !file_name.is_relative()
        || file_name.components().count() != 1
        || !matches!(file_name.components().next(), Some(Component::Normal(_)))
        || expected_release.len() != 40
        || !expected_release
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(HostReadError::Invalid);
    }
    let directory = unsafe {
        File::from_raw_fd(open_trusted_dir(
            root,
            expected_uid,
            expected_gid,
            directory_mode,
            test_mode,
        )?)
    };
    let name =
        CString::new(file_name.as_os_str().as_bytes()).map_err(|_| HostReadError::Invalid)?;
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return Err(
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
                HostReadError::Missing
            } else {
                HostReadError::Invalid
            },
        );
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let before = stat_fd(file.as_raw_fd())?;
    if !is_regular(&before)
        || before.st_nlink != 1
        || before.st_uid != expected_uid
        || before.st_gid != expected_gid
        || (before.st_mode & 0o7777) != file_mode
        || before.st_size < 1
        || before.st_size as usize > HOST_MAX_BYTES
    {
        return Err(HostReadError::Invalid);
    }
    let opened = OpenSnapshot {
        directory,
        file,
        name,
        before,
    };
    opened.validate_unchanged()?;
    Ok(opened)
}

use std::os::fd::AsRawFd;

fn open_trusted_dir(
    path: &Path,
    expected_uid: u32,
    expected_gid: u32,
    expected_mode: u32,
    test_mode: bool,
) -> Result<libc::c_int, HostReadError> {
    if !path.is_absolute()
        || path.to_string_lossy().ends_with('/')
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
        || path == Path::new("/")
    {
        return Err(HostReadError::Invalid);
    }
    let slash = CString::new("/").expect("literal");
    let mut current = unsafe {
        libc::open(
            slash.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW,
        )
    };
    if current < 0 {
        return Err(HostReadError::Invalid);
    }
    let mut built = PathBuf::from("/");
    for component in path.components().skip(1) {
        let Component::Normal(os_name) = component else {
            unsafe { libc::close(current) };
            return Err(HostReadError::Invalid);
        };
        let Ok(name) = CString::new(os_name.as_bytes()) else {
            unsafe { libc::close(current) };
            return Err(HostReadError::Invalid);
        };
        let child = unsafe {
            libc::openat(
                current,
                name.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW,
            )
        };
        if child < 0 {
            let missing = std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound;
            unsafe { libc::close(current) };
            return Err(if missing {
                HostReadError::Missing
            } else {
                HostReadError::Invalid
            });
        }
        built.push(os_name);
        let stat = match stat_fd(child) {
            Ok(value) => value,
            Err(error) => {
                unsafe {
                    libc::close(child);
                    libc::close(current);
                }
                return Err(error);
            }
        };
        if !is_directory(&stat) {
            unsafe {
                libc::close(child);
                libc::close(current);
            }
            return Err(HostReadError::Invalid);
        }
        let is_final = built == path;
        if is_final {
            if stat.st_uid != expected_uid
                || stat.st_gid != expected_gid
                || (stat.st_mode & 0o7777) != expected_mode
            {
                unsafe {
                    libc::close(child);
                    libc::close(current);
                }
                return Err(HostReadError::Invalid);
            }
        } else {
            let is_tmp = test_mode && built == Path::new("/tmp");
            let owner_ok = if test_mode && !is_tmp {
                stat.st_uid == expected_uid && stat.st_gid == expected_gid
            } else {
                stat.st_uid == 0 && stat.st_gid == 0
            };
            let mode_ok = if is_tmp {
                (stat.st_mode & libc::S_ISVTX) != 0
            } else {
                (stat.st_mode & 0o022) == 0
            };
            if !owner_ok || !mode_ok {
                unsafe {
                    libc::close(child);
                    libc::close(current);
                }
                return Err(HostReadError::Invalid);
            }
        }
        unsafe { libc::close(current) };
        current = child;
    }
    Ok(current)
}

fn stat_fd(fd: libc::c_int) -> Result<libc::stat, HostReadError> {
    let mut stat = MaybeUninit::<libc::stat>::zeroed();
    if unsafe { libc::fstat(fd, stat.as_mut_ptr()) } != 0 {
        return Err(HostReadError::Invalid);
    }
    Ok(unsafe { stat.assume_init() })
}

fn is_directory(stat: &libc::stat) -> bool {
    (stat.st_mode & libc::S_IFMT) == libc::S_IFDIR
}

fn is_regular(stat: &libc::stat) -> bool {
    (stat.st_mode & libc::S_IFMT) == libc::S_IFREG
}

fn same_file_observation(a: &libc::stat, b: &libc::stat) -> bool {
    a.st_dev == b.st_dev
        && a.st_ino == b.st_ino
        && a.st_size == b.st_size
        && a.st_mtime == b.st_mtime
        && a.st_mtime_nsec == b.st_mtime_nsec
        && a.st_ctime == b.st_ctime
        && a.st_ctime_nsec == b.st_ctime_nsec
        && a.st_uid == b.st_uid
        && a.st_gid == b.st_gid
        && a.st_mode == b.st_mode
        && a.st_nlink == b.st_nlink
}

/// Deserialize JSON while rejecting duplicate object keys at every level.
pub fn strict_json(bytes: &[u8]) -> Result<Value, serde_json::Error> {
    let text = std::str::from_utf8(bytes).map_err(|error| {
        <serde_json::Error as serde::de::Error>::custom(format!(
            "invalid UTF-8 at {}",
            error.valid_up_to()
        ))
    })?;
    let mut de = serde_json::Deserializer::from_str(text);
    let value = StrictValue::deserialize(&mut de)?.0;
    de.end()?;
    Ok(value)
}

struct StrictValue(Value);

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct StrictVisitor;
        impl<'de> Visitor<'de> for StrictVisitor {
            type Value = StrictValue;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a JSON value with unique object keys")
            }
            fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Bool(value)))
            }
            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Number(value.into())))
            }
            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Number(value.into())))
            }
            fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(value)
                    .map(|number| StrictValue(Value::Number(number)))
                    .ok_or_else(|| E::custom("non-finite number"))
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::String(value.to_owned())))
            }
            fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::String(value)))
            }
            fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Null))
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = sequence.next_element::<StrictValue>()? {
                    values.push(value.0);
                }
                Ok(StrictValue(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut values = Map::new();
                while let Some((key, value)) = map.next_entry::<String, StrictValue>()? {
                    if values.insert(key, value.0).is_some() {
                        return Err(serde::de::Error::custom("duplicate object key"));
                    }
                }
                Ok(StrictValue(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(StrictVisitor)
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct HostSnapshot {
    schema_version: String,
    generated_at: String,
    collection_started_at: String,
    booted_at: String,
    policy: HostPolicy,
    observations: Vec<HostObservation>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct HostPolicy {
    schema_version: String,
    profile: String,
    release_commit: String,
    compose_project: String,
    owner_access: String,
    quote_transport: String,
    window_source: String,
    research_mode: String,
    policy_installed_at: String,
    job_lease_seconds: HashMap<String, u32>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct HostObservation {
    id: String,
    result: String,
    observed_at: Option<String>,
    process: Option<HostProcess>,
    timer: Option<HostTimer>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct HostProcess {
    state: String,
    health: String,
    started_at: Option<String>,
    finished_at: Option<String>,
    exit_code: Option<u8>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct HostTimer {
    load: String,
    active: String,
    enabled: String,
    last_trigger_at: Option<String>,
    next_run_at: Option<String>,
    unit_binding: String,
    last_result: String,
    exit_status: Option<u8>,
    last_started_at: Option<String>,
    last_finished_at: Option<String>,
    last_success_at: Option<String>,
}

fn parse_instant(value: &str) -> Option<DateTime<Utc>> {
    if !value.ends_with('Z') {
        return None;
    }
    let base = value.strip_suffix('Z')?;
    if let Some((_, fraction)) = base.rsplit_once('.')
        && (fraction.is_empty()
            || fraction.len() > 6
            || !fraction.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

fn fmt_instant(value: DateTime<Utc>) -> String {
    // DB and accepted source clocks already have microsecond precision. Only
    // the local serving clock may be finer; truncate without widening a bound.
    DateTime::from_timestamp_micros(value.timestamp_micros())
        .expect("supported UTC instant")
        .to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

fn get_instant(value: &Value) -> Option<DateTime<Utc>> {
    parse_instant(value.as_str()?)
}

fn optional_instant(value: &Value) -> Option<DateTime<Utc>> {
    if value.is_null() {
        None
    } else {
        get_instant(value)
    }
}

fn validate_host_snapshot(
    value: &Value,
    now: DateTime<Utc>,
    expected_release: &str,
) -> Result<(), HostReadError> {
    let snapshot: HostSnapshot =
        serde_json::from_value(value.clone()).map_err(|_| HostReadError::Invalid)?;
    if snapshot.schema_version != "service-status.host.v1"
        || snapshot.policy.schema_version != "service-status.policy.v1"
        || snapshot.observations.len() != 20
        || snapshot.policy.release_commit != expected_release
    {
        return Err(if snapshot.policy.release_commit != expected_release {
            HostReadError::PolicyMismatch
        } else {
            HostReadError::Invalid
        });
    }
    if !matches!(
        snapshot.policy.profile.as_str(),
        "owner_daily" | "owner_daemon"
    ) || !matches!(snapshot.policy.research_mode.as_str(), "daily" | "daemon")
        || snapshot.profile_for_mode_mismatch()
        || snapshot.policy.owner_access != "owner_only"
        || snapshot.policy.quote_transport != "market_ws"
        || snapshot.policy.window_source != "operational_v1"
        || snapshot.policy.compose_project.is_empty()
        || snapshot.policy.compose_project.len() > 63
        || !snapshot
            .policy
            .compose_project
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        || snapshot.policy.job_lease_seconds.len() != 5
    {
        return Err(HostReadError::Invalid);
    }
    for key in [
        "owner_equity_v2",
        "owner_beta_price_recommendation",
        "candidate_compute",
        "recommendation",
        "backtest",
    ] {
        if !snapshot
            .policy
            .job_lease_seconds
            .get(key)
            .is_some_and(|v| (1..=3600).contains(v))
        {
            return Err(HostReadError::Invalid);
        }
    }
    let generated = parse_instant(&snapshot.generated_at).ok_or(HostReadError::Invalid)?;
    let started = parse_instant(&snapshot.collection_started_at).ok_or(HostReadError::Invalid)?;
    let booted = parse_instant(&snapshot.booted_at).ok_or(HostReadError::Invalid)?;
    let installed =
        parse_instant(&snapshot.policy.policy_installed_at).ok_or(HostReadError::Invalid)?;
    if generated > now || started > generated || booted > started || installed > now {
        return Err(HostReadError::Future);
    }
    if generated - started > ChronoDuration::seconds(20) {
        return Err(HostReadError::Invalid);
    }
    let catalog = catalog();
    let expected: BTreeSet<&str> = catalog
        .iter()
        .filter(|item| {
            item.get("host_source")
                .and_then(Value::as_str)
                .is_some_and(|source| source != "none" && source != "parent")
        })
        .filter_map(|item| item.get("id").and_then(Value::as_str))
        .collect();
    let mut found = BTreeSet::new();
    for observation in &snapshot.observations {
        if !found.insert(observation.id.as_str()) || !expected.contains(observation.id.as_str()) {
            return Err(HostReadError::Invalid);
        }
        if !matches!(
            observation.result.as_str(),
            "ok" | "missing"
                | "timeout"
                | "command_unavailable"
                | "denied"
                | "invalid"
                | "deadline"
        ) {
            return Err(HostReadError::Invalid);
        }
        let expected_at = match observation.observed_at.as_deref() {
            Some(value) => Some(parse_instant(value).ok_or(HostReadError::Invalid)?),
            None => None,
        };
        if observation.result == "ok" && expected_at.is_none() {
            return Err(HostReadError::Invalid);
        }
        if let Some(observed) = expected_at {
            if observed > now || observed < started || observed > generated || observed < booted {
                return Err(HostReadError::Future);
            }
        }
        if let Some(process) = &observation.process {
            if !matches!(
                process.state.as_str(),
                "running"
                    | "created"
                    | "restarting"
                    | "paused"
                    | "exited"
                    | "dead"
                    | "removing"
                    | "active"
                    | "activating"
                    | "inactive"
                    | "failed"
                    | "deactivating"
                    | "missing"
                    | "unknown"
            ) || !matches!(
                process.health.as_str(),
                "healthy" | "unhealthy" | "starting" | "none" | "unknown"
            ) {
                return Err(HostReadError::Invalid);
            }
            validate_ordered_times(
                process.started_at.as_deref(),
                process.finished_at.as_deref(),
                booted,
                generated,
            )?;
        }
        if let Some(timer) = &observation.timer {
            if !matches!(
                timer.load.as_str(),
                "loaded" | "not-found" | "masked" | "error"
            ) || !matches!(
                timer.active.as_str(),
                "active" | "inactive" | "activating" | "failed" | "unknown"
            ) || !matches!(
                timer.enabled.as_str(),
                "enabled" | "disabled" | "masked" | "static" | "unknown"
            ) || !matches!(
                timer.unit_binding.as_str(),
                "matched" | "mismatch" | "unknown"
            ) || !matches!(
                timer.last_result.as_str(),
                "success" | "failure" | "none" | "unknown"
            ) {
                return Err(HostReadError::Invalid);
            }
            validate_ordered_times(
                timer.last_started_at.as_deref(),
                timer.last_finished_at.as_deref(),
                booted,
                generated,
            )?;
            for instant in [
                timer.last_trigger_at.as_deref(),
                timer.last_started_at.as_deref(),
                timer.last_finished_at.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                let parsed = parse_instant(instant).ok_or(HostReadError::Invalid)?;
                if parsed > generated || parsed < booted {
                    return Err(HostReadError::Future);
                }
            }
            // Verified success survives reboot independently of current
            // process/trigger intervals. Keep its original source clock and
            // strict upper bound; only current observations use the boot floor.
            if let Some(instant) = timer.last_success_at.as_deref() {
                let parsed = parse_instant(instant).ok_or(HostReadError::Invalid)?;
                if parsed > generated {
                    return Err(HostReadError::Future);
                }
            }
            if timer
                .next_run_at
                .as_deref()
                .is_some_and(|value| parse_instant(value).is_none())
            {
                return Err(HostReadError::Invalid);
            }
            if let (Some(success), Some(finished)) =
                (&timer.last_success_at, &timer.last_finished_at)
                && parse_instant(success)
                    .zip(parse_instant(finished))
                    .is_some_and(|(a, b)| a > b)
            {
                return Err(HostReadError::Invalid);
            }
        }
    }
    if found != expected {
        return Err(HostReadError::Invalid);
    }
    // Every source kind has an exact shape; capability IDs never come from
    // the host observer and scheduled units always carry both process/timer.
    for item in catalog.iter().filter(|item| {
        item.get("host_source")
            .and_then(Value::as_str)
            .is_some_and(|source| source != "none" && source != "parent")
    }) {
        let id = item.get("id").and_then(Value::as_str).unwrap_or_default();
        let source = item
            .get("host_source")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let obs = snapshot
            .observations
            .iter()
            .find(|obs| obs.id == id)
            .ok_or(HostReadError::Invalid)?;
        let shape_ok = match source {
            "daily_or_daemon" => obs.process.is_some() && obs.timer.is_some(),
            "systemd" if item.get("kind").and_then(Value::as_str) == Some("scheduled") => {
                obs.process.is_some() && obs.timer.is_some()
            }
            "systemd" => obs.process.is_some() && obs.timer.is_none(),
            _ => obs.process.is_some() && obs.timer.is_none(),
        };
        if obs.result == "ok" && !shape_ok {
            return Err(HostReadError::Invalid);
        }
    }
    Ok(())
}

impl HostSnapshot {
    fn profile_for_mode_mismatch(&self) -> bool {
        self.policy.profile
            != if self.policy.research_mode == "daily" {
                "owner_daily"
            } else {
                "owner_daemon"
            }
    }
}

fn validate_ordered_times(
    start: Option<&str>,
    finish: Option<&str>,
    booted: DateTime<Utc>,
    generated: DateTime<Utc>,
) -> Result<(), HostReadError> {
    let start = start
        .map(|value| parse_instant(value).ok_or(HostReadError::Invalid))
        .transpose()?;
    let finish = finish
        .map(|value| parse_instant(value).ok_or(HostReadError::Invalid))
        .transpose()?;
    if start.is_some_and(|v| v < booted || v > generated)
        || finish.is_some_and(|v| v < booted || v > generated)
        || start.zip(finish).is_some_and(|(a, b)| b < a)
    {
        return Err(HostReadError::Invalid);
    }
    Ok(())
}

fn catalog() -> &'static Vec<Value> {
    static CATALOG: OnceLock<Vec<Value>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str::<Value>(CATALOG_JSON)
            .expect("embedded service-status catalog is valid")
            .get("services")
            .and_then(Value::as_array)
            .expect("catalog services array")
            .clone()
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Failed,
    Degraded,
    Unknown,
    Waiting,
    Healthy,
    Disabled,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Self::Failed => "failed",
            Self::Degraded => "degraded",
            Self::Unknown => "unknown",
            Self::Waiting => "waiting",
            Self::Healthy => "healthy",
            Self::Disabled => "disabled",
        }
    }

    fn rank(self) -> u8 {
        match self {
            Self::Failed => 0,
            Self::Degraded => 1,
            Self::Unknown => 2,
            Self::Waiting => 3,
            Self::Healthy => 4,
            Self::Disabled => 5,
        }
    }
}

#[derive(Debug, Clone)]
struct Fact {
    status: Status,
    reason: String,
}

impl Fact {
    fn new(status: Status, reason: impl Into<String>) -> Self {
        Self {
            status,
            reason: reason.into(),
        }
    }
}

/// Evaluate one input against the supplied clock. Test expectations are
/// intentionally applied by tests outside this reducer.
pub fn project(input: &Value, now: DateTime<Utc>) -> Result<Value, &'static str> {
    project_with_host_error(input, now, None)
}

pub fn project_with_host_error(
    input: &Value,
    now: DateTime<Utc>,
    host_error: Option<&str>,
) -> Result<Value, &'static str> {
    if !input.is_object() {
        return Err("evaluation input must be an object");
    }
    let host_read = input.get("host_read").ok_or("host_read missing")?;
    let db = input
        .get("db_read_results")
        .ok_or("db_read_results missing")?;
    let window = input.get("window_read").ok_or("window_read missing")?;
    let raw_snapshot = host_read.get("snapshot").filter(|v| !v.is_null());
    let snapshot = raw_snapshot
        .map(|value| {
            serde_json::from_value::<HostSnapshot>(value.clone())
                .map_err(|_| "host snapshot shape invalid")
        })
        .transpose()?;
    let validation_error = snapshot.as_ref().and_then(|host| {
        validate_host_snapshot(
            raw_snapshot.expect("parsed snapshot has raw value"),
            now,
            &host.policy.release_commit,
        )
        .err()
    });
    // Reject the host source without aborting independent database evaluation.
    // A future envelope rejects observations; already-validated non-future
    // policy receipt fields are independent of the observer's source clocks.
    let host_error = host_error.or(validation_error.map(HostReadError::reason));
    if validation_error.is_some_and(|error| error != HostReadError::Future) {
        let mut rejected = input.clone();
        rejected["host_read"]["snapshot"] = Value::Null;
        rejected["host_read"]["result"] = json!(validation_error.unwrap().result());
        return project_with_host_error(&rejected, now, host_error);
    }
    let host_result = host_read
        .get("result")
        .and_then(Value::as_str)
        .unwrap_or("unavailable");
    let (policy_state, policy_mode) = match snapshot.as_ref() {
        Some(_) if host_error == Some("POLICY_MISMATCH") => ("mismatch", None),
        Some(host) if parse_instant(&host.policy.policy_installed_at).is_none_or(|at| at > now) => {
            ("unavailable", None)
        }
        Some(_) if host_result != "ok" => ("unavailable", None),
        Some(host) => {
            let expected_commit = input.get("expected_release").and_then(Value::as_str);
            if expected_commit.is_some_and(|commit| commit != host.policy.release_commit) {
                ("mismatch", None)
            } else {
                ("verified", Some(host.policy.research_mode.as_str()))
            }
        }
        None if host_result == "invalid" && host_error == Some("POLICY_MISMATCH") => {
            ("mismatch", None)
        }
        None => ("unavailable", None),
    };
    let host_map = if let Some(host) = &snapshot {
        host.observations
            .iter()
            .map(|observation| {
                let value = serde_json::to_value(observation).expect("host observation serializes");
                (observation.id.clone(), value)
            })
            .collect::<HashMap<_, _>>()
    } else {
        HashMap::new()
    };
    let source_policy = snapshot.as_ref().map(|host| &host.policy);
    let mut rows = Vec::with_capacity(22);
    let mut overall = Vec::new();
    for item in catalog() {
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .ok_or("catalog id missing")?;
        let row = project_service(
            id,
            item,
            &host_map,
            host_read,
            db,
            window,
            source_policy,
            policy_state,
            policy_mode,
            now,
            host_error,
        )?;
        if item
            .get("contributes_to_overall")
            .and_then(Value::as_bool)
            .unwrap_or(true)
        {
            overall.push(
                row.get("status")
                    .and_then(Value::as_str)
                    .and_then(parse_status)
                    .unwrap_or(Status::Unknown),
            );
        }
        rows.push(row);
    }
    let overall_status = overall
        .into_iter()
        .min_by_key(|status| status.rank())
        .unwrap_or(Status::Disabled);
    Ok(json!({
        "schema_version": "service-status.api.v1",
        "generated_at": fmt_instant(now),
        "overall_status": overall_status.as_str(),
        "policy_state": policy_state,
        "services": rows,
    }))
}

fn parse_status(value: &str) -> Option<Status> {
    match value {
        "failed" => Some(Status::Failed),
        "degraded" => Some(Status::Degraded),
        "unknown" => Some(Status::Unknown),
        "waiting" => Some(Status::Waiting),
        "healthy" => Some(Status::Healthy),
        "disabled" => Some(Status::Disabled),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn project_service(
    id: &str,
    item: &Value,
    host_map: &HashMap<String, Value>,
    host_read: &Value,
    db: &Value,
    window: &Value,
    policy: Option<&HostPolicy>,
    policy_state: &str,
    policy_mode: Option<&str>,
    now: DateTime<Utc>,
    host_error: Option<&str>,
) -> Result<Value, &'static str> {
    let kind = item.get("kind").and_then(Value::as_str).unwrap_or("daemon");
    let source_kind = item
        .get("host_source")
        .and_then(Value::as_str)
        .unwrap_or("none");
    let host_required = source_kind != "none";
    let uses_database = matches!(
        id,
        "owner-equity-v2-runner"
            | "owner-beta-runner"
            | "candidate-runner"
            | "recommendation-runner"
            | "nt-backtest-worker-1"
            | "nt-backtest-worker-2"
            | "kis-eod"
            | "market-stream"
            | "intraday-calendar"
    );
    let uses_window = matches!(id, "market-stream" | "intraday-calendar");
    let expected_mode = expected_mode(item, policy_mode);
    // Capability rows backed by a parent process keep the parent's host
    // observation as process evidence; they have no separate observer row.
    let service_observation = host_map.get(id);
    let observation = if source_kind == "parent" {
        host_map.get("owner-equity-v2-runner")
    } else if id == "kis-eod" && expected_mode == "daemon" {
        host_map.get("research-worker")
    } else {
        service_observation
    };
    let process_source = if host_required {
        Some(source_for_host(host_read, observation, now, host_error))
    } else {
        None
    };
    let host_source = if id == "kis-eod" {
        let timer_source = source_for_host(host_read, service_observation, now, host_error);
        let worker_source =
            source_for_host(host_read, host_map.get("research-worker"), now, host_error);
        Some(merge_host_sources(&timer_source, &worker_source))
    } else {
        process_source.clone()
    };
    let window_source = if uses_window {
        Some(source_for_window(window, now))
    } else {
        None
    };
    let db_source = if uses_database {
        Some(source_for_database(id, db, now))
    } else {
        None
    };
    let mut metrics = empty_metrics();
    let mut work_status = "unknown";
    let mut facts: Vec<Fact> = Vec::new();
    let mut semantic_until: Option<DateTime<Utc>> = None;

    let (mut process_status, host_fact, until) = if host_required {
        host_process_fact(
            id,
            expected_mode,
            observation,
            process_source.as_ref(),
            now,
            host_error,
        )
    } else {
        ("not_applicable", None, None)
    };
    semantic_until = min_time(semantic_until, until);
    if let Some(fact) = host_fact {
        // A healthy process is operational context for database-backed
        // services, while their work evidence carries the user-facing reason.
        // Preserve every non-healthy host fact so process failures still win.
        if !(uses_database && fact.reason == "PROCESS_RUNNING") {
            facts.push(fact);
        }
    }

    let mut work_facts = Vec::new();
    match id {
        "owner-equity-v2-runner" => {
            let q = db.get("queue").ok_or("queue group missing")?;
            let v2 = db.get("v2").ok_or("v2 group missing")?;
            let calendar = db.get("calendar").ok_or("calendar group missing")?;
            let calendar_view = calendar_metric_values(&mut metrics, calendar, &Value::Null, now);
            if let Some(fact) = calendar_view.reason.clone() {
                work_facts.push(fact);
            }
            let queue = queue_view(q, "owner_equity_v2", now, policy);
            put_queue_metrics(&mut metrics, &queue, "actor");
            let v2_fact = v2_fact(v2, calendar, now, &mut metrics);
            let queue_is_relevant = queue.has_failure || queue.work_status != "idle";
            if queue_is_relevant {
                work_status = queue.work_status;
                work_facts.extend(queue.facts);
            } else if let Some(fact) = v2_fact.as_ref() {
                work_status = match fact.reason.as_str() {
                    "DATA_CURRENT" => "current",
                    "EOD_PENDING" => "pending",
                    "SNAPSHOT_STALE" | "V2_COVERAGE_INCOMPLETE" | "EOD_STALE" => "stale",
                    "DB_TIMEOUT" | "DB_UNAVAILABLE" | "SCHEMA_DRIFT" | "SOURCE_FUTURE"
                    | "QUERY_LIMIT" => "unknown",
                    _ => "unknown",
                };
            }
            if let Some(fact) = v2_fact {
                work_facts.push(fact);
            }
            semantic_until = min_time(semantic_until, queue.valid_until);
            semantic_until = min_time(semantic_until, v2_semantic_until(v2, now));
            semantic_until = min_time(semantic_until, calendar_view.current_valid_until);
            semantic_until = min_time(semantic_until, eod_transition_until(now, &metrics));
        }
        "owner-beta-runner"
        | "recommendation-runner"
        | "nt-backtest-worker-1"
        | "nt-backtest-worker-2" => {
            let q = db.get("queue").ok_or("queue group missing")?;
            let job = item
                .get("job_type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let queue = queue_view(q, job, now, policy);
            put_queue_metrics(
                &mut metrics,
                &queue,
                if job == "backtest" {
                    "actor_shared"
                } else {
                    "actor"
                },
            );
            work_status = queue.work_status;
            if id == "owner-beta-runner" {
                metrics["last_success_at"] = max_time(
                    metrics["last_success_at"].as_str().and_then(parse_instant),
                    q.get("owner_beta_success_at")
                        .and_then(optional_instant)
                        .filter(|at| *at <= now),
                )
                .map(fmt_instant)
                .map(Value::String)
                .unwrap_or(Value::Null);
            }
            semantic_until = min_time(semantic_until, queue.valid_until);
            work_facts.extend(queue.facts);
        }
        "candidate-runner" => {
            let q = db.get("queue").ok_or("queue group missing")?;
            let candidate = db.get("candidate").ok_or("candidate group missing")?;
            let calendar = db.get("calendar").ok_or("calendar group missing")?;
            let calendar_view = calendar_metric_values(&mut metrics, calendar, &Value::Null, now);
            metrics["data_due_at"] = Value::Null;
            if let Some(fact) = calendar_view.reason.clone() {
                work_facts.push(fact);
            }
            let queue = queue_view(q, "candidate_compute", now, policy);
            put_queue_metrics(&mut metrics, &queue, "actor");
            let candidate_fact = candidate_fact(candidate, calendar, now, &mut metrics);
            work_status = queue.work_status;
            work_facts.extend(queue.facts);
            if let Some(fact) = candidate_fact {
                if fact.reason != "DATA_CURRENT" {
                    if queue.work_status == "idle" && !queue.has_failure {
                        work_status = if fact.reason == "EOD_PENDING" {
                            "pending"
                        } else if fact.reason == "EOD_STALE" {
                            "stale"
                        } else if fact.status == Status::Degraded {
                            "degraded"
                        } else {
                            "unknown"
                        };
                        work_facts.retain(|fact| fact.reason != "QUEUE_IDLE");
                    }
                    work_facts.push(fact);
                }
            }
            semantic_until = min_time(semantic_until, queue.valid_until);
            semantic_until = min_time(semantic_until, candidate_semantic_until(candidate, now));
            semantic_until = min_time(semantic_until, calendar_view.current_valid_until);
            semantic_until = min_time(semantic_until, eod_transition_until(now, &metrics));
        }
        "kis-eod" => {
            let etf = db.get("etf").ok_or("etf group missing")?;
            let research = db.get("research").ok_or("research group missing")?;
            let calendar = db.get("calendar").ok_or("calendar group missing")?;
            if let Some(fact) = calendar_eod_view(calendar, now).reason {
                work_facts.push(fact);
            }
            if let Some(fact) = research_negative_fact(research, now, &mut metrics) {
                work_facts.push(fact);
            }
            if let Some(fact) = etf_negative_fact(etf, calendar, now, &mut metrics) {
                work_facts.push(fact);
            }
            let worker = host_map.get("research-worker");
            let timer_source = source_for_host(host_read, service_observation, now, host_error);
            let worker_source = source_for_host(host_read, worker, now, host_error);
            for source in [&timer_source, &worker_source] {
                if let Some(fact) = host_source_fact(Some(source), host_error) {
                    work_facts.push(fact);
                }
            }
            let (fact, work, until) = eod_fact(
                etf,
                research,
                calendar,
                worker.filter(|_| worker_source["result"] == "ok"),
                service_observation,
                timer_source["result"] == "ok",
                policy_mode,
                now,
                &mut metrics,
            );
            work_status = work;
            if let Some(fact) = fact {
                work_facts.push(fact);
            }
            if work_status == "unknown" {
                if work_facts.iter().any(|fact| fact.status == Status::Failed) {
                    work_status = "failed";
                } else if work_facts
                    .iter()
                    .any(|fact| fact.status == Status::Degraded)
                {
                    work_status = "stale";
                }
            }
            semantic_until = min_time(semantic_until, until);
        }
        "market-stream" => {
            let stream = db.get("stream").ok_or("stream group missing")?;
            let calendar = db.get("calendar").ok_or("calendar group missing")?;
            let mut facts = stream_facts(
                stream,
                calendar,
                window,
                now,
                &mut metrics,
                &mut semantic_until,
            );
            // Only validated source/binding evidence populates these metrics.
            // Diagnose exposed clocks independently after every stream branch
            // has retained its facts, counts and exact assurance boundaries.
            if [
                "last_received_at",
                "oldest_provider_event_at",
                "last_state_change_at",
            ]
            .iter()
            .any(|name| {
                metrics
                    .get(*name)
                    .and_then(optional_instant)
                    .is_some_and(|at| at > now)
            }) {
                facts.push(Fact::new(Status::Degraded, "SOURCE_FUTURE"));
            }
            let winning = facts
                .iter()
                .map(|fact| fact.status)
                .min_by_key(|status| status.rank())
                .unwrap_or(Status::Unknown);
            let reason = facts
                .iter()
                .find(|fact| fact.status == winning)
                .map(|fact| fact.reason.as_str())
                .unwrap_or("DB_UNAVAILABLE");
            work_status = stream_work_status(stream, window, calendar, now, winning, reason);
            work_facts.extend(facts);
        }
        "intraday-calendar" => {
            let calendar = db.get("calendar").ok_or("calendar group missing")?;
            let (fact, calendar_status, window_state, until) =
                calendar_window_fact(calendar, window, now);
            metrics["calendar_status"] = json!(calendar_status);
            metrics["window_status"] = json!(if matches!(
                window_state,
                "missing" | "invalid" | "unknown"
            ) {
                window_state
            } else {
                window_status(window, now)
            });
            if let Some(fact) = fact.filter(|fact| fact.status != Status::Waiting) {
                work_status = if fact.status == Status::Unknown {
                    "unknown"
                } else {
                    "failed"
                };
                work_facts.push(fact);
            } else {
                work_status = "current";
                work_facts.push(Fact::new(Status::Healthy, "DATA_CURRENT"));
            }
            if let Some(window_failure) = window_negative_fact(window, now) {
                work_status = if window_failure.status == Status::Failed {
                    "failed"
                } else {
                    work_status
                };
                work_facts.push(window_failure);
            }
            semantic_until = min_time(semantic_until, until);
        }
        "research-worker" => {
            if expected_mode == "unknown" {
                process_status = "unknown";
            }
            work_status = "not_applicable";
            if expected_mode == "unknown" {
                work_facts.push(Fact::new(Status::Unknown, "POLICY_MISMATCH"));
            } else if expected_mode == "daily" {
                work_facts.push(Fact::new(Status::Waiting, "NOT_APPLICABLE"));
            }
        }
        "production-backup" | "production-backup-verify" | "tls-renewal" => {
            let timer = observation.and_then(|v| v.get("timer"));
            let process_state = observation
                .and_then(|value| value.pointer("/process/state"))
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let process_started = observation
                .and_then(|value| value.pointer("/process/started_at"))
                .and_then(optional_instant);
            let (fact, work, next, attempt, success, failure, until) =
                scheduled_fact(id, timer, process_state, process_started, now);
            work_status = work;
            if let Some(fact) = fact {
                work_facts.push(fact);
            }
            metrics["next_run_at"] = next.map(|v| json!(fmt_instant(v))).unwrap_or(Value::Null);
            metrics["last_attempt_at"] = attempt
                .map(|v| json!(fmt_instant(v)))
                .unwrap_or(Value::Null);
            metrics["last_success_at"] = success
                .map(|v| json!(fmt_instant(v)))
                .unwrap_or(Value::Null);
            metrics["last_failure_at"] = failure
                .map(|v| json!(fmt_instant(v)))
                .unwrap_or(Value::Null);
            semantic_until = min_time(semantic_until, until);
        }
        _ => {
            work_status = if expected_mode == "disabled" {
                "disabled"
            } else {
                "not_applicable"
            };
        }
    }

    if policy_state != "verified" && !matches!(id, "research-worker") {
        let known_failure = facts
            .iter()
            .chain(work_facts.iter())
            .any(|fact| fact.status == Status::Failed);
        if !known_failure {
            work_facts.push(Fact::new(
                Status::Unknown,
                if policy_state == "mismatch" {
                    "POLICY_MISMATCH"
                } else {
                    "POLICY_MISMATCH"
                },
            ));
            if expected_mode == "unknown" {
                process_status = if host_required && process_status != "unknown" {
                    process_status
                } else {
                    "unknown"
                };
            }
        }
    }

    if facts.is_empty() && work_facts.is_empty() {
        facts.push(Fact::new(Status::Unknown, "DB_UNAVAILABLE"));
    }
    let all_facts = facts
        .iter()
        .chain(work_facts.iter())
        .cloned()
        .collect::<Vec<_>>();
    let status = all_facts
        .iter()
        .map(|fact| fact.status)
        .min_by_key(|status| status.rank())
        .unwrap_or(Status::Unknown);
    let has_negative = all_facts
        .iter()
        .any(|fact| fact.status.rank() <= Status::Unknown.rank());
    // Verified work context can accompany a known present process failure or
    // readiness diagnosis. Missing/unvalidated processes and invalid work
    // proofs cannot supply positive context. Every independent negative stays.
    let work_context_valid = !matches!(process_status, "missing" | "unknown")
        && !work_facts
            .iter()
            .any(|fact| fact.status.rank() <= Status::Unknown.rank());
    let mut reason_groups: Vec<(u8, String)> = all_facts
        .iter()
        .filter(|fact| {
            !has_negative || work_context_valid || fact.status.rank() <= Status::Unknown.rank()
        })
        .map(|fact| (fact.status.rank(), fact.reason.clone()))
        .collect();
    reason_groups.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let mut unique_reasons = BTreeSet::new();
    reason_groups.retain(|(_, reason)| unique_reasons.insert(reason.clone()));
    let mut reasons = reason_groups
        .into_iter()
        .map(|(_, reason)| reason)
        .collect::<Vec<_>>();
    reasons.truncate(12);
    if reasons.is_empty() {
        reasons.push("SOURCE_UNAVAILABLE".to_owned());
    }

    let host_src = host_source;
    let db_src = db_source;
    let window_src = window_source;
    let source_times = [host_src.as_ref(), db_src.as_ref(), window_src.as_ref()]
        .into_iter()
        .flatten()
        .filter_map(|src| src.get("observed_at").and_then(get_instant))
        .collect::<Vec<_>>();
    let observed_at = source_times.iter().copied().min();
    let source_valids = [host_src.as_ref(), db_src.as_ref(), window_src.as_ref()]
        .into_iter()
        .flatten()
        .filter_map(|src| src.get("valid_until").and_then(optional_instant))
        .collect::<Vec<_>>();
    let mut valid_until = source_valids
        .into_iter()
        .chain(std::iter::once(semantic_until).flatten())
        .min();
    if status == Status::Unknown {
        valid_until = None;
    }

    Ok(json!({
        "id": id,
        "kind": kind,
        "expected_mode": expected_mode,
        "process_status": process_status,
        "work_status": work_status,
        "status": status.as_str(),
        "reason_codes": reasons,
        "observed_at": observed_at.map(fmt_instant),
        "valid_until": valid_until.map(fmt_instant),
        "sources": {
            "host": host_src.unwrap_or(Value::Null),
            "database": db_src.unwrap_or(Value::Null),
            "window": window_src.unwrap_or(Value::Null),
        },
        "metrics": metrics,
    }))
}

fn empty_metrics() -> Value {
    json!({
        "last_attempt_at": null, "last_success_at": null, "last_failure_at": null,
        "last_received_at": null, "oldest_provider_event_at": null, "next_run_at": null,
        "data_due_at": null, "data_as_of": null, "expected_session": null, "raw_as_of": null,
        "v2_data_as_of": null, "v2_snapshot_as_of": null, "queued": null, "running": null,
        "recent_failed": null, "expired_claims": null, "active_demand": null,
        "live_identities": null, "halted_identities": null, "required_identities": null,
        "covered_identities": null, "queue_scope": "not_applicable", "calendar_status": "not_applicable",
        "window_status": "not_applicable", "last_state_change_at": null
    })
}

fn expected_mode(item: &Value, policy_mode: Option<&str>) -> &'static str {
    if policy_mode.is_none() {
        return "unknown";
    }
    let daily = policy_mode == Some("daily");
    let mode = item
        .get(if daily {
            "owner_daily_mode"
        } else {
            "owner_daemon_mode"
        })
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    match mode {
        "always" => "always",
        "daily" => "daily",
        "daemon" => "daemon",
        "capability" => "capability",
        "disabled" => "disabled",
        "daily_or_daemon" => {
            if daily {
                "daily"
            } else {
                "daemon"
            }
        }
        _ => "unknown",
    }
}

fn min_time(current: Option<DateTime<Utc>>, next: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
    match (current, next) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (None, value) | (value, None) => value,
    }
}

fn source_object(
    result: &str,
    observed: Option<DateTime<Utc>>,
    valid_until: Option<DateTime<Utc>>,
) -> Value {
    json!({
        "result": result,
        "observed_at": observed.map(fmt_instant),
        "valid_until": valid_until.map(fmt_instant),
    })
}

fn map_host_result(result: &str) -> &'static str {
    match result {
        "ok" => "ok",
        "missing" => "missing",
        "timeout" => "timeout",
        "command_unavailable" => "unavailable",
        "denied" => "denied",
        "invalid" => "invalid",
        "deadline" => "timeout",
        _ => "invalid",
    }
}

fn host_reason(result: &str) -> &'static str {
    match result {
        "missing" => "HOST_MISSING",
        "timeout" => "HOST_TIMEOUT",
        "command_unavailable" => "HOST_COMMAND_UNAVAILABLE",
        "denied" => "HOST_PERMISSION_DENIED",
        "deadline" => "HOST_DEADLINE",
        "invalid" => "HOST_INVALID",
        _ => "HOST_INVALID",
    }
}

fn source_for_host(
    host_read: &Value,
    observation: Option<&Value>,
    now: DateTime<Utc>,
    host_error: Option<&str>,
) -> Value {
    let root_result = host_read
        .get("result")
        .and_then(Value::as_str)
        .unwrap_or("unavailable");
    if let Some(reason) = host_error {
        return source_object(
            if reason == "HOST_MISSING" {
                "missing"
            } else if reason == "HOST_FUTURE" {
                "future"
            } else {
                "invalid"
            },
            None,
            None,
        );
    }
    if root_result != "ok" {
        return source_object(map_root_host_result(root_result), None, None);
    }
    let Some(observation) = observation else {
        return source_object("missing", None, None);
    };
    let result = observation
        .get("result")
        .and_then(Value::as_str)
        .unwrap_or("invalid");
    let observed = observation.get("observed_at").and_then(optional_instant);
    if result != "ok" {
        return source_object(map_host_result(result), observed, None);
    }
    let Some(observed) = observed else {
        return source_object("invalid", None, None);
    };
    if observed > now {
        return source_object("future", Some(observed), None);
    }
    let valid_until = observed + ChronoDuration::seconds(HOST_TTL_SECONDS);
    if now > valid_until {
        source_object("stale", None, None)
    } else {
        source_object("ok", Some(observed), Some(valid_until))
    }
}

fn merge_host_sources(left: &Value, right: &Value) -> Value {
    let left_result = left["result"].as_str().unwrap_or("invalid");
    let right_result = right["result"].as_str().unwrap_or("invalid");
    let result = if left_result == right_result {
        left_result
    } else {
        "partial"
    };
    source_object(
        result,
        min_time(
            optional_instant(&left["observed_at"]),
            optional_instant(&right["observed_at"]),
        ),
        if result == "ok" {
            min_time(
                optional_instant(&left["valid_until"]),
                optional_instant(&right["valid_until"]),
            )
        } else {
            None
        },
    )
}

fn map_root_host_result(result: &str) -> &'static str {
    match result {
        "missing" => "missing",
        "unavailable" => "unavailable",
        "invalid" => "invalid",
        "ok" => "invalid",
        _ => "invalid",
    }
}

fn source_for_window(window: &Value, now: DateTime<Utc>) -> Value {
    let result = window
        .get("result")
        .and_then(Value::as_str)
        .unwrap_or("unavailable");
    if result != "ok" {
        return source_object(map_window_result(result), None, None);
    }
    let observed = window.get("observed_at").and_then(optional_instant);
    let Some(observed) = observed else {
        return source_object("invalid", None, None);
    };
    if observed > now
        || window
            .pointer("/entry/evidence_retrieved_at")
            .and_then(optional_instant)
            .is_some_and(|at| at > now)
    {
        return source_object("future", None, None);
    }
    let Some(domain_bound) = window_semantic_until(window, observed) else {
        return source_object("invalid", None, None);
    };
    let valid_until = observed + ChronoDuration::seconds(DB_TTL_SECONDS);
    source_object(
        if now > valid_until { "stale" } else { "ok" },
        Some(observed),
        Some(valid_until.min(domain_bound)),
    )
}

fn window_semantic_until(window: &Value, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    if window.get("result").and_then(Value::as_str) != Some("ok")
        || window
            .get("activation_hash_matches")
            .and_then(Value::as_bool)
            != Some(true)
        || window.get("metadata_safe").and_then(Value::as_bool) != Some(true)
    {
        return None;
    }
    let entry = window.get("entry")?;
    let evidence = entry
        .get("evidence_retrieved_at")
        .and_then(optional_instant)?;
    let today = kst_date(now);
    if parse_date(&entry["date"]) != Some(today) || evidence > now || kst_date(evidence) != today {
        return None;
    }
    let kst = FixedOffset::east_opt(KST_SECONDS)?;
    let midnight = today
        .succ_opt()
        .and_then(|date| {
            kst.from_local_datetime(&date.and_time(NaiveTime::MIN))
                .single()
        })
        .map(|at| at.with_timezone(&Utc));
    match entry.get("disposition").and_then(Value::as_str) {
        Some("closed") if entry["open_at"].is_null() && entry["close_at"].is_null() => midnight,
        Some("trading") => {
            let open = entry.get("open_at").and_then(optional_instant)?;
            let close = entry.get("close_at").and_then(optional_instant)?;
            if open >= close || kst_date(open) != today || kst_date(close) != today {
                return None;
            }
            [
                midnight,
                Some(open).filter(|at| *at > now),
                Some(close).filter(|at| *at > now),
            ]
            .into_iter()
            .flatten()
            .min()
        }
        _ => None,
    }
}

fn map_window_result(result: &str) -> &'static str {
    match result {
        "missing" => "missing",
        "invalid" => "invalid",
        "timeout" => "timeout",
        "unavailable" => "unavailable",
        "ok" => "invalid",
        _ => "invalid",
    }
}

fn db_group_names(id: &str) -> &'static [&'static str] {
    match id {
        "owner-equity-v2-runner" => &["queue", "v2", "calendar"],
        "owner-beta-runner" => &["queue"],
        "candidate-runner" => &["queue", "candidate", "calendar"],
        "recommendation-runner" | "nt-backtest-worker-1" | "nt-backtest-worker-2" => &["queue"],
        "kis-eod" => &["etf", "research", "calendar", "rights"],
        "market-stream" => &["stream", "calendar"],
        "intraday-calendar" => &["calendar"],
        _ => &[],
    }
}

fn source_for_database(id: &str, db: &Value, now: DateTime<Utc>) -> Value {
    let names = db_group_names(id);
    if names.is_empty() {
        return Value::Null;
    }
    let mut results = Vec::new();
    let mut observed = Vec::new();
    let mut valid = Vec::new();
    for name in names {
        // The fixture corpus predates the separately executed rights query;
        // production always attaches its result as the eighth evidence group.
        if *name == "rights" && db.get("rights").is_none() {
            continue;
        }
        let Some(raw_group) = db.get(*name) else {
            results.push("unavailable");
            continue;
        };
        let group = if *name == "queue" {
            queue_group(raw_group, job_type_for_service(id))
        } else {
            raw_group
        };
        let explicit = group
            .get("result")
            .and_then(Value::as_str)
            .unwrap_or("unavailable");
        let group_observed = group.get("observed_at").and_then(optional_instant);
        let mut computed = db_source_result(explicit, group_observed, now);
        if *name == "stream" && explicit == "ok" {
            if let (Some(observed), Some(domain_until)) = (
                group_observed,
                group.get("valid_until").and_then(optional_instant),
            ) {
                computed.2 =
                    Some(domain_until.min(observed + ChronoDuration::seconds(DB_TTL_SECONDS)));
            }
        }
        if *name == "calendar"
            && explicit == "ok"
            && matches!(id, "market-stream" | "intraday-calendar")
        {
            computed.2 = min_time(computed.2, calendar_observation_until(group));
        }
        results.push(computed.0);
        if let Some(time) = computed
            .1
            .filter(|_| explicit == "ok" || (*name == "queue" && group.get("queued").is_some()))
        {
            observed.push(time);
        }
        if let Some(time) = computed.2 {
            valid.push(time);
        }
    }
    let all_ok = !results.is_empty() && results.iter().all(|result| *result == "ok");
    let any_source = results.iter().any(|result| *result == "ok");
    let result = if all_ok {
        "ok"
    } else if any_source {
        "partial"
    } else {
        results
            .iter()
            .copied()
            .max_by_key(|value| source_failure_priority(value))
            .unwrap_or("unavailable")
    };
    source_object(
        result,
        observed.into_iter().min(),
        if all_ok {
            valid.into_iter().min()
        } else {
            None
        },
    )
}

fn db_source_result(
    explicit: &str,
    observed: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> (&'static str, Option<DateTime<Utc>>, Option<DateTime<Utc>>) {
    if !matches!(explicit, "ok") {
        return (map_db_result(explicit), observed, None);
    }
    let Some(observed) = observed else {
        return ("invalid", None, None);
    };
    if observed > now {
        return ("future", Some(observed), None);
    }
    let valid = observed + ChronoDuration::seconds(DB_TTL_SECONDS);
    if now > valid {
        ("stale", Some(observed), Some(valid))
    } else {
        ("ok", Some(observed), Some(valid))
    }
}

fn map_db_result(result: &str) -> &'static str {
    match result {
        "ok" => "ok",
        "missing" => "missing",
        "timeout" => "timeout",
        "unavailable" => "unavailable",
        "denied" => "denied",
        "invalid" => "invalid",
        "future" => "future",
        "stale" => "stale",
        "limit" => "limit",
        "partial" => "partial",
        _ => "invalid",
    }
}

fn source_failure_priority(result: &str) -> u8 {
    match result {
        "future" | "invalid" | "denied" => 8,
        "timeout" => 7,
        "stale" => 6,
        "limit" => 5,
        "missing" => 4,
        "unavailable" => 3,
        _ => 1,
    }
}

fn source_fact(result: &str, scope: &str) -> Option<Fact> {
    if result == "ok" {
        return None;
    }
    let reason = match (scope, result) {
        (_, "timeout") => "DB_TIMEOUT",
        (_, "unavailable" | "missing") => "DB_UNAVAILABLE",
        (_, "invalid") => "SCHEMA_DRIFT",
        (_, "denied") => "DB_UNAVAILABLE",
        (_, "future") => "SOURCE_FUTURE",
        (_, "stale") => "DB_UNAVAILABLE",
        (_, "limit") => "QUERY_LIMIT",
        _ => "DB_UNAVAILABLE",
    };
    Some(Fact::new(Status::Unknown, reason))
}

fn host_source_fact(source: Option<&Value>, host_error: Option<&str>) -> Option<Fact> {
    let source = source?;
    let result = source
        .get("result")
        .and_then(Value::as_str)
        .unwrap_or("unavailable");
    if result == "ok" {
        return None;
    }
    let reason = match host_error {
        Some("POLICY_MISMATCH") => "POLICY_MISMATCH",
        Some("HOST_FUTURE") => "HOST_FUTURE",
        Some("HOST_MISSING") => "HOST_MISSING",
        Some("HOST_INVALID") => "HOST_INVALID",
        _ => match result {
            "missing" => "HOST_MISSING",
            "timeout" => "HOST_TIMEOUT",
            "unavailable" => "HOST_COMMAND_UNAVAILABLE",
            "denied" => "HOST_PERMISSION_DENIED",
            "future" => "HOST_FUTURE",
            "stale" => "HOST_STALE",
            _ => "HOST_INVALID",
        },
    };
    Some(Fact::new(Status::Unknown, reason))
}

fn host_process_fact(
    id: &str,
    expected_mode: &str,
    observation: Option<&Value>,
    source: Option<&Value>,
    now: DateTime<Utc>,
    host_error: Option<&str>,
) -> (&'static str, Option<Fact>, Option<DateTime<Utc>>) {
    let unknown = || {
        (
            "unknown",
            host_source_fact(source, host_error)
                .or_else(|| Some(Fact::new(Status::Unknown, "HOST_MISSING"))),
            None,
        )
    };
    let Some(observation) = observation else {
        return unknown();
    };
    let result = observation
        .get("result")
        .and_then(Value::as_str)
        .unwrap_or("invalid");
    if result != "ok" {
        return (
            "unknown",
            Some(Fact::new(Status::Unknown, host_reason(result))),
            None,
        );
    }
    let source_result = source
        .and_then(|value| value.get("result"))
        .and_then(Value::as_str)
        .unwrap_or("invalid");
    if source_result != "ok" {
        return ("unknown", host_source_fact(source, host_error), None);
    }
    let Some(process) = observation.get("process") else {
        return unknown();
    };
    let state = process
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let health = process
        .get("health")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let started = process.get("started_at").and_then(optional_instant);
    let valid_until = source
        .and_then(|value| value.get("valid_until"))
        .and_then(optional_instant);
    if expected_mode == "disabled" {
        if matches!(
            state,
            "running" | "active" | "created" | "restarting" | "paused" | "activating"
        ) {
            return (
                process_status_from_host(state),
                Some(Fact::new(Status::Failed, "UNEXPECTED_ACTIVE")),
                valid_until,
            );
        }
        return (
            "disabled",
            Some(Fact::new(Status::Disabled, "FEATURE_DISABLED")),
            valid_until,
        );
    }
    if id == "research-worker"
        && expected_mode == "daily"
        && matches!(state, "running" | "active" | "exited" | "inactive" | "dead")
    {
        return (
            if matches!(state, "running" | "active") {
                "running"
            } else {
                "inactive"
            },
            Some(Fact::new(Status::Waiting, "NOT_APPLICABLE")),
            valid_until,
        );
    }
    if matches!(state, "running" | "active") {
        if health == "unhealthy" {
            return (
                "running",
                Some(Fact::new(Status::Degraded, "PROCESS_UNHEALTHY")),
                valid_until,
            );
        }
        if health == "starting" {
            return process_start_fact(started, now, valid_until);
        }
        return (
            "running",
            Some(Fact::new(Status::Healthy, "PROCESS_RUNNING")),
            valid_until,
        );
    }
    if matches!(
        state,
        "created" | "restarting" | "paused" | "activating" | "deactivating"
    ) {
        return process_start_fact(started, now, valid_until);
    }
    if state == "missing" {
        return (
            "missing",
            Some(Fact::new(Status::Failed, "PROCESS_MISSING")),
            valid_until,
        );
    }
    if matches!(state, "failed" | "exited" | "dead" | "removing") {
        if matches!(
            id,
            "kis-eod" | "production-backup" | "production-backup-verify" | "tls-renewal"
        ) && expected_mode != "daemon"
        {
            return ("inactive", None, valid_until);
        }
        return (
            "failed",
            Some(Fact::new(Status::Failed, "PROCESS_FAILED")),
            valid_until,
        );
    }
    if state == "inactive" {
        if matches!(
            id,
            "kis-eod" | "production-backup" | "production-backup-verify" | "tls-renewal"
        ) {
            return ("inactive", None, valid_until);
        }
        if expected_mode == "daily" {
            return (
                "inactive",
                Some(Fact::new(Status::Waiting, "NOT_APPLICABLE")),
                valid_until,
            );
        }
        return (
            "inactive",
            Some(Fact::new(Status::Failed, "PROCESS_FAILED")),
            valid_until,
        );
    }
    (
        "unknown",
        Some(Fact::new(Status::Unknown, "HOST_INVALID")),
        valid_until,
    )
}

fn process_status_from_host(state: &str) -> &'static str {
    match state {
        "running" | "active" => "running",
        "created" | "restarting" | "paused" | "activating" | "deactivating" => "starting",
        "inactive" => "inactive",
        "failed" | "exited" | "dead" | "removing" => "failed",
        "missing" => "missing",
        _ => "unknown",
    }
}

fn process_start_fact(
    started: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    valid_until: Option<DateTime<Utc>>,
) -> (&'static str, Option<Fact>, Option<DateTime<Utc>>) {
    let Some(started) = started else {
        return (
            "unknown",
            Some(Fact::new(Status::Unknown, "PROCESS_STARTING")),
            valid_until,
        );
    };
    let bound = started + ChronoDuration::seconds(90);
    let status = if now <= bound {
        Status::Waiting
    } else {
        Status::Degraded
    };
    (
        "starting",
        Some(Fact::new(status, "PROCESS_STARTING")),
        min_time(valid_until, if now <= bound { Some(bound) } else { None }),
    )
}

#[derive(Debug, Clone)]
struct QueueView {
    work_status: &'static str,
    facts: Vec<Fact>,
    queued: Option<u64>,
    running: Option<u64>,
    recent_failed: Option<u64>,
    expired_claims: Option<u64>,
    last_success_at: Option<DateTime<Utc>>,
    last_failure_at: Option<DateTime<Utc>>,
    valid_until: Option<DateTime<Utc>>,
    has_failure: bool,
}

fn queue_view(
    group: &Value,
    job_type: &str,
    now: DateTime<Utc>,
    policy: Option<&HostPolicy>,
) -> QueueView {
    let group = queue_group(group, job_type);
    if group.get("queued").is_some() {
        return queue_summary_view(group, job_type, now, policy);
    }
    let result = group
        .get("result")
        .and_then(Value::as_str)
        .unwrap_or("unavailable");
    if result != "ok" {
        let fact = source_fact(result, "queue")
            .unwrap_or_else(|| Fact::new(Status::Unknown, "DB_UNAVAILABLE"));
        return QueueView {
            work_status: "unknown",
            facts: vec![fact],
            queued: None,
            running: None,
            recent_failed: None,
            expired_claims: None,
            last_success_at: None,
            last_failure_at: None,
            valid_until: None,
            has_failure: false,
        };
    }
    let observed = group.get("observed_at").and_then(optional_instant);
    let source = db_source_result(result, observed, now);
    if source.0 != "ok" {
        return QueueView {
            work_status: "unknown",
            facts: vec![
                source_fact(source.0, "queue")
                    .unwrap_or_else(|| Fact::new(Status::Unknown, "DB_UNAVAILABLE")),
            ],
            queued: None,
            running: None,
            recent_failed: None,
            expired_claims: None,
            last_success_at: None,
            last_failure_at: None,
            valid_until: source.2,
            has_failure: false,
        };
    }
    let rows = group
        .get("rows")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if rows
        .iter()
        .filter(|row| row.get("job_type").and_then(Value::as_str) == Some(job_type))
        .count()
        >= 10001
    {
        return QueueView {
            work_status: "unknown",
            facts: vec![Fact::new(Status::Degraded, "QUERY_LIMIT")],
            queued: None,
            running: None,
            recent_failed: None,
            expired_claims: None,
            last_success_at: None,
            last_failure_at: None,
            valid_until: source.2,
            has_failure: false,
        };
    }
    let lease = policy
        .and_then(|p| p.job_lease_seconds.get(job_type).copied())
        .or_else(|| {
            group
                .get("lease_seconds")
                .and_then(Value::as_u64)
                .map(|v| v as u32)
        })
        .unwrap_or(0);
    if lease == 0 || lease > 10001 {
        return QueueView {
            work_status: "unknown",
            facts: vec![Fact::new(Status::Unknown, "POLICY_MISMATCH")],
            queued: None,
            running: None,
            recent_failed: None,
            expired_claims: None,
            last_success_at: None,
            last_failure_at: None,
            valid_until: source.2,
            has_failure: false,
        };
    }
    let mut queued = 0u64;
    let mut running = 0u64;
    let mut recent_failed = 0u64;
    let mut expired_claims = 0u64;
    let mut eligible_queued = Vec::new();
    let mut future_queued = Vec::new();
    let mut active_claims = Vec::new();
    let mut latest_success = None;
    let mut latest_failure = None;
    let mut next_failure_expiry = None;
    let mut invalid = false;
    for row in rows
        .iter()
        .filter(|row| row.get("job_type").and_then(Value::as_str) == Some(job_type))
    {
        let status = row.get("status").and_then(Value::as_str).unwrap_or("");
        let available = row.get("available_at").and_then(optional_instant);
        let locked = row.get("locked_at").and_then(optional_instant);
        let finished = row.get("finished_at").and_then(optional_instant);
        if available.is_none()
            || (status == "RUNNING" && locked.is_none())
            || (matches!(status, "SUCCEEDED" | "FAILED") && finished.is_none())
            || row
                .get("locked_at")
                .is_some_and(|v| !v.is_null() && locked.is_none())
            || row
                .get("finished_at")
                .is_some_and(|v| !v.is_null() && finished.is_none())
            || finished.is_some_and(|v| v > now)
            || locked.is_some_and(|v| v > now)
            || !matches!(
                status,
                "QUEUED" | "RUNNING" | "SUCCEEDED" | "FAILED" | "CANCELED"
            )
        {
            invalid = true;
            continue;
        }
        match status {
            "QUEUED" => {
                queued += 1;
                if let Some(available) = available {
                    if available <= now {
                        eligible_queued.push(available);
                    } else {
                        future_queued.push(available);
                    }
                }
            }
            "RUNNING" => {
                running += 1;
                if let Some(locked) = locked {
                    active_claims.push(locked);
                }
                if locked
                    .is_some_and(|locked| locked + ChronoDuration::seconds(lease as i64) <= now)
                {
                    expired_claims += 1;
                }
            }
            "SUCCEEDED" => latest_success = max_time(latest_success, finished),
            "FAILED" => {
                latest_failure = max_time(latest_failure, finished);
                if finished.is_some_and(|finished| {
                    now - finished <= ChronoDuration::seconds(RECENT_FAILURE_SECONDS)
                }) {
                    recent_failed += 1;
                    next_failure_expiry = min_time(
                        next_failure_expiry,
                        finished
                            .map(|value| value + ChronoDuration::seconds(RECENT_FAILURE_SECONDS)),
                    );
                }
            }
            _ => {}
        }
    }
    if invalid {
        return QueueView {
            work_status: "unknown",
            facts: vec![Fact::new(Status::Unknown, "DB_UNAVAILABLE")],
            queued: None,
            running: None,
            recent_failed: None,
            expired_claims: None,
            last_success_at: None,
            last_failure_at: None,
            valid_until: source.2,
            has_failure: false,
        };
    }
    queue_outcome(
        now,
        queued,
        running,
        recent_failed,
        expired_claims,
        eligible_queued.into_iter().min(),
        future_queued.into_iter().min(),
        active_claims
            .into_iter()
            .map(|locked| locked + ChronoDuration::seconds(lease as i64))
            .filter(|deadline| *deadline > now)
            .min(),
        next_failure_expiry,
        latest_success,
        latest_failure,
        source.2,
    )
}

fn job_type_for_service(id: &str) -> &str {
    match id {
        "owner-equity-v2-runner" => "owner_equity_v2",
        "owner-beta-runner" => "owner_beta_price_recommendation",
        "candidate-runner" => "candidate_compute",
        "recommendation-runner" => "recommendation",
        "nt-backtest-worker-1" | "nt-backtest-worker-2" => "backtest",
        _ => "",
    }
}

fn queue_group<'a>(group: &'a Value, job_type: &str) -> &'a Value {
    match group.get("groups") {
        Some(groups) => groups.get(job_type).unwrap_or(&Value::Null),
        None => group,
    }
}

fn queue_summary_requires_refresh(input: &Value, now: DateTime<Utc>) -> bool {
    input
        .pointer("/db_read_results/queue/groups")
        .and_then(Value::as_object)
        .is_some_and(|groups| {
            groups.values().any(|group| {
                group
                    .get("recheck_at")
                    .and_then(optional_instant)
                    .is_some_and(|boundary| now >= boundary)
            })
        })
}

impl QueueView {
    fn unavailable(fact: Fact, valid_until: Option<DateTime<Utc>>) -> Self {
        Self {
            work_status: "unknown",
            facts: vec![fact],
            queued: None,
            running: None,
            recent_failed: None,
            expired_claims: None,
            last_success_at: None,
            last_failure_at: None,
            valid_until,
            has_failure: false,
        }
    }
}

fn queue_summary_view(
    group: &Value,
    job_type: &str,
    now: DateTime<Utc>,
    policy: Option<&HostPolicy>,
) -> QueueView {
    let result = group
        .get("result")
        .and_then(Value::as_str)
        .unwrap_or("invalid");
    if result == "limit" {
        let source = db_source_result(
            "ok",
            group.get("observed_at").and_then(optional_instant),
            now,
        );
        if source.0 != "ok" {
            return QueueView::unavailable(
                source_fact(source.0, "queue")
                    .unwrap_or_else(|| Fact::new(Status::Unknown, "DB_UNAVAILABLE")),
                source.2,
            );
        }
        return QueueView::unavailable(Fact::new(Status::Degraded, "QUERY_LIMIT"), source.2);
    }
    let source = db_source_result(
        result,
        group.get("observed_at").and_then(optional_instant),
        now,
    );
    if source.0 != "ok" {
        return QueueView::unavailable(
            source_fact(source.0, "queue")
                .unwrap_or_else(|| Fact::new(Status::Unknown, "DB_UNAVAILABLE")),
            source.2,
        );
    }
    let lease = policy
        .and_then(|policy| policy.job_lease_seconds.get(job_type).copied())
        .map(u64::from)
        .or_else(|| group.get("lease_seconds").and_then(Value::as_u64));
    if lease.is_none_or(|seconds| !(1..=3600).contains(&seconds)) {
        return QueueView::unavailable(Fact::new(Status::Unknown, "POLICY_MISMATCH"), source.2);
    }
    let observed = group.get("observed_at").and_then(optional_instant);
    let recheck = group.get("recheck_at").and_then(optional_instant);
    // Counts are exact at the SQL clock, including already-expired claims.
    // Serving refreshes before a later count transition instead of guessing a
    // complete count from one minimum or caching thousands of raw job objects.
    if recheck.is_some_and(|boundary| now > boundary && observed.is_some_and(|at| now > at)) {
        return QueueView::unavailable(Fact::new(Status::Unknown, "DB_UNAVAILABLE"), recheck);
    }
    let counts = [
        "queued",
        "running",
        "recent_failed",
        "expired_claims",
        "eligible_queued",
    ]
    .map(|name| group.get(name).and_then(Value::as_u64));
    let [
        Some(queued),
        Some(running),
        Some(recent_failed),
        Some(expired_claims),
        Some(eligible),
    ] = counts
    else {
        return QueueView::unavailable(Fact::new(Status::Unknown, "DB_UNAVAILABLE"), source.2);
    };
    if [queued, running, recent_failed, expired_claims, eligible]
        .iter()
        .any(|count| *count > 10000)
        || expired_claims > running
        || eligible > queued
        || group.get("invalid").and_then(Value::as_bool) != Some(false)
        || ["last_success_at", "last_failure_at"].iter().any(|name| {
            group
                .get(*name)
                .and_then(optional_instant)
                .is_some_and(|time| time > now)
        })
    {
        return QueueView::unavailable(Fact::new(Status::Unknown, "SCHEMA_DRIFT"), source.2);
    }
    queue_outcome(
        now,
        queued,
        running,
        recent_failed,
        expired_claims,
        group.get("oldest_eligible_at").and_then(optional_instant),
        group.get("next_available_at").and_then(optional_instant),
        group.get("next_claim_expiry_at").and_then(optional_instant),
        group
            .get("next_failure_expiry_at")
            .and_then(optional_instant),
        group.get("last_success_at").and_then(optional_instant),
        group.get("last_failure_at").and_then(optional_instant),
        source.2,
    )
}

#[allow(clippy::too_many_arguments)]
fn queue_outcome(
    now: DateTime<Utc>,
    queued: u64,
    running: u64,
    recent_failed: u64,
    expired_claims: u64,
    oldest_eligible: Option<DateTime<Utc>>,
    next_available: Option<DateTime<Utc>>,
    next_claim_expiry: Option<DateTime<Utc>>,
    next_failure_expiry: Option<DateTime<Utc>>,
    latest_success: Option<DateTime<Utc>>,
    latest_failure: Option<DateTime<Utc>>,
    source_until: Option<DateTime<Utc>>,
) -> QueueView {
    let mut facts = Vec::new();
    let mut until = [
        source_until,
        next_available,
        next_claim_expiry,
        next_failure_expiry,
    ]
    .into_iter()
    .flatten()
    .min();
    let mut work = "idle";
    if expired_claims > 0 {
        work = "failed";
        facts.push(Fact::new(Status::Failed, "CLAIM_EXPIRED"));
    }
    if recent_failed > 0 {
        work = if expired_claims > 0 {
            "failed"
        } else {
            "degraded"
        };
        facts.push(Fact::new(Status::Degraded, "RECENT_JOB_FAILURE"));
    }
    if expired_claims == 0 {
        if running > 0 {
            work = "active";
            facts.push(Fact::new(Status::Healthy, "QUEUE_ACTIVE"));
        } else if let Some(oldest) = oldest_eligible {
            work = "pending";
            let boundary = oldest + ChronoDuration::seconds(120);
            let overdue = now > boundary;
            if !overdue {
                until = min_time(until, Some(boundary));
            }
            facts.push(Fact::new(
                if overdue {
                    Status::Degraded
                } else {
                    Status::Waiting
                },
                if overdue {
                    "QUEUE_DELAYED"
                } else {
                    "QUEUE_PENDING"
                },
            ));
        } else if next_available.is_some() {
            work = "scheduled";
            facts.push(Fact::new(Status::Waiting, "QUEUE_PENDING"));
        } else if facts.is_empty() {
            facts.push(Fact::new(Status::Waiting, "QUEUE_IDLE"));
        }
    }
    QueueView {
        work_status: work,
        facts,
        queued: Some(queued),
        running: Some(running),
        recent_failed: Some(recent_failed),
        expired_claims: Some(expired_claims),
        last_success_at: latest_success,
        last_failure_at: latest_failure,
        valid_until: until,
        has_failure: expired_claims > 0 || recent_failed > 0,
    }
}

fn max_time(current: Option<DateTime<Utc>>, next: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
    match (current, next) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (None, value) | (value, None) => value,
    }
}

fn put_queue_metrics(metrics: &mut Value, queue: &QueueView, scope: &str) {
    metrics["queued"] = queue
        .queued
        .map(|value| json!(value))
        .unwrap_or(Value::Null);
    metrics["running"] = queue
        .running
        .map(|value| json!(value))
        .unwrap_or(Value::Null);
    metrics["recent_failed"] = queue
        .recent_failed
        .map(|value| json!(value))
        .unwrap_or(Value::Null);
    metrics["expired_claims"] = queue
        .expired_claims
        .map(|value| json!(value))
        .unwrap_or(Value::Null);
    metrics["last_success_at"] = queue
        .last_success_at
        .map(|value| json!(fmt_instant(value)))
        .unwrap_or(Value::Null);
    metrics["last_failure_at"] = queue
        .last_failure_at
        .map(|value| json!(fmt_instant(value)))
        .unwrap_or(Value::Null);
    metrics["queue_scope"] = json!(scope);
}

fn parse_date(value: &Value) -> Option<NaiveDate> {
    let text = value.as_str()?;
    if text.len() != 10 {
        return None;
    }
    NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()
}

fn fmt_date(value: NaiveDate) -> String {
    value.format("%Y-%m-%d").to_string()
}

fn kst_date(now: DateTime<Utc>) -> NaiveDate {
    now.with_timezone(&FixedOffset::east_opt(KST_SECONDS).expect("valid KST offset"))
        .date_naive()
}

fn kst_time(now: DateTime<Utc>) -> NaiveTime {
    now.with_timezone(&FixedOffset::east_opt(KST_SECONDS).expect("valid KST offset"))
        .time()
}

#[derive(Debug, Clone)]
struct CalendarView {
    current_status: &'static str,
    expected: Option<NaiveDate>,
    due_at: Option<DateTime<Utc>>,
    reason: Option<Fact>,
    current_valid_until: Option<DateTime<Utc>>,
}

// Preserve bounds of the originally accepted current-day proof on a cache
// hit; a day/freshness transition does not manufacture a fresh DB observation.
fn calendar_observation_until(group: &Value) -> Option<DateTime<Utc>> {
    let observed = group.get("observed_at").and_then(optional_instant)?;
    let ttl = observed + ChronoDuration::seconds(DB_TTL_SECONDS);
    let today = kst_date(observed);
    let rows = group.get("rows").and_then(Value::as_array)?;
    let current = rows
        .iter()
        .filter(|row| row.get("session_date").and_then(parse_date) == Some(today))
        .collect::<Vec<_>>();
    if current.len() != 1 || !calendar_lineage_valid(current[0]) {
        return Some(ttl);
    }
    let retrievals = [
        "calendar_retrieved_at",
        "version_retrieved_at",
        "raw_retrieved_at",
    ]
    .map(|name| current[0].get(name).and_then(optional_instant));
    if retrievals
        .iter()
        .any(|at| at.is_none_or(|at| at > observed || observed - at > ChronoDuration::hours(36)))
    {
        return Some(ttl);
    }
    let kst = FixedOffset::east_opt(KST_SECONDS)?;
    let midnight = today
        .succ_opt()
        .and_then(|date| {
            kst.from_local_datetime(&date.and_time(NaiveTime::MIN))
                .single()
        })
        .map(|at| at.with_timezone(&Utc));
    [
        Some(ttl),
        midnight,
        retrievals
            .into_iter()
            .flatten()
            .map(|at| at + ChronoDuration::hours(36))
            .min(),
    ]
    .into_iter()
    .flatten()
    .min()
}

fn calendar_view(group: &Value, now: DateTime<Utc>) -> CalendarView {
    let source = db_source_result(
        group
            .get("result")
            .and_then(Value::as_str)
            .unwrap_or("unavailable"),
        group.get("observed_at").and_then(optional_instant),
        now,
    );
    let failure = |status: &'static str, fact: Fact, until: Option<DateTime<Utc>>| CalendarView {
        current_status: status,
        expected: None,
        due_at: None,
        reason: Some(fact),
        current_valid_until: until,
    };
    if source.0 != "ok" {
        return failure(
            "unknown",
            source_fact(source.0, "calendar")
                .unwrap_or_else(|| Fact::new(Status::Unknown, "DB_UNAVAILABLE")),
            source.2,
        );
    }
    let observed = group
        .get("observed_at")
        .and_then(optional_instant)
        .expect("source clock validated");
    let rows = group.get("rows").and_then(Value::as_array);
    let Some(rows) = rows else {
        return failure(
            "invalid",
            Fact::new(Status::Failed, "CALENDAR_LINEAGE_INVALID"),
            source.2,
        );
    };
    if rows.len() > 370 {
        return failure(
            "unknown",
            Fact::new(Status::Degraded, "QUERY_LIMIT"),
            source.2,
        );
    }
    let today = kst_date(now);
    let current = rows
        .iter()
        .filter(|row| row.get("session_date").and_then(parse_date) == Some(today))
        .collect::<Vec<_>>();
    let bound = calendar_observation_until(group);
    if current.is_empty() {
        return failure(
            "missing",
            Fact::new(Status::Failed, "CALENDAR_UNAVAILABLE"),
            bound,
        );
    }
    if current.len() != 1 || !calendar_lineage_valid(current[0]) {
        return failure(
            "invalid",
            Fact::new(Status::Failed, "CALENDAR_LINEAGE_INVALID"),
            source.2,
        );
    }
    let row = current[0];
    for field in [
        "calendar_retrieved_at",
        "version_retrieved_at",
        "raw_retrieved_at",
    ] {
        let Some(at) = row.get(field).and_then(optional_instant) else {
            return failure(
                "invalid",
                Fact::new(Status::Failed, "CALENDAR_LINEAGE_INVALID"),
                source.2,
            );
        };
        if at > observed || at > now {
            return failure(
                "invalid",
                Fact::new(Status::Failed, "SOURCE_FUTURE"),
                source.2,
            );
        }
        if now - at > ChronoDuration::hours(36) {
            return failure("stale", Fact::new(Status::Failed, "CALENDAR_STALE"), bound);
        }
    }
    CalendarView {
        current_status: if row.get("session_type").and_then(Value::as_str) == Some("CLOSED") {
            "closed"
        } else {
            "ready"
        },
        expected: None,
        due_at: None,
        reason: None,
        current_valid_until: bound,
    }
}

/// EOD eligibility needs verified historical lineage through the eligible
/// civil date; it does not borrow the current-day intraday 36-hour assurance.
fn calendar_eod_view(group: &Value, now: DateTime<Utc>) -> CalendarView {
    let observed = group.get("observed_at").and_then(optional_instant);
    let source = db_source_result(
        group
            .get("result")
            .and_then(Value::as_str)
            .unwrap_or("unavailable"),
        observed,
        now,
    );
    let failure = |fact: Fact, until: Option<DateTime<Utc>>| CalendarView {
        current_status: "unknown",
        expected: None,
        due_at: None,
        reason: Some(fact),
        current_valid_until: until,
    };
    if source.0 != "ok" {
        return failure(
            source_fact(source.0, "calendar")
                .unwrap_or_else(|| Fact::new(Status::Unknown, "DB_UNAVAILABLE")),
            source.2,
        );
    }
    let observed = observed.expect("successful DB source has an observation");
    let rows = group.get("rows").and_then(Value::as_array);
    let Some(rows) = rows else {
        return failure(Fact::new(Status::Unknown, "SCHEMA_DRIFT"), source.2);
    };
    if rows.len() > 370 {
        return failure(Fact::new(Status::Degraded, "QUERY_LIMIT"), source.2);
    }
    let today = kst_date(now);
    let eligible = if kst_time(now) < NaiveTime::from_hms_opt(16, 30, 0).expect("time") {
        today.pred_opt().unwrap_or(today)
    } else {
        today
    };
    let mut days = BTreeMap::new();
    for row in rows {
        let Some(date) = row.get("session_date").and_then(parse_date) else {
            return failure(
                Fact::new(Status::Failed, "CALENDAR_LINEAGE_INVALID"),
                source.2,
            );
        };
        if date > eligible {
            continue;
        }
        if !calendar_lineage_valid(row) || days.insert(date, row).is_some() {
            return failure(
                Fact::new(Status::Failed, "CALENDAR_LINEAGE_INVALID"),
                source.2,
            );
        }
        for field in [
            "calendar_retrieved_at",
            "version_retrieved_at",
            "raw_retrieved_at",
        ] {
            let Some(at) = row.get(field).and_then(optional_instant) else {
                return failure(
                    Fact::new(Status::Failed, "CALENDAR_LINEAGE_INVALID"),
                    source.2,
                );
            };
            if at > observed || at > now {
                return failure(Fact::new(Status::Failed, "SOURCE_FUTURE"), source.2);
            }
        }
    }
    let expected = days.iter().rev().find_map(|(date, row)| {
        if row.get("session_type").and_then(Value::as_str) == Some("TRADING") {
            Some(*date)
        } else {
            None
        }
    });
    let Some(expected) = expected else {
        return failure(Fact::new(Status::Failed, "CALENDAR_UNAVAILABLE"), source.2);
    };
    let mut date = expected;
    while date <= eligible {
        if !days.contains_key(&date) {
            return failure(Fact::new(Status::Failed, "CALENDAR_COVERAGE_GAP"), source.2);
        }
        let Some(next) = date.succ_opt() else {
            break;
        };
        date = next;
    }
    let kst = FixedOffset::east_opt(KST_SECONDS).expect("KST");
    let due = kst
        .from_local_datetime(&expected.and_time(NaiveTime::from_hms_opt(18, 30, 0).expect("time")))
        .single()
        .map(|at| at.with_timezone(&Utc));
    let midnight = kst_date(observed)
        .succ_opt()
        .and_then(|date| {
            kst.from_local_datetime(&date.and_time(NaiveTime::MIN))
                .single()
        })
        .map(|at| at.with_timezone(&Utc));
    CalendarView {
        current_status: "ready",
        expected: Some(expected),
        due_at: due,
        reason: None,
        current_valid_until: min_time(source.2, midnight),
    }
}

fn calendar_lineage_valid(row: &Value) -> bool {
    let Some(batch) = row.get("calendar_batch").and_then(Value::as_str) else {
        return false;
    };
    let Some(hash) = row.get("calendar_hash").and_then(Value::as_str) else {
        return false;
    };
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return false;
    }
    row.get("exchange").and_then(Value::as_str) == Some("KRX")
        && row.get("timezone").and_then(Value::as_str) == Some("Asia/Seoul")
        && row.get("source").and_then(Value::as_str) == Some("kis")
        && row.get("source_version").and_then(Value::as_str) == Some("kis-chk-holiday-v1:schema-1")
        && row.get("version_batch").and_then(Value::as_str) == Some(batch)
        && row.get("raw_batch").and_then(Value::as_str) == Some(batch)
        && row.get("version_hash").and_then(Value::as_str) == Some(hash)
        && row.get("raw_hash").and_then(Value::as_str) == Some(hash)
        && row.get("raw_kind").and_then(Value::as_str) == Some("CALENDAR")
        && row.get("raw_file").and_then(Value::as_str) == Some("calendar.json")
        && row.get("raw_fetch_mode").and_then(Value::as_str) == Some("credentialed")
        && matches!(
            row.get("session_type").and_then(Value::as_str),
            Some("TRADING" | "CLOSED")
        )
}

fn window_negative_fact(window: &Value, now: DateTime<Utc>) -> Option<Fact> {
    if window.get("result").and_then(Value::as_str) == Some("ok")
        && window.pointer("/entry/date").and_then(parse_date) != Some(kst_date(now))
    {
        return Some(Fact::new(Status::Failed, "SESSION_WINDOW_UNAVAILABLE"));
    }
    let source = source_for_window(window, now);
    match source["result"].as_str().unwrap_or("invalid") {
        "ok" => None,
        "missing" | "unavailable" => Some(Fact::new(Status::Failed, "SESSION_WINDOW_UNAVAILABLE")),
        "timeout" => Some(Fact::new(Status::Unknown, "DB_TIMEOUT")),
        "stale" => Some(Fact::new(Status::Unknown, "DB_UNAVAILABLE")),
        _ => Some(Fact::new(Status::Failed, "SESSION_WINDOW_INVALID")),
    }
}

fn calendar_window_fact(
    calendar: &Value,
    window: &Value,
    now: DateTime<Utc>,
) -> (
    Option<Fact>,
    &'static str,
    &'static str,
    Option<DateTime<Utc>>,
) {
    let calendar_view = calendar_view(calendar, now);
    if let Some(reason) = calendar_view.reason.clone() {
        return (
            Some(reason),
            calendar_view.current_status,
            window_status(window, now),
            calendar_view.current_valid_until,
        );
    }
    let Some(entry) = window.get("entry") else {
        return (
            Some(Fact::new(Status::Failed, "SESSION_WINDOW_UNAVAILABLE")),
            calendar_view.current_status,
            "missing",
            calendar_view.current_valid_until,
        );
    };
    let result = window
        .get("result")
        .and_then(Value::as_str)
        .unwrap_or("unavailable");
    if result != "ok" {
        let (status, reason, window_status) = match result {
            "missing" | "unavailable" => (Status::Failed, "SESSION_WINDOW_UNAVAILABLE", "missing"),
            "timeout" => (Status::Unknown, "DB_TIMEOUT", "unknown"),
            _ => (Status::Failed, "SESSION_WINDOW_INVALID", "invalid"),
        };
        return (
            Some(Fact::new(status, reason)),
            calendar_view.current_status,
            window_status,
            calendar_view.current_valid_until,
        );
    }
    if window
        .get("activation_hash_matches")
        .and_then(Value::as_bool)
        != Some(true)
        || window.get("metadata_safe").and_then(Value::as_bool) != Some(true)
    {
        return (
            Some(Fact::new(Status::Failed, "SESSION_WINDOW_INVALID")),
            calendar_view.current_status,
            "invalid",
            calendar_view.current_valid_until,
        );
    }
    let today = kst_date(now);
    if parse_date(&entry["date"]) != Some(today)
        || entry
            .get("evidence_retrieved_at")
            .and_then(optional_instant)
            .is_none()
    {
        return (
            Some(Fact::new(Status::Failed, "SESSION_WINDOW_UNAVAILABLE")),
            calendar_view.current_status,
            "missing",
            calendar_view.current_valid_until,
        );
    }
    let evidence = entry
        .get("evidence_retrieved_at")
        .and_then(optional_instant)
        .unwrap();
    if evidence > now
        || evidence
            .with_timezone(&FixedOffset::east_opt(KST_SECONDS).unwrap())
            .date_naive()
            != today
    {
        return (
            Some(Fact::new(Status::Failed, "SESSION_WINDOW_INVALID")),
            calendar_view.current_status,
            "invalid",
            calendar_view.current_valid_until,
        );
    }
    let disposition = entry
        .get("disposition")
        .and_then(Value::as_str)
        .unwrap_or("");
    let mut semantic = calendar_view.current_valid_until;
    let result = match disposition {
        "closed" if entry["open_at"].is_null() && entry["close_at"].is_null() => (None, "closed"),
        "trading" => {
            let Some(open) = entry.get("open_at").and_then(optional_instant) else {
                return (
                    Some(Fact::new(Status::Failed, "SESSION_WINDOW_INVALID")),
                    calendar_view.current_status,
                    "invalid",
                    semantic,
                );
            };
            let Some(close) = entry.get("close_at").and_then(optional_instant) else {
                return (
                    Some(Fact::new(Status::Failed, "SESSION_WINDOW_INVALID")),
                    calendar_view.current_status,
                    "invalid",
                    semantic,
                );
            };
            if open >= close
                || open
                    .with_timezone(&FixedOffset::east_opt(KST_SECONDS).unwrap())
                    .date_naive()
                    != today
                || close
                    .with_timezone(&FixedOffset::east_opt(KST_SECONDS).unwrap())
                    .date_naive()
                    != today
            {
                return (
                    Some(Fact::new(Status::Failed, "SESSION_WINDOW_INVALID")),
                    calendar_view.current_status,
                    "invalid",
                    semantic,
                );
            }
            if now < open {
                semantic = min_time(semantic, Some(open));
                (
                    Some(Fact::new(Status::Waiting, "SESSION_PREOPEN")),
                    "closed",
                )
            } else if now >= close {
                (Some(Fact::new(Status::Waiting, "SESSION_CLOSED")), "closed")
            } else {
                semantic = min_time(semantic, Some(close));
                (None, "ready")
            }
        }
        _ => {
            return (
                Some(Fact::new(Status::Failed, "SESSION_WINDOW_INVALID")),
                calendar_view.current_status,
                "invalid",
                semantic,
            );
        }
    };
    let evidence_until = evidence + ChronoDuration::hours(36);
    semantic = min_time(semantic, Some(evidence_until));
    semantic = min_time(
        semantic,
        source_for_window(window, now)
            .get("valid_until")
            .and_then(optional_instant),
    );
    (result.0, calendar_view.current_status, result.1, semantic)
}

fn window_status(window: &Value, now: DateTime<Utc>) -> &'static str {
    match window.get("result").and_then(Value::as_str) {
        Some("missing" | "unavailable") => "missing",
        Some("invalid") => "invalid",
        Some("timeout") => "unknown",
        Some("ok") => {
            let entry = &window["entry"];
            if parse_date(&entry["date"]) != Some(kst_date(now)) {
                "missing"
            } else if entry
                .get("evidence_retrieved_at")
                .and_then(optional_instant)
                .is_some_and(|time| time <= now)
            {
                if entry.get("disposition").and_then(Value::as_str) == Some("closed") {
                    "closed"
                } else {
                    "ready"
                }
            } else {
                "invalid"
            }
        }
        _ => "unknown",
    }
}

fn calendar_metric_values(
    metrics: &mut Value,
    calendar: &Value,
    _window: &Value,
    now: DateTime<Utc>,
) -> CalendarView {
    let view = calendar_eod_view(calendar, now);
    metrics["expected_session"] = view
        .expected
        .map(|value| json!(fmt_date(value)))
        .unwrap_or(Value::Null);
    metrics["data_due_at"] = view
        .due_at
        .map(|value| json!(fmt_instant(value)))
        .unwrap_or(Value::Null);
    view
}

fn v2_fact(
    group: &Value,
    calendar: &Value,
    now: DateTime<Utc>,
    metrics: &mut Value,
) -> Option<Fact> {
    let result = group
        .get("result")
        .and_then(Value::as_str)
        .unwrap_or("unavailable");
    if result != "ok" {
        return source_fact(result, "v2");
    }
    let observed = group.get("observed_at").and_then(optional_instant);
    let src = db_source_result(result, observed, now);
    if src.0 != "ok" {
        return source_fact(src.0, "v2");
    }
    let required = group.get("required_count").and_then(Value::as_u64);
    let covered = group.get("covered_count").and_then(Value::as_u64);
    metrics["last_success_at"] = group
        .get("snapshot_published_at")
        .cloned()
        .unwrap_or(Value::Null);
    metrics["required_identities"] = required.map(|v| json!(v)).unwrap_or(Value::Null);
    metrics["covered_identities"] = covered.map(|v| json!(v)).unwrap_or(Value::Null);
    metrics["v2_snapshot_as_of"] = group.get("snapshot_as_of").cloned().unwrap_or(Value::Null);
    if required.is_none() || covered.is_none() {
        return Some(Fact::new(Status::Unknown, "DB_UNAVAILABLE"));
    }
    if required.unwrap_or(0) > 100 || result == "limit" {
        return Some(Fact::new(Status::Unknown, "QUERY_LIMIT"));
    }
    if required != covered
        || group
            .get("current_admissions_match")
            .and_then(Value::as_bool)
            != Some(true)
    {
        metrics["v2_data_as_of"] = Value::Null;
        return Some(Fact::new(Status::Degraded, "V2_COVERAGE_INCOMPLETE"));
    }
    metrics["v2_data_as_of"] = group
        .get("generation_last_session")
        .cloned()
        .unwrap_or(Value::Null);
    let expected = metrics.get("expected_session").and_then(parse_date);
    let latest = group.get("generation_last_session").and_then(parse_date);
    if expected.is_some() && latest != expected {
        return Some(eod_lag_fact(metrics, calendar, latest, now, "EOD_STALE"));
    }
    let snapshot_as_of = group.get("snapshot_as_of").and_then(parse_date);
    if snapshot_as_of.is_some() && expected.is_some() && snapshot_as_of != expected {
        return Some(eod_lag_fact(
            metrics,
            calendar,
            snapshot_as_of,
            now,
            "SNAPSHOT_STALE",
        ));
    }
    if required.unwrap_or(0) > 0
        && (snapshot_as_of.is_none()
            || group
                .get("snapshot_published_at")
                .and_then(optional_instant)
                .is_none())
    {
        return Some(eod_lag_fact(
            metrics,
            calendar,
            snapshot_as_of,
            now,
            "SNAPSHOT_STALE",
        ));
    }
    Some(Fact::new(Status::Healthy, "DATA_CURRENT"))
}

fn v2_semantic_until(group: &Value, _now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    // Snapshot publication age is not a freshness contract; v2_fact validates
    // the session and exact coverage. The read evidence itself keeps its
    // bounded observation TTL.
    group
        .get("observed_at")
        .and_then(optional_instant)
        .map(|value| value + ChronoDuration::seconds(DB_TTL_SECONDS))
}

fn candidate_fact(
    group: &Value,
    calendar: &Value,
    now: DateTime<Utc>,
    metrics: &mut Value,
) -> Option<Fact> {
    let result = group
        .get("result")
        .and_then(Value::as_str)
        .unwrap_or("unavailable");
    if result != "ok" {
        return source_fact(result, "candidate");
    }
    let source = db_source_result(
        result,
        group.get("observed_at").and_then(optional_instant),
        now,
    );
    if source.0 != "ok" {
        return source_fact(source.0, "candidate");
    }
    metrics["data_as_of"] = group.get("as_of").cloned().unwrap_or(Value::Null);
    metrics["last_success_at"] = group.get("published_at").cloned().unwrap_or(Value::Null);
    if group.get("enabled").and_then(Value::as_bool) != Some(true) {
        return Some(Fact::new(Status::Degraded, "SCHEDULER_DISABLED"));
    }
    let required = group.get("required_universes").and_then(Value::as_u64);
    let published = group.get("published_universes").and_then(Value::as_u64);
    if required.is_none() || published.is_none() {
        return Some(Fact::new(Status::Unknown, "DB_UNAVAILABLE"));
    }
    if required.unwrap_or(0) > 16 || published.unwrap_or(0) > 16 {
        return Some(Fact::new(Status::Unknown, "QUERY_LIMIT"));
    }
    if required != published {
        return Some(Fact::new(Status::Degraded, "V2_COVERAGE_INCOMPLETE"));
    }
    if let Some(expected) = metrics.get("expected_session").and_then(parse_date)
        && group.get("as_of").and_then(parse_date) != Some(expected)
    {
        metrics["data_due_at"] = calendar_eod_view(calendar, now)
            .due_at
            .map(fmt_instant)
            .map(Value::String)
            .unwrap_or(Value::Null);
        return Some(eod_lag_fact(
            metrics,
            calendar,
            group.get("as_of").and_then(parse_date),
            now,
            "EOD_STALE",
        ));
    }
    Some(Fact::new(Status::Healthy, "DATA_CURRENT"))
}

fn candidate_semantic_until(group: &Value, _now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    group
        .get("observed_at")
        .and_then(optional_instant)
        .map(|time| time + ChronoDuration::seconds(DB_TTL_SECONDS))
}

fn eod_transition_until(now: DateTime<Utc>, metrics: &Value) -> Option<DateTime<Utc>> {
    metrics.get("expected_session").and_then(parse_date)?;
    let kst = FixedOffset::east_opt(KST_SECONDS)?;
    let cutoff = kst
        .from_local_datetime(&kst_date(now).and_time(NaiveTime::from_hms_opt(16, 30, 0)?))
        .single()?
        .with_timezone(&Utc);
    [
        Some(cutoff).filter(|at| *at > now),
        metrics
            .get("data_due_at")
            .and_then(optional_instant)
            .filter(|at| *at > now),
    ]
    .into_iter()
    .flatten()
    .min()
}

fn eod_lag_fact(
    metrics: &mut Value,
    calendar: &Value,
    published: Option<NaiveDate>,
    now: DateTime<Utc>,
    overdue_reason: &str,
) -> Fact {
    let expected = metrics.get("expected_session").and_then(parse_date);
    let first_missing = published.and_then(|published| {
        calendar
            .get("rows")
            .and_then(Value::as_array)
            .and_then(|rows| {
                rows.iter()
                    .filter(|row| {
                        row.get("session_type").and_then(Value::as_str) == Some("TRADING")
                    })
                    .filter_map(|row| row.get("session_date").and_then(parse_date))
                    .filter(|date| {
                        *date > published && expected.is_some_and(|expected| *date <= expected)
                    })
                    .min()
            })
    });
    if let Some(date) = first_missing {
        let kst = FixedOffset::east_opt(KST_SECONDS).expect("KST");
        let due = kst
            .from_local_datetime(
                &date.and_time(NaiveTime::from_hms_opt(18, 30, 0).expect("valid time")),
            )
            .single()
            .map(|at| at.with_timezone(&Utc));
        metrics["data_due_at"] = due
            .map(fmt_instant)
            .map(Value::String)
            .unwrap_or(Value::Null);
    }
    let pending = metrics
        .get("data_due_at")
        .and_then(optional_instant)
        .is_some_and(|due| now < due);
    Fact::new(
        if pending {
            Status::Waiting
        } else {
            Status::Degraded
        },
        if pending {
            "EOD_PENDING"
        } else {
            overdue_reason
        },
    )
}

fn research_negative_fact(group: &Value, now: DateTime<Utc>, metrics: &mut Value) -> Option<Fact> {
    if db_source_result(
        group
            .get("result")
            .and_then(Value::as_str)
            .unwrap_or("unavailable"),
        group.get("observed_at").and_then(optional_instant),
        now,
    )
    .0 != "ok"
    {
        return None;
    }
    let row = group.get("row").filter(|row| row.is_object())?;
    let observed = group.get("observed_at").and_then(optional_instant)?;
    if [
        "attempt_started_at",
        "heartbeat_at",
        "attempt_finished_at",
        "last_success_at",
        "last_failure_at",
    ]
    .iter()
    .any(|name| {
        row.get(*name)
            .and_then(optional_instant)
            .is_some_and(|at| at > observed)
    }) {
        return Some(Fact::new(Status::Unknown, "SOURCE_FUTURE"));
    }
    for (metric, source) in [
        ("last_attempt_at", "attempt_started_at"),
        ("last_failure_at", "last_failure_at"),
    ] {
        metrics[metric] = row
            .get(source)
            .and_then(optional_instant)
            .map(fmt_instant)
            .map(Value::String)
            .unwrap_or(Value::Null);
    }
    match row.get("attempt_state").and_then(Value::as_str) {
        Some("FAILED") => Some(Fact::new(Status::Failed, "RUN_FAILED")),
        Some("RUNNING")
            if row
                .get("heartbeat_at")
                .and_then(optional_instant)
                .is_some_and(|at| now - at > ChronoDuration::seconds(90)) =>
        {
            Some(Fact::new(Status::Degraded, "EXECUTION_INTERRUPTED"))
        }
        _ => None,
    }
}

fn etf_negative_fact(
    group: &Value,
    calendar: &Value,
    now: DateTime<Utc>,
    metrics: &mut Value,
) -> Option<Fact> {
    if db_source_result(
        group
            .get("result")
            .and_then(Value::as_str)
            .unwrap_or("unavailable"),
        group.get("observed_at").and_then(optional_instant),
        now,
    )
    .0 != "ok"
    {
        return None;
    }
    if group.get("dataset_state").and_then(Value::as_str) == Some("BLOCKED")
        || group.get("binding_state").and_then(Value::as_str) == Some("BLOCKED")
        || (group.get("binding_state").and_then(Value::as_str) == Some("PUBLISHED")
            && group.get("fetch_mode").and_then(Value::as_str) != Some("credentialed"))
        || (group
            .get("rights_binding")
            .is_some_and(|binding| !binding.is_null())
            && group.get("rights_valid").and_then(Value::as_bool) == Some(false))
    {
        return Some(Fact::new(Status::Degraded, "DATA_BLOCKED"));
    }
    let view = calendar_eod_view(calendar, now);
    if view.reason.is_some() {
        return None;
    }
    let publication = group.get("publication_session").and_then(parse_date);
    if publication != view.expected {
        metrics["expected_session"] = view
            .expected
            .map(fmt_date)
            .map(Value::String)
            .unwrap_or(Value::Null);
        metrics["data_due_at"] = view
            .due_at
            .map(fmt_instant)
            .map(Value::String)
            .unwrap_or(Value::Null);
        let reason = if publication.is_none() {
            "RAW_ONLY"
        } else {
            "EOD_STALE"
        };
        let fact = eod_lag_fact(metrics, calendar, publication, now, reason);
        if fact.status == Status::Degraded {
            return Some(fact);
        }
    }
    None
}

fn eod_fact(
    etf: &Value,
    research: &Value,
    calendar: &Value,
    research_worker: Option<&Value>,
    service_observation: Option<&Value>,
    timer_source_valid: bool,
    policy_mode: Option<&str>,
    now: DateTime<Utc>,
    metrics: &mut Value,
) -> (Option<Fact>, &'static str, Option<DateTime<Utc>>) {
    if policy_mode == Some("daily") {
        metrics["next_run_at"] = service_observation
            .and_then(|entry| entry.pointer("/timer/next_run_at"))
            .and_then(optional_instant)
            .map(fmt_instant)
            .map(Value::String)
            .unwrap_or(Value::Null);
    }
    let view = calendar_metric_values(metrics, calendar, &Value::Null, now);
    let mut until = min_time(view.current_valid_until, eod_transition_until(now, metrics));
    let etf_result = etf
        .get("result")
        .and_then(Value::as_str)
        .unwrap_or("unavailable");
    if etf_result != "ok" {
        return (
            source_fact(etf_result, "etf"),
            "unknown",
            min_time(
                until,
                etf.get("observed_at")
                    .and_then(optional_instant)
                    .map(|time| time + ChronoDuration::seconds(DB_TTL_SECONDS)),
            ),
        );
    }
    let observed = etf.get("observed_at").and_then(optional_instant);
    let src = db_source_result(etf_result, observed, now);
    if src.0 != "ok" {
        return (source_fact(src.0, "etf"), "unknown", min_time(until, src.2));
    }
    until = min_time(until, src.2);
    let raw_as_of = etf
        .get("raw_as_of")
        .filter(|v| !v.is_null())
        .cloned()
        .unwrap_or(Value::Null);
    metrics["raw_as_of"] = raw_as_of;
    metrics["required_identities"] = etf.get("required_count").cloned().unwrap_or(Value::Null);
    let required = etf.get("required_count").and_then(Value::as_u64);
    let covered = etf.get("covered_count").and_then(Value::as_u64);
    metrics["covered_identities"] = covered.map(|value| json!(value)).unwrap_or(Value::Null);
    if let Some(rights_result) = etf.get("rights_result").and_then(Value::as_str) {
        let rights_source = db_source_result(
            rights_result,
            etf.get("rights_observed_at").and_then(optional_instant),
            now,
        );
        until = min_time(until, rights_source.2);
        if rights_source.0 != "ok" {
            return (source_fact(rights_source.0, "rights"), "unknown", until);
        }
    }
    let publication = etf.get("publication_session").and_then(parse_date);
    let published_at = etf.get("published_at").and_then(optional_instant);
    if published_at.is_some_and(|at| at > now) {
        return (
            Some(Fact::new(Status::Unknown, "SOURCE_FUTURE")),
            "unknown",
            until,
        );
    }
    let expected = view.expected;
    let data_state = etf
        .get("dataset_state")
        .and_then(Value::as_str)
        .unwrap_or("");
    let binding_state = etf
        .get("binding_state")
        .and_then(Value::as_str)
        .unwrap_or("");
    let fetch_mode = etf.get("fetch_mode").and_then(Value::as_str).unwrap_or("");
    // A later BLOCKED dataset denies present Curated assurance. Its exact,
    // complete published binding still records the previous success clock.
    let published_binding_complete = required == Some(11)
        && covered == Some(11)
        && binding_state == "PUBLISHED"
        && fetch_mode == "credentialed"
        && publication.is_some()
        && published_at.is_some()
        && etf
            .get("rights_valid")
            .map(Value::as_bool)
            .unwrap_or(Some(true))
            == Some(true);
    if published_binding_complete {
        metrics["last_success_at"] = published_at
            .map(|value| json!(fmt_instant(value)))
            .unwrap_or(Value::Null);
    }
    let publication_complete =
        published_binding_complete && matches!(data_state, "READY" | "WARNING");
    if publication_complete {
        metrics["data_as_of"] = json!(fmt_date(publication.unwrap()));
    }
    let research_result = research
        .get("result")
        .and_then(Value::as_str)
        .unwrap_or("unavailable");
    let research_src = db_source_result(
        research_result,
        research.get("observed_at").and_then(optional_instant),
        now,
    );
    until = min_time(until, research_src.2);
    let row = research.get("row");
    if let Some(row) =
        row.filter(|row| !row.is_null() && research_result == "ok" && research_src.0 == "ok")
    {
        let attempt = row.get("attempt_started_at").and_then(optional_instant);
        let last_failure = row.get("last_failure_at").and_then(optional_instant);
        metrics["last_attempt_at"] = attempt
            .map(|value| json!(fmt_instant(value)))
            .unwrap_or(Value::Null);
        metrics["last_failure_at"] = last_failure
            .map(|value| json!(fmt_instant(value)))
            .unwrap_or(Value::Null);
        if row.get("attempt_state").and_then(Value::as_str) == Some("RUNNING") {
            if let Some(heartbeat) = row.get("heartbeat_at").and_then(optional_instant) {
                if heartbeat > now {
                    return (
                        Some(Fact::new(Status::Unknown, "SOURCE_FUTURE")),
                        "unknown",
                        until,
                    );
                }
                let heartbeat_until = heartbeat + ChronoDuration::seconds(90);
                if now <= heartbeat_until {
                    until = min_time(until, Some(heartbeat_until));
                }
                if now - heartbeat > ChronoDuration::seconds(90) {
                    return (
                        Some(Fact::new(Status::Degraded, "EXECUTION_INTERRUPTED")),
                        "degraded",
                        until,
                    );
                }
                return (
                    Some(Fact::new(Status::Healthy, "RUN_ACTIVE")),
                    "active",
                    until,
                );
            }
            return (
                Some(Fact::new(Status::Unknown, "DB_UNAVAILABLE")),
                "unknown",
                until,
            );
        }
        match row.get("attempt_state").and_then(Value::as_str) {
            Some("FAILED") => {
                return (
                    Some(Fact::new(Status::Failed, "RUN_FAILED")),
                    "failed",
                    until,
                );
            }
            Some("NO_WORK") => {
                if !publication_complete {
                    return (
                        Some(Fact::new(Status::Waiting, "NOT_APPLICABLE")),
                        "idle",
                        until,
                    );
                }
            }
            Some("SUCCEEDED") => {}
            _ => {
                return (
                    Some(Fact::new(Status::Unknown, "RUN_RESULT_UNKNOWN")),
                    "unknown",
                    until,
                );
            }
        }
    } else if research_result != "ok" || research_src.0 != "ok" {
        return (source_fact(research_src.0, "research"), "unknown", until);
    }
    if timer_source_valid && policy_mode == Some("daemon") {
        let timer = service_observation.and_then(|value| value.get("timer"));
        if timer.is_some_and(|timer| timer["active"] == "active" || timer["enabled"] == "enabled") {
            return (
                Some(Fact::new(Status::Failed, "DAEMON_MODE_CONFLICT")),
                "failed",
                until,
            );
        }
    } else if timer_source_valid {
        let timer = service_observation.and_then(|value| value.get("timer"));
        let process_state = service_observation
            .and_then(|value| value.pointer("/process/state"))
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let timer_valid = timer.is_some();
        if !timer_valid {
            return (
                Some(Fact::new(Status::Failed, "TIMER_MISSING")),
                "failed",
                until,
            );
        }
        let timer = timer.unwrap();
        if timer
            .get("exit_status")
            .and_then(Value::as_u64)
            .is_some_and(|status| matches!(status, 74 | 75))
        {
            return (
                Some(Fact::new(Status::Unknown, "RUN_RESULT_UNKNOWN")),
                "unknown",
                until,
            );
        }
        if timer.get("unit_binding").and_then(Value::as_str) != Some("matched") {
            return (
                Some(Fact::new(Status::Failed, "POLICY_MISMATCH")),
                "failed",
                until,
            );
        }
        if timer.get("load").and_then(Value::as_str) != Some("loaded") {
            return (
                Some(Fact::new(Status::Failed, "TIMER_MISSING")),
                "failed",
                until,
            );
        }
        if timer.get("enabled").and_then(Value::as_str) != Some("enabled") {
            return (
                Some(Fact::new(Status::Failed, "TIMER_DISABLED")),
                "failed",
                until,
            );
        }
        if timer.get("active").and_then(Value::as_str) != Some("active") {
            return (
                Some(Fact::new(Status::Failed, "TIMER_INACTIVE")),
                "failed",
                until,
            );
        }
        let next_run = timer.get("next_run_at").and_then(optional_instant);
        metrics["next_run_at"] = next_run
            .map(|value| json!(fmt_instant(value)))
            .unwrap_or(Value::Null);
        if next_run.is_none() {
            return (
                Some(Fact::new(Status::Failed, "NEXT_RUN_MISSING")),
                "failed",
                until,
            );
        }
        until = min_time(until, next_run.filter(|at| *at > now));
        let started = timer.get("last_started_at").and_then(optional_instant);
        let finished = timer.get("last_finished_at").and_then(optional_instant);
        let success = timer.get("last_success_at").and_then(optional_instant);
        metrics["last_attempt_at"] = started
            .map(|value| json!(fmt_instant(value)))
            .unwrap_or(Value::Null);
        if timer.get("last_result").and_then(Value::as_str) == Some("failure") {
            metrics["last_failure_at"] = finished
                .map(|value| json!(fmt_instant(value)))
                .unwrap_or(Value::Null);
            return (
                Some(Fact::new(Status::Failed, "RUN_FAILED")),
                "failed",
                until,
            );
        }
        if matches!(process_state, "active" | "activating" | "running") {
            let started = started.or_else(|| {
                service_observation
                    .and_then(|value| value.pointer("/process/started_at"))
                    .and_then(optional_instant)
            });
            let Some(started) = started else {
                return (
                    Some(Fact::new(Status::Unknown, "RUN_RESULT_UNKNOWN")),
                    "unknown",
                    until,
                );
            };
            let over = now - started > ChronoDuration::hours(12);
            if !over {
                until = min_time(until, Some(started + ChronoDuration::hours(12)));
            }
            return (
                Some(Fact::new(
                    if over {
                        Status::Degraded
                    } else {
                        Status::Healthy
                    },
                    if over { "RUN_OVERDUE" } else { "RUN_ACTIVE" },
                )),
                "active",
                until,
            );
        }
        if timer.get("last_result").and_then(Value::as_str) == Some("none")
            || (started.is_none() && finished.is_none() && success.is_none())
        {
            return (
                Some(Fact::new(Status::Unknown, "RUN_HISTORY_EMPTY")),
                "unknown",
                until,
            );
        }
        if timer.get("last_result").and_then(Value::as_str) != Some("success") || success.is_none()
        {
            return (
                Some(Fact::new(Status::Unknown, "RUN_RESULT_UNKNOWN")),
                "unknown",
                until,
            );
        }
    }
    if !publication_complete {
        if let Some(expected) = expected {
            if let Some(due) = view.due_at {
                metrics["expected_session"] = json!(fmt_date(expected));
                metrics["data_due_at"] = json!(fmt_instant(due));
                let lag = eod_lag_fact(metrics, calendar, publication, now, "EOD_STALE");
                let deadline = metrics
                    .get("data_due_at")
                    .and_then(optional_instant)
                    .unwrap_or(due);
                until = min_time(until, Some(deadline).filter(|at| *at > now));
                if data_state == "BLOCKED"
                    || binding_state == "BLOCKED"
                    || (!binding_state.is_empty() && fetch_mode != "credentialed")
                {
                    return (
                        Some(Fact::new(Status::Degraded, "DATA_BLOCKED")),
                        "stale",
                        until,
                    );
                }
                if etf
                    .get("rights_binding")
                    .is_some_and(|value| !value.is_null())
                    && etf.get("rights_valid").and_then(Value::as_bool) == Some(false)
                {
                    return (
                        Some(Fact::new(Status::Degraded, "DATA_BLOCKED")),
                        "stale",
                        until,
                    );
                }
                let raw_only = publication.is_none() || covered.unwrap_or(0) == 0;
                if raw_only {
                    let before_due = now < deadline;
                    let status = if before_due {
                        Status::Waiting
                    } else {
                        Status::Degraded
                    };
                    let reason = if before_due {
                        "EOD_PENDING"
                    } else {
                        "RAW_ONLY"
                    };
                    let work = if before_due { "pending" } else { "stale" };
                    return (Some(Fact::new(status, reason)), work, until);
                }
                let work = if lag.status == Status::Waiting {
                    "pending"
                } else {
                    "stale"
                };
                return (Some(lag), work, until);
            }
        }
        if let Some(reason) = view.reason {
            return (Some(reason), "unknown", until);
        }
        return (
            Some(Fact::new(Status::Unknown, "DB_UNAVAILABLE")),
            "unknown",
            until,
        );
    }
    let Some(expected) = expected else {
        return (
            view.reason
                .or_else(|| Some(Fact::new(Status::Unknown, "DB_UNAVAILABLE"))),
            "unknown",
            until,
        );
    };
    metrics["expected_session"] = json!(fmt_date(expected));
    metrics["data_due_at"] = view
        .due_at
        .map(|value| json!(fmt_instant(value)))
        .unwrap_or(Value::Null);
    if publication != Some(expected) {
        let lag = eod_lag_fact(metrics, calendar, publication, now, "EOD_STALE");
        until = min_time(
            until,
            metrics
                .get("data_due_at")
                .and_then(optional_instant)
                .filter(|at| *at > now),
        );
        let work = if lag.status == Status::Waiting {
            "pending"
        } else {
            "stale"
        };
        return (Some(lag), work, until);
    }
    if let Some(worker) = research_worker {
        let worker_result = worker
            .get("result")
            .and_then(Value::as_str)
            .unwrap_or("invalid");
        if worker_result != "ok" {
            return (
                Some(Fact::new(Status::Unknown, host_reason(worker_result))),
                "unknown",
                until,
            );
        }
        if policy_mode == Some("daily")
            && worker
                .pointer("/process/state")
                .and_then(Value::as_str)
                .is_some_and(|state| matches!(state, "running" | "active"))
        {
            return (
                Some(Fact::new(Status::Failed, "DAEMON_MODE_CONFLICT")),
                "failed",
                until,
            );
        }
    }
    if policy_mode == Some("daemon") {
        return (
            Some(Fact::new(Status::Healthy, "DATA_CURRENT")),
            "current",
            until,
        );
    }
    (
        Some(Fact::new(Status::Waiting, "DATA_CURRENT")),
        "current",
        until,
    )
}

fn scheduled_fact(
    id: &str,
    timer: Option<&Value>,
    process_state: &str,
    process_started: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> (
    Option<Fact>,
    &'static str,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
) {
    let Some(timer) = timer else {
        return (
            Some(Fact::new(Status::Failed, "TIMER_MISSING")),
            "failed",
            None,
            None,
            None,
            None,
            None,
        );
    };
    if timer.get("load").and_then(Value::as_str) != Some("loaded") {
        return (
            Some(Fact::new(Status::Failed, "TIMER_MISSING")),
            "failed",
            None,
            None,
            None,
            None,
            None,
        );
    }
    if timer.get("enabled").and_then(Value::as_str) != Some("enabled") {
        return (
            Some(Fact::new(Status::Failed, "TIMER_DISABLED")),
            "failed",
            None,
            None,
            None,
            None,
            None,
        );
    }
    if timer.get("active").and_then(Value::as_str) != Some("active") {
        return (
            Some(Fact::new(Status::Failed, "TIMER_INACTIVE")),
            "failed",
            None,
            None,
            None,
            None,
            None,
        );
    }
    if timer.get("unit_binding").and_then(Value::as_str) != Some("matched") {
        return (
            Some(Fact::new(Status::Failed, "POLICY_MISMATCH")),
            "failed",
            None,
            None,
            None,
            None,
            None,
        );
    }
    let next = timer.get("next_run_at").and_then(optional_instant);
    if next.is_none() {
        return (
            Some(Fact::new(Status::Failed, "NEXT_RUN_MISSING")),
            "failed",
            None,
            None,
            None,
            None,
            None,
        );
    }
    let attempt = timer.get("last_started_at").and_then(optional_instant);
    let finish = timer.get("last_finished_at").and_then(optional_instant);
    let success = timer.get("last_success_at").and_then(optional_instant);
    let failure = None;
    let run_timeout = match id {
        "production-backup-verify" => ChronoDuration::hours(6),
        "tls-renewal" => ChronoDuration::minutes(10),
        _ => ChronoDuration::hours(12),
    };
    if matches!(process_state, "active" | "activating" | "running") {
        let Some(started) = process_started.or(attempt) else {
            return (
                Some(Fact::new(Status::Unknown, "RUN_RESULT_UNKNOWN")),
                "unknown",
                next,
                attempt,
                success,
                failure,
                next,
            );
        };
        let deadline = started + run_timeout;
        let overdue = now > deadline;
        let until = min_time(
            next.filter(|at| *at > now),
            if !overdue { Some(deadline) } else { None },
        );
        return (
            Some(Fact::new(
                if overdue {
                    Status::Failed
                } else {
                    Status::Healthy
                },
                if overdue { "RUN_OVERDUE" } else { "RUN_ACTIVE" },
            )),
            if overdue { "failed" } else { "active" },
            next,
            attempt.or(Some(started)),
            success,
            failure,
            until,
        );
    }
    if timer.get("last_result").and_then(Value::as_str) == Some("failure") {
        return (
            Some(Fact::new(Status::Failed, "RUN_FAILED")),
            "failed",
            next,
            attempt,
            success,
            finish,
            next,
        );
    }
    if timer.get("last_result").and_then(Value::as_str) == Some("none")
        || (attempt.is_none() && finish.is_none() && success.is_none())
    {
        return (
            Some(Fact::new(Status::Unknown, "RUN_HISTORY_EMPTY")),
            "unknown",
            next,
            attempt,
            success,
            failure,
            next,
        );
    }
    if timer.get("last_result").and_then(Value::as_str) != Some("success") || success.is_none() {
        return (
            Some(Fact::new(Status::Unknown, "RUN_RESULT_UNKNOWN")),
            "unknown",
            next,
            attempt,
            success,
            failure,
            next,
        );
    }
    (
        Some(Fact::new(Status::Waiting, "NOT_APPLICABLE")),
        "scheduled",
        next,
        attempt,
        success,
        failure,
        next,
    )
}

fn stream_facts(
    stream: &Value,
    calendar: &Value,
    window: &Value,
    now: DateTime<Utc>,
    metrics: &mut Value,
    semantic_until: &mut Option<DateTime<Utc>>,
) -> Vec<Fact> {
    let cw = calendar_window_fact(calendar, window, now);
    metrics["calendar_status"] = json!(cw.1);
    metrics["window_status"] = json!(if matches!(cw.2, "missing" | "invalid" | "unknown") {
        cw.2
    } else {
        window_status(window, now)
    });
    *semantic_until = min_time(*semantic_until, cw.3);
    let calendar_failure =
        cw.0.as_ref()
            .filter(|fact| fact.status != Status::Waiting)
            .cloned();
    let mut independent = calendar_failure.clone().into_iter().collect::<Vec<_>>();
    if let Some(window_failure) = window_negative_fact(window, now) {
        if !independent
            .iter()
            .any(|fact| fact.reason == window_failure.reason)
        {
            independent.push(window_failure);
        }
    }
    let result = stream
        .get("result")
        .and_then(Value::as_str)
        .unwrap_or("unavailable");
    let source = db_source_result(
        result,
        stream.get("observed_at").and_then(optional_instant),
        now,
    );
    if source.0 != "ok" {
        independent.push(
            source_fact(source.0, "stream")
                .unwrap_or_else(|| Fact::new(Status::Unknown, "DB_UNAVAILABLE")),
        );
        return independent;
    }
    *semantic_until = min_time(*semantic_until, source.2);
    if stream.get("owner_binding_valid").and_then(Value::as_bool) != Some(true) {
        if source_for_window(window, now)["result"] == "ok" {
            independent.push(Fact::new(Status::Failed, "STREAM_BINDING_UNAVAILABLE"));
        }
        return independent;
    }
    let counts = ["demand_count", "live_count", "halted_count"]
        .map(|name| stream.get(name).and_then(Value::as_u64));
    let [Some(demand), Some(live), Some(halted)] = counts else {
        independent.push(Fact::new(Status::Unknown, "DB_UNAVAILABLE"));
        return independent;
    };
    if demand > 30 || live > demand || halted > demand || live + halted > demand {
        independent.push(Fact::new(Status::Degraded, "QUERY_LIMIT"));
        return independent;
    }
    let query_until = stream.get("valid_until").and_then(optional_instant);
    *semantic_until = min_time(*semantic_until, query_until);
    let code = stream
        .get("status_code")
        .filter(|value| !value.is_null())
        .and_then(Value::as_str);
    metrics["active_demand"] = json!(demand);
    metrics["live_identities"] = json!(live);
    metrics["halted_identities"] = json!(halted);
    for (metric, source) in [
        ("last_received_at", "last_received_at"),
        ("oldest_provider_event_at", "oldest_provider_trade_at"),
        ("last_state_change_at", "producer_status_at"),
    ] {
        metrics[metric] = stream.get(source).cloned().unwrap_or(Value::Null);
    }
    // An expired cached aggregate is not a fresh observation. Keep its real
    // bound and diagnostic clocks, and stop publishing its positive metrics.
    if query_until.is_some_and(|bound| now > bound) {
        metrics["active_demand"] = Value::Null;
        metrics["live_identities"] = Value::Null;
        metrics["halted_identities"] = Value::Null;
    }
    if !independent.is_empty() {
        // An unavailable calendar cannot erase a separately observed typed
        // producer failure or failed binding; it cannot establish closure.
        if let Some(code) = code.filter(|code| stream_code_is_failed(code)) {
            independent.push(Fact::new(Status::Failed, code));
        }
        if live + halted > 0 {
            add_stream_semantic_boundaries(stream, now, semantic_until, true);
        }
        return independent;
    }
    let closed = cw.1 == "closed" || cw.2 == "closed";
    if closed {
        return vec![cw.0.unwrap_or_else(|| Fact::new(Status::Waiting, "SESSION_CLOSED"))];
    }
    if let Some(code) = code {
        let outcome = if stream_code_is_failed(code) {
            Status::Failed
        } else if code == "NO_ACTIVE_DEMAND" && demand == 0 {
            Status::Waiting
        } else {
            Status::Degraded
        };
        if matches!(code, "NO_ACTIVE_DEMAND" | "SESSION_CLOSED") && outcome == Status::Degraded {
            return vec![Fact::new(Status::Degraded, "QUOTE_LINEAGE_INVALID")];
        }
        // Typed failure/connecting/gap facts do not borrow old success clocks.
        // Any actual positive coverage is nevertheless bounded by SQL's
        // filtered assurance minima and the applicable producer predicates.
        if live + halted > 0 {
            add_stream_semantic_boundaries(stream, now, semantic_until, true);
        }
        return vec![Fact::new(outcome, code)];
    }
    if demand == 0 {
        return vec![Fact::new(Status::Waiting, "NO_ACTIVE_DEMAND")];
    }
    if stream.get("producer_present").and_then(Value::as_bool) != Some(true) {
        return vec![Fact::new(Status::Failed, "PRODUCER_UNAVAILABLE")];
    }
    match stream.get("connection_state").and_then(Value::as_str) {
        Some("DISCONNECTED" | "STOPPED") => {
            return vec![Fact::new(Status::Failed, "STREAM_DISCONNECTED")];
        }
        Some("CONNECTING") => return vec![Fact::new(Status::Degraded, "STREAM_CONNECTING")],
        Some("BACKOFF") => return vec![Fact::new(Status::Degraded, "STREAM_BACKOFF")],
        Some("CONNECTED") => {}
        _ => return vec![Fact::new(Status::Unknown, "DB_UNAVAILABLE")],
    }
    if stream.get("gap_present").and_then(Value::as_bool) == Some(true) {
        return vec![Fact::new(Status::Degraded, "STREAM_GAP")];
    }
    if stream.get("lineage_valid").and_then(Value::as_bool) != Some(true) {
        return vec![Fact::new(Status::Degraded, "QUOTE_LINEAGE_INVALID")];
    }
    let heartbeat = stream
        .get("producer_heartbeat_at")
        .and_then(optional_instant);
    let lease = stream
        .get("producer_lease_expires_at")
        .and_then(optional_instant);
    if heartbeat.is_none_or(|at| at > now || now - at > ChronoDuration::seconds(10))
        || lease.is_none_or(|at| at <= now + ChronoDuration::seconds(5))
    {
        if live + halted > 0 {
            metrics["live_identities"] = Value::Null;
            metrics["halted_identities"] = Value::Null;
        }
        return vec![Fact::new(Status::Failed, "PRODUCER_STALE")];
    }
    let receipt = stream.get("oldest_received_at").and_then(optional_instant);
    let provider = stream
        .get("oldest_provider_trade_at")
        .and_then(optional_instant);
    if receipt.is_some_and(|at| at > now) || provider.is_some_and(|at| at > now) {
        return vec![Fact::new(Status::Degraded, "SOURCE_FUTURE")];
    }
    if receipt.is_some_and(|at| now - at > ChronoDuration::seconds(30))
        || provider.is_some_and(|at| now - at > ChronoDuration::seconds(30))
    {
        if live + halted > 0 {
            add_stream_semantic_boundaries(stream, now, semantic_until, true);
        }
        return vec![Fact::new(Status::Degraded, "QUOTE_STALE")];
    }
    // Pending readiness and every normal/halted positive metric retain their
    // exact assurance, including accepted inclusive equality.
    add_stream_semantic_boundaries(stream, now, semantic_until, live + halted > 0);
    if receipt.is_none() || provider.is_none() {
        return vec![Fact::new(Status::Degraded, "AWAITING_FIRST_TRADE")];
    }
    if live + halted != demand {
        return vec![Fact::new(
            Status::Degraded,
            if live + halted == 0 {
                "AWAITING_FIRST_TRADE"
            } else {
                "QUOTE_STALE"
            },
        )];
    }
    if query_until.is_some_and(|bound| now > bound) {
        return vec![Fact::new(Status::Degraded, "QUOTE_STALE")];
    }
    if halted == demand {
        return vec![Fact::new(Status::Waiting, "INSTRUMENT_HALTED")];
    }
    vec![Fact::new(Status::Healthy, "DATA_CURRENT")]
}

fn stream_code_is_failed(code: &str) -> bool {
    matches!(
        code,
        "FEATURE_DISABLED"
            | "CALENDAR_UNAVAILABLE"
            | "SESSION_WINDOW_UNAVAILABLE"
            | "CONNECTION_LOST"
            | "SUBSCRIPTION_REJECTED"
            | "APPROVAL_UNAVAILABLE"
            | "PRODUCER_UNAVAILABLE"
            | "ACCESS_REVOKED"
    )
}

fn add_stream_semantic_boundaries(
    stream: &Value,
    now: DateTime<Utc>,
    until: &mut Option<DateTime<Utc>>,
    covered: bool,
) {
    if let Some(at) = stream
        .get("producer_heartbeat_at")
        .and_then(optional_instant)
        && at <= now
        && now - at <= ChronoDuration::seconds(10)
    {
        *until = min_time(*until, Some(at + ChronoDuration::seconds(10)));
    }
    if let Some(at) = stream
        .get("producer_lease_expires_at")
        .and_then(optional_instant)
        && at > now + ChronoDuration::seconds(5)
    {
        *until = min_time(*until, Some(at - ChronoDuration::seconds(5)));
    }
    if covered {
        // Raw minima may include a stale uncovered identity. Only still-valid
        // raw bounds can be used here; SQL retains filtered covered minima in
        // its immutable aggregate deadline, including partial/halted coverage.
        for field in ["oldest_received_at", "oldest_provider_trade_at"] {
            if let Some(at) = stream.get(field).and_then(optional_instant)
                && at <= now
                && now - at <= ChronoDuration::seconds(30)
            {
                *until = min_time(*until, Some(at + ChronoDuration::seconds(30)));
            }
        }
    }
}

fn stream_work_status(
    stream: &Value,
    window: &Value,
    calendar: &Value,
    now: DateTime<Utc>,
    status: Status,
    reason: &str,
) -> &'static str {
    if status == Status::Waiting {
        let cw = calendar_window_fact(calendar, window, now);
        if cw.1 == "closed"
            || cw.2 == "closed"
            || (stream.get("halted_count").and_then(Value::as_u64)
                == stream.get("demand_count").and_then(Value::as_u64)
                && stream
                    .get("demand_count")
                    .and_then(Value::as_u64)
                    .is_some_and(|count| count > 0))
        {
            return "closed";
        }
        if stream.get("demand_count").and_then(Value::as_u64) == Some(0) {
            return "no_demand";
        }
    }
    match status {
        Status::Failed => "failed",
        Status::Degraded
            if stream.get("status_code").is_none_or(Value::is_null)
                && matches!(
                    reason,
                    "QUOTE_STALE" | "QUOTE_LINEAGE_INVALID" | "SOURCE_FUTURE"
                ) =>
        {
            "stale"
        }
        Status::Degraded
            if stream.get("status_code").is_none_or(Value::is_null)
                && reason == "AWAITING_FIRST_TRADE" =>
        {
            "pending"
        }
        Status::Degraded => "degraded",
        Status::Unknown => "unknown",
        Status::Healthy => "current",
        Status::Waiting => "pending",
        Status::Disabled => "disabled",
    }
}
