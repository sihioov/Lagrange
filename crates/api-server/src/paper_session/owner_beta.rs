//! Owner-bound R5 beta preview worker integration.
//!
//! This path accepts no caller supplied identity. The actual protected
//! process and Current scope are retained by `run_bound_paper_action`; SQL
//! remains the authority for consent, source lineage, lease, and account
//! fencing.

use std::{collections::BTreeMap, time::Duration};

use chrono::{FixedOffset, NaiveDate, Utc};
use domain::{ContentHash, Currency, InstrumentId, Money, Quantity, TradingDate, Weight};
use job_queue::{
    JobQueue,
    owner_beta::paper::{
        action::{
            BoundPaperActionError, BoundPaperActionErrorKind, PaperActionContext,
            PaperActionFuture, PaperActionWork, run_bound_paper_action,
        },
        calculation::{
            AvailabilityDecision, BetaAccountStateIdentity, BetaExecutionDisposition,
            BetaNoTradeReason, BetaPlanStatus, PriceUse, calculate_beta_preview,
            execute_beta_session_open, validate_price_availability, value_beta_close,
        },
        contract::{PaperEligibilityDecision, UntrustedPaperContractV1},
        evidence::financial::{
            ObservedPaperCalculationInputs, PaperCalculationInputRequest, PaperSessionObservation,
            read_paper_calculation_inputs,
        },
        runtime::PaperProcessRole,
    },
    paper_execution::set_paper_transaction_timeouts,
    paper_preview::{
        PreviewDecisionV1, PreviewFinancialPresentationV1, PreviewOrderV1, account_state_sha256,
        present_sizing_report,
    },
    types::{ClaimedJob, ErrorClass, HeartbeatStatus},
};
use portfolio_model::{CostProfile, LedgerState, PendingTarget, Side, sizing::TargetAllocation};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tokio::{sync::watch, task::JoinHandle};
use uuid::Uuid;

use crate::http::dto::{
    OwnerBetaPaperCalculationResultDto, OwnerBetaPaperObservedPriceDto, OwnerBetaPaperPlanStatus,
};

const ACTION_TIMEOUT: Duration = Duration::from_secs(90);
const HEARTBEAT_PERIOD: Duration = Duration::from_secs(10);
#[derive(Clone, Copy)]
enum LifecycleStage {
    BoundBegin,
    BoundReturn,
    CallbackBegin,
    CallbackReturn,
    DueBegin,
    DueReturn,
    SettleCaptureBegin,
    SettleCaptureReturn,
    SettleCalculated,
    SettleSqlBegin,
    SettleSqlReply,
    SettleCommitReturn,
    ValueCaptureBegin,
    ValueCaptureReturn,
    ValueCalculated,
    ValueSqlBegin,
    ValueSqlReply,
    ValueCommitReturn,
    LatestCaptureBegin,
    LatestCaptureReturn,
    AutoQueueReturn,
    ClaimCaptureBegin,
    ClaimCaptureReturn,
    PublishReply,
}

impl LifecycleStage {
    fn code(self) -> &'static str {
        match self {
            Self::BoundBegin => "BETA_BOUND_BEGIN",
            Self::BoundReturn => "BETA_BOUND_RETURN",
            Self::CallbackBegin => "BETA_CALLBACK_BEGIN",
            Self::CallbackReturn => "BETA_CALLBACK_RETURN",
            Self::DueBegin => "BETA_DUE_BEGIN",
            Self::DueReturn => "BETA_DUE_RETURN",
            Self::SettleCaptureBegin => "BETA_SETTLE_CAPTURE_BEGIN",
            Self::SettleCaptureReturn => "BETA_SETTLE_CAPTURE_RETURN",
            Self::SettleCalculated => "BETA_SETTLE_CALCULATED",
            Self::SettleSqlBegin => "BETA_SETTLE_SQL_BEGIN",
            Self::SettleSqlReply => "BETA_SETTLE_SQL_REPLY",
            Self::SettleCommitReturn => "BETA_SETTLE_COMMIT_RETURN",
            Self::ValueCaptureBegin => "BETA_VALUE_CAPTURE_BEGIN",
            Self::ValueCaptureReturn => "BETA_VALUE_CAPTURE_RETURN",
            Self::ValueCalculated => "BETA_VALUE_CALCULATED",
            Self::ValueSqlBegin => "BETA_VALUE_SQL_BEGIN",
            Self::ValueSqlReply => "BETA_VALUE_SQL_REPLY",
            Self::ValueCommitReturn => "BETA_VALUE_COMMIT_RETURN",
            Self::LatestCaptureBegin => "BETA_LATEST_CAPTURE_BEGIN",
            Self::LatestCaptureReturn => "BETA_LATEST_CAPTURE_RETURN",
            Self::AutoQueueReturn => "BETA_AUTO_QUEUE_RETURN",
            Self::ClaimCaptureBegin => "BETA_CLAIM_CAPTURE_BEGIN",
            Self::ClaimCaptureReturn => "BETA_CLAIM_CAPTURE_RETURN",
            Self::PublishReply => "BETA_PUBLISH_REPLY",
        }
    }
}

fn trace_lifecycle(stage: LifecycleStage) {
    tracing::debug!(target: "owner_beta_paper_lifecycle", stage = stage.code());
}

const PREVIEW_SCHEMA: &str = "owner-beta-paper-preview-v1";
const RESULT_SCHEMA: &str = "owner-beta-paper-preview-result-v1";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct OwnerBetaCycleOutcome {
    pub preview_published: bool,
    pub targets_seen: usize,
    pub targets_settled: usize,
    pub valuations_written: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum OwnerBetaPaperError {
    #[error("PAPER_BETA_RUNTIME_REJECTED")]
    Runtime,
    #[error("PAPER_BETA_CAPTURE_REJECTED")]
    Capture,
    #[error("PAPER_BETA_SQL_REJECTED")]
    Sql,
    #[error("PAPER_BETA_SNAPSHOT_INVALID")]
    Snapshot,
    #[error("PAPER_BETA_CALCULATION_REJECTED")]
    Calculation,
    #[error("PAPER_BETA_RETRY_REQUIRED")]
    Retry,
    #[error("PAPER_BETA_LEASE_LOST")]
    LeaseLost,
    #[error("PAPER_BETA_COMMIT_UNKNOWN_READBACK_REQUIRED")]
    CommitUnknown,
}

impl OwnerBetaPaperError {
    fn terminal_failure(self) -> Option<(ErrorClass, &'static str, &'static str)> {
        match self {
            Self::Snapshot => Some((
                ErrorClass::Integrity,
                "PAPER_BETA_SNAPSHOT_INVALID",
                "Owner-beta Paper snapshot failed closed validation",
            )),
            Self::Calculation => Some((
                ErrorClass::Determinism,
                "PAPER_BETA_CALCULATION_REJECTED",
                "Owner-beta Paper calculation failed closed validation",
            )),
            _ => None,
        }
    }
}

pub(crate) async fn run_preview_once(
    pool: &PgPool,
    queue: &JobQueue,
    worker_id: &str,
    shutdown: Option<watch::Receiver<bool>>,
) -> Result<OwnerBetaCycleOutcome, OwnerBetaPaperError> {
    let work = OwnerBetaPreviewWork {
        pool: pool.clone(),
        queue: queue.clone(),
        worker_id: worker_id.to_owned(),
    };
    trace_lifecycle(LifecycleStage::BoundBegin);
    let result =
        run_bound_paper_action(PaperProcessRole::Worker, ACTION_TIMEOUT, shutdown, work).await;
    trace_lifecycle(LifecycleStage::BoundReturn);
    match result {
        Ok(result) => Ok(result),
        Err(error) => Err(map_bound_action_error(error)),
    }
}

fn map_bound_action_error(
    error: BoundPaperActionError<OwnerBetaPaperError>,
) -> OwnerBetaPaperError {
    map_bound_action_error_parts(error.kind(), error.work_error().copied())
}

fn map_bound_action_error_parts(
    kind: BoundPaperActionErrorKind,
    work_error: Option<OwnerBetaPaperError>,
) -> OwnerBetaPaperError {
    if matches!(
        kind,
        BoundPaperActionErrorKind::CommitUnknown | BoundPaperActionErrorKind::CompletionUnclean
    ) {
        OwnerBetaPaperError::CommitUnknown
    } else {
        work_error.unwrap_or(OwnerBetaPaperError::Runtime)
    }
}

struct OwnerBetaPreviewWork {
    pool: PgPool,
    queue: JobQueue,
    worker_id: String,
}

impl PaperActionWork<OwnerBetaCycleOutcome, OwnerBetaPaperError> for OwnerBetaPreviewWork {
    fn run<'ctx, 'scope, 'attempt>(
        self: Box<Self>,
        context: &'ctx PaperActionContext<'ctx, 'scope, 'attempt>,
    ) -> PaperActionFuture<'ctx, OwnerBetaCycleOutcome, OwnerBetaPaperError> {
        Box::pin(async move {
            trace_lifecycle(LifecycleStage::CallbackBegin);
            let result = self.run_scoped(context).await;
            trace_lifecycle(LifecycleStage::CallbackReturn);
            result
        })
    }
}

impl OwnerBetaPreviewWork {
    async fn run_scoped(
        self,
        context: &PaperActionContext<'_, '_, '_>,
    ) -> Result<OwnerBetaCycleOutcome, OwnerBetaPaperError> {
        let runtime = context.runtime();
        let owner = parse_runtime_uuid(runtime.owner_user_id())?;
        let account = parse_runtime_uuid(runtime.account_id())?;
        let strategy = parse_runtime_uuid(runtime.strategy_config_id())?;
        if owner.is_nil() || account.is_nil() || strategy.is_nil() || runtime.control_epoch() < 0 {
            return Err(OwnerBetaPaperError::Runtime);
        }

        // Recover only this Owner's expired beta preview attempts. Generic
        // queue sweep intentionally excludes this job type.
        self.queue
            .sweep_paper_for_owner(owner)
            .await
            .map_err(|_| OwnerBetaPaperError::Sql)?;

        let selector = selector_from_current(context)?;
        // Resolve the earliest due session before claiming or publishing a
        // fresh T preview. Settlement changes the account fingerprint that a
        // later preview must snapshot.
        trace_lifecycle(LifecycleStage::DueBegin);
        let due = self.run_targets_once(context, &selector).await;
        trace_lifecycle(LifecycleStage::DueReturn);
        let (targets_seen, targets_settled, valuations_written) = due?;
        if targets_settled > 0 || valuations_written > 0 {
            // Completed due progress is one worker turn. The next normal run
            // reloads protected authority and captures fresh T against the updated account.
            return Ok(OwnerBetaCycleOutcome {
                targets_seen,
                targets_settled,
                valuations_written,
                ..OwnerBetaCycleOutcome::default()
            });
        }
        self.retry_ready_auto_preview_once(context).await?;

        let scan = self.scan_targets(context).await?;
        validate_target_scan(&scan)?;
        if scan.state != "ENABLED" || has_due_unvalued_target(&scan) {
            return Ok(OwnerBetaCycleOutcome {
                targets_seen,
                targets_settled,
                valuations_written,
                ..OwnerBetaCycleOutcome::default()
            });
        }

        // Capture only after the due-target pass. This real current-T run is
        // the source for AUTO queueing; the subsequent preview snapshot reads
        // the now-current durable account preimage.
        trace_lifecycle(LifecycleStage::LatestCaptureBegin);
        let initial = capture(
            &self.pool,
            &selector,
            &PaperCalculationInputRequest::new([], None),
        )
        .await;
        trace_lifecycle(LifecycleStage::LatestCaptureReturn);
        let initial = initial?;
        validate_capture_current(context, &initial)?;
        let latest = latest_session(&initial)?;
        let today = Utc::now()
            .with_timezone(&FixedOffset::east_opt(9 * 60 * 60).expect("fixed Seoul offset"))
            .date_naive();
        if latest.snapshot().as_of().as_naive_date() == today {
            context
                .revalidate()
                .map_err(|_| OwnerBetaPaperError::Runtime)?;
            let auto: Result<Value, sqlx::Error> =
                sqlx::query_scalar("SELECT queue_owner_beta_paper_target_v1($1,$2,$3)")
                    .bind(latest.input().run_id())
                    .bind(runtime.control_epoch())
                    .bind(runtime.config_sha256())
                    .fetch_one(&self.pool)
                    .await;
            trace_lifecycle(LifecycleStage::AutoQueueReturn);
            let auto = auto.map_err(|_| OwnerBetaPaperError::Sql)?;
            let queued = decode_action(auto, "QUEUE_TARGET")?;
            if !matches!(
                queued.outcome.as_str(),
                "QUEUED" | "REPLAY" | "WAITING_CALENDAR" | "CONSENT_REQUIRED"
            ) {
                return Err(OwnerBetaPaperError::Snapshot);
            }
        }

        context
            .revalidate()
            .map_err(|_| OwnerBetaPaperError::Runtime)?;
        let claim = self
            .queue
            .claim_next_paper_for_owner(&self.worker_id, owner)
            .await
            .map_err(|_| OwnerBetaPaperError::Sql)?;
        let preview_published = if let Some(claim) = claim {
            let mut heartbeat = ClaimHeartbeat::start(self.queue.clone(), claim.clone());
            let result = self
                .process_claim(context, &claim, &selector, &heartbeat.lost)
                .await;
            heartbeat.stop().await;
            match result {
                Err(error) if error.terminal_failure().is_some() => {
                    self.fail_claim(context, &claim, error).await?;
                    return Err(error);
                }
                Err(error) => return Err(error),
                Ok(()) => true,
            }
        } else {
            false
        };
        Ok(OwnerBetaCycleOutcome {
            preview_published,
            targets_seen,
            targets_settled,
            valuations_written,
        })
    }

