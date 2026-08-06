//! Shared test helpers for the Todo 21 robustness suite.
//!
//! Every robustness test file includes this module via `mod common;`; the
//! helpers build deterministic, `BacktestResult::validate`-clean fixtures so
//! tests focus on the robustness contract rather than fixture plumbing.

use domain::provenance::{Engine, RandomSeed, RunProvenance};
use domain::version::{SemVer, StrategyVersion};
use domain::{CodeCommit, ContentHash, Currency, DatasetVersionId, Money, StrategyId, Zone};

/// A deterministic parent provenance (dual_momentum 1.2.0 on the pinned
/// engine/data versions) used by the lineage and suite tests.
pub fn provenance() -> RunProvenance {
    RunProvenance {
        engine: Engine::NautilusTrader,
        engine_version: SemVer::parse("1.231.0").unwrap(),
        strategy_id: StrategyId::parse("dual_momentum").unwrap(),
        strategy_version: StrategyVersion::parse("1.2.0").unwrap(),
        dataset_version: DatasetVersionId::parse("kr-etf-daily-20260804.1").unwrap(),
        config_hash: ContentHash::from_bytes(b"parent-config"),
        code_commit: CodeCommit::parse("0123456789abcdef").unwrap(),
        random_seed: RandomSeed::new(42),
        timezone: Zone::SEOUL,
    }
}

/// The derived provenance: identical pinning to [`provenance`], different
/// config hash (the axis changed the configuration).
pub fn derived_provenance() -> RunProvenance {
    let mut p = provenance();
    p.config_hash = ContentHash::from_bytes(b"derived-config");
    p
}

/// Ten million KRW in scale-4 (the golden-scenario initial capital).
pub fn ten_million() -> Money {
    Money::parse("10000000.0000", Currency::KRW).unwrap()
}
