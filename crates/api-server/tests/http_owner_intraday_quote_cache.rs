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
use chrono::{DateTime, Duration, FixedOffset, NaiveDate, TimeZone, Utc};
use collectors::intraday_quotes::{IntradayMarketState, IntradaySessionWindowContract};
use common::{Harness, UserCtx, status};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::any::Any;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context, Poll};
use uuid::Uuid;

const GET_PATH_PREFIX: &str =
    "/api/v1/research/owner-beta/equity-universe-v2/instruments/069500.KRX/quote";
const INSTRUMENT: &str = "069500.KRX";
const CODE_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const ENTITLEMENT_SHA256: &str =
    "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
const CALENDAR_HASH: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

static API_CLOCK: OnceLock<Mutex<DateTime<Utc>>> = OnceLock::new();

#[derive(Debug, Clone, Copy)]
struct MembershipFixture {
    membership_id: Uuid,
    generation_id: Uuid,
    generation: u64,
}

#[derive(Clone)]
struct FixtureTimeBundle {
    db_now: DateTime<Utc>,
    session_date: NaiveDate,
    evidence_at: DateTime<Utc>,
    api_initial: DateTime<Utc>,
    api_after_stale: DateTime<Utc>,
    receipt_at: DateTime<Utc>,
    attempt_at: DateTime<Utc>,
    failure_at: DateTime<Utc>,
    window: Arc<IntradaySessionWindowContract>,
}

fn set_api_clock(now: DateTime<Utc>) {
    *API_CLOCK
        .get_or_init(|| Mutex::new(now))
        .lock()
        .expect("API clock mutex") = now;
}

fn injected_api_clock() -> DateTime<Utc> {
    *API_CLOCK
        .get()
        .expect("API clock initialized before router restart")
        .lock()
        .expect("API clock mutex")
}

fn current_kst_date(now: DateTime<Utc>) -> NaiveDate {
    now.with_timezone(&FixedOffset::east_opt(9 * 60 * 60).expect("KST offset"))
        .date_naive()
}

fn window_for_kind(
    date: NaiveDate,
    disposition: &str,
    open_local: Option<&str>,
    close_local: Option<&str>,
    evidence_at: DateTime<Utc>,
) -> Arc<IntradaySessionWindowContract> {
    let bytes = serde_json::to_vec(&json!({
        "schema_version": 1,
        "exchange": "KRX",
        "timezone": "Asia/Seoul",
        "entries": [{
            "date": date.to_string(),
            "disposition": disposition,
            "open_local": open_local,
            "close_local": close_local,
            "evidence_url": "https://global.krx.co.kr/contents/test",
            "evidence_retrieved_at": evidence_at.to_rfc3339(),
            "evidence_sha256": format!("sha256:{}", "b".repeat(64)),
        }],
    }))
    .expect("window fixture serializes");
    let hash = format!("sha256:{:x}", Sha256::digest(&bytes));
    Arc::new(IntradaySessionWindowContract::from_bytes(&bytes, &hash).unwrap())
}

fn kst_instant(date: NaiveDate, hour: u32, minute: u32, second: u32) -> DateTime<Utc> {
    FixedOffset::east_opt(9 * 60 * 60)
        .expect("KST offset")
        .from_local_datetime(
            &date
                .and_hms_opt(hour, minute, second)
                .expect("valid KST fixture time"),
        )
        .single()
        .expect("unambiguous KST time")
        .with_timezone(&Utc)
}

fn fixture_time_bundle(db_now: DateTime<Utc>) -> FixtureTimeBundle {
    let session_date = current_kst_date(db_now);
    let evidence_at = kst_instant(session_date, 0, 0, 0);
    // Preserve enough same-date room to advance only the injected API clock
    // beyond the 30-second freshness boundary, even if the DB is read near
    // KST midnight.
    let latest_initial = kst_instant(session_date, 23, 59, 20);
    let api_initial = std::cmp::min(db_now, latest_initial);
    let api_after_stale = api_initial + Duration::seconds(31);
    let window = window_for_kind(
        session_date,
        "SPECIAL",
        Some("00:00:00"),
        Some("23:59:59"),
        evidence_at,
    );
    let bundle = FixtureTimeBundle {
        db_now,
        session_date,
        evidence_at,
        api_initial,
        api_after_stale,
        receipt_at: api_initial,
        attempt_at: api_initial,
        failure_at: api_initial,
        window,
    };
    assert_fixture_time_bundle(&bundle);
    bundle
}

