use std::collections::{BTreeMap, BTreeSet};

use domain::ContentHash;
use job_queue::owner_beta::simulation::v2::{
    V2ContractError, V2DecisionKind, V2FactorDefinition, V2InputIdentity, V2Ledger,
    V2MarketSession, V2Schedule, V2SimulationPair, V2SimulationResult, V2Strategy,
    V2StrategyIdentity, V2TargetAllocation, V2UnfilledReason,
};
use job_queue::owner_beta::simulation::{CostProfile, EngineIdentity};
use market_data::KR_ETF_CORE_SYMBOLS;
use serde_json::{Value, json};

const A: &str = "069500.KRX";
const B: &str = "102110.KRX";

fn hash(value: u8) -> String {
    format!("sha256:{value:064x}")
}

fn snapshot_hash(strategy: V2Strategy, config_json: &str) -> String {
    let config: Value = serde_json::from_str(config_json).unwrap();
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
    ContentHash::from_bytes(&serde_json::to_vec(&tagged).unwrap())
        .as_str()
        .to_owned()
}

fn strategy_identity(strategy: V2Strategy) -> V2StrategyIdentity {
    let snapshot_hash = snapshot_hash(strategy, strategy.default_config_json());
    let factors = strategy
        .factor_names()
        .iter()
        .enumerate()
        .map(|(index, name)| {
            V2FactorDefinition::new(name, "factor-v1", &hash((index + 1) as u8)).unwrap()
        })
        .collect();
    V2StrategyIdentity::from_default_snapshot(
        strategy.as_str(),
        "1.0.0",
        strategy.default_config_json(),
        &snapshot_hash,
        factors,
    )
    .unwrap()
}

fn schedule(strategy: V2Strategy, evaluation: &[&str]) -> V2Schedule {
    let preparation = vec!["2026-01-28".to_owned(), "2026-01-29".to_owned()];
    V2Schedule::new(
        strategy,
        preparation,
        evaluation.iter().map(|date| (*date).to_owned()).collect(),
    )
    .unwrap()
}

fn allocation(weights: &[(&str, u32)]) -> V2TargetAllocation {
    allocation_with_hash(90, weights)
}

fn allocation_with_hash(hash_byte: u8, weights: &[(&str, u32)]) -> V2TargetAllocation {
    let requested = weights.iter().copied().collect::<BTreeMap<_, _>>();
    let mut instruments = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| format!("{symbol}.KRX"))
        .collect::<Vec<_>>();
    instruments.sort_unstable();
    let mut total = 0_u32;
    let weights = instruments
        .into_iter()
        .map(|instrument| {
            let weight = requested.get(instrument.as_str()).copied().unwrap_or(0);
            total += weight;
            (instrument, weight)
        })
        .collect();
    V2TargetAllocation::new(&hash(hash_byte), weights, 1_000_000 - total).unwrap()
}

fn market(
    date: &str,
    a_open: &str,
    a_close: &str,
    b_open: &str,
    b_close: &str,
    volume: u64,
) -> V2MarketSession {
    let mut instruments = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| format!("{symbol}.KRX"))
        .collect::<Vec<_>>();
    instruments.sort_unstable();
    let prices = instruments
        .into_iter()
        .map(|instrument| {
            let (open, close) = match instrument.as_str() {
                A => (a_open, a_close),
                B => (b_open, b_close),
                _ => ("20000000.0000", "20000000.0000"),
            };
            (instrument, open.to_owned(), close.to_owned(), volume)
        })
        .collect();
    V2MarketSession::new(date, prices).unwrap()
}

fn execute(
    schedule: &V2Schedule,
    profile: CostProfile,
    sessions: Vec<V2MarketSession>,
    targets: Vec<Option<V2TargetAllocation>>,
) -> job_queue::owner_beta::simulation::v2::V2LedgerRun {
    assert_eq!(sessions.len(), targets.len());
    let mut ledger = V2Ledger::new(profile).unwrap();
    for (session, target) in sessions.into_iter().zip(targets) {
        let decision = schedule.decision_for_execution(session.date()).copied();
        ledger.open_session(session).unwrap();
        if schedule.strategy() == V2Strategy::TrendFollowing {
            if let Some(decision) = decision.as_ref() {
                ledger
                    .observe_trend_target(schedule, decision, target.expect("daily trend target"))
                    .unwrap();
            } else {
                assert!(target.is_none());
            }
        } else if let Some(target) = target {
            ledger
                .rebalance(decision.as_ref().unwrap(), target)
                .unwrap();
        }
        ledger.close_session().unwrap();
    }
    ledger.finish(schedule).unwrap()
}

