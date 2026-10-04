#![allow(dead_code)]

use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::time::Duration;

use api_server::http::api_router;
use api_server::http::owner_market_stream_config::{MarketStreamReadConfig, MarketStreamReadPins};
use api_server::http::state::{
    ApiConfig, ApiState, OwnerBetaAccessMode, OwnerBetaEquitySignalsMode, OwnerBetaPaperMode,
    OwnerBetaPriceInputMode, OwnerIntradayQuoteReadConfig,
};
use auth::entitlement::Role;
use auth::sessions::cookie;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderName, HeaderValue, Method, Request, StatusCode, header};
use axum::response::Response;
use chrono::{DateTime, Utc};
use futures_util::FutureExt;
use http_body_util::BodyExt;
use job_queue::owner_equity_v2::StreamIdentity;
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tokio::time::{Instant, sleep, timeout, timeout_at};
use tower::ServiceExt;
use uuid::Uuid;

use super::boundary_support::{self as boundary, DisposableDatabase, Fixture};

type CaseResult<T = ()> = Result<T, &'static str>;
type CaseFuture<'a> = Pin<Box<dyn Future<Output = CaseResult> + 'a>>;

const SETUP_BUDGET: Duration = Duration::from_secs(45);
const OPERATIONS_BUDGET: Duration = Duration::from_secs(75);
const CLEANUP_BUDGET: Duration = Duration::from_secs(30);
const MAX_BODY: usize = 64 * 1024;
const STREAM_PREFIX: &str = "/api/v1/research/owner-beta/equity-universe-v2";
const OWNER_CONTRACT_HASH: &str = boundary::NETWORK_HASH;

#[derive(Clone, Copy)]
pub enum Case {
    BindingHelper,
    LeaseAuthentication,
    InitialSnapshot,
    ReplacementAndRevocation,
    ConsumerCapacity,
    DisabledAndRevoked,
}

impl Case {
    fn name(self) -> &'static str {
        match self {
            Self::BindingHelper => "binding_helper_is_app_only_and_reversible",
            Self::LeaseAuthentication => {
                "leases_authenticate_and_fence_consumer_session_and_sequence"
            }
            Self::InitialSnapshot => "sse_initial_snapshot_is_private_and_read_only",
            Self::ReplacementAndRevocation => {
                "sse_replacement_and_revocation_clear_old_memberships"
            }
            Self::ConsumerCapacity => "twenty_consumers_are_bounded_and_drops_release_capacity",
            Self::DisabledAndRevoked => "disabled_or_revoked_binding_denies_demand_and_stream",
        }
    }

    fn future<'a>(self, context: &'a mut CaseContext) -> CaseFuture<'a> {
        match self {
            Self::BindingHelper => Box::pin(binding_helper_case(context)),
            Self::LeaseAuthentication => Box::pin(lease_authentication_case(context)),
            Self::InitialSnapshot => Box::pin(initial_snapshot_case(context)),
            Self::ReplacementAndRevocation => Box::pin(replacement_and_revocation_case(context)),
            Self::ConsumerCapacity => Box::pin(consumer_capacity_case(context)),
            Self::DisabledAndRevoked => Box::pin(disabled_and_revoked_case(context)),
        }
    }
}

pub async fn run(case: Case) {
    let started = Instant::now();
    let setup_deadline = started + SETUP_BUDGET;
    let operation_deadline = setup_deadline + OPERATIONS_BUDGET;
    let cleanup_deadline = operation_deadline + CLEANUP_BUDGET;
    let database = match DisposableDatabase::create_until(setup_deadline, cleanup_deadline).await {
        Ok(database) => database,
        Err(_) => panic!("WP-4-H {}: bounded C2 fixture setup failed", case.name()),
    };

    let setup = AssertUnwindSafe(async {
        let fixture = boundary::seed_fixture(&database)
            .await
            .map_err(|_| "bounded C2 stream fixture seeding failed")?;
        let users = seed_http_sessions(&database, &fixture)
            .await
            .map_err(|_| "synthetic HTTP session fixture seeding failed")?;
        Ok::<_, &'static str>((fixture, users))
    })
    .catch_unwind();
    let setup_result = timeout_at(setup_deadline, setup).await;
    let (fixture, users) = match setup_result {
        Ok(Ok(Ok(value))) => value,
        _ => {
            let cleanup_ok = database.cleanup_until(cleanup_deadline).await.is_ok();
            if cleanup_ok {
                panic!(
                    "WP-4-H {}: bounded fixture setup did not complete",
                    case.name()
                );
            }
            panic!(
                "WP-4-H {}: setup and bounded fixture cleanup failed",
                case.name()
            );
        }
    };

    let audit_pool = match timeout_at(
        setup_deadline,
        role_pool(&database, "audit_writer", 8, "wp4h-audit"),
    )
    .await
    {
        Ok(Ok(pool)) => pool,
        _ => {
            let cleanup_ok = database.cleanup_until(cleanup_deadline).await.is_ok();
            if cleanup_ok {
                panic!("WP-4-H {}: audit fixture pool setup failed", case.name());
            }
            panic!(
                "WP-4-H {}: audit pool setup and fixture cleanup failed",
                case.name()
            );
        }
    };

    let mut context = CaseContext {
        database: Some(database),
        fixture,
        users,
        audit_pool: audit_pool.clone(),
        extra_pools: vec![audit_pool],
        routers: Vec::new(),
        operation_deadline,
    };
    let result = timeout_at(
        operation_deadline,
        AssertUnwindSafe(case.future(&mut context)).catch_unwind(),
    )
    .await;

    // Drop every router/state clone before closing the additional role pools.
    context.routers.clear();
    let mut cleanup_ok = true;
    for pool in context.extra_pools.drain(..) {
        if timeout_at(cleanup_deadline, pool.close()).await.is_err() {
            cleanup_ok = false;
        }
    }
    drop(context.audit_pool);
    let database_cleanup_ok = match context.database.take() {
        Some(database) => database.cleanup_until(cleanup_deadline).await.is_ok(),
        None => false,
    };
    cleanup_ok &= database_cleanup_ok;

    match result {
        Ok(Ok(Ok(()))) if cleanup_ok => {}
        Ok(Ok(Ok(()))) => panic!("WP-4-H {}: bounded cleanup failed", case.name()),
        Ok(Ok(Err(primary))) if cleanup_ok => {
            panic!("WP-4-H {}: {}", case.name(), primary)
        }
        Ok(Ok(Err(primary))) => {
            panic!(
                "WP-4-H {}: {}; bounded cleanup also failed",
                case.name(),
                primary
            )
        }
        Ok(Err(_)) if cleanup_ok => {
            panic!(
                "WP-4-H {}: assertion path unwound after cleanup",
                case.name()
            )
        }
        Ok(Err(_)) => {
            panic!(
                "WP-4-H {}: assertion path and bounded cleanup failed",
                case.name()
            )
        }
        Err(_) if cleanup_ok => {
            panic!(
                "WP-4-H {}: operation deadline expired; owned cleanup completed",
                case.name()
            )
        }
        Err(_) => {
            panic!(
                "WP-4-H {}: operation deadline and bounded cleanup failed",
                case.name()
            )
        }
    }
}

#[derive(Clone)]
pub(super) struct TestSession {
    pub(super) user_id: Uuid,
    pub(super) cookie_value: String,
    pub(super) csrf_token: String,
    pub(super) session_hash: String,
}

pub(super) struct TestSessions {
    pub(super) owner: TestSession,
    secondary_owner: TestSession,
    other_owner: TestSession,
    member: TestSession,
}

struct CaseContext {
    database: Option<DisposableDatabase>,
    fixture: Fixture,
    users: TestSessions,
    audit_pool: PgPool,
    extra_pools: Vec<PgPool>,
    routers: Vec<axum::Router>,
    operation_deadline: Instant,
}

impl CaseContext {
    fn db(&self) -> &DisposableDatabase {
        self.database
            .as_ref()
            .expect("fixture is owned until cleanup")
    }

    fn owner_config(&self) -> CaseResult<MarketStreamReadConfig> {
        Ok(MarketStreamReadConfig::OwnerOnly(
            MarketStreamReadPins::new(
                "https://quotes.example".to_owned(),
                self.fixture.credential_slot_id,
                self.fixture.grant_id,
                OWNER_CONTRACT_HASH.to_owned(),
                None,
            )
            .map_err(|_| "owner deployment pins rejected")?,
        ))
    }

