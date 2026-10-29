//! Private, non-mutating observation of the worker's simulation-table ACL boundary.
//!
//! This records one completed connection observation only. It is not startup
//! readiness, rights authorization, a pool-wide claim, or a reusable permission.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use sqlx::{PgConnection, PgPool, Postgres, Row, pool::PoolConnection, postgres::PgRow};

use super::SimulationProcessConfig;

const WORKER_ROLE: &str = "worker";
const MIGRATION_OWNER_ROLE: &str = "migration_owner";
const PUBLIC_SCHEMA: &str = "public";

const CONTEXT_ROW_CAP: usize = 1;
const MEMBERSHIP_ROW_CAP: usize = 1;
const RELATION_ROW_CAP: usize = 2;
const COLUMN_ROW_CAP: usize = 128;
const ACL_ROW_CAP: usize = 256;

const BEGIN_SQL: &str = "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY";
const SETUP_SQL: &str = "\
    SELECT pg_catalog.set_config('search_path', 'pg_catalog', true), \
           pg_catalog.set_config('statement_timeout', $1::pg_catalog.text, true), \
           pg_catalog.set_config('lock_timeout', $1::pg_catalog.text, true)";
const CONTEXT_SQL: &str = r#"
SELECT current_user::pg_catalog.text AS current_user,
       session_user::pg_catalog.text AS session_user,
       pg_catalog.current_database()::pg_catalog.text AS observed_database_name,
       d.datname::pg_catalog.text AS database_name,
       n.nspname::pg_catalog.text AS schema_name,
       r.oid::pg_catalog.int8 AS worker_oid,
       d.oid::pg_catalog.int8 AS database_oid,
       n.oid::pg_catalog.int8 AS schema_oid,
       d.datdba::pg_catalog.int8 AS database_owner_oid,
       n.nspowner::pg_catalog.int8 AS schema_owner_oid,
       r.rolcanlogin AS can_login,
       r.rolsuper AS is_superuser,
       r.rolcreatedb AS can_create_database,
       r.rolcreaterole AS can_create_role,
       r.rolreplication AS can_replicate,
       r.rolbypassrls AS bypasses_rls,
       pg_catalog.current_setting('row_security')::pg_catalog.text AS row_security,
       pg_catalog.current_setting('session_replication_role')::pg_catalog.text
           AS session_replication_role,
       pg_catalog.current_setting('transaction_read_only')::pg_catalog.bool
           AS transaction_read_only,
       pg_catalog.current_setting('transaction_isolation')::pg_catalog.text
           AS transaction_isolation,
       pg_catalog.has_database_privilege(current_user, d.oid, 'CONNECT')
           AS database_connect,
       pg_catalog.has_database_privilege(current_user, d.oid, 'CREATE')
           AS database_create,
       pg_catalog.has_schema_privilege(current_user, n.oid, 'USAGE')
           AS schema_usage,
       pg_catalog.has_schema_privilege(current_user, n.oid, 'CREATE')
           AS schema_create,
       (
           pg_catalog.has_database_privilege(
               current_user, d.oid, 'CONNECT WITH GRANT OPTION'
           )
           OR pg_catalog.has_database_privilege(
               current_user, d.oid, 'CREATE WITH GRANT OPTION'
           )
           OR pg_catalog.has_database_privilege(
               current_user, d.oid, 'TEMPORARY WITH GRANT OPTION'
           )
       ) AS database_effective_grant_option,
       (
           pg_catalog.has_schema_privilege(
               current_user, n.oid, 'USAGE WITH GRANT OPTION'
           )
           OR pg_catalog.has_schema_privilege(
               current_user, n.oid, 'CREATE WITH GRANT OPTION'
           )
       ) AS schema_effective_grant_option,
       EXISTS (
           SELECT 1
             FROM pg_catalog.aclexplode(
                 COALESCE(d.datacl, pg_catalog.acldefault('d', d.datdba))
             ) AS acl
            WHERE acl.grantee IN (0::pg_catalog.oid, r.oid)
              AND acl.is_grantable
              AND acl.privilege_type IN ('CONNECT', 'CREATE', 'TEMPORARY')
       ) AS database_grant_option,
       EXISTS (
           SELECT 1
             FROM pg_catalog.aclexplode(
                 COALESCE(n.nspacl, pg_catalog.acldefault('n', n.nspowner))
             ) AS acl
            WHERE acl.grantee IN (0::pg_catalog.oid, r.oid)
              AND acl.is_grantable
              AND acl.privilege_type IN ('USAGE', 'CREATE')
       ) AS schema_grant_option
  FROM pg_catalog.pg_database AS d
  JOIN pg_catalog.pg_namespace AS n ON n.nspname = 'public'
  JOIN pg_catalog.pg_roles AS r ON r.rolname = current_user
 WHERE d.datname = pg_catalog.current_database()
 LIMIT 2
"#;
const MEMBERSHIP_SQL: &str = r#"
SELECT membership.member::pg_catalog.int8 AS member_oid,
       membership.roleid::pg_catalog.int8 AS role_oid,
       membership.grantor::pg_catalog.int8 AS grantor_oid,
       membership.admin_option,
       membership.inherit_option,
       membership.set_option
  FROM pg_catalog.pg_auth_members AS membership
 WHERE membership.member = $1::pg_catalog.oid
 ORDER BY membership.roleid, membership.grantor
 LIMIT 2
"#;
const RELATIONS_SQL: &str = r#"
SELECT n.oid::pg_catalog.int8 AS schema_oid,
       n.nspname::pg_catalog.text AS schema_name,
       c.relname::pg_catalog.text AS relation_name,
       c.oid::pg_catalog.int8 AS relation_oid,
       c.relowner::pg_catalog.int8 AS owner_oid,
       owner_role.rolname::pg_catalog.text AS owner_name,
       c.relkind::pg_catalog.text AS relation_kind,
       c.relpersistence::pg_catalog.text AS persistence,
       c.relispartition AS is_partition,
       EXISTS (
           SELECT 1
             FROM pg_catalog.pg_inherits AS inheritance
            WHERE inheritance.inhparent = c.oid
               OR inheritance.inhrelid = c.oid
       ) AS has_inheritance_edge,
       pg_catalog.has_table_privilege(current_user, c.oid, 'SELECT') AS table_select,
       pg_catalog.has_table_privilege(current_user, c.oid, 'INSERT') AS table_insert,
       pg_catalog.has_table_privilege(current_user, c.oid, 'UPDATE') AS table_update,
       pg_catalog.has_table_privilege(current_user, c.oid, 'DELETE') AS table_delete,
       pg_catalog.has_table_privilege(current_user, c.oid, 'TRUNCATE') AS table_truncate,
       pg_catalog.has_table_privilege(current_user, c.oid, 'REFERENCES')
           AS table_references,
       pg_catalog.has_table_privilege(current_user, c.oid, 'TRIGGER') AS table_trigger,
       (
           pg_catalog.has_table_privilege(
               current_user, c.oid, 'SELECT WITH GRANT OPTION'
           )
           OR pg_catalog.has_table_privilege(
               current_user, c.oid, 'INSERT WITH GRANT OPTION'
           )
           OR pg_catalog.has_table_privilege(
               current_user, c.oid, 'UPDATE WITH GRANT OPTION'
           )
           OR pg_catalog.has_table_privilege(
               current_user, c.oid, 'DELETE WITH GRANT OPTION'
           )
           OR pg_catalog.has_table_privilege(
               current_user, c.oid, 'TRUNCATE WITH GRANT OPTION'
           )
           OR pg_catalog.has_table_privilege(
               current_user, c.oid, 'REFERENCES WITH GRANT OPTION'
           )
           OR pg_catalog.has_table_privilege(
               current_user, c.oid, 'TRIGGER WITH GRANT OPTION'
           )
       ) AS table_effective_grant_option
  FROM pg_catalog.pg_class AS c
  JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
  LEFT JOIN pg_catalog.pg_roles AS owner_role ON owner_role.oid = c.relowner
 WHERE n.oid = $1::pg_catalog.oid
   AND c.relname IN (
       'owner_beta_simulation_runs',
       'owner_beta_simulation_result_pairs'
   )
 ORDER BY c.relname, c.oid
 LIMIT 3
