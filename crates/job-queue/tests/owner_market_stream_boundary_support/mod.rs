//! Disposable, real-role support for the WS-3A storage boundary.
//!
//! This harness deliberately does not read DATABASE_URL.  The supervisor
//! URL is an explicit WS-3A test input, and every test database name is
//! generated, checked for absence, and dropped by exact name after the test.
//! The role and per-database bootstrap files are the repository copies used by
//! the migration-contract suite; no inline replacement schema is permitted.

use std::error::Error;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::owner_equity_v2::market_stream::{
    StreamIdentity, StreamLeaseIdentity, StreamSessionProof,
};
use base64::Engine;
use chrono::{DateTime, Duration as ChronoDuration, FixedOffset, NaiveDate, Utc};
use kis_client::MarketStreamClient;
use kis_client::clock::{Clock, TestClock};
use kis_client::market_stream::{MarketStreamConfig, MarketStreamEvent, MarketStreamSession};
use kis_client::market_stream_approval::ApprovalClient;
use kis_client::market_stream_state::MarketStreamDomain;
use kis_client::market_stream_wire::MarketStreamSessionProof;
use kis_client::secret::Secret;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgListener, PgPoolOptions};
use sqlx::{Executor, PgPool};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use uuid::Uuid;

pub(super) mod c2_fixture;
pub(super) use c2_fixture::{C2Endpoint, SupervisorTarget, relay_endpoint_from_environment};

pub const SUPERVISOR_ENV: &str = "LAGRANGE_WS3A_SUPERVISOR_URL";
pub const STREAM_IDENTITY_HASH: &str =
    "0e6a9b3aef6b310685b9bd5594a39452c2902d11af623197699cf6dc46931e79";
pub const DOCUMENT_HASH: &str = "1111111111111111111111111111111111111111111111111111111111111111";
pub const TRANSITION_HASH: &str =
    "sha256:2222222222222222222222222222222222222222222222222222222222222222";
pub const NETWORK_HASH: &str = "3333333333333333333333333333333333333333333333333333333333333333";
pub const WINDOW_HASH: &str =
    "sha256:4444444444444444444444444444444444444444444444444444444444444444";
pub const CALENDAR_HASH: &str = "5555555555555555555555555555555555555555555555555555555555555555";
pub const CODE_COMMIT: &str = "7757245553ab8cbed5d6f706e46a87849e0cedc6";
const EXPECTED_SYSTEM_IDENTIFIER: &str = "7687959257259950090";
const EXPECTED_DATA_DIRECTORY: &str = "/tmp/lagrange-kis-stream-20260921/pg-local/data";
const EXPECTED_PORT: &str = "55449";

pub const MARKET_SYMBOLS: [&str; 30] = [
    "005930", "000660", "373220", "207940", "005380", "000270", "105560", "055550", "068270",
    "035420", "035720", "005490", "051910", "006400", "012330", "028260", "012450", "329180",
    "034020", "015760", "017670", "030200", "066570", "009150", "096770", "036570", "090430",
    "011200", "003490", "000810",
];

const ROLE_BOOTSTRAP_SQL: &str =
    include_str!("../../../../tests/integration/migration-contract/role-bootstrap.sql");
const BOOTSTRAP_SQL: &str =
    include_str!("../../../../tests/integration/migration-contract/bootstrap.sql");
static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");
static DB_COUNTER: AtomicU64 = AtomicU64::new(0);

fn generated_database_name() -> String {
    let count = DB_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("lagrange_ws3a_{}_{}", std::process::id(), count)
}

fn safe_database_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn database_ddl(name: &str, statement: &str) -> sqlx::AssertSqlSafe<String> {
    assert!(safe_database_name(name));
    sqlx::AssertSqlSafe(statement.replace("{db}", name))
}

async fn drop_generated_database(
    supervisor_target: &SupervisorTarget,
    name: &str,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    assert!(safe_database_name(name));
    let supervisor = c2_fixture::connect_supervisor(supervisor_target).await?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_database WHERE datname = $1)",
    )
    .bind(name)
    .fetch_one(&supervisor)
    .await?;
    if exists {
        supervisor
            .execute(database_ddl(name, "DROP DATABASE {db} WITH (FORCE)"))
            .await?;
    }
    let remains: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_database WHERE datname = $1)",
    )
    .bind(name)
    .fetch_one(&supervisor)
    .await?;
    supervisor.close().await;
    if remains {
        return Err(format!("generated WS-3A database {name} remains after cleanup").into());
    }
    Ok(())
}

#[derive(Debug)]
struct FixtureCleanupError {
    primary: Box<dyn Error + Send + Sync>,
    cleanup: Box<dyn Error + Send + Sync>,
}

impl std::fmt::Display for FixtureCleanupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "fixture operation failed: {}; cleanup failed: {}",
            self.primary, self.cleanup
        )
    }
}

impl Error for FixtureCleanupError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.primary.as_ref())
    }
}

pub(super) fn combine_primary_cleanup(
    primary: Box<dyn Error + Send + Sync>,
    cleanup: Box<dyn Error + Send + Sync>,
) -> Box<dyn Error + Send + Sync> {
    Box::new(FixtureCleanupError { primary, cleanup })
}

pub(super) async fn connect_supervisor(
    target: &SupervisorTarget,
) -> Result<PgPool, Box<dyn Error + Send + Sync>> {
    Ok(c2_fixture::connect_supervisor(target).await?)
}

pub(super) async fn connect_supervisor_from_environment()
-> Result<PgPool, Box<dyn Error + Send + Sync>> {
    let target = SupervisorTarget::from_environment()?;
    connect_supervisor(&target).await
}

async fn connect(options: PgConnectOptions) -> Result<PgPool, Box<dyn Error + Send + Sync>> {
    Ok(PgPoolOptions::new()
        .max_connections(16)
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(options)
        .await?)
}

#[derive(Clone)]
pub struct DisposableDatabase {
    name: String,
    supervisor_target: SupervisorTarget,
    pub migration_owner: PgPool,
    pub app: PgPool,
    pub worker: PgPool,
    pub research_writer: PgPool,
    pub admin: PgPool,
}

