//! Daily common candidate feed and deep stock-analysis routes.

use crate::contract::UniverseKey;
use crate::http::error::{api_error, code_error, request_id};
use crate::http::session::Session;
use crate::http::state::ApiState;
use crate::http::validation::parse_date;
use crate::repos::candidates::{CandidateAnalysisRow, CandidateRunRow};
use auth::entitlement::{AccessRequest, CalendarDate, DatasetId, KrUse};
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

const DISCLAIMER: &str = "연구 정보이며 투자 권유, 목표가 또는 수익 확률이 아닙니다.";

pub async fn latest_feed(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Query(params): Query<UniverseParams>,
) -> Response {
    let rid = request_id(&headers);
    let universe = match parse_universe(params.universe.as_deref()) {
        Ok(universe) => universe,
        Err(message) => return code_error("INVALID_PARAMETER", message, &rid),
    };
    feed_response(&state, &session, &headers, universe, None).await
}

pub async fn feed_on_date(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(date): Path<String>,
    Query(params): Query<UniverseParams>,
) -> Response {
    let rid = request_id(&headers);
    let Some(date) = parse_date(&date) else {
        return api_error(
            StatusCode::BAD_REQUEST,
            "INVALID_DATE",
            "feed date must be YYYY-MM-DD",
            &rid,
            None,
        );
    };
    let universe = match parse_universe(params.universe.as_deref()) {
        Ok(universe) => universe,
        Err(message) => return code_error("INVALID_PARAMETER", message, &rid),
    };
    feed_response(&state, &session, &headers, universe, Some(date)).await
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisParams {
    pub date: Option<String>,
    pub universe: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UniverseParams {
    pub universe: Option<String>,
}

fn parse_universe(value: Option<&str>) -> Result<UniverseKey, &'static str> {
    UniverseKey::parse(value.unwrap_or("kospi200"))
        .map_err(|_| "universe must be kospi200 or kosdaq150")
}

pub async fn stock_analysis(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(instrument_id): Path<String>,
    Query(params): Query<AnalysisParams>,
) -> Response {
    let rid = request_id(&headers);
    if domain::InstrumentId::parse(&instrument_id).is_err() {
        return api_error(
            StatusCode::BAD_REQUEST,
            "INVALID_PARAMETER",
            "instrument_id must be a canonical KRX instrument",
            &rid,
            None,
        );
    }
    let as_of = match params.date.as_deref() {
        Some(value) => match parse_date(value) {
            Some(date) => Some(date),
            None => {
                return api_error(
                    StatusCode::BAD_REQUEST,
                    "INVALID_DATE",
                    "date must be YYYY-MM-DD",
                    &rid,
                    None,
                );
            }
        },
        None => None,
    };
    let universe = match parse_universe(params.universe.as_deref()) {
        Ok(universe) => universe,
        Err(message) => return code_error("INVALID_PARAMETER", message, &rid),
    };
    let (run, analysis) = match state
        .candidates()
        .instrument_analysis(&instrument_id, universe.as_str(), as_of)
        .await
    {
        Ok(Some(row)) => row,
        Ok(None) | Err(crate::error::TenancyError::NotFound) => {
            return code_error("RESOURCE_NOT_FOUND", "stock analysis not found", &rid);
        }
        Err(error) => return crate::http::tenancy_response(error, &rid, "RESOURCE_NOT_FOUND"),
    };
    if let Err(response) = gate_run(&state, &session, &headers, &run).await {
        return response;
    }
    let licenses = match state.candidates().license_attributions(run.id).await {
        Ok(rows) => rows,
        Err(error) => return crate::http::tenancy_response(error, &rid, "RESOURCE_NOT_FOUND"),
    };
    let freshness = match research_state(&state, run.as_of_date, &rid).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    no_store(Json(json!({
        "universe": run.universe_key,
        "state": freshness,
        "as_of": run.as_of_date,
        "cutoff_at": run.cutoff_at,
        "scoring_config": {
            "version": run.scoring_config_version,
            "sha256": run.scoring_config_sha256,
        },
        "dataset_pins": dataset_pins(&run),
        "license_attributions": licenses,
        "disclaimer": DISCLAIMER,
        "analysis": analysis_json(analysis),
    })))
}

async fn feed_response(
    state: &ApiState,
    session: &Session,
    headers: &HeaderMap,
    universe: UniverseKey,
    as_of: Option<chrono::NaiveDate>,
) -> Response {
    let rid = request_id(headers);
    let (feed, run) = match state
        .candidates()
        .latest_feed(universe.as_str(), as_of)
        .await
    {
        Ok(Some(rows)) => rows,
        Ok(None) | Err(crate::error::TenancyError::NotFound) => {
            return code_error("RESOURCE_NOT_FOUND", "candidate feed not found", &rid);
        }
        Err(error) => return crate::http::tenancy_response(error, &rid, "RESOURCE_NOT_FOUND"),
    };
    if let Err(response) = gate_run(state, session, headers, &run).await {
        return response;
    }
    let items = match state.candidates().feed_items(feed.id).await {
        Ok(items) => items.into_iter().map(analysis_json).collect::<Vec<_>>(),
        Err(error) => return crate::http::tenancy_response(error, &rid, "RESOURCE_NOT_FOUND"),
    };
    let licenses = match state.candidates().license_attributions(run.id).await {
        Ok(rows) => rows,
        Err(error) => return crate::http::tenancy_response(error, &rid, "RESOURCE_NOT_FOUND"),
    };
    let freshness = match research_state(state, run.as_of_date, &rid).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    no_store(Json(json!({
        "feed_id": feed.id,
        "universe": run.universe_key,
        "state": freshness,
        "as_of": run.as_of_date,
        "cutoff_at": run.cutoff_at,
        "published_at": feed.published_at,
        "computation_seq": feed.computation_seq,
        "scoring_config": {
            "version": run.scoring_config_version,
            "sha256": run.scoring_config_sha256,
        },
        "dataset_pins": dataset_pins(&run),
        "license_attributions": licenses,
        "disclaimer": DISCLAIMER,
        "items": items,
    })))
}

pub(crate) async fn gate_run(
    state: &ApiState,
    session: &Session,
    headers: &HeaderMap,
    run: &CandidateRunRow,
) -> Result<(), Response> {
    let rid = request_id(headers);
    let attributions = state
        .candidates()
        .license_attributions(run.id)
        .await
        .map_err(|error| crate::http::tenancy_response(error, &rid, "RESOURCE_NOT_FOUND"))?;
    // A published run carries the exact entitlement selected when each source
    // was admitted. If any attribution is missing or the contract metadata no
    // longer resolves, fail closed before touching candidate rows.
    let expected_sources = [
        "price",
        "universe",
        "market_status",
        "flow",
        "fundamental",
        "sector",
    ];
    let distinct_dataset_ids = attributions
        .iter()
        .map(|attribution| attribution.dataset_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    if attributions.len() != expected_sources.len()
        || distinct_dataset_ids.len() != expected_sources.len()
        || expected_sources.iter().any(|source| {
            attributions
                .iter()
                .filter(|attribution| attribution.source == *source)
                .count()
                != 1
        })
    {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "DATA_ENTITLEMENT_REQUIRED",
            "candidate source entitlement lineage is incomplete",
            &rid,
            None,
        ));
    }
    let expected_universe_dataset = match run.universe_key.as_str() {
        "kospi200" => "krx_kospi200_membership",
        "kosdaq150" => "krx_kosdaq150_membership",
        _ => {
            return Err(api_error(
                StatusCode::FORBIDDEN,
                "DATA_ENTITLEMENT_REQUIRED",
                "candidate run has an unknown universe identity",
                &rid,
                None,
            ));
        }
    };
    if attributions.iter().any(|attribution| {
        attribution.source == "universe" && attribution.dataset_id != expected_universe_dataset
    }) || !attributions
        .iter()
        .any(|attribution| attribution.source == "universe")
    {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "DATA_ENTITLEMENT_REQUIRED",
            "candidate universe entitlement lineage does not match the run",
            &rid,
            None,
        ));
    }
    let dataset_ids: Vec<&str> = attributions
        .iter()
        .map(|attribution| attribution.dataset_id.as_str())
        .collect();
    let service = crate::http::entitlement::fresh_service(state).await?;
    let as_of = run.as_of_date.format("%Y-%m-%d").to_string();
    let calendar_date = CalendarDate::parse(&as_of).map_err(|_| {
        api_error(
            StatusCode::BAD_REQUEST,
            "INVALID_DATE",
            "candidate run date is invalid",
            &rid,
            None,
        )
    })?;
    for dataset_id in dataset_ids {
        if service
            .authorize_use(
                KrUse::Candidate,
                &AccessRequest {
                    actor: session.actor(),
                    dataset: DatasetId::new(dataset_id),
                    as_of: calendar_date,
                },
            )
            .is_err()
        {
            return Err(api_error(
                StatusCode::FORBIDDEN,
                "DATA_ENTITLEMENT_REQUIRED",
                "an ACTIVE KRX entitlement covering use candidate is required",
                &rid,
                None,
            ));
        }
    }
    Ok(())
}

