#![allow(dead_code)]

use std::time::Duration;

use auth::entitlement::Role;
use chrono::{DateTime, NaiveDate, Utc};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{ConnectOptions, PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

pub use crate::common::{Harness, UserCtx};

pub async fn required_harness() -> Harness {
    assert!(
        std::env::var_os("DATABASE_URL").is_some(),
        "service_status_reads requires DATABASE_URL; this suite must not silently skip"
    );
    Harness::new()
        .await
        .expect("required service-status database harness was not created")
}

pub async fn actor_session_pool(harness: &Harness, user_id: Uuid, session_hash: &str) -> PgPool {
    let options = harness
        .app_url
        .parse::<PgConnectOptions>()
        .expect("fixture app URL parses")
        .options([
            ("app.actor_user_id", user_id.to_string()),
            ("app.market_stream_session_hash", session_hash.to_owned()),
        ])
        .log_statements(tracing::log::LevelFilter::Off);
    PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(options)
        .await
        .expect("actor/session-scoped fixture pool connects")
}

#[derive(Debug, Clone)]
pub struct StreamFixture {
    pub owner_id: Uuid,
    pub session_hash: String,
    pub slot_id: Uuid,
    pub grant_id: Uuid,
    pub contract_sha256: String,
    pub window_sha256: String,
    pub session_date: NaiveDate,
    pub open_at: DateTime<Utc>,
    pub close_at: DateTime<Utc>,
    pub producer_epoch: Uuid,
    pub session_proof_id: Uuid,
    pub session_proof_sha256: String,
    pub calendar_batch_id: Uuid,
    pub calendar_content_sha256: String,
}

/// Seed only synthetic rights, calendar, and producer lineage through the
/// migration-owner test boundary. No provider or copied production data is used.
pub async fn seed_stream_fixture(harness: &Harness) -> Result<StreamFixture, &'static str> {
    seed_stream_fixture_with_retrievals(harness, [None, None, None]).await
}

/// Insert immutable calendar-version evidence at its intended synthetic age;
/// never disable append-only protection or rewrite an already inserted version.
pub async fn seed_expiring_stream_fixture(
    harness: &Harness,
) -> Result<(StreamFixture, DateTime<Utc>), &'static str> {
    let seeded: DateTime<Utc> = sqlx::query_scalar("SELECT pg_catalog.clock_timestamp()")
        .fetch_one(&harness.owner_pool)
        .await
        .map_err(|_| "proof expiry seed clock failed")?;
    let retrievals = [8, 9, 10].map(|seconds| {
        Some(seeded - chrono::Duration::hours(36) + chrono::Duration::seconds(seconds))
    });
    let stream = seed_stream_fixture_with_retrievals(harness, retrievals).await?;
    Ok((stream, seeded + chrono::Duration::seconds(8)))
}

async fn seed_stream_fixture_with_retrievals(
    harness: &Harness,
    retrievals: [Option<DateTime<Utc>>; 3],
) -> Result<StreamFixture, &'static str> {
    let owner_id = harness.owner.user_id;
    let session_hash = crate::common::sha256_hex(harness.owner.cookie_value.as_bytes());
    let slot_id = Uuid::new_v4();
    let grant_id = Uuid::new_v4();
    let entitlement_id = Uuid::new_v4();
    let calendar_batch_id = Uuid::new_v4();
    let producer_epoch = Uuid::new_v4();
    let session_proof_id = Uuid::new_v4();
    let contract_sha256 = "a".repeat(64);
    let window_sha256 = format!("sha256:{}", "b".repeat(64));
    let entitlement_sha256 = "c".repeat(64);
    let calendar_content_sha256 = "d".repeat(64);

    let mut tx = harness
        .owner_pool
        .begin()
        .await
        .map_err(|_| "owner fixture transaction could not start")?;
    sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
        .bind(owner_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|_| "owner fixture actor context could not be set")?;

    let (session_date, open_at, close_at): (NaiveDate, DateTime<Utc>, DateTime<Utc>) =
        sqlx::query_as(
            "SELECT local_now::date,
                    ((local_now::date + TIME '00:00:00') AT TIME ZONE 'Asia/Seoul'),
                    ((local_now::date + TIME '23:59:59') AT TIME ZONE 'Asia/Seoul')
               FROM (SELECT pg_catalog.clock_timestamp()
                            AT TIME ZONE 'Asia/Seoul' AS local_now) AS clock",
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| "fixture KST date/window could not be read")?;

    sqlx::query(
        "INSERT INTO public.data_entitlements (
             id, contract_document_sha256, contract_reference, status,
             covered_datasets, covered_uses, effective_from, effective_until, managed_by
         ) VALUES ($1, $2, $3, 'ACTIVE', '[]'::jsonb, '[]'::jsonb, $4, $4, $5)",
    )
    .bind(entitlement_id)
    .bind(&entitlement_sha256)
    .bind("fixture:service-status-stream")
    .bind(session_date)
    .bind(owner_id)
    .execute(&mut *tx)
    .await
    .map_err(|_| "synthetic stream entitlement could not be seeded")?;

    sqlx::query(
        "INSERT INTO public.owner_market_stream_grants (
             id, credential_slot_id, credential_generation, owner_user_id,
             entitlement_id, entitlement_reference, entitlement_document_sha256,
             tr_id, wire_version, network_contract_sha256, identity_list_sha256,
             effective_from, effective_until, activation_commit, state, grant_revision
         ) VALUES (
             $1, $2, '1', $3, $4, $5, $6, 'H0STCNT0',
             'kis-h0stcnt0-20260914-v1', $7,
             '0e6a9b3aef6b310685b9bd5594a39452c2902d11af623197699cf6dc46931e79',
             $8, $8, 'c3295be', 'ACTIVE', $9
         )",
    )
    .bind(grant_id)
    .bind(slot_id)
    .bind(owner_id)
    .bind(entitlement_id)
    .bind("fixture:service-status-stream")
    .bind(&entitlement_sha256)
    .bind(&contract_sha256)
    .bind(session_date)
    .bind(Uuid::new_v4())
    .execute(&mut *tx)
    .await
    .map_err(|_| "synthetic stream grant could not be seeded")?;

    sqlx::query(
        "INSERT INTO public.data_batches (
             id, source_batch_id, provider, market, batch_date, kind, storage_path,
             content_sha256, bytes_size, retrieved_at, source_file_name, fetch_mode
         ) VALUES (
             $1, $1, 'KRX', 'KR', $2, 'CALENDAR', 'fixture/service-status/calendar.json',
             $3, 1, COALESCE($4, pg_catalog.clock_timestamp()), 'calendar.json', 'credentialed'
         )",
    )
    .bind(calendar_batch_id)
    .bind(session_date)
    .bind(&calendar_content_sha256)
    .bind(retrievals[2])
    .execute(&mut *tx)
    .await
    .map_err(|_| "synthetic calendar batch could not be seeded")?;

    sqlx::query(
        "INSERT INTO public.trading_calendars (
             exchange, session_date, session_type, timezone, source, source_version,
             source_batch_id, content_sha256, retrieved_at
         ) VALUES (
             'KRX', $1, 'TRADING', 'Asia/Seoul', 'kis',
             'kis-chk-holiday-v1:schema-1', $2, $3, COALESCE($4, pg_catalog.clock_timestamp())
         )",
    )
    .bind(session_date)
    .bind(calendar_batch_id)
    .bind(&calendar_content_sha256)
    .bind(retrievals[0])
    .execute(&mut *tx)
    .await
    .map_err(|_| "synthetic current calendar projection could not be seeded")?;

    sqlx::query(
        "INSERT INTO public.trading_calendar_versions (
             exchange, session_date, session_type, timezone, source, source_version,
             source_batch_id, content_sha256, retrieved_at
         ) VALUES (
             'KRX', $1, 'TRADING', 'Asia/Seoul', 'kis',
             'kis-chk-holiday-v1:schema-1', $2, $3, COALESCE($4, pg_catalog.clock_timestamp())
         )",
    )
    .bind(session_date)
    .bind(calendar_batch_id)
    .bind(&calendar_content_sha256)
    .bind(retrievals[1])
    .execute(&mut *tx)
    .await
    .map_err(|_| "synthetic immutable calendar version could not be seeded")?;

    let proof_body = format!(
        "owner-market-stream-day-v1\n{}\n{}\n{}\n{}\n{}\n{}\n",
        session_date.format("%Y-%m-%d"),
        calendar_batch_id,
        calendar_content_sha256,
        window_sha256,
        open_at.format("%Y-%m-%dT%H:%M:%SZ"),
        close_at.format("%Y-%m-%dT%H:%M:%SZ"),
    );
    let proof_sha256: String = sqlx::query_scalar(
        "SELECT 'sha256:' || pg_catalog.encode(
            pg_catalog.sha256(pg_catalog.convert_to($1, 'UTF8')), 'hex'
         )",
    )
    .bind(proof_body)
    .fetch_one(&mut *tx)
    .await
    .map_err(|_| "synthetic day-proof digest could not be calculated")?;
    let grant_revision: Uuid = sqlx::query_scalar(
        "SELECT grant_revision FROM public.owner_market_stream_grants WHERE id = $1",
    )
    .bind(grant_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|_| "synthetic grant revision could not be loaded")?;

    sqlx::query(
        "INSERT INTO public.owner_market_stream_producers (
             credential_slot_id, grant_id, grant_revision, owner_user_id, holder_id,
             fencing_token, current_epoch, session_date, session_proof_id,
             session_proof_sha256, calendar_source, calendar_source_version,
             calendar_source_batch_id, calendar_content_sha256,
             window_contract_sha256, lease_expires_at, heartbeat_at,
             connection_state, gap_generation, state_version
         ) VALUES (
             $1, $2, $3, $4, $5, 1, $6, $7, $8, $9, 'kis',
             'kis-chk-holiday-v1:schema-1', $10, $11, $12,
             pg_catalog.clock_timestamp() + INTERVAL '2 minutes',
             pg_catalog.clock_timestamp(), 'CONNECTED', 0, 1
         )",
    )
    .bind(slot_id)
    .bind(grant_id)
    .bind(grant_revision)
    .bind(owner_id)
    .bind(Uuid::new_v4())
    .bind(producer_epoch)
    .bind(session_date)
    .bind(session_proof_id)
    .bind(&proof_sha256)
    .bind(calendar_batch_id)
    .bind(&calendar_content_sha256)
    .bind(&window_sha256)
    .execute(&mut *tx)
    .await
    .map_err(|_| "synthetic stream producer could not be seeded")?;

    tx.commit()
        .await
        .map_err(|_| "synthetic stream fixture could not commit")?;

    Ok(StreamFixture {
        owner_id,
        session_hash,
        slot_id,
        grant_id,
        contract_sha256,
        window_sha256,
        session_date,
        open_at,
        close_at,
        producer_epoch,
        session_proof_id,
        session_proof_sha256: proof_sha256,
        calendar_batch_id,
        calendar_content_sha256,
    })
}

