//! Authorized-artifact routes: metadata + inline payload, ownership derived
//! from the parent run (direct id guesses and foreign replays are 404). The
//! response never exposes the filesystem path (no direct artifact URL).

use crate::http::dto::ArtifactDto;
use crate::http::error::{api_error, request_id};
use crate::http::session::Session;
use crate::http::state::ApiState;
use crate::http::tenancy_response;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use uuid::Uuid;

pub async fn get(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(artifact_id): Path<String>,
) -> Response {
    let rid = request_id(&headers);
    let actor = session.actor();
    let id = match Uuid::parse_str(&artifact_id) {
        Ok(i) => i,
        Err(_) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "INVALID_PARAMETER",
                "artifact id must be a uuid",
                &rid,
                None,
            );
        }
    };
    match state.artifacts().get_owned(&actor, id).await {
        Ok(row) => {
            let run_id = row.backtest_run_id;
            (
                StatusCode::OK,
                Json(ArtifactDto {
                    id: row.id.to_string(),
                    run_id: run_id.to_string(),
                    artifact_type: row.artifact_type,
                    row_count: row.row_count,
                    sha256: row.sha256,
                    size_bytes: row.size_bytes,
                    summary: row.summary_json,
                    download_path: format!("/api/v1/artifacts/{}/download", row.id),
                }),
            )
                .into_response()
        }
        Err(e) => tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    }
}

pub async fn download(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(artifact_id): Path<String>,
) -> Response {
    let rid = request_id(&headers);
    let actor = session.actor();
    let id = match Uuid::parse_str(&artifact_id) {
        Ok(i) => i,
        Err(_) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "INVALID_PARAMETER",
                "artifact id must be a uuid",
                &rid,
                None,
            );
        }
    };
    match state.artifacts().get_owned(&actor, id).await {
        Ok(row) => {
            let run_id = row.backtest_run_id;
            let summary = row.summary_json.clone();
            let artifact = ArtifactDto {
                id: row.id.to_string(),
                run_id: run_id.to_string(),
                artifact_type: row.artifact_type,
                row_count: row.row_count,
                sha256: row.sha256,
                size_bytes: row.size_bytes,
                summary: summary.clone(),
                download_path: format!("/api/v1/artifacts/{}/download", row.id),
            };
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "artifact": artifact,
                    "payload": summary,
                })),
            )
                .into_response()
        }
        Err(e) => tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    }
}