"#;
const COLUMNS_SQL: &str = r#"
SELECT a.attrelid::pg_catalog.int8 AS relation_oid,
       c.relname::pg_catalog.text AS relation_name,
       a.attnum::pg_catalog.int4 AS attnum,
       a.attname::pg_catalog.text AS column_name,
       type_namespace.nspname::pg_catalog.text AS type_namespace,
       column_type.typname::pg_catalog.text AS type_name,
       a.attnotnull AS not_null,
       a.attisdropped AS is_dropped,
       a.attgenerated::pg_catalog.text AS generated,
       a.attidentity::pg_catalog.text AS identity,
       CASE WHEN NOT a.attisdropped
            THEN pg_catalog.has_column_privilege(
                current_user, a.attrelid, a.attname::pg_catalog.text, 'SELECT'
            )
            ELSE false
       END AS column_select,
       CASE WHEN NOT a.attisdropped
            THEN pg_catalog.has_column_privilege(
                current_user, a.attrelid, a.attname::pg_catalog.text, 'INSERT'
            )
            ELSE false
       END AS column_insert,
       CASE WHEN NOT a.attisdropped
            THEN pg_catalog.has_column_privilege(
                current_user, a.attrelid, a.attname::pg_catalog.text, 'UPDATE'
            )
            ELSE false
       END AS column_update,
       CASE WHEN NOT a.attisdropped
            THEN pg_catalog.has_column_privilege(
                current_user, a.attrelid, a.attname::pg_catalog.text, 'REFERENCES'
            )
            ELSE false
       END AS column_references,
       CASE WHEN NOT a.attisdropped
            THEN (
                pg_catalog.has_column_privilege(
                    current_user, a.attrelid, a.attname::pg_catalog.text,
                    'SELECT WITH GRANT OPTION'
                )
                OR pg_catalog.has_column_privilege(
                    current_user, a.attrelid, a.attname::pg_catalog.text,
                    'INSERT WITH GRANT OPTION'
                )
                OR pg_catalog.has_column_privilege(
                    current_user, a.attrelid, a.attname::pg_catalog.text,
                    'UPDATE WITH GRANT OPTION'
                )
                OR pg_catalog.has_column_privilege(
                    current_user, a.attrelid, a.attname::pg_catalog.text,
                    'REFERENCES WITH GRANT OPTION'
                )
            )
            ELSE false
       END AS column_effective_grant_option
  FROM pg_catalog.pg_attribute AS a
  JOIN pg_catalog.pg_class AS c ON c.oid = a.attrelid
  LEFT JOIN pg_catalog.pg_type AS column_type ON column_type.oid = a.atttypid
  LEFT JOIN pg_catalog.pg_namespace AS type_namespace
         ON type_namespace.oid = column_type.typnamespace
 WHERE a.attrelid IN ($1::pg_catalog.oid, $2::pg_catalog.oid)
   AND a.attnum > 0
 ORDER BY a.attrelid, a.attnum
 LIMIT 129
"#;
const ACLS_SQL: &str = r#"
WITH target_relations AS (
    SELECT c.oid, c.relowner, c.relacl
      FROM pg_catalog.pg_class AS c
     WHERE c.oid IN ($1::pg_catalog.oid, $2::pg_catalog.oid)
), raw_acl AS (
    SELECT target.oid::pg_catalog.int8 AS relation_oid,
           NULL::pg_catalog.int4 AS attnum,
           acl.grantee::pg_catalog.int8 AS grantee_oid,
           acl.grantor::pg_catalog.int8 AS grantor_oid,
           acl.privilege_type::pg_catalog.text AS privilege_type,
           acl.is_grantable
      FROM target_relations AS target
      CROSS JOIN LATERAL pg_catalog.aclexplode(
          COALESCE(target.relacl, pg_catalog.acldefault('r', target.relowner))
      ) AS acl
     WHERE acl.grantee IN (0::pg_catalog.oid, $3::pg_catalog.oid)
    UNION ALL
    SELECT attribute.attrelid::pg_catalog.int8 AS relation_oid,
           attribute.attnum::pg_catalog.int4 AS attnum,
           acl.grantee::pg_catalog.int8 AS grantee_oid,
           acl.grantor::pg_catalog.int8 AS grantor_oid,
           acl.privilege_type::pg_catalog.text AS privilege_type,
           acl.is_grantable
      FROM pg_catalog.pg_attribute AS attribute
      CROSS JOIN LATERAL pg_catalog.aclexplode(attribute.attacl) AS acl
     WHERE attribute.attrelid IN ($1::pg_catalog.oid, $2::pg_catalog.oid)
       AND attribute.attnum > 0
       AND acl.grantee IN (0::pg_catalog.oid, $3::pg_catalog.oid)
)
SELECT raw_acl.relation_oid,
       raw_acl.attnum,
       raw_acl.grantee_oid,
       raw_acl.grantor_oid,
       raw_acl.privilege_type,
       raw_acl.is_grantable
  FROM raw_acl
 ORDER BY raw_acl.relation_oid,
          raw_acl.attnum NULLS FIRST,
          raw_acl.grantee_oid,
          raw_acl.privilege_type,
          raw_acl.grantor_oid,
          raw_acl.is_grantable
 LIMIT 257
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(super) enum SimulationWorkerAclError {
    #[error("simulation worker ACL observation has an invalid bound")]
    InvalidBound,
    #[error("simulation worker ACL observation was canceled")]
    Canceled,
    #[error("simulation worker ACL connection acquisition failed")]
    AcquisitionFailed,
    #[error("simulation worker ACL transaction could not be started")]
    BeginFailed,
    #[error("simulation worker ACL transaction setup failed")]
    SetupFailed,
    #[error("simulation worker ACL catalog observation is unavailable")]
    CatalogUnavailable,
    #[error("simulation worker ACL catalog observation is invalid")]
    CatalogInvalid,
    #[error("simulation worker ACL access policy was rejected")]
    AccessRejected,
    #[error("simulation worker ACL transaction completion is unconfirmed")]
    CompletionUnconfirmed,
    #[error("simulation worker ACL transaction completion and cleanup are unconfirmed")]
    CompletionAndCleanupUnconfirmed,
    #[error("simulation worker ACL connection cleanup is unconfirmed")]
    CleanupUnconfirmed,
}

/// Opaque evidence from one fully completed, dedicated connection observation.
#[allow(dead_code)] // Relation identities remain in the sealed observation.
pub(super) struct SimulationWorkerAclObservation {
    config_sha256: String,
    database_oid: u32,
    worker_oid: u32,
    public_schema_oid: u32,
    runs_oid: u32,
    pairs_oid: u32,
}

