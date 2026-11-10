//! Owner-bound R5 beta preview worker integration.
//!
//! This path accepts no caller supplied identity. The actual protected
//! process and Current scope are retained by `run_bound_paper_action`; SQL
//! remains the authority for consent, source lineage, lease, and account
//! fencing.

#[cfg(all(test, target_os = "linux"))]
mod action_native_tests;
#[cfg(all(test, target_os = "linux"))]
mod claim_native_tests;

use std::{collections::BTreeMap, time::Duration};

use chrono::{FixedOffset, NaiveDate, Utc};
use domain::{ContentHash, Currency, InstrumentId, Money, Quantity, TradingDate, Weight};
use job_queue::owner_beta::paper::evidence::financial::{LocalInputFuture, PaperInputActionError};
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
        contract::PaperEligibilityReasonCode,
        evidence::financial::{
            CheckedPaperAction, CheckedPaperActionOutcome, PaperActionInputRequest,
            PaperActionInputs, PaperAdmittedPriceObservation, PaperFinancialConsumer,
            PaperSignalObservation, with_checked_paper_action, with_checked_paper_recovery_action,
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
    ClaimCaptureBegin,
    ClaimCaptureReturn,
    PublishReply,
    PrecommitClaimLost,
    PrecommitContextRefused,
    PrecommitDeadline,
    CommitRuntimeRefused,
    CommitSqlRefused,
    CommitInputRefused,
    CommitStateRefused,
    CommitCleanupRefused,
    CommitUnknownRefused,
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
            Self::ClaimCaptureBegin => "BETA_CLAIM_CAPTURE_BEGIN",
            Self::ClaimCaptureReturn => "BETA_CLAIM_CAPTURE_RETURN",
            Self::PublishReply => "BETA_PUBLISH_REPLY",
            Self::PrecommitClaimLost => "BETA_PRECOMMIT_CLAIM_LOST",
            Self::PrecommitContextRefused => "BETA_PRECOMMIT_CONTEXT_REFUSED",
            Self::PrecommitDeadline => "BETA_PRECOMMIT_DEADLINE",
            Self::CommitRuntimeRefused => "BETA_COMMIT_RUNTIME_REFUSED",
            Self::CommitSqlRefused => "BETA_COMMIT_SQL_REFUSED",
            Self::CommitInputRefused => "BETA_COMMIT_INPUT_REFUSED",
            Self::CommitStateRefused => "BETA_COMMIT_STATE_REFUSED",
            Self::CommitCleanupRefused => "BETA_COMMIT_CLEANUP_REFUSED",
            Self::CommitUnknownRefused => "BETA_COMMIT_UNKNOWN_REFUSED",
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

fn map_action_input_error(error: PaperInputActionError) -> OwnerBetaPaperError {
    match error {
        PaperInputActionError::Sql => OwnerBetaPaperError::Sql,
        PaperInputActionError::Runtime => OwnerBetaPaperError::Runtime,
        PaperInputActionError::CommitUnknown => OwnerBetaPaperError::CommitUnknown,
        PaperInputActionError::Input(_)
        | PaperInputActionError::TransactionState
        | PaperInputActionError::Cleanup => OwnerBetaPaperError::Capture,
    }
}

fn finish_checked_action<T>(
    outcome: CheckedPaperActionOutcome<T, OwnerBetaPaperError>,
) -> Result<T, OwnerBetaPaperError> {
    match outcome.into_result() {
        Ok(value) => Ok(value),
        Err(failure) => {
            let commit_started = failure.commit_started() || failure.committed();
            let first = failure.first_error();
            match failure.into_original() {
                Some(Err(error)) if !commit_started => Err(error),
                _ if commit_started => Err(OwnerBetaPaperError::CommitUnknown),
                _ => Err(first
                    .map(map_action_input_error)
                    .unwrap_or(OwnerBetaPaperError::Capture)),
            }
        }
    }
}

pub(crate) async fn run_preview_once(
    pool: &PgPool,
    queue: &JobQueue,
    worker_id: &str,
    heartbeat_period: Duration,
    shutdown: Option<watch::Receiver<bool>>,
) -> Result<OwnerBetaCycleOutcome, OwnerBetaPaperError> {
    if !valid_heartbeat_period(heartbeat_period, queue.config().lease) {
        return Err(OwnerBetaPaperError::Runtime);
    }
    let work = OwnerBetaPreviewWork {
        pool: pool.clone(),
        queue: queue.clone(),
        worker_id: worker_id.to_owned(),
        heartbeat_period,
        #[cfg(all(test, target_os = "linux"))]
        native_probe: None,
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

fn valid_heartbeat_period(period: Duration, lease: Duration) -> bool {
    !period.is_zero() && period < lease
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
    heartbeat_period: Duration,
    #[cfg(all(test, target_os = "linux"))]
    native_probe: Option<std::sync::Arc<action_native_tests::Probe>>,
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
    #[cfg(all(test, target_os = "linux"))]
    async fn native_before_sql(
        &self,
        action: &mut CheckedPaperAction<'_>,
    ) -> Result<(), OwnerBetaPaperError> {
        if let Some(probe) = &self.native_probe {
            probe.before_sql(action).await?;
        }
        Ok(())
    }

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

        // Resolve the earliest due session before claiming or publishing a
        // fresh T preview. Settlement changes the account fingerprint that a
        // later preview must snapshot.
        trace_lifecycle(LifecycleStage::DueBegin);
        let due = self.run_targets_once(context).await;
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
        let initial = with_checked_paper_action(
            &self.pool,
            context,
            &PaperActionInputRequest::LatestSignal,
            QueueSignalConsumer {
                work: &self,
                context,
            },
        )
        .await;
        trace_lifecycle(LifecycleStage::LatestCaptureReturn);
        finish_checked_action(initial)?;

        context
            .revalidate()
            .map_err(|_| OwnerBetaPaperError::Runtime)?;
        let claim = self
            .queue
            .claim_next_paper_for_owner(&self.worker_id, owner)
            .await
            .map_err(|_| OwnerBetaPaperError::Sql)?;
        let preview_published = if let Some(claim) = claim {
            let heartbeat =
                ClaimHeartbeat::start(self.queue.clone(), claim.clone(), self.heartbeat_period);
            let result = heartbeat
                .run_and_join(self.process_claim(context, &claim, &heartbeat))
                .await;
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
        heartbeat: &ClaimHeartbeat,
    ) -> Result<(), OwnerBetaPaperError> {
        let claim_lost = &heartbeat.lost;
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

        let t =
            TradingDate::parse(&snapshot.signal_date).map_err(|_| OwnerBetaPaperError::Snapshot)?;
        let e = TradingDate::parse(&snapshot.effective_session)
            .map_err(|_| OwnerBetaPaperError::Snapshot)?;
        trace_lifecycle(LifecycleStage::ClaimCaptureBegin);
        let outcome = with_checked_paper_action(
            &self.pool,
            context,
            &PaperActionInputRequest::Preview {
                signal: t,
                effective: e,
                run_id: snapshot.recommendation_run_id,
            },
            PreviewConsumer {
                work: self,
                context,
                claim,
                heartbeat,
                snapshot,
            },
        )
        .await;
        trace_lifecycle(LifecycleStage::ClaimCaptureReturn);
        finish_checked_action(outcome)
    }

    async fn process_claim_inputs(
        &self,
        action: &mut CheckedPaperAction<'_>,
        context: &PaperActionContext<'_, '_, '_>,
        claim: &ClaimedJob,
        heartbeat: &ClaimHeartbeat,
        snapshot: &PreviewSnapshot,
    ) -> Result<(), OwnerBetaPaperError> {
        let claim_lost = &heartbeat.lost;
        ensure_claim_live(claim_lost)?;
        context
            .revalidate()
            .map_err(|_| OwnerBetaPaperError::Runtime)?;
        action.revalidate_files().map_err(map_action_input_error)?;
        let observation = action.inputs().signal();
        let signal_as_of = observation.snapshot().as_of();
        let signal_availability = observation.availability();
        validate_snapshot_capture(
            snapshot,
            observation,
            action.inputs(),
            context,
            #[cfg(all(test, target_os = "linux"))]
            self.native_probe.as_deref(),
        )?;
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
        let preview_token = preview_token(snapshot, &result)?;
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
        action.revalidate_files().map_err(map_action_input_error)?;
        action
            .begin_transaction()
            .await
            .map_err(map_action_input_error)?;
        #[cfg(all(test, target_os = "linux"))]
        self.native_before_sql(action).await?;
        context
            .revalidate()
            .map_err(|_| OwnerBetaPaperError::Runtime)?;
        ensure_claim_live(claim_lost)?;
        // A watch request alone cannot hand off ownership: join the actual
        // in-flight updater before acquiring the row it would update.
        heartbeat.quiesce().await?;
        let lease = self
            .queue
            .lock_claim_transaction_lease_in(
                action.transaction().map_err(map_action_input_error)?,
                claim,
            )
            .await;
        let lease = match lease {
            Ok(lease) => lease,
            Err(error) => {
                action.retain_queue_error(error);
                return Err(OwnerBetaPaperError::LeaseLost);
            }
        };
        let mut fence = PreviewCommitFence {
            queue: &self.queue,
            lease,
        };
        context
            .revalidate()
            .map_err(|_| OwnerBetaPaperError::Runtime)?;
        fence.check_action(action).await?;
        let result_value: Result<Value, sqlx::Error> =
            sqlx::query_scalar("SELECT publish_owner_beta_paper_preview_v1($1,$2,$3,$4)")
                .bind(snapshot.preview_id)
                .bind(snapshot.job_id)
                .bind(publication)
                .bind(context.runtime().config_sha256())
                .fetch_one(&mut **action.transaction().map_err(map_action_input_error)?)
                .await;
        trace_lifecycle(LifecycleStage::PublishReply);
        let result_value = match result_value {
            Ok(value) => {
                action.retain_value(value.clone());
                value
            }
            Err(error) => {
                action.retain_sql_error(error);
                return Err(OwnerBetaPaperError::Sql);
            }
        };
        let published = decode_action(result_value, "PUBLISH_PREVIEW")?;
        if published.outcome == "WAITING_EARLIER_SESSION" {
            action.rollback().await.map_err(map_action_input_error)?;
            return Err(OwnerBetaPaperError::Retry);
        }
        if published.outcome != "OK" {
            action.rollback().await.map_err(map_action_input_error)?;
            return Err(if published.outcome == "STALE" {
                OwnerBetaPaperError::Retry
            } else {
                OwnerBetaPaperError::Snapshot
            });
        }
        fence.check_action(action).await?;
        context
            .revalidate()
            .map_err(|_| OwnerBetaPaperError::Runtime)?;
        fence.check_action(action).await?;
        let mut final_check = || {
            ensure_claim_live(claim_lost).map_err(|_| {
                trace_lifecycle(LifecycleStage::PrecommitClaimLost);
                job_queue::owner_beta::paper::evidence::financial::PaperInputActionError::Runtime
            })?;
            context.revalidate().map_err(|refusal| {
                trace_lifecycle(match refusal {
                    job_queue::owner_beta::paper::action::PaperActionRefusal::DeadlineExceeded => {
                        LifecycleStage::PrecommitDeadline
                    }
                    _ => LifecycleStage::PrecommitContextRefused,
                });
                job_queue::owner_beta::paper::evidence::financial::PaperInputActionError::Runtime
            })?;
            let signal_date = signal_as_of.as_naive_date();
            let effective = TradingDate::parse(&snapshot.effective_session).map_err(|_| {
                job_queue::owner_beta::paper::evidence::financial::PaperInputActionError::Runtime
            })?;
            validate_price_availability(
                signal_as_of,
                effective,
                Utc::now(),
                signal_availability,
                PriceUse::SignalT,
            )
            .map_err(|_| {
                PaperInputActionError::Input(PaperEligibilityReasonCode::PriceNotAvailable)
            })?;
            if signal_date >= effective.as_naive_date() {
                return Err(
                    job_queue::owner_beta::paper::evidence::financial::PaperInputActionError::Input(
                        PaperEligibilityReasonCode::CalendarUnproven,
                    ),
                );
            }
            Ok(())
        };
        match action.commit_fenced(&mut final_check, &mut fence).await {
            Ok(()) => Ok(()),
            Err(_) if action.commit_started() => {
                // Read-only reconciliation stays inside the retained scope.
                let readback: Result<Option<(String, Value, Option<String>)>, sqlx::Error> =
                    sqlx::query_as(
                        "SELECT status::text,payload_json,idempotency_key FROM public.jobs \
                     WHERE id=$1 AND owner_user_id=$2 \
                       AND job_type='owner_beta_paper_preview'",
                    )
                    .bind(claim.job.id)
                    .bind(claim.job.owner_user_id)
                    .fetch_optional(&self.pool)
                    .await;
                match readback {
                    Ok(row) => {
                        action.retain_value(json!({"job_readback": row.clone()}));
                        let matches_request =
                            row.as_ref().is_some_and(|(job_status, payload, key)| {
                                job_status == "SUCCEEDED"
                                    && payload["schema"] == "owner-beta-paper-preview-job-v1"
                                    && payload["preview_id"] == snapshot.preview_id.to_string()
                                    && payload["input_kind"] == "OWNER_BETA_DAILY_V1"
                                    && payload["apply_mode"] == snapshot.apply_mode
                                    && claim
                                        .job
                                        .idempotency_key
                                        .as_deref()
                                        .is_some_and(|expected| key.as_deref() == Some(expected))
                            });
                        action.retain_value(json!({"matches_request": matches_request}));
                    }
                    Err(error) => action.retain_sql_error(error),
                }
                Err(OwnerBetaPaperError::CommitUnknown)
            }
            Err(error) => {
                trace_lifecycle(match error {
                    PaperInputActionError::Runtime => LifecycleStage::CommitRuntimeRefused,
                    PaperInputActionError::Sql => LifecycleStage::CommitSqlRefused,
                    PaperInputActionError::Input(_) => LifecycleStage::CommitInputRefused,
                    PaperInputActionError::TransactionState => LifecycleStage::CommitStateRefused,
                    PaperInputActionError::Cleanup => LifecycleStage::CommitCleanupRefused,
                    PaperInputActionError::CommitUnknown => LifecycleStage::CommitUnknownRefused,
                });
                Err(OwnerBetaPaperError::Runtime)
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
            if self.settle_target(context, &snapshot).await? {
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
        let valued = self.value_target(context, &fresh).await?;
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
        target: &TargetSnapshot,
    ) -> Result<bool, OwnerBetaPaperError> {
        let signal = TradingDate::parse(&target.signal_date.to_string())
            .map_err(|_| OwnerBetaPaperError::Snapshot)?;
        let effective = TradingDate::parse(&target.effective_session.to_string())
            .map_err(|_| OwnerBetaPaperError::Snapshot)?;
        trace_lifecycle(LifecycleStage::SettleCaptureBegin);
        let request = PaperActionInputRequest::Settle {
            signal,
            effective,
            run_id: target.recommendation_run_id,
        };
        let consumer = SettlementConsumer {
            work: self,
            context,
            target,
        };
        let outcome =
            with_checked_target_action(&self.pool, context, target, &request, consumer).await?;
        trace_lifecycle(LifecycleStage::SettleCaptureReturn);
        finish_checked_action(outcome)
    }

    async fn settle_target_with(
        &self,
        action: &mut CheckedPaperAction<'_>,
        context: &PaperActionContext<'_, '_, '_>,
        target: &TargetSnapshot,
    ) -> Result<bool, OwnerBetaPaperError> {
        action.revalidate_files().map_err(map_action_input_error)?;
        TradingDate::parse(&target.signal_date.to_string())
            .map_err(|_| OwnerBetaPaperError::Snapshot)?;
        let e = TradingDate::parse(&target.effective_session.to_string())
            .map_err(|_| OwnerBetaPaperError::Snapshot)?;
        let (t_observation, e_observation) = validate_target_capture(
            target,
            action.inputs(),
            context,
            #[cfg(all(test, target_os = "linux"))]
            self.native_probe.as_deref(),
        )?;
        let e_as_of = e_observation.snapshot().as_of();
        let e_availability = e_observation.availability();
        let now = Utc::now();
        let recovery_required = matches!(
            validate_price_availability(e_as_of, e, now, e_availability, PriceUse::SettleE,)
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
        let proof = action
            .inputs()
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
        let calendar_proof = action
            .inputs()
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
        let expected_calculation_sha256 = settlement["calculation_sha256"]
            .as_str()
            .ok_or(OwnerBetaPaperError::Snapshot)?
            .to_owned();
        trace_lifecycle(LifecycleStage::SettleCalculated);
        context
            .revalidate()
            .map_err(|_| OwnerBetaPaperError::Runtime)?;
        action.revalidate_files().map_err(map_action_input_error)?;
        trace_lifecycle(LifecycleStage::SettleSqlBegin);
        action
            .begin_transaction()
            .await
            .map_err(map_action_input_error)?;
        #[cfg(all(test, target_os = "linux"))]
        self.native_before_sql(action).await?;
        context
            .revalidate()
            .map_err(|_| OwnerBetaPaperError::Runtime)?;
        let value: Result<Value, sqlx::Error> =
            sqlx::query_scalar("SELECT commit_owner_beta_paper_settlement_v1($1,$2,$3,$4)")
                .bind(target.pending_target_id)
                .bind(context.runtime().control_epoch())
                .bind(&settlement)
                .bind(context.runtime().config_sha256())
                .fetch_one(&mut **action.transaction().map_err(map_action_input_error)?)
                .await;
        trace_lifecycle(LifecycleStage::SettleSqlReply);
        let value = match value {
            Ok(value) => {
                action.retain_value(value.clone());
                value
            }
            Err(error) => {
                action.retain_sql_error(error);
                return Err(OwnerBetaPaperError::Sql);
            }
        };
        let committed = decode_action(value, "COMMIT_SETTLEMENT")?;
        if matches!(
            committed.outcome.as_str(),
            "WAITING_SESSION" | "WAITING_EARLIER_SESSION"
        ) {
            action.rollback().await.map_err(map_action_input_error)?;
            if recovery_required && committed.outcome == "WAITING_SESSION" {
                self.record_target_wait(context, target, "RECOVERY_REQUIRED")
                    .await?;
            }
            return Ok(false);
        }
        if !matches!(committed.outcome.as_str(), "OK" | "REPLAY") {
            action.rollback().await.map_err(map_action_input_error)?;
            return Err(OwnerBetaPaperError::Snapshot);
        }
        let mut final_check = || {
            context
                .revalidate()
                .map_err(|_| PaperInputActionError::Runtime)?;
            validate_price_availability(e_as_of, e, Utc::now(), e_availability, PriceUse::SettleE)
                .map_err(|_| {
                    PaperInputActionError::Input(PaperEligibilityReasonCode::PriceNotAvailable)
                })?;
            Ok(())
        };
        let commit = action.commit(&mut final_check).await;
        trace_lifecycle(LifecycleStage::SettleCommitReturn);
        match commit {
            Ok(()) => Ok(true),
            Err(_) if action.commit_started() => {
                let readback: Result<Value, sqlx::Error> =
                    sqlx::query_scalar("SELECT snapshot_owner_beta_paper_target_v1($1,$2,$3)")
                        .bind(target.pending_target_id)
                        .bind(context.runtime().control_epoch())
                        .bind(context.runtime().config_sha256())
                        .fetch_one(&self.pool)
                        .await;
                match readback {
                    Ok(value) => {
                        action.retain_value(value.clone());
                        let matches_request = target_readback_matches(
                            &value,
                            target.pending_target_id,
                            "settlement",
                            &expected_calculation_sha256,
                        );
                        action.retain_value(json!({"matches_request": matches_request}));
                    }
                    Err(error) => action.retain_sql_error(error),
                }
                Err(OwnerBetaPaperError::CommitUnknown)
            }
            Err(error) => Err(map_action_input_error(error)),
        }
    }

    async fn value_target(
        &self,
        context: &PaperActionContext<'_, '_, '_>,
        target: &TargetSnapshot,
    ) -> Result<bool, OwnerBetaPaperError> {
        let t = TradingDate::parse(&target.signal_date.to_string())
            .map_err(|_| OwnerBetaPaperError::Snapshot)?;
        let e = TradingDate::parse(&target.effective_session.to_string())
            .map_err(|_| OwnerBetaPaperError::Snapshot)?;
        trace_lifecycle(LifecycleStage::ValueCaptureBegin);
        let request = PaperActionInputRequest::Value {
            signal: t,
            effective: e,
            run_id: target.recommendation_run_id,
        };
        let consumer = ValuationConsumer {
            work: self,
            context,
            target,
        };
        let outcome =
            with_checked_target_action(&self.pool, context, target, &request, consumer).await?;
        trace_lifecycle(LifecycleStage::ValueCaptureReturn);
        finish_checked_action(outcome)
    }

    async fn value_target_with(
        &self,
        action: &mut CheckedPaperAction<'_>,
        context: &PaperActionContext<'_, '_, '_>,
        target: &TargetSnapshot,
    ) -> Result<bool, OwnerBetaPaperError> {
        action.revalidate_files().map_err(map_action_input_error)?;
        let e = TradingDate::parse(&target.effective_session.to_string())
            .map_err(|_| OwnerBetaPaperError::Snapshot)?;
        let (_, observation) = validate_target_capture(
            target,
            action.inputs(),
            context,
            #[cfg(all(test, target_os = "linux"))]
            self.native_probe.as_deref(),
        )?;
        let e_as_of = observation.snapshot().as_of();
        let e_availability = observation.availability();
        let pins = observation.snapshot().pins();
        let admission = pins.admission_sha256.as_str();
        if !is_prefixed_hash(admission) {
            return Err(OwnerBetaPaperError::Capture);
        }
        let _availability =
            validate_price_availability(e_as_of, e, Utc::now(), e_availability, PriceUse::ValueE)
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
                "readback": observation.readback().as_untrusted_value(),
                "admission_sha256": admission,
                "price_pins": {
                    "content_sha256": pins.content_sha256.as_str(),
                    "manifest_sha256": pins.manifest_sha256.as_str(),
                    "policy_sha256": pins.policy_sha256.as_str(),
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
        let expected_calculation_sha256 = calculation_hash.clone();
        let bytes = serde_json::to_vec(&value).map_err(|_| OwnerBetaPaperError::Snapshot)?;
        if bytes.len() > 262_144 {
            return Err(OwnerBetaPaperError::Snapshot);
        }
        trace_lifecycle(LifecycleStage::ValueCalculated);
        context
            .revalidate()
            .map_err(|_| OwnerBetaPaperError::Runtime)?;
        action.revalidate_files().map_err(map_action_input_error)?;
        trace_lifecycle(LifecycleStage::ValueSqlBegin);
        action
            .begin_transaction()
            .await
            .map_err(map_action_input_error)?;
        #[cfg(all(test, target_os = "linux"))]
        self.native_before_sql(action).await?;
        context
            .revalidate()
            .map_err(|_| OwnerBetaPaperError::Runtime)?;
        let reply: Result<Value, sqlx::Error> =
            sqlx::query_scalar("SELECT commit_owner_beta_paper_valuation_v1($1,$2,$3,$4)")
                .bind(target.pending_target_id)
                .bind(context.runtime().control_epoch())
                .bind(value)
                .bind(context.runtime().config_sha256())
                .fetch_one(&mut **action.transaction().map_err(map_action_input_error)?)
                .await;
        trace_lifecycle(LifecycleStage::ValueSqlReply);
        let reply = match reply {
            Ok(reply) => {
                action.retain_value(reply.clone());
                reply
            }
            Err(error) => {
                action.retain_sql_error(error);
                return Err(OwnerBetaPaperError::Sql);
            }
        };
        let committed = decode_action(reply, "COMMIT_VALUATION")?;
        if committed.outcome == "WAITING_SESSION" {
            action.rollback().await.map_err(map_action_input_error)?;
            return Ok(false);
        }
        if !matches!(committed.outcome.as_str(), "OK" | "REPLAY") {
            action.rollback().await.map_err(map_action_input_error)?;
            return Err(OwnerBetaPaperError::Snapshot);
        }
        let mut final_check = || {
            context
                .revalidate()
                .map_err(|_| PaperInputActionError::Runtime)?;
            validate_price_availability(e_as_of, e, Utc::now(), e_availability, PriceUse::ValueE)
                .map_err(|_| {
                PaperInputActionError::Input(PaperEligibilityReasonCode::PriceNotAvailable)
            })?;
            Ok(())
        };
        let commit = action.commit(&mut final_check).await;
        trace_lifecycle(LifecycleStage::ValueCommitReturn);
        match commit {
            Ok(()) => Ok(true),
            Err(_) if action.commit_started() => {
                let readback: Result<Value, sqlx::Error> =
                    sqlx::query_scalar("SELECT snapshot_owner_beta_paper_target_v1($1,$2,$3)")
                        .bind(target.pending_target_id)
                        .bind(context.runtime().control_epoch())
                        .bind(context.runtime().config_sha256())
                        .fetch_one(&self.pool)
                        .await;
                match readback {
                    Ok(value) => {
                        action.retain_value(value.clone());
                        let matches_request = target_readback_matches(
                            &value,
                            target.pending_target_id,
                            "valuation",
                            &expected_calculation_sha256,
                        );
                        action.retain_value(json!({"matches_request": matches_request}));
                    }
                    Err(error) => action.retain_sql_error(error),
                }
                Err(OwnerBetaPaperError::CommitUnknown)
            }
            Err(error) => Err(map_action_input_error(error)),
        }
    }
}

struct QueueSignalConsumer<'c, 's, 'attempt> {
    work: &'c OwnerBetaPreviewWork,
    context: &'c PaperActionContext<'c, 's, 'attempt>,
}

impl<'c, 's, 'attempt> PaperFinancialConsumer<(), OwnerBetaPaperError>
    for QueueSignalConsumer<'c, 's, 'attempt>
{
    fn run<'a, 'hold>(
        self,
        action: &'a mut CheckedPaperAction<'hold>,
    ) -> LocalInputFuture<'a, Result<(), OwnerBetaPaperError>>
    where
        Self: 'a,
        'hold: 'a,
    {
        Box::pin(async move {
            let signal = action.inputs().signal();
            let signal_date = signal.snapshot().as_of().as_naive_date();
            let signal_run_id = signal.input().run_id();
            let signal_as_of = signal.snapshot().as_of();
            let availability = signal.availability();
            let today = Utc::now()
                .with_timezone(&FixedOffset::east_opt(9 * 60 * 60).expect("fixed Seoul offset"))
                .date_naive();
            if signal_date != today {
                return Ok(());
            }
            self.context
                .revalidate()
                .map_err(|_| OwnerBetaPaperError::Runtime)?;
            action.revalidate_files().map_err(map_action_input_error)?;
            let owner_id = parse_runtime_uuid(self.context.runtime().owner_user_id())?;
            let account_id = parse_runtime_uuid(self.context.runtime().account_id())?;
            let expected_idempotency_key = format!(
                "owner-beta-paper-preview:{account_id}:auto:{signal_run_id}:{}",
                self.context.runtime().control_epoch()
            );
            action
                .begin_transaction()
                .await
                .map_err(map_action_input_error)?;
            #[cfg(all(test, target_os = "linux"))]
            self.work.native_before_sql(action).await?;
            let runtime = self.context.runtime();
            let tx = action.transaction().map_err(map_action_input_error)?;
            let result: Result<Value, sqlx::Error> =
                sqlx::query_scalar("SELECT queue_owner_beta_paper_target_v1($1,$2,$3)")
                    .bind(signal_run_id)
                    .bind(runtime.control_epoch())
                    .bind(runtime.config_sha256())
                    .fetch_one(&mut **tx)
                    .await;
            let value = match result {
                Ok(value) => {
                    action.retain_value(value.clone());
                    value
                }
                Err(error) => {
                    action.retain_sql_error(error);
                    return Err(OwnerBetaPaperError::Sql);
                }
            };
            #[cfg(all(test, target_os = "linux"))]
            if let Some(probe) = &self.work.native_probe {
                probe.queue_reply(&value)?;
            }
            let queued = decode_action(value, "QUEUE_TARGET")?;
            if matches!(
                queued.outcome.as_str(),
                "WAITING_CALENDAR" | "CONSENT_REQUIRED"
            ) {
                action.rollback().await.map_err(map_action_input_error)?;
                return Ok(());
            }
            if !matches!(queued.outcome.as_str(), "QUEUED" | "REPLAY") {
                action.rollback().await.map_err(map_action_input_error)?;
                return Err(OwnerBetaPaperError::Snapshot);
            }
            let preview_id = queued.data["preview_id"]
                .as_str()
                .and_then(|id| Uuid::parse_str(id).ok())
                .filter(|id| !id.is_nil())
                .ok_or(OwnerBetaPaperError::Snapshot)?;
            let job_id = queued.data["job_id"]
                .as_str()
                .and_then(|id| Uuid::parse_str(id).ok())
                .filter(|id| !id.is_nil())
                .ok_or(OwnerBetaPaperError::Snapshot)?;
            let effective = queued.data["effective_session"]
                .as_str()
                .and_then(|date| TradingDate::parse(date).ok())
                .ok_or(OwnerBetaPaperError::Snapshot)?;
            let mut final_check = || {
                self.context
                    .revalidate()
                    .map_err(|_| PaperInputActionError::Runtime)?;
                let current_day = Utc::now()
                    .with_timezone(&FixedOffset::east_opt(9 * 60 * 60).expect("fixed Seoul offset"))
                    .date_naive();
                if current_day != signal_date || signal_as_of.as_naive_date() != signal_date {
                    return Err(PaperInputActionError::Input(
                        PaperEligibilityReasonCode::EligibilityExpired,
                    ));
                }
                validate_price_availability(
                    signal_as_of,
                    effective,
                    Utc::now(),
                    availability,
                    PriceUse::ApplyT,
                )
                .map(|_| ())
                .map_err(|_| {
                    PaperInputActionError::Input(PaperEligibilityReasonCode::PriceNotAvailable)
                })?;
                if effective.as_naive_date() <= signal_date {
                    return Err(PaperInputActionError::Input(
                        PaperEligibilityReasonCode::CalendarUnproven,
                    ));
                }
                Ok(())
            };
            match action.commit(&mut final_check).await {
                Ok(()) => Ok(()),
                Err(_) if action.commit_started() => {
                    let readback: Result<Option<(String, Value, String)>, sqlx::Error> =
                        sqlx::query_as(
                            "SELECT status::text,payload_json,idempotency_key FROM public.jobs \
                         WHERE id=$1 AND owner_user_id=$2 \
                           AND job_type='owner_beta_paper_preview'",
                        )
                        .bind(job_id)
                        .bind(owner_id)
                        .fetch_optional(&self.work.pool)
                        .await;
                    match readback {
                        Ok(row) => {
                            action.retain_value(json!({"job_readback": row.clone()}));
                            let matches_request =
                                row.as_ref().is_some_and(|(status, payload, key)| {
                                    matches!(status.as_str(), "QUEUED" | "RUNNING" | "SUCCEEDED")
                                        && payload["schema"] == "owner-beta-paper-preview-job-v1"
                                        && payload["preview_id"] == preview_id.to_string()
                                        && payload["input_kind"] == "OWNER_BETA_DAILY_V1"
                                        && payload["apply_mode"] == "AUTO"
                                        && key == &expected_idempotency_key
                                });
                            action.retain_value(json!({"matches_request": matches_request}));
                        }
                        Err(error) => action.retain_sql_error(error),
                    }
                    Err(OwnerBetaPaperError::CommitUnknown)
                }
                Err(error) => Err(map_action_input_error(error)),
            }
        })
    }
}

struct PreviewConsumer<'c, 's, 'attempt> {
    work: &'c OwnerBetaPreviewWork,
    context: &'c PaperActionContext<'c, 's, 'attempt>,
    claim: &'c ClaimedJob,
    heartbeat: &'c ClaimHeartbeat,
    snapshot: PreviewSnapshot,
}

impl<'c, 's, 'attempt> PaperFinancialConsumer<(), OwnerBetaPaperError>
    for PreviewConsumer<'c, 's, 'attempt>
{
    fn run<'a, 'hold>(
        self,
        action: &'a mut CheckedPaperAction<'hold>,
    ) -> LocalInputFuture<'a, Result<(), OwnerBetaPaperError>>
    where
        Self: 'a,
        'hold: 'a,
    {
        Box::pin(async move {
            self.work
                .process_claim_inputs(
                    action,
                    self.context,
                    self.claim,
                    self.heartbeat,
                    &self.snapshot,
                )
                .await
        })
    }
}

struct PreviewCommitFence<'a> {
    queue: &'a JobQueue,
    lease: job_queue::queue::ClaimTransactionLease,
}

impl PreviewCommitFence<'_> {
    async fn check_action(
        &mut self,
        action: &mut CheckedPaperAction<'_>,
    ) -> Result<(), OwnerBetaPaperError> {
        let checked = self
            .queue
            .checkpoint_claim_transaction_lease_in(
                action.transaction().map_err(map_action_input_error)?,
                &mut self.lease,
            )
            .await;
        if let Err(error) = checked {
            action.retain_queue_error(error);
            return Err(OwnerBetaPaperError::LeaseLost);
        }
        Ok(())
    }
}

impl job_queue::owner_beta::paper::evidence::financial::PaperActionCommitFence
    for PreviewCommitFence<'_>
{
    fn checkpoint<'a>(
        &'a mut self,
        transaction: &'a mut sqlx::Transaction<'static, sqlx::Postgres>,
    ) -> LocalInputFuture<'a, Result<(), job_queue::QueueError>> {
        Box::pin(async move {
            self.queue
                .checkpoint_claim_transaction_lease_in(transaction, &mut self.lease)
                .await
        })
    }
    fn settle<'a>(
        &'a mut self,
        transaction: &'a mut sqlx::Transaction<'static, sqlx::Postgres>,
    ) -> LocalInputFuture<'a, Result<job_queue::SettleResult, job_queue::QueueError>> {
        Box::pin(async move {
            self.queue
                .settle_claim_transaction_lease_in(transaction, &mut self.lease)
                .await
        })
    }
    fn after_settlement<'a>(
        &'a mut self,
        transaction: &'a mut sqlx::Transaction<'static, sqlx::Postgres>,
    ) -> LocalInputFuture<'a, Result<(), job_queue::QueueError>> {
        Box::pin(async move {
            self.queue
                .seal_claim_transaction_lease_in(transaction, &mut self.lease)
                .await
        })
    }
}

