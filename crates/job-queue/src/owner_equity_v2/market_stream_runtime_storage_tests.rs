//! Focused runtime-storage contract tests.
//!
//! The `unit` module has no storage/network inputs. `database_cases` compiles
//! only with `market-stream-db-tests`; C1 deliberately does not execute it.

use chrono::{DateTime, Duration as ChronoDuration, FixedOffset, NaiveDate, TimeZone, Utc};

fn snapshot_fixture_bounds(
    session_date: NaiveDate,
    now: DateTime<Utc>,
) -> Result<(DateTime<Utc>, DateTime<Utc>), super::market_stream_runtime::MarketStreamRuntimeError>
{
    use super::market_stream_runtime::MarketStreamRuntimeError;

    let kst = FixedOffset::east_opt(9 * 60 * 60).ok_or(MarketStreamRuntimeError::DayInvalid)?;
    if now.with_timezone(&kst).date_naive() != session_date {
        return Err(MarketStreamRuntimeError::DayInvalid);
    }

    let day_start = kst
        .from_local_datetime(
            &session_date
                .and_hms_opt(0, 0, 0)
                .ok_or(MarketStreamRuntimeError::DayInvalid)?,
        )
        .single()
        .ok_or(MarketStreamRuntimeError::DayInvalid)?
        .with_timezone(&Utc);
    let day_last_second = kst
        .from_local_datetime(
            &session_date
                .and_hms_opt(23, 59, 59)
                .ok_or(MarketStreamRuntimeError::DayInvalid)?,
        )
        .single()
        .ok_or(MarketStreamRuntimeError::DayInvalid)?
        .with_timezone(&Utc);

    let open_at = if now.signed_duration_since(day_start) <= ChronoDuration::seconds(60) {
        day_start
    } else {
        now - ChronoDuration::seconds(60)
    };
    let close_at = if day_last_second.signed_duration_since(now) <= ChronoDuration::hours(1) {
        day_last_second
    } else {
        now + ChronoDuration::hours(1)
    };

    if open_at > now || now >= close_at || open_at >= close_at {
        return Err(MarketStreamRuntimeError::WindowUnavailable);
    }
    Ok((open_at, close_at))
}

mod unit {
    use chrono::{DateTime, Duration as ChronoDuration, NaiveDate, Utc};
    use uuid::Uuid;

    use super::super::market_stream::{RuntimeTransition, StreamAvailability};
    use super::super::market_stream_runtime::{
        MarketStreamRuntimeError, OwnerMarketStreamRuntimeConfig, fixture_day,
    };
    use super::snapshot_fixture_bounds;
    use crate::owner_equity_v2::intraday::IntradaySessionProof;

    #[test]
    fn snapshot_fixture_window_stays_inside_kst_date() {
        let session_date = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let calendar = IntradaySessionProof::new(
            session_date,
            Uuid::from_u128(1),
            "a".repeat(64),
            format!("sha256:{}", "b".repeat(64)),
        )
        .unwrap();
        let cases = [
            (
                "2026-10-01T14:16:50Z",
                "2026-10-01T14:15:50Z",
                "2026-10-01T14:59:59Z",
                231550,
                235959,
                true,
            ),
            (
                "2026-09-30T15:00:30Z",
                "2026-09-30T15:00:00Z",
                "2026-09-30T16:00:30Z",
                0,
                10030,
                true,
            ),
            (
                "2026-10-01T00:00:00Z",
                "2026-09-30T23:59:00Z",
                "2026-10-01T01:00:00Z",
                85900,
                100000,
                false,
            ),
        ];

        for (now_text, open_text, close_text, start_hms, end_hms, old_bounds_rejected) in cases {
            let now = DateTime::parse_from_rfc3339(now_text)
                .unwrap()
                .with_timezone(&Utc);
            let expected_open = DateTime::parse_from_rfc3339(open_text)
                .unwrap()
                .with_timezone(&Utc);
            let expected_close = DateTime::parse_from_rfc3339(close_text)
                .unwrap()
                .with_timezone(&Utc);
            let (open_at, close_at) = snapshot_fixture_bounds(session_date, now).unwrap();

            assert_eq!(open_at, expected_open);
            assert_eq!(close_at, expected_close);
            assert!(open_at <= now && now < close_at);

            let day = fixture_day(calendar.clone(), open_at, close_at, Uuid::from_u128(2))
                .expect("clamped synthetic day passes the existing constructors");
            assert_eq!(day.session_date(), session_date);
            assert_eq!(day.calendar().session_date, session_date);
            assert_eq!(day.session().session_date, session_date);
            let transport = day.transport_proof();
            assert_eq!(transport.session_date(), 20261001);
            assert_eq!(transport.session_start_hhmmss(), start_hms);
            assert_eq!(transport.session_end_hhmmss(), end_hms);

            if old_bounds_rejected {
                assert_eq!(
                    fixture_day(
                        calendar.clone(),
                        now - ChronoDuration::seconds(60),
                        now + ChronoDuration::hours(1),
                        Uuid::from_u128(3),
                    ),
                    Err(MarketStreamRuntimeError::WindowUnavailable),
                    "the former unclamped relative bounds cross the KST date"
                );
            }
        }
    }

    #[test]
    fn snapshot_fixture_window_rejects_wrong_day_and_close_boundary() {
        let session_date = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let calendar = IntradaySessionProof::new(
            session_date,
            Uuid::from_u128(1),
            "a".repeat(64),
            format!("sha256:{}", "b".repeat(64)),
        )
        .unwrap();
        let wrong_day = DateTime::parse_from_rfc3339("2026-10-02T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            snapshot_fixture_bounds(session_date, wrong_day),
            Err(MarketStreamRuntimeError::DayInvalid)
        );

        for now_text in ["2026-10-01T14:59:59Z", "2026-10-01T14:59:59.000000001Z"] {
            let now = DateTime::parse_from_rfc3339(now_text)
                .unwrap()
                .with_timezone(&Utc);
            assert_eq!(
                now.with_timezone(&chrono::FixedOffset::east_opt(9 * 60 * 60).unwrap())
                    .date_naive(),
                session_date
            );
            assert_eq!(
                snapshot_fixture_bounds(session_date, now),
                Err(MarketStreamRuntimeError::WindowUnavailable)
            );
        }

        let valid = DateTime::parse_from_rfc3339("2026-10-01T03:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            fixture_day(calendar, valid, valid, Uuid::from_u128(4)),
            Err(MarketStreamRuntimeError::WindowUnavailable),
            "the existing half-open day constructor still rejects an empty interval"
        );
    }

    #[test]
    fn runtime_config_accepts_only_the_canonical_fixed_pin_shape() {
        let slot = Uuid::new_v4();
        let grant = Uuid::new_v4();
        let holder = Uuid::new_v4();
        let hash = "a".repeat(64);
        assert!(OwnerMarketStreamRuntimeConfig::from_values(slot, grant, &hash, 1, holder).is_ok());
    }

    #[test]
    fn runtime_config_rejects_nil_ids_zero_generation_and_noncanonical_hash() {
        let slot = Uuid::new_v4();
        let grant = Uuid::new_v4();
        let holder = Uuid::new_v4();
        let hash = "a".repeat(64);
        for result in [
            OwnerMarketStreamRuntimeConfig::from_values(Uuid::nil(), grant, &hash, 1, holder),
            OwnerMarketStreamRuntimeConfig::from_values(slot, Uuid::nil(), &hash, 1, holder),
            OwnerMarketStreamRuntimeConfig::from_values(slot, grant, &hash, 1, Uuid::nil()),
            OwnerMarketStreamRuntimeConfig::from_values(slot, grant, &hash, 0, holder),
            OwnerMarketStreamRuntimeConfig::from_values(
                slot,
                grant,
                &hash.to_ascii_uppercase(),
                1,
                holder,
            ),
            OwnerMarketStreamRuntimeConfig::from_values(slot, grant, "sha256:invalid", 1, holder),
        ] {
            assert_eq!(result, Err(MarketStreamRuntimeError::ConfigurationInvalid));
        }
    }

    #[test]
    fn pre_ack_transitions_never_create_live_availability() {
        for transition in [
            RuntimeTransition::Connecting,
            RuntimeTransition::AwaitingFirstTrade,
            RuntimeTransition::SubscriptionPending,
            RuntimeTransition::SubscriptionRejected,
            RuntimeTransition::SubscriptionAmbiguous,
        ] {
            assert_ne!(transition.values().availability, StreamAvailability::Live);
        }
    }
}

#[cfg(feature = "market-stream-db-tests")]
#[path = "../../tests/owner_market_stream_boundary_support/mod.rs"]
mod database_support;

#[cfg(feature = "market-stream-db-tests")]
mod database_cases {
    use std::error::Error;
    use std::future::Future;
    use std::pin::Pin;
    use std::time::{Duration, Instant};

    use chrono::{DateTime, Duration as ChronoDuration, Utc};
    use sqlx::PgPool;
    use uuid::Uuid;

    use super::super::market_stream::{
        MarketStreamStorageError, OwnerMarketStreamRepository, RuntimeTransition, StreamCacheRow,
        StreamEpochProof, StreamIdentity, StreamLease, StreamLeaseIdentity, StreamLeaseRequest,
        StreamProducerLease, StreamPublicationContext, StreamPublicationObservation,
        StreamSnapshot, StreamStatusCode, clear_resolved_commit_test_gate,
        register_resolved_commit_test_gate,
    };
    use super::super::market_stream_runtime::{
        OwnerMarketStreamRuntimeConfig, ResolvedMarketStreamDay, fixture_day,
        snapshot_window_fixture,
    };
    use super::database_support as support;
    use crate::owner_equity_v2::intraday::IntradaySessionProof;

    type TestError = Box<dyn Error + Send + Sync>;
    type TestResult = Result<(), TestError>;
    type CaseFuture = Pin<Box<dyn Future<Output = TestResult> + Send>>;
    type DatabaseCase = fn(support::DisposableDatabase, support::Fixture) -> CaseFuture;

    #[derive(Debug)]
    struct CloseCaseFailure {
        primary: Option<TestError>,
        cleanup: Vec<TestError>,
    }

