//! Gated, one-shot OpenDART Raw-ingest CLI.
//!
//! Performs exactly one fetch of one documented OpenDART read surface
//! (`corpcode`, `list`, or `company`) and stores it as one immutable Raw
//! batch via [`market_data::OpenDartProvider`]. This binary owns argument
//! parsing, environment validation, and output formatting only — every
//! actual HTTP/validation/storage rule lives in `opendart-client` and
//! `market-data`; this file adds none of its own.
//!
//! # Why this is gated
//!
//! A live OpenDART call is a real, rate-limited network request against a
//! third party. Three independent gates must all pass before one is ever
//! sent:
//!
//! - `--execute` (not `--plan`) must be the chosen action;
//! - `OPENDART_CONFIRM` must equal exactly [`CONFIRM_LITERAL`], read fresh
//!   from the environment for this run;
//! - every other required environment variable
//!   ([`opendart_client::CRTFC_KEY_FILE_ENV_VAR`], [`RAW_ROOT_ENV_VAR`],
//!   [`ENTITLEMENT_REFERENCE_ENV_VAR`]) must be set and non-empty.
//!
//! `--plan` never constructs a live transport at all — see [`run_plan`] —
//! so it cannot make a request no matter what the environment holds.
//!
//! # The credential never enters this binary
//!
//! This CLI never calls `std::env::var` on the OpenDART key itself, never
//! opens the file [`opendart_client::CRTFC_KEY_FILE_ENV_VAR`] names, and
//! never asks [`opendart_client::OpenDartClient`] for the loaded value.
//! [`opendart_client::CRTFC_KEY_FILE_ENV_VAR`] is read here only to confirm
//! it is set (fail closed if not) and, optionally, that the path it names
//! exists on disk (a plain `Path::exists` check — the file is never
//! opened). Reading the actual key from that file is entirely
//! [`opendart_client::OpenDartClient::with_default_credentials`]'s job, on
//! the far side of a crate boundary this binary never reaches across.
//!
//! Every printed line in this file is reviewed against the same rule:
//! never the key, never a full URL, never a query parameter *value*, never
//! response bytes. Only query parameter *names*, typed error `Display`s,
//! and the fields [`market_data::ManifestEntry`]/[`market_data::FileEntry`]
//! already expose are ever printed.

use std::fmt;
use std::path::Path;
use std::process::ExitCode;

use chrono::Datelike;
use domain::{TradingDate, UtcTimestamp};
use market_data::{
    DisclosureListFilter, FetchMode, MARKET_KR, ManifestEntry, OPENDART_DISCLOSURE_LIST_ENDPOINT,
    OPENDART_ENTITY_COMPANY_ENDPOINT, OPENDART_ENTITY_CORPCODE_ENDPOINT, OpenDartError,
    OpenDartOutcome, OpenDartProvider, RawStore,
};
use opendart_client::{
    CRTFC_KEY_FILE_ENV_VAR, ClientConfig, OpenDartClient, OpenDartTransportError,
};

const USAGE: &str = "\
opendart-raw --surface <corpcode|list|company> [--corp-code <8 digits>] \
[--bgn-de <YYYYMMDD>] [--end-de <YYYYMMDD>] (--plan | --execute)

Surfaces:
  corpcode   GET /api/corpCode.xml   -- no further parameters
  company    GET /api/company.json   -- requires --corp-code
  list       GET /api/list.json      -- requires --corp-code;
                                         --bgn-de/--end-de optional

Required environment (all read at startup, all fail closed if absent or
empty):
  OPENDART_CRTFC_KEY_FILE       path to the OpenDART credential file
  OPENDART_RAW_ROOT             immutable Raw store root
  OPENDART_ENTITLEMENT_REFERENCE  the required entitlement reference

--execute additionally requires:
  OPENDART_CONFIRM = I_UNDERSTAND_READ_ONLY_OPENDART_CALLS

--plan prints exactly what would be requested and makes no network request.
--execute performs exactly one fetch and stores one immutable Raw batch.
";

/// House ceiling: 10-second connect timeout. Never read from configuration
/// -- there is no house-approved reason to raise it for this CLI.
const OPENDART_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// House ceiling: 30-second read timeout.
const OPENDART_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// House ceiling: 1 request per second, matching `opendart-client`'s own
/// documented pacing convention.
const OPENDART_MIN_REQUEST_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// The reserved OpenDART credential query parameter name. Never a value --
/// only ever printed as a bare name in `--plan` output, mirroring the
/// treatment already established in `market-data` and `opendart-client`.
const CRTFC_KEY_PARAM: &str = "crtfc_key";

