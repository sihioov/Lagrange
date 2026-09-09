#[allow(dead_code)]
mod intraday_quotes_support;

use chrono::Days;
use intraday_quotes_support::{
    IntradayTestDb, MembershipFixture, run_body, wait_until_database_time,
};
use job_queue::owner_equity_v2::{
    DemandMutationKind, DemandReleaseKind, IntradayQuoteDemandRequest, IntradayQuoteReleaseRequest,
    IntradayStorageError,
};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

fn demand_request(
    membership_id: Uuid,
    consumer_id: Uuid,
    generation: u64,
    sequence: u64,
    key: &str,
) -> IntradayQuoteDemandRequest {
    IntradayQuoteDemandRequest::new(
        consumer_id,
        membership_id,
        generation,
        sequence,
        key.to_owned(),
    )
    .expect("synthetic demand request is valid")
}

fn release_request(consumer_id: Uuid, sequence: u64, key: &str) -> IntradayQuoteReleaseRequest {
    IntradayQuoteReleaseRequest::new(consumer_id, sequence, key.to_owned())
        .expect("synthetic release request is valid")
}

async fn fingerprint_rows(pool: &PgPool, owner_user_id: Uuid) -> Result<[Value; 5], String> {
    let demands: Value = sqlx::query_scalar(
        "SELECT COALESCE(
                    jsonb_agg(pg_catalog.to_jsonb(demand) ORDER BY demand.id),
                    '[]'::jsonb
                )
           FROM public.owner_intraday_quote_demands AS demand
          WHERE demand.owner_user_id = $1",
    )
    .bind(owner_user_id)
    .fetch_one(pool)
    .await
    .map_err(|_| "could not fingerprint demand rows".to_owned())?;
    let cache: Value = sqlx::query_scalar(
        "SELECT COALESCE(
                    jsonb_agg(pg_catalog.to_jsonb(cache) ORDER BY cache.membership_id),
                    '[]'::jsonb
                )
           FROM public.owner_intraday_quote_cache AS cache
          WHERE cache.owner_user_id = $1",
    )
    .bind(owner_user_id)
    .fetch_one(pool)
    .await
    .map_err(|_| "could not fingerprint cache rows".to_owned())?;
    let producers: Value = sqlx::query_scalar(
        "SELECT COALESCE(
                    jsonb_agg(pg_catalog.to_jsonb(producer) ORDER BY producer.owner_user_id),
                    '[]'::jsonb
                )
           FROM public.owner_intraday_quote_producers AS producer
          WHERE producer.owner_user_id = $1",
    )
    .bind(owner_user_id)
    .fetch_one(pool)
    .await
    .map_err(|_| "could not fingerprint producer rows".to_owned())?;
    let memberships: Value = sqlx::query_scalar(
        "SELECT COALESCE(
                    jsonb_agg(pg_catalog.to_jsonb(membership) ORDER BY membership.id),
                    '[]'::jsonb
                )
           FROM public.owner_equity_memberships AS membership
          WHERE membership.owner_user_id = $1",
    )
    .bind(owner_user_id)
    .fetch_one(pool)
    .await
    .map_err(|_| "could not fingerprint membership rows".to_owned())?;
    let admissions: Value = sqlx::query_scalar(
        "SELECT COALESCE(
                    jsonb_agg(
                        pg_catalog.to_jsonb(admission)
                        ORDER BY admission.generation_id
                    ),
                    '[]'::jsonb
                )
           FROM public.owner_equity_generation_admissions AS admission
          WHERE admission.owner_user_id = $1",
    )
    .bind(owner_user_id)
    .fetch_one(pool)
    .await
    .map_err(|_| "could not fingerprint admission rows".to_owned())?;
    Ok([demands, cache, producers, memberships, admissions])
}