    async fn other_owner_config(&self) -> CaseResult<MarketStreamReadConfig> {
        let (slot, grant): (Uuid, Uuid) = sqlx::query_as(
            "SELECT credential_slot_id, id
               FROM public.owner_market_stream_grants
              WHERE owner_user_id = $1 AND state = 'ACTIVE'",
        )
        .bind(self.fixture.other_owner_user_id)
        .fetch_one(&self.db().migration_owner)
        .await
        .map_err(|_| "other-owner grant fixture lookup failed")?;
        Ok(MarketStreamReadConfig::OwnerOnly(
            MarketStreamReadPins::new(
                "https://quotes.example".to_owned(),
                slot,
                grant,
                OWNER_CONTRACT_HASH.to_owned(),
                None,
            )
            .map_err(|_| "other-owner deployment pins rejected")?,
        ))
    }

    fn api_config(&self, stream: MarketStreamReadConfig) -> ApiConfig {
        ApiConfig {
            cursor_secret: [0x44; 32],
            max_jobs_per_owner: 10,
            recommendation_dataset: job_queue::recommendation::input::DatasetPin {
                id: Uuid::from_u128(0x4401),
                dataset_id: "wp4h-synthetic".to_owned(),
                version: "fixture".to_owned(),
                curated_version: 1,
                manifest_sha256: "0".repeat(64),
            },
            // The selected routes use only the pools supplied to from_pools.
            // This inert value is not resolved or opened by the fixture.
            db_url: "postgres://unused:unused@fixture.invalid/unused".to_owned(),
            step_up_max_auth_age_secs: 900,
            artifact_root: std::path::PathBuf::from("/unused"),
            seoul_today: api_server::http::state::system_seoul_today,
            candidate_eod_ready: api_server::http::state::system_candidate_eod_ready,
            code_commit: boundary::CODE_COMMIT.to_owned(),
            owner_beta_access: OwnerBetaAccessMode::Disabled,
            owner_beta_paper: OwnerBetaPaperMode::Disabled,
            owner_beta_price_input: OwnerBetaPriceInputMode::Disabled,
            owner_beta_equity_signals: OwnerBetaEquitySignalsMode::Disabled,
            stock_price_beta_artifact_root: std::path::PathBuf::from("/unused"),
            owner_equity_v2_pins: None,
            owner_equity_v2_api_artifact_root: None,
            owner_intraday_quotes: OwnerIntradayQuoteReadConfig::Disabled,
            owner_market_stream: stream,
            intraday_now: api_server::http::state::system_intraday_now,
        }
    }

    async fn router(
        &mut self,
        stream: MarketStreamReadConfig,
        app_pool: Option<PgPool>,
    ) -> CaseResult<axum::Router> {
        let state = ApiState::from_pools(
            self.api_config(stream),
            app_pool.unwrap_or_else(|| self.db().app.clone()),
            self.db().admin.clone(),
            self.audit_pool.clone(),
        )
        .await
        .map_err(|_| "API state construction failed")?;
        let router = api_router(state);
        self.routers.push(router.clone());
        Ok(router)
    }

    async fn app_pool(
        &mut self,
        max_connections: u32,
        application_name: &str,
    ) -> CaseResult<PgPool> {
        let options = self
            .db()
            .app
            .connect_options()
            .as_ref()
            .clone()
            .application_name(application_name);
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(Duration::from_secs(5))
            .connect_with(options)
            .await
            .map_err(|_| "owned app pool construction failed")?;
        self.extra_pools.push(pool.clone());
        Ok(pool)
    }

    async fn other_owner_route(&mut self) -> CaseResult<axum::Router> {
        let pins = self.other_owner_config().await?;
        self.router(pins, None).await
    }
}

