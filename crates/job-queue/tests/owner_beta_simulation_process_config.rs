use job_queue::owner_beta::simulation_execution::{
    SimulationProcessConfig, SimulationProcessConfigError, SimulationProcessLimits,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const TEST_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const VALID_CONFIG_JSON: &str = r###"{"schema":"owner-beta-simulation-process-v1","code_commit":"0123456789abcdef0123456789abcdef01234567","manifest_format":"LAGRANGE_RELEASE_MANIFEST_V3","manifest_sha256":"sha256:1111111111111111111111111111111111111111111111111111111111111111","image_id":"sha256:2222222222222222222222222222222222222222222222222222222222222222","receipt_sha256":"sha256:3333333333333333333333333333333333333333333333333333333333333333","binary_sha256":"sha256:4444444444444444444444444444444444444444444444444444444444444444","source_input_sha256":"sha256:5555555555555555555555555555555555555555555555555555555555555555","recipe_sha256":"sha256:6666666666666666666666666666666666666666666666666666666666666666","build_origin":"official-release-v1","package":"job-queue","bin":"owner-beta-simulation-runner","target":"x86_64-unknown-linux-musl","profile":"release","toolchain":"1.97.1","owner":"11111111-2222-3333-4444-555555555555","entitlement_reference":"fixture:synthetic-owner-rights-v1","entitlement_sha256":"sha256:7777777777777777777777777777777777777777777777777777777777777777","historical_root":"/synthetic/history/δ","reader_uid":10001,"reader_gid":10001,"paths":{"config":"/run/owner-beta-simulation/config.json","artifact_root":"/data/owner-beta-artifacts","receipt":"/usr/share/lagrange/owner-beta-simulation/artifact.json","complete":"/usr/share/lagrange/owner-beta-simulation/COMPLETE","executable":"/usr/local/bin/owner-beta-simulation-runner","worker_id":"owner-beta-simulation-runner"},"limits":{"memory_bytes":1234567,"nano_cpus":2345678,"health_tmpfs_bytes":3456789,"pids_limit":42,"startup_timeout_seconds":123,"health_max_age_seconds":234,"stop_timeout_seconds":345}}"###;
const VALID_CONFIG_JSON_SHA256: &str =
    "sha256:366b4941febbbfde0aeac8260321210888cb867e4aee9e5e6eb79f88842b6252";
const VALID_CONFIG_PRETTY_JSON: &str = r###"{
  "schema": "owner-beta-simulation-process-v1",
  "code_commit": "0123456789abcdef0123456789abcdef01234567",
  "manifest_format": "LAGRANGE_RELEASE_MANIFEST_V3",
  "manifest_sha256": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
  "image_id": "sha256:2222222222222222222222222222222222222222222222222222222222222222",
  "receipt_sha256": "sha256:3333333333333333333333333333333333333333333333333333333333333333",
  "binary_sha256": "sha256:4444444444444444444444444444444444444444444444444444444444444444",
  "source_input_sha256": "sha256:5555555555555555555555555555555555555555555555555555555555555555",
  "recipe_sha256": "sha256:6666666666666666666666666666666666666666666666666666666666666666",
  "build_origin": "official-release-v1",
  "package": "job-queue",
  "bin": "owner-beta-simulation-runner",
  "target": "x86_64-unknown-linux-musl",
  "profile": "release",
  "toolchain": "1.97.1",
  "owner": "11111111-2222-3333-4444-555555555555",
  "entitlement_reference": "fixture:synthetic-owner-rights-v1",
  "entitlement_sha256": "sha256:7777777777777777777777777777777777777777777777777777777777777777",
  "historical_root": "/synthetic/history/δ",
  "reader_uid": 10001,
  "reader_gid": 10001,
  "paths": {
    "config": "/run/owner-beta-simulation/config.json",
    "artifact_root": "/data/owner-beta-artifacts",
    "receipt": "/usr/share/lagrange/owner-beta-simulation/artifact.json",
    "complete": "/usr/share/lagrange/owner-beta-simulation/COMPLETE",
    "executable": "/usr/local/bin/owner-beta-simulation-runner",
    "worker_id": "owner-beta-simulation-runner"
  },
  "limits": {
    "memory_bytes": 1234567,
    "nano_cpus": 2345678,
    "health_tmpfs_bytes": 3456789,
    "pids_limit": 42,
    "startup_timeout_seconds": 123,
    "health_max_age_seconds": 234,
    "stop_timeout_seconds": 345
  }
}"###;
const VALID_CONFIG_PRETTY_JSON_SHA256: &str =
    "sha256:4b21a0a348a4eb7ed887b3ba3fe8042e8543fc787c3cb68f5dd7d5e44a08df97";

