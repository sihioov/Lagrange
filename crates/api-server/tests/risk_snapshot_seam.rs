//! The seam between the submission route and the Risk Gateway.
//!
//! This file exists because that seam had no test at all, and a defect lived
//! there: the route handed the gate `risk_gateway::testing::snapshot_all_green()`
//! -- a fixture whose own documentation is *"A snapshot that passes all twelve
//! checks"* -- with `instrument_id` and `correlation_id` overwritten, while the
//! order sent to the broker was built separately from the caller's request.
//!
//! Every gate suite passed throughout. They exercise the gate directly with
//! real snapshots, so they prove the gate works; none of them proves the
//! server ASKS IT ABOUT THE RIGHT ORDER. That is the only property here.

mod common;

use api_server::repos::reconciliation::ReconciliationRepo;
use api_server::risk_snapshot::{GateOrder, for_submission, limits_for, parse_side, side_str};
use collectors::{PostgresPublicationSink, PublicationSink, PublishOutcome};
use common::{Harness, actor_pool};
use domain::{Price, Quantity, TradingDate, UtcTimestamp};
use market_data::contract::MARKET_KR;
use market_data::ingest::{IngestRequest, ingest_bundle};
use market_data::provider::{KrxProvider, RecordedBundle};
use market_data::publication::{DataBatchKind, PublicationBundle};
use market_data::storage::RawStore;
use risk_gateway::snapshot::{
    Allowlisted, DataFreshness, IntentConflict, MarketSession, Side, StrategyPromotion,
};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

const CONTRACT_BUNDLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/kr-etf/contract"
);

struct RawPublicationFixture {
    bundle: PublicationBundle,
    _raw_root: tempfile::TempDir,
}

fn raw_publication(target_date: &str, retrieved_at: &str) -> RawPublicationFixture {
    let raw_root = tempfile::tempdir().expect("raw fixture root");
    let store = RawStore::new(raw_root.path());
    let provider = KrxProvider::synthetic(
        RecordedBundle::open(CONTRACT_BUNDLE).expect("canonical recorded bundle"),
    );
    let outcome = ingest_bundle(
        &store,
        &provider,
        &IngestRequest::new(
            MARKET_KR.to_owned(),
            TradingDate::parse(target_date).expect("target date"),
            UtcTimestamp::parse_rfc3339(retrieved_at).expect("retrieved_at"),
        ),
        None,
    )
    .expect("persist canonical Raw fixture");
    let manifest = store
        .read_manifest("krx", "kr")
        .expect("read Raw manifest")
        .into_iter()
        .find(|entry| entry.batch_id == outcome.batch_id)
        .expect("persisted Raw manifest");
    let bundle = PublicationBundle::from_raw(&store, &manifest).expect("verified publication");
    RawPublicationFixture {
        bundle,
        _raw_root: raw_root,
    }
}

async fn research_writer_pool(h: &Harness) -> sqlx::PgPool {
    let options = h
        .app_url
        .parse::<sqlx::postgres::PgConnectOptions>()
        .expect("app URL parses")
        .username("research_writer");
    PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .expect("research writer connects")
}

fn utc_epoch(timestamp: &str) -> i64 {
    timestamp
        .parse::<chrono::DateTime<chrono::Utc>>()
        .expect("UTC timestamp")
        .timestamp()
}

async fn seeded_account(h: &Harness) -> Uuid {
    let pool = actor_pool(&h.app_url, &h.owner.user_id.to_string(), 2).await;
    let account: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (owner_user_id, account_type, name, currency) \
         VALUES ($1, 'LIVE', 'seam-live', 'KRW') \
         ON CONFLICT (owner_user_id, name) DO UPDATE SET currency = EXCLUDED.currency \
         RETURNING id",
    )
    .bind(h.owner.user_id)
    .fetch_one(&pool)
    .await
    .expect("account");

    // The AUTHORITY, not a snapshot of it: one opening deposit, so the running
    // balance and the replayed events agree.
    sqlx::query(
        "INSERT INTO cash_ledger \
         (account_id, owner_user_id, seq, event_type, amount, balance, currency) \
         VALUES ($1, $2, 1, 'DEPOSIT', 4000000.0000, 4000000.0000, 'KRW') \
         ON CONFLICT (account_id, seq) DO NOTHING",
    )
    .bind(account)
    .bind(h.owner.user_id)
    .execute(&pool)
    .await
    .expect("ledger");

    sqlx::query(
        "INSERT INTO risk_limits (version, max_symbol_weight_bp, max_order_value, \
         max_daily_order_value, max_daily_loss, max_data_age_secs) \
         VALUES ('risk-limits-seam', 3000, 1000000, 5000000, 500000, 300) \
         ON CONFLICT (version) DO NOTHING",
    )
    .execute(&h.owner_pool)
    .await
    .expect("limits");

    account
}

