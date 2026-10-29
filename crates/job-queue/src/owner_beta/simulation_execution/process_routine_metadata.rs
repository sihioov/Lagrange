//! Private, bounded observation of the three adopted version-helper metadata
//! contracts. This does not inspect or prove any helper body or startup state.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use sqlx::{
    Column, PgConnection, PgPool, Postgres, Row, TypeInfo, pool::PoolConnection, postgres::PgRow,
};

use super::SimulationProcessConfig;

const POLICY_ID: &str = "r4-simulation-version-helper-metadata-v1";
const POLICY_FINGERPRINT: &str =
    "sha256:4b1cce9bed4f2b0273a418a137c432073c7e73003c1c24c0219978908ad28384";
const PUBLIC_SCHEMA: &str = "public";
const PG_CATALOG_SCHEMA: &str = "pg_catalog";
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

const CONTEXT_ACCEPT_LIMIT: usize = 1;
const CONTEXT_FETCH_LIMIT: usize = 2;
const ROLE_ACCEPT_LIMIT: usize = 6;
const ROLE_FETCH_LIMIT: usize = 7;
const MEMBERSHIP_ACCEPT_LIMIT: usize = 0;
const MEMBERSHIP_FETCH_LIMIT: usize = 1;
const ROUTINE_ACCEPT_LIMIT: usize = 3;
const ROUTINE_FETCH_LIMIT: usize = 4;
const ARGUMENT_ACCEPT_LIMIT: usize = 25;
const ARGUMENT_FETCH_LIMIT: usize = 26;
const ARGUMENTS_PER_ROUTINE_LIMIT: usize = 16;
const RAW_ACL_ACCEPT_LIMIT: usize = 64;
const RAW_ACL_FETCH_LIMIT: usize = 65;
const EFFECTIVE_ACCEPT_LIMIT: usize = 18;
const EFFECTIVE_FETCH_LIMIT: usize = 19;
const TEXT_ACCEPT_BYTES: usize = 1024;
const TEXT_FETCH_BYTES: usize = 1025;
const LOGICAL_PAYLOAD_LIMIT: usize = 65_536;

const BEGIN_SQL: &str = "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY";
const SETUP_SQL: &str = "\
    SELECT pg_catalog.set_config('search_path', 'pg_catalog', true), \
           pg_catalog.set_config('statement_timeout', $1::pg_catalog.text, true), \
           pg_catalog.set_config('lock_timeout', $1::pg_catalog.text, true)";

const CONTEXT_SQL: &str = r#"
SELECT pg_catalog.octet_length(current_user::pg_catalog.text)::pg_catalog.int8 AS current_user_bytes,
       CASE WHEN pg_catalog.octet_length(current_user::pg_catalog.text) <= 1025
            THEN current_user::pg_catalog.text
            ELSE pg_catalog.left(current_user::pg_catalog.text, 256)
       END::pg_catalog.text AS current_user,
       pg_catalog.octet_length(session_user::pg_catalog.text)::pg_catalog.int8 AS session_user_bytes,
       CASE WHEN pg_catalog.octet_length(session_user::pg_catalog.text) <= 1025
            THEN session_user::pg_catalog.text
            ELSE pg_catalog.left(session_user::pg_catalog.text, 256)
       END::pg_catalog.text AS session_user,
       pg_catalog.octet_length(pg_catalog.current_database()::pg_catalog.text)::pg_catalog.int8
           AS observed_database_name_bytes,
       CASE WHEN pg_catalog.octet_length(pg_catalog.current_database()::pg_catalog.text) <= 1025
            THEN pg_catalog.current_database()::pg_catalog.text
            ELSE pg_catalog.left(pg_catalog.current_database()::pg_catalog.text, 256)
       END::pg_catalog.text AS observed_database_name,
       pg_catalog.octet_length(database.datname::pg_catalog.text)::pg_catalog.int8 AS database_name_bytes,
       CASE WHEN pg_catalog.octet_length(database.datname::pg_catalog.text) <= 1025
            THEN database.datname::pg_catalog.text
            ELSE pg_catalog.left(database.datname::pg_catalog.text, 256)
       END::pg_catalog.text AS database_name,
       pg_catalog.octet_length(public_schema.nspname::pg_catalog.text)::pg_catalog.int8 AS schema_name_bytes,
       CASE WHEN pg_catalog.octet_length(public_schema.nspname::pg_catalog.text) <= 1025
            THEN public_schema.nspname::pg_catalog.text
            ELSE pg_catalog.left(public_schema.nspname::pg_catalog.text, 256)
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
       pg_catalog.octet_length(pg_catalog.current_setting('row_security')::pg_catalog.text)::pg_catalog.int8
           AS row_security_bytes,
       CASE WHEN pg_catalog.octet_length(pg_catalog.current_setting('row_security')::pg_catalog.text) <= 1025
            THEN pg_catalog.current_setting('row_security')::pg_catalog.text
            ELSE pg_catalog.left(pg_catalog.current_setting('row_security')::pg_catalog.text, 256)
       END::pg_catalog.text AS row_security,
       pg_catalog.octet_length(pg_catalog.current_setting('session_replication_role')::pg_catalog.text)::pg_catalog.int8
           AS session_replication_role_bytes,
       CASE WHEN pg_catalog.octet_length(pg_catalog.current_setting('session_replication_role')::pg_catalog.text) <= 1025
            THEN pg_catalog.current_setting('session_replication_role')::pg_catalog.text
            ELSE pg_catalog.left(pg_catalog.current_setting('session_replication_role')::pg_catalog.text, 256)
       END::pg_catalog.text AS session_replication_role,
       pg_catalog.current_setting('transaction_read_only')::pg_catalog.bool AS transaction_read_only,
       pg_catalog.octet_length(pg_catalog.current_setting('transaction_isolation')::pg_catalog.text)::pg_catalog.int8
           AS transaction_isolation_bytes,
       CASE WHEN pg_catalog.octet_length(pg_catalog.current_setting('transaction_isolation')::pg_catalog.text) <= 1025
            THEN pg_catalog.current_setting('transaction_isolation')::pg_catalog.text
            ELSE pg_catalog.left(pg_catalog.current_setting('transaction_isolation')::pg_catalog.text, 256)
       END::pg_catalog.text AS transaction_isolation,
       pg_catalog.has_database_privilege(current_user, database.oid, 'CONNECT')::pg_catalog.bool
           AS database_connect,
       pg_catalog.has_database_privilege(current_user, database.oid, 'CREATE')::pg_catalog.bool
           AS database_create,
       pg_catalog.has_schema_privilege(current_user, public_schema.oid, 'USAGE')::pg_catalog.bool
           AS schema_usage,
       pg_catalog.has_schema_privilege(current_user, public_schema.oid, 'CREATE')::pg_catalog.bool
           AS schema_create,
       (
           pg_catalog.has_database_privilege(current_user, database.oid, 'CONNECT WITH GRANT OPTION')
           OR pg_catalog.has_database_privilege(current_user, database.oid, 'CREATE WITH GRANT OPTION')
           OR pg_catalog.has_database_privilege(current_user, database.oid, 'TEMPORARY WITH GRANT OPTION')
       )::pg_catalog.bool AS database_effective_grant_option,
       (
           pg_catalog.has_schema_privilege(current_user, public_schema.oid, 'USAGE WITH GRANT OPTION')
           OR pg_catalog.has_schema_privilege(current_user, public_schema.oid, 'CREATE WITH GRANT OPTION')
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
                 COALESCE(public_schema.nspacl, pg_catalog.acldefault('n', public_schema.nspowner))
             ) AS acl
            WHERE acl.grantee IN (0::pg_catalog.oid, worker.oid)
              AND acl.is_grantable
              AND acl.privilege_type IN ('USAGE', 'CREATE')
       )::pg_catalog.bool AS schema_raw_grant_option
  FROM pg_catalog.pg_database AS database
  JOIN pg_catalog.pg_namespace AS public_schema ON public_schema.nspname = 'public'
  JOIN pg_catalog.pg_roles AS worker ON worker.rolname = current_user
  JOIN pg_catalog.pg_roles AS migration_owner ON migration_owner.rolname = 'migration_owner'
 WHERE database.datname = pg_catalog.current_database()
 LIMIT 2
"#;

const ROLES_SQL: &str = r#"
WITH expected_roles(ordinal, expected_name) AS (
    VALUES (1, 'worker'::pg_catalog.text),
           (2, 'migration_owner'::pg_catalog.text),
           (3, 'app'::pg_catalog.text),
           (4, 'admin'::pg_catalog.text),
           (5, 'audit_writer'::pg_catalog.text),
           (6, 'research_writer'::pg_catalog.text)
)
SELECT expected_roles.ordinal::pg_catalog.int4 AS role_ordinal,
       pg_catalog.octet_length(expected_roles.expected_name)::pg_catalog.int8 AS expected_name_bytes,
       CASE WHEN pg_catalog.octet_length(expected_roles.expected_name) <= 1025
            THEN expected_roles.expected_name
            ELSE pg_catalog.left(expected_roles.expected_name, 256)
       END::pg_catalog.text AS expected_name,
       pg_catalog.octet_length(actual_role.rolname::pg_catalog.text)::pg_catalog.int8 AS actual_name_bytes,
       CASE WHEN actual_role.rolname IS NULL THEN NULL
            WHEN pg_catalog.octet_length(actual_role.rolname::pg_catalog.text) <= 1025
            THEN actual_role.rolname::pg_catalog.text
            ELSE pg_catalog.left(actual_role.rolname::pg_catalog.text, 256)
       END::pg_catalog.text AS actual_name,
       actual_role.oid::pg_catalog.int8 AS role_oid
  FROM expected_roles
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

