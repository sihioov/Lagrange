//! Fresh disposable-database acceptance for the protected Owner-beta Paper path.
//!
//! The test is intentionally ignored in ordinary suites. The sealed workflow
//! driver invokes this exact test executable from its root-owned, read-only
//! fixture volume and supplies database URLs through its private allowlist.

use api_server::http::{
    api_router,
    state::{
        ApiConfig, ApiState, OwnerBetaAccessMode, OwnerBetaPaperMode, OwnerIntradayQuoteReadConfig,
    },
};
use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use domain::{InstrumentId, TradingDate};
use futures_util::FutureExt;
use http_body_util::BodyExt;
use job_queue::owner_beta::paper::runtime::{PaperProcessRole, TrustedPaperRuntime};
use job_queue::owner_beta::paper::{
    action::{
        BoundPaperActionErrorKind, PaperActionFuture, PaperActionRefusal, PaperActionWork,
        run_bound_paper_action,
    },
    evidence::financial::{
        CheckedPaperAction, LocalInputFuture, PaperActionInputRequest, PaperFinancialConsumer,
        PaperInputActionError, with_checked_paper_action,
    },
};
use portfolio_model::{PendingTarget, Side};
use serde_json::Value;
use sqlx::{
    ConnectOptions, PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{
    cell::Cell,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{oneshot, watch};
use tower::ServiceExt;
use uuid::Uuid;

const CONTRACT: &str = r#"{"contract_id":"owner-beta-paper-v1-draft-1","contract_sha256":"sha256:18cde9fa7f15e19c9ec272abb780d06ae4be672068ed3f9208c39dc2297ab8d0","decision_sha256":"sha256:8ef602a35586d3e2ca532c5fbb5efc6f2e727eb25ebeb45a44dad0ea0d0ca394","cost_profile_id":"KRX_ETF_DEFAULT","cost_profile_version":1}"#;

const HTTP_DIAGNOSTIC_CODES: &[&str] = &[
    "SESSION_UNKNOWN",
    "SESSION_EXPIRED",
    "FORBIDDEN",
    "CSRF_DENIED",
    "DATA_ENTITLEMENT_REQUIRED",
    "PAPER_DISABLED",
    "PAPER_UNBOUND",
    "PAPER_STALE",
    "PAPER_CONFLICT",
    "PAPER_NOT_READY",
    "PAPER_RIGHTS_BLOCKED",
    "PAPER_SOURCE_BLOCKED",
    "PAPER_COMMIT_UNKNOWN",
    "RESOURCE_NOT_FOUND",
    "IDEMPOTENCY_KEY_REQUIRED",
    "IDEMPOTENCY_KEY_MISMATCH",
    "INVALID_PARAMETER",
    "INVALID_DATE",
    "INTERNAL",
];

#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum ProgressStage {
    PhaseBegin,
    PhaseEnd,
    PoolsOpenBegin,
    PoolsOpenEnd,
    FixtureSessionBegin,
    FixtureSessionEnd,
    TrustedApiLoadBegin,
    TrustedApiLoadEnd,
    TrustedWorkerLoadBegin,
    TrustedWorkerLoadEnd,
    ApiStateBegin,
    ApiStateEnd,
    ServicesBegin,
    ServicesEnd,
    CycleReturnErr,
    BetaStageOk,
    BetaStageTimedOut,
    BetaStageCanceled,
    BetaStageFailedRuntime,
    BetaStageFailedCapture,
    BetaStageFailedSql,
    BetaStageFailedSnapshot,
    BetaStageFailedCalculation,
    BetaStageFailedRetry,
    BetaStageFailedLeaseLost,
    BetaStageFailedCommitUnknown,
    CycleDeadlinePreview,
    CycleDeadlineOther,
    CycleShutdown,
    CyclePreviewRejected,
    CyclePendingScanRejected,
    CycleAccountScanRejected,
    CycleAnnouncementScanRejected,
    CycleInvalidDate,
    BetaBoundBegin,
    BetaBoundReturn,
    BetaCallbackBegin,
    BetaCallbackReturn,
    BetaDueBegin,
    BetaDueReturn,
    BetaSettleCaptureBegin,
    BetaSettleCaptureReturn,
    BetaSettleCalculated,
    BetaSettleSqlBegin,
    BetaSettleSqlReply,
    BetaSettleCommitReturn,
    BetaValueCaptureBegin,
    BetaValueCaptureReturn,
    BetaValueCalculated,
    BetaValueSqlBegin,
    BetaValueSqlReply,
    BetaValueCommitReturn,
    BetaLatestCaptureBegin,
    BetaLatestCaptureReturn,
    BetaAutoQueueReturn,
    BetaClaimCaptureBegin,
    BetaClaimCaptureReturn,
    BetaPublishReply,
    BetaPrecommitClaimLost,
    BetaPrecommitContextRefused,
    BetaPrecommitDeadline,
    BetaCommitRuntimeRefused,
    BetaCommitSqlRefused,
    BetaCommitInputRefused,
    BetaCommitStateRefused,
    BetaCommitCleanupRefused,
    BetaCommitUnknownRefused,
    PostPreviewBegin,
    PostPreviewEnd,
    WorkerCycleBegin,
    WorkerCycleEnd,
    WorkerStageTimedOut,
    WorkerItemError,
    WorkerRuntimeRejected,
    WorkerCaptureRejected,
    WorkerSqlRejected,
    WorkerSnapshotInvalid,
    WorkerCalculationRejected,
    WorkerRetryRequired,
    WorkerLeaseLost,
    WorkerCommitUnknown,
    GetPreviewBegin,
    GetPreviewEnd,
    ApplyBegin,
    ApplyEnd,
    ApplyReplayBegin,
    ApplyReplayEnd,
    DurableAssertionsBegin,
    DurableAssertionsEnd,
    PoolsCloseBegin,
    PoolsCloseEnd,
    AppPoolCloseBegin,
    AppPoolCloseEnd,
    AdminPoolCloseBegin,
    AdminPoolCloseEnd,
    AuditPoolCloseBegin,
    AuditPoolCloseEnd,
    WorkerPoolCloseBegin,
    WorkerPoolCloseEnd,
    MigrationPoolCloseBegin,
    MigrationPoolCloseEnd,
    InspectionPoolCloseBegin,
    InspectionPoolCloseEnd,
}

fn progress(stage: ProgressStage) {
    const MAX_ELAPSED_MS: u64 = 3_600_000;
    static START: OnceLock<Instant> = OnceLock::new();
    let elapsed_ms = u64::try_from(START.get_or_init(Instant::now).elapsed().as_millis())
        .unwrap_or(MAX_ELAPSED_MS)
        .min(MAX_ELAPSED_MS);
    eprintln!(
        "R5_PAPER_NATIVE_PROGRESS {}",
        serde_json::json!({"stage": stage, "elapsed_ms": elapsed_ms})
    );
}

const LIFECYCLE_TARGET: &str = "owner_beta_paper_lifecycle";
const MAX_ENGINE_EVENTS: usize = 24;
const MAX_BETA_OUTCOMES: usize = 8;
const BETA_OUTCOME_CODES: &[&str] = &[
    "BETA_STAGE_OK",
    "BETA_STAGE_TIMED_OUT",
    "BETA_STAGE_CANCELED",
    "BETA_STAGE_FAILED_RUNTIME",
    "BETA_STAGE_FAILED_CAPTURE",
    "BETA_STAGE_FAILED_SQL",
    "BETA_STAGE_FAILED_SNAPSHOT",
    "BETA_STAGE_FAILED_CALCULATION",
    "BETA_STAGE_FAILED_RETRY",
    "BETA_STAGE_FAILED_LEASE_LOST",
    "BETA_STAGE_FAILED_COMMIT_UNKNOWN",
];
const LIFECYCLE_CODES: &[&str] = &[
    "BETA_BOUND_BEGIN",
    "BETA_BOUND_RETURN",
    "BETA_CALLBACK_BEGIN",
    "BETA_CALLBACK_RETURN",
    "BETA_DUE_BEGIN",
    "BETA_DUE_RETURN",
    "BETA_SETTLE_CAPTURE_BEGIN",
    "BETA_SETTLE_CAPTURE_RETURN",
    "BETA_SETTLE_CALCULATED",
    "BETA_SETTLE_SQL_BEGIN",
    "BETA_SETTLE_SQL_REPLY",
    "BETA_SETTLE_COMMIT_RETURN",
    "BETA_VALUE_CAPTURE_BEGIN",
    "BETA_VALUE_CAPTURE_RETURN",
    "BETA_VALUE_CALCULATED",
    "BETA_VALUE_SQL_BEGIN",
    "BETA_VALUE_SQL_REPLY",
    "BETA_VALUE_COMMIT_RETURN",
    "BETA_LATEST_CAPTURE_BEGIN",
    "BETA_LATEST_CAPTURE_RETURN",
    "BETA_AUTO_QUEUE_RETURN",
    "BETA_CLAIM_CAPTURE_BEGIN",
    "BETA_CLAIM_CAPTURE_RETURN",
    "BETA_PUBLISH_REPLY",
    "BETA_PRECOMMIT_CLAIM_LOST",
    "BETA_PRECOMMIT_CONTEXT_REFUSED",
    "BETA_PRECOMMIT_DEADLINE",
    "BETA_COMMIT_RUNTIME_REFUSED",
    "BETA_COMMIT_SQL_REFUSED",
    "BETA_COMMIT_INPUT_REFUSED",
    "BETA_COMMIT_STATE_REFUSED",
    "BETA_COMMIT_CLEANUP_REFUSED",
    "BETA_COMMIT_UNKNOWN_REFUSED",
];

fn lifecycle_progress_stage(code: &str) -> Option<ProgressStage> {
    match code {
        "BETA_STAGE_OK" => Some(ProgressStage::BetaStageOk),
        "BETA_STAGE_TIMED_OUT" => Some(ProgressStage::BetaStageTimedOut),
        "BETA_STAGE_CANCELED" => Some(ProgressStage::BetaStageCanceled),
        "BETA_STAGE_FAILED_RUNTIME" => Some(ProgressStage::BetaStageFailedRuntime),
        "BETA_STAGE_FAILED_CAPTURE" => Some(ProgressStage::BetaStageFailedCapture),
        "BETA_STAGE_FAILED_SQL" => Some(ProgressStage::BetaStageFailedSql),
        "BETA_STAGE_FAILED_SNAPSHOT" => Some(ProgressStage::BetaStageFailedSnapshot),
        "BETA_STAGE_FAILED_CALCULATION" => Some(ProgressStage::BetaStageFailedCalculation),
        "BETA_STAGE_FAILED_RETRY" => Some(ProgressStage::BetaStageFailedRetry),
        "BETA_STAGE_FAILED_LEASE_LOST" => Some(ProgressStage::BetaStageFailedLeaseLost),
        "BETA_STAGE_FAILED_COMMIT_UNKNOWN" => Some(ProgressStage::BetaStageFailedCommitUnknown),
        "BETA_BOUND_BEGIN" => Some(ProgressStage::BetaBoundBegin),
        "BETA_BOUND_RETURN" => Some(ProgressStage::BetaBoundReturn),
        "BETA_CALLBACK_BEGIN" => Some(ProgressStage::BetaCallbackBegin),
        "BETA_CALLBACK_RETURN" => Some(ProgressStage::BetaCallbackReturn),
        "BETA_DUE_BEGIN" => Some(ProgressStage::BetaDueBegin),
        "BETA_DUE_RETURN" => Some(ProgressStage::BetaDueReturn),
        "BETA_SETTLE_CAPTURE_BEGIN" => Some(ProgressStage::BetaSettleCaptureBegin),
        "BETA_SETTLE_CAPTURE_RETURN" => Some(ProgressStage::BetaSettleCaptureReturn),
        "BETA_SETTLE_CALCULATED" => Some(ProgressStage::BetaSettleCalculated),
        "BETA_SETTLE_SQL_BEGIN" => Some(ProgressStage::BetaSettleSqlBegin),
        "BETA_SETTLE_SQL_REPLY" => Some(ProgressStage::BetaSettleSqlReply),
        "BETA_SETTLE_COMMIT_RETURN" => Some(ProgressStage::BetaSettleCommitReturn),
        "BETA_VALUE_CAPTURE_BEGIN" => Some(ProgressStage::BetaValueCaptureBegin),
        "BETA_VALUE_CAPTURE_RETURN" => Some(ProgressStage::BetaValueCaptureReturn),
        "BETA_VALUE_CALCULATED" => Some(ProgressStage::BetaValueCalculated),
        "BETA_VALUE_SQL_BEGIN" => Some(ProgressStage::BetaValueSqlBegin),
        "BETA_VALUE_SQL_REPLY" => Some(ProgressStage::BetaValueSqlReply),
        "BETA_VALUE_COMMIT_RETURN" => Some(ProgressStage::BetaValueCommitReturn),
        "BETA_LATEST_CAPTURE_BEGIN" => Some(ProgressStage::BetaLatestCaptureBegin),
        "BETA_LATEST_CAPTURE_RETURN" => Some(ProgressStage::BetaLatestCaptureReturn),
        "BETA_AUTO_QUEUE_RETURN" => Some(ProgressStage::BetaAutoQueueReturn),
        "BETA_CLAIM_CAPTURE_BEGIN" => Some(ProgressStage::BetaClaimCaptureBegin),
        "BETA_CLAIM_CAPTURE_RETURN" => Some(ProgressStage::BetaClaimCaptureReturn),
        "BETA_PUBLISH_REPLY" => Some(ProgressStage::BetaPublishReply),
        "BETA_PRECOMMIT_CLAIM_LOST" => Some(ProgressStage::BetaPrecommitClaimLost),
        "BETA_PRECOMMIT_CONTEXT_REFUSED" => Some(ProgressStage::BetaPrecommitContextRefused),
        "BETA_PRECOMMIT_DEADLINE" => Some(ProgressStage::BetaPrecommitDeadline),
        "BETA_COMMIT_RUNTIME_REFUSED" => Some(ProgressStage::BetaCommitRuntimeRefused),
        "BETA_COMMIT_SQL_REFUSED" => Some(ProgressStage::BetaCommitSqlRefused),
        "BETA_COMMIT_INPUT_REFUSED" => Some(ProgressStage::BetaCommitInputRefused),
        "BETA_COMMIT_STATE_REFUSED" => Some(ProgressStage::BetaCommitStateRefused),
        "BETA_COMMIT_CLEANUP_REFUSED" => Some(ProgressStage::BetaCommitCleanupRefused),
        "BETA_COMMIT_UNKNOWN_REFUSED" => Some(ProgressStage::BetaCommitUnknownRefused),
        _ => None,
    }
}

#[derive(Default)]
struct LifecycleVisitor {
    stage: Option<ProgressStage>,
    invalid: bool,
}

impl tracing::field::Visit for LifecycleVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() != "stage" || self.stage.is_some() {
            self.invalid = true;
            return;
        }
        self.stage = lifecycle_progress_stage(value);
        self.invalid |= self.stage.is_none();
    }

    fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {
        self.invalid = true;
    }
}

struct LifecycleSubscriber {
    emitted: AtomicUsize,
    beta_outcomes: AtomicUsize,
    emit: Arc<dyn Fn(ProgressStage) + Send + Sync>,
}

impl LifecycleSubscriber {
    fn new(emit: Arc<dyn Fn(ProgressStage) + Send + Sync>) -> Self {
        Self {
            emitted: AtomicUsize::new(0),
            beta_outcomes: AtomicUsize::new(0),
            emit,
        }
    }

    fn accepts(metadata: &tracing::Metadata<'_>) -> bool {
        metadata.is_event()
            && metadata.target() == LIFECYCLE_TARGET
            && *metadata.level() == tracing::Level::DEBUG
            && metadata.fields().iter().count() == 1
            && metadata.fields().field("stage").is_some()
    }
}

impl tracing::Subscriber for LifecycleSubscriber {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        Self::accepts(metadata)
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        if !Self::accepts(event.metadata()) {
            return;
        }
        let mut visitor = LifecycleVisitor::default();
        event.record(&mut visitor);
        if visitor.invalid {
            return;
        }
        if let Some(stage) = visitor.stage {
            // Reserve a separately bounded channel for the beta outcome so
            // earlier engine progress cannot erase the failure-chain witness.
            let is_outcome = BETA_OUTCOME_CODES
                .iter()
                .any(|code| lifecycle_progress_stage(code) == Some(stage));
            let (counter, cap) = if is_outcome {
                (&self.beta_outcomes, MAX_BETA_OUTCOMES)
            } else {
                (&self.emitted, MAX_ENGINE_EVENTS)
            };
            if counter
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                    (count < cap).then_some(count + 1)
                })
                .is_ok()
            {
                (self.emit)(stage);
            }
        }
    }
}

