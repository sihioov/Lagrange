//! Provider-free candidate pin discovery for the bounded historical price beta.
//!
//! This command reads committed Raw manifest metadata only. It does not read
//! response bodies, reconcile manifests, write Raw, or approve a pin. The
//! explicit `kis-historical-price-beta-verify` command remains a separate
//! owner-reviewed second step.

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use market_data::{
    HistoricalPriceOnlyBetaPins, RawStore, discover_historical_price_only_beta_pins,
};

const USAGE: &str = "kis-historical-price-beta-pin-discover --raw-root <ABSOLUTE_DATA_ROOT>\n\nReads committed Raw manifest metadata only. It reads no response bodies, writes nothing, and does not approve a pin.";

#[derive(Debug)]
struct Args {
    raw_root: PathBuf,
}

#[derive(Debug)]
struct Failure(&'static str);

fn main() -> ExitCode {
    let argv = env::args().skip(1).collect::<Vec<_>>();
    if argv.len() == 1 && argv[0] == "--help" {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    match parse_args(&argv).and_then(run) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure(code)) => {
            eprintln!("KIS_HISTORICAL_PRICE_BETA_PIN_DISCOVER status=blocked reason={code}");
            ExitCode::FAILURE
        }
    }
}

fn parse_args(argv: &[String]) -> Result<Args, Failure> {
    let mut raw_root = None;
    let mut index = 0;
    while index < argv.len() {
        let key = argv[index].as_str();
        let value = argv.get(index + 1).ok_or(Failure("missing_option_value"))?;
        match key {
            "--raw-root" if raw_root.is_none() => raw_root = Some(PathBuf::from(value)),
            _ => return Err(Failure("unknown_or_repeated_option")),
        }
        index += 2;
    }
    let raw_root = raw_root.ok_or(Failure("missing_raw_root"))?;
    if !raw_root.is_absolute() {
        return Err(Failure("raw_root_must_be_absolute"));
    }
    Ok(Args { raw_root })
}

fn run(args: Args) -> Result<(), Failure> {
    let raw = RawStore::new(args.raw_root);
    let pins = discover_historical_price_only_beta_pins(&raw).map_err(map_discovery_error)?;
    print_success(&pins);
    Ok(())
}

fn print_success(pins: &HistoricalPriceOnlyBetaPins) {
    println!("{}", success_line(pins));
}

fn success_line(pins: &HistoricalPriceOnlyBetaPins) -> String {
    format!(
        "KIS_HISTORICAL_PRICE_BETA_PIN_DISCOVER status=candidate contract={} range={}..{} \
         source_batch_id={} source_manifest_sha256={} source_files={} \
         action_batch_id={} action_manifest_sha256={} action_files={} \
         body_bytes_read=false raw_writes=false approved=false review_required=true",
        pins.contract(),
        pins.range_start(),
        pins.range_end(),
        pins.source_batch_id(),
        pins.source_manifest_hash(),
        pins.source_file_count(),
        pins.action_batch_id(),
        pins.action_manifest_hash(),
        pins.action_file_count(),
    )
}

/// Only static reason codes cross the CLI boundary. Raw/provider error text
/// can contain paths or caller-controlled metadata and is never echoed.
fn map_discovery_error(error: market_data::RangeCanonicalError) -> Failure {
    use market_data::RangeCanonicalError as E;
    match error {
        E::HistoricalBetaContract { .. } | E::UnsupportedScope { .. } | E::UnsupportedMode => {
            Failure("historical_beta_contract")
        }
        E::MissingActionEvidence { .. } => Failure("action_evidence"),
        E::Store(_) | E::Serialization(_) => Failure("raw_store"),
        E::UnsupportedHistoricalSessionSchedule { .. }
        | E::MissingListingMasterEvidence { .. }
        | E::NonStrictPitNotApproved { .. }
        | E::UnsupportedAction { .. }
        | E::InvalidBarValue { .. }
        | E::MalformedStage4A { .. }
        | E::UnsupportedLegacyStage4A { .. }
        | E::InvalidLineage { .. }
        | E::InvalidSession { .. }
        | E::EvidencePackage { .. }
        | E::UnsafeEvidencePath { .. }
        | E::EvidenceArtifact { .. }
        | E::UpstreamManifest { .. }
        | E::ActionEvidence { .. }
        | E::IncompleteActionPagination { .. } => Failure("unexpected_evidence_surface"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_root(value: &str) -> Vec<String> {
        ["--raw-root", value]
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn only_absolute_raw_root_is_accepted() {
        assert_eq!(
            parse_args(&parse_root("/data")).unwrap().raw_root,
            PathBuf::from("/data")
        );
        assert!(parse_args(&parse_root("data")).is_err());
        assert!(parse_args(&[]).is_err());
    }

    #[test]
    fn repeated_and_unknown_arguments_are_rejected() {
        assert!(
            parse_args(&[
                "--raw-root".to_owned(),
                "/data".to_owned(),
                "--raw-root".to_owned(),
                "/other".to_owned(),
            ])
            .is_err()
        );
        assert!(parse_args(&["--other".to_owned(), "/data".to_owned()]).is_err());
    }

    #[test]
    fn failure_mapping_is_static() {
        let Failure(code) =
            map_discovery_error(market_data::RangeCanonicalError::HistoricalBetaContract {
                reason: "SENTINEL_PATH_QUERY_BODY_SECRET".to_owned(),
            });
        assert_eq!(code, "historical_beta_contract");
        assert!(!code.contains("SENTINEL"));

        let Failure(code) = map_discovery_error(market_data::RangeCanonicalError::Store(
            market_data::StoreError::UnsafePath {
                path: "SENTINEL_PATH".to_owned(),
                reason: "SENTINEL_PROVIDER_MESSAGE".to_owned(),
            },
        ));
        assert_eq!(code, "raw_store");
        assert!(!code.contains("SENTINEL"));
    }
}
