//! KRX raw-ingestion collector CLI (Todo 8 manual QA channel).
//!
//! ```
//! collectors ingest-krx --root data --date 2020-01-31 --mode synthetic \
//!     --bundle tests/fixtures/kr-etf/contract
//! ```
//!
//! - `--mode synthetic` plays back recorded synthetic contract fixtures (CI).
//! - `--mode credentialed` is the Owner-only licensed mode; it fails typed with
//!   `CredentialsUnavailable` unless real KRX credentials exist (they do not).
//! - Every stderr log line is routed through the redactor; stdout carries JSON
//!   only. Exit codes: 0 success, 1 usage, 2 typed ingest failure.

use std::process::ExitCode;

use domain::{TradingDate, UtcTimestamp};
use market_data::EodProvider;
use market_data::contract::{MARKET_KR, PROVIDER_KRX};
use market_data::ingest::{IngestRequest, ingest_bundle};
use market_data::provider::{CredentialRef, KrxProvider, RecordedBundle};
use market_data::redact::{Redactor, SECRET_KEYS};
use market_data::storage::RawStore;
use serde::Serialize;

const USAGE: &str = "\
collectors ingest-krx --root <dir> --date <YYYY-MM-DD> --mode <synthetic|credentialed> [options]

options:
  --bundle <dir>        recorded synthetic bundle (default: tests/fixtures/kr-etf/contract)
  --now <RFC3339>       retrieval clock (default: current UTC)
  --entitlement-ref <s> contract reference recorded on the manifest row (QA override)
  --help                this help
";

#[derive(Serialize)]
struct CliError {
    status: &'static str,
    error_code: String,
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
struct FileSummary {
    kind: String,
    file_name: String,
    content_hash: String,
    size_bytes: u64,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut redactor = Redactor::new();
    for key in SECRET_KEYS {
        if let Ok(value) = std::env::var(key) {
            redactor.add_secret(value);
        }
    }
    let log = |line: String| eprintln!("{}", redactor.redact(&line));

    let outcome = run(&args, &log);
    match outcome {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err((exit, msg)) => {
            if exit == 1 {
                eprint!("{msg}");
            } else {
                let err = CliError {
                    status: "error",
                    error_code: "INGEST_FAILED".to_owned(),
                    message: redactor.redact(&msg),
                };
                println!("{}", serde_json::to_string_pretty(&err).unwrap_or(msg));
            }
            ExitCode::from(exit)
        }
    }
}

type Log<'a> = dyn Fn(String) + 'a;

fn run(args: &[String], log: &Log<'_>) -> Result<String, (u8, String)> {
    let args = match args.first().map(String::as_str) {
        Some("ingest-krx") => &args[1..],
        _ => return Err((1, USAGE.to_owned())),
    };
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        return Err((1, USAGE.to_owned()));
    }
    let mut root = "data".to_owned();
    let mut date: Option<TradingDate> = None;
    let mut mode = String::new();
    let mut bundle = "tests/fixtures/kr-etf/contract".to_owned();
    let mut now: Option<UtcTimestamp> = None;
    let mut entitlement_ref: Option<String> = None;

    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let take =
            |it: &mut std::slice::Iter<'_, String>, flag: &str| -> Result<String, (u8, String)> {
                it.next()
                    .cloned()
                    .ok_or_else(|| (1, format!("missing value for {flag}\n{USAGE}")))
            };
        match flag.as_str() {
            "--root" => root = take(&mut it, flag)?,
            "--date" => {
                let v = take(&mut it, flag)?;
                date = Some(
                    TradingDate::parse(&v)
                        .map_err(|e| (1, format!("invalid --date: {e}\n{USAGE}")))?,
                );
            }
            "--mode" => mode = take(&mut it, flag)?,
            "--bundle" => bundle = take(&mut it, flag)?,
            "--now" => {
                let v = take(&mut it, flag)?;
                now = Some(
                    UtcTimestamp::parse_rfc3339(&v)
                        .map_err(|e| (1, format!("invalid --now: {e}\n{USAGE}")))?,
                );
            }
            "--entitlement-ref" => entitlement_ref = Some(take(&mut it, flag)?),
            other => return Err((1, format!("unknown flag {other:?}\n{USAGE}"))),
        }
    }

    let date = date.ok_or_else(|| (1, format!("--date is required\n{USAGE}")))?;
    let provider = match mode.as_str() {
        "synthetic" => {
            let recorded = RecordedBundle::open(&bundle)
                .map_err(|e| (2, format!("recorded bundle {bundle:?} unreadable: {e}")))?;
            KrxProvider::synthetic(recorded)
        }
        "credentialed" => {
            log("collector: Owner-only credentialed KRX mode selected".to_owned());
            KrxProvider::credentialed(CredentialRef::new("env:KRX_CREDENTIAL_REF"))
        }
        other => {
            return Err((
                1,
                format!("unknown --mode {other:?} (expected synthetic|credentialed)\n{USAGE}"),
            ));
        }
    };

    let store = RawStore::new(&root);
    let request = IngestRequest::new(
        MARKET_KR.to_owned(),
        date,
        now.unwrap_or_else(UtcTimestamp::now),
    );
    log(format!(
        "collector: ingesting provider={} market={} date={} mode={}",
        provider.provider_id(),
        MARKET_KR,
        date.to_iso(),
        provider.fetch_mode()
    ));

    // QA override: record an explicit contract reference on the manifest row.
    // Production wiring supplies it from the EntitlementService snapshot.
    let outcome = ingest_bundle(&store, &provider, &request, entitlement_ref.as_deref())
        .map_err(|e| (2, format!("ingest failed: {e}")))?;

    log(format!(
        "collector: batch={} stored {} files under date={}",
        outcome.batch_id,
        outcome.entry.files.len(),
        outcome.entry.date.to_iso()
    ));

    let files: Vec<FileSummary> = outcome
        .entry
        .files
        .iter()
        .map(|f| FileSummary {
            kind: f.kind.to_string(),
            file_name: f.file_name.clone(),
            content_hash: f.content_hash.to_string(),
            size_bytes: f.size_bytes,
        })
        .collect();
    let out = CliOutcome {
        status: "ok",
        batch_id: outcome.batch_id.to_string(),
        provider: &outcome.entry.provider,
        market: &outcome.entry.market,
        date: outcome.entry.date.to_iso(),
        mode: outcome.entry.mode.to_string(),
        entitlement_reference: outcome.entry.entitlement_reference.as_deref(),
        files,
        manifest: store
            .manifest_path(PROVIDER_KRX, MARKET_KR)
            .display()
            .to_string(),
    };
    serde_json::to_string_pretty(&out).map_err(|e| (2, format!("serialize outcome: {e}")))
}