fn order(account: Uuid, side: Side, qty: &str, price: &str) -> GateOrder {
    GateOrder {
        intent_ref: format!("intent-seam-{qty}-{price}"),
        account_id: account,
        instrument_id: "069500.KRX".into(),
        side,
        quantity: Quantity::parse(qty).expect("quantity"),
        price: Some(Price::parse(price).expect("price")),
        correlation_id: "corr-seam".into(),
    }
}

/// THE regression lock: the gate is asked about the order that was submitted.
///
/// A 9,700-unit sell at 10,250 shares nothing with the old fixture's 10-unit
/// buy at 7,250 except the instrument. Before this was fixed, every assertion
/// below failed -- and nothing in the repository noticed.
#[tokio::test]
async fn the_snapshot_describes_the_submitted_order() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let account = seeded_account(&h).await;
    let recon = ReconciliationRepo::new(h.app_pool.clone(), h.owner.actor(), h.owner.user_id);
    let submitted = order(account, Side::Sell, "9700", "10250");

    let snap = for_submission(
        &h.app_pool,
        &h.owner.actor(),
        &recon,
        None,
        Some(false),
        &submitted,
        1_800_000_000,
    )
    .await
    .expect("snapshot builds");

    assert_eq!(snap.intent.side, Side::Sell, "a SELL was checked as a BUY");
    assert_eq!(snap.intent.quantity, submitted.quantity);
    assert_eq!(snap.intent.price, submitted.price);
    assert_eq!(snap.intent.instrument_id, submitted.instrument_id);
    assert_eq!(snap.intent.intent_ref, submitted.intent_ref);
    assert_eq!(snap.intent.account_id, account.to_string());

    // The account the checks divide by is this account, not a fabricated one,
    // and its cash comes from the ledger rather than a stored column. The
    // fixture claimed 1,000,000 equity and 500,000 cash for every account
    // that ever submitted an order.
    assert_eq!(
        snap.account.available_cash.amount().to_string(),
        "4000000.0000"
    );
    assert_eq!(snap.account.equity.amount().to_string(), "4000000.0000");

    h.teardown().await;
}

/// A ledger that contradicts itself refuses to produce a snapshot.
///
/// `cash_ledger` carries both a running `balance` and the `amount` of each
/// event, so the balance can be recomputed from the events and compared. The
/// gate check named "ledger-reconciliation" never does this -- it runs
/// `portfolio-model` in memory and never touches Postgres -- so before this,
/// no test in the repository could tell a correct stored balance from a wrong
/// one.
///
/// This is the check made to FIRE, which is the only way to know it can.
#[tokio::test]
async fn a_ledger_that_disagrees_with_itself_denies_the_order() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let account = seeded_account(&h).await;
    let pool = actor_pool(&h.app_url, &h.owner.user_id.to_string(), 2).await;

    // A second event whose running balance does not account for its own
    // amount: replaying the events gives 4,500,000, the stored balance says
    // 4,000,000. One of them is wrong and nothing here can tell which.
    sqlx::query(
        "INSERT INTO cash_ledger \
         (account_id, owner_user_id, seq, event_type, amount, balance, currency) \
         VALUES ($1, $2, 2, 'DEPOSIT', 500000.0000, 4000000.0000, 'KRW')",
    )
    .bind(account)
    .bind(h.owner.user_id)
    .execute(&pool)
    .await
    .expect("divergent ledger row");

    let recon = ReconciliationRepo::new(h.app_pool.clone(), h.owner.actor(), h.owner.user_id);
    let err = for_submission(
        &h.app_pool,
        &h.owner.actor(),
        &recon,
        None,
        Some(false),
        &order(account, Side::Buy, "10", "7250"),
        1_800_000_000,
    )
    .await
    .expect_err("a self-contradicting ledger must not produce a snapshot");

    let message = format!("{err:?}");
    assert!(
        message.contains("disagrees with itself"),
        "the refusal must name what diverged: {message}"
    );

    h.teardown().await;
}

