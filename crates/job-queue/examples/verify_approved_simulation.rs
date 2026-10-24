//! One-shot, read-only verification of the sealed approved historical artifact.
//!
//! This example is intentionally not a production daemon or job CLI. Runtime
//! identity comes only from the compile-time R4_VERIFICATION_* options and the
//! bytes of this executable.

use std::{env, ffi::OsString, fs, io, io::Read, io::Write, path::PathBuf, process};

use job_queue::owner_beta::simulation::{
    EngineIdentity, HashedSimulationResult, SimulationResult, SimulationRunError,
    SimulationRunIdentity, SimulationRunRequest, run_approved_historical_price_only_simulation,
};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const BUILD_GIT_COMMIT: Option<&str> = option_env!("R4_VERIFICATION_GIT_COMMIT");
const BUILD_TOOLCHAIN: Option<&str> = option_env!("R4_VERIFICATION_TOOLCHAIN");
const BUILD_TARGET: Option<&str> = option_env!("R4_VERIFICATION_TARGET");
const BUILD_PROFILE: Option<&str> = option_env!("R4_VERIFICATION_PROFILE");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Failure {
    Arguments,
    Provenance,
    BinaryIdentity,
    Simulation(SimulationRunError),
    Result,
    Summary,
}

impl Failure {
    fn sanitized_message(self) -> &'static str {
        match self {
            Self::Arguments => "error: invalid arguments",
            Self::Provenance => "error: compile-time provenance is missing or invalid",
            Self::BinaryIdentity => "error: executable identity verification failed",
            Self::Simulation(error) => match error {
                SimulationRunError::ApprovalRejected => "error: artifact approval rejected",
                SimulationRunError::InputInvalid => "error: approved input validation failed",
                SimulationRunError::SignalComputationFailed => "error: signal computation failed",
                SimulationRunError::FactorIdentityUnavailable => {
                    "error: factor identity unavailable"
                }
                SimulationRunError::ResultInvalid => "error: simulation result validation failed",
            },
            Self::Result => "error: result contract verification failed",
            Self::Summary => "error: summary generation failed",
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
struct Arguments {
    artifact_root: PathBuf,
    expected_binary_sha256: String,
}

struct BuildProvenance<'a> {
    git_commit: &'a str,
    toolchain: &'a str,
    target: &'a str,
    profile: &'a str,
}

struct CheckedResult {
    canonical: Vec<u8>,
    sha256: String,
    value: Value,
}

fn main() {
    if let Err(failure) = run() {
        eprintln!("{}", failure.sanitized_message());
        process::exit(1);
    }
}

fn run() -> Result<(), Failure> {
    let arguments = parse_arguments(env::args_os().skip(1))?;
    let provenance = BuildProvenance::parse(
        BUILD_GIT_COMMIT,
        BUILD_TOOLCHAIN,
        BUILD_TARGET,
        BUILD_PROFILE,
    )?;

    let executable = env::current_exe().map_err(|_| Failure::BinaryIdentity)?;
    let mut executable = fs::File::open(executable).map_err(|_| Failure::BinaryIdentity)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = executable
            .read(&mut buffer)
            .map_err(|_| Failure::BinaryIdentity)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let observed_binary_sha256 = format!("sha256:{:x}", hasher.finalize());
    if observed_binary_sha256 != arguments.expected_binary_sha256 {
        return Err(Failure::BinaryIdentity);
    }

    let engine = EngineIdentity::new(
        provenance.git_commit,
        &observed_binary_sha256,
        provenance.toolchain,
        provenance.target,
        provenance.profile,
    )
    .map_err(|_| Failure::Provenance)?;
    let identity = SimulationRunIdentity::new(engine);
    let request = SimulationRunRequest::new(Uuid::nil(), Uuid::nil());

    let first = run_approved_historical_price_only_simulation(
        &arguments.artifact_root,
        &request,
        &identity,
    )
    .map_err(Failure::Simulation)?;
    let second = run_approved_historical_price_only_simulation(
        &arguments.artifact_root,
        &request,
        &identity,
    )
    .map_err(Failure::Simulation)?;

    let first_illustrative = check_result(first.illustrative_10bp_per_side())?;
    let first_zero_cost = check_result(first.zero_cost_reference())?;
    let second_illustrative = check_result(second.illustrative_10bp_per_side())?;
    let second_zero_cost = check_result(second.zero_cost_reference())?;

    if !repeated_results_equal(&first_illustrative, &second_illustrative)
        || !repeated_results_equal(&first_zero_cost, &second_zero_cost)
    {
        return Err(Failure::Result);
    }

    let summary = build_summary(&first_illustrative, &first_zero_cost)?;
    let mut output = serde_json::to_vec(&summary).map_err(|_| Failure::Summary)?;
    output.push(b'\n');
    io::stdout()
        .lock()
        .write_all(&output)
        .map_err(|_| Failure::Summary)?;
    Ok(())
}

fn parse_arguments<I>(arguments: I) -> Result<Arguments, Failure>
where
    I: IntoIterator<Item = OsString>,
{
    let mut arguments = arguments.into_iter();
    let mut artifact_root = None;
    let mut expected_binary_sha256 = None;

    while let Some(argument) = arguments.next() {
        match argument.to_str().ok_or(Failure::Arguments)? {
            "--artifact-root" => {
                if artifact_root.is_some() {
                    return Err(Failure::Arguments);
                }
                let value = arguments.next().ok_or(Failure::Arguments)?;
                let path = PathBuf::from(value);
                if !path.is_absolute() {
                    return Err(Failure::Arguments);
                }
                artifact_root = Some(path);
            }
            "--expected-binary-sha256" => {
                if expected_binary_sha256.is_some() {
                    return Err(Failure::Arguments);
                }
                let value = arguments
                    .next()
                    .ok_or(Failure::Arguments)?
                    .into_string()
                    .map_err(|_| Failure::Arguments)?;
                if !is_lower_sha256(&value) {
                    return Err(Failure::Arguments);
                }
                expected_binary_sha256 = Some(value);
            }
            _ => return Err(Failure::Arguments),
        }
    }

    Ok(Arguments {
        artifact_root: artifact_root.ok_or(Failure::Arguments)?,
        expected_binary_sha256: expected_binary_sha256.ok_or(Failure::Arguments)?,
    })
}

impl<'a> BuildProvenance<'a> {
    fn parse(
        git_commit: Option<&'a str>,
        toolchain: Option<&'a str>,
        target: Option<&'a str>,
        profile: Option<&'a str>,
    ) -> Result<Self, Failure> {
        let git_commit = git_commit.ok_or(Failure::Provenance)?;
        let toolchain = toolchain.ok_or(Failure::Provenance)?;
        let target = target.ok_or(Failure::Provenance)?;
        let profile = profile.ok_or(Failure::Provenance)?;
        if !is_full_lower_hex(git_commit, 40)
            || !is_identity_token(toolchain)
            || !is_identity_token(target)
            || !is_identity_token(profile)
        {
            return Err(Failure::Provenance);
        }
        Ok(Self {
            git_commit,
            toolchain,
            target,
            profile,
        })
    }
}

fn is_identity_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte))
}

