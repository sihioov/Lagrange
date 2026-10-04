//! Private actual-role regression suite for the WS-3A storage boundary.

#[path = "../../tests/owner_market_stream_boundary_support/mod.rs"]
mod owner_market_stream_boundary_support;

use crate::owner_equity_v2::{
    MarketStreamStorageError, StreamAvailability, StreamConnectionState, StreamFreshness,
    StreamLeaseIdentity, StreamMarketState, StreamStatus, StreamStatusCode,
};
use market_data::market_stream::StreamBasePriceReason;
use owner_market_stream_boundary_support as support;
use uuid::Uuid;

type C3aTestError = Box<dyn std::error::Error + Send + Sync>;
type C3aTestResult = Result<(), C3aTestError>;
type C3aJoinedResult = Result<C3aTestResult, tokio::task::JoinError>;
#[derive(Debug, PartialEq, sqlx::FromRow)]
struct SubscriptionSnapshot {
    credential_slot_id: Uuid,
    symbol: String,
    grant_revision: Uuid,
    epoch: Option<Uuid>,
    state: String,
    pending_operation: Option<String>,
    pending_ordinal: Option<i64>,
    pending_reserved_at: Option<chrono::DateTime<chrono::Utc>>,
    pending_deadline: Option<chrono::DateTime<chrono::Utc>>,
    desired_reference_count: i32,
    subscription_revision: Uuid,
    acked_at: Option<chrono::DateTime<chrono::Utc>>,
    updated_at: chrono::DateTime<chrono::Utc>,
}
type ProducerSnapshot = (
    Uuid,
    i64,
    Option<Uuid>,
    Option<chrono::NaiveDate>,
    chrono::DateTime<chrono::Utc>,
    Option<chrono::DateTime<chrono::Utc>>,
    String,
    i64,
    i64,
    chrono::DateTime<chrono::Utc>,
);

async fn generated_database_name(
    database: &support::DisposableDatabase,
) -> Result<String, sqlx::Error> {
    sqlx::query_scalar("SELECT pg_catalog.current_database()")
        .fetch_one(&database.app)
        .await
}

async fn prepare_database_cleanup(
    database: support::DisposableDatabase,
) -> Result<(support::DisposableDatabase, String), C3aTestError> {
    match generated_database_name(&database).await {
        Ok(database_name) => {
            eprintln!("C3A_FIX_GENERATED_DB {database_name}");
            Ok((database, database_name))
        }
        Err(name_error) => match database.cleanup().await {
            Ok(()) => Err(Box::new(name_error)),
            Err(cleanup_error) => Err(format!(
                "could not read exact generated database name ({name_error}); support cleanup also failed ({cleanup_error})"
            )
            .into()),
        },
    }
}

async fn assert_generated_database_absent(database_name: &str) -> C3aTestResult {
    use sqlx::postgres::PgPoolOptions;

    let supervisor_url = std::env::var(support::SUPERVISOR_ENV)?;
    let supervisor = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&supervisor_url)
        .await?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_database WHERE datname = $1)",
    )
    .bind(database_name)
    .fetch_one(&supervisor)
    .await?;
    let connections: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM pg_catalog.pg_stat_activity WHERE datname = $1",
    )
    .bind(database_name)
    .fetch_one(&supervisor)
    .await?;
    supervisor.close().await;
    if exists || connections != 0 {
        return Err(format!(
            "generated database cleanup incomplete: database_present={exists}, connections={connections}, name={database_name}"
        )
        .into());
    }
    Ok(())
}

async fn cleanup_after_join(
    database: support::DisposableDatabase,
    database_name: String,
    task_result: C3aJoinedResult,
) -> (C3aJoinedResult, C3aTestResult) {
    let db_cleanup = database.cleanup().await;
    let catalog_cleanup = assert_generated_database_absent(&database_name).await;
    let cleanup_result = match (db_cleanup, catalog_cleanup) {
        (Ok(()), Ok(())) => {
            eprintln!(
                "C3A_FIX_CLEANUP database={database_name} present=false connections=0"
            );
            Ok(())
        }
        (database_result, catalog_result) => Err(format!(
            "exact generated database cleanup failed: database_cleanup={database_result:?}; catalog_check={catalog_result:?}"
        )
        .into()),
    };
    (task_result, cleanup_result)
}

fn finish_joined_test(
    task_result: C3aJoinedResult,
    cleanup_result: C3aTestResult,
    label: &str,
) -> C3aTestResult {
    match (task_result, cleanup_result) {
        (Ok(Ok(())), Ok(())) => Ok(()),
        (Ok(Err(test_error)), Ok(())) => Err(test_error),
        (Err(join_error), Ok(())) => Err(Box::new(join_error)),
        (Ok(test_result), Err(cleanup_error)) => Err(format!(
            "{label} body result {test_result:?}; cleanup failed: {cleanup_error}"
        )
        .into()),
        (Err(join_error), Err(cleanup_error)) => {
            Err(format!("{label} task failed {join_error}; cleanup failed: {cleanup_error}").into())
        }
    }
}

async fn read_subscription_snapshot(
    database: &support::DisposableDatabase,
    owner_user_id: Uuid,
    credential_slot_id: Uuid,
    symbol: &str,
) -> Result<Option<SubscriptionSnapshot>, sqlx::Error> {
    let mut tx = database.worker.begin().await?;
    sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
        .bind(owner_user_id.to_string())
        .execute(&mut *tx)
        .await?;
    let row = sqlx::query_as(
        "SELECT credential_slot_id, symbol, grant_revision, epoch, state,
                pending_operation, pending_ordinal, pending_reserved_at,
                pending_deadline, desired_reference_count, subscription_revision,
                acked_at, updated_at
           FROM public.owner_market_stream_subscriptions
          WHERE credential_slot_id = $1 AND symbol = $2",
    )
    .bind(credential_slot_id)
    .bind(symbol)
    .fetch_optional(&mut *tx)
    .await?;
    tx.rollback().await?;
    Ok(row)
}

