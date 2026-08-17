//! Candidate screener and actor-owned saved-screen routes.

use crate::contract::UniverseKey;
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
use serde::{Deserialize, Deserializer, Serialize, de};
use serde_json::{Value, json};
use sha2::Sha256;
use uuid::Uuid;

const DISCLAIMER: &str = "연구 정보이며 투자 권유, 목표가 또는 수익 확률이 아닙니다.";
const MAX_SECTORS: usize = 64;
const SCREEN_CURSOR_V1: u8 = 1;
const SCREEN_CURSOR_V2: u8 = 2;
type HmacSha256 = Hmac<Sha256>;

/// Version one was issued by the original KOSPI-only screener. It remains
/// readable only for requests that omit `criteria.universes`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ScreenCursorV1 {
    cursor_version: u8,
    run_id: Uuid,
    criteria_sha256: String,
    score: String,
    instrument_id: String,
}

/// Version two signs the complete immutable run-set and the universe block in
/// the last key. A later correction can therefore never splice a new run into
/// an in-progress page sequence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ScreenCursorV2 {
    cursor_version: u8,
    run_set: Vec<ScreenCursorRun>,
    criteria_sha256: String,
    after_universe: String,
    after_score: String,
    after_instrument: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ScreenCursorRun {
    universe: String,
    run_id: Uuid,
}

enum DecodedScreenCursor {
    V1(ScreenCursorV1),
    V2(ScreenCursorV2),
}

fn encode_cursor<T: Serialize>(cursor: &T, secret: &[u8]) -> String {
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(cursor).expect("cursor json"));
    let mut mac = HmacSha256::new_from_slice(secret).expect("hmac key");
    mac.update(payload.as_bytes());
    let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    format!("{payload}.{signature}")
}

fn decode_cursor(raw: &str, secret: &[u8]) -> Result<DecodedScreenCursor, ()> {
    let (payload, signature) = raw.split_once('.').ok_or(())?;
    let mut mac = HmacSha256::new_from_slice(secret).map_err(|_| ())?;
    mac.update(payload.as_bytes());
    let signature = URL_SAFE_NO_PAD.decode(signature).map_err(|_| ())?;
    mac.verify_slice(&signature).map_err(|_| ())?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).map_err(|_| ())?;
    let version = serde_json::from_slice::<serde_json::Value>(&bytes)
        .map_err(|_| ())?
        .get("cursor_version")
        .and_then(Value::as_u64)
        .ok_or(())?;
    match version {
        1 => serde_json::from_slice(&bytes)
            .map(DecodedScreenCursor::V1)
            .map_err(|_| ()),
        2 => serde_json::from_slice(&bytes)
            .map(DecodedScreenCursor::V2)
            .map_err(|_| ()),
        _ => Err(()),
    }
}

impl ScreenCursorV2 {
    fn encode(&self, secret: &[u8]) -> String {
        encode_cursor(self, secret)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScreenCriteria {
    /// `None` means the legacy omitted-universe request and defaults to KOSPI
    /// 200. An explicit empty array is invalid rather than another spelling
    /// of the default.
    #[serde(
        default,
        deserialize_with = "deserialize_nonnull_universes",
        skip_serializing_if = "Option::is_none"
    )]
    pub universes: Option<Vec<String>>,
    #[serde(default)]
    pub sectors: Vec<String>,
    #[serde(default)]
    pub evidence_strength: Vec<String>,
    pub min_total_score: Option<f64>,
    pub min_flow_score: Option<f64>,
    pub min_fundamental_score: Option<f64>,
    pub min_technical_score: Option<f64>,
}

