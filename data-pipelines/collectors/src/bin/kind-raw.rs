//! Gated, one-shot KIND ETF disclosure capture-ingest CLI.
//!
//! Reads a **capture staging directory** already produced by a separate
//! browser-capture stage (`capture.json` plus one HTML file per page) and
//! commits it as one immutable Raw batch via
//! [`market_data::ingest_disclosure_capture`]. This binary owns argument
//! parsing, environment validation, staging-directory parsing, and output
//! formatting only — every content/sequencing/credential rule already lives
//! in `market_data::providers::kind`; this file does not duplicate any of
//! it.
//!
//! `capture.json`'s `surface` field must name the approved ETF-scoped KIND
//! search surface (see [`market_data::KindSurface`]). `DetailEtf` remains a
//! compatibility discriminator but is deferred/not allowed for Raw ingest.
//!
//! # Why this is gated
//!
//! `--execute` commits an immutable Raw batch that can never be overwritten.
//! Two independent gates must both pass before anything is written:
//!
//! - `--execute` (not `--plan`) must be the chosen action;
//! - [`CONFIRM_ENV_VAR`] must equal exactly [`CONFIRM_LITERAL`], read fresh
//!   from the environment for this run.
//!
//! Both modes also require [`RAW_ROOT_ENV_VAR`] and
//! [`ENTITLEMENT_REFERENCE_ENV_VAR`] to be set and non-empty.
//!
//! `--plan` never opens a page's HTML file and never constructs a
//! [`RawStore`] or calls [`ingest_disclosure_capture`] — see
//! [`run_plan`], which only ever receives an already-parsed [`CaptureJson`]
//! and prints from it. Loading that structure ([`load_staging`]) itself only
//! reads `capture.json` and the staging directory's file listing; it never
//! opens a page's HTML file either. There is therefore no code path from
//! `--plan` to a write of any kind.
//!
//! # The staging directory is untrusted input
//!
//! The browser-capture stage that produced this directory is not part of
//! this crate and is treated as untrusted, exactly as
//! `market_data::providers::kind` treats every [`CapturedPage`] field: before
//! any page file is opened, its `file` name is validated as a plain file
//! name — no path separator, no `..`, not absolute — see
//! [`validate_page_file_name`]. [`load_staging`] additionally refuses a
//! wrong `surface`/`source`, an empty `pages` array, a malformed
//! `capture.json`, and a staging directory containing an HTML file
//! `capture.json` does not reference.
//!
//! # Output discipline
//!
//! Never a form field *value*, a response body, or file contents — only
//! names, counts, and whatever [`ManifestEntry`]/[`market_data::FileEntry`]
//! already expose.

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use chrono::Datelike;
use domain::{TradingDate, UtcTimestamp};
#[cfg(test)]
use market_data::providers::kind::KIND_ETF_DISCLOSURE_ENDPOINT;
use market_data::providers::kind::{
    KIND_ETF_DISCLOSURE_ENTRY_URL, KIND_ETF_DISCLOSURE_FORWARD, KIND_ETF_DISCLOSURE_METHOD,
    KindCaptureTermination,
};
use market_data::{
    CapturedPage, FetchMode, KindError, KindSurface, MARKET_KR, ManifestEntry, RawStore,
    ingest_disclosure_capture,
};
use serde::Deserialize;

const USAGE: &str = "\
kind-raw --staging <DIR> (--plan | --execute)

Reads a capture staging directory (capture.json plus one HTML file per page)
produced by the browser-capture stage, and ingests it into the immutable Raw
zone as one batch.

Required environment (read at startup, fail closed if absent or empty, for
both --plan and --execute):
  KIND_RAW_ROOT                immutable Raw store root
  KIND_ENTITLEMENT_REFERENCE   the required entitlement reference

--execute additionally requires:
  KIND_CONFIRM = I_UNDERSTAND_READ_ONLY_KIND_INGEST

--plan prints exactly what would be ingested and writes nothing.
--execute ingests the staging directory as one immutable Raw batch.
";

const RAW_ROOT_ENV_VAR: &str = "KIND_RAW_ROOT";
const ENTITLEMENT_REFERENCE_ENV_VAR: &str = "KIND_ENTITLEMENT_REFERENCE";
const CONFIRM_ENV_VAR: &str = "KIND_CONFIRM";
/// The exact literal `KIND_CONFIRM` must equal for `--execute` to run.
const CONFIRM_LITERAL: &str = "I_UNDERSTAND_READ_ONLY_KIND_INGEST";

/// The one capture source this CLI accepts.
const EXPECTED_SOURCE: &str = "kind.krx.co.kr";
/// The staging `surface` id for the one owner-approved Raw surface.
#[cfg(test)]
const EXPECTED_SURFACE: &str = KindSurface::EtfList.staging_id();
/// Compatibility-only staging id; `load_staging` must reject it before any
/// page file/body processing.
#[cfg(test)]
const EXPECTED_SURFACE_DETAIL_ETF: &str = KindSurface::DetailEtf.staging_id();
const CAPTURE_JSON_FILE_NAME: &str = "capture.json";
const HTML_EXTENSION: &str = "html";
/// Maximum accepted size for one untrusted staged response page. Observed
/// KIND pages are roughly 11–13 KiB; 1 MiB leaves ample protocol headroom.
const MAX_CAPTURED_PAGE_BYTES: u64 = 1024 * 1024;

/// `capture.json`'s `requested_range` object.
#[derive(Debug, Deserialize)]
struct RequestedRange {
    from: String,
    to: String,
}

/// One entry of `capture.json`'s `pages` array, exactly as the
/// browser-capture stage wrote it — untrusted until validated by
/// [`load_staging`].
#[derive(Debug, Deserialize)]
struct CapturePageRecord {
    page_index: u32,
    file: String,
    retrieved_at: String,
    form_fields: Vec<(String, String)>,
}

/// The parsed, but not yet content-validated, shape of `capture.json`.
/// [`load_staging`] is the only place one of these is produced, and it never
/// returns one without first checking `surface`, `source`, non-empty
/// `pages`, every page's file-name safety, and the no-stray-file invariant.
#[derive(Debug, Deserialize)]
struct CaptureJson {
    source: String,
    entry_url: String,
    surface: String,
    requested_range: RequestedRange,
    termination: String,
    pages: Vec<CapturePageRecord>,
}

