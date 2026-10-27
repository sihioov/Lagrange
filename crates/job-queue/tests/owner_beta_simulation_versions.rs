//! Synthetic queue/pair boundary regressions only. These fixtures reconstruct
//! fixed calendar bindings and checked ledgers; they never manufacture an
//! ApprovedHistoricalPriceOnlyArtifact or claim factor-readiness proof.

use std::collections::BTreeMap;

use domain::{ContentHash, TradingDate};
use job_queue::{
    owner_beta::{
        OwnerBetaStrategySnapshot,
        simulation::{
            APPROVED_BARS_SHA256, APPROVED_CONTRACT_SHA256, CostProfile, EngineIdentity,
            v2::{
                V2FactorDefinition, V2InputIdentity, V2Ledger, V2LedgerRun, V2MarketSession,
                V2Schedule, V2SimulationPair, V2SimulationResult, V2Strategy, V2StrategyIdentity,
                V2TargetAllocation,
            },
        },
        simulation_execution::{
            SimulationContractVersion, SimulationExecutionError, SimulationJobInput,
            V2SimulationJobInput, VersionedCheckedSimulationPair, VersionedSimulationJobInput,
            VersionedSimulationResultRef, check_stored_versioned_simulation_pair,
        },
    },
    resolver::ResolvedConfig,
};
use market_data::{KR_ETF_CORE_SYMBOLS, range_normalize::ExpectedRangeSessions};
use serde_json::{Value, json};
use uuid::Uuid;

const V2_JOB_SCHEMA: &str = "owner-beta-price-only-simulation-job-v2";
const V1_JOB_SCHEMA: &str = "owner-beta-price-only-simulation-job-v1";

fn hash(byte: u8) -> String {
    format!("sha256:{byte:064x}")
}

fn snapshot(strategy: V2Strategy) -> OwnerBetaStrategySnapshot {
    let config = serde_json::from_str(strategy.default_config_json()).expect("default JSON");
    OwnerBetaStrategySnapshot::from_resolved_config(&ResolvedConfig {
        strategy_id: strategy.as_str().to_owned(),
        strategy_version: "1.0.0".to_owned(),
        config,
    })
    .expect("default snapshot")
}

fn snapshot_for(strategy_id: &str, version: &str, config: Value) -> OwnerBetaStrategySnapshot {
    OwnerBetaStrategySnapshot::from_resolved_config(&ResolvedConfig {
        strategy_id: strategy_id.to_owned(),
        strategy_version: version.to_owned(),
        config,
    })
    .expect("synthetic snapshot")
}

fn approved_sessions() -> Vec<TradingDate> {
    let start = TradingDate::parse("2016-08-29").expect("approved start");
    let end = TradingDate::parse("2026-08-28").expect("approved end");
    ExpectedRangeSessions::approved_xkrx(start, end)
        .expect("fixed XKRX range")
        .sessions
}

fn input_value(
    strategy: OwnerBetaStrategySnapshot,
    evaluation_start: &str,
    evaluation_end: &str,
    schema: &str,
) -> Value {
    json!({
        "schema": schema,
        "run_id": Uuid::from_u128(1),
        "strategy_config_id": Uuid::from_u128(2),
        "evaluation_start": evaluation_start,
        "evaluation_end": evaluation_end,
        "pins": {
            "candidate_content_sha256": "sha256:b0fb82f6a580f3def13a1e1e34bea68e30d95dc720306e7ee54b6c3199cf402d",
            "artifact_manifest_sha256": "sha256:53587f32ce67ae1a8488b9c00096465185d03e35b1b38a04f39ed5462da7f6b6",
            "stage5_manifest_sha256": "sha256:5b2900594e03e322d4e81adaa79fc6c40ae70035213786972ed5cb7a22870d2a",
            "action_manifest_sha256": "sha256:080d38142a6506f741114eb75a77a36c23c2554a0d39eba8843ff62bfb484550",
            "approval_registry_sha256": "sha256:5d3aa2b354d8c0c51d0d7d029e9fd3f92e0570fe074262bfc900829a5a2bb707"
        },
        "contract_document_sha256": if schema == V1_JOB_SCHEMA {
            APPROVED_CONTRACT_SHA256
        } else {
            job_queue::owner_beta::simulation::v2::APPROVED_CONTRACT_SHA256
        },
        "bars_sha256": APPROVED_BARS_SHA256,
        "strategy": strategy
    })
}

