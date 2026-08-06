//! Backtest orchestration routes: create (validation + dataset/entitlement/
//! capacity gates + queue enqueue), cancel, get, metrics/equity/trades reads,
//! robustness enqueue, and compare. Results stay in Parquet manifests; the
//! API serves the DB manifests and summaries only.

use crate::http::dto::{
    ArtifactDto, BacktestBody, BacktestRunDto, CancelDto, CompareBody, CompareDto, CompareRunDto,
    EquityDto, MetricDto, PageDto, RobustnessDto, TradeDto,
};
use crate::http::entitlement::require_use;
use crate::http::error::{api_error, code_error, request_id};
use crate::http::pagination::Cursor;
use crate::http::session::{PageParams, Session};
use crate::http::state::ApiState;
use crate::http::validation::{is_supported_currency, is_valid_decimal, parse_date, sha256_hex};
use crate::http::{JsonBody, audit, idempotent, tenancy_response};
use crate::repos::backtest_runs::NewBacktestRun;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use uuid::Uuid;

pub async fn create(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    JsonBody(body): JsonBody<BacktestBody>,
) -> Response {
    let rid = request_id(&headers);
    if let Err(r) = crate::http::session::require_csrf(&headers, &session.0) {
        return r;
    }
    let actor = session.actor();

    // --- validation (typed 4xx, zero side effects) -------------------------
    let Some(start) = parse_date(&body.start_date) else {
        return bad_date(&rid, "start_date");
    };
    let Some(end) = parse_date(&body.end_date) else {
        return bad_date(&rid, "end_date");
    };
    if start > end {
        return api_error(
            StatusCode::BAD_REQUEST,
            "INVALID_PARAMETER",
            "start_date must not be after end_date",
            &rid,
            None,
        );
    }
    if !is_supported_currency(&body.initial_cash.currency) {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "UNSUPPORTED_MARKET_CURRENCY",
            format!(
                "currency {} is not supported (KRW first)",
                body.initial_cash.currency
            ),
            &rid,
            None,
        );
    }
    if !is_valid_decimal(&body.initial_cash.amount) {
        return api_error(
            StatusCode::BAD_REQUEST,
            "INVALID_DECIMAL",
            "initial_cash.amount must be a finite fixed-point decimal string",
            &rid,
            None,
        );
    }
    if !crate::http::validation::in_fixed_universe(&body.benchmark) {
        return api_error(
            StatusCode::BAD_REQUEST,
            "INVALID_PARAMETER",
            format!(
                "benchmark {} is not a member of the fixed Korean ETF v1 universe",
                body.benchmark
            ),
            &rid,
            None,
        );
    }
    let cfg_id = match Uuid::parse_str(&body.strategy_config_id) {
        Ok(i) => i,
        Err(_) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "INVALID_PARAMETER",
                "strategy_config_id must be a uuid",
                &rid,
                None,
            );
        }
    };
    let dataset_id = match Uuid::parse_str(&body.dataset_version_id) {
        Ok(i) => i,
        Err(_) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "INVALID_PARAMETER",
                "dataset_version_id must be a uuid",
                &rid,
                None,
            );
        }
    };

    // --- gates -------------------------------------------------------------
    if let Err(r) = require_use(
        &state,
        &session,
        &headers,
        auth::entitlement::KrUse::Backtest,
        &body.end_date,
    )
    .await
    {
        return r;
    }
    let active_jobs = match state.ops().count_active_jobs(&actor).await {
        Ok(n) => n,
        Err(e) => return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    };
    if active_jobs >= state.cfg.max_jobs_per_owner as i64 {
        return api_error(
            StatusCode::TOO_MANY_REQUESTS,
            "BACKTEST_CAPACITY_EXCEEDED",
            format!(
                "per-owner queued job capacity ({}) exceeded",
                state.cfg.max_jobs_per_owner
            ),
            &rid,
            None,
        );
    }

    let body_value = serde_json::to_value(&body).unwrap_or_default();
    let body_hash = crate::http::idempotency::body_hash(&body_value);
    let key = crate::http::idempotency::key_from(&headers);
    idempotent(&state, &session, &headers, &body_hash, async {
        // Dataset gate: READY only; WARNING with stale/blocking issues fails.
        let shared = state.shared();
        let dataset = match shared.get_dataset_version_by_id(&actor, dataset_id).await {
            Ok(d) => d,
            Err(e) => return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
        };
        if dataset.status == "BLOCKED" {
            let issues = match shared
                .dataset_issues(&actor, &dataset.dataset_id, &dataset.version)
                .await
            {
                Ok(i) => i,
                Err(e) => return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
            };
            return api_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "DATASET_BLOCKED",
                format!("dataset {} is quality-blocked", dataset.dataset_id),
                &rid,
                Some(serde_json::json!({
                    "dataset_id": dataset.dataset_id,
                    "version": dataset.version,
                    "issues": issues.iter().map(|i| serde_json::json!({
                        "issue_code": i.issue_code,
                        "severity": i.severity,
                    })).collect::<Vec<_>>(),
                })),
            );
        }
        if dataset.status == "WARNING" {
            let issues = match shared
                .dataset_issues(&actor, &dataset.dataset_id, &dataset.version)
                .await
            {
                Ok(i) => i,
                Err(e) => return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
            };
            let blocking: Vec<String> = issues
                .iter()
                .filter(|i| i.severity == "ERROR")
                .map(|i| i.issue_code.clone())
                .collect();
            if !blocking.is_empty() {
                return api_error(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "DATASET_BLOCKED",
                    format!(
                        "dataset {} has blocking quality issues: {}",
                        dataset.dataset_id,
                        blocking.join(",")
                    ),
                    &rid,
                    None,
                );
            }
            if issues.iter().any(|i| i.issue_code == "DATA_STALE") {
                return api_error(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "DATA_STALE",
                    format!(
                        "dataset {} is stale for the requested as-of",
                        dataset.dataset_id
                    ),
                    &rid,
                    None,
                );
            }
        }
        // Config ownership + strategy resolution.
        let config = match state.strategy_configs().get(&actor, cfg_id).await {
            Ok(c) => c,
            Err(e) => return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
        };
        // Create the PENDING run (config hash is the canonical sha256 hex).
        let run = match state
            .backtest_runs()
            .create(
                &actor,
                NewBacktestRun {
                    strategy_id: config.strategy_id.clone(),
                    strategy_version: config.strategy_version.clone(),
                    dataset_version: format!("{}@{}", dataset.dataset_id, dataset.version),
                    engine_version: "1.231.0".to_string(),
                    config_sha256: sha256_hex(&config.config_json),
                    code_commit: "PENDING".to_string(),
                    random_seed: None,
                    timezone: "Asia/Seoul".to_string(),
                    summary_json: serde_json::json!({}),
                },
            )
            .await
        {
            Ok(r) => r,
            Err(e) => return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
        };
        // Enqueue the backtest job (queue-level idempotency key rides along).
        let queue = match state.queue_for(&actor).await {
            Ok(q) => q,
            Err(_) => return code_error("INTERNAL", "queue unavailable", &rid),
        };
        let job = match queue
            .submit(job_queue::SubmitJob {
                owner_user_id: crate::actor_tx::actor_uuid(&actor).unwrap_or_default(),
                job_type: "backtest".to_string(),
                payload: serde_json::json!({
                    "kind": "backtest",
                    "run_id": run.id,
                    "strategy_config_id": cfg_id,
                    "dataset_version_id": dataset_id,
                    "start_date": body.start_date,
                    "end_date": body.end_date,
                    "initial_cash": body.initial_cash,
                    "benchmark": body.benchmark,
                    "cost_profile_id": body.cost_profile_id,
                    "execution_profile": body.execution_profile,
                }),
                priority: 10,
                idempotency_key: key,
                max_attempts: 3,
                available_at: None,
            })
            .await
        {
            Ok(j) => j,
            Err(e) => return code_error("INTERNAL", format!("enqueue failed: {e}"), &rid),
        };
        // Persist the run -> job link (actor-scoped UPDATE).
        let linked = match crate::actor_tx::begin_actor_tx(&state.app_pool, &actor).await {
            Ok(mut tx) => {
                let r = sqlx::query(
                    "UPDATE backtest_runs SET job_id = $2 WHERE id = $1 AND job_id IS NULL",
                )
                .bind(run.id)
                .bind(job.id)
                .execute(&mut *tx)
                .await
                .map_err(crate::error::TenancyError::from_sqlx);
                match r {
                    Ok(_) => tx
                        .commit()
                        .await
                        .map_err(crate::error::TenancyError::from_sqlx),
                    Err(e) => Err(e),
                }
            }
            Err(e) => Err(e),
        };
        if let Err(e) = linked {
            return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND");
        }
        audit(
            &state,
            &session,
            &headers,
            "backtest.create",
            "backtest_run",
            &run.id.to_string(),
            None,
            Some(serde_json::json!({
                "strategy_config_id": cfg_id,
                "dataset_version_id": dataset_id,
                "job_id": job.id,
            })),
            None,
        )
        .await;
        (
            StatusCode::CREATED,
            Json(BacktestRunDto {
                id: run.id.to_string(),
                strategy_id: run.strategy_id,
                strategy_version: run.strategy_version,
                dataset_version: run.dataset_version,
                engine: run.engine,
                engine_version: run.engine_version,
                status: run.status,
                job_id: Some(job.id.to_string()),
                config_sha256: run.config_sha256,
                benchmark: Some(body.benchmark),
                start_date: Some(start),
                end_date: Some(end),
                started_at: run.started_at,
                finished_at: run.finished_at,
                created_at: run.created_at,
                summary: run.summary_json,
            }),
        )
            .into_response()
    })
    .await
}

