mod intraday_quotes_support;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use chrono::{DateTime, FixedOffset, NaiveDate, TimeZone, Utc};
use collectors::intraday_quotes::IntradaySessionWindowContract;
use intraday_quotes_support::{
    MembershipFixture, run_body, wait_for_blocked_session, wait_until_database_time,
};
use job_queue::owner_equity_v2::{
    EligibilityCheck, IntradayAttemptReservation, IntradayProducer, IntradayProducerConfig,
    IntradayPublicationContext, IntradayQuoteDemandRequest, IntradayQuoteFailureCode,
    IntradayQuoteIdentity, IntradayQuoteReader, IntradayQuoteReceipt, IntradaySessionWindow,
    IntradayStorageError, ProducerLease,
};
use kis_client::{IntradayAttemptError, IntradayAttemptOutcome};
use market_data::intraday_quotes::{IntradayQuote, IntradayQuoteDirection};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, watch};
use uuid::Uuid;

fn demand_request(membership_id: Uuid, consumer_id: Uuid, key: &str) -> IntradayQuoteDemandRequest {
    IntradayQuoteDemandRequest::new(consumer_id, membership_id, 1, 0, key.to_owned())
        .expect("synthetic demand request is valid")
}

fn entry(
    date: NaiveDate,
    disposition: &str,
    open_local: Option<&str>,
    close_local: Option<&str>,
) -> Value {
    json!({
        "date": date.to_string(),
        "disposition": disposition,
        "open_local": open_local,
        "close_local": close_local,
        "evidence_url": "https://global.krx.co.kr/contents/test",
        "evidence_retrieved_at": "1970-01-01T00:00:00Z",
        "evidence_sha256": format!("sha256:{}", "c".repeat(64)),
    })
}

fn contract_for(date: NaiveDate, disposition: &str) -> Arc<IntradaySessionWindowContract> {
    let entries = match disposition {
        "CLOSED" => vec![entry(date, disposition, None, None)],
        _ => vec![entry(date, "SPECIAL", Some("00:00:00"), Some("23:59:59"))],
    };
    let bytes = serde_json::to_vec(&json!({
        "schema_version": 1,
        "exchange": "KRX",
        "timezone": "Asia/Seoul",
        "entries": entries,
    }))
    .expect("session-window fixture serializes");
    let hash = format!("sha256:{:x}", Sha256::digest(&bytes));
    Arc::new(
        IntradaySessionWindowContract::from_bytes(&bytes, &hash)
            .expect("session-window fixture is valid"),
    )
}

#[derive(Debug)]
struct CountingReader {
    calls: AtomicUsize,
    symbols: Mutex<Vec<String>>,
    result: IntradayAttemptError,
}

