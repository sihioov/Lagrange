//! Gated, one-shot KIND correction/version viewer Raw ingest CLI.
//!
//! The browser capture stage owns all navigation and interaction. This binary
//! accepts only its two-file staging directory (`capture.json` and
//! `viewer.html`), validates the exact approved metadata/file contract, and
//! delegates the final strict viewer/body validation and one immutable Raw
//! commit to `market-data`. It performs no network, provider, database, or
//! order calls.

use std::collections::BTreeSet;
use std::fmt;
use std::fs::File;
use std::io::Read;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use chrono::Datelike;
use domain::{TradingDate, UtcTimestamp};
use market_data::providers::kind::{
    KIND_CORRECTION_ARTIFACT_KIND, KIND_CORRECTION_ENTRY_URL, KIND_CORRECTION_SURFACE,
    KIND_CORRECTION_TERMINATION, KIND_CORRECTION_TERMINATION_STAGE, KIND_CORRECTION_VIEWER_FILE,
    KIND_CORRECTION_VIEWER_ORIGIN_PATH, MAX_KIND_CORRECTION_DIAGNOSTIC_COUNT,
    MAX_KIND_CORRECTION_METADATA_BYTES, MAX_KIND_CORRECTION_RESPONSE_BODY_BYTES,
    MAX_KIND_CORRECTION_VIEWER_BYTES,
};
use market_data::{
    FetchMode, KindCorrectionCapture, KindCorrectionResponseDiagnostics, KindError, MARKET_KR,
    ManifestEntry, RawStore, ingest_correction_capture,
};
use serde::Deserialize;

const USAGE: &str = "\
kind-correction-raw --staging <DIR> (--plan | --execute)

Reads a complete KIND correction/version viewer staging directory
(capture.json plus viewer.html) and, with --execute, commits one immutable
disclosure-evidence Raw batch.

Required environment (for both modes):
  KIND_CORRECTION_RAW_ROOT       immutable Raw store root
  KIND_CORRECTION_ENTITLEMENT_REFERENCE

--execute additionally requires:
  KIND_CORRECTION_CONFIRM=KIND_CORRECTION_EVIDENCE_CAPTURE

--plan validates metadata and prints names/counts only; it never reads the
viewer body and never writes a Raw batch.
";

const RAW_ROOT_ENV_VAR: &str = "KIND_CORRECTION_RAW_ROOT";
const ENTITLEMENT_ENV_VAR: &str = "KIND_CORRECTION_ENTITLEMENT_REFERENCE";
const CONFIRM_ENV_VAR: &str = "KIND_CORRECTION_CONFIRM";
const CONFIRM_LITERAL: &str = "KIND_CORRECTION_EVIDENCE_CAPTURE";
const CAPTURE_JSON_FILE: &str = "capture.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Plan,
    Execute,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Args {
    staging: PathBuf,
    action: Action,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestedRange {
    from: String,
    to: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseDiagnostics {
    body_size: u64,
    form_field_count: u64,
    target_handler_occurrences: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureJson {
    source: String,
    entry_url: String,
    surface: String,
    requested_range: RequestedRange,
    anchor_acceptance_number: String,
    viewer_origin_path: String,
    artifact_kind: String,
    retrieved_at: String,
    termination: String,
    termination_stage: String,
    response_diagnostics: ResponseDiagnostics,
    file: String,
}

#[derive(Debug)]
struct LoadedStaging {
    capture: CaptureJson,
    from: TradingDate,
    to: TradingDate,
    retrieved_at: UtcTimestamp,
    viewer_file: File,
    viewer_len: u64,
}

#[derive(Debug)]
struct StagingDirectory {
    path: PathBuf,
    #[cfg(unix)]
    fd: std::os::fd::OwnedFd,
}

#[derive(Debug)]
enum StagingError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    CaptureMetadataTooLarge {
        actual: u64,
        max: u64,
    },
    CaptureMetadataMalformed,
    DirectoryNotReal,
    UnexpectedFile(String),
    MissingViewer,
    ViewerNotRegular,
    ViewerSymlink,
    ViewerTooLarge {
        actual: u64,
        max: u64,
    },
    ViewerChanged,
    InvalidMetadata(&'static str),
    InvalidDate(&'static str),
    ReversedRange,
    InvalidRetrievedAt,
}

impl fmt::Display for StagingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(f, "staging I/O failed at {}: {source}", path.display())
            }
            Self::CaptureMetadataTooLarge { actual, max } => {
                write!(
                    f,
                    "capture.json is {actual} bytes, exceeding the {max}-byte limit"
                )
            }
            Self::CaptureMetadataMalformed => {
                f.write_str("capture.json is malformed or incomplete")
            }
            Self::DirectoryNotReal => {
                f.write_str("staging path must be a real non-symlink directory")
            }
            Self::UnexpectedFile(name) => write!(f, "staging contains unexpected entry {name:?}"),
            Self::MissingViewer => {
                f.write_str("complete correction capture must contain viewer.html")
            }
            Self::ViewerNotRegular => f.write_str("viewer.html must be a regular file"),
            Self::ViewerSymlink => f.write_str("viewer.html must not be a symlink"),
            Self::ViewerTooLarge { actual, max } => {
                write!(
                    f,
                    "viewer.html is {actual} bytes, exceeding the {max}-byte limit"
                )
            }
            Self::ViewerChanged => {
                f.write_str("viewer.html changed while its verified file handle was read")
            }
            Self::InvalidMetadata(field) => write!(f, "capture metadata field {field} is invalid"),
            Self::InvalidDate(field) => {
                write!(f, "requested_range.{field} is not a valid ISO date")
            }
            Self::ReversedRange => {
                f.write_str("requested_range.from must not be after requested_range.to")
            }
            Self::InvalidRetrievedAt => f.write_str("retrieved_at is not RFC3339"),
        }
    }
}

