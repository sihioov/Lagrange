//! Private, bounded observation of the adopted simulation relation and RLS
//! policy metadata. This records identity and clause NULL shape only; it does
//! not inspect policy expressions or establish full startup safety.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use sqlx::{
    Column, PgConnection, PgPool, Postgres, Row, TypeInfo, pool::PoolConnection, postgres::PgRow,
};

use super::SimulationProcessConfig;

const POLICY_ID: &str = "r4-simulation-rls-metadata-v1";
const POLICY_FINGERPRINT: &str =
    "sha256:12add1b0db073a458329c7342e3da0b7ea3f26c37b857b6a5b1459e3d5e317c0";
const PUBLIC_SCHEMA: &str = "public";
const WORKER_ROLE: &str = "worker";
const MIGRATION_OWNER_ROLE: &str = "migration_owner";
const ROLE_NAMES: [&str; 6] = [
    "worker",
    "migration_owner",
    "app",
    "admin",
    "audit_writer",
    "research_writer",
];
#[cfg(test)]
const RELATION_NAMES: [&str; 2] = [
    "owner_beta_simulation_runs",
    "owner_beta_simulation_result_pairs",
];

const CONTEXT_ACCEPT_LIMIT: usize = 1;
const CONTEXT_FETCH_LIMIT: usize = 2;
const ROLE_ACCEPT_LIMIT: usize = 6;
const ROLE_FETCH_LIMIT: usize = 7;
const MEMBERSHIP_ACCEPT_LIMIT: usize = 0;
const MEMBERSHIP_FETCH_LIMIT: usize = 1;
const RELATION_ACCEPT_LIMIT: usize = 2;
const RELATION_FETCH_LIMIT: usize = 3;
const INHERITANCE_ACCEPT_LIMIT: usize = 0;
const INHERITANCE_FETCH_LIMIT: usize = 1;
const POLICY_ACCEPT_LIMIT: usize = 10;
const POLICY_FETCH_LIMIT: usize = 11;
const POLICY_ROLE_ACCEPT_LIMIT: usize = 10;
const POLICY_ROLE_FETCH_LIMIT: usize = 11;
const TEXT_ACCEPT_BYTES: usize = 1024;
const TEXT_FETCH_BYTES: usize = 1025;
const LOGICAL_PAYLOAD_LIMIT: usize = 65_536;

const BEGIN_SQL: &str = "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY";
const SETUP_SQL: &str = "\
    SELECT pg_catalog.set_config('search_path', 'pg_catalog', true), \
           pg_catalog.set_config('statement_timeout', $1::pg_catalog.text, true), \
           pg_catalog.set_config('lock_timeout', $1::pg_catalog.text, true)";

