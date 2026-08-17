//! Lease-supervised candidate computation and atomic publication.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use domain::{DatasetId, InstrumentId, TradingDate};
use factor_engine::bars::Bars;
use factor_engine::{
    CandidateAnalysis, CandidateInstrumentInput, CandidateSession, EvidenceStrength,
    FrozenUniverse, NormalizationScope, score_candidates, top_five,
};
use market_data::{CurateStore, FundamentalProfile, dataset_manifest_hash};
use serde_json::{Value, json};
use sqlx::PgPool;
use thiserror::Error;
use tokio::time::MissedTickBehavior;
use uuid::Uuid;

use super::input::{
    AttestedCandidateInput, CandidateInputError, CandidatePayload, attest_candidate_input,
};
use crate::error::{QueueError, database_error_class, queue_error_class};
use crate::queue::JobQueue;
use crate::types::{ClaimedJob, ErrorClass, HeartbeatStatus, JobStatus, SettleResult};

#[derive(Debug, Clone)]
pub struct CandidateRunnerPaths {
    /// Trusted data root. The DB-attested dataset root must canonicalize below
    /// this directory and may not traverse through a symlink outside it.
    pub data_root: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CandidateRunnerConfig {
    heartbeat_interval: Duration,
    lease: Duration,
}

impl CandidateRunnerConfig {
    pub fn new(
        heartbeat_interval: Duration,
        lease: Duration,
    ) -> Result<Self, CandidateRunnerError> {
        if heartbeat_interval.is_zero() || lease.is_zero() || heartbeat_interval >= lease {
            return Err(CandidateRunnerError::Config(
                "heartbeat and lease must be positive and heartbeat must precede lease".into(),
            ));
        }
        Ok(Self {
            heartbeat_interval,
            lease,
        })
    }

    pub const fn heartbeat_interval(&self) -> Duration {
        self.heartbeat_interval
    }