/// Inputs without a calendar or batch stay `Unknown` and deny. The intent
/// table, however, is an existing source: an empty account has a concrete
/// `None` answer rather than an unavailable one.
///
/// `checks.rs`: "Every `Unknown` input denies with `InputUnavailable`. §16
/// requires missing inputs to deny." Calendar and batch rows remain required;
/// conflict detection can answer `None` from an empty actor-scoped query.
#[tokio::test]
async fn inputs_without_a_source_stay_closed() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let account = seeded_account(&h).await;
    let recon = ReconciliationRepo::new(h.app_pool.clone(), h.owner.actor(), h.owner.user_id);

    let snap = for_submission(
        &h.app_pool,
        &h.owner.actor(),
        &recon,
        None,
        Some(false),
        &order(account, Side::Buy, "10", "7250"),
        1_800_000_000,
    )
    .await
    .expect("snapshot builds");

    assert_eq!(snap.market_session, MarketSession::Unknown);
    assert_eq!(snap.data_freshness, DataFreshness::Unknown);
    assert_eq!(snap.conflict, IntentConflict::None);

    // Which means even an order with fully wired promotion/allowlist is still
    // refused: the two missing metadata sources are enough on their own.
    let decision = risk_gateway::evaluate(
        &snap,
        &limits_for(&h.app_pool, &h.owner.actor(), h.owner.user_id)
            .await
            .expect("limits"),
    );
    assert!(
        !decision.is_approved(),
        "an order with unsourced inputs must not be approved"
    );

    h.teardown().await;
}

/// The three Phase 3 inputs come from the shared metadata and the actor's
/// intent table, not from a test fixture.
#[tokio::test]
async fn wired_inputs_read_calendar_batch_and_open_intents() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let account = seeded_account(&h).await;
    h.seed_shared(
        "INSERT INTO trading_calendars \
         (exchange, session_date, session_type, timezone, source, source_version) \
         VALUES ('KRX', '2027-01-15', 'TRADING', 'Asia/Seoul', 'qa', 'v1') \
         ON CONFLICT (exchange, session_date) DO UPDATE SET session_type = EXCLUDED.session_type, \
             timezone = EXCLUDED.timezone",
    )
    .await;
    h.seed_shared(
        "INSERT INTO data_batches \
         (provider, market, batch_date, kind, storage_path, content_sha256, bytes_size, retrieved_at) \
         VALUES ('KRX', 'KR', '2027-01-15', 'EOD', 'qa/eod', repeat('a', 64), 1, \
                 '2027-01-15 00:29:00+00')",
    )
    .await;

    let recon = ReconciliationRepo::new(h.app_pool.clone(), h.owner.actor(), h.owner.user_id);
    let snap = for_submission(
        &h.app_pool,
        &h.owner.actor(),
        &recon,
        None,
        Some(false),
        &order(account, Side::Buy, "10", "7250"),
        1_799_973_000, // 2027-01-15 09:30:00 KST
    )
    .await
    .expect("snapshot builds");

    assert_eq!(snap.market_session, MarketSession::Open);
    assert_eq!(snap.data_freshness, DataFreshness::Age(60));
    assert_eq!(snap.conflict, IntentConflict::None);
    h.teardown().await;
}

/// Raw provider evidence becomes the exact shared metadata read by the Live
/// risk snapshot. There are no test inserts into either metadata table here:
/// the production research-writer sink owns the publication boundary.
#[tokio::test]
async fn risk_snapshot_consumes_metadata_published_from_canonical_raw() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let account = seeded_account(&h).await;
    let fixture = raw_publication("2020-01-31", "2020-01-31T00:29:00Z");
    assert_eq!(fixture.bundle.files[0].kind, DataBatchKind::Eod);

    let writer = research_writer_pool(&h).await;
    let sink = PostgresPublicationSink::new(writer.clone());
    assert_eq!(
        sink.publish(&fixture.bundle)
            .await
            .expect("publish metadata"),
        PublishOutcome::Published
    );

    let recon = ReconciliationRepo::new(h.app_pool.clone(), h.owner.actor(), h.owner.user_id);
    let snap = for_submission(
        &h.app_pool,
        &h.owner.actor(),
        &recon,
        None,
        Some(false),
        &order(account, Side::Buy, "10", "7250"),
        utc_epoch("2020-01-31T00:30:00Z"),
    )
    .await
    .expect("snapshot builds from published metadata");

    assert_eq!(snap.market_session, MarketSession::Open);
    assert_eq!(snap.data_freshness, DataFreshness::Age(60));

    writer.close().await;
    h.teardown().await;
}

