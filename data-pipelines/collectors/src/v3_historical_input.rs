//! Provider-free loader for the approved V3 historical price-only inputs.
//!
//! This module is the only collectors-side Raw read boundary for the V3
//! replay checker and artifact command.  It reads an already committed Raw
//! manifest, independently authenticates the exact `batch.json` and manifest
//! JSONL bytes, and then delegates schema/coverage checks to the market-data
//! verifiers.  It never contacts a provider, writes a store, or carries a
//! provider diagnostic in an error.

use std::fs::File;
use std::io::{Read, Take};
use std::path::{Component, Path};

use domain::{BatchId, ContentHash};
use market_data::contract::StoredFile;
use market_data::range_to_canonical_v3::{
    HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_ID, HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_JSON_SHA256,
    HISTORICAL_PRICE_ONLY_V3_ACTION_FILE_COUNT,
    HISTORICAL_PRICE_ONLY_V3_ACTION_MANIFEST_LINE_SHA256, HistoricalPriceOnlyV3ActionError,
    HistoricalPriceOnlyV3ActionEvidence, verify_historical_price_only_v3_action_input,
};
use market_data::range_to_canonical_v3_price::{
    HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_ID, HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_JSON_SHA256,
    HISTORICAL_PRICE_ONLY_V3_PRICE_FILE_COUNT, HISTORICAL_PRICE_ONLY_V3_PRICE_MANIFEST_LINE_SHA256,
    HistoricalPriceOnlyV3PriceError, HistoricalPriceOnlyV3PriceEvidence,
    verify_historical_price_only_v3_price_input,
};
use market_data::storage::{ManifestEntry, RawStore};
use market_data::{MARKET_KR, PROVIDER_KIS, PROVIDER_KIS_DAILY_RANGE};
use thiserror::Error;

/// The maximum accepted immutable `batch.json` size.
pub const BATCH_JSON_MAX_BYTES: u64 = 1024 * 1024;
/// The maximum accepted committed manifest size.
pub const MANIFEST_MAX_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
struct SourceSpec {
    provider: &'static str,
    batch_id: &'static str,
    file_count: usize,
    batch_json_sha256: &'static str,
    manifest_line_sha256: &'static str,
}

const PRICE_SOURCE: SourceSpec = SourceSpec {
    provider: PROVIDER_KIS_DAILY_RANGE,
    batch_id: HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_ID,
    file_count: HISTORICAL_PRICE_ONLY_V3_PRICE_FILE_COUNT,
    batch_json_sha256: HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_JSON_SHA256,
    manifest_line_sha256: HISTORICAL_PRICE_ONLY_V3_PRICE_MANIFEST_LINE_SHA256,
};

const ACTION_SOURCE: SourceSpec = SourceSpec {
    provider: PROVIDER_KIS,
    batch_id: HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_ID,
    file_count: HISTORICAL_PRICE_ONLY_V3_ACTION_FILE_COUNT,
    batch_json_sha256: HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_JSON_SHA256,
    manifest_line_sha256: HISTORICAL_PRICE_ONLY_V3_ACTION_MANIFEST_LINE_SHA256,
};

/// Both verified V3 evidence objects returned by [`load_historical_price_only_v3_input`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoricalPriceOnlyV3Input {
    price: HistoricalPriceOnlyV3PriceEvidence,
    action: HistoricalPriceOnlyV3ActionEvidence,
}

impl HistoricalPriceOnlyV3Input {
    /// Verified daily-price evidence.
    pub fn price(&self) -> &HistoricalPriceOnlyV3PriceEvidence {
        &self.price
    }

    /// Alias emphasizing that this value is evidence rather than a source body.
    pub fn price_evidence(&self) -> &HistoricalPriceOnlyV3PriceEvidence {
        self.price()
    }

    /// Verified corporate-action evidence.
    pub fn action(&self) -> &HistoricalPriceOnlyV3ActionEvidence {
        &self.action
    }

    /// Alias emphasizing that this value is evidence rather than a source body.
    pub fn action_evidence(&self) -> &HistoricalPriceOnlyV3ActionEvidence {
        self.action()
    }
}

