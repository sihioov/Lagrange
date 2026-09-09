//! Real-role HTTP coverage for the WP4-C3 owner intraday quote cache GET.
//!
//! The fixture uses only synthetic database rows in a disposable harness DB.
//! The request goes through the production session extractor, app-role RLS,
//! the accepted read seams, and the production projection helper. No provider
//! or worker is started by this suite.

mod common;

use api_server::http::state::OwnerIntradayQuoteReadConfig;
use auth::entitlement::Role;
use axum::http::StatusCode;
use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use collectors::intraday_quotes::IntradaySessionWindowContract;
use common::{Harness, UserCtx, status};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

const GET_PATH_PREFIX: &str =
    "/api/v1/research/owner-beta/equity-universe-v2/instruments/069500.KRX/quote";
const INSTRUMENT: &str = "069500.KRX";
const CODE_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const ENTITLEMENT_SHA256: &str =
    "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
const CALENDAR_HASH: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

#[derive(Debug, Clone, Copy)]
struct MembershipFixture {
    membership_id: Uuid,
    generation_id: Uuid,
    generation: u64,
}

fn test_clock() -> DateTime<Utc> {
    Utc::now()
}

fn current_kst_date(now: DateTime<Utc>) -> NaiveDate {
    now.with_timezone(&FixedOffset::east_opt(9 * 60 * 60).expect("KST offset"))
        .date_naive()
}

fn window_for(date: NaiveDate) -> Arc<IntradaySessionWindowContract> {
    let bytes = serde_json::to_vec(&json!({
        "schema_version": 1,
        "exchange": "KRX",
        "timezone": "Asia/Seoul",
        "entries": [{
            "date": date.to_string(),
            "disposition": "REGULAR",
            "open_local": "09:00:00",
            "close_local": "15:30:00",
            "evidence_url": "https://global.krx.co.kr/contents/test",
            "evidence_retrieved_at": format!("{date}T00:00:00+09:00"),
            "evidence_sha256": format!("sha256:{}", "b".repeat(64)),
        }],
    }))
    .expect("window fixture serializes");
    let hash = format!("sha256:{:x}", Sha256::digest(&bytes));
    Arc::new(IntradaySessionWindowContract::from_bytes(&bytes, &hash).unwrap())
}

async fn seed_ready_membership(
    harness: &Harness,
    owner: &UserCtx,
    instrument_id: &str,
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
    MembershipFixture {
        membership_id,
        generation_id,
        generation: 1,
    }
}