const ROUTINES_SQL: &str = r#"
SELECT pg_catalog.octet_length(routine_name::pg_catalog.text)::pg_catalog.int8 AS routine_name_bytes,
       CASE WHEN pg_catalog.octet_length(routine_name::pg_catalog.text) <= 1025
            THEN routine_name::pg_catalog.text
            ELSE pg_catalog.left(routine_name::pg_catalog.text, 256)
       END::pg_catalog.text AS routine_name,
       routine_oid::pg_catalog.int8 AS routine_oid,
       schema_oid::pg_catalog.int8 AS schema_oid,
       pg_catalog.octet_length(schema_name::pg_catalog.text)::pg_catalog.int8 AS schema_name_bytes,
       CASE WHEN pg_catalog.octet_length(schema_name::pg_catalog.text) <= 1025
            THEN schema_name::pg_catalog.text
            ELSE pg_catalog.left(schema_name::pg_catalog.text, 256)
       END::pg_catalog.text AS schema_name,
       owner_oid::pg_catalog.int8 AS owner_oid,
       pg_catalog.octet_length(owner_name::pg_catalog.text)::pg_catalog.int8 AS owner_name_bytes,
       CASE WHEN pg_catalog.octet_length(owner_name::pg_catalog.text) <= 1025
            THEN owner_name::pg_catalog.text
            ELSE pg_catalog.left(owner_name::pg_catalog.text, 256)
       END::pg_catalog.text AS owner_name,
       pg_catalog.octet_length(routine_kind::pg_catalog.text)::pg_catalog.int8 AS kind_bytes,
       CASE WHEN pg_catalog.octet_length(routine_kind::pg_catalog.text) <= 1025
            THEN routine_kind::pg_catalog.text
            ELSE pg_catalog.left(routine_kind::pg_catalog.text, 256)
       END::pg_catalog.text AS routine_kind,
       language_oid::pg_catalog.int8 AS language_oid,
       pg_catalog.octet_length(language_name::pg_catalog.text)::pg_catalog.int8 AS language_name_bytes,
       CASE WHEN pg_catalog.octet_length(language_name::pg_catalog.text) <= 1025
            THEN language_name::pg_catalog.text
            ELSE pg_catalog.left(language_name::pg_catalog.text, 256)
       END::pg_catalog.text AS language_name,
       return_type_oid::pg_catalog.int8 AS return_type_oid,
       return_namespace_oid::pg_catalog.int8 AS return_namespace_oid,
       pg_catalog.octet_length(return_namespace::pg_catalog.text)::pg_catalog.int8
           AS return_namespace_bytes,
       CASE WHEN pg_catalog.octet_length(return_namespace::pg_catalog.text) <= 1025
            THEN return_namespace::pg_catalog.text
            ELSE pg_catalog.left(return_namespace::pg_catalog.text, 256)
       END::pg_catalog.text AS return_namespace,
       pg_catalog.octet_length(return_type_name::pg_catalog.text)::pg_catalog.int8 AS return_type_name_bytes,
       CASE WHEN pg_catalog.octet_length(return_type_name::pg_catalog.text) <= 1025
            THEN return_type_name::pg_catalog.text
            ELSE pg_catalog.left(return_type_name::pg_catalog.text, 256)
       END::pg_catalog.text AS return_type_name,
       returns_set::pg_catalog.bool AS returns_set,
       pg_catalog.octet_length(volatility::pg_catalog.text)::pg_catalog.int8 AS volatility_bytes,
       CASE WHEN pg_catalog.octet_length(volatility::pg_catalog.text) <= 1025
            THEN volatility::pg_catalog.text
            ELSE pg_catalog.left(volatility::pg_catalog.text, 256)
       END::pg_catalog.text AS volatility,
       is_strict::pg_catalog.bool AS is_strict,
       security_definer::pg_catalog.bool AS security_definer,
       is_leakproof::pg_catalog.bool AS is_leakproof,
       pg_catalog.octet_length(parallel_mode::pg_catalog.text)::pg_catalog.int8 AS parallel_mode_bytes,
       CASE WHEN pg_catalog.octet_length(parallel_mode::pg_catalog.text) <= 1025
            THEN parallel_mode::pg_catalog.text
            ELSE pg_catalog.left(parallel_mode::pg_catalog.text, 256)
       END::pg_catalog.text AS parallel_mode,
       argument_count::pg_catalog.int4 AS argument_count,
       argument_defaults::pg_catalog.int4 AS argument_defaults,
       variadic_oid::pg_catalog.int8 AS variadic_oid,
       all_arg_types_null::pg_catalog.bool AS all_arg_types_null,
       arg_modes_null::pg_catalog.bool AS arg_modes_null,
       arg_defaults_null::pg_catalog.bool AS arg_defaults_null,
       arg_names_dimensions::pg_catalog.int4 AS arg_names_dimensions,
       arg_names_lower::pg_catalog.int4 AS arg_names_lower,
       arg_names_length::pg_catalog.int4 AS arg_names_length,
       config_dimensions::pg_catalog.int4 AS config_dimensions,
       config_lower::pg_catalog.int4 AS config_lower,
       config_length::pg_catalog.int4 AS config_length,
       pg_catalog.octet_length(config_first::pg_catalog.text)::pg_catalog.int8 AS config_first_bytes,
       CASE WHEN config_first IS NULL THEN NULL
            WHEN pg_catalog.octet_length(config_first::pg_catalog.text) <= 1025
            THEN config_first::pg_catalog.text
            ELSE pg_catalog.left(config_first::pg_catalog.text, 256)
       END::pg_catalog.text AS config_first
  FROM (
       SELECT p.proname AS routine_name,
              p.oid AS routine_oid,
              namespace.oid AS schema_oid,
              namespace.nspname AS schema_name,
              p.proowner AS owner_oid,
              owner_role.rolname AS owner_name,
              p.prokind AS routine_kind,
              p.prolang AS language_oid,
              language.lanname AS language_name,
              p.prorettype AS return_type_oid,
              return_namespace.oid AS return_namespace_oid,
              return_namespace.nspname AS return_namespace,
              return_type.typname AS return_type_name,
              p.proretset AS returns_set,
              p.provolatile AS volatility,
              p.proisstrict AS is_strict,
              p.prosecdef AS security_definer,
              p.proleakproof AS is_leakproof,
              p.proparallel AS parallel_mode,
              p.pronargs AS argument_count,
              p.pronargdefaults AS argument_defaults,
              p.provariadic AS variadic_oid,
              p.proallargtypes IS NULL AS all_arg_types_null,
              p.proargmodes IS NULL AS arg_modes_null,
              p.proargdefaults IS NULL AS arg_defaults_null,
              pg_catalog.array_ndims(p.proargnames) AS arg_names_dimensions,
              pg_catalog.array_lower(p.proargnames, 1) AS arg_names_lower,
              pg_catalog.array_length(p.proargnames, 1) AS arg_names_length,
              pg_catalog.array_ndims(p.proconfig) AS config_dimensions,
              pg_catalog.array_lower(p.proconfig, 1) AS config_lower,
              pg_catalog.array_length(p.proconfig, 1) AS config_length,
              (p.proconfig)[1] AS config_first
         FROM pg_catalog.pg_proc AS p
         JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = p.pronamespace
         LEFT JOIN pg_catalog.pg_roles AS owner_role ON owner_role.oid = p.proowner
         LEFT JOIN pg_catalog.pg_language AS language ON language.oid = p.prolang
         LEFT JOIN pg_catalog.pg_type AS return_type ON return_type.oid = p.prorettype
         LEFT JOIN pg_catalog.pg_namespace AS return_namespace
                ON return_namespace.oid = return_type.typnamespace
        WHERE namespace.nspname = 'public'
          AND p.proname IN (
              'owner_beta_simulation_payload_matches',
              'owner_beta_simulation_version_binding_matches',
              'owner_beta_simulation_version_namespace_matches'
          )
  ) AS routine_catalog
 ORDER BY CASE routine_name
            WHEN 'owner_beta_simulation_version_binding_matches' THEN 1
            WHEN 'owner_beta_simulation_version_namespace_matches' THEN 2
            WHEN 'owner_beta_simulation_payload_matches' THEN 3
            ELSE 4
          END,
          routine_oid
 LIMIT 4
"#;

const ARGUMENTS_SQL: &str = r#"
SELECT pg_catalog.octet_length(p.proname::pg_catalog.text)::pg_catalog.int8 AS routine_name_bytes,
       CASE WHEN pg_catalog.octet_length(p.proname::pg_catalog.text) <= 1025
            THEN p.proname::pg_catalog.text
            ELSE pg_catalog.left(p.proname::pg_catalog.text, 256)
       END::pg_catalog.text AS routine_name,
       p.oid::pg_catalog.int8 AS routine_oid,
       argument.ordinality::pg_catalog.int4 AS argument_ordinal,
       pg_catalog.octet_length(argument_name::pg_catalog.text)::pg_catalog.int8 AS argument_name_bytes,
       CASE WHEN argument_name IS NULL THEN NULL
            WHEN pg_catalog.octet_length(argument_name::pg_catalog.text) <= 1025
            THEN argument_name::pg_catalog.text
            ELSE pg_catalog.left(argument_name::pg_catalog.text, 256)
       END::pg_catalog.text AS argument_name,
       argument.type_oid::pg_catalog.int8 AS argument_type_oid,
       argument_type.oid::pg_catalog.int8 AS resolved_type_oid,
       argument_type_namespace.oid::pg_catalog.int8 AS type_namespace_oid,
       pg_catalog.octet_length(argument_type_namespace.nspname::pg_catalog.text)::pg_catalog.int8
           AS type_namespace_bytes,
       CASE WHEN argument_type_namespace.nspname IS NULL THEN NULL
            WHEN pg_catalog.octet_length(argument_type_namespace.nspname::pg_catalog.text) <= 1025
            THEN argument_type_namespace.nspname::pg_catalog.text
            ELSE pg_catalog.left(argument_type_namespace.nspname::pg_catalog.text, 256)
       END::pg_catalog.text AS type_namespace,
       pg_catalog.octet_length(argument_type.typname::pg_catalog.text)::pg_catalog.int8 AS type_name_bytes,
       CASE WHEN argument_type.typname IS NULL THEN NULL
            WHEN pg_catalog.octet_length(argument_type.typname::pg_catalog.text) <= 1025
            THEN argument_type.typname::pg_catalog.text
            ELSE pg_catalog.left(argument_type.typname::pg_catalog.text, 256)
       END::pg_catalog.text AS type_name
  FROM pg_catalog.pg_proc AS p
  JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = p.pronamespace
  CROSS JOIN LATERAL pg_catalog.unnest(p.proargtypes::pg_catalog.oid[])
       WITH ORDINALITY AS argument(type_oid, ordinality)
  LEFT JOIN LATERAL (
       SELECT (p.proargnames)[argument.ordinality::pg_catalog.int4] AS argument_name
  ) AS argument_name_row ON true
  LEFT JOIN pg_catalog.pg_type AS argument_type ON argument_type.oid = argument.type_oid
  LEFT JOIN pg_catalog.pg_namespace AS argument_type_namespace
         ON argument_type_namespace.oid = argument_type.typnamespace
 WHERE namespace.nspname = 'public'
   AND p.proname IN (
       'owner_beta_simulation_version_binding_matches',
       'owner_beta_simulation_version_namespace_matches',
       'owner_beta_simulation_payload_matches'
   )
 ORDER BY CASE p.proname
            WHEN 'owner_beta_simulation_version_binding_matches' THEN 1
            WHEN 'owner_beta_simulation_version_namespace_matches' THEN 2
            WHEN 'owner_beta_simulation_payload_matches' THEN 3
            ELSE 4
          END,
          p.oid, argument.ordinality
 LIMIT 26