/// Finite, sanitized failures from the V3 Raw input boundary.
///
/// No variant stores a path, response body, provider message, or filesystem
/// error.  Callers can safely map this enum to a stable public error code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum HistoricalPriceOnlyV3InputError {
    #[error("the Raw root must be absolute")]
    RawRootMustBeAbsolute,
    #[error("this platform cannot provide the required no-follow Raw read")]
    UnsupportedPlatform,
    #[error("the committed Raw store could not be read")]
    RawStore,
    #[error("the approved target batch is missing")]
    TargetBatchMissing,
    #[error("the approved target batch is duplicated")]
    TargetBatchDuplicate,
    #[error("batch metadata could not be read")]
    BatchJsonRead,
    #[error("batch metadata is malformed")]
    BatchJsonMalformed,
    #[error("batch metadata commitment differs")]
    BatchJsonMismatch,
    #[error("the committed manifest could not be read")]
    ManifestRead,
    #[error("the approved manifest record is missing")]
    ManifestLineMissing,
    #[error("the approved manifest record is duplicated")]
    ManifestLineDuplicate,
    #[error("a committed manifest record is malformed")]
    ManifestLineMalformed,
    #[error("the approved manifest record commitment differs")]
    ManifestLineMismatch,
    #[error("the approved source file count differs")]
    SourceFileCount,
    #[error("the price source identity is not approved")]
    PriceInvalidSource,
    #[error("the price source matrix is incomplete")]
    PriceIncompleteMatrix,
    #[error("the price source file metadata is invalid")]
    PriceInvalidFileMetadata,
    #[error("the price source pagination metadata is invalid")]
    PriceInvalidPagination,
    #[error("the price stored-file commitment differs")]
    PriceStoredFileMismatch,
    #[error("the price response is invalid")]
    PriceInvalidResponse,
    #[error("the price source has a symbol conflict")]
    PriceSymbolConflict,
    #[error("the price source has a duplicate observation")]
    PriceDuplicateObservation,
    #[error("the price source coverage is invalid")]
    PriceInvalidCoverage,
    #[error("the action source identity is not approved")]
    ActionInvalidSource,
    #[error("the action source matrix is incomplete")]
    ActionIncompleteMatrix,
    #[error("the action source file metadata is invalid")]
    ActionInvalidFileMetadata,
    #[error("the action source pagination metadata is invalid")]
    ActionInvalidPagination,
    #[error("the action stored-file commitment differs")]
    ActionStoredFileMismatch,
    #[error("the action response is invalid")]
    ActionInvalidResponse,
    #[error("the action source has a symbol conflict")]
    ActionSymbolConflict,
    #[error("the action source contains an unsupported action")]
    ActionUnsupported,
    #[error("the price and action policies disagree")]
    PolicyMismatch,
}

impl HistoricalPriceOnlyV3InputError {
    /// Stable snake_case code used by the operator-facing CLIs.
    pub const fn error_code(self) -> &'static str {
        match self {
            Self::RawRootMustBeAbsolute => "raw_root_must_be_absolute",
            Self::UnsupportedPlatform => "unsupported_platform",
            Self::RawStore => "raw_store",
            Self::TargetBatchMissing => "target_batch_missing",
            Self::TargetBatchDuplicate => "target_batch_duplicate",
            Self::BatchJsonRead => "batch_json_read",
            Self::BatchJsonMalformed => "batch_json_malformed",
            Self::BatchJsonMismatch => "batch_json_mismatch",
            Self::ManifestRead => "manifest_read",
            Self::ManifestLineMissing => "manifest_line_missing",
            Self::ManifestLineDuplicate => "manifest_line_duplicate",
            Self::ManifestLineMalformed => "manifest_line_malformed",
            Self::ManifestLineMismatch => "manifest_line_mismatch",
            Self::SourceFileCount => "source_file_count",
            Self::PriceInvalidSource => "price_invalid_source",
            Self::PriceIncompleteMatrix => "price_incomplete_matrix",
            Self::PriceInvalidFileMetadata => "price_invalid_file_metadata",
            Self::PriceInvalidPagination => "price_invalid_pagination",
            Self::PriceStoredFileMismatch => "price_stored_file_mismatch",
            Self::PriceInvalidResponse => "price_invalid_response",
            Self::PriceSymbolConflict => "price_symbol_conflict",
            Self::PriceDuplicateObservation => "price_duplicate_observation",
            Self::PriceInvalidCoverage => "price_invalid_coverage",
            Self::ActionInvalidSource => "action_invalid_source",
            Self::ActionIncompleteMatrix => "action_incomplete_matrix",
            Self::ActionInvalidFileMetadata => "action_invalid_file_metadata",
            Self::ActionInvalidPagination => "action_invalid_pagination",
            Self::ActionStoredFileMismatch => "action_stored_file_mismatch",
            Self::ActionInvalidResponse => "action_invalid_response",
            Self::ActionSymbolConflict => "action_symbol_conflict",
            Self::ActionUnsupported => "action_unsupported",
            Self::PolicyMismatch => "policy_mismatch",
        }
    }

    /// Alias for callers that name the field `code`.
    pub const fn as_str(self) -> &'static str {
        self.error_code()
    }
}