// The two real economic consumers and their native fixture share this exact
// selector seam. Descriptive projection data never constructs checked evidence.
async fn with_checked_target_action<
    T: 'static,
    C: PaperFinancialConsumer<T, OwnerBetaPaperError>,
>(
    pool: &PgPool,
    context: &PaperActionContext<'_, '_, '_>,
    target: &TargetSnapshot,
    request: &PaperActionInputRequest,
    consumer: C,
) -> Result<CheckedPaperActionOutcome<T, OwnerBetaPaperError>, OwnerBetaPaperError> {
    Ok(if let Some(recovery) = &target.recovery_authority {
        let projection =
            serde_json::to_value(recovery).map_err(|_| OwnerBetaPaperError::Snapshot)?;
        with_checked_paper_recovery_action(
            pool,
            context,
            request,
            target.pending_target_id,
            &projection,
            consumer,
        )
        .await
    } else {
        with_checked_paper_action(pool, context, request, consumer).await
    })
}

struct SettlementConsumer<'c, 's, 'attempt, 'target> {
    work: &'c OwnerBetaPreviewWork,
    context: &'c PaperActionContext<'c, 's, 'attempt>,
    target: &'target TargetSnapshot,
}

impl<'c, 's, 'attempt, 'target> PaperFinancialConsumer<bool, OwnerBetaPaperError>
    for SettlementConsumer<'c, 's, 'attempt, 'target>
{
    fn run<'a, 'hold>(
        self,
        action: &'a mut CheckedPaperAction<'hold>,
    ) -> LocalInputFuture<'a, Result<bool, OwnerBetaPaperError>>
    where
        Self: 'a,
        'hold: 'a,
    {
        Box::pin(async move {
            self.work
                .settle_target_with(action, self.context, self.target)
                .await
        })
    }
}