fn is_full_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_lower_sha256(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|digest| is_full_lower_hex(digest, 64))
}

fn sha256(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn check_result(result: &HashedSimulationResult) -> Result<CheckedResult, Failure> {
    let canonical = result
        .result()
        .canonical_json()
        .map_err(|_| Failure::Result)?;
    let library_hash = result.result_sha256().as_str();
    if sha256(&canonical) != library_hash
        || result
            .result()
            .result_content_sha256()
            .map_err(|_| Failure::Result)?
            .as_str()
            != library_hash
    {
        return Err(Failure::Result);
    }

    let strict_roundtrip = SimulationResult::from_json(&canonical).map_err(|_| Failure::Result)?;
    if strict_roundtrip
        .canonical_json()
        .map_err(|_| Failure::Result)?
        != canonical
    {
        return Err(Failure::Result);
    }

    let value = serde_json::from_slice(&canonical).map_err(|_| Failure::Result)?;
    Ok(CheckedResult {
        canonical,
        sha256: library_hash.to_owned(),
        value,
    })
}

fn repeated_results_equal(left: &CheckedResult, right: &CheckedResult) -> bool {
    left.canonical == right.canonical && left.sha256 == right.sha256
}

fn build_summary(
    illustrative: &CheckedResult,
    zero_cost: &CheckedResult,
) -> Result<Value, Failure> {
    let input = object_field(&illustrative.value, "input")?;
    let pins = object_field(input, "pins")?;
    let bars_sha256 = field(input, "bars_sha256")?.clone();
    let schedule = object_field(&illustrative.value, "schedule")?;
    let sessions = field(schedule, "evaluation_sessions")?
        .as_array()
        .ok_or(Failure::Summary)?;
    let start = sessions
        .first()
        .and_then(Value::as_str)
        .ok_or(Failure::Summary)?;
    let end = sessions
        .last()
        .and_then(Value::as_str)
        .ok_or(Failure::Summary)?;
    let engine = field(&illustrative.value, "engine")?.clone();

    Ok(json!({
        "status": "success",
        "engine": engine,
        "input_pins": pins,
        "bars_sha256": bars_sha256,
        "evaluation": {
            "start": start,
            "end": end,
            "session_count": sessions.len(),
        },
        "results": {
            "illustrative_10bp_per_side": profile_summary(illustrative)?,
            "zero_cost_reference": profile_summary(zero_cost)?,
        },
        "repeated_equal": true,
    }))
}

fn profile_summary(result: &CheckedResult) -> Result<Value, Failure> {
    let strategy_ledger = object_field(&result.value, "strategy_ledger")?;
    let benchmark_ledger = object_field(&result.value, "benchmark_ledger")?;
    let strategy_metrics = selected_metrics(object_field(strategy_ledger, "metrics")?)?;
    let benchmark_metrics = selected_metrics(object_field(benchmark_ledger, "metrics")?)?;
    let benchmark_return_difference = field(strategy_ledger, "benchmark_return_difference")?;

    Ok(json!({
        "sha256": result.sha256,
        "strategy_metrics": strategy_metrics,
        "benchmark_metrics": benchmark_metrics,
        "benchmark_return_difference": benchmark_return_difference,
    }))
}

fn selected_metrics(metrics: &Value) -> Result<Value, Failure> {
    const AGGREGATE_FIELDS: &[&str] = &[
        "period_return",
        "ending_nav",
        "ending_cash",
        "ending_holdings",
        "cumulative_costs",
        "buy_notional",
        "sell_notional",
        "maximum_drawdown",
        "period_turnover",
        "fill_count",
    ];
    let mut selected = Map::new();
    for name in AGGREGATE_FIELDS {
        selected.insert((*name).to_owned(), field(metrics, name)?.clone());
    }
    Ok(Value::Object(selected))
}

fn object_field<'a>(value: &'a Value, name: &str) -> Result<&'a Value, Failure> {
    let value = field(value, name)?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(Failure::Summary)
    }
}