impl CountingReader {
    fn new(result: IntradayAttemptError) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            symbols: Mutex::new(Vec::new()),
            result,
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl job_queue::owner_equity_v2::IntradayQuoteReader for CountingReader {
    async fn get_intraday_attempt(
        &self,
        query: &[(String, String)],
        eligibility_check: EligibilityCheck,
    ) -> IntradayAttemptOutcome {
        if !matches!(eligibility_check().await, Ok(true)) {
            return IntradayAttemptOutcome::Failed {
                error: IntradayAttemptError::CallerIneligible,
                reservation: None,
            };
        }
        let symbol = query
            .iter()
            .find(|(key, _)| key == "FID_INPUT_ISCD")
            .map(|(_, value)| value.clone())
            .unwrap_or_default();
        self.symbols.lock().await.push(symbol);
        self.calls.fetch_add(1, Ordering::SeqCst);
        IntradayAttemptOutcome::Failed {
            error: self.result.clone(),
            reservation: None,
        }
    }
}

fn identity(fixture: &MembershipFixture) -> IntradayQuoteIdentity {
    IntradayQuoteIdentity::new(
        fixture.owner_user_id,
        fixture.membership_id,
        fixture.generation_id,
        fixture.instrument_id.clone(),
        fixture.generation,
    )
    .expect("synthetic identity is valid")
}

fn quote(symbol: &str) -> IntradayQuote {
    IntradayQuote {
        symbol: symbol.to_owned(),
        price: "72500".to_owned(),
        change_from_previous_day: "1500".to_owned(),
        change_percent_from_previous_day: "2.11".to_owned(),
        direction: IntradayQuoteDirection::Up,
        base_price: "71000".to_owned(),
        halted: false,
    }
}

fn day_bounds(date: NaiveDate) -> (DateTime<Utc>, DateTime<Utc>) {
    let kst = FixedOffset::east_opt(9 * 60 * 60).expect("KST offset");
    let open = kst
        .from_local_datetime(&date.and_hms_opt(0, 0, 0).expect("midnight"))
        .single()
        .expect("unambiguous KST midnight")
        .with_timezone(&Utc);
    let close = kst
        .from_local_datetime(
            &date
                .and_hms_micro_opt(23, 59, 59, 999_999)
                .expect("end of day"),
        )
        .single()
        .expect("unambiguous KST close")
        .with_timezone(&Utc);
    (open, close)
}

fn context(
    db: &intraday_quotes_support::IntradayTestDb,
    fixture: &MembershipFixture,
    lease: ProducerLease,
) -> IntradayPublicationContext {
    let attempt = IntradayAttemptReservation::from_shared_evidence(
        fixture.owner_user_id,
        lease.holder_id,
        db.session_date,
        1,
        lease.fencing_token,
    )
    .expect("synthetic shared reservation evidence is valid");
    IntradayPublicationContext::new(lease, identity(fixture), db.session_proof(), attempt)
        .expect("synthetic publication context is valid")
}

fn producer_config() -> IntradayProducerConfig {
    IntradayProducerConfig::for_worker("intraday-test-worker")
        .expect("synthetic producer worker id is valid")
}

#[tokio::test]
async fn no_demand_unknown_closed_and_expired_demand_do_zero_reader_calls() {
    run_body(|db| async move {
        let owner = db.seed_owner("producer-gates").await?;
        let reader = Arc::new(CountingReader::new(IntradayAttemptError::Busy));
        let (_shutdown_tx, mut shutdown) = watch::channel(false);
        let producer = IntradayProducer::new(
            db.repository_as_worker(),
            reader.clone(),
            contract_for(db.session_date, "CLOSED"),
            producer_config(),
        );
        let report = producer
            .run_cycle(&mut shutdown)
            .await
            .map_err(|error| format!("no-demand cycle failed: {error}"))?;
        if report.owners_seen != 0 || report.attempts_started != 0 || reader.calls() != 0 {
            return Err("no-demand cycle reached the reader".to_owned());
        }

        let fixture = db.seed_ready_membership(owner, "005930.KRX").await?;
        let app = db.repository_as_app();
        let demand = app
            .create_or_renew_demand(
                owner,
                &demand_request(fixture.membership_id, Uuid::new_v4(), "producer-gate"),
            )
            .await
            .map_err(|error| format!("gate demand setup failed: {error}"))?;

        let unknown_reader = Arc::new(CountingReader::new(IntradayAttemptError::Busy));
        let unknown = IntradayProducer::new(
            db.repository_as_worker(),
            unknown_reader.clone(),
            contract_for(
                db.session_date
                    .checked_add_days(chrono::Days::new(1))
                    .expect("next date"),
                "SPECIAL",
            ),
            producer_config(),
        );
        let unknown_report = unknown
            .run_cycle(&mut shutdown)
            .await
            .map_err(|error| format!("unknown cycle failed: {error}"))?;
        if unknown_report.attempts_started != 0 || unknown_reader.calls() != 0 {
            return Err("unknown session reached the reader".to_owned());
        }

        let closed_reader = Arc::new(CountingReader::new(IntradayAttemptError::Busy));
        let closed = IntradayProducer::new(
            db.repository_as_worker(),
            closed_reader.clone(),
            contract_for(db.session_date, "CLOSED"),
            producer_config(),
        );
        let closed_report = closed
            .run_cycle(&mut shutdown)
            .await
            .map_err(|error| format!("closed cycle failed: {error}"))?;
        if closed_report.attempts_started != 0 || closed_reader.calls() != 0 {
            return Err("closed session reached the reader".to_owned());
        }

        sqlx::query(
            "UPDATE public.owner_intraday_quote_demands
                SET created_at = pg_catalog.clock_timestamp() - INTERVAL '2 seconds',
                    lease_expires_at = pg_catalog.clock_timestamp() - INTERVAL '1 second'
              WHERE id = $1",
        )
        .bind(demand.lease.demand_id)
        .execute(&db.superuser)
        .await
        .map_err(|_| "could not expire gate demand".to_owned())?;
        let expired_reader = Arc::new(CountingReader::new(IntradayAttemptError::Busy));
        let expired = IntradayProducer::new(
            db.repository_as_worker(),
            expired_reader.clone(),
            contract_for(db.session_date, "SPECIAL"),
            producer_config(),
        );
        let expired_report = expired
            .run_cycle(&mut shutdown)
            .await
            .map_err(|error| format!("expired cycle failed: {error}"))?;
        if expired_report.owners_seen != 0 || expired_reader.calls() != 0 {
            return Err("expired ACTIVE demand was treated as live".to_owned());
        }
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn duplicate_consumers_merge_and_five_identities_are_enumerated_once() {
    run_body(|db| async move {
        let owner = db.seed_owner("producer-fairness").await?;
        let mut fixtures = Vec::new();
        for index in 0..5 {
            let instrument = format!("{:06}.KRX", 100 + index);
            let fixture = db.seed_ready_membership(owner, &instrument).await?;
            let app = db.repository_as_app();
            app.create_or_renew_demand(
                owner,
                &demand_request(
                    fixture.membership_id,
                    Uuid::new_v4(),
                    &format!("fair-{index}-a"),
                ),
            )
            .await
            .map_err(|error| format!("fairness demand failed: {error}"))?;
            if index == 0 {
                app.create_or_renew_demand(
                    owner,
                    &demand_request(fixture.membership_id, Uuid::new_v4(), "fair-duplicate"),
                )
                .await
                .map_err(|error| format!("duplicate demand failed: {error}"))?;
            }
            fixtures.push(fixture);
        }
        let work = db
            .repository_as_worker()
            .active_quote_work(owner)
            .await
            .map_err(|error| format!("active quote merge failed: {error}"))?;
        let identities = work
            .iter()
            .map(|item| item.identity.instrument_id.clone())
            .collect::<Vec<_>>();
        if work.len() != 5
            || identities
                != (0..5)
                    .map(|index| format!("{:06}.KRX", 100 + index))
                    .collect::<Vec<_>>()
        {
            return Err(format!(
                "duplicate merge did not produce five ordered identities: {identities:?}"
            ));
        }
        let distinct = identities.iter().cloned().collect::<BTreeSet<_>>();
        if distinct.len() != 5 {
            return Err("duplicate consumers created duplicate scheduler turns".to_owned());
        }
        let owners = db
            .repository_as_worker()
            .active_demand_owners()
            .await
            .map_err(|error| format!("active owner enumeration failed: {error}"))?;
        if owners != vec![owner] {
            return Err("active owner enumeration was not owner-scoped".to_owned());
        }
        let _ = fixtures;
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn open_cycle_rechecks_eligibility_and_busy_reader_is_one_nonblocking_attempt() {
    run_body(|db| async move {
        let owner = db.seed_owner("producer-open").await?;
        let fixture = db.seed_ready_membership(owner, "005931.KRX").await?;
        db.repository_as_app()
            .create_or_renew_demand(
                owner,
                &demand_request(fixture.membership_id, Uuid::new_v4(), "open-demand"),
            )
            .await
            .map_err(|error| format!("open demand setup failed: {error}"))?;
        let reader = Arc::new(CountingReader::new(IntradayAttemptError::Busy));
        let contract = contract_for(db.session_date, "SPECIAL");
        let work = db
            .repository_as_worker()
            .active_quote_work(owner)
            .await
            .map_err(|error| format!("open work discovery failed: {error}"))?;
        if work.len() != 1 || work[0].identity != identity(&fixture) {
            return Err("open cycle did not discover the one current identity".to_owned());
        }
        let producer = IntradayProducer::new(
            db.repository_as_worker(),
            reader.clone(),
            contract,
            producer_config(),
        );
        let (_shutdown_tx, mut shutdown) = watch::channel(false);
        let report = producer
            .run_cycle(&mut shutdown)
            .await
            .map_err(|error| format!("open cycle failed: {error}"))?;
        if report.attempts_started != 1
            || report.successful_quotes != 0
            || report.failures_recorded != 0
            || reader.calls() != 1
        {
            return Err(format!("unexpected open-cycle report: {report:?}"));
        }
        let mut second_shutdown = shutdown.clone();
        let second = producer
            .run_cycle(&mut second_shutdown)
            .await
            .map_err(|error| format!("busy-deferred cycle failed: {error}"))?;
        if second.attempts_started != 0 || reader.calls() != 1 {
            return Err("shared busy skip entered an eager retry loop".to_owned());
        }
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn eligibility_rechecks_window_after_producer_lock_wait_and_dispatch_stays_guarded() {
    run_body(|db| async move {
        let owner = db.seed_owner("eligibility-window-close").await?;
        let fixture = db.seed_ready_membership(owner, "005933.KRX").await?;
        db.repository_as_app()
            .create_or_renew_demand(
                owner,
                &demand_request(fixture.membership_id, Uuid::new_v4(), "eligibility-close"),
            )
            .await
            .map_err(|error| format!("eligibility demand setup failed: {error}"))?;
        let worker = db.repository_as_worker();
        let claim = worker
            .claim_producer(owner, Uuid::new_v4())
            .await
            .map_err(|error| format!("eligibility claim failed: {error}"))?;
        let lease = claim.lease;
        let quote_identity = identity(&fixture);
        let session = db.session_proof();
        let (open_at, _) = day_bounds(db.session_date);
        let before_wait: DateTime<Utc> = sqlx::query_scalar("SELECT pg_catalog.clock_timestamp()")
            .fetch_one(&db.superuser)
            .await
            .map_err(|_| "could not sample eligibility close start".to_owned())?;
        let close_at = before_wait + chrono::Duration::seconds(1);
        if close_at
            .with_timezone(&FixedOffset::east_opt(9 * 60 * 60).expect("KST"))
            .date_naive()
            != db.session_date
        {
            return Err("eligibility close fixture crossed the KST date boundary".to_owned());
        }
        let window = IntradaySessionWindow::new(db.session_date, open_at, close_at)
            .map_err(|error| format!("eligibility close window failed: {error}"))?;

        if !worker
            .quote_attempt_eligible(&lease, &quote_identity, &session, &window)
            .await
            .map_err(|error| format!("open eligibility check failed: {error}"))?
        {
            return Err(
                "open eligibility fixture was not eligible before the lock wait".to_owned(),
            );
        }

        let mut observer_tx = db
            .superuser
            .begin()
            .await
            .map_err(|_| "could not begin eligibility close observer".to_owned())?;
        let observer_pid: i32 = sqlx::query_scalar("SELECT pg_catalog.pg_backend_pid()")
            .fetch_one(&mut *observer_tx)
            .await
            .map_err(|_| "could not identify eligibility close observer".to_owned())?;
        sqlx::query(
            "SELECT owner_user_id
               FROM public.owner_intraday_quote_producers
              WHERE owner_user_id = $1
              FOR UPDATE",
        )
        .bind(owner)
        .fetch_one(&mut *observer_tx)
        .await
        .map_err(|_| "could not hold producer row for eligibility close".to_owned())?;

        let eligibility_task = tokio::spawn({
            let worker = worker.clone();
            let lease = lease.clone();
            let quote_identity = quote_identity.clone();
            let session = session.clone();
            async move {
                worker
                    .quote_attempt_eligible(&lease, &quote_identity, &session, &window)
                    .await
            }
        });
        let (_, blockers) =
            wait_for_blocked_session(&db.superuser, "worker", "owner_intraday_quote_producers")
                .await?;
        if !blockers.contains(&observer_pid) {
            return Err("eligibility close check did not block on producer row".to_owned());
        }
        wait_until_database_time(
            &db.superuser,
            close_at + chrono::Duration::milliseconds(100),
        )
        .await?;
        observer_tx
            .commit()
            .await
            .map_err(|_| "could not release eligibility close producer lock".to_owned())?;
        let eligible = eligibility_task
            .await
            .map_err(|_| "eligibility close task failed".to_owned())?
            .map_err(|error| format!("eligibility close check failed: {error}"))?;
        if eligible {
            return Err("eligibility remained true after the proven window close".to_owned());
        }

        let reader = CountingReader::new(IntradayAttemptError::Busy);
        let outcome = reader
            .get_intraday_attempt(
                &[("FID_INPUT_ISCD".to_owned(), "005933".to_owned())],
                Box::new({
                    let worker = worker.clone();
                    let lease = lease.clone();
                    let quote_identity = quote_identity.clone();
                    let session = session.clone();
                    move || {
                        let worker = worker.clone();
                        let lease = lease.clone();
                        let quote_identity = quote_identity.clone();
                        let session = session.clone();
                        Box::pin(async move {
                            worker
                                .quote_attempt_eligible(&lease, &quote_identity, &session, &window)
                                .await
                                .map_err(|_| ())
                        })
                    }
                }),
            )
            .await;
        if !matches!(
            outcome,
            IntradayAttemptOutcome::Failed {
                error: IntradayAttemptError::CallerIneligible,
                reservation: None
            }
        ) || reader.calls() != 0
        {
            return Err("closed final eligibility guard dispatched the fake reader".to_owned());
        }
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn eligibility_rechecks_producer_and_demand_expiry_after_lock_wait() {
    run_body(|db| async move {
        let owner = db.seed_owner("eligibility-expiry").await?;
        let fixture = db.seed_ready_membership(owner, "005934.KRX").await?;
        db.repository_as_app()
            .create_or_renew_demand(
                owner,
                &demand_request(fixture.membership_id, Uuid::new_v4(), "eligibility-expiry"),
            )
            .await
            .map_err(|error| format!("expiry demand setup failed: {error}"))?;
        let worker = db.repository_as_worker();
        let claim = worker
            .claim_producer(owner, Uuid::new_v4())
            .await
            .map_err(|error| format!("expiry claim failed: {error}"))?;
        let lease = claim.lease;
        let quote_identity = identity(&fixture);
        let session = db.session_proof();
        let (open_at, close_at) = day_bounds(db.session_date);
        sqlx::query(
            "UPDATE public.owner_intraday_quote_producers
                SET lease_expires_at = pg_catalog.clock_timestamp() + INTERVAL '1 second',
                    heartbeat_at = pg_catalog.clock_timestamp(),
                    updated_at = pg_catalog.clock_timestamp()
              WHERE owner_user_id = $1",
        )
        .bind(owner)
        .execute(&db.superuser)
        .await
        .map_err(|_| "could not shorten producer fixture lease".to_owned())?;
        sqlx::query(
            "UPDATE public.owner_intraday_quote_demands
                SET lease_expires_at = pg_catalog.clock_timestamp() + INTERVAL '1 second',
                    updated_at = pg_catalog.clock_timestamp()
              WHERE owner_user_id = $1 AND membership_id = $2",
        )
        .bind(owner)
        .bind(fixture.membership_id)
        .execute(&db.superuser)
        .await
        .map_err(|_| "could not shorten demand fixture lease".to_owned())?;
        let wait_start: DateTime<Utc> = sqlx::query_scalar("SELECT pg_catalog.clock_timestamp()")
            .fetch_one(&db.superuser)
            .await
            .map_err(|_| "could not sample expiry wait start".to_owned())?;

        let mut observer_tx = db
            .superuser
            .begin()
            .await
            .map_err(|_| "could not begin expiry observer".to_owned())?;
        let observer_pid: i32 = sqlx::query_scalar("SELECT pg_catalog.pg_backend_pid()")
            .fetch_one(&mut *observer_tx)
            .await
            .map_err(|_| "could not identify expiry observer".to_owned())?;
        sqlx::query(
            "SELECT owner_user_id
               FROM public.owner_intraday_quote_producers
              WHERE owner_user_id = $1
              FOR UPDATE",
        )
        .bind(owner)
        .fetch_one(&mut *observer_tx)
        .await
        .map_err(|_| "could not hold producer row for expiry".to_owned())?;
        let eligibility_task = tokio::spawn({
            let worker = worker.clone();
            let lease = lease.clone();
            let quote_identity = quote_identity.clone();
            let session = session.clone();
            async move {
                worker
                    .quote_attempt_eligible(
                        &lease,
                        &quote_identity,
                        &session,
                        &IntradaySessionWindow::new(session.session_date, open_at, close_at)
                            .expect("full-day eligibility window is valid"),
                    )
                    .await
            }
        });
        let (_, blockers) =
            wait_for_blocked_session(&db.superuser, "worker", "owner_intraday_quote_producers")
                .await?;
        if !blockers.contains(&observer_pid) {
            return Err("expiry eligibility check did not block on producer row".to_owned());
        }
        wait_until_database_time(&db.superuser, wait_start + chrono::Duration::seconds(2)).await?;
        observer_tx
            .commit()
            .await
            .map_err(|_| "could not release expiry producer lock".to_owned())?;
        let eligible = eligibility_task
            .await
            .map_err(|_| "expiry eligibility task failed".to_owned())?
            .map_err(|error| format!("expiry eligibility check failed: {error}"))?;
        if eligible {
            return Err("expired producer and demand remained eligible after lock wait".to_owned());
        }
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn in_window_publication_and_failure_keep_last_good_and_close_after_lock_wait_discards() {
    run_body(|db| async move {
        let owner = db.seed_owner("producer-publication").await?;
        let fixture = db.seed_ready_membership(owner, "005932.KRX").await?;
        db.repository_as_app()
            .create_or_renew_demand(
                owner,
                &demand_request(fixture.membership_id, Uuid::new_v4(), "publication-demand"),
            )
            .await
            .map_err(|error| format!("publication demand setup failed: {error}"))?;
        let worker = db.repository_as_worker();
        let claim = worker
            .claim_producer(owner, Uuid::new_v4())
            .await
            .map_err(|error| format!("publication claim failed: {error}"))?;
        let publication_context = context(&db, &fixture, claim.lease.clone());
        let (open_at, close_at) = day_bounds(db.session_date);
        let window = IntradaySessionWindow::new(db.session_date, open_at, close_at)
            .map_err(|error| format!("day window failed: {error}"))?;
        let received_at: DateTime<Utc> = sqlx::query_scalar("SELECT pg_catalog.clock_timestamp()")
            .fetch_one(&db.superuser)
            .await
            .map_err(|_| "could not sample publication receipt time".to_owned())?;
        let success = worker
            .publish_success_in_window(
                &publication_context,
                &quote("005932"),
                IntradayQuoteReceipt::captured(received_at),
                &window,
            )
            .await
            .map_err(|error| format!("in-window publication failed: {error}"))?;
        let failure = worker
            .record_failure_in_window(
                &publication_context,
                IntradayQuoteFailureCode::ProviderTimeout,
                &window,
            )
            .await
            .map_err(|error| format!("in-window failure failed: {error}"))?;
        if failure.quote_version != success.quote_version
            || failure.price != success.price
            || failure.last_success_at != success.last_success_at
            || failure.last_failure_code != Some(IntradayQuoteFailureCode::ProviderTimeout)
        {
            return Err("failure changed the last-good quote or version".to_owned());
        }

        let now: DateTime<Utc> = sqlx::query_scalar("SELECT pg_catalog.clock_timestamp()")
            .fetch_one(&db.superuser)
            .await
            .map_err(|_| "could not sample close-wait start".to_owned())?;
        let short_window = IntradaySessionWindow::new(
            db.session_date,
            open_at,
            now + chrono::Duration::seconds(1),
        )
        .map_err(|error| format!("short window failed: {error}"))?;
        let mut observer_tx = db
            .superuser
            .begin()
            .await
            .map_err(|_| "could not begin close-wait observer".to_owned())?;
        let observer_pid: i32 = sqlx::query_scalar("SELECT pg_catalog.pg_backend_pid()")
            .fetch_one(&mut *observer_tx)
            .await
            .map_err(|_| "could not identify close-wait observer".to_owned())?;
        sqlx::query(
            "SELECT owner_user_id
               FROM public.owner_intraday_quote_producers
              WHERE owner_user_id = $1
              FOR UPDATE",
        )
        .bind(owner)
        .fetch_one(&mut *observer_tx)
        .await
        .map_err(|_| "could not hold producer row for close-wait".to_owned())?;
        let publish_task = tokio::spawn({
            let worker = worker.clone();
            let publication_context = publication_context.clone();
            async move {
                worker
                    .publish_success_in_window(
                        &publication_context,
                        &quote("005932"),
                        IntradayQuoteReceipt::captured(received_at),
                        &short_window,
                    )
                    .await
            }
        });
        let (_, blockers) =
            wait_for_blocked_session(&db.superuser, "worker", "owner_intraday_quote_producers")
                .await?;
        if !blockers.contains(&observer_pid) {
            return Err("close-wait publication did not block on the producer row".to_owned());
        }
        wait_until_database_time(&db.superuser, now + chrono::Duration::seconds(2)).await?;
        observer_tx
            .commit()
            .await
            .map_err(|_| "could not release close-wait producer lock".to_owned())?;
        let late = publish_task
            .await
            .map_err(|_| "close-wait publication task failed".to_owned())?;
        if late != Err(IntradayStorageError::SessionProofInvalid) {
            return Err(format!(
                "late close publication was not discarded: {late:?}"
            ));
        }
        let current = db
            .repository_as_app()
            .read_current_cache(owner, fixture.membership_id, 1, &db.session_proof())
            .await
            .map_err(|error| format!("last-good read failed: {error}"))?
            .ok_or_else(|| "late close publication removed the last-good quote".to_owned())?;
        if current.quote_version != success.quote_version || current.price != success.price {
            return Err("late close publication changed the last-good quote".to_owned());
        }
        Ok(())
    })
    .await;
}