/// Load and fully verify the approved V3 price and action Raw batches.
///
/// This function performs no network or write operation.  It requires the
/// existing committed-manifest controls and reads metadata through
/// descriptor-relative no-follow opens before passing the resulting bytes to
/// the market-data verifiers.
pub fn load_historical_price_only_v3_input(
    raw_root: &Path,
) -> Result<HistoricalPriceOnlyV3Input, HistoricalPriceOnlyV3InputError> {
    if !raw_root.is_absolute() {
        return Err(HistoricalPriceOnlyV3InputError::RawRootMustBeAbsolute);
    }
    validate_directory_no_follow(&raw_root.join("raw")).map_err(map_raw_root_read_error)?;

    let store = RawStore::new(raw_root);
    let price = read_source(&store, PRICE_SOURCE)?;
    let action = read_source(&store, ACTION_SOURCE)?;
    let price_verified = verify_historical_price_only_v3_price_input(
        &price.entry,
        &price.stored,
        &price.batch_json_hash,
        &price.manifest_line_hash,
    )
    .map_err(map_price_verification_error)?;
    let action_verified = verify_historical_price_only_v3_action_input(
        &action.entry,
        &action.stored,
        &action.batch_json_hash,
        &action.manifest_line_hash,
    )
    .map_err(map_action_verification_error)?;
    if price_verified.vendor_snapshot() != action_verified.vendor_snapshot()
        || price_verified.strict_pit() != action_verified.strict_pit()
        || price_verified.pit_policy() != action_verified.pit_policy()
    {
        return Err(HistoricalPriceOnlyV3InputError::PolicyMismatch);
    }

    Ok(HistoricalPriceOnlyV3Input {
        price: price_verified,
        action: action_verified,
    })
}

/// Compatibility spelling for callers that treat the loader as a verifier.
pub fn verify_historical_price_only_v3_input(
    raw_root: &Path,
) -> Result<HistoricalPriceOnlyV3Input, HistoricalPriceOnlyV3InputError> {
    load_historical_price_only_v3_input(raw_root)
}

struct SourceInput {
    entry: ManifestEntry,
    stored: Vec<StoredFile>,
    batch_json_hash: ContentHash,
    manifest_line_hash: ContentHash,
}

