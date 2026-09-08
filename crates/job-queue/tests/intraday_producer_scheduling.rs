#[allow(dead_code)]
mod intraday_producer_pipeline_support;
#[allow(dead_code)]
mod intraday_quotes_support;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use collectors::intraday_quotes::{INTRADAY_QUOTE_PATH, INTRADAY_QUOTE_TR_ID};
use intraday_producer_pipeline_support::{
    ClientHarness, PipelineClient, RequestRecord, TransportStep, assert_minimum_dispatch_spacing,
    halted_quote_response, install_current_window_contract, rate_limited_response,
    valid_quote_response,
};
use intraday_quotes_support::{IntradayTestDb, MembershipFixture, run_body};
use job_queue::owner_equity_v2::{
    IntradayCacheRecord, IntradayProducer, IntradayProducerConfig, IntradayProducerCycleReport,
    IntradayQuoteDemandRequest, IntradayQuoteFailureCode,
};
use kis_client::Clock;
use kis_client::clock::SystemClock;
use serde_json::Value;
use sqlx::PgPool;
use tokio::sync::watch;
use uuid::Uuid;

#[derive(Clone)]
struct DemandHandle {
    fixture: MembershipFixture,
    consumer_id: Uuid,
    idempotency_prefix: String,
}

fn demand_request(
    fixture: &MembershipFixture,
    consumer_id: Uuid,
    sequence: u64,
    idempotency_key: String,
) -> IntradayQuoteDemandRequest {
    IntradayQuoteDemandRequest::new(
        consumer_id,
        fixture.membership_id,
        fixture.generation,
        sequence,
        idempotency_key,
    )
    .expect("synthetic demand request is valid")
}

async fn add_demand(
    db: &IntradayTestDb,
    fixture: MembershipFixture,
    idempotency_prefix: &str,
) -> Result<DemandHandle, String> {
    let consumer_id = Uuid::new_v4();
    db.repository_as_app()
        .create_or_renew_demand(
            fixture.owner_user_id,
            &demand_request(&fixture, consumer_id, 0, idempotency_prefix.to_owned()),
        )
        .await
        .map_err(|_| "synthetic demand setup failed".to_owned())?;
    Ok(DemandHandle {
        fixture,
        consumer_id,
        idempotency_prefix: idempotency_prefix.to_owned(),
    })
}

struct DemandRenewalTask {
    stop_tx: watch::Sender<bool>,
    join: Option<tokio::task::JoinHandle<Result<(), String>>>,
}

fn start_demand_renewals(db: &IntradayTestDb, demands: Vec<DemandHandle>) -> DemandRenewalTask {
    let app = db.repository_as_app();
    let (stop_tx, mut stop_rx) = watch::channel(false);
    let join = tokio::spawn(async move {
        let mut next_sequences = vec![1_u64; demands.len()];
        loop {
            tokio::select! {
                changed = stop_rx.changed() => {
                    if changed.is_err() || *stop_rx.borrow() {
                        return Ok(());
                    }
                }
                _ = tokio::time::sleep(Duration::from_secs(15)) => {}
            }

            for (index, demand) in demands.iter().enumerate() {
                if *stop_rx.borrow() {
                    return Ok(());
                }
                let sequence = next_sequences[index];
                let request = demand_request(
                    &demand.fixture,
                    demand.consumer_id,
                    sequence,
                    format!("{}-renew-{sequence}", demand.idempotency_prefix),
                );
                app.create_or_renew_demand(demand.fixture.owner_user_id, &request)
                    .await
                    .map_err(|_| "synthetic demand renewal failed".to_owned())?;
                next_sequences[index] = sequence
                    .checked_add(1)
                    .ok_or_else(|| "synthetic demand renewal sequence overflow".to_owned())?;
            }
        }
    });
    DemandRenewalTask {
        stop_tx,
        join: Some(join),
    }
}