struct ValuationConsumer<'c, 's, 'attempt, 'target> {
    work: &'c OwnerBetaPreviewWork,
    context: &'c PaperActionContext<'c, 's, 'attempt>,
    target: &'target TargetSnapshot,
}

impl<'c, 's, 'attempt, 'target> PaperFinancialConsumer<bool, OwnerBetaPaperError>
    for ValuationConsumer<'c, 's, 'attempt, 'target>
{
    fn run<'a, 'hold>(
        self,
        action: &'a mut CheckedPaperAction<'hold>,
    ) -> LocalInputFuture<'a, Result<bool, OwnerBetaPaperError>>
    where
        Self: 'a,
        'hold: 'a,
    {
        Box::pin(async move {
            self.work
                .value_target_with(action, self.context, self.target)
                .await
        })
    }
}

type ClaimHeartbeatResult = std::sync::Arc<Result<HeartbeatStatus, job_queue::QueueError>>;

struct ClaimHeartbeat {
    stop: watch::Sender<bool>,
    lost: watch::Receiver<bool>,
    lost_tx: watch::Sender<bool>,
    task: tokio::sync::Mutex<ClaimHeartbeatTask>,
    runtime: tokio::runtime::Handle,
}

struct ClaimHeartbeatTask {
    running: Option<JoinHandle<Option<ClaimHeartbeatResult>>>,
    joined: Option<Result<Option<ClaimHeartbeatResult>, tokio::task::JoinError>>,
}

