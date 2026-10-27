//! Dual-version owner-beta simulation HTTP read and binding regressions.
//! The v2 rows and result pairs below are synthetic contract fixtures; they
//! never open approved input or claim execution readiness.

mod common;

use axum::http::StatusCode;
use common::{Harness, SimulationFixture, status};
use serde_json::json;
use sqlx::Connection;
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

async fn setup() -> Option<Harness> {
    let Some(mut harness) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set; this is not a database test pass");
        return None;
    };
    harness.seed_simulation_backtest_rights().await;
    harness.restart_api_with_owner_beta_access().await;
    Some(harness)
}

async fn fixture(
    harness: &Harness,
    actor: &common::UserCtx,
    strategy_id: &str,
) -> (Uuid, SimulationFixture) {
    let config_id = harness
        .seed_simulation_v2_strategy_config(actor, strategy_id)
        .await;
    let fixture = harness
        .seed_simulation_v2_pending(actor, config_id, strategy_id, START, END)
        .await;
    (config_id, fixture)
}

#[tokio::test]
async fn v2_five_default_strategies_keep_version_metadata_in_run_and_list() {
    let Some(harness) = setup().await else {
        return;
    };
    let contract_version =
        job_queue::owner_beta::simulation_execution::SimulationContractVersion::V2
            .contract_version();
    let result_schema =
        job_queue::owner_beta::simulation_execution::SimulationContractVersion::V2.result_schema();
    let strategies = job_queue::owner_beta::simulation::v2::V2Strategy::ALL;
    let mut expected_ids = Vec::new();
    for strategy in strategies {
        let (config_id, fixture) = fixture(&harness, &harness.owner, strategy.as_str()).await;
        expected_ids.push(fixture.run_id.to_string());
        let response = harness
            .get(&run_path(fixture.run_id), Some(&harness.owner))
            .await;
        assert_eq!(status(&response), StatusCode::OK);
        let body = Harness::body_json(response).await;
        assert_eq!(body["id"], fixture.run_id.to_string());
        assert_eq!(body["strategy_config_id"], config_id.to_string());
        assert_eq!(body["strategy_id"], strategy.as_str());
        assert_eq!(body["contract_version"], contract_version);
        assert_eq!(
            body["contract_document_sha256"],
            job_queue::owner_beta::simulation_execution::SimulationContractVersion::V2
                .contract_sha256()
        );
        assert_eq!(body["result_schema_version"], result_schema);
        assert_eq!(body["audience"], "OWNER_ONLY");
        assert_eq!(body["capability"], "PRICE_RETURN_ONLY");
        assert_eq!(body["start_date"], START);
        assert_eq!(body["end_date"], END);
    }

    let page = harness
        .get(&format!("{BASE}/runs"), Some(&harness.owner))
        .await;
    assert_eq!(status(&page), StatusCode::OK);
    let page = Harness::body_json(page).await;
    let items = page["items"].as_array().expect("closed simulation page");
    assert_eq!(items.len(), strategies.len());
    for item in items {
        assert_eq!(item["contract_version"], contract_version);
        assert_eq!(item["result_schema_version"], result_schema);
        assert!(expected_ids.contains(&item["id"].as_str().unwrap().to_owned()));
    }
    harness.teardown().await;
}

