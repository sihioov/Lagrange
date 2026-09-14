//! Provider-free session-window and intraday quote capture seams.
//!
//! The runner supplies the already-guarded KIS one-shot result.  This module
//! owns only the fixed, hash-pinned session-window contract, the exact quote
//! request shape, and the hand-off to the accepted seven-field parser.  It has
//! no job-queue dependency and never writes Raw or provider response bodies.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveTime, TimeZone, Utc};
use kis_client::{
    IntradayAttemptError, IntradayAttemptMetadata, IntradayAttemptOutcome,
    IntradayAttemptReservationMetadata,
};
use market_data::intraday_quotes::{IntradayQuote, IntradayQuoteError, parse_intraday_quote};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const INTRADAY_QUOTE_PATH: &str = "/uapi/domestic-stock/v1/quotations/inquire-price";
pub const INTRADAY_QUOTE_TR_ID: &str = "FHKST01010100";
pub const INTRADAY_SESSION_WINDOWS_PATH: &str =
    "/opt/lagrange/configs/market-hours/krx-intraday-session-windows-v1.json";
pub const INTRADAY_SESSION_WINDOWS_SHA256_ENV: &str = "OWNER_INTRADAY_SESSION_WINDOWS_SHA256";
pub const INTRADAY_SESSION_WINDOWS_SOURCE_ENV: &str = "OWNER_INTRADAY_SESSION_WINDOWS_SOURCE";
pub const INTRADAY_OPERATIONAL_SESSION_WINDOWS_PATH: &str =
    "/run/lagrange/intraday-session-windows";
pub const INTRADAY_OPERATIONAL_SESSION_WINDOWS_ACTIVATION: &str = "activation.json";
pub const INTRADAY_SESSION_SCHEMA_VERSION: u32 = 1;
pub const INTRADAY_SESSION_EXCHANGE: &str = "KRX";
pub const INTRADAY_SESSION_TIMEZONE: &str = "Asia/Seoul";

const MAX_WINDOW_BYTES: usize = 1_048_576;
const MAX_OPERATIONAL_ACTIVATION_BYTES: usize = 4096;
const KST_SECONDS_EAST: i32 = 9 * 60 * 60;

pub const INTRADAY_OPERATIONAL_DIRECTORY_UID: u32 = 0;
pub const INTRADAY_OPERATIONAL_DIRECTORY_GID: u32 = 10001;
pub const INTRADAY_OPERATIONAL_DIRECTORY_MODE: u32 = 0o750;
pub const INTRADAY_OPERATIONAL_FILE_UID: u32 = 0;
pub const INTRADAY_OPERATIONAL_FILE_GID: u32 = 10001;
pub const INTRADAY_OPERATIONAL_FILE_MODE: u32 = 0o640;

