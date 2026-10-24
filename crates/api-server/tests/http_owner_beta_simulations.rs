//! Real-router PostgreSQL and ownership tests for the dedicated simulation API.
//! All rows are synthetic fixtures; no approved artifact bytes or calculations
//! are loaded by this test target.

mod common;

use axum::http::StatusCode;
use common::{Harness, SimulationFixture, status};
use serde_json::{Value, json};
use sqlx::{Connection, PgConnection, Postgres, Transaction};
use uuid::Uuid;

const BASE: &str = "/api/v1/backtests/owner-beta/price-only";
const START: &str = "2026-08-27";
const END: &str = "2026-08-28";

fn run_path(run_id: Uuid) -> String {
    format!("{BASE}/runs/{run_id}")
}

fn pair_path(run_id: Uuid) -> String {
    format!("{BASE}/runs/{run_id}/pair")
}

fn cancel_path(run_id: Uuid) -> String {
    format!("{BASE}/runs/{run_id}/cancel")
}

fn enqueue_body(config_id: Uuid) -> Value {
    json!({
        "strategy_config_id": config_id,
        "start_date": START,
        "end_date": END,
    })
}

fn assert_private(response: &axum::response::Response) {
    assert_private_for(response, Harness::RID);
}

fn assert_private_for(response: &axum::response::Response, expected_rid: &str) {
    assert_eq!(
        response
            .headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok()),
        Some("no-store")
    );
    assert_eq!(
        response
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok()),
        Some(expected_rid)
    );
}

async fn setup() -> Option<(Harness, Uuid)> {
    let Some(mut h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return None;
    };
    h.seed_simulation_backtest_rights().await;
    let config_id = h.seed_simulation_strategy_config(&h.owner).await;
    h.restart_api_with_owner_beta_access().await;
    Some((h, config_id))
}

async fn seed_pending(h: &Harness, actor: &common::UserCtx, config: Uuid) -> SimulationFixture {
    h.seed_simulation_pending(actor, config, START, END).await
}

async fn visible_in_transaction(tx: &mut Transaction<'_, Postgres>, run_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*)::bigint FROM public.owner_beta_simulation_runs WHERE id=$1")
        .bind(run_id)
        .fetch_one(&mut **tx)
        .await
        .expect("transaction-scoped simulation visibility query")
}

async fn visible_on_app_connection(conn: &mut PgConnection, run_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*)::bigint FROM public.owner_beta_simulation_runs WHERE id=$1")
        .bind(run_id)
        .fetch_one(conn)
        .await
        .expect("held app-connection simulation visibility query")
}

