//! Provider-free operator check for the approved V3 KIS price and action Raw batches.
//!
//! This command is deliberately a read-only boundary.  It reads only an
//! already committed Raw manifests and the two approved batches, performs no
//! network operation, and never writes Raw, Curated, a database, or an
//! approval record.  The batch and manifest metadata are read a second time
//! through descriptor-relative no-follow descriptors so the two source pins
//! passed to the V3 verifier are commitments to the bytes actually inspected
//! by this process.

use std::env;
use std::fs::File;
use std::io::{Read, Take};
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

use domain::{BatchId, ContentHash};
use market_data::StoredFile;
use market_data::range_to_canonical_v3::{
    HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_ID, HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_JSON_SHA256,
    HISTORICAL_PRICE_ONLY_V3_ACTION_FILE_COUNT,
    HISTORICAL_PRICE_ONLY_V3_ACTION_MANIFEST_LINE_SHA256, HistoricalPriceOnlyV3ActionError,
    verify_historical_price_only_v3_action_input,
};
use market_data::range_to_canonical_v3_price::{
    HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_ID, HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_JSON_SHA256,
    HISTORICAL_PRICE_ONLY_V3_PRICE_FILE_COUNT, HISTORICAL_PRICE_ONLY_V3_PRICE_MANIFEST_LINE_SHA256,
    HistoricalPriceOnlyV3PriceError, verify_historical_price_only_v3_price_input,
};
use market_data::storage::{ManifestEntry, RawStore};
use market_data::{MARKET_KR, PROVIDER_KIS, PROVIDER_KIS_DAILY_RANGE};
use serde::Serialize;

/// `batch.json` is metadata only and should remain far below this bound.  The
/// fixed cap also prevents an operator-controlled path from causing an
/// unbounded allocation before JSON validation.
pub const BATCH_JSON_MAX_BYTES: u64 = 1024 * 1024;
/// A production manifest can contain many historical batches, but 64 MiB is a
/// deliberately fixed upper bound for this one-shot read-only checker.
pub const MANIFEST_MAX_BYTES: u64 = 64 * 1024 * 1024;