fn read_source(
    store: &RawStore,
    source: SourceSpec,
) -> Result<SourceInput, HistoricalPriceOnlyV3InputError> {
    // The strict reader requires the existing commit lock and manifest.  It
    // does not discover, reconcile, or expose orphan batches.
    let entries = store
        .read_committed_manifest(source.provider, MARKET_KR)
        .map_err(|_| HistoricalPriceOnlyV3InputError::RawStore)?;
    let target_id = source
        .batch_id
        .parse::<BatchId>()
        .map_err(|_| HistoricalPriceOnlyV3InputError::TargetBatchMissing)?;
    let matching = entries
        .iter()
        .filter(|entry| entry.batch_id == target_id)
        .collect::<Vec<_>>();
    let entry = match matching.as_slice() {
        [] => return Err(HistoricalPriceOnlyV3InputError::TargetBatchMissing),
        [entry] => (*entry).clone(),
        _ => return Err(HistoricalPriceOnlyV3InputError::TargetBatchDuplicate),
    };
    if entry.files.len() != source.file_count {
        return Err(HistoricalPriceOnlyV3InputError::SourceFileCount);
    }

    // RawStore verifies each immutable leaf hash before the verifier sees it.
    let stored = store
        .read_batch_bytes(source.provider, MARKET_KR, &entry)
        .map_err(|_| HistoricalPriceOnlyV3InputError::RawStore)?;

    let batch_json_path = store
        .batch_dir(source.provider, MARKET_KR, &entry.date, &entry.batch_id)
        .join(entry.batch_json_file_name());
    let batch_json = read_regular_file_no_follow(&batch_json_path, BATCH_JSON_MAX_BYTES)
        .map_err(map_batch_json_read_error)?;
    let batch_json_hash = parse_batch_json(&batch_json, &entry)?;
    let expected_batch_json_hash = ContentHash::parse(source.batch_json_sha256)
        .map_err(|_| HistoricalPriceOnlyV3InputError::BatchJsonMismatch)?;
    if batch_json_hash != expected_batch_json_hash {
        return Err(HistoricalPriceOnlyV3InputError::BatchJsonMismatch);
    }

    let manifest_path = store.manifest_path(source.provider, MARKET_KR);
    let manifest_bytes = read_regular_file_no_follow(&manifest_path, MANIFEST_MAX_BYTES)
        .map_err(map_manifest_read_error)?;
    let manifest_line = exact_manifest_line(&manifest_bytes, &entry)?;
    let manifest_line_hash = ContentHash::from_bytes(manifest_line);
    let expected_manifest_line_hash = ContentHash::parse(source.manifest_line_sha256)
        .map_err(|_| HistoricalPriceOnlyV3InputError::ManifestLineMismatch)?;
    if manifest_line_hash != expected_manifest_line_hash {
        return Err(HistoricalPriceOnlyV3InputError::ManifestLineMismatch);
    }

    Ok(SourceInput {
        entry,
        stored,
        batch_json_hash,
        manifest_line_hash,
    })
}

fn parse_batch_json(
    bytes: &[u8],
    entry: &ManifestEntry,
) -> Result<ContentHash, HistoricalPriceOnlyV3InputError> {
    let parsed = serde_json::from_slice::<ManifestEntry>(bytes)
        .map_err(|_| HistoricalPriceOnlyV3InputError::BatchJsonMalformed)?;
    let parsed_value = serde_json::from_slice::<serde_json::Value>(bytes)
        .map_err(|_| HistoricalPriceOnlyV3InputError::BatchJsonMalformed)?;
    let expected_value = serde_json::to_value(entry)
        .map_err(|_| HistoricalPriceOnlyV3InputError::BatchJsonMismatch)?;
    if parsed != *entry || parsed_value != expected_value {
        return Err(HistoricalPriceOnlyV3InputError::BatchJsonMismatch);
    }
    Ok(ContentHash::from_bytes(bytes))
}

fn exact_manifest_line<'a>(
    bytes: &'a [u8],
    entry: &ManifestEntry,
) -> Result<&'a [u8], HistoricalPriceOnlyV3InputError> {
    let mut matching = None;
    let mut matching_count = 0usize;
    let mut same_batch_count = 0usize;
    let expected_value = serde_json::to_value(entry)
        .map_err(|_| HistoricalPriceOnlyV3InputError::ManifestLineMismatch)?;
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        let Some(payload) = line.strip_suffix(b"\n") else {
            continue;
        };
        if payload.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let parsed = serde_json::from_slice::<ManifestEntry>(payload)
            .map_err(|_| HistoricalPriceOnlyV3InputError::ManifestLineMalformed)?;
        if parsed.batch_id != entry.batch_id {
            continue;
        }
        same_batch_count += 1;
        let parsed_value = serde_json::from_slice::<serde_json::Value>(payload)
            .map_err(|_| HistoricalPriceOnlyV3InputError::ManifestLineMalformed)?;
        if parsed != *entry || parsed_value != expected_value {
            return Err(HistoricalPriceOnlyV3InputError::ManifestLineMismatch);
        }
        matching_count += 1;
        matching = Some(line);
    }
    if same_batch_count == 0 {
        return Err(HistoricalPriceOnlyV3InputError::ManifestLineMissing);
    }
    if matching_count != 1 {
        return Err(HistoricalPriceOnlyV3InputError::ManifestLineDuplicate);
    }
    matching.ok_or(HistoricalPriceOnlyV3InputError::ManifestLineMissing)
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SafeReadError {
    UnsupportedPlatform,
    Io,
    NotRegular,
    TooLarge,
}