    async fn process_claim(
        &self,
        context: &PaperActionContext<'_, '_, '_>,
        claim: &ClaimedJob,
        selector: &UntrustedPaperContractV1,
        claim_lost: &watch::Receiver<bool>,
    ) -> Result<(), OwnerBetaPaperError> {
        ensure_claim_live(claim_lost)?;
        context
            .revalidate()
            .map_err(|_| OwnerBetaPaperError::Runtime)?;
        let snapshot_value: Value =
            sqlx::query_scalar("SELECT snapshot_owner_beta_paper_preview_v1($1,$2,$3)")
                .bind(
                    claim.job.payload_json["preview_id"]
                        .as_str()
                        .and_then(|s| Uuid::parse_str(s).ok())
                        .ok_or(OwnerBetaPaperError::Snapshot)?,
                )
                .bind(claim.job.id)
                .bind(context.runtime().config_sha256())
                .fetch_one(&self.pool)
                .await
                .map_err(|_| OwnerBetaPaperError::Sql)?;
        let envelope = decode_action(snapshot_value, "SNAPSHOT_PREVIEW")?;
        if envelope.outcome != "OK" {
            return Err(if envelope.outcome == "STALE" {
                OwnerBetaPaperError::Retry
            } else {
                OwnerBetaPaperError::Snapshot
            });
        }
        let snapshot: PreviewSnapshot =
            serde_json::from_value(envelope.data).map_err(|_| OwnerBetaPaperError::Snapshot)?;
        validate_snapshot_identity(context, claim, &snapshot)?;

        let t_date = parse_date(&snapshot.signal_date)?;
        let t =
            TradingDate::parse(&snapshot.signal_date).map_err(|_| OwnerBetaPaperError::Snapshot)?;
        let e = TradingDate::parse(&snapshot.effective_session)
            .map_err(|_| OwnerBetaPaperError::Snapshot)?;
        trace_lifecycle(LifecycleStage::ClaimCaptureBegin);
        let captured = capture(
            &self.pool,
            selector,
            &PaperCalculationInputRequest::new([t_date], Some((t, e))),
        )
        .await;
        trace_lifecycle(LifecycleStage::ClaimCaptureReturn);
        let captured = captured?;
        ensure_claim_live(claim_lost)?;
        validate_capture_current(context, &captured)?;
        let observation = captured
            .sessions()
            .get(&t_date)
            .ok_or(OwnerBetaPaperError::Capture)?;
        validate_snapshot_capture(&snapshot, observation, &captured, context)?;
        let cash_text = snapshot.account_state_preimage["cash"]
            .as_str()
            .ok_or(OwnerBetaPaperError::Snapshot)?;
        let positions_value = &snapshot.account_state_preimage["positions"];
        let verified_hash =
            account_state_sha256(snapshot.account_state_version, cash_text, positions_value)
                .map_err(|_| OwnerBetaPaperError::Snapshot)?;
        if verified_hash != snapshot.account_state_sha256 {
            return Err(OwnerBetaPaperError::Snapshot);
        }
        if snapshot.account_state_preimage["schema_version"] != 1
            || snapshot.account_state_preimage["paper_state_version"]
                != snapshot.account_state_version
        {
            return Err(OwnerBetaPaperError::Snapshot);
        }
        let cash =
            Money::parse(cash_text, Currency::KRW).map_err(|_| OwnerBetaPaperError::Snapshot)?;
        let positions = parse_positions(positions_value)?;
        let profile = CostProfile::resolve(&snapshot.cost_profile_id)
            .map_err(|_| OwnerBetaPaperError::Snapshot)?;
        if profile.version != snapshot.cost_profile_version as u32 {
            return Err(OwnerBetaPaperError::Snapshot);
        }
        let state = LedgerState {
            positions,
            ..LedgerState::new(cash, profile.clone())
        };
        let previous = parse_targets(snapshot.previous_target_weights.as_ref())?;
        let account_hash = ContentHash::parse(&format!("sha256:{}", snapshot.account_state_sha256))
            .map_err(|_| OwnerBetaPaperError::Snapshot)?;
        let calculated = calculate_beta_preview(
            observation.snapshot(),
            observation.input(),
            observation.target(),
            BetaAccountStateIdentity {
                account_id: snapshot.account_id,
                state_version: snapshot.account_state_version,
                state_sha256: account_hash,
            },
            previous.as_deref(),
            &state,
        )
        .map_err(|_| OwnerBetaPaperError::Calculation)?;
        ensure_claim_live(claim_lost)?;
        let allocation_hash = snapshot.signal_allocation_sha256.as_str();
        let result = present_beta_result(
            &calculated,
            observation,
            &state,
            &profile,
            &snapshot.target_snapshot_sha256,
            allocation_hash,
        )?;
        let preview_token = preview_token(&snapshot, &result)?;
        let publication = json!({
            "schema": "owner-beta-paper-preview-v1",
            "preview_id": snapshot.preview_id,
            "lineage_id": snapshot.lineage_id,
            "apply_mode": snapshot.apply_mode,
            "control_epoch": snapshot.control_epoch,
            "attempt_id": snapshot.attempt_id,
            "attempt_no": snapshot.attempt_no,
            "worker_id": snapshot.worker_id,
            "process_config_sha256": context.runtime().config_sha256(),
            "account_state_version": snapshot.account_state_version,
            "account_state_sha256": snapshot.account_state_sha256,
            "account_state_preimage": snapshot.account_state_preimage,
            "cost_profile_id": snapshot.cost_profile_id,
            "cost_profile_version": snapshot.cost_profile_version,
            "target_snapshot_sha256": snapshot.target_snapshot_sha256,
            "signal_allocation_sha256": snapshot.signal_allocation_sha256,
            "preview_token": preview_token,
            "result": result,
        });
        let bytes = serde_json::to_vec(&publication).map_err(|_| OwnerBetaPaperError::Snapshot)?;
        if bytes.len() > 262_144 {
            return Err(OwnerBetaPaperError::Snapshot);
        }

        context
            .revalidate()
            .map_err(|_| OwnerBetaPaperError::Runtime)?;
        ensure_claim_live(claim_lost)?;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| OwnerBetaPaperError::Sql)?;
        set_paper_transaction_timeouts(&mut tx)
            .await
            .map_err(|_| OwnerBetaPaperError::Sql)?;
        context
            .revalidate()
            .map_err(|_| OwnerBetaPaperError::Runtime)?;
        ensure_claim_live(claim_lost)?;
        self.queue
            .lock_claim_in(&mut tx, claim)
            .await
            .map_err(|_| OwnerBetaPaperError::LeaseLost)?;
        context
            .revalidate()
            .map_err(|_| OwnerBetaPaperError::Runtime)?;
        let result_value: Result<Value, sqlx::Error> =
            sqlx::query_scalar("SELECT publish_owner_beta_paper_preview_v1($1,$2,$3,$4)")
                .bind(snapshot.preview_id)
                .bind(snapshot.job_id)
                .bind(publication)
                .bind(context.runtime().config_sha256())
                .fetch_one(&mut *tx)
                .await;
        trace_lifecycle(LifecycleStage::PublishReply);
        let result_value = result_value.map_err(|_| OwnerBetaPaperError::Sql)?;
        let published = decode_action(result_value, "PUBLISH_PREVIEW")?;
        if !matches!(published.outcome.as_str(), "OK" | "WAITING_EARLIER_SESSION") {
            tx.rollback()
                .await
                .map_err(|_| OwnerBetaPaperError::CommitUnknown)?;
            return Err(if published.outcome == "STALE" {
                OwnerBetaPaperError::Retry
            } else {
                OwnerBetaPaperError::Snapshot
            });
        }
        context
            .revalidate()
            .map_err(|_| OwnerBetaPaperError::Runtime)?;
        self.queue
            .settle_success_in(&mut tx, claim)
            .await
            .map_err(|_| OwnerBetaPaperError::LeaseLost)?;
        match tx.commit().await {
            Ok(()) => Ok(()),
            Err(_) => {
                // Publication and queue success share one transaction. A
                // durable SUCCEEDED readback therefore proves that the
                // publication committed without reading beta tables.
                let status: Option<String> = sqlx::query_scalar(
                    "SELECT status FROM jobs
                     WHERE id = $1 AND owner_user_id = $2
                       AND job_type = 'owner_beta_paper_preview'",
                )
                .bind(claim.job.id)
                .bind(claim.job.owner_user_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|_| OwnerBetaPaperError::CommitUnknown)?;
                if status.as_deref() == Some("SUCCEEDED") {
                    Ok(())
                } else {
                    Err(OwnerBetaPaperError::CommitUnknown)
                }
            }
        }
    }

    async fn fail_claim(
        &self,
        context: &PaperActionContext<'_, '_, '_>,
        claim: &ClaimedJob,
        error: OwnerBetaPaperError,
    ) -> Result<(), OwnerBetaPaperError> {
        let (class, code, message) = error.terminal_failure().ok_or(error)?;
        let preview_id = claim.job.payload_json["preview_id"]
            .as_str()
            .and_then(|value| Uuid::parse_str(value).ok())
            .filter(|id| !id.is_nil())
            .ok_or(OwnerBetaPaperError::Snapshot)?;
        context
            .revalidate()
            .map_err(|_| OwnerBetaPaperError::Runtime)?;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| OwnerBetaPaperError::Sql)?;
        set_paper_transaction_timeouts(&mut tx)
            .await
            .map_err(|_| OwnerBetaPaperError::Sql)?;
        let failure = json!({
            "code": code,
            "message": message,
            "attempt_id": claim.attempt.id,
            "attempt_no": claim.attempt.attempt_no,
            "worker_id": claim.worker_id,
            "process_config_sha256": context.runtime().config_sha256(),
        });
        context
            .revalidate()
            .map_err(|_| OwnerBetaPaperError::Runtime)?;
        let value: Value =
            sqlx::query_scalar("SELECT fail_owner_beta_paper_preview_v1($1,$2,$3,$4)")
                .bind(preview_id)
                .bind(claim.job.id)
                .bind(failure)
                .bind(context.runtime().config_sha256())
                .fetch_one(&mut *tx)
                .await
                .map_err(|_| OwnerBetaPaperError::Sql)?;
        let envelope = decode_action(value, "FAIL_PREVIEW")?;
        if envelope.outcome != "OK" {
            tx.rollback()
                .await
                .map_err(|_| OwnerBetaPaperError::CommitUnknown)?;
            return Err(OwnerBetaPaperError::LeaseLost);
        }
        self.queue
            .settle_failure_in(&mut tx, claim, class, code, message)
            .await
            .map_err(|_| OwnerBetaPaperError::LeaseLost)?;
        tx.commit()
            .await
            .map_err(|_| OwnerBetaPaperError::CommitUnknown)?;
        Ok(())
    }

    async fn run_targets_once(
        &self,
        context: &PaperActionContext<'_, '_, '_>,
        selector: &UntrustedPaperContractV1,
    ) -> Result<(usize, usize, usize), OwnerBetaPaperError> {
        let first_scan = self.scan_targets(context).await?;
        validate_target_scan(&first_scan)?;
        if first_scan.state != "ENABLED" {
            return Ok((0, 0, 0));
        }
        let targets_seen = first_scan.targets.len();
        let Some(target_row) = first_scan.targets.first() else {
            return Ok((targets_seen, 0, 0));
        };
        if target_row.effective_session > first_scan.today {
            return Ok((targets_seen, 0, 0));
        }
        context
            .revalidate()
            .map_err(|_| OwnerBetaPaperError::Runtime)?;
        let value: Value =
            sqlx::query_scalar("SELECT snapshot_owner_beta_paper_target_v1($1,$2,$3)")
                .bind(target_row.pending_target_id)
                .bind(context.runtime().control_epoch())
                .bind(context.runtime().config_sha256())
                .fetch_one(&self.pool)
                .await
                .map_err(|_| OwnerBetaPaperError::Sql)?;
        let envelope = decode_action(value, "SNAPSHOT_TARGET")?;
        if !matches!(
            envelope.outcome.as_str(),
            "OK" | "WAITING_SESSION" | "WAITING_ADMISSION"
        ) {
            return Err(OwnerBetaPaperError::Snapshot);
        }
        let snapshot: TargetSnapshot =
            serde_json::from_value(envelope.data).map_err(|_| OwnerBetaPaperError::Snapshot)?;
        validate_target_snapshot(context, &snapshot)?;
        if snapshot.pending_target_id != target_row.pending_target_id
            || snapshot.lineage_id != target_row.lineage_id
            || snapshot.signal_date != target_row.signal_date
            || snapshot.effective_session != target_row.effective_session
            || snapshot.session_state != target_row.state
            || snapshot.signal_admission_sha256 != target_row.signal_admission_sha256
            || snapshot.target_snapshot_sha256 != target_row.target_snapshot_sha256
            || !execution_admission_matches_scan(
                target_row.execution_admission_sha256.as_deref(),
                snapshot.execution_admission_sha256.as_deref(),
            )
            || snapshot.settlement_sha256 != target_row.settlement_sha256
            || snapshot.valuation_sha256 != target_row.valuation_sha256
        {
            return Err(OwnerBetaPaperError::Snapshot);
        }
        match envelope.outcome.as_str() {
            "WAITING_SESSION" => return Ok((targets_seen, 0, 0)),
            "WAITING_ADMISSION" => {
                self.record_target_wait(context, &snapshot, "WAITING_ADMISSION")
                    .await?;
                return Ok((targets_seen, 0, 0));
            }
            _ => {}
        }

        let mut settled = 0;
        if snapshot.settlement_sha256.is_none() {
            if matches!(snapshot.session_state.as_str(), "BLOCKED" | "VALUED") {
                return Ok((targets_seen, 0, 0));
            }
            if snapshot.execution_admission_sha256.is_none() {
                self.record_target_wait(context, &snapshot, "WAITING_ADMISSION")
                    .await?;
                return Ok((targets_seen, 0, 0));
            }
            if self.settle_target(context, selector, &snapshot).await? {
                settled = 1;
            } else {
                return Ok((targets_seen, settled, 0));
            }
        }

        // Always reacquire the SQL projection after settlement, including
        // when the COMMIT acknowledgement was lost. The immutable receipt is
        // the sole authority for deciding whether valuation may continue.
        let fresh = self
            .snapshot_target(context, snapshot.pending_target_id)
            .await?;
        validate_target_snapshot(context, &fresh)?;
        if fresh.settlement_sha256.is_none() {
            return Err(OwnerBetaPaperError::CommitUnknown);
        }
        if fresh.valuation_sha256.is_some() {
            return Ok((targets_seen, settled, 0));
        }
        let valued = self.value_target(context, selector, &fresh).await?;
        Ok((targets_seen, settled, if valued { 1 } else { 0 }))
    }

    async fn retry_ready_auto_preview_once(
        &self,
        context: &PaperActionContext<'_, '_, '_>,
    ) -> Result<(), OwnerBetaPaperError> {
        let scan = self.scan_targets(context).await?;
        validate_target_scan(&scan)?;
        if scan.state != "ENABLED" || has_due_unvalued_target(&scan) {
            return Ok(());
        }
        let Some(preview) = scan.auto_previews.iter().find(|row| {
            row.apply_mode == "AUTO"
                && row.preview_status == "READY"
                && !row.blocked_by_earlier_session
        }) else {
            return Ok(());
        };
        context
            .revalidate()
            .map_err(|_| OwnerBetaPaperError::Runtime)?;
        let value: Value =
            sqlx::query_scalar("SELECT apply_owner_beta_paper_preview_auto_v1($1,$2,$3)")
                .bind(preview.preview_id)
                .bind(context.runtime().control_epoch())
                .bind(context.runtime().config_sha256())
                .fetch_one(&self.pool)
                .await
                .map_err(|_| OwnerBetaPaperError::Sql)?;
        let applied = decode_action(value, "AUTO_APPLY_PREVIEW")?;
        match applied.outcome.as_str() {
            "OK" | "REPLAY" | "WAITING_EARLIER_SESSION" => Ok(()),
            "STALE" => Err(OwnerBetaPaperError::Retry),
            _ => Err(OwnerBetaPaperError::Snapshot),
        }
    }

    async fn scan_targets(
        &self,
        context: &PaperActionContext<'_, '_, '_>,
    ) -> Result<TargetScan, OwnerBetaPaperError> {
        context
            .revalidate()
            .map_err(|_| OwnerBetaPaperError::Runtime)?;
        let value: Value = sqlx::query_scalar("SELECT scan_owner_beta_paper_targets_v1($1,$2)")
            .bind(1_i32)
            .bind(context.runtime().config_sha256())
            .fetch_one(&self.pool)
            .await
            .map_err(|_| OwnerBetaPaperError::Sql)?;
        let envelope = decode_action(value, "SCAN_TARGETS")?;
        if envelope.outcome != "OK"
            || envelope.control_epoch != Some(context.runtime().control_epoch())
            || envelope.account_id.map(|id| id.to_string()).as_deref()
                != Some(context.runtime().account_id())
        {
            return Err(OwnerBetaPaperError::Snapshot);
        }
        serde_json::from_value(envelope.data).map_err(|_| OwnerBetaPaperError::Snapshot)
    }

    async fn snapshot_target(
        &self,
        context: &PaperActionContext<'_, '_, '_>,
        target_id: Uuid,
    ) -> Result<TargetSnapshot, OwnerBetaPaperError> {
        context
            .revalidate()
            .map_err(|_| OwnerBetaPaperError::Runtime)?;
        let value: Value =
            sqlx::query_scalar("SELECT snapshot_owner_beta_paper_target_v1($1,$2,$3)")
                .bind(target_id)
                .bind(context.runtime().control_epoch())
                .bind(context.runtime().config_sha256())
                .fetch_one(&self.pool)
                .await
                .map_err(|_| OwnerBetaPaperError::Sql)?;
        let envelope = decode_action(value, "SNAPSHOT_TARGET")?;
        if envelope.outcome != "OK" {
            return Err(OwnerBetaPaperError::Snapshot);
        }
        serde_json::from_value(envelope.data).map_err(|_| OwnerBetaPaperError::Snapshot)
    }

    async fn record_target_wait(
        &self,
        context: &PaperActionContext<'_, '_, '_>,
        snapshot: &TargetSnapshot,
        reason: &str,
    ) -> Result<(), OwnerBetaPaperError> {
        let bytes = serde_json::to_vec(snapshot).map_err(|_| OwnerBetaPaperError::Snapshot)?;
        if bytes.len() > 262_144 {
            return Err(OwnerBetaPaperError::Snapshot);
        }
        let evidence = prefixed_sha256(&bytes);
        context
            .revalidate()
            .map_err(|_| OwnerBetaPaperError::Runtime)?;
        let value: Value =
            sqlx::query_scalar("SELECT record_owner_beta_paper_wait_v1($1,$2,$3,$4,$5)")
                .bind(snapshot.pending_target_id)
                .bind(context.runtime().control_epoch())
                .bind(reason)
                .bind(evidence)
                .bind(context.runtime().config_sha256())
                .fetch_one(&self.pool)
                .await
                .map_err(|_| OwnerBetaPaperError::Sql)?;
        if decode_action(value, "RECORD_WAIT")?.outcome != "OK" {
            return Err(OwnerBetaPaperError::Snapshot);
        }
        Ok(())
    }

    async fn settle_target(
        &self,
        context: &PaperActionContext<'_, '_, '_>,
        selector: &UntrustedPaperContractV1,
        target: &TargetSnapshot,
    ) -> Result<bool, OwnerBetaPaperError> {
        let t_date = target.signal_date;
        let e_date = target.effective_session;
        let t =
            TradingDate::parse(&t_date.to_string()).map_err(|_| OwnerBetaPaperError::Snapshot)?;
        let e =
            TradingDate::parse(&e_date.to_string()).map_err(|_| OwnerBetaPaperError::Snapshot)?;
        trace_lifecycle(LifecycleStage::SettleCaptureBegin);
        let captured = capture(
            &self.pool,
            selector,
            &PaperCalculationInputRequest::new([t_date, e_date], Some((t, e))),
        )
        .await;
        trace_lifecycle(LifecycleStage::SettleCaptureReturn);
        let captured = captured?;
        validate_capture_current(context, &captured)?;
        let (t_observation, e_observation) = validate_target_capture(target, &captured, context)?;
        let now = Utc::now();
        let recovery_required = matches!(
            validate_price_availability(
                e_observation.snapshot().as_of(),
                e,
                now,
                e_observation.availability(),
                PriceUse::SettleE,
            )
            .map_err(|_| OwnerBetaPaperError::Capture)?,
            AvailabilityDecision::RecoveryRequired
        );
        let cash_text = target.account_state_preimage["cash"]
            .as_str()
            .ok_or(OwnerBetaPaperError::Snapshot)?;
        let cash =
            Money::parse(cash_text, Currency::KRW).map_err(|_| OwnerBetaPaperError::Snapshot)?;
        let positions = parse_positions(&target.account_state_preimage["positions"])?;
        if positions
            .keys()
            .any(|instrument| instrument.to_string() != "069500.KRX")
        {
            return Err(OwnerBetaPaperError::Snapshot);
        }
        let profile =
            CostProfile::resolve("KRX_ETF_DEFAULT").map_err(|_| OwnerBetaPaperError::Snapshot)?;
        let state = LedgerState {
            positions,
            ..LedgerState::new(cash, profile.clone())
        };
        let previous = parse_targets(target.previous_target_weights.as_ref())?;
        let cohort_owner = parse_runtime_uuid(
            context.binding()["cohort"]["owner_user_id"]
                .as_str()
                .ok_or(OwnerBetaPaperError::Runtime)?,
        )?;
        let calendar_policy = ContentHash::parse(
            context.binding()["cohort"]["policy_sha256"]
                .as_str()
                .ok_or(OwnerBetaPaperError::Runtime)?,
        )
        .map_err(|_| OwnerBetaPaperError::Runtime)?;
        let proof = captured
            .calendar_range_proof()
            .ok_or(OwnerBetaPaperError::Capture)?;
        let execution = execute_beta_session_open(
            t_observation.snapshot(),
            e_observation.snapshot(),
            t_observation.input(),
            t_observation.target(),
            target.account_id,
            e,
            proof,
            cohort_owner,
            &calendar_policy,
            previous.as_deref(),
            &state,
        )
        .map_err(|_| OwnerBetaPaperError::Calculation)?;
        let calendar_proof = captured
            .calendar_range_proof()
            .ok_or(OwnerBetaPaperError::Capture)?;
        let settlement = build_settlement(
            target,
            t_observation,
            e_observation,
            calendar_proof,
            &execution,
            &state,
            &profile,
        )?;
        trace_lifecycle(LifecycleStage::SettleCalculated);
        context
            .revalidate()
            .map_err(|_| OwnerBetaPaperError::Runtime)?;
        trace_lifecycle(LifecycleStage::SettleSqlBegin);
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| OwnerBetaPaperError::Sql)?;
        set_paper_transaction_timeouts(&mut tx)
            .await
            .map_err(|_| OwnerBetaPaperError::Sql)?;
        context
            .revalidate()
            .map_err(|_| OwnerBetaPaperError::Runtime)?;
        let value: Result<Value, sqlx::Error> =
            sqlx::query_scalar("SELECT commit_owner_beta_paper_settlement_v1($1,$2,$3,$4)")
                .bind(target.pending_target_id)
                .bind(context.runtime().control_epoch())
                .bind(&settlement)
                .bind(context.runtime().config_sha256())
                .fetch_one(&mut *tx)
                .await;
        trace_lifecycle(LifecycleStage::SettleSqlReply);
        let value = value.map_err(|_| OwnerBetaPaperError::Sql)?;
        let committed = decode_action(value, "COMMIT_SETTLEMENT")?;
        if matches!(
            committed.outcome.as_str(),
            "WAITING_SESSION" | "WAITING_EARLIER_SESSION"
        ) {
            tx.rollback()
                .await
                .map_err(|_| OwnerBetaPaperError::CommitUnknown)?;
            if recovery_required && committed.outcome == "WAITING_SESSION" {
                self.record_target_wait(context, target, "RECOVERY_REQUIRED")
                    .await?;
            }
            return Ok(false);
        }
        if !matches!(committed.outcome.as_str(), "OK" | "REPLAY") {
            tx.rollback()
                .await
                .map_err(|_| OwnerBetaPaperError::CommitUnknown)?;
            return Err(OwnerBetaPaperError::Snapshot);
        }
        context
            .revalidate()
            .map_err(|_| OwnerBetaPaperError::Runtime)?;
        let commit = tx.commit().await;
        trace_lifecycle(LifecycleStage::SettleCommitReturn);
        match commit {
            Ok(()) => Ok(true),
            Err(_) => {
                let readback = self
                    .snapshot_target(context, target.pending_target_id)
                    .await?;
                validate_target_snapshot(context, &readback)?;
                if readback.settlement_sha256.as_deref()
                    == settlement["calculation_sha256"].as_str()
                {
                    Ok(true)
                } else {
                    Err(OwnerBetaPaperError::CommitUnknown)
                }
            }
        }
    }

    async fn value_target(
        &self,
        context: &PaperActionContext<'_, '_, '_>,
        selector: &UntrustedPaperContractV1,
        target: &TargetSnapshot,
    ) -> Result<bool, OwnerBetaPaperError> {
        let e = TradingDate::parse(&target.effective_session.to_string())
            .map_err(|_| OwnerBetaPaperError::Snapshot)?;
        trace_lifecycle(LifecycleStage::ValueCaptureBegin);
        let captured = capture(
            &self.pool,
            selector,
            &PaperCalculationInputRequest::new([target.effective_session], None),
        )
        .await;
        trace_lifecycle(LifecycleStage::ValueCaptureReturn);
        let captured = captured?;
        validate_capture_current(context, &captured)?;
        let observation = captured
            .sessions()
            .get(&target.effective_session)
            .ok_or(OwnerBetaPaperError::Capture)?;
        let admission = observation.input().pins().admission_sha256().as_str();
        if !is_prefixed_hash(admission) {
            return Err(OwnerBetaPaperError::Capture);
        }
        let _availability = validate_price_availability(
            observation.snapshot().as_of(),
            e,
            Utc::now(),
            observation.availability(),
            PriceUse::ValueE,
        )
        .map_err(|_| OwnerBetaPaperError::Capture)?;
        let after_hash = target
            .ledger_after_sha256
            .as_deref()
            .ok_or(OwnerBetaPaperError::Snapshot)?;
        let after_version = target
            .ledger_after_version
            .ok_or(OwnerBetaPaperError::Snapshot)?;
        let current_cash = target.account_state_preimage["cash"]
            .as_str()
            .ok_or(OwnerBetaPaperError::Snapshot)?;
        let current_positions = &target.account_state_preimage["positions"];
        let account_hash = account_state_sha256(
            target.account_state_version,
            current_cash,
            current_positions,
        )
        .map_err(|_| OwnerBetaPaperError::Snapshot)?;
        if target.account_state_version != after_version
            || account_hash != target.account_state_sha256
            || format!("sha256:{account_hash}") != after_hash
        {
            return Err(OwnerBetaPaperError::Snapshot);
        }
        let cash =
            Money::parse(current_cash, Currency::KRW).map_err(|_| OwnerBetaPaperError::Snapshot)?;
        let positions = parse_positions(current_positions)?;
        if positions
            .keys()
            .any(|instrument| instrument.to_string() != "069500.KRX")
        {
            return Err(OwnerBetaPaperError::Snapshot);
        }
        let profile =
            CostProfile::resolve("KRX_ETF_DEFAULT").map_err(|_| OwnerBetaPaperError::Snapshot)?;
        let state = LedgerState {
            positions,
            ..LedgerState::new(cash, profile)
        };
        let valuation = value_beta_close(observation.snapshot(), e, &state)
            .map_err(|_| OwnerBetaPaperError::Calculation)?;
        let mut closes = Vec::with_capacity(state.positions.len());
        for instrument in state.positions.keys() {
            let price = valuation
                .raw_close_prices
                .get(instrument)
                .ok_or(OwnerBetaPaperError::Calculation)?;
            closes.push(json!({
                "instrument_id": instrument.to_string(),
                "raw_close": price.as_decimal_string(),
            }));
        }
        let observed = observed_input(observation);
        let input_digest = prefixed_sha256(
            &serde_json::to_vec(&json!({
                "schema": "owner-beta-paper-valuation-input-v1",
                "target_id": target.pending_target_id,
                "snapshot": target,
                "readback": observation.readback_value(),
                "admission_sha256": admission,
                "price_pins": {
                    "content_sha256": observation.input().pins().content_sha256().as_str(),
                    "manifest_sha256": observation.input().pins().manifest_sha256().as_str(),
                    "policy_sha256": observation.input().pins().policy_sha256().as_str(),
                },
                "observed_input": observed,
            }))
            .map_err(|_| OwnerBetaPaperError::Snapshot)?,
        );
        let mut value = json!({
            "schema": "owner-beta-paper-valuation-v1",
            "target_id": target.pending_target_id,
            "control_epoch": target.control_epoch,
            "valuation_admission_sha256": admission,
            "valuation_input_sha256": input_digest,
            "settlement_sha256": target.settlement_sha256,
            "ledger_after_sha256": after_hash,
            "account_state_version": target.account_state_version,
            "account_state_sha256": target.account_state_sha256,
            "closes": closes,
            "equity": valuation.equity.as_decimal_string(),
            "cash": valuation.cash.as_decimal_string(),
            "positions_value": valuation.positions_value.as_decimal_string(),
            "currency": "KRW",
            "observed_input": observed,
        });
        let calculation_hash = prefixed_sha256(
            &serde_json::to_vec(&value).map_err(|_| OwnerBetaPaperError::Snapshot)?,
        );
        value["calculation_sha256"] = Value::String(calculation_hash.clone());
        let bytes = serde_json::to_vec(&value).map_err(|_| OwnerBetaPaperError::Snapshot)?;
        if bytes.len() > 262_144 {
            return Err(OwnerBetaPaperError::Snapshot);
        }
        trace_lifecycle(LifecycleStage::ValueCalculated);
        context
            .revalidate()
            .map_err(|_| OwnerBetaPaperError::Runtime)?;
        trace_lifecycle(LifecycleStage::ValueSqlBegin);
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| OwnerBetaPaperError::Sql)?;
        set_paper_transaction_timeouts(&mut tx)
            .await
            .map_err(|_| OwnerBetaPaperError::Sql)?;
        context
            .revalidate()
            .map_err(|_| OwnerBetaPaperError::Runtime)?;
        let reply: Result<Value, sqlx::Error> =
            sqlx::query_scalar("SELECT commit_owner_beta_paper_valuation_v1($1,$2,$3,$4)")
                .bind(target.pending_target_id)
                .bind(context.runtime().control_epoch())
                .bind(value)
                .bind(context.runtime().config_sha256())
                .fetch_one(&mut *tx)
                .await;
        trace_lifecycle(LifecycleStage::ValueSqlReply);
        let reply = reply.map_err(|_| OwnerBetaPaperError::Sql)?;
        let committed = decode_action(reply, "COMMIT_VALUATION")?;
        if committed.outcome == "WAITING_SESSION" {
            tx.rollback()
                .await
                .map_err(|_| OwnerBetaPaperError::CommitUnknown)?;
            return Ok(false);
        }
        if !matches!(committed.outcome.as_str(), "OK" | "REPLAY") {
            tx.rollback()
                .await
                .map_err(|_| OwnerBetaPaperError::CommitUnknown)?;
            return Err(OwnerBetaPaperError::Snapshot);
        }
        context
            .revalidate()
            .map_err(|_| OwnerBetaPaperError::Runtime)?;
        let commit = tx.commit().await;
        trace_lifecycle(LifecycleStage::ValueCommitReturn);
        match commit {
            Ok(()) => Ok(true),
            Err(_) => {
                let readback = self
                    .snapshot_target(context, target.pending_target_id)
                    .await?;
                validate_target_snapshot(context, &readback)?;
                if readback.valuation_sha256.as_deref() == Some(calculation_hash.as_str()) {
                    Ok(true)
                } else {
                    Err(OwnerBetaPaperError::CommitUnknown)
                }
            }
        }
    }
}

