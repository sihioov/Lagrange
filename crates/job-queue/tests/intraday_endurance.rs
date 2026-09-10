#[allow(dead_code)]
mod intraday_producer_pipeline_support;
#[allow(dead_code)]
mod intraday_quotes_support;

use std::fs;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use collectors::intraday_quotes::{INTRADAY_QUOTE_PATH, INTRADAY_QUOTE_TR_ID};
use intraday_producer_pipeline_support::{
    ClientHarness, PipelineClient, RequestRecord, TransportStep, install_current_window_contract,
    publication_counts, valid_quote_response,
};
use intraday_quotes_support::{IntradayTestDb, MembershipFixture, run_body};
use job_queue::owner_equity_v2::{
    DemandMutationKind, DemandReleaseKind, IntradayProducer, IntradayProducerConfig,
    IntradayProducerCycleReport, IntradayQuoteDemandLease, IntradayQuoteDemandRequest,
    IntradayQuoteReleaseRequest,
};
use serde_json::Value;
use sqlx::PgPool;
use tokio::sync::watch;
use uuid::Uuid;

const OBSERVED_WINDOW: Duration = Duration::from_secs(1_800);
const OBSERVED_WINDOW_SECONDS: i64 = 1_800;
const WARMUP: Duration = Duration::from_secs(60);
const CYCLE_INTERVAL: Duration = Duration::from_secs(1);
const RENEWAL_INTERVAL: Duration = Duration::from_secs(10);
const RSS_SAMPLE_INTERVAL: Duration = Duration::from_secs(10);
const RSS_MAX_SAMPLE_GAP: Duration = Duration::from_secs(20);
const MIN_RSS_SAMPLES: usize = 90;
const HALF_WINDOW: Duration = Duration::from_secs(900);
const TAIL_GUARD: Duration = Duration::from_secs(5);
const SESSION_WINDOW_MARGIN_SECONDS: i64 = 120;
const CONSUMER_COUNT: usize = 20;
const MAX_SCRIPTED_STEPS: usize = 400;
const MAX_ATTEMPTS: usize = 361;
const MIN_ATTEMPTS: usize = 300;
const MIN_CYCLES: usize = 1_500;
const MIN_RENEWAL_BATCHES: usize = 170;
const MAX_RENEWAL_BATCHES: usize =
    (OBSERVED_WINDOW_SECONDS as usize / RENEWAL_INTERVAL.as_secs() as usize) + 1;
const MAX_RSS_GROWTH_KIB: u64 = 32 * 1024;
const MIN_SYNTHETIC_TOKEN_TTL_MS: i64 = 60 * 60 * 1_000;
const MAX_SYNTHETIC_TOKEN_TTL_MS: i64 = 48 * 60 * 60 * 1_000;

// Exact opt-in invocation (real 1,800-second wall-time window; no duration override):
// DATABASE_URL=postgres://postgres:lagrange@127.0.0.1:55438/postgres CARGO_BUILD_JOBS=2 cargo test -p job-queue --locked --offline --test intraday_endurance -- --ignored --exact intraday_endurance_real_30_minute_provider_free_soak --nocapture --test-threads=1

#[derive(Clone)]
struct DemandHandle {
    consumer_id: Uuid,
    demand_id: Uuid,
    renewal_sequence: u64,
}

#[derive(Debug, Clone, Copy)]
struct RowBounds {
    demand_rows: i64,
    cache_rows: i64,
    producer_rows: i64,
    active_leases: i64,
    released_demands: i64,
}

#[derive(Debug, Clone, Copy)]
struct RssObservation {
    baseline_kib: u64,
    max_kib: u64,
    final_kib: u64,
    samples: usize,
}

#[derive(Default)]
struct SampledHalves {
    first: bool,
    second: bool,
}

fn demand_request(
    membership_id: Uuid,
    consumer_id: Uuid,
    sequence: u64,
    key: String,
) -> Result<IntradayQuoteDemandRequest, String> {
    IntradayQuoteDemandRequest::new(consumer_id, membership_id, 1, sequence, key)
        .map_err(|_| "synthetic demand request construction failed".to_owned())
}

