//! First-party, provider-free Stage4B-0 evidence-package assembler CLI.
//!
//! This binary is intentionally a very small adapter around
//! `market_data::write_evidence_package`. It reads an already committed
//! Stage4A normalized batch and caller-supplied schedule/listing/PIT-policy
//! files, and writes an evidence package (`manifest.json` plus the three
//! artifact files) to `--out`. It does not launch a browser, use a network
//! client, fabricate schedule/listing/action evidence, or approve its own
//! output: the printed `manifest_sha256` still requires operator review and
//! a commit to `configs/evidence/kis-range-canonical-approved-manifests.json`
//! before the bridge will ever load the package it writes.

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use domain::BatchId;
use market_data::{
    MARKET_KR, ManifestEntry, PROVIDER_KIS_DAILY_RANGE_NORMALIZED, RawStore, write_evidence_package,
};

const USAGE: &str = "kis-range-evidence-package --raw-root <DATA_ROOT> \\\n    --normalized-batch-id <UUID> --schedule <FILE> --listing <FILE> \\\n    --pit-policy <FILE> --out <DIR>\n\nThe raw root is the data/ directory; RawStore places evidence below raw/.\nschedule/listing/pit-policy are caller-supplied evidence files: this tool\nnever fabricates their contents, only validates and pins them.";

#[derive(Debug)]
struct Args {
    raw_root: PathBuf,
    normalized_batch_id: BatchId,
    schedule: PathBuf,
    listing: PathBuf,
    pit_policy: PathBuf,
    out: PathBuf,
}

#[derive(Debug)]
struct Failure(&'static str);

fn main() -> ExitCode {
    let argv: Vec<String> = env::args().skip(1).collect();
    if argv.len() == 1 && argv[0] == "--help" {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    match parse_args(&argv).and_then(run) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure(code)) => {
            eprintln!("KIS_RANGE_EVIDENCE_PACKAGE status=incomplete reason={code}");
            ExitCode::FAILURE
        }
    }
}

fn parse_args(argv: &[String]) -> Result<Args, Failure> {
    let mut raw_root = None;
    let mut normalized_batch_id = None;
    let mut schedule = None;
    let mut listing = None;
    let mut pit_policy = None;
    let mut out = None;
    let mut index = 0;

    while index < argv.len() {
        let key = argv[index].as_str();
        let value = argv.get(index + 1).ok_or(Failure("missing_option_value"))?;
        if !key.starts_with("--") {
            return Err(Failure("unexpected_argument"));
        }
        match key {
            "--raw-root" if raw_root.is_none() => raw_root = Some(PathBuf::from(value)),
            "--normalized-batch-id" if normalized_batch_id.is_none() => {
                normalized_batch_id = Some(value.parse().map_err(|_| Failure("invalid_batch_id"))?)
            }
            "--schedule" if schedule.is_none() => schedule = Some(PathBuf::from(value)),
            "--listing" if listing.is_none() => listing = Some(PathBuf::from(value)),
            "--pit-policy" if pit_policy.is_none() => pit_policy = Some(PathBuf::from(value)),
            "--out" if out.is_none() => out = Some(PathBuf::from(value)),
            _ => return Err(Failure("unknown_or_repeated_option")),
        }
        index += 2;
    }

    let raw_root = raw_root.ok_or(Failure("missing_raw_root"))?;
    if !raw_root.is_absolute() {
        return Err(Failure("raw_root_must_be_absolute"));
    }
    let out = out.ok_or(Failure("missing_out"))?;
    if !out.is_absolute() {
        return Err(Failure("out_must_be_absolute"));
    }
    Ok(Args {
        raw_root,
        normalized_batch_id: normalized_batch_id.ok_or(Failure("missing_normalized_batch_id"))?,
        schedule: schedule.ok_or(Failure("missing_schedule"))?,
        listing: listing.ok_or(Failure("missing_listing"))?,
        pit_policy: pit_policy.ok_or(Failure("missing_pit_policy"))?,
        out,
    })
}

fn find_normalized_entry(
    raw: &RawStore,
    normalized_batch_id: BatchId,
) -> Result<ManifestEntry, Failure> {
    let entries = raw
        .read_manifest(PROVIDER_KIS_DAILY_RANGE_NORMALIZED, MARKET_KR)
        .map_err(|_| Failure("normalized_manifest_read_failed"))?;
    let mut matches = entries
        .into_iter()
        .filter(|entry| entry.batch_id == normalized_batch_id);
    let entry = matches
        .next()
        .ok_or(Failure("normalized_batch_not_found"))?;
    if matches.next().is_some() {
        return Err(Failure("normalized_batch_not_unique"));
    }
    Ok(entry)
}