const DIGEST_FIELDS: [&str; 7] = [
    "manifest_sha256",
    "image_id",
    "receipt_sha256",
    "binary_sha256",
    "source_input_sha256",
    "recipe_sha256",
    "entitlement_sha256",
];

fn raw_sha256(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn valid_baseline() -> Value {
    let checked = SimulationProcessConfig::from_json(
        VALID_CONFIG_JSON.as_bytes(),
        TEST_COMMIT,
        VALID_CONFIG_JSON_SHA256,
    )
    .expect("the pinned compact fixture is valid");
    assert_eq!(checked.config_sha256(), VALID_CONFIG_JSON_SHA256);
    serde_json::from_slice(VALID_CONFIG_JSON.as_bytes()).expect("fixture JSON parses")
}

fn assert_invalid_bytes(bytes: &[u8]) {
    let matching_digest = raw_sha256(bytes);
    assert!(matches!(
        SimulationProcessConfig::from_json(bytes, TEST_COMMIT, &matching_digest),
        Err(SimulationProcessConfigError::Invalid)
    ));
}

fn assert_size_limit(bytes: &[u8]) {
    let matching_digest = raw_sha256(bytes);
    assert!(matches!(
        SimulationProcessConfig::from_json(bytes, TEST_COMMIT, &matching_digest),
        Err(SimulationProcessConfigError::SizeLimitExceeded)
    ));
}

fn assert_invalid_value(mutate: impl FnOnce(&mut Value)) {
    let mut value = valid_baseline();
    mutate(&mut value);
    let bytes = serde_json::to_vec(&value).expect("mutated fixture serializes");
    assert_invalid_bytes(&bytes);
}

fn checked_value(mutate: impl FnOnce(&mut Value)) -> SimulationProcessConfig {
    let mut value = valid_baseline();
    mutate(&mut value);
    let bytes = serde_json::to_vec(&value).expect("mutated fixture serializes");
    let matching_digest = raw_sha256(&bytes);
    SimulationProcessConfig::from_json(&bytes, TEST_COMMIT, &matching_digest)
        .expect("mutated fixture remains valid")
}

fn replace_once(input: &str, before: &str, after: &str) -> String {
    assert_eq!(
        input.matches(before).count(),
        1,
        "fixture marker occurs once"
    );
    input.replacen(before, after, 1)
}

fn replace_bytes_once(input: &[u8], before: &[u8], after: &[u8]) -> Vec<u8> {
    let position = input
        .windows(before.len())
        .position(|window| window == before)
        .expect("fixture byte marker exists");
    let mut result = Vec::with_capacity(input.len() - before.len() + after.len());
    result.extend_from_slice(&input[..position]);
    result.extend_from_slice(after);
    result.extend_from_slice(&input[position + before.len()..]);
    result
}

fn sequence_for_object(value: &Value, fields_in_wire_order: &[&str]) -> Value {
    Value::Array(
        fields_in_wire_order
            .iter()
            .map(|field| value[*field].clone())
            .collect(),
    )
}

#[test]
fn compact_and_formatted_bytes_bind_their_own_sha_and_checked_getters() {
    let compact = SimulationProcessConfig::from_json(
        VALID_CONFIG_JSON.as_bytes(),
        TEST_COMMIT,
        VALID_CONFIG_JSON_SHA256,
    )
    .expect("compact fixture is valid");
    let pretty = SimulationProcessConfig::from_json(
        VALID_CONFIG_PRETTY_JSON.as_bytes(),
        TEST_COMMIT,
        VALID_CONFIG_PRETTY_JSON_SHA256,
    )
    .expect("formatted fixture is valid");

    assert_eq!(compact.config_sha256(), VALID_CONFIG_JSON_SHA256);
    assert_eq!(pretty.config_sha256(), VALID_CONFIG_PRETTY_JSON_SHA256);
    assert_ne!(compact.config_sha256(), pretty.config_sha256());
    assert_eq!(
        serde_json::from_str::<Value>(VALID_CONFIG_JSON).unwrap(),
        serde_json::from_str::<Value>(VALID_CONFIG_PRETTY_JSON).unwrap()
    );
    assert!(matches!(
        SimulationProcessConfig::from_json(
            VALID_CONFIG_PRETTY_JSON.as_bytes(),
            TEST_COMMIT,
            VALID_CONFIG_JSON_SHA256
        ),
        Err(SimulationProcessConfigError::Invalid)
    ));

    assert_eq!(compact.code_commit(), TEST_COMMIT);
    assert_eq!(
        compact.manifest_sha256(),
        format!("sha256:{}", "1".repeat(64))
    );
    assert_eq!(compact.image_id(), format!("sha256:{}", "2".repeat(64)));
    assert_eq!(
        compact.receipt_sha256(),
        format!("sha256:{}", "3".repeat(64))
    );
    assert_eq!(
        compact.binary_sha256(),
        format!("sha256:{}", "4".repeat(64))
    );
    assert_eq!(
        compact.source_input_sha256(),
        format!("sha256:{}", "5".repeat(64))
    );
    assert_eq!(
        compact.recipe_sha256(),
        format!("sha256:{}", "6".repeat(64))
    );
    assert_eq!(
        compact.owner(),
        Uuid::parse_str("11111111-2222-3333-4444-555555555555").unwrap()
    );
    assert_eq!(
        compact.entitlement_reference(),
        "fixture:synthetic-owner-rights-v1"
    );
    assert_eq!(
        compact.entitlement_sha256(),
        format!("sha256:{}", "7".repeat(64))
    );
    assert_eq!(compact.historical_root(), "/synthetic/history/δ");
    assert_eq!(compact.reader_uid(), 10001);
    assert_eq!(compact.reader_gid(), 10001);

    assert_eq!(
        compact.config_path(),
        "/run/owner-beta-simulation/config.json"
    );
    assert_eq!(compact.artifact_root(), "/data/owner-beta-artifacts");
    assert_eq!(
        compact.receipt_path(),
        "/usr/share/lagrange/owner-beta-simulation/artifact.json"
    );
    assert_eq!(
        compact.complete_path(),
        "/usr/share/lagrange/owner-beta-simulation/COMPLETE"
    );
    assert_eq!(
        compact.executable_path(),
        "/usr/local/bin/owner-beta-simulation-runner"
    );
    assert_eq!(compact.worker_id(), "owner-beta-simulation-runner");

    let limits = compact.limits();
    assert_eq!(limits.memory_bytes(), 1_234_567);
    assert_eq!(limits.nano_cpus(), 2_345_678);
    assert_eq!(limits.health_tmpfs_bytes(), 3_456_789);
    assert_eq!(limits.pids_limit(), 42);
    assert_eq!(limits.startup_timeout_seconds(), 123);
    assert_eq!(limits.health_max_age_seconds(), 234);
    assert_eq!(limits.stop_timeout_seconds(), 345);
    assert_eq!(compact.historical_root(), "/synthetic/history/δ");
}

#[test]
fn parser_rejects_non_object_shapes_duplicates_unknowns_and_malformed_bytes() {
    let _baseline = valid_baseline();

    let duplicate_root = replace_once(
        VALID_CONFIG_JSON,
        r#""schema":"owner-beta-simulation-process-v1","#,
        r#""schema":"owner-beta-simulation-process-v1","schema":"owner-beta-simulation-process-v1","#,
    );
    assert_invalid_bytes(duplicate_root.as_bytes());

    let escaped_duplicate_root = replace_once(
        VALID_CONFIG_JSON,
        r#""schema":"owner-beta-simulation-process-v1","#,
        r#""schema":"owner-beta-simulation-process-v1","\u0073chema":"owner-beta-simulation-process-v1","#,
    );
    assert_invalid_bytes(escaped_duplicate_root.as_bytes());

    let duplicate_path = replace_once(
        VALID_CONFIG_JSON,
        r#""config":"/run/owner-beta-simulation/config.json","#,
        r#""config":"/run/owner-beta-simulation/config.json","config":"/run/owner-beta-simulation/config.json","#,
    );
    assert_invalid_bytes(duplicate_path.as_bytes());

    let duplicate_limit = replace_once(
        VALID_CONFIG_JSON,
        r#""memory_bytes":1234567,"#,
        r#""memory_bytes":1234567,"memory_bytes":1234567,"#,
    );
    assert_invalid_bytes(duplicate_limit.as_bytes());

    assert_invalid_value(|value| {
        value.as_object_mut().unwrap().remove("bin");
    });
    assert_invalid_value(|value| {
        value["unexpected"] = json!("unknown");
    });
    assert_invalid_value(|value| {
        value["paths"].as_object_mut().unwrap().remove("config");
    });
    assert_invalid_value(|value| {
        value["paths"]["unexpected"] = json!("unknown");
    });
    assert_invalid_value(|value| {
        value["limits"]
            .as_object_mut()
            .unwrap()
            .remove("memory_bytes");
    });
    assert_invalid_value(|value| {
        value["limits"]["unexpected"] = json!(1);
    });

    for malformed in [
        &b"{"[..],
        b"{\"schema\":}",
        b"{\"schema\":\"x\",}",
        b"[]",
        b"null",
        b"true",
        b"1",
        b"\"scalar\"",
    ] {
        assert_invalid_bytes(malformed);
    }

    let invalid_utf8 = replace_bytes_once(
        VALID_CONFIG_JSON.as_bytes(),
        b"fixture:synthetic-owner-rights-v1",
        b"fixture:\xffnthetic-owner-rights-v1",
    );
    assert_invalid_bytes(&invalid_utf8);

    let lone_surrogate = replace_bytes_once(
        VALID_CONFIG_JSON.as_bytes(),
        b"fixture:synthetic-owner-rights-v1",
        br"\ud800",
    );
    assert_invalid_bytes(&lone_surrogate);

    let mut trailing = VALID_CONFIG_JSON.as_bytes().to_vec();
    trailing.extend_from_slice(b"not-json");
    assert_invalid_bytes(&trailing);

    let mut whitespace = VALID_CONFIG_JSON.as_bytes().to_vec();
    whitespace.extend_from_slice(b" \r\n\t");
    let whitespace_sha = raw_sha256(&whitespace);
    assert!(SimulationProcessConfig::from_json(&whitespace, TEST_COMMIT, &whitespace_sha).is_ok());

    let mut exact_size_limit = VALID_CONFIG_JSON.as_bytes().to_vec();
    exact_size_limit.resize(16 * 1024, b' ');
    let exact_size_digest = raw_sha256(&exact_size_limit);
    assert!(
        SimulationProcessConfig::from_json(&exact_size_limit, TEST_COMMIT, &exact_size_digest)
            .is_ok()
    );

    let oversized = vec![b' '; 16 * 1024 + 1];
    assert_size_limit(&oversized);
}

#[test]
fn root_paths_and_limits_must_be_json_objects_not_full_struct_sequences() {
    const ROOT_FIELDS_IN_WIRE_ORDER: [&str; 23] = [
        "schema",
        "code_commit",
        "manifest_format",
        "manifest_sha256",
        "image_id",
        "receipt_sha256",
        "binary_sha256",
        "source_input_sha256",
        "recipe_sha256",
        "build_origin",
        "package",
        "bin",
        "target",
        "profile",
        "toolchain",
        "owner",
        "entitlement_reference",
        "entitlement_sha256",
        "historical_root",
        "reader_uid",
        "reader_gid",
        "paths",
        "limits",
    ];
    const PATH_FIELDS_IN_WIRE_ORDER: [&str; 6] = [
        "config",
        "artifact_root",
        "receipt",
        "complete",
        "executable",
        "worker_id",
    ];
    const LIMIT_FIELDS_IN_WIRE_ORDER: [&str; 7] = [
        "memory_bytes",
        "nano_cpus",
        "health_tmpfs_bytes",
        "pids_limit",
        "startup_timeout_seconds",
        "health_max_age_seconds",
        "stop_timeout_seconds",
    ];

    let baseline = valid_baseline();
    let root_sequence = sequence_for_object(&baseline, &ROOT_FIELDS_IN_WIRE_ORDER);
    assert_invalid_bytes(&serde_json::to_vec(&root_sequence).unwrap());

    let mut paths_sequence = valid_baseline();
    paths_sequence["paths"] = sequence_for_object(&baseline["paths"], &PATH_FIELDS_IN_WIRE_ORDER);
    assert_invalid_bytes(&serde_json::to_vec(&paths_sequence).unwrap());

    let mut limits_sequence = valid_baseline();
    limits_sequence["limits"] =
        sequence_for_object(&baseline["limits"], &LIMIT_FIELDS_IN_WIRE_ORDER);
    assert_invalid_bytes(&serde_json::to_vec(&limits_sequence).unwrap());
}

#[test]
fn commits_fixed_strings_and_digest_syntax_are_checked_individually() {
    let _baseline = valid_baseline();

    let wrong_commit = "fedcba9876543210fedcba9876543210fedcba98";
    assert!(matches!(
        SimulationProcessConfig::from_json(
            VALID_CONFIG_JSON.as_bytes(),
            wrong_commit,
            VALID_CONFIG_JSON_SHA256
        ),
        Err(SimulationProcessConfigError::Invalid)
    ));
    assert!(matches!(
        SimulationProcessConfig::from_json(
            VALID_CONFIG_JSON.as_bytes(),
            &"0".repeat(40),
            VALID_CONFIG_JSON_SHA256
        ),
        Err(SimulationProcessConfigError::Invalid)
    ));
    assert!(matches!(
        SimulationProcessConfig::from_json(
            VALID_CONFIG_JSON.as_bytes(),
            &TEST_COMMIT.to_ascii_uppercase(),
            VALID_CONFIG_JSON_SHA256
        ),
        Err(SimulationProcessConfigError::Invalid)
    ));
    assert!(matches!(
        SimulationProcessConfig::from_json(
            VALID_CONFIG_JSON.as_bytes(),
            &TEST_COMMIT[..39],
            VALID_CONFIG_JSON_SHA256
        ),
        Err(SimulationProcessConfigError::Invalid)
    ));
    assert!(matches!(
        SimulationProcessConfig::from_json(
            VALID_CONFIG_JSON.as_bytes(),
            TEST_COMMIT,
            &format!("sha256:{}", "0".repeat(64))
        ),
        Err(SimulationProcessConfigError::Invalid)
    ));
    assert!(matches!(
        SimulationProcessConfig::from_json(
            VALID_CONFIG_JSON.as_bytes(),
            TEST_COMMIT,
            &format!("sha256:{}", "A".repeat(64))
        ),
        Err(SimulationProcessConfigError::Invalid)
    ));
    for malformed_digest in ["SHA256:abc", "sha256:ABC", "sha256:abc"] {
        assert!(matches!(
            SimulationProcessConfig::from_json(
                VALID_CONFIG_JSON.as_bytes(),
                TEST_COMMIT,
                malformed_digest
            ),
            Err(SimulationProcessConfigError::Invalid)
        ));
    }

    for key in [
        "schema",
        "manifest_format",
        "build_origin",
        "package",
        "bin",
        "target",
        "profile",
        "toolchain",
    ] {
        assert_invalid_value(|value| value[key] = json!("synthetic-mismatch"));
    }
    assert_invalid_value(|value| {
        value["code_commit"] = json!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    });
    assert_invalid_value(|value| {
        value["code_commit"] = json!("0000000000000000000000000000000000000000");
    });

    let uppercase_digest = format!("sha256:{}", "A".repeat(64));
    for key in DIGEST_FIELDS {
        assert_invalid_value(|value| value[key] = json!(uppercase_digest));
        assert_invalid_value(|value| value[key] = json!("sha256:abc"));
    }

    let zero_digest = format!("sha256:{}", "0".repeat(64));
    let all_zero_digests = checked_value(|value| {
        for key in DIGEST_FIELDS {
            value[key] = json!(zero_digest);
        }
    });
    assert_eq!(all_zero_digests.code_commit(), TEST_COMMIT);
}

#[test]
fn uuid_entitlement_and_historical_path_unicode_grammar_matches_c1() {
    let _baseline = valid_baseline();

    for invalid_owner in [
        "11111111-2222-3333-4444-55555555555A",
        "{11111111-2222-3333-4444-555555555555}",
        "11111111222233334444555555555555",
        "00000000-0000-0000-0000-000000000000",
        " 11111111-2222-3333-4444-555555555555",
    ] {
        assert_invalid_value(|value| value["owner"] = json!(invalid_owner));
    }
    let non_version_restricted_uuid = checked_value(|value| {
        value["owner"] = json!("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee");
    });
    assert_eq!(
        non_version_restricted_uuid.owner().to_string(),
        "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
    );

    let unicode_reference = "권리: retained 内部 whitespace / fixture";
    let unicode_config = checked_value(|value| {
        value["entitlement_reference"] = json!(unicode_reference);
    });
    assert_eq!(unicode_config.entitlement_reference(), unicode_reference);

    let maximum_scalar_reference = "🧪".repeat(512);
    let maximum_reference_config = checked_value(|value| {
        value["entitlement_reference"] = json!(maximum_scalar_reference.clone());
    });
    assert_eq!(
        maximum_reference_config
            .entitlement_reference()
            .chars()
            .count(),
        512
    );
    assert!(maximum_reference_config.entitlement_reference().len() <= 4096);
    // The 512-scalar ceiling implies at most 2048 UTF-8 bytes, so the 4096-byte
    // ceiling is redundant for accepted Unicode scalar strings. Exercise its
    // over-limit branch with a synthetic value that exceeds both ceilings.
    let over_byte_limit_reference = "🧪".repeat(1025);
    assert!(over_byte_limit_reference.len() > 4096);
    assert_invalid_value(|value| {
        value["entitlement_reference"] = json!(over_byte_limit_reference);
    });

    for invalid_reference in [
        String::new(),
        " padded ".to_owned(),
        "\u{00a0}padded".to_owned(),
        "padded\u{2003}".to_owned(),
        "too-many-scalars".repeat(40),
        "control\u{0000}inside".to_owned(),
        "control\u{009f}inside".to_owned(),
    ] {
        assert_invalid_value(|value| value["entitlement_reference"] = json!(invalid_reference));
    }

    let valid_unicode_path = "/synthetic/archive/分区/space name/δ";
    let unicode_path_config = checked_value(|value| {
        value["historical_root"] = json!(valid_unicode_path);
    });
    assert_eq!(unicode_path_config.historical_root(), valid_unicode_path);

    for invalid_path in [
        "/",
        "/synthetic/",
        "//synthetic/history",
        "/synthetic//history",
        "/synthetic/./history",
        "/synthetic/../history",
        "/synthetic/control\u{007f}path",
        "/synthetic/control\u{009f}path",
    ] {
        assert_invalid_value(|value| value["historical_root"] = json!(invalid_path));
    }
    let normalized_counterpart = checked_value(|value| {
        value["historical_root"] = json!("/synthetic/archive/history");
    });
    assert_eq!(
        normalized_counterpart.historical_root(),
        "/synthetic/archive/history"
    );
}

#[test]
fn reader_ids_paths_and_all_limit_categories_reject_wrong_json_types_and_bounds() {
    let _baseline = valid_baseline();

    for key in ["reader_uid", "reader_gid"] {
        for invalid in [
            json!(10000),
            json!(10002),
            json!(-1),
            json!(u32::MAX as u64 + 1),
            json!(true),
            json!("10001"),
            json!(10001.0),
        ] {
            assert_invalid_value(|value| value[key] = invalid);
        }
        let before = format!("\"{key}\":10001");
        let after = format!("\"{key}\":10001e0");
        let exponent = replace_once(VALID_CONFIG_JSON, &before, &after);
        assert_invalid_bytes(exponent.as_bytes());
    }

    for (key, wrong) in [
        ("config", "/run/other/config.json"),
        ("artifact_root", "/data/other-artifacts"),
        ("receipt", "/usr/share/other/artifact.json"),
        ("complete", "/usr/share/other/COMPLETE"),
        ("executable", "/usr/local/bin/other-runner"),
        ("worker_id", "other-runner"),
    ] {
        assert_invalid_value(|value| value["paths"][key] = json!(wrong));
    }

    const LIMIT_CASES: [(&str, u64, u64); 7] = [
        ("memory_bytes", 1, i64::MAX as u64),
        ("nano_cpus", 1, i64::MAX as u64),
        ("health_tmpfs_bytes", 1, i64::MAX as u64),
        ("pids_limit", 1, i32::MAX as u64),
        ("startup_timeout_seconds", 1, i32::MAX as u64),
        ("health_max_age_seconds", 1, i32::MAX as u64),
        ("stop_timeout_seconds", 11, i32::MAX as u64),
    ];

    for (key, minimum, maximum) in LIMIT_CASES {
        for bound in [minimum, maximum] {
            let config = checked_value(|value| value["limits"][key] = json!(bound));
            assert_eq!(limit_for_key(config.limits(), key), Some(bound));
        }

        for invalid in [
            json!(minimum - 1),
            json!(maximum + 1),
            json!(0),
            json!(-1),
            json!(true),
            json!("1"),
            json!(1.0),
        ] {
            assert_invalid_value(|value| value["limits"][key] = invalid);
        }

        let baseline = valid_baseline();
        let baseline_number = baseline["limits"][key].as_u64().unwrap();
        let before = format!("\"{key}\":{baseline_number}");
        let exponent = replace_once(
            VALID_CONFIG_JSON,
            &before,
            &format!("\"{key}\":{baseline_number}e0"),
        );
        assert_invalid_bytes(exponent.as_bytes());

        let overflow = replace_once(
            VALID_CONFIG_JSON,
            &before,
            &format!("\"{key}\":18446744073709551616"),
        );
        assert_invalid_bytes(overflow.as_bytes());
    }
}

fn limit_for_key(limits: &SimulationProcessLimits, key: &str) -> Option<u64> {
    match key {
        "memory_bytes" => Some(limits.memory_bytes()),
        "nano_cpus" => Some(limits.nano_cpus()),
        "health_tmpfs_bytes" => Some(limits.health_tmpfs_bytes()),
        "pids_limit" => Some(limits.pids_limit()),
        "startup_timeout_seconds" => Some(limits.startup_timeout_seconds()),
        "health_max_age_seconds" => Some(limits.health_max_age_seconds()),
        "stop_timeout_seconds" => Some(limits.stop_timeout_seconds()),
        _ => None,
    }
}

#[test]
fn errors_and_debug_are_value_free_for_synthetic_owner_and_path_values() {
    const SENTINEL: &str = "SYNTHETIC_SECRET_SENTINEL_WP5C3A";
    const PATH_SENTINEL: &str = "/synthetic/private/PATH_SENTINEL_WP5C3A";
    const OWNER_SENTINEL: &str = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";

    let checked = checked_value(|value| {
        value["entitlement_reference"] = json!(SENTINEL);
        value["historical_root"] = json!(PATH_SENTINEL);
        value["owner"] = json!(OWNER_SENTINEL);
    });
    let debug = format!("{checked:?}");
    let limits_debug = format!("{:?}", checked.limits());
    assert_eq!(debug, "SimulationProcessConfig");
    assert_eq!(limits_debug, "SimulationProcessLimits");

    let mut invalid = valid_baseline();
    invalid["entitlement_reference"] = json!(SENTINEL);
    invalid["historical_root"] = json!(PATH_SENTINEL);
    invalid["owner"] = json!(OWNER_SENTINEL);
    invalid["unexpected"] = json!(SENTINEL);
    let raw = serde_json::to_vec(&invalid).unwrap();
    let matching_digest = raw_sha256(&raw);
    let error = SimulationProcessConfig::from_json(&raw, TEST_COMMIT, &matching_digest)
        .expect_err("unknown key rejects the sentinel-bearing config");
    let diagnostic = format!("{error:?} {error}");
    let raw_text = String::from_utf8(raw).unwrap();
    assert_eq!(error, SimulationProcessConfigError::Invalid);
    for marker in [SENTINEL, PATH_SENTINEL, OWNER_SENTINEL, raw_text.as_str()] {
        assert!(!debug.contains(marker));
        assert!(!limits_debug.contains(marker));
        assert!(!diagnostic.contains(marker));
    }
}
