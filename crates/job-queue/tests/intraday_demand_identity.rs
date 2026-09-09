#[allow(dead_code)]
mod intraday_quotes_support;

use chrono::{DateTime, Days, Utc};
use intraday_quotes_support::{IntradayTestDb, MembershipFixture, run_body};
use job_queue::owner_equity_v2::{
    DemandMutationKind, DemandMutationOutcome, IntradayQuoteDemandRequest, IntradayStorageError,
};
use sqlx::FromRow;
use uuid::Uuid;

/// Complete durable demand-row observation used to prove rejected legacy
/// renewals leave the retained consumer record byte-for-byte equivalent at the
/// SQL-column boundary.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
struct RetainedDemandRow {
    id: Uuid,
    owner_user_id: Uuid,
    consumer_id: Uuid,
    membership_id: Uuid,
    generation_id: Uuid,
    instrument_id: String,
    generation: i64,
    state: String,
    renewal_sequence: i64,
    lease_expires_at: DateTime<Utc>,
    released_at: Option<DateTime<Utc>>,
    idempotency_key_sha256: String,
    request_sha256: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

fn demand_request(
    membership_id: Uuid,
    consumer_id: Uuid,
    generation: u64,
    renewal_sequence: u64,
    idempotency_key: &str,
) -> IntradayQuoteDemandRequest {
    IntradayQuoteDemandRequest::new(
        consumer_id,
        membership_id,
        generation,
        renewal_sequence,
        idempotency_key.to_owned(),
    )
    .expect("identity regression request is valid")
}

async fn retained_demand_row(
    db: &IntradayTestDb,
    demand_id: Uuid,
) -> Result<RetainedDemandRow, String> {
    sqlx::query_as(
        "SELECT id, owner_user_id, consumer_id, membership_id, generation_id,
                instrument_id, generation, state, renewal_sequence,
                lease_expires_at, released_at, idempotency_key_sha256,
                request_sha256, created_at, updated_at
           FROM public.owner_intraday_quote_demands
          WHERE id = $1",
    )
    .bind(demand_id)
    .fetch_one(&db.superuser)
    .await
    .map_err(|_| "could not observe the complete retained demand row".to_owned())
}

async fn assert_real_app_role(db: &IntradayTestDb) -> Result<(), String> {
    let current_user: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&db.app)
        .await
        .map_err(|_| "could not observe the app-role test connection".to_owned())?;
    if current_user != "app" {
        return Err(format!(
            "legacy demand mutation did not use app role: {current_user}"
        ));
    }
    Ok(())
}

async fn seed_newer_current_admission(
    db: &IntradayTestDb,
    fixture: &MembershipFixture,
) -> Result<u64, String> {
    let next_generation = fixture
        .generation
        .checked_add(1)
        .ok_or_else(|| "fixture generation overflowed".to_owned())?;
    let next_generation_i64 = i64::try_from(next_generation)
        .map_err(|_| "fixture generation did not fit PostgreSQL bigint".to_owned())?;
    let first_session = db
        .session_date
        .checked_sub_days(Days::new(120))
        .ok_or_else(|| "fixture first session underflowed".to_owned())?;
    let next_generation_id = Uuid::new_v4();
    let mut tx = db
        .migration_owner
        .begin()
        .await
        .map_err(|_| "could not begin migration-owner fixture transaction".to_owned())?;
    sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
        .bind(fixture.owner_user_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|_| "could not set migration-owner fixture actor".to_owned())?;
    sqlx::query(
        "INSERT INTO public.owner_equity_instrument_generations
            (id, membership_id, owner_user_id, instrument_id, generation,
             target_observed_sessions, minimum_observed_sessions,
             observed_sessions, first_session, last_session)
         VALUES ($1, $2, $3, $4, $5, 261, 121, 121, $6, $7)",
    )
    .bind(next_generation_id)
    .bind(fixture.membership_id)
    .bind(fixture.owner_user_id)
    .bind(&fixture.instrument_id)
    .bind(next_generation_i64)
    .bind(first_session)
    .bind(db.session_date)
    .execute(&mut *tx)
    .await
    .map_err(|_| "could not insert newer generation fixture".to_owned())?;
    sqlx::query(
        "INSERT INTO public.owner_equity_generation_admissions
            (generation_id, owner_user_id, membership_id, instrument_id,
             generation, raw_manifest_sha256, artifact_manifest_sha256,
             entitlement_sha256, capture_code_commit, materializer_code_commit)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9)",
    )
    .bind(next_generation_id)
    .bind(fixture.owner_user_id)
    .bind(fixture.membership_id)
    .bind(&fixture.instrument_id)
    .bind(next_generation_i64)
    .bind(format!("sha256:{}", "1".repeat(64)))
    .bind(format!("sha256:{}", "2".repeat(64)))
    .bind(format!("sha256:{}", "3".repeat(64)))
    .bind("c".repeat(40))
    .execute(&mut *tx)
    .await
    .map_err(|_| "could not insert newer admission fixture".to_owned())?;
    tx.commit()
        .await
        .map_err(|_| "could not commit newer admission fixture".to_owned())?;
    Ok(next_generation)
}

