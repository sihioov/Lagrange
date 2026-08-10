//! Manual KRX Raw ingestion and publication CLI.
//!
//! `ingest-krx` remains the Raw-only QA path. `ingest-and-publish-krx` uses the
//! same provider/request construction and publishes through `DATABASE_URL`.

use std::process::ExitCode;
use std::str::FromStr;
use std::time::Duration;

use collectors::{
    FailureClass, PipelineError, PipelineStage, PostgresPublicationSink, PublishOutcome,
    ingest_and_publish, provider_failure_class,
};
use domain::{BatchId, TradingDate, UtcTimestamp};
use market_data::EodProvider;
use market_data::contract::{MARKET_KR, PROVIDER_KRX};
use market_data::ingest::{IngestRequest, ingest_bundle};
use market_data::provider::{CredentialRef, KrxProvider, RecordedBundle};
use market_data::redact::{Redactor, SECRET_KEYS};
use market_data::storage::{ManifestEntry, RawStore};
use serde::Serialize;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

const USAGE: &str = "\
collectors ingest-krx --root <dir> --date <YYYY-MM-DD> --mode <synthetic|credentialed> [options]
collectors ingest-and-publish-krx --root <dir> --date <YYYY-MM-DD> --mode <synthetic|credentialed> [options]

options:
  --bundle <dir>        recorded synthetic bundle (default: tests/fixtures/kr-etf/contract)
  --now <RFC3339>       retrieval clock (default: current UTC)
  --entitlement-ref <s> contract reference recorded on the manifest row (QA override)
  --help                this help

environment:
  DATABASE_URL          required only by ingest-and-publish-krx; PostgreSQL connection URL
";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandKind {
    Raw,
    Publish,
}

#[derive(Serialize)]
struct LegacyCliError {
    status: &'static str,
    error_code: String,
    message: String,
}

#[derive(Serialize)]
struct PublishCliError {
    status: &'static str,
    error_code: &'static str,
    class: &'static str,
    batch_id: Option<String>,
    message: String,
}

#[derive(Serialize)]
struct CliOutcome<'a> {
    status: &'static str,
    batch_id: String,
    provider: &'a str,
    market: &'a str,
    date: String,
    mode: String,
    entitlement_reference: Option<&'a str>,
    files: Vec<FileSummary>,
    manifest: String,
}

#[derive(Serialize)]
struct PublishedCliOutcome<'a> {
    status: &'static str,
    batch_id: String,
    provider: &'a str,
    market: &'a str,
    date: String,
    mode: String,
    entitlement_reference: Option<&'a str>,
    files: Vec<FileSummary>,
    manifest: String,
    published: &'static str,
}

#[derive(Serialize)]
struct FileSummary {
    kind: String,
    file_name: String,
    content_hash: String,
    size_bytes: u64,
}

enum CliFailure {
    Usage(String),
    Legacy(String),
    Publish {
        error_code: &'static str,
        class: FailureClass,
        batch_id: Option<BatchId>,
        message: String,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut redactor = Redactor::new();
    for key in SECRET_KEYS {
        if let Ok(value) = std::env::var(key) {
            redactor.add_secret(value);
        }
    }
    if let Ok(value) = std::env::var("DATABASE_URL") {
        redactor.add_secret(value);
    }
    let log = |line: String| eprintln!("{}", redactor.redact(&line));

    match run(&args, &log).await {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(CliFailure::Usage(message)) => {
            eprint!("{message}");
            ExitCode::from(1)
        }
        Err(CliFailure::Legacy(message)) => {
            let error = LegacyCliError {
                status: "error",
                error_code: "INGEST_FAILED".to_owned(),
                message: redactor.redact(&message),
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&error).unwrap_or_else(|_| {
                    "{\"status\":\"error\",\"error_code\":\"INGEST_FAILED\"}".to_owned()
                })
            );
            ExitCode::from(2)
        }
        Err(CliFailure::Publish {
            error_code,
            class,
            batch_id,
            message,
        }) => {
            let error = PublishCliError {
                status: "error",
                error_code,
                class: class.as_str(),
                batch_id: batch_id.map(|id| id.to_string()),
                message: redactor.redact(&message),
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&error).unwrap_or_else(|_| {
                    "{\"status\":\"error\",\"error_code\":\"PUBLISH_FAILED\"}".to_owned()
                })
            );
            ExitCode::from(2)
        }
    }
}

type Log<'a> = dyn Fn(String) + 'a;