fn map_raw_root_read_error(error: SafeReadError) -> HistoricalPriceOnlyV3InputError {
    match error {
        SafeReadError::UnsupportedPlatform => HistoricalPriceOnlyV3InputError::UnsupportedPlatform,
        SafeReadError::Io | SafeReadError::NotRegular | SafeReadError::TooLarge => {
            HistoricalPriceOnlyV3InputError::RawStore
        }
    }
}

fn map_batch_json_read_error(error: SafeReadError) -> HistoricalPriceOnlyV3InputError {
    match error {
        SafeReadError::UnsupportedPlatform => HistoricalPriceOnlyV3InputError::UnsupportedPlatform,
        SafeReadError::TooLarge | SafeReadError::Io | SafeReadError::NotRegular => {
            HistoricalPriceOnlyV3InputError::BatchJsonRead
        }
    }
}

fn map_manifest_read_error(error: SafeReadError) -> HistoricalPriceOnlyV3InputError {
    match error {
        SafeReadError::UnsupportedPlatform => HistoricalPriceOnlyV3InputError::UnsupportedPlatform,
        SafeReadError::TooLarge | SafeReadError::Io | SafeReadError::NotRegular => {
            HistoricalPriceOnlyV3InputError::ManifestRead
        }
    }
}

fn map_price_verification_error(
    error: HistoricalPriceOnlyV3PriceError,
) -> HistoricalPriceOnlyV3InputError {
    match error {
        HistoricalPriceOnlyV3PriceError::InvalidSource => {
            HistoricalPriceOnlyV3InputError::PriceInvalidSource
        }
        HistoricalPriceOnlyV3PriceError::IncompleteMatrix => {
            HistoricalPriceOnlyV3InputError::PriceIncompleteMatrix
        }
        HistoricalPriceOnlyV3PriceError::InvalidFileMetadata => {
            HistoricalPriceOnlyV3InputError::PriceInvalidFileMetadata
        }
        HistoricalPriceOnlyV3PriceError::InvalidPagination => {
            HistoricalPriceOnlyV3InputError::PriceInvalidPagination
        }
        HistoricalPriceOnlyV3PriceError::StoredFileMismatch => {
            HistoricalPriceOnlyV3InputError::PriceStoredFileMismatch
        }
        HistoricalPriceOnlyV3PriceError::InvalidResponse => {
            HistoricalPriceOnlyV3InputError::PriceInvalidResponse
        }
        HistoricalPriceOnlyV3PriceError::SymbolConflict => {
            HistoricalPriceOnlyV3InputError::PriceSymbolConflict
        }
        HistoricalPriceOnlyV3PriceError::DuplicateObservation => {
            HistoricalPriceOnlyV3InputError::PriceDuplicateObservation
        }
        HistoricalPriceOnlyV3PriceError::InvalidCoverage => {
            HistoricalPriceOnlyV3InputError::PriceInvalidCoverage
        }
    }
}

fn map_action_verification_error(
    error: HistoricalPriceOnlyV3ActionError,
) -> HistoricalPriceOnlyV3InputError {
    match error {
        HistoricalPriceOnlyV3ActionError::InvalidSource => {
            HistoricalPriceOnlyV3InputError::ActionInvalidSource
        }
        HistoricalPriceOnlyV3ActionError::IncompleteMatrix => {
            HistoricalPriceOnlyV3InputError::ActionIncompleteMatrix
        }
        HistoricalPriceOnlyV3ActionError::InvalidFileMetadata => {
            HistoricalPriceOnlyV3InputError::ActionInvalidFileMetadata
        }
        HistoricalPriceOnlyV3ActionError::InvalidPagination => {
            HistoricalPriceOnlyV3InputError::ActionInvalidPagination
        }
        HistoricalPriceOnlyV3ActionError::StoredFileMismatch => {
            HistoricalPriceOnlyV3InputError::ActionStoredFileMismatch
        }
        HistoricalPriceOnlyV3ActionError::InvalidResponse => {
            HistoricalPriceOnlyV3InputError::ActionInvalidResponse
        }
        HistoricalPriceOnlyV3ActionError::SymbolConflict => {
            HistoricalPriceOnlyV3InputError::ActionSymbolConflict
        }
        HistoricalPriceOnlyV3ActionError::UnsupportedAction { .. } => {
            HistoricalPriceOnlyV3InputError::ActionUnsupported
        }
    }
}

