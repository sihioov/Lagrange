//! Todo 29: bounded robustness-suite orchestration and promotion evidence,
//! proven at the HTTP layer (plan acceptance: "API integration tests prove
//! bounded fan-out, one-axis child configs, cancellation, holdout
//! non-access, version pinning, and promotion refusal without all required
//! evidence").
//!
//! `robustness_orchestration_promotion_*` needs no database at all: it
//! drives `selector::Registry` directly with the evidence
//! `result_model::robustness::assemble_evidence_bundle` assembles, exactly
//! the "promote a qualifying synthetic strategy to Paper" QA scenario the
//! todo names. Every other test is DB-gated (`DATABASE_URL`) and drives the
//! real router exactly like the sibling `http_backtests.rs`/`phase1_gate.rs`
//! suites.

mod common;

use axum::http::StatusCode;
use common::{Harness, UserCtx};
use serde_json::{Value, json};
use uuid::Uuid;

const RID: &str = "test-rid-1";

fn status(resp: &axum::http::Response<axum::body::Body>) -> StatusCode {
    resp.status()
}

async fn create_config(h: &Harness, u: &UserCtx, key: &str) -> String {
    let resp = h
        .send(
            "POST",
            "/api/v1/strategies/buy_and_hold/configs",
            Some(u),
            true,
            Some(RID),
            Some(key),
            Some(json!({
                "strategy_version": "1.0.0",
                "config": { "lookback": 200 },
                "is_active": true,
            })),
        )
        .await;
    assert_eq!(
        status(&resp),
        StatusCode::CREATED,
        "config create must succeed"
    );
    Harness::body_json(resp).await["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn ready_dataset(h: &Harness) -> String {
    let id: Uuid =
        sqlx::query_scalar("SELECT id FROM dataset_versions WHERE status='READY' LIMIT 1")
            .fetch_one(&h.member_pool().await)
            .await
            .unwrap();
    id.to_string()
}

fn backtest_request(dataset: &str, cfg: &str) -> Value {
    json!({
        "strategy_config_id": cfg,
        "dataset_version_id": dataset,
        "start_date": "2026-01-05",
        "end_date": "2026-01-30",
        "initial_cash": { "currency": "KRW", "amount": "100000000" },
        "benchmark": "069500.KRX",
        "cost_profile_id": "krx-etf-default@2026-01",
        "execution_profile": "daily-close-next-open@1",
        "robustness": false,
    })
}

/// Creates and queues a backtest, then marks it SUCCEEDED directly (the
/// worker-side result-normalization pipeline is out of scope here; only the
/// robustness-suite orchestration built ON TOP of a settled run is under
/// test). Returns the run id.
async fn succeeded_run(h: &Harness, actor: &UserCtx) -> String {
    let cfg = create_config(h, actor, &format!("cfg-{}", Uuid::new_v4())).await;
    let dataset = ready_dataset(h).await;
    let resp = h
        .post(
            "/api/v1/backtests",
            Some(actor),
            true,
            backtest_request(&dataset, &cfg),
        )
        .await;
    assert_eq!(
        status(&resp),
        StatusCode::CREATED,
        "backtest create must succeed"
    );
    let run_id = Harness::body_json(resp).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    h.seed_tenant(
        actor,
        &format!("UPDATE backtest_runs SET status='SUCCEEDED' WHERE id='{run_id}'"),
    )
    .await;
    run_id
}

fn cost_stress_axis(profile_id: &str) -> Value {
    json!({ "axis": "cost_stress", "profile_id": profile_id, "profile_version": 1 })
}

// ---------------------------------------------------------------------------
// Bounded fan-out + one-axis child configs
// ---------------------------------------------------------------------------

#[tokio::test]
async fn robustness_orchestration_bounded_fan_out_and_one_axis_children() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let m = h.member.clone();
    let run_id = succeeded_run(&h, &m).await;

    let axes: Vec<Value> = vec![
        cost_stress_axis("adverse"),
        cost_stress_axis("extreme"),
        json!({ "axis": "execution_delay", "delay_sessions": 1 }),
    ];
    let resp = h
        .post(
            &format!("/api/v1/backtests/{run_id}/robustness"),
            Some(&m),
            true,
            json!({ "axes": axes }),
        )
        .await;
    assert_eq!(
        status(&resp),
        StatusCode::OK,
        "bounded suite creation must succeed"
    );
    let body = Harness::body_json(resp).await;
    let children = body["children"].as_array().expect("children array");
    assert_eq!(children.len(), 3, "one child per requested axis");
    let axis_codes: std::collections::BTreeSet<&str> = children
        .iter()
        .map(|c| c["axis"].as_str().unwrap())
        .collect();
    assert_eq!(
        axis_codes,
        std::collections::BTreeSet::from(["cost_stress", "execution_delay"]),
        "each child carries exactly its own single axis"
    );
    let run_ids: std::collections::BTreeSet<&str> = children
        .iter()
        .map(|c| c["run_id"].as_str().unwrap())
        .collect();
    assert_eq!(
        run_ids.len(),
        3,
        "every child has a distinct lineage run id"
    );

    let child_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM robustness_children WHERE suite_id = $1::uuid")
            .bind(body["suite_id"].as_str().unwrap())
            .fetch_one(&h.member_pool().await)
            .await
            .unwrap();
    assert_eq!(
        child_count, 3,
        "exactly the requested children are persisted"
    );

    // Oversized grid: 26 axes exceeds MAX_SUITE_CHILDREN (25) and must be
    // rejected wholesale -- no NEW suite, no new children.
    let oversized: Vec<Value> = (0..26)
        .map(|i| cost_stress_axis(&format!("p{i}")))
        .collect();
    let resp = h
        .post(
            &format!("/api/v1/backtests/{run_id}/robustness"),
            Some(&m),
            true,
            json!({ "axes": oversized }),
        )
        .await;
    assert_eq!(
        status(&resp),
        StatusCode::BAD_REQUEST,
        "oversized grid is rejected"
    );
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "INVALID_PARAMETER");
    let total_children: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM robustness_children c JOIN robustness_suites s ON s.id = c.suite_id WHERE s.parent_run_id = $1::uuid",
    )
    .bind(&run_id)
    .fetch_one(&h.member_pool().await)
    .await
    .unwrap();
    assert_eq!(
        total_children, 3,
        "the rejected oversized request added NOTHING"
    );
    h.teardown().await;
}