fn release_request(
    consumer_id: Uuid,
    sequence: u64,
    key: String,
) -> Result<IntradayQuoteReleaseRequest, String> {
    IntradayQuoteReleaseRequest::new(consumer_id, sequence, key)
        .map_err(|_| "synthetic release request construction failed".to_owned())
}

fn lease_matches(lease: &IntradayQuoteDemandLease, fixture: &MembershipFixture) -> bool {
    lease.owner_user_id == fixture.owner_user_id
        && lease.membership_id == fixture.membership_id
        && lease.generation_id == fixture.generation_id
        && lease.instrument_id == fixture.instrument_id
        && lease.generation == fixture.generation
}

async fn assert_app_role(db: &IntradayTestDb) -> Result<(), String> {
    let current_user: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&db.app)
        .await
        .map_err(|_| "could not inspect the synthetic app-role connection".to_owned())?;
    if current_user != "app" {
        return Err("demand mutations did not use the synthetic app role".to_owned());
    }
    Ok(())
}

async fn seed_demands(
    db: &IntradayTestDb,
    fixture: &MembershipFixture,
) -> Result<Vec<DemandHandle>, String> {
    let app = db.repository_as_app();
    let mut demands = Vec::with_capacity(CONSUMER_COUNT);
    for index in 0..CONSUMER_COUNT {
        let consumer_id = Uuid::new_v4();
        if demands
            .iter()
            .any(|demand: &DemandHandle| demand.consumer_id == consumer_id)
        {
            return Err("synthetic consumer UUIDs were not distinct".to_owned());
        }
        let request = demand_request(
            fixture.membership_id,
            consumer_id,
            0,
            format!("intraday-endurance-create-{index}"),
        )?;
        let outcome = app
            .create_or_renew_demand_current(fixture.owner_user_id, &request)
            .await
            .map_err(|_| "synthetic app demand creation failed".to_owned())?;
        if outcome.kind != DemandMutationKind::Created
            || !lease_matches(&outcome.lease, fixture)
            || outcome.lease.consumer_id != consumer_id
            || outcome.lease.renewal_sequence != 0
        {
            return Err("synthetic demand creation returned an unexpected lease".to_owned());
        }
        demands.push(DemandHandle {
            consumer_id,
            demand_id: outcome.lease.demand_id,
            renewal_sequence: 0,
        });
    }
    Ok(demands)
}

async fn renew_demands(
    db: &IntradayTestDb,
    fixture: &MembershipFixture,
    demands: &mut [DemandHandle],
    batch_index: usize,
) -> Result<usize, String> {
    let app = db.repository_as_app();
    let mut expiry_times = Vec::with_capacity(demands.len());
    for (index, demand) in demands.iter_mut().enumerate() {
        let sequence = demand
            .renewal_sequence
            .checked_add(1)
            .ok_or_else(|| "synthetic demand renewal sequence overflowed".to_owned())?;
        let request = demand_request(
            fixture.membership_id,
            demand.consumer_id,
            sequence,
            format!("intraday-endurance-renew-{batch_index}-{index}-{sequence}"),
        )?;
        let outcome = app
            .create_or_renew_demand_current(fixture.owner_user_id, &request)
            .await
            .map_err(|_| "synthetic app demand renewal failed".to_owned())?;
        if outcome.kind != DemandMutationKind::Renewed
            || !lease_matches(&outcome.lease, fixture)
            || outcome.lease.consumer_id != demand.consumer_id
            || outcome.lease.demand_id != demand.demand_id
            || outcome.lease.renewal_sequence != sequence
        {
            return Err("synthetic demand renewal returned an unexpected lease".to_owned());
        }
        expiry_times.push(outcome.lease.lease_expires_at);
        demand.renewal_sequence = sequence;
    }

    let database_now = app
        .current_database_time()
        .await
        .map_err(|_| "could not inspect the synthetic database clock after renewal".to_owned())?;
    if expiry_times
        .iter()
        .any(|lease_expires_at| *lease_expires_at <= database_now)
    {
        return Err("a demand lease was not active after its ten-second renewal batch".to_owned());
    }
    let bounds = read_row_bounds(&db.superuser, fixture.owner_user_id).await?;
    if bounds.active_leases != CONSUMER_COUNT as i64 {
        return Err("renewal did not keep all twenty demand leases active".to_owned());
    }
    Ok(demands.len())
}

