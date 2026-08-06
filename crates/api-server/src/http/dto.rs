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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<RecommendationItemDto>>,
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
pub struct RobustnessDto {
    pub run_id: String,
    pub job_id: String,
    pub status: &'static str,
}

// ---------------------------------------------------------------------------
// Paper
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct AccountDto {
    pub id: String,
    pub account_type: String,
    pub name: String,
    pub currency: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BindStrategyDto {
    pub account_id: String,
    pub strategy_config_id: String,
    pub bound_at: DateTime<Utc>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewAccountBody {
    pub name: String,
    pub currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindStrategyBody {
    pub strategy_config_id: String,
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