fn result(profile: CostProfile) -> V2SimulationResult {
    let sessions = base_market_path();
    result_with_paths(profile, sessions.clone(), sessions).unwrap()
}

fn base_market_path() -> Vec<V2MarketSession> {
    vec![
        market(
            "2026-01-30",
            "1000000.0000",
            "1000000.0000",
            "500000.0000",
            "500000.0000",
            1,
        ),
        market(
            "2026-02-02",
            "1100000.0000",
            "1100000.0000",
            "500000.0000",
            "500000.0000",
            1,
        ),
    ]
}

fn changed_unheld_hold_day(sessions: &[V2MarketSession]) -> Vec<V2MarketSession> {
    sessions
        .iter()
        .map(|session| {
            let prices = session
                .prices()
                .iter()
                .map(|price| {
                    let changed = session.date().to_string() == "2026-02-02"
                        && price.instrument_id() == "114260.KRX";
                    (
                        price.instrument_id().to_owned(),
                        if changed {
                            "20000001.0000".to_owned()
                        } else {
                            price.open().as_str().to_owned()
                        },
                        if changed {
                            "20000001.0000".to_owned()
                        } else {
                            price.close().as_str().to_owned()
                        },
                        price.volume() + if changed { 1 } else { 0 },
                    )
                })
                .collect();
            V2MarketSession::new(&session.date().to_string(), prices).unwrap()
        })
        .collect()
}

fn result_with_paths(
    profile: CostProfile,
    ledger_sessions: Vec<V2MarketSession>,
    benchmark_sessions: Vec<V2MarketSession>,
) -> Result<V2SimulationResult, V2ContractError> {
    let evaluation_schedule = schedule(V2Strategy::BuyAndHold, &["2026-01-30", "2026-02-02"]);
    let benchmark_schedule = schedule(V2Strategy::BuyAndHold, &["2026-01-30", "2026-02-02"]);
    let target = allocation(&[(A, 1_000_000)]);
    let ledger = execute(
        &evaluation_schedule,
        profile,
        ledger_sessions,
        vec![Some(target.clone()), None],
    );
    let benchmark = execute(
        &benchmark_schedule,
        profile,
        benchmark_sessions,
        vec![Some(target), None],
    );
    V2SimulationResult::new(
        strategy_identity(V2Strategy::BuyAndHold),
        EngineIdentity::new(
            &"a".repeat(40),
            &hash(91),
            "rustc-1.97.1",
            "x86_64-unknown-linux-gnu",
            "release",
        )
        .unwrap(),
        evaluation_schedule,
        profile,
        ledger,
        benchmark,
    )
}

fn trend_result() -> V2SimulationResult {
    let trend_schedule = schedule(V2Strategy::TrendFollowing, &["2026-01-30", "2026-02-02"]);
    let benchmark_schedule = schedule(V2Strategy::BuyAndHold, &["2026-01-30", "2026-02-02"]);
    let sessions = vec![
        market(
            "2026-01-30",
            "1000000.0000",
            "1000000.0000",
            "500000.0000",
            "500000.0000",
            1,
        ),
        market(
            "2026-02-02",
            "1000000.0000",
            "1000000.0000",
            "500000.0000",
            "500000.0000",
            1,
        ),
    ];
    let benchmark_target = allocation(&[(A, 1_000_000)]);
    let ledger = execute(
        &trend_schedule,
        CostProfile::Illustrative10BpPerSideV1,
        sessions.clone(),
        vec![
            Some(allocation_with_hash(90, &[(A, 1_000_000)])),
            Some(allocation_with_hash(91, &[(A, 1_000_000)])),
        ],
    );
    let benchmark = execute(
        &benchmark_schedule,
        CostProfile::Illustrative10BpPerSideV1,
        sessions,
        vec![Some(benchmark_target), None],
    );
    V2SimulationResult::new(
        strategy_identity(V2Strategy::TrendFollowing),
        EngineIdentity::new(
            &"a".repeat(40),
            &hash(91),
            "rustc-1.97.1",
            "x86_64-unknown-linux-gnu",
            "release",
        )
        .unwrap(),
        trend_schedule,
        CostProfile::Illustrative10BpPerSideV1,
        ledger,
        benchmark,
    )
    .unwrap()
}

