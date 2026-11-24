//! Owner-beta Paper's disabled boundary and authenticated router surface.

mod common;
#[allow(dead_code)]
mod owner_beta_daily_support;

use axum::http::StatusCode;
use common::{Harness, status};
use serde_json::{Value, json};

const PREVIEW_ID: &str = "00000000-0000-0000-0000-000000000003";

fn paper_routes() -> [(&'static str, &'static str, Option<Value>); 6] {
    [
        ("GET", "/api/v1/paper/owner-beta/state", None),
        (
            "POST",
            "/api/v1/paper/owner-beta/consent",
            Some(json!({"opt_in": true, "expected_epoch": 0})),
        ),
        (
            "POST",
            "/api/v1/paper/owner-beta/previews",
            Some(json!({
                "recommendation_run_id": "00000000-0000-0000-0000-000000000001",
                "expected_epoch": 0
            })),
        ),
        (
            "GET",
            "/api/v1/paper/owner-beta/previews/00000000-0000-0000-0000-000000000003",
            None,
        ),
        (
            "POST",
            "/api/v1/paper/owner-beta/previews/00000000-0000-0000-0000-000000000003/apply",
            Some(
                json!({"preview_token": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "expected_epoch": 0}),
            ),
        ),
        (
            "GET",
            "/api/v1/paper/owner-beta/sessions/00000000-0000-0000-0000-000000000004",
            None,
        ),
    ]
}

#[tokio::test]
async fn owner_beta_paper_routes_keep_owner_gate_and_typed_disabled_boundary() {
    let Some(mut harness) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    harness.restart_api_with_owner_beta_access().await;

    for (method, path, body) in paper_routes() {
        let member = harness
            .send(
                method,
                path,
                Some(&harness.member),
                false,
                Some("paper-member"),
                None,
                body.clone(),
            )
            .await;
        assert_eq!(status(&member), StatusCode::FORBIDDEN, "{method} {path}");
        assert_eq!(
            Harness::error_code(&Harness::body_json(member).await),
            "FORBIDDEN",
            "{method} {path}"
        );

        let owner = harness
            .send(
                method,
                path,
                Some(&harness.owner),
                false,
                Some("paper-owner-disabled"),
                None,
                body,
            )
            .await;
        if path.ends_with("/state") {
            assert_eq!(status(&owner), StatusCode::OK);
            let response_id = owner
                .headers()
                .get("x-request-id")
                .and_then(|value| value.to_str().ok())
                .expect("request correlation is echoed");
            assert_eq!(response_id, "paper-owner-disabled");
            assert_eq!(
                owner
                    .headers()
                    .get("cache-control")
                    .and_then(|v| v.to_str().ok()),
                Some("no-store")
            );
            let body = Harness::body_json(owner).await;
            assert_eq!(body["schema"], "owner-beta-paper-state-v1");
            assert_eq!(body["state"], "DISABLED");
            assert_eq!(body["sessions"], json!([]));
        } else {
            assert_eq!(status(&owner), StatusCode::FORBIDDEN, "{method} {path}");
            assert_eq!(
                Harness::error_code(&Harness::body_json(owner).await),
                "PAPER_DISABLED",
                "{method} {path}"
            );
        }
    }

    let generic_paper = harness
        .get("/api/v1/paper/accounts", Some(&harness.owner))
        .await;
    assert_eq!(status(&generic_paper), StatusCode::FORBIDDEN);
    assert_eq!(
        Harness::error_code(&Harness::body_json(generic_paper).await),
        "FORBIDDEN",
        "generic Paper keeps its existing disabled behavior"
    );

    harness.teardown().await;
}

#[tokio::test]
async fn owner_beta_paper_enabled_without_matching_protected_runtime_cannot_queue_or_apply() {
    let Some(mut harness) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    harness.restart_api_with_owner_beta_paper().await;

    let submit = harness
        .send(
            "POST",
            "/api/v1/paper/owner-beta/previews",
            Some(&harness.owner),
            true,
            Some("paper-runtime-required-submit"),
            Some("runtime-required-submit"),
            Some(json!({
                "recommendation_run_id": "00000000-0000-0000-0000-000000000001",
                "expected_epoch": 0
            })),
        )
        .await;
    assert_ne!(status(&submit), StatusCode::ACCEPTED);
    assert_ne!(status(&submit), StatusCode::OK);
    let submit_code = Harness::error_code(&Harness::body_json(submit).await);
    assert!(submit_code == "PAPER_UNBOUND" || submit_code == "PAPER_SOURCE_BLOCKED");

    let apply = harness
        .send(
            "POST",
            &format!("/api/v1/paper/owner-beta/previews/{PREVIEW_ID}/apply"),
            Some(&harness.owner),
            true,
            Some("paper-runtime-required-apply"),
            Some("runtime-required-apply"),
            Some(json!({
                "preview_token": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "expected_epoch": 0
            })),
        )
        .await;
    assert_ne!(status(&apply), StatusCode::OK);
    let apply_code = Harness::error_code(&Harness::body_json(apply).await);
    assert!(apply_code == "PAPER_UNBOUND" || apply_code == "PAPER_SOURCE_BLOCKED");

    let queued: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM jobs WHERE owner_user_id=$1 AND job_type='owner_beta_paper_preview'",
    )
    .bind(harness.owner.user_id)
    .fetch_one(&harness.owner_pool)
    .await
    .expect("scratch database queue count");
    let targets: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pending_targets WHERE owner_user_id=$1 AND source_kind IN ('OWNER_BETA_DAILY_MANUAL','OWNER_BETA_DAILY_AUTO')",
    )
    .bind(harness.owner.user_id)
    .fetch_one(&harness.owner_pool)
    .await
    .expect("scratch database target count");
    assert_eq!(queued, 0, "runtime refusal must not create a Paper job");
    assert_eq!(
        targets, 0,
        "runtime refusal must not create an economic target"
    );

    harness.teardown().await;
}

