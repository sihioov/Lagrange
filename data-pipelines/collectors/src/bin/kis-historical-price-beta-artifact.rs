//! Restricted owner-only materialize/check boundary for the historical price beta artifact.

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use domain::ContentHash;
use market_data::{
    HISTORICAL_PRICE_ONLY_BETA_SESSION_COUNT, HistoricalPriceOnlyArtifactError,
    HistoricalPriceOnlyError, KR_ETF_CORE_SYMBOLS, RangeCanonicalError, RawStore,
    materialize_historical_price_only_beta, read_historical_price_only_artifact,
    verify_historical_price_only_beta_input, write_historical_price_only_artifact,
};

const USAGE: &str = "kis-historical-price-beta-artifact materialize --raw-root <ABS_DATA_ROOT> --artifact-root <ABS_ARTIFACT_ROOT> --stage5-manifest-sha256 <sha256:64hex> --action-manifest-sha256 <sha256:64hex>\nkis-historical-price-beta-artifact check --artifact-root <ABS_ARTIFACT_ROOT> --candidate-content-sha256 <sha256:64hex>";

const INSTRUMENT_COUNT: usize = KR_ETF_CORE_SYMBOLS.len();
const SESSION_COUNT: usize = HISTORICAL_PRICE_ONLY_BETA_SESSION_COUNT;
const BAR_COUNT: usize = INSTRUMENT_COUNT * SESSION_COUNT;

const MATERIALIZE_OPTIONS: [&str; 4] = [
    "--raw-root",
    "--artifact-root",
    "--stage5-manifest-sha256",
    "--action-manifest-sha256",
];
const CHECK_OPTIONS: [&str; 2] = ["--artifact-root", "--candidate-content-sha256"];

struct MaterializeArgs {
    raw_root: PathBuf,
    artifact_root: PathBuf,
    stage5_manifest_sha256: ContentHash,
    action_manifest_sha256: ContentHash,
}

struct CheckArgs {
    artifact_root: PathBuf,
    candidate_content_sha256: ContentHash,
}

enum Command {
    Materialize(MaterializeArgs),
    Check(CheckArgs),
}

enum ParseOutcome {
    Help,
    Command(Command),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operation {
    Materialize,
    Check,
    Unknown,
}

impl Operation {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Materialize => "materialize",
            Self::Check => "check",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug)]
struct Failure {
    operation: Operation,
    reason: &'static str,
}

impl Failure {
    const fn new(operation: Operation, reason: &'static str) -> Self {
        Self { operation, reason }
    }
}

fn main() -> ExitCode {
    let argv = env::args().skip(1).collect::<Vec<_>>();
    match parse_args(&argv) {
        Ok(ParseOutcome::Help) => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Ok(ParseOutcome::Command(command)) => match execute(command) {
            Ok(line) => {
                println!("{line}");
                ExitCode::SUCCESS
            }
            Err(failure) => {
                eprintln!(
                    "HISTORICAL_PRICE_BETA_ARTIFACT status=blocked operation={} reason={}",
                    failure.operation.as_str(),
                    failure.reason
                );
                ExitCode::FAILURE
            }
        },
        Err(failure) => {
            eprintln!(
                "HISTORICAL_PRICE_BETA_ARTIFACT status=blocked operation={} reason={}",
                failure.operation.as_str(),
                failure.reason
            );
            ExitCode::FAILURE
        }
    }
}

fn parse_args(argv: &[String]) -> Result<ParseOutcome, Failure> {
    if argv.len() == 1 && argv[0] == "--help" {
        return Ok(ParseOutcome::Help);
    }
    let Some(command) = argv.first() else {
        return Err(Failure::new(Operation::Unknown, "missing_command"));
    };
    match command.as_str() {
        "materialize" => parse_materialize(&argv[1..])
            .map(|args| ParseOutcome::Command(Command::Materialize(args))),
        "check" => parse_check(&argv[1..]).map(|args| ParseOutcome::Command(Command::Check(args))),
        _ => Err(Failure::new(Operation::Unknown, "unknown_command")),
    }
}