#[test]
fn only_the_five_exact_default_strategy_snapshots_are_admitted() {
    for strategy in V2Strategy::ALL {
        let identity = strategy_identity(strategy);
        assert_eq!(identity.strategy(), strategy);
        assert_eq!(identity.version(), "1.0.0");
        assert_eq!(
            identity.canonical_config_json(),
            strategy.default_config_json()
        );
    }

    let err = V2StrategyIdentity::from_default_snapshot(
        "custom_strategy",
        "1.0.0",
        "{}",
        &hash(1),
        Vec::new(),
    );
    assert_eq!(err.unwrap_err(), V2ContractError::InvalidIdentity);

    let integer_default =
        r#"{"benchmark_instrument":"069500.KRX","rebalance_cadence":"none","target_weight":1}"#;
    let err = V2StrategyIdentity::from_default_snapshot(
        "buy_and_hold",
        "1.0.0",
        integer_default,
        &snapshot_hash(V2Strategy::BuyAndHold, integer_default),
        Vec::new(),
    );
    assert_eq!(err.unwrap_err(), V2ContractError::InvalidIdentity);

    let signed_zero = r#"{"absolute_threshold":-0.0,"lookback_months":12}"#;
    let err = V2StrategyIdentity::from_default_snapshot(
        "dual_momentum",
        "1.0.0",
        signed_zero,
        &snapshot_hash(V2Strategy::DualMomentum, signed_zero),
        vec![V2FactorDefinition::new("return_12m", "factor-v1", &hash(1)).unwrap()],
    );
    assert_eq!(err.unwrap_err(), V2ContractError::InvalidIdentity);

    let unknown_field = r#"{"benchmark_instrument":"069500.KRX","rebalance_cadence":"none","target_weight":1.0,"custom":true}"#;
    let err = V2StrategyIdentity::from_default_snapshot(
        "buy_and_hold",
        "1.0.0",
        unknown_field,
        &hash(1),
        Vec::new(),
    );
    assert_eq!(err.unwrap_err(), V2ContractError::InvalidIdentity);

    let err = V2StrategyIdentity::from_default_snapshot(
        "buy_and_hold",
        "1.0.0",
        r#"{"benchmark_instrument":"069500.KRX","target_weight":1.0,"target_weight":0.5}"#,
        &hash(1),
        Vec::new(),
    );
    assert_eq!(err.unwrap_err(), V2ContractError::InvalidIdentity);
}

#[test]
fn schedule_uses_actual_common_preparation_and_strategy_calendar_rules() {
    let buy_hold = schedule(V2Strategy::BuyAndHold, &["2026-01-30", "2026-02-02"]);
    assert_eq!(buy_hold.preparation_session_count(), 2);
    assert_eq!(buy_hold.first_signal_date().to_string(), "2026-01-29");
    assert_eq!(buy_hold.decisions().len(), 1);
    assert_eq!(
        buy_hold.decisions()[0].kind(),
        V2DecisionKind::InitialCommonSession
    );

    let trend = schedule(
        V2Strategy::TrendFollowing,
        &["2026-01-30", "2026-02-02", "2026-02-03"],
    );
    assert_eq!(trend.decisions().len(), 3);
    assert!(
        trend.decisions()[1..]
            .iter()
            .all(|decision| decision.kind() == V2DecisionKind::DailyTrendTransitionCheck)
    );

    let monthly = schedule(
        V2Strategy::RelativeMomentum,
        &["2026-01-30", "2026-02-02", "2026-02-27", "2026-03-02"],
    );
    assert_eq!(monthly.decisions().len(), 3);
    assert_eq!(
        monthly.decisions()[1].signal_date().to_string(),
        "2026-01-30"
    );
    assert_eq!(
        monthly.decisions()[1].execution_date().to_string(),
        "2026-02-02"
    );
    assert_eq!(
        monthly.decisions()[2].signal_date().to_string(),
        "2026-02-27"
    );
    assert_eq!(
        monthly.decisions()[2].execution_date().to_string(),
        "2026-03-02"
    );
}

