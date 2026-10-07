//! Owner-only HTTP contract for the managed equity universe V2.
//!
//! DTOs intentionally whitelist lifecycle, policy, coverage, and price/volume
//! research fields.  Raw evidence, provider prose, entitlement references,
//! lineage paths, and credential/account/order concepts never cross this
//! boundary.

use std::collections::BTreeSet;

use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Datelike, Months, NaiveDate, Utc};
use collectors::owner_equity_v2::artifact::{
    OwnerEquityArtifactError, VerifiedOwnerEquityArtifact, read_owner_equity_artifact,
};
use domain::ContentHash;
use market_data::owner_equity_v2::PRICE_SEMANTICS;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::PathBuf;
use uuid::Uuid;

use crate::http::JsonBody;
use crate::http::equity_signals::EquitySignalsCondition;
use crate::http::error::{api_error, code_error, request_id};
use crate::http::idempotency;
use crate::http::session::{Session, require_csrf};
use crate::http::state::ApiState;
use crate::repos::owner_equity_v2::{
    OwnerEquityChartDescriptor, OwnerEquityLatestSnapshot, OwnerEquityMembershipRecord,
    OwnerEquityMutationPins, OwnerEquityMutationResult, OwnerEquityPolicyRecord,
    OwnerEquityRepoError, OwnerEquitySnapshotRowRecord,
};