pub(crate) fn analysis_json(row: CandidateAnalysisRow) -> serde_json::Value {
    json!({
        "analysis_id": row.id,
        "run_id": row.run_id,
        "universe": row.universe_key,
        "instrument_id": row.instrument_id,
        "name": row.instrument_name,
        "sector_code": row.sector_code,
        "fundamental_profile": row.fundamental_profile,
        "eligible": row.eligible,
        "exclusion_codes": row.exclusion_codes,
        "scores": {
            "flow": row.flow_score,
            "fundamental": row.fundamental_score,
            "technical": row.technical_score,
            "total": row.total_score,
        },
        "coverage": {
            "flow": row.flow_coverage,
            "fundamental": row.fundamental_coverage,
            "technical": row.technical_coverage,
        },
        "evidence_strength": row.evidence_strength,
        "rank": row.rank,
        "normalization_scope": row.normalization_scope,
        "factors": row.factors_json,
        "scenarios": row.scenarios_json,
        "provenance": row.provenance_json,
        "content_sha256": row.content_sha256,
    })
}

pub(crate) fn dataset_pins(run: &CandidateRunRow) -> serde_json::Value {
    json!({
        "universe_snapshot_id": run.universe_snapshot_id,
        "price": {
            "dataset_version_id": run.price_dataset_version_id,
            "curated_version": run.price_curated_version,
            "manifest_sha256": run.price_manifest_sha256,
        },
        "market_status": {
            "dataset_version_id": run.status_dataset_version_id,
            "manifest_sha256": run.status_manifest_sha256,
        },
        "flow": {
            "dataset_version_id": run.flow_dataset_version_id,
            "manifest_sha256": run.flow_manifest_sha256,
        },
        "fundamental": {
            "dataset_version_id": run.fundamental_dataset_version_id,
            "manifest_sha256": run.fundamental_manifest_sha256,
        },
        "sector_version_id": run.sector_version_id,
        "input_identity_sha256": run.input_identity_sha256,
    })
}

pub(crate) async fn research_state(
    state: &ApiState,
    as_of: chrono::NaiveDate,
    rid: &str,
) -> Result<&'static str, Response> {
    state
        .candidates()
        .freshness_state(as_of)
        .await
        .map_err(|error| crate::http::tenancy_response(error, rid, "RESOURCE_NOT_FOUND"))
}

fn no_store<T: serde::Serialize>(body: Json<T>) -> Response {
    let mut response = (StatusCode::OK, body).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}