#[cfg(test)]
mod lifecycle_diagnostic_tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn worker_actor_scope_accepts_unset_or_empty_and_rejects_foreign_values() {
        assert!(worker_actor_is_unseeded(None));
        assert!(worker_actor_is_unseeded(Some("")));
        for actor in [" ", "foreign-owner", "00000000-0000-0000-0000-000000000001"] {
            assert!(!worker_actor_is_unseeded(Some(actor)));
        }
    }

    #[test]
    fn subscriber_accepts_only_closed_stage_events() {
        let observed = Arc::new(Mutex::new(Vec::new()));
        let sink = observed.clone();
        let subscriber = LifecycleSubscriber::new(Arc::new(move |stage| {
            sink.lock().expect("ordinary observation lock").push(stage);
        }));
        tracing::subscriber::with_default(subscriber, || {
            tracing::debug!(target: LIFECYCLE_TARGET, stage = "SENTINEL_STAGE");
            tracing::debug!(target: "SENTINEL_TARGET", stage = "BETA_BOUND_BEGIN");
            tracing::debug!(target: LIFECYCLE_TARGET, stage = "BETA_BOUND_BEGIN", body = "SENTINEL_BODY");
            tracing::debug!(target: LIFECYCLE_TARGET, stage = 7_u64);
            tracing::debug!(target: LIFECYCLE_TARGET, stage = ?"BETA_BOUND_BEGIN");
            tracing::info!(target: LIFECYCLE_TARGET, stage = "BETA_BOUND_BEGIN");
            assert!(
                observed
                    .lock()
                    .expect("ordinary observation lock")
                    .is_empty()
            );
            for code in LIFECYCLE_CODES {
                tracing::debug!(target: LIFECYCLE_TARGET, stage = *code);
            }
            tracing::debug!(target: LIFECYCLE_TARGET, stage = "BETA_BOUND_BEGIN");
        });
        let observed = observed.lock().expect("ordinary observation lock");
        assert!(observed.len() == MAX_ENGINE_EVENTS);
        for (actual, code) in observed.iter().zip(LIFECYCLE_CODES) {
            assert!(Some(*actual) == lifecycle_progress_stage(code));
        }
        let encoded = serde_json::to_string(&*observed).expect("ordinary closed stage encoding");
        assert!(!encoded.contains("SENTINEL"));
    }

    #[test]
    fn subscriber_keeps_closed_commit_refusals_without_payloads() {
        let observed = Arc::new(Mutex::new(Vec::new()));
        let sink = observed.clone();
        let subscriber = LifecycleSubscriber::new(Arc::new(move |stage| {
            sink.lock().expect("ordinary lock").push(stage);
        }));
        tracing::subscriber::with_default(subscriber, || {
            tracing::debug!(target: LIFECYCLE_TARGET, stage = "BETA_PRECOMMIT_CLAIM_LOST", body = "SENTINEL_SECRET");
            tracing::debug!(target: LIFECYCLE_TARGET, stage = "BETA_PRECOMMIT_CLAIM_LOST");
            tracing::debug!(target: LIFECYCLE_TARGET, stage = "BETA_PRECOMMIT_CONTEXT_REFUSED");
            tracing::debug!(target: LIFECYCLE_TARGET, stage = "BETA_PRECOMMIT_DEADLINE");
            tracing::debug!(target: LIFECYCLE_TARGET, stage = "BETA_COMMIT_RUNTIME_REFUSED");
        });
        let rows = observed.lock().expect("ordinary lock");
        assert_eq!(rows.len(), 4);
        assert!(rows[0] == ProgressStage::BetaPrecommitClaimLost);
        assert!(rows[1] == ProgressStage::BetaPrecommitContextRefused);
        assert!(rows[2] == ProgressStage::BetaPrecommitDeadline);
        assert!(rows[3] == ProgressStage::BetaCommitRuntimeRefused);
        assert!(
            !serde_json::to_string(&*rows)
                .expect("closed enum")
                .contains("SENTINEL")
        );
    }

    #[test]
    fn beta_outcome_survives_exhausted_engine_progress_budget() {
        let observed = Arc::new(Mutex::new(Vec::new()));
        let sink = observed.clone();
        let subscriber = LifecycleSubscriber::new(Arc::new(move |stage| {
            sink.lock().expect("ordinary lock").push(stage);
        }));
        tracing::subscriber::with_default(subscriber, || {
            for _ in 0..MAX_ENGINE_EVENTS + 1 {
                tracing::debug!(target: LIFECYCLE_TARGET, stage = "BETA_BOUND_BEGIN");
            }
            tracing::debug!(target: LIFECYCLE_TARGET, stage = "BETA_STAGE_TIMED_OUT", body = "SENTINEL_SECRET");
            tracing::debug!(target: LIFECYCLE_TARGET, stage = "BETA_STAGE_TIMED_OUT");
            for _ in 0..MAX_BETA_OUTCOMES {
                tracing::debug!(target: LIFECYCLE_TARGET, stage = "BETA_STAGE_FAILED_COMMIT_UNKNOWN");
            }
        });
        let rows = observed.lock().expect("ordinary lock");
        assert_eq!(rows.len(), MAX_ENGINE_EVENTS + MAX_BETA_OUTCOMES);
        assert!(rows[MAX_ENGINE_EVENTS] == ProgressStage::BetaStageTimedOut);
        assert!(
            !serde_json::to_string(&*rows)
                .expect("closed stages")
                .contains("SENTINEL")
        );
    }

    #[test]
    fn cycle_error_classification_never_serializes_error_payloads() {
        use api_server::paper_runner::RunnerError;
        let cases = [
            (
                RunnerError::Deadline {
                    operation: "preview",
                    timeout: Duration::from_secs(90),
                },
                ProgressStage::CycleDeadlinePreview,
            ),
            (
                RunnerError::Deadline {
                    operation: "SENTINEL_SECRET",
                    timeout: Duration::from_secs(90),
                },
                ProgressStage::CycleDeadlineOther,
            ),
            (RunnerError::Shutdown, ProgressStage::CycleShutdown),
            (
                RunnerError::InvalidDate("SENTINEL_SECRET".to_owned()),
                ProgressStage::CycleInvalidDate,
            ),
            (
                RunnerError::AccountScan(sqlx::Error::Protocol("SENTINEL_SECRET".to_owned())),
                ProgressStage::CycleAccountScanRejected,
            ),
        ];
        for (error, expected) in cases {
            let stage = cycle_error_stage(&error);
            assert!(stage == expected);
            let encoded = serde_json::to_string(&stage).expect("closed cycle error class");
            assert!(!encoded.contains("SENTINEL"));
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn actual_cycle_error_survives_fixture_cleanup() {
        use api_server::paper_runner::RunnerError;
        let cleaned = Cell::new(false);
        let original = std::panic::AssertUnwindSafe(async {
            fail_worker_cycle(RunnerError::AccountScan(sqlx::Error::Protocol(
                "SENTINEL_SECRET".to_owned(),
            )));
        })
        .catch_unwind()
        .await;
        let observed = std::panic::AssertUnwindSafe(finish_fixture_phase(original, async {
            cleaned.set(true);
        }))
        .catch_unwind()
        .await;
        assert!(cleaned.get());
        let error = observed
            .expect_err("original typed failure is resumed")
            .downcast::<RunnerError>()
            .expect("original RunnerError retained");
        assert!(
            matches!(*error, RunnerError::AccountScan(sqlx::Error::Protocol(ref message)) if message == "SENTINEL_SECRET")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cleanup_runs_before_original_panic_is_resumed() {
        for (body_panics, cleanup_panics) in
            [(true, false), (true, true), (false, true), (false, false)]
        {
            let body_payload = Arc::new(());
            let cleanup_payload = Arc::new(());
            let order = Mutex::new(Vec::new());
            let original = std::panic::AssertUnwindSafe(async {
                order.lock().expect("ordinary order lock").push("body");
                if body_panics {
                    std::panic::panic_any(body_payload.clone());
                }
            })
            .catch_unwind()
            .await;
            let result = std::panic::AssertUnwindSafe(finish_fixture_phase(original, async {
                order.lock().expect("ordinary order lock").push("cleanup");
                if cleanup_panics {
                    std::panic::panic_any(cleanup_payload.clone());
                }
            }))
            .catch_unwind()
            .await;
            order.lock().expect("ordinary order lock").push("observed");
            match result {
                Ok(()) => assert!(!body_panics && !cleanup_panics),
                Err(payload) => {
                    let payload = payload
                        .downcast::<Arc<()>>()
                        .expect("original ordinary payload type");
                    let expected = if body_panics {
                        &body_payload
                    } else {
                        &cleanup_payload
                    };
                    assert!(Arc::ptr_eq(&payload, expected));
                }
            }
            assert!(*order.lock().expect("ordinary order lock") == ["body", "cleanup", "observed"]);
        }
    }
}

fn install_lifecycle_subscriber() {
    tracing::subscriber::set_global_default(LifecycleSubscriber::new(Arc::new(progress)))
        .unwrap_or_else(|_| panic!("fixture lifecycle subscriber installation failed"));
}

async fn finish_fixture_phase<F>(original: std::thread::Result<()>, cleanup: F)
where
    F: std::future::Future<Output = ()>,
{
    let later = std::panic::AssertUnwindSafe(cleanup).catch_unwind().await;
    if let Err(payload) = original {
        std::panic::resume_unwind(payload);
    }
    if let Err(payload) = later {
        std::panic::resume_unwind(payload);
    }
}

struct FixturePools {
    app: PgPool,
    admin: PgPool,
    audit: PgPool,
    worker: PgPool,
    queue_worker: PgPool,
    migration_owner: PgPool,
    inspection: PgPool,
}

impl FixturePools {
    async fn open() -> Self {
        Self {
            app: pool("R5_PAPER_APP_URL").await,
            admin: pool("R5_PAPER_ADMIN_URL").await,
            audit: pool("R5_PAPER_AUDIT_URL").await,
            worker: pool_with_max("R5_PAPER_WORKER_URL", 1).await,
            queue_worker: queue_worker_pool().await,
            migration_owner: pool("R5_PAPER_MIGRATION_OWNER_URL").await,
            inspection: inspection_pool().await,
        }
    }

    async fn close(self) {
        progress(ProgressStage::PoolsCloseBegin);
        progress(ProgressStage::AppPoolCloseBegin);
        self.app.close().await;
        progress(ProgressStage::AppPoolCloseEnd);
        progress(ProgressStage::AdminPoolCloseBegin);
        self.admin.close().await;
        progress(ProgressStage::AdminPoolCloseEnd);
        progress(ProgressStage::AuditPoolCloseBegin);
        self.audit.close().await;
        progress(ProgressStage::AuditPoolCloseEnd);
        progress(ProgressStage::WorkerPoolCloseBegin);
        self.queue_worker.close().await;
        self.worker.close().await;
        progress(ProgressStage::WorkerPoolCloseEnd);
        progress(ProgressStage::MigrationPoolCloseBegin);
        self.migration_owner.close().await;
        progress(ProgressStage::MigrationPoolCloseEnd);
        progress(ProgressStage::InspectionPoolCloseBegin);
        self.inspection.close().await;
        progress(ProgressStage::InspectionPoolCloseEnd);
        progress(ProgressStage::PoolsCloseEnd);
    }
}

struct RcSqlConsumer(Rc<Cell<bool>>);

impl PaperFinancialConsumer<(), &'static str> for RcSqlConsumer {
    fn run<'a, 'hold>(
        self,
        action: &'a mut CheckedPaperAction<'hold>,
    ) -> LocalInputFuture<'a, Result<(), &'static str>>
    where
        Self: 'a,
        'hold: 'a,
    {
        Box::pin(async move {
            action.begin_transaction().await.map_err(|_| "begin")?;
            let result = {
                let transaction = action.transaction().map_err(|_| "transaction")?;
                sqlx::query_scalar::<_, i32>("SELECT 1")
                    .fetch_one(&mut **transaction)
                    .await
                    .map_err(|_| "query")?
            };
            self.0.set(result == 1);
            action.rollback().await.map_err(|_| "rollback")?;
            if self.0.get() {
                Ok(())
            } else {
                Err("unexpected SQL result")
            }
        })
    }
}

struct LocalExecutorSqlProbe {
    pool: PgPool,
}

impl PaperActionWork<bool, &'static str> for LocalExecutorSqlProbe {
    fn run<'ctx, 'scope, 'attempt>(
        self: Box<Self>,
        context: &'ctx job_queue::owner_beta::paper::action::PaperActionContext<
            'ctx,
            'scope,
            'attempt,
        >,
    ) -> PaperActionFuture<'ctx, bool, &'static str> {
        Box::pin(async move {
            let observed = Rc::new(Cell::new(false));
            let outcome = with_checked_paper_action(
                &self.pool,
                context,
                &PaperActionInputRequest::LatestSignal,
                RcSqlConsumer(Rc::clone(&observed)),
            )
            .await;
            match outcome.into_result() {
                Ok(()) => Ok(observed.get()),
                Err(failure) => Err(failure
                    .into_original()
                    .and_then(Result::err)
                    .unwrap_or("checked action refused")),
            }
        })
    }
}

#[derive(Clone, Copy, Debug)]
struct CheckedActionProbe {
    consumer_entered: bool,
    committed: bool,
    commit_started: bool,
    first_error: Option<PaperInputActionError>,
    secondary_error_count: usize,
    rollback_succeeded: bool,
}

struct AdmissionReadbackWork {
    pool: PgPool,
    date: chrono::NaiveDate,
}

struct OldTargetReadbackWork {
    pool: PgPool,
    target: Uuid,
    process_date: chrono::NaiveDate,
}
impl PaperActionWork<Value, &'static str> for OldTargetReadbackWork {
    fn run<'ctx, 'scope, 'attempt>(
        self: Box<Self>,
        context: &'ctx job_queue::owner_beta::paper::action::PaperActionContext<
            'ctx,
            'scope,
            'attempt,
        >,
    ) -> PaperActionFuture<'ctx, Value, &'static str> {
        Box::pin(async move {
            context.revalidate().map_err(|_| "old target currentness")?;
            let target: Value =
                sqlx::query_scalar("SELECT public.snapshot_owner_beta_paper_target_v1($1,$2,$3)")
                    .bind(self.target)
                    .bind(context.runtime().control_epoch())
                    .bind(context.runtime().config_sha256())
                    .fetch_one(&self.pool)
                    .await
                    .map_err(|_| "old target actual snapshot")?;
            let start = self
                .process_date
                .checked_sub_days(chrono::Days::new(369))
                .ok_or("calendar range")?;
            let evidence: Vec<Value> = sqlx::query_scalar(
                "SELECT evidence FROM public.read_owner_beta_daily_source_evidence($1,$2)",
            )
            .bind(start)
            .bind(self.process_date)
            .fetch_all(&self.pool)
            .await
            .map_err(|_| "actual calendar readback")?;
            context
                .revalidate()
                .map_err(|_| "old target final currentness")?;
            Ok(serde_json::json!({"target":target,"calendar":evidence}))
        })
    }
}

async fn old_target_witness(
    pools: &FixturePools,
) -> (chrono::NaiveDate, chrono::NaiveDate, Uuid, bool) {
    let process = chrono::NaiveDate::parse_from_str(&env("R5_PAPER_PROCESS_DATE"), "%Y-%m-%d")
        .expect("current process date");
    let effective =
        chrono::NaiveDate::parse_from_str(&env("R5_PAPER_TARGET_EFFECTIVE_SESSION"), "%Y-%m-%d")
            .expect("explicit old target E");
    let target = Uuid::parse_str(&env("R5_PAPER_TARGET_ID")).expect("explicit real old target ID");
    let today = chrono::Utc::now()
        .with_timezone(&chrono::FixedOffset::east_opt(9 * 60 * 60).unwrap())
        .date_naive();
    assert_eq!(
        process, today,
        "process date must be CURRENT; never rewind to E"
    );
    assert!(effective < process);
    let actual = run_bound_paper_action(
        PaperProcessRole::Worker,
        Duration::from_secs(90),
        None,
        OldTargetReadbackWork {
            pool: pools.worker.clone(),
            target,
            process_date: process,
        },
    )
    .await
    .unwrap_or_else(|_| panic!("old target actual protected readback unavailable"));
    let snapshot = &actual["target"]["data"];
    assert_eq!(snapshot["pending_target_id"], target.to_string());
    assert_eq!(snapshot["effective_session"], effective.to_string());
    assert_eq!(snapshot["account_id"], env("R5_PAPER_ACCOUNT_ID"));
    assert_eq!(snapshot["owner_user_id"], env("R5_PAPER_OWNER_USER_ID"));
    assert!(
        snapshot["valuation_sha256"].is_null(),
        "old-E phase starts before remaining valuation"
    );
    let mut days = std::collections::BTreeMap::new();
    for row in actual["calendar"]
        .as_array()
        .expect("actual calendar evidence")
    {
        if row["kind"] != "calendar" {
            continue;
        }
        let date = chrono::NaiveDate::parse_from_str(
            row["session_date"].as_str().expect("calendar date"),
            "%Y-%m-%d",
        )
        .unwrap();
        assert!(date <= process);
        let kind = row["session_type"]
            .as_str()
            .expect("calendar session type")
            .to_owned();
        assert!(
            row["calendar_evidence_sha256"].as_str().is_some(),
            "real calendar-source witness required"
        );
        if let Some(old) = days.insert(date, kind.clone()) {
            assert_eq!(old, kind, "conflicting calendar witnesses");
        }
    }
    let latest: Vec<_> = days
        .iter()
        .rev()
        .filter(|(_, kind)| *kind == "TRADING")
        .take(3)
        .map(|(date, _)| *date)
        .collect();
    assert_eq!(latest.len(), 3);
    assert!(
        latest.iter().all(|day| *day > effective),
        "E must be outside actual witnessed latest3"
    );
    (
        process,
        effective,
        target,
        !snapshot["settlement_sha256"].is_null(),
    )
}

