//! Shared DB-backed HTTP harness for the api-server contract suites.
//!
//! Conventions (inherited from the Todo 3b/19/23 harnesses): `DATABASE_URL`
//! is required (tests skip cleanly without it); each test gets a FRESH
//! scratch database with the production roles bootstrapped cluster-wide and
//! all migrations (0001..0010) applied; tenant rows are seeded through an
//! `app.actor_user_id`-GUC'd pool so RLS accepts them; shared system-owned
//! rows are seeded as the table owner (`migration_owner`).
//!
//! The router under test is the real `api_server::http::api_router` built
//! over the scratch database, exercised through `tower::ServiceExt::oneshot`
//! with real session cookies (opaque value -> sha256 hash stored in
//! `web_sessions`), CSRF tokens, and `X-Request-Id` headers.

#![allow(dead_code)]

use api_server::http::api_router;
use api_server::http::state::{
    ApiConfig, ApiState, OwnerBetaAccessMode, OwnerBetaPaperMode, OwnerIntradayQuoteReadConfig,
};
use auth::entitlement::{Actor, Role};
use auth::sessions::cookie;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use axum::response::Response;
use http_body_util::BodyExt;
use sha2::{Digest, Sha256};
use sqlx::ConnectOptions;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{PgPool, Row};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tower::ServiceExt;
use uuid::Uuid;

/// sha256 hex of a byte slice (harness manifests must match the DB CHECK).
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

fn checked_simulation_result_bytes(
    input: &job_queue::owner_beta::simulation_execution::SimulationJobInput,
    profile: job_queue::owner_beta::simulation::CostProfile,
) -> Vec<u8> {
    use job_queue::owner_beta::simulation::{
        EngineIdentity, EvaluationPriceRow, Price4, SignalSnapshot, SimulationResult,
        StrategyIdentity, simulate_ledgers,
    };

    let schedule = input.schedule();
    let engine = EngineIdentity::new(
        &"a".repeat(40),
        &format!("sha256:{}", "a".repeat(64)),
        "rustc-test",
        "x86_64-unknown-linux-gnu",
        "test",
    )
    .expect("synthetic declarative engine identity");
    let factor_hash = format!("sha256:{}", "b".repeat(64));
    let target_hash = format!("sha256:{}", "c".repeat(64));
    let mut targets = market_data::KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| {
            let instrument = format!("{symbol}.KRX");
            let weight = if instrument == "069500.KRX" {
                1_000_000
            } else {
                0
            };
            (instrument, weight)
        })
        .collect::<Vec<_>>();
    targets.sort_by(|left, right| left.0.cmp(&right.0));
    let signals = schedule
        .signal_execution_pairs()
        .into_iter()
        .map(|(signal_date, _)| {
            SignalSnapshot::new(
                &signal_date.to_iso(),
                &factor_hash,
                &target_hash,
                targets.clone(),
                0,
            )
            .expect("synthetic checked signal")
        })
        .collect::<Vec<_>>();
    let open = Price4::parse("100.0000").expect("synthetic open price");
    let close = Price4::parse("101.0000").expect("synthetic close price");
    let prices = schedule
        .evaluation_sessions()
        .iter()
        .map(|session| {
            EvaluationPriceRow::new(&session.to_iso(), open.clone(), close.clone(), 1_000_000)
                .expect("synthetic checked evaluation price")
        })
        .collect::<Vec<_>>();
    let ledgers = simulate_ledgers(schedule, &signals, &prices)
        .expect("synthetic production-capital ledgers");
    let strategy = StrategyIdentity::trend_following_v1(
        input.strategy().config_sha256().as_str(),
        "1.0.0",
        &format!("sha256:{}", "d".repeat(64)),
    )
    .expect("synthetic fixed strategy identity");
    let profile_ledgers = match profile {
        job_queue::owner_beta::simulation::CostProfile::Illustrative10BpPerSideV1 => {
            ledgers.illustrative_10bp_per_side()
        }
        job_queue::owner_beta::simulation::CostProfile::ZeroCostReferenceV1 => {
            ledgers.zero_cost_reference()
        }
    };
    SimulationResult::new(
        engine,
        strategy,
        schedule.clone(),
        profile,
        signals,
        profile_ledgers.strategy().clone(),
        profile_ledgers.benchmark().clone(),
    )
    .expect("synthetic checked simulation result")
    .canonical_json()
    .expect("canonical simulation result bytes")
}

fn checked_simulation_pair_bytes(
    fixture: &SimulationFixture,
) -> (Vec<u8>, Vec<u8>, serde_json::Value) {
    use job_queue::owner_beta::simulation::CostProfile;
    use job_queue::owner_beta::simulation_execution::check_stored_simulation_pair;

    let input = Harness::simulation_test_input(
        fixture.strategy_config_id,
        fixture.run_id,
        "2026-08-27",
        "2026-08-28",
    );
    let illustrative =
        checked_simulation_result_bytes(&input, CostProfile::Illustrative10BpPerSideV1);
    let zero_cost = checked_simulation_result_bytes(&input, CostProfile::ZeroCostReferenceV1);
    let illustrative_hash = format!("sha256:{}", sha256_hex(&illustrative));
    let zero_cost_hash = format!("sha256:{}", sha256_hex(&zero_cost));
    let engine_json = serde_json::json!({
        "git_commit": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "binary_sha256": format!("sha256:{}", "a".repeat(64)),
        "toolchain": "rustc-test",
        "target": "x86_64-unknown-linux-gnu",
        "profile": "test"
    });
    let engine = job_queue::owner_beta::simulation::EngineIdentity::new(
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        &format!("sha256:{}", "a".repeat(64)),
        "rustc-test",
        "x86_64-unknown-linux-gnu",
        "test",
    )
    .expect("synthetic engine identity validates");
    let _checked = check_stored_simulation_pair(
        &illustrative,
        &illustrative_hash,
        &zero_cost,
        &zero_cost_hash,
        &input,
        &engine,
    )
    .expect("production checker accepts the complete synthetic pair");
    (illustrative, zero_cost, engine_json)
}

/// Roles bootstrapped cluster-wide by the harness (migrations grant to them).
pub const ROLE_BOOTSTRAP_SQL: &str = r#"
SELECT pg_advisory_lock(424242001);
DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'migration_owner') THEN
    CREATE ROLE migration_owner LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD 'lagrange';
  END IF;
END $role$;
DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'app') THEN
    CREATE ROLE app LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD 'lagrange';
  END IF;
END $role$;
DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'worker') THEN
    CREATE ROLE worker LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD 'lagrange';
  END IF;
END $role$;
DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'audit_writer') THEN
    CREATE ROLE audit_writer LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD 'lagrange';
  END IF;
END $role$;
DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'research_writer') THEN
    CREATE ROLE research_writer LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD 'lagrange';
  END IF;
END $role$;
DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'admin') THEN
    CREATE ROLE admin LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD 'lagrange';
  END IF;
END $role$;
DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'daily_input_admitter') THEN
    CREATE ROLE daily_input_admitter LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE
      NOREPLICATION NOBYPASSRLS PASSWORD 'lagrange';
  END IF;
