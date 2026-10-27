use job_queue::owner_beta::simulation_execution::{
    SimulationHealthError, SimulationHealthEvent, SimulationHealthFailure, SimulationHealthPhase,
    SimulationHealthRecord, SimulationHealthTracker, SimulationProcessConfig,
    SimulationProcessGeneration,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const TEST_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const VALID_CONFIG_JSON: &str = r###"{"schema":"owner-beta-simulation-process-v1","code_commit":"0123456789abcdef0123456789abcdef01234567","manifest_format":"LAGRANGE_RELEASE_MANIFEST_V3","manifest_sha256":"sha256:1111111111111111111111111111111111111111111111111111111111111111","image_id":"sha256:2222222222222222222222222222222222222222222222222222222222222222","receipt_sha256":"sha256:3333333333333333333333333333333333333333333333333333333333333333","binary_sha256":"sha256:4444444444444444444444444444444444444444444444444444444444444444","source_input_sha256":"sha256:5555555555555555555555555555555555555555555555555555555555555555","recipe_sha256":"sha256:6666666666666666666666666666666666666666666666666666666666666666","build_origin":"official-release-v1","package":"job-queue","bin":"owner-beta-simulation-runner","target":"x86_64-unknown-linux-musl","profile":"release","toolchain":"1.97.1","owner":"11111111-2222-3333-4444-555555555555","entitlement_reference":"fixture:synthetic-owner-rights-v1","entitlement_sha256":"sha256:7777777777777777777777777777777777777777777777777777777777777777","historical_root":"/synthetic/history/δ","reader_uid":10001,"reader_gid":10001,"paths":{"config":"/run/owner-beta-simulation/config.json","artifact_root":"/data/owner-beta-artifacts","receipt":"/usr/share/lagrange/owner-beta-simulation/artifact.json","complete":"/usr/share/lagrange/owner-beta-simulation/COMPLETE","executable":"/usr/local/bin/owner-beta-simulation-runner","worker_id":"owner-beta-simulation-runner"},"limits":{"memory_bytes":1234567,"nano_cpus":2345678,"health_tmpfs_bytes":3456789,"pids_limit":42,"startup_timeout_seconds":123,"health_max_age_seconds":234,"stop_timeout_seconds":345}}"###;
const VALID_CONFIG_JSON_SHA256: &str =
    "sha256:366b4941febbbfde0aeac8260321210888cb867e4aee9e5e6eb79f88842b6252";
const TEST_BOOT_ID: &str = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
const TEST_PID: u32 = 4242;
const TEST_START_TICKS: u64 = 987_654;

fn raw_sha256(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn checked_config() -> SimulationProcessConfig {
    SimulationProcessConfig::from_json(
        VALID_CONFIG_JSON.as_bytes(),
        TEST_COMMIT,
        VALID_CONFIG_JSON_SHA256,
    )
    .expect("the pinned compact C3A fixture is valid")
}

fn checked_config_from(bytes: &[u8]) -> SimulationProcessConfig {
    let digest = raw_sha256(bytes);
    SimulationProcessConfig::from_json(bytes, TEST_COMMIT, &digest)
        .expect("the formatted C3A fixture remains valid")
}

fn generation() -> SimulationProcessGeneration {
    SimulationProcessGeneration::new(
        TEST_PID,
        TEST_START_TICKS,
        Uuid::parse_str(TEST_BOOT_ID).expect("synthetic UUID is valid"),
    )
    .expect("synthetic process generation is valid")
}

fn tracker_in(phase: SimulationHealthPhase) -> SimulationHealthTracker {
    let config = checked_config();
    let mut tracker = SimulationHealthTracker::new(&config, generation(), 10_000);
    match phase {
        SimulationHealthPhase::Starting => {}
        SimulationHealthPhase::Ready => tracker
            .apply(SimulationHealthEvent::StartupVerified, 10_001)
            .unwrap(),
        SimulationHealthPhase::Busy => {
            tracker
                .apply(SimulationHealthEvent::StartupVerified, 10_001)
                .unwrap();
            tracker
                .apply(SimulationHealthEvent::ClaimVerified, 10_002)
                .unwrap();
        }
        SimulationHealthPhase::Draining => tracker
            .apply(SimulationHealthEvent::ShutdownRequested, 10_001)
            .unwrap(),
        SimulationHealthPhase::Failed => tracker
            .apply(
                SimulationHealthEvent::Fatal(SimulationHealthFailure::CommitUnresolved),
                10_001,
            )
            .unwrap(),
    }
    tracker
}

fn ready_record_at(
    started_boot_ms: u64,
) -> (
    SimulationProcessConfig,
    SimulationProcessGeneration,
    SimulationHealthRecord,
) {
    let config = checked_config();
    let expected_generation = generation();
    let mut tracker = SimulationHealthTracker::new(&config, expected_generation, started_boot_ms);
    tracker
        .apply(SimulationHealthEvent::StartupVerified, started_boot_ms)
        .unwrap();
    (config, expected_generation, tracker.record())
}

fn ready_json_value() -> Value {
    let (_, _, record) = ready_record_at(10_000);
    serde_json::from_slice(&record.to_json().unwrap()).unwrap()
}

fn assert_invalid_value(value: Value) {
    let config = checked_config();
    let expected_generation = generation();
    let bytes = serde_json::to_vec(&value).expect("mutated JSON value serializes");
    assert_eq!(
        SimulationHealthRecord::from_json(&bytes, &config, &expected_generation),
        Err(SimulationHealthError::Invalid),
        "mutated value should be rejected"
    );
}

fn assert_invalid_bytes(bytes: &[u8]) {
    let config = checked_config();
    let expected_generation = generation();
    assert_eq!(
        SimulationHealthRecord::from_json(bytes, &config, &expected_generation),
        Err(SimulationHealthError::Invalid)
    );
}

fn replace_once(input: &str, before: &str, after: &str) -> String {
    assert_eq!(input.matches(before).count(), 1, "marker must be unique");
    input.replacen(before, after, 1)
}

fn reordered_root_json(value: &Value) -> Vec<u8> {
    let order = [
        "failure",
        "sequence",
        "schema",
        "progress_boot_ms",
        "started_boot_ms",
        "phase",
        "generation",
        "receipt_sha256",
        "observed_boot_ms",
        "config_sha256",
    ];
    let object = value.as_object().unwrap();
    let members = order
        .iter()
        .map(|key| {
            format!(
                "{} : {}",
                serde_json::to_string(key).unwrap(),
                serde_json::to_string(&object[*key]).unwrap()
            )
        })
        .collect::<Vec<_>>();
    format!(" {{ {} }} ", members.join(" ,\n ")).into_bytes()
}

fn all_events() -> [SimulationHealthEvent; 9] {
    [
        SimulationHealthEvent::StartupVerified,
        SimulationHealthEvent::ClaimVerified,
        SimulationHealthEvent::LeaseVerified,
        SimulationHealthEvent::CycleVerified,
        SimulationHealthEvent::ShutdownRequested,
        SimulationHealthEvent::Fatal(SimulationHealthFailure::CleanupUnconfirmed),
        SimulationHealthEvent::Fatal(SimulationHealthFailure::CommitUnresolved),
        SimulationHealthEvent::Fatal(SimulationHealthFailure::RuntimeFailure),
        SimulationHealthEvent::Sample,
    ]
}

fn is_allowed(phase: SimulationHealthPhase, event: SimulationHealthEvent) -> bool {
    match phase {
        SimulationHealthPhase::Starting => matches!(
            event,
            SimulationHealthEvent::StartupVerified
                | SimulationHealthEvent::ShutdownRequested
                | SimulationHealthEvent::Fatal(_)
                | SimulationHealthEvent::Sample
        ),
        SimulationHealthPhase::Ready => matches!(
            event,
            SimulationHealthEvent::ClaimVerified
                | SimulationHealthEvent::CycleVerified
                | SimulationHealthEvent::ShutdownRequested
                | SimulationHealthEvent::Fatal(_)
                | SimulationHealthEvent::Sample
        ),
        SimulationHealthPhase::Busy => matches!(
            event,
            SimulationHealthEvent::LeaseVerified
                | SimulationHealthEvent::CycleVerified
                | SimulationHealthEvent::ShutdownRequested
                | SimulationHealthEvent::Fatal(_)
                | SimulationHealthEvent::Sample
        ),
        SimulationHealthPhase::Draining => matches!(
            event,
            SimulationHealthEvent::ShutdownRequested
                | SimulationHealthEvent::Fatal(_)
                | SimulationHealthEvent::Sample
        ),
        SimulationHealthPhase::Failed => matches!(event, SimulationHealthEvent::Sample),
    }
}

fn expected_phase(
    current: SimulationHealthPhase,
    event: SimulationHealthEvent,
) -> SimulationHealthPhase {
    match (current, event) {
        (SimulationHealthPhase::Starting, SimulationHealthEvent::StartupVerified) => {
            SimulationHealthPhase::Ready
        }
        (SimulationHealthPhase::Ready, SimulationHealthEvent::ClaimVerified) => {
            SimulationHealthPhase::Busy
        }
        (SimulationHealthPhase::Busy, SimulationHealthEvent::CycleVerified) => {
            SimulationHealthPhase::Ready
        }
        (
            SimulationHealthPhase::Starting
            | SimulationHealthPhase::Ready
            | SimulationHealthPhase::Busy,
            SimulationHealthEvent::ShutdownRequested,
        ) => SimulationHealthPhase::Draining,
        (
            SimulationHealthPhase::Starting
            | SimulationHealthPhase::Ready
            | SimulationHealthPhase::Busy
            | SimulationHealthPhase::Draining,
            SimulationHealthEvent::Fatal(_),
        ) => SimulationHealthPhase::Failed,
        _ => current,
    }
}

#[test]
fn generation_requires_all_three_valid_components() {
    let valid = generation();
    assert_eq!(valid.pid(), TEST_PID);
    assert_eq!(valid.start_time_ticks(), TEST_START_TICKS);
    assert_eq!(valid.boot_id().to_string(), TEST_BOOT_ID);
    assert_eq!(format!("{valid:?}"), "SimulationProcessGeneration");

    assert_eq!(
        SimulationProcessGeneration::new(0, 1, Uuid::parse_str(TEST_BOOT_ID).unwrap()),
        Err(SimulationHealthError::Invalid)
    );
    assert_eq!(
        SimulationProcessGeneration::new(
            i32::MAX as u32 + 1,
            1,
            Uuid::parse_str(TEST_BOOT_ID).unwrap()
        ),
        Err(SimulationHealthError::Invalid)
    );
    assert_eq!(
        SimulationProcessGeneration::new(1, 0, Uuid::parse_str(TEST_BOOT_ID).unwrap()),
        Err(SimulationHealthError::Invalid)
    );
    assert_eq!(
        SimulationProcessGeneration::new(1, 1, Uuid::nil()),
        Err(SimulationHealthError::Invalid)
    );
}

#[test]
fn checked_record_roundtrips_with_exact_bindings_and_alternate_key_order() {
    let (config, expected_generation, record) = ready_record_at(10_000);
    let canonical = record.to_json().unwrap();
    assert!(canonical.len() <= 8192);
    assert_ne!(canonical.last(), Some(&b'\n'));

    let canonical_value: Value = serde_json::from_slice(&canonical).unwrap();
    let root = canonical_value.as_object().unwrap();
    let mut root_keys = root.keys().map(String::as_str).collect::<Vec<_>>();
    root_keys.sort_unstable();
    assert_eq!(
        root_keys,
        [
            "config_sha256",
            "failure",
            "generation",
            "observed_boot_ms",
            "phase",
            "progress_boot_ms",
            "receipt_sha256",
            "schema",
            "sequence",
            "started_boot_ms",
        ]
    );
    let generation_object = root["generation"].as_object().unwrap();
    let mut generation_keys = generation_object
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    generation_keys.sort_unstable();
    assert_eq!(generation_keys, ["boot_id", "pid", "start_time_ticks"]);

    let decoded = SimulationHealthRecord::from_json(&canonical, &config, &expected_generation)
        .expect("canonical record roundtrips");
    assert_eq!(decoded, record);
    assert_eq!(decoded.phase(), SimulationHealthPhase::Ready);
    assert_eq!(decoded.failure(), None);
    assert_eq!(decoded.generation(), &expected_generation);
    assert_eq!(decoded.config_sha256(), config.config_sha256());
    assert_eq!(decoded.receipt_sha256(), config.receipt_sha256());
    assert_eq!(decoded.started_boot_ms(), 10_000);
    assert_eq!(decoded.observed_boot_ms(), 10_000);
    assert_eq!(decoded.progress_boot_ms(), Some(10_000));
    assert_eq!(decoded.sequence(), 1);
    assert_eq!(format!("{decoded:?}"), "SimulationHealthRecord");

    let reordered = reordered_root_json(&canonical_value);
    let reordered_record =
        SimulationHealthRecord::from_json(&reordered, &config, &expected_generation)
            .expect("alternate order and surrounding whitespace are accepted");
    assert_eq!(reordered_record, record);

    let pretty_config = VALID_CONFIG_JSON.replace(",\"", ", \"");
    let pretty_config_value: Value = serde_json::from_str(&pretty_config).unwrap();
    assert_eq!(
        pretty_config_value,
        serde_json::from_str::<Value>(VALID_CONFIG_JSON).unwrap()
    );
    let pretty_config = checked_config_from(pretty_config.as_bytes());
    assert_ne!(pretty_config.config_sha256(), config.config_sha256());
    assert_eq!(
        SimulationHealthRecord::from_json(&canonical, &pretty_config, &expected_generation),
        Err(SimulationHealthError::BindingMismatch)
    );
}

#[test]
fn every_phase_event_pair_matches_the_closed_transition_table_atomically() {
    let phases = [
        SimulationHealthPhase::Starting,
        SimulationHealthPhase::Ready,
        SimulationHealthPhase::Busy,
        SimulationHealthPhase::Draining,
        SimulationHealthPhase::Failed,
    ];

    for phase in phases {
        for event in all_events() {
            let mut tracker = tracker_in(phase);
            let before = tracker.record();
            let before_json = before.to_json().unwrap();
            let now = before.observed_boot_ms() + 5;
            let result = tracker.apply(event, now);
            if !is_allowed(phase, event) {
                assert_eq!(result, Err(SimulationHealthError::InvalidTransition));
                assert_eq!(tracker.record().to_json().unwrap(), before_json);
                continue;
            }

            result.expect("table-listed transition succeeds");
            let after = tracker.record();
            assert_eq!(after.phase(), expected_phase(phase, event));
            assert_eq!(after.sequence(), before.sequence() + 1);
            assert_eq!(after.observed_boot_ms(), now);
            let verified_progress = matches!(
                event,
                SimulationHealthEvent::StartupVerified
                    | SimulationHealthEvent::ClaimVerified
                    | SimulationHealthEvent::LeaseVerified
                    | SimulationHealthEvent::CycleVerified
            );
            assert_eq!(
                after.progress_boot_ms(),
                if verified_progress {
                    Some(now)
                } else {
                    before.progress_boot_ms()
                }
            );
            let expected_failure = match event {
                SimulationHealthEvent::Fatal(reason) => Some(reason),
                _ if phase == SimulationHealthPhase::Failed => before.failure(),
                _ => None,
            };
            assert_eq!(after.failure(), expected_failure);
        }
    }
}

#[test]
fn backward_time_and_invalid_events_leave_the_whole_record_unchanged() {
    let mut tracker = tracker_in(SimulationHealthPhase::Starting);
    let before = tracker.record().to_json().unwrap();
    assert_eq!(
        tracker.apply(SimulationHealthEvent::Sample, 9_999),
        Err(SimulationHealthError::ClockInvalid)
    );
    assert_eq!(tracker.record().to_json().unwrap(), before);

    assert_eq!(
        tracker.apply(SimulationHealthEvent::ClaimVerified, 10_001),
        Err(SimulationHealthError::InvalidTransition)
    );
    assert_eq!(tracker.record().to_json().unwrap(), before);

    tracker
        .apply(SimulationHealthEvent::Sample, 10_000)
        .expect("equal monotonic time is accepted");
    assert_eq!(tracker.record().sequence(), 1);
    assert_eq!(tracker.record().observed_boot_ms(), 10_000);
    assert_eq!(tracker.record().progress_boot_ms(), None);
}

#[test]
fn samples_refresh_observation_but_never_refresh_verified_progress() {
    let mut tracker = tracker_in(SimulationHealthPhase::Ready);
    let progress = tracker.record().progress_boot_ms().unwrap();
    let fresh_sample_time = progress + 234_001;
    tracker
        .apply(SimulationHealthEvent::Sample, fresh_sample_time)
        .unwrap();
    let stale_progress = tracker.record();
    assert_eq!(stale_progress.observed_boot_ms(), fresh_sample_time);
    assert_eq!(stale_progress.progress_boot_ms(), Some(progress));
    assert_eq!(
        stale_progress.check_ready_at(fresh_sample_time),
        Err(SimulationHealthError::Stale)
    );

    tracker
        .apply(SimulationHealthEvent::CycleVerified, fresh_sample_time)
        .expect("a verified cycle advances progress");
    let renewed = tracker.record();
    assert_eq!(renewed.progress_boot_ms(), Some(fresh_sample_time));
    assert_eq!(renewed.check_ready_at(fresh_sample_time), Ok(()));
}

#[test]
fn readiness_checks_both_ages_at_the_exact_boundary_and_one_millisecond_over() {
    let (_, _, record) = ready_record_at(50_000);
    assert_eq!(record.check_ready_at(284_000), Ok(()));
    assert_eq!(
        record.check_ready_at(284_001),
        Err(SimulationHealthError::Stale)
    );

    let mut tracker = tracker_in(SimulationHealthPhase::Ready);
    let progress = tracker.record().progress_boot_ms().unwrap();
    let observed = progress + 100_000;
    tracker
        .apply(SimulationHealthEvent::Sample, observed)
        .unwrap();
    let sampled = tracker.record();
    assert_eq!(sampled.check_ready_at(progress + 234_000), Ok(()));
    assert_eq!(
        sampled.check_ready_at(progress + 234_001),
        Err(SimulationHealthError::Stale)
    );
    assert_eq!(
        sampled.check_ready_at(observed - 1),
        Err(SimulationHealthError::ClockInvalid)
    );
    assert_eq!(
        sampled.check_ready_at(u64::MAX),
        Err(SimulationHealthError::Stale)
    );
}

#[test]
fn shutdown_and_fatal_states_cannot_resume_or_become_ready() {
    let mut starting = tracker_in(SimulationHealthPhase::Starting);
    starting
        .apply(SimulationHealthEvent::ShutdownRequested, 10_001)
        .unwrap();
    assert_eq!(starting.record().phase(), SimulationHealthPhase::Draining);
    assert_eq!(
        starting.record().check_ready_at(10_001),
        Err(SimulationHealthError::NotReady)
    );
    let draining_before = starting.record().to_json().unwrap();
    assert_eq!(
        starting.apply(SimulationHealthEvent::StartupVerified, 10_002),
        Err(SimulationHealthError::InvalidTransition)
    );
    assert_eq!(starting.record().to_json().unwrap(), draining_before);

    let mut busy = tracker_in(SimulationHealthPhase::Busy);
    busy.apply(SimulationHealthEvent::ShutdownRequested, 10_003)
        .unwrap();
    assert_eq!(busy.record().phase(), SimulationHealthPhase::Draining);
    assert_eq!(
        busy.record().check_ready_at(10_003),
        Err(SimulationHealthError::NotReady)
    );

    for reason in [
        SimulationHealthFailure::CleanupUnconfirmed,
        SimulationHealthFailure::CommitUnresolved,
        SimulationHealthFailure::RuntimeFailure,
    ] {
        let mut fatal = tracker_in(SimulationHealthPhase::Ready);
        fatal
            .apply(SimulationHealthEvent::Fatal(reason), 10_010)
            .unwrap();
        let failed = fatal.record();
        assert_eq!(failed.phase(), SimulationHealthPhase::Failed);
        assert_eq!(failed.failure(), Some(reason));
        assert_eq!(
            failed.check_ready_at(10_010),
            Err(SimulationHealthError::NotReady)
        );
        let before_rejection = failed.to_json().unwrap();
        assert_eq!(
            fatal.apply(
                SimulationHealthEvent::Fatal(SimulationHealthFailure::RuntimeFailure),
                10_011
            ),
            Err(SimulationHealthError::InvalidTransition)
        );
        assert_eq!(fatal.record().to_json().unwrap(), before_rejection);
        fatal.apply(SimulationHealthEvent::Sample, 10_011).unwrap();
        assert_eq!(fatal.record().failure(), Some(reason));
        assert_eq!(fatal.record().phase(), SimulationHealthPhase::Failed);
        assert_eq!(
            fatal.record().check_ready_at(10_011),
            Err(SimulationHealthError::NotReady)
        );
    }
}

#[test]
fn fatal_records_bind_each_reason_and_allow_failure_before_startup_progress() {
    let config = checked_config();
    let expected_generation = generation();
    for (reason, wire) in [
        (
            SimulationHealthFailure::CleanupUnconfirmed,
            "cleanup-unconfirmed",
        ),
        (
            SimulationHealthFailure::CommitUnresolved,
            "commit-unresolved",
        ),
        (SimulationHealthFailure::RuntimeFailure, "runtime-failure"),
    ] {
        let mut tracker = SimulationHealthTracker::new(&config, expected_generation, 20_000);
        tracker
            .apply(SimulationHealthEvent::Fatal(reason), 20_001)
            .unwrap();
        let record = tracker.record();
        assert_eq!(record.failure(), Some(reason));
        assert_eq!(record.progress_boot_ms(), None);
        let bytes = record.to_json().unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["failure"], wire);
        assert_eq!(
            SimulationHealthRecord::from_json(&bytes, &config, &expected_generation),
            Ok(record)
        );
    }
}

#[test]
fn json_root_and_generation_require_exact_objects_and_every_key() {
    let (config, expected_generation, record) = ready_record_at(30_000);
    let bytes = record.to_json().unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();

    assert_invalid_bytes(b"[]");
    assert_invalid_bytes(b"{}");
    let root_sequence = Value::Array(
        [
            "schema",
            "config_sha256",
            "receipt_sha256",
            "generation",
            "phase",
            "started_boot_ms",
            "observed_boot_ms",
            "progress_boot_ms",
            "sequence",
            "failure",
        ]
        .iter()
        .map(|key| value[*key].clone())
        .collect(),
    );
    assert_invalid_bytes(&serde_json::to_vec(&root_sequence).unwrap());

    let mut missing_failure = value.clone();
    missing_failure.as_object_mut().unwrap().remove("failure");
    assert_invalid_value(missing_failure);
    let mut missing_nullable_progress = value.clone();
    missing_nullable_progress
        .as_object_mut()
        .unwrap()
        .remove("progress_boot_ms");
    assert_invalid_value(missing_nullable_progress);
    let mut unknown_root = value.clone();
    unknown_root
        .as_object_mut()
        .unwrap()
        .insert("unexpected".into(), Value::Null);
    assert_invalid_value(unknown_root);

    let mut generation_array = value.clone();
    let original_generation = value["generation"].as_object().unwrap();
    generation_array["generation"] = Value::Array(
        ["pid", "start_time_ticks", "boot_id"]
            .iter()
            .map(|key| original_generation[*key].clone())
            .collect(),
    );
    assert_invalid_value(generation_array);
    let mut generation_missing = value.clone();
    generation_missing["generation"]
        .as_object_mut()
        .unwrap()
        .remove("pid");
    assert_invalid_value(generation_missing);
    let mut generation_unknown = value.clone();
    generation_unknown["generation"]
        .as_object_mut()
        .unwrap()
        .insert("extra".into(), json!(false));
    assert_invalid_value(generation_unknown);

    let canonical = String::from_utf8(bytes).unwrap();
    let duplicate_root = replace_once(
        &canonical,
        "\"schema\":\"owner-beta-simulation-health-v1\"",
        "\"schema\":\"owner-beta-simulation-health-v1\",\"\\u0073chema\":\"owner-beta-simulation-health-v1\"",
    );
    assert_eq!(
        SimulationHealthRecord::from_json(duplicate_root.as_bytes(), &config, &expected_generation),
        Err(SimulationHealthError::Invalid)
    );
    let duplicate_generation = replace_once(
        &canonical,
        &format!("\"pid\":{TEST_PID}"),
        &format!("\"pid\":{TEST_PID},\"\\u0070id\":{TEST_PID}"),
    );
    assert_eq!(
        SimulationHealthRecord::from_json(
            duplicate_generation.as_bytes(),
            &config,
            &expected_generation
        ),
        Err(SimulationHealthError::Invalid)
    );
}

#[test]
fn strict_json_rejects_malformed_types_and_noncanonical_numeric_tokens() {
    let (config, expected_generation, record) = ready_record_at(40_000);
    let canonical = String::from_utf8(record.to_json().unwrap()).unwrap();
    assert_invalid_bytes(&[0xff, 0xfe]);
    assert_invalid_bytes(b"{");
    assert_invalid_bytes(br#"{"bad":"\ud800"}"#);
    assert_invalid_bytes(b"{} trailing");

    let float = replace_once(&canonical, "\"sequence\":1", "\"sequence\":1.0");
    let exponent = replace_once(&canonical, "\"sequence\":1", "\"sequence\":1e0");
    let boolean = replace_once(&canonical, "\"sequence\":1", "\"sequence\":true");
    let overflow = replace_once(
        &canonical,
        "\"sequence\":1",
        "\"sequence\":18446744073709551616",
    );
    let negative = replace_once(&canonical, "\"sequence\":1", "\"sequence\":-1");
    for malformed in [float, exponent, boolean, overflow, negative] {
        assert_eq!(
            SimulationHealthRecord::from_json(malformed.as_bytes(), &config, &expected_generation),
            Err(SimulationHealthError::Invalid)
        );
    }

    let mut wrong_phase_type = ready_json_value();
    wrong_phase_type["phase"] = json!(7);
    assert_invalid_value(wrong_phase_type);
    let mut wrong_sequence_type = ready_json_value();
    wrong_sequence_type["sequence"] = json!("1");
    assert_invalid_value(wrong_sequence_type);
    let mut wrong_generation_type = ready_json_value();
    wrong_generation_type["generation"]["pid"] = json!(true);
    assert_invalid_value(wrong_generation_type);
    let mut wrong_failure_type = ready_json_value();
    wrong_failure_type["failure"] = json!(false);
    assert_invalid_value(wrong_failure_type);
}

#[test]
fn health_json_size_limit_includes_whitespace_and_output() {
    let (config, expected_generation, record) = ready_record_at(60_000);
    let canonical = record.to_json().unwrap();
    let mut at_limit = canonical.clone();
    at_limit.resize(8192, b' ');
    assert_eq!(at_limit.len(), 8192);
    assert!(SimulationHealthRecord::from_json(&at_limit, &config, &expected_generation).is_ok());

    at_limit.push(b' ');
    assert_eq!(at_limit.len(), 8193);
    assert_eq!(
        SimulationHealthRecord::from_json(&at_limit, &config, &expected_generation),
        Err(SimulationHealthError::SizeLimitExceeded)
    );
    assert!(record.to_json().unwrap().len() <= 8192);
}

#[test]
fn binding_and_each_generation_component_are_checked_without_fallbacks() {
    let (config, expected_generation, record) = ready_record_at(70_000);
    let canonical = record.to_json().unwrap();
    let value: Value = serde_json::from_slice(&canonical).unwrap();

    let mut wrong_config = value.clone();
    wrong_config["config_sha256"] = json!(format!("sha256:{}", "a".repeat(64)));
    assert_eq!(
        SimulationHealthRecord::from_json(
            &serde_json::to_vec(&wrong_config).unwrap(),
            &config,
            &expected_generation
        ),
        Err(SimulationHealthError::BindingMismatch)
    );
    let mut wrong_receipt = value.clone();
    wrong_receipt["receipt_sha256"] = json!(format!("sha256:{}", "b".repeat(64)));
    assert_eq!(
        SimulationHealthRecord::from_json(
            &serde_json::to_vec(&wrong_receipt).unwrap(),
            &config,
            &expected_generation
        ),
        Err(SimulationHealthError::BindingMismatch)
    );
    let mut malformed_digest = value.clone();
    malformed_digest["config_sha256"] = json!("SHA256:untrusted");
    assert_invalid_value(malformed_digest);

    let other_boot = Uuid::parse_str("bbbbbbbb-cccc-4ddd-8eee-ffffffffffff").unwrap();
    let mismatches = [
        SimulationProcessGeneration::new(
            TEST_PID + 1,
            TEST_START_TICKS,
            expected_generation.boot_id(),
        )
        .unwrap(),
        SimulationProcessGeneration::new(
            TEST_PID,
            TEST_START_TICKS + 1,
            expected_generation.boot_id(),
        )
        .unwrap(),
        SimulationProcessGeneration::new(TEST_PID, TEST_START_TICKS, other_boot).unwrap(),
    ];
    for mismatch in mismatches {
        assert_eq!(
            SimulationHealthRecord::from_json(&canonical, &config, &mismatch),
            Err(SimulationHealthError::GenerationMismatch)
        );
    }

    for (field, replacement) in [
        ("pid", json!(TEST_PID + 1)),
        ("start_time_ticks", json!(TEST_START_TICKS + 1)),
        ("boot_id", json!(other_boot.to_string())),
    ] {
        let mut wrong_generation = value.clone();
        wrong_generation["generation"][field] = replacement;
        assert_eq!(
            SimulationHealthRecord::from_json(
                &serde_json::to_vec(&wrong_generation).unwrap(),
                &config,
                &expected_generation
            ),
            Err(SimulationHealthError::GenerationMismatch)
        );
    }

    let mut upper_uuid = value.clone();
    upper_uuid["generation"]["boot_id"] = json!(TEST_BOOT_ID.to_ascii_uppercase());
    assert_invalid_value(upper_uuid);
    let mut nil_uuid = value.clone();
    nil_uuid["generation"]["boot_id"] = json!(Uuid::nil().to_string());
    assert_invalid_value(nil_uuid);
    let mut zero_pid = value.clone();
    zero_pid["generation"]["pid"] = json!(0);
    assert_invalid_value(zero_pid);
    let mut large_pid = value.clone();
    large_pid["generation"]["pid"] = json!(i32::MAX as u64 + 1);
    assert_invalid_value(large_pid);
    let mut zero_ticks = value.clone();
    zero_ticks["generation"]["start_time_ticks"] = json!(0);
    assert_invalid_value(zero_ticks);
}

#[test]
fn timestamp_and_phase_failure_progress_associations_are_closed() {
    let baseline = ready_json_value();

    let mut observed_before_start = baseline.clone();
    observed_before_start["observed_boot_ms"] = json!(9_999);
    assert_invalid_value(observed_before_start);
    let mut progress_before_start = baseline.clone();
    progress_before_start["progress_boot_ms"] = json!(9_999);
    assert_invalid_value(progress_before_start);
    let mut progress_after_observed = baseline.clone();
    progress_after_observed["progress_boot_ms"] = json!(10_001);
    assert_invalid_value(progress_after_observed);
    let mut ready_without_progress = baseline.clone();
    ready_without_progress["progress_boot_ms"] = Value::Null;
    assert_invalid_value(ready_without_progress);
    let mut non_starting_zero_sequence = baseline.clone();
    non_starting_zero_sequence["sequence"] = json!(0);
    assert_invalid_value(non_starting_zero_sequence);

    let mut starting_with_progress = baseline.clone();
    starting_with_progress["phase"] = json!("starting");
    starting_with_progress["sequence"] = json!(0);
    assert_invalid_value(starting_with_progress);
    let mut nonfailed_with_failure = baseline.clone();
    nonfailed_with_failure["failure"] = json!("runtime-failure");
    assert_invalid_value(nonfailed_with_failure);
    let mut failed_without_failure = baseline.clone();
    failed_without_failure["phase"] = json!("failed");
    failed_without_failure["failure"] = Value::Null;
    assert_invalid_value(failed_without_failure);
    let mut failed_with_unknown_reason = baseline;
    failed_with_unknown_reason["phase"] = json!("failed");
    failed_with_unknown_reason["failure"] = json!("provider-message-is-not-an-error");
    assert_invalid_value(failed_with_unknown_reason);
}

#[test]
fn starting_draining_and_failed_shapes_keep_nullable_keys_and_valid_sequences() {
    let config = checked_config();
    let expected_generation = generation();
    let mut starting = SimulationHealthTracker::new(&config, expected_generation, 80_000);
    for expected_sequence in [0, 1] {
        if expected_sequence == 1 {
            starting
                .apply(SimulationHealthEvent::Sample, 80_000)
                .unwrap();
        }
        let record = starting.record();
        assert_eq!(record.phase(), SimulationHealthPhase::Starting);
        assert_eq!(record.progress_boot_ms(), None);
        assert_eq!(record.sequence(), expected_sequence);
        let bytes = record.to_json().unwrap();
        assert_eq!(
            SimulationHealthRecord::from_json(&bytes, &config, &expected_generation),
            Ok(record)
        );
    }

    let mut draining = SimulationHealthTracker::new(&config, expected_generation, 80_000);
    draining
        .apply(SimulationHealthEvent::ShutdownRequested, 80_001)
        .unwrap();
    let draining_record = draining.record();
    assert_eq!(draining_record.progress_boot_ms(), None);
    assert_eq!(
        SimulationHealthRecord::from_json(
            &draining_record.to_json().unwrap(),
            &config,
            &expected_generation
        ),
        Ok(draining_record)
    );

    let mut failed = SimulationHealthTracker::new(&config, expected_generation, 80_000);
    failed
        .apply(
            SimulationHealthEvent::Fatal(SimulationHealthFailure::CleanupUnconfirmed),
            80_001,
        )
        .unwrap();
    let failed_record = failed.record();
    assert_eq!(failed_record.progress_boot_ms(), None);
    assert_eq!(
        SimulationHealthRecord::from_json(
            &failed_record.to_json().unwrap(),
            &config,
            &expected_generation
        ),
        Ok(failed_record)
    );
}

#[test]
fn type_debug_and_errors_never_disclose_input_or_binding_values() {
    let config = checked_config();
    let expected_generation = generation();
    let mut tracker = SimulationHealthTracker::new(&config, expected_generation, 90_000);
    let debug_tracker = format!("{tracker:?}");
    assert_eq!(debug_tracker, "SimulationHealthTracker");

    let record = tracker.record();
    assert_eq!(format!("{record:?}"), "SimulationHealthRecord");
    let bytes = record.to_json().unwrap();
    assert_eq!(bytes, record.to_json().unwrap());
    let sentinel = "synthetic-sensitive-sentinel";
    let bad = format!(
        "{{\"schema\":\"owner-beta-simulation-health-v1\",\"config_sha256\":\"{sentinel}\"}}"
    );
    let error = SimulationHealthRecord::from_json(bad.as_bytes(), &config, &expected_generation)
        .expect_err("the incomplete sentinel record is rejected");
    let diagnostic = format!("{error:?} {error}");
    assert!(!diagnostic.contains(sentinel));
    assert!(!format!("{record:?}").contains(config.config_sha256()));
    assert!(!format!("{tracker:?}").contains(config.receipt_sha256()));

    let oversized = format!("{sentinel}{}", "x".repeat(8192));
    let oversized_error =
        SimulationHealthRecord::from_json(oversized.as_bytes(), &config, &expected_generation)
            .expect_err("oversized input is rejected before parsing");
    assert_eq!(oversized_error, SimulationHealthError::SizeLimitExceeded);
    assert!(!format!("{oversized_error:?} {oversized_error}").contains(sentinel));

    tracker
        .apply(SimulationHealthEvent::Sample, 90_000)
        .unwrap();
    assert_eq!(tracker.record().sequence(), 1);
}