impl std::error::Error for StagingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Debug)]
enum CliError {
    Usage(String),
    MissingEnv(&'static str),
    EmptyEnv(&'static str),
    InvalidConfirmation,
    Staging(StagingError),
    Ingest(KindError),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(message) => write!(f, "{message}\n\n{USAGE}"),
            Self::MissingEnv(name) => write!(f, "required environment variable {name} is not set"),
            Self::EmptyEnv(name) => {
                write!(f, "required environment variable {name} is set but empty")
            }
            Self::InvalidConfirmation => write!(
                f,
                "{CONFIRM_ENV_VAR} must equal exactly {CONFIRM_LITERAL:?}"
            ),
            Self::Staging(error) => write!(f, "{error}"),
            Self::Ingest(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for CliError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Staging(error) => Some(error),
            Self::Ingest(error) => Some(error),
            _ => None,
        }
    }
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.iter().any(|arg| arg == "--help" || arg == "-h") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    match run(&argv) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn parse_args(argv: &[String]) -> Result<Args, CliError> {
    if argv.len() != 3 {
        return Err(CliError::Usage(
            "expected --staging DIR and exactly one of --plan/--execute".to_owned(),
        ));
    }
    let mut staging = None;
    let mut action = None;
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--staging" if staging.is_none() && index + 1 < argv.len() => {
                if argv[index + 1].is_empty() {
                    return Err(CliError::Usage("expected --staging DIR".to_owned()));
                }
                staging = Some(PathBuf::from(&argv[index + 1]));
                index += 2;
            }
            "--plan" if action.is_none() => {
                action = Some(Action::Plan);
                index += 1;
            }
            "--execute" if action.is_none() => {
                action = Some(Action::Execute);
                index += 1;
            }
            _ => {
                return Err(CliError::Usage(
                    "expected --staging DIR and exactly one of --plan/--execute".to_owned(),
                ));
            }
        }
    }
    let Some(staging) = staging else {
        return Err(CliError::Usage("expected --staging DIR".to_owned()));
    };
    let Some(action) = action else {
        return Err(CliError::Usage("expected --plan or --execute".to_owned()));
    };
    Ok(Args { staging, action })
}

fn required_env(name: &'static str) -> Result<String, CliError> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => Ok(value),
        Ok(_) => Err(CliError::EmptyEnv(name)),
        Err(std::env::VarError::NotPresent) => Err(CliError::MissingEnv(name)),
        Err(std::env::VarError::NotUnicode(_)) => Err(CliError::EmptyEnv(name)),
    }
}

fn parse_date(value: &str, field: &'static str) -> Result<TradingDate, StagingError> {
    let bytes = value.as_bytes();
    let exact_shape = bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit());
    if !exact_shape {
        return Err(StagingError::InvalidDate(field));
    }
    TradingDate::parse(value).map_err(|_| StagingError::InvalidDate(field))
}

