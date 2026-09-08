mod intraday_producer_pipeline_support;
#[allow(dead_code)]
mod intraday_quotes_support;

use std::time::{Duration, Instant};

use chrono::{DateTime, NaiveDate, Utc};
use collectors::intraday_quotes::{INTRADAY_QUOTE_PATH, INTRADAY_QUOTE_TR_ID};
use intraday_producer_pipeline_support::{
    ClientHarness, PipelineClient, RequestRecord, TransportStep, assert_minimum_dispatch_spacing,
    install_current_window_contract, malformed_quote_response, publication_counts,
    rate_limited_response, transport_timeout_step, valid_quote_response,
};
use intraday_quotes_support::{
    IntradayTestDb, MembershipFixture, run_body, wait_until_database_time,
};
use job_queue::owner_equity_v2::{
    IntradayCacheRecord, IntradayProducer, IntradayProducerConfig, IntradayProducerCycleReport,
    IntradayQuoteDemandRequest, IntradayQuoteFailureCode,
};
use kis_client::transport::HttpResponse;
use market_data::intraday_quotes::IntradayQuoteDirection;
use serde_json::Value;
use tokio::sync::watch;
use uuid::Uuid;

fn demand_request(membership_id: Uuid, consumer_id: Uuid, key: &str) -> IntradayQuoteDemandRequest {
    IntradayQuoteDemandRequest::new(consumer_id, membership_id, 1, 0, key.to_owned())
        .expect("synthetic demand request is valid")
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
        IntradayProducer<PipelineClient>,
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
            &demand_request(
                fixture.membership_id,
                Uuid::new_v4(),
                &format!("{suffix}-demand"),
            ),
        )
        .await
        .map_err(|error| format!("synthetic demand setup failed: {error}"))?;
    let windows = install_current_window_contract(db, close_after_seconds).await?;
    let harness = ClientHarness::new(suffix, steps);
    let config = IntradayProducerConfig::for_worker(&format!("b2b-c1-{suffix}"))
        .map_err(|error| format!("synthetic producer configuration failed: {error}"))?;
    let producer = IntradayProducer::new(
        db.repository_as_worker(),
        harness.client.clone(),
        windows,
        config,
    );
    Ok((fixture, harness, producer))
}