pub async fn list(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Query(params): Query<PageParams>,
) -> Response {
    let rid = request_id(&headers);
    let actor = session.actor();
    if let Err(r) = require_use(
        &state,
        &session,
        &headers,
        auth::entitlement::KrUse::Backtest,
        &crate::http::entitlement::today_iso(),
    )
    .await
    {
        return r;
    }
    let cursor = match decode_cursor(&state, &rid, params.cursor.as_deref()) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let limit = params.limit_or(PageParams::DEFAULT_LIMIT);
    match state
        .backtest_runs()
        .list_page(&actor, cursor.as_ref(), limit)
        .await
    {
        Ok((rows, next)) => {
            let next = next.map(|c| c.encode(&state.cfg.cursor_secret));
            let items = rows
                .into_iter()
                .map(|r| BacktestRunDto {
                    id: r.id.to_string(),
                    strategy_id: r.strategy_id,
                    strategy_version: r.strategy_version,
                    dataset_version: r.dataset_version,
                    engine: r.engine,
                    engine_version: r.engine_version,
                    status: r.status,
                    job_id: r.job_id.map(|j| j.to_string()),
                    config_sha256: r.config_sha256,
                    benchmark: None,
                    start_date: None,
                    end_date: None,
                    started_at: r.started_at,
                    finished_at: r.finished_at,
                    created_at: r.created_at,
                    summary: r.summary_json,
                })
                .collect();
            (StatusCode::OK, Json(PageDto::new(items, next))).into_response()
        }
        Err(e) => tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    }
}

