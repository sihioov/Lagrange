//! Todo 38: the PostgreSQL `RiskEventStore`, against a real database.
//!
//! `risk-gateway`'s own suite proves the decision logic against in-memory
//! doubles. What cannot be proven there is that the real store honours the
//! three promises the trait makes — committed on return, one decision per
//! intent, append-only — because all three are properties of the schema and
//! the RLS policy, not of the Rust.

mod common;

use common::{Harness, actor_pool};
use risk_gateway::{Check, DenyReason, evaluate, evaluate_and_record, testing};

/// Publishes the limit set the decisions reference. `risk_events.limits_version`
/// is a foreign key, so a decision naming an unpublished version cannot be
/// written at all — the audit row can never point at limits that never existed.
async fn publish_limits(h: &Harness) {
    sqlx::query(
        "INSERT INTO risk_limits (version, max_symbol_weight_bp, max_order_value, \
         max_daily_order_value, max_daily_loss, max_data_age_secs) \
         VALUES ('risk-limits-v1', 3000, 1000000, 5000000, 500000, 300) \
         ON CONFLICT (version) DO NOTHING",
    )
    .execute(&h.owner_pool)
    .await
    .expect("limits publish");
}

/// A pool carrying the owner's actor GUC.
///
/// `risk_events` is a FORCE-RLS tenant table: a pool without the GUC sees NO
/// rows on read and fails the policy's uuid cast on write. The repository
/// itself goes through `begin_actor_tx`, so only these verification queries
/// need it -- but reading through a bare pool would have made every assertion
/// below vacuously "row not found" rather than a real check.
async fn owner_view(h: &Harness) -> sqlx::PgPool {
    actor_pool(&h.app_url, &h.owner.user_id.to_string(), 2).await
}

fn repo(h: &Harness) -> api_server::repos::risk::RiskRepo {
    api_server::repos::risk::RiskRepo::new(
        h.app_pool.clone(),
        h.owner.actor(),
        h.owner.user_id,
        None,
    )
}

#[tokio::test]
async fn risk_store_records_an_approval_with_its_snapshot_and_check_trail() {
    let Some(h) = Harness::new().await else {
        return;
    };
    publish_limits(&h).await;

    let snapshot = testing::snapshot_all_green();
    let outcome = evaluate_and_record(&snapshot, &testing::limits(), &repo(&h)).await;
    let approval = outcome
        .into_approval()
        .expect("an all-green decision is approved");

    // The token names the row, so approval and audit trail join both ways.
    let row = sqlx::query_as::<_, (String, String, String, Option<String>, serde_json::Value)>(
        "SELECT decision, severity, reason_code, denied_by_check, payload_json \
         FROM risk_events WHERE id = $1::uuid",
    )
    .bind(approval.risk_event_id())
    .fetch_one(&owner_view(&h).await)
    .await
    .expect("the approval names a row that exists");

    assert_eq!(row.0, "APPROVED");
    assert_eq!(row.1, "INFO");
    assert_eq!(row.3, None, "an approval names no denying check");

    // All twelve checks are in the payload, in order, so the decision can be
    // reconstructed rather than merely believed.
    let checks = row.4["checks"].as_array().expect("check trail");
    assert_eq!(checks.len(), 12);
    // And the inputs it was made from, which is what a replay needs.
    let stored: risk_gateway::RiskSnapshot =
        serde_json::from_value(row.4["snapshot"].clone()).expect("snapshot round-trips");
    assert_eq!(stored, snapshot);
    assert_eq!(
        evaluate(&stored, &testing::limits()),
        evaluate(&snapshot, &testing::limits()),
        "a decision replayed from the stored snapshot is the same decision"
    );
}