#[cfg(unix)]
fn validate_directory_no_follow(path: &Path) -> Result<(), SafeReadError> {
    use rustix::fs::{FileType, Mode, OFlags, fstat, open, openat};
    use rustix::io::Errno;

    fn open_error(_error: Errno) -> SafeReadError {
        SafeReadError::Io
    }

    if !path.is_absolute() {
        return Err(SafeReadError::Io);
    }
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => components.push(name.to_owned()),
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                return Err(SafeReadError::Io);
            }
        }
    }
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mut directory = open("/", flags, Mode::empty()).map_err(open_error)?;
    for component in components {
        let next = openat(&directory, &component, flags, Mode::empty()).map_err(open_error)?;
        let stat = fstat(&next).map_err(|_| SafeReadError::Io)?;
        if FileType::from_raw_mode(stat.st_mode) != FileType::Directory {
            return Err(SafeReadError::NotRegular);
        }
        directory = next;
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_directory_no_follow(_path: &Path) -> Result<(), SafeReadError> {
    Err(SafeReadError::UnsupportedPlatform)
}

#[cfg(unix)]
fn read_regular_file_no_follow(path: &Path, max_bytes: u64) -> Result<Vec<u8>, SafeReadError> {
    use rustix::fs::{FileType, Mode, OFlags, fstat, open, openat};
    use rustix::io::Errno;

    fn open_error(error: Errno) -> SafeReadError {
        // ELOOP is O_NOFOLLOW's symlink signal.  It is intentionally mapped
        // to the same safe outward result as every other read failure.
        let _is_symlink = error == Errno::LOOP;
        SafeReadError::Io
    }

    if !path.is_absolute() {
        return Err(SafeReadError::Io);
    }
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => components.push(name.to_owned()),
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                return Err(SafeReadError::Io);
            }
        }
    }
    let leaf = components.pop().ok_or(SafeReadError::Io)?;
    let directory_flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let file_flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mut directory = open("/", directory_flags, Mode::empty()).map_err(open_error)?;
    for component in components {
        let next =
            openat(&directory, &component, directory_flags, Mode::empty()).map_err(open_error)?;
        let stat = fstat(&next).map_err(|_| SafeReadError::Io)?;
        if FileType::from_raw_mode(stat.st_mode) != FileType::Directory {
            return Err(SafeReadError::NotRegular);
        }
        directory = next;
    }
    let fd = openat(&directory, &leaf, file_flags, Mode::empty()).map_err(open_error)?;
    let mut file = File::from(fd);
    let before = fstat(&file).map_err(|_| SafeReadError::Io)?;
    if FileType::from_raw_mode(before.st_mode) != FileType::RegularFile {
        return Err(SafeReadError::NotRegular);
    }
    let before_size = u64::try_from(before.st_size).map_err(|_| SafeReadError::Io)?;
    if before_size > max_bytes {
        return Err(SafeReadError::TooLarge);
    }
    let mut bytes = Vec::with_capacity(before_size as usize);
    let mut bounded: Take<&mut File> = (&mut file).take(max_bytes.saturating_add(1));
    bounded
        .read_to_end(&mut bytes)
        .map_err(|_| SafeReadError::Io)?;
    if bytes.len() as u64 > max_bytes {
        return Err(SafeReadError::TooLarge);
    }
    let after = fstat(&file).map_err(|_| SafeReadError::Io)?;
    if FileType::from_raw_mode(after.st_mode) != FileType::RegularFile
        || u64::try_from(after.st_size).ok() != Some(before_size)
        || bytes.len() as u64 != before_size
    {
        return Err(SafeReadError::Io);
    }
    Ok(bytes)
}

