//! Provider-free verifier for the bounded owner-only historical price beta.
//!
//! This command never discovers or approves a convenient input. The operator
//! must supply two independently reviewed immutable manifest hashes: the
//! fixed Stage5 range batch and the exact seven-file KSD action batch. Success
//! prints only typed counts and pins, never Raw response bytes or provider
//! messages. It writes no Curated data and creates no database `READY` row.

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use domain::ContentHash;
use market_data::{
    HISTORICAL_PRICE_ONLY_BETA_CONTRACT, HistoricalPriceOnlyBetaInput, RawStore,
    verify_historical_price_only_beta_input,
};

const USAGE: &str = "kis-historical-price-beta-verify --raw-root <ABSOLUTE_DATA_ROOT> \\
    --stage5-manifest-sha256 <sha256:64hex> \\
    --action-manifest-sha256 <sha256:64hex>\n\nReads immutable Raw only. It performs no network, provider, Curated, DB, or approval write.";

#[derive(Debug)]
struct Args {
    raw_root: PathBuf,
    stage5_manifest_hash: ContentHash,
    action_manifest_hash: ContentHash,
}

#[derive(Debug)]
struct Failure(&'static str);

fn main() -> ExitCode {
    let argv = env::args().skip(1).collect::<Vec<_>>();
    if argv.len() == 1 && argv[0] == "--help" {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if argv.is_empty() {
        eprintln!("{USAGE}");
        return ExitCode::FAILURE;
    }
    match parse_args(&argv).and_then(run) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure(code)) => {
            eprintln!("KIS_HISTORICAL_PRICE_BETA_VERIFY status=blocked reason={code}");
            ExitCode::FAILURE
        }
    }
}

fn parse_args(argv: &[String]) -> Result<Args, Failure> {
    let mut raw_root = None;
    let mut stage5_manifest_hash = None;
    let mut action_manifest_hash = None;
    let mut index = 0;
    while index < argv.len() {
        let key = argv[index].as_str();
        let value = argv.get(index + 1).ok_or(Failure("missing_option_value"))?;
        match key {
            "--raw-root" if raw_root.is_none() => raw_root = Some(PathBuf::from(value)),
            "--stage5-manifest-sha256" if stage5_manifest_hash.is_none() => {
                stage5_manifest_hash = Some(
                    ContentHash::parse(value)
                        .map_err(|_| Failure("invalid_stage5_manifest_hash"))?,
                );
            }
            "--action-manifest-sha256" if action_manifest_hash.is_none() => {
                action_manifest_hash = Some(
                    ContentHash::parse(value)
                        .map_err(|_| Failure("invalid_action_manifest_hash"))?,
                );
            }
            _ => return Err(Failure("unknown_or_repeated_option")),
        }
        index += 2;
    }
    let raw_root = raw_root.ok_or(Failure("missing_raw_root"))?;
    if !raw_root.is_absolute() {
        return Err(Failure("raw_root_must_be_absolute"));
    }
    Ok(Args {
        raw_root,
        stage5_manifest_hash: stage5_manifest_hash
            .ok_or(Failure("missing_stage5_manifest_hash"))?,
        action_manifest_hash: action_manifest_hash
            .ok_or(Failure("missing_action_manifest_hash"))?,
    })
}

fn run(args: Args) -> Result<(), Failure> {
    let raw = RawStore::new(args.raw_root);
    let verified = verify_historical_price_only_beta_input(
        &raw,
        &args.stage5_manifest_hash,
        &args.action_manifest_hash,
    )
    .map_err(map_verify_error)?;
    print_success(&verified);
    Ok(())
}

fn print_success(verified: &HistoricalPriceOnlyBetaInput) {
    println!(
        "KIS_HISTORICAL_PRICE_BETA_VERIFY status=ok contract={} range={}..{} \
         source_batch_id={} source_manifest_sha256={} source_files={} \
         action_batch_id={} action_manifest_sha256={} action_files={} \
         sessions={} instruments=11 bars={} actions={} audience=OWNER_ONLY \
         vendor_snapshot=true strict_pit=false intended_capability=PRICE_RETURN_ONLY \
         materialized=false bonus_adjustment=NOT_APPLIED ready=false",
        HISTORICAL_PRICE_ONLY_BETA_CONTRACT,
        verified.range_start(),
        verified.range_end(),
        verified.source_batch_id(),
        verified.source_manifest_hash(),
        verified.source_files().len(),
        verified.action_batch_id(),
        verified.action_manifest_hash(),
        verified.action_file_count(),
        verified.sessions().len(),
        verified.bars().len(),
        verified.actions().len(),
    );
}

/// Only static reason codes cross the CLI boundary. Raw/provider error text
/// can contain caller-controlled metadata and is never echoed.
fn map_verify_error(error: market_data::RangeCanonicalError) -> Failure {
    use market_data::RangeCanonicalError as E;
    match error {
        E::HistoricalBetaContract { .. } => Failure("historical_beta_contract"),
        E::MissingActionEvidence { .. }
        | E::ActionEvidence { .. }
        | E::IncompleteActionPagination { .. } => Failure("action_evidence"),
        E::UnsupportedAction { .. } => Failure("unsupported_action"),
        E::InvalidBarValue { .. } => Failure("invalid_bar_value"),
        E::MalformedStage4A { .. }
        | E::UnsupportedLegacyStage4A { .. }
        | E::InvalidLineage { .. }
        | E::InvalidSession { .. }
        | E::UpstreamManifest { .. } => Failure("stage5_evidence"),
        E::NonStrictPitNotApproved { .. } => Failure("pit_contract"),
        E::UnsupportedScope { .. } | E::UnsupportedMode => Failure("unsupported_scope"),
        E::Store(_) | E::Serialization(_) => Failure("raw_store"),
        E::EvidencePackage { .. }
        | E::UnsafeEvidencePath { .. }
        | E::EvidenceArtifact { .. }
        | E::UnsupportedHistoricalSessionSchedule { .. }
        | E::MissingListingMasterEvidence { .. } => Failure("unexpected_evidence_surface"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(byte: char) -> String {
        format!("sha256:{}", byte.to_string().repeat(64))
    }

    #[test]
    fn exact_pins_and_absolute_root_are_required() {
        let args = parse_args(&[
            "--raw-root".to_owned(),
            "/data".to_owned(),
            "--stage5-manifest-sha256".to_owned(),
            hash('a'),
            "--action-manifest-sha256".to_owned(),
            hash('b'),
        ])
        .unwrap();
        assert_eq!(args.raw_root, PathBuf::from("/data"));

        let relative = [
            "--raw-root".to_owned(),
            "data".to_owned(),
            "--stage5-manifest-sha256".to_owned(),
            hash('a'),
            "--action-manifest-sha256".to_owned(),
            hash('b'),
        ];
        assert!(parse_args(&relative).is_err());
    }

    #[test]
    fn malformed_missing_and_repeated_arguments_are_rejected() {
        assert!(parse_args(&[]).is_err());
        assert!(parse_args(&["--raw-root".to_owned(), "/data".to_owned()]).is_err());
        let repeated = [
            "--raw-root".to_owned(),
            "/data".to_owned(),
            "--raw-root".to_owned(),
            "/other".to_owned(),
            "--stage5-manifest-sha256".to_owned(),
            "bad".to_owned(),
        ];
        assert!(parse_args(&repeated).is_err());
    }
}