/// Every way a staging directory can be malformed or untrustworthy. Kept
/// distinct from [`CliError`] because these are about the *shape* of
/// untrusted input, not about CLI usage or environment/ingest failures.
#[derive(Debug)]
enum StagingError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// `capture.json` was not valid JSON, or was valid JSON missing a
    /// required field — both surface as this one distinct variant.
    MalformedCaptureJson(serde_json::Error),
    UnsafeFileName(String),
    UnsupportedSurface(String),
    UnsupportedRawSurface(KindSurface),
    UnsupportedSource(String),
    UnsupportedEntryUrl,
    UnsupportedTermination(String),
    IncompleteTermination(KindCaptureTermination),
    EmptyPages,
    InvalidRequestedRangeDate {
        field: &'static str,
    },
    ReversedRequestedRange,
    MissingFormField {
        page_index: u32,
        field: &'static str,
    },
    DuplicateFormField {
        page_index: u32,
        field: &'static str,
    },
    MismatchedFormField {
        page_index: u32,
        field: &'static str,
    },
    InvalidRetrievedAt {
        page_index: u32,
        message: String,
    },
    /// An HTML file on disk that no `pages` entry references.
    StrayFile(String),
}

impl fmt::Display for StagingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(f, "failed to read {}: {source}", path.display())
            }
            Self::MalformedCaptureJson(source) => {
                write!(f, "malformed {CAPTURE_JSON_FILE_NAME}: {source}")
            }
            Self::UnsafeFileName(value) => write!(
                f,
                "unsafe page file name {value:?}: must be a plain file name with no path separator, no `..`, and not absolute"
            ),
            Self::UnsupportedSurface(value) => write!(
                f,
                "unsupported surface {value:?} (expected {:?} or {:?})",
                KindSurface::EtfList.staging_id(),
                KindSurface::DetailEtf.staging_id()
            ),
            Self::UnsupportedRawSurface(surface) => write!(
                f,
                "surface {} is known for compatibility but is not admitted to KIND Raw ingest",
                surface.staging_id()
            ),
            Self::UnsupportedSource(value) => write!(
                f,
                "unsupported source {value:?} (expected {EXPECTED_SOURCE:?})"
            ),
            Self::UnsupportedEntryUrl => write!(
                f,
                "capture entry_url is not the exact approved ETF disclosure-list URL"
            ),
            Self::UnsupportedTermination(value) => {
                write!(f, "unsupported capture termination {value:?}")
            }
            Self::IncompleteTermination(termination) => write!(
                f,
                "capture termination {} is incomplete; only clamped_duplicate may be ingested",
                termination.staging_id()
            ),
            Self::EmptyPages => write!(f, "{CAPTURE_JSON_FILE_NAME} listed no pages"),
            Self::InvalidRequestedRangeDate { field } => {
                write!(f, "requested_range.{field} is not a valid YYYY-MM-DD date")
            }
            Self::ReversedRequestedRange => {
                write!(
                    f,
                    "requested_range.from must not be after requested_range.to"
                )
            }
            Self::MissingFormField { page_index, field } => write!(
                f,
                "page {page_index} is missing required form field {field:?}"
            ),
            Self::DuplicateFormField { page_index, field } => {
                write!(f, "page {page_index} has duplicate form field {field:?}")
            }
            Self::MismatchedFormField { page_index, field } => write!(
                f,
                "page {page_index} has a form field {field:?} that does not match capture metadata"
            ),
            Self::InvalidRetrievedAt {
                page_index,
                message,
            } => write!(
                f,
                "page {page_index} has an invalid retrieved_at: {message}"
            ),
            Self::StrayFile(name) => write!(
                f,
                "staging directory contains {name:?}, an html file {CAPTURE_JSON_FILE_NAME} does not reference"
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Plan,
    Execute,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedArgs {
    staging: PathBuf,
    action: Action,
}

/// Every way this binary refuses to run. `Display` never renders a form
/// field value, a response body, or file contents — see the module doc
/// comment.
#[derive(Debug)]
enum CliError {
    Usage(String),
    MissingEnv(&'static str),
    EmptyEnv(&'static str),
    InvalidConfirm,
    Staging(StagingError),
    PageRead {
        file: String,
        source: std::io::Error,
    },
    PageMetadata {
        file: String,
        source: std::io::Error,
    },
    PageSymlink {
        file: String,
    },
    PageNotRegular {
        file: String,
    },
    PageTooLarge {
        file: String,
        max_bytes: u64,
        actual_bytes: u64,
    },
    Ingest(KindError),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(message) => write!(f, "{message}\n\n{USAGE}"),
            Self::MissingEnv(name) => {
                write!(f, "required environment variable {name} is not set")
            }
            Self::EmptyEnv(name) => {
                write!(f, "required environment variable {name} is set but empty")
            }
            Self::InvalidConfirm => write!(
                f,
                "{CONFIRM_ENV_VAR} must equal exactly {CONFIRM_LITERAL:?} to run --execute"
            ),
            Self::Staging(source) => write!(f, "{source}"),
            Self::PageRead { file, source } => {
                write!(f, "failed to read page file {file:?}: {source}")
            }
            Self::PageMetadata { file, source } => {
                write!(f, "failed to inspect page file {file:?}: {source}")
            }
            Self::PageSymlink { file } => {
                write!(f, "page file {file:?} must not be a symlink")
            }
            Self::PageNotRegular { file } => {
                write!(f, "page file {file:?} must be a regular file")
            }
            Self::PageTooLarge {
                file,
                max_bytes,
                actual_bytes,
            } => write!(
                f,
                "page file {file:?} is {actual_bytes} bytes, exceeding the {max_bytes}-byte limit"
            ),
            Self::Ingest(source) => write!(f, "{source}"),
        }
    }
}

fn main() -> ExitCode {
    let raw_args: Vec<String> = std::env::args().skip(1).collect();
    if raw_args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    match run(&raw_args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run(raw_args: &[String]) -> Result<(), CliError> {
    let args = parse_args(raw_args)?;

    // Required for both --plan and --execute.
    let raw_root = required_env(RAW_ROOT_ENV_VAR)?;
    let entitlement_reference = required_env(ENTITLEMENT_REFERENCE_ENV_VAR)?;

    let (capture, surface, termination) = load_staging(&args.staging).map_err(CliError::Staging)?;

    match args.action {
        Action::Plan => {
            run_plan(
                &args.staging,
                &capture,
                surface,
                &raw_root,
                &entitlement_reference,
            );
            Ok(())
        }
        Action::Execute => {
            let confirm = required_env(CONFIRM_ENV_VAR)?;
            if confirm != CONFIRM_LITERAL {
                return Err(CliError::InvalidConfirm);
            }
            run_execute(
                &args.staging,
                capture,
                surface,
                termination,
                raw_root,
                entitlement_reference,
            )
        }
    }
}

/// Validates one page's `file` value as a plain file name: non-empty, no
/// path separator (`/` or `\`), no `..` component, and not absolute. Called
/// before anything under that name is ever opened.
fn validate_page_file_name(value: &str) -> Result<(), StagingError> {
    let is_safe = !value.is_empty()
        && !value.contains('/')
        && !value.contains('\\')
        && !value.contains("..")
        && !Path::new(value).is_absolute();
    if is_safe {
        Ok(())
    } else {
        Err(StagingError::UnsafeFileName(value.to_owned()))
    }
}

/// Lists `dir` and fails if it contains an `.html` file that `referenced`
/// (the set of file names named in `capture.json`) does not include — a
/// stray file means the staging directory is not what it claims.
fn check_no_stray_html_files(
    dir: &Path,
    referenced: &BTreeSet<String>,
) -> Result<(), StagingError> {
    let entries = std::fs::read_dir(dir).map_err(|source| StagingError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| StagingError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let is_html = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case(HTML_EXTENSION));
        if !is_html {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().into_owned();
        if !referenced.contains(&file_name) {
            return Err(StagingError::StrayFile(file_name));
        }
    }
    Ok(())
}

/// Validates the staged search window using the same ISO date contract as the
/// browser capture arguments, and rejects an inverted range before any page
/// metadata or body is considered.
fn validate_requested_range(range: &RequestedRange) -> Result<(), StagingError> {
    let from = parse_requested_date(&range.from, "from")?;
    let to = parse_requested_date(&range.to, "to")?;
    if from > to {
        return Err(StagingError::ReversedRequestedRange);
    }
    Ok(())
}

fn parse_requested_date(value: &str, field: &'static str) -> Result<TradingDate, StagingError> {
    let bytes = value.as_bytes();
    let exact_shape = bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit());
    if !exact_shape {
        return Err(StagingError::InvalidRequestedRangeDate { field });
    }
    TradingDate::parse(value).map_err(|_| StagingError::InvalidRequestedRangeDate { field })
}

/// Requires exactly one value for a form field and compares it to the
/// capture-level/page-level contract. Missing, duplicated, and mismatched
/// values are all fail-closed metadata errors; the values themselves never
/// enter an error message.
fn validate_required_form_field(
    page: &CapturePageRecord,
    field: &'static str,
    expected: &str,
) -> Result<(), StagingError> {
    let mut values = page
        .form_fields
        .iter()
        .filter(|(name, _)| name == field)
        .map(|(_, value)| value.as_str());
    let Some(value) = values.next() else {
        return Err(StagingError::MissingFormField {
            page_index: page.page_index,
            field,
        });
    };
    if values.next().is_some() {
        return Err(StagingError::DuplicateFormField {
            page_index: page.page_index,
            field,
        });
    }
    if value != expected {
        return Err(StagingError::MismatchedFormField {
            page_index: page.page_index,
            field,
        });
    }
    Ok(())
}

/// An observed field is checked when present, but absence is allowed. This is
/// important for fields that vary with KIND's rendered form; the loader must
/// not invent a requirement for a field the capture did not observe.
fn validate_optional_exact_form_field(
    page: &CapturePageRecord,
    field: &'static str,
    expected: &str,
) -> Result<(), StagingError> {
    let mut values = page
        .form_fields
        .iter()
        .filter(|(name, _)| name == field)
        .map(|(_, value)| value.as_str());
    let Some(value) = values.next() else {
        return Ok(());
    };
    if values.next().is_some() {
        return Err(StagingError::DuplicateFormField {
            page_index: page.page_index,
            field,
        });
    }
    if value != expected {
        return Err(StagingError::MismatchedFormField {
            page_index: page.page_index,
            field,
        });
    }
    Ok(())
}

/// Validates the browser-observed request metadata. Date, page-index, method,
/// and forward fields are required because together they bind each stored
/// response to the approved ETF-list request. The optional surface field is
/// exact-checked when the browser records it.
fn validate_page_form_fields(
    page: &CapturePageRecord,
    requested_range: &RequestedRange,
    surface: KindSurface,
) -> Result<(), StagingError> {
    validate_required_form_field(page, "fromDate", &requested_range.from)?;
    validate_required_form_field(page, "toDate", &requested_range.to)?;
    let expected_page_index = page.page_index.to_string();
    validate_required_form_field(page, "pageIndex", &expected_page_index)?;
    validate_required_form_field(page, "method", KIND_ETF_DISCLOSURE_METHOD)?;
    validate_required_form_field(page, "forward", KIND_ETF_DISCLOSURE_FORWARD)?;
    validate_optional_exact_form_field(page, "surface", surface.staging_id())?;
    Ok(())
}

/// Reads and validates `dir/capture.json` plus `dir`'s file listing. Never
/// opens a page's HTML file — see the module doc comment. This is the one
/// function both `--plan` and `--execute` call to make sense of a staging
/// directory.
///
/// Returns the parsed [`CaptureJson`] alongside the [`KindSurface`] its
/// `surface` field named. Known compatibility surfaces are parsed first, then
/// the Raw admission gate rejects every surface except [`KindSurface::EtfList`]
/// before page-file processing.
fn load_staging(
    dir: &Path,
) -> Result<(CaptureJson, KindSurface, KindCaptureTermination), StagingError> {
    let capture_path = dir.join(CAPTURE_JSON_FILE_NAME);
    let raw = std::fs::read_to_string(&capture_path).map_err(|source| StagingError::Io {
        path: capture_path.clone(),
        source,
    })?;
    let capture: CaptureJson =
        serde_json::from_str(&raw).map_err(StagingError::MalformedCaptureJson)?;

    let surface = KindSurface::parse_staging_id(&capture.surface)
        .ok_or_else(|| StagingError::UnsupportedSurface(capture.surface.clone()))?;
    surface
        .ensure_raw_admitted()
        .map_err(|_| StagingError::UnsupportedRawSurface(surface))?;
    if capture.source != EXPECTED_SOURCE {
        return Err(StagingError::UnsupportedSource(capture.source));
    }
    if capture.entry_url != KIND_ETF_DISCLOSURE_ENTRY_URL {
        return Err(StagingError::UnsupportedEntryUrl);
    }
    validate_requested_range(&capture.requested_range)?;
    let termination = KindCaptureTermination::parse_staging_id(&capture.termination)
        .ok_or_else(|| StagingError::UnsupportedTermination(capture.termination.clone()))?;
    if !termination.is_complete() {
        return Err(StagingError::IncompleteTermination(termination));
    }
    if capture.pages.is_empty() {
        return Err(StagingError::EmptyPages);
    }

    let mut referenced_file_names: BTreeSet<String> = BTreeSet::new();
    for page in &capture.pages {
        validate_page_form_fields(page, &capture.requested_range, surface)?;
        validate_page_file_name(&page.file)?;
        UtcTimestamp::parse_rfc3339(&page.retrieved_at).map_err(|source| {
            StagingError::InvalidRetrievedAt {
                page_index: page.page_index,
                message: source.to_string(),
            }
        })?;
        referenced_file_names.insert(page.file.clone());
    }

    check_no_stray_html_files(dir, &referenced_file_names)?;

    Ok((capture, surface, termination))
}

/// Reads every page's bytes from disk exactly as they are (no transform)
/// and assembles the [`CapturedPage`] list [`ingest_disclosure_capture`]
/// expects. Only ever called from `--execute`; `--plan` never calls this.
/// The checks protect a static staging tree. Concurrent replacement between
/// metadata inspection and the read is outside this patch's threat model; that
/// would require platform-specific fd-based `O_NOFOLLOW` and bounded reads.
fn build_captured_pages(dir: &Path, capture: &CaptureJson) -> Result<Vec<CapturedPage>, CliError> {
    let mut pages = Vec::with_capacity(capture.pages.len());
    for page in &capture.pages {
        let path = dir.join(&page.file);
        let metadata =
            std::fs::symlink_metadata(&path).map_err(|source| CliError::PageMetadata {
                file: page.file.clone(),
                source,
            })?;
        if metadata.file_type().is_symlink() {
            return Err(CliError::PageSymlink {
                file: page.file.clone(),
            });
        }
        if !metadata.file_type().is_file() {
            return Err(CliError::PageNotRegular {
                file: page.file.clone(),
            });
        }
        if metadata.len() > MAX_CAPTURED_PAGE_BYTES {
            return Err(CliError::PageTooLarge {
                file: page.file.clone(),
                max_bytes: MAX_CAPTURED_PAGE_BYTES,
                actual_bytes: metadata.len(),
            });
        }
        let bytes = std::fs::read(&path).map_err(|source| CliError::PageRead {
            file: page.file.clone(),
            source,
        })?;
        let retrieved_at = UtcTimestamp::parse_rfc3339(&page.retrieved_at)
            .expect("load_staging already validated retrieved_at parses");
        pages.push(CapturedPage {
            page_index: page.page_index,
            bytes,
            retrieved_at,
            form_fields: page.form_fields.clone(),
        });
    }
    Ok(pages)
}

/// Prints exactly what would be ingested and returns without ever
/// constructing a [`RawStore`] or calling [`ingest_disclosure_capture`]
/// — there is no code path from here to a write of any kind.
fn run_plan(
    staging_dir: &Path,
    capture: &CaptureJson,
    surface: KindSurface,
    raw_root: &str,
    entitlement_reference: &str,
) {
    println!("staging: {}", staging_dir.display());
    println!("surface: {}", capture.surface);
    println!("endpoint: {}", surface.endpoint_id());
    println!(
        "requested_range: {} to {}",
        capture.requested_range.from, capture.requested_range.to
    );
    println!("page_count: {}", capture.pages.len());
    for page in &capture.pages {
        println!("page: index={} file={}", page.page_index, page.file);
        let field_names: Vec<&str> = page
            .form_fields
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        println!("  form_field_names: {}", field_names.join(", "));
    }
    println!("raw_root: {raw_root}");
    println!("entitlement_reference: {entitlement_reference}");
    println!("no write was made (--plan)");
}

fn run_execute(
    staging_dir: &Path,
    capture: CaptureJson,
    surface: KindSurface,
    termination: KindCaptureTermination,
    raw_root: String,
    entitlement_reference: String,
) -> Result<(), CliError> {
    let pages = build_captured_pages(staging_dir, &capture)?;

    let store = RawStore::new(raw_root);
    let today = UtcTimestamp::now().as_datetime().date_naive();
    let date = TradingDate::new(today.year(), today.month(), today.day())
        .expect("the system clock's current UTC date is always a valid calendar date");

    let entry = ingest_disclosure_capture(
        &store,
        MARKET_KR,
        &date,
        &entitlement_reference,
        FetchMode::Credentialed,
        surface,
        termination,
        &pages,
    )
    .map_err(CliError::Ingest)?;

    print_success(&entry);
    Ok(())
}

/// Prints only what [`ManifestEntry`]/[`market_data::FileEntry`] already
/// expose: the batch id and, per file, its name, content hash, and byte
/// size. Never a form field value, never file contents.
fn print_success(entry: &ManifestEntry) {
    println!("batch_id: {}", entry.batch_id);
    for file in &entry.files {
        println!(
            "file: {} content_hash={} size_bytes={}",
            file.file_name, file.content_hash, file.size_bytes
        );
    }
}

fn required_env(name: &'static str) -> Result<String, CliError> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => Ok(value),
        Ok(_) => Err(CliError::EmptyEnv(name)),
        Err(_) => Err(CliError::MissingEnv(name)),
    }
}

fn parse_args(raw_args: &[String]) -> Result<ParsedArgs, CliError> {
    let mut staging: Option<PathBuf> = None;
    let mut action: Option<Action> = None;

    let mut iter = raw_args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--staging" => {
                let value = next_value(&mut iter, "--staging")?;
                reject_duplicate("--staging", staging.replace(PathBuf::from(value)))?;
            }
            "--plan" => {
                reject_duplicate_action(action.replace(Action::Plan))?;
            }
            "--execute" => {
                reject_duplicate_action(action.replace(Action::Execute))?;
            }
            other => {
                return Err(CliError::Usage(format!("unrecognized argument: {other}")));
            }
        }
    }

    let staging = staging.ok_or_else(|| CliError::Usage("--staging is required".to_owned()))?;
    let action = action.ok_or_else(|| {
        CliError::Usage("exactly one of --plan or --execute is required".to_owned())
    })?;

    Ok(ParsedArgs { staging, action })
}

