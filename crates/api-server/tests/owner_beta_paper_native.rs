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
use portfolio_model::{PendingTarget, Side};
use sqlx::{
    ConnectOptions, PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
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
];

fn lifecycle_progress_stage(code: &str) -> Option<ProgressStage> {
    match code {
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
    emit: Arc<dyn Fn(ProgressStage) + Send + Sync>,
}

impl LifecycleSubscriber {
    fn new(emit: Arc<dyn Fn(ProgressStage) + Send + Sync>) -> Self {
        Self {
            emitted: AtomicUsize::new(0),
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
        if let Some(stage) = visitor.stage
            && self
                .emitted
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                    (count < MAX_ENGINE_EVENTS).then_some(count + 1)
                })
                .is_ok()
        {
            (self.emit)(stage);
        }
    }
}

#[cfg(test)]
mod lifecycle_diagnostic_tests {
    use super::*;
    use std::sync::Mutex;

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

async fn write_worker_backend_pid(pools: &FixturePools) -> i32 {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let (pid, actor) = worker_backend_identity(pools).await;
    assert_eq!(
        actor, None,
        "actual worker login starts with app.actor_user_id unset"
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
        actor.as_deref().is_none_or(str::is_empty),
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
        actor.as_deref().is_none_or(str::is_empty),
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
        Duration::from_secs(2),
        Duration::from_secs(60),
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
    progress(ProgressStage::WorkerCycleBegin);
    let worker_backend_pid = write_worker_backend_pid(pools).await;
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
        .unwrap_or_else(|_| {
            progress(ProgressStage::CycleReturnErr);
            panic!("actual Paper worker cycle failed")
        });
    record_worker_item_errors(&report);
    assert_worker_backend_unset(pools, worker_backend_pid).await;
    assert_queue_worker_unset(pools).await;
    progress(ProgressStage::WorkerCycleEnd);
    report
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
                    .unwrap_or_else(|_| {
                        progress(ProgressStage::CycleReturnErr);
                        panic!("actual Paper worker cycle failed")
                    });
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