fn deserialize_nonnull_universes<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<Vec<String>>::deserialize(deserializer)?
        .map(Some)
        .ok_or_else(|| de::Error::custom("universes must be an array when present"))
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
    let universes = match normalized_universes(&body.criteria, &rid) {
        Ok(universes) => universes,
        Err(response) => return response,
    };
    let criteria_sha256 = canonical_criteria_sha256(&body.criteria, &universes);
    let (run_set, after_universe, after_score, after_instrument) =
        match build_run_set(&state, &body, &universes, &criteria_sha256, &rid).await {
            Ok(value) => value,
            Err(response) => return response,
        };
    let filter = match build_filter(
        &body,
        run_set,
        after_universe,
        after_score,
        after_instrument,
        &rid,
    ) {
        Ok(filter) => filter,
        Err(response) => return response,
    };
    let limit = filter.limit;
    let blocks = match state.candidates().screen(&filter).await {
        Ok(result) => result,
        Err(error) => return crate::http::tenancy_response(error, &rid, "RESOURCE_NOT_FOUND"),
    };
    let mut runs = Vec::with_capacity(blocks.len());
    let mut rows = Vec::new();
    for (run, block_rows) in blocks {
        if let Err(response) = gate_run(&state, &session, &headers, &run).await {
            return response;
        }
        runs.push(run);
        rows.extend(block_rows);
    }
    let has_more = rows.len() > limit;
    rows.truncate(limit);
    let next_cursor = if has_more {
        rows.last().and_then(|row| {
            row.total_score_text.as_ref().map(|score| {
                ScreenCursorV2 {
                    cursor_version: SCREEN_CURSOR_V2,
                    run_set: runs
                        .iter()
                        .map(|run| ScreenCursorRun {
                            universe: run.universe_key.clone(),
                            run_id: run.id,
                        })
                        .collect(),
                    criteria_sha256: criteria_sha256.clone(),
                    after_universe: row.universe_key.clone(),
                    after_score: score.clone(),
                    after_instrument: row.instrument_id.clone(),
                }
                .encode(&state.cfg.cursor_secret)
            })
        })
    } else {
        None
    };
    let Some(first_run) = runs.first() else {
        return code_error("RESOURCE_NOT_FOUND", "candidate run not found", &rid);
    };
    let mut licenses = Vec::new();
    let mut freshness = "READY";
    for run in &runs {
        let mut run_licenses = match state.candidates().license_attributions(run.id).await {
            Ok(rows) => rows,
            Err(error) => {
                return crate::http::tenancy_response(error, &rid, "RESOURCE_NOT_FOUND");
            }
        };
        licenses.append(&mut run_licenses);
        match research_state(&state, run.as_of_date, &rid).await {
            Ok("STALE") => freshness = "STALE",
            Ok(_) => {}
            Err(response) => return response,
        }
    }
    let run_ids = runs
        .iter()
        .map(|run| json!({ "universe": run.universe_key, "run_id": run.id }))
        .collect::<Vec<_>>();
    let single_run_id = (runs.len() == 1).then_some(first_run.id);
    no_store(Json(json!({
        "state": freshness,
        "as_of": first_run.as_of_date,
        "cutoff_at": first_run.cutoff_at,
        "universe": (runs.len() == 1).then_some(first_run.universe_key.clone()),
        "universes": universes.iter().map(|universe| universe.as_str()).collect::<Vec<_>>(),
        "run_id": single_run_id,
        "run_ids": run_ids,
        "scoring_config": {
            "version": first_run.scoring_config_version,
            "sha256": first_run.scoring_config_sha256,
        },
        "dataset_pins": dataset_pins(first_run),
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
    let normalized = match canonicalize_criteria(&body.criteria, &rid) {
        Ok(criteria) => criteria,
        Err(response) => return response,
    };
    let criteria = serde_json::to_value(&normalized).expect("screen criteria serialize");
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
async fn build_run_set(
    state: &ApiState,
    body: &ScreenQueryBody,
    universes: &[UniverseKey],
    criteria_sha256: &str,
    rid: &str,
) -> Result<
    (
        Vec<(String, Uuid)>,
        Option<String>,
        Option<String>,
        Option<String>,
    ),
    Response,
> {
    let as_of_date = body.as_of.as_deref().and_then(parse_date);
    if body.as_of.is_some() && as_of_date.is_none() {
        return Err(code_error("INVALID_DATE", "as_of must be YYYY-MM-DD", rid));
    }
    validate_criteria(&body.criteria, rid)?;

    if body.run_id.is_some() && universes.len() != 1 {
        return Err(code_error(
            "INVALID_PARAMETER",
            "run_id can only be combined with exactly one universe",
            rid,
        ));
    }

    let parsed_run_id = body
        .run_id
        .as_deref()
        .map(Uuid::parse_str)
        .transpose()
        .map_err(|_| code_error("INVALID_PARAMETER", "run_id must be a UUID", rid))?;

    if let Some(raw) = body.cursor.as_deref() {
        let cursor = decode_cursor(raw, &state.cfg.cursor_secret)
            .map_err(|_| code_error("INVALID_CURSOR", "pagination cursor is invalid", rid))?;
        return match cursor {
            DecodedScreenCursor::V1(cursor) => {
                // The old capability had no universe axis. It is accepted
                // only for the omitted-universe KOSPI compatibility request.
                if body.criteria.universes.is_some()
                    || cursor.cursor_version != SCREEN_CURSOR_V1
                    || cursor.criteria_sha256 != legacy_criteria_sha256(&body.criteria)
                    || canonical_score_text(&cursor.score).is_none()
                    || domain::InstrumentId::parse(&cursor.instrument_id).is_err()
                    || !universes
                        .iter()
                        .all(|universe| *universe == UniverseKey::Kospi200)
                    || parsed_run_id.is_some_and(|run_id| run_id != cursor.run_id)
                {
                    return Err(code_error(
                        "INVALID_CURSOR",
                        "pagination cursor is invalid",
                        rid,
                    ));
                }
                let stored_universe = state
                    .candidates()
                    .run_universe(cursor.run_id)
                    .await
                    .map_err(|_| {
                        code_error("INVALID_CURSOR", "pagination cursor is invalid", rid)
                    })?;
                if stored_universe.as_deref() != Some("kospi200") {
                    return Err(code_error(
                        "INVALID_CURSOR",
                        "pagination cursor is invalid",
                        rid,
                    ));
                }
                Ok((
                    vec![("kospi200".to_owned(), cursor.run_id)],
                    Some("kospi200".to_owned()),
                    Some(cursor.score),
                    Some(cursor.instrument_id),
                ))
            }
            DecodedScreenCursor::V2(cursor) => {
                let ScreenCursorV2 {
                    cursor_version,
                    run_set,
                    criteria_sha256: cursor_criteria_sha256,
                    after_universe,
                    after_score,
                    after_instrument,
                } = cursor;
                if cursor_version != SCREEN_CURSOR_V2
                    || cursor_criteria_sha256 != criteria_sha256
                    || run_set.len() != universes.len()
                    || after_universe.is_empty()
                    || canonical_score_text(&after_score).is_none()
                    || domain::InstrumentId::parse(&after_instrument).is_err()
                {
                    return Err(code_error(
                        "INVALID_CURSOR",
                        "pagination cursor is invalid",
                        rid,
                    ));
                }
                let expected = universes
                    .iter()
                    .map(|universe| universe.as_str())
                    .collect::<Vec<_>>();
                let actual = run_set
                    .iter()
                    .map(|run| run.universe.as_str())
                    .collect::<Vec<_>>();
                if actual != expected
                    || run_set.iter().any(|run| run.run_id.is_nil())
                    || !expected.contains(&after_universe.as_str())
                    || parsed_run_id
                        .is_some_and(|run_id| run_set.len() != 1 || run_set[0].run_id != run_id)
                {
                    return Err(code_error(
                        "INVALID_CURSOR",
                        "pagination cursor is invalid",
                        rid,
                    ));
                }
                for run in &run_set {
                    let stored_universe = state
                        .candidates()
                        .run_universe(run.run_id)
                        .await
                        .map_err(|_| {
                            code_error("INVALID_CURSOR", "pagination cursor is invalid", rid)
                        })?;
                    if stored_universe.as_deref() != Some(run.universe.as_str()) {
                        return Err(code_error(
                            "INVALID_CURSOR",
                            "pagination cursor is invalid",
                            rid,
                        ));
                    }
                }
                Ok((
                    run_set
                        .into_iter()
                        .map(|run| (run.universe, run.run_id))
                        .collect(),
                    Some(after_universe),
                    Some(after_score),
                    Some(after_instrument),
                ))
            }
        };
    }

    let run_set = if let Some(run_id) = parsed_run_id {
        let universe = universes[0];
        let stored_universe = state
            .candidates()
            .run_universe(run_id)
            .await
            .map_err(|error| crate::http::tenancy_response(error, rid, "RESOURCE_NOT_FOUND"))?
            .ok_or_else(|| code_error("RESOURCE_NOT_FOUND", "candidate run not found", rid))?;
        if stored_universe != universe.as_str() {
            return Err(code_error(
                "INVALID_PARAMETER",
                "run_id does not belong to the requested universe",
                rid,
            ));
        }
        vec![(universe.as_str().to_owned(), run_id)]
    } else {
        let requested = universes
            .iter()
            .map(|universe| universe.as_str().to_owned())
            .collect::<Vec<_>>();
        let rows = state
            .candidates()
            .latest_runs(&requested, as_of_date)
            .await
            .map_err(|error| crate::http::tenancy_response(error, rid, "RESOURCE_NOT_FOUND"))?;
        if rows.len() != universes.len() {
            return Err(code_error(
                "RESOURCE_NOT_FOUND",
                "candidate feed not found",
                rid,
            ));
        }
        universes
            .iter()
            .map(|universe| {
                let run = rows
                    .iter()
                    .find(|run| run.universe_key == universe.as_str())
                    .expect("latest run count checked");
                (universe.as_str().to_owned(), run.id)
            })
            .collect()
    };
    Ok((run_set, None, None, None))
}

#[allow(clippy::result_large_err)]
fn build_filter(
    body: &ScreenQueryBody,
    run_set: Vec<(String, Uuid)>,
    after_universe: Option<String>,
    after_score: Option<String>,
    after_instrument: Option<String>,
    rid: &str,
) -> Result<ScreenFilter, Response> {
    let as_of_date = body.as_of.as_deref().and_then(parse_date);
    if body.as_of.is_some() && as_of_date.is_none() {
        return Err(code_error("INVALID_DATE", "as_of must be YYYY-MM-DD", rid));
    }
    validate_criteria(&body.criteria, rid)?;
    let limit = body.limit.unwrap_or(25).clamp(1, 100) as usize;
    Ok(ScreenFilter {
        run_set,
        as_of_date,
        sectors: body.criteria.sectors.clone(),
        evidence: body.criteria.evidence_strength.clone(),
        min_total_score: body.criteria.min_total_score,
        min_flow_score: body.criteria.min_flow_score,
        min_fundamental_score: body.criteria.min_fundamental_score,
        min_technical_score: body.criteria.min_technical_score,
        after_universe,
        after_score,
        after_instrument,
        limit,
    })
}

#[allow(clippy::result_large_err)]
fn normalized_universes(
    criteria: &ScreenCriteria,
    rid: &str,
) -> Result<Vec<UniverseKey>, Response> {
    let default_universe = ["kospi200".to_owned()];
    let raw = criteria.universes.as_deref().unwrap_or(&default_universe);
    if raw.is_empty() {
        return Err(code_error(
            "INVALID_PARAMETER",
            "universes must contain at least one universe",
            rid,
        ));
    }
    let mut parsed = Vec::with_capacity(raw.len());
    for value in raw {
        let universe = UniverseKey::parse(value).map_err(|_| {
            code_error(
                "INVALID_PARAMETER",
                "universes must contain only kospi200 or kosdaq150",
                rid,
            )
        })?;
        if parsed.contains(&universe) {
            return Err(code_error(
                "INVALID_PARAMETER",
                "universes must not contain duplicates",
                rid,
            ));
        }
        parsed.push(universe);
    }
    parsed.sort_by_key(|universe| universe.sort_order());
    Ok(parsed)
}

#[allow(clippy::result_large_err)]
fn canonicalize_criteria(criteria: &ScreenCriteria, rid: &str) -> Result<ScreenCriteria, Response> {
    let universes = normalized_universes(criteria, rid)?
        .into_iter()
        .map(|universe| universe.as_str().to_owned())
        .collect();
    let mut sectors = criteria.sectors.clone();
    sectors.sort_unstable();
    sectors.dedup();
    let mut evidence_strength = criteria.evidence_strength.clone();
    evidence_strength.sort_unstable();
    evidence_strength.dedup();
    Ok(ScreenCriteria {
        universes: Some(universes),
        sectors,
        evidence_strength,
        min_total_score: criteria.min_total_score,
        min_flow_score: criteria.min_flow_score,
        min_fundamental_score: criteria.min_fundamental_score,
        min_technical_score: criteria.min_technical_score,
    })
}

fn canonical_criteria_sha256(criteria: &ScreenCriteria, universes: &[UniverseKey]) -> String {
    let mut sectors = criteria.sectors.clone();
    sectors.sort_unstable();
    sectors.dedup();
    let mut evidence = criteria.evidence_strength.clone();
    evidence.sort_unstable();
    evidence.dedup();
    let canonical = json!({
        "universes": universes.iter().map(|universe| universe.as_str()).collect::<Vec<_>>(),
        "sectors": sectors,
        "evidence_strength": evidence,
        "min_total_score": criteria.min_total_score,
        "min_flow_score": criteria.min_flow_score,
        "min_fundamental_score": criteria.min_fundamental_score,
        "min_technical_score": criteria.min_technical_score,
    });
    sha256_hex(&canonical)
}

fn legacy_criteria_sha256(criteria: &ScreenCriteria) -> String {
    let mut sectors = criteria.sectors.clone();
    sectors.sort_unstable();
    sectors.dedup();
    let mut evidence = criteria.evidence_strength.clone();
    evidence.sort_unstable();
    evidence.dedup();
    sha256_hex(&json!({
        "sectors": sectors,
        "evidence_strength": evidence,
        "min_total_score": criteria.min_total_score,
        "min_flow_score": criteria.min_flow_score,
        "min_fundamental_score": criteria.min_fundamental_score,
        "min_technical_score": criteria.min_technical_score,
    }))
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
    normalized_universes(criteria, rid)?;
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
    // Schema-v1 rows are immutable history. Normalize only the response view
    // so an omitted universe is visible as KOSPI200 without mutating storage.
    let criteria = serde_json::from_value::<ScreenCriteria>(row.criteria_json.clone())
        .ok()
        .and_then(|criteria| canonicalize_criteria(&criteria, "saved-screen").ok())
        .and_then(|criteria| serde_json::to_value(criteria).ok())
        .unwrap_or(row.criteria_json);
    json!({
        "id": row.id,
        "name": row.name,
        "criteria_schema_version": row.criteria_schema_version,
        "criteria": criteria,
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
            universes: None,
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
        let cursor = ScreenCursorV2 {
            cursor_version: SCREEN_CURSOR_V2,
            run_set: vec![ScreenCursorRun {
                universe: "kospi200".into(),
                run_id,
            }],
            criteria_sha256: canonical_criteria_sha256(&criteria(), &[UniverseKey::Kospi200]),
            after_universe: "kospi200".into(),
            after_score: "75.25000000".into(),
            after_instrument: "005930.KRX".into(),
        }
        .encode(&SECRET);
        let DecodedScreenCursor::V2(decoded) =
            decode_cursor(&cursor, &SECRET).expect("valid signed cursor")
        else {
            panic!("expected v2 cursor")
        };
        assert_eq!(decoded.run_set[0].run_id, run_id);
        assert_eq!(decoded.after_score, "75.25000000");
        assert!(domain::InstrumentId::parse(&decoded.after_instrument).is_ok());
        assert!(canonical_score_text(&decoded.after_score).is_some());
        assert_ne!(
            decoded.criteria_sha256,
            canonical_criteria_sha256(
                &ScreenCriteria {
                    min_total_score: Some(61.0),
                    ..criteria()
                },
                &[UniverseKey::Kospi200],
            )
        );
        assert_ne!(
            decoded.run_set[0].run_id,
            Uuid::parse_str("00000000-0000-4000-8000-000000000002").unwrap()
        );
        let mut tampered = cursor;
        tampered.replace_range(0..1, "A");
        assert!(decode_cursor(&tampered, &SECRET).is_err());
    }

    #[test]
    fn explicit_universes_are_sorted_and_duplicates_rejected() {
        let criteria = ScreenCriteria {
            universes: Some(vec!["kosdaq150".into(), "kospi200".into()]),
            ..criteria()
        };
        assert_eq!(
            normalized_universes(&criteria, "test").unwrap(),
            vec![UniverseKey::Kospi200, UniverseKey::Kosdaq150]
        );
        let duplicate = ScreenCriteria {
            universes: Some(vec!["kospi200".into(), "kospi200".into()]),
            ..criteria
        };
        assert!(normalized_universes(&duplicate, "test").is_err());
        assert!(
            serde_json::from_value::<ScreenCriteria>(json!({
                "universes": null,
                "sectors": []
            }))
            .is_err()
        );
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