async fn seed_calendar_and_cache(
    harness: &Harness,
    owner: &UserCtx,
    fixture: MembershipFixture,
    window: &IntradaySessionWindowContract,
    session_date: NaiveDate,
) {
    let batch_id = Uuid::new_v4();
    harness
        .seed_shared(&format!(
            "INSERT INTO data_batches \
             (id, provider, market, batch_date, kind, storage_path, content_sha256, \
              bytes_size, retrieved_at) \
             VALUES ('{batch_id}', 'KIS', 'KR', '{session_date}', 'CALENDAR', \
                     'synthetic/wp4-c3-calendar', '{CALENDAR_HASH}', 1, clock_timestamp())"
        ))
        .await;
    harness
        .seed_shared(&format!(
            "INSERT INTO trading_calendar_versions \
             (exchange, session_date, session_type, timezone, source, source_version, \
              source_batch_id, content_sha256, retrieved_at) \
             VALUES ('KRX', '{session_date}', 'TRADING', 'Asia/Seoul', 'kis', \
                     'kis-chk-holiday-v1:schema-1', '{batch_id}', '{CALENDAR_HASH}', \
                     clock_timestamp())"
        ))
        .await;
    harness
        .seed_shared(&format!(
            "INSERT INTO trading_calendars \
             (exchange, session_date, session_type, timezone, source, source_version, \
              source_batch_id, content_sha256, retrieved_at) \
             VALUES ('KRX', '{session_date}', 'TRADING', 'Asia/Seoul', 'kis', \
                     'kis-chk-holiday-v1:schema-1', '{batch_id}', '{CALENDAR_HASH}', \
                     clock_timestamp())"
        ))
        .await;
    let snapshot_id = Uuid::new_v4();
    let universe_sha256 = format!("sha256:{}", common::sha256_hex(INSTRUMENT.as_bytes()));
    harness
        .seed_migration_owner(
            owner,
            &format!(
                "INSERT INTO owner_equity_signal_snapshots \
                 (id, owner_user_id, as_of_session, universe_sha256, row_count, \
                  signal_code_commit) \
                 VALUES ('{snapshot_id}', '{}', '{session_date}', '{universe_sha256}', 1, \
                         '{CODE_COMMIT}')",
                owner.user_id,
            ),
        )
        .await;
    harness
        .seed_migration_owner(
            owner,
            &format!(
                "INSERT INTO owner_equity_signal_snapshot_rows \
                 (snapshot_id, owner_user_id, instrument_id, membership_id, generation_id, \
                  generation, rank, signals_json) \
                 VALUES ('{snapshot_id}', '{}', '{INSTRUMENT}', '{}', '{}', 1, 1, \
                         '{{\"score\":\"synthetic\"}}'::jsonb)",
                owner.user_id, fixture.membership_id, fixture.generation_id
            ),
        )
        .await;
    harness
        .seed_migration_owner(
            owner,
            &format!(
                "UPDATE owner_equity_signal_snapshots SET published_at = clock_timestamp() \
                 WHERE id = '{snapshot_id}'"
            ),
        )
        .await;
    harness
        .seed_migration_owner(
            owner,
            &format!(
                "INSERT INTO owner_intraday_quote_producers \
                 (owner_user_id, holder_id, fencing_token, lease_expires_at, heartbeat_at) \
                 VALUES ('{}', '{}', 1, clock_timestamp() + interval '20 seconds', \
                         clock_timestamp())",
                owner.user_id,
                Uuid::new_v4()
            ),
        )
        .await;
    harness
        .seed_migration_owner(
            owner,
            &format!(
                "INSERT INTO owner_intraday_quote_cache \
                 (owner_user_id, membership_id, generation_id, instrument_id, generation, \
                  session_date, calendar_source, calendar_source_version, \
                  calendar_source_batch_id, calendar_content_sha256, window_contract_sha256, \
                  price, base_price, change_amount, change_percent, direction, halted, \
                  received_at, last_success_at, quote_version, last_attempt_at, producer_fence) \
                 VALUES ('{}', '{}', '{}', '{INSTRUMENT}', 1, '{session_date}', 'kis', \
                         'kis-chk-holiday-v1:schema-1', '{batch_id}', '{CALENDAR_HASH}', \
                         '{}', 100.25, 99.00, 1.25, 1.26, 'UP', false, \
                         now() - interval '10 seconds', \
                         now() - interval '10 seconds', 1, \
                         now() - interval '1 second', 1)",
                owner.user_id,
                fixture.membership_id,
                fixture.generation_id,
                window.window_contract_sha256()
            ),
        )
        .await;
}

fn demand_body(fixture: MembershipFixture) -> Value {
    json!({
        "schema_version": 1,
        "consumer_id": Uuid::new_v4(),
        "membership_id": fixture.membership_id,
        "generation": fixture.generation,
        "renewal_sequence": 0,
    })
}

async fn assert_error(resp: axum::response::Response, status_code: StatusCode, code: &str) {
    assert_eq!(status(&resp), status_code);
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), code);
}