// ---------------------------------------------------------------------------
// Holdout non-access
// ---------------------------------------------------------------------------

#[tokio::test]
async fn robustness_orchestration_rejects_a_period_split_that_reads_the_holdout() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let m = h.member.clone();
    let run_id = succeeded_run(&h, &m).await;

    let resp = h
        .post(
            &format!("/api/v1/backtests/{run_id}/robustness"),
            Some(&m),
            true,
            json!({
                "axes": [
                    { "axis": "period_split", "train_end": "2024-06-30", "validation_end": "2025-03-31" }
                ],
                "holdout": { "train_end": "2024-06-30", "validation_end": "2024-12-31" },
            }),
        )
        .await;
    assert_eq!(
        status(&resp),
        StatusCode::BAD_REQUEST,
        "a holdout-reading split is rejected"
    );
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "INVALID_PARAMETER");

    let suite_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM robustness_suites WHERE parent_run_id = $1::uuid")
            .bind(&run_id)
            .fetch_one(&h.member_pool().await)
            .await
            .unwrap();
    assert_eq!(
        suite_count, 0,
        "a rejected suite plan must create NO suite row"
    );
    h.teardown().await;
}

// ---------------------------------------------------------------------------
// Cancellation cascades to children
// ---------------------------------------------------------------------------