async fn release_demands(
    db: &IntradayTestDb,
    fixture: &MembershipFixture,
    demands: &[DemandHandle],
) -> Result<(), String> {
    let app = db.repository_as_app();
    for (index, demand) in demands.iter().enumerate() {
        let request = release_request(
            demand.consumer_id,
            demand.renewal_sequence,
            format!(
                "intraday-endurance-release-{index}-{}",
                demand.renewal_sequence
            ),
        )?;
        let outcome = app
            .release_demand_current(fixture.owner_user_id, demand.demand_id, &request)
            .await
            .map_err(|_| "synthetic app demand release failed".to_owned())?;
        if outcome.kind != DemandReleaseKind::Released || outcome.demand_id != demand.demand_id {
            return Err("synthetic demand release returned an unexpected outcome".to_owned());
        }
    }
    Ok(())
}

async fn read_row_bounds(pool: &PgPool, owner_user_id: Uuid) -> Result<RowBounds, String> {
    let demand_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.owner_intraday_quote_demands
          WHERE owner_user_id = $1",
    )
    .bind(owner_user_id)
    .fetch_one(pool)
    .await
    .map_err(|_| "could not count synthetic demand rows".to_owned())?;
    let cache_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.owner_intraday_quote_cache
          WHERE owner_user_id = $1",
    )
    .bind(owner_user_id)
    .fetch_one(pool)
    .await
    .map_err(|_| "could not count synthetic cache rows".to_owned())?;
    let producer_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.owner_intraday_quote_producers
          WHERE owner_user_id = $1",
    )
    .bind(owner_user_id)
    .fetch_one(pool)
    .await
    .map_err(|_| "could not count synthetic producer rows".to_owned())?;
    let active_leases: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.owner_intraday_quote_demands
          WHERE owner_user_id = $1
            AND state = 'ACTIVE'
            AND lease_expires_at > pg_catalog.clock_timestamp()",
    )
    .bind(owner_user_id)
    .fetch_one(pool)
    .await
    .map_err(|_| "could not count active synthetic demand leases".to_owned())?;
    let released_demands: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.owner_intraday_quote_demands
          WHERE owner_user_id = $1 AND state = 'RELEASED'",
    )
    .bind(owner_user_id)
    .fetch_one(pool)
    .await
    .map_err(|_| "could not count released synthetic demand rows".to_owned())?;
    Ok(RowBounds {
        demand_rows,
        cache_rows,
        producer_rows,
        active_leases,
        released_demands,
    })
}

fn check_row_bounds(bounds: RowBounds, require_active: bool) -> Result<(), String> {
    if bounds.demand_rows > CONSUMER_COUNT as i64 {
        return Err("synthetic demand row bound exceeded twenty rows".to_owned());
    }
    if bounds.cache_rows > 1 {
        return Err("synthetic cache row bound exceeded one row".to_owned());
    }
    if bounds.producer_rows > 1 {
        return Err("synthetic producer row bound exceeded one row".to_owned());
    }
    if bounds.active_leases > CONSUMER_COUNT as i64 {
        return Err("synthetic active demand row bound exceeded twenty rows".to_owned());
    }
    if require_active && bounds.active_leases != CONSUMER_COUNT as i64 {
        return Err("synthetic active demand leases were not continuously present".to_owned());
    }
    Ok(())
}

fn process_rss_kib() -> Result<u64, String> {
    let status = fs::read_to_string("/proc/self/status").map_err(|_| {
        "RSS observation unavailable: /proc/self/status could not be read".to_owned()
    })?;
    let line = status
        .lines()
        .find(|line| line.starts_with("VmRSS:"))
        .ok_or_else(|| "RSS observation unavailable: VmRSS was not present".to_owned())?;
    let mut fields = line.split_whitespace();
    if fields.next() != Some("VmRSS:") {
        return Err("RSS observation unavailable: VmRSS label was malformed".to_owned());
    }
    let value = fields
        .next()
        .ok_or_else(|| "RSS observation unavailable: VmRSS value was missing".to_owned())?
        .parse::<u64>()
        .map_err(|_| "RSS observation unavailable: VmRSS value was not numeric".to_owned())?;
    if fields.next() != Some("kB") {
        return Err("RSS observation unavailable: VmRSS unit was not kB".to_owned());
    }
    Ok(value)
}