fn v2_input_bytes(strategy: V2Strategy, start_index: usize, end_index: usize) -> Vec<u8> {
    let sessions = approved_sessions();
    serde_json::to_vec(&input_value(
        snapshot(strategy),
        &sessions[start_index].to_iso(),
        &sessions[end_index].to_iso(),
        V2_JOB_SCHEMA,
    ))
    .expect("v2 input bytes")
}

fn v2_input(strategy: V2Strategy, start_index: usize, end_index: usize) -> V2SimulationJobInput {
    V2SimulationJobInput::from_json(&v2_input_bytes(strategy, start_index, end_index))
        .expect("fixed calendar-bound v2 input")
}

fn v1_input_bytes() -> Vec<u8> {
    let strategy = OwnerBetaStrategySnapshot::from_resolved_config(&ResolvedConfig {
        strategy_id: "trend_following".to_owned(),
        strategy_version: "1.0.0".to_owned(),
        config: json!({"benchmark_instrument":"069500.KRX","fast_ma":50,"slow_ma":200}),
    })
    .expect("v1 strategy snapshot");
    serde_json::to_vec(&input_value(
        strategy,
        "2026-08-27",
        "2026-08-28",
        V1_JOB_SCHEMA,
    ))
    .expect("v1 input bytes")
}

fn engine(marker: char) -> EngineIdentity {
    EngineIdentity::new(
        &marker.to_string().repeat(40),
        &format!("sha256:{}", marker.to_string().repeat(64)),
        "rustc-test",
        "x86_64-unknown-linux-gnu",
        "test",
    )
    .expect("synthetic declared engine")
}

fn input_bytes_with_snapshot(mut value: Value, strategy: OwnerBetaStrategySnapshot) -> Vec<u8> {
    value["strategy"] = serde_json::to_value(strategy).expect("strategy value");
    serde_json::to_vec(&value).expect("input bytes")
}

fn replace_top_level(bytes: &[u8], field: &str, value: Value) -> Vec<u8> {
    let mut wire: Value = serde_json::from_slice(bytes).expect("fixture JSON");
    wire[field] = value;
    serde_json::to_vec(&wire).expect("mutated JSON")
}

fn v2_strategy_identity(strategy: V2Strategy) -> V2StrategyIdentity {
    let config: Value = serde_json::from_str(strategy.default_config_json()).expect("default");
    let mut tagged = BTreeMap::new();
    tagged.insert("config_json", config);
    tagged.insert(
        "schema",
        Value::String(
            job_queue::owner_beta::input::OWNER_BETA_STRATEGY_CONFIG_SNAPSHOT_SCHEMA.to_owned(),
        ),
    );
    tagged.insert("strategy_id", Value::String(strategy.as_str().to_owned()));
    tagged.insert("strategy_version", Value::String("1.0.0".to_owned()));
    let snapshot_hash = ContentHash::from_bytes(&serde_json::to_vec(&tagged).expect("preimage"));
    let factors = strategy
        .factor_names()
        .iter()
        .enumerate()
        .map(|(index, name)| {
            V2FactorDefinition::new(name, "factor-v1", &hash((index + 1) as u8))
                .expect("synthetic factor identity")
        })
        .collect();
    V2StrategyIdentity::from_default_snapshot(
        strategy.as_str(),
        "1.0.0",
        strategy.default_config_json(),
        snapshot_hash.as_str(),
        factors,
    )
    .expect("checked default strategy identity")
}

