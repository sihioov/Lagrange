//! Durable state and local resource bounds for the market-only socket.
//!
//! The state in this module is intentionally a different file and lock from
//! the REST token/read-coordination ledger.  It is suitable for a later
//! installer to mount into the one authorized producer, while all tests use a
//! temporary directory explicitly supplied by the test.

use std::collections::VecDeque;
use std::ffi::CString;
use std::fmt;
#[cfg(any(feature = "test-support", test))]
use std::fs;
use std::fs::File;
#[cfg(any(feature = "test-support", test))]
use std::fs::{OpenOptions, Permissions};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::ffi::OsStrExt;
#[cfg(any(feature = "test-support", test))]
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const STATE_SCHEMA: &str = "kis-market-stream-state-v1";
pub const STATE_FILE: &str = "approval-state-v1.json";
pub const CONNECTION_LOCK_FILE: &str = "connection.lock";
pub const STATE_LOCK_FILE: &str = "state.lock";
const PRODUCTION_STATE_DIRECTORY: &str = "/run/lagrange/kis-market-stream";
const PRODUCTION_ANCHOR_DIRECTORY: &str = "/run/lagrange/kis-market-stream-locks";
#[cfg(feature = "market-stream-provisioning")]
const PRODUCTION_STATE_LEAF: &str = "kis-market-stream";
#[cfg(feature = "market-stream-provisioning")]
const PRODUCTION_ANCHOR_LEAF: &str = "kis-market-stream-locks";
const MAX_STATE_FILE_BYTES: u64 = 256 * 1024;
const MAX_PERSISTED_APPROVAL_KEY_BYTES: usize = 8 * 1024;
const PRODUCTION_STATE_UID: u32 = 10_001;
const PRODUCTION_STATE_GID: u32 = 10_001;
const PRODUCTION_ANCHOR_UID: u32 = 0;
const PRODUCTION_ANCHOR_GID: u32 = 10_001;
const STATE_DIRECTORY_MODE: u32 = 0o700;
const STATE_FILE_MODE: u32 = 0o600;
const ANCHOR_DIRECTORY_MODE: u32 = 0o750;
const ANCHOR_FILE_MODE: u32 = 0o440;

// These are the Linux constants used by the existing read-coordination
// implementation.  Opening relative to one verified directory descriptor is
// important: a pathname metadata check followed by a pathname open is a
// replaceable TOCTOU pair.
const O_RDONLY: i32 = 0;
const O_WRONLY: i32 = 1;
const O_CREAT: i32 = 0o100;
const O_EXCL: i32 = 0o200;
const O_CLOEXEC: i32 = 0o2000000;
const O_DIRECTORY: i32 = 0o200000;
const O_NOFOLLOW: i32 = 0o400000;
#[cfg(feature = "market-stream-provisioning")]
const O_PATH: i32 = 0o10000000;
#[cfg(feature = "market-stream-provisioning")]
const RENAME_NOREPLACE: u32 = 1;

unsafe extern "C" {
    fn open(path: *const std::ffi::c_char, flags: i32, mode: u32) -> RawFd;
    fn openat(dirfd: RawFd, path: *const std::ffi::c_char, flags: i32, mode: u32) -> RawFd;
    fn renameat(
        olddirfd: RawFd,
        oldpath: *const std::ffi::c_char,
        newdirfd: RawFd,
        newpath: *const std::ffi::c_char,
    ) -> i32;
    fn unlinkat(dirfd: RawFd, path: *const std::ffi::c_char, flags: i32) -> i32;
    #[cfg(feature = "market-stream-provisioning")]
    fn mkdirat(dirfd: RawFd, path: *const std::ffi::c_char, mode: u32) -> i32;
    #[cfg(feature = "market-stream-provisioning")]
    fn fchown(fd: RawFd, owner: u32, group: u32) -> i32;
    #[cfg(feature = "market-stream-provisioning")]
    fn fchmod(fd: RawFd, mode: u32) -> i32;
    #[cfg(feature = "market-stream-provisioning")]
    fn renameat2(
        olddirfd: RawFd,
        oldpath: *const std::ffi::c_char,
        newdirfd: RawFd,
        newpath: *const std::ffi::c_char,
        flags: u32,
    ) -> i32;
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionEpoch(Uuid);

impl ConnectionEpoch {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn uuid(self) -> Uuid {
        self.0
    }
}

impl Default for ConnectionEpoch {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for ConnectionEpoch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Display for ConnectionEpoch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Backoff,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetError {
    MinimumSpacing,
    RollingWindow,
    DailyLimit,
}

impl fmt::Display for BudgetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::MinimumSpacing => "market attempt minimum spacing is not met",
            Self::RollingWindow => "market attempt rolling limit is exhausted",
            Self::DailyLimit => "market attempt daily limit is exhausted",
        })
    }
}

impl std::error::Error for BudgetError {}

/// Outbound subscribe/unsubscribe budget.  It is a project bound, not a KIS
/// quota claim, and is persisted before a command is sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandBudget {
    attempts_ms: VecDeque<i64>,
    last_attempt_ms: Option<i64>,
}

impl Default for CommandBudget {
    fn default() -> Self {
        Self {
            attempts_ms: VecDeque::new(),
            last_attempt_ms: None,
        }
    }
}

impl CommandBudget {
    pub(crate) const MIN_SPACING_MS: i64 = 1_000;
    pub(crate) const ROLLING_WINDOW_MS: i64 = 10 * 60 * 1_000;
    pub(crate) const ROLLING_LIMIT: usize = 120;
    pub(crate) const DAILY_WINDOW_MS: i64 = 24 * 60 * 60 * 1_000;
    pub(crate) const DAILY_LIMIT: usize = 1_000;

    pub(crate) fn reserve(&mut self, now_ms: i64) -> Result<(), BudgetError> {
        self.prune(now_ms);
        if let Some(last) = self.last_attempt_ms
            && now_ms.saturating_sub(last) < Self::MIN_SPACING_MS
        {
            return Err(BudgetError::MinimumSpacing);
        }
        let rolling_start = now_ms.saturating_sub(Self::ROLLING_WINDOW_MS);
        if self
            .attempts_ms
            .iter()
            .filter(|timestamp| **timestamp > rolling_start)
            .count()
            >= Self::ROLLING_LIMIT
        {
            return Err(BudgetError::RollingWindow);
        }
        let daily_start = now_ms.saturating_sub(Self::DAILY_WINDOW_MS);
        if self
            .attempts_ms
            .iter()
            .filter(|timestamp| **timestamp > daily_start)
            .count()
            >= Self::DAILY_LIMIT
        {
            return Err(BudgetError::DailyLimit);
        }
        self.attempts_ms.push_back(now_ms);
        self.last_attempt_ms = Some(now_ms);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.attempts_ms.len()
    }

    pub(crate) fn attempts(&self) -> impl Iterator<Item = i64> + '_ {
        self.attempts_ms.iter().copied()
    }

    pub(crate) fn from_attempts(attempts: impl IntoIterator<Item = i64>) -> Self {
        let mut out = Self::default();
        for timestamp in attempts {
            out.attempts_ms.push_back(timestamp);
            out.last_attempt_ms = Some(timestamp);
        }
        out
    }

    fn prune(&mut self, now_ms: i64) {
        let start = now_ms.saturating_sub(Self::DAILY_WINDOW_MS);
        while self
            .attempts_ms
            .front()
            .is_some_and(|timestamp| *timestamp <= start)
        {
            self.attempts_ms.pop_front();
        }
        if self
            .last_attempt_ms
            .is_some_and(|timestamp| timestamp <= start)
        {
            self.last_attempt_ms = self.attempts_ms.back().copied();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReconnectPolicy {
    pub minimum_spacing: Duration,
    pub base_backoff: Duration,
    pub capped_backoff: Duration,
    pub positive_jitter_percent: u8,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            minimum_spacing: Duration::from_secs(10),
            base_backoff: Duration::from_secs(10),
            capped_backoff: Duration::from_secs(60),
            positive_jitter_percent: 20,
        }
    }
}

impl ReconnectPolicy {
    pub(crate) fn delay_for_ordinal(&self, ordinal: u32, jitter_percent: u8) -> Duration {
        let exponent = ordinal.saturating_sub(1).min(3);
        let base = self
            .base_backoff
            .saturating_mul(1u32 << exponent)
            .min(self.capped_backoff);
        let jitter = u64::from(jitter_percent.min(self.positive_jitter_percent));
        base.saturating_add(base.mul_f64(jitter as f64 / 100.0))
    }
}

/// Durable reconnect budget.  The caller supplies deterministic jitter in
/// tests; production may use a bounded random percentage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReconnectBudget {
    attempts_10m_ms: VecDeque<i64>,
    attempts_day_ms: VecDeque<i64>,
    last_attempt_ms: Option<i64>,
}

impl Default for ReconnectBudget {
    fn default() -> Self {
        Self {
            attempts_10m_ms: VecDeque::new(),
            attempts_day_ms: VecDeque::new(),
            last_attempt_ms: None,
        }
    }
}