const USAGE: &str = "kis-historical-price-v3-input-check --raw-root <ABSOLUTE_DATA_ROOT> (--plan | --check)\n\n--plan is provider-free and does not read Raw. --check reads only the committed KIS daily-range price and KIS action Raw manifests and their approved batches; it makes no network, provider, Curated, database, or approval write.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Plan,
    Check,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Args {
    raw_root: PathBuf,
    action: Action,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ErrorCode {
    Usage,
    RawRootMustBeAbsolute,
    UnsupportedPlatform,
    RawStore,
    TargetBatchMissing,
    TargetBatchDuplicate,
    BatchJsonRead,
    BatchJsonMalformed,
    BatchJsonMismatch,
    ManifestRead,
    ManifestLineMissing,
    ManifestLineDuplicate,
    ManifestLineMalformed,
    ManifestLineMismatch,
    SourceFileCount,
    PriceInvalidSource,
    PriceIncompleteMatrix,
    PriceInvalidFileMetadata,
    PriceInvalidPagination,
    PriceStoredFileMismatch,
    PriceInvalidResponse,
    PriceSymbolConflict,
    PriceDuplicateObservation,
    PriceInvalidCoverage,
    ActionInvalidSource,
    ActionIncompleteMatrix,
    ActionInvalidFileMetadata,
    ActionInvalidPagination,
    ActionStoredFileMismatch,
    ActionInvalidResponse,
    ActionSymbolConflict,
    ActionUnsupported,
    PolicyMismatch,
}

impl ErrorCode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Usage => "usage",
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CheckError(ErrorCode);

impl CheckError {
    const fn new(code: ErrorCode) -> Self {
        Self(code)
    }
}

#[derive(Debug, Serialize)]
struct ErrorRecord {
    status: &'static str,
    error_code: &'static str,
}

#[derive(Debug, Serialize)]
struct PlanRecord {
    status: &'static str,
    network: &'static str,
    raw_read: &'static str,
    raw_write: &'static str,
    price_batch_id: &'static str,
    price_file_count: usize,
    action_batch_id: &'static str,
    action_file_count: usize,
}

#[derive(Debug, Serialize)]
struct SuccessRecord {
    status: &'static str,
    price: PriceSummary,
    action: ActionSummary,
    vendor_snapshot: bool,
    strict_pit: bool,
    pit_policy: String,
}

#[derive(Debug, Serialize)]
struct PriceSummary {
    batch_id: String,
    file_count: usize,
    session_count: usize,
    bar_count: usize,
    bars_sha256: String,
    capture_contract_commit: String,
    response_marker_evidence: String,
}

#[derive(Debug, Serialize)]
struct ActionSummary {
    batch_id: String,
    file_count: usize,
    action_count: usize,
    cash_row_count: usize,
    cash_rows_sha256: String,
}

fn main() -> ExitCode {
    let argv = env::args().skip(1).collect::<Vec<_>>();
    match parse_args(&argv) {
        Ok(ParseOutcome::Help) => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Ok(ParseOutcome::Args(args)) => match args.action {
            Action::Plan => {
                print_json(&PlanRecord {
                    status: "plan",
                    network: "none",
                    raw_read: "none",
                    raw_write: "none",
                    price_batch_id: HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_ID,
                    price_file_count: HISTORICAL_PRICE_ONLY_V3_PRICE_FILE_COUNT,
                    action_batch_id: HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_ID,
                    action_file_count: HISTORICAL_PRICE_ONLY_V3_ACTION_FILE_COUNT,
                });
                ExitCode::SUCCESS
            }
            Action::Check => match run_check(&args.raw_root) {
                Ok(record) => {
                    print_json(&record);
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    print_error(error);
                    ExitCode::FAILURE
                }
            },
        },
        Err(error) => {
            print_error(error);
            ExitCode::FAILURE
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ParseOutcome {
    Help,
    Args(Args),
}

fn parse_args(argv: &[String]) -> Result<ParseOutcome, CheckError> {
    if argv.len() == 1 && matches!(argv[0].as_str(), "--help" | "-h") {
        return Ok(ParseOutcome::Help);
    }
    let mut raw_root = None;
    let mut action = None;
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--raw-root" => {
                if raw_root.is_some() {
                    return Err(CheckError::new(ErrorCode::Usage));
                }
                let value = argv
                    .get(index + 1)
                    .ok_or(CheckError::new(ErrorCode::Usage))?;
                raw_root = Some(PathBuf::from(value));
                index += 2;
            }
            "--plan" => {
                if action.replace(Action::Plan).is_some() {
                    return Err(CheckError::new(ErrorCode::Usage));
                }
                index += 1;
            }
            "--check" => {
                if action.replace(Action::Check).is_some() {
                    return Err(CheckError::new(ErrorCode::Usage));
                }
                index += 1;
            }
            _ => return Err(CheckError::new(ErrorCode::Usage)),
        }
    }
    let raw_root = raw_root.ok_or(CheckError::new(ErrorCode::Usage))?;
    if !raw_root.is_absolute() {
        return Err(CheckError::new(ErrorCode::RawRootMustBeAbsolute));
    }
    let action = action.ok_or(CheckError::new(ErrorCode::Usage))?;
    Ok(ParseOutcome::Args(Args { raw_root, action }))
}

fn run_check(raw_root: &Path) -> Result<SuccessRecord, CheckError> {
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
        return Err(CheckError::new(ErrorCode::PolicyMismatch));
    }
    Ok(SuccessRecord {
        status: "ok",
        price: PriceSummary {
            batch_id: price_verified.source_batch_id().to_string(),
            file_count: price_verified.files().len(),
            session_count: price_verified.session_count(),
            bar_count: price_verified.bar_count(),
            bars_sha256: price_verified.bars_sha256().to_string(),
            capture_contract_commit: price_verified.capture_contract_commit().to_owned(),
            response_marker_evidence: price_verified.response_marker_evidence().to_owned(),
        },
        action: ActionSummary {
            batch_id: action_verified.source_batch_id().to_string(),
            file_count: action_verified.files().len(),
            action_count: action_verified.action_count(),
            cash_row_count: action_verified.cash_dividends().row_count(),
            cash_rows_sha256: action_verified.cash_dividends().rows_sha256().to_string(),
        },
        vendor_snapshot: price_verified.vendor_snapshot(),
        strict_pit: price_verified.strict_pit(),
        pit_policy: price_verified.pit_policy().to_owned(),
    })
}

