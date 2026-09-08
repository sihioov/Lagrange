mod intraday_quotes_support;

use chrono::{Duration, Utc};
use intraday_quotes_support::{
    MembershipFixture, run_body, wait_for_blocked_session, wait_until_database_time,
};
use job_queue::owner_equity_v2::{
    DemandMutationKind, DemandReleaseKind, IntradayAttemptReservation, IntradayPublicationContext,
    IntradayQuoteDemandRequest, IntradayQuoteFailureCode, IntradayQuoteIdentity,
    IntradayQuoteReceipt, IntradayQuoteReleaseRequest, IntradaySessionProof, IntradayStorageError,
    ProducerClaimKind,
};
use market_data::intraday_quotes::{IntradayQuote, IntradayQuoteDirection};
use tokio::sync::Barrier;
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

fn identity(fixture: &MembershipFixture) -> IntradayQuoteIdentity {
    IntradayQuoteIdentity::new(
        fixture.owner_user_id,
        fixture.membership_id,
        fixture.generation_id,
        fixture.instrument_id.clone(),
        fixture.generation,
    )
    .expect("synthetic fixture identity is valid")
}

fn quote(symbol: &str, price: &str, amount: &str, percent: &str) -> IntradayQuote {
    IntradayQuote {
        symbol: symbol.to_owned(),
        price: price.to_owned(),
        change_from_previous_day: amount.to_owned(),
        change_percent_from_previous_day: percent.to_owned(),
        direction: if amount == "0" && percent == "0" {
            IntradayQuoteDirection::Flat
        } else if amount.starts_with('-') {
            IntradayQuoteDirection::Down
        } else {
            IntradayQuoteDirection::Up
        },
        base_price: "71000".to_owned(),
        halted: false,
    }
}

fn context(
    db: &intraday_quotes_support::IntradayTestDb,
    fixture: &MembershipFixture,
    lease: job_queue::owner_equity_v2::ProducerLease,
) -> IntradayPublicationContext {
    let session = db.session_proof();
    let attempt = IntradayAttemptReservation::new(Uuid::new_v4(), db.session_date, 1)
        .expect("synthetic budget reservation is explicit and valid");
    IntradayPublicationContext::new(lease, identity(fixture), session, attempt)
        .expect("synthetic publication context is valid")
}

