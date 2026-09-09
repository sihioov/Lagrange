//! Real-role HTTP coverage for the WP4-B owner intraday quote demand API.
//!
//! These fixtures admit only immutable 0053 membership/generation rows.  The
//! HTTP requests go through the production router, app-role pool, admin-role
//! session lookup, RLS actor context, and durable 0054 repository.  No worker,
//! provider, token, cache, or producer path is started by this suite.

mod common;

use auth::entitlement::Role;
use auth::sessions::cookie;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::response::Response;
use chrono::{DateTime, Utc};
use common::{Harness, UserCtx, status};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::collections::BTreeSet;
use tower::ServiceExt;
use uuid::Uuid;

const POST_PATH: &str = "/api/v1/research/owner-beta/equity-universe-v2/quote-demands";
const CODE_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const ENTITLEMENT_SHA256: &str =
    "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

#[derive(Debug, Clone, Copy)]
struct MembershipFixture {
    membership_id: Uuid,
    generation: u64,
}

async fn seed_membership(
    harness: &Harness,
    owner: &UserCtx,
    instrument_id: &str,
    ready: bool,
) -> MembershipFixture {
    let membership_id = Uuid::new_v4();
    let generation_id = Uuid::new_v4();
    harness
        .seed_migration_owner(
            owner,
            &format!(
                "INSERT INTO owner_equity_memberships \
                 (id, owner_user_id, instrument_id, \
                  transition_actor_user_id, transition_code_commit, \
                  transition_entitlement_sha256) \
                 VALUES ('{membership_id}', '{}', '{instrument_id}', '{}', \
                         '{CODE_COMMIT}', '{ENTITLEMENT_SHA256}')",
                owner.user_id, owner.user_id
            ),
        )
        .await;

    if ready {
        for state in ["VALIDATING", "BACKFILLING"] {
            harness
                .seed_migration_owner(
                    owner,
                    &format!(
                        "UPDATE owner_equity_memberships \
                         SET state = '{state}', updated_at = now() \
                         WHERE id = '{membership_id}'"
                    ),
                )
                .await;
        }
        harness
            .seed_migration_owner(
                owner,
                &format!(
                    "INSERT INTO owner_equity_instrument_generations \
                     (id, membership_id, owner_user_id, instrument_id, generation, \
                      target_observed_sessions, minimum_observed_sessions, observed_sessions, \
                      first_session, last_session) \
                     VALUES ('{generation_id}', '{membership_id}', '{}', '{instrument_id}', 1, \
                             261, 121, 121, '2026-04-15', '2026-08-13')",
                    owner.user_id
                ),
            )
            .await;
        harness
            .seed_migration_owner(
                owner,
                &format!(
                    "INSERT INTO owner_equity_generation_admissions \
                     (generation_id, owner_user_id, membership_id, instrument_id, generation, \
                      raw_manifest_sha256, artifact_manifest_sha256, entitlement_sha256, \
                      capture_code_commit, materializer_code_commit) \
                     VALUES ('{generation_id}', '{}', '{membership_id}', '{instrument_id}', 1, \
                             'sha256:{}', 'sha256:{}', '{ENTITLEMENT_SHA256}', \
                             '{CODE_COMMIT}', '{CODE_COMMIT}')",
                    owner.user_id,
                    "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                    "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
                ),
            )
            .await;
        for state in ["MATERIALIZING", "READY"] {
            harness
                .seed_migration_owner(
                    owner,
                    &format!(
                        "UPDATE owner_equity_memberships \
                         SET state = '{state}', updated_at = now() \
                         WHERE id = '{membership_id}'"
                    ),
                )
                .await;
        }
    }

    MembershipFixture {
        membership_id,
        generation: 1,
    }
}

async fn disable_membership(harness: &Harness, owner: &UserCtx, fixture: MembershipFixture) {
    harness
        .seed_migration_owner(
            owner,
            &format!(
                "UPDATE owner_equity_memberships \
                 SET state = 'DISABLED', disabled_at = now(), updated_at = now() \
                 WHERE id = '{}'",
                fixture.membership_id
            ),
        )
        .await;
}