const RAW_ROOT_ENV_VAR: &str = "OPENDART_RAW_ROOT";
const ENTITLEMENT_REFERENCE_ENV_VAR: &str = "OPENDART_ENTITLEMENT_REFERENCE";
const CONFIRM_ENV_VAR: &str = "OPENDART_CONFIRM";
/// The exact literal `OPENDART_CONFIRM` must equal for `--execute` to run.
const CONFIRM_LITERAL: &str = "I_UNDERSTAND_READ_ONLY_OPENDART_CALLS";

const LIST_JSON_PATH: &str = "/api/list.json";
const CORP_CODE_XML_PATH: &str = "/api/corpCode.xml";
const COMPANY_JSON_PATH: &str = "/api/company.json";

/// One of the three documented OpenDART read surfaces this CLI can fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Surface {
    CorpCode,
    List,
    Company,
}

impl Surface {
    fn parse(value: &str) -> Result<Self, CliError> {
        match value {
            "corpcode" => Ok(Self::CorpCode),
            "list" => Ok(Self::List),
            "company" => Ok(Self::Company),
            other => Err(CliError::Usage(format!(
                "unknown --surface value {other:?} (expected corpcode, list, or company)"
            ))),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::CorpCode => "corpcode",
            Self::List => "list",
            Self::Company => "company",
        }
    }

