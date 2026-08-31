//! Owner-only HTTP contract for the managed equity universe V2.
//!
//! DTOs intentionally whitelist lifecycle, policy, coverage, and price/volume
//! research fields.  Raw evidence, provider prose, entitlement references,
//! lineage paths, and credential/account/order concepts never cross this
//! boundary.

use std::collections::BTreeSet;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::http::JsonBody;
use crate::http::equity_signals::EquitySignalsCondition;
use crate::http::error::{api_error, code_error, request_id};
use crate::http::idempotency;
use crate::http::session::{Session, require_csrf};
use crate::http::state::ApiState;
use crate::repos::owner_equity_v2::{
    OwnerEquityLatestSnapshot, OwnerEquityMembershipRecord, OwnerEquityMutationPins,
    OwnerEquityMutationResult, OwnerEquityPolicyRecord, OwnerEquityRepoError,
    OwnerEquitySnapshotRowRecord,
};

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
