use std::error::Error;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use domain::{TradingDate, UtcTimestamp};
use market_data::contract::MARKET_KR;
use market_data::ingest::{IngestRequest, ingest_bundle};
use market_data::provider::{KrxProvider, RecordedBundle};
use market_data::publication::PublicationBundle;
use market_data::storage::RawStore;
use sqlx::migrate::{MigrationType, Migrator};
use sqlx::postgres::PgPoolOptions;
use sqlx::{Executor, PgPool};

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");
static COUNTER: AtomicU64 = AtomicU64::new(0);

const ROLE_BOOTSTRAP_SQL: &str = r#"
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
"#;

fn ddl_for(db: &str, statement: &str) -> sqlx::AssertSqlSafe<String> {
    sqlx::AssertSqlSafe(statement.replace("{db}", db))
}

fn conn_url(base: &str, role: &str, db: &str) -> String {
    let (scheme, rest) = base.split_once("://").expect("DATABASE_URL scheme");
    let (auth, host_and_db) = rest.rsplit_once('@').expect("DATABASE_URL credentials");
    let password = auth.split_once(':').map(|(_, password)| password);
    let host = host_and_db
        .rsplit_once('/')
        .expect("DATABASE_URL database")
        .0;
    match password {
        Some(password) => format!("{scheme}://{role}:{password}@{host}/{db}"),
        None => format!("{scheme}://{role}@{host}/{db}"),
    }
}

async fn pool(url: &str, max_connections: u32) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(Duration::from_secs(10))
        .connect(url)
        .await
}

fn up_migration_count() -> usize {
    MIGRATOR
        .migrations
        .iter()
        .filter(|migration| migration.migration_type != MigrationType::ReversibleDown)
        .count()
}

pub struct ScratchDb {
    pub supervisor: PgPool,
    pub writer: PgPool,
    name: String,
    supervisor_url: String,
}

impl ScratchDb {
    pub async fn create() -> Option<Self> {
        let supervisor_url = std::env::var("DATABASE_URL").ok()?;
        Some(
            Self::build(supervisor_url)
                .await
                .unwrap_or_else(|error| panic!("scratch database setup failed: {error}")),
        )
    }

    async fn build(supervisor_url: String) -> Result<Self, Box<dyn Error>> {
        let name = format!(
            "collector_pub_{}_{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let admin = pool(&supervisor_url, 2).await?;
        admin
            .execute(ddl_for(&name, "DROP DATABASE IF EXISTS {db} WITH (FORCE)"))
            .await?;
        admin
            .execute(ddl_for(&name, "CREATE DATABASE {db}"))
            .await?;
        drop(admin);

        let supervisor = pool(&conn_url(&supervisor_url, "postgres", &name), 4).await?;
        sqlx::raw_sql(ROLE_BOOTSTRAP_SQL)
            .execute(&supervisor)
            .await?;
        supervisor
            .execute(ddl_for(
                &name,
                "GRANT CONNECT ON DATABASE {db} TO migration_owner, app, worker, audit_writer, research_writer, admin",
            ))
            .await?;

        let owner = pool(&conn_url(&supervisor_url, "migration_owner", &name), 2).await?;
        MIGRATOR.run(&owner).await?;
        let applied: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
            .fetch_one(&owner)
            .await?;
        assert_eq!(applied as usize, up_migration_count());
        owner.close().await;

        let writer = pool(&conn_url(&supervisor_url, "research_writer", &name), 4).await?;
        let current_user: String = sqlx::query_scalar("SELECT current_user")
            .fetch_one(&writer)
            .await?;
        assert_eq!(current_user, "research_writer");
        Ok(Self {
            supervisor,
            writer,
            name,
            supervisor_url,
        })
    }

    pub async fn drop_db(self) {
        self.writer.close().await;
        self.supervisor.close().await;
        if let Ok(admin) = pool(&self.supervisor_url, 1).await {
            let _ = admin
                .execute(ddl_for(
                    &self.name,
                    "DROP DATABASE IF EXISTS {db} WITH (FORCE)",
                ))
                .await;
        }
    }
}

pub struct FixtureBundle {
    pub bundle: PublicationBundle,
    _root: tempfile::TempDir,
}

pub fn synthetic_bundle(retrieved_at: &str) -> FixtureBundle {
    let root = tempfile::tempdir().expect("raw fixture root");
    let store = RawStore::new(root.path());
    let provider = KrxProvider::synthetic(
        RecordedBundle::open(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/kr-etf/contract"
        ))
        .expect("recorded bundle"),
    );
    let outcome = ingest_bundle(
        &store,
        &provider,
        &IngestRequest::new(
            MARKET_KR.to_owned(),
            TradingDate::parse("2020-01-31").expect("date"),
            UtcTimestamp::parse_rfc3339(retrieved_at).expect("retrieved_at"),
        ),
        None,
    )
    .expect("persist synthetic fixture");
    let manifest = store
        .read_manifest("krx", "kr")
        .expect("read manifest")
        .into_iter()
        .find(|entry| entry.batch_id == outcome.batch_id)
        .expect("persisted manifest");
    let bundle = PublicationBundle::from_raw(&store, &manifest).expect("verified publication");
    FixtureBundle {
        bundle,
        _root: root,
    }
}
