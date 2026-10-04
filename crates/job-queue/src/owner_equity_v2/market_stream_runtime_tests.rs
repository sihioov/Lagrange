use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use crate::owner_equity_v2::market_stream::{
    OwnerMarketStreamRepository, StreamStatusCode, StreamSubscriptionDeliveryState,
};
use crate::owner_equity_v2::market_stream_runtime::{
    MarketStreamRuntimeError, MarketStreamRuntimeExit,
};
use chrono::NaiveDate;
use collectors::intraday_quotes::{
    IntradaySessionWindowContract, IntradaySessionWindowError, IntradaySessionWindowSource,
};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use uuid::Uuid;

#[path = "market_stream_runtime_test_support.rs"]
mod support;

use support::{
    CaseDeadlines, LeaseRenewer, LoopbackTransport, MetricsSnapshot, TestDatabase, TestFailure,
    TestResult,
};

const FIXTURE_PHASE_BOUND: Duration = Duration::from_secs(8);

tokio::task_local! {
    static WINDOW_SCOPE: watch::Receiver<Option<IntradaySessionWindowContract>>;
}

pub(super) fn load_window_contract(
    source: IntradaySessionWindowSource,
) -> Result<IntradaySessionWindowContract, IntradaySessionWindowError> {
    match WINDOW_SCOPE.try_with(|receiver| receiver.borrow().clone()) {
        Ok(Some(contract)) => Ok(contract),
        Ok(None) => Err(IntradaySessionWindowError::Missing),
        Err(_) => IntradaySessionWindowContract::from_source(source),
    }
}

type DaemonResult = Result<MarketStreamRuntimeExit, MarketStreamRuntimeError>;

struct RuntimeHarness {
    deadlines: CaseDeadlines,
    database: Option<TestDatabase>,
    transport: Option<LoopbackTransport>,
    shutdown: Option<watch::Sender<bool>>,
    window_scope: Option<watch::Sender<Option<IntradaySessionWindowContract>>>,
    daemon: Option<JoinHandle<DaemonResult>>,
    renewers: Vec<LeaseRenewer>,
    leases: Vec<(Uuid, Uuid)>,
    released_leases: BTreeSet<Uuid>,
}

impl RuntimeHarness {
    async fn create(deadlines: CaseDeadlines) -> support::SetupResult<Self> {
        let database = TestDatabase::create(deadlines).await?;
        let bind_result = tokio::time::timeout_at(
            deadlines.setup,
            LoopbackTransport::bind(
                database.fixture.credential_slot_id,
                database.fixture.session_date,
            ),
        )
        .await;
        let transport = match bind_result {
            Ok(Ok(transport)) if tokio::time::Instant::now() < deadlines.setup => transport,
            Ok(Ok(mut transport)) => {
                let transport_cleanup = transport.stop_and_join(deadlines.cleanup).await;
                let database_cleanup = database.cleanup_until(deadlines.cleanup).await;
                let primary = TestFailure("transport setup exceeded its absolute deadline");
                let cleanup = match (transport_cleanup, database_cleanup) {
                    (Ok(()), Ok(())) => None,
                    (Err(transport), Ok(())) => {
                        Some(Box::new(transport) as Box<dyn std::error::Error + Send + Sync>)
                    }
                    (Ok(()), Err(database)) => Some(database),
                    (Err(transport), Err(database)) => Some(support::combine_cleanup_failures(
                        Box::new(transport),
                        database,
                    )),
                };
                return match cleanup {
                    Some(cleanup) => Err(support::combine_setup_cleanup_failure(primary, cleanup)),
                    None => Err(Box::new(primary)),
                };
            }
            Ok(Err(primary)) => {
                return match database.cleanup_until(deadlines.cleanup).await {
                    Ok(()) => Err(Box::new(primary)),
                    Err(cleanup) => Err(support::combine_setup_cleanup_failure(primary, cleanup)),
                };
            }
            Err(_) => {
                let primary = TestFailure("transport setup exceeded its absolute deadline");
                return match database.cleanup_until(deadlines.cleanup).await {
                    Ok(()) => Err(Box::new(primary)),
                    Err(cleanup) => Err(support::combine_setup_cleanup_failure(primary, cleanup)),
                };
            }
        };
        Ok(Self {
            deadlines,
            database: Some(database),
            transport: Some(transport),
            shutdown: None,
            window_scope: None,
            daemon: None,
            renewers: Vec::new(),
            leases: Vec::new(),
            released_leases: BTreeSet::new(),
        })
    }