fn parse_materialize(argv: &[String]) -> Result<MaterializeArgs, Failure> {
    let values = exact_option_values(
        argv,
        Operation::Materialize,
        &MATERIALIZE_OPTIONS,
        &CHECK_OPTIONS,
    )?;
    let raw_root = absolute_path(
        values[0],
        Operation::Materialize,
        "raw_root_must_be_absolute",
    )?;
    let artifact_root = absolute_path(
        values[1],
        Operation::Materialize,
        "artifact_root_must_be_absolute",
    )?;
    let stage5_manifest_sha256 = parse_hash(
        values[2],
        Operation::Materialize,
        "invalid_stage5_manifest_sha256",
    )?;
    let action_manifest_sha256 = parse_hash(
        values[3],
        Operation::Materialize,
        "invalid_action_manifest_sha256",
    )?;
    Ok(MaterializeArgs {
        raw_root,
        artifact_root,
        stage5_manifest_sha256,
        action_manifest_sha256,
    })
}

fn parse_check(argv: &[String]) -> Result<CheckArgs, Failure> {
    let values = exact_option_values(argv, Operation::Check, &CHECK_OPTIONS, &MATERIALIZE_OPTIONS)?;
    let artifact_root = absolute_path(
        values[0],
        Operation::Check,
        "artifact_root_must_be_absolute",
    )?;
    let candidate_content_sha256 = parse_hash(
        values[1],
        Operation::Check,
        "invalid_candidate_content_sha256",
    )?;
    Ok(CheckArgs {
        artifact_root,
        candidate_content_sha256,
    })
}

fn exact_option_values<'a>(
    argv: &'a [String],
    operation: Operation,
    expected: &[&str],
    other_command_options: &[&str],
) -> Result<Vec<&'a str>, Failure> {
    if argv.len() != expected.len() * 2 {
        let reason = if argv.last().is_some_and(|value| value.starts_with("--")) {
            "missing_option_value"
        } else if argv.len() < expected.len() * 2 {
            "missing_option"
        } else {
            "unknown_or_repeated_option"
        };
        return Err(Failure::new(operation, reason));
    }

    let mut values = Vec::with_capacity(expected.len());
    for (index, expected_option) in expected.iter().enumerate() {
        let actual = argv[index * 2].as_str();
        if actual != *expected_option {
            let reason = if other_command_options.contains(&actual) {
                "cross_command_option"
            } else {
                "unknown_or_repeated_option"
            };
            return Err(Failure::new(operation, reason));
        }
        values.push(argv[index * 2 + 1].as_str());
    }
    Ok(values)
}

fn absolute_path(
    value: &str,
    operation: Operation,
    reason: &'static str,
) -> Result<PathBuf, Failure> {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(Failure::new(operation, reason))
    }
}

fn parse_hash(
    value: &str,
    operation: Operation,
    reason: &'static str,
) -> Result<ContentHash, Failure> {
    ContentHash::parse(value).map_err(|_| Failure::new(operation, reason))
}

fn execute(command: Command) -> Result<String, Failure> {
    match command {
        Command::Materialize(args) => execute_materialize(args),
        Command::Check(args) => execute_check(args),
    }
}

fn execute_materialize(args: MaterializeArgs) -> Result<String, Failure> {
    filesystem_separation_gate(&args.raw_root, &args.artifact_root)
        .map_err(|reason| Failure::new(Operation::Materialize, reason))?;

    let raw = RawStore::new(args.raw_root);
    let input = verify_historical_price_only_beta_input(
        &raw,
        &args.stage5_manifest_sha256,
        &args.action_manifest_sha256,
    )
    .map_err(|error| Failure::new(Operation::Materialize, map_range_error(error)))?;
    let candidate = materialize_historical_price_only_beta(&input)
        .map_err(|error| Failure::new(Operation::Materialize, map_historical_error(error)))?;
    let verified = write_historical_price_only_artifact(&args.artifact_root, &candidate)
        .map_err(|error| Failure::new(Operation::Materialize, map_artifact_error(error)))?;

    Ok(materialize_success_line(
        verified.candidate_content_sha256(),
        verified.approval_summary().artifact_manifest_sha256(),
        &args.stage5_manifest_sha256,
        &args.action_manifest_sha256,
        verified
            .approval_summary()
            .ignored_cash_dividend_row_count(),
        verified
            .approval_summary()
            .ignored_cash_dividend_rows_sha256(),
    ))
}