fn demand_body(fixture: MembershipFixture, consumer_id: Uuid, renewal_sequence: u64) -> Value {
    json!({
        "schema_version": 1,
        "consumer_id": consumer_id,
        "membership_id": fixture.membership_id,
        "generation": fixture.generation,
        "renewal_sequence": renewal_sequence,
    })
}

fn release_body(consumer_id: Uuid, renewal_sequence: u64) -> Value {
    json!({
        "schema_version": 1,
        "consumer_id": consumer_id,
        "renewal_sequence": renewal_sequence,
    })
}

async fn post_demand(
    harness: &Harness,
    owner: &UserCtx,
    fixture: MembershipFixture,
    consumer_id: Uuid,
    renewal_sequence: u64,
    key: &str,
) -> Response {
    harness
        .send(
            "POST",
            POST_PATH,
            Some(owner),
            true,
            Some("test-rid-1"),
            Some(key),
            Some(demand_body(fixture, consumer_id, renewal_sequence)),
        )
        .await
}

async fn delete_demand(
    harness: &Harness,
    owner: &UserCtx,
    demand_id: Uuid,
    consumer_id: Uuid,
    renewal_sequence: u64,
    key: &str,
) -> Response {
    harness
        .send(
            "DELETE",
            &format!("{POST_PATH}/{demand_id}"),
            Some(owner),
            true,
            Some("test-rid-1"),
            Some(key),
            Some(release_body(consumer_id, renewal_sequence)),
        )
        .await
}

fn response_fields(body: &Value) -> BTreeSet<&str> {
    body.as_object()
        .expect("demand response object")
        .keys()
        .map(String::as_str)
        .collect()
}

fn assert_demand_response_shape(body: &Value) {
    let expected = [
        "schema_version",
        "demand_id",
        "consumer_id",
        "membership_id",
        "instrument_id",
        "generation",
        "renewal_sequence",
        "lease_expires_at",
        "renew_after_ms",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    assert_eq!(response_fields(body), expected);
    assert_eq!(body["schema_version"], 1);
    assert_eq!(body["instrument_id"], "069500.KRX");
    assert_eq!(body["renew_after_ms"], 15_000);
}

fn demand_id(body: &Value) -> Uuid {
    body["demand_id"]
        .as_str()
        .expect("demand_id")
        .parse()
        .expect("demand_id UUID")
}

fn expiry(body: &Value) -> DateTime<Utc> {
    body["lease_expires_at"]
        .as_str()
        .expect("lease_expires_at")
        .parse::<DateTime<Utc>>()
        .expect("RFC3339 lease expiry")
}

async fn assert_error(resp: Response, expected_status: StatusCode, expected_code: &str) {
    assert_eq!(status(&resp), expected_status);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), expected_code);
}

async fn raw_send(
    harness: &Harness,
    method: &str,
    path: &str,
    user: Option<&UserCtx>,
    csrf: bool,
    idem: Option<&str>,
    payload: &str,
) -> Response {
    let mut builder = Request::builder()
        .method(Method::from_bytes(method.as_bytes()).expect("method"))
        .uri(path)
        .header("content-type", "application/json")
        .header("x-request-id", "raw-rid");
    if let Some(user) = user {
        builder = builder.header("cookie", format!("{}={}", cookie::NAME, user.cookie_value));
        if csrf {
            builder = builder.header("x-csrf-token", &user.csrf_token);
        }
    }
    if let Some(idem) = idem {
        builder = builder.header("idempotency-key", idem);
    }
    harness
        .app
        .clone()
        .oneshot(
            builder
                .body(Body::from(payload.to_owned()))
                .expect("raw request"),
        )
        .await
        .expect("raw request response")
}