const MAX_CHART_BARS: usize = 261;
const CHART_WARNINGS: [&str; 3] = [
    "NOT_REALTIME",
    "CORPORATE_ACTIONS_NOT_ADJUSTED",
    "RESEARCH_ONLY",
];

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AddMembershipBody {
    pub instrument_code: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionBody {}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SignalScreenBody {
    #[serde(default)]
    pub instrument_ids: Option<Vec<String>>,
    #[serde(default)]
    pub conditions: Option<Vec<EquitySignalsCondition>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OwnerEquityPolicyDto {
    pub max_active_instruments: u32,
    pub active_instruments: u32,
    pub remaining_capacity: u32,
    pub target_observed_sessions: u32,
    pub minimum_observed_sessions: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct OwnerEquityCoverageDto {
    pub observed_sessions: u32,
    pub target_observed_sessions: u32,
    pub minimum_observed_sessions: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_session: Option<NaiveDate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_session: Option<NaiveDate>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OwnerEquityFailureDto {
    pub code: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct OwnerEquityMembershipDto {
    pub id: Uuid,
    pub instrument_id: String,
    pub lifecycle: String,
    pub generation: u64,
    pub coverage: OwnerEquityCoverageDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<OwnerEquityFailureDto>,
    pub requested_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MembershipListDto {
    pub policy: OwnerEquityPolicyDto,
    pub memberships: Vec<OwnerEquityMembershipDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MembershipStatusDto {
    pub policy: OwnerEquityPolicyDto,
    pub membership: OwnerEquityMembershipDto,
}

#[derive(Debug, Clone, Serialize)]
pub struct MembershipMutationDto {
    pub resource: OwnerEquityMembershipDto,
    pub job_id: Uuid,
    pub duplicate_active: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct OwnerEquitySnapshotDto {
    pub snapshot_id: Uuid,
    pub as_of: NaiveDate,
    pub universe_sha256: String,
    pub row_count: u32,
    pub published_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OwnerEquitySignalDto {
    pub instrument_id: String,
    pub generation: u64,
    pub rank: u32,
    pub score: f64,
    pub condition: EquitySignalsCondition,
    pub return_20: f64,
    pub return_60: f64,
    pub return_120: f64,
    pub volatility_20: f64,
    pub volatility_60: f64,
    pub volatility_120: f64,
    pub max_drawdown_120: f64,
    pub sma_20: f64,
    pub sma_60: f64,
    pub average_volume_20: f64,
    pub volume_ratio_20_60: f64,
    pub average_trading_value_20: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LatestSignalsDto {
    pub snapshot: OwnerEquitySnapshotDto,
    pub rows: Vec<OwnerEquitySignalDto>,
    pub top5: Vec<OwnerEquitySignalDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScreenSignalsDto {
    pub snapshot: OwnerEquitySnapshotDto,
    pub rows: Vec<OwnerEquitySignalDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SignalDetailDto {
    pub snapshot: OwnerEquitySnapshotDto,
    pub signal: OwnerEquitySignalDto,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OwnerEquityChartQuery {
    snapshot_id: Uuid,
    range: OwnerEquityChartRange,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
enum OwnerEquityChartRange {
    #[serde(rename = "1m")]
    OneMonth,
    #[serde(rename = "3m")]
    ThreeMonths,
    #[serde(rename = "6m")]
    SixMonths,
    #[serde(rename = "1y")]
    OneYear,
}

impl OwnerEquityChartRange {
    const fn months(self) -> u32 {
        match self {
            Self::OneMonth => 1,
            Self::ThreeMonths => 3,
            Self::SixMonths => 6,
            Self::OneYear => 12,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum OwnerEquityChartFreshness {
    Current,
    Stale,
    Unverifiable,
}

#[derive(Debug, Clone, Serialize)]
struct OwnerEquityChartLatestDto {
    session_date: NaiveDate,
    close: u64,
    change: i64,
    change_rate: f64,
    volume: u64,
}

#[derive(Debug, Clone, Serialize)]
struct OwnerEquityChartBarDto {
    session_date: NaiveDate,
    open: u64,
    high: u64,
    low: u64,
    close: u64,
    volume: u64,
    sma_20: Option<f64>,
    sma_60: Option<f64>,
}

/// The deliberately small chart projection. Lineage pins, source references,
/// paths, hashes, and artifact metadata remain private to the verifier.
#[derive(Debug, Clone, Serialize)]
struct OwnerEquityChartDto {
    snapshot_id: Uuid,
    instrument_id: String,
    generation: u64,
    range: OwnerEquityChartRange,
    as_of: NaiveDate,
    freshness: OwnerEquityChartFreshness,
    expected_as_of: Option<NaiveDate>,
    price_semantics: &'static str,
    latest: OwnerEquityChartLatestDto,
    bars: Vec<OwnerEquityChartBarDto>,
    warnings: [&'static str; 3],
}

#[derive(Debug, Clone)]
struct ValidatedChartBar {
    session_date: NaiveDate,
    open: u64,
    high: u64,
    low: u64,
    close: u64,
    volume: u64,
    sma_20: Option<f64>,
    sma_60: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OwnerEquityChartArtifactError {
    Unavailable,
    Integrity,
}

pub async fn list_memberships(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
) -> Response {
    let rid = request_id(&headers);
    if let Some(response) = require_owner(&session, &rid) {
        return response;
    }
    match state.owner_equity_v2().list(&session.actor()).await {
        Ok((policy, memberships)) => {
            let policy = match policy_dto(&policy) {
                Ok(policy) => policy,
                Err(error) => return repo_error(error, &rid),
            };
            match memberships
                .iter()
                .map(membership_dto)
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(memberships) => (
                    StatusCode::OK,
                    Json(MembershipListDto {
                        policy,
                        memberships,
                    }),
                )
                    .into_response(),
                Err(error) => repo_error(error, &rid),
            }
        }
        Err(error) => repo_error(error, &rid),
    }
}

pub async fn membership_status(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(membership_id): Path<Uuid>,
) -> Response {
    let rid = request_id(&headers);
    if let Some(response) = require_owner(&session, &rid) {
        return response;
    }
    match state
        .owner_equity_v2()
        .get(&session.actor(), membership_id)
        .await
    {
        Ok((policy, membership)) => {
            let policy = match policy_dto(&policy) {
                Ok(value) => value,
                Err(error) => return repo_error(error, &rid),
            };
            match membership_dto(&membership) {
                Ok(membership) => (
                    StatusCode::OK,
                    Json(MembershipStatusDto { policy, membership }),
                )
                    .into_response(),
                Err(error) => repo_error(error, &rid),
            }
        }
        Err(error) => repo_error(error, &rid),
    }
}

pub async fn add_membership(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    JsonBody(body): JsonBody<AddMembershipBody>,
) -> Response {
    let rid = request_id(&headers);
    if let Some(response) = mutation_guard(&session, &headers, &rid) {
        return response;
    }
    if !canonical_code(&body.instrument_code) {
        return code_error(
            "INVALID_PARAMETER",
            "instrument_code must be six ASCII digits",
            &rid,
        );
    }
    let Some(key) = validated_key(&headers, &rid) else {
        return idempotency_key_error(&headers, &rid);
    };
    let binding = binding_hash(json!({"action": "ADD", "body": body}));
    let pins = match mutation_pins(&state) {
        Ok(pins) => pins,
        Err(error) => return mutation_pin_error(error, &rid),
    };
    match state
        .owner_equity_v2()
        .add(
            &session.actor(),
            &body.instrument_code,
            &key,
            &binding,
            &pins,
        )
        .await
    {
        Ok(result) => {
            mutation_response(&state, &session, &headers, result, "owner_equity_v2.add").await
        }
        Err(error) => repo_error(error, &rid),
    }
}

pub async fn retry_membership(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(membership_id): Path<Uuid>,
    JsonBody(_body): JsonBody<TransitionBody>,
) -> Response {
    transition_handler(state, session, headers, membership_id, true).await
}

pub async fn disable_membership(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(membership_id): Path<Uuid>,
    JsonBody(_body): JsonBody<TransitionBody>,
) -> Response {
    transition_handler(state, session, headers, membership_id, false).await
}

async fn transition_handler(
    state: ApiState,
    session: Session,
    headers: HeaderMap,
    membership_id: Uuid,
    retry: bool,
) -> Response {
    let rid = request_id(&headers);
    if let Some(response) = mutation_guard(&session, &headers, &rid) {
        return response;
    }
    let Some(key) = validated_key(&headers, &rid) else {
        return idempotency_key_error(&headers, &rid);
    };
    let action = if retry { "RETRY" } else { "DISABLE" };
    let binding = binding_hash(json!({"action": action, "membership_id": membership_id}));
    let pins = match mutation_pins(&state) {
        Ok(pins) => pins,
        Err(error) => return mutation_pin_error(error, &rid),
    };
    let result = if retry {
        state
            .owner_equity_v2()
            .retry(&session.actor(), membership_id, &key, &binding, &pins)
            .await
    } else {
        state
            .owner_equity_v2()
            .disable(&session.actor(), membership_id, &key, &binding, &pins)
            .await
    };
    match result {
        Ok(result) => {
            let event = if retry {
                "owner_equity_v2.retry"
            } else {
                "owner_equity_v2.disable"
            };
            mutation_response(&state, &session, &headers, result, event).await
        }
        Err(error) => repo_error(error, &rid),
    }
}

async fn mutation_response(
    state: &ApiState,
    session: &Session,
    headers: &HeaderMap,
    result: OwnerEquityMutationResult,
    event: &str,
) -> Response {
    let rid = request_id(headers);
    let resource = match membership_dto(&result.membership) {
        Ok(value) => value,
        Err(error) => return repo_error(error, &rid),
    };
    if !result.replayed {
        crate::http::audit(
            state,
            session,
            headers,
            event,
            "owner_equity_membership",
            &resource.id.to_string(),
            None,
            serde_json::to_value(&resource).ok(),
            None,
        )
        .await;
    }
    let mut response = (
        StatusCode::ACCEPTED,
        Json(MembershipMutationDto {
            resource,
            job_id: result.job_id,
            duplicate_active: result.duplicate_active,
        }),
    )
        .into_response();
    if result.replayed {
        response.headers_mut().insert(
            "X-Idempotent-Replay",
            axum::http::HeaderValue::from_static("true"),
        );
    }
    response
}

pub async fn latest_signals(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
) -> Response {
    let rid = request_id(&headers);
    if let Some(response) = require_owner(&session, &rid) {
        return response;
    }
    match latest_bundle(&state, &session).await {
        Ok(bundle) => {
            let snapshot = match snapshot_dto(&bundle) {
                Ok(value) => value,
                Err(error) => return repo_error(error, &rid),
            };
            let rows = match bundle
                .rows
                .iter()
                .map(signal_dto)
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(rows) => rows,
                Err(error) => return repo_error(error, &rid),
            };
            (
                StatusCode::OK,
                Json(LatestSignalsDto {
                    snapshot,
                    top5: rows.iter().take(5).cloned().collect(),
                    rows,
                }),
            )
                .into_response()
        }
        Err(error) => repo_error(error, &rid),
    }
}

pub async fn screen_signals(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    JsonBody(body): JsonBody<SignalScreenBody>,
) -> Response {
    let rid = request_id(&headers);
    if let Some(response) = require_owner(&session, &rid) {
        return response;
    }
    let (policy, _) = match state.owner_equity_v2().list(&session.actor()).await {
        Ok(value) => value,
        Err(error) => return repo_error(error, &rid),
    };
    if let Err(error) = validate_screen(&body, &policy) {
        return repo_error(error, &rid);
    }
    let selected = body
        .instrument_ids
        .as_ref()
        .map(|items| items.iter().cloned().collect::<BTreeSet<_>>());
    let conditions = body
        .conditions
        .as_ref()
        .map(|items| items.iter().copied().collect::<BTreeSet<_>>());
    match latest_bundle(&state, &session).await {
        Ok(bundle) => {
            let snapshot = match snapshot_dto(&bundle) {
                Ok(value) => value,
                Err(error) => return repo_error(error, &rid),
            };
            let rows = bundle
                .rows
                .iter()
                .filter(|row| {
                    selected
                        .as_ref()
                        .is_none_or(|items| items.contains(&row.instrument_id))
                        && conditions.as_ref().is_none_or(|items| {
                            items.contains(&EquitySignalsCondition::from(row.signal.condition))
                        })
                })
                .map(signal_dto)
                .collect::<Result<Vec<_>, _>>();
            match rows {
                Ok(rows) => {
                    (StatusCode::OK, Json(ScreenSignalsDto { snapshot, rows })).into_response()
                }
                Err(error) => repo_error(error, &rid),
            }
        }
        Err(error) => repo_error(error, &rid),
    }
}

pub async fn signal_detail(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(instrument_id): Path<String>,
) -> Response {
    let rid = request_id(&headers);
    if let Some(response) = require_owner(&session, &rid) {
        return response;
    }
    if !canonical_instrument(&instrument_id) {
        return code_error("RESOURCE_NOT_FOUND", "resource not found", &rid);
    }
    match latest_bundle(&state, &session).await {
        Ok(bundle) => {
            let Some(row) = bundle
                .rows
                .iter()
                .find(|row| row.instrument_id == instrument_id)
            else {
                return code_error("RESOURCE_NOT_FOUND", "resource not found", &rid);
            };
            match (snapshot_dto(&bundle), signal_dto(row)) {
                (Ok(snapshot), Ok(signal)) => {
                    (StatusCode::OK, Json(SignalDetailDto { snapshot, signal })).into_response()
                }
                _ => repo_error(OwnerEquityRepoError::Integrity, &rid),
            }
        }
        Err(error) => repo_error(error, &rid),
    }
}

/// GET one snapshot-pinned, artifact-verified EOD chart. This handler reads
/// neither a provider nor Raw data; its only filesystem input is the exact
/// immutable candidate admitted by the requested published signal row.
pub(crate) async fn chart(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(instrument_id): Path<String>,
    query: Result<Query<OwnerEquityChartQuery>, QueryRejection>,
) -> Response {
    let rid = request_id(&headers);
    if let Some(response) = require_owner(&session, &rid) {
        return chart_no_store(response);
    }
    if !canonical_instrument(&instrument_id) {
        return chart_no_store(code_error(
            "INVALID_PARAMETER",
            "invalid owner equity chart request",
            &rid,
        ));
    }
    let Query(query) = match query {
        Ok(value) => value,
        Err(_) => {
            return chart_no_store(code_error(
                "INVALID_PARAMETER",
                "invalid owner equity chart request",
                &rid,
            ));
        }
    };

    let descriptor = match state
        .owner_equity_v2()
        .chart_descriptor(&session.actor(), query.snapshot_id, &instrument_id)
        .await
    {
        Ok(Some(value)) => value,
        Ok(None) | Err(OwnerEquityRepoError::NotFound) => {
            return chart_no_store(code_error("RESOURCE_NOT_FOUND", "resource not found", &rid));
        }
        Err(OwnerEquityRepoError::InvalidRequest) => {
            return chart_no_store(code_error(
                "INVALID_PARAMETER",
                "invalid owner equity chart request",
                &rid,
            ));
        }
        Err(OwnerEquityRepoError::Integrity) => {
            return chart_no_store(chart_artifact_error(
                OwnerEquityChartArtifactError::Integrity,
                &rid,
            ));
        }
        Err(error @ OwnerEquityRepoError::Database(_)) => {
            return chart_no_store(repo_error(error, &rid));
        }
        Err(_) => {
            return chart_no_store(chart_artifact_error(
                OwnerEquityChartArtifactError::Integrity,
                &rid,
            ));
        }
    };
    let Some(root) = state.cfg.owner_equity_v2_api_artifact_root.clone() else {
        return chart_no_store(chart_artifact_error(
            OwnerEquityChartArtifactError::Unavailable,
            &rid,
        ));
    };
    let manifest_sha256 = match ContentHash::parse(&descriptor.artifact_manifest_sha256) {
        Ok(value) => value,
        Err(_) => {
            return chart_no_store(chart_artifact_error(
                OwnerEquityChartArtifactError::Integrity,
                &rid,
            ));
        }
    };
    let artifact = match load_chart_artifact(&state, root, manifest_sha256).await {
        Ok(value) => value,
        Err(error) => return chart_no_store(chart_artifact_error(error, &rid)),
    };
    if !artifact_matches_descriptor(&artifact, &descriptor, query.snapshot_id, &instrument_id) {
        return chart_no_store(chart_artifact_error(
            OwnerEquityChartArtifactError::Integrity,
            &rid,
        ));
    }
    let confirmed_close = match state.candidates().latest_confirmed_krx_close().await {
        Ok(value) => value,
        Err(_) => {
            return chart_no_store(chart_internal_error(&rid));
        }
    };
    let expected_as_of = match chart_expected_as_of(
        descriptor.as_of_session,
        confirmed_close,
        (state.cfg.seoul_today)(),
        (state.cfg.candidate_eod_ready)(),
    ) {
        Ok(value) => value,
        Err(()) => {
            return chart_no_store(chart_artifact_error(
                OwnerEquityChartArtifactError::Integrity,
                &rid,
            ));
        }
    };
    let dto = match chart_dto(&artifact, &descriptor, query.range, expected_as_of) {
        Ok(value) => value,
        Err(()) => {
            return chart_no_store(chart_artifact_error(
                OwnerEquityChartArtifactError::Integrity,
                &rid,
            ));
        }
    };
    chart_no_store((StatusCode::OK, Json(dto)).into_response())
}

async fn load_chart_artifact(
    state: &ApiState,
    root: PathBuf,
    manifest_sha256: ContentHash,
) -> Result<VerifiedOwnerEquityArtifact, OwnerEquityChartArtifactError> {
    let permit = state
        .owner_equity_v2_artifact
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| OwnerEquityChartArtifactError::Unavailable)?;
    let read =
        tokio::task::spawn_blocking(move || read_owner_equity_artifact(&root, &manifest_sha256))
            .await
            .map_err(|_| OwnerEquityChartArtifactError::Integrity)?;
    drop(permit);
    read.map_err(map_chart_artifact_error)
}

fn map_chart_artifact_error(error: OwnerEquityArtifactError) -> OwnerEquityChartArtifactError {
    match error {
        OwnerEquityArtifactError::Missing
        | OwnerEquityArtifactError::UnsafeRoot
        | OwnerEquityArtifactError::WriteFailed => OwnerEquityChartArtifactError::Unavailable,
        OwnerEquityArtifactError::UnsafePermissions
        | OwnerEquityArtifactError::Tampered
        | OwnerEquityArtifactError::Conflict
        | OwnerEquityArtifactError::CandidateInvalid => OwnerEquityChartArtifactError::Integrity,
    }
}

fn artifact_matches_descriptor(
    artifact: &VerifiedOwnerEquityArtifact,
    descriptor: &OwnerEquityChartDescriptor,
    snapshot_id: Uuid,
    instrument_id: &str,
) -> bool {
    let Ok(generation) = u64::try_from(descriptor.generation) else {
        return false;
    };
    // The sealed artifact format deliberately carries the stable generation
    // number, not the database-private generation UUID. The descriptor query
    // binds that UUID through every admission/generation join; reject an
    // impossible value here so it cannot become an inert field at this edge.
    if descriptor.generation_id == Uuid::nil() {
        return false;
    }
    let source = &artifact.candidate.source_pins;
    descriptor.snapshot_id == snapshot_id
        && descriptor.instrument_id == instrument_id
        && artifact.owner_user_id == descriptor.owner_user_id
        && artifact.membership_id == descriptor.membership_id
        && artifact.generation == generation
        && artifact.candidate.instrument_id.to_string() == descriptor.instrument_id
        && artifact.manifest_sha256.as_str() == descriptor.artifact_manifest_sha256
        && source.raw_manifest_sha256.as_str() == descriptor.raw_manifest_sha256
        && source.entitlement_sha256.as_str() == descriptor.entitlement_sha256
        && source.capture_code_commit.as_str() == descriptor.capture_code_commit
        && source.materializer_code_commit.as_str() == descriptor.materializer_code_commit
        && artifact.candidate.price_semantics == PRICE_SEMANTICS
        && artifact.candidate.owner_only
        && artifact.candidate.vendor_snapshot
        && !artifact.candidate.strict_pit
}

fn chart_expected_as_of(
    as_of: NaiveDate,
    confirmed_close: Option<NaiveDate>,
    today: NaiveDate,
    current_session_closed: bool,
) -> Result<Option<NaiveDate>, ()> {
    let cutoff = requested_through_date(today, current_session_closed).ok_or(())?;
    if as_of > cutoff || confirmed_close.is_some_and(|reference| reference > cutoff) {
        return Err(());
    }
    // The shared EOD reference can lag independently admitted Owner data.
    // An older reference cannot establish its freshness; keep the verified
    // chart available as UNVERIFIABLE without inventing a newer close proof.
    Ok(confirmed_close.filter(|reference| *reference >= as_of))
}

fn chart_dto(
    artifact: &VerifiedOwnerEquityArtifact,
    descriptor: &OwnerEquityChartDescriptor,
    range: OwnerEquityChartRange,
    expected_as_of: Option<NaiveDate>,
) -> Result<OwnerEquityChartDto, ()> {
    let generation = u64::try_from(descriptor.generation).map_err(|_| ())?;
    let candidate = &artifact.candidate;
    if candidate.bars.is_empty()
        || candidate.bars.len() > MAX_CHART_BARS
        || candidate.observed_sessions as usize != candidate.bars.len()
    {
        return Err(());
    }

    let mut history = Vec::with_capacity(candidate.bars.len());
    let mut previous_date = None;
    let mut sma_20_sum = 0_u128;
    let mut sma_60_sum = 0_u128;
    for (index, bar) in candidate.bars.iter().enumerate() {
        let session_date = bar.session_date.as_naive_date();
        if previous_date.is_some_and(|date| date >= session_date)
            || session_date > descriptor.as_of_session
            || bar.open == 0
            || bar.high == 0
            || bar.low == 0
            || bar.close == 0
            || bar.high < bar.open
            || bar.high < bar.close
            || bar.high < bar.low
            || bar.low > bar.open
            || bar.low > bar.close
        {
            return Err(());
        }
        previous_date = Some(session_date);
        sma_20_sum += u128::from(bar.close);
        sma_60_sum += u128::from(bar.close);
        if index >= 20 {
            sma_20_sum -= u128::from(candidate.bars[index - 20].close);
        }
        if index >= 60 {
            sma_60_sum -= u128::from(candidate.bars[index - 60].close);
        }
        let sma_20 = (index + 1 >= 20)
            .then(|| sma_20_sum as f64 / 20.0)
            .filter(|value| value.is_finite());
        let sma_60 = (index + 1 >= 60)
            .then(|| sma_60_sum as f64 / 60.0)
            .filter(|value| value.is_finite());
        if (index + 1 >= 20 && sma_20.is_none()) || (index + 1 >= 60 && sma_60.is_none()) {
            return Err(());
        }
        history.push(ValidatedChartBar {
            session_date,
            open: bar.open,
            high: bar.high,
            low: bar.low,
            close: bar.close,
            volume: bar.volume,
            sma_20,
            sma_60,
        });
    }
    let current = history.last().cloned().ok_or(())?;
    let previous = history
        .get(history.len().checked_sub(2).ok_or(())?)
        .cloned()
        .ok_or(())?;
    if current.session_date != descriptor.as_of_session
        || candidate.first_observed_date.as_naive_date()
            != history.first().map(|bar| bar.session_date).ok_or(())?
        || candidate.last_observed_date.as_naive_date() != current.session_date
        || previous.close == 0
    {
        return Err(());
    }
    let change = i128::from(current.close) - i128::from(previous.close);
    let change = i64::try_from(change).map_err(|_| ())?;
    let change_rate = change as f64 / previous.close as f64;
    if !change_rate.is_finite() {
        return Err(());
    }
    let freshness = match expected_as_of {
        None => OwnerEquityChartFreshness::Unverifiable,
        Some(reference) if descriptor.as_of_session == reference => {
            OwnerEquityChartFreshness::Current
        }
        Some(reference) if descriptor.as_of_session < reference => OwnerEquityChartFreshness::Stale,
        Some(_) => return Err(()),
    };
    let range_start = descriptor
        .as_of_session
        .checked_sub_months(Months::new(range.months()))
        .ok_or(())?;
    let bars = history
        .into_iter()
        .filter(|bar| bar.session_date >= range_start)
        .map(|bar| OwnerEquityChartBarDto {
            session_date: bar.session_date,
            open: bar.open,
            high: bar.high,
            low: bar.low,
            close: bar.close,
            volume: bar.volume,
            sma_20: bar.sma_20,
            sma_60: bar.sma_60,
        })
        .collect::<Vec<_>>();
    if bars.is_empty() || bars.len() > MAX_CHART_BARS {
        return Err(());
    }
    Ok(OwnerEquityChartDto {
        snapshot_id: descriptor.snapshot_id,
        instrument_id: descriptor.instrument_id.clone(),
        generation,
        range,
        as_of: descriptor.as_of_session,
        freshness,
        expected_as_of,
        price_semantics: "ORIGINAL_UNADJUSTED",
        latest: OwnerEquityChartLatestDto {
            session_date: current.session_date,
            close: current.close,
            change,
            change_rate,
            volume: current.volume,
        },
        bars,
        warnings: CHART_WARNINGS,
    })
}

fn chart_artifact_error(error: OwnerEquityChartArtifactError, rid: &str) -> Response {
    match error {
        OwnerEquityChartArtifactError::Unavailable => api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "OWNER_EQUITY_CHART_UNAVAILABLE",
            "owner equity chart unavailable",
            rid,
            None,
        ),
        OwnerEquityChartArtifactError::Integrity => api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "OWNER_EQUITY_INTEGRITY_FAILED",
            "owner equity chart integrity check failed",
            rid,
            None,
        ),
    }
}

fn chart_internal_error(rid: &str) -> Response {
    crate::observability::log::LogEvent::critical("owner_equity_v2.chart_freshness")
        .correlation(rid)
        .error_code("INTERNAL")
        .emit();
    api_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "INTERNAL",
        "internal error",
        rid,
        None,
    )
}

fn chart_no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert("Cache-Control", HeaderValue::from_static("no-store"));
    response
}

async fn latest_bundle(
    state: &ApiState,
    session: &Session,
) -> Result<OwnerEquityLatestSnapshot, OwnerEquityRepoError> {
    state
        .owner_equity_v2()
        .latest_snapshot(&session.actor())
        .await?
        .ok_or(OwnerEquityRepoError::SnapshotUnavailable)
}

fn require_owner(session: &Session, rid: &str) -> Option<Response> {
    (!session.actor().is_owner()).then(|| code_error("FORBIDDEN", "forbidden", rid))
}

fn mutation_guard(session: &Session, headers: &HeaderMap, rid: &str) -> Option<Response> {
    require_owner(session, rid).or_else(|| require_csrf(headers, &session.0).err())
}

fn validated_key(headers: &HeaderMap, _rid: &str) -> Option<String> {
    let key = idempotency::key_from(headers)?;
    job_queue::owner_equity_v2::durable_idempotency_key(&key)
        .ok()
        .map(|_| key)
}

fn idempotency_key_error(headers: &HeaderMap, rid: &str) -> Response {
    if idempotency::key_from(headers).is_none() {
        code_error(
            "IDEMPOTENCY_KEY_REQUIRED",
            "mutating routes require an Idempotency-Key header",
            rid,
        )
    } else {
        code_error(
            "INVALID_PARAMETER",
            "Idempotency-Key must be 1..=128 visible ASCII characters excluding colon and backslash",
            rid,
        )
    }
}

#[derive(Debug, Clone, Copy)]
enum MutationPinError {
    EntitlementUnavailable,
    Integrity,
}

fn mutation_pins(state: &ApiState) -> Result<OwnerEquityMutationPins, MutationPinError> {
    let pins = state
        .cfg
        .owner_equity_v2_pins
        .as_ref()
        .ok_or(MutationPinError::EntitlementUnavailable)?;
    let requested_date =
        requested_through_date((state.cfg.seoul_today)(), (state.cfg.candidate_eod_ready)())
            .ok_or(MutationPinError::Integrity)?;
    let requested_through = domain::TradingDate::new(
        requested_date.year(),
        requested_date.month(),
        requested_date.day(),
    )
    .map_err(|_| MutationPinError::Integrity)?;
    Ok(OwnerEquityMutationPins {
        code_commit: state.cfg.code_commit.clone(),
        entitlement_reference: pins.entitlement_reference.clone(),
        entitlement_sha256: pins.entitlement_sha256.clone(),
        requested_through,
    })
}

fn requested_through_date(today: NaiveDate, current_session_closed: bool) -> Option<NaiveDate> {
    current_session_closed
        .then_some(today)
        .or_else(|| today.pred_opt())
}

fn mutation_pin_error(error: MutationPinError, rid: &str) -> Response {
    match error {
        MutationPinError::EntitlementUnavailable => code_error(
            "OWNER_EQUITY_ENTITLEMENT_UNAVAILABLE",
            "owner equity entitlement unavailable",
            rid,
        ),
        MutationPinError::Integrity => code_error(
            "OWNER_EQUITY_INTEGRITY_FAILED",
            "owner equity runtime date invalid",
            rid,
        ),
    }
}

fn binding_hash(value: serde_json::Value) -> String {
    idempotency::body_hash(&value)
}

fn policy_dto(
    policy: &OwnerEquityPolicyRecord,
) -> Result<OwnerEquityPolicyDto, OwnerEquityRepoError> {
    let max = u32::try_from(policy.max_active_instruments)
        .map_err(|_| OwnerEquityRepoError::Integrity)?;
    let active =
        u32::try_from(policy.active_instruments).map_err(|_| OwnerEquityRepoError::Integrity)?;
    if active > max {
        return Err(OwnerEquityRepoError::Integrity);
    }
    Ok(OwnerEquityPolicyDto {
        max_active_instruments: max,
        active_instruments: active,
        remaining_capacity: max.saturating_sub(active),
        target_observed_sessions: u32::try_from(policy.target_observed_sessions)
            .map_err(|_| OwnerEquityRepoError::Integrity)?,
        minimum_observed_sessions: u32::try_from(policy.minimum_observed_sessions)
            .map_err(|_| OwnerEquityRepoError::Integrity)?,
    })
}

fn membership_dto(
    membership: &OwnerEquityMembershipRecord,
) -> Result<OwnerEquityMembershipDto, OwnerEquityRepoError> {
    membership.lifecycle()?;
    let failure = match (&membership.error_code, membership.error_retryable) {
        (Some(code), Some(retryable)) => Some(OwnerEquityFailureDto {
            code: code.clone(),
            retryable,
        }),
        (None, None) => None,
        _ => return Err(OwnerEquityRepoError::Integrity),
    };
    Ok(OwnerEquityMembershipDto {
        id: membership.id,
        instrument_id: membership.instrument_id.clone(),
        lifecycle: membership.state.clone(),
        generation: u64::try_from(membership.generation)
            .map_err(|_| OwnerEquityRepoError::Integrity)?,
        coverage: OwnerEquityCoverageDto {
            observed_sessions: u32::try_from(membership.observed_sessions)
                .map_err(|_| OwnerEquityRepoError::Integrity)?,
            target_observed_sessions: u32::try_from(membership.target_observed_sessions)
                .map_err(|_| OwnerEquityRepoError::Integrity)?,
            minimum_observed_sessions: u32::try_from(membership.minimum_observed_sessions)
                .map_err(|_| OwnerEquityRepoError::Integrity)?,
            first_session: membership.first_session,
            last_session: membership.last_session,
        },
        failure,
        requested_at: membership.requested_at,
        disabled_at: membership.disabled_at,
        updated_at: membership.updated_at,
    })
}

fn snapshot_dto(
    latest: &OwnerEquityLatestSnapshot,
) -> Result<OwnerEquitySnapshotDto, OwnerEquityRepoError> {
    Ok(OwnerEquitySnapshotDto {
        snapshot_id: latest.snapshot.id,
        as_of: latest.snapshot.as_of_session,
        universe_sha256: latest.snapshot.universe_sha256.clone(),
        row_count: u32::try_from(latest.snapshot.row_count)
            .map_err(|_| OwnerEquityRepoError::Integrity)?,
        published_at: latest.snapshot.published_at,
    })
}

fn signal_dto(
    row: &OwnerEquitySnapshotRowRecord,
) -> Result<OwnerEquitySignalDto, OwnerEquityRepoError> {
    let signal = &row.signal;
    Ok(OwnerEquitySignalDto {
        instrument_id: row.instrument_id.clone(),
        generation: u64::try_from(row.generation).map_err(|_| OwnerEquityRepoError::Integrity)?,
        rank: u32::try_from(row.rank).map_err(|_| OwnerEquityRepoError::Integrity)?,
        score: signal.score,
        condition: signal.condition.into(),
        return_20: signal.return_20,
        return_60: signal.return_60,
        return_120: signal.return_120,
        volatility_20: signal.volatility_20,
        volatility_60: signal.volatility_60,
        volatility_120: signal.volatility_120,
        max_drawdown_120: signal.max_drawdown_120,
        sma_20: signal.sma_20,
        sma_60: signal.sma_60,
        average_volume_20: signal.average_volume_20,
        volume_ratio_20_60: signal.volume_ratio_20_60,
        average_trading_value_20: signal.average_trading_value_20,
    })
}

fn validate_screen(
    body: &SignalScreenBody,
    policy: &OwnerEquityPolicyRecord,
) -> Result<(), OwnerEquityRepoError> {
    if let Some(ids) = &body.instrument_ids {
        let maximum = usize::try_from(policy.max_active_instruments)
            .map_err(|_| OwnerEquityRepoError::Integrity)?;
        let unique = ids.iter().collect::<BTreeSet<_>>();
        if ids.len() > maximum
            || unique.len() != ids.len()
            || ids
                .iter()
                .any(|instrument| !canonical_instrument(instrument))
        {
            return Err(OwnerEquityRepoError::InvalidRequest);
        }
    }
    if let Some(conditions) = &body.conditions {
        let unique = conditions.iter().collect::<BTreeSet<_>>();
        if unique.len() != conditions.len() {
            return Err(OwnerEquityRepoError::InvalidRequest);
        }
    }
    Ok(())
}

fn canonical_code(value: &str) -> bool {
    value.len() == 6 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn canonical_instrument(value: &str) -> bool {
    value.len() == 10
        && value.ends_with(".KRX")
        && value.as_bytes()[..6]
            .iter()
            .all(|byte| byte.is_ascii_digit())
}

fn repo_error(error: OwnerEquityRepoError, rid: &str) -> Response {
    match error {
        OwnerEquityRepoError::InvalidRequest | OwnerEquityRepoError::InvalidIdempotencyKey => {
            code_error("INVALID_PARAMETER", "invalid owner equity request", rid)
        }
        OwnerEquityRepoError::IdempotencyMismatch => code_error(
            "IDEMPOTENCY_KEY_MISMATCH",
            "the same Idempotency-Key was already used with a different request",
            rid,
        ),
        OwnerEquityRepoError::PolicyUnavailable => code_error(
            "OWNER_EQUITY_POLICY_UNAVAILABLE",
            "owner equity policy unavailable",
            rid,
        ),
        OwnerEquityRepoError::CapacityExceeded => code_error(
            "OWNER_EQUITY_CAPACITY_EXCEEDED",
            "owner equity active capacity reached",
            rid,
        ),
        OwnerEquityRepoError::NotFound => code_error(
            "OWNER_EQUITY_MEMBERSHIP_NOT_FOUND",
            "resource not found",
            rid,
        ),
        OwnerEquityRepoError::InvalidState => code_error(
            "OWNER_EQUITY_INVALID_STATE",
            "owner equity membership is not in the required state",
            rid,
        ),
        OwnerEquityRepoError::EntitlementUnavailable => code_error(
            "OWNER_EQUITY_ENTITLEMENT_UNAVAILABLE",
            "owner equity entitlement unavailable",
            rid,
        ),
        OwnerEquityRepoError::Integrity => code_error(
            "OWNER_EQUITY_INTEGRITY_FAILED",
            "owner equity evidence failed verification",
            rid,
        ),
        OwnerEquityRepoError::SnapshotUnavailable => code_error(
            "OWNER_EQUITY_SNAPSHOT_UNAVAILABLE",
            "owner equity admitted snapshot unavailable",
            rid,
        ),
        OwnerEquityRepoError::Database(_) => {
            crate::observability::log::LogEvent::critical("owner_equity_v2.database")
                .correlation(rid)
                .error_code("INTERNAL")
                .emit();
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "internal error",
                rid,
                None,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use auth::entitlement::{Role, UserId};
    use auth::sessions::SessionInfo;
    use axum::body::to_bytes;
    use chrono::{Datelike, Duration};
    use domain::{BatchId, CodeCommit, InstrumentId, TradingDate};
    use market_data::owner_equity_v2::{
        OwnerEquityBar, OwnerEquityCaptureKind, OwnerEquityGenerationCandidate,
        OwnerEquitySourcePins,
    };

    fn session(role: Role) -> Session {
        Session(SessionInfo {
            user_id: UserId::new(Uuid::new_v4().to_string()),
            role,
            auth_time_secs: 0,
            amr: vec![],
            expires_at_secs: i64::MAX,
            csrf_token_hash: auth::csrf::hash_token("csrf"),
        })
    }

    #[test]
    fn member_is_denied_before_repository_access() {
        let response = require_owner(&session(Role::Member), "request").unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(require_owner(&session(Role::Owner), "request").is_none());
    }

    #[test]
    fn exact_input_and_screen_duplicates_fail_closed() {
        assert!(canonical_code("005930"));
        for invalid in ["5930", "005930.KRX", "００５９３０", "00593a"] {
            assert!(!canonical_code(invalid));
        }
        let policy = OwnerEquityPolicyRecord {
            max_active_instruments: 2,
            active_instruments: 1,
            target_observed_sessions: 261,
            minimum_observed_sessions: 121,
        };
        let duplicate = SignalScreenBody {
            instrument_ids: Some(vec!["005930.KRX".into(), "005930.KRX".into()]),
            conditions: None,
        };
        assert!(validate_screen(&duplicate, &policy).is_err());
    }

    #[test]
    fn body_binding_includes_action_and_path_identity() {
        let add = binding_hash(json!({"action": "ADD", "body": {"instrument_code": "005930"}}));
        let retry = binding_hash(json!({"action": "RETRY", "membership_id": Uuid::nil()}));
        let disable = binding_hash(json!({"action": "DISABLE", "membership_id": Uuid::nil()}));
        assert_ne!(add, retry);
        assert_ne!(retry, disable);
    }

    #[test]
    fn mutation_range_never_requests_an_unconfirmed_current_session() {
        let today = NaiveDate::from_ymd_opt(2026, 8, 31).unwrap();
        assert_eq!(requested_through_date(today, true), Some(today));
        assert_eq!(
            requested_through_date(today, false),
            NaiveDate::from_ymd_opt(2026, 8, 30)
        );
    }

    #[tokio::test]
    async fn csrf_and_idempotency_rejections_precede_database_access() {
        let state =
            ApiState::test_without_database(crate::http::state::OwnerBetaAccessMode::OwnerOnly);
        let owner = session(Role::Owner);
        let response = add_membership(
            State(state.clone()),
            owner.clone(),
            HeaderMap::new(),
            JsonBody(AddMembershipBody {
                instrument_code: "005930".into(),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let bytes = to_bytes(response.into_body(), 16 * 1024)
            .await
            .expect("bounded CSRF body");
        let body: serde_json::Value = serde_json::from_slice(&bytes).expect("typed JSON");
        assert_eq!(body["error"]["code"], "CSRF_DENIED");

        let mut headers = HeaderMap::new();
        headers.insert("x-csrf-token", "csrf".parse().unwrap());
        let response = add_membership(
            State(state.clone()),
            owner,
            headers,
            JsonBody(AddMembershipBody {
                instrument_code: "005930".into(),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let bytes = to_bytes(response.into_body(), 16 * 1024)
            .await
            .expect("bounded idempotency body");
        let body: serde_json::Value = serde_json::from_slice(&bytes).expect("typed JSON");
        assert_eq!(body["error"]["code"], "IDEMPOTENCY_KEY_REQUIRED");
        assert_eq!(state.app_pool.size(), 0);
    }

    #[test]
    fn policy_capacity_uses_repository_count_not_response_slice() {
        let policy = OwnerEquityPolicyRecord {
            max_active_instruments: 73,
            active_instruments: 72,
            target_observed_sessions: 261,
            minimum_observed_sessions: 121,
        };
        let dto = policy_dto(&policy).unwrap();
        assert_eq!(dto.active_instruments, 72);
        assert_eq!(dto.remaining_capacity, 1);
    }

    fn chart_fixture() -> (VerifiedOwnerEquityArtifact, OwnerEquityChartDescriptor) {
        let start = NaiveDate::from_ymd_opt(2025, 12, 14).expect("fixture start");
        let bars = (0..=260)
            .map(|offset| {
                let date = start
                    .checked_add_signed(Duration::days(offset))
                    .expect("fixture date");
                let close = 100 + u64::try_from(offset).expect("fixture close");
                OwnerEquityBar {
                    session_date: TradingDate::new(date.year(), date.month(), date.day())
                        .expect("fixture trading date"),
                    open: close - 1,
                    high: close + 2,
                    low: close - 2,
                    close,
                    volume: 1_000 + u64::try_from(offset).expect("fixture volume"),
                }
            })
            .collect::<Vec<_>>();
        let owner_user_id = Uuid::from_u128(1);
        let membership_id = Uuid::from_u128(2);
        let snapshot_id = Uuid::from_u128(3);
        let generation_id = Uuid::from_u128(4);
        let raw_manifest_sha256 = ContentHash::from_bytes(b"fixture raw manifest");
        let entitlement_sha256 = ContentHash::from_bytes(b"fixture entitlement");
        let capture_code_commit =
            CodeCommit::parse("0123456789abcdef0123456789abcdef01234567").expect("commit");
        let materializer_code_commit =
            CodeCommit::parse("89abcdef0123456789abcdef0123456789abcdef").expect("commit");
        let candidate = OwnerEquityGenerationCandidate {
            candidate_version: "fixture".to_owned(),
            contract_version: "fixture".to_owned(),
            capture_kind: OwnerEquityCaptureKind::Initial,
            instrument_id: InstrumentId::parse("005930.KRX").expect("fixture instrument"),
            display_name: None,
            requested_start: bars.first().expect("fixture bars").session_date,
            requested_end: bars.last().expect("fixture bars").session_date,
            target_observed_sessions: 261,
            minimum_observed_sessions: 121,
            observed_sessions: u32::try_from(bars.len()).expect("fixture observation count"),
            first_observed_date: bars.first().expect("fixture bars").session_date,
            last_observed_date: bars.last().expect("fixture bars").session_date,
            bars,
            source_pins: OwnerEquitySourcePins {
                capture_identity_sha256: ContentHash::from_bytes(b"fixture identity"),
                raw_batch_id: BatchId::from_uuid(Uuid::from_u128(5)),
                raw_manifest_sha256: raw_manifest_sha256.clone(),
                batch_json_sha256: ContentHash::from_bytes(b"fixture batch"),
                entitlement_reference: "fixture entitlement reference".to_owned(),
                entitlement_sha256: entitlement_sha256.clone(),
                capture_code_commit: capture_code_commit.clone(),
                materializer_code_commit: materializer_code_commit.clone(),
                prior_candidate_sha256: None,
                prior_artifact_manifest_sha256: None,
                files: Vec::new(),
            },
            price_semantics: PRICE_SEMANTICS.to_owned(),
            owner_only: true,
            vendor_snapshot: true,
            strict_pit: false,
            warnings: Vec::new(),
            claims_not_made: Vec::new(),
        };
        let manifest_sha256 = ContentHash::from_bytes(b"fixture artifact manifest");
        let descriptor = OwnerEquityChartDescriptor {
            owner_user_id,
            membership_id,
            generation_id,
            generation: 1,
            instrument_id: "005930.KRX".to_owned(),
            snapshot_id,
            as_of_session: candidate.last_observed_date.as_naive_date(),
            artifact_manifest_sha256: manifest_sha256.as_str().to_owned(),
            raw_manifest_sha256: raw_manifest_sha256.as_str().to_owned(),
            entitlement_sha256: entitlement_sha256.as_str().to_owned(),
            capture_code_commit: capture_code_commit.as_str().to_owned(),
            materializer_code_commit: materializer_code_commit.as_str().to_owned(),
        };
        let artifact = VerifiedOwnerEquityArtifact {
            owner_user_id,
            membership_id,
            generation: 1,
            candidate,
            candidate_sha256: ContentHash::from_bytes(b"fixture candidate"),
            manifest_sha256,
            replayed: false,
        };
        (artifact, descriptor)
    }

    #[test]
    fn chart_descriptor_and_artifact_lineage_are_exact() {
        let (artifact, descriptor) = chart_fixture();
        assert!(artifact_matches_descriptor(
            &artifact,
            &descriptor,
            descriptor.snapshot_id,
            &descriptor.instrument_id,
        ));

        let mut wrong_owner = descriptor.clone();
        wrong_owner.owner_user_id = Uuid::from_u128(101);
        let mut wrong_membership = descriptor.clone();
        wrong_membership.membership_id = Uuid::from_u128(102);
        let mut wrong_generation = descriptor.clone();
        wrong_generation.generation = 2;
        let mut wrong_generation_id = descriptor.clone();
        wrong_generation_id.generation_id = Uuid::nil();
        let mut wrong_instrument = descriptor.clone();
        wrong_instrument.instrument_id = "000660.KRX".to_owned();
        let mut wrong_manifest = descriptor.clone();
        wrong_manifest.artifact_manifest_sha256 = ContentHash::from_bytes(b"other manifest")
            .as_str()
            .to_owned();
        let mut wrong_raw = descriptor.clone();
        wrong_raw.raw_manifest_sha256 = ContentHash::from_bytes(b"other raw").as_str().to_owned();
        let mut wrong_entitlement = descriptor.clone();
        wrong_entitlement.entitlement_sha256 = ContentHash::from_bytes(b"other entitlement")
            .as_str()
            .to_owned();
        let mut wrong_capture_commit = descriptor.clone();
        wrong_capture_commit.capture_code_commit =
            "1111111111111111111111111111111111111111".to_owned();
        let mut wrong_materializer_commit = descriptor.clone();
        wrong_materializer_commit.materializer_code_commit =
            "2222222222222222222222222222222222222222".to_owned();

        for mismatch in [
            wrong_owner,
            wrong_membership,
            wrong_generation,
            wrong_generation_id,
            wrong_instrument,
            wrong_manifest,
            wrong_raw,
            wrong_entitlement,
            wrong_capture_commit,
            wrong_materializer_commit,
        ] {
            assert!(!artifact_matches_descriptor(
                &artifact,
                &mismatch,
                mismatch.snapshot_id,
                &mismatch.instrument_id,
            ));
        }
        assert!(!artifact_matches_descriptor(
            &artifact,
            &descriptor,
            Uuid::from_u128(103),
            &descriptor.instrument_id,
        ));

        let mut wrong_price_semantics = artifact.clone();
        wrong_price_semantics.candidate.price_semantics = "ADJUSTED".to_owned();
        assert!(!artifact_matches_descriptor(
            &wrong_price_semantics,
            &descriptor,
            descriptor.snapshot_id,
            &descriptor.instrument_id,
        ));
        let mut not_owner_only = artifact.clone();
        not_owner_only.candidate.owner_only = false;
        assert!(!artifact_matches_descriptor(
            &not_owner_only,
            &descriptor,
            descriptor.snapshot_id,
            &descriptor.instrument_id,
        ));
        let mut not_vendor_snapshot = artifact.clone();
        not_vendor_snapshot.candidate.vendor_snapshot = false;
        assert!(!artifact_matches_descriptor(
            &not_vendor_snapshot,
            &descriptor,
            descriptor.snapshot_id,
            &descriptor.instrument_id,
        ));
        let mut strict_pit = artifact;
        strict_pit.candidate.strict_pit = true;
        assert!(!artifact_matches_descriptor(
            &strict_pit,
            &descriptor,
            descriptor.snapshot_id,
            &descriptor.instrument_id,
        ));
    }

    #[test]
    fn chart_freshness_reference_keeps_newer_owner_data_and_rejects_future_sessions() {
        let as_of = NaiveDate::from_ymd_opt(2026, 10, 6).expect("fixture date");
        let today = NaiveDate::from_ymd_opt(2026, 10, 7).expect("fixture date");
        let old_reference = NaiveDate::from_ymd_opt(2026, 8, 31).expect("fixture date");
        assert_eq!(
            chart_expected_as_of(as_of, Some(old_reference), today, false),
            Ok(None)
        );
        assert_eq!(chart_expected_as_of(as_of, None, today, false), Ok(None));
        assert_eq!(
            chart_expected_as_of(as_of, Some(as_of), today, false),
            Ok(Some(as_of))
        );
        assert_eq!(
            chart_expected_as_of(as_of, Some(today), today, true),
            Ok(Some(today))
        );
        for reference in [None, Some(old_reference), Some(today)] {
            assert!(chart_expected_as_of(today, reference, today, false).is_err());
        }
        assert_eq!(
            chart_expected_as_of(today, Some(old_reference), today, true),
            Ok(None)
        );
        let tomorrow = today.succ_opt().expect("fixture successor");
        assert!(chart_expected_as_of(tomorrow, None, today, true).is_err());
        assert!(chart_expected_as_of(as_of, Some(tomorrow), today, true).is_err());
        assert!(chart_expected_as_of(NaiveDate::MIN, None, NaiveDate::MIN, false).is_err());
    }

    #[test]
    fn chart_calendar_ranges_sma_change_and_freshness_are_snapshot_pinned() {
        let (artifact, descriptor) = chart_fixture();
        for range in [
            OwnerEquityChartRange::OneMonth,
            OwnerEquityChartRange::ThreeMonths,
            OwnerEquityChartRange::SixMonths,
            OwnerEquityChartRange::OneYear,
        ] {
            let dto = chart_dto(&artifact, &descriptor, range, None).expect("valid chart");
            let start = descriptor
                .as_of_session
                .checked_sub_months(Months::new(range.months()))
                .expect("representable range");
            let expected_dates = artifact
                .candidate
                .bars
                .iter()
                .map(|bar| bar.session_date.as_naive_date())
                .filter(|date| *date >= start)
                .collect::<Vec<_>>();
            let actual_dates = dto
                .bars
                .iter()
                .map(|bar| bar.session_date)
                .collect::<Vec<_>>();
            assert_eq!(actual_dates, expected_dates);
            assert!(dto.bars.len() <= MAX_CHART_BARS);
            assert_eq!(dto.snapshot_id, descriptor.snapshot_id);
            assert_eq!(dto.instrument_id, descriptor.instrument_id);
            assert_eq!(dto.as_of, descriptor.as_of_session);
            assert_eq!(dto.freshness, OwnerEquityChartFreshness::Unverifiable);
            assert_eq!(dto.expected_as_of, None);
        }

        let one_month = chart_dto(
            &artifact,
            &descriptor,
            OwnerEquityChartRange::OneMonth,
            Some(descriptor.as_of_session),
        )
        .expect("current chart");
        assert_eq!(one_month.freshness, OwnerEquityChartFreshness::Current);
        assert_eq!(one_month.latest.close, 360);
        assert_eq!(one_month.latest.change, 1);
        assert!((one_month.latest.change_rate - (1.0 / 359.0)).abs() < f64::EPSILON);
        let latest_bar = one_month.bars.last().expect("latest bar");
        assert_eq!(latest_bar.sma_20, Some(350.5));
        assert_eq!(latest_bar.sma_60, Some(330.5));
        assert!(one_month.bars.iter().all(|bar| bar.sma_20.is_some()));
        assert!(one_month.bars.iter().all(|bar| bar.sma_60.is_some()));
        assert_eq!(one_month.warnings, CHART_WARNINGS);
        assert_eq!(one_month.price_semantics, "ORIGINAL_UNADJUSTED");

        let stale_date = descriptor
            .as_of_session
            .succ_opt()
            .expect("fixture date has successor");
        assert_eq!(
            chart_dto(
                &artifact,
                &descriptor,
                OwnerEquityChartRange::OneMonth,
                Some(stale_date),
            )
            .expect("stale chart")
            .freshness,
            OwnerEquityChartFreshness::Stale
        );
        assert!(
            chart_dto(
                &artifact,
                &descriptor,
                OwnerEquityChartRange::OneMonth,
                Some(
                    descriptor
                        .as_of_session
                        .pred_opt()
                        .expect("fixture date has predecessor"),
                ),
            )
            .is_err()
        );
    }

    #[test]
    fn chart_rejects_malformed_history_without_fabricating_sessions() {
        let (artifact, descriptor) = chart_fixture();

        let mut missing_session = artifact.clone();
        missing_session.candidate.bars.remove(100);
        missing_session.candidate.observed_sessions = 260;
        let sparse = chart_dto(
            &missing_session,
            &descriptor,
            OwnerEquityChartRange::OneYear,
            None,
        )
        .expect("observed-session gaps remain gaps");
        assert_eq!(sparse.bars.len(), 260);
        assert_ne!(
            sparse.bars[100].session_date,
            artifact.candidate.bars[100].session_date.as_naive_date(),
            "the route must not invent the removed observation"
        );

        let mut too_many = artifact.clone();
        too_many
            .candidate
            .bars
            .push(too_many.candidate.bars.last().expect("fixture bar").clone());
        too_many.candidate.observed_sessions = 262;
        assert!(chart_dto(&too_many, &descriptor, OwnerEquityChartRange::OneYear, None,).is_err());

        let mut count_mismatch = artifact.clone();
        count_mismatch.candidate.observed_sessions = 260;
        assert!(
            chart_dto(
                &count_mismatch,
                &descriptor,
                OwnerEquityChartRange::OneYear,
                None,
            )
            .is_err()
        );

        let mut duplicate = artifact.clone();
        duplicate.candidate.bars[80].session_date = duplicate.candidate.bars[79].session_date;
        assert!(
            chart_dto(
                &duplicate,
                &descriptor,
                OwnerEquityChartRange::OneYear,
                None,
            )
            .is_err()
        );

        let mut reversed = artifact.clone();
        reversed.candidate.bars.swap(80, 81);
        assert!(chart_dto(&reversed, &descriptor, OwnerEquityChartRange::OneYear, None,).is_err());

        let mut future = artifact.clone();
        future.candidate.bars[200].session_date =
            TradingDate::new(2027, 1, 1).expect("future date");
        assert!(chart_dto(&future, &descriptor, OwnerEquityChartRange::OneYear, None,).is_err());

        let mut bad_high = artifact.clone();
        bad_high.candidate.bars[100].high = bad_high.candidate.bars[100].low - 1;
        assert!(chart_dto(&bad_high, &descriptor, OwnerEquityChartRange::OneYear, None,).is_err());

        let mut bad_low = artifact.clone();
        bad_low.candidate.bars[100].low = bad_low.candidate.bars[100].high + 1;
        assert!(chart_dto(&bad_low, &descriptor, OwnerEquityChartRange::OneYear, None,).is_err());

        let mut zero_previous_close = artifact.clone();
        zero_previous_close.candidate.bars[259].close = 0;
        assert!(
            chart_dto(
                &zero_previous_close,
                &descriptor,
                OwnerEquityChartRange::OneYear,
                None,
            )
            .is_err()
        );

        let mut missing_as_of = descriptor.clone();
        missing_as_of.as_of_session = missing_as_of
            .as_of_session
            .succ_opt()
            .expect("fixture date has successor");
        assert!(
            chart_dto(
                &artifact,
                &missing_as_of,
                OwnerEquityChartRange::OneYear,
                None,
            )
            .is_err()
        );
    }

    #[test]
    fn response_dtos_have_no_forbidden_fields() {
        let source = include_str!("owner_equity_v2.rs").to_ascii_lowercase();
        let identifiers = source
            .split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
            .collect::<BTreeSet<_>>();
        for forbidden in [
            concat!("raw_", "bytes"),
            concat!("provider_", "message"),
            concat!("account_", "id"),
            concat!("target_", "price"),
            concat!("prob", "ability"),
            concat!("portfolio_", "weight"),
            concat!("buy_", "claim"),
            concat!("sell_", "claim"),
        ] {
            assert!(
                !identifiers.contains(forbidden),
                "forbidden field: {forbidden}"
            );
        }
    }
}