impl DisposableDatabase {
    pub async fn create() -> Result<Self, Box<dyn Error + Send + Sync>> {
        let supervisor_target = SupervisorTarget::from_environment()?;
        let supervisor_options = supervisor_target.options();
        let supervisor = connect_supervisor(&supervisor_target).await?;
        if !supervisor_target.is_c2() {
            let system_identifier: String = sqlx::query_scalar(
                "SELECT system_identifier::text FROM pg_catalog.pg_control_system()",
            )
            .fetch_one(&supervisor)
            .await?;
            let data_directory: String =
                sqlx::query_scalar("SELECT pg_catalog.current_setting('data_directory')")
                    .fetch_one(&supervisor)
                    .await?;
            let listen_addresses: String =
                sqlx::query_scalar("SELECT pg_catalog.current_setting('listen_addresses')")
                    .fetch_one(&supervisor)
                    .await?;
            let port: String = sqlx::query_scalar("SELECT pg_catalog.current_setting('port')")
                .fetch_one(&supervisor)
                .await?;
            let server_address: Option<String> =
                sqlx::query_scalar("SELECT pg_catalog.inet_server_addr()::text")
                    .fetch_one(&supervisor)
                    .await?;
            if system_identifier != EXPECTED_SYSTEM_IDENTIFIER
                || data_directory != EXPECTED_DATA_DIRECTORY
                || !listen_addresses.is_empty()
                || port != EXPECTED_PORT
                || server_address.is_some()
            {
                return Err(format!(
                    "WS-3A supervisor identity mismatch: identifier={system_identifier}, data_directory={data_directory}, listen_addresses={listen_addresses:?}, port={port}, unix_socket={}",
                    server_address.is_none()
                )
                .into());
            }
        }
        let name = generated_database_name();
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_database WHERE datname = $1)",
        )
        .bind(&name)
        .fetch_one(&supervisor)
        .await?;
        assert!(
            !exists,
            "generated WS-3A database name already exists: {name}"
        );

        let mut role_tx = supervisor.begin().await?;
        sqlx::query("SELECT pg_catalog.pg_advisory_xact_lock(hashtext($1))")
            .bind("lagrange-ws3a-role-bootstrap")
            .execute(&mut *role_tx)
            .await?;
        sqlx::raw_sql(ROLE_BOOTSTRAP_SQL)
            .execute(&mut *role_tx)
            .await?;
        role_tx.commit().await?;
        supervisor
            .execute(database_ddl(&name, "CREATE DATABASE {db}"))
            .await?;
        let build_result: Result<Self, Box<dyn Error + Send + Sync>> = async {
            supervisor
                .execute(database_ddl(
                    &name,
                    "GRANT CONNECT ON DATABASE {db} TO migration_owner, app, worker, \
                     audit_writer, research_writer, admin",
                ))
                .await?;

            let database_supervisor = connect(supervisor_options.clone().database(&name)).await?;
            sqlx::raw_sql(BOOTSTRAP_SQL)
                .execute(&database_supervisor)
                .await?;
            database_supervisor.close().await;

            let migration_owner = connect(
                supervisor_options
                    .clone()
                    .database(&name)
                    .username("migration_owner")
                    .password("lagrange"),
            )
            .await?;
            MIGRATOR.run(&migration_owner).await?;
            let applied: i64 = sqlx::query_scalar("SELECT count(*) FROM public._sqlx_migrations")
                .fetch_one(&migration_owner)
                .await?;
            let expected_applied = MIGRATOR
                .migrations
                .iter()
                .filter(|migration| migration.migration_type.is_up_migration())
                .count();
            if applied as usize != expected_applied {
                return Err(format!(
                    "WS-3A boundary applied {applied} of {expected_applied} repository up migrations"
                )
                .into());
            }

            let app = connect(
                supervisor_options
                    .clone()
                    .database(&name)
                    .username("app")
                    .password("lagrange"),
            )
            .await?;
            let worker = connect(
                supervisor_options
                    .clone()
                    .database(&name)
                    .username("worker")
                    .password("lagrange"),
            )
            .await?;
            let research_writer = connect(
                supervisor_options
                    .clone()
                    .database(&name)
                    .username("research_writer")
                    .password("lagrange"),
            )
            .await?;
            let admin = connect(
                supervisor_options
                    .clone()
                    .database(&name)
                    .username("admin")
                    .password("lagrange"),
            )
            .await?;
            Ok(Self {
                name: name.clone(),
                supervisor_target: supervisor_target.clone(),
                migration_owner,
                app,
                worker,
                research_writer,
                admin,
            })
        }
        .await;
        drop(supervisor);
        match build_result {
            Ok(database) => Ok(database),
            Err(primary) => match drop_generated_database(&supervisor_target, &name).await {
                Ok(()) => Err(primary),
                Err(cleanup) => Err(combine_primary_cleanup(primary, cleanup)),
            },
        }
    }

    pub async fn cleanup(self) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.migration_owner.close().await;
        self.app.close().await;
        self.worker.close().await;
        self.research_writer.close().await;
        self.admin.close().await;
        drop_generated_database(&self.supervisor_target, &self.name).await
    }
}

pub struct Fixture {
    pub owner_user_id: Uuid,
    pub other_owner_user_id: Uuid,
    pub owner_session_hash: String,
    pub secondary_owner_session_hash: String,
    pub other_session_hash: String,
    pub credential_slot_id: Uuid,
    pub grant_id: Uuid,
    pub grant_revision: Uuid,
    pub session_date: NaiveDate,
    pub session: StreamSessionProof,
    pub identities: Vec<StreamIdentity>,
}

impl Fixture {
    pub fn lease_identities(&self) -> Vec<StreamLeaseIdentity> {
        self.identities
            .iter()
            .map(|identity| {
                StreamLeaseIdentity::new(
                    identity.membership_id,
                    identity.instrument_id.clone(),
                    identity.generation,
                )
            })
            .collect::<Result<Vec<_>, _>>()
            .expect("fixture identities are canonical")
    }
}