#[derive(Debug, Clone, Copy)]
pub struct DemandFixture {
    pub membership_id: Uuid,
    pub generation_id: Uuid,
    pub instrument_id: &'static str,
    pub subscription_revision: Uuid,
    pub original_lease_id: Uuid,
}

async fn insert_ready_owner_membership(
    tx: &mut Transaction<'_, Postgres>,
    owner_id: Uuid,
    session_date: NaiveDate,
    instrument_id: &str,
) -> Result<(Uuid, Uuid), &'static str> {
    let membership_id = Uuid::new_v4();
    let generation_id = Uuid::new_v4();
    let entitlement_hash = format!("sha256:{}", "e".repeat(64));
    let commit = "c3295be";
    sqlx::query(
        "INSERT INTO public.owner_equity_memberships (
             id, owner_user_id, instrument_id, state, transition_actor_user_id,
             transition_code_commit, transition_entitlement_sha256
         ) VALUES ($1, $2, $3, 'REQUESTED', $2, $4, $5)",
    )
    .bind(membership_id)
    .bind(owner_id)
    .bind(instrument_id)
    .bind(commit)
    .bind(&entitlement_hash)
    .execute(&mut **tx)
    .await
    .map_err(|_| "synthetic second Owner membership could not be requested")?;
    for state in ["VALIDATING", "BACKFILLING", "MATERIALIZING"] {
        sqlx::query(
            "UPDATE public.owner_equity_memberships
                SET state = $2, transition_actor_user_id = $3,
                    transition_code_commit = $4,
                    transition_entitlement_sha256 = $5
              WHERE id = $1",
        )
        .bind(membership_id)
        .bind(state)
        .bind(owner_id)
        .bind(commit)
        .bind(&entitlement_hash)
        .execute(&mut **tx)
        .await
        .map_err(|_| "synthetic second Owner membership lifecycle transition failed")?;
    }
    sqlx::query(
        "INSERT INTO public.owner_equity_instrument_generations (
             id, membership_id, owner_user_id, instrument_id, generation,
             target_observed_sessions, minimum_observed_sessions, observed_sessions,
             first_session, last_session
         ) VALUES ($1, $2, $3, $4, 1, 121, 121, 121, $5, $5)",
    )
    .bind(generation_id)
    .bind(membership_id)
    .bind(owner_id)
    .bind(instrument_id)
    .bind(session_date)
    .execute(&mut **tx)
    .await
    .map_err(|_| "synthetic second Owner generation could not be seeded")?;
    sqlx::query(
        "INSERT INTO public.owner_equity_generation_admissions (
             generation_id, owner_user_id, membership_id, instrument_id, generation,
             raw_manifest_sha256, artifact_manifest_sha256, entitlement_sha256,
             capture_code_commit, materializer_code_commit
         ) VALUES ($1, $2, $3, $4, 1, $5, $5, $5, $6, $6)",
    )
    .bind(generation_id)
    .bind(owner_id)
    .bind(membership_id)
    .bind(instrument_id)
    .bind(&entitlement_hash)
    .bind(commit)
    .execute(&mut **tx)
    .await
    .map_err(|_| "synthetic second Owner generation admission could not be seeded")?;
    sqlx::query(
        "UPDATE public.owner_equity_memberships
            SET state = 'READY', transition_actor_user_id = $2,
                transition_code_commit = $3,
                transition_entitlement_sha256 = $4
          WHERE id = $1",
    )
    .bind(membership_id)
    .bind(owner_id)
    .bind(commit)
    .bind(&entitlement_hash)
    .execute(&mut **tx)
    .await
    .map_err(|_| "synthetic second Owner membership could not become READY")?;
    Ok((membership_id, generation_id))
}

