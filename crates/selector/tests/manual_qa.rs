//! Todo 16 manual QA channel: rank the full 11-ETF seed universe on one
//! as-of date into a constrained target portfolio and print the ordered
//! explainable table (instrument, raw/normalized factors, rank, reason,
//! target weight, cash).
//!
//! The universe is the REAL v1 manifest (`configs/universes/kr-etf-core-v1.yaml`)
//! published through the real pipeline, so `universe_snapshot_id` and the
//! membership are production-shaped. Factor VALUES are synthetic fixtures —
//! the selector consumes factor snapshots (Todo 15) and never recomputes
//! factors. Run with `--nocapture` to capture the transcript; the rerun
//! assertion proves byte-identical determinism.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use auth::entitlement::{
    CalendarDate, ContractRef, DataProvider, DatasetId, DocumentHash, Entitlement,
    EntitlementService, EntitlementState, KrUse, UserId,
};
use domain::{ContentHash, DataState, InstrumentId, TradingDate};
use factor_engine::snapshot::NormalizationMeta;
use factor_engine::{FactorRow, FactorSnapshot};
use market_data::{QualityReport, seed_universe};
use selector::spec::SelectionSpec;
use selector::target::TargetPortfolio;
use selector::{PublishedSnapshot, UniversePublisher, parse_manifest, select_targets};

const V1_SYMBOLS: [&str; 11] = [
    "069500", "102110", "229200", "143850", "133690", "195930", "192090", "148070", "114260",
    "153130", "132030",
];
const AS_OF: (i32, u32, u32) = (2020, 1, 31);

fn manifest_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/universes/kr-etf-core-v1.yaml")
}

fn active_entitlement() -> Entitlement {
    Entitlement::builder()
        .id(auth::entitlement::EntitlementId::new("ent_krx_universe_v1"))
        .provider(DataProvider::Krx)
        .contract(ContractRef::new(
            DocumentHash::sha256("0".repeat(64)),
            "vault://krx-entitlements/ent_krx_universe_v1.pdf",
        ))
        .lifecycle(EntitlementState::Active)
        .effective(
            CalendarDate::parse("2019-01-01").expect("valid window"),
            CalendarDate::parse("2030-12-31").expect("valid window"),
        )
        .covered_datasets([DatasetId::krx_eod_bars()])
        .covered_uses([KrUse::Dataset])
        .covered_users([UserId::new("universe_publisher")])
        .build()
}

fn ready_report() -> QualityReport {
    QualityReport {
        dataset_id: domain::DatasetId::parse("kr-etf-daily").expect("valid id"),
        version: 1,
        state: DataState::Ready,
        issues: vec![],
        exclusions: vec![],
        content_hash: ContentHash::from_bytes(b"ready-report"),
    }
}

/// The real v1 universe, published through the real pipeline.
fn real_universe() -> PublishedSnapshot {
    let yaml = fs::read_to_string(manifest_path()).expect("v1 manifest reads");
    let manifest = parse_manifest(&yaml).expect("v1 manifest parses");
    UniversePublisher::new(
        seed_universe(),
        EntitlementService::new(vec![active_entitlement()]),
    )
    .publish(&manifest)
    .expect("v1 universe publishes")
}

/// Synthetic factor values over the REAL universe (ids + snapshot id are
/// production-shaped; values are documented fixtures — the selector consumes,
/// never computes, factors).
fn fixture_factors(universe: &PublishedSnapshot) -> FactorSnapshot {
    let as_of = TradingDate::new(AS_OF.0, AS_OF.1, AS_OF.2).expect("valid date");
    let values: [(&str, f64, f64, f64, f64); 11] = [
        ("069500", 0.182, 1.5, 0.121, -0.4),
        ("102110", 0.121, 1.2, 0.095, -0.1),
        ("229200", 0.095, 0.9, 0.141, 0.2),
        ("143850", 0.071, 0.6, 0.110, 0.1),
        ("133690", 0.048, 0.3, 0.132, 0.3),
        ("195930", 0.021, 0.0, 0.104, 0.0),
        ("192090", -0.012, -0.3, 0.160, 0.5),
        ("148070", -0.041, -0.6, 0.120, -0.2),
        ("114260", -0.072, -0.9, 0.135, 0.4),
        ("153130", -0.101, -1.2, 0.098, -0.3),
        ("132030", -0.135, -1.5, 0.115, -0.5),
    ];
    let mut rows = Vec::new();
    for (symbol, raw12, norm12, rawvol, normvol) in values {
        rows.push(FactorRow {
            date: as_of.to_iso(),
            instrument: InstrumentId::parse(&format!("{symbol}.KRX"))
                .expect("valid id")
                .as_str(),
            factor: "return_12m".to_owned(),
            raw: Some(raw12),
            normalized: Some(norm12),
        });
        rows.push(FactorRow {
            date: as_of.to_iso(),
            instrument: InstrumentId::parse(&format!("{symbol}.KRX"))
                .expect("valid id")
                .as_str(),
            factor: "vol_20d".to_owned(),
            raw: Some(rawvol),
            normalized: Some(normvol),
        });
    }
    FactorSnapshot {
        as_of,
        universe_snapshot_id: universe.universe_snapshot_id.as_str().to_owned(),
        dataset_id: "kr-etf-daily".to_owned(),
        dataset_version: 1,
        factor_versions: BTreeMap::from([
            ("return_12m".to_owned(), "1.0.0".to_owned()),
            ("vol_20d".to_owned(), "1.0.0".to_owned()),
        ]),
        normalization: NormalizationMeta {
            id: "z_score".to_owned(),
            version: "1.0.0".to_owned(),
            params: BTreeMap::from([("cap".to_owned(), "3.0".to_owned())]),
        },
        rows,
        hash: ContentHash::from_bytes(b"qa-factors-fixture"),
    }
}