pub async fn seed_fixture(
    database: &DisposableDatabase,
) -> Result<Fixture, Box<dyn Error + Send + Sync>> {
    let owner_user_id = Uuid::new_v4();
    let other_owner_user_id = Uuid::new_v4();
    let credential_slot_id = Uuid::new_v4();
    let other_credential_slot_id = Uuid::new_v4();
    let owner_session_hash =
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned();
    let secondary_owner_session_hash =
        "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".to_owned();
    let other_session_hash =
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned();
    let other_session_id = Uuid::new_v4();
    let owner_session_id = Uuid::new_v4();
    let entitlement_id = Uuid::new_v4();
    let other_entitlement_id = Uuid::new_v4();
    let session_date: NaiveDate =
        sqlx::query_scalar("SELECT (pg_catalog.clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date")
            .fetch_one(&database.migration_owner)
            .await?;
    let mut tx = database.migration_owner.begin().await?;
    set_actor(&mut tx, owner_user_id).await?;
    sqlx::query(
        "INSERT INTO public.roles (id, description)
         VALUES ('owner', 'synthetic owner'), ('member', 'synthetic member')
         ON CONFLICT (id) DO NOTHING",
    )
    .execute(&mut *tx)
    .await?;
    for (user_id, suffix) in [(owner_user_id, "owner-a"), (other_owner_user_id, "owner-b")] {
        sqlx::query(
            "INSERT INTO public.users (id, issuer, subject, email, display_name)
             VALUES ($1, 'ws3a-synthetic', $2, $3, $4)",
        )
        .bind(user_id)
        .bind(suffix)
        .bind(format!("{suffix}@invalid.test"))
        .bind(suffix)
        .execute(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO public.user_roles (user_id, role_id) VALUES ($1, 'owner')")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
    }
    set_actor(&mut tx, owner_user_id).await?;
    for (entitlement_id, managed_by) in [
        (entitlement_id, owner_user_id),
        (other_entitlement_id, other_owner_user_id),
    ] {
        sqlx::query(
            "INSERT INTO public.data_entitlements
                (id, contract_document_sha256, contract_reference, status,
                 covered_datasets, covered_uses, effective_from, effective_until,
                 managed_by)
             VALUES ($1, $2, $3, 'ACTIVE', '{}'::jsonb, '{}'::jsonb,
                     $4 - 1, $4 + 1, $5)",
        )
        .bind(entitlement_id)
        .bind(DOCUMENT_HASH)
        .bind("synthetic-ws3a-entitlement")
        .bind(session_date)
        .bind(managed_by)
        .execute(&mut *tx)
        .await?;
    }
    let now = Utc::now();
    for (user_id, session_id, session_hash, csrf_hash) in [
        (
            owner_user_id,
            owner_session_id,
            &owner_session_hash,
            "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        ),
        (
            owner_user_id,
            Uuid::new_v4(),
            &secondary_owner_session_hash,
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        ),
        (
            other_owner_user_id,
            other_session_id,
            &other_session_hash,
            "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        ),
    ] {
        set_actor(&mut tx, user_id).await?;
        sqlx::query(
            "INSERT INTO public.web_sessions
                (id, user_id, session_hash, csrf_hash, expires_at, created_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(session_id)
        .bind(user_id)
        .bind(session_hash)
        .bind(csrf_hash)
        .bind(now + ChronoDuration::hours(1))
        .bind(now - ChronoDuration::minutes(1))
        .execute(&mut *tx)
        .await?;
    }

    set_actor(&mut tx, owner_user_id).await?;
    let grant_id = Uuid::new_v4();
    let other_grant_id = Uuid::new_v4();
    let grant_revision: Uuid = sqlx::query_scalar(
        "INSERT INTO public.owner_market_stream_grants
            (id, credential_slot_id, credential_generation, owner_user_id,
             entitlement_id, entitlement_reference, entitlement_document_sha256,
             tr_id, wire_version, network_contract_sha256, identity_list_sha256,
             effective_from, effective_until, activation_commit, state)
         VALUES ($1, $2, '7', $3, $4, 'synthetic-ws3a-entitlement', $5,
                 'H0STCNT0', 'kis-h0stcnt0-20260914-v1', $6, $7,
                 $8 - 1, $8 + 1, $9, 'ACTIVE')
         RETURNING grant_revision",
    )
    .bind(grant_id)
    .bind(credential_slot_id)
    .bind(owner_user_id)
    .bind(entitlement_id)
    .bind(DOCUMENT_HASH)
    .bind(NETWORK_HASH)
    .bind(STREAM_IDENTITY_HASH)
    .bind(session_date)
    .bind(CODE_COMMIT)
    .fetch_one(&mut *tx)
    .await?;
    let _other_revision: Uuid = sqlx::query_scalar(
        "INSERT INTO public.owner_market_stream_grants
            (id, credential_slot_id, credential_generation, owner_user_id,
             entitlement_id, entitlement_reference, entitlement_document_sha256,
             tr_id, wire_version, network_contract_sha256, identity_list_sha256,
             effective_from, effective_until, activation_commit, state)
         VALUES ($1, $2, '7', $3, $4, 'synthetic-ws3a-entitlement', $5,
                 'H0STCNT0', 'kis-h0stcnt0-20260914-v1', $6, $7,
                 $8 - 1, $8 + 1, $9, 'ACTIVE')
         RETURNING grant_revision",
    )
    .bind(other_grant_id)
    .bind(other_credential_slot_id)
    .bind(other_owner_user_id)
    .bind(other_entitlement_id)
    .bind(DOCUMENT_HASH)
    .bind(NETWORK_HASH)
    .bind(STREAM_IDENTITY_HASH)
    .bind(session_date)
    .bind(CODE_COMMIT)
    .fetch_one(&mut *tx)
    .await?;

    let mut identities = Vec::with_capacity(MARKET_SYMBOLS.len());
    for symbol in MARKET_SYMBOLS {
        let membership_id = Uuid::new_v4();
        let generation_id = Uuid::new_v4();
        let instrument_id = format!("{symbol}.KRX");
        set_actor(&mut tx, owner_user_id).await?;
        sqlx::query(
            "INSERT INTO public.owner_equity_memberships
                (id, owner_user_id, instrument_id, state,
                 transition_actor_user_id, transition_code_commit,
                 transition_entitlement_sha256)
             VALUES ($1, $2, $3, 'REQUESTED', $2, $4, $5)",
        )
        .bind(membership_id)
        .bind(owner_user_id)
        .bind(&instrument_id)
        .bind(CODE_COMMIT)
        .bind(TRANSITION_HASH)
        .execute(&mut *tx)
        .await?;
        transition_fixture_membership(&mut tx, owner_user_id, membership_id, "VALIDATING").await?;
        transition_fixture_membership(&mut tx, owner_user_id, membership_id, "BACKFILLING").await?;
        sqlx::query(
            "INSERT INTO public.owner_equity_instrument_generations
                (id, membership_id, owner_user_id, instrument_id, generation,
                 target_observed_sessions, minimum_observed_sessions,
                 observed_sessions, first_session, last_session)
             VALUES ($1, $2, $3, $4, 1, 261, 121, 121, $5 - 120, $5)",
        )
        .bind(generation_id)
        .bind(membership_id)
        .bind(owner_user_id)
        .bind(&instrument_id)
        .bind(session_date)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO public.owner_equity_generation_admissions
                (generation_id, owner_user_id, membership_id, instrument_id,
                 generation, raw_manifest_sha256, artifact_manifest_sha256,
                 entitlement_sha256, capture_code_commit, materializer_code_commit)
             VALUES ($1, $2, $3, $4, 1, $5, $6, $7, $8, $8)",
        )
        .bind(generation_id)
        .bind(owner_user_id)
        .bind(membership_id)
        .bind(&instrument_id)
        .bind(TRANSITION_HASH)
        .bind(TRANSITION_HASH)
        .bind(TRANSITION_HASH)
        .bind(CODE_COMMIT)
        .execute(&mut *tx)
        .await?;
        transition_fixture_membership(&mut tx, owner_user_id, membership_id, "MATERIALIZING")
            .await?;
        transition_fixture_membership(&mut tx, owner_user_id, membership_id, "READY").await?;
        identities.push(StreamIdentity::new(
            owner_user_id,
            membership_id,
            generation_id,
            instrument_id,
            1,
        )?);
    }
    tx.commit().await?;

    let session = StreamSessionProof::new(
        session_date,
        Uuid::new_v4(),
        format!("sha256:{}", "6".repeat(64)),
        Uuid::new_v4(),
        CALENDAR_HASH.to_owned(),
        WINDOW_HASH.to_owned(),
    )?;
    Ok(Fixture {
        owner_user_id,
        other_owner_user_id,
        owner_session_hash,
        secondary_owner_session_hash,
        other_session_hash,
        credential_slot_id,
        grant_id,
        grant_revision,
        session_date,
        session,
        identities,
    })
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

async fn transition_fixture_membership(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    owner_user_id: Uuid,
    membership_id: Uuid,
    state: &str,
) -> Result<(), sqlx::Error> {
    set_actor(tx, owner_user_id).await?;
    sqlx::query(
        "UPDATE public.owner_equity_memberships
            SET state = $3, transition_actor_user_id = $2,
                transition_code_commit = $4,
                transition_entitlement_sha256 = $5,
                updated_at = pg_catalog.clock_timestamp()
          WHERE id = $1 AND owner_user_id = $2",
    )
    .bind(membership_id)
    .bind(owner_user_id)
    .bind(state)
    .bind(CODE_COMMIT)
    .bind(TRANSITION_HASH)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn set_stream_lease_expired(
    database: &DisposableDatabase,
    lease_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE public.owner_market_stream_leases
            SET lease_expires_at = created_at
          WHERE id = $1",
    )
    .bind(lease_id)
    .execute(&database.migration_owner)
    .await?;
    Ok(())
}

pub async fn set_producer_expired(
    database: &DisposableDatabase,
    slot_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "WITH expired AS (SELECT pg_catalog.clock_timestamp() AS observed_at)
         UPDATE public.owner_market_stream_producers
            SET heartbeat_at = expired.observed_at - INTERVAL '2 seconds',
                lease_expires_at = expired.observed_at - INTERVAL '1 second'
           FROM expired
          WHERE credential_slot_id = $1",
    )
    .bind(slot_id)
    .execute(&database.migration_owner)
    .await?;
    Ok(())
}

pub async fn revoke_session(
    database: &DisposableDatabase,
    owner_user_id: Uuid,
    session_hash: &str,
) -> Result<(), sqlx::Error> {
    let mut tx = database.migration_owner.begin().await?;
    set_actor(&mut tx, owner_user_id).await?;
    sqlx::query(
        "UPDATE public.web_sessions
            SET revoked_at = pg_catalog.clock_timestamp()
          WHERE session_hash = $1",
    )
    .bind(session_hash)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn restore_session(
    database: &DisposableDatabase,
    owner_user_id: Uuid,
    session_hash: &str,
) -> Result<(), sqlx::Error> {
    let mut tx = database.migration_owner.begin().await?;
    set_actor(&mut tx, owner_user_id).await?;
    sqlx::query("UPDATE public.web_sessions SET revoked_at = NULL WHERE session_hash = $1")
        .bind(session_hash)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn revoke_entitlement(
    database: &DisposableDatabase,
    grant_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE public.data_entitlements
            SET status = 'REVOKED'
          WHERE id = (
              SELECT entitlement_id
                FROM public.owner_market_stream_grants
               WHERE id = $1
          )",
    )
    .bind(grant_id)
    .execute(&database.migration_owner)
    .await?;
    Ok(())
}

pub async fn restore_entitlement(
    database: &DisposableDatabase,
    grant_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE public.data_entitlements
            SET status = 'ACTIVE'
          WHERE id = (
              SELECT entitlement_id
                FROM public.owner_market_stream_grants
               WHERE id = $1
          )",
    )
    .bind(grant_id)
    .execute(&database.migration_owner)
    .await?;
    Ok(())
}

pub async fn supersede_generation(
    database: &DisposableDatabase,
    identity: &StreamIdentity,
) -> Result<Uuid, Box<dyn Error + Send + Sync>> {
    let generation_id = Uuid::new_v4();
    let next_generation = i64::try_from(identity.generation + 1)?;
    let mut tx = database.migration_owner.begin().await?;
    set_actor(&mut tx, identity.owner_user_id).await?;
    sqlx::query(
        "INSERT INTO public.owner_equity_instrument_generations
            (id, membership_id, owner_user_id, instrument_id, generation,
             target_observed_sessions, minimum_observed_sessions,
             observed_sessions, first_session, last_session)
         VALUES ($1, $2, $3, $4, $5, 261, 121, 121,
                 $6 - 120, $6)",
    )
    .bind(generation_id)
    .bind(identity.membership_id)
    .bind(identity.owner_user_id)
    .bind(&identity.instrument_id)
    .bind(next_generation)
    .bind(identity_generation_date(database).await?)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO public.owner_equity_generation_admissions
            (generation_id, owner_user_id, membership_id, instrument_id,
             generation, raw_manifest_sha256, artifact_manifest_sha256,
             entitlement_sha256, capture_code_commit, materializer_code_commit)
         VALUES ($1, $2, $3, $4, $5, $6, $6, $6, $7, $7)",
    )
    .bind(generation_id)
    .bind(identity.owner_user_id)
    .bind(identity.membership_id)
    .bind(&identity.instrument_id)
    .bind(next_generation)
    .bind(TRANSITION_HASH)
    .bind(CODE_COMMIT)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(generation_id)
}

async fn identity_generation_date(database: &DisposableDatabase) -> Result<NaiveDate, sqlx::Error> {
    sqlx::query_scalar("SELECT (pg_catalog.clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date")
        .fetch_one(&database.migration_owner)
        .await
}

pub async fn force_cache_state_version_max(
    database: &DisposableDatabase,
    membership_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE public.owner_market_stream_cache
            SET state_version = 9223372036854775807
          WHERE membership_id = $1",
    )
    .bind(membership_id)
    .execute(&database.migration_owner)
    .await?;
    Ok(())
}

pub async fn now(database: &DisposableDatabase) -> Result<DateTime<Utc>, sqlx::Error> {
    sqlx::query_scalar("SELECT pg_catalog.clock_timestamp()")
        .fetch_one(&database.worker)
        .await
}

const DEFAULT_LOOPBACK_CLOSE_LIFETIME: Duration = Duration::from_secs(2);
const MAX_LOOPBACK_CLOSE_LIFETIME: Duration = Duration::from_secs(30);
const LOOPBACK_CHILD_JOIN_BOUND: Duration = Duration::from_secs(32);

type LoopbackTaskResult = Result<(), Box<dyn Error + Send + Sync>>;

#[derive(Debug)]
struct LoopbackChildFailure {
    child: &'static str,
    source: Box<dyn Error + Send + Sync>,
}

impl std::fmt::Display for LoopbackChildFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "synthetic {} child failed: {}",
            self.child, self.source
        )
    }
}