async fn disable_fixture_membership(
    db: &IntradayTestDb,
    fixture: &MembershipFixture,
) -> Result<(), String> {
    let mut tx = db
        .migration_owner
        .begin()
        .await
        .map_err(|_| "could not begin migration-owner disable fixture".to_owned())?;
    sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
        .bind(fixture.owner_user_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|_| "could not set disable fixture actor".to_owned())?;
    sqlx::query(
        "UPDATE public.owner_equity_memberships
            SET state = 'DISABLED', disabled_at = pg_catalog.clock_timestamp(),
                updated_at = pg_catalog.clock_timestamp()
          WHERE id = $1 AND owner_user_id = $2",
    )
    .bind(fixture.membership_id)
    .bind(fixture.owner_user_id)
    .execute(&mut *tx)
    .await
    .map_err(|_| "could not disable membership fixture".to_owned())?;
    tx.commit()
        .await
        .map_err(|_| "could not commit membership disable fixture".to_owned())?;
    Ok(())
}

fn assert_identity_mismatch_preserves_retained_row(
    scenario: &str,
    outcome: Result<DemandMutationOutcome, IntradayStorageError>,
    before: &RetainedDemandRow,
    after: &RetainedDemandRow,
) -> Result<(), String> {
    if outcome != Err(IntradayStorageError::IdentityMismatch) || after != before {
        return Err(format!(
            "{scenario}: expected IdentityMismatch and an unchanged complete retained row; \
             outcome={outcome:?}; before={before:?}; after={after:?}"
        ));
    }
    Ok(())
}

#[tokio::test]
async fn legacy_next_renewal_rejects_a_different_ready_membership_without_mutation() {
    run_body(|db| async move {
        let owner = db.seed_owner("legacy-different-membership").await?;
        let retained = db.seed_ready_membership(owner, "005930.KRX").await?;
        let requested = db.seed_ready_membership(owner, "000660.KRX").await?;
        assert_real_app_role(&db).await?;
        let app = db.repository_as_app();
        let consumer_id = Uuid::new_v4();
        let created = app
            .create_or_renew_demand(
                owner,
                &demand_request(
                    retained.membership_id,
                    consumer_id,
                    retained.generation,
                    0,
                    "legacy-different-membership-create",
                ),
            )
            .await
            .map_err(|error| format!("could not create retained legacy demand: {error}"))?;
        let before = retained_demand_row(&db, created.lease.demand_id).await?;
        let outcome = app
            .create_or_renew_demand(
                owner,
                &demand_request(
                    requested.membership_id,
                    consumer_id,
                    requested.generation,
                    1,
                    "legacy-different-membership-next",
                ),
            )
            .await;
        let after = retained_demand_row(&db, created.lease.demand_id).await?;
        assert_identity_mismatch_preserves_retained_row(
            "different READY membership next renewal",
            outcome,
            &before,
            &after,
        )
    })
    .await;
}

