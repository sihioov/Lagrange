//! Candidate screener and actor-owned saved-screen routes.

use crate::http::candidates::{analysis_json, dataset_pins, gate_run, research_state};
use crate::http::error::{api_error, code_error, request_id};
use crate::http::session::Session;
use crate::http::state::ApiState;
use crate::http::validation::parse_date;
use crate::http::validation::sha256_hex;
use crate::http::{JsonBody, audit, idempotent};
use crate::repos::candidates::{SavedScreenRow, ScreenFilter};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Sha256;
use uuid::Uuid;

const DISCLAIMER: &str = "연구 정보이며 투자 권유, 목표가 또는 수익 확률이 아닙니다.";
const MAX_SECTORS: usize = 64;
const SCREEN_CURSOR_VERSION: u8 = 1;
type HmacSha256 = Hmac<Sha256>;

/// A screener cursor is a capability for exactly one immutable run and one
/// canonical filter. The score remains decimal text all the way from
/// PostgreSQL to the cursor; converting it through `f64` would permit a page
/// boundary to drift on replay.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ScreenCursor {
    cursor_version: u8,
    run_id: Uuid,
    criteria_sha256: String,
    score: String,
    instrument_id: String,
}

impl ScreenCursor {
    fn encode(&self, secret: &[u8]) -> String {
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(self).expect("cursor json"));
        let mut mac = HmacSha256::new_from_slice(secret).expect("hmac key");
        mac.update(payload.as_bytes());
        let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        format!("{payload}.{signature}")
    }

    fn decode(raw: &str, secret: &[u8]) -> Result<Self, ()> {
        let (payload, signature) = raw.split_once('.').ok_or(())?;
        let mut mac = HmacSha256::new_from_slice(secret).map_err(|_| ())?;
        mac.update(payload.as_bytes());
        let signature = URL_SAFE_NO_PAD.decode(signature).map_err(|_| ())?;
        mac.verify_slice(&signature).map_err(|_| ())?;
        let bytes = URL_SAFE_NO_PAD.decode(payload).map_err(|_| ())?;
        serde_json::from_slice(&bytes).map_err(|_| ())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScreenCriteria {
    #[serde(default)]
    pub sectors: Vec<String>,
    #[serde(default)]
    pub evidence_strength: Vec<String>,
    pub min_total_score: Option<f64>,
    pub min_flow_score: Option<f64>,
    pub min_fundamental_score: Option<f64>,
    pub min_technical_score: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScreenQueryBody {
    pub run_id: Option<String>,
    pub as_of: Option<String>,
    pub criteria: ScreenCriteria,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SavedScreenBody {
    pub name: String,
    pub criteria: ScreenCriteria,
}

pub async fn query(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    JsonBody(body): JsonBody<ScreenQueryBody>,
) -> Response {
    let rid = request_id(&headers);
    let filter = match build_filter(&state, &body, &rid) {
        Ok(filter) => filter,
        Err(response) => return response,
    };
    let limit = filter.limit;
    let (run, mut rows) = match state.candidates().screen(&filter).await {
        Ok(result) => result,
        Err(error) => return crate::http::tenancy_response(error, &rid, "RESOURCE_NOT_FOUND"),
    };
    if let Err(response) = gate_run(&state, &session, &headers, &run).await {
        return response;
    }
    let has_more = rows.len() > limit;
    rows.truncate(limit);
    let criteria_sha256 = canonical_criteria_sha256(&body.criteria);
    let next_cursor = if has_more {
        rows.last().and_then(|row| {
            row.total_score_text.as_ref().map(|score| {
                ScreenCursor {
                    cursor_version: SCREEN_CURSOR_VERSION,
                    run_id: run.id,
                    criteria_sha256: criteria_sha256.clone(),
                    score: score.clone(),
                    instrument_id: row.instrument_id.clone(),
                }
                .encode(&state.cfg.cursor_secret)
            })
        })
    } else {
        None
    };
    let licenses = match state.candidates().license_attributions(run.id).await {
        Ok(rows) => rows,
        Err(error) => return crate::http::tenancy_response(error, &rid, "RESOURCE_NOT_FOUND"),
    };
    let freshness = match research_state(&state, run.as_of_date, &rid).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    no_store(Json(json!({
        "state": freshness,
        "as_of": run.as_of_date,
        "cutoff_at": run.cutoff_at,
        "run_id": run.id,
        "scoring_config": {
            "version": run.scoring_config_version,
            "sha256": run.scoring_config_sha256,
        },
        "dataset_pins": dataset_pins(&run),
        "license_attributions": licenses,
        "disclaimer": DISCLAIMER,
        "items": rows.into_iter().map(analysis_json).collect::<Vec<_>>(),
        "next_cursor": next_cursor,
    })))
}

pub async fn list_screens(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
) -> Response {
    let rid = request_id(&headers);
    match state.candidates().list_screens(&session.actor()).await {
        Ok(rows) => no_store(Json(json!({
            "items": rows.into_iter().map(saved_screen_json).collect::<Vec<_>>()
        }))),
        Err(error) => crate::http::tenancy_response(error, &rid, "RESOURCE_NOT_FOUND"),
    }
}

pub async fn get_screen(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let rid = request_id(&headers);
    let id = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return code_error("INVALID_PARAMETER", "screen id must be a UUID", &rid),
    };
    match state.candidates().get_screen(&session.actor(), id).await {
        Ok(row) => no_store(Json(saved_screen_json(row))),
        Err(error) => crate::http::tenancy_response(error, &rid, "RESOURCE_NOT_FOUND"),
    }
}

pub async fn create_screen(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    JsonBody(body): JsonBody<SavedScreenBody>,
) -> Response {
    mutate_screen(state, session, headers, None, body).await
}

pub async fn update_screen(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(id): Path<String>,
    JsonBody(body): JsonBody<SavedScreenBody>,
) -> Response {
    let rid = request_id(&headers);
    let id = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return code_error("INVALID_PARAMETER", "screen id must be a UUID", &rid),
    };
    mutate_screen(state, session, headers, Some(id), body).await
}

async fn mutate_screen(
    state: ApiState,
    session: Session,
    headers: HeaderMap,
    id: Option<Uuid>,
    body: SavedScreenBody,
) -> Response {
    let rid = request_id(&headers);
    if let Err(response) = crate::http::session::require_csrf(&headers, &session.0) {
        return response;
    }
    let name = body.name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return code_error(
            "INVALID_PARAMETER",
            "screen name must contain 1 to 80 characters",
            &rid,
        );
    }
    if let Err(response) = validate_criteria(&body.criteria, &rid) {
        return response;
    }
    let criteria = serde_json::to_value(&body.criteria).expect("screen criteria serialize");
    let canonical = json!({ "id": id, "name": name, "criteria": criteria });
    let body_hash = crate::http::idempotency::body_hash(&canonical);
    let response = idempotent(&state, &session, &headers, &body_hash, async {
        let actor = session.actor();
        let result = match id {
            Some(id) => {
                state
                    .candidates()
                    .update_screen(&actor, id, name, &criteria)
                    .await
            }
            None => {
                state
                    .candidates()
                    .create_screen(&actor, name, &criteria)
                    .await
            }
        };
        match result {
            Ok(row) => {
                let action = if id.is_some() {
                    "screener.screen.update"
                } else {
                    "screener.screen.create"
                };
                let status = if id.is_some() {
                    StatusCode::OK
                } else {
                    StatusCode::CREATED
                };
                audit(
                    &state,
                    &session,
                    &headers,
                    action,
                    "screener_saved_screen",
                    &row.id.to_string(),
                    None,
                    Some(saved_screen_json(row.clone())),
                    None,
                )
                .await;
                (status, Json(saved_screen_json(row))).into_response()
            }
            Err(error) => crate::http::tenancy_response(error, &rid, "RESOURCE_NOT_FOUND"),
        }
    })
    .await;
    mark_no_store(response)
}