impl Error for LoopbackChildFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.source.as_ref())
    }
}

#[derive(Debug)]
struct LoopbackCleanupFailure {
    primary: Option<Box<dyn Error + Send + Sync>>,
    cleanup: Vec<Box<dyn Error + Send + Sync>>,
}

impl std::fmt::Display for LoopbackCleanupFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(primary) = &self.primary {
            write!(formatter, "loopback setup failed: {primary}")?;
        }
        for (index, error) in self.cleanup.iter().enumerate() {
            if index == 0 && self.primary.is_none() {
                write!(formatter, "loopback child cleanup failed: {error}")?;
            } else {
                write!(formatter, "; child cleanup failure {}: {error}", index + 1)?;
            }
        }
        Ok(())
    }
}

impl Error for LoopbackCleanupFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.primary
            .as_ref()
            .map(|error| error.as_ref() as &(dyn Error + 'static))
            .or_else(|| {
                self.cleanup
                    .first()
                    .map(|error| error.as_ref() as &(dyn Error + 'static))
            })
    }
}

fn finish_loopback_errors(
    primary: Option<Box<dyn Error + Send + Sync>>,
    cleanup: Vec<Box<dyn Error + Send + Sync>>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    if cleanup.is_empty() {
        return primary.map_or(Ok(()), Err);
    }
    Err(Box::new(LoopbackCleanupFailure { primary, cleanup }))
}

async fn join_loopback_child(
    child: &'static str,
    task: JoinHandle<LoopbackTaskResult>,
    timeout_after: Duration,
    abort_first: bool,
) -> Vec<Box<dyn Error + Send + Sync>> {
    let mut failures = Vec::new();
    let mut owned_task = AbortOnDropTask { task: Some(task) };
    let result = if abort_first {
        let result = {
            let task = owned_task.task.as_mut().expect("owned loopback task");
            task.abort();
            task.await
        };
        owned_task.task.take();
        result
    } else {
        match tokio::time::timeout(
            timeout_after,
            owned_task.task.as_mut().expect("owned loopback task"),
        )
        .await
        {
            Ok(result) => {
                owned_task.task.take();
                result
            }
            Err(_) => {
                failures.push(Box::new(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!("synthetic {child} child exceeded its bounded join wait"),
                )) as Box<dyn Error + Send + Sync>);
                let result = {
                    let task = owned_task.task.as_mut().expect("owned loopback task");
                    task.abort();
                    task.await
                };
                owned_task.task.take();
                result
            }
        }
    };
    match result {
        Ok(Ok(())) => {}
        Ok(Err(source)) => failures.push(Box::new(LoopbackChildFailure { child, source })),
        Err(join_error) if abort_first && join_error.is_cancelled() => {}
        Err(join_error) if failures.len() > 0 && join_error.is_cancelled() => {}
        Err(join_error) => failures.push(Box::new(LoopbackChildFailure {
            child,
            source: Box::new(join_error),
        })),
    }
    failures
}

struct AbortOnDropTask {
    task: Option<JoinHandle<LoopbackTaskResult>>,
}

impl Drop for AbortOnDropTask {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

async fn join_loopback_children(
    approval_task: Option<JoinHandle<LoopbackTaskResult>>,
    ws_task: Option<JoinHandle<LoopbackTaskResult>>,
    timeout_after: Duration,
    abort_first: bool,
) -> Vec<Box<dyn Error + Send + Sync>> {
    let mut failures = Vec::new();
    let mut owner = LoopbackChildOwner {
        approval_task,
        ws_task,
    };
    if let Some(task) = owner.approval_task.take() {
        failures.extend(join_loopback_child("approval", task, timeout_after, abort_first).await);
    }
    if let Some(task) = owner.ws_task.take() {
        failures.extend(join_loopback_child("websocket", task, timeout_after, abort_first).await);
    }
    failures
}

struct LoopbackChildOwner {
    approval_task: Option<JoinHandle<LoopbackTaskResult>>,
    ws_task: Option<JoinHandle<LoopbackTaskResult>>,
}

impl LoopbackChildOwner {
    async fn finish(
        mut self,
        timeout_after: Duration,
        abort_first: bool,
    ) -> Vec<Box<dyn Error + Send + Sync>> {
        join_loopback_children(
            self.approval_task.take(),
            self.ws_task.take(),
            timeout_after,
            abort_first,
        )
        .await
    }
}

impl Drop for LoopbackChildOwner {
    fn drop(&mut self) {
        if let Some(task) = self.approval_task.take() {
            task.abort();
        }
        if let Some(task) = self.ws_task.take() {
            task.abort();
        }
    }
}

pub struct LoopbackMarketSession {
    pub session: MarketStreamSession,
    clock: Arc<TestClock>,
    children: LoopbackChildOwner,
    _directory: TempDir,
}

impl LoopbackMarketSession {
    pub fn advance_clock_to(&self, target_ms: i64) {
        let delta = target_ms.saturating_sub(self.clock.now_ms());
        if delta > 0 {
            self.clock.advance_ms(delta);
        }
    }

    pub async fn next_receipt(
        &mut self,
    ) -> Result<kis_client::MarketReceipt, Box<dyn Error + Send + Sync>> {
        loop {
            match self.session.next_event().await? {
                MarketStreamEvent::Receipt(receipt) => return Ok(receipt),
                MarketStreamEvent::ApplicationHeartbeat
                | MarketStreamEvent::ControlPong
                | MarketStreamEvent::Status { .. } => continue,
            }
        }
    }

    pub async fn close(self) -> Result<(), Box<dyn Error + Send + Sync>> {
        let Self {
            session,
            clock: _,
            children,
            _directory,
        } = self;
        drop(session);
        let failures = children.finish(LOOPBACK_CHILD_JOIN_BOUND, false).await;
        drop(_directory);
        finish_loopback_errors(None, failures)
    }
}

pub async fn loopback_session_for_symbols(
    session_date: NaiveDate,
    now_ms: i64,
    credential_slot_id: Uuid,
    symbols: Vec<String>,
    ack_timeout: Duration,
) -> Result<LoopbackMarketSession, Box<dyn Error + Send + Sync>> {
    loopback_session_for_symbols_with_close_lifetime(
        session_date,
        now_ms,
        credential_slot_id,
        symbols,
        ack_timeout,
        DEFAULT_LOOPBACK_CLOSE_LIFETIME,
    )
    .await
}

pub async fn loopback_session_for_symbols_with_close_lifetime(
    session_date: NaiveDate,
    now_ms: i64,
    credential_slot_id: Uuid,
    symbols: Vec<String>,
    ack_timeout: Duration,
    close_lifetime: Duration,
) -> Result<LoopbackMarketSession, Box<dyn Error + Send + Sync>> {
    if !valid_loopback_close_lifetime(close_lifetime) {
        return Err("synthetic WebSocket close lifetime is outside its bound".into());
    }
    if symbols.is_empty() || symbols.iter().any(|symbol| !valid_symbol(symbol)) {
        return Err("synthetic WebSocket symbols are invalid".into());
    }
    let directory = tempfile::tempdir()?;
    let domain = MarketStreamDomain::for_test(directory.path(), credential_slot_id)?;
    let approval_listener = TcpListener::bind("127.0.0.1:0").await?;
    let approval_port = approval_listener.local_addr()?.port();
    let ws_listener = TcpListener::bind("127.0.0.1:0").await?;
    let ws_port = ws_listener.local_addr()?.port();
    let clock = Arc::new(TestClock::at(now_ms));
    let approval = ApprovalClient::for_loopback(
        &format!("http://127.0.0.1:{approval_port}"),
        Secret::new("SYNTHETIC_APPKEY".to_owned()),
        Secret::new("SYNTHETIC_SECRET".to_owned()),
        domain,
        "7",
    )?
    .with_clock(clock.clone());
    let session_date_wire = session_date.format("%Y%m%d").to_string().parse::<u32>()?;
    let kst = FixedOffset::east_opt(9 * 60 * 60).ok_or("KST offset invalid")?;
    let midnight_ms = session_date
        .and_hms_opt(0, 0, 0)
        .ok_or("KST midnight invalid")?
        .and_local_timezone(kst)
        .single()
        .ok_or("KST midnight ambiguous")?
        .timestamp_millis();
    let proof = MarketStreamSessionProof::new(session_date_wire, 0, 240_000, midnight_ms)?;
    let config =
        MarketStreamConfig::loopback(approval, &format!("ws://127.0.0.1:{ws_port}/tryitout"))?
            .with_session_proof(proof)
            .with_ack_timeout(ack_timeout)
            .with_clock(clock.clone());
    let children = LoopbackChildOwner {
        approval_task: Some(tokio::spawn(run_approval_server(approval_listener))),
        ws_task: Some(tokio::spawn(run_ws_server(
            ws_listener,
            session_date,
            symbols,
            close_lifetime,
        ))),
    };
    match MarketStreamClient::new(config).connect().await {
        Ok(session) => Ok(LoopbackMarketSession {
            session,
            clock,
            children,
            _directory: directory,
        }),
        Err(primary) => {
            let cleanup = children.finish(LOOPBACK_CHILD_JOIN_BOUND, true).await;
            drop(directory);
            finish_loopback_errors(Some(Box::new(primary)), cleanup)?;
            unreachable!("a failed loopback setup always returns its primary error")
        }
    }
}

fn valid_loopback_close_lifetime(close_lifetime: Duration) -> bool {
    !close_lifetime.is_zero() && close_lifetime <= MAX_LOOPBACK_CLOSE_LIFETIME
}

#[cfg(test)]
mod loopback_lifetime_tests {
    use std::future::pending;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    #[test]
    fn loopback_close_lifetime_keeps_default_and_caps_explicit_bound() {
        assert_eq!(DEFAULT_LOOPBACK_CLOSE_LIFETIME, Duration::from_secs(2));
        assert!(valid_loopback_close_lifetime(Duration::from_secs(20)));
        assert!(valid_loopback_close_lifetime(MAX_LOOPBACK_CLOSE_LIFETIME));
        assert!(!valid_loopback_close_lifetime(Duration::ZERO));
        assert!(!valid_loopback_close_lifetime(Duration::from_secs(31)));
    }

    #[tokio::test]
    async fn first_loopback_child_error_still_joins_the_remaining_child() {
        let remaining_finished = Arc::new(AtomicBool::new(false));
        let approval = tokio::spawn(async {
            Err(
                Box::new(std::io::Error::other("synthetic approval failure"))
                    as Box<dyn Error + Send + Sync>,
            )
        });
        let finished = remaining_finished.clone();
        let websocket = tokio::spawn(async move {
            finished.store(true, Ordering::SeqCst);
            Ok(())
        });
        let failures = join_loopback_children(
            Some(approval),
            Some(websocket),
            Duration::from_secs(1),
            false,
        )
        .await;
        assert_eq!(failures.len(), 1);
        assert!(remaining_finished.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn first_loopback_child_panic_still_joins_the_remaining_child() {
        let remaining_finished = Arc::new(AtomicBool::new(false));
        let approval = tokio::spawn(async { panic!("synthetic approval panic") });
        let finished = remaining_finished.clone();
        let websocket = tokio::spawn(async move {
            finished.store(true, Ordering::SeqCst);
            Ok(())
        });
        let failures = join_loopback_children(
            Some(approval),
            Some(websocket),
            Duration::from_secs(1),
            false,
        )
        .await;
        assert_eq!(failures.len(), 1);
        assert!(remaining_finished.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn timed_out_loopback_child_is_aborted_and_remaining_child_is_joined() {
        struct DropMark(Arc<AtomicBool>);
        impl Drop for DropMark {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let first_dropped = Arc::new(AtomicBool::new(false));
        let drop_mark = first_dropped.clone();
        let approval = tokio::spawn(async move {
            let _drop_mark = DropMark(drop_mark);
            pending::<()>().await;
            Ok(())
        });
        let remaining_finished = Arc::new(AtomicBool::new(false));
        let finished = remaining_finished.clone();
        let websocket = tokio::spawn(async move {
            finished.store(true, Ordering::SeqCst);
            Ok(())
        });
        let failures = join_loopback_children(
            Some(approval),
            Some(websocket),
            Duration::from_millis(10),
            false,
        )
        .await;
        assert_eq!(failures.len(), 1);
        assert!(first_dropped.load(Ordering::SeqCst));
        assert!(remaining_finished.load(Ordering::SeqCst));
    }
}

fn valid_symbol(symbol: &str) -> bool {
    symbol.len() == 6 && symbol.bytes().all(|byte| byte.is_ascii_digit())
}

async fn run_approval_server(listener: TcpListener) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (mut stream, _) = listener.accept().await?;
    let request = read_http_headers(&mut stream).await?;
    let length = header_value(&request, "content-length")
        .ok_or("missing synthetic approval content length")?
        .parse::<usize>()?;
    let mut body = vec![0_u8; length];
    stream.read_exact(&mut body).await?;
    let response_body = br#"{"approval_key":"SYNTHETIC_APPROVAL_KEY"}"#;
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response_body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.write_all(response_body).await?;
    Ok(())
}

async fn run_ws_server(
    listener: TcpListener,
    session_date: NaiveDate,
    symbols: Vec<String>,
    close_lifetime: Duration,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (mut stream, _) = listener.accept().await?;
    let request = read_http_headers(&mut stream).await?;
    let key = header_value(&request, "sec-websocket-key").ok_or("missing websocket key")?;
    let mut digest = sha1_smol::Sha1::new();
    digest.update(format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes());
    let accept = base64::engine::general_purpose::STANDARD.encode(digest.digest().bytes());
    stream
        .write_all(
            format!(
                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
            )
            .as_bytes(),
        )
        .await?;
    for symbol in symbols {
        let (opcode, command) = read_ws_frame(&mut stream).await?;
        assert_eq!(opcode, 0x1);
        let command: serde_json::Value = serde_json::from_slice(&command)?;
        assert_eq!(command["body"]["input"]["tr_id"], "H0STCNT0");
        assert_eq!(command["body"]["input"]["tr_key"], symbol);
        let (tr_type, acknowledgement) = match command["header"]["tr_type"].as_str() {
            Some("1") => ("1", "SUBSCRIBE SUCCESS"),
            Some("2") => ("2", "UNSUBSCRIBE SUCCESS"),
            _ => return Err("synthetic WebSocket command type is invalid".into()),
        };
        assert_eq!(command["header"]["tr_type"], tr_type);
        let ack = serde_json::to_vec(&serde_json::json!({
            "header": {"tr_id": "H0STCNT0", "tr_key": symbol.clone(), "encrypt": "N"},
            "body": {"rt_cd": "0", "msg_cd": "OPSP0000", "msg1": acknowledgement}
        }))?;
        write_ws_frame(&mut stream, 0x1, &ack).await?;
        if tr_type == "1" {
            // H0STCNT0 carries trade time only at one-second precision. Wait
            // past the current second boundary so capture cannot predate the
            // loopback socket-open timestamp by sub-second truncation.
            tokio::time::sleep(Duration::from_millis(1_100)).await;
            let kst = FixedOffset::east_opt(9 * 60 * 60).ok_or("KST offset invalid")?;
            let trade_time = Utc::now().with_timezone(&kst).format("%H%M%S").to_string();
            let fields = synthetic_market_fields(session_date, &trade_time, &symbol);
            let message = format!("0|H0STCNT0|001|{}", fields.join("^")).into_bytes();
            write_ws_frame(&mut stream, 0x1, &message).await?;
        }
    }
    let mut trailing_byte = [0_u8; 1];
    match tokio::time::timeout(close_lifetime, stream.read(&mut trailing_byte)).await {
        Ok(Ok(0)) => {}
        Ok(Ok(_)) => {
            return Err("unexpected synthetic WebSocket bytes after expected commands".into());
        }
        Ok(Err(error)) => return Err(error.into()),
        Err(_) => {
            return Err("synthetic WebSocket client did not close after expected commands".into());
        }
    }
    Ok(())
}

fn synthetic_market_fields(session_date: NaiveDate, trade_time: &str, symbol: &str) -> Vec<String> {
    let mut fields = vec![String::new(); 47];
    fields[0] = symbol.to_owned();
    fields[1] = trade_time.to_owned();
    fields[2] = "70000".to_owned();
    fields[3] = "2".to_owned();
    fields[4] = "100".to_owned();
    fields[5] = "0.14".to_owned();
    fields[12] = "10".to_owned();
    fields[13] = "20".to_owned();
    fields[33] = session_date.format("%Y%m%d").to_string();
    fields[34] = "20".to_owned();
    fields[35] = "N".to_owned();
    fields[43] = "0".to_owned();
    fields[46] = "2".to_owned();
    fields
}

async fn read_http_headers(stream: &mut TcpStream) -> Result<Vec<u8>, std::io::Error> {
    let mut bytes = Vec::new();
    let mut one = [0_u8; 1];
    while bytes.len() < 16 * 1024 {
        stream.read_exact(&mut one).await?;
        bytes.push(one[0]);
        if bytes.ends_with(b"\r\n\r\n") {
            return Ok(bytes);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "synthetic HTTP headers too large",
    ))
}

fn header_value(request: &[u8], wanted: &str) -> Option<String> {
    let text = String::from_utf8_lossy(request);
    text.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case(wanted)
            .then(|| value.trim().to_owned())
    })
}

async fn read_ws_frame(stream: &mut TcpStream) -> Result<(u8, Vec<u8>), std::io::Error> {
    let mut header = [0_u8; 2];
    stream.read_exact(&mut header).await?;
    let opcode = header[0] & 0x0f;
    let masked = header[1] & 0x80 != 0;
    let mut length = usize::from(header[1] & 0x7f);
    if length == 126 {
        let mut extended = [0_u8; 2];
        stream.read_exact(&mut extended).await?;
        length = usize::from(u16::from_be_bytes(extended));
    }
    let mut mask = [0_u8; 4];
    if masked {
        stream.read_exact(&mut mask).await?;
    }
    let mut payload = vec![0_u8; length];
    stream.read_exact(&mut payload).await?;
    if masked {
        for (index, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask[index % 4];
        }
    }
    Ok((opcode, payload))
}

async fn write_ws_frame(
    stream: &mut TcpStream,
    opcode: u8,
    payload: &[u8],
) -> Result<(), std::io::Error> {
    let mut frame = vec![0x80 | opcode];
    match payload.len() {
        0..=125 => frame.push(payload.len() as u8),
        126..=65_535 => {
            frame.push(126);
            frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        }
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "synthetic WebSocket payload too large",
            ));
        }
    }
    frame.extend_from_slice(payload);
    stream.write_all(&frame).await
}

pub async fn listener(database: &DisposableDatabase) -> Result<PgListener, sqlx::Error> {
    PgListener::connect_with(&database.worker).await
}
fn bounded_fixture_error(message: &'static str) -> Box<dyn Error + Send + Sync> {
    Box::new(std::io::Error::other(message))
}

async fn close_pools_until<'a>(
    pools: impl IntoIterator<Item = &'a PgPool>,
    deadline: tokio::time::Instant,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut close_failed = false;
    for pool in pools {
        if tokio::time::Instant::now() >= deadline
            || tokio::time::timeout_at(deadline, pool.close())
                .await
                .is_err()
        {
            close_failed = true;
        }
    }
    if close_failed {
        Err(bounded_fixture_error(
            "owned C2 fixture pool close exceeded its absolute deadline",
        ))
    } else {
        Ok(())
    }
}