pub(super) async fn seed_http_sessions(
    database: &DisposableDatabase,
    fixture: &Fixture,
) -> Result<TestSessions, sqlx::Error> {
    let owner = synthetic_session(fixture.owner_user_id, "owner-primary");
    let secondary = synthetic_session(fixture.owner_user_id, "owner-secondary");
    let other = synthetic_session(fixture.other_owner_user_id, "owner-b");

    replace_fixture_session(
        database,
        fixture.owner_user_id,
        &fixture.owner_session_hash,
        &owner,
    )
    .await?;
    replace_fixture_session(
        database,
        fixture.owner_user_id,
        &fixture.secondary_owner_session_hash,
        &secondary,
    )
    .await?;
    replace_fixture_session(
        database,
        fixture.other_owner_user_id,
        &fixture.other_session_hash,
        &other,
    )
    .await?;

    let member_id = Uuid::new_v4();
    let member = synthetic_session(member_id, "member");
    let mut tx = database.migration_owner.begin().await?;
    set_actor(&mut tx, fixture.owner_user_id).await?;
    sqlx::query(
        "INSERT INTO public.users (id, issuer, subject, email, display_name)
         VALUES ($1, 'wp4h-synthetic', $2, $3, 'synthetic member')",
    )
    .bind(member_id)
    .bind(member_id.to_string())
    .bind(format!("{}@invalid.test", member_id))
    .execute(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO public.user_roles (user_id, role_id) VALUES ($1, 'member')")
        .bind(member_id)
        .execute(&mut *tx)
        .await?;
    set_actor(&mut tx, member_id).await?;
    sqlx::query(
        "INSERT INTO public.web_sessions
            (id, user_id, session_hash, csrf_hash, expires_at, created_at)
         VALUES ($1, $2, $3, $4, pg_catalog.clock_timestamp() + interval '1 hour',
                 pg_catalog.clock_timestamp() - interval '1 minute')",
    )
    .bind(Uuid::new_v4())
    .bind(member_id)
    .bind(&member.session_hash)
    .bind(auth::csrf::hash_token(&member.csrf_token))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(TestSessions {
        owner,
        secondary_owner: secondary,
        other_owner: other,
        member,
    })
}

fn synthetic_session(user_id: Uuid, label: &str) -> TestSession {
    let cookie_value = format!("wp4h-{label}-{}", Uuid::new_v4());
    let csrf_token = format!("wp4h-csrf-{label}-{}", Uuid::new_v4());
    TestSession {
        user_id,
        session_hash: cookie::hash(&cookie_value),
        cookie_value,
        csrf_token,
    }
}

async fn replace_fixture_session(
    database: &DisposableDatabase,
    owner_user_id: Uuid,
    old_hash: &str,
    session: &TestSession,
) -> Result<(), sqlx::Error> {
    let mut tx = database.migration_owner.begin().await?;
    set_actor(&mut tx, owner_user_id).await?;
    sqlx::query(
        "UPDATE public.web_sessions
            SET session_hash = $1, csrf_hash = $2
          WHERE user_id = $3 AND session_hash = $4 AND revoked_at IS NULL",
    )
    .bind(&session.session_hash)
    .bind(auth::csrf::hash_token(&session.csrf_token))
    .bind(owner_user_id)
    .bind(old_hash)
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

async fn set_actor(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    owner_user_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
        .bind(owner_user_id.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub(super) async fn role_pool(
    database: &DisposableDatabase,
    role: &str,
    max_connections: u32,
    application_name: &str,
) -> Result<PgPool, sqlx::Error> {
    let options = database
        .app
        .connect_options()
        .as_ref()
        .clone()
        .username(role)
        .application_name(application_name);
    PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(Duration::from_secs(5))
        .connect_with(options)
        .await
}

fn request(
    method: Method,
    path: &str,
    session: Option<&TestSession>,
    csrf: bool,
    idempotency_key: Option<&str>,
    origin: Option<&str>,
    body: Vec<u8>,
) -> CaseResult<Request<Body>> {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, "quotes.example")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(origin) = origin {
        builder = builder.header(header::ORIGIN, origin);
    }
    if let Some(session) = session {
        builder = builder.header(
            header::COOKIE,
            format!("{}={}", cookie::NAME, session.cookie_value),
        );
        if csrf {
            builder = builder.header("x-csrf-token", session.csrf_token.as_str());
        }
    }
    if let Some(key) = idempotency_key {
        builder = builder.header("idempotency-key", key);
    }
    builder
        .body(Body::from(body))
        .map_err(|_| "HTTP request construction failed")
}

async fn send(
    context: &CaseContext,
    router: &axum::Router,
    method: Method,
    path: &str,
    session: Option<&TestSession>,
    csrf: bool,
    idempotency_key: Option<&str>,
    origin: Option<&str>,
    body: Vec<u8>,
) -> CaseResult<Response> {
    let request = request(method, path, session, csrf, idempotency_key, origin, body)?;
    timeout_at(context.operation_deadline, router.clone().oneshot(request))
        .await
        .map_err(|_| "HTTP router operation deadline expired")?
        .map_err(|_| "HTTP router returned an unexpected service error")
}

async fn response_json(context: &CaseContext, response: Response) -> CaseResult<Value> {
    let bytes = timeout_at(
        context.operation_deadline,
        to_bytes(response.into_body(), MAX_BODY),
    )
    .await
    .map_err(|_| "HTTP response body deadline expired")?
    .map_err(|_| "HTTP response body was not bounded JSON")?;
    serde_json::from_slice(&bytes).map_err(|_| "HTTP response JSON was invalid")
}

fn lease_body(identities: &[StreamIdentity], consumer_id: Uuid, sequence: u64) -> Value {
    let mut rows = identities
        .iter()
        .map(|identity| {
            json!({
                "membership_id": identity.membership_id.to_string(),
                "instrument_id": identity.instrument_id,
                "generation": identity.generation,
            })
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        left["membership_id"]
            .as_str()
            .cmp(&right["membership_id"].as_str())
    });
    json!({
        "schema_version": 2,
        "consumer_id": consumer_id.to_string(),
        "renewal_sequence": sequence,
        "identities": rows,
    })
}

fn fixture_membership_ids(fixture: &Fixture) -> Vec<Uuid> {
    let mut ids = fixture
        .identities
        .iter()
        .map(|identity| identity.membership_id)
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

fn sorted_identities(mut identities: Vec<StreamIdentity>) -> Vec<StreamIdentity> {
    identities.sort_by_key(|identity| identity.membership_id);
    identities
}

#[derive(Clone, Debug)]
struct LeaseReply {
    lease_id: Uuid,
    consumer_id: Uuid,
    sequence: u64,
}

async fn post_lease(
    context: &CaseContext,
    router: &axum::Router,
    session: &TestSession,
    consumer_id: Uuid,
    sequence: u64,
    identities: &[StreamIdentity],
    idempotency_key: &str,
) -> CaseResult<Response> {
    let bytes = serde_json::to_vec(&lease_body(identities, consumer_id, sequence))
        .map_err(|_| "lease body serialization failed")?;
    send(
        context,
        router,
        Method::POST,
        &format!("{STREAM_PREFIX}/stream-leases"),
        Some(session),
        true,
        Some(idempotency_key),
        Some("https://quotes.example"),
        bytes,
    )
    .await
}

async fn lease_id_from_response(context: &CaseContext, response: Response) -> CaseResult<Uuid> {
    if response.status() != StatusCode::OK && response.status() != StatusCode::CREATED {
        return Err("lease mutation did not return a successful status");
    }
    let value = response_json(context, response).await?;
    value["lease_id"]
        .as_str()
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or("lease response omitted its canonical lease id")
}

async fn create_lease_id(
    context: &CaseContext,
    router: &axum::Router,
    session: &TestSession,
    consumer_id: Uuid,
    identities: &[StreamIdentity],
    idempotency_key: &str,
) -> CaseResult<LeaseReply> {
    let response = post_lease(
        context,
        router,
        session,
        consumer_id,
        0,
        identities,
        idempotency_key,
    )
    .await?;
    let lease_id = lease_id_from_response(context, response).await?;
    Ok(LeaseReply {
        lease_id,
        consumer_id,
        sequence: 0,
    })
}

async fn get_stream(
    context: &CaseContext,
    router: &axum::Router,
    session: &TestSession,
    lease_id: Uuid,
    last_event_id: Option<&str>,
) -> CaseResult<Response> {
    let mut request = request(
        Method::GET,
        &format!("{STREAM_PREFIX}/market-stream?lease_id={lease_id}"),
        Some(session),
        false,
        None,
        Some("https://quotes.example"),
        Vec::new(),
    )?;
    if let Some(cursor) = last_event_id {
        request.headers_mut().insert(
            HeaderName::from_static("last-event-id"),
            HeaderValue::from_str(cursor).map_err(|_| "cursor header construction failed")?,
        );
    }
    send_existing(context, router, request).await
}

async fn send_existing(
    context: &CaseContext,
    router: &axum::Router,
    request: Request<Body>,
) -> CaseResult<Response> {
    timeout_at(context.operation_deadline, router.clone().oneshot(request))
        .await
        .map_err(|_| "HTTP router operation deadline expired")?
        .map_err(|_| "HTTP router returned an unexpected service error")
}

async fn release_lease(
    context: &CaseContext,
    router: &axum::Router,
    session: &TestSession,
    lease: &LeaseReply,
    idempotency_key: &str,
) -> CaseResult<Response> {
    let body = serde_json::to_vec(&json!({
        "schema_version": 2,
        "consumer_id": lease.consumer_id.to_string(),
        "renewal_sequence": lease.sequence,
    }))
    .map_err(|_| "release body serialization failed")?;
    send(
        context,
        router,
        Method::DELETE,
        &format!("{STREAM_PREFIX}/stream-leases/{}", lease.lease_id),
        Some(session),
        true,
        Some(idempotency_key),
        Some("https://quotes.example"),
        body,
    )
    .await
}

#[derive(Debug)]
struct SseEvent {
    name: String,
    value: Value,
}

struct SseReader {
    body: Body,
    pending: Vec<u8>,
}

impl SseReader {
    fn new(body: Body) -> Self {
        Self {
            body,
            pending: Vec::new(),
        }
    }

    async fn next(&mut self) -> CaseResult<Option<SseEvent>> {
        timeout(Duration::from_secs(2), async {
            loop {
                if let Some(end) = self.pending.windows(2).position(|pair| pair == b"\n\n") {
                    let block = self.pending.drain(..end + 2).collect::<Vec<_>>();
                    let text =
                        std::str::from_utf8(&block).map_err(|_| "SSE frame was not UTF-8")?;
                    let mut event_name = None;
                    let mut data = Vec::new();
                    for line in text.lines() {
                        let line = line.trim_end_matches('\r');
                        if let Some(value) = line.strip_prefix("event: ") {
                            event_name = Some(value.to_owned());
                        } else if let Some(value) = line.strip_prefix("data: ") {
                            data.push(value);
                        }
                    }
                    if let Some(name) = event_name {
                        if !data.is_empty() {
                            let value = serde_json::from_str(&data.join("\n"))
                                .map_err(|_| "SSE event JSON was invalid")?;
                            return Ok(Some(SseEvent { name, value }));
                        }
                    }
                    continue;
                }
                let Some(frame) = self.body.frame().await else {
                    return Ok(None);
                };
                let frame = frame.map_err(|_| "SSE body frame failed")?;
                if let Ok(data) = frame.into_data() {
                    self.pending.extend_from_slice(&data);
                }
            }
        })
        .await
        .map_err(|_| "SSE event deadline expired")?
    }
}

fn snapshot_rows(event: &SseEvent) -> CaseResult<&Vec<Value>> {
    event.value["body"]["rows"]
        .as_array()
        .ok_or("SSE snapshot omitted rows")
}

async fn read_initial_snapshot(reader: &mut SseReader, expected: &[Uuid]) -> CaseResult<()> {
    let reset = reader.next().await?.ok_or("SSE closed before reset")?;
    ensure(reset.name == "reset", "first SSE event was not reset")?;
    let snapshot = reader.next().await?.ok_or("SSE closed before snapshot")?;
    ensure(
        snapshot.name == "snapshot",
        "initial SSE event was not snapshot",
    )?;
    let rows = snapshot_rows(&snapshot)?;
    ensure(
        rows.len() == expected.len(),
        "initial snapshot was not complete",
    )?;
    let ids = rows
        .iter()
        .map(|row| {
            row["membership_id"]
                .as_str()
                .and_then(|value| Uuid::parse_str(value).ok())
                .ok_or("snapshot membership identity was invalid")
        })
        .collect::<CaseResult<Vec<_>>>()?;
    ensure(
        ids == expected,
        "snapshot identity set was not canonical and complete",
    )?;
    for row in rows {
        ensure(row["quote"].is_null(), "windowless fixture exposed a quote")?;
        ensure(
            row["availability"] == "UNAVAILABLE"
                && row["reason_code"] == "SESSION_WINDOW_UNAVAILABLE",
            "windowless fixture did not expose typed unavailability",
        )?;
    }
    let status = reader.next().await?.ok_or("SSE closed before status")?;
    ensure(status.name == "status", "initial SSE status was missing")?;
    ensure(
        status.value["body"]["reason_code"] == "SESSION_WINDOW_UNAVAILABLE",
        "initial status did not preserve the windowless reason",
    )
}

fn ensure(condition: bool, message: &'static str) -> CaseResult {
    if condition { Ok(()) } else { Err(message) }
}

async fn binding_value(
    context: &CaseContext,
    actor_guc: Uuid,
    session_guc: &str,
    owner: Uuid,
    session_hash: &str,
    slot: Uuid,
    grant: Uuid,
    source_hash: &str,
) -> CaseResult<bool> {
    let mut tx = context
        .db()
        .app
        .begin()
        .await
        .map_err(|_| "app binding transaction did not start")?;
    set_actor(&mut tx, actor_guc)
        .await
        .map_err(|_| "app actor context was not set")?;
    sqlx::query("SELECT pg_catalog.set_config('app.market_stream_session_hash', $1, true)")
        .bind(session_guc)
        .execute(&mut *tx)
        .await
        .map_err(|_| "app session context was not set")?;
    let valid: bool = sqlx::query_scalar(
        "SELECT public.owner_market_stream_api_binding_valid($1, $2, $3, $4, $5)",
    )
    .bind(owner)
    .bind(session_hash)
    .bind(slot)
    .bind(grant)
    .bind(source_hash)
    .fetch_one(&mut *tx)
    .await
    .map_err(|_| "app binding helper call failed")?;
    tx.commit()
        .await
        .map_err(|_| "app binding transaction did not commit")?;
    Ok(valid)
}

fn sqlstate_is_permission_denied(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|database| database.code())
        .is_some_and(|code| code.as_ref() == "42501")
}

async fn non_app_helper_is_denied(pool: &PgPool, context: &CaseContext) -> CaseResult<bool> {
    match sqlx::query_scalar::<_, bool>(
        "SELECT public.owner_market_stream_api_binding_valid($1, $2, $3, $4, $5)",
    )
    .bind(context.fixture.owner_user_id)
    .bind(&context.users.owner.session_hash)
    .bind(context.fixture.credential_slot_id)
    .bind(context.fixture.grant_id)
    .bind(OWNER_CONTRACT_HASH)
    .fetch_one(pool)
    .await
    {
        Ok(_) => Ok(false),
        Err(error) => Ok(sqlstate_is_permission_denied(&error)),
    }
}

async fn function_security_metadata(
    pool: &PgPool,
) -> CaseResult<(String, String, bool, bool, bool, bool, bool, bool)> {
    sqlx::query_as(
        "SELECT pg_catalog.pg_get_userbyid(p.proowner)::text,
                COALESCE(pg_catalog.array_to_string(p.proconfig, ','), ''),
                EXISTS (
                    SELECT 1
                      FROM pg_catalog.aclexplode(
                          COALESCE(p.proacl, pg_catalog.acldefault('f', p.proowner))
                      ) AS acl
                     WHERE acl.grantee = 0
                       AND acl.privilege_type = 'EXECUTE'
                ),
                pg_catalog.has_function_privilege('app', p.oid, 'EXECUTE'),
                pg_catalog.has_function_privilege('worker', p.oid, 'EXECUTE'),
                pg_catalog.has_function_privilege('admin', p.oid, 'EXECUTE'),
                pg_catalog.has_function_privilege('research_writer', p.oid, 'EXECUTE'),
                pg_catalog.has_function_privilege('audit_writer', p.oid, 'EXECUTE')
           FROM pg_catalog.pg_proc AS p
           JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace
          WHERE n.nspname = 'public'
            AND p.proname = 'owner_market_stream_api_binding_valid'
            AND pg_catalog.pg_get_function_identity_arguments(p.oid) =
                'p_owner_id uuid, p_session_hash text, p_slot_id uuid, p_grant_id uuid, p_contract_sha256 text'",
    )
    .fetch_one(pool)
    .await
    .map_err(|_| "binding helper security metadata query failed")
}

async fn grant_and_cache_fingerprints(pool: &PgPool) -> CaseResult<(i64, String, i64, String)> {
    let grants: (i64, String) = sqlx::query_as(
        "SELECT pg_catalog.count(*)::bigint,
                COALESCE(pg_catalog.md5(pg_catalog.string_agg(
                    pg_catalog.to_jsonb(g)::text, E'\\n' ORDER BY pg_catalog.to_jsonb(g)::text
                )), pg_catalog.md5(''))
           FROM public.owner_market_stream_grants AS g",
    )
    .fetch_one(pool)
    .await
    .map_err(|_| "grant fixture fingerprint query failed")?;
    let cache: (i64, String) = sqlx::query_as(
        "SELECT pg_catalog.count(*)::bigint,
                COALESCE(pg_catalog.md5(pg_catalog.string_agg(
                    pg_catalog.to_jsonb(c)::text, E'\\n' ORDER BY pg_catalog.to_jsonb(c)::text
                )), pg_catalog.md5(''))
           FROM public.owner_market_stream_cache AS c",
    )
    .fetch_one(pool)
    .await
    .map_err(|_| "cache fixture fingerprint query failed")?;
    Ok((grants.0, grants.1, cache.0, cache.1))
}

async fn successful_migration_count(pool: &PgPool) -> CaseResult<i64> {
    sqlx::query_scalar(
        "SELECT pg_catalog.count(*)::bigint
           FROM public._sqlx_migrations
          WHERE success",
    )
    .fetch_one(pool)
    .await
    .map_err(|_| "migration count query failed")
}

async fn binding_function_exists(pool: &PgPool) -> CaseResult<bool> {
    sqlx::query_scalar(
        "SELECT pg_catalog.to_regprocedure(
            'public.owner_market_stream_api_binding_valid(uuid,text,uuid,uuid,text)'
        ) IS NOT NULL",
    )
    .fetch_one(pool)
    .await
    .map_err(|_| "binding helper existence query failed")
}

const API_BINDING_DOWN: &str =
    include_str!("../../../../migrations/0057_owner_market_stream_api_binding.down.sql");
const API_BINDING_UP: &str =
    include_str!("../../../../migrations/0057_owner_market_stream_api_binding.up.sql");

async fn binding_helper_case(context: &mut CaseContext) -> CaseResult {
    let fixture = &context.fixture;
    let owner = &context.users.owner;
    ensure(
        binding_value(
            context,
            fixture.owner_user_id,
            &owner.session_hash,
            fixture.owner_user_id,
            &owner.session_hash,
            fixture.credential_slot_id,
            fixture.grant_id,
            OWNER_CONTRACT_HASH,
        )
        .await?,
        "exact app binding was not accepted",
    )?;
    for (slot, grant, contract) in [
        (
            Uuid::new_v4(),
            fixture.grant_id,
            OWNER_CONTRACT_HASH.to_owned(),
        ),
        (
            fixture.credential_slot_id,
            Uuid::new_v4(),
            OWNER_CONTRACT_HASH.to_owned(),
        ),
        (fixture.credential_slot_id, fixture.grant_id, "a".repeat(64)),
    ] {
        ensure(
            !binding_value(
                context,
                fixture.owner_user_id,
                &owner.session_hash,
                fixture.owner_user_id,
                &owner.session_hash,
                slot,
                grant,
                &contract,
            )
            .await?,
            "mismatched immutable binding was accepted",
        )?;
    }
    ensure(
        !binding_value(
            context,
            fixture.owner_user_id,
            &owner.session_hash,
            fixture.other_owner_user_id,
            &owner.session_hash,
            fixture.credential_slot_id,
            fixture.grant_id,
            OWNER_CONTRACT_HASH,
        )
        .await?,
        "wrong owner binding was accepted",
    )?;
    let absent_hash = "0".repeat(64);
    ensure(
        !binding_value(
            context,
            fixture.owner_user_id,
            &owner.session_hash,
            fixture.owner_user_id,
            &absent_hash,
            fixture.credential_slot_id,
            fixture.grant_id,
            OWNER_CONTRACT_HASH,
        )
        .await?,
        "wrong session binding was accepted",
    )?;
    ensure(
        !binding_value(
            context,
            fixture.other_owner_user_id,
            &owner.session_hash,
            fixture.owner_user_id,
            &owner.session_hash,
            fixture.credential_slot_id,
            fixture.grant_id,
            OWNER_CONTRACT_HASH,
        )
        .await?,
        "wrong actor GUC was accepted",
    )?;
    ensure(
        !binding_value(
            context,
            fixture.owner_user_id,
            &context.users.other_owner.session_hash,
            fixture.owner_user_id,
            &owner.session_hash,
            fixture.credential_slot_id,
            fixture.grant_id,
            OWNER_CONTRACT_HASH,
        )
        .await?,
        "wrong stream-session GUC was accepted",
    )?;

    let metadata = function_security_metadata(&context.db().migration_owner).await?;
    ensure(
        metadata.0 == "migration_owner",
        "binding helper owner was unexpected",
    )?;
    ensure(
        metadata
            .1
            .split(',')
            .any(|value| value == "search_path=pg_catalog"),
        "binding helper search path was not pinned",
    )?;
    ensure(!metadata.2, "binding helper remained executable by PUBLIC")?;
    ensure(metadata.3, "app did not retain binding helper execution")?;
    ensure(
        !metadata.4 && !metadata.5 && !metadata.6 && !metadata.7,
        "a non-app role retained binding helper execution",
    )?;

    let direct_select = sqlx::query_scalar::<_, i64>(
        "SELECT pg_catalog.count(*)::bigint FROM public.owner_market_stream_grants",
    )
    .fetch_one(&context.db().app)
    .await;
    ensure(
        direct_select
            .as_ref()
            .is_err_and(sqlstate_is_permission_denied),
        "app could directly select private grants",
    )?;
    for pool in [
        &context.db().worker,
        &context.db().admin,
        &context.db().research_writer,
        &context.audit_pool,
        &context.db().migration_owner,
    ] {
        ensure(
            non_app_helper_is_denied(pool, context).await?,
            "non-app role executed the app-only binding helper",
        )?;
    }

    let before_data = grant_and_cache_fingerprints(&context.db().migration_owner).await?;
    let before_migrations = successful_migration_count(&context.db().migration_owner).await?;
    let mut tx = context
        .db()
        .migration_owner
        .begin()
        .await
        .map_err(|_| "migration-owner rollback transaction did not start")?;
    sqlx::raw_sql(API_BINDING_DOWN)
        .execute(&mut *tx)
        .await
        .map_err(|_| "bounded migration 0057 rollback failed")?;
    let absent: bool = sqlx::query_scalar(
        "SELECT pg_catalog.to_regprocedure(
            'public.owner_market_stream_api_binding_valid(uuid,text,uuid,uuid,text)'
        ) IS NULL",
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|_| "binding helper absence check failed")?;
    ensure(absent, "migration down left the binding helper installed")?;
    sqlx::raw_sql(API_BINDING_UP)
        .execute(&mut *tx)
        .await
        .map_err(|_| "bounded migration 0057 reapplication failed")?;
    let restored: bool = sqlx::query_scalar(
        "SELECT pg_catalog.to_regprocedure(
            'public.owner_market_stream_api_binding_valid(uuid,text,uuid,uuid,text)'
        ) IS NOT NULL",
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|_| "binding helper restoration check failed")?;
    ensure(restored, "migration up did not restore the binding helper")?;
    tx.rollback()
        .await
        .map_err(|_| "bounded migration rollback transaction did not roll back")?;
    let after_data = grant_and_cache_fingerprints(&context.db().migration_owner).await?;
    let after_migrations = successful_migration_count(&context.db().migration_owner).await?;
    ensure(
        before_data == after_data,
        "grant/cache data changed during migration check",
    )?;
    ensure(
        before_migrations == after_migrations,
        "migration ledger changed during reversible check",
    )
}