pub async fn delete_screen(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let rid = request_id(&headers);
    if let Err(response) = crate::http::session::require_csrf(&headers, &session.0) {
        return response;
    }
    let id = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return code_error("INVALID_PARAMETER", "screen id must be a UUID", &rid),
    };
    let body_hash = crate::http::idempotency::body_hash(&json!({ "id": id }));
    let response = idempotent(&state, &session, &headers, &body_hash, async {
        let before = state
            .candidates()
            .get_screen(&session.actor(), id)
            .await
            .ok();
        match state.candidates().delete_screen(&session.actor(), id).await {
            Ok(()) => {
                audit(
                    &state,
                    &session,
                    &headers,
                    "screener.screen.delete",
                    "screener_saved_screen",
                    &id.to_string(),
                    before.map(saved_screen_json),
                    Some(json!({ "deleted": true })),
                    None,
                )
                .await;
                (StatusCode::OK, Json(json!({ "id": id, "deleted": true }))).into_response()
            }
            Err(error) => crate::http::tenancy_response(error, &rid, "RESOURCE_NOT_FOUND"),
        }
    })
    .await;
    mark_no_store(response)
}

#[allow(clippy::result_large_err)]
fn build_filter(
    state: &ApiState,
    body: &ScreenQueryBody,
    rid: &str,
) -> Result<ScreenFilter, Response> {
    if body.run_id.is_none() {
        return Err(code_error(
            "INVALID_PARAMETER",
            "run_id is required to select an immutable analysis run",
            rid,
        ));
    }
    let run_id = body
        .run_id
        .as_deref()
        .map(Uuid::parse_str)
        .transpose()
        .map_err(|_| code_error("INVALID_PARAMETER", "run_id must be a UUID", rid))?
        .expect("run_id required above");
    let as_of_date = body.as_of.as_deref().and_then(parse_date);
    if body.as_of.is_some() && as_of_date.is_none() {
        return Err(code_error("INVALID_DATE", "as_of must be YYYY-MM-DD", rid));
    }
    validate_criteria(&body.criteria, rid)?;
    let criteria_sha256 = canonical_criteria_sha256(&body.criteria);
    let (after_score, after_instrument) = match body.cursor.as_deref() {
        None => (None, None),
        Some(raw) => {
            let cursor = ScreenCursor::decode(raw, &state.cfg.cursor_secret)
                .map_err(|_| code_error("INVALID_CURSOR", "pagination cursor is invalid", rid))?;
            if cursor.cursor_version != SCREEN_CURSOR_VERSION
                || cursor.run_id != run_id
                || cursor.criteria_sha256 != criteria_sha256
                || canonical_score_text(&cursor.score).is_none()
                || domain::InstrumentId::parse(&cursor.instrument_id).is_err()
            {
                return Err(code_error(
                    "INVALID_CURSOR",
                    "pagination cursor is invalid",
                    rid,
                ));
            }
            (Some(cursor.score), Some(cursor.instrument_id))
        }
    };
    let limit = body.limit.unwrap_or(25).clamp(1, 100) as usize;
    Ok(ScreenFilter {
        run_id: Some(run_id),
        as_of_date,
        sectors: body.criteria.sectors.clone(),
        evidence: body.criteria.evidence_strength.clone(),
        min_total_score: body.criteria.min_total_score,
        min_flow_score: body.criteria.min_flow_score,
        min_fundamental_score: body.criteria.min_fundamental_score,
        min_technical_score: body.criteria.min_technical_score,
        after_score,
        after_instrument,
        limit,
    })
}