#[tokio::test]
async fn v2_success_pairs_read_all_defaults_and_preserve_v1_history() {
    let Some(harness) = setup().await else {
        return;
    };
    use job_queue::owner_beta::{
        simulation::v2::V2Strategy, simulation_execution::SimulationContractVersion,
    };

    let v2_contract = SimulationContractVersion::V2.contract_version();
    let v2_result_schema = SimulationContractVersion::V2.result_schema();
    let mut v2_fixtures = Vec::new();
    for strategy in V2Strategy::ALL {
        let (config_id, fixture) = fixture(&harness, &harness.owner, strategy.as_str()).await;
        let (illustrative_hash, zero_cost_hash, illustrative_result, zero_cost_result) = harness
            .seed_simulation_v2_success_pair(&fixture, strategy.as_str(), START, END)
            .await;

        let run = harness
            .get(&run_path(fixture.run_id), Some(&harness.owner))
            .await;
        assert_eq!(status(&run), StatusCode::OK);
        let run = Harness::body_json(run).await;
        assert_eq!(run["id"], fixture.run_id.to_string());
        assert_eq!(run["strategy_config_id"], config_id.to_string());
        assert_eq!(run["strategy_id"], strategy.as_str());
        assert_eq!(run["status"], "SUCCEEDED");
        assert_eq!(run["contract_version"], v2_contract);
        assert_eq!(
            run["contract_document_sha256"],
            SimulationContractVersion::V2.contract_sha256()
        );
        assert_eq!(run["result_schema_version"], v2_result_schema);

        let pair = harness
            .get(&pair_path(fixture.run_id), Some(&harness.owner))
            .await;
        assert_eq!(status(&pair), StatusCode::OK);
        let pair = Harness::body_json(pair).await;
        assert_eq!(pair["run_id"], fixture.run_id.to_string());
        assert_eq!(pair["status"], "SUCCEEDED");
        for (profile, expected_hash, expected_result) in [
            (
                "illustrative_10bp_per_side",
                illustrative_hash,
                illustrative_result,
            ),
            ("zero_cost_reference", zero_cost_hash, zero_cost_result),
        ] {
            assert_eq!(pair[profile]["result_content_sha256"], expected_hash);
            assert_eq!(pair[profile]["result"], expected_result);
            assert_eq!(pair[profile]["result"]["schema"], v2_result_schema);
            assert_eq!(pair[profile]["result"]["contract_version"], v2_contract);
            assert_eq!(
                pair[profile]["result"]["strategy"]["strategy"],
                strategy.as_str()
            );
        }
        v2_fixtures.push((strategy, fixture));
    }

    let v1_config_id = harness
        .seed_simulation_strategy_config(&harness.owner)
        .await;
    let v1_fixture = harness
        .seed_simulation_pending(&harness.owner, v1_config_id, START, END)
        .await;
    let (v1_illustrative_hash, v1_zero_cost_hash) =
        harness.seed_simulation_success_pair(&v1_fixture).await;
    let v1_run = harness
        .get(&run_path(v1_fixture.run_id), Some(&harness.owner))
        .await;
    assert_eq!(status(&v1_run), StatusCode::OK);
    let v1_run = Harness::body_json(v1_run).await;
    assert_eq!(v1_run["id"], v1_fixture.run_id.to_string());
    assert_eq!(v1_run["strategy_config_id"], v1_config_id.to_string());
    assert_eq!(v1_run["strategy_id"], "trend_following");
    assert_eq!(v1_run["status"], "SUCCEEDED");
    assert_eq!(
        v1_run["contract_version"],
        SimulationContractVersion::V1.contract_version()
    );
    assert_eq!(
        v1_run["result_schema_version"],
        SimulationContractVersion::V1.result_schema()
    );
    let v1_pair = harness
        .get(&pair_path(v1_fixture.run_id), Some(&harness.owner))
        .await;
    assert_eq!(status(&v1_pair), StatusCode::OK);
    let v1_pair = Harness::body_json(v1_pair).await;
    assert_eq!(v1_pair["run_id"], v1_fixture.run_id.to_string());
    assert_eq!(v1_pair["status"], "SUCCEEDED");
    assert_eq!(
        v1_pair["illustrative_10bp_per_side"]["result_content_sha256"],
        v1_illustrative_hash
    );
    assert_eq!(
        v1_pair["zero_cost_reference"]["result_content_sha256"],
        v1_zero_cost_hash
    );
    assert_eq!(
        v1_pair["illustrative_10bp_per_side"]["result"]["schema"],
        SimulationContractVersion::V1.result_schema()
    );
    assert_eq!(
        v1_pair["illustrative_10bp_per_side"]["result"]["contract_version"],
        SimulationContractVersion::V1.contract_version()
    );

    let page = harness
        .get(&format!("{BASE}/runs"), Some(&harness.owner))
        .await;
    assert_eq!(status(&page), StatusCode::OK);
    let page = Harness::body_json(page).await;
    let items = page["items"].as_array().expect("closed mixed-version page");
    assert_eq!(items.len(), V2Strategy::ALL.len() + 1);
    for (strategy, fixture) in v2_fixtures {
        let id = fixture.run_id.to_string();
        let item = items
            .iter()
            .find(|item| item["id"].as_str() == Some(id.as_str()))
            .expect("succeeded v2 run is present in history");
        assert_eq!(item["strategy_id"], strategy.as_str());
        assert_eq!(item["status"], "SUCCEEDED");
        assert_eq!(item["contract_version"], v2_contract);
        assert_eq!(item["result_schema_version"], v2_result_schema);
    }
    let v1_id = v1_fixture.run_id.to_string();
    let v1_item = items
        .iter()
        .find(|item| item["id"].as_str() == Some(v1_id.as_str()))
        .expect("succeeded v1 run remains in mixed-version history");
    assert_eq!(v1_item["strategy_id"], "trend_following");
    assert_eq!(v1_item["status"], "SUCCEEDED");
    assert_eq!(
        v1_item["contract_version"],
        SimulationContractVersion::V1.contract_version()
    );
    assert_eq!(
        v1_item["result_schema_version"],
        SimulationContractVersion::V1.result_schema()
    );
    harness.teardown().await;
}