#[tokio::test]
async fn simulation_owner_admission_and_closed_body() {
    let Some(mut h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let config_id = h.seed_simulation_strategy_config(&h.owner).await;
    let path = format!("{BASE}/runs");
    let random_run = Uuid::new_v4();
    let gated = [
        ("POST", path.clone(), Some(enqueue_body(config_id))),
        ("GET", format!("{BASE}/runs"), None),
        ("GET", run_path(random_run), None),
        ("POST", cancel_path(random_run), Some(json!({}))),
        ("GET", pair_path(random_run), None),
    ];
    for (method, route, body) in &gated {
        let anonymous = h
            .send(
                method,
                route,
                None,
                false,
                Some(Harness::RID),
                Some("anonymous-gate-key"),
                body.clone(),
            )
            .await;
        assert_private(&anonymous);
        assert_eq!(
            status(&anonymous),
            StatusCode::UNAUTHORIZED,
            "{method} {route}"
        );
        assert_eq!(
            Harness::error_code(&Harness::body_json(anonymous).await),
            "SESSION_UNKNOWN"
        );
    }

    // The local handler repeats the exact OwnerOnly/Owner check, even before
    // the central beta middleware is enabled.
    let disabled = h
        .send(
            "POST",
            &path,
            Some(&h.owner),
            false,
            Some(Harness::RID),
            Some("disabled-owner-key"),
            Some(enqueue_body(config_id)),
        )
        .await;
    assert_eq!(status(&disabled), StatusCode::FORBIDDEN);
    assert_private(&disabled);

    h.restart_api_with_owner_beta_access().await;
    for (method, route, body) in &gated {
        let member = h
            .send(
                method,
                route,
                Some(&h.member),
                true,
                Some(Harness::RID),
                Some("member-gate-key"),
                body.clone(),
            )
            .await;
        assert_private(&member);
        assert_eq!(status(&member), StatusCode::FORBIDDEN, "{method} {route}");
        assert_eq!(
            Harness::error_code(&Harness::body_json(member).await),
            "FORBIDDEN"
        );
    }
    h.restart_api_with_owner_beta_disabled().await;
    for (method, route, body) in gated.iter().skip(1) {
        let disabled = h
            .send(
                method,
                route,
                Some(&h.owner),
                false,
                Some(Harness::RID),
                Some("disabled-gate-key"),
                body.clone(),
            )
            .await;
        assert_private(&disabled);
        assert_eq!(status(&disabled), StatusCode::FORBIDDEN, "{method} {route}");
        assert_eq!(
            Harness::error_code(&Harness::body_json(disabled).await),
            "FORBIDDEN"
        );
    }
    h.restart_api_with_owner_beta_access().await;
    let mut extra = enqueue_body(config_id);
    extra["owner_user_id"] = json!(h.owner.user_id);
    let closed = h
        .send(
            "POST",
            &path,
            Some(&h.owner),
            true,
            Some(Harness::RID),
            Some("closed-body-key"),
            Some(extra),
        )
        .await;
    assert_private(&closed);
    assert_eq!(status(&closed), StatusCode::BAD_REQUEST);
    assert_eq!(
        Harness::error_code(&Harness::body_json(closed).await),
        "INVALID_PARAMETER"
    );

    let duplicate_body = format!(
        r#"{{"strategy_config_id":"{config_id}","strategy_config_id":"{config_id}","start_date":"{START}","end_date":"{END}"}}"#
    );
    let duplicate = h
        .send_raw(
            "POST",
            &path,
            Some(&h.owner),
            true,
            Some(Harness::RID),
            Some("duplicate-field-key"),
            Some(&duplicate_body),
        )
        .await;
    assert_private(&duplicate);
    assert_eq!(status(&duplicate), StatusCode::BAD_REQUEST);
    assert_eq!(
        Harness::error_code(&Harness::body_json(duplicate).await),
        "INVALID_PARAMETER"
    );

    h.restart_api_with_owner_beta_price_input().await;
    let no_csrf = h
        .send(
            "POST",
            &path,
            Some(&h.owner),
            false,
            Some(Harness::RID),
            Some("csrf-required-key"),
            Some(enqueue_body(config_id)),
        )
        .await;
    assert_private(&no_csrf);
    assert_eq!(status(&no_csrf), StatusCode::FORBIDDEN);
    assert_eq!(
        Harness::error_code(&Harness::body_json(no_csrf).await),
        "CSRF_DENIED"
    );

    let no_key = h
        .send(
            "POST",
            &path,
            Some(&h.owner),
            true,
            Some(Harness::RID),
            None,
            Some(enqueue_body(config_id)),
        )
        .await;
    assert_private(&no_key);
    assert_eq!(status(&no_key), StatusCode::BAD_REQUEST);
    assert_eq!(
        Harness::error_code(&Harness::body_json(no_key).await),
        "IDEMPOTENCY_KEY_REQUIRED"
    );

    let oversized_key_value = "k".repeat(201);
    let oversized_key = h
        .send(
            "POST",
            &path,
            Some(&h.owner),
            true,
            Some(Harness::RID),
            Some(&oversized_key_value),
            Some(enqueue_body(config_id)),
        )
        .await;
    assert_private(&oversized_key);
    assert_eq!(status(&oversized_key), StatusCode::BAD_REQUEST);
    assert_eq!(
        Harness::error_code(&Harness::body_json(oversized_key).await),
        "INVALID_PARAMETER"
    );

    let mut null_date = enqueue_body(config_id);
    null_date["start_date"] = Value::Null;
    let invalid_date = h
        .send(
            "POST",
            &path,
            Some(&h.owner),
            true,
            Some(Harness::RID),
            Some("null-date-key"),
            Some(null_date),
        )
        .await;
    assert_private(&invalid_date);
    assert_eq!(status(&invalid_date), StatusCode::BAD_REQUEST);
    assert_eq!(
        Harness::error_code(&Harness::body_json(invalid_date).await),
        "INVALID_DATE"
    );

    // Production-mode admission reaches only the sealed approval function.
    // This scratch harness contains no approved artifact, so it fails closed.
    let unavailable = h
        .send(
            "POST",
            &path,
            Some(&h.owner),
            true,
            Some(Harness::RID),
            Some("sealed-admission-key"),
            Some(enqueue_body(config_id)),
        )
        .await;
    assert_private(&unavailable);
    assert_eq!(status(&unavailable), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        Harness::error_code(&Harness::body_json(unavailable).await),
        "OWNER_BETA_PRICE_INPUT_UNAVAILABLE"
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.jobs WHERE owner_user_id=$1 AND job_type='owner_beta_price_only_simulation'",
    )
    .bind(h.owner.user_id)
    .fetch_one(&h.owner_pool)
    .await
    .expect("owner-scoped queue fixture count");
    assert_eq!(count, 0, "failed sealed admission must not enqueue");
    h.teardown().await;
}

#[tokio::test]
async fn simulation_http_cross_owner_list_get_cancel_pair() {
    let Some((mut h, config_a)) = setup().await else {
        return;
    };
    let owner_b = h
        .seed_user(
            auth::entitlement::Role::Owner,
            "simulation-owner-b@lagrange.test",
            "simulation-owner-b-iss",
            "simulation-owner-b-sub",
        )
        .await;
    let config_b = h.seed_simulation_strategy_config(&owner_b).await;
    let fixture_a = seed_pending(&h, &h.owner, config_a).await;
    let fixture_b = seed_pending(&h, &owner_b, config_b).await;

    let mut conn = h.app_pool.acquire().await.expect("app connection acquired");
    let (app_login, runs_update, pairs_update): (String, bool, bool) = sqlx::query_as(
        "SELECT current_user::text,
                has_table_privilege(current_user, 'public.owner_beta_simulation_runs', 'UPDATE'),
                has_table_privilege(current_user, 'public.owner_beta_simulation_result_pairs', 'UPDATE')",
    )
    .fetch_one(&mut *conn)
    .await
    .expect("production app-role privileges are queryable");
    assert_eq!(app_login, "app");
    assert!(!runs_update, "app must not update simulation runs");
    assert!(!pairs_update, "app must not update simulation result pairs");

    let mut owner_a_commit = conn.begin().await.expect("owner A transaction begins");
    api_server::actor_tx::set_actor_guc(&mut owner_a_commit, &h.owner.actor())
        .await
        .expect("owner A actor GUC is set on the held connection");
    assert_eq!(
        visible_in_transaction(&mut owner_a_commit, fixture_a.run_id).await,
        1
    );
    assert_eq!(
        visible_in_transaction(&mut owner_a_commit, fixture_b.run_id).await,
        0
    );
    owner_a_commit
        .commit()
        .await
        .expect("owner A transaction commits");
    assert_eq!(
        visible_on_app_connection(&mut conn, fixture_a.run_id).await,
        0
    );
    assert_eq!(
        visible_on_app_connection(&mut conn, fixture_b.run_id).await,
        0
    );

    let mut owner_a_rollback = conn
        .begin()
        .await
        .expect("owner A rollback transaction begins");
    api_server::actor_tx::set_actor_guc(&mut owner_a_rollback, &h.owner.actor())
        .await
        .expect("owner A actor GUC is reset for rollback check");
    assert_eq!(
        visible_in_transaction(&mut owner_a_rollback, fixture_a.run_id).await,
        1
    );
    owner_a_rollback
        .rollback()
        .await
        .expect("owner A transaction rolls back");
    assert_eq!(
        visible_on_app_connection(&mut conn, fixture_a.run_id).await,
        0
    );
    assert_eq!(
        visible_on_app_connection(&mut conn, fixture_b.run_id).await,
        0
    );

    let mut owner_b_tx = conn.begin().await.expect("owner B transaction begins");
    api_server::actor_tx::set_actor_guc(&mut owner_b_tx, &owner_b.actor())
        .await
        .expect("owner B actor GUC replaces owner A on the held connection");
    assert_eq!(
        visible_in_transaction(&mut owner_b_tx, fixture_a.run_id).await,
        0
    );
    assert_eq!(
        visible_in_transaction(&mut owner_b_tx, fixture_b.run_id).await,
        1
    );
    owner_b_tx
        .commit()
        .await
        .expect("owner B transaction commits");
    assert_eq!(
        visible_on_app_connection(&mut conn, fixture_a.run_id).await,
        0
    );
    assert_eq!(
        visible_on_app_connection(&mut conn, fixture_b.run_id).await,
        0
    );

    assert_eq!(
        h.simulation_run_visible_to_actor(&h.owner, fixture_a.run_id)
            .await,
        1
    );
    assert_eq!(
        h.simulation_run_visible_to_actor(&owner_b, fixture_a.run_id)
            .await,
        0
    );
    assert_eq!(
        h.simulation_run_visible_without_actor(fixture_a.run_id)
            .await,
        0
    );

    let list_a = h.get(&format!("{BASE}/runs"), Some(&h.owner)).await;
    assert_private(&list_a);
    assert_eq!(status(&list_a), StatusCode::OK);
    let list_a = Harness::body_json(list_a).await;
    assert_eq!(list_a["items"].as_array().unwrap().len(), 1);
    assert_eq!(list_a["items"][0]["id"], fixture_a.run_id.to_string());
    assert_eq!(list_a["items"][0]["status"], "PENDING");
    for forbidden in [
        "owner_user_id",
        "result_content_sha256",
        "metrics",
        "engine",
    ] {
        assert!(list_a["items"][0].get(forbidden).is_none());
    }

    let list_b = h.get(&format!("{BASE}/runs"), Some(&owner_b)).await;
    assert_private(&list_b);
    assert_eq!(status(&list_b), StatusCode::OK);
    let list_b = Harness::body_json(list_b).await;
    assert_eq!(list_b["items"].as_array().unwrap().len(), 1);
    assert_eq!(list_b["items"][0]["id"], fixture_b.run_id.to_string());
    assert_eq!(h.simulation_job_count(&h.owner).await, 1);
    assert_eq!(h.simulation_job_count(&owner_b).await, 1);

    let foreign_get = h.get(&run_path(fixture_a.run_id), Some(&owner_b)).await;
    assert_private(&foreign_get);
    let foreign_get_status = status(&foreign_get);
    let foreign_get_body = Harness::body_json(foreign_get).await;
    let missing_get = h.get(&run_path(Uuid::new_v4()), Some(&owner_b)).await;
    assert_private(&missing_get);
    assert_eq!(foreign_get_status, StatusCode::NOT_FOUND);
    assert_eq!(status(&missing_get), foreign_get_status);
    assert_eq!(Harness::error_code(&foreign_get_body), "RESOURCE_NOT_FOUND");
    assert_eq!(
        Harness::error_code(&Harness::body_json(missing_get).await),
        "RESOURCE_NOT_FOUND"
    );

    let foreign_pair = h.get(&pair_path(fixture_a.run_id), Some(&owner_b)).await;
    assert_private(&foreign_pair);
    assert_eq!(status(&foreign_pair), StatusCode::NOT_FOUND);
    assert_eq!(
        Harness::error_code(&Harness::body_json(foreign_pair).await),
        "RESOURCE_NOT_FOUND"
    );
    let foreign_cancel = h
        .send(
            "POST",
            &cancel_path(fixture_a.run_id),
            Some(&owner_b),
            true,
            Some(Harness::RID),
            Some("foreign-cancel-key"),
            Some(json!({})),
        )
        .await;
    assert_private(&foreign_cancel);
    assert_eq!(status(&foreign_cancel), StatusCode::NOT_FOUND);

    let pending_pair = h.get(&pair_path(fixture_a.run_id), Some(&h.owner)).await;
    assert_private(&pending_pair);
    assert_eq!(status(&pending_pair), StatusCode::CONFLICT);
    let pending_pair_body = Harness::body_json(pending_pair).await;
    assert_eq!(
        Harness::error_code(&pending_pair_body),
        "SIMULATION_RESULT_NOT_READY"
    );
    assert!(
        pending_pair_body
            .get("illustrative_10bp_per_side")
            .is_none()
    );
    assert!(pending_pair_body.get("zero_cost_reference").is_none());

    let cancel = h
        .send(
            "POST",
            &cancel_path(fixture_a.run_id),
            Some(&h.owner),
            true,
            Some(Harness::RID),
            Some("owner-cancel-after-revoke"),
            Some(json!({})),
        )
        .await;
    assert_private(&cancel);
    assert_eq!(status(&cancel), StatusCode::OK);
    let cancel_body = Harness::body_json(cancel).await;
    assert_eq!(cancel_body["run_id"], fixture_a.run_id.to_string());
    assert_eq!(cancel_body["job_id"], fixture_a.job_id.to_string());
    assert_eq!(cancel_body["status"], "CANCEL_REQUESTED");

    let closed_cancel = h
        .send(
            "POST",
            &cancel_path(fixture_b.run_id),
            Some(&owner_b),
            true,
            Some(Harness::RID),
            Some("closed-cancel-body"),
            Some(json!({ "force": true })),
        )
        .await;
    assert_private(&closed_cancel);
    assert_eq!(status(&closed_cancel), StatusCode::BAD_REQUEST);
    assert_eq!(
        Harness::error_code(&Harness::body_json(closed_cancel).await),
        "INVALID_PARAMETER"
    );

    let successful_fixture_a = seed_pending(&h, &h.owner, config_a).await;
    h.seed_simulation_success_pair(&successful_fixture_a).await;
    let successful_pair = h
        .get(&pair_path(successful_fixture_a.run_id), Some(&h.owner))
        .await;
    assert_private(&successful_pair);
    assert_eq!(status(&successful_pair), StatusCode::OK);
    let successful_pair = Harness::body_json(successful_pair).await;
    assert_eq!(successful_pair["status"], "SUCCEEDED");
    assert!(successful_pair["illustrative_10bp_per_side"].is_object());
    assert!(successful_pair["zero_cost_reference"].is_object());

    let list_with_success = h.get(&format!("{BASE}/runs"), Some(&h.owner)).await;
    assert_private(&list_with_success);
    assert_eq!(status(&list_with_success), StatusCode::OK);
    let list_with_success = Harness::body_json(list_with_success).await;
    assert_eq!(list_with_success["items"].as_array().unwrap().len(), 2);
    assert!(
        list_with_success["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| {
                item["id"] == successful_fixture_a.run_id.to_string()
                    && item["status"] == "SUCCEEDED"
            })
    );
    h.teardown().await;
}