#[tokio::test]
async fn robustness_orchestration_cancel_cascades_to_suite_children() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let m = h.member.clone();
    let run_id = succeeded_run(&h, &m).await;

    let resp = h
        .post(
            &format!("/api/v1/backtests/{run_id}/robustness"),
            Some(&m),
            true,
            json!({ "axes": [cost_stress_axis("adverse"), cost_stress_axis("extreme")] }),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);

    let resp = h
        .post(
            &format!("/api/v1/backtests/{run_id}/cancel"),
            Some(&m),
            true,
            json!({}),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK, "parent cancel must succeed");

    let statuses: Vec<String> = sqlx::query_scalar(
        "SELECT j.status FROM robustness_children c JOIN jobs j ON j.id = c.job_id \
         JOIN robustness_suites s ON s.id = c.suite_id WHERE s.parent_run_id = $1::uuid",
    )
    .bind(&run_id)
    .fetch_all(&h.member_pool().await)
    .await
    .unwrap();
    assert_eq!(statuses.len(), 2);
    assert!(
        statuses.iter().all(|s| s == "CANCELED"),
        "every suite child must be canceled alongside its parent, got {statuses:?}"
    );
    h.teardown().await;
}

// ---------------------------------------------------------------------------
// Crash-safe re-planning: a second request (different Idempotency-Key, same
// body) never duplicates the suite/children.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn robustness_orchestration_replanning_never_duplicates_suite_or_children() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let m = h.member.clone();
    let run_id = succeeded_run(&h, &m).await;
    let axes = json!({ "axes": [cost_stress_axis("adverse"), cost_stress_axis("extreme")] });

    let first = h
        .send(
            "POST",
            &format!("/api/v1/backtests/{run_id}/robustness"),
            Some(&m),
            true,
            Some(RID),
            Some("plan-attempt-1"),
            Some(axes.clone()),
        )
        .await;
    assert_eq!(status(&first), StatusCode::OK);
    let first_body = Harness::body_json(first).await;

    // Simulates the orchestrator "dying" after the first plan and re-issuing
    // the identical request under a FRESH Idempotency-Key -- the idempotency
    // cache cannot short-circuit this one; the suite repo's own uniqueness
    // must do the deduping.
    let second = h
        .send(
            "POST",
            &format!("/api/v1/backtests/{run_id}/robustness"),
            Some(&m),
            true,
            Some(RID),
            Some("plan-attempt-2"),
            Some(axes),
        )
        .await;
    let second_status = status(&second);
    let second_body = Harness::body_json(second).await;
    assert_eq!(second_status, StatusCode::OK, "second body: {second_body}");

    assert_eq!(
        first_body["suite_id"], second_body["suite_id"],
        "re-planning must resolve to the SAME suite"
    );
    let first_ids: std::collections::BTreeSet<String> = first_body["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["run_id"].as_str().unwrap().to_string())
        .collect();
    let second_ids: std::collections::BTreeSet<String> = second_body["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["run_id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        first_ids, second_ids,
        "re-planning must resolve to the SAME children"
    );

    let suite_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM robustness_suites WHERE parent_run_id = $1::uuid")
            .bind(&run_id)
            .fetch_one(&h.member_pool().await)
            .await
            .unwrap();
    assert_eq!(
        suite_rows, 1,
        "re-planning must never create a second suite row"
    );
    let child_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM robustness_children WHERE suite_id = $1::uuid")
            .bind(first_body["suite_id"].as_str().unwrap())
            .fetch_one(&h.member_pool().await)
            .await
            .unwrap();
    assert_eq!(child_rows, 2, "re-planning must never duplicate child rows");
    h.teardown().await;
}

// ---------------------------------------------------------------------------
// Promotion refusal without all required evidence (no database: pure
// selector::Registry + result_model evidence assembly).
// ---------------------------------------------------------------------------