fn spec() -> SelectionSpec {
    SelectionSpec::new(
        "relative_momentum",
        "1.0.0",
        BTreeMap::from([("return_12m".to_owned(), 0.7), ("vol_20d".to_owned(), 0.3)]),
        ["return_12m".to_owned()].into_iter().collect(),
        7,
        0.8 / 7.0,
        0.2,
        4,
        1e-9,
    )
    .expect("spec validates")
}

fn run() -> TargetPortfolio {
    let universe = real_universe();
    let factors = fixture_factors(&universe);
    select_targets(&spec(), &ready_report(), &universe, &factors).expect("selection succeeds")
}

fn print_table(portfolio: &TargetPortfolio) {
    println!(
        "=== Lagrange Station selector QA (as-of {}) ===",
        portfolio.as_of.to_iso()
    );
    println!("universe_snapshot_id: {}", portfolio.universe_snapshot_id);
    println!("factor_snapshot_hash: {}", portfolio.factor_snapshot_hash);
    println!("strategy: {}", portfolio.strategy_version);
    println!(
        "constraints: top_n={} max_weight={} cash_floor={} weight_scale={} tolerance={}",
        portfolio.constraints.top_n,
        portfolio.constraints.max_weight,
        portfolio.constraints.cash_floor,
        portfolio.constraints.weight_scale,
        portfolio.constraints.tolerance
    );
    println!(
        "{:>3}  {:<12} {:>9} {:>9} {:>10} {:>10} {:>10}  {:<44} weight",
        "rk", "instrument", "raw_12m", "norm_12m", "raw_vol", "norm_vol", "score", "reason",
    );
    for t in &portfolio.targets {
        let f12 = &t.factors["return_12m"];
        let fvol = &t.factors["vol_20d"];
        let reasons: Vec<String> = t
            .reasons
            .iter()
            .map(|r| format!("{}:{}", r.code.as_str(), r.text_en))
            .collect();
        println!(
            "{:>3}  {:<12} {:>9} {:>9} {:>10} {:>10} {:>10.6}  {:<44} {:.4}",
            t.rank,
            t.instrument_id,
            fmt_opt(f12.raw),
            fmt_opt(f12.normalized),
            fmt_opt(fvol.raw),
            fmt_opt(fvol.normalized),
            t.score,
            reasons.join("; "),
            t.target_weight
        );
    }
    println!(
        "cash_weight: {:.4}  (portfolio reasons: {})",
        portfolio.cash_weight,
        portfolio
            .portfolio_reasons
            .iter()
            .map(|r| format!("{}:{}", r.code.as_str(), r.text_en))
            .collect::<Vec<_>>()
            .join("; ")
    );
    println!(
        "exclusions: {}",
        portfolio
            .exclusions
            .iter()
            .map(|e| format!("{} ({})", e.instrument, e.reason.text_en))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("portfolio_snapshot_id: {}", portfolio.portfolio_snapshot_id);
    println!(
        "sum(targets) + cash = {:.8} + {:.8} = {:.8}",
        portfolio
            .targets
            .iter()
            .map(|t| t.target_weight)
            .sum::<f64>(),
        portfolio.cash_weight,
        portfolio
            .targets
            .iter()
            .map(|t| t.target_weight)
            .sum::<f64>()
            + portfolio.cash_weight
    );
}

fn fmt_opt(v: Option<f64>) -> String {
    match v {
        Some(x) => format!("{x:.6}"),
        None => "NULL".to_owned(),
    }
}

#[test]
fn qa_ranks_full_seed_universe_and_reruns_byte_identical() {
    let universe = real_universe();
    assert_eq!(universe.instruments.len(), 11);
    assert!(V1_SYMBOLS.iter().all(|s| {
        universe
            .instruments
            .contains(&InstrumentId::parse(&format!("{s}.KRX")).expect("valid id"))
    }));

    let a = run();
    let b = run();
    assert_eq!(a, b, "identical inputs -> identical portfolio");
    assert_eq!(
        serde_json::to_vec(&a).expect("serializes A"),
        serde_json::to_vec(&b).expect("serializes B"),
        "rerun must be byte-identical"
    );
    print_table(&a);
    println!("byte_identical_rerun: true");
}