#[cfg(not(unix))]
fn read_regular_file_no_follow(_path: &Path, _max_bytes: u64) -> Result<Vec<u8>, SafeReadError> {
    Err(SafeReadError::UnsupportedPlatform)
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::UtcTimestamp;
    use market_data::contract::{FetchMode, RequestMetadata, ResponseKind};

    fn fixture_entry_for(source: SourceSpec) -> ManifestEntry {
        let batch_id = source.batch_id.parse::<BatchId>().unwrap();
        ManifestEntry {
            batch_id,
            provider: source.provider.to_owned(),
            market: MARKET_KR.to_owned(),
            date: "2016-08-29".parse().unwrap(),
            retrieved_at: UtcTimestamp::parse_rfc3339("2026-08-29T00:00:00Z").unwrap(),
            mode: FetchMode::Credentialed,
            entitlement_reference: None,
            files: vec![market_data::FileEntry {
                kind: ResponseKind::CorporateActions,
                file_name: "fixture.json".to_owned(),
                content_hash: ContentHash::from_bytes(b"fixture"),
                size_bytes: 7,
                request: RequestMetadata {
                    endpoint: "fixture".to_owned(),
                    query: Vec::new(),
                    headers: Vec::new(),
                    mode: FetchMode::Credentialed,
                },
                response_continuation: Some("E".to_owned()),
            }],
        }
    }

    fn fixture_entry() -> ManifestEntry {
        fixture_entry_for(ACTION_SOURCE)
    }

    fn compact_line(entry: &ManifestEntry) -> Vec<u8> {
        let mut line = serde_json::to_vec(entry).unwrap();
        line.push(b'\n');
        line
    }

    #[test]
    fn typed_verification_failures_keep_price_and_action_boundaries_distinct() {
        assert_eq!(
            map_price_verification_error(HistoricalPriceOnlyV3PriceError::InvalidPagination),
            HistoricalPriceOnlyV3InputError::PriceInvalidPagination
        );
        assert_eq!(
            map_action_verification_error(HistoricalPriceOnlyV3ActionError::InvalidPagination),
            HistoricalPriceOnlyV3InputError::ActionInvalidPagination
        );
        assert_ne!(
            HistoricalPriceOnlyV3InputError::PriceInvalidPagination.error_code(),
            HistoricalPriceOnlyV3InputError::ActionInvalidPagination.error_code()
        );
    }

    #[test]
    fn both_source_manifests_bind_distinct_batch_scopes_and_pins() {
        let store = RawStore::new("/data");
        assert_eq!(PRICE_SOURCE.provider, PROVIDER_KIS_DAILY_RANGE);
        assert_eq!(
            PRICE_SOURCE.batch_id,
            HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_ID
        );
        assert_eq!(
            PRICE_SOURCE.file_count,
            HISTORICAL_PRICE_ONLY_V3_PRICE_FILE_COUNT
        );
        assert_eq!(ACTION_SOURCE.provider, PROVIDER_KIS);
        assert_eq!(
            ACTION_SOURCE.batch_id,
            HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_ID
        );
        assert_eq!(
            ACTION_SOURCE.file_count,
            HISTORICAL_PRICE_ONLY_V3_ACTION_FILE_COUNT
        );
        assert_eq!(
            store.manifest_path(PRICE_SOURCE.provider, MARKET_KR),
            std::path::PathBuf::from(
                "/data/raw/manifests/provider=kis-daily-range/market=kr/manifest.jsonl"
            )
        );
        assert_eq!(
            store.manifest_path(ACTION_SOURCE.provider, MARKET_KR),
            std::path::PathBuf::from("/data/raw/manifests/provider=kis/market=kr/manifest.jsonl")
        );

        for (source, batch_pin, line_pin) in [
            (
                PRICE_SOURCE,
                HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_JSON_SHA256,
                HISTORICAL_PRICE_ONLY_V3_PRICE_MANIFEST_LINE_SHA256,
            ),
            (
                ACTION_SOURCE,
                HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_JSON_SHA256,
                HISTORICAL_PRICE_ONLY_V3_ACTION_MANIFEST_LINE_SHA256,
            ),
        ] {
            let entry = fixture_entry_for(source);
            let batch_json = serde_json::to_vec_pretty(&entry).unwrap();
            let batch_hash = parse_batch_json(&batch_json, &entry).unwrap();
            let line = compact_line(&entry);
            let selected = exact_manifest_line(&line, &entry).unwrap();
            assert_eq!(selected, line.as_slice());
            assert_eq!(batch_hash, ContentHash::from_bytes(&batch_json));
            assert_eq!(
                ContentHash::parse(batch_pin).unwrap().to_string(),
                batch_pin
            );
            assert_eq!(ContentHash::parse(line_pin).unwrap().to_string(), line_pin);
        }
    }

    #[test]
    fn manifest_line_hash_includes_terminating_newline() {
        let entry = fixture_entry();
        let bytes = serde_json::to_vec_pretty(&entry).unwrap();
        let hash = parse_batch_json(&bytes, &entry).unwrap();
        assert_eq!(hash, ContentHash::from_bytes(&bytes));
        let line = compact_line(&entry);
        let selected = exact_manifest_line(&line, &entry).unwrap();
        assert_eq!(selected, line.as_slice());
        assert_eq!(
            ContentHash::from_bytes(selected),
            ContentHash::from_bytes(&line)
        );
        assert_ne!(
            ContentHash::from_bytes(selected),
            ContentHash::from_bytes(&line[..line.len() - 1])
        );
    }

    #[test]
    fn manifest_line_rejects_duplicate_missing_and_conflicting_records() {
        let entry = fixture_entry();
        let line = compact_line(&entry);
        let mut duplicate = line.clone();
        duplicate.extend_from_slice(&line);
        assert_eq!(
            exact_manifest_line(&duplicate, &entry),
            Err(HistoricalPriceOnlyV3InputError::ManifestLineDuplicate)
        );
        assert_eq!(
            exact_manifest_line(&line[..line.len() - 1], &entry),
            Err(HistoricalPriceOnlyV3InputError::ManifestLineMissing)
        );
        assert_eq!(
            exact_manifest_line(b"{\"not\":\"a manifest\"}\n", &entry),
            Err(HistoricalPriceOnlyV3InputError::ManifestLineMalformed)
        );
        let mut conflicting = entry.clone();
        conflicting.market = "other".to_owned();
        assert_eq!(
            exact_manifest_line(&compact_line(&conflicting), &entry),
            Err(HistoricalPriceOnlyV3InputError::ManifestLineMismatch)
        );
    }

    #[test]
    fn batch_json_is_hashed_as_read_and_must_deserialize_equal() {
        let entry = fixture_entry();
        let bytes = serde_json::to_vec_pretty(&entry).unwrap();
        let hash = parse_batch_json(&bytes, &entry).unwrap();
        let mut with_extra_newline = bytes.clone();
        with_extra_newline.push(b'\n');
        let extra_hash = parse_batch_json(&with_extra_newline, &entry).unwrap();
        assert_ne!(hash, extra_hash);
        let mut altered = entry;
        altered.market = "altered".to_owned();
        let altered_bytes = serde_json::to_vec(&altered).unwrap();
        assert_eq!(
            parse_batch_json(&altered_bytes, &fixture_entry()),
            Err(HistoricalPriceOnlyV3InputError::BatchJsonMismatch)
        );
    }

    #[test]
    fn error_codes_are_finite_and_stable() {
        assert_eq!(
            HistoricalPriceOnlyV3InputError::PriceInvalidPagination.error_code(),
            "price_invalid_pagination"
        );
        assert_eq!(
            HistoricalPriceOnlyV3InputError::ActionInvalidPagination.error_code(),
            "action_invalid_pagination"
        );
        assert_ne!(
            HistoricalPriceOnlyV3InputError::PriceInvalidPagination.error_code(),
            HistoricalPriceOnlyV3InputError::ActionInvalidPagination.error_code()
        );
    }

    #[cfg(unix)]
    #[test]
    fn no_follow_reader_rejects_symlink_and_enforces_size_cap() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        let link = temp.path().join("link");
        let nested = temp.path().join("nested");
        let nested_link = temp.path().join("nested-link");
        std::fs::write(&target, b"safe").unwrap();
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(nested.join("target"), b"safe").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        std::os::unix::fs::symlink(&nested, &nested_link).unwrap();
        assert_eq!(
            read_regular_file_no_follow(&link, 1024),
            Err(SafeReadError::Io)
        );
        assert_eq!(
            read_regular_file_no_follow(&nested_link.join("target"), 1024),
            Err(SafeReadError::Io)
        );
        assert_eq!(
            read_regular_file_no_follow(&temp.path().join("."), 1024),
            Err(SafeReadError::NotRegular)
        );
        assert_eq!(
            read_regular_file_no_follow(temp.path(), 1024),
            Err(SafeReadError::NotRegular)
        );
        assert_eq!(
            read_regular_file_no_follow(&target, 3),
            Err(SafeReadError::TooLarge)
        );
        assert_eq!(read_regular_file_no_follow(&target, 4).unwrap(), b"safe");
    }
}
