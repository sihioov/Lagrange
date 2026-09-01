//! Restricted materialize/check boundary for the approved V3 historical
//! price-only artifact.
//!
//! `plan` is provider-free and does not inspect either root.  `materialize`
//! reads only the verified V3 Raw input through the collectors library and
//! delegates descriptor-safe publication to market-data.  `check` reads only
//! the requested artifact and never opens Raw.

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use collectors::{HistoricalPriceOnlyV3InputError, load_historical_price_only_v3_input};
use domain::ContentHash;
use market_data::range_to_canonical_v3::{
    HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_ID, HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_JSON_SHA256,
    HISTORICAL_PRICE_ONLY_V3_ACTION_MANIFEST_LINE_SHA256,
};
use market_data::range_to_canonical_v3_price::{
    HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_ID, HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_JSON_SHA256,
    HISTORICAL_PRICE_ONLY_V3_PRICE_CAPTURE_COMMIT,
    HISTORICAL_PRICE_ONLY_V3_PRICE_MANIFEST_LINE_SHA256,
    HISTORICAL_PRICE_ONLY_V3_PRICE_RESPONSE_MARKER_EVIDENCE,
};
use market_data::{
    HISTORICAL_PRICE_ONLY_V3_ACTION_FILES, HISTORICAL_PRICE_ONLY_V3_ARTIFACT_SCHEMA_VERSION,
    HISTORICAL_PRICE_ONLY_V3_BAR_COUNT, HISTORICAL_PRICE_ONLY_V3_CASH_ROW_COUNT,
    HISTORICAL_PRICE_ONLY_V3_CASH_ROWS_SHA256, HISTORICAL_PRICE_ONLY_V3_CASH_TREATMENT,
    HISTORICAL_PRICE_ONLY_V3_CONTRACT, HISTORICAL_PRICE_ONLY_V3_INSTRUMENT_COUNT,
    HISTORICAL_PRICE_ONLY_V3_MATERIALIZER_VERSION, HISTORICAL_PRICE_ONLY_V3_PIT_POLICY,
    HISTORICAL_PRICE_ONLY_V3_PRICE_FILES, HISTORICAL_PRICE_ONLY_V3_RANGE_END,
    HISTORICAL_PRICE_ONLY_V3_RANGE_START, HISTORICAL_PRICE_ONLY_V3_SCHEMA_ID,
    HISTORICAL_PRICE_ONLY_V3_SESSION_COUNT, HistoricalPriceOnlyV3ArtifactApprovalSummary,
    HistoricalPriceOnlyV3ArtifactError, HistoricalPriceOnlyV3Error,
    materialize_historical_price_only_v3, read_historical_price_only_v3_artifact,
    write_historical_price_only_v3_artifact,
};
use serde::Serialize;