pub async fn get(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(run_id): Path<String>,
) -> Response {
    let rid = request_id(&headers);
    let actor = session.actor();
    let id = match parse_uuid(&rid, "run id", &run_id) {
        Ok(i) => i,
        Err(r) => return r,
    };
    if let Err(r) = require_use(
        &state,
        &session,
        &headers,
        auth::entitlement::KrUse::Backtest,
        &crate::http::entitlement::today_iso(),
    )
    .await
    {
        return r;
    }
    match state.backtest_runs().get(&actor, id).await {
        Ok(r) => (
            StatusCode::OK,
            Json(BacktestRunDto {
                id: r.id.to_string(),
                strategy_id: r.strategy_id,
                strategy_version: r.strategy_version,
                dataset_version: r.dataset_version,
                engine: r.engine,
                engine_version: r.engine_version,
                status: r.status,
                job_id: r.job_id.map(|j| j.to_string()),
                config_sha256: r.config_sha256,
                benchmark: None,
                start_date: None,
                end_date: None,
                started_at: r.started_at,
                finished_at: r.finished_at,
                created_at: r.created_at,
                summary: r.summary_json,
            }),
        )
            .into_response(),
        Err(e) => tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    }
}

pub async fn cancel(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(run_id): Path<String>,
    JsonBody(_body): JsonBody<crate::http::dto::EmptyBody>,
) -> Response {
    let rid = request_id(&headers);
    if let Err(r) = crate::http::session::require_csrf(&headers, &session.0) {
        return r;
    }
    let actor = session.actor();
    let id = match parse_uuid(&rid, "run id", &run_id) {
        Ok(i) => i,
        Err(r) => return r,
    };
    let body_hash = crate::http::idempotency::body_hash(&serde_json::json!({}));
    idempotent(&state, &session, &headers, &body_hash, async {
        let run = match state.backtest_runs().get(&actor, id).await {
            Ok(r) => r,
            Err(e) => return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
        };
        let Some(job_id) = run.job_id else {
            return code_error("INVALID_PARAMETER", "run has no job to cancel", &rid);
        };
        let queue = match state.queue_for(&actor).await {
            Ok(q) => q,
            Err(_) => return code_error("INTERNAL", "queue unavailable", &rid),
        };
        let actor_for_audit = job_queue::AuditActor {
            role: if session.actor().is_owner() {
                "owner"
            } else {
                "member"
            }
            .to_string(),
            user_id: crate::actor_tx::actor_uuid(&session.actor()).ok(),
            correlation_id: Some(rid.clone()),
        };
        if let Err(e) = queue.request_cancel(job_id, &actor_for_audit).await {
            return code_error("INTERNAL", format!("cancel failed: {e}"), &rid);
        }
        audit(
            &state,
            &session,
            &headers,
            "backtest.cancel",
            "backtest_run",
            &run.id.to_string(),
            Some(serde_json::json!({ "job_id": job_id })),
            Some(serde_json::json!({ "requested": true })),
            None,
        )
        .await;
        (
            StatusCode::OK,
            Json(CancelDto {
                run_id: run.id.to_string(),
                job_id: Some(job_id.to_string()),
                status: "CANCEL_REQUESTED",
            }),
        )
            .into_response()
    })
    .await
}

