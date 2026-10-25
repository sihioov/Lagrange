//! Synthetic contract tests only. They use production capital and checked
//! constructors; no approved provider artifact or prior private result is read.

use domain::ContentHash;
use job_queue::owner_beta::{
    OwnerBetaStrategySnapshot,
    simulation::{
        APPROVED_BARS_SHA256, APPROVED_CONTRACT_SHA256, CostProfile, EngineIdentity,
        EvaluationPriceRow, Price4, SignalSnapshot, SimulationResult, SimulationSchedule,
        StrategyIdentity, simulate_ledgers,
    },
    simulation_execution::{CheckedSimulationPair, SimulationExecutionError, SimulationJobInput},
};
use market_data::KR_ETF_CORE_SYMBOLS;
use serde_json::{Value, json};
use uuid::Uuid;

fn strategy_snapshot(fast_ma: u64) -> OwnerBetaStrategySnapshot {
    OwnerBetaStrategySnapshot::from_resolved_config(&job_queue::resolver::ResolvedConfig {
        strategy_id: "trend_following".to_owned(),
        strategy_version: "1.0.0".to_owned(),
        config: json!({
            "benchmark_instrument": "069500.KRX",
            "fast_ma": fast_ma,
            "slow_ma": 200
        }),
    })
    .expect("synthetic strategy snapshot")
}

fn input_json(
    run_id: Uuid,
    strategy_config_id: Uuid,
    evaluation_start: &str,
    evaluation_end: &str,
    strategy: OwnerBetaStrategySnapshot,
) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schema": "owner-beta-price-only-simulation-job-v1",
        "run_id": run_id,
        "strategy_config_id": strategy_config_id,
        "evaluation_start": evaluation_start,
        "evaluation_end": evaluation_end,
        "pins": {
            "candidate_content_sha256": "sha256:b0fb82f6a580f3def13a1e1e34bea68e30d95dc720306e7ee54b6c3199cf402d",
            "artifact_manifest_sha256": "sha256:53587f32ce67ae1a8488b9c00096465185d03e35b1b38a04f39ed5462da7f6b6",
            "stage5_manifest_sha256": "sha256:5b2900594e03e322d4e81adaa79fc6c40ae70035213786972ed5cb7a22870d2a",
            "action_manifest_sha256": "sha256:080d38142a6506f741114eb75a77a36c23c2554a0d39eba8843ff62bfb484550",
            "approval_registry_sha256": "sha256:5d3aa2b354d8c0c51d0d7d029e9fd3f92e0570fe074262bfc900829a5a2bb707"
        },
        "contract_document_sha256": APPROVED_CONTRACT_SHA256,
        "bars_sha256": APPROVED_BARS_SHA256,
        "strategy": strategy
    }))
    .expect("synthetic input JSON")
}

fn valid_input() -> SimulationJobInput {
    SimulationJobInput::from_json(&input_json(
        Uuid::from_u128(1),
        Uuid::from_u128(2),
        "2026-08-27",
        "2026-08-28",
        strategy_snapshot(50),
    ))
    .expect("fixed approved bindings and calendar window")
}

fn synthetic_engine(marker: char) -> EngineIdentity {
    EngineIdentity::new(
        &marker.to_string().repeat(40),
        &format!("sha256:{}", marker.to_string().repeat(64)),
        "rustc-test",
        "x86_64-unknown-linux-gnu",
        "test",
    )
    .expect("synthetic declarative engine identity")
}

fn signal_targets() -> Vec<(String, u32)> {
    let mut targets = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| {
            let instrument = format!("{symbol}.KRX");
            let weight = if instrument == "069500.KRX" {
                1_000_000
            } else {
                0
            };
            (instrument, weight)
        })
        .collect::<Vec<_>>();
    targets.sort_by(|left, right| left.0.cmp(&right.0));
    targets
}