async fn read_producer_snapshot(
    database: &support::DisposableDatabase,
    owner_user_id: Uuid,
    credential_slot_id: Uuid,
) -> Result<Option<ProducerSnapshot>, sqlx::Error> {
    let mut tx = database.worker.begin().await?;
    sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
        .bind(owner_user_id.to_string())
        .execute(&mut *tx)
        .await?;
    let row = sqlx::query_as(
        "SELECT holder_id, fencing_token, current_epoch, session_date,
                lease_expires_at, heartbeat_at, connection_state, gap_generation,
                state_version, updated_at
           FROM public.owner_market_stream_producers
          WHERE credential_slot_id = $1",
    )
    .bind(credential_slot_id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.rollback().await?;
    Ok(row)
}

#[cfg(feature = "market-stream-db-tests")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn owner_market_stream_real_role_boundary()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use std::time::Duration;

    use crate::owner_equity_v2::market_stream::{
        OwnerMarketStreamRepository, StreamEpochProof, StreamIdentity, StreamLeaseRequest,
        StreamProducerLease, StreamPublicationContext, StreamPublicationItem,
        StreamPublicationObservation, StreamSessionProof,
    };
    use tokio::time::timeout;
    use uuid::Uuid;

    async fn current_loopback_publication(
        database: &owner_market_stream_boundary_support::DisposableDatabase,
        worker: &OwnerMarketStreamRepository,
        producer: &StreamProducerLease,
        session: &StreamSessionProof,
        identity: &StreamIdentity,
    ) -> Result<
        (StreamPublicationContext, StreamPublicationObservation),
        Box<dyn std::error::Error + Send + Sync>,
    > {
        use owner_market_stream_boundary_support as support;

        let now = support::now(database).await?;
        let mut transport = support::loopback_session_for_symbols(
            session.session_date,
            now.timestamp_millis(),
            producer.credential_slot_id,
            vec![identity.symbol().to_owned()],
            Duration::from_secs(5),
        )
        .await?;
        let epoch = transport.session.epoch().uuid();
        let epoch_proof = worker
            .start_stream_epoch(producer, session.clone(), epoch)
            .await?;
        let symbol = identity.symbol().to_owned();
        worker
            .set_subscription_desired(producer, epoch, &symbol, 1)
            .await?;
        let prepared = transport
            .session
            .prepare_subscribe(&symbol)?
            .ok_or("synthetic subscribe unexpectedly became a no-op")?;
        let pending = worker
            .commit_prepared_subscription(producer, &prepared)
            .await?;
        let ack = transport.session.send_prepared(prepared).await?;
        let ack_proof = worker
            .commit_subscription_ack(producer, pending, ack)
            .await?
            .ok_or("subscribe ACK unexpectedly produced no proof")?;
        let receipt = transport.next_receipt().await?;
        transport.close().await?;
        Ok((
            StreamPublicationContext::single(
                producer.clone(),
                epoch_proof,
                identity.clone(),
                ack_proof,
            )?,
            StreamPublicationObservation::from_receipt(receipt),
        ))
    }

    async fn assert_pending_shape_constraint(
        database: &owner_market_stream_boundary_support::DisposableDatabase,
        owner_user_id: Uuid,
        credential_slot_id: Uuid,
        symbol: &str,
        assignment: &'static str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut tx = database.worker.begin().await?;
        sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
            .bind(owner_user_id.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query("SAVEPOINT c3a_pending_shape")
            .execute(&mut *tx)
            .await?;
        let statement = format!(
            "UPDATE public.owner_market_stream_subscriptions
                SET {assignment}
              WHERE credential_slot_id = $1 AND symbol = $2"
        );
        let error = sqlx::query(sqlx::AssertSqlSafe(statement))
            .bind(credential_slot_id)
            .bind(symbol)
            .execute(&mut *tx)
            .await
            .expect_err("invalid pending-field combination passed its database constraint");
        let code = match error {
            sqlx::Error::Database(database) => database.code().map(|code| code.into_owned()),
            other => panic!("expected a PostgreSQL check violation, got {other:?}"),
        };
        assert_eq!(code.as_deref(), Some("23514"), "assignment={assignment}");
        sqlx::query("ROLLBACK TO SAVEPOINT c3a_pending_shape")
            .execute(&mut *tx)
            .await?;
        tx.rollback().await?;
        Ok(())
    }

    async fn run_real_boundary(
        database: &owner_market_stream_boundary_support::DisposableDatabase,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use owner_market_stream_boundary_support as support;

        let fixture = support::seed_fixture(database).await?;
        let app = OwnerMarketStreamRepository::new(database.app.clone());
        let worker = OwnerMarketStreamRepository::new(database.worker.clone());

        // All four serving roles are connected directly.  These statements
        // must fail with PostgreSQL privilege errors, not repository mocks.
        assert_direct_write_denied(
            &database.app,
            "INSERT INTO public.owner_market_stream_cache DEFAULT VALUES",
        )
        .await?;
        assert_direct_write_denied(
            &database.app,
            "INSERT INTO public.owner_market_stream_grants DEFAULT VALUES",
        )
        .await?;
        assert_direct_write_denied(
            &database.worker,
            "INSERT INTO public.owner_market_stream_grants DEFAULT VALUES",
        )
        .await?;
        assert_direct_write_denied(
            &database.research_writer,
            "SELECT 1 FROM public.owner_market_stream_cache",
        )
        .await?;
        assert_direct_write_denied(
            &database.admin,
            "SELECT 1 FROM public.owner_market_stream_grants",
        )
        .await?;

        // Row-locking SELECT needs UPDATE privilege even when the caller only
        // wants to serialize.  These assertions keep the capability confined
        // to migration-owner SECURITY DEFINER helpers.
        for (pool, statement) in [
            (
                &database.app,
                "SELECT id FROM public.owner_equity_memberships LIMIT 1 FOR SHARE",
            ),
            (
                &database.app,
                "SELECT generation_id FROM public.owner_equity_generation_admissions LIMIT 1 FOR SHARE",
            ),
            (
                &database.app,
                "SELECT lease_id FROM public.owner_market_stream_lease_items LIMIT 1 FOR UPDATE",
            ),
            (
                &database.worker,
                "SELECT id FROM public.owner_market_stream_grants LIMIT 1 FOR SHARE",
            ),
            (
                &database.worker,
                "SELECT generation_id FROM public.owner_equity_generation_admissions LIMIT 1 FOR SHARE",
            ),
            (
                &database.worker,
                "SELECT id FROM public.owner_market_stream_leases LIMIT 1 FOR SHARE",
            ),
            (
                &database.worker,
                "SELECT lease_id FROM public.owner_market_stream_lease_items LIMIT 1 FOR SHARE",
            ),
        ] {
            assert_direct_write_denied(pool, statement).await?;
        }

        for (signature, target) in [
            (
                "public.lock_owner_market_stream_rights(uuid,uuid,uuid,uuid,date)",
                "worker",
            ),
            (
                "public.lock_owner_market_stream_session(uuid,uuid,uuid,uuid,text)",
                "worker",
            ),
            (
                "public.lock_owner_market_stream_app_admission(uuid,text,uuid,text,bigint)",
                "app",
            ),
            (
                "public.lock_owner_market_stream_worker_admission(uuid,uuid,uuid,uuid,uuid,uuid,text,bigint)",
                "worker",
            ),
            (
                "public.lock_owner_market_stream_app_lease_items(uuid,text,uuid)",
                "app",
            ),
            (
                "public.lock_owner_market_stream_worker_demand(uuid,uuid,uuid,uuid,uuid[])",
                "worker",
            ),
        ] {
            assert_helper_acl(&database.migration_owner, signature, target).await?;
        }
        let (
            worker_insert,
            worker_update,
            app_insert,
            app_update,
            admin_update,
            research_update,
            worker_table_insert,
        ): (bool, bool, bool, bool, bool, bool, bool) = sqlx::query_as(
            "SELECT pg_catalog.has_column_privilege(
                        'worker', 'public.owner_market_stream_subscriptions',
                        'pending_reserved_at', 'INSERT'),
                    pg_catalog.has_column_privilege(
                        'worker', 'public.owner_market_stream_subscriptions',
                        'pending_reserved_at', 'UPDATE'),
                    pg_catalog.has_column_privilege(
                        'app', 'public.owner_market_stream_subscriptions',
                        'pending_reserved_at', 'INSERT'),
                    pg_catalog.has_column_privilege(
                        'app', 'public.owner_market_stream_subscriptions',
                        'pending_reserved_at', 'UPDATE'),
                    pg_catalog.has_column_privilege(
                        'admin', 'public.owner_market_stream_subscriptions',
                        'pending_reserved_at', 'UPDATE'),
                    pg_catalog.has_column_privilege(
                        'research_writer', 'public.owner_market_stream_subscriptions',
                        'pending_reserved_at', 'UPDATE'),
                    pg_catalog.has_table_privilege(
                        'worker', 'public.owner_market_stream_subscriptions', 'INSERT')",
        )
        .fetch_one(&database.migration_owner)
        .await?;
        assert!(worker_insert && worker_update);
        assert!(!app_insert && !app_update && !admin_update && !research_update);
        assert!(
            !worker_table_insert,
            "worker must retain column-only INSERT"
        );
        assert_direct_write_denied(
            &database.app,
            "SELECT public.lock_owner_market_stream_rights(NULL::uuid, NULL::uuid, NULL::uuid, NULL::uuid, NULL::date)",
        )
        .await?;
        assert_direct_write_denied(
            &database.worker,
            "SELECT * FROM public.lock_owner_market_stream_app_admission(NULL::uuid, NULL::text, NULL::uuid, NULL::text, NULL::bigint)",
        )
        .await?;
        assert_direct_write_denied(
            &database.admin,
            "SELECT public.lock_owner_market_stream_worker_demand(NULL::uuid, NULL::uuid, NULL::uuid, NULL::uuid, NULL::uuid[])",
        )
        .await?;
        assert_direct_write_denied(
            &database.research_writer,
            "SELECT public.lock_owner_market_stream_app_lease_items(NULL::uuid, NULL::text, NULL::uuid)",
        )
        .await?;

        let identities = fixture.identities.clone();
        let lease_identities = fixture.lease_identities();
        let first_consumer = Uuid::new_v4();
        let first_request =
            lease_request(first_consumer, 0, lease_identities.clone(), "first-request");
        let first = app
            .replace_stream_lease(
                fixture.owner_user_id,
                &fixture.owner_session_hash,
                &first_request,
            )
            .await
            .map_err(|error| format!("first app lease replacement failed: {error:?}"))?;

        // Both successful and empty app helper paths must restore every
        // caller-provided lookup GUC, including a stale sentinel value.
        let mut app_helper_tx = database.app.begin().await?;
        set_actor_context(
            &mut app_helper_tx,
            fixture.owner_user_id,
            &fixture.owner_session_hash,
        )
        .await?;
        let lookup_sentinel = "abababababababababababababababababababababababababababababababab";
        sqlx::query(
            "SELECT pg_catalog.set_config('app.market_stream_lookup_session_hash', $1, true)",
        )
        .bind(lookup_sentinel)
        .execute(&mut *app_helper_tx)
        .await?;
        sqlx::query("SELECT pg_catalog.set_config('app.market_stream_lookup_owner_id', $1, true)")
            .bind(fixture.other_owner_user_id.to_string())
            .execute(&mut *app_helper_tx)
            .await?;
        let admission_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM public.lock_owner_market_stream_app_admission(
                $1, $2, $3, $4, $5
            )",
        )
        .bind(fixture.owner_user_id)
        .bind(&fixture.owner_session_hash)
        .bind(identities[0].membership_id)
        .bind(&identities[0].instrument_id)
        .bind(i64::try_from(identities[0].generation)?)
        .fetch_one(&mut *app_helper_tx)
        .await?;
        assert_eq!(admission_count, 1);
        let missing_admission_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM public.lock_owner_market_stream_app_admission(
                $1, $2, $3, $4, $5
            )",
        )
        .bind(fixture.owner_user_id)
        .bind(&fixture.owner_session_hash)
        .bind(Uuid::new_v4())
        .bind(&identities[0].instrument_id)
        .bind(i64::try_from(identities[0].generation)?)
        .fetch_one(&mut *app_helper_tx)
        .await?;
        assert_eq!(missing_admission_count, 0);
        let cross_owner_admission_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM public.lock_owner_market_stream_app_admission(
                $1, $2, $3, $4, $5
            )",
        )
        .bind(fixture.other_owner_user_id)
        .bind(&fixture.owner_session_hash)
        .bind(identities[0].membership_id)
        .bind(&identities[0].instrument_id)
        .bind(i64::try_from(identities[0].generation)?)
        .fetch_one(&mut *app_helper_tx)
        .await?;
        assert_eq!(cross_owner_admission_count, 0);
        let mismatched_session_admission_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM public.lock_owner_market_stream_app_admission(
                $1, $2, $3, $4, $5
            )",
        )
        .bind(fixture.owner_user_id)
        .bind(&fixture.other_session_hash)
        .bind(identities[0].membership_id)
        .bind(&identities[0].instrument_id)
        .bind(i64::try_from(identities[0].generation)?)
        .fetch_one(&mut *app_helper_tx)
        .await?;
        assert_eq!(mismatched_session_admission_count, 0);
        let locked_lease_id: Uuid = sqlx::query_scalar(
            "SELECT id FROM public.owner_market_stream_leases
              WHERE id = $1 AND owner_user_id = $2 FOR UPDATE",
        )
        .bind(first.lease_id)
        .bind(fixture.owner_user_id)
        .fetch_one(&mut *app_helper_tx)
        .await?;
        assert_eq!(locked_lease_id, first.lease_id);
        let restored_lookup: (String, String) = sqlx::query_as(
            "SELECT pg_catalog.current_setting('app.market_stream_lookup_session_hash', true),
                    pg_catalog.current_setting('app.market_stream_lookup_owner_id', true)",
        )
        .fetch_one(&mut *app_helper_tx)
        .await?;
        assert_eq!(restored_lookup.0, lookup_sentinel);
        assert_eq!(restored_lookup.1, fixture.other_owner_user_id.to_string());
        let locked_item_count: i32 = sqlx::query_scalar(
            "SELECT public.lock_owner_market_stream_app_lease_items($1, $2, $3)",
        )
        .bind(fixture.owner_user_id)
        .bind(&fixture.owner_session_hash)
        .bind(first.lease_id)
        .fetch_one(&mut *app_helper_tx)
        .await?;
        assert_eq!(locked_item_count, 30);
        let absent_item_count: i32 = sqlx::query_scalar(
            "SELECT public.lock_owner_market_stream_app_lease_items($1, $2, $3)",
        )
        .bind(fixture.owner_user_id)
        .bind(&fixture.owner_session_hash)
        .bind(Uuid::new_v4())
        .fetch_one(&mut *app_helper_tx)
        .await?;
        assert_eq!(absent_item_count, 0);
        let unauthorized_item_count: i32 = sqlx::query_scalar(
            "SELECT public.lock_owner_market_stream_app_lease_items($1, $2, $3)",
        )
        .bind(fixture.other_owner_user_id)
        .bind(&fixture.owner_session_hash)
        .bind(first.lease_id)
        .fetch_one(&mut *app_helper_tx)
        .await?;
        assert_eq!(unauthorized_item_count, 0);
        let restored_after_items: (String, String) = sqlx::query_as(
            "SELECT pg_catalog.current_setting('app.market_stream_lookup_session_hash', true),
                    pg_catalog.current_setting('app.market_stream_lookup_owner_id', true)",
        )
        .fetch_one(&mut *app_helper_tx)
        .await?;
        assert_eq!(restored_after_items.0, lookup_sentinel);
        assert_eq!(
            restored_after_items.1,
            fixture.other_owner_user_id.to_string()
        );
        app_helper_tx.rollback().await?;
        let replay = app
            .replace_stream_lease(
                fixture.owner_user_id,
                &fixture.owner_session_hash,
                &first_request,
            )
            .await?;
        assert_eq!(replay.lease_id, first.lease_id);
        assert_eq!(replay.renewal_sequence, 0);

        let damaged_consumer = Uuid::new_v4();
        let damaged_lease = app
            .replace_stream_lease(
                fixture.owner_user_id,
                &fixture.owner_session_hash,
                &lease_request(
                    damaged_consumer,
                    0,
                    lease_identities[..1].to_vec(),
                    "damaged-items",
                ),
            )
            .await?;
        sqlx::query("DELETE FROM public.owner_market_stream_lease_items WHERE lease_id = $1")
            .bind(damaged_lease.lease_id)
            .execute(&database.migration_owner)
            .await?;
        assert_eq!(
            app.replace_stream_lease(
                fixture.owner_user_id,
                &fixture.owner_session_hash,
                &lease_request(
                    damaged_consumer,
                    1,
                    lease_identities[..1].to_vec(),
                    "damaged-items-renewal",
                ),
            )
            .await
            .unwrap_err(),
            MarketStreamStorageError::DatabaseIntegrity
        );
        let damaged_sequence: i64 = sqlx::query_scalar(
            "SELECT renewal_sequence FROM public.owner_market_stream_leases WHERE id = $1",
        )
        .bind(damaged_lease.lease_id)
        .fetch_one(&database.migration_owner)
        .await?;
        assert_eq!(
            damaged_sequence, 0,
            "missing item locks must fail before DML"
        );
        app.release_stream_lease(
            fixture.owner_user_id,
            &fixture.owner_session_hash,
            damaged_lease.lease_id,
            0,
        )
        .await?;

        let one_consumer = Uuid::new_v4();
        let one_identity_request = lease_request(
            one_consumer,
            0,
            lease_identities[..1].to_vec(),
            "single-identity-create",
        );
        let one_identity_lease = app
            .replace_stream_lease(
                fixture.owner_user_id,
                &fixture.owner_session_hash,
                &one_identity_request,
            )
            .await?;
        let one_identity_replay = app
            .replace_stream_lease(
                fixture.owner_user_id,
                &fixture.owner_session_hash,
                &one_identity_request,
            )
            .await?;
        assert_eq!(one_identity_replay.lease_id, one_identity_lease.lease_id);
        let one_identity_renewal = app
            .replace_stream_lease(
                fixture.owner_user_id,
                &fixture.owner_session_hash,
                &lease_request(
                    one_consumer,
                    1,
                    lease_identities[1..2].to_vec(),
                    "single-identity-replacement",
                ),
            )
            .await?;
        assert_eq!(one_identity_renewal.identities.len(), 1);
        let one_identity_snapshot = app
            .read_stream_snapshot(
                fixture.owner_user_id,
                &fixture.owner_session_hash,
                one_identity_lease.lease_id,
            )
            .await
            .map_err(|error| format!("single-identity snapshot failed: {error:?}"))?;
        assert_eq!(one_identity_snapshot.lease_id, one_identity_lease.lease_id);
        assert!(one_identity_snapshot.lease_expires_at > chrono::Utc::now());
        assert_eq!(one_identity_renewal.identities.len(), 1);
        app.release_stream_lease(
            fixture.owner_user_id,
            &fixture.owner_session_hash,
            one_identity_lease.lease_id,
            1,
        )
        .await?;

        let changed_same_sequence = lease_request(
            first_consumer,
            0,
            lease_identities[..1].to_vec(),
            "changed-request",
        );
        assert_eq!(
            app.replace_stream_lease(
                fixture.owner_user_id,
                &fixture.owner_session_hash,
                &changed_same_sequence
            )
            .await
            .unwrap_err(),
            MarketStreamStorageError::IdempotencyMismatch
        );
        let skipped_sequence = lease_request(
            first_consumer,
            2,
            lease_identities.clone(),
            "skipped-request",
        );
        assert_eq!(
            app.replace_stream_lease(
                fixture.owner_user_id,
                &fixture.owner_session_hash,
                &skipped_sequence
            )
            .await
            .unwrap_err(),
            MarketStreamStorageError::SequenceConflict
        );
        let first_renewal = app
            .replace_stream_lease(
                fixture.owner_user_id,
                &fixture.owner_session_hash,
                &lease_request(first_consumer, 1, lease_identities.clone(), "first-renewal"),
            )
            .await?;
        assert_eq!(first_renewal.renewal_sequence, 1);
        assert_eq!(first_renewal.identities.len(), 30);
        let narrowed_first = app
            .replace_stream_lease(
                fixture.owner_user_id,
                &fixture.owner_session_hash,
                &lease_request(
                    first_consumer,
                    2,
                    lease_identities[..1].to_vec(),
                    "first-narrowed",
                ),
            )
            .await?;
        assert_eq!(narrowed_first.identities.len(), 1);
        let renewed_first = app
            .replace_stream_lease(
                fixture.owner_user_id,
                &fixture.owner_session_hash,
                &lease_request(
                    first_consumer,
                    3,
                    lease_identities.clone(),
                    "first-restored-board",
                ),
            )
            .await?;

        // Ten concurrent tabs ask for the same exact 30 identities.  The
        // repository owns the 20-lease mutex and must retain only 30 desired
        // upstream identities, with reference counts rather than duplicates.
        let mut leases = vec![renewed_first.clone()];
        let mut tasks = Vec::new();
        for index in 1..10_u32 {
            let repository = app.clone();
            let session_hash = fixture.owner_session_hash.clone();
            let owner = fixture.owner_user_id;
            let request = lease_request(
                Uuid::new_v4(),
                0,
                lease_identities.clone(),
                &format!("tab-{index}"),
            );
            tasks.push(tokio::spawn(async move {
                repository
                    .replace_stream_lease(owner, &session_hash, &request)
                    .await
            }));
        }
        for task in tasks {
            leases.push(task.await??);
        }
        for index in 10..20_u32 {
            leases.push(
                app.replace_stream_lease(
                    fixture.owner_user_id,
                    &fixture.owner_session_hash,
                    &lease_request(
                        Uuid::new_v4(),
                        0,
                        lease_identities[..1].to_vec(),
                        &format!("single-{index}"),
                    ),
                )
                .await?,
            );
        }
        assert_eq!(leases.len(), 20);
        let over_lease_capacity = lease_request(
            Uuid::new_v4(),
            0,
            lease_identities[..1].to_vec(),
            "over-capacity",
        );
        assert_eq!(
            app.replace_stream_lease(
                fixture.owner_user_id,
                &fixture.owner_session_hash,
                &over_lease_capacity
            )
            .await
            .unwrap_err(),
            MarketStreamStorageError::LeaseCapacity
        );
        let desired = worker
            .read_stream_demand(fixture.credential_slot_id)
            .await?;
        assert_eq!(desired.items.len(), 30);
        let first_identity_demand = desired
            .items
            .iter()
            .find(|item| item.identity.membership_id == identities[0].membership_id)
            .expect("first identity is in desired union");
        assert_eq!(first_identity_demand.reference_count, 20);

        // Keep one full-board lease, release the other 19, and prove an
        // expired lease cannot be renewed with its old consumer namespace.
        for lease in leases.iter().skip(1) {
            app.release_stream_lease(
                fixture.owner_user_id,
                &fixture.owner_session_hash,
                lease.lease_id,
                lease.renewal_sequence,
            )
            .await?;
        }
        let expired = app
            .replace_stream_lease(
                fixture.owner_user_id,
                &fixture.owner_session_hash,
                &lease_request(
                    Uuid::new_v4(),
                    0,
                    lease_identities[..1].to_vec(),
                    "expires-soon",
                ),
            )
            .await?;
        support::set_stream_lease_expired(database, expired.lease_id).await?;
        assert_eq!(
            app.replace_stream_lease(
                fixture.owner_user_id,
                &fixture.owner_session_hash,
                &lease_request(
                    expired.consumer_id,
                    1,
                    lease_identities[..1].to_vec(),
                    "expired-renewal",
                ),
            )
            .await
            .unwrap_err(),
            MarketStreamStorageError::LeaseExpired
        );
        app.release_stream_lease(
            fixture.owner_user_id,
            &fixture.owner_session_hash,
            expired.lease_id,
            0,
        )
        .await?;

        // RLS is actor and canonical-session scoped; the second owner cannot
        // read the first owner's lease even when it knows the UUID.
        assert_eq!(
            app.read_stream_snapshot(
                fixture.other_owner_user_id,
                &fixture.other_session_hash,
                first.lease_id,
            )
            .await
            .unwrap_err(),
            MarketStreamStorageError::LeaseNotFound
        );
        assert_eq!(
            actor_visible_lease_count(
                &database.app,
                fixture.other_owner_user_id,
                &fixture.other_session_hash
            )
            .await?,
            0
        );

        let producer_holder = Uuid::new_v4();
        let producer = worker
            .claim_stream_producer(
                fixture.credential_slot_id,
                producer_holder,
                fixture.grant_revision,
            )
            .await?;
        let same_claim = worker
            .claim_stream_producer(
                fixture.credential_slot_id,
                producer_holder,
                fixture.grant_revision,
            )
            .await?;
        assert_eq!(same_claim.fencing_token, producer.fencing_token);
        assert_eq!(
            worker
                .claim_stream_producer(
                    fixture.credential_slot_id,
                    Uuid::new_v4(),
                    fixture.grant_revision
                )
                .await
                .unwrap_err(),
            MarketStreamStorageError::ProducerHeld
        );

        // Confirm the production session-lock SELECT can see and lock this
        // exact live session under the lookup policy, independently of the
        // worker helper's immutable grant preflight.
        let mut session_lock_probe = database.migration_owner.begin().await?;
        sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
            .bind(fixture.owner_user_id.to_string())
            .execute(&mut *session_lock_probe)
            .await?;
        sqlx::query(
            "SELECT pg_catalog.set_config('app.market_stream_lookup_session_hash', $1, true)",
        )
        .bind(&fixture.owner_session_hash)
        .execute(&mut *session_lock_probe)
        .await?;
        sqlx::query("SELECT pg_catalog.set_config('app.market_stream_lookup_owner_id', $1, true)")
            .bind(fixture.owner_user_id.to_string())
            .execute(&mut *session_lock_probe)
            .await?;
        let lockable_session: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1
                  FROM public.web_sessions AS session
                  JOIN public.user_roles AS user_role
                    ON user_role.user_id = session.user_id
                   AND user_role.role_id = 'owner'
                 WHERE session.session_hash = $1
                   AND session.user_id = $2
                   AND session.revoked_at IS NULL
                   AND session.expires_at > pg_catalog.clock_timestamp()
                 FOR SHARE OF session, user_role
            )",
        )
        .bind(&fixture.owner_session_hash)
        .bind(fixture.owner_user_id)
        .fetch_one(&mut *session_lock_probe)
        .await?;
        assert!(lockable_session, "exact owner session row was not lockable");
        session_lock_probe.rollback().await?;

        // Direct target-role calls cover the exact six locking capabilities.
        // Malformed identities return no rows/false before target locks, and
        // temporary lookup/actor GUCs are restored on both success and reject.
        let mut worker_helper_tx = database.worker.begin().await?;
        sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
            .bind(fixture.other_owner_user_id.to_string())
            .execute(&mut *worker_helper_tx)
            .await?;
        let lookup_session_sentinel =
            "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd";
        sqlx::query(
            "SELECT pg_catalog.set_config('app.market_stream_lookup_session_hash', $1, true)",
        )
        .bind(lookup_session_sentinel)
        .execute(&mut *worker_helper_tx)
        .await?;
        sqlx::query("SELECT pg_catalog.set_config('app.market_stream_lookup_owner_id', $1, true)")
            .bind(fixture.other_owner_user_id.to_string())
            .execute(&mut *worker_helper_tx)
            .await?;
        let locked_rights: bool =
            sqlx::query_scalar("SELECT public.lock_owner_market_stream_rights($1, $2, $3, $4, $5)")
                .bind(fixture.credential_slot_id)
                .bind(fixture.grant_id)
                .bind(fixture.grant_revision)
                .bind(fixture.owner_user_id)
                .bind(fixture.session_date)
                .fetch_one(&mut *worker_helper_tx)
                .await?;
        assert!(locked_rights);
        let visible_session: bool =
            sqlx::query_scalar("SELECT public.owner_market_stream_session_valid($1, $2)")
                .bind(&fixture.owner_session_hash)
                .bind(fixture.owner_user_id)
                .fetch_one(&mut *worker_helper_tx)
                .await?;
        assert!(
            visible_session,
            "session validator did not see the exact live owner session"
        );
        let wrong_slot_rights: bool =
            sqlx::query_scalar("SELECT public.lock_owner_market_stream_rights($1, $2, $3, $4, $5)")
                .bind(Uuid::new_v4())
                .bind(fixture.grant_id)
                .bind(fixture.grant_revision)
                .bind(fixture.owner_user_id)
                .bind(fixture.session_date)
                .fetch_one(&mut *worker_helper_tx)
                .await?;
        assert!(!wrong_slot_rights);
        let locked_session: bool = sqlx::query_scalar(
            "SELECT public.lock_owner_market_stream_session($1, $2, $3, $4, $5)",
        )
        .bind(fixture.credential_slot_id)
        .bind(fixture.grant_id)
        .bind(fixture.grant_revision)
        .bind(fixture.owner_user_id)
        .bind(&fixture.owner_session_hash)
        .fetch_one(&mut *worker_helper_tx)
        .await?;
        assert!(locked_session);
        let wrong_revision_session: bool = sqlx::query_scalar(
            "SELECT public.lock_owner_market_stream_session($1, $2, $3, $4, $5)",
        )
        .bind(fixture.credential_slot_id)
        .bind(fixture.grant_id)
        .bind(Uuid::new_v4())
        .bind(fixture.owner_user_id)
        .bind(&fixture.owner_session_hash)
        .fetch_one(&mut *worker_helper_tx)
        .await?;
        assert!(!wrong_revision_session);
        let locked_admission: bool = sqlx::query_scalar(
            "SELECT public.lock_owner_market_stream_worker_admission(
                $1, $2, $3, $4, $5, $6, $7, $8
            )",
        )
        .bind(fixture.credential_slot_id)
        .bind(fixture.grant_id)
        .bind(fixture.grant_revision)
        .bind(fixture.owner_user_id)
        .bind(identities[0].membership_id)
        .bind(identities[0].generation_id)
        .bind(&identities[0].instrument_id)
        .bind(i64::try_from(identities[0].generation)?)
        .fetch_one(&mut *worker_helper_tx)
        .await?;
        assert!(locked_admission);
        let wrong_identity_admission: bool = sqlx::query_scalar(
            "SELECT public.lock_owner_market_stream_worker_admission(
                $1, $2, $3, $4, $5, $6, $7, $8
            )",
        )
        .bind(fixture.credential_slot_id)
        .bind(fixture.grant_id)
        .bind(fixture.grant_revision)
        .bind(fixture.owner_user_id)
        .bind(identities[0].membership_id)
        .bind(Uuid::new_v4())
        .bind(&identities[0].instrument_id)
        .bind(i64::try_from(identities[0].generation)?)
        .fetch_one(&mut *worker_helper_tx)
        .await?;
        assert!(!wrong_identity_admission);
        let one_member_ids = vec![identities[0].membership_id];
        let valid_demand_count: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint
               FROM public.lock_owner_market_stream_worker_demand($1, $2, $3, $4, $5)",
        )
        .bind(fixture.credential_slot_id)
        .bind(fixture.grant_id)
        .bind(fixture.grant_revision)
        .bind(fixture.owner_user_id)
        .bind(&one_member_ids)
        .fetch_one(&mut *worker_helper_tx)
        .await?;
        assert_eq!(valid_demand_count, 1);
        let duplicate_demand_count: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint FROM public.lock_owner_market_stream_worker_demand(
                $1, $2, $3, $4, ARRAY[$5::uuid, $5::uuid]
            )",
        )
        .bind(fixture.credential_slot_id)
        .bind(fixture.grant_id)
        .bind(fixture.grant_revision)
        .bind(fixture.owner_user_id)
        .bind(identities[0].membership_id)
        .fetch_one(&mut *worker_helper_tx)
        .await?;
        assert_eq!(duplicate_demand_count, 0);
        let null_member_demand_count: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint FROM public.lock_owner_market_stream_worker_demand(
                $1, $2, $3, $4, ARRAY[$5::uuid, NULL::uuid]
            )",
        )
        .bind(fixture.credential_slot_id)
        .bind(fixture.grant_id)
        .bind(fixture.grant_revision)
        .bind(fixture.owner_user_id)
        .bind(identities[0].membership_id)
        .fetch_one(&mut *worker_helper_tx)
        .await?;
        assert_eq!(null_member_demand_count, 0);
        let empty_demand_count: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint FROM public.lock_owner_market_stream_worker_demand(
                $1, $2, $3, $4, ARRAY[]::uuid[]
            )",
        )
        .bind(fixture.credential_slot_id)
        .bind(fixture.grant_id)
        .bind(fixture.grant_revision)
        .bind(fixture.owner_user_id)
        .fetch_one(&mut *worker_helper_tx)
        .await?;
        assert_eq!(empty_demand_count, 0);
        let missing_member_demand_count: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint FROM public.lock_owner_market_stream_worker_demand(
                $1, $2, $3, $4, ARRAY[$5::uuid]
            )",
        )
        .bind(fixture.credential_slot_id)
        .bind(fixture.grant_id)
        .bind(fixture.grant_revision)
        .bind(fixture.owner_user_id)
        .bind(Uuid::new_v4())
        .fetch_one(&mut *worker_helper_tx)
        .await?;
        assert_eq!(missing_member_demand_count, 0);
        let oversized_ids: Vec<Uuid> = (0..31).map(|_| Uuid::new_v4()).collect();
        let oversized_demand_count: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint
               FROM public.lock_owner_market_stream_worker_demand($1, $2, $3, $4, $5)",
        )
        .bind(fixture.credential_slot_id)
        .bind(fixture.grant_id)
        .bind(fixture.grant_revision)
        .bind(fixture.owner_user_id)
        .bind(&oversized_ids)
        .fetch_one(&mut *worker_helper_tx)
        .await?;
        assert_eq!(oversized_demand_count, 0);
        let null_array_demand_count: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint FROM public.lock_owner_market_stream_worker_demand(
                $1, $2, $3, $4, NULL::uuid[]
            )",
        )
        .bind(fixture.credential_slot_id)
        .bind(fixture.grant_id)
        .bind(fixture.grant_revision)
        .bind(fixture.owner_user_id)
        .fetch_one(&mut *worker_helper_tx)
        .await?;
        assert_eq!(null_array_demand_count, 0);
        let nil_member_demand_count: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint FROM public.lock_owner_market_stream_worker_demand(
                $1, $2, $3, $4, ARRAY['00000000-0000-0000-0000-000000000000'::uuid]
            )",
        )
        .bind(fixture.credential_slot_id)
        .bind(fixture.grant_id)
        .bind(fixture.grant_revision)
        .bind(fixture.owner_user_id)
        .fetch_one(&mut *worker_helper_tx)
        .await?;
        assert_eq!(nil_member_demand_count, 0);
        let wrong_revision_demand_count: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint
               FROM public.lock_owner_market_stream_worker_demand($1, $2, $3, $4, $5)",
        )
        .bind(fixture.credential_slot_id)
        .bind(fixture.grant_id)
        .bind(Uuid::new_v4())
        .bind(fixture.owner_user_id)
        .bind(&one_member_ids)
        .fetch_one(&mut *worker_helper_tx)
        .await?;
        assert_eq!(wrong_revision_demand_count, 0);
        let wrong_owner_demand_count: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint
               FROM public.lock_owner_market_stream_worker_demand($1, $2, $3, $4, $5)",
        )
        .bind(fixture.credential_slot_id)
        .bind(fixture.grant_id)
        .bind(fixture.grant_revision)
        .bind(fixture.other_owner_user_id)
        .bind(&one_member_ids)
        .fetch_one(&mut *worker_helper_tx)
        .await?;
        assert_eq!(wrong_owner_demand_count, 0);
        let restored_worker_gucs: (String, String, String) = sqlx::query_as(
            "SELECT pg_catalog.current_setting('app.actor_user_id', true),
                    pg_catalog.current_setting('app.market_stream_lookup_session_hash', true),
                    pg_catalog.current_setting('app.market_stream_lookup_owner_id', true)",
        )
        .fetch_one(&mut *worker_helper_tx)
        .await?;
        assert_eq!(
            restored_worker_gucs.0,
            fixture.other_owner_user_id.to_string()
        );
        assert_eq!(restored_worker_gucs.1, lookup_session_sentinel);
        assert_eq!(
            restored_worker_gucs.2,
            fixture.other_owner_user_id.to_string()
        );
        worker_helper_tx.rollback().await?;

        // Positive subscription transitions use actual C2 capabilities
        // produced and consumed on synthetic loopback sockets.
        let first_symbol = identities[0].symbol().to_owned();
        let second_symbol = identities[1].symbol().to_owned();
        let database_now = support::now(database).await?;
        let mut transport = support::loopback_session_for_symbols(
            fixture.session_date,
            database_now.timestamp_millis(),
            fixture.credential_slot_id,
            vec![first_symbol.clone()],
            Duration::from_millis(1_500),
        )
        .await?;
        let epoch = transport.session.epoch().uuid();
        let epoch_proof = worker
            .start_stream_epoch(&same_claim, fixture.session.clone(), epoch)
            .await?;
        worker
            .set_subscription_desired(&same_claim, epoch, &first_symbol, 1)
            .await?;
        let prepared = transport
            .session
            .prepare_subscribe(&first_symbol)?
            .ok_or("first synthetic subscribe unexpectedly became a no-op")?;
        let reserved_at_ms = prepared.reserved_at_ms();
        let deadline_at_ms = prepared.deadline_at_ms();
        let pending = worker
            .commit_prepared_subscription(&same_claim, &prepared)
            .await?;
        let mut pending_read = database.worker.begin().await?;
        sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
            .bind(fixture.owner_user_id.to_string())
            .execute(&mut *pending_read)
            .await?;
        let persisted_pending: (
            String,
            i64,
            chrono::DateTime<chrono::Utc>,
            chrono::DateTime<chrono::Utc>,
            String,
            Uuid,
        ) = sqlx::query_as(
            "SELECT pending_operation, pending_ordinal, pending_reserved_at,
                    pending_deadline, state, subscription_revision
               FROM public.owner_market_stream_subscriptions
              WHERE credential_slot_id = $1 AND symbol = $2",
        )
        .bind(fixture.credential_slot_id)
        .bind(&first_symbol)
        .fetch_one(&mut *pending_read)
        .await?;
        assert_eq!(persisted_pending.0, "SUBSCRIBE");
        assert_eq!(persisted_pending.1, 1);
        assert_eq!(persisted_pending.2.timestamp_millis(), reserved_at_ms);
        assert_eq!(persisted_pending.3.timestamp_millis(), deadline_at_ms);
        assert_eq!(persisted_pending.4, "PENDING_SUBSCRIBE");
        pending_read.rollback().await?;
        for assignment in [
            "pending_operation = NULL",
            "pending_ordinal = NULL",
            "pending_reserved_at = NULL",
            "pending_deadline = NULL",
            "pending_ordinal = 0",
            "pending_reserved_at = pending_deadline",
        ] {
            assert_pending_shape_constraint(
                database,
                fixture.owner_user_id,
                fixture.credential_slot_id,
                &first_symbol,
                assignment,
            )
            .await?;
        }
        worker
            .set_subscription_desired(&same_claim, epoch, &first_symbol, 1)
            .await?;
        let mut no_op_read = database.worker.begin().await?;
        sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
            .bind(fixture.owner_user_id.to_string())
            .execute(&mut *no_op_read)
            .await?;
        let no_op_revision: Uuid = sqlx::query_scalar(
            "SELECT subscription_revision
               FROM public.owner_market_stream_subscriptions
              WHERE credential_slot_id = $1 AND symbol = $2",
        )
        .bind(fixture.credential_slot_id)
        .bind(&first_symbol)
        .fetch_one(&mut *no_op_read)
        .await?;
        assert_eq!(no_op_revision, persisted_pending.5);
        no_op_read.rollback().await?;
        assert_eq!(
            worker
                .set_subscription_desired(&same_claim, epoch, &first_symbol, 0)
                .await
                .unwrap_err(),
            MarketStreamStorageError::SubscriptionInvalid,
            "a changed desired count cannot clear an in-flight command"
        );
        let mut ack_gate = database.worker.begin().await?;
        sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
            .bind(fixture.owner_user_id.to_string())
            .execute(&mut *ack_gate)
            .await?;
        let locked_ordinal: i64 = sqlx::query_scalar(
            "SELECT pending_ordinal
               FROM public.owner_market_stream_subscriptions
              WHERE credential_slot_id = $1 AND symbol = $2
              FOR UPDATE",
        )
        .bind(fixture.credential_slot_id)
        .bind(&first_symbol)
        .fetch_one(&mut *ack_gate)
        .await?;
        assert_eq!(locked_ordinal, 1);
        let ack = transport.session.send_prepared(prepared).await?;
        let acked_at_ms = ack.ack_received_at_ms();
        let ack_worker = worker.clone();
        let ack_producer = same_claim.clone();
        let ack_commit = tokio::spawn(async move {
            ack_worker
                .commit_subscription_ack(&ack_producer, pending, ack)
                .await
        });
        timeout(Duration::from_secs(5), async {
            loop {
                if support::now(database).await?.timestamp_millis() > deadline_at_ms {
                    return Ok::<(), Box<dyn std::error::Error + Send + Sync>>(());
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await??;
        ack_gate.commit().await?;
        let acknowledged_subscription = timeout(Duration::from_secs(5), ack_commit)
            .await???
            .ok_or("subscribe ACK unexpectedly produced no proof")?;
        assert_eq!(acknowledged_subscription.state, "ACKED");
        assert_eq!(
            acknowledged_subscription
                .acked_at
                .as_ref()
                .unwrap()
                .timestamp_millis(),
            acked_at_ms
        );
        let receipt = transport.next_receipt().await?;
        assert_eq!(receipt.observation().symbol, first_symbol);
        assert_eq!(receipt.observation().base_price, None);
        let observation = StreamPublicationObservation::from_receipt(receipt);
        transport.close().await?;

        let context = StreamPublicationContext::single(
            same_claim.clone(),
            epoch_proof.clone(),
            identities[0].clone(),
            acknowledged_subscription.clone(),
        )?;

        // A released tombstone with a subsequently revoked session is not a
        // current demand session and must not poison another valid lease.
        let dead_session_lease = app
            .replace_stream_lease(
                fixture.owner_user_id,
                &fixture.secondary_owner_session_hash,
                &lease_request(
                    Uuid::new_v4(),
                    0,
                    lease_identities[..1].to_vec(),
                    "released-revoked-session",
                ),
            )
            .await?;
        app.release_stream_lease(
            fixture.owner_user_id,
            &fixture.secondary_owner_session_hash,
            dead_session_lease.lease_id,
            0,
        )
        .await?;
        let expired_dead_session_lease = app
            .replace_stream_lease(
                fixture.owner_user_id,
                &fixture.secondary_owner_session_hash,
                &lease_request(
                    Uuid::new_v4(),
                    0,
                    lease_identities[..1].to_vec(),
                    "expired-revoked-session",
                ),
            )
            .await?;
        support::set_stream_lease_expired(database, expired_dead_session_lease.lease_id).await?;
        support::revoke_session(
            database,
            fixture.owner_user_id,
            &fixture.secondary_owner_session_hash,
        )
        .await?;
        assert_eq!(
            worker
                .read_stream_demand(fixture.credential_slot_id)
                .await?
                .items
                .len(),
            30,
            "revoked sessions on released/expired leases do not establish demand"
        );

        // Entitlement revocation racing publication is serialized by the
        // rights FOR SHARE capability.  The mutation wins only after the
        // bounded publisher has reached and waited at that exact row.
        let mut entitlement_revoke_tx = database.migration_owner.begin().await?;
        sqlx::query(
            "UPDATE public.data_entitlements
                SET status = 'REVOKED'
              WHERE id = (
                  SELECT entitlement_id FROM public.owner_market_stream_grants WHERE id = $1
              )",
        )
        .bind(fixture.grant_id)
        .execute(&mut *entitlement_revoke_tx)
        .await?;
        let rights_race_worker = worker.clone();
        let rights_race_context = context.clone();
        let rights_race_observation = observation.clone();
        let rights_race = tokio::spawn(async move {
            rights_race_worker
                .publish_stream_latest(
                    &rights_race_context,
                    std::slice::from_ref(&rights_race_observation),
                )
                .await
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            !rights_race.is_finished(),
            "publication must wait for rights revoke"
        );
        entitlement_revoke_tx.commit().await?;
        assert_eq!(
            timeout(Duration::from_secs(5), rights_race)
                .await??
                .unwrap_err(),
            MarketStreamStorageError::RightsInvalid
        );
        support::restore_entitlement(database, fixture.grant_id).await?;

        let (session_race_context, session_race_observation) = current_loopback_publication(
            database,
            &worker,
            &same_claim,
            &fixture.session,
            &identities[0],
        )
        .await?;
        let mut session_revoke_tx = database.migration_owner.begin().await?;
        sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
            .bind(fixture.owner_user_id.to_string())
            .execute(&mut *session_revoke_tx)
            .await?;
        let revoked_rows = sqlx::query(
            "UPDATE public.web_sessions
                SET revoked_at = pg_catalog.clock_timestamp()
              WHERE session_hash = $1",
        )
        .bind(&fixture.owner_session_hash)
        .execute(&mut *session_revoke_tx)
        .await?;
        assert_eq!(
            revoked_rows.rows_affected(),
            1,
            "session revoke fixture must update one row"
        );
        let session_race_worker = worker.clone();
        let session_race = tokio::spawn(async move {
            session_race_worker
                .publish_stream_latest(
                    &session_race_context,
                    std::slice::from_ref(&session_race_observation),
                )
                .await
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        if session_race.is_finished() {
            let early_result = session_race.await?;
            panic!("publication finished before session revoke committed: {early_result:?}");
        }
        session_revoke_tx.commit().await?;
        assert_eq!(
            timeout(Duration::from_secs(5), session_race)
                .await??
                .unwrap_err(),
            MarketStreamStorageError::SessionInvalid
        );
        support::restore_session(database, fixture.owner_user_id, &fixture.owner_session_hash)
            .await?;

        let (owner_role_race_context, owner_role_race_observation) = current_loopback_publication(
            database,
            &worker,
            &same_claim,
            &fixture.session,
            &identities[0],
        )
        .await?;
        let mut owner_role_revoke_tx = database.migration_owner.begin().await?;
        sqlx::query("DELETE FROM public.user_roles WHERE user_id = $1 AND role_id = 'owner'")
            .bind(fixture.owner_user_id)
            .execute(&mut *owner_role_revoke_tx)
            .await?;
        let owner_role_race_worker = worker.clone();
        let owner_role_race_context_for_task = owner_role_race_context.clone();
        let owner_role_race_observation_for_task = owner_role_race_observation.clone();
        let owner_role_race = tokio::spawn(async move {
            owner_role_race_worker
                .publish_stream_latest(
                    &owner_role_race_context_for_task,
                    std::slice::from_ref(&owner_role_race_observation_for_task),
                )
                .await
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            !owner_role_race.is_finished(),
            "publication must wait for Owner-role revoke"
        );
        owner_role_revoke_tx.commit().await?;
        assert_eq!(
            timeout(Duration::from_secs(5), owner_role_race)
                .await??
                .unwrap_err(),
            MarketStreamStorageError::SessionInvalid
        );
        sqlx::query("INSERT INTO public.user_roles (user_id, role_id) VALUES ($1, 'owner')")
            .bind(fixture.owner_user_id)
            .execute(&database.migration_owner)
            .await?;

        let context = owner_role_race_context;
        let observation = owner_role_race_observation;
        let epoch = context.epoch.epoch;
        let epoch_proof = context.epoch.clone();
        let first_commit = worker
            .publish_stream_latest(&context, std::slice::from_ref(&observation))
            .await?;
        assert!(first_commit.changed);
        let published = &first_commit.rows[0];
        assert!(published.quote.is_some());
        assert_eq!(published.quote_version, 1);
        assert_eq!(
            published.quote.as_ref().expect("quote").base_price_reason,
            StreamBasePriceReason::NotProvidedByChannel
        );
        let duplicate_commit = worker
            .publish_stream_latest(&context, std::slice::from_ref(&observation))
            .await?;
        assert!(!duplicate_commit.changed);
        assert_eq!(duplicate_commit.rows[0].quote_version, 1);

        // Hold the exact owner-capacity advisory mutex while app lease
        // replacement and worker publication contend. Both must wait, then
        // finish in one canonical order without a demand/session escape.
        support::restore_session(
            database,
            fixture.owner_user_id,
            &fixture.secondary_owner_session_hash,
        )
        .await?;
        let mut capacity_blocker = database.app.begin().await?;
        sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
            .bind(fixture.owner_user_id.to_string())
            .execute(&mut *capacity_blocker)
            .await?;
        sqlx::query(
            "SELECT pg_catalog.pg_advisory_xact_lock(
                pg_catalog.hashtextextended($1, 0)
            )",
        )
        .bind(format!(
            "owner-market-stream-capacity:{}",
            fixture.owner_user_id
        ))
        .execute(&mut *capacity_blocker)
        .await?;
        let race_app = app.clone();
        let race_owner = fixture.owner_user_id;
        let race_session = fixture.secondary_owner_session_hash.clone();
        let race_request = lease_request(
            Uuid::new_v4(),
            0,
            lease_identities[..1].to_vec(),
            "capacity-publication-race",
        );
        let lease_race = tokio::spawn(async move {
            race_app
                .replace_stream_lease(race_owner, &race_session, &race_request)
                .await
        });
        let race_worker = worker.clone();
        let race_context = context.clone();
        let race_observation = observation.clone();
        let publication_race = tokio::spawn(async move {
            race_worker
                .publish_stream_latest(&race_context, std::slice::from_ref(&race_observation))
                .await
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            !lease_race.is_finished(),
            "lease mutation must wait on owner mutex"
        );
        assert!(
            !publication_race.is_finished(),
            "publication must wait on owner mutex"
        );
        capacity_blocker.commit().await?;
        let raced_lease = timeout(Duration::from_secs(5), lease_race).await???;
        let raced_publication = timeout(Duration::from_secs(5), publication_race).await???;
        assert!(!raced_publication.changed);
        app.release_stream_lease(
            fixture.owner_user_id,
            &fixture.secondary_owner_session_hash,
            raced_lease.lease_id,
            raced_lease.renewal_sequence,
        )
        .await?;
        support::revoke_session(
            database,
            fixture.owner_user_id,
            &fixture.secondary_owner_session_hash,
        )
        .await?;

        let mut listener = support::listener(database).await?;
        listener.listen("owner_market_stream_changed").await?;
        let retained_status = StreamStatus::new(
            StreamStatusCode::ConnectionLost,
            StreamConnectionState::Disconnected,
            StreamMarketState::Unknown,
            StreamFreshness::Stale,
            StreamAvailability::LastKnown,
            true,
            epoch_proof.gap_generation,
        )?;
        let status_commit = worker
            .record_stream_status(&context, retained_status)
            .await?;
        assert!(status_commit.changed);
        assert!(status_commit.rows[0].quote.is_some());
        let notification = timeout(Duration::from_secs(2), listener.recv()).await??;
        assert_eq!(notification.payload(), "v1");
        let equal_status = worker
            .record_stream_status(&context, retained_status)
            .await?;
        assert!(!equal_status.changed);
        assert_eq!(equal_status.rows[0].quote_version, 1);

        // A second ACKed symbol lets the same status transaction exercise
        // atomic rollback after one row has already been touched.  The
        // migration-owner fixture forces the second row to its checked
        // version ceiling; the first row must remain unchanged.
        let same_claim = worker.renew_stream_producer(&same_claim).await?;
        let second_database_now = support::now(database).await?;
        let mut transport_two = support::loopback_session_for_symbols(
            fixture.session_date,
            second_database_now.timestamp_millis(),
            fixture.credential_slot_id,
            vec![first_symbol.clone(), second_symbol.clone()],
            Duration::from_millis(1_500),
        )
        .await?;
        let second_epoch = transport_two.session.epoch().uuid();
        let epoch_proof_two = worker
            .start_stream_epoch(&same_claim, fixture.session.clone(), second_epoch)
            .await?;
        worker
            .set_subscription_desired(&same_claim, second_epoch, &first_symbol, 1)
            .await?;
        let prepared_first_two = transport_two
            .session
            .prepare_subscribe(&first_symbol)?
            .ok_or("first current-epoch subscribe unexpectedly became a no-op")?;
        assert_eq!(prepared_first_two.epoch().uuid(), second_epoch);
        let pending_first_two = worker
            .commit_prepared_subscription(&same_claim, &prepared_first_two)
            .await?;
        let first_ack_two = transport_two
            .session
            .send_prepared(prepared_first_two)
            .await?;
        let first_subscription_two = worker
            .commit_subscription_ack(&same_claim, pending_first_two, first_ack_two)
            .await?
            .ok_or("first current-epoch subscribe ACK unexpectedly produced no proof")?;
        let _first_receipt_two = transport_two.next_receipt().await?;

        let next_database_now = support::now(database).await?;
        transport_two.advance_clock_to(next_database_now.timestamp_millis());
        worker
            .set_subscription_desired(&same_claim, second_epoch, &second_symbol, 1)
            .await?;
        let prepared_second = transport_two
            .session
            .prepare_subscribe(&second_symbol)?
            .ok_or("second current-epoch subscribe unexpectedly became a no-op")?;
        assert_eq!(prepared_second.epoch().uuid(), second_epoch);
        let pending_second = worker
            .commit_prepared_subscription(&same_claim, &prepared_second)
            .await?;
        let second_ack = transport_two.session.send_prepared(prepared_second).await?;
        let second_subscription = worker
            .commit_subscription_ack(&same_claim, pending_second, second_ack)
            .await?
            .ok_or("second current-epoch subscribe ACK unexpectedly produced no proof")?;
        assert_eq!(second_subscription.state, "ACKED");
        let _second_receipt = transport_two.next_receipt().await?;
        transport_two.close().await?;
        let context_two = StreamPublicationContext::new(
            same_claim.clone(),
            epoch_proof_two.clone(),
            vec![
                StreamPublicationItem {
                    admission: identities[0].clone(),
                    subscription: first_subscription_two,
                },
                StreamPublicationItem {
                    admission: identities[1].clone(),
                    subscription: second_subscription.clone(),
                },
            ],
        )?;
        let initial_two_status = StreamStatus::new(
            StreamStatusCode::AwaitingFirstTrade,
            StreamConnectionState::Connected,
            StreamMarketState::Open,
            StreamFreshness::Unavailable,
            StreamAvailability::AwaitingFirstTrade,
            false,
            epoch_proof_two.gap_generation,
        )?;
        let _ = worker
            .record_stream_status(&context_two, initial_two_status)
            .await?;
        while timeout(Duration::from_millis(10), listener.recv())
            .await
            .is_ok()
        {}
        support::force_cache_state_version_max(database, identities[1].membership_id).await?;
        let rollback_status = StreamStatus::new(
            StreamStatusCode::ReconnectGap,
            StreamConnectionState::Backoff,
            StreamMarketState::Unknown,
            StreamFreshness::Unavailable,
            StreamAvailability::LastKnown,
            true,
            epoch_proof_two.gap_generation,
        )?;
        assert_eq!(
            worker
                .record_stream_status(&context_two, rollback_status)
                .await
                .unwrap_err(),
            MarketStreamStorageError::VersionExhausted
        );
        assert!(
            timeout(Duration::from_millis(100), listener.recv())
                .await
                .is_err(),
            "rolled-back status transaction must not notify"
        );
        let snapshot_after_rollback = app
            .read_stream_snapshot(
                fixture.owner_user_id,
                &fixture.owner_session_hash,
                first.lease_id,
            )
            .await
            .map_err(|error| format!("rollback snapshot failed: {error:?}"))?;
        let first_after_rollback = snapshot_after_rollback
            .rows
            .iter()
            .find(|row| row.identity.membership_id == identities[0].membership_id)
            .expect("first cache row");
        assert_eq!(
            first_after_rollback
                .status
                .as_ref()
                .map(|status| status.code),
            Some(StreamStatusCode::AwaitingFirstTrade)
        );

        // Reconnect with a real loopback receipt for the second identity,
        // then race its current admission against a membership transition.
        // The worker must wait for the 0053 row and reject after DISABLED wins.
        let (second_context_after_reconnect, second_observation) = current_loopback_publication(
            database,
            &worker,
            &same_claim,
            &fixture.session,
            &identities[1],
        )
        .await?;
        let mut membership_change_tx = database.migration_owner.begin().await?;
        sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
            .bind(fixture.owner_user_id.to_string())
            .execute(&mut *membership_change_tx)
            .await?;
        sqlx::query(
            "UPDATE public.owner_equity_memberships
                SET state = 'DISABLED', disabled_at = pg_catalog.clock_timestamp(),
                    updated_at = pg_catalog.clock_timestamp()
              WHERE id = $1 AND owner_user_id = $2",
        )
        .bind(identities[1].membership_id)
        .bind(fixture.owner_user_id)
        .execute(&mut *membership_change_tx)
        .await?;
        let membership_race_worker = worker.clone();
        let membership_race_context = second_context_after_reconnect.clone();
        let membership_race_observation = second_observation.clone();
        let membership_race = tokio::spawn(async move {
            membership_race_worker
                .publish_stream_latest(
                    &membership_race_context,
                    std::slice::from_ref(&membership_race_observation),
                )
                .await
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            !membership_race.is_finished(),
            "publication must wait for current membership transition"
        );
        membership_change_tx.commit().await?;
        assert_eq!(
            timeout(Duration::from_secs(5), membership_race)
                .await??
                .unwrap_err(),
            MarketStreamStorageError::MembershipNotReady
        );

        // Gate publication at the canonical owner mutex, commit a newer
        // 0053 admission generation, then release it. The old full identity
        // must be rejected before any quote write.
        let (generation_race_context, generation_race_observation) = current_loopback_publication(
            database,
            &worker,
            &same_claim,
            &fixture.session,
            &identities[3],
        )
        .await?;
        let mut generation_capacity_blocker = database.app.begin().await?;
        sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
            .bind(fixture.owner_user_id.to_string())
            .execute(&mut *generation_capacity_blocker)
            .await?;
        sqlx::query(
            "SELECT pg_catalog.pg_advisory_xact_lock(
                pg_catalog.hashtextextended($1, 0)
            )",
        )
        .bind(format!(
            "owner-market-stream-capacity:{}",
            fixture.owner_user_id
        ))
        .execute(&mut *generation_capacity_blocker)
        .await?;
        let generation_race_worker = worker.clone();
        let generation_race_context_for_task = generation_race_context.clone();
        let generation_race_observation_for_task = generation_race_observation.clone();
        let generation_race = tokio::spawn(async move {
            generation_race_worker
                .publish_stream_latest(
                    &generation_race_context_for_task,
                    std::slice::from_ref(&generation_race_observation_for_task),
                )
                .await
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            !generation_race.is_finished(),
            "generation publication must wait on owner mutex"
        );
        support::supersede_generation(database, &identities[3]).await?;
        generation_capacity_blocker.commit().await?;
        assert_eq!(
            timeout(Duration::from_secs(5), generation_race)
                .await??
                .unwrap_err(),
            MarketStreamStorageError::MembershipNotReady
        );
        assert_eq!(
            worker
                .read_stream_demand(fixture.credential_slot_id)
                .await?
                .items
                .len(),
            28,
            "latest generation removes one old demanded identity"
        );

        // Date and epoch races invalidate the old publication context.  The
        // old receipt is never retimed or republished.
        sqlx::query(
            "UPDATE public.owner_market_stream_producers
                SET session_date = session_date - 1
              WHERE credential_slot_id = $1",
        )
        .bind(fixture.credential_slot_id)
        .execute(&database.migration_owner)
        .await?;
        assert!(matches!(
            worker
                .record_stream_status(&context, retained_status)
                .await
                .unwrap_err(),
            MarketStreamStorageError::ProducerLost | MarketStreamStorageError::SessionInvalid
        ));
        let _new_epoch = worker
            .start_stream_epoch(&same_claim, fixture.session.clone(), Uuid::new_v4())
            .await?;
        assert!(matches!(
            worker
                .publish_stream_latest(&context, std::slice::from_ref(&observation))
                .await
                .unwrap_err(),
            MarketStreamStorageError::ProducerLost | MarketStreamStorageError::SubscriptionNotAcked
        ));

        support::set_producer_expired(database, fixture.credential_slot_id).await?;
        let takeover_holder = Uuid::new_v4();
        let takeover = worker
            .claim_stream_producer(
                fixture.credential_slot_id,
                takeover_holder,
                fixture.grant_revision,
            )
            .await?;
        assert!(takeover.fencing_token > same_claim.fencing_token);
        assert_eq!(
            worker.renew_stream_producer(&same_claim).await.unwrap_err(),
            MarketStreamStorageError::ProducerLost
        );

        // Prepare one current identity after takeover through the same real
        // C2 transport and private storage transitions.
        let (grant_race_context, grant_race_observation) = current_loopback_publication(
            database,
            &worker,
            &takeover,
            &fixture.session,
            &identities[2],
        )
        .await?;
        let grant_race_initial_commit = worker
            .publish_stream_latest(
                &grant_race_context,
                std::slice::from_ref(&grant_race_observation),
            )
            .await?;
        assert!(grant_race_initial_commit.changed);

        // Session, membership/generation, entitlement, and grant changes are
        // independently revalidated rather than treated as one entitlement.
        support::revoke_session(database, fixture.owner_user_id, &fixture.owner_session_hash)
            .await?;
        assert_eq!(
            app.read_stream_snapshot(
                fixture.owner_user_id,
                &fixture.owner_session_hash,
                first.lease_id
            )
            .await
            .unwrap_err(),
            MarketStreamStorageError::SessionInvalid
        );
        support::restore_session(database, fixture.owner_user_id, &fixture.owner_session_hash)
            .await?;
        assert!(
            worker
                .read_stream_demand(fixture.credential_slot_id)
                .await?
                .items
                .len()
                <= 30
        );

        support::supersede_generation(database, &identities[0]).await?;
        let demand_after_generation = worker
            .read_stream_demand(fixture.credential_slot_id)
            .await?;
        assert_eq!(demand_after_generation.items.len(), 27);
        let snapshot_after_generation = app
            .read_stream_snapshot(
                fixture.owner_user_id,
                &fixture.owner_session_hash,
                first.lease_id,
            )
            .await;
        // The delivery helper rejects the entire lease when any demanded
        // identity is no longer current; a subset is not a valid snapshot.
        assert_eq!(
            snapshot_after_generation,
            Err(MarketStreamStorageError::MembershipNotReady),
            "generation change must deny the stale full-board snapshot"
        );

        let snapshot_after_membership = app
            .read_stream_snapshot(
                fixture.owner_user_id,
                &fixture.owner_session_hash,
                first.lease_id,
            )
            .await;
        assert_eq!(
            snapshot_after_membership,
            Err(MarketStreamStorageError::MembershipNotReady),
            "disabled membership must deny the stale full-board snapshot"
        );

        support::revoke_entitlement(database, fixture.grant_id).await?;
        assert_eq!(
            worker
                .read_stream_demand(fixture.credential_slot_id)
                .await
                .unwrap_err(),
            MarketStreamStorageError::RightsInvalid
        );
        support::restore_entitlement(database, fixture.grant_id).await?;
        let mut grant_revoke_tx = database.migration_owner.begin().await?;
        sqlx::query(
            "UPDATE public.owner_market_stream_grants
                SET state = 'REVOKED', revoked_at = pg_catalog.clock_timestamp(),
                    updated_at = pg_catalog.clock_timestamp()
              WHERE id = $1",
        )
        .bind(fixture.grant_id)
        .execute(&mut *grant_revoke_tx)
        .await?;
        let grant_race_worker = worker.clone();
        let grant_race_context_for_task = grant_race_context.clone();
        let grant_race_observation_for_task = grant_race_observation.clone();
        let grant_revoke_race = tokio::spawn(async move {
            grant_race_worker
                .publish_stream_latest(
                    &grant_race_context_for_task,
                    std::slice::from_ref(&grant_race_observation_for_task),
                )
                .await
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            !grant_revoke_race.is_finished(),
            "publication must wait for immutable grant revocation"
        );
        grant_revoke_tx.commit().await?;
        assert_eq!(
            timeout(Duration::from_secs(5), grant_revoke_race)
                .await??
                .unwrap_err(),
            MarketStreamStorageError::RightsInvalid
        );
        assert_eq!(
            worker
                .read_stream_demand(fixture.credential_slot_id)
                .await
                .unwrap_err(),
            MarketStreamStorageError::RightsInvalid
        );
        let revoked_snapshot = app
            .read_stream_snapshot(
                fixture.owner_user_id,
                &fixture.owner_session_hash,
                first.lease_id,
            )
            .await;
        assert_eq!(
            revoked_snapshot,
            Err(MarketStreamStorageError::MembershipNotReady)
        );

        // The generated database is disposable, so exercise the exact
        // rollback artifacts in reverse order after the role and race evidence.
        let mut down_tx = database.migration_owner.begin().await?;
        sqlx::raw_sql(include_str!(
            "../../../../migrations/0056_owner_market_stream_runtime.down.sql"
        ))
        .execute(&mut *down_tx)
        .await?;
        sqlx::raw_sql(include_str!(
            "../../../../migrations/0055_owner_market_stream.down.sql"
        ))
        .execute(&mut *down_tx)
        .await?;
        let remaining_tables: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint
               FROM pg_catalog.unnest(ARRAY[
                   'owner_market_stream_grants',
                   'owner_market_stream_leases',
                   'owner_market_stream_lease_items',
                   'owner_market_stream_producers',
                   'owner_market_stream_subscriptions',
                   'owner_market_stream_cache'
               ]::text[]) AS expected(name)
              WHERE pg_catalog.to_regclass('public.' || expected.name) IS NOT NULL",
        )
        .fetch_one(&mut *down_tx)
        .await?;
        assert_eq!(remaining_tables, 0, "0055 down left a stream table behind");
        let remaining_functions: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint
               FROM pg_catalog.unnest(ARRAY[
                   'public.owner_market_stream_delivery_state(uuid,text,uuid)',
                   'public.owner_market_stream_grants_guard()',
                   'public.owner_market_stream_rights_valid(uuid,uuid,date)',
                   'public.owner_market_stream_session_valid(text,uuid)',
                   'public.lock_owner_market_stream_rights(uuid,uuid,uuid,uuid,date)',
                   'public.lock_owner_market_stream_session(uuid,uuid,uuid,uuid,text)',
                   'public.lock_owner_market_stream_app_admission(uuid,text,uuid,text,bigint)',
                   'public.lock_owner_market_stream_worker_admission(uuid,uuid,uuid,uuid,uuid,uuid,text,bigint)',
                   'public.lock_owner_market_stream_app_lease_items(uuid,text,uuid)',
                   'public.lock_owner_market_stream_worker_demand(uuid,uuid,uuid,uuid,uuid[])'
               ]::text[]) AS expected(signature)
              WHERE pg_catalog.to_regprocedure(expected.signature) IS NOT NULL",
        )
        .fetch_one(&mut *down_tx)
        .await?;
        assert_eq!(
            remaining_functions, 0,
            "0055 down left a stream function behind"
        );
        let session_policy_remains: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM pg_catalog.pg_policies
                 WHERE schemaname = 'public'
                   AND tablename = 'web_sessions'
                   AND policyname = 'owner_market_stream_session_lookup'
            )",
        )
        .fetch_one(&mut *down_tx)
        .await?;
        assert!(
            !session_policy_remains,
            "0055 down left its session lookup policy"
        );
        down_tx.commit().await?;
        Ok(())
    }

    fn lease_request(
        consumer_id: Uuid,
        renewal_sequence: u64,
        identities: Vec<StreamLeaseIdentity>,
        idempotency_key: &str,
    ) -> StreamLeaseRequest {
        StreamLeaseRequest::new(
            consumer_id,
            renewal_sequence,
            identities,
            idempotency_key.to_owned(),
        )
        .expect("synthetic lease request is valid")
    }

    async fn assert_direct_write_denied(
        pool: &sqlx::PgPool,
        statement: &'static str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let error = sqlx::query(statement)
            .execute(pool)
            .await
            .expect_err("serving role unexpectedly has WS-3A privilege");
        let code = match error {
            sqlx::Error::Database(database) => database.code().map(|code| code.into_owned()),
            other => panic!("expected PostgreSQL permission denial, got {other:?}"),
        };
        assert_eq!(code.as_deref(), Some("42501"));
        Ok(())
    }

    async fn assert_helper_acl(
        pool: &sqlx::PgPool,
        signature: &str,
        target_role: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let (public_exec, app_exec, worker_exec, admin_exec, audit_exec, research_exec): (
            bool,
            bool,
            bool,
            bool,
            bool,
            bool,
        ) = sqlx::query_as(
            "SELECT EXISTS (
                        SELECT 1
                          FROM pg_catalog.pg_proc AS candidate
                          CROSS JOIN LATERAL pg_catalog.aclexplode(candidate.proacl) AS acl
                         WHERE candidate.oid = pg_catalog.to_regprocedure($1)
                           AND acl.grantee = 0
                           AND acl.privilege_type = 'EXECUTE'
                    ),
                    pg_catalog.has_function_privilege('app', candidate.oid, 'EXECUTE'),
                    pg_catalog.has_function_privilege('worker', candidate.oid, 'EXECUTE'),
                    pg_catalog.has_function_privilege('admin', candidate.oid, 'EXECUTE'),
                    pg_catalog.has_function_privilege('audit_writer', candidate.oid, 'EXECUTE'),
                    pg_catalog.has_function_privilege('research_writer', candidate.oid, 'EXECUTE')
               FROM pg_catalog.pg_proc AS candidate
              WHERE candidate.oid = pg_catalog.to_regprocedure($1)",
        )
        .bind(signature)
        .fetch_one(pool)
        .await?;
        assert!(
            !public_exec,
            "PUBLIC EXECUTE unexpectedly granted on {signature}"
        );
        assert_eq!(app_exec, target_role == "app", "app ACL for {signature}");
        assert_eq!(
            worker_exec,
            target_role == "worker",
            "worker ACL for {signature}"
        );
        assert!(
            !admin_exec,
            "admin EXECUTE unexpectedly granted on {signature}"
        );
        assert!(
            !audit_exec,
            "audit_writer EXECUTE unexpectedly granted on {signature}"
        );
        assert!(
            !research_exec,
            "research_writer EXECUTE unexpectedly granted on {signature}"
        );
        Ok(())
    }

    async fn set_actor_context(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        owner_user_id: Uuid,
        session_hash: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
            .bind(owner_user_id.to_string())
            .execute(&mut **tx)
            .await?;
        sqlx::query("SELECT pg_catalog.set_config('app.market_stream_session_hash', $1, true)")
            .bind(session_hash)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    async fn actor_visible_lease_count(
        pool: &sqlx::PgPool,
        owner_user_id: Uuid,
        session_hash: &str,
    ) -> Result<i64, sqlx::Error> {
        let mut tx = pool.begin().await?;
        sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
            .bind(owner_user_id.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query("SELECT pg_catalog.set_config('app.market_stream_session_hash', $1, true)")
            .bind(session_hash)
            .execute(&mut *tx)
            .await?;
        let count =
            sqlx::query_scalar("SELECT count(*)::bigint FROM public.owner_market_stream_leases")
                .fetch_one(&mut *tx)
                .await?;
        tx.commit().await?;
        Ok(count)
    }

    let database = owner_market_stream_boundary_support::DisposableDatabase::create().await?;
    let (database, database_name) = prepare_database_cleanup(database).await?;
    let task_database = database.clone();
    let result = tokio::spawn(async move { run_real_boundary(&task_database).await }).await;
    let (result, cleanup) = cleanup_after_join(database, database_name, result).await;
    finish_joined_test(result, cleanup, "WS-3A boundary")
}

#[cfg(feature = "market-stream-db-tests")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn owner_market_stream_pending_identity_rejection_and_unsubscribe()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use std::time::Duration;

    use crate::owner_equity_v2::market_stream::{OwnerMarketStreamRepository, StreamLeaseRequest};
    use kis_client::market_stream::MarketSubscriptionOperation;
    use owner_market_stream_boundary_support as support;
    use uuid::Uuid;

    async fn run(
        database: &support::DisposableDatabase,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let fixture = support::seed_fixture(database).await?;
        let app = OwnerMarketStreamRepository::new(database.app.clone());
        let worker = OwnerMarketStreamRepository::new(database.worker.clone());
        let identity = &fixture.identities[0];
        let symbol = identity.symbol().to_owned();
        let lease = app
            .replace_stream_lease(
                fixture.owner_user_id,
                &fixture.owner_session_hash,
                &StreamLeaseRequest::new(
                    Uuid::new_v4(),
                    0,
                    fixture.lease_identities()[..1].to_vec(),
                    "c3a-pending-identity".to_owned(),
                )?,
            )
            .await?;
        let producer = worker
            .claim_stream_producer(
                fixture.credential_slot_id,
                Uuid::new_v4(),
                fixture.grant_revision,
            )
            .await?;

        // A real Prepared capability is durably reserved. Replaying the
        // borrowed capability cannot mint a second pending token.
        let database_now = support::now(database).await?;
        let mut rejected_transport = support::loopback_session_for_symbols(
            fixture.session_date,
            database_now.timestamp_millis(),
            fixture.credential_slot_id,
            vec![symbol.clone()],
            Duration::from_secs(5),
        )
        .await?;
        let rejected_epoch = rejected_transport.session.epoch().uuid();
        worker
            .start_stream_epoch(&producer, fixture.session.clone(), rejected_epoch)
            .await?;
        worker
            .set_subscription_desired(&producer, rejected_epoch, &symbol, 1)
            .await?;
        let rejected_prepared = rejected_transport
            .session
            .prepare_subscribe(&symbol)?
            .ok_or("synthetic subscribe unexpectedly became a no-op")?;
        let rejected_pending = worker
            .commit_prepared_subscription(&producer, &rejected_prepared)
            .await?;
        let replay = worker
            .commit_prepared_subscription(&producer, &rejected_prepared)
            .await;
        match replay {
            Err(error) => assert_eq!(error, MarketStreamStorageError::SubscriptionInvalid),
            Ok(_) => panic!("replayed prepared capability minted another pending token"),
        }

        // Only the task-owned synthetic row is corrupted, and the altered
        // reservation remains within the database's four-field constraint.
        let mut fixture_tx = database.worker.begin().await?;
        sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
            .bind(fixture.owner_user_id.to_string())
            .execute(&mut *fixture_tx)
            .await?;
        let changed = sqlx::query(
            "UPDATE public.owner_market_stream_subscriptions
                SET pending_reserved_at = pending_reserved_at + INTERVAL '1 millisecond'
              WHERE credential_slot_id = $1 AND symbol = $2",
        )
        .bind(fixture.credential_slot_id)
        .bind(&symbol)
        .execute(&mut *fixture_tx)
        .await?;
        assert_eq!(changed.rows_affected(), 1);
        fixture_tx.commit().await?;

        let rejected_ack = rejected_transport
            .session
            .send_prepared(rejected_prepared)
            .await?;
        assert_eq!(
            worker
                .commit_subscription_ack(&producer, rejected_pending, rejected_ack)
                .await
                .unwrap_err(),
            MarketStreamStorageError::SubscriptionInvalid
        );
        let mut rejected_read = database.worker.begin().await?;
        sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
            .bind(fixture.owner_user_id.to_string())
            .execute(&mut *rejected_read)
            .await?;
        let retained_pending: (String, Option<String>, Option<i64>) = sqlx::query_as(
            "SELECT state, pending_operation, pending_ordinal
               FROM public.owner_market_stream_subscriptions
              WHERE credential_slot_id = $1 AND symbol = $2",
        )
        .bind(fixture.credential_slot_id)
        .bind(&symbol)
        .fetch_one(&mut *rejected_read)
        .await?;
        assert_eq!(retained_pending.0, "PENDING_SUBSCRIBE");
        assert_eq!(retained_pending.1.as_deref(), Some("SUBSCRIBE"));
        assert_eq!(retained_pending.2, Some(1));
        rejected_read.rollback().await?;
        let _rejected_receipt = rejected_transport.next_receipt().await?;
        rejected_transport.close().await?;

        // A new authentic transport epoch clears the old uncertainty. The
        // same socket then performs both a true subscribe and unsubscribe.
        let database_now = support::now(database).await?;
        let mut transport = support::loopback_session_for_symbols(
            fixture.session_date,
            database_now.timestamp_millis(),
            fixture.credential_slot_id,
            vec![symbol.clone(), symbol.clone()],
            Duration::from_secs(5),
        )
        .await?;
        let epoch = transport.session.epoch().uuid();
        worker
            .start_stream_epoch(&producer, fixture.session.clone(), epoch)
            .await?;
        worker
            .set_subscription_desired(&producer, epoch, &symbol, 1)
            .await?;
        let subscribe = transport
            .session
            .prepare_subscribe(&symbol)?
            .ok_or("fresh epoch subscribe unexpectedly became a no-op")?;
        let subscribe_pending = worker
            .commit_prepared_subscription(&producer, &subscribe)
            .await?;
        let subscribe_ack = transport.session.send_prepared(subscribe).await?;
        assert_eq!(
            subscribe_ack.operation(),
            MarketSubscriptionOperation::Subscribe
        );
        let subscribe_proof = worker
            .commit_subscription_ack(&producer, subscribe_pending, subscribe_ack)
            .await?
            .ok_or("subscribe ACK unexpectedly produced no proof")?;
        assert_eq!(subscribe_proof.state, "ACKED");
        let _receipt = transport.next_receipt().await?;

        worker
            .set_subscription_desired(&producer, epoch, &symbol, 0)
            .await?;
        let database_now = support::now(database).await?;
        transport.advance_clock_to(database_now.timestamp_millis());
        let unsubscribe = transport
            .session
            .prepare_unsubscribe(&symbol)?
            .ok_or("subscribed symbol unexpectedly became an unsubscribe no-op")?;
        assert_eq!(
            unsubscribe.operation(),
            MarketSubscriptionOperation::Unsubscribe
        );
        let unsubscribe_pending = worker
            .commit_prepared_subscription(&producer, &unsubscribe)
            .await?;
        let unsubscribe_ack = transport.session.send_prepared(unsubscribe).await?;
        assert_eq!(
            unsubscribe_ack.operation(),
            MarketSubscriptionOperation::Unsubscribe
        );
        assert!(
            worker
                .commit_subscription_ack(&producer, unsubscribe_pending, unsubscribe_ack)
                .await?
                .is_none(),
            "unsubscribe ACK must not mint a publication proof"
        );
        let mut after_unsubscribe = database.worker.begin().await?;
        sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
            .bind(fixture.owner_user_id.to_string())
            .execute(&mut *after_unsubscribe)
            .await?;
        let absent: (
            String,
            Option<String>,
            Option<i64>,
            Option<chrono::DateTime<chrono::Utc>>,
            Option<chrono::DateTime<chrono::Utc>>,
            Option<chrono::DateTime<chrono::Utc>>,
        ) = sqlx::query_as(
            "SELECT state, pending_operation, pending_ordinal, pending_reserved_at,
                    pending_deadline, acked_at
               FROM public.owner_market_stream_subscriptions
              WHERE credential_slot_id = $1 AND symbol = $2",
        )
        .bind(fixture.credential_slot_id)
        .bind(&symbol)
        .fetch_one(&mut *after_unsubscribe)
        .await?;
        assert_eq!(absent.0, "ABSENT");
        assert!(absent.1.is_none() && absent.2.is_none());
        assert!(absent.3.is_none() && absent.4.is_none() && absent.5.is_none());
        after_unsubscribe.rollback().await?;
        transport.close().await?;
        let _ = lease;
        Ok(())
    }

    let database = support::DisposableDatabase::create().await?;
    let (database, database_name) = prepare_database_cleanup(database).await?;
    let task_database = database.clone();
    let result = tokio::spawn(async move { run(&task_database).await }).await;
    let (result, cleanup) = cleanup_after_join(database, database_name, result).await;
    finish_joined_test(result, cleanup, "C3A private ACK boundary")
}

#[cfg(feature = "market-stream-db-tests")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn owner_market_stream_ack_cannot_cross_credential_slots() -> C3aTestResult {
    use std::time::Duration;

    use crate::owner_equity_v2::market_stream::{OwnerMarketStreamRepository, StreamLeaseRequest};
    use kis_client::market_stream::MarketSubscriptionOperation;

    async fn run(database: &support::DisposableDatabase) -> C3aTestResult {
        let fixture = support::seed_fixture(database).await?;
        let app = OwnerMarketStreamRepository::new(database.app.clone());
        let worker = OwnerMarketStreamRepository::new(database.worker.clone());
        let identity = &fixture.identities[0];
        let symbol = identity.symbol().to_owned();

        // Keep current owner demand legitimate while preparing two distinct
        // producer slots with the same composite-legal grant revision.
        let _owner_lease = app
            .replace_stream_lease(
                fixture.owner_user_id,
                &fixture.owner_session_hash,
                &StreamLeaseRequest::new(
                    Uuid::new_v4(),
                    0,
                    fixture.lease_identities()[..1].to_vec(),
                    "c3a-cross-slot-demand".to_owned(),
                )?,
            )
            .await?;

        let second_slot = Uuid::new_v4();
        let second_grant = Uuid::new_v4();
        let second_membership = Uuid::new_v4();
        let second_generation = Uuid::new_v4();
        let old_other_grant: Uuid = sqlx::query_scalar(
            "SELECT id FROM public.owner_market_stream_grants
              WHERE owner_user_id = $1 AND state = 'ACTIVE'",
        )
        .bind(fixture.other_owner_user_id)
        .fetch_one(&database.migration_owner)
        .await?;
        let mut grant_tx = database.migration_owner.begin().await?;
        sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
            .bind(fixture.other_owner_user_id.to_string())
            .execute(&mut *grant_tx)
            .await?;
        let revoked = sqlx::query(
            "UPDATE public.owner_market_stream_grants
                SET state = 'REVOKED', revoked_at = pg_catalog.clock_timestamp()
              WHERE id = $1 AND owner_user_id = $2 AND state = 'ACTIVE'",
        )
        .bind(old_other_grant)
        .bind(fixture.other_owner_user_id)
        .execute(&mut *grant_tx)
        .await?;
        assert_eq!(revoked.rows_affected(), 1);
        let cloned_revision: Uuid = sqlx::query_scalar(
            "INSERT INTO public.owner_market_stream_grants
                (id, credential_slot_id, credential_generation, owner_user_id,
                 entitlement_id, entitlement_reference, entitlement_document_sha256,
                 tr_id, wire_version, network_contract_sha256, identity_list_sha256,
                 effective_from, effective_until, activation_commit, state,
                 grant_revision)
             SELECT $2, $3, credential_generation, owner_user_id,
                    entitlement_id, entitlement_reference, entitlement_document_sha256,
                    tr_id, wire_version, network_contract_sha256, identity_list_sha256,
                    effective_from, effective_until, activation_commit, 'ACTIVE', $4
               FROM public.owner_market_stream_grants
              WHERE id = $1 AND owner_user_id = $5 AND state = 'REVOKED'
             RETURNING grant_revision",
        )
        .bind(old_other_grant)
        .bind(second_grant)
        .bind(second_slot)
        .bind(fixture.grant_revision)
        .bind(fixture.other_owner_user_id)
        .fetch_one(&mut *grant_tx)
        .await?;

        sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
            .bind(fixture.other_owner_user_id.to_string())
            .execute(&mut *grant_tx)
            .await?;
        sqlx::query(
            "INSERT INTO public.owner_equity_memberships
                (id, owner_user_id, instrument_id, state,
                 transition_actor_user_id, transition_code_commit,
                 transition_entitlement_sha256)
             VALUES ($1, $2, $3, 'REQUESTED', $2, $4, $5)",
        )
        .bind(second_membership)
        .bind(fixture.other_owner_user_id)
        .bind(format!("{symbol}.KRX"))
        .bind(support::CODE_COMMIT)
        .bind(support::TRANSITION_HASH)
        .execute(&mut *grant_tx)
        .await?;
        for state in ["VALIDATING", "BACKFILLING"] {
            sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
                .bind(fixture.other_owner_user_id.to_string())
                .execute(&mut *grant_tx)
                .await?;
            sqlx::query(
                "UPDATE public.owner_equity_memberships
                    SET state = $3, transition_actor_user_id = $2,
                        transition_code_commit = $4,
                        transition_entitlement_sha256 = $5,
                        updated_at = pg_catalog.clock_timestamp()
                  WHERE id = $1 AND owner_user_id = $2",
            )
            .bind(second_membership)
            .bind(fixture.other_owner_user_id)
            .bind(state)
            .bind(support::CODE_COMMIT)
            .bind(support::TRANSITION_HASH)
            .execute(&mut *grant_tx)
            .await?;
        }
        sqlx::query(
            "INSERT INTO public.owner_equity_instrument_generations
                (id, membership_id, owner_user_id, instrument_id, generation,
                 target_observed_sessions, minimum_observed_sessions,
                 observed_sessions, first_session, last_session)
             VALUES ($1, $2, $3, $4, 1, 261, 121, 121, $5 - 120, $5)",
        )
        .bind(second_generation)
        .bind(second_membership)
        .bind(fixture.other_owner_user_id)
        .bind(format!("{symbol}.KRX"))
        .bind(fixture.session_date)
        .execute(&mut *grant_tx)
        .await?;
        sqlx::query(
            "INSERT INTO public.owner_equity_generation_admissions
                (generation_id, owner_user_id, membership_id, instrument_id,
                 generation, raw_manifest_sha256, artifact_manifest_sha256,
                 entitlement_sha256, capture_code_commit, materializer_code_commit)
             VALUES ($1, $2, $3, $4, 1, $5, $5, $6, $7, $7)",
        )
        .bind(second_generation)
        .bind(fixture.other_owner_user_id)
        .bind(second_membership)
        .bind(format!("{symbol}.KRX"))
        .bind(support::TRANSITION_HASH)
        .bind(support::TRANSITION_HASH)
        .bind(support::CODE_COMMIT)
        .execute(&mut *grant_tx)
        .await?;
        for state in ["MATERIALIZING", "READY"] {
            sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
                .bind(fixture.other_owner_user_id.to_string())
                .execute(&mut *grant_tx)
                .await?;
            sqlx::query(
                "UPDATE public.owner_equity_memberships
                    SET state = $3, transition_actor_user_id = $2,
                        transition_code_commit = $4,
                        transition_entitlement_sha256 = $5,
                        updated_at = pg_catalog.clock_timestamp()
                  WHERE id = $1 AND owner_user_id = $2",
            )
            .bind(second_membership)
            .bind(fixture.other_owner_user_id)
            .bind(state)
            .bind(support::CODE_COMMIT)
            .bind(support::TRANSITION_HASH)
            .execute(&mut *grant_tx)
            .await?;
        }
        grant_tx.commit().await?;
        assert_eq!(cloned_revision, fixture.grant_revision);

        let _other_owner_lease = app
            .replace_stream_lease(
                fixture.other_owner_user_id,
                &fixture.other_session_hash,
                &StreamLeaseRequest::new(
                    Uuid::new_v4(),
                    0,
                    vec![StreamLeaseIdentity::new(
                        second_membership,
                        format!("{symbol}.KRX"),
                        1,
                    )?],
                    "c3a-cross-slot-owner-b-demand".to_owned(),
                )?,
            )
            .await?;

        let producer_a = worker
            .claim_stream_producer(
                fixture.credential_slot_id,
                Uuid::new_v4(),
                fixture.grant_revision,
            )
            .await?;
        let producer_b = worker
            .claim_stream_producer(second_slot, Uuid::new_v4(), cloned_revision)
            .await?;
        assert_ne!(producer_a.credential_slot_id, producer_b.credential_slot_id);
        assert_eq!(producer_a.grant_revision, producer_b.grant_revision);

        let database_now = support::now(database).await?;
        let mut transport = support::loopback_session_for_symbols(
            fixture.session_date,
            database_now.timestamp_millis(),
            second_slot,
            vec![symbol.clone()],
            Duration::from_secs(5),
        )
        .await?;
        let shared_epoch = transport.session.epoch().uuid();
        worker
            .start_stream_epoch(&producer_b, fixture.session.clone(), shared_epoch)
            .await?;
        worker
            .start_stream_epoch(&producer_a, fixture.session.clone(), shared_epoch)
            .await?;
        worker
            .set_subscription_desired(&producer_a, shared_epoch, &symbol, 1)
            .await?;
        worker
            .set_subscription_desired(&producer_b, shared_epoch, &symbol, 1)
            .await?;

        let prepared = transport
            .session
            .prepare_subscribe(&symbol)?
            .ok_or("slot B subscribe unexpectedly became a no-op")?;
        let pending = worker
            .commit_prepared_subscription(&producer_b, &prepared)
            .await?;
        let ack = transport.session.send_prepared(prepared).await?;
        assert_eq!(ack.credential_slot_id(), second_slot);
        assert_eq!(ack.epoch().uuid(), shared_epoch);
        assert_eq!(ack.symbol(), symbol);
        assert_eq!(ack.operation(), MarketSubscriptionOperation::Subscribe);

        let slot_a_subscription_before = read_subscription_snapshot(
            database,
            fixture.owner_user_id,
            fixture.credential_slot_id,
            &symbol,
        )
        .await?
        .ok_or("slot A subscription row was not found")?;
        let slot_b_subscription_before =
            read_subscription_snapshot(database, fixture.other_owner_user_id, second_slot, &symbol)
                .await?
                .ok_or("slot B pending subscription row was not found")?;
        let slot_a_producer_before =
            read_producer_snapshot(database, fixture.owner_user_id, fixture.credential_slot_id)
                .await?
                .ok_or("slot A producer row was not found")?;
        let slot_b_producer_before =
            read_producer_snapshot(database, fixture.other_owner_user_id, second_slot)
                .await?
                .ok_or("slot B producer row was not found")?;

        let reserved_at = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(i64::try_from(
            ack.reserved_at_ms(),
        )?)
        .ok_or("authentic ACK reservation timestamp was out of range")?;
        let deadline = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(i64::try_from(
            ack.deadline_at_ms(),
        )?)
        .ok_or("authentic ACK deadline timestamp was out of range")?;
        let ack_ordinal = i64::try_from(ack.ordinal())?;
        assert_eq!(slot_b_subscription_before.credential_slot_id, second_slot);
        assert_eq!(
            slot_b_subscription_before.grant_revision,
            fixture.grant_revision
        );
        assert_eq!(slot_b_subscription_before.epoch, Some(shared_epoch));
        assert_eq!(slot_b_subscription_before.state, "PENDING_SUBSCRIBE");
        assert_eq!(
            slot_b_subscription_before.pending_operation.as_deref(),
            Some("SUBSCRIBE")
        );
        assert_eq!(
            slot_b_subscription_before.pending_ordinal,
            Some(ack_ordinal)
        );
        assert_eq!(
            slot_b_subscription_before.pending_reserved_at,
            Some(reserved_at)
        );
        assert_eq!(slot_b_subscription_before.pending_deadline, Some(deadline));
        assert!(reserved_at < deadline);
        assert_eq!(slot_b_subscription_before.desired_reference_count, 1);
        assert_ne!(
            slot_b_subscription_before.subscription_revision,
            Uuid::nil()
        );
        assert!(slot_b_subscription_before.acked_at.is_none());
        assert_eq!(slot_a_subscription_before.state, "DESIRED");
        assert!(slot_a_subscription_before.pending_operation.is_none());
        assert!(slot_a_subscription_before.pending_ordinal.is_none());
        assert!(slot_a_subscription_before.pending_reserved_at.is_none());
        assert!(slot_a_subscription_before.pending_deadline.is_none());
        assert_eq!(slot_a_subscription_before.desired_reference_count, 1);
        assert!(slot_a_subscription_before.acked_at.is_none());
        assert_eq!(slot_a_producer_before.2, Some(shared_epoch));
        assert_eq!(slot_b_producer_before.2, Some(shared_epoch));

        let cross_slot_result = worker
            .commit_subscription_ack(&producer_a, pending, ack)
            .await;
        match cross_slot_result {
            Err(error) => assert_eq!(error, MarketStreamStorageError::SubscriptionInvalid),
            Ok(None) => return Err("cross-slot ACK returned success without a proof".into()),
            Ok(Some(_)) => return Err("cross-slot ACK returned a publication proof".into()),
        }

        let slot_a_subscription_after = read_subscription_snapshot(
            database,
            fixture.owner_user_id,
            fixture.credential_slot_id,
            &symbol,
        )
        .await?
        .ok_or("slot A subscription row disappeared after rejection")?;
        let slot_b_subscription_after =
            read_subscription_snapshot(database, fixture.other_owner_user_id, second_slot, &symbol)
                .await?
                .ok_or("slot B pending row disappeared after rejection")?;
        let slot_a_producer_after =
            read_producer_snapshot(database, fixture.owner_user_id, fixture.credential_slot_id)
                .await?
                .ok_or("slot A producer row disappeared after rejection")?;
        let slot_b_producer_after =
            read_producer_snapshot(database, fixture.other_owner_user_id, second_slot)
                .await?
                .ok_or("slot B producer row disappeared after rejection")?;
        assert_eq!(slot_a_subscription_after, slot_a_subscription_before);
        assert_eq!(slot_b_subscription_after, slot_b_subscription_before);
        assert_eq!(slot_a_producer_after, slot_a_producer_before);
        assert_eq!(slot_b_producer_after, slot_b_producer_before);
        assert_eq!(slot_b_subscription_after.state, "PENDING_SUBSCRIBE");
        assert_eq!(
            slot_b_subscription_after.pending_operation.as_deref(),
            Some("SUBSCRIBE")
        );
        assert_eq!(slot_b_subscription_after.pending_ordinal, Some(ack_ordinal));
        assert_eq!(
            slot_b_subscription_after.pending_reserved_at,
            Some(reserved_at)
        );
        assert_eq!(slot_b_subscription_after.pending_deadline, Some(deadline));
        assert_eq!(
            slot_b_subscription_after.grant_revision,
            fixture.grant_revision
        );
        assert_eq!(slot_b_subscription_after.desired_reference_count, 1);
        assert!(slot_b_subscription_after.acked_at.is_none());

        transport.close().await?;
        Ok(())
    }

    let database = support::DisposableDatabase::create().await?;
    let (database, database_name) = prepare_database_cleanup(database).await?;
    let task_database = database.clone();
    let result = tokio::spawn(async move { run(&task_database).await }).await;
    let (result, cleanup) = cleanup_after_join(database, database_name, result).await;
    finish_joined_test(result, cleanup, "C3A cross-slot ACK regression")
}

#[cfg(feature = "market-stream-db-tests")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owner_market_stream_panic_body_still_cleans_generated_database() -> C3aTestResult {
    async fn intentional_panic() -> C3aTestResult {
        panic!("intentional C3A cleanup control panic")
    }

    let database = support::DisposableDatabase::create().await?;
    let (database, database_name) = prepare_database_cleanup(database).await?;
    let task_database = database.clone();
    let result = tokio::spawn(async move {
        let _database_lifetime = task_database;
        intentional_panic().await
    })
    .await;
    let (result, cleanup) = cleanup_after_join(database, database_name, result).await;
    match (result, cleanup) {
        (Err(join_error), Ok(())) if join_error.is_panic() => Ok(()),
        (Err(join_error), Ok(())) => Err(format!(
            "intentional cleanup control did not observe a panic JoinError: {join_error}"
        )
        .into()),
        (Ok(Ok(())), Ok(())) => {
            Err("intentional cleanup control body unexpectedly returned success".into())
        }
        (Ok(Err(test_error)), Ok(())) => Err(format!(
            "intentional cleanup control returned an error instead of panicking: {test_error}"
        )
        .into()),
        (task_result, Err(cleanup_error)) => Err(format!(
            "intentional cleanup control task result {task_result:?}; cleanup failed: {cleanup_error}"
        )
        .into()),
    }
}
