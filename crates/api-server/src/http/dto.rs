//! Wire DTOs of the `/api/v1` contract. These are deliberately NOT database,
//! NT, or provider models: they are the documented contract shapes, mapped
//! from repository rows with no tenant/ownership or internal path leakage.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct StrategyDto {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub risk_description: String,
    pub state: String,
    pub latest_version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StrategyConfigDto {
    pub id: String,
    pub strategy_id: String,
    pub strategy_version: String,
    pub config: Value,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Recommendations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct RecommendationRunDto {
    pub id: String,
    pub strategy_config_id: Option<String>,
    pub as_of: NaiveDate,
    pub status: String,
    pub summary: Value,
    pub created_at: DateTime<Utc>,
    pub trigger_kind: String,
    pub provenance: RecommendationProvenanceDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<RecommendationItemDto>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecommendationProvenanceDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dataset_version_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dataset_manifest_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecommendationItemDto {
    pub instrument_id: String,
    pub rank: Option<i32>,
    pub target_weight: Option<String>,
    pub excluded: bool,
    pub exclusion_reason: Option<String>,
    pub reason_codes: Vec<String>,
    pub factors: Value,
}

// ---------------------------------------------------------------------------
// Backtests
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct BacktestRunDto {
    pub id: String,
    pub owner_user_id: String,
    pub can_manage: bool,
    pub strategy_id: String,
    pub strategy_version: String,
    pub dataset_version: String,
    pub engine: String,
    pub engine_version: String,
    pub status: String,
    pub job_id: Option<String>,
    pub config_sha256: String,
    pub benchmark: Option<String>,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub summary: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct MetricDto {
    pub metric_key: String,
    pub metric_value: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct WarningDto {
    pub warning_code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtifactDto {
    pub id: String,
    pub run_id: String,
    pub artifact_type: String,
    pub row_count: i64,
    pub sha256: String,
    pub size_bytes: i64,
    pub summary: Value,
    pub download_path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EquityDto {
    pub run_id: String,
    pub artifact: ArtifactDto,
    pub summary: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct TradeDto {
    pub run_id: String,
    pub artifact_type: String,
    pub artifact: ArtifactDto,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompareRunDto {
    pub run_id: String,
    pub strategy_id: String,
    pub status: String,
    pub summary: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompareDto {
    pub run_ids: Vec<String>,
    pub runs: Vec<CompareRunDto>,
    pub deltas: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct CancelDto {
    pub run_id: String,
    pub job_id: Option<String>,
    pub status: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct RobustnessChildDto {
    pub run_id: String,
    pub job_id: String,
    pub axis: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RobustnessDto {
    pub run_id: String,
    pub suite_id: String,
    pub children: Vec<RobustnessChildDto>,
}

// ---------------------------------------------------------------------------
// Paper
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct AccountDto {
    pub id: String,
    pub owner_user_id: String,
    pub can_manage: bool,
    pub account_type: String,
    pub name: String,
    pub currency: String,
    pub status: String,
    pub initial_cash: Option<String>,
    pub cost_profile_id: String,
    pub cost_profile_version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BindStrategyDto {
    pub account_id: String,
    pub strategy_config_id: String,
    pub strategy_id: String,
    pub strategy_version: String,
    pub auto_apply_recommendations: bool,
    pub bound_at: DateTime<Utc>,
}

/// One day of ledger-derived performance. `equity`/`cash`/`positions_value`
/// are read straight from `daily_equity` (which the runner writes from the
/// shared ledger); `return_pct` is the day-over-day change computed on
/// read, never stored — there is no second source of truth to drift.
#[derive(Debug, Clone, Serialize)]
pub struct PerformancePointDto {
    pub trading_date: chrono::NaiveDate,
    pub equity: String,
    pub cash: String,
    pub positions_value: String,
    pub currency: String,
    /// Day-over-day return as a decimal string; absent on the first point.
    pub return_pct: Option<String>,
    /// Whether `cash` agrees with `cash_ledger`, the authority, as of this
    /// date. `false` means this is a stored figure nobody has proven agrees
    /// with the ledger -- served, not hidden, but not presented as settled.
    pub cash_reconciled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PerformanceDto {
    pub account_id: String,
    pub points: Vec<PerformancePointDto>,
    /// Rendered verbatim by the UI. Paper results are simulated and are
    /// never a promise of future returns (design §10.2's reporting duty).
    pub disclaimer: &'static str,
}

/// One entry of the account's strategy-binding history (Todo 30). The
/// history is immutable: a rebind closes the old row and opens a new one,
/// so this is the account's full branching lineage.
#[derive(Debug, Clone, Serialize)]
pub struct BindingHistoryDto {
    pub strategy_config_id: String,
    pub strategy_id: String,
    pub strategy_version: String,
    pub auto_apply_recommendations: bool,
    pub bound_at: DateTime<Utc>,
    pub unbound_at: Option<DateTime<Utc>>,
    pub active: bool,
}

/// One queued/executed target (Todo 31), correlating a close(T) computation
/// with the session it executed at.
#[derive(Debug, Clone, Serialize)]
pub struct TargetLineageDto {
    pub id: String,
    pub computed_on: chrono::NaiveDate,
    pub effective_date: chrono::NaiveDate,
    pub status: String,
    pub executed_at: Option<DateTime<Utc>>,
    pub non_execution_reason: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RebalancePreviewDto {
    pub id: String,
    pub account_id: String,
    pub recommendation_run_id: String,
    pub target_portfolio_id: String,
    pub strategy_config_id: String,
    pub job_id: String,
    pub status: String,
    pub price_basis: String,
    pub price_date: NaiveDate,
    pub proposed_effective_date: Option<NaiveDate>,
    pub dataset_version_id: String,
    pub dataset_manifest_sha256: String,
    pub target_portfolio_sha256: String,
    pub preview_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<job_queue::paper_preview::PreviewResultV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RebalancePreviewErrorDto>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub applied_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RebalancePreviewErrorDto {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LineageDto {
    pub account_id: String,
    pub bindings: Vec<BindingHistoryDto>,
    pub targets: Vec<TargetLineageDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ParityDto {
    pub account_id: String,
    pub as_of: String,
    pub status: String,
    pub lineage: Value,
    pub divergences: Value,
    pub fill_model_difference: String,
    /// True when the report is worth a WARNING-grade alert (design §15.3).
    pub warrants_alert: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct OrderDto {
    pub id: String,
    pub order_ref: String,
    pub instrument_id: String,
    pub side: String,
    pub quantity: String,
    pub price: Option<String>,
    pub status: String,
    pub submitted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PositionDto {
    pub instrument_id: String,
    pub quantity: String,
    pub avg_price: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EquityPointDto {
    pub trading_date: NaiveDate,
    pub equity: String,
    pub cash: String,
    pub positions_value: String,
    pub currency: String,
    /// Whether `cash` agrees with `cash_ledger`, the authority, as of this
    /// date. See `PerformancePointDto::cash_reconciled`.
    pub cash_reconciled: bool,
}

// ---------------------------------------------------------------------------
// Admin / ops
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct IssueDto {
    pub issue_code: String,
    pub severity: String,
    pub detail: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdminDatasetDto {
    pub id: String,
    pub dataset_id: String,
    pub version: String,
    pub status: String,
    pub manifest_sha256: String,
    pub created_at: DateTime<Utc>,
    pub blocking_issues: Vec<IssueDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JobDto {
    pub id: String,
    pub job_type: String,
    pub status: String,
    pub priority: i32,
    pub idempotency_key: Option<String>,
    pub attempt_count: i32,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkerDto {
    pub worker_id: String,
    pub last_heartbeat_at: Option<DateTime<Utc>>,
    pub active_job_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdminUserDto {
    pub id: String,
    pub email: String,
    pub roles: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuditDto {
    pub id: String,
    pub action: String,
    pub actor_role: String,
    pub actor_user_id: Option<String>,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub reason: Option<String>,
    pub correlation_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DatasetVerdictDto {
    pub dataset_id: String,
    pub version: String,
    pub status: String,
    pub verdict: &'static str,
    pub reason: String,
}

// ---------------------------------------------------------------------------
// Licensing / auth
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct LicensingDatasetDto {
    pub dataset_id: String,
    pub use_kind: String,
    pub state: String,
    pub effective_from: Option<NaiveDate>,
    pub effective_until: Option<NaiveDate>,
    pub covered: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LicensingStatusDto {
    pub as_of: NaiveDate,
    pub datasets: Vec<LicensingDatasetDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionDto {
    pub user_id: String,
    pub role: &'static str,
    pub expires_at_secs: i64,
    pub auth_time_secs: i64,
    /// Non-secret deployment policy used by the Web as defense in depth.
    /// The API middleware remains the authoritative admission boundary.
    pub owner_beta_access_mode: crate::http::state::OwnerBetaAccessMode,
    /// Separate Paper activation inside an Owner-only beta.
    pub owner_beta_paper_mode: crate::http::state::OwnerBetaPaperMode,
}

// ---------------------------------------------------------------------------
// Notifications / alerts
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct NotificationDto {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    /// Every attempt made to deliver this notification. A failed channel is
    /// carried here with its error detail so an outage is visible in the
    /// feed itself (FR-RPT-002) rather than only in the Owner's admin view.
    pub deliveries: Vec<NotificationDeliveryDto>,
}

/// One delivery attempt on a notification the actor owns.
#[derive(Debug, Clone, Serialize)]
pub struct NotificationDeliveryDto {
    pub channel: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubscriptionDto {
    pub kind: String,
    pub channel: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeliveryOutcomeDto {
    pub notification_id: String,
    pub channel: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TestNotificationResult {
    pub notifications: Vec<String>,
    pub deliveries: Vec<DeliveryOutcomeDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdminDeliveryDto {
    pub notification_id: String,
    pub owner_user_id: String,
    pub channel: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_detail: Option<String>,
    pub attempted_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Request bodies (deny_unknown_fields => typed 400 INVALID_PARAMETER)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewStrategyConfigBody {
    pub strategy_version: String,
    pub config: Value,
    #[serde(default = "default_true")]
    pub is_active: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecommendationRunBody {
    pub strategy_config_id: String,
    pub as_of: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BacktestBody {
    pub strategy_config_id: String,
    pub dataset_version_id: String,
    pub start_date: String,
    pub end_date: String,
    pub initial_cash: CashBody,
    pub benchmark: String,
    pub cost_profile_id: String,
    pub execution_profile: String,
    #[serde(default)]
    pub robustness: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CashBody {
    pub currency: String,
    pub amount: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompareBody {
    pub run_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmptyBody {}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HoldoutSpec {
    pub train_end: String,
    pub validation_end: String,
}

/// A robustness-suite creation request: one axis change per requested
/// child. `axis` reuses `result_model::robustness::DerivedAxis`'s own wire
/// shape (internally tagged on `"axis"`), so the request body IS the same
/// shape the crate already validates against — no parallel DTO to drift.
/// `axes` defaults to the documented standard cost-stress pair when omitted
/// (the product's zero-configuration "Run robustness evidence" button never
/// selects axes itself).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RobustnessSuiteBody {
    #[serde(default)]
    pub axes: Option<Vec<result_model::robustness::DerivedAxis>>,
    #[serde(default)]
    pub holdout: Option<HoldoutSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewAccountBody {
    pub name: String,
    pub currency: String,
    /// A decimal string; PAPER accounts must open positively funded.
    pub initial_cash: String,
    /// Defaults to the versioned `KRX_ETF_DEFAULT` profile when omitted.
    #[serde(default)]
    pub cost_profile_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindStrategyBody {
    pub strategy_config_id: String,
    #[serde(default)]
    pub auto_apply_recommendations: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RebalancePreviewBody {
    pub recommendation_run_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyRebalancePreviewBody {
    pub preview_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppliedRebalancePreviewDto {
    pub preview_id: String,
    pub pending_target_id: String,
    pub effective_date: NaiveDate,
    pub source_kind: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubscriptionBody {
    pub kind: String,
    pub channel: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestNotificationBody {
    pub severity: String,
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
}

// ---------------------------------------------------------------------------
// Pagination wrapper
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct PageDto<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

impl<T> PageDto<T> {
    pub fn new(items: Vec<T>, next_cursor: Option<String>) -> Self {
        let has_more = next_cursor.is_some();
        Self {
            items,
            next_cursor,
            has_more,
        }
    }
}