struct ParsedArgs {
    command: CommandKind,
    root: String,
    date: TradingDate,
    mode: String,
    bundle: String,
    now: UtcTimestamp,
    entitlement_reference: Option<String>,
}

fn parse_args(args: &[String]) -> Result<ParsedArgs, CliFailure> {
    let (command, args) = match args.first().map(String::as_str) {
        Some("ingest-krx") => (CommandKind::Raw, &args[1..]),
        Some("ingest-and-publish-krx") => (CommandKind::Publish, &args[1..]),
        _ => return Err(CliFailure::Usage(USAGE.to_owned())),
    };
    if args.is_empty() || args.iter().any(|arg| arg == "--help" || arg == "-h") {
        return Err(CliFailure::Usage(USAGE.to_owned()));
    }
    let mut root = "data".to_owned();
    let mut date = None;
    let mut mode = String::new();
    let mut bundle = "tests/fixtures/kr-etf/contract".to_owned();
    let mut now = None;
    let mut entitlement_reference = None;
    let mut iterator = args.iter();
    while let Some(flag) = iterator.next() {
        let take = |iterator: &mut std::slice::Iter<'_, String>, flag: &str| {
            iterator
                .next()
                .cloned()
                .ok_or_else(|| CliFailure::Usage(format!("missing value for {flag}\n{USAGE}")))
        };
        match flag.as_str() {
            "--root" => root = take(&mut iterator, flag)?,
            "--date" => {
                let value = take(&mut iterator, flag)?;
                date = Some(TradingDate::parse(&value).map_err(|error| {
                    CliFailure::Usage(format!("invalid --date: {error}\n{USAGE}"))
                })?);
            }
            "--mode" => mode = take(&mut iterator, flag)?,
            "--bundle" => bundle = take(&mut iterator, flag)?,
            "--now" => {
                let value = take(&mut iterator, flag)?;
                now = Some(UtcTimestamp::parse_rfc3339(&value).map_err(|error| {
                    CliFailure::Usage(format!("invalid --now: {error}\n{USAGE}"))
                })?);
            }
            "--entitlement-ref" => entitlement_reference = Some(take(&mut iterator, flag)?),
            other => {
                return Err(CliFailure::Usage(format!(
                    "unknown flag {other:?}\n{USAGE}"
                )));
            }
        }
    }
    let date = date.ok_or_else(|| CliFailure::Usage(format!("--date is required\n{USAGE}")))?;
    Ok(ParsedArgs {
        command,
        root,
        date,
        mode,
        bundle,
        now: now.unwrap_or_else(UtcTimestamp::now),
        entitlement_reference,
    })
}

fn provider(args: &ParsedArgs, log: &Log<'_>) -> Result<KrxProvider, CliFailure> {
    match args.mode.as_str() {
        "synthetic" => {
            let recorded =
                RecordedBundle::open(&args.bundle).map_err(|error| match args.command {
                    CommandKind::Raw => CliFailure::Legacy(format!(
                        "recorded bundle {:?} unreadable: {error}",
                        args.bundle
                    )),
                    CommandKind::Publish => CliFailure::Publish {
                        error_code: "PROVIDER_UNAVAILABLE",
                        class: provider_failure_class(&error),
                        batch_id: None,
                        message: format!("recorded bundle {:?} unreadable: {error}", args.bundle),
                    },
                })?;
            Ok(KrxProvider::synthetic(recorded))
        }
        "credentialed" => {
            log("collector: Owner-only credentialed KRX mode selected".to_owned());
            Ok(KrxProvider::credentialed(CredentialRef::new(
                "env:KRX_CREDENTIAL_REF",
            )))
        }
        other => Err(CliFailure::Usage(format!(
            "unknown --mode {other:?} (expected synthetic|credentialed)\n{USAGE}"
        ))),
    }
}

async fn run(args: &[String], log: &Log<'_>) -> Result<String, CliFailure> {
    let args = parse_args(args)?;
    let provider = provider(&args, log)?;
    let store = RawStore::new(&args.root);
    let request = IngestRequest::new(MARKET_KR.to_owned(), args.date, args.now);
    log(format!(
        "collector: ingesting provider={} market={} date={} mode={}",
        provider.provider_id(),
        MARKET_KR,
        args.date.to_iso(),
        provider.fetch_mode()
    ));

    match args.command {
        CommandKind::Raw => run_raw(&store, &provider, &request, &args, log),
        CommandKind::Publish => run_publish(&store, &provider, &request, &args, log).await,
    }
}