fn allocation() -> V2TargetAllocation {
    let mut instruments = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| format!("{symbol}.KRX"))
        .collect::<Vec<_>>();
    instruments.sort_unstable();
    let weights = instruments
        .into_iter()
        .map(|instrument| {
            let weight = if instrument == "069500.KRX" {
                1_000_000
            } else {
                0
            };
            (instrument, weight)
        })
        .collect();
    V2TargetAllocation::new(&hash(90), weights, 0).expect("synthetic target")
}

fn market(date: TradingDate) -> V2MarketSession {
    let mut instruments = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| format!("{symbol}.KRX"))
        .collect::<Vec<_>>();
    instruments.sort_unstable();
    let prices = instruments
        .into_iter()
        .map(|instrument| {
            (
                instrument,
                "100.0000".to_owned(),
                "100.0000".to_owned(),
                10_000,
            )
        })
        .collect();
    V2MarketSession::new(&date.to_iso(), prices).expect("synthetic market session")
}

fn execute(
    schedule: &V2Schedule,
    profile: CostProfile,
    sessions: Vec<V2MarketSession>,
    target: &V2TargetAllocation,
) -> V2LedgerRun {
    let mut ledger = V2Ledger::new(profile).expect("ledger");
    for session in sessions {
        let decision = schedule.decision_for_execution(session.date()).copied();
        ledger
            .open_session(session)
            .expect("open synthetic session");
        if let Some(decision) = decision {
            if schedule.strategy() == V2Strategy::TrendFollowing {
                ledger
                    .observe_trend_target(schedule, &decision, target.clone())
                    .expect("trend target observation");
            } else {
                ledger
                    .rebalance(&decision, target.clone())
                    .expect("synthetic rebalance");
            }
        }
        ledger.close_session().expect("close synthetic session");
    }
    ledger.finish(schedule).expect("checked synthetic ledger")
}

fn synthetic_result(
    input: &V2SimulationJobInput,
    profile: CostProfile,
    engine: &EngineIdentity,
) -> V2SimulationResult {
    let schedule = input.schedule().clone();
    let sessions = schedule
        .evaluation_sessions()
        .iter()
        .copied()
        .map(market)
        .collect::<Vec<_>>();
    let strategy_ledger = execute(&schedule, profile, sessions.clone(), &allocation());
    let benchmark_schedule = V2Schedule::new(
        V2Strategy::BuyAndHold,
        schedule
            .preparation_sessions()
            .iter()
            .map(TradingDate::to_iso)
            .collect(),
        schedule
            .evaluation_sessions()
            .iter()
            .map(TradingDate::to_iso)
            .collect(),
    )
    .expect("benchmark schedule");
    let benchmark_ledger = execute(&benchmark_schedule, profile, sessions, &allocation());
    V2SimulationResult::new(
        v2_strategy_identity(input.schedule().strategy()),
        engine.clone(),
        schedule,
        profile,
        strategy_ledger,
        benchmark_ledger,
    )
    .expect("checked synthetic result")
}

fn synthetic_pair_bytes(
    input: &V2SimulationJobInput,
    engine: &EngineIdentity,
) -> (Vec<u8>, Vec<u8>) {
    let illustrative = synthetic_result(input, CostProfile::Illustrative10BpPerSideV1, engine);
    let zero_cost = synthetic_result(input, CostProfile::ZeroCostReferenceV1, engine);
    V2SimulationPair::new(illustrative, zero_cost)
        .expect("checked synthetic pair")
        .canonical_json()
        .expect("canonical synthetic bodies")
}