#[tokio::test]
async fn simulation_same_hash_independent_ownership() {
    let Some((h, config_a)) = setup().await else {
        return;
    };
    let owner_b = h
        .seed_user(
            auth::entitlement::Role::Owner,
            "simulation-same-hash-b@lagrange.test",
            "simulation-same-hash-b-iss",
            "simulation-same-hash-b-sub",
        )
        .await;
    let config_b = h.seed_simulation_strategy_config(&owner_b).await;
    let a = seed_pending(&h, &h.owner, config_a).await;
    let b = seed_pending(&h, &owner_b, config_b).await;
    let a_hashes = h.seed_simulation_success_pair(&a).await;
    let b_hashes = h.seed_simulation_success_pair(&b).await;
    assert_ne!(a.run_id, b.run_id);
    assert_eq!(
        a_hashes, b_hashes,
        "identical calculations may share hashes"
    );
    let a_run = h.get(&run_path(a.run_id), Some(&h.owner)).await;
    let b_run = h.get(&run_path(b.run_id), Some(&owner_b)).await;
    assert_private(&a_run);
    assert_private(&b_run);
    assert_eq!(status(&a_run), StatusCode::OK);
    assert_eq!(status(&b_run), StatusCode::OK);
    let a_json = Harness::body_json(a_run).await;
    let b_json = Harness::body_json(b_run).await;
    assert_eq!(
        a_json["contract_document_sha256"],
        b_json["contract_document_sha256"]
    );
    assert_eq!(a_json["status"], "SUCCEEDED");
    assert_eq!(b_json["status"], "SUCCEEDED");
    let a_pair = h.get(&pair_path(a.run_id), Some(&h.owner)).await;
    let b_pair = h.get(&pair_path(b.run_id), Some(&owner_b)).await;
    assert_private(&a_pair);
    assert_private(&b_pair);
    assert_eq!(status(&a_pair), StatusCode::OK);
    assert_eq!(status(&b_pair), StatusCode::OK);
    let a_pair = Harness::body_json(a_pair).await;
    let b_pair = Harness::body_json(b_pair).await;
    assert_eq!(
        a_pair["illustrative_10bp_per_side"]["result_content_sha256"],
        b_pair["illustrative_10bp_per_side"]["result_content_sha256"]
    );
    assert_eq!(
        a_pair["zero_cost_reference"]["result_content_sha256"],
        b_pair["zero_cost_reference"]["result_content_sha256"]
    );
    let foreign = h.get(&run_path(a.run_id), Some(&owner_b)).await;
    assert_private(&foreign);
    assert_eq!(status(&foreign), StatusCode::NOT_FOUND);
    h.teardown().await;
}