/// Seed one current admitted identity and four leases. Two valid Owner browser
/// sessions request it (duplicate tabs count once); expired and revoked session
/// leases remain stored but must be excluded by the session validator.
pub async fn seed_demand_fixture(
    harness: &Harness,
    stream: &StreamFixture,
) -> Result<DemandFixture, &'static str> {
    let owner_id = stream.owner_id;
    let membership_id = Uuid::new_v4();
    let generation_id = Uuid::new_v4();
    let entitlement_hash = format!("sha256:{}", "e".repeat(64));
    let commit = "c3295be";

    let mut tx = harness
        .owner_pool
        .begin()
        .await
        .map_err(|_| "demand fixture transaction could not start")?;
    sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
        .bind(owner_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|_| "demand fixture actor context could not be set")?;

    sqlx::query(
        "INSERT INTO public.owner_equity_memberships (
             id, owner_user_id, instrument_id, state, transition_actor_user_id,
             transition_code_commit, transition_entitlement_sha256
         ) VALUES ($1, $2, '069500.KRX', 'REQUESTED', $2, $3, $4)",
    )
    .bind(membership_id)
    .bind(owner_id)
    .bind(commit)
    .bind(&entitlement_hash)
    .execute(&mut *tx)
    .await
    .map_err(|_| "synthetic Owner membership could not be requested")?;

    for state in ["VALIDATING", "BACKFILLING", "MATERIALIZING"] {
        sqlx::query(
            "UPDATE public.owner_equity_memberships
                SET state = $2, transition_actor_user_id = $3,
                    transition_code_commit = $4,
                    transition_entitlement_sha256 = $5
              WHERE id = $1",
        )
        .bind(membership_id)
        .bind(state)
        .bind(owner_id)
        .bind(commit)
        .bind(&entitlement_hash)
        .execute(&mut *tx)
        .await
        .map_err(|_| "synthetic Owner membership lifecycle transition failed")?;
    }

    sqlx::query(
        "INSERT INTO public.owner_equity_instrument_generations (
             id, membership_id, owner_user_id, instrument_id, generation,
             target_observed_sessions, minimum_observed_sessions, observed_sessions,
             first_session, last_session
         ) VALUES ($1, $2, $3, '069500.KRX', 1, 121, 121, 121, $4, $4)",
    )
    .bind(generation_id)
    .bind(membership_id)
    .bind(owner_id)
    .bind(stream.session_date)
    .execute(&mut *tx)
    .await
    .map_err(|_| "synthetic Owner equity generation could not be seeded")?;

    sqlx::query(
        "INSERT INTO public.owner_equity_generation_admissions (
             generation_id, owner_user_id, membership_id, instrument_id, generation,
             raw_manifest_sha256, artifact_manifest_sha256, entitlement_sha256,
             capture_code_commit, materializer_code_commit
         ) VALUES ($1, $2, $3, '069500.KRX', 1, $4, $5, $6, $7, $7)",
    )
    .bind(generation_id)
    .bind(owner_id)
    .bind(membership_id)
    .bind(&entitlement_hash)
    .bind(&entitlement_hash)
    .bind(&entitlement_hash)
    .bind(commit)
    .execute(&mut *tx)
    .await
    .map_err(|_| "synthetic Owner generation admission could not be seeded")?;

    sqlx::query(
        "UPDATE public.owner_equity_memberships
            SET state = 'READY', transition_actor_user_id = $2,
                transition_code_commit = $3,
                transition_entitlement_sha256 = $4
          WHERE id = $1",
    )
    .bind(membership_id)
    .bind(owner_id)
    .bind(commit)
    .bind(&entitlement_hash)
    .execute(&mut *tx)
    .await
    .map_err(|_| "synthetic Owner membership could not become READY")?;

    let (invalid_session_membership_id, invalid_session_generation_id) =
        insert_ready_owner_membership(&mut tx, owner_id, stream.session_date, "069501.KRX").await?;

    let other_active_session = crate::common::sha256_hex(b"fixture-active-extra-session");
    let expired_session = crate::common::sha256_hex(b"fixture-expired-session");
    let revoked_session = crate::common::sha256_hex(b"fixture-revoked-session");
    for (hash, csrf, expiration) in [
        (&other_active_session, "f", "active"),
        (&expired_session, "1", "expired"),
        (&revoked_session, "2", "revoked"),
    ] {
        let csrf_hash = crate::common::sha256_hex(csrf.as_bytes());
        match expiration {
            "active" => {
                sqlx::query(
                    "INSERT INTO public.web_sessions (user_id, session_hash, csrf_hash, expires_at)
                     VALUES ($1, $2, $3, pg_catalog.clock_timestamp() + INTERVAL '1 hour')",
                )
                .bind(owner_id)
                .bind(hash)
                .bind(csrf_hash)
                .execute(&mut *tx)
                .await
                .map_err(|_| "additional active Owner session could not be seeded")?;
            }
            "expired" => {
                sqlx::query(
                    "INSERT INTO public.web_sessions (
                         user_id, session_hash, csrf_hash, created_at, expires_at
                     ) VALUES (
                         $1, $2, $3, pg_catalog.clock_timestamp() - INTERVAL '2 minutes',
                         pg_catalog.clock_timestamp() - INTERVAL '1 minute'
                     )",
                )
                .bind(owner_id)
                .bind(hash)
                .bind(csrf_hash)
                .execute(&mut *tx)
                .await
                .map_err(|_| "expired Owner session could not be seeded")?;
            }
            _ => {
                sqlx::query(
                    "INSERT INTO public.web_sessions (user_id, session_hash, csrf_hash, expires_at)
                     VALUES ($1, $2, $3, pg_catalog.clock_timestamp() + INTERVAL '1 hour')",
                )
                .bind(owner_id)
                .bind(hash)
                .bind(csrf_hash)
                .execute(&mut *tx)
                .await
                .map_err(|_| "revoked Owner session could not be seeded")?;
                sqlx::query(
                    "UPDATE public.web_sessions SET revoked_at = pg_catalog.clock_timestamp()
                      WHERE session_hash = $1",
                )
                .bind(hash)
                .execute(&mut *tx)
                .await
                .map_err(|_| "synthetic Owner session revocation failed")?;
            }
        }
    }

    let mut original_lease_id = Uuid::nil();
    for (session_hash, suffix) in [
        (&stream.session_hash, "original"),
        (&other_active_session, "other"),
        (&expired_session, "expired"),
        (&revoked_session, "revoked"),
    ] {
        let lease_id = Uuid::new_v4();
        if suffix == "original" {
            original_lease_id = lease_id;
        }
        sqlx::query(
            "INSERT INTO public.owner_market_stream_leases (
                 id, owner_user_id, consumer_id, session_hash, kind,
                 lease_expires_at, idempotency_key_sha256, request_sha256
             ) VALUES (
                 $1, $2, $3, $4, 'BROWSER', pg_catalog.clock_timestamp() + INTERVAL '1 hour',
                 'sha256:' || pg_catalog.repeat($5, 64),
                 'sha256:' || pg_catalog.repeat($5, 64)
             )",
        )
        .bind(lease_id)
        .bind(owner_id)
        .bind(Uuid::new_v4())
        .bind(session_hash)
        .bind(match suffix {
            "original" => "a",
            "other" => "b",
            "expired" => "c",
            _ => "d",
        })
        .execute(&mut *tx)
        .await
        .map_err(|_| "synthetic stream lease could not be seeded")?;
        let (item_membership_id, item_generation_id, item_instrument_id) = match suffix {
            "expired" | "revoked" => (
                invalid_session_membership_id,
                invalid_session_generation_id,
                "069501.KRX",
            ),
            _ => (membership_id, generation_id, "069500.KRX"),
        };
        sqlx::query(
            "INSERT INTO public.owner_market_stream_lease_items (
                 lease_id, owner_user_id, membership_id, generation_id,
                 instrument_id, generation
             ) VALUES ($1, $2, $3, $4, $5, 1)",
        )
        .bind(lease_id)
        .bind(owner_id)
        .bind(item_membership_id)
        .bind(item_generation_id)
        .bind(item_instrument_id)
        .execute(&mut *tx)
        .await
        .map_err(|_| "synthetic stream lease item could not be seeded")?;
    }

    tx.commit()
        .await
        .map_err(|_| "synthetic demand fixture could not commit")?;
    Ok(DemandFixture {
        membership_id,
        generation_id,
        instrument_id: "069500.KRX",
        subscription_revision: Uuid::new_v4(),
        original_lease_id,
    })
}