#[tokio::test]
async fn migration_up_down_preserves_0053_lineage_and_drops_only_0054() {
    run_body(|db| async move {
        let owner = db.seed_owner("migration").await?;
        let fixture = db.seed_ready_membership(owner, "005930.KRX").await?;
        let before: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM public.owner_equity_memberships WHERE id = $1",
        )
        .bind(fixture.membership_id)
        .fetch_one(&db.superuser)
        .await
        .map_err(|_| "0053 fixture disappeared before down migration".to_owned())?;
        if before != 1 {
            return Err("expected one durable 0053 membership before down migration".to_owned());
        }

        sqlx::raw_sql(include_str!(
            "../../../migrations/0054_owner_intraday_quotes.down.sql"
        ))
        .execute(&db.migration_owner)
        .await
        .map_err(|_| "0054 down migration failed".to_owned())?;
        let demand_table: Option<String> =
            sqlx::query_scalar("SELECT to_regclass('public.owner_intraday_quote_demands')::text")
                .fetch_one(&db.superuser)
                .await
                .map_err(|_| "could not inspect dropped demand table".to_owned())?;
        let durable_after: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM public.owner_equity_memberships WHERE id = $1",
        )
        .bind(fixture.membership_id)
        .fetch_one(&db.superuser)
        .await
        .map_err(|_| "0053 membership was not preserved by down migration".to_owned())?;
        if demand_table.is_some() || durable_after != 1 {
            return Err("down migration changed the wrong tables".to_owned());
        }
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn demand_sequences_replay_expiry_release_scope_and_gc_are_typed() {
    run_body(|db| async move {
        let owner = db.seed_owner("demand").await?;
        let fixture = db.seed_ready_membership(owner, "005930.KRX").await?;
        let repository = db.repository_as_app();
        let consumer = Uuid::new_v4();
        let first = repository
            .create_or_renew_demand(
                owner,
                &demand_request(fixture.membership_id, consumer, 1, 0, "demand-0"),
            )
            .await
            .map_err(|error| format!("initial demand failed: {error}"))?;
        if first.kind != DemandMutationKind::Created || first.lease.renewal_sequence != 0 {
            return Err("sequence-zero demand was not created".to_owned());
        }
        let merged = repository
            .active_demand_identities(owner)
            .await
            .map_err(|error| format!("active demand merge failed: {error}"))?;
        if merged != vec![identity(&fixture)] {
            return Err("one active consumer did not merge to its exact identity".to_owned());
        }
        let original_expiry = first.lease.lease_expires_at;
        let stored_digests: (String, String) = sqlx::query_as(
            "SELECT idempotency_key_sha256, request_sha256
               FROM public.owner_intraday_quote_demands
              WHERE id = $1",
        )
        .bind(first.lease.demand_id)
        .fetch_one(&db.superuser)
        .await
        .map_err(|_| "could not inspect demand digests".to_owned())?;
        if stored_digests.0.contains("demand-0")
            || stored_digests.1.contains("demand-0")
            || !stored_digests.0.starts_with("sha256:")
            || !stored_digests.1.starts_with("sha256:")
        {
            return Err("raw idempotency input was stored instead of canonical digests".to_owned());
        }

        let replay = repository
            .create_or_renew_demand(
                owner,
                &demand_request(fixture.membership_id, consumer, 1, 0, "demand-0"),
            )
            .await
            .map_err(|error| format!("exact replay failed: {error}"))?;
        if replay.kind != DemandMutationKind::Replayed
            || replay.lease.lease_expires_at != original_expiry
        {
            return Err("exact replay extended or changed the original expiry".to_owned());
        }
        let mismatch = repository
            .create_or_renew_demand(
                owner,
                &demand_request(fixture.membership_id, consumer, 1, 1, "demand-0"),
            )
            .await;
        if mismatch != Err(IntradayStorageError::IdempotencyMismatch) {
            return Err("same idempotency key with a different body was accepted".to_owned());
        }
        let skipped = repository
            .create_or_renew_demand(
                owner,
                &demand_request(fixture.membership_id, consumer, 1, 3, "demand-3"),
            )
            .await;
        if skipped != Err(IntradayStorageError::SequenceConflict) {
            return Err("skipped renewal sequence was accepted".to_owned());
        }

        let renewed = repository
            .create_or_renew_demand(
                owner,
                &demand_request(fixture.membership_id, consumer, 1, 1, "demand-1"),
            )
            .await
            .map_err(|error| format!("renewal failed: {error}"))?;
        if renewed.kind != DemandMutationKind::Renewed
            || renewed.lease.renewal_sequence != 1
            || renewed.lease.lease_expires_at <= original_expiry
        {
            return Err("exact next renewal did not advance the lease".to_owned());
        }
        sqlx::query(
            "UPDATE public.owner_intraday_quote_demands
                SET created_at = pg_catalog.now() - INTERVAL '2 seconds',
                    lease_expires_at = pg_catalog.now() - INTERVAL '1 second'
              WHERE id = $1",
        )
        .bind(renewed.lease.demand_id)
        .execute(&db.superuser)
        .await
        .map_err(|_| "could not expire retained active demand".to_owned())?;
        let reactivated = repository
            .create_or_renew_demand(
                owner,
                &demand_request(fixture.membership_id, consumer, 1, 2, "demand-2"),
            )
            .await
            .map_err(|error| format!("retained expiry renewal failed: {error}"))?;
        if reactivated.kind != DemandMutationKind::Renewed
            || reactivated.lease.renewal_sequence != 2
        {
            return Err("expired ACTIVE demand did not renew at the next sequence".to_owned());
        }

        let sibling_consumer = Uuid::new_v4();
        let sibling = repository
            .create_or_renew_demand(
                owner,
                &demand_request(fixture.membership_id, sibling_consumer, 1, 0, "sibling-0"),
            )
            .await
            .map_err(|error| format!("sibling demand failed: {error}"))?;
        let released = repository
            .release_demand(
                owner,
                sibling.lease.demand_id,
                &release_request(sibling_consumer, 0, "release-sibling"),
            )
            .await
            .map_err(|error| format!("release failed: {error}"))?;
        if released.kind != DemandReleaseKind::Released {
            return Err("scoped release was not recorded".to_owned());
        }
        let release_replay = repository
            .release_demand(
                owner,
                sibling.lease.demand_id,
                &release_request(sibling_consumer, 0, "release-sibling"),
            )
            .await
            .map_err(|error| format!("release replay failed: {error}"))?;
        if release_replay.kind != DemandReleaseKind::Replayed {
            return Err("exact release replay was not idempotent".to_owned());
        }
        let resurrect = repository
            .create_or_renew_demand(
                owner,
                &demand_request(fixture.membership_id, sibling_consumer, 1, 1, "resurrect"),
            )
            .await;
        if resurrect != Err(IntradayStorageError::DemandReleased) {
            return Err("RELEASED demand was resurrected".to_owned());
        }
        let active_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM public.owner_intraday_quote_demands
              WHERE owner_user_id = '00000000-0000-0000-0000-000000000000'::uuid",
        )
        .fetch_one(&db.worker)
        .await
        .map_err(|_| "count query failed".to_owned())?;
        if active_count != 0 {
            return Err("test count query unexpectedly selected another owner".to_owned());
        }

        let gc_consumer = Uuid::new_v4();
        let gc_demand = repository
            .create_or_renew_demand(
                owner,
                &demand_request(fixture.membership_id, gc_consumer, 1, 0, "gc-0"),
            )
            .await
            .map_err(|error| format!("GC demand setup failed: {error}"))?;
        sqlx::query(
            "UPDATE public.owner_intraday_quote_demands
                SET created_at = pg_catalog.now() - INTERVAL '26 hours',
                    lease_expires_at = pg_catalog.now() - INTERVAL '25 hours'
              WHERE id = $1",
        )
        .bind(gc_demand.lease.demand_id)
        .execute(&db.superuser)
        .await
        .map_err(|_| "could not age GC demand".to_owned())?;
        let gc = db
            .repository_as_worker()
            .gc_expired(owner)
            .await
            .map_err(|error| format!("demand GC failed: {error}"))?;
        if gc.demand_rows_deleted != 1 {
            return Err("expired demand tombstone was not garbage collected".to_owned());
        }
        let missing_renewal = repository
            .create_or_renew_demand(
                owner,
                &demand_request(fixture.membership_id, gc_consumer, 1, 1, "gc-renewal"),
            )
            .await;
        if missing_renewal != Err(IntradayStorageError::DemandNotFound) {
            return Err("missing GC renewal created a new consumer implicitly".to_owned());
        }
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn concurrent_demand_edges_are_serialized_at_both_caps() {
    run_body(|db| async move {
        let owner = db.seed_owner("caps").await?;
        let fixture = db.seed_ready_membership(owner, "005930.KRX").await?;
        let repository = db.repository_as_app();
        for index in 0..19 {
            let result = repository
                .create_or_renew_demand(
                    owner,
                    &demand_request(
                        fixture.membership_id,
                        Uuid::new_v4(),
                        1,
                        0,
                        &format!("consumer-{index}"),
                    ),
                )
                .await
                .map_err(|error| format!("capacity setup failed: {error}"))?;
            if result.kind != DemandMutationKind::Created {
                return Err("capacity setup did not create a demand".to_owned());
            }
        }
        let barrier = std::sync::Arc::new(Barrier::new(3));
        let left_repo = repository.clone();
        let right_repo = repository.clone();
        let left_barrier = barrier.clone();
        let right_barrier = barrier.clone();
        let left_consumer = Uuid::new_v4();
        let right_consumer = Uuid::new_v4();
        let left = async move {
            left_barrier.wait().await;
            left_repo
                .create_or_renew_demand(
                    owner,
                    &demand_request(fixture.membership_id, left_consumer, 1, 0, "cap-edge-left"),
                )
                .await
        };
        let right = async move {
            right_barrier.wait().await;
            right_repo
                .create_or_renew_demand(
                    owner,
                    &demand_request(
                        fixture.membership_id,
                        right_consumer,
                        1,
                        0,
                        "cap-edge-right",
                    ),
                )
                .await
        };
        let coordinator_barrier = barrier.clone();
        let (left, right, _) = tokio::join!(left, right, async move {
            coordinator_barrier.wait().await;
        });
        let outcomes = [left, right];
        let created = outcomes
            .iter()
            .filter(
                |outcome| matches!(outcome, Ok(value) if value.kind == DemandMutationKind::Created),
            )
            .count();
        let capacity = outcomes
            .iter()
            .filter(|outcome| **outcome == Err(IntradayStorageError::DemandCapacity))
            .count();
        if created != 1 || capacity != 1 {
            return Err("concurrent 20-consumer edge exceeded or underfilled the cap".to_owned());
        }
        let merged = repository
            .active_demand_identities(owner)
            .await
            .map_err(|error| format!("active consumer merge after cap edge failed: {error}"))?;
        if merged != vec![identity(&fixture)] {
            return Err("duplicate active consumers produced more than one identity".to_owned());
        }

        let owner2 = db.seed_owner("identity-cap").await?;
        let mut memberships = Vec::new();
        for symbol in ["000001.KRX", "000002.KRX", "000003.KRX", "000004.KRX"] {
            memberships.push(db.seed_ready_membership(owner2, symbol).await?);
        }
        for (index, membership) in memberships.iter().enumerate() {
            repository
                .create_or_renew_demand(
                    owner2,
                    &demand_request(
                        membership.membership_id,
                        Uuid::new_v4(),
                        1,
                        0,
                        &format!("identity-setup-{index}"),
                    ),
                )
                .await
                .map_err(|error| format!("identity capacity setup failed: {error}"))?;
        }
        let fifth = db.seed_ready_membership(owner2, "000005.KRX").await?;
        let sixth = db.seed_ready_membership(owner2, "000006.KRX").await?;
        let barrier = std::sync::Arc::new(Barrier::new(3));
        let left_repo = repository.clone();
        let right_repo = repository.clone();
        let left_barrier = barrier.clone();
        let right_barrier = barrier.clone();
        let left = async move {
            left_barrier.wait().await;
            left_repo
                .create_or_renew_demand(
                    owner2,
                    &demand_request(fifth.membership_id, Uuid::new_v4(), 1, 0, "identity-edge-5"),
                )
                .await
        };
        let right = async move {
            right_barrier.wait().await;
            right_repo
                .create_or_renew_demand(
                    owner2,
                    &demand_request(sixth.membership_id, Uuid::new_v4(), 1, 0, "identity-edge-6"),
                )
                .await
        };
        let coordinator_barrier = barrier.clone();
        let (left, right, _) = tokio::join!(left, right, async move {
            coordinator_barrier.wait().await;
        });
        let outcomes = [left, right];
        let created = outcomes
            .iter()
            .filter(
                |outcome| matches!(outcome, Ok(value) if value.kind == DemandMutationKind::Created),
            )
            .count();
        let capacity = outcomes
            .iter()
            .filter(|outcome| **outcome == Err(IntradayStorageError::IdentityCapacity))
            .count();
        if created != 1 || capacity != 1 {
            return Err(
                "concurrent fifth-identity edge exceeded or underfilled the cap".to_owned(),
            );
        }
        let merged = repository
            .active_demand_identities(owner2)
            .await
            .map_err(|error| format!("active identity merge after cap edge failed: {error}"))?;
        if merged.len() != 5 {
            return Err("active demand merge did not expose five distinct identities".to_owned());
        }
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn existing_identity_can_add_a_consumer_at_the_five_identity_cap() {
    run_body(|db| async move {
        let owner = db.seed_owner("same-identity-cap").await?;
        let app = db.repository_as_app();
        let mut memberships = Vec::new();
        for symbol in [
            "000001.KRX",
            "000002.KRX",
            "000003.KRX",
            "000004.KRX",
            "000005.KRX",
        ] {
            memberships.push(db.seed_ready_membership(owner, symbol).await?);
        }
        for (index, membership) in memberships.iter().enumerate() {
            app.create_or_renew_demand(
                owner,
                &demand_request(
                    membership.membership_id,
                    Uuid::new_v4(),
                    1,
                    0,
                    &format!("five-identity-{index}"),
                ),
            )
            .await
            .map_err(|error| format!("five-identity setup failed: {error}"))?;
        }
        let duplicate = app
            .create_or_renew_demand(
                owner,
                &demand_request(
                    memberships[0].membership_id,
                    Uuid::new_v4(),
                    1,
                    0,
                    "same-identity-at-cap",
                ),
            )
            .await
            .map_err(|error| format!("existing identity was incorrectly denied: {error}"))?;
        if duplicate.kind != DemandMutationKind::Created {
            return Err(
                "existing identity did not admit a sixth consumer at the identity cap".to_owned(),
            );
        }
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn expired_demand_reactivates_with_an_active_sibling_at_identity_cap() {
    run_body(|db| async move {
        let owner = db.seed_owner("expired-sibling").await?;
        let app = db.repository_as_app();
        let mut memberships = Vec::new();
        for symbol in [
            "000011.KRX",
            "000012.KRX",
            "000013.KRX",
            "000014.KRX",
            "000015.KRX",
        ] {
            memberships.push(db.seed_ready_membership(owner, symbol).await?);
        }
        let first_consumer = Uuid::new_v4();
        let first = app
            .create_or_renew_demand(
                owner,
                &demand_request(
                    memberships[0].membership_id,
                    first_consumer,
                    1,
                    0,
                    "expired-sibling-first",
                ),
            )
            .await
            .map_err(|error| format!("expired-sibling first demand failed: {error}"))?;
        for (index, membership) in memberships.iter().enumerate().skip(1) {
            app.create_or_renew_demand(
                owner,
                &demand_request(
                    membership.membership_id,
                    Uuid::new_v4(),
                    1,
                    0,
                    &format!("expired-sibling-other-{index}"),
                ),
            )
            .await
            .map_err(|error| format!("expired-sibling setup failed: {error}"))?;
        }
        app.create_or_renew_demand(
            owner,
            &demand_request(
                memberships[0].membership_id,
                Uuid::new_v4(),
                1,
                0,
                "expired-sibling-active",
            ),
        )
        .await
        .map_err(|error| format!("expired-sibling active sibling failed: {error}"))?;
        sqlx::query(
            "UPDATE public.owner_intraday_quote_demands
                SET created_at = pg_catalog.clock_timestamp() - INTERVAL '2 seconds',
                    lease_expires_at = pg_catalog.clock_timestamp() - INTERVAL '1 second'
              WHERE id = $1",
        )
        .bind(first.lease.demand_id)
        .execute(&db.superuser)
        .await
        .map_err(|_| "could not expire demand beside its active sibling".to_owned())?;

        let renewed = app
            .create_or_renew_demand(
                owner,
                &demand_request(
                    memberships[0].membership_id,
                    first_consumer,
                    1,
                    1,
                    "expired-sibling-renewal",
                ),
            )
            .await
            .map_err(|error| format!("expired demand with active sibling was denied: {error}"))?;
        if renewed.kind != DemandMutationKind::Renewed || renewed.lease.renewal_sequence != 1 {
            return Err("expired demand did not renew at the exact next sequence".to_owned());
        }
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn sixth_distinct_identity_is_denied_at_the_identity_cap() {
    run_body(|db| async move {
        let owner = db.seed_owner("sixth-identity").await?;
        let app = db.repository_as_app();
        let mut memberships = Vec::new();
        for symbol in [
            "000021.KRX",
            "000022.KRX",
            "000023.KRX",
            "000024.KRX",
            "000025.KRX",
            "000026.KRX",
        ] {
            memberships.push(db.seed_ready_membership(owner, symbol).await?);
        }
        for (index, membership) in memberships.iter().take(5).enumerate() {
            app.create_or_renew_demand(
                owner,
                &demand_request(
                    membership.membership_id,
                    Uuid::new_v4(),
                    1,
                    0,
                    &format!("five-distinct-{index}"),
                ),
            )
            .await
            .map_err(|error| format!("five-distinct setup failed: {error}"))?;
        }
        let denied = app
            .create_or_renew_demand(
                owner,
                &demand_request(
                    memberships[5].membership_id,
                    Uuid::new_v4(),
                    1,
                    0,
                    "sixth-distinct",
                ),
            )
            .await;
        if denied != Err(IntradayStorageError::IdentityCapacity) {
            return Err("sixth distinct identity did not return identity capacity".to_owned());
        }
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn twentieth_consumer_cap_is_checked_independently_of_identity_cap() {
    run_body(|db| async move {
        let owner = db.seed_owner("twentieth-consumer").await?;
        let app = db.repository_as_app();
        let mut memberships = Vec::new();
        for symbol in [
            "000031.KRX",
            "000032.KRX",
            "000033.KRX",
            "000034.KRX",
            "000035.KRX",
        ] {
            memberships.push(db.seed_ready_membership(owner, symbol).await?);
        }
        for membership in &memberships {
            for _ in 0..4 {
                app.create_or_renew_demand(
                    owner,
                    &demand_request(
                        membership.membership_id,
                        Uuid::new_v4(),
                        1,
                        0,
                        &format!("twentieth-{}", Uuid::new_v4()),
                    ),
                )
                .await
                .map_err(|error| format!("twentieth-consumer setup failed: {error}"))?;
            }
        }
        let denied = app
            .create_or_renew_demand(
                owner,
                &demand_request(
                    memberships[0].membership_id,
                    Uuid::new_v4(),
                    1,
                    0,
                    "twenty-one-same-identity",
                ),
            )
            .await;
        if denied != Err(IntradayStorageError::DemandCapacity) {
            return Err("the 20-consumer cap was not reported independently".to_owned());
        }
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn demand_advisory_capacity_mutex_has_observable_real_lock_contention() {
    run_body(|db| async move {
        let owner = db.seed_owner("advisory-lock").await?;
        let fixture = db.seed_ready_membership(owner, "000041.KRX").await?;
        let mut observer_tx = db
            .superuser
            .begin()
            .await
            .map_err(|_| "could not begin advisory-lock observer transaction".to_owned())?;
        let observer_pid: i32 = sqlx::query_scalar("SELECT pg_catalog.pg_backend_pid()")
            .fetch_one(&mut *observer_tx)
            .await
            .map_err(|_| "could not identify advisory-lock observer backend".to_owned())?;
        sqlx::query(
            "SELECT pg_catalog.pg_advisory_xact_lock(
                        pg_catalog.hashtextextended($1, 0))",
        )
        .bind(format!("owner-intraday-demand-cap|{owner}"))
        .execute(&mut *observer_tx)
        .await
        .map_err(|_| "could not hold owner demand advisory mutex".to_owned())?;

        let repository = db.repository_as_app();
        let task = tokio::spawn(async move {
            repository
                .create_or_renew_demand(
                    owner,
                    &demand_request(
                        fixture.membership_id,
                        Uuid::new_v4(),
                        1,
                        0,
                        "advisory-blocked-demand",
                    ),
                )
                .await
        });
        let (_, blockers) =
            wait_for_blocked_session(&db.superuser, "app", "pg_advisory_xact_lock").await?;
        if !blockers.contains(&observer_pid) {
            return Err(
                "app demand mutation was not blocked by the observer advisory lock".to_owned(),
            );
        }
        observer_tx
            .commit()
            .await
            .map_err(|_| "could not release owner demand advisory mutex".to_owned())?;
        let outcome = task
            .await
            .map_err(|_| "advisory-blocked demand task did not finish".to_owned())?
            .map_err(|error| format!("advisory-blocked demand failed after release: {error}"))?;
        if outcome.kind != DemandMutationKind::Created {
            return Err("advisory-blocked demand did not create after mutex release".to_owned());
        }
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn publication_rechecks_expired_producer_after_observed_row_lock_wait() {
    run_body(|db| async move {
        let owner = db.seed_owner("publication-lock-wait").await?;
        let fixture = db.seed_ready_membership(owner, "000051.KRX").await?;
        let app = db.repository_as_app();
        app.create_or_renew_demand(
            owner,
            &demand_request(
                fixture.membership_id,
                Uuid::new_v4(),
                1,
                0,
                "publication-lock-wait-demand",
            ),
        )
        .await
        .map_err(|error| format!("publication lock-wait demand setup failed: {error}"))?;
        let worker = db.repository_as_worker();
        let claim = worker
            .claim_producer(owner, Uuid::new_v4())
            .await
            .map_err(|error| format!("publication lock-wait producer setup failed: {error}"))?;
        let publication_context = context(&db, &fixture, claim.lease);
        let mut observer_tx = db
            .superuser
            .begin()
            .await
            .map_err(|_| "could not begin producer-lock observer transaction".to_owned())?;
        let observer_pid: i32 = sqlx::query_scalar("SELECT pg_catalog.pg_backend_pid()")
            .fetch_one(&mut *observer_tx)
            .await
            .map_err(|_| "could not identify producer-lock observer backend".to_owned())?;
        let hold_started: chrono::DateTime<Utc> =
            sqlx::query_scalar("SELECT pg_catalog.clock_timestamp()")
                .fetch_one(&db.superuser)
                .await
                .map_err(|_| "could not sample producer-lock start time".to_owned())?;
        sqlx::query(
            "UPDATE public.owner_intraday_quote_producers
                SET lease_expires_at = pg_catalog.clock_timestamp() + INTERVAL '1 second',
                    heartbeat_at = pg_catalog.clock_timestamp(),
                    updated_at = pg_catalog.clock_timestamp()
              WHERE owner_user_id = $1",
        )
        .bind(owner)
        .execute(&mut *observer_tx)
        .await
        .map_err(|_| "could not age producer lease inside lock observer".to_owned())?;
        sqlx::query(
            "SELECT owner_user_id
               FROM public.owner_intraday_quote_producers
              WHERE owner_user_id = $1
              FOR UPDATE",
        )
        .bind(owner)
        .fetch_one(&mut *observer_tx)
        .await
        .map_err(|_| "could not hold producer row lock".to_owned())?;

        let receipt = IntradayQuoteReceipt::captured(Utc::now());
        let publish_task = tokio::spawn({
            let worker = worker.clone();
            let publication_context = publication_context.clone();
            async move {
                worker
                    .publish_success(
                        &publication_context,
                        &quote("000051", "72500", "1500", "2.11"),
                        receipt,
                    )
                    .await
            }
        });
        let (_, blockers) =
            wait_for_blocked_session(&db.superuser, "worker", "owner_intraday_quote_producers")
                .await?;
        if !blockers.contains(&observer_pid) {
            return Err("publication was not blocked by the held producer row".to_owned());
        }
        wait_until_database_time(&db.superuser, hold_started + Duration::seconds(2)).await?;
        observer_tx
            .commit()
            .await
            .map_err(|_| "could not release producer row lock".to_owned())?;
        let publication = publish_task
            .await
            .map_err(|_| "publication lock-wait task did not finish".to_owned())?;
        if publication != Err(IntradayStorageError::ProducerLeaseLost) {
            return Err(
                "expired producer lease was accepted after a real producer-row lock wait"
                    .to_owned(),
            );
        }
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn delayed_demand_renewal_extends_from_post_lock_database_time() {
    run_body(|db| async move {
        let owner = db.seed_owner("delayed-renewal").await?;
        let fixture = db.seed_ready_membership(owner, "000061.KRX").await?;
        let app = db.repository_as_app();
        let consumer = Uuid::new_v4();
        app.create_or_renew_demand(
            owner,
            &demand_request(
                fixture.membership_id,
                consumer,
                1,
                0,
                "delayed-renewal-zero",
            ),
        )
        .await
        .map_err(|error| format!("delayed renewal setup failed: {error}"))?;
        let mut observer_tx = db
            .superuser
            .begin()
            .await
            .map_err(|_| "could not begin demand-lock observer transaction".to_owned())?;
        let observer_pid: i32 = sqlx::query_scalar("SELECT pg_catalog.pg_backend_pid()")
            .fetch_one(&mut *observer_tx)
            .await
            .map_err(|_| "could not identify demand-lock observer backend".to_owned())?;
        let hold_started: chrono::DateTime<Utc> =
            sqlx::query_scalar("SELECT pg_catalog.clock_timestamp()")
                .fetch_one(&db.superuser)
                .await
                .map_err(|_| "could not sample demand-lock start time".to_owned())?;
        sqlx::query(
            "SELECT pg_catalog.pg_advisory_xact_lock(
                        pg_catalog.hashtextextended($1, 0))",
        )
        .bind(format!("owner-intraday-demand-cap|{owner}"))
        .execute(&mut *observer_tx)
        .await
        .map_err(|_| "could not hold demand renewal advisory mutex".to_owned())?;

        let renewal_task = tokio::spawn({
            let app = app.clone();
            async move {
                app.create_or_renew_demand(
                    owner,
                    &demand_request(fixture.membership_id, consumer, 1, 1, "delayed-renewal-one"),
                )
                .await
            }
        });
        let (_, blockers) =
            wait_for_blocked_session(&db.superuser, "app", "pg_advisory_xact_lock").await?;
        if !blockers.contains(&observer_pid) {
            return Err("renewal was not blocked by the held demand advisory mutex".to_owned());
        }
        wait_until_database_time(&db.superuser, hold_started + Duration::seconds(2)).await?;
        observer_tx
            .commit()
            .await
            .map_err(|_| "could not release demand renewal advisory mutex".to_owned())?;
        let renewed = renewal_task
            .await
            .map_err(|_| "delayed renewal task did not finish".to_owned())?
            .map_err(|error| format!("delayed demand renewal failed: {error}"))?;
        let after: chrono::DateTime<Utc> =
            sqlx::query_scalar("SELECT pg_catalog.clock_timestamp()")
                .fetch_one(&db.superuser)
                .await
                .map_err(|_| "could not sample post-renewal database time".to_owned())?;
        if renewed.lease.lease_expires_at <= after + Duration::seconds(29) {
            return Err("demand renewal lease was based on transaction-start time".to_owned());
        }
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn delayed_heartbeat_extends_from_post_lock_database_time() {
    run_body(|db| async move {
        let owner = db.seed_owner("delayed-heartbeat").await?;
        let worker = db.repository_as_worker();
        let claim = worker
            .claim_producer(owner, Uuid::new_v4())
            .await
            .map_err(|error| format!("delayed heartbeat setup failed: {error}"))?;
        let mut observer_tx = db
            .superuser
            .begin()
            .await
            .map_err(|_| "could not begin heartbeat-lock observer transaction".to_owned())?;
        let observer_pid: i32 = sqlx::query_scalar("SELECT pg_catalog.pg_backend_pid()")
            .fetch_one(&mut *observer_tx)
            .await
            .map_err(|_| "could not identify heartbeat-lock observer backend".to_owned())?;
        let hold_started: chrono::DateTime<Utc> =
            sqlx::query_scalar("SELECT pg_catalog.clock_timestamp()")
                .fetch_one(&db.superuser)
                .await
                .map_err(|_| "could not sample heartbeat-lock start time".to_owned())?;
        sqlx::query(
            "UPDATE public.owner_intraday_quote_producers
                SET lease_expires_at = pg_catalog.clock_timestamp() + INTERVAL '10 seconds',
                    heartbeat_at = pg_catalog.clock_timestamp(),
                    updated_at = pg_catalog.clock_timestamp()
              WHERE owner_user_id = $1",
        )
        .bind(owner)
        .execute(&mut *observer_tx)
        .await
        .map_err(|_| "could not prepare delayed heartbeat producer row".to_owned())?;
        sqlx::query(
            "SELECT owner_user_id
               FROM public.owner_intraday_quote_producers
              WHERE owner_user_id = $1
              FOR UPDATE",
        )
        .bind(owner)
        .fetch_one(&mut *observer_tx)
        .await
        .map_err(|_| "could not hold heartbeat producer row lock".to_owned())?;

        let heartbeat_task = tokio::spawn({
            let worker = worker.clone();
            let lease = claim.lease.clone();
            async move { worker.heartbeat_producer(&lease).await }
        });
        let (_, blockers) =
            wait_for_blocked_session(&db.superuser, "worker", "owner_intraday_quote_producers")
                .await?;
        if !blockers.contains(&observer_pid) {
            return Err("heartbeat was not blocked by the held producer row".to_owned());
        }
        wait_until_database_time(&db.superuser, hold_started + Duration::seconds(2)).await?;
        observer_tx
            .commit()
            .await
            .map_err(|_| "could not release heartbeat producer row lock".to_owned())?;
        let heartbeat = heartbeat_task
            .await
            .map_err(|_| "delayed heartbeat task did not finish".to_owned())?
            .map_err(|error| format!("delayed heartbeat failed: {error}"))?;
        let after: chrono::DateTime<Utc> =
            sqlx::query_scalar("SELECT pg_catalog.clock_timestamp()")
                .fetch_one(&db.superuser)
                .await
                .map_err(|_| "could not sample post-heartbeat database time".to_owned())?;
        if heartbeat.lease_expires_at <= after + Duration::seconds(19) {
            return Err("heartbeat lease was based on transaction-start time".to_owned());
        }
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn delayed_claim_takes_over_after_post_lock_expiry_recheck() {
    run_body(|db| async move {
        let owner = db.seed_owner("delayed-claim").await?;
        let worker = db.repository_as_worker();
        let first = worker
            .claim_producer(owner, Uuid::new_v4())
            .await
            .map_err(|error| format!("delayed claim setup failed: {error}"))?;
        let mut observer_tx = db
            .superuser
            .begin()
            .await
            .map_err(|_| "could not begin claim-lock observer transaction".to_owned())?;
        let observer_pid: i32 = sqlx::query_scalar("SELECT pg_catalog.pg_backend_pid()")
            .fetch_one(&mut *observer_tx)
            .await
            .map_err(|_| "could not identify claim-lock observer backend".to_owned())?;
        let hold_started: chrono::DateTime<Utc> =
            sqlx::query_scalar("SELECT pg_catalog.clock_timestamp()")
                .fetch_one(&db.superuser)
                .await
                .map_err(|_| "could not sample claim-lock start time".to_owned())?;
        sqlx::query(
            "UPDATE public.owner_intraday_quote_producers
                SET lease_expires_at = pg_catalog.clock_timestamp() + INTERVAL '1 second',
                    heartbeat_at = pg_catalog.clock_timestamp(),
                    updated_at = pg_catalog.clock_timestamp()
              WHERE owner_user_id = $1",
        )
        .bind(owner)
        .execute(&mut *observer_tx)
        .await
        .map_err(|_| "could not prepare delayed claim producer row".to_owned())?;
        sqlx::query(
            "SELECT owner_user_id
               FROM public.owner_intraday_quote_producers
              WHERE owner_user_id = $1
              FOR UPDATE",
        )
        .bind(owner)
        .fetch_one(&mut *observer_tx)
        .await
        .map_err(|_| "could not hold claim producer row lock".to_owned())?;

        let second_holder = Uuid::new_v4();
        let claim_task = tokio::spawn({
            let worker = worker.clone();
            async move { worker.claim_producer(owner, second_holder).await }
        });
        let (_, blockers) =
            wait_for_blocked_session(&db.superuser, "worker", "owner_intraday_quote_producers")
                .await?;
        if !blockers.contains(&observer_pid) {
            return Err("claim was not blocked by the held producer row".to_owned());
        }
        wait_until_database_time(&db.superuser, hold_started + Duration::seconds(2)).await?;
        observer_tx
            .commit()
            .await
            .map_err(|_| "could not release claim producer row lock".to_owned())?;
        let takeover = claim_task
            .await
            .map_err(|_| "delayed claim task did not finish".to_owned())?
            .map_err(|error| format!("delayed claim failed: {error}"))?;
        if takeover.kind != ProducerClaimKind::TakenOver
            || takeover.lease.fencing_token != first.lease.fencing_token + 1
        {
            return Err("claim did not recheck expiry after its real row-lock wait".to_owned());
        }
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn producer_lease_heartbeat_takeover_and_stale_fence_are_enforced() {
    run_body(|db| async move {
        let owner = db.seed_owner("producer").await?;
        let fixture = db.seed_ready_membership(owner, "005930.KRX").await?;
        let repository = db.repository_as_worker();
        let first_holder = Uuid::new_v4();
        let second_holder = Uuid::new_v4();
        let first = repository
            .claim_producer(owner, first_holder)
            .await
            .map_err(|error| format!("initial producer claim failed: {error}"))?;
        if first.kind != ProducerClaimKind::Acquired || first.lease.fencing_token != 1 {
            return Err("initial producer fence did not start at one".to_owned());
        }
        let held = repository.claim_producer(owner, second_holder).await;
        if held != Err(IntradayStorageError::ProducerLeaseHeld) {
            return Err("a second live producer holder stole the lease".to_owned());
        }
        let heartbeat = repository
            .heartbeat_producer(&first.lease)
            .await
            .map_err(|error| format!("producer heartbeat failed: {error}"))?;
        if heartbeat.fencing_token != first.lease.fencing_token
            || heartbeat.lease_expires_at <= first.lease.lease_expires_at
        {
            return Err("heartbeat did not retain the fence or extend the lease".to_owned());
        }
        sqlx::query(
            "UPDATE public.owner_intraday_quote_producers
                SET heartbeat_at = pg_catalog.now() - INTERVAL '25 seconds',
                    lease_expires_at = pg_catalog.now() - INTERVAL '1 second'
              WHERE owner_user_id = $1",
        )
        .bind(owner)
        .execute(&db.superuser)
        .await
        .map_err(|_| "could not expire producer lease".to_owned())?;
        let takeover = repository
            .claim_producer(owner, second_holder)
            .await
            .map_err(|error| format!("producer takeover failed: {error}"))?;
        if takeover.kind != ProducerClaimKind::TakenOver
            || takeover.lease.fencing_token != first.lease.fencing_token + 1
        {
            return Err("producer takeover did not monotonically advance the fence".to_owned());
        }
        if repository.heartbeat_producer(&first.lease).await
            != Err(IntradayStorageError::ProducerLeaseLost)
        {
            return Err("stale producer heartbeat was accepted".to_owned());
        }

        let demand = db.repository_as_app();
        demand
            .create_or_renew_demand(
                owner,
                &demand_request(
                    fixture.membership_id,
                    Uuid::new_v4(),
                    1,
                    0,
                    "producer-demand",
                ),
            )
            .await
            .map_err(|error| format!("producer demand setup failed: {error}"))?;
        let old_context = context(&db, &fixture, first.lease);
        let stale_publish = repository
            .publish_success(
                &old_context,
                &quote("005930", "72500", "1500", "2.11"),
                IntradayQuoteReceipt::captured(Utc::now()),
            )
            .await;
        if stale_publish != Err(IntradayStorageError::ProducerLeaseLost) {
            return Err("old producer fence published after takeover".to_owned());
        }
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn fenced_success_failure_read_gate_and_gc_preserve_last_good() {
    run_body(|db| async move {
        let owner = db.seed_owner("publish").await?;
        let fixture = db.seed_ready_membership(owner, "005930.KRX").await?;
        let app = db.repository_as_app();
        app.create_or_renew_demand(
            owner,
            &demand_request(
                fixture.membership_id,
                Uuid::new_v4(),
                1,
                0,
                "publish-demand",
            ),
        )
        .await
        .map_err(|error| format!("publication demand setup failed: {error}"))?;
        let worker = db.repository_as_worker();
        let claim = worker
            .claim_producer(owner, Uuid::new_v4())
            .await
            .map_err(|error| format!("publication producer claim failed: {error}"))?;
        let context = context(&db, &fixture, claim.lease);
        let success = worker
            .publish_success(
                &context,
                &quote("005930", "72500", "1500", "2.11"),
                IntradayQuoteReceipt::captured(Utc::now()),
            )
            .await
            .map_err(|error| format!("fenced success failed: {error}"))?;
        if success.quote_version != 1
            || success.price.as_deref() != Some("72500.00000000")
            || success.last_success_at.is_none()
        {
            return Err("successful quote did not create version-one last-good state".to_owned());
        }
        let visible = app
            .read_current_cache(owner, fixture.membership_id, 1, &db.session_proof())
            .await
            .map_err(|error| format!("same-session app read failed: {error}"))?
            .ok_or_else(|| "same-session cache was not readable".to_owned())?;
        if visible.price != success.price || visible.generation_id != fixture.generation_id {
            return Err("app read returned a different quote identity".to_owned());
        }

        let failed = worker
            .record_failure(&context, IntradayQuoteFailureCode::ProviderTimeout)
            .await
            .map_err(|error| format!("fenced failure record failed: {error}"))?;
        if failed.quote_version != 1
            || failed.price != success.price
            || failed.last_success_at != success.last_success_at
            || failed.last_failure_code != Some(IntradayQuoteFailureCode::ProviderTimeout)
        {
            return Err("failure changed the last-good quote or version".to_owned());
        }
        let after_failure = app
            .read_current_cache(owner, fixture.membership_id, 1, &db.session_proof())
            .await
            .map_err(|error| format!("post-failure app read failed: {error}"))?
            .ok_or_else(|| "post-failure cache disappeared".to_owned())?;
        if after_failure.price != success.price
            || after_failure.last_failure_code != Some(IntradayQuoteFailureCode::ProviderTimeout)
        {
            return Err("post-failure read hid the typed failure or changed price".to_owned());
        }

        let old_session = db.session_proof();
        let next_date = db
            .session_date
            .checked_add_days(chrono::Days::new(1))
            .ok_or_else(|| "fixture date overflowed".to_owned())?;
        let next_proof = IntradaySessionProof::new(
            next_date,
            db.calendar_source_batch_id,
            db.calendar_content_sha256.clone(),
            db.window_contract_sha256.clone(),
        )
        .map_err(|error| format!("next proof construction failed: {error}"))?;
        let prior_session_read = app
            .read_current_cache(owner, fixture.membership_id, 1, &next_proof)
            .await;
        if prior_session_read != Err(IntradayStorageError::CalendarProofUnavailable) {
            return Err("prior-session quote was exposed under a new session proof".to_owned());
        }
        if old_session.session_date == next_proof.session_date {
            return Err("synthetic next session did not change date".to_owned());
        }

        sqlx::query(
            "UPDATE public.owner_intraday_quote_cache
                SET last_attempt_at = pg_catalog.now() - INTERVAL '25 hours',
                    last_success_at = pg_catalog.now() - INTERVAL '25 hours',
                    received_at = pg_catalog.now() - INTERVAL '25 hours'
              WHERE owner_user_id = $1 AND membership_id = $2",
        )
        .bind(owner)
        .bind(fixture.membership_id)
        .execute(&db.superuser)
        .await
        .map_err(|_| "could not age cache row for GC".to_owned())?;
        let gc = worker
            .gc_expired(owner)
            .await
            .map_err(|error| format!("cache GC failed: {error}"))?;
        if gc.cache_rows_deleted != 1 {
            return Err("cache row older than later attempt/success was not GC'd".to_owned());
        }
        let durable: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM public.owner_equity_memberships WHERE id = $1",
        )
        .bind(fixture.membership_id)
        .fetch_one(&db.superuser)
        .await
        .map_err(|_| "could not verify EOD/V2 durable fixture".to_owned())?;
        if durable != 1 {
            return Err("quote GC touched durable 0053 lineage".to_owned());
        }
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn older_same_identity_receipt_is_rejected_without_changing_last_good() {
    run_body(|db| async move {
        let owner = db.seed_owner("stale-receipt").await?;
        let fixture = db.seed_ready_membership(owner, "000071.KRX").await?;
        let app = db.repository_as_app();
        app.create_or_renew_demand(
            owner,
            &demand_request(
                fixture.membership_id,
                Uuid::new_v4(),
                1,
                0,
                "stale-receipt-demand",
            ),
        )
        .await
        .map_err(|error| format!("stale receipt demand setup failed: {error}"))?;
        let worker = db.repository_as_worker();
        let claim = worker
            .claim_producer(owner, Uuid::new_v4())
            .await
            .map_err(|error| format!("stale receipt producer setup failed: {error}"))?;
        let publication_context = context(&db, &fixture, claim.lease);
        let newer_receipt = Utc::now();
        let first = worker
            .publish_success(
                &publication_context,
                &quote("000071", "72500", "1500", "2.11"),
                IntradayQuoteReceipt::captured(newer_receipt),
            )
            .await
            .map_err(|error| format!("newer receipt setup failed: {error}"))?;
        let older = worker
            .publish_success(
                &publication_context,
                &quote("000071", "72000", "1000", "1.41"),
                IntradayQuoteReceipt::captured(newer_receipt - Duration::seconds(1)),
            )
            .await;
        if older != Err(IntradayStorageError::QuoteReceiptStale) {
            return Err("an older same-fence receipt did not return typed stale error".to_owned());
        }
        let after = app
            .read_current_cache(owner, fixture.membership_id, 1, &db.session_proof())
            .await
            .map_err(|error| format!("stale receipt cache read failed: {error}"))?
            .ok_or_else(|| "stale receipt removed the last-good cache".to_owned())?;
        if after != first {
            return Err("stale receipt changed a last-good cache field".to_owned());
        }
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn serialized_concurrent_receipts_keep_newest_arrival_and_reject_late_older_one() {
    run_body(|db| async move {
        let owner = db.seed_owner("concurrent-receipts").await?;
        let fixture = db.seed_ready_membership(owner, "000081.KRX").await?;
        let app = db.repository_as_app();
        app.create_or_renew_demand(
            owner,
            &demand_request(
                fixture.membership_id,
                Uuid::new_v4(),
                1,
                0,
                "concurrent-receipts-demand",
            ),
        )
        .await
        .map_err(|error| format!("concurrent receipt demand setup failed: {error}"))?;
        let worker = db.repository_as_worker();
        let claim = worker
            .claim_producer(owner, Uuid::new_v4())
            .await
            .map_err(|error| format!("concurrent receipt producer setup failed: {error}"))?;
        let publication_context = context(&db, &fixture, claim.lease);
        let base_receipt = Utc::now() - Duration::seconds(2);
        worker
            .publish_success(
                &publication_context,
                &quote("000081", "71000", "1000", "1.41"),
                IntradayQuoteReceipt::captured(base_receipt),
            )
            .await
            .map_err(|error| format!("concurrent receipt base setup failed: {error}"))?;

        let mut observer_tx = db
            .superuser
            .begin()
            .await
            .map_err(|_| "could not begin cache-lock observer transaction".to_owned())?;
        let observer_pid: i32 = sqlx::query_scalar("SELECT pg_catalog.pg_backend_pid()")
            .fetch_one(&mut *observer_tx)
            .await
            .map_err(|_| "could not identify cache-lock observer backend".to_owned())?;
        sqlx::query(
            "SELECT owner_user_id
               FROM public.owner_intraday_quote_cache
              WHERE owner_user_id = $1 AND membership_id = $2
              FOR UPDATE",
        )
        .bind(owner)
        .bind(fixture.membership_id)
        .fetch_one(&mut *observer_tx)
        .await
        .map_err(|_| "could not hold cache row lock for concurrent receipts".to_owned())?;

        let newer_receipt = Utc::now();
        let newer_task = tokio::spawn({
            let worker = worker.clone();
            let publication_context = publication_context.clone();
            async move {
                worker
                    .publish_success(
                        &publication_context,
                        &quote("000081", "72500", "1500", "2.11"),
                        IntradayQuoteReceipt::captured(newer_receipt),
                    )
                    .await
            }
        });
        let (_, blockers) =
            wait_for_blocked_session(&db.superuser, "worker", "owner_intraday_quote_cache").await?;
        if !blockers.contains(&observer_pid) {
            return Err("newer receipt was not blocked by the held cache row".to_owned());
        }

        let older_task = tokio::spawn({
            let worker = worker.clone();
            let publication_context = publication_context.clone();
            async move {
                worker
                    .publish_success(
                        &publication_context,
                        &quote("000081", "72000", "1000", "1.41"),
                        IntradayQuoteReceipt::captured(newer_receipt - Duration::seconds(1)),
                    )
                    .await
            }
        });
        observer_tx
            .commit()
            .await
            .map_err(|_| "could not release cache row lock".to_owned())?;
        let newer = newer_task
            .await
            .map_err(|_| "newer concurrent receipt task did not finish".to_owned())?
            .map_err(|error| format!("newer concurrent receipt failed: {error}"))?;
        let older = older_task
            .await
            .map_err(|_| "older concurrent receipt task did not finish".to_owned())?;
        if older != Err(IntradayStorageError::QuoteReceiptStale) {
            return Err("serialized older receipt did not return typed stale error".to_owned());
        }
        let after = app
            .read_current_cache(owner, fixture.membership_id, 1, &db.session_proof())
            .await
            .map_err(|error| format!("concurrent receipt cache read failed: {error}"))?
            .ok_or_else(|| "concurrent receipt cache disappeared".to_owned())?;
        if after.price != newer.price
            || after.quote_version != newer.quote_version
            || after.received_at != newer.received_at
        {
            return Err("serialized concurrent receipt did not retain the newer quote".to_owned());
        }
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn publication_rechecks_expired_last_demand_after_observed_row_lock_wait() {
    run_body(|db| async move {
        let owner = db.seed_owner("expired-last-demand-lock").await?;
        let fixture = db.seed_ready_membership(owner, "000091.KRX").await?;
        let app = db.repository_as_app();
        let demand = app
            .create_or_renew_demand(
                owner,
                &demand_request(
                    fixture.membership_id,
                    Uuid::new_v4(),
                    1,
                    0,
                    "expired-last-demand",
                ),
            )
            .await
            .map_err(|error| format!("expired-last-demand setup failed: {error}"))?;
        let worker = db.repository_as_worker();
        let claim = worker
            .claim_producer(owner, Uuid::new_v4())
            .await
            .map_err(|error| format!("expired-last-demand producer setup failed: {error}"))?;
        let publication_context = context(&db, &fixture, claim.lease);
        let mut observer_tx = db
            .superuser
            .begin()
            .await
            .map_err(|_| "could not begin demand-row observer transaction".to_owned())?;
        let observer_pid: i32 = sqlx::query_scalar("SELECT pg_catalog.pg_backend_pid()")
            .fetch_one(&mut *observer_tx)
            .await
            .map_err(|_| "could not identify demand-row observer backend".to_owned())?;
        let hold_started: chrono::DateTime<Utc> =
            sqlx::query_scalar("SELECT pg_catalog.clock_timestamp()")
                .fetch_one(&db.superuser)
                .await
                .map_err(|_| "could not sample demand-row lock start time".to_owned())?;
        sqlx::query(
            "UPDATE public.owner_intraday_quote_demands
                SET lease_expires_at = pg_catalog.clock_timestamp() + INTERVAL '1 second'
              WHERE id = $1",
        )
        .bind(demand.lease.demand_id)
        .execute(&mut *observer_tx)
        .await
        .map_err(|_| "could not prepare an expiring last demand".to_owned())?;
        sqlx::query(
            "SELECT id
               FROM public.owner_intraday_quote_demands
              WHERE id = $1
              FOR UPDATE",
        )
        .bind(demand.lease.demand_id)
        .fetch_one(&mut *observer_tx)
        .await
        .map_err(|_| "could not hold last demand row lock".to_owned())?;

        let failure_task = tokio::spawn({
            let worker = worker.clone();
            let publication_context = publication_context.clone();
            async move {
                worker
                    .record_failure(
                        &publication_context,
                        IntradayQuoteFailureCode::ProviderTimeout,
                    )
                    .await
            }
        });
        let (_, blockers) =
            wait_for_blocked_session(&db.superuser, "worker", "owner_intraday_quote_demands")
                .await?;
        if !blockers.contains(&observer_pid) {
            return Err("publication was not blocked by the held last-demand row".to_owned());
        }
        wait_until_database_time(&db.superuser, hold_started + Duration::seconds(2)).await?;
        observer_tx
            .commit()
            .await
            .map_err(|_| "could not release last-demand row lock".to_owned())?;
        let failure = failure_task
            .await
            .map_err(|_| "expired last-demand task did not finish".to_owned())?;
        if failure != Err(IntradayStorageError::ActiveDemandRequired) {
            return Err(
                "expired last demand was accepted after a real demand-row lock wait".to_owned(),
            );
        }
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn invalid_proofs_values_receipts_and_disable_races_fail_closed() {
    run_body(|db| async move {
        let owner = db.seed_owner("invalid").await?;
        let fixture = db.seed_ready_membership(owner, "005930.KRX").await?;
        let app = db.repository_as_app();
        app.create_or_renew_demand(
            owner,
            &demand_request(
                fixture.membership_id,
                Uuid::new_v4(),
                1,
                0,
                "invalid-demand",
            ),
        )
        .await
        .map_err(|error| format!("invalid test demand setup failed: {error}"))?;
        let worker = db.repository_as_worker();
        let claim = worker
            .claim_producer(owner, Uuid::new_v4())
            .await
            .map_err(|error| format!("invalid test producer setup failed: {error}"))?;
        let publication_context = context(&db, &fixture, claim.lease.clone());
        let invalid_quote = worker
            .publish_success(
                &publication_context,
                &quote("005930", "0", "1500", "2.11"),
                IntradayQuoteReceipt::captured(Utc::now()),
            )
            .await;
        if invalid_quote != Err(IntradayStorageError::QuoteInvalid) {
            return Err("zero current price was accepted by storage".to_owned());
        }
        let future_receipt = worker
            .publish_success(
                &publication_context,
                &quote("005930", "72500", "1500", "2.11"),
                IntradayQuoteReceipt::captured(Utc::now() + Duration::hours(1)),
            )
            .await;
        if future_receipt != Err(IntradayStorageError::ReceiptInvalid) {
            return Err("future receipt timestamp was accepted".to_owned());
        }
        let malformed_proof = IntradaySessionProof::new(
            db.session_date,
            Uuid::new_v4(),
            "not-a-hash".to_owned(),
            "sha256:not-a-hash".to_owned(),
        );
        if malformed_proof != Err(IntradayStorageError::SessionProofInvalid) {
            return Err("malformed session proof was constructible".to_owned());
        }

        sqlx::query(
            "UPDATE public.trading_calendars
                SET content_sha256 = $2
              WHERE exchange = 'KRX' AND session_date = $1",
        )
        .bind(db.session_date)
        .bind("c".repeat(64))
        .execute(&db.superuser)
        .await
        .map_err(|_| "could not invalidate calendar projection fixture".to_owned())?;
        let invalid_calendar = worker
            .record_failure(
                &publication_context,
                IntradayQuoteFailureCode::ProviderTimeout,
            )
            .await;
        if invalid_calendar != Err(IntradayStorageError::CalendarProofUnavailable) {
            return Err("changed calendar lineage did not fail closed".to_owned());
        }

        let owner2 = db.seed_owner("disabled").await?;
        let fixture2 = db.seed_ready_membership(owner2, "005931.KRX").await?;
        let app2 = db.repository_as_app();
        app2.create_or_renew_demand(
            owner2,
            &demand_request(
                fixture2.membership_id,
                Uuid::new_v4(),
                1,
                0,
                "disabled-demand",
            ),
        )
        .await
        .map_err(|error| format!("disable race demand setup failed: {error}"))?;
        let claim2 = worker
            .claim_producer(owner2, Uuid::new_v4())
            .await
            .map_err(|error| format!("disable race producer setup failed: {error}"))?;
        let context2 = context(&db, &fixture2, claim2.lease);
        sqlx::query(
            "UPDATE public.owner_equity_memberships
                SET state = 'DISABLED', disabled_at = pg_catalog.now(),
                    updated_at = pg_catalog.now()
              WHERE id = $1",
        )
        .bind(fixture2.membership_id)
        .execute(&db.superuser)
        .await
        .map_err(|_| "could not disable membership fixture".to_owned())?;
        let disabled_publish = worker
            .publish_success(
                &context2,
                &quote("005931", "72500", "1500", "2.11"),
                IntradayQuoteReceipt::captured(Utc::now()),
            )
            .await;
        if disabled_publish != Err(IntradayStorageError::MembershipNotReady) {
            return Err("disabled membership accepted an in-flight publication".to_owned());
        }
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn generation_change_fences_old_cache_and_inflight_publication() {
    run_body(|db| async move {
        let owner = db.seed_owner("generation-race").await?;
        let fixture = db.seed_ready_membership(owner, "005930.KRX").await?;
        let app = db.repository_as_app();
        app.create_or_renew_demand(
            owner,
            &demand_request(
                fixture.membership_id,
                Uuid::new_v4(),
                fixture.generation,
                0,
                "generation-race-demand",
            ),
        )
        .await
        .map_err(|error| format!("generation race demand setup failed: {error}"))?;
        let worker = db.repository_as_worker();
        let claim = worker
            .claim_producer(owner, Uuid::new_v4())
            .await
            .map_err(|error| format!("generation race producer setup failed: {error}"))?;
        let publication_context = context(&db, &fixture, claim.lease);
        worker
            .publish_success(
                &publication_context,
                &quote("005930", "72500", "1500", "2.11"),
                IntradayQuoteReceipt::captured(Utc::now()),
            )
            .await
            .map_err(|error| format!("generation race cache setup failed: {error}"))?;

        let next_generation_id = Uuid::new_v4();
        let first_session = db
            .session_date
            .checked_sub_days(chrono::Days::new(120))
            .ok_or_else(|| "generation race fixture date underflowed".to_owned())?;
        sqlx::query(
            "INSERT INTO public.owner_equity_instrument_generations
                (id, membership_id, owner_user_id, instrument_id, generation,
                 target_observed_sessions, minimum_observed_sessions,
                 observed_sessions, first_session, last_session)
             VALUES ($1, $2, $3, $4, 2, 261, 121, 121, $5, $6)",
        )
        .bind(next_generation_id)
        .bind(fixture.membership_id)
        .bind(owner)
        .bind(&fixture.instrument_id)
        .bind(first_session)
        .bind(db.session_date)
        .execute(&db.superuser)
        .await
        .map_err(|_| "could not insert newer generation fixture".to_owned())?;
        sqlx::query(
            "INSERT INTO public.owner_equity_generation_admissions
                (generation_id, owner_user_id, membership_id, instrument_id,
                 generation, raw_manifest_sha256, artifact_manifest_sha256,
                 entitlement_sha256, capture_code_commit, materializer_code_commit)
             VALUES ($1, $2, $3, $4, 2, $5, $6, $7, $8, $8)",
        )
        .bind(next_generation_id)
        .bind(owner)
        .bind(fixture.membership_id)
        .bind(&fixture.instrument_id)
        .bind(format!("sha256:{}", "1".repeat(64)))
        .bind(format!("sha256:{}", "2".repeat(64)))
        .bind(format!("sha256:{}", "3".repeat(64)))
        .bind("c".repeat(40))
        .execute(&db.superuser)
        .await
        .map_err(|_| "could not insert newer admission fixture".to_owned())?;

        let old_read = app
            .read_current_cache(
                owner,
                fixture.membership_id,
                fixture.generation,
                &db.session_proof(),
            )
            .await
            .map_err(|error| format!("old-generation cache read failed: {error}"))?;
        if old_read.is_some() {
            return Err("old-generation cache remained visible after newer admission".to_owned());
        }
        let stale_publication = worker
            .record_failure(
                &publication_context,
                IntradayQuoteFailureCode::ProviderTimeout,
            )
            .await;
        if stale_publication != Err(IntradayStorageError::MembershipNotReady) {
            return Err(
                "old-generation publication crossed the current-admission fence".to_owned(),
            );
        }
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn real_roles_obey_intraday_rls_and_grants() {
    run_body(|db| async move {
        let owner = db.seed_owner("rls").await?;
        let other_owner = db.seed_owner("rls-other").await?;
        let fixture = db.seed_ready_membership(owner, "005930.KRX").await?;
        let app = db.repository_as_app();
        let demand = app
            .create_or_renew_demand(
                owner,
                &demand_request(fixture.membership_id, Uuid::new_v4(), 1, 0, "rls-demand"),
            )
            .await
            .map_err(|error| format!("RLS demand setup failed: {error}"))?;
        let worker = db.repository_as_worker();
        let producer = worker
            .claim_producer(owner, Uuid::new_v4())
            .await
            .map_err(|error| format!("RLS producer setup failed: {error}"))?;
        worker
            .publish_success(
                &context(&db, &fixture, producer.lease),
                &quote("005930", "72500", "1500", "2.11"),
                IntradayQuoteReceipt::captured(Utc::now()),
            )
            .await
            .map_err(|error| format!("RLS cache setup failed: {error}"))?;
        let visible = app
            .read_current_cache(owner, fixture.membership_id, 1, &db.session_proof())
            .await
            .map_err(|error| format!("owner cache read failed: {error}"))?;
        if visible.is_none() {
            return Err("owner app role could not read its own cache".to_owned());
        }
        let hidden = app
            .read_current_cache(other_owner, fixture.membership_id, 1, &db.session_proof())
            .await
            .map_err(|error| format!("other-owner cache read failed: {error}"))?;
        if hidden.is_some() {
            return Err("other owner observed the first owner's cache".to_owned());
        }
        let worker_demand_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM public.owner_intraday_quote_demands
              WHERE id = $1 AND owner_user_id = $2",
        )
        .bind(demand.lease.demand_id)
        .bind(owner)
        .fetch_one(&db.worker)
        .await
        .map_err(|_| "worker could not read demand rows".to_owned())?;
        if worker_demand_count != 1 {
            return Err("worker RLS did not expose its operational demand row".to_owned());
        }
        for pool in [&db.admin, &db.audit_writer, &db.research_writer] {
            let denied = sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM public.owner_intraday_quote_cache",
            )
            .fetch_one(pool)
            .await;
            if denied.is_ok() {
                return Err("non-serving role could read intraday cache".to_owned());
            }
        }
        if app.claim_producer(owner, Uuid::new_v4()).await.is_ok() {
            return Err("app role acquired producer write access".to_owned());
        }
        if sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM public.owner_intraday_quote_producers",
        )
        .fetch_one(&db.app)
        .await
        .is_ok()
        {
            return Err("app role could read producer rows".to_owned());
        }
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn composite_lineage_constraints_and_cache_failure_shape_are_enforced() {
    run_body(|db| async move {
        let owner = db.seed_owner("constraints").await?;
        let fixture = db.seed_ready_membership(owner, "005930.KRX").await?;
        let wrong_generation = Uuid::new_v4();
        let fk_error = sqlx::query(
            "INSERT INTO public.owner_intraday_quote_demands
                (owner_user_id, consumer_id, membership_id, generation_id,
                 instrument_id, generation, state, renewal_sequence,
                 lease_expires_at, idempotency_key_sha256, request_sha256)
             VALUES ($1, $2, $3, $4, $5, 1, 'ACTIVE', 0,
                     pg_catalog.now() + INTERVAL '30 seconds', $6, $6)",
        )
        .bind(owner)
        .bind(Uuid::new_v4())
        .bind(fixture.membership_id)
        .bind(wrong_generation)
        .bind(&fixture.instrument_id)
        .bind(format!("sha256:{}", "a".repeat(64)))
        .execute(&db.superuser)
        .await;
        if fk_error.is_ok() {
            return Err("demand with an unadmitted generation passed the composite FK".to_owned());
        }
        let invalid_hash = sqlx::query(
            "INSERT INTO public.owner_intraday_quote_demands
                (owner_user_id, consumer_id, membership_id, generation_id,
                 instrument_id, generation, state, renewal_sequence,
                 lease_expires_at, idempotency_key_sha256, request_sha256)
             VALUES ($1, $2, $3, $4, $5, 1, 'ACTIVE', 0,
                     pg_catalog.now() + INTERVAL '30 seconds', 'not-a-hash', $6)",
        )
        .bind(owner)
        .bind(Uuid::new_v4())
        .bind(fixture.membership_id)
        .bind(fixture.generation_id)
        .bind(&fixture.instrument_id)
        .bind(format!("sha256:{}", "b".repeat(64)))
        .execute(&db.superuser)
        .await;
        if invalid_hash.is_ok() {
            return Err("noncanonical idempotency hash passed the schema check".to_owned());
        }
        let invalid_quote_shape = sqlx::query(
            "INSERT INTO public.owner_intraday_quote_cache
                (owner_user_id, membership_id, generation_id, instrument_id,
                 generation, session_date, calendar_source,
                 calendar_source_version, calendar_source_batch_id,
                 calendar_content_sha256, window_contract_sha256,
                 price, base_price, change_amount, change_percent,
                 direction, halted, received_at, quote_version,
                 last_attempt_at, producer_fence)
             VALUES ($1, $2, $3, $4, 1, $5, 'kis',
                     'kis-chk-holiday-v1:schema-1', $6, $7, $8,
                     72500, NULL, 1500, 2.11, 'UP', false,
                     pg_catalog.now(), 0, pg_catalog.now(), 1)",
        )
        .bind(owner)
        .bind(fixture.membership_id)
        .bind(fixture.generation_id)
        .bind(&fixture.instrument_id)
        .bind(db.session_date)
        .bind(db.calendar_source_batch_id)
        .bind(&db.calendar_content_sha256)
        .bind(&db.window_contract_sha256)
        .execute(&db.superuser)
        .await;
        if invalid_quote_shape.is_ok() {
            return Err("partial quote values passed the all-or-none cache check".to_owned());
        }
        let demand_table: Option<String> =
            sqlx::query_scalar("SELECT to_regclass('public.owner_intraday_quote_demands')::text")
                .fetch_one(&db.superuser)
                .await
                .map_err(|_| "could not inspect operational table".to_owned())?;
        if demand_table.as_deref() != Some("owner_intraday_quote_demands") {
            return Err("intraday table unexpectedly missing".to_owned());
        }
        Ok(())
    })
    .await;
}