impl PaperActionWork<Value, &'static str> for AdmissionReadbackWork {
    fn run<'ctx, 'scope, 'attempt>(
        self: Box<Self>,
        context: &'ctx job_queue::owner_beta::paper::action::PaperActionContext<
            'ctx,
            'scope,
            'attempt,
        >,
    ) -> PaperActionFuture<'ctx, Value, &'static str> {
        Box::pin(async move {
            context
                .revalidate()
                .map_err(|_| "current admission scope")?;
            let mut transaction = self
                .pool
                .begin()
                .await
                .map_err(|_| "admission read begin")?;
            // Rollback is awaited even when setup or the actual wire read fails.
            let original = async {
                job_queue::paper_execution::set_paper_transaction_timeouts(&mut transaction)
                    .await
                    .map_err(|_| "admission read timeouts")?;
                sqlx::query_scalar::<_, Value>(
                    "SELECT public.read_owner_beta_paper_session_evidence($1,$2)",
                )
                .bind(self.date)
                .bind(&context.binding()["cohort"])
                .fetch_one(&mut *transaction)
                .await
                .map_err(|_| "actual admission readback")
            }
            .await;
            let rollback = transaction.rollback().await;
            let value = original?;
            rollback.map_err(|_| "admission read rollback")?;
            context
                .revalidate()
                .map_err(|_| "current admission postcheck")?;
            Ok(value)
        })
    }
}

struct SqlWaitConsumer {
    backend_pid: Option<oneshot::Sender<i32>>,
    consumer_entered: Arc<AtomicBool>,
    rollback_succeeded: Arc<AtomicBool>,
    local_callback_wait: bool,
}

impl PaperFinancialConsumer<(), &'static str> for SqlWaitConsumer {
    fn run<'a, 'hold>(
        mut self,
        action: &'a mut CheckedPaperAction<'hold>,
    ) -> LocalInputFuture<'a, Result<(), &'static str>>
    where
        Self: 'a,
        'hold: 'a,
    {
        Box::pin(async move {
            action.begin_transaction().await.map_err(|_| "begin")?;
            let backend_pid = {
                let transaction = action.transaction().map_err(|_| "transaction")?;
                sqlx::query_scalar::<_, i32>("SELECT pg_backend_pid()")
                    .fetch_one(&mut **transaction)
                    .await
            };
            let backend_pid = match backend_pid {
                Ok(backend_pid) => backend_pid,
                Err(error) => {
                    action.retain_sql_error(error);
                    self.rollback_succeeded
                        .store(action.rollback().await.is_ok(), Ordering::Release);
                    return Err("backend pid query");
                }
            };
            self.consumer_entered.store(true, Ordering::Release);
            if let Some(sender) = self.backend_pid.take() {
                let _ = sender.send(backend_pid);
            }

            loop {
                if let Err(error) = action.revalidate_files() {
                    // Only the closed public refusal enum is observed; never a retained payload.
                    eprintln!(
                        "R5_INTEG_SQL_WAIT_BOUNDARY_{}",
                        format!("{error:?}")
                            .to_ascii_uppercase()
                            .replace(['(', ')', ' '], "_")
                    );
                    self.rollback_succeeded
                        .store(action.rollback().await.is_ok(), Ordering::Release);
                    return Err("action boundary");
                }
                let query_result = {
                    let transaction = action.transaction().map_err(|_| "transaction")?;
                    if self.local_callback_wait {
                        sqlx::query("SELECT 1").execute(&mut **transaction).await
                    } else {
                        sqlx::query("SELECT pg_sleep($1::double precision)")
                            .bind(1.0_f64)
                            .execute(&mut **transaction)
                            .await
                    }
                };
                if let Err(error) = query_result {
                    // Retain the actual error below; export only a validated SQLSTATE witness.
                    if let Some(code) = error
                        .as_database_error()
                        .and_then(|value| value.code())
                        .filter(|value| {
                            value.len() == 5
                                && value
                                    .bytes()
                                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
                        })
                    {
                        eprintln!("R5_INTEG_SQL_WAIT_SQLSTATE_{code}");
                    } else {
                        eprintln!("R5_INTEG_SQL_WAIT_SQL_ERROR_NON_DATABASE");
                    }
                    action.retain_sql_error(error);
                    if action.revalidate_files().is_err() {
                        self.rollback_succeeded
                            .store(action.rollback().await.is_ok(), Ordering::Release);
                        return Err("action boundary");
                    }
                    self.rollback_succeeded
                        .store(action.rollback().await.is_ok(), Ordering::Release);
                    return Err("SQL wait");
                }
                if self.local_callback_wait {
                    // Keep the real transaction and borrowed input readers alive while
                    // the original outer deadline expires, without a long SQL statement.
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        })
    }
}

struct SqlWaitActionWork {
    pool: PgPool,
    backend_pid: oneshot::Sender<i32>,
    completion: oneshot::Sender<CheckedActionProbe>,
    consumer_entered: Arc<AtomicBool>,
    rollback_succeeded: Arc<AtomicBool>,
    local_callback_wait: bool,
}

impl PaperActionWork<CheckedActionProbe, &'static str> for SqlWaitActionWork {
    fn run<'ctx, 'scope, 'attempt>(
        self: Box<Self>,
        context: &'ctx job_queue::owner_beta::paper::action::PaperActionContext<
            'ctx,
            'scope,
            'attempt,
        >,
    ) -> PaperActionFuture<'ctx, CheckedActionProbe, &'static str> {
        let Self {
            pool,
            backend_pid,
            completion,
            consumer_entered,
            rollback_succeeded,
            local_callback_wait,
        } = *self;
        Box::pin(async move {
            let outcome = with_checked_paper_action(
                &pool,
                context,
                &PaperActionInputRequest::LatestSignal,
                SqlWaitConsumer {
                    backend_pid: Some(backend_pid),
                    consumer_entered: Arc::clone(&consumer_entered),
                    rollback_succeeded: Arc::clone(&rollback_succeeded),
                    local_callback_wait,
                },
            )
            .await;
            let committed = outcome.committed();
            let commit_started = outcome.commit_started();
            let failure = outcome.into_result().err();
            let first_error = failure.as_ref().and_then(|failure| failure.first_error());
            let secondary_error_count = failure
                .as_ref()
                .map_or(0, |failure| failure.secondary_error_count());
            let probe = CheckedActionProbe {
                consumer_entered: consumer_entered.load(Ordering::Acquire),
                committed,
                commit_started,
                first_error,
                secondary_error_count,
                rollback_succeeded: rollback_succeeded.load(Ordering::Acquire),
            };
            if let Some(error) = probe.first_error {
                eprintln!(
                    "R5_INTEG_SQL_WAIT_FINAL_{}",
                    format!("{error:?}")
                        .to_ascii_uppercase()
                        .replace(['(', ')', ' '], "_")
                );
            }
            let _ = completion.send(probe);
            Ok(probe)
        })
    }
}

struct CommitWindowConsumer {
    backend_pid: Option<oneshot::Sender<i32>>,
    marker_insert: String,
    consumer_entered: Arc<AtomicBool>,
}

impl PaperFinancialConsumer<(), &'static str> for CommitWindowConsumer {
    fn run<'a, 'hold>(
        mut self,
        action: &'a mut CheckedPaperAction<'hold>,
    ) -> LocalInputFuture<'a, Result<(), &'static str>>
    where
        Self: 'a,
        'hold: 'a,
    {
        Box::pin(async move {
            action.begin_transaction().await.map_err(|_| "begin")?;
            let insertion = {
                let transaction = action.transaction().map_err(|_| "transaction")?;
                sqlx::query(sqlx::AssertSqlSafe(self.marker_insert.clone()))
                    .execute(&mut **transaction)
                    .await
            };
            if let Err(error) = insertion {
                action.retain_sql_error(error);
                let _ = action.rollback().await;
                return Err("private commit marker insert");
            }
            let pid = {
                let transaction = action.transaction().map_err(|_| "transaction")?;
                sqlx::query_scalar::<_, i32>("SELECT pg_backend_pid()")
                    .fetch_one(&mut **transaction)
                    .await
            };
            let pid = match pid {
                Ok(pid) => pid,
                Err(error) => {
                    action.retain_sql_error(error);
                    let _ = action.rollback().await;
                    return Err("commit marker backend");
                }
            };
            self.consumer_entered.store(true, Ordering::Release);
            if let Some(sender) = self.backend_pid.take() {
                let _ = sender.send(pid);
            }
            // The deferred FK blocks the server's COMMIT, after the real final
            // cancellation veto. No production commit hook is replaced.
            action
                .commit(&mut || Ok(()))
                .await
                .map_err(|_| "commit boundary")
        })
    }
}

struct CommitWindowActionWork {
    pool: PgPool,
    backend_pid: oneshot::Sender<i32>,
    marker_insert: String,
    completion: oneshot::Sender<CheckedActionProbe>,
    consumer_entered: Arc<AtomicBool>,
}

impl PaperActionWork<CheckedActionProbe, &'static str> for CommitWindowActionWork {
    fn run<'ctx, 'scope, 'attempt>(
        self: Box<Self>,
        context: &'ctx job_queue::owner_beta::paper::action::PaperActionContext<
            'ctx,
            'scope,
            'attempt,
        >,
    ) -> PaperActionFuture<'ctx, CheckedActionProbe, &'static str> {
        let Self {
            pool,
            backend_pid,
            marker_insert,
            completion,
            consumer_entered,
        } = *self;
        Box::pin(async move {
            let outcome = with_checked_paper_action(
                &pool,
                context,
                &PaperActionInputRequest::LatestSignal,
                CommitWindowConsumer {
                    backend_pid: Some(backend_pid),
                    marker_insert,
                    consumer_entered: Arc::clone(&consumer_entered),
                },
            )
            .await;
            let committed = outcome.committed();
            let commit_started = outcome.commit_started();
            let failure = outcome.into_result().err();
            let first_error = failure.as_ref().and_then(|failure| failure.first_error());
            let secondary_error_count = failure
                .as_ref()
                .map_or(0, |failure| failure.secondary_error_count());
            let probe = CheckedActionProbe {
                consumer_entered: consumer_entered.load(Ordering::Acquire),
                committed,
                commit_started,
                first_error,
                secondary_error_count,
                rollback_succeeded: false,
            };
            let _ = completion.send(probe);
            Ok(probe)
        })
    }
}

async fn wait_for_entered_action(
    pools: &FixturePools,
    backend_pid: i32,
    local_callback_wait: bool,
) {
    let wait = async {
        loop {
            let active: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1 AND usename=current_user \
                 AND (($2 AND state='idle in transaction' AND xact_start IS NOT NULL) \
                   OR (NOT $2 AND state='active' AND wait_event_type='Timeout' \
                       AND wait_event='PgSleep' AND query LIKE 'SELECT pg_sleep(%')))",
            )
            .bind(backend_pid)
            .bind(local_callback_wait)
            // The separate worker login can see its own role's session details.
            // migration_owner has no cross-role pg_stat_activity visibility.
            .fetch_one(&pools.queue_worker)
            .await
            .unwrap_or_else(|_| panic!("action SQL activity readback failed"));
            if active {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(90), wait)
        .await
        .unwrap_or_else(|_| panic!("checked action did not enter its selected resource wait"));
}

async fn checked_action_cancel_or_deadline_phase(pools: &FixturePools, deadline: bool) {
    let before = action_economic_state(pools).await;
    let (backend_pid_tx, backend_pid_rx) = oneshot::channel();
    let (completion_tx, completion_rx) = oneshot::channel();
    let consumer_entered = Arc::new(AtomicBool::new(false));
    let rollback_succeeded = Arc::new(AtomicBool::new(false));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let shutdown = (!deadline).then_some(shutdown_rx);
    let task = tokio::spawn(run_bound_paper_action(
        PaperProcessRole::Worker,
        Duration::from_secs(90),
        shutdown,
        SqlWaitActionWork {
            pool: pools.worker.clone(),
            backend_pid: backend_pid_tx,
            completion: completion_tx,
            consumer_entered: Arc::clone(&consumer_entered),
            rollback_succeeded: Arc::clone(&rollback_succeeded),
            local_callback_wait: deadline,
        },
    ));
    let backend_pid = tokio::time::timeout(Duration::from_secs(90), backend_pid_rx)
        .await
        .unwrap_or_else(|_| panic!("checked SQL consumer did not enter before its deadline"))
        .unwrap_or_else(|_| panic!("checked SQL consumer did not publish its backend"));
    wait_for_entered_action(pools, backend_pid, deadline).await;
    if !deadline {
        shutdown_tx
            .send(true)
            .unwrap_or_else(|_| panic!("checked action shutdown receiver disappeared"));
    }
    let error = task
        .await
        .unwrap_or_else(|_| panic!("checked action task panicked"))
        .expect_err("checked action unexpectedly completed cleanly");
    assert_eq!(error.kind(), BoundPaperActionErrorKind::CommitUnknown);
    assert_eq!(
        error.refusal(),
        Some(if deadline {
            PaperActionRefusal::DeadlineExceeded
        } else {
            PaperActionRefusal::Canceled
        })
    );

    let probe = tokio::time::timeout(Duration::from_secs(20), completion_rx)
        .await
        .unwrap_or_else(|_| panic!("checked action scope did not finish after cancellation"))
        .unwrap_or_else(|_| panic!("checked action scope completion was dropped"));
    assert!(probe.consumer_entered);
    assert!(!probe.committed);
    assert!(!probe.commit_started);
    assert!(probe.rollback_succeeded);
    assert_eq!(
        action_economic_state(pools).await,
        before,
        "an entered action canceled with held inputs must roll back without economics"
    );
    let connection = pools
        .worker
        .try_acquire()
        .unwrap_or_else(|| panic!("checked action did not release its worker connection"));
    drop(connection);
    let phase = if deadline {
        "checked_action_deadline_after_entry"
    } else {
        "checked_action_cancel_sql_wait"
    };
    eprintln!(
        "R5_PAPER_NATIVE phase={} classification=SYNTHETIC_TEST_AUTHORITY_NOT_ORIGIN actual_checked_consumer=true sql_wait_observed={} held_transaction_callback_wait={} outer_commit_unknown=true rollback_awaited=true economic_state_unchanged=true worker_connection_released=true",
        phase, !deadline, deadline
    );
}

async fn prepare_commit_probe(pools: &FixturePools) -> String {
    let nonce = env("R5_INTEG_NONCE");
    assert!(
        nonce.len() == 32
            && nonce
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    );
    // SQLx's owned-string audit is limited to fixed SQL plus this identifier:
    // a fixed prefix and exactly 32 lowercase ASCII hex bytes, never SQL data.
    let schema = format!("r5_commit_gate_{nonce}");
    let mut transaction = pools
        .migration_owner
        .begin()
        .await
        .unwrap_or_else(|_| panic!("private commit fixture setup begin failed"));
    let setup: Result<(), sqlx::Error> = async {
        for statement in [
            format!("CREATE SCHEMA {schema} AUTHORIZATION migration_owner"),
            format!("CREATE TABLE {schema}.parent(id integer PRIMARY KEY)"),
            format!("CREATE TABLE {schema}.marker(id integer PRIMARY KEY,parent_id integer NOT NULL,FOREIGN KEY(parent_id) REFERENCES {schema}.parent(id) DEFERRABLE INITIALLY DEFERRED)"),
            format!("INSERT INTO {schema}.parent(id) VALUES(1)"),
            format!("GRANT USAGE ON SCHEMA {schema} TO worker"),
            format!("GRANT SELECT ON {schema}.parent TO worker"),
            format!("GRANT INSERT ON {schema}.marker TO worker"),
        ] {
            sqlx::query(sqlx::AssertSqlSafe(statement)).execute(&mut *transaction).await?;
        }
        Ok(())
    }.await;
    if let Err(error) = setup {
        if transaction.rollback().await.is_err() {
            eprintln!("R5_PAPER_NATIVE_FIXTURE secondary=COMMIT_SETUP_ROLLBACK_FAILED");
        }
        std::panic::panic_any(error);
    }
    transaction
        .commit()
        .await
        .unwrap_or_else(|_| panic!("private commit fixture setup commit failed"));
    schema
}

async fn drop_commit_probe(pools: &FixturePools, schema: &str) {
    let owner: String =
        sqlx::query_scalar("SELECT pg_get_userbyid(nspowner) FROM pg_namespace WHERE nspname=$1")
            .bind(schema)
            .fetch_one(&pools.migration_owner)
            .await
            .unwrap_or_else(|_| panic!("private commit schema ownership unavailable"));
    assert_eq!(owner, "migration_owner");
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT c.relname::text FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace \
         WHERE n.nspname=$1 AND c.relkind IN ('r','p','v','m','f') ORDER BY c.relname",
    )
    .bind(schema)
    .fetch_all(&pools.migration_owner)
    .await
    .unwrap_or_else(|_| panic!("private commit schema inventory unavailable"));
    assert_eq!(tables, ["marker", "parent"]);
    let mut transaction = pools
        .migration_owner
        .begin()
        .await
        .unwrap_or_else(|_| panic!("private commit fixture cleanup begin failed"));
    let cleanup: Result<(), sqlx::Error> = async {
        for statement in [
            format!("DROP TABLE {schema}.marker"),
            format!("DROP TABLE {schema}.parent"),
            format!("DROP SCHEMA {schema}"),
        ] {
            sqlx::query(sqlx::AssertSqlSafe(statement))
                .execute(&mut *transaction)
                .await?;
        }
        Ok(())
    }
    .await;
    if let Err(error) = cleanup {
        if transaction.rollback().await.is_err() {
            eprintln!("R5_PAPER_NATIVE_FIXTURE secondary=COMMIT_CLEANUP_ROLLBACK_FAILED");
        }
        std::panic::panic_any(error);
    }
    transaction
        .commit()
        .await
        .unwrap_or_else(|_| panic!("private commit fixture cleanup commit failed"));
}

