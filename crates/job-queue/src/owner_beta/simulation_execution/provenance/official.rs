use std::{collections::BTreeMap, path::Path};

use serde::Deserialize;
use serde_json::Value;

use crate::owner_beta::simulation::EngineIdentity;

#[cfg(test)]
use super::sha256_file;
use super::{SimulationWorkerError, is_lower_sha256};

pub(super) const TOOLCHAIN: &str = "1.97.1";
pub(super) const TARGET: &str = "x86_64-unknown-linux-musl";
pub(super) const PROFILE: &str = "release";

const RECEIPT_FORMAT: &str = "lagrange-rust-artifact-v1";
const BUILD_ORIGIN: &str = "official-release-v1";
const PLATFORM: &str = "linux/amd64";
const PACKAGE: &str = "job-queue";
const BIN: &str = "owner-beta-simulation-runner";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CompileIdentity {
    git_commit: String,
    input_sha256: String,
    recipe_sha256: String,
}

impl CompileIdentity {
    pub(super) fn new(
        git_commit: &str,
        toolchain: &str,
        target: &str,
        profile: &str,
        input_sha256: &str,
        recipe_sha256: &str,
    ) -> Result<Self, ()> {
        if !is_lower_nonzero_hex(git_commit, 40)
            || toolchain != TOOLCHAIN
            || target != TARGET
            || profile != PROFILE
            || !is_lower_sha256(input_sha256)
            || !is_lower_sha256(recipe_sha256)
        {
            return Err(());
        }
        Ok(Self {
            git_commit: git_commit.to_owned(),
            input_sha256: input_sha256.to_owned(),
            recipe_sha256: recipe_sha256.to_owned(),
        })
    }
}