async fn fingerprint(harness: &Harness, owner_id: Uuid) -> Vec<String> {
    let queries = [
        "SELECT COALESCE(jsonb_agg(to_jsonb(row))::text, '[]') FROM (SELECT * FROM owner_intraday_quote_demands ORDER BY id) AS row",
        "SELECT COALESCE(jsonb_agg(to_jsonb(row))::text, '[]') FROM (SELECT * FROM owner_intraday_quote_cache ORDER BY owner_user_id, membership_id) AS row",
        "SELECT COALESCE(jsonb_agg(to_jsonb(row))::text, '[]') FROM (SELECT * FROM owner_intraday_quote_producers ORDER BY owner_user_id) AS row",
        "SELECT COALESCE(jsonb_agg(to_jsonb(row))::text, '[]') FROM (SELECT * FROM owner_equity_memberships ORDER BY id) AS row",
        "SELECT COALESCE(jsonb_agg(to_jsonb(row))::text, '[]') FROM (SELECT * FROM owner_equity_instrument_generations ORDER BY id) AS row",
        "SELECT COALESCE(jsonb_agg(to_jsonb(row))::text, '[]') FROM (SELECT * FROM owner_equity_generation_admissions ORDER BY generation_id) AS row",
        "SELECT COALESCE(jsonb_agg(to_jsonb(row))::text, '[]') FROM (SELECT * FROM trading_calendars ORDER BY id) AS row",
        "SELECT COALESCE(jsonb_agg(to_jsonb(row))::text, '[]') FROM (SELECT * FROM trading_calendar_versions ORDER BY id) AS row",
        "SELECT COALESCE(jsonb_agg(to_jsonb(row))::text, '[]') FROM (SELECT * FROM data_batches ORDER BY id) AS row",
        "SELECT COALESCE(jsonb_agg(to_jsonb(row))::text, '[]') FROM (SELECT * FROM jobs ORDER BY id) AS row",
        "SELECT COALESCE(jsonb_agg(to_jsonb(row))::text, '[]') FROM (SELECT * FROM owner_equity_signal_snapshots ORDER BY id) AS row",
        "SELECT COALESCE(jsonb_agg(to_jsonb(row))::text, '[]') FROM (SELECT * FROM owner_equity_signal_snapshot_rows ORDER BY snapshot_id, instrument_id) AS row",
        "SELECT COALESCE(jsonb_agg(to_jsonb(row))::text, '[]') FROM (SELECT * FROM dataset_versions WHERE dataset_id = 'krx_eod_bars' ORDER BY id) AS row",
    ];
    let mut tx = harness.owner_pool.begin().await.expect("fingerprint tx");
    sqlx::query("SELECT set_config('app.actor_user_id', $1, true)")
        .bind(owner_id.to_string())
        .execute(&mut *tx)
        .await
        .expect("fingerprint actor");
    let mut values = Vec::with_capacity(queries.len());
    for query in queries {
        values.push(
            sqlx::query_scalar::<_, String>(query)
                .fetch_one(&mut *tx)
                .await
                .expect("fingerprint query"),
        );
    }
    tx.commit().await.expect("fingerprint commit");
    values
}

async fn assert_app_role_and_producer_is_forbidden(harness: &Harness) {
    let role: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&harness.app_pool)
        .await
        .expect("app current_user");
    assert_eq!(role, "app");
    let error = sqlx::query("SELECT count(*) FROM owner_intraday_quote_producers")
        .fetch_one(&harness.app_pool)
        .await
        .expect_err("app must not be able to read producer leases");
    assert_eq!(
        error
            .as_database_error()
            .and_then(|database| database.code())
            .as_deref(),
        Some("42501")
    );
}