#[allow(dead_code)]
impl SimulationWorkerAclObservation {
    fn completed(config_sha256: &str, evidence: ObservationEvidence) -> Self {
        Self {
            config_sha256: config_sha256.to_owned(),
            database_oid: evidence.database_oid,
            worker_oid: evidence.worker_oid,
            public_schema_oid: evidence.public_schema_oid,
            runs_oid: evidence.runs_oid,
            pairs_oid: evidence.pairs_oid,
        }
    }

    pub(super) fn config_sha256(&self) -> &str {
        &self.config_sha256
    }

    pub(super) fn database_oid(&self) -> u32 {
        self.database_oid
    }

    pub(super) fn worker_oid(&self) -> u32 {
        self.worker_oid
    }

    pub(super) fn public_schema_oid(&self) -> u32 {
        self.public_schema_oid
    }

    pub(super) fn runs_oid(&self) -> u32 {
        self.runs_oid
    }

    pub(super) fn pairs_oid(&self) -> u32 {
        self.pairs_oid
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Statement {
    Begin,
    Configure {
        timeout_ms: i32,
    },
    Context,
    Membership {
        worker_oid: u32,
    },
    Relations {
        schema_oid: u32,
    },
    Columns {
        runs_oid: u32,
        pairs_oid: u32,
    },
    Acls {
        runs_oid: u32,
        pairs_oid: u32,
        worker_oid: u32,
    },
    Commit,
    Rollback,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TransportFailure {
    Database,
    Decode,
    OverLimit,
    State,
}

#[derive(Clone, PartialEq, Eq)]
struct ContextRow {
    current_user: String,
    session_user: String,
    observed_database_name: String,
    database_name: String,
    schema_name: String,
    worker_oid: i64,
    database_oid: i64,
    schema_oid: i64,
    database_owner_oid: i64,
    schema_owner_oid: i64,
    can_login: bool,
    is_superuser: bool,
    can_create_database: bool,
    can_create_role: bool,
    can_replicate: bool,
    bypasses_rls: bool,
    row_security: String,
    session_replication_role: String,
    transaction_read_only: bool,
    transaction_isolation: String,
    database_connect: bool,
    database_create: bool,
    schema_usage: bool,
    schema_create: bool,
    database_effective_grant_option: bool,
    schema_effective_grant_option: bool,
    database_grant_option: bool,
    schema_grant_option: bool,
}

#[derive(Clone, PartialEq, Eq)]
struct MembershipRow {
    member_oid: i64,
    role_oid: i64,
    grantor_oid: i64,
    admin_option: bool,
    inherit_option: bool,
    set_option: bool,
}

#[derive(Clone, PartialEq, Eq)]
struct RelationRow {
    schema_oid: i64,
    schema_name: String,
    relation_name: String,
    relation_oid: i64,
    owner_oid: i64,
    owner_name: Option<String>,
    relation_kind: String,
    persistence: String,
    is_partition: bool,
    has_inheritance_edge: bool,
    table_select: bool,
    table_insert: bool,
    table_update: bool,
    table_delete: bool,
    table_truncate: bool,
    table_references: bool,
    table_trigger: bool,
    table_effective_grant_option: bool,
}

#[derive(Clone, PartialEq, Eq)]
struct ColumnRow {
    relation_oid: i64,
    relation_name: String,
    attnum: i32,
    column_name: String,
    type_namespace: Option<String>,
    type_name: Option<String>,
    not_null: bool,
    is_dropped: bool,
    generated: String,
    identity: String,
    column_select: bool,
    column_insert: bool,
    column_update: bool,
    column_references: bool,
    column_effective_grant_option: bool,
}

#[derive(Clone, PartialEq, Eq)]
struct AclRow {
    relation_oid: i64,
    attnum: Option<i32>,
    grantee_oid: i64,
    grantor_oid: i64,
    privilege_type: String,
    is_grantable: bool,
}

#[derive(Clone, PartialEq, Eq)]
enum CatalogRow {
    Command,
    Context(ContextRow),
    Membership(MembershipRow),
    Relation(RelationRow),
    Column(ColumnRow),
    Acl(AclRow),
}

#[async_trait]
trait ObserverTransport: Send {
    async fn acquire(&mut self) -> Result<(), TransportFailure>;
    fn arm_close_on_drop(&mut self);
    async fn execute(&mut self, statement: Statement) -> Result<Vec<CatalogRow>, TransportFailure>;
    async fn close(&mut self) -> Result<(), TransportFailure>;
}

// A command may return no rows; any row is a value-free rejection sentinel.
fn classify_command_result<T>(row: Option<T>) -> Vec<CatalogRow> {
    row.map(|_| CatalogRow::Command).into_iter().collect()
}

struct SqlxObserverTransport<'a> {
    pool: &'a PgPool,
    connection: Option<PoolConnection<Postgres>>,
}

impl<'a> SqlxObserverTransport<'a> {
    fn new(pool: &'a PgPool) -> Self {
        Self {
            pool,
            connection: None,
        }
    }

    fn connection(&mut self) -> Result<&mut PgConnection, TransportFailure> {
        let connection = self.connection.as_mut().ok_or(TransportFailure::State)?;
        Ok(&mut **connection)
    }

    async fn fetch_context(&mut self) -> Result<Vec<CatalogRow>, TransportFailure> {
        let rows = sqlx::query(CONTEXT_SQL)
            .fetch_all(self.connection()?)
            .await
            .map_err(|_| TransportFailure::Database)?;
        check_pg_row_cap(&rows, CONTEXT_ROW_CAP)?;
        rows.iter()
            .map(decode_context)
            .map(|row| row.map(CatalogRow::Context))
            .collect()
    }

    async fn fetch_memberships(
        &mut self,
        worker_oid: u32,
    ) -> Result<Vec<CatalogRow>, TransportFailure> {
        let rows = sqlx::query(MEMBERSHIP_SQL)
            .bind(i64::from(worker_oid))
            .fetch_all(self.connection()?)
            .await
            .map_err(|_| TransportFailure::Database)?;
        check_pg_row_cap(&rows, MEMBERSHIP_ROW_CAP)?;
        rows.iter()
            .map(decode_membership)
            .map(|row| row.map(CatalogRow::Membership))
            .collect()
    }

    async fn fetch_relations(
        &mut self,
        schema_oid: u32,
    ) -> Result<Vec<CatalogRow>, TransportFailure> {
        let rows = sqlx::query(RELATIONS_SQL)
            .bind(i64::from(schema_oid))
            .fetch_all(self.connection()?)
            .await
            .map_err(|_| TransportFailure::Database)?;
        check_pg_row_cap(&rows, RELATION_ROW_CAP)?;
        rows.iter()
            .map(decode_relation)
            .map(|row| row.map(CatalogRow::Relation))
            .collect()
    }

    async fn fetch_columns(
        &mut self,
        runs_oid: u32,
        pairs_oid: u32,
    ) -> Result<Vec<CatalogRow>, TransportFailure> {
        let rows = sqlx::query(COLUMNS_SQL)
            .bind(i64::from(runs_oid))
            .bind(i64::from(pairs_oid))
            .fetch_all(self.connection()?)
            .await
            .map_err(|_| TransportFailure::Database)?;
        check_pg_row_cap(&rows, COLUMN_ROW_CAP)?;
        rows.iter()
            .map(decode_column)
            .map(|row| row.map(CatalogRow::Column))
            .collect()
    }

    async fn fetch_acls(
        &mut self,
        runs_oid: u32,
        pairs_oid: u32,
        worker_oid: u32,
    ) -> Result<Vec<CatalogRow>, TransportFailure> {
        let rows = sqlx::query(ACLS_SQL)
            .bind(i64::from(runs_oid))
            .bind(i64::from(pairs_oid))
            .bind(i64::from(worker_oid))
            .fetch_all(self.connection()?)
            .await
            .map_err(|_| TransportFailure::Database)?;
        check_pg_row_cap(&rows, ACL_ROW_CAP)?;
        rows.iter()
            .map(decode_acl)
            .map(|row| row.map(CatalogRow::Acl))
            .collect()
    }
}

#[async_trait]
impl ObserverTransport for SqlxObserverTransport<'_> {
    async fn acquire(&mut self) -> Result<(), TransportFailure> {
        if self.connection.is_some() {
            return Err(TransportFailure::State);
        }
        self.connection = Some(
            self.pool
                .acquire()
                .await
                .map_err(|_| TransportFailure::Database)?,
        );
        Ok(())
    }

    fn arm_close_on_drop(&mut self) {
        if let Some(connection) = self.connection.as_mut() {
            connection.close_on_drop();
        }
    }

    async fn execute(&mut self, statement: Statement) -> Result<Vec<CatalogRow>, TransportFailure> {
        match statement {
            Statement::Begin => {
                let row = sqlx::query(BEGIN_SQL)
                    .fetch_optional(self.connection()?)
                    .await
                    .map_err(|_| TransportFailure::Database)?;
                Ok(classify_command_result(row))
            }
            Statement::Configure { timeout_ms } => {
                let timeout = timeout_ms.to_string();
                sqlx::query(SETUP_SQL)
                    .bind(timeout)
                    .execute(self.connection()?)
                    .await
                    .map_err(|_| TransportFailure::Database)?;
                Ok(Vec::new())
            }
            Statement::Context => self.fetch_context().await,
            Statement::Membership { worker_oid } => self.fetch_memberships(worker_oid).await,
            Statement::Relations { schema_oid } => self.fetch_relations(schema_oid).await,
            Statement::Columns {
                runs_oid,
                pairs_oid,
            } => self.fetch_columns(runs_oid, pairs_oid).await,
            Statement::Acls {
                runs_oid,
                pairs_oid,
                worker_oid,
            } => self.fetch_acls(runs_oid, pairs_oid, worker_oid).await,
            Statement::Commit => {
                let row = sqlx::query("COMMIT")
                    .fetch_optional(self.connection()?)
                    .await
                    .map_err(|_| TransportFailure::Database)?;
                Ok(classify_command_result(row))
            }
            Statement::Rollback => {
                let row = sqlx::query("ROLLBACK")
                    .fetch_optional(self.connection()?)
                    .await
                    .map_err(|_| TransportFailure::Database)?;
                Ok(classify_command_result(row))
            }
        }
    }

    async fn close(&mut self) -> Result<(), TransportFailure> {
        let connection = self.connection.take().ok_or(TransportFailure::State)?;
        connection
            .close()
            .await
            .map_err(|_| TransportFailure::Database)
    }
}

fn check_pg_row_cap(rows: &[PgRow], cap: usize) -> Result<(), TransportFailure> {
    if rows.len() > cap {
        Err(TransportFailure::OverLimit)
    } else {
        Ok(())
    }
}

fn decode<T>(row: &PgRow, column: &'static str) -> Result<T, TransportFailure>
where
    T: for<'r> sqlx::Decode<'r, Postgres> + sqlx::Type<Postgres>,
{
    row.try_get(column).map_err(|_| TransportFailure::Decode)
}

fn decode_context(row: &PgRow) -> Result<ContextRow, TransportFailure> {
    Ok(ContextRow {
        current_user: decode(row, "current_user")?,
        session_user: decode(row, "session_user")?,
        observed_database_name: decode(row, "observed_database_name")?,
        database_name: decode(row, "database_name")?,
        schema_name: decode(row, "schema_name")?,
        worker_oid: decode(row, "worker_oid")?,
        database_oid: decode(row, "database_oid")?,
        schema_oid: decode(row, "schema_oid")?,
        database_owner_oid: decode(row, "database_owner_oid")?,
        schema_owner_oid: decode(row, "schema_owner_oid")?,
        can_login: decode(row, "can_login")?,
        is_superuser: decode(row, "is_superuser")?,
        can_create_database: decode(row, "can_create_database")?,
        can_create_role: decode(row, "can_create_role")?,
        can_replicate: decode(row, "can_replicate")?,
        bypasses_rls: decode(row, "bypasses_rls")?,
        row_security: decode(row, "row_security")?,
        session_replication_role: decode(row, "session_replication_role")?,
        transaction_read_only: decode(row, "transaction_read_only")?,
        transaction_isolation: decode(row, "transaction_isolation")?,
        database_connect: decode(row, "database_connect")?,
        database_create: decode(row, "database_create")?,
        schema_usage: decode(row, "schema_usage")?,
        schema_create: decode(row, "schema_create")?,
        database_effective_grant_option: decode(row, "database_effective_grant_option")?,
        schema_effective_grant_option: decode(row, "schema_effective_grant_option")?,
        database_grant_option: decode(row, "database_grant_option")?,
        schema_grant_option: decode(row, "schema_grant_option")?,
    })
}

fn decode_membership(row: &PgRow) -> Result<MembershipRow, TransportFailure> {
    Ok(MembershipRow {
        member_oid: decode(row, "member_oid")?,
        role_oid: decode(row, "role_oid")?,
        grantor_oid: decode(row, "grantor_oid")?,
        admin_option: decode(row, "admin_option")?,
        inherit_option: decode(row, "inherit_option")?,
        set_option: decode(row, "set_option")?,
    })
}

fn decode_relation(row: &PgRow) -> Result<RelationRow, TransportFailure> {
    Ok(RelationRow {
        schema_oid: decode(row, "schema_oid")?,
        schema_name: decode(row, "schema_name")?,
        relation_name: decode(row, "relation_name")?,
        relation_oid: decode(row, "relation_oid")?,
        owner_oid: decode(row, "owner_oid")?,
        owner_name: decode(row, "owner_name")?,
        relation_kind: decode(row, "relation_kind")?,
        persistence: decode(row, "persistence")?,
        is_partition: decode(row, "is_partition")?,
        has_inheritance_edge: decode(row, "has_inheritance_edge")?,
        table_select: decode(row, "table_select")?,
        table_insert: decode(row, "table_insert")?,
        table_update: decode(row, "table_update")?,
        table_delete: decode(row, "table_delete")?,
        table_truncate: decode(row, "table_truncate")?,
        table_references: decode(row, "table_references")?,
        table_trigger: decode(row, "table_trigger")?,
        table_effective_grant_option: decode(row, "table_effective_grant_option")?,
    })
}

fn decode_column(row: &PgRow) -> Result<ColumnRow, TransportFailure> {
    Ok(ColumnRow {
        relation_oid: decode(row, "relation_oid")?,
        relation_name: decode(row, "relation_name")?,
        attnum: decode(row, "attnum")?,
        column_name: decode(row, "column_name")?,
        type_namespace: decode(row, "type_namespace")?,
        type_name: decode(row, "type_name")?,
        not_null: decode(row, "not_null")?,
        is_dropped: decode(row, "is_dropped")?,
        generated: decode(row, "generated")?,
        identity: decode(row, "identity")?,
        column_select: decode(row, "column_select")?,
        column_insert: decode(row, "column_insert")?,
        column_update: decode(row, "column_update")?,
        column_references: decode(row, "column_references")?,
        column_effective_grant_option: decode(row, "column_effective_grant_option")?,
    })
}

fn decode_acl(row: &PgRow) -> Result<AclRow, TransportFailure> {
    Ok(AclRow {
        relation_oid: decode(row, "relation_oid")?,
        attnum: decode(row, "attnum")?,
        grantee_oid: decode(row, "grantee_oid")?,
        grantor_oid: decode(row, "grantor_oid")?,
        privilege_type: decode(row, "privilege_type")?,
        is_grantable: decode(row, "is_grantable")?,
    })
}

#[derive(Clone, Copy)]
struct ColumnSpec {
    name: &'static str,
    type_name: &'static str,
    not_null: bool,
}

const RUN_COLUMNS: [ColumnSpec; 32] = [
    ColumnSpec {
        name: "id",
        type_name: "uuid",
        not_null: true,
    },
    ColumnSpec {
        name: "owner_user_id",
        type_name: "uuid",
        not_null: true,
    },
    ColumnSpec {
        name: "strategy_config_id",
        type_name: "uuid",
        not_null: true,
    },
    ColumnSpec {
        name: "job_id",
        type_name: "uuid",
        not_null: true,
    },
    ColumnSpec {
        name: "strategy_id",
        type_name: "text",
        not_null: true,
    },
    ColumnSpec {
        name: "strategy_version",
        type_name: "text",
        not_null: true,
    },
    ColumnSpec {
        name: "strategy_config_json",
        type_name: "jsonb",
        not_null: true,
    },
    ColumnSpec {
        name: "strategy_config_sha256",
        type_name: "text",
        not_null: true,
    },
    ColumnSpec {
        name: "evaluation_start",
        type_name: "date",
        not_null: true,
    },
    ColumnSpec {
        name: "evaluation_end",
        type_name: "date",
        not_null: true,
    },
    ColumnSpec {
        name: "session_count",
        type_name: "int4",
        not_null: true,
    },
    ColumnSpec {
        name: "input_kind",
        type_name: "text",
        not_null: true,
    },
    ColumnSpec {
        name: "capability",
        type_name: "text",
        not_null: true,
    },
    ColumnSpec {
        name: "audience",
        type_name: "text",
        not_null: true,
    },
    ColumnSpec {
        name: "vendor_snapshot",
        type_name: "bool",
        not_null: true,
    },
    ColumnSpec {
        name: "strict_pit",
        type_name: "bool",
        not_null: true,
    },
    ColumnSpec {
        name: "action_count",
        type_name: "int4",
        not_null: true,
    },
    ColumnSpec {
        name: "candidate_content_sha256",
        type_name: "text",
        not_null: true,
    },
    ColumnSpec {
        name: "artifact_manifest_sha256",
        type_name: "text",
        not_null: true,
    },
    ColumnSpec {
        name: "stage5_manifest_sha256",
        type_name: "text",
        not_null: true,
    },
    ColumnSpec {
        name: "action_manifest_sha256",
        type_name: "text",
        not_null: true,
    },
    ColumnSpec {
        name: "approval_registry_sha256",
        type_name: "text",
        not_null: true,
    },
    ColumnSpec {
        name: "contract_document_sha256",
        type_name: "text",
        not_null: true,
    },
    ColumnSpec {
        name: "bars_sha256",
        type_name: "text",
        not_null: true,
    },
    ColumnSpec {
        name: "status",
        type_name: "text",
        not_null: true,
    },
    ColumnSpec {
        name: "error_code",
        type_name: "text",
        not_null: false,
    },
    ColumnSpec {
        name: "engine_json",
        type_name: "jsonb",
        not_null: false,
    },
    ColumnSpec {
        name: "published_attempt_id",
        type_name: "uuid",
        not_null: false,
    },
    ColumnSpec {
        name: "created_at",
        type_name: "timestamptz",
        not_null: true,
    },
    ColumnSpec {
        name: "started_at",
        type_name: "timestamptz",
        not_null: false,
    },
    ColumnSpec {
        name: "finished_at",
        type_name: "timestamptz",
        not_null: false,
    },
    ColumnSpec {
        name: "updated_at",
        type_name: "timestamptz",
        not_null: true,
    },
];

const PAIR_COLUMNS: [ColumnSpec; 7] = [
    ColumnSpec {
        name: "run_id",
        type_name: "uuid",
        not_null: true,
    },
    ColumnSpec {
        name: "owner_user_id",
        type_name: "uuid",
        not_null: true,
    },
    ColumnSpec {
        name: "illustrative_canonical",
        type_name: "bytea",
        not_null: true,
    },
    ColumnSpec {
        name: "illustrative_sha256",
        type_name: "text",
        not_null: true,
    },
    ColumnSpec {
        name: "zero_cost_canonical",
        type_name: "bytea",
        not_null: true,
    },
    ColumnSpec {
        name: "zero_cost_sha256",
        type_name: "text",
        not_null: true,
    },
    ColumnSpec {
        name: "created_at",
        type_name: "timestamptz",
        not_null: true,
    },
];

#[derive(Clone, Copy)]
struct ContextEvidence {
    worker_oid: u32,
    database_oid: u32,
    schema_oid: u32,
}

#[derive(Clone, Copy)]
struct RelationEvidence {
    runs_oid: u32,
    pairs_oid: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct AclIdentity {
    relation_oid: u32,
    attnum: Option<i32>,
    grantee_oid: u32,
    grantor_oid: u32,
    privilege_type: &'static str,
    is_grantable: bool,
}

#[derive(Clone, Copy)]
struct ValidAcl<'a> {
    relation_oid: u32,
    attnum: Option<i32>,
    grantee_oid: u32,
    grantor_oid: u32,
    privilege_type: &'a str,
    is_grantable: bool,
}

/// Dropping this future yields no observation. The dedicated SQLx connection
/// is armed with close-on-drop before SQL, but drop does not prove an awaited
/// server rollback or close; the caller still owns task cancellation/reaping.
pub(super) async fn observe_simulation_worker_acl(
    pool: &PgPool,
    config: &SimulationProcessConfig,
    canceled: &AtomicBool,
) -> Result<SimulationWorkerAclObservation, SimulationWorkerAclError> {
    let mut transport = SqlxObserverTransport::new(pool);
    observe_with_transport(&mut transport, config, canceled).await
}

async fn observe_with_transport<T: ObserverTransport>(
    transport: &mut T,
    config: &SimulationProcessConfig,
    canceled: &AtomicBool,
) -> Result<SimulationWorkerAclObservation, SimulationWorkerAclError> {
    let timeout_ms = timeout_millis(config.limits().startup_timeout_seconds())?;
    if is_canceled(canceled) {
        return Err(SimulationWorkerAclError::Canceled);
    }

    if transport.acquire().await.is_err() {
        return Err(SimulationWorkerAclError::AcquisitionFailed);
    }
    transport.arm_close_on_drop();
    if is_canceled(canceled) {
        return Err(close_before_begin(transport, SimulationWorkerAclError::Canceled).await);
    }

    match transport.execute(Statement::Begin).await {
        Ok(rows) if rows.is_empty() => {}
        Ok(_) => {
            return Err(
                rollback_and_close(transport, SimulationWorkerAclError::CatalogInvalid).await,
            );
        }
        Err(_) => {
            return Err(close_before_begin(transport, SimulationWorkerAclError::BeginFailed).await);
        }
    }
    if is_canceled(canceled) {
        return Err(rollback_and_close(transport, SimulationWorkerAclError::Canceled).await);
    }

    match transport.execute(Statement::Configure { timeout_ms }).await {
        Ok(rows) if rows.is_empty() => {}
        _ => {
            return Err(rollback_and_close(transport, SimulationWorkerAclError::SetupFailed).await);
        }
    }
    if is_canceled(canceled) {
        return Err(rollback_and_close(transport, SimulationWorkerAclError::Canceled).await);
    }

    let context_rows = match transport.execute(Statement::Context).await {
        Ok(rows) => rows,
        Err(error) => {
            return Err(rollback_and_close(transport, catalog_error(error)).await);
        }
    };
    if is_canceled(canceled) {
        return Err(rollback_and_close(transport, SimulationWorkerAclError::Canceled).await);
    }
    let context = match compare_context(context_rows) {
        Ok(context) => context,
        Err(error) => return Err(rollback_and_close(transport, error).await),
    };

    let membership_rows = match transport
        .execute(Statement::Membership {
            worker_oid: context.worker_oid,
        })
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            return Err(rollback_and_close(transport, catalog_error(error)).await);
        }
    };
    if is_canceled(canceled) {
        return Err(rollback_and_close(transport, SimulationWorkerAclError::Canceled).await);
    }
    if let Err(error) = compare_memberships(membership_rows, context.worker_oid) {
        return Err(rollback_and_close(transport, error).await);
    }