pub(super) fn compile_matches_process_config(
    compile: &CompileIdentity,
    code_commit: &str,
    source_input_sha256: &str,
    recipe_sha256: &str,
) -> bool {
    compile.git_commit == code_commit
        && source_input_sha256.strip_prefix("sha256:") == Some(compile.input_sha256.as_str())
        && recipe_sha256.strip_prefix("sha256:") == Some(compile.recipe_sha256.as_str())
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct OfficialReceipt {
    binary_mode: String,
    binary_sha256: String,
    bin: String,
    cache_key: String,
    cargo_success: bool,
    compile_env: BTreeMap<String, String>,
    features: Vec<String>,
    format: String,
    h_sha256: String,
    host_triple: String,
    input_sha256: String,
    package: String,
    platform: String,
    profile: String,
    recipe_sha256: String,
    source_commit: String,
}

#[cfg(test)]
pub(super) fn validate_receipt_value(
    value: Value,
    compile: &CompileIdentity,
    executable_path: &Path,
) -> Result<(EngineIdentity, String), SimulationWorkerError> {
    validate_receipt_value_with(value, compile, executable_path, sha256_file)
}

pub(super) fn validate_receipt_value_with(
    value: Value,
    compile: &CompileIdentity,
    executable_path: &Path,
    sha256_executable: impl FnOnce(&Path) -> Result<String, SimulationWorkerError>,
) -> Result<(EngineIdentity, String), SimulationWorkerError> {
    let receipt: OfficialReceipt =
        serde_json::from_value(value).map_err(|_| SimulationWorkerError::ProvenanceInvalid)?;
    if !receipt_matches_compile(&receipt, compile) {
        return Err(SimulationWorkerError::ProvenanceInvalid);
    }
    let executable_sha256 = sha256_executable(executable_path)?;
    if executable_sha256 != receipt.binary_sha256 {
        return Err(SimulationWorkerError::ProvenanceInvalid);
    }
    let engine = EngineIdentity::new(
        &receipt.source_commit,
        &format!("sha256:{}", receipt.binary_sha256),
        TOOLCHAIN,
        TARGET,
        PROFILE,
    )
    .map_err(|_| SimulationWorkerError::ProvenanceInvalid)?;
    Ok((engine, receipt.binary_sha256))
}

fn receipt_matches_compile(receipt: &OfficialReceipt, compile: &CompileIdentity) -> bool {
    receipt.format == RECEIPT_FORMAT
        && receipt.binary_mode == "0755"
        && receipt.bin == BIN
        && receipt.cargo_success
        && receipt.features.is_empty()
        && receipt.host_triple == TARGET
        && receipt.package == PACKAGE
        && receipt.platform == PLATFORM
        && receipt.profile == PROFILE
        && receipt.source_commit == compile.git_commit
        && is_lower_nonzero_hex(&receipt.source_commit, 40)
        && is_lower_sha256(&receipt.binary_sha256)
        && is_lower_sha256(&receipt.cache_key)
        && is_lower_sha256(&receipt.h_sha256)
        && is_lower_sha256(&receipt.input_sha256)
        && is_lower_sha256(&receipt.recipe_sha256)
        && receipt.input_sha256 == compile.input_sha256
        && receipt.recipe_sha256 == compile.recipe_sha256
        && receipt.compile_env
            == expected_compile_env(
                &receipt.source_commit,
                &receipt.input_sha256,
                &receipt.recipe_sha256,
            )
}

fn expected_compile_env(
    source_commit: &str,
    input_sha256: &str,
    recipe_sha256: &str,
) -> BTreeMap<String, String> {
    [
        ("CARGO_BUILD_JOBS", "2"),
        ("CARGO_TARGET_DIR", "/cargo-target"),
        ("RUSTFLAGS", "<unset>"),
        ("LAGRANGE_CODE_COMMIT", source_commit),
        ("R4_SIMULATION_BUILD_GIT_COMMIT", source_commit),
        ("R4_SIMULATION_BUILD_TOOLCHAIN", TOOLCHAIN),
        ("R4_SIMULATION_BUILD_TARGET", TARGET),
        ("R4_SIMULATION_BUILD_PROFILE", PROFILE),
        ("R4_SIMULATION_BUILD_ORIGIN", BUILD_ORIGIN),
        ("R4_SIMULATION_BUILD_SOURCE_INPUT_SHA256", input_sha256),
        ("R4_SIMULATION_BUILD_RECIPE_SHA256", recipe_sha256),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect()
}

fn is_lower_nonzero_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && value.bytes().any(|byte| byte != b'0')
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
    const INPUT: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    const RECIPE: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

    fn compile() -> CompileIdentity {
        CompileIdentity::new(COMMIT, TOOLCHAIN, TARGET, PROFILE, INPUT, RECIPE)
            .expect("official compile identity is fixed and complete")
    }

    fn receipt(binary_sha256: &str) -> Value {
        serde_json::json!({
            "binary_mode": "0755",
            "binary_sha256": binary_sha256,
            "bin": BIN,
            "cache_key": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "cargo_success": true,
            "compile_env": expected_compile_env(COMMIT, INPUT, RECIPE),
            "features": [],
            "format": RECEIPT_FORMAT,
            "h_sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "host_triple": TARGET,
            "input_sha256": INPUT,
            "package": PACKAGE,
            "platform": PLATFORM,
            "profile": PROFILE,
            "recipe_sha256": RECIPE,
            "source_commit": COMMIT,
        })
    }

    fn assert_rejected(value: Value, executable_path: &Path) {
        assert!(
            validate_receipt_value(value, &compile(), executable_path).is_err(),
            "invalid official receipt material must fail closed"
        );
    }

    #[test]
    fn official_receipt_accepts_only_the_exact_fixed_contract_and_engine_identity() {
        let executable = b"synthetic official runner";
        let binary_sha256 = super::super::sha256_hex(executable);
        let directory = tempfile::tempdir().expect("synthetic official provenance directory");
        let executable_path = directory.path().join("runner-fixture");
        std::fs::write(&executable_path, executable).expect("synthetic executable writes");

        let value = receipt(&binary_sha256);
        assert_eq!(value.as_object().expect("receipt object").len(), 16);
        assert_eq!(
            value["compile_env"]
                .as_object()
                .expect("compile environment")
                .len(),
            11
        );
        let (engine, parsed_binary_sha256) =
            validate_receipt_value(value, &compile(), &executable_path)
                .expect("matching official receipt and executable are accepted");
        assert_eq!(parsed_binary_sha256, binary_sha256);
        assert_eq!(
            engine,
            EngineIdentity::new(
                COMMIT,
                &format!("sha256:{binary_sha256}"),
                TOOLCHAIN,
                TARGET,
                PROFILE,
            )
            .expect("canonical official engine identity")
        );
    }

    #[test]
    fn official_receipt_rejects_unknown_duplicate_trailing_nonfinite_and_wrong_types() {
        use crate::owner_beta::simulation::parse_strict_json;

        let executable = b"synthetic receipt parser fixture";
        let binary_sha256 = super::super::sha256_hex(executable);
        let directory = tempfile::tempdir().expect("synthetic parser provenance directory");
        let executable_path = directory.path().join("runner-fixture");
        std::fs::write(&executable_path, executable).expect("synthetic executable writes");
        assert!(
            validate_receipt_value(receipt(&binary_sha256), &compile(), &executable_path,).is_ok()
        );

        let mut unknown = receipt(&binary_sha256);
        unknown["unrecognized"] = serde_json::json!(true);
        assert_rejected(unknown, &executable_path);
        let mut missing = receipt(&binary_sha256);
        missing
            .as_object_mut()
            .expect("receipt object")
            .remove("cache_key");
        assert_rejected(missing, &executable_path);

        let raw = serde_json::to_vec(&receipt(&binary_sha256)).expect("fixture serializes");
        let mut duplicate = raw.clone();
        duplicate.pop();
        duplicate.extend_from_slice(b",\"bin\":\"unexpected\"}");
        assert!(parse_strict_json(&duplicate).is_err());

        let mut trailing = raw.clone();
        trailing.extend_from_slice(b" {} ");
        assert!(parse_strict_json(&trailing).is_err());

        let nonfinite = String::from_utf8(raw)
            .expect("fixture is UTF-8")
            .replace("\"cargo_success\":true", "\"cargo_success\":NaN");
        assert!(parse_strict_json(nonfinite.as_bytes()).is_err());

        let mut wrong_bool = receipt(&binary_sha256);
        wrong_bool["cargo_success"] = serde_json::json!("true");
        assert_rejected(wrong_bool, &executable_path);
        let mut wrong_feature = receipt(&binary_sha256);
        wrong_feature["features"] = serde_json::json!([7]);
        assert_rejected(wrong_feature, &executable_path);
        let mut wrong_env_type = receipt(&binary_sha256);
        wrong_env_type["compile_env"]["CARGO_BUILD_JOBS"] = serde_json::json!(2);
        assert_rejected(wrong_env_type, &executable_path);
    }

    #[test]
    fn official_receipt_rejects_every_nonmatching_fixed_value_and_hash_shape() {
        let directory = tempfile::tempdir().expect("synthetic official provenance directory");
        let executable_path = directory.path().join("runner-fixture");
        std::fs::write(&executable_path, b"synthetic runner").expect("synthetic executable writes");
        let binary_sha256 = super::super::sha256_hex(b"synthetic runner");

        let invalid_values = [
            ("format", serde_json::json!("other-format")),
            ("binary_mode", serde_json::json!("0700")),
            ("bin", serde_json::json!("other-runner")),
            ("package", serde_json::json!("other-package")),
            ("platform", serde_json::json!("linux/arm64")),
            ("host_triple", serde_json::json!("x86_64-unknown-linux-gnu")),
            ("profile", serde_json::json!("debug")),
            ("features", serde_json::json!(["default"])),
            ("cargo_success", serde_json::json!(false)),
            ("source_commit", serde_json::json!("0".repeat(40))),
            ("binary_sha256", serde_json::json!("A".repeat(64))),
            ("cache_key", serde_json::json!("z".repeat(64))),
            ("h_sha256", serde_json::json!("not-a-digest")),
        ];
        for (field, invalid) in invalid_values {
            let mut value = receipt(&binary_sha256);
            value[field] = invalid;
            assert_rejected(value, &executable_path);
        }

        let mut invalid_input = receipt(&binary_sha256);
        invalid_input["input_sha256"] = serde_json::json!("f".repeat(64));
        assert_rejected(invalid_input, &executable_path);
        let mut invalid_recipe = receipt(&binary_sha256);
        invalid_recipe["recipe_sha256"] = serde_json::json!("e".repeat(64));
        assert_rejected(invalid_recipe, &executable_path);
        let mut invalid_commit = receipt(&binary_sha256);
        invalid_commit["source_commit"] =
            serde_json::json!("1123456789abcdef0123456789abcdef01234567");
        assert_rejected(invalid_commit, &executable_path);
    }

    #[test]
    fn official_compile_environment_is_exact_and_matches_all_source_bindings() {
        let directory = tempfile::tempdir().expect("synthetic official provenance directory");
        let executable_path = directory.path().join("runner-fixture");
        std::fs::write(&executable_path, b"synthetic runner").expect("synthetic executable writes");
        let binary_sha256 = super::super::sha256_hex(b"synthetic runner");
        assert!(
            validate_receipt_value(receipt(&binary_sha256), &compile(), &executable_path,).is_ok()
        );

        let mut incomplete = receipt(&binary_sha256);
        incomplete["compile_env"]
            .as_object_mut()
            .expect("compile environment")
            .remove("R4_SIMULATION_BUILD_PROFILE");
        assert_rejected(incomplete, &executable_path);

        let mut extra = receipt(&binary_sha256);
        extra["compile_env"]["WORKFLOW_RUN_ID"] = serde_json::json!("40000000001");
        assert_rejected(extra, &executable_path);

        for (key, value) in [
            ("CARGO_BUILD_JOBS", "3"),
            ("CARGO_TARGET_DIR", "/tmp/target"),
            ("RUSTFLAGS", "-C opt-level=3"),
            (
                "LAGRANGE_CODE_COMMIT",
                "1123456789abcdef0123456789abcdef01234567",
            ),
            (
                "R4_SIMULATION_BUILD_GIT_COMMIT",
                "1123456789abcdef0123456789abcdef01234567",
            ),
            ("R4_SIMULATION_BUILD_TOOLCHAIN", "1.97.2"),
            ("R4_SIMULATION_BUILD_TARGET", "x86_64-unknown-linux-gnu"),
            ("R4_SIMULATION_BUILD_PROFILE", "debug"),
            ("R4_SIMULATION_BUILD_ORIGIN", "hosted"),
            ("R4_SIMULATION_BUILD_SOURCE_INPUT_SHA256", "f"),
            ("R4_SIMULATION_BUILD_RECIPE_SHA256", "e"),
        ] {
            let mut inconsistent = receipt(&binary_sha256);
            inconsistent["compile_env"][key] = serde_json::json!(value);
            assert_rejected(inconsistent, &executable_path);
        }
    }

    #[test]
    fn official_receipt_rejects_self_consistent_but_uncompiled_source_identity() {
        let directory = tempfile::tempdir().expect("synthetic official provenance directory");
        let executable_path = directory.path().join("runner-fixture");
        std::fs::write(&executable_path, b"synthetic runner").expect("synthetic executable writes");
        let binary_sha256 = super::super::sha256_hex(b"synthetic runner");
        assert!(
            validate_receipt_value(receipt(&binary_sha256), &compile(), &executable_path,).is_ok()
        );

        let other_commit = "1123456789abcdef0123456789abcdef01234567";
        let mut commit_mismatch = receipt(&binary_sha256);
        commit_mismatch["source_commit"] = serde_json::json!(other_commit);
        commit_mismatch["compile_env"]["LAGRANGE_CODE_COMMIT"] = serde_json::json!(other_commit);
        commit_mismatch["compile_env"]["R4_SIMULATION_BUILD_GIT_COMMIT"] =
            serde_json::json!(other_commit);
        assert_rejected(commit_mismatch, &executable_path);

        let other_input = "f".repeat(64);
        let mut input_mismatch = receipt(&binary_sha256);
        input_mismatch["input_sha256"] = serde_json::json!(&other_input);
        input_mismatch["compile_env"]["R4_SIMULATION_BUILD_SOURCE_INPUT_SHA256"] =
            serde_json::json!(&other_input);
        assert_rejected(input_mismatch, &executable_path);

        let other_recipe = "e".repeat(64);
        let mut recipe_mismatch = receipt(&binary_sha256);
        recipe_mismatch["recipe_sha256"] = serde_json::json!(&other_recipe);
        recipe_mismatch["compile_env"]["R4_SIMULATION_BUILD_RECIPE_SHA256"] =
            serde_json::json!(&other_recipe);
        assert_rejected(recipe_mismatch, &executable_path);
    }

    #[test]
    fn official_receipt_rejects_executable_digest_mismatch() {
        let directory = tempfile::tempdir().expect("synthetic official provenance directory");
        let executable_path = directory.path().join("runner-fixture");
        std::fs::write(&executable_path, b"different executable")
            .expect("synthetic executable writes");
        let value = receipt(&super::super::sha256_hex(b"expected executable"));
        assert_rejected(value, &executable_path);
    }
}