#[tokio::test]
async fn simulation_pair_only_after_atomic_success() {
    let Some((h, config_id)) = setup().await else {
        return;
    };
    let fixture = seed_pending(&h, &h.owner, config_id).await;
    let before = h.get(&pair_path(fixture.run_id), Some(&h.owner)).await;
    assert_private(&before);
    assert_eq!(status(&before), StatusCode::CONFLICT);
    let before_body = Harness::body_json(before).await;
    assert_eq!(
        Harness::error_code(&before_body),
        "SIMULATION_RESULT_NOT_READY"
    );
    assert!(before_body.get("illustrative_10bp_per_side").is_none());
    assert!(before_body.get("zero_cost_reference").is_none());

    let (illustrative_hash, zero_cost_hash) = h.seed_simulation_success_pair(&fixture).await;
    let complete = h.get(&pair_path(fixture.run_id), Some(&h.owner)).await;
    assert_private(&complete);
    assert_eq!(status(&complete), StatusCode::OK);
    let complete = Harness::body_json(complete).await;
    assert_eq!(complete["status"], "SUCCEEDED");
    assert_eq!(
        complete["illustrative_10bp_per_side"]["result_content_sha256"],
        illustrative_hash
    );
    assert_eq!(
        complete["zero_cost_reference"]["result_content_sha256"],
        zero_cost_hash
    );
    for profile in ["illustrative_10bp_per_side", "zero_cost_reference"] {
        let result = complete[profile]["result"]
            .as_object()
            .expect("full canonical result");
        assert_eq!(
            result
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>(),
            std::collections::BTreeSet::from([
                "contract".to_owned(),
                "input".to_owned(),
                "engine".to_owned(),
                "strategy".to_owned(),
                "schedule".to_owned(),
                "conventions".to_owned(),
                "signals".to_owned(),
                "strategy_ledger".to_owned(),
                "benchmark_ledger".to_owned(),
            ])
        );
    }

    let corrupt_fixture = seed_pending(&h, &h.owner, config_id).await;
    h.seed_simulation_corrupt_success_pair(&corrupt_fixture)
        .await;
    for path in [
        run_path(corrupt_fixture.run_id),
        pair_path(corrupt_fixture.run_id),
        format!("{BASE}/runs"),
    ] {
        let response = h.get(&path, Some(&h.owner)).await;
        assert_private(&response);
        assert_eq!(
            status(&response),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{path}"
        );
        let body = Harness::body_json(response).await;
        assert_eq!(Harness::error_code(&body), "RESULT_INTEGRITY_FAILED");
        for profile in ["illustrative_10bp_per_side", "zero_cost_reference"] {
            assert!(body.get(profile).is_none(), "no partial profile on {path}");
        }
    }
    h.teardown().await;
}

