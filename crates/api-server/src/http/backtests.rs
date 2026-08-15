//! Backtest orchestration routes: create (validation + dataset/entitlement/
//! capacity gates + queue enqueue), cancel, get, metrics/equity/trades reads,
//! robustness enqueue, and compare. Results stay in Parquet manifests; the
//! API serves the DB manifests and summaries only.

use crate::http::dto::{
    ArtifactDto, BacktestBody, BacktestRunDto, CancelDto, CompareBody, CompareDto, CompareRunDto,
    EquityDto, MetricDto, PageDto, RobustnessChildDto, RobustnessDto, TradeDto,
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
    let code_commit = state.cfg.code_commit.clone();

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
    // The cost profile must resolve HERE, not in the runner.
    //
    // This route used to pass `cost_profile_id` into the job payload without
    // looking at it, so an id that resolved nowhere became a job that failed
    // minutes later in a worker -- if it failed at all. The paper route has
    // always rejected the same input at submission. Both now call the one
    // resolver in `portfolio-model`.
    if let Err(e) = portfolio_model::cost::CostProfile::resolve(&body.cost_profile_id) {
        use portfolio_model::cost::CostProfileLookupError as E;
        // `CUSTOM` is a real identity whose rates this body does not carry;
        // saying "unknown" would send someone hunting for a typo that is not
        // there.
        let code = match e {
            E::CustomNotConfigurable => "UNSUPPORTED_COST_PROFILE",
            _ => "INVALID_PARAMETER",
        };
        return api_error(
            StatusCode::BAD_REQUEST,
            code,
            format!("cost_profile_id {}: {e}", body.cost_profile_id),
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
        // Reserve global owner capacity and create the run + job atomically.
        let job_payload = serde_json::json!({
            "kind": "backtest",
            "strategy_config_id": cfg_id,
            "dataset_version_id": dataset_id,
            "start_date": body.start_date,
            "end_date": body.end_date,
            "initial_cash": body.initial_cash,
            "benchmark": body.benchmark,
            "cost_profile_id": body.cost_profile_id,
            "execution_profile": body.execution_profile,
            "robustness": body.robustness,
        });
        let run = match state
            .backtest_runs()
            .create_and_enqueue(
                &actor,
                crate::repos::backtest_runs::NewQueuedBacktest {
                    run: NewBacktestRun {
                        strategy_id: config.strategy_id.clone(),
                        strategy_version: config.strategy_version.clone(),
                        dataset_version: format!("{}@{}", dataset.dataset_id, dataset.version),
                        engine_version: "1.231.0".to_string(),
                        config_sha256: sha256_hex(&config.config_json),
                        code_commit: code_commit.clone(),
                        // The seed is persisted before execution and is part
                        // of the worker's exact provenance contract.
                        random_seed: Some(42),
                        timezone: "Asia/Seoul".to_string(),
                        summary_json: serde_json::json!({}),
                    },
                    payload: job_payload,
                    idempotency_key: key,
                    max_jobs_per_owner: state.cfg.max_jobs_per_owner,
                },
            )
            .await
        {
            Ok(r) => r,
            Err(crate::repos::backtest_runs::SubmitBacktestError::CapacityExceeded) => {
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
            Err(crate::repos::backtest_runs::SubmitBacktestError::IdempotencyMismatch) => {
                return code_error(
                    "IDEMPOTENCY_KEY_MISMATCH",
                    "the same Idempotency-Key was already used with a different request body",
                    &rid,
                );
            }
            Err(crate::repos::backtest_runs::SubmitBacktestError::Tenancy(e)) => {
                return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND");
            }
        };
        let job_id = run
            .job_id
            .expect("atomic backtest submission links its job");
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
                "job_id": job_id,
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
                job_id: Some(job_id.to_string()),
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
        // Cascade: any robustness suite planned from this run cancels with
        // it. A suite child is never left running against a canceled parent.
        if let Ok(Some(suite)) = state.robustness_suites().find_by_parent(&actor, id).await
            && let Ok((_, children)) = state
                .robustness_suites()
                .suite_status(&actor, suite.id)
                .await
        {
            let child_job_ids: Vec<Uuid> = children.iter().map(|c| c.job_id).collect();
            let _ = job_queue::batch::cancel_batch(&queue, &child_job_ids, &actor_for_audit).await;
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

/// Builds the parent run's [`RunProvenance`] from its persisted columns.
/// Every robustness child pins THIS provenance verbatim (except the single
/// declared axis) -- the caller never supplies strategy/data/engine
/// identity, so a pin mismatch can only ever be a server bug, never a
/// client input (plan Todo 29: version pinning is structural, not a
/// runtime check on user input).
/// Sanitizes a raw `dataset_version` column value (Todo 20's `create()`
/// stores `"{dataset_id}@{version}"`, and `@` is outside
/// [`domain::DatasetVersionId`]'s slug alphabet) into a valid slug. This
/// identity is used ONLY to compare a suite's children against their
/// parent for equality -- not to look anything up externally -- so a
/// stable, deterministic substitution preserves correctness.
fn sanitize_dataset_version(raw: &str) -> String {
    raw.chars()
        .map(|c| if c == '@' { '.' } else { c })
        .collect()
}

fn parent_provenance(
    run: &crate::repos::backtest_runs::BacktestRunRow,
) -> Result<domain::provenance::RunProvenance, String> {
    use domain::provenance::{Engine, RandomSeed, RunProvenance};
    use domain::version::{SemVer, StrategyVersion};
    use domain::{CodeCommit, ContentHash, DatasetVersionId, StrategyId, Zone};

    let engine = match run.engine.as_str() {
        "nautilustrader" => Engine::NautilusTrader,
        other => return Err(format!("unknown engine {other}")),
    };
    let code_commit = CodeCommit::parse(&run.code_commit).map_err(|e| e.to_string())?;
    let random_seed = run
        .random_seed
        .ok_or_else(|| "backtest run has no persisted random seed".to_owned())?;
    if random_seed < 0 {
        return Err("backtest run random seed must be non-negative".to_owned());
    }
    Ok(RunProvenance {
        engine,
        engine_version: SemVer::parse(&run.engine_version).map_err(|e| e.to_string())?,
        strategy_id: StrategyId::parse(&run.strategy_id).map_err(|e| e.to_string())?,
        strategy_version: StrategyVersion::parse(&run.strategy_version)
            .map_err(|e| e.to_string())?,
        dataset_version: DatasetVersionId::parse(&sanitize_dataset_version(&run.dataset_version))
            .map_err(|e| e.to_string())?,
        config_hash: ContentHash::parse(&format!("sha256:{}", run.config_sha256))
            .map_err(|e| e.to_string())?,
        code_commit,
        random_seed: RandomSeed::new(random_seed as u64),
        timezone: Zone::from_name(&run.timezone).map_err(|e| e.to_string())?,
    })
}

/// The standard cost-stress pair the zero-configuration "Run robustness
/// evidence" button requests when the caller supplies no `axes` (FR-ROB-003:
/// adverse and extreme cost/slippage scenarios compared against the base
/// run).
fn default_axes() -> Vec<result_model::robustness::DerivedAxis> {
    use result_model::robustness::DerivedAxis;
    vec![
        DerivedAxis::CostStress {
            profile_id: "adverse".to_string(),
            profile_version: 1,
        },
        DerivedAxis::CostStress {
            profile_id: "extreme".to_string(),
            profile_version: 1,
        },
    ]
}

fn robustness_error_response(
    rid: &str,
    err: result_model::robustness::RobustnessError,
) -> Response {
    use result_model::robustness::RobustnessError;
    let message = err.to_string();
    match err {
        RobustnessError::GridTooLarge { .. }
        | RobustnessError::MultiAxisChange { .. }
        | RobustnessError::HoldoutViolation { .. }
        | RobustnessError::PinMismatch { .. }
        | RobustnessError::EmptySeries { .. }
        | RobustnessError::InvalidSplit { .. } => api_error(
            StatusCode::BAD_REQUEST,
            "INVALID_PARAMETER",
            message,
            rid,
            None,
        ),
        _ => code_error("INTERNAL", message, rid),
    }
}

pub async fn robustness(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(run_id): Path<String>,
    JsonBody(body): JsonBody<crate::http::dto::RobustnessSuiteBody>,
) -> Response {
    use result_model::robustness::{
        HoldoutBarrier, LineageRegistry, PeriodSplit, PlannedChild, SuiteRequest, plan_suite,
    };

    let rid = request_id(&headers);
    if let Err(r) = crate::http::session::require_csrf(&headers, &session.0) {
        return r;
    }
    let actor = session.actor();
    let id = match parse_uuid(&rid, "run id", &run_id) {
        Ok(i) => i,
        Err(r) => return r,
    };
    let body_hash = crate::http::idempotency::body_hash(&serde_json::json!({
        "axes": &body.axes,
    }));
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
        let parent = match parent_provenance(&run) {
            Ok(p) => p,
            Err(detail) => {
                return code_error(
                    "INTERNAL",
                    format!("parent provenance invalid: {detail}"),
                    &rid,
                );
            }
        };
        let holdout = match &body.holdout {
            Some(h) => {
                let split = PeriodSplit {
                    train_end: h.train_end.clone(),
                    validation_end: h.validation_end.clone(),
                };
                if let Err(e) = split.validate() {
                    return robustness_error_response(&rid, e);
                }
                Some(HoldoutBarrier::new(&split))
            }
            None => None,
        };
        let requested_axes = body.axes.clone().unwrap_or_else(default_axes);
        let children: Vec<PlannedChild> = requested_axes
            .iter()
            .map(|axis| PlannedChild {
                axes: vec![axis.clone()],
                provenance: parent.clone(),
            })
            .collect();
        let suite_request = SuiteRequest {
            parent_run_id: id,
            parent: parent.clone(),
            children,
        };
        let mut registry = LineageRegistry::new();
        let plan = match plan_suite(&mut registry, holdout.as_ref(), suite_request) {
            Ok(p) => p,
            Err(e) => return robustness_error_response(&rid, e),
        };

        let batch_items: Vec<crate::repos::robustness::NewRobustnessJob> = plan
            .items
            .iter()
            .map(|item| crate::repos::robustness::NewRobustnessJob {
                run_id: item.lineage.run_id,
                axis_code: item.lineage.changed_axis.code().to_string(),
                axis_json: serde_json::to_value(&item.lineage.changed_axis)
                    .unwrap_or(serde_json::Value::Null),
                payload: serde_json::json!({
                    "parent_run_id": id,
                    "run_id": item.lineage.run_id,
                    "axis": item.lineage.changed_axis,
                }),
                idempotency_key: item.idempotency_key.clone(),
            })
            .collect();
        let submitted = match state
            .robustness_suites()
            .submit_suite(
                &actor,
                id,
                &batch_items,
                state.cfg.max_jobs_per_owner,
            )
            .await
        {
            Ok(submitted) => submitted,
            Err(crate::repos::robustness::SubmitRobustnessError::CapacityExceeded) => {
                return api_error(
                    StatusCode::TOO_MANY_REQUESTS,
                    "ROBUSTNESS_CAPACITY_EXCEEDED",
                    format!(
                        "robustness fan-out would exceed per-owner queued job capacity ({})",
                        state.cfg.max_jobs_per_owner
                    ),
                    &rid,
                    None,
                );
            }
            Err(crate::repos::robustness::SubmitRobustnessError::IdempotencyMismatch) => {
                return code_error(
                    "INTERNAL",
                    "robustness lineage key resolved to incompatible input",
                    &rid,
                );
            }
            Err(crate::repos::robustness::SubmitRobustnessError::Tenancy(e)) => {
                return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND");
            }
        };

        audit(
            &state,
            &session,
            &headers,
            "backtest.robustness",
            "backtest_run",
            &run.id.to_string(),
            None,
            Some(serde_json::json!({ "suite_id": submitted.suite.id, "children": submitted.children.len() })),
            None,
        )
        .await;

        let dto_children: Vec<RobustnessChildDto> = plan
            .items
            .iter()
            .zip(submitted.children.iter())
            .map(|(item, child)| RobustnessChildDto {
                run_id: item.lineage.run_id.to_string(),
                job_id: child.job_id.to_string(),
                axis: item.lineage.changed_axis.code().to_string(),
                status: child.job_status.clone(),
            })
            .collect();
        (
            StatusCode::OK,
            Json(RobustnessDto {
                run_id: run.id.to_string(),
                suite_id: submitted.suite.id.to_string(),
                children: dto_children,
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