async fn seed_minimal_operational_rows(
    db: &IntradayTestDb,
    fixture: &MembershipFixture,
) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO public.owner_intraday_quote_cache
            (owner_user_id, membership_id, generation_id, instrument_id,
             generation, quote_version, last_attempt_at, producer_fence)
         VALUES ($1, $2, $3, $4, $5, 0, pg_catalog.clock_timestamp(), 0)",
    )
    .bind(fixture.owner_user_id)
    .bind(fixture.membership_id)
    .bind(fixture.generation_id)
    .bind(&fixture.instrument_id)
    .bind(i64::try_from(fixture.generation).expect("fixture generation fits bigint"))
    .execute(&db.superuser)
    .await
    .map_err(|_| "could not seed minimal cache row".to_owned())?;

    let holder_id = Uuid::new_v4();
    sqlx::query(
        "WITH observed AS (
                SELECT pg_catalog.clock_timestamp() AS observed_at
         )
         INSERT INTO public.owner_intraday_quote_producers
             (owner_user_id, holder_id, fencing_token,
              lease_expires_at, heartbeat_at, updated_at)
         SELECT $1, $2, 1, observed_at + INTERVAL '20 seconds',
                observed_at, observed_at
           FROM observed",
    )
    .bind(fixture.owner_user_id)
    .bind(holder_id)
    .execute(&db.superuser)
    .await
    .map_err(|_| "could not seed minimal producer row".to_owned())?;
    Ok(())
}

async fn seed_requested_membership(
    db: &IntradayTestDb,
    owner_user_id: Uuid,
    instrument_id: &str,
) -> Result<Uuid, String> {
    let membership_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO public.owner_equity_memberships
            (id, owner_user_id, instrument_id, state,
             transition_actor_user_id, transition_code_commit,
             transition_entitlement_sha256)
         VALUES ($1, $2, $3, 'REQUESTED', $2, $4, $5)",
    )
    .bind(membership_id)
    .bind(owner_user_id)
    .bind(instrument_id)
    .bind("c".repeat(40))
    .bind(format!("sha256:{}", "d".repeat(64)))
    .execute(&db.superuser)
    .await
    .map_err(|_| "could not seed requested membership".to_owned())?;
    Ok(membership_id)
}

async fn seed_newer_generation(
    db: &IntradayTestDb,
    fixture: &MembershipFixture,
) -> Result<Uuid, String> {
    let generation_id = Uuid::new_v4();
    let first_session = db
        .session_date
        .checked_sub_days(Days::new(120))
        .ok_or_else(|| "newer generation fixture date underflowed".to_owned())?;
    sqlx::query(
        "INSERT INTO public.owner_equity_instrument_generations
            (id, membership_id, owner_user_id, instrument_id, generation,
             target_observed_sessions, minimum_observed_sessions,
             observed_sessions, first_session, last_session)
         VALUES ($1, $2, $3, $4, 2, 261, 121, 121, $5, $6)",
    )
    .bind(generation_id)
    .bind(fixture.membership_id)
    .bind(fixture.owner_user_id)
    .bind(&fixture.instrument_id)
    .bind(first_session)
    .bind(db.session_date)
    .execute(&db.superuser)
    .await
    .map_err(|_| "could not seed newer generation".to_owned())?;
    sqlx::query(
        "INSERT INTO public.owner_equity_generation_admissions
            (generation_id, owner_user_id, membership_id, instrument_id,
             generation, raw_manifest_sha256, artifact_manifest_sha256,
             entitlement_sha256, capture_code_commit, materializer_code_commit)
         VALUES ($1, $2, $3, $4, 2, $5, $6, $7, $8, $8)",
    )
    .bind(generation_id)
    .bind(fixture.owner_user_id)
    .bind(fixture.membership_id)
    .bind(&fixture.instrument_id)
    .bind(format!("sha256:{}", "1".repeat(64)))
    .bind(format!("sha256:{}", "2".repeat(64)))
    .bind(format!("sha256:{}", "3".repeat(64)))
    .bind("c".repeat(40))
    .execute(&db.superuser)
    .await
    .map_err(|_| "could not seed newer admission".to_owned())?;
    Ok(generation_id)
}