"#;

const RAW_ACL_SQL: &str = r#"
SELECT pg_catalog.octet_length(p.proname::pg_catalog.text)::pg_catalog.int8 AS routine_name_bytes,
       CASE WHEN pg_catalog.octet_length(p.proname::pg_catalog.text) <= 1025
            THEN p.proname::pg_catalog.text
            ELSE pg_catalog.left(p.proname::pg_catalog.text, 256)
       END::pg_catalog.text AS routine_name,
       p.oid::pg_catalog.int8 AS routine_oid,
       acl.grantee::pg_catalog.int8 AS grantee_oid,
       pg_catalog.octet_length(grantee_role.rolname::pg_catalog.text)::pg_catalog.int8 AS grantee_name_bytes,
       CASE WHEN grantee_role.rolname IS NULL THEN NULL
            WHEN pg_catalog.octet_length(grantee_role.rolname::pg_catalog.text) <= 1025
            THEN grantee_role.rolname::pg_catalog.text
            ELSE pg_catalog.left(grantee_role.rolname::pg_catalog.text, 256)
       END::pg_catalog.text AS grantee_name,
       acl.grantor::pg_catalog.int8 AS grantor_oid,
       pg_catalog.octet_length(grantor_role.rolname::pg_catalog.text)::pg_catalog.int8 AS grantor_name_bytes,
       CASE WHEN grantor_role.rolname IS NULL THEN NULL
            WHEN pg_catalog.octet_length(grantor_role.rolname::pg_catalog.text) <= 1025
            THEN grantor_role.rolname::pg_catalog.text
            ELSE pg_catalog.left(grantor_role.rolname::pg_catalog.text, 256)
       END::pg_catalog.text AS grantor_name,
       pg_catalog.octet_length(acl.privilege_type::pg_catalog.text)::pg_catalog.int8 AS privilege_type_bytes,
       CASE WHEN pg_catalog.octet_length(acl.privilege_type::pg_catalog.text) <= 1025
            THEN acl.privilege_type::pg_catalog.text
            ELSE pg_catalog.left(acl.privilege_type::pg_catalog.text, 256)
       END::pg_catalog.text AS privilege_type,
       acl.is_grantable::pg_catalog.bool AS is_grantable
  FROM pg_catalog.pg_proc AS p
  JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = p.pronamespace
  CROSS JOIN LATERAL pg_catalog.aclexplode(
      COALESCE(p.proacl, pg_catalog.acldefault('f', p.proowner))
  ) AS acl
  LEFT JOIN pg_catalog.pg_roles AS grantee_role ON grantee_role.oid = acl.grantee
  LEFT JOIN pg_catalog.pg_roles AS grantor_role ON grantor_role.oid = acl.grantor
 WHERE namespace.nspname = 'public'
   AND p.proname IN (
       'owner_beta_simulation_payload_matches',
       'owner_beta_simulation_version_binding_matches',
       'owner_beta_simulation_version_namespace_matches'
   )
 ORDER BY p.proname, p.oid, acl.grantee, acl.grantor, acl.privilege_type, acl.is_grantable
 LIMIT 65
"#;

const EFFECTIVE_PRIVILEGES_SQL: &str = r#"
WITH expected_routines(ordinal, routine_oid) AS (
    VALUES (1, $1::pg_catalog.oid),
           (2, $2::pg_catalog.oid),
           (3, $3::pg_catalog.oid)
), expected_roles(ordinal, role_oid) AS (
    VALUES (1, $4::pg_catalog.oid),
           (2, $5::pg_catalog.oid),
           (3, $6::pg_catalog.oid),
           (4, $7::pg_catalog.oid),
           (5, $8::pg_catalog.oid),
           (6, $9::pg_catalog.oid)
)
SELECT pg_catalog.octet_length(routine.proname::pg_catalog.text)::pg_catalog.int8 AS routine_name_bytes,
       CASE WHEN pg_catalog.octet_length(routine.proname::pg_catalog.text) <= 1025
            THEN routine.proname::pg_catalog.text
            ELSE pg_catalog.left(routine.proname::pg_catalog.text, 256)
       END::pg_catalog.text AS routine_name,
       routine.oid::pg_catalog.int8 AS routine_oid,
       pg_catalog.octet_length(role.rolname::pg_catalog.text)::pg_catalog.int8 AS role_name_bytes,
       CASE WHEN pg_catalog.octet_length(role.rolname::pg_catalog.text) <= 1025
            THEN role.rolname::pg_catalog.text
            ELSE pg_catalog.left(role.rolname::pg_catalog.text, 256)
       END::pg_catalog.text AS role_name,
       role.oid::pg_catalog.int8 AS role_oid,
       pg_catalog.has_function_privilege(role.oid, routine.oid, 'EXECUTE')::pg_catalog.bool
           AS can_execute,
       pg_catalog.has_function_privilege(
           role.oid, routine.oid, 'EXECUTE WITH GRANT OPTION'
       )::pg_catalog.bool AS can_execute_with_grant_option
  FROM expected_routines
  JOIN pg_catalog.pg_proc AS routine ON routine.oid = expected_routines.routine_oid
  CROSS JOIN expected_roles
  JOIN pg_catalog.pg_roles AS role ON role.oid = expected_roles.role_oid
 ORDER BY expected_routines.ordinal, expected_roles.ordinal
 LIMIT 19
"#;

const FINAL_CONTEXT_SQL: &str = CONTEXT_SQL;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(super) enum SimulationVersionHelperMetadataError {
    #[error("simulation version-helper metadata observation has an invalid bound")]
    InvalidBound,
    #[error("simulation version-helper metadata observation was canceled")]
    Canceled,
    #[error("simulation version-helper metadata connection acquisition failed")]
    AcquisitionFailed,
    #[error("simulation version-helper metadata transaction could not be started")]
    BeginFailed,
    #[error("simulation version-helper metadata transaction start response was invalid")]
    BeginResultInvalid,
    #[error("simulation version-helper metadata transaction setup failed")]
    SetupFailed,
    #[error("simulation version-helper metadata catalog is unavailable")]
    CatalogUnavailable,
    #[error("simulation version-helper metadata catalog response is invalid")]
    CatalogInvalid,
    #[error("simulation version-helper metadata policy was rejected")]
    PolicyRejected,
    #[error("simulation version-helper metadata exceeded a fixed bound")]
    BoundExceeded,
    #[error("simulation version-helper metadata logical payload exceeded its fixed bound")]
    PayloadExceeded,
    #[error("simulation version-helper metadata completion is unconfirmed")]
    CompletionUnconfirmed,
    #[error("simulation version-helper metadata completion and cleanup are unconfirmed")]
    CompletionAndCleanupUnconfirmed,
    #[error("simulation version-helper metadata cleanup is unconfirmed")]
    CleanupUnconfirmed,
    #[error("simulation version-helper metadata rollback is unconfirmed after {primary:?}")]
    RollbackUnconfirmed { primary: PrimaryFailure },
    #[error("simulation version-helper metadata close is unconfirmed after {primary:?}")]
    CloseUnconfirmed { primary: PrimaryFailure },
    #[error(
        "simulation version-helper metadata rollback and close are unconfirmed after {primary:?}"
    )]
    RollbackAndCloseUnconfirmed { primary: PrimaryFailure },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PrimaryFailure {
    Canceled,
    BeginFailed,
    BeginResultInvalid,
    SetupFailed,
    CatalogUnavailable,
    CatalogInvalid,
    PolicyRejected,
    BoundExceeded,
    PayloadExceeded,
}

impl From<SimulationVersionHelperMetadataError> for PrimaryFailure {
    fn from(error: SimulationVersionHelperMetadataError) -> Self {
        match error {
            SimulationVersionHelperMetadataError::Canceled => Self::Canceled,
            SimulationVersionHelperMetadataError::BeginFailed => Self::BeginFailed,
            SimulationVersionHelperMetadataError::BeginResultInvalid => Self::BeginResultInvalid,
            SimulationVersionHelperMetadataError::SetupFailed => Self::SetupFailed,
            SimulationVersionHelperMetadataError::CatalogUnavailable => Self::CatalogUnavailable,
            SimulationVersionHelperMetadataError::PolicyRejected => Self::PolicyRejected,
            SimulationVersionHelperMetadataError::BoundExceeded => Self::BoundExceeded,
            SimulationVersionHelperMetadataError::PayloadExceeded => Self::PayloadExceeded,
            _ => Self::CatalogInvalid,
        }
    }
}

/// Opaque evidence from one acknowledged metadata-only observation.
#[allow(dead_code)] // Routine pins are retained after catalog validation.
pub(super) struct SimulationVersionHelperMetadataObservation {
    config_sha256: String,
    policy_id: &'static str,
    policy_fingerprint: &'static str,
    database_oid: u32,
    worker_oid: u32,
    public_schema_oid: u32,
    migration_owner_oid: u32,
    routine_oids: [u32; 3],
}