impl ClaimHeartbeat {
    fn start(queue: JobQueue, claim: ClaimedJob, heartbeat_period: Duration) -> Self {
        #[cfg(test)]
        {
            Self::start_inner(queue, claim, heartbeat_period, None)
        }
        #[cfg(not(test))]
        {
            Self::start_inner(queue, claim, heartbeat_period)
        }
    }

    #[cfg(all(test, target_os = "linux"))]
    fn start_observed(
        queue: JobQueue,
        claim: ClaimedJob,
        heartbeat_period: Duration,
        observer: tokio::sync::mpsc::Sender<ClaimHeartbeatResult>,
    ) -> Self {
        Self::start_inner(queue, claim, heartbeat_period, Some(observer))
    }

    fn start_inner(
        queue: JobQueue,
        claim: ClaimedJob,
        heartbeat_period: Duration,
        #[cfg(test)] mut observer: Option<tokio::sync::mpsc::Sender<ClaimHeartbeatResult>>,
    ) -> Self {
        let (stop, mut stopped) = watch::channel(false);
        let (lost_tx, lost) = watch::channel(false);
        let task_lost = lost_tx.clone();
        let task = tokio::spawn(async move {
            let mut last = None;
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(heartbeat_period) => {
                        let result = std::sync::Arc::new(queue.heartbeat(&claim).await);
                        let extended = matches!(result.as_ref(), Ok(HeartbeatStatus::Extended));
                        if !extended {
                            let _ = task_lost.send(true);
                        }
                        #[cfg(test)]
                        if let Some(observer) = observer.as_mut() {
                            let _ = observer.try_send(std::sync::Arc::clone(&result));
                        }
                        last = Some(result);
                        if !extended { return last; }
                    }
                    changed = stopped.changed() => {
                        if changed.is_err() || *stopped.borrow() { return last; }
                    }
                }
            }
        });
        Self {
            stop,
            lost,
            lost_tx,
            task: tokio::sync::Mutex::new(ClaimHeartbeatTask {
                running: Some(task),
                joined: None,
            }),
            runtime: tokio::runtime::Handle::current(),
        }
    }

    async fn quiesce(&self) -> Result<(), OwnerBetaPaperError> {
        let _ = self.stop.send(true);
        let mut state = self.task.lock().await;
        if let Some(task) = state.running.as_mut() {
            // Keep the handle IN its owner across await, so cancellation of
            // this join future cannot detach/erase the in-flight result.
            let joined = task.await;
            let failed = match &joined {
                Ok(Some(result)) => !matches!(result.as_ref(), Ok(HeartbeatStatus::Extended)),
                Ok(None) => false,
                Err(_) => true,
            };
            if failed {
                let _ = self.lost_tx.send(true);
            }
            state.running = None;
            state.joined = Some(joined);
        }
        ensure_claim_live(&self.lost)
    }

    async fn stop(&self) {
        let _ = self.quiesce().await;
    }

    async fn run_and_join<F: std::future::Future>(&self, work: F) -> F::Output {
        // Catch inside the async owner: the per-action executor must remain
        // alive until the real in-flight heartbeat has returned and joined.
        let outcome =
            futures_util::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(work)).await;
        self.stop().await;
        match outcome {
            Ok(value) => value,
            Err(original_panic) => std::panic::resume_unwind(original_panic),
        }
    }

    #[cfg(test)]
    async fn joined(&self) -> bool {
        let state = self.task.lock().await;
        state.running.is_none() && state.joined.is_some()
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
        if let Some(task) = self.task.get_mut().running.take() {
            // Last-resort stop requests never abort in-flight SQL or certify
            // cleanup. The production owner uses run_and_join while its
            // per-action executor is alive, including callback unwinds.
            let _ = self.lost_tx.send(true);
            let _drain = self.runtime.spawn(async move {
                let _joined_outcome = task.await;
            });
        }
    }
}