fn assert_fixture_time_bundle(bundle: &FixtureTimeBundle) {
    assert_eq!(current_kst_date(bundle.db_now), bundle.session_date);
    assert_eq!(current_kst_date(bundle.evidence_at), bundle.session_date);
    assert_eq!(current_kst_date(bundle.api_initial), bundle.session_date);
    assert_eq!(
        current_kst_date(bundle.api_after_stale),
        bundle.session_date
    );
    assert!(bundle.api_initial <= bundle.db_now);
    for (name, instant) in [
        ("evidence", bundle.evidence_at),
        ("receipt", bundle.receipt_at),
        ("attempt", bundle.attempt_at),
        ("failure", bundle.failure_at),
    ] {
        assert_eq!(
            current_kst_date(instant),
            bundle.session_date,
            "{name} date"
        );
        assert!(instant <= bundle.db_now, "{name} must not be future to DB");
        assert!(
            instant <= bundle.api_initial,
            "{name} must not be future to initial API clock"
        );
    }
    let initial_age = bundle.api_initial - bundle.receipt_at;
    assert!(initial_age >= Duration::zero() && initial_age <= Duration::seconds(30));
    assert!(bundle.api_after_stale - bundle.receipt_at > Duration::seconds(30));
    assert_eq!(
        bundle.window.state_at(bundle.api_initial, false),
        IntradayMarketState::Open
    );
    assert_eq!(
        bundle.window.state_at(bundle.api_after_stale, false),
        IntradayMarketState::Open
    );
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

async fn seed_calendar_rows(
    harness: &Harness,
    times: &FixtureTimeBundle,
    disposition: &str,
) -> Uuid {
    let batch_id = Uuid::new_v4();
    let session_date = times.session_date;
    let retrieved_at = times.db_now.to_rfc3339();
    harness
        .seed_shared(&format!(
            "INSERT INTO data_batches \
             (id, provider, market, batch_date, kind, storage_path, content_sha256, \
              bytes_size, retrieved_at) \
             VALUES ('{batch_id}', 'KIS', 'KR', '{session_date}', 'CALENDAR', \
                     'synthetic/wp4-c3-calendar', '{CALENDAR_HASH}', 1, \
                     '{retrieved_at}'::timestamptz)"
        ))
        .await;
    harness
        .seed_shared(&format!(
            "INSERT INTO trading_calendar_versions \
             (exchange, session_date, session_type, timezone, source, source_version, \
              source_batch_id, content_sha256, retrieved_at) \
             VALUES ('KRX', '{session_date}', '{disposition}', 'Asia/Seoul', 'kis', \
                     'kis-chk-holiday-v1:schema-1', '{batch_id}', '{CALENDAR_HASH}', \
                     '{retrieved_at}'::timestamptz)"
        ))
        .await;
    harness
        .seed_shared(&format!(
            "INSERT INTO trading_calendars \
             (exchange, session_date, session_type, timezone, source, source_version, \
              source_batch_id, content_sha256, retrieved_at) \
             VALUES ('KRX', '{session_date}', '{disposition}', 'Asia/Seoul', 'kis', \
                     'kis-chk-holiday-v1:schema-1', '{batch_id}', '{CALENDAR_HASH}', \
                     '{retrieved_at}'::timestamptz)"
        ))
        .await;
    batch_id
}

async fn seed_calendar_and_cache(
    harness: &Harness,
    owner: &UserCtx,
    fixture: MembershipFixture,
    times: &FixtureTimeBundle,
    calendar_disposition: &str,
) {
    let batch_id = seed_calendar_rows(harness, times, calendar_disposition).await;
    let session_date = times.session_date;
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
    let receipt_at = times.receipt_at.to_rfc3339();
    let attempt_at = times.attempt_at.to_rfc3339();
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
                         '{receipt_at}'::timestamptz, '{receipt_at}'::timestamptz, 1, \
                         '{attempt_at}'::timestamptz, 1)",
                owner.user_id,
                fixture.membership_id,
                fixture.generation_id,
                times.window.window_contract_sha256()
            ),
        )
        .await;
}