fn execute_check(args: CheckArgs) -> Result<String, Failure> {
    let verified =
        read_historical_price_only_artifact(&args.artifact_root, &args.candidate_content_sha256)
            .map_err(|error| Failure::new(Operation::Check, map_artifact_error(error)))?;
    Ok(check_success_line(
        verified.candidate_content_sha256(),
        verified.approval_summary().artifact_manifest_sha256(),
        verified
            .approval_summary()
            .ignored_cash_dividend_row_count(),
        verified
            .approval_summary()
            .ignored_cash_dividend_rows_sha256(),
    ))
}

#[cfg(not(unix))]
fn filesystem_separation_gate(_raw_root: &Path, _artifact_root: &Path) -> Result<(), &'static str> {
    Err("unsupported_platform")
}

#[cfg(unix)]
struct ResolvedDirectory {
    path: PathBuf,
    dev: u64,
    ino: u64,
}

#[cfg(unix)]
fn filesystem_separation_gate(raw_root: &Path, artifact_root: &Path) -> Result<(), &'static str> {
    let raw = resolve_directory(raw_root)?;
    let artifact = resolve_directory(artifact_root)?;

    if artifact.path == raw.path || raw.path.starts_with(&artifact.path) {
        return Err("artifact_root_not_separate");
    }

    let raw_boundary = resolve_optional_directory(&raw.path.join("raw"))?;
    let curated_boundary = resolve_optional_directory(&raw.path.join("curated"))?;
    let boundaries = [raw_boundary, curated_boundary];

    for boundary in boundaries.iter().flatten() {
        if artifact.path == boundary.path || artifact.path.starts_with(&boundary.path) {
            return Err("artifact_root_not_separate");
        }
    }

    if (artifact.dev, artifact.ino) == (raw.dev, raw.ino) {
        return Err("artifact_root_not_separate");
    }
    let forbidden_identities = boundaries
        .iter()
        .flatten()
        .map(|boundary| (boundary.dev, boundary.ino))
        .collect::<Vec<_>>();
    for ancestor in artifact.path.ancestors() {
        let metadata = std::fs::metadata(ancestor).map_err(|_| "root_error")?;
        let identity = (
            std::os::unix::fs::MetadataExt::dev(&metadata),
            std::os::unix::fs::MetadataExt::ino(&metadata),
        );
        if forbidden_identities.contains(&identity) {
            return Err("artifact_root_not_separate");
        }
    }

    Ok(())
}

#[cfg(unix)]
fn resolve_directory(path: &Path) -> Result<ResolvedDirectory, &'static str> {
    let resolved = std::fs::canonicalize(path).map_err(|_| "root_error")?;
    let metadata = std::fs::metadata(&resolved).map_err(|_| "root_error")?;
    if !metadata.is_dir() {
        return Err("root_error");
    }
    Ok(ResolvedDirectory {
        path: resolved,
        dev: std::os::unix::fs::MetadataExt::dev(&metadata),
        ino: std::os::unix::fs::MetadataExt::ino(&metadata),
    })
}

#[cfg(unix)]
fn resolve_optional_directory(path: &Path) -> Result<Option<ResolvedDirectory>, &'static str> {
    let lexical_metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("root_error"),
    };
    let resolved = std::fs::canonicalize(path).map_err(|_| "root_error")?;
    let metadata = std::fs::metadata(&resolved).map_err(|_| "root_error")?;
    if !metadata.is_dir() || lexical_metadata.file_type().is_file() {
        return Err("root_error");
    }
    Ok(Some(ResolvedDirectory {
        path: resolved,
        dev: std::os::unix::fs::MetadataExt::dev(&metadata),
        ino: std::os::unix::fs::MetadataExt::ino(&metadata),
    }))
}

fn materialize_success_line(
    candidate_content_sha256: &ContentHash,
    artifact_manifest_sha256: &ContentHash,
    stage5_manifest_sha256: &ContentHash,
    action_manifest_sha256: &ContentHash,
    ignored_cash_dividend_row_count: usize,
    ignored_cash_dividend_rows_sha256: &ContentHash,
) -> String {
    format!(
        "HISTORICAL_PRICE_BETA_ARTIFACT status=ok operation=materialize candidate_content_sha256={} artifact_manifest_sha256={} stage5_manifest_sha256={} action_manifest_sha256={} instrument_count={} session_count={} bar_count={} cash_dividend_treatment={} ignored_cash_dividends={} ignored_cash_dividend_rows_sha256={} raw_authenticity=PINNED_RAW_VERIFIED_IN_PROCESS audience=OWNER_ONLY vendor_snapshot=true strict_pit=false capability=PRICE_RETURN_ONLY materialization_status=MATERIALIZED registration_status=UNREGISTERED publication_status=NOT_PUBLISHED",
        candidate_content_sha256.as_str(),
        artifact_manifest_sha256.as_str(),
        stage5_manifest_sha256.as_str(),
        action_manifest_sha256.as_str(),
        INSTRUMENT_COUNT,
        SESSION_COUNT,
        BAR_COUNT,
        market_data::HISTORICAL_PRICE_ONLY_CASH_DIVIDEND_TREATMENT,
        ignored_cash_dividend_row_count,
        ignored_cash_dividend_rows_sha256,
    )
}