    let relation_rows = match transport
        .execute(Statement::Relations {
            schema_oid: context.schema_oid,
        })
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            return Err(rollback_and_close(transport, catalog_error(error)).await);
        }
    };
    if is_canceled(canceled) {
        return Err(rollback_and_close(transport, SimulationWorkerAclError::Canceled).await);
    }
    let relations = match compare_relations(relation_rows, context) {
        Ok(relations) => relations,
        Err(error) => return Err(rollback_and_close(transport, error).await),
    };

    let column_rows = match transport
        .execute(Statement::Columns {
            runs_oid: relations.runs_oid,
            pairs_oid: relations.pairs_oid,
        })
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            return Err(rollback_and_close(transport, catalog_error(error)).await);
        }
    };
    if is_canceled(canceled) {
        return Err(rollback_and_close(transport, SimulationWorkerAclError::Canceled).await);
    }
    if let Err(error) = compare_columns(column_rows, relations) {
        return Err(rollback_and_close(transport, error).await);
    }

    let acl_rows = match transport
        .execute(Statement::Acls {
            runs_oid: relations.runs_oid,
            pairs_oid: relations.pairs_oid,
            worker_oid: context.worker_oid,
        })
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            return Err(rollback_and_close(transport, catalog_error(error)).await);
        }
    };
    if is_canceled(canceled) {
        return Err(rollback_and_close(transport, SimulationWorkerAclError::Canceled).await);
    }
    if let Err(error) = compare_acls(acl_rows, relations, context.worker_oid) {
        return Err(rollback_and_close(transport, error).await);
    }

    if is_canceled(canceled) {
        return Err(rollback_and_close(transport, SimulationWorkerAclError::Canceled).await);
    }
    match transport.execute(Statement::Commit).await {
        Ok(rows) if rows.is_empty() => {}
        Ok(_) | Err(_) => {
            return Err(match transport.close().await {
                Ok(()) => SimulationWorkerAclError::CompletionUnconfirmed,
                Err(
                    TransportFailure::Database
                    | TransportFailure::Decode
                    | TransportFailure::OverLimit
                    | TransportFailure::State,
                ) => SimulationWorkerAclError::CompletionAndCleanupUnconfirmed,
            });
        }
    }

    let canceled_after_commit = is_canceled(canceled);
    if transport.close().await.is_err() {
        return Err(SimulationWorkerAclError::CleanupUnconfirmed);
    }
    if canceled_after_commit || is_canceled(canceled) {
        return Err(SimulationWorkerAclError::Canceled);
    }

    Ok(SimulationWorkerAclObservation::completed(
        config.config_sha256(),
        ObservationEvidence {
            database_oid: context.database_oid,
            worker_oid: context.worker_oid,
            public_schema_oid: context.schema_oid,
            runs_oid: relations.runs_oid,
            pairs_oid: relations.pairs_oid,
        },
    ))
}