fn map_price_verification_error(error: HistoricalPriceOnlyV3PriceError) -> CheckError {
    let code = match error {
        HistoricalPriceOnlyV3PriceError::InvalidSource => ErrorCode::PriceInvalidSource,
        HistoricalPriceOnlyV3PriceError::IncompleteMatrix => ErrorCode::PriceIncompleteMatrix,
        HistoricalPriceOnlyV3PriceError::InvalidFileMetadata => ErrorCode::PriceInvalidFileMetadata,
        HistoricalPriceOnlyV3PriceError::InvalidPagination => ErrorCode::PriceInvalidPagination,
        HistoricalPriceOnlyV3PriceError::StoredFileMismatch => ErrorCode::PriceStoredFileMismatch,
        HistoricalPriceOnlyV3PriceError::InvalidResponse => ErrorCode::PriceInvalidResponse,
        HistoricalPriceOnlyV3PriceError::SymbolConflict => ErrorCode::PriceSymbolConflict,
        HistoricalPriceOnlyV3PriceError::DuplicateObservation => {
            ErrorCode::PriceDuplicateObservation
        }
        HistoricalPriceOnlyV3PriceError::InvalidCoverage => ErrorCode::PriceInvalidCoverage,
    };
    CheckError::new(code)
}

fn map_action_verification_error(error: HistoricalPriceOnlyV3ActionError) -> CheckError {
    let code = match error {
        HistoricalPriceOnlyV3ActionError::InvalidSource => ErrorCode::ActionInvalidSource,
        HistoricalPriceOnlyV3ActionError::IncompleteMatrix => ErrorCode::ActionIncompleteMatrix,
        HistoricalPriceOnlyV3ActionError::InvalidFileMetadata => {
            ErrorCode::ActionInvalidFileMetadata
        }
        HistoricalPriceOnlyV3ActionError::InvalidPagination => ErrorCode::ActionInvalidPagination,
        HistoricalPriceOnlyV3ActionError::StoredFileMismatch => ErrorCode::ActionStoredFileMismatch,
        HistoricalPriceOnlyV3ActionError::InvalidResponse => ErrorCode::ActionInvalidResponse,
        HistoricalPriceOnlyV3ActionError::SymbolConflict => ErrorCode::ActionSymbolConflict,
        HistoricalPriceOnlyV3ActionError::UnsupportedAction { .. } => ErrorCode::ActionUnsupported,
    };
    CheckError::new(code)
}

fn map_raw_root_read_error(error: SafeReadError) -> CheckError {
    match error {
        SafeReadError::UnsupportedPlatform => CheckError::new(ErrorCode::UnsupportedPlatform),
        SafeReadError::Io | SafeReadError::NotRegular | SafeReadError::TooLarge => {
            CheckError::new(ErrorCode::RawStore)
        }
    }
}

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

struct SourceInput {
    entry: ManifestEntry,
    stored: Vec<StoredFile>,
    batch_json_hash: ContentHash,
    manifest_line_hash: ContentHash,
}

/// Read one approved source through the strict committed-manifest path and
/// independently authenticate its metadata bytes.  The same routine is used
/// for the KIS daily-range price scope and the KIS action scope, so neither
/// source can accidentally skip a metadata or newline commitment check.
fn read_source(store: &RawStore, source: SourceSpec) -> Result<SourceInput, CheckError> {
    // This is the strict reader: it requires the existing commit lock and
    // manifest and never discovers/reconciles orphan batches.
    let entries = store
        .read_committed_manifest(source.provider, MARKET_KR)
        .map_err(|_| CheckError::new(ErrorCode::RawStore))?;
    let target_id = source
        .batch_id
        .parse::<BatchId>()
        .map_err(|_| CheckError::new(ErrorCode::TargetBatchMissing))?;
    let matching = entries
        .iter()
        .filter(|entry| entry.batch_id == target_id)
        .collect::<Vec<_>>();
    let entry = match matching.as_slice() {
        [] => return Err(CheckError::new(ErrorCode::TargetBatchMissing)),
        [entry] => (*entry).clone(),
        _ => return Err(CheckError::new(ErrorCode::TargetBatchDuplicate)),
    };
    if entry.files.len() != source.file_count {
        return Err(CheckError::new(ErrorCode::SourceFileCount));
    }

    // RawStore performs its established immutable leaf hash checks.  The
    // returned bodies are handed to the corresponding V3 verifier, which
    // performs schema and action/price classification without exposing them.
    let stored = store
        .read_batch_bytes(source.provider, MARKET_KR, &entry)
        .map_err(|_| CheckError::new(ErrorCode::RawStore))?;

    let batch_json_path = store
        .batch_dir(source.provider, MARKET_KR, &entry.date, &entry.batch_id)
        .join(entry.batch_json_file_name());
    let batch_json = read_regular_file_no_follow(&batch_json_path, BATCH_JSON_MAX_BYTES)
        .map_err(map_batch_json_read_error)?;
    let batch_json_hash = parse_batch_json(&batch_json, &entry)?;
    let expected_batch_json_hash = ContentHash::parse(source.batch_json_sha256)
        .map_err(|_| CheckError::new(ErrorCode::BatchJsonMismatch))?;
    if batch_json_hash != expected_batch_json_hash {
        return Err(CheckError::new(ErrorCode::BatchJsonMismatch));
    }

    let manifest_path = store.manifest_path(source.provider, MARKET_KR);
    let manifest_bytes = read_regular_file_no_follow(&manifest_path, MANIFEST_MAX_BYTES)
        .map_err(map_manifest_read_error)?;
    let manifest_line = exact_manifest_line(&manifest_bytes, &entry)?;
    let manifest_line_hash = ContentHash::from_bytes(manifest_line);
    let expected_manifest_line_hash = ContentHash::parse(source.manifest_line_sha256)
        .map_err(|_| CheckError::new(ErrorCode::ManifestLineMismatch))?;
    if manifest_line_hash != expected_manifest_line_hash {
        return Err(CheckError::new(ErrorCode::ManifestLineMismatch));
    }

    Ok(SourceInput {
        entry,
        stored,
        batch_json_hash,
        manifest_line_hash,
    })
}