#[test]
fn v1_roundtrip_keeps_original_wire_and_rejection_behavior() {
    let wire = v1_input_bytes();
    let input = SimulationJobInput::from_json(&wire).expect("existing v1 input");
    let serialized = serde_json::to_vec(&input).expect("v1 serializer");
    let versioned =
        VersionedSimulationJobInput::from_json(&serialized).expect("versioned v1 dispatch");
    assert_eq!(versioned.version(), SimulationContractVersion::V1);
    assert_eq!(
        serde_json::to_vec(&versioned).expect("direct wire"),
        serialized
    );
    assert_eq!(versioned.schema(), V1_JOB_SCHEMA);
    assert_eq!(versioned.strategy(), input.strategy());
    assert_eq!(
        versioned.required_dates(),
        input
            .schedule()
            .warmup_sessions()
            .iter()
            .chain(input.schedule().evaluation_sessions())
            .copied()
            .collect::<Vec<_>>()
    );
    assert_eq!(
        versioned.contract_document_sha256().as_str(),
        SimulationContractVersion::V1.contract_sha256()
    );

    let unknown = replace_top_level(&serialized, "extra", json!(true));
    assert_eq!(
        SimulationJobInput::from_json(&unknown).unwrap_err(),
        SimulationExecutionError::InputInvalid
    );
    let wrong_contract = replace_top_level(
        &serialized,
        "contract_document_sha256",
        json!(job_queue::owner_beta::simulation::v2::APPROVED_CONTRACT_SHA256),
    );
    assert!(SimulationJobInput::from_json(&wrong_contract).is_err());
}

#[test]
fn all_five_default_snapshots_match_the_frozen_preimages_and_hashes() {
    let expected = [
        (
            "buy_and_hold",
            r#"{"benchmark_instrument":"069500.KRX","rebalance_cadence":"none","target_weight":1.0}"#,
            "sha256:4de76616ae1d72646bd695d0e5aa7df7d6544c2eef5bef7ca949536c1801fc06",
            r#"{"config_json":{"benchmark_instrument":"069500.KRX","rebalance_cadence":"none","target_weight":1.0},"schema":"owner-beta-strategy-config-v1","strategy_id":"buy_and_hold","strategy_version":"1.0.0"}"#,
        ),
        (
            "trend_following",
            r#"{"benchmark_instrument":"069500.KRX","fast_ma":50,"slow_ma":200}"#,
            "sha256:90e5b4c6972678e9186d4655d76496676e7cfb8dbe473a249f062963873c04a0",
            r#"{"config_json":{"benchmark_instrument":"069500.KRX","fast_ma":50,"slow_ma":200},"schema":"owner-beta-strategy-config-v1","strategy_id":"trend_following","strategy_version":"1.0.0"}"#,
        ),
        (
            "relative_momentum",
            r#"{"lookback_months":12,"top_n":3}"#,
            "sha256:edace2510fb491084b142615a709f8ef689a5b207d7bae25ce7a886afceb8d07",
            r#"{"config_json":{"lookback_months":12,"top_n":3},"schema":"owner-beta-strategy-config-v1","strategy_id":"relative_momentum","strategy_version":"1.0.0"}"#,
        ),
        (
            "dual_momentum",
            r#"{"absolute_threshold":0.0,"lookback_months":12}"#,
            "sha256:ac6a1ac393e02952fe2db6308e5274436f766479f3e631ebe42e429a677426a6",
            r#"{"config_json":{"absolute_threshold":0.0,"lookback_months":12},"schema":"owner-beta-strategy-config-v1","strategy_id":"dual_momentum","strategy_version":"1.0.0"}"#,
        ),
        (
            "inverse_volatility",
            r#"{"max_weight":0.3,"vol_window":60}"#,
            "sha256:6252f27903f5ea0a7755ea7fc6c05a935518a2dbe44edf371a1e71f127d1aa38",
            r#"{"config_json":{"max_weight":0.3,"vol_window":60},"schema":"owner-beta-strategy-config-v1","strategy_id":"inverse_volatility","strategy_version":"1.0.0"}"#,
        ),
    ];

    for (strategy, (id, config_json, expected_hash, expected_preimage)) in
        V2Strategy::ALL.into_iter().zip(expected)
    {
        assert_eq!(strategy.as_str(), id);
        assert_eq!(strategy.default_config_json(), config_json);
        let snapshot = snapshot(strategy);
        assert_eq!(snapshot.config_sha256().as_str(), expected_hash);
        assert_eq!(
            serde_json::to_string(snapshot.config_json()).expect("canonical config"),
            config_json
        );
        let mut preimage = BTreeMap::new();
        preimage.insert("config_json", snapshot.config_json().clone());
        preimage.insert(
            "schema",
            Value::String(
                job_queue::owner_beta::input::OWNER_BETA_STRATEGY_CONFIG_SNAPSHOT_SCHEMA.to_owned(),
            ),
        );
        preimage.insert("strategy_id", Value::String(id.to_owned()));
        preimage.insert("strategy_version", Value::String("1.0.0".to_owned()));
        assert_eq!(
            serde_json::to_string(&preimage).expect("tagged preimage"),
            expected_preimage
        );
        assert_eq!(
            ContentHash::from_bytes(expected_preimage.as_bytes()).as_str(),
            expected_hash
        );
        let input = V2SimulationJobInput::from_json(&v2_input_bytes(strategy, 200, 201))
            .expect("exact default accepted");
        assert_eq!(input.strategy(), &snapshot);
    }
}

