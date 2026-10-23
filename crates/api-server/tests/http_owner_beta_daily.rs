//! Real HTTP/app-RLS tests against synthetic daily SQL evidence. No Raw,
//! filesystem approval, provider or production execution is exercised.
mod common;
mod owner_beta_daily_support;

use axum::http::StatusCode;
use common::Harness;
use job_queue::owner_beta::runner::daily_runtime::DailyProcessMode;
use job_queue::owner_beta::{
    OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE, OwnerBetaPublicationFailure,
    OwnerBetaPublicationOutcome,
};
use owner_beta_daily_support::{
    DAILY_PATH, RUNS_PATH, assert_contract, control, create, historical, patch, publish_fixture,
    ready_input, reserve, run, state,
};
use serde_json::{Value, json};
use uuid::Uuid;

const ON: DailyProcessMode = DailyProcessMode::OwnerOnly;

#[tokio::test]
async fn http_daily_config_defaults_and_partial_updates_preserve_optin_and_epoch() {
    run("config_defaults_partial", ON, |h| async move {
        let (id, initial) = create(&h, &h.owner, None).await?;
        assert_eq!(initial["owner_beta_daily_enabled"], false);
        assert_eq!(initial["owner_beta_daily_epoch"], 0);
        let response = patch(
            &h,
            &h.owner,
            id,
            json!({"owner_beta_daily_enabled":true}),
            "daily-enable",
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let enabled = Harness::body_json(response).await;
        assert_contract("StrategyConfig", &enabled);
        assert_eq!(enabled["owner_beta_daily_enabled"], true);
        assert_eq!(enabled["owner_beta_daily_epoch"], 1);
        let response = patch(
            &h,
            &h.owner,
            id,
            json!({"config":{"lookback":201}}),
            "daily-config-edit",
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let edited = Harness::body_json(response).await;
        assert_eq!(edited["owner_beta_daily_enabled"], true);
        assert_eq!(edited["owner_beta_daily_epoch"], 2);
        assert_eq!(edited["config"], json!({"lookback":201}));
        let response = patch(
            &h,
            &h.owner,
            id,
            json!({"is_active":false}),
            "daily-inactive",
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let inactive = Harness::body_json(response).await;
        assert_eq!(inactive["owner_beta_daily_enabled"], true);
        assert_eq!(inactive["owner_beta_daily_epoch"], 3);
        let response = patch(
            &h,
            &h.owner,
            id,
            json!({"owner_beta_daily_enabled":false}),
            "daily-disable",
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let disabled = Harness::body_json(response).await;
        assert_eq!(disabled["owner_beta_daily_enabled"], false);
        assert_eq!(disabled["owner_beta_daily_epoch"], 4);
        let response = patch(
            &h,
            &h.owner,
            id,
            json!({"owner_beta_daily_enabled":false}),
            "daily-same-disable",
        )
        .await;
        assert_eq!(
            Harness::body_json(response).await["owner_beta_daily_epoch"],
            4
        );
        let response = h
            .get(&format!("/api/v1/strategy-configs/{id}"), Some(&h.owner))
            .await;
        let saved = Harness::body_json(response).await;
        assert_eq!(saved["owner_beta_daily_enabled"], false);
        assert_eq!(
            saved["owner_beta_daily_epoch"],
            disabled["owner_beta_daily_epoch"]
        );
        assert_eq!(saved["config"], disabled["config"]);
        Ok(())
    })
    .await;
}
#[tokio::test]
async fn http_daily_config_requires_owner_csrf_and_closed_request_fields() {
    run("config_authority",ON,|h| async move {
  let (member_id,_)=create(&h,&h.member,None).await?;
  let forbidden=patch(&h,&h.member,member_id,json!({"owner_beta_daily_enabled":true}),"member-enable").await;
  assert_eq!(forbidden.status(),StatusCode::FORBIDDEN);
  let create_denied=h.post("/api/v1/strategies/buy_and_hold/configs",Some(&h.member),true,
   json!({"strategy_version":"1.0.0","config":{"lookback":200},"owner_beta_daily_enabled":true})).await;
  assert_eq!(create_denied.status(),StatusCode::FORBIDDEN);
  let (id,_)=create(&h,&h.owner,Some(true)).await?;
  let path=format!("/api/v1/strategy-configs/{id}");
  let csrf=h.send("PATCH",&path,Some(&h.owner),false,None,Some("no-csrf"),Some(json!({"owner_beta_daily_enabled":false}))).await;
  assert_eq!(csrf.status(),StatusCode::FORBIDDEN);
  for body in [
   json!({}),json!({"owner_beta_daily_enabled":null}),json!({"is_active":null}),
   json!({"config":null}),json!({"owner_beta_daily_epoch":0}),json!({"input":{}}),
  ] {
   let response=patch(&h,&h.owner,id,body,&Uuid::new_v4().to_string()).await;
   assert_eq!(response.status(),StatusCode::BAD_REQUEST);
  }
  let invalid=patch(&h,&h.owner,id,json!({"config":{"lookback":0}}),"invalid-params").await;
  assert_eq!(invalid.status(),StatusCode::UNPROCESSABLE_ENTITY);
  let foreign=patch(&h,&h.owner,member_id,json!({"is_active":false}),"foreign-config").await;
  assert_eq!(foreign.status(),StatusCode::NOT_FOUND);
  let response=h.get(&path,Some(&h.owner)).await;
  let saved=Harness::body_json(response).await;
  assert_eq!(saved["owner_beta_daily_enabled"],true);
  assert_eq!(saved["owner_beta_daily_epoch"],0);
  Ok(())
 }).await;
}
#[tokio::test]
async fn http_daily_config_idempotency_is_bound_to_resource_and_audit_once() {
    run("config_idempotency",ON,|h| async move {
  let (a,_)=create(&h,&h.owner,None).await?;
  let (b,_)=create(&h,&h.owner,None).await?;
  let body=json!({"owner_beta_daily_enabled":true});
  let first=patch(&h,&h.owner,a,body.clone(),"same-daily-update-key").await;
  assert_eq!(first.status(),StatusCode::OK);
  let first=Harness::body_json(first).await;
  let replay=patch(&h,&h.owner,a,body.clone(),"same-daily-update-key").await;
  assert_eq!(replay.status(),StatusCode::OK);
  assert_eq!(Harness::body_json(replay).await,first);
  let other=patch(&h,&h.owner,b,body,"same-daily-update-key").await;
  assert_eq!(other.status(),StatusCode::CONFLICT);
  let changed=patch(&h,&h.owner,a,json!({"owner_beta_daily_enabled":false}),"same-daily-update-key").await;
  assert_eq!(changed.status(),StatusCode::CONFLICT);
  let response=h.get(&format!("/api/v1/strategy-configs/{b}"),Some(&h.owner)).await;
  assert_eq!(Harness::body_json(response).await["owner_beta_daily_enabled"],false);
  let count:i64=sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action='strategy_config.update' AND target_id=$1")
   .bind(a.to_string()).fetch_one(&h.admin_pool).await?;
  assert_eq!(count,1);
  Ok(())
 }).await;
}
#[tokio::test]
async fn http_daily_state_enforces_session_owner_entitlement_and_config_rls() {
    run("state_authority", ON, |h| async move {
        let (id, _) = create(&h, &h.owner, Some(true)).await?;
        let (foreign, _) = create(&h, &h.member, None).await?;
        let path = format!("{DAILY_PATH}?strategy_config_id={id}");
        assert_eq!(h.get(&path, None).await.status(), StatusCode::UNAUTHORIZED);
        let expired = h.seed_user(auth::entitlement::Role::Owner,
            "daily-expired@lagrange.test", "daily-expired", "daily-expired").await;
        h.seed_tenant(&expired, &format!(
            "UPDATE web_sessions SET created_at=now()-interval '2 hours', expires_at=now()-interval '1 second' WHERE user_id='{}'",
            expired.user_id)).await;
        let response = h.get(&path, Some(&expired)).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(Harness::error_code(&Harness::body_json(response).await), "SESSION_EXPIRED");
        assert_eq!(
            h.get(&path, Some(&h.member)).await.status(),
            StatusCode::FORBIDDEN
        );
        for query in [
            "",
            "?strategy_config_id=bad",
            "?strategy_config_id=00000000-0000-0000-0000-000000000000",
            "?strategy_config_id=bad&extra=1",
        ] {
            assert_eq!(
                h.get(&format!("{DAILY_PATH}{query}"), Some(&h.owner))
                    .await
                    .status(),
                StatusCode::BAD_REQUEST
            );
        }
        assert_eq!(
            h.get(&format!("{path}&extra=1"), Some(&h.owner))
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            h.get(
                &format!("{DAILY_PATH}?strategy_config_id={foreign}"),
                Some(&h.owner)
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
        sqlx::query("UPDATE data_entitlements SET status='REVOKED' WHERE status='ACTIVE'")
            .execute(&h.owner_pool)
            .await?;
        let response = h.get(&path, Some(&h.owner)).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            Harness::error_code(&Harness::body_json(response).await),
            "DATA_ENTITLEMENT_REQUIRED"
        );
        Ok(())
    })
    .await;
}
#[tokio::test]
async fn http_daily_state_is_read_only_and_never_uses_generic_ready_data() {
    run("state_no_fallback",DailyProcessMode::Disabled,|h| async move {
  let (id,_)=create(&h,&h.owner,Some(true)).await?;
  let before:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM jobs),(SELECT count(*) FROM owner_beta_recommendation_runs),(SELECT count(*) FROM audit_logs)")
   .fetch_one(&h.admin_pool).await?;
  let disabled=state(&h,id).await;
  assert_eq!(disabled["state"],"DISABLED");assert_eq!(disabled["reason_code"],"RUNTIME_DISABLED");
  assert_eq!(disabled["expected_session"],Value::Null);assert_eq!(disabled["data_support_end"],Value::Null);
  assert_eq!(disabled["recommendation_as_of"],Value::Null);
  control(&h,true).await?;
  let _=historical(&h,id).await?;
  let current=state(&h,id).await;
  assert_eq!(current["state"],"DISABLED");
  assert_eq!(current["data_support_end"],Value::Null);
  assert_eq!(current["recommendation_as_of"],Value::Null);
  let after:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM jobs),(SELECT count(*) FROM owner_beta_recommendation_runs),(SELECT count(*) FROM audit_logs)")
   .fetch_one(&h.admin_pool).await?;
  assert_eq!(after,(before.0+1,before.1+1,before.2));
  Ok(())
 }).await;
}
#[tokio::test]
async fn http_daily_state_reads_exact_admission_and_rejects_expired_or_blocked_input() {
    run("state_admission",ON,|h| async move {
  let (id,_)=create(&h,&h.owner,Some(true)).await?;
  let absent=state(&h,id).await;
  assert_eq!(absent["state"],"UNVERIFIABLE");assert_eq!(absent["reason_code"],"CONTROL_UNAVAILABLE");
  let end=ready_input(&h).await?;
  let fresh=state(&h,id).await;
  assert_eq!(fresh["expected_session"],end.to_string());assert_eq!(fresh["data_support_end"],end.to_string());
  assert_eq!(fresh["recommendation_as_of"],Value::Null);assert_eq!(fresh["reason_code"],"AWAITING_SCHEDULE");
  assert_ne!(fresh["state"],"CURRENT");
  sqlx::query("UPDATE owner_beta_daily_poll_receipts SET started_at=clock_timestamp()-interval '302 seconds',completed_at=clock_timestamp()-interval '301 seconds' WHERE stage='ADMISSION'")
   .execute(&h.owner_pool).await?;
  let stale=state(&h,id).await;
  let observed=chrono::DateTime::parse_from_rfc3339(stale["observed_at"].as_str().ok_or("observed")?)?;
  let due=chrono::DateTime::parse_from_rfc3339(stale["data_due_at"].as_str().ok_or("due")?)?;
  assert_eq!(stale["state"],if observed>=due {"STALE"}else{"UNVERIFIABLE"});
  assert_eq!(stale["reason_code"],"VERIFICATION_EXPIRED");
  assert_eq!(stale["data_support_end"],Value::Null);assert_eq!(stale["expected_session"],end.to_string());
  sqlx::query("INSERT INTO owner_beta_daily_source_events(event_sha256,owner_user_id,source_kind,range_start,range_end,code,evidence_sha256) VALUES($1,$2,'BAR',$3,$3,'SEALED_SOURCE_CHANGED',$4)")
   .bind(format!("sha256:{}","c".repeat(64))).bind(h.owner.user_id).bind(end).bind(format!("sha256:{}","a".repeat(64)))
   .execute(&h.owner_pool).await?;
  let blocked=state(&h,id).await;
  assert_eq!(blocked["state"],"INPUT_BLOCKED");assert_eq!(blocked["reason_code"],"SEALED_SOURCE_CHANGED");
  assert_eq!(blocked["expected_session"],end.to_string());
  assert_eq!(blocked["data_support_end"],Value::Null);
  Ok(())
 }).await;
}
#[tokio::test]
async fn http_daily_read_union_preserves_manual_pins_and_daily_pending_identity() {
    run("run_union", ON, |h| async move {
        let (config, _) = create(&h, &h.owner, Some(true)).await?;
        ready_input(&h).await?;
        let (worker, _, daily) = reserve(&h).await?;
        let manual = historical(&h, config).await?;
        for (id, kind, trigger) in [(daily, "daily", "DAILY"), (manual, "historical", "MANUAL")] {
            let response = h.get(&format!("{RUNS_PATH}/{id}"), Some(&h.owner)).await;
            assert_eq!(response.status(), StatusCode::OK);
            let body = Harness::body_json(response).await;
            assert_contract("OwnerBetaPriceOnlyReadRun", &body);
            assert_eq!(body["input"]["kind"], kind);
            assert_eq!(body["trigger_kind"], trigger);
            assert_eq!(body["status"], "PENDING");
            assert_eq!(body["items"], json!([]));
            assert!(body.get("published_at").is_none());
            for pin in [
                "candidate_content_sha256",
                "artifact_manifest_sha256",
                "stage5_manifest_sha256",
                "action_manifest_sha256",
                "approval_registry_sha256",
            ] {
                if kind == "daily" {
                    assert!(body.get(pin).is_none());
                    assert!(body["input"]["base"][pin].is_string());
                } else {
                    assert_eq!(body[pin], body["input"][pin]);
                    assert!(body[pin].is_string());
                }
            }
        }
        let page = h.get(&format!("{RUNS_PATH}?limit=1"), Some(&h.owner)).await;
        assert_eq!(page.status(), StatusCode::OK);
        let page = Harness::body_json(page).await;
        assert_contract("OwnerBetaPriceOnlyReadPage", &page);
        assert_eq!(page["items"].as_array().ok_or("page")?.len(), 1);
        let cursor = page["next_cursor"].as_str().ok_or("next cursor")?;
        let next = h
            .get(
                &format!("{RUNS_PATH}?limit=1&cursor={cursor}"),
                Some(&h.owner),
            )
            .await;
        assert_eq!(next.status(), StatusCode::OK);
        let next = Harness::body_json(next).await;
        assert_contract("OwnerBetaPriceOnlyReadPage", &next);
        assert_eq!(next["items"].as_array().ok_or("next page")?.len(), 1);
        assert_ne!(page["items"][0]["id"], next["items"][0]["id"]);
        assert!(next["items"][0].get("items").is_none());
        assert_eq!(
            h.get(&format!("{RUNS_PATH}/{daily}"), Some(&h.member))
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        worker.close().await;
        Ok(())
    })
    .await;
}
#[tokio::test]
async fn http_daily_state_distinguishes_running_retry_and_terminal_failure() {
    run("queue_failure", ON, |h| async move {
        let (config, _) = create(&h, &h.owner, Some(true)).await?;
        ready_input(&h).await?;
        let (worker, queue, _) = reserve(&h).await?;
        let queued = state(&h, config).await;
        assert_eq!(queued["reason_code"], "JOB_QUEUED");
        let claim = queue
            .claim_next_for("api-retry", OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE)
            .await?
            .ok_or("claim")?;
        let running = state(&h, config).await;
        assert_eq!(running["reason_code"], "JOB_RUNNING");
        let input = serde_json::from_value(claim.job.payload_json.clone())?;
        let settled = job_queue::owner_beta::publish::settle_owner_beta_daily_failure(
            &worker,
            &queue,
            &claim,
            &input,
            OwnerBetaPublicationFailure::ComputationUnavailable,
        )
        .await?;
        assert_eq!(settled, OwnerBetaPublicationOutcome::Retrying);
        let retry = state(&h, config).await;
        assert_eq!(retry["reason_code"], "JOB_RETRYING");
        // Make the next attempt available through the allowed worker transition,
        // without sleeps or editing protected daily timestamps.
        let second = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Some(claim) = queue
                    .claim_next_for("api-terminal", OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE)
                    .await?
                {
                    break Ok::<_, job_queue::QueueError>(claim);
                }
                tokio::task::yield_now().await;
            }
        })
        .await??;
        let settled = job_queue::owner_beta::publish::settle_owner_beta_daily_failure(
            &worker,
            &queue,
            &second,
            &input,
            OwnerBetaPublicationFailure::InputInvalid,
        )
        .await?;
        assert_eq!(settled, OwnerBetaPublicationOutcome::Failed);
        let failed = state(&h, config).await;
        assert_eq!(failed["state"], "FAILED");
        assert_eq!(failed["reason_code"], "JOB_FAILED");
        assert_eq!(failed["data_due_at"], queued["data_due_at"]);
        assert_eq!(failed["recommendation_as_of"], Value::Null);
        worker.close().await;
        Ok(())
    })
    .await;
}
#[tokio::test]
async fn http_daily_published_result_retains_dates_across_disable_and_verification_loss() {
    run("published_dates",ON,|h| async move {
  let (config,_)=create(&h,&h.owner,Some(true)).await?;
  let end=ready_input(&h).await?;
  let (worker,queue,_)=reserve(&h).await?;
  let id=publish_fixture(&h,&worker,&queue).await?;
  let current=state(&h,config).await;
  assert_eq!(current["state"],"CURRENT");assert_eq!(current["recommendation_as_of"],end.to_string());
  assert_eq!(current["input_kind"],"owner_beta_daily_etf11_price_only_v1");assert_eq!(current["trigger_kind"],"DAILY");
  let response=h.get(&format!("{RUNS_PATH}/{id}"),Some(&h.owner)).await;
  assert_eq!(response.status(),StatusCode::OK);
  let detail=Harness::body_json(response).await;
  assert_contract("OwnerBetaPriceOnlyReadRun",&detail);
  assert_eq!(detail["items"].as_array().ok_or("items")?.len(),11);
  for item in detail["items"].as_array().ok_or("items")? {
   if item["excluded"]==true { assert!(item.get("target_weight").is_none()); }
  }
  assert_eq!(detail["published_at"],detail["finished_at"]);assert_eq!(current["published_at"],detail["published_at"]);
  sqlx::query("UPDATE owner_beta_daily_poll_receipts SET started_at=clock_timestamp()-interval '302 seconds',completed_at=clock_timestamp()-interval '301 seconds' WHERE stage='ADMISSION'")
   .execute(&h.owner_pool).await?;
  let expired=state(&h,config).await;
  assert_eq!(expired["state"],"UNVERIFIABLE");assert_eq!(expired["data_support_end"],Value::Null);
  assert_eq!(expired["recommendation_as_of"],current["recommendation_as_of"]);assert_eq!(expired["published_at"],current["published_at"]);
  let response=patch(&h,&h.owner,config,json!({"owner_beta_daily_enabled":false}),"disable-published").await;
  assert_eq!(response.status(),StatusCode::OK);
  let disabled=state(&h,config).await;
  assert_eq!(disabled["state"],"DISABLED");assert_eq!(disabled["published_at"],current["published_at"]);
  worker.close().await;Ok(())
 }).await;
}
#[tokio::test]
async fn http_daily_state_rechecks_exact_control_rights_and_calendar_gap() {
    run("exact_rights_calendar",ON,|h| async move {
  let (id,_)=create(&h,&h.owner,Some(true)).await?;
  control(&h,true).await?;
  let gap=state(&h,id).await;
  assert_eq!(gap["state"],"UNVERIFIABLE");assert_eq!(gap["reason_code"],"CALENDAR_COVERAGE_GAP");
  assert_eq!(gap["expected_session"],Value::Null);
  sqlx::query("UPDATE data_entitlements SET status='REVOKED' WHERE contract_reference='daily-fixture'").execute(&h.owner_pool).await?;
  // The unrelated generic active entitlement still exists. It cannot stand in
  // for the control's exact contract reference/hash/date range.
  let response=h.get(&format!("{DAILY_PATH}?strategy_config_id={id}"),Some(&h.owner)).await;
  assert_eq!(response.status(),StatusCode::FORBIDDEN);
  assert_eq!(Harness::error_code(&Harness::body_json(response).await),"FORBIDDEN");
  Ok(())
 }).await;
}
