//! Owner-only HTTP mutations for bounded intraday quote demand leases.
//!
//! This module owns only the HTTP DTO, privacy/order checks, and response
//! projection. Durable replay, sequence, expiry, release, and capacity rules
//! stay in the job-queue repository and are never mirrored here.

use axum::extract::rejection::PathRejection;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use job_queue::owner_equity_v2::{
    IntradayQuoteDemandRequest, IntradayQuoteReleaseRequest, IntradayStorageError,
};

use crate::http::JsonBody;
use crate::http::error::{code_error, request_id};
use crate::http::idempotency;
use crate::http::session::{Session, require_csrf};
use crate::http::state::ApiState;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IntradayQuoteDemandBody {
    pub schema_version: u32,
    pub consumer_id: Uuid,
    pub membership_id: Uuid,
    pub generation: u64,
    pub renewal_sequence: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IntradayQuoteReleaseBody {
    pub schema_version: u32,
    pub consumer_id: Uuid,
    pub renewal_sequence: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct IntradayQuoteDemandDto {
    pub schema_version: u32,
    pub demand_id: Uuid,
    pub consumer_id: Uuid,
    pub membership_id: Uuid,
    pub instrument_id: String,
    pub generation: u64,
    pub renewal_sequence: u64,
    pub lease_expires_at: chrono::DateTime<chrono::Utc>,
    pub renew_after_ms: i64,
}

pub async fn create_or_renew(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    body: Result<JsonBody<IntradayQuoteDemandBody>, Response>,
) -> Response {
    let rid = request_id(&headers);
    if let Some(response) = mutation_guard(&session, &headers, &rid) {
        return response;
    }
    let JsonBody(body) = match body {
        Ok(body) => body,
        Err(response) => return response,
    };
    let Some(key) = idempotency_key(&headers) else {
        return code_error(
            "INVALID_PARAMETER",
            "invalid intraday demand idempotency key",
            &rid,
        );
    };
    let request = match IntradayQuoteDemandRequest::with_schema_version(
        body.schema_version,
        body.consumer_id,
        body.membership_id,
        body.generation,
        body.renewal_sequence,
        key,
    ) {
        Ok(request) => request,
        Err(error) => return storage_error(error, &rid),
    };

    match state
        .owner_intraday_quotes()
        .create_or_renew_demand(&session.actor(), &request)
        .await
    {
        Ok(outcome) => {
            let lease = outcome.lease;
            (
                StatusCode::OK,
                axum::Json(IntradayQuoteDemandDto {
                    schema_version: 1,
                    demand_id: lease.demand_id,
                    consumer_id: lease.consumer_id,
                    membership_id: lease.membership_id,
                    instrument_id: lease.instrument_id,
                    generation: lease.generation,
                    renewal_sequence: lease.renewal_sequence,
                    lease_expires_at: lease.lease_expires_at,
                    renew_after_ms: lease.renew_after_ms,
                }),
            )
                .into_response()
        }
        Err(error) => storage_error(error, &rid),
    }
}

pub async fn release(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    path: Result<Path<Uuid>, PathRejection>,
    body: Result<JsonBody<IntradayQuoteReleaseBody>, Response>,
) -> Response {
    let rid = request_id(&headers);
    if let Some(response) = mutation_guard(&session, &headers, &rid) {
        return response;
    }
    let demand_id = match path {
        Ok(Path(demand_id)) => demand_id,
        Err(_) => return code_error("INVALID_PARAMETER", "invalid intraday demand request", &rid),
    };
    let JsonBody(body) = match body {
        Ok(body) => body,
        Err(response) => return response,
    };
    let Some(key) = idempotency_key(&headers) else {
        return code_error(
            "INVALID_PARAMETER",
            "invalid intraday demand idempotency key",
            &rid,
        );
    };
    if body.schema_version != 1 {
        return storage_error(IntradayStorageError::InvalidInput, &rid);
    }
    let request =
        match IntradayQuoteReleaseRequest::new(body.consumer_id, body.renewal_sequence, key) {
            Ok(request) => request,
            Err(error) => return storage_error(error, &rid),
        };

    match state
        .owner_intraday_quotes()
        .release_demand(&session.actor(), demand_id, &request)
        .await
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => storage_error(error, &rid),
    }
}

fn mutation_guard(session: &Session, headers: &HeaderMap, rid: &str) -> Option<Response> {
    if !session.actor().is_owner() {
        return Some(code_error("FORBIDDEN", "forbidden", rid));
    }
    require_csrf(headers, &session.0).err()
}

fn idempotency_key(headers: &HeaderMap) -> Option<String> {
    let key = headers
        .get(idempotency::HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)?;
    if key.is_empty()
        || key.len() > 128
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && byte != b':' && byte != b'\\')
    {
        return None;
    }
    Some(key)
}

fn storage_error(error: IntradayStorageError, rid: &str) -> Response {
    match error {
        IntradayStorageError::InvalidInput => {
            code_error("INVALID_PARAMETER", "invalid intraday demand request", rid)
        }
        IntradayStorageError::DemandNotFound
        | IntradayStorageError::IdentityMismatch
        | IntradayStorageError::MembershipNotReady => {
            code_error("RESOURCE_NOT_FOUND", "resource not found", rid)
        }
        IntradayStorageError::DemandReleased | IntradayStorageError::SequenceConflict => {
            code_error(
                "QUOTE_DEMAND_SEQUENCE_CONFLICT",
                "intraday demand sequence cannot be applied",
                rid,
            )
        }
        IntradayStorageError::IdempotencyMismatch => code_error(
            "IDEMPOTENCY_MISMATCH",
            "the same Idempotency-Key was already used with a different request",
            rid,
        ),
        IntradayStorageError::DemandCapacity | IntradayStorageError::IdentityCapacity => {
            let mut response = code_error(
                "QUOTE_DEMAND_CAPACITY",
                "intraday quote demand capacity is exhausted",
                rid,
            );
            response
                .headers_mut()
                .insert("Retry-After", HeaderValue::from_static("15"));
            response
        }
        IntradayStorageError::PolicyUnavailable
        | IntradayStorageError::DatabaseUnavailable
        | IntradayStorageError::DatabaseIntegrity
        | IntradayStorageError::PermissionDenied
        | IntradayStorageError::CommitUnknown
        | IntradayStorageError::ProducerLeaseHeld
        | IntradayStorageError::ProducerLeaseLost
        | IntradayStorageError::ActiveDemandRequired
        | IntradayStorageError::SessionProofInvalid
        | IntradayStorageError::CalendarProofUnavailable
        | IntradayStorageError::BudgetProofInvalid
        | IntradayStorageError::ReceiptInvalid
        | IntradayStorageError::QuoteInvalid
        | IntradayStorageError::QuoteReceiptStale
        | IntradayStorageError::CacheNotFound
        | IntradayStorageError::QuoteVersionExhausted
        | IntradayStorageError::ProducerFenceExhausted => code_error(
            "QUOTE_CACHE_UNAVAILABLE",
            "intraday quote demand storage is unavailable",
            rid,
        ),
    }
}