#[test]
fn target_inventory_requires_all_eleven_sorted_members_and_exact_ppm_total() {
    let all = allocation(&[(A, 500_000), (B, 500_000)]);
    assert_eq!(all.weights().len(), 11);
    assert_eq!(all.cash_weight_ppm(), 0);
    assert_ne!(all.target_sha256(), all.allocation_sha256());
    assert!(all.lineage_sha256().as_str().starts_with("sha256:"));
    let capped_signal_target = allocation(&[(A, 300_000), (B, 300_000), ("114260.KRX", 300_000)]);
    assert_eq!(capped_signal_target.cash_weight_ppm(), 100_000);

    let mut only_one = vec![(A.to_owned(), 1_000_000)];
    assert_eq!(
        V2TargetAllocation::new(&hash(1), std::mem::take(&mut only_one), 0).unwrap_err(),
        V2ContractError::InvalidValue
    );
    assert_eq!(
        V2TargetAllocation::new(
            &hash(1),
            allocation(&[(A, 500_000), (B, 500_000)])
                .weights()
                .iter()
                .map(|item| (item.instrument_id().to_owned(), item.weight_ppm()))
                .collect(),
            1,
        )
        .unwrap_err(),
        V2ContractError::InvalidValue
    );
}

#[test]
fn production_capital_ledger_matches_no_cost_and_cost_inclusive_sizing() {
    let schedule = schedule(V2Strategy::BuyAndHold, &["2026-01-30", "2026-02-02"]);
    let target = allocation(&[(A, 500_000), (B, 500_000)]);
    let sessions = vec![
        market(
            "2026-01-30",
            "1000000.0000",
            "1000000.0000",
            "500000.0000",
            "500000.0000",
            10,
        ),
        market(
            "2026-02-02",
            "1000000.0000",
            "1000000.0000",
            "500000.0000",
            "500000.0000",
            10,
        ),
    ];

    let no_cost = execute(
        &schedule,
        CostProfile::ZeroCostReferenceV1,
        sessions.clone(),
        vec![Some(target.clone()), None],
    );
    assert_eq!(no_cost.ending_positions()[0].quantity(), 5);
    assert_eq!(no_cost.ending_positions()[1].quantity(), 10);
    assert_eq!(no_cost.metrics().ending_cash().as_str(), "0.0000");
    assert_eq!(no_cost.metrics().cumulative_costs().as_str(), "0.0000");

    let with_cost = execute(
        &schedule,
        CostProfile::Illustrative10BpPerSideV1,
        sessions,
        vec![Some(target), None],
    );
    assert_eq!(with_cost.ending_positions()[0].quantity(), 4);
    assert_eq!(with_cost.ending_positions()[1].quantity(), 9);
    assert_eq!(with_cost.metrics().ending_cash().as_str(), "1491500.0000");
    assert_eq!(with_cost.metrics().cumulative_costs().as_str(), "8500.0000");
    assert!(with_cost.rebalances()[0].orders().iter().all(|order| {
        order.filled_quantity() < order.requested_quantity()
            && order.unfilled_reason() == Some(V2UnfilledReason::InsufficientCostInclusiveBudget)
    }));
}