pub async fn seed_live_quote_fixture(
    harness: &Harness,
    stream: &StreamFixture,
    demand: &DemandFixture,
) -> Result<(), &'static str> {
    let mut tx = harness
        .owner_pool
        .begin()
        .await
        .map_err(|_| "quote fixture transaction could not start")?;
    sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
        .bind(stream.owner_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|_| "quote fixture actor context could not be set")?;

    sqlx::query(
        "INSERT INTO public.owner_market_stream_subscriptions (
             credential_slot_id, symbol, provider, environment, venue, tr_id,
             grant_revision, epoch, state, acked_at, desired_reference_count,
             subscription_revision
         ) VALUES (
             $1, $5, 'kis', 'live', 'KRX', 'H0STCNT0',
             (SELECT grant_revision FROM public.owner_market_stream_grants WHERE id = $2),
             $3, 'ACKED', pg_catalog.clock_timestamp(), 1, $4
         )",
    )
    .bind(stream.slot_id)
    .bind(stream.grant_id)
    .bind(stream.producer_epoch)
    .bind(demand.subscription_revision)
    .bind(&demand.instrument_id[..6])
    .execute(&mut *tx)
    .await
    .map_err(|_| "synthetic ACKED subscription could not be seeded")?;

    sqlx::query(
        "WITH capture AS MATERIALIZED (
             SELECT pg_catalog.clock_timestamp() - INTERVAL '1 second' AS received_at
         )
         INSERT INTO public.owner_market_stream_cache (
             owner_user_id, membership_id, generation_id, instrument_id, generation,
             grant_id, credential_slot_id, grant_revision, source, wire_version,
             venue, currency, session_date, calendar_source, calendar_source_version,
             session_proof_id, session_proof_sha256, calendar_source_batch_id,
             calendar_content_sha256, window_contract_sha256, epoch, fencing_token,
             subscription_revision, receive_ordinal, price, change_amount,
             change_percent, direction, trade_volume, cumulative_volume, halted,
             business_date, trade_time, provider_trade_at, quote_version,
             connection_state, market_state, freshness, availability, gap_generation,
             received_at, committed_at
         )
         SELECT $1, $2, $3, $4, 1, $5, $6,
                (SELECT grant_revision FROM public.owner_market_stream_grants WHERE id = $5),
                'KIS_MARKET_WS', 'kis-h0stcnt0-20260914-v1', 'KRX', 'KRW', $7,
                'kis', 'kis-chk-holiday-v1:schema-1', $8, $9, $10, $11, $12,
                $13, 1, $14, 1, 101, 1, 1, 'UP', 10, 100, false,
                (capture.received_at AT TIME ZONE 'Asia/Seoul')::date,
                (capture.received_at AT TIME ZONE 'Asia/Seoul')::time,
                capture.received_at, 1, 'CONNECTED', 'OPEN', 'RECENT', 'LIVE', 0,
                capture.received_at, capture.received_at + INTERVAL '100 milliseconds'
           FROM capture",
    )
    .bind(stream.owner_id)
    .bind(demand.membership_id)
    .bind(demand.generation_id)
    .bind(demand.instrument_id)
    .bind(stream.grant_id)
    .bind(stream.slot_id)
    .bind(stream.session_date)
    .bind(stream.session_proof_id)
    .bind(&stream.session_proof_sha256)
    .bind(stream.calendar_batch_id)
    .bind(&stream.calendar_content_sha256)
    .bind(&stream.window_sha256)
    .bind(stream.producer_epoch)
    .bind(demand.subscription_revision)
    .execute(&mut *tx)
    .await
    .map_err(|_| "synthetic live cache row could not be seeded")?;

    tx.commit()
        .await
        .map_err(|_| "synthetic quote fixture could not commit")?;
    Ok(())
}

pub async fn seed_active_lease_overflow(
    harness: &Harness,
    stream: &StreamFixture,
) -> Result<Vec<Uuid>, &'static str> {
    let mut tx = harness
        .owner_pool
        .begin()
        .await
        .map_err(|_| "lease overflow fixture transaction could not start")?;
    sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
        .bind(stream.owner_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|_| "lease overflow fixture actor context failed")?;
    let mut ids = Vec::with_capacity(21);
    for _ in 0..21 {
        let lease_id = Uuid::new_v4();
        ids.push(lease_id);
        sqlx::query(
            "INSERT INTO public.owner_market_stream_leases (
                 id, owner_user_id, consumer_id, session_hash, kind,
                 lease_expires_at, idempotency_key_sha256, request_sha256
             ) VALUES (
                 $1, $2, $3, $4, 'BROWSER', pg_catalog.clock_timestamp() + INTERVAL '1 hour',
                 'sha256:' || pg_catalog.repeat('f', 64),
                 'sha256:' || pg_catalog.repeat('f', 64)
             )",
        )
        .bind(lease_id)
        .bind(stream.owner_id)
        .bind(Uuid::new_v4())
        .bind(&stream.session_hash)
        .execute(&mut *tx)
        .await
        .map_err(|_| "active lease overflow row could not be seeded")?;
    }
    tx.commit()
        .await
        .map_err(|_| "active lease overflow fixture could not commit")?;
    Ok(ids)
}

