//! Process-shared coordination for KIS read-only calls.
//!
//! This module is deliberately disconnected from the existing live clients.
//! WP-2B can compose it around token and read callbacks after its constructor
//! compatibility gate. Nothing here reads credentials, environment variables,
//! provider state, or a production path implicitly.

use std::collections::BTreeMap;
use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::future::Future;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use fs2::FileExt;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::auth::{AccessToken, DEFAULT_REFRESH_MARGIN_MS, MIN_ISSUE_INTERVAL_MS, TokenIssuer};
use crate::clock::Clock;
use crate::secret::Secret;

/// Intended production owner for the protected runtime state directory.
pub const PRODUCTION_EXPECTED_UID: u32 = 10_001;
pub const LOCK_FILE_NAME: &str = "coordination.lock";
pub const STATE_FILE_NAME: &str = "state-v1.json";

const ROOT_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;
const STATE_SCHEMA_VERSION: u32 = 1;
const MAX_STATE_BYTES: u64 = 64 * 1024;
const GLOBAL_READ_INTERVAL_MS: i64 = 1_000;
const CHANNEL_READ_INTERVAL_MS: i64 = 1_000;
const INTRADAY_READ_INTERVAL_MS: i64 = 5_000;
const INTRADAY_DAILY_LIMIT: u32 = 5_000;
const MAX_LOCK_WAIT: Duration = Duration::from_secs(30);
const MAX_REQUEST_TIMEOUT_MS: i64 = 5 * 60 * 1_000;
const MAX_TOKEN_FUTURE_MS: i64 = 48 * 60 * 60 * 1_000;
// Last millisecond of year 9999 UTC. This keeps canonical civil dates at the
// fixed YYYY-MM-DD width and rejects corrupt far-future clocks.
const MAX_SUPPORTED_WALL_MS: i64 = 253_402_300_799_999;
const DOMAIN: &[u8] = b"lagrange-kis-read-v1\0";

/// The exact nine read-only endpoint/TR pairs duplicated from
/// `market_data::READ_ONLY_CHANNELS` for WP-2A isolation. WP-2B should
/// consolidate the two definitions when it wires this primitive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReadChannel {
    DailyItemChartPrice,
    InquirePrice,
    CheckHoliday,
    PaidInCapitalIncrease,
    BonusIssue,
    Dividend,
    MergerSplit,
    ReverseSplit,
    CapitalDecrease,
}

impl ReadChannel {
    pub const ALL: [Self; 9] = [
        Self::DailyItemChartPrice,
        Self::InquirePrice,
        Self::CheckHoliday,
        Self::PaidInCapitalIncrease,
        Self::BonusIssue,
        Self::Dividend,
        Self::MergerSplit,
        Self::ReverseSplit,
        Self::CapitalDecrease,
    ];

    pub const fn pair(self) -> (&'static str, &'static str) {
        match self {
            Self::DailyItemChartPrice => (
                "/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice",
                "FHKST03010100",
            ),
            Self::InquirePrice => (
                "/uapi/domestic-stock/v1/quotations/inquire-price",
                "FHKST01010100",
            ),
            Self::CheckHoliday => (
                "/uapi/domestic-stock/v1/quotations/chk-holiday",
                "CTCA0903R",
            ),
            Self::PaidInCapitalIncrease => (
                "/uapi/domestic-stock/v1/ksdinfo/paidin-capin",
                "HHKDB669100C0",
            ),
            Self::BonusIssue => (
                "/uapi/domestic-stock/v1/ksdinfo/bonus-issue",
                "HHKDB669101C0",
            ),
            Self::Dividend => ("/uapi/domestic-stock/v1/ksdinfo/dividend", "HHKDB669102C0"),
            Self::MergerSplit => (
                "/uapi/domestic-stock/v1/ksdinfo/merger-split",
                "HHKDB669104C0",
            ),
            Self::ReverseSplit => ("/uapi/domestic-stock/v1/ksdinfo/rev-split", "HHKDB669105C0"),
            Self::CapitalDecrease => ("/uapi/domestic-stock/v1/ksdinfo/cap-dcrs", "HHKDB669106C0"),
        }
    }

    pub fn from_pair(path: &str, tr_id: &str) -> Result<Self, ReadCoordinationError> {
        Self::ALL
            .into_iter()
            .find(|channel| channel.pair() == (path, tr_id))
            .ok_or_else(|| ReadCoordinationError::UnsupportedChannel {
                path: path.to_owned(),
                tr_id: tr_id.to_owned(),
            })
    }

    const fn state_key(self) -> &'static str {
        match self {
            Self::DailyItemChartPrice => "daily-item-chart-price/FHKST03010100",
            Self::InquirePrice => "inquire-price/FHKST01010100",
            Self::CheckHoliday => "check-holiday/CTCA0903R",
            Self::PaidInCapitalIncrease => "paid-in-capital/HHKDB669100C0",
            Self::BonusIssue => "bonus-issue/HHKDB669101C0",
            Self::Dividend => "dividend/HHKDB669102C0",
            Self::MergerSplit => "merger-split/HHKDB669104C0",
            Self::ReverseSplit => "reverse-split/HHKDB669105C0",
            Self::CapitalDecrease => "capital-decrease/HHKDB669106C0",
        }
    }

    const fn is_intraday(self) -> bool {
        matches!(self, Self::InquirePrice)
    }
}

/// Explicit credential values and their monotonically increasing generation.
pub struct ReadCredentials {
    app_key: Secret<String>,
    app_secret: Secret<String>,
    generation: u64,
}

impl ReadCredentials {
    pub fn new(
        app_key: Secret<String>,
        app_secret: Secret<String>,
        generation: u64,
    ) -> Result<Self, ReadCoordinationError> {
        if generation == 0
            || app_key.expose().trim().is_empty()
            || app_secret.expose().trim().is_empty()
        {
            return Err(ReadCoordinationError::InvalidCredentials);
        }
        Ok(Self {
            app_key,
            app_secret,
            generation,
        })
    }

