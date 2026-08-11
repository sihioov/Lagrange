use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use domain::TradingDate;
use job_queue::recommendation::compute::{
    AttestedUniverse, StrategyRequirements, compute_close, compute_close_async, requirements_for,
};
use job_queue::recommendation::input::{AttestedDataset, AttestedDatasetStatus};
use job_queue::resolver::ResolvedConfig;
use serde_json::json;
use tempfile::TempDir;
use uuid::Uuid;

const MEMBERS: [&str; 11] = [
    "069500.KRX",
    "102110.KRX",
    "229200.KRX",
    "143850.KRX",
    "133690.KRX",
    "195930.KRX",
    "192090.KRX",
    "148070.KRX",
    "114260.KRX",
    "153130.KRX",
    "132030.KRX",
];

fn config(strategy_id: &str, config: serde_json::Value) -> ResolvedConfig {
    ResolvedConfig {
        strategy_id: strategy_id.to_owned(),
        strategy_version: "1.0.0".to_owned(),
        config,
    }
}

#[test]
fn requirements_follow_the_validated_strategy_parameters() {
    let cases = [
        (
            config(
                "buy_and_hold",
                json!({
                    "benchmark_instrument": "069500.KRX",
                    "target_weight": 1.0
                }),
            ),
            Vec::<String>::new(),
            0,
        ),
        (
            config(
                "trend_following",
                json!({
                    "benchmark_instrument": "069500.KRX",
                    "fast_ma": 100,
                    "slow_ma": 200
                }),
            ),
            vec!["trend_100".to_owned(), "trend_200".to_owned()],
            200,
        ),
        (
            config(
                "relative_momentum",
                json!({"top_n": 3, "lookback_months": 6}),
            ),
            vec!["return_6m".to_owned()],
            126,
        ),
        (
            config(
                "relative_momentum",
                json!({"top_n": 3, "lookback_months": 12}),
            ),
            vec!["momentum_12_1".to_owned()],
            252,
        ),
        (
            config(
                "dual_momentum",
                json!({"absolute_threshold": 0.0, "lookback_months": 6}),
            ),
            vec!["return_6m".to_owned()],
            126,
        ),
        (
            config(
                "inverse_volatility",
                json!({"vol_window": 120, "max_weight": 0.3}),
            ),
            vec!["vol_120".to_owned()],
            120,
        ),
    ];

    for (resolved, expected_factors, expected_lookback) in cases {
        let requirements = requirements_for(&resolved)
            .unwrap_or_else(|error| panic!("{}: {error}", resolved.strategy_id));
        assert_eq!(requirements.factor_ids, expected_factors);
        assert_eq!(requirements.minimum_lookback_sessions, expected_lookback);
    }
}

#[test]
fn requirements_reject_unshipped_versions_and_schema_invalid_parameters() {
    let mut wrong_version = config(
        "buy_and_hold",
        json!({"benchmark_instrument": "069500.KRX", "target_weight": 1.0}),
    );
    wrong_version.strategy_version = "1.0.1".to_owned();
    assert!(requirements_for(&wrong_version).is_err());

    for invalid in [
        config(
            "trend_following",
            json!({
                "benchmark_instrument": "069500.KRX",
                "fast_ma": 4,
                "slow_ma": 200
            }),
        ),
        config(
            "relative_momentum",
            json!({"top_n": 3, "lookback_months": 9}),
        ),
        config(
            "inverse_volatility",
            json!({"vol_window": 120, "max_weight": 0.3, "extra": true}),
        ),
    ] {
        assert!(
            requirements_for(&invalid).is_err(),
            "schema-invalid config was accepted: {} {}",
            invalid.strategy_id,
            invalid.config
        );
    }
}

#[test]
fn equal_trend_windows_require_one_factor_without_panicking_the_snapshot_builder() {
    let requirements = requirements_for(&config(
        "trend_following",
        json!({
            "benchmark_instrument": "069500.KRX",
            "fast_ma": 100,
            "slow_ma": 100
        }),
    ))
    .expect("equal windows are allowed by the shipped JSON schema");
    assert_eq!(requirements.factor_ids, vec!["trend_100"]);
    assert_eq!(requirements.minimum_lookback_sessions, 100);
}

struct QaDataset {
    _temp: TempDir,
    pin: AttestedDataset,
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("job-queue lives under repository/crates")
        .to_path_buf()
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create QA-only partition directory");
    for entry in std::fs::read_dir(from).expect("read QA-only source partition") {
        let entry = entry.expect("read QA-only source entry");
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).expect("copy QA-only source file");
        }
    }
}

