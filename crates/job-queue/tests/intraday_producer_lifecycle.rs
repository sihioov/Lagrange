mod intraday_producer_lifecycle_support;
#[allow(dead_code)]
mod intraday_producer_pipeline_support;
#[allow(dead_code)]
mod intraday_quotes_support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use collectors::intraday_quotes::{INTRADAY_QUOTE_PATH, INTRADAY_QUOTE_TR_ID};
use intraday_producer_lifecycle_support::{
    AttemptOutcomeClass, CacheRowFingerprint, ChildDropSignal, EligibilityBarrier,
    EligibilityHoldingReader, EligibilityObservation, EligibilityReturn, LifecycleTask,
    OutcomeBarrier, OutcomeHoldingReader, ResponseTask, add_new_generation,
    durable_signal_row_fingerprint, raw_cache_row_fingerprint, run_lifecycle_body,
    seed_published_signal_row, wait_for_blocked_backend, wait_for_blocked_backend_with_blocker,
    wait_for_database_time, wait_for_heartbeat_advance, wait_for_quote_version,
    wait_for_request_count,
};
use intraday_producer_pipeline_support::{
    ClientHarness, PipelineClient, RequestRecord, ResponseBarrier, TransportStep,
    install_current_window_contract, publication_counts, valid_quote_response,
};
use intraday_quotes_support::{IntradayTestDb, MembershipFixture};
use job_queue::owner_equity_v2::{
    IntradayProducer, IntradayProducerConfig, IntradayProducerCycleReport, IntradayQuoteReader,
    ProducerClaimKind,
};
use kis_client::error::KisError;
use serde_json::Value;
use tokio::sync::{oneshot, watch};
use uuid::Uuid;

const EOD_QUERY_PATH: &str = "/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice";
const EOD_QUERY_TR_ID: &str = "FHKST03010100";

fn demand_request(
    membership_id: Uuid,
    consumer_id: Uuid,
    key: &str,
) -> job_queue::owner_equity_v2::IntradayQuoteDemandRequest {
    job_queue::owner_equity_v2::IntradayQuoteDemandRequest::new(
        consumer_id,
        membership_id,
        1,
        0,
        key.to_owned(),
    )
    .expect("synthetic lifecycle demand is valid")
}

async fn setup_pipeline(
    db: &mut IntradayTestDb,
    suffix: &str,
    symbol: &str,
    close_after_seconds: i64,
    steps: impl IntoIterator<Item = TransportStep>,
) -> Result<
    (
        MembershipFixture,
        ClientHarness,
        Arc<IntradayProducer<PipelineClient>>,
    ),
    String,
> {
    let owner = db.seed_owner(suffix).await?;
    let fixture = db
        .seed_ready_membership(owner, &format!("{symbol}.KRX"))
        .await?;
    db.repository_as_app()
        .create_or_renew_demand(
            owner,
            &demand_request(fixture.membership_id, Uuid::new_v4(), suffix),
        )
        .await
        .map_err(|error| format!("lifecycle demand setup failed: {error}"))?;
    let windows = install_current_window_contract(db, close_after_seconds).await?;
    let harness = ClientHarness::new(suffix, steps);
    let config = IntradayProducerConfig::for_worker(&format!("b2b-c2b-{suffix}"))
        .map_err(|error| format!("lifecycle producer configuration failed: {error}"))?;
    let producer = Arc::new(IntradayProducer::new(
        db.repository_as_worker(),
        harness.client.clone(),
        windows,
        config,
    ));
    Ok((fixture, harness, producer))
}

async fn setup_outcome_holding(
    db: &mut IntradayTestDb,
    suffix: &str,
    symbol: &str,
    close_after_seconds: i64,
) -> Result<
    (
        MembershipFixture,
        ClientHarness,
        Arc<IntradayProducer<OutcomeHoldingReader>>,
        OutcomeBarrier,
        CacheRowFingerprint,
    ),
    String,