pub async fn metrics(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(run_id): Path<String>,
) -> Response {
    let rid = request_id(&headers);
    let actor = session.actor();
    let id = match parse_uuid(&rid, "run id", &run_id) {
        Ok(i) => i,
        Err(r) => return r,
    };
    if let Err(r) = require_use(
        &state,
        &session,
        &headers,
        auth::entitlement::KrUse::Backtest,
        &crate::http::entitlement::today_iso(),
    )
    .await
    {
        return r;
    }
    if let Err(e) = state.backtest_runs().get(&actor, id).await {
        return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND");
    }
    match state.metrics().metrics(&actor, id).await {
        Ok(rows) => {
            let items: Vec<MetricDto> = rows
                .into_iter()
                .map(|m| MetricDto {
                    metric_key: m.metric_key,
                    metric_value: m.metric_value,
                })
                .collect();
            (StatusCode::OK, Json(PageDto::new(items, None))).into_response()
        }
        Err(e) => tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    }
}

pub async fn equity(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(run_id): Path<String>,
) -> Response {
    let rid = request_id(&headers);
    let actor = session.actor();
    let id = match parse_uuid(&rid, "run id", &run_id) {
        Ok(i) => i,
        Err(r) => return r,
    };
    if let Err(r) = require_use(
        &state,
        &session,
        &headers,
        auth::entitlement::KrUse::Backtest,
        &crate::http::entitlement::today_iso(),
    )
    .await
    {
        return r;
    }
    if let Err(e) = state.backtest_runs().get(&actor, id).await {
        return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND");
    }
    let artifacts = match state.metrics().artifacts(&actor, id).await {
        Ok(a) => a,
        Err(e) => return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    };
    let curve = artifacts
        .into_iter()
        .find(|a| a.artifact_type == "EQUITY_CURVE")
        .ok_or_else(|| {
            api_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "RESULT_INTEGRITY_FAILED",
                "run is SUCCEEDED but has no verified EQUITY_CURVE artifact",
                &rid,
                None,
            )
        });
    match curve {
        Ok(c) => {
            let summary = c.summary_json.clone();
            (
                StatusCode::OK,
                Json(EquityDto {
                    run_id: id.to_string(),
                    artifact: artifact_dto(c, id),
                    summary,
                }),
            )
                .into_response()
        }
        Err(r) => r,
    }
}