async fn assert_app_producer_select_denied(
    db: &IntradayTestDb,
    owner_user_id: Uuid,
) -> Result<(), String> {
    let error = sqlx::query(
        "SELECT owner_user_id
           FROM public.owner_intraday_quote_producers
          WHERE owner_user_id = $1",
    )
    .bind(owner_user_id)
    .execute(&db.app)
    .await
    .expect_err("app producer SELECT unexpectedly succeeded");
    if !matches!(
        error,
        sqlx::Error::Database(database) if database.code().as_deref() == Some("42501")
    ) {
        return Err(
            "app producer SELECT failed for a reason other than permission denial".to_owned(),
        );
    }
    Ok(())
}

#[tokio::test]
async fn app_identity_read_reports_current_state_and_stays_read_only() {
    run_body(|db| async move {
        let owner = db.seed_owner("read-state").await?;
        let fixture = db.seed_ready_membership(owner, "005930.KRX").await?;
        let app = db.repository_as_app();

        let initial = app
            .read_current_identity_state(
                owner,
                fixture.membership_id,
                &fixture.instrument_id,
                fixture.generation,
            )
            .await
            .map_err(|error| format!("initial identity read failed: {error}"))?
            .ok_or_else(|| "READY current admission was not readable".to_owned())?;
        if initial.identity.owner_user_id != owner
            || initial.identity.membership_id != fixture.membership_id
            || initial.identity.generation_id != fixture.generation_id
            || initial.identity.instrument_id != fixture.instrument_id
            || initial.identity.generation != fixture.generation
            || initial.has_active_demand
        {
            return Err(
                "initial identity read did not return exact current Some(false)".to_owned(),
            );
        }
        let database_now: chrono::DateTime<chrono::Utc> =
            sqlx::query_scalar("SELECT pg_catalog.clock_timestamp()")
                .fetch_one(&db.superuser)
                .await
                .map_err(|_| "could not read QA database clock".to_owned())?;
        if initial.observed_at > database_now {
            return Err("identity read observed_at was ahead of the database clock".to_owned());
        }

        for (label, result) in [
            (
                "nil owner",
                app.read_current_identity_state(
                    Uuid::nil(),
                    fixture.membership_id,
                    &fixture.instrument_id,
                    fixture.generation,
                )
                .await,
            ),
            (
                "nil membership",
                app.read_current_identity_state(
                    owner,
                    Uuid::nil(),
                    &fixture.instrument_id,
                    fixture.generation,
                )
                .await,
            ),
            (
                "non-canonical instrument",
                app.read_current_identity_state(
                    owner,
                    fixture.membership_id,
                    "005930.krx",
                    fixture.generation,
                )
                .await,
            ),
            (
                "zero generation",
                app.read_current_identity_state(
                    owner,
                    fixture.membership_id,
                    &fixture.instrument_id,
                    0,
                )
                .await,
            ),
            (
                "bigint overflow generation",
                app.read_current_identity_state(
                    owner,
                    fixture.membership_id,
                    &fixture.instrument_id,
                    u64::MAX,
                )
                .await,
            ),
        ] {
            if result != Err(IntradayStorageError::InvalidInput) {
                return Err(format!("{label} did not produce InvalidInput"));
            }
        }

        let consumer = Uuid::new_v4();
        let request = demand_request(
            fixture.membership_id,
            consumer,
            fixture.generation,
            0,
            "identity-read-demand",
        );
        let created = app
            .create_or_renew_demand(owner, &request)
            .await
            .map_err(|error| format!("public demand setup failed: {error}"))?;
        if created.kind != DemandMutationKind::Created {
            return Err("public demand was not created".to_owned());
        }
        let demanded = app
            .read_current_identity_state(
                owner,
                fixture.membership_id,
                &fixture.instrument_id,
                fixture.generation,
            )
            .await
            .map_err(|error| format!("active-demand identity read failed: {error}"))?
            .ok_or_else(|| "identity disappeared after demand creation".to_owned())?;
        if !demanded.has_active_demand {
            return Err("active public demand was not reflected in identity state".to_owned());
        }

        let replay = app
            .create_or_renew_demand(owner, &request)
            .await
            .map_err(|error| format!("duplicate consumer demand failed: {error}"))?;
        if replay.kind != DemandMutationKind::Replayed {
            return Err("duplicate consumer demand did not replay".to_owned());
        }
        let duplicate_state = app
            .read_current_identity_state(
                owner,
                fixture.membership_id,
                &fixture.instrument_id,
                fixture.generation,
            )
            .await
            .map_err(|error| format!("duplicate-demand identity read failed: {error}"))?
            .ok_or_else(|| "identity disappeared after duplicate demand".to_owned())?;
        if !duplicate_state.has_active_demand {
            return Err("duplicate consumer changed active-demand state".to_owned());
        }

        let released = app
            .release_demand(
                owner,
                created.lease.demand_id,
                &release_request(consumer, 0, "identity-read-release"),
            )
            .await
            .map_err(|error| format!("public demand release failed: {error}"))?;
        if released.kind != DemandReleaseKind::Released {
            return Err("public demand was not released".to_owned());
        }
        let released_state = app
            .read_current_identity_state(
                owner,
                fixture.membership_id,
                &fixture.instrument_id,
                fixture.generation,
            )
            .await
            .map_err(|error| format!("released-demand identity read failed: {error}"))?
            .ok_or_else(|| "identity disappeared after demand release".to_owned())?;
        if released_state.has_active_demand {
            return Err("released demand remained active".to_owned());
        }

        let expired = app
            .create_or_renew_demand(
                owner,
                &demand_request(
                    fixture.membership_id,
                    Uuid::new_v4(),
                    fixture.generation,
                    0,
                    "identity-read-expired",
                ),
            )
            .await
            .map_err(|error| format!("expired-demand setup failed: {error}"))?;
        sqlx::query(
            "UPDATE public.owner_intraday_quote_demands
                SET created_at = pg_catalog.clock_timestamp() - INTERVAL '2 seconds',
                    lease_expires_at = pg_catalog.clock_timestamp() - INTERVAL '1 second',
                    updated_at = pg_catalog.clock_timestamp()
              WHERE id = $1",
        )
        .bind(expired.lease.demand_id)
        .execute(&db.superuser)
        .await
        .map_err(|_| "could not expire ACTIVE demand with database time".to_owned())?;
        let expired_at: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
            "SELECT lease_expires_at
               FROM public.owner_intraday_quote_demands
              WHERE id = $1",
        )
        .bind(expired.lease.demand_id)
        .fetch_one(&db.superuser)
        .await
        .map_err(|_| "could not observe expired demand".to_owned())?;
        wait_until_database_time(&db.superuser, expired_at).await?;
        let expired_state = app
            .read_current_identity_state(
                owner,
                fixture.membership_id,
                &fixture.instrument_id,
                fixture.generation,
            )
            .await
            .map_err(|error| format!("expired-demand identity read failed: {error}"))?
            .ok_or_else(|| "identity disappeared after demand expiry".to_owned())?;
        if expired_state.has_active_demand {
            return Err("expired ACTIVE demand remained active at database time".to_owned());
        }

        seed_minimal_operational_rows(&db, &fixture).await?;
        let before = fingerprint_rows(&db.superuser, owner).await?;
        assert_app_producer_select_denied(&db, owner).await?;
        for count in [1_usize, 10, 100] {
            for _ in 0..count {
                let state = app
                    .read_current_identity_state(
                        owner,
                        fixture.membership_id,
                        &fixture.instrument_id,
                        fixture.generation,
                    )
                    .await
                    .map_err(|error| format!("repeated identity read failed: {error}"))?
                    .ok_or_else(|| "repeated identity read returned None".to_owned())?;
                if state.has_active_demand {
                    return Err("repeated identity read revived expired demand".to_owned());
                }
            }
        }
        assert_app_producer_select_denied(&db, owner).await?;
        let after = fingerprint_rows(&db.superuser, owner).await?;
        if before != after {
            return Err(
                "identity reads changed an UPDATE-sensitive JSON row fingerprint".to_owned(),
            );
        }

        if app
            .read_current_identity_state(
                Uuid::new_v4(),
                fixture.membership_id,
                &fixture.instrument_id,
                fixture.generation,
            )
            .await
            .map_err(|error| format!("unknown-owner read failed: {error}"))?
            .is_some()
        {
            return Err("unknown owner received identity state".to_owned());
        }
        let foreign_owner = db.seed_owner("foreign-owner").await?;
        if app
            .read_current_identity_state(
                foreign_owner,
                fixture.membership_id,
                &fixture.instrument_id,
                fixture.generation,
            )
            .await
            .map_err(|error| format!("foreign-owner read failed: {error}"))?
            .is_some()
        {
            return Err("foreign owner received another owner's identity state".to_owned());
        }
        if app
            .read_current_identity_state(
                owner,
                Uuid::new_v4(),
                &fixture.instrument_id,
                fixture.generation,
            )
            .await
            .map_err(|error| format!("unknown-membership read failed: {error}"))?
            .is_some()
        {
            return Err("unknown membership received identity state".to_owned());
        }
        if app
            .read_current_identity_state(
                owner,
                fixture.membership_id,
                "005931.KRX",
                fixture.generation,
            )
            .await
            .map_err(|error| format!("instrument-mismatch read failed: {error}"))?
            .is_some()
        {
            return Err("instrument mismatch received identity state".to_owned());
        }
        if app
            .read_current_identity_state(owner, fixture.membership_id, &fixture.instrument_id, 2)
            .await
            .map_err(|error| format!("no-admission read failed: {error}"))?
            .is_some()
        {
            return Err("generation without an admission received identity state".to_owned());
        }

        let not_ready_owner = db.seed_owner("not-ready").await?;
        let not_ready_membership =
            seed_requested_membership(&db, not_ready_owner, "005932.KRX").await?;
        if app
            .read_current_identity_state(not_ready_owner, not_ready_membership, "005932.KRX", 1)
            .await
            .map_err(|error| format!("not-ready read failed: {error}"))?
            .is_some()
        {
            return Err("not-READY membership received identity state".to_owned());
        }

        let disabled_fixture = db
            .seed_ready_membership(foreign_owner, "005933.KRX")
            .await?;
        sqlx::query(
            "UPDATE public.owner_equity_memberships
                SET state = 'DISABLED', disabled_at = pg_catalog.clock_timestamp(),
                    updated_at = pg_catalog.clock_timestamp()
              WHERE id = $1",
        )
        .bind(disabled_fixture.membership_id)
        .execute(&db.superuser)
        .await
        .map_err(|_| "could not disable membership fixture".to_owned())?;
        if app
            .read_current_identity_state(
                foreign_owner,
                disabled_fixture.membership_id,
                &disabled_fixture.instrument_id,
                disabled_fixture.generation,
            )
            .await
            .map_err(|error| format!("disabled-membership read failed: {error}"))?
            .is_some()
        {
            return Err("disabled membership received identity state".to_owned());
        }

        let newer_generation_id = seed_newer_generation(&db, &fixture).await?;
        if app
            .read_current_identity_state(
                owner,
                fixture.membership_id,
                &fixture.instrument_id,
                fixture.generation,
            )
            .await
            .map_err(|error| format!("stale-generation read failed: {error}"))?
            .is_some()
        {
            return Err("old generation remained current after newer admission".to_owned());
        }
        let current_newer = app
            .read_current_identity_state(owner, fixture.membership_id, &fixture.instrument_id, 2)
            .await
            .map_err(|error| format!("new-generation read failed: {error}"))?
            .ok_or_else(|| "newer current admission was not readable".to_owned())?;
        if current_newer.identity.generation_id != newer_generation_id
            || current_newer.identity.generation != 2
            || current_newer.has_active_demand
        {
            return Err("newer current admission returned the wrong identity state".to_owned());
        }
        if app
            .read_current_identity_state(owner, fixture.membership_id, &fixture.instrument_id, 3)
            .await
            .map_err(|error| format!("unknown-generation read failed: {error}"))?
            .is_some()
        {
            return Err("unknown generation received identity state".to_owned());
        }
        Ok(())
    })
    .await;
}