fn check_success_line(
    candidate_content_sha256: &ContentHash,
    artifact_manifest_sha256: &ContentHash,
    ignored_cash_dividend_row_count: usize,
    ignored_cash_dividend_rows_sha256: &ContentHash,
) -> String {
    format!(
        "HISTORICAL_PRICE_BETA_ARTIFACT status=ok operation=check candidate_content_sha256={} artifact_manifest_sha256={} instrument_count={} session_count={} bar_count={} cash_dividend_treatment={} ignored_cash_dividends={} ignored_cash_dividend_rows_sha256={} raw_authenticity=NOT_REAUTHENTICATED audience=OWNER_ONLY vendor_snapshot=true strict_pit=false capability=PRICE_RETURN_ONLY materialization_status=MATERIALIZED registration_status=UNREGISTERED publication_status=NOT_PUBLISHED",
        candidate_content_sha256.as_str(),
        artifact_manifest_sha256.as_str(),
        INSTRUMENT_COUNT,
        SESSION_COUNT,
        BAR_COUNT,
        market_data::HISTORICAL_PRICE_ONLY_CASH_DIVIDEND_TREATMENT,
        ignored_cash_dividend_row_count,
        ignored_cash_dividend_rows_sha256,
    )
}

fn map_range_error(error: RangeCanonicalError) -> &'static str {
    use RangeCanonicalError as Error;
    match error {
        Error::UnsupportedScope { .. } | Error::UnsupportedMode => "unsupported_scope",
        Error::UnsupportedHistoricalSessionSchedule { .. }
        | Error::MissingListingMasterEvidence { .. }
        | Error::EvidencePackage { .. }
        | Error::UnsafeEvidencePath { .. }
        | Error::EvidenceArtifact { .. } => "unexpected_evidence_surface",
        Error::MissingActionEvidence { .. }
        | Error::ActionEvidence { .. }
        | Error::IncompleteActionPagination { .. } => "action_evidence",
        Error::NonStrictPitNotApproved { .. } => "pit_contract",
        Error::UnsupportedAction { .. } => "unsupported_action",
        Error::InvalidBarValue { .. } => "invalid_bar_value",
        Error::MalformedStage4A { .. }
        | Error::UnsupportedLegacyStage4A { .. }
        | Error::InvalidLineage { .. }
        | Error::InvalidSession { .. }
        | Error::UpstreamManifest { .. } => "stage5_evidence",
        Error::HistoricalBetaContract { .. } => "historical_beta_contract",
        Error::Store(_) | Error::Serialization(_) => "raw_store",
    }
}

fn map_historical_error(error: HistoricalPriceOnlyError) -> &'static str {
    use HistoricalPriceOnlyError as Error;
    match error {
        Error::InputMismatch { .. } => "candidate_input_mismatch",
        Error::DuplicateBar { .. }
        | Error::DuplicateAction { .. }
        | Error::ConflictingAction { .. } => "candidate_duplicate",
        Error::NonCanonicalOrdering { .. } => "candidate_ordering",
        Error::UnsupportedAction { .. } => "unsupported_action",
        Error::InvalidSplitFactor { .. } => "invalid_split_factor",
        Error::OhlcInvariant { .. } => "candidate_ohlc_invariant",
        Error::ArithmeticOverflow { .. } => "candidate_arithmetic_overflow",
        Error::Serialization => "candidate_serialization",
    }
}