pub async fn remove_active_lease_overflow(
    harness: &Harness,
    owner_id: Uuid,
    ids: &[Uuid],
) -> Result<(), &'static str> {
    sqlx::query(
        "DELETE FROM public.owner_market_stream_leases
          WHERE owner_user_id = $1 AND id = ANY($2::uuid[])",
    )
    .bind(owner_id)
    .bind(ids)
    .execute(&harness.owner_pool)
    .await
    .map_err(|_| "active lease overflow fixture could not be removed")?;
    Ok(())
}

pub async fn seed_identity_overflow(
    harness: &Harness,
    stream: &StreamFixture,
) -> Result<(), &'static str> {
    let mut tx = harness
        .owner_pool
        .begin()
        .await
        .map_err(|_| "identity overflow fixture transaction could not start")?;
    sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
        .bind(stream.owner_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|_| "identity overflow fixture actor context failed")?;
    let lease_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO public.owner_market_stream_leases (
             id, owner_user_id, consumer_id, session_hash, kind,
             lease_expires_at, idempotency_key_sha256, request_sha256
         ) VALUES (
             $1, $2, $3, $4, 'BROWSER', pg_catalog.clock_timestamp() + INTERVAL '1 hour',
             'sha256:' || pg_catalog.repeat('e', 64),
             'sha256:' || pg_catalog.repeat('e', 64)
         )",
    )
    .bind(lease_id)
    .bind(stream.owner_id)
    .bind(Uuid::new_v4())
    .bind(&stream.session_hash)
    .execute(&mut *tx)
    .await
    .map_err(|_| "identity overflow lease could not be seeded")?;

    let entitlement_hash = format!("sha256:{}", "e".repeat(64));
    for number in 1..=31 {
        let instrument_id = format!("8{number:05}.KRX");
        let membership_id = Uuid::new_v4();
        let generation_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO public.owner_equity_memberships (
                 id, owner_user_id, instrument_id, state, transition_actor_user_id,
                 transition_code_commit, transition_entitlement_sha256
             ) VALUES ($1, $2, $3, 'REQUESTED', $2, 'c3295be', $4)",
        )
        .bind(membership_id)
        .bind(stream.owner_id)
        .bind(&instrument_id)
        .bind(&entitlement_hash)
        .execute(&mut *tx)
        .await
        .map_err(|_| "identity overflow membership request failed")?;
        for state in ["VALIDATING", "BACKFILLING", "MATERIALIZING"] {
            sqlx::query(
                "UPDATE public.owner_equity_memberships
                    SET state = $2, transition_actor_user_id = $3,
                        transition_code_commit = 'c3295be',
                        transition_entitlement_sha256 = $4
                  WHERE id = $1",
            )
            .bind(membership_id)
            .bind(state)
            .bind(stream.owner_id)
            .bind(&entitlement_hash)
            .execute(&mut *tx)
            .await
            .map_err(|_| "identity overflow membership transition failed")?;
        }
        sqlx::query(
            "INSERT INTO public.owner_equity_instrument_generations (
                 id, membership_id, owner_user_id, instrument_id, generation,
                 target_observed_sessions, minimum_observed_sessions,
                 observed_sessions, first_session, last_session
             ) VALUES ($1, $2, $3, $4, 1, 121, 121, 121, $5, $5)",
        )
        .bind(generation_id)
        .bind(membership_id)
        .bind(stream.owner_id)
        .bind(&instrument_id)
        .bind(stream.session_date)
        .execute(&mut *tx)
        .await
        .map_err(|_| "identity overflow generation could not be seeded")?;
        sqlx::query(
            "INSERT INTO public.owner_equity_generation_admissions (
                 generation_id, owner_user_id, membership_id, instrument_id, generation,
                 raw_manifest_sha256, artifact_manifest_sha256, entitlement_sha256,
                 capture_code_commit, materializer_code_commit
             ) VALUES ($1, $2, $3, $4, 1, $5, $5, $5, 'c3295be', 'c3295be')",
        )
        .bind(generation_id)
        .bind(stream.owner_id)
        .bind(membership_id)
        .bind(&instrument_id)
        .bind(&entitlement_hash)
        .execute(&mut *tx)
        .await
        .map_err(|_| "identity overflow admission could not be seeded")?;
        sqlx::query(
            "UPDATE public.owner_equity_memberships
                SET state = 'READY', transition_actor_user_id = $2,
                    transition_code_commit = 'c3295be',
                    transition_entitlement_sha256 = $3
              WHERE id = $1",
        )
        .bind(membership_id)
        .bind(stream.owner_id)
        .bind(&entitlement_hash)
        .execute(&mut *tx)
        .await
        .map_err(|_| "identity overflow membership could not become READY")?;
        sqlx::query(
            "INSERT INTO public.owner_market_stream_lease_items (
                 lease_id, owner_user_id, membership_id, generation_id,
                 instrument_id, generation
             ) VALUES ($1, $2, $3, $4, $5, 1)",
        )
        .bind(lease_id)
        .bind(stream.owner_id)
        .bind(membership_id)
        .bind(generation_id)
        .bind(instrument_id)
        .execute(&mut *tx)
        .await
        .map_err(|_| "identity overflow lease item could not be seeded")?;
    }
    tx.commit()
        .await
        .map_err(|_| "identity overflow fixture could not commit")?;
    Ok(())
}

pub async fn seed_additional_owner(harness: &Harness, label: &str) -> UserCtx {
    let email = format!("status-{label}@lagrange.test");
    harness
        .seed_user(
            Role::Owner,
            &email,
            &format!("status-{label}-issuer"),
            &format!("status-{label}-subject"),
        )
        .await
}

pub async fn call_stream(
    pool: &PgPool,
    fixture: &StreamFixture,
    owner_id: Uuid,
    session_hash: &str,
    window_hash: &str,
) -> Result<Option<sqlx::postgres::PgRow>, &'static str> {
    call_stream_with_bounds(
        pool,
        fixture,
        owner_id,
        session_hash,
        window_hash,
        Some(fixture.open_at),
        Some(fixture.close_at),
    )
    .await
}

pub async fn call_stream_with_bounds(
    pool: &PgPool,
    fixture: &StreamFixture,
    owner_id: Uuid,
    session_hash: &str,
    window_hash: &str,
    open_at: Option<DateTime<Utc>>,
    close_at: Option<DateTime<Utc>>,
) -> Result<Option<sqlx::postgres::PgRow>, &'static str> {
    sqlx::query("SELECT * FROM public.owner_service_status_stream($1,$2,$3,$4,$5,$6,$7,$8)")
        .bind(owner_id)
        .bind(session_hash)
        .bind(fixture.slot_id)
        .bind(fixture.grant_id)
        .bind(&fixture.contract_sha256)
        .bind(window_hash)
        .bind(open_at)
        .bind(close_at)
        .fetch_optional(pool)
        .await
        .map_err(|_| "stream observation query failed")
}

pub async fn role_pool(url: &str) -> PgPool {
    let options = url
        .parse::<PgConnectOptions>()
        .expect("fixture role URL parses")
        .log_statements(tracing::log::LevelFilter::Off);
    PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(options)
        .await
        .expect("fixture role pool connects")
}