fn synthetic_result_bytes(
    schedule: &SimulationSchedule,
    engine: &EngineIdentity,
    strategy_hash: &str,
    profile: CostProfile,
    signal_marker: char,
) -> Vec<u8> {
    let factor_hash = format!("sha256:{}", signal_marker.to_string().repeat(64));
    let target_marker = if signal_marker == 'a' { 'b' } else { 'c' };
    let target_hash = format!("sha256:{}", target_marker.to_string().repeat(64));
    let signals = schedule
        .signal_execution_pairs()
        .into_iter()
        .map(|(signal_date, _)| {
            SignalSnapshot::new(
                &signal_date.to_iso(),
                &factor_hash,
                &target_hash,
                signal_targets(),
                0,
            )
            .expect("synthetic checked signal")
        })
        .collect::<Vec<_>>();
    let prices = schedule
        .evaluation_sessions()
        .iter()
        .map(|session| {
            EvaluationPriceRow::new(
                &session.to_iso(),
                Price4::parse("100.0000").expect("open price"),
                Price4::parse("101.0000").expect("close price"),
                1_000_000,
            )
            .expect("synthetic checked price")
        })
        .collect::<Vec<_>>();
    let ledgers = simulate_ledgers(schedule, &signals, &prices)
        .expect("production-capital synthetic ledgers");
    let strategy = StrategyIdentity::trend_following_v1(
        strategy_hash,
        "1.0.0",
        &format!("sha256:{}", "d".repeat(64)),
    )
    .expect("fixed strategy identity");
    let profile_ledgers = match profile {
        CostProfile::Illustrative10BpPerSideV1 => ledgers.illustrative_10bp_per_side(),
        CostProfile::ZeroCostReferenceV1 => ledgers.zero_cost_reference(),
    };
    SimulationResult::new(
        engine.clone(),
        strategy,
        schedule.clone(),
        profile,
        signals,
        profile_ledgers.strategy().clone(),
        profile_ledgers.benchmark().clone(),
    )
    .expect("synthetic checked simulation result")
    .canonical_json()
    .expect("canonical result bytes")
}

fn synthetic_pair_bytes(
    input: &SimulationJobInput,
    engine: &EngineIdentity,
    strategy_hash: &str,
    signal_marker: char,
) -> (Vec<u8>, Vec<u8>) {
    (
        synthetic_result_bytes(
            input.schedule(),
            engine,
            strategy_hash,
            CostProfile::Illustrative10BpPerSideV1,
            signal_marker,
        ),
        synthetic_result_bytes(
            input.schedule(),
            engine,
            strategy_hash,
            CostProfile::ZeroCostReferenceV1,
            signal_marker,
        ),
    )
}

fn set_json_field(bytes: &[u8], field: &str, value: Value) -> Vec<u8> {
    let mut root: Value = serde_json::from_slice(bytes).expect("valid fixture JSON");
    root[field] = value;
    serde_json::to_vec(&root).expect("mutated fixture JSON")
}

#[test]
fn checked_pair_preserves_original_canonical_bytes_and_hashes() {
    let input = valid_input();
    let serialized_input = serde_json::to_vec(&input).expect("validated input serialization");
    assert!(!String::from_utf8_lossy(&serialized_input).contains("\"schedule\""));
    let round_tripped: SimulationJobInput =
        serde_json::from_slice(&serialized_input).expect("validated Deserialize");
    assert_eq!(round_tripped, input);

    let engine = synthetic_engine('a');
    let (illustrative, zero_cost) = synthetic_pair_bytes(
        &input,
        &engine,
        input.strategy().config_sha256().as_str(),
        'a',
    );

    let pair = CheckedSimulationPair::from_canonical(&illustrative, &zero_cost, &input, &engine)
        .expect("complete checked pair");

    assert_eq!(
        pair.illustrative_10bp_per_side_bytes(),
        illustrative.as_slice()
    );
    assert_eq!(pair.zero_cost_reference_bytes(), zero_cost.as_slice());
    assert_eq!(
        pair.illustrative_10bp_per_side_sha256().as_str(),
        ContentHash::from_bytes(&illustrative).as_str()
    );
    assert_eq!(
        pair.zero_cost_reference_sha256().as_str(),
        ContentHash::from_bytes(&zero_cost).as_str()
    );
    assert_eq!(
        pair.illustrative_10bp_per_side_result()
            .conventions()
            .cost_profile(),
        CostProfile::Illustrative10BpPerSideV1
    );
    assert_eq!(
        pair.zero_cost_reference_result()
            .conventions()
            .cost_profile(),
        CostProfile::ZeroCostReferenceV1
    );
}