#[tokio::test]
async fn owner_intraday_quote_cache_get_is_read_only_and_owner_scoped() {
    let mut harness = Harness::new().await.expect("DATABASE_URL is required");
    let owner_fixture = seed_ready_membership(&harness, &harness.owner, INSTRUMENT).await;
    let other_owner = harness
        .seed_user(
            Role::Owner,
            "intraday-cache-other@lagrange.test",
            "intraday-cache-other-iss",
            "intraday-cache-other-sub",
        )
        .await;
    let foreign_fixture = seed_ready_membership(&harness, &other_owner, INSTRUMENT).await;
    let db_date: NaiveDate =
        sqlx::query_scalar("SELECT (clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date")
            .fetch_one(&harness.owner_pool)
            .await
            .expect("database KST date");
    let api_date = current_kst_date(test_clock());
    assert_eq!(
        db_date, api_date,
        "fixture must use the exact current DB KST date"
    );
    let window = window_for(db_date);
    seed_calendar_and_cache(&harness, &harness.owner, owner_fixture, &window, db_date).await;

    harness
        .restart_api_with_intraday_read_config(
            OwnerIntradayQuoteReadConfig::OwnerOnly {
                window: Some(window.clone()),
            },
            test_clock,
        )
        .await;
    let demand = harness
        .send(
            "POST",
            "/api/v1/research/owner-beta/equity-universe-v2/quote-demands",
            Some(&harness.owner),
            true,
            Some("cache-demand-rid"),
            Some("cache-demand-key"),
            Some(demand_body(owner_fixture)),
        )
        .await;
    assert_eq!(status(&demand), StatusCode::OK);

    assert_app_role_and_producer_is_forbidden(&harness).await;
    let before = fingerprint(&harness, harness.owner.user_id).await;

    let path = format!(
        "{GET_PATH_PREFIX}?membership_id={}&generation=1",
        owner_fixture.membership_id
    );
    let first = harness.get(&path, Some(&harness.owner)).await;
    assert_eq!(status(&first), StatusCode::OK);
    assert_eq!(first.headers().get("cache-control").unwrap(), "no-store");
    let body = Harness::body_json(first).await;
    let fields = body
        .as_object()
        .expect("quote DTO")
        .keys()
        .collect::<Vec<_>>();
    assert_eq!(
        fields
            .into_iter()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>(),
        [
            "schema_version",
            "membership_id",
            "instrument_id",
            "venue",
            "currency",
            "generation",
            "session",
            "market_state",
            "freshness",
            "reason_code",
            "quote",
            "next_poll_after_ms",
        ]
        .into_iter()
        .collect()
    );
    assert_eq!(
        body["membership_id"],
        owner_fixture.membership_id.to_string()
    );
    assert_eq!(body["instrument_id"], INSTRUMENT);
    assert_eq!(body["venue"], "KRX");
    assert_eq!(body["currency"], "KRW");
    assert_eq!(body["generation"], 1);
    assert_eq!(body["next_poll_after_ms"], 5000);
    assert_eq!(body["session"]["timezone"], "Asia/Seoul");
    assert_eq!(body["session"]["calendar_source"], "kis");
    assert_eq!(
        body["session"]["calendar_source_version"],
        "kis-chk-holiday-v1:schema-1"
    );
    assert_eq!(body["session"]["calendar_content_sha256"], CALENDAR_HASH);
    assert_eq!(
        body["session"]["window_contract_sha256"],
        window.window_contract_sha256()
    );
    assert_eq!(body["quote"]["price"], "100.25000000");
    assert_eq!(body["quote"]["base_price"], "99.00000000");
    assert_eq!(body["quote"]["change_from_previous_day"], "1.25000000");
    assert_eq!(
        body["quote"]["change_percent_from_previous_day"],
        "1.26000000"
    );
    assert_eq!(body["quote"]["direction"], "UP");
    assert_eq!(body["quote"]["quote_version"], "1");
    assert!(body["owner"].is_null() && body["generation_id"].is_null());

    let second = harness.get(&path, Some(&harness.owner)).await;
    assert_eq!(status(&second), StatusCode::OK);
    assert_eq!(second.headers().get("cache-control").unwrap(), "no-store");
    assert_eq!(Harness::body_json(second).await, body);

    let unauthenticated = harness.get(&path, None).await;
    assert_error(unauthenticated, StatusCode::UNAUTHORIZED, "SESSION_UNKNOWN").await;
    let expired = harness
        .seed_user(
            Role::Owner,
            "intraday-cache-expired@lagrange.test",
            "intraday-cache-expired-iss",
            "intraday-cache-expired-sub",
        )
        .await;
    harness
        .seed_migration_owner(
            &expired,
            &format!(
                "UPDATE web_sessions SET created_at = now() - interval '2 seconds', \
                 expires_at = now() - interval '1 second' \
                 WHERE user_id = '{}'",
                expired.user_id
            ),
        )
        .await;
    let expired_response = harness.get(&path, Some(&expired)).await;
    assert_error(
        expired_response,
        StatusCode::UNAUTHORIZED,
        "SESSION_EXPIRED",
    )
    .await;
    let member_invalid = harness
        .get(
            "/api/v1/research/owner-beta/equity-universe-v2/instruments/not-an-instrument/quote?unknown=1",
            Some(&harness.member),
        )
        .await;
    assert_error(member_invalid, StatusCode::FORBIDDEN, "FORBIDDEN").await;
    let owner_invalid = harness
        .get(
            &format!(
                "{GET_PATH_PREFIX}?membership_id={}&generation=0",
                owner_fixture.membership_id
            ),
            Some(&harness.owner),
        )
        .await;
    assert_error(owner_invalid, StatusCode::BAD_REQUEST, "INVALID_PARAMETER").await;
    let owner_mismatched_instrument = harness
        .get(
            &format!(
                "/api/v1/research/owner-beta/equity-universe-v2/instruments/229200.KRX/quote?membership_id={}&generation=1",
                owner_fixture.membership_id
            ),
            Some(&harness.owner),
        )
        .await;
    assert_error(
        owner_mismatched_instrument,
        StatusCode::NOT_FOUND,
        "RESOURCE_NOT_FOUND",
    )
    .await;
    let owner_mismatched_generation = harness
        .get(
            &format!(
                "{GET_PATH_PREFIX}?membership_id={}&generation=2",
                owner_fixture.membership_id
            ),
            Some(&harness.owner),
        )
        .await;
    assert_error(
        owner_mismatched_generation,
        StatusCode::NOT_FOUND,
        "RESOURCE_NOT_FOUND",
    )
    .await;
    let foreign = harness
        .get(
            &format!(
                "{GET_PATH_PREFIX}?membership_id={}&generation=1",
                foreign_fixture.membership_id
            ),
            Some(&harness.owner),
        )
        .await;
    assert_error(foreign, StatusCode::NOT_FOUND, "RESOURCE_NOT_FOUND").await;
    let missing = harness
        .get(
            &format!(
                "{GET_PATH_PREFIX}?membership_id={}&generation=1",
                Uuid::new_v4()
            ),
            Some(&harness.owner),
        )
        .await;
    assert_error(missing, StatusCode::NOT_FOUND, "RESOURCE_NOT_FOUND").await;
    assert_eq!(fingerprint(&harness, harness.owner.user_id).await, before);

    harness
        .restart_api_with_intraday_read_config(OwnerIntradayQuoteReadConfig::Disabled, test_clock)
        .await;
    let disabled = harness.get(&path, Some(&harness.owner)).await;
    assert_eq!(status(&disabled), StatusCode::OK);
    assert_eq!(disabled.headers().get("cache-control").unwrap(), "no-store");
    let disabled_body = Harness::body_json(disabled).await;
    assert_eq!(disabled_body["market_state"], "UNKNOWN");
    assert_eq!(disabled_body["freshness"], "UNAVAILABLE");
    assert_eq!(disabled_body["reason_code"], "FEATURE_DISABLED");
    assert!(disabled_body["session"].is_null() && disabled_body["quote"].is_null());
    assert_eq!(fingerprint(&harness, harness.owner.user_id).await, before);

    harness
        .seed_migration_owner(
            &harness.owner,
            &format!(
                "UPDATE owner_equity_memberships SET state = 'DISABLED', disabled_at = now(), \
                 updated_at = now() WHERE id = '{}'",
                owner_fixture.membership_id
            ),
        )
        .await;
    harness
        .restart_api_with_intraday_read_config(
            OwnerIntradayQuoteReadConfig::OwnerOnly {
                window: Some(window),
            },
            test_clock,
        )
        .await;
    let disabled_identity = harness.get(&path, Some(&harness.owner)).await;
    assert_error(
        disabled_identity,
        StatusCode::NOT_FOUND,
        "RESOURCE_NOT_FOUND",
    )
    .await;
    harness.teardown().await;
}