#[test]
fn sells_precede_proportional_buys_and_monthly_same_target_rebalances_drift() {
    let rotation_schedule = schedule(V2Strategy::RelativeMomentum, &["2026-01-30", "2026-02-02"]);
    let sessions = vec![
        market(
            "2026-01-30",
            "999000.0000",
            "999000.0000",
            "500000.0000",
            "500000.0000",
            10,
        ),
        market(
            "2026-02-02",
            "999000.0000",
            "999000.0000",
            "500000.0000",
            "500000.0000",
            10,
        ),
    ];
    let rotated = execute(
        &rotation_schedule,
        CostProfile::Illustrative10BpPerSideV1,
        sessions,
        vec![
            Some(allocation(&[(A, 1_000_000)])),
            Some(allocation(&[(B, 1_000_000)])),
        ],
    );
    assert_eq!(rotated.rebalances().len(), 2);
    let second = &rotated.rebalances()[1];
    assert_eq!(
        second.orders()[0].side(),
        job_queue::owner_beta::simulation::v2::V2OrderSide::Sell
    );
    assert_eq!(second.orders()[0].instrument_id(), A);
    assert_eq!(
        second.orders()[1].side(),
        job_queue::owner_beta::simulation::v2::V2OrderSide::Buy
    );
    assert_eq!(second.orders()[1].instrument_id(), B);
    assert_eq!(rotated.metrics().ending_cash().as_str(), "470520.0000");

    let monthly_schedule = schedule(
        V2Strategy::RelativeMomentum,
        &["2026-01-30", "2026-02-02", "2026-02-27", "2026-03-02"],
    );
    let monthly_sessions = vec![
        market(
            "2026-01-30",
            "1000000.0000",
            "1000000.0000",
            "500000.0000",
            "500000.0000",
            10,
        ),
        market(
            "2026-02-02",
            "1000000.0000",
            "1500000.0000",
            "500000.0000",
            "500000.0000",
            10,
        ),
        market(
            "2026-02-27",
            "1500000.0000",
            "1500000.0000",
            "500000.0000",
            "500000.0000",
            10,
        ),
        market(
            "2026-03-02",
            "1500000.0000",
            "1500000.0000",
            "500000.0000",
            "500000.0000",
            10,
        ),
    ];
    let same_target = allocation(&[(A, 500_000), (B, 500_000)]);
    let drift = execute(
        &monthly_schedule,
        CostProfile::Illustrative10BpPerSideV1,
        monthly_sessions,
        vec![
            Some(same_target.clone()),
            Some(same_target.clone()),
            None,
            Some(same_target),
        ],
    );
    assert_eq!(drift.rebalances().len(), 3);
    assert_eq!(drift.target_observations().len(), 3);
    assert!(drift.rebalances()[1].orders().is_empty());
    assert_eq!(
        drift
            .target_lineage_for_session(domain::TradingDate::parse("2026-02-27").unwrap())
            .unwrap()
            .target_sha256()
            .as_str(),
        hash(90).as_str()
    );
    let mid_month_positions = drift
        .positions_at_close(domain::TradingDate::parse("2026-02-27").unwrap())
        .unwrap();
    assert_eq!(mid_month_positions[0].quantity(), 4);
    assert_eq!(mid_month_positions[1].quantity(), 9);
    assert_eq!(drift.ending_positions()[0].quantity(), 3);
    assert_eq!(drift.ending_positions()[1].quantity(), 11);
    assert_eq!(drift.metrics().ending_cash().as_str(), "1989000.0000");
}

#[test]
fn zero_volume_rejects_filled_orders() {
    let simple = schedule(V2Strategy::BuyAndHold, &["2026-01-30", "2026-02-02"]);
    let mut ledger = V2Ledger::new(CostProfile::ZeroCostReferenceV1).unwrap();
    let day = market(
        "2026-01-30",
        "1000000.0000",
        "1000000.0000",
        "500000.0000",
        "500000.0000",
        0,
    );
    ledger.open_session(day).unwrap();
    assert_eq!(
        ledger
            .rebalance(&simple.decisions()[0], allocation(&[(A, 1_000_000)]),)
            .unwrap_err(),
        V2ContractError::InvalidLedger
    );
}

#[test]
fn trend_observes_daily_targets_but_trades_only_on_valid_allocation_transitions() {
    let trend = schedule(
        V2Strategy::TrendFollowing,
        &["2026-01-30", "2026-02-02", "2026-02-03", "2026-02-04"],
    );
    let sessions = vec![
        market(
            "2026-01-30",
            "10000000.0000",
            "10000000.0000",
            "5000000.0000",
            "5000000.0000",
            1,
        ),
        market(
            "2026-02-02",
            "5000000.0000",
            "5000000.0000",
            "5000000.0000",
            "5000000.0000",
            1,
        ),
        market(
            "2026-02-03",
            "5000000.0000",
            "5000000.0000",
            "5000000.0000",
            "5000000.0000",
            1,
        ),
        market(
            "2026-02-04",
            "5000000.0000",
            "5000000.0000",
            "5000000.0000",
            "5000000.0000",
            1,
        ),
    ];
    let run = execute(
        &trend,
        CostProfile::Illustrative10BpPerSideV1,
        sessions,
        vec![
            Some(allocation_with_hash(90, &[(A, 1_000_000)])),
            Some(allocation_with_hash(91, &[(A, 1_000_000)])),
            Some(allocation_with_hash(92, &[])),
            Some(allocation_with_hash(93, &[(A, 1_000_000)])),
        ],
    );
    assert_eq!(run.target_observations().len(), 4);
    assert_ne!(
        run.target_observations()[0].target().target_sha256(),
        run.target_observations()[1].target().target_sha256()
    );
    assert_eq!(run.rebalances().len(), 3);
    assert_eq!(run.rebalances()[0].orders().len(), 1);
    assert_eq!(run.rebalances()[0].orders()[0].filled_quantity(), 0);
    assert_eq!(
        run.rebalances()[0].orders()[0].unfilled_reason(),
        Some(V2UnfilledReason::InsufficientCostInclusiveBudget)
    );
    assert_eq!(
        run.rebalances()[1].execution_date().to_string(),
        "2026-02-03"
    );
    assert!(run.rebalances()[1].orders().is_empty());
    assert_eq!(
        run.rebalances()[2].execution_date().to_string(),
        "2026-02-04"
    );
    assert_eq!(run.rebalances()[2].orders()[0].filled_quantity(), 1);
    assert_eq!(
        run.target_lineage_for_session(domain::TradingDate::parse("2026-02-02").unwrap())
            .unwrap()
            .target_sha256()
            .as_str(),
        hash(90).as_str()
    );
    assert_eq!(run.metrics().fill_count(), 1);
}