#[test]
fn v2_rejects_custom_unknown_version_forged_hash_and_numeric_aliases() {
    let base = serde_json::from_slice::<Value>(&v2_input_bytes(V2Strategy::BuyAndHold, 200, 201))
        .expect("base input");

    let mut custom_config = snapshot(V2Strategy::BuyAndHold).config_json().clone();
    custom_config["target_weight"] = json!(0.5);
    let custom = snapshot_for("buy_and_hold", "1.0.0", custom_config);
    assert!(
        V2SimulationJobInput::from_json(&input_bytes_with_snapshot(base.clone(), custom)).is_err()
    );

    let versioned = snapshot_for(
        "buy_and_hold",
        "2.0.0",
        snapshot(V2Strategy::BuyAndHold).config_json().clone(),
    );
    assert!(
        V2SimulationJobInput::from_json(&input_bytes_with_snapshot(base.clone(), versioned))
            .is_err()
    );

    let unknown = snapshot_for("other_strategy", "1.0.0", json!({"custom":1}));
    assert!(
        V2SimulationJobInput::from_json(&input_bytes_with_snapshot(base.clone(), unknown)).is_err()
    );

    let numeric_alias = snapshot_for(
        "buy_and_hold",
        "1.0.0",
        json!({"benchmark_instrument":"069500.KRX","rebalance_cadence":"none","target_weight":1}),
    );
    assert!(
        V2SimulationJobInput::from_json(&input_bytes_with_snapshot(base.clone(), numeric_alias))
            .is_err()
    );

    let mut forged: Value = serde_json::to_value(snapshot(V2Strategy::BuyAndHold)).unwrap();
    forged["config_sha256"] = json!(hash(222));
    let mut forged_input = base;
    forged_input["strategy"] = forged;
    assert!(V2SimulationJobInput::from_json(&serde_json::to_vec(&forged_input).unwrap()).is_err());
}

#[test]
fn strict_v2_wire_rejects_duplicates_unknowns_mixed_contracts_and_bad_pins() {
    let valid = v2_input_bytes(V2Strategy::BuyAndHold, 200, 201);
    let text = String::from_utf8(valid.clone()).expect("UTF-8 input");
    let duplicate = format!("{{\"schema\":\"{V2_JOB_SCHEMA}\",{}", &text[1..]);
    assert!(V2SimulationJobInput::from_json(duplicate.as_bytes()).is_err());
    let duplicate_numeric_config = text.replacen(
        "\"target_weight\":1.0",
        "\"target_weight\":1.0,\"target_weight\":1.0",
        1,
    );
    assert_ne!(duplicate_numeric_config, text);
    assert!(V2SimulationJobInput::from_json(duplicate_numeric_config.as_bytes()).is_err());

    let unknown = replace_top_level(&valid, "unrecognized", json!(true));
    assert!(V2SimulationJobInput::from_json(&unknown).is_err());

    let wrong_schema = replace_top_level(
        &valid,
        "schema",
        json!("owner-beta-price-only-simulation-job-v3"),
    );
    assert!(VersionedSimulationJobInput::from_json(&wrong_schema).is_err());

    let mixed_contract = replace_top_level(
        &valid,
        "contract_document_sha256",
        json!(APPROVED_CONTRACT_SHA256),
    );
    assert!(VersionedSimulationJobInput::from_json(&mixed_contract).is_err());

    for field in [
        "candidate_content_sha256",
        "artifact_manifest_sha256",
        "stage5_manifest_sha256",
        "action_manifest_sha256",
        "approval_registry_sha256",
    ] {
        let mut wire: Value = serde_json::from_slice(&valid).expect("valid input");
        wire["pins"][field] = json!(hash(199));
        assert!(V2SimulationJobInput::from_json(&serde_json::to_vec(&wire).unwrap()).is_err());
    }
}