fn map_batch_json_read_error(error: SafeReadError) -> CheckError {
    match error {
        SafeReadError::UnsupportedPlatform => CheckError::new(ErrorCode::UnsupportedPlatform),
        SafeReadError::TooLarge | SafeReadError::Io | SafeReadError::NotRegular => {
            CheckError::new(ErrorCode::BatchJsonRead)
        }
    }
}

fn map_manifest_read_error(error: SafeReadError) -> CheckError {
    match error {
        SafeReadError::UnsupportedPlatform => CheckError::new(ErrorCode::UnsupportedPlatform),
        SafeReadError::TooLarge | SafeReadError::Io | SafeReadError::NotRegular => {
            CheckError::new(ErrorCode::ManifestRead)
        }
    }
}

/// Parse the separately opened metadata bytes and return the hash of those
/// exact bytes.  The `Value` comparison rejects otherwise-deserializable JSON
/// carrying unknown fields, so the verifier receives a commitment to the
/// complete `batch.json` document rather than merely to its known fields.
fn parse_batch_json(bytes: &[u8], entry: &ManifestEntry) -> Result<ContentHash, CheckError> {
    let parsed = serde_json::from_slice::<ManifestEntry>(bytes)
        .map_err(|_| CheckError::new(ErrorCode::BatchJsonMalformed))?;
    let parsed_value = serde_json::from_slice::<serde_json::Value>(bytes)
        .map_err(|_| CheckError::new(ErrorCode::BatchJsonMalformed))?;
    let expected_value =
        serde_json::to_value(entry).map_err(|_| CheckError::new(ErrorCode::BatchJsonMismatch))?;
    if parsed != *entry || parsed_value != expected_value {
        return Err(CheckError::new(ErrorCode::BatchJsonMismatch));
    }
    Ok(ContentHash::from_bytes(bytes))
}