#[tokio::test]
async fn legacy_next_renewal_rejects_a_newer_current_admission_without_mutation() {
    run_body(|db| async move {
        let owner = db.seed_owner("legacy-newer-admission").await?;
        let retained = db.seed_ready_membership(owner, "005930.KRX").await?;
        assert_real_app_role(&db).await?;
        let app = db.repository_as_app();
        let consumer_id = Uuid::new_v4();
        let created = app
            .create_or_renew_demand(
                owner,
                &demand_request(
                    retained.membership_id,
                    consumer_id,
                    retained.generation,
                    0,
                    "legacy-newer-admission-create",
                ),
            )
            .await
            .map_err(|error| format!("could not create retained legacy demand: {error}"))?;
        let before = retained_demand_row(&db, created.lease.demand_id).await?;
        let current_generation = seed_newer_current_admission(&db, &retained).await?;
        let outcome = app
            .create_or_renew_demand(
                owner,
                &demand_request(
                    retained.membership_id,
                    consumer_id,
                    current_generation,
                    1,
                    "legacy-newer-admission-next",
                ),
            )
            .await;
        let after = retained_demand_row(&db, created.lease.demand_id).await?;
        assert_identity_mismatch_preserves_retained_row(
            "newer current admission next renewal",
            outcome,
            &before,
            &after,
        )
    })
    .await;
}

#[tokio::test]
async fn legacy_exact_identity_next_renewal_remains_positive() {
    run_body(|db| async move {
        let owner = db.seed_owner("legacy-exact-identity").await?;
        let fixture = db.seed_ready_membership(owner, "005930.KRX").await?;
        assert_real_app_role(&db).await?;
        let app = db.repository_as_app();
        let consumer_id = Uuid::new_v4();
        let created = app
            .create_or_renew_demand(
                owner,
                &demand_request(
                    fixture.membership_id,
                    consumer_id,
                    fixture.generation,
                    0,
                    "legacy-exact-identity-create",
                ),
            )
            .await
            .map_err(|error| format!("could not create exact legacy demand: {error}"))?;
        let renewed = app
            .create_or_renew_demand(
                owner,
                &demand_request(
                    fixture.membership_id,
                    consumer_id,
                    fixture.generation,
                    1,
                    "legacy-exact-identity-next",
                ),
            )
            .await
            .map_err(|error| format!("exact legacy renewal was rejected: {error}"))?;
        if renewed.kind != DemandMutationKind::Renewed
            || renewed.lease.demand_id != created.lease.demand_id
            || renewed.lease.membership_id != fixture.membership_id
            || renewed.lease.generation != fixture.generation
            || renewed.lease.renewal_sequence != 1
            || renewed.lease.lease_expires_at <= created.lease.lease_expires_at
        {
            return Err("exact legacy identity did not renew at its next sequence".to_owned());
        }
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn legacy_exact_replay_still_precedes_admission_validation_after_invalidation() {
    run_body(|db| async move {
        let owner = db.seed_owner("legacy-replay-after-invalidation").await?;
        let fixture = db.seed_ready_membership(owner, "005930.KRX").await?;
        assert_real_app_role(&db).await?;
        let app = db.repository_as_app();
        let consumer_id = Uuid::new_v4();
        let request = demand_request(
            fixture.membership_id,
            consumer_id,
            fixture.generation,
            0,
            "legacy-replay-after-invalidation",
        );
        let created = app
            .create_or_renew_demand(owner, &request)
            .await
            .map_err(|error| format!("could not create replay fixture demand: {error}"))?;
        let before = retained_demand_row(&db, created.lease.demand_id).await?;
        disable_fixture_membership(&db, &fixture).await?;
        let replay = app
            .create_or_renew_demand(owner, &request)
            .await
            .map_err(|error| format!("legacy exact replay changed ordering: {error}"))?;
        let after = retained_demand_row(&db, created.lease.demand_id).await?;
        if replay.kind != DemandMutationKind::Replayed
            || replay.lease != created.lease
            || after != before
        {
            return Err(
                "legacy exact replay after invalidation changed its response or retained row"
                    .to_owned(),
            );
        }
        Ok(())
    })
    .await;
}