async fn close_before_begin<T: ObserverTransport>(
    transport: &mut T,
    primary: SimulationWorkerAclError,
) -> SimulationWorkerAclError {
    if transport.close().await.is_err() {
        SimulationWorkerAclError::CleanupUnconfirmed
    } else {
        primary
    }
}

async fn rollback_and_close<T: ObserverTransport>(
    transport: &mut T,
    primary: SimulationWorkerAclError,
) -> SimulationWorkerAclError {
    let rollback = matches!(
        transport.execute(Statement::Rollback).await,
        Ok(rows) if rows.is_empty()
    );
    let close = transport.close().await;
    if !rollback || close.is_err() {
        SimulationWorkerAclError::CleanupUnconfirmed
    } else {
        primary
    }
}

fn catalog_error(error: TransportFailure) -> SimulationWorkerAclError {
    match error {
        TransportFailure::OverLimit | TransportFailure::Decode | TransportFailure::State => {
            SimulationWorkerAclError::CatalogInvalid
        }
        TransportFailure::Database => SimulationWorkerAclError::CatalogUnavailable,
    }
}

fn timeout_millis(seconds: u64) -> Result<i32, SimulationWorkerAclError> {
    if seconds == 0 {
        return Err(SimulationWorkerAclError::InvalidBound);
    }
    let milliseconds = seconds
        .checked_mul(1000)
        .ok_or(SimulationWorkerAclError::InvalidBound)?;
    let milliseconds =
        i32::try_from(milliseconds).map_err(|_| SimulationWorkerAclError::InvalidBound)?;
    if milliseconds <= 0 {
        return Err(SimulationWorkerAclError::InvalidBound);
    }
    Ok(milliseconds)
}

