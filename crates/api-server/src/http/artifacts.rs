//! Authorized-artifact routes: metadata + internal `X-Accel-Redirect`
//! delivery, ownership derived from the parent run (direct id guesses and
//! foreign replays are 404).
//!
//! Download flow (design §7.3 "결과 Artifact 접근은 DB 권한 확인 후"): the
//! API authorizes ownership (RLS-scoped join), the KR-data entitlement, the
//! stored path (safe relative, no traversal), and the on-disk file against
//! the manifest sha256 — and ONLY then answers with the INTERNAL
//! `X-Accel-Redirect: /internal-artifacts/<rel>` header that Nginx alone can
//! follow. The response never exposes a filesystem path or inline payload;
//! Nginx's `internal;` location + `disable_symlinks on` (deploy/nginx) close
//! direct requests and symlink escapes.

use crate::http::dto::ArtifactDto;
use crate::http::entitlement::{require_use, today_iso, use_from_name};
use crate::http::error::{api_error, request_id};
use crate::http::session::Session;
use crate::http::state::ApiState;
use crate::http::tenancy_response;
use crate::repos::artifacts::{derive_internal_path, media_type};
use axum::Json;
use axum::extract::{Path as UrlPath, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};
use std::path::Path as FsPath;
use tokio::io::AsyncReadExt;
use uuid::Uuid;

/// The internal alias prefix Nginx mounts the artifact tree under.
const INTERNAL_PREFIX: &str = "/internal-artifacts/";

pub async fn get(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    UrlPath(artifact_id): UrlPath<String>,
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
    if let Err(resp) = require_use(&state, &session, &headers, use_from_name("download").unwrap(), &today_iso()).await {
        return resp;
    }
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
    UrlPath(artifact_id): UrlPath<String>,
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
    // 1. Ownership: RLS-scoped parent-run join. Foreign/direct-id -> 404
    //    (indistinguishable from missing, by design).
    let row = match state.artifacts().get_owned(&actor, id).await {
        Ok(r) => r,
        Err(e) => return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    };
    // 2. KR-data entitlement (fail closed on PENDING/EXPIRED/REVOKED).
    if let Err(resp) =
        require_use(&state, &session, &headers, use_from_name("download").unwrap(), &today_iso()).await
    {
        return resp;
    }
    // 3. Safe internal path: no absolute path, no traversal, no drive/UNC.
    let rel = match derive_internal_path(&row.parquet_path) {
        Ok(r) => r,
        Err(e) => return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    };
    let media = media_type(&row.artifact_type);
    // 4. Manifest hash vs the on-disk file (streaming; never a path in any
    //    error body).
    if let Err(e) = verify_manifest_hash(&state.cfg.artifact_root, &rel, &row.sha256).await {
        return tenancy_response(e, &rid, "RESULT_INTEGRITY_FAILED");
    }
    (
        StatusCode::OK,
        [
            ("X-Accel-Redirect", format!("{INTERNAL_PREFIX}{rel}")),
            ("Content-Type", media.to_string()),
            ("Content-Length", row.size_bytes.to_string()),
            ("X-Lagrange-Sha256", row.sha256.clone()),
            ("Cache-Control", "no-store".to_string()),
        ],
        "",
    )
        .into_response()
}

/// Stream an artifact file and compare its sha256 against the manifest.
/// A missing file, an unreadable file, or a hash mismatch is a typed
/// `ResultIntegrity` failure (422 RESULT_INTEGRITY_FAILED upstream).
async fn verify_manifest_hash(
    root: &FsPath,
    rel: &str,
    expected_hex: &str,
) -> Result<(), crate::error::TenancyError> {
    let path = root.join(rel);
    let mut file = tokio::fs::File::open(&path).await.map_err(|e| {
        crate::error::TenancyError::ResultIntegrity(format!("artifact file unreadable: {e}"))
    })?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).await.map_err(|e| {
            crate::error::TenancyError::ResultIntegrity(format!("artifact file read failed: {e}"))
        })?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let actual = hex::encode(hasher.finalize());
    if actual != expected_hex {
        return Err(crate::error::TenancyError::ResultIntegrity(
            "artifact file does not match the manifest hash".to_string(),
        ));
    }
    Ok(())
}