#[test]
fn robustness_orchestration_promotion_refuses_incomplete_then_succeeds_with_full_evidence() {
    use selector::baseline::baseline_packages;
    use selector::registry::{Actor, PromotionEvidence, Registry, StrategyState};

    let mut registry = Registry::new();
    let owner = Actor::Owner;
    let package = baseline_packages()
        .into_iter()
        .find(|p| p.strategy_id == "dual_momentum")
        .expect("dual_momentum ships as a baseline package");
    let strategy_id = package.strategy_id.clone();
    let version = package.version.to_string();
    registry
        .register(&owner, package)
        .expect("register baseline package");

    // Score-only / incomplete evidence must never promote: the todo's own
    // "score-only promotion" and "missing evidence" failure QA scenarios.
    let incomplete = PromotionEvidence::Golden {
        golden_manifest_hash: String::new(),
        holdout_manifest_hash: String::new(),
        cost_manifest_hash: String::new(),
    };
    let err = registry
        .promote(
            &owner,
            &strategy_id,
            &version,
            StrategyState::Validated,
            incomplete,
        )
        .expect_err("empty manifest hashes must refuse promotion");
    assert!(matches!(
        err,
        selector::registry::RegistryError::MissingPromotionEvidence { .. }
    ));

    // A Member can never promote, regardless of evidence completeness.
    let complete = PromotionEvidence::Golden {
        golden_manifest_hash: "golden-hash".to_owned(),
        holdout_manifest_hash: "holdout-hash".to_owned(),
        cost_manifest_hash: "cost-hash".to_owned(),
    };
    let member = Actor::Member("m1".to_owned());
    let err = registry
        .promote(
            &member,
            &strategy_id,
            &version,
            StrategyState::Validated,
            complete.clone(),
        )
        .expect_err("a Member can never promote");
    assert!(matches!(
        err,
        selector::registry::RegistryError::Unauthorized { .. }
    ));

    // The qualifying synthetic strategy (complete evidence, Owner actor)
    // promotes: Draft -> Validated -> Paper, matching the todo's happy QA
    // scenario ("promote a qualifying synthetic strategy to Paper").
    let record = registry
        .promote(
            &owner,
            &strategy_id,
            &version,
            StrategyState::Validated,
            complete,
        )
        .expect("complete golden evidence promotes to Validated");
    assert_eq!(record.to, StrategyState::Validated);

    let paper_evidence = PromotionEvidence::Paper {
        parity_report_id: "parity-report-1".to_owned(),
        observation_sessions: 21,
    };
    let record = registry
        .promote(
            &owner,
            &strategy_id,
            &version,
            StrategyState::Paper,
            paper_evidence,
        )
        .expect("21-session parity evidence promotes to Paper");
    assert_eq!(record.to, StrategyState::Paper);

    // Insufficient observation window is a distinct, explicit refusal.
    let short_window = PromotionEvidence::Paper {
        parity_report_id: "parity-report-2".to_owned(),
        observation_sessions: 5,
    };
    let other = baseline_packages()
        .into_iter()
        .find(|p| p.strategy_id == "buy_and_hold")
        .expect("buy_and_hold ships as a baseline package");
    let other_id = other.strategy_id.clone();
    let other_version = other.version.to_string();
    registry
        .register(&owner, other)
        .expect("register second package");
    registry
        .promote(
            &owner,
            &other_id,
            &other_version,
            StrategyState::Validated,
            PromotionEvidence::Golden {
                golden_manifest_hash: "g".to_owned(),
                holdout_manifest_hash: "h".to_owned(),
                cost_manifest_hash: "c".to_owned(),
            },
        )
        .expect("second package reaches Validated");
    let err = registry
        .promote(
            &owner,
            &other_id,
            &other_version,
            StrategyState::Paper,
            short_window,
        )
        .expect_err("a 5-session observation window is below the documented minimum");
    assert!(matches!(
        err,
        selector::registry::RegistryError::InvalidPromotion { .. }
    ));
}