/// Publishing an old Raw batch today is a backfill, not current market data.
/// Freshness is bounded by the end of the batch's Korean civil date even when
/// the immutable evidence was retrieved years later.
#[tokio::test]
async fn historical_backfill_published_now_remains_stale() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let account = seeded_account(&h).await;
    let now = "2026-08-10T00:00:00Z";
    let fixture = raw_publication("2020-01-31", now);
    let writer = research_writer_pool(&h).await;
    PostgresPublicationSink::new(writer.clone())
        .publish(&fixture.bundle)
        .await
        .expect("publish historical backfill");

    let recon = ReconciliationRepo::new(h.app_pool.clone(), h.owner.actor(), h.owner.user_id);
    let snap = for_submission(
        &h.app_pool,
        &h.owner.actor(),
        &recon,
        None,
        Some(false),
        &order(account, Side::Buy, "10", "7250"),
        utc_epoch(now),
    )
    .await
    .expect("snapshot builds from historical backfill");

    assert!(
        matches!(snap.data_freshness, DataFreshness::Age(age) if age > 365 * 24 * 60 * 60),
        "historical backfill must stay stale, got {:?}",
        snap.data_freshness
    );
    writer.close().await;
    h.teardown().await;
}

/// A publication claiming a future Korean batch date is inapplicable at the
/// decision instant. It cannot supersede the latest true EOD on or before the
/// current Korean civil date.
#[tokio::test]
async fn future_batch_date_never_supersedes_latest_applicable_eod() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let account = seeded_account(&h).await;
    h.seed_shared(
        "INSERT INTO data_batches \
         (provider, market, batch_date, kind, storage_path, content_sha256, bytes_size, retrieved_at) \
         VALUES \
         ('KRX','KR','2026-08-09','EOD','qa/applicable',repeat('a',64),1,'2026-08-09 00:00:00+00'), \
         ('KRX','KR','2026-08-11','EOD','qa/future-date',repeat('b',64),1,'2026-08-10 00:00:00+00')",
    )
    .await;

    let recon = ReconciliationRepo::new(h.app_pool.clone(), h.owner.actor(), h.owner.user_id);
    let snap = for_submission(
        &h.app_pool,
        &h.owner.actor(),
        &recon,
        None,
        Some(false),
        &order(account, Side::Buy, "10", "7250"),
        utc_epoch("2026-08-10T00:00:00Z"),
    )
    .await
    .expect("snapshot builds using applicable EOD");

    assert_eq!(snap.data_freshness, DataFreshness::Age(86_400));
    h.teardown().await;
}

/// A successful collection without a target-date bar is publication evidence,
/// but it is not EOD market data. Even when it is newer, `EOD_UNAVAILABLE`
/// must neither invent freshness nor supersede the latest real EOD batch.
#[tokio::test]
async fn eod_unavailable_publication_never_counts_as_fresh_eod() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let account = seeded_account(&h).await;
    let writer = research_writer_pool(&h).await;
    let sink = PostgresPublicationSink::new(writer.clone());
    let unavailable = raw_publication("2020-02-03", "2020-02-03T00:29:00Z");
    assert_eq!(
        unavailable.bundle.files[0].kind,
        DataBatchKind::EodUnavailable
    );
    sink.publish(&unavailable.bundle)
        .await
        .expect("publish unavailable evidence");

    let recon = ReconciliationRepo::new(h.app_pool.clone(), h.owner.actor(), h.owner.user_id);
    let gate_order = order(account, Side::Buy, "10", "7250");
    let now = utc_epoch("2020-02-03T00:30:00Z");
    let without_eod = for_submission(
        &h.app_pool,
        &h.owner.actor(),
        &recon,
        None,
        Some(false),
        &gate_order,
        now,
    )
    .await
    .expect("snapshot builds without EOD");
    assert_eq!(without_eod.data_freshness, DataFreshness::Unknown);

    let real_eod = raw_publication("2020-01-31", "2020-01-31T00:29:00Z");
    assert_eq!(real_eod.bundle.files[0].kind, DataBatchKind::Eod);
    sink.publish(&real_eod.bundle)
        .await
        .expect("publish prior real EOD");

    let with_prior_eod = for_submission(
        &h.app_pool,
        &h.owner.actor(),
        &recon,
        None,
        Some(false),
        &gate_order,
        now,
    )
    .await
    .expect("snapshot builds with prior EOD");
    assert_eq!(with_prior_eod.data_freshness, DataFreshness::Age(259_260));

    writer.close().await;
    h.teardown().await;
}

