//! `backtest-manifest` — publishes a worker-produced `manifest.json` into the
//! T3 backtest-result tables (plan Todo 20; the worker's Rust-facing path).
//!
//! Usage:
//!   DATABASE_URL=<worker-or-superuser url> \
//!     backtest-manifest --manifest <run_dir>/manifest.json [--url <url>]
//!
//! Writes the manifest in one short transaction (idempotent by run id) and
//! prints a machine-readable report; exits 0 only when the write succeeded.

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use result_model::manifest::{BacktestManifest, ManifestWriter, WriteReport};

fn main() -> ExitCode {
    let mut manifest_arg: Option<PathBuf> = None;
    let mut url_arg: Option<String> = None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--manifest" => manifest_arg = args.next().map(PathBuf::from),
            "--url" => url_arg = args.next(),
            "--help" | "-h" => {
                println!("usage: backtest-manifest --manifest <path> [--url <url>]");
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("backtest-manifest: unknown argument {other:?}");
                return ExitCode::FAILURE;
            }
        }
    }
    let manifest_path = match manifest_arg {
        Some(path) => path,
        None => {
            eprintln!("backtest-manifest: --manifest <path> is required");
            return ExitCode::FAILURE;
        }
    };
    let url = match url_arg.or_else(|| env::var("DATABASE_URL").ok().filter(|s| !s.is_empty())) {
        Some(url) => url,
        None => {
            eprintln!("backtest-manifest: DATABASE_URL is not set");
            return ExitCode::FAILURE;
        }
    };

    let manifest: BacktestManifest = match std::fs::read_to_string(&manifest_path) {
        Ok(text) => match serde_json::from_str(&text) {
            Ok(manifest) => manifest,
            Err(error) => {
                eprintln!(
                    "backtest-manifest: invalid manifest {}: {error}",
                    manifest_path.display()
                );
                return ExitCode::FAILURE;
            }
        },
        Err(error) => {
            eprintln!(
                "backtest-manifest: cannot read {}: {error}",
                manifest_path.display()
            );
            return ExitCode::FAILURE;
        }
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime builds");
    let report: WriteReport = match runtime.block_on(async {
        let writer = ManifestWriter::connect(&url).await?;
        writer.write(&manifest).await
    }) {
        Ok(report) => report,
        Err(error) => {
            eprintln!("backtest-manifest: write failed: {error}");
            return ExitCode::FAILURE;
        }
    };
    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "inserted": report.inserted,
            "run_id": report.run_id.to_string(),
            "artifacts": report.artifacts,
        }))
        .expect("report serializes")
    );
    ExitCode::SUCCESS
}