async fn run_cycle(
    producer: &IntradayProducer<PipelineClient>,
) -> Result<IntradayProducerCycleReport, String> {
    let (_shutdown_tx, mut shutdown_rx) = watch::channel(false);
    producer
        .run_cycle(&mut shutdown_rx)
        .await
        .map_err(|error| format!("intraday producer cycle failed: {error}"))
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
        .map_err(|error| format!("current intraday cache read failed: {error}"))?
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

fn assert_exact_gets(harness: &ClientHarness, symbol: &str, expected: usize) {
    let requests = harness.transport.requests();
    assert_eq!(requests.len(), expected);
    for request in &requests {
        assert_exact_get(request, symbol);
    }
}

fn assert_ledger(
    harness: &ClientHarness,
    session_date: NaiveDate,
    attempts: u64,
    next_fence: u64,
    no_in_flight: bool,
) {
    let state = harness.state();
    let expected_date = session_date.to_string();
    assert_eq!(
        state.get("intraday_kst_date").and_then(Value::as_str),
        Some(expected_date.as_str())
    );
    assert_eq!(
        state.get("intraday_attempts").and_then(Value::as_u64),
        Some(attempts)
    );
    assert_eq!(
        state.get("next_fence").and_then(Value::as_u64),
        Some(next_fence)
    );
    if no_in_flight {
        assert!(state.get("in_flight").is_some_and(Value::is_null));
    }
}

fn assert_last_good_preserved(before: &IntradayCacheRecord, after: &IntradayCacheRecord) {
    assert_eq!(after.price, before.price);
    assert_eq!(after.base_price, before.base_price);
    assert_eq!(after.change_amount, before.change_amount);
    assert_eq!(after.change_percent, before.change_percent);
    assert_eq!(after.direction, before.direction);
    assert_eq!(after.halted, before.halted);
    assert_eq!(after.received_at, before.received_at);
    assert_eq!(after.last_success_at, before.last_success_at);
    assert_eq!(after.quote_version, before.quote_version);
}

fn assert_failure_without_success(
    cache: &IntradayCacheRecord,
    failure: IntradayQuoteFailureCode,
    producer_fence: u64,
) {
    assert_eq!(cache.price, None);
    assert_eq!(cache.base_price, None);
    assert_eq!(cache.change_amount, None);
    assert_eq!(cache.change_percent, None);
    assert_eq!(cache.direction, None);
    assert_eq!(cache.halted, None);
    assert_eq!(cache.received_at, None);
    assert_eq!(cache.last_success_at, None);
    assert_eq!(cache.quote_version, 0);
    assert_eq!(cache.last_failure_code, Some(failure));
    assert!(cache.last_failure_at.is_some());
    assert_eq!(cache.producer_fence, producer_fence);
}

fn assert_retry_after_persisted(harness: &ClientHarness) {
    let state = harness.state();
    let cooldown = state
        .get("broker_cooldown_until_ms")
        .and_then(Value::as_i64)
        .expect("broker cooldown is persisted");
    let completed = state
        .get("last_global_attempt_ms")
        .and_then(Value::as_i64)
        .expect("completed attempt time is persisted");
    assert!(cooldown >= completed.saturating_add(9_000));
}

#[tokio::test]
async fn actual_guarded_client_parser_and_producer_publish_one_private_quote() {
    run_body(|mut db| async move {
        let (fixture, harness, producer) = setup_pipeline(
            &mut db,
            "success",
            "005930",
            25,
            [TransportStep::Response(valid_quote_response("005930"))],
        )
        .await?;
        let before_eod = publication_counts(&db.superuser).await?;

        let report = run_cycle(&producer).await?;
        assert_eq!(report.owners_seen, 1);
        assert_eq!(report.owners_claimed, 1);
        assert_eq!(report.attempts_started, 1);
        assert_eq!(report.successful_quotes, 1);
        assert_eq!(report.failures_recorded, 0);
        assert_exact_gets(&harness, "005930", 1);
        assert_eq!(harness.issuer.calls(), 1);

        let cache = read_cache(&db, &fixture).await?;
        assert_eq!(cache.owner_user_id, fixture.owner_user_id);
        assert_eq!(cache.membership_id, fixture.membership_id);
        assert_eq!(cache.generation_id, fixture.generation_id);
        assert_eq!(cache.instrument_id, fixture.instrument_id);
        assert_eq!(cache.generation, fixture.generation);
        assert_eq!(cache.session_date, Some(db.session_date));
        assert_eq!(cache.calendar_source.as_deref(), Some("kis"));
        assert_eq!(
            cache.calendar_source_version.as_deref(),
            Some("kis-chk-holiday-v1:schema-1")
        );
        assert_eq!(
            cache.calendar_source_batch_id,
            Some(db.calendar_source_batch_id)
        );
        assert_eq!(
            cache.calendar_content_sha256.as_deref(),
            Some(db.calendar_content_sha256.as_str())
        );
        assert_eq!(
            cache.window_contract_sha256.as_deref(),
            Some(db.window_contract_sha256.as_str())
        );
        assert_eq!(cache.price.as_deref(), Some("72500.00000000"));
        assert_eq!(cache.base_price.as_deref(), Some("71000.00000000"));
        assert_eq!(cache.change_amount.as_deref(), Some("1500.00000000"));
        assert_eq!(cache.change_percent.as_deref(), Some("2.11000000"));
        assert_eq!(cache.direction, Some(IntradayQuoteDirection::Up));
        assert_eq!(cache.halted, Some(false));
        assert_eq!(cache.quote_version, 1);
        assert_eq!(cache.producer_fence, 1);
        assert!(cache.last_failure_code.is_none());
        let received_at = cache.received_at.expect("successful quote has a receipt");
        assert_eq!(cache.last_success_at, Some(received_at));
        let database_now: DateTime<Utc> = sqlx::query_scalar("SELECT pg_catalog.clock_timestamp()")
            .fetch_one(&db.superuser)
            .await
            .map_err(|_| "could not sample publication verification clock".to_owned())?;
        assert!(received_at <= database_now);
        assert_ledger(&harness, db.session_date, 1, 2, true);

        let after_eod = publication_counts(&db.superuser).await?;
        assert_eq!(before_eod, after_eod);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn invalid_schema_and_identity_are_not_retried_and_keep_last_good_quote() {
    run_body(|mut db| async move {
        let (fixture, harness, producer) = setup_pipeline(
            &mut db,
            "invalid-followup",
            "005930",
            40,
            [
                TransportStep::Response(valid_quote_response("005930")),
                TransportStep::Response(malformed_quote_response()),
                TransportStep::Response(valid_quote_response("999999")),
            ],
        )
        .await?;

        let first = run_cycle(&producer).await?;
        assert_eq!(first.attempts_started, 1);
        assert_eq!(first.successful_quotes, 1);
        let baseline = read_cache(&db, &fixture).await?;

        wait_until_database_time(
            &db.superuser,
            baseline.last_attempt_at + chrono::Duration::seconds(5),
        )
        .await?;
        let malformed = run_cycle(&producer).await?;
        assert_eq!(malformed.attempts_started, 1);
        assert_eq!(malformed.successful_quotes, 0);
        assert_eq!(malformed.failures_recorded, 1);
        assert_exact_gets(&harness, "005930", 2);
        let after_schema = read_cache(&db, &fixture).await?;
        assert_last_good_preserved(&baseline, &after_schema);
        assert_eq!(
            after_schema.last_failure_code,
            Some(IntradayQuoteFailureCode::ProviderResponseInvalid)
        );
        assert!(after_schema.last_failure_at.is_some());

        wait_until_database_time(
            &db.superuser,
            after_schema.last_attempt_at + chrono::Duration::seconds(5),
        )
        .await?;
        let wrong_identity = run_cycle(&producer).await?;
        assert_eq!(wrong_identity.attempts_started, 1);
        assert_eq!(wrong_identity.successful_quotes, 0);
        assert_eq!(wrong_identity.failures_recorded, 1);
        assert_exact_gets(&harness, "005930", 3);
        let after_identity = read_cache(&db, &fixture).await?;
        assert_last_good_preserved(&baseline, &after_identity);
        assert_eq!(
            after_identity.last_failure_code,
            Some(IntradayQuoteFailureCode::ProviderResponseInvalid)
        );
        assert!(after_identity.last_failure_at.is_some());
        assert_eq!(after_identity.producer_fence, 1);
        assert_ledger(&harness, db.session_date, 3, 4, true);
        assert_eq!(harness.issuer.calls(), 1);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn transient_503_then_success_uses_two_real_reservations_and_two_gets() {
    run_body(|mut db| async move {
        let (fixture, harness, producer) = setup_pipeline(
            &mut db,
            "retry-success",
            "005931",
            25,
            [
                TransportStep::ResponseAfter(
                    Duration::from_millis(100),
                    HttpResponse::status(503, "synthetic provider payload"),
                ),
                TransportStep::Response(valid_quote_response("005931")),
            ],
        )
        .await?;
        let report = run_cycle(&producer).await?;
        assert_eq!(report.owners_seen, 1);
        assert_eq!(report.owners_claimed, 1);
        assert_eq!(report.attempts_started, 2);
        assert_eq!(report.successful_quotes, 1);
        assert_eq!(report.failures_recorded, 1);
        assert_exact_gets(&harness, "005931", 2);
        assert_minimum_dispatch_spacing(&harness, Duration::from_secs(5));
        assert_eq!(harness.issuer.calls(), 1);

        let cache = read_cache(&db, &fixture).await?;
        assert_eq!(cache.price.as_deref(), Some("72500.00000000"));
        assert_eq!(cache.quote_version, 1);
        assert_eq!(cache.producer_fence, 1);
        assert!(cache.received_at.is_some());
        assert!(cache.last_success_at.is_some());
        assert!(cache.last_failure_code.is_none());
        assert_ledger(&harness, db.session_date, 2, 3, true);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn three_transient_503s_stop_at_three_real_gets_without_a_fourth_attempt() {
    run_body(|mut db| async move {
        let (fixture, harness, producer) = setup_pipeline(
            &mut db,
            "retry-exhausted",
            "005932",
            25,
            [
                TransportStep::ResponseAfter(
                    Duration::from_millis(100),
                    HttpResponse::status(503, "synthetic provider payload"),
                ),
                TransportStep::ResponseAfter(
                    Duration::from_millis(100),
                    HttpResponse::status(503, "synthetic provider payload"),
                ),
                TransportStep::ResponseAfter(
                    Duration::from_millis(100),
                    HttpResponse::status(503, "synthetic provider payload"),
                ),
            ],
        )
        .await?;
        let report = run_cycle(&producer).await?;
        assert_eq!(report.attempts_started, 3);
        assert_eq!(report.successful_quotes, 0);
        assert_eq!(report.failures_recorded, 3);
        assert_exact_gets(&harness, "005932", 3);
        assert_minimum_dispatch_spacing(&harness, Duration::from_secs(5));
        assert_eq!(harness.issuer.calls(), 1);

        let cache = read_cache(&db, &fixture).await?;
        assert_failure_without_success(&cache, IntradayQuoteFailureCode::ProviderUnavailable, 1);
        assert_ledger(&harness, db.session_date, 3, 4, true);

        let no_fourth = run_cycle(&producer).await?;
        assert_eq!(no_fourth.attempts_started, 0);
        assert_eq!(harness.transport.request_count(), 3);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn hanging_transport_is_bounded_by_three_seconds_and_records_typed_timeout() {
    run_body(|mut db| async move {
        let (fixture, harness, producer) =
            setup_pipeline(&mut db, "timeout", "005933", 7, [TransportStep::Hang]).await?;
        let started = Instant::now();
        let report = tokio::time::timeout(Duration::from_secs(5), run_cycle(&producer))
            .await
            .map_err(|_| "producer timeout cycle exceeded bounded test wait".to_owned())??;
        let elapsed = started.elapsed();
        assert!(elapsed >= Duration::from_secs(3));
        assert!(elapsed < Duration::from_secs(5));
        assert_eq!(report.attempts_started, 1);
        assert_eq!(report.successful_quotes, 0);
        assert_eq!(report.failures_recorded, 1);
        assert_exact_gets(&harness, "005933", 1);
        assert_eq!(harness.issuer.calls(), 1);
        let cache = read_cache(&db, &fixture).await?;
        assert_failure_without_success(&cache, IntradayQuoteFailureCode::ProviderTimeout, 1);
        assert_ledger(&harness, db.session_date, 1, 2, false);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn transport_shaped_504_is_typed_timeout_without_a_second_get() {
    run_body(|mut db| async move {
        let (fixture, harness, producer) = setup_pipeline(
            &mut db,
            "transport-timeout",
            "005934",
            4,
            [transport_timeout_step()],
        )
        .await?;
        let report = run_cycle(&producer).await?;
        assert_eq!(report.attempts_started, 1);
        assert_eq!(report.successful_quotes, 0);
        assert_eq!(report.failures_recorded, 1);
        assert_exact_gets(&harness, "005934", 1);
        assert_eq!(harness.issuer.calls(), 1);
        let cache = read_cache(&db, &fixture).await?;
        assert_failure_without_success(&cache, IntradayQuoteFailureCode::ProviderTimeout, 1);
        assert_ledger(&harness, db.session_date, 1, 2, true);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn retry_after_is_persisted_and_stops_when_session_demand_or_producer_bound_is_too_short() {
    run_body(|mut db| async move {
        let (fixture, harness, producer) = setup_pipeline(
            &mut db,
            "429-session-bound",
            "005935",
            4,
            [TransportStep::Response(rate_limited_response(9))],
        )
        .await?;
        let report = run_cycle(&producer).await?;
        assert_eq!(report.attempts_started, 1);
        assert_eq!(report.failures_recorded, 1);
        assert_exact_gets(&harness, "005935", 1);
        let cache = read_cache(&db, &fixture).await?;
        assert_failure_without_success(&cache, IntradayQuoteFailureCode::ProviderRateLimited, 1);
        assert_retry_after_persisted(&harness);
        assert_ledger(&harness, db.session_date, 1, 2, true);
        Ok(())
    })
    .await;

    run_body(|mut db| async move {
        let (fixture, harness, producer) = setup_pipeline(
            &mut db,
            "429-demand-bound",
            "005936",
            25,
            [TransportStep::Response(rate_limited_response(9))],
        )
        .await?;
        sqlx::query(
            "UPDATE public.owner_intraday_quote_demands
                SET lease_expires_at = pg_catalog.clock_timestamp() + INTERVAL '5 seconds'
              WHERE owner_user_id = $1 AND membership_id = $2",
        )
        .bind(fixture.owner_user_id)
        .bind(fixture.membership_id)
        .execute(&db.superuser)
        .await
        .map_err(|_| "could not shorten demand-bound fixture lease".to_owned())?;
        let report = run_cycle(&producer).await?;
        assert_eq!(report.attempts_started, 1);
        assert_eq!(report.failures_recorded, 1);
        assert_exact_gets(&harness, "005936", 1);
        let cache = read_cache(&db, &fixture).await?;
        assert_failure_without_success(&cache, IntradayQuoteFailureCode::ProviderRateLimited, 1);
        assert_retry_after_persisted(&harness);
        assert_ledger(&harness, db.session_date, 1, 2, true);
        Ok(())
    })
    .await;

    run_body(|mut db| async move {
        let owner = db.seed_owner("429-producer-bound").await?;
        let fixture = db.seed_ready_membership(owner, "005937.KRX").await?;
        db.repository_as_app()
            .create_or_renew_demand(
                owner,
                &demand_request(
                    fixture.membership_id,
                    Uuid::new_v4(),
                    "429-producer-bound-demand",
                ),
            )
            .await
            .map_err(|error| format!("producer-bound demand setup failed: {error}"))?;
        let windows = install_current_window_contract(&mut db, 25).await?;
        let harness = ClientHarness::new(
            "429-producer-bound",
            [TransportStep::ResponseAfterShorteningProducerLease {
                wait_before_update: Duration::from_millis(200),
                response: rate_limited_response(9),
                pool: db.superuser.clone(),
                owner_user_id: owner,
            }],
        );
        let config = IntradayProducerConfig::for_worker("b2b-c1-429-producer-bound")
            .map_err(|error| format!("producer-bound configuration failed: {error}"))?;
        let producer = IntradayProducer::new(
            db.repository_as_worker(),
            harness.client.clone(),
            windows,
            config,
        );
        let report = run_cycle(&producer).await?;
        assert_eq!(report.attempts_started, 1);
        assert_eq!(report.failures_recorded, 1);
        assert_exact_gets(&harness, "005937", 1);
        let cache = read_cache(&db, &fixture).await?;
        assert_failure_without_success(&cache, IntradayQuoteFailureCode::ProviderRateLimited, 1);
        assert_retry_after_persisted(&harness);
        assert_ledger(&harness, db.session_date, 1, 2, true);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn unauthorized_invalidates_shared_token_keeps_sixty_second_issue_debt_and_allows_one_retry_check()
 {
    run_body(|mut db| async move {
        let (fixture, harness, producer) = setup_pipeline(
            &mut db,
            "401-issue-debt",
            "005938",
            15,
            [TransportStep::Response(HttpResponse::status(
                401,
                "synthetic provider payload",
            ))],
        )
        .await?;
        let report = run_cycle(&producer).await?;
        assert_eq!(report.attempts_started, 2);
        assert_eq!(report.successful_quotes, 0);
        assert_eq!(report.failures_recorded, 1);
        assert_exact_gets(&harness, "005938", 1);
        assert_eq!(harness.issuer.calls(), 1);
        let state = harness.state();
        assert!(state.get("token").is_some_and(Value::is_null));
        assert!(
            state
                .get("last_issue_attempt_ms")
                .and_then(Value::as_i64)
                .is_some()
        );
        assert_ledger(&harness, db.session_date, 1, 2, true);
        let cache = read_cache(&db, &fixture).await?;
        assert_failure_without_success(&cache, IntradayQuoteFailureCode::ProviderUnavailable, 1);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn unauthorized_stops_without_retry_when_session_evidence_expires_first() {
    run_body(|mut db| async move {
        let (fixture, harness, producer) = setup_pipeline(
            &mut db,
            "401-session-expiry",
            "005939",
            4,
            [TransportStep::Response(HttpResponse::status(
                401,
                "synthetic provider payload",
            ))],
        )
        .await?;
        let report = run_cycle(&producer).await?;
        assert_eq!(report.attempts_started, 1);
        assert_eq!(report.successful_quotes, 0);
        assert_eq!(report.failures_recorded, 1);
        assert_exact_gets(&harness, "005939", 1);
        assert_eq!(harness.issuer.calls(), 1);
        let state = harness.state();
        assert!(state.get("token").is_some_and(Value::is_null));
        assert_ledger(&harness, db.session_date, 1, 2, true);
        let cache = read_cache(&db, &fixture).await?;
        assert_failure_without_success(&cache, IntradayQuoteFailureCode::ProviderUnavailable, 1);
        Ok(())
    })
    .await;
}
