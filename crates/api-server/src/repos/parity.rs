//! Assembles the two sides of a backtest-vs-Paper parity comparison
//! (plan Todo 32).
//!
//! The comparison itself lives in `result_model::paper_parity` (pure, and
//! tested there). This module only fetches:
//!
//! - the **backtest side** from `recommendation_runs` + `recommendation_items`
//!   for the account's bound strategy config at the requested `as_of`;
//! - the **Paper side** from the `pending_targets` row that same close
//!   produced.
//!
//! Nothing is stored: a report is computed on read, so it can never go
//! stale against the lineage it describes.
//!
//! Fail-closed shape: when either side is missing, or the Paper target
//! records no `dataset_version` (rows queued before migration 0015), the
//! assembled sides deliberately differ so `evaluate_parity` returns
//! `NOT_COMPARABLE` — the report never claims comparability it cannot
//! prove.

use std::collections::BTreeMap;

use auth::entitlement::Actor;
use domain::provenance::{Engine, RandomSeed, RunProvenance};
use domain::version::{SemVer, StrategyVersion};
use domain::{CodeCommit, ContentHash, DatasetVersionId, InstrumentId, StrategyId, Weight, Zone};
use result_model::paper_parity::{ParityReport, SignalSet, evaluate_parity};
use uuid::Uuid;

use crate::actor_tx::begin_actor_tx;
use crate::error::{TenancyError, TenancyResult};

/// A sentinel dataset id used when a side's dataset is unknown. It can
/// never equal a real one, so the parity report degrades to
/// NOT_COMPARABLE rather than silently comparing across datasets.
const UNKNOWN_DATASET: &str = "unknown-dataset";

/// The raw rows of one side, before typing.
struct RawSide {
    strategy_id: String,
    strategy_version: String,
    dataset_version: String,
    weights: BTreeMap<String, String>,
}

/// Typed repository assembling parity reports.
#[derive(Debug, Clone)]
pub struct ParityRepo {
    pool: sqlx::PgPool,
}

impl ParityRepo {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    /// Builds the parity report for one account and session.
    pub async fn report(
        &self,
        actor: &Actor,
        account_id: Uuid,
        as_of: &str,
    ) -> TenancyResult<ParityReport> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;

        // --- the Paper side: the target that close produced ----------------
        let paper_row: Option<(Uuid, serde_json::Value, Option<String>, String, String)> =
            sqlx::query_as(
                "SELECT pt.strategy_config_id, pt.targets_json, pt.dataset_version, \
                        c.strategy_id, c.strategy_version \
                 FROM pending_targets pt \
                 JOIN user_strategy_configs c ON c.id = pt.strategy_config_id \
                 WHERE pt.account_id = $1 AND pt.computed_on = $2::date",
            )
            .bind(account_id)
            .bind(as_of)
            .fetch_optional(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?;

        // --- the backtest side: the recommendation run for that config -----
        let backtest_row: Option<(Uuid, String, String, Option<String>)> = match &paper_row {
            Some((config_id, _, _, _, _)) => sqlx::query_as(
                "SELECT r.id, c.strategy_id, c.strategy_version, \
                        r.summary_json->>'dataset_version' \
                 FROM recommendation_runs r \
                 JOIN user_strategy_configs c ON c.id = r.strategy_config_id \
                 WHERE r.strategy_config_id = $1 AND r.as_of = $2::date \
                   AND r.status = 'SUCCEEDED' \
                 ORDER BY r.created_at DESC LIMIT 1",
            )
            .bind(config_id)
            .bind(as_of)
            .fetch_optional(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?,
            None => None,
        };

        let backtest_weights: BTreeMap<String, String> = match &backtest_row {
            Some((run_id, _, _, _)) => {
                let rows: Vec<(String, Option<String>)> = sqlx::query_as(
                    "SELECT instrument_id, target_weight::text FROM recommendation_items \
                     WHERE recommendation_run_id = $1 AND excluded = false",
                )
                .bind(run_id)
                .fetch_all(&mut *tx)
                .await
                .map_err(TenancyError::from_sqlx)?;
                rows.into_iter()
                    .filter_map(|(id, w)| w.map(|w| (id, w)))
                    .collect()
            }
            None => BTreeMap::new(),
        };
        tx.commit().await.map_err(TenancyError::from_sqlx)?;

        let paper = match paper_row {
            Some((_, targets_json, dataset, strategy_id, strategy_version)) => RawSide {
                strategy_id,
                strategy_version,
                dataset_version: dataset.unwrap_or_else(|| UNKNOWN_DATASET.to_owned()),
                weights: weights_from_json(&targets_json),
            },
            None => missing_side("paper"),
        };
        let backtest = match backtest_row {
            Some((_, strategy_id, strategy_version, dataset)) => RawSide {
                strategy_id,
                strategy_version,
                dataset_version: dataset.unwrap_or_else(|| UNKNOWN_DATASET.to_owned()),
                weights: backtest_weights,
            },
            None => missing_side("backtest"),
        };

        Ok(evaluate_parity(
            &to_signal_set(&backtest, as_of),
            &to_signal_set(&paper, as_of),
        ))
    }
}

/// A side that does not exist. Its strategy id is deliberately unique per
/// side so the lineage comparison fails and the report reads
/// NOT_COMPARABLE instead of "match against nothing".
fn missing_side(which: &str) -> RawSide {
    RawSide {
        strategy_id: format!("missing-{which}"),
        strategy_version: "0.0.0".to_owned(),
        dataset_version: UNKNOWN_DATASET.to_owned(),
        weights: BTreeMap::new(),
    }
}

/// `pending_targets.targets_json` is the selector's wire shape:
/// `[{"instrument_id": "...", "weight": "0.600000"}, ...]`.
fn weights_from_json(value: &serde_json::Value) -> BTreeMap<String, String> {
    value
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    let id = row.get("instrument_id")?.as_str()?.to_owned();
                    let weight = row.get("weight")?.as_str()?.to_owned();
                    Some((id, weight))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Types a raw side, dropping any row whose id or weight the domain types
/// reject. A malformed row is never silently coerced into a comparable
/// value; it simply cannot participate, which surfaces as a divergence.
fn to_signal_set(side: &RawSide, as_of: &str) -> SignalSet {
    let targets: BTreeMap<InstrumentId, Weight> = side
        .weights
        .iter()
        .filter_map(|(id, w)| Some((InstrumentId::parse(id).ok()?, Weight::parse(w.trim()).ok()?)))
        .collect();
    SignalSet {
        provenance: RunProvenance {
            engine: Engine::NautilusTrader,
            engine_version: SemVer::parse("1.231.0").expect("pinned engine version parses"),
            strategy_id: StrategyId::parse(&side.strategy_id)
                .unwrap_or_else(|_| StrategyId::parse("unknown").expect("fallback id parses")),
            strategy_version: StrategyVersion::parse(&side.strategy_version)
                .unwrap_or_else(|_| StrategyVersion::parse("0.0.0").expect("fallback parses")),
            dataset_version: DatasetVersionId::parse(&side.dataset_version).unwrap_or_else(|_| {
                DatasetVersionId::parse(UNKNOWN_DATASET).expect("fallback dataset parses")
            }),
            config_hash: ContentHash::from_bytes(b"parity"),
            code_commit: CodeCommit::parse("0000000").expect("fallback commit parses"),
            random_seed: RandomSeed::new(0),
            timezone: Zone::SEOUL,
        },
        as_of: as_of.to_owned(),
        targets,
    }
}