fn is_canceled(canceled: &AtomicBool) -> bool {
    canceled.load(Ordering::Acquire)
}

fn compare_context(rows: Vec<CatalogRow>) -> Result<ContextEvidence, SimulationWorkerAclError> {
    if rows.len() > CONTEXT_ROW_CAP {
        return Err(SimulationWorkerAclError::CatalogInvalid);
    }
    let [CatalogRow::Context(row)] = rows.as_slice() else {
        return Err(SimulationWorkerAclError::CatalogUnavailable);
    };
    if !valid_catalog_name(&row.current_user, 63)
        || !valid_catalog_name(&row.session_user, 63)
        || !valid_catalog_name(&row.observed_database_name, 63)
        || !valid_catalog_name(&row.database_name, 63)
        || !valid_catalog_name(&row.schema_name, 63)
        || !valid_catalog_name(&row.row_security, 63)
        || !valid_catalog_name(&row.session_replication_role, 63)
        || !valid_catalog_name(&row.transaction_isolation, 63)
    {
        return Err(SimulationWorkerAclError::CatalogInvalid);
    }

    let worker_oid = positive_oid(row.worker_oid)?;
    let database_oid = positive_oid(row.database_oid)?;
    let schema_oid = positive_oid(row.schema_oid)?;
    let database_owner_oid = positive_oid(row.database_owner_oid)?;
    let schema_owner_oid = positive_oid(row.schema_owner_oid)?;
    if row.current_user != WORKER_ROLE
        || row.session_user != WORKER_ROLE
        || row.observed_database_name != row.database_name
        || row.schema_name != PUBLIC_SCHEMA
        || database_owner_oid == worker_oid
        || schema_owner_oid == worker_oid
        || !row.can_login
        || row.is_superuser
        || row.can_create_database
        || row.can_create_role
        || row.can_replicate
        || row.bypasses_rls
        || row.row_security != "on"
        || row.session_replication_role != "origin"
        || !row.transaction_read_only
        || row.transaction_isolation != "repeatable read"
        || !row.database_connect
        || row.database_create
        || !row.schema_usage
        || row.schema_create
        || row.database_effective_grant_option
        || row.schema_effective_grant_option
        || row.database_grant_option
        || row.schema_grant_option
    {
        return Err(SimulationWorkerAclError::AccessRejected);
    }
    Ok(ContextEvidence {
        worker_oid,
        database_oid,
        schema_oid,
    })
}