struct ClaimHeartbeat {
    stop: watch::Sender<bool>,
    lost: watch::Receiver<bool>,
    task: Option<JoinHandle<()>>,
}

impl ClaimHeartbeat {
    fn start(queue: JobQueue, claim: ClaimedJob) -> Self {
        let (stop, mut stopped) = watch::channel(false);
        let (lost_tx, lost) = watch::channel(false);
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(HEARTBEAT_PERIOD) => {
                        match queue.heartbeat(&claim).await {
                            Ok(HeartbeatStatus::Extended) => {},
                            _ => { let _ = lost_tx.send(true); return; },
                        }
                    }
                    changed = stopped.changed() => {
                        if changed.is_err() || *stopped.borrow() { return; }
                    }
                }
            }
        });
        Self {
            stop,
            lost,
            task: Some(task),
        }
    }

    async fn stop(&mut self) {
        let _ = self.stop.send(true);
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

fn ensure_claim_live(lost: &watch::Receiver<bool>) -> Result<(), OwnerBetaPaperError> {
    if *lost.borrow() {
        Err(OwnerBetaPaperError::LeaseLost)
    } else {
        Ok(())
    }
}

impl Drop for ClaimHeartbeat {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ActionEnvelope {
    schema: String,
    action: String,
    outcome: String,
    control_epoch: Option<i64>,
    account_id: Option<Uuid>,
    data: Value,
}

fn decode_action(value: Value, expected: &str) -> Result<ActionEnvelope, OwnerBetaPaperError> {
    let envelope: ActionEnvelope =
        serde_json::from_value(value).map_err(|_| OwnerBetaPaperError::Snapshot)?;
    if envelope.schema != "owner-beta-paper-action-v1"
        || envelope.action != expected
        || envelope.control_epoch.is_some_and(|value| value < 0)
        || envelope
            .account_id
            .is_some_and(|account_id| account_id.is_nil())
    {
        return Err(OwnerBetaPaperError::Snapshot);
    }
    Ok(envelope)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewSnapshot {
    preview_id: Uuid,
    job_id: Uuid,
    lineage_id: Uuid,
    apply_mode: String,
    owner_user_id: Uuid,
    account_id: Uuid,
    strategy_config_id: Uuid,
    recommendation_run_id: Uuid,
    signal_date: String,
    effective_session: String,
    daily_pins: DailyPins,
    signal_admission_sha256: String,
    factor_snapshot_sha256: String,
    target_snapshot_sha256: String,
    target_weights: Value,
    previous_target_weights: Option<Value>,
    config_epoch: i64,
    daily_activation_epoch: i64,
    cohort_sha256: String,
    current_cohort: Value,
    consent_event_id: Uuid,
    control_epoch: i64,
    calendar_evidence_sha256: String,
    calendar_link: Value,
    account_state_version: i64,
    account_state_sha256: String,
    account_state_preimage: Value,
    cost_profile_id: String,
    cost_profile_version: i32,
    attempt_id: Uuid,
    attempt_no: i32,
    worker_id: String,
    signal_allocation_sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DailyPins {
    admission_sha256: String,
    content_sha256: String,
    manifest_sha256: String,
    policy_sha256: String,
}

#[derive(Serialize)]
struct PreviewTokenInput<'a> {
    schema: &'static str,
    lineage_id: Uuid,
    owner_user_id: Uuid,
    account_id: Uuid,
    strategy_config_id: Uuid,
    recommendation_run_id: Uuid,
    signal_date: &'a str,
    effective_session: &'a str,
    daily_pins: &'a DailyPins,
    signal_admission_sha256: &'a str,
    factor_snapshot_sha256: &'a str,
    target_snapshot_sha256: &'a str,
    signal_allocation_sha256: &'a str,
    config_epoch: i64,
    daily_activation_epoch: i64,
    cohort_sha256: &'a str,
    consent_event_id: Uuid,
    control_epoch: i64,
    account_state_version: i64,
    account_state_sha256: &'a str,
    account_state_preimage: &'a Value,
    result: &'a OwnerBetaPaperCalculationResultDto,
}

fn preview_token(
    snapshot: &PreviewSnapshot,
    result: &OwnerBetaPaperCalculationResultDto,
) -> Result<String, OwnerBetaPaperError> {
    let input = PreviewTokenInput {
        schema: PREVIEW_SCHEMA,
        lineage_id: snapshot.lineage_id,
        owner_user_id: snapshot.owner_user_id,
        account_id: snapshot.account_id,
        strategy_config_id: snapshot.strategy_config_id,
        recommendation_run_id: snapshot.recommendation_run_id,
        signal_date: &snapshot.signal_date,
        effective_session: &snapshot.effective_session,
        daily_pins: &snapshot.daily_pins,
        signal_admission_sha256: &snapshot.signal_admission_sha256,
        factor_snapshot_sha256: &snapshot.factor_snapshot_sha256,
        target_snapshot_sha256: &snapshot.target_snapshot_sha256,
        signal_allocation_sha256: &snapshot.signal_allocation_sha256,
        config_epoch: snapshot.config_epoch,
        daily_activation_epoch: snapshot.daily_activation_epoch,
        cohort_sha256: &snapshot.cohort_sha256,
        consent_event_id: snapshot.consent_event_id,
        control_epoch: snapshot.control_epoch,
        account_state_version: snapshot.account_state_version,
        account_state_sha256: &snapshot.account_state_sha256,
        account_state_preimage: &snapshot.account_state_preimage,
        result,
    };
    let bytes = serde_json::to_vec(&input).map_err(|_| OwnerBetaPaperError::Snapshot)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

async fn capture(
    pool: &PgPool,
    selector: &UntrustedPaperContractV1,
    request: &PaperCalculationInputRequest,
) -> Result<ObservedPaperCalculationInputs, OwnerBetaPaperError> {
    let captured = read_paper_calculation_inputs(pool, selector, request)
        .await
        .map_err(|_| OwnerBetaPaperError::Capture)?;
    if captured.report().decision() != PaperEligibilityDecision::Eligible
        || captured.report().expires_at() <= Utc::now()
    {
        return Err(OwnerBetaPaperError::Capture);
    }
    Ok(captured)
}

fn selector_from_current(
    context: &PaperActionContext<'_, '_, '_>,
) -> Result<UntrustedPaperContractV1, OwnerBetaPaperError> {
    let bytes = serde_json::to_vec(context.binding()).map_err(|_| OwnerBetaPaperError::Runtime)?;
    UntrustedPaperContractV1::decode(&bytes).map_err(|_| OwnerBetaPaperError::Runtime)
}

fn validate_capture_current(
    context: &PaperActionContext<'_, '_, '_>,
    captured: &ObservedPaperCalculationInputs,
) -> Result<(), OwnerBetaPaperError> {
    if captured.current_binding() != context.binding() {
        return Err(OwnerBetaPaperError::Capture);
    }
    context
        .runtime()
        .validate_current_binding(captured.current_binding())
        .map_err(|_| OwnerBetaPaperError::Runtime)?;
    if captured.report().expires_at() <= Utc::now() {
        return Err(OwnerBetaPaperError::Capture);
    }
    Ok(())
}

fn latest_session(
    captured: &ObservedPaperCalculationInputs,
) -> Result<&PaperSessionObservation, OwnerBetaPaperError> {
    let date = captured
        .report()
        .sessions()
        .last()
        .ok_or(OwnerBetaPaperError::Capture)?;
    let date =
        NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| OwnerBetaPaperError::Capture)?;
    captured
        .sessions()
        .get(&date)
        .ok_or(OwnerBetaPaperError::Capture)
}

fn validate_snapshot_identity(
    context: &PaperActionContext<'_, '_, '_>,
    claim: &ClaimedJob,
    snapshot: &PreviewSnapshot,
) -> Result<(), OwnerBetaPaperError> {
    let runtime = context.runtime();
    if snapshot.preview_id.is_nil()
        || snapshot.job_id != claim.job.id
        || snapshot.owner_user_id.to_string() != runtime.owner_user_id()
        || snapshot.account_id.to_string() != runtime.account_id()
        || snapshot.strategy_config_id.to_string() != runtime.strategy_config_id()
        || snapshot.control_epoch != runtime.control_epoch()
        || snapshot.attempt_id != claim.attempt.id
        || snapshot.attempt_no != claim.attempt.attempt_no
        || snapshot.worker_id != claim.worker_id
        || !matches!(snapshot.apply_mode.as_str(), "MANUAL" | "AUTO")
        || snapshot.account_state_version < 0
        || snapshot.config_epoch < 0
        || snapshot.daily_activation_epoch < 0
        || snapshot.cost_profile_version <= 0
        || snapshot.current_cohort != context.binding()["cohort"]
        || !is_prefixed_hash(&snapshot.cohort_sha256)
        || !is_prefixed_hash(&snapshot.target_snapshot_sha256)
        || !is_prefixed_hash(&snapshot.factor_snapshot_sha256)
        || !is_prefixed_hash(&snapshot.signal_admission_sha256)
        || !is_prefixed_hash(&snapshot.calendar_evidence_sha256)
        || !snapshot.calendar_link.is_object()
        || !is_bare_hash(&snapshot.account_state_sha256)
        || snapshot.daily_pins.admission_sha256 != snapshot.signal_admission_sha256
    {
        return Err(OwnerBetaPaperError::Snapshot);
    }
    Ok(())
}

fn validate_snapshot_capture(
    snapshot: &PreviewSnapshot,
    observation: &PaperSessionObservation,
    captured: &ObservedPaperCalculationInputs,
    context: &PaperActionContext<'_, '_, '_>,
) -> Result<(), OwnerBetaPaperError> {
    let input = observation.input();
    let target = observation.target();
    if input.run_id() != snapshot.recommendation_run_id
        || input.strategy_config_id() != snapshot.strategy_config_id
        || input.as_of().to_string() != snapshot.signal_date
        || input.config_epoch() != snapshot.config_epoch
        || input.activation_epoch() != snapshot.daily_activation_epoch
        || input.pins().admission_sha256().as_str() != snapshot.daily_pins.admission_sha256
        || input.pins().content_sha256().as_str() != snapshot.daily_pins.content_sha256
        || input.pins().manifest_sha256().as_str() != snapshot.daily_pins.manifest_sha256
        || input.pins().policy_sha256().as_str() != snapshot.daily_pins.policy_sha256
        || target.as_of().to_string() != snapshot.signal_date
        || target.factor_snapshot_sha256().as_str() != snapshot.factor_snapshot_sha256
        || target.target_snapshot_sha256().as_str() != snapshot.target_snapshot_sha256
    {
        return Err(OwnerBetaPaperError::Capture);
    }
    if validate_price_availability(
        observation.snapshot().as_of(),
        TradingDate::parse(&snapshot.effective_session)
            .map_err(|_| OwnerBetaPaperError::Snapshot)?,
        Utc::now(),
        observation.availability(),
        PriceUse::SignalT,
    )
    .is_err()
    {
        return Err(OwnerBetaPaperError::Capture);
    }
    let start =
        TradingDate::parse(&snapshot.signal_date).map_err(|_| OwnerBetaPaperError::Snapshot)?;
    let end = TradingDate::parse(&snapshot.effective_session)
        .map_err(|_| OwnerBetaPaperError::Snapshot)?;
    let proof = captured
        .calendar_range_proof()
        .ok_or(OwnerBetaPaperError::Capture)?;
    if proof.owner().to_string() != context.runtime().owner_user_id()
        || !proof.days().last().is_some_and(|day| {
            day.witnesses()
                .iter()
                .any(|witness| witness.as_str() == snapshot.calendar_evidence_sha256)
        })
    {
        return Err(OwnerBetaPaperError::Capture);
    }
    let day_count = usize::try_from((end.as_naive_date() - start.as_naive_date()).num_days() + 1)
        .map_err(|_| OwnerBetaPaperError::Capture)?;
    if proof.days().len() != day_count
        || proof.days().first().map(|d| d.date()) != Some(start)
        || proof.days().last().map(|d| d.date()) != Some(end)
        || proof.days().first().map(|d| d.session_type())
            != Some(market_data::daily_price_only::DailySessionTypeV1::Trading)
        || proof.days().last().map(|d| d.session_type())
            != Some(market_data::daily_price_only::DailySessionTypeV1::Trading)
        || proof
            .days()
            .iter()
            .skip(1)
            .take(day_count.saturating_sub(2))
            .any(|d| d.session_type() == market_data::daily_price_only::DailySessionTypeV1::Trading)
    {
        return Err(OwnerBetaPaperError::Capture);
    }
    let link_days = snapshot.calendar_link["covered_days"]
        .as_array()
        .ok_or(OwnerBetaPaperError::Snapshot)?;
    if snapshot.calendar_link["schema"] != "owner-beta-paper-calendar-link-v1"
        || snapshot.calendar_link["after"].as_str() != Some(&snapshot.signal_date)
        || snapshot.calendar_link["effective_session"].as_str() != Some(&snapshot.effective_session)
        || snapshot.calendar_link["calendar_evidence_sha256"].as_str()
            != Some(snapshot.calendar_evidence_sha256.as_str())
        || link_days.len() != proof.days().len().saturating_sub(1)
        || link_days
            .iter()
            .zip(proof.days().iter().skip(1))
            .any(|(row, day)| {
                row["date"].as_str() != Some(day.date().to_string().as_str())
                    || row["kind"].as_str()
                        != Some(match day.session_type() {
                            market_data::daily_price_only::DailySessionTypeV1::Trading => "TRADING",
                            market_data::daily_price_only::DailySessionTypeV1::Closed => "CLOSED",
                        })
                    || !calendar_row_witness_matches(row, day)
            })
    {
        return Err(OwnerBetaPaperError::Capture);
    }
    let captured_targets = target_allocations(observation)?;
    let expected_targets: Vec<TargetWeightRow> = captured_targets
        .iter()
        .map(|target| TargetWeightRow {
            instrument_id: target.instrument_id.to_string(),
            weight: target.weight.as_decimal_string(),
        })
        .collect();
    let stored_targets: Vec<TargetWeightRow> =
        serde_json::from_value(snapshot.target_weights.clone())
            .map_err(|_| OwnerBetaPaperError::Snapshot)?;
    if serde_json::to_value(expected_targets).ok() != serde_json::to_value(stored_targets).ok() {
        return Err(OwnerBetaPaperError::Capture);
    }
    Ok(())
}

fn present_beta_result(
    calculated: &job_queue::owner_beta::paper::calculation::BetaPreviewResultV1,
    observation: &PaperSessionObservation,
    state: &LedgerState,
    profile: &CostProfile,
    target_snapshot_sha256: &str,
    signal_allocation_sha256: &str,
) -> Result<OwnerBetaPaperCalculationResultDto, OwnerBetaPaperError> {
    let presentation = match calculated.report() {
        Some(report) => present_sizing_report(
            report,
            state.cash,
            &state.positions,
            calculated.raw_close_prices(),
            &target_allocations(observation)?,
            profile,
        )
        .map_err(|_| OwnerBetaPaperError::Calculation)?,
        None => no_trade_presentation(calculated, observation, state)?,
    };
    let price_observations = calculated
        .raw_close_prices()
        .keys()
        .map(|instrument| OwnerBetaPaperObservedPriceDto {
            instrument_id: instrument.to_string(),
            session_date: observation.snapshot().as_of().as_naive_date(),
            observed_at: observation.observed_at(),
        })
        .collect();
    Ok(OwnerBetaPaperCalculationResultDto {
        schema: RESULT_SCHEMA.to_owned(),
        target_snapshot_sha256: target_snapshot_sha256.to_owned(),
        signal_allocation_sha256: signal_allocation_sha256.to_owned(),
        plan_status: match calculated.status() {
            BetaPlanStatus::Rebalance => OwnerBetaPaperPlanStatus::Rebalance,
            BetaPlanStatus::NoTrade => OwnerBetaPaperPlanStatus::NoTrade,
        },
        no_trade_reasons: calculated
            .no_trade_reasons()
            .iter()
            .map(no_trade_code)
            .map(str::to_owned)
            .collect(),
        equity: presentation.equity.as_decimal_string(),
        cash_before: state.cash.as_decimal_string(),
        available_cash: presentation.available_cash.as_decimal_string(),
        leftover_cash: presentation.leftover_cash.as_decimal_string(),
        buy_notional: presentation.buy_notional.as_decimal_string(),
        sell_notional: presentation.sell_notional.as_decimal_string(),
        explicit_fees: presentation.explicit_fees.as_decimal_string(),
        informational_slippage: presentation.informational_slippage.as_decimal_string(),
        decisions: presentation.decisions,
        orders: presentation.orders,
        cost_profile_id: profile.id_str().to_owned(),
        cost_profile_version: i32::try_from(profile.version)
            .map_err(|_| OwnerBetaPaperError::Calculation)?,
        price_observations,
    })
}

fn no_trade_presentation(
    calculated: &job_queue::owner_beta::paper::calculation::BetaPreviewResultV1,
    observation: &PaperSessionObservation,
    state: &LedgerState,
) -> Result<PreviewFinancialPresentationV1, OwnerBetaPaperError> {
    let valuation = value_beta_close(
        observation.snapshot(),
        observation.snapshot().as_of(),
        state,
    )
    .map_err(|_| OwnerBetaPaperError::Calculation)?;
    let zero = Money::zero(state.base_currency);
    if calculated.status() != BetaPlanStatus::NoTrade {
        return Err(OwnerBetaPaperError::Calculation);
    }
    Ok(PreviewFinancialPresentationV1 {
        equity: valuation.equity,
        available_cash: state.cash,
        leftover_cash: state.cash,
        buy_notional: zero,
        sell_notional: zero,
        explicit_fees: zero,
        informational_slippage: zero,
        decisions: Vec::<PreviewDecisionV1>::new(),
        orders: Vec::<PreviewOrderV1>::new(),
    })
}

fn target_allocations(
    observation: &PaperSessionObservation,
) -> Result<Vec<TargetAllocation>, OwnerBetaPaperError> {
    let mut targets = Vec::new();
    for item in observation.target().items() {
        if let Some(weight) = item.target_weight() {
            let parsed = Weight::parse(&weight).map_err(|_| OwnerBetaPaperError::Capture)?;
            if item.instrument_id() == "069500.KRX" {
                if !parsed.is_zero() {
                    targets.push(TargetAllocation {
                        instrument_id: InstrumentId::parse(item.instrument_id())
                            .map_err(|_| OwnerBetaPaperError::Capture)?,
                        weight: parsed,
                    });
                }
            } else if !parsed.is_zero() {
                return Err(OwnerBetaPaperError::Capture);
            }
        }
    }
    Ok(targets)
}

fn parse_positions(value: &Value) -> Result<BTreeMap<InstrumentId, Quantity>, OwnerBetaPaperError> {
    let rows = value.as_object().ok_or(OwnerBetaPaperError::Snapshot)?;
    let mut positions = BTreeMap::new();
    for (instrument, quantity) in rows {
        let instrument =
            InstrumentId::parse(instrument).map_err(|_| OwnerBetaPaperError::Snapshot)?;
        let quantity = quantity.as_str().ok_or(OwnerBetaPaperError::Snapshot)?;
        let (whole, fraction) = match quantity.split_once('.') {
            Some((whole, fraction)) => {
                if fraction.len() != 4
                    || !fraction.bytes().all(|byte| byte == b'0')
                    || whole.is_empty()
                {
                    return Err(OwnerBetaPaperError::Snapshot);
                }
                (whole, fraction)
            }
            None => (quantity, ""),
        };
        let _ = fraction;
        if whole.bytes().any(|byte| !byte.is_ascii_digit())
            || (whole.len() > 1 && whole.starts_with('0'))
        {
            return Err(OwnerBetaPaperError::Snapshot);
        }
        let quantity = Quantity::parse(whole).map_err(|_| OwnerBetaPaperError::Snapshot)?;
        if !quantity.is_zero() && positions.insert(instrument, quantity).is_some() {
            return Err(OwnerBetaPaperError::Snapshot);
        }
    }
    Ok(positions)
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TargetWeightRow {
    instrument_id: String,
    weight: String,
}

fn parse_targets(
    value: Option<&Value>,
) -> Result<Option<Vec<TargetAllocation>>, OwnerBetaPaperError> {
    let Some(value) = value else { return Ok(None) };
    let rows: Vec<TargetWeightRow> =
        serde_json::from_value(value.clone()).map_err(|_| OwnerBetaPaperError::Snapshot)?;
    let mut targets = Vec::with_capacity(rows.len());
    let mut last: Option<String> = None;
    for row in rows {
        if last.as_ref().is_some_and(|prev| prev >= &row.instrument_id) {
            return Err(OwnerBetaPaperError::Snapshot);
        }
        let weight = Weight::parse(&row.weight).map_err(|_| OwnerBetaPaperError::Snapshot)?;
        if weight.is_zero() {
            return Err(OwnerBetaPaperError::Snapshot);
        }
        let instrument_id =
            InstrumentId::parse(&row.instrument_id).map_err(|_| OwnerBetaPaperError::Snapshot)?;
        last = Some(row.instrument_id);
        targets.push(TargetAllocation {
            instrument_id,
            weight,
        });
    }
    Ok(Some(targets))
}

fn no_trade_code(reason: &BetaNoTradeReason) -> &'static str {
    match reason {
        BetaNoTradeReason::SignalUnchanged => "SIGNAL_UNCHANGED",
        BetaNoTradeReason::CashTarget => "CASH_TARGET",
        BetaNoTradeReason::Sizer(_) => "SIZER_NO_TRADE",
    }
}

fn parse_runtime_uuid(value: &str) -> Result<Uuid, OwnerBetaPaperError> {
    Uuid::parse_str(value)
        .ok()
        .filter(|parsed| !parsed.is_nil() && parsed.to_string() == value)
        .ok_or(OwnerBetaPaperError::Runtime)
}

fn parse_date(value: &str) -> Result<NaiveDate, OwnerBetaPaperError> {
    if value.len() != 10 {
        return Err(OwnerBetaPaperError::Snapshot);
    }
    NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| OwnerBetaPaperError::Snapshot)
}

fn is_bare_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_prefixed_hash(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(is_bare_hash)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetScan {
    today: NaiveDate,
    targets: Vec<TargetScanRow>,
    auto_previews: Vec<AutoPreviewRow>,
    state: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetScanRow {
    pending_target_id: Uuid,
    effective_session: NaiveDate,
    state: String,
    processing_origin: String,
    reason_code: Option<String>,
    lineage_id: Uuid,
    signal_date: NaiveDate,
    signal_admission_sha256: String,
    target_snapshot_sha256: String,
    execution_admission_sha256: Option<String>,
    settlement_sha256: Option<String>,
    valuation_sha256: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AutoPreviewRow {
    preview_id: Uuid,
    job_id: Uuid,
    lineage_id: Uuid,
    apply_mode: String,
    preview_status: String,
    signal_date: NaiveDate,
    effective_session: NaiveDate,
    signal_allocation_sha256: String,
    target_snapshot_sha256: String,
    blocked_by_earlier_session: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TargetSnapshot {
    pending_target_id: Uuid,
    lineage_id: Uuid,
    recommendation_run_id: Uuid,
    owner_user_id: Uuid,
    account_id: Uuid,
    strategy_config_id: Uuid,
    config_epoch: i64,
    daily_activation_epoch: i64,
    control_epoch: i64,
    daily_pins: Value,
    current_cohort: Value,
    cohort_sha256: String,
    consent_event_id: Uuid,
    activation_event_id: Uuid,
    process_binding_sha256: String,
    account_state_version: i64,
    account_state_sha256: String,
    initial_account_state_sha256: String,
    account_state_preimage: Value,
    target_weights: Value,
    previous_target_weights: Option<Value>,
    signal_date: NaiveDate,
    effective_session: NaiveDate,
    signal_admission_sha256: String,
    execution_admission_sha256: Option<String>,
    factor_snapshot_sha256: String,
    target_snapshot_sha256: String,
    calendar_evidence_sha256: String,
    calendar_link: Value,
    target_status: String,
    session_state: String,
    settlement_sha256: Option<String>,
    ledger_after_sha256: Option<String>,
    ledger_after_version: Option<i64>,
    valuation_sha256: Option<String>,
    receipt_json: Value,
}

fn validate_target_scan(scan: &TargetScan) -> Result<(), OwnerBetaPaperError> {
    if !matches!(
        scan.state.as_str(),
        "ENABLED" | "DRAINING" | "OFF" | "RESTORED_OFF" | "BLOCKED"
    ) {
        return Err(OwnerBetaPaperError::Snapshot);
    }
    for target in &scan.targets {
        if target.pending_target_id.is_nil()
            || target.lineage_id.is_nil()
            || target.signal_date >= target.effective_session
            || !is_prefixed_hash(&target.signal_admission_sha256)
            || !is_prefixed_hash(&target.target_snapshot_sha256)
            || target
                .execution_admission_sha256
                .as_deref()
                .is_some_and(|hash| !is_prefixed_hash(hash))
            || target
                .settlement_sha256
                .as_deref()
                .is_some_and(|hash| !is_prefixed_hash(hash))
            || target
                .valuation_sha256
                .as_deref()
                .is_some_and(|hash| !is_prefixed_hash(hash))
            || !matches!(
                target.processing_origin.as_str(),
                "AUTO" | "MANUAL" | "RECOVERY"
            )
            || !matches!(
                target.state.as_str(),
                "PENDING"
                    | "WAITING_PRICE"
                    | "WAITING_ADMISSION"
                    | "HELD"
                    | "BLOCKED"
                    | "RECOVERY_REQUIRED"
                    | "SETTLED_TRADE"
                    | "SETTLED_NO_TRADE"
            )
            || target.reason_code.as_deref().is_some_and(|code| {
                code.is_empty()
                    || code.len() > 64
                    || !code.bytes().all(|byte| {
                        byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'
                    })
            })
        {
            return Err(OwnerBetaPaperError::Snapshot);
        }
    }
    for preview in &scan.auto_previews {
        if preview.preview_id.is_nil()
            || preview.job_id.is_nil()
            || preview.lineage_id.is_nil()
            || preview.apply_mode != "AUTO"
            || preview.preview_status != "READY"
            || preview.signal_date >= preview.effective_session
            || !is_prefixed_hash(&preview.signal_allocation_sha256)
            || !is_prefixed_hash(&preview.target_snapshot_sha256)
        {
            return Err(OwnerBetaPaperError::Snapshot);
        }
    }
    Ok(())
}

// A pending session has no persisted E admission until settlement commits.
// The fresh snapshot may discover the now-available E input; an admission
// already recorded by settlement must remain identical.
fn execution_admission_matches_scan(persisted: Option<&str>, current: Option<&str>) -> bool {
    current.is_none_or(is_prefixed_hash)
        && persisted.is_none_or(|expected| current == Some(expected))
}

fn has_due_unvalued_target(scan: &TargetScan) -> bool {
    scan.targets
        .iter()
        .any(|target| target.effective_session <= scan.today && target.valuation_sha256.is_none())
}

fn receipt_lineage_matches(receipt_json: &Value, lineage_id: Uuid, settled: bool) -> bool {
    let lineage = if settled {
        &receipt_json["settlement"]["lineage_id"]
    } else {
        &receipt_json["lineage_id"]
    };
    lineage == &Value::String(lineage_id.to_string())
}

fn validate_target_snapshot(
    context: &PaperActionContext<'_, '_, '_>,
    snapshot: &TargetSnapshot,
) -> Result<(), OwnerBetaPaperError> {
    let runtime = context.runtime();
    let cohort = &snapshot.current_cohort;
    let preimage = &snapshot.account_state_preimage;
    let cash = preimage["cash"]
        .as_str()
        .ok_or(OwnerBetaPaperError::Snapshot)?;
    let positions = &preimage["positions"];
    let account_hash = account_state_sha256(snapshot.account_state_version, cash, positions)
        .map_err(|_| OwnerBetaPaperError::Snapshot)?;
    let daily_pins = snapshot
        .daily_pins
        .as_object()
        .ok_or(OwnerBetaPaperError::Snapshot)?;
    let pin_keys: BTreeMap<&str, bool> =
        daily_pins.keys().map(|key| (key.as_str(), true)).collect();
    let valid_pin_shape = pin_keys.keys().all(|key| {
        matches!(
            *key,
            "admission_sha256" | "content_sha256" | "manifest_sha256" | "policy_sha256"
        )
    }) && pin_keys.contains_key("admission_sha256");
    let receipt_lineage = receipt_lineage_matches(
        &snapshot.receipt_json,
        snapshot.lineage_id,
        snapshot.settlement_sha256.is_some(),
    );
    if snapshot.pending_target_id.is_nil()
        || snapshot.lineage_id.is_nil()
        || snapshot.recommendation_run_id.is_nil()
        || snapshot.owner_user_id.to_string() != runtime.owner_user_id()
        || snapshot.account_id.to_string() != runtime.account_id()
        || snapshot.strategy_config_id.to_string() != runtime.strategy_config_id()
        || snapshot.consent_event_id.to_string() != runtime.binding().consent_event_id()
        || snapshot.control_epoch != runtime.control_epoch()
        || snapshot.config_epoch < 0
        || snapshot.daily_activation_epoch < 0
        || snapshot.account_state_version < 0
        || snapshot.signal_date >= snapshot.effective_session
        || snapshot.current_cohort != context.binding()["cohort"]
        || cohort["owner_user_id"].as_str() != Some(runtime.owner_user_id())
        || cohort["strategy_config_id"].as_str() != Some(runtime.strategy_config_id())
        || cohort["config_epoch"].as_i64() != Some(snapshot.config_epoch)
        || cohort["activation_epoch"].as_i64() != Some(snapshot.daily_activation_epoch)
        || snapshot.cohort_sha256 != runtime.binding().daily_cohort_sha256()
        || !is_prefixed_hash(&snapshot.cohort_sha256)
        || !is_prefixed_hash(&snapshot.process_binding_sha256)
        || !is_prefixed_hash(&snapshot.factor_snapshot_sha256)
        || !is_prefixed_hash(&snapshot.target_snapshot_sha256)
        || !is_prefixed_hash(&snapshot.signal_admission_sha256)
        || !is_prefixed_hash(&snapshot.calendar_evidence_sha256)
        || !is_bare_hash(&snapshot.account_state_sha256)
        || !is_bare_hash(&snapshot.initial_account_state_sha256)
        || !valid_pin_shape
        || daily_pins["admission_sha256"] != snapshot.signal_admission_sha256
        || preimage["schema_version"] != 1
        || preimage["paper_state_version"] != snapshot.account_state_version
        || account_hash != snapshot.account_state_sha256
        || !snapshot.calendar_link.is_object()
        || snapshot.receipt_json.is_null()
        || snapshot.receipt_json.as_object().is_none()
        || !receipt_lineage
    {
        return Err(OwnerBetaPaperError::Snapshot);
    }
    for key in ["content_sha256", "manifest_sha256", "policy_sha256"] {
        if let Some(value) = daily_pins.get(key)
            && !value.as_str().is_some_and(is_prefixed_hash)
        {
            return Err(OwnerBetaPaperError::Snapshot);
        }
    }
    match snapshot.settlement_sha256.as_deref() {
        None => {
            if snapshot.target_status != "PENDING"
                || !matches!(
                    snapshot.session_state.as_str(),
                    "PENDING"
                        | "WAITING_PRICE"
                        | "WAITING_ADMISSION"
                        | "HELD"
                        | "RECOVERY_REQUIRED"
                        | "BLOCKED"
                )
                || snapshot.initial_account_state_sha256 != snapshot.account_state_sha256
                || snapshot.receipt_json["account_state_preimage"] != *preimage
                || snapshot.receipt_json["target_snapshot_sha256"]
                    != snapshot.target_snapshot_sha256
            {
                return Err(OwnerBetaPaperError::Snapshot);
            }
        }
        Some(settlement_hash) => {
            let after_hash = snapshot
                .ledger_after_sha256
                .as_deref()
                .ok_or(OwnerBetaPaperError::Snapshot)?;
            let after_version = snapshot
                .ledger_after_version
                .ok_or(OwnerBetaPaperError::Snapshot)?;
            if !is_prefixed_hash(settlement_hash)
                || !is_prefixed_hash(after_hash)
                || after_version != snapshot.account_state_version
                || format!("sha256:{}", snapshot.account_state_sha256) != after_hash
                || !matches!(snapshot.target_status.as_str(), "EXECUTED" | "SKIPPED")
                || !matches!(
                    snapshot.session_state.as_str(),
                    "SETTLED_TRADE" | "SETTLED_NO_TRADE" | "VALUED"
                )
                || snapshot.receipt_json["settlement"]["calculation_sha256"] != settlement_hash
                || snapshot.receipt_json["settlement"]["account_state_sha256"]
                    != snapshot.initial_account_state_sha256
                || snapshot.receipt_json["settlement"]["account_state_version"]
                    .as_i64()
                    .is_none_or(|version| version < 0 || version > snapshot.account_state_version)
                || snapshot.receipt_json["settlement"]["ledger_before_sha256"]
                    != format!("sha256:{}", snapshot.initial_account_state_sha256)
            {
                return Err(OwnerBetaPaperError::Snapshot);
            }
            if let Some(valuation_hash) = snapshot.valuation_sha256.as_deref()
                && (!is_prefixed_hash(valuation_hash)
                    || snapshot.receipt_json["valuation"]["calculation_sha256"] != valuation_hash)
            {
                return Err(OwnerBetaPaperError::Snapshot);
            }
        }
    }
    Ok(())
}

fn validate_target_capture<'a>(
    snapshot: &TargetSnapshot,
    captured: &'a ObservedPaperCalculationInputs,
    context: &PaperActionContext<'_, '_, '_>,
) -> Result<(&'a PaperSessionObservation, &'a PaperSessionObservation), OwnerBetaPaperError> {
    let t = captured
        .sessions()
        .get(&snapshot.signal_date)
        .ok_or(OwnerBetaPaperError::Capture)?;
    let e = captured
        .sessions()
        .get(&snapshot.effective_session)
        .ok_or(OwnerBetaPaperError::Capture)?;
    let t_input = t.input();
    let t_target = t.target();
    let e_input = e.input();
    if t.snapshot().as_of().as_naive_date() != snapshot.signal_date
        || t_input.run_id() != snapshot.recommendation_run_id
        || t_input.strategy_config_id().to_string() != context.runtime().strategy_config_id()
        || t_input.as_of().as_naive_date() != snapshot.signal_date
        || t_input.config_epoch() != snapshot.config_epoch
        || t_input.activation_epoch() != snapshot.daily_activation_epoch
        || t_input.pins().admission_sha256().as_str() != snapshot.signal_admission_sha256
        || t_target.factor_snapshot_sha256().as_str() != snapshot.factor_snapshot_sha256
        || t_target.target_snapshot_sha256().as_str() != snapshot.target_snapshot_sha256
        || t_target.as_of().as_naive_date() != snapshot.signal_date
        || e.snapshot().as_of().as_naive_date() != snapshot.effective_session
        || e_input.as_of().as_naive_date() != snapshot.effective_session
        || snapshot.execution_admission_sha256.as_deref()
            != Some(e_input.pins().admission_sha256().as_str())
        || e_input.pins().policy_sha256() != t_input.pins().policy_sha256()
    {
        return Err(OwnerBetaPaperError::Capture);
    }
    let cohort_policy = context.binding()["cohort"]["policy_sha256"]
        .as_str()
        .ok_or(OwnerBetaPaperError::Runtime)?;
    if t_input.pins().policy_sha256().as_str() != cohort_policy {
        return Err(OwnerBetaPaperError::Capture);
    }
    for (name, expected) in [
        (
            "admission_sha256",
            t_input.pins().admission_sha256().as_str(),
        ),
        ("content_sha256", t_input.pins().content_sha256().as_str()),
        ("manifest_sha256", t_input.pins().manifest_sha256().as_str()),
        ("policy_sha256", t_input.pins().policy_sha256().as_str()),
    ] {
        if let Some(stored) = snapshot.daily_pins.get(name).and_then(Value::as_str)
            && stored != expected
        {
            return Err(OwnerBetaPaperError::Capture);
        }
    }
    validate_calendar_link(snapshot, captured, context)?;
    let computed: Vec<TargetWeightRow> = target_allocations(t)
        .map_err(|_| OwnerBetaPaperError::Capture)?
        .iter()
        .map(|item| TargetWeightRow {
            instrument_id: item.instrument_id.to_string(),
            weight: item.weight.as_decimal_string(),
        })
        .collect();
    let persisted: Vec<TargetWeightRow> = serde_json::from_value(snapshot.target_weights.clone())
        .map_err(|_| OwnerBetaPaperError::Snapshot)?;
    if serde_json::to_value(computed).ok() != serde_json::to_value(persisted).ok() {
        return Err(OwnerBetaPaperError::Capture);
    }
    Ok((t, e))
}

fn calendar_row_witness_matches(
    row: &Value,
    day: &market_data::daily_price_only::calendar::ProvenCalendarDay,
) -> bool {
    match (
        row["calendar_evidence_sha256"].as_str(),
        row["row_ordinal"]
            .as_u64()
            .and_then(|ordinal| u32::try_from(ordinal).ok()),
        row["row_sha256"].as_str(),
    ) {
        (Some(evidence), Some(ordinal), Some(hash)) => day.has_row_witness(evidence, ordinal, hash),
        _ => false,
    }
}

fn validate_calendar_link(
    snapshot: &TargetSnapshot,
    captured: &ObservedPaperCalculationInputs,
    context: &PaperActionContext<'_, '_, '_>,
) -> Result<(), OwnerBetaPaperError> {
    let proof = captured
        .calendar_range_proof()
        .ok_or(OwnerBetaPaperError::Capture)?;
    let start = TradingDate::parse(&snapshot.signal_date.to_string())
        .map_err(|_| OwnerBetaPaperError::Snapshot)?;
    let end = TradingDate::parse(&snapshot.effective_session.to_string())
        .map_err(|_| OwnerBetaPaperError::Snapshot)?;
    let length = usize::try_from((end.as_naive_date() - start.as_naive_date()).num_days() + 1)
        .map_err(|_| OwnerBetaPaperError::Capture)?;
    let owner = parse_runtime_uuid(
        context.binding()["cohort"]["owner_user_id"]
            .as_str()
            .ok_or(OwnerBetaPaperError::Runtime)?,
    )?;
    let policy = context.binding()["cohort"]["policy_sha256"]
        .as_str()
        .ok_or(OwnerBetaPaperError::Runtime)?;
    if proof.owner() != owner
        || proof.policy_sha256().as_str() != policy
        || proof.days().len() != length
        || proof.days().first().map(|day| day.date()) != Some(start)
        || proof.days().last().map(|day| day.date()) != Some(end)
        || proof.days().first().map(|day| day.session_type())
            != Some(market_data::daily_price_only::DailySessionTypeV1::Trading)
        || proof.days().last().map(|day| day.session_type())
            != Some(market_data::daily_price_only::DailySessionTypeV1::Trading)
        || proof
            .days()
            .iter()
            .skip(1)
            .take(length.saturating_sub(2))
            .any(|day| {
                day.session_type() == market_data::daily_price_only::DailySessionTypeV1::Trading
            })
        || !proof.days().last().is_some_and(|day| {
            day.witnesses()
                .iter()
                .any(|hash| hash.as_str() == snapshot.calendar_evidence_sha256)
        })
    {
        return Err(OwnerBetaPaperError::Capture);
    }
    let link = &snapshot.calendar_link;
    let rows = link["covered_days"]
        .as_array()
        .ok_or(OwnerBetaPaperError::Snapshot)?;
    if link["schema"] != "owner-beta-paper-calendar-link-v1"
        || link["after"].as_str() != Some(snapshot.signal_date.to_string().as_str())
        || link["effective_session"].as_str()
            != Some(snapshot.effective_session.to_string().as_str())
        || link["calendar_evidence_sha256"].as_str()
            != Some(snapshot.calendar_evidence_sha256.as_str())
        || rows.len() != proof.days().len().saturating_sub(1)
        || rows
            .iter()
            .zip(proof.days().iter().skip(1))
            .any(|(row, day)| {
                row["date"].as_str() != Some(day.date().to_string().as_str())
                    || row["kind"].as_str()
                        != Some(match day.session_type() {
                            market_data::daily_price_only::DailySessionTypeV1::Trading => "TRADING",
                            market_data::daily_price_only::DailySessionTypeV1::Closed => "CLOSED",
                        })
                    || !calendar_row_witness_matches(row, day)
            })
    {
        return Err(OwnerBetaPaperError::Capture);
    }
    Ok(())
}

fn build_settlement(
    target: &TargetSnapshot,
    t: &PaperSessionObservation,
    e: &PaperSessionObservation,
    calendar_proof: &market_data::daily_price_only::calendar::CalendarRangeProof,
    execution: &job_queue::owner_beta::paper::calculation::BetaExecutionResultV1,
    before: &LedgerState,
    profile: &CostProfile,
) -> Result<Value, OwnerBetaPaperError> {
    let effective = TradingDate::parse(&target.effective_session.to_string())
        .map_err(|_| OwnerBetaPaperError::Snapshot)?;
    let pending = PendingTarget {
        account_id: target.account_id,
        effective_date: effective,
        targets: target_allocations(t)?,
    };
    let (outcome, reason) = match execution.disposition() {
        BetaExecutionDisposition::Executed => ("TRADE", "TRADE_EXECUTED"),
        BetaExecutionDisposition::NoTrade(reasons) => (
            "NO_TRADE",
            reasons
                .first()
                .map(no_trade_code)
                .unwrap_or("SIZER_NO_TRADE"),
        ),
    };
    let mut order_rows = Vec::new();
    if outcome == "TRADE" {
        let plan = execution.plan().ok_or(OwnerBetaPaperError::Calculation)?;
        if plan.report.orders.len() != execution.state().fills.len() || plan.report.orders.len() > 1
        {
            return Err(OwnerBetaPaperError::Calculation);
        }
        for (request, fill) in plan.report.orders.iter().zip(&execution.state().fills) {
            if request.instrument_id != fill.instrument_id
                || request.side != fill.side
                || request.quantity != fill.quantity
                || fill.order_id.to_string()
                    != pending
                        .order_id_for(&fill.instrument_id, fill.side)
                        .to_string()
                || fill.fill_id.to_string()
                    != pending
                        .fill_id_for(&fill.instrument_id, fill.side)
                        .to_string()
            {
                return Err(OwnerBetaPaperError::Calculation);
            }
            let raw_open = execution
                .raw_open_prices()
                .get(&fill.instrument_id)
                .ok_or(OwnerBetaPaperError::Calculation)?;
            let costs = profile
                .estimate(fill.side, &fill.quantity, &fill.price)
                .map_err(|_| OwnerBetaPaperError::Calculation)?;
            if costs.commission != fill.commission || costs.tax != fill.tax {
                return Err(OwnerBetaPaperError::Calculation);
            }
            order_rows.push(json!({
                "order_ref": fill.order_id.to_string(),
                "fill_ref": fill.fill_id.to_string(),
                "instrument_id": fill.instrument_id.to_string(),
                "side": if fill.side == Side::Buy { "BUY" } else { "SELL" },
                "quantity": fill.quantity.as_decimal_string(),
                "raw_open": raw_open.as_decimal_string(),
                "fill_price": fill.price.as_decimal_string(),
                "commission": fill.commission.as_decimal_string(),
                "tax": fill.tax.as_decimal_string(),
                "slippage": costs.slippage.as_decimal_string(),
                "cash_before": fill.cash_before.as_decimal_string(),
                "cash_after": fill.cash_after.as_decimal_string(),
            }));
        }
    }
    let positions_after = union_positions(before, execution.state())?;
    let observation = observed_input(e);
    let calendar_rows: Vec<Value> = calendar_proof
        .days()
        .iter()
        .map(|day| {
            json!({
                "date": day.date(),
                "kind": match day.session_type() {
                    market_data::daily_price_only::DailySessionTypeV1::Trading => "TRADING",
                    market_data::daily_price_only::DailySessionTypeV1::Closed => "CLOSED",
                },
                "witnesses": day.witnesses().iter().map(|hash| hash.as_str()).collect::<Vec<_>>(),
            })
        })
        .collect();
    let input = json!({
        "schema": "owner-beta-paper-execution-input-v1",
        "target_snapshot": target,
        "t_readback": t.readback_value(),
        "e_readback": e.readback_value(),
        "t_pins": {
            "admission_sha256": t.input().pins().admission_sha256().as_str(),
            "content_sha256": t.input().pins().content_sha256().as_str(),
            "manifest_sha256": t.input().pins().manifest_sha256().as_str(),
            "policy_sha256": t.input().pins().policy_sha256().as_str(),
        },
        "e_pins": {
            "admission_sha256": e.input().pins().admission_sha256().as_str(),
            "content_sha256": e.input().pins().content_sha256().as_str(),
            "manifest_sha256": e.input().pins().manifest_sha256().as_str(),
            "policy_sha256": e.input().pins().policy_sha256().as_str(),
        },
        "t_target_snapshot_sha256": t.target().target_snapshot_sha256().as_str(),
        "calendar_days": calendar_rows,
    });
    let input_bytes = serde_json::to_vec(&input).map_err(|_| OwnerBetaPaperError::Snapshot)?;
    let mut settlement = json!({
        "schema": "owner-beta-paper-settlement-v1",
        "target_id": target.pending_target_id,
        "lineage_id": target.lineage_id,
        "control_epoch": target.control_epoch,
        "execution_admission_sha256": e.input().pins().admission_sha256().as_str(),
        "execution_input_sha256": prefixed_sha256(&input_bytes),
        "account_state_version": target.account_state_version,
        "account_state_sha256": target.initial_account_state_sha256,
        "ledger_before_sha256": format!("sha256:{}", target.initial_account_state_sha256),
        "outcome": outcome,
        "reason_code": reason,
        "observed_input": observation,
        "orders": order_rows,
        "positions_after": positions_after,
        "cash_after": execution.state().cash.as_decimal_string(),
    });
    let calculation_hash = prefixed_sha256(
        &serde_json::to_vec(&settlement).map_err(|_| OwnerBetaPaperError::Snapshot)?,
    );
    settlement["calculation_sha256"] = Value::String(calculation_hash);
    let bytes = serde_json::to_vec(&settlement).map_err(|_| OwnerBetaPaperError::Snapshot)?;
    if bytes.len() > 262_144 {
        return Err(OwnerBetaPaperError::Snapshot);
    }
    Ok(settlement)
}

fn union_positions(
    before: &LedgerState,
    after: &LedgerState,
) -> Result<Vec<Value>, OwnerBetaPaperError> {
    let mut ids = before
        .positions
        .keys()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    ids.extend(after.positions.keys().cloned());
    let mut rows = Vec::with_capacity(ids.len());
    for instrument in ids {
        if instrument.to_string() != "069500.KRX" {
            return Err(OwnerBetaPaperError::Calculation);
        }
        let quantity = match after.positions.get(&instrument) {
            Some(quantity) => quantity.as_decimal_string(),
            None if before.positions.contains_key(&instrument) => "0".to_owned(),
            None => return Err(OwnerBetaPaperError::Calculation),
        };
        rows.push(json!({ "instrument_id": instrument.to_string(), "quantity": quantity }));
    }
    Ok(rows)
}

fn observed_input(observation: &PaperSessionObservation) -> Value {
    let availability = observation.availability();
    let format =
        |value: chrono::DateTime<Utc>| value.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true);
    json!({
        "original_retrieved_at": format(availability.source_original_retrieved_at),
        "admitted_at": format(availability.input_admitted_at),
        "usable_at": format(availability.usable_at),
        "precision": "APPLICATION_OBSERVED",
    })
}

fn prefixed_sha256(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target_row(effective_session: NaiveDate, valued: bool) -> TargetScanRow {
        TargetScanRow {
            pending_target_id: Uuid::from_u128(1),
            effective_session,
            state: if valued {
                "SETTLED_NO_TRADE"
            } else {
                "PENDING"
            }
            .to_owned(),
            processing_origin: "AUTO".to_owned(),
            reason_code: None,
            lineage_id: Uuid::from_u128(2),
            signal_date: effective_session.pred_opt().expect("previous date"),
            signal_admission_sha256: format!("sha256:{}", "a".repeat(64)),
            target_snapshot_sha256: format!("sha256:{}", "b".repeat(64)),
            execution_admission_sha256: None,
            settlement_sha256: valued.then(|| format!("sha256:{}", "c".repeat(64))),
            valuation_sha256: valued.then(|| format!("sha256:{}", "d".repeat(64))),
        }
    }

    fn scan(today: NaiveDate, target: TargetScanRow) -> TargetScan {
        TargetScan {
            today,
            targets: vec![target],
            auto_previews: Vec::new(),
            state: "ENABLED".to_owned(),
        }
    }

    #[test]
    fn first_execution_admission_can_arrive_without_replacing_a_committed_pin() {
        let admitted = format!("sha256:{}", "a".repeat(64));
        let changed = format!("sha256:{}", "b".repeat(64));
        assert!(execution_admission_matches_scan(None, None));
        assert!(execution_admission_matches_scan(None, Some(&admitted)));
        assert!(execution_admission_matches_scan(
            Some(&admitted),
            Some(&admitted)
        ));
        assert!(!execution_admission_matches_scan(Some(&admitted), None));
        assert!(!execution_admission_matches_scan(
            Some(&admitted),
            Some(&changed)
        ));
        assert!(!execution_admission_matches_scan(None, Some("unproven")));
    }

    #[test]
    fn due_unvalued_target_defers_preview_but_future_target_does_not() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 4).expect("valid date");
        let due = target_row(today, false);
        let future = target_row(today.succ_opt().expect("next date"), false);
        let valued = target_row(today, true);

        assert!(has_due_unvalued_target(&scan(today, due)));
        assert!(!has_due_unvalued_target(&scan(today, future)));
        assert!(!has_due_unvalued_target(&scan(today, valued)));
    }

    #[test]
    fn settled_receipt_lineage_is_read_from_nested_settlement() {
        let lineage = Uuid::from_u128(77);
        let receipt = json!({ "schema": "owner-beta-paper-session-receipt-v1",
            "settlement": { "lineage_id": lineage } });

        assert!(receipt_lineage_matches(&receipt, lineage, true));
        assert!(!receipt_lineage_matches(
            &receipt,
            Uuid::from_u128(78),
            true
        ));
        assert!(!receipt_lineage_matches(&receipt, lineage, false));
    }

    #[test]
    fn bound_action_errors_preserve_unknown_commit_and_work_error() {
        assert_eq!(
            map_bound_action_error_parts(
                BoundPaperActionErrorKind::CommitUnknown,
                Some(OwnerBetaPaperError::Calculation),
            ),
            OwnerBetaPaperError::CommitUnknown
        );
        assert_eq!(
            map_bound_action_error_parts(
                BoundPaperActionErrorKind::CompletionUnclean,
                Some(OwnerBetaPaperError::Snapshot),
            ),
            OwnerBetaPaperError::CommitUnknown
        );
        assert_eq!(
            map_bound_action_error_parts(
                BoundPaperActionErrorKind::ActionFailed,
                Some(OwnerBetaPaperError::Calculation),
            ),
            OwnerBetaPaperError::Calculation
        );
        assert_eq!(
            map_bound_action_error_parts(BoundPaperActionErrorKind::Refused, None),
            OwnerBetaPaperError::Runtime
        );
    }
}
