//! Provider-free owner-only approval check for the sealed historical-price beta.

use std::{env, path::PathBuf, process::ExitCode};

use market_data::approve_historical_price_only_artifact;

const USAGE: &str =
    "kis-historical-price-beta-approval-check check --artifact-root <ABS_ARTIFACT_ROOT>";
const CHECK_OPTIONS: [&str; 1] = ["--artifact-root"];

struct CheckArgs {
    artifact_root: PathBuf,
}
enum ParseOutcome {
    Help,
    Check(CheckArgs),
}
#[derive(Clone, Copy)]
struct Failure {
    reason: &'static str,
}

fn main() -> ExitCode {
    let argv = env::args().skip(1).collect::<Vec<_>>();
    match parse_args(&argv) {
        Ok(ParseOutcome::Help) => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Ok(ParseOutcome::Check(args)) => match execute_check(&args) {
            Ok(line) => {
                println!("{line}");
                ExitCode::SUCCESS
            }
            Err(failure) => {
                eprintln!("{}", failure_line(failure));
                ExitCode::FAILURE
            }
        },
        Err(failure) => {
            eprintln!("{}", failure_line(failure));
            ExitCode::FAILURE
        }
    }
}

fn parse_args(argv: &[String]) -> Result<ParseOutcome, Failure> {
    if argv.len() == 1 && argv[0] == "--help" {
        return Ok(ParseOutcome::Help);
    }
    if argv.first().map(String::as_str) != Some("check") {
        return Err(Failure {
            reason: if argv.is_empty() {
                "missing_command"
            } else {
                "unknown_command"
            },
        });
    }
    if argv.len() != 3 || argv[1] != CHECK_OPTIONS[0] {
        return Err(Failure {
            reason: if argv.last().is_some_and(|value| value.starts_with("--")) {
                "missing_option_value"
            } else if argv.len() < 3 {
                "missing_option"
            } else {
                "unknown_or_repeated_option"
            },
        });
    }
    let artifact_root = PathBuf::from(&argv[2]);
    if !artifact_root.is_absolute() {
        return Err(Failure {
            reason: "artifact_root_must_be_absolute",
        });
    }
    Ok(ParseOutcome::Check(CheckArgs { artifact_root }))
}

fn execute_check(args: &CheckArgs) -> Result<String, Failure> {
    let approved =
        approve_historical_price_only_artifact(&args.artifact_root).map_err(|_| Failure {
            reason: "artifact_not_approved",
        })?;
    if approved.bars().len() != 26972 {
        return Err(Failure {
            reason: "artifact_not_approved",
        });
    }
    Ok(format!(
        "HISTORICAL_PRICE_BETA_APPROVAL status=ok operation=check approval_registry_sha256={} approval_status=APPROVED audience=OWNER_ONLY vendor_snapshot=true strict_pit=false capability=PRICE_RETURN_ONLY materialization_status=MATERIALIZED registration_status=UNREGISTERED publication_status=NOT_PUBLISHED instrument_count=11 session_count=2452 bar_count=26972",
        approved.pins().approval_registry_sha256(),
    ))
}

fn failure_line(failure: Failure) -> String {
    format!(
        "HISTORICAL_PRICE_BETA_APPROVAL status=blocked operation=check reason={}",
        failure.reason
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grammar_accepts_only_the_artifact_root() {
        let root = tempfile::tempdir().unwrap();
        let argv = vec![
            "check".into(),
            "--artifact-root".into(),
            root.path().display().to_string(),
        ];
        assert!(matches!(parse_args(&argv), Ok(ParseOutcome::Check(_))));
        let mut caller_pin = argv;
        caller_pin.extend([
            "--candidate-content-sha256".into(),
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        ]);
        assert!(parse_args(&caller_pin).is_err());
    }

    #[test]
    fn failures_are_sanitized() {
        assert_eq!(
            failure_line(Failure {
                reason: "artifact_not_approved"
            }),
            "HISTORICAL_PRICE_BETA_APPROVAL status=blocked operation=check reason=artifact_not_approved"
        );
    }
}