#[test]
fn v2_calendar_wire_reconstructs_full_prefix_and_rejects_invalid_windows() {
    let sessions = approved_sessions();
    let start = sessions[200].to_iso();
    let end = sessions[201].to_iso();
    let bytes = v2_input_bytes(V2Strategy::RelativeMomentum, 200, 201);
    let input = V2SimulationJobInput::from_json(&bytes).expect("valid calendar binding");
    assert_eq!(input.schedule().preparation_sessions().len(), 200);
    assert_eq!(input.schedule().evaluation_sessions().len(), 2);
    assert_eq!(input.schedule().evaluation_start().to_iso(), start);
    assert_eq!(input.schedule().evaluation_end().to_iso(), end);
    let required = input.required_dates();
    assert_eq!(required.len(), 202);
    assert_eq!(required.first(), sessions.first());
    assert_eq!(required.last(), Some(&sessions[201]));
    assert_eq!(&required[..200], &sessions[..200]);
    assert_eq!(&required[200..], &sessions[200..=201]);

    let base = serde_json::from_slice::<Value>(&bytes).expect("valid input");
    for (bad_start, bad_end) in [
        (sessions[199].to_iso(), sessions[200].to_iso()),
        ("2016-09-03".to_owned(), sessions[201].to_iso()),
        ("2016-08-28".to_owned(), sessions[201].to_iso()),
        (sessions[200].to_iso(), "2026-08-31".to_owned()),
        (sessions[202].to_iso(), sessions[201].to_iso()),
    ] {
        let mut candidate = base.clone();
        candidate["evaluation_start"] = json!(bad_start);
        candidate["evaluation_end"] = json!(bad_end);
        assert!(V2SimulationJobInput::from_json(&serde_json::to_vec(&candidate).unwrap()).is_err());
    }

    // Calendar reconstruction binds dates and the necessary 200-close lower
    // bound only; it deliberately does not claim actual common readiness.
    let versioned = VersionedSimulationJobInput::from_json(&bytes).expect("versioned v2 input");
    assert_eq!(versioned.version(), SimulationContractVersion::V2);
    assert_eq!(versioned.required_dates(), required);
    assert_eq!(versioned.schema(), V2_JOB_SCHEMA);
    let serialized = serde_json::to_vec(&versioned).expect("direct versioned wire");
    assert_eq!(
        serde_json::from_slice::<Value>(&serialized).unwrap()["schema"],
        V2_JOB_SCHEMA
    );
    assert!(!String::from_utf8_lossy(&serialized).contains("\"V2\""));
    let roundtrip = VersionedSimulationJobInput::from_json(&serialized).expect("v2 wire replay");
    assert_eq!(serde_json::to_vec(&roundtrip).unwrap(), serialized);
    assert_eq!(
        versioned.contract_document_sha256().as_str(),
        SimulationContractVersion::V2.contract_sha256()
    );
    assert_eq!(SimulationContractVersion::V2.queue_schema(), V2_JOB_SCHEMA);
    assert_eq!(
        SimulationContractVersion::V2.result_schema(),
        job_queue::owner_beta::simulation::v2::RESULT_SCHEMA_VERSION
    );
    assert_eq!(
        SimulationContractVersion::V2.contract_version(),
        "owner-beta-price-only-simulation-v2-draft-1"
    );
    assert_eq!(
        SimulationContractVersion::V2.idempotency_prefix(),
        "owner-beta:price-only-simulation:v2:"
    );
}