/// Test-only fixture updates immediately precede the real 0058 function inside
/// one server call. Its captured DB clock is never replaced or passed by callers.
/// Half-second margins and the measured observation delay bound distinguish the
/// inside/outside cases without sleeps; this does not prove exact TTL equality.
/// Immediately after KST midnight, an age-valid quote can still predate this
/// session. Assert that independent denial instead of expecting previous-day live.
pub async fn check_quote_sql_boundaries(
    harness: &Harness,
    app: &PgPool,
    stream: &StreamFixture,
    demand: &DemandFixture,
) -> Result<(), &'static str> {
    sqlx::raw_sql(
        "CREATE FUNCTION public.service_status_test_quote_timing(
             p_owner uuid, p_session text, p_slot uuid, p_grant uuid,
             p_contract text, p_window text, p_open timestamptz, p_close timestamptz,
             p_membership uuid, p_heartbeat_age_ms integer, p_lease_ms integer,
             p_received_age_ms integer, p_provider_age_ms integer, p_proof uuid
         ) RETURNS TABLE (
             seeded_at timestamptz, observed_at timestamptz,
             demand_count bigint, live_count bigint
         ) LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog
         AS $timing$
         DECLARE fixture_clock timestamptz := pg_catalog.clock_timestamp();
                 provider_at timestamptz := fixture_clock
                     - p_provider_age_ms * INTERVAL '1 millisecond';
         BEGIN
             UPDATE public.owner_market_stream_producers
                SET heartbeat_at = fixture_clock - p_heartbeat_age_ms * INTERVAL '1 millisecond',
                    lease_expires_at = fixture_clock + p_lease_ms * INTERVAL '1 millisecond'
              WHERE credential_slot_id = p_slot AND owner_user_id = p_owner;
             IF NOT FOUND THEN RAISE EXCEPTION 'synthetic producer absent'; END IF;
             UPDATE public.owner_market_stream_cache
                SET received_at = fixture_clock - p_received_age_ms * INTERVAL '1 millisecond',
                    committed_at = fixture_clock - p_received_age_ms * INTERVAL '1 millisecond'
                                                + INTERVAL '100 milliseconds',
                    provider_trade_at = provider_at,
                    business_date = (provider_at AT TIME ZONE 'Asia/Seoul')::date,
                    trade_time = (provider_at AT TIME ZONE 'Asia/Seoul')::time,
                    session_proof_id = p_proof
              WHERE owner_user_id = p_owner AND membership_id = p_membership;
             IF NOT FOUND THEN RAISE EXCEPTION 'synthetic quote absent'; END IF;
             RETURN QUERY
                 SELECT fixture_clock, status.observed_at, status.demand_count, status.live_count
                   FROM public.owner_service_status_stream(
                       p_owner, p_session, p_slot, p_grant, p_contract, p_window, p_open, p_close
                   ) AS status;
         END
         $timing$;
         REVOKE ALL ON FUNCTION public.service_status_test_quote_timing(
             uuid,text,uuid,uuid,text,text,timestamptz,timestamptz,
             uuid,integer,integer,integer,integer,uuid
         ) FROM PUBLIC;
         GRANT EXECUTE ON FUNCTION public.service_status_test_quote_timing(
             uuid,text,uuid,uuid,text,text,timestamptz,timestamptz,
             uuid,integer,integer,integer,integer,uuid
         ) TO app",
    )
    .execute(&harness.owner_pool)
    .await
    .map_err(|_| "isolated quote timing fixture could not be installed")?;

    let different_proof = Uuid::new_v4();
    for (reason, heartbeat_ms, lease_ms, receipt_ms, provider_ms, proof, expected_live) in [
        (
            "baseline current quote was not live",
            500,
            120_000,
            1_000,
            1_000,
            stream.session_proof_id,
            1,
        ),
        (
            "different nonnil proof ID with same hash stayed live",
            500,
            120_000,
            1_000,
            1_000,
            different_proof,
            0,
        ),
        (
            "restoring exact proof ID did not restore live count",
            500,
            120_000,
            1_000,
            1_000,
            stream.session_proof_id,
            1,
        ),
        (
            "heartbeat just inside 10s was not live",
            9_500,
            120_000,
            1_000,
            1_000,
            stream.session_proof_id,
            1,
        ),
        (
            "heartbeat just outside 10s stayed live",
            10_500,
            120_000,
            1_000,
            1_000,
            stream.session_proof_id,
            0,
        ),
        (
            "provider event just inside 30s was not live",
            500,
            120_000,
            1_000,
            29_500,
            stream.session_proof_id,
            1,
        ),
        (
            "provider event just outside 30s stayed live",
            500,
            120_000,
            1_000,
            30_500,
            stream.session_proof_id,
            0,
        ),
        (
            "receipt just inside 30s was not live",
            500,
            120_000,
            29_500,
            29_000,
            stream.session_proof_id,
            1,
        ),
        (
            "receipt just outside 30s stayed live",
            500,
            120_000,
            30_500,
            29_000,
            stream.session_proof_id,
            0,
        ),
        (
            "producer lease just above now+5s was not live",
            500,
            5_500,
            1_000,
            1_000,
            stream.session_proof_id,
            1,
        ),
        (
            "producer lease just below now+5s stayed live",
            500,
            4_500,
            1_000,
            1_000,
            stream.session_proof_id,
            0,
        ),
        (
            "restoring baseline timing did not restore live count",
            500,
            120_000,
            1_000,
            1_000,
            stream.session_proof_id,
            1,
        ),
    ] {
        let row = sqlx::query(
            "SELECT * FROM public.service_status_test_quote_timing(
                 $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14
             )",
        )
        .bind(stream.owner_id)
        .bind(&stream.session_hash)
        .bind(stream.slot_id)
        .bind(stream.grant_id)
        .bind(&stream.contract_sha256)
        .bind(&stream.window_sha256)
        .bind(stream.open_at)
        .bind(stream.close_at)
        .bind(demand.membership_id)
        .bind(heartbeat_ms)
        .bind(lease_ms)
        .bind(receipt_ms)
        .bind(provider_ms)
        .bind(proof)
        .fetch_one(app)
        .await
        .map_err(|_| "actual SQL timing/proof fixture execution failed")?;
        let seeded_at: DateTime<Utc> = row_value(&row, "seeded_at");
        let observed_at: DateTime<Utc> = row_value(&row, "observed_at");
        let elapsed = observed_at - seeded_at;
        if elapsed < chrono::Duration::zero() || elapsed >= chrono::Duration::milliseconds(250) {
            return Err("SQL fixture-to-observation delay exceeded the verified boundary margin");
        }
        if row_value::<i64>(&row, "demand_count") != 1 {
            return Err("SQL timing or proof invalidity removed the demanded identity");
        }
        let before_open = seeded_at - chrono::Duration::milliseconds(i64::from(provider_ms))
            < stream.open_at
            || seeded_at - chrono::Duration::milliseconds(i64::from(receipt_ms)) < stream.open_at;
        if before_open {
            if row_value::<i64>(&row, "live_count") != 0 {
                return Err("quote before the current session open became live");
            }
            eprintln!("SERVICE_STATUS_TIMING_CASE:BEFORE_OPEN_DENIED");
            continue;
        }
        if row_value::<i64>(&row, "live_count") != expected_live {
            return Err(reason);
        }
        eprintln!("SERVICE_STATUS_TIMING_CASE:IN_SESSION_EXPECTATION_MATCHED");
    }
    sqlx::raw_sql(
        "DROP FUNCTION public.service_status_test_quote_timing(
             uuid,text,uuid,uuid,text,text,timestamptz,timestamptz,
             uuid,integer,integer,integer,integer,uuid
         )",
    )
    .execute(&harness.owner_pool)
    .await
    .map_err(|_| "isolated quote timing fixture cleanup failed")?;
    Ok(())
}