fn field<'a>(value: &'a Value, name: &str) -> Result<&'a Value, Failure> {
    value.get(name).ok_or(Failure::Summary)
}

#[cfg(test)]
mod tests {
    use super::{BuildProvenance, Failure, parse_arguments};
    use std::ffi::OsString;
    use std::path::PathBuf;

    const HASH: &str = concat!(
        "sha256:",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );

    fn arguments(values: &[&str]) -> Vec<OsString> {
        values.iter().map(|value| OsString::from(*value)).collect()
    }

    #[test]
    fn accepts_the_two_required_options_in_either_order() {
        let parsed = parse_arguments(arguments(&[
            "--expected-binary-sha256",
            HASH,
            "--artifact-root",
            "/approved",
        ]))
        .expect("valid arguments");
        assert_eq!(parsed.artifact_root, PathBuf::from("/approved"));
        assert_eq!(parsed.expected_binary_sha256, HASH);
    }

    #[test]
    fn rejects_unknown_repeated_missing_and_relative_arguments() {
        assert_eq!(
            parse_arguments(arguments(&["--unknown", "x"])),
            Err(Failure::Arguments)
        );
        assert_eq!(
            parse_arguments(arguments(&[
                "--artifact-root",
                "/approved",
                "--artifact-root",
                "/again",
                "--expected-binary-sha256",
                HASH,
            ])),
            Err(Failure::Arguments)
        );
        assert_eq!(
            parse_arguments(arguments(&[
                "--artifact-root",
                "/approved",
                "--expected-binary-sha256",
                HASH,
                "--expected-binary-sha256",
                HASH,
            ])),
            Err(Failure::Arguments)
        );
        assert_eq!(parse_arguments(arguments(&[])), Err(Failure::Arguments));
        assert_eq!(
            parse_arguments(arguments(&["--artifact-root", "/approved"])),
            Err(Failure::Arguments)
        );
        assert_eq!(
            parse_arguments(arguments(&[
                "--artifact-root",
                "relative",
                "--expected-binary-sha256",
                HASH,
            ])),
            Err(Failure::Arguments)
        );
    }

    #[test]
    fn rejects_malformed_expected_binary_hashes() {
        for invalid in [
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaA",
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ] {
            assert_eq!(
                parse_arguments(arguments(&[
                    "--artifact-root",
                    "/approved",
                    "--expected-binary-sha256",
                    invalid,
                ])),
                Err(Failure::Arguments)
            );
        }
    }

    #[test]
    fn rejects_missing_or_invalid_compile_time_provenance() {
        let uppercase_commit = "A".repeat(40);
        let lowercase_commit = "a".repeat(40);
        assert_eq!(
            BuildProvenance::parse(
                None,
                Some("rustc-1.97.1"),
                Some("x86_64-linux"),
                Some("release")
            )
            .map(|_| ()),
            Err(Failure::Provenance)
        );
        assert_eq!(
            BuildProvenance::parse(
                Some(uppercase_commit.as_str()),
                Some("rustc-1.97.1"),
                Some("x86_64-linux"),
                Some("release"),
            )
            .map(|_| ()),
            Err(Failure::Provenance)
        );
        assert_eq!(
            BuildProvenance::parse(
                Some(lowercase_commit.as_str()),
                Some("rustc 1.97.1"),
                Some("x86_64-linux"),
                Some("release"),
            )
            .map(|_| ()),
            Err(Failure::Provenance)
        );
    }
}