fn canonical_criteria_sha256(criteria: &ScreenCriteria) -> String {
    let mut sectors = criteria.sectors.clone();
    sectors.sort_unstable();
    sectors.dedup();
    let mut evidence = criteria.evidence_strength.clone();
    evidence.sort_unstable();
    evidence.dedup();
    let canonical = json!({
        "sectors": sectors,
        "evidence_strength": evidence,
        "min_total_score": criteria.min_total_score,
        "min_flow_score": criteria.min_flow_score,
        "min_fundamental_score": criteria.min_fundamental_score,
        "min_technical_score": criteria.min_technical_score,
    });
    sha256_hex(&canonical)
}

/// Validate the decimal text emitted by PostgreSQL `numeric::text` without
/// normalizing it. The original text is part of the signed cursor identity.
fn canonical_score_text(value: &str) -> Option<&str> {
    let (whole, fraction) = value.split_once('.')?;
    if whole.is_empty()
        || fraction.len() != 8
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || (whole.len() > 1 && whole.starts_with('0'))
    {
        return None;
    }
    let whole = whole.parse::<u16>().ok()?;
    if whole > 100 || (whole == 100 && !fraction.bytes().all(|byte| byte == b'0')) {
        return None;
    }
    Some(value)
}

#[allow(clippy::result_large_err)]
fn validate_criteria(criteria: &ScreenCriteria, rid: &str) -> Result<(), Response> {
    if criteria.sectors.len() > MAX_SECTORS
        || criteria.sectors.iter().any(|sector| {
            sector.is_empty()
                || sector.len() > 32
                || !sector
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        })
    {
        return Err(code_error(
            "INVALID_PARAMETER",
            "sectors must contain at most 64 canonical codes",
            rid,
        ));
    }
    if criteria
        .evidence_strength
        .iter()
        .any(|value| !matches!(value.as_str(), "STRONG" | "MODERATE" | "WEAK"))
    {
        return Err(code_error(
            "INVALID_PARAMETER",
            "evidence_strength values must be STRONG, MODERATE, or WEAK",
            rid,
        ));
    }
    for value in [
        criteria.min_total_score,
        criteria.min_flow_score,
        criteria.min_fundamental_score,
        criteria.min_technical_score,
    ]
    .into_iter()
    .flatten()
    {
        if !value.is_finite() || !(0.0..=100.0).contains(&value) {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                "INVALID_PARAMETER",
                "score thresholds must be finite values from 0 through 100",
                rid,
                None,
            ));
        }
    }
    Ok(())
}

