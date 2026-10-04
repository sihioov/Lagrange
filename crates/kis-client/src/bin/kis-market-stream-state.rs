use std::ffi::OsString;
use std::process::ExitCode;

use kis_client::market_stream_provisioning::{
    initialize_production_market_stream_domain, validate_production_market_stream_domain,
};
use kis_client::market_stream_state::StateError;
use kis_client::read_coordination_config::CREDENTIAL_GENERATION_ENV;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Initialize { slot: Uuid, generation: u64 },
    Validate { slot: Uuid, generation: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CliError {
    Usage,
    Configuration,
}

fn canonical_slot(value: &str) -> Option<Uuid> {
    let slot = Uuid::parse_str(value).ok()?;
    (!slot.is_nil() && slot.to_string() == value).then_some(slot)
}

fn canonical_generation(value: &str) -> Option<u64> {
    let generation = value.parse::<u64>().ok()?;
    (generation > 0 && generation.to_string() == value).then_some(generation)
}

fn parse_args(args: &[OsString]) -> Result<Action, CliError> {
    let command = args
        .first()
        .and_then(|arg| arg.to_str())
        .ok_or(CliError::Usage)?;
    let mut slot = None;
    let mut generation = None;
    let mut index = 1;
    while index < args.len() {
        let flag = args[index].to_str().ok_or(CliError::Usage)?;
        let value = args
            .get(index + 1)
            .and_then(|arg| arg.to_str())
            .ok_or(CliError::Usage)?;
        match flag {
            "--credential-slot-id" if slot.is_none() => {
                slot = Some(canonical_slot(value).ok_or(CliError::Usage)?);
            }
            "--credential-generation" if generation.is_none() => {
                generation = Some(canonical_generation(value).ok_or(CliError::Usage)?);
            }
            _ => return Err(CliError::Usage),
        }
        index += 2;
    }
    let slot = slot.ok_or(CliError::Usage)?;
    let generation = generation.ok_or(CliError::Usage)?;
    match command {
        "initialize-new" => Ok(Action::Initialize { slot, generation }),
        "validate-existing" => Ok(Action::Validate { slot, generation }),
        _ => Err(CliError::Usage),
    }
}

fn configured_generation() -> Result<u64, CliError> {
    std::env::var(CREDENTIAL_GENERATION_ENV)
        .ok()
        .and_then(|value| canonical_generation(&value))
        .ok_or(CliError::Configuration)
}

fn emit_error(code: &'static str, status: u8) -> ExitCode {
    eprintln!("{code}");
    ExitCode::from(status)
}

fn emit_state_error(error: StateError, initializing: bool) -> ExitCode {
    match error {
        StateError::ProvisioningActorDenied => emit_error("ERR_ACTOR", 3),
        StateError::ProvisioningInputInvalid => emit_error("ERR_INPUT", 2),
        _ if initializing => emit_error("ERR_INITIALIZATION", 6),
        _ => emit_error("ERR_STATE", 5),
    }
}

fn run(args: &[OsString]) -> ExitCode {
    let action = match parse_args(args) {
        Ok(action) => action,
        Err(CliError::Usage) => return emit_error("ERR_USAGE", 2),
        Err(CliError::Configuration) => return emit_error("ERR_CONFIG", 4),
    };
    let configured_generation = match configured_generation() {
        Ok(generation) => generation,
        Err(CliError::Configuration) => return emit_error("ERR_CONFIG", 4),
        Err(CliError::Usage) => return emit_error("ERR_CONFIG", 4),
    };
    let generation = match action {
        Action::Initialize { generation, .. } | Action::Validate { generation, .. } => generation,
    };
    if generation != configured_generation {
        return emit_error("ERR_CONFIG", 4);
    }
    match action {
        Action::Initialize { slot, generation } => {
            match initialize_production_market_stream_domain(slot, generation) {
                Ok(_) => {
                    println!("OK_INITIALIZED");
                    ExitCode::SUCCESS
                }
                Err(error) => emit_state_error(error, true),
            }
        }
        Action::Validate { slot, generation } => {
            match validate_production_market_stream_domain(slot, generation) {
                Ok(()) => {
                    println!("OK_VALIDATED");
                    ExitCode::SUCCESS
                }
                Err(error) => emit_state_error(error, false),
            }
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    run(&args)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(parts: &[&str]) -> Vec<OsString> {
        parts.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_only_the_two_exact_subcommands_and_canonical_inputs() {
        let slot = "62c6713e-19b7-4bd3-8be6-6a8c5fa3dd3c";
        assert_eq!(
            parse_args(&args(&[
                "initialize-new",
                "--credential-slot-id",
                slot,
                "--credential-generation",
                "17",
            ])),
            Ok(Action::Initialize {
                slot: Uuid::parse_str(slot).unwrap(),
                generation: 17,
            })
        );
        assert_eq!(
            parse_args(&args(&[
                "validate-existing",
                "--credential-generation",
                "17",
                "--credential-slot-id",
                slot,
            ])),
            Ok(Action::Validate {
                slot: Uuid::parse_str(slot).unwrap(),
                generation: 17,
            })
        );
    }

    #[test]
    fn rejects_missing_unknown_duplicate_positional_and_noncanonical_arguments() {
        let slot = "62c6713e-19b7-4bd3-8be6-6a8c5fa3dd3c";
        for invalid in [
            vec!["initialize-new"],
            vec!["initialize-new", "--credential-slot-id", slot],
            vec![
                "initialize-new",
                "--credential-slot-id",
                slot,
                "--credential-generation",
                "17",
                "extra",
            ],
            vec![
                "initialize-new",
                "--credential-slot-id",
                slot,
                "--credential-slot-id",
                slot,
                "--credential-generation",
                "17",
            ],
            vec![
                "initialize-new",
                "--credential-slot-id",
                slot,
                "--credential-generation",
                "17",
                "--path",
                "/tmp",
            ],
            vec![
                "initialize-new",
                "--credential-slot-id",
                "00000000-0000-0000-0000-000000000000",
                "--credential-generation",
                "17",
            ],
            vec![
                "initialize-new",
                "--credential-slot-id",
                "62C6713E-19B7-4BD3-8BE6-6A8C5FA3DD3C",
                "--credential-generation",
                "17",
            ],
            vec![
                "initialize-new",
                "--credential-slot-id",
                slot,
                "--credential-generation",
                "017",
            ],
            vec![
                "initialize-new",
                "--credential-slot-id",
                slot,
                "--credential-generation",
                "0",
            ],
            vec![
                "reset",
                "--credential-slot-id",
                slot,
                "--credential-generation",
                "17",
            ],
        ] {
            assert_eq!(
                parse_args(&args(&invalid)),
                Err(CliError::Usage),
                "{invalid:?}"
            );
        }
    }
}