async fn lease_state(
    context: &CaseContext,
    lease_id: Uuid,
) -> CaseResult<(i64, String, DateTime<Utc>, i64)> {
    sqlx::query_as(
        "SELECT lease.renewal_sequence,
                lease.state,
                lease.lease_expires_at,
                pg_catalog.count(item.membership_id)::bigint
           FROM public.owner_market_stream_leases AS lease
           LEFT JOIN public.owner_market_stream_lease_items AS item
             ON item.lease_id = lease.id
            AND item.owner_user_id = lease.owner_user_id
          WHERE lease.id = $1
          GROUP BY lease.id",
    )
    .bind(lease_id)
    .fetch_one(&context.db().migration_owner)
    .await
    .map_err(|_| "owned lease state query failed")
}

async fn lease_item_ids(context: &CaseContext, lease_id: Uuid) -> CaseResult<Vec<Uuid>> {
    sqlx::query_scalar(
        "SELECT membership_id
           FROM public.owner_market_stream_lease_items
          WHERE lease_id = $1
          ORDER BY membership_id",
    )
    .bind(lease_id)
    .fetch_all(&context.db().migration_owner)
    .await
    .map_err(|_| "owned lease item query failed")
}

async fn invalid_lease_status(
    context: &CaseContext,
    router: &axum::Router,
    session: &TestSession,
    consumer_id: Uuid,
    body: Value,
    key: &str,
) -> CaseResult<StatusCode> {
    let bytes = serde_json::to_vec(&body).map_err(|_| "invalid lease body serialization failed")?;
    let response = send(
        context,
        router,
        Method::POST,
        &format!("{STREAM_PREFIX}/stream-leases"),
        Some(session),
        true,
        Some(key),
        Some("https://quotes.example"),
        bytes,
    )
    .await?;
    let _ = consumer_id;
    Ok(response.status())
}