fn validate_digits(value: &str) -> bool {
    value.len() == 14 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn validate_diagnostic(value: u64) -> bool {
    value > 0 && value <= MAX_KIND_CORRECTION_DIAGNOSTIC_COUNT
}

#[cfg(unix)]
fn errno_to_io(error: rustix::io::Errno) -> std::io::Error {
    std::io::Error::from_raw_os_error(error.raw_os_error())
}

fn open_staging_directory(path: &Path) -> Result<StagingDirectory, StagingError> {
    let path_metadata = std::fs::symlink_metadata(path).map_err(|source| StagingError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if path_metadata.file_type().is_symlink() || !path_metadata.file_type().is_dir() {
        return Err(StagingError::DirectoryNotReal);
    }

    #[cfg(unix)]
    {
        use rustix::fs::{Mode, OFlags, fstat, open};

        let fd = open(
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| StagingError::Io {
            path: path.to_path_buf(),
            source: errno_to_io(error),
        })?;
        let opened = fstat(&fd).map_err(|error| StagingError::Io {
            path: path.to_path_buf(),
            source: errno_to_io(error),
        })?;
        if opened.st_dev != path_metadata.dev() || opened.st_ino != path_metadata.ino() {
            return Err(StagingError::DirectoryNotReal);
        }
        Ok(StagingDirectory {
            path: path.to_path_buf(),
            fd,
        })
    }

    #[cfg(not(unix))]
    Ok(StagingDirectory {
        path: path.to_path_buf(),
    })
}

fn open_staging_child(dir: &StagingDirectory, name: &str) -> Result<File, StagingError> {
    let path = dir.path.join(name);
    #[cfg(unix)]
    {
        use rustix::fs::{Mode, OFlags, openat};

        openat(
            &dir.fd,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map(File::from)
        .map_err(|error| StagingError::Io {
            path,
            source: errno_to_io(error),
        })
    }

    #[cfg(not(unix))]
    File::open(&path).map_err(|source| StagingError::Io { path, source })
}

fn read_capture_json(dir: &StagingDirectory) -> Result<(CaptureJson, u64), StagingError> {
    let metadata_path = dir.path.join(CAPTURE_JSON_FILE);
    #[cfg(not(unix))]
    {
        let path_metadata = std::fs::symlink_metadata(&metadata_path)
            .map_err(|_| StagingError::CaptureMetadataMalformed)?;
        if path_metadata.file_type().is_symlink() || !path_metadata.file_type().is_file() {
            return Err(StagingError::CaptureMetadataMalformed);
        }
    }
    let mut file = open_staging_child(dir, CAPTURE_JSON_FILE)
        .map_err(|_| StagingError::CaptureMetadataMalformed)?;
    let metadata = file.metadata().map_err(|source| StagingError::Io {
        path: metadata_path.clone(),
        source,
    })?;
    if !metadata.file_type().is_file() {
        return Err(StagingError::CaptureMetadataMalformed);
    }
    if metadata.len() > MAX_KIND_CORRECTION_METADATA_BYTES {
        return Err(StagingError::CaptureMetadataTooLarge {
            actual: metadata.len(),
            max: MAX_KIND_CORRECTION_METADATA_BYTES,
        });
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    (&mut file)
        .take(MAX_KIND_CORRECTION_METADATA_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| StagingError::Io {
            path: metadata_path.clone(),
            source,
        })?;
    if bytes.len() as u64 > MAX_KIND_CORRECTION_METADATA_BYTES {
        return Err(StagingError::CaptureMetadataTooLarge {
            actual: bytes.len() as u64,
            max: MAX_KIND_CORRECTION_METADATA_BYTES,
        });
    }
    let final_len = file
        .metadata()
        .map_err(|source| StagingError::Io {
            path: metadata_path,
            source,
        })?
        .len();
    if bytes.len() as u64 != metadata.len() || final_len != metadata.len() {
        return Err(StagingError::CaptureMetadataMalformed);
    }
    serde_json::from_slice(&bytes)
        .map(|capture| (capture, metadata.len()))
        .map_err(|_| StagingError::CaptureMetadataMalformed)
}

fn validate_directory_entries(dir: &StagingDirectory) -> Result<(), StagingError> {
    let allowed: BTreeSet<&str> = [CAPTURE_JSON_FILE, KIND_CORRECTION_VIEWER_FILE]
        .into_iter()
        .collect();

    #[cfg(unix)]
    {
        let entries = rustix::fs::Dir::read_from(&dir.fd).map_err(|error| StagingError::Io {
            path: dir.path.clone(),
            source: errno_to_io(error),
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| StagingError::Io {
                path: dir.path.clone(),
                source: errno_to_io(error),
            })?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if matches!(name.as_str(), "." | "..") {
                continue;
            }
            if !allowed.contains(name.as_str()) {
                return Err(StagingError::UnexpectedFile(name));
            }
        }
        Ok(())
    }

    #[cfg(not(unix))]
    {
        let entries = std::fs::read_dir(&dir.path).map_err(|source| StagingError::Io {
            path: dir.path.clone(),
            source,
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| StagingError::Io {
                path: dir.path.clone(),
                source,
            })?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !allowed.contains(name.as_str()) {
                return Err(StagingError::UnexpectedFile(name));
            }
        }
        Ok(())
    }
}

fn validate_metadata(
    capture: &CaptureJson,
) -> Result<(TradingDate, TradingDate, UtcTimestamp), StagingError> {
    if capture.source != "kind.krx.co.kr" {
        return Err(StagingError::InvalidMetadata("source"));
    }
    if capture.entry_url != KIND_CORRECTION_ENTRY_URL {
        return Err(StagingError::InvalidMetadata("entry_url"));
    }
    if capture.surface != KIND_CORRECTION_SURFACE {
        return Err(StagingError::InvalidMetadata("surface"));
    }
    let from = parse_date(&capture.requested_range.from, "from")?;
    let to = parse_date(&capture.requested_range.to, "to")?;
    if from > to {
        return Err(StagingError::ReversedRange);
    }
    if !validate_digits(&capture.anchor_acceptance_number) {
        return Err(StagingError::InvalidMetadata("anchor_acceptance_number"));
    }
    if capture.viewer_origin_path != KIND_CORRECTION_VIEWER_ORIGIN_PATH {
        return Err(StagingError::InvalidMetadata("viewer_origin_path"));
    }
    if capture.artifact_kind != KIND_CORRECTION_ARTIFACT_KIND {
        return Err(StagingError::InvalidMetadata("artifact_kind"));
    }
    if capture.termination != KIND_CORRECTION_TERMINATION {
        return Err(StagingError::InvalidMetadata("termination"));
    }
    if capture.termination_stage != KIND_CORRECTION_TERMINATION_STAGE {
        return Err(StagingError::InvalidMetadata("termination_stage"));
    }
    if capture.file != KIND_CORRECTION_VIEWER_FILE {
        return Err(StagingError::InvalidMetadata("file"));
    }
    if capture.response_diagnostics.body_size == 0
        || capture.response_diagnostics.body_size > MAX_KIND_CORRECTION_RESPONSE_BODY_BYTES
        || !validate_diagnostic(capture.response_diagnostics.form_field_count)
        || !validate_diagnostic(capture.response_diagnostics.target_handler_occurrences)
    {
        return Err(StagingError::InvalidMetadata("response_diagnostics"));
    }
    let retrieved_at = UtcTimestamp::parse_rfc3339(&capture.retrieved_at)
        .map_err(|_| StagingError::InvalidRetrievedAt)?;
    Ok((from, to, retrieved_at))
}

fn inspect_viewer(dir: &StagingDirectory) -> Result<(File, u64), StagingError> {
    let path = dir.path.join(KIND_CORRECTION_VIEWER_FILE);
    #[cfg(not(unix))]
    {
        let path_metadata =
            std::fs::symlink_metadata(&path).map_err(|source| StagingError::Io {
                path: path.clone(),
                source,
            })?;
        if path_metadata.file_type().is_symlink() {
            return Err(StagingError::ViewerSymlink);
        }
        if !path_metadata.file_type().is_file() {
            return Err(StagingError::ViewerNotRegular);
        }
    }
    let file = match open_staging_child(dir, KIND_CORRECTION_VIEWER_FILE) {
        #[cfg(unix)]
        Err(StagingError::Io { source, .. })
            if source.raw_os_error() == Some(rustix::io::Errno::LOOP.raw_os_error()) =>
        {
            return Err(StagingError::ViewerSymlink);
        }
        result => result?,
    };
    let metadata = file.metadata().map_err(|source| StagingError::Io {
        path: path.clone(),
        source,
    })?;
    if !metadata.file_type().is_file() {
        return Err(StagingError::ViewerNotRegular);
    }
    if metadata.len() == 0 || metadata.len() > MAX_KIND_CORRECTION_VIEWER_BYTES {
        return Err(StagingError::ViewerTooLarge {
            actual: metadata.len(),
            max: MAX_KIND_CORRECTION_VIEWER_BYTES,
        });
    }
    Ok((file, metadata.len()))
}

fn read_viewer_bytes(
    mut file: File,
    expected_len: u64,
    path: &Path,
) -> Result<Vec<u8>, StagingError> {
    let mut bytes = Vec::with_capacity(expected_len as usize);
    (&mut file)
        .take(MAX_KIND_CORRECTION_VIEWER_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| StagingError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    if bytes.len() as u64 > MAX_KIND_CORRECTION_VIEWER_BYTES {
        return Err(StagingError::ViewerTooLarge {
            actual: bytes.len() as u64,
            max: MAX_KIND_CORRECTION_VIEWER_BYTES,
        });
    }
    let final_len = file
        .metadata()
        .map_err(|source| StagingError::Io {
            path: path.to_path_buf(),
            source,
        })?
        .len();
    if bytes.len() as u64 != expected_len || final_len != expected_len {
        return Err(StagingError::ViewerChanged);
    }
    Ok(bytes)
}

fn load_staging(dir: &Path) -> Result<LoadedStaging, StagingError> {
    let directory = open_staging_directory(dir)?;
    validate_directory_entries(&directory)?;
    let (capture, _) = read_capture_json(&directory)?;
    let (viewer_file, viewer_len) = inspect_viewer(&directory).map_err(|error| match error {
        StagingError::Io { ref source, .. } if source.kind() == std::io::ErrorKind::NotFound => {
            StagingError::MissingViewer
        }
        other => other,
    })?;
    let (from, to, retrieved_at) = validate_metadata(&capture)?;
    Ok(LoadedStaging {
        capture,
        from,
        to,
        retrieved_at,
        viewer_file,
        viewer_len,
    })
}

fn run(args: &[String]) -> Result<(), CliError> {
    let args = parse_args(args)?;
    let raw_root = required_env(RAW_ROOT_ENV_VAR)?;
    let entitlement = required_env(ENTITLEMENT_ENV_VAR)?;
    let loaded = load_staging(&args.staging).map_err(CliError::Staging)?;
    match args.action {
        Action::Plan => {
            run_plan(
                &args.staging,
                &loaded.capture,
                loaded.from,
                loaded.to,
                loaded.viewer_len,
                &raw_root,
                &entitlement,
            );
            Ok(())
        }
        Action::Execute => {
            let confirm = required_env(CONFIRM_ENV_VAR)?;
            if confirm != CONFIRM_LITERAL {
                return Err(CliError::InvalidConfirmation);
            }
            let viewer_path = args.staging.join(KIND_CORRECTION_VIEWER_FILE);
            let viewer_bytes =
                read_viewer_bytes(loaded.viewer_file, loaded.viewer_len, &viewer_path)
                    .map_err(CliError::Staging)?;
            let capture = loaded.capture;
            let capture = KindCorrectionCapture {
                source: capture.source,
                entry_url: capture.entry_url,
                surface: capture.surface,
                requested_from: loaded.from,
                requested_to: loaded.to,
                anchor_acceptance_number: capture.anchor_acceptance_number,
                viewer_origin_path: capture.viewer_origin_path,
                artifact_kind: capture.artifact_kind,
                retrieved_at: loaded.retrieved_at,
                termination: capture.termination,
                termination_stage: capture.termination_stage,
                response_diagnostics: KindCorrectionResponseDiagnostics {
                    body_size: capture.response_diagnostics.body_size,
                    form_field_count: capture.response_diagnostics.form_field_count,
                    target_handler_occurrences: capture
                        .response_diagnostics
                        .target_handler_occurrences,
                },
                file_name: capture.file,
                viewer_bytes,
            };
            let date = TradingDate::new(
                loaded.retrieved_at.as_datetime().year(),
                loaded.retrieved_at.as_datetime().month(),
                loaded.retrieved_at.as_datetime().day(),
            )
            .expect("retrieved_at always contains a valid calendar date");
            let store = RawStore::new(raw_root);
            let entry = ingest_correction_capture(
                &store,
                MARKET_KR,
                &date,
                &entitlement,
                FetchMode::Credentialed,
                &capture,
            )
            .map_err(CliError::Ingest)?;
            print_success(&entry);
            Ok(())
        }
    }
}

/// Prints only safe staging metadata and never opens the viewer body or
/// constructs a [`RawStore`]. Both environment values are references supplied
/// by the operator, matching the existing KIND raw CLI's plan contract.
fn run_plan(
    staging: &Path,
    capture: &CaptureJson,
    from: TradingDate,
    to: TradingDate,
    viewer_len: u64,
    raw_root: &str,
    entitlement: &str,
) {
    println!("staging: {}", staging.display());
    println!("surface: {}", capture.surface);
    println!("file: {} size_bytes={viewer_len}", capture.file);
    println!("requested_range: {} to {}", from, to);
    println!("anchor_acceptance_number: present (opaque)");
    println!("raw_root: {raw_root}");
    println!("entitlement_reference: {entitlement}");
    println!("no write was made (--plan)");
}

fn print_success(entry: &ManifestEntry) {
    println!("batch_id: {}", entry.batch_id);
    for file in &entry.files {
        println!(
            "file: {} content_hash={} size_bytes={}",
            file.file_name, file.content_hash, file.size_bytes
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const ANCHOR: &str = "20200207000058";

    fn viewer_bytes() -> Vec<u8> {
        br#"<select id="mainDoc" name="mainDoc"><option value=""></option><option value="20200207000081|Y">2020.02.07 ETF disclosure</option></select>"#.to_vec()
    }

    fn capture_json(response_body_size: u64, file: Option<&str>) -> String {
        let file_field = file
            .map(|value| format!(",\n  \"file\": \"{value}\""))
            .unwrap_or_default();
        format!(
            r#"{{
  "source": "kind.krx.co.kr",
  "entry_url": "{KIND_CORRECTION_ENTRY_URL}",
  "surface": "{KIND_CORRECTION_SURFACE}",
  "requested_range": {{ "from": "2020-02-03", "to": "2020-02-07" }},
  "anchor_acceptance_number": "{ANCHOR}",
  "viewer_origin_path": "{KIND_CORRECTION_VIEWER_ORIGIN_PATH}",
  "artifact_kind": "{KIND_CORRECTION_ARTIFACT_KIND}",
  "retrieved_at": "2026-08-20T00:00:00Z",
  "termination": "{KIND_CORRECTION_TERMINATION}",
  "termination_stage": "{KIND_CORRECTION_TERMINATION_STAGE}",
  "response_diagnostics": {{ "body_size": {response_body_size}, "form_field_count": 1, "target_handler_occurrences": 1 }}{file_field}
}}"#
        )
    }

    fn valid_staging() -> TempDir {
        let staging = tempfile::tempdir().expect("staging tempdir");
        let body = viewer_bytes();
        std::fs::write(staging.path().join(KIND_CORRECTION_VIEWER_FILE), &body)
            .expect("write viewer.html");
        std::fs::write(
            staging.path().join(CAPTURE_JSON_FILE),
            capture_json(12_852, Some(KIND_CORRECTION_VIEWER_FILE)),
        )
        .expect("write capture.json");
        staging
    }

    #[test]
    fn valid_staging_loads_without_parsing_viewer_body() {
        let staging = valid_staging();
        let loaded = load_staging(staging.path()).expect("valid staging must load");
        assert_eq!(loaded.capture.file, KIND_CORRECTION_VIEWER_FILE);
        assert_eq!(loaded.from.to_string(), "2020-02-03");
        assert_eq!(loaded.to.to_string(), "2020-02-07");
        assert_eq!(loaded.retrieved_at.to_string(), "2026-08-20T00:00:00Z");
        assert_eq!(loaded.viewer_len, viewer_bytes().len() as u64);
    }

    #[test]
    fn incomplete_capture_without_viewer_is_rejected() {
        let staging = valid_staging();
        std::fs::remove_file(staging.path().join(KIND_CORRECTION_VIEWER_FILE))
            .expect("remove viewer.html");
        let error = load_staging(staging.path()).expect_err("missing viewer must fail closed");
        assert!(matches!(error, StagingError::MissingViewer));
    }

    #[test]
    fn incomplete_capture_without_file_metadata_is_rejected() {
        let staging = valid_staging();
        let body_size = viewer_bytes().len() as u64;
        std::fs::write(
            staging.path().join(CAPTURE_JSON_FILE),
            capture_json(body_size, None),
        )
        .expect("write incomplete capture metadata");
        let error = load_staging(staging.path()).expect_err("missing file metadata must fail");
        assert!(matches!(error, StagingError::CaptureMetadataMalformed));
    }

    #[test]
    fn termination_must_be_viewer_loaded() {
        let staging = valid_staging();
        let json = capture_json(
            viewer_bytes().len() as u64,
            Some(KIND_CORRECTION_VIEWER_FILE),
        )
        .replace(
            &format!("\"termination\": \"{KIND_CORRECTION_TERMINATION}\""),
            "\"termination\": \"no_response\"",
        );
        std::fs::write(staging.path().join(CAPTURE_JSON_FILE), json)
            .expect("write incomplete termination metadata");
        let error = load_staging(staging.path()).expect_err("incomplete termination must fail");
        assert!(matches!(
            error,
            StagingError::InvalidMetadata("termination")
        ));
    }

    #[test]
    fn metadata_above_size_bound_is_rejected_before_json_parse() {
        let staging = valid_staging();
        let path = staging.path().join(CAPTURE_JSON_FILE);
        std::fs::File::create(&path)
            .expect("create sparse capture metadata")
            .set_len(MAX_KIND_CORRECTION_METADATA_BYTES + 1)
            .expect("extend sparse capture metadata");
        let error = load_staging(staging.path()).expect_err("oversized metadata must fail");
        assert!(matches!(
            error,
            StagingError::CaptureMetadataTooLarge {
                actual,
                max: MAX_KIND_CORRECTION_METADATA_BYTES
            } if actual == MAX_KIND_CORRECTION_METADATA_BYTES + 1
        ));
    }

    #[cfg(unix)]
    #[test]
    fn capture_metadata_symlink_is_rejected() {
        let staging = valid_staging();
        let metadata = staging.path().join(CAPTURE_JSON_FILE);
        let target = tempfile::NamedTempFile::new().expect("create metadata symlink target");
        std::fs::write(
            target.path(),
            capture_json(12_852, Some(KIND_CORRECTION_VIEWER_FILE)),
        )
        .expect("write metadata target");
        std::fs::remove_file(&metadata).expect("remove capture metadata");
        std::os::unix::fs::symlink(target.path(), &metadata)
            .expect("create capture metadata symlink");
        let error = load_staging(staging.path()).expect_err("metadata symlink must fail closed");
        assert!(matches!(error, StagingError::CaptureMetadataMalformed));
    }

    #[test]
    fn extra_directory_entry_is_rejected() {
        let staging = valid_staging();
        std::fs::write(staging.path().join("extra.txt"), b"sentinel")
            .expect("write unexpected entry");
        let error = load_staging(staging.path()).expect_err("extra entry must fail closed");
        assert!(matches!(error, StagingError::UnexpectedFile(name) if name == "extra.txt"));
    }

    #[cfg(unix)]
    #[test]
    fn viewer_symlink_is_rejected() {
        let staging = valid_staging();
        let viewer = staging.path().join(KIND_CORRECTION_VIEWER_FILE);
        let target = tempfile::NamedTempFile::new().expect("create symlink target");
        std::fs::write(target.path(), b"viewer target").expect("write symlink target");
        std::fs::remove_file(&viewer).expect("remove viewer");
        std::os::unix::fs::symlink(target.path(), &viewer).expect("create viewer symlink");
        let error = load_staging(staging.path()).expect_err("viewer symlink must fail closed");
        assert!(matches!(error, StagingError::ViewerSymlink));
    }

    #[test]
    fn viewer_directory_is_rejected() {
        let staging = valid_staging();
        let viewer = staging.path().join(KIND_CORRECTION_VIEWER_FILE);
        std::fs::remove_file(&viewer).expect("remove viewer");
        std::fs::create_dir(&viewer).expect("create viewer directory");
        let error = load_staging(staging.path()).expect_err("viewer directory must fail closed");
        assert!(matches!(error, StagingError::ViewerNotRegular));
    }

    #[test]
    fn viewer_above_size_bound_is_rejected_before_read() {
        let staging = valid_staging();
        let viewer = staging.path().join(KIND_CORRECTION_VIEWER_FILE);
        std::fs::File::create(&viewer)
            .expect("create sparse viewer")
            .set_len(MAX_KIND_CORRECTION_VIEWER_BYTES + 1)
            .expect("extend sparse viewer");
        let error = load_staging(staging.path()).expect_err("oversized viewer must fail closed");
        assert!(matches!(
            error,
            StagingError::ViewerTooLarge {
                actual,
                max: MAX_KIND_CORRECTION_VIEWER_BYTES
            } if actual == MAX_KIND_CORRECTION_VIEWER_BYTES + 1
        ));
    }

    #[test]
    fn response_body_size_is_independent_from_viewer_size() {
        let staging = valid_staging();
        let loaded = load_staging(staging.path()).expect("distinct bounded sizes must be accepted");
        assert_eq!(loaded.capture.response_diagnostics.body_size, 12_852);
        assert_ne!(
            loaded.capture.response_diagnostics.body_size,
            loaded.viewer_len
        );
    }

    #[test]
    fn response_body_size_above_bound_is_rejected() {
        let staging = valid_staging();
        std::fs::write(
            staging.path().join(CAPTURE_JSON_FILE),
            capture_json(
                MAX_KIND_CORRECTION_RESPONSE_BODY_BYTES + 1,
                Some(KIND_CORRECTION_VIEWER_FILE),
            ),
        )
        .expect("write oversized response diagnostic");
        let error = load_staging(staging.path()).expect_err("oversized response must fail closed");
        assert!(matches!(
            error,
            StagingError::InvalidMetadata("response_diagnostics")
        ));
    }

    #[test]
    fn plan_path_does_not_write_raw_store() {
        let staging = valid_staging();
        let raw_root = tempfile::tempdir().expect("raw root tempdir");
        let loaded = load_staging(staging.path()).expect("valid staging must load");
        run_plan(
            staging.path(),
            &loaded.capture,
            loaded.from,
            loaded.to,
            loaded.viewer_len,
            &raw_root.path().display().to_string(),
            "vault://test-entitlements/kind-correction-test-only.pdf",
        );
        assert!(
            std::fs::read_dir(raw_root.path())
                .expect("read raw root")
                .next()
                .is_none(),
            "plan must not create a RawStore directory or batch"
        );
    }

    #[test]
    fn execute_reads_the_verified_viewer_handle_after_path_replacement() {
        let staging = valid_staging();
        let loaded = load_staging(staging.path()).expect("valid staging must load");
        let viewer_path = staging.path().join(KIND_CORRECTION_VIEWER_FILE);
        let moved_path = staging.path().join("viewer-original.html");
        std::fs::rename(&viewer_path, &moved_path).expect("move verified viewer path");
        std::fs::write(&viewer_path, b"replacement must not be read").expect("replace viewer path");

        let bytes = read_viewer_bytes(loaded.viewer_file, loaded.viewer_len, &viewer_path)
            .expect("opened handle must remain pinned to verified viewer");
        assert_eq!(bytes, viewer_bytes());
    }

    #[cfg(unix)]
    #[test]
    fn opened_staging_directory_remains_anchored_after_path_replacement() {
        let staging = valid_staging();
        let directory = open_staging_directory(staging.path()).expect("open staging directory");
        let moved = staging.path().with_extension("opened-directory");
        let replacement = tempfile::tempdir().expect("replacement directory");
        std::fs::write(
            replacement.path().join(CAPTURE_JSON_FILE),
            b"not the capture",
        )
        .expect("write replacement capture");
        std::fs::write(
            replacement.path().join(KIND_CORRECTION_VIEWER_FILE),
            b"not the viewer",
        )
        .expect("write replacement viewer");
        std::fs::rename(staging.path(), &moved).expect("move original staging directory");
        std::os::unix::fs::symlink(replacement.path(), staging.path())
            .expect("replace staging path with symlink directory");

        validate_directory_entries(&directory).expect("directory listing must use opened fd");
        let (capture, _) = read_capture_json(&directory).expect("capture must use opened dirfd");
        let (viewer, viewer_len) =
            inspect_viewer(&directory).expect("viewer must use opened dirfd");
        let bytes = read_viewer_bytes(
            viewer,
            viewer_len,
            &directory.path.join(KIND_CORRECTION_VIEWER_FILE),
        )
        .expect("viewer read must remain on opened directory");
        assert_eq!(capture.anchor_acceptance_number, ANCHOR);
        assert_eq!(bytes, viewer_bytes());

        std::fs::remove_file(staging.path()).expect("remove replacement staging symlink");
        std::fs::rename(&moved, staging.path()).expect("restore tempdir path for cleanup");
    }

    #[test]
    fn execute_bounded_read_rejects_growth_after_viewer_open() {
        let staging = valid_staging();
        let loaded = load_staging(staging.path()).expect("valid staging must load");
        let viewer_path = staging.path().join(KIND_CORRECTION_VIEWER_FILE);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&viewer_path)
            .expect("open viewer writer")
            .set_len(MAX_KIND_CORRECTION_VIEWER_BYTES + 1)
            .expect("grow viewer after validation");

        let error = read_viewer_bytes(loaded.viewer_file, loaded.viewer_len, &viewer_path)
            .expect_err("bounded descriptor read must reject post-open growth");
        assert!(matches!(
            error,
            StagingError::ViewerTooLarge {
                actual,
                max: MAX_KIND_CORRECTION_VIEWER_BYTES
            } if actual == MAX_KIND_CORRECTION_VIEWER_BYTES + 1
        ));
    }
}
