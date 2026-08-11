use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use domain::{ContentHash, DatasetId, InstrumentId, TradingDate, UtcTimestamp};
use job_queue::ErrorClass;
use job_queue::recommendation::compute::{
    AttestedUniverse, StrategyRequirements, compute_close, compute_close_async, requirements_for,
};
use job_queue::recommendation::input::{AttestedDataset, AttestedDatasetStatus};
use job_queue::resolver::ResolvedConfig;
use market_data::curate::schema::{read_adjusted_bars, read_bars, write_adjusted_bars, write_bars};
use market_data::{Capability, CurateStore, DatasetManifest, dataset_manifest_hash};
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
    let error = requirements_for(&wrong_version).expect_err("unshipped version is invalid input");
    assert_eq!(error.class(), ErrorClass::Input);
    assert_eq!(error.code(), "RECOMMENDATION_INPUT_INVALID");

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

fn clone_symbol_for_qa(store_root: &Path, source_symbol: &str, destination_symbol: &str) {
    let market = store_root.join("curated/bars/market=kr");
    let source = market.join(format!("symbol={source_symbol}"));
    let destination = market.join(format!("symbol={destination_symbol}"));
    copy_dir(&source, &destination);

    let store = CurateStore::new(store_root);
    let destination_id = InstrumentId::parse(destination_symbol).expect("canonical QA instrument");
    for year_entry in std::fs::read_dir(&destination).expect("read cloned QA symbol") {
        let year_entry = year_entry.expect("read cloned QA year");
        let name = year_entry.file_name().to_string_lossy().into_owned();
        let Some(year) = name
            .strip_prefix("year=")
            .and_then(|value| value.parse::<i32>().ok())
        else {
            continue;
        };
        let bars_path = store.bars_path("kr", destination_symbol, year, 2);
        if bars_path.is_file() {
            let mut rows = read_bars(&bars_path).expect("read cloned QA raw bars");
            for row in &mut rows {
                row.instrument_id = destination_id.clone();
            }
            write_bars(&bars_path, &rows).expect("rewrite cloned QA raw identity");
        }
        for adjusted_path in [
            store.adjusted_bars_path("kr", destination_symbol, year, 2),
            store.total_return_bars_path("kr", destination_symbol, year, 2),
        ] {
            if adjusted_path.is_file() {
                let mut rows =
                    read_adjusted_bars(&adjusted_path).expect("read cloned QA adjusted bars");
                for row in &mut rows {
                    row.instrument_id = destination_id.clone();
                }
                write_adjusted_bars(&adjusted_path, &rows)
                    .expect("rewrite cloned QA adjusted identity");
            }
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
    for member in MEMBERS {
        let destination = market.join(format!("symbol={member}"));
        if !destination.exists() {
            clone_symbol_for_qa(&store, "069500.KRX", member);
        }
    }
    eprintln!("QA_ONLY_SYNTHETIC: cloned Phase-0 partitions for fixed-universe computation");

    // QA_ONLY_SYNTHETIC: the Phase-0 script predates dataset manifests. The
    // recommendation seam must still exercise a real, self-hashed manifest,
    // so this temp-only fixture attests the cloned partitions explicitly.
    let curate_store = CurateStore::new(&store);
    let manifest = DatasetManifest {
        dataset_id: DatasetId::parse("krx_eod_bars").expect("QA dataset id"),
        version: 2,
        capability: Capability::PriceReturnOnly,
        created_at: UtcTimestamp::parse_rfc3339("2021-01-29T06:30:00Z")
            .expect("QA manifest timestamp"),
        source_batches: Vec::new(),
        bar_count: 11 * 260,
        action_count: 0,
        content_hash: ContentHash::from_bytes(b"placeholder"),
    };
    let manifest = DatasetManifest {
        content_hash: dataset_manifest_hash(&manifest).expect("QA manifest hash"),
        ..manifest
    };
    curate_store
        .write_dataset_manifest(&manifest)
        .expect("write QA-only manifest");
    let manifest_sha256 = manifest
        .content_hash
        .as_str()
        .strip_prefix("sha256:")
        .expect("content hash has sha256 prefix")
        .to_owned();

    QaDataset {
        _temp: temp,
        pin: AttestedDataset {
            id: Uuid::nil(),
            dataset_id: "krx_eod_bars".to_owned(),
            version: "phase0-v2-qa-only".to_owned(),
            curated_version: 2,
            status: AttestedDatasetStatus::Ready,
            manifest_sha256,
            storage_path: store.to_string_lossy().into_owned(),
        },
    }
}

fn manifest_path(qa: &QaDataset) -> PathBuf {
    Path::new(&qa.pin.storage_path).join("curated/datasets/krx_eod_bars/version=2/manifest.json")
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
fn bounded_dynamic_factor_ids_compute_a_real_snapshot() {
    let qa = qa_only_fixed_universe_dataset();
    let universe = fixed_universe();
    let mut requirements = requirements_for(&config(
        "trend_following",
        json!({
            "benchmark_instrument": "069500.KRX",
            "fast_ma": 37,
            "slow_ma": 123
        }),
    ))
    .expect("schema-valid dynamic trend windows resolve");
    assert_eq!(requirements.factor_ids, vec!["trend_37", "trend_123"]);
    assert_eq!(requirements.minimum_lookback_sessions, 123);
    requirements.factor_ids.push("vol_21".to_owned());
    let computed = compute_close(
        &qa.pin,
        &universe,
        TradingDate::parse("2021-01-29").unwrap(),
        &requirements,
    )
    .expect("bounded dynamic factors compute through the real snapshot builder");

    assert_eq!(computed.factors.len(), 11);
    for member in MEMBERS {
        let values = &computed.factors[member];
        assert_eq!(values.len(), 3);
        assert!(values["trend_37"].is_finite());
        assert!(values["trend_123"].is_finite());
        assert!(values["vol_21"].is_finite());
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
    assert_eq!(error.class(), ErrorClass::DataBlocked);

    clone_symbol_for_qa(Path::new(&qa.pin.storage_path), "069500.KRX", "153130.KRX");
    let error = compute_close(
        &qa.pin,
        &universe,
        TradingDate::parse("2021-01-28").unwrap(),
        &requirements,
    )
    .expect_err("a store with a row after as-of must fail closed");
    assert!(error.to_string().contains("future-dated row"), "{error}");
    assert_eq!(error.class(), ErrorClass::Integrity, "{error:?}");
}

#[test]
fn close_attests_the_pinned_manifest_before_reading_factors() {
    let universe = fixed_universe();
    let as_of = TradingDate::parse("2021-01-29").unwrap();
    let requirements = StrategyRequirements {
        factor_ids: vec!["vol_21".to_owned()],
        minimum_lookback_sessions: 21,
    };

    let missing = qa_only_fixed_universe_dataset();
    std::fs::remove_file(manifest_path(&missing)).expect("remove QA manifest");
    let error = compute_close(&missing.pin, &universe, as_of, &requirements)
        .expect_err("a missing pinned manifest blocks computation");
    assert_eq!(error.class(), ErrorClass::DataBlocked);
    assert_eq!(error.code(), "RECOMMENDATION_DATA_BLOCKED");

    let corrupt = qa_only_fixed_universe_dataset();
    std::fs::write(manifest_path(&corrupt), b"{not-json").expect("corrupt QA manifest bytes");
    let error = compute_close(&corrupt.pin, &universe, as_of, &requirements)
        .expect_err("a corrupt manifest is an integrity failure");
    assert_eq!(error.class(), ErrorClass::Integrity);

    let wrong_self_hash = qa_only_fixed_universe_dataset();
    let store = CurateStore::new(&wrong_self_hash.pin.storage_path);
    let dataset_id = DatasetId::parse(&wrong_self_hash.pin.dataset_id).unwrap();
    let mut manifest = store
        .read_dataset_manifest(&dataset_id, wrong_self_hash.pin.curated_version)
        .expect("read QA manifest")
        .expect("QA manifest exists");
    manifest.content_hash = ContentHash::from_bytes(b"wrong-self-hash");
    store
        .write_dataset_manifest(&manifest)
        .expect("write self-hash mismatch");
    let error = compute_close(&wrong_self_hash.pin, &universe, as_of, &requirements)
        .expect_err("a manifest whose self-hash is false is rejected");
    assert_eq!(error.class(), ErrorClass::Integrity);

    let wrong_pin_hash = qa_only_fixed_universe_dataset();
    let mut pin = wrong_pin_hash.pin.clone();
    pin.manifest_sha256 = "f".repeat(64);
    let error = compute_close(&pin, &universe, as_of, &requirements)
        .expect_err("the DB pin must equal the canonical on-disk hash");
    assert_eq!(error.class(), ErrorClass::Integrity);
}

#[test]
fn membership_discovery_is_scoped_to_the_attested_curated_version() {
    let qa = qa_only_fixed_universe_dataset();
    let market = Path::new(&qa.pin.storage_path).join("curated/bars/market=kr");
    let source = market.join("symbol=069500.KRX");
    let unrelated = market.join("symbol=999999.KRX");
    for year in ["year=2020", "year=2021"] {
        copy_dir(
            &source.join(year).join("version=2"),
            &unrelated.join(year).join("version=999"),
        );
    }

    let computed = compute_close(
        &qa.pin,
        &fixed_universe(),
        TradingDate::parse("2021-01-29").unwrap(),
        &StrategyRequirements {
            factor_ids: vec!["vol_21".to_owned()],
            minimum_lookback_sessions: 21,
        },
    )
    .expect("another version's symbol is outside this attested dataset");
    assert_eq!(computed.factors.len(), 11);

    for year in ["year=2020", "year=2021"] {
        let member_year = market.join("symbol=153130.KRX").join(year);
        std::fs::rename(
            member_year.join("version=2"),
            member_year.join("version=999"),
        )
        .expect("move one member outside the attested version");
    }
    let error = compute_close(
        &qa.pin,
        &fixed_universe(),
        TradingDate::parse("2021-01-29").unwrap(),
        &StrategyRequirements {
            factor_ids: vec!["vol_21".to_owned()],
            minimum_lookback_sessions: 21,
        },
    )
    .expect_err("another version must not satisfy attested membership");
    assert_eq!(error.class(), ErrorClass::DataBlocked);
}

#[test]
fn global_lookback_counts_prior_sessions_and_short_members_still_yield_null() {
    let qa = qa_only_fixed_universe_dataset();
    let as_of = TradingDate::parse("2021-01-29").unwrap();
    let error = compute_close(
        &qa.pin,
        &fixed_universe(),
        as_of,
        &StrategyRequirements {
            factor_ids: Vec::new(),
            minimum_lookback_sessions: 260,
        },
    )
    .expect_err("260 total closes provide only 259 prior sessions");
    assert_eq!(error.class(), ErrorClass::DataBlocked);
    assert_eq!(error.code(), "RECOMMENDATION_DATA_BLOCKED");

    let short_history =
        Path::new(&qa.pin.storage_path).join("curated/bars/market=kr/symbol=153130.KRX/year=2020");
    std::fs::remove_dir_all(short_history).expect("shorten one QA-only member's history");
    let computed = compute_close(
        &qa.pin,
        &fixed_universe(),
        as_of,
        &StrategyRequirements {
            factor_ids: vec!["return_6m".to_owned()],
            minimum_lookback_sessions: 126,
        },
    )
    .expect("the common session basis is long enough despite one new member");
    assert!(!computed.factors["153130.KRX"].contains_key("return_6m"));
}

#[test]
fn unavailable_dataset_paths_are_transient_without_message_parsing() {
    let qa = qa_only_fixed_universe_dataset();
    let mut pin = qa.pin.clone();
    pin.storage_path = qa
        ._temp
        .path()
        .join("unmounted-store")
        .display()
        .to_string();
    let error = compute_close(
        &pin,
        &fixed_universe(),
        TradingDate::parse("2021-01-29").unwrap(),
        &StrategyRequirements {
            factor_ids: Vec::new(),
            minimum_lookback_sessions: 0,
        },
    )
    .expect_err("an unavailable attested root is retryable");
    assert_eq!(error.class(), ErrorClass::Transient);
    assert_eq!(error.code(), "RECOMMENDATION_COMPUTE_UNAVAILABLE");
}

#[test]
fn malformed_parquet_is_integrity_not_a_retryable_store_error() {
    let qa = qa_only_fixed_universe_dataset();
    let adjusted = Path::new(&qa.pin.storage_path)
        .join("curated/bars/market=kr/symbol=069500.KRX/year=2021/version=2/adjusted_bars.parquet");
    std::fs::write(adjusted, b"not parquet").expect("corrupt QA parquet");
    let error = compute_close(
        &qa.pin,
        &fixed_universe(),
        TradingDate::parse("2021-01-29").unwrap(),
        &StrategyRequirements {
            factor_ids: vec!["vol_21".to_owned()],
            minimum_lookback_sessions: 21,
        },
    )
    .expect_err("malformed immutable bytes are an integrity failure");
    assert_eq!(error.class(), ErrorClass::Integrity, "{error:?}");
}

#[test]
fn missing_required_adjusted_component_is_data_blocked() {
    let qa = qa_only_fixed_universe_dataset();
    let adjusted = Path::new(&qa.pin.storage_path)
        .join("curated/bars/market=kr/symbol=069500.KRX/year=2021/version=2/adjusted_bars.parquet");
    std::fs::remove_file(adjusted).expect("remove required adjusted component");
    let error = compute_close(
        &qa.pin,
        &fixed_universe(),
        TradingDate::parse("2021-01-29").unwrap(),
        &StrategyRequirements {
            factor_ids: vec!["vol_21".to_owned()],
            minimum_lookback_sessions: 21,
        },
    )
    .expect_err("a missing immutable component blocks this dataset version");
    assert_eq!(error.class(), ErrorClass::DataBlocked, "{error:?}");
}

#[test]
fn semantically_invalid_parquet_value_is_integrity() {
    let qa = qa_only_fixed_universe_dataset();
    let adjusted = Path::new(&qa.pin.storage_path)
        .join("curated/bars/market=kr/symbol=069500.KRX/year=2021/version=2/adjusted_bars.parquet");
    let python = std::env::var_os("PYTHON").unwrap_or_else(|| "python".into());
    let output = Command::new(python)
        .arg("-c")
        .arg(
            "import sys, pyarrow as pa, pyarrow.parquet as pq; p=sys.argv[1]; t=pq.ParquetFile(p).read(); i=t.schema.get_field_index('currency'); f=t.schema.field(i); t=t.set_column(i, f, pa.array(['krw'] * t.num_rows, type=f.type)); pq.write_table(t, p)",
        )
        .arg(&adjusted)
        .output()
        .expect("launch Python semantic-corruption helper");
    assert!(
        output.status.success(),
        "semantic corruption helper failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let error = compute_close(
        &qa.pin,
        &fixed_universe(),
        TradingDate::parse("2021-01-29").unwrap(),
        &StrategyRequirements {
            factor_ids: vec!["vol_21".to_owned()],
            minimum_lookback_sessions: 21,
        },
    )
    .expect_err("semantic Parquet corruption is not retryable I/O");
    assert_eq!(error.class(), ErrorClass::Integrity, "{error:?}");
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