#[allow(dead_code)]
impl SimulationVersionHelperMetadataObservation {
    fn completed(config_sha256: &str, evidence: PreparedObservation) -> Self {
        Self {
            config_sha256: config_sha256.to_owned(),
            policy_id: POLICY_ID,
            policy_fingerprint: POLICY_FINGERPRINT,
            database_oid: evidence.context.database_oid,
            worker_oid: evidence.context.worker_oid,
            public_schema_oid: evidence.context.schema_oid,
            migration_owner_oid: evidence.context.migration_owner_oid,
            routine_oids: evidence.routine_oids,
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

    /// Routine order is payload, binding, namespace, matching the baseline.
    pub(super) fn routine_oids(&self) -> &[u32; 3] {
        &self.routine_oids
    }
}

#[derive(Clone, Copy)]
struct ExpectedArgument {
    name: &'static str,
    type_name: &'static str,
}

#[derive(Clone, Copy)]
struct RoutineSpec {
    name: &'static str,
    arguments: &'static [ExpectedArgument],
    serving_execute: &'static [&'static str],
}

const PAYLOAD_ARGUMENTS: [ExpectedArgument; 16] = [
    ExpectedArgument {
        name: "p_payload",
        type_name: "jsonb",
    },
    ExpectedArgument {
        name: "p_run_id",
        type_name: "uuid",
    },
    ExpectedArgument {
        name: "p_config_id",
        type_name: "uuid",
    },
    ExpectedArgument {
        name: "p_evaluation_start",
        type_name: "date",
    },
    ExpectedArgument {
        name: "p_evaluation_end",
        type_name: "date",
    },
    ExpectedArgument {
        name: "p_candidate_sha256",
        type_name: "text",
    },
    ExpectedArgument {
        name: "p_artifact_sha256",
        type_name: "text",
    },
    ExpectedArgument {
        name: "p_stage5_sha256",
        type_name: "text",
    },
    ExpectedArgument {
        name: "p_action_sha256",
        type_name: "text",
    },
    ExpectedArgument {
        name: "p_approval_sha256",
        type_name: "text",
    },
    ExpectedArgument {
        name: "p_contract_sha256",
        type_name: "text",
    },
    ExpectedArgument {
        name: "p_bars_sha256",
        type_name: "text",
    },
    ExpectedArgument {
        name: "p_strategy_id",
        type_name: "text",
    },
    ExpectedArgument {
        name: "p_strategy_version",
        type_name: "text",
    },
    ExpectedArgument {
        name: "p_strategy_config_json",
        type_name: "jsonb",
    },
    ExpectedArgument {
        name: "p_strategy_config_sha256",
        type_name: "text",
    },
];
const BINDING_ARGUMENTS: [ExpectedArgument; 6] = [
    ExpectedArgument {
        name: "p_schema",
        type_name: "text",
    },
    ExpectedArgument {
        name: "p_contract_sha256",
        type_name: "text",
    },
    ExpectedArgument {
        name: "p_strategy_id",
        type_name: "text",
    },
    ExpectedArgument {
        name: "p_strategy_version",
        type_name: "text",
    },
    ExpectedArgument {
        name: "p_config_json",
        type_name: "jsonb",
    },
    ExpectedArgument {
        name: "p_config_sha256",
        type_name: "text",
    },
];
const NAMESPACE_ARGUMENTS: [ExpectedArgument; 3] = [
    ExpectedArgument {
        name: "p_schema",
        type_name: "text",
    },
    ExpectedArgument {
        name: "p_contract_sha256",
        type_name: "text",
    },
    ExpectedArgument {
        name: "p_idempotency_key",
        type_name: "text",
    },
];
const ROUTINE_SPECS: [RoutineSpec; 3] = [
    RoutineSpec {
        name: "owner_beta_simulation_payload_matches",
        arguments: &PAYLOAD_ARGUMENTS,
        serving_execute: &[],
    },
    RoutineSpec {
        name: "owner_beta_simulation_version_binding_matches",
        arguments: &BINDING_ARGUMENTS,
        serving_execute: &["app", "worker"],
    },
    RoutineSpec {
        name: "owner_beta_simulation_version_namespace_matches",
        arguments: &NAMESPACE_ARGUMENTS,
        serving_execute: &[],
    },
];
const ROUTINE_QUERY_ORDER: [&str; 3] = [
    "owner_beta_simulation_version_binding_matches",
    "owner_beta_simulation_version_namespace_matches",
    "owner_beta_simulation_payload_matches",
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RawCategory {
    Context,
    Role,
    Membership,
    Routine,
    Argument,
    RawAcl,
    Effective,
}

#[derive(Clone, PartialEq, Eq, Debug)]
enum RawCell {
    Null,
    WrongType,
    Bool(bool),
    I32(i32),
    I64(i64),
    Text { value: String, original_bytes: i64 },
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
    fn charge(&mut self, cell: &RawCell) -> Result<(), SimulationVersionHelperMetadataError> {
        let bytes = match cell {
            RawCell::Text { value, .. } => value
                .len()
                .checked_add(8)
                .ok_or(SimulationVersionHelperMetadataError::PayloadExceeded)?,
            RawCell::Bool(_) | RawCell::Null | RawCell::WrongType => 1,
            RawCell::I32(_) => 4,
            RawCell::I64(_) => 8,
        };
        self.used = self
            .used
            .checked_add(bytes)
            .filter(|total| *total <= LOGICAL_PAYLOAD_LIMIT)
            .ok_or(SimulationVersionHelperMetadataError::PayloadExceeded)?;
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
        expected_cells: usize,
        budget: &'a mut LogicalPayloadBudget,
    ) -> Result<Self, SimulationVersionHelperMetadataError> {
        for cell in &row.cells {
            budget.charge(cell)?;
        }
        if row.category != category || row.cells.len() != expected_cells {
            return Err(SimulationVersionHelperMetadataError::CatalogInvalid);
        }
        Ok(Self {
            cells: row.cells.into_iter(),
            budget,
        })
    }

    fn take(&mut self) -> Result<RawCell, SimulationVersionHelperMetadataError> {
        self.cells
            .next()
            .ok_or(SimulationVersionHelperMetadataError::CatalogInvalid)
    }

    fn boolean(&mut self) -> Result<bool, SimulationVersionHelperMetadataError> {
        match self.take()? {
            RawCell::Bool(value) => Ok(value),
            RawCell::Null
            | RawCell::WrongType
            | RawCell::I32(_)
            | RawCell::I64(_)
            | RawCell::Text { .. } => Err(SimulationVersionHelperMetadataError::CatalogInvalid),
        }
    }

    fn i32(&mut self) -> Result<i32, SimulationVersionHelperMetadataError> {
        match self.take()? {
            RawCell::I32(value) => Ok(value),
            _ => Err(SimulationVersionHelperMetadataError::CatalogInvalid),
        }
    }

    fn optional_i32(&mut self) -> Result<Option<i32>, SimulationVersionHelperMetadataError> {
        match self.take()? {
            RawCell::I32(value) => Ok(Some(value)),
            RawCell::Null => Ok(None),
            _ => Err(SimulationVersionHelperMetadataError::CatalogInvalid),
        }
    }

    fn i64(&mut self) -> Result<i64, SimulationVersionHelperMetadataError> {
        match self.take()? {
            RawCell::I64(value) => Ok(value),
            _ => Err(SimulationVersionHelperMetadataError::CatalogInvalid),
        }
    }

    fn text(&mut self) -> Result<String, SimulationVersionHelperMetadataError> {
        self.text_inner(false)?
            .ok_or(SimulationVersionHelperMetadataError::CatalogInvalid)
    }

    fn optional_text(&mut self) -> Result<Option<String>, SimulationVersionHelperMetadataError> {
        self.text_inner(true)
    }

    fn text_inner(
        &mut self,
        nullable: bool,
    ) -> Result<Option<String>, SimulationVersionHelperMetadataError> {
        match self.take()? {
            RawCell::Text {
                value,
                original_bytes,
            } => {
                if original_bytes < 0
                    || value.len() > TEXT_FETCH_BYTES
                    || value.as_bytes().contains(&0)
                {
                    return Err(SimulationVersionHelperMetadataError::CatalogInvalid);
                }
                let length = usize::try_from(original_bytes)
                    .map_err(|_| SimulationVersionHelperMetadataError::CatalogInvalid)?;
                if length > TEXT_ACCEPT_BYTES {
                    return Err(SimulationVersionHelperMetadataError::BoundExceeded);
                }
                if length != value.len() || value.is_empty() {
                    return Err(SimulationVersionHelperMetadataError::CatalogInvalid);
                }
                Ok(Some(value))
            }
            RawCell::Null if nullable => Ok(None),
            _ => Err(SimulationVersionHelperMetadataError::CatalogInvalid),
        }
    }

    fn finish(self) -> Result<(), SimulationVersionHelperMetadataError> {
        let _ = self.budget.used;
        if self.cells.len() == 0 {
            Ok(())
        } else {
            Err(SimulationVersionHelperMetadataError::CatalogInvalid)
        }
    }
}

#[derive(Clone, Copy)]
enum PgCellKind {
    Bool(&'static str),
    I32(&'static str),
    OptionalI32(&'static str),
    I64(&'static str),
    Text {
        bytes_alias: &'static str,
        value_alias: &'static str,
    },
}

const CONTEXT_SHAPE: [PgCellKind; 29] = [
    PgCellKind::Text {
        bytes_alias: "current_user_bytes",
        value_alias: "current_user",
    },
    PgCellKind::Text {
        bytes_alias: "session_user_bytes",
        value_alias: "session_user",
    },
    PgCellKind::Text {
        bytes_alias: "observed_database_name_bytes",
        value_alias: "observed_database_name",
    },
    PgCellKind::Text {
        bytes_alias: "database_name_bytes",
        value_alias: "database_name",
    },
    PgCellKind::Text {
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
        bytes_alias: "row_security_bytes",
        value_alias: "row_security",
    },
    PgCellKind::Text {
        bytes_alias: "session_replication_role_bytes",
        value_alias: "session_replication_role",
    },
    PgCellKind::Bool("transaction_read_only"),
    PgCellKind::Text {
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
        bytes_alias: "expected_name_bytes",
        value_alias: "expected_name",
    },
    PgCellKind::Text {
        bytes_alias: "actual_name_bytes",
        value_alias: "actual_name",
    },
    PgCellKind::I64("role_oid"),
];
const MEMBERSHIP_SHAPE: [PgCellKind; 6] = [
    PgCellKind::I64("member_oid"),
    PgCellKind::I64("role_oid"),
    PgCellKind::I64("grantor_oid"),
    PgCellKind::Bool("admin_option"),
    PgCellKind::Bool("inherit_option"),
    PgCellKind::Bool("set_option"),
];
const ROUTINE_SHAPE: [PgCellKind; 32] = [
    PgCellKind::Text {
        bytes_alias: "routine_name_bytes",
        value_alias: "routine_name",
    },
    PgCellKind::I64("routine_oid"),
    PgCellKind::I64("schema_oid"),
    PgCellKind::Text {
        bytes_alias: "schema_name_bytes",
        value_alias: "schema_name",
    },
    PgCellKind::I64("owner_oid"),
    PgCellKind::Text {
        bytes_alias: "owner_name_bytes",
        value_alias: "owner_name",
    },
    PgCellKind::Text {
        bytes_alias: "kind_bytes",
        value_alias: "routine_kind",
    },
    PgCellKind::I64("language_oid"),
    PgCellKind::Text {
        bytes_alias: "language_name_bytes",
        value_alias: "language_name",
    },
    PgCellKind::I64("return_type_oid"),
    PgCellKind::I64("return_namespace_oid"),
    PgCellKind::Text {
        bytes_alias: "return_namespace_bytes",
        value_alias: "return_namespace",
    },
    PgCellKind::Text {
        bytes_alias: "return_type_name_bytes",
        value_alias: "return_type_name",
    },
    PgCellKind::Bool("returns_set"),
    PgCellKind::Text {
        bytes_alias: "volatility_bytes",
        value_alias: "volatility",
    },
    PgCellKind::Bool("is_strict"),
    PgCellKind::Bool("security_definer"),
    PgCellKind::Bool("is_leakproof"),
    PgCellKind::Text {
        bytes_alias: "parallel_mode_bytes",
        value_alias: "parallel_mode",
    },
    PgCellKind::I32("argument_count"),
    PgCellKind::I32("argument_defaults"),
    PgCellKind::I64("variadic_oid"),
    PgCellKind::Bool("all_arg_types_null"),
    PgCellKind::Bool("arg_modes_null"),
    PgCellKind::Bool("arg_defaults_null"),
    PgCellKind::OptionalI32("arg_names_dimensions"),
    PgCellKind::OptionalI32("arg_names_lower"),
    PgCellKind::OptionalI32("arg_names_length"),
    PgCellKind::OptionalI32("config_dimensions"),
    PgCellKind::OptionalI32("config_lower"),
    PgCellKind::OptionalI32("config_length"),
    PgCellKind::Text {
        bytes_alias: "config_first_bytes",
        value_alias: "config_first",
    },
];
const ARGUMENT_SHAPE: [PgCellKind; 9] = [
    PgCellKind::Text {
        bytes_alias: "routine_name_bytes",
        value_alias: "routine_name",
    },
    PgCellKind::I64("routine_oid"),
    PgCellKind::I32("argument_ordinal"),
    PgCellKind::Text {
        bytes_alias: "argument_name_bytes",
        value_alias: "argument_name",
    },
    PgCellKind::I64("argument_type_oid"),
    PgCellKind::I64("resolved_type_oid"),
    PgCellKind::I64("type_namespace_oid"),
    PgCellKind::Text {
        bytes_alias: "type_namespace_bytes",
        value_alias: "type_namespace",
    },
    PgCellKind::Text {
        bytes_alias: "type_name_bytes",
        value_alias: "type_name",
    },
];
const RAW_ACL_SHAPE: [PgCellKind; 8] = [
    PgCellKind::Text {
        bytes_alias: "routine_name_bytes",
        value_alias: "routine_name",
    },
    PgCellKind::I64("routine_oid"),
    PgCellKind::I64("grantee_oid"),
    PgCellKind::Text {
        bytes_alias: "grantee_name_bytes",
        value_alias: "grantee_name",
    },
    PgCellKind::I64("grantor_oid"),
    PgCellKind::Text {
        bytes_alias: "grantor_name_bytes",
        value_alias: "grantor_name",
    },
    PgCellKind::Text {
        bytes_alias: "privilege_type_bytes",
        value_alias: "privilege_type",
    },
    PgCellKind::Bool("is_grantable"),
];
const EFFECTIVE_SHAPE: [PgCellKind; 6] = [
    PgCellKind::Text {
        bytes_alias: "routine_name_bytes",
        value_alias: "routine_name",
    },
    PgCellKind::I64("routine_oid"),
    PgCellKind::Text {
        bytes_alias: "role_name_bytes",
        value_alias: "role_name",
    },
    PgCellKind::I64("role_oid"),
    PgCellKind::Bool("can_execute"),
    PgCellKind::Bool("can_execute_with_grant_option"),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Statement {
    Begin,
    Configure {
        timeout_ms: i32,
    },
    Context,
    Roles,
    Memberships {
        worker_oid: u32,
    },
    Routines,
    Arguments,
    RawAcl,
    Effective {
        routine_oids: [u32; 3],
        role_oids: [u32; 6],
    },
    FinalContext,
    Commit,
    Rollback,
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

fn membership_worker_bind_value(worker_oid: u32) -> i64 {
    i64::from(worker_oid)
}

fn effective_bind_values(routine_oids: [u32; 3], role_oids: [u32; 6]) -> [i64; 9] {
    [
        i64::from(routine_oids[0]),
        i64::from(routine_oids[1]),
        i64::from(routine_oids[2]),
        i64::from(role_oids[0]),
        i64::from(role_oids[1]),
        i64::from(role_oids[2]),
        i64::from(role_oids[3]),
        i64::from(role_oids[4]),
        i64::from(role_oids[5]),
    ]
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

    async fn fetch_effective(
        &mut self,
        routine_oids: [u32; 3],
        role_oids: [u32; 6],
    ) -> Result<Vec<RawRow>, TransportFailure> {
        let mut query = sqlx::query(EFFECTIVE_PRIVILEGES_SQL);
        for oid in effective_bind_values(routine_oids, role_oids) {
            query = query.bind(oid);
        }
        let rows = query
            .fetch_all(self.connection()?)
            .await
            .map_err(|_| TransportFailure::Database)?;
        raw_pg_rows(
            rows,
            RawCategory::Effective,
            &EFFECTIVE_SHAPE,
            EFFECTIVE_FETCH_LIMIT,
        )
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
                    .bind(membership_worker_bind_value(worker_oid))
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
            Statement::Routines => {
                self.fetch_rows(
                    ROUTINES_SQL,
                    RawCategory::Routine,
                    &ROUTINE_SHAPE,
                    ROUTINE_FETCH_LIMIT,
                )
                .await
            }
            Statement::Arguments => {
                self.fetch_rows(
                    ARGUMENTS_SQL,
                    RawCategory::Argument,
                    &ARGUMENT_SHAPE,
                    ARGUMENT_FETCH_LIMIT,
                )
                .await
            }
            Statement::RawAcl => {
                self.fetch_rows(
                    RAW_ACL_SQL,
                    RawCategory::RawAcl,
                    &RAW_ACL_SHAPE,
                    RAW_ACL_FETCH_LIMIT,
                )
                .await
            }
            Statement::Effective {
                routine_oids,
                role_oids,
            } => self.fetch_effective(routine_oids, role_oids).await,
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
                Ok(None) => RawCell::Null,
                Err(_) => RawCell::WrongType,
            }
        }
        PgCellKind::I32(alias) | PgCellKind::OptionalI32(alias) => {
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
                Ok(None) => RawCell::Null,
                Err(_) => RawCell::WrongType,
            }
        }
        PgCellKind::Text {
            bytes_alias,
            value_alias,
        } => {
            let len_at = *index;
            let value_at = len_at + 1;
            *index += 2;
            if !column_matches(row, len_at, bytes_alias, "INT8")
                || !column_matches(row, value_at, value_alias, "TEXT")
            {
                return RawCell::WrongType;
            }
            match (
                row.try_get::<Option<i64>, _>(len_at),
                row.try_get::<Option<&str>, _>(value_at),
            ) {
                (Ok(Some(original_bytes)), Ok(Some(value))) => RawCell::Text {
                    value: value.to_owned(),
                    original_bytes,
                },
                (Ok(None), Ok(None)) => RawCell::Null,
                _ => RawCell::WrongType,
            }
        }
    }
}

fn pg_column_count(shape: &[PgCellKind]) -> usize {
    shape
        .iter()
        .map(|kind| match kind {
            PgCellKind::Text { .. } => 2,
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

#[derive(Clone)]
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
    database_name: String,
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

struct RoutineHeader {
    name: String,
    oid: u32,
    schema_oid: u32,
    schema_name: String,
    owner_oid: u32,
    owner_name: String,
    kind: String,
    language_oid: u32,
    language_name: String,
    return_type_oid: u32,
    return_namespace_oid: u32,
    return_namespace: String,
    return_type_name: String,
    returns_set: bool,
    volatility: String,
    is_strict: bool,
    security_definer: bool,
    is_leakproof: bool,
    parallel_mode: String,
    argument_count: i32,
    argument_defaults: i32,
    variadic_oid: i64,
    all_arg_types_null: bool,
    arg_modes_null: bool,
    arg_defaults_null: bool,
    arg_names_dimensions: Option<i32>,
    arg_names_lower: Option<i32>,
    arg_names_length: Option<i32>,
    config_dimensions: Option<i32>,
    config_lower: Option<i32>,
    config_length: Option<i32>,
    config_first: Option<String>,
}

struct ArgumentRow {
    routine_name: String,
    routine_oid: u32,
    ordinal: i32,
    name: String,
    type_oid: u32,
    resolved_type_oid: u32,
    type_namespace_oid: u32,
    type_namespace: String,
    type_name: String,
}

struct RawAclRow {
    routine_name: String,
    routine_oid: u32,
    grantee_oid: i64,
    grantee_name: Option<String>,
    grantor_oid: u32,
    grantor_name: String,
    privilege_type: String,
    grantable: bool,
}

struct EffectiveRow {
    routine_name: String,
    routine_oid: u32,
    role_name: String,
    role_oid: u32,
    can_execute: bool,
    can_execute_with_grant_option: bool,
}

struct PreparedObservation {
    context: ContextEvidence,
    routine_oids: [u32; 3],
}

fn decode_context(
    row: RawRow,
    budget: &mut LogicalPayloadBudget,
) -> Result<ContextRow, SimulationVersionHelperMetadataError> {
    let mut d = RawDecoder::new(row, RawCategory::Context, CONTEXT_SHAPE.len(), budget)?;
    let value = ContextRow {
        current_user: d.text()?,
        session_user: d.text()?,
        observed_database_name: d.text()?,
        database_name: d.text()?,
        schema_name: d.text()?,
        worker_oid: d.i64()?,
        database_oid: d.i64()?,
        schema_oid: d.i64()?,
        migration_owner_oid: d.i64()?,
        database_owner_oid: d.i64()?,
        schema_owner_oid: d.i64()?,
        worker_can_login: d.boolean()?,
        worker_is_superuser: d.boolean()?,
        worker_can_create_database: d.boolean()?,
        worker_can_create_role: d.boolean()?,
        worker_can_replicate: d.boolean()?,
        worker_bypasses_rls: d.boolean()?,
        row_security: d.text()?,
        session_replication_role: d.text()?,
        transaction_read_only: d.boolean()?,
        transaction_isolation: d.text()?,
        database_connect: d.boolean()?,
        database_create: d.boolean()?,
        schema_usage: d.boolean()?,
        schema_create: d.boolean()?,
        database_effective_grant_option: d.boolean()?,
        schema_effective_grant_option: d.boolean()?,
        database_raw_grant_option: d.boolean()?,
        schema_raw_grant_option: d.boolean()?,
    };
    d.finish()?;
    Ok(value)
}

fn decode_roles(
    row: RawRow,
    budget: &mut LogicalPayloadBudget,
) -> Result<RoleRow, SimulationVersionHelperMetadataError> {
    let mut d = RawDecoder::new(row, RawCategory::Role, ROLE_SHAPE.len(), budget)?;
    let value = RoleRow {
        ordinal: d.i32()?,
        expected_name: d.text()?,
        actual_name: d.optional_text()?,
        oid: match d.take()? {
            RawCell::I64(value) => Some(value),
            RawCell::Null => None,
            _ => return Err(SimulationVersionHelperMetadataError::CatalogInvalid),
        },
    };
    d.finish()?;
    Ok(value)
}

fn decode_membership(
    row: RawRow,
    budget: &mut LogicalPayloadBudget,
) -> Result<(i64, i64, i64), SimulationVersionHelperMetadataError> {
    let mut d = RawDecoder::new(row, RawCategory::Membership, MEMBERSHIP_SHAPE.len(), budget)?;
    let member = d.i64()?;
    let role = d.i64()?;
    let grantor = d.i64()?;
    let _admin = d.boolean()?;
    let _inherit = d.boolean()?;
    let _set = d.boolean()?;
    d.finish()?;
    Ok((member, role, grantor))
}

fn decode_routine(
    row: RawRow,
    budget: &mut LogicalPayloadBudget,
) -> Result<RoutineHeader, SimulationVersionHelperMetadataError> {
    let mut d = RawDecoder::new(row, RawCategory::Routine, ROUTINE_SHAPE.len(), budget)?;
    let value = RoutineHeader {
        name: d.text()?,
        oid: positive_oid(d.i64()?)?,
        schema_oid: positive_oid(d.i64()?)?,
        schema_name: d.text()?,
        owner_oid: positive_oid(d.i64()?)?,
        owner_name: d.text()?,
        kind: d.text()?,
        language_oid: positive_oid(d.i64()?)?,
        language_name: d.text()?,
        return_type_oid: positive_oid(d.i64()?)?,
        return_namespace_oid: positive_oid(d.i64()?)?,
        return_namespace: d.text()?,
        return_type_name: d.text()?,
        returns_set: d.boolean()?,
        volatility: d.text()?,
        is_strict: d.boolean()?,
        security_definer: d.boolean()?,
        is_leakproof: d.boolean()?,
        parallel_mode: d.text()?,
        argument_count: d.i32()?,
        argument_defaults: d.i32()?,
        variadic_oid: d.i64()?,
        all_arg_types_null: d.boolean()?,
        arg_modes_null: d.boolean()?,
        arg_defaults_null: d.boolean()?,
        arg_names_dimensions: d.optional_i32()?,
        arg_names_lower: d.optional_i32()?,
        arg_names_length: d.optional_i32()?,
        config_dimensions: d.optional_i32()?,
        config_lower: d.optional_i32()?,
        config_length: d.optional_i32()?,
        config_first: d.optional_text()?,
    };
    d.finish()?;
    Ok(value)
}

fn decode_argument(
    row: RawRow,
    budget: &mut LogicalPayloadBudget,
) -> Result<ArgumentRow, SimulationVersionHelperMetadataError> {
    let mut d = RawDecoder::new(row, RawCategory::Argument, ARGUMENT_SHAPE.len(), budget)?;
    let value = ArgumentRow {
        routine_name: d.text()?,
        routine_oid: positive_oid(d.i64()?)?,
        ordinal: d.i32()?,
        name: d.text()?,
        type_oid: positive_oid(d.i64()?)?,
        resolved_type_oid: positive_oid(d.i64()?)?,
        type_namespace_oid: positive_oid(d.i64()?)?,
        type_namespace: d.text()?,
        type_name: d.text()?,
    };
    d.finish()?;
    Ok(value)
}

fn decode_raw_acl(
    row: RawRow,
    budget: &mut LogicalPayloadBudget,
) -> Result<RawAclRow, SimulationVersionHelperMetadataError> {
    let mut d = RawDecoder::new(row, RawCategory::RawAcl, RAW_ACL_SHAPE.len(), budget)?;
    let value = RawAclRow {
        routine_name: d.text()?,
        routine_oid: positive_oid(d.i64()?)?,
        grantee_oid: d.i64()?,
        grantee_name: d.optional_text()?,
        grantor_oid: positive_oid(d.i64()?)?,
        grantor_name: d.text()?,
        privilege_type: d.text()?,
        grantable: d.boolean()?,
    };
    d.finish()?;
    Ok(value)
}

fn decode_effective(
    row: RawRow,
    budget: &mut LogicalPayloadBudget,
) -> Result<EffectiveRow, SimulationVersionHelperMetadataError> {
    let mut d = RawDecoder::new(row, RawCategory::Effective, EFFECTIVE_SHAPE.len(), budget)?;
    let value = EffectiveRow {
        routine_name: d.text()?,
        routine_oid: positive_oid(d.i64()?)?,
        role_name: d.text()?,
        role_oid: positive_oid(d.i64()?)?,
        can_execute: d.boolean()?,
        can_execute_with_grant_option: d.boolean()?,
    };
    d.finish()?;
    Ok(value)
}

fn positive_oid(value: i64) -> Result<u32, SimulationVersionHelperMetadataError> {
    u32::try_from(value)
        .ok()
        .filter(|oid| *oid > 0)
        .ok_or(SimulationVersionHelperMetadataError::CatalogInvalid)
}

fn check_row_count(
    count: usize,
    accept_limit: usize,
    fetch_limit: usize,
    exact: Option<usize>,
) -> Result<(), SimulationVersionHelperMetadataError> {
    if count > fetch_limit || count > accept_limit {
        return Err(SimulationVersionHelperMetadataError::BoundExceeded);
    }
    if exact.is_some_and(|expected| count != expected) {
        return Err(SimulationVersionHelperMetadataError::CatalogInvalid);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)] // Keep the catalog row and byte bounds explicit.
fn decode_many<T>(
    rows: Vec<RawRow>,
    category: RawCategory,
    shape_len: usize,
    accept_limit: usize,
    fetch_limit: usize,
    exact: Option<usize>,
    budget: &mut LogicalPayloadBudget,
    mut decode: impl FnMut(
        RawRow,
        &mut LogicalPayloadBudget,
    ) -> Result<T, SimulationVersionHelperMetadataError>,
) -> Result<Vec<T>, SimulationVersionHelperMetadataError> {
    if rows.len() > fetch_limit {
        return Err(SimulationVersionHelperMetadataError::BoundExceeded);
    }
    let mut decoded = Vec::with_capacity(rows.len());
    for row in rows {
        if row.category != category || row.cells.len() != shape_len {
            for cell in &row.cells {
                budget.charge(cell)?;
            }
            return Err(SimulationVersionHelperMetadataError::CatalogInvalid);
        }
        decoded.push(decode(row, budget)?);
    }
    check_row_count(decoded.len(), accept_limit, fetch_limit, exact)?;
    Ok(decoded)
}

fn decode_one_context(
    rows: Vec<RawRow>,
    budget: &mut LogicalPayloadBudget,
) -> Result<ContextEvidence, SimulationVersionHelperMetadataError> {
    let contexts = decode_many(
        rows,
        RawCategory::Context,
        CONTEXT_SHAPE.len(),
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
            .ok_or(SimulationVersionHelperMetadataError::CatalogInvalid)?,
    )
}

fn validate_context(
    row: ContextRow,
) -> Result<ContextEvidence, SimulationVersionHelperMetadataError> {
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
        return Err(SimulationVersionHelperMetadataError::PolicyRejected);
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
        return Err(SimulationVersionHelperMetadataError::PolicyRejected);
    }
    Ok(ContextEvidence {
        database_name: row.database_name,
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
) -> Result<HashMap<String, u32>, SimulationVersionHelperMetadataError> {
    let rows = decode_many(
        rows,
        RawCategory::Role,
        ROLE_SHAPE.len(),
        ROLE_ACCEPT_LIMIT,
        ROLE_FETCH_LIMIT,
        Some(ROLE_ACCEPT_LIMIT),
        budget,
        decode_roles,
    )?;
    let mut by_name = HashMap::with_capacity(ROLE_NAMES.len());
    let mut unique_oids = HashSet::with_capacity(ROLE_NAMES.len());
    for (index, row) in rows.into_iter().enumerate() {
        let expected = ROLE_NAMES[index];
        let oid = positive_oid(
            row.oid
                .ok_or(SimulationVersionHelperMetadataError::CatalogInvalid)?,
        )?;
        if row.ordinal != (index + 1) as i32
            || row.expected_name != expected
            || row.actual_name.as_deref() != Some(expected)
            || !unique_oids.insert(oid)
        {
            return Err(SimulationVersionHelperMetadataError::PolicyRejected);
        }
        by_name.insert(expected.to_owned(), oid);
    }
    if by_name.get(WORKER_ROLE) != Some(&context.worker_oid)
        || by_name.get(MIGRATION_OWNER_ROLE) != Some(&context.migration_owner_oid)
    {
        return Err(SimulationVersionHelperMetadataError::PolicyRejected);
    }
    Ok(by_name)
}

fn validate_memberships(
    rows: Vec<RawRow>,
    worker_oid: u32,
    budget: &mut LogicalPayloadBudget,
) -> Result<(), SimulationVersionHelperMetadataError> {
    let rows = decode_many(
        rows,
        RawCategory::Membership,
        MEMBERSHIP_SHAPE.len(),
        MEMBERSHIP_ACCEPT_LIMIT,
        MEMBERSHIP_FETCH_LIMIT,
        Some(0),
        budget,
        decode_membership,
    )?;
    if rows.is_empty() {
        Ok(())
    } else if rows
        .iter()
        .all(|(member, _, _)| *member == i64::from(worker_oid))
    {
        Err(SimulationVersionHelperMetadataError::PolicyRejected)
    } else {
        Err(SimulationVersionHelperMetadataError::CatalogInvalid)
    }
}

fn role_oid(
    roles: &HashMap<String, u32>,
    name: &str,
) -> Result<u32, SimulationVersionHelperMetadataError> {
    roles
        .get(name)
        .copied()
        .ok_or(SimulationVersionHelperMetadataError::CatalogInvalid)
}

fn spec_by_name(name: &str) -> Option<&'static RoutineSpec> {
    ROUTINE_SPECS.iter().find(|spec| spec.name == name)
}

fn header_by_name<'a>(headers: &'a [RoutineHeader], name: &str) -> Option<&'a RoutineHeader> {
    headers.iter().find(|header| header.name == name)
}

fn validate_routines(
    rows: Vec<RawRow>,
    context: &ContextEvidence,
    roles: &HashMap<String, u32>,
    budget: &mut LogicalPayloadBudget,
) -> Result<Vec<RoutineHeader>, SimulationVersionHelperMetadataError> {
    let headers = decode_many(
        rows,
        RawCategory::Routine,
        ROUTINE_SHAPE.len(),
        ROUTINE_ACCEPT_LIMIT,
        ROUTINE_FETCH_LIMIT,
        Some(ROUTINE_ACCEPT_LIMIT),
        budget,
        decode_routine,
    )?;
    let owner_oid = role_oid(roles, MIGRATION_OWNER_ROLE)?;
    let expected_names = ROUTINE_QUERY_ORDER;
    let mut seen_routine_oids = HashSet::with_capacity(ROUTINE_ACCEPT_LIMIT);
    let mut return_type_oid = None;
    let mut language_oid = None;
    let mut pg_catalog_oid = None;
    for (index, header) in headers.iter().enumerate() {
        let spec = spec_by_name(expected_names[index])
            .ok_or(SimulationVersionHelperMetadataError::CatalogInvalid)?;
        if header.name != spec.name
            || !seen_routine_oids.insert(header.oid)
            || header.schema_oid != context.schema_oid
            || header.schema_name != PUBLIC_SCHEMA
            || header.owner_oid != owner_oid
            || header.owner_name != MIGRATION_OWNER_ROLE
            || header.kind != "f"
            || header.language_name != "sql"
            || header.returns_set
            || header.volatility != "i"
            || header.is_strict
            || header.security_definer
            || header.is_leakproof
            || header.parallel_mode != "u"
            || usize::try_from(header.argument_count).ok() != Some(spec.arguments.len())
            || header.argument_defaults != 0
            || header.variadic_oid != 0
            || !header.all_arg_types_null
            || !header.arg_modes_null
            || !header.arg_defaults_null
            || header.arg_names_dimensions != Some(1)
            || header.arg_names_lower != Some(1)
            || usize::try_from(header.arg_names_length.unwrap_or_default()).ok()
                != Some(spec.arguments.len())
            || header.config_dimensions != Some(1)
            || header.config_lower != Some(1)
            || header.config_length != Some(1)
            || header.config_first.as_deref() != Some("search_path=pg_catalog")
        {
            return Err(SimulationVersionHelperMetadataError::PolicyRejected);
        }
        let language_oid_checked = header.language_oid;
        if language_oid.is_some_and(|expected| expected != language_oid_checked) {
            return Err(SimulationVersionHelperMetadataError::CatalogInvalid);
        }
        language_oid = Some(language_oid_checked);
        if header.return_namespace != PG_CATALOG_SCHEMA || header.return_type_name != "bool" {
            return Err(SimulationVersionHelperMetadataError::PolicyRejected);
        }
        if pg_catalog_oid.is_some_and(|expected| expected != header.return_namespace_oid) {
            return Err(SimulationVersionHelperMetadataError::CatalogInvalid);
        }
        pg_catalog_oid = Some(header.return_namespace_oid);
        if return_type_oid.is_some_and(|expected| expected != header.return_type_oid) {
            return Err(SimulationVersionHelperMetadataError::CatalogInvalid);
        }
        return_type_oid = Some(header.return_type_oid);
    }
    if headers.len() != ROUTINE_ACCEPT_LIMIT {
        return Err(SimulationVersionHelperMetadataError::CatalogInvalid);
    }
    Ok(headers)
}

fn ordered_routine_oids(
    headers: &[RoutineHeader],
) -> Result<[u32; 3], SimulationVersionHelperMetadataError> {
    let mut result = [0_u32; 3];
    for (index, spec) in ROUTINE_SPECS.iter().enumerate() {
        result[index] = header_by_name(headers, spec.name)
            .ok_or(SimulationVersionHelperMetadataError::CatalogInvalid)?
            .oid;
    }
    if result.contains(&0) || HashSet::from(result).len() != result.len() {
        return Err(SimulationVersionHelperMetadataError::CatalogInvalid);
    }
    Ok(result)
}

fn validate_arguments(
    rows: Vec<RawRow>,
    headers: &[RoutineHeader],
    budget: &mut LogicalPayloadBudget,
) -> Result<(), SimulationVersionHelperMetadataError> {
    let arguments = decode_many(
        rows,
        RawCategory::Argument,
        ARGUMENT_SHAPE.len(),
        ARGUMENT_ACCEPT_LIMIT,
        ARGUMENT_FETCH_LIMIT,
        Some(ARGUMENT_ACCEPT_LIMIT),
        budget,
        decode_argument,
    )?;
    let mut cursor = 0;
    let mut pg_catalog_oid = None;
    for routine_name in ROUTINE_QUERY_ORDER {
        let spec = spec_by_name(routine_name)
            .ok_or(SimulationVersionHelperMetadataError::CatalogInvalid)?;
        if spec.arguments.len() > ARGUMENTS_PER_ROUTINE_LIMIT {
            return Err(SimulationVersionHelperMetadataError::BoundExceeded);
        }
        let header = header_by_name(headers, routine_name)
            .ok_or(SimulationVersionHelperMetadataError::CatalogInvalid)?;
        for (index, expected) in spec.arguments.iter().enumerate() {
            let argument = arguments
                .get(cursor)
                .ok_or(SimulationVersionHelperMetadataError::CatalogInvalid)?;
            if argument.routine_name != routine_name
                || argument.routine_oid != header.oid
                || argument.ordinal != (index + 1) as i32
                || argument.name != expected.name
                || argument.type_oid != argument.resolved_type_oid
                || argument.type_namespace != PG_CATALOG_SCHEMA
                || argument.type_name != expected.type_name
            {
                return Err(SimulationVersionHelperMetadataError::PolicyRejected);
            }
            if pg_catalog_oid
                .is_some_and(|expected_oid| expected_oid != argument.type_namespace_oid)
            {
                return Err(SimulationVersionHelperMetadataError::CatalogInvalid);
            }
            pg_catalog_oid = Some(argument.type_namespace_oid);
            cursor += 1;
        }
    }
    if cursor != arguments.len() || cursor != ARGUMENT_ACCEPT_LIMIT {
        return Err(SimulationVersionHelperMetadataError::CatalogInvalid);
    }
    if let Some(header_namespace_oid) = headers.first().map(|header| header.return_namespace_oid)
        && pg_catalog_oid != Some(header_namespace_oid)
    {
        return Err(SimulationVersionHelperMetadataError::CatalogInvalid);
    }
    Ok(())
}

fn validate_raw_acl(
    rows: Vec<RawRow>,
    headers: &[RoutineHeader],
    roles: &HashMap<String, u32>,
    budget: &mut LogicalPayloadBudget,
) -> Result<(), SimulationVersionHelperMetadataError> {
    let acls = decode_many(
        rows,
        RawCategory::RawAcl,
        RAW_ACL_SHAPE.len(),
        RAW_ACL_ACCEPT_LIMIT,
        RAW_ACL_FETCH_LIMIT,
        None,
        budget,
        decode_raw_acl,
    )?;
    let owner_oid = role_oid(roles, MIGRATION_OWNER_ROLE)?;
    let mut seen = HashSet::new();
    let mut binding_allowances = HashSet::new();
    for acl in acls {
        if acl.grantee_oid == 0 {
            return Err(SimulationVersionHelperMetadataError::PolicyRejected);
        }
        let grantee_oid = positive_oid(acl.grantee_oid)?;
        let spec = spec_by_name(&acl.routine_name)
            .ok_or(SimulationVersionHelperMetadataError::CatalogInvalid)?;
        let header = header_by_name(headers, &acl.routine_name)
            .ok_or(SimulationVersionHelperMetadataError::CatalogInvalid)?;
        let grantee_name = ROLE_NAMES
            .iter()
            .find(|name| roles.get(**name) == Some(&grantee_oid))
            .copied()
            .ok_or(SimulationVersionHelperMetadataError::PolicyRejected)?;
        if acl.routine_oid != header.oid
            || acl.grantor_oid != owner_oid
            || acl.grantor_name != MIGRATION_OWNER_ROLE
            || acl.grantee_name.as_deref() != Some(grantee_name)
            || acl.privilege_type != "EXECUTE"
            || acl.grantable
        {
            return Err(SimulationVersionHelperMetadataError::PolicyRejected);
        }
        if grantee_name != MIGRATION_OWNER_ROLE && !spec.serving_execute.contains(&grantee_name) {
            return Err(SimulationVersionHelperMetadataError::PolicyRejected);
        }
        if !seen.insert((acl.routine_oid, grantee_oid, acl.grantor_oid)) {
            return Err(SimulationVersionHelperMetadataError::PolicyRejected);
        }
        if spec.name == "owner_beta_simulation_version_binding_matches"
            && (grantee_name == "app" || grantee_name == WORKER_ROLE)
        {
            binding_allowances.insert(grantee_name);
        }
    }
    if !binding_allowances.contains("app") || !binding_allowances.contains(WORKER_ROLE) {
        return Err(SimulationVersionHelperMetadataError::PolicyRejected);
    }
    Ok(())
}

fn validate_effective(
    rows: Vec<RawRow>,
    routine_oids: [u32; 3],
    roles: &HashMap<String, u32>,
    budget: &mut LogicalPayloadBudget,
) -> Result<(), SimulationVersionHelperMetadataError> {
    let effective = decode_many(
        rows,
        RawCategory::Effective,
        EFFECTIVE_SHAPE.len(),
        EFFECTIVE_ACCEPT_LIMIT,
        EFFECTIVE_FETCH_LIMIT,
        Some(EFFECTIVE_ACCEPT_LIMIT),
        budget,
        decode_effective,
    )?;
    let mut cursor = 0;
    for (routine_index, spec) in ROUTINE_SPECS.iter().enumerate() {
        for role_name in ROLE_NAMES {
            let row = effective
                .get(cursor)
                .ok_or(SimulationVersionHelperMetadataError::CatalogInvalid)?;
            let expected_execute = role_name == MIGRATION_OWNER_ROLE
                || (spec.name == "owner_beta_simulation_version_binding_matches"
                    && (role_name == "app" || role_name == WORKER_ROLE));
            if row.routine_name != spec.name
                || row.routine_oid != routine_oids[routine_index]
                || row.role_name != role_name
                || row.role_oid != role_oid(roles, role_name)?
                || row.can_execute != expected_execute
                || (role_name != MIGRATION_OWNER_ROLE && row.can_execute_with_grant_option)
            {
                return Err(SimulationVersionHelperMetadataError::PolicyRejected);
            }
            cursor += 1;
        }
    }
    if cursor != EFFECTIVE_ACCEPT_LIMIT || cursor != effective.len() {
        return Err(SimulationVersionHelperMetadataError::CatalogInvalid);
    }
    Ok(())
}

async fn execute_checked<T: ObserverTransport>(
    transport: &mut T,
    statement: Statement,
    canceled: &AtomicBool,
    error: SimulationVersionHelperMetadataError,
) -> Result<Vec<RawRow>, SimulationVersionHelperMetadataError> {
    if is_canceled(canceled) {
        return Err(SimulationVersionHelperMetadataError::Canceled);
    }
    let result = transport.execute(statement).await;
    if is_canceled(canceled) {
        return Err(SimulationVersionHelperMetadataError::Canceled);
    }
    result.map_err(|failure| map_transport_failure(failure, error))
}

fn map_transport_failure(
    failure: TransportFailure,
    fallback: SimulationVersionHelperMetadataError,
) -> SimulationVersionHelperMetadataError {
    match failure {
        TransportFailure::Database
            if fallback == SimulationVersionHelperMetadataError::SetupFailed =>
        {
            fallback
        }
        TransportFailure::Database => SimulationVersionHelperMetadataError::CatalogUnavailable,
        TransportFailure::State => SimulationVersionHelperMetadataError::CatalogInvalid,
        TransportFailure::FetchBound => SimulationVersionHelperMetadataError::BoundExceeded,
    }
}

async fn fetch_checked<T: ObserverTransport>(
    transport: &mut T,
    statement: Statement,
    canceled: &AtomicBool,
    error: SimulationVersionHelperMetadataError,
) -> Result<Vec<RawRow>, SimulationVersionHelperMetadataError> {
    execute_checked(transport, statement, canceled, error).await
}

async fn prepare_observation<T: ObserverTransport>(
    transport: &mut T,
    canceled: &AtomicBool,
    timeout_ms: i32,
) -> Result<PreparedObservation, SimulationVersionHelperMetadataError> {
    let mut budget = LogicalPayloadBudget::default();
    let setup = execute_checked(
        transport,
        Statement::Configure { timeout_ms },
        canceled,
        SimulationVersionHelperMetadataError::SetupFailed,
    )
    .await?;
    if !setup.is_empty() {
        return Err(SimulationVersionHelperMetadataError::SetupFailed);
    }

    let context_rows = fetch_checked(
        transport,
        Statement::Context,
        canceled,
        SimulationVersionHelperMetadataError::CatalogInvalid,
    )
    .await?;
    let context = decode_one_context(context_rows, &mut budget)?;

    let role_rows = fetch_checked(
        transport,
        Statement::Roles,
        canceled,
        SimulationVersionHelperMetadataError::CatalogInvalid,
    )
    .await?;
    let roles = validate_roles(role_rows, &context, &mut budget)?;

    let membership_rows = fetch_checked(
        transport,
        Statement::Memberships {
            worker_oid: context.worker_oid,
        },
        canceled,
        SimulationVersionHelperMetadataError::CatalogInvalid,
    )
    .await?;
    validate_memberships(membership_rows, context.worker_oid, &mut budget)?;

    let routine_rows = fetch_checked(
        transport,
        Statement::Routines,
        canceled,
        SimulationVersionHelperMetadataError::CatalogInvalid,
    )
    .await?;
    let headers = validate_routines(routine_rows, &context, &roles, &mut budget)?;
    let routine_oids = ordered_routine_oids(&headers)?;

    let argument_rows = fetch_checked(
        transport,
        Statement::Arguments,
        canceled,
        SimulationVersionHelperMetadataError::CatalogInvalid,
    )
    .await?;
    validate_arguments(argument_rows, &headers, &mut budget)?;

    let acl_rows = fetch_checked(
        transport,
        Statement::RawAcl,
        canceled,
        SimulationVersionHelperMetadataError::CatalogInvalid,
    )
    .await?;
    validate_raw_acl(acl_rows, &headers, &roles, &mut budget)?;

    let role_oids = [
        role_oid(&roles, ROLE_NAMES[0])?,
        role_oid(&roles, ROLE_NAMES[1])?,
        role_oid(&roles, ROLE_NAMES[2])?,
        role_oid(&roles, ROLE_NAMES[3])?,
        role_oid(&roles, ROLE_NAMES[4])?,
        role_oid(&roles, ROLE_NAMES[5])?,
    ];
    let effective_rows = fetch_checked(
        transport,
        Statement::Effective {
            routine_oids,
            role_oids,
        },
        canceled,
        SimulationVersionHelperMetadataError::CatalogInvalid,
    )
    .await?;
    validate_effective(effective_rows, routine_oids, &roles, &mut budget)?;

    let final_context_rows = fetch_checked(
        transport,
        Statement::FinalContext,
        canceled,
        SimulationVersionHelperMetadataError::CatalogInvalid,
    )
    .await?;
    let final_context = decode_one_context(final_context_rows, &mut budget)?;
    if final_context != context {
        return Err(SimulationVersionHelperMetadataError::PolicyRejected);
    }
    Ok(PreparedObservation {
        context,
        routine_oids,
    })
}

fn is_canceled(canceled: &AtomicBool) -> bool {
    canceled.load(Ordering::Acquire)
}

fn timeout_millis(timeout_seconds: u64) -> Result<i32, SimulationVersionHelperMetadataError> {
    let milliseconds = timeout_seconds
        .checked_mul(1000)
        .filter(|value| *value > 0 && *value <= i32::MAX as u64)
        .ok_or(SimulationVersionHelperMetadataError::InvalidBound)?;
    i32::try_from(milliseconds).map_err(|_| SimulationVersionHelperMetadataError::InvalidBound)
}

async fn close_before_begin<T: ObserverTransport>(
    transport: &mut T,
    primary: SimulationVersionHelperMetadataError,
) -> SimulationVersionHelperMetadataError {
    match transport.close().await {
        Ok(()) => primary,
        Err(_) => SimulationVersionHelperMetadataError::CloseUnconfirmed {
            primary: primary.into(),
        },
    }
}

async fn rollback_and_close<T: ObserverTransport>(
    transport: &mut T,
    primary: SimulationVersionHelperMetadataError,
) -> SimulationVersionHelperMetadataError {
    let rollback_ok = matches!(
        transport.execute(Statement::Rollback).await,
        Ok(rows) if rows.is_empty()
    );
    let close_ok = transport.close().await.is_ok();
    match (rollback_ok, close_ok) {
        (true, true) => primary,
        (false, true) => SimulationVersionHelperMetadataError::RollbackUnconfirmed {
            primary: primary.into(),
        },
        (true, false) => SimulationVersionHelperMetadataError::CloseUnconfirmed {
            primary: primary.into(),
        },
        (false, false) => SimulationVersionHelperMetadataError::RollbackAndCloseUnconfirmed {
            primary: primary.into(),
        },
    }
}

async fn observe_with_transport<T: ObserverTransport>(
    transport: &mut T,
    config: &SimulationProcessConfig,
    canceled: &AtomicBool,
) -> Result<SimulationVersionHelperMetadataObservation, SimulationVersionHelperMetadataError> {
    let timeout_ms = timeout_millis(config.limits().startup_timeout_seconds())?;
    if is_canceled(canceled) {
        return Err(SimulationVersionHelperMetadataError::Canceled);
    }
    if transport.acquire().await.is_err() {
        return Err(SimulationVersionHelperMetadataError::AcquisitionFailed);
    }
    transport.arm_close_on_drop();
    if is_canceled(canceled) {
        return Err(
            close_before_begin(transport, SimulationVersionHelperMetadataError::Canceled).await,
        );
    }

    let begin = transport.execute(Statement::Begin).await;
    match begin {
        Ok(rows) if rows.is_empty() => {}
        Ok(_) => {
            return Err(rollback_and_close(
                transport,
                SimulationVersionHelperMetadataError::BeginResultInvalid,
            )
            .await);
        }
        Err(_) => {
            return Err(rollback_and_close(
                transport,
                SimulationVersionHelperMetadataError::BeginFailed,
            )
            .await);
        }
    }
    if is_canceled(canceled) {
        return Err(
            rollback_and_close(transport, SimulationVersionHelperMetadataError::Canceled).await,
        );
    }

    let prepared = match prepare_observation(transport, canceled, timeout_ms).await {
        Ok(prepared) => prepared,
        Err(error) => return Err(rollback_and_close(transport, error).await),
    };
    if is_canceled(canceled) {
        return Err(
            rollback_and_close(transport, SimulationVersionHelperMetadataError::Canceled).await,
        );
    }

    if is_canceled(canceled) {
        return Err(
            rollback_and_close(transport, SimulationVersionHelperMetadataError::Canceled).await,
        );
    }
    let commit = transport.execute(Statement::Commit).await;
    let commit_confirmed = matches!(commit, Ok(rows) if rows.is_empty());
    if !commit_confirmed {
        return Err(match transport.close().await {
            Ok(()) => SimulationVersionHelperMetadataError::CompletionUnconfirmed,
            Err(_) => SimulationVersionHelperMetadataError::CompletionAndCleanupUnconfirmed,
        });
    }
    let canceled_after_commit = is_canceled(canceled);
    if transport.close().await.is_err() {
        return Err(if canceled_after_commit || is_canceled(canceled) {
            SimulationVersionHelperMetadataError::CloseUnconfirmed {
                primary: PrimaryFailure::Canceled,
            }
        } else {
            SimulationVersionHelperMetadataError::CleanupUnconfirmed
        });
    }
    if canceled_after_commit || is_canceled(canceled) {
        return Err(SimulationVersionHelperMetadataError::Canceled);
    }
    Ok(SimulationVersionHelperMetadataObservation::completed(
        config.config_sha256(),
        prepared,
    ))
}

/// Observe metadata on one fresh disposable connection. Dropping this future
/// yields no token; close-on-drop disposes an armed SQLx connection but does
/// not claim an awaited server rollback or close receipt.
pub(super) async fn observe_simulation_version_helper_metadata(
    pool: &PgPool,
    config: &SimulationProcessConfig,
    canceled: &AtomicBool,
) -> Result<SimulationVersionHelperMetadataObservation, SimulationVersionHelperMetadataError> {
    let mut transport = SqlxObserverTransport::new(pool);
    observe_with_transport(&mut transport, config, canceled).await
}

#[cfg(test)]
mod tests;