#[cfg(test)]
mod heartbeat_handoff_tests {
    use super::*;
    use std::{future::Future, task::Poll};

    fn ordinary_task(task: JoinHandle<Option<ClaimHeartbeatResult>>) -> ClaimHeartbeat {
        let (stop, _) = watch::channel(false);
        let (lost_tx, lost) = watch::channel(false);
        ClaimHeartbeat {
            stop,
            lost,
            lost_tx,
            task: tokio::sync::Mutex::new(ClaimHeartbeatTask {
                running: Some(task),
                joined: None,
            }),
            runtime: tokio::runtime::Handle::current(),
        }
    }

    #[tokio::test]
    async fn canceled_join_future_keeps_actual_task_until_acknowledged_quiesce() {
        let (release, wait) = tokio::sync::oneshot::channel();
        let heartbeat = ordinary_task(tokio::spawn(async move {
            wait.await.unwrap();
            None
        }));
        let mut joining = Box::pin(heartbeat.quiesce());
        std::future::poll_fn(|context| {
            assert!(
                joining.as_mut().poll(context).is_pending(),
                "stop request is not a join acknowledgment"
            );
            Poll::Ready(())
        })
        .await;
        drop(joining);
        assert!(
            heartbeat.task.lock().await.running.is_some(),
            "canceled await detached the handle"
        );
        release.send(()).unwrap();
        assert!(heartbeat.quiesce().await.is_ok());
        assert!(heartbeat.joined().await && !*heartbeat.lost.borrow());
        assert!(
            heartbeat.quiesce().await.is_ok(),
            "second join must be idempotent"
        );
    }