fn saved_screen_json(row: SavedScreenRow) -> Value {
    json!({
        "id": row.id,
        "name": row.name,
        "criteria_schema_version": row.criteria_schema_version,
        "criteria": row.criteria_json,
        "created_at": row.created_at,
        "updated_at": row.updated_at,
    })
}

fn no_store<T: Serialize>(body: Json<T>) -> Response {
    let response = (StatusCode::OK, body).into_response();
    mark_no_store(response)
}

fn mark_no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: [u8; 32] = *b"candidate-cursor-secret-01234567";

    fn criteria() -> ScreenCriteria {
        ScreenCriteria {
            sectors: vec!["G25".into()],
            evidence_strength: vec!["STRONG".into(), "MODERATE".into()],
            min_total_score: Some(60.0),
            min_flow_score: None,
            min_fundamental_score: None,
            min_technical_score: None,
        }
    }

    #[test]
    fn criteria_rejects_nonfinite_out_of_range_and_unknown_evidence() {
        assert!(validate_criteria(&criteria(), "test").is_ok());
        let mut invalid = criteria();
        invalid.min_total_score = Some(f64::NAN);
        assert!(validate_criteria(&invalid, "test").is_err());
        invalid.min_total_score = Some(101.0);
        assert!(validate_criteria(&invalid, "test").is_err());
        invalid.min_total_score = Some(60.0);
        invalid.evidence_strength = vec!["CERTAIN".into()];
        assert!(validate_criteria(&invalid, "test").is_err());
    }

    #[test]
    fn score_cursor_binds_version_run_filter_exact_score_and_instrument() {
        let run_id = Uuid::parse_str("00000000-0000-4000-8000-000000000001").unwrap();
        let cursor = ScreenCursor {
            cursor_version: SCREEN_CURSOR_VERSION,
            run_id,
            criteria_sha256: canonical_criteria_sha256(&criteria()),
            score: "75.25000000".into(),
            instrument_id: "005930.KRX".into(),
        }
        .encode(&SECRET);
        let decoded = ScreenCursor::decode(&cursor, &SECRET).expect("valid signed cursor");
        assert_eq!(decoded.run_id, run_id);
        assert_eq!(decoded.score, "75.25000000");
        assert!(domain::InstrumentId::parse(&decoded.instrument_id).is_ok());
        assert!(canonical_score_text(&decoded.score).is_some());
        assert_ne!(
            decoded.criteria_sha256,
            canonical_criteria_sha256(&ScreenCriteria {
                min_total_score: Some(61.0),
                ..criteria()
            })
        );
        assert_ne!(
            decoded.run_id,
            Uuid::parse_str("00000000-0000-4000-8000-000000000002").unwrap()
        );
        let mut tampered = cursor;
        tampered.replace_range(0..1, "A");
        assert!(ScreenCursor::decode(&tampered, &SECRET).is_err());
    }

    #[test]
    fn score_cursor_rejects_noncanonical_or_out_of_range_numeric_text() {
        assert!(canonical_score_text("75.25000000").is_some());
        assert!(canonical_score_text("75").is_none());
        assert!(canonical_score_text("101.00000000").is_none());
        assert!(canonical_score_text("100.00000001").is_none());
        assert!(canonical_score_text("075.25000000").is_none());
        assert!(canonical_score_text("75.250000000000").is_none());
        assert!(canonical_score_text("nan.00000000").is_none());
        assert!(canonical_score_text("75.25e0").is_none());
    }

    #[test]
    fn research_copy_never_claims_probability_or_target_price() {
        let copy = DISCLAIMER.to_lowercase();
        assert!(copy.contains("투자 권유"));
        assert!(copy.contains("목표가"));
        assert!(copy.contains("확률"));
    }
}