#[test]
fn every_v2_strategy_pair_roundtrips_as_one_typed_canonical_unit() {
    let engine = engine('a');
    for strategy in V2Strategy::ALL {
        let input = v2_input(strategy, 200, 201);
        let versioned = VersionedSimulationJobInput::V2(input.clone());
        let (illustrative, zero_cost) = synthetic_pair_bytes(&input, &engine);
        let checked = VersionedCheckedSimulationPair::from_canonical(
            &illustrative,
            &zero_cost,
            &versioned,
            &engine,
        )
        .expect("complete checked v2 pair");
        assert_eq!(checked.version(), SimulationContractVersion::V2);
        assert_eq!(checked.illustrative_bytes(), illustrative.as_slice());
        assert_eq!(checked.zero_cost_bytes(), zero_cost.as_slice());
        assert_eq!(
            checked.illustrative_sha256().as_str(),
            ContentHash::from_bytes(&illustrative).as_str()
        );
        assert_eq!(
            checked.zero_cost_sha256().as_str(),
            ContentHash::from_bytes(&zero_cost).as_str()
        );
        match checked.illustrative_result() {
            VersionedSimulationResultRef::V2(result) => {
                assert_eq!(result.schedule(), input.schedule());
                assert_eq!(result.strategy().strategy(), strategy);
                assert_eq!(
                    result.cost_profile(),
                    CostProfile::Illustrative10BpPerSideV1
                );
            }
            VersionedSimulationResultRef::V1(_) => panic!("versioned result erased to v1"),
        }
    }
}

#[test]
fn versioned_pair_rejects_cost_engine_hash_schema_input_strategy_window_and_mixing() {
    let expected_engine = engine('a');
    let wrong_engine = engine('b');
    let input = v2_input(V2Strategy::BuyAndHold, 200, 201);
    let versioned = VersionedSimulationJobInput::V2(input.clone());
    let (illustrative, zero_cost) = synthetic_pair_bytes(&input, &expected_engine);

    assert!(
        VersionedCheckedSimulationPair::from_canonical(
            &zero_cost,
            &illustrative,
            &versioned,
            &expected_engine,
        )
        .is_err()
    );
    assert!(matches!(
        VersionedCheckedSimulationPair::from_canonical(
            &illustrative,
            &zero_cost,
            &versioned,
            &wrong_engine,
        ),
        Err(SimulationExecutionError::ProvenanceMismatch)
    ));

    let checked = VersionedCheckedSimulationPair::from_canonical(
        &illustrative,
        &zero_cost,
        &versioned,
        &expected_engine,
    )
    .expect("valid pair");
    assert!(
        check_stored_versioned_simulation_pair(
            &illustrative,
            &hash(255),
            &zero_cost,
            checked.zero_cost_sha256().as_str(),
            &versioned,
            &expected_engine,
        )
        .is_err()
    );

    let wrong_schema = replace_top_level(
        &illustrative,
        "schema",
        json!("owner-beta-price-only-result-v1"),
    );
    assert!(
        VersionedCheckedSimulationPair::from_canonical(
            &wrong_schema,
            &zero_cost,
            &versioned,
            &expected_engine,
        )
        .is_err()
    );

    let mut wrong_result_input: Value =
        serde_json::from_slice(&illustrative).expect("canonical result body");
    wrong_result_input["input"]["bars_sha256"] = json!(hash(254));
    let wrong_result_input = serde_json::to_vec(&wrong_result_input).expect("tampered input body");
    assert!(
        VersionedCheckedSimulationPair::from_canonical(
            &wrong_result_input,
            &zero_cost,
            &versioned,
            &expected_engine,
        )
        .is_err()
    );

    assert!(
        VersionedCheckedSimulationPair::from_canonical(
            &illustrative,
            &[],
            &versioned,
            &expected_engine,
        )
        .is_err()
    );

    let mut noncanonical = illustrative.clone();
    noncanonical.push(b' ');
    assert!(
        VersionedCheckedSimulationPair::from_canonical(
            &noncanonical,
            &zero_cost,
            &versioned,
            &expected_engine,
        )
        .is_err()
    );

    let wrong_strategy = v2_input(V2Strategy::TrendFollowing, 200, 201);
    let (trend_illustrative, trend_zero_cost) =
        synthetic_pair_bytes(&wrong_strategy, &expected_engine);
    // The schedule includes the strategy and is checked before the separate
    // strategy snapshot, so this cross-strategy pair fails the schedule binding.
    assert!(matches!(
        VersionedCheckedSimulationPair::from_canonical(
            &trend_illustrative,
            &trend_zero_cost,
            &versioned,
            &expected_engine,
        ),
        Err(SimulationExecutionError::WindowInvalid)
    ));

    let later_window = v2_input(V2Strategy::BuyAndHold, 201, 202);
    let (later_illustrative, later_zero_cost) =
        synthetic_pair_bytes(&later_window, &expected_engine);
    assert!(matches!(
        VersionedCheckedSimulationPair::from_canonical(
            &later_illustrative,
            &later_zero_cost,
            &versioned,
            &expected_engine,
        ),
        Err(SimulationExecutionError::WindowInvalid)
    ));

    let v1 = SimulationJobInput::from_json(&v1_input_bytes()).expect("valid v1 input");
    let mixed = VersionedSimulationJobInput::V1(v1);
    assert!(
        VersionedCheckedSimulationPair::from_canonical(
            &illustrative,
            &zero_cost,
            &mixed,
            &expected_engine,
        )
        .is_err()
    );

    let too_large = vec![b' '; 64 * 1024 * 1024 + 1];
    assert_eq!(
        VersionedCheckedSimulationPair::from_canonical(
            &too_large,
            &[],
            &versioned,
            &expected_engine,
        )
        .unwrap_err(),
        SimulationExecutionError::SizeLimitExceeded
    );
}