#[test]
fn trend_unchanged_allocation_does_not_repair_price_drift() {
    let trend = schedule(V2Strategy::TrendFollowing, &["2026-01-30", "2026-02-02"]);
    let run = execute(
        &trend,
        CostProfile::ZeroCostReferenceV1,
        vec![
            market(
                "2026-01-30",
                "3000000.0000",
                "3000000.0000",
                "5000000.0000",
                "5000000.0000",
                10,
            ),
            market(
                "2026-02-02",
                "500000.0000",
                "500000.0000",
                "5000000.0000",
                "5000000.0000",
                10,
            ),
        ],
        vec![
            Some(allocation_with_hash(90, &[(A, 1_000_000)])),
            Some(allocation_with_hash(91, &[(A, 1_000_000)])),
        ],
    );
    assert_eq!(run.target_observations().len(), 2);
    assert_eq!(run.rebalances().len(), 1);
    assert_eq!(run.ending_positions()[0].quantity(), 3);
    assert_eq!(run.metrics().ending_cash().as_str(), "1000000.0000");
    assert_eq!(
        run.target_lineage_for_session(domain::TradingDate::parse("2026-02-02").unwrap())
            .unwrap()
            .target_sha256()
            .as_str(),
        hash(90).as_str()
    );
}

#[test]
fn trend_rejects_fractional_foreign_duplicate_off_calendar_and_missing_observations() {
    let trend = schedule(V2Strategy::TrendFollowing, &["2026-01-30", "2026-02-02"]);
    let mut fractional = V2Ledger::new(CostProfile::ZeroCostReferenceV1).unwrap();
    fractional
        .open_session(market(
            "2026-01-30",
            "1000000.0000",
            "1000000.0000",
            "500000.0000",
            "500000.0000",
            1,
        ))
        .unwrap();
    assert_eq!(
        fractional
            .observe_trend_target(
                &trend,
                &trend.decisions()[0],
                allocation(&[(A, 500_000), (B, 500_000)]),
            )
            .unwrap_err(),
        V2ContractError::InvalidLedger
    );
    assert_eq!(
        fractional
            .observe_trend_target(&trend, &trend.decisions()[0], allocation(&[(B, 1_000_000)]),)
            .unwrap_err(),
        V2ContractError::InvalidLedger
    );

    let other_prefix = V2Schedule::new(
        V2Strategy::TrendFollowing,
        vec!["2026-01-27".to_owned(), "2026-01-28".to_owned()],
        vec!["2026-01-30".to_owned(), "2026-02-02".to_owned()],
    )
    .unwrap();
    assert_eq!(
        fractional
            .observe_trend_target(
                &trend,
                &other_prefix.decisions()[0],
                allocation(&[(A, 1_000_000)]),
            )
            .unwrap_err(),
        V2ContractError::InvalidLedger
    );
    assert!(
        fractional
            .observe_trend_target(&trend, &trend.decisions()[0], allocation(&[(A, 1_000_000)]),)
            .unwrap()
    );
    assert_eq!(
        fractional
            .observe_trend_target(&trend, &trend.decisions()[0], allocation(&[(A, 1_000_000)]),)
            .unwrap_err(),
        V2ContractError::InvalidLedger
    );

    let mut missing = V2Ledger::new(CostProfile::ZeroCostReferenceV1).unwrap();
    for (index, date) in ["2026-01-30", "2026-02-02"].iter().enumerate() {
        missing
            .open_session(market(
                date,
                "1000000.0000",
                "1000000.0000",
                "500000.0000",
                "500000.0000",
                1,
            ))
            .unwrap();
        if index == 0 {
            missing
                .observe_trend_target(&trend, &trend.decisions()[0], allocation(&[(A, 1_000_000)]))
                .unwrap();
        }
        missing.close_session().unwrap();
    }
    assert_eq!(
        missing.finish(&trend).unwrap_err(),
        V2ContractError::InvalidLedger
    );
}