async fn close_optional_pools_until(
    pools: [&Option<PgPool>; 7],
    deadline: tokio::time::Instant,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    close_pools_until(pools.into_iter().filter_map(Option::as_ref), deadline).await
}

async fn drop_generated_database_until(
    target: &SupervisorTarget,
    name: &str,
    deadline: tokio::time::Instant,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    if tokio::time::Instant::now() >= deadline {
        return Err(bounded_fixture_error(
            "generated C2 database cleanup deadline already expired",
        ));
    }
    match tokio::time::timeout_at(deadline, drop_generated_database(target, name)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(_)) => Err(bounded_fixture_error(
            "generated C2 database cleanup failed",
        )),
        Err(_) => Err(bounded_fixture_error(
            "generated C2 database cleanup exceeded its absolute deadline",
        )),
    }
}

fn combine_bounded_cleanup_errors(
    first: Option<Box<dyn Error + Send + Sync>>,
    second: Option<Box<dyn Error + Send + Sync>>,
) -> Option<Box<dyn Error + Send + Sync>> {
    match (first, second) {
        (Some(first), Some(second)) => Some(combine_primary_cleanup(first, second)),
        (Some(error), None) | (None, Some(error)) => Some(error),
        (None, None) => None,
    }
}

impl DisposableDatabase {
    pub(super) async fn create_until(
        setup_deadline: tokio::time::Instant,
        cleanup_deadline: tokio::time::Instant,
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let now = tokio::time::Instant::now();
        if setup_deadline <= now || cleanup_deadline <= setup_deadline {
            return Err(bounded_fixture_error(
                "C2 fixture deadlines are expired or out of order",
            ));
        }

        let supervisor_target = SupervisorTarget::from_environment()
            .map_err(|_| bounded_fixture_error("explicit C2 supervisor target was rejected"))?;
        if !supervisor_target.is_c2() {
            return Err(bounded_fixture_error(
                "bounded fixture creation requires the explicit current C2 target",
            ));
        }
        if tokio::time::Instant::now() >= setup_deadline {
            return Err(bounded_fixture_error(
                "C2 fixture setup deadline expired before connection",
            ));
        }

        let supervisor_options = supervisor_target.options();
        let name = generated_database_name();
        if !safe_database_name(&name) {
            return Err(bounded_fixture_error(
                "generated C2 database name failed its safety check",
            ));
        }

        // Keep every pool, target, and generated name outside the cancellable
        // setup future so timeout cleanup retains exact ownership.
        let mut supervisor: Option<PgPool> = None;
        let mut database_supervisor: Option<PgPool> = None;
        let mut migration_owner: Option<PgPool> = None;
        let mut app: Option<PgPool> = None;
        let mut worker: Option<PgPool> = None;
        let mut research_writer: Option<PgPool> = None;
        let mut admin: Option<PgPool> = None;
        let mut create_started = false;

        let setup = async {
            supervisor = Some(connect_supervisor(&supervisor_target).await?);
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_database WHERE datname = $1)",
            )
            .bind(&name)
            .fetch_one(
                supervisor
                    .as_ref()
                    .ok_or_else(|| bounded_fixture_error("C2 supervisor pool is unavailable"))?,
            )
            .await?;
            if exists {
                return Err(bounded_fixture_error(
                    "generated C2 database name already exists",
                ));
            }