pub fn row_value<T>(row: &sqlx::postgres::PgRow, column: &str) -> T
where
    for<'row> T: sqlx::Decode<'row, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>,
{
    row.try_get(column)
        .expect("stream aggregate has contract column")
}

/// Reuse the second current READY identity already seeded for excluded sessions;
/// adding it to the authenticated lease proves the real two-identity denominator.
pub async fn seed_second_demand_fixture(
    harness: &Harness,
    stream: &StreamFixture,
    first: &DemandFixture,
) -> Result<DemandFixture, &'static str> {
    let mut tx = harness
        .owner_pool
        .begin()
        .await
        .map_err(|_| "second demand transaction failed")?;
    sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
        .bind(stream.owner_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|_| "second demand actor context failed")?;
    let (membership_id, generation_id): (Uuid, Uuid) = sqlx::query_as(
        "SELECT membership.id, admission.generation_id
           FROM public.owner_equity_memberships AS membership
           JOIN public.owner_equity_generation_admissions AS admission
             ON admission.membership_id = membership.id
            AND admission.owner_user_id = membership.owner_user_id
          WHERE membership.owner_user_id = $1 AND membership.instrument_id = '069501.KRX'
            AND membership.state = 'READY' AND admission.generation = 1",
    )
    .bind(stream.owner_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|_| "second ready identity absent")?;
    sqlx::query(
        "INSERT INTO public.owner_market_stream_lease_items (
             lease_id, owner_user_id, membership_id, generation_id, instrument_id, generation
         ) VALUES ($1,$2,$3,$4,'069501.KRX',1)",
    )
    .bind(first.original_lease_id)
    .bind(stream.owner_id)
    .bind(membership_id)
    .bind(generation_id)
    .execute(&mut *tx)
    .await
    .map_err(|_| "second authenticated demand item failed")?;
    tx.commit()
        .await
        .map_err(|_| "second demand commit failed")?;
    Ok(DemandFixture {
        membership_id,
        generation_id,
        instrument_id: "069501.KRX",
        subscription_revision: Uuid::new_v4(),
        original_lease_id: first.original_lease_id,
    })
}

/// Explicit synthetic persisted evidence, not a replacement production clock.
/// Each mutation uses one clock_timestamp() from the disposable database. Actual
/// G1 reads still run independently under session_user=app and their own clock.
#[derive(Clone, Copy)]
pub struct EvidenceCase {
    pub heartbeat_age_ms: i32,
    pub lease_remaining_ms: i32,
    pub received_age_ms: [i32; 2],
    pub provider_age_ms: [i32; 2],
    pub halted: [bool; 2],
    pub connection: &'static str,
    pub gap: bool,
}

impl Default for EvidenceCase {
    fn default() -> Self {
        Self {
            heartbeat_age_ms: 500,
            lease_remaining_ms: 120_000,
            received_age_ms: [1_000, 1_000],
            provider_age_ms: [1_000, 1_000],
            halted: [true, true],
            connection: "CONNECTED",
            gap: false,
        }
    }
}

pub async fn set_stream_evidence(
    harness: &Harness,
    stream: &StreamFixture,
    demands: &[DemandFixture],
    case: EvidenceCase,
) -> Result<DateTime<Utc>, &'static str> {
    let mut tx = harness
        .owner_pool
        .begin()
        .await
        .map_err(|_| "evidence fixture transaction failed")?;
    sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
        .bind(stream.owner_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|_| "evidence fixture actor context failed")?;
    let seeded_at: DateTime<Utc> = sqlx::query_scalar("SELECT pg_catalog.clock_timestamp()")
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| "evidence fixture DB clock failed")?;
    let updated = sqlx::query(
        "UPDATE public.owner_market_stream_producers
            SET heartbeat_at = $2 - $3 * INTERVAL '1 millisecond',
                lease_expires_at = $2 + $4 * INTERVAL '1 millisecond',
                connection_state = $5, status_code = NULL, status_at = NULL,
                gap_since = CASE WHEN $6 THEN $2 ELSE NULL END, session_has_gap = $6
          WHERE credential_slot_id = $1",
    )
    .bind(stream.slot_id)
    .bind(seeded_at)
    .bind(case.heartbeat_age_ms)
    .bind(case.lease_remaining_ms)
    .bind(case.connection)
    .bind(case.gap)
    .execute(&mut *tx)
    .await
    .map_err(|_| "synthetic producer evidence update failed")?;
    if updated.rows_affected() != 1 {
        return Err("synthetic producer evidence update did not affect exactly one row");
    }
    if demands.len() > 2 {
        return Err("bounded evidence fixture supports exactly two identities at most");
    }
    for (i, demand) in demands.iter().enumerate() {
        let updated = sqlx::query(
            "UPDATE public.owner_market_stream_cache
                SET received_at = $3 - $4 * INTERVAL '1 millisecond',
                    committed_at = $3 - $4 * INTERVAL '1 millisecond' + INTERVAL '100 milliseconds',
                    provider_trade_at = $3 - $5 * INTERVAL '1 millisecond',
                    business_date = (($3 - $5 * INTERVAL '1 millisecond') AT TIME ZONE 'Asia/Seoul')::date,
                    trade_time = (($3 - $5 * INTERVAL '1 millisecond') AT TIME ZONE 'Asia/Seoul')::time,
                    halted = $6
              WHERE owner_user_id = $1 AND membership_id = $2",
        ).bind(stream.owner_id).bind(demand.membership_id).bind(seeded_at)
            .bind(case.received_age_ms[i]).bind(case.provider_age_ms[i]).bind(case.halted[i])
            .execute(&mut *tx).await.map_err(|_| "synthetic quote evidence update failed")?;
        if updated.rows_affected() != 1 {
            return Err("synthetic quote evidence update did not affect exactly one row");
        }
    }
    tx.commit()
        .await
        .map_err(|_| "evidence fixture commit failed")?;
    Ok(seeded_at)
}

pub fn check_counts(
    row: &sqlx::postgres::PgRow,
    demand: i64,
    live: i64,
    halted: i64,
) -> Result<(), &'static str> {
    if row_value::<i64>(row, "demand_count") != demand
        || row_value::<i64>(row, "live_count") != live
        || row_value::<i64>(row, "halted_count") != halted
    {
        return Err("actual G1 inclusive coverage/subset/denominator differed");
    }
    Ok(())
}

