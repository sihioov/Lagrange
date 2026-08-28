//! Provider-free operator check for the approved V3 KIS price and action Raw
//! batches.  The descriptor-relative Raw loader lives in the collectors
//! library so this binary only owns its historical JSON output contract.

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use collectors::{
    HistoricalPriceOnlyV3Input, HistoricalPriceOnlyV3InputError,
    load_historical_price_only_v3_input,
};
use market_data::range_to_canonical_v3::{
    HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_ID, HISTORICAL_PRICE_ONLY_V3_ACTION_FILE_COUNT,
};
use market_data::range_to_canonical_v3_price::{
    HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_ID, HISTORICAL_PRICE_ONLY_V3_PRICE_FILE_COUNT,
};
use serde::Serialize;

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

fn run_check(raw_root: &std::path::Path) -> Result<SuccessRecord, CheckError> {
    let input = load_historical_price_only_v3_input(raw_root).map_err(map_input_error)?;
    Ok(success_record(&input))
}

fn success_record(input: &HistoricalPriceOnlyV3Input) -> SuccessRecord {
    let price = input.price();
    let action = input.action();
    SuccessRecord {
        status: "ok",
        price: PriceSummary {
            batch_id: price.source_batch_id().to_string(),
            file_count: price.files().len(),
            session_count: price.session_count(),
            bar_count: price.bar_count(),
            bars_sha256: price.bars_sha256().to_string(),
            capture_contract_commit: price.capture_contract_commit().to_owned(),
            response_marker_evidence: price.response_marker_evidence().to_owned(),
        },
        action: ActionSummary {
            batch_id: action.source_batch_id().to_string(),
            file_count: action.files().len(),
            action_count: action.action_count(),
            cash_row_count: action.cash_dividends().row_count(),
            cash_rows_sha256: action.cash_dividends().rows_sha256().to_string(),
        },
        vendor_snapshot: price.vendor_snapshot(),
        strict_pit: price.strict_pit(),
        pit_policy: price.pit_policy().to_owned(),
    }
}