    #[tokio::test]
    async fn join_panic_is_retained_and_cannot_certify_live_handoff() {
        let heartbeat = ordinary_task(tokio::spawn(async { panic!("ordinary join sentinel") }));
        assert!(matches!(
            heartbeat.quiesce().await,
            Err(OwnerBetaPaperError::LeaseLost)
        ));
        assert!(heartbeat.joined().await && *heartbeat.lost.borrow());
        let state = heartbeat.task.lock().await;
        assert!(matches!(&state.joined, Some(Err(error)) if error.is_panic()));
    }

    #[tokio::test]
    async fn completed_task_cannot_clear_an_existing_sticky_loss() {
        let heartbeat = ordinary_task(tokio::spawn(async { None }));
        heartbeat.lost_tx.send(true).unwrap();
        assert!(matches!(
            heartbeat.quiesce().await,
            Err(OwnerBetaPaperError::LeaseLost)
        ));
        assert!(heartbeat.joined().await && *heartbeat.lost.borrow());
    }

    #[tokio::test]
    async fn actual_returned_queue_failure_remains_whole_after_join() {
        let original = std::sync::Arc::new(Err(job_queue::QueueError::Database(
            sqlx::Error::PoolTimedOut,
        )));
        let returned = std::sync::Arc::clone(&original);
        let heartbeat = ordinary_task(tokio::spawn(async move { Some(returned) }));
        assert!(matches!(
            heartbeat.quiesce().await,
            Err(OwnerBetaPaperError::LeaseLost)
        ));
        let state = heartbeat.task.lock().await;
        let Some(Ok(Some(actual))) = &state.joined else {
            panic!("whole returned queue failure missing")
        };
        assert!(std::sync::Arc::ptr_eq(&original, actual));
        assert!(matches!(
            actual.as_ref(),
            Err(job_queue::QueueError::Database(sqlx::Error::PoolTimedOut))
        ));
        assert!(*heartbeat.lost.borrow());
    }

    #[tokio::test]
    async fn canceled_task_join_cannot_certify_live_handoff() {
        let task =
            tokio::spawn(async { std::future::pending::<Option<ClaimHeartbeatResult>>().await });
        task.abort(); // Ordinary task cancellation input, never an SQL heartbeat.
        let heartbeat = ordinary_task(task);
        assert!(matches!(
            heartbeat.quiesce().await,
            Err(OwnerBetaPaperError::LeaseLost)
        ));
        let state = heartbeat.task.lock().await;
        assert!(matches!(&state.joined, Some(Err(error)) if error.is_cancelled()));
    }