> {
    let owner = db.seed_owner(suffix).await?;
    let fixture = db
        .seed_ready_membership(owner, &format!("{symbol}.KRX"))
        .await?;
    db.repository_as_app()
        .create_or_renew_demand(
            owner,
            &demand_request(fixture.membership_id, Uuid::new_v4(), suffix),
        )
        .await
        .map_err(|error| format!("lifecycle demand setup failed: {error}"))?;
    let windows = install_current_window_contract(db, close_after_seconds).await?;
    let harness = ClientHarness::new(
        suffix,
        [
            TransportStep::Response(valid_quote_response(symbol)),
            TransportStep::Response(valid_quote_response(symbol)),
        ],
    );
    let first_config = IntradayProducerConfig::for_worker(&format!("b2b-c2b-{suffix}"))
        .map_err(|error| format!("lifecycle producer configuration failed: {error}"))?;
    let first_producer = Arc::new(IntradayProducer::new(
        db.repository_as_worker(),
        harness.client.clone(),
        windows.clone(),
        first_config,
    ));
    let (_first_shutdown_tx, mut first_shutdown_rx) = watch::channel(false);
    let first_report = first_producer
        .run_cycle(&mut first_shutdown_rx)
        .await
        .map_err(|error| format!("first-good producer cycle failed: {error}"))?;
    require_report_success(first_report, 1, 1)?;
    let first_request = wait_for_request_count(&harness.transport, 1, Duration::from_secs(2))
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| "first-good request was not recorded".to_owned())?;
    assert_exact_quote_request(&first_request, symbol)?;
    let initial_cache =
        raw_cache_row_fingerprint(&db.superuser, fixture.owner_user_id, fixture.membership_id)
            .await?;
    require(
        initial_cache.quote_version == 1
            && initial_cache.price.is_some()
            && initial_cache.received_at.is_some()
            && initial_cache.last_success_at.is_some()
            && initial_cache.last_failure_code.is_none()
            && initial_cache.last_failure_at.is_none(),
        "first-good lifecycle cycle did not leave a successful raw cache row",
    )?;
    wait_for_database_time(
        &db.superuser,
        initial_cache.last_attempt_at + chrono::Duration::seconds(5),
        Duration::from_secs(8),
    )
    .await?;
    let outcome_barrier = OutcomeBarrier::new();
    let reader = OutcomeHoldingReader::new(harness.client.clone(), outcome_barrier.clone());
    let config = IntradayProducerConfig::for_worker(&format!("b2b-c2b-{suffix}"))
        .map_err(|error| format!("lifecycle producer configuration failed: {error}"))?;
    let producer = Arc::new(IntradayProducer::new(
        db.repository_as_worker(),
        reader,
        windows,
        config,
    ));
    Ok((fixture, harness, producer, outcome_barrier, initial_cache))
}

fn spawn_cycle<R>(
    producer: Arc<IntradayProducer<R>>,
    outcome_barrier: Option<OutcomeBarrier>,
) -> LifecycleTask<
    Result<IntradayProducerCycleReport, job_queue::owner_equity_v2::IntradayProducerError>,
>
where
    R: IntradayQuoteReader + 'static,
{
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let handle = tokio::spawn(async move {
        let mut shutdown_rx = shutdown_rx;
        producer.run_cycle(&mut shutdown_rx).await
    });
    LifecycleTask::new(shutdown_tx, handle, outcome_barrier)
}

fn spawn_daemon<R>(
    producer: Arc<IntradayProducer<R>>,
) -> LifecycleTask<Result<(), job_queue::owner_equity_v2::IntradayProducerError>>
where
    R: IntradayQuoteReader + 'static,
{
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let handle = tokio::spawn(async move { producer.run_daemon(shutdown_rx).await });
    LifecycleTask::new(shutdown_tx, handle, None)
}

fn require(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.to_owned())
    }
}

fn require_report_success(
    report: IntradayProducerCycleReport,
    attempts: usize,
    successes: usize,
) -> Result<(), String> {
    if report.attempts_started != attempts
        || report.successful_quotes != successes
        || report.failures_recorded != 0
    {
        return Err(format!("unexpected lifecycle producer report: {report:?}"));
    }
    Ok(())
}

fn assert_exact_quote_request(record: &RequestRecord, symbol: &str) -> Result<(), String> {
    require(record.method == "GET", "lifecycle quote was not a GET")?;
    require(
        record.path == INTRADAY_QUOTE_PATH,
        "lifecycle quote used an unexpected path",
    )?;
    require(
        record.tr_id == INTRADAY_QUOTE_TR_ID,
        "lifecycle quote used an unexpected TR ID",
    )?;
    require(
        record.query
            == vec![
                ("FID_COND_MRKT_DIV_CODE".to_owned(), "J".to_owned()),
                ("FID_INPUT_ISCD".to_owned(), symbol.to_owned()),
            ],
        "lifecycle quote query was not the exact J/symbol query",
    )?;
    require(
        record.headers.get("custtype").map(String::as_str) == Some("P"),
        "lifecycle quote omitted the expected custtype",
    )?;
    require(
        !record.headers.contains_key("tr_cont"),
        "lifecycle quote sent a continuation header",
    )
}

fn require_second_real_quote(harness: &ClientHarness, symbol: &str) -> Result<(), String> {
    let requests = harness.transport.requests();
    require(
        requests.len() == 2,
        "lifecycle did not record exactly the first-good and held-outcome GETs",
    )?;
    assert_exact_quote_request(&requests[1], symbol)?;
    require(
        requests[1]
            .dispatched_at
            .duration_since(requests[0].dispatched_at)
            >= Duration::from_secs(5),
        "held-outcome GET did not preserve the real five-second spacing",
    )
}

fn state_attempts(harness: &ClientHarness, expected: u64) -> Result<(), String> {
    let state = harness.state();
    require(
        state.get("intraday_attempts").and_then(Value::as_u64) == Some(expected),
        "lifecycle ledger attempt count did not match the observed reservations",
    )
}