async fn wait_for_commit_lock(pools: &FixturePools, pid: i32) {
    let wait = async {
        loop {
            let started = std::time::Instant::now();
            let observed: Option<f64> = sqlx::query_scalar(
                "SELECT EXTRACT(EPOCH FROM (clock_timestamp()-query_start))::double precision \
                 FROM pg_stat_activity WHERE pid=$1 AND usename=current_user \
                 AND datname=current_database() AND state='active' AND wait_event_type='Lock' \
                 AND query LIKE 'COMMIT%'",
            )
            .bind(pid)
            .fetch_optional(&pools.queue_worker)
            .await
            .unwrap_or_else(|_| panic!("actual COMMIT lock observation failed"));
            if let Some(age) = observed {
                assert!(
                    (0.0..1.0).contains(&age) && started.elapsed() < Duration::from_millis(500),
                    "actual COMMIT observation must leave headroom under the unchanged five-second lock limit"
                );
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(90), wait)
        .await
        .unwrap_or_else(|_| panic!("checked consumer did not enter actual deferred COMMIT lock"));
}

async fn checked_action_dirty_postcommit_phase(pools: &FixturePools) {
    use sqlx::Connection as _;
    let before = action_economic_state(pools).await;
    let schema = prepare_commit_probe(pools).await;
    let original = std::panic::AssertUnwindSafe(async {
        let mut blocker_connection = pools.migration_owner.acquire().await
            .unwrap_or_else(|_| panic!("private commit blocker connection failed"));
        let mut blocker = blocker_connection.begin().await
            .unwrap_or_else(|_| panic!("private commit blocker begin failed"));
        let parent = sqlx::query_scalar::<_, i32>(sqlx::AssertSqlSafe(format!(
            "SELECT id FROM {schema}.parent WHERE id=1 FOR UPDATE",
        ))).fetch_one(&mut *blocker).await;
        if !matches!(parent, Ok(1)) {
            let released = blocker.rollback().await.is_ok();
            if released { drop(blocker_connection); }
            else { let _ = blocker_connection.close().await; }
            match parent {
                Err(error) => std::panic::panic_any(error),
                Ok(_) => panic!("private commit parent lock witness mismatch"),
            }
        }
        let (backend_pid_tx, backend_pid_rx) = oneshot::channel();
        let (completion_tx, completion_rx) = oneshot::channel();
        let consumer_entered = Arc::new(AtomicBool::new(false));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let mut task = tokio::spawn(run_bound_paper_action(
            PaperProcessRole::Worker, Duration::from_secs(90), Some(shutdown_rx),
            CommitWindowActionWork {
                pool: pools.worker.clone(), backend_pid: backend_pid_tx,
                marker_insert: format!("INSERT INTO {schema}.marker(id,parent_id) VALUES(1,1)"),
                completion: completion_tx, consumer_entered: Arc::clone(&consumer_entered),
            },
        ));
        let mut outer_observed = false;
        let stage = std::panic::AssertUnwindSafe(async {
            let pid = tokio::time::timeout(Duration::from_secs(90), backend_pid_rx).await
                .unwrap_or_else(|_| panic!("commit consumer did not enter"))
                .unwrap_or_else(|_| panic!("commit consumer backend was not published"));
            wait_for_commit_lock(pools, pid).await;
            shutdown_tx.send(true).expect("commit window shutdown receiver");
            // The real cancellation helper returns while entered blocking work
            // still owns its slot. Never hold this DB blocker while joining it.
            let result = tokio::time::timeout(Duration::from_secs(1), &mut task).await
                .unwrap_or_else(|_| panic!("outer cancellation exceeded short COMMIT release coordination"));
            outer_observed = true;
            result
        }).catch_unwind().await;
        // Always release the exact owned lock before assertions or dependent
        // joins, including a failed observation/send or an unwinding stage.
        let released = blocker.rollback().await.is_ok();
        if released { drop(blocker_connection); }
        else { let _ = blocker_connection.close().await; }
        if !outer_observed {
            let _ = shutdown_tx.send(true);
            let _ = task.await;
        }
        let response = match stage {
            Ok(response) => response,
            Err(payload) => std::panic::resume_unwind(payload),
        };
        let error = response.unwrap_or_else(|_| panic!("checked commit task panicked"))
            .expect_err("dirty postcommit action unexpectedly completed cleanly");
        assert!(released, "owned COMMIT blocker rollback failed");
        assert_eq!(error.kind(), BoundPaperActionErrorKind::CommitUnknown);
        assert_eq!(error.refusal(), Some(PaperActionRefusal::Canceled));
        let probe = tokio::time::timeout(Duration::from_secs(20), completion_rx).await
            .unwrap_or_else(|_| panic!("dirty postcommit checked scope did not finish"))
            .unwrap_or_else(|_| panic!("dirty postcommit completion dropped"));
        assert!(probe.consumer_entered && probe.commit_started && probe.committed,
            "actual successful COMMIT must survive canceled postcheck");
        assert_eq!(probe.first_error, Some(PaperInputActionError::Runtime));
        assert!(probe.secondary_error_count >= 1,
            "secondary commit uncertainty must remain retained");
        let markers: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {schema}.marker")))
            .fetch_one(&pools.migration_owner).await
            .unwrap_or_else(|_| panic!("actual committed private marker readback failed"));
        assert_eq!(markers, 1);
        assert_eq!(action_economic_state(pools).await, before,
            "private marker writes must not change canonical Paper economics");
        let connection = pools.worker.try_acquire()
            .unwrap_or_else(|| panic!("dirty commit action did not release worker connection"));
        drop(connection);
        eprintln!("R5_PAPER_NATIVE phase=checked_action_dirty_postcommit_unknown classification=SYNTHETIC_TEST_AUTHORITY_NOT_ORIGIN actual_checked_consumer=true actual_COMMIT_lock_observed=true outer_commit_unknown=true actual_commit_succeeded=true first_runtime_preserved=true secondary_uncertainty_count={} private_marker_committed=true economic_state_unchanged=true", probe.secondary_error_count);
    }).catch_unwind().await;
    finish_fixture_phase(original, drop_commit_probe(pools, &schema)).await;
}

async fn pool(env_key: &str) -> PgPool {
    pool_with_max(env_key, 4).await
}

async fn pool_with_max(env_key: &str, max_connections: u32) -> PgPool {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(connect_options(env_key))
        .await
        .unwrap_or_else(|_| panic!("fixture role connection failed"))
}

fn queue_application_name() -> String {
    format!("r5-paper-native-queue-{}", env("R5_INTEG_NONCE"))
}

async fn queue_worker_pool() -> PgPool {
    PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(
            connect_options("R5_PAPER_WORKER_URL").application_name(&queue_application_name()),
        )
        .await
        .unwrap_or_else(|_| panic!("fixture queue worker connection failed"))
}

fn connect_options(env_key: &str) -> PgConnectOptions {
    let url = std::env::var(env_key).unwrap_or_else(|_| panic!("fixture database URL is required"));
    url.parse::<PgConnectOptions>()
        .unwrap_or_else(|_| panic!("fixture database URL is invalid"))
        .log_statements(tracing::log::LevelFilter::Off)
}

async fn inspection_pool() -> PgPool {
    let owner = Uuid::parse_str(&env("R5_PAPER_OWNER_USER_ID")).expect("fixture Owner UUID");
    PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(10))
        .after_connect(move |connection, _| {
            Box::pin(async move {
                sqlx::query("SELECT set_config('app.actor_user_id',$1,false)")
                    .bind(owner.to_string())
                    .execute(&mut *connection)
                    .await?;
                sqlx::query("SET default_transaction_read_only = on")
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect_with(connect_options("R5_PAPER_MIGRATION_OWNER_URL"))
        .await
        .unwrap_or_else(|_| panic!("fixture read-only inspection connection failed"))
}

async fn refresh_fixture_session(
    pools: &FixturePools,
    user_id: Uuid,
    cookie_value: &str,
    csrf_token: &str,
) {
    let mut tx = pools
        .migration_owner
        .begin()
        .await
        .unwrap_or_else(|_| panic!("fixture session transaction failed"));
    sqlx::query("SELECT set_config('app.actor_user_id',$1,true)")
        .bind(user_id.to_string())
        .execute(&mut *tx)
        .await
        .unwrap_or_else(|_| panic!("fixture session actor scope failed"));
    let result = sqlx::query(
        "INSERT INTO public.web_sessions(id,user_id,session_hash,csrf_hash,expires_at) \
         VALUES($1,$2,$3,$4,clock_timestamp()+interval '2 hours') \
         ON CONFLICT(session_hash) DO UPDATE SET expires_at=EXCLUDED.expires_at \
         WHERE web_sessions.user_id=EXCLUDED.user_id \
           AND web_sessions.csrf_hash=EXCLUDED.csrf_hash AND web_sessions.revoked_at IS NULL",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(auth::sessions::cookie::hash(cookie_value))
    .bind(auth::csrf::hash_token(csrf_token))
    .execute(&mut *tx)
    .await
    .unwrap_or_else(|_| panic!("scoped fixture session seed or refresh failed"));
    assert_eq!(
        result.rows_affected(),
        1,
        "refresh only the same active fixture identity"
    );
    tx.commit()
        .await
        .unwrap_or_else(|_| panic!("fixture session commit failed"));
}

fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("sealed workflow input is missing"))
}

#[allow(
    clippy::option_env_unwrap,
    reason = "the ignored native test compiles in ordinary CI without this variable; execution is gated by its pinned ELF and manifest"
)]
fn config(app_url: String, paper_enabled: bool) -> ApiConfig {
    ApiConfig {
        cursor_secret: [0x5a; 32],
        max_jobs_per_owner: 10,
        recommendation_dataset: job_queue::recommendation::input::DatasetPin {
            id: Uuid::nil(),
            dataset_id: "unused-generic-fixture".to_owned(),
            version: "unused-generic-fixture".to_owned(),
            curated_version: 1,
            manifest_sha256: "0".repeat(64),
        },
        db_url: app_url,
        step_up_max_auth_age_secs: 900,
        artifact_root: PathBuf::from("/unused-generic-fixture"),
        seoul_today: api_server::http::state::system_seoul_today,
        candidate_eod_ready: || true,
        code_commit: option_env!("LAGRANGE_CODE_COMMIT")
            .expect("test must be compiled with the pinned R5-03 revision")
            .to_owned(),
        owner_beta_access: OwnerBetaAccessMode::OwnerOnly,
        owner_beta_paper: if paper_enabled {
            OwnerBetaPaperMode::Enabled
        } else {
            OwnerBetaPaperMode::Disabled
        },
        owner_beta_price_input: api_server::http::state::OwnerBetaPriceInputMode::Disabled,
        owner_beta_daily: job_queue::owner_beta::runner::daily_runtime::DailyProcessMode::Disabled,
        owner_beta_equity_signals: api_server::http::state::OwnerBetaEquitySignalsMode::Disabled,
        stock_price_beta_artifact_root: PathBuf::from("/unused-generic-fixture"),
        owner_equity_v2_pins: None,
        owner_equity_v2_api_artifact_root: None,
        owner_intraday_quotes: OwnerIntradayQuoteReadConfig::Disabled,
        owner_market_stream:
            api_server::http::owner_market_stream_config::MarketStreamReadConfig::Disabled,
        intraday_now: api_server::http::state::system_intraday_now,
    }
}

async fn route(pools: &FixturePools, paper_enabled: bool) -> axum::Router {
    let app_url = env("R5_PAPER_APP_URL");
    progress(ProgressStage::ApiStateBegin);
    let state = ApiState::from_pools(
        config(app_url, paper_enabled),
        pools.app.clone(),
        pools.admin.clone(),
        pools.audit.clone(),
    )
    .await
    .unwrap_or_else(|_| panic!("fixture API state failed"));
    progress(ProgressStage::ApiStateEnd);
    api_router(state)
}

async fn request(
    app: axum::Router,
    method: &str,
    path: &str,
    cookie_value: Option<&str>,
    csrf: Option<&str>,
    body: Option<&str>,
    idempotency: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-request-id", "r5-paper-native");
    if let Some(value) = cookie_value {
        request = request.header(
            header::COOKIE,
            format!("{}={value}", auth::sessions::cookie::NAME),
        );
    }
    if let Some(value) = csrf {
        request = request.header("x-csrf-token", value);
    }
    if let Some(value) = idempotency {
        request = request.header("idempotency-key", value);
    }
    let response = app
        .oneshot(
            request
                .body(
                    body.map(|value| Body::from(value.to_owned()))
                        .unwrap_or_else(Body::empty),
                )
                .expect("fixture request builds"),
        )
        .await
        .unwrap_or_else(|_| panic!("fixture router failed"));
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .unwrap_or_else(|_| panic!("fixture response body failed"))
        .to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_else(|_| {
        http_diagnostic(status, None);
        panic!("fixture response was not JSON")
    });
    http_diagnostic(status, Some(&value));
    (status, value)
}

fn http_diagnostic(status: StatusCode, value: Option<&serde_json::Value>) {
    let code = value
        .and_then(|value| value.get("error"))
        .and_then(|error| error.get("code"))
        .and_then(serde_json::Value::as_str)
        .and_then(|code| {
            HTTP_DIAGNOSTIC_CODES
                .iter()
                .copied()
                .find(|known| *known == code)
        });
    eprintln!(
        "R5_PAPER_HTTP_DIAGNOSTIC {}",
        serde_json::json!({"status": status.as_u16(), "code": code})
    );
}

fn worker_actor_is_unseeded(actor: Option<&str>) -> bool {
    // PostgreSQL may retain an empty custom-GUC placeholder after the genuine
    // admission readback rolls back its transaction-local actor setting.
    actor.is_none_or(str::is_empty)
}

async fn write_worker_backend_pid(pools: &FixturePools) -> i32 {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let (pid, actor) = worker_backend_identity(pools).await;
    assert!(
        worker_actor_is_unseeded(actor.as_deref()),
        "actual worker connection has no actor authority seed before the cycle"
    );
    let path = PathBuf::from(env("R5_PAPER_BACKEND_PID_OUTPUT"));
    let parent = path.parent().expect("private worker pid parent");
    let metadata = std::fs::symlink_metadata(parent).expect("private worker pid directory");
    // SAFETY: this verifies the actual test process identity only.
    let uid = unsafe { libc::geteuid() };
    assert!(metadata.is_dir() && metadata.uid() == uid && metadata.mode() & 0o7777 == 0o700);
    let phase = env("R5_PAPER_NATIVE_PHASE").replace('_', "-");
    assert_eq!(
        path,
        PathBuf::from(format!("/data/.integ/paper/backend-{phase}.json")),
        "worker backend identity stays in the owned phase directory"
    );
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)
        .expect("exclusive worker backend identity");
    let raw = serde_json::to_vec(&serde_json::json!({
        "classification": "SYNTHETIC_TEST_AUTHORITY_NOT_ORIGIN",
        "pid": pid,
    }))
    .expect("bounded backend identity");
    use std::io::Write;
    file.write_all(&raw).expect("backend identity write");
    file.sync_all().expect("backend identity sync");
    pid
}

async fn worker_backend_identity(pools: &FixturePools) -> (i32, Option<String>) {
    let (pid, actor, current_user, session_user, role, application): (
        i32,
        Option<String>,
        String,
        String,
        String,
        String,
    ) = sqlx::query_as(
        "SELECT pg_backend_pid(), current_setting('app.actor_user_id', true), \
         current_user::text, session_user::text, current_setting('role'), \
         current_setting('application_name')",
    )
    .fetch_one(&pools.worker)
    .await
    .unwrap_or_else(|_| panic!("worker backend identity query failed"));
    assert!(pid > 0);
    assert!(
        current_user == "worker" && session_user == "worker" && role == "none",
        "actual worker connection retains its login and has no active role override"
    );
    assert!(
        application == format!("r5-paper-native-worker-{}", env("R5_INTEG_NONCE")),
        "actual worker backend has the exact owned application name"
    );
    (pid, actor)
}

async fn assert_worker_backend_unset(pools: &FixturePools, expected_pid: i32) {
    let (pid, actor) = worker_backend_identity(pools).await;
    assert!(
        worker_actor_is_unseeded(actor.as_deref()),
        "worker action restores the unset or empty actor scope after returning"
    );
    if pid != expected_pid {
        // A pool borrow may reconnect. Accept that only after the previous
        // observed backend is absent; the pre-action crash marker stays fixed.
        let retired: bool = tokio::time::timeout(
            Duration::from_secs(5),
            sqlx::query_scalar(
                "SELECT NOT EXISTS(SELECT 1 FROM pg_catalog.pg_stat_activity WHERE pid=$1)",
            )
            .bind(expected_pid)
            .fetch_one(&pools.inspection),
        )
        .await
        .unwrap_or_else(|_| panic!("worker backend retirement check timed out"))
        .unwrap_or_else(|_| panic!("worker backend retirement query failed"));
        assert!(retired, "the previous owned worker backend must be retired");
    }
}

type WorkerConnectionIdentity = (i32, Option<String>, String, String, String, String);

async fn read_worker_connection_identity(
    connection: &mut sqlx::PgConnection,
) -> Result<WorkerConnectionIdentity, sqlx::Error> {
    sqlx::query_as(
        "SELECT pg_backend_pid(), current_setting('app.actor_user_id', true), \
         current_user::text, session_user::text, current_setting('role'), \
         current_setting('application_name')",
    )
    .fetch_one(connection)
    .await
}

fn assert_worker_connection_identity(
    identity: WorkerConnectionIdentity,
    expected_application: &str,
) -> i32 {
    let (pid, actor, current_user, session_user, role, application) = identity;
    assert!(pid > 0, "owned worker backend has a positive PID");
    assert!(
        current_user == "worker" && session_user == "worker" && role == "none",
        "owned connection retains its actual worker login without a role override"
    );
    assert!(
        worker_actor_is_unseeded(actor.as_deref()),
        "owned worker connection has no actor authority seed"
    );
    assert!(
        application == expected_application,
        "owned worker connection has the exact nonce-bound application identity"
    );
    pid
}

async fn assert_independent_queue_connection(pools: &FixturePools) {
    let mut transaction = tokio::time::timeout(Duration::from_secs(10), pools.worker.begin())
        .await
        .unwrap_or_else(|_| panic!("economic connection hold timed out"))
        .unwrap_or_else(|_| panic!("economic connection hold failed"));
    let identities = tokio::time::timeout(Duration::from_secs(10), async {
        let economic = read_worker_connection_identity(&mut transaction).await?;
        let mut queue = pools.queue_worker.acquire().await?;
        let queue_identity = read_worker_connection_identity(&mut queue).await?;
        Ok::<_, sqlx::Error>((economic, queue_identity))
    })
    .await;
    // Retain the original observation until the owned economic hold is released.
    let released = tokio::time::timeout(Duration::from_secs(10), transaction.rollback()).await;
    let (economic, queue) = identities
        .unwrap_or_else(|_| panic!("independent queue connection observation timed out"))
        .unwrap_or_else(|_| panic!("independent queue connection observation failed"));
    released
        .unwrap_or_else(|_| panic!("economic connection release timed out"))
        .unwrap_or_else(|_| panic!("economic connection release failed"));
    let economic_pid = assert_worker_connection_identity(
        economic,
        &format!("r5-paper-native-worker-{}", env("R5_INTEG_NONCE")),
    );
    let queue_pid = assert_worker_connection_identity(queue, &queue_application_name());
    assert!(
        economic_pid != queue_pid,
        "queue acquisition succeeds on a distinct backend while the economic transaction is held"
    );
}

async fn assert_queue_worker_unset(pools: &FixturePools) {
    let identity = tokio::time::timeout(Duration::from_secs(10), async {
        let mut queue = pools.queue_worker.acquire().await?;
        read_worker_connection_identity(&mut queue).await
    })
    .await
    .unwrap_or_else(|_| panic!("queue worker identity observation timed out"))
    .unwrap_or_else(|_| panic!("queue worker identity observation failed"));
    assert_worker_connection_identity(identity, &queue_application_name());
}

async fn fixture_runner_services(
    state: ApiState,
    pools: &FixturePools,
) -> api_server::paper_runner::RunnerServices {
    progress(ProgressStage::ServicesBegin);
    assert_independent_queue_connection(pools).await;
    let mut services = api_server::paper_runner::RunnerServices::new(
        state,
        pools.queue_worker.clone(),
        PathBuf::from("/unused-generic-fixture"),
    )
    .with_owner_beta_modes(OwnerBetaAccessMode::OwnerOnly, OwnerBetaPaperMode::Enabled)
    .with_deadlines(Duration::from_secs(90), Duration::from_secs(90))
    .with_preview_worker(
        "r5-paper-native-worker".to_owned(),
        Duration::from_secs(1),
        Duration::from_secs(5),
        Duration::from_secs(1),
    );
    // The queue keeps its independent pool; the caller-owned economic
    // transaction still performs publication and queue settlement atomically.
    services.worker_pool = pools.worker.clone();
    progress(ProgressStage::ServicesEnd);
    services
}

fn worker_item_error_stage(kind: &str, detail: &str) -> ProgressStage {
    if kind != "owner_beta_preview" {
        return ProgressStage::WorkerItemError;
    }
    match detail {
        "owner-beta preview exceeded the bounded cycle stage" => ProgressStage::WorkerStageTimedOut,
        "PAPER_BETA_RUNTIME_REJECTED" => ProgressStage::WorkerRuntimeRejected,
        "PAPER_BETA_CAPTURE_REJECTED" => ProgressStage::WorkerCaptureRejected,
        "PAPER_BETA_SQL_REJECTED" => ProgressStage::WorkerSqlRejected,
        "PAPER_BETA_SNAPSHOT_INVALID" => ProgressStage::WorkerSnapshotInvalid,
        "PAPER_BETA_CALCULATION_REJECTED" => ProgressStage::WorkerCalculationRejected,
        "PAPER_BETA_RETRY_REQUIRED" => ProgressStage::WorkerRetryRequired,
        "PAPER_BETA_LEASE_LOST" => ProgressStage::WorkerLeaseLost,
        "PAPER_BETA_COMMIT_UNKNOWN_READBACK_REQUIRED" => ProgressStage::WorkerCommitUnknown,
        _ => ProgressStage::WorkerItemError,
    }
}

// Emit only a closed type classification. Preserve the actual error as the
// original panic payload so the fixture's existing cleanup cannot replace it.
fn cycle_error_stage(error: &api_server::paper_runner::RunnerError) -> ProgressStage {
    use api_server::paper_runner::RunnerError;
    match error {
        RunnerError::Deadline {
            operation: "preview",
            ..
        } => ProgressStage::CycleDeadlinePreview,
        RunnerError::Deadline { .. } => ProgressStage::CycleDeadlineOther,
        RunnerError::Shutdown => ProgressStage::CycleShutdown,
        RunnerError::Preview(_) => ProgressStage::CyclePreviewRejected,
        RunnerError::PendingScan(_) => ProgressStage::CyclePendingScanRejected,
        RunnerError::AccountScan(_) => ProgressStage::CycleAccountScanRejected,
        RunnerError::AnnouncementScan(_) => ProgressStage::CycleAnnouncementScanRejected,
        RunnerError::InvalidDate(_) => ProgressStage::CycleInvalidDate,
    }
}

fn fail_worker_cycle(error: api_server::paper_runner::RunnerError) -> ! {
    progress(ProgressStage::CycleReturnErr);
    progress(cycle_error_stage(&error));
    std::panic::panic_any(error);
}

fn record_worker_item_errors(report: &api_server::paper_runner::CycleReport) {
    let mut emitted = Vec::with_capacity(10);
    for item in &report.item_errors {
        let stage = worker_item_error_stage(item.kind, &item.detail);
        if !emitted.contains(&stage) {
            progress(stage);
            emitted.push(stage);
        }
    }
}

async fn worker_cycle(
    pools: &FixturePools,
    date: chrono::NaiveDate,
) -> api_server::paper_runner::CycleReport {
    worker_cycle_with_backend_witness(pools, date, true).await
}

async fn worker_cycle_replay(
    pools: &FixturePools,
    date: chrono::NaiveDate,
) -> api_server::paper_runner::CycleReport {
    worker_cycle_with_backend_witness(pools, date, false).await
}

async fn worker_cycle_with_backend_witness(
    pools: &FixturePools,
    date: chrono::NaiveDate,
    publish_backend_witness: bool,
) -> api_server::paper_runner::CycleReport {
    progress(ProgressStage::WorkerCycleBegin);
    // The host's backend witness belongs to this phase. A replay still checks
    // its actual connection and actor scope, without replacing that first file.
    let worker_backend_pid = if publish_backend_witness {
        write_worker_backend_pid(pools).await
    } else {
        let (pid, actor) = worker_backend_identity(pools).await;
        assert!(worker_actor_is_unseeded(actor.as_deref()));
        pid
    };
    progress(ProgressStage::TrustedApiLoadBegin);
    TrustedPaperRuntime::load(PaperProcessRole::Api)
        .and_then(|runtime| runtime.revalidate())
        .expect("actual protected API executable and config revalidate");
    progress(ProgressStage::TrustedApiLoadEnd);
    progress(ProgressStage::TrustedWorkerLoadBegin);
    TrustedPaperRuntime::load(PaperProcessRole::Worker)
        .and_then(|runtime| runtime.revalidate())
        .expect("actual protected worker executable and config revalidate");
    progress(ProgressStage::TrustedWorkerLoadEnd);
    progress(ProgressStage::ApiStateBegin);
    let state = ApiState::from_pools(
        config(env("R5_PAPER_APP_URL"), true),
        pools.app.clone(),
        pools.admin.clone(),
        pools.audit.clone(),
    )
    .await
    .unwrap_or_else(|_| panic!("fixture API state failed"));
    progress(ProgressStage::ApiStateEnd);
    let services = fixture_runner_services(state, pools).await;
    let report = api_server::paper_runner::run_cycle_with_shutdown(&services, date, None)
        .await
        .unwrap_or_else(|error| fail_worker_cycle(error));
    record_worker_item_errors(&report);
    assert_worker_backend_unset(pools, worker_backend_pid).await;
    assert_queue_worker_unset(pools).await;
    progress(ProgressStage::WorkerCycleEnd);
    report
}

#[derive(Debug, PartialEq, Eq)]
struct ActionEconomicState(i64, i64, i64, i64, i64, i64, i64);

async fn action_economic_state(pools: &FixturePools) -> ActionEconomicState {
    let values: (i64, i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT \
           (SELECT count(*) FROM public.owner_beta_paper_sessions \
             WHERE account_id=(SELECT account_id FROM public.owner_beta_paper_control WHERE id='etf11')), \
           (SELECT count(*) FROM public.orders \
             WHERE account_id=(SELECT account_id FROM public.owner_beta_paper_control WHERE id='etf11')), \
           (SELECT count(*) FROM public.fills \
             WHERE account_id=(SELECT account_id FROM public.owner_beta_paper_control WHERE id='etf11')), \
           (SELECT count(*) FROM public.cash_ledger \
             WHERE account_id=(SELECT account_id FROM public.owner_beta_paper_control WHERE id='etf11') \
               AND event_type<>'DEPOSIT'), \
           (SELECT count(*) FROM public.paper_settlement_outbox \
             WHERE pending_target_id IN (SELECT pending_target_id \
               FROM public.owner_beta_paper_sessions \
               WHERE account_id=(SELECT account_id FROM public.owner_beta_paper_control WHERE id='etf11'))), \
           (SELECT count(*) FROM public.daily_equity \
             WHERE account_id=(SELECT account_id FROM public.owner_beta_paper_control WHERE id='etf11')), \
           (SELECT count(*) FROM public.paper_rebalance_previews \
             WHERE account_id=(SELECT account_id FROM public.owner_beta_paper_control WHERE id='etf11') \
               AND status='READY')",
    )
    .fetch_one(&pools.inspection)
    .await
    .unwrap_or_else(|_| panic!("checked-action economic snapshot unavailable"));
    ActionEconomicState(
        values.0, values.1, values.2, values.3, values.4, values.5, values.6,
    )
}

fn is_checked_action_capture_refusal(report: &api_server::paper_runner::CycleReport) -> bool {
    report.item_errors.iter().any(|error| {
        error.kind == "owner_beta_preview" && error.detail == "PAPER_BETA_CAPTURE_REJECTED"
    })
}

async fn fixture_auto_setup(pools: &FixturePools, auto_apply: bool) {
    // This selects the fixture's manual/AUTO precondition. Authenticated consent,
    // the control epoch, and the protected process binding stay authoritative.
    let owner = Uuid::parse_str(&env("R5_PAPER_OWNER_USER_ID")).expect("fixture Owner UUID");
    let strategy =
        Uuid::parse_str(&env("R5_PAPER_STRATEGY_CONFIG_ID")).expect("fixture strategy UUID");
    let mut tx = pools
        .migration_owner
        .begin()
        .await
        .unwrap_or_else(|_| panic!("fixture AUTO setup transaction failed"));
    sqlx::query("SELECT set_config('app.actor_user_id',$1,true)")
        .bind(owner.to_string())
        .execute(&mut *tx)
        .await
        .unwrap_or_else(|_| panic!("fixture AUTO setup actor scope failed"));
    let result = sqlx::query(
        "UPDATE public.account_strategy_bindings b SET auto_apply_recommendations=$1 \
         FROM public.owner_beta_paper_control c \
         WHERE c.id='etf11' AND c.owner_user_id=$2 AND c.strategy_config_id=$3 \
           AND b.owner_user_id=c.owner_user_id AND b.account_id=c.account_id \
           AND b.strategy_config_id=c.strategy_config_id AND b.unbound_at IS NULL",
    )
    .bind(auto_apply)
    .bind(owner)
    .bind(strategy)
    .execute(&mut *tx)
    .await
    .unwrap_or_else(|_| panic!("exact configured fixture AUTO precondition failed"));
    assert_eq!(result.rows_affected(), 1);
    tx.commit()
        .await
        .unwrap_or_else(|_| panic!("fixture AUTO setup commit failed"));
}

async fn bootstrap_off(pools: &FixturePools) {
    let owner = Uuid::parse_str(&env("R5_PAPER_OWNER_USER_ID")).expect("fixture Owner UUID");
    let strategy =
        Uuid::parse_str(&env("R5_PAPER_STRATEGY_CONFIG_ID")).expect("fixture strategy UUID");
    let configured: serde_json::Value = sqlx::query_scalar(
        "SELECT public.configure_owner_beta_paper_v1($1,'R5 synthetic Paper',$2::jsonb,NULL::jsonb,$3)",
    ).bind(strategy).bind(CONTRACT).bind("sealed-workflow-configure-v1")
        .fetch_one(&pools.migration_owner).await.unwrap_or_else(|_| panic!("fixture Paper configuration failed"));
    assert_eq!(
        configured["outcome"], "OK",
        "fixture account configures once while OFF"
    );
    let account_id = configured["data"]["account_id"]
        .as_str()
        .expect("configured account identity")
        .to_owned();

    let nonce = env("R5_INTEG_NONCE");
    let cookie_value = format!("r5-paper-cookie-{nonce}");
    let csrf_token = format!("r5-paper-csrf-{nonce}");
    refresh_fixture_session(pools, owner, &cookie_value, &csrf_token).await;
    let member = Uuid::parse_str(&env("R5_PAPER_MEMBER_USER_ID")).expect("fixture member UUID");
    let member_cookie = format!("r5-paper-member-cookie-{nonce}");
    let member_csrf = format!("r5-paper-member-csrf-{nonce}");
    refresh_fixture_session(pools, member, &member_cookie, &member_csrf).await;
    let app = route(pools, true).await;
    let consent = "{\"opt_in\":true,\"expected_epoch\":0}".to_owned();
    let (status, value) = request(
        app,
        "POST",
        "/api/v1/paper/owner-beta/consent",
        Some(&cookie_value),
        Some(&csrf_token),
        Some(&consent),
        Some("sealed-workflow-consent-v1"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "actual authenticated consent route is available while OFF"
    );
    assert_eq!(value["account_id"], account_id);
    assert_eq!(value["opt_in"], true);
    assert!(value["event_id"].as_str().is_some());
    let (replay_status, replayed) = request(
        route(pools, true).await,
        "POST",
        "/api/v1/paper/owner-beta/consent",
        Some(&cookie_value),
        Some(&csrf_token),
        Some(&consent),
        Some("sealed-workflow-consent-v1"),
    )
    .await;
    assert_eq!(replay_status, StatusCode::OK);
    for field in ["event_id", "account_id", "control_epoch", "opt_in"] {
        assert_eq!(replayed[field], value[field]);
    }
    assert_eq!(replayed["replayed"], true);
    let (mismatch_status, mismatch) = request(
        route(pools, true).await,
        "POST",
        "/api/v1/paper/owner-beta/consent",
        Some(&cookie_value),
        Some(&csrf_token),
        Some("{\"opt_in\":false,\"expected_epoch\":0}"),
        Some("sealed-workflow-consent-v1"),
    )
    .await;
    assert_eq!(mismatch_status, StatusCode::CONFLICT);
    assert!(
        mismatch["error"]["code"] == "IDEMPOTENCY_KEY_MISMATCH",
        "changed consent returns the typed idempotency denial"
    );
    let event_id = Uuid::parse_str(value["event_id"].as_str().expect("consent event UUID"))
        .expect("consent event UUID");
    let (kind, event_epoch, opt_in, selected_event, control_epoch, control_state, events):
        (String, i64, bool, Option<Uuid>, i64, String, i64) = sqlx::query_as(
            "SELECT e.kind,e.control_epoch,(e.payload->>'opt_in')::boolean, \
                    c.consent_event_id,c.control_epoch,c.state, \
                    (SELECT count(*) FROM public.owner_beta_paper_events x \
                     WHERE x.account_id=c.account_id AND x.kind IN ('CONSENT_GRANTED','CONSENT_REVOKED')) \
             FROM public.owner_beta_paper_events e JOIN public.owner_beta_paper_control c \
               ON c.account_id=e.account_id AND c.owner_user_id=e.owner_user_id \
             WHERE c.id='etf11' AND e.id=$1",
        )
        .bind(event_id)
        .fetch_one(&pools.inspection)
        .await
        .unwrap_or_else(|_| panic!("consent replay and mismatch durable readback failed"));
    assert_eq!(kind, "CONSENT_GRANTED");
    assert_eq!((event_epoch, control_epoch, events), (0, 0, 1));
    assert!(opt_in);
    assert_eq!(selected_event, Some(event_id));
    assert_eq!(control_state, "OFF");
    fixture_auto_setup(pools, false).await;
    let state_cookie = format!("r5-paper-cookie-{nonce}");
    let (state_status, state) = request(
        route(pools, true).await,
        "GET",
        "/api/v1/paper/owner-beta/state",
        Some(&state_cookie),
        None,
        None,
        None,
    )
    .await;
    assert_eq!(
        state_status,
        StatusCode::OK,
        "real state route reads the configured OFF account"
    );
    assert_eq!(state["state"], "OFF");
    assert_eq!(state["account_id"], account_id);
    let (member_status, _) = request(
        route(pools, true).await,
        "GET",
        "/api/v1/paper/owner-beta/state",
        Some(&member_cookie),
        None,
        None,
        None,
    )
    .await;
    assert_eq!(
        member_status,
        StatusCode::FORBIDDEN,
        "another tenant cannot read the configured Owner account"
    );
    let result = serde_json::json!({"classification":"SYNTHETIC_TEST_AUTHORITY_NOT_ORIGIN","owner_user_id":owner,"strategy_config_id":strategy,"account_id":account_id,"consent_event_id":value["event_id"],"control_epoch":value["control_epoch"]});
    let out = PathBuf::from(env("R5_PAPER_BOOTSTRAP_OUTPUT"));
    std::fs::write(
        out,
        serde_json::to_vec(&result).expect("bounded fixture ids"),
    )
    .expect("private fixture result");
    eprintln!(
        "R5_PAPER_NATIVE phase=bootstrap_off classification=SYNTHETIC_TEST_AUTHORITY_NOT_ORIGIN outcome=OK fixture_only_auto_setup=false authenticated_consent=true consent_replay=true consent_key_mismatch=409 consent_events=1"
    );
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "fresh Root-reviewed synthetic five-session workflow and protected API/worker process required"]
async fn owner_beta_paper_native_workflow_phase() {
    install_lifecycle_subscriber();
    progress(ProgressStage::PhaseBegin);
    progress(ProgressStage::PoolsOpenBegin);
    let pools = FixturePools::open().await;
    progress(ProgressStage::PoolsOpenEnd);
    let original = std::panic::AssertUnwindSafe(async {
        let phase = env("R5_PAPER_NATIVE_PHASE");
        if phase == "protected_process" {
            progress(ProgressStage::FixtureSessionBegin);
            let nonce = env("R5_INTEG_NONCE");
            let owner = Uuid::parse_str(&env("R5_PAPER_OWNER_USER_ID")).expect("fixture Owner UUID");
            refresh_fixture_session(
                &pools,
                owner,
                &format!("r5-paper-cookie-{nonce}"),
                &format!("r5-paper-csrf-{nonce}"),
            )
            .await;
            progress(ProgressStage::FixtureSessionEnd);
        }
        match phase.as_str() {
            "bootstrap_off" => bootstrap_off(&pools).await,
            "local_non_send_sql_probe" => {
                let result = run_bound_paper_action(
                    PaperProcessRole::Worker,
                    Duration::from_secs(90),
                    None,
                    LocalExecutorSqlProbe {
                        pool: pools.worker.clone(),
                    },
                )
                .await;
                assert!(matches!(result, Ok(true)));
                eprintln!(
                    "R5_PAPER_NATIVE phase=local_non_send_sql_probe classification=SYNTHETIC_TEST_AUTHORITY_NOT_ORIGIN actual_checked_action=true rc_across_sql_await=true same_pool_transaction=true rolled_back=true"
                );
            }
            "checked_action_cancel_sql_wait" => {
                checked_action_cancel_or_deadline_phase(&pools, false).await;
            }
            "checked_action_deadline_after_entry" => {
                checked_action_cancel_or_deadline_phase(&pools, true).await;
            }
            "checked_action_dirty_postcommit_unknown" => {
                checked_action_dirty_postcommit_phase(&pools).await;
            }
            "protected_process" => {
                progress(ProgressStage::TrustedApiLoadBegin);
                let trusted = TrustedPaperRuntime::load(PaperProcessRole::Api)
                    .expect("actual protected API executable and config bind");
                trusted
                    .revalidate()
                    .expect("actual protected API config revalidates");
                progress(ProgressStage::TrustedApiLoadEnd);
                progress(ProgressStage::TrustedWorkerLoadBegin);
                let worker_runtime = TrustedPaperRuntime::load(PaperProcessRole::Worker)
                    .expect("actual protected worker executable and config bind");
                worker_runtime
                    .revalidate()
                    .expect("actual protected worker config revalidates");
                progress(ProgressStage::TrustedWorkerLoadEnd);
                progress(ProgressStage::ApiStateBegin);
                let state = ApiState::from_pools(
                    config(env("R5_PAPER_APP_URL"), true),
                    pools.app.clone(),
                    pools.admin.clone(),
                    pools.audit.clone(),
                )
                .await
                .unwrap_or_else(|_| panic!("fixture API state failed"));
                progress(ProgressStage::ApiStateEnd);
                let date = chrono::NaiveDate::parse_from_str(&env("R5_PAPER_PROCESS_DATE"), "%Y-%m-%d")
                    .expect("fixed fixture process date");
                let nonce = env("R5_INTEG_NONCE");
                let cookie_value = format!("r5-paper-cookie-{nonce}");
                let csrf_token = format!("r5-paper-csrf-{nonce}");
                let run_id = Uuid::parse_str(&env("R5_PAPER_T_RUN_ID")).expect("actual Daily T run id");
                let submit_body =
                    format!("{{\"recommendation_run_id\":\"{run_id}\",\"expected_epoch\":1}}");
                progress(ProgressStage::PostPreviewBegin);
                let (submit_status, submitted) = request(
                    api_router(state.clone()),
                    "POST",
                    "/api/v1/paper/owner-beta/previews",
                    Some(&cookie_value),
                    Some(&csrf_token),
                    Some(&submit_body),
                    Some("sealed-paper-manual-preview-v1"),
                )
                .await;
                progress(ProgressStage::PostPreviewEnd);
                assert_eq!(
                    submit_status,
                    StatusCode::ACCEPTED,
                    "authenticated Owner manual preview is accepted"
                );
                let preview_id = submitted["preview_id"]
                    .as_str()
                    .expect("actual preview identity")
                    .to_owned();
                let services = fixture_runner_services(state.clone(), &pools).await;
                progress(ProgressStage::WorkerCycleBegin);
                let worker_backend_pid = write_worker_backend_pid(&pools).await;
                let report = api_server::paper_runner::run_cycle_with_shutdown(&services, date, None)
                    .await
                    .unwrap_or_else(|error| fail_worker_cycle(error));
                record_worker_item_errors(&report);
                assert_worker_backend_unset(&pools, worker_backend_pid).await;
                assert_queue_worker_unset(&pools).await;
                progress(ProgressStage::WorkerCycleEnd);
                progress(ProgressStage::GetPreviewBegin);
                let (_, preview) = request(
                    api_router(state.clone()),
                    "GET",
                    &format!("/api/v1/paper/owner-beta/previews/{preview_id}"),
                    Some(&cookie_value),
                    None,
                    None,
                    None,
                )
                .await;
                progress(ProgressStage::GetPreviewEnd);
                assert_eq!(
                    preview["status"], "READY",
                    "actual worker published the computed T-close preview"
                );
                let preview_token = preview["preview_token"]
                    .as_str()
                    .expect("actual immutable preview token");
                let apply_body =
                    format!("{{\"preview_token\":\"{preview_token}\",\"expected_epoch\":1}}");
                progress(ProgressStage::ApplyBegin);
                let (apply_status, applied) = request(
                    api_router(state.clone()),
                    "POST",
                    &format!("/api/v1/paper/owner-beta/previews/{preview_id}/apply"),
                    Some(&cookie_value),
                    Some(&csrf_token),
                    Some(&apply_body),
                    Some("sealed-paper-manual-apply-v1"),
                )
                .await;
                progress(ProgressStage::ApplyEnd);
                assert_eq!(
                    apply_status,
                    StatusCode::OK,
                    "actual authenticated apply commits the immutable T preview"
                );
                let pending_target_id = applied["pending_target_id"]
                    .as_str()
                    .expect("actual durable target identity")
                    .to_owned();
                let pending_target_uuid =
                    Uuid::parse_str(&pending_target_id).expect("actual pending target UUID");
                assert_eq!(applied["replayed"], false);
                progress(ProgressStage::ApplyReplayBegin);
                let (replay_status, replayed) = request(
                    api_router(state.clone()),
                    "POST",
                    &format!("/api/v1/paper/owner-beta/previews/{preview_id}/apply"),
                    Some(&cookie_value),
                    Some(&csrf_token),
                    Some(&apply_body),
                    Some("sealed-paper-manual-apply-v1"),
                )
                .await;
                progress(ProgressStage::ApplyReplayEnd);
                assert_eq!(replay_status, StatusCode::OK);
                assert_eq!(replayed["pending_target_id"], pending_target_id);
                assert_eq!(replayed["replayed"], true);
                progress(ProgressStage::DurableAssertionsBegin);
                let (target_state, unsettled, unvalued): (String, bool, bool) = sqlx::query_as(
                    "SELECT state,settled_at IS NULL,valued_at IS NULL \
                     FROM public.owner_beta_paper_sessions WHERE pending_target_id=$1",
                )
                .bind(pending_target_uuid)
                .fetch_one(&pools.inspection)
                .await
                .unwrap_or_else(|_| panic!("pre-E target readback failed"));
                assert_eq!(target_state, "PENDING", "E has not been captured at T");
                assert!(unsettled && unvalued);
                let (orders, fills, non_deposit_ledger_rows): (i64, i64, i64) = sqlx::query_as(
                    "SELECT \
                       (SELECT count(*) FROM public.orders WHERE account_id=(SELECT account_id FROM public.owner_beta_paper_control WHERE id='etf11')), \
                       (SELECT count(*) FROM public.fills WHERE account_id=(SELECT account_id FROM public.owner_beta_paper_control WHERE id='etf11')), \
                       (SELECT count(*) FROM public.cash_ledger WHERE account_id=(SELECT account_id FROM public.owner_beta_paper_control WHERE id='etf11') AND event_type<>'DEPOSIT')",
                )
                .fetch_one(&pools.inspection)
                .await
                .unwrap_or_else(|_| panic!("pre-E ledger readback failed"));
                assert_eq!((orders, fills, non_deposit_ledger_rows), (0, 0, 0));
                let (status, state_body) = request(
                    api_router(state),
                    "GET",
                    "/api/v1/paper/owner-beta/state",
                    Some(&cookie_value),
                    None,
                    None,
                    None,
                )
                .await;
                assert_eq!(
                    status,
                    StatusCode::OK,
                    "actual authenticated Owner state route is available"
                );
                assert_eq!(state_body["state"], "ENABLED");
                progress(ProgressStage::DurableAssertionsEnd);
                eprintln!(
                    "R5_PAPER_NATIVE phase=protected_process classification=SYNTHETIC_TEST_AUTHORITY_NOT_ORIGIN api_and_worker_executable=config_revalidated=true preview_id={} pending_target_id={} manual_apply_replay=true pre_E_pending=true pre_E_orders=0 pre_E_fills=0 pre_E_non_deposit_ledger_rows=0 worker_cycle=completed targets_seen={} targets_settled={} valuations_written={}",
                    preview_id,
                    pending_target_id,
                    report.owner_beta_targets_seen,
                    report.owner_beta_targets_settled,
                    report.owner_beta_valuations_written
                );
            }
            "missing_e_wait" => {
                let date = chrono::NaiveDate::parse_from_str(&env("R5_PAPER_PROCESS_DATE"), "%Y-%m-%d")
                    .expect("fixed missing-E process date");
                let report = worker_cycle(&pools, date).await;
                assert_eq!(report.owner_beta_targets_seen, 1);
                assert_eq!(report.owner_beta_targets_settled, 0);
                assert_eq!(report.owner_beta_valuations_written, 0);
                let (target_id, state, unsettled, unvalued): (Uuid, String, bool, bool) =
                    sqlx::query_as(
                        "SELECT s.pending_target_id,s.state,s.settled_at IS NULL,s.valued_at IS NULL \
                         FROM public.owner_beta_paper_sessions s \
                         WHERE s.account_id=(SELECT account_id FROM public.owner_beta_paper_control WHERE id='etf11') \
                           AND s.effective_session=$1",
                    )
                    .bind(date)
                    .fetch_one(&pools.inspection)
                    .await
                    .unwrap_or_else(|_| panic!("missing-E target readback failed"));
                assert!(matches!(
                    state.as_str(),
                    "WAITING_PRICE" | "WAITING_ADMISSION"
                ));
                assert!(unsettled && unvalued);
                let outbox: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM public.paper_settlement_outbox WHERE pending_target_id=$1",
                )
                .bind(target_id)
                .fetch_one(&pools.inspection)
                .await
                .unwrap_or_else(|_| panic!("missing-E outbox readback failed"));
                assert_eq!(outbox, 0);
                eprintln!(
                    "R5_PAPER_NATIVE phase=missing_e_wait classification=SYNTHETIC_TEST_AUTHORITY_NOT_ORIGIN actual_worker=true target_state={} targets_seen={} settled=0 valued=0 outbox=0",
                    state, report.owner_beta_targets_seen
                );
            }
            "failed_e_settle" | "admitted_e_no_run_settle" => {
                let phase = env("R5_PAPER_NATIVE_PHASE");
                let date = chrono::NaiveDate::parse_from_str(
                    &env("R5_PAPER_PROCESS_DATE"),
                    "%Y-%m-%d",
                )
                .expect("fixed admitted-E process date");
                let readback = run_bound_paper_action(
                    PaperProcessRole::Worker, Duration::from_secs(90), None,
                    AdmissionReadbackWork { pool: pools.worker.clone(), date },
                ).await.unwrap_or_else(|_| panic!("genuine E admission readback unavailable"));
                assert!(!readback["admission"].is_null());
                assert!(!readback["candidate_pins"].is_null());
                assert!(!readback["admitted_at"].is_null());
                if phase == "admitted_e_no_run_settle" {
                    for key in ["candidate_run_id", "run", "job_id", "attempt_id", "terminal", "attempts", "finished_at"] {
                        assert!(readback[key].is_null(), "the actual no-run E SQL field {key} must be NULL before settlement");
                    }
                    let owner = Uuid::parse_str(readback["slot"]["cohort"]["owner_user_id"].as_str().expect("actual cohort Owner")).unwrap();
                    let config = Uuid::parse_str(readback["slot"]["cohort"]["strategy_config_id"].as_str().expect("actual cohort strategy")).unwrap();
                    let reservations: i64 = sqlx::query_scalar(
                        "SELECT count(*) FROM public.owner_beta_daily_reservations \
                         WHERE owner_user_id=$1 AND strategy_config_id=$2 AND session_date=$3",
                    ).bind(owner).bind(config).bind(date).fetch_one(&pools.inspection).await
                        .unwrap_or_else(|_| panic!("no-run E reservation witness unavailable"));
                    assert_eq!(reservations, 0, "E has no reservation, not merely no linked run");
                } else {
                    assert_eq!(readback["terminal"], "FAILED");
                    assert!(!readback["candidate_run_id"].is_null());
                    assert!(!readback["finished_at"].is_null());
                }
                let report = worker_cycle(&pools, date).await;
                assert_eq!(report.owner_beta_targets_seen, 1);
                assert_eq!(report.owner_beta_targets_settled, 1);
                assert_eq!(report.owner_beta_valuations_written, 1);
                let (target_id, state, settled, valued): (Uuid, String, bool, bool) =
                    sqlx::query_as(
                        "SELECT pending_target_id,state,settled_at IS NOT NULL,valued_at IS NOT NULL \
                         FROM public.owner_beta_paper_sessions \
                         WHERE account_id=(SELECT account_id FROM public.owner_beta_paper_control WHERE id='etf11') \
                           AND effective_session=$1",
                    )
                    .bind(date)
                    .fetch_one(&pools.inspection)
                    .await
                    .unwrap_or_else(|_| panic!("failed-E admitted-price result readback failed"));
                assert_eq!(state, "VALUED");
                assert!(settled && valued);
                let first_state = action_economic_state(&pools).await;
                let replay = worker_cycle_replay(&pools, date).await;
                assert_eq!(replay.owner_beta_targets_settled, 0);
                assert_eq!(replay.owner_beta_valuations_written, 0);
                assert_eq!(action_economic_state(&pools).await, first_state);
                let outbox: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM public.paper_settlement_outbox WHERE pending_target_id=$1",
                )
                .bind(target_id)
                .fetch_one(&pools.inspection)
                .await
                .unwrap_or_else(|_| panic!("failed-E settlement outbox readback failed"));
                assert_eq!(outbox, 1);
                eprintln!(
                    "R5_PAPER_NATIVE phase={} classification=SYNTHETIC_TEST_AUTHORITY_NOT_ORIGIN actual_worker=true actual_E_SQL_precondition_witnessed=true e_admission_linked=true settled_once=true valued_once=true replay_no_extra_economics=true",
                    phase
                );
            }
            "blocked_tamper_auto_queue"
            | "blocked_tamper_preview"
            | "blocked_tamper_settlement"
            | "blocked_tamper_valuation" => {
                let phase = env("R5_PAPER_NATIVE_PHASE");
                let date = chrono::NaiveDate::parse_from_str(
                    &env("R5_PAPER_PROCESS_DATE"),
                    "%Y-%m-%d",
                )
                .expect("fixed blocked-action process date");
                let before = action_economic_state(&pools).await;
                let report = worker_cycle(&pools, date).await;
                let after = action_economic_state(&pools).await;
                assert!(
                    is_checked_action_capture_refusal(&report),
                    "the external latch must release after a retained input changes during the selected blocked SQL action"
                );
                assert_eq!(
                    after, before,
                    "a checked-action precommit refusal must leave no new target, READY preview, fill, ledger, outbox or equity row"
                );
                eprintln!(
                    "R5_PAPER_NATIVE phase={} classification=SYNTHETIC_TEST_AUTHORITY_NOT_ORIGIN actual_worker=true blocked_sql_released=true capture_refused=true economic_state_unchanged=true heartbeat_ms=1000 lease_ms=5000",
                    phase
                );
            }
            "calendar_link_refusal_preview"
            | "calendar_link_refusal_settlement"
            | "calendar_link_refusal_valuation" => {
                let phase = env("R5_PAPER_NATIVE_PHASE");
                let date = chrono::NaiveDate::parse_from_str(
                    &env("R5_PAPER_PROCESS_DATE"),
                    "%Y-%m-%d",
                )
                .expect("fixed calendar-link process date");
                let before = action_economic_state(&pools).await;
                let report = worker_cycle(&pools, date).await;
                assert!(
                    is_checked_action_capture_refusal(&report),
                    "a mismatched persisted calendar link must fail in the selected real consumer"
                );
                assert_eq!(
                    action_economic_state(&pools).await,
                    before,
                    "a mismatched owner, policy, row or terminal witness cannot commit economics"
                );
                eprintln!(
                    "R5_PAPER_NATIVE phase={} classification=SYNTHETIC_TEST_AUTHORITY_NOT_ORIGIN actual_worker=true calendar_link_refused=true economic_state_unchanged=true",
                    phase
                );
            }
            "old_e_without_recovery" => {
                let (date, effective, target_id, already_settled) = old_target_witness(&pools).await;
                let before = old_economic_rows(&pools).await;
                let before_rows = old_economic_value(&before);
                let report = worker_cycle(&pools, date).await;
                assert!(report.item_errors.is_empty(), "old-E refusal must not be an unrelated early worker failure");
                assert_eq!(report.owner_beta_targets_seen, 1);
                assert_eq!(report.owner_beta_targets_settled, 0);
                assert_eq!(report.owner_beta_valuations_written, 0);
                let after = old_economic_rows(&pools).await;
                assert!(old_economic_value(&after)==before_rows,"old-E economic row difference (values remain private)");
                let (state, unsettled, unvalued): (String, bool, bool) = sqlx::query_as(
                    "SELECT state,settled_at IS NULL,valued_at IS NULL \
                     FROM public.owner_beta_paper_sessions \
                     WHERE account_id=(SELECT account_id FROM public.owner_beta_paper_control WHERE id='etf11') \
                       AND effective_session=$1 AND pending_target_id=$2",
                )
                .bind(effective)
                .bind(target_id)
                .fetch_one(&pools.inspection)
                .await
                .unwrap_or_else(|_| panic!("old-E recovery wait readback failed"));
                assert!(matches!(state.as_str(),"WAITING_PRICE"|"WAITING_ADMISSION"|"RECOVERY_REQUIRED") || (already_settled && matches!(state.as_str(),"SETTLED_TRADE"|"SETTLED_NO_TRADE")));
                assert_eq!(unsettled, !already_settled);
                assert!(unvalued);
                eprintln!(
                    "R5_PAPER_NATIVE phase=old_e_without_recovery classification=SYNTHETIC_TEST_AUTHORITY_NOT_ORIGIN actual_worker=true old_e=true explicit_recovery=false settled=0 valued=0 economic_state_unchanged=true"
                );
            }
            "old_e_with_recovery" => {
                let (date, effective, target_id, already_settled) = old_target_witness(&pools).await;
                let first = worker_cycle(&pools, date).await;
                assert!(first.item_errors.is_empty());
                assert_eq!(first.owner_beta_targets_seen, 1);
                assert_eq!(first.owner_beta_targets_settled, usize::from(!already_settled));
                assert_eq!(first.owner_beta_valuations_written, 1);
                let (target_id, state, settled, valued): (Uuid, String, bool, bool) =
                    sqlx::query_as(
                        "SELECT pending_target_id,state,settled_at IS NOT NULL,valued_at IS NOT NULL \
                         FROM public.owner_beta_paper_sessions \
                         WHERE account_id=(SELECT account_id FROM public.owner_beta_paper_control WHERE id='etf11') \
                           AND effective_session=$1 AND pending_target_id=$2",
                    )
                    .bind(effective)
                .bind(target_id)
                    .fetch_one(&pools.inspection)
                    .await
                    .unwrap_or_else(|_| panic!("old-E recovery completion readback failed"));
                assert_eq!(state, "VALUED");
                assert!(settled && valued);
                let after_first = old_economic_rows(&pools).await;
                let after_first_rows = old_economic_value(&after_first);
                let replay = worker_cycle_replay(&pools, date).await;
                assert!(replay.item_errors.is_empty());
                assert_eq!(replay.owner_beta_targets_settled, 0);
                assert_eq!(replay.owner_beta_valuations_written, 0);
                let after_replay = old_economic_rows(&pools).await;
                assert!(old_economic_value(&after_replay)==after_first_rows,"replay economic row difference (values remain private)");
                let receipt_count: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM public.paper_settlement_outbox WHERE pending_target_id=$1",
                )
                .bind(target_id)
                .fetch_one(&pools.inspection)
                .await
                .unwrap_or_else(|_| panic!("old-E recovery outbox readback failed"));
                assert_eq!(receipt_count, 1);
                eprintln!(
                    "R5_PAPER_NATIVE phase=old_e_with_recovery classification=SYNTHETIC_TEST_AUTHORITY_NOT_ORIGIN actual_worker=true old_e=true explicit_recovery=true settled_once=true valued_once=true replay_no_extra_economics=true"
                );
            }
            "crash_precommit" | "crash_prevaluation" => {
                let phase = env("R5_PAPER_NATIVE_PHASE");
                let date = chrono::NaiveDate::parse_from_str(&env("R5_PAPER_PROCESS_DATE"), "%Y-%m-%d")
                    .expect("fixed crash-boundary process date");
                let report = worker_cycle(&pools, date).await;
                eprintln!(
                    "R5_PAPER_NATIVE phase={} classification=SYNTHETIC_TEST_AUTHORITY_NOT_ORIGIN crash_boundary_not_observed=true targets_seen={} settled={} valued={}",
                    phase,
                    report.owner_beta_targets_seen,
                    report.owner_beta_targets_settled,
                    report.owner_beta_valuations_written
                );
            }
            "settle_e" => {
                let date = chrono::NaiveDate::parse_from_str(&env("R5_PAPER_PROCESS_DATE"), "%Y-%m-%d")
                    .expect("fixed E process date");
                let account_id: Uuid = sqlx::query_scalar(
                    "SELECT account_id FROM public.owner_beta_paper_control WHERE id='etf11'",
                )
                .fetch_one(&pools.inspection)
                .await
                .unwrap_or_else(|_| panic!("configured account identity unavailable"));
                let (
                    prior_target_id,
                    prior_state,
                    prior_kind,
                    prior_origin,
                    prior_settlement_sha,
                    prior_valuation_sha,
                    prior_receipt,
                ): (
                    Uuid,
                    String,
                    Option<String>,
                    String,
                    Option<String>,
                    Option<String>,
                    serde_json::Value,
                ) = sqlx::query_as(
                    "SELECT s.pending_target_id,s.state,s.settlement_kind,s.processing_origin, \
                            s.settlement_sha256,s.valuation_sha256,s.receipt_json \
                     FROM public.owner_beta_paper_sessions s \
                     WHERE s.account_id=$1 AND s.effective_session=$2",
                )
                .bind(account_id)
                .bind(date)
                .fetch_one(&pools.inspection)
                .await
                .unwrap_or_else(|_| panic!("preexisting committed E settlement readback failed"));
                assert_eq!(prior_state, "SETTLED_TRADE");
                assert_eq!(prior_kind.as_deref(), Some("TRADE"));
                assert_eq!(prior_origin, "MANUAL");
                assert!(prior_settlement_sha.is_some());
                assert_eq!(prior_valuation_sha, None);
                assert_eq!(
                    prior_receipt["settlement"]["calculation_sha256"].as_str(),
                    prior_settlement_sha.as_deref()
                );
                let prior_receipt_ids = prior_receipt["order_fill_ids"]
                    .as_array()
                    .expect("preexisting committed settlement order/fill identities")
                    .clone();
                assert_eq!(prior_receipt_ids.len(), 1);
                let prior_outbox_id = prior_receipt["outbox_id"]
                    .as_str()
                    .expect("preexisting committed settlement outbox identity")
                    .to_owned();
                let prior_order_refs: Vec<(String, String, String, String, String)> = sqlx::query_as(
                    "SELECT o.id::text,f.id::text,o.side::text,o.order_ref,f.fill_ref \
                         FROM public.orders o JOIN public.fills f ON f.order_id=o.id \
                         WHERE o.account_id=$1 ORDER BY o.order_ref",
                )
                .bind(account_id)
                .fetch_all(&pools.inspection)
                .await
                .unwrap_or_else(|_| panic!("preexisting committed order/fill identities unavailable"));
                assert_eq!(prior_order_refs.len(), 1);
                assert_eq!(
                    prior_receipt_ids[0]["order_id"].as_str(),
                    Some(prior_order_refs[0].0.as_str())
                );
                assert_eq!(
                    prior_receipt_ids[0]["fill_id"].as_str(),
                    Some(prior_order_refs[0].1.as_str())
                );
                assert_eq!(
                    prior_receipt_ids[0]["order_ref"].as_str(),
                    Some(prior_order_refs[0].3.as_str())
                );
                assert_eq!(
                    prior_receipt_ids[0]["fill_ref"].as_str(),
                    Some(prior_order_refs[0].4.as_str())
                );
                let prior_ledger_rows: Vec<(String, i64, String, String, String, Option<String>)> =
                    sqlx::query_as(
                        "SELECT id::text,seq,event_type,to_char(amount,'FM99999999999990.0000'), \
                                to_char(balance,'FM99999999999990.0000'),reference_id::text \
                         FROM public.cash_ledger WHERE account_id=$1 ORDER BY seq",
                    )
                    .bind(account_id)
                    .fetch_all(&pools.inspection)
                    .await
                    .unwrap_or_else(|_| panic!("preexisting committed ledger identities unavailable"));
                assert_eq!(prior_ledger_rows.len(), 2);
                assert_eq!(
                    prior_ledger_rows
                        .iter()
                        .filter(|row| row.2 == "DEPOSIT")
                        .count(),
                    1
                );
                assert_eq!(
                    prior_ledger_rows
                        .iter()
                        .filter(|row| row.2 != "DEPOSIT")
                        .count(),
                    1
                );
                let prior_outbox_id_readback: String = sqlx::query_scalar(
                    "SELECT id::text FROM public.paper_settlement_outbox WHERE pending_target_id=$1",
                )
                .bind(prior_target_id)
                .fetch_one(&pools.inspection)
                .await
                .unwrap_or_else(|_| panic!("preexisting committed outbox identity unavailable"));
                assert_eq!(prior_outbox_id_readback, prior_outbox_id);
                let prior_equity_count: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM public.daily_equity WHERE account_id=$1 AND trading_date=$2",
                )
                .bind(account_id)
                .bind(date)
                .fetch_one(&pools.inspection)
                .await
                .unwrap_or_else(|_| panic!("pre-retry valuation count unavailable"));
                assert_eq!(prior_equity_count, 0);

                let report = worker_cycle(&pools, date).await;
                assert!(report.owner_beta_preview_idle, "due progress defers fresh preview work");
                assert_eq!(report.owner_beta_previews_published, 0);
                assert_eq!(report.owner_beta_targets_seen, 1);
                assert_eq!(report.owner_beta_targets_settled, 0);
                assert_eq!(report.owner_beta_valuations_written, 1);
                let (target_id, state, kind, origin, settled, valued, settlement_sha, receipt): (
                    Uuid,
                    String,
                    Option<String>,
                    String,
                    bool,
                    bool,
                    Option<String>,
                    serde_json::Value,
                ) = sqlx::query_as(
                    "SELECT s.pending_target_id,s.state,s.settlement_kind,s.processing_origin, \
                                s.settlement_sha256 IS NOT NULL,s.valuation_sha256 IS NOT NULL, \
                                s.settlement_sha256,s.receipt_json \
                         FROM public.owner_beta_paper_sessions s \
                         WHERE s.account_id=$1 AND s.effective_session=$2",
                )
                .bind(account_id)
                .bind(date)
                .fetch_one(&pools.inspection)
                .await
                .unwrap_or_else(|_| panic!("E settlement readback failed"));
                assert_eq!(state, "VALUED");
                assert_eq!(kind.as_deref(), Some("TRADE"));
                assert_eq!(origin, "MANUAL");
                assert!(settled && valued);
                assert_eq!(target_id, prior_target_id);
                assert_eq!(settlement_sha, prior_settlement_sha);
                assert_eq!(receipt["settlement"], prior_receipt["settlement"]);
                assert_eq!(receipt["order_fill_ids"], prior_receipt["order_fill_ids"]);
                assert_eq!(
                    receipt["outbox_id"].as_str(),
                    Some(prior_outbox_id.as_str())
                );
                let (orders, fills, deposits, deposit_amount, outbox, equity):
                    (i64, i64, i64, String, i64, i64) =
                    sqlx::query_as(
                        "SELECT \
                           (SELECT count(*) FROM public.orders WHERE account_id=(SELECT account_id FROM public.owner_beta_paper_control WHERE id='etf11')), \
                           (SELECT count(*) FROM public.fills WHERE account_id=(SELECT account_id FROM public.owner_beta_paper_control WHERE id='etf11')), \
                           (SELECT count(*) FROM public.cash_ledger WHERE account_id=(SELECT account_id FROM public.owner_beta_paper_control WHERE id='etf11') AND event_type='DEPOSIT'), \
                           (SELECT to_char(COALESCE(sum(amount),0),'FM99999999999990.0000') FROM public.cash_ledger WHERE account_id=(SELECT account_id FROM public.owner_beta_paper_control WHERE id='etf11') AND event_type='DEPOSIT'), \
                           (SELECT count(*) FROM public.paper_settlement_outbox WHERE pending_target_id=$1), \
                           (SELECT count(*) FROM public.daily_equity WHERE account_id=(SELECT account_id FROM public.owner_beta_paper_control WHERE id='etf11') AND trading_date=$2)",
                    )
                    .bind(target_id)
                    .bind(date)
                    .fetch_one(&pools.inspection)
                    .await
                    .unwrap_or_else(|_| panic!("E economic readback failed"));
                assert_eq!(
                    (orders, fills),
                    (1, 1),
                    "the fixed high-price fixture produces one ETF order and fill"
                );
                assert_eq!((deposits, outbox, equity), (1, 1, 1));
                assert_eq!(
                    deposit_amount, "10000000.0000",
                    "only the configured initial cash deposit exists"
                );
                let non_deposit_ledger_rows: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM public.cash_ledger WHERE account_id=(SELECT account_id FROM public.owner_beta_paper_control WHERE id='etf11') AND event_type<>'DEPOSIT'",
                )
                .fetch_one(&pools.inspection)
                .await
                .unwrap_or_else(|_| panic!("settlement ledger row count unavailable"));
                assert_eq!(
                    non_deposit_ledger_rows, 1,
                    "one actual order produces one cash-ledger entry"
                );
                let receipt_ids = receipt["order_fill_ids"]
                    .as_array()
                    .expect("SQL settlement records exact order/fill identities");
                assert_eq!(orders, fills, "each Paper order has exactly one fill");
                assert_eq!(receipt_ids.len() as i64, orders);
                let target = PendingTarget {
                    account_id,
                    effective_date: TradingDate::parse(&date.format("%Y-%m-%d").to_string())
                        .expect("fixed settlement session date"),
                    targets: Vec::new(),
                };
                let instrument = InstrumentId::parse("069500.KRX").expect("fixed ETF instrument");
                let persisted_refs: Vec<(String, String, String, String, String)> = sqlx::query_as(
                    "SELECT o.id::text,f.id::text,o.side::text,o.order_ref,f.fill_ref \
                     FROM public.orders o JOIN public.fills f ON f.order_id=o.id \
                     WHERE o.account_id=$1 ORDER BY o.order_ref",
                )
                .bind(account_id)
                .fetch_all(&pools.inspection)
                .await
                .unwrap_or_else(|_| {
                    panic!("committed deterministic order/fill references unavailable")
                });
                assert_eq!(persisted_refs.len() as i64, orders);
                assert_eq!(persisted_refs, prior_order_refs);
                for (persisted, receipt_id) in persisted_refs.iter().zip(receipt_ids) {
                    let side = match persisted.2.as_str() {
                        "BUY" => Side::Buy,
                        "SELL" => Side::Sell,
                        _ => panic!("committed Paper side is outside the closed contract"),
                    };
                    assert_eq!(receipt_id["order_id"].as_str(), Some(persisted.0.as_str()));
                    assert_eq!(receipt_id["fill_id"].as_str(), Some(persisted.1.as_str()));
                    assert_eq!(receipt_id["order_ref"].as_str(), Some(persisted.3.as_str()));
                    assert_eq!(receipt_id["fill_ref"].as_str(), Some(persisted.4.as_str()));
                    assert_eq!(
                        persisted.3,
                        target.order_id_for(&instrument, side).to_string()
                    );
                    assert_eq!(
                        persisted.4,
                        target.fill_id_for(&instrument, side).to_string()
                    );
                }
                let ledger_rows: Vec<(String, i64, String, String, String, Option<String>)> =
                    sqlx::query_as(
                        "SELECT id::text,seq,event_type,to_char(amount,'FM99999999999990.0000'), \
                                to_char(balance,'FM99999999999990.0000'),reference_id::text \
                         FROM public.cash_ledger WHERE account_id=$1 ORDER BY seq",
                    )
                    .bind(account_id)
                    .fetch_all(&pools.inspection)
                    .await
                    .unwrap_or_else(|_| panic!("committed ledger identities unavailable"));
                assert_eq!(ledger_rows, prior_ledger_rows);
                let outbox_id: String = sqlx::query_scalar(
                    "SELECT id::text FROM public.paper_settlement_outbox WHERE pending_target_id=$1",
                )
                .bind(target_id)
                .fetch_one(&pools.inspection)
                .await
                .unwrap_or_else(|_| panic!("committed outbox identity unavailable"));
                assert_eq!(outbox_id, prior_outbox_id);
                eprintln!(
                    "R5_PAPER_NATIVE phase=settle_e classification=SYNTHETIC_TEST_AUTHORITY_NOT_ORIGIN actual_worker=true outcome=TRADE targets_seen=1 settled=0 valued=1 total_settlements=1 orders={} fills={} deposit_rows=1 deposit_amount=10000000.0000 shared_uuid5_refs=true outbox_rows=1 daily_equity_rows=1",
                    orders, fills
                );
            }
            "enable_auto" => {
                fixture_auto_setup(&pools, true).await;
                let date = chrono::NaiveDate::parse_from_str(&env("R5_PAPER_PROCESS_DATE"), "%Y-%m-%d")
                    .expect("fixed AUTO signal date");
                let report = worker_cycle(&pools, date).await;
                assert_eq!(report.owner_beta_previews_published, 1);
                let next_session = chrono::NaiveDate::parse_from_str("2026-09-04", "%Y-%m-%d")
                    .expect("fixed following session");
                let (state, origin, source_kind): (String, String, String) = sqlx::query_as(
                    "SELECT s.state,s.processing_origin,t.source_kind \
                     FROM public.owner_beta_paper_sessions s \
                     JOIN public.pending_targets t ON t.id=s.pending_target_id \
                     WHERE s.account_id=(SELECT account_id FROM public.owner_beta_paper_control WHERE id='etf11') \
                       AND s.effective_session=$1",
                )
                .bind(next_session)
                .fetch_one(&pools.inspection)
                .await
                .unwrap_or_else(|_| panic!("AUTO target readback failed"));
                assert_eq!(
                    (state.as_str(), origin.as_str(), source_kind.as_str()),
                    ("PENDING", "AUTO", "OWNER_BETA_DAILY_AUTO")
                );
                let preview_count: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM public.paper_rebalance_previews \
                     WHERE account_id=(SELECT account_id FROM public.owner_beta_paper_control WHERE id='etf11') \
                       AND input_kind='OWNER_BETA_DAILY_V1' AND beta_apply_mode='AUTO' \
                       AND price_date=$1 AND status='APPLIED'",
                )
                .bind(date)
                .fetch_one(&pools.inspection)
                .await
                .unwrap_or_else(|_| panic!("AUTO preview readback failed"));
                assert_eq!(preview_count, 1);
                let previous_target: Uuid = sqlx::query_scalar(
                    "SELECT pending_target_id FROM public.owner_beta_paper_sessions \
                     WHERE account_id=(SELECT account_id FROM public.owner_beta_paper_control WHERE id='etf11') \
                       AND effective_session='2026-09-03'",
                )
                .fetch_one(&pools.inspection)
                .await
                .unwrap_or_else(|_| panic!("previous target identity unavailable"));
                let previous_delivered: bool = sqlx::query_scalar(
                    "SELECT delivered_at IS NOT NULL FROM public.paper_settlement_outbox WHERE pending_target_id=$1",
                )
                .bind(previous_target)
                .fetch_one(&pools.inspection)
                .await
                .unwrap_or_else(|_| panic!("prior settlement notification readback failed"));
                assert!(
                    previous_delivered,
                    "the restarted worker cycle delivers the prior outbox"
                );
                eprintln!(
                    "R5_PAPER_NATIVE phase=enable_auto classification=SYNTHETIC_TEST_AUTHORITY_NOT_ORIGIN fixture_only_auto_setup=true auto_preview_published=1 next_session_target_pending=true prior_outbox_delivered=true"
                );
            }
            "next_session" => {
                let date = chrono::NaiveDate::parse_from_str(&env("R5_PAPER_PROCESS_DATE"), "%Y-%m-%d")
                    .expect("fixed following-session date");
                let report = worker_cycle(&pools, date).await;
                assert!(report.owner_beta_preview_idle, "due progress defers fresh preview work");
                assert_eq!(report.owner_beta_previews_published, 0);
                assert_eq!(report.owner_beta_targets_seen, 1);
                assert_eq!(report.owner_beta_targets_settled, 1);
                assert_eq!(report.owner_beta_valuations_written, 1);
                let (target_id, state, kind, origin, settled, valued, receipt):
                    (Uuid, String, Option<String>, String, bool, bool, serde_json::Value) =
                    sqlx::query_as(
                        "SELECT s.pending_target_id,s.state,s.settlement_kind,s.processing_origin, \
                                s.settlement_sha256 IS NOT NULL,s.valuation_sha256 IS NOT NULL,s.receipt_json \
                         FROM public.owner_beta_paper_sessions s \
                         WHERE s.account_id=(SELECT account_id FROM public.owner_beta_paper_control WHERE id='etf11') \
                           AND s.effective_session=$1",
                    )
                    .bind(date)
                    .fetch_one(&pools.inspection)
                    .await
                    .unwrap_or_else(|_| panic!("next-session settlement readback failed"));
                assert_eq!(state, "VALUED");
                assert_eq!(kind.as_deref(), Some("NO_TRADE"));
                assert_eq!(origin, "AUTO");
                assert!(settled && valued);
                assert_eq!(
                    receipt["order_fill_ids"].as_array().map(Vec::len),
                    Some(0),
                    "unchanged signal creates no order or fill"
                );
                let (deposit_count, deposit_amount, ledger_count, outbox_count, equity_count, no_trade_events, valued_events):
                    (i64, String, i64, i64, i64, i64, i64) = sqlx::query_as(
                        "SELECT \
                           (SELECT count(*) FROM public.cash_ledger WHERE account_id=(SELECT account_id FROM public.owner_beta_paper_control WHERE id='etf11') AND event_type='DEPOSIT'), \
                           (SELECT to_char(COALESCE(sum(amount),0),'FM99999999999990.0000') FROM public.cash_ledger WHERE account_id=(SELECT account_id FROM public.owner_beta_paper_control WHERE id='etf11') AND event_type='DEPOSIT'), \
                           (SELECT count(*) FROM public.cash_ledger WHERE account_id=(SELECT account_id FROM public.owner_beta_paper_control WHERE id='etf11') AND event_type<>'DEPOSIT'), \
                           (SELECT count(*) FROM public.paper_settlement_outbox WHERE pending_target_id=$1), \
                           (SELECT count(*) FROM public.daily_equity WHERE account_id=(SELECT account_id FROM public.owner_beta_paper_control WHERE id='etf11') AND trading_date=$2), \
                           (SELECT count(*) FROM public.owner_beta_paper_session_events WHERE pending_target_id=$1 AND kind='SETTLED_NO_TRADE'), \
                           (SELECT count(*) FROM public.owner_beta_paper_session_events WHERE pending_target_id=$1 AND kind='VALUED')",
                    )
                    .bind(target_id)
                    .bind(date)
                    .fetch_one(&pools.inspection)
                    .await
                    .unwrap_or_else(|_| panic!("NO_TRADE settlement evidence readback failed"));
                assert_eq!(
                    (
                        deposit_count,
                        outbox_count,
                        equity_count,
                        no_trade_events,
                        valued_events
                    ),
                    (1, 1, 1, 1, 1)
                );
                assert_eq!(
                    ledger_count, 1,
                    "NO_TRADE does not write another ledger entry"
                );
                assert_eq!(
                    deposit_amount, "10000000.0000",
                    "NO_TRADE does not top up cash"
                );
                eprintln!(
                    "R5_PAPER_NATIVE phase=next_session classification=SYNTHETIC_TEST_AUTHORITY_NOT_ORIGIN actual_worker=true outcome=NO_TRADE targets_seen=1 settled=1 valued=1 orders=0 fills=0 deposit_rows=1 outbox_rows=1 daily_equity_rows=1"
                );
            }
            _ => panic!("sealed workflow phase is invalid"),
        }
    }).catch_unwind().await;
    finish_fixture_phase(original, pools.close()).await;
    progress(ProgressStage::PhaseEnd);
}