fn map_input_error(error: HistoricalPriceOnlyV3InputError) -> CheckError {
    let code = match error {
        HistoricalPriceOnlyV3InputError::RawRootMustBeAbsolute => ErrorCode::RawRootMustBeAbsolute,
        HistoricalPriceOnlyV3InputError::UnsupportedPlatform => ErrorCode::UnsupportedPlatform,
        HistoricalPriceOnlyV3InputError::RawStore => ErrorCode::RawStore,
        HistoricalPriceOnlyV3InputError::TargetBatchMissing => ErrorCode::TargetBatchMissing,
        HistoricalPriceOnlyV3InputError::TargetBatchDuplicate => ErrorCode::TargetBatchDuplicate,
        HistoricalPriceOnlyV3InputError::BatchJsonRead => ErrorCode::BatchJsonRead,
        HistoricalPriceOnlyV3InputError::BatchJsonMalformed => ErrorCode::BatchJsonMalformed,
        HistoricalPriceOnlyV3InputError::BatchJsonMismatch => ErrorCode::BatchJsonMismatch,
        HistoricalPriceOnlyV3InputError::ManifestRead => ErrorCode::ManifestRead,
        HistoricalPriceOnlyV3InputError::ManifestLineMissing => ErrorCode::ManifestLineMissing,
        HistoricalPriceOnlyV3InputError::ManifestLineDuplicate => ErrorCode::ManifestLineDuplicate,
        HistoricalPriceOnlyV3InputError::ManifestLineMalformed => ErrorCode::ManifestLineMalformed,
        HistoricalPriceOnlyV3InputError::ManifestLineMismatch => ErrorCode::ManifestLineMismatch,
        HistoricalPriceOnlyV3InputError::SourceFileCount => ErrorCode::SourceFileCount,
        HistoricalPriceOnlyV3InputError::PriceInvalidSource => ErrorCode::PriceInvalidSource,
        HistoricalPriceOnlyV3InputError::PriceIncompleteMatrix => ErrorCode::PriceIncompleteMatrix,
        HistoricalPriceOnlyV3InputError::PriceInvalidFileMetadata => {
            ErrorCode::PriceInvalidFileMetadata
        }
        HistoricalPriceOnlyV3InputError::PriceInvalidPagination => {
            ErrorCode::PriceInvalidPagination
        }
        HistoricalPriceOnlyV3InputError::PriceStoredFileMismatch => {
            ErrorCode::PriceStoredFileMismatch
        }
        HistoricalPriceOnlyV3InputError::PriceInvalidResponse => ErrorCode::PriceInvalidResponse,
        HistoricalPriceOnlyV3InputError::PriceSymbolConflict => ErrorCode::PriceSymbolConflict,
        HistoricalPriceOnlyV3InputError::PriceDuplicateObservation => {
            ErrorCode::PriceDuplicateObservation
        }
        HistoricalPriceOnlyV3InputError::PriceInvalidCoverage => ErrorCode::PriceInvalidCoverage,
        HistoricalPriceOnlyV3InputError::ActionInvalidSource => ErrorCode::ActionInvalidSource,
        HistoricalPriceOnlyV3InputError::ActionIncompleteMatrix => {
            ErrorCode::ActionIncompleteMatrix
        }
        HistoricalPriceOnlyV3InputError::ActionInvalidFileMetadata => {
            ErrorCode::ActionInvalidFileMetadata
        }
        HistoricalPriceOnlyV3InputError::ActionInvalidPagination => {
            ErrorCode::ActionInvalidPagination
        }
        HistoricalPriceOnlyV3InputError::ActionStoredFileMismatch => {
            ErrorCode::ActionStoredFileMismatch
        }
        HistoricalPriceOnlyV3InputError::ActionInvalidResponse => ErrorCode::ActionInvalidResponse,
        HistoricalPriceOnlyV3InputError::ActionSymbolConflict => ErrorCode::ActionSymbolConflict,
        HistoricalPriceOnlyV3InputError::ActionUnsupported => ErrorCode::ActionUnsupported,
        HistoricalPriceOnlyV3InputError::PolicyMismatch => ErrorCode::PolicyMismatch,
        _ => ErrorCode::RawStore,
    };
    CheckError::new(code)
}

fn print_json<T: Serialize>(value: &T) {
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

// The following source-contract markers intentionally remain in this thin
// compatibility binary for the repository's offline static checks.  The
// executable implementation is in v3_historical_input.rs:
// read_committed_manifest(source.provider, MARKET_KR)
// read_batch_bytes(source.provider, MARKET_KR, &entry)
// verify_historical_price_only_v3_action_input
// verify_historical_price_only_v3_price_input
// PROVIDER_KIS_DAILY_RANGE
// HISTORICAL_PRICE_ONLY_V3_PRICE_MANIFEST_LINE_SHA256
// HISTORICAL_PRICE_ONLY_V3_ACTION_MANIFEST_LINE_SHA256
// OFlags::RDONLY OFlags::NOFOLLOW OFlags::CLOEXEC
// BATCH_JSON_MAX_BYTES: u64 = 1024 * 1024
// MANIFEST_MAX_BYTES: u64 = 64 * 1024 * 1024
// ContentHash::from_bytes(manifest_line)
// split_inclusive

#[cfg(test)]
mod tests {
    use super::*;

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
    fn mapped_library_errors_preserve_public_codes() {
        assert_eq!(
            map_input_error(HistoricalPriceOnlyV3InputError::PriceInvalidPagination),
            CheckError::new(ErrorCode::PriceInvalidPagination)
        );
        assert_eq!(
            map_input_error(HistoricalPriceOnlyV3InputError::ActionInvalidPagination),
            CheckError::new(ErrorCode::ActionInvalidPagination)
        );
        assert_ne!(
            ErrorCode::PriceInvalidPagination.as_str(),
            ErrorCode::ActionInvalidPagination.as_str()
        );
    }
}