    #[test]
    fn panicking_claim_joins_before_private_runtime_is_dropped() {
        let original = std::sync::Arc::new(Err(job_queue::QueueError::Database(
            sqlx::Error::PoolTimedOut,
        )));
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            runtime.block_on(async {
                let (release, wait) = tokio::sync::oneshot::channel();
                let returned = std::sync::Arc::clone(&original);
                let heartbeat = ordinary_task(tokio::spawn(async move {
                    wait.await.unwrap();
                    Some(returned)
                }));
                let guarded = futures_util::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(
                    heartbeat.run_and_join(async { panic!("claim owner original panic") }),
                ));
                let mut guarded = Box::pin(guarded);
                std::future::poll_fn(|context| {
                    assert!(
                        guarded.as_mut().poll(context).is_pending(),
                        "original unwind escaped before the real heartbeat joined"
                    );
                    Poll::Ready(())
                })
                .await;
                assert!(
                    heartbeat.task.try_lock().is_err(),
                    "the pending join must retain the original task owner"
                );
                release.send(()).unwrap();
                let original_panic = match guarded.await {
                    Err(original_panic) => original_panic,
                    Ok(()) => panic!("claim panic was swallowed"),
                };
                let state = heartbeat.task.lock().await;
                assert!(state.running.is_none());
                let Some(Ok(Some(actual))) = &state.joined else {
                    panic!("actual heartbeat result was lost during unwind")
                };
                assert!(std::sync::Arc::ptr_eq(&original, actual));
                assert!(matches!(
                    actual.as_ref(),
                    Err(job_queue::QueueError::Database(sqlx::Error::PoolTimedOut))
                ));
                assert!(*heartbeat.lost.borrow());
                drop(state);
                std::panic::resume_unwind(original_panic);
            });
        }));
        // This is the actual per-action runtime lifetime boundary. The join
        // and whole error assertions above ran before its shutdown.
        drop(runtime);
        let original_panic = unwound.expect_err("claim callback must still unwind");
        assert_eq!(
            original_panic.downcast_ref::<&str>(),
            Some(&"claim owner original panic")
        );
    }

    #[tokio::test]
    async fn dropped_handoff_drains_inflight_task_without_aborting_it() {
        let (release, wait) = tokio::sync::oneshot::channel();
        let (finished, completion) = tokio::sync::oneshot::channel();
        let heartbeat = ordinary_task(tokio::spawn(async move {
            wait.await.unwrap();
            finished.send(()).unwrap();
            None
        }));
        drop(heartbeat);
        release.send(()).unwrap();
        completion
            .await
            .expect("Drop aborted the in-flight operation");
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

fn target_readback_matches(
    value: &Value,
    target_id: Uuid,
    receipt_kind: &str,
    expected_calculation_sha256: &str,
) -> bool {
    let durable_hash_field = match receipt_kind {
        "settlement" => "settlement_sha256",
        "valuation" => "valuation_sha256",
        _ => return false,
    };
    let Ok(envelope) = decode_action(value.clone(), "SNAPSHOT_TARGET") else {
        return false;
    };
    if envelope.outcome != "OK" {
        return false;
    }
    let Some(actual_target_id) = envelope.data["pending_target_id"]
        .as_str()
        .and_then(|id| Uuid::parse_str(id).ok())
    else {
        return false;
    };
    actual_target_id == target_id
        && envelope.data[durable_hash_field].as_str() == Some(expected_calculation_sha256)
        && envelope.data["receipt_json"][receipt_kind]["calculation_sha256"].as_str()
            == Some(expected_calculation_sha256)
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
    observation: &PaperSignalObservation,
    inputs: &PaperActionInputs<'_>,
    context: &PaperActionContext<'_, '_, '_>,
    #[cfg(all(test, target_os = "linux"))] native_probe: Option<&action_native_tests::Probe>,
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
    let proof = inputs
        .calendar_range_proof()
        .ok_or(OwnerBetaPaperError::Capture)?;
    let start =
        TradingDate::parse(&snapshot.signal_date).map_err(|_| OwnerBetaPaperError::Snapshot)?;
    let end = TradingDate::parse(&snapshot.effective_session)
        .map_err(|_| OwnerBetaPaperError::Snapshot)?;
    validate_calendar_link(
        start,
        end,
        &snapshot.calendar_evidence_sha256,
        &snapshot.calendar_link,
        proof,
        context,
        #[cfg(all(test, target_os = "linux"))]
        native_probe,
    )?;
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
    observation: &PaperSignalObservation,
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
    observation: &PaperSignalObservation,
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
    observation: &PaperSignalObservation,
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

// Descriptive SQL projection only. Its exact fields are independently matched
// to committed grant/dispatch rows by the worker checker before any economics.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RecoveryAuthority {
    grant_id: Uuid,
    active_epoch: i64,
    active_process_sha256: String,
    active_consent_id: Uuid,
    step: RecoveryStep,
    expires_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum RecoveryStep {
    SettleThenValue,
    ValueOnly,
}

fn deserialize_recovery_authority<'de, D>(
    deserializer: D,
) -> Result<Option<RecoveryAuthority>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    // Missing is ordinary; explicit null is not the closed recovery projection.
    RecoveryAuthority::deserialize(deserializer).map(Some)
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TargetSnapshot {
    #[serde(
        default,
        deserialize_with = "deserialize_recovery_authority",
        skip_serializing
    )]
    recovery_authority: Option<RecoveryAuthority>,
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
    let historical = if let Some(recovery) = &snapshot.recovery_authority {
        if recovery.grant_id.is_nil()
            || recovery.active_epoch != runtime.control_epoch()
            || recovery.active_process_sha256 != runtime.config_sha256()
            || recovery.active_consent_id.to_string() != runtime.binding().consent_event_id()
            || recovery.expires_at <= Utc::now()
            || snapshot.control_epoch >= recovery.active_epoch
            || snapshot.control_epoch < 0
            || snapshot.consent_event_id.is_nil()
            || snapshot.effective_session
                >= Utc::now()
                    .with_timezone(&FixedOffset::east_opt(9 * 3600).expect("KST"))
                    .date_naive()
            || (snapshot.settlement_sha256.is_none()
                && recovery.step != RecoveryStep::SettleThenValue)
            || snapshot.valuation_sha256.is_some()
        {
            return Err(OwnerBetaPaperError::Snapshot);
        }
        true
    } else {
        false
    };
    // Current cohort stays current; only the explicitly granted original
    // lineage compares epochs/consent with its actual T Source capture below.
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
        || (!historical
            && snapshot.consent_event_id.to_string() != runtime.binding().consent_event_id())
        || (!historical && snapshot.control_epoch != runtime.control_epoch())
        || snapshot.config_epoch < 0
        || snapshot.daily_activation_epoch < 0
        || snapshot.account_state_version < 0
        || snapshot.signal_date >= snapshot.effective_session
        || snapshot.current_cohort != context.binding()["cohort"]
        || cohort["owner_user_id"].as_str() != Some(runtime.owner_user_id())
        || cohort["strategy_config_id"].as_str() != Some(runtime.strategy_config_id())
        || (!historical && cohort["config_epoch"].as_i64() != Some(snapshot.config_epoch))
        || (!historical
            && cohort["activation_epoch"].as_i64() != Some(snapshot.daily_activation_epoch))
        || (!historical && snapshot.cohort_sha256 != runtime.binding().daily_cohort_sha256())
        || !is_prefixed_hash(&snapshot.cohort_sha256)
        || !is_prefixed_hash(&snapshot.process_binding_sha256)
        || (historical && snapshot.process_binding_sha256 != runtime.config_sha256())
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
    inputs: &'a PaperActionInputs<'_>,
    context: &PaperActionContext<'_, '_, '_>,
    #[cfg(all(test, target_os = "linux"))] native_probe: Option<&action_native_tests::Probe>,
) -> Result<
    (
        &'a PaperSignalObservation<'a>,
        &'a PaperAdmittedPriceObservation<'a>,
    ),
    OwnerBetaPaperError,