const CONTEXT_SQL: &str = r#"
SELECT (current_user IS NULL)::pg_catalog.bool AS current_user_is_null,
       CASE WHEN current_user IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(current_user::pg_catalog.text)::pg_catalog.int8
       END::pg_catalog.int8 AS current_user_bytes,
       CASE WHEN current_user IS NULL
                  OR pg_catalog.octet_length(current_user::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text
            ELSE current_user::pg_catalog.text
       END::pg_catalog.text AS current_user,
       (session_user IS NULL)::pg_catalog.bool AS session_user_is_null,
       CASE WHEN session_user IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(session_user::pg_catalog.text)::pg_catalog.int8
       END::pg_catalog.int8 AS session_user_bytes,
       CASE WHEN session_user IS NULL
                  OR pg_catalog.octet_length(session_user::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text
            ELSE session_user::pg_catalog.text
       END::pg_catalog.text AS session_user,
       (pg_catalog.current_database() IS NULL)::pg_catalog.bool
           AS observed_database_name_is_null,
       CASE WHEN pg_catalog.current_database() IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(pg_catalog.current_database()::pg_catalog.text)
                    ::pg_catalog.int8
       END::pg_catalog.int8 AS observed_database_name_bytes,
       CASE WHEN pg_catalog.current_database() IS NULL
                  OR pg_catalog.octet_length(pg_catalog.current_database()::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text
            ELSE pg_catalog.current_database()::pg_catalog.text
       END::pg_catalog.text AS observed_database_name,
       (database.datname IS NULL)::pg_catalog.bool AS database_name_is_null,
       CASE WHEN database.datname IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(database.datname::pg_catalog.text)::pg_catalog.int8
       END::pg_catalog.int8 AS database_name_bytes,
       CASE WHEN database.datname IS NULL
                  OR pg_catalog.octet_length(database.datname::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text
            ELSE database.datname::pg_catalog.text
       END::pg_catalog.text AS database_name,
       (public_schema.nspname IS NULL)::pg_catalog.bool AS schema_name_is_null,
       CASE WHEN public_schema.nspname IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(public_schema.nspname::pg_catalog.text)
                    ::pg_catalog.int8
       END::pg_catalog.int8 AS schema_name_bytes,
       CASE WHEN public_schema.nspname IS NULL
                  OR pg_catalog.octet_length(public_schema.nspname::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text
            ELSE public_schema.nspname::pg_catalog.text
       END::pg_catalog.text AS schema_name,
       worker.oid::pg_catalog.int8 AS worker_oid,
       database.oid::pg_catalog.int8 AS database_oid,
       public_schema.oid::pg_catalog.int8 AS schema_oid,
       migration_owner.oid::pg_catalog.int8 AS migration_owner_oid,
       database.datdba::pg_catalog.int8 AS database_owner_oid,
       public_schema.nspowner::pg_catalog.int8 AS schema_owner_oid,
       worker.rolcanlogin::pg_catalog.bool AS worker_can_login,
       worker.rolsuper::pg_catalog.bool AS worker_is_superuser,
       worker.rolcreatedb::pg_catalog.bool AS worker_can_create_database,
       worker.rolcreaterole::pg_catalog.bool AS worker_can_create_role,
       worker.rolreplication::pg_catalog.bool AS worker_can_replicate,
       worker.rolbypassrls::pg_catalog.bool AS worker_bypasses_rls,
       (pg_catalog.current_setting('row_security') IS NULL)::pg_catalog.bool
           AS row_security_is_null,
       CASE WHEN pg_catalog.current_setting('row_security') IS NULL
            THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(
                pg_catalog.current_setting('row_security')::pg_catalog.text
            )::pg_catalog.int8
       END::pg_catalog.int8 AS row_security_bytes,
       CASE WHEN pg_catalog.current_setting('row_security') IS NULL
                  OR pg_catalog.octet_length(
                      pg_catalog.current_setting('row_security')::pg_catalog.text
                  ) > 1025
            THEN ''::pg_catalog.text
            ELSE pg_catalog.current_setting('row_security')::pg_catalog.text
       END::pg_catalog.text AS row_security,
       (pg_catalog.current_setting('session_replication_role') IS NULL)::pg_catalog.bool
           AS session_replication_role_is_null,
       CASE WHEN pg_catalog.current_setting('session_replication_role') IS NULL
            THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(
                pg_catalog.current_setting('session_replication_role')::pg_catalog.text
            )::pg_catalog.int8
       END::pg_catalog.int8 AS session_replication_role_bytes,
       CASE WHEN pg_catalog.current_setting('session_replication_role') IS NULL
                  OR pg_catalog.octet_length(
                      pg_catalog.current_setting('session_replication_role')::pg_catalog.text
                  ) > 1025
            THEN ''::pg_catalog.text
            ELSE pg_catalog.current_setting('session_replication_role')::pg_catalog.text
       END::pg_catalog.text AS session_replication_role,
       pg_catalog.current_setting('transaction_read_only')::pg_catalog.bool
           AS transaction_read_only,
       (pg_catalog.current_setting('transaction_isolation') IS NULL)::pg_catalog.bool
           AS transaction_isolation_is_null,
       CASE WHEN pg_catalog.current_setting('transaction_isolation') IS NULL
            THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(
                pg_catalog.current_setting('transaction_isolation')::pg_catalog.text
            )::pg_catalog.int8
       END::pg_catalog.int8 AS transaction_isolation_bytes,
       CASE WHEN pg_catalog.current_setting('transaction_isolation') IS NULL
                  OR pg_catalog.octet_length(
                      pg_catalog.current_setting('transaction_isolation')::pg_catalog.text
                  ) > 1025
            THEN ''::pg_catalog.text
            ELSE pg_catalog.current_setting('transaction_isolation')::pg_catalog.text
       END::pg_catalog.text AS transaction_isolation,
       pg_catalog.has_database_privilege(
           current_user, database.oid, 'CONNECT'
       )::pg_catalog.bool AS database_connect,
       pg_catalog.has_database_privilege(
           current_user, database.oid, 'CREATE'
       )::pg_catalog.bool AS database_create,
       pg_catalog.has_schema_privilege(
           current_user, public_schema.oid, 'USAGE'
       )::pg_catalog.bool AS schema_usage,
       pg_catalog.has_schema_privilege(
           current_user, public_schema.oid, 'CREATE'
       )::pg_catalog.bool AS schema_create,
       (
           pg_catalog.has_database_privilege(
               current_user, database.oid, 'CONNECT WITH GRANT OPTION'
           )
           OR pg_catalog.has_database_privilege(
               current_user, database.oid, 'CREATE WITH GRANT OPTION'
           )
           OR pg_catalog.has_database_privilege(
               current_user, database.oid, 'TEMPORARY WITH GRANT OPTION'
           )
       )::pg_catalog.bool AS database_effective_grant_option,
       (
           pg_catalog.has_schema_privilege(
               current_user, public_schema.oid, 'USAGE WITH GRANT OPTION'
           )
           OR pg_catalog.has_schema_privilege(
               current_user, public_schema.oid, 'CREATE WITH GRANT OPTION'
           )
       )::pg_catalog.bool AS schema_effective_grant_option,
       EXISTS (
           SELECT 1
             FROM pg_catalog.aclexplode(
                 COALESCE(database.datacl, pg_catalog.acldefault('d', database.datdba))
             ) AS acl
            WHERE acl.grantee IN (0::pg_catalog.oid, worker.oid)
              AND acl.is_grantable
              AND acl.privilege_type IN ('CONNECT', 'CREATE', 'TEMPORARY')
       )::pg_catalog.bool AS database_raw_grant_option,
       EXISTS (
           SELECT 1
             FROM pg_catalog.aclexplode(
                 COALESCE(public_schema.nspacl,
                          pg_catalog.acldefault('n', public_schema.nspowner))
             ) AS acl
            WHERE acl.grantee IN (0::pg_catalog.oid, worker.oid)
              AND acl.is_grantable
              AND acl.privilege_type IN ('USAGE', 'CREATE')
       )::pg_catalog.bool AS schema_raw_grant_option
  FROM pg_catalog.pg_database AS database
  JOIN pg_catalog.pg_namespace AS public_schema ON public_schema.nspname = 'public'
  JOIN pg_catalog.pg_roles AS worker ON worker.rolname = 'worker'
  JOIN pg_catalog.pg_roles AS migration_owner ON migration_owner.rolname = 'migration_owner'
 WHERE database.datname = pg_catalog.current_database()
 LIMIT 2
"#;

const ROLES_SQL: &str = r#"
SELECT expected_roles.ordinal::pg_catalog.int4 AS role_ordinal,
       false::pg_catalog.bool AS expected_name_is_null,
       pg_catalog.octet_length(expected_roles.expected_name)::pg_catalog.int8
           AS expected_name_bytes,
       expected_roles.expected_name::pg_catalog.text AS expected_name,
       (actual_role.rolname IS NULL)::pg_catalog.bool AS actual_name_is_null,
       CASE WHEN actual_role.rolname IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(actual_role.rolname::pg_catalog.text)
                    ::pg_catalog.int8
       END::pg_catalog.int8 AS actual_name_bytes,
       CASE WHEN actual_role.rolname IS NULL
                  OR pg_catalog.octet_length(actual_role.rolname::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text
            ELSE actual_role.rolname::pg_catalog.text
       END::pg_catalog.text AS actual_name,
       actual_role.oid::pg_catalog.int8 AS role_oid
  FROM (VALUES
      (1::pg_catalog.int4, 'worker'::pg_catalog.text),
      (2::pg_catalog.int4, 'migration_owner'::pg_catalog.text),
      (3::pg_catalog.int4, 'app'::pg_catalog.text),
      (4::pg_catalog.int4, 'admin'::pg_catalog.text),
      (5::pg_catalog.int4, 'audit_writer'::pg_catalog.text),
      (6::pg_catalog.int4, 'research_writer'::pg_catalog.text)
  ) AS expected_roles(ordinal, expected_name)
  LEFT JOIN pg_catalog.pg_roles AS actual_role
    ON actual_role.rolname = expected_roles.expected_name
 ORDER BY expected_roles.ordinal
 LIMIT 7
"#;

const MEMBERSHIPS_SQL: &str = r#"
SELECT membership.member::pg_catalog.int8 AS member_oid,
       membership.roleid::pg_catalog.int8 AS role_oid,
       membership.grantor::pg_catalog.int8 AS grantor_oid,
       membership.admin_option::pg_catalog.bool AS admin_option,
       membership.inherit_option::pg_catalog.bool AS inherit_option,
       membership.set_option::pg_catalog.bool AS set_option
  FROM pg_catalog.pg_auth_members AS membership
 WHERE membership.member = $1::pg_catalog.oid
 ORDER BY membership.roleid, membership.grantor
 LIMIT 1
"#;

const RELATIONS_SQL: &str = r#"
SELECT namespace.oid::pg_catalog.int8 AS schema_oid,
       (namespace.nspname IS NULL)::pg_catalog.bool AS schema_name_is_null,
       CASE WHEN namespace.nspname IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(namespace.nspname::pg_catalog.text)
                    ::pg_catalog.int8
       END::pg_catalog.int8 AS schema_name_bytes,
       CASE WHEN namespace.nspname IS NULL
                  OR pg_catalog.octet_length(namespace.nspname::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text
            ELSE namespace.nspname::pg_catalog.text
       END::pg_catalog.text AS schema_name,
       (relation.relname IS NULL)::pg_catalog.bool AS relation_name_is_null,
       CASE WHEN relation.relname IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(relation.relname::pg_catalog.text)
                    ::pg_catalog.int8
       END::pg_catalog.int8 AS relation_name_bytes,
       CASE WHEN relation.relname IS NULL
                  OR pg_catalog.octet_length(relation.relname::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text
            ELSE relation.relname::pg_catalog.text
       END::pg_catalog.text AS relation_name,
       relation.oid::pg_catalog.int8 AS relation_oid,
       relation.relowner::pg_catalog.int8 AS owner_oid,
       (owner_role.rolname IS NULL)::pg_catalog.bool AS owner_name_is_null,
       CASE WHEN owner_role.rolname IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(owner_role.rolname::pg_catalog.text)
                    ::pg_catalog.int8
       END::pg_catalog.int8 AS owner_name_bytes,
       CASE WHEN owner_role.rolname IS NULL
                  OR pg_catalog.octet_length(owner_role.rolname::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text
            ELSE owner_role.rolname::pg_catalog.text
       END::pg_catalog.text AS owner_name,
       (relation.relkind IS NULL)::pg_catalog.bool AS relation_kind_is_null,
       CASE WHEN relation.relkind IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(relation.relkind::pg_catalog.text)
                    ::pg_catalog.int8
       END::pg_catalog.int8 AS relation_kind_bytes,
       CASE WHEN relation.relkind IS NULL
                  OR pg_catalog.octet_length(relation.relkind::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text
            ELSE relation.relkind::pg_catalog.text
       END::pg_catalog.text AS relation_kind,
       (relation.relpersistence IS NULL)::pg_catalog.bool AS persistence_is_null,
       CASE WHEN relation.relpersistence IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(relation.relpersistence::pg_catalog.text)
                    ::pg_catalog.int8
       END::pg_catalog.int8 AS persistence_bytes,
       CASE WHEN relation.relpersistence IS NULL
                  OR pg_catalog.octet_length(relation.relpersistence::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text
            ELSE relation.relpersistence::pg_catalog.text
       END::pg_catalog.text AS persistence,
       relation.relispartition::pg_catalog.bool AS is_partition,
       relation.relrowsecurity::pg_catalog.bool AS row_security_enabled,
       relation.relforcerowsecurity::pg_catalog.bool AS row_security_forced
  FROM pg_catalog.pg_class AS relation
  JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace
  LEFT JOIN pg_catalog.pg_roles AS owner_role ON owner_role.oid = relation.relowner
 WHERE namespace.nspname = 'public'
   AND relation.relname IN (
       'owner_beta_simulation_runs',
       'owner_beta_simulation_result_pairs'
   )
 ORDER BY namespace.oid, relation.oid
 LIMIT 3
"#;

const INHERITANCE_SQL: &str = r#"
SELECT inheritance.inhrelid::pg_catalog.int8 AS child_oid,
       inheritance.inhparent::pg_catalog.int8 AS parent_oid
  FROM pg_catalog.pg_inherits AS inheritance
 WHERE inheritance.inhrelid IN ($1::pg_catalog.oid, $2::pg_catalog.oid)
    OR inheritance.inhparent IN ($1::pg_catalog.oid, $2::pg_catalog.oid)
 ORDER BY inheritance.inhrelid, inheritance.inhparent
 LIMIT 1
"#;

const POLICY_HEADERS_SQL: &str = r#"
SELECT policy.oid::pg_catalog.int8 AS policy_oid,
       policy.polrelid::pg_catalog.int8 AS relation_oid,
       (policy.polname IS NULL)::pg_catalog.bool AS policy_name_is_null,
       CASE WHEN policy.polname IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(policy.polname::pg_catalog.text)
                    ::pg_catalog.int8
       END::pg_catalog.int8 AS policy_name_bytes,
       CASE WHEN policy.polname IS NULL
                  OR pg_catalog.octet_length(policy.polname::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text
            ELSE policy.polname::pg_catalog.text
       END::pg_catalog.text AS policy_name,
       false::pg_catalog.bool AS command_is_null,
       pg_catalog.octet_length(policy.polcmd::pg_catalog.text)::pg_catalog.int8
           AS command_bytes,
       policy.polcmd::pg_catalog.text AS command,
       policy.polpermissive::pg_catalog.bool AS permissive,
       (policy.polroles IS NULL)::pg_catalog.bool AS roles_is_null,
       pg_catalog.array_ndims(policy.polroles)::pg_catalog.int4 AS roles_dimensions,
       pg_catalog.array_lower(policy.polroles, 1)::pg_catalog.int4 AS roles_lower_bound,
       pg_catalog.cardinality(policy.polroles)::pg_catalog.int4 AS roles_length,
       (policy.polqual IS NULL)::pg_catalog.bool AS using_is_null,
       (policy.polwithcheck IS NULL)::pg_catalog.bool AS with_check_is_null
  FROM pg_catalog.pg_policy AS policy
 WHERE policy.polrelid IN ($1::pg_catalog.oid, $2::pg_catalog.oid)
 ORDER BY policy.polrelid, policy.oid
 LIMIT 11
"#;

const POLICY_ROLES_SQL: &str = r#"
SELECT policy.oid::pg_catalog.int8 AS policy_oid,
       policy.polrelid::pg_catalog.int8 AS relation_oid,
       policy_role.ordinality::pg_catalog.int8 AS role_ordinal,
       policy_role.role_oid::pg_catalog.int8 AS role_oid
  FROM pg_catalog.pg_policy AS policy
 CROSS JOIN LATERAL pg_catalog.unnest(policy.polroles)
      WITH ORDINALITY AS policy_role(role_oid, ordinality)
 WHERE policy.polrelid IN ($1::pg_catalog.oid, $2::pg_catalog.oid)
 ORDER BY policy.polrelid, policy.oid, policy_role.ordinality
 LIMIT 11
"#;

const FINAL_CONTEXT_SQL: &str = CONTEXT_SQL;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrimaryFailure {
    InvalidBound,
    Canceled,
    AcquisitionFailed,
    BeginFailed,
    BeginResultInvalid,
    SetupFailed,
    CatalogUnavailable,
    CatalogInvalid,
    PolicyRejected,
    BoundExceeded,
    PayloadExceeded,
    CompletionUnconfirmed,
    CleanupUnconfirmed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompletionState {
    NotAttempted,
    Acknowledged,
    Unconfirmed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CleanupState {
    NotApplicable,
    Confirmed,
    Unconfirmed,
}

/// Static failure and cleanup classification. No provider text or SQL error
/// is retained in this value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SimulationRlsMetadataError {
    primary: PrimaryFailure,
    completion: CompletionState,
    rollback: CleanupState,
    close: CleanupState,
    canceled: bool,
}

impl SimulationRlsMetadataError {
    fn primary(primary: PrimaryFailure) -> Self {
        Self {
            primary,
            completion: CompletionState::NotAttempted,
            rollback: CleanupState::NotApplicable,
            close: CleanupState::NotApplicable,
            canceled: false,
        }
    }
}

impl fmt::Display for SimulationRlsMetadataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("simulation RLS metadata observation failed closed")
    }
}

impl std::error::Error for SimulationRlsMetadataError {}

/// Opaque metadata from one acknowledged transaction and awaited close.
#[allow(dead_code)] // Policy and relation pins remain in the observation.
pub(super) struct SimulationRlsMetadataObservation {
    config_sha256: String,
    policy_id: &'static str,
    policy_fingerprint: &'static str,
    database_oid: u32,
    worker_oid: u32,
    public_schema_oid: u32,
    migration_owner_oid: u32,
    relation_oids: [u32; 2],
}

#[allow(dead_code)]
impl SimulationRlsMetadataObservation {
    fn completed(config_sha256: &str, context: &ContextEvidence, relation_oids: [u32; 2]) -> Self {
        Self {
            config_sha256: config_sha256.to_owned(),
            policy_id: POLICY_ID,
            policy_fingerprint: POLICY_FINGERPRINT,
            database_oid: context.database_oid,
            worker_oid: context.worker_oid,
            public_schema_oid: context.schema_oid,
            migration_owner_oid: context.migration_owner_oid,
            relation_oids,
        }
    }

    pub(super) fn config_sha256(&self) -> &str {
        &self.config_sha256
    }

    pub(super) fn policy_id(&self) -> &str {
        self.policy_id
    }

    pub(super) fn policy_fingerprint(&self) -> &str {
        self.policy_fingerprint
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

    pub(super) fn migration_owner_oid(&self) -> u32 {
        self.migration_owner_oid
    }

    pub(super) fn relation_oids(&self) -> &[u32; 2] {
        &self.relation_oids
    }
}

#[derive(Clone, Copy)]
struct RelationSpec {
    name: &'static str,
    slot: usize,
}

const RELATION_SPECS: [RelationSpec; 2] = [
    RelationSpec {
        name: "owner_beta_simulation_runs",
        slot: 0,
    },
    RelationSpec {
        name: "owner_beta_simulation_result_pairs",
        slot: 1,
    },
];

#[derive(Clone, Copy)]
struct PolicySpec {
    relation_slot: usize,
    name: &'static str,
    command: &'static str,
    role: &'static str,
    permissive: bool,
    using_is_null: bool,
    with_check_is_null: bool,
}

const POLICY_SPECS: [PolicySpec; 10] = [
    PolicySpec {
        relation_slot: 0,
        name: "owner_beta_simulation_runs_admin_select",
        command: "r",
        role: "admin",
        permissive: true,
        using_is_null: false,
        with_check_is_null: true,
    },
    PolicySpec {
        relation_slot: 0,
        name: "owner_beta_simulation_runs_app_insert",
        command: "a",
        role: "app",
        permissive: true,
        using_is_null: true,
        with_check_is_null: false,
    },
    PolicySpec {
        relation_slot: 0,
        name: "owner_beta_simulation_runs_app_select",
        command: "r",
        role: "app",
        permissive: true,
        using_is_null: false,
        with_check_is_null: true,
    },
    PolicySpec {
        relation_slot: 0,
        name: "owner_beta_simulation_runs_migration_owner_actor",
        command: "*",
        role: "migration_owner",
        permissive: true,
        using_is_null: false,
        with_check_is_null: false,
    },
    PolicySpec {
        relation_slot: 0,
        name: "owner_beta_simulation_runs_worker_select",
        command: "r",
        role: "worker",
        permissive: true,
        using_is_null: false,
        with_check_is_null: true,
    },
    PolicySpec {
        relation_slot: 0,
        name: "owner_beta_simulation_runs_worker_update",
        command: "w",
        role: "worker",
        permissive: true,
        using_is_null: false,
        with_check_is_null: false,
    },
    PolicySpec {
        relation_slot: 1,
        name: "owner_beta_simulation_pairs_app_select",
        command: "r",
        role: "app",
        permissive: true,
        using_is_null: false,
        with_check_is_null: true,
    },
    PolicySpec {
        relation_slot: 1,
        name: "owner_beta_simulation_pairs_migration_owner_actor",
        command: "*",
        role: "migration_owner",
        permissive: true,
        using_is_null: false,
        with_check_is_null: false,
    },
    PolicySpec {
        relation_slot: 1,
        name: "owner_beta_simulation_pairs_worker_insert",
        command: "a",
        role: "worker",
        permissive: true,
        using_is_null: true,
        with_check_is_null: false,
    },
    PolicySpec {
        relation_slot: 1,
        name: "owner_beta_simulation_pairs_worker_select",
        command: "r",
        role: "worker",
        permissive: true,
        using_is_null: false,
        with_check_is_null: true,
    },
];

#[derive(Clone, Copy)]
struct PolicyIdentity<'a> {
    relation_slot: usize,
    name: &'a str,
}

#[derive(Clone, Copy)]
enum PgCellKind {
    Bool(&'static str),
    I32(&'static str),
    OptionalI32(&'static str),
    I64(&'static str),
    OptionalI64(&'static str),
    Text {
        null_alias: &'static str,
        bytes_alias: &'static str,
        value_alias: &'static str,
    },
}

const CONTEXT_SHAPE: [PgCellKind; 29] = [
    PgCellKind::Text {
        null_alias: "current_user_is_null",
        bytes_alias: "current_user_bytes",
        value_alias: "current_user",
    },
    PgCellKind::Text {
        null_alias: "session_user_is_null",
        bytes_alias: "session_user_bytes",
        value_alias: "session_user",
    },
    PgCellKind::Text {
        null_alias: "observed_database_name_is_null",
        bytes_alias: "observed_database_name_bytes",
        value_alias: "observed_database_name",
    },
    PgCellKind::Text {
        null_alias: "database_name_is_null",
        bytes_alias: "database_name_bytes",
        value_alias: "database_name",
    },
    PgCellKind::Text {
        null_alias: "schema_name_is_null",
        bytes_alias: "schema_name_bytes",
        value_alias: "schema_name",
    },
    PgCellKind::I64("worker_oid"),
    PgCellKind::I64("database_oid"),
    PgCellKind::I64("schema_oid"),
    PgCellKind::I64("migration_owner_oid"),
    PgCellKind::I64("database_owner_oid"),
    PgCellKind::I64("schema_owner_oid"),
    PgCellKind::Bool("worker_can_login"),
    PgCellKind::Bool("worker_is_superuser"),
    PgCellKind::Bool("worker_can_create_database"),
    PgCellKind::Bool("worker_can_create_role"),
    PgCellKind::Bool("worker_can_replicate"),
    PgCellKind::Bool("worker_bypasses_rls"),
    PgCellKind::Text {
        null_alias: "row_security_is_null",
        bytes_alias: "row_security_bytes",
        value_alias: "row_security",
    },
    PgCellKind::Text {
        null_alias: "session_replication_role_is_null",
        bytes_alias: "session_replication_role_bytes",
        value_alias: "session_replication_role",
    },
    PgCellKind::Bool("transaction_read_only"),
    PgCellKind::Text {
        null_alias: "transaction_isolation_is_null",
        bytes_alias: "transaction_isolation_bytes",
        value_alias: "transaction_isolation",
    },
    PgCellKind::Bool("database_connect"),
    PgCellKind::Bool("database_create"),
    PgCellKind::Bool("schema_usage"),
    PgCellKind::Bool("schema_create"),
    PgCellKind::Bool("database_effective_grant_option"),
    PgCellKind::Bool("schema_effective_grant_option"),
    PgCellKind::Bool("database_raw_grant_option"),
    PgCellKind::Bool("schema_raw_grant_option"),
];

const ROLE_SHAPE: [PgCellKind; 4] = [
    PgCellKind::I32("role_ordinal"),
    PgCellKind::Text {
        null_alias: "expected_name_is_null",
        bytes_alias: "expected_name_bytes",
        value_alias: "expected_name",
    },
    PgCellKind::Text {
        null_alias: "actual_name_is_null",
        bytes_alias: "actual_name_bytes",
        value_alias: "actual_name",
    },
    PgCellKind::OptionalI64("role_oid"),
];

const MEMBERSHIP_SHAPE: [PgCellKind; 6] = [
    PgCellKind::I64("member_oid"),
    PgCellKind::I64("role_oid"),
    PgCellKind::I64("grantor_oid"),
    PgCellKind::Bool("admin_option"),
    PgCellKind::Bool("inherit_option"),
    PgCellKind::Bool("set_option"),
];

const RELATION_SHAPE: [PgCellKind; 11] = [
    PgCellKind::I64("schema_oid"),
    PgCellKind::Text {
        null_alias: "schema_name_is_null",
        bytes_alias: "schema_name_bytes",
        value_alias: "schema_name",
    },
    PgCellKind::Text {
        null_alias: "relation_name_is_null",
        bytes_alias: "relation_name_bytes",
        value_alias: "relation_name",
    },
    PgCellKind::I64("relation_oid"),
    PgCellKind::I64("owner_oid"),
    PgCellKind::Text {
        null_alias: "owner_name_is_null",
        bytes_alias: "owner_name_bytes",
        value_alias: "owner_name",
    },
    PgCellKind::Text {
        null_alias: "relation_kind_is_null",
        bytes_alias: "relation_kind_bytes",
        value_alias: "relation_kind",
    },
    PgCellKind::Text {
        null_alias: "persistence_is_null",
        bytes_alias: "persistence_bytes",
        value_alias: "persistence",
    },
    PgCellKind::Bool("is_partition"),
    PgCellKind::Bool("row_security_enabled"),
    PgCellKind::Bool("row_security_forced"),
];

const INHERITANCE_SHAPE: [PgCellKind; 2] =
    [PgCellKind::I64("child_oid"), PgCellKind::I64("parent_oid")];

const POLICY_HEADER_SHAPE: [PgCellKind; 11] = [
    PgCellKind::I64("policy_oid"),
    PgCellKind::I64("relation_oid"),
    PgCellKind::Text {
        null_alias: "policy_name_is_null",
        bytes_alias: "policy_name_bytes",
        value_alias: "policy_name",
    },
    PgCellKind::Text {
        null_alias: "command_is_null",
        bytes_alias: "command_bytes",
        value_alias: "command",
    },
    PgCellKind::Bool("permissive"),
    PgCellKind::Bool("roles_is_null"),
    PgCellKind::OptionalI32("roles_dimensions"),
    PgCellKind::OptionalI32("roles_lower_bound"),
    PgCellKind::OptionalI32("roles_length"),
    PgCellKind::Bool("using_is_null"),
    PgCellKind::Bool("with_check_is_null"),
];

const POLICY_ROLE_SHAPE: [PgCellKind; 4] = [
    PgCellKind::I64("policy_oid"),
    PgCellKind::I64("relation_oid"),
    PgCellKind::I64("role_ordinal"),
    PgCellKind::I64("role_oid"),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RawCategory {
    Context,
    Role,
    Membership,
    Relation,
    Inheritance,
    PolicyHeader,
    PolicyRole,
}

#[derive(Clone, PartialEq, Eq, Debug)]
enum RawCell {
    Null,
    WrongType,
    Bool(bool),
    I32(i32),
    I64(i64),
    Text {
        is_null: bool,
        original_bytes: Option<i64>,
        value: String,
    },
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct RawRow {
    category: RawCategory,
    cells: Vec<RawCell>,
}

impl RawRow {
    fn new(category: RawCategory, cells: Vec<RawCell>) -> Self {
        Self { category, cells }
    }
}

#[derive(Default)]
struct LogicalPayloadBudget {
    used: usize,
}

impl LogicalPayloadBudget {
    fn charge(
        &mut self,
        cell: &RawCell,
        kind: Option<PgCellKind>,
    ) -> Result<(), SimulationRlsMetadataError> {
        let bytes = match (cell, kind) {
            (RawCell::Text { value, .. }, Some(PgCellKind::Text { .. })) => {
                value.len().checked_add(10).ok_or_else(|| {
                    SimulationRlsMetadataError::primary(PrimaryFailure::PayloadExceeded)
                })?
            }
            (RawCell::Bool(_), _) | (RawCell::Null, Some(PgCellKind::Bool(_))) => 2,
            (RawCell::I32(_), _)
            | (RawCell::Null, Some(PgCellKind::I32(_)))
            | (RawCell::Null, Some(PgCellKind::OptionalI32(_))) => 5,
            (RawCell::I64(_), _)
            | (RawCell::Null, Some(PgCellKind::I64(_)))
            | (RawCell::Null, Some(PgCellKind::OptionalI64(_))) => 9,
            (RawCell::Null, Some(PgCellKind::Text { .. })) => 10,
            (RawCell::WrongType, Some(PgCellKind::Bool(_))) => 2,
            (RawCell::WrongType, Some(PgCellKind::I32(_)))
            | (RawCell::WrongType, Some(PgCellKind::OptionalI32(_))) => 5,
            (RawCell::WrongType, Some(PgCellKind::I64(_)))
            | (RawCell::WrongType, Some(PgCellKind::OptionalI64(_))) => 9,
            (RawCell::WrongType, Some(PgCellKind::Text { .. })) => 10,
            (RawCell::Null, _) | (RawCell::WrongType, _) => 1,
            (RawCell::Text { value, .. }, _) => value.len().checked_add(1).ok_or_else(|| {
                SimulationRlsMetadataError::primary(PrimaryFailure::PayloadExceeded)
            })?,
        };
        self.used = self
            .used
            .checked_add(bytes)
            .filter(|total| *total <= LOGICAL_PAYLOAD_LIMIT)
            .ok_or_else(|| SimulationRlsMetadataError::primary(PrimaryFailure::PayloadExceeded))?;
        Ok(())
    }

    fn charge_row(
        &mut self,
        row: &RawRow,
        shape: &[PgCellKind],
    ) -> Result<(), SimulationRlsMetadataError> {
        for (index, cell) in row.cells.iter().enumerate() {
            self.charge(cell, shape.get(index).copied())?;
        }
        Ok(())
    }
}

struct RawDecoder<'a> {
    cells: std::vec::IntoIter<RawCell>,
    budget: &'a mut LogicalPayloadBudget,
}

impl<'a> RawDecoder<'a> {
    fn new(
        row: RawRow,
        category: RawCategory,
        shape: &[PgCellKind],
        budget: &'a mut LogicalPayloadBudget,
    ) -> Result<Self, SimulationRlsMetadataError> {
        budget.charge_row(&row, shape)?;
        if row.category != category || row.cells.len() != shape.len() {
            return Err(SimulationRlsMetadataError::primary(
                PrimaryFailure::CatalogInvalid,
            ));
        }
        Ok(Self {
            cells: row.cells.into_iter(),
            budget,
        })
    }

    fn take(&mut self) -> Result<RawCell, SimulationRlsMetadataError> {
        self.cells
            .next()
            .ok_or_else(|| SimulationRlsMetadataError::primary(PrimaryFailure::CatalogInvalid))
    }

    fn boolean(&mut self) -> Result<bool, SimulationRlsMetadataError> {
        match self.take()? {
            RawCell::Bool(value) => Ok(value),
            _ => Err(SimulationRlsMetadataError::primary(
                PrimaryFailure::CatalogInvalid,
            )),
        }
    }

    fn i32(&mut self) -> Result<i32, SimulationRlsMetadataError> {
        match self.take()? {
            RawCell::I32(value) => Ok(value),
            _ => Err(SimulationRlsMetadataError::primary(
                PrimaryFailure::CatalogInvalid,
            )),
        }
    }

    fn optional_i32(&mut self) -> Result<Option<i32>, SimulationRlsMetadataError> {
        match self.take()? {
            RawCell::I32(value) => Ok(Some(value)),
            RawCell::Null => Ok(None),
            _ => Err(SimulationRlsMetadataError::primary(
                PrimaryFailure::CatalogInvalid,
            )),
        }
    }

    fn i64(&mut self) -> Result<i64, SimulationRlsMetadataError> {
        match self.take()? {
            RawCell::I64(value) => Ok(value),
            _ => Err(SimulationRlsMetadataError::primary(
                PrimaryFailure::CatalogInvalid,
            )),
        }
    }

    fn optional_i64(&mut self) -> Result<Option<i64>, SimulationRlsMetadataError> {
        match self.take()? {
            RawCell::I64(value) => Ok(Some(value)),
            RawCell::Null => Ok(None),
            _ => Err(SimulationRlsMetadataError::primary(
                PrimaryFailure::CatalogInvalid,
            )),
        }
    }

    fn text(&mut self) -> Result<String, SimulationRlsMetadataError> {
        self.text_inner(false)?
            .ok_or_else(|| SimulationRlsMetadataError::primary(PrimaryFailure::CatalogInvalid))
    }

    fn optional_text(&mut self) -> Result<Option<String>, SimulationRlsMetadataError> {
        self.text_inner(true)
    }

    fn text_inner(&mut self, nullable: bool) -> Result<Option<String>, SimulationRlsMetadataError> {
        match self.take()? {
            RawCell::Text {
                is_null: true,
                original_bytes: None,
                value,
            } if nullable && value.is_empty() => Ok(None),
            RawCell::Text {
                is_null: false,
                original_bytes: Some(original_bytes),
                value,
            } => {
                if original_bytes < 0 || value.len() > TEXT_FETCH_BYTES || value.contains('\0') {
                    return Err(SimulationRlsMetadataError::primary(
                        PrimaryFailure::CatalogInvalid,
                    ));
                }
                let original_length = usize::try_from(original_bytes).map_err(|_| {
                    SimulationRlsMetadataError::primary(PrimaryFailure::CatalogInvalid)
                })?;
                if original_length > TEXT_ACCEPT_BYTES {
                    return Err(SimulationRlsMetadataError::primary(
                        PrimaryFailure::BoundExceeded,
                    ));
                }
                if original_length != value.len() {
                    return Err(SimulationRlsMetadataError::primary(
                        PrimaryFailure::CatalogInvalid,
                    ));
                }
                Ok(Some(value))
            }
            _ => Err(SimulationRlsMetadataError::primary(
                PrimaryFailure::CatalogInvalid,
            )),
        }
    }

    fn finish(self) -> Result<(), SimulationRlsMetadataError> {
        let _ = self.budget.used;
        if self.cells.len() == 0 {
            Ok(())
        } else {
            Err(SimulationRlsMetadataError::primary(
                PrimaryFailure::CatalogInvalid,
            ))
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Statement {
    Begin,
    Configure { timeout_ms: i32 },
    Context,
    Roles,
    Memberships { worker_oid: u32 },
    Relations,
    Inheritance { relation_oids: [u32; 2] },
    PolicyHeaders { relation_oids: [u32; 2] },
    PolicyRoles { relation_oids: [u32; 2] },
    FinalContext,
    Commit,
    Rollback,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg(test)]
enum StatementKind {
    Begin,
    Configure,
    Context,
    Roles,
    Memberships,
    Relations,
    Inheritance,
    PolicyHeaders,
    PolicyRoles,
    FinalContext,
    Commit,
    Rollback,
}

impl Statement {
    #[cfg(test)]
    fn kind(self) -> StatementKind {
        match self {
            Self::Begin => StatementKind::Begin,
            Self::Configure { .. } => StatementKind::Configure,
            Self::Context => StatementKind::Context,
            Self::Roles => StatementKind::Roles,
            Self::Memberships { .. } => StatementKind::Memberships,
            Self::Relations => StatementKind::Relations,
            Self::Inheritance { .. } => StatementKind::Inheritance,
            Self::PolicyHeaders { .. } => StatementKind::PolicyHeaders,
            Self::PolicyRoles { .. } => StatementKind::PolicyRoles,
            Self::FinalContext => StatementKind::FinalContext,
            Self::Commit => StatementKind::Commit,
            Self::Rollback => StatementKind::Rollback,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TransportFailure {
    Database,
    State,
    FetchBound,
}

#[async_trait]
trait ObserverTransport: Send {
    async fn acquire(&mut self) -> Result<(), TransportFailure>;
    fn arm_close_on_drop(&mut self);
    async fn execute(&mut self, statement: Statement) -> Result<Vec<RawRow>, TransportFailure>;
    async fn close(&mut self) -> Result<(), TransportFailure>;
}

// A command may return no rows; any row is a value-free rejection sentinel.
fn classify_command_result<T>(row: Option<T>) -> Vec<RawRow> {
    row.map(|_| RawRow::new(RawCategory::Context, Vec::new()))
        .into_iter()
        .collect()
}

struct SqlxObserverTransport<'a> {
    pool: &'a PgPool,
    connection: Option<PoolConnection<Postgres>>,
}

fn timeout_bind_value(timeout_ms: i32) -> String {
    timeout_ms.to_string()
}

fn worker_oid_bind_value(worker_oid: u32) -> i64 {
    i64::from(worker_oid)
}

fn relation_oid_bind_values(relation_oids: [u32; 2]) -> [i64; 2] {
    [i64::from(relation_oids[0]), i64::from(relation_oids[1])]
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

    async fn fetch_rows(
        &mut self,
        sql: &'static str,
        category: RawCategory,
        shape: &[PgCellKind],
        fetch_limit: usize,
    ) -> Result<Vec<RawRow>, TransportFailure> {
        let rows = sqlx::query(sql)
            .fetch_all(self.connection()?)
            .await
            .map_err(|_| TransportFailure::Database)?;
        raw_pg_rows(rows, category, shape, fetch_limit)
    }

    async fn fetch_bound_rows(
        &mut self,
        sql: &'static str,
        relation_oids: [u32; 2],
        category: RawCategory,
        shape: &[PgCellKind],
        fetch_limit: usize,
    ) -> Result<Vec<RawRow>, TransportFailure> {
        let [first_oid, second_oid] = relation_oid_bind_values(relation_oids);
        let rows = sqlx::query(sql)
            .bind(first_oid)
            .bind(second_oid)
            .fetch_all(self.connection()?)
            .await
            .map_err(|_| TransportFailure::Database)?;
        raw_pg_rows(rows, category, shape, fetch_limit)
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

    async fn execute(&mut self, statement: Statement) -> Result<Vec<RawRow>, TransportFailure> {
        match statement {
            Statement::Begin => {
                let row = sqlx::query(BEGIN_SQL)
                    .fetch_optional(self.connection()?)
                    .await
                    .map_err(|_| TransportFailure::Database)?;
                Ok(classify_command_result(row))
            }
            Statement::Configure { timeout_ms } => {
                let timeout = timeout_bind_value(timeout_ms);
                sqlx::query(SETUP_SQL)
                    .bind(timeout)
                    .execute(self.connection()?)
                    .await
                    .map_err(|_| TransportFailure::Database)?;
                Ok(Vec::new())
            }
            Statement::Context => {
                self.fetch_rows(
                    CONTEXT_SQL,
                    RawCategory::Context,
                    &CONTEXT_SHAPE,
                    CONTEXT_FETCH_LIMIT,
                )
                .await
            }
            Statement::Roles => {
                self.fetch_rows(ROLES_SQL, RawCategory::Role, &ROLE_SHAPE, ROLE_FETCH_LIMIT)
                    .await
            }
            Statement::Memberships { worker_oid } => {
                let rows = sqlx::query(MEMBERSHIPS_SQL)
                    .bind(worker_oid_bind_value(worker_oid))
                    .fetch_all(self.connection()?)
                    .await
                    .map_err(|_| TransportFailure::Database)?;
                raw_pg_rows(
                    rows,
                    RawCategory::Membership,
                    &MEMBERSHIP_SHAPE,
                    MEMBERSHIP_FETCH_LIMIT,
                )
            }
            Statement::Relations => {
                self.fetch_rows(
                    RELATIONS_SQL,
                    RawCategory::Relation,
                    &RELATION_SHAPE,
                    RELATION_FETCH_LIMIT,
                )
                .await
            }
            Statement::Inheritance { relation_oids } => {
                self.fetch_bound_rows(
                    INHERITANCE_SQL,
                    relation_oids,
                    RawCategory::Inheritance,
                    &INHERITANCE_SHAPE,
                    INHERITANCE_FETCH_LIMIT,
                )
                .await
            }
            Statement::PolicyHeaders { relation_oids } => {
                self.fetch_bound_rows(
                    POLICY_HEADERS_SQL,
                    relation_oids,
                    RawCategory::PolicyHeader,
                    &POLICY_HEADER_SHAPE,
                    POLICY_FETCH_LIMIT,
                )
                .await
            }
            Statement::PolicyRoles { relation_oids } => {
                self.fetch_bound_rows(
                    POLICY_ROLES_SQL,
                    relation_oids,
                    RawCategory::PolicyRole,
                    &POLICY_ROLE_SHAPE,
                    POLICY_ROLE_FETCH_LIMIT,
                )
                .await
            }
            Statement::FinalContext => {
                self.fetch_rows(
                    FINAL_CONTEXT_SQL,
                    RawCategory::Context,
                    &CONTEXT_SHAPE,
                    CONTEXT_FETCH_LIMIT,
                )
                .await
            }
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

fn column_matches(row: &PgRow, index: usize, name: &str, type_name: &str) -> bool {
    row.columns()
        .get(index)
        .is_some_and(|column| column.name() == name && column.type_info().name() == type_name)
}

fn raw_pg_cell(row: &PgRow, index: &mut usize, kind: PgCellKind) -> RawCell {
    match kind {
        PgCellKind::Bool(alias) => {
            let at = *index;
            *index += 1;
            if !column_matches(row, at, alias, "BOOL") {
                return RawCell::WrongType;
            }
            match row.try_get::<Option<bool>, _>(at) {
                Ok(Some(value)) => RawCell::Bool(value),
                _ => RawCell::WrongType,
            }
        }
        PgCellKind::I32(alias) => {
            let at = *index;
            *index += 1;
            if !column_matches(row, at, alias, "INT4") {
                return RawCell::WrongType;
            }
            match row.try_get::<Option<i32>, _>(at) {
                Ok(Some(value)) => RawCell::I32(value),
                _ => RawCell::WrongType,
            }
        }
        PgCellKind::OptionalI32(alias) => {
            let at = *index;
            *index += 1;
            if !column_matches(row, at, alias, "INT4") {
                return RawCell::WrongType;
            }
            match row.try_get::<Option<i32>, _>(at) {
                Ok(Some(value)) => RawCell::I32(value),
                Ok(None) => RawCell::Null,
                Err(_) => RawCell::WrongType,
            }
        }
        PgCellKind::I64(alias) => {
            let at = *index;
            *index += 1;
            if !column_matches(row, at, alias, "INT8") {
                return RawCell::WrongType;
            }
            match row.try_get::<Option<i64>, _>(at) {
                Ok(Some(value)) => RawCell::I64(value),
                _ => RawCell::WrongType,
            }
        }
        PgCellKind::OptionalI64(alias) => {
            let at = *index;
            *index += 1;
            if !column_matches(row, at, alias, "INT8") {
                return RawCell::WrongType;
            }
            match row.try_get::<Option<i64>, _>(at) {
                Ok(Some(value)) => RawCell::I64(value),
                Ok(None) => RawCell::Null,
                Err(_) => RawCell::WrongType,
            }
        }
        PgCellKind::Text {
            null_alias,
            bytes_alias,
            value_alias,
        } => {
            let null_at = *index;
            let bytes_at = null_at + 1;
            let value_at = null_at + 2;
            *index += 3;
            if !column_matches(row, null_at, null_alias, "BOOL")
                || !column_matches(row, bytes_at, bytes_alias, "INT8")
                || !column_matches(row, value_at, value_alias, "TEXT")
            {
                return RawCell::WrongType;
            }
            match (
                row.try_get::<Option<bool>, _>(null_at),
                row.try_get::<Option<i64>, _>(bytes_at),
                row.try_get::<Option<&str>, _>(value_at),
            ) {
                (Ok(Some(is_null)), Ok(original_bytes), Ok(Some(value))) => RawCell::Text {
                    is_null,
                    original_bytes,
                    value: value.to_owned(),
                },
                _ => RawCell::WrongType,
            }
        }
    }
}

fn pg_column_count(shape: &[PgCellKind]) -> usize {
    shape
        .iter()
        .map(|kind| match kind {
            PgCellKind::Text { .. } => 3,
            _ => 1,
        })
        .sum()
}

fn raw_pg_rows(
    rows: Vec<PgRow>,
    category: RawCategory,
    shape: &[PgCellKind],
    fetch_limit: usize,
) -> Result<Vec<RawRow>, TransportFailure> {
    if rows.len() > fetch_limit {
        return Err(TransportFailure::FetchBound);
    }
    let field_count = shape.len();
    rows.into_iter()
        .map(|row| {
            if row.columns().len() != pg_column_count(shape) {
                return Ok(RawRow::new(category, vec![RawCell::WrongType; field_count]));
            }
            let mut column_index = 0;
            let cells = shape
                .iter()
                .map(|kind| raw_pg_cell(&row, &mut column_index, *kind))
                .collect();
            Ok(RawRow::new(category, cells))
        })
        .collect()
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
    migration_owner_oid: i64,
    database_owner_oid: i64,
    schema_owner_oid: i64,
    worker_can_login: bool,
    worker_is_superuser: bool,
    worker_can_create_database: bool,
    worker_can_create_role: bool,
    worker_can_replicate: bool,
    worker_bypasses_rls: bool,
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
    database_raw_grant_option: bool,
    schema_raw_grant_option: bool,
}

#[derive(Clone, PartialEq, Eq)]
struct ContextEvidence {
    snapshot: ContextRow,
    database_oid: u32,
    worker_oid: u32,
    schema_oid: u32,
    migration_owner_oid: u32,
    database_owner_oid: u32,
    schema_owner_oid: u32,
}

struct RoleRow {
    ordinal: i32,
    expected_name: String,
    actual_name: Option<String>,
    oid: Option<i64>,
}

struct MembershipRow {
    member_oid: i64,
    role_oid: i64,
    grantor_oid: i64,
}

struct RelationRow {
    schema_oid: i64,
    schema_name: String,
    name: String,
    oid: i64,
    owner_oid: i64,
    owner_name: Option<String>,
    kind: String,
    persistence: String,
    is_partition: bool,
    row_security_enabled: bool,
    row_security_forced: bool,
}

struct InheritanceRow {
    child_oid: i64,
    parent_oid: i64,
}

struct PolicyHeader {
    oid: u32,
    relation_oid: u32,
    name: String,
    command: String,
    permissive: bool,
    roles_is_null: bool,
    roles_dimensions: Option<i32>,
    roles_lower_bound: Option<i32>,
    roles_length: Option<i32>,
    using_is_null: bool,
    with_check_is_null: bool,
}

struct PolicyRoleRow {
    policy_oid: u32,
    relation_oid: u32,
    ordinal: i64,
    role_oid: u32,
}

struct PreparedObservation {
    context: ContextEvidence,
    relation_oids: [u32; 2],
}

fn decode_context(
    row: RawRow,
    budget: &mut LogicalPayloadBudget,
) -> Result<ContextRow, SimulationRlsMetadataError> {
    let mut decoder = RawDecoder::new(row, RawCategory::Context, &CONTEXT_SHAPE, budget)?;
    let value = ContextRow {
        current_user: decoder.text()?,
        session_user: decoder.text()?,
        observed_database_name: decoder.text()?,
        database_name: decoder.text()?,
        schema_name: decoder.text()?,
        worker_oid: decoder.i64()?,
        database_oid: decoder.i64()?,
        schema_oid: decoder.i64()?,
        migration_owner_oid: decoder.i64()?,
        database_owner_oid: decoder.i64()?,
        schema_owner_oid: decoder.i64()?,
        worker_can_login: decoder.boolean()?,
        worker_is_superuser: decoder.boolean()?,
        worker_can_create_database: decoder.boolean()?,
        worker_can_create_role: decoder.boolean()?,
        worker_can_replicate: decoder.boolean()?,
        worker_bypasses_rls: decoder.boolean()?,
        row_security: decoder.text()?,
        session_replication_role: decoder.text()?,
        transaction_read_only: decoder.boolean()?,
        transaction_isolation: decoder.text()?,
        database_connect: decoder.boolean()?,
        database_create: decoder.boolean()?,
        schema_usage: decoder.boolean()?,
        schema_create: decoder.boolean()?,
        database_effective_grant_option: decoder.boolean()?,
        schema_effective_grant_option: decoder.boolean()?,
        database_raw_grant_option: decoder.boolean()?,
        schema_raw_grant_option: decoder.boolean()?,
    };
    decoder.finish()?;
    Ok(value)
}

fn decode_role(
    row: RawRow,
    budget: &mut LogicalPayloadBudget,
) -> Result<RoleRow, SimulationRlsMetadataError> {
    let mut decoder = RawDecoder::new(row, RawCategory::Role, &ROLE_SHAPE, budget)?;
    let value = RoleRow {
        ordinal: decoder.i32()?,
        expected_name: decoder.text()?,
        actual_name: decoder.optional_text()?,
        oid: decoder.optional_i64()?,
    };
    decoder.finish()?;
    Ok(value)
}

fn decode_membership(
    row: RawRow,
    budget: &mut LogicalPayloadBudget,
) -> Result<MembershipRow, SimulationRlsMetadataError> {
    let mut decoder = RawDecoder::new(row, RawCategory::Membership, &MEMBERSHIP_SHAPE, budget)?;
    let value = MembershipRow {
        member_oid: decoder.i64()?,
        role_oid: decoder.i64()?,
        grantor_oid: decoder.i64()?,
    };
    let _admin = decoder.boolean()?;
    let _inherit = decoder.boolean()?;
    let _set = decoder.boolean()?;
    decoder.finish()?;
    Ok(value)
}

fn decode_relation(
    row: RawRow,
    budget: &mut LogicalPayloadBudget,
) -> Result<RelationRow, SimulationRlsMetadataError> {
    let mut decoder = RawDecoder::new(row, RawCategory::Relation, &RELATION_SHAPE, budget)?;
    let value = RelationRow {
        schema_oid: decoder.i64()?,
        schema_name: decoder.text()?,
        name: decoder.text()?,
        oid: decoder.i64()?,
        owner_oid: decoder.i64()?,
        owner_name: decoder.optional_text()?,
        kind: decoder.text()?,
        persistence: decoder.text()?,
        is_partition: decoder.boolean()?,
        row_security_enabled: decoder.boolean()?,
        row_security_forced: decoder.boolean()?,
    };
    decoder.finish()?;
    Ok(value)
}

fn decode_inheritance(
    row: RawRow,
    budget: &mut LogicalPayloadBudget,
) -> Result<InheritanceRow, SimulationRlsMetadataError> {
    let mut decoder = RawDecoder::new(row, RawCategory::Inheritance, &INHERITANCE_SHAPE, budget)?;
    let value = InheritanceRow {
        child_oid: decoder.i64()?,
        parent_oid: decoder.i64()?,
    };
    decoder.finish()?;
    Ok(value)
}

fn decode_policy_header(
    row: RawRow,
    budget: &mut LogicalPayloadBudget,
) -> Result<PolicyHeader, SimulationRlsMetadataError> {
    let mut decoder =
        RawDecoder::new(row, RawCategory::PolicyHeader, &POLICY_HEADER_SHAPE, budget)?;
    let value = PolicyHeader {
        oid: positive_oid(decoder.i64()?)?,
        relation_oid: positive_oid(decoder.i64()?)?,
        name: decoder.text()?,
        command: decoder.text()?,
        permissive: decoder.boolean()?,
        roles_is_null: decoder.boolean()?,
        roles_dimensions: decoder.optional_i32()?,
        roles_lower_bound: decoder.optional_i32()?,
        roles_length: decoder.optional_i32()?,
        using_is_null: decoder.boolean()?,
        with_check_is_null: decoder.boolean()?,
    };
    decoder.finish()?;
    Ok(value)
}

fn decode_policy_role(
    row: RawRow,
    budget: &mut LogicalPayloadBudget,
) -> Result<PolicyRoleRow, SimulationRlsMetadataError> {
    let mut decoder = RawDecoder::new(row, RawCategory::PolicyRole, &POLICY_ROLE_SHAPE, budget)?;
    let value = PolicyRoleRow {
        policy_oid: positive_oid(decoder.i64()?)?,
        relation_oid: positive_oid(decoder.i64()?)?,
        ordinal: decoder.i64()?,
        role_oid: positive_oid(decoder.i64()?)?,
    };
    decoder.finish()?;
    Ok(value)
}

fn positive_oid(value: i64) -> Result<u32, SimulationRlsMetadataError> {
    u32::try_from(value)
        .ok()
        .filter(|oid| *oid > 0)
        .ok_or_else(|| SimulationRlsMetadataError::primary(PrimaryFailure::CatalogInvalid))
}

fn check_row_count(
    count: usize,
    accept_limit: usize,
    fetch_limit: usize,
    exact: Option<usize>,
) -> Result<(), SimulationRlsMetadataError> {
    if count > fetch_limit || count > accept_limit {
        return Err(SimulationRlsMetadataError::primary(
            PrimaryFailure::BoundExceeded,
        ));
    }
    if exact.is_some_and(|expected| count != expected) {
        return Err(SimulationRlsMetadataError::primary(
            PrimaryFailure::CatalogInvalid,
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)] // The query shape and three row bounds are independent.
fn decode_many<T>(
    rows: Vec<RawRow>,
    category: RawCategory,
    shape: &[PgCellKind],
    accept_limit: usize,
    fetch_limit: usize,
    exact: Option<usize>,
    budget: &mut LogicalPayloadBudget,
    mut decode: impl FnMut(RawRow, &mut LogicalPayloadBudget) -> Result<T, SimulationRlsMetadataError>,
) -> Result<Vec<T>, SimulationRlsMetadataError> {
    if rows.len() > fetch_limit {
        return Err(SimulationRlsMetadataError::primary(
            PrimaryFailure::BoundExceeded,
        ));
    }
    let mut decoded = Vec::with_capacity(rows.len());
    for row in rows {
        if row.category != category || row.cells.len() != shape.len() {
            budget.charge_row(&row, shape)?;
            return Err(SimulationRlsMetadataError::primary(
                PrimaryFailure::CatalogInvalid,
            ));
        }
        decoded.push(decode(row, budget)?);
    }
    check_row_count(decoded.len(), accept_limit, fetch_limit, exact)?;
    Ok(decoded)
}

fn decode_one_context(
    rows: Vec<RawRow>,
    budget: &mut LogicalPayloadBudget,
) -> Result<ContextEvidence, SimulationRlsMetadataError> {
    let contexts = decode_many(
        rows,
        RawCategory::Context,
        &CONTEXT_SHAPE,
        CONTEXT_ACCEPT_LIMIT,
        CONTEXT_FETCH_LIMIT,
        Some(1),
        budget,
        decode_context,
    )?;
    validate_context(
        contexts
            .into_iter()
            .next()
            .ok_or_else(|| SimulationRlsMetadataError::primary(PrimaryFailure::CatalogInvalid))?,
    )
}

fn validate_context(row: ContextRow) -> Result<ContextEvidence, SimulationRlsMetadataError> {
    let worker_oid = positive_oid(row.worker_oid)?;
    let database_oid = positive_oid(row.database_oid)?;
    let schema_oid = positive_oid(row.schema_oid)?;
    let migration_owner_oid = positive_oid(row.migration_owner_oid)?;
    let database_owner_oid = positive_oid(row.database_owner_oid)?;
    let schema_owner_oid = positive_oid(row.schema_owner_oid)?;
    if row.current_user != WORKER_ROLE
        || row.session_user != WORKER_ROLE
        || row.observed_database_name != row.database_name
        || row.schema_name != PUBLIC_SCHEMA
        || database_owner_oid == worker_oid
        || schema_owner_oid == worker_oid
        || migration_owner_oid == worker_oid
    {
        return Err(SimulationRlsMetadataError::primary(
            PrimaryFailure::PolicyRejected,
        ));
    }
    if !row.worker_can_login
        || row.worker_is_superuser
        || row.worker_can_create_database
        || row.worker_can_create_role
        || row.worker_can_replicate
        || row.worker_bypasses_rls
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
        || row.database_raw_grant_option
        || row.schema_raw_grant_option
    {
        return Err(SimulationRlsMetadataError::primary(
            PrimaryFailure::PolicyRejected,
        ));
    }
    Ok(ContextEvidence {
        snapshot: row,
        database_oid,
        worker_oid,
        schema_oid,
        migration_owner_oid,
        database_owner_oid,
        schema_owner_oid,
    })
}

fn validate_roles(
    rows: Vec<RawRow>,
    context: &ContextEvidence,
    budget: &mut LogicalPayloadBudget,
) -> Result<HashMap<String, u32>, SimulationRlsMetadataError> {
    let rows = decode_many(
        rows,
        RawCategory::Role,
        &ROLE_SHAPE,
        ROLE_ACCEPT_LIMIT,
        ROLE_FETCH_LIMIT,
        Some(ROLE_ACCEPT_LIMIT),
        budget,
        decode_role,
    )?;
    let mut by_name = HashMap::with_capacity(ROLE_NAMES.len());
    let mut unique_oids = HashSet::with_capacity(ROLE_NAMES.len());
    for (index, row) in rows.into_iter().enumerate() {
        let expected = ROLE_NAMES[index];
        let oid =
            positive_oid(row.oid.ok_or_else(|| {
                SimulationRlsMetadataError::primary(PrimaryFailure::CatalogInvalid)
            })?)?;
        if row.ordinal != (index + 1) as i32
            || row.expected_name != expected
            || row.actual_name.as_deref() != Some(expected)
            || !unique_oids.insert(oid)
        {
            return Err(SimulationRlsMetadataError::primary(
                PrimaryFailure::PolicyRejected,
            ));
        }
        by_name.insert(expected.to_owned(), oid);
    }
    if by_name.get(WORKER_ROLE) != Some(&context.worker_oid)
        || by_name.get(MIGRATION_OWNER_ROLE) != Some(&context.migration_owner_oid)
    {
        return Err(SimulationRlsMetadataError::primary(
            PrimaryFailure::PolicyRejected,
        ));
    }
    Ok(by_name)
}

fn validate_memberships(
    rows: Vec<RawRow>,
    worker_oid: u32,
    budget: &mut LogicalPayloadBudget,
) -> Result<(), SimulationRlsMetadataError> {
    if rows.len() > MEMBERSHIP_FETCH_LIMIT {
        return Err(SimulationRlsMetadataError::primary(
            PrimaryFailure::BoundExceeded,
        ));
    }
    let mut decoded = Vec::with_capacity(rows.len());
    for row in rows {
        decoded.push(decode_membership(row, budget)?);
    }
    check_row_count(
        decoded.len(),
        MEMBERSHIP_FETCH_LIMIT,
        MEMBERSHIP_FETCH_LIMIT,
        None,
    )?;
    for membership in &decoded {
        let member_oid = positive_oid(membership.member_oid)?;
        let _role_oid = positive_oid(membership.role_oid)?;
        let _grantor_oid = positive_oid(membership.grantor_oid)?;
        if member_oid != worker_oid {
            return Err(SimulationRlsMetadataError::primary(
                PrimaryFailure::CatalogInvalid,
            ));
        }
    }
    if decoded.len() > MEMBERSHIP_ACCEPT_LIMIT {
        Err(SimulationRlsMetadataError::primary(
            PrimaryFailure::PolicyRejected,
        ))
    } else {
        check_row_count(
            decoded.len(),
            MEMBERSHIP_ACCEPT_LIMIT,
            MEMBERSHIP_FETCH_LIMIT,
            Some(MEMBERSHIP_ACCEPT_LIMIT),
        )
    }
}

fn validate_relations(
    rows: Vec<RawRow>,
    context: &ContextEvidence,
    roles: &HashMap<String, u32>,
    budget: &mut LogicalPayloadBudget,
) -> Result<[u32; 2], SimulationRlsMetadataError> {
    let rows = decode_many(
        rows,
        RawCategory::Relation,
        &RELATION_SHAPE,
        RELATION_ACCEPT_LIMIT,
        RELATION_FETCH_LIMIT,
        Some(RELATION_ACCEPT_LIMIT),
        budget,
        decode_relation,
    )?;
    let migration_owner_oid = role_oid(roles, MIGRATION_OWNER_ROLE)?;
    let mut relation_oids = [0_u32; 2];
    let mut seen_oids = HashSet::with_capacity(RELATION_ACCEPT_LIMIT);
    let mut seen_names = HashSet::with_capacity(RELATION_ACCEPT_LIMIT);
    for row in rows {
        let schema_oid = positive_oid(row.schema_oid)?;
        let relation_oid = positive_oid(row.oid)?;
        let owner_oid = positive_oid(row.owner_oid)?;
        let spec = RELATION_SPECS
            .iter()
            .find(|spec| spec.name == row.name)
            .ok_or_else(|| SimulationRlsMetadataError::primary(PrimaryFailure::PolicyRejected))?;
        if !seen_names.insert((row.schema_name.clone(), row.name.clone()))
            || !seen_oids.insert(relation_oid)
            || schema_oid != context.schema_oid
            || row.schema_name != PUBLIC_SCHEMA
            || owner_oid != migration_owner_oid
            || row.owner_name.as_deref() != Some(MIGRATION_OWNER_ROLE)
            || row.kind != "r"
            || row.persistence != "p"
            || row.is_partition
            || !row.row_security_enabled
            || !row.row_security_forced
        {
            return Err(SimulationRlsMetadataError::primary(
                PrimaryFailure::PolicyRejected,
            ));
        }
        relation_oids[spec.slot] = relation_oid;
    }
    if relation_oids.contains(&0) || seen_names.len() != RELATION_ACCEPT_LIMIT {
        return Err(SimulationRlsMetadataError::primary(
            PrimaryFailure::CatalogInvalid,
        ));
    }
    Ok(relation_oids)
}

fn validate_inheritance(
    rows: Vec<RawRow>,
    relation_oids: [u32; 2],
    budget: &mut LogicalPayloadBudget,
) -> Result<(), SimulationRlsMetadataError> {
    if rows.len() > INHERITANCE_FETCH_LIMIT {
        return Err(SimulationRlsMetadataError::primary(
            PrimaryFailure::BoundExceeded,
        ));
    }
    let mut decoded = Vec::with_capacity(rows.len());
    for row in rows {
        decoded.push(decode_inheritance(row, budget)?);
    }
    for edge in &decoded {
        let child_oid = positive_oid(edge.child_oid)?;
        let parent_oid = positive_oid(edge.parent_oid)?;
        if !relation_oids.contains(&child_oid) && !relation_oids.contains(&parent_oid) {
            return Err(SimulationRlsMetadataError::primary(
                PrimaryFailure::CatalogInvalid,
            ));
        }
    }
    if decoded.len() > INHERITANCE_ACCEPT_LIMIT {
        Err(SimulationRlsMetadataError::primary(
            PrimaryFailure::PolicyRejected,
        ))
    } else {
        check_row_count(
            decoded.len(),
            INHERITANCE_ACCEPT_LIMIT,
            INHERITANCE_FETCH_LIMIT,
            Some(INHERITANCE_ACCEPT_LIMIT),
        )
    }
}

fn relation_slot(relation_oid: u32, relation_oids: [u32; 2]) -> Option<usize> {
    relation_oids.iter().position(|oid| *oid == relation_oid)
}

fn policy_spec(identity: PolicyIdentity<'_>) -> Option<&'static PolicySpec> {
    POLICY_SPECS
        .iter()
        .find(|spec| spec.relation_slot == identity.relation_slot && spec.name == identity.name)
}

fn validate_policy_headers(
    rows: Vec<RawRow>,
    relation_oids: [u32; 2],
    budget: &mut LogicalPayloadBudget,
) -> Result<Vec<PolicyHeader>, SimulationRlsMetadataError> {
    let headers = decode_many(
        rows,
        RawCategory::PolicyHeader,
        &POLICY_HEADER_SHAPE,
        POLICY_ACCEPT_LIMIT,
        POLICY_FETCH_LIMIT,
        Some(POLICY_ACCEPT_LIMIT),
        budget,
        decode_policy_header,
    )?;
    let mut seen_oids = HashSet::with_capacity(POLICY_ACCEPT_LIMIT);
    let mut seen_identities = HashSet::with_capacity(POLICY_ACCEPT_LIMIT);
    for header in &headers {
        let slot = relation_slot(header.relation_oid, relation_oids)
            .ok_or_else(|| SimulationRlsMetadataError::primary(PrimaryFailure::CatalogInvalid))?;
        let spec = policy_spec(PolicyIdentity {
            relation_slot: slot,
            name: &header.name,
        })
        .ok_or_else(|| SimulationRlsMetadataError::primary(PrimaryFailure::PolicyRejected))?;
        if !seen_oids.insert(header.oid)
            || !seen_identities.insert((header.relation_oid, header.name.clone()))
            || header.command != spec.command
            || header.permissive != spec.permissive
            || header.using_is_null != spec.using_is_null
            || header.with_check_is_null != spec.with_check_is_null
        {
            return Err(SimulationRlsMetadataError::primary(
                PrimaryFailure::PolicyRejected,
            ));
        }
        if header.roles_is_null
            || header.roles_dimensions != Some(1)
            || header.roles_lower_bound != Some(1)
            || header.roles_length != Some(1)
        {
            return Err(SimulationRlsMetadataError::primary(
                PrimaryFailure::PolicyRejected,
            ));
        }
    }
    if seen_identities.len() != POLICY_ACCEPT_LIMIT {
        return Err(SimulationRlsMetadataError::primary(
            PrimaryFailure::CatalogInvalid,
        ));
    }
    Ok(headers)
}

fn validate_policy_roles(
    rows: Vec<RawRow>,
    headers: &[PolicyHeader],
    relation_oids: [u32; 2],
    roles: &HashMap<String, u32>,
    budget: &mut LogicalPayloadBudget,
) -> Result<(), SimulationRlsMetadataError> {
    let role_rows = decode_many(
        rows,
        RawCategory::PolicyRole,
        &POLICY_ROLE_SHAPE,
        POLICY_ROLE_ACCEPT_LIMIT,
        POLICY_ROLE_FETCH_LIMIT,
        Some(POLICY_ROLE_ACCEPT_LIMIT),
        budget,
        decode_policy_role,
    )?;
    let mut headers_by_identity = HashMap::with_capacity(headers.len());
    for header in headers {
        if headers_by_identity
            .insert((header.oid, header.relation_oid), header)
            .is_some()
        {
            return Err(SimulationRlsMetadataError::primary(
                PrimaryFailure::PolicyRejected,
            ));
        }
    }
    let mut seen = HashSet::with_capacity(POLICY_ROLE_ACCEPT_LIMIT);
    let mut observed_roles = HashMap::with_capacity(POLICY_ROLE_ACCEPT_LIMIT);
    for role_row in role_rows {
        let header = headers_by_identity
            .get(&(role_row.policy_oid, role_row.relation_oid))
            .ok_or_else(|| SimulationRlsMetadataError::primary(PrimaryFailure::PolicyRejected))?;
        if relation_slot(role_row.relation_oid, relation_oids).is_none()
            || role_row.ordinal != 1
            || !seen.insert((role_row.policy_oid, role_row.relation_oid, role_row.ordinal))
            || observed_roles
                .insert(
                    (role_row.policy_oid, role_row.relation_oid),
                    role_row.role_oid,
                )
                .is_some()
        {
            return Err(SimulationRlsMetadataError::primary(
                PrimaryFailure::PolicyRejected,
            ));
        }
        let _ = header;
    }
    if observed_roles.len() != POLICY_ROLE_ACCEPT_LIMIT {
        return Err(SimulationRlsMetadataError::primary(
            PrimaryFailure::PolicyRejected,
        ));
    }
    for header in headers {
        let slot = relation_slot(header.relation_oid, relation_oids)
            .ok_or_else(|| SimulationRlsMetadataError::primary(PrimaryFailure::CatalogInvalid))?;
        let spec = policy_spec(PolicyIdentity {
            relation_slot: slot,
            name: &header.name,
        })
        .ok_or_else(|| SimulationRlsMetadataError::primary(PrimaryFailure::PolicyRejected))?;
        let expected_role_oid = role_oid(roles, spec.role)?;
        if observed_roles.get(&(header.oid, header.relation_oid)) != Some(&expected_role_oid) {
            return Err(SimulationRlsMetadataError::primary(
                PrimaryFailure::PolicyRejected,
            ));
        }
    }
    Ok(())
}

fn role_oid(roles: &HashMap<String, u32>, name: &str) -> Result<u32, SimulationRlsMetadataError> {
    roles
        .get(name)
        .copied()
        .ok_or_else(|| SimulationRlsMetadataError::primary(PrimaryFailure::CatalogInvalid))
}

async fn execute_checked<T: ObserverTransport>(
    transport: &mut T,
    statement: Statement,
    canceled: &AtomicBool,
    fallback: PrimaryFailure,
) -> Result<Vec<RawRow>, SimulationRlsMetadataError> {
    if is_canceled(canceled) {
        return Err(SimulationRlsMetadataError::primary(
            PrimaryFailure::Canceled,
        ));
    }
    match transport.execute(statement).await {
        Err(failure) => {
            let primary = match failure {
                TransportFailure::Database if fallback == PrimaryFailure::SetupFailed => {
                    PrimaryFailure::SetupFailed
                }
                TransportFailure::Database => PrimaryFailure::CatalogUnavailable,
                TransportFailure::State => PrimaryFailure::CatalogInvalid,
                TransportFailure::FetchBound => PrimaryFailure::BoundExceeded,
            };
            let mut error = SimulationRlsMetadataError::primary(primary);
            error.canceled = is_canceled(canceled);
            Err(error)
        }
        Ok(_rows) if is_canceled(canceled) => Err(SimulationRlsMetadataError::primary(
            PrimaryFailure::Canceled,
        )),
        Ok(rows) => Ok(rows),
    }
}

async fn fetch_checked<T: ObserverTransport>(
    transport: &mut T,
    statement: Statement,
    canceled: &AtomicBool,
    fallback: PrimaryFailure,
) -> Result<Vec<RawRow>, SimulationRlsMetadataError> {
    execute_checked(transport, statement, canceled, fallback).await
}

async fn prepare_observation<T: ObserverTransport>(
    transport: &mut T,
    canceled: &AtomicBool,
    timeout_ms: i32,
    budget: &mut LogicalPayloadBudget,
) -> Result<PreparedObservation, SimulationRlsMetadataError> {
    let setup = execute_checked(
        transport,
        Statement::Configure { timeout_ms },
        canceled,
        PrimaryFailure::SetupFailed,
    )
    .await?;
    if !setup.is_empty() {
        return Err(SimulationRlsMetadataError::primary(
            PrimaryFailure::SetupFailed,
        ));
    }

    let context_rows = fetch_checked(
        transport,
        Statement::Context,
        canceled,
        PrimaryFailure::CatalogUnavailable,
    )
    .await?;
    let context = decode_one_context(context_rows, budget)?;

    let role_rows = fetch_checked(
        transport,
        Statement::Roles,
        canceled,
        PrimaryFailure::CatalogUnavailable,
    )
    .await?;
    let roles = validate_roles(role_rows, &context, budget)?;

    let membership_rows = fetch_checked(
        transport,
        Statement::Memberships {
            worker_oid: context.worker_oid,
        },
        canceled,
        PrimaryFailure::CatalogUnavailable,
    )
    .await?;
    validate_memberships(membership_rows, context.worker_oid, budget)?;

    let relation_rows = fetch_checked(
        transport,
        Statement::Relations,
        canceled,
        PrimaryFailure::CatalogUnavailable,
    )
    .await?;
    let relation_oids = validate_relations(relation_rows, &context, &roles, budget)?;

    let inheritance_rows = fetch_checked(
        transport,
        Statement::Inheritance { relation_oids },
        canceled,
        PrimaryFailure::CatalogUnavailable,
    )
    .await?;
    validate_inheritance(inheritance_rows, relation_oids, budget)?;

    let policy_header_rows = fetch_checked(
        transport,
        Statement::PolicyHeaders { relation_oids },
        canceled,
        PrimaryFailure::CatalogUnavailable,
    )
    .await?;
    let policy_headers = validate_policy_headers(policy_header_rows, relation_oids, budget)?;

    let policy_role_rows = fetch_checked(
        transport,
        Statement::PolicyRoles { relation_oids },
        canceled,
        PrimaryFailure::CatalogUnavailable,
    )
    .await?;
    validate_policy_roles(
        policy_role_rows,
        &policy_headers,
        relation_oids,
        &roles,
        budget,
    )?;

    let final_context_rows = fetch_checked(
        transport,
        Statement::FinalContext,
        canceled,
        PrimaryFailure::CatalogUnavailable,
    )
    .await?;
    let final_context = decode_one_context(final_context_rows, budget)?;
    if final_context != context {
        return Err(SimulationRlsMetadataError::primary(
            PrimaryFailure::PolicyRejected,
        ));
    }
    Ok(PreparedObservation {
        context,
        relation_oids,
    })
}

fn is_canceled(canceled: &AtomicBool) -> bool {
    canceled.load(Ordering::Acquire)
}

fn timeout_millis(timeout_seconds: u64) -> Result<i32, SimulationRlsMetadataError> {
    let milliseconds = timeout_seconds
        .checked_mul(1000)
        .filter(|value| *value > 0 && *value <= i32::MAX as u64)
        .ok_or_else(|| SimulationRlsMetadataError::primary(PrimaryFailure::InvalidBound))?;
    i32::try_from(milliseconds)
        .map_err(|_| SimulationRlsMetadataError::primary(PrimaryFailure::InvalidBound))
}

async fn close_before_begin<T: ObserverTransport>(
    transport: &mut T,
    mut error: SimulationRlsMetadataError,
    canceled: &AtomicBool,
) -> SimulationRlsMetadataError {
    error.close = if transport.close().await.is_ok() {
        CleanupState::Confirmed
    } else {
        CleanupState::Unconfirmed
    };
    error.canceled |= is_canceled(canceled);
    error
}

async fn rollback_and_close<T: ObserverTransport>(
    transport: &mut T,
    mut error: SimulationRlsMetadataError,
    canceled: &AtomicBool,
) -> SimulationRlsMetadataError {
    error.rollback = if matches!(
        transport.execute(Statement::Rollback).await,
        Ok(rows) if rows.is_empty()
    ) {
        CleanupState::Confirmed
    } else {
        CleanupState::Unconfirmed
    };
    error.close = if transport.close().await.is_ok() {
        CleanupState::Confirmed
    } else {
        CleanupState::Unconfirmed
    };
    error.canceled |= is_canceled(canceled);
    error
}

async fn observe_with_transport<T: ObserverTransport>(
    transport: &mut T,
    config: &SimulationProcessConfig,
    canceled: &AtomicBool,
) -> Result<SimulationRlsMetadataObservation, SimulationRlsMetadataError> {
    let mut budget = LogicalPayloadBudget::default();
    observe_with_transport_and_budget(transport, config, canceled, &mut budget).await
}

async fn observe_with_transport_and_budget<T: ObserverTransport>(
    transport: &mut T,
    config: &SimulationProcessConfig,
    canceled: &AtomicBool,
    budget: &mut LogicalPayloadBudget,
) -> Result<SimulationRlsMetadataObservation, SimulationRlsMetadataError> {
    let timeout_ms = timeout_millis(config.limits().startup_timeout_seconds())?;
    if is_canceled(canceled) {
        return Err(SimulationRlsMetadataError::primary(
            PrimaryFailure::Canceled,
        ));
    }
    if transport.acquire().await.is_err() {
        let mut error = SimulationRlsMetadataError::primary(PrimaryFailure::AcquisitionFailed);
        error.canceled = is_canceled(canceled);
        return Err(error);
    }
    transport.arm_close_on_drop();
    if is_canceled(canceled) {
        return Err(close_before_begin(
            transport,
            SimulationRlsMetadataError::primary(PrimaryFailure::Canceled),
            canceled,
        )
        .await);
    }

    let begin = transport.execute(Statement::Begin).await;
    match begin {
        Ok(rows) if rows.is_empty() => {}
        Ok(_) => {
            return Err(rollback_and_close(
                transport,
                SimulationRlsMetadataError::primary(PrimaryFailure::BeginResultInvalid),
                canceled,
            )
            .await);
        }
        Err(_) => {
            return Err(rollback_and_close(
                transport,
                SimulationRlsMetadataError::primary(PrimaryFailure::BeginFailed),
                canceled,
            )
            .await);
        }
    }
    if is_canceled(canceled) {
        return Err(rollback_and_close(
            transport,
            SimulationRlsMetadataError::primary(PrimaryFailure::Canceled),
            canceled,
        )
        .await);
    }

    let prepared = match prepare_observation(transport, canceled, timeout_ms, budget).await {
        Ok(prepared) => prepared,
        Err(error) => return Err(rollback_and_close(transport, error, canceled).await),
    };
    if is_canceled(canceled) {
        return Err(rollback_and_close(
            transport,
            SimulationRlsMetadataError::primary(PrimaryFailure::Canceled),
            canceled,
        )
        .await);
    }

    let commit = transport.execute(Statement::Commit).await;
    let completion = if matches!(commit, Ok(rows) if rows.is_empty()) {
        CompletionState::Acknowledged
    } else {
        CompletionState::Unconfirmed
    };
    let canceled_during_commit = is_canceled(canceled);
    if completion == CompletionState::Unconfirmed {
        let close = if transport.close().await.is_ok() {
            CleanupState::Confirmed
        } else {
            CleanupState::Unconfirmed
        };
        return Err(SimulationRlsMetadataError {
            primary: PrimaryFailure::CompletionUnconfirmed,
            completion,
            rollback: CleanupState::NotApplicable,
            close,
            canceled: canceled_during_commit || is_canceled(canceled),
        });
    }

    let close = if transport.close().await.is_ok() {
        CleanupState::Confirmed
    } else {
        CleanupState::Unconfirmed
    };
    let canceled_after_close = is_canceled(canceled);
    if close == CleanupState::Unconfirmed {
        return Err(SimulationRlsMetadataError {
            primary: if canceled_during_commit || canceled_after_close {
                PrimaryFailure::Canceled
            } else {
                PrimaryFailure::CleanupUnconfirmed
            },
            completion,
            rollback: CleanupState::NotApplicable,
            close,
            canceled: canceled_during_commit || canceled_after_close,
        });
    }
    if canceled_during_commit || canceled_after_close {
        return Err(SimulationRlsMetadataError {
            primary: PrimaryFailure::Canceled,
            completion,
            rollback: CleanupState::NotApplicable,
            close,
            canceled: true,
        });
    }
    Ok(SimulationRlsMetadataObservation::completed(
        config.config_sha256(),
        &prepared.context,
        prepared.relation_oids,
    ))
}

/// Observe the adopted relation and policy metadata on one fresh disposable
/// connection. Dropping this future disposes an armed SQLx connection but is
/// not an awaited rollback or close receipt.
pub(super) async fn observe_simulation_rls_metadata(
    pool: &PgPool,
    config: &SimulationProcessConfig,
    canceled: &AtomicBool,
) -> Result<SimulationRlsMetadataObservation, SimulationRlsMetadataError> {
    let mut transport = SqlxObserverTransport::new(pool);
    observe_with_transport(&mut transport, config, canceled).await
}

#[cfg(test)]
mod tests;