async fn seed_queue_row(harness: &Harness, owner: &UserCtx) {
    harness
        .seed_migration_owner(
            owner,
            &format!(
                "INSERT INTO jobs \
             (owner_user_id, job_type, status, idempotency_key, payload_json) \
             VALUES ('{}', 'wp4_c3_fingerprint', 'QUEUED', \
                     'wp4-c3-fingerprint', '{{}}'::jsonb)",
                owner.user_id
            ),
        )
        .await;
}

async fn set_demand_expiry(
    harness: &Harness,
    owner: &UserCtx,
    fixture: MembershipFixture,
    active: bool,
) {
    let expiry = if active {
        "now() + interval '1 hour'"
    } else {
        "now() - interval '1 second'"
    };
    harness
        .seed_migration_owner(
            owner,
            &format!(
                "UPDATE owner_intraday_quote_demands \
                    SET lease_expires_at = {expiry}, updated_at = now() \
                  WHERE owner_user_id = '{}' AND membership_id = '{}' AND generation = {} \
                    AND state = 'ACTIVE'",
                owner.user_id, fixture.membership_id, fixture.generation
            ),
        )
        .await;
}

async fn set_cache_pending(
    harness: &Harness,
    owner: &UserCtx,
    fixture: MembershipFixture,
    at: DateTime<Utc>,
) {
    let at = at.to_rfc3339();
    harness
        .seed_migration_owner(
            owner,
            &format!(
                "UPDATE owner_intraday_quote_cache \
                    SET price = NULL, base_price = NULL, change_amount = NULL, \
                        change_percent = NULL, direction = NULL, halted = NULL, \
                        received_at = NULL, last_success_at = NULL, quote_version = 0, \
                        last_attempt_at = '{at}'::timestamptz, \
                        last_failure_code = NULL, last_failure_at = NULL, updated_at = now() \
                  WHERE owner_user_id = '{}' AND membership_id = '{}'",
                owner.user_id, fixture.membership_id
            ),
        )
        .await;
}

async fn set_cache_good(
    harness: &Harness,
    owner: &UserCtx,
    fixture: MembershipFixture,
    receipt_at: DateTime<Utc>,
    attempt_at: DateTime<Utc>,
    failure: Option<(&str, DateTime<Utc>)>,
) {
    let receipt_at = receipt_at.to_rfc3339();
    let attempt_at = attempt_at.to_rfc3339();
    let (failure_code, failure_at) = failure
        .map(|(code, failure_at)| {
            (
                format!("'{code}'"),
                format!("'{}'::timestamptz", failure_at.to_rfc3339()),
            )
        })
        .unwrap_or_else(|| ("NULL".to_owned(), "NULL".to_owned()));
    harness
        .seed_migration_owner(
            owner,
            &format!(
                "UPDATE owner_intraday_quote_cache \
                    SET price = 100.25, base_price = 99.00, change_amount = 1.25, \
                        change_percent = 1.26, direction = 'UP', halted = false, \
                        received_at = '{receipt_at}'::timestamptz, \
                        last_success_at = '{receipt_at}'::timestamptz, \
                        quote_version = 1, last_attempt_at = '{attempt_at}'::timestamptz, \
                        last_failure_code = {failure_code}, last_failure_at = {failure_at}, \
                        updated_at = now() \
                  WHERE owner_user_id = '{}' AND membership_id = '{}'",
                owner.user_id, fixture.membership_id
            ),
        )
        .await;
}