    fn verifier(&self) -> [u8; 32] {
        let mut mac = Hmac::<Sha256>::new_from_slice(self.app_secret.expose().trim().as_bytes())
            .expect("HMAC accepts keys of every size");
        mac.update(DOMAIN);
        mac.update(self.app_key.expose().trim().as_bytes());
        mac.finalize().into_bytes().into()
    }
}

impl std::fmt::Debug for ReadCredentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReadCredentials")
            .field("app_key", &self.app_key)
            .field("app_secret", &self.app_secret)
            .field("generation", &self.generation)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct ReadCoordinationConfig {
    state_root: PathBuf,
    expected_uid: u32,
}

impl ReadCoordinationConfig {
    pub fn new(state_root: impl Into<PathBuf>, expected_uid: u32) -> Self {
        Self {
            state_root: state_root.into(),
            expected_uid,
        }
    }

    pub fn state_root(&self) -> &Path {
        &self.state_root
    }
}

/// Quote work never waits. Blocking EOD/manual work polls `flock` away from
/// the Tokio reactor and is bounded by a real monotonic deadline.
#[derive(Debug, Clone, Copy)]
pub enum LockAcquisition {
    NonBlocking,
    Bounded(Duration),
}

/// A provider callback result with no free-form provider text.
pub enum ReadCallbackResult<T> {
    Success(T),
    Unauthorized,
    RateLimited { retry_after: Duration },
    CompletedFailure(ReadFailureKind),
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadFailureKind {
    Transport,
    Timeout,
    ProviderUnavailable,
    ResponseInvalid,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReadCoordinationError {
    #[error("KIS read channel is not allowed: {path} ({tr_id})")]
    UnsupportedChannel { path: String, tr_id: String },
    #[error("KIS read coordination credentials are invalid")]
    InvalidCredentials,
    #[error("KIS read credential scope does not match shared state")]
    CredentialScopeMismatch,
    #[error("KIS read credential generation is stale")]
    StaleCredentialGeneration,
    #[error("KIS read coordination root is unavailable")]
    RootUnavailable,
    #[error("KIS read coordination root has unsafe ownership, type, or mode")]
    UnsafeRoot,
    #[error("KIS read coordination file has unsafe ownership, type, mode, or link count")]
    UnsafeFile,
    #[error("KIS read coordination state is corrupt or has an unknown schema")]
    CorruptState,
    #[error("KIS read coordination state is not canonical")]
    NonCanonicalState,
    #[error("KIS read coordination state exceeds its size bound")]
    StateTooLarge,
    #[doc(hidden)]
    #[error("KIS read coordination state is not initialized")]
    StateMissing,
    #[doc(hidden)]
    #[error("KIS read coordination lock is not initialized")]
    LockMissing,
    #[doc(hidden)]
    #[error("KIS read coordination lock initialization raced")]
    LockAlreadyExists,
    #[doc(hidden)]
    #[error("KIS read coordination lock acquisition was cancelled")]
    AcquisitionCancelled,
    #[error("KIS read coordination wall clock moved backward")]
    ClockRollback,
    #[error("KIS read coordination state contains an invalid or future value")]
    InvalidStateTime,
    #[error("KIS read coordination lock is busy")]
    LockBusy,
    #[error("KIS read coordination lock acquisition timed out")]
    LockTimeout,
    #[error("KIS read coordination lock wait is outside the supported bound")]
    InvalidLockWait,
    #[error("KIS read callback deadline is outside the supported bound")]
    InvalidRequestTimeout,
    #[error("an earlier KIS read reservation is still active ({retry_after_ms}ms remaining)")]
    ReservationActive { retry_after_ms: u64 },
    #[error("KIS token issue cooldown is active ({retry_after_ms}ms remaining)")]
    TokenIssueCooldown { retry_after_ms: u64 },
    #[error("KIS token issuer failed")]
    TokenIssueFailed,
    #[error("KIS token issuer returned an unusable token")]
    UnusableToken,
    #[error("KIS broker cooldown is active ({retry_after_ms}ms remaining)")]
    BrokerCooldown { retry_after_ms: u64 },
    #[error("KIS global read spacing is active ({retry_after_ms}ms remaining)")]
    GlobalSpacing { retry_after_ms: u64 },
    #[error("KIS channel read spacing is active ({retry_after_ms}ms remaining)")]
    ChannelSpacing { retry_after_ms: u64 },
    #[error("KIS intraday read spacing is active ({retry_after_ms}ms remaining)")]
    IntradaySpacing { retry_after_ms: u64 },
    #[error("KIS intraday daily attempt budget is exhausted")]
    IntradayBudgetExhausted,
    #[error("KIS read callback returned unauthorized")]
    Unauthorized,
    #[error("KIS read callback was rate limited ({retry_after_ms}ms)")]
    CallbackRateLimited { retry_after_ms: u64 },
    #[error("KIS read callback completed with {kind:?}")]
    CallbackFailed { kind: ReadFailureKind },
    #[error("KIS read callback outcome is ambiguous")]
    CallbackAmbiguous,
    #[error("KIS read coordination I/O failed during {operation}")]
    Io { operation: &'static str },
    #[error("KIS read coordination blocking task failed")]
    BlockingTaskFailed,
}

#[derive(Clone)]
pub struct ReadCoordinator {
    config: ReadCoordinationConfig,
    credentials: Arc<ReadCredentials>,
    clock: Arc<dyn Clock>,
}

impl ReadCoordinator {
    pub fn new(
        config: ReadCoordinationConfig,
        credentials: ReadCredentials,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            config,
            credentials: Arc::new(credentials),
            clock,
        }
    }

    /// Acquire or reuse the shared token without reserving or consuming a GET.
    pub async fn token(
        &self,
        issuer: &dyn TokenIssuer,
        acquisition: LockAcquisition,
    ) -> Result<AccessToken, ReadCoordinationError> {
        let mut gate = self.acquire(acquisition).await?;
        let observed_at = self.clock.now_ms();
        let mut state = gate.load_and_validate(self.credentials.as_ref(), observed_at)?;
        gate.write_state(&state)?;
        let token = self.ensure_token(&mut gate, &mut state, issuer).await?;
        let now = self.clock.now_ms();
        let advanced = now > state.wall_time_high_water_ms;
        advance_high_water(&mut state, now)?;
        if advanced {
            gate.write_state(&state)?;
        }
        Ok(token)
    }

    /// Execute one allowlisted read while holding the global process-shared gate.
    /// Invalid pairs are rejected before opening state or invoking either callback.
    pub async fn execute<T, F, Fut>(
        &self,
        path: &str,
        tr_id: &str,
        issuer: &dyn TokenIssuer,
        acquisition: LockAcquisition,
        request_timeout: Duration,
        read: F,
    ) -> Result<T, ReadCoordinationError>
    where
        F: FnOnce(AccessToken) -> Fut,
        Fut: Future<Output = ReadCallbackResult<T>>,
    {
        let channel = ReadChannel::from_pair(path, tr_id)?;
        let timeout_ms = duration_ms(request_timeout)
            .filter(|value| *value > 0 && *value <= MAX_REQUEST_TIMEOUT_MS)
            .ok_or(ReadCoordinationError::InvalidRequestTimeout)?;

        let mut gate = self.acquire(acquisition).await?;
        let observed_at = self.clock.now_ms();
        let mut state = gate.load_and_validate(self.credentials.as_ref(), observed_at)?;
        // Persist a credential-generation rotation and the wall-time high-water
        // even when a cooldown or older reservation denies this read.
        gate.write_state(&state)?;
        check_read_eligibility(&mut state, channel, observed_at)?;

        let token = self.ensure_token(&mut gate, &mut state, issuer).await?;

        // Token issuance can be slow. Reserve against the actual final GET
        // start time, never the time at which token work began.
        let start_ms = self.clock.now_ms();
        let advanced = start_ms > state.wall_time_high_water_ms;
        advance_high_water(&mut state, start_ms)?;
        if advanced {
            gate.write_state(&state)?;
        }
        check_read_eligibility(&mut state, channel, start_ms)?;
        let fence = reserve_read(&mut state, channel, start_ms, timeout_ms)?;
        gate.write_state(&state)?;

        let outcome = read(token).await;
        let finished_ms = self.clock.now_ms();
        advance_high_water(&mut state, finished_ms)?;

        match outcome {
            ReadCallbackResult::Success(value) => {
                clear_matching_reservation(&mut state, fence);
                gate.write_state(&state)?;
                Ok(value)
            }
            ReadCallbackResult::Unauthorized => {
                state.token = None;
                clear_matching_reservation(&mut state, fence);
                gate.write_state(&state)?;
                Err(ReadCoordinationError::Unauthorized)
            }
            ReadCallbackResult::RateLimited { retry_after } => {
                let Some(retry_after_ms) = duration_ms(retry_after) else {
                    gate.write_state(&state)?;
                    return Err(ReadCoordinationError::InvalidStateTime);
                };
                let Some(cooldown_until) = finished_ms.checked_add(retry_after_ms) else {
                    gate.write_state(&state)?;
                    return Err(ReadCoordinationError::InvalidStateTime);
                };
                state.broker_cooldown_until_ms = Some(
                    state
                        .broker_cooldown_until_ms
                        .unwrap_or(0)
                        .max(cooldown_until),
                );
                clear_matching_reservation(&mut state, fence);
                gate.write_state(&state)?;
                Err(ReadCoordinationError::CallbackRateLimited {
                    retry_after_ms: retry_after_ms as u64,
                })
            }
            ReadCallbackResult::CompletedFailure(kind) => {
                clear_matching_reservation(&mut state, fence);
                gate.write_state(&state)?;
                Err(ReadCoordinationError::CallbackFailed { kind })
            }
            ReadCallbackResult::Ambiguous => {
                // Preserve the fence until its durable deadline. We know only
                // that local completion is ambiguous, not what the broker did.
                gate.write_state(&state)?;
                Err(ReadCoordinationError::CallbackAmbiguous)
            }
        }
    }

    async fn ensure_token(
        &self,
        gate: &mut LockedRoot,
        state: &mut PersistedState,
        issuer: &dyn TokenIssuer,
    ) -> Result<AccessToken, ReadCoordinationError> {
        let now = self.clock.now_ms();
        let advanced = now > state.wall_time_high_water_ms;
        advance_high_water(state, now)?;
        if advanced {
            gate.write_state(state)?;
        }
        if let Some(token) = state.token.as_ref()
            && now.saturating_add(DEFAULT_REFRESH_MARGIN_MS) < token.expires_at_ms
        {
            return Ok(token.to_access_token());
        }

        if let Some(last) = state.last_issue_attempt_ms {
            let elapsed = now.saturating_sub(last);
            if elapsed < MIN_ISSUE_INTERVAL_MS {
                return Err(ReadCoordinationError::TokenIssueCooldown {
                    retry_after_ms: (MIN_ISSUE_INTERVAL_MS - elapsed) as u64,
                });
            }
        }

        // This write precedes the await. Cancellation, panic, or process death
        // in the issuer therefore keeps the one-minute debt.
        state.last_issue_attempt_ms = Some(now);
        gate.write_state(state)?;
        let issued = match issuer.issue().await {
            Ok(issued) => issued,
            Err(_) => {
                let failed_at = self.clock.now_ms();
                advance_high_water(state, failed_at)?;
                gate.write_state(state)?;
                return Err(ReadCoordinationError::TokenIssueFailed);
            }
        };
        let after = self.clock.now_ms();
        advance_high_water(state, after)?;
        if issued.value.expose().trim().is_empty()
            || issued.expires_at_ms <= after.saturating_add(DEFAULT_REFRESH_MARGIN_MS)
            || issued.expires_at_ms > after.saturating_add(MAX_TOKEN_FUTURE_MS)
        {
            gate.write_state(state)?;
            return Err(ReadCoordinationError::UnusableToken);
        }
        state.token = Some(PersistedToken::from_access_token(&issued));
        gate.write_state(state)?;
        Ok(issued)
    }

    async fn acquire(
        &self,
        acquisition: LockAcquisition,
    ) -> Result<LockedRoot, ReadCoordinationError> {
        if let LockAcquisition::Bounded(wait) = acquisition
            && (wait.is_zero() || wait > MAX_LOCK_WAIT)
        {
            return Err(ReadCoordinationError::InvalidLockWait);
        }
        let config = self.config.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel_on_drop = CancelOnDrop(Arc::clone(&cancelled));
        let result = tokio::task::spawn_blocking(move || {
            LockedRoot::acquire(&config, acquisition, &cancelled)
        })
        .await
        .map_err(|_| ReadCoordinationError::BlockingTaskFailed)?;
        drop(cancel_on_drop);
        result
    }
}

impl std::fmt::Debug for ReadCoordinator {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReadCoordinator")
            .field("config", &self.config)
            .field("credentials", &self.credentials)
            .finish_non_exhaustive()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedToken {
    value: String,
    expires_at_ms: i64,
}

impl PersistedToken {
    fn from_access_token(token: &AccessToken) -> Self {
        Self {
            value: token.value.expose().clone(),
            expires_at_ms: token.expires_at_ms,
        }
    }

    fn to_access_token(&self) -> AccessToken {
        AccessToken {
            value: Secret::new(self.value.clone()),
            expires_at_ms: self.expires_at_ms,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InFlight {
    fence: u64,
    channel: String,
    reserved_at_ms: i64,
    expires_at_ms: i64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedState {
    schema_version: u32,
    credential_generation: u64,
    credential_verifier: String,
    token: Option<PersistedToken>,
    last_issue_attempt_ms: Option<i64>,
    wall_time_high_water_ms: i64,
    last_global_attempt_ms: Option<i64>,
    channel_attempts_ms: BTreeMap<String, i64>,
    last_intraday_attempt_ms: Option<i64>,
    broker_cooldown_until_ms: Option<i64>,
    intraday_kst_date: Option<String>,
    intraday_attempts: u32,
    next_fence: u64,
    in_flight: Option<InFlight>,
}

impl PersistedState {
    fn initial(credentials: &ReadCredentials, now: i64) -> Result<Self, ReadCoordinationError> {
        if now < 0 {
            return Err(ReadCoordinationError::InvalidStateTime);
        }
        Ok(Self {
            schema_version: STATE_SCHEMA_VERSION,
            credential_generation: credentials.generation,
            credential_verifier: encode_hex(&credentials.verifier()),
            token: None,
            last_issue_attempt_ms: None,
            wall_time_high_water_ms: now,
            last_global_attempt_ms: None,
            channel_attempts_ms: BTreeMap::new(),
            last_intraday_attempt_ms: None,
            broker_cooldown_until_ms: None,
            intraday_kst_date: None,
            intraday_attempts: 0,
            next_fence: 1,
            in_flight: None,
        })
    }
}

struct LockedRoot {
    directory: File,
    _lock: File,
    expected_uid: u32,
}

struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

impl LockedRoot {
    fn acquire(
        config: &ReadCoordinationConfig,
        acquisition: LockAcquisition,
        cancelled: &AtomicBool,
    ) -> Result<Self, ReadCoordinationError> {
        if cancelled.load(Ordering::Acquire) {
            return Err(ReadCoordinationError::AcquisitionCancelled);
        }
        let directory = open_root(&config.state_root, config.expected_uid)?;
        let lock = match openat_file(
            directory.as_raw_fd(),
            LOCK_FILE_NAME,
            OpenKind::LockExisting,
        ) {
            Ok(lock) => lock,
            Err(ReadCoordinationError::LockMissing) => {
                // Once state exists, losing the stable lock inode is fatal: a
                // replacement could split active processes across two flocks.
                match openat_file(directory.as_raw_fd(), STATE_FILE_NAME, OpenKind::ReadOnly) {
                    Ok(_) => return Err(ReadCoordinationError::UnsafeFile),
                    Err(ReadCoordinationError::StateMissing) => {}
                    Err(error) => return Err(error),
                }
                match openat_file(directory.as_raw_fd(), LOCK_FILE_NAME, OpenKind::LockNew) {
                    Ok(lock) => lock,
                    Err(ReadCoordinationError::LockAlreadyExists) => openat_file(
                        directory.as_raw_fd(),
                        LOCK_FILE_NAME,
                        OpenKind::LockExisting,
                    )?,
                    Err(error) => return Err(error),
                }
            }
            Err(error) => return Err(error),
        };
        validate_regular_file(&lock, config.expected_uid)?;

        match acquisition {
            LockAcquisition::NonBlocking => {
                FileExt::try_lock_exclusive(&lock).map_err(|_| ReadCoordinationError::LockBusy)?
            }
            LockAcquisition::Bounded(wait) => {
                let deadline = Instant::now() + wait;
                loop {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(ReadCoordinationError::AcquisitionCancelled);
                    }
                    match FileExt::try_lock_exclusive(&lock) {
                        Ok(()) => break,
                        Err(_) if Instant::now() >= deadline => {
                            return Err(ReadCoordinationError::LockTimeout);
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(5)),
                    }
                }
            }
        }

        if cancelled.load(Ordering::Acquire) {
            return Err(ReadCoordinationError::AcquisitionCancelled);
        }

        // Revalidate the locked descriptor; the stable inode is never renamed.
        validate_regular_file(&lock, config.expected_uid)?;
        let lock_entry = openat_file(
            directory.as_raw_fd(),
            LOCK_FILE_NAME,
            OpenKind::LockExisting,
        )?;
        validate_regular_file(&lock_entry, config.expected_uid)?;
        let locked_metadata = lock
            .metadata()
            .map_err(|_| ReadCoordinationError::UnsafeFile)?;
        let entry_metadata = lock_entry
            .metadata()
            .map_err(|_| ReadCoordinationError::UnsafeFile)?;
        if locked_metadata.dev() != entry_metadata.dev()
            || locked_metadata.ino() != entry_metadata.ino()
        {
            return Err(ReadCoordinationError::UnsafeFile);
        }
        Ok(Self {
            directory,
            _lock: lock,
            expected_uid: config.expected_uid,
        })
    }

    fn load_and_validate(
        &mut self,
        credentials: &ReadCredentials,
        now: i64,
    ) -> Result<PersistedState, ReadCoordinationError> {
        let mut state = match openat_file(
            self.directory.as_raw_fd(),
            STATE_FILE_NAME,
            OpenKind::ReadOnly,
        ) {
            Ok(file) => {
                validate_regular_file(&file, self.expected_uid)?;
                let size = file
                    .metadata()
                    .map_err(|_| io_error("inspect state"))?
                    .len();
                if size > MAX_STATE_BYTES {
                    return Err(ReadCoordinationError::StateTooLarge);
                }
                let mut bytes = Vec::with_capacity(size as usize);
                file.take(MAX_STATE_BYTES + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| io_error("read state"))?;
                if bytes.len() as u64 > MAX_STATE_BYTES {
                    return Err(ReadCoordinationError::StateTooLarge);
                }
                let parsed: PersistedState = serde_json::from_slice(&bytes)
                    .map_err(|_| ReadCoordinationError::CorruptState)?;
                let canonical = canonical_state_bytes(&parsed)?;
                if !constant_time_eq(&bytes, &canonical) {
                    return Err(ReadCoordinationError::NonCanonicalState);
                }
                parsed
            }
            Err(ReadCoordinationError::StateMissing) => PersistedState::initial(credentials, now)?,
            Err(error) => return Err(error),
        };

        validate_state(&state, now)?;
        apply_credentials(&mut state, credentials)?;
        advance_high_water(&mut state, now)?;
        Ok(state)
    }

    fn write_state(&mut self, state: &PersistedState) -> Result<(), ReadCoordinationError> {
        validate_state(state, state.wall_time_high_water_ms)?;
        let bytes = canonical_state_bytes(state)?;
        if bytes.len() as u64 > MAX_STATE_BYTES {
            return Err(ReadCoordinationError::StateTooLarge);
        }

        static NONCE: AtomicU64 = AtomicU64::new(1);
        let temporary_name = format!(
            ".state-v1.tmp.{}.{}",
            std::process::id(),
            NONCE.fetch_add(1, Ordering::Relaxed)
        );
        let mut temporary = openat_file(
            self.directory.as_raw_fd(),
            &temporary_name,
            OpenKind::WriteNew,
        )?;
        let temporary_metadata = temporary
            .metadata()
            .map_err(|_| io_error("inspect temporary state"))?;
        validate_regular_file(&temporary, self.expected_uid)?;

        let result = (|| {
            temporary
                .write_all(&bytes)
                .map_err(|_| io_error("write temporary state"))?;
            temporary
                .sync_all()
                .map_err(|_| io_error("sync temporary state"))?;
            renameat(self.directory.as_raw_fd(), &temporary_name, STATE_FILE_NAME)?;
            self.directory
                .sync_all()
                .map_err(|_| io_error("sync state directory"))?;

            let canonical = openat_file(
                self.directory.as_raw_fd(),
                STATE_FILE_NAME,
                OpenKind::ReadOnly,
            )?;
            validate_regular_file(&canonical, self.expected_uid)?;
            let canonical_metadata = canonical
                .metadata()
                .map_err(|_| io_error("inspect committed state"))?;
            if canonical_metadata.dev() != temporary_metadata.dev()
                || canonical_metadata.ino() != temporary_metadata.ino()
            {
                return Err(ReadCoordinationError::UnsafeFile);
            }
            Ok(())
        })();

        if result.is_err() {
            let _ = unlinkat(self.directory.as_raw_fd(), &temporary_name);
        }
        result
    }
}

fn apply_credentials(
    state: &mut PersistedState,
    credentials: &ReadCredentials,
) -> Result<(), ReadCoordinationError> {
    let supplied = credentials.verifier();
    let stored = decode_verifier(&state.credential_verifier)?;
    if credentials.generation < state.credential_generation {
        return Err(ReadCoordinationError::StaleCredentialGeneration);
    }
    if credentials.generation == state.credential_generation {
        if !constant_time_eq(&stored, &supplied) {
            return Err(ReadCoordinationError::CredentialScopeMismatch);
        }
        return Ok(());
    }

    // A higher generation rotates only token identity. Conservative attempt,
    // cooldown, daily-ledger, and in-flight debt all survive the rotation.
    state.credential_generation = credentials.generation;
    state.credential_verifier = encode_hex(&supplied);
    state.token = None;
    Ok(())
}

fn validate_state(state: &PersistedState, now: i64) -> Result<(), ReadCoordinationError> {
    if state.schema_version != STATE_SCHEMA_VERSION
        || state.credential_generation == 0
        || state.next_fence == 0
        || state.intraday_attempts > INTRADAY_DAILY_LIMIT
        || decode_verifier(&state.credential_verifier).is_err()
        || !(0..=MAX_SUPPORTED_WALL_MS).contains(&now)
        || !(0..=MAX_SUPPORTED_WALL_MS).contains(&state.wall_time_high_water_ms)
    {
        return Err(ReadCoordinationError::CorruptState);
    }
    if now < state.wall_time_high_water_ms {
        return Err(ReadCoordinationError::ClockRollback);
    }

    let historical = [
        state.last_issue_attempt_ms,
        state.last_global_attempt_ms,
        state.last_intraday_attempt_ms,
    ];
    if historical
        .into_iter()
        .flatten()
        .any(|value| value < 0 || value > state.wall_time_high_water_ms)
        || state
            .channel_attempts_ms
            .values()
            .any(|value| *value < 0 || *value > state.wall_time_high_water_ms)
    {
        return Err(ReadCoordinationError::InvalidStateTime);
    }
    if state.channel_attempts_ms.keys().any(|key| {
        !ReadChannel::ALL
            .iter()
            .any(|channel| channel.state_key() == key)
    }) {
        return Err(ReadCoordinationError::CorruptState);
    }
    if let Some(until) = state.broker_cooldown_until_ms
        && until < 0
    {
        return Err(ReadCoordinationError::InvalidStateTime);
    }
    if let Some(token) = state.token.as_ref()
        && (token.value.trim().is_empty()
            || token.expires_at_ms < 0
            || token.expires_at_ms
                > state
                    .wall_time_high_water_ms
                    .saturating_add(MAX_TOKEN_FUTURE_MS))
    {
        return Err(ReadCoordinationError::InvalidStateTime);
    }
    if let Some(in_flight) = state.in_flight.as_ref()
        && (in_flight.fence == 0
            || in_flight.fence >= state.next_fence
            || in_flight.reserved_at_ms < 0
            || in_flight.reserved_at_ms > state.wall_time_high_water_ms
            || in_flight.expires_at_ms <= in_flight.reserved_at_ms
            || in_flight.expires_at_ms
                > in_flight
                    .reserved_at_ms
                    .saturating_add(MAX_REQUEST_TIMEOUT_MS)
            || !ReadChannel::ALL
                .iter()
                .any(|channel| channel.state_key() == in_flight.channel))
    {
        return Err(ReadCoordinationError::InvalidStateTime);
    }
    if state.intraday_kst_date.is_none() != (state.intraday_attempts == 0) {
        return Err(ReadCoordinationError::CorruptState);
    }
    if let Some(date) = state.intraday_kst_date.as_deref()
        && (!valid_civil_date(date) || date > kst_date(now).as_str())
    {
        return Err(ReadCoordinationError::InvalidStateTime);
    }
    Ok(())
}

fn advance_high_water(state: &mut PersistedState, now: i64) -> Result<(), ReadCoordinationError> {
    if now < state.wall_time_high_water_ms {
        return Err(ReadCoordinationError::ClockRollback);
    }
    state.wall_time_high_water_ms = now;
    Ok(())
}

fn check_read_eligibility(
    state: &mut PersistedState,
    channel: ReadChannel,
    now: i64,
) -> Result<(), ReadCoordinationError> {
    if let Some(in_flight) = state.in_flight.as_ref()
        && now < in_flight.expires_at_ms
    {
        return Err(ReadCoordinationError::ReservationActive {
            retry_after_ms: (in_flight.expires_at_ms - now) as u64,
        });
    }
    if let Some(until) = state.broker_cooldown_until_ms
        && now < until
    {
        return Err(ReadCoordinationError::BrokerCooldown {
            retry_after_ms: (until - now) as u64,
        });
    }
    enforce_spacing(
        state.last_global_attempt_ms,
        now,
        GLOBAL_READ_INTERVAL_MS,
        |retry_after_ms| ReadCoordinationError::GlobalSpacing { retry_after_ms },
    )?;
    enforce_spacing(
        state.channel_attempts_ms.get(channel.state_key()).copied(),
        now,
        CHANNEL_READ_INTERVAL_MS,
        |retry_after_ms| ReadCoordinationError::ChannelSpacing { retry_after_ms },
    )?;
    if channel.is_intraday() {
        enforce_spacing(
            state.last_intraday_attempt_ms,
            now,
            INTRADAY_READ_INTERVAL_MS,
            |retry_after_ms| ReadCoordinationError::IntradaySpacing { retry_after_ms },
        )?;
        let today = kst_date(now);
        let attempts_today = match state.intraday_kst_date.as_deref() {
            None => 0,
            Some(saved) if saved < today.as_str() => 0,
            Some(saved) if saved > today.as_str() => {
                return Err(ReadCoordinationError::ClockRollback);
            }
            Some(_) => state.intraday_attempts,
        };
        if attempts_today >= INTRADAY_DAILY_LIMIT {
            return Err(ReadCoordinationError::IntradayBudgetExhausted);
        }
    }
    Ok(())
}

fn enforce_spacing<F>(
    last: Option<i64>,
    now: i64,
    interval: i64,
    error: F,
) -> Result<(), ReadCoordinationError>
where
    F: FnOnce(u64) -> ReadCoordinationError,
{
    if let Some(last) = last {
        let elapsed = now.saturating_sub(last);
        if elapsed < interval {
            return Err(error((interval - elapsed) as u64));
        }
    }
    Ok(())
}

fn reserve_read(
    state: &mut PersistedState,
    channel: ReadChannel,
    now: i64,
    timeout_ms: i64,
) -> Result<u64, ReadCoordinationError> {
    let fence = state.next_fence;
    state.next_fence = state
        .next_fence
        .checked_add(1)
        .ok_or(ReadCoordinationError::CorruptState)?;
    state.last_global_attempt_ms = Some(now);
    state
        .channel_attempts_ms
        .insert(channel.state_key().to_owned(), now);
    if channel.is_intraday() {
        let today = kst_date(now);
        if state.intraday_kst_date.as_deref() != Some(today.as_str()) {
            state.intraday_kst_date = Some(today);
            state.intraday_attempts = 0;
        }
        state.last_intraday_attempt_ms = Some(now);
        state.intraday_attempts = state
            .intraday_attempts
            .checked_add(1)
            .ok_or(ReadCoordinationError::IntradayBudgetExhausted)?;
    }
    state.in_flight = Some(InFlight {
        fence,
        channel: channel.state_key().to_owned(),
        reserved_at_ms: now,
        expires_at_ms: now
            .checked_add(timeout_ms)
            .filter(|expires| *expires <= MAX_SUPPORTED_WALL_MS)
            .ok_or(ReadCoordinationError::InvalidRequestTimeout)?,
    });
    Ok(fence)
}

fn clear_matching_reservation(state: &mut PersistedState, fence: u64) {
    if state.in_flight.as_ref().map(|entry| entry.fence) == Some(fence) {
        state.in_flight = None;
    }
}

fn canonical_state_bytes(state: &PersistedState) -> Result<Vec<u8>, ReadCoordinationError> {
    let mut bytes = serde_json::to_vec(state).map_err(|_| ReadCoordinationError::CorruptState)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn duration_ms(duration: Duration) -> Option<i64> {
    i64::try_from(duration.as_millis()).ok()
}

fn io_error(operation: &'static str) -> ReadCoordinationError {
    ReadCoordinationError::Io { operation }
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn decode_verifier(value: &str) -> Result<[u8; 32], ReadCoordinationError> {
    if value.len() != 64 {
        return Err(ReadCoordinationError::CorruptState);
    }
    let mut decoded = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        decoded[index] = (decode_nibble(pair[0])? << 4) | decode_nibble(pair[1])?;
    }
    Ok(decoded)
}

fn decode_nibble(value: u8) -> Result<u8, ReadCoordinationError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err(ReadCoordinationError::CorruptState),
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn kst_date(unix_ms: i64) -> String {
    let local_ms = unix_ms.saturating_add(9 * 60 * 60 * 1_000);
    let days = local_ms.div_euclid(86_400_000);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

fn valid_civil_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(index, byte)| index != 4 && index != 7 && !byte.is_ascii_digit())
    {
        return false;
    }
    let Ok(year) = value[0..4].parse::<u32>() else {
        return false;
    };
    let Ok(month) = value[5..7].parse::<u32>() else {
        return false;
    };
    let Ok(day) = value[8..10].parse::<u32>() else {
        return false;
    };
    if year == 0 || !(1..=12).contains(&month) {
        return false;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    (1..=days).contains(&day)
}

// Howard Hinnant's civil calendar conversion, with Unix day zero offset.
fn civil_from_days(unix_days: i64) -> (i64, i64, i64) {
    let z = unix_days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

fn open_root(path: &Path, expected_uid: u32) -> Result<File, ReadCoordinationError> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC);
    let directory = options
        .open(path)
        .map_err(|_| ReadCoordinationError::RootUnavailable)?;
    let metadata = directory
        .metadata()
        .map_err(|_| ReadCoordinationError::RootUnavailable)?;
    if !metadata.file_type().is_dir()
        || metadata.uid() != expected_uid
        || metadata.permissions().mode() & 0o7777 != ROOT_MODE
    {
        return Err(ReadCoordinationError::UnsafeRoot);
    }
    Ok(directory)
}

fn validate_regular_file(file: &File, expected_uid: u32) -> Result<(), ReadCoordinationError> {
    let metadata = file
        .metadata()
        .map_err(|_| ReadCoordinationError::UnsafeFile)?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.uid() != expected_uid
        || metadata.nlink() != 1
        || metadata.permissions().mode() & 0o7777 != FILE_MODE
    {
        return Err(ReadCoordinationError::UnsafeFile);
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum OpenKind {
    ReadOnly,
    LockExisting,
    LockNew,
    WriteNew,
}

fn openat_file(
    directory_fd: RawFd,
    name: &str,
    kind: OpenKind,
) -> Result<File, ReadCoordinationError> {
    let name = CString::new(name).map_err(|_| ReadCoordinationError::UnsafeFile)?;
    let (flags, operation) = match kind {
        OpenKind::ReadOnly => (O_RDONLY | O_NONBLOCK | O_NOFOLLOW | O_CLOEXEC, "open state"),
        OpenKind::LockExisting => (O_RDWR | O_NOFOLLOW | O_CLOEXEC, "open lock file"),
        OpenKind::LockNew => (
            O_RDWR | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC,
            "create lock file",
        ),
        OpenKind::WriteNew => (
            O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC,
            "create temporary state",
        ),
    };
    // SAFETY: `directory_fd` is a live directory descriptor; `name` is a
    // NUL-terminated single component; flags and mode are Linux constants.
    let fd = unsafe { c_openat(directory_fd, name.as_ptr(), flags, FILE_MODE) };
    if fd < 0 {
        let error = std::io::Error::last_os_error();
        if matches!(kind, OpenKind::ReadOnly) && error.kind() == std::io::ErrorKind::NotFound {
            return Err(ReadCoordinationError::StateMissing);
        }
        if matches!(kind, OpenKind::LockExisting) && error.kind() == std::io::ErrorKind::NotFound {
            return Err(ReadCoordinationError::LockMissing);
        }
        if matches!(kind, OpenKind::LockNew) && error.kind() == std::io::ErrorKind::AlreadyExists {
            return Err(ReadCoordinationError::LockAlreadyExists);
        }
        if matches!(kind, OpenKind::ReadOnly) {
            return Err(ReadCoordinationError::UnsafeFile);
        }
        if matches!(kind, OpenKind::LockExisting | OpenKind::LockNew) {
            return Err(ReadCoordinationError::UnsafeFile);
        }
        return Err(io_error(operation));
    }
    // SAFETY: `openat` returned a newly owned descriptor.
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn renameat(directory_fd: RawFd, from: &str, to: &str) -> Result<(), ReadCoordinationError> {
    let from = CString::new(from).map_err(|_| ReadCoordinationError::UnsafeFile)?;
    let to = CString::new(to).map_err(|_| ReadCoordinationError::UnsafeFile)?;
    // SAFETY: both names are NUL-terminated single components relative to the
    // same live directory descriptor.
    if unsafe { c_renameat(directory_fd, from.as_ptr(), directory_fd, to.as_ptr()) } != 0 {
        return Err(io_error("rename state"));
    }
    Ok(())
}

fn unlinkat(directory_fd: RawFd, name: &str) -> Result<(), ReadCoordinationError> {
    let name = CString::new(name).map_err(|_| ReadCoordinationError::UnsafeFile)?;
    // SAFETY: `name` is a NUL-terminated single component and flags=0 removes
    // only a non-directory entry relative to the verified descriptor.
    if unsafe { c_unlinkat(directory_fd, name.as_ptr(), 0) } != 0 {
        return Err(io_error("remove temporary state"));
    }
    Ok(())
}

// Linux open(2) values. The production runtime and supported test target are
// Linux; declaring this narrow ABI avoids adding an extra direct dependency.
const O_RDONLY: i32 = 0;
const O_WRONLY: i32 = 1;
const O_RDWR: i32 = 2;
const O_CREAT: i32 = 0o100;
const O_EXCL: i32 = 0o200;
const O_NONBLOCK: i32 = 0o4000;
const O_DIRECTORY: i32 = 0o200000;
const O_NOFOLLOW: i32 = 0o400000;
const O_CLOEXEC: i32 = 0o2000000;

unsafe extern "C" {
    #[link_name = "openat"]
    fn c_openat(directory_fd: i32, path: *const std::ffi::c_char, flags: i32, mode: u32) -> i32;
    #[link_name = "renameat"]
    fn c_renameat(
        old_directory_fd: i32,
        old_path: *const std::ffi::c_char,
        new_directory_fd: i32,
        new_path: *const std::ffi::c_char,
    ) -> i32;
    #[link_name = "unlinkat"]
    fn c_unlinkat(directory_fd: i32, path: *const std::ffi::c_char, flags: i32) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kst_date_rolls_at_korean_midnight() {
        assert_eq!(kst_date(0), "1970-01-01");
        assert_eq!(kst_date(15 * 60 * 60 * 1_000 - 1), "1970-01-01");
        assert_eq!(kst_date(15 * 60 * 60 * 1_000), "1970-01-02");
    }

    #[test]
    fn five_thousand_intraday_reservations_exhaust_one_kst_day_and_next_day_resets() {
        let credentials = ReadCredentials::new(
            Secret::new("fake-key".to_owned()),
            Secret::new("fake-secret".to_owned()),
            1,
        )
        .expect("credentials");
        let start = 0_i64;
        let mut state = PersistedState::initial(&credentials, start).expect("state");

        for attempt in 0..INTRADAY_DAILY_LIMIT {
            let now = start + i64::from(attempt) * INTRADAY_READ_INTERVAL_MS;
            advance_high_water(&mut state, now).expect("nonregressing time");
            check_read_eligibility(&mut state, ReadChannel::InquirePrice, now)
                .expect("budget remains");
            let fence = reserve_read(&mut state, ReadChannel::InquirePrice, now, 3_000)
                .expect("reserve each attempt");
            clear_matching_reservation(&mut state, fence);
        }
        let exhausted_at = start + i64::from(INTRADAY_DAILY_LIMIT) * INTRADAY_READ_INTERVAL_MS;
        advance_high_water(&mut state, exhausted_at).expect("advance");
        assert!(matches!(
            check_read_eligibility(&mut state, ReadChannel::InquirePrice, exhausted_at),
            Err(ReadCoordinationError::IntradayBudgetExhausted)
        ));

        // Unix 15:00 UTC is midnight KST. The reset is allowed only because
        // both wall time and the KST civil date moved forward.
        let next_kst_day = 15 * 60 * 60 * 1_000;
        advance_high_water(&mut state, next_kst_day).expect("next day advances");
        check_read_eligibility(&mut state, ReadChannel::InquirePrice, next_kst_day)
            .expect("next day budget resets");
        reserve_read(&mut state, ReadChannel::InquirePrice, next_kst_day, 3_000)
            .expect("first next-day attempt");
        assert_eq!(state.intraday_attempts, 1);
        assert_eq!(state.intraday_kst_date.as_deref(), Some("1970-01-02"));
    }

    #[test]
    fn every_reserved_retry_or_crash_is_counted_before_completion() {
        let credentials = ReadCredentials::new(
            Secret::new("fake-key".to_owned()),
            Secret::new("fake-secret".to_owned()),
            1,
        )
        .expect("credentials");
        let mut state = PersistedState::initial(&credentials, 0).expect("state");
        let fence =
            reserve_read(&mut state, ReadChannel::InquirePrice, 0, 3_000).expect("initial attempt");
        assert_eq!(state.intraday_attempts, 1);
        assert_eq!(
            state.in_flight.as_ref().map(|entry| entry.fence),
            Some(fence)
        );

        // Model an abandoned request after its deadline. Replacing its fence
        // reserves a distinct retry and never refunds the first attempt.
        advance_high_water(&mut state, 5_000).expect("advance");
        check_read_eligibility(&mut state, ReadChannel::InquirePrice, 5_000)
            .expect("expired fence permits bounded retry");
        reserve_read(&mut state, ReadChannel::InquirePrice, 5_000, 3_000)
            .expect("retry reservation");
        assert_eq!(state.intraday_attempts, 2);
    }
}