fn map_artifact_error(error: HistoricalPriceOnlyArtifactError) -> &'static str {
    use HistoricalPriceOnlyArtifactError as Error;
    match error {
        Error::UnsupportedPlatform => "unsupported_platform",
        Error::UnsafePath => "artifact_unsafe_path",
        Error::InvalidCandidate => "artifact_invalid_candidate",
        Error::InvalidArtifact => "artifact_invalid_artifact",
        Error::Io(_) => "artifact_io",
        Error::Conflict { .. } => "artifact_conflict",
        Error::UnsupportedAtomicNoReplace => "artifact_noreplace_unavailable",
        Error::IndeterminateCommit => "artifact_indeterminate_commit",
        Error::CleanupFailed => "artifact_cleanup_failed",
        Error::StagingNameExhausted => "artifact_staging_name_exhausted",
        _ => "artifact_error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(byte: char) -> String {
        format!("sha256:{}", byte.to_string().repeat(64))
    }

    fn materialize_argv(raw_root: &str, artifact_root: &str) -> Vec<String> {
        vec![
            "materialize".into(),
            "--raw-root".into(),
            raw_root.into(),
            "--artifact-root".into(),
            artifact_root.into(),
            "--stage5-manifest-sha256".into(),
            hash('a'),
            "--action-manifest-sha256".into(),
            hash('b'),
        ]
    }

    fn check_argv(artifact_root: &str) -> Vec<String> {
        vec![
            "check".into(),
            "--artifact-root".into(),
            artifact_root.into(),
            "--candidate-content-sha256".into(),
            hash('c'),
        ]
    }

    #[test]
    fn grammar_is_exact_and_hashes_are_pinned() {
        let parsed = parse_args(&materialize_argv("/data", "/artifacts")).unwrap();
        let ParseOutcome::Command(Command::Materialize(args)) = parsed else {
            panic!("expected materialize command");
        };
        assert_eq!(args.raw_root, PathBuf::from("/data"));
        assert_eq!(args.artifact_root, PathBuf::from("/artifacts"));
        assert_eq!(args.stage5_manifest_sha256.as_str(), hash('a'));
        assert_eq!(args.action_manifest_sha256.as_str(), hash('b'));

        let parsed = parse_args(&check_argv("/artifacts")).unwrap();
        let ParseOutcome::Command(Command::Check(args)) = parsed else {
            panic!("expected check command");
        };
        assert_eq!(args.artifact_root, PathBuf::from("/artifacts"));
        assert_eq!(args.candidate_content_sha256.as_str(), hash('c'));
    }

    #[test]
    fn malformed_missing_repeated_unknown_relative_and_cross_command_options_fail() {
        assert!(parse_args(&[]).is_err());
        assert!(parse_args(&["materialize".into()]).is_err());
        assert!(parse_args(&["check".into(), "--help".into()]).is_err());

        let mut repeated = materialize_argv("/data", "/artifacts");
        repeated[3] = "--raw-root".into();
        assert!(parse_args(&repeated).is_err());

        let mut unknown = materialize_argv("/data", "/artifacts");
        unknown[3] = "--unknown".into();
        assert!(parse_args(&unknown).is_err());

        assert!(parse_args(&materialize_argv("data", "/artifacts")).is_err());
        assert!(parse_args(&check_argv("artifacts")).is_err());

        let mut cross = materialize_argv("/data", "/artifacts");
        cross[5] = "--candidate-content-sha256".into();
        assert!(parse_args(&cross).is_err());

        let mut cross = check_argv("/artifacts");
        cross[3] = "--raw-root".into();
        assert!(parse_args(&cross).is_err());
    }

    #[cfg(unix)]
    fn gate_roots() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        let raw = data.join("raw");
        let curated = data.join("curated");
        let artifacts = data.join("artifacts");
        let sibling = temp.path().join("trusted-sibling");
        std::fs::create_dir_all(&raw).unwrap();
        std::fs::create_dir_all(&curated).unwrap();
        std::fs::create_dir(&artifacts).unwrap();
        std::fs::create_dir(&sibling).unwrap();
        (temp, data, raw, curated, sibling)
    }

    #[cfg(unix)]
    #[test]
    fn separation_gate_rejects_data_raw_curated_aliases_and_allows_separate_roots() {
        use std::os::unix::fs::symlink;

        let (_temp, data, raw, curated, sibling) = gate_roots();
        let raw_child = raw.join("child");
        let curated_child = curated.join("child");
        std::fs::create_dir(&raw_child).unwrap();
        std::fs::create_dir(&curated_child).unwrap();
        let raw_alias = data.join("raw-alias");
        let curated_alias = data.join("curated-alias");
        symlink(&raw, &raw_alias).unwrap();
        symlink(&curated, &curated_alias).unwrap();

        for forbidden in [
            data.clone(),
            raw.clone(),
            raw_child,
            curated.clone(),
            curated_child,
            data.join("raw/../raw"),
            raw_alias,
            curated_alias,
        ] {
            assert_eq!(
                filesystem_separation_gate(&data, &forbidden),
                Err("artifact_root_not_separate")
            );
        }
        assert_eq!(
            filesystem_separation_gate(&data, &data.join("artifacts")),
            Ok(())
        );
        assert_eq!(filesystem_separation_gate(&data, &sibling), Ok(()));
    }

    #[cfg(unix)]
    #[test]
    fn separation_failure_precedes_raw_verification_and_writes_nothing() {
        let (_temp, data, _raw, _curated, _sibling) = gate_roots();
        let before = std::fs::read_dir(&data)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        let command = parse_args(&materialize_argv(
            data.to_str().unwrap(),
            data.to_str().unwrap(),
        ))
        .unwrap();
        let ParseOutcome::Command(command) = command else {
            panic!("expected command");
        };
        let failure = execute(command).unwrap_err();
        assert_eq!(failure.operation, Operation::Materialize);
        assert_eq!(failure.reason, "artifact_root_not_separate");
        assert_eq!(
            std::fs::read_dir(&data)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect::<Vec<_>>(),
            before
        );
    }

    #[cfg(unix)]
    #[test]
    fn separate_root_with_missing_or_bad_raw_evidence_writes_no_artifact() {
        let (_temp, data, raw, _curated, _sibling) = gate_roots();
        let artifacts = data.join("artifacts");
        let command = parse_args(&materialize_argv(
            data.to_str().unwrap(),
            artifacts.to_str().unwrap(),
        ))
        .unwrap();
        let ParseOutcome::Command(command) = command else {
            panic!("expected command");
        };
        let failure = execute(command).unwrap_err();
        assert_eq!(failure.operation, Operation::Materialize);
        assert_eq!(failure.reason, "raw_store");
        assert!(std::fs::read_dir(&artifacts).unwrap().next().is_none());

        let manifest = RawStore::new(&data).manifest_path("kis-daily-range", "kr");
        std::fs::create_dir_all(manifest.parent().unwrap()).unwrap();
        std::fs::write(manifest.parent().unwrap().join("commit.lock"), b"").unwrap();
        std::fs::write(&manifest, b"{bad-json}\n").unwrap();
        let command = parse_args(&materialize_argv(
            data.to_str().unwrap(),
            artifacts.to_str().unwrap(),
        ))
        .unwrap();
        let ParseOutcome::Command(command) = command else {
            panic!("expected command");
        };
        let failure = execute(command).unwrap_err();
        assert_eq!(failure.operation, Operation::Materialize);
        assert_eq!(failure.reason, "raw_store");
        assert!(std::fs::read_dir(&artifacts).unwrap().next().is_none());
        assert!(raw.exists());
    }

    #[cfg(unix)]
    #[test]
    fn check_missing_or_corrupt_artifact_root_writes_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("missing");
        std::fs::create_dir(&missing).unwrap();
        let command = parse_args(&check_argv(missing.to_str().unwrap())).unwrap();
        let ParseOutcome::Command(command) = command else {
            panic!("expected command");
        };
        let failure = execute(command).unwrap_err();
        assert_eq!(failure.operation, Operation::Check);
        assert!(matches!(
            failure.reason,
            "artifact_io" | "artifact_unsafe_path"
        ));
        assert!(std::fs::read_dir(&missing).unwrap().next().is_none());

        let corrupt = temp.path().join("corrupt");
        let version = corrupt.join("kis-historical-price-only-beta").join("v2");
        std::fs::create_dir_all(&version).unwrap();
        std::fs::write(version.join("unexpected"), b"corrupt").unwrap();
        let before = std::fs::read_dir(&corrupt)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        let command = parse_args(&check_argv(corrupt.to_str().unwrap())).unwrap();
        let ParseOutcome::Command(command) = command else {
            panic!("expected command");
        };
        let failure = execute(command).unwrap_err();
        assert_eq!(failure.operation, Operation::Check);
        assert!(matches!(
            failure.reason,
            "artifact_io" | "artifact_unsafe_path"
        ));
        assert_eq!(
            std::fs::read_dir(&corrupt)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect::<Vec<_>>(),
            before
        );
    }

    #[test]
    fn success_lines_are_exact_and_distinguish_raw_authenticity() {
        let candidate = ContentHash::parse(&hash('c')).unwrap();
        let stage5 = ContentHash::parse(&hash('a')).unwrap();
        let action = ContentHash::parse(&hash('b')).unwrap();
        let dividend_rows = ContentHash::parse(&hash('d')).unwrap();
        let artifact = ContentHash::parse(&hash('e')).unwrap();
        let materialize =
            materialize_success_line(&candidate, &artifact, &stage5, &action, 3, &dividend_rows);
        assert_eq!(
            materialize,
            format!(
                "HISTORICAL_PRICE_BETA_ARTIFACT status=ok operation=materialize candidate_content_sha256={} artifact_manifest_sha256={} stage5_manifest_sha256={} action_manifest_sha256={} instrument_count=11 session_count=1608 bar_count=17688 cash_dividend_treatment=CASH_ONLY_EXCLUDED_FROM_PRICE_RETURN_ONLY_V1 ignored_cash_dividends=3 ignored_cash_dividend_rows_sha256={} raw_authenticity=PINNED_RAW_VERIFIED_IN_PROCESS audience=OWNER_ONLY vendor_snapshot=true strict_pit=false capability=PRICE_RETURN_ONLY materialization_status=MATERIALIZED registration_status=UNREGISTERED publication_status=NOT_PUBLISHED",
                candidate.as_str(),
                artifact.as_str(),
                stage5.as_str(),
                action.as_str(),
                dividend_rows.as_str(),
            )
        );
        let check = check_success_line(&candidate, &artifact, 3, &dividend_rows);
        assert_eq!(
            check,
            format!(
                "HISTORICAL_PRICE_BETA_ARTIFACT status=ok operation=check candidate_content_sha256={} artifact_manifest_sha256={} instrument_count=11 session_count=1608 bar_count=17688 cash_dividend_treatment=CASH_ONLY_EXCLUDED_FROM_PRICE_RETURN_ONLY_V1 ignored_cash_dividends=3 ignored_cash_dividend_rows_sha256={} raw_authenticity=NOT_REAUTHENTICATED audience=OWNER_ONLY vendor_snapshot=true strict_pit=false capability=PRICE_RETURN_ONLY materialization_status=MATERIALIZED registration_status=UNREGISTERED publication_status=NOT_PUBLISHED",
                candidate.as_str(),
                artifact.as_str(),
                dividend_rows.as_str(),
            )
        );
        assert!(materialize.contains("raw_authenticity=PINNED_RAW_VERIFIED_IN_PROCESS"));
        assert!(check.contains("raw_authenticity=NOT_REAUTHENTICATED"));
        for output in [materialize, check] {
            for sentinel in [
                "/operator-root-sentinel",
                "provider",
                "request",
                "body",
                "batch-id",
                "credential-sentinel",
            ] {
                assert!(!output.contains(sentinel), "leaked {sentinel}");
            }
            assert!(!output.contains("READY"));
            assert!(!output.contains("strict_pit=true"));
        }
    }

    #[test]
    fn internal_error_mapping_is_static_even_for_sentinel_values() {
        let range = map_range_error(RangeCanonicalError::InvalidBarValue {
            instrument: "/provider-sentinel".into(),
            field: "request-sentinel".into(),
            value: "body-sentinel".into(),
            reason: "batch-sentinel".into(),
        });
        let historical = map_historical_error(HistoricalPriceOnlyError::InputMismatch {
            reason: "/operator-root-sentinel".into(),
        });
        let artifact = map_artifact_error(HistoricalPriceOnlyArtifactError::Io(
            std::io::Error::other("/credential-sentinel"),
        ));
        for reason in [range, historical, artifact] {
            let failure = Failure::new(Operation::Materialize, reason);
            let line = format!(
                "HISTORICAL_PRICE_BETA_ARTIFACT status=blocked operation={} reason={}",
                failure.operation.as_str(),
                failure.reason
            );
            for sentinel in [
                "/provider-sentinel",
                "request-sentinel",
                "body-sentinel",
                "batch-sentinel",
                "/operator-root-sentinel",
                "/credential-sentinel",
            ] {
                assert!(!line.contains(sentinel), "leaked {sentinel}");
            }
        }
    }
}