END $role$;
DO $role_preflight$ BEGIN
  IF NOT EXISTS (
    SELECT 1
    FROM pg_roles
    WHERE rolname = 'daily_input_admitter'
      AND rolcanlogin
      AND NOT rolinherit
      AND NOT rolsuper
      AND NOT rolcreatedb
      AND NOT rolcreaterole
      AND NOT rolreplication
      AND NOT rolbypassrls
  ) THEN
    RAISE EXCEPTION 'daily_input_admitter role attributes mismatch';
  END IF;

  IF EXISTS (
    SELECT 1
    FROM pg_auth_members
    WHERE member = (SELECT oid FROM pg_roles WHERE rolname = 'daily_input_admitter')
       OR roleid = (SELECT oid FROM pg_roles WHERE rolname = 'daily_input_admitter')
  ) THEN
    RAISE EXCEPTION 'daily_input_admitter has unexpected role membership';
  END IF;

  IF EXISTS (
    WITH RECURSIVE membership_closure(start_role, reachable_role) AS (
      SELECT oid, oid
      FROM pg_roles
      WHERE rolname IN ('app', 'worker')
      UNION
      SELECT closure.start_role, membership.roleid
      FROM membership_closure AS closure
      JOIN pg_auth_members AS membership ON membership.member = closure.reachable_role
    )
    SELECT 1
    FROM membership_closure AS closure
    JOIN pg_roles AS start_role ON start_role.oid = closure.start_role
    JOIN pg_roles AS reachable_role ON reachable_role.oid = closure.reachable_role
    WHERE start_role.rolname IN ('app', 'worker')
      AND reachable_role.rolname IN ('migration_owner', 'daily_input_admitter')
  ) THEN
    RAISE EXCEPTION 'app or worker reaches a protected migration role';
  END IF;
END $role_preflight$;
GRANT USAGE ON SCHEMA public TO migration_owner, app, worker, audit_writer, research_writer, admin;
GRANT CREATE ON SCHEMA public TO migration_owner;
SELECT pg_advisory_unlock(424242001);
"#;

/// Per-scratch-database privileges for the isolated daily-input test role.
const DAILY_INPUT_ADMITTER_DB_BOOTSTRAP_SQL: &str = r#"
DO $database_privilege$ BEGIN
  EXECUTE format('GRANT CONNECT ON DATABASE %I TO daily_input_admitter', current_database());
END $database_privilege$;
REVOKE CREATE ON SCHEMA public FROM PUBLIC;
GRANT USAGE ON SCHEMA public TO daily_input_admitter;
GRANT CREATE ON SCHEMA public TO migration_owner;
DO $privilege_preflight$ BEGIN
  IF NOT has_database_privilege('daily_input_admitter', current_database(), 'CONNECT') THEN
    RAISE EXCEPTION 'daily_input_admitter lacks CONNECT on the scratch database';
  END IF;
  IF NOT has_schema_privilege('daily_input_admitter', 'public', 'USAGE') THEN
    RAISE EXCEPTION 'daily_input_admitter lacks USAGE on scratch public schema';
  END IF;
  IF has_database_privilege('daily_input_admitter', current_database(), 'CREATE') THEN
    RAISE EXCEPTION 'daily_input_admitter has CREATE on the scratch database';
  END IF;
  IF has_schema_privilege('daily_input_admitter', 'public', 'CREATE') THEN
    RAISE EXCEPTION 'daily_input_admitter has CREATE on scratch public schema';
  END IF;
  IF NOT has_schema_privilege('migration_owner', 'public', 'CREATE') THEN
    RAISE EXCEPTION 'migration_owner lacks CREATE on scratch public schema';
  END IF;
END $privilege_preflight$;
"#;

/// The sqlx Migrator embedding the real workspace migrations.
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