fn state_keeps_in_flight_reservation(harness: &ClientHarness) -> Result<(), String> {
    require(
        harness
            .state()
            .get("in_flight")
            .is_some_and(|value| !value.is_null()),
        "cancelled or caller-denied lifecycle attempt refunded its durable reservation",
    )
}

async fn cache_count(db: &IntradayTestDb, fixture: &MembershipFixture) -> Result<i64, String> {
    sqlx::query_scalar(
        "SELECT count(*)
           FROM public.owner_intraday_quote_cache
          WHERE owner_user_id = $1 AND membership_id = $2",
    )
    .bind(fixture.owner_user_id)
    .bind(fixture.membership_id)
    .fetch_one(&db.superuser)
    .await
    .map_err(|_| "could not count lifecycle quote cache rows".to_owned())
}

async fn wait_for_elapsed(
    start: Instant,
    duration: Duration,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        if start.elapsed() >= duration {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("timed out waiting for real shared-read spacing".to_owned());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn producer_row(
    db: &IntradayTestDb,
    owner_user_id: Uuid,
) -> Result<(Uuid, i64, DateTime<Utc>, DateTime<Utc>), String> {
    sqlx::query_as(
        "SELECT holder_id, fencing_token, lease_expires_at, heartbeat_at
           FROM public.owner_intraday_quote_producers
          WHERE owner_user_id = $1",
    )
    .bind(owner_user_id)
    .fetch_one(&db.superuser)
    .await
    .map_err(|_| "could not read lifecycle producer lease".to_owned())
}

#[tokio::test]
async fn lifecycle_daemon_automatically_dispatches_on_real_five_second_cadence_and_shutdowns() {
    run_lifecycle_body(|mut db| async move {
        let (fixture, harness, producer) = setup_pipeline(
            &mut db,
            "daemon-cadence",
            "005930",
            30,
            [
                TransportStep::Response(valid_quote_response("005930")),
                TransportStep::Response(valid_quote_response("005930")),
            ],
        )
        .await?;
        let daemon = spawn_daemon(producer);
        let requests =
            wait_for_request_count(&harness.transport, 2, Duration::from_secs(9)).await?;
        require(
            requests.len() == 2,
            "daemon made an unexpected number of requests",
        )?;
        assert_exact_quote_request(&requests[0], "005930")?;
        assert_exact_quote_request(&requests[1], "005930")?;
        require(
            requests[1]
                .dispatched_at
                .duration_since(requests[0].dispatched_at)
                >= Duration::from_secs(5),
            "daemon dispatches did not preserve the real five-second spacing",
        )?;
        wait_for_quote_version(
            &db.superuser,
            fixture.owner_user_id,
            fixture.membership_id,
            2,
            Duration::from_secs(2),
        )
        .await?;
        let daemon_result = daemon
            .shutdown(Duration::from_secs(3))
            .await
            .map_err(|error| format!("daemon cleanup failed: {error}"))?;
        daemon_result.map_err(|error| format!("daemon returned an error: {error}"))?;
        let request_count_after_join = harness.transport.request_count();
        tokio::time::sleep(Duration::from_millis(1100)).await;
        require(
            harness.transport.request_count() == request_count_after_join,
            "daemon dispatched after its bounded shutdown join",
        )?;
        state_attempts(&harness, 2)
    })
    .await;
}

#[tokio::test]
async fn lifecycle_inflight_shutdown_discards_publication_and_retains_real_debt() {
    run_lifecycle_body(|mut db| async move {
        let response_barrier = ResponseBarrier::new();
        let (fixture, harness, producer) = setup_pipeline(
            &mut db,
            "cancel-inflight",
            "005931",
            30,
            [TransportStep::ResponseAfterBarrier {
                barrier: response_barrier.clone(),
                response: valid_quote_response("005931"),
            }],
        )
        .await?;
        let task = spawn_cycle(producer, None);
        wait_for_request_count(&harness.transport, 1, Duration::from_secs(2)).await?;
        tokio::time::timeout(
            Duration::from_secs(2),
            response_barrier.wait_until_reached(),
        )
        .await
        .map_err(|_| "in-flight response barrier was not reached".to_owned())?;
        let task_result = task
            .shutdown(Duration::from_secs(3))
            .await
            .map_err(|error| format!("in-flight producer cleanup failed: {error}"))?;
        response_barrier.release();
        task_result.map_err(|error| format!("in-flight producer returned an error: {error}"))?;
        require(
            cache_count(&db, &fixture).await? == 0,
            "cancelled GET published a quote",
        )?;
        require(
            harness.transport.request_count() == 1,
            "in-flight cancellation caused an additional provider dispatch",
        )?;
        state_attempts(&harness, 1)?;
        state_keeps_in_flight_reservation(&harness)
    })
    .await;
}

#[tokio::test]
async fn lifecycle_delayed_real_outcome_receives_independent_heartbeat_then_publishes() {
    run_lifecycle_body(|mut db| async move {
        let (fixture, harness, producer, outcome_barrier, _initial_cache) =
            setup_outcome_holding(&mut db, "heartbeat-delayed-outcome", "005932", 30).await?;
        let task = spawn_cycle(producer, Some(outcome_barrier.clone()));
        wait_for_request_count(&harness.transport, 1, Duration::from_secs(2)).await?;
        tokio::time::timeout(Duration::from_secs(3), outcome_barrier.wait_for_capture())
            .await
            .map_err(|_| "post-capture outcome barrier was not reached".to_owned())?;
        require(
            outcome_barrier.outcome() == Some(AttemptOutcomeClass::Success),
            "held delayed outcome was not a successful actual client outcome",
        )?;
        let (_, _, _, heartbeat_before) = producer_row(&db, fixture.owner_user_id).await?;
        let heartbeat_after = wait_for_heartbeat_advance(
            &db.superuser,
            fixture.owner_user_id,
            heartbeat_before,
            Duration::from_secs(8),
        )
        .await?;
        require(
            heartbeat_after > heartbeat_before,
            "producer heartbeat did not advance while the real outcome was held",
        )?;
        let (_, _, lease_expires_after, _) = producer_row(&db, fixture.owner_user_id).await?;
        let database_now: DateTime<Utc> = sqlx::query_scalar("SELECT pg_catalog.clock_timestamp()")
            .fetch_one(&db.superuser)
            .await
            .map_err(|_| "could not sample live delayed-outcome clock".to_owned())?;
        require(
            lease_expires_after > database_now,
            "producer lease was not live while the real outcome was held",
        )?;
        let task_result = task
            .join(Duration::from_secs(5))
            .await
            .map_err(|error| format!("delayed-outcome task cleanup failed: {error}"))?;
        let report =
            task_result.map_err(|error| format!("delayed-outcome cycle failed: {error}"))?;
        require_report_success(report, 1, 1)?;
        wait_for_quote_version(
            &db.superuser,
            fixture.owner_user_id,
            fixture.membership_id,
            2,
            Duration::from_secs(2),
        )
        .await?;
        require_second_real_quote(&harness, "005932")?;
        state_attempts(&harness, 2)
    })
    .await;
}

#[tokio::test]
async fn lifecycle_blocked_heartbeat_expires_then_takeover_fences_old_outcome() {
    run_lifecycle_body(|mut db| async move {
        let (fixture, harness, producer, outcome_barrier, initial_cache) =
            setup_outcome_holding(&mut db, "heartbeat-takeover", "005933", 40).await?;
        let task = spawn_cycle(producer, Some(outcome_barrier.clone()));
        wait_for_request_count(&harness.transport, 1, Duration::from_secs(2)).await?;
        tokio::time::timeout(Duration::from_secs(3), outcome_barrier.wait_for_capture())
            .await
            .map_err(|_| "takeover outcome barrier was not reached".to_owned())?;
        require(
            outcome_barrier.outcome() == Some(AttemptOutcomeClass::Success),
            "held takeover outcome was not a successful actual client outcome",
        )?;

        let mut blocker = db
            .superuser
            .begin()
            .await
            .map_err(|_| "could not begin heartbeat blocker".to_owned())?;
        let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_catalog.pg_backend_pid()")
            .fetch_one(&mut *blocker)
            .await
            .map_err(|_| "could not identify heartbeat blocker".to_owned())?;
        sqlx::query(
            "SELECT owner_user_id
               FROM public.owner_intraday_quote_producers
              WHERE owner_user_id = $1
              FOR UPDATE",
        )
        .bind(fixture.owner_user_id)
        .fetch_one(&mut *blocker)
        .await
        .map_err(|_| "could not hold producer row for heartbeat blocker".to_owned())?;
        let lease_expires_at: DateTime<Utc> = sqlx::query_scalar(
            "SELECT lease_expires_at
               FROM public.owner_intraday_quote_producers
              WHERE owner_user_id = $1",
        )
        .bind(fixture.owner_user_id)
        .fetch_one(&mut *blocker)
        .await
        .map_err(|_| "could not record original producer lease expiry".to_owned())?;
        let blocker_now: DateTime<Utc> = sqlx::query_scalar("SELECT pg_catalog.clock_timestamp()")
            .fetch_one(&mut *blocker)
            .await
            .map_err(|_| "could not sample original lease clock".to_owned())?;
        let remaining = lease_expires_at - blocker_now;
        require(
            remaining >= chrono::Duration::seconds(15)
                && remaining <= chrono::Duration::seconds(25),
            "heartbeat fixture did not retain the configured approximately twenty-second lease",
        )?;
        let (_, blockers) = wait_for_blocked_backend(
            &db.superuser,
            "worker",
            "owner_intraday_quote_producers",
            Duration::from_secs(8),
        )
        .await?;
        require(
            blockers.contains(&blocker_pid),
            "heartbeat SQL was not blocked by the observed producer-row transaction",
        )?;
        wait_for_database_time(
            &db.superuser,
            lease_expires_at + chrono::Duration::milliseconds(100),
            Duration::from_secs(25),
        )
        .await?;
        let expired: bool = sqlx::query_scalar(
            "SELECT pg_catalog.clock_timestamp() >= lease_expires_at
               FROM public.owner_intraday_quote_producers
              WHERE owner_user_id = $1",
        )
        .bind(fixture.owner_user_id)
        .fetch_one(&mut *blocker)
        .await
        .map_err(|_| "could not verify heartbeat expiry while blocked".to_owned())?;
        require(
            expired,
            "heartbeat blocker released before the actual lease expiry",
        )?;
        blocker
            .commit()
            .await
            .map_err(|_| "could not release expired heartbeat blocker".to_owned())?;

        let takeover_holder = Uuid::new_v4();
        let takeover = db
            .repository_as_worker()
            .claim_producer(fixture.owner_user_id, takeover_holder)
            .await
            .map_err(|error| format!("public producer takeover failed: {error}"))?;
        require(
            takeover.kind == ProducerClaimKind::TakenOver && takeover.lease.fencing_token == 2,
            "public producer takeover did not increment the fencing token",
        )?;
        let task_result = task
            .join(Duration::from_secs(5))
            .await
            .map_err(|error| format!("takeover task cleanup failed: {error}"))?;
        let report = task_result.map_err(|error| format!("takeover cycle failed: {error}"))?;
        require(
            report.successful_quotes == 0 && report.failures_recorded == 0,
            "old delayed outcome was reported as a publication or failure after takeover",
        )?;
        require(
            raw_cache_row_fingerprint(&db.superuser, fixture.owner_user_id, fixture.membership_id)
                .await?
                == initial_cache,
            "old outcome changed the last-good cache row across the takeover fence",
        )?;
        require_second_real_quote(&harness, "005933")?;
        state_attempts(&harness, 2)
    })
    .await;
}

#[tokio::test]
async fn lifecycle_generation_change_and_disable_discard_late_real_results() {
    run_lifecycle_body(|mut db| async move {
        let (fixture, harness, producer, outcome_barrier, initial_cache) =
            setup_outcome_holding(&mut db, "generation-late-result", "005934", 30).await?;
        let task = spawn_cycle(producer, Some(outcome_barrier.clone()));
        wait_for_request_count(&harness.transport, 1, Duration::from_secs(2)).await?;
        tokio::time::timeout(Duration::from_secs(3), outcome_barrier.wait_for_capture())
            .await
            .map_err(|_| "generation late-result barrier was not reached".to_owned())?;
        require(
            outcome_barrier.outcome() == Some(AttemptOutcomeClass::Success),
            "held generation outcome was not a successful actual client outcome",
        )?;
        add_new_generation(&db, &fixture).await?;
        let task_result = task
            .join(Duration::from_secs(5))
            .await
            .map_err(|error| format!("generation late-result cleanup failed: {error}"))?;
        let report = task_result.map_err(|error| format!("generation cycle failed: {error}"))?;
        require(
            report.successful_quotes == 0 && report.failures_recorded == 0,
            "late generation-one result was reported instead of discarded",
        )?;
        require(
            raw_cache_row_fingerprint(&db.superuser, fixture.owner_user_id, fixture.membership_id)
                .await?
                == initial_cache,
            "generation fence changed the last-good cache row",
        )?;
        require_second_real_quote(&harness, "005934")?;
        state_attempts(&harness, 2)
    })
    .await;

    run_lifecycle_body(|mut db| async move {
        let (fixture, harness, producer, outcome_barrier, initial_cache) =
            setup_outcome_holding(&mut db, "disable-late-result", "005935", 30).await?;
        let task = spawn_cycle(producer, Some(outcome_barrier.clone()));
        wait_for_request_count(&harness.transport, 1, Duration::from_secs(2)).await?;
        tokio::time::timeout(Duration::from_secs(3), outcome_barrier.wait_for_capture())
            .await
            .map_err(|_| "disable late-result barrier was not reached".to_owned())?;
        require(
            outcome_barrier.outcome() == Some(AttemptOutcomeClass::Success),
            "held disabled-membership outcome was not a successful actual client outcome",
        )?;
        sqlx::query(
            "UPDATE public.owner_equity_memberships
                SET state = 'DISABLED', disabled_at = pg_catalog.clock_timestamp(),
                    updated_at = pg_catalog.clock_timestamp()
              WHERE id = $1",
        )
        .bind(fixture.membership_id)
        .execute(&db.superuser)
        .await
        .map_err(|_| "could not disable lifecycle membership fixture".to_owned())?;
        let task_result = task
            .join(Duration::from_secs(5))
            .await
            .map_err(|error| format!("disable late-result cleanup failed: {error}"))?;
        let report = task_result.map_err(|error| format!("disable cycle failed: {error}"))?;
        require(
            report.successful_quotes == 0 && report.failures_recorded == 0,
            "late disabled-membership result was reported instead of discarded",
        )?;
        require(
            raw_cache_row_fingerprint(&db.superuser, fixture.owner_user_id, fixture.membership_id)
                .await?
                == initial_cache,
            "disable fence changed the last-good cache row",
        )?;
        require_second_real_quote(&harness, "005935")?;
        state_attempts(&harness, 2)
    })
    .await;
}

#[tokio::test]
async fn lifecycle_demand_only_expiry_across_sql_barrier_denies_final_dispatch() {
    run_lifecycle_body(|mut db| async move {
        let owner = db.seed_owner("demand-only-expiry").await?;
        let fixture = db.seed_ready_membership(owner, "005936.KRX").await?;
        db.repository_as_app()
            .create_or_renew_demand(
                owner,
                &demand_request(fixture.membership_id, Uuid::new_v4(), "demand-only-expiry"),
            )
            .await
            .map_err(|error| format!("demand-only expiry setup failed: {error}"))?;
        let windows = install_current_window_contract(&mut db, 30).await?;
        let harness = ClientHarness::new(
            "demand-only-expiry",
            [TransportStep::Response(valid_quote_response("005936"))],
        );
        let eligibility_barrier = EligibilityBarrier::new();
        let eligibility_observation = EligibilityObservation::new();
        let reader = EligibilityHoldingReader::new(
            harness.client.clone(),
            eligibility_barrier.clone(),
            eligibility_observation.clone(),
        );
        let config = IntradayProducerConfig::for_worker("b2b-c2b-demand-only-expiry")
            .map_err(|error| format!("demand-only producer configuration failed: {error}"))?;
        let producer = Arc::new(IntradayProducer::new(
            db.repository_as_worker(),
            reader,
            windows.clone(),
            config,
        ));
        let task = spawn_cycle(producer, None);
        tokio::time::timeout(Duration::from_secs(2), eligibility_barrier.wait_for_entry())
            .await
            .map_err(|_| "final eligibility entry barrier was not reached".to_owned())?;

        let mut blocker = db
            .superuser
            .begin()
            .await
            .map_err(|_| "could not begin demand-only blocker".to_owned())?;
        let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_catalog.pg_backend_pid()")
            .fetch_one(&mut *blocker)
            .await
            .map_err(|_| "could not identify demand-only blocker".to_owned())?;
        sqlx::query(
            "SELECT owner_user_id
               FROM public.owner_intraday_quote_producers
              WHERE owner_user_id = $1
              FOR UPDATE",
        )
        .bind(owner)
        .fetch_one(&mut *blocker)
        .await
        .map_err(|_| "could not hold producer row for demand-only blocker".to_owned())?;
        sqlx::query(
            "UPDATE public.owner_intraday_quote_demands
                SET lease_expires_at = pg_catalog.clock_timestamp() + INTERVAL '2 seconds',
                    updated_at = pg_catalog.clock_timestamp()
              WHERE owner_user_id = $1 AND membership_id = $2",
        )
        .bind(owner)
        .bind(fixture.membership_id)
        .execute(&mut *blocker)
        .await
        .map_err(|_| "could not shorten only the demand lease".to_owned())?;
        let demand_expires_at: DateTime<Utc> = sqlx::query_scalar(
            "SELECT lease_expires_at
               FROM public.owner_intraday_quote_demands
              WHERE owner_user_id = $1 AND membership_id = $2",
        )
        .bind(owner)
        .bind(fixture.membership_id)
        .fetch_one(&mut *blocker)
        .await
        .map_err(|_| "could not record demand-only expiry".to_owned())?;
        eligibility_barrier.release();
        tokio::time::timeout(
            Duration::from_secs(2),
            eligibility_observation.wait_for_callback_entry(),
        )
        .await
        .map_err(|_| "final eligibility callback entry was not observed".to_owned())?;
        eligibility_observation.release_callback();
        let (waiter_pid, blockers) = wait_for_blocked_backend_with_blocker(
            &db.superuser,
            "worker",
            "owner_intraday_quote_producers",
            blocker_pid,
            Duration::from_secs(1),
        )
        .await?;
        let blocker_message = format!(
            "final eligibility SQL waiter {waiter_pid} did not retain blocker {blocker_pid}: {blockers:?}"
        );
        require(blockers.contains(&blocker_pid), &blocker_message)?;
        wait_for_database_time(
            &db.superuser,
            demand_expires_at + chrono::Duration::milliseconds(100),
            Duration::from_secs(5),
        )
        .await?;
        let blocker_now: DateTime<Utc> = sqlx::query_scalar("SELECT pg_catalog.clock_timestamp()")
            .fetch_one(&mut *blocker)
            .await
            .map_err(|_| "could not sample demand-only barrier clock".to_owned())?;
        let producer_expires_at: DateTime<Utc> = sqlx::query_scalar(
            "SELECT lease_expires_at
               FROM public.owner_intraday_quote_producers
              WHERE owner_user_id = $1",
        )
        .bind(owner)
        .fetch_one(&mut *blocker)
        .await
        .map_err(|_| "could not read producer expiry during demand-only barrier".to_owned())?;
        let (_, close_at) = windows
            .entry(db.session_date)
            .and_then(|entry| entry.utc_bounds())
            .ok_or_else(|| "demand-only window fixture had no bounds".to_owned())?;
        require(
            demand_expires_at <= blocker_now
                && producer_expires_at > blocker_now
                && close_at > blocker_now,
            "demand-only barrier did not keep producer and session window independently valid",
        )?;
        blocker
            .commit()
            .await
            .map_err(|_| "could not release demand-only SQL blocker".to_owned())?;
        let task_result = task
            .join(Duration::from_secs(5))
            .await
            .map_err(|error| format!("demand-only task cleanup failed: {error}"))?;
        let report = task_result.map_err(|error| format!("demand-only cycle failed: {error}"))?;
        tokio::time::timeout(
            Duration::from_secs(1),
            eligibility_observation.wait_for_callback_return(),
        )
        .await
        .map_err(|_| "final eligibility callback return was not observed".to_owned())?;
        require(
            eligibility_observation.returned() == Some(EligibilityReturn::False),
            "final eligibility callback did not return the observed false result",
        )?;
        require(
            eligibility_observation.outcome() == Some(AttemptOutcomeClass::CallerIneligible),
            "actual guarded client did not classify the callback result as CallerIneligible",
        )?;
        require(
            report.successful_quotes == 0,
            "expired demand published a quote",
        )?;
        require(
            harness.transport.request_count() == 0,
            "expired demand crossed the final eligibility guard and dispatched a GET",
        )?;
        state_attempts(&harness, 1)?;
        state_keeps_in_flight_reservation(&harness)?;
        require(
            cache_count(&db, &fixture).await? == 0,
            "demand-only expiry created a cache row",
        )
    })
    .await;
}

#[tokio::test]
async fn lifecycle_shared_eod_arbitration_then_quote_success_preserves_durable_eod_row() {
    run_lifecycle_body(|mut db| async move {
        let eod_barrier = ResponseBarrier::new();
        let (fixture, harness, producer) = setup_pipeline(
            &mut db,
            "shared-eod-arbitration",
            "005937",
            35,
            [
                TransportStep::ResponseAfterBarrier {
                    barrier: eod_barrier.clone(),
                    response: kis_client::transport::HttpResponse::ok(
                        r#"{"rt_cd":"0","output2":[]}"#,
                    ),
                },
                TransportStep::Response(valid_quote_response("005937")),
            ],
        )
        .await?;
        let (_, eod_before) = seed_published_signal_row(&db, &fixture).await?;
        let counts_before = publication_counts(&db.superuser).await?;
        let eod_client = harness.client.clone();
        let eod_handle = tokio::spawn(async move {
            eod_client
                .get(
                    EOD_QUERY_PATH,
                    EOD_QUERY_TR_ID,
                    &[("FID_INPUT_ISCD".to_owned(), "005937".to_owned())],
                    None,
                )
                .await
        });
        let eod_task = ResponseTask::new(eod_handle, eod_barrier.clone());
        tokio::time::timeout(Duration::from_secs(2), eod_barrier.wait_until_reached())
            .await
            .map_err(|_| "EOD response barrier was not reached".to_owned())?;

        let busy_task = spawn_cycle(producer.clone(), None);
        let busy_result = busy_task
            .join(Duration::from_secs(3))
            .await
            .map_err(|error| format!("busy quote cycle cleanup failed: {error}"))?;
        let busy_report =
            busy_result.map_err(|error| format!("busy quote cycle failed: {error}"))?;
        require(
            busy_report.successful_quotes == 0,
            "quote cycle reported success while shared EOD read was in flight",
        )?;
        let busy_requests = harness.transport.requests();
        require(
            busy_requests.len() == 1 && busy_requests[0].path == EOD_QUERY_PATH,
            "shared EOD arbitration allowed a quote transport dispatch",
        )?;

        let eod_result = eod_task
            .join(Duration::from_secs(3))
            .await
            .map_err(|error| format!("EOD response cleanup failed: {error}"))?;
        let eod_reply =
            eod_result.map_err(|error: KisError| format!("EOD read failed: {error}"))?;
        require(
            eod_reply.continuation.is_none(),
            "synthetic EOD arbitration unexpectedly returned continuation metadata",
        )?;
        let eod_record = harness
            .transport
            .requests()
            .into_iter()
            .find(|record| record.path == EOD_QUERY_PATH)
            .ok_or_else(|| "completed EOD request was not recorded".to_owned())?;
        wait_for_elapsed(
            eod_record.dispatched_at,
            Duration::from_secs(6),
            Duration::from_secs(8),
        )
        .await?;

        let success_task = spawn_cycle(producer, None);
        let success_result = success_task
            .join(Duration::from_secs(5))
            .await
            .map_err(|error| format!("post-EOD quote cleanup failed: {error}"))?;
        let success_report =
            success_result.map_err(|error| format!("post-EOD quote cycle failed: {error}"))?;
        require_report_success(success_report, 1, 1)?;
        let requests = harness.transport.requests();
        require(
            requests.len() == 2,
            "shared EOD scenario did not make exactly two reads",
        )?;
        require(
            requests[0].path == EOD_QUERY_PATH && requests[0].tr_id == EOD_QUERY_TR_ID,
            "first shared arbitration request was not the exact EOD read",
        )?;
        assert_exact_quote_request(&requests[1], "005937")?;
        require(
            requests[1]
                .dispatched_at
                .duration_since(requests[0].dispatched_at)
                >= Duration::from_secs(5),
            "quote did not honor shared EOD-to-intraday spacing",
        )?;
        require(
            harness.issuer.calls() == 1,
            "shared EOD arbitration issued a second token",
        )?;
        state_attempts(&harness, 1)?;
        wait_for_quote_version(
            &db.superuser,
            fixture.owner_user_id,
            fixture.membership_id,
            1,
            Duration::from_secs(2),
        )
        .await?;
        let eod_after =
            durable_signal_row_fingerprint(&db.superuser, eod_before.snapshot_id).await?;
        require(
            eod_after == eod_before,
            "intraday quote changed the UPDATE-sensitive durable EOD/signal row fingerprint",
        )?;
        require(
            publication_counts(&db.superuser).await? == counts_before,
            "intraday quote changed durable EOD/publication table counts",
        )
    })
    .await;
}

fn spawn_pending_child() -> (
    tokio::task::JoinHandle<()>,
    oneshot::Receiver<()>,
    oneshot::Receiver<()>,
    tokio::task::AbortHandle,
) {
    let (started_tx, started_rx) = oneshot::channel();
    let (dropped_tx, dropped_rx) = oneshot::channel();
    let handle = tokio::spawn(async move {
        let _ = started_tx.send(());
        let _drop_signal = ChildDropSignal::new(dropped_tx);
        std::future::pending::<()>().await;
    });
    let abort_handle = handle.abort_handle();
    (handle, started_rx, dropped_rx, abort_handle)
}

#[tokio::test]
async fn lifecycle_task_outer_join_cancellation_aborts_owned_child() {
    let (mut handle, started, dropped, abort_handle) = spawn_pending_child();
    let started = tokio::time::timeout(Duration::from_secs(1), started).await;
    if !matches!(started, Ok(Ok(()))) {
        handle.abort();
        let _ = tokio::time::timeout(Duration::from_secs(1), &mut handle).await;
        panic!("lifecycle child did not start");
    }
    let (shutdown_tx, _shutdown_rx) = watch::channel(false);
    let task = LifecycleTask::new(shutdown_tx, handle, None);
    let outer = tokio::time::timeout(
        Duration::from_millis(50),
        task.join(Duration::from_secs(30)),
    )
    .await;
    assert!(outer.is_err(), "outer lifecycle join did not time out");
    let mut dropped = dropped;
    let observed = tokio::time::timeout(Duration::from_secs(1), &mut dropped)
        .await
        .is_ok();
    if !observed {
        abort_handle.abort();
        let cleaned = tokio::time::timeout(Duration::from_secs(1), &mut dropped)
            .await
            .is_ok();
        assert!(
            cleaned,
            "lifecycle child did not clean up after forced abort"
        );
    }
    assert!(
        observed,
        "outer lifecycle join cancellation detached its pending child"
    );
}

#[tokio::test]
async fn response_task_outer_join_cancellation_aborts_owned_child() {
    let (mut handle, started, dropped, abort_handle) = spawn_pending_child();
    let started = tokio::time::timeout(Duration::from_secs(1), started).await;
    if !matches!(started, Ok(Ok(()))) {
        handle.abort();
        let _ = tokio::time::timeout(Duration::from_secs(1), &mut handle).await;
        panic!("response child did not start");
    }
    let task = ResponseTask::new(handle, ResponseBarrier::new());
    let outer = tokio::time::timeout(
        Duration::from_millis(50),
        task.join(Duration::from_secs(30)),
    )
    .await;
    assert!(outer.is_err(), "outer response join did not time out");
    let mut dropped = dropped;
    let observed = tokio::time::timeout(Duration::from_secs(1), &mut dropped)
        .await
        .is_ok();
    if !observed {
        abort_handle.abort();
        let cleaned = tokio::time::timeout(Duration::from_secs(1), &mut dropped)
            .await
            .is_ok();
        assert!(
            cleaned,
            "response child did not clean up after forced abort"
        );
    }
    assert!(
        observed,
        "outer response join cancellation detached its pending child"
    );
}