fn qa_only_fixed_universe_dataset() -> QaDataset {
    let repo = repo_root();
    let temp = tempfile::Builder::new()
        .prefix(".recommendation-compute-qa-")
        .tempdir_in(&repo)
        .expect("create repository-local QA tempdir");
    let generated = temp.path().join("phase0");
    let python = std::env::var_os("PYTHON").unwrap_or_else(|| "python".into());
    let output = Command::new(python)
        .current_dir(&repo)
        .arg(repo.join("scripts/ci/prepare_phase0.py"))
        .arg("--root")
        .arg(&generated)
        .output()
        .expect("launch the existing Phase-0 generator");
    assert!(
        output.status.success(),
        "Phase-0 generation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // QA_ONLY_SYNTHETIC: Phase-0 contains three seed ETFs. Clone one of its
    // deterministic partitions under the remaining canonical ids only in
    // this temp root, so production still fails closed on incomplete data.
    let store = generated.join("curated");
    let market = store.join("curated/bars/market=kr");
    let source = market.join("symbol=069500.KRX");
    for member in MEMBERS {
        let destination = market.join(format!("symbol={member}"));
        if !destination.exists() {
            copy_dir(&source, &destination);
        }
    }
    eprintln!("QA_ONLY_SYNTHETIC: cloned Phase-0 partitions for fixed-universe computation");

    QaDataset {
        _temp: temp,
        pin: AttestedDataset {
            id: Uuid::nil(),
            dataset_id: "krx_eod_bars".to_owned(),
            version: "phase0-v2-qa-only".to_owned(),
            curated_version: 2,
            status: AttestedDatasetStatus::Ready,
            manifest_sha256: "0".repeat(64),
            storage_path: store.to_string_lossy().into_owned(),
        },
    }
}

fn fixed_universe() -> AttestedUniverse {
    AttestedUniverse::from_manifest_yaml(include_str!(
        "../../../configs/universes/kr-etf-core-v1.yaml"
    ))
    .expect("the repository fixed-universe manifest is valid")
}

#[test]
fn fixed_universe_close_is_exact_finite_and_deterministic() {
    let qa = qa_only_fixed_universe_dataset();
    let universe = fixed_universe();
    assert_eq!(universe.universe_id, "kr-etf-core-v1");
    assert_eq!(
        universe.members.iter().cloned().collect::<BTreeSet<_>>(),
        MEMBERS.into_iter().map(str::to_owned).collect()
    );

    let as_of = TradingDate::parse("2021-01-29").unwrap();
    let requirements = StrategyRequirements {
        factor_ids: vec!["trend_100".to_owned(), "trend_200".to_owned()],
        minimum_lookback_sessions: 200,
    };
    let first = compute_close(&qa.pin, &universe, as_of, &requirements).expect("compute close");
    let second = compute_close(&qa.pin, &universe, as_of, &requirements).expect("repeat close");

    assert_eq!(first.as_of, as_of, "only the requested close is returned");
    assert_eq!(first.factor_snapshot_hash, second.factor_snapshot_hash);
    assert_eq!(first.factors, second.factors);
    assert_eq!(first.factors.len(), 11);
    for member in MEMBERS {
        let values = &first.factors[member];
        assert_eq!(values.len(), 2);
        assert!(values["trend_100"].is_finite());
        assert!(values["trend_200"].is_finite());
    }
}

#[test]
fn close_omits_null_factor_values() {
    let qa = qa_only_fixed_universe_dataset();
    let universe = fixed_universe();
    let short_history =
        Path::new(&qa.pin.storage_path).join("curated/bars/market=kr/symbol=153130.KRX/year=2020");
    std::fs::remove_dir_all(short_history).expect("shorten one QA-only member's history");
    let requirements = StrategyRequirements {
        factor_ids: vec!["return_6m".to_owned()],
        minimum_lookback_sessions: 126,
    };
    let computed = compute_close(
        &qa.pin,
        &universe,
        TradingDate::parse("2021-01-29").unwrap(),
        &requirements,
    )
    .expect("NULL values are a downstream exclusion, not a fabricated zero");

    assert!(computed.factors["069500.KRX"].contains_key("return_6m"));
    assert!(!computed.factors["153130.KRX"].contains_key("return_6m"));
}

#[test]
fn close_rejects_an_incomplete_fixed_universe_and_future_rows() {
    let qa = qa_only_fixed_universe_dataset();
    let universe = fixed_universe();
    let requirements = StrategyRequirements {
        factor_ids: vec!["vol_120".to_owned()],
        minimum_lookback_sessions: 120,
    };
    let missing = Path::new(&qa.pin.storage_path).join("curated/bars/market=kr/symbol=153130.KRX");
    std::fs::remove_dir_all(&missing).expect("remove one QA-only member");
    let error = compute_close(
        &qa.pin,
        &universe,
        TradingDate::parse("2021-01-29").unwrap(),
        &requirements,
    )
    .expect_err("an incomplete fixed universe must fail closed");
    assert!(error.to_string().contains("universe"), "{error}");

    copy_dir(
        &Path::new(&qa.pin.storage_path).join("curated/bars/market=kr/symbol=069500.KRX"),
        &missing,
    );
    let error = compute_close(
        &qa.pin,
        &universe,
        TradingDate::parse("2021-01-28").unwrap(),
        &requirements,
    )
    .expect_err("a store with a row after as-of must fail closed");
    assert!(error.to_string().contains("future-dated row"), "{error}");
}

#[tokio::test]
async fn async_close_uses_the_blocking_boundary() {
    let qa = qa_only_fixed_universe_dataset();
    let as_of = TradingDate::parse("2021-01-29").unwrap();
    let computed = compute_close_async(
        qa.pin.clone(),
        fixed_universe(),
        as_of,
        StrategyRequirements {
            factor_ids: vec!["vol_120".to_owned()],
            minimum_lookback_sessions: 120,
        },
    )
    .await
    .expect("spawn_blocking computation");
    assert_eq!(computed.as_of, as_of);
    assert_eq!(computed.factors.len(), 11);
}