    fn database(&self) -> TestResult<&TestDatabase> {
        self.database
            .as_ref()
            .ok_or(TestFailure("test database already cleaned"))
    }

    fn transport(&self) -> TestResult<&LoopbackTransport> {
        self.transport
            .as_ref()
            .ok_or(TestFailure("loopback fixture already cleaned"))
    }

    fn daemon_is_running(&self) -> bool {
        self.daemon
            .as_ref()
            .is_some_and(|daemon| !daemon.is_finished())
    }

    async fn start(&mut self, include_window: bool) -> TestResult<()> {
        let database = self.database()?;
        let transport = self.transport()?;
        let runtime = database.runtime(transport)?;
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let window_value = include_window.then(|| database.window_contract.clone());
        let (window_tx, window_rx) = watch::channel(window_value);
        let scoped = WINDOW_SCOPE.scope(window_rx, runtime.run_daemon(shutdown_rx));
        let active = transport.task_guard();
        self.daemon = Some(tokio::spawn(async move {
            let _active = active;
            scoped.await
        }));
        self.shutdown = Some(shutdown_tx);
        self.window_scope = Some(window_tx);
        Ok(())
    }

    async fn add_consumer(&mut self) -> TestResult<Uuid> {
        let consumer_id = Uuid::new_v4();
        let (lease, app, owner, session_hash, identities, active) = {
            let database = self.database()?;
            let identities = database.fixture.lease_identities();
            let lease = database
                .create_lease(consumer_id, identities.clone())
                .await?;
            let active = self.transport()?.active_counter();
            (
                lease,
                database.database.app.clone(),
                database.fixture.owner_user_id,
                database.fixture.owner_session_hash.clone(),
                identities,
                active,
            )
        };
        self.leases.push((lease.lease_id, consumer_id));
        self.renewers.push(LeaseRenewer::start(
            app,
            owner,
            session_hash,
            consumer_id,
            identities,
            lease.renewal_sequence,
            active,
        ));
        Ok(lease.lease_id)
    }

    async fn release_consumer(&mut self, index: usize) -> TestResult<()> {
        let (lease_id, _) = *self
            .leases
            .get(index)
            .ok_or(TestFailure("consumer lease index invalid"))?;
        let renewer = self
            .renewers
            .get_mut(index)
            .ok_or(TestFailure("consumer renewer index invalid"))?;
        let sequence = renewer.stop_and_join(self.deadlines.work).await?;
        let database = self.database()?;
        let operation_deadline =
            (tokio::time::Instant::now() + FIXTURE_PHASE_BOUND).min(self.deadlines.work);
        let outcome = tokio::time::timeout_at(
            operation_deadline,
            OwnerMarketStreamRepository::new(database.database.app.clone()).release_stream_lease(
                database.fixture.owner_user_id,
                &database.fixture.owner_session_hash,
                lease_id,
                sequence,
            ),
        )
        .await
        .map_err(|_| TestFailure("synthetic browser lease release exceeded its bound"))?
        .map_err(|_| TestFailure("synthetic consumer lease release failed"))?;
        if !outcome.released {
            return Err(TestFailure("synthetic consumer lease did not release"));
        }
        self.released_leases.insert(lease_id);
        Ok(())
    }

    async fn shutdown_and_join(
        &mut self,
        absolute_deadline: tokio::time::Instant,
    ) -> TestResult<MarketStreamRuntimeExit> {
        if let Some(shutdown) = &self.shutdown {
            shutdown.send_replace(true);
        }
        if self.daemon.is_none() {
            return Err(TestFailure("runtime daemon was not started"));
        }
        let outcome = {
            let daemon = self.daemon.as_mut().expect("checked runtime handle");
            let phase_deadline =
                (tokio::time::Instant::now() + Duration::from_secs(15)).min(absolute_deadline);
            match tokio::time::timeout_at(phase_deadline, &mut *daemon).await {
                Ok(Ok(Ok(exit))) => Ok(exit),
                Ok(Ok(Err(_))) => Err(TestFailure("runtime daemon returned a typed failure")),
                Ok(Err(_)) => Err(TestFailure("runtime daemon task failed to join")),
                Err(_) => {
                    daemon.abort();
                    let _ = daemon.await;
                    Err(TestFailure("runtime daemon exceeded its cleanup bound"))
                }
            }
        };
        if tokio::time::Instant::now() > absolute_deadline {
            self.daemon.take();
            return Err(TestFailure(
                "runtime daemon completed beyond its aggregate deadline",
            ));
        }
        self.daemon.take();
        outcome
    }