fn compare_memberships(
    rows: Vec<CatalogRow>,
    worker_oid: u32,
) -> Result<(), SimulationWorkerAclError> {
    if rows.len() > MEMBERSHIP_ROW_CAP {
        return Err(SimulationWorkerAclError::CatalogInvalid);
    }
    if let Some(row) = rows.into_iter().next() {
        let CatalogRow::Membership(row) = row else {
            return Err(SimulationWorkerAclError::CatalogInvalid);
        };
        if positive_oid(row.member_oid)? != worker_oid {
            return Err(SimulationWorkerAclError::CatalogInvalid);
        }
        positive_oid(row.role_oid)?;
        positive_oid(row.grantor_oid)?;
        let _options = (row.admin_option, row.inherit_option, row.set_option);
        return Err(SimulationWorkerAclError::AccessRejected);
    }
    Ok(())
}

fn compare_relations(
    rows: Vec<CatalogRow>,
    context: ContextEvidence,
) -> Result<RelationEvidence, SimulationWorkerAclError> {
    if rows.len() > RELATION_ROW_CAP {
        return Err(SimulationWorkerAclError::CatalogInvalid);
    }
    if rows.len() != RELATION_ROW_CAP {
        return Err(SimulationWorkerAclError::CatalogInvalid);
    }

    let mut runs_oid = None;
    let mut pairs_oid = None;
    let mut migration_owner_oid = None;
    for row in rows {
        let CatalogRow::Relation(row) = row else {
            return Err(SimulationWorkerAclError::CatalogInvalid);
        };
        validate_catalog_name(&row.schema_name, 63)?;
        validate_catalog_name(&row.relation_name, 63)?;
        let schema_oid = positive_oid(row.schema_oid)?;
        let relation_oid = positive_oid(row.relation_oid)?;
        let owner_oid = positive_oid(row.owner_oid)?;
        let Some(owner_name) = row.owner_name.as_deref() else {
            return Err(SimulationWorkerAclError::CatalogInvalid);
        };
        validate_catalog_name(owner_name, 63)?;
        if migration_owner_oid
            .replace(owner_oid)
            .is_some_and(|previous| previous != owner_oid)
        {
            return Err(SimulationWorkerAclError::CatalogInvalid);
        }
        validate_catalog_name(&row.relation_kind, 32)?;
        validate_catalog_name(&row.persistence, 32)?;
        if schema_oid != context.schema_oid
            || row.schema_name != PUBLIC_SCHEMA
            || owner_name != MIGRATION_OWNER_ROLE
            || owner_oid == context.worker_oid
            || row.relation_kind != "r"
            || row.persistence != "p"
            || row.is_partition
            || row.has_inheritance_edge
            || !row.table_select
            || row.table_insert
            || row.table_update
            || row.table_delete
            || row.table_truncate
            || row.table_references
            || row.table_trigger
            || row.table_effective_grant_option
        {
            return Err(SimulationWorkerAclError::AccessRejected);
        }

        match row.relation_name.as_str() {
            "owner_beta_simulation_runs" if runs_oid.replace(relation_oid).is_none() => {}
            "owner_beta_simulation_result_pairs" if pairs_oid.replace(relation_oid).is_none() => {}
            _ => return Err(SimulationWorkerAclError::CatalogInvalid),
        }
    }
    let runs_oid = runs_oid.ok_or(SimulationWorkerAclError::CatalogInvalid)?;
    let pairs_oid = pairs_oid.ok_or(SimulationWorkerAclError::CatalogInvalid)?;
    if runs_oid == pairs_oid {
        return Err(SimulationWorkerAclError::CatalogInvalid);
    }
    Ok(RelationEvidence {
        runs_oid,
        pairs_oid,
    })
}