fn next_value<'a>(
    iter: &mut std::slice::Iter<'a, String>,
    flag: &'static str,
) -> Result<&'a str, CliError> {
    iter.next()
        .map(String::as_str)
        .ok_or_else(|| CliError::Usage(format!("{flag} requires a value")))
}

fn reject_duplicate<T>(flag: &'static str, previous: Option<T>) -> Result<(), CliError> {
    if previous.is_some() {
        Err(CliError::Usage(format!(
            "{flag} may only be specified once"
        )))
    } else {
        Ok(())
    }
}

fn reject_duplicate_action(previous: Option<Action>) -> Result<(), CliError> {
    if previous.is_some() {
        Err(CliError::Usage(
            "--plan and --execute are mutually exclusive; specify exactly one".to_owned(),
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    fn write_capture_json(dir: &Path, json: &str) {
        std::fs::write(dir.join(CAPTURE_JSON_FILE_NAME), json).expect("write capture.json");
    }

    fn write_page_file(dir: &Path, name: &str, marker: &str) {
        let body = format!("<table><tr><th>시간</th></tr><tr><td>{marker}</td></tr></table>");
        std::fs::write(dir.join(name), body).expect("write page file");
    }

    fn page_snippet(index: u32, file: &str) -> String {
        format!(
            r#"{{ "page_index": {index}, "file": "{file}", "retrieved_at": "2026-08-19T23:16:09Z", "form_fields": [["method", "searchDisclosureByStockTypeEtfSub"], ["forward", "disclosurebystocktype_etf_sub"], ["fromDate", "2020-02-03"], ["toDate", "2020-02-07"], ["pageIndex", "{index}"]] }}"#
        )
    }

    fn capture_json_full(surface: &str, source: &str, pages_json: &str) -> String {
        format!(
            r#"{{
  "source": "{source}",
  "entry_url": "{KIND_ETF_DISCLOSURE_ENTRY_URL}",
  "surface": "{surface}",
  "requested_range": {{ "from": "2020-02-03", "to": "2020-02-07" }},
  "termination": "clamped_duplicate",
  "pages": [{pages_json}]
}}"#
        )
    }

    fn two_page_capture_json() -> String {
        let pages = format!(
            "{},{}",
            page_snippet(1, "page-0001.html"),
            page_snippet(2, "page-0002.html")
        );
        capture_json_full(EXPECTED_SURFACE, EXPECTED_SOURCE, &pages)
    }

    // -----------------------------------------------------------------
    // 1. Argument parsing.
    // -----------------------------------------------------------------

    #[test]
    fn missing_staging_is_rejected() {
        assert!(parse_args(&args(&["--plan"])).is_err());
    }

    #[test]
    fn plan_and_execute_are_required_and_mutually_exclusive() {
        assert!(
            parse_args(&args(&["--staging", "/tmp/x"])).is_err(),
            "one of --plan/--execute is required"
        );
        assert!(
            parse_args(&args(&["--staging", "/tmp/x", "--plan", "--execute"])).is_err(),
            "--plan and --execute are mutually exclusive"
        );
    }

    #[test]
    fn duplicate_flags_are_rejected() {
        assert!(
            parse_args(&args(&[
                "--staging",
                "/tmp/a",
                "--staging",
                "/tmp/b",
                "--plan"
            ]))
            .is_err()
        );
        assert!(parse_args(&args(&["--staging", "/tmp/a", "--plan", "--plan"])).is_err());
    }

    #[test]
    fn unrecognized_arguments_are_rejected() {
        assert!(parse_args(&args(&["--staging", "/tmp/a", "--plan", "--bogus"])).is_err());
        assert!(parse_args(&args(&["positional-garbage"])).is_err());
    }

    #[test]
    fn valid_args_parse_into_expected_shape() {
        let parsed =
            parse_args(&args(&["--staging", "/tmp/a", "--plan"])).expect("valid plan args");
        assert_eq!(parsed.staging, PathBuf::from("/tmp/a"));
        assert_eq!(parsed.action, Action::Plan);

        let parsed =
            parse_args(&args(&["--staging", "/tmp/a", "--execute"])).expect("valid execute args");
        assert_eq!(parsed.action, Action::Execute);
    }

    // -----------------------------------------------------------------
    // 2. Unsafe `file` values are rejected before any read.
    // -----------------------------------------------------------------

    #[test]
    fn unsafe_page_file_names_are_rejected() {
        for bad in [
            "../evil.html",
            "sub/dir.html",
            "sub\\dir.html",
            "/etc/passwd",
            "..",
            "",
        ] {
            assert!(
                validate_page_file_name(bad).is_err(),
                "{bad:?} must be rejected"
            );
        }
        for good in ["page-0001.html", "page.html"] {
            assert!(
                validate_page_file_name(good).is_ok(),
                "{good:?} should be accepted"
            );
        }
    }

    #[test]
    fn staging_load_rejects_unsafe_file_name_before_any_read() {
        let temp = tempfile::tempdir().expect("tempdir");
        let json = capture_json_full(
            EXPECTED_SURFACE,
            EXPECTED_SOURCE,
            &page_snippet(1, "../escape.html"),
        );
        write_capture_json(temp.path(), &json);
        // Deliberately do NOT create ../escape.html anywhere. If the loader
        // ever attempted to open it before validating the name, this would
        // surface as an unrelated Io error instead of UnsafeFileName.
        let error = load_staging(temp.path()).expect_err("unsafe file name must be rejected");
        assert!(matches!(error, StagingError::UnsafeFileName(name) if name == "../escape.html"));
    }

    // -----------------------------------------------------------------
    // 3. Wrong surface or source.
    // -----------------------------------------------------------------

    #[test]
    fn wrong_surface_is_rejected() {
        let temp = tempfile::tempdir().expect("tempdir");
        let json = capture_json_full(
            "etf-disclosure-list-v2",
            EXPECTED_SOURCE,
            &page_snippet(1, "page-0001.html"),
        );
        write_capture_json(temp.path(), &json);
        write_page_file(temp.path(), "page-0001.html", "A");

        let error = load_staging(temp.path()).expect_err("wrong surface must be rejected");
        assert!(
            matches!(error, StagingError::UnsupportedSurface(s) if s == "etf-disclosure-list-v2")
        );
    }

    #[test]
    fn wrong_source_is_rejected() {
        let temp = tempfile::tempdir().expect("tempdir");
        let json = capture_json_full(
            EXPECTED_SURFACE,
            "kind.krx.co.kr.evil",
            &page_snippet(1, "page-0001.html"),
        );
        write_capture_json(temp.path(), &json);
        write_page_file(temp.path(), "page-0001.html", "A");

        let error = load_staging(temp.path()).expect_err("wrong source must be rejected");
        assert!(matches!(error, StagingError::UnsupportedSource(s) if s == "kind.krx.co.kr.evil"));
    }

    #[test]
    fn unknown_or_wrong_entry_url_is_rejected_before_page_processing() {
        for entry_url in [
            "https://kind.krx.co.kr/disclosure/details.do",
            "https://example.invalid/not-kind",
        ] {
            let temp = tempfile::tempdir().expect("tempdir");
            let json = capture_json_full(
                EXPECTED_SURFACE,
                EXPECTED_SOURCE,
                &page_snippet(1, "page-0001.html"),
            )
            .replace(KIND_ETF_DISCLOSURE_ENTRY_URL, entry_url);
            write_capture_json(temp.path(), &json);
            // No page file: the URL gate must fire before any page-file read.

            let error = load_staging(temp.path()).expect_err("wrong entry_url must be rejected");
            assert!(matches!(error, StagingError::UnsupportedEntryUrl));
        }
    }

    // -----------------------------------------------------------------
    // 3b. The approved staging surface is accepted; the compatibility-only
    //     DetailEtf id is rejected before any page file is processed.
    // -----------------------------------------------------------------

    #[test]
    fn etf_list_surface_is_accepted() {
        let temp = tempfile::tempdir().expect("tempdir");
        let json = capture_json_full(
            EXPECTED_SURFACE,
            EXPECTED_SOURCE,
            &page_snippet(1, "page-0001.html"),
        );
        write_capture_json(temp.path(), &json);
        write_page_file(temp.path(), "page-0001.html", "A");

        let (_capture, surface, termination) =
            load_staging(temp.path()).expect("ETF-list surface must be accepted");
        assert_eq!(surface, KindSurface::EtfList);
        assert_eq!(surface.endpoint_id(), KIND_ETF_DISCLOSURE_ENDPOINT);
        assert_eq!(termination, KindCaptureTermination::ClampedDuplicate);
    }

    #[test]
    fn detail_etf_surface_is_rejected_before_page_processing_for_plan_and_execute() {
        let temp = tempfile::tempdir().expect("tempdir");
        let json = capture_json_full(
            EXPECTED_SURFACE_DETAIL_ETF,
            EXPECTED_SOURCE,
            &page_snippet(1, "page-0001.html"),
        );
        write_capture_json(temp.path(), &json);
        // A directory would fail execute-only page processing if the surface
        // gate were placed too late. `load_staging` must reject first, which
        // is also the gate used by --plan.
        std::fs::create_dir(temp.path().join("page-0001.html"))
            .expect("create sentinel page directory");

        let error = load_staging(temp.path()).expect_err("DetailEtf must be rejected");
        assert!(matches!(
            error,
            StagingError::UnsupportedRawSurface(KindSurface::DetailEtf)
        ));
    }

    #[test]
    fn unknown_surface_id_is_rejected() {
        let temp = tempfile::tempdir().expect("tempdir");
        let json = capture_json_full(
            "some-other-surface",
            EXPECTED_SOURCE,
            &page_snippet(1, "page-0001.html"),
        );
        write_capture_json(temp.path(), &json);
        write_page_file(temp.path(), "page-0001.html", "A");

        let error = load_staging(temp.path()).expect_err("unknown surface must be rejected");
        assert!(matches!(error, StagingError::UnsupportedSurface(s) if s == "some-other-surface"));
    }

    // -----------------------------------------------------------------
    // 4. Empty `pages` array.
    // -----------------------------------------------------------------

    #[test]
    fn empty_pages_array_is_rejected() {
        let temp = tempfile::tempdir().expect("tempdir");
        let json = capture_json_full(EXPECTED_SURFACE, EXPECTED_SOURCE, "");
        write_capture_json(temp.path(), &json);

        let error = load_staging(temp.path()).expect_err("empty pages must be rejected");
        assert!(matches!(error, StagingError::EmptyPages));
    }

    #[test]
    fn missing_or_unknown_capture_termination_is_rejected() {
        let missing = tempfile::tempdir().expect("tempdir");
        let json = r#"{
  "source": "kind.krx.co.kr",
  "surface": "etf-disclosure-list",
  "requested_range": { "from": "2020-02-03", "to": "2020-02-07" },
  "pages": []
}"#;
        write_capture_json(missing.path(), json);
        let error = load_staging(missing.path()).expect_err("missing termination must be rejected");
        assert!(matches!(error, StagingError::MalformedCaptureJson(_)));

        let unknown = tempfile::tempdir().expect("tempdir");
        let json = capture_json_full(EXPECTED_SURFACE, EXPECTED_SOURCE, "")
            .replace("clamped_duplicate", "future_terminal");
        write_capture_json(unknown.path(), &json);
        let error = load_staging(unknown.path()).expect_err("unknown termination must be rejected");
        assert!(
            matches!(error, StagingError::UnsupportedTermination(value) if value == "future_terminal")
        );
    }

    #[test]
    fn incomplete_capture_termination_is_rejected_before_page_processing() {
        for (value, expected) in [
            (
                "page_bound_reached",
                KindCaptureTermination::PageBoundReached,
            ),
            (
                "advance_control_missing",
                KindCaptureTermination::AdvanceControlMissing,
            ),
            ("no_response", KindCaptureTermination::NoResponse),
        ] {
            let temp = tempfile::tempdir().expect("tempdir");
            let json = capture_json_full(EXPECTED_SURFACE, EXPECTED_SOURCE, "")
                .replace("clamped_duplicate", value);
            write_capture_json(temp.path(), &json);

            let error = load_staging(temp.path())
                .expect_err("incomplete termination must be rejected before empty pages");
            assert!(
                matches!(error, StagingError::IncompleteTermination(actual) if actual == expected)
            );
        }
    }

    // -----------------------------------------------------------------
    // 5. Stray HTML file not referenced by `pages`.
    // -----------------------------------------------------------------

    #[test]
    fn stray_html_file_not_referenced_is_rejected() {
        let temp = tempfile::tempdir().expect("tempdir");
        let json = capture_json_full(
            EXPECTED_SURFACE,
            EXPECTED_SOURCE,
            &page_snippet(1, "page-0001.html"),
        );
        write_capture_json(temp.path(), &json);
        write_page_file(temp.path(), "page-0001.html", "A");
        write_page_file(temp.path(), "page-0002.html", "B"); // not referenced

        let error = load_staging(temp.path()).expect_err("stray file must be rejected");
        assert!(matches!(error, StagingError::StrayFile(name) if name == "page-0002.html"));
    }

    // -----------------------------------------------------------------
    // 6. Malformed capture.json: not JSON, or missing a required field.
    // -----------------------------------------------------------------

    #[test]
    fn capture_json_that_is_not_json_is_rejected_with_distinct_error() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            temp.path().join(CAPTURE_JSON_FILE_NAME),
            b"not json at all {{{",
        )
        .expect("write garbage");

        let error = load_staging(temp.path()).expect_err("garbage must be rejected");
        assert!(matches!(error, StagingError::MalformedCaptureJson(_)));
    }

    #[test]
    fn capture_json_missing_required_field_is_rejected_with_distinct_error() {
        let temp = tempfile::tempdir().expect("tempdir");
        // Missing "surface" entirely.
        let json = r#"{
  "source": "kind.krx.co.kr",
  "entry_url": "https://kind.krx.co.kr/disclosure/disclosurebystocktype.do?method=searchDisclosureByStockTypeEtf",
  "requested_range": { "from": "2020-02-03", "to": "2020-02-07" },
  "pages": []
}"#;
        write_capture_json(temp.path(), json);

        let error = load_staging(temp.path()).expect_err("missing field must be rejected");
        assert!(matches!(error, StagingError::MalformedCaptureJson(_)));
    }

    #[test]
    fn capture_json_missing_entry_url_is_rejected_as_malformed() {
        let temp = tempfile::tempdir().expect("tempdir");
        let json = r#"{
  "source": "kind.krx.co.kr",
  "surface": "etf-disclosure-list",
  "requested_range": { "from": "2020-02-03", "to": "2020-02-07" },
  "termination": "clamped_duplicate",
  "pages": []
}"#;
        write_capture_json(temp.path(), json);

        let error = load_staging(temp.path()).expect_err("missing entry_url must be rejected");
        assert!(matches!(error, StagingError::MalformedCaptureJson(_)));
    }

    // -----------------------------------------------------------------
    // 7. A valid staging directory parses into the expected CapturedPage
    //    values.
    // -----------------------------------------------------------------

    #[test]
    fn valid_staging_directory_parses_into_expected_captured_pages() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_capture_json(temp.path(), &two_page_capture_json());
        write_page_file(temp.path(), "page-0001.html", "PAGE-1");
        write_page_file(temp.path(), "page-0002.html", "PAGE-2");

        let (capture, surface, termination) =
            load_staging(temp.path()).expect("valid staging directory must load");
        assert_eq!(capture.pages.len(), 2);
        assert_eq!(surface, KindSurface::EtfList);
        assert_eq!(termination, KindCaptureTermination::ClampedDuplicate);

        let pages = build_captured_pages(temp.path(), &capture).expect("bytes must be readable");
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].page_index, 1);
        assert_eq!(pages[1].page_index, 2);

        let expected_page_1 = std::fs::read(temp.path().join("page-0001.html")).unwrap();
        let expected_page_2 = std::fs::read(temp.path().join("page-0002.html")).unwrap();
        assert_eq!(pages[0].bytes, expected_page_1);
        assert_eq!(pages[1].bytes, expected_page_2);

        for page in &pages {
            let names: Vec<&str> = page.form_fields.iter().map(|(n, _)| n.as_str()).collect();
            assert_eq!(
                names,
                vec!["method", "forward", "fromDate", "toDate", "pageIndex"]
            );
        }
    }

    #[test]
    fn requested_range_and_page_indices_bind_exactly_to_form_fields() {
        type FormFieldCase = (&'static str, fn(String) -> String, bool, &'static str);
        let cases: [FormFieldCase; 7] = [
            (
                "from_mismatch",
                |json: String| {
                    json.replace(
                        "\"fromDate\", \"2020-02-03\"",
                        "\"fromDate\", \"2020-02-04\"",
                    )
                },
                false,
                "fromDate",
            ),
            (
                "to_mismatch",
                |json: String| {
                    json.replace("\"toDate\", \"2020-02-07\"", "\"toDate\", \"2020-02-08\"")
                },
                false,
                "toDate",
            ),
            (
                "page_index_mismatch",
                |json: String| json.replace("\"pageIndex\", \"1\"", "\"pageIndex\", \"2\""),
                false,
                "pageIndex",
            ),
            (
                "page_index_duplicate",
                |json: String| {
                    json.replace(
                        "[\"pageIndex\", \"1\"]",
                        "[\"pageIndex\", \"1\"], [\"pageIndex\", \"1\"]",
                    )
                },
                true,
                "pageIndex",
            ),
            (
                "method_mismatch",
                |json: String| {
                    json.replace(
                        "searchDisclosureByStockTypeEtfSub",
                        "searchDisclosureByStockTypeEtfOther",
                    )
                },
                false,
                "method",
            ),
            (
                "forward_mismatch",
                |json: String| {
                    json.replace(
                        "disclosurebystocktype_etf_sub",
                        "disclosurebystocktype_other",
                    )
                },
                false,
                "forward",
            ),
            (
                "surface_discriminator_mismatch",
                |json: String| {
                    json.replace(
                        "[\"pageIndex\", \"1\"]",
                        "[\"pageIndex\", \"1\"], [\"surface\", \"etf-detail\"]",
                    )
                },
                false,
                "surface",
            ),
        ];

        for (label, mutate, duplicate, expected_field) in cases {
            let temp = tempfile::tempdir().expect("tempdir");
            let json = mutate(capture_json_full(
                EXPECTED_SURFACE,
                EXPECTED_SOURCE,
                &page_snippet(1, "page-0001.html"),
            ));
            write_capture_json(temp.path(), &json);
            let error = load_staging(temp.path()).expect_err(label);
            match (error, duplicate) {
                (StagingError::MismatchedFormField { page_index, field }, false) => {
                    assert_eq!(page_index, 1, "{label}: page index");
                    assert_eq!(field, expected_field, "{label}: field");
                }
                (StagingError::DuplicateFormField { page_index, field }, true) => {
                    assert_eq!(page_index, 1, "{label}: page index");
                    assert_eq!(field, expected_field, "{label}: field");
                }
                (other, _) => panic!("{label}: unexpected error {other:?}"),
            }
        }
    }

    #[test]
    fn required_form_fields_cannot_be_missing_and_optional_surface_is_not_invented() {
        let temp = tempfile::tempdir().expect("tempdir");
        let json = capture_json_full(
            EXPECTED_SURFACE,
            EXPECTED_SOURCE,
            &page_snippet(1, "page-0001.html").replace("[\"fromDate\", \"2020-02-03\"], ", ""),
        );
        write_capture_json(temp.path(), &json);
        let error = load_staging(temp.path()).expect_err("missing fromDate must be rejected");
        assert!(matches!(
            error,
            StagingError::MissingFormField {
                page_index: 1,
                field: "fromDate"
            }
        ));

        let temp = tempfile::tempdir().expect("tempdir");
        let json = capture_json_full(
            EXPECTED_SURFACE,
            EXPECTED_SOURCE,
            &page_snippet(1, "page-0001.html")
                .replace("[\"method\", \"searchDisclosureByStockTypeEtfSub\"], ", ""),
        );
        write_capture_json(temp.path(), &json);
        let error = load_staging(temp.path()).expect_err("missing method must be rejected");
        assert!(matches!(
            error,
            StagingError::MissingFormField {
                page_index: 1,
                field: "method"
            }
        ));

        let temp = tempfile::tempdir().expect("tempdir");
        let json = capture_json_full(
            EXPECTED_SURFACE,
            EXPECTED_SOURCE,
            &page_snippet(1, "page-0001.html"),
        );
        write_capture_json(temp.path(), &json);
        let (_capture, surface, termination) =
            load_staging(temp.path()).expect("absent optional surface is allowed");
        assert_eq!(surface, KindSurface::EtfList);
        assert_eq!(termination, KindCaptureTermination::ClampedDuplicate);
    }

    #[test]
    fn requested_range_date_format_and_order_are_fail_closed() {
        let malformed = capture_json_full(
            EXPECTED_SURFACE,
            EXPECTED_SOURCE,
            &page_snippet(1, "page-0001.html"),
        )
        .replace("\"from\": \"2020-02-03\"", "\"from\": \"2020-2-3\"");
        let temp = tempfile::tempdir().expect("tempdir");
        write_capture_json(temp.path(), &malformed);
        let error = load_staging(temp.path()).expect_err("malformed date must be rejected");
        assert!(matches!(
            error,
            StagingError::InvalidRequestedRangeDate { field: "from" }
        ));

        let reversed = capture_json_full(
            EXPECTED_SURFACE,
            EXPECTED_SOURCE,
            &page_snippet(1, "page-0001.html"),
        )
        .replace("\"from\": \"2020-02-03\"", "\"from\": \"2020-02-08\"");
        let temp = tempfile::tempdir().expect("tempdir");
        write_capture_json(temp.path(), &reversed);
        let error = load_staging(temp.path()).expect_err("reversed range must be rejected");
        assert!(matches!(error, StagingError::ReversedRequestedRange));
    }

    // -----------------------------------------------------------------
    // 8. Execute-only page file safety checks.
    // -----------------------------------------------------------------

    #[cfg(unix)]
    #[test]
    fn execute_reader_rejects_a_symlinked_page_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_capture_json(
            temp.path(),
            &capture_json_full(
                EXPECTED_SURFACE,
                EXPECTED_SOURCE,
                &page_snippet(1, "page-0001.html"),
            ),
        );
        std::fs::write(temp.path().join("target"), b"not a page").expect("write target");
        std::os::unix::fs::symlink("target", temp.path().join("page-0001.html"))
            .expect("create symlink");

        let (capture, _, _) =
            load_staging(temp.path()).expect("plan-time parse must not open page");
        let error = match build_captured_pages(temp.path(), &capture) {
            Err(error) => error,
            Ok(_) => panic!("execute reader must reject a symlink"),
        };
        assert!(matches!(error, CliError::PageSymlink { file } if file == "page-0001.html"));
    }

    #[test]
    fn execute_reader_rejects_a_non_regular_page_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_capture_json(
            temp.path(),
            &capture_json_full(
                EXPECTED_SURFACE,
                EXPECTED_SOURCE,
                &page_snippet(1, "page-0001.html"),
            ),
        );
        std::fs::create_dir(temp.path().join("page-0001.html")).expect("create directory");

        let (capture, _, _) =
            load_staging(temp.path()).expect("directory is not opened by staging parse");
        let error = match build_captured_pages(temp.path(), &capture) {
            Err(error) => error,
            Ok(_) => panic!("execute reader must reject a directory"),
        };
        assert!(matches!(error, CliError::PageNotRegular { file } if file == "page-0001.html"));
    }

    #[test]
    fn execute_reader_rejects_a_sparse_page_larger_than_the_limit() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_capture_json(
            temp.path(),
            &capture_json_full(
                EXPECTED_SURFACE,
                EXPECTED_SOURCE,
                &page_snippet(1, "page-0001.html"),
            ),
        );
        std::fs::File::create(temp.path().join("page-0001.html"))
            .expect("create sparse page")
            .set_len(MAX_CAPTURED_PAGE_BYTES + 1)
            .expect("extend sparse page");

        let (capture, _, _) = load_staging(temp.path()).expect("staging parse must not read bytes");
        let error = match build_captured_pages(temp.path(), &capture) {
            Err(error) => error,
            Ok(_) => panic!("execute reader must reject an oversized page before reading it"),
        };
        assert!(matches!(
            error,
            CliError::PageTooLarge {
                file,
                max_bytes: MAX_CAPTURED_PAGE_BYTES,
                actual_bytes,
            } if file == "page-0001.html" && actual_bytes == MAX_CAPTURED_PAGE_BYTES + 1
        ));
    }

    #[test]
    fn execute_reader_accepts_a_page_at_the_exact_size_limit() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_capture_json(
            temp.path(),
            &capture_json_full(
                EXPECTED_SURFACE,
                EXPECTED_SOURCE,
                &page_snippet(1, "page-0001.html"),
            ),
        );
        let path = temp.path().join("page-0001.html");
        std::fs::write(
            &path,
            b"<table><tr><th>\xEC\x8B\x9C\xEA\xB0\x84</th></tr></table>",
        )
        .expect("write page");
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("open page")
            .set_len(MAX_CAPTURED_PAGE_BYTES)
            .expect("extend page to exact limit");

        let (capture, _, _) = load_staging(temp.path()).expect("valid staging directory must load");
        let pages = build_captured_pages(temp.path(), &capture)
            .expect("an exactly-limit regular page must be readable");
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].bytes.len() as u64, MAX_CAPTURED_PAGE_BYTES);
    }

    // -----------------------------------------------------------------
    // 9. The plan path writes nothing and never opens page bodies.
    // -----------------------------------------------------------------

    #[test]
    fn plan_path_writes_nothing() {
        let staging = tempfile::tempdir().expect("staging tempdir");
        let raw_root = tempfile::tempdir().expect("raw root tempdir");

        write_capture_json(staging.path(), &two_page_capture_json());
        write_page_file(staging.path(), "page-0001.html", "PAGE-1");
        write_page_file(staging.path(), "page-0002.html", "PAGE-2");

        let (capture, surface, _termination) =
            load_staging(staging.path()).expect("valid staging directory must load");
        run_plan(
            staging.path(),
            &capture,
            surface,
            &raw_root.path().display().to_string(),
            "vault://test-entitlements/kind-test-only.pdf",
        );

        let entries: Vec<_> = std::fs::read_dir(raw_root.path())
            .expect("read raw root")
            .collect();
        assert!(
            entries.is_empty(),
            "the plan path must never write into the raw root"
        );
    }

    #[test]
    fn plan_path_does_not_open_a_referenced_page_body() {
        let staging = tempfile::tempdir().expect("staging tempdir");
        let raw_root = tempfile::tempdir().expect("raw root tempdir");
        write_capture_json(
            staging.path(),
            &capture_json_full(
                EXPECTED_SURFACE,
                EXPECTED_SOURCE,
                &page_snippet(1, "page-0001.html"),
            ),
        );
        std::fs::create_dir(staging.path().join("page-0001.html"))
            .expect("create non-regular page entry");

        let (capture, surface, _) =
            load_staging(staging.path()).expect("plan-time staging parse must not open the page");
        run_plan(
            staging.path(),
            &capture,
            surface,
            &raw_root.path().display().to_string(),
            "vault://test-entitlements/kind-test-only.pdf",
        );
        assert!(
            std::fs::read_dir(raw_root.path())
                .expect("read raw root")
                .next()
                .is_none(),
            "the plan path must not write into the raw root"
        );
    }

    #[test]
    fn confirm_literal_matches_the_documented_value() {
        assert_eq!(CONFIRM_LITERAL, "I_UNDERSTAND_READ_ONLY_KIND_INGEST");
    }
}