/// A known closed session and an old batch are concrete answers, not missing
/// inputs. The risk evaluator must therefore identify the data-age denial.
#[tokio::test]
async fn closed_session_and_stale_batch_are_reported_by_name() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let account = seeded_account(&h).await;
    h.seed_shared(
        "INSERT INTO trading_calendars \
         (exchange, session_date, session_type, timezone, source, source_version) \
         VALUES ('KRX', '2027-01-15', 'CLOSED', 'Asia/Seoul', 'qa', 'v1') \
         ON CONFLICT (exchange, session_date) DO UPDATE SET session_type = EXCLUDED.session_type",
    )
    .await;
    h.seed_shared(
        "INSERT INTO data_batches \
         (provider, market, batch_date, kind, storage_path, content_sha256, bytes_size, retrieved_at) \
         VALUES ('KRX', 'KR', '2027-01-14', 'EOD', 'qa/eod-old', repeat('b', 64), 1, \
                 '2027-01-14 00:00:00+00')",
    )
    .await;

    let recon = ReconciliationRepo::new(h.app_pool.clone(), h.owner.actor(), h.owner.user_id);
    let snap = for_submission(
        &h.app_pool,
        &h.owner.actor(),
        &recon,
        None,
        Some(false),
        &order(account, Side::Buy, "10", "7250"),
        1_799_996_400, // 2027-01-15 16:00:00 KST
    )
    .await
    .expect("snapshot builds");

    assert_eq!(snap.market_session, MarketSession::Closed);
    assert_eq!(snap.data_freshness, DataFreshness::Age(111_600));
    let decision = risk_gateway::evaluate(
        &snap,
        &limits_for(&h.app_pool, &h.owner.actor(), h.owner.user_id)
            .await
            .expect("limits"),
    );
    assert!(!decision.is_approved());
    assert_eq!(
        decision.reason,
        Some(risk_gateway::DenyReason::MarketSessionClosed)
    );
    h.teardown().await;
}

/// An active intent for the same account/instrument is a concrete conflict;
/// an intent owned by another actor is invisible through FORCE RLS.
#[tokio::test]
async fn active_intent_conflict_is_tenant_scoped() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let account = seeded_account(&h).await;
    let owner_pool = actor_pool(&h.app_url, &h.owner.user_id.to_string(), 2).await;
    sqlx::query(
        "INSERT INTO order_intents \
         (intent_ref, owner_user_id, account_id, instrument_id, side, quantity, price, correlation_id, state) \
         VALUES ('oi-active-seam', $1, $2, '069500.KRX', 'BUY', 1, 7250, 'corr-active', 'RISK_APPROVED')",
    )
    .bind(h.owner.user_id)
    .bind(account)
    .execute(&owner_pool)
    .await
    .expect("active intent");

    let recon = ReconciliationRepo::new(h.app_pool.clone(), h.owner.actor(), h.owner.user_id);
    let snap = for_submission(
        &h.app_pool,
        &h.owner.actor(),
        &recon,
        None,
        Some(false),
        &order(account, Side::Buy, "10", "7250"),
        1_799_973_000,
    )
    .await
    .expect("snapshot builds");
    assert_eq!(snap.conflict, IntentConflict::Conflicting);
    h.teardown().await;
}

/// A calendar row with an unsupported timezone cannot be used to infer an
/// open session and must remain fail-closed.
#[tokio::test]
async fn unsupported_calendar_timezone_stays_unknown() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let account = seeded_account(&h).await;
    h.seed_shared(
        "INSERT INTO trading_calendars \
         (exchange, session_date, session_type, timezone, source, source_version) \
         VALUES ('KRX', '2027-01-15', 'TRADING', 'UTC', 'qa', 'v1')",
    )
    .await;
    h.seed_shared(
        "INSERT INTO data_batches \
         (provider, market, batch_date, kind, storage_path, content_sha256, bytes_size, retrieved_at) \
         VALUES ('KRX', 'KR', '2027-01-15', 'EOD', 'qa/eod-tz', repeat('c', 64), 1, \
                 '2027-01-15 00:29:00+00')",
    )
    .await;
    let recon = ReconciliationRepo::new(h.app_pool.clone(), h.owner.actor(), h.owner.user_id);
    let snap = for_submission(
        &h.app_pool,
        &h.owner.actor(),
        &recon,
        None,
        Some(false),
        &order(account, Side::Buy, "10", "7250"),
        1_799_973_000,
    )
    .await
    .expect("snapshot builds");
    assert_eq!(snap.market_session, MarketSession::Unknown);
    h.teardown().await;
}