#[tokio::test]
async fn v2_foreign_and_missing_are_indistinguishable_and_revocation_keeps_cancel() {
    let Some(harness) = setup().await else {
        return;
    };
    let (_, run) = fixture(&harness, &harness.owner, "relative_momentum").await;
    let owner_b = harness
        .seed_user(
            auth::entitlement::Role::Owner,
            "simulation-v2-owner-b@lagrange.test",
            "simulation-v2-owner-b-iss",
            "simulation-v2-owner-b-sub",
        )
        .await;
    let mut connection = harness.app_pool.acquire().await.expect("app connection");
    let mut owner_a_tx = connection.begin().await.expect("owner A transaction");
    api_server::actor_tx::set_actor_guc(&mut owner_a_tx, &harness.owner.actor())
        .await
        .expect("owner A GUC is transaction-local");
    let visible_as_a: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM public.owner_beta_simulation_runs WHERE id=$1",
    )
    .bind(run.run_id)
    .fetch_one(&mut *owner_a_tx)
    .await
    .expect("owner A sees its v2 row");
    assert_eq!(visible_as_a, 1);
    owner_a_tx
        .commit()
        .await
        .expect("owner A transaction commits");

    let mut owner_b_tx = connection.begin().await.expect("owner B transaction");
    api_server::actor_tx::set_actor_guc(&mut owner_b_tx, &owner_b.actor())
        .await
        .expect("owner B GUC replaces owner A on the same connection");
    let visible_as_b: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM public.owner_beta_simulation_runs WHERE id=$1",
    )
    .bind(run.run_id)
    .fetch_one(&mut *owner_b_tx)
    .await
    .expect("owner B cannot see owner A v2 row");
    assert_eq!(visible_as_b, 0);
    owner_b_tx
        .commit()
        .await
        .expect("owner B transaction commits");

    let foreign = harness.get(&run_path(run.run_id), Some(&owner_b)).await;
    let missing = harness.get(&run_path(Uuid::new_v4()), Some(&owner_b)).await;
    assert_eq!(status(&foreign), StatusCode::NOT_FOUND);
    assert_eq!(status(&missing), StatusCode::NOT_FOUND);
    assert_eq!(
        Harness::error_code(&Harness::body_json(foreign).await),
        Harness::error_code(&Harness::body_json(missing).await)
    );

    harness.revoke_simulation_backtest_rights().await;
    let denied = harness
        .get(&run_path(run.run_id), Some(&harness.owner))
        .await;
    assert_eq!(status(&denied), StatusCode::FORBIDDEN);
    assert_eq!(
        Harness::error_code(&Harness::body_json(denied).await),
        "DATA_ENTITLEMENT_REQUIRED"
    );
    let canceled = harness
        .send(
            "POST",
            &cancel_path(run.run_id),
            Some(&harness.owner),
            true,
            Some(Harness::RID),
            Some("simulation-v2-revoked-cancel"),
            Some(json!({})),
        )
        .await;
    assert_eq!(status(&canceled), StatusCode::OK);
    assert_eq!(
        Harness::body_json(canceled).await["status"],
        "CANCEL_REQUESTED"
    );
    harness.teardown().await;
}

#[tokio::test]
async fn v2_readers_reject_corrupt_mixed_and_truncated_pair_members_atomically() {
    let Some(harness) = setup().await else {
        return;
    };
    let (_, corrupt) = fixture(&harness, &harness.owner, "buy_and_hold").await;
    harness.seed_simulation_corrupt_success_pair(&corrupt).await;
    let (_, mixed) = fixture(&harness, &harness.owner, "trend_following").await;
    harness.seed_simulation_success_pair(&mixed).await; // valid v1 members under v2 row binding
    let (_, truncated) = fixture(&harness, &harness.owner, "dual_momentum").await;
    harness
        .seed_simulation_incomplete_success_pair(&truncated)
        .await;

    for fixture in [corrupt, mixed, truncated] {
        for path in [run_path(fixture.run_id), pair_path(fixture.run_id)] {
            let response = harness.get(&path, Some(&harness.owner)).await;
            assert_eq!(
                status(&response),
                StatusCode::UNPROCESSABLE_ENTITY,
                "{path}"
            );
            let body = Harness::body_json(response).await;
            assert_eq!(Harness::error_code(&body), "RESULT_INTEGRITY_FAILED");
            assert!(body.get("illustrative_10bp_per_side").is_none());
            assert!(body.get("zero_cost_reference").is_none());
        }
    }
    let list = harness
        .get(&format!("{BASE}/runs"), Some(&harness.owner))
        .await;
    assert_eq!(status(&list), StatusCode::UNPROCESSABLE_ENTITY);
    harness.teardown().await;
}