            let mut role_tx = supervisor
                .as_ref()
                .ok_or_else(|| bounded_fixture_error("C2 supervisor pool is unavailable"))?
                .begin()
                .await?;
            sqlx::query("SELECT pg_catalog.pg_advisory_xact_lock(hashtext($1))")
                .bind("lagrange-ws3a-role-bootstrap")
                .execute(&mut *role_tx)
                .await?;
            sqlx::raw_sql(ROLE_BOOTSTRAP_SQL)
                .execute(&mut *role_tx)
                .await?;
            role_tx.commit().await?;

            let supervisor_pool = supervisor
                .as_ref()
                .ok_or_else(|| bounded_fixture_error("C2 supervisor pool is unavailable"))?;
            create_started = true;
            supervisor_pool
                .execute(database_ddl(&name, "CREATE DATABASE {db}"))
                .await?;
            supervisor_pool
                .execute(database_ddl(
                    &name,
                    "GRANT CONNECT ON DATABASE {db} TO migration_owner, app, worker, \
                     audit_writer, research_writer, admin",
                ))
                .await?;

            database_supervisor = Some(connect(supervisor_options.clone().database(&name)).await?);
            sqlx::raw_sql(BOOTSTRAP_SQL)
                .execute(
                    database_supervisor
                        .as_ref()
                        .ok_or_else(|| bounded_fixture_error("C2 database pool is unavailable"))?,
                )
                .await?;
            let database_supervisor_pool = database_supervisor
                .as_ref()
                .ok_or_else(|| bounded_fixture_error("C2 database pool is unavailable"))?;
            tokio::time::timeout_at(setup_deadline, database_supervisor_pool.close())
                .await
                .map_err(|_| {
                    bounded_fixture_error(
                        "C2 database bootstrap pool close exceeded setup deadline",
                    )
                })?;
            database_supervisor.take();