async fn lease_authentication_case(context: &mut CaseContext) -> CaseResult {
    let router = context.router(context.owner_config()?, None).await?;
    let all = sorted_identities(context.fixture.identities.clone());
    let owner = context.users.owner.clone();
    let consumer_id = Uuid::new_v4();
    let valid = lease_body(&all, consumer_id, 0);
    let valid_bytes = serde_json::to_vec(&valid).map_err(|_| "valid lease encoding failed")?;
    let path = format!("{STREAM_PREFIX}/stream-leases");

    let response = send(
        context,
        &router,
        Method::POST,
        &path,
        None,
        false,
        Some("wp4h-auth-no-session"),
        Some("https://quotes.example"),
        b"not-json".to_vec(),
    )
    .await?;
    ensure(
        response.status() == StatusCode::UNAUTHORIZED,
        "missing session was not rejected",
    )?;

    let response = send(
        context,
        &router,
        Method::POST,
        &path,
        Some(&context.users.member),
        true,
        Some("wp4h-auth-member"),
        Some("https://quotes.example"),
        valid_bytes.clone(),
    )
    .await?;
    ensure(
        response.status() == StatusCode::FORBIDDEN,
        "member session was not rejected",
    )?;

    let response = send(
        context,
        &router,
        Method::POST,
        &path,
        Some(&owner),
        false,
        Some("wp4h-auth-no-csrf"),
        Some("https://quotes.example"),
        valid_bytes.clone(),
    )
    .await?;
    ensure(
        response.status() == StatusCode::FORBIDDEN,
        "missing mutation CSRF was not rejected",
    )?;

    let response = send(
        context,
        &router,
        Method::POST,
        &path,
        Some(&owner),
        true,
        Some("wp4h-auth-wrong-origin"),
        Some("https://attacker.example"),
        valid_bytes.clone(),
    )
    .await?;
    ensure(
        response.status() == StatusCode::FORBIDDEN,
        "wrong request Origin was not rejected",
    )?;

    let response = send(
        context,
        &router,
        Method::POST,
        &path,
        Some(&owner),
        true,
        None,
        Some("https://quotes.example"),
        valid_bytes.clone(),
    )
    .await?;
    ensure(
        response.status() == StatusCode::BAD_REQUEST,
        "missing idempotency key was not rejected",
    )?;
    let mut duplicate = request(
        Method::POST,
        &path,
        Some(&owner),
        true,
        Some("wp4h-duplicate-key"),
        Some("https://quotes.example"),
        valid_bytes.clone(),
    )?;
    duplicate.headers_mut().append(
        HeaderName::from_static("idempotency-key"),
        HeaderValue::from_static("wp4h-duplicate-key-2"),
    );
    let response = send_existing(context, &router, duplicate).await?;
    ensure(
        response.status() == StatusCode::BAD_REQUEST,
        "duplicate idempotency key was not rejected",
    )?;

    let response = send(
        context,
        &router,
        Method::POST,
        &format!("{path}?unexpected=1"),
        Some(&owner),
        true,
        Some("wp4h-query-rejected"),
        Some("https://quotes.example"),
        valid_bytes.clone(),
    )
    .await?;
    ensure(
        response.status() == StatusCode::BAD_REQUEST,
        "unsupported lease query was not rejected",
    )?;

    let mut malformed = valid.clone();
    malformed["schema_version"] = json!(1);
    ensure(
        invalid_lease_status(
            context,
            &router,
            &owner,
            consumer_id,
            malformed,
            "wp4h-bad-schema",
        )
        .await?
            == StatusCode::BAD_REQUEST,
        "wrong schema version was not rejected",
    )?;
    let mut malformed = valid.clone();
    malformed["consumer_id"] = json!("ABCDEF00-0000-4000-8000-000000000001");
    ensure(
        invalid_lease_status(
            context,
            &router,
            &owner,
            consumer_id,
            malformed,
            "wp4h-bad-uuid",
        )
        .await?
            == StatusCode::BAD_REQUEST,
        "noncanonical UUID was not rejected",
    )?;
    let mut malformed = valid.clone();
    malformed["renewal_sequence"] = json!((1_u64 << 53));
    ensure(
        invalid_lease_status(
            context,
            &router,
            &owner,
            consumer_id,
            malformed,
            "wp4h-bad-sequence",
        )
        .await?
            == StatusCode::BAD_REQUEST,
        "unsafe sequence integer was not rejected",
    )?;
    let mut malformed = valid.clone();
    malformed["identities"][0]["generation"] = json!((1_u64 << 53));
    ensure(
        invalid_lease_status(
            context,
            &router,
            &owner,
            consumer_id,
            malformed,
            "wp4h-bad-generation",
        )
        .await?
            == StatusCode::BAD_REQUEST,
        "unsafe membership generation was not rejected",
    )?;
    let mut malformed = valid.clone();
    malformed["identities"].as_array_mut().unwrap().reverse();
    ensure(
        invalid_lease_status(
            context,
            &router,
            &owner,
            consumer_id,
            malformed,
            "wp4h-unsorted-identities",
        )
        .await?
            == StatusCode::BAD_REQUEST,
        "unsorted identity set was not rejected",
    )?;
    let mut malformed = valid.clone();
    let first = malformed["identities"][0].clone();
    malformed["identities"][1]["instrument_id"] = first["instrument_id"].clone();
    ensure(
        invalid_lease_status(
            context,
            &router,
            &owner,
            consumer_id,
            malformed,
            "wp4h-duplicate-symbol",
        )
        .await?
            == StatusCode::BAD_REQUEST,
        "duplicate symbol was not rejected",
    )?;
    let mut malformed = valid.clone();
    let first_membership = malformed["identities"][0]["membership_id"].clone();
    malformed["identities"][1]["membership_id"] = first_membership;
    ensure(
        invalid_lease_status(
            context,
            &router,
            &owner,
            consumer_id,
            malformed,
            "wp4h-duplicate-identity",
        )
        .await?
            == StatusCode::BAD_REQUEST,
        "duplicate membership identity was not rejected",
    )?;
    let oversized = vec![b' '; 16 * 1024 + 1];
    let response = send(
        context,
        &router,
        Method::POST,
        &path,
        Some(&owner),
        true,
        Some("wp4h-oversize"),
        Some("https://quotes.example"),
        oversized,
    )
    .await?;
    ensure(
        response.status() == StatusCode::PAYLOAD_TOO_LARGE,
        "oversized request body was not rejected",
    )?;

    let created = post_lease(
        context,
        &router,
        &owner,
        consumer_id,
        0,
        &all,
        "wp4h-lease-create",
    )
    .await?;
    ensure(
        created.status() == StatusCode::OK,
        "initial thirty-identity lease did not succeed",
    )?;
    let lease_id = lease_id_from_response(context, created).await?;

    let replay = post_lease(
        context,
        &router,
        &owner,
        consumer_id,
        0,
        &all,
        "wp4h-lease-create",
    )
    .await?;
    ensure(
        replay.status() == StatusCode::OK,
        "exact lease replay did not succeed",
    )?;
    ensure(
        lease_id_from_response(context, replay).await? == lease_id,
        "exact replay changed the durable lease identity",
    )?;

    let renewal = post_lease(
        context,
        &router,
        &owner,
        consumer_id,
        1,
        &all,
        "wp4h-lease-renew-1",
    )
    .await?;
    ensure(
        renewal.status() == StatusCode::OK,
        "valid lease renewal did not succeed",
    )?;
    ensure(
        lease_id_from_response(context, renewal).await? == lease_id,
        "valid renewal replaced the consumer lease identity",
    )?;

    let stale = post_lease(
        context,
        &router,
        &owner,
        consumer_id,
        0,
        &all,
        "wp4h-lease-stale-sequence",
    )
    .await?;
    ensure(
        stale.status() == StatusCode::CONFLICT,
        "stale lease sequence did not return conflict",
    )?;
    let mut wrong_generation = all.clone();
    wrong_generation[0].generation += 1;
    let wrong_generation_response = post_lease(
        context,
        &router,
        &owner,
        consumer_id,
        2,
        &wrong_generation,
        "wp4h-wrong-generation",
    )
    .await?;
    ensure(
        wrong_generation_response.status() == StatusCode::NOT_FOUND,
        "wrong membership generation was not refused",
    )?;
    let mut wrong_instrument = all.clone();
    wrong_instrument[0].instrument_id = "999999.KRX".to_owned();
    let wrong_instrument_response = post_lease(
        context,
        &router,
        &owner,
        consumer_id,
        2,
        &wrong_instrument,
        "wp4h-wrong-instrument",
    )
    .await?;
    ensure(
        wrong_instrument_response.status() == StatusCode::NOT_FOUND,
        "wrong membership instrument was not refused",
    )?;

    let secondary_write = post_lease(
        context,
        &router,
        &context.users.secondary_owner,
        consumer_id,
        2,
        &all,
        "wp4h-secondary-session-write",
    )
    .await?;
    ensure(
        secondary_write.status() == StatusCode::NOT_FOUND,
        "secondary owner session mutated the primary session lease",
    )?;
    let secondary_read = get_stream(
        context,
        &router,
        &context.users.secondary_owner,
        lease_id,
        None,
    )
    .await?;
    ensure(
        secondary_read.status() == StatusCode::NOT_FOUND,
        "secondary owner session read the primary session lease",
    )?;

    let other_router = context.other_owner_route().await?;
    let other_read = get_stream(
        context,
        &other_router,
        &context.users.other_owner,
        lease_id,
        None,
    )
    .await?;
    ensure(
        other_read.status() == StatusCode::NOT_FOUND,
        "other owner read the primary owner lease",
    )?;
    let lease = LeaseReply {
        lease_id,
        consumer_id,
        sequence: 1,
    };
    let wrong_consumer = LeaseReply {
        lease_id,
        consumer_id: Uuid::new_v4(),
        sequence: 1,
    };
    let wrong_consumer_release = release_lease(
        context,
        &router,
        &owner,
        &wrong_consumer,
        "wp4h-wrong-consumer-release",
    )
    .await?;
    ensure(
        wrong_consumer_release.status() == StatusCode::NOT_FOUND,
        "wrong consumer could release the lease",
    )?;
    let other_release = release_lease(
        context,
        &other_router,
        &context.users.other_owner,
        &lease,
        "wp4h-other-owner-release",
    )
    .await?;
    ensure(
        other_release.status() == StatusCode::NOT_FOUND,
        "other owner mutated the primary owner lease",
    )?;
    ensure(
        lease_state(context, lease_id).await?.0 == 1
            && lease_item_ids(context, lease_id).await? == fixture_membership_ids(&context.fixture),
        "unauthorized consumer changed the primary lease rows",
    )?;

    let released = release_lease(context, &router, &owner, &lease, "wp4h-release-once").await?;
    ensure(
        released.status() == StatusCode::OK,
        "lease release did not succeed",
    )?;
    let replay_router = context.router(context.owner_config()?, None).await?;
    let release_replay =
        release_lease(context, &replay_router, &owner, &lease, "wp4h-release-once").await?;
    ensure(
        release_replay.status() == StatusCode::OK,
        "exact release replay in a fresh API state did not succeed",
    )?;
    let after_release = get_stream(context, &router, &owner, lease_id, None).await?;
    ensure(
        after_release.status() == StatusCode::NOT_FOUND,
        "released lease remained readable",
    )?;
    let resurrect = post_lease(
        context,
        &router,
        &owner,
        consumer_id,
        2,
        &all,
        "wp4h-release-resurrection",
    )
    .await?;
    ensure(
        resurrect.status() == StatusCode::NOT_FOUND,
        "released consumer lease was resurrected",
    )?;
    let final_state = lease_state(context, lease_id).await?;
    ensure(
        final_state.0 == 1 && final_state.1 == "RELEASED" && final_state.3 == 30,
        "final lease tombstone or exact item set changed",
    )
}