// Old-E control uses exact canonical row text so decimal precision is not lost
// through JSON float decoding. Only documented wait-state fields are separated;
// all account/cash/position/receipt/outbox/target/financial rows stay exact.
type OldEconomicImage = (
    Result<std::collections::BTreeMap<&'static str, Vec<String>>, sqlx::Error>,
    Option<Result<(), sqlx::Error>>,
);
fn old_economic_value(
    image: &OldEconomicImage,
) -> &std::collections::BTreeMap<&'static str, Vec<String>> {
    match image {
        (Ok(rows), Some(Ok(()))) => rows,
        _ => panic!("OLD_E_EXACT_IMAGE_OR_ROLLBACK_FAILED"),
    }
}
async fn old_economic_rows(pools: &FixturePools) -> OldEconomicImage {
    let account = Uuid::parse_str(&env("R5_PAPER_ACCOUNT_ID")).expect("real old-E account");
    let mut tx = match pools.inspection.begin().await {
        Ok(tx) => tx,
        Err(error) => return (Err(error), None),
    };
    let original=async {
        sqlx::raw_sql("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY").execute(&mut *tx).await?;
        job_queue::paper_execution::set_paper_transaction_timeouts(&mut tx).await?;
        let mut result=std::collections::BTreeMap::new();
        for (name,query) in [
            ("accounts","SELECT to_jsonb(x)::text FROM public.accounts x WHERE id=$1"),
            ("positions","SELECT to_jsonb(x)::text FROM public.positions x WHERE account_id=$1"),
            ("cash","SELECT to_jsonb(x)::text FROM public.cash_ledger x WHERE account_id=$1"),
            ("orders","SELECT to_jsonb(x)::text FROM public.orders x WHERE account_id=$1"),
            ("fills","SELECT to_jsonb(x)::text FROM public.fills x WHERE account_id=$1"),
            ("equity","SELECT to_jsonb(x)::text FROM public.daily_equity x WHERE account_id=$1"),
            ("targets","SELECT to_jsonb(x)::text FROM public.pending_targets x WHERE account_id=$1"),
            ("lineage","SELECT to_jsonb(x)::text FROM public.owner_beta_paper_lineage x WHERE account_id=$1"),
            ("sessions","SELECT (to_jsonb(x)-'state'-'reason_code'-'updated_at')::text FROM public.owner_beta_paper_sessions x WHERE account_id=$1"),
            ("outbox","SELECT to_jsonb(x)::text FROM public.paper_settlement_outbox x WHERE pending_target_id IN (SELECT id FROM public.pending_targets WHERE account_id=$1)"),
        ] {
            let mut rows:Vec<String>=sqlx::query_scalar(query).bind(account).fetch_all(&mut *tx).await?;
            rows.sort(); result.insert(name,rows);
        }
        Ok::<_,sqlx::Error>(result)
    }.await;
    let cleanup = tx.rollback().await;
    (original, Some(cleanup))
}