> {
    let t = inputs.signal();
    let e = inputs.execution().ok_or(OwnerBetaPaperError::Capture)?;
    let t_input = t.input();
    if snapshot.recovery_authority.is_some()
        && t.readback().as_untrusted_value()["slot"]["cohort_sha256"] != snapshot.cohort_sha256
    {
        return Err(OwnerBetaPaperError::Capture);
    }
    let t_target = t.target();
    let e_pins = e.snapshot().pins();
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
        || snapshot.execution_admission_sha256.as_deref() != Some(e_pins.admission_sha256.as_str())
        || e_pins.policy_sha256.as_str() != t_input.pins().policy_sha256().as_str()
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
    validate_calendar_link(
        TradingDate::parse(&snapshot.signal_date.to_string())
            .map_err(|_| OwnerBetaPaperError::Snapshot)?,
        TradingDate::parse(&snapshot.effective_session.to_string())
            .map_err(|_| OwnerBetaPaperError::Snapshot)?,
        &snapshot.calendar_evidence_sha256,
        &snapshot.calendar_link,
        inputs
            .calendar_range_proof()
            .ok_or(OwnerBetaPaperError::Capture)?,
        context,
        #[cfg(all(test, target_os = "linux"))]
        native_probe,
    )?;
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
    start: TradingDate,
    end: TradingDate,
    calendar_evidence_sha256: &str,
    link: &Value,
    proof: &market_data::daily_price_only::calendar::CalendarRangeProof,
    context: &PaperActionContext<'_, '_, '_>,
    #[cfg(all(test, target_os = "linux"))] native_probe: Option<&action_native_tests::Probe>,
) -> Result<(), OwnerBetaPaperError> {
    #[cfg(all(test, target_os = "linux"))]
    if let Some(probe) = native_probe {
        probe.calendar_entered();
    }
    let span = end
        .as_naive_date()
        .signed_duration_since(start.as_naive_date())
        .num_days();
    if span < 1 || span as u64 > 369 {
        return Err(OwnerBetaPaperError::Capture);
    }
    let length = usize::try_from(span + 1).map_err(|_| OwnerBetaPaperError::Capture)?;
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
                .any(|hash| hash.as_str() == calendar_evidence_sha256)
        })
    {
        return Err(OwnerBetaPaperError::Capture);
    }
    for (index, day) in proof.days().iter().enumerate() {
        let offset = i64::try_from(index).map_err(|_| OwnerBetaPaperError::Capture)?;
        if day.date()
            != start
                .checked_add_days(offset)
                .map_err(|_| OwnerBetaPaperError::Capture)?
            || day.witnesses().is_empty()
        {
            return Err(OwnerBetaPaperError::Capture);
        }
    }
    let rows = link["covered_days"]
        .as_array()
        .ok_or(OwnerBetaPaperError::Snapshot)?;
    if link["schema"] != "owner-beta-paper-calendar-link-v1"
        || link["after"].as_str() != Some(start.to_string().as_str())
        || link["effective_session"].as_str() != Some(end.to_string().as_str())
        || link["calendar_evidence_sha256"].as_str() != Some(calendar_evidence_sha256)
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
    t: &PaperSignalObservation<'_>,
    e: &PaperAdmittedPriceObservation<'_>,
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
        "t_readback": t.readback().as_untrusted_value(),
        "e_readback": e.readback().as_untrusted_value(),
        "t_pins": {
            "admission_sha256": t.input().pins().admission_sha256().as_str(),
            "content_sha256": t.input().pins().content_sha256().as_str(),
            "manifest_sha256": t.input().pins().manifest_sha256().as_str(),
            "policy_sha256": t.input().pins().policy_sha256().as_str(),
        },
        "e_pins": {
            "admission_sha256": e.snapshot().pins().admission_sha256.as_str(),
            "content_sha256": e.snapshot().pins().content_sha256.as_str(),
            "manifest_sha256": e.snapshot().pins().manifest_sha256.as_str(),
            "policy_sha256": e.snapshot().pins().policy_sha256.as_str(),
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
        "execution_admission_sha256": e.snapshot().pins().admission_sha256.as_str(),
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

fn observed_input(observation: &PaperAdmittedPriceObservation<'_>) -> Value {
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

    #[test]
    fn configured_heartbeat_must_be_positive_and_shorter_than_lease() {
        assert!(valid_heartbeat_period(
            Duration::from_secs(1),
            Duration::from_secs(5)
        ));
        assert!(!valid_heartbeat_period(
            Duration::ZERO,
            Duration::from_secs(5)
        ));
        assert!(!valid_heartbeat_period(
            Duration::from_secs(5),
            Duration::from_secs(5)
        ));
        assert!(!valid_heartbeat_period(
            Duration::from_secs(6),
            Duration::from_secs(5)
        ));
    }

    #[tokio::test]
    async fn invalid_non_cli_heartbeat_is_rejected_before_bound_runtime() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy_with(sqlx::postgres::PgConnectOptions::new());
        let queue = JobQueue::new(pool.clone(), None, job_queue::QueueConfig::default());
        assert!(matches!(
            run_preview_once(&pool, &queue, "fixture-worker", Duration::ZERO, None).await,
            Err(OwnerBetaPaperError::Runtime)
        ));
    }

    #[test]
    fn target_reconciliation_requires_exact_target_and_receipt_hash() {
        let target_id = Uuid::from_u128(1);
        let other_target_id = Uuid::from_u128(2);
        let hash = format!("sha256:{}", "a".repeat(64));
        let readback = json!({
            "schema": "owner-beta-paper-action-v1",
            "action": "SNAPSHOT_TARGET",
            "outcome": "OK",
            "control_epoch": 1,
            "account_id": Uuid::from_u128(3),
            "data": {
                "pending_target_id": target_id,
                "settlement_sha256": hash.clone(),
                "valuation_sha256": hash.clone(),
                "receipt_json": {
                    "settlement": {"calculation_sha256": hash.clone()},
                    "valuation": {"calculation_sha256": hash.clone()}
                }
            }
        });

        assert!(target_readback_matches(
            &readback,
            target_id,
            "settlement",
            &hash
        ));
        assert!(target_readback_matches(
            &readback,
            target_id,
            "valuation",
            &hash
        ));
        assert!(!target_readback_matches(
            &readback,
            other_target_id,
            "settlement",
            &hash
        ));
        assert!(!target_readback_matches(
            &readback,
            target_id,
            "settlement",
            &format!("sha256:{}", "b".repeat(64))
        ));
        let mut malformed = readback;
        malformed["outcome"] = Value::String("STALE".to_owned());
        assert!(!target_readback_matches(
            &malformed,
            target_id,
            "settlement",
            &hash
        ));
    }

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

#[cfg(test)]
mod recovery_decoder_tests {
    use super::*;

    // Decoder-only data; never a Current, grant, Source or financial capability.
    fn snapshot_data() -> Value {
        let id = Uuid::from_u128(1);
        let hash = format!("sha256:{}", "a".repeat(64));
        json!({"pending_target_id":id,"lineage_id":id,"recommendation_run_id":id,
            "owner_user_id":id,"account_id":id,"strategy_config_id":id,"config_epoch":1,
            "daily_activation_epoch":1,"control_epoch":1,"daily_pins":{},"current_cohort":{},
            "cohort_sha256":hash,"consent_event_id":id,"activation_event_id":id,
            "process_binding_sha256":hash,"account_state_version":0,"account_state_sha256":"a".repeat(64),
            "initial_account_state_sha256":"a".repeat(64),"account_state_preimage":{},"target_weights":[],
            "previous_target_weights":null,"signal_date":"2026-01-02","effective_session":"2026-01-05",
            "signal_admission_sha256":hash,"execution_admission_sha256":hash,"factor_snapshot_sha256":hash,
            "target_snapshot_sha256":hash,"calendar_evidence_sha256":hash,"calendar_link":{},
            "target_status":"PENDING","session_state":"RECOVERY_REQUIRED","settlement_sha256":null,
            "ledger_after_sha256":null,"ledger_after_version":null,"valuation_sha256":null,"receipt_json":{}})
    }

    #[test]
    fn private_target_decoder_requires_exact_recovery_projection() {
        let mut value = snapshot_data();
        assert!(serde_json::from_value::<TargetSnapshot>(value.clone()).is_ok());
        let projection = json!({"grant_id":Uuid::from_u128(2),"active_epoch":3,
            "active_process_sha256":format!("sha256:{}", "b".repeat(64)),
            "active_consent_id":Uuid::from_u128(3),"step":"SETTLE_THEN_VALUE",
            "expires_at":"2026-11-10T15:00:00Z"});
        value["recovery_authority"] = projection.clone();
        assert!(
            serde_json::from_value::<TargetSnapshot>(value.clone()).is_ok(),
            "data decoding is not authority"
        );
        for bad in [
            Value::Null,
            json!(true),
            json!({}),
            json!({"grant_id":Uuid::from_u128(2)}),
        ] {
            value["recovery_authority"] = bad;
            assert!(serde_json::from_value::<TargetSnapshot>(value.clone()).is_err());
        }
        value["recovery_authority"] = projection.clone();
        value["recovery_authority"]["checked"] = json!(true);
        assert!(serde_json::from_value::<TargetSnapshot>(value.clone()).is_err());
        value["recovery_authority"] = projection;
        value["recovery_authority"]["step"] = json!("AUTO_RESUME");
        assert!(serde_json::from_value::<TargetSnapshot>(value.clone()).is_err());
        value["recovery_authority"]["step"] = json!("VALUE_ONLY");
        value["unexpected"] = json!(true);
        assert!(serde_json::from_value::<TargetSnapshot>(value).is_err());
    }

    #[test]
    fn recovery_selector_is_excluded_from_ordinary_financial_snapshot_bytes() {
        let original = snapshot_data();
        let mut selected = original.clone();
        selected["recovery_authority"] = json!({"grant_id":Uuid::from_u128(2),"active_epoch":3,
            "active_process_sha256":format!("sha256:{}", "b".repeat(64)),
            "active_consent_id":Uuid::from_u128(3),"step":"VALUE_ONLY","expires_at":"2026-11-10T15:00:00Z"});
        let ordinary: TargetSnapshot = serde_json::from_value(original).unwrap();
        let recovery: TargetSnapshot = serde_json::from_value(selected).unwrap();
        assert_eq!(
            serde_json::to_vec(&ordinary).unwrap(),
            serde_json::to_vec(&recovery).unwrap()
        );
        assert_eq!(ordinary.control_epoch, recovery.control_epoch);
        assert_eq!(ordinary.consent_event_id, recovery.consent_event_id);
    }
}