fn run(args: Args) -> Result<(), Failure> {
    let raw = RawStore::new(&args.raw_root);
    let normalized_entry = find_normalized_entry(&raw, args.normalized_batch_id)?;
    let schedule_bytes =
        std::fs::read(&args.schedule).map_err(|_| Failure("schedule_read_failed"))?;
    let listing_bytes = std::fs::read(&args.listing).map_err(|_| Failure("listing_read_failed"))?;
    let pit_policy_bytes =
        std::fs::read(&args.pit_policy).map_err(|_| Failure("pit_policy_read_failed"))?;

    let manifest_sha256 = write_evidence_package(
        &raw,
        &normalized_entry,
        &schedule_bytes,
        &listing_bytes,
        &pit_policy_bytes,
        &args.out,
    )
    .map_err(map_write_error)?;

    println!(
        "KIS_RANGE_EVIDENCE_PACKAGE status=ok normalized_batch_id={} session_date={} manifest_sha256={manifest_sha256}",
        normalized_entry.batch_id, normalized_entry.date
    );
    Ok(())
}

/// Only a static reason code ever reaches stdout/stderr — never a provider
/// message, response body, or the error's free-form `reason` text, which may
/// echo caller-supplied file contents.
fn map_write_error(error: market_data::RangeCanonicalError) -> Failure {
    use market_data::RangeCanonicalError as E;
    match error {
        E::UnsupportedScope { .. } => Failure("unsupported_scope"),
        E::UnsupportedMode => Failure("unsupported_mode"),
        E::UnsupportedHistoricalSessionSchedule { .. } => Failure("invalid_schedule_evidence"),
        E::MissingListingMasterEvidence { .. } => Failure("invalid_listing_evidence"),
        E::MissingActionEvidence { .. } => Failure("invalid_action_evidence"),
        E::NonStrictPitNotApproved { .. } => Failure("invalid_pit_policy_evidence"),
        E::UnsupportedAction { .. } => Failure("unsupported_action"),
        E::InvalidBarValue { .. } => Failure("invalid_bar_value"),
        E::MalformedStage4A { .. } => Failure("malformed_stage4a"),
        E::UnsupportedLegacyStage4A { .. } => Failure("unsupported_legacy_stage4a"),
        E::InvalidLineage { .. } => Failure("invalid_lineage"),
        E::InvalidSession { .. } => Failure("invalid_session"),
        E::Store(_) => Failure("raw_store_error"),
        E::Serialization(_) => Failure("serialization_error"),
        E::EvidencePackage { .. } => Failure("evidence_package_error"),
        E::UnsafeEvidencePath { .. } => Failure("unsafe_evidence_path"),
        E::EvidenceArtifact { .. } => Failure("evidence_artifact_error"),
        E::UpstreamManifest { .. } => Failure("upstream_manifest_error"),
        E::ActionEvidence { .. } => Failure("action_evidence_error"),
        E::IncompleteActionPagination { .. } => Failure("incomplete_action_pagination"),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_args;

    #[test]
    fn help_flag_is_recognized_by_caller_before_parsing() {
        // main() intercepts a lone --help before parse_args runs; parse_args
        // itself simply rejects an unknown/incomplete flag set.
        let argv = ["--help".to_owned()];
        assert!(parse_args(&argv).is_err());
    }

    #[test]
    fn missing_required_options_are_rejected() {
        let argv = ["--raw-root".to_owned(), "/data".to_owned()];
        assert!(parse_args(&argv).is_err());
    }

    #[test]
    fn relative_roots_are_rejected() {
        let argv = [
            "--raw-root".to_owned(),
            "data".to_owned(),
            "--normalized-batch-id".to_owned(),
            "00000000-0000-0000-0000-000000000000".to_owned(),
            "--schedule".to_owned(),
            "/schedule.json".to_owned(),
            "--listing".to_owned(),
            "/listing.json".to_owned(),
            "--pit-policy".to_owned(),
            "/pit.json".to_owned(),
            "--out".to_owned(),
            "/out".to_owned(),
        ];
        assert!(parse_args(&argv).is_err());
    }
}