#[tokio::test]
async fn simulation_cancel_publish_and_lease_races() {
    let Some((h, config_id)) = setup().await else {
        return;
    };
    let fixture = seed_pending(&h, &h.owner, config_id).await;
    let path = cancel_path(fixture.run_id);
    let first = h.send(
        "POST",
        &path,
        Some(&h.owner),
        true,
        Some("cancel-race-a"),
        Some("same-cancel-intent"),
        Some(json!({})),
    );
    let second = h.send(
        "POST",
        &path,
        Some(&h.owner),
        true,
        Some("cancel-race-b"),
        Some("same-cancel-intent"),
        Some(json!({})),
    );
    let (first, second) = tokio::join!(first, second);
    assert_private_for(&first, "cancel-race-a");
    assert_private_for(&second, "cancel-race-b");
    assert_eq!(status(&first), StatusCode::OK);
    assert_eq!(status(&second), StatusCode::OK);
    let first_body = Harness::body_json(first).await;
    let second_body = Harness::body_json(second).await;
    assert_eq!(first_body["status"], "CANCEL_REQUESTED");
    assert_eq!(second_body["status"], "CANCEL_REQUESTED");
    let queue = h
        .state()
        .queue_for(&h.owner.actor())
        .await
        .expect("actor queue");
    let job = queue
        .get_by_id(fixture.job_id)
        .await
        .expect("canceled job visible");
    assert_eq!(job.status.as_str(), "CANCELED");
    let current = h.get(&run_path(fixture.run_id), Some(&h.owner)).await;
    assert_private(&current);
    assert_eq!(status(&current), StatusCode::OK);
    assert_eq!(Harness::body_json(current).await["status"], "CANCELED");

    let other_fixture = seed_pending(&h, &h.owner, config_id).await;
    let key_reused_for_other_run = h
        .send(
            "POST",
            &cancel_path(other_fixture.run_id),
            Some(&h.owner),
            true,
            Some("cancel-other-run-key"),
            Some("same-cancel-intent"),
            Some(json!({})),
        )
        .await;
    assert_private_for(&key_reused_for_other_run, "cancel-other-run-key");
    assert_eq!(status(&key_reused_for_other_run), StatusCode::CONFLICT);
    assert_eq!(
        Harness::error_code(&Harness::body_json(key_reused_for_other_run).await),
        "IDEMPOTENCY_KEY_MISMATCH"
    );
    let queue = h
        .state()
        .queue_for(&h.owner.actor())
        .await
        .expect("actor queue");
    let still_queued = queue
        .get_by_id(other_fixture.job_id)
        .await
        .expect("unrelated job remains visible");
    assert_eq!(still_queued.status.as_str(), "QUEUED");
    h.teardown().await;
}