async fn seed_next_generation(
    harness: &Harness,
    owner: &UserCtx,
    fixture: MembershipFixture,
) -> MembershipFixture {
    let generation_id = Uuid::new_v4();
    harness
        .seed_migration_owner(
            owner,
            &format!(
                "INSERT INTO owner_equity_instrument_generations \
                 (id, membership_id, owner_user_id, instrument_id, generation, \
                  target_observed_sessions, minimum_observed_sessions, observed_sessions, \
                  first_session, last_session) \
                 VALUES ('{generation_id}', '{}', '{}', '{INSTRUMENT}', 2, \
                         261, 121, 121, '2026-04-15', '2026-08-13')",
                fixture.membership_id, owner.user_id
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
                 VALUES ('{generation_id}', '{}', '{}', '{INSTRUMENT}', 2, \
                         'sha256:{}', 'sha256:{}', '{ENTITLEMENT_SHA256}', \
                         '{CODE_COMMIT}', '{CODE_COMMIT}')",
                owner.user_id,
                fixture.membership_id,
                "1111111111111111111111111111111111111111111111111111111111111111",
                "2222222222222222222222222222222222222222222222222222222222222222"
            ),
        )
        .await;
    MembershipFixture {
        membership_id: fixture.membership_id,
        generation_id,
        generation: 2,
    }
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

fn assert_seed_fingerprints(values: &[String]) {
    assert_eq!(values.len(), 13);
    for (index, value) in values.iter().enumerate() {
        assert_ne!(value, "[]", "fingerprint slot {index} must be seeded");
    }
    let eod_rows: Value = serde_json::from_str(&values[12]).expect("EOD fingerprint JSON");
    assert_eq!(eod_rows.as_array().expect("EOD rows array").len(), 3);
}

async fn assert_get_batch(
    harness: &Harness,
    path: &str,
    user: &UserCtx,
    count: usize,
    expected: Option<&Value>,
) -> Value {
    assert!(matches!(count, 1 | 10 | 100));
    let before = fingerprint(harness, user.user_id).await;
    let mut first = None;
    for _ in 0..count {
        let response = harness.get(path, Some(user)).await;
        assert_eq!(status(&response), StatusCode::OK);
        assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
        let body = Harness::body_json(response).await;
        if let Some(expected) = expected {
            assert_eq!(&body, expected);
        }
        if first.is_none() {
            first = Some(body);
        }
    }
    assert_eq!(fingerprint(harness, user.user_id).await, before);
    first.expect("GET batch has one response")
}

struct CatchUnwindFuture<F> {
    future: Pin<Box<F>>,
}

impl<F: Future> Future for CatchUnwindFuture<F> {
    type Output = Result<F::Output, Box<dyn Any + Send>>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        match catch_unwind(AssertUnwindSafe(|| this.future.as_mut().poll(context))) {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(output)) => Poll::Ready(Ok(output)),
            Err(payload) => Poll::Ready(Err(payload)),
        }
    }
}

fn catch_unwind_async<F: Future>(future: F) -> CatchUnwindFuture<F> {
    CatchUnwindFuture {
        future: Box::pin(future),
    }
}

async fn fixture_time_bundle_from_database(harness: &Harness) -> FixtureTimeBundle {
    let db_now = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&harness.owner_pool)
        .await
        .expect("captured database clock");
    fixture_time_bundle(db_now)
}

#[test]
fn fixture_time_bundle_covers_kst_day_edges_without_changing_db_time() {
    // Pure planner coverage: these are synthetic captured DB instants, not
    // mutations of the database clock.
    let date = NaiveDate::from_ymd_opt(2026, 9, 9).expect("synthetic date");
    let cases = [
        ("early midnight", kst_instant(date, 0, 0, 1)),
        ("08:00", kst_instant(date, 8, 0, 0)),
        ("before 11:00", kst_instant(date, 10, 59, 59)),
        ("11:00", kst_instant(date, 11, 0, 0)),
        ("after close", kst_instant(date, 15, 30, 1)),
        ("late 23:59", kst_instant(date, 23, 59, 55)),
        (
            "UTC/KST boundary",
            Utc.with_ymd_and_hms(2026, 9, 8, 15, 0, 1)
                .single()
                .expect("UTC boundary instant"),
        ),
    ];

    for (name, db_now) in cases {
        let bundle = fixture_time_bundle(db_now);
        assert_eq!(bundle.session_date, date, "{name} KST date");
        assert_fixture_time_bundle(&bundle);
    }
}