#[tokio::test(flavor = "current_thread")]
#[ignore = "NOT_STAGED: genuine retained RECOVER grant, original T/run/E Source/Raw/sealed Daily, current protected API/worker ELF/Current and existing isolated fixture pools; invoke once for each grant step"]
async fn owner_beta_paper_recovery_real_worker_cycle_consumes_once() {
    let pools=FixturePools::open().await;
    let original=std::panic::AssertUnwindSafe(async {
        let account=Uuid::parse_str(&env("R5_PAPER_ACCOUNT_ID")).expect("actual fixture account");
        let (target,step,old_epoch,current_epoch,first_after_e,original_settlement):(Uuid,String,i64,i64,bool,Option<String>)=sqlx::query_as(
            "SELECT g.pending_target_id,g.step,l.control_epoch,c.control_epoch,d.first_auto_session>s.effective_session,s.settlement_sha256 FROM owner_beta_paper_recovery_grants g JOIN owner_beta_paper_lineage l ON l.id=g.lineage_id JOIN owner_beta_paper_sessions s ON s.pending_target_id=g.pending_target_id JOIN owner_beta_paper_control c ON c.account_id=g.account_id JOIN owner_beta_daily_control d ON d.owner_user_id=c.owner_user_id WHERE g.account_id=$1 AND g.active_epoch=c.control_epoch AND g.revoked_at IS NULL AND g.valuation_receipt_sha256 IS NULL ORDER BY s.effective_session LIMIT 1",
        ).bind(account).fetch_one(&pools.inspection).await.expect("genuine committed grant and originals already staged");
        assert!(old_epoch<current_epoch && first_after_e);
        assert!(matches!(step.as_str(),"SETTLE_THEN_VALUE"|"VALUE_ONLY"));
        let before=old_economic_rows(&pools).await;
        let before=old_economic_value(&before).clone();
        let today:chrono::NaiveDate=sqlx::query_scalar("SELECT (clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date")
            .fetch_one(&pools.inspection).await.unwrap();
        // Actual existing runner -> private target decoder -> checked retained
        // T/E capture -> existing financial algorithms -> SQL -> checked COMMIT.
        let report=worker_cycle(&pools,today).await;
        assert!(report.item_errors.is_empty(),"failed/uncertain first cycle is never resubmitted");
        let (settlement,valuation,outbox,equity):(String,String,i64,i64)=sqlx::query_as(
            "SELECT s.settlement_sha256,s.valuation_sha256,(SELECT count(*) FROM paper_settlement_outbox o WHERE o.pending_target_id=s.pending_target_id),(SELECT count(*) FROM daily_equity e WHERE e.account_id=s.account_id AND e.trading_date=s.effective_session) FROM owner_beta_paper_sessions s WHERE s.pending_target_id=$1 AND s.state='VALUED'",
        ).bind(target).fetch_one(&pools.inspection).await.expect("both receipts durably read back before any second cycle");
        assert!(!valuation.is_empty());assert_eq!((outbox,equity),(1,1));
        let after=old_economic_rows(&pools).await;
        let after=old_economic_value(&after).clone();
        for original_lineage in &before["lineage"] {
            assert!(after["lineage"].contains(original_lineage),"every old lineage remains byte-identical; normal future preview work may add a lineage");
        }
        if step=="VALUE_ONLY" {
            assert_eq!(Some(settlement),original_settlement);
            for key in ["cash","positions","orders","fills","outbox"] {assert_eq!(before[key],after[key]);}
        }
        // Fixture requires a quiet, sealed Source set throughout both cycles:
        // no external admission, delivery or other due target is introduced.
        // This is an explicit post-success duplicate-prevention test, never
        // an automatic retry of an uncertain COMMIT.
        let replay=worker_cycle_replay(&pools,today).await;
        assert!(replay.item_errors.is_empty());
        let repeated=old_economic_rows(&pools).await;
        assert_eq!(after,*old_economic_value(&repeated),"no second settlement/value/outbox or delivery resend");
    }).catch_unwind().await;
    pools.close().await;
    if let Err(payload)=original {std::panic::resume_unwind(payload);}
}