            migration_owner = Some(
                connect(
                    supervisor_options
                        .clone()
                        .database(&name)
                        .username("migration_owner")
                        .password("lagrange"),
                )
                .await?,
            );
            let migration_pool = migration_owner
                .as_ref()
                .ok_or_else(|| bounded_fixture_error("migration-owner pool is unavailable"))?;
            MIGRATOR.run(migration_pool).await?;
            let applied: i64 = sqlx::query_scalar("SELECT count(*) FROM public._sqlx_migrations")
                .fetch_one(migration_pool)
                .await?;
            let expected_applied = MIGRATOR
                .migrations
                .iter()
                .filter(|migration| migration.migration_type.is_up_migration())
                .count();
            if applied as usize != expected_applied {
                return Err(bounded_fixture_error(
                    "C2 fixture did not apply every repository up migration",
                ));
            }

            app = Some(
                connect(
                    supervisor_options
                        .clone()
                        .database(&name)
                        .username("app")
                        .password("lagrange"),
                )
                .await?,
            );
            worker = Some(
                connect(
                    supervisor_options
                        .clone()
                        .database(&name)
                        .username("worker")
                        .password("lagrange"),
                )
                .await?,
            );
            research_writer = Some(
                connect(
                    supervisor_options
                        .clone()
                        .database(&name)
                        .username("research_writer")
                        .password("lagrange"),
                )
                .await?,
            );
            admin = Some(
                connect(
                    supervisor_options
                        .clone()
                        .database(&name)
                        .username("admin")
                        .password("lagrange"),
                )
                .await?,
            );