#[test]
fn per_side_cost_rounds_up_once_to_the_money_scale() {
    let schedule = schedule(V2Strategy::BuyAndHold, &["2026-01-30", "2026-02-02"]);
    let run = execute(
        &schedule,
        CostProfile::Illustrative10BpPerSideV1,
        vec![
            market(
                "2026-01-30",
                "1.0001",
                "1.0001",
                "20000000.0000",
                "20000000.0000",
                1,
            ),
            market(
                "2026-02-02",
                "1.0001",
                "1.0001",
                "20000000.0000",
                "20000000.0000",
                1,
            ),
        ],
        vec![Some(allocation(&[(A, 1)])), None],
    );
    assert_eq!(run.rebalances()[0].orders()[0].filled_quantity(), 9);
    assert_eq!(run.rebalances()[0].orders()[0].cost().as_str(), "0.0091");
}

#[test]
fn result_pair_roundtrips_and_rejects_duplicate_unknown_or_tampered_claims() {
    let pair = V2SimulationPair::new(
        result(CostProfile::Illustrative10BpPerSideV1),
        result(CostProfile::ZeroCostReferenceV1),
    )
    .unwrap();
    let (with_cost, no_cost) = pair.canonical_json().unwrap();
    let checked = V2SimulationResult::from_json(&with_cost).unwrap();
    assert_eq!(&checked, pair.illustrative_10bp_per_side());
    assert_eq!(checked.canonical_json().unwrap(), with_cost);
    assert_eq!(
        V2SimulationPair::from_json(&with_cost, &no_cost).unwrap(),
        pair
    );
    assert_ne!(
        pair.illustrative_10bp_per_side().sha256().unwrap(),
        pair.zero_cost_reference().sha256().unwrap()
    );
    assert!(V2SimulationResult::from_json(&no_cost).is_ok());

    let duplicate = br#"{"schema":"owner-beta-price-only-result-v2","schema":"owner-beta-price-only-result-v2"}"#;
    assert_eq!(
        V2SimulationResult::from_json(duplicate).unwrap_err(),
        V2ContractError::InvalidJson
    );

    let mut value: Value = serde_json::from_slice(&with_cost).unwrap();
    value["unknown"] = json!(true);
    assert!(V2SimulationResult::from_json(&serde_json::to_vec(&value).unwrap()).is_err());

    let mut value: Value = serde_json::from_slice(&with_cost).unwrap();
    value["ledger"]["rebalances"][0]["orders"][0]["cost"] = json!("0.0000");
    assert!(V2SimulationResult::from_json(&serde_json::to_vec(&value).unwrap()).is_err());

    let mut value: Value = serde_json::from_slice(&with_cost).unwrap();
    value["ledger"]["daily_records"][0]["closing_nav"] = json!("1.0000");
    assert!(V2SimulationResult::from_json(&serde_json::to_vec(&value).unwrap()).is_err());

    let mut value: Value = serde_json::from_slice(&with_cost).unwrap();
    value["ledger"]["rebalances"][0]["orders"][0]["filled_quantity"] = json!(1);
    assert!(V2SimulationResult::from_json(&serde_json::to_vec(&value).unwrap()).is_err());

    let mut value: Value = serde_json::from_slice(&with_cost).unwrap();
    value["ledger"]["rebalances"][0]["target"]["weights"][0]["weight_ppm"] = json!(999_999);
    value["ledger"]["rebalances"][0]["target"]["cash_weight_ppm"] = json!(1);
    assert!(V2SimulationResult::from_json(&serde_json::to_vec(&value).unwrap()).is_err());

    let mut value: Value = serde_json::from_slice(&with_cost).unwrap();
    value["ledger"]["metrics"]["period_return"] = json!("9.000000000000");
    assert!(V2SimulationResult::from_json(&serde_json::to_vec(&value).unwrap()).is_err());

    let mut value: Value = serde_json::from_slice(&with_cost).unwrap();
    value["contract_sha256"] = json!(hash(2));
    assert!(V2SimulationResult::from_json(&serde_json::to_vec(&value).unwrap()).is_err());

    let mut value: Value = serde_json::from_slice(&with_cost).unwrap();
    value["strategy"]["snapshot_sha256"] = json!(hash(2));
    assert!(V2SimulationResult::from_json(&serde_json::to_vec(&value).unwrap()).is_err());

    let mut value: Value = serde_json::from_slice(&with_cost).unwrap();
    value["ledger"]["rebalances"][0]["target"]["target_sha256"] = json!(hash(2));
    assert!(V2SimulationResult::from_json(&serde_json::to_vec(&value).unwrap()).is_err());

    let mut value: Value = serde_json::from_slice(&with_cost).unwrap();
    value["ledger"]["daily_records"][0]["surprise"] = json!(1);
    assert!(V2SimulationResult::from_json(&serde_json::to_vec(&value).unwrap()).is_err());

    let mut value: Value = serde_json::from_slice(&with_cost).unwrap();
    assert!(
        value["ledger"]["market_sessions"][1]["prices"]
            .as_array_mut()
            .unwrap()
            .pop()
            .is_some()
    );
    assert!(V2SimulationResult::from_json(&serde_json::to_vec(&value).unwrap()).is_err());

    let mut value: Value = serde_json::from_slice(&with_cost).unwrap();
    let first_market_instrument =
        value["ledger"]["market_sessions"][1]["prices"][0]["instrument_id"].clone();
    value["ledger"]["market_sessions"][1]["prices"][1]["instrument_id"] = first_market_instrument;
    assert!(V2SimulationResult::from_json(&serde_json::to_vec(&value).unwrap()).is_err());

    let mut value: Value = serde_json::from_slice(&with_cost).unwrap();
    value["ledger"]["market_sessions"][1]["prices"][0]["instrument_id"] = json!("000000.KRX");
    assert!(V2SimulationResult::from_json(&serde_json::to_vec(&value).unwrap()).is_err());
}