fn assert_quote_dto(
    body: &Value,
    fixture: MembershipFixture,
    window: &IntradaySessionWindowContract,
    session_date: NaiveDate,
) {
    let fields = body
        .as_object()
        .expect("quote DTO")
        .keys()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        fields,
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
    assert_eq!(body["schema_version"], 1);
    assert_eq!(body["membership_id"], fixture.membership_id.to_string());
    assert_eq!(body["instrument_id"], INSTRUMENT);
    assert_eq!(body["venue"], "KRX");
    assert_eq!(body["currency"], "KRW");
    assert_eq!(body["generation"], fixture.generation);
    assert_eq!(body["next_poll_after_ms"], 5000);
    assert_eq!(body["session"]["date"], session_date.to_string());
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
    assert!(body.get("owner").is_none() && body.get("generation_id").is_none());
    assert_eq!(body["quote"]["price"], "100.25000000");
    assert_eq!(body["quote"]["base_price"], "99.00000000");
    assert_eq!(body["quote"]["change_from_previous_day"], "1.25000000");
    assert_eq!(
        body["quote"]["change_percent_from_previous_day"],
        "1.26000000"
    );
    assert_eq!(body["quote"]["direction"], "UP");
    assert_eq!(body["quote"]["quote_version"], "1");
}