fn fresh_db_name() -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_millis();
    format!(
        "api24_{}_{}_{}",
        std::process::id(),
        ts,
        SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

pub(crate) fn conn_url(super_url: &str, role: &str, db: &str) -> String {
    let (without_query, query) = match super_url.split_once('?') {
        Some((url, query)) => (url, Some(query)),
        None => (super_url, None),
    };
    let (_scheme, rest) = without_query
        .split_once("://")
        .expect("DATABASE_URL must start with a scheme");
    let (auth, hostport_db) = rest.split_once('@').expect("DATABASE_URL must contain @");
    let (_, pw) = match auth.split_once(':') {
        Some((_, p)) => ((), Some(p)),
        None => ((), None),
    };
    let (hostport, _old_db) = hostport_db
        .rsplit_once('/')
        .expect("DATABASE_URL must contain a database path");
    let mut url = match pw {
        Some(p) => format!("postgres://{role}:{p}@{hostport}/{db}"),
        None => format!("postgres://{role}@{hostport}/{db}"),
    };
    if let Some(query) = query {
        url.push('?');
        url.push_str(query);
    }
    url
}

/// A test user: identity + a live session cookie + a valid CSRF token.
#[derive(Debug, Clone)]
pub struct UserCtx {
    pub user_id: Uuid,
    pub role: Role,
    /// The opaque session cookie value (plaintext; sha256-hashed in DB).
    pub cookie_value: String,
    /// A valid synchronizer token (stored as sha256 hash in DB).
    pub csrf_token: String,
}

impl UserCtx {
    pub fn actor(&self) -> Actor {
        Actor::new(self.user_id.to_string(), self.role)
    }
}

/// The assembled test world: fresh DB, seeded baseline, live router.
pub struct Harness {
    pub db_name: String,
    pub app: axum::Router,
    pub owner: UserCtx,
    pub member: UserCtx,
    pub app_pool: PgPool,
    pub admin_pool: PgPool,
    pub audit_pool: PgPool,
    /// migration_owner pool: seeds shared system-owned rows.
    pub owner_pool: PgPool,
    /// base app-role URL used to build actor-GUC pools for queue calls.
    pub app_url: String,
    /// worker-role URL: the daemon connection the Paper runner writes the
    /// ledger through. The `worker` policies are `USING (true)` and carry no
    /// actor GUC, so this pool needs none — every statement binds its own
    /// `account_id`/`owner_user_id` predicate instead.
    pub worker_url: String,
    /// Research publication role used by end-to-end Raw ingestion tests.
    pub research_writer_url: String,
    /// Temp artifact tree the download route hashes against (C: temp, keeps
    /// D: safe; mirrors the read-only /data/artifacts mount in compose).
    pub artifact_root: std::path::PathBuf,
    /// The state the router runs on; `None` only while `new()` assembles it.
    state: Option<ApiState>,
}

/// Read DATABASE_URL or return None (tests skip).
fn base_url() -> Option<String> {
    std::env::var("DATABASE_URL").ok()
}

async fn pool(url: &str, max: u32) -> PgPool {
    let opts = url
        .parse::<PgConnectOptions>()
        .expect("pool url parses")
        .log_statements(tracing::log::LevelFilter::Off);
    PgPoolOptions::new()
        .max_connections(max)
        .acquire_timeout(Duration::from_secs(20))
        .connect_with(opts)
        .await
        .expect("harness pool connects")
}

/// Build a pool whose connections carry the actor GUC (queue-crate calls).
pub async fn actor_pool(url: &str, user_id: &str, max: u32) -> PgPool {
    let opts = url
        .parse::<PgConnectOptions>()
        .expect("app url parses")
        .options([("app.actor_user_id", user_id.to_string())]);
    PgPoolOptions::new()
        .max_connections(max)
        .acquire_timeout(Duration::from_secs(20))
        .connect_with(opts)
        .await
        .expect("actor pool connects")
}

/// Serializes the cluster-wide role bootstrap within one test process
/// (tokio threads run tests concurrently; the DB advisory lock already
/// serializes across processes).
static BOOTSTRAP_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

impl Harness {
    /// Create a disposable database, bootstrap roles, run migrations, seed
    /// the baseline (users, sessions, instruments, strategies, datasets,
    /// entitlements), and build the real router.
    pub async fn new() -> Option<Harness> {
        let _bootstrap_guard = BOOTSTRAP_LOCK.lock().await;
        let super_url = base_url()?;
        let db_name = fresh_db_name();
        let super_pool = pool(&super_url, 2).await;
        // Roles FIRST: the CREATE DATABASE below names migration_owner as the
        // owner, so on a cluster that has never hosted this suite the role must
        // already exist. Doing it the other way round only ever worked because
        // a previous run had left the roles behind - it failed immediately on
        // the fresh QA cluster (42704: role "migration_owner" does not exist).
        sqlx::raw_sql(ROLE_BOOTSTRAP_SQL)
            .execute(&super_pool)
            .await
            .expect("bootstrap roles");
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "CREATE DATABASE \"{db_name}\" OWNER migration_owner"
        )))
        .execute(&super_pool)
        .await
        .expect("create scratch db");
        drop(super_pool);

        let setup_options = super_url
            .parse::<PgConnectOptions>()
            .unwrap_or_else(|_| panic!("scratch setup connection options parse"))
            .database(&db_name)
            .log_statements(tracing::log::LevelFilter::Off);
        let target_setup = PgPoolOptions::new()
            .max_connections(2)
            .acquire_timeout(Duration::from_secs(20))
            .connect_with(setup_options)
            .await
            .unwrap_or_else(|_| panic!("scratch database setup connection opens"));
        sqlx::raw_sql(DAILY_INPUT_ADMITTER_DB_BOOTSTRAP_SQL)
            .execute(&target_setup)
            .await
            .unwrap_or_else(|_| panic!("scratch daily-input role privileges are prepared"));
        target_setup.close().await;

        let owner_url = conn_url(&super_url, "migration_owner", &db_name);
        let app_url = conn_url(&super_url, "app", &db_name);
        let admin_url = conn_url(&super_url, "admin", &db_name);
        let audit_url = conn_url(&super_url, "audit_writer", &db_name);
        let worker_url = conn_url(&super_url, "worker", &db_name);
        let research_writer_url = conn_url(&super_url, "research_writer", &db_name);

        let owner_pool = pool(&owner_url, 4).await;
        MIGRATOR.run(&owner_pool).await.expect("migrations run");

        let app_pool = pool(&app_url, 8).await;
        let admin_pool = pool(&admin_url, 4).await;
        let audit_pool = pool(&audit_url, 4).await;

        let artifact_root = std::env::temp_dir().join(format!(
            "lagrange-artifacts-{}-{}",
            std::process::id(),
            db_name
        ));
        std::fs::create_dir_all(&artifact_root).expect("artifact root creates");
        #[cfg(unix)]
        std::fs::set_permissions(&artifact_root, std::fs::Permissions::from_mode(0o700))
            .expect("artifact root permissions set to 0700");

        let mut h = Harness {
            db_name,
            app: axum::Router::new(),
            owner: UserCtx {
                user_id: Uuid::nil(),
                role: Role::Owner,
                cookie_value: String::new(),
                csrf_token: String::new(),
            },
            member: UserCtx {
                user_id: Uuid::nil(),
                role: Role::Member,
                cookie_value: String::new(),
                csrf_token: String::new(),
            },
            app_pool,
            admin_pool,
            audit_pool,
            owner_pool,
            app_url,
            worker_url,
            research_writer_url,
            artifact_root,
            state: None,
        };

        h.seed_shared(
            "INSERT INTO roles (id, description) VALUES ('owner','Owner'), ('member','Member') \
             ON CONFLICT (id) DO NOTHING",
        )
        .await;

        // Fixed universe subset + benchmark (canonical ids from Todo 12).
        h.seed_shared(
            "INSERT INTO instruments (id, symbol, venue, currency, name, asset_class, status) VALUES \
             ('069500.KRX','069500','KRX','KRW','KODEX 200','ETF','ACTIVE'), \
             ('229200.KRX','229200','KRX','KRW','KODEX Korea MSCI','ETF','ACTIVE'), \
             ('114260.KRX','114260','KRX','KRW','KODEX 200 Futures Inverse 2X','ETF','ACTIVE'), \
             ('SPY.ARCA','SPY','ARCA','USD','SPDR S&P 500','ETF','ACTIVE') \
             ON CONFLICT (id) DO NOTHING",
        )
        .await;
        h.seed_shared(
            "INSERT INTO strategies (id, display_name, description, risk_description, state) VALUES \
             ('buy_and_hold','Buy & Hold','hold the benchmark','benchmark risk','Validated'), \
             ('trend_following','Trend Following','MA crossover','trend risk','Validated') \
             ON CONFLICT (id) DO NOTHING",
        )
        .await;
        h.seed_shared(
            "INSERT INTO strategy_versions (id, strategy_id, version, required_factors, min_lookback, supported_market, cadence) VALUES \
             (gen_random_uuid(),'buy_and_hold','1.0.0','[]'::jsonb,1,'KRX','daily'), \
             (gen_random_uuid(),'trend_following','1.0.0','[\"trend\"]'::jsonb,50,'KRX','daily')",
        )
        .await;
        h.seed_shared(
            "INSERT INTO strategy_parameter_schemas (id, strategy_id, version, schema_json) VALUES \
             (gen_random_uuid(),'buy_and_hold','1.0.0', '{\"type\":\"object\",\"properties\":{\"lookback\":{\"type\":\"integer\",\"minimum\":1,\"maximum\":500}},\"additionalProperties\":false,\"required\":[\"lookback\"]}'::jsonb), \
             (gen_random_uuid(),'trend_following','1.0.0', '{\"type\":\"object\",\"properties\":{\"fast_ma\":{\"type\":\"integer\",\"minimum\":1},\"slow_ma\":{\"type\":\"integer\",\"minimum\":1},\"allocation\":{\"type\":\"number\",\"minimum\":0.0,\"maximum\":1.0}},\"additionalProperties\":false,\"required\":[\"fast_ma\",\"slow_ma\"]}'::jsonb)",
        )
        .await;
        // Datasets: READY + WARNING + BLOCKED (freshness/quality states).
        h.seed_shared(
            "INSERT INTO dataset_versions (id, dataset_id, version, status, manifest_sha256, storage_path) VALUES \
             (gen_random_uuid(),'krx_eod_bars','2026-01-01','READY', repeat('a',64), 'data/curated/krx_eod_bars/2026-01-01'), \
             (gen_random_uuid(),'krx_eod_bars','2026-01-02','WARNING', repeat('b',64), 'data/curated/krx_eod_bars/2026-01-02'), \
             (gen_random_uuid(),'krx_eod_bars','2026-01-03','BLOCKED', repeat('c',64), 'data/curated/krx_eod_bars/2026-01-03')",
        )
        .await;
        h.seed_shared(
            "INSERT INTO data_quality_issues (id, dataset_id, dataset_version, issue_code, severity, detail_json) VALUES \
             (gen_random_uuid(),'krx_eod_bars','2026-01-03','MISSING_REQUIRED_BAR','ERROR','{\"instrument\":\"069500.KRX\"}'::jsonb), \
             (gen_random_uuid(),'krx_eod_bars','2026-01-02','DATA_STALE','WARNING','{\"sessions\":2}'::jsonb)",
        )
        .await;

        let recommendation_dataset: (Uuid, String, String, String) = sqlx::query_as(
            "SELECT id, dataset_id, version, manifest_sha256 \
             FROM dataset_versions WHERE status = 'READY' ORDER BY created_at LIMIT 1",
        )
        .fetch_one(&h.owner_pool)
        .await
        .expect("configured recommendation dataset exists");

        // Users + sessions.
        h.owner = h
            .seed_user(Role::Owner, "owner@lagrange.test", "owner-iss", "owner-sub")
            .await;
        h.member = h
            .seed_user(
                Role::Member,
                "member@lagrange.test",
                "member-iss",
                "member-sub",
            )
            .await;

        // ACTIVE entitlement covering the platform (managed by the owner).
        h.seed_shared(
            "INSERT INTO data_entitlements \
             (id, contract_document_sha256, contract_reference, status, covered_datasets, covered_uses, effective_from, effective_until, managed_by) \
             VALUES \
             (gen_random_uuid(), repeat('d',64), 'krx-2026-01', 'ACTIVE', '[\"krx_eod_bars\"]'::jsonb, \
              '[\"dataset\",\"factor\",\"recommendation\",\"backtest\",\"report\",\"benchmark\",\"paper_view\",\"payload\",\"download\"]'::jsonb, \
              '2026-01-01', '2026-12-31', (SELECT id FROM users WHERE email='owner@lagrange.test')), \
             (gen_random_uuid(), repeat('e',64), 'krx-expired', 'EXPIRED', '[\"krx_eod_bars\"]'::jsonb, \
              '[\"dataset\",\"factor\",\"recommendation\",\"backtest\",\"report\",\"benchmark\",\"paper_view\",\"payload\",\"download\"]'::jsonb, \
              '2020-01-01', '2020-12-31', (SELECT id FROM users WHERE email='owner@lagrange.test'))",
        )
        .await;

        let cfg = ApiConfig {
            cursor_secret: *b"api24-cursor-secret-0123456789ab",
            max_jobs_per_owner: 10,
            recommendation_dataset: job_queue::recommendation::input::DatasetPin {
                id: recommendation_dataset.0,
                dataset_id: recommendation_dataset.1,
                version: recommendation_dataset.2,
                curated_version: 2,
                manifest_sha256: recommendation_dataset.3,
            },
            db_url: h.app_url.clone(),
            step_up_max_auth_age_secs: 900,
            artifact_root: h.artifact_root.clone(),
            seoul_today: || chrono::NaiveDate::from_ymd_opt(2026, 8, 13).unwrap(),
            candidate_eod_ready: || true,
            code_commit: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            owner_beta_access: OwnerBetaAccessMode::Disabled,
            owner_beta_paper: OwnerBetaPaperMode::Disabled,
            owner_beta_price_input: api_server::http::state::OwnerBetaPriceInputMode::Disabled,
            owner_beta_daily:
                job_queue::owner_beta::runner::daily_runtime::DailyProcessMode::Disabled,
            owner_beta_equity_signals:
                api_server::http::state::OwnerBetaEquitySignalsMode::Disabled,
            stock_price_beta_artifact_root: h.artifact_root.clone(),
            owner_equity_v2_pins: None,
            owner_equity_v2_api_artifact_root: None,
            owner_intraday_quotes: api_server::http::state::OwnerIntradayQuoteReadConfig::Disabled,
            owner_market_stream:
                api_server::http::owner_market_stream_config::MarketStreamReadConfig::Disabled,
            intraday_now: api_server::http::state::system_intraday_now,
        };
        let state = ApiState::from_pools(
            cfg,
            h.app_pool.clone(),
            h.admin_pool.clone(),
            h.audit_pool.clone(),
        )
        .await
        .expect("api state builds from pools");
        h.state = Some(state.clone());
        h.app = api_router(state);
        Some(h)
    }

    /// The live `ApiState` behind the router.
    ///
    /// Services that have no HTTP surface of their own — the Paper runner's
    /// settle-and-announce path (Todo 32) — are driven through this, exactly
    /// as the runner process will drive them.
    pub fn state(&self) -> ApiState {
        self.state.clone().expect("state built during Harness::new")
    }

    /// Rebuild the HTTP state from the same database, as a new API process
    /// would.  In particular, this deliberately drops the in-memory
    /// idempotency cache so tests exercise the durable replay branch.
    pub async fn restart_api(&mut self) {
        let cfg = (*self.state().cfg).clone();
        self.restart_api_with_config(cfg).await;
    }

    /// Rebuild the HTTP state with a supplied immutable recommendation pin.
    /// A test-local curated fixture uses this to model deployment config
    /// pointing at the attested data it just created.
    pub async fn restart_api_with_recommendation_dataset(
        &mut self,
        recommendation_dataset: job_queue::recommendation::input::DatasetPin,
    ) {
        let mut cfg = (*self.state().cfg).clone();
        cfg.recommendation_dataset = recommendation_dataset;
        self.restart_api_with_config(cfg).await;
    }

    /// Rebuild the router with the explicit owner-beta admission policy.
    pub async fn restart_api_with_owner_beta_access(&mut self) {
        let mut cfg = (*self.state().cfg).clone();
        cfg.owner_beta_access = OwnerBetaAccessMode::OwnerOnly;
        self.restart_api_with_config(cfg).await;
    }

    pub async fn restart_api_with_owner_beta_disabled(&mut self) {
        let mut cfg = (*self.state().cfg).clone();
        cfg.owner_beta_access = OwnerBetaAccessMode::Disabled;
        self.restart_api_with_config(cfg).await;
    }

    pub async fn restart_api_with_owner_beta_paper(&mut self) {
        let mut cfg = (*self.state().cfg).clone();
        cfg.owner_beta_access = OwnerBetaAccessMode::OwnerOnly;
        cfg.owner_beta_paper = OwnerBetaPaperMode::Enabled;
        self.restart_api_with_config(cfg).await;
    }

    pub async fn restart_api_with_owner_beta_price_input(&mut self) {
        let mut cfg = (*self.state().cfg).clone();
        cfg.owner_beta_access = OwnerBetaAccessMode::OwnerOnly;
        cfg.owner_beta_price_input = api_server::http::state::OwnerBetaPriceInputMode::SealedV1;
        self.restart_api_with_config(cfg).await;
    }

    pub async fn restart_api_with_max_jobs_per_owner(&mut self, max_jobs: u32) {
        let mut cfg = (*self.state().cfg).clone();
        cfg.max_jobs_per_owner = max_jobs;
        self.restart_api_with_config(cfg).await;
    }

    async fn restart_api_with_config(&mut self, cfg: ApiConfig) {
        let state = ApiState::from_pools(
            cfg,
            self.app_pool.clone(),
            self.admin_pool.clone(),
            self.audit_pool.clone(),
        )
        .await
        .expect("api state rebuilds from pools");
        self.state = Some(state.clone());
        self.app = api_router(state);
    }

    /// GUC'd verification pool scoped to the member actor (tenant reads).
    pub async fn member_pool(&self) -> PgPool {
        actor_pool(&self.app_url, &self.member.user_id.to_string(), 2).await
    }

    /// A plain `worker`-role pool: the Paper runner's own connection.
    ///
    /// No actor GUC, deliberately. That is exactly the production shape — the
    /// runner is a daemon serving every tenant — and it is what makes the
    /// engine's explicit `account_id`/`owner_user_id` predicates load-bearing
    /// rather than decorative.
    pub async fn worker_pool(&self) -> PgPool {
        pool(&self.worker_url, 4).await
    }

    /// A direct-login pool for the isolated daily-input admission role.
    ///
    /// The caller owns the returned pool and must close it before dropping
    /// this scratch database. It carries no SET ROLE or actor GUC setup.
    pub async fn daily_input_admitter_pool(&self) -> PgPool {
        let options = self
            .app_url
            .parse::<PgConnectOptions>()
            .unwrap_or_else(|_| panic!("daily input admitter connection options parse"))
            .username("daily_input_admitter")
            .password("lagrange")
            .log_statements(tracing::log::LevelFilter::Off);
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .acquire_timeout(Duration::from_secs(20))
            .connect_with(options)
            .await
            .unwrap_or_else(|_| panic!("daily input admitter test pool connects"));
        let identities: (String, String, Option<String>) = sqlx::query_as(
            "SELECT current_user, session_user, current_setting('app.actor_user_id', true)",
        )
        .fetch_one(&pool)
        .await
        .unwrap_or_else(|_| panic!("daily input admitter identity check runs"));
        assert_eq!(identities.0, "daily_input_admitter");
        assert_eq!(identities.1, "daily_input_admitter");
        assert!(
            identities.2.is_none(),
            "admitter pool must not carry actor GUC"
        );
        pool
    }

    pub async fn research_writer_pool(&self) -> PgPool {
        pool(&self.research_writer_url, 4).await
    }

    pub async fn restart_api_with_daily(
        &mut self,
        mode: job_queue::owner_beta::runner::daily_runtime::DailyProcessMode,
    ) {
        let mut cfg = (*self.state().cfg).clone();
        cfg.owner_beta_access = OwnerBetaAccessMode::OwnerOnly;
        cfg.owner_beta_daily = mode;
        self.restart_api_with_config(cfg).await;
    }

    pub async fn restart_api_with_candidate_clock(
        &mut self,
        seoul_today: fn() -> chrono::NaiveDate,
        candidate_eod_ready: fn() -> bool,
    ) {
        let mut cfg = (*self.state().cfg).clone();
        cfg.seoul_today = seoul_today;
        cfg.candidate_eod_ready = candidate_eod_ready;
        self.restart_api_with_config(cfg).await;
    }

    pub async fn restart_api_with_intraday_read_config(
        &mut self,
        config: OwnerIntradayQuoteReadConfig,
        clock: fn() -> chrono::DateTime<chrono::Utc>,
    ) {
        let mut cfg = (*self.state().cfg).clone();
        cfg.owner_intraday_quotes = config;
        cfg.intraday_now = clock;
        self.restart_api_with_config(cfg).await;
    }

    /// Configure only the chart route's read-only V2 artifact root. Existing
    /// V2 lifecycle and signal routes deliberately ignore this optional path.
    pub async fn restart_api_with_owner_equity_v2_artifact_root(
        &mut self,
        root: Option<std::path::PathBuf>,
    ) {
        let mut cfg = (*self.state().cfg).clone();
        cfg.owner_equity_v2_api_artifact_root = root;
        self.restart_api_with_config(cfg).await;
    }

    /// The `pending_targets` repository over the harness app pool.
    ///
    /// Todo 31 has no HTTP surface for queueing/settling a target — that is
    /// the scheduler's own path — so the scheduler suite drives the typed
    /// repository directly, exactly as the runner will.
    pub fn state_pending_targets(&self) -> api_server::repos::pending_targets::PendingTargetRepo {
        api_server::repos::pending_targets::PendingTargetRepo::new(self.app_pool.clone())
    }

    /// Drop the scratch database (call at the end of every test).
    pub async fn teardown(self) {
        drop(self.app_pool);
        drop(self.admin_pool);
        drop(self.audit_pool);
        drop(self.owner_pool);
        let _ = std::fs::remove_dir_all(&self.artifact_root);
        let super_url = base_url().expect("DATABASE_URL still set");
        let super_pool = pool(&super_url, 2).await;
        let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS \"{}\" WITH (FORCE)",
            self.db_name
        )))
        .execute(&super_pool)
        .await;
    }

    /// Write artifact bytes under the harness artifact root; returns the
    /// sha256 hex the manifest row must carry.
    pub fn write_artifact(&self, rel: &str, bytes: &[u8]) -> String {
        let path = self.artifact_root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("artifact parent dirs create");
        }
        std::fs::write(&path, bytes).expect("artifact file writes");
        sha256_hex(bytes)
    }

    /// Seed a row into a shared system-owned table (as migration_owner).
    pub async fn seed_shared(&self, sql: &str) {
        sqlx::query(sqlx::AssertSqlSafe(sql.to_string()))
            .execute(&self.owner_pool)
            .await
            .unwrap_or_else(|e| panic!("seed_shared failed: {e} ({sql:?})"));
    }

    /// Seed a row into a tenant table under `actor`'s RLS context (app+GUC).
    pub async fn seed_tenant(&self, actor: &UserCtx, sql: &str) {
        let p = actor_pool(&self.app_url, &actor.user_id.to_string(), 2).await;
        sqlx::query(sqlx::AssertSqlSafe(sql.to_string()))
            .execute(&p)
            .await
            .unwrap_or_else(|e| panic!("seed_tenant failed: {e} ({sql:?})"));
    }

    /// Seed related tenant rows in one transaction so migration deferred
    /// guards see the complete job/run binding at commit.
    pub async fn seed_tenant_batch(&self, actor: &UserCtx, statements: &[String]) {
        let p = actor_pool(&self.app_url, &actor.user_id.to_string(), 2).await;
        let mut tx = p.begin().await.expect("actor seed transaction begins");
        for sql in statements {
            sqlx::query(sqlx::AssertSqlSafe(sql.clone()))
                .execute(&mut *tx)
                .await
                .unwrap_or_else(|e| panic!("seed_tenant_batch failed: {e}"));
        }
        tx.commit().await.expect("actor seed transaction commits");
    }

    pub async fn seed_simulation_backtest_rights(&self) {
        self.seed_shared(
            "UPDATE data_entitlements SET covered_uses = covered_uses - 'backtest' \
              WHERE contract_reference='krx-2026-01'",
        )
        .await;
        self.seed_shared(
            "INSERT INTO data_entitlements \
             (id, contract_document_sha256, contract_reference, status, covered_datasets, covered_uses, effective_from, effective_until, managed_by) \
             VALUES (gen_random_uuid(), repeat('f',64), 'simulation-api-test', 'ACTIVE', \
               '[\"krx_eod_bars\"]'::jsonb, '[\"backtest\"]'::jsonb, \
               '2016-08-29', '2026-12-31', \
               (SELECT id FROM users WHERE email='owner@lagrange.test'))",
        )
        .await;
    }

    pub async fn simulation_job_count(&self, actor: &UserCtx) -> i64 {
        let p = actor_pool(&self.app_url, &actor.user_id.to_string(), 1).await;
        sqlx::query_scalar(
            "SELECT count(*)::bigint FROM public.jobs
              WHERE owner_user_id=$1 AND job_type='owner_beta_price_only_simulation'",
        )
        .bind(actor.user_id)
        .fetch_one(&p)
        .await
        .expect("actor-scoped simulation job count")
    }

    pub async fn simulation_run_visible_to_actor(&self, actor: &UserCtx, run_id: Uuid) -> i64 {
        let p = actor_pool(&self.app_url, &actor.user_id.to_string(), 1).await;
        sqlx::query_scalar(
            "SELECT count(*)::bigint FROM public.owner_beta_simulation_runs WHERE id=$1",
        )
        .bind(run_id)
        .fetch_one(&p)
        .await
        .expect("actor-scoped simulation visibility query")
    }

    pub async fn simulation_run_visible_without_actor(&self, run_id: Uuid) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*)::bigint FROM public.owner_beta_simulation_runs WHERE id=$1",
        )
        .bind(run_id)
        .fetch_one(&self.app_pool)
        .await
        .expect("unscoped app-role simulation visibility query")
    }

    pub async fn revoke_simulation_backtest_rights(&self) {
        self.seed_shared(
            "UPDATE data_entitlements SET status='REVOKED' \
              WHERE contract_reference='simulation-api-test'",
        )
        .await;
    }

    pub async fn seed_simulation_strategy_config(&self, actor: &UserCtx) -> Uuid {
        let row = self
            .state()
            .strategy_configs()
            .create(
                &actor.actor(),
                api_server::repos::strategy_configs::NewStrategyConfig {
                    strategy_id: "trend_following".to_owned(),
                    strategy_version: "1.0.0".to_owned(),
                    config_json: serde_json::json!({
                        "benchmark_instrument": "069500.KRX",
                        "fast_ma": 50,
                        "slow_ma": 200
                    }),
                    is_active: true,
                },
            )
            .await
            .expect("fixed owner-beta strategy configuration is seeded");
        row.id
    }

    /// A validated synthetic input for the private library-test seam or a
    /// row fixture. It carries only the already-frozen contract pins and
    /// never reads or copies approved artifact bytes.
    pub fn simulation_test_input(
        strategy_config_id: Uuid,
        run_id: Uuid,
        evaluation_start: &str,
        evaluation_end: &str,
    ) -> job_queue::owner_beta::simulation_execution::SimulationJobInput {
        let wire = serde_json::json!({
            "schema": job_queue::owner_beta::simulation_execution::OWNER_BETA_PRICE_ONLY_SIMULATION_SCHEMA,
            "run_id": run_id,
            "strategy_config_id": strategy_config_id,
            "evaluation_start": evaluation_start,
            "evaluation_end": evaluation_end,
            "pins": {
                "candidate_content_sha256": "sha256:b0fb82f6a580f3def13a1e1e34bea68e30d95dc720306e7ee54b6c3199cf402d",
                "artifact_manifest_sha256": "sha256:53587f32ce67ae1a8488b9c00096465185d03e35b1b38a04f39ed5462da7f6b6",
                "stage5_manifest_sha256": "sha256:5b2900594e03e322d4e81adaa79fc6c40ae70035213786972ed5cb7a22870d2a",
                "action_manifest_sha256": "sha256:080d38142a6506f741114eb75a77a36c23c2554a0d39eba8843ff62bfb484550",
                "approval_registry_sha256": "sha256:5d3aa2b354d8c0c51d0d7d029e9fd3f92e0570fe074262bfc900829a5a2bb707"
            },
            "contract_document_sha256": job_queue::owner_beta::simulation::APPROVED_CONTRACT_SHA256,
            "bars_sha256": "sha256:23354c54708d7a458aadc4f2cb2fca77469969e91256b6d3b119101ebae98ffe",
            "strategy": {
                "strategy_id": "trend_following",
                "strategy_version": "1.0.0",
                "config_json": {
                    "benchmark_instrument": "069500.KRX",
                    "fast_ma": 50,
                    "slow_ma": 200
                },
                "config_sha256": "sha256:90e5b4c6972678e9186d4655d76496676e7cfb8dbe473a249f062963873c04a0"
            }
        });
        let bytes = serde_json::to_vec(&wire).expect("synthetic input serializes");
        job_queue::owner_beta::simulation_execution::SimulationJobInput::from_json(&bytes)
            .expect("frozen synthetic input validates")
    }

    /// Seed a valid PENDING row pair through the real app role and RLS. The
    /// fixture uses synthetic metadata only and is never an execution proof.
    pub async fn seed_simulation_pending(
        &self,
        actor: &UserCtx,
        strategy_config_id: Uuid,
        evaluation_start: &str,
        evaluation_end: &str,
    ) -> SimulationFixture {
        let run_id = Uuid::new_v4();
        let job_id = Uuid::new_v4();
        let input = Self::simulation_test_input(
            strategy_config_id,
            run_id,
            evaluation_start,
            evaluation_end,
        );
        let payload = serde_json::to_value(&input).expect("synthetic input serializes");
        let schedule = input.schedule();
        let session_count = schedule.warmup_sessions().len() + schedule.evaluation_sessions().len();
        let pins = input.pins();
        let namespace_key = format!("owner-beta:price-only-simulation:v1:fixture-{job_id}");

        let p = actor_pool(&self.app_url, &actor.user_id.to_string(), 2).await;
        let mut tx = p.begin().await.expect("simulation seed transaction begins");
        sqlx::query(
            "INSERT INTO public.jobs
                (id, owner_user_id, job_type, status, priority, idempotency_key,
                 payload_json, max_attempts, available_at)
             VALUES ($1,$2,'owner_beta_price_only_simulation','QUEUED',10,$3,$4,3,pg_catalog.now())",
        )
        .bind(job_id)
        .bind(actor.user_id)
        .bind(namespace_key)
        .bind(payload)
        .execute(&mut *tx)
        .await
        .expect("simulation queue row is seeded");
        sqlx::query(
            "INSERT INTO public.owner_beta_simulation_runs
                (id, owner_user_id, strategy_config_id, job_id,
                 strategy_id, strategy_version, strategy_config_json,
                 strategy_config_sha256, evaluation_start, evaluation_end,
                 session_count, candidate_content_sha256, artifact_manifest_sha256,
                 stage5_manifest_sha256, action_manifest_sha256,
                 approval_registry_sha256, contract_document_sha256, bars_sha256)
             VALUES ($1,$2,$3,$4,'trend_following','1.0.0',
                 '{\"benchmark_instrument\":\"069500.KRX\",\"fast_ma\":50,\"slow_ma\":200}'::jsonb,
                 'sha256:90e5b4c6972678e9186d4655d76496676e7cfb8dbe473a249f062963873c04a0',
                 $5,$6,$7,$8,$9,$10,$11,$12,$13,$14)",
        )
        .bind(run_id)
        .bind(actor.user_id)
        .bind(strategy_config_id)
        .bind(job_id)
        .bind(input.evaluation_start().as_naive_date())
        .bind(input.evaluation_end().as_naive_date())
        .bind(session_count as i32)
        .bind(pins.candidate_content_sha256().as_str())
        .bind(pins.artifact_manifest_sha256().as_str())
        .bind(pins.stage5_manifest_sha256().as_str())
        .bind(pins.action_manifest_sha256().as_str())
        .bind(pins.approval_registry_sha256().as_str())
        .bind(input.contract_document_sha256().as_str())
        .bind(input.bars_sha256().as_str())
        .execute(&mut *tx)
        .await
        .expect("simulation run row is seeded");
        tx.commit().await.expect("simulation seed commits");

        SimulationFixture {
            run_id,
            job_id,
            owner_user_id: actor.user_id,
            strategy_config_id,
        }
    }

    /// Build a database-consistent but intentionally malformed canonical
    /// pair so HTTP readers must reject the entire success without a partial
    /// profile. The queue/attempt/run success bindings commit atomically.
    pub async fn seed_simulation_corrupt_success_pair(&self, fixture: &SimulationFixture) {
        let malformed = b"{}".to_vec();
        let digest = format!("sha256:{}", sha256_hex(&malformed));
        self.seed_simulation_success_rows(
            fixture,
            malformed.clone(),
            digest.clone(),
            malformed,
            digest,
            serde_json::json!({
                "git_commit": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "binary_sha256": format!("sha256:{}", "a".repeat(64)),
                "toolchain": "rustc-test",
                "target": "x86_64-unknown-linux-gnu",
                "profile": "test"
            }),
        )
        .await;
    }

    /// Publish checked synthetic pair bytes through one worker transaction.
    /// This is database fixture construction only; it does not run the real
    /// worker or attest the declared test engine.
    pub async fn seed_simulation_success_pair(
        &self,
        fixture: &SimulationFixture,
    ) -> (String, String) {
        let (illustrative, zero_cost, engine) = checked_simulation_pair_bytes(fixture);
        let illustrative_hash = format!("sha256:{}", sha256_hex(&illustrative));
        let zero_cost_hash = format!("sha256:{}", sha256_hex(&zero_cost));
        self.seed_simulation_success_rows(
            fixture,
            illustrative,
            illustrative_hash.clone(),
            zero_cost,
            zero_cost_hash.clone(),
            engine,
        )
        .await;
        (illustrative_hash, zero_cost_hash)
    }

    async fn seed_simulation_success_rows(
        &self,
        fixture: &SimulationFixture,
        illustrative: Vec<u8>,
        illustrative_hash: String,
        zero_cost: Vec<u8>,
        zero_cost_hash: String,
        engine: serde_json::Value,
    ) {
        let worker = self.worker_pool().await;
        let mut tx = worker.begin().await.expect("worker transaction begins");
        let attempt_id = Uuid::new_v4();
        let worker_id = format!("api-test-{attempt_id}");
        sqlx::query(
            "UPDATE public.jobs
                SET status='RUNNING', attempt_count=1, locked_by=$2,
                    locked_at=pg_catalog.now(), started_at=pg_catalog.now(), updated_at=pg_catalog.now()
              WHERE id=$1 AND job_type='owner_beta_price_only_simulation'",
        )
        .bind(fixture.job_id)
        .bind(&worker_id)
        .execute(&mut *tx)
        .await
        .expect("job enters the test lease");
        sqlx::query(
            "INSERT INTO public.job_attempts
                (id, job_id, attempt_no, outcome, claimed_by, started_at)
             VALUES ($1,$2,1,'RUNNING',$3,pg_catalog.now())",
        )
        .bind(attempt_id)
        .bind(fixture.job_id)
        .bind(&worker_id)
        .execute(&mut *tx)
        .await
        .expect("running attempt is seeded");
        sqlx::query(
            "INSERT INTO public.owner_beta_simulation_result_pairs
                (run_id, owner_user_id, illustrative_canonical, illustrative_sha256,
                 zero_cost_canonical, zero_cost_sha256)
             VALUES ($1,$2,$3,$4,$5,$6)",
        )
        .bind(fixture.run_id)
        .bind(fixture.owner_user_id)
        .bind(illustrative)
        .bind(illustrative_hash)
        .bind(zero_cost)
        .bind(zero_cost_hash)
        .execute(&mut *tx)
        .await
        .expect("complete pair fixture is stored atomically");
        sqlx::query(
            "UPDATE public.owner_beta_simulation_runs
                SET status='SUCCEEDED', engine_json=$2, published_attempt_id=$3,
                    started_at=pg_catalog.now(), finished_at=pg_catalog.now(), updated_at=pg_catalog.now()
              WHERE id=$1",
        )
        .bind(fixture.run_id)
        .bind(engine)
        .bind(attempt_id)
        .execute(&mut *tx)
        .await
        .expect("test run is marked successful");
        sqlx::query(
            "UPDATE public.jobs
                SET status='SUCCEEDED', locked_by=NULL, locked_at=NULL,
                    finished_at=pg_catalog.now(), updated_at=pg_catalog.now()
              WHERE id=$1",
        )
        .bind(fixture.job_id)
        .execute(&mut *tx)
        .await
        .expect("test job is marked successful");
        sqlx::query(
            "UPDATE public.job_attempts
                SET outcome='SUCCEEDED', finished_at=pg_catalog.now()
              WHERE id=$1",
        )
        .bind(attempt_id)
        .execute(&mut *tx)
        .await
        .expect("test attempt is marked successful");
        tx.commit()
            .await
            .expect("synthetic success fixture commits");
    }

    /// Seed a tenant row through the migration-owner fixture while carrying an
    /// explicit actor GUC. This is reserved for immutable migration-origin
    /// fixtures (for example, recommendation lineage) that the app role must
    /// not be able to forge through the production trigger boundary.
    pub async fn seed_migration_owner(&self, actor: &UserCtx, sql: &str) {
        let mut tx = self
            .owner_pool
            .begin()
            .await
            .expect("owner seed transaction");
        sqlx::query("SELECT set_config('app.actor_user_id', $1, true)")
            .bind(actor.user_id.to_string())
            .execute(&mut *tx)
            .await
            .expect("owner seed actor context");
        sqlx::query(sqlx::AssertSqlSafe(sql.to_string()))
            .execute(&mut *tx)
            .await
            .unwrap_or_else(|e| panic!("seed_migration_owner failed: {e} ({sql:?})"));
        tx.commit().await.expect("owner seed transaction commits");
    }

    /// Create a user (shared rows) + a live session (tenant row, GUC'd).
    pub async fn seed_user(&self, role: Role, email: &str, iss: &str, sub: &str) -> UserCtx {
        let role_id = match role {
            Role::Owner => "owner",
            Role::Member => "member",
        };
        self.seed_shared(&format!(
            "INSERT INTO users (id, issuer, subject, email) VALUES \
             (gen_random_uuid(), '{iss}', '{sub}', '{email}') \
             ON CONFLICT (issuer, subject) DO UPDATE SET email = EXCLUDED.email \
             RETURNING id"
        ))
        .await;
        let row = sqlx::query("SELECT id FROM users WHERE issuer = $1 AND subject = $2")
            .bind(iss)
            .bind(sub)
            .fetch_one(&self.owner_pool)
            .await
            .expect("user row");
        let user_id: Uuid = row.get("id");
        self.seed_shared(&format!(
            "INSERT INTO user_roles (user_id, role_id, granted_by) VALUES \
             ('{user_id}', '{role_id}', (SELECT id FROM users WHERE email='owner@lagrange.test' OR email='{email}' LIMIT 1)) \
             ON CONFLICT DO NOTHING"
        ))
        .await;

        let cookie_value = format!("test-session-{email}");
        let csrf_token = format!("test-csrf-{email}");
        let session_hash = cookie::hash(&cookie_value);
        let csrf_hash = auth::csrf::hash_token(&csrf_token);
        self.seed_tenant(
            &UserCtx {
                user_id,
                role,
                cookie_value: String::new(),
                csrf_token: String::new(),
            },
            &format!(
                "INSERT INTO web_sessions (id, user_id, session_hash, csrf_hash, expires_at) VALUES \
                 (gen_random_uuid(), '{user_id}', '{session_hash}', '{csrf_hash}', now() + interval '1 hour')"
            ),
        )
        .await;
        UserCtx {
            user_id,
            role,
            cookie_value,
            csrf_token,
        }
    }

    /// Seed a user whose session carries authentication-method claims and an
    /// explicit authentication time.
    ///
    /// Step-up (Owner + fresh MFA) cannot be exercised through `seed_user`,
    /// whose sessions carry no `amr` — by design, since that is what an
    /// ordinary password login looks like. A test that needs to pass step-up,
    /// or to prove that STALE MFA still fails, has to control both facts.
    pub async fn seed_user_with_amr(
        &self,
        role: Role,
        email: &str,
        iss: &str,
        sub: &str,
        amr: &[&str],
        auth_time_secs: i64,
    ) -> UserCtx {
        let ctx = self.seed_user(role, email, iss, sub).await;
        let amr_literal = amr
            .iter()
            .map(|a| format!("'{a}'"))
            .collect::<Vec<_>>()
            .join(",");
        self.seed_tenant(
            &ctx,
            &format!(
                "UPDATE web_sessions SET amr = ARRAY[{amr_literal}]::text[], \
                        auth_time = to_timestamp({auth_time_secs}) \
                 WHERE user_id = '{}'",
                ctx.user_id
            ),
        )
        .await;
        ctx
    }

    // ------------------------------------------------------------------
    // HTTP plumbing: oneshot against the router with cookie/csrf/request id.
    // ------------------------------------------------------------------

    /// Send a request with full control. `user` adds the session cookie;
    /// `csrf` adds the X-CSRF-Token header; `rid` sets X-Request-Id; `idem`
    /// adds the Idempotency-Key header.
    #[allow(clippy::too_many_arguments)]
    pub async fn send(
        &self,
        method: &str,
        path: &str,
        user: Option<&UserCtx>,
        csrf: bool,
        rid: Option<&str>,
        idem: Option<&str>,
        body: Option<serde_json::Value>,
    ) -> Response {
        Self::send_to(self.app.clone(), method, path, user, csrf, rid, idem, body).await
    }

    /// Send through a captured router. This lets tests exercise concurrent
    /// requests from independent API states against the same durable store.
    #[allow(clippy::too_many_arguments)]
    pub async fn send_to(
        app: axum::Router,
        method: &str,
        path: &str,
        user: Option<&UserCtx>,
        csrf: bool,
        rid: Option<&str>,
        idem: Option<&str>,
        body: Option<serde_json::Value>,
    ) -> Response {
        Self::send_payload_to(
            app,
            method,
            path,
            user,
            csrf,
            rid,
            idem,
            body.map(|value| value.to_string()),
        )
        .await
    }

    /// Send exact JSON bytes for parser-boundary tests such as duplicate keys.
    #[allow(clippy::too_many_arguments)]
    pub async fn send_raw(
        &self,
        method: &str,
        path: &str,
        user: Option<&UserCtx>,
        csrf: bool,
        rid: Option<&str>,
        idem: Option<&str>,
        body: Option<&str>,
    ) -> Response {
        Self::send_payload_to(
            self.app.clone(),
            method,
            path,
            user,
            csrf,
            rid,
            idem,
            body.map(str::to_owned),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn send_payload_to(
        app: axum::Router,
        method: &str,
        path: &str,
        user: Option<&UserCtx>,
        csrf: bool,
        rid: Option<&str>,
        idem: Option<&str>,
        payload: Option<String>,
    ) -> Response {
        let mut builder = Request::builder()
            .method(Method::from_bytes(method.as_bytes()).expect("method"))
            .uri(path)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(u) = user {
            builder = builder.header(
                header::COOKIE,
                format!("{}={}", cookie::NAME, u.cookie_value),
            );
        }
        if csrf && let Some(u) = user {
            builder = builder.header("x-csrf-token", &u.csrf_token);
        }
        builder = builder.header("x-request-id", rid.unwrap_or("test-rid-1"));
        if let Some(k) = idem {
            builder = builder.header("idempotency-key", k);
        }
        let body = payload.map(Body::from).unwrap_or_else(Body::empty);
        let req = builder.body(body).expect("request builds");
        app.oneshot(req).await.expect("oneshot")
    }

    pub async fn get(&self, path: &str, user: Option<&UserCtx>) -> Response {
        self.send("GET", path, user, false, Some("test-rid-1"), None, None)
            .await
    }

    /// Convenience POST with an auto-generated Idempotency-Key (mutating
    /// routes require one; tests that must prove the 400 use `send` with
    /// `idem = None`).
    pub async fn post(
        &self,
        path: &str,
        user: Option<&UserCtx>,
        csrf: bool,
        body: serde_json::Value,
    ) -> Response {
        let key = format!("auto-{}", uuid::Uuid::new_v4());
        self.send(
            "POST",
            path,
            user,
            csrf,
            Some("test-rid-1"),
            Some(&key),
            Some(body),
        )
        .await
    }

    /// Collect the response body as UTF-8 text.
    pub async fn body_text(resp: Response) -> String {
        let bytes = resp
            .into_body()
            .collect()
            .await
            .expect("collect body")
            .to_bytes();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// Collect the response body as JSON.
    pub async fn body_json(resp: Response) -> serde_json::Value {
        let text = Self::body_text(resp).await;
        serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("response is not JSON: {e} (body: {text})"))
    }

    pub fn rid(resp: &Response) -> String {
        resp.headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string()
    }

    pub fn assert_rid_echo(&self, resp: &Response) {
        assert_eq!(
            resp.headers()
                .get("x-request-id")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default(),
            "test-rid-1",
            "X-Request-Id must be echoed"
        );
    }

    pub fn error_code(body: &serde_json::Value) -> String {
        let err = body
            .get("error")
            .unwrap_or_else(|| panic!("missing error envelope in {body}"));
        err.get("code")
            .and_then(|c| c.as_str())
            .unwrap_or_else(|| panic!("missing error.code in {body}"))
            .to_string()
    }

    /// The canonical request id for tests.
    pub const RID: &'static str = "test-rid-1";
}

#[derive(Debug, Clone, Copy)]
pub struct SimulationFixture {
    pub run_id: Uuid,
    pub job_id: Uuid,
    pub owner_user_id: Uuid,
    pub strategy_config_id: Uuid,
}

/// Collect the status code (response is not consumed).
pub fn status(resp: &Response) -> StatusCode {
    resp.status()
}