#[test]
fn full_market_path_must_match_strategy_benchmark_and_both_cost_profiles() {
    let base = base_market_path();
    let changed = changed_unheld_hold_day(&base);
    assert_ne!(base[1], changed[1]);

    assert!(
        result_with_paths(
            CostProfile::Illustrative10BpPerSideV1,
            changed.clone(),
            base.clone(),
        )
        .is_err()
    );

    let with_cost =
        result_with_paths(CostProfile::Illustrative10BpPerSideV1, base.clone(), base).unwrap();
    let no_cost =
        result_with_paths(CostProfile::ZeroCostReferenceV1, changed.clone(), changed).unwrap();
    assert_eq!(
        with_cost.ledger().market_sessions()[1].date().to_string(),
        "2026-02-02"
    );
    assert!(V2SimulationPair::new(with_cost.clone(), no_cost.clone()).is_err());
    let with_cost_json = with_cost.canonical_json().unwrap();
    let no_cost_json = no_cost.canonical_json().unwrap();
    assert!(V2SimulationPair::from_json(&with_cost_json, &no_cost_json).is_err());
}

#[test]
fn result_decoder_rejects_fractional_or_foreign_trend_target_observations() {
    let valid = trend_result();
    let bytes = valid.canonical_json().unwrap();
    assert_eq!(V2SimulationResult::from_json(&bytes).unwrap(), valid);
    for invalid_target in [
        allocation(&[(A, 500_000), (B, 500_000)]),
        allocation(&[(B, 1_000_000)]),
    ] {
        let mut value: Value = serde_json::from_slice(&bytes).unwrap();
        value["ledger"]["target_observations"][0]["target"] =
            serde_json::to_value(invalid_target).unwrap();
        assert!(V2SimulationResult::from_json(&serde_json::to_vec(&value).unwrap()).is_err());
    }
}

#[test]
fn approved_input_identity_keeps_existing_price_only_labels_and_pins() {
    let identity = V2InputIdentity::approved_v3_etf11().unwrap();
    assert_eq!(identity.ownership_scope(), "OWNER_ONLY");
    assert_eq!(identity.return_method(), "PRICE_RETURN_ONLY");
    assert!(identity.vendor_snapshot());
    assert!(!identity.strict_pit());
    let pins = [
        identity.candidate_content_sha256(),
        identity.artifact_manifest_sha256(),
        identity.stage5_manifest_sha256(),
        identity.action_manifest_sha256(),
        identity.approval_registry_sha256(),
    ];
    assert_eq!(pins.iter().collect::<BTreeSet<_>>().len(), 5);
}
