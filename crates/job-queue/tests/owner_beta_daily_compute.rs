//! Loader and pure computation-contract regression tests. These synthetic wire
//! claims deliberately do not construct or stand in for an admitted artifact.
use std::collections::BTreeMap;

use domain::{ContentHash, TradingDate};
use factor_engine::daily_price_only::{
    DAILY_FACTOR_SNAPSHOT_SCHEMA, DAILY_PRICE_ONLY_INPUT_KIND, DailyPriceOnlyFactorSnapshot,
};
use factor_engine::price_only::{PRICE_ONLY_CAPABILITY, PRICE_ONLY_INPUT_KIND};
use factor_engine::{
    PriceOnlyFactorSnapshot,
    snapshot::{FactorRow, NormalizationMeta},
};
use job_queue::owner_beta::daily_input::{
    OwnerBetaDailyRecommendationInputV1, OwnerBetaExecutionInput, OwnerBetaExecutionProvenance,
    decode_owner_beta_execution_input,
};
use job_queue::owner_beta::daily_target::{
    OWNER_BETA_DAILY_TARGET_SNAPSHOT_SCHEMA, build_daily_target_snapshot,
};
use job_queue::owner_beta::{
    OwnerBetaPriceRecommendationInput, OwnerBetaStrategySnapshot, OwnerBetaTargetSnapshotError,
    build_target_snapshot,
};
use job_queue::resolver::ResolvedConfig;
use market_data::KR_ETF_CORE_SYMBOLS;
use serde_json::{Value, json};

fn pin(n: u8) -> String {
    format!("sha256:{n:064x}")
}
fn date() -> TradingDate {
    TradingDate::parse("2026-08-31").unwrap()
}
fn strategy(id: &str, config: Value) -> OwnerBetaStrategySnapshot {
    OwnerBetaStrategySnapshot::from_resolved_config(&ResolvedConfig {
        strategy_id: id.into(),
        strategy_version: "1.0.0".into(),
        config,
    })
    .unwrap()
}
fn daily_value(strategy: &OwnerBetaStrategySnapshot) -> Value {
    json!({"schema":"owner-beta-daily-recommendation-v1", "run_id":"00000000-0000-4000-8000-000000000001", "strategy_config_id":"00000000-0000-4000-8000-000000000002", "as_of":"2026-08-31",
        "input":{"kind":"owner_beta_daily_etf11_price_only_v1","content_sha256":pin(1),"manifest_sha256":pin(2),"admission_sha256":pin(3),"policy_sha256":pin(4)},
        "strategy":strategy,"trigger":{"kind":"DAILY","version":"owner-beta-daily-trigger-v1","config_epoch":7,"activation_epoch":9}})
}
fn daily(strategy: &OwnerBetaStrategySnapshot) -> OwnerBetaDailyRecommendationInputV1 {
    serde_json::from_value(daily_value(strategy)).unwrap()
}
fn buy() -> OwnerBetaStrategySnapshot {
    strategy(
        "buy_and_hold",
        json!({"benchmark_instrument":"069500.KRX","target_weight":1.0}),
    )
}
fn historical(strategy: &OwnerBetaStrategySnapshot) -> OwnerBetaPriceRecommendationInput {
    serde_json::from_value(json!({"run_id":"00000000-0000-4000-8000-000000000001", "strategy_config_id":"00000000-0000-4000-8000-000000000002", "as_of":"2026-08-31",
        "pins":{"candidate_content_sha256":pin(11),"artifact_manifest_sha256":pin(12),"stage5_manifest_sha256":pin(13),"action_manifest_sha256":pin(14),"approval_registry_sha256":pin(15)},"strategy":strategy})).unwrap()
}
fn factors(
    input: &OwnerBetaDailyRecommendationInputV1,
    ids: &[&str],
    raw: Option<f64>,
) -> DailyPriceOnlyFactorSnapshot {
    let mut result = DailyPriceOnlyFactorSnapshot {
        schema: DAILY_FACTOR_SNAPSHOT_SCHEMA.into(),
        input_kind: DAILY_PRICE_ONLY_INPUT_KIND.into(),
        capability: PRICE_ONLY_CAPABILITY.into(),
        as_of: date(),
        content_sha256: input.pins().content_sha256().to_string(),
        manifest_sha256: input.pins().manifest_sha256().to_string(),
        admission_sha256: input.pins().admission_sha256().to_string(),
        policy_sha256: input.pins().policy_sha256().to_string(),
        factor_versions: ids
            .iter()
            .map(|id| ((*id).into(), "1.0.0".into()))
            .collect(),
        normalization: NormalizationMeta {
            id: "test".into(),
            version: "1.0.0".into(),
            params: BTreeMap::new(),
        },
        rows: KR_ETF_CORE_SYMBOLS
            .iter()
            .flat_map(|symbol| {
                ids.iter().map(move |id| FactorRow {
                    date: date().to_iso(),
                    instrument: format!("{symbol}.KRX"),
                    factor: (*id).into(),
                    raw,
                    normalized: raw.map(|_| 0.0),
                })
            })
            .collect(),
        hash: ContentHash::from_bytes(b"placeholder"),
    };
    rehash(&mut result);
    result
}
fn rehash(snapshot: &mut DailyPriceOnlyFactorSnapshot) {
    snapshot.hash = snapshot.compute_hash().unwrap();
}
fn historical_factors(
    input: &OwnerBetaPriceRecommendationInput,
    value: &DailyPriceOnlyFactorSnapshot,
) -> PriceOnlyFactorSnapshot {
    let mut result = PriceOnlyFactorSnapshot {
        input_kind: PRICE_ONLY_INPUT_KIND.into(),
        capability: PRICE_ONLY_CAPABILITY.into(),
        as_of: value.as_of,
        candidate_content_sha256: input.pins().candidate_content_sha256().to_string(),
        artifact_manifest_sha256: input.pins().artifact_manifest_sha256().to_string(),
        stage5_manifest_sha256: input.pins().stage5_manifest_sha256().to_string(),
        action_manifest_sha256: input.pins().action_manifest_sha256().to_string(),
        approval_registry_sha256: input.pins().approval_registry_sha256().to_string(),
        factor_versions: value.factor_versions.clone(),
        normalization: value.normalization.clone(),
        rows: value.rows.clone(),
        hash: ContentHash::from_bytes(b"placeholder"),
    };
    result.hash = result.compute_hash().unwrap();
    result
}

