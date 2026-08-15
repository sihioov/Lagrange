//! Assembles the two sides of a backtest-vs-Paper parity comparison
//! (plan Todo 32).
//!
//! The comparison itself lives in `result_model::paper_parity` (pure, and
//! tested there). This module only fetches:
//!
//! - the **backtest side** from the target's exact
//!   `recommendation_run_id` + `recommendation_items` lineage;
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
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::actor_tx::{actor_uuid, begin_actor_tx};
use crate::error::{TenancyError, TenancyResult};
use crate::repos::pending_targets::PendingTargetRow;

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

type PaperParityRow = (
    Uuid,
    serde_json::Value,
    Option<String>,
    String,
    String,
    Option<Uuid>,
    Option<Uuid>,
    Option<String>,
);

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
        let paper_row: Option<PaperParityRow> = sqlx::query_as(
            "SELECT pt.strategy_config_id, pt.targets_json, pt.dataset_version, \
                    c.strategy_id, c.strategy_version, pt.recommendation_run_id, \
                    pt.dataset_version_id, pt.dataset_manifest_sha256 \
             FROM pending_targets pt \
             JOIN user_strategy_configs c ON c.id = pt.strategy_config_id \
             WHERE pt.account_id = $1 AND pt.computed_on = $2::date",
        )
        .bind(account_id)
        .bind(as_of)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;

        // --- the backtest side: the target's exact recommendation run ------
        let backtest_row: Option<(Uuid, String, String, Option<String>)> = match &paper_row {
            Some((_, _, _, _, _, Some(run_id), dataset_version_id, dataset_manifest_sha256)) => {
                sqlx::query_as(
                    "SELECT r.id, c.strategy_id, c.strategy_version, \
                            dataset.version \
                     FROM recommendation_runs r \
                     JOIN user_strategy_configs c \
                       ON c.id = r.strategy_config_id \
                      AND c.owner_user_id = r.owner_user_id \
                     JOIN dataset_versions dataset ON dataset.id = r.dataset_version_id \
                     WHERE r.id = $1 \
                       AND r.owner_user_id = $5 \
                       AND r.as_of = $2::date \
                       AND r.status = 'SUCCEEDED' \
                       AND r.dataset_version_id = $3 \
                       AND r.dataset_manifest_sha256 = $4 \
                     FOR SHARE OF r, c, dataset",
                )
                .bind(run_id)
                .bind(as_of)
                .bind(dataset_version_id)
                .bind(dataset_manifest_sha256)
                .bind(actor_uuid(actor)?)
                .fetch_optional(&mut *tx)
                .await
                .map_err(TenancyError::from_sqlx)?
            }
            _ => None,
        };

        let backtest_weights: BTreeMap<String, String> = match &backtest_row {
            Some((run_id, _, _, _)) => {
                let rows: Vec<(String, Option<String>)> = sqlx::query_as(
                    "SELECT instrument_id, target_weight::text FROM recommendation_items \
                     WHERE recommendation_run_id = $1 \
                       AND owner_user_id = ( \
                           SELECT owner_user_id FROM recommendation_runs WHERE id = $1 \
                       ) \
                       AND excluded = false \
                     FOR SHARE",
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
            Some((_, targets_json, dataset, strategy_id, strategy_version, _, _, _)) => RawSide {
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

    /// Build the settlement snapshot from the target's exact immutable run
    /// lineage while the caller holds `pending_targets FOR UPDATE` in the
    /// same transaction.  There is intentionally no config/as_of fallback:
    /// when the target has no exact recommendation_run_id, the backtest side
    /// is missing and parity is NOT_COMPARABLE.
    pub(crate) async fn report_for_target_tx(
        tx: &mut Transaction<'_, Postgres>,
        target: &PendingTargetRow,
    ) -> TenancyResult<ParityReport> {
        let paper_identity: Option<(String, String)> = sqlx::query_as(
            "SELECT strategy_id, strategy_version \
             FROM user_strategy_configs \
             WHERE id = $1 AND owner_user_id = $2 \
             FOR SHARE",
        )
        .bind(target.strategy_config_id)
        .bind(target.owner_user_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        let paper_row = match paper_identity {
            Some((strategy_id, strategy_version)) => RawSide {
                strategy_id,
                strategy_version,
                dataset_version: target
                    .dataset_version
                    .clone()
                    .unwrap_or_else(|| UNKNOWN_DATASET.to_owned()),
                weights: weights_from_json(&target.targets_json),
            },
            None => missing_side("paper"),
        };

        let backtest_row: Option<(Uuid, String, String, String)> =
            if let Some(run_id) = target.recommendation_run_id {
                sqlx::query_as(
                    "SELECT run.id, config.strategy_id, config.strategy_version, dataset.version \
                     FROM recommendation_runs AS run \
                     JOIN user_strategy_configs AS config \
                       ON config.id = run.strategy_config_id \
                      AND config.owner_user_id = run.owner_user_id \
                     JOIN dataset_versions AS dataset \
                       ON dataset.id = run.dataset_version_id \
                     WHERE run.id = $1 \
                       AND run.owner_user_id = $2 \
                       AND run.strategy_config_id = $3 \
                       AND run.as_of = $4 \
                       AND run.status = 'SUCCEEDED' \
                       AND run.dataset_version_id = $5 \
                       AND run.dataset_manifest_sha256 = $6 \
                       AND dataset.version = $7 \
                     FOR SHARE OF run, config, dataset",
                )
                .bind(run_id)
                .bind(target.owner_user_id)
                .bind(target.strategy_config_id)
                .bind(target.computed_on)
                .bind(target.dataset_version_id)
                .bind(&target.dataset_manifest_sha256)
                .bind(&target.dataset_version)
                .fetch_optional(&mut **tx)
                .await
                .map_err(TenancyError::from_sqlx)?
            } else {
                None
            };

        let backtest_weights = if let Some((run_id, _, _, _)) = backtest_row.as_ref() {
            let rows: Vec<(String, Option<String>)> = sqlx::query_as(
                "SELECT instrument_id, target_weight::text \
                 FROM recommendation_items \
                 WHERE recommendation_run_id = $1 \
                   AND owner_user_id = $2 \
                   AND excluded = false \
                 FOR SHARE",
            )
            .bind(run_id)
            .bind(target.owner_user_id)
            .fetch_all(&mut **tx)
            .await
            .map_err(TenancyError::from_sqlx)?;
            rows.into_iter()
                .filter_map(|(id, weight)| weight.map(|weight| (id, weight)))
                .collect()
        } else {
            BTreeMap::new()
        };

        let backtest = match backtest_row {
            Some((_, strategy_id, strategy_version, dataset_version)) => RawSide {
                strategy_id,
                strategy_version,
                dataset_version,
                weights: backtest_weights,
            },
            None => missing_side("backtest"),
        };
        Ok(evaluate_parity(
            &to_signal_set(&backtest, &target.computed_on.to_string()),
            &to_signal_set(&paper_row, &target.computed_on.to_string()),
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
        .filter_map(|(id, w)| {
            let weight = Weight::parse(w.trim()).ok()?;
            let instrument = InstrumentId::parse(id).ok()?;
            (!weight.is_zero()).then_some((instrument, weight))
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_weight_eligible_rows_do_not_create_false_parity_targets() {
        let side = RawSide {
            strategy_id: "dual_momentum".into(),
            strategy_version: "1.0.0".into(),
            dataset_version: "phase0-v2".into(),
            weights: BTreeMap::from([
                ("069500.KRX".into(), "1.000000".into()),
                ("229200.KRX".into(), "0.000000".into()),
            ]),
        };

        let signals = to_signal_set(&side, "2026-08-11");

        assert_eq!(signals.targets.len(), 1);
        assert!(
            signals
                .targets
                .contains_key(&InstrumentId::parse("069500.KRX").unwrap())
        );
        assert!(
            !signals
                .targets
                .contains_key(&InstrumentId::parse("229200.KRX").unwrap())
        );
    }
}
