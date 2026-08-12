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
use api_server::http::state::{ApiConfig, ApiState};
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
GRANT USAGE ON SCHEMA public TO migration_owner, app, worker, audit_writer, research_writer, admin;
GRANT CREATE ON SCHEMA public TO migration_owner;
SELECT pg_advisory_unlock(424242001);
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

fn conn_url(super_url: &str, role: &str, db: &str) -> String {
    let (_scheme, rest) = super_url
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
    match pw {
        Some(p) => format!("postgres://{role}:{p}@{hostport}/{db}"),
        None => format!("postgres://{role}@{hostport}/{db}"),
    }
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

        let owner_url = conn_url(&super_url, "migration_owner", &db_name);
        let app_url = conn_url(&super_url, "app", &db_name);
        let admin_url = conn_url(&super_url, "admin", &db_name);
        let audit_url = conn_url(&super_url, "audit_writer", &db_name);
        let worker_url = conn_url(&super_url, "worker", &db_name);

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
        let body = match body {
            Some(v) => Body::from(v.to_string()),
            None => Body::empty(),
        };
        let req = builder.body(body).expect("request builds");
        self.app.clone().oneshot(req).await.expect("oneshot")
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

/// Collect the status code (response is not consumed).
pub fn status(resp: &Response) -> StatusCode {
    resp.status()
}