async fn run_owner_intraday_quote_cache_matrix(harness: &mut Harness) {
    let owner_fixture = seed_ready_membership(harness, &harness.owner, INSTRUMENT).await;
    let other_owner = harness
        .seed_user(
            Role::Owner,
            "intraday-cache-other@lagrange.test",
            "intraday-cache-other-iss",
            "intraday-cache-other-sub",
        )
        .await;
    let foreign_fixture = seed_ready_membership(harness, &other_owner, INSTRUMENT).await;
    let times = fixture_time_bundle_from_database(harness).await;
    let db_date = times.session_date;
    set_api_clock(times.api_initial);
    let window = times.window.clone();
    seed_calendar_and_cache(harness, &harness.owner, owner_fixture, &times, "TRADING").await;
    seed_queue_row(harness, &harness.owner).await;

    harness
        .restart_api_with_intraday_read_config(
            OwnerIntradayQuoteReadConfig::OwnerOnly {
                window: Some(window.clone()),
            },
            injected_api_clock,
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

    assert_app_role_and_producer_is_forbidden(harness).await;
    let before = fingerprint(harness, harness.owner.user_id).await;
    assert_seed_fingerprints(&before);

    let path = format!(
        "{GET_PATH_PREFIX}?membership_id={}&generation=1",
        owner_fixture.membership_id
    );
    let body = assert_get_batch(harness, &path, &harness.owner, 1, None).await;
    assert_eq!(body["market_state"], "OPEN");
    assert_eq!(body["freshness"], "RECENT");
    assert!(body["reason_code"].is_null());
    assert_quote_dto(&body, owner_fixture, &window, db_date);
    let _ = assert_get_batch(harness, &path, &harness.owner, 10, Some(&body)).await;
    let _ = assert_get_batch(harness, &path, &harness.owner, 100, Some(&body)).await;

    let before_auth_errors = fingerprint(harness, harness.owner.user_id).await;
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
    let member_malformed = harness
        .get(
            &format!("{GET_PATH_PREFIX}?membership_id=not-a-uuid&generation=not-a-generation"),
            Some(&harness.member),
        )
        .await;
    assert_error(member_malformed, StatusCode::FORBIDDEN, "FORBIDDEN").await;
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
    let owner_unknown_query = harness
        .get(&format!("{path}&unknown=1"), Some(&harness.owner))
        .await;
    assert_error(
        owner_unknown_query,
        StatusCode::BAD_REQUEST,
        "INVALID_PARAMETER",
    )
    .await;
    let owner_duplicate_query = harness
        .get(
            &format!(
                "{GET_PATH_PREFIX}?membership_id={}&membership_id={}&generation=1",
                owner_fixture.membership_id, owner_fixture.membership_id
            ),
            Some(&harness.owner),
        )
        .await;
    assert_error(
        owner_duplicate_query,
        StatusCode::BAD_REQUEST,
        "INVALID_PARAMETER",
    )
    .await;
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
    assert_eq!(
        fingerprint(harness, harness.owner.user_id).await,
        before_auth_errors
    );

    harness
        .restart_api_with_intraday_read_config(
            OwnerIntradayQuoteReadConfig::Disabled,
            injected_api_clock,
        )
        .await;
    let disabled = harness.get(&path, Some(&harness.owner)).await;
    assert_eq!(status(&disabled), StatusCode::OK);
    assert_eq!(disabled.headers().get("cache-control").unwrap(), "no-store");
    let disabled_body = Harness::body_json(disabled).await;
    assert_eq!(disabled_body["market_state"], "UNKNOWN");
    assert_eq!(disabled_body["freshness"], "UNAVAILABLE");
    assert_eq!(disabled_body["reason_code"], "FEATURE_DISABLED");
    assert!(disabled_body["session"].is_null() && disabled_body["quote"].is_null());
    assert_eq!(
        fingerprint(harness, harness.owner.user_id).await,
        before_auth_errors
    );

    harness
        .restart_api_with_intraday_read_config(
            OwnerIntradayQuoteReadConfig::OwnerOnly {
                window: Some(window.clone()),
            },
            injected_api_clock,
        )
        .await;
    set_demand_expiry(harness, &harness.owner, owner_fixture, false).await;
    let no_demand = assert_get_batch(harness, &path, &harness.owner, 1, None).await;
    assert_eq!(no_demand["market_state"], "OPEN");
    assert_eq!(no_demand["freshness"], "RECENT");
    assert_eq!(no_demand["reason_code"], "NO_ACTIVE_DEMAND");
    assert!(no_demand["quote"].is_object());

    set_demand_expiry(harness, &harness.owner, owner_fixture, true).await;
    set_cache_pending(harness, &harness.owner, owner_fixture, times.attempt_at).await;
    let pending = assert_get_batch(harness, &path, &harness.owner, 1, None).await;
    assert_eq!(pending["market_state"], "OPEN");
    assert_eq!(pending["freshness"], "UNAVAILABLE");
    assert_eq!(pending["reason_code"], "QUOTE_PENDING");
    assert!(pending["quote"].is_null());

    set_cache_good(
        harness,
        &harness.owner,
        owner_fixture,
        times.receipt_at,
        times.attempt_at,
        Some(("PROVIDER_TIMEOUT", times.failure_at)),
    )
    .await;
    let failure = assert_get_batch(harness, &path, &harness.owner, 1, None).await;
    assert_eq!(failure["market_state"], "OPEN");
    assert_eq!(failure["freshness"], "RECENT");
    assert_eq!(failure["reason_code"], "PROVIDER_TIMEOUT");
    assert!(failure["quote"].is_object());

    set_cache_good(
        harness,
        &harness.owner,
        owner_fixture,
        times.receipt_at,
        times.attempt_at,
        None,
    )
    .await;
    let recent_before_restart = assert_get_batch(harness, &path, &harness.owner, 1, None).await;
    assert_eq!(recent_before_restart["market_state"], "OPEN");
    assert_eq!(recent_before_restart["freshness"], "RECENT");
    assert!(recent_before_restart["reason_code"].is_null());
    let before_restart = fingerprint(harness, harness.owner.user_id).await;
    set_api_clock(times.api_after_stale);
    harness
        .restart_api_with_intraday_read_config(
            OwnerIntradayQuoteReadConfig::OwnerOnly {
                window: Some(window.clone()),
            },
            injected_api_clock,
        )
        .await;
    let restarted_stale = assert_get_batch(harness, &path, &harness.owner, 1, None).await;
    assert_eq!(restarted_stale["market_state"], "OPEN");
    assert_eq!(restarted_stale["freshness"], "STALE");
    assert_eq!(restarted_stale["reason_code"], "QUOTE_STALE");
    assert_eq!(restarted_stale["quote"], recent_before_restart["quote"]);
    assert_eq!(
        restarted_stale["quote"]["quote_version"],
        recent_before_restart["quote"]["quote_version"]
    );
    assert_eq!(
        restarted_stale["quote"]["last_success_at"],
        recent_before_restart["quote"]["last_success_at"]
    );
    assert_eq!(
        fingerprint(harness, harness.owner.user_id).await,
        before_restart
    );
    set_api_clock(times.api_initial);
    harness
        .seed_migration_owner(
            &harness.owner,
            &format!(
                "UPDATE owner_intraday_quote_cache SET session_date = '{}' \
                  WHERE owner_user_id = '{}' AND membership_id = '{}'",
                db_date - chrono::Duration::days(1),
                harness.owner.user_id,
                owner_fixture.membership_id
            ),
        )
        .await;
    let old_session = assert_get_batch(harness, &path, &harness.owner, 1, None).await;
    assert_eq!(old_session["reason_code"], "QUOTE_PENDING");
    assert!(old_session["quote"].is_null());

    harness
        .seed_migration_owner(
            &harness.owner,
            &format!(
                "UPDATE owner_intraday_quote_cache SET session_date = '{}', \
                        window_contract_sha256 = 'sha256:{}' \
                  WHERE owner_user_id = '{}' AND membership_id = '{}'",
                db_date,
                "a".repeat(64),
                harness.owner.user_id,
                owner_fixture.membership_id
            ),
        )
        .await;
    let old_hash = assert_get_batch(harness, &path, &harness.owner, 1, None).await;
    assert_eq!(old_hash["reason_code"], "QUOTE_PENDING");
    assert!(old_hash["quote"].is_null());

    harness
        .seed_migration_owner(
            &harness.owner,
            &format!(
                "UPDATE owner_intraday_quote_cache SET window_contract_sha256 = '{}' \
                  WHERE owner_user_id = '{}' AND membership_id = '{}'",
                window.window_contract_sha256(),
                harness.owner.user_id,
                owner_fixture.membership_id
            ),
        )
        .await;
    let restored_old_generation = assert_get_batch(harness, &path, &harness.owner, 1, None).await;
    assert_eq!(restored_old_generation["market_state"], "OPEN");
    assert_eq!(restored_old_generation["freshness"], "RECENT");
    assert!(restored_old_generation["reason_code"].is_null());
    assert_quote_dto(&restored_old_generation, owner_fixture, &window, db_date);

    let next_fixture = seed_next_generation(harness, &harness.owner, owner_fixture).await;
    let next_path = format!(
        "{GET_PATH_PREFIX}?membership_id={}&generation=2",
        next_fixture.membership_id
    );
    let next_demand = harness
        .send(
            "POST",
            "/api/v1/research/owner-beta/equity-universe-v2/quote-demands",
            Some(&harness.owner),
            true,
            Some("cache-next-demand-rid"),
            Some("cache-next-demand-key"),
            Some(demand_body(next_fixture)),
        )
        .await;
    assert_eq!(status(&next_demand), StatusCode::OK);
    let before_new_admission_gets = fingerprint(harness, harness.owner.user_id).await;
    let old_generation = harness.get(&path, Some(&harness.owner)).await;
    assert_error(old_generation, StatusCode::NOT_FOUND, "RESOURCE_NOT_FOUND").await;
    let current_generation = harness.get(&next_path, Some(&harness.owner)).await;
    assert_eq!(status(&current_generation), StatusCode::OK);
    assert_eq!(
        current_generation.headers().get("cache-control").unwrap(),
        "no-store"
    );
    let current_generation_body = Harness::body_json(current_generation).await;
    assert_eq!(current_generation_body["generation"], 2);
    assert_eq!(current_generation_body["reason_code"], "QUOTE_PENDING");
    assert!(current_generation_body["quote"].is_null());
    assert_eq!(
        fingerprint(harness, harness.owner.user_id).await,
        before_new_admission_gets
    );

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
                window: Some(window.clone()),
            },
            injected_api_clock,
        )
        .await;
    let disabled_identity = harness.get(&next_path, Some(&harness.owner)).await;
    assert_error(
        disabled_identity,
        StatusCode::NOT_FOUND,
        "RESOURCE_NOT_FOUND",
    )
    .await;
}