fn run_raw(
    store: &RawStore,
    provider: &KrxProvider,
    request: &IngestRequest,
    args: &ParsedArgs,
    log: &Log<'_>,
) -> Result<String, CliFailure> {
    let outcome = ingest_bundle(
        store,
        provider,
        request,
        args.entitlement_reference.as_deref(),
    )
    .map_err(|error| CliFailure::Legacy(format!("ingest failed: {error}")))?;
    log_stored(log, &outcome.entry);
    let out = CliOutcome {
        status: "ok",
        batch_id: outcome.batch_id.to_string(),
        provider: &outcome.entry.provider,
        market: &outcome.entry.market,
        date: outcome.entry.date.to_iso(),
        mode: outcome.entry.mode.to_string(),
        entitlement_reference: outcome.entry.entitlement_reference.as_deref(),
        files: summarize_files(&outcome.entry),
        manifest: manifest_path(store),
    };
    serde_json::to_string_pretty(&out)
        .map_err(|error| CliFailure::Legacy(format!("serialize outcome: {error}")))
}

async fn run_publish(
    store: &RawStore,
    provider: &KrxProvider,
    request: &IngestRequest,
    args: &ParsedArgs,
    log: &Log<'_>,
) -> Result<String, CliFailure> {
    let database_url = std::env::var("DATABASE_URL").map_err(|_| CliFailure::Publish {
        error_code: "DATABASE_URL_UNAVAILABLE",
        class: FailureClass::Permanent,
        batch_id: None,
        message: "DATABASE_URL is required for ingest-and-publish-krx".to_owned(),
    })?;
    let options = PgConnectOptions::from_str(&database_url).map_err(|_| CliFailure::Publish {
        error_code: "DATABASE_URL_INVALID",
        class: FailureClass::Permanent,
        batch_id: None,
        message: "DATABASE_URL is not a valid PostgreSQL connection URL".to_owned(),
    })?;
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(10))
        .connect_lazy_with(options);
    let sink = PostgresPublicationSink::new(pool);
    let outcome = ingest_and_publish(
        store,
        provider,
        request,
        args.entitlement_reference.as_deref(),
        &sink,
    )
    .await
    .map_err(pipeline_cli_failure)?;
    log_stored(log, &outcome.manifest);
    let published = match outcome.published {
        PublishOutcome::Published => "published",
        PublishOutcome::AlreadyPublished => "already_published",
    };
    let out = PublishedCliOutcome {
        status: "ok",
        batch_id: outcome.manifest.batch_id.to_string(),
        provider: &outcome.manifest.provider,
        market: &outcome.manifest.market,
        date: outcome.manifest.date.to_iso(),
        mode: outcome.manifest.mode.to_string(),
        entitlement_reference: outcome.manifest.entitlement_reference.as_deref(),
        files: summarize_files(&outcome.manifest),
        manifest: manifest_path(store),
        published,
    };
    serde_json::to_string_pretty(&out).map_err(|error| CliFailure::Publish {
        error_code: "OUTCOME_SERIALIZATION_FAILED",
        class: FailureClass::Permanent,
        batch_id: Some(outcome.manifest.batch_id),
        message: format!("serialize outcome: {error}"),
    })
}

fn pipeline_cli_failure(error: PipelineError) -> CliFailure {
    let error_code = match error.stage() {
        PipelineStage::Ingest => "INGEST_FAILED",
        PipelineStage::ReadManifest => "MANIFEST_READ_FAILED",
        PipelineStage::PublicationState => "PUBLICATION_STATE_FAILED",
        PipelineStage::VerifyRaw => "RAW_VERIFICATION_FAILED",
        PipelineStage::Publish => "PUBLISH_FAILED",
    };
    CliFailure::Publish {
        error_code,
        class: error.failure_class(),
        batch_id: error.batch_id(),
        message: error.to_string(),
    }
}

fn log_stored(log: &Log<'_>, manifest: &ManifestEntry) {
    log(format!(
        "collector: batch={} stored {} files under date={}",
        manifest.batch_id,
        manifest.files.len(),
        manifest.date.to_iso()
    ));
}

fn summarize_files(entry: &ManifestEntry) -> Vec<FileSummary> {
    entry
        .files
        .iter()
        .map(|file| FileSummary {
            kind: file.kind.to_string(),
            file_name: file.file_name.clone(),
            content_hash: file.content_hash.to_string(),
            size_bytes: file.size_bytes,
        })
        .collect()
}

fn manifest_path(store: &RawStore) -> String {
    store
        .manifest_path(PROVIDER_KRX, MARKET_KR)
        .display()
        .to_string()
}