fn compare_columns(
    rows: Vec<CatalogRow>,
    relations: RelationEvidence,
) -> Result<(), SimulationWorkerAclError> {
    if rows.len() > COLUMN_ROW_CAP {
        return Err(SimulationWorkerAclError::CatalogInvalid);
    }
    if rows.len() != RUN_COLUMNS.len() + PAIR_COLUMNS.len() {
        return Err(SimulationWorkerAclError::CatalogInvalid);
    }

    let mut runs = Vec::with_capacity(RUN_COLUMNS.len());
    let mut pairs = Vec::with_capacity(PAIR_COLUMNS.len());
    for row in rows {
        let CatalogRow::Column(row) = row else {
            return Err(SimulationWorkerAclError::CatalogInvalid);
        };
        match positive_oid(row.relation_oid)? {
            oid if oid == relations.runs_oid => runs.push(row),
            oid if oid == relations.pairs_oid => pairs.push(row),
            _ => return Err(SimulationWorkerAclError::CatalogInvalid),
        }
    }
    compare_table_columns(
        &runs,
        relations.runs_oid,
        "owner_beta_simulation_runs",
        &RUN_COLUMNS,
        TableKind::Runs,
    )?;
    compare_table_columns(
        &pairs,
        relations.pairs_oid,
        "owner_beta_simulation_result_pairs",
        &PAIR_COLUMNS,
        TableKind::Pairs,
    )?;
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TableKind {
    Runs,
    Pairs,
}

fn compare_table_columns(
    rows: &[ColumnRow],
    relation_oid: u32,
    relation_name: &str,
    expected: &[ColumnSpec],
    table: TableKind,
) -> Result<(), SimulationWorkerAclError> {
    if rows.len() != expected.len() {
        return Err(SimulationWorkerAclError::CatalogInvalid);
    }
    for (index, (row, expected)) in rows.iter().zip(expected).enumerate() {
        validate_catalog_name(&row.relation_name, 63)?;
        validate_catalog_name(&row.column_name, 63)?;
        let Some(type_namespace) = row.type_namespace.as_deref() else {
            return Err(SimulationWorkerAclError::CatalogInvalid);
        };
        let Some(type_name) = row.type_name.as_deref() else {
            return Err(SimulationWorkerAclError::CatalogInvalid);
        };
        validate_catalog_name(type_namespace, 63)?;
        validate_catalog_name(type_name, 63)?;

        let expected_attnum =
            i32::try_from(index + 1).map_err(|_| SimulationWorkerAclError::CatalogInvalid)?;
        if row.relation_oid != i64::from(relation_oid)
            || row.relation_name != relation_name
            || row.attnum <= 0
            || row.attnum != expected_attnum
            || row.column_name != expected.name
            || type_namespace != "pg_catalog"
            || type_name != expected.type_name
            || row.not_null != expected.not_null
            || row.is_dropped
            || !row.generated.is_empty()
            || !row.identity.is_empty()
        {
            return Err(SimulationWorkerAclError::CatalogInvalid);
        }
        if row.generated.len() > 1
            || row.identity.len() > 1
            || row.generated.chars().any(char::is_control)
            || row.identity.chars().any(char::is_control)
        {
            return Err(SimulationWorkerAclError::CatalogInvalid);
        }

        let expected_insert = table == TableKind::Pairs && pair_insert_column(expected.name);
        let expected_update = table == TableKind::Runs && runs_update_column(expected.name);
        if !row.column_select
            || row.column_references
            || row.column_effective_grant_option
            || row.column_insert != expected_insert
            || row.column_update != expected_update
        {
            return Err(SimulationWorkerAclError::AccessRejected);
        }
    }
    Ok(())
}

fn pair_insert_column(column: &str) -> bool {
    matches!(
        column,
        "run_id"
            | "owner_user_id"
            | "illustrative_canonical"
            | "illustrative_sha256"
            | "zero_cost_canonical"
            | "zero_cost_sha256"
    )
}

fn runs_update_column(column: &str) -> bool {
    matches!(
        column,
        "status"
            | "error_code"
            | "engine_json"
            | "published_attempt_id"
            | "started_at"
            | "finished_at"
            | "updated_at"
    )
}

fn compare_acls(
    rows: Vec<CatalogRow>,
    relations: RelationEvidence,
    worker_oid: u32,
) -> Result<(), SimulationWorkerAclError> {
    if rows.len() > ACL_ROW_CAP {
        return Err(SimulationWorkerAclError::CatalogInvalid);
    }

    let mut seen = HashSet::with_capacity(rows.len());
    let mut valid = Vec::with_capacity(rows.len());
    for row in rows {
        let CatalogRow::Acl(row) = row else {
            return Err(SimulationWorkerAclError::CatalogInvalid);
        };
        let relation_oid = positive_oid(row.relation_oid)?;
        let grantee_oid = if row.grantee_oid == 0 {
            0
        } else {
            positive_oid(row.grantee_oid)?
        };
        let grantor_oid = positive_oid(row.grantor_oid)?;
        validate_catalog_name(&row.privilege_type, 32)?;
        if relation_oid != relations.runs_oid && relation_oid != relations.pairs_oid {
            return Err(SimulationWorkerAclError::CatalogInvalid);
        }
        let Some(privilege_type) = known_privilege(row.privilege_type.as_str()) else {
            return Err(SimulationWorkerAclError::AccessRejected);
        };
        if !seen.insert(AclIdentity {
            relation_oid,
            attnum: row.attnum,
            grantee_oid,
            grantor_oid,
            privilege_type,
            is_grantable: row.is_grantable,
        }) {
            return Err(SimulationWorkerAclError::CatalogInvalid);
        }
        if grantee_oid == 0 {
            return Err(SimulationWorkerAclError::AccessRejected);
        }
        if grantee_oid != worker_oid {
            return Err(SimulationWorkerAclError::CatalogInvalid);
        }
        if row.is_grantable {
            return Err(SimulationWorkerAclError::AccessRejected);
        }

        match row.attnum {
            None => {
                if !matches!(privilege_type, "SELECT") {
                    return Err(SimulationWorkerAclError::AccessRejected);
                }
            }
            Some(attnum) => {
                let column = column_name(relations, relation_oid, attnum)?;
                let allowed = privilege_type == "SELECT"
                    || (relation_oid == relations.runs_oid
                        && privilege_type == "UPDATE"
                        && runs_update_column(column))
                    || (relation_oid == relations.pairs_oid
                        && privilege_type == "INSERT"
                        && pair_insert_column(column));
                if !allowed {
                    return Err(SimulationWorkerAclError::AccessRejected);
                }
            }
        }

        valid.push(ValidAcl {
            relation_oid,
            attnum: row.attnum,
            grantee_oid,
            grantor_oid,
            privilege_type,
            is_grantable: row.is_grantable,
        });
    }

    for relation_oid in [relations.runs_oid, relations.pairs_oid] {
        if !has_direct_acl(&valid, relation_oid, None, worker_oid, "SELECT") {
            return Err(SimulationWorkerAclError::AccessRejected);
        }
    }
    for (index, column) in RUN_COLUMNS.iter().enumerate() {
        if runs_update_column(column.name)
            && !has_direct_acl(
                &valid,
                relations.runs_oid,
                Some((index + 1) as i32),
                worker_oid,
                "UPDATE",
            )
        {
            return Err(SimulationWorkerAclError::AccessRejected);
        }
    }
    for (index, column) in PAIR_COLUMNS.iter().enumerate() {
        if pair_insert_column(column.name)
            && !has_direct_acl(
                &valid,
                relations.pairs_oid,
                Some((index + 1) as i32),
                worker_oid,
                "INSERT",
            )
        {
            return Err(SimulationWorkerAclError::AccessRejected);
        }
    }
    Ok(())
}

fn known_privilege(privilege: &str) -> Option<&'static str> {
    match privilege {
        "SELECT" | "INSERT" | "UPDATE" | "DELETE" | "TRUNCATE" | "REFERENCES" | "TRIGGER" => {
            Some(match privilege {
                "SELECT" => "SELECT",
                "INSERT" => "INSERT",
                "UPDATE" => "UPDATE",
                "DELETE" => "DELETE",
                "TRUNCATE" => "TRUNCATE",
                "REFERENCES" => "REFERENCES",
                "TRIGGER" => "TRIGGER",
                _ => return None,
            })
        }
        _ => None,
    }
}

fn column_name(
    relations: RelationEvidence,
    relation_oid: u32,
    attnum: i32,
) -> Result<&'static str, SimulationWorkerAclError> {
    if attnum <= 0 {
        return Err(SimulationWorkerAclError::CatalogInvalid);
    }
    let index =
        usize::try_from(attnum - 1).map_err(|_| SimulationWorkerAclError::CatalogInvalid)?;
    let columns = if relation_oid == relations.runs_oid {
        &RUN_COLUMNS[..]
    } else if relation_oid == relations.pairs_oid {
        &PAIR_COLUMNS[..]
    } else {
        return Err(SimulationWorkerAclError::CatalogInvalid);
    };
    columns
        .get(index)
        .map(|column| column.name)
        .ok_or(SimulationWorkerAclError::CatalogInvalid)
}

fn has_direct_acl(
    rows: &[ValidAcl<'_>],
    relation_oid: u32,
    attnum: Option<i32>,
    worker_oid: u32,
    privilege: &str,
) -> bool {
    rows.iter().any(|row| {
        row.relation_oid == relation_oid
            && row.attnum == attnum
            && row.grantee_oid == worker_oid
            && row.grantor_oid > 0
            && !row.is_grantable
            && row.privilege_type == privilege
    })
}

fn positive_oid(value: i64) -> Result<u32, SimulationWorkerAclError> {
    let oid = u32::try_from(value).map_err(|_| SimulationWorkerAclError::CatalogInvalid)?;
    if oid == 0 {
        Err(SimulationWorkerAclError::CatalogInvalid)
    } else {
        Ok(oid)
    }
}

fn valid_catalog_name(value: &str, max_bytes: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_bytes
        && !value.chars().any(char::is_control)
        && !value.contains('\0')
}

fn validate_catalog_name(value: &str, max_bytes: usize) -> Result<(), SimulationWorkerAclError> {
    if valid_catalog_name(value, max_bytes) {
        Ok(())
    } else {
        Err(SimulationWorkerAclError::CatalogInvalid)
    }
}

struct ObservationEvidence {
    database_oid: u32,
    worker_oid: u32,
    public_schema_oid: u32,
    runs_oid: u32,
    pairs_oid: u32,
}

#[cfg(test)]
#[path = "process_acl/tests.rs"]
mod tests;