fn take_rss_sample(
    measurement_started: Instant,
    previous_sample_at: &mut Option<Instant>,
    baseline_rss_kib: &mut Option<u64>,
    baseline_elapsed: &mut Option<Duration>,
    max_rss_kib: &mut Option<u64>,
    sample_count: &mut usize,
    sampled_halves: &mut SampledHalves,
) -> Result<u64, String> {
    let sampled_at = Instant::now();
    if let Some(previous) = *previous_sample_at
        && sampled_at.duration_since(previous) > RSS_MAX_SAMPLE_GAP
    {
        return Err("RSS sampling gap exceeded twenty seconds".to_owned());
    }
    let rss_kib = process_rss_kib()?;
    let elapsed = sampled_at.duration_since(measurement_started);
    *sample_count = sample_count
        .checked_add(1)
        .ok_or_else(|| "RSS sample counter overflowed".to_owned())?;
    if elapsed <= HALF_WINDOW {
        sampled_halves.first = true;
    } else {
        sampled_halves.second = true;
    }
    if baseline_rss_kib.is_none() && elapsed >= WARMUP {
        *baseline_rss_kib = Some(rss_kib);
        *baseline_elapsed = Some(elapsed);
        *max_rss_kib = Some(rss_kib);
    }
    if let Some(maximum) = max_rss_kib.as_mut() {
        *maximum = (*maximum).max(rss_kib);
    }
    *previous_sample_at = Some(sampled_at);
    Ok(rss_kib)
}

fn verify_exact_get(record: &RequestRecord, symbol: &str) -> Result<(), String> {
    if record.method != "GET"
        || record.path != INTRADAY_QUOTE_PATH
        || record.tr_id != INTRADAY_QUOTE_TR_ID
    {
        return Err("synthetic transport observed a non-allowlisted quote request".to_owned());
    }
    if record.query
        != vec![
            ("FID_COND_MRKT_DIV_CODE".to_owned(), "J".to_owned()),
            ("FID_INPUT_ISCD".to_owned(), symbol.to_owned()),
        ]
    {
        return Err("synthetic quote request query was not the exact J six-digit shape".to_owned());
    }
    if record.headers.get("custtype").map(String::as_str) != Some("P")
        || record.headers.contains_key("tr_cont")
    {
        return Err("synthetic quote request headers widened the approved shape".to_owned());
    }
    Ok(())
}

fn measured_requests(
    harness: &ClientHarness,
    measurement_started: Instant,
    symbol: &str,
) -> Result<Vec<RequestRecord>, String> {
    let requests = harness.transport.requests();
    if requests.len() > MAX_SCRIPTED_STEPS {
        return Err("synthetic transport request log exceeded four hundred steps".to_owned());
    }
    if requests.is_empty() {
        return Err("synthetic producer made no quote attempt".to_owned());
    }
    for (index, request) in requests.iter().enumerate() {
        if request.dispatched_at < measurement_started {
            return Err(
                "synthetic quote dispatch occurred before the measured boundary".to_owned(),
            );
        }
        if request.dispatched_at.duration_since(measurement_started) >= OBSERVED_WINDOW {
            return Err("synthetic quote dispatch crossed the 1,800-second boundary".to_owned());
        }
        verify_exact_get(request, symbol)?;
        if index == 0 && request.dispatched_at.duration_since(measurement_started) > TAIL_GUARD {
            return Err(
                "first synthetic quote dispatch missed the measurement boundary".to_owned(),
            );
        }
    }
    Ok(requests)
}

fn verify_dispatch_spacing(requests: &[RequestRecord]) -> Result<(), String> {
    for pair in requests.windows(2) {
        if pair[1].dispatched_at.duration_since(pair[0].dispatched_at) < Duration::from_secs(5) {
            return Err(
                "synthetic quote dispatches violated the five-second shared spacing".to_owned(),
            );
        }
    }
    Ok(())
}