impl ReconnectBudget {
    pub const MIN_SPACING_MS: i64 = 10_000;
    pub const TEN_MINUTES_MS: i64 = 10 * 60 * 1_000;
    pub const TEN_MINUTES_LIMIT: usize = 6;
    pub const DAY_MS: i64 = 24 * 60 * 60 * 1_000;
    pub const DAY_LIMIT: usize = 20;

    pub(crate) fn reserve(&mut self, now_ms: i64) -> Result<(), BudgetError> {
        self.prune(now_ms);
        if self
            .last_attempt_ms
            .is_some_and(|last| now_ms.saturating_sub(last) < Self::MIN_SPACING_MS)
        {
            return Err(BudgetError::MinimumSpacing);
        }
        if self.attempts_10m_ms.len() >= Self::TEN_MINUTES_LIMIT {
            return Err(BudgetError::RollingWindow);
        }
        if self.attempts_day_ms.len() >= Self::DAY_LIMIT {
            return Err(BudgetError::DailyLimit);
        }
        self.attempts_10m_ms.push_back(now_ms);
        self.attempts_day_ms.push_back(now_ms);
        self.last_attempt_ms = Some(now_ms);
        Ok(())
    }

    pub(crate) fn attempts_10m(&self) -> impl Iterator<Item = i64> + '_ {
        self.attempts_10m_ms.iter().copied()
    }

    pub(crate) fn attempts_day(&self) -> impl Iterator<Item = i64> + '_ {
        self.attempts_day_ms.iter().copied()
    }

    pub(crate) fn from_attempts(
        attempts_10m: impl IntoIterator<Item = i64>,
        attempts_day: impl IntoIterator<Item = i64>,
    ) -> Self {
        let mut out = Self::default();
        out.attempts_10m_ms.extend(attempts_10m);
        out.attempts_day_ms.extend(attempts_day);
        out.last_attempt_ms = out.attempts_day_ms.back().copied();
        out
    }

    fn prune(&mut self, now_ms: i64) {
        let start_10m = now_ms.saturating_sub(Self::TEN_MINUTES_MS);
        while self
            .attempts_10m_ms
            .front()
            .is_some_and(|timestamp| *timestamp <= start_10m)
        {
            self.attempts_10m_ms.pop_front();
        }
        let start_day = now_ms.saturating_sub(Self::DAY_MS);
        while self
            .attempts_day_ms
            .front()
            .is_some_and(|timestamp| *timestamp <= start_day)
        {
            self.attempts_day_ms.pop_front();
        }
        if self
            .last_attempt_ms
            .is_some_and(|timestamp| timestamp <= start_day)
        {
            self.last_attempt_ms = self.attempts_day_ms.back().copied();
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateError {
    Io,
    UnsafePath,
    InvalidState,
    LockBusy,
    PriorSessionUncertain,
    ReconnectNotReady,
    Serialization,
    Budget(BudgetError),
    #[cfg(feature = "market-stream-provisioning")]
    ProvisioningActorDenied,
    #[cfg(feature = "market-stream-provisioning")]
    ProvisioningInputInvalid,
    #[cfg(feature = "market-stream-provisioning")]
    ProvisioningUncertain,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ObjectIdentity {
    dev: u64,
    ino: u64,
}

impl ObjectIdentity {
    fn from_metadata(metadata: &std::fs::Metadata) -> Self {
        Self {
            dev: metadata.dev(),
            ino: metadata.ino(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
struct DomainBinding {
    state_directory: ObjectIdentity,
    anchor_directory: ObjectIdentity,
    connection_anchor: ObjectIdentity,
    state_anchor: ObjectIdentity,
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Io => "market stream state I/O failed",
            Self::UnsafePath => "market stream state path is not a safe single-link path",
            Self::InvalidState => "market stream state is invalid",
            Self::LockBusy => "market stream lock is held by another process",
            Self::PriorSessionUncertain => {
                "market stream has an uncertain prior broker session and needs operator review"
            }
            Self::ReconnectNotReady => "market stream reconnect backoff is not ready",
            Self::Serialization => "market stream state serialization failed",
            Self::Budget(error) => return error.fmt(f),
            #[cfg(feature = "market-stream-provisioning")]
            Self::ProvisioningActorDenied => "market stream provisioning actor is not allowed",
            #[cfg(feature = "market-stream-provisioning")]
            Self::ProvisioningInputInvalid => "market stream provisioning input is invalid",
            #[cfg(feature = "market-stream-provisioning")]
            Self::ProvisioningUncertain => "market stream provisioning outcome is uncertain",
        })
    }
}

impl std::error::Error for StateError {}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct DurablePendingCommand {
    pub(crate) epoch: String,
    pub(crate) operation: String,
    pub(crate) symbol: String,
    pub(crate) ordinal: u64,
    pub(crate) sent_at_ms: i64,
    pub(crate) deadline_ms: i64,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub(crate) struct DurableWsState {
    pub(crate) schema: String,
    credential_slot_id: String,
    domain_binding: DomainBinding,
    pub(crate) credential_generation: String,
    pub(crate) approval_key: Option<String>,
    pub(crate) approval_issued_at_ms: Option<i64>,
    pub(crate) approval_expires_at_ms: Option<i64>,
    pub(crate) approval_reservations_ms: Vec<i64>,
    pub(crate) approval_ambiguous: bool,
    pub(crate) approval_dispatch_in_flight: bool,
    pub(crate) current_epoch: Option<String>,
    pub(crate) command_attempts_ms: Vec<i64>,
    pub(crate) reconnect_attempts_10m_ms: Vec<i64>,
    pub(crate) reconnect_attempts_day_ms: Vec<i64>,
    #[serde(default)]
    pub(crate) next_command_ordinal: u64,
    #[serde(default)]
    pub(crate) pending_command: Option<DurablePendingCommand>,
    #[serde(default)]
    pub(crate) connection_attempt_id: Option<String>,
    #[serde(default)]
    pub(crate) connection_attempt_started_ms: Option<i64>,
    #[serde(default)]
    pub(crate) reconnect_next_allowed_at_ms: Option<i64>,
    #[serde(default)]
    pub(crate) reconnect_sequence: u32,
    #[serde(default)]
    pub(crate) has_connected: bool,
}

impl DurableWsState {
    fn is_valid(&self) -> bool {
        self.schema == STATE_SCHEMA
            && self.credential_generation.len() <= 128
            && self.approval_key.as_ref().is_none_or(|key| {
                key.len() <= MAX_PERSISTED_APPROVAL_KEY_BYTES
                    && !key.bytes().any(|byte| byte < 0x20 || byte == 0x7f)
            })
            && (self.approval_key.is_none()
                || (!self.credential_generation.is_empty()
                    && self.approval_issued_at_ms.is_some()
                    && self.approval_expires_at_ms.is_some()))
            && self.approval_reservations_ms.len() <= 2
            && self.command_attempts_ms.len() <= CommandBudget::DAILY_LIMIT
            && self.reconnect_attempts_10m_ms.len() <= ReconnectBudget::TEN_MINUTES_LIMIT
            && self.reconnect_attempts_day_ms.len() <= ReconnectBudget::DAY_LIMIT
            && self
                .current_epoch
                .as_ref()
                .is_none_or(|epoch| Uuid::parse_str(epoch).is_ok())
            && self
                .connection_attempt_id
                .as_ref()
                .is_none_or(|attempt| Uuid::parse_str(attempt).is_ok())
            && self.pending_command.as_ref().is_none_or(|pending| {
                Uuid::parse_str(&pending.epoch).is_ok()
                    && matches!(pending.operation.as_str(), "subscribe" | "unsubscribe")
                    && pending.symbol.len() == 6
                    && pending.symbol.bytes().all(|byte| byte.is_ascii_digit())
                    && pending.ordinal > 0
                    && pending.deadline_ms >= pending.sent_at_ms
            })
            && self.next_command_ordinal <= u64::MAX - 1
            && Uuid::parse_str(&self.credential_slot_id).is_ok()
    }
}

/// The canonical state/lock domain for one opaque credential slot.
///
/// Production construction accepts no paths, owners, modes, or anchor names.
/// The installer owns provisioning; runtime only opens and validates existing
/// objects.
///
/// ```compile_fail
/// use kis_client::market_stream_state::WsStateStore;
/// ```
///
/// ```compile_fail
/// use kis_client::MarketStreamDomain;
/// use uuid::Uuid;
/// let _ = MarketStreamDomain::for_test("/tmp/arbitrary", Uuid::nil());
/// ```
#[derive(Clone)]
pub struct MarketStreamDomain {
    state: WsStateStore,
}

impl fmt::Debug for MarketStreamDomain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MarketStreamDomain")
            .field("credential_slot_id", &self.state.credential_slot_id)
            .finish_non_exhaustive()
    }
}

impl MarketStreamDomain {
    pub fn open_production(credential_slot_id: Uuid) -> Result<Self, StateError> {
        Self::open_verified(
            PathBuf::from(PRODUCTION_STATE_DIRECTORY),
            PathBuf::from(PRODUCTION_ANCHOR_DIRECTORY),
            credential_slot_id,
            PRODUCTION_STATE_UID,
            PRODUCTION_STATE_GID,
            PRODUCTION_ANCHOR_UID,
            PRODUCTION_ANCHOR_GID,
            false,
        )
    }

    #[cfg(any(feature = "test-support", test))]
    pub fn for_test(root: impl AsRef<Path>, credential_slot_id: Uuid) -> Result<Self, StateError> {
        let root = root.as_ref();
        let state_dir = root.join("kis-market-stream");
        let anchor_dir = root.join("kis-market-stream-locks");
        let root_metadata = fs::metadata(root).map_err(|_| StateError::UnsafePath)?;
        let uid = root_metadata.uid();
        let gid = root_metadata.gid();
        let state_exists = fs::symlink_metadata(&state_dir).is_ok();
        let anchor_exists = fs::symlink_metadata(&anchor_dir).is_ok();
        let created_layout = !state_exists && !anchor_exists;
        if created_layout {
            provision_new_test_layout(&state_dir, &anchor_dir)?;
        } else if !state_exists || !anchor_exists {
            return Err(StateError::UnsafePath);
        }
        Self::open_verified(
            state_dir,
            anchor_dir,
            credential_slot_id,
            uid,
            gid,
            uid,
            gid,
            created_layout,
        )
    }

    fn open_verified(
        state_dir: PathBuf,
        anchor_dir: PathBuf,
        credential_slot_id: Uuid,
        state_uid: u32,
        state_gid: u32,
        anchor_uid: u32,
        anchor_gid: u32,
        initialize_new: bool,
    ) -> Result<Self, StateError> {
        let state_directory =
            open_directory(&state_dir, state_uid, state_gid, STATE_DIRECTORY_MODE)?;
        let anchor_directory =
            open_directory(&anchor_dir, anchor_uid, anchor_gid, ANCHOR_DIRECTORY_MODE)?;
        let state_identity = ObjectIdentity::from_metadata(
            &state_directory
                .metadata()
                .map_err(|_| StateError::UnsafePath)?,
        );
        let anchor_identity = ObjectIdentity::from_metadata(
            &anchor_directory
                .metadata()
                .map_err(|_| StateError::UnsafePath)?,
        );
        let connection = open_anchor(
            &anchor_directory,
            CONNECTION_LOCK_FILE,
            anchor_uid,
            anchor_gid,
        )?;
        let state = open_anchor(&anchor_directory, STATE_LOCK_FILE, anchor_uid, anchor_gid)?;
        let connection_identity = ObjectIdentity::from_metadata(
            &connection.metadata().map_err(|_| StateError::UnsafePath)?,
        );
        let state_anchor_identity =
            ObjectIdentity::from_metadata(&state.metadata().map_err(|_| StateError::UnsafePath)?);
        if connection_identity == state_anchor_identity {
            return Err(StateError::UnsafePath);
        }
        let binding = DomainBinding {
            state_directory: state_identity,
            anchor_directory: anchor_identity,
            connection_anchor: connection_identity,
            state_anchor: state_anchor_identity,
        };
        let store = WsStateStore {
            dir: state_dir,
            directory: Arc::new(state_directory),
            anchor_directory: Arc::new(anchor_directory),
            credential_slot_id,
            binding,
            state_uid,
            state_gid,
            anchor_uid,
            anchor_gid,
        };
        if initialize_new {
            store.initialize_new()?;
        } else {
            store.validate_existing()?;
        }
        Ok(Self { state: store })
    }

    pub(crate) fn state(&self) -> &WsStateStore {
        &self.state
    }

    pub(crate) fn credential_slot_id(&self) -> Uuid {
        self.state.credential_slot_id
    }

    #[cfg(any(feature = "test-support", test))]
    pub fn state_directory(&self) -> &Path {
        &self.state.dir
    }

    #[cfg(any(feature = "test-support", test))]
    pub fn test_lock(&self) -> Result<MarketStreamLock, StateError> {
        self.state.lock()
    }

    #[cfg(feature = "test-support")]
    pub fn test_state_lock(&self) -> Result<MarketStreamLock, StateError> {
        self.state.test_state_lock()
    }

    #[cfg(feature = "test-support")]
    pub fn test_append_command_attempt(&self, timestamp_ms: i64) -> Result<(), StateError> {
        self.state.test_append_command_attempt(timestamp_ms)
    }
}

#[derive(Clone)]
pub(crate) struct WsStateStore {
    dir: PathBuf,
    directory: Arc<File>,
    anchor_directory: Arc<File>,
    credential_slot_id: Uuid,
    binding: DomainBinding,
    state_uid: u32,
    state_gid: u32,
    anchor_uid: u32,
    anchor_gid: u32,
}

impl fmt::Debug for WsStateStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WsStateStore")
            .field("dir", &self.dir)
            .finish()
    }
}

impl WsStateStore {
    fn initialize_new(&self) -> Result<(), StateError> {
        let _guard = self.state_guard()?;
        match open_at(&self.directory, STATE_FILE, O_RDONLY, 0) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.persist_new(&self.new_state())
            }
            _ => Err(StateError::PriorSessionUncertain),
        }
    }

    fn validate_existing(&self) -> Result<(), StateError> {
        let _guard = self.state_guard()?;
        let state = self.load_unlocked()?;
        self.validate_domain(&state)
    }

    fn new_state(&self) -> DurableWsState {
        DurableWsState {
            schema: STATE_SCHEMA.to_owned(),
            credential_slot_id: self.credential_slot_id.to_string(),
            domain_binding: self.binding.clone(),
            ..DurableWsState::default()
        }
    }

    fn validate_domain(&self, state: &DurableWsState) -> Result<(), StateError> {
        if state.credential_slot_id != self.credential_slot_id.to_string()
            || state.domain_binding != self.binding
        {
            return Err(StateError::InvalidState);
        }
        Ok(())
    }

    /// Test-only durability hook used by the separate-process acceptance
    /// test.  It records a non-secret attempt timestamp and cannot provision
    /// an approval key, grant, epoch or publication receipt.
    #[cfg(feature = "test-support")]
    pub fn test_append_command_attempt(&self, timestamp_ms: i64) -> Result<(), StateError> {
        self.with_locked_state(|state| {
            state.command_attempts_ms.push(timestamp_ms);
            Ok(())
        })
    }

    pub(crate) fn lock(&self) -> Result<MarketStreamLock, StateError> {
        self.validate_directories()?;
        MarketStreamLock::open(self, CONNECTION_LOCK_FILE, self.binding.connection_anchor)
    }

    #[cfg(feature = "test-support")]
    fn test_state_lock(&self) -> Result<MarketStreamLock, StateError> {
        self.validate_directories()?;
        MarketStreamLock::open(self, STATE_LOCK_FILE, self.binding.state_anchor)
    }

    fn state_guard(&self) -> Result<StateFileLock, StateError> {
        self.validate_directories()?;
        StateFileLock::open(self, STATE_LOCK_FILE, self.binding.state_anchor)
    }

    pub(crate) fn load(&self) -> Result<DurableWsState, StateError> {
        let _guard = self.state_guard()?;
        self.load_unlocked()
    }

    fn load_unlocked(&self) -> Result<DurableWsState, StateError> {
        let state = self.load_existing()?;
        self.validate_domain(&state)?;
        Ok(state)
    }

    fn load_existing(&self) -> Result<DurableWsState, StateError> {
        let file = match open_at(&self.directory, STATE_FILE, O_RDONLY, 0) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(StateError::PriorSessionUncertain);
            }
            Err(_) => return Err(StateError::UnsafePath),
        };
        validate_regular_descriptor(&file, self.state_uid, self.state_gid, STATE_FILE_MODE, true)?;
        let mut bytes = Vec::new();
        file.take(MAX_STATE_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| StateError::Io)?;
        if bytes.len() as u64 > MAX_STATE_FILE_BYTES {
            return Err(StateError::InvalidState);
        }
        if bytes.is_empty() {
            return Err(StateError::PriorSessionUncertain);
        }
        let state: DurableWsState =
            serde_json::from_slice(&bytes).map_err(|_| StateError::InvalidState)?;
        if !state.is_valid() {
            return Err(StateError::InvalidState);
        }
        Ok(state)
    }

    fn persist_new(&self, state: &DurableWsState) -> Result<(), StateError> {
        match open_at(&self.directory, STATE_FILE, O_RDONLY, 0) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            _ => return Err(StateError::PriorSessionUncertain),
        }
        self.write_atomic(state, false)
    }

    pub(crate) fn persist_existing(&self, state: &DurableWsState) -> Result<(), StateError> {
        let file = open_at(&self.directory, STATE_FILE, O_RDONLY, 0).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                StateError::PriorSessionUncertain
            } else {
                StateError::UnsafePath
            }
        })?;
        validate_regular_descriptor(&file, self.state_uid, self.state_gid, STATE_FILE_MODE, true)?;
        if file.metadata().map_err(|_| StateError::UnsafePath)?.len() == 0 {
            return Err(StateError::PriorSessionUncertain);
        }
        self.write_atomic(state, true)
    }

    fn write_atomic(
        &self,
        state: &DurableWsState,
        require_existing: bool,
    ) -> Result<(), StateError> {
        if !state.is_valid() {
            return Err(StateError::InvalidState);
        }
        let bytes = serde_json::to_vec(state).map_err(|_| StateError::Serialization)?;
        if bytes.len() as u64 > MAX_STATE_FILE_BYTES {
            return Err(StateError::InvalidState);
        }
        let temporary = format! {".approval-state-{}.tmp", Uuid::new_v4()};
        let mut file = open_at(
            &self.directory,
            &temporary,
            O_WRONLY | O_CREAT | O_EXCL,
            STATE_FILE_MODE,
        )
        .map_err(|_| StateError::Io)?;
        let write_result = (|| {
            file.write_all(&bytes).map_err(|_| StateError::Io)?;
            file.sync_all().map_err(|_| StateError::Io)?;
            validate_regular_descriptor(
                &file,
                self.state_uid,
                self.state_gid,
                STATE_FILE_MODE,
                true,
            )
        })();
        if let Err(error) = write_result {
            let _ = unlink_at(&self.directory, &temporary);
            return Err(error);
        }
        let target = open_at(&self.directory, STATE_FILE, O_RDONLY, 0);
        let target_is_valid = match target {
            Ok(file) if require_existing => {
                validate_regular_descriptor(
                    &file,
                    self.state_uid,
                    self.state_gid,
                    STATE_FILE_MODE,
                    true,
                )
                .is_ok()
                    && file.metadata().is_ok_and(|metadata| metadata.len() > 0)
            }
            Err(error) if !require_existing && error.kind() == io::ErrorKind::NotFound => true,
            _ => false,
        };
        if !target_is_valid {
            let _ = unlink_at(&self.directory, &temporary);
            return Err(StateError::PriorSessionUncertain);
        }
        if rename_at(&self.directory, &temporary, STATE_FILE).is_err() {
            let _ = unlink_at(&self.directory, &temporary);
            return Err(StateError::Io);
        }
        self.directory.sync_all().map_err(|_| StateError::Io)?;
        Ok(())
    }

    fn validate_directories(&self) -> Result<(), StateError> {
        validate_directory_descriptor(
            &self.directory,
            self.state_uid,
            self.state_gid,
            STATE_DIRECTORY_MODE,
            self.binding.state_directory,
        )?;
        validate_directory_descriptor(
            &self.anchor_directory,
            self.anchor_uid,
            self.anchor_gid,
            ANCHOR_DIRECTORY_MODE,
            self.binding.anchor_directory,
        )
    }

    pub(crate) fn with_locked_state<T>(
        &self,
        operation: impl FnOnce(&mut DurableWsState) -> Result<T, StateError>,
    ) -> Result<T, StateError> {
        let _guard = self.state_guard()?;
        let mut state = self.load_unlocked()?;
        let result = operation(&mut state)?;
        self.persist_existing(&state)?;
        Ok(result)
    }

    pub(crate) fn begin_connection_attempt(
        &self,
        now_ms: i64,
        reconnect: bool,
        policy: ReconnectPolicy,
    ) -> Result<ConnectionPermit, StateError> {
        let attempt_id = Uuid::new_v4();
        self.with_locked_state(|state| {
            if state.current_epoch.is_some()
                || state.connection_attempt_id.is_some()
                || state.pending_command.is_some()
            {
                return Err(StateError::PriorSessionUncertain);
            }
            // A successful first connection is free; every later public
            // connection, including a new client/process after a clean close,
            // consumes this one durable budget.
            let enforce_reconnect_budget = reconnect || state.has_connected;
            let reconnect_ordinal = if enforce_reconnect_budget {
                if state
                    .reconnect_next_allowed_at_ms
                    .is_some_and(|allowed| now_ms < allowed)
                {
                    return Err(StateError::ReconnectNotReady);
                }
                let mut budget = ReconnectBudget::from_attempts(
                    state.reconnect_attempts_10m_ms.iter().copied(),
                    state.reconnect_attempts_day_ms.iter().copied(),
                );
                budget.reserve(now_ms).map_err(StateError::Budget)?;
                state.reconnect_attempts_10m_ms = budget.attempts_10m().collect();
                state.reconnect_attempts_day_ms = budget.attempts_day().collect();
                let ordinal = state.reconnect_sequence.saturating_add(1);
                if ordinal == 0 {
                    return Err(StateError::InvalidState);
                }
                state.reconnect_sequence = ordinal;
                let delay = policy.delay_for_ordinal(ordinal, 0);
                state.reconnect_next_allowed_at_ms = Some(
                    now_ms.saturating_add(i64::try_from(delay.as_millis()).unwrap_or(i64::MAX)),
                );
                Some(ordinal)
            } else {
                None
            };
            state.connection_attempt_id = Some(attempt_id.to_string());
            state.connection_attempt_started_ms = Some(now_ms);
            let _ = reconnect_ordinal;
            Ok(ConnectionPermit { attempt_id })
        })
    }

    pub(crate) fn complete_connection(
        &self,
        attempt_id: Uuid,
        epoch: ConnectionEpoch,
    ) -> Result<(), StateError> {
        self.with_locked_state(|state| {
            if state.connection_attempt_id.as_deref() != Some(attempt_id.to_string().as_str()) {
                return Err(StateError::PriorSessionUncertain);
            }
            let started_at_ms = state
                .connection_attempt_started_ms
                .ok_or(StateError::InvalidState)?;
            let was_initial = !state.has_connected;
            state.connection_attempt_id = None;
            state.connection_attempt_started_ms = None;
            state.current_epoch = Some(epoch.uuid().to_string());
            state.has_connected = true;
            if was_initial {
                state.reconnect_next_allowed_at_ms =
                    Some(started_at_ms.saturating_add(ReconnectBudget::MIN_SPACING_MS));
            }
            Ok(())
        })
    }

    pub(crate) fn clear_connection_attempt(&self, attempt_id: Uuid) -> Result<(), StateError> {
        self.with_locked_state(|state| {
            if state.connection_attempt_id.as_deref() != Some(attempt_id.to_string().as_str()) {
                return Err(StateError::PriorSessionUncertain);
            }
            if state.current_epoch.is_some() || state.pending_command.is_some() {
                return Err(StateError::PriorSessionUncertain);
            }
            state.connection_attempt_id = None;
            state.connection_attempt_started_ms = None;
            Ok(())
        })
    }

    pub(crate) fn reserve_command(
        &self,
        epoch: ConnectionEpoch,
        operation: &str,
        symbol: &str,
        sent_at_ms: i64,
        deadline_ms: i64,
    ) -> Result<u64, StateError> {
        self.with_locked_state(|state| {
            if state.current_epoch.as_deref() != Some(epoch.uuid().to_string().as_str()) {
                return Err(StateError::PriorSessionUncertain);
            }
            if state.pending_command.is_some() {
                return Err(StateError::InvalidState);
            }
            state.next_command_ordinal = state
                .next_command_ordinal
                .checked_add(1)
                .ok_or(StateError::InvalidState)?;
            let ordinal = state.next_command_ordinal;
            state.pending_command = Some(DurablePendingCommand {
                epoch: epoch.uuid().to_string(),
                operation: operation.to_owned(),
                symbol: symbol.to_owned(),
                ordinal,
                sent_at_ms,
                deadline_ms,
            });
            Ok(ordinal)
        })
    }

    pub(crate) fn clear_pending_command(
        &self,
        epoch: ConnectionEpoch,
        ordinal: u64,
    ) -> Result<(), StateError> {
        self.with_locked_state(|state| {
            let Some(pending) = state.pending_command.as_ref() else {
                return Err(StateError::PriorSessionUncertain);
            };
            if pending.epoch != epoch.uuid().to_string() || pending.ordinal != ordinal {
                return Err(StateError::PriorSessionUncertain);
            }
            state.pending_command = None;
            Ok(())
        })
    }

    pub(crate) fn clear_clean_connection(&self, epoch: ConnectionEpoch) -> Result<(), StateError> {
        self.with_locked_state(|state| {
            if state.current_epoch.as_deref() != Some(epoch.uuid().to_string().as_str()) {
                return Err(StateError::PriorSessionUncertain);
            }
            if state.pending_command.is_some() {
                return Err(StateError::PriorSessionUncertain);
            }
            state.current_epoch = None;
            state.connection_attempt_id = None;
            state.connection_attempt_started_ms = None;
            Ok(())
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ConnectionPermit {
    pub(crate) attempt_id: Uuid,
}

pub struct MarketStreamLock {
    file: File,
}

impl fmt::Debug for MarketStreamLock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MarketStreamLock(<held>)")
    }
}

impl MarketStreamLock {
    fn open(
        store: &WsStateStore,
        name: &str,
        identity: ObjectIdentity,
    ) -> Result<Self, StateError> {
        let file = open_anchor(
            &store.anchor_directory,
            name,
            store.anchor_uid,
            store.anchor_gid,
        )?;
        validate_identity(&file, identity)?;
        if file.try_lock_exclusive().is_err() {
            return Err(StateError::LockBusy);
        }
        Ok(Self { file })
    }

    pub fn is_held(&self) -> bool {
        let _ = &self.file;
        true
    }
}

impl Drop for MarketStreamLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

struct StateFileLock {
    file: File,
}

impl StateFileLock {
    fn open(
        store: &WsStateStore,
        name: &str,
        identity: ObjectIdentity,
    ) -> Result<Self, StateError> {
        let file = open_anchor(
            &store.anchor_directory,
            name,
            store.anchor_uid,
            store.anchor_gid,
        )?;
        validate_identity(&file, identity)?;
        file.try_lock_exclusive()
            .map_err(|_| StateError::LockBusy)?;
        Ok(Self { file })
    }
}

impl Drop for StateFileLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

fn open_directory(path: &Path, uid: u32, gid: u32, mode: u32) -> Result<File, StateError> {
    let bytes = CString::new(path.as_os_str().as_bytes()).map_err(|_| StateError::UnsafePath)?;
    // SAFETY: `bytes` is a NUL-terminated path and the returned descriptor is
    // owned by the File below.
    let fd = unsafe {
        open(
            bytes.as_ptr(),
            O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC,
            0,
        )
    };
    if fd < 0 {
        return Err(StateError::UnsafePath);
    }
    // SAFETY: `open` returned a new descriptor owned by this function.
    let file = unsafe { File::from_raw_fd(fd) };
    let metadata = file.metadata().map_err(|_| StateError::UnsafePath)?;
    let path_metadata = fs_metadata(path)?;
    if !metadata.file_type().is_dir()
        || metadata.uid() != uid
        || metadata.gid() != gid
        || metadata.permissions().mode() & 0o7777 != mode
        || ObjectIdentity::from_metadata(&metadata) != ObjectIdentity::from_metadata(&path_metadata)
    {
        return Err(StateError::UnsafePath);
    }
    Ok(file)
}

fn open_at(directory: &File, name: &str, flags: i32, mode: u32) -> io::Result<File> {
    let name = CString::new(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    // SAFETY: `directory` is a live directory descriptor and `name` is one
    // NUL-terminated component.  O_NOFOLLOW prevents leaf symlink traversal.
    let fd = unsafe {
        openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            flags | O_NOFOLLOW | O_CLOEXEC,
            mode,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `openat` returned a new descriptor owned by this File.
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn validate_regular_descriptor(
    file: &File,
    expected_uid: u32,
    expected_gid: u32,
    expected_mode: u32,
    check_size: bool,
) -> Result<(), StateError> {
    let metadata = file.metadata().map_err(|_| StateError::UnsafePath)?;
    if !metadata.file_type().is_file()
        || metadata.uid() != expected_uid
        || metadata.gid() != expected_gid
        || metadata.nlink() != 1
        || metadata.permissions().mode() & 0o7777 != expected_mode
        || (check_size && metadata.len() > MAX_STATE_FILE_BYTES)
    {
        return Err(StateError::UnsafePath);
    }
    Ok(())
}

fn open_anchor(directory: &File, name: &str, uid: u32, gid: u32) -> Result<File, StateError> {
    let file = open_at(directory, name, O_RDONLY, 0).map_err(|_| StateError::UnsafePath)?;
    validate_regular_descriptor(&file, uid, gid, ANCHOR_FILE_MODE, false)?;
    if file.metadata().map_err(|_| StateError::UnsafePath)?.len() != 0 {
        return Err(StateError::UnsafePath);
    }
    Ok(file)
}

fn validate_identity(file: &File, expected: ObjectIdentity) -> Result<(), StateError> {
    let metadata = file.metadata().map_err(|_| StateError::UnsafePath)?;
    (ObjectIdentity::from_metadata(&metadata) == expected)
        .then_some(())
        .ok_or(StateError::UnsafePath)
}

fn validate_directory_descriptor(
    file: &File,
    uid: u32,
    gid: u32,
    mode: u32,
    expected: ObjectIdentity,
) -> Result<(), StateError> {
    let metadata = file.metadata().map_err(|_| StateError::UnsafePath)?;
    if !metadata.file_type().is_dir()
        || metadata.uid() != uid
        || metadata.gid() != gid
        || metadata.permissions().mode() & 0o7777 != mode
        || ObjectIdentity::from_metadata(&metadata) != expected
    {
        return Err(StateError::UnsafePath);
    }
    Ok(())
}

fn fs_metadata(path: &Path) -> Result<std::fs::Metadata, StateError> {
    std::fs::symlink_metadata(path).map_err(|_| StateError::UnsafePath)
}

#[cfg(any(feature = "test-support", test))]
fn provision_new_test_layout(state_dir: &Path, anchor_dir: &Path) -> Result<(), StateError> {
    provision_test_directory_exclusive(state_dir, STATE_DIRECTORY_MODE)?;
    provision_test_directory_exclusive(anchor_dir, ANCHOR_DIRECTORY_MODE)?;
    provision_test_anchor_exclusive(&anchor_dir.join(CONNECTION_LOCK_FILE))?;
    provision_test_anchor_exclusive(&anchor_dir.join(STATE_LOCK_FILE))
}

#[cfg(any(feature = "test-support", test))]
fn provision_test_directory_exclusive(path: &Path, mode: u32) -> Result<(), StateError> {
    fs::create_dir(path).map_err(|_| StateError::UnsafePath)?;
    fs::set_permissions(path, Permissions::from_mode(mode)).map_err(|_| StateError::Io)
}

#[cfg(any(feature = "test-support", test))]
fn provision_test_anchor_exclusive(path: &Path) -> Result<(), StateError> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(ANCHOR_FILE_MODE)
        .open(path)
        .map_err(|_| StateError::UnsafePath)?;
    file.set_permissions(Permissions::from_mode(ANCHOR_FILE_MODE))
        .map_err(|_| StateError::Io)?;
    file.sync_all().map_err(|_| StateError::Io)
}

fn rename_at(directory: &File, from: &str, to: &str) -> io::Result<()> {
    let from = CString::new(from).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let to = CString::new(to).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    // SAFETY: both names are single components relative to the same verified
    // directory descriptor.
    if unsafe {
        renameat(
            directory.as_raw_fd(),
            from.as_ptr(),
            directory.as_raw_fd(),
            to.as_ptr(),
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn unlink_at(directory: &File, name: &str) -> io::Result<()> {
    let name = CString::new(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    // SAFETY: `name` is one component in the verified directory.
    if unsafe { unlinkat(directory.as_raw_fd(), name.as_ptr(), 0) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Owner IDs for the one-shot production layout.  The fixture constructor is
/// only compiled into this crate's unit tests; no public API accepts owners.
#[cfg(feature = "market-stream-provisioning")]
#[derive(Clone, Copy)]
pub(crate) struct ProvisioningOwners {
    state_uid: u32,
    state_gid: u32,
    anchor_uid: u32,
    anchor_gid: u32,
    parent_uid: u32,
    parent_gid: u32,
}

#[cfg(feature = "market-stream-provisioning")]
impl ProvisioningOwners {
    pub(crate) const fn production() -> Self {
        Self {
            state_uid: PRODUCTION_STATE_UID,
            state_gid: PRODUCTION_STATE_GID,
            anchor_uid: PRODUCTION_ANCHOR_UID,
            anchor_gid: PRODUCTION_ANCHOR_GID,
            parent_uid: 0,
            parent_gid: PRODUCTION_STATE_GID,
        }
    }

    #[cfg(test)]
    pub(crate) fn fixture(uid: u32, gid: u32) -> Self {
        Self {
            state_uid: uid,
            state_gid: gid,
            anchor_uid: uid,
            anchor_gid: gid,
            parent_uid: uid,
            parent_gid: gid,
        }
    }
}

#[cfg(feature = "market-stream-provisioning")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProvisioningSyncPoint {
    StateDirectory,
    ParentAfterStateDirectory,
    AnchorDirectory,
    ParentAfterAnchorDirectory,
    ConnectionAnchor,
    StateAnchor,
    AnchorDirectoryAfterAnchors,
    ParentAfterAnchors,
    StateTemporaryFile,
    StateDirectoryAfterInstall,
}

#[cfg(all(test, feature = "market-stream-provisioning"))]
thread_local! {
    static PROVISIONING_FAIL_FSYNC_AT: std::cell::Cell<Option<ProvisioningSyncPoint>> = const { std::cell::Cell::new(None) };
    static PROVISIONING_RACE_INSTALL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(all(test, feature = "market-stream-provisioning"))]
pub(crate) fn fail_provisioning_fsync_at(point: Option<ProvisioningSyncPoint>) {
    PROVISIONING_FAIL_FSYNC_AT.with(|configured| configured.set(point));
}

#[cfg(all(test, feature = "market-stream-provisioning"))]
pub(crate) fn race_provisioning_install(enabled: bool) {
    PROVISIONING_RACE_INSTALL.with(|configured| configured.set(enabled));
}

#[cfg(feature = "market-stream-provisioning")]
fn provisioning_sync(file: &File, _point: ProvisioningSyncPoint) -> Result<(), StateError> {
    #[cfg(test)]
    if PROVISIONING_FAIL_FSYNC_AT.with(|configured| configured.get() == Some(_point)) {
        return Err(StateError::ProvisioningUncertain);
    }
    file.sync_all()
        .map_err(|_| StateError::ProvisioningUncertain)
}

/// Open the trusted `/run/lagrange` parent through no-follow directory
/// descriptors.  No caller-supplied path reaches this production helper.
#[cfg(feature = "market-stream-provisioning")]
pub(crate) fn open_production_provisioning_parent() -> Result<File, StateError> {
    let root = open_directory_component(Path::new("/"), None)?;
    validate_trusted_parent(&root, 0, 0)?;
    let run = open_directory_component(Path::new("run"), Some(&root))?;
    validate_trusted_parent(&run, 0, 0)?;
    let lagrange = open_directory_component(Path::new("lagrange"), Some(&run))?;
    validate_trusted_parent(&lagrange, 0, PRODUCTION_STATE_GID)?;
    Ok(lagrange)
}

#[cfg(feature = "market-stream-provisioning")]
fn open_directory_component(name: &Path, parent: Option<&File>) -> Result<File, StateError> {
    let file = match parent {
        Some(parent) => open_at(
            parent,
            name.to_str().ok_or(StateError::UnsafePath)?,
            O_RDONLY | O_DIRECTORY,
            0,
        ),
        None => {
            let bytes =
                CString::new(name.as_os_str().as_bytes()).map_err(|_| StateError::UnsafePath)?;
            // SAFETY: `bytes` is a NUL-terminated absolute path.  The opened
            // directory descriptor is transferred into the returned `File`.
            let fd = unsafe {
                open(
                    bytes.as_ptr(),
                    O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC,
                    0,
                )
            };
            if fd < 0 {
                return Err(StateError::UnsafePath);
            }
            // SAFETY: `open` returned a new descriptor owned by this function.
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }
    .map_err(|_| StateError::UnsafePath)?;
    if !file
        .metadata()
        .map_err(|_| StateError::UnsafePath)?
        .file_type()
        .is_dir()
    {
        return Err(StateError::UnsafePath);
    }
    Ok(file)
}

#[cfg(feature = "market-stream-provisioning")]
fn validate_trusted_parent(
    parent: &File,
    expected_uid: u32,
    expected_gid: u32,
) -> Result<(), StateError> {
    let metadata = parent.metadata().map_err(|_| StateError::UnsafePath)?;
    if !metadata.file_type().is_dir()
        || metadata.uid() != expected_uid
        || metadata.gid() != expected_gid
        || metadata.permissions().mode() & 0o022 != 0
    {
        return Err(StateError::UnsafePath);
    }
    Ok(())
}

#[cfg(all(test, feature = "market-stream-provisioning"))]
pub(crate) fn open_fixture_provisioning_parent(
    path: &Path,
    uid: u32,
    gid: u32,
) -> Result<File, StateError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| StateError::UnsafePath)?;
    let mode = metadata.permissions().mode() & 0o7777;
    let file = open_directory(path, uid, gid, mode)?;
    validate_trusted_parent(&file, uid, gid)?;
    Ok(file)
}

#[cfg(feature = "market-stream-provisioning")]
fn provisioning_input(slot: Uuid, generation: u64) -> Result<(), StateError> {
    if slot.is_nil() || generation == 0 {
        return Err(StateError::ProvisioningInputInvalid);
    }
    Ok(())
}

#[cfg(feature = "market-stream-provisioning")]
fn ensure_absent_at(parent: &File, name: &str) -> Result<(), StateError> {
    match open_at(parent, name, O_PATH, 0) {
        Ok(_existing) => Err(StateError::PriorSessionUncertain),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(StateError::UnsafePath),
    }
}

#[cfg(feature = "market-stream-provisioning")]
fn mkdir_at(parent: &File, name: &str, mode: u32) -> io::Result<()> {
    let name = CString::new(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    // SAFETY: `name` is one NUL-terminated child component of the live parent
    // descriptor; `mkdirat` does not follow a leaf symlink.
    if unsafe { mkdirat(parent.as_raw_fd(), name.as_ptr(), mode) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(feature = "market-stream-provisioning")]
fn set_new_object_owner_mode(file: &File, uid: u32, gid: u32, mode: u32) -> Result<(), StateError> {
    // SAFETY: the descriptor is open and refers only to an object created by
    // this fresh-layout invocation.
    if unsafe { fchown(file.as_raw_fd(), uid, gid) } != 0 {
        return Err(StateError::ProvisioningUncertain);
    }
    // SAFETY: the descriptor is open and refers only to an object created by
    // this fresh-layout invocation.
    if unsafe { fchmod(file.as_raw_fd(), mode) } != 0 {
        return Err(StateError::ProvisioningUncertain);
    }
    Ok(())
}

#[cfg(feature = "market-stream-provisioning")]
fn create_provisioning_directory(
    parent: &File,
    name: &str,
    uid: u32,
    gid: u32,
    mode: u32,
    directory_sync: ProvisioningSyncPoint,
    parent_sync: ProvisioningSyncPoint,
) -> Result<File, StateError> {
    mkdir_at(parent, name, mode).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            StateError::PriorSessionUncertain
        } else {
            StateError::Io
        }
    })?;
    let directory = open_at(parent, name, O_RDONLY | O_DIRECTORY, 0)
        .map_err(|_| StateError::ProvisioningUncertain)?;
    set_new_object_owner_mode(&directory, uid, gid, mode)?;
    validate_directory_descriptor(
        &directory,
        uid,
        gid,
        mode,
        ObjectIdentity::from_metadata(
            &directory
                .metadata()
                .map_err(|_| StateError::ProvisioningUncertain)?,
        ),
    )
    .map_err(|_| StateError::ProvisioningUncertain)?;
    provisioning_sync(&directory, directory_sync)?;
    provisioning_sync(parent, parent_sync)?;
    Ok(directory)
}

#[cfg(feature = "market-stream-provisioning")]
fn create_provisioning_anchor(
    directory: &File,
    name: &str,
    uid: u32,
    gid: u32,
    sync_point: ProvisioningSyncPoint,
) -> Result<File, StateError> {
    let file = open_at(
        directory,
        name,
        O_WRONLY | O_CREAT | O_EXCL,
        ANCHOR_FILE_MODE,
    )
    .map_err(|_| StateError::ProvisioningUncertain)?;
    set_new_object_owner_mode(&file, uid, gid, ANCHOR_FILE_MODE)?;
    validate_regular_descriptor(&file, uid, gid, ANCHOR_FILE_MODE, false)
        .map_err(|_| StateError::ProvisioningUncertain)?;
    let metadata = file
        .metadata()
        .map_err(|_| StateError::ProvisioningUncertain)?;
    if metadata.len() != 0 || metadata.nlink() != 1 {
        return Err(StateError::ProvisioningUncertain);
    }
    provisioning_sync(&file, sync_point)?;
    Ok(file)
}

#[cfg(feature = "market-stream-provisioning")]
fn rename_at_noreplace(directory: &File, from: &str, to: &str) -> io::Result<()> {
    let from = CString::new(from).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let to = CString::new(to).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    #[cfg(test)]
    if PROVISIONING_RACE_INSTALL.with(std::cell::Cell::get) {
        let mut raced = open_at(
            directory,
            to.to_str().unwrap_or_default(),
            O_WRONLY | O_CREAT | O_EXCL,
            0o600,
        )?;
        raced
            .write_all(b"race")
            .map_err(|_| io::Error::other("install race fixture failed"))?;
        raced.sync_all()?;
    }
    // SAFETY: both names are one NUL-terminated component relative to the
    // same verified directory; RENAME_NOREPLACE refuses a raced destination.
    if unsafe {
        renameat2(
            directory.as_raw_fd(),
            from.as_ptr(),
            directory.as_raw_fd(),
            to.as_ptr(),
            RENAME_NOREPLACE,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(feature = "market-stream-provisioning")]
fn verify_provisioning_directory_name(
    parent: &File,
    name: &str,
    expected: ObjectIdentity,
) -> Result<(), StateError> {
    let object = open_at(parent, name, O_PATH, 0).map_err(|_| StateError::ProvisioningUncertain)?;
    let metadata = object
        .metadata()
        .map_err(|_| StateError::ProvisioningUncertain)?;
    if !metadata.file_type().is_dir() || ObjectIdentity::from_metadata(&metadata) != expected {
        return Err(StateError::ProvisioningUncertain);
    }
    Ok(())
}

#[cfg(feature = "market-stream-provisioning")]
fn provisioning_store_at(
    parent: &File,
    slot: Uuid,
    owners: ProvisioningOwners,
) -> Result<WsStateStore, StateError> {
    validate_trusted_parent(parent, owners.parent_uid, owners.parent_gid)?;
    let directory =
        open_at(parent, PRODUCTION_STATE_LEAF, O_RDONLY | O_DIRECTORY, 0).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                StateError::PriorSessionUncertain
            } else {
                StateError::UnsafePath
            }
        })?;
    let anchor_directory = open_at(parent, PRODUCTION_ANCHOR_LEAF, O_RDONLY | O_DIRECTORY, 0)
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                StateError::PriorSessionUncertain
            } else {
                StateError::UnsafePath
            }
        })?;
    validate_directory_descriptor(
        &directory,
        owners.state_uid,
        owners.state_gid,
        STATE_DIRECTORY_MODE,
        ObjectIdentity::from_metadata(&directory.metadata().map_err(|_| StateError::UnsafePath)?),
    )?;
    validate_directory_descriptor(
        &anchor_directory,
        owners.anchor_uid,
        owners.anchor_gid,
        ANCHOR_DIRECTORY_MODE,
        ObjectIdentity::from_metadata(
            &anchor_directory
                .metadata()
                .map_err(|_| StateError::UnsafePath)?,
        ),
    )?;
    let state_directory_identity =
        ObjectIdentity::from_metadata(&directory.metadata().map_err(|_| StateError::UnsafePath)?);
    let anchor_directory_identity = ObjectIdentity::from_metadata(
        &anchor_directory
            .metadata()
            .map_err(|_| StateError::UnsafePath)?,
    );
    if state_directory_identity == anchor_directory_identity {
        return Err(StateError::UnsafePath);
    }
    let connection = open_anchor(
        &anchor_directory,
        CONNECTION_LOCK_FILE,
        owners.anchor_uid,
        owners.anchor_gid,
    )?;
    let state_anchor = open_anchor(
        &anchor_directory,
        STATE_LOCK_FILE,
        owners.anchor_uid,
        owners.anchor_gid,
    )?;
    let connection_identity =
        ObjectIdentity::from_metadata(&connection.metadata().map_err(|_| StateError::UnsafePath)?);
    let state_anchor_identity = ObjectIdentity::from_metadata(
        &state_anchor
            .metadata()
            .map_err(|_| StateError::UnsafePath)?,
    );
    if connection_identity == state_anchor_identity {
        return Err(StateError::UnsafePath);
    }
    let binding = DomainBinding {
        state_directory: state_directory_identity,
        anchor_directory: anchor_directory_identity,
        connection_anchor: connection_identity,
        state_anchor: state_anchor_identity,
    };
    Ok(WsStateStore {
        dir: PathBuf::from(PRODUCTION_STATE_DIRECTORY),
        directory: Arc::new(directory),
        anchor_directory: Arc::new(anchor_directory),
        credential_slot_id: slot,
        binding,
        state_uid: owners.state_uid,
        state_gid: owners.state_gid,
        anchor_uid: owners.anchor_uid,
        anchor_gid: owners.anchor_gid,
    })
}

#[cfg(feature = "market-stream-provisioning")]
fn validate_provisioning_store(
    store: &WsStateStore,
    slot: Uuid,
    generation: u64,
) -> Result<(), StateError> {
    store.validate_directories()?;
    let state = store.load_unlocked()?;
    store.validate_domain(&state)?;
    if state.credential_slot_id != slot.to_string()
        || state.credential_generation != generation.to_string()
    {
        return Err(StateError::InvalidState);
    }
    Ok(())
}

#[cfg(feature = "market-stream-provisioning")]
pub(crate) fn validate_provisioning_at(
    parent: &File,
    slot: Uuid,
    generation: u64,
    owners: ProvisioningOwners,
) -> Result<(), StateError> {
    provisioning_input(slot, generation)?;
    let store = provisioning_store_at(parent, slot, owners)?;
    let _connection = store.lock()?;
    let _state = store.state_guard()?;
    validate_provisioning_store(&store, slot, generation)
}

#[cfg(feature = "market-stream-provisioning")]
pub(crate) fn initialize_provisioning_at(
    parent: &File,
    slot: Uuid,
    generation: u64,
    owners: ProvisioningOwners,
) -> Result<(), StateError> {
    provisioning_input(slot, generation)?;
    validate_trusted_parent(parent, owners.parent_uid, owners.parent_gid)?;
    ensure_absent_at(parent, PRODUCTION_STATE_LEAF)?;
    ensure_absent_at(parent, PRODUCTION_ANCHOR_LEAF)?;

    let state_directory = create_provisioning_directory(
        parent,
        PRODUCTION_STATE_LEAF,
        owners.state_uid,
        owners.state_gid,
        STATE_DIRECTORY_MODE,
        ProvisioningSyncPoint::StateDirectory,
        ProvisioningSyncPoint::ParentAfterStateDirectory,
    )?;
    let anchor_directory = create_provisioning_directory(
        parent,
        PRODUCTION_ANCHOR_LEAF,
        owners.anchor_uid,
        owners.anchor_gid,
        ANCHOR_DIRECTORY_MODE,
        ProvisioningSyncPoint::AnchorDirectory,
        ProvisioningSyncPoint::ParentAfterAnchorDirectory,
    )
    .map_err(|_| StateError::ProvisioningUncertain)?;

    let connection = create_provisioning_anchor(
        &anchor_directory,
        CONNECTION_LOCK_FILE,
        owners.anchor_uid,
        owners.anchor_gid,
        ProvisioningSyncPoint::ConnectionAnchor,
    )?;
    let state_anchor = create_provisioning_anchor(
        &anchor_directory,
        STATE_LOCK_FILE,
        owners.anchor_uid,
        owners.anchor_gid,
        ProvisioningSyncPoint::StateAnchor,
    )?;
    let connection_identity = ObjectIdentity::from_metadata(
        &connection
            .metadata()
            .map_err(|_| StateError::ProvisioningUncertain)?,
    );
    let state_anchor_identity = ObjectIdentity::from_metadata(
        &state_anchor
            .metadata()
            .map_err(|_| StateError::ProvisioningUncertain)?,
    );
    if connection_identity == state_anchor_identity {
        return Err(StateError::ProvisioningUncertain);
    }
    provisioning_sync(
        &anchor_directory,
        ProvisioningSyncPoint::AnchorDirectoryAfterAnchors,
    )?;
    provisioning_sync(parent, ProvisioningSyncPoint::ParentAfterAnchors)?;

    let store = provisioning_store_at(parent, slot, owners)
        .map_err(|_| StateError::ProvisioningUncertain)?;
    let _connection_lock = store
        .lock()
        .map_err(|_| StateError::ProvisioningUncertain)?;
    let _state_lock = store
        .state_guard()
        .map_err(|_| StateError::ProvisioningUncertain)?;
    let state = DurableWsState {
        schema: STATE_SCHEMA.to_owned(),
        credential_slot_id: slot.to_string(),
        domain_binding: store.binding.clone(),
        credential_generation: generation.to_string(),
        ..DurableWsState::default()
    };
    if !state.is_valid() {
        return Err(StateError::ProvisioningUncertain);
    }
    let bytes = serde_json::to_vec(&state).map_err(|_| StateError::ProvisioningUncertain)?;
    if bytes.len() as u64 > MAX_STATE_FILE_BYTES {
        return Err(StateError::ProvisioningUncertain);
    }
    let temporary = format!(".approval-state-{}.provision.tmp", Uuid::new_v4());
    let mut file = open_at(
        &store.directory,
        &temporary,
        O_WRONLY | O_CREAT | O_EXCL,
        STATE_FILE_MODE,
    )
    .map_err(|_| StateError::ProvisioningUncertain)?;
    set_new_object_owner_mode(&file, owners.state_uid, owners.state_gid, STATE_FILE_MODE)?;
    file.write_all(&bytes)
        .map_err(|_| StateError::ProvisioningUncertain)?;
    validate_regular_descriptor(
        &file,
        owners.state_uid,
        owners.state_gid,
        STATE_FILE_MODE,
        true,
    )
    .map_err(|_| StateError::ProvisioningUncertain)?;
    let temporary_metadata = file
        .metadata()
        .map_err(|_| StateError::ProvisioningUncertain)?;
    if temporary_metadata.len() != bytes.len() as u64 || temporary_metadata.nlink() != 1 {
        return Err(StateError::ProvisioningUncertain);
    }
    provisioning_sync(&file, ProvisioningSyncPoint::StateTemporaryFile)?;
    drop(file);
    ensure_absent_at(&store.directory, STATE_FILE)
        .map_err(|_| StateError::ProvisioningUncertain)?;
    rename_at_noreplace(&store.directory, &temporary, STATE_FILE)
        .map_err(|_| StateError::ProvisioningUncertain)?;
    provisioning_sync(
        &store.directory,
        ProvisioningSyncPoint::StateDirectoryAfterInstall,
    )?;

    verify_provisioning_directory_name(
        parent,
        PRODUCTION_STATE_LEAF,
        ObjectIdentity::from_metadata(
            &state_directory
                .metadata()
                .map_err(|_| StateError::ProvisioningUncertain)?,
        ),
    )?;
    verify_provisioning_directory_name(
        parent,
        PRODUCTION_ANCHOR_LEAF,
        ObjectIdentity::from_metadata(
            &anchor_directory
                .metadata()
                .map_err(|_| StateError::ProvisioningUncertain)?,
        ),
    )?;
    let reopened = provisioning_store_at(parent, slot, owners)
        .map_err(|_| StateError::ProvisioningUncertain)?;
    validate_provisioning_store(&reopened, slot, generation)
        .map_err(|_| StateError::ProvisioningUncertain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::{Clock, TestClock};

    #[cfg(feature = "market-stream-provisioning")]
    #[test]
    fn runtime_validation_branch_never_creates_a_missing_layout() {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), Permissions::from_mode(0o700)).unwrap();
        let metadata = fs::metadata(root.path()).unwrap();
        let state_dir = root.path().join("kis-market-stream");
        let anchor_dir = root.path().join("kis-market-stream-locks");
        assert!(
            MarketStreamDomain::open_verified(
                state_dir.clone(),
                anchor_dir.clone(),
                Uuid::new_v4(),
                metadata.uid(),
                metadata.gid(),
                metadata.uid(),
                metadata.gid(),
                false,
            )
            .is_err()
        );
        assert!(!state_dir.exists());
        assert!(!anchor_dir.exists());
    }

    #[test]
    fn stale_absence_observer_cannot_claim_an_existing_synthetic_layout() {
        let root = tempfile::tempdir().unwrap();
        let state_dir = root.path().join("kis-market-stream");
        let anchor_dir = root.path().join("kis-market-stream-locks");
        provision_new_test_layout(&state_dir, &anchor_dir).unwrap();
        let connection = fs::metadata(anchor_dir.join(CONNECTION_LOCK_FILE)).unwrap();
        let state = fs::metadata(anchor_dir.join(STATE_LOCK_FILE)).unwrap();

        assert_eq!(
            provision_new_test_layout(&state_dir, &anchor_dir).unwrap_err(),
            StateError::UnsafePath
        );
        assert_eq!(
            provision_test_anchor_exclusive(&anchor_dir.join(CONNECTION_LOCK_FILE)).unwrap_err(),
            StateError::UnsafePath
        );
        assert!(!state_dir.join(STATE_FILE).exists());
        let connection_after = fs::metadata(anchor_dir.join(CONNECTION_LOCK_FILE)).unwrap();
        let state_after = fs::metadata(anchor_dir.join(STATE_LOCK_FILE)).unwrap();
        assert_eq!(
            (connection.dev(), connection.ino()),
            (connection_after.dev(), connection_after.ino())
        );
        assert_eq!(
            (state.dev(), state.ino()),
            (state_after.dev(), state_after.ino())
        );
    }

    #[test]
    fn command_budget_uses_source_limits_without_sleeping() {
        let mut budget = CommandBudget::default();
        assert!(budget.reserve(1_000).is_ok());
        assert_eq!(budget.reserve(1_999), Err(BudgetError::MinimumSpacing));
        assert!(budget.reserve(2_000).is_ok());
        assert_eq!(budget.len(), 2);
    }

    #[test]
    fn command_rolling_limit_expires_without_erasing_daily_history() {
        let mut budget = CommandBudget::default();
        for index in 0..CommandBudget::ROLLING_LIMIT {
            assert!(
                budget
                    .reserve(index as i64 * CommandBudget::MIN_SPACING_MS)
                    .is_ok()
            );
        }
        let next = CommandBudget::ROLLING_LIMIT as i64 * CommandBudget::MIN_SPACING_MS;
        assert_eq!(budget.reserve(next), Err(BudgetError::RollingWindow));
        assert!(
            budget
                .reserve(CommandBudget::ROLLING_WINDOW_MS + next)
                .is_ok()
        );
        assert_eq!(budget.len(), CommandBudget::ROLLING_LIMIT + 1);
    }

    #[test]
    fn reconnect_policy_is_10_20_40_60_with_bounded_positive_jitter() {
        let policy = ReconnectPolicy::default();
        assert_eq!(policy.delay_for_ordinal(1, 0), Duration::from_secs(10));
        assert_eq!(policy.delay_for_ordinal(2, 0), Duration::from_secs(20));
        assert_eq!(policy.delay_for_ordinal(3, 0), Duration::from_secs(40));
        assert_eq!(policy.delay_for_ordinal(4, 0), Duration::from_secs(60));
        assert_eq!(policy.delay_for_ordinal(5, 20), Duration::from_secs(72));
    }

    #[test]
    fn state_is_atomic_and_reloads_attempt_history() {
        let directory = tempfile::tempdir().unwrap();
        let domain = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
        let store = domain.state().clone();
        store
            .with_locked_state(|state| {
                state.credential_generation = "7".into();
                state.command_attempts_ms.push(100);
                Ok(())
            })
            .unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.credential_generation, "7");
        assert_eq!(loaded.command_attempts_ms, vec![100]);
        assert_eq!(
            fs::metadata(domain.state_directory())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(domain.state_directory().join(STATE_FILE))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let clock = TestClock::at(100);
        assert_eq!(clock.now_ms(), 100);
    }

    #[test]
    fn synthetic_anchors_keep_exact_mode_under_restrictive_umask() {
        const CHILD_MARKER: &str = "KIS_MARKET_STREAM_RESTRICTIVE_UMASK_CHILD";

        if std::env::var_os(CHILD_MARKER).as_deref() != Some(std::ffi::OsStr::new("child")) {
            let executable = std::env::current_exe().unwrap();
            let status = std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg("umask 077; exec \"$@\"")
                .arg("sh")
                .arg(executable)
                .arg("--exact")
                .arg("market_stream_state::tests::synthetic_anchors_keep_exact_mode_under_restrictive_umask")
                .arg("--test-threads=1")
                .env_clear()
                .env(CHILD_MARKER, "child")
                .status()
                .unwrap();
            assert!(status.success(), "restrictive-umask child failed: {status}");
            return;
        }

        let directory = tempfile::tempdir().unwrap();
        let slot = Uuid::new_v4();
        let domain = MarketStreamDomain::for_test(directory.path(), slot).unwrap();
        let anchor_directory = directory.path().join("kis-market-stream-locks");

        for anchor_name in [CONNECTION_LOCK_FILE, STATE_LOCK_FILE] {
            let metadata = fs::symlink_metadata(anchor_directory.join(anchor_name)).unwrap();
            assert!(metadata.file_type().is_file());
            assert_eq!(metadata.nlink(), 1);
            assert_eq!(metadata.permissions().mode() & 0o7777, ANCHOR_FILE_MODE);
        }

        let store = domain.state();
        store
            .with_locked_state(|state| {
                state.command_attempts_ms.push(100);
                Ok(())
            })
            .unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.credential_slot_id, slot.to_string());
        assert_eq!(loaded.command_attempts_ms, vec![100]);
    }

    #[test]
    fn lock_is_exclusive_inside_one_process() {
        let directory = tempfile::tempdir().unwrap();
        let domain = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
        let store = domain.state().clone();
        let first = store.lock().unwrap();
        assert!(first.is_held());
        assert_eq!(store.lock().unwrap_err(), StateError::LockBusy);
    }

    #[test]
    fn symlinked_state_paths_fail_closed() {
        let directory = tempfile::tempdir().unwrap();
        let domain = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
        let store = domain.state().clone();
        let target = directory.path().join("state-target.json");
        fs::write(&target, br#"{"schema":"kis-market-stream-state-v1"}"#).unwrap();
        fs::remove_file(domain.state_directory().join(STATE_FILE)).unwrap();
        std::os::unix::fs::symlink(&target, domain.state_directory().join(STATE_FILE)).unwrap();
        assert!(matches!(store.load(), Err(StateError::UnsafePath)));
    }

    #[test]
    fn slot_mismatch_fails_without_mutating_state() {
        let directory = tempfile::tempdir().unwrap();
        let first = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
        let before = fs::read(first.state_directory().join(STATE_FILE)).unwrap();
        assert!(matches!(
            MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()),
            Err(StateError::InvalidState)
        ));
        assert_eq!(
            before,
            fs::read(first.state_directory().join(STATE_FILE)).unwrap()
        );
    }

    #[test]
    fn reconnect_backoff_is_durable_and_not_caller_supplied() {
        let directory = tempfile::tempdir().unwrap();
        let domain = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
        let store = domain.state().clone();
        let policy = ReconnectPolicy::default();
        let initial = store
            .begin_connection_attempt(1_000, false, policy)
            .unwrap();
        let first_epoch = ConnectionEpoch::new();
        store
            .complete_connection(initial.attempt_id, first_epoch)
            .unwrap();
        store.clear_clean_connection(first_epoch).unwrap();

        let first = store
            .begin_connection_attempt(11_000, true, policy)
            .unwrap();
        let reconnect_epoch = ConnectionEpoch::new();
        store
            .complete_connection(first.attempt_id, reconnect_epoch)
            .unwrap();
        store.clear_clean_connection(reconnect_epoch).unwrap();
        assert!(matches!(
            store.begin_connection_attempt(11_001, false, policy),
            Err(StateError::ReconnectNotReady)
        ));

        let second = store
            .begin_connection_attempt(21_000, false, policy)
            .unwrap();
        assert_ne!(second.attempt_id, first.attempt_id);
        let loaded = store.load().unwrap();
        assert_eq!(loaded.reconnect_sequence, 2);
        assert_eq!(loaded.reconnect_next_allowed_at_ms, Some(41_000));
    }

    #[test]
    fn clean_public_reconnect_still_uses_durable_budget() {
        let directory = tempfile::tempdir().unwrap();
        let domain = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
        let store = domain.state().clone();
        let policy = ReconnectPolicy::default();
        let initial = store
            .begin_connection_attempt(100_000, false, policy)
            .unwrap();
        let epoch = ConnectionEpoch::new();
        store
            .complete_connection(initial.attempt_id, epoch)
            .unwrap();
        store.clear_clean_connection(epoch).unwrap();
        assert_eq!(
            store
                .begin_connection_attempt(100_000, false, policy)
                .unwrap_err(),
            StateError::ReconnectNotReady,
            "a new public client/process cannot turn a clean reconnect into an initial connect"
        );
    }
}