#[tokio::test]
async fn prepared_off_consent_uses_owner_csrf_rights_and_epoch_without_economic_routes() {
    let Some(mut harness) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let owner_id = harness.owner.user_id;
    harness.restart_api_with_paper_off_consent().await;
    let state_path = "/api/v1/paper/owner-beta/state";
    let consent_path = "/api/v1/paper/owner-beta/consent";
    let absent = harness.get(state_path, Some(&harness.owner)).await;
    assert_eq!(Harness::body_json(absent).await["state"], "DISABLED");

    owner_beta_daily_support::control(&harness, false)
        .await
        .unwrap();
    sqlx::query("INSERT INTO strategies(id,display_name) VALUES('trend_following','Trend following') ON CONFLICT DO NOTHING")
        .execute(&harness.owner_pool).await.unwrap();
    let config: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO user_strategy_configs(owner_user_id,strategy_id,strategy_version,config_json,daily_enabled) \
         VALUES($1,'trend_following','1.0.0',$2,true) RETURNING id",
    ).bind(owner_id).bind(json!({"benchmark_instrument":"069500.KRX","fast_ma":50,"slow_ma":200}))
     .fetch_one(&harness.owner_pool).await.unwrap();
    let contract = json!({
        "contract_id":"owner-beta-paper-v1-draft-1",
        "contract_sha256":"sha256:18cde9fa7f15e19c9ec272abb780d06ae4be672068ed3f9208c39dc2297ab8d0",
        "decision_sha256":"sha256:8ef602a35586d3e2ca532c5fbb5efc6f2e727eb25ebeb45a44dad0ea0d0ca394",
        "cost_profile_id":"KRX_ETF_DEFAULT",
        "cost_profile_version":1
    });
    let configured: Value = sqlx::query_scalar(
        "SELECT configure_owner_beta_paper_v1($1,'OFF consent fixture',$2,NULL::jsonb,'configure')",
    )
    .bind(config)
    .bind(contract)
    .fetch_one(&harness.owner_pool)
    .await
    .unwrap();
    assert_eq!(configured["outcome"], "OK");
    let initial = Harness::body_json(harness.get(state_path, Some(&harness.owner)).await).await;
    assert_eq!(initial["state"], "OFF");
    assert_eq!(initial["control_epoch"], 0);
    assert_eq!(initial["account_id"], configured["account_id"]);
    assert_eq!(initial["consent_present"], false);
    assert_eq!(initial["sessions"], json!([]));

    for (method, path, body) in paper_routes()
        .into_iter()
        .filter(|(_, path, _)| *path != state_path && *path != consent_path)
    {
        let denied = harness
            .send(
                method,
                path,
                Some(&harness.owner),
                true,
                None,
                Some("not-economic"),
                body,
            )
            .await;
        assert_eq!(
            Harness::error_code(&Harness::body_json(denied).await),
            "PAPER_DISABLED"
        );
    }
    let generic = harness
        .get("/api/v1/paper/accounts", Some(&harness.owner))
        .await;
    assert_eq!(status(&generic), StatusCode::FORBIDDEN);

    let payload = || Some(json!({"expected_epoch":0,"opt_in":true}));
    let member = harness
        .send(
            "POST",
            consent_path,
            Some(&harness.member),
            true,
            None,
            Some("member"),
            payload(),
        )
        .await;
    assert_eq!(
        Harness::error_code(&Harness::body_json(member).await),
        "FORBIDDEN"
    );
    let csrf = harness
        .send(
            "POST",
            consent_path,
            Some(&harness.owner),
            false,
            None,
            Some("no-csrf"),
            payload(),
        )
        .await;
    assert_eq!(
        Harness::error_code(&Harness::body_json(csrf).await),
        "CSRF_DENIED"
    );
    let grant = harness
        .send(
            "POST",
            consent_path,
            Some(&harness.owner),
            true,
            None,
            Some("grant-consent"),
            payload(),
        )
        .await;
    assert_eq!(status(&grant), StatusCode::OK);
    assert_eq!(Harness::body_json(grant).await["consent_present"], true);
    let stale = harness
        .send(
            "POST",
            consent_path,
            Some(&harness.owner),
            true,
            None,
            Some("stale"),
            Some(json!({"expected_epoch":1,"opt_in":true})),
        )
        .await;
    assert_eq!(
        Harness::error_code(&Harness::body_json(stale).await),
        "PAPER_STALE"
    );

    sqlx::query(
        "UPDATE data_entitlements SET covered_uses=covered_uses-'paper_view'-'recommendation'",
    )
    .execute(&harness.owner_pool)
    .await
    .unwrap();
    let rights = harness
        .send(
            "POST",
            consent_path,
            Some(&harness.owner),
            true,
            None,
            Some("rights-missing"),
            payload(),
        )
        .await;
    assert_eq!(
        Harness::error_code(&Harness::body_json(rights).await),
        "DATA_ENTITLEMENT_REQUIRED"
    );
    let read = Harness::body_json(harness.get(state_path, Some(&harness.owner)).await).await;
    assert_eq!(read["consent_present"], true);
    let revoke = harness
        .send(
            "POST",
            consent_path,
            Some(&harness.owner),
            true,
            None,
            Some("revoke-consent"),
            Some(json!({"expected_epoch":0,"opt_in":false})),
        )
        .await;
    assert_eq!(status(&revoke), StatusCode::OK);
    let revocation = Harness::body_json(revoke).await;
    assert_eq!(revocation["consent_present"], false);
    assert_eq!(revocation["control_epoch"], 1);

    let (mode,jobs,targets): (String,i64,i64) = sqlx::query_as(
        "SELECT state,(SELECT count(*) FROM jobs),(SELECT count(*) FROM pending_targets) FROM owner_beta_paper_control",
    ).fetch_one(&harness.owner_pool).await.unwrap();
    assert_eq!((mode.as_str(), jobs, targets), ("OFF", 0, 0));

    // A retained API process may see the scheduler migration rolled back.
    // Its preflight refuses the old database even with opt-in still configured.
    let mut schema = harness.owner_pool.begin().await.unwrap();
    sqlx::raw_sql(include_str!(
        "../../../migrations/0070_exclude_owner_beta_paper_from_generic_schedule.down.sql"
    ))
    .execute(&mut *schema)
    .await
    .unwrap();
    schema.commit().await.unwrap();
    let old_schema = Harness::body_json(harness.get(state_path, Some(&harness.owner)).await).await;
    assert_eq!(old_schema["state"], "DISABLED");
    let maintenance = harness
        .send(
            "POST",
            consent_path,
            Some(&harness.owner),
            true,
            None,
            Some("old-schema"),
            Some(json!({"expected_epoch":1,"opt_in":false})),
        )
        .await;
    assert_eq!(
        Harness::error_code(&Harness::body_json(maintenance).await),
        "PAPER_NOT_READY"
    );
    let mut schema = harness.owner_pool.begin().await.unwrap();
    sqlx::raw_sql(include_str!(
        "../../../migrations/0070_exclude_owner_beta_paper_from_generic_schedule.up.sql"
    ))
    .execute(&mut *schema)
    .await
    .unwrap();
    schema.commit().await.unwrap();
    let reinstated = Harness::body_json(harness.get(state_path, Some(&harness.owner)).await).await;
    assert_eq!(reinstated["state"], "OFF");

    // Represent an earlier lifecycle solely in this disposable account's
    // ledger, while leaving the current control and its binding OFF.
    sqlx::query(
        "INSERT INTO owner_beta_paper_events(owner_user_id,account_id,control_epoch,kind,actor_user_id,payload,payload_sha256) \
         SELECT owner_user_id,account_id,control_epoch,'ENABLED',owner_user_id,'{}'::jsonb,daily_0060_digest('{}') \
         FROM owner_beta_paper_control",
    )
    .execute(&harness.owner_pool)
    .await
    .unwrap();
    let former = Harness::body_json(harness.get(state_path, Some(&harness.owner)).await).await;
    assert_eq!(former["state"], "DISABLED");

    sqlx::query("UPDATE owner_beta_paper_control SET state='RESTORED_OFF'")
        .execute(&harness.owner_pool)
        .await
        .unwrap();
    let outside = Harness::body_json(harness.get(state_path, Some(&harness.owner)).await).await;
    assert_eq!(outside["state"], "DISABLED");
    let blocked = harness
        .send(
            "POST",
            consent_path,
            Some(&harness.owner),
            true,
            None,
            Some("outside"),
            Some(json!({"expected_epoch":1,"opt_in":false})),
        )
        .await;
    assert_eq!(
        Harness::error_code(&Harness::body_json(blocked).await),
        "PAPER_NOT_READY"
    );

    harness.teardown().await;
}