async fn cache_versions(context: &CaseContext, fixture: &Fixture) -> CaseResult<Vec<(Uuid, i64)>> {
    let ids = fixture_membership_ids(fixture);
    sqlx::query_as(
        "SELECT membership.id, COALESCE(cache.state_version, 0)::bigint
           FROM public.owner_equity_memberships AS membership
           LEFT JOIN public.owner_market_stream_cache AS cache
             ON cache.membership_id = membership.id
          WHERE membership.id = ANY($1)
          ORDER BY membership.id",
    )
    .bind(ids)
    .fetch_all(&context.db().migration_owner)
    .await
    .map_err(|_| "fixture cache version query failed")
}

async fn producer_count(context: &CaseContext, slot: Uuid) -> CaseResult<i64> {
    sqlx::query_scalar(
        "SELECT pg_catalog.count(*)::bigint
           FROM public.owner_market_stream_producers
          WHERE credential_slot_id = $1",
    )
    .bind(slot)
    .fetch_one(&context.db().migration_owner)
    .await
    .map_err(|_| "fixture producer absence query failed")
}

fn sse_response_valid(response: &Response) -> bool {
    response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/event-stream"))
        && response
            .headers()
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok())
            == Some("no-store")
        && response
            .headers()
            .get("x-accel-buffering")
            .and_then(|value| value.to_str().ok())
            == Some("no")
}

