//! Run provenance metadata (system design §6.9 execution metadata).
//!
//! Every run — backtest, Paper, or Live — records its engine, pinned engine
//! version, strategy + version, dataset version, config hash, code commit,
//! random seed, and timezone so results are reproducible and attributable.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;
use crate::hash::{CodeCommit, ContentHash};
use crate::ids::{DatasetVersionId, StrategyId};
use crate::time::Zone;
use crate::version::{SemVer, StrategyVersion};

/// The execution engine of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    /// NautilusTrader (pinned 1.231.0) — backtest, Paper, and Live.
    NautilusTrader,
}

/// A deterministic random seed used by any stochastic step of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RandomSeed(u64);

impl RandomSeed {
    /// Wraps a seed value.
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// The seed value.
    pub fn value(&self) -> u64 {
        self.0
    }
}

impl fmt::Display for RandomSeed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for RandomSeed {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<u64>()
            .map(Self)
            .map_err(|_| DomainError::InvalidId {
                kind: "random_seed".to_owned(),
                value: s.to_owned(),
            })
    }
}

/// Immutable provenance of a single run (design §6.9 execution metadata).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunProvenance {
    /// The execution engine (`nautilustrader`).
    pub engine: Engine,
    /// The pinned engine version.
    pub engine_version: SemVer,
    /// The strategy package id (e.g. `dual_momentum`).
    pub strategy_id: StrategyId,
    /// The immutable strategy package version.
    pub strategy_version: StrategyVersion,
    /// The dataset version consumed by the run.
    pub dataset_version: DatasetVersionId,
    /// The strategy-configuration content hash.
    pub config_hash: ContentHash,
    /// The code commit that produced the run.
    pub code_commit: CodeCommit,
    /// The deterministic random seed.
    pub random_seed: RandomSeed,
    /// The timezone of the run's market data / session semantics.
    pub timezone: Zone,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provenance_matches_documented_shape() {
        let provenance = RunProvenance {
            engine: Engine::NautilusTrader,
            engine_version: SemVer::parse("1.231.0").unwrap(),
            strategy_id: StrategyId::parse("dual_momentum").unwrap(),
            strategy_version: StrategyVersion::parse("1.2.0").unwrap(),
            dataset_version: DatasetVersionId::parse("kr-etf-daily-20260804.1").unwrap(),
            config_hash: ContentHash::from_bytes(b"config"),
            code_commit: CodeCommit::parse("abcdef1234567").unwrap(),
            random_seed: RandomSeed::new(42),
            timezone: Zone::SEOUL,
        };
        let json = serde_json::to_string(&provenance).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["engine"], "nautilustrader");
        assert_eq!(value["engine_version"], "1.231.0");
        assert_eq!(value["strategy_id"], "dual_momentum");
        assert_eq!(value["strategy_version"], "1.2.0");
        assert_eq!(value["dataset_version"], "kr-etf-daily-20260804.1");
        assert_eq!(value["random_seed"], 42);
        assert_eq!(value["timezone"], "Asia/Seoul");
        // round-trip
        let back: RunProvenance = serde_json::from_str(&json).unwrap();
        assert_eq!(back, provenance);
    }
}