/// Locate exactly one terminating-newline JSONL record whose deserialized
/// value equals `entry`.  Hashing the returned slice therefore commits the
/// newline as well as the JSON bytes.  A duplicate exact record is rejected;
/// a conflicting record is also rejected rather than silently selecting one.
fn exact_manifest_line<'a>(bytes: &'a [u8], entry: &ManifestEntry) -> Result<&'a [u8], CheckError> {
    let mut matching = None;
    let mut matching_count = 0usize;
    let mut same_batch_count = 0usize;
    let expected_value = serde_json::to_value(entry)
        .map_err(|_| CheckError::new(ErrorCode::ManifestLineMismatch))?;
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        let Some(payload) = line.strip_suffix(b"\n") else {
            continue;
        };
        if payload.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let parsed = serde_json::from_slice::<ManifestEntry>(payload)
            .map_err(|_| CheckError::new(ErrorCode::ManifestLineMalformed))?;
        if parsed.batch_id != entry.batch_id {
            continue;
        }
        same_batch_count += 1;
        let parsed_value = serde_json::from_slice::<serde_json::Value>(payload)
            .map_err(|_| CheckError::new(ErrorCode::ManifestLineMalformed))?;
        if parsed != *entry || parsed_value != expected_value {
            return Err(CheckError::new(ErrorCode::ManifestLineMismatch));
        }
        matching_count += 1;
        matching = Some(line);
    }
    if same_batch_count == 0 {
        return Err(CheckError::new(ErrorCode::ManifestLineMissing));
    }
    if matching_count != 1 {
        return Err(CheckError::new(ErrorCode::ManifestLineDuplicate));
    }
    matching.ok_or(CheckError::new(ErrorCode::ManifestLineMissing))
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SafeReadError {
    UnsupportedPlatform,
    Io,
    NotRegular,
    TooLarge,
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
        // ELOOP is the kernel's O_NOFOLLOW signal.  It has the same safe
        // outward result as all other read failures, but keeping this branch
        // explicit documents that symlink traversal is never accepted.
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

fn print_json<T: Serialize>(value: &T) {
    // These records contain only fixed safe fields.  Serialization of these
    // structs is infallible in practice; retain a typed fallback to avoid an
    // accidental diagnostic path if that ever changes.
    match serde_json::to_string(value) {
        Ok(line) => println!("{line}"),
        Err(_) => println!(r#"{{"status":"error","error_code":"serialization"}}"#),
    }
}

fn print_error(error: CheckError) {
    print_json(&ErrorRecord {
        status: "error",
        error_code: error.0.as_str(),
    });
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
            CheckError::new(ErrorCode::PriceInvalidPagination)
        );
        assert_eq!(
            map_action_verification_error(HistoricalPriceOnlyV3ActionError::InvalidPagination),
            CheckError::new(ErrorCode::ActionInvalidPagination)
        );
        assert_ne!(
            ErrorCode::PriceInvalidPagination.as_str(),
            ErrorCode::ActionInvalidPagination.as_str()
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
            PathBuf::from("/data/raw/manifests/provider=kis-daily-range/market=kr/manifest.jsonl")
        );
        assert_eq!(
            store.manifest_path(ACTION_SOURCE.provider, MARKET_KR),
            PathBuf::from("/data/raw/manifests/provider=kis/market=kr/manifest.jsonl")
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
    fn parser_requires_absolute_root_and_exactly_one_action() {
        let argv = vec![
            "--raw-root".to_owned(),
            "/data".to_owned(),
            "--check".to_owned(),
        ];
        let ParseOutcome::Args(args) = parse_args(&argv).unwrap() else {
            panic!("expected args")
        };
        assert_eq!(args.raw_root, PathBuf::from("/data"));
        assert_eq!(args.action, Action::Check);
        assert_eq!(
            parse_args(&["--raw-root".into(), "relative".into(), "--check".into()]),
            Err(CheckError::new(ErrorCode::RawRootMustBeAbsolute))
        );
        assert!(parse_args(&["--raw-root".into(), "/data".into()]).is_err());
        assert!(
            parse_args(&[
                "--raw-root".into(),
                "/data".into(),
                "--check".into(),
                "--plan".into()
            ])
            .is_err()
        );
    }

    #[test]
    fn manifest_line_hash_includes_terminating_newline() {
        let entry = fixture_entry();
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
        assert_eq!(
            exact_manifest_line(&line[..line.len() - 1], &entry),
            Err(CheckError::new(ErrorCode::ManifestLineMissing))
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
            Err(CheckError::new(ErrorCode::ManifestLineDuplicate))
        );
        assert_eq!(
            exact_manifest_line(b"{\"not\":\"a manifest\"}\n", &entry),
            Err(CheckError::new(ErrorCode::ManifestLineMalformed))
        );
        let mut conflicting = entry.clone();
        conflicting.market = "other".to_owned();
        let mut conflicting_bytes = compact_line(&conflicting);
        // Preserve the target batch id while changing a scope field.
        assert_eq!(
            exact_manifest_line(&conflicting_bytes, &entry),
            Err(CheckError::new(ErrorCode::ManifestLineMismatch))
        );
        conflicting_bytes.clear();
    }

    #[test]
    fn batch_json_is_hashed_as_read_and_must_deserialize_equal() {
        let entry = fixture_entry();
        let bytes = serde_json::to_vec_pretty(&entry).unwrap();
        let hash = parse_batch_json(&bytes, &entry).unwrap();
        assert_eq!(hash, ContentHash::from_bytes(&bytes),);
        let mut altered = entry;
        altered.market = "altered".to_owned();
        let altered_bytes = serde_json::to_vec(&altered).unwrap();
        assert_eq!(
            parse_batch_json(&altered_bytes, &fixture_entry()),
            Err(CheckError::new(ErrorCode::BatchJsonMismatch))
        );

        // Whitespace is part of the immutable metadata bytes.  It remains
        // semantically valid, but its actual hash must not be replaced with a
        // re-serialized/canonical JSON hash by the operator check.
        let mut with_extra_newline = bytes.clone();
        with_extra_newline.push(b'\n');
        let extra_hash = parse_batch_json(&with_extra_newline, &fixture_entry()).unwrap();
        assert_ne!(extra_hash, hash);
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