async fn initial_snapshot_case(context: &mut CaseContext) -> CaseResult {
    let router = context.router(context.owner_config()?, None).await?;
    let lease = create_lease_id(
        context,
        &router,
        &context.users.owner,
        Uuid::new_v4(),
        &context.fixture.identities,
        "wp4h-snapshot-create",
    )
    .await?;

    let before_versions = cache_versions(context, &context.fixture).await?;
    let before_lease = lease_state(context, lease.lease_id).await?;
    let before_items = lease_item_ids(context, lease.lease_id).await?;
    ensure(
        before_items.len() == 30,
        "fixture lease did not contain thirty items",
    )?;
    let bad_query = send(
        context,
        &router,
        Method::GET,
        &format!("{STREAM_PREFIX}/market-stream?other={}", lease.lease_id),
        Some(&context.users.owner),
        false,
        None,
        Some("https://quotes.example"),
        Vec::new(),
    )
    .await?;
    ensure(
        bad_query.status() == StatusCode::BAD_REQUEST,
        "invalid stream query was not denied",
    )?;
    let bad_cursor = get_stream(
        context,
        &router,
        &context.users.owner,
        lease.lease_id,
        Some("invalid-cursor"),
    )
    .await?;
    ensure(
        bad_cursor.status() == StatusCode::BAD_REQUEST,
        "invalid Last-Event-ID was not denied",
    )?;
    let mut duplicate_cursor = request(
        Method::GET,
        &format!("{STREAM_PREFIX}/market-stream?lease_id={}", lease.lease_id),
        Some(&context.users.owner),
        false,
        None,
        Some("https://quotes.example"),
        Vec::new(),
    )?;
    duplicate_cursor.headers_mut().append(
        HeaderName::from_static("last-event-id"),
        HeaderValue::from_static("00000000-0000-4000-8000-000000000099:1"),
    );
    duplicate_cursor.headers_mut().append(
        HeaderName::from_static("last-event-id"),
        HeaderValue::from_static("00000000-0000-4000-8000-000000000099:2"),
    );
    let duplicate_cursor_response = send_existing(context, &router, duplicate_cursor).await?;
    ensure(
        duplicate_cursor_response.status() == StatusCode::BAD_REQUEST,
        "duplicate Last-Event-ID was not denied",
    )?;

    let response = get_stream(context, &router, &context.users.owner, lease.lease_id, None).await?;
    ensure(
        response.status() == StatusCode::OK && sse_response_valid(&response),
        "SSE response did not carry private streaming headers",
    )?;
    let mut reader = SseReader::new(response.into_body());
    let expected = fixture_membership_ids(&context.fixture);
    timeout(
        Duration::from_secs(2),
        read_initial_snapshot(&mut reader, &expected),
    )
    .await
    .map_err(|_| "initial reset/snapshot/status exceeded two seconds")??;
    drop(reader);

    ensure(
        cache_versions(context, &context.fixture).await? == before_versions,
        "GET or status changed cache state versions",
    )?;
    ensure(
        lease_state(context, lease.lease_id).await? == before_lease,
        "GET or status renewed the browser lease",
    )?;
    ensure(
        lease_item_ids(context, lease.lease_id).await? == before_items,
        "GET or status changed the demanded membership set",
    )?;
    ensure(
        producer_count(context, context.fixture.credential_slot_id).await? == 0,
        "read-only fixture unexpectedly created a producer",
    )
}

async fn access_revoked_then_eof(reader: &mut SseReader) -> CaseResult {
    let access = timeout(Duration::from_secs(2), async {
        loop {
            let event = reader
                .next()
                .await?
                .ok_or("SSE closed before revocation status")?;
            if event.name == "status" && event.value["body"]["reason_code"] == "ACCESS_REVOKED" {
                return Ok::<_, &'static str>(());
            }
        }
    })
    .await
    .map_err(|_| "revocation status did not arrive within two seconds")??;
    let _ = access;
    ensure(
        reader.next().await?.is_none(),
        "SSE did not close after access revocation",
    )
}

async fn replacement_and_revocation_case(context: &mut CaseContext) -> CaseResult {
    let router = context.router(context.owner_config()?, None).await?;
    let expected_all = fixture_membership_ids(&context.fixture);
    let consumer = Uuid::new_v4();
    let mut lease = create_lease_id(
        context,
        &router,
        &context.users.owner,
        consumer,
        &context.fixture.identities,
        "wp4h-replace-create",
    )
    .await?;
    let response = get_stream(context, &router, &context.users.owner, lease.lease_id, None).await?;
    ensure(
        response.status() == StatusCode::OK,
        "initial replacement stream did not open",
    )?;
    let mut reader = SseReader::new(response.into_body());
    timeout(
        Duration::from_secs(2),
        read_initial_snapshot(&mut reader, &expected_all),
    )
    .await
    .map_err(|_| "initial replacement snapshot exceeded two seconds")??;

    let removed = context.fixture.identities[0].membership_id;
    let mut reduced = context.fixture.identities.clone();
    reduced.remove(0);
    let renewal = post_lease(
        context,
        &router,
        &context.users.owner,
        consumer,
        1,
        &reduced,
        "wp4h-replace-29",
    )
    .await?;
    ensure(
        renewal.status() == StatusCode::OK,
        "thirty-to-twenty-nine identity replacement failed",
    )?;
    ensure(
        lease_id_from_response(context, renewal).await? == lease.lease_id,
        "replacement changed the browser lease identity",
    )?;
    lease.sequence = 1;
    let expected_reduced = {
        let mut ids = expected_all.clone();
        ids.retain(|id| *id != removed);
        ids
    };
    let mut saw_reset = false;
    let mut saw_snapshot = false;
    for _ in 0..6 {
        let event = reader
            .next()
            .await?
            .ok_or("SSE closed during identity replacement")?;
        if event.name == "reset" {
            saw_reset = true;
        }
        if event.name == "snapshot" {
            let rows = snapshot_rows(&event)?;
            let ids = rows
                .iter()
                .map(|row| {
                    row["membership_id"]
                        .as_str()
                        .and_then(|value| Uuid::parse_str(value).ok())
                        .ok_or("replacement snapshot membership id was invalid")
                })
                .collect::<CaseResult<Vec<_>>>()?;
            ensure(
                ids == expected_reduced,
                "replacement snapshot was not the complete new set",
            )?;
            ensure(
                !ids.contains(&removed),
                "removed membership remained in replacement",
            )?;
            ensure(
                rows.iter().all(|row| row["quote"].is_null()),
                "windowless replacement exposed quote data",
            )?;
            saw_snapshot = true;
            break;
        }
        if event.name == "delta" {
            let rows = event.value["body"]["rows"]
                .as_array()
                .ok_or("replacement delta omitted rows")?;
            ensure(
                rows.iter()
                    .all(|row| row["membership_id"] != removed.to_string()),
                "removed membership reappeared in a delta",
            )?;
        }
    }
    ensure(
        saw_reset && saw_snapshot,
        "replacement omitted reset or full snapshot",
    )?;
    let after_snapshot = reader
        .next()
        .await?
        .ok_or("SSE closed after replacement snapshot")?;
    ensure(
        after_snapshot.name == "status"
            && after_snapshot.value["body"]["reason_code"] == "SESSION_WINDOW_UNAVAILABLE",
        "replacement emitted an unexpected post-snapshot event",
    )?;
    let released = release_lease(
        context,
        &router,
        &context.users.owner,
        &lease,
        "wp4h-replace-release",
    )
    .await?;
    ensure(
        released.status() == StatusCode::OK,
        "replacement lease release failed",
    )?;
    access_revoked_then_eof(&mut reader).await?;
    drop(reader);

    let session_lease = create_lease_id(
        context,
        &router,
        &context.users.owner,
        Uuid::new_v4(),
        &context.fixture.identities,
        "wp4h-session-revoke-create",
    )
    .await?;
    let response = get_stream(
        context,
        &router,
        &context.users.owner,
        session_lease.lease_id,
        None,
    )
    .await?;
    ensure(
        response.status() == StatusCode::OK,
        "session-revocation SSE did not open",
    )?;
    let mut session_reader = SseReader::new(response.into_body());
    timeout(
        Duration::from_secs(2),
        read_initial_snapshot(&mut session_reader, &expected_all),
    )
    .await
    .map_err(|_| "session-revocation initial snapshot exceeded two seconds")??;
    boundary::revoke_session(
        context.db(),
        context.fixture.owner_user_id,
        &context.users.owner.session_hash,
    )
    .await
    .map_err(|_| "synthetic owner session revocation failed")?;
    access_revoked_then_eof(&mut session_reader).await?;
    drop(session_reader);

    let generation_lease = create_lease_id(
        context,
        &router,
        &context.users.secondary_owner,
        Uuid::new_v4(),
        &context.fixture.identities,
        "wp4h-generation-create",
    )
    .await?;
    let response = get_stream(
        context,
        &router,
        &context.users.secondary_owner,
        generation_lease.lease_id,
        None,
    )
    .await?;
    ensure(
        response.status() == StatusCode::OK,
        "generation invalidation SSE did not open",
    )?;
    let mut generation_reader = SseReader::new(response.into_body());
    timeout(
        Duration::from_secs(2),
        read_initial_snapshot(&mut generation_reader, &expected_all),
    )
    .await
    .map_err(|_| "generation invalidation initial snapshot exceeded two seconds")??;
    boundary::supersede_generation(context.db(), &context.fixture.identities[0])
        .await
        .map_err(|_| "synthetic generation supersession failed")?;
    access_revoked_then_eof(&mut generation_reader).await
}