#[test]
fn stored_versioned_pair_checks_both_original_hashes_and_v1_metadata_stays_fixed() {
    let engine = engine('a');
    let input = v2_input(V2Strategy::DualMomentum, 200, 201);
    let versioned = VersionedSimulationJobInput::V2(input.clone());
    let (illustrative, zero_cost) = synthetic_pair_bytes(&input, &engine);
    let illustrative_hash = ContentHash::from_bytes(&illustrative);
    let zero_cost_hash = ContentHash::from_bytes(&zero_cost);
    let checked = check_stored_versioned_simulation_pair(
        &illustrative,
        illustrative_hash.as_str(),
        &zero_cost,
        zero_cost_hash.as_str(),
        &versioned,
        &engine,
    )
    .expect("stored pair passes both hash bindings");
    assert_eq!(
        checked.illustrative_sha256().as_str(),
        illustrative_hash.as_str()
    );
    assert_eq!(checked.zero_cost_sha256().as_str(), zero_cost_hash.as_str());
    assert!(matches!(
        check_stored_versioned_simulation_pair(
            &illustrative,
            illustrative_hash.as_str(),
            &zero_cost,
            &hash(253),
            &versioned,
            &engine,
        ),
        Err(job_queue::owner_beta::simulation_execution::SimulationPublicationError::Integrity)
    ));

    let v1_version = SimulationContractVersion::V1;
    assert_eq!(v1_version.queue_schema(), V1_JOB_SCHEMA);
    assert_eq!(
        v1_version.contract_version(),
        "owner-beta-price-only-simulation-v1-draft-1"
    );
    assert_eq!(v1_version.contract_sha256(), APPROVED_CONTRACT_SHA256);
    assert_eq!(
        v1_version.result_schema(),
        "owner-beta-price-only-result-v1"
    );
    assert_eq!(
        v1_version.idempotency_prefix(),
        "owner-beta:price-only-simulation:v1:"
    );
}