#[test]
fn loader_round_trips_original_historical_bytes_without_a_tag() {
    let legacy = historical(&buy());
    let original = serde_json::to_vec(&legacy).unwrap();
    let decoded = decode_owner_beta_execution_input(&original).unwrap();
    assert!(matches!(
        decoded.provenance(),
        OwnerBetaExecutionProvenance::Historical(_)
    ));
    assert_eq!(decoded.run_id(), legacy.run_id());
    assert_eq!(decoded.strategy_config_id(), legacy.strategy_config_id());
    assert_eq!(decoded.as_of(), legacy.as_of());
    assert_eq!(serde_json::to_vec(&decoded).unwrap(), original);
    assert!(
        serde_json::to_value(decoded)
            .unwrap()
            .get("schema")
            .is_none()
    );
}

#[test]
fn daily_loader_keeps_exact_shape_pins_and_epochs() {
    let expected = daily_value(&buy());
    let decoded =
        decode_owner_beta_execution_input(&serde_json::to_vec(&expected).unwrap()).unwrap();
    assert!(matches!(
        decoded.provenance(),
        OwnerBetaExecutionProvenance::Daily(_)
    ));
    assert_eq!(serde_json::to_value(&decoded).unwrap(), expected);
    let OwnerBetaExecutionInput::Daily(input) = decoded else {
        panic!("daily branch")
    };
    assert_eq!(input.config_epoch(), 7);
    assert_eq!(input.activation_epoch(), 9);
    assert_eq!(input.pins().admission_sha256().as_str(), pin(3));
}