    async fn cleanup(&mut self) -> TestResult<()> {
        let mut first_error = None;
        for renewer in &self.renewers {
            renewer.request_stop();
        }
        if self.daemon.is_some() {
            match self.shutdown_and_join(self.deadlines.cleanup).await {
                Ok(MarketStreamRuntimeExit::Shutdown) => {}
                Ok(_) => {
                    first_error.get_or_insert(TestFailure("runtime returned an unexpected exit"));
                }
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            };
        }
        let renewer_deadline =
            (tokio::time::Instant::now() + FIXTURE_PHASE_BOUND).min(self.deadlines.cleanup);
        for renewer in &mut self.renewers {
            if let Err(error) = renewer.join_until(renewer_deadline).await {
                first_error.get_or_insert(error);
            }
        }
        if let Some(transport) = &mut self.transport
            && let Err(error) = transport.stop_and_join(self.deadlines.cleanup).await
        {
            first_error.get_or_insert(error);
        }
        if let Some(database) = self.database.take() {
            if let Err(error) = database.cleanup_until(self.deadlines.cleanup).await {
                first_error.get_or_insert(support::cleanup_test_failure(error.as_ref()));
            }
        }
        if tokio::time::Instant::now() > self.deadlines.cleanup {
            first_error.get_or_insert(TestFailure(
                "runtime fixture cleanup exceeded its aggregate deadline",
            ));
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

fn check(condition: bool, message: &'static str) -> TestResult<()> {
    if condition {
        Ok(())
    } else {
        Err(TestFailure(message))
    }
}

fn quiet_transport(metrics: &MetricsSnapshot) -> TestResult<()> {
    check(
        metrics.approval_accepts == 0
            && metrics.websocket_accepts == 0
            && metrics.current_websockets == 0
            && metrics.subscribe_commands == 0
            && metrics.unsubscribe_commands == 0
            && metrics.command_events.is_empty()
            && metrics.unexpected_requests_or_commands == 0
            && metrics.server_errors == 0
            && !metrics.event_overflow,
        "runtime performed a forbidden pre-eligibility transport operation",
    )
}

fn snapshot_epoch_and_live_rows(
    snapshot: &crate::owner_equity_v2::market_stream::StreamSnapshot,
    date: NaiveDate,
) -> TestResult<Uuid> {
    check(
        snapshot.delivery_rows.len() == 30,
        "authorized snapshot did not contain 30 identities",
    )?;
    let first = snapshot
        .delivery_rows
        .first()
        .ok_or(TestFailure("snapshot delivery rows missing"))?;
    let producer = first
        .producer
        .as_ref()
        .ok_or(TestFailure("snapshot producer lineage missing"))?;
    let epoch = producer
        .current_epoch
        .ok_or(TestFailure("snapshot has no current epoch"))?;
    check(!epoch.is_nil(), "snapshot epoch was nil")?;
    for row in &snapshot.delivery_rows {
        check(row.live, "authorized snapshot row was not LIVE")?;
        let producer = row
            .producer
            .as_ref()
            .ok_or(TestFailure("delivery producer metadata missing"))?;
        let subscription = row
            .subscription
            .as_ref()
            .ok_or(TestFailure("delivery ACK metadata missing"))?;
        let cache = row
            .cache
            .as_ref()
            .ok_or(TestFailure("delivery cache row missing"))?;
        check(
            producer.current_epoch == Some(epoch)
                && producer.session_date == Some(date)
                && producer.session_proof_id.is_some()
                && producer.session_proof_sha256.is_some()
                && producer.calendar_source_batch_id.is_some()
                && producer.calendar_content_sha256.is_some()
                && producer.window_contract_sha256.is_some()
                && subscription.state == StreamSubscriptionDeliveryState::Acked
                && subscription.epoch == Some(epoch)
                && cache.epoch == Some(epoch)
                && cache.quote_version > 0
                && cache.receive_ordinal.is_some()
                && cache.received_at.is_some()
                && cache.committed_at.is_some(),
            "snapshot lineage or receipt metadata was incomplete",
        )?;
    }
    Ok(epoch)
}

async fn wait_live_snapshot(
    harness: &RuntimeHarness,
    lease_id: Uuid,
    deadline: Instant,
) -> TestResult<crate::owner_equity_v2::market_stream::StreamSnapshot> {
    loop {
        if !harness.daemon_is_running() {
            return Err(TestFailure("runtime daemon stopped before a LIVE snapshot"));
        }
        let database = harness.database()?;
        let snapshot = database
            .snapshot(lease_id, Some(database.snapshot_window.clone()))
            .await?;
        if snapshot.delivery_rows.len() == 30 && snapshot.delivery_rows.iter().all(|row| row.live) {
            return Ok(snapshot);
        }
        if Instant::now() >= deadline {
            return Err(TestFailure("bounded wait for LIVE snapshot expired"));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn observe_runtime_for(harness: &RuntimeHarness, duration: Duration) -> TestResult<()> {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        if !harness.daemon_is_running() {
            return Err(TestFailure(
                "runtime daemon stopped during observation window",
            ));
        }
        if harness.renewers.len() != harness.leases.len()
            || harness
                .renewers
                .iter()
                .zip(&harness.leases)
                .any(|(renewer, (lease_id, _))| {
                    !harness.released_leases.contains(lease_id) && !renewer.is_running()
                })
        {
            return Err(TestFailure("a browser lease renewer stopped unexpectedly"));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Ok(())
}

fn finish_case(primary: TestResult<()>, cleanup: TestResult<()>) {
    match (primary, cleanup) {
        (Ok(()), Ok(())) => {}
        (Err(primary), Ok(())) => panic!("runtime integration case failed: {primary}"),
        (Ok(()), Err(cleanup)) => panic!("runtime integration cleanup failed: {cleanup}"),
        (Err(primary), Err(cleanup)) => {
            panic!("runtime integration case failed: {primary}; cleanup also failed: {cleanup}")
        }
    }
}

async fn bounded_case<F>(deadline: tokio::time::Instant, case: F) -> TestResult<()>
where
    F: std::future::Future<Output = TestResult<()>>,
{
    tokio::time::timeout_at(deadline, case)
        .await
        .unwrap_or(Err(TestFailure(
            "runtime integration case exceeded its absolute work deadline",
        )))
}

#[tokio::test]
async fn runtime_no_demand_keeps_transport_unopened() {
    let deadlines = CaseDeadlines::from_test_entry();
    let mut harness = RuntimeHarness::create(deadlines)
        .await
        .unwrap_or_else(|error| panic!("guarded runtime fixture setup failed: {error}"));
    let primary = bounded_case(deadlines.work, async {
        harness.start(true).await?;
        observe_runtime_for(&harness, Duration::from_secs(3)).await?;
        let database = harness.database()?;
        check(
            database.producer_state().await?.is_none(),
            "zero demand created a producer claim",
        )?;
        check(
            database.quote_version_sum().await? == 0,
            "zero demand wrote a cache quote",
        )?;
        quiet_transport(&harness.transport()?.metrics()?)
    })
    .await;
    let cleanup = harness.cleanup().await;
    let final_metrics = harness.transport().and_then(LoopbackTransport::metrics);
    finish_case(primary, cleanup);
    check(
        final_metrics.is_ok_and(|metrics| metrics.active_fixture_tasks == 0),
        "no-demand fixture children did not all join",
    )
    .unwrap_or_else(|_| panic!("no-demand fixture task cleanup failed"));
    check(
        tokio::time::Instant::now() <= deadlines.cleanup,
        "no-demand case exceeded its aggregate cleanup deadline",
    )
    .unwrap_or_else(|_| panic!("no-demand case exceeded its aggregate cleanup deadline"));
}

#[tokio::test]
async fn runtime_missing_window_keeps_positive_demand_off_transport() {
    let deadlines = CaseDeadlines::from_test_entry();
    let mut harness = RuntimeHarness::create(deadlines)
        .await
        .unwrap_or_else(|error| panic!("guarded runtime fixture setup failed: {error}"));
    let primary = bounded_case(deadlines.work, async {
        let lease_id = harness.add_consumer().await?;
        harness.start(false).await?;
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut reached_status = false;
        loop {
            if !harness.daemon_is_running() {
                return Err(TestFailure(
                    "runtime daemon stopped before missing-window status",
                ));
            }
            let database = harness.database()?;
            let snapshot = database.snapshot(lease_id, None).await?;
            if snapshot.delivery_rows.len() == 30
                && snapshot.delivery_rows.iter().all(|row| {
                    !row.live
                        && row.producer.as_ref().is_some_and(|producer| {
                            producer.reason == Some(StreamStatusCode::SessionWindowUnavailable)
                                && producer.current_epoch.is_none()
                        })
                        && row
                            .cache
                            .as_ref()
                            .is_none_or(|cache| cache.quote_version == 0 && cache.quote.is_none())
                })
            {
                reached_status = true;
                break;
            }
            if Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        check(
            reached_status,
            "missing window did not reach typed unavailable status",
        )?;
        observe_runtime_for(&harness, Duration::from_secs(3)).await?;
        quiet_transport(&harness.transport()?.metrics()?)
    })
    .await;
    let cleanup = harness.cleanup().await;
    let final_metrics = harness.transport().and_then(LoopbackTransport::metrics);
    finish_case(primary, cleanup);
    check(
        final_metrics.is_ok_and(|metrics| metrics.active_fixture_tasks == 0),
        "missing-window fixture children did not all join",
    )
    .unwrap_or_else(|_| panic!("missing-window fixture task cleanup failed"));
    check(
        tokio::time::Instant::now() <= deadlines.cleanup,
        "missing-window case exceeded its aggregate cleanup deadline",
    )
    .unwrap_or_else(|_| panic!("missing-window case exceeded its aggregate cleanup deadline"));
}

#[tokio::test]
async fn runtime_thirty_symbols_share_one_socket_across_ten_consumers() {
    let deadlines = CaseDeadlines::from_test_entry();
    let mut harness = RuntimeHarness::create(deadlines)
        .await
        .unwrap_or_else(|error| panic!("guarded runtime fixture setup failed: {error}"));
    let primary = bounded_case(deadlines.work, async {
        let first_lease = harness.add_consumer().await?;
        harness.start(true).await?;
        let initial = wait_live_snapshot(
            &harness,
            first_lease,
            Instant::now() + Duration::from_secs(60),
        )
        .await?;
        let date = harness.database()?.fixture.session_date;
        let epoch = snapshot_epoch_and_live_rows(&initial, date)?;
        let initial_versions = initial
            .delivery_rows
            .iter()
            .filter_map(|row| {
                row.cache
                    .as_ref()
                    .map(|cache| (row.identity.membership_id, cache.quote_version))
            })
            .collect::<BTreeMap<_, _>>();
        let quote_deadline = Instant::now() + Duration::from_secs(6);
        let mut version_advanced = false;
        while Instant::now() < quote_deadline && !version_advanced {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let database = harness.database()?;
            let snapshot = database
                .snapshot(first_lease, Some(database.snapshot_window.clone()))
                .await?;
            version_advanced = snapshot.delivery_rows.iter().any(|row| {
                row.cache.as_ref().is_some_and(|cache| {
                    initial_versions
                        .get(&row.identity.membership_id)
                        .is_some_and(|initial| cache.quote_version > *initial)
                })
            });
        }
        check(
            version_advanced,
            "runtime did not commit a later authentic receipt",
        )?;

        for _ in 1..10 {
            harness.add_consumer().await?;
        }
        observe_runtime_for(&harness, Duration::from_secs(4)).await?;
        let counts = harness.database()?.reference_counts().await?;
        check(
            counts.len() == 30 && counts.iter().all(|(_, count)| *count == 10),
            "ten consumers did not produce desired reference count ten",
        )?;
        for (lease_id, _) in &harness.leases {
            let database = harness.database()?;
            let snapshot = database
                .snapshot(*lease_id, Some(database.snapshot_window.clone()))
                .await?;
            check(
                snapshot_epoch_and_live_rows(&snapshot, date)? == epoch,
                "consumer snapshot changed epoch or lost live identity rows",
            )?;
        }
        let before_release = harness.transport()?.metrics()?;
        check(
            before_release.approval_accepts == 1
                && before_release.websocket_accepts == 1
                && before_release.maximum_websockets == 1
                && before_release.subscribe_commands == 30
                && before_release.unsubscribe_commands == 0,
            "additional consumer leases changed the socket or command set",
        )?;

        for index in 1..10 {
            harness.release_consumer(index).await?;
        }
        observe_runtime_for(&harness, Duration::from_secs(4)).await?;
        let counts = harness.database()?.reference_counts().await?;
        check(
            counts.len() == 30 && counts.iter().all(|(_, count)| *count == 1),
            "single remaining consumer did not restore reference count one",
        )?;
        let final_snapshot = harness
            .database()?
            .snapshot(
                first_lease,
                Some(harness.database()?.snapshot_window.clone()),
            )
            .await?;
        check(
            snapshot_epoch_and_live_rows(&final_snapshot, date)? == epoch,
            "single-consumer snapshot lost its live epoch",
        )?;
        let metrics = harness.transport()?.metrics()?;
        check(
            metrics.approval_accepts == 1
                && metrics.websocket_accepts == 1
                && metrics.current_websockets == 1
                && metrics.maximum_websockets == 1
                && metrics.subscribe_commands == 30
                && metrics.unsubscribe_commands == 0
                && metrics.command_events.len() == 30
                && !metrics.event_overflow
                && metrics.unexpected_requests_or_commands == 0
                && metrics.server_errors == 0,
            "runtime did not retain one socket and exactly thirty subscriptions",
        )?;
        let unique_symbols = metrics
            .command_events
            .iter()
            .map(|event| event.symbol.as_str())
            .collect::<BTreeSet<_>>();
        check(
            unique_symbols.len() == 30,
            "subscription command symbols were not unique",
        )?;
        check(
            metrics
                .command_events
                .iter()
                .all(|event| event.kind == support::CommandKind::Subscribe),
            "unexpected unsubscribe command was recorded",
        )?;
        let command_state = harness.database()?.command_state(harness.transport()?)?;
        check(
            command_state.command_attempts_ms.len() == 30
                && command_state
                    .command_attempts_ms
                    .windows(2)
                    .all(|pair| pair[1] >= pair[0] && pair[1].saturating_sub(pair[0]) >= 1_000)
                && command_state.pending_command.is_none(),
            "durable command pacing did not show thirty spaced completed attempts",
        )?;

        check(
            harness.shutdown_and_join(deadlines.work).await? == MarketStreamRuntimeExit::Shutdown,
            "runtime did not return the requested shutdown outcome",
        )?;
        let versions_after_return = harness.database()?.cache_versions().await?;
        let retired = harness.database()?.producer_state().await?;
        check(
            retired.is_some_and(|(epoch, state, fence)| {
                epoch.is_none() && state == "STOPPED" && fence > 0
            }),
            "runtime shutdown did not retire its current producer epoch",
        )?;
        tokio::time::sleep(Duration::from_millis(1_100)).await;
        let versions_after_shutdown = harness.database()?.cache_versions().await?;
        check(
            versions_after_return == versions_after_shutdown,
            "cache changed after the runtime returned",
        )?;
        let reservation = harness
            .transport()?
            .client
            .reserve_connection()
            .map_err(|_| TestFailure("clean shutdown did not release the connection anchor"))?;
        drop(reservation);
        Ok(())
    })
    .await;
    let cleanup = harness.cleanup().await;
    let final_metrics = harness.transport().and_then(LoopbackTransport::metrics);
    finish_case(primary, cleanup);
    check(
        final_metrics.is_ok_and(|metrics| {
            metrics.client_close_seen
                && metrics.current_websockets == 0
                && metrics.active_fixture_tasks == 0
                && metrics.server_errors == 0
        }),
        "runtime socket or synthetic fixture children did not close cleanly",
    )
    .unwrap_or_else(|_| panic!("runtime transport cleanup verification failed"));
    check(
        tokio::time::Instant::now() <= deadlines.cleanup,
        "multi-consumer case exceeded its aggregate cleanup deadline",
    )
    .unwrap_or_else(|_| panic!("multi-consumer case exceeded its aggregate cleanup deadline"));
}