const USAGE: &str = "kis-historical-price-v3-artifact --raw-root <ABS_DATA_ROOT> --artifact-root <ABS_ARTIFACT_ROOT> (--plan | --materialize | --check) [--candidate-content-sha256 <sha256:64hex>]";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Plan,
    Materialize,
    Check,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Args {
    raw_root: PathBuf,
    artifact_root: PathBuf,
    mode: Mode,
    candidate_content_sha256: Option<ContentHash>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ErrorCode {
    Usage,
    RawRootMustBeAbsolute,
    ArtifactRootMustBeAbsolute,
    CandidateContentSha256Required,
    CandidateContentSha256Unexpected,
    InvalidCandidateContentSha256,
    ArtifactRootNotSeparate,
    RootError,
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
    MaterializationInputMismatch,
    MaterializationPolicyMismatch,
    MaterializationDuplicateBar,
    MaterializationOhlcInvariant,
    MaterializationSerialization,
    ArtifactUnsafePath,
    ArtifactInvalidCandidate,
    ArtifactInvalidArtifact,
    ArtifactIo,
    ArtifactConflict,
    ArtifactNoreplaceUnavailable,
    ArtifactIndeterminateCommit,
    ArtifactCleanupFailed,
    ArtifactStagingNameExhausted,
    ArtifactError,
}

impl ErrorCode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Usage => "usage",
            Self::RawRootMustBeAbsolute => "raw_root_must_be_absolute",
            Self::ArtifactRootMustBeAbsolute => "artifact_root_must_be_absolute",
            Self::CandidateContentSha256Required => "candidate_content_sha256_required",
            Self::CandidateContentSha256Unexpected => "candidate_content_sha256_unexpected",
            Self::InvalidCandidateContentSha256 => "invalid_candidate_content_sha256",
            Self::ArtifactRootNotSeparate => "artifact_root_not_separate",
            Self::RootError => "root_error",
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
            Self::MaterializationInputMismatch => "materialization_input_mismatch",
            Self::MaterializationPolicyMismatch => "materialization_policy_mismatch",
            Self::MaterializationDuplicateBar => "materialization_duplicate_bar",
            Self::MaterializationOhlcInvariant => "materialization_ohlc_invariant",
            Self::MaterializationSerialization => "materialization_serialization",
            Self::ArtifactUnsafePath => "artifact_unsafe_path",
            Self::ArtifactInvalidCandidate => "artifact_invalid_candidate",
            Self::ArtifactInvalidArtifact => "artifact_invalid_artifact",
            Self::ArtifactIo => "artifact_io",
            Self::ArtifactConflict => "artifact_conflict",
            Self::ArtifactNoreplaceUnavailable => "artifact_noreplace_unavailable",
            Self::ArtifactIndeterminateCommit => "artifact_indeterminate_commit",
            Self::ArtifactCleanupFailed => "artifact_cleanup_failed",
            Self::ArtifactStagingNameExhausted => "artifact_staging_name_exhausted",
            Self::ArtifactError => "artifact_error",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Failure {
    mode: Mode,
    code: ErrorCode,
}

impl Failure {
    const fn new(mode: Mode, code: ErrorCode) -> Self {
        Self { mode, code }
    }
}

#[derive(Debug, Serialize)]
struct ErrorRecord {
    status: &'static str,
    operation: &'static str,
    error_code: &'static str,
}

#[derive(Debug, Serialize)]
struct PlanRecord {
    status: &'static str,
    operation: &'static str,
    network: &'static str,
    raw_read: &'static str,
    raw_write: &'static str,
    artifact_write: &'static str,
    schema_id: &'static str,
    schema_version: u32,
    contract: &'static str,
    materializer_version: &'static str,
    range_start: &'static str,
    range_end: &'static str,
    instrument_count: usize,
    session_count: usize,
    bar_count: usize,
    price_batch_id: &'static str,
    price_batch_json_sha256: &'static str,
    price_manifest_line_sha256: &'static str,
    price_capture_contract_commit: &'static str,
    price_response_marker_evidence: &'static str,
    action_batch_id: &'static str,
    action_batch_json_sha256: &'static str,
    action_manifest_line_sha256: &'static str,
    price_file_count: usize,
    action_file_count: usize,
    action_count: usize,
    cash_dividend_treatment: &'static str,
    cash_dividend_row_count: usize,
    cash_dividend_rows_sha256: &'static str,
    vendor_snapshot: bool,
    strict_pit: bool,
    pit_policy: &'static str,
}

#[derive(Debug, Serialize)]
struct ArtifactRecord {
    status: &'static str,
    operation: &'static str,
    raw_authenticity: &'static str,
    candidate_content_sha256: String,
    artifact_manifest_sha256: String,
    schema_id: String,
    schema_version: u32,
    contract: String,
    materializer_version: String,
    audience: String,
    vendor_snapshot: bool,
    strict_pit: bool,
    pit_policy: &'static str,
    capability: String,
    materialization_status: String,
    registration_status: String,
    publication_status: String,
    range_start: String,
    range_end: String,
    instruments: Vec<String>,
    instrument_count: usize,
    session_count: usize,
    bar_count: usize,
    row_count: usize,
    price_batch_id: String,
    price_batch_json_sha256: String,
    price_manifest_line_sha256: String,
    price_bars_sha256: String,
    price_file_count: usize,
    price_capture_contract_commit: String,
    price_response_marker_evidence: String,
    price_acquired_at: String,
    action_batch_id: String,
    action_batch_json_sha256: String,
    action_manifest_line_sha256: String,
    action_file_count: usize,
    action_count: usize,
    cash_dividend_treatment: String,
    cash_dividend_row_count: usize,
    cash_dividend_rows_sha256: String,
    action_acquired_at: String,
    price: PriceRecord,
    action: ActionRecord,
}

#[derive(Debug, Serialize)]
struct PriceRecord {
    batch_id: String,
    batch_json_sha256: String,
    manifest_line_sha256: String,
    bars_sha256: String,
    file_count: usize,
    capture_contract_commit: String,
    response_marker_evidence: String,
    acquired_at: String,
}

#[derive(Debug, Serialize)]
struct ActionRecord {
    batch_id: String,
    batch_json_sha256: String,
    manifest_line_sha256: String,
    file_count: usize,
    action_count: usize,
    cash_dividend_treatment: String,
    cash_dividend_row_count: usize,
    cash_dividend_rows_sha256: String,
    acquired_at: String,
}

fn main() -> ExitCode {
    let argv = env::args().skip(1).collect::<Vec<_>>();
    match parse_args(&argv) {
        Ok(ParseOutcome::Help) => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Ok(ParseOutcome::Args(args)) => match execute(args) {
            Ok(Output::Plan(record)) => {
                print_json(&record);
                ExitCode::SUCCESS
            }
            Ok(Output::Artifact(record)) => {
                print_json(&record);
                ExitCode::SUCCESS
            }
            Err(failure) => {
                print_error(failure);
                ExitCode::FAILURE
            }
        },
        Err(failure) => {
            print_error(failure);
            ExitCode::FAILURE
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ParseOutcome {
    Help,
    Args(Args),
}

fn parse_args(argv: &[String]) -> Result<ParseOutcome, Failure> {
    if argv.len() == 1 && matches!(argv[0].as_str(), "--help" | "-h") {
        return Ok(ParseOutcome::Help);
    }

    let mut raw_root = None;
    let mut artifact_root = None;
    let mut mode = None;
    let mut candidate_content_sha256 = None;
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--raw-root" => {
                if raw_root.is_some() {
                    return Err(Failure::new(mode.unwrap_or(Mode::Plan), ErrorCode::Usage));
                }
                let value = argv
                    .get(index + 1)
                    .ok_or(Failure::new(mode.unwrap_or(Mode::Plan), ErrorCode::Usage))?;
                if value.starts_with("--") {
                    return Err(Failure::new(mode.unwrap_or(Mode::Plan), ErrorCode::Usage));
                }
                raw_root = Some(PathBuf::from(value));
                index += 2;
            }
            "--artifact-root" => {
                if artifact_root.is_some() {
                    return Err(Failure::new(mode.unwrap_or(Mode::Plan), ErrorCode::Usage));
                }
                let value = argv
                    .get(index + 1)
                    .ok_or(Failure::new(mode.unwrap_or(Mode::Plan), ErrorCode::Usage))?;
                if value.starts_with("--") {
                    return Err(Failure::new(mode.unwrap_or(Mode::Plan), ErrorCode::Usage));
                }
                artifact_root = Some(PathBuf::from(value));
                index += 2;
            }
            "--plan" => {
                if mode.replace(Mode::Plan).is_some() {
                    return Err(Failure::new(Mode::Plan, ErrorCode::Usage));
                }
                index += 1;
            }
            "--materialize" => {
                if mode.replace(Mode::Materialize).is_some() {
                    return Err(Failure::new(Mode::Materialize, ErrorCode::Usage));
                }
                index += 1;
            }
            "--check" => {
                if mode.replace(Mode::Check).is_some() {
                    return Err(Failure::new(Mode::Check, ErrorCode::Usage));
                }
                index += 1;
            }
            "--candidate-content-sha256" => {
                if candidate_content_sha256.is_some() {
                    return Err(Failure::new(mode.unwrap_or(Mode::Plan), ErrorCode::Usage));
                }
                let operation = mode.unwrap_or(Mode::Check);
                let value = argv
                    .get(index + 1)
                    .ok_or(Failure::new(operation, ErrorCode::Usage))?;
                if value.starts_with("--") {
                    return Err(Failure::new(operation, ErrorCode::Usage));
                }
                candidate_content_sha256 = Some(value.clone());
                index += 2;
            }
            _ => return Err(Failure::new(mode.unwrap_or(Mode::Plan), ErrorCode::Usage)),
        }
    }

    let mode = mode.ok_or(Failure::new(Mode::Plan, ErrorCode::Usage))?;
    let raw_root = raw_root.ok_or(Failure::new(mode, ErrorCode::Usage))?;
    if !raw_root.is_absolute() {
        return Err(Failure::new(mode, ErrorCode::RawRootMustBeAbsolute));
    }
    let artifact_root = artifact_root.ok_or(Failure::new(mode, ErrorCode::Usage))?;
    if !artifact_root.is_absolute() {
        return Err(Failure::new(mode, ErrorCode::ArtifactRootMustBeAbsolute));
    }
    match (mode, candidate_content_sha256) {
        (Mode::Check, None) => Err(Failure::new(
            Mode::Check,
            ErrorCode::CandidateContentSha256Required,
        )),
        (Mode::Check, Some(candidate_content_sha256)) => {
            let candidate_content_sha256 = ContentHash::parse(&candidate_content_sha256)
                .map_err(|_| Failure::new(Mode::Check, ErrorCode::InvalidCandidateContentSha256))?;
            Ok(ParseOutcome::Args(Args {
                raw_root,
                artifact_root,
                mode,
                candidate_content_sha256: Some(candidate_content_sha256),
            }))
        }
        (_, Some(_)) => Err(Failure::new(
            mode,
            ErrorCode::CandidateContentSha256Unexpected,
        )),
        (_, None) => Ok(ParseOutcome::Args(Args {
            raw_root,
            artifact_root,
            mode,
            candidate_content_sha256: None,
        })),
    }
}

enum Output {
    Plan(PlanRecord),
    Artifact(ArtifactRecord),
}

fn execute(args: Args) -> Result<Output, Failure> {
    match args.mode {
        Mode::Plan => Ok(Output::Plan(plan_record())),
        Mode::Materialize => execute_materialize(args).map(Output::Artifact),
        Mode::Check => execute_check(args).map(Output::Artifact),
    }
}

fn plan_record() -> PlanRecord {
    PlanRecord {
        status: "plan",
        operation: "plan",
        network: "none",
        raw_read: "none",
        raw_write: "none",
        artifact_write: "none",
        schema_id: HISTORICAL_PRICE_ONLY_V3_SCHEMA_ID,
        schema_version: HISTORICAL_PRICE_ONLY_V3_ARTIFACT_SCHEMA_VERSION,
        contract: HISTORICAL_PRICE_ONLY_V3_CONTRACT,
        materializer_version: HISTORICAL_PRICE_ONLY_V3_MATERIALIZER_VERSION,
        range_start: HISTORICAL_PRICE_ONLY_V3_RANGE_START,
        range_end: HISTORICAL_PRICE_ONLY_V3_RANGE_END,
        instrument_count: HISTORICAL_PRICE_ONLY_V3_INSTRUMENT_COUNT,
        session_count: HISTORICAL_PRICE_ONLY_V3_SESSION_COUNT,
        bar_count: HISTORICAL_PRICE_ONLY_V3_BAR_COUNT,
        price_batch_id: HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_ID,
        price_batch_json_sha256: HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_JSON_SHA256,
        price_manifest_line_sha256: HISTORICAL_PRICE_ONLY_V3_PRICE_MANIFEST_LINE_SHA256,
        price_capture_contract_commit: HISTORICAL_PRICE_ONLY_V3_PRICE_CAPTURE_COMMIT,
        price_response_marker_evidence: HISTORICAL_PRICE_ONLY_V3_PRICE_RESPONSE_MARKER_EVIDENCE,
        action_batch_id: HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_ID,
        action_batch_json_sha256: HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_JSON_SHA256,
        action_manifest_line_sha256: HISTORICAL_PRICE_ONLY_V3_ACTION_MANIFEST_LINE_SHA256,
        price_file_count: HISTORICAL_PRICE_ONLY_V3_PRICE_FILES,
        action_file_count: HISTORICAL_PRICE_ONLY_V3_ACTION_FILES,
        action_count: 0,
        cash_dividend_treatment: HISTORICAL_PRICE_ONLY_V3_CASH_TREATMENT,
        cash_dividend_row_count: HISTORICAL_PRICE_ONLY_V3_CASH_ROW_COUNT,
        cash_dividend_rows_sha256: HISTORICAL_PRICE_ONLY_V3_CASH_ROWS_SHA256,
        vendor_snapshot: true,
        strict_pit: false,
        pit_policy: HISTORICAL_PRICE_ONLY_V3_PIT_POLICY,
    }
}

fn execute_materialize(args: Args) -> Result<ArtifactRecord, Failure> {
    filesystem_separation_gate(&args.raw_root, &args.artifact_root)
        .map_err(|code| Failure::new(Mode::Materialize, code))?;
    let input = load_historical_price_only_v3_input(&args.raw_root)
        .map_err(|error| Failure::new(Mode::Materialize, map_input_error(error)))?;
    let candidate = materialize_historical_price_only_v3(input.price(), input.action())
        .map_err(|error| Failure::new(Mode::Materialize, map_materialization_error(error)))?;
    let verified = write_historical_price_only_v3_artifact(&args.artifact_root, &candidate)
        .map_err(|error| Failure::new(Mode::Materialize, map_artifact_error(error)))?;
    Ok(artifact_record(
        "materialize",
        "PINNED_RAW_VERIFIED_IN_PROCESS",
        verified.candidate_content_sha256(),
        verified.approval_summary(),
    ))
}

fn execute_check(args: Args) -> Result<ArtifactRecord, Failure> {
    let Some(candidate_hash) = args.candidate_content_sha256 else {
        return Err(Failure::new(
            Mode::Check,
            ErrorCode::CandidateContentSha256Required,
        ));
    };
    let verified = read_historical_price_only_v3_artifact(&args.artifact_root, &candidate_hash)
        .map_err(|error| Failure::new(Mode::Check, map_artifact_error(error)))?;
    Ok(artifact_record(
        "check",
        "NOT_REAUTHENTICATED",
        verified.candidate_content_sha256(),
        verified.approval_summary(),
    ))
}

fn artifact_record(
    operation: &'static str,
    raw_authenticity: &'static str,
    candidate_hash: &ContentHash,
    summary: &HistoricalPriceOnlyV3ArtifactApprovalSummary,
) -> ArtifactRecord {
    ArtifactRecord {
        status: "ok",
        operation,
        raw_authenticity,
        candidate_content_sha256: candidate_hash.as_str().to_owned(),
        artifact_manifest_sha256: summary.artifact_manifest_sha256().as_str().to_owned(),
        schema_id: summary.schema_id().to_owned(),
        schema_version: summary.schema_version(),
        contract: summary.contract().to_owned(),
        materializer_version: summary.materializer_version().to_owned(),
        audience: summary.audience().to_owned(),
        vendor_snapshot: summary.vendor_snapshot(),
        strict_pit: summary.strict_pit(),
        pit_policy: HISTORICAL_PRICE_ONLY_V3_PIT_POLICY,
        capability: summary.capability().to_owned(),
        materialization_status: summary.materialization_status().to_owned(),
        registration_status: summary.registration_status().to_owned(),
        publication_status: summary.publication_status().to_owned(),
        range_start: summary.range_start().to_string(),
        range_end: summary.range_end().to_string(),
        instruments: summary.instruments().to_vec(),
        instrument_count: summary.instrument_count(),
        session_count: summary.session_count(),
        bar_count: summary.row_count(),
        row_count: summary.row_count(),
        price_batch_id: summary.price_batch_id().to_string(),
        price_batch_json_sha256: summary.price_batch_json_sha256().as_str().to_owned(),
        price_manifest_line_sha256: summary.price_manifest_line_sha256().as_str().to_owned(),
        price_bars_sha256: summary.price_bars_sha256().as_str().to_owned(),
        price_file_count: summary.price_file_count(),
        price_capture_contract_commit: summary.price_capture_contract_commit().to_owned(),
        price_response_marker_evidence: summary.price_response_marker_evidence().to_owned(),
        price_acquired_at: summary.price_acquired_at().to_string(),
        action_batch_id: summary.action_batch_id().to_string(),
        action_batch_json_sha256: summary.action_batch_json_sha256().as_str().to_owned(),
        action_manifest_line_sha256: summary.action_manifest_line_sha256().as_str().to_owned(),
        action_file_count: summary.action_file_count(),
        action_count: summary.action_count(),
        cash_dividend_treatment: summary.cash_dividend_treatment_id().to_owned(),
        cash_dividend_row_count: summary.cash_dividend_row_count(),
        cash_dividend_rows_sha256: summary.cash_dividend_rows_sha256().as_str().to_owned(),
        action_acquired_at: summary.action_acquired_at().to_string(),
        price: PriceRecord {
            batch_id: summary.price_batch_id().to_string(),
            batch_json_sha256: summary.price_batch_json_sha256().as_str().to_owned(),
            manifest_line_sha256: summary.price_manifest_line_sha256().as_str().to_owned(),
            bars_sha256: summary.price_bars_sha256().as_str().to_owned(),
            file_count: summary.price_file_count(),
            capture_contract_commit: summary.price_capture_contract_commit().to_owned(),
            response_marker_evidence: summary.price_response_marker_evidence().to_owned(),
            acquired_at: summary.price_acquired_at().to_string(),
        },
        action: ActionRecord {
            batch_id: summary.action_batch_id().to_string(),
            batch_json_sha256: summary.action_batch_json_sha256().as_str().to_owned(),
            manifest_line_sha256: summary.action_manifest_line_sha256().as_str().to_owned(),
            file_count: summary.action_file_count(),
            action_count: summary.action_count(),
            cash_dividend_treatment: summary.cash_dividend_treatment_id().to_owned(),
            cash_dividend_row_count: summary.cash_dividend_row_count(),
            cash_dividend_rows_sha256: summary.cash_dividend_rows_sha256().as_str().to_owned(),
            acquired_at: summary.action_acquired_at().to_string(),
        },
    }
}

fn map_input_error(error: HistoricalPriceOnlyV3InputError) -> ErrorCode {
    match error {
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
    }
}

fn map_materialization_error(error: HistoricalPriceOnlyV3Error) -> ErrorCode {
    match error {
        HistoricalPriceOnlyV3Error::InputMismatch { .. } => ErrorCode::MaterializationInputMismatch,
        HistoricalPriceOnlyV3Error::PolicyMismatch => ErrorCode::MaterializationPolicyMismatch,
        HistoricalPriceOnlyV3Error::DuplicateBar { .. } => ErrorCode::MaterializationDuplicateBar,
        HistoricalPriceOnlyV3Error::OhlcInvariant { .. } => ErrorCode::MaterializationOhlcInvariant,
        HistoricalPriceOnlyV3Error::Serialization => ErrorCode::MaterializationSerialization,
    }
}

fn map_artifact_error(error: HistoricalPriceOnlyV3ArtifactError) -> ErrorCode {
    match error {
        HistoricalPriceOnlyV3ArtifactError::UnsupportedPlatform => ErrorCode::UnsupportedPlatform,
        HistoricalPriceOnlyV3ArtifactError::UnsafePath => ErrorCode::ArtifactUnsafePath,
        HistoricalPriceOnlyV3ArtifactError::InvalidCandidate => ErrorCode::ArtifactInvalidCandidate,
        HistoricalPriceOnlyV3ArtifactError::InvalidArtifact => ErrorCode::ArtifactInvalidArtifact,
        HistoricalPriceOnlyV3ArtifactError::Io(_) => ErrorCode::ArtifactIo,
        HistoricalPriceOnlyV3ArtifactError::Conflict { .. } => ErrorCode::ArtifactConflict,
        HistoricalPriceOnlyV3ArtifactError::UnsupportedAtomicNoReplace => {
            ErrorCode::ArtifactNoreplaceUnavailable
        }
        HistoricalPriceOnlyV3ArtifactError::IndeterminateCommit => {
            ErrorCode::ArtifactIndeterminateCommit
        }
        HistoricalPriceOnlyV3ArtifactError::CleanupFailed => ErrorCode::ArtifactCleanupFailed,
        HistoricalPriceOnlyV3ArtifactError::StagingNameExhausted => {
            ErrorCode::ArtifactStagingNameExhausted
        }
        _ => ErrorCode::ArtifactError,
    }
}

#[cfg(not(unix))]
fn filesystem_separation_gate(_raw_root: &Path, _artifact_root: &Path) -> Result<(), ErrorCode> {
    Err(ErrorCode::UnsupportedPlatform)
}

#[cfg(unix)]
struct ResolvedDirectory {
    path: PathBuf,
    dev: u64,
    ino: u64,
}

#[cfg(unix)]
fn filesystem_separation_gate(raw_root: &Path, artifact_root: &Path) -> Result<(), ErrorCode> {
    let raw = resolve_directory(raw_root)?;
    let artifact = resolve_directory(artifact_root)?;
    if artifact.path == raw.path || raw.path.starts_with(&artifact.path) {
        return Err(ErrorCode::ArtifactRootNotSeparate);
    }

    let raw_boundary = resolve_optional_directory(&raw.path.join("raw"))?;
    let curated_boundary = resolve_optional_directory(&raw.path.join("curated"))?;
    let boundaries = [raw_boundary, curated_boundary];
    for boundary in boundaries.iter().flatten() {
        if artifact.path == boundary.path || artifact.path.starts_with(&boundary.path) {
            return Err(ErrorCode::ArtifactRootNotSeparate);
        }
    }
    if (artifact.dev, artifact.ino) == (raw.dev, raw.ino) {
        return Err(ErrorCode::ArtifactRootNotSeparate);
    }
    let forbidden_identities = boundaries
        .iter()
        .flatten()
        .map(|boundary| (boundary.dev, boundary.ino))
        .collect::<Vec<_>>();
    for ancestor in artifact.path.ancestors() {
        let metadata = std::fs::metadata(ancestor).map_err(|_| ErrorCode::RootError)?;
        let identity = (
            std::os::unix::fs::MetadataExt::dev(&metadata),
            std::os::unix::fs::MetadataExt::ino(&metadata),
        );
        if forbidden_identities.contains(&identity) {
            return Err(ErrorCode::ArtifactRootNotSeparate);
        }
    }
    Ok(())
}

#[cfg(unix)]
fn resolve_directory(path: &Path) -> Result<ResolvedDirectory, ErrorCode> {
    let resolved = std::fs::canonicalize(path).map_err(|_| ErrorCode::RootError)?;
    let metadata = std::fs::metadata(&resolved).map_err(|_| ErrorCode::RootError)?;
    if !metadata.is_dir() {
        return Err(ErrorCode::RootError);
    }
    Ok(ResolvedDirectory {
        path: resolved,
        dev: std::os::unix::fs::MetadataExt::dev(&metadata),
        ino: std::os::unix::fs::MetadataExt::ino(&metadata),
    })
}

#[cfg(unix)]
fn resolve_optional_directory(path: &Path) -> Result<Option<ResolvedDirectory>, ErrorCode> {
    let lexical_metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(ErrorCode::RootError),
    };
    let resolved = std::fs::canonicalize(path).map_err(|_| ErrorCode::RootError)?;
    let metadata = std::fs::metadata(&resolved).map_err(|_| ErrorCode::RootError)?;
    if !metadata.is_dir() || lexical_metadata.file_type().is_file() {
        return Err(ErrorCode::RootError);
    }
    Ok(Some(ResolvedDirectory {
        path: resolved,
        dev: std::os::unix::fs::MetadataExt::dev(&metadata),
        ino: std::os::unix::fs::MetadataExt::ino(&metadata),
    }))
}