pub fn check_observation_delay(
    row: &sqlx::postgres::PgRow,
    seeded_at: DateTime<Utc>,
) -> Result<(), &'static str> {
    let observed_at: DateTime<Utc> = row_value(row, "observed_at");
    let elapsed = observed_at - seeded_at;
    if elapsed < chrono::Duration::zero() || elapsed >= chrono::Duration::milliseconds(500) {
        return Err("actual G1 observation exceeded the explicit 500ms fixture margin");
    }
    Ok(())
}

pub async fn assert_raw_source_clocks(
    harness: &Harness,
    stream: &StreamFixture,
    row: &sqlx::postgres::PgRow,
) -> Result<(), &'static str> {
    let stored = sqlx::query(
        "SELECT producer.heartbeat_at, producer.lease_expires_at, producer.status_at,
                (SELECT min(received_at) FROM public.owner_market_stream_cache WHERE owner_user_id=$1) AS oldest_received_at,
                (SELECT min(provider_trade_at) FROM public.owner_market_stream_cache WHERE owner_user_id=$1) AS oldest_provider_trade_at,
                (SELECT max(received_at) FROM public.owner_market_stream_cache WHERE owner_user_id=$1) AS last_received_at
           FROM public.owner_market_stream_producers AS producer WHERE producer.credential_slot_id=$2",
    ).bind(stream.owner_id).bind(stream.slot_id).fetch_one(&harness.owner_pool).await
        .map_err(|_| "persisted diagnostic clocks could not be read")?;
    for (aggregate, column) in [
        ("producer_heartbeat_at", "heartbeat_at"),
        ("producer_lease_expires_at", "lease_expires_at"),
        ("producer_status_at", "status_at"),
        ("oldest_received_at", "oldest_received_at"),
        ("oldest_provider_trade_at", "oldest_provider_trade_at"),
        ("last_received_at", "last_received_at"),
    ] {
        if row_value::<Option<DateTime<Utc>>>(row, aggregate)
            != row_value::<Option<DateTime<Utc>>>(&stored, column)
        {
            return Err("G1 replaced a persisted diagnostic clock with observation time");
        }
    }
    Ok(())
}

pub fn negative_observation_until(
    row: &sqlx::postgres::PgRow,
    stream: &StreamFixture,
    null_bounds: bool,
) -> DateTime<Utc> {
    let observed: DateTime<Utc> = row_value(row, "observed_at");
    let midnight = (stream.session_date + chrono::Duration::days(1))
        .and_hms_opt(0, 0, 0)
        .expect("midnight")
        .and_local_timezone(chrono::FixedOffset::east_opt(9 * 3600).expect("KST"))
        .single()
        .expect("fixed KST midnight")
        .with_timezone(&Utc);
    let mut until = (observed + chrono::Duration::seconds(30)).min(midnight);
    if !null_bounds {
        if stream.open_at > observed {
            until = until.min(stream.open_at);
        }
        if stream.close_at > observed {
            until = until.min(stream.close_at);
        }
    }
    until
}

pub async fn repin_fixture_window(
    harness: &Harness,
    stream: &StreamFixture,
    close_at: DateTime<Utc>,
) -> Result<StreamFixture, &'static str> {
    let mut updated = stream.clone();
    updated.close_at = close_at;
    let proof = format!(
        "owner-market-stream-day-v1\n{}\n{}\n{}\n{}\n{}\n{}\n",
        stream.session_date.format("%Y-%m-%d"),
        stream.calendar_batch_id,
        stream.calendar_content_sha256,
        stream.window_sha256,
        stream.open_at.format("%Y-%m-%dT%H:%M:%SZ"),
        close_at.format("%Y-%m-%dT%H:%M:%SZ")
    );
    updated.session_proof_sha256 = sqlx::query_scalar(
        "SELECT 'sha256:' || pg_catalog.encode(pg_catalog.sha256(pg_catalog.convert_to($1,'UTF8')),'hex')",
    ).bind(proof).fetch_one(&harness.owner_pool).await.map_err(|_| "repinned fixture proof digest failed")?;
    let mut tx = harness
        .owner_pool
        .begin()
        .await
        .map_err(|_| "window proof fixture transaction failed")?;
    sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
        .bind(stream.owner_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|_| "window proof actor failed")?;
    sqlx::query("UPDATE public.owner_market_stream_producers SET session_proof_sha256=$2 WHERE credential_slot_id=$1")
        .bind(stream.slot_id).bind(&updated.session_proof_sha256).execute(&mut *tx).await
        .map_err(|_| "producer window proof update failed")?;
    sqlx::query("UPDATE public.owner_market_stream_cache SET session_proof_sha256=$2 WHERE owner_user_id=$1")
        .bind(stream.owner_id).bind(&updated.session_proof_sha256).execute(&mut *tx).await
        .map_err(|_| "quote window proof update failed")?;
    tx.commit()
        .await
        .map_err(|_| "window proof commit failed")?;
    Ok(updated)
}

/// Append a real newer admission; never rewrite or delete the old generation.
/// The existing demanded/cache identity must remain in the denominator but lose
/// coverage through the production anti-join predicate.
pub async fn seed_newer_admission_fixture(
    harness: &Harness,
    stream: &StreamFixture,
    demand: &DemandFixture,
) -> Result<(), &'static str> {
    let mut tx = harness
        .owner_pool
        .begin()
        .await
        .map_err(|_| "newer admission transaction failed")?;
    sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
        .bind(stream.owner_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|_| "newer admission actor context failed")?;
    let newer = Uuid::new_v4();
    let inserted = sqlx::query(
        "INSERT INTO public.owner_equity_instrument_generations (
             id, membership_id, owner_user_id, instrument_id, generation,
             target_observed_sessions, minimum_observed_sessions, observed_sessions, first_session, last_session
         ) SELECT $1, membership_id, owner_user_id, instrument_id, generation+1,
                  target_observed_sessions, minimum_observed_sessions, observed_sessions, first_session, last_session
             FROM public.owner_equity_instrument_generations WHERE id=$2 AND owner_user_id=$3",
    ).bind(newer).bind(demand.generation_id).bind(stream.owner_id).execute(&mut *tx).await
        .map_err(|_| "newer instrument generation insertion failed")?;
    if inserted.rows_affected() != 1 {
        return Err("newer generation fixture did not copy one existing identity");
    }
    sqlx::query(
        "INSERT INTO public.owner_equity_generation_admissions (
             generation_id, owner_user_id, membership_id, instrument_id, generation,
             raw_manifest_sha256, artifact_manifest_sha256, entitlement_sha256,
             capture_code_commit, materializer_code_commit
         ) VALUES ($1,$2,$3,$4,2,$5,$5,$5,'c3295be','c3295be')",
    )
    .bind(newer)
    .bind(stream.owner_id)
    .bind(demand.membership_id)
    .bind(demand.instrument_id)
    .bind(format!("sha256:{}", "e".repeat(64)))
    .execute(&mut *tx)
    .await
    .map_err(|_| "newer current admission insertion failed")?;
    tx.commit()
        .await
        .map_err(|_| "newer admission fixture commit failed")?;
    Ok(())
}