#[tokio::test]
async fn owner_intraday_quote_cache_get_is_read_only_and_owner_scoped() {
    let mut harness = Harness::new()
        .await
        .expect("DATABASE_URL is required for real-role cache HTTP coverage");
    let result = catch_unwind_async(run_owner_intraday_quote_cache_matrix(&mut harness)).await;
    harness.teardown().await;
    if let Err(payload) = result {
        resume_unwind(payload);
    }
}

async fn run_intraday_quote_evidence_matrix(harness: &mut Harness) {
    let fixture = seed_ready_membership(harness, &harness.owner, INSTRUMENT).await;
    let times = fixture_time_bundle_from_database(harness).await;
    let db_date = times.session_date;
    set_api_clock(times.api_initial);
    let window = times.window.clone();
    let path = format!(
        "{GET_PATH_PREFIX}?membership_id={}&generation=1",
        fixture.membership_id
    );

    harness
        .restart_api_with_intraday_read_config(
            OwnerIntradayQuoteReadConfig::OwnerOnly {
                window: Some(window.clone()),
            },
            injected_api_clock,
        )
        .await;
    let missing_calendar = harness.get(&path, Some(&harness.owner)).await;
    assert_eq!(status(&missing_calendar), StatusCode::OK);
    assert_eq!(
        missing_calendar.headers().get("cache-control").unwrap(),
        "no-store"
    );
    let missing_calendar_body = Harness::body_json(missing_calendar).await;
    assert_eq!(missing_calendar_body["market_state"], "UNKNOWN");
    assert_eq!(missing_calendar_body["freshness"], "UNAVAILABLE");
    assert_eq!(missing_calendar_body["reason_code"], "CALENDAR_UNAVAILABLE");
    assert!(missing_calendar_body["session"].is_null());
    assert!(missing_calendar_body["quote"].is_null());

    seed_calendar_and_cache(harness, &harness.owner, fixture, &times, "CLOSED").await;
    harness
        .restart_api_with_intraday_read_config(
            OwnerIntradayQuoteReadConfig::OwnerOnly {
                window: Some(window.clone()),
            },
            injected_api_clock,
        )
        .await;
    let disagreement = harness.get(&path, Some(&harness.owner)).await;
    assert_eq!(status(&disagreement), StatusCode::OK);
    let disagreement_body = Harness::body_json(disagreement).await;
    assert_eq!(disagreement_body["market_state"], "UNKNOWN");
    assert_eq!(disagreement_body["freshness"], "UNAVAILABLE");
    assert_eq!(
        disagreement_body["reason_code"],
        "SESSION_WINDOW_UNAVAILABLE"
    );
    assert!(disagreement_body["session"].is_null() && disagreement_body["quote"].is_null());

    harness
        .restart_api_with_intraday_read_config(
            OwnerIntradayQuoteReadConfig::OwnerOnly { window: None },
            injected_api_clock,
        )
        .await;
    let missing_window = harness.get(&path, Some(&harness.owner)).await;
    assert_eq!(status(&missing_window), StatusCode::OK);
    let missing_window_body = Harness::body_json(missing_window).await;
    assert_eq!(missing_window_body["market_state"], "UNKNOWN");
    assert_eq!(
        missing_window_body["reason_code"],
        "SESSION_WINDOW_UNAVAILABLE"
    );
    assert!(missing_window_body["session"].is_null() && missing_window_body["quote"].is_null());

    let closed_window = window_for_kind(db_date, "CLOSED", None, None, times.evidence_at);
    harness
        .restart_api_with_intraday_read_config(
            OwnerIntradayQuoteReadConfig::OwnerOnly {
                window: Some(closed_window),
            },
            injected_api_clock,
        )
        .await;
    let closed = harness.get(&path, Some(&harness.owner)).await;
    assert_eq!(status(&closed), StatusCode::OK);
    let closed_body = Harness::body_json(closed).await;
    assert_eq!(closed_body["market_state"], "CLOSED");
    assert_eq!(closed_body["freshness"], "UNAVAILABLE");
    assert_eq!(closed_body["reason_code"], "SESSION_CLOSED");
    assert!(closed_body["session"].is_object() && closed_body["quote"].is_null());
}

#[tokio::test]
async fn owner_intraday_quote_cache_evidence_states_are_real_role_read_only() {
    let mut harness = Harness::new()
        .await
        .expect("DATABASE_URL is required for real-role cache HTTP coverage");
    let result = catch_unwind_async(run_intraday_quote_evidence_matrix(&mut harness)).await;
    harness.teardown().await;
    if let Err(payload) = result {
        resume_unwind(payload);
    }
}

#[tokio::test]
async fn async_projection_scenario_panic_is_captured_before_teardown() {
    let result = catch_unwind_async(async { panic!("synthetic WP4-C3 scenario failure") }).await;
    assert!(result.is_err());
}