            let supervisor_pool = supervisor
                .as_ref()
                .ok_or_else(|| bounded_fixture_error("C2 supervisor pool is unavailable"))?;
            tokio::time::timeout_at(setup_deadline, supervisor_pool.close())
                .await
                .map_err(|_| {
                    bounded_fixture_error("C2 supervisor pool close exceeded setup deadline")
                })?;
            supervisor.take();
            Ok::<(), Box<dyn Error + Send + Sync>>(())
        };

        let setup_result = tokio::time::timeout_at(setup_deadline, setup).await;
        let primary = match setup_result {
            Ok(Ok(())) if tokio::time::Instant::now() < setup_deadline => None,
            Ok(Ok(())) | Err(_) => Some(bounded_fixture_error(
                "bounded C2 fixture setup exceeded its absolute deadline",
            )),
            Ok(Err(_)) => Some(bounded_fixture_error("bounded C2 fixture setup failed")),
        };
        if let Some(primary) = primary {
            let pool_cleanup = close_optional_pools_until(
                [
                    &supervisor,
                    &database_supervisor,
                    &migration_owner,
                    &app,
                    &worker,
                    &research_writer,
                    &admin,
                ],
                cleanup_deadline,
            )
            .await
            .err();
            let database_cleanup = if create_started {
                drop_generated_database_until(&supervisor_target, &name, cleanup_deadline)
                    .await
                    .err()
            } else {
                None
            };
            return match combine_bounded_cleanup_errors(pool_cleanup, database_cleanup) {
                Some(cleanup) => Err(combine_primary_cleanup(primary, cleanup)),
                None => Err(primary),
            };
        }

        if migration_owner.is_none()
            || app.is_none()
            || worker.is_none()
            || research_writer.is_none()
            || admin.is_none()
        {
            let primary = bounded_fixture_error("bounded C2 fixture pool set is incomplete");
            let pool_cleanup = close_optional_pools_until(
                [
                    &supervisor,
                    &database_supervisor,
                    &migration_owner,
                    &app,
                    &worker,
                    &research_writer,
                    &admin,
                ],
                cleanup_deadline,
            )
            .await
            .err();
            let database_cleanup =
                drop_generated_database_until(&supervisor_target, &name, cleanup_deadline)
                    .await
                    .err();
            return match combine_bounded_cleanup_errors(pool_cleanup, database_cleanup) {
                Some(cleanup) => Err(combine_primary_cleanup(primary, cleanup)),
                None => Err(primary),
            };
        }

        Ok(Self {
            name,
            supervisor_target,
            migration_owner: migration_owner.expect("checked migration-owner pool"),
            app: app.expect("checked app pool"),
            worker: worker.expect("checked worker pool"),
            research_writer: research_writer.expect("checked research-writer pool"),
            admin: admin.expect("checked admin pool"),
        })
    }

    pub(super) async fn cleanup_until(
        self,
        cleanup_deadline: tokio::time::Instant,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let Self {
            name,
            supervisor_target,
            migration_owner,
            app,
            worker,
            research_writer,
            admin,
        } = self;
        let close_result = close_pools_until(
            [&migration_owner, &app, &worker, &research_writer, &admin],
            cleanup_deadline,
        )
        .await;
        let drop_result =
            drop_generated_database_until(&supervisor_target, &name, cleanup_deadline).await;
        match (close_result, drop_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(cleanup), Ok(())) | (Ok(()), Err(cleanup)) => Err(cleanup),
            (Err(close), Err(drop)) => Err(combine_primary_cleanup(close, drop)),
        }
    }
}