#[test]
fn loader_rejects_unknown_null_and_mixed_schema_without_fallback() {
    let valid = daily_value(&buy());
    for case in 0..7 {
        let mut changed = valid.clone();
        match case {
            0 => {
                changed["schema"] = json!("unknown");
            }
            1 => {
                changed["schema"] = Value::Null;
            }
            2 => {
                changed.as_object_mut().unwrap().remove("schema");
            }
            3 => {
                changed["pins"] = json!({});
            }
            4 => {
                changed["input"]["approval_registry_sha256"] = json!(pin(8));
            }
            5 => {
                changed["trigger"]["extra"] = json!(true);
            }
            _ => {
                changed["input"]["kind"] = json!(PRICE_ONLY_INPUT_KIND);
            }
        }
        assert!(
            decode_owner_beta_execution_input(&serde_json::to_vec(&changed).unwrap()).is_err(),
            "case {case}"
        );
    }
    let mut old = serde_json::to_value(historical(&buy())).unwrap();
    old["schema"] = json!("owner-beta-daily-recommendation-v1");
    assert!(decode_owner_beta_execution_input(&serde_json::to_vec(&old).unwrap()).is_err());
}

#[test]
fn duplicate_keys_are_rejected_even_when_the_last_value_is_valid() {
    let text = serde_json::to_string(&daily_value(&buy())).unwrap();
    for (needle, replacement) in [
        (
            "\"schema\":\"owner-beta-daily-recommendation-v1\"",
            "\"schema\":null,\"schema\":\"owner-beta-daily-recommendation-v1\"",
        ),
        (
            "\"config_epoch\":7",
            "\"config_epoch\":-1,\"config_epoch\":7",
        ),
        (
            "\"target_weight\":1.0",
            "\"target_weight\":0.5,\"target_weight\":1.0",
        ),
    ] {
        let changed = text.replacen(needle, replacement, 1);
        assert_ne!(changed, text);
        assert!(decode_owner_beta_execution_input(changed.as_bytes()).is_err());
        assert!(serde_json::from_str::<OwnerBetaDailyRecommendationInputV1>(&changed).is_err());
    }
}

#[test]
fn daily_trigger_accepts_only_exact_nonnegative_i64_epochs() {
    for key in ["config_epoch", "activation_epoch"] {
        for valid in [0, i64::MAX] {
            let mut value = daily_value(&buy());
            value["trigger"][key] = json!(valid);
            assert!(serde_json::from_value::<OwnerBetaDailyRecommendationInputV1>(value).is_ok());
        }
        for invalid in [
            json!(-1),
            json!(1.5),
            json!("7"),
            json!(u64::MAX),
            Value::Null,
        ] {
            let mut value = daily_value(&buy());
            value["trigger"][key] = invalid;
            assert!(serde_json::from_value::<OwnerBetaDailyRecommendationInputV1>(value).is_err());
        }
    }
}

#[test]
fn daily_strategy_hash_and_input_size_are_checked_before_compute() {
    let mut value = daily_value(&buy());
    value["strategy"]["config_json"]["target_weight"] = json!(0.5);
    assert!(serde_json::from_value::<OwnerBetaDailyRecommendationInputV1>(value).is_err());
    assert!(decode_owner_beta_execution_input(&vec![b' '; 1024 * 1024 + 1]).is_err());
}