fn unix_now_ms() -> Result<i64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "synthetic token TTL could not read the process wall clock".to_owned())?;
    i64::try_from(duration.as_millis())
        .map_err(|_| "synthetic token TTL process wall clock overflowed".to_owned())
}

fn verify_coordination_state(
    harness: &ClientHarness,
    expected_attempts: usize,
    session_date: &str,
) -> Result<(), String> {
    if harness.issuer.calls() != 1 {
        return Err("synthetic issuer issued more than one token".to_owned());
    }
    let state = harness
        .state_if_present()
        .ok_or_else(|| "synthetic coordination state was not persisted".to_owned())?;
    let token = state
        .get("token")
        .filter(|token| !token.is_null())
        .ok_or_else(|| "synthetic coordination token was not retained".to_owned())?;
    let expires_at_ms = token
        .get("expires_at_ms")
        .and_then(Value::as_i64)
        .ok_or_else(|| "synthetic coordination token expiry was not inspectable".to_owned())?;
    let ttl_ms = expires_at_ms.saturating_sub(unix_now_ms()?);
    if !(MIN_SYNTHETIC_TOKEN_TTL_MS..=MAX_SYNTHETIC_TOKEN_TTL_MS).contains(&ttl_ms) {
        return Err(
            "synthetic coordination token TTL was outside the bounded harness policy".to_owned(),
        );
    }
    if state.get("intraday_kst_date").and_then(Value::as_str) != Some(session_date) {
        return Err("synthetic coordination ledger did not retain the session date".to_owned());
    }
    if state.get("intraday_attempts").and_then(Value::as_u64) != Some(expected_attempts as u64) {
        return Err(
            "synthetic coordination attempt ledger disagreed with transport count".to_owned(),
        );
    }
    if state.get("next_fence").and_then(Value::as_u64) != Some(expected_attempts as u64 + 1) {
        return Err(
            "synthetic coordination fence ledger disagreed with transport count".to_owned(),
        );
    }
    if !state.get("in_flight").is_some_and(Value::is_null) {
        return Err("synthetic coordination retained an in-flight attempt".to_owned());
    }
    Ok(())
}