    pub const fn lease(&self) -> Duration {
        self.lease
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateOutcome {
    Idle,
    Succeeded { job_id: Uuid, run_id: Uuid },
    Retrying { job_id: Uuid, code: String },
    Blocked { job_id: Uuid, code: String },
    Failed { job_id: Uuid, code: String },
    LeaseLost { job_id: Uuid },
}

#[derive(Debug, Error)]
pub enum CandidateRunnerError {
    #[error("candidate runner configuration is invalid: {0}")]
    Config(String),
    #[error("candidate queue unavailable: {0}")]
    Queue(#[from] QueueError),
}

#[derive(Debug)]
struct StageFailure {
    class: ErrorClass,
    code: &'static str,
    message: &'static str,
}

impl StageFailure {
    const fn new(class: ErrorClass, code: &'static str, message: &'static str) -> Self {
        Self {
            class,
            code,
            message,
        }
    }
}

struct PreparedPublication {
    run_id: Uuid,
    snapshots: Value,
    summary: Value,
}

/// Claim and execute at most one `candidate_compute` job.
pub async fn run_once(
    pool: &PgPool,
    queue: &JobQueue,
    worker_id: &str,
    paths: &CandidateRunnerPaths,
    config: &CandidateRunnerConfig,
) -> Result<CandidateOutcome, CandidateRunnerError> {
    if config.lease != queue.config().lease {
        return Err(CandidateRunnerError::Config(
            "candidate runner lease does not match queue lease".into(),
        ));
    }
    let Some(claim) = queue.claim_next_for(worker_id, "candidate_compute").await? else {
        return Ok(CandidateOutcome::Idle);
    };
    if claim.job.job_type != "candidate_compute" {
        return settle_failure(
            pool,
            queue,
            &claim,
            StageFailure::new(
                ErrorClass::Integrity,
                "CANDIDATE_WRONG_JOB_TYPE",
                "candidate worker received the wrong job type",
            ),
        )
        .await;
    }

    let preparation = prepare(pool, &claim, paths);
    tokio::pin!(preparation);
    let mut heartbeat = tokio::time::interval(config.heartbeat_interval);
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);
    heartbeat.tick().await;
    let prepared = loop {
        tokio::select! {
            result = &mut preparation => break result,
            _ = heartbeat.tick() => {
                match queue.heartbeat(&claim).await {
                    Ok(HeartbeatStatus::Extended) => {}
                    Ok(HeartbeatStatus::Canceled) => {
                        return settle_failure(
                            pool,
                            queue,
                            &claim,
                            StageFailure::new(
                                ErrorClass::Input,
                                "CANDIDATE_CANCELED",
                                "candidate computation was canceled",
                            ),
                        ).await;
                    }
                    Ok(HeartbeatStatus::LeaseLost) => {
                        return Ok(CandidateOutcome::LeaseLost { job_id: claim.job.id });
                    }
                    Err(error) => {
                        let class = queue_error_class(&error);
                        return settle_failure(
                            pool,
                            queue,
                            &claim,
                            StageFailure::new(
                                class,
                                "CANDIDATE_HEARTBEAT_UNAVAILABLE",
                                "candidate lease heartbeat is unavailable",
                            ),
                        ).await;
                    }
                }
            }
        }
    };
    let prepared = match prepared {
        Ok(prepared) => prepared,
        Err(failure) => return settle_failure(pool, queue, &claim, failure).await,
    };

    match queue.heartbeat(&claim).await {
        Ok(HeartbeatStatus::Extended) => {}
        Ok(HeartbeatStatus::Canceled) => {
            return settle_failure(
                pool,
                queue,
                &claim,
                StageFailure::new(
                    ErrorClass::Input,
                    "CANDIDATE_CANCELED",
                    "candidate computation was canceled",
                ),
            )
            .await;
        }
        Ok(HeartbeatStatus::LeaseLost) => {
            return Ok(CandidateOutcome::LeaseLost {
                job_id: claim.job.id,
            });
        }
        Err(error) => {
            return settle_failure(
                pool,
                queue,
                &claim,
                StageFailure::new(
                    queue_error_class(&error),
                    "CANDIDATE_HEARTBEAT_UNAVAILABLE",
                    "candidate lease heartbeat is unavailable",
                ),
            )
            .await;
        }
    }

    match publish(queue, &claim, &prepared).await {
        Ok(()) => Ok(CandidateOutcome::Succeeded {
            job_id: claim.job.id,
            run_id: prepared.run_id,
        }),
        Err(error) => {
            let class = match &error {
                QueueError::Database(database) => database_error_class(database),
                _ => queue_error_class(&error),
            };
            settle_failure(
                pool,
                queue,
                &claim,
                StageFailure::new(
                    class,
                    "CANDIDATE_PUBLICATION_FAILED",
                    "candidate publication failed",
                ),
            )
            .await
        }
    }
}

async fn prepare(
    pool: &PgPool,
    claim: &ClaimedJob,
    paths: &CandidateRunnerPaths,
) -> Result<PreparedPublication, StageFailure> {
    let payload = CandidatePayload::try_from(claim.job.payload_json.clone()).map_err(map_input)?;
    let input = attest_candidate_input(pool, claim.job.id, claim.job.owner_user_id, payload)
        .await
        .map_err(map_input)?;
    let paths = paths.clone();
    tokio::task::spawn_blocking(move || compute(input, paths))
        .await
        .map_err(|_| {
            StageFailure::new(
                ErrorClass::Transient,
                "CANDIDATE_COMPUTE_UNAVAILABLE",
                "candidate computation task is unavailable",
            )
        })?
}

fn compute(
    input: AttestedCandidateInput,
    paths: CandidateRunnerPaths,
) -> Result<PreparedPublication, StageFailure> {
    let trusted_root = std::fs::canonicalize(&paths.data_root).map_err(|_| path_unavailable())?;
    let dataset_root =
        std::fs::canonicalize(&input.price.storage_path).map_err(|_| path_unavailable())?;
    if !trusted_root.is_dir() || !dataset_root.is_dir() || !dataset_root.starts_with(&trusted_root)
    {
        return Err(StageFailure::new(
            ErrorClass::Integrity,
            "CANDIDATE_DATASET_PATH_INTEGRITY",
            "candidate price dataset path is outside the trusted root",
        ));
    }

    let store = CurateStore::new(&dataset_root);
    attest_price_manifest(
        &store,
        &input.price.dataset_id,
        input.payload.price_curated_version,
        &input.payload.price_manifest_sha256,
    )?;

    let universe = FrozenUniverse::from_instruments(
        input.payload.universe_snapshot_id.to_string(),
        input.members.iter().map(|member| member.instrument.clone()),
    )
    .map_err(|_| factor_integrity())?;
    let as_of = TradingDate::parse(&input.payload.as_of_date.format("%Y-%m-%d").to_string())
        .map_err(|_| factor_integrity())?;
    let bars = Bars::from_curated(
        &store,
        "kr",
        &input.price.dataset_id,
        input.payload.price_curated_version,
        &universe,
        as_of,
    )
    .map_err(map_factor)?;

    let mut flow = BTreeMap::new();
    for row in &input.flows {
        flow.insert(
            (
                row.instrument.clone(),
                row.trade_date,
                row.investor_class.as_str(),
            ),
            row.net_amount,
        );
    }
    let mut fundamentals: BTreeMap<InstrumentId, BTreeMap<String, f64>> = BTreeMap::new();
    for row in &input.fundamentals {
        fundamentals
            .entry(row.instrument.clone())
            .or_default()
            .insert(row.metric.clone(), row.value);
    }
    let candidates: Vec<CandidateInstrumentInput> = input
        .members
        .iter()
        .map(|member| {
            let points = bars.points(&member.instrument).unwrap_or_default();
            let has_as_of_price = points.iter().any(|point| point.date == as_of);
            let sessions = points
                .iter()
                .map(|point| CandidateSession {
                    date: point.date,
                    close: point.close,
                    trading_value: point.trading_value.unwrap_or_default(),
                    foreign_net_amount: flow
                        .get(&(member.instrument.clone(), point.date, "FOREIGN"))
                        .copied(),
                    institution_net_amount: flow
                        .get(&(member.instrument.clone(), point.date, "INSTITUTION"))
                        .copied(),
                })
                .collect();
            let mut flags = member.flags;
            flags.data_stale |= !has_as_of_price;
            CandidateInstrumentInput {
                instrument: member.instrument.clone(),
                sector_code: member.sector_code.clone(),
                is_financial: member.fundamental_profile == FundamentalProfile::Financial,
                sessions,
                fundamentals: fundamentals.remove(&member.instrument).unwrap_or_default(),
                flags,
            }
        })
        .collect();
    let analyses = score_candidates(&input.scoring, &candidates).map_err(|_| factor_integrity())?;
    if top_five(&analyses).len() < 5 {
        return Err(StageFailure::new(
            ErrorClass::DataBlocked,
            "CANDIDATE_INSUFFICIENT_EVIDENCE",
            "fewer than five stocks satisfy the evidence gate",
        ));
    }
    build_publication(&input, &analyses)
}

fn attest_price_manifest(
    store: &CurateStore,
    dataset_id: &str,
    generation: u32,
    expected_sha256: &str,
) -> Result<(), StageFailure> {
    let dataset_id = DatasetId::parse(dataset_id).map_err(|_| factor_integrity())?;
    let manifest = store
        .read_dataset_manifest(&dataset_id, generation)
        .map_err(|_| path_unavailable())?
        .ok_or_else(path_unavailable)?;
    let computed = dataset_manifest_hash(&manifest).map_err(|_| factor_integrity())?;
    let computed_hex = computed
        .as_str()
        .strip_prefix("sha256:")
        .ok_or_else(factor_integrity)?;
    if manifest.dataset_id != dataset_id
        || manifest.version != generation
        || manifest.content_hash != computed
        || computed_hex != expected_sha256
        || manifest.bar_count == 0
    {
        return Err(StageFailure::new(
            ErrorClass::Integrity,
            "CANDIDATE_PRICE_MANIFEST_INTEGRITY",
            "candidate price manifest does not match the database pin",
        ));
    }
    Ok(())
}

fn build_publication(
    input: &AttestedCandidateInput,
    analyses: &[CandidateAnalysis],
) -> Result<PreparedPublication, StageFailure> {
    let profiles: BTreeMap<&InstrumentId, FundamentalProfile> = input
        .members
        .iter()
        .map(|member| (&member.instrument, member.fundamental_profile))
        .collect();
    let provenance = json!({
        "as_of_date": input.payload.as_of_date.format("%Y-%m-%d").to_string(),
        "cutoff_at": input.payload.cutoff_at,
        "universe_key": input.payload.universe_key,
        "input_identity_sha256": input.payload.input_identity_sha256,
        "scoring_config_version": input.payload.scoring_config_version,
        "scoring_config_sha256": input.payload.scoring_config_sha256,
        "universe_snapshot_id": input.payload.universe_snapshot_id,
        "universe_entitlement_id": input.payload.universe_entitlement_id,
        "price_dataset_version_id": input.payload.price_dataset_version_id,
        "price_entitlement_id": input.payload.price_entitlement_id,
        "price_curated_version": input.payload.price_curated_version,
        "price_manifest_sha256": input.payload.price_manifest_sha256,
        "status_dataset_version_id": input.payload.status_dataset_version_id,
        "status_entitlement_id": input.payload.status_entitlement_id,
        "status_manifest_sha256": input.payload.status_manifest_sha256,
        "flow_dataset_version_id": input.payload.flow_dataset_version_id,
        "flow_entitlement_id": input.payload.flow_entitlement_id,
        "flow_manifest_sha256": input.payload.flow_manifest_sha256,
        "fundamental_dataset_version_id": input.payload.fundamental_dataset_version_id,
        "fundamental_entitlement_id": input.payload.fundamental_entitlement_id,
        "fundamental_manifest_sha256": input.payload.fundamental_manifest_sha256,
        "sector_version_id": input.payload.sector_version_id,
        "sector_entitlement_id": input.payload.sector_entitlement_id,
    });
    let mut snapshots = Vec::with_capacity(analyses.len());
    for analysis in analyses {
        let flow = &analysis.axes["flow"];
        let fundamental = &analysis.axes["fundamental"];
        let technical = &analysis.axes["technical"];
        let normalized: Vec<NormalizationScope> = analysis
            .factors
            .values()
            .filter(|factor| factor.normalized.is_some())
            .map(|factor| factor.normalization_scope)
            .collect();
        let normalization_scope = if normalized.is_empty() {
            "UNAVAILABLE"
        } else if normalized.contains(&NormalizationScope::UniverseFallback) {
            "UNIVERSE_FALLBACK"
        } else {
            "SECTOR"
        };
        let profile = match profiles[&analysis.instrument] {
            FundamentalProfile::NonFinancial => "candidate-non-financial-v1",
            FundamentalProfile::Financial => "candidate-financial-v1",
            FundamentalProfile::Unsupported => "unsupported",
        };
        let eligible = analysis.is_top_five_eligible();
        let snapshot = json!({
            "instrument_id": analysis.instrument.to_string(),
            "sector_code": analysis.sector_code,
            "fundamental_profile": profile,
            "eligible": eligible,
            "exclusion_codes": analysis.exclusions,
            "flow_score": flow.score,
            "fundamental_score": fundamental.score,
            "technical_score": technical.score,
            "total_score": analysis.composite_score,
            "flow_coverage": flow.coverage,
            "fundamental_coverage": fundamental.coverage,
            "technical_coverage": technical.coverage,
            "evidence_strength": evidence_name(analysis.evidence_strength),
            "normalization_scope": normalization_scope,
            "factors_json": analysis.factors,
            "scenarios_json": analysis.scenarios,
            "provenance_json": provenance,
        });
        snapshots.push(snapshot);
    }
    let ranked = top_five(analyses);
    let summary = json!({
        "universe_key": input.payload.universe_key,
        "universe_count": analyses.len(),
        "eligible_count": analyses.iter().filter(|row| row.is_top_five_eligible()).count(),
        "top_five": ranked.iter().map(|row| row.instrument.to_string()).collect::<Vec<_>>(),
        "scoring_config_version": input.payload.scoring_config_version,
        "input_identity_sha256": input.payload.input_identity_sha256,
    });
    Ok(PreparedPublication {
        run_id: input.payload.run_id,
        snapshots: Value::Array(snapshots),
        summary,
    })
}

async fn publish(
    queue: &JobQueue,
    claim: &ClaimedJob,
    prepared: &PreparedPublication,
) -> Result<(), QueueError> {
    let mut tx = queue.begin().await?;
    let _: Uuid =
        sqlx::query_scalar("SELECT public.publish_candidate_analysis($1, $2, $3, $4, $5, $6)")
            .bind(prepared.run_id)
            .bind(claim.job.id)
            .bind(claim.attempt.attempt_no)
            .bind(&claim.worker_id)
            .bind(&prepared.snapshots)
            .bind(&prepared.summary)
            .fetch_one(&mut *tx)
            .await?;
    match queue.settle_success_in(&mut tx, claim).await? {
        SettleResult::Committed(job) if job.status == JobStatus::Succeeded => {}
        SettleResult::Canceled(_) => {
            tx.rollback().await?;
            return Err(QueueError::StaleClaim(claim.job.id));
        }
        SettleResult::Committed(_) => {
            tx.rollback().await?;
            return Err(QueueError::Internal(
                "candidate success settlement returned a non-success status".into(),
            ));
        }
    }
    tx.commit().await?;
    Ok(())
}

async fn settle_failure(
    _pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
    failure: StageFailure,
) -> Result<CandidateOutcome, CandidateRunnerError> {
    let mut tx = queue.begin().await?;
    let settlement = queue
        .settle_failure_in(&mut tx, claim, failure.class, failure.code, failure.message)
        .await?;
    let canceled = matches!(&settlement, SettleResult::Canceled(_));
    let (outcome, terminal_status) = match settlement {
        SettleResult::Committed(job) if job.status == JobStatus::Queued => (
            CandidateOutcome::Retrying {
                job_id: claim.job.id,
                code: failure.code.to_owned(),
            },
            None,
        ),
        SettleResult::Committed(job) if job.status == JobStatus::Failed => {
            let blocked = failure.class == ErrorClass::DataBlocked;
            (
                if blocked {
                    CandidateOutcome::Blocked {
                        job_id: claim.job.id,
                        code: failure.code.to_owned(),
                    }
                } else {
                    CandidateOutcome::Failed {
                        job_id: claim.job.id,
                        code: failure.code.to_owned(),
                    }
                },
                Some(if blocked { "BLOCKED" } else { "FAILED" }),
            )
        }
        SettleResult::Canceled(_) => (
            CandidateOutcome::Failed {
                job_id: claim.job.id,
                code: "CANDIDATE_CANCELED".to_owned(),
            },
            Some("FAILED"),
        ),
        SettleResult::Committed(_) => {
            tx.rollback().await.map_err(QueueError::Database)?;
            return Err(CandidateRunnerError::Queue(QueueError::Internal(
                "candidate failure settlement returned an invalid status".into(),
            )));
        }
    };
    if let Some(status) = terminal_status {
        let changed: bool =
            sqlx::query_scalar("SELECT public.fail_candidate_analysis_run($1, $2, $3, $4, $5, $6)")
                .bind(extract_run_id(&claim.job.payload_json))
                .bind(claim.job.id)
                .bind(status)
                .bind(if status == "BLOCKED" {
                    failure.code
                } else if canceled {
                    "CANDIDATE_CANCELED"
                } else {
                    failure.code
                })
                .bind(failure.message)
                .bind(json!({"code": failure.code, "message": failure.message}))
                .fetch_one(&mut *tx)
                .await
                .map_err(QueueError::Database)?;
        if !changed {
            tx.rollback().await.map_err(QueueError::Database)?;
            return Err(CandidateRunnerError::Queue(QueueError::Internal(
                "candidate terminal failure did not update its run".into(),
            )));
        }
    }
    tx.commit().await.map_err(QueueError::Database)?;
    Ok(outcome)
}

fn extract_run_id(payload: &Value) -> Uuid {
    payload
        .get("run_id")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .unwrap_or(Uuid::nil())
}

fn map_input(error: CandidateInputError) -> StageFailure {
    let class = match &error {
        CandidateInputError::Unavailable { .. } => ErrorClass::Transient,
        CandidateInputError::DataBlocked { .. } => ErrorClass::DataBlocked,
        CandidateInputError::Malformed { .. } | CandidateInputError::NotFound => ErrorClass::Input,
        CandidateInputError::Integrity { .. } => ErrorClass::Integrity,
    };
    StageFailure::new(class, error.code(), "candidate input attestation failed")
}

fn map_factor(error: factor_engine::FactorError) -> StageFailure {
    match error {
        factor_engine::FactorError::StoreIo { .. } => StageFailure::new(
            ErrorClass::Transient,
            "CANDIDATE_PRICE_DATA_UNAVAILABLE",
            "candidate price data is temporarily unavailable",
        ),
        factor_engine::FactorError::MissingData { .. } => StageFailure::new(
            ErrorClass::DataBlocked,
            "CANDIDATE_PRICE_DATA_BLOCKED",
            "candidate price data is incomplete",
        ),
        _ => factor_integrity(),
    }
}

const fn factor_integrity() -> StageFailure {
    StageFailure::new(
        ErrorClass::Integrity,
        "CANDIDATE_COMPUTE_INTEGRITY",
        "candidate computation failed integrity validation",
    )
}

const fn path_unavailable() -> StageFailure {
    StageFailure::new(
        ErrorClass::Transient,
        "CANDIDATE_DATASET_PATH_UNAVAILABLE",
        "candidate price dataset path is unavailable",
    )
}

const fn evidence_name(value: EvidenceStrength) -> &'static str {
    match value {
        EvidenceStrength::Strong => "STRONG",
        EvidenceStrength::Moderate => "MODERATE",
        EvidenceStrength::Weak => "WEAK",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{ContentHash, UtcTimestamp};
    use market_data::{Capability, DatasetManifest};

    #[test]
    fn evidence_names_are_stable() {
        assert_eq!(evidence_name(EvidenceStrength::Strong), "STRONG");
        assert_eq!(evidence_name(EvidenceStrength::Moderate), "MODERATE");
        assert_eq!(evidence_name(EvidenceStrength::Weak), "WEAK");
    }

    #[test]
    fn run_id_extraction_fails_closed() {
        assert_eq!(extract_run_id(&json!({})), Uuid::nil());
    }

    #[test]
    fn price_manifest_must_match_exact_dataset_generation_and_hash() {
        let temp = tempfile::tempdir().expect("temporary curated root");
        let store = CurateStore::new(temp.path());
        let dataset_id = DatasetId::parse("krx_eod_bars").expect("dataset id");
        let manifest = DatasetManifest {
            dataset_id: dataset_id.clone(),
            version: 7,
            capability: Capability::PriceReturnOnly,
            created_at: UtcTimestamp::parse_rfc3339("2026-08-14T07:00:00Z").expect("timestamp"),
            source_batches: Vec::new(),
            bar_count: 1,
            action_count: 0,
            content_hash: ContentHash::from_bytes(b"placeholder"),
        };
        let manifest = DatasetManifest {
            content_hash: dataset_manifest_hash(&manifest).expect("canonical hash"),
            ..manifest
        };
        store
            .write_dataset_manifest(&manifest)
            .expect("write manifest");
        let hash = manifest
            .content_hash
            .as_str()
            .strip_prefix("sha256:")
            .expect("sha256 prefix");

        assert!(attest_price_manifest(&store, "krx_eod_bars", 7, hash).is_ok());
        assert!(attest_price_manifest(&store, "krx_eod_bars", 8, hash).is_err());
        assert!(attest_price_manifest(&store, "wrong_dataset", 7, hash).is_err());
        assert!(attest_price_manifest(&store, "krx_eod_bars", 7, &"0".repeat(64)).is_err());
    }
}