fn print_json<T: Serialize>(value: &T) {
    match serde_json::to_string(value) {
        Ok(line) => println!("{line}"),
        Err(_) => {
            println!(r#"{{"status":"error","operation":"unknown","error_code":"serialization"}}"#)
        }
    }
}

fn print_error(failure: Failure) {
    let operation = match failure.mode {
        Mode::Plan => "plan",
        Mode::Materialize => "materialize",
        Mode::Check => "check",
    };
    print_json(&ErrorRecord {
        status: "error",
        operation,
        error_code: failure.code.as_str(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(byte: char) -> String {
        format!("sha256:{}", byte.to_string().repeat(64))
    }

    fn base_argv(mode: &str) -> Vec<String> {
        vec![
            "--raw-root".into(),
            "/data".into(),
            "--artifact-root".into(),
            "/artifacts".into(),
            mode.into(),
        ]
    }

    #[test]
    fn grammar_is_exact_and_candidate_hash_is_check_only() {
        let parsed = parse_args(&base_argv("--materialize")).unwrap();
        let ParseOutcome::Args(args) = parsed else {
            panic!("expected args")
        };
        assert_eq!(args.mode, Mode::Materialize);
        assert_eq!(args.raw_root, PathBuf::from("/data"));
        assert_eq!(args.artifact_root, PathBuf::from("/artifacts"));

        let mut check = base_argv("--check");
        check.extend(["--candidate-content-sha256".into(), hash('c')]);
        let ParseOutcome::Args(args) = parse_args(&check).unwrap() else {
            panic!("expected check args")
        };
        assert_eq!(args.candidate_content_sha256.unwrap().as_str(), hash('c'));
        assert_eq!(
            parse_args(&base_argv("--check")),
            Err(Failure::new(
                Mode::Check,
                ErrorCode::CandidateContentSha256Required
            ))
        );
        let mut unexpected = base_argv("--plan");
        unexpected.extend(["--candidate-content-sha256".into(), hash('c')]);
        assert_eq!(
            parse_args(&unexpected),
            Err(Failure::new(
                Mode::Plan,
                ErrorCode::CandidateContentSha256Unexpected
            ))
        );
    }

    #[test]
    fn mode_exclusivity_and_absolute_roots_are_enforced() {
        let mut modes = base_argv("--plan");
        modes.push("--check".into());
        assert_eq!(
            parse_args(&modes),
            Err(Failure::new(Mode::Check, ErrorCode::Usage))
        );
        let mut raw = base_argv("--plan");
        raw[1] = "relative".into();
        assert_eq!(
            parse_args(&raw),
            Err(Failure::new(Mode::Plan, ErrorCode::RawRootMustBeAbsolute))
        );
        let mut artifact = base_argv("--plan");
        artifact[3] = "relative".into();
        assert_eq!(
            parse_args(&artifact),
            Err(Failure::new(
                Mode::Plan,
                ErrorCode::ArtifactRootMustBeAbsolute
            ))
        );
    }

    #[test]
    fn plan_does_not_read_missing_roots() {
        let args = match parse_args(&[
            "--raw-root".into(),
            "/path/that/does/not/exist".into(),
            "--artifact-root".into(),
            "/another/missing/path".into(),
            "--plan".into(),
        ])
        .unwrap()
        {
            ParseOutcome::Args(args) => args,
            ParseOutcome::Help => panic!("expected args"),
        };
        assert!(matches!(execute(args), Ok(Output::Plan(_))));
    }

    #[test]
    fn plan_record_is_safe_json_with_fixed_contract_facts() {
        let encoded = serde_json::to_string(&plan_record()).unwrap();
        assert!(encoded.contains(r#""instrument_count":11"#));
        assert!(encoded.contains(r#""session_count":2452"#));
        assert!(encoded.contains(r#""bar_count":26972"#));
        assert!(encoded.contains("CASH_ONLY_EXCLUDED_FROM_PRICE_RETURN_ONLY_V1"));
        assert!(!encoded.contains("/data"));
    }

    #[test]
    fn internal_failures_map_to_finite_safe_codes() {
        assert_eq!(
            map_materialization_error(HistoricalPriceOnlyV3Error::Serialization).as_str(),
            "materialization_serialization"
        );
        assert_eq!(
            map_artifact_error(HistoricalPriceOnlyV3ArtifactError::InvalidArtifact).as_str(),
            "artifact_invalid_artifact"
        );
        assert_eq!(
            map_input_error(HistoricalPriceOnlyV3InputError::ActionUnsupported).as_str(),
            "action_unsupported"
        );
    }
}