    fn path(self) -> &'static str {
        match self {
            Self::CorpCode => CORP_CODE_XML_PATH,
            Self::List => LIST_JSON_PATH,
            Self::Company => COMPANY_JSON_PATH,
        }
    }

    fn endpoint_id(self) -> &'static str {
        match self {
            Self::CorpCode => OPENDART_ENTITY_CORPCODE_ENDPOINT,
            Self::List => OPENDART_DISCLOSURE_LIST_ENDPOINT,
            Self::Company => OPENDART_ENTITY_COMPANY_ENDPOINT,
        }
    }

    /// Names only, in the exact order the underlying adapter builds them
    /// -- never values. See the module doc comment's leak rule.
    fn query_parameter_names(self, args: &ParsedArgs) -> Vec<&'static str> {
        match self {
            Self::CorpCode => vec![CRTFC_KEY_PARAM],
            Self::Company => vec!["corp_code", CRTFC_KEY_PARAM],
            Self::List => {
                let mut names = vec!["page_no", "page_count", "corp_code"];
                if args.bgn_de.is_some() {
                    names.push("bgn_de");
                }
                if args.end_de.is_some() {
                    names.push("end_de");
                }
                names.push(CRTFC_KEY_PARAM);
                names
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Plan,
    Execute,
}

#[derive(Debug, Clone)]
struct ParsedArgs {
    surface: Surface,
    corp_code: Option<String>,
    bgn_de: Option<String>,
    end_de: Option<String>,
    action: Action,
}

/// Every way this binary refuses to run. `Display` never renders a key, a
/// URL, a query parameter value, or response bytes -- see the module doc
/// comment.
#[derive(Debug)]
enum CliError {
    Usage(String),
    MissingEnv(&'static str),
    EmptyEnv(&'static str),
    InvalidConfirm,
    /// The path named by [`CRTFC_KEY_FILE_ENV_VAR`] does not exist. Carries
    /// only that path (never the key it would contain, which this binary
    /// never reads).
    CredentialFileMissing(String),
    Transport(OpenDartTransportError),
    Ingest(OpenDartError),
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
            Self::CredentialFileMissing(path) => write!(
                f,
                "credential file named by {CRTFC_KEY_FILE_ENV_VAR} does not exist: {path}"
            ),
            Self::Transport(source) => write!(f, "opendart transport failure: {source}"),
            Self::Ingest(source) => write!(f, "{source}"),
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let raw_args: Vec<String> = std::env::args().skip(1).collect();
    if raw_args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    match run(&raw_args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

async fn run(raw_args: &[String]) -> Result<(), CliError> {
    let args = parse_args(raw_args)?;

    // Required for both --plan and --execute.
    let key_file_path = required_env(CRTFC_KEY_FILE_ENV_VAR)?;
    if !Path::new(&key_file_path).exists() {
        return Err(CliError::CredentialFileMissing(key_file_path));
    }
    let raw_root = required_env(RAW_ROOT_ENV_VAR)?;
    let entitlement_reference = required_env(ENTITLEMENT_REFERENCE_ENV_VAR)?;

    match args.action {
        Action::Plan => {
            run_plan(&args, &raw_root, &entitlement_reference);
            Ok(())
        }
        Action::Execute => {
            let confirm = required_env(CONFIRM_ENV_VAR)?;
            if confirm != CONFIRM_LITERAL {
                return Err(CliError::InvalidConfirm);
            }
            run_execute(&args, raw_root, entitlement_reference).await
        }
    }
}

/// Prints exactly what would be requested and returns without ever
/// constructing a transport, a client, or a query. This is what guarantees
/// `--plan` makes no request: there is no code path from here to
/// [`opendart_client::OpenDartClient`] at all.
fn run_plan(args: &ParsedArgs, raw_root: &str, entitlement_reference: &str) {
    print!("{}", render_plan(args, raw_root, entitlement_reference));
}

/// Renders the exact safe `--plan` output. Query parameter *names* are shown
/// for operator review, while parsed query values never enter this string.
fn render_plan(args: &ParsedArgs, raw_root: &str, entitlement_reference: &str) -> String {
    format!(
        "surface: {}\npath: {}\nendpoint: {}\nraw_root: {raw_root}\nentitlement_reference: {entitlement_reference}\nquery_parameter_names: {}\nno request was made (--plan)\n",
        args.surface.as_str(),
        args.surface.path(),
        args.surface.endpoint_id(),
        args.surface.query_parameter_names(args).join(", "),
    )
}

async fn run_execute(
    args: &ParsedArgs,
    raw_root: String,
    entitlement_reference: String,
) -> Result<(), CliError> {
    let config = ClientConfig {
        connect_timeout: OPENDART_CONNECT_TIMEOUT,
        read_timeout: OPENDART_READ_TIMEOUT,
        min_request_interval: OPENDART_MIN_REQUEST_INTERVAL,
    };
    let client = OpenDartClient::with_default_credentials(config).map_err(CliError::Transport)?;
    let provider = OpenDartProvider::new(client);
    let store = RawStore::new(raw_root);

    let retrieved_at = UtcTimestamp::now();
    let today = retrieved_at.as_datetime().date_naive();
    let date = TradingDate::new(today.year(), today.month(), today.day())
        .expect("the system clock's current UTC date is always a valid calendar date");

    let outcome = match args.surface {
        Surface::CorpCode => {
            provider
                .ingest_entity_master(
                    &store,
                    MARKET_KR,
                    &date,
                    retrieved_at,
                    FetchMode::Credentialed,
                    &entitlement_reference,
                )
                .await
        }
        Surface::Company => {
            let corp_code = args
                .corp_code
                .as_deref()
                .expect("parse_args guarantees --corp-code for --surface company");
            provider
                .ingest_entity_profile(
                    &store,
                    MARKET_KR,
                    &date,
                    retrieved_at,
                    FetchMode::Credentialed,
                    corp_code,
                    &entitlement_reference,
                )
                .await
        }
        Surface::List => {
            let filter = DisclosureListFilter {
                corp_code: args.corp_code.as_deref(),
                bgn_de: args.bgn_de.as_deref(),
                end_de: args.end_de.as_deref(),
            };
            provider
                .ingest_disclosure_index(
                    &store,
                    MARKET_KR,
                    &date,
                    retrieved_at,
                    FetchMode::Credentialed,
                    filter,
                    &entitlement_reference,
                )
                .await
        }
    }
    .map_err(CliError::Ingest)?;

    match outcome {
        OpenDartOutcome::Stored(entry) => {
            print_success(args.surface, &entry);
        }
        OpenDartOutcome::Empty => {
            println!("surface: {}", args.surface.as_str());
            println!(
                "outcome: opendart reported the documented status=013 (no data); no batch was stored"
            );
        }
    }
    Ok(())
}

/// Prints only what [`ManifestEntry`]/[`market_data::FileEntry`] already
/// expose: the batch id and, per file, its name, content hash, and byte
/// size. Never the request bytes, never a query value.
fn print_success(surface: Surface, entry: &ManifestEntry) {
    println!("surface: {}", surface.as_str());
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
    let mut surface: Option<Surface> = None;
    let mut corp_code: Option<String> = None;
    let mut bgn_de: Option<String> = None;
    let mut end_de: Option<String> = None;
    let mut action: Option<Action> = None;

    let mut iter = raw_args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--surface" => {
                let value = next_value(&mut iter, "--surface")?;
                reject_duplicate("--surface", surface.replace(Surface::parse(value)?))?;
            }
            "--corp-code" => {
                let value = next_value(&mut iter, "--corp-code")?;
                reject_duplicate("--corp-code", corp_code.replace(value.to_owned()))?;
            }
            "--bgn-de" => {
                let value = next_value(&mut iter, "--bgn-de")?;
                reject_duplicate("--bgn-de", bgn_de.replace(value.to_owned()))?;
            }
            "--end-de" => {
                let value = next_value(&mut iter, "--end-de")?;
                reject_duplicate("--end-de", end_de.replace(value.to_owned()))?;
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

    let surface = surface.ok_or_else(|| CliError::Usage("--surface is required".to_owned()))?;
    let action = action.ok_or_else(|| {
        CliError::Usage("exactly one of --plan or --execute is required".to_owned())
    })?;

    if let Some(corp_code) = &corp_code
        && !is_exactly_n_ascii_digits(corp_code, 8)
    {
        return Err(CliError::Usage(
            "--corp-code must be exactly 8 ASCII digits".to_owned(),
        ));
    }
    for (flag, value) in [("--bgn-de", &bgn_de), ("--end-de", &end_de)] {
        if let Some(value) = value
            && !is_exactly_n_ascii_digits(value, 8)
        {
            return Err(CliError::Usage(format!(
                "{flag} must be exactly 8 ASCII digits (YYYYMMDD)"
            )));
        }
    }

    match surface {
        Surface::CorpCode => {
            if corp_code.is_some() || bgn_de.is_some() || end_de.is_some() {
                return Err(CliError::Usage(
                    "--surface corpcode accepts no --corp-code/--bgn-de/--end-de".to_owned(),
                ));
            }
        }
        Surface::Company => {
            if corp_code.is_none() {
                return Err(CliError::Usage(
                    "--surface company requires --corp-code".to_owned(),
                ));
            }
            if bgn_de.is_some() || end_de.is_some() {
                return Err(CliError::Usage(
                    "--surface company accepts no --bgn-de/--end-de".to_owned(),
                ));
            }
        }
        Surface::List => {
            if corp_code.is_none() {
                return Err(CliError::Usage(
                    "--surface list requires --corp-code".to_owned(),
                ));
            }
        }
    }

    Ok(ParsedArgs {
        surface,
        corp_code,
        bgn_de,
        end_de,
        action,
    })
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

fn is_exactly_n_ascii_digits(value: &str, n: usize) -> bool {
    value.len() == n && value.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn corpcode_surface_requires_no_further_parameters() {
        let parsed = parse_args(&args(&["--surface", "corpcode", "--plan"]))
            .expect("corpcode with no filters is valid");
        assert_eq!(parsed.surface, Surface::CorpCode);
        assert_eq!(parsed.action, Action::Plan);

        assert!(
            parse_args(&args(&[
                "--surface",
                "corpcode",
                "--corp-code",
                "00126380",
                "--plan"
            ]))
            .is_err(),
            "corpcode must reject an extraneous --corp-code"
        );
    }

    #[test]
    fn company_surface_requires_corp_code_and_rejects_date_filters() {
        assert!(
            parse_args(&args(&["--surface", "company", "--plan"])).is_err(),
            "company without --corp-code must fail"
        );
        let parsed = parse_args(&args(&[
            "--surface",
            "company",
            "--corp-code",
            "00126380",
            "--plan",
        ]))
        .expect("company with a valid corp_code is valid");
        assert_eq!(parsed.corp_code.as_deref(), Some("00126380"));

        assert!(
            parse_args(&args(&[
                "--surface",
                "company",
                "--corp-code",
                "00126380",
                "--bgn-de",
                "20260101",
                "--plan"
            ]))
            .is_err(),
            "company must reject date filters"
        );
    }

    #[test]
    fn list_surface_requires_corp_code_but_dates_are_optional() {
        assert!(
            parse_args(&args(&["--surface", "list", "--plan"])).is_err(),
            "list without --corp-code must fail (unbounded search window)"
        );
        let parsed = parse_args(&args(&[
            "--surface",
            "list",
            "--corp-code",
            "00126380",
            "--plan",
        ]))
        .expect("list with only corp_code is valid");
        assert!(parsed.bgn_de.is_none() && parsed.end_de.is_none());

        let parsed = parse_args(&args(&[
            "--surface",
            "list",
            "--corp-code",
            "00126380",
            "--bgn-de",
            "20260101",
            "--end-de",
            "20260201",
            "--execute",
        ]))
        .expect("list with corp_code and both dates is valid");
        assert_eq!(parsed.bgn_de.as_deref(), Some("20260101"));
        assert_eq!(parsed.end_de.as_deref(), Some("20260201"));
        assert_eq!(parsed.action, Action::Execute);
    }

    #[test]
    fn corp_code_and_dates_must_be_exactly_8_ascii_digits() {
        for bad in ["0012638", "001263800", "0012638a", "", "0012 638"] {
            assert!(
                parse_args(&args(&[
                    "--surface",
                    "company",
                    "--corp-code",
                    bad,
                    "--plan"
                ]))
                .is_err(),
                "corp_code {bad:?} must be rejected"
            );
        }
        for bad in ["2026101", "202601011", "2026-01-01"] {
            assert!(
                parse_args(&args(&[
                    "--surface",
                    "list",
                    "--corp-code",
                    "00126380",
                    "--bgn-de",
                    bad,
                    "--plan"
                ]))
                .is_err(),
                "bgn_de {bad:?} must be rejected"
            );
        }
    }

    #[test]
    fn plan_and_execute_are_required_and_mutually_exclusive() {
        assert!(
            parse_args(&args(&["--surface", "corpcode"])).is_err(),
            "one of --plan/--execute is required"
        );
        assert!(
            parse_args(&args(&["--surface", "corpcode", "--plan", "--execute"])).is_err(),
            "--plan and --execute are mutually exclusive"
        );
    }

    #[test]
    fn unrecognized_arguments_are_rejected() {
        assert!(parse_args(&args(&["--surface", "corpcode", "--plan", "--bogus"])).is_err());
        assert!(parse_args(&args(&["positional-garbage"])).is_err());
    }

    #[test]
    fn duplicate_flags_are_rejected() {
        assert!(
            parse_args(&args(&[
                "--surface",
                "corpcode",
                "--surface",
                "list",
                "--plan"
            ]))
            .is_err()
        );
    }

    #[test]
    fn query_parameter_names_follow_list_optional_filters() {
        let parsed = parse_args(&args(&[
            "--surface",
            "list",
            "--corp-code",
            "00126380",
            "--bgn-de",
            "20260101",
            "--plan",
        ]))
        .expect("valid list args");
        let names = parsed.surface.query_parameter_names(&parsed);
        assert!(names.contains(&"corp_code"));
        assert!(names.contains(&"bgn_de"));
        assert!(!names.contains(&"end_de"));
        assert!(names.contains(&CRTFC_KEY_PARAM));
    }

    #[test]
    fn plan_output_includes_safe_fields_but_never_query_values() {
        const CORP_CODE_SENTINEL: &str = "00126380";
        const BGN_DE_SENTINEL: &str = "20260101";
        const END_DE_SENTINEL: &str = "20260201";
        let parsed = parse_args(&args(&[
            "--surface",
            "list",
            "--corp-code",
            CORP_CODE_SENTINEL,
            "--bgn-de",
            BGN_DE_SENTINEL,
            "--end-de",
            END_DE_SENTINEL,
            "--plan",
        ]))
        .expect("valid list plan args");

        let output = render_plan(
            &parsed,
            "/synthetic/raw-root",
            "vault://synthetic-entitlements/opendart-test-only.pdf",
        );

        for expected in [
            "surface: list",
            "path: /api/list.json",
            "endpoint: opendart.disclosure.list.v1",
            "raw_root: /synthetic/raw-root",
            "entitlement_reference: vault://synthetic-entitlements/opendart-test-only.pdf",
            "query_parameter_names: page_no, page_count, corp_code, bgn_de, end_de, crtfc_key",
            "no request was made (--plan)",
        ] {
            assert!(
                output.contains(expected),
                "plan output omitted {expected:?}"
            );
        }
        for value in [CORP_CODE_SENTINEL, BGN_DE_SENTINEL, END_DE_SENTINEL] {
            assert!(
                !output.contains(value),
                "plan output leaked query value {value:?}: {output}"
            );
        }
    }

    #[test]
    fn confirm_literal_matches_the_documented_value() {
        assert_eq!(CONFIRM_LITERAL, "I_UNDERSTAND_READ_ONLY_OPENDART_CALLS");
    }
}