#[test]
fn five_daily_strategies_preserve_historical_arithmetic_and_distinct_hashes() {
    let cases = [
        (
            strategy(
                "buy_and_hold",
                json!({"benchmark_instrument":"069500.KRX","target_weight":1.0}),
            ),
            vec![],
        ),
        (
            strategy(
                "trend_following",
                json!({"benchmark_instrument":"069500.KRX","fast_ma":100,"slow_ma":200}),
            ),
            vec!["trend_100", "trend_200"],
        ),
        (
            strategy("relative_momentum", json!({"top_n":3,"lookback_months":12})),
            vec!["momentum_12_1"],
        ),
        (
            strategy(
                "dual_momentum",
                json!({"absolute_threshold":0.0,"lookback_months":12}),
            ),
            vec!["return_12m"],
        ),
        (
            strategy(
                "inverse_volatility",
                json!({"vol_window":60,"max_weight":0.3}),
            ),
            vec!["vol_60"],
        ),
    ];
    for (strategy, ids) in cases {
        let input = daily(&strategy);
        let snapshot = factors(&input, &ids, Some(0.1));
        let target = build_daily_target_snapshot(&input, &snapshot).unwrap();
        let old = historical(&strategy);
        let old_snapshot = historical_factors(&old, &snapshot);
        let old_target = build_target_snapshot(&old, &old_snapshot).unwrap();
        assert_eq!(target.items(), old_target.items());
        assert_eq!(target.cash_weight_ppm(), old_target.cash_weight_ppm());
        assert_eq!(target.portfolio_reasons(), old_target.portfolio_reasons());
        assert_eq!(target.items().len(), 11);
        assert_eq!(
            target
                .items()
                .iter()
                .filter_map(|x| x.target_weight_ppm())
                .sum::<i64>()
                + target.cash_weight_ppm(),
            1_000_000
        );
        assert_ne!(
            target.target_snapshot_sha256(),
            old_target.target_snapshot_sha256()
        );
        target.validate_hash().unwrap();
    }
}

#[test]
fn daily_relative_nulls_fail_closed_without_forward_fill() {
    let input = daily(&strategy(
        "relative_momentum",
        json!({"top_n":3,"lookback_months":12}),
    ));
    let mut snapshot = factors(&input, &["momentum_12_1"], None);
    assert_eq!(
        build_daily_target_snapshot(&input, &snapshot),
        Err(OwnerBetaTargetSnapshotError::FactorSnapshotInvalid)
    );
    let mut older = snapshot.rows.clone();
    for row in &mut older {
        row.date = "2026-08-28".into();
        row.raw = Some(100.0);
        row.normalized = Some(1.0);
    }
    snapshot.rows.splice(0..0, older);
    rehash(&mut snapshot);
    assert_eq!(
        build_daily_target_snapshot(&input, &snapshot),
        Err(OwnerBetaTargetSnapshotError::FactorSnapshotInvalid)
    );
    let legacy_input = historical(input.strategy_snapshot());
    assert_eq!(
        build_target_snapshot(&legacy_input, &historical_factors(&legacy_input, &snapshot)),
        Err(OwnerBetaTargetSnapshotError::FactorSnapshotInvalid)
    );

    // One current value enables exactly that identity. The ten earlier, larger
    // values cannot fill the other current NULLs or compete for selection.
    let current = snapshot
        .rows
        .iter_mut()
        .find(|row| row.date == date().to_iso())
        .unwrap();
    current.raw = Some(1.0);
    current.normalized = Some(0.0);
    let selected = current.instrument.clone();
    rehash(&mut snapshot);
    let target = build_daily_target_snapshot(&input, &snapshot).unwrap();
    assert_eq!(target.items().len(), 11);
    assert_eq!(target.cash_weight_ppm(), 0);
    for item in target.items() {
        assert_eq!(
            item.target_weight_ppm(),
            (item.instrument_id() == selected).then_some(1_000_000)
        );
    }
}

#[test]
fn daily_trend_equal_averages_preserve_explicit_all_cash() {
    let input = daily(&strategy(
        "trend_following",
        json!({"benchmark_instrument":"069500.KRX","fast_ma":100,"slow_ma":200}),
    ));
    let snapshot = factors(&input, &["trend_100", "trend_200"], Some(0.0));
    let target = build_daily_target_snapshot(&input, &snapshot).unwrap();
    let legacy_input = historical(input.strategy_snapshot());
    let legacy =
        build_target_snapshot(&legacy_input, &historical_factors(&legacy_input, &snapshot))
            .unwrap();
    assert_eq!(target.items(), legacy.items());
    assert_eq!(target.portfolio_reasons(), legacy.portfolio_reasons());
    assert_eq!(target.cash_weight_ppm(), 1_000_000);
    assert!(
        target
            .items()
            .iter()
            .all(|item| item.target_weight_ppm().is_none())
    );
    assert!(
        target
            .portfolio_reasons()
            .iter()
            .any(|r| r.code() == "TREND_NEGATIVE_CASH")
    );
}