/// An account bound to an active, live-candidate strategy is NOT `Unknown`.
///
/// The lookup succeeds and answers "yes" -- `LiveCandidate`, not a guess.
#[tokio::test]
async fn a_bound_account_trading_an_allowed_instrument_is_promoted_and_allowed() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let account = seeded_account(&h).await;
    let owner = &h.owner;

    let config_resp = h
        .send(
            "POST",
            "/api/v1/strategies/buy_and_hold/configs",
            Some(owner),
            true,
            Some("test-rid-promo"),
            Some("seam-promo-cfg"),
            Some(serde_json::json!({
                "strategy_version": "1.0.0",
                "config": { "lookback": 200 },
                "is_active": true,
            })),
        )
        .await;
    assert_eq!(config_resp.status(), axum::http::StatusCode::CREATED);
    let config_id: Uuid = Harness::body_json(config_resp)
        .await
        .get("id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .expect("config id");

    h.state()
        .accounts()
        .bind_strategy(
            &owner.actor(),
            account,
            config_id,
            "buy_and_hold",
            "1.0.0",
            false,
        )
        .await
        .expect("binding succeeds");

    let recon = ReconciliationRepo::new(h.app_pool.clone(), owner.actor(), owner.user_id);
    let snap = for_submission(
        &h.app_pool,
        &owner.actor(),
        &recon,
        None,
        Some(false),
        // 069500.KRX is a member of the fixed universe.
        &order(account, Side::Buy, "10", "7250"),
        1_800_000_000,
    )
    .await
    .expect("snapshot builds");

    assert_eq!(snap.strategy_promotion, StrategyPromotion::LiveCandidate);
    assert_eq!(snap.instrument_allowed, Allowlisted::Allowed);

    h.teardown().await;
}

/// An account with NO active binding is `NotPromoted`, and an instrument
/// outside the fixed universe is `NotAllowed` -- both answers, not `Unknown`.
#[tokio::test]
async fn an_unbound_account_is_not_promoted_and_an_outside_instrument_is_not_allowed() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let account = seeded_account(&h).await;
    let recon = ReconciliationRepo::new(h.app_pool.clone(), h.owner.actor(), h.owner.user_id);

    let mut unlisted = order(account, Side::Buy, "10", "7250");
    unlisted.instrument_id = "005930.KRX".to_owned(); // a real code, not a member.

    let snap = for_submission(
        &h.app_pool,
        &h.owner.actor(),
        &recon,
        None,
        Some(false),
        &unlisted,
        1_800_000_000,
    )
    .await
    .expect("snapshot builds");

    assert_eq!(
        snap.strategy_promotion,
        StrategyPromotion::NotPromoted,
        "no binding is a real answer, not an unknown"
    );
    assert_eq!(
        snap.instrument_allowed,
        Allowlisted::NotAllowed,
        "a real instrument outside the fixed universe is refused by name"
    );

    h.teardown().await;
}

/// The limits come from `risk_limits`, not from the test fixture.
#[tokio::test]
async fn limits_come_from_the_configured_table() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    seeded_account(&h).await;

    let limits = limits_for(&h.app_pool, &h.owner.actor(), h.owner.user_id)
        .await
        .expect("configured limits");
    assert_eq!(
        limits.version, "risk-limits-seam",
        "the version stamped on every recorded decision must name a row somebody configured"
    );
    assert_eq!(limits.max_symbol_weight_bp, 3000);

    h.teardown().await;
}

/// An unrecognised side has no third answer.
///
/// The old code resolved this with `eq_ignore_ascii_case("SELL")` and a bare
/// `else`, so each of these became a BUY.
#[test]
fn only_buy_and_sell_are_sides() {
    assert_eq!(parse_side("BUY"), Some(Side::Buy));
    assert_eq!(parse_side("sell"), Some(Side::Sell));
    assert_eq!(parse_side(" SELL "), Some(Side::Sell));
    for bad in ["SEL", "", "매도", "B", "SELLL", "0"] {
        assert_eq!(parse_side(bad), None, "{bad:?} must not resolve to a side");
    }
    assert_eq!(side_str(Side::Buy), "BUY");
    assert_eq!(side_str(Side::Sell), "SELL");
}