#[tokio::test]
async fn risk_store_records_a_denial_with_the_check_that_denied_it() {
    let Some(h) = Harness::new().await else {
        return;
    };
    publish_limits(&h).await;

    let mut snapshot = testing::snapshot_all_green();
    snapshot.intent.intent_ref = "intent-denied-1".into();
    snapshot.kill_switch = risk_gateway::snapshot::KillSwitch::Engaged;

    let outcome = evaluate_and_record(&snapshot, &testing::limits(), &repo(&h)).await;
    assert!(outcome.into_approval().is_none());

    let row = sqlx::query_as::<_, (String, String, String, Option<String>)>(
        "SELECT decision, severity, reason_code, denied_by_check FROM risk_events \
         WHERE intent_ref = $1",
    )
    .bind("intent-denied-1")
    .fetch_one(&owner_view(&h).await)
    .await
    .expect("a denial is recorded, not discarded");

    assert_eq!(row.0, "DENIED");
    // §15.3: a kill-switch block is CRITICAL, not a routine rejection.
    assert_eq!(row.1, "CRITICAL");
    assert_eq!(row.2, DenyReason::LiveKillSwitchEngaged.as_str());
    assert_eq!(row.3.as_deref(), Some(Check::KillSwitch.as_str()));
}

#[tokio::test]
async fn risk_store_refuses_a_second_decision_for_one_intent() {
    let Some(h) = Harness::new().await else {
        return;
    };
    publish_limits(&h).await;

    let mut snapshot = testing::snapshot_all_green();
    snapshot.intent.intent_ref = "intent-once-only".into();

    let first = evaluate_and_record(&snapshot, &testing::limits(), &repo(&h)).await;
    assert!(first.into_approval().is_some());

    // Re-deciding the same intent must NOT yield a second approval: the
    // partial unique index refuses the row, the store reports it, and the
    // gate denies rather than minting a token for an unrecorded decision.
    let second = evaluate_and_record(&snapshot, &testing::limits(), &repo(&h)).await;
    let decision = second.decision().clone();
    assert_eq!(decision.reason, Some(DenyReason::NotPersisted));
    assert!(
        second.into_approval().is_none(),
        "an intent may be approved exactly once"
    );

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM risk_events WHERE intent_ref = $1")
        .bind("intent-once-only")
        .fetch_one(&owner_view(&h).await)
        .await
        .expect("count");
    assert_eq!(count, 1);
}

#[tokio::test]
async fn a_recorded_decision_cannot_be_edited_or_removed_by_the_app_role() {
    let Some(h) = Harness::new().await else {
        return;
    };
    publish_limits(&h).await;

    let mut snapshot = testing::snapshot_all_green();
    snapshot.intent.intent_ref = "intent-immutable".into();
    let _ = evaluate_and_record(&snapshot, &testing::limits(), &repo(&h)).await;

    // The role the API actually runs as holds no mutation grant (0018 revoked
    // what 0009 had given), so an attempt to rewrite the decision that
    // authorised an order fails rather than silently succeeding.
    for statement in [
        "UPDATE risk_events SET decision = 'DENIED' WHERE intent_ref = 'intent-immutable'",
        "DELETE FROM risk_events WHERE intent_ref = 'intent-immutable'",
    ] {
        let err = sqlx::query(sqlx::AssertSqlSafe(statement.to_string()))
            .execute(&owner_view(&h).await)
            .await
            .expect_err("a recorded risk decision must be immutable");
        let code = match &err {
            sqlx::Error::Database(e) => e.code().map(|c| c.into_owned()),
            _ => None,
        };
        assert_eq!(code.as_deref(), Some("42501"), "{statement}");
    }
}

#[tokio::test]
async fn a_decision_naming_unpublished_limits_is_refused() {
    let Some(h) = Harness::new().await else {
        return;
    };
    publish_limits(&h).await;

    // A version that was never published. The foreign key refuses the row, so
    // the gate denies -- an audit trail that pointed at limits nobody can
    // read would not explain anything.
    let limits = risk_gateway::RiskLimits::new(
        "risk-limits-never-published",
        3_000,
        testing::krw("1000000"),
        testing::krw("5000000"),
        testing::krw("500000"),
        300,
    )
    .expect("valid limit set");

    let mut snapshot = testing::snapshot_all_green();
    snapshot.intent.intent_ref = "intent-bad-limits".into();

    let outcome = evaluate_and_record(&snapshot, &limits, &repo(&h)).await;
    assert_eq!(
        outcome.decision().reason,
        Some(DenyReason::NotPersisted),
        "an unrecordable decision denies"
    );
    assert!(outcome.into_approval().is_none());
}