#[tokio::test]
async fn simulation_entitlement_revoke_and_safe_errors() {
    let Some((h, config_id)) = setup().await else {
        return;
    };
    let fixture = seed_pending(&h, &h.owner, config_id).await;
    let before = h.get(&run_path(fixture.run_id), Some(&h.owner)).await;
    assert_eq!(status(&before), StatusCode::OK);
    h.revoke_simulation_backtest_rights().await;

    for path in [
        format!("{BASE}/runs"),
        run_path(fixture.run_id),
        pair_path(fixture.run_id),
    ] {
        let response = h.get(&path, Some(&h.owner)).await;
        assert_private(&response);
        assert_eq!(status(&response), StatusCode::FORBIDDEN, "{path}");
        let body = Harness::body_json(response).await;
        assert_eq!(Harness::error_code(&body), "DATA_ENTITLEMENT_REQUIRED");
        assert_eq!(body["error"]["request_id"], Harness::RID);
        assert!(body["error"]["message"].as_str().unwrap().len() < 160);
    }

    // Revocation blocks every payload read but leaves own cancellation open.
    let cancel = h
        .send(
            "POST",
            &cancel_path(fixture.run_id),
            Some(&h.owner),
            true,
            Some(Harness::RID),
            Some("revoked-owner-cancel"),
            Some(json!({})),
        )
        .await;
    assert_private(&cancel);
    assert_eq!(status(&cancel), StatusCode::OK);
    h.teardown().await;
}