pub type IntradayUtcBounds = (DateTime<Utc>, DateTime<Utc>);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum IntradaySessionWindowError {
    #[error("INTRADAY_SESSION_WINDOW_MISSING")]
    Missing,
    #[error("INTRADAY_SESSION_WINDOW_INVALID")]
    Invalid,
    #[error("INTRADAY_SESSION_WINDOW_HASH_MISMATCH")]
    HashMismatch,
    #[error("INTRADAY_SESSION_WINDOW_UNSUPPORTED")]
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum IntradaySessionWindowSourceError {
    #[error("INTRADAY_SESSION_WINDOW_SOURCE_INVALID")]
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntradaySessionWindowSource {
    ReleaseV1,
    OperationalV1,
}

impl IntradaySessionWindowSource {
    pub fn from_optional_str(
        value: Option<&str>,
    ) -> Result<Self, IntradaySessionWindowSourceError> {
        match value {
            None | Some("release_v1") => Ok(Self::ReleaseV1),
            Some("operational_v1") => Ok(Self::OperationalV1),
            Some(_) => Err(IntradaySessionWindowSourceError::Invalid),
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReleaseV1 => "release_v1",
            Self::OperationalV1 => "operational_v1",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationalSessionWindowActivation {
    schema_version: u32,
    window_sha256: String,
}

impl OperationalSessionWindowActivation {
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub fn window_sha256(&self) -> &str {
        &self.window_sha256
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationalSessionWindowMetadata {
    pub is_directory: bool,
    pub is_symlink: bool,
    pub is_regular: bool,
    pub uid: u32,
    pub gid: u32,
    pub mode: u32,
    pub link_count: u64,
}

pub fn validate_operational_ancestor_metadata(
    metadata: OperationalSessionWindowMetadata,
) -> Result<(), IntradaySessionWindowError> {
    if metadata.is_symlink
        || !metadata.is_directory
        || metadata.uid != 0
        || metadata.mode & 0o022 != 0
    {
        Err(IntradaySessionWindowError::Invalid)
    } else {
        Ok(())
    }
}

pub fn validate_operational_directory_metadata(
    metadata: OperationalSessionWindowMetadata,
) -> Result<(), IntradaySessionWindowError> {
    if metadata.is_symlink
        || !metadata.is_directory
        || metadata.uid != INTRADAY_OPERATIONAL_DIRECTORY_UID
        || metadata.gid != INTRADAY_OPERATIONAL_DIRECTORY_GID
        || metadata.mode != INTRADAY_OPERATIONAL_DIRECTORY_MODE
    {
        Err(IntradaySessionWindowError::Invalid)
    } else {
        Ok(())
    }
}

pub fn validate_operational_file_metadata(
    metadata: OperationalSessionWindowMetadata,
) -> Result<(), IntradaySessionWindowError> {
    if metadata.is_symlink
        || metadata.is_directory
        || !metadata.is_regular
        || metadata.uid != INTRADAY_OPERATIONAL_FILE_UID
        || metadata.gid != INTRADAY_OPERATIONAL_FILE_GID
        || metadata.mode != INTRADAY_OPERATIONAL_FILE_MODE
        || metadata.link_count != 1
    {
        Err(IntradaySessionWindowError::Invalid)
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntradaySessionDisposition {
    Regular,
    Special,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntradaySessionWindowEntry {
    pub date: NaiveDate,
    pub disposition: IntradaySessionDisposition,
    pub open_local: Option<NaiveTime>,
    pub close_local: Option<NaiveTime>,
    pub evidence_url: String,
    pub evidence_retrieved_at: DateTime<Utc>,
    pub evidence_sha256: String,
}

impl IntradaySessionWindowEntry {
    fn validate(&self) -> Result<(), IntradaySessionWindowError> {
        if self.evidence_url.is_empty()
            || self
                .evidence_url
                .bytes()
                .any(|byte| byte.is_ascii_control())
            || !self.evidence_url.starts_with("https://global.krx.co.kr/")
            || !is_prefixed_sha256(&self.evidence_sha256)
        {
            return Err(IntradaySessionWindowError::Invalid);
        }
        match self.disposition {
            IntradaySessionDisposition::Closed => {
                if self.open_local.is_some() || self.close_local.is_some() {
                    return Err(IntradaySessionWindowError::Invalid);
                }
            }
            IntradaySessionDisposition::Regular => {
                if self.open_local != NaiveTime::from_hms_opt(9, 0, 0)
                    || self.close_local != NaiveTime::from_hms_opt(15, 30, 0)
                {
                    return Err(IntradaySessionWindowError::Invalid);
                }
            }
            IntradaySessionDisposition::Special => {
                let (Some(open), Some(close)) = (self.open_local, self.close_local) else {
                    return Err(IntradaySessionWindowError::Invalid);
                };
                if open >= close {
                    return Err(IntradaySessionWindowError::Invalid);
                }
            }
        }
        Ok(())
    }

    pub fn is_closed(&self) -> bool {
        self.disposition == IntradaySessionDisposition::Closed
    }

    pub fn utc_bounds(&self) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
        let (Some(open), Some(close)) = (self.open_local, self.close_local) else {
            return None;
        };
        let offset = FixedOffset::east_opt(KST_SECONDS_EAST).expect("valid KST offset");
        let open = offset
            .from_local_datetime(&self.date.and_time(open))
            .single()?
            .with_timezone(&Utc);
        let close = offset
            .from_local_datetime(&self.date.and_time(close))
            .single()?
            .with_timezone(&Utc);
        Some((open, close))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntradaySessionWindowContract {
    window_contract_sha256: String,
    entries: BTreeMap<NaiveDate, IntradaySessionWindowEntry>,
}

impl IntradaySessionWindowContract {
    /// Validate fixture-provided bytes against the exact expected whole-file
    /// hash.  Production callers must obtain bytes only from the fixed path.
    pub fn from_bytes(
        bytes: &[u8],
        expected_hash: &str,
    ) -> Result<Self, IntradaySessionWindowError> {
        if bytes.is_empty() || bytes.len() > MAX_WINDOW_BYTES || !is_prefixed_sha256(expected_hash)
        {
            return Err(IntradaySessionWindowError::Invalid);
        }
        let actual = format!("sha256:{:x}", Sha256::digest(bytes));
        if actual != expected_hash {
            return Err(IntradaySessionWindowError::HashMismatch);
        }
        let wire: WireContract =
            serde_json::from_slice(bytes).map_err(|_| IntradaySessionWindowError::Invalid)?;
        if wire.schema_version != INTRADAY_SESSION_SCHEMA_VERSION
            || wire.exchange != INTRADAY_SESSION_EXCHANGE
            || wire.timezone != INTRADAY_SESSION_TIMEZONE
        {
            return Err(IntradaySessionWindowError::Invalid);
        }

        let mut entries = BTreeMap::new();
        let mut previous_date = None;
        for wire_entry in wire.entries {
            let entry = wire_entry.into_entry()?;
            if previous_date.is_some_and(|previous| entry.date <= previous) {
                return Err(IntradaySessionWindowError::Invalid);
            }
            previous_date = Some(entry.date);
            if entries.insert(entry.date, entry).is_some() {
                return Err(IntradaySessionWindowError::Invalid);
            }
        }
        if entries.values().any(|entry| entry.validate().is_err()) {
            return Err(IntradaySessionWindowError::Invalid);
        }
        Ok(Self {
            window_contract_sha256: expected_hash.to_owned(),
            entries,
        })
    }

    pub fn from_fixed_path() -> Result<Self, IntradaySessionWindowError> {
        let expected_hash = std::env::var(INTRADAY_SESSION_WINDOWS_SHA256_ENV)
            .map_err(|_| IntradaySessionWindowError::Missing)?;
        let bytes = std::fs::read(Path::new(INTRADAY_SESSION_WINDOWS_PATH))
            .map_err(|_| IntradaySessionWindowError::Missing)?;
        Self::from_bytes(&bytes, &expected_hash)
    }

    /// Load the selected source through the single session-window startup
    /// boundary. `ReleaseV1` retains the existing fixed path and environment
    /// hash contract; `OperationalV1` can only read the fixed operational
    /// directory and derives the immutable filename from `activation.json`.
    pub fn from_source(
        source: IntradaySessionWindowSource,
    ) -> Result<Self, IntradaySessionWindowError> {
        match source {
            IntradaySessionWindowSource::ReleaseV1 => Self::from_fixed_path(),
            IntradaySessionWindowSource::OperationalV1 => from_operational_path(),
        }
    }

    /// Resolve the source setting from the process environment and then use
    /// the common loader. Non-UTF-8 or unknown source values fail closed.
    pub fn from_configured_source() -> Result<Self, IntradaySessionWindowError> {
        let source = std::env::var_os(INTRADAY_SESSION_WINDOWS_SOURCE_ENV)
            .map(|value| {
                let value = value
                    .into_string()
                    .map_err(|_| IntradaySessionWindowError::Invalid)?;
                IntradaySessionWindowSource::from_optional_str(Some(&value))
                    .map_err(|_| IntradaySessionWindowError::Invalid)
            })
            .transpose()?
            .unwrap_or(IntradaySessionWindowSource::ReleaseV1);
        Self::from_source(source)
    }

    /// Provider-free fixture seam for the operational descriptor and exact
    /// immutable content bytes. No path or filesystem metadata is accepted.
    pub fn from_operational_bytes(
        activation_bytes: &[u8],
        window_bytes: &[u8],
    ) -> Result<Self, IntradaySessionWindowError> {
        let activation = parse_operational_activation(activation_bytes)?;
        let _filename = operational_window_filename(activation.window_sha256())?;
        Self::from_bytes(window_bytes, activation.window_sha256())
    }

    pub fn window_contract_sha256(&self) -> &str {
        &self.window_contract_sha256
    }

    pub fn entry(&self, date: NaiveDate) -> Option<&IntradaySessionWindowEntry> {
        self.entries.get(&date)
    }

    /// Resolve only the exact KST civil date. No weekday, nearest-date, or
    /// calendar inference is possible here.
    pub fn state_at(&self, now: DateTime<Utc>, halted: bool) -> IntradayMarketState {
        let offset = FixedOffset::east_opt(KST_SECONDS_EAST).expect("valid KST offset");
        let date = now.with_timezone(&offset).date_naive();
        let Some(entry) = self.entries.get(&date) else {
            return IntradayMarketState::Unknown;
        };
        // Session-window evidence is valid only for the same, non-future KST
        // civil date as both the selected entry and this evaluation instant.
        let evidence_date = entry
            .evidence_retrieved_at
            .with_timezone(&offset)
            .date_naive();
        if entry.evidence_retrieved_at > now || evidence_date != entry.date || evidence_date != date
        {
            return IntradayMarketState::Unknown;
        }
        let Some((open, close)) = entry.utc_bounds() else {
            return IntradayMarketState::Closed;
        };
        if now < open || now >= close {
            IntradayMarketState::Closed
        } else if halted {
            IntradayMarketState::Halted
        } else {
            IntradayMarketState::Open
        }
    }

    pub fn utc_bounds_for(
        &self,
        date: NaiveDate,
    ) -> Result<Option<IntradayUtcBounds>, IntradaySessionWindowError> {
        self.entry(date)
            .map(|entry| {
                if entry.is_closed() {
                    Ok(None)
                } else {
                    entry
                        .utc_bounds()
                        .ok_or(IntradaySessionWindowError::Invalid)
                        .map(Some)
                }
            })
            .transpose()
            .map(|value| value.flatten())
    }
}

pub fn parse_operational_activation(
    bytes: &[u8],
) -> Result<OperationalSessionWindowActivation, IntradaySessionWindowError> {
    if bytes.is_empty() || bytes.len() > MAX_OPERATIONAL_ACTIVATION_BYTES {
        return Err(IntradaySessionWindowError::Invalid);
    }
    let activation: WireOperationalActivation =
        serde_json::from_slice(bytes).map_err(|_| IntradaySessionWindowError::Invalid)?;
    if activation.schema_version != INTRADAY_SESSION_SCHEMA_VERSION
        || !is_prefixed_sha256(&activation.window_sha256)
    {
        return Err(IntradaySessionWindowError::Invalid);
    }
    Ok(OperationalSessionWindowActivation {
        schema_version: activation.schema_version,
        window_sha256: activation.window_sha256,
    })
}

pub fn operational_window_filename(
    window_sha256: &str,
) -> Result<String, IntradaySessionWindowError> {
    let Some(hex) = window_sha256.strip_prefix("sha256:") else {
        return Err(IntradaySessionWindowError::Invalid);
    };
    if !is_prefixed_sha256(window_sha256) {
        return Err(IntradaySessionWindowError::Invalid);
    }
    Ok(format!("windows-{hex}.json"))
}

fn from_operational_path() -> Result<IntradaySessionWindowContract, IntradaySessionWindowError> {
    #[cfg(unix)]
    {
        return unix::load_operational();
    }
    #[cfg(not(unix))]
    {
        Err(IntradaySessionWindowError::Unsupported)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntradayMarketState {
    Unknown,
    Closed,
    Open,
    Halted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntradayQuoteCapture {
    pub quote: IntradayQuote,
    pub metadata: IntradayAttemptMetadata,
}

impl IntradayQuoteCapture {
    pub fn into_parts(self) -> (IntradayQuote, IntradayAttemptMetadata) {
        (self.quote, self.metadata)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IntradayQuoteCaptureError {
    #[error("intraday attempt failed")]
    Attempt {
        error: IntradayAttemptError,
        reservation: Option<IntradayAttemptReservationMetadata>,
    },
    #[error("intraday response parsing failed")]
    Response {
        error: IntradayQuoteError,
        reservation: IntradayAttemptReservationMetadata,
    },
}

impl IntradayQuoteCaptureError {
    pub fn reservation(&self) -> Option<IntradayAttemptReservationMetadata> {
        match self {
            Self::Attempt { reservation, .. } => reservation.clone(),
            Self::Response { reservation, .. } => Some(reservation.clone()),
        }
    }

    pub fn attempt_error(&self) -> Option<&IntradayAttemptError> {
        match self {
            Self::Attempt { error, .. } => Some(error),
            Self::Response { .. } => None,
        }
    }

    pub fn response_error(&self) -> Option<IntradayQuoteError> {
        match self {
            Self::Attempt { .. } => None,
            Self::Response { error, .. } => Some(*error),
        }
    }
}

/// Parse a successful guarded attempt, or preserve its real reservation on a
/// bounded typed failure. Parsing never creates a receipt or network request.
pub fn parse_intraday_attempt(
    requested_symbol: &str,
    outcome: IntradayAttemptOutcome,
) -> Result<IntradayQuoteCapture, IntradayQuoteCaptureError> {
    match outcome {
        IntradayAttemptOutcome::Success(reply) => {
            let (market_data, metadata) = reply.into_parts();
            match parse_intraday_quote(requested_symbol, &market_data.body) {
                Ok(quote) => Ok(IntradayQuoteCapture { quote, metadata }),
                Err(error) => Err(IntradayQuoteCaptureError::Response {
                    error,
                    reservation: metadata.reservation_metadata(),
                }),
            }
        }
        IntradayAttemptOutcome::Failed { error, reservation } => {
            Err(IntradayQuoteCaptureError::Attempt { error, reservation })
        }
    }
}

pub fn intraday_quote_query(
    symbol: &str,
) -> Result<Vec<(String, String)>, IntradaySessionWindowError> {
    if symbol.len() != 6 || !symbol.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(IntradaySessionWindowError::Invalid);
    }
    Ok(vec![
        ("FID_COND_MRKT_DIV_CODE".to_owned(), "J".to_owned()),
        ("FID_INPUT_ISCD".to_owned(), symbol.to_owned()),
    ])
}

fn is_prefixed_sha256(value: &str) -> bool {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64
        && hex
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireOperationalActivation {
    schema_version: u32,
    window_sha256: String,
}

#[cfg(unix)]
mod unix {
    use super::*;
    use rustix::fs::{FileType, Mode, OFlags, fstat, open, openat};
    use rustix::io::Errno;
    use std::fs::File;
    use std::os::fd::{AsFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;

    const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
        .union(OFlags::DIRECTORY)
        .union(OFlags::NOFOLLOW)
        .union(OFlags::CLOEXEC);
    const FILE_FLAGS: OFlags = OFlags::RDONLY
        .union(OFlags::NONBLOCK)
        .union(OFlags::NOFOLLOW)
        .union(OFlags::CLOEXEC);

    fn map_open_error(error: Errno) -> IntradaySessionWindowError {
        if error == Errno::NOENT {
            IntradaySessionWindowError::Missing
        } else {
            IntradaySessionWindowError::Invalid
        }
    }

    fn stat_metadata(stat: &rustix::fs::Stat) -> OperationalSessionWindowMetadata {
        let file_type = FileType::from_raw_mode(stat.st_mode);
        OperationalSessionWindowMetadata {
            is_directory: file_type == FileType::Directory,
            // Every opened component and file is opened with NOFOLLOW. A
            // symlink therefore fails before fstat; the explicit field keeps
            // the metadata validation seam useful for provider-free fixtures.
            is_symlink: false,
            is_regular: file_type == FileType::RegularFile,
            uid: stat.st_uid,
            gid: stat.st_gid,
            mode: Mode::from_raw_mode(stat.st_mode).bits() & 0o7777,
            link_count: stat.st_nlink,
        }
    }

    fn open_operational_root() -> Result<OwnedFd, IntradaySessionWindowError> {
        let components = Path::new(INTRADAY_OPERATIONAL_SESSION_WINDOWS_PATH)
            .components()
            .filter_map(|component| match component {
                std::path::Component::Normal(name) => Some(name.as_bytes().to_owned()),
                std::path::Component::RootDir => None,
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut directory =
            open(Path::new("/"), DIRECTORY_FLAGS, Mode::empty()).map_err(map_open_error)?;
        for component in components {
            let next = openat(&directory, &component, DIRECTORY_FLAGS, Mode::empty())
                .map_err(map_open_error)?;
            let stat = fstat(&next).map_err(|_| IntradaySessionWindowError::Invalid)?;
            let metadata = stat_metadata(&stat);
            validate_operational_ancestor_metadata(metadata)?;
            directory = next;
        }
        let stat = fstat(&directory).map_err(|_| IntradaySessionWindowError::Invalid)?;
        validate_operational_directory_metadata(stat_metadata(&stat))?;
        Ok(directory)
    }

    fn read_file(
        directory: &impl AsFd,
        name: &[u8],
        max_bytes: usize,
    ) -> Result<Vec<u8>, IntradaySessionWindowError> {
        let fd = openat(directory, name, FILE_FLAGS, Mode::empty()).map_err(map_open_error)?;
        let mut file = File::from(fd);
        let before = fstat(&file).map_err(|_| IntradaySessionWindowError::Invalid)?;
        let before_type = FileType::from_raw_mode(before.st_mode);
        if before_type != FileType::RegularFile {
            return Err(IntradaySessionWindowError::Invalid);
        }
        let before_metadata = stat_metadata(&before);
        validate_operational_file_metadata(before_metadata)?;
        let before_size =
            usize::try_from(before.st_size).map_err(|_| IntradaySessionWindowError::Invalid)?;
        if before_size > max_bytes {
            return Err(IntradaySessionWindowError::Invalid);
        }
        let mut bytes = Vec::with_capacity(before_size);
        file.by_ref()
            .take(max_bytes.saturating_add(1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| IntradaySessionWindowError::Invalid)?;
        if bytes.len() > max_bytes {
            return Err(IntradaySessionWindowError::Invalid);
        }
        let after = fstat(&file).map_err(|_| IntradaySessionWindowError::Invalid)?;
        let after_metadata = stat_metadata(&after);
        validate_operational_file_metadata(after_metadata)?;
        if after.st_dev != before.st_dev
            || after.st_ino != before.st_ino
            || after.st_size != before.st_size
            || bytes.len() != before_size
        {
            return Err(IntradaySessionWindowError::Invalid);
        }
        Ok(bytes)
    }

    pub(super) fn load_operational()
    -> Result<IntradaySessionWindowContract, IntradaySessionWindowError> {
        let directory = open_operational_root()?;
        let activation_bytes = read_file(
            &directory,
            INTRADAY_OPERATIONAL_SESSION_WINDOWS_ACTIVATION.as_bytes(),
            MAX_OPERATIONAL_ACTIVATION_BYTES,
        )?;
        let activation = parse_operational_activation(&activation_bytes)?;
        let filename = operational_window_filename(activation.window_sha256())?;
        let window_bytes = read_file(&directory, filename.as_bytes(), MAX_WINDOW_BYTES)?;
        IntradaySessionWindowContract::from_bytes(&window_bytes, activation.window_sha256())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireContract {
    schema_version: u32,
    exchange: String,
    timezone: String,
    entries: Vec<WireEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireEntry {
    date: String,
    disposition: WireDisposition,
    open_local: RequiredNullableString,
    close_local: RequiredNullableString,
    evidence_url: String,
    evidence_retrieved_at: String,
    evidence_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(transparent)]
struct RequiredNullableString(Option<String>);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum WireDisposition {
    Regular,
    Special,
    Closed,
}

impl WireEntry {
    fn into_entry(self) -> Result<IntradaySessionWindowEntry, IntradaySessionWindowError> {
        let date = NaiveDate::parse_from_str(&self.date, "%Y-%m-%d")
            .map_err(|_| IntradaySessionWindowError::Invalid)?;
        let parse_time = |value: Option<String>| {
            value
                .map(|value| {
                    NaiveTime::parse_from_str(&value, "%H:%M:%S")
                        .map_err(|_| IntradaySessionWindowError::Invalid)
                })
                .transpose()
        };
        let open_local = parse_time(self.open_local.0)?;
        let close_local = parse_time(self.close_local.0)?;
        let evidence_retrieved_at = DateTime::parse_from_rfc3339(&self.evidence_retrieved_at)
            .map_err(|_| IntradaySessionWindowError::Invalid)?
            .with_timezone(&Utc);
        let disposition = match self.disposition {
            WireDisposition::Regular => IntradaySessionDisposition::Regular,
            WireDisposition::Special => IntradaySessionDisposition::Special,
            WireDisposition::Closed => IntradaySessionDisposition::Closed,
        };
        Ok(IntradaySessionWindowEntry {
            date,
            disposition,
            open_local,
            close_local,
            evidence_url: self.evidence_url,
            evidence_retrieved_at,
            evidence_sha256: self.evidence_sha256,
        })
    }
}