#[test]
fn input_rejects_malformed_unknown_and_duplicate_keys_at_every_closed_level() {
    let good = input_json(
        Uuid::from_u128(1),
        Uuid::from_u128(2),
        "2026-08-27",
        "2026-08-28",
        strategy_snapshot(50),
    );
    assert!(SimulationJobInput::from_json(b"{").is_err());

    let top_unknown = set_json_field(&good, "unknown", json!(true));
    assert!(SimulationJobInput::from_json(&top_unknown).is_err());

    for nested in ["pins", "strategy"] {
        let mut value: Value = serde_json::from_slice(&good).expect("valid fixture JSON");
        value[nested]["unknown"] = json!(true);
        assert!(SimulationJobInput::from_json(&serde_json::to_vec(&value).unwrap()).is_err());
    }

    let mut unknown_config: Value = serde_json::from_slice(&good).expect("valid fixture JSON");
    unknown_config["strategy"]["config_json"]["extra"] = json!(1);
    assert!(SimulationJobInput::from_json(&serde_json::to_vec(&unknown_config).unwrap()).is_err());

    let text = String::from_utf8(good.clone()).expect("JSON UTF-8");
    let duplicate_top = text.replacen(
        "\"run_id\":",
        "\"run_id\":\"00000000-0000-0000-0000-000000000001\",\"run_id\":",
        1,
    );
    assert!(SimulationJobInput::from_json(duplicate_top.as_bytes()).is_err());

    let duplicate_nested = text.replacen("\"slow_ma\":200", "\"slow_ma\":200,\"slow_ma\":200", 1);
    assert!(SimulationJobInput::from_json(duplicate_nested.as_bytes()).is_err());
}

#[test]
fn input_rejects_wrong_provenance_strategy_and_invalid_windows() {
    let good = input_json(
        Uuid::from_u128(1),
        Uuid::from_u128(2),
        "2026-08-27",
        "2026-08-28",
        strategy_snapshot(50),
    );

    let mut wrong_pin: Value = serde_json::from_slice(&good).expect("valid fixture JSON");
    wrong_pin["pins"]["candidate_content_sha256"] = json!(format!("sha256:{}", "0".repeat(64)));
    assert_eq!(
        SimulationJobInput::from_json(&serde_json::to_vec(&wrong_pin).unwrap()),
        Err(SimulationExecutionError::ProvenanceMismatch)
    );

    let wrong_contract = set_json_field(
        &good,
        "contract_document_sha256",
        json!(format!("sha256:{}", "0".repeat(64))),
    );
    assert_eq!(
        SimulationJobInput::from_json(&wrong_contract),
        Err(SimulationExecutionError::ProvenanceMismatch)
    );

    let wrong_strategy = input_json(
        Uuid::from_u128(1),
        Uuid::from_u128(2),
        "2026-08-27",
        "2026-08-28",
        strategy_snapshot(51),
    );
    assert_eq!(
        SimulationJobInput::from_json(&wrong_strategy),
        Err(SimulationExecutionError::StrategyInvalid)
    );

    for (start, end) in [
        ("2026-08-28", "2026-08-27"), // reversed approved sessions
        ("2026-08-27", "2026-08-29"), // Saturday is not a session
        ("2016-08-29", "2026-08-28"), // fewer than 200 warmup sessions
        ("2026-08-28", "2026-08-28"), // fewer than two evaluation sessions
    ] {
        let invalid = input_json(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            start,
            end,
            strategy_snapshot(50),
        );
        assert!(SimulationJobInput::from_json(&invalid).is_err());
    }
}

#[test]
fn pair_rejects_missing_swapped_and_duplicate_cost_profiles() {
    let input = valid_input();
    let engine = synthetic_engine('a');
    let (illustrative, zero_cost) = synthetic_pair_bytes(
        &input,
        &engine,
        input.strategy().config_sha256().as_str(),
        'a',
    );

    assert!(CheckedSimulationPair::from_canonical(&illustrative, &[], &input, &engine).is_err());
    assert!(
        CheckedSimulationPair::from_canonical(&illustrative, &illustrative, &input, &engine)
            .is_err()
    );
    assert!(
        CheckedSimulationPair::from_canonical(&zero_cost, &illustrative, &input, &engine).is_err()
    );
    assert!(
        CheckedSimulationPair::from_canonical(&illustrative, &zero_cost, &input, &engine).is_ok()
    );
}