#[test]
fn daily_relative_rounding_has_exact_cash_residue() {
    let input = daily(&strategy(
        "relative_momentum",
        json!({"top_n":3,"lookback_months":12}),
    ));
    let target =
        build_daily_target_snapshot(&input, &factors(&input, &["momentum_12_1"], Some(1.0)))
            .unwrap();
    let selected = target
        .items()
        .iter()
        .filter(|i| i.target_weight_ppm().is_some())
        .collect::<Vec<_>>();
    assert_eq!(
        selected
            .iter()
            .map(|i| i.instrument_id())
            .collect::<Vec<_>>(),
        vec!["069500.KRX", "102110.KRX", "114260.KRX"]
    );
    assert!(
        selected
            .iter()
            .all(|i| i.target_weight_ppm() == Some(333_300))
    );
    assert_eq!(target.cash_weight(), "0.000100");
}

#[test]
fn daily_target_rejects_future_missing_duplicate_unknown_and_bad_version_rows() {
    let input = daily(&strategy(
        "relative_momentum",
        json!({"top_n":3,"lookback_months":12}),
    ));
    let valid = factors(&input, &["momentum_12_1"], Some(1.0));
    for case in 0..6 {
        let mut bad = valid.clone();
        match case {
            0 => bad.rows[0].date = "2026-09-01".into(),
            1 => {
                bad.rows.pop();
            }
            2 => bad.rows.push(bad.rows[0].clone()),
            3 => bad.rows[0].instrument = "000020.KRX".into(),
            4 => bad.rows[0].factor = "avg_value_20".into(),
            _ => {
                bad.factor_versions
                    .insert("momentum_12_1".into(), "999.0.0".into());
            }
        }
        rehash(&mut bad);
        assert!(
            build_daily_target_snapshot(&input, &bad).is_err(),
            "case {case}"
        );
    }
}

#[test]
fn daily_target_binds_each_pin_and_rejects_mismatched_factor_provenance() {
    let original = daily(&buy());
    let baseline = build_daily_target_snapshot(&original, &factors(&original, &[], None)).unwrap();
    for key in [
        "content_sha256",
        "manifest_sha256",
        "admission_sha256",
        "policy_sha256",
    ] {
        let mut value = daily_value(&buy());
        value["input"][key] = json!(pin(99));
        let changed: OwnerBetaDailyRecommendationInputV1 = serde_json::from_value(value).unwrap();
        assert!(build_daily_target_snapshot(&changed, &factors(&original, &[], None)).is_err());
        let output = build_daily_target_snapshot(&changed, &factors(&changed, &[], None)).unwrap();
        assert_ne!(
            output.target_snapshot_sha256(),
            baseline.target_snapshot_sha256()
        );
    }
    let encoded: Value = serde_json::from_slice(&baseline.canonical_bytes().unwrap()).unwrap();
    assert_eq!(encoded["schema"], OWNER_BETA_DAILY_TARGET_SNAPSHOT_SCHEMA);
    assert_eq!(encoded["admission_sha256"], pin(3));
    assert!(encoded.get("approval_registry_sha256").is_none());
    assert!(encoded.get("target_snapshot_sha256").is_none());
}

#[test]
fn daily_target_rejects_stale_hash_and_wrong_domain_before_strategy_work() {
    let input = daily(&buy());
    let valid = factors(&input, &[], None);
    for case in 0..5 {
        let mut bad = valid.clone();
        match case {
            0 => bad.hash = ContentHash::from_bytes(b"bad"),
            1 => bad.schema = "historical".into(),
            2 => bad.input_kind = PRICE_ONLY_INPUT_KIND.into(),
            3 => bad.capability = "PRICE_VOLUME".into(),
            _ => bad.as_of = TradingDate::parse("2026-08-28").unwrap(),
        }
        if case != 0 {
            rehash(&mut bad);
        }
        assert!(build_daily_target_snapshot(&input, &bad).is_err());
    }
}