async fn wait_for_pool_idle(pool: &PgPool) -> CaseResult {
    timeout(Duration::from_secs(5), async {
        loop {
            if pool.size() as usize == pool.num_idle() {
                return Ok::<_, &'static str>(());
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .map_err(|_| "owned LISTEN pool connections did not return to idle within five seconds")?
}

async fn consumer_capacity_case(context: &mut CaseContext) -> CaseResult {
    let app_pool = context.app_pool(32, "wp4h-market-stream-capacity").await?;
    let router = context
        .router(context.owner_config()?, Some(app_pool.clone()))
        .await?;
    let consumer = Uuid::new_v4();
    let lease = create_lease_id(
        context,
        &router,
        &context.users.owner,
        consumer,
        &context.fixture.identities,
        "wp4h-capacity-lease",
    )
    .await?;
    let mut bodies = Vec::with_capacity(20);
    for _ in 0..20 {
        let response =
            get_stream(context, &router, &context.users.owner, lease.lease_id, None).await?;
        ensure(
            response.status() == StatusCode::OK,
            "one of the twenty bounded SSE consumers was refused",
        )?;
        bodies.push(response.into_body());
    }
    let twenty_first =
        get_stream(context, &router, &context.users.owner, lease.lease_id, None).await?;
    ensure(
        twenty_first.status() == StatusCode::CONFLICT,
        "twenty-first SSE consumer did not hit the per-owner bound",
    )?;
    let code = response_code(context, twenty_first).await?;
    ensure(
        code == "STREAM_CONSUMER_CAPACITY",
        "twenty-first SSE consumer returned the wrong typed error",
    )?;
    drop(bodies);
    wait_for_pool_idle(&app_pool).await?;

    let recovered =
        get_stream(context, &router, &context.users.owner, lease.lease_id, None).await?;
    ensure(
        recovered.status() == StatusCode::OK,
        "consumer admission did not recover after response drops",
    )?;
    drop(recovered);
    wait_for_pool_idle(&app_pool).await
}

async fn response_code(context: &CaseContext, response: Response) -> CaseResult<String> {
    let value = response_json(context, response).await?;
    value["error"]["code"]
        .as_str()
        .map(str::to_owned)
        .ok_or("HTTP error omitted its finite code")
}

async fn stream_counts(context: &CaseContext) -> CaseResult<(i64, i64)> {
    let leases: i64 = sqlx::query_scalar(
        "SELECT pg_catalog.count(*)::bigint FROM public.owner_market_stream_leases",
    )
    .fetch_one(&context.db().migration_owner)
    .await
    .map_err(|_| "synthetic stream lease count query failed")?;
    let items: i64 = sqlx::query_scalar(
        "SELECT pg_catalog.count(*)::bigint FROM public.owner_market_stream_lease_items",
    )
    .fetch_one(&context.db().migration_owner)
    .await
    .map_err(|_| "synthetic lease item count query failed")?;
    Ok((leases, items))
}

fn owner_only_pins(
    slot: Uuid,
    grant: Uuid,
    contract_hash: &str,
) -> CaseResult<MarketStreamReadConfig> {
    Ok(MarketStreamReadConfig::OwnerOnly(
        MarketStreamReadPins::new(
            "https://quotes.example".to_owned(),
            slot,
            grant,
            contract_hash.to_owned(),
            None,
        )
        .map_err(|_| "test deployment pin construction failed")?,
    ))
}

async fn disabled_and_revoked_case(context: &mut CaseContext) -> CaseResult {
    let before = stream_counts(context).await?;
    let disabled = context
        .router(MarketStreamReadConfig::Disabled, None)
        .await?;
    let identities = sorted_identities(context.fixture.identities.clone());
    let consumer = Uuid::new_v4();
    let disabled_response = post_lease(
        context,
        &disabled,
        &context.users.owner,
        consumer,
        0,
        &identities,
        "wp4h-disabled",
    )
    .await?;
    ensure(
        disabled_response.status() == StatusCode::SERVICE_UNAVAILABLE,
        "disabled market stream did not return unavailable",
    )?;
    ensure(
        response_code(context, disabled_response).await? == "FEATURE_DISABLED",
        "disabled market stream returned the wrong finite error",
    )?;

    let valid_pins = context.owner_config()?;
    let wrong_slot = owner_only_pins(
        Uuid::new_v4(),
        context.fixture.grant_id,
        OWNER_CONTRACT_HASH,
    )?;
    let wrong_grant = owner_only_pins(
        context.fixture.credential_slot_id,
        Uuid::new_v4(),
        OWNER_CONTRACT_HASH,
    )?;
    let wrong_hash = owner_only_pins(
        context.fixture.credential_slot_id,
        context.fixture.grant_id,
        &"a".repeat(64),
    )?;
    for (pins, key) in [
        (wrong_slot, "wp4h-pin-slot"),
        (wrong_grant, "wp4h-pin-grant"),
        (wrong_hash, "wp4h-pin-contract"),
    ] {
        let wrong = context.router(pins, None).await?;
        let denied = post_lease(
            context,
            &wrong,
            &context.users.owner,
            consumer,
            0,
            &identities,
            key,
        )
        .await?;
        ensure(
            denied.status() == StatusCode::FORBIDDEN,
            "wrong immutable deployment binding was not denied",
        )?;
        ensure(
            response_code(context, denied).await? == "FORBIDDEN",
            "wrong deployment binding returned the wrong typed error",
        )?;
    }
    ensure(
        stream_counts(context).await? == before,
        "disabled or wrong deployment binding wrote lease or demand rows",
    )?;

    let valid = context.router(valid_pins, None).await?;
    let lease = create_lease_id(
        context,
        &valid,
        &context.users.owner,
        Uuid::new_v4(),
        &identities,
        "wp4h-revoked-rights-lease",
    )
    .await?;
    let response = get_stream(context, &valid, &context.users.owner, lease.lease_id, None).await?;
    ensure(
        response.status() == StatusCode::OK,
        "active rights stream did not open",
    )?;
    let mut reader = SseReader::new(response.into_body());
    timeout(
        Duration::from_secs(2),
        read_initial_snapshot(&mut reader, &fixture_membership_ids(&context.fixture)),
    )
    .await
    .map_err(|_| "active rights initial snapshot exceeded two seconds")??;
    boundary::revoke_entitlement(context.db(), context.fixture.grant_id)
        .await
        .map_err(|_| "synthetic entitlement revocation failed")?;
    access_revoked_then_eof(&mut reader).await?;
    drop(reader);

    let before_denied_write = stream_counts(context).await?;
    let denied_write = post_lease(
        context,
        &valid,
        &context.users.owner,
        Uuid::new_v4(),
        0,
        &identities,
        "wp4h-revoked-rights-new-lease",
    )
    .await?;
    ensure(
        denied_write.status() == StatusCode::FORBIDDEN,
        "revoked entitlement admitted a new demand lease",
    )?;
    ensure(
        response_code(context, denied_write).await? == "FORBIDDEN"
            && stream_counts(context).await? == before_denied_write,
        "revoked entitlement changed demand rows or returned an unexpected error",
    )?;

    let closed_app = context.app_pool(4, "wp4h-market-stream-closed").await?;
    let closed_router = context
        .router(context.owner_config()?, Some(closed_app.clone()))
        .await?;
    timeout(Duration::from_secs(5), closed_app.close())
        .await
        .map_err(|_| "owned app pool close did not finish")?;
    let unavailable = get_stream(
        context,
        &closed_router,
        &context.users.owner,
        lease.lease_id,
        None,
    )
    .await?;
    ensure(
        unavailable.status() == StatusCode::SERVICE_UNAVAILABLE,
        "closed owned app pool did not produce typed unavailable response",
    )?;
    ensure(
        response_code(context, unavailable).await? == "MARKET_STREAM_UNAVAILABLE",
        "closed owned app pool exposed an unexpected error class",
    )
}