pub async fn trades(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(run_id): Path<String>,
) -> Response {
    let rid = request_id(&headers);
    let actor = session.actor();
    let id = match parse_uuid(&rid, "run id", &run_id) {
        Ok(i) => i,
        Err(r) => return r,
    };
    if let Err(r) = require_use(
        &state,
        &session,
        &headers,
        auth::entitlement::KrUse::Backtest,
        &crate::http::entitlement::today_iso(),
    )
    .await
    {
        return r;
    }
    if let Err(e) = state.backtest_runs().get(&actor, id).await {
        return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND");
    }
    match state.metrics().artifacts(&actor, id).await {
        Ok(artifacts) => {
            let items: Vec<TradeDto> = artifacts
                .into_iter()
                .filter(|a| a.artifact_type == "ORDERS" || a.artifact_type == "FILLS")
                .map(|a| TradeDto {
                    run_id: id.to_string(),
                    artifact_type: a.artifact_type.clone(),
                    artifact: artifact_dto(a, id),
                })
                .collect();
            (StatusCode::OK, Json(PageDto::new(items, None))).into_response()
        }
        Err(e) => tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    }
}

pub async fn robustness(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(run_id): Path<String>,
    JsonBody(_body): JsonBody<crate::http::dto::EmptyBody>,
) -> Response {
    let rid = request_id(&headers);
    if let Err(r) = crate::http::session::require_csrf(&headers, &session.0) {
        return r;
    }
    let actor = session.actor();
    let id = match parse_uuid(&rid, "run id", &run_id) {
        Ok(i) => i,
        Err(r) => return r,
    };
    let body_hash = crate::http::idempotency::body_hash(&serde_json::json!({}));
    idempotent(&state, &session, &headers, &body_hash, async {
        let run = match state.backtest_runs().get(&actor, id).await {
            Ok(r) => r,
            Err(e) => return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
        };
        if run.status != "SUCCEEDED" {
            return api_error(
                StatusCode::BAD_REQUEST,
                "INVALID_PARAMETER",
                format!(
                    "robustness requires a SUCCEEDED run (run is {})",
                    run.status
                ),
                &rid,
                None,
            );
        }
        let queue = match state.queue_for(&actor).await {
            Ok(q) => q,
            Err(_) => return code_error("INTERNAL", "queue unavailable", &rid),
        };
        let job = match queue
            .submit(job_queue::SubmitJob {
                owner_user_id: crate::actor_tx::actor_uuid(&actor).unwrap_or_default(),
                job_type: "backtest".to_string(),
                payload: serde_json::json!({ "kind": "robustness", "run_id": id }),
                priority: 10,
                idempotency_key: crate::http::idempotency::key_from(&headers),
                max_attempts: 3,
                available_at: None,
            })
            .await
        {
            Ok(j) => j,
            Err(e) => return code_error("INTERNAL", format!("enqueue failed: {e}"), &rid),
        };
        audit(
            &state,
            &session,
            &headers,
            "backtest.robustness",
            "backtest_run",
            &run.id.to_string(),
            None,
            Some(serde_json::json!({ "job_id": job.id })),
            None,
        )
        .await;
        (
            StatusCode::OK,
            Json(RobustnessDto {
                run_id: run.id.to_string(),
                job_id: job.id.to_string(),
                status: "QUEUED",
            }),
        )
            .into_response()
    })
    .await
}

pub async fn compare(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    JsonBody(body): JsonBody<CompareBody>,
) -> Response {
    let rid = request_id(&headers);
    let actor = session.actor();
    if let Err(r) = require_use(
        &state,
        &session,
        &headers,
        auth::entitlement::KrUse::Backtest,
        &crate::http::entitlement::today_iso(),
    )
    .await
    {
        return r;
    }
    if body.run_ids.len() < 2 {
        return api_error(
            StatusCode::BAD_REQUEST,
            "INVALID_PARAMETER",
            "compare requires at least two run ids",
            &rid,
            None,
        );
    }
    let mut ids = Vec::with_capacity(body.run_ids.len());
    for raw in &body.run_ids {
        match Uuid::parse_str(raw) {
            Ok(i) => ids.push(i),
            Err(_) => {
                return api_error(
                    StatusCode::BAD_REQUEST,
                    "INVALID_PARAMETER",
                    format!("run id {raw} is not a uuid"),
                    &rid,
                    None,
                );
            }
        }
    }
    let mut runs = Vec::with_capacity(ids.len());
    for id in &ids {
        let run = match state.backtest_runs().get(&actor, *id).await {
            Ok(r) => r,
            Err(e) => return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
        };
        if run.status != "SUCCEEDED" {
            return api_error(
                StatusCode::BAD_REQUEST,
                "INVALID_PARAMETER",
                format!(
                    "compare requires SUCCEEDED runs (run {} is {})",
                    run.id, run.status
                ),
                &rid,
                None,
            );
        }
        runs.push(CompareRunDto {
            run_id: run.id.to_string(),
            strategy_id: run.strategy_id,
            status: run.status,
            summary: run.summary_json,
        });
    }
    let deltas = compare_deltas(&runs);
    (
        StatusCode::OK,
        Json(CompareDto {
            run_ids: ids.iter().map(|i| i.to_string()).collect(),
            runs,
            deltas,
        }),
    )
        .into_response()
}