#[tokio::test]
async fn simulation_generic_surfaces_stay_separate() {
    let Some((h, config_id)) = setup().await else {
        return;
    };
    let fixture = seed_pending(&h, &h.owner, config_id).await;
    let generic = h.get("/api/v1/backtests", Some(&h.owner)).await;
    assert_eq!(status(&generic), StatusCode::OK);
    let generic_body = Harness::body_json(generic).await;
    assert!(generic_body["items"].as_array().unwrap().is_empty());
    assert_eq!(generic_body.pointer("/items/0/id"), None);
    for path in [
        format!("/api/v1/backtests/{}", fixture.run_id),
        format!("/api/v1/backtests/{}/metrics", fixture.run_id),
        format!("/api/v1/backtests/{}/equity", fixture.run_id),
        format!("/api/v1/backtests/{}/trades", fixture.run_id),
    ] {
        let response = h.get(&path, Some(&h.owner)).await;
        assert_private(&response);
        assert_eq!(status(&response), StatusCode::NOT_FOUND, "{path}");
    }
    let dedicated = h.get(&run_path(fixture.run_id), Some(&h.owner)).await;
    assert_eq!(status(&dedicated), StatusCode::OK);
    assert_eq!(
        Harness::body_json(dedicated).await["id"],
        fixture.run_id.to_string()
    );
    h.teardown().await;
}