#[test]
fn pair_rejects_mixed_engine_schedule_signals_and_strategy() {
    let input = valid_input();
    let engine = synthetic_engine('a');
    let other_engine = synthetic_engine('b');
    let strategy_hash = input.strategy().config_sha256().as_str();
    let (illustrative, _) = synthetic_pair_bytes(&input, &engine, strategy_hash, 'a');

    let other_zero = synthetic_result_bytes(
        input.schedule(),
        &other_engine,
        strategy_hash,
        CostProfile::ZeroCostReferenceV1,
        'a',
    );
    assert!(
        CheckedSimulationPair::from_canonical(&illustrative, &other_zero, &input, &engine).is_err()
    );

    let alternate_input = SimulationJobInput::from_json(&input_json(
        Uuid::from_u128(9),
        Uuid::from_u128(8),
        "2026-08-26",
        "2026-08-28",
        strategy_snapshot(50),
    ))
    .expect("alternate approved calendar window");
    let (alternate_illustrative, alternate_zero) =
        synthetic_pair_bytes(&alternate_input, &engine, strategy_hash, 'a');
    assert!(
        CheckedSimulationPair::from_canonical(
            &alternate_illustrative,
            &alternate_zero,
            &input,
            &engine,
        )
        .is_err()
    );

    let mixed_signal_zero = synthetic_result_bytes(
        input.schedule(),
        &engine,
        strategy_hash,
        CostProfile::ZeroCostReferenceV1,
        'c',
    );
    assert!(
        CheckedSimulationPair::from_canonical(&illustrative, &mixed_signal_zero, &input, &engine,)
            .is_err()
    );

    let (wrong_strategy_illustrative, wrong_strategy_zero) =
        synthetic_pair_bytes(&input, &engine, &format!("sha256:{}", "e".repeat(64)), 'a');
    assert!(
        CheckedSimulationPair::from_canonical(
            &wrong_strategy_illustrative,
            &wrong_strategy_zero,
            &input,
            &engine,
        )
        .is_err()
    );
}

#[test]
fn pair_rejects_noncanonical_truncated_tampered_and_oversized_bytes() {
    let input = valid_input();
    let engine = synthetic_engine('a');
    let (illustrative, zero_cost) = synthetic_pair_bytes(
        &input,
        &engine,
        input.strategy().config_sha256().as_str(),
        'a',
    );

    let mut noncanonical = illustrative.clone();
    noncanonical.push(b' ');
    assert!(
        CheckedSimulationPair::from_canonical(&noncanonical, &zero_cost, &input, &engine).is_err()
    );

    assert!(
        CheckedSimulationPair::from_canonical(
            &illustrative[..illustrative.len() - 3],
            &zero_cost,
            &input,
            &engine,
        )
        .is_err()
    );

    let text = String::from_utf8(illustrative.clone()).expect("canonical UTF-8");
    let tampered = text.replacen(&"a".repeat(64), &"f".repeat(64), 1);
    assert_ne!(tampered.as_bytes(), illustrative);
    assert!(
        CheckedSimulationPair::from_canonical(tampered.as_bytes(), &zero_cost, &input, &engine,)
            .is_err()
    );

    let oversized = vec![0; 64 * 1024 * 1024 + 1];
    assert!(matches!(
        CheckedSimulationPair::from_canonical(&oversized, &[], &input, &engine),
        Err(SimulationExecutionError::SizeLimitExceeded)
    ));
}

#[test]
fn run_and_config_ids_do_not_change_checked_calculation_hashes() {
    let input_one = valid_input();
    let input_two = SimulationJobInput::from_json(&input_json(
        Uuid::from_u128(3),
        Uuid::from_u128(4),
        "2026-08-27",
        "2026-08-28",
        strategy_snapshot(50),
    ))
    .expect("same calculation binding with distinct owner IDs");
    let engine = synthetic_engine('a');
    let (illustrative, zero_cost) = synthetic_pair_bytes(
        &input_one,
        &engine,
        input_one.strategy().config_sha256().as_str(),
        'a',
    );

    let pair_one =
        CheckedSimulationPair::from_canonical(&illustrative, &zero_cost, &input_one, &engine)
            .expect("pair bound to first IDs");
    let pair_two =
        CheckedSimulationPair::from_canonical(&illustrative, &zero_cost, &input_two, &engine)
            .expect("same pair bound to second IDs");
    assert_eq!(
        pair_one.illustrative_10bp_per_side_sha256(),
        pair_two.illustrative_10bp_per_side_sha256()
    );
    assert_eq!(
        pair_one.zero_cost_reference_sha256(),
        pair_two.zero_cost_reference_sha256()
    );
}