    impl std::fmt::Display for CloseCaseFailure {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            if let Some(primary) = &self.primary {
                write!(formatter, "close-crossing case failed: {primary}")?;
            }
            for (index, error) in self.cleanup.iter().enumerate() {
                if index == 0 && self.primary.is_none() {
                    write!(formatter, "close-crossing cleanup failed: {error}")?;
                } else {
                    write!(formatter, "; cleanup failure {}: {error}", index + 1)?;
                }
            }
            Ok(())
        }
    }

    impl Error for CloseCaseFailure {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            self.primary
                .as_ref()
                .map(|error| error.as_ref() as &(dyn Error + 'static))
                .or_else(|| {
                    self.cleanup
                        .first()
                        .map(|error| error.as_ref() as &(dyn Error + 'static))
                })
        }
    }

    fn test_error(message: impl Into<String>) -> TestError {
        Box::new(std::io::Error::other(message.into()))
    }

    async fn database_clock_now(
        database: &support::DisposableDatabase,
    ) -> Result<DateTime<Utc>, TestError> {
        tokio::time::timeout(Duration::from_secs(2), support::now(database))
            .await
            .map_err(|_| test_error("database clock observation exceeded two seconds"))?
            .map_err(|error| Box::new(error) as TestError)
    }

    fn finish_close_case(primary: Option<TestError>, cleanup: Vec<TestError>) -> TestResult {
        if cleanup.is_empty() {
            return primary.map_or(Ok(()), Err);
        }
        Err(Box::new(CloseCaseFailure { primary, cleanup }))
    }

    async fn close_loopback_with_primary(
        transport: support::LoopbackMarketSession,
        primary: TestError,
    ) -> TestResult {
        let cleanup = match transport.close().await {
            Ok(()) => Vec::new(),
            Err(error) => vec![error],
        };
        finish_close_case(Some(primary), cleanup)
    }

    struct ResolvedTaskFinish<T> {
        output: Option<T>,
        release_sent: Option<bool>,
        aborted: bool,
        error: Option<TestError>,
    }

    async fn finish_resolved_commit_task<T>(
        key: Uuid,
        mut task: tokio::task::JoinHandle<T>,
        release: Option<tokio::sync::oneshot::Sender<()>>,
        abort: bool,
    ) -> ResolvedTaskFinish<T>
    where
        T: Send + 'static,
    {
        let (release_sent, aborted) = if abort {
            task.abort();
            drop(release);
            (None, true)
        } else if let Some(release) = release {
            let sent = release.send(()).is_ok();
            if sent {
                (Some(true), false)
            } else {
                task.abort();
                (Some(false), true)
            }
        } else {
            task.abort();
            (Some(false), true)
        };

        let joined = tokio::time::timeout(Duration::from_secs(15), &mut task).await;
        let (output, error) = match joined {
            Ok(Ok(output)) => (Some(output), None),
            Ok(Err(join_error)) if aborted && join_error.is_cancelled() => (None, None),
            Ok(Err(join_error)) => (None, Some(Box::new(join_error) as TestError)),
            Err(_) => {
                task.abort();
                let timeout_error: TestError = Box::new(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "resolved operation child exceeded its 15 second join bound",
                ));
                match task.await {
                    Err(join_error) if join_error.is_cancelled() => (None, Some(timeout_error)),
                    Ok(output) => (
                        Some(output),
                        Some(Box::new(CloseCaseFailure {
                            primary: Some(timeout_error),
                            cleanup: vec![test_error(
                                "resolved operation child completed while abort was pending",
                            )],
                        }) as TestError),
                    ),
                    Err(join_error) => (
                        None,
                        Some(Box::new(CloseCaseFailure {
                            primary: Some(timeout_error),
                            cleanup: vec![Box::new(join_error)],
                        }) as TestError),
                    ),
                }
            }
        };
        clear_resolved_commit_test_gate(key);
        ResolvedTaskFinish {
            output,
            release_sent,
            aborted,
            error,
        }
    }

    fn aborted_operation_output_error<T>(
        output: Result<T, MarketStreamStorageError>,
        label: &str,
    ) -> TestError {
        match output {
            Ok(_) => test_error(format!("{label} child completed while cleanup aborted it")),
            Err(error) => Box::new(error),
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct QuoteCapture {
        quote_version: u64,
        received_at: DateTime<Utc>,
        committed_at: DateTime<Utc>,
    }

    async fn generated_database_name(
        database: &support::DisposableDatabase,
    ) -> Result<String, sqlx::Error> {
        sqlx::query_scalar("SELECT pg_catalog.current_database()")
            .fetch_one(&database.migration_owner)
            .await
    }

    async fn assert_generated_database_absent(name: &str) -> TestResult {
        let supervisor = support::connect_supervisor_from_environment().await?;
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_database WHERE datname = $1)",
        )
        .bind(name)
        .fetch_one(&supervisor)
        .await?;
        let connections: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint FROM pg_catalog.pg_stat_activity WHERE datname = $1",
        )
        .bind(name)
        .fetch_one(&supervisor)
        .await?;
        supervisor.close().await;
        if exists || connections != 0 {
            return Err("generated database cleanup did not leave exact absence".into());
        }
        Ok(())
    }

    async fn run_database_case(case: DatabaseCase) -> TestResult {
        let database = support::DisposableDatabase::create().await?;
        let name = match generated_database_name(&database).await {
            Ok(name) => name,
            Err(error) => {
                database.cleanup().await?;
                return Err(Box::new(error));
            }
        };
        let task_database = database.clone();
        let task = tokio::spawn(async move {
            let fixture = support::seed_fixture(&task_database).await?;
            case(task_database, fixture).await
        });
        let task_result = task.await;
        let cleanup_result = database.cleanup().await;
        let absence_result = assert_generated_database_absent(&name).await;
        match (task_result, cleanup_result, absence_result) {
            (Ok(Ok(())), Ok(()), Ok(())) => Ok(()),
            (Ok(Err(error)), Ok(()), Ok(())) => Err(error),
            (Err(join_error), Ok(()), Ok(())) => Err(Box::new(join_error)),
            (task_result, cleanup_result, absence_result) => Err(format!(
                "database case task={task_result:?}; cleanup={cleanup_result:?}; exact_absence={absence_result:?}"
            )
            .into()),
        }
    }

    fn single_lease_identity(
        identity: &StreamIdentity,
    ) -> Result<StreamLeaseIdentity, MarketStreamStorageError> {
        StreamLeaseIdentity::new(
            identity.membership_id,
            identity.instrument_id.clone(),
            identity.generation,
        )
    }

    async fn create_lease(
        database: &support::DisposableDatabase,
        fixture: &support::Fixture,
        identities: &[StreamIdentity],
    ) -> Result<StreamLease, MarketStreamStorageError> {
        let request = StreamLeaseRequest::new(
            Uuid::new_v4(),
            0,
            identities
                .iter()
                .map(single_lease_identity)
                .collect::<Result<Vec<_>, _>>()?,
            format!("runtime-test-{}", Uuid::new_v4()),
        )?;
        let lease = OwnerMarketStreamRepository::new(database.app.clone())
            .replace_stream_lease(fixture.owner_user_id, &fixture.owner_session_hash, &request)
            .await?;
        assert_eq!(lease.renewal_sequence, 0);
        Ok(lease)
    }

    async fn start_pre_ack_epoch(
        database: &support::DisposableDatabase,
        fixture: &support::Fixture,
        producer: &StreamProducerLease,
        identities: &[StreamIdentity],
    ) -> Result<(Uuid, StreamEpochProof), MarketStreamStorageError> {
        let epoch = Uuid::new_v4();
        let worker = OwnerMarketStreamRepository::new(database.worker.clone());
        let proof = worker
            .start_stream_epoch(producer, fixture.session.clone(), epoch)
            .await?;
        for symbol in identities
            .iter()
            .map(|identity| identity.symbol().to_owned())
            .collect::<std::collections::BTreeSet<_>>()
        {
            worker
                .set_subscription_desired(producer, epoch, &symbol, 1)
                .await?;
        }
        Ok((epoch, proof))
    }

    async fn publish_one_quote(
        database: &support::DisposableDatabase,
        fixture: &support::Fixture,
        producer: &StreamProducerLease,
        identity: &StreamIdentity,
    ) -> Result<(Uuid, StreamEpochProof), TestError> {
        let worker = OwnerMarketStreamRepository::new(database.worker.clone());
        let now = database_clock_now(database).await?;
        let mut transport = support::loopback_session_for_symbols(
            fixture.session_date,
            now.timestamp_millis(),
            producer.credential_slot_id,
            vec![identity.symbol().to_owned()],
            Duration::from_secs(5),
        )
        .await?;
        let epoch = transport.session.epoch().uuid();
        let epoch_proof = worker
            .start_stream_epoch(producer, fixture.session.clone(), epoch)
            .await?;
        let symbol = identity.symbol().to_owned();
        worker
            .set_subscription_desired(producer, epoch, &symbol, 1)
            .await?;
        let prepared = transport
            .session
            .prepare_subscribe(&symbol)?
            .ok_or("expected a real subscribe command")?;
        let pending = worker
            .commit_prepared_subscription(producer, &prepared)
            .await?;
        let ack = transport.session.send_prepared(prepared).await?;
        let subscription = worker
            .commit_subscription_ack(producer, pending, ack)
            .await?
            .ok_or("expected a committed subscribe ACK")?;
        let receipt = transport.next_receipt().await?;
        let context = StreamPublicationContext::single(
            producer.clone(),
            epoch_proof.clone(),
            identity.clone(),
            subscription,
        )?;
        worker
            .publish_stream_latest(
                &context,
                &[StreamPublicationObservation::from_receipt(receipt)],
            )
            .await?;
        transport.close().await?;
        Ok((epoch, epoch_proof))
    }

    async fn resolved_context_with_transport(
        database: &support::DisposableDatabase,
        fixture: &support::Fixture,
        producer: &StreamProducerLease,
        identity: &StreamIdentity,
        day: &ResolvedMarketStreamDay,
        close_lifetime: Duration,
    ) -> Result<(support::LoopbackMarketSession, StreamPublicationContext), TestError> {
        let now = database_clock_now(database).await?;
        let mut transport = support::loopback_session_for_symbols_with_close_lifetime(
            fixture.session_date,
            now.timestamp_millis(),
            producer.credential_slot_id,
            vec![identity.symbol().to_owned()],
            Duration::from_secs(5),
            close_lifetime,
        )
        .await?;
        let setup_result: Result<StreamPublicationContext, TestError> = async {
            let worker = OwnerMarketStreamRepository::new(database.worker.clone());
            let epoch = transport.session.epoch().uuid();
            let proof = worker
                .start_stream_epoch_resolved(producer, day, epoch)
                .await?;
            let symbol = identity.symbol().to_owned();
            worker
                .set_subscription_desired(producer, epoch, &symbol, 1)
                .await?;
            let prepared = transport
                .session
                .prepare_subscribe(&symbol)?
                .ok_or("expected a real subscribe command")?;
            let pending = worker
                .commit_prepared_subscription(producer, &prepared)
                .await?;
            let ack = transport.session.send_prepared(prepared).await?;
            let subscription = worker
                .commit_subscription_ack(producer, pending, ack)
                .await?
                .ok_or("expected a committed subscribe ACK")?;
            Ok(StreamPublicationContext::single(
                producer.clone(),
                proof,
                identity.clone(),
                subscription,
            )?)
        }
        .await;
        match setup_result {
            Ok(context) => Ok((transport, context)),
            Err(error) => match transport.close().await {
                Ok(()) => Err(error),
                Err(cleanup) => Err(support::combine_primary_cleanup(error, cleanup)),
            },
        }
    }

    async fn publish_resolved_quote(
        database: &support::DisposableDatabase,
        fixture: &support::Fixture,
        producer: &StreamProducerLease,
        identity: &StreamIdentity,
        day: &ResolvedMarketStreamDay,
    ) -> Result<StreamEpochProof, TestError> {
        let (mut transport, context) = resolved_context_with_transport(
            database,
            fixture,
            producer,
            identity,
            day,
            Duration::from_secs(2),
        )
        .await?;
        let publish_result: Result<StreamEpochProof, TestError> = async {
            let receipt = transport.next_receipt().await?;
            OwnerMarketStreamRepository::new(database.worker.clone())
                .publish_stream_latest_resolved(
                    &context,
                    day,
                    &[StreamPublicationObservation::from_receipt(receipt)],
                )
                .await?;
            Ok(context.epoch.clone())
        }
        .await;
        transport.close().await?;
        publish_result
    }

    async fn read_snapshot_with_window(
        database: &support::DisposableDatabase,
        fixture: &support::Fixture,
        lease: &StreamLease,
        window: super::super::market_stream_runtime::SnapshotWindowEvidence,
    ) -> Result<StreamSnapshot, MarketStreamStorageError> {
        OwnerMarketStreamRepository::new(database.app.clone())
            .read_stream_snapshot_with_window_fixture(
                fixture.owner_user_id,
                &fixture.owner_session_hash,
                lease.lease_id,
                Some(window),
            )
            .await
    }

    async fn seeded_resolved_snapshot(
        database: &support::DisposableDatabase,
        fixture: &support::Fixture,
    ) -> Result<
        (
            StreamLease,
            StreamProducerLease,
            ResolvedMarketStreamDay,
            super::super::market_stream_runtime::SnapshotWindowEvidence,
            Uuid,
        ),
        TestError,
    > {
        let identity = fixture.identities[0].clone();
        let lease = create_lease(database, fixture, std::slice::from_ref(&identity)).await?;
        let producer = OwnerMarketStreamRepository::new(database.worker.clone())
            .claim_stream_producer(
                fixture.credential_slot_id,
                Uuid::new_v4(),
                fixture.grant_revision,
            )
            .await?;
        let calendar = IntradaySessionProof::new(
            fixture.session_date,
            fixture.session.calendar_source_batch_id,
            fixture.session.calendar_content_sha256.clone(),
            fixture.session.window_contract_sha256.clone(),
        )?;
        let now = database_clock_now(database).await?;
        seed_calendar_lineage(database, &calendar, now).await?;
        let (open_at, close_at) = super::snapshot_fixture_bounds(fixture.session_date, now)?;
        let day = fixture_day(calendar, open_at, close_at, Uuid::new_v4())?;
        let epoch = publish_resolved_quote(database, fixture, &producer, &identity, &day)
            .await?
            .epoch;
        let window = snapshot_window_fixture(
            day.session_date(),
            day.open_at(),
            day.close_at(),
            fixture.session.window_contract_sha256.clone(),
        );
        Ok((lease, producer, day, window, epoch))
    }

    fn quote_capture(row: &StreamCacheRow) -> Result<QuoteCapture, TestError> {
        Ok(QuoteCapture {
            quote_version: row.quote_version,
            received_at: row.received_at.ok_or("committed row lacks received_at")?,
            committed_at: row.committed_at.ok_or("committed row lacks committed_at")?,
        })
    }

    fn snapshot_quote_capture(snapshot: &StreamSnapshot) -> Result<QuoteCapture, TestError> {
        let delivery = snapshot
            .delivery_rows
            .first()
            .ok_or("authorized delivery row is missing")?;
        quote_capture(
            delivery
                .cache
                .as_ref()
                .ok_or("committed delivery cache is missing")?,
        )
    }

    #[derive(Clone, Copy)]
    enum SnapshotOverride {
        StaleHeartbeat,
        ShortProducerLease,
        EpochMismatch,
        MissingAck,
        ClosedWindow,
        LineageMismatch,
    }

    async fn apply_snapshot_override(
        database: &support::DisposableDatabase,
        identity: &StreamIdentity,
        producer: &StreamProducerLease,
        epoch: Uuid,
        window: &super::super::market_stream_runtime::SnapshotWindowEvidence,
        override_case: SnapshotOverride,
    ) -> Result<super::super::market_stream_runtime::SnapshotWindowEvidence, TestError> {
        match override_case {
            SnapshotOverride::StaleHeartbeat => {
                let result = sqlx::query(
                    "UPDATE public.owner_market_stream_producers
                        SET heartbeat_at = pg_catalog.clock_timestamp() - INTERVAL '11 seconds'
                      WHERE credential_slot_id = $1",
                )
                .bind(producer.credential_slot_id)
                .execute(&database.migration_owner)
                .await?;
                assert_eq!(result.rows_affected(), 1);
                Ok(window.clone())
            }
            SnapshotOverride::ShortProducerLease => {
                let result = sqlx::query(
                    "UPDATE public.owner_market_stream_producers
                        SET lease_expires_at = pg_catalog.clock_timestamp() + INTERVAL '5 seconds'
                      WHERE credential_slot_id = $1",
                )
                .bind(producer.credential_slot_id)
                .execute(&database.migration_owner)
                .await?;
                assert_eq!(result.rows_affected(), 1);
                Ok(window.clone())
            }
            SnapshotOverride::EpochMismatch => {
                let result = sqlx::query(
                    "UPDATE public.owner_market_stream_producers
                        SET current_epoch = $2
                      WHERE credential_slot_id = $1 AND current_epoch = $3",
                )
                .bind(producer.credential_slot_id)
                .bind(Uuid::new_v4())
                .bind(epoch)
                .execute(&database.migration_owner)
                .await?;
                assert_eq!(result.rows_affected(), 1);
                Ok(window.clone())
            }
            SnapshotOverride::MissingAck => {
                let result = sqlx::query(
                    "UPDATE public.owner_market_stream_subscriptions
                        SET state = 'DESIRED', acked_at = NULL
                      WHERE credential_slot_id = $1 AND symbol = $2 AND epoch = $3",
                )
                .bind(producer.credential_slot_id)
                .bind(identity.symbol())
                .bind(epoch)
                .execute(&database.migration_owner)
                .await?;
                assert_eq!(result.rows_affected(), 1);
                Ok(window.clone())
            }
            SnapshotOverride::ClosedWindow => {
                let now = database_clock_now(database).await?;
                Ok(snapshot_window_fixture(
                    window.date(),
                    window.open_at(),
                    now - ChronoDuration::seconds(1),
                    window.contract_sha256().to_owned(),
                ))
            }
            SnapshotOverride::LineageMismatch => {
                let result = sqlx::query(
                    "UPDATE public.owner_market_stream_producers
                        SET session_proof_sha256 = $2
                      WHERE credential_slot_id = $1",
                )
                .bind(producer.credential_slot_id)
                .bind(format!("sha256:{}", "f".repeat(64)))
                .execute(&database.migration_owner)
                .await?;
                assert_eq!(result.rows_affected(), 1);
                Ok(window.clone())
            }
        }
    }

    async fn snapshot_override_case(
        database: support::DisposableDatabase,
        fixture: support::Fixture,
        override_case: SnapshotOverride,
    ) -> TestResult {
        let identity = fixture.identities[0].clone();
        let (lease, producer, _day, window, epoch) =
            seeded_resolved_snapshot(&database, &fixture).await?;
        let baseline =
            read_snapshot_with_window(&database, &fixture, &lease, window.clone()).await?;
        assert_eq!(baseline.delivery_rows.len(), 1);
        assert!(
            baseline.delivery_rows[0].live,
            "valid ACK-backed quote must be LIVE first"
        );
        let capture = snapshot_quote_capture(&baseline)?;
        assert!(capture.quote_version > 0);

        let negative_window = apply_snapshot_override(
            &database,
            &identity,
            &producer,
            epoch,
            &window,
            override_case,
        )
        .await?;
        let negative =
            read_snapshot_with_window(&database, &fixture, &lease, negative_window).await?;
        assert_eq!(negative.delivery_rows.len(), 1);
        assert!(!negative.delivery_rows[0].live);
        assert_eq!(snapshot_quote_capture(&negative)?, capture);
        let after = database_clock_now(&database).await?;
        assert!(after.signed_duration_since(capture.received_at) <= ChronoDuration::seconds(3));
        Ok(())
    }

    async fn wait_for_database_time(
        database: &support::DisposableDatabase,
        target: DateTime<Utc>,
    ) -> TestResult {
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if database_clock_now(database).await? >= target {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|_| test_error("database clock target was not reached within 15 seconds"))?
    }

    async fn observe_epoch_close_before_release(
        database: &support::DisposableDatabase,
        slot_id: Uuid,
        target: DateTime<Utc>,
    ) -> Result<(DateTime<Utc>, DateTime<Utc>), TestError> {
        wait_for_database_time(database, target).await?;
        database_clock_and_producer_expiry(database, slot_id).await
    }

    struct PublicationCloseObservation {
        database_now: DateTime<Utc>,
        producer_expiry: DateTime<Utc>,
        receipt_wall_age: chrono::Duration,
        receipt_monotonic_age: Duration,
    }

    async fn observe_publication_close_before_release(
        database: &support::DisposableDatabase,
        slot_id: Uuid,
        target: DateTime<Utc>,
        received_at: DateTime<Utc>,
        received_monotonic: Instant,
    ) -> Result<PublicationCloseObservation, TestError> {
        wait_for_database_time(database, target).await?;
        let (database_now, producer_expiry) =
            database_clock_and_producer_expiry(database, slot_id).await?;
        let receipt_monotonic_age = Instant::now()
            .checked_duration_since(received_monotonic)
            .ok_or_else(|| test_error("loopback receipt monotonic timestamp is in the future"))?;
        Ok(PublicationCloseObservation {
            database_now,
            producer_expiry,
            receipt_wall_age: database_now.signed_duration_since(received_at),
            receipt_monotonic_age,
        })
    }

    async fn wait_for_resolved_commit_gate(
        entered: tokio::sync::oneshot::Receiver<()>,
    ) -> TestResult {
        match tokio::time::timeout(Duration::from_secs(15), entered).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => Err(test_error(
                "resolved operation ended before its post-lock commit gate",
            )),
            Err(_) => Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "resolved operation did not reach its post-lock commit gate within 15 seconds",
            ))),
        }
    }

    async fn stored_current_epoch(
        database: &support::DisposableDatabase,
        slot_id: Uuid,
    ) -> Result<Option<Uuid>, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT current_epoch FROM public.owner_market_stream_producers
              WHERE credential_slot_id = $1",
        )
        .bind(slot_id)
        .fetch_one(&database.migration_owner)
        .await
    }

    async fn database_clock_and_producer_expiry(
        database: &support::DisposableDatabase,
        slot_id: Uuid,
    ) -> Result<(DateTime<Utc>, DateTime<Utc>), TestError> {
        tokio::time::timeout(
            Duration::from_secs(2),
            sqlx::query_as(
                "SELECT pg_catalog.clock_timestamp(), lease_expires_at
               FROM public.owner_market_stream_producers
              WHERE credential_slot_id = $1",
            )
            .bind(slot_id)
            .fetch_one(&database.migration_owner),
        )
        .await
        .map_err(|_| test_error("producer expiry clock query exceeded two seconds"))?
        .map_err(|error| Box::new(error) as TestError)
    }

    async fn seed_calendar_lineage(
        database: &support::DisposableDatabase,
        calendar: &IntradaySessionProof,
        retrieved_at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        let batch_id = calendar.calendar_source_batch_id;
        let content_sha256 = calendar.calendar_content_sha256.clone();
        sqlx::query(
            "INSERT INTO public.data_batches
                (provider, market, batch_date, kind, storage_path, content_sha256,
                 bytes_size, retrieved_at, source_batch_id, source_file_name, fetch_mode)
             VALUES ('KRX', 'KR', $1, 'CALENDAR', $2, $3, 1, $4, $5,
                     'calendar.json', 'credentialed')",
        )
        .bind(calendar.session_date)
        .bind(format!("synthetic-kis-calendar/{batch_id}/calendar.json"))
        .bind(&content_sha256)
        .bind(retrieved_at)
        .bind(batch_id)
        .execute(&database.migration_owner)
        .await?;
        sqlx::query(
            "INSERT INTO public.trading_calendar_versions
                (exchange, session_date, session_type, timezone, source, source_version,
                 source_batch_id, content_sha256, retrieved_at)
             VALUES ('KRX', $1, 'TRADING', 'Asia/Seoul', $2, $3, $4, $5, $6)",
        )
        .bind(calendar.session_date)
        .bind(calendar.calendar_source())
        .bind(calendar.calendar_source_version())
        .bind(batch_id)
        .bind(&content_sha256)
        .bind(retrieved_at)
        .execute(&database.migration_owner)
        .await?;
        sqlx::query(
            "INSERT INTO public.trading_calendars
                (exchange, session_date, session_type, timezone, source, source_version,
                 source_batch_id, content_sha256, retrieved_at)
             VALUES ('KRX', $1, 'TRADING', 'Asia/Seoul', $2, $3, $4, $5, $6)",
        )
        .bind(calendar.session_date)
        .bind(calendar.calendar_source())
        .bind(calendar.calendar_source_version())
        .bind(batch_id)
        .bind(&content_sha256)
        .bind(retrieved_at)
        .execute(&database.migration_owner)
        .await?;
        Ok(())
    }

    async fn quote_version(
        worker: &PgPool,
        owner_id: Uuid,
        membership_id: Uuid,
    ) -> Result<Option<i64>, sqlx::Error> {
        let mut tx = worker.begin().await?;
        sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
            .bind(owner_id.to_string())
            .execute(&mut *tx)
            .await?;
        let version = sqlx::query_scalar(
            "SELECT quote_version FROM public.owner_market_stream_cache
              WHERE owner_user_id = $1 AND membership_id = $2",
        )
        .bind(owner_id)
        .bind(membership_id)
        .fetch_optional(&mut *tx)
        .await?;
        tx.rollback().await?;
        Ok(version)
    }

    async fn cache_capture(
        database: &support::DisposableDatabase,
        owner_id: Uuid,
        membership_id: Uuid,
    ) -> Result<Option<QuoteCapture>, TestError> {
        let row: Option<(i64, Option<DateTime<Utc>>, Option<DateTime<Utc>>)> = sqlx::query_as(
            "SELECT quote_version, received_at, committed_at
               FROM public.owner_market_stream_cache
              WHERE owner_user_id = $1 AND membership_id = $2",
        )
        .bind(owner_id)
        .bind(membership_id)
        .fetch_optional(&database.migration_owner)
        .await?;
        row.map(|(quote_version, received_at, committed_at)| {
            Ok(QuoteCapture {
                quote_version: u64::try_from(quote_version)?,
                received_at: received_at.ok_or("stored cache row lacks received_at")?,
                committed_at: committed_at.ok_or("stored cache row lacks committed_at")?,
            })
        })
        .transpose()
    }

    async fn read_snapshot(
        database: &support::DisposableDatabase,
        fixture: &support::Fixture,
        lease: &StreamLease,
    ) -> Result<StreamSnapshot, MarketStreamStorageError> {
        OwnerMarketStreamRepository::new(database.app.clone())
            .read_stream_snapshot(
                fixture.owner_user_id,
                &fixture.owner_session_hash,
                lease.lease_id,
            )
            .await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn pre_ack_thirty_identity_snapshot_keeps_never_quoted_rows_non_live() -> TestResult {
        run_database_case(pre_ack_thirty_identity_snapshot_case).await
    }

    fn pre_ack_thirty_identity_snapshot_case(
        database: support::DisposableDatabase,
        fixture: support::Fixture,
    ) -> CaseFuture {
        Box::pin(async move {
            if fixture.identities.len() != 30 {
                return Err("fresh test fixture must contain exactly 30 identities".into());
            }
            let lease = create_lease(&database, &fixture, &fixture.identities).await?;
            let worker = OwnerMarketStreamRepository::new(database.worker.clone());
            let producer = worker
                .claim_stream_producer(
                    fixture.credential_slot_id,
                    Uuid::new_v4(),
                    fixture.grant_revision,
                )
                .await?;
            let (epoch, _) =
                start_pre_ack_epoch(&database, &fixture, &producer, &fixture.identities).await?;
            let result = worker
                .record_runtime_status(&producer, Some(epoch), RuntimeTransition::Connecting)
                .await?;
            assert!(result.changed());
            let snapshot = read_snapshot(&database, &fixture, &lease).await?;
            assert!(snapshot.rows.is_empty());
            assert_eq!(snapshot.delivery_rows.len(), 30);
            assert!(snapshot.delivery_rows.iter().all(|row| {
                row.cache.is_none()
                    && row.row_generation == lease.lease_id
                    && row.state_version == 0
                    && !row.live
            }));
            Ok(())
        })
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn resolved_snapshot_positive_actual_role_requires_authentic_ack_and_day_evidence()
    -> TestResult {
        run_database_case(resolved_snapshot_positive_case).await
    }

    fn resolved_snapshot_positive_case(
        database: support::DisposableDatabase,
        fixture: support::Fixture,
    ) -> CaseFuture {
        Box::pin(async move {
            let (lease, _producer, _day, window, _epoch) =
                seeded_resolved_snapshot(&database, &fixture).await?;
            let snapshot = read_snapshot_with_window(&database, &fixture, &lease, window).await?;
            assert_eq!(snapshot.delivery_rows.len(), 1);
            assert!(snapshot.delivery_rows[0].live);
            assert!(snapshot_quote_capture(&snapshot)?.quote_version > 0);
            Ok(())
        })
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn resolved_snapshot_stale_heartbeat_overrides_live_without_quote_mutation() -> TestResult
    {
        run_database_case(stale_heartbeat_snapshot_case).await
    }

    fn stale_heartbeat_snapshot_case(
        database: support::DisposableDatabase,
        fixture: support::Fixture,
    ) -> CaseFuture {
        Box::pin(snapshot_override_case(
            database,
            fixture,
            SnapshotOverride::StaleHeartbeat,
        ))
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn resolved_snapshot_short_producer_lease_overrides_live_without_quote_mutation()
    -> TestResult {
        run_database_case(short_producer_lease_snapshot_case).await
    }

    fn short_producer_lease_snapshot_case(
        database: support::DisposableDatabase,
        fixture: support::Fixture,
    ) -> CaseFuture {
        Box::pin(snapshot_override_case(
            database,
            fixture,
            SnapshotOverride::ShortProducerLease,
        ))
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn resolved_snapshot_epoch_mismatch_overrides_live_without_quote_mutation() -> TestResult
    {
        run_database_case(epoch_mismatch_snapshot_case).await
    }

    fn epoch_mismatch_snapshot_case(
        database: support::DisposableDatabase,
        fixture: support::Fixture,
    ) -> CaseFuture {
        Box::pin(snapshot_override_case(
            database,
            fixture,
            SnapshotOverride::EpochMismatch,
        ))
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn resolved_snapshot_missing_ack_overrides_live_without_quote_mutation() -> TestResult {
        run_database_case(missing_ack_snapshot_case).await
    }

    fn missing_ack_snapshot_case(
        database: support::DisposableDatabase,
        fixture: support::Fixture,
    ) -> CaseFuture {
        Box::pin(snapshot_override_case(
            database,
            fixture,
            SnapshotOverride::MissingAck,
        ))
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn resolved_snapshot_closed_window_overrides_live_without_quote_mutation() -> TestResult {
        run_database_case(closed_window_snapshot_case).await
    }

    fn closed_window_snapshot_case(
        database: support::DisposableDatabase,
        fixture: support::Fixture,
    ) -> CaseFuture {
        Box::pin(snapshot_override_case(
            database,
            fixture,
            SnapshotOverride::ClosedWindow,
        ))
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn resolved_snapshot_lineage_mismatch_overrides_live_without_quote_mutation() -> TestResult
    {
        run_database_case(lineage_mismatch_snapshot_case).await
    }

    fn lineage_mismatch_snapshot_case(
        database: support::DisposableDatabase,
        fixture: support::Fixture,
    ) -> CaseFuture {
        Box::pin(snapshot_override_case(
            database,
            fixture,
            SnapshotOverride::LineageMismatch,
        ))
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn reason_gap_and_retirement_preserve_quote_version_and_capture() -> TestResult {
        run_database_case(reason_gap_and_retirement_case).await
    }

    fn reason_gap_and_retirement_case(
        database: support::DisposableDatabase,
        fixture: support::Fixture,
    ) -> CaseFuture {
        Box::pin(async move {
            let identity = fixture.identities[0].clone();
            let _lease = create_lease(&database, &fixture, std::slice::from_ref(&identity)).await?;
            let worker = OwnerMarketStreamRepository::new(database.worker.clone());
            let producer = worker
                .claim_stream_producer(
                    fixture.credential_slot_id,
                    Uuid::new_v4(),
                    fixture.grant_revision,
                )
                .await?;
            let (epoch, _) = publish_one_quote(&database, &fixture, &producer, &identity).await?;
            let before = quote_version(
                &database.worker,
                fixture.owner_user_id,
                identity.membership_id,
            )
            .await?
            .ok_or("quote publication did not create cache row")?;
            let capture_before =
                cache_capture(&database, fixture.owner_user_id, identity.membership_id)
                    .await?
                    .ok_or("quote publication did not create capture metadata")?;
            let first = worker
                .record_runtime_status(&producer, Some(epoch), RuntimeTransition::ConnectionLost)
                .await?;
            assert!(first.changed());
            assert_eq!(first.gap_generation(), producer.gap_generation + 1);
            assert_eq!(
                cache_capture(&database, fixture.owner_user_id, identity.membership_id).await?,
                Some(capture_before.clone())
            );
            let repeated = worker
                .record_runtime_status(&producer, Some(epoch), RuntimeTransition::ConnectionLost)
                .await?;
            assert!(!repeated.changed());
            assert_eq!(repeated.gap_generation(), first.gap_generation());
            assert_eq!(
                cache_capture(&database, fixture.owner_user_id, identity.membership_id).await?,
                Some(capture_before.clone())
            );
            assert_eq!(
                quote_version(
                    &database.worker,
                    fixture.owner_user_id,
                    identity.membership_id
                )
                .await?,
                Some(before)
            );
            let retired = worker
                .retire_stream_producer(&producer, Some(epoch), StreamStatusCode::ConnectionLost)
                .await?;
            assert!(retired.changed());
            assert_eq!(retired.current_epoch(), None);
            assert_eq!(retired.gap_generation(), first.gap_generation());
            assert_eq!(
                cache_capture(&database, fixture.owner_user_id, identity.membership_id).await?,
                Some(capture_before)
            );
            assert_eq!(
                quote_version(
                    &database.worker,
                    fixture.owner_user_id,
                    identity.membership_id
                )
                .await?,
                Some(before)
            );
            let (epoch_after, _) =
                publish_one_quote(&database, &fixture, &producer, &identity).await?;
            assert_ne!(epoch_after, epoch);
            let producer_state: (
                Option<Uuid>,
                Option<chrono::NaiveDate>,
                Option<Uuid>,
                Option<DateTime<Utc>>,
                bool,
                i64,
            ) = sqlx::query_as(
                "SELECT current_epoch, session_date, session_proof_id, gap_since,
                            session_has_gap, gap_generation
                       FROM public.owner_market_stream_producers WHERE credential_slot_id = $1",
            )
            .bind(fixture.credential_slot_id)
            .fetch_one(&database.worker)
            .await?;
            assert_eq!(producer_state.0, Some(epoch_after));
            assert_eq!(producer_state.1, Some(fixture.session_date));
            assert_eq!(producer_state.2, Some(fixture.session.session_proof_id));
            assert!(producer_state.3.is_none());
            assert!(producer_state.4);
            assert_eq!(producer_state.5, i64::try_from(first.gap_generation())?);
            sqlx::query(
                "UPDATE public.owner_market_stream_producers
                    SET session_date = $2
                  WHERE credential_slot_id = $1",
            )
            .bind(fixture.credential_slot_id)
            .bind(fixture.session_date - ChronoDuration::days(1))
            .execute(&database.migration_owner)
            .await?;
            worker
                .start_stream_epoch(&producer, fixture.session.clone(), Uuid::new_v4())
                .await?;
            let next_day: (Option<chrono::NaiveDate>, Option<DateTime<Utc>>, bool, i64) =
                sqlx::query_as(
                    "SELECT session_date, gap_since, session_has_gap, gap_generation
                       FROM public.owner_market_stream_producers WHERE credential_slot_id = $1",
                )
                .bind(fixture.credential_slot_id)
                .fetch_one(&database.worker)
                .await?;
            assert_eq!(next_day.0, Some(fixture.session_date));
            assert!(next_day.1.is_none());
            assert!(!next_day.2);
            assert_eq!(next_day.3, i64::try_from(first.gap_generation())?);
            Ok(())
        })
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn retirement_clears_empty_demand_but_preserves_rejoined_consumers() -> TestResult {
        run_database_case(retirement_demand_case).await
    }

    fn retirement_demand_case(
        database: support::DisposableDatabase,
        fixture: support::Fixture,
    ) -> CaseFuture {
        Box::pin(async move {
            let identity = fixture.identities[0].clone();
            let lease = create_lease(&database, &fixture, std::slice::from_ref(&identity)).await?;
            let worker = OwnerMarketStreamRepository::new(database.worker.clone());
            let producer = worker
                .claim_stream_producer(
                    fixture.credential_slot_id,
                    Uuid::new_v4(),
                    fixture.grant_revision,
                )
                .await?;
            let (epoch, _) = publish_one_quote(&database, &fixture, &producer, &identity).await?;
            let capture_before =
                cache_capture(&database, fixture.owner_user_id, identity.membership_id)
                    .await?
                    .ok_or("quote publication did not create capture metadata")?;
            let version_before = quote_version(
                &database.worker,
                fixture.owner_user_id,
                identity.membership_id,
            )
            .await?;

            // A consumer can rejoin after the caller observed NoDemand. The
            // locked current demand, not the stop reason, governs the count.
            worker
                .retire_stream_producer(&producer, Some(epoch), StreamStatusCode::NoActiveDemand)
                .await?;
            let count: i32 = sqlx::query_scalar(
                "SELECT desired_reference_count FROM public.owner_market_stream_subscriptions
                  WHERE credential_slot_id = $1 AND symbol = $2",
            )
            .bind(fixture.credential_slot_id)
            .bind(identity.symbol())
            .fetch_one(&database.worker)
            .await?;
            assert_eq!(count, 1, "retirement discarded a current consumer");

            let (next_epoch, _) = start_pre_ack_epoch(
                &database,
                &fixture,
                &producer,
                std::slice::from_ref(&identity),
            )
            .await?;
            OwnerMarketStreamRepository::new(database.app.clone())
                .release_stream_lease(
                    fixture.owner_user_id,
                    &fixture.owner_session_hash,
                    lease.lease_id,
                    lease.renewal_sequence,
                )
                .await?;
            assert!(
                worker
                    .read_stream_demand(fixture.credential_slot_id)
                    .await?
                    .items
                    .is_empty()
            );
            assert_eq!(
                worker
                    .retire_stream_producer(
                        &producer,
                        Some(epoch),
                        StreamStatusCode::NoActiveDemand
                    )
                    .await,
                Err(MarketStreamStorageError::ProducerLost)
            );
            let still_owned: (Option<Uuid>, i32) = sqlx::query_as(
                "SELECT epoch, desired_reference_count FROM public.owner_market_stream_subscriptions
                  WHERE credential_slot_id = $1 AND symbol = $2",
            )
            .bind(fixture.credential_slot_id)
            .bind(identity.symbol())
            .fetch_one(&database.worker)
            .await?;
            assert_eq!(still_owned, (Some(next_epoch), 1));

            let retired = worker
                .retire_stream_producer(
                    &producer,
                    Some(next_epoch),
                    StreamStatusCode::NoActiveDemand,
                )
                .await?;
            assert!(retired.changed());
            assert_eq!(retired.current_epoch(), None);
            let after: (Option<Uuid>, String, i32, Option<String>, Option<i64>) = sqlx::query_as(
                "SELECT epoch, state, desired_reference_count, pending_operation, pending_ordinal
                   FROM public.owner_market_stream_subscriptions
                  WHERE credential_slot_id = $1 AND symbol = $2",
            )
            .bind(fixture.credential_slot_id)
            .bind(identity.symbol())
            .fetch_one(&database.worker)
            .await?;
            assert_eq!(after, (None, "ABSENT".to_owned(), 0, None, None));
            let repeated = worker
                .retire_stream_producer(&producer, None, StreamStatusCode::NoActiveDemand)
                .await?;
            assert!(!repeated.changed());
            assert_eq!(
                cache_capture(&database, fixture.owner_user_id, identity.membership_id).await?,
                Some(capture_before)
            );
            assert_eq!(
                quote_version(
                    &database.worker,
                    fixture.owner_user_id,
                    identity.membership_id
                )
                .await?,
                version_before
            );
            Ok(())
        })
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn exact_epoch_and_fence_expectations_reject_stale_status_and_retire() -> TestResult {
        run_database_case(exact_epoch_fence_case).await
    }

    fn exact_epoch_fence_case(
        database: support::DisposableDatabase,
        fixture: support::Fixture,
    ) -> CaseFuture {
        Box::pin(async move {
            let identity = fixture.identities[0].clone();
            let _lease = create_lease(&database, &fixture, std::slice::from_ref(&identity)).await?;
            let worker = OwnerMarketStreamRepository::new(database.worker.clone());
            let old = worker
                .claim_stream_producer(
                    fixture.credential_slot_id,
                    Uuid::new_v4(),
                    fixture.grant_revision,
                )
                .await?;
            let (epoch, _) =
                start_pre_ack_epoch(&database, &fixture, &old, std::slice::from_ref(&identity))
                    .await?;
            support::set_producer_expired(&database, fixture.credential_slot_id).await?;
            let current = worker
                .claim_stream_producer(
                    fixture.credential_slot_id,
                    Uuid::new_v4(),
                    fixture.grant_revision,
                )
                .await?;
            assert!(current.fencing_token > old.fencing_token);
            assert_eq!(
                worker
                    .record_runtime_status(&old, Some(epoch), RuntimeTransition::ConnectionLost)
                    .await,
                Err(MarketStreamStorageError::ProducerLost)
            );
            assert_eq!(
                worker
                    .record_runtime_status(
                        &current,
                        Some(Uuid::new_v4()),
                        RuntimeTransition::Connecting
                    )
                    .await,
                Err(MarketStreamStorageError::ProducerLost)
            );
            worker
                .record_runtime_status(&current, None, RuntimeTransition::Connecting)
                .await?;
            let next_epoch = Uuid::new_v4();
            worker
                .start_stream_epoch(&current, fixture.session.clone(), next_epoch)
                .await?;
            assert_eq!(
                worker
                    .record_runtime_status(&current, None, RuntimeTransition::Connecting)
                    .await,
                Err(MarketStreamStorageError::ProducerLost)
            );
            assert_eq!(
                worker
                    .retire_stream_producer(&current, None, StreamStatusCode::AccessRevoked)
                    .await,
                Err(MarketStreamStorageError::ProducerLost)
            );
            let retired = worker
                .retire_stream_producer(
                    &current,
                    Some(next_epoch),
                    StreamStatusCode::ConnectionLost,
                )
                .await?;
            assert_eq!(retired.current_epoch(), None);
            let stored: (
                Option<Uuid>,
                Option<Uuid>,
                Option<String>,
                Option<Uuid>,
                Option<String>,
                Option<String>,
                i64,
            ) = sqlx::query_as(
                "SELECT current_epoch, session_proof_id, session_proof_sha256,
                        calendar_source_batch_id, calendar_content_sha256,
                        window_contract_sha256, gap_generation
                   FROM public.owner_market_stream_producers WHERE credential_slot_id = $1",
            )
            .bind(fixture.credential_slot_id)
            .fetch_one(&database.worker)
            .await?;
            assert_eq!(stored.0, None);
            assert_eq!(stored.1, Some(fixture.session.session_proof_id));
            assert_eq!(
                stored.2.as_deref(),
                Some(fixture.session.session_proof_sha256.as_str())
            );
            assert_eq!(stored.3, Some(fixture.session.calendar_source_batch_id));
            assert_eq!(
                stored.4.as_deref(),
                Some(fixture.session.calendar_content_sha256.as_str())
            );
            assert_eq!(
                stored.5.as_deref(),
                Some(fixture.session.window_contract_sha256.as_str())
            );
            assert_eq!(stored.6, i64::try_from(retired.gap_generation())?);
            Ok(())
        })
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn resolved_day_requires_persisted_lineage_and_rejects_close_boundary() -> TestResult {
        run_database_case(resolved_day_rejection_case).await
    }

    fn resolved_day_rejection_case(
        database: support::DisposableDatabase,
        fixture: support::Fixture,
    ) -> CaseFuture {
        Box::pin(async move {
            let identity = fixture.identities[0].clone();
            let _lease = create_lease(&database, &fixture, std::slice::from_ref(&identity)).await?;
            let worker = OwnerMarketStreamRepository::new(database.worker.clone());
            let producer = worker
                .claim_stream_producer(
                    fixture.credential_slot_id,
                    Uuid::new_v4(),
                    fixture.grant_revision,
                )
                .await?;
            let calendar = IntradaySessionProof::new(
                fixture.session_date,
                fixture.session.calendar_source_batch_id,
                fixture.session.calendar_content_sha256.clone(),
                fixture.session.window_contract_sha256.clone(),
            )?;
            let now = database_clock_now(&database).await?;
            seed_calendar_lineage(&database, &calendar, now).await?;
            let open = now - ChronoDuration::seconds(30);
            let close = now + ChronoDuration::seconds(90);
            let valid_day = fixture_day(calendar.clone(), open, close, Uuid::new_v4())?;
            let valid_epoch = Uuid::new_v4();
            let committed = worker
                .start_stream_epoch_resolved(&producer, &valid_day, valid_epoch)
                .await?;
            assert_eq!(committed.epoch, valid_epoch);

            let mismatched_calendar = IntradaySessionProof::new(
                calendar.session_date,
                calendar.calendar_source_batch_id,
                "9999999999999999999999999999999999999999999999999999999999999999".to_owned(),
                calendar.window_contract_sha256.clone(),
            )?;
            let mismatched_lineage = fixture_day(mismatched_calendar, open, close, Uuid::new_v4())?;
            assert_eq!(
                worker
                    .start_stream_epoch_resolved(&producer, &mismatched_lineage, Uuid::new_v4())
                    .await,
                Err(MarketStreamStorageError::SessionInvalid)
            );

            let close_boundary = fixture_day(calendar, open, now, Uuid::new_v4())?;
            assert_eq!(
                worker
                    .start_stream_epoch_resolved(&producer, &close_boundary, Uuid::new_v4())
                    .await,
                Err(MarketStreamStorageError::SessionInvalid)
            );
            Ok(())
        })
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn resolved_epoch_rechecks_close_after_locked_work_and_rolls_back() -> TestResult {
        run_database_case(resolved_epoch_close_boundary_case).await
    }

    fn resolved_epoch_close_boundary_case(
        database: support::DisposableDatabase,
        fixture: support::Fixture,
    ) -> CaseFuture {
        Box::pin(async move {
            let identity = fixture.identities[0].clone();
            let _lease = create_lease(&database, &fixture, std::slice::from_ref(&identity)).await?;
            let worker = OwnerMarketStreamRepository::new(database.worker.clone());
            let producer = worker
                .claim_stream_producer(
                    fixture.credential_slot_id,
                    Uuid::new_v4(),
                    fixture.grant_revision,
                )
                .await?;
            let (prior_epoch, _) =
                publish_one_quote(&database, &fixture, &producer, &identity).await?;
            let prior_capture =
                cache_capture(&database, fixture.owner_user_id, identity.membership_id)
                    .await?
                    .ok_or("baseline quote capture is missing")?;

            let calendar = IntradaySessionProof::new(
                fixture.session_date,
                fixture.session.calendar_source_batch_id,
                fixture.session.calendar_content_sha256.clone(),
                fixture.session.window_contract_sha256.clone(),
            )?;
            let before = database_clock_now(&database).await?;
            seed_calendar_lineage(&database, &calendar, before).await?;
            let close_at = before + ChronoDuration::seconds(8);
            let day = fixture_day(
                calendar,
                before - ChronoDuration::seconds(60),
                close_at,
                Uuid::new_v4(),
            )?;
            let candidate_epoch = Uuid::new_v4();
            let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
            let (release_tx, release_rx) = tokio::sync::oneshot::channel();
            register_resolved_commit_test_gate(candidate_epoch, entered_tx, release_rx)?;
            let task_database = database.clone();
            let task_producer = producer.clone();
            let task_day = day.clone();
            let task = tokio::spawn(async move {
                OwnerMarketStreamRepository::new(task_database.worker.clone())
                    .start_stream_epoch_resolved(&task_producer, &task_day, candidate_epoch)
                    .await
            });
            let after_close = close_at + ChronoDuration::milliseconds(100);
            let pre_release = match wait_for_resolved_commit_gate(entered_rx).await {
                Ok(()) => {
                    observe_epoch_close_before_release(
                        &database,
                        fixture.credential_slot_id,
                        after_close,
                    )
                    .await
                }
                Err(error) => Err(error),
            };
            let (observation, primary) = match pre_release {
                Ok(observation) => (Some(observation), None),
                Err(error) => (None, Some(error)),
            };
            let abort_child = observation.is_none();
            let finished = finish_resolved_commit_task(
                candidate_epoch,
                task,
                if abort_child { None } else { Some(release_tx) },
                abort_child,
            )
            .await;
            let ResolvedTaskFinish {
                output,
                release_sent,
                aborted,
                error,
            } = finished;
            let mut cleanup_errors = Vec::new();
            if let Some(error) = error {
                cleanup_errors.push(error);
            }
            if release_sent == Some(false) {
                cleanup_errors.push(test_error(
                    "resolved epoch commit gate release receiver was unavailable",
                ));
            }
            if aborted {
                if let Some(output) = output {
                    cleanup_errors.push(aborted_operation_output_error(output, "resolved epoch"));
                }
                return finish_close_case(primary, cleanup_errors);
            }
            finish_close_case(primary, cleanup_errors)?;

            let (closed_at, producer_expiry) = match observation {
                Some(observation) => observation,
                None => {
                    return Err(test_error(
                        "resolved epoch close observation was missing after cleanup",
                    ));
                }
            };
            assert!(closed_at >= close_at);
            assert!(producer_expiry > closed_at + ChronoDuration::seconds(5));
            let result = match output {
                Some(result) => result,
                None => return Err(test_error("resolved epoch child returned no joined result")),
            };
            assert_eq!(result, Err(MarketStreamStorageError::SessionInvalid));
            assert_eq!(
                stored_current_epoch(&database, fixture.credential_slot_id).await?,
                Some(prior_epoch)
            );
            assert_eq!(
                cache_capture(&database, fixture.owner_user_id, identity.membership_id).await?,
                Some(prior_capture)
            );
            Ok(())
        })
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn resolved_publication_rechecks_close_after_locked_work_and_rolls_back() -> TestResult {
        run_database_case(resolved_publication_close_boundary_case).await
    }

    fn resolved_publication_close_boundary_case(
        database: support::DisposableDatabase,
        fixture: support::Fixture,
    ) -> CaseFuture {
        Box::pin(async move {
            let identity = fixture.identities[0].clone();
            let _lease = create_lease(&database, &fixture, std::slice::from_ref(&identity)).await?;
            let worker = OwnerMarketStreamRepository::new(database.worker.clone());
            let producer = worker
                .claim_stream_producer(
                    fixture.credential_slot_id,
                    Uuid::new_v4(),
                    fixture.grant_revision,
                )
                .await?;
            let _ = publish_one_quote(&database, &fixture, &producer, &identity).await?;
            let prior_capture =
                cache_capture(&database, fixture.owner_user_id, identity.membership_id)
                    .await?
                    .ok_or("baseline quote capture is missing")?;

            let calendar = IntradaySessionProof::new(
                fixture.session_date,
                fixture.session.calendar_source_batch_id,
                fixture.session.calendar_content_sha256.clone(),
                fixture.session.window_contract_sha256.clone(),
            )?;
            let before = database_clock_now(&database).await?;
            seed_calendar_lineage(&database, &calendar, before).await?;
            let receipt_time = before + ChronoDuration::seconds(6);
            let close_at = receipt_time + ChronoDuration::milliseconds(1_800);
            let day = fixture_day(
                calendar,
                before - ChronoDuration::seconds(60),
                close_at,
                Uuid::new_v4(),
            )?;
            let (mut transport, context) = resolved_context_with_transport(
                &database,
                &fixture,
                &producer,
                &identity,
                &day,
                Duration::from_secs(20),
            )
            .await?;

            let capture_at = receipt_time - ChronoDuration::milliseconds(300);
            if let Err(error) = wait_for_database_time(&database, capture_at).await {
                return close_loopback_with_primary(transport, error).await;
            }
            transport.advance_clock_to(receipt_time.timestamp_millis());
            let receipt = match transport.next_receipt().await {
                Ok(receipt) => receipt,
                Err(error) => {
                    return close_loopback_with_primary(transport, error).await;
                }
            };
            let received_at = match DateTime::<Utc>::from_timestamp_millis(receipt.received_at_ms())
            {
                Some(received_at) => received_at,
                None => {
                    return close_loopback_with_primary(
                        transport,
                        test_error("loopback receipt timestamp is invalid"),
                    )
                    .await;
                }
            };
            let received_monotonic = receipt.received_monotonic();
            if let Err(error) = wait_for_database_time(&database, receipt_time).await {
                return close_loopback_with_primary(transport, error).await;
            }

            let observation = StreamPublicationObservation::from_receipt(receipt);
            let task_database = database.clone();
            let task_context = context.clone();
            let task_day = day.clone();
            let gate_key = context.epoch.epoch;
            let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
            let (release_tx, release_rx) = tokio::sync::oneshot::channel();
            if let Err(error) = register_resolved_commit_test_gate(gate_key, entered_tx, release_rx)
            {
                return close_loopback_with_primary(transport, Box::new(error)).await;
            }
            let task = tokio::spawn(async move {
                OwnerMarketStreamRepository::new(task_database.worker.clone())
                    .publish_stream_latest_resolved(&task_context, &task_day, &[observation])
                    .await
            });
            let after_close = close_at + ChronoDuration::milliseconds(100);
            let pre_release = match wait_for_resolved_commit_gate(entered_rx).await {
                Ok(()) => {
                    observe_publication_close_before_release(
                        &database,
                        fixture.credential_slot_id,
                        after_close,
                        received_at,
                        received_monotonic,
                    )
                    .await
                }
                Err(error) => Err(error),
            };
            let (observation, primary) = match pre_release {
                Ok(observation) => (Some(observation), None),
                Err(error) => (None, Some(error)),
            };
            let abort_child = observation.is_none();
            let finished = finish_resolved_commit_task(
                gate_key,
                task,
                if abort_child { None } else { Some(release_tx) },
                abort_child,
            )
            .await;
            let transport_close = transport.close().await;
            let ResolvedTaskFinish {
                output,
                release_sent,
                aborted,
                error,
            } = finished;
            let mut cleanup_errors = Vec::new();
            if let Some(error) = error {
                cleanup_errors.push(error);
            }
            if release_sent == Some(false) {
                cleanup_errors.push(test_error(
                    "resolved publication commit gate release receiver was unavailable",
                ));
            }
            if let Err(error) = transport_close {
                cleanup_errors.push(error);
            }
            if aborted {
                if let Some(output) = output {
                    cleanup_errors.push(aborted_operation_output_error(
                        output,
                        "resolved publication",
                    ));
                }
                return finish_close_case(primary, cleanup_errors);
            }
            finish_close_case(primary, cleanup_errors)?;

            let observation = match observation {
                Some(observation) => observation,
                None => {
                    return Err(test_error(
                        "resolved publication close observation was missing after cleanup",
                    ));
                }
            };
            assert!(observation.database_now >= close_at);
            assert!(observation.receipt_wall_age >= chrono::Duration::zero());
            assert!(observation.receipt_wall_age <= ChronoDuration::seconds(3));
            assert!(observation.receipt_monotonic_age <= Duration::from_secs(3));
            assert!(
                observation.producer_expiry > observation.database_now + ChronoDuration::seconds(5)
            );
            let result = match output {
                Some(result) => result,
                None => {
                    return Err(test_error(
                        "resolved publication child returned no joined result",
                    ));
                }
            };
            assert_eq!(result, Err(MarketStreamStorageError::SessionInvalid));
            assert_eq!(
                stored_current_epoch(&database, fixture.credential_slot_id).await?,
                Some(context.epoch.epoch)
            );
            assert_eq!(
                cache_capture(&database, fixture.owner_user_id, identity.membership_id).await?,
                Some(prior_capture)
            );
            Ok(())
        })
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn actor_helper_denies_bad_context_rights_and_stale_generation_without_table_grants()
    -> TestResult {
        run_database_case(actor_helper_denial_case).await
    }

    fn actor_helper_denial_case(
        database: support::DisposableDatabase,
        fixture: support::Fixture,
    ) -> CaseFuture {
        Box::pin(async move {
            let identity = fixture.identities[0].clone();
            let lease = create_lease(&database, &fixture, std::slice::from_ref(&identity)).await?;
            let grants: (bool, bool, bool) = sqlx::query_as(
                "SELECT has_table_privilege('app', 'public.owner_market_stream_producers', 'SELECT'),
                        has_table_privilege('app', 'public.owner_market_stream_subscriptions', 'SELECT'),
                        has_table_privilege('app', 'public.owner_market_stream_grants', 'SELECT')",
            )
            .fetch_one(&database.migration_owner)
            .await?;
            assert_eq!(grants, (false, false, false));

            let app = &database.app;
            let mut tx = app.begin().await?;
            sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
                .bind(fixture.owner_user_id.to_string())
                .execute(&mut *tx)
                .await?;
            sqlx::query("SELECT pg_catalog.set_config('app.market_stream_session_hash', $1, true)")
                .bind(&fixture.owner_session_hash)
                .execute(&mut *tx)
                .await?;
            let valid: i64 = sqlx::query_scalar(
                "SELECT count(*)::bigint FROM public.owner_market_stream_delivery_state($1, $2, $3)",
            )
            .bind(fixture.owner_user_id)
            .bind(&fixture.owner_session_hash)
            .bind(lease.lease_id)
            .fetch_one(&mut *tx)
            .await?;
            assert_eq!(valid, 1);
            let wrong_owner: i64 = sqlx::query_scalar(
                "SELECT count(*)::bigint FROM public.owner_market_stream_delivery_state($1, $2, $3)",
            )
            .bind(fixture.other_owner_user_id)
            .bind(&fixture.owner_session_hash)
            .bind(lease.lease_id)
            .fetch_one(&mut *tx)
            .await?;
            assert_eq!(wrong_owner, 0);
            sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', '', true)")
                .execute(&mut *tx)
                .await?;
            let missing_guc: i64 = sqlx::query_scalar(
                "SELECT count(*)::bigint FROM public.owner_market_stream_delivery_state($1, $2, $3)",
            )
            .bind(fixture.owner_user_id)
            .bind(&fixture.owner_session_hash)
            .bind(lease.lease_id)
            .fetch_one(&mut *tx)
            .await?;
            assert_eq!(missing_guc, 0);
            tx.rollback().await?;

            support::revoke_session(
                &database,
                fixture.owner_user_id,
                &fixture.owner_session_hash,
            )
            .await?;
            let denied_session = OwnerMarketStreamRepository::new(database.app.clone())
                .read_stream_snapshot(
                    fixture.owner_user_id,
                    &fixture.owner_session_hash,
                    lease.lease_id,
                )
                .await;
            assert_eq!(
                denied_session,
                Err(MarketStreamStorageError::SessionInvalid)
            );
            support::restore_session(
                &database,
                fixture.owner_user_id,
                &fixture.owner_session_hash,
            )
            .await?;
            support::revoke_entitlement(&database, fixture.grant_id).await?;
            let mut rights_tx = app.begin().await?;
            sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
                .bind(fixture.owner_user_id.to_string())
                .execute(&mut *rights_tx)
                .await?;
            sqlx::query("SELECT pg_catalog.set_config('app.market_stream_session_hash', $1, true)")
                .bind(&fixture.owner_session_hash)
                .execute(&mut *rights_tx)
                .await?;
            let denied_rights: i64 = sqlx::query_scalar(
                "SELECT count(*)::bigint FROM public.owner_market_stream_delivery_state($1, $2, $3)",
            )
            .bind(fixture.owner_user_id)
            .bind(&fixture.owner_session_hash)
            .bind(lease.lease_id)
            .fetch_one(&mut *rights_tx)
            .await?;
            assert_eq!(denied_rights, 0);
            rights_tx.rollback().await?;
            support::restore_entitlement(&database, fixture.grant_id).await?;
            support::supersede_generation(&database, &identity).await?;
            let stale_generation = OwnerMarketStreamRepository::new(database.app.clone())
                .read_stream_snapshot(
                    fixture.owner_user_id,
                    &fixture.owner_session_hash,
                    lease.lease_id,
                )
                .await;
            assert_eq!(
                stale_generation,
                Err(MarketStreamStorageError::MembershipNotReady)
            );
            Ok(())
        })
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn migration_0056_up_and_down_preserve_0055_acl_and_dependencies() -> TestResult {
        run_database_case(migration_0056_up_down_case).await
    }

    fn migration_0056_up_down_case(
        database: support::DisposableDatabase,
        _fixture: support::Fixture,
    ) -> CaseFuture {
        Box::pin(async move {
            let migration: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM public._sqlx_migrations WHERE version = 56)",
            )
            .fetch_one(&database.migration_owner)
            .await?;
            assert!(migration);
            let helper: (bool, bool, bool, bool) = sqlx::query_as(
                "SELECT EXISTS (
                            SELECT 1 FROM pg_catalog.pg_proc AS p
                            CROSS JOIN LATERAL pg_catalog.aclexplode(p.proacl) AS acl
                             WHERE p.oid = pg_catalog.to_regprocedure('public.owner_market_stream_delivery_state(uuid,text,uuid)')
                               AND acl.grantee = 0 AND acl.privilege_type = 'EXECUTE'
                        ),
                        has_function_privilege('app', 'public.owner_market_stream_delivery_state(uuid,text,uuid)', 'EXECUTE'),
                        has_function_privilege('worker', 'public.owner_market_stream_delivery_state(uuid,text,uuid)', 'EXECUTE'),
                        pg_catalog.pg_get_userbyid(p.proowner) = 'migration_owner'
                   FROM pg_catalog.pg_proc AS p
                  WHERE p.oid = pg_catalog.to_regprocedure('public.owner_market_stream_delivery_state(uuid,text,uuid)')",
            )
            .fetch_one(&database.migration_owner)
            .await?;
            assert_eq!(helper, (false, true, false, true));
            let worker_new_column_insert: bool = sqlx::query_scalar(
                "SELECT has_column_privilege('worker', 'public.owner_market_stream_producers', 'status_code', 'INSERT')",
            )
            .fetch_one(&database.migration_owner)
            .await?;
            assert!(!worker_new_column_insert);

            sqlx::raw_sql(include_str!(
                "../../../../migrations/0056_owner_market_stream_runtime.down.sql"
            ))
            .execute(&database.migration_owner)
            .await?;
            let after_down: (bool, bool, bool) = sqlx::query_as(
                "SELECT pg_catalog.to_regprocedure('public.owner_market_stream_delivery_state(uuid,text,uuid)') IS NULL,
                        pg_catalog.to_regprocedure('public.lock_owner_market_stream_rights(uuid,uuid,uuid,uuid,date)') IS NOT NULL,
                        has_table_privilege('worker', 'public.owner_market_stream_producers', 'SELECT')",
            )
            .fetch_one(&database.migration_owner)
            .await?;
            assert_eq!(after_down, (true, true, true));
            let status_columns: bool = sqlx::query_scalar(
                "SELECT EXISTS (
                    SELECT 1 FROM pg_catalog.pg_attribute
                     WHERE attrelid = 'public.owner_market_stream_producers'::regclass
                       AND attname IN ('status_code', 'status_at', 'gap_since', 'session_has_gap')
                       AND NOT attisdropped
                 )",
            )
            .fetch_one(&database.migration_owner)
            .await?;
            assert!(!status_columns);
            Ok(())
        })
    }

    #[derive(Clone, Copy)]
    enum SnapshotLeaseCollision {
        Renew,
        Release,
        HoldPastDeadline,
    }

    async fn snapshot_lease_collision(
        database: support::DisposableDatabase,
        fixture: support::Fixture,
        collision: SnapshotLeaseCollision,
    ) -> TestResult {
        // Real app-role/RLS writes touch only the task-owned demand lease. No
        // provider, producer, quote or authorization evidence is manufactured.
        let lease = create_lease(&database, &fixture, &fixture.identities).await?;
        let before = read_snapshot(&database, &fixture, &lease).await?;
        assert_eq!(before.delivery_rows.len(), 30);
        let mut blocker = database.app.begin().await?;
        sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
            .bind(fixture.owner_user_id.to_string())
            .execute(&mut *blocker)
            .await?;
        sqlx::query("SELECT pg_catalog.set_config('app.market_stream_session_hash', $1, true)")
            .bind(&fixture.owner_session_hash)
            .execute(&mut *blocker)
            .await?;
        let role: String = sqlx::query_scalar("SELECT current_user")
            .fetch_one(&mut *blocker)
            .await?;
        assert_eq!(role, "app");
        let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_catalog.pg_backend_pid()")
            .fetch_one(&mut *blocker)
            .await?;
        let statement = match collision {
            SnapshotLeaseCollision::Release => {
                "UPDATE public.owner_market_stream_leases
                    SET state = 'RELEASED', released_at = pg_catalog.clock_timestamp()
                  WHERE id = $1 AND owner_user_id = $2 AND session_hash = $3"
            }
            _ => {
                "UPDATE public.owner_market_stream_leases
                    SET lease_expires_at = lease_expires_at + INTERVAL '1 millisecond'
                  WHERE id = $1 AND owner_user_id = $2 AND session_hash = $3"
            }
        };
        let changed = sqlx::query(statement)
            .bind(lease.lease_id)
            .bind(fixture.owner_user_id)
            .bind(&fixture.owner_session_hash)
            .execute(&mut *blocker)
            .await?;
        assert_eq!(changed.rows_affected(), 1);
        let mut blocker = Some(blocker);
        let started = tokio::time::Instant::now();
        let deadline = started + Duration::from_secs(1);
        let result: Result<_, TestError> = async {
            // The same absolute caller deadline owns pool acquisition, actor
            // setup, the conflict, rollback and any fresh snapshot attempt.
            let mut reading = Box::pin(tokio::time::timeout_at(
                deadline,
                read_snapshot(&database, &fixture, &lease),
            ));
            let observe_deadline = started + Duration::from_millis(600);
            loop {
                let blocked = tokio::time::timeout_at(
                    observe_deadline,
                    sqlx::query_scalar::<_, i64>(
                        "SELECT count(*)::bigint FROM pg_catalog.pg_stat_activity
                          WHERE datname = pg_catalog.current_database()
                            AND usename = 'app'
                            AND $1 = ANY(pg_catalog.pg_blocking_pids(pid))",
                    )
                    .bind(blocker_pid)
                    .fetch_one(&database.migration_owner),
                );
                tokio::select! {
                    _ = &mut reading => return Err(test_error("snapshot completed before observed lease contention")),
                    count = blocked => {
                        let count = count.map_err(|_| test_error("lease contention was not observed within600ms"))??;
                        if count == 1 { break; }
                        if count != 0 { return Err(test_error("unexpected app waiter count")); }
                    }
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            if !matches!(collision, SnapshotLeaseCollision::HoldPastDeadline) {
                blocker.take().ok_or("lease blocker missing")?.commit().await?;
            }
            Ok((reading.await, started.elapsed()))
        }
        .await;
        // Also release the exact held transaction after any observation error
        // or timeout. The read future has been completed/dropped before here.
        if let Some(blocker) = blocker.take() {
            blocker.rollback().await?;
        }
        let (read, elapsed) = result?;
        match collision {
            SnapshotLeaseCollision::Renew => {
                let snapshot = read
                    .map_err(|_| test_error("snapshot renewal exceeded the original1s deadline"))?
                    .expect("concurrent renewal must not terminate the actor snapshot");
                assert!(elapsed < Duration::from_secs(1));
                assert_eq!(snapshot.lease_id, lease.lease_id);
                assert_eq!(snapshot.delivery_rows.len(), before.delivery_rows.len());
                assert_eq!(
                    snapshot.lease_expires_at,
                    before.lease_expires_at + ChronoDuration::milliseconds(1)
                );
                assert!(snapshot.rows.is_empty());
            }
            SnapshotLeaseCollision::Release => {
                assert!(elapsed < Duration::from_secs(1));
                assert_eq!(read?, Err(MarketStreamStorageError::LeaseReleased));
            }
            SnapshotLeaseCollision::HoldPastDeadline => {
                assert!(
                    read.is_err(),
                    "held lease read escaped its original deadline"
                );
                // Runtime deadline remains exactly1s. The extra250ms is only
                // measurement tolerance for scheduling the completed timeout.
                assert!(elapsed >= Duration::from_secs(1));
                assert!(elapsed < Duration::from_millis(1250));
                let next = tokio::time::timeout(
                    Duration::from_secs(1),
                    read_snapshot(&database, &fixture, &lease),
                )
                .await??;
                assert_eq!(next.lease_expires_at, before.lease_expires_at);
                assert_eq!(next.delivery_rows.len(), 30);
            }
        }
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn actor_snapshot_recovers_one_concurrent_lease_renewal() -> TestResult {
        run_database_case(|database, fixture| {
            Box::pin(snapshot_lease_collision(
                database,
                fixture,
                SnapshotLeaseCollision::Renew,
            ))
        })
        .await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn actor_snapshot_conflict_rechecks_concurrent_lease_release() -> TestResult {
        run_database_case(|database, fixture| {
            Box::pin(snapshot_lease_collision(
                database,
                fixture,
                SnapshotLeaseCollision::Release,
            ))
        })
        .await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn actor_snapshot_contention_keeps_original_deadline_and_rollback() -> TestResult {
        run_database_case(|database, fixture| {
            Box::pin(snapshot_lease_collision(
                database,
                fixture,
                SnapshotLeaseCollision::HoldPastDeadline,
            ))
        })
        .await
    }

    async fn runtime_fault_producer_count(
        database: &support::DisposableDatabase,
        fixture: &support::Fixture,
    ) -> Result<i64, TestError> {
        Ok(tokio::time::timeout(
            Duration::from_secs(2),
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*)::bigint FROM public.owner_market_stream_producers
                  WHERE credential_slot_id = $1",
            )
            .bind(fixture.credential_slot_id)
            .fetch_one(&database.migration_owner),
        )
        .await??)
    }

    async fn runtime_pool_fault(
        database: support::DisposableDatabase,
        fixture: support::Fixture,
        cancel: bool,
    ) -> TestResult {
        use super::super::market_stream::RuntimeMarketStreamStorageError as RuntimeError;
        let repository =
            OwnerMarketStreamRepository::new(database.worker.clone()).runtime_repository();
        let shared = repository.clone();
        let maximum = database.worker.options().get_max_connections();
        assert!((1..=16).contains(&maximum));
        let mut held = Vec::with_capacity(maximum as usize);
        for _ in 0..maximum {
            held.push(
                tokio::time::timeout(Duration::from_secs(2), database.worker.acquire()).await??,
            );
        }
        let role: String = tokio::time::timeout(
            Duration::from_secs(2),
            sqlx::query_scalar("SELECT current_user")
                .fetch_one(&mut **held.first_mut().ok_or("no held worker connection")?),
        )
        .await??;
        assert_eq!(role, "worker");
        let started = tokio::time::Instant::now();
        let mut call = Box::pin(repository.claim_stream_producer(
            fixture.credential_slot_id,
            Uuid::new_v4(),
            fixture.grant_revision,
        ));
        let result = if cancel {
            let result = tokio::select! {
                value = &mut call => Some(value),
                _ = tokio::time::sleep(Duration::from_millis(50)) => None,
            };
            drop(call);
            result
        } else {
            Some(call.await)
        };
        let elapsed = started.elapsed();
        // Return every owned connection before any result assertion can panic.
        drop(held);
        if cancel {
            assert!(
                result.is_none(),
                "pool-blocked call completed before cancellation"
            );
        } else {
            assert!(matches!(result, Some(Err(RuntimeError::DeadlineExceeded))));
            assert!(elapsed >= Duration::from_secs(1));
            assert!(elapsed < Duration::from_millis(1250));
        }
        let followup = tokio::time::timeout(
            Duration::from_millis(100),
            shared.claim_stream_producer(
                fixture.credential_slot_id,
                Uuid::new_v4(),
                fixture.grant_revision,
            ),
        )
        .await?;
        assert!(matches!(followup, Err(RuntimeError::Terminal)));
        assert_eq!(runtime_fault_producer_count(&database, &fixture).await?, 0);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn runtime_pool_acquisition_deadline_latches_without_claim() -> TestResult {
        run_database_case(|database, fixture| {
            Box::pin(runtime_pool_fault(database, fixture, false))
        })
        .await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn runtime_pending_pool_call_cancellation_latches_without_claim() -> TestResult {
        run_database_case(|database, fixture| Box::pin(runtime_pool_fault(database, fixture, true)))
            .await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn runtime_database_lock_deadline_rolls_back_and_latches() -> TestResult {
        run_database_case(|database, fixture| Box::pin(async move {
            use super::super::market_stream::RuntimeMarketStreamStorageError as RuntimeError;
            let repository = OwnerMarketStreamRepository::new(database.worker.clone()).runtime_repository();
            let shared = repository.clone();
            let mut blocker = tokio::time::timeout(Duration::from_secs(2), database.worker.begin()).await??;
            let (role, blocker_pid): (String, i32) = tokio::time::timeout(
                Duration::from_secs(2),
                sqlx::query_as("SELECT current_user::text, pg_catalog.pg_backend_pid()")
                    .fetch_one(&mut *blocker),
            ).await??;
            assert_eq!(role, "worker");
            tokio::time::timeout(
                Duration::from_secs(2),
                sqlx::query("SELECT pg_catalog.pg_advisory_xact_lock(pg_catalog.hashtextextended($1, 0))")
                    .bind(format!("owner-market-stream-producer:{}", fixture.credential_slot_id))
                    .execute(&mut *blocker),
            ).await??;
            let started = tokio::time::Instant::now();
            let result: Result<_, TestError> = async {
                let mut call = Box::pin(repository.claim_stream_producer(
                    fixture.credential_slot_id, Uuid::new_v4(), fixture.grant_revision,
                ));
                let observation_deadline = started + Duration::from_millis(600);
                loop {
                    let blocked = tokio::time::timeout_at(
                        observation_deadline,
                        sqlx::query_scalar::<_, i64>(
                            "SELECT count(*)::bigint FROM pg_catalog.pg_stat_activity
                              WHERE datname = pg_catalog.current_database()
                                AND usename = 'worker'
                                AND $1 = ANY(pg_catalog.pg_blocking_pids(pid))",
                        ).bind(blocker_pid).fetch_one(&database.migration_owner),
                    );
                    tokio::select! {
                        _ = &mut call => return Err(test_error("runtime claim completed before observed lock contention")),
                        count = blocked => {
                            let count = count??;
                            if count == 1 { break; }
                            if count != 0 { return Err(test_error("unexpected worker waiter count")); }
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                Ok((call.await, started.elapsed()))
            }.await;
            // The pending call is completed/dropped before releasing the owned lock.
            tokio::time::timeout(Duration::from_secs(2), blocker.rollback()).await??;
            let (result, elapsed) = result?;
            assert!(matches!(result,
                Err(RuntimeError::DeadlineExceeded)
                | Err(RuntimeError::Storage(MarketStreamStorageError::DatabaseUnavailable))
            ));
            assert!(elapsed >= Duration::from_secs(1));
            assert!(elapsed < Duration::from_millis(1250));
            let followup = tokio::time::timeout(Duration::from_millis(100), shared.claim_stream_producer(
                fixture.credential_slot_id, Uuid::new_v4(), fixture.grant_revision,
            )).await?;
            assert!(matches!(followup, Err(RuntimeError::Terminal)));
            assert_eq!(runtime_fault_producer_count(&database, &fixture).await?, 0);
            let waiters: i64 = tokio::time::timeout(
                Duration::from_secs(2),
                sqlx::query_scalar("SELECT count(*)::bigint FROM pg_catalog.pg_stat_activity
                    WHERE datname = pg_catalog.current_database() AND usename = 'worker'
                      AND $1 = ANY(pg_catalog.pg_blocking_pids(pid))")
                    .bind(blocker_pid).fetch_one(&database.migration_owner),
            ).await??;
            assert_eq!(waiters, 0);
            Ok(())
        })).await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn runtime_known_rights_denial_preserves_adapter() -> TestResult {
        run_database_case(|database, fixture| {
            Box::pin(async move {
                use super::super::market_stream::RuntimeMarketStreamStorageError as RuntimeError;
                let repository =
                    OwnerMarketStreamRepository::new(database.worker.clone()).runtime_repository();
                let holder = Uuid::new_v4();
                let wrong_grant = Uuid::new_v4();
                assert_ne!(wrong_grant, fixture.grant_revision);
                let denied = repository
                    .claim_stream_producer(fixture.credential_slot_id, holder, wrong_grant)
                    .await;
                assert!(matches!(
                    denied,
                    Err(RuntimeError::Storage(
                        MarketStreamStorageError::RightsInvalid
                    ))
                ));
                assert_eq!(runtime_fault_producer_count(&database, &fixture).await?, 0);
                let lease = repository
                    .claim_stream_producer(
                        fixture.credential_slot_id,
                        holder,
                        fixture.grant_revision,
                    )
                    .await?;
                assert_eq!(lease.holder_id, holder);
                let stored: (Uuid, i64) = tokio::time::timeout(
                    Duration::from_secs(2),
                    sqlx::query_as(
                        "SELECT holder_id, fencing_token FROM public.owner_market_stream_producers
                    WHERE credential_slot_id = $1",
                    )
                    .bind(fixture.credential_slot_id)
                    .fetch_one(&database.migration_owner),
                )
                .await??;
                assert_eq!(stored, (holder, i64::try_from(lease.fencing_token)?));
                assert_eq!(runtime_fault_producer_count(&database, &fixture).await?, 1);
                Ok(())
            })
        })
        .await
    }

    #[allow(dead_code)]
    fn _assert_runtime_config_type_is_safe(
        slot: Uuid,
        grant: Uuid,
        holder: Uuid,
    ) -> Result<
        OwnerMarketStreamRuntimeConfig,
        super::super::market_stream_runtime::MarketStreamRuntimeError,
    > {
        OwnerMarketStreamRuntimeConfig::from_values(slot, grant, &"a".repeat(64), 1, holder)
    }

    #[allow(dead_code)]
    fn _current_day_fixture_shape(
        date: chrono::NaiveDate,
        now: DateTime<Utc>,
        batch_id: Uuid,
    ) -> Result<
        super::super::market_stream_runtime::ResolvedMarketStreamDay,
        super::super::market_stream_runtime::MarketStreamRuntimeError,
    > {
        let calendar = IntradaySessionProof::new(
            date,
            batch_id,
            "a".repeat(64),
            format!("sha256:{}", "b".repeat(64)),
        )
        .map_err(|_| super::super::market_stream_runtime::MarketStreamRuntimeError::DayInvalid)?;
        fixture_day(
            calendar,
            now - ChronoDuration::seconds(1),
            now + ChronoDuration::seconds(1),
            Uuid::new_v4(),
        )
    }
}