async fn owner_scoped_values(
    pool: &PgPool,
    owner_id: Uuid,
    queries: &[&'static str],
) -> Vec<String> {
    let mut tx = pool.begin().await.expect("owner-scoped fingerprint tx");
    sqlx::query("SELECT set_config('app.actor_user_id', $1, true)")
        .bind(owner_id.to_string())
        .execute(&mut *tx)
        .await
        .expect("owner-scoped fingerprint actor");
    let mut values = Vec::with_capacity(queries.len());
    for query in queries {
        values.push(
            sqlx::query_scalar::<_, String>(*query)
                .fetch_one(&mut *tx)
                .await
                .expect("owner-scoped fingerprint query"),
        );
    }
    tx.commit().await.expect("owner-scoped fingerprint commit");
    values
}

async fn side_effect_fingerprint(harness: &Harness, owner_id: Uuid) -> Vec<String> {
    owner_scoped_values(
        &harness.owner_pool,
        owner_id,
        &[
            "SELECT COALESCE(jsonb_agg(to_jsonb(row))::text, '[]') \
               FROM (SELECT * FROM owner_intraday_quote_cache \
                     ORDER BY owner_user_id, membership_id) AS row",
            "SELECT COALESCE(jsonb_agg(to_jsonb(row))::text, '[]') \
               FROM (SELECT * FROM owner_intraday_quote_producers \
                     ORDER BY owner_user_id) AS row",
            "SELECT COALESCE(jsonb_agg(to_jsonb(row))::text, '[]') \
               FROM (SELECT * FROM jobs ORDER BY id) AS row",
            "SELECT COALESCE(jsonb_agg(to_jsonb(row))::text, '[]') \
               FROM (SELECT * FROM owner_equity_signal_snapshots \
                     ORDER BY id) AS row",
            "SELECT COALESCE(jsonb_agg(to_jsonb(row))::text, '[]') \
               FROM (SELECT * FROM owner_equity_signal_snapshot_rows \
                     ORDER BY snapshot_id, instrument_id) AS row",
        ],
    )
    .await
}

async fn state_fingerprint(harness: &Harness, owner_id: Uuid) -> Vec<String> {
    let mut values = owner_scoped_values(
        &harness.owner_pool,
        owner_id,
        &["SELECT COALESCE(jsonb_agg(to_jsonb(row))::text, '[]') \
               FROM (SELECT * FROM owner_intraday_quote_demands \
                     ORDER BY id) AS row"],
    )
    .await;
    values.extend(side_effect_fingerprint(harness, owner_id).await);
    values
}

async fn active_demand_count(harness: &Harness, owner_id: Uuid) -> i64 {
    owner_scoped_values(
        &harness.owner_pool,
        owner_id,
        &["SELECT count(*)::text FROM owner_intraday_quote_demands WHERE state = 'ACTIVE'"],
    )
    .await[0]
        .parse()
        .expect("active demand count")
}

#[tokio::test]
async fn owner_intraday_quote_demand_lifecycle_replays_and_releases_durably() {
    let mut harness = Harness::new().await.expect("DATABASE_URL is required");
    let fixture = seed_membership(&harness, &harness.owner, "069500.KRX", true).await;
    let sibling_consumer = Uuid::new_v4();
    let primary_consumer = Uuid::new_v4();
    let side_effects_before = side_effect_fingerprint(&harness, harness.owner.user_id).await;

    let first = post_demand(
        &harness,
        &harness.owner,
        fixture,
        primary_consumer,
        0,
        "primary-0",
    )
    .await;
    harness.assert_rid_echo(&first);
    assert_eq!(status(&first), StatusCode::OK);
    let first_body = Harness::body_json(first).await;
    assert_demand_response_shape(&first_body);
    assert_eq!(first_body["consumer_id"], primary_consumer.to_string());
    assert_eq!(
        first_body["membership_id"],
        fixture.membership_id.to_string()
    );
    assert_eq!(first_body["generation"], fixture.generation);
    let primary_demand_id = demand_id(&first_body);
    let first_expiry = expiry(&first_body);

    let sibling = post_demand(
        &harness,
        &harness.owner,
        fixture,
        sibling_consumer,
        0,
        "sibling-0",
    )
    .await;
    assert_eq!(status(&sibling), StatusCode::OK);
    let sibling_body = Harness::body_json(sibling).await;
    assert_demand_response_shape(&sibling_body);
    assert_ne!(demand_id(&sibling_body), primary_demand_id);
    let state_after_create = state_fingerprint(&harness, harness.owner.user_id).await;

    let replay = post_demand(
        &harness,
        &harness.owner,
        fixture,
        primary_consumer,
        0,
        "primary-0",
    )
    .await;
    assert_eq!(status(&replay), StatusCode::OK);
    assert_eq!(Harness::body_json(replay).await, first_body);

    harness.restart_api().await;
    let replay_after_restart = post_demand(
        &harness,
        &harness.owner,
        fixture,
        primary_consumer,
        0,
        "primary-0",
    )
    .await;
    assert_eq!(status(&replay_after_restart), StatusCode::OK);
    assert_eq!(Harness::body_json(replay_after_restart).await, first_body);
    assert_eq!(
        state_fingerprint(&harness, harness.owner.user_id).await,
        state_after_create,
        "durable replay must not mutate the stored demand or side-effect tables"
    );

    let renewed = post_demand(
        &harness,
        &harness.owner,
        fixture,
        primary_consumer,
        1,
        "primary-1",
    )
    .await;
    assert_eq!(status(&renewed), StatusCode::OK);
    let renewed_body = Harness::body_json(renewed).await;
    assert_demand_response_shape(&renewed_body);
    assert_eq!(renewed_body["renewal_sequence"], 1);
    assert!(expiry(&renewed_body) > first_expiry);

    let mismatch = post_demand(
        &harness,
        &harness.owner,
        fixture,
        primary_consumer,
        0,
        "primary-1",
    )
    .await;
    assert_error(mismatch, StatusCode::CONFLICT, "IDEMPOTENCY_MISMATCH").await;

    harness
        .seed_migration_owner(
            &harness.owner,
            &format!(
                "UPDATE owner_intraday_quote_demands \
                 SET lease_expires_at = created_at \
                 WHERE id = '{primary_demand_id}'"
            ),
        )
        .await;
    let expired_before_replay = state_fingerprint(&harness, harness.owner.user_id).await;
    let expired_replay = post_demand(
        &harness,
        &harness.owner,
        fixture,
        primary_consumer,
        1,
        "primary-1",
    )
    .await;
    assert_eq!(status(&expired_replay), StatusCode::OK);
    let expired_replay_body = Harness::body_json(expired_replay).await;
    assert_demand_response_shape(&expired_replay_body);
    assert_eq!(expired_replay_body["renewal_sequence"], 1);
    assert!(expiry(&expired_replay_body) < Utc::now());
    let expired_replay_again = post_demand(
        &harness,
        &harness.owner,
        fixture,
        primary_consumer,
        1,
        "primary-1",
    )
    .await;
    assert_eq!(status(&expired_replay_again), StatusCode::OK);
    assert_eq!(
        Harness::body_json(expired_replay_again).await,
        expired_replay_body,
        "expired exact replay must not extend its stored expiry"
    );
    assert_eq!(
        state_fingerprint(&harness, harness.owner.user_id).await,
        expired_before_replay,
        "expired exact replay must preserve the original expiry and row"
    );
    let recovered = post_demand(
        &harness,
        &harness.owner,
        fixture,
        primary_consumer,
        2,
        "primary-2",
    )
    .await;
    assert_eq!(status(&recovered), StatusCode::OK);
    let recovered_body = Harness::body_json(recovered).await;
    assert_demand_response_shape(&recovered_body);
    assert_eq!(recovered_body["renewal_sequence"], 2);
    assert!(expiry(&recovered_body) > expiry(&renewed_body));

    let sequence_gap = post_demand(
        &harness,
        &harness.owner,
        fixture,
        primary_consumer,
        4,
        "primary-gap",
    )
    .await;
    assert_error(
        sequence_gap,
        StatusCode::CONFLICT,
        "QUOTE_DEMAND_SEQUENCE_CONFLICT",
    )
    .await;

    let released = delete_demand(
        &harness,
        &harness.owner,
        primary_demand_id,
        primary_consumer,
        2,
        "primary-release",
    )
    .await;
    assert_eq!(status(&released), StatusCode::NO_CONTENT);
    assert!(Harness::body_text(released).await.is_empty());
    let state_after_release = state_fingerprint(&harness, harness.owner.user_id).await;

    let release_replay = delete_demand(
        &harness,
        &harness.owner,
        primary_demand_id,
        primary_consumer,
        2,
        "primary-release",
    )
    .await;
    assert_eq!(status(&release_replay), StatusCode::NO_CONTENT);
    assert!(Harness::body_text(release_replay).await.is_empty());
    assert_eq!(
        state_fingerprint(&harness, harness.owner.user_id).await,
        state_after_release,
        "release replay must not mutate the terminal tombstone"
    );

    let release_conflict = delete_demand(
        &harness,
        &harness.owner,
        primary_demand_id,
        primary_consumer,
        2,
        "primary-release-again",
    )
    .await;
    assert_error(
        release_conflict,
        StatusCode::CONFLICT,
        "QUOTE_DEMAND_SEQUENCE_CONFLICT",
    )
    .await;

    let resurrect = post_demand(
        &harness,
        &harness.owner,
        fixture,
        primary_consumer,
        3,
        "primary-after-release",
    )
    .await;
    assert_error(
        resurrect,
        StatusCode::CONFLICT,
        "QUOTE_DEMAND_SEQUENCE_CONFLICT",
    )
    .await;
    assert_eq!(
        state_fingerprint(&harness, harness.owner.user_id).await,
        state_after_release,
        "sequence/released conflicts must not mutate demand rows"
    );
    assert_eq!(
        active_demand_count(&harness, harness.owner.user_id).await,
        1
    );

    let sibling_renewed = post_demand(
        &harness,
        &harness.owner,
        fixture,
        sibling_consumer,
        1,
        "sibling-1",
    )
    .await;
    assert_eq!(status(&sibling_renewed), StatusCode::OK);
    assert_eq!(
        Harness::body_json(sibling_renewed).await["renewal_sequence"],
        1
    );
    let demand_state_summary = owner_scoped_values(
        &harness.owner_pool,
        harness.owner.user_id,
        &["SELECT format('%s:%s:%s', count(*), \
                    count(*) FILTER (WHERE state = 'ACTIVE'), \
                    count(*) FILTER (WHERE state = 'RELEASED')) \
               FROM owner_intraday_quote_demands"],
    )
    .await;
    assert_eq!(demand_state_summary, vec!["2:1:1"]);
    assert_eq!(
        side_effect_fingerprint(&harness, harness.owner.user_id).await,
        side_effects_before,
        "demand mutations must not create cache, producer, job, or EOD rows"
    );

    harness.teardown().await;
}

#[tokio::test]
async fn owner_intraday_quote_http_privacy_validation_and_role_ordering() {
    let harness = Harness::new().await.expect("DATABASE_URL is required");
    let ready = seed_membership(&harness, &harness.owner, "069500.KRX", true).await;
    let not_ready = seed_membership(&harness, &harness.owner, "229200.KRX", false).await;
    let disabled = seed_membership(&harness, &harness.owner, "114260.KRX", true).await;
    disable_membership(&harness, &harness.owner, disabled).await;

    let other_owner = harness
        .seed_user(
            Role::Owner,
            "intraday-other@lagrange.test",
            "intraday-other-iss",
            "intraday-other-sub",
        )
        .await;
    let foreign = seed_membership(&harness, &other_owner, "900001.KRX", true).await;
    let foreign_demand = post_demand(
        &harness,
        &harness.owner,
        ready,
        Uuid::new_v4(),
        0,
        "privacy-fixture-demand",
    )
    .await;
    assert_eq!(status(&foreign_demand), StatusCode::OK);
    let foreign_demand_body = Harness::body_json(foreign_demand).await;
    let foreign_demand_id = demand_id(&foreign_demand_body);
    let before_invalid_requests = state_fingerprint(&harness, harness.owner.user_id).await;

    let unauthenticated =
        raw_send(&harness, "POST", POST_PATH, None, false, None, "{not-json").await;
    assert_error(unauthenticated, StatusCode::UNAUTHORIZED, "SESSION_UNKNOWN").await;

    let csrf_first = raw_send(
        &harness,
        "POST",
        POST_PATH,
        Some(&harness.owner),
        false,
        Some("csrf-first"),
        "{not-json",
    )
    .await;
    assert_error(csrf_first, StatusCode::FORBIDDEN, "CSRF_DENIED").await;

    let member_forbidden = harness
        .send(
            "POST",
            POST_PATH,
            Some(&harness.member),
            true,
            Some("test-rid-1"),
            Some("member-forbidden"),
            Some(demand_body(ready, Uuid::new_v4(), 0)),
        )
        .await;
    assert_error(member_forbidden, StatusCode::FORBIDDEN, "FORBIDDEN").await;

    let malformed_owner = raw_send(
        &harness,
        "POST",
        POST_PATH,
        Some(&harness.owner),
        true,
        Some("malformed-owner"),
        "{not-json",
    )
    .await;
    assert_error(
        malformed_owner,
        StatusCode::BAD_REQUEST,
        "INVALID_PARAMETER",
    )
    .await;

    let malformed_delete_no_csrf = raw_send(
        &harness,
        "DELETE",
        &format!("{POST_PATH}/not-a-uuid"),
        Some(&harness.owner),
        false,
        Some("malformed-delete"),
        "{not-json",
    )
    .await;
    assert_error(
        malformed_delete_no_csrf,
        StatusCode::FORBIDDEN,
        "CSRF_DENIED",
    )
    .await;

    let malformed_path = raw_send(
        &harness,
        "DELETE",
        &format!("{POST_PATH}/not-a-uuid"),
        Some(&harness.owner),
        true,
        Some("bad-path"),
        &release_body(Uuid::new_v4(), 0).to_string(),
    )
    .await;
    assert_error(malformed_path, StatusCode::BAD_REQUEST, "INVALID_PARAMETER").await;

    let missing_key = harness
        .send(
            "POST",
            POST_PATH,
            Some(&harness.owner),
            true,
            Some("test-rid-1"),
            None,
            Some(demand_body(ready, Uuid::new_v4(), 0)),
        )
        .await;
    assert_error(missing_key, StatusCode::BAD_REQUEST, "INVALID_PARAMETER").await;

    let colon_key = harness
        .send(
            "POST",
            POST_PATH,
            Some(&harness.owner),
            true,
            Some("test-rid-1"),
            Some("bad:key"),
            Some(demand_body(ready, Uuid::new_v4(), 0)),
        )
        .await;
    assert_error(colon_key, StatusCode::BAD_REQUEST, "INVALID_PARAMETER").await;

    let whitespace_key = harness
        .send(
            "POST",
            POST_PATH,
            Some(&harness.owner),
            true,
            Some("test-rid-1"),
            Some("bad key"),
            Some(demand_body(ready, Uuid::new_v4(), 0)),
        )
        .await;
    assert_error(whitespace_key, StatusCode::BAD_REQUEST, "INVALID_PARAMETER").await;

    let long_key = "k".repeat(129);
    let long_key_response = harness
        .send(
            "POST",
            POST_PATH,
            Some(&harness.owner),
            true,
            Some("test-rid-1"),
            Some(&long_key),
            Some(demand_body(ready, Uuid::new_v4(), 0)),
        )
        .await;
    assert_error(
        long_key_response,
        StatusCode::BAD_REQUEST,
        "INVALID_PARAMETER",
    )
    .await;

    let unknown_field = harness
        .send(
            "POST",
            POST_PATH,
            Some(&harness.owner),
            true,
            Some("test-rid-1"),
            Some("unknown-field"),
            Some(json!({
                "schema_version": 1,
                "consumer_id": Uuid::new_v4(),
                "membership_id": ready.membership_id,
                "generation": 1,
                "renewal_sequence": 0,
                "unexpected": true,
            })),
        )
        .await;
    assert_error(unknown_field, StatusCode::BAD_REQUEST, "INVALID_PARAMETER").await;

    let schema_mismatch = harness
        .send(
            "POST",
            POST_PATH,
            Some(&harness.owner),
            true,
            Some("test-rid-1"),
            Some("schema-mismatch"),
            Some(json!({
                "schema_version": 2,
                "consumer_id": Uuid::new_v4(),
                "membership_id": ready.membership_id,
                "generation": 1,
                "renewal_sequence": 0,
            })),
        )
        .await;
    assert_error(
        schema_mismatch,
        StatusCode::BAD_REQUEST,
        "INVALID_PARAMETER",
    )
    .await;

    let invalid_uuid = harness
        .send(
            "POST",
            POST_PATH,
            Some(&harness.owner),
            true,
            Some("test-rid-1"),
            Some("invalid-uuid"),
            Some(json!({
                "schema_version": 1,
                "consumer_id": "not-a-uuid",
                "membership_id": ready.membership_id,
                "generation": 1,
                "renewal_sequence": 0,
            })),
        )
        .await;
    assert_error(invalid_uuid, StatusCode::BAD_REQUEST, "INVALID_PARAMETER").await;

    let not_ready_response = post_demand(
        &harness,
        &harness.owner,
        not_ready,
        Uuid::new_v4(),
        0,
        "not-ready",
    )
    .await;
    assert_error(
        not_ready_response,
        StatusCode::NOT_FOUND,
        "RESOURCE_NOT_FOUND",
    )
    .await;

    let disabled_response = post_demand(
        &harness,
        &harness.owner,
        disabled,
        Uuid::new_v4(),
        0,
        "disabled",
    )
    .await;
    assert_error(
        disabled_response,
        StatusCode::NOT_FOUND,
        "RESOURCE_NOT_FOUND",
    )
    .await;

    let stale_generation = harness
        .send(
            "POST",
            POST_PATH,
            Some(&harness.owner),
            true,
            Some("test-rid-1"),
            Some("stale-generation"),
            Some(json!({
                "schema_version": 1,
                "consumer_id": Uuid::new_v4(),
                "membership_id": ready.membership_id,
                "generation": 2,
                "renewal_sequence": 0,
            })),
        )
        .await;
    assert_error(
        stale_generation,
        StatusCode::NOT_FOUND,
        "RESOURCE_NOT_FOUND",
    )
    .await;

    let unknown_membership = harness
        .send(
            "POST",
            POST_PATH,
            Some(&harness.owner),
            true,
            Some("test-rid-1"),
            Some("unknown-membership"),
            Some(demand_body(
                MembershipFixture {
                    membership_id: Uuid::new_v4(),
                    generation: 1,
                },
                Uuid::new_v4(),
                0,
            )),
        )
        .await;
    assert_error(
        unknown_membership,
        StatusCode::NOT_FOUND,
        "RESOURCE_NOT_FOUND",
    )
    .await;

    let foreign_membership = harness
        .send(
            "POST",
            POST_PATH,
            Some(&harness.owner),
            true,
            Some("test-rid-1"),
            Some("foreign-membership"),
            Some(demand_body(foreign, Uuid::new_v4(), 0)),
        )
        .await;
    assert_error(
        foreign_membership,
        StatusCode::NOT_FOUND,
        "RESOURCE_NOT_FOUND",
    )
    .await;

    let wrong_owner_post = harness
        .send(
            "POST",
            POST_PATH,
            Some(&other_owner),
            true,
            Some("test-rid-1"),
            Some("wrong-owner-post"),
            Some(demand_body(ready, Uuid::new_v4(), 0)),
        )
        .await;
    assert_error(
        wrong_owner_post,
        StatusCode::NOT_FOUND,
        "RESOURCE_NOT_FOUND",
    )
    .await;

    let wrong_owner_delete = delete_demand(
        &harness,
        &other_owner,
        foreign_demand_id,
        Uuid::new_v4(),
        0,
        "wrong-owner-delete",
    )
    .await;
    assert_error(
        wrong_owner_delete,
        StatusCode::NOT_FOUND,
        "RESOURCE_NOT_FOUND",
    )
    .await;

    let unknown_demand_delete = delete_demand(
        &harness,
        &harness.owner,
        Uuid::new_v4(),
        Uuid::new_v4(),
        0,
        "unknown-demand-delete",
    )
    .await;
    assert_error(
        unknown_demand_delete,
        StatusCode::NOT_FOUND,
        "RESOURCE_NOT_FOUND",
    )
    .await;

    assert_eq!(
        state_fingerprint(&harness, harness.owner.user_id).await,
        before_invalid_requests,
        "invalid/private HTTP requests must not mutate demand, cache, producer, job, or EOD state"
    );

    harness.teardown().await;
}

#[tokio::test]
async fn owner_intraday_quote_demand_enforces_consumer_and_identity_capacity() {
    let harness = Harness::new().await.expect("DATABASE_URL is required");
    let fixture = seed_membership(&harness, &harness.owner, "069500.KRX", true).await;
    let mut first_demand_id = None;
    for index in 0..20_u128 {
        let consumer_id = Uuid::from_u128(0x1000 + index);
        let response = post_demand(
            &harness,
            &harness.owner,
            fixture,
            consumer_id,
            0,
            &format!("consumer-{index}"),
        )
        .await;
        assert_eq!(status(&response), StatusCode::OK, "consumer {index}");
        let body = Harness::body_json(response).await;
        assert_demand_response_shape(&body);
        if index == 0 {
            first_demand_id = Some(demand_id(&body));
        }
    }
    assert_eq!(
        active_demand_count(&harness, harness.owner.user_id).await,
        20
    );

    let before_consumer_capacity = state_fingerprint(&harness, harness.owner.user_id).await;
    let over_consumer_limit = post_demand(
        &harness,
        &harness.owner,
        fixture,
        Uuid::from_u128(0x2000),
        0,
        "consumer-over-limit",
    )
    .await;
    assert_eq!(status(&over_consumer_limit), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        over_consumer_limit
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok()),
        Some("15")
    );
    assert_error(
        over_consumer_limit,
        StatusCode::TOO_MANY_REQUESTS,
        "QUOTE_DEMAND_CAPACITY",
    )
    .await;
    assert_eq!(
        state_fingerprint(&harness, harness.owner.user_id).await,
        before_consumer_capacity,
        "consumer capacity rejection must not mutate any state"
    );

    let first_demand_id = first_demand_id.expect("first demand id");
    let release = delete_demand(
        &harness,
        &harness.owner,
        first_demand_id,
        Uuid::from_u128(0x1000),
        0,
        "consumer-0-release",
    )
    .await;
    assert_eq!(status(&release), StatusCode::NO_CONTENT);
    assert!(Harness::body_text(release).await.is_empty());

    let admitted_after_release = post_demand(
        &harness,
        &harness.owner,
        fixture,
        Uuid::from_u128(0x2000),
        0,
        "consumer-over-limit",
    )
    .await;
    assert_eq!(status(&admitted_after_release), StatusCode::OK);
    assert_eq!(
        active_demand_count(&harness, harness.owner.user_id).await,
        20
    );

    harness.teardown().await;

    let harness = Harness::new().await.expect("DATABASE_URL is required");
    let mut memberships = Vec::new();
    for index in 0..6 {
        memberships.push(
            seed_membership(
                &harness,
                &harness.owner,
                &format!("{:06}.KRX", 910_000 + index),
                true,
            )
            .await,
        );
    }
    for (index, membership) in memberships.iter().take(5).enumerate() {
        let response = post_demand(
            &harness,
            &harness.owner,
            *membership,
            Uuid::from_u128(0x3000 + index as u128),
            0,
            &format!("identity-{index}"),
        )
        .await;
        assert_eq!(status(&response), StatusCode::OK, "identity {index}");
        let body = Harness::body_json(response).await;
        assert_eq!(body["instrument_id"], format!("{:06}.KRX", 910_000 + index));
    }
    assert_eq!(
        active_demand_count(&harness, harness.owner.user_id).await,
        5
    );

    let before_identity_capacity = state_fingerprint(&harness, harness.owner.user_id).await;
    let over_identity_limit = post_demand(
        &harness,
        &harness.owner,
        memberships[5],
        Uuid::from_u128(0x4000),
        0,
        "identity-over-limit",
    )
    .await;
    assert_eq!(status(&over_identity_limit), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        over_identity_limit
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok()),
        Some("15")
    );
    assert_error(
        over_identity_limit,
        StatusCode::TOO_MANY_REQUESTS,
        "QUOTE_DEMAND_CAPACITY",
    )
    .await;
    assert_eq!(
        state_fingerprint(&harness, harness.owner.user_id).await,
        before_identity_capacity,
        "identity capacity rejection must not mutate any state"
    );

    let duplicate_identity_at_limit = post_demand(
        &harness,
        &harness.owner,
        memberships[0],
        Uuid::from_u128(0x4001),
        0,
        "identity-duplicate-at-limit",
    )
    .await;
    assert_eq!(status(&duplicate_identity_at_limit), StatusCode::OK);
    assert_eq!(
        Harness::body_json(duplicate_identity_at_limit).await["instrument_id"],
        "910000.KRX"
    );
    assert_eq!(
        active_demand_count(&harness, harness.owner.user_id).await,
        6
    );

    harness.teardown().await;
}