/// Decimal-string subtraction at scale 8 for the compare deltas.
fn compare_deltas(runs: &[CompareRunDto]) -> serde_json::Value {
    let mut out = serde_json::Map::new();
    let first = &runs[0].summary;
    let last = runs.last().map(|r| &r.summary).unwrap_or(first);
    for key in ["total_return", "cagr", "mdd"] {
        let a = first.get(key).and_then(|v| v.as_str());
        let b = last.get(key).and_then(|v| v.as_str());
        if let (Some(a), Some(b)) = (a, b) {
            // delta = last run minus first run (the compared difference).
            if let Some(diff) = decimal_diff(a, b) {
                out.insert(key.to_string(), serde_json::json!(diff));
            }
        }
    }
    serde_json::Value::Object(out)
}

fn decimal_diff(a: &str, b: &str) -> Option<String> {
    let parse = |s: &str| -> Option<i128> {
        let neg = s.starts_with('-');
        let s = s.trim_start_matches('-');
        let (int, frac) = match s.split_once('.') {
            Some((i, f)) => (i, f),
            None => (s, ""),
        };
        let int: i128 = int.parse().ok()?;
        let frac_len = frac.len();
        let frac: i128 = frac.parse().ok()?;
        let scale = 10i128.pow(8 - frac_len as u32);
        let v = int * 10i128.pow(8) + frac * scale;
        Some(if neg { -v } else { v })
    };
    let (av, bv) = (parse(a)?, parse(b)?);
    let diff = bv - av;
    let neg = diff < 0;
    let mag = diff.unsigned_abs();
    let int = mag / 10_000_000;
    let frac = mag % 10_000_000;
    let frac = format!("{frac:08}");
    let frac = frac.trim_end_matches('0');
    Some(format!(
        "{}{}.{}",
        if neg { "-" } else { "" },
        int,
        if frac.is_empty() { "0" } else { frac }
    ))
}

fn artifact_dto(a: crate::repos::artifacts::ArtifactRow, run_id: Uuid) -> ArtifactDto {
    ArtifactDto {
        id: a.id.to_string(),
        run_id: run_id.to_string(),
        artifact_type: a.artifact_type,
        row_count: a.row_count,
        sha256: a.sha256,
        size_bytes: a.size_bytes,
        summary: a.summary_json,
        download_path: format!("/api/v1/artifacts/{}/download", a.id),
    }
}

fn bad_date(rid: &str, field: &str) -> Response {
    api_error(
        StatusCode::BAD_REQUEST,
        "INVALID_DATE",
        format!("{field} must be a valid YYYY-MM-DD calendar date"),
        rid,
        None,
    )
}

#[allow(clippy::result_large_err)]
fn parse_uuid(rid: &str, what: &str, raw: &str) -> Result<Uuid, Response> {
    Uuid::parse_str(raw).map_err(|_| {
        api_error(
            StatusCode::BAD_REQUEST,
            "INVALID_PARAMETER",
            format!("{what} must be a uuid"),
            rid,
            None,
        )
    })
}

#[allow(clippy::result_large_err)]
fn decode_cursor(
    state: &ApiState,
    rid: &str,
    raw: Option<&str>,
) -> Result<Option<Cursor>, Response> {
    match raw {
        None => Ok(None),
        Some(r) => match Cursor::decode(r, &state.cfg.cursor_secret) {
            Ok(c) => Ok(Some(c)),
            Err(_) => Err(code_error(
                "INVALID_CURSOR",
                "pagination cursor is invalid",
                rid,
            )),
        },
    }
}