async fn run_cycle(
    producer: &IntradayProducer<PipelineClient>,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<IntradayProducerCycleReport, String> {
    producer
        .run_cycle(shutdown)
        .await
        .map_err(|_| "synthetic intraday producer cycle failed".to_owned())
}

async fn endurance_body(mut db: IntradayTestDb) -> Result<(), String> {
    assert_app_role(&db).await?;
    let owner = db.seed_owner("endurance").await?;
    let fixture = db.seed_ready_membership(owner, "005930.KRX").await?;
    if fixture.instrument_id != "005930.KRX" || fixture.generation != 1 {
        return Err(
            "synthetic endurance membership was not the fixed generation-one identity".to_owned(),
        );
    }
    let mut demands = seed_demands(&db, &fixture).await?;
    let initial_bounds = read_row_bounds(&db.superuser, owner).await?;
    check_row_bounds(initial_bounds, true)?;
    if initial_bounds.demand_rows != CONSUMER_COUNT as i64
        || initial_bounds.active_leases != CONSUMER_COUNT as i64
    {
        return Err("synthetic setup did not create twenty active demand rows".to_owned());
    }

    let database_kst_date: chrono::NaiveDate =
        sqlx::query_scalar("SELECT (pg_catalog.clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date")
            .fetch_one(&db.superuser)
            .await
            .map_err(|_| "could not verify the synthetic database KST date".to_owned())?;
    if database_kst_date != db.session_date {
        return Err(
            "endurance setup crossed a KST date boundary; refusing invalid session proof"
                .to_owned(),
        );
    }

    let windows = install_current_window_contract(
        &mut db,
        OBSERVED_WINDOW_SECONDS + SESSION_WINDOW_MARGIN_SECONDS,
    )
    .await
    .map_err(|_| {
        "endurance setup could not install a same-date window with a 1,800-second plus 120-second margin"
            .to_owned()
    })?;

    let mut steps = Vec::with_capacity(MAX_SCRIPTED_STEPS);
    for _ in 0..MAX_SCRIPTED_STEPS {
        steps.push(TransportStep::Response(valid_quote_response("005930")));
    }
    let harness = ClientHarness::new("endurance", steps);
    let config = IntradayProducerConfig::for_worker("b2b-c1-endurance")
        .map_err(|_| "synthetic endurance producer configuration failed".to_owned())?;
    let producer = IntradayProducer::new(
        db.repository_as_worker(),
        harness.client.clone(),
        windows,
        config,
    );
    let publication_before = publication_counts(&db.superuser).await?;
    let (_shutdown_tx, mut shutdown_rx) = watch::channel(false);

    let measurement_started = Instant::now();
    let measurement_deadline = measurement_started + OBSERVED_WINDOW;
    let mut previous_sample_at = None;
    let mut baseline_rss_kib = None;
    let mut baseline_elapsed = None;
    let mut max_rss_kib = None;
    let mut sample_count = 0usize;
    let mut sampled_halves = SampledHalves::default();
    take_rss_sample(
        measurement_started,
        &mut previous_sample_at,
        &mut baseline_rss_kib,
        &mut baseline_elapsed,
        &mut max_rss_kib,
        &mut sample_count,
        &mut sampled_halves,
    )?;

    let mut next_cycle_at = measurement_started;
    let mut next_renewal_at = measurement_started + RENEWAL_INTERVAL;
    let mut next_sample_at = measurement_started + RSS_SAMPLE_INTERVAL;
    let mut renewal_batches = 0usize;
    let mut renewal_operations = 0usize;
    let mut cycles = 0usize;
    let mut owner_observations = 0usize;
    let mut reported_attempts = 0usize;
    let mut reported_successes = 0usize;
    let mut reported_failures = 0usize;

    println!("intraday endurance observation started: real_seconds=1800 synthetic_consumers=20");

    loop {
        let now = Instant::now();
        if now >= measurement_deadline {
            break;
        }
        if now >= next_renewal_at {
            if now.duration_since(next_renewal_at) > Duration::from_secs(5) {
                return Err(
                    "ten-second demand renewal schedule fell behind its bounded lease cadence"
                        .to_owned(),
                );
            }
            let renewed = renew_demands(&db, &fixture, &mut demands, renewal_batches).await?;
            if renewed != CONSUMER_COUNT {
                return Err("a demand renewal batch did not renew all twenty consumers".to_owned());
            }
            renewal_batches = renewal_batches
                .checked_add(1)
                .ok_or_else(|| "demand renewal batch counter overflowed".to_owned())?;
            renewal_operations = renewal_operations
                .checked_add(renewed)
                .ok_or_else(|| "demand renewal operation counter overflowed".to_owned())?;
            next_renewal_at += RENEWAL_INTERVAL;
            continue;
        }
        if now >= next_sample_at {
            let rss_kib = take_rss_sample(
                measurement_started,
                &mut previous_sample_at,
                &mut baseline_rss_kib,
                &mut baseline_elapsed,
                &mut max_rss_kib,
                &mut sample_count,
                &mut sampled_halves,
            )?;
            let sampled_bounds = read_row_bounds(&db.superuser, owner).await?;
            check_row_bounds(sampled_bounds, true)?;
            let sampled_now = Instant::now();
            if sample_count.is_multiple_of(6) {
                println!(
                    "intraday endurance progress: elapsed_s={} cycles={} attempts={} active_leases={} rss_kib={}",
                    sampled_now.duration_since(measurement_started).as_secs(),
                    cycles,
                    reported_attempts,
                    sampled_bounds.active_leases,
                    rss_kib,
                );
            }
            while next_sample_at <= sampled_now {
                next_sample_at += RSS_SAMPLE_INTERVAL;
            }
            continue;
        }
        let remaining = measurement_deadline.duration_since(now);
        if remaining <= TAIL_GUARD {
            tokio::time::sleep(remaining).await;
            continue;
        }
        if now < next_cycle_at {
            let mut wake_at = next_cycle_at;
            if next_renewal_at < wake_at {
                wake_at = next_renewal_at;
            }
            if next_sample_at < wake_at {
                wake_at = next_sample_at;
            }
            if measurement_deadline < wake_at {
                wake_at = measurement_deadline;
            }
            let sleep_for = wake_at.duration_since(now);
            if !sleep_for.is_zero() {
                tokio::time::sleep(sleep_for).await;
            }
            continue;
        }

        let report = run_cycle(&producer, &mut shutdown_rx).await?;
        if report.owners_seen == 0 {
            return Err(
                "producer observed no active owner during the measured demand soak".to_owned(),
            );
        }
        owner_observations = owner_observations
            .checked_add(report.owners_seen)
            .ok_or_else(|| "owner observation counter overflowed".to_owned())?;
        cycles = cycles
            .checked_add(1)
            .ok_or_else(|| "producer cycle counter overflowed".to_owned())?;
        reported_attempts = reported_attempts
            .checked_add(report.attempts_started)
            .ok_or_else(|| "producer attempt counter overflowed".to_owned())?;
        reported_successes = reported_successes
            .checked_add(report.successful_quotes)
            .ok_or_else(|| "producer success counter overflowed".to_owned())?;
        reported_failures = reported_failures
            .checked_add(report.failures_recorded)
            .ok_or_else(|| "producer failure counter overflowed".to_owned())?;
        next_cycle_at = Instant::now() + CYCLE_INTERVAL;
    }

    let elapsed = measurement_started.elapsed();
    if elapsed < OBSERVED_WINDOW {
        return Err("intraday endurance measurement ended before 1,800 real seconds".to_owned());
    }
    let final_rss_kib = take_rss_sample(
        measurement_started,
        &mut previous_sample_at,
        &mut baseline_rss_kib,
        &mut baseline_elapsed,
        &mut max_rss_kib,
        &mut sample_count,
        &mut sampled_halves,
    )?;

    let baseline_rss_kib = baseline_rss_kib.ok_or_else(|| {
        "RSS warmup baseline was not observed at or after sixty seconds".to_owned()
    })?;
    let baseline_elapsed = baseline_elapsed
        .ok_or_else(|| "RSS warmup baseline timestamp was not observed".to_owned())?;
    let max_rss_kib =
        max_rss_kib.ok_or_else(|| "RSS maximum was not observed after warmup".to_owned())?;
    if baseline_elapsed > WARMUP + RSS_SAMPLE_INTERVAL {
        return Err(
            "RSS warmup baseline was not captured within the bounded warmup sample".to_owned(),
        );
    }
    if !sampled_halves.first || !sampled_halves.second {
        return Err("RSS samples did not cover both halves of the measured window".to_owned());
    }
    if sample_count < MIN_RSS_SAMPLES {
        return Err("RSS observation count was below the bounded cadence minimum".to_owned());
    }
    let rss_observation = RssObservation {
        baseline_kib: baseline_rss_kib,
        max_kib: max_rss_kib,
        final_kib: final_rss_kib,
        samples: sample_count,
    };
    // This is a bounded observed process footprint check, not a mathematical leak proof.
    if rss_observation
        .max_kib
        .saturating_sub(rss_observation.baseline_kib)
        > MAX_RSS_GROWTH_KIB
    {
        return Err("TEST-PROCESS RSS grew by more than 32 MiB after warmup".to_owned());
    }

    if cycles < MIN_CYCLES {
        return Err("producer did not run approximately once per second for the soak".to_owned());
    }
    if owner_observations == 0 {
        return Err("producer owner observation was vacuous".to_owned());
    }
    if !(MIN_RENEWAL_BATCHES..=MAX_RENEWAL_BATCHES).contains(&renewal_batches) {
        return Err(
            "demand renewal cadence was outside the bounded ten-second schedule".to_owned(),
        );
    }
    if renewal_operations != renewal_batches * CONSUMER_COUNT {
        return Err("demand renewal operation count did not match complete batches".to_owned());
    }
    for demand in &demands {
        if demand.renewal_sequence != renewal_batches as u64 {
            return Err("consumer renewal sequences were not monotonic and aligned".to_owned());
        }
    }

    let requests = measured_requests(&harness, measurement_started, "005930")?;
    verify_dispatch_spacing(&requests)?;
    let actual_attempts = requests.len();
    if !(MIN_ATTEMPTS..=MAX_ATTEMPTS).contains(&actual_attempts) {
        return Err("actual synthetic quote attempts were outside the 300..361 bound".to_owned());
    }
    if reported_attempts != actual_attempts {
        return Err("producer attempt report disagreed with actual transport attempts".to_owned());
    }
    if reported_successes != actual_attempts {
        return Err("producer successes did not match actual valid quote responses".to_owned());
    }
    if reported_failures != 0 {
        return Err("valid synthetic quote responses recorded a producer failure".to_owned());
    }
    verify_coordination_state(&harness, actual_attempts, &db.session_date.to_string())?;

    let cache = db
        .repository_as_app()
        .read_current_cache(
            owner,
            fixture.membership_id,
            fixture.generation,
            &db.session_proof(),
        )
        .await
        .map_err(|_| "could not read the final synthetic quote cache".to_owned())?
        .ok_or_else(|| "valid synthetic quote responses did not publish a cache row".to_owned())?;
    if cache.quote_version != actual_attempts as u64
        || cache.price.as_deref() != Some("72500.00000000")
        || cache.last_success_at.is_none()
        || cache.last_failure_code.is_some()
    {
        return Err(
            "final synthetic cache did not contain the expected valid quote state".to_owned(),
        );
    }

    let active_bounds = read_row_bounds(&db.superuser, owner).await?;
    check_row_bounds(active_bounds, true)?;
    if active_bounds.demand_rows != CONSUMER_COUNT as i64
        || active_bounds.cache_rows != 1
        || active_bounds.producer_rows != 1
    {
        return Err("final active row bounds did not match the one-membership soak".to_owned());
    }

    let requests_before_release = harness.transport.request_count();
    release_demands(&db, &fixture, &demands).await?;
    let released_bounds = read_row_bounds(&db.superuser, owner).await?;
    check_row_bounds(released_bounds, false)?;
    if released_bounds.demand_rows != CONSUMER_COUNT as i64
        || released_bounds.released_demands != CONSUMER_COUNT as i64
        || released_bounds.active_leases != 0
        || released_bounds.cache_rows != 1
        || released_bounds.producer_rows != 1
    {
        return Err("released demand cleanup left unexpected synthetic row bounds".to_owned());
    }

    for _ in 0..3 {
        let report = run_cycle(&producer, &mut shutdown_rx).await?;
        if report.owners_seen != 0 || report.attempts_started != 0 || report.successful_quotes != 0
        {
            return Err(
                "producer attempted a provider read after all demands were released".to_owned(),
            );
        }
    }
    if harness.transport.request_count() != requests_before_release {
        return Err("provider attempt count increased after demand release".to_owned());
    }
    let after_idle_bounds = read_row_bounds(&db.superuser, owner).await?;
    check_row_bounds(after_idle_bounds, false)?;
    if after_idle_bounds.active_leases != 0
        || after_idle_bounds.demand_rows != CONSUMER_COUNT as i64
        || after_idle_bounds.cache_rows != 1
        || after_idle_bounds.producer_rows != 1
    {
        return Err("idle cleanup changed the bounded synthetic row shape".to_owned());
    }
    let publication_after = publication_counts(&db.superuser).await?;
    if publication_before != publication_after {
        return Err("intraday endurance changed EOD/publication counts".to_owned());
    }

    println!(
        "intraday endurance counters: elapsed_s={} cycles={} attempts={} successes={} renewal_batches={} renewal_operations={} rss_baseline_kib={} rss_max_kib={} rss_final_kib={} rss_samples={}",
        elapsed.as_secs(),
        cycles,
        actual_attempts,
        reported_successes,
        renewal_batches,
        renewal_operations,
        rss_observation.baseline_kib,
        rss_observation.max_kib,
        rss_observation.final_kib,
        rss_observation.samples,
    );
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "30-minute real-wall-time provider-free endurance measurement"]
async fn intraday_endurance_real_30_minute_provider_free_soak() {
    run_body(endurance_body).await;
}