impl DemandRenewalTask {
    async fn stop(mut self) -> Result<(), String> {
        let _ = self.stop_tx.send(true);
        self.join
            .take()
            .ok_or_else(|| "synthetic demand renewal task was already stopped".to_owned())?
            .await
            .map_err(|_| "synthetic demand renewal task failed".to_owned())?
    }
}

impl Drop for DemandRenewalTask {
    fn drop(&mut self) {
        let _ = self.stop_tx.send(true);
        if let Some(join) = self.join.as_ref() {
            join.abort();
        }
    }
}

struct TaskDropSignal(Arc<AtomicBool>);

impl Drop for TaskDropSignal {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

async fn run_cycle(
    producer: &IntradayProducer<PipelineClient>,
) -> Result<IntradayProducerCycleReport, String> {
    let (_shutdown_tx, mut shutdown_rx) = watch::channel(false);
    producer
        .run_cycle(&mut shutdown_rx)
        .await
        .map_err(|_| "intraday producer cycle failed".to_owned())
}

async fn read_cache(
    db: &IntradayTestDb,
    fixture: &MembershipFixture,
) -> Result<IntradayCacheRecord, String> {
    db.repository_as_app()
        .read_current_cache(
            fixture.owner_user_id,
            fixture.membership_id,
            fixture.generation,
            &db.session_proof(),
        )
        .await
        .map_err(|_| "current intraday cache read failed".to_owned())?
        .ok_or_else(|| "expected current intraday cache row".to_owned())
}

fn assert_exact_get(record: &RequestRecord, symbol: &str) {
    assert_eq!(record.method, "GET");
    assert_eq!(record.path, INTRADAY_QUOTE_PATH);
    assert_eq!(record.tr_id, INTRADAY_QUOTE_TR_ID);
    assert_eq!(
        record.query,
        vec![
            ("FID_COND_MRKT_DIV_CODE".to_owned(), "J".to_owned()),
            ("FID_INPUT_ISCD".to_owned(), symbol.to_owned()),
        ]
    );
    assert_eq!(
        record.headers.get("custtype").map(String::as_str),
        Some("P")
    );
    assert!(!record.headers.contains_key("tr_cont"));
}

fn assert_request_sequence(harness: &ClientHarness, symbols: &[&str]) {
    let requests = harness.transport.requests();
    assert_eq!(requests.len(), symbols.len());
    for (request, symbol) in requests.iter().zip(symbols) {
        assert_exact_get(request, symbol);
    }
}

fn reservation_ledger(harness: &ClientHarness) -> Value {
    let state = harness.state();
    Value::Array(
        [
            "intraday_kst_date",
            "intraday_attempts",
            "next_fence",
            "in_flight",
        ]
        .into_iter()
        .map(|key| state.get(key).cloned().unwrap_or(Value::Null))
        .collect(),
    )
}

fn assert_ledger(
    harness: &ClientHarness,
    session_date: chrono::NaiveDate,
    attempts: u64,
    next_fence: u64,
) {
    let state = harness.state();
    assert_eq!(
        state.get("intraday_kst_date").and_then(Value::as_str),
        Some(session_date.to_string().as_str())
    );
    assert_eq!(
        state.get("intraday_attempts").and_then(Value::as_u64),
        Some(attempts)
    );
    assert_eq!(
        state.get("next_fence").and_then(Value::as_u64),
        Some(next_fence)
    );
}

fn assert_failure_without_success(
    cache: &IntradayCacheRecord,
    failure: IntradayQuoteFailureCode,
    producer_fence: u64,
) {
    assert_eq!(cache.price, None);
    assert_eq!(cache.received_at, None);
    assert_eq!(cache.last_success_at, None);
    assert_eq!(cache.quote_version, 0);
    assert_eq!(cache.last_failure_code, Some(failure));
    assert!(cache.last_failure_at.is_some());
    assert_eq!(cache.producer_fence, producer_fence);
}

fn state_i64(state: &Value, key: &str) -> Result<i64, String> {
    state
        .get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| "synthetic coordinator state field missing".to_owned())
}

async fn sample_database_time(observer: &PgPool) -> Result<DateTime<Utc>, String> {
    sqlx::query_scalar("SELECT pg_catalog.clock_timestamp()")
        .fetch_one(observer)
        .await
        .map_err(|_| "could not sample the disposable database clock".to_owned())
}

fn require_database_bracket(
    observed: DateTime<Utc>,
    lower: DateTime<Utc>,
    upper: DateTime<Utc>,
    label: &str,
) -> Result<(), String> {
    if observed < lower {
        return Err(format!(
            "{label} negative timing evidence started before its lower bound"
        ));
    }
    if observed >= upper {
        return Err(format!(
            "{label} negative timing evidence missed its upper bound"
        ));
    }
    Ok(())
}

async fn wait_until_database_time_bounded(
    observer: &PgPool,
    target: DateTime<Utc>,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let reached: bool =
            sqlx::query_scalar("SELECT pg_catalog.clock_timestamp() >= $1::timestamptz")
                .bind(target)
                .fetch_one(observer)
                .await
                .map_err(|_| "could not observe the disposable database clock".to_owned())?;
        if reached {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("timed out waiting for the disposable database clock".to_owned());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn wait_for_dispatch_spacing(
    harness: &ClientHarness,
    minimum: Duration,
    timeout: Duration,
) -> Result<(), String> {
    let last_dispatch = harness
        .transport
        .requests()
        .last()
        .map(|request| request.dispatched_at)
        .ok_or_else(|| "expected a prior synthetic dispatch".to_owned())?;
    let target = last_dispatch + minimum + Duration::from_millis(100);
    let deadline = Instant::now() + timeout;
    loop {
        let now = Instant::now();
        if now >= target {
            return Ok(());
        }
        if now >= deadline {
            return Err("timed out waiting for the observed dispatch spacing".to_owned());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn wait_for_failure_persistence(
    db: &IntradayTestDb,
    fixture: &MembershipFixture,
    failure: IntradayQuoteFailureCode,
    timeout: Duration,
) -> Result<IntradayCacheRecord, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(cache) = db
            .repository_as_app()
            .read_current_cache(
                fixture.owner_user_id,
                fixture.membership_id,
                fixture.generation,
                &db.session_proof(),
            )
            .await
            .map_err(|_| "could not observe failure persistence".to_owned())?
            && cache.last_failure_code == Some(failure)
            && cache.last_failure_at.is_some()
        {
            return Ok(cache);
        }
        if Instant::now() >= deadline {
            return Err("timed out observing persisted intraday failure".to_owned());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn wait_for_cooldown_expiry(
    harness: &ClientHarness,
    timeout: Duration,
) -> Result<i64, String> {
    let deadline = Instant::now() + timeout;
    loop {
        let state = harness.state();
        let cooldown_until = state_i64(&state, "broker_cooldown_until_ms")?;
        if SystemClock.now_ms() >= cooldown_until {
            return Ok(cooldown_until);
        }
        if Instant::now() >= deadline {
            return Err("timed out waiting for persisted broker cooldown".to_owned());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn renewal_sequence(
    pool: &PgPool,
    owner_user_id: Uuid,
    consumer_id: Uuid,
) -> Result<u64, String> {
    let sequence: i64 = sqlx::query_scalar(
        "SELECT renewal_sequence
           FROM public.owner_intraday_quote_demands
          WHERE owner_user_id = $1 AND consumer_id = $2",
    )
    .bind(owner_user_id)
    .bind(consumer_id)
    .fetch_one(pool)
    .await
    .map_err(|_| "could not inspect synthetic demand renewal sequence".to_owned())?;
    u64::try_from(sequence).map_err(|_| "synthetic demand renewal sequence was negative".to_owned())
}

#[tokio::test]
async fn dropped_demand_renewal_guard_aborts_its_owned_task() {
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let dropped = Arc::new(AtomicBool::new(false));
    let task_dropped = Arc::clone(&dropped);
    let join = tokio::spawn(async move {
        let _drop_signal = TaskDropSignal(task_dropped);
        started_tx
            .send(())
            .expect("renewal guard handshake receiver is alive");
        std::future::pending::<()>().await;
        Ok(())
    });
    let (stop_tx, stop_rx) = watch::channel(false);
    let guard = DemandRenewalTask {
        stop_tx,
        join: Some(join),
    };
    started_rx
        .await
        .expect("renewal task reached deterministic handshake");
    drop(guard);
    assert!(*stop_rx.borrow());
    tokio::time::timeout(Duration::from_secs(1), async {
        while !dropped.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("dropped renewal guard did not abort its owned task");
}

#[tokio::test]
async fn one_identity_dispatches_again_on_the_next_due_cycle() {
    run_body(|mut db| async move {
        let owner = db.seed_owner("c2a-one-identity").await?;
        let fixture = db.seed_ready_membership(owner, "005940.KRX").await?;
        let _demand = add_demand(&db, fixture.clone(), "c2a-one-demand").await?;
        let windows = install_current_window_contract(&mut db, 20).await?;
        let harness = ClientHarness::new(
            "c2a-one-identity",
            [
                TransportStep::Response(valid_quote_response("005940")),
                TransportStep::Response(valid_quote_response("005940")),
            ],
        );
        let config = IntradayProducerConfig::for_worker("b2b-c2a-one-identity")
            .map_err(|_| "synthetic producer configuration failed".to_owned())?;
        let producer = IntradayProducer::new(
            db.repository_as_worker(),
            harness.client.clone(),
            windows,
            config,
        );

        let first = run_cycle(&producer).await?;
        assert_eq!(first.attempts_started, 1);
        assert_eq!(first.successful_quotes, 1);
        let first_cache = read_cache(&db, &fixture).await?;
        let pre_due_at = first_cache.last_attempt_at + chrono::Duration::seconds(2);
        let due_at = first_cache.last_attempt_at + chrono::Duration::seconds(5);
        wait_until_database_time_bounded(&db.superuser, pre_due_at, Duration::from_secs(4)).await?;
        let early_before = sample_database_time(&db.superuser).await?;
        require_database_bracket(early_before, pre_due_at, due_at, "ordinary")?;
        let requests_before_early = harness.transport.request_count();
        let ledger_before_early = reservation_ledger(&harness);

        let early = run_cycle(&producer).await?;
        let early_after = sample_database_time(&db.superuser).await?;
        require_database_bracket(early_after, pre_due_at, due_at, "ordinary")?;
        assert_eq!(early.attempts_started, 0);
        assert_eq!(early.successful_quotes, 0);
        assert_eq!(early.failures_recorded, 0);
        assert_eq!(harness.transport.request_count(), requests_before_early);
        assert_eq!(read_cache(&db, &fixture).await?, first_cache);
        assert_eq!(reservation_ledger(&harness), ledger_before_early);

        wait_until_database_time_bounded(&db.superuser, due_at, Duration::from_secs(4)).await?;
        wait_for_dispatch_spacing(&harness, Duration::from_secs(5), Duration::from_secs(7)).await?;

        let second = run_cycle(&producer).await?;
        assert_eq!(second.attempts_started, 1);
        assert_eq!(second.successful_quotes, 1);
        assert_request_sequence(&harness, &["005940", "005940"]);
        assert_minimum_dispatch_spacing(&harness, Duration::from_secs(5));
        let requests = harness.transport.requests();
        assert!(
            requests[1]
                .dispatched_at
                .duration_since(requests[0].dispatched_at)
                >= Duration::from_secs(5)
        );
        let cache = read_cache(&db, &fixture).await?;
        assert_eq!(cache.quote_version, 2);
        assert_eq!(cache.halted, Some(false));
        assert_eq!(cache.producer_fence, 1);
        assert_ledger(&harness, db.session_date, 2, 3);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn five_identities_run_two_sorted_rounds_and_duplicate_consumer_adds_no_turn() {
    run_body(|mut db| async move {
        let owner = db.seed_owner("c2a-five-identities").await?;
        let symbols = ["005941", "005942", "005943", "005944", "005945"];
        let mut fixtures = Vec::new();
        let mut demands = Vec::new();
        for (index, symbol) in symbols.iter().enumerate() {
            let fixture = db
                .seed_ready_membership(owner, &format!("{symbol}.KRX"))
                .await?;
            let demand = add_demand(&db, fixture.clone(), &format!("c2a-five-{index}-a")).await?;
            demands.push(demand.clone());
            if index == 0 {
                demands.push(add_demand(&db, fixture.clone(), "c2a-five-duplicate").await?);
            }
            fixtures.push(fixture);
        }
        let work = db
            .repository_as_worker()
            .active_quote_work(owner)
            .await
            .map_err(|_| "could not enumerate synthetic quote work".to_owned())?;
        assert_eq!(work.len(), 5);

        let expected = [
            "005941", "005942", "005943", "005944", "005945", "005941", "005942", "005943",
            "005944", "005945",
        ];
        let steps = expected
            .iter()
            .map(|symbol| TransportStep::Response(valid_quote_response(symbol)));
        let windows = install_current_window_contract(&mut db, 75).await?;
        let harness = ClientHarness::new("c2a-five-identities", steps);
        let config = IntradayProducerConfig::for_worker("b2b-c2a-five-identities")
            .map_err(|_| "synthetic producer configuration failed".to_owned())?;
        let producer = Arc::new(IntradayProducer::new(
            db.repository_as_worker(),
            harness.client.clone(),
            windows,
            config,
        ));
        let renewals = start_demand_renewals(&db, demands);

        let result = tokio::time::timeout(Duration::from_secs(70), async {
            for (index, symbol) in expected.iter().enumerate() {
                if index > 0 {
                    wait_for_dispatch_spacing(
                        &harness,
                        Duration::from_secs(5),
                        Duration::from_secs(7),
                    )
                    .await?;
                }
                let report = run_cycle(producer.as_ref()).await?;
                assert_eq!(report.attempts_started, 1);
                assert_eq!(report.successful_quotes, 1);
                assert_eq!(harness.transport.request_count(), index + 1);
                let request = harness
                    .transport
                    .requests()
                    .pop()
                    .ok_or_else(|| "expected scheduled synthetic GET".to_owned())?;
                assert_exact_get(&request, symbol);
            }
            let expected_refs = expected.to_vec();
            assert_request_sequence(&harness, &expected_refs);
            assert_minimum_dispatch_spacing(&harness, Duration::from_secs(5));
            for fixture in &fixtures {
                let cache = read_cache(&db, fixture).await?;
                assert_eq!(cache.quote_version, 2);
                assert_eq!(cache.halted, Some(false));
                assert_eq!(cache.producer_fence, 1);
            }
            assert_ledger(&harness, db.session_date, 10, 11);
            Ok::<(), String>(())
        })
        .await
        .map_err(|_| "five-identity scheduling exceeded its bounded test timeout".to_owned());
        renewals.stop().await?;
        result??;
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn persisted_nine_second_retry_after_blocks_a_new_cycle_until_expiry() {
    run_body(|mut db| async move {
        let owner = db.seed_owner("c2a-retry-after").await?;
        let fixture = db.seed_ready_membership(owner, "005946.KRX").await?;
        let _demand = add_demand(&db, fixture.clone(), "c2a-retry-after-demand").await?;
        let windows = install_current_window_contract(&mut db, 30).await?;
        let harness = ClientHarness::new(
            "c2a-retry-after",
            [
                TransportStep::Response(rate_limited_response(9)),
                TransportStep::Response(valid_quote_response("005946")),
            ],
        );
        let config = IntradayProducerConfig::for_worker("b2b-c2a-retry-after")
            .map_err(|_| "synthetic producer configuration failed".to_owned())?;
        let producer = Arc::new(IntradayProducer::new(
            db.repository_as_worker(),
            harness.client.clone(),
            windows,
            config,
        ));

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let running_producer = Arc::clone(&producer);
        let first_cycle = tokio::spawn(async move {
            let mut shutdown = shutdown_rx;
            running_producer
                .run_cycle(&mut shutdown)
                .await
                .map_err(|_| "first retry-after cycle failed".to_owned())
        });
        let persisted = wait_for_failure_persistence(
            &db,
            &fixture,
            IntradayQuoteFailureCode::ProviderRateLimited,
            Duration::from_secs(5),
        )
        .await;
        let cancellation_observed = shutdown_tx.send(true).is_ok();
        let first = first_cycle
            .await
            .map_err(|_| "first retry-after cycle task failed".to_owned())??;
        if !cancellation_observed {
            return Err(
                "retry-after cycle ended before persistence cancellation barrier".to_owned(),
            );
        }
        let persisted = persisted?;
        assert_eq!(first.attempts_started, 1);
        assert_eq!(first.successful_quotes, 0);
        assert_eq!(first.failures_recorded, 1);
        assert_request_sequence(&harness, &["005946"]);
        assert_failure_without_success(
            &persisted,
            IntradayQuoteFailureCode::ProviderRateLimited,
            1,
        );

        let cooldown_before_state = harness.state();
        let cooldown_before = state_i64(&cooldown_before_state, "broker_cooldown_until_ms")?;
        assert!(cooldown_before > SystemClock.now_ms());
        wait_until_database_time_bounded(
            &db.superuser,
            persisted.last_attempt_at + chrono::Duration::seconds(5),
            Duration::from_secs(6),
        )
        .await?;
        assert!(SystemClock.now_ms() < cooldown_before);

        let blocked_cycle = run_cycle(producer.as_ref()).await?;
        assert_eq!(blocked_cycle.attempts_started, 1);
        assert_eq!(blocked_cycle.successful_quotes, 0);
        assert_eq!(blocked_cycle.failures_recorded, 0);
        assert_eq!(harness.transport.request_count(), 1);
        assert_eq!(harness.issuer.calls(), 1);
        let after_blocked_cache = read_cache(&db, &fixture).await?;
        assert_eq!(
            after_blocked_cache.last_attempt_at,
            persisted.last_attempt_at
        );
        assert_eq!(
            after_blocked_cache.last_failure_code,
            persisted.last_failure_code
        );
        let cooldown_after_state = harness.state();
        assert_eq!(
            state_i64(&cooldown_after_state, "broker_cooldown_until_ms")?,
            cooldown_before
        );

        wait_for_cooldown_expiry(&harness, Duration::from_secs(5)).await?;
        let successful_cycle = run_cycle(producer.as_ref()).await?;
        assert_eq!(successful_cycle.attempts_started, 1);
        assert_eq!(successful_cycle.successful_quotes, 1);
        assert_eq!(successful_cycle.failures_recorded, 0);
        assert_request_sequence(&harness, &["005946", "005946"]);
        assert_minimum_dispatch_spacing(&harness, Duration::from_secs(5));
        let requests = harness.transport.requests();
        assert!(
            requests[1]
                .dispatched_at
                .duration_since(requests[0].dispatched_at)
                >= Duration::from_secs(9)
        );
        let cache = read_cache(&db, &fixture).await?;
        assert_eq!(cache.quote_version, 1);
        assert_eq!(cache.halted, Some(false));
        assert!(cache.received_at.is_some());
        assert!(cache.last_failure_code.is_none());
        assert_ledger(&harness, db.session_date, 2, 3);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn halted_identity_waits_sixty_seconds_then_reprobes_after_real_demand_renewals() {
    run_body(|mut db| async move {
        let owner = db.seed_owner("c2a-halt").await?;
        let fixture = db.seed_ready_membership(owner, "005947.KRX").await?;
        let demand = add_demand(&db, fixture.clone(), "c2a-halt-demand").await?;
        let windows = install_current_window_contract(&mut db, 75).await?;
        let harness = ClientHarness::new(
            "c2a-halt",
            [
                TransportStep::Response(halted_quote_response("005947")),
                TransportStep::Response(valid_quote_response("005947")),
            ],
        );
        let config = IntradayProducerConfig::for_worker("b2b-c2a-halt")
            .map_err(|_| "synthetic producer configuration failed".to_owned())?;
        let producer = Arc::new(IntradayProducer::new(
            db.repository_as_worker(),
            harness.client.clone(),
            windows,
            config,
        ));
        let renewals = start_demand_renewals(&db, vec![demand.clone()]);

        let result = tokio::time::timeout(Duration::from_secs(75), async {
            let first = run_cycle(producer.as_ref()).await?;
            assert_eq!(first.attempts_started, 1);
            assert_eq!(first.successful_quotes, 1);
            assert_request_sequence(&harness, &["005947"]);
            let halted = read_cache(&db, &fixture).await?;
            assert_eq!(halted.halted, Some(true));
            assert_eq!(halted.quote_version, 1);
            let reprobe_at = halted.last_attempt_at + chrono::Duration::seconds(60);

            let before_reprobe = run_cycle(producer.as_ref()).await?;
            assert_eq!(before_reprobe.attempts_started, 0);
            assert_eq!(before_reprobe.successful_quotes, 0);
            assert_eq!(harness.transport.request_count(), 1);
            let negative_at = halted.last_attempt_at + chrono::Duration::seconds(7);
            let requests_before_negative = harness.transport.request_count();
            let ledger_before_negative = reservation_ledger(&harness);
            wait_until_database_time_bounded(&db.superuser, negative_at, Duration::from_secs(9))
                .await?;
            let negative_before = sample_database_time(&db.superuser).await?;
            require_database_bracket(negative_before, negative_at, reprobe_at, "halt")?;

            let early = run_cycle(producer.as_ref()).await?;
            let negative_after = sample_database_time(&db.superuser).await?;
            require_database_bracket(negative_after, negative_at, reprobe_at, "halt")?;
            assert_eq!(early.attempts_started, 0);
            assert_eq!(early.successful_quotes, 0);
            assert_eq!(early.failures_recorded, 0);
            assert_eq!(harness.transport.request_count(), requests_before_negative);
            assert_eq!(read_cache(&db, &fixture).await?, halted);
            assert_eq!(reservation_ledger(&harness), ledger_before_negative);

            wait_until_database_time_bounded(&db.superuser, reprobe_at, Duration::from_secs(68))
                .await?;
            wait_for_dispatch_spacing(&harness, Duration::from_secs(60), Duration::from_secs(68))
                .await?;
            let second = run_cycle(producer.as_ref()).await?;
            assert_eq!(second.attempts_started, 1);
            assert_eq!(second.successful_quotes, 1);
            assert_request_sequence(&harness, &["005947", "005947"]);
            assert_minimum_dispatch_spacing(&harness, Duration::from_secs(5));
            let requests = harness.transport.requests();
            assert!(
                requests[1]
                    .dispatched_at
                    .duration_since(requests[0].dispatched_at)
                    >= Duration::from_secs(60)
            );
            let unhalted = read_cache(&db, &fixture).await?;
            assert_eq!(unhalted.halted, Some(false));
            assert_eq!(unhalted.quote_version, 2);
            assert!(unhalted.received_at.is_some());
            assert!(unhalted.last_failure_code.is_none());
            assert_eq!(unhalted.producer_fence, 2);
            assert_ledger(&harness, db.session_date, 2, 3);
            Ok::<(), String>(())
        })
        .await
        .map_err(|_| "halt scheduling exceeded its bounded test timeout".to_owned());
        renewals.stop().await?;
        result??;
        let sequence =
            renewal_sequence(&db.superuser, fixture.owner_user_id, demand.consumer_id).await?;
        assert!(sequence >= 3);
        Ok(())
    })
    .await;
}
