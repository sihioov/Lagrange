//! Private, bounded observation of the adopted trigger attachment and routine
//! header candidate. This records metadata only; it does not inspect function
//! bodies, prove trigger execution, or establish RI, ACL, or startup safety.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use sqlx::{
    Column, PgConnection, PgPool, Postgres, Row, TypeInfo, ValueRef, pool::PoolConnection,
    postgres::PgRow,
};

use super::SimulationProcessConfig;

const POLICY_ID: &str = "r4-simulation-trigger-metadata-source-candidate-v1";
const POLICY_FINGERPRINT: &str =
    "sha256:567470f4aeab6c799218a9a4a460e985cb35a65270abc64cab7dc6592368327b";
const PUBLIC_SCHEMA: &str = "public";
const PG_CATALOG_SCHEMA: &str = "pg_catalog";
const WORKER_ROLE: &str = "worker";
const MIGRATION_OWNER_ROLE: &str = "migration_owner";
const SIMULATION_FUNCTION_PREFIX: &str = "owner_beta_simulation_";

const INITIAL_CONTEXT_ACCEPT_LIMIT: usize = 1;
const INITIAL_CONTEXT_FETCH_LIMIT: usize = 2;
const FINAL_CONTEXT_ACCEPT_LIMIT: usize = 1;
const FINAL_CONTEXT_FETCH_LIMIT: usize = 2;
const ROLE_ACCEPT_LIMIT: usize = 2;
const ROLE_FETCH_LIMIT: usize = 3;
const WORKER_MEMBERSHIP_ACCEPT_LIMIT: usize = 0;
const WORKER_MEMBERSHIP_FETCH_LIMIT: usize = 1;
const RELATION_IDENTITY_ACCEPT_LIMIT: usize = 10;
const RELATION_IDENTITY_FETCH_LIMIT: usize = 11;
const PRIMARY_INHERITANCE_ACCEPT_LIMIT: usize = 0;
const PRIMARY_INHERITANCE_FETCH_LIMIT: usize = 1;
const ROOT_HEADER_ACCEPT_LIMIT: usize = 17;
const ROOT_HEADER_FETCH_LIMIT: usize = 18;
const ROOT_SETTING_ACCEPT_LIMIT: usize = 25;
const ROOT_SETTING_FETCH_LIMIT: usize = 26;
const FORWARD_TRIGGER_ACCEPT_LIMIT: usize = 21;
const FORWARD_TRIGGER_FETCH_LIMIT: usize = 22;
const ADJACENT_HOOK_ACCEPT_LIMIT: usize = 1;
const ADJACENT_HOOK_FETCH_LIMIT: usize = 2;
const REVERSE_HOOK_ACCEPT_LIMIT: usize = 27;
const REVERSE_HOOK_FETCH_LIMIT: usize = 28;
const UPDATE_ATTRIBUTE_ACCEPT_LIMIT: usize = 2;
const UPDATE_ATTRIBUTE_FETCH_LIMIT: usize = 3;
const USER_CONSTRAINT_ACCEPT_LIMIT: usize = 13;
const USER_CONSTRAINT_FETCH_LIMIT: usize = 14;
const SETUP_ACCEPT_LIMIT: usize = 1;
const SETUP_FETCH_LIMIT: usize = 2;
const TEXT_ACCEPT_BYTES: usize = 1024;
const TEXT_FETCH_BYTES: usize = 1025;
const TRIGGER_ARGS_FETCH_BYTES: usize = 1;
const LOGICAL_PAYLOAD_LIMIT: usize = 131_072;

const BEGIN_SQL: &str = "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY";
const SETUP_SQL: &str = r#"
WITH setup AS (
    SELECT pg_catalog.set_config('search_path', 'pg_catalog', true)::pg_catalog.text
               AS search_path,
           pg_catalog.set_config(
               'statement_timeout', $1::pg_catalog.int8::pg_catalog.text || 'ms', true
           )::pg_catalog.text AS statement_timeout,
           pg_catalog.set_config(
               'lock_timeout', $1::pg_catalog.int8::pg_catalog.text || 'ms', true
           )::pg_catalog.text AS lock_timeout
)
SELECT (setup.search_path IS NULL)::pg_catalog.bool AS search_path_is_null,
       pg_catalog.octet_length(setup.search_path::pg_catalog.text)::pg_catalog.int8
           AS search_path_bytes,
       setup.search_path::pg_catalog.text AS search_path,
       (setup.statement_timeout IS NULL)::pg_catalog.bool AS statement_timeout_is_null,
       pg_catalog.octet_length(setup.statement_timeout::pg_catalog.text)::pg_catalog.int8
           AS statement_timeout_bytes,
       setup.statement_timeout::pg_catalog.text AS statement_timeout,
       (setup.lock_timeout IS NULL)::pg_catalog.bool AS lock_timeout_is_null,
       pg_catalog.octet_length(setup.lock_timeout::pg_catalog.text)::pg_catalog.int8
           AS lock_timeout_bytes,
       setup.lock_timeout::pg_catalog.text AS lock_timeout
  FROM setup
LIMIT 2
"#;

const CONTEXT_SQL: &str = r#"
WITH context AS (
    SELECT current_user::pg_catalog.text AS current_user,
           session_user::pg_catalog.text AS session_user,
           pg_catalog.current_database()::pg_catalog.text AS database_name,
           pg_catalog.current_setting('row_security')::pg_catalog.text AS row_security,
           pg_catalog.current_setting('session_replication_role')::pg_catalog.text
               AS session_replication_role,
           pg_catalog.current_setting('transaction_read_only')::pg_catalog.bool
               AS transaction_read_only,
           pg_catalog.current_setting('transaction_isolation')::pg_catalog.text
               AS transaction_isolation,
           pg_catalog.current_setting('search_path')::pg_catalog.text AS search_path,
           pg_catalog.current_setting('statement_timeout')::pg_catalog.text AS statement_timeout,
           pg_catalog.current_setting('lock_timeout')::pg_catalog.text AS lock_timeout
)
SELECT (context.current_user IS NULL)::pg_catalog.bool AS current_user_is_null,
       CASE WHEN context.current_user IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(context.current_user::pg_catalog.text)::pg_catalog.int8
       END::pg_catalog.int8 AS current_user_bytes,
       CASE WHEN context.current_user IS NULL
                  OR pg_catalog.octet_length(context.current_user::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text ELSE context.current_user END::pg_catalog.text AS current_user,
       (context.session_user IS NULL)::pg_catalog.bool AS session_user_is_null,
       CASE WHEN context.session_user IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(context.session_user::pg_catalog.text)::pg_catalog.int8
       END::pg_catalog.int8 AS session_user_bytes,
       CASE WHEN context.session_user IS NULL
                  OR pg_catalog.octet_length(context.session_user::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text ELSE context.session_user END::pg_catalog.text AS session_user,
       (context.database_name IS NULL)::pg_catalog.bool AS observed_database_name_is_null,
       CASE WHEN context.database_name IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(context.database_name::pg_catalog.text)::pg_catalog.int8
       END::pg_catalog.int8 AS observed_database_name_bytes,
       CASE WHEN context.database_name IS NULL
                  OR pg_catalog.octet_length(context.database_name::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text ELSE context.database_name END::pg_catalog.text
           AS observed_database_name,
       (database.datname IS NULL)::pg_catalog.bool AS database_name_is_null,
       CASE WHEN database.datname IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(database.datname::pg_catalog.text)::pg_catalog.int8
       END::pg_catalog.int8 AS database_name_bytes,
       CASE WHEN database.datname IS NULL
                  OR pg_catalog.octet_length(database.datname::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text ELSE database.datname::pg_catalog.text END::pg_catalog.text
           AS database_name,
       (public_schema.nspname IS NULL)::pg_catalog.bool AS schema_name_is_null,
       CASE WHEN public_schema.nspname IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(public_schema.nspname::pg_catalog.text)::pg_catalog.int8
       END::pg_catalog.int8 AS schema_name_bytes,
       CASE WHEN public_schema.nspname IS NULL
                  OR pg_catalog.octet_length(public_schema.nspname::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text ELSE public_schema.nspname::pg_catalog.text END::pg_catalog.text
           AS schema_name,
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
       worker.rolinherit::pg_catalog.bool AS worker_inherits_roles,
       (context.row_security IS NULL)::pg_catalog.bool AS row_security_is_null,
       CASE WHEN context.row_security IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(context.row_security::pg_catalog.text)::pg_catalog.int8
       END::pg_catalog.int8 AS row_security_bytes,
       CASE WHEN context.row_security IS NULL
                  OR pg_catalog.octet_length(context.row_security::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text ELSE context.row_security END::pg_catalog.text AS row_security,
       (context.session_replication_role IS NULL)::pg_catalog.bool
           AS session_replication_role_is_null,
       CASE WHEN context.session_replication_role IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(
                context.session_replication_role::pg_catalog.text
            )::pg_catalog.int8
       END::pg_catalog.int8 AS session_replication_role_bytes,
       CASE WHEN context.session_replication_role IS NULL
                  OR pg_catalog.octet_length(
                      context.session_replication_role::pg_catalog.text
                  ) > 1025
            THEN ''::pg_catalog.text ELSE context.session_replication_role END::pg_catalog.text
           AS session_replication_role,
       context.transaction_read_only::pg_catalog.bool AS transaction_read_only,
       (context.transaction_isolation IS NULL)::pg_catalog.bool
           AS transaction_isolation_is_null,
       CASE WHEN context.transaction_isolation IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(
                context.transaction_isolation::pg_catalog.text
            )::pg_catalog.int8
       END::pg_catalog.int8 AS transaction_isolation_bytes,
       CASE WHEN context.transaction_isolation IS NULL
                  OR pg_catalog.octet_length(
                      context.transaction_isolation::pg_catalog.text
                  ) > 1025
            THEN ''::pg_catalog.text ELSE context.transaction_isolation END::pg_catalog.text
           AS transaction_isolation,
       (context.search_path IS NULL)::pg_catalog.bool AS search_path_is_null,
       CASE WHEN context.search_path IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(context.search_path::pg_catalog.text)::pg_catalog.int8
       END::pg_catalog.int8 AS search_path_bytes,
       CASE WHEN context.search_path IS NULL
                  OR pg_catalog.octet_length(context.search_path::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text ELSE context.search_path END::pg_catalog.text AS search_path,
       (context.statement_timeout IS NULL)::pg_catalog.bool AS statement_timeout_is_null,
       CASE WHEN context.statement_timeout IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(
                context.statement_timeout::pg_catalog.text
            )::pg_catalog.int8
       END::pg_catalog.int8 AS statement_timeout_bytes,
       CASE WHEN context.statement_timeout IS NULL
                  OR pg_catalog.octet_length(context.statement_timeout::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text ELSE context.statement_timeout END::pg_catalog.text
           AS statement_timeout,
       (context.lock_timeout IS NULL)::pg_catalog.bool AS lock_timeout_is_null,
       CASE WHEN context.lock_timeout IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(context.lock_timeout::pg_catalog.text)::pg_catalog.int8
       END::pg_catalog.int8 AS lock_timeout_bytes,
       CASE WHEN context.lock_timeout IS NULL
                  OR pg_catalog.octet_length(context.lock_timeout::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text ELSE context.lock_timeout END::pg_catalog.text AS lock_timeout
  FROM context
  LEFT JOIN pg_catalog.pg_database AS database ON database.datname = context.database_name
  LEFT JOIN pg_catalog.pg_namespace AS public_schema ON public_schema.nspname = 'public'
  LEFT JOIN pg_catalog.pg_roles AS worker ON worker.rolname = 'worker'
  LEFT JOIN pg_catalog.pg_roles AS migration_owner ON migration_owner.rolname = 'migration_owner'
 LIMIT 2
"#;

const ROLES_SQL: &str = r#"
SELECT expected_roles.ordinal::pg_catalog.int4 AS role_ordinal,
       false::pg_catalog.bool AS expected_name_is_null,
       pg_catalog.octet_length(expected_roles.expected_name)::pg_catalog.int8 AS expected_name_bytes,
       expected_roles.expected_name::pg_catalog.text AS expected_name,
       (actual_role.rolname IS NULL)::pg_catalog.bool AS actual_name_is_null,
       CASE WHEN actual_role.rolname IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(actual_role.rolname::pg_catalog.text)::pg_catalog.int8
       END::pg_catalog.int8 AS actual_name_bytes,
       CASE WHEN actual_role.rolname IS NULL
                  OR pg_catalog.octet_length(actual_role.rolname::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text ELSE actual_role.rolname::pg_catalog.text END::pg_catalog.text
           AS actual_name,
       actual_role.oid::pg_catalog.int8 AS role_oid
  FROM (VALUES
      (1::pg_catalog.int4, 'worker'::pg_catalog.text),
      (2::pg_catalog.int4, 'migration_owner'::pg_catalog.text)
  ) AS expected_roles(ordinal, expected_name)
  LEFT JOIN pg_catalog.pg_roles AS actual_role
    ON actual_role.rolname = expected_roles.expected_name
 ORDER BY expected_roles.ordinal
 LIMIT 3
"#;

const WORKER_MEMBERSHIPS_SQL: &str = r#"
SELECT membership.member::pg_catalog.int8 AS member_oid,
       membership.roleid::pg_catalog.int8 AS role_oid,
       membership.grantor::pg_catalog.int8 AS grantor_oid,
       membership.admin_option::pg_catalog.bool AS admin_option,
       membership.inherit_option::pg_catalog.bool AS inherit_option,
       membership.set_option::pg_catalog.bool AS set_option
  FROM pg_catalog.pg_auth_members AS membership
 WHERE membership.member::pg_catalog.int8 = $1::pg_catalog.int8
 ORDER BY membership.roleid, membership.grantor
 LIMIT 1
"#;

const RELATIONS_SQL: &str = r#"
SELECT expected_relation.ordinal::pg_catalog.int4 AS relation_ordinal,
       false::pg_catalog.bool AS expected_name_is_null,
       pg_catalog.octet_length(expected_relation.expected_name)::pg_catalog.int8
           AS expected_name_bytes,
       expected_relation.expected_name::pg_catalog.text AS expected_name,
       relation.oid::pg_catalog.int8 AS relation_oid,
       namespace.oid::pg_catalog.int8 AS schema_oid,
       (namespace.nspname IS NULL)::pg_catalog.bool AS schema_name_is_null,
       CASE WHEN namespace.nspname IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(namespace.nspname::pg_catalog.text)::pg_catalog.int8
       END::pg_catalog.int8 AS schema_name_bytes,
       CASE WHEN namespace.nspname IS NULL
                  OR pg_catalog.octet_length(namespace.nspname::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text ELSE namespace.nspname::pg_catalog.text END::pg_catalog.text
           AS schema_name,
       (relation.relname IS NULL)::pg_catalog.bool AS actual_name_is_null,
       CASE WHEN relation.relname IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(relation.relname::pg_catalog.text)::pg_catalog.int8
       END::pg_catalog.int8 AS actual_name_bytes,
       CASE WHEN relation.relname IS NULL
                  OR pg_catalog.octet_length(relation.relname::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text ELSE relation.relname::pg_catalog.text END::pg_catalog.text
           AS actual_name,
       relation.relowner::pg_catalog.int8 AS owner_oid,
       (owner_role.rolname IS NULL)::pg_catalog.bool AS owner_name_is_null,
       CASE WHEN owner_role.rolname IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(owner_role.rolname::pg_catalog.text)::pg_catalog.int8
       END::pg_catalog.int8 AS owner_name_bytes,
       CASE WHEN owner_role.rolname IS NULL
                  OR pg_catalog.octet_length(owner_role.rolname::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text ELSE owner_role.rolname::pg_catalog.text END::pg_catalog.text
           AS owner_name,
       (relation.relkind IS NULL)::pg_catalog.bool AS relation_kind_is_null,
       CASE WHEN relation.relkind IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(relation.relkind::pg_catalog.text)::pg_catalog.int8
       END::pg_catalog.int8 AS relation_kind_bytes,
       CASE WHEN relation.relkind IS NULL THEN ''::pg_catalog.text
            ELSE relation.relkind::pg_catalog.text END::pg_catalog.text AS relation_kind,
       (relation.relpersistence IS NULL)::pg_catalog.bool AS persistence_is_null,
       CASE WHEN relation.relpersistence IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(
                relation.relpersistence::pg_catalog.text
            )::pg_catalog.int8
       END::pg_catalog.int8 AS persistence_bytes,
       CASE WHEN relation.relpersistence IS NULL THEN ''::pg_catalog.text
            ELSE relation.relpersistence::pg_catalog.text END::pg_catalog.text AS persistence,
       relation.relispartition::pg_catalog.bool AS is_partition
  FROM (VALUES
      (1::pg_catalog.int4, 'job_attempts'::pg_catalog.text),
      (2::pg_catalog.int4, 'jobs'::pg_catalog.text),
      (3::pg_catalog.int4, 'owner_beta_daily_publication_claims'::pg_catalog.text),
      (4::pg_catalog.int4, 'owner_beta_daily_reservations'::pg_catalog.text),
      (5::pg_catalog.int4, 'owner_beta_recommendation_items'::pg_catalog.text),
      (6::pg_catalog.int4, 'owner_beta_recommendation_runs'::pg_catalog.text),
      (7::pg_catalog.int4, 'owner_beta_simulation_result_pairs'::pg_catalog.text),
      (8::pg_catalog.int4, 'owner_beta_simulation_runs'::pg_catalog.text),
      (9::pg_catalog.int4, 'stock_analysis_runs'::pg_catalog.text),
      (10::pg_catalog.int4, 'user_strategy_configs'::pg_catalog.text)
  ) AS expected_relation(ordinal, expected_name)
  LEFT JOIN pg_catalog.pg_class AS relation ON relation.relname = expected_relation.expected_name
  LEFT JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace
  LEFT JOIN pg_catalog.pg_roles AS owner_role ON owner_role.oid = relation.relowner
 ORDER BY expected_relation.ordinal, relation.oid NULLS FIRST
 LIMIT 11
"#;

const PRIMARY_INHERITANCE_SQL: &str = r#"
SELECT inheritance.inhrelid::pg_catalog.int8 AS child_oid,
       inheritance.inhparent::pg_catalog.int8 AS parent_oid
  FROM pg_catalog.pg_inherits AS inheritance
 WHERE inheritance.inhrelid::pg_catalog.int8 = ANY($1::pg_catalog.int8[])
    OR inheritance.inhparent::pg_catalog.int8 = ANY($1::pg_catalog.int8[])
 ORDER BY inheritance.inhrelid, inheritance.inhparent
 LIMIT 1
"#;

const ROOT_HEADERS_SQL: &str = r#"
SELECT expected_function.ordinal::pg_catalog.int4 AS expected_ordinal,
       false::pg_catalog.bool AS expected_name_is_null,
       pg_catalog.octet_length(expected_function.expected_name)::pg_catalog.int8
           AS expected_name_bytes,
       expected_function.expected_name::pg_catalog.text AS expected_name,
       routine.oid::pg_catalog.int8 AS routine_oid,
       namespace.oid::pg_catalog.int8 AS schema_oid,
       (namespace.nspname IS NULL)::pg_catalog.bool AS schema_name_is_null,
       CASE WHEN namespace.nspname IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(namespace.nspname::pg_catalog.text)::pg_catalog.int8
       END::pg_catalog.int8 AS schema_name_bytes,
       CASE WHEN namespace.nspname IS NULL
                  OR pg_catalog.octet_length(namespace.nspname::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text ELSE namespace.nspname::pg_catalog.text END::pg_catalog.text
           AS schema_name,
       (routine.proname IS NULL)::pg_catalog.bool AS actual_name_is_null,
       CASE WHEN routine.proname IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(routine.proname::pg_catalog.text)::pg_catalog.int8
       END::pg_catalog.int8 AS actual_name_bytes,
       CASE WHEN routine.proname IS NULL
                  OR pg_catalog.octet_length(routine.proname::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text ELSE routine.proname::pg_catalog.text END::pg_catalog.text
           AS actual_name,
       routine.proowner::pg_catalog.int8 AS owner_oid,
       (owner_role.rolname IS NULL)::pg_catalog.bool AS owner_name_is_null,
       CASE WHEN owner_role.rolname IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(owner_role.rolname::pg_catalog.text)::pg_catalog.int8
       END::pg_catalog.int8 AS owner_name_bytes,
       CASE WHEN owner_role.rolname IS NULL
                  OR pg_catalog.octet_length(owner_role.rolname::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text ELSE owner_role.rolname::pg_catalog.text END::pg_catalog.text
           AS owner_name,
       (routine.prokind IS NULL)::pg_catalog.bool AS routine_kind_is_null,
       CASE WHEN routine.prokind IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(routine.prokind::pg_catalog.text)::pg_catalog.int8
       END::pg_catalog.int8 AS routine_kind_bytes,
       CASE WHEN routine.prokind IS NULL THEN ''::pg_catalog.text
            ELSE routine.prokind::pg_catalog.text END::pg_catalog.text AS routine_kind,
       routine.pronargs::pg_catalog.int4 AS argument_count,
       pg_catalog.cardinality(routine.proargtypes)::pg_catalog.int4 AS input_type_count,
       routine.pronargdefaults::pg_catalog.int4 AS argument_defaults_count,
       routine.provariadic::pg_catalog.int8 AS variadic_oid,
       routine.proretset::pg_catalog.bool AS returns_set,
       routine.prosecdef::pg_catalog.bool AS security_definer,
       (routine.proallargtypes IS NULL)::pg_catalog.bool AS all_argument_types_is_null,
       (routine.proargmodes IS NULL)::pg_catalog.bool AS argument_modes_is_null,
       (routine.proargnames IS NULL)::pg_catalog.bool AS argument_names_is_null,
       (routine.proargdefaults IS NULL)::pg_catalog.bool AS argument_defaults_is_null,
       language.oid::pg_catalog.int8 AS language_oid,
       (language.lanname IS NULL)::pg_catalog.bool AS language_name_is_null,
       CASE WHEN language.lanname IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(language.lanname::pg_catalog.text)::pg_catalog.int8
       END::pg_catalog.int8 AS language_name_bytes,
       CASE WHEN language.lanname IS NULL
                  OR pg_catalog.octet_length(language.lanname::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text ELSE language.lanname::pg_catalog.text END::pg_catalog.text
           AS language_name,
       return_type.oid::pg_catalog.int8 AS return_type_oid,
       return_namespace.oid::pg_catalog.int8 AS return_namespace_oid,
       (return_namespace.nspname IS NULL)::pg_catalog.bool AS return_namespace_name_is_null,
       CASE WHEN return_namespace.nspname IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(
                return_namespace.nspname::pg_catalog.text
            )::pg_catalog.int8
       END::pg_catalog.int8 AS return_namespace_name_bytes,
       CASE WHEN return_namespace.nspname IS NULL
                  OR pg_catalog.octet_length(return_namespace.nspname::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text ELSE return_namespace.nspname::pg_catalog.text
       END::pg_catalog.text AS return_namespace_name,
       (return_type.typname IS NULL)::pg_catalog.bool AS return_type_name_is_null,
       CASE WHEN return_type.typname IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(return_type.typname::pg_catalog.text)::pg_catalog.int8
       END::pg_catalog.int8 AS return_type_name_bytes,
       CASE WHEN return_type.typname IS NULL
                  OR pg_catalog.octet_length(return_type.typname::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text ELSE return_type.typname::pg_catalog.text END::pg_catalog.text
           AS return_type_name,
       (routine.proconfig IS NULL)::pg_catalog.bool AS config_is_null,
       pg_catalog.array_ndims(routine.proconfig)::pg_catalog.int4 AS config_dimensions,
       pg_catalog.array_lower(routine.proconfig, 1)::pg_catalog.int4 AS config_lower_bound,
       pg_catalog.cardinality(routine.proconfig)::pg_catalog.int4 AS config_cardinality
  FROM (VALUES
      (1::pg_catalog.int4, 'assert_candidate_publication_settlement'::pg_catalog.text),
      (2::pg_catalog.int4, 'daily_0060_attempt_guard'::pg_catalog.text),
      (3::pg_catalog.int4, 'daily_0060_config_epoch'::pg_catalog.text),
      (4::pg_catalog.int4, 'daily_0060_consistency'::pg_catalog.text),
      (5::pg_catalog.int4, 'daily_0060_job_guard'::pg_catalog.text),
      (6::pg_catalog.int4, 'jobs_protect_owner_beta_recommendation_lineage'::pg_catalog.text),
      (7::pg_catalog.int4, 'jobs_reject_candidate_scheduled_mutation'::pg_catalog.text),
      (8::pg_catalog.int4, 'jobs_reject_scheduled_recommendation_mutation'::pg_catalog.text),
      (9::pg_catalog.int4, 'owner_beta_simulation_attempt_guard'::pg_catalog.text),
      (10::pg_catalog.int4, 'owner_beta_simulation_deferred_guard'::pg_catalog.text),
      (11::pg_catalog.int4, 'owner_beta_simulation_jobs_insert_guard'::pg_catalog.text),
      (12::pg_catalog.int4, 'owner_beta_simulation_jobs_lineage_guard'::pg_catalog.text),
      (13::pg_catalog.int4, 'owner_beta_simulation_jobs_validate_insert_payload'::pg_catalog.text),
      (14::pg_catalog.int4, 'owner_beta_simulation_pairs_validate_insert'::pg_catalog.text),
      (15::pg_catalog.int4, 'owner_beta_simulation_runs_lock_config_on_success'::pg_catalog.text),
      (16::pg_catalog.int4, 'owner_beta_simulation_runs_validate_binding'::pg_catalog.text),
      (17::pg_catalog.int4, 'sync_recommendation_run_from_terminal_job'::pg_catalog.text)
  ) AS expected_function(ordinal, expected_name)
  LEFT JOIN pg_catalog.pg_proc AS routine ON routine.proname = expected_function.expected_name
  LEFT JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = routine.pronamespace
  LEFT JOIN pg_catalog.pg_roles AS owner_role ON owner_role.oid = routine.proowner
  LEFT JOIN pg_catalog.pg_language AS language ON language.oid = routine.prolang
  LEFT JOIN pg_catalog.pg_type AS return_type ON return_type.oid = routine.prorettype
  LEFT JOIN pg_catalog.pg_namespace AS return_namespace
    ON return_namespace.oid = return_type.typnamespace
 ORDER BY expected_function.ordinal, routine.oid NULLS FIRST
 LIMIT 18
"#;

const ROOT_SETTINGS_SQL: &str = r#"
SELECT routine.oid::pg_catalog.int8 AS routine_oid,
       setting.ordinality::pg_catalog.int4 AS setting_ordinal,
       (setting.value IS NULL)::pg_catalog.bool AS setting_is_null,
       CASE WHEN setting.value IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(setting.value::pg_catalog.text)::pg_catalog.int8
       END::pg_catalog.int8 AS setting_bytes,
       CASE WHEN setting.value IS NULL
                  OR pg_catalog.octet_length(setting.value::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text ELSE setting.value::pg_catalog.text END::pg_catalog.text
           AS setting_value
  FROM pg_catalog.pg_proc AS routine
  LEFT JOIN LATERAL pg_catalog.unnest(
      CASE WHEN routine.proconfig IS NOT NULL
                  AND pg_catalog.array_ndims(routine.proconfig) = 1
                  AND pg_catalog.array_lower(routine.proconfig, 1) = 1
                  AND pg_catalog.cardinality(routine.proconfig) IN (1, 3)
           THEN routine.proconfig ELSE ARRAY[]::pg_catalog.text[] END
  ) WITH ORDINALITY AS setting(value, ordinality) ON true
 WHERE routine.oid::pg_catalog.int8 = ANY($1::pg_catalog.int8[])
 ORDER BY routine.oid, setting.ordinality NULLS FIRST
 LIMIT 26
"#;

macro_rules! trigger_query {
    ($predicate:literal, $limit:literal) => {
        concat!(
            r#"SELECT trigger.oid::pg_catalog.int8 AS trigger_oid,
       trigger.tgrelid::pg_catalog.int8 AS relation_oid,
       relation_namespace.oid::pg_catalog.int8 AS schema_oid,
       (relation_namespace.nspname IS NULL)::pg_catalog.bool AS schema_name_is_null,
       CASE WHEN relation_namespace.nspname IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(
                relation_namespace.nspname::pg_catalog.text
            )::pg_catalog.int8
       END::pg_catalog.int8 AS schema_name_bytes,
       CASE WHEN relation_namespace.nspname IS NULL
                  OR pg_catalog.octet_length(relation_namespace.nspname::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text ELSE relation_namespace.nspname::pg_catalog.text
       END::pg_catalog.text AS schema_name,
       (relation.relname IS NULL)::pg_catalog.bool AS relation_name_is_null,
       CASE WHEN relation.relname IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(relation.relname::pg_catalog.text)::pg_catalog.int8
       END::pg_catalog.int8 AS relation_name_bytes,
       CASE WHEN relation.relname IS NULL
                  OR pg_catalog.octet_length(relation.relname::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text ELSE relation.relname::pg_catalog.text END::pg_catalog.text
           AS relation_name,
       relation.relowner::pg_catalog.int8 AS relation_owner_oid,
       (relation_owner.rolname IS NULL)::pg_catalog.bool AS relation_owner_name_is_null,
       CASE WHEN relation_owner.rolname IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(relation_owner.rolname::pg_catalog.text)::pg_catalog.int8
       END::pg_catalog.int8 AS relation_owner_name_bytes,
       CASE WHEN relation_owner.rolname IS NULL
                  OR pg_catalog.octet_length(relation_owner.rolname::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text ELSE relation_owner.rolname::pg_catalog.text
       END::pg_catalog.text AS relation_owner_name,
       (trigger.tgname IS NULL)::pg_catalog.bool AS trigger_name_is_null,
       CASE WHEN trigger.tgname IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(trigger.tgname::pg_catalog.text)::pg_catalog.int8
       END::pg_catalog.int8 AS trigger_name_bytes,
       CASE WHEN trigger.tgname IS NULL
                  OR pg_catalog.octet_length(trigger.tgname::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text ELSE trigger.tgname::pg_catalog.text END::pg_catalog.text
           AS trigger_name,
       trigger.tgfoid::pg_catalog.int8 AS function_oid,
       function_namespace.oid::pg_catalog.int8 AS function_schema_oid,
       (function_namespace.nspname IS NULL)::pg_catalog.bool AS function_schema_name_is_null,
       CASE WHEN function_namespace.nspname IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(
                function_namespace.nspname::pg_catalog.text
            )::pg_catalog.int8
       END::pg_catalog.int8 AS function_schema_name_bytes,
       CASE WHEN function_namespace.nspname IS NULL
                  OR pg_catalog.octet_length(function_namespace.nspname::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text ELSE function_namespace.nspname::pg_catalog.text
       END::pg_catalog.text AS function_schema_name,
       (routine.proname IS NULL)::pg_catalog.bool AS function_name_is_null,
       CASE WHEN routine.proname IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(routine.proname::pg_catalog.text)::pg_catalog.int8
       END::pg_catalog.int8 AS function_name_bytes,
       CASE WHEN routine.proname IS NULL
                  OR pg_catalog.octet_length(routine.proname::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text ELSE routine.proname::pg_catalog.text END::pg_catalog.text
           AS function_name,
       routine.proowner::pg_catalog.int8 AS function_owner_oid,
       (function_owner.rolname IS NULL)::pg_catalog.bool AS function_owner_name_is_null,
       CASE WHEN function_owner.rolname IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(function_owner.rolname::pg_catalog.text)::pg_catalog.int8
       END::pg_catalog.int8 AS function_owner_name_bytes,
       CASE WHEN function_owner.rolname IS NULL
                  OR pg_catalog.octet_length(function_owner.rolname::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text ELSE function_owner.rolname::pg_catalog.text
       END::pg_catalog.text AS function_owner_name,
       trigger.tgisinternal::pg_catalog.bool AS is_internal,
       (trigger.tgenabled IS NULL)::pg_catalog.bool AS enabled_is_null,
       CASE WHEN trigger.tgenabled IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(trigger.tgenabled::pg_catalog.text)::pg_catalog.int8
       END::pg_catalog.int8 AS enabled_bytes,
       CASE WHEN trigger.tgenabled IS NULL THEN ''::pg_catalog.text
            ELSE trigger.tgenabled::pg_catalog.text END::pg_catalog.text AS enabled,
       trigger.tgparentid::pg_catalog.int8 AS parent_oid,
       trigger.tgtype::pg_catalog.int4 AS event_bits,
       trigger.tgnargs::pg_catalog.int4 AS argument_count,
       pg_catalog.array_ndims(trigger.tgattr::pg_catalog.int2[])::pg_catalog.int4
           AS attribute_dimensions,
       pg_catalog.array_lower(trigger.tgattr::pg_catalog.int2[], 1)::pg_catalog.int4
           AS attribute_lower_bound,
       pg_catalog.array_upper(trigger.tgattr::pg_catalog.int2[], 1)::pg_catalog.int4
           AS attribute_upper_bound,
       pg_catalog.cardinality(trigger.tgattr::pg_catalog.int2[])::pg_catalog.int4
           AS attribute_cardinality,
       (trigger.tgargs IS NULL)::pg_catalog.bool AS trigger_args_is_null,
       CASE WHEN trigger.tgargs IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(trigger.tgargs)::pg_catalog.int8
       END::pg_catalog.int8 AS trigger_args_bytes,
       CASE WHEN trigger.tgargs IS NULL THEN ''::pg_catalog.bytea
            ELSE pg_catalog.substring(trigger.tgargs FROM 1 FOR 1) END::pg_catalog.bytea
           AS trigger_args_prefix,
       (trigger.tgqual IS NULL)::pg_catalog.bool AS when_is_null,
       trigger.tgconstrrelid::pg_catalog.int8 AS constraint_relation_oid,
       trigger.tgconstraint::pg_catalog.int8 AS constraint_oid,
       trigger.tgnattr::pg_catalog.int4 AS update_attribute_count,
       (trigger.tgoldtable IS NULL)::pg_catalog.bool AS old_transition_name_is_null,
       (trigger.tgnewtable IS NULL)::pg_catalog.bool AS new_transition_name_is_null,
       trigger.tgdeferrable::pg_catalog.bool AS is_deferrable,
       trigger.tginitdeferred::pg_catalog.bool AS initially_deferred
  FROM pg_catalog.pg_trigger AS trigger
  LEFT JOIN pg_catalog.pg_class AS relation ON relation.oid = trigger.tgrelid
  LEFT JOIN pg_catalog.pg_namespace AS relation_namespace
    ON relation_namespace.oid = relation.relnamespace
  LEFT JOIN pg_catalog.pg_roles AS relation_owner ON relation_owner.oid = relation.relowner
  LEFT JOIN pg_catalog.pg_proc AS routine ON routine.oid = trigger.tgfoid
  LEFT JOIN pg_catalog.pg_namespace AS function_namespace
    ON function_namespace.oid = routine.pronamespace
  LEFT JOIN pg_catalog.pg_roles AS function_owner ON function_owner.oid = routine.proowner
"#,
            $predicate,
            " ORDER BY trigger.tgrelid, trigger.tgname, trigger.oid LIMIT ",
            $limit
        )
    };
}

const FORWARD_TRIGGERS_SQL: &str = trigger_query!(
    r#" WHERE trigger.tgrelid::pg_catalog.int8 = ANY($1::pg_catalog.int8[])
   AND (
       NOT trigger.tgisinternal
       OR trigger.tgname = ANY(ARRAY[
           'jobs_protect_scheduled_recommendation_lineage',
           'jobs_sync_recommendation_terminal_run',
           'jobs_protect_candidate_scheduled_lineage',
           'stock_analysis_run_requires_settlement',
           'candidate_job_requires_publication',
           'candidate_attempt_requires_publication',
           'jobs_protect_owner_beta_recommendation_lineage',
           'daily_0060_config_epoch',
           'daily_0060_job_guard',
           'daily_0060_attempt_guard',
           'daily_0060_consistency',
           'owner_beta_simulation_jobs_insert_guard',
           'owner_beta_simulation_jobs_lineage_guard',
           'owner_beta_simulation_runs_validate_binding',
           'owner_beta_simulation_runs_lock_config_on_success',
           'owner_beta_simulation_jobs_validate_insert_payload',
           'owner_beta_simulation_pairs_validate_insert',
           'owner_beta_simulation_attempt_guard',
           'owner_beta_simulation_jobs_deferred_guard',
           'owner_beta_simulation_runs_deferred_guard',
           'owner_beta_simulation_pairs_deferred_guard',
           'owner_beta_simulation_attempts_deferred_guard'
       ]::pg_catalog.name[])
       OR trigger.tgfoid::pg_catalog.int8 = ANY($2::pg_catalog.int8[])
       OR pg_catalog.left(
           routine.proname::pg_catalog.text,
           pg_catalog.length('owner_beta_simulation_'::pg_catalog.text)
       ) = 'owner_beta_simulation_'
   )"#,
    "22"
);

const ADJACENT_CONFIG_HOOK_SQL: &str = trigger_query!(
    r#" WHERE trigger.tgrelid::pg_catalog.int8 = $1::pg_catalog.int8
   AND trigger.tgname = 'daily_0060_config_epoch'"#,
    "2"
);

const REVERSE_PROTECTED_HOOKS_SQL: &str = trigger_query!(
    r#" WHERE trigger.tgfoid::pg_catalog.int8 = ANY($1::pg_catalog.int8[])
    OR trigger.tgname = ANY(ARRAY[
           'jobs_protect_scheduled_recommendation_lineage',
           'jobs_sync_recommendation_terminal_run',
           'jobs_protect_candidate_scheduled_lineage',
           'stock_analysis_run_requires_settlement',
           'candidate_job_requires_publication',
           'candidate_attempt_requires_publication',
           'jobs_protect_owner_beta_recommendation_lineage',
           'daily_0060_config_epoch',
           'daily_0060_job_guard',
           'daily_0060_attempt_guard',
           'daily_0060_consistency',
           'owner_beta_simulation_jobs_insert_guard',
           'owner_beta_simulation_jobs_lineage_guard',
           'owner_beta_simulation_runs_validate_binding',
           'owner_beta_simulation_runs_lock_config_on_success',
           'owner_beta_simulation_jobs_validate_insert_payload',
           'owner_beta_simulation_pairs_validate_insert',
           'owner_beta_simulation_attempt_guard',
           'owner_beta_simulation_jobs_deferred_guard',
           'owner_beta_simulation_runs_deferred_guard',
           'owner_beta_simulation_pairs_deferred_guard',
           'owner_beta_simulation_attempts_deferred_guard'
       ]::pg_catalog.name[])
    OR pg_catalog.left(
           routine.proname::pg_catalog.text,
           pg_catalog.length('owner_beta_simulation_'::pg_catalog.text)
       ) = 'owner_beta_simulation_'"#,
    "28"
);

const UPDATE_OF_ATTRIBUTES_SQL: &str = r#"
SELECT trigger.oid::pg_catalog.int8 AS trigger_oid,
       trigger.tgrelid::pg_catalog.int8 AS relation_oid,
       attributes.ordinality::pg_catalog.int8 AS attribute_ordinal,
       attributes.attribute_number::pg_catalog.int4 AS attribute_number,
       (attribute.attname IS NULL)::pg_catalog.bool AS attribute_name_is_null,
       CASE WHEN attribute.attname IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(attribute.attname::pg_catalog.text)::pg_catalog.int8
       END::pg_catalog.int8 AS attribute_name_bytes,
       CASE WHEN attribute.attname IS NULL
                  OR pg_catalog.octet_length(attribute.attname::pg_catalog.text) > 1025
            THEN ''::pg_catalog.text ELSE attribute.attname::pg_catalog.text END::pg_catalog.text
           AS attribute_name,
       attribute.attrelid::pg_catalog.int8 AS attribute_relation_oid,
       attribute.attisdropped::pg_catalog.bool AS attribute_is_dropped
  FROM pg_catalog.pg_trigger AS trigger
  LEFT JOIN LATERAL pg_catalog.unnest(
      CASE WHEN pg_catalog.array_ndims(trigger.tgattr::pg_catalog.int2[]) = 1
                  AND pg_catalog.array_lower(trigger.tgattr::pg_catalog.int2[], 1) = 0
                  AND pg_catalog.array_upper(trigger.tgattr::pg_catalog.int2[], 1) = 0
                  AND pg_catalog.cardinality(trigger.tgattr::pg_catalog.int2[]) = 1
           THEN trigger.tgattr::pg_catalog.int2[]
           ELSE ARRAY[]::pg_catalog.int2[] END
  ) WITH ORDINALITY AS attributes(attribute_number, ordinality) ON true
  LEFT JOIN pg_catalog.pg_attribute AS attribute
    ON attribute.attrelid = trigger.tgrelid
   AND attribute.attnum = attributes.attribute_number
 WHERE trigger.oid::pg_catalog.int8 = ANY($1::pg_catalog.int8[])
 ORDER BY trigger.oid, attributes.ordinality NULLS FIRST
 LIMIT 3
"#;

const USER_TRIGGER_CONSTRAINTS_SQL: &str = r#"
SELECT trigger.oid::pg_catalog.int8 AS trigger_oid,
       trigger.tgrelid::pg_catalog.int8 AS relation_oid,
       trigger.tgconstraint::pg_catalog.int8 AS trigger_constraint_oid,
       constraint_row.oid::pg_catalog.int8 AS constraint_oid,
       constraint_row.conrelid::pg_catalog.int8 AS constraint_relation_oid,
       constraint_row.connamespace::pg_catalog.int8 AS constraint_schema_oid,
       (constraint_namespace.nspname IS NULL)::pg_catalog.bool AS constraint_schema_name_is_null,
       CASE WHEN constraint_namespace.nspname IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(
                constraint_namespace.nspname::pg_catalog.text
            )::pg_catalog.int8
       END::pg_catalog.int8 AS constraint_schema_name_bytes,
       CASE WHEN constraint_namespace.nspname IS NULL
                  OR pg_catalog.octet_length(
                      constraint_namespace.nspname::pg_catalog.text
                  ) > 1025
            THEN ''::pg_catalog.text ELSE constraint_namespace.nspname::pg_catalog.text
       END::pg_catalog.text AS constraint_schema_name,
       (constraint_row.contype IS NULL)::pg_catalog.bool AS constraint_kind_is_null,
       CASE WHEN constraint_row.contype IS NULL THEN NULL::pg_catalog.int8
            ELSE pg_catalog.octet_length(
                constraint_row.contype::pg_catalog.text
            )::pg_catalog.int8
       END::pg_catalog.int8 AS constraint_kind_bytes,
       CASE WHEN constraint_row.contype IS NULL THEN ''::pg_catalog.text
            ELSE constraint_row.contype::pg_catalog.text END::pg_catalog.text AS constraint_kind,
       constraint_row.condeferrable::pg_catalog.bool AS is_deferrable,
       constraint_row.condeferred::pg_catalog.bool AS initially_deferred,
       constraint_row.contypid::pg_catalog.int8 AS constraint_type_oid,
       constraint_row.conindid::pg_catalog.int8 AS index_oid,
       constraint_row.confrelid::pg_catalog.int8 AS foreign_relation_oid
  FROM pg_catalog.pg_trigger AS trigger
  LEFT JOIN pg_catalog.pg_constraint AS constraint_row
    ON constraint_row.oid = trigger.tgconstraint
  LEFT JOIN pg_catalog.pg_namespace AS constraint_namespace
    ON constraint_namespace.oid = constraint_row.connamespace
 WHERE trigger.oid::pg_catalog.int8 = ANY($1::pg_catalog.int8[])
 ORDER BY trigger.oid
 LIMIT 14
"#;

#[derive(Clone, Copy)]
struct RelationSpec {
    name: &'static str,
    primary: bool,
    adjacent_config: bool,
}

const RELATION_SPECS: [RelationSpec; RELATION_IDENTITY_ACCEPT_LIMIT] = [
    RelationSpec {
        name: "job_attempts",
        primary: true,
        adjacent_config: false,
    },
    RelationSpec {
        name: "jobs",
        primary: true,
        adjacent_config: false,
    },
    RelationSpec {
        name: "owner_beta_daily_publication_claims",
        primary: false,
        adjacent_config: false,
    },
    RelationSpec {
        name: "owner_beta_daily_reservations",
        primary: false,
        adjacent_config: false,
    },
    RelationSpec {
        name: "owner_beta_recommendation_items",
        primary: false,
        adjacent_config: false,
    },
    RelationSpec {
        name: "owner_beta_recommendation_runs",
        primary: false,
        adjacent_config: false,
    },
    RelationSpec {
        name: "owner_beta_simulation_result_pairs",
        primary: true,
        adjacent_config: false,
    },
    RelationSpec {
        name: "owner_beta_simulation_runs",
        primary: true,
        adjacent_config: false,
    },
    RelationSpec {
        name: "stock_analysis_runs",
        primary: false,
        adjacent_config: false,
    },
    RelationSpec {
        name: "user_strategy_configs",
        primary: false,
        adjacent_config: true,
    },
];

#[derive(Clone, Copy)]
struct RootSpec {
    name: &'static str,
    security_definer: bool,
    search_path: &'static str,
    daily_timeouts: bool,
}

const ROOT_SPECS: [RootSpec; ROOT_HEADER_ACCEPT_LIMIT] = [
    RootSpec {
        name: "assert_candidate_publication_settlement",
        security_definer: false,
        search_path: "pg_catalog",
        daily_timeouts: false,
    },
    RootSpec {
        name: "daily_0060_attempt_guard",
        security_definer: true,
        search_path: "pg_catalog,pg_temp",
        daily_timeouts: true,
    },
    RootSpec {
        name: "daily_0060_config_epoch",
        security_definer: true,
        search_path: "pg_catalog,pg_temp",
        daily_timeouts: true,
    },
    RootSpec {
        name: "daily_0060_consistency",
        security_definer: true,
        search_path: "pg_catalog,pg_temp",
        daily_timeouts: true,
    },
    RootSpec {
        name: "daily_0060_job_guard",
        security_definer: true,
        search_path: "pg_catalog,pg_temp",
        daily_timeouts: true,
    },
    RootSpec {
        name: "jobs_protect_owner_beta_recommendation_lineage",
        security_definer: false,
        search_path: "pg_catalog",
        daily_timeouts: false,
    },
    RootSpec {
        name: "jobs_reject_candidate_scheduled_mutation",
        security_definer: false,
        search_path: "pg_catalog",
        daily_timeouts: false,
    },
    RootSpec {
        name: "jobs_reject_scheduled_recommendation_mutation",
        security_definer: false,
        search_path: "pg_catalog,public",
        daily_timeouts: false,
    },
    RootSpec {
        name: "owner_beta_simulation_attempt_guard",
        security_definer: true,
        search_path: "pg_catalog,pg_temp",
        daily_timeouts: false,
    },
    RootSpec {
        name: "owner_beta_simulation_deferred_guard",
        security_definer: true,
        search_path: "pg_catalog,pg_temp",
        daily_timeouts: false,
    },
    RootSpec {
        name: "owner_beta_simulation_jobs_insert_guard",
        security_definer: true,
        search_path: "pg_catalog,pg_temp",
        daily_timeouts: false,
    },
    RootSpec {
        name: "owner_beta_simulation_jobs_lineage_guard",
        security_definer: true,
        search_path: "pg_catalog,pg_temp",
        daily_timeouts: false,
    },
    RootSpec {
        name: "owner_beta_simulation_jobs_validate_insert_payload",
        security_definer: true,
        search_path: "pg_catalog,pg_temp",
        daily_timeouts: false,
    },
    RootSpec {
        name: "owner_beta_simulation_pairs_validate_insert",
        security_definer: true,
        search_path: "pg_catalog,pg_temp",
        daily_timeouts: false,
    },
    RootSpec {
        name: "owner_beta_simulation_runs_lock_config_on_success",
        security_definer: true,
        search_path: "pg_catalog,pg_temp",
        daily_timeouts: false,
    },
    RootSpec {
        name: "owner_beta_simulation_runs_validate_binding",
        security_definer: true,
        search_path: "pg_catalog,pg_temp",
        daily_timeouts: false,
    },
    RootSpec {
        name: "sync_recommendation_run_from_terminal_job",
        security_definer: true,
        search_path: "pg_catalog,pg_temp",
        daily_timeouts: false,
    },
];

#[derive(Clone, Copy)]
struct AttachmentSpec {
    relation: &'static str,
    name: &'static str,
    function: &'static str,
    event_bits: i32,
    update_of_status: bool,
    constraint: bool,
}

const ATTACHMENT_SPECS: [AttachmentSpec; REVERSE_HOOK_ACCEPT_LIMIT] = [
    AttachmentSpec {
        relation: "jobs",
        name: "jobs_protect_scheduled_recommendation_lineage",
        function: "jobs_reject_scheduled_recommendation_mutation",
        event_bits: 31,
        update_of_status: false,
        constraint: false,
    },
    AttachmentSpec {
        relation: "jobs",
        name: "jobs_sync_recommendation_terminal_run",
        function: "sync_recommendation_run_from_terminal_job",
        event_bits: 17,
        update_of_status: true,
        constraint: false,
    },
    AttachmentSpec {
        relation: "jobs",
        name: "jobs_protect_candidate_scheduled_lineage",
        function: "jobs_reject_candidate_scheduled_mutation",
        event_bits: 31,
        update_of_status: false,
        constraint: false,
    },
    AttachmentSpec {
        relation: "stock_analysis_runs",
        name: "stock_analysis_run_requires_settlement",
        function: "assert_candidate_publication_settlement",
        event_bits: 29,
        update_of_status: false,
        constraint: true,
    },
    AttachmentSpec {
        relation: "jobs",
        name: "candidate_job_requires_publication",
        function: "assert_candidate_publication_settlement",
        event_bits: 29,
        update_of_status: false,
        constraint: true,
    },
    AttachmentSpec {
        relation: "job_attempts",
        name: "candidate_attempt_requires_publication",
        function: "assert_candidate_publication_settlement",
        event_bits: 29,
        update_of_status: false,
        constraint: true,
    },
    AttachmentSpec {
        relation: "jobs",
        name: "jobs_protect_owner_beta_recommendation_lineage",
        function: "jobs_protect_owner_beta_recommendation_lineage",
        event_bits: 27,
        update_of_status: false,
        constraint: false,
    },
    AttachmentSpec {
        relation: "user_strategy_configs",
        name: "daily_0060_config_epoch",
        function: "daily_0060_config_epoch",
        event_bits: 23,
        update_of_status: false,
        constraint: false,
    },
    AttachmentSpec {
        relation: "jobs",
        name: "daily_0060_job_guard",
        function: "daily_0060_job_guard",
        event_bits: 31,
        update_of_status: false,
        constraint: false,
    },
    AttachmentSpec {
        relation: "job_attempts",
        name: "daily_0060_attempt_guard",
        function: "daily_0060_attempt_guard",
        event_bits: 31,
        update_of_status: false,
        constraint: false,
    },
    AttachmentSpec {
        relation: "jobs",
        name: "daily_0060_consistency",
        function: "daily_0060_consistency",
        event_bits: 29,
        update_of_status: false,
        constraint: true,
    },
    AttachmentSpec {
        relation: "job_attempts",
        name: "daily_0060_consistency",
        function: "daily_0060_consistency",
        event_bits: 29,
        update_of_status: false,
        constraint: true,
    },
    AttachmentSpec {
        relation: "owner_beta_recommendation_runs",
        name: "daily_0060_consistency",
        function: "daily_0060_consistency",
        event_bits: 29,
        update_of_status: false,
        constraint: true,
    },
    AttachmentSpec {
        relation: "owner_beta_recommendation_items",
        name: "daily_0060_consistency",
        function: "daily_0060_consistency",
        event_bits: 29,
        update_of_status: false,
        constraint: true,
    },
    AttachmentSpec {
        relation: "owner_beta_daily_reservations",
        name: "daily_0060_consistency",
        function: "daily_0060_consistency",
        event_bits: 29,
        update_of_status: false,
        constraint: true,
    },
    AttachmentSpec {
        relation: "owner_beta_daily_publication_claims",
        name: "daily_0060_consistency",
        function: "daily_0060_consistency",
        event_bits: 29,
        update_of_status: false,
        constraint: true,
    },
    AttachmentSpec {
        relation: "jobs",
        name: "owner_beta_simulation_jobs_insert_guard",
        function: "owner_beta_simulation_jobs_insert_guard",
        event_bits: 7,
        update_of_status: false,
        constraint: false,
    },
    AttachmentSpec {
        relation: "jobs",
        name: "owner_beta_simulation_jobs_lineage_guard",
        function: "owner_beta_simulation_jobs_lineage_guard",
        event_bits: 27,
        update_of_status: false,
        constraint: false,
    },
    AttachmentSpec {
        relation: "owner_beta_simulation_runs",
        name: "owner_beta_simulation_runs_validate_binding",
        function: "owner_beta_simulation_runs_validate_binding",
        event_bits: 31,
        update_of_status: false,
        constraint: false,
    },
    AttachmentSpec {
        relation: "owner_beta_simulation_runs",
        name: "owner_beta_simulation_runs_lock_config_on_success",
        function: "owner_beta_simulation_runs_lock_config_on_success",
        event_bits: 19,
        update_of_status: true,
        constraint: false,
    },
    AttachmentSpec {
        relation: "jobs",
        name: "owner_beta_simulation_jobs_validate_insert_payload",
        function: "owner_beta_simulation_jobs_validate_insert_payload",
        event_bits: 7,
        update_of_status: false,
        constraint: false,
    },
    AttachmentSpec {
        relation: "owner_beta_simulation_result_pairs",
        name: "owner_beta_simulation_pairs_validate_insert",
        function: "owner_beta_simulation_pairs_validate_insert",
        event_bits: 31,
        update_of_status: false,
        constraint: false,
    },
    AttachmentSpec {
        relation: "job_attempts",
        name: "owner_beta_simulation_attempt_guard",
        function: "owner_beta_simulation_attempt_guard",
        event_bits: 31,
        update_of_status: false,
        constraint: false,
    },
    AttachmentSpec {
        relation: "jobs",
        name: "owner_beta_simulation_jobs_deferred_guard",
        function: "owner_beta_simulation_deferred_guard",
        event_bits: 29,
        update_of_status: false,
        constraint: true,
    },
    AttachmentSpec {
        relation: "owner_beta_simulation_runs",
        name: "owner_beta_simulation_runs_deferred_guard",
        function: "owner_beta_simulation_deferred_guard",
        event_bits: 29,
        update_of_status: false,
        constraint: true,
    },
    AttachmentSpec {
        relation: "owner_beta_simulation_result_pairs",
        name: "owner_beta_simulation_pairs_deferred_guard",
        function: "owner_beta_simulation_deferred_guard",
        event_bits: 29,
        update_of_status: false,
        constraint: true,
    },
    AttachmentSpec {
        relation: "job_attempts",
        name: "owner_beta_simulation_attempts_deferred_guard",
        function: "owner_beta_simulation_deferred_guard",
        event_bits: 29,
        update_of_status: false,
        constraint: true,
    },
];

const PROTECTED_TRIGGER_NAMES: [&str; 22] = [
    "jobs_protect_scheduled_recommendation_lineage",
    "jobs_sync_recommendation_terminal_run",
    "jobs_protect_candidate_scheduled_lineage",
    "stock_analysis_run_requires_settlement",
    "candidate_job_requires_publication",
    "candidate_attempt_requires_publication",
    "jobs_protect_owner_beta_recommendation_lineage",
    "daily_0060_config_epoch",
    "daily_0060_job_guard",
    "daily_0060_attempt_guard",
    "daily_0060_consistency",
    "owner_beta_simulation_jobs_insert_guard",
    "owner_beta_simulation_jobs_lineage_guard",
    "owner_beta_simulation_runs_validate_binding",
    "owner_beta_simulation_runs_lock_config_on_success",
    "owner_beta_simulation_jobs_validate_insert_payload",
    "owner_beta_simulation_pairs_validate_insert",
    "owner_beta_simulation_attempt_guard",
    "owner_beta_simulation_jobs_deferred_guard",
    "owner_beta_simulation_runs_deferred_guard",
    "owner_beta_simulation_pairs_deferred_guard",
    "owner_beta_simulation_attempts_deferred_guard",
];

const TRIGGER_ROW: [PgCellKind; 32] = [
    PgCellKind::I64("trigger_oid"),
    PgCellKind::I64("relation_oid"),
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
    PgCellKind::I64("relation_owner_oid"),
    PgCellKind::Text {
        null_alias: "relation_owner_name_is_null",
        bytes_alias: "relation_owner_name_bytes",
        value_alias: "relation_owner_name",
    },
    PgCellKind::Text {
        null_alias: "trigger_name_is_null",
        bytes_alias: "trigger_name_bytes",
        value_alias: "trigger_name",
    },
    PgCellKind::I64("function_oid"),
    PgCellKind::I64("function_schema_oid"),
    PgCellKind::Text {
        null_alias: "function_schema_name_is_null",
        bytes_alias: "function_schema_name_bytes",
        value_alias: "function_schema_name",
    },
    PgCellKind::Text {
        null_alias: "function_name_is_null",
        bytes_alias: "function_name_bytes",
        value_alias: "function_name",
    },
    PgCellKind::I64("function_owner_oid"),
    PgCellKind::Text {
        null_alias: "function_owner_name_is_null",
        bytes_alias: "function_owner_name_bytes",
        value_alias: "function_owner_name",
    },
    PgCellKind::Bool("is_internal"),
    PgCellKind::Text {
        null_alias: "enabled_is_null",
        bytes_alias: "enabled_bytes",
        value_alias: "enabled",
    },
    PgCellKind::I64("parent_oid"),
    PgCellKind::I32("event_bits"),
    PgCellKind::I32("argument_count"),
    PgCellKind::OptionalI32("attribute_dimensions"),
    PgCellKind::OptionalI32("attribute_lower_bound"),
    PgCellKind::OptionalI32("attribute_upper_bound"),
    PgCellKind::OptionalI32("attribute_cardinality"),
    PgCellKind::Bytes {
        null_alias: "trigger_args_is_null",
        bytes_alias: "trigger_args_bytes",
        value_alias: "trigger_args_prefix",
    },
    PgCellKind::Bool("when_is_null"),
    PgCellKind::I64("constraint_relation_oid"),
    PgCellKind::I64("constraint_oid"),
    PgCellKind::I32("update_attribute_count"),
    PgCellKind::Bool("old_transition_name_is_null"),
    PgCellKind::Bool("new_transition_name_is_null"),
    PgCellKind::Bool("is_deferrable"),
    PgCellKind::Bool("initially_deferred"),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PgCellKind {
    Bool(&'static str),
    OptionalBool(&'static str),
    I32(&'static str),
    OptionalI32(&'static str),
    I64(&'static str),
    OptionalI64(&'static str),
    Text {
        null_alias: &'static str,
        bytes_alias: &'static str,
        value_alias: &'static str,
    },
    Bytes {
        null_alias: &'static str,
        bytes_alias: &'static str,
        value_alias: &'static str,
    },
}

const SETUP_ROW: [PgCellKind; 3] = [
    PgCellKind::Text {
        null_alias: "search_path_is_null",
        bytes_alias: "search_path_bytes",
        value_alias: "search_path",
    },
    PgCellKind::Text {
        null_alias: "statement_timeout_is_null",
        bytes_alias: "statement_timeout_bytes",
        value_alias: "statement_timeout",
    },
    PgCellKind::Text {
        null_alias: "lock_timeout_is_null",
        bytes_alias: "lock_timeout_bytes",
        value_alias: "lock_timeout",
    },
];

const CONTEXT_ROW: [PgCellKind; 25] = [
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
    PgCellKind::Bool("worker_inherits_roles"),
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
    PgCellKind::Text {
        null_alias: "search_path_is_null",
        bytes_alias: "search_path_bytes",
        value_alias: "search_path",
    },
    PgCellKind::Text {
        null_alias: "statement_timeout_is_null",
        bytes_alias: "statement_timeout_bytes",
        value_alias: "statement_timeout",
    },
    PgCellKind::Text {
        null_alias: "lock_timeout_is_null",
        bytes_alias: "lock_timeout_bytes",
        value_alias: "lock_timeout",
    },
];

const ROLE_ROW: [PgCellKind; 4] = [
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

const MEMBERSHIP_ROW: [PgCellKind; 6] = [
    PgCellKind::I64("member_oid"),
    PgCellKind::I64("role_oid"),
    PgCellKind::I64("grantor_oid"),
    PgCellKind::Bool("admin_option"),
    PgCellKind::Bool("inherit_option"),
    PgCellKind::Bool("set_option"),
];

const RELATION_ROW: [PgCellKind; 11] = [
    PgCellKind::I32("relation_ordinal"),
    PgCellKind::Text {
        null_alias: "expected_name_is_null",
        bytes_alias: "expected_name_bytes",
        value_alias: "expected_name",
    },
    PgCellKind::OptionalI64("relation_oid"),
    PgCellKind::OptionalI64("schema_oid"),
    PgCellKind::Text {
        null_alias: "schema_name_is_null",
        bytes_alias: "schema_name_bytes",
        value_alias: "schema_name",
    },
    PgCellKind::Text {
        null_alias: "actual_name_is_null",
        bytes_alias: "actual_name_bytes",
        value_alias: "actual_name",
    },
    PgCellKind::OptionalI64("owner_oid"),
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
    PgCellKind::OptionalBool("is_partition"),
];

const INHERITANCE_ROW: [PgCellKind; 2] =
    [PgCellKind::I64("child_oid"), PgCellKind::I64("parent_oid")];

const ROOT_HEADER_ROW: [PgCellKind; 29] = [
    PgCellKind::I32("expected_ordinal"),
    PgCellKind::Text {
        null_alias: "expected_name_is_null",
        bytes_alias: "expected_name_bytes",
        value_alias: "expected_name",
    },
    PgCellKind::OptionalI64("routine_oid"),
    PgCellKind::OptionalI64("schema_oid"),
    PgCellKind::Text {
        null_alias: "schema_name_is_null",
        bytes_alias: "schema_name_bytes",
        value_alias: "schema_name",
    },
    PgCellKind::Text {
        null_alias: "actual_name_is_null",
        bytes_alias: "actual_name_bytes",
        value_alias: "actual_name",
    },
    PgCellKind::OptionalI64("owner_oid"),
    PgCellKind::Text {
        null_alias: "owner_name_is_null",
        bytes_alias: "owner_name_bytes",
        value_alias: "owner_name",
    },
    PgCellKind::Text {
        null_alias: "routine_kind_is_null",
        bytes_alias: "routine_kind_bytes",
        value_alias: "routine_kind",
    },
    PgCellKind::OptionalI32("argument_count"),
    PgCellKind::OptionalI32("input_type_count"),
    PgCellKind::OptionalI32("argument_defaults_count"),
    PgCellKind::OptionalI64("variadic_oid"),
    PgCellKind::OptionalBool("returns_set"),
    PgCellKind::OptionalBool("security_definer"),
    PgCellKind::OptionalBool("all_argument_types_is_null"),
    PgCellKind::OptionalBool("argument_modes_is_null"),
    PgCellKind::OptionalBool("argument_names_is_null"),
    PgCellKind::OptionalBool("argument_defaults_is_null"),
    PgCellKind::OptionalI64("language_oid"),
    PgCellKind::Text {
        null_alias: "language_name_is_null",
        bytes_alias: "language_name_bytes",
        value_alias: "language_name",
    },
    PgCellKind::OptionalI64("return_type_oid"),
    PgCellKind::OptionalI64("return_namespace_oid"),
    PgCellKind::Text {
        null_alias: "return_namespace_name_is_null",
        bytes_alias: "return_namespace_name_bytes",
        value_alias: "return_namespace_name",
    },
    PgCellKind::Text {
        null_alias: "return_type_name_is_null",
        bytes_alias: "return_type_name_bytes",
        value_alias: "return_type_name",
    },
    PgCellKind::Bool("config_is_null"),
    PgCellKind::OptionalI32("config_dimensions"),
    PgCellKind::OptionalI32("config_lower_bound"),
    PgCellKind::OptionalI32("config_cardinality"),
];

const ROOT_SETTING_ROW: [PgCellKind; 3] = [
    PgCellKind::I64("routine_oid"),
    PgCellKind::OptionalI32("setting_ordinal"),
    PgCellKind::Text {
        null_alias: "setting_is_null",
        bytes_alias: "setting_bytes",
        value_alias: "setting_value",
    },
];

const UPDATE_ATTRIBUTE_ROW: [PgCellKind; 7] = [
    PgCellKind::I64("trigger_oid"),
    PgCellKind::I64("relation_oid"),
    PgCellKind::OptionalI64("attribute_ordinal"),
    PgCellKind::OptionalI32("attribute_number"),
    PgCellKind::Text {
        null_alias: "attribute_name_is_null",
        bytes_alias: "attribute_name_bytes",
        value_alias: "attribute_name",
    },
    PgCellKind::OptionalI64("attribute_relation_oid"),
    PgCellKind::OptionalBool("attribute_is_dropped"),
];

const USER_CONSTRAINT_ROW: [PgCellKind; 13] = [
    PgCellKind::I64("trigger_oid"),
    PgCellKind::I64("relation_oid"),
    PgCellKind::I64("trigger_constraint_oid"),
    PgCellKind::OptionalI64("constraint_oid"),
    PgCellKind::OptionalI64("constraint_relation_oid"),
    PgCellKind::OptionalI64("constraint_schema_oid"),
    PgCellKind::Text {
        null_alias: "constraint_schema_name_is_null",
        bytes_alias: "constraint_schema_name_bytes",
        value_alias: "constraint_schema_name",
    },
    PgCellKind::Text {
        null_alias: "constraint_kind_is_null",
        bytes_alias: "constraint_kind_bytes",
        value_alias: "constraint_kind",
    },
    PgCellKind::OptionalBool("is_deferrable"),
    PgCellKind::OptionalBool("initially_deferred"),
    PgCellKind::OptionalI64("constraint_type_oid"),
    PgCellKind::OptionalI64("index_oid"),
    PgCellKind::OptionalI64("foreign_relation_oid"),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RawCategory {
    Setup,
    Context,
    Role,
    Membership,
    Relation,
    Inheritance,
    RootHeader,
    RootSetting,
    ForwardTrigger,
    AdjacentHook,
    ReverseHook,
    UpdateAttribute,
    UserConstraint,
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
        value: Vec<u8>,
    },
    Bytes {
        is_null: bool,
        original_bytes: Option<i64>,
        prefix: Vec<u8>,
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
    fn charge(&mut self, cell: &RawCell) -> Result<(), SimulationTriggerMetadataError> {
        let bytes = match cell {
            RawCell::Text { value, .. } => value.len().checked_add(8),
            RawCell::Bytes { prefix, .. } => prefix.len().checked_add(8),
            RawCell::Bool(_) | RawCell::Null | RawCell::WrongType => Some(1),
            RawCell::I32(_) => Some(4),
            RawCell::I64(_) => Some(8),
        }
        .ok_or_else(|| SimulationTriggerMetadataError::primary(PrimaryFailure::PayloadExceeded))?;
        self.used = self
            .used
            .checked_add(bytes)
            .filter(|total| *total <= LOGICAL_PAYLOAD_LIMIT)
            .ok_or_else(|| {
                SimulationTriggerMetadataError::primary(PrimaryFailure::PayloadExceeded)
            })?;
        Ok(())
    }

    fn charge_row(&mut self, row: &RawRow) -> Result<(), SimulationTriggerMetadataError> {
        for cell in &row.cells {
            self.charge(cell)?;
        }
        Ok(())
    }
}

struct RawDecoder {
    cells: std::vec::IntoIter<RawCell>,
}

impl RawDecoder {
    fn new(
        row: RawRow,
        category: RawCategory,
        shape: &[PgCellKind],
        budget: &mut LogicalPayloadBudget,
    ) -> Result<Self, SimulationTriggerMetadataError> {
        budget.charge_row(&row)?;
        if row.category != category || row.cells.len() != shape.len() {
            return Err(SimulationTriggerMetadataError::primary(
                PrimaryFailure::CatalogInvalid,
            ));
        }
        Ok(Self {
            cells: row.cells.into_iter(),
        })
    }

    fn take(&mut self) -> Result<RawCell, SimulationTriggerMetadataError> {
        self.cells
            .next()
            .ok_or_else(|| SimulationTriggerMetadataError::primary(PrimaryFailure::CatalogInvalid))
    }

    fn bool(&mut self) -> Result<bool, SimulationTriggerMetadataError> {
        match self.take()? {
            RawCell::Bool(value) => Ok(value),
            _ => Err(SimulationTriggerMetadataError::primary(
                PrimaryFailure::CatalogInvalid,
            )),
        }
    }

    fn optional_bool(&mut self) -> Result<Option<bool>, SimulationTriggerMetadataError> {
        match self.take()? {
            RawCell::Bool(value) => Ok(Some(value)),
            RawCell::Null => Ok(None),
            _ => Err(SimulationTriggerMetadataError::primary(
                PrimaryFailure::CatalogInvalid,
            )),
        }
    }

    fn i32(&mut self) -> Result<i32, SimulationTriggerMetadataError> {
        match self.take()? {
            RawCell::I32(value) => Ok(value),
            _ => Err(SimulationTriggerMetadataError::primary(
                PrimaryFailure::CatalogInvalid,
            )),
        }
    }

    fn optional_i32(&mut self) -> Result<Option<i32>, SimulationTriggerMetadataError> {
        match self.take()? {
            RawCell::I32(value) => Ok(Some(value)),
            RawCell::Null => Ok(None),
            _ => Err(SimulationTriggerMetadataError::primary(
                PrimaryFailure::CatalogInvalid,
            )),
        }
    }

    fn i64(&mut self) -> Result<i64, SimulationTriggerMetadataError> {
        match self.take()? {
            RawCell::I64(value) => Ok(value),
            _ => Err(SimulationTriggerMetadataError::primary(
                PrimaryFailure::CatalogInvalid,
            )),
        }
    }

    fn optional_i64(&mut self) -> Result<Option<i64>, SimulationTriggerMetadataError> {
        match self.take()? {
            RawCell::I64(value) => Ok(Some(value)),
            RawCell::Null => Ok(None),
            _ => Err(SimulationTriggerMetadataError::primary(
                PrimaryFailure::CatalogInvalid,
            )),
        }
    }

    fn text(&mut self) -> Result<String, SimulationTriggerMetadataError> {
        self.optional_text()?
            .ok_or_else(|| SimulationTriggerMetadataError::primary(PrimaryFailure::CatalogInvalid))
    }

    fn optional_text(&mut self) -> Result<Option<String>, SimulationTriggerMetadataError> {
        let cell = self.take()?;
        match cell {
            RawCell::Text {
                is_null,
                original_bytes,
                value,
            } => {
                let Some(length) = original_bytes else {
                    if is_null && value.is_empty() {
                        return Ok(None);
                    }
                    return Err(SimulationTriggerMetadataError::primary(
                        PrimaryFailure::CatalogInvalid,
                    ));
                };
                let length = usize::try_from(length).map_err(|_| {
                    SimulationTriggerMetadataError::primary(PrimaryFailure::CatalogInvalid)
                })?;
                if is_null
                    || value.len() > TEXT_FETCH_BYTES
                    || value.len() != length
                    || length > TEXT_ACCEPT_BYTES
                {
                    return Err(SimulationTriggerMetadataError::primary(
                        PrimaryFailure::PolicyRejected,
                    ));
                }
                String::from_utf8(value).map(Some).map_err(|_| {
                    SimulationTriggerMetadataError::primary(PrimaryFailure::CatalogInvalid)
                })
            }
            _ => Err(SimulationTriggerMetadataError::primary(
                PrimaryFailure::CatalogInvalid,
            )),
        }
    }

    fn bytes(&mut self) -> Result<RawBytes, SimulationTriggerMetadataError> {
        match self.take()? {
            RawCell::Bytes {
                is_null,
                original_bytes,
                prefix,
            } => {
                let Some(length) = original_bytes else {
                    if is_null && prefix.is_empty() {
                        return Ok(RawBytes {
                            is_null: true,
                            length: None,
                            prefix,
                        });
                    }
                    return Err(SimulationTriggerMetadataError::primary(
                        PrimaryFailure::CatalogInvalid,
                    ));
                };
                let length = usize::try_from(length).map_err(|_| {
                    SimulationTriggerMetadataError::primary(PrimaryFailure::CatalogInvalid)
                })?;
                if is_null
                    || prefix.len() > TRIGGER_ARGS_FETCH_BYTES
                    || prefix.len() != length.min(1)
                {
                    return Err(SimulationTriggerMetadataError::primary(
                        PrimaryFailure::PolicyRejected,
                    ));
                }
                Ok(RawBytes {
                    is_null,
                    length: Some(length),
                    prefix,
                })
            }
            _ => Err(SimulationTriggerMetadataError::primary(
                PrimaryFailure::CatalogInvalid,
            )),
        }
    }

    fn finish(self) -> Result<(), SimulationTriggerMetadataError> {
        if self.cells.len() == 0 {
            Ok(())
        } else {
            Err(SimulationTriggerMetadataError::primary(
                PrimaryFailure::CatalogInvalid,
            ))
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct RawBytes {
    is_null: bool,
    length: Option<usize>,
    prefix: Vec<u8>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
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
    worker_inherits_roles: bool,
    row_security: String,
    session_replication_role: String,
    transaction_read_only: bool,
    transaction_isolation: String,
    search_path: String,
    statement_timeout: String,
    lock_timeout: String,
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct ContextEvidence {
    current_user: String,
    session_user: String,
    database_name: String,
    schema_name: String,
    database_oid: u32,
    worker_oid: u32,
    schema_oid: u32,
    migration_owner_oid: u32,
    database_owner_oid: u32,
    schema_owner_oid: u32,
    worker_can_login: bool,
    worker_is_superuser: bool,
    worker_can_create_database: bool,
    worker_can_create_role: bool,
    worker_can_replicate: bool,
    worker_bypasses_rls: bool,
    worker_inherits_roles: bool,
    row_security: String,
    session_replication_role: String,
    transaction_read_only: bool,
    transaction_isolation: String,
    search_path: String,
    statement_timeout: String,
    lock_timeout: String,
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct RoleRow {
    ordinal: i32,
    expected_name: String,
    actual_name: Option<String>,
    oid: Option<i64>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct MembershipRow {
    member_oid: i64,
    role_oid: i64,
    grantor_oid: i64,
    admin_option: bool,
    inherit_option: bool,
    set_option: bool,
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct RelationRow {
    ordinal: i32,
    expected_name: String,
    oid: Option<i64>,
    schema_oid: Option<i64>,
    schema_name: Option<String>,
    actual_name: Option<String>,
    owner_oid: Option<i64>,
    owner_name: Option<String>,
    kind: Option<String>,
    persistence: Option<String>,
    is_partition: Option<bool>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct RelationEvidence {
    ordered: [u32; RELATION_IDENTITY_ACCEPT_LIMIT],
    by_name: HashMap<&'static str, u32>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct RootHeaderRow {
    ordinal: i32,
    expected_name: String,
    oid: Option<i64>,
    schema_oid: Option<i64>,
    schema_name: Option<String>,
    actual_name: Option<String>,
    owner_oid: Option<i64>,
    owner_name: Option<String>,
    kind: Option<String>,
    argument_count: Option<i32>,
    input_type_count: Option<i32>,
    argument_defaults_count: Option<i32>,
    variadic_oid: Option<i64>,
    returns_set: Option<bool>,
    security_definer: Option<bool>,
    all_argument_types_is_null: Option<bool>,
    argument_modes_is_null: Option<bool>,
    argument_names_is_null: Option<bool>,
    argument_defaults_is_null: Option<bool>,
    language_oid: Option<i64>,
    language_name: Option<String>,
    return_type_oid: Option<i64>,
    return_namespace_oid: Option<i64>,
    return_namespace_name: Option<String>,
    return_type_name: Option<String>,
    config_is_null: bool,
    config_dimensions: Option<i32>,
    config_lower_bound: Option<i32>,
    config_cardinality: Option<i32>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct RootHeaderEvidence {
    name: &'static str,
    oid: u32,
    config_count: usize,
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct RootSettingRow {
    routine_oid: i64,
    ordinal: Option<i32>,
    value: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct TriggerRow {
    trigger_oid: i64,
    relation_oid: i64,
    schema_oid: i64,
    schema_name: String,
    relation_name: String,
    relation_owner_oid: i64,
    relation_owner_name: String,
    trigger_name: String,
    function_oid: i64,
    function_schema_oid: i64,
    function_schema_name: String,
    function_name: String,
    function_owner_oid: i64,
    function_owner_name: String,
    is_internal: bool,
    enabled: String,
    parent_oid: i64,
    event_bits: i32,
    argument_count: i32,
    attribute_dimensions: Option<i32>,
    attribute_lower_bound: Option<i32>,
    attribute_upper_bound: Option<i32>,
    attribute_cardinality: Option<i32>,
    trigger_args: RawBytes,
    when_is_null: bool,
    constraint_relation_oid: i64,
    constraint_oid: i64,
    update_attribute_count: i32,
    old_transition_name_is_null: bool,
    new_transition_name_is_null: bool,
    is_deferrable: bool,
    initially_deferred: bool,
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct TriggerEvidence {
    trigger_oid: u32,
    relation_oid: u32,
    relation: String,
    name: String,
    function_oid: u32,
    function: String,
    event_bits: i32,
    constraint_oid: Option<u32>,
    is_deferrable: bool,
    initially_deferred: bool,
}

type AttachmentKey = (String, String, String);

#[derive(Clone, PartialEq, Eq, Debug)]
struct UpdateAttributeRow {
    trigger_oid: i64,
    relation_oid: i64,
    ordinal: Option<i64>,
    attribute_number: Option<i32>,
    attribute_name: Option<String>,
    attribute_relation_oid: Option<i64>,
    is_dropped: Option<bool>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct UserConstraintRow {
    trigger_oid: i64,
    relation_oid: i64,
    trigger_constraint_oid: i64,
    constraint_oid: Option<i64>,
    constraint_relation_oid: Option<i64>,
    constraint_schema_oid: Option<i64>,
    constraint_schema_name: Option<String>,
    kind: Option<String>,
    is_deferrable: Option<bool>,
    initially_deferred: Option<bool>,
    constraint_type_oid: Option<i64>,
    index_oid: Option<i64>,
    foreign_relation_oid: Option<i64>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct PreparedObservation {
    context: ContextEvidence,
    relations: RelationEvidence,
    root_oids: [u32; ROOT_HEADER_ACCEPT_LIMIT],
    trigger_oids: [u32; REVERSE_HOOK_ACCEPT_LIMIT],
    constraint_oids: [u32; USER_CONSTRAINT_ACCEPT_LIMIT],
}

fn decode_setup(
    row: RawRow,
    budget: &mut LogicalPayloadBudget,
) -> Result<[String; 3], SimulationTriggerMetadataError> {
    let mut decoder = RawDecoder::new(row, RawCategory::Setup, &SETUP_ROW, budget)?;
    let search_path = decoder.text()?;
    let statement_timeout = decoder.text()?;
    let lock_timeout = decoder.text()?;
    decoder.finish()?;
    Ok([search_path, statement_timeout, lock_timeout])
}

fn decode_context(
    row: RawRow,
    budget: &mut LogicalPayloadBudget,
) -> Result<ContextRow, SimulationTriggerMetadataError> {
    let mut decoder = RawDecoder::new(row, RawCategory::Context, &CONTEXT_ROW, budget)?;
    let current_user = decoder.text()?;
    let session_user = decoder.text()?;
    let observed_database_name = decoder.text()?;
    let database_name = decoder.text()?;
    let schema_name = decoder.text()?;
    let worker_oid = decoder.i64()?;
    let database_oid = decoder.i64()?;
    let schema_oid = decoder.i64()?;
    let migration_owner_oid = decoder.i64()?;
    let database_owner_oid = decoder.i64()?;
    let schema_owner_oid = decoder.i64()?;
    let worker_can_login = decoder.bool()?;
    let worker_is_superuser = decoder.bool()?;
    let worker_can_create_database = decoder.bool()?;
    let worker_can_create_role = decoder.bool()?;
    let worker_can_replicate = decoder.bool()?;
    let worker_bypasses_rls = decoder.bool()?;
    let worker_inherits_roles = decoder.bool()?;
    let row_security = decoder.text()?;
    let session_replication_role = decoder.text()?;
    let transaction_read_only = decoder.bool()?;
    let transaction_isolation = decoder.text()?;
    let search_path = decoder.text()?;
    let statement_timeout = decoder.text()?;
    let lock_timeout = decoder.text()?;
    decoder.finish()?;
    Ok(ContextRow {
        current_user,
        session_user,
        observed_database_name,
        database_name,
        schema_name,
        worker_oid,
        database_oid,
        schema_oid,
        migration_owner_oid,
        database_owner_oid,
        schema_owner_oid,
        worker_can_login,
        worker_is_superuser,
        worker_can_create_database,
        worker_can_create_role,
        worker_can_replicate,
        worker_bypasses_rls,
        worker_inherits_roles,
        row_security,
        session_replication_role,
        transaction_read_only,
        transaction_isolation,
        search_path,
        statement_timeout,
        lock_timeout,
    })
}

fn decode_role(
    row: RawRow,
    budget: &mut LogicalPayloadBudget,
) -> Result<RoleRow, SimulationTriggerMetadataError> {
    let mut decoder = RawDecoder::new(row, RawCategory::Role, &ROLE_ROW, budget)?;
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
) -> Result<MembershipRow, SimulationTriggerMetadataError> {
    let mut decoder = RawDecoder::new(row, RawCategory::Membership, &MEMBERSHIP_ROW, budget)?;
    let value = MembershipRow {
        member_oid: decoder.i64()?,
        role_oid: decoder.i64()?,
        grantor_oid: decoder.i64()?,
        admin_option: decoder.bool()?,
        inherit_option: decoder.bool()?,
        set_option: decoder.bool()?,
    };
    decoder.finish()?;
    Ok(value)
}

fn decode_relation(
    row: RawRow,
    budget: &mut LogicalPayloadBudget,
) -> Result<RelationRow, SimulationTriggerMetadataError> {
    let mut decoder = RawDecoder::new(row, RawCategory::Relation, &RELATION_ROW, budget)?;
    let value = RelationRow {
        ordinal: decoder.i32()?,
        expected_name: decoder.text()?,
        oid: decoder.optional_i64()?,
        schema_oid: decoder.optional_i64()?,
        schema_name: decoder.optional_text()?,
        actual_name: decoder.optional_text()?,
        owner_oid: decoder.optional_i64()?,
        owner_name: decoder.optional_text()?,
        kind: decoder.optional_text()?,
        persistence: decoder.optional_text()?,
        is_partition: decoder.optional_bool()?,
    };
    decoder.finish()?;
    Ok(value)
}

fn decode_inheritance(
    row: RawRow,
    budget: &mut LogicalPayloadBudget,
) -> Result<(i64, i64), SimulationTriggerMetadataError> {
    let mut decoder = RawDecoder::new(row, RawCategory::Inheritance, &INHERITANCE_ROW, budget)?;
    let value = (decoder.i64()?, decoder.i64()?);
    decoder.finish()?;
    Ok(value)
}

fn decode_root_header(
    row: RawRow,
    budget: &mut LogicalPayloadBudget,
) -> Result<RootHeaderRow, SimulationTriggerMetadataError> {
    let mut decoder = RawDecoder::new(row, RawCategory::RootHeader, &ROOT_HEADER_ROW, budget)?;
    let value = RootHeaderRow {
        ordinal: decoder.i32()?,
        expected_name: decoder.text()?,
        oid: decoder.optional_i64()?,
        schema_oid: decoder.optional_i64()?,
        schema_name: decoder.optional_text()?,
        actual_name: decoder.optional_text()?,
        owner_oid: decoder.optional_i64()?,
        owner_name: decoder.optional_text()?,
        kind: decoder.optional_text()?,
        argument_count: decoder.optional_i32()?,
        input_type_count: decoder.optional_i32()?,
        argument_defaults_count: decoder.optional_i32()?,
        variadic_oid: decoder.optional_i64()?,
        returns_set: decoder.optional_bool()?,
        security_definer: decoder.optional_bool()?,
        all_argument_types_is_null: decoder.optional_bool()?,
        argument_modes_is_null: decoder.optional_bool()?,
        argument_names_is_null: decoder.optional_bool()?,
        argument_defaults_is_null: decoder.optional_bool()?,
        language_oid: decoder.optional_i64()?,
        language_name: decoder.optional_text()?,
        return_type_oid: decoder.optional_i64()?,
        return_namespace_oid: decoder.optional_i64()?,
        return_namespace_name: decoder.optional_text()?,
        return_type_name: decoder.optional_text()?,
        config_is_null: decoder.bool()?,
        config_dimensions: decoder.optional_i32()?,
        config_lower_bound: decoder.optional_i32()?,
        config_cardinality: decoder.optional_i32()?,
    };
    decoder.finish()?;
    Ok(value)
}

fn decode_root_setting(
    row: RawRow,
    budget: &mut LogicalPayloadBudget,
) -> Result<RootSettingRow, SimulationTriggerMetadataError> {
    let mut decoder = RawDecoder::new(row, RawCategory::RootSetting, &ROOT_SETTING_ROW, budget)?;
    let value = RootSettingRow {
        routine_oid: decoder.i64()?,
        ordinal: decoder.optional_i32()?,
        value: decoder.optional_text()?,
    };
    decoder.finish()?;
    Ok(value)
}

fn decode_trigger(
    row: RawRow,
    category: RawCategory,
    budget: &mut LogicalPayloadBudget,
) -> Result<TriggerRow, SimulationTriggerMetadataError> {
    let mut decoder = RawDecoder::new(row, category, &TRIGGER_ROW, budget)?;
    let value = TriggerRow {
        trigger_oid: decoder.i64()?,
        relation_oid: decoder.i64()?,
        schema_oid: decoder.i64()?,
        schema_name: decoder.text()?,
        relation_name: decoder.text()?,
        relation_owner_oid: decoder.i64()?,
        relation_owner_name: decoder.text()?,
        trigger_name: decoder.text()?,
        function_oid: decoder.i64()?,
        function_schema_oid: decoder.i64()?,
        function_schema_name: decoder.text()?,
        function_name: decoder.text()?,
        function_owner_oid: decoder.i64()?,
        function_owner_name: decoder.text()?,
        is_internal: decoder.bool()?,
        enabled: decoder.text()?,
        parent_oid: decoder.i64()?,
        event_bits: decoder.i32()?,
        argument_count: decoder.i32()?,
        attribute_dimensions: decoder.optional_i32()?,
        attribute_lower_bound: decoder.optional_i32()?,
        attribute_upper_bound: decoder.optional_i32()?,
        attribute_cardinality: decoder.optional_i32()?,
        trigger_args: decoder.bytes()?,
        when_is_null: decoder.bool()?,
        constraint_relation_oid: decoder.i64()?,
        constraint_oid: decoder.i64()?,
        update_attribute_count: decoder.i32()?,
        old_transition_name_is_null: decoder.bool()?,
        new_transition_name_is_null: decoder.bool()?,
        is_deferrable: decoder.bool()?,
        initially_deferred: decoder.bool()?,
    };
    decoder.finish()?;
    Ok(value)
}

fn decode_update_attribute(
    row: RawRow,
    budget: &mut LogicalPayloadBudget,
) -> Result<UpdateAttributeRow, SimulationTriggerMetadataError> {
    let mut decoder = RawDecoder::new(
        row,
        RawCategory::UpdateAttribute,
        &UPDATE_ATTRIBUTE_ROW,
        budget,
    )?;
    let value = UpdateAttributeRow {
        trigger_oid: decoder.i64()?,
        relation_oid: decoder.i64()?,
        ordinal: decoder.optional_i64()?,
        attribute_number: decoder.optional_i32()?,
        attribute_name: decoder.optional_text()?,
        attribute_relation_oid: decoder.optional_i64()?,
        is_dropped: decoder.optional_bool()?,
    };
    decoder.finish()?;
    Ok(value)
}

fn decode_user_constraint(
    row: RawRow,
    budget: &mut LogicalPayloadBudget,
) -> Result<UserConstraintRow, SimulationTriggerMetadataError> {
    let mut decoder = RawDecoder::new(
        row,
        RawCategory::UserConstraint,
        &USER_CONSTRAINT_ROW,
        budget,
    )?;
    let value = UserConstraintRow {
        trigger_oid: decoder.i64()?,
        relation_oid: decoder.i64()?,
        trigger_constraint_oid: decoder.i64()?,
        constraint_oid: decoder.optional_i64()?,
        constraint_relation_oid: decoder.optional_i64()?,
        constraint_schema_oid: decoder.optional_i64()?,
        constraint_schema_name: decoder.optional_text()?,
        kind: decoder.optional_text()?,
        is_deferrable: decoder.optional_bool()?,
        initially_deferred: decoder.optional_bool()?,
        constraint_type_oid: decoder.optional_i64()?,
        index_oid: decoder.optional_i64()?,
        foreign_relation_oid: decoder.optional_i64()?,
    };
    decoder.finish()?;
    Ok(value)
}

fn decode_many<T>(
    rows: Vec<RawRow>,
    category: RawCategory,
    accept_limit: usize,
    fetch_limit: usize,
    exact_count: usize,
    budget: &mut LogicalPayloadBudget,
    mut decode: impl FnMut(
        RawRow,
        &mut LogicalPayloadBudget,
    ) -> Result<T, SimulationTriggerMetadataError>,
) -> Result<Vec<T>, SimulationTriggerMetadataError> {
    if rows.len() > fetch_limit {
        for row in &rows {
            budget.charge_row(row)?;
        }
        return Err(SimulationTriggerMetadataError::primary(
            PrimaryFailure::BoundExceeded,
        ));
    }
    let mut decoded = Vec::with_capacity(rows.len());
    for row in rows {
        if row.category != category {
            budget.charge_row(&row)?;
            return Err(SimulationTriggerMetadataError::primary(
                PrimaryFailure::CatalogInvalid,
            ));
        }
        decoded.push(decode(row, budget)?);
    }
    if decoded.len() > accept_limit || decoded.len() > fetch_limit {
        return Err(SimulationTriggerMetadataError::primary(
            PrimaryFailure::BoundExceeded,
        ));
    }
    if decoded.len() != exact_count {
        return Err(SimulationTriggerMetadataError::primary(
            PrimaryFailure::CatalogInvalid,
        ));
    }
    Ok(decoded)
}

fn positive_oid(value: i64) -> Result<u32, SimulationTriggerMetadataError> {
    if value <= 0 || value > i64::from(u32::MAX) {
        return Err(SimulationTriggerMetadataError::primary(
            PrimaryFailure::CatalogInvalid,
        ));
    }
    u32::try_from(value)
        .map_err(|_| SimulationTriggerMetadataError::primary(PrimaryFailure::CatalogInvalid))
}

fn required<T>(value: Option<T>) -> Result<T, SimulationTriggerMetadataError> {
    value.ok_or_else(|| SimulationTriggerMetadataError::primary(PrimaryFailure::CatalogInvalid))
}

fn decode_context_rows(
    rows: Vec<RawRow>,
    accept_limit: usize,
    fetch_limit: usize,
    budget: &mut LogicalPayloadBudget,
) -> Result<ContextRow, SimulationTriggerMetadataError> {
    decode_many(
        rows,
        RawCategory::Context,
        accept_limit,
        fetch_limit,
        1,
        budget,
        decode_context,
    )?
    .pop()
    .ok_or_else(|| SimulationTriggerMetadataError::primary(PrimaryFailure::CatalogInvalid))
}

fn timeout_millis(config: &SimulationProcessConfig) -> Result<i64, SimulationTriggerMetadataError> {
    let value = config
        .limits()
        .startup_timeout_seconds()
        .checked_mul(1000)
        .filter(|value| *value > 0 && *value <= i64::MAX as u64)
        .ok_or_else(|| SimulationTriggerMetadataError::primary(PrimaryFailure::InvalidBound))?;
    i64::try_from(value)
        .map_err(|_| SimulationTriggerMetadataError::primary(PrimaryFailure::InvalidBound))
}

fn timeout_text(timeout_ms: i64) -> String {
    format!("{timeout_ms}ms")
}

fn validate_setup(
    rows: Vec<RawRow>,
    timeout_ms: i64,
    budget: &mut LogicalPayloadBudget,
) -> Result<(), SimulationTriggerMetadataError> {
    let decoded = decode_many(
        rows,
        RawCategory::Setup,
        SETUP_ACCEPT_LIMIT,
        SETUP_FETCH_LIMIT,
        1,
        budget,
        decode_setup,
    )?;
    let [search_path, statement_timeout, lock_timeout] =
        decoded.into_iter().next().ok_or_else(|| {
            SimulationTriggerMetadataError::primary(PrimaryFailure::SetupResultInvalid)
        })?;
    let expected = timeout_text(timeout_ms);
    if search_path != "pg_catalog" || statement_timeout != expected || lock_timeout != expected {
        return Err(SimulationTriggerMetadataError::primary(
            PrimaryFailure::SetupResultInvalid,
        ));
    }
    Ok(())
}

fn validate_context(
    row: ContextRow,
    timeout_ms: i64,
) -> Result<ContextEvidence, SimulationTriggerMetadataError> {
    let worker_oid = positive_oid(row.worker_oid)?;
    let database_oid = positive_oid(row.database_oid)?;
    let schema_oid = positive_oid(row.schema_oid)?;
    let migration_owner_oid = positive_oid(row.migration_owner_oid)?;
    let database_owner_oid = positive_oid(row.database_owner_oid)?;
    let schema_owner_oid = positive_oid(row.schema_owner_oid)?;
    let expected_timeout = timeout_text(timeout_ms);
    if row.current_user != WORKER_ROLE
        || row.session_user != WORKER_ROLE
        || row.observed_database_name != row.database_name
        || row.schema_name != PUBLIC_SCHEMA
        || worker_oid == migration_owner_oid
        || database_owner_oid == worker_oid
        || schema_owner_oid == worker_oid
    {
        return Err(SimulationTriggerMetadataError::primary(
            PrimaryFailure::PolicyRejected,
        ));
    }
    if !row.worker_can_login
        || row.worker_is_superuser
        || row.worker_can_create_database
        || row.worker_can_create_role
        || row.worker_can_replicate
        || row.worker_bypasses_rls
        || row.worker_inherits_roles
        || row.row_security != "on"
        || row.session_replication_role != "origin"
        || !row.transaction_read_only
        || row.transaction_isolation != "repeatable read"
        || row.search_path != "pg_catalog"
        || row.statement_timeout != expected_timeout
        || row.lock_timeout != expected_timeout
    {
        return Err(SimulationTriggerMetadataError::primary(
            PrimaryFailure::PolicyRejected,
        ));
    }
    Ok(ContextEvidence {
        current_user: row.current_user,
        session_user: row.session_user,
        database_name: row.database_name,
        schema_name: row.schema_name,
        database_oid,
        worker_oid,
        schema_oid,
        migration_owner_oid,
        database_owner_oid,
        schema_owner_oid,
        worker_can_login: row.worker_can_login,
        worker_is_superuser: row.worker_is_superuser,
        worker_can_create_database: row.worker_can_create_database,
        worker_can_create_role: row.worker_can_create_role,
        worker_can_replicate: row.worker_can_replicate,
        worker_bypasses_rls: row.worker_bypasses_rls,
        worker_inherits_roles: row.worker_inherits_roles,
        row_security: row.row_security,
        session_replication_role: row.session_replication_role,
        transaction_read_only: row.transaction_read_only,
        transaction_isolation: row.transaction_isolation,
        search_path: row.search_path,
        statement_timeout: row.statement_timeout,
        lock_timeout: row.lock_timeout,
    })
}

fn validate_roles(
    rows: Vec<RawRow>,
    context: &ContextEvidence,
    budget: &mut LogicalPayloadBudget,
) -> Result<HashMap<&'static str, u32>, SimulationTriggerMetadataError> {
    const EXPECTED: [&str; ROLE_ACCEPT_LIMIT] = ["worker", "migration_owner"];
    let decoded = decode_many(
        rows,
        RawCategory::Role,
        ROLE_ACCEPT_LIMIT,
        ROLE_FETCH_LIMIT,
        ROLE_ACCEPT_LIMIT,
        budget,
        decode_role,
    )?;
    let mut roles = HashMap::with_capacity(ROLE_ACCEPT_LIMIT);
    let mut oids = HashSet::with_capacity(ROLE_ACCEPT_LIMIT);
    for (index, role) in decoded.into_iter().enumerate() {
        let expected = EXPECTED[index];
        let oid = positive_oid(required(role.oid)?)?;
        if role.ordinal != (index + 1) as i32
            || role.expected_name != expected
            || role.actual_name.as_deref() != Some(expected)
            || !oids.insert(oid)
        {
            return Err(SimulationTriggerMetadataError::primary(
                PrimaryFailure::PolicyRejected,
            ));
        }
        roles.insert(expected, oid);
    }
    if roles.get(WORKER_ROLE) != Some(&context.worker_oid)
        || roles.get(MIGRATION_OWNER_ROLE) != Some(&context.migration_owner_oid)
    {
        return Err(SimulationTriggerMetadataError::primary(
            PrimaryFailure::PolicyRejected,
        ));
    }
    Ok(roles)
}

fn validate_worker_memberships(
    rows: Vec<RawRow>,
    worker_oid: u32,
    budget: &mut LogicalPayloadBudget,
) -> Result<(), SimulationTriggerMetadataError> {
    let decoded = decode_many(
        rows,
        RawCategory::Membership,
        WORKER_MEMBERSHIP_ACCEPT_LIMIT,
        WORKER_MEMBERSHIP_FETCH_LIMIT,
        WORKER_MEMBERSHIP_ACCEPT_LIMIT,
        budget,
        decode_membership,
    )?;
    if let Some(row) = decoded.into_iter().next() {
        let member_oid = positive_oid(row.member_oid)?;
        let _role_oid = positive_oid(row.role_oid)?;
        let _grantor_oid = positive_oid(row.grantor_oid)?;
        if member_oid != worker_oid {
            return Err(SimulationTriggerMetadataError::primary(
                PrimaryFailure::CatalogInvalid,
            ));
        }
        return Err(SimulationTriggerMetadataError::primary(
            PrimaryFailure::PolicyRejected,
        ));
    }
    Ok(())
}

fn validate_relations(
    rows: Vec<RawRow>,
    context: &ContextEvidence,
    roles: &HashMap<&'static str, u32>,
    budget: &mut LogicalPayloadBudget,
) -> Result<RelationEvidence, SimulationTriggerMetadataError> {
    let decoded = decode_many(
        rows,
        RawCategory::Relation,
        RELATION_IDENTITY_ACCEPT_LIMIT,
        RELATION_IDENTITY_FETCH_LIMIT,
        RELATION_IDENTITY_ACCEPT_LIMIT,
        budget,
        decode_relation,
    )?;
    let migration_owner_oid = *roles
        .get(MIGRATION_OWNER_ROLE)
        .ok_or_else(|| SimulationTriggerMetadataError::primary(PrimaryFailure::CatalogInvalid))?;
    let mut ordered = [0; RELATION_IDENTITY_ACCEPT_LIMIT];
    let mut by_name = HashMap::with_capacity(RELATION_IDENTITY_ACCEPT_LIMIT);
    let mut unique_oids = HashSet::with_capacity(RELATION_IDENTITY_ACCEPT_LIMIT);
    for (index, relation) in decoded.into_iter().enumerate() {
        let spec = RELATION_SPECS[index];
        let oid = positive_oid(required(relation.oid)?)?;
        let schema_oid = positive_oid(required(relation.schema_oid)?)?;
        let owner_oid = positive_oid(required(relation.owner_oid)?)?;
        if relation.ordinal != (index + 1) as i32
            || relation.expected_name != spec.name
            || relation.actual_name.as_deref() != Some(spec.name)
            || relation.schema_name.as_deref() != Some(PUBLIC_SCHEMA)
            || schema_oid != context.schema_oid
            || relation.owner_name.as_deref() != Some(MIGRATION_OWNER_ROLE)
            || owner_oid != migration_owner_oid
            || relation.kind.as_deref() != Some("r")
            || relation.persistence.as_deref() != Some("p")
            || relation.is_partition != Some(false)
            || !unique_oids.insert(oid)
        {
            return Err(SimulationTriggerMetadataError::primary(
                PrimaryFailure::PolicyRejected,
            ));
        }
        ordered[index] = oid;
        by_name.insert(spec.name, oid);
    }
    if by_name.len() != RELATION_IDENTITY_ACCEPT_LIMIT {
        return Err(SimulationTriggerMetadataError::primary(
            PrimaryFailure::CatalogInvalid,
        ));
    }
    Ok(RelationEvidence { ordered, by_name })
}

fn primary_relation_oids(
    relations: &RelationEvidence,
) -> Result<[u32; 4], SimulationTriggerMetadataError> {
    let names = [
        "owner_beta_simulation_runs",
        "owner_beta_simulation_result_pairs",
        "jobs",
        "job_attempts",
    ];
    let mut oids = [0; 4];
    for (index, name) in names.into_iter().enumerate() {
        oids[index] = *relations.by_name.get(name).ok_or_else(|| {
            SimulationTriggerMetadataError::primary(PrimaryFailure::CatalogInvalid)
        })?;
    }
    Ok(oids)
}

fn adjacent_relation_oid(
    relations: &RelationEvidence,
) -> Result<u32, SimulationTriggerMetadataError> {
    relations
        .by_name
        .get("user_strategy_configs")
        .copied()
        .ok_or_else(|| SimulationTriggerMetadataError::primary(PrimaryFailure::CatalogInvalid))
}

fn validate_primary_inheritance(
    rows: Vec<RawRow>,
    budget: &mut LogicalPayloadBudget,
) -> Result<(), SimulationTriggerMetadataError> {
    let decoded = decode_many(
        rows,
        RawCategory::Inheritance,
        PRIMARY_INHERITANCE_ACCEPT_LIMIT,
        PRIMARY_INHERITANCE_FETCH_LIMIT,
        PRIMARY_INHERITANCE_ACCEPT_LIMIT,
        budget,
        decode_inheritance,
    )?;
    if decoded.is_empty() {
        Ok(())
    } else {
        Err(SimulationTriggerMetadataError::primary(
            PrimaryFailure::PolicyRejected,
        ))
    }
}

fn validate_root_headers(
    rows: Vec<RawRow>,
    context: &ContextEvidence,
    roles: &HashMap<&'static str, u32>,
    budget: &mut LogicalPayloadBudget,
) -> Result<[RootHeaderEvidence; ROOT_HEADER_ACCEPT_LIMIT], SimulationTriggerMetadataError> {
    let decoded = decode_many(
        rows,
        RawCategory::RootHeader,
        ROOT_HEADER_ACCEPT_LIMIT,
        ROOT_HEADER_FETCH_LIMIT,
        ROOT_HEADER_ACCEPT_LIMIT,
        budget,
        decode_root_header,
    )?;
    let migration_owner_oid = *roles
        .get(MIGRATION_OWNER_ROLE)
        .ok_or_else(|| SimulationTriggerMetadataError::primary(PrimaryFailure::CatalogInvalid))?;
    let mut headers = Vec::with_capacity(ROOT_HEADER_ACCEPT_LIMIT);
    let mut unique_routine_oids = HashSet::with_capacity(ROOT_HEADER_ACCEPT_LIMIT);
    let mut language_oid = None;
    let mut return_type_oid = None;
    let mut return_namespace_oid = None;

    for (index, row) in decoded.into_iter().enumerate() {
        let spec = ROOT_SPECS[index];
        let oid = positive_oid(required(row.oid)?)?;
        let schema_oid = positive_oid(required(row.schema_oid)?)?;
        let owner_oid = positive_oid(required(row.owner_oid)?)?;
        let language = positive_oid(required(row.language_oid)?)?;
        let return_oid = positive_oid(required(row.return_type_oid)?)?;
        let return_schema_oid = positive_oid(required(row.return_namespace_oid)?)?;
        let setting_count = if spec.daily_timeouts { 3 } else { 1 };

        if row.ordinal != (index + 1) as i32
            || row.expected_name != spec.name
            || row.actual_name.as_deref() != Some(spec.name)
            || row.schema_name.as_deref() != Some(PUBLIC_SCHEMA)
            || schema_oid != context.schema_oid
            || row.owner_name.as_deref() != Some(MIGRATION_OWNER_ROLE)
            || owner_oid != migration_owner_oid
            || row.kind.as_deref() != Some("f")
            || row.argument_count != Some(0)
            || row.input_type_count != Some(0)
            || row.argument_defaults_count != Some(0)
            || row.variadic_oid != Some(0)
            || row.returns_set != Some(false)
            || row.security_definer != Some(spec.security_definer)
            || row.all_argument_types_is_null != Some(true)
            || row.argument_modes_is_null != Some(true)
            || row.argument_names_is_null != Some(true)
            || row.argument_defaults_is_null != Some(true)
            || row.language_name.as_deref() != Some("plpgsql")
            || row.return_namespace_name.as_deref() != Some(PG_CATALOG_SCHEMA)
            || row.return_type_name.as_deref() != Some("trigger")
            || row.config_is_null
            || row.config_dimensions != Some(1)
            || row.config_lower_bound != Some(1)
            || row.config_cardinality != Some(setting_count as i32)
            || !unique_routine_oids.insert(oid)
        {
            return Err(SimulationTriggerMetadataError::primary(
                PrimaryFailure::PolicyRejected,
            ));
        }
        if language_oid.is_some_and(|expected| expected != language)
            || return_type_oid.is_some_and(|expected| expected != return_oid)
            || return_namespace_oid.is_some_and(|expected| expected != return_schema_oid)
        {
            return Err(SimulationTriggerMetadataError::primary(
                PrimaryFailure::CatalogInvalid,
            ));
        }
        language_oid = Some(language);
        return_type_oid = Some(return_oid);
        return_namespace_oid = Some(return_schema_oid);
        headers.push(RootHeaderEvidence {
            name: spec.name,
            oid,
            config_count: setting_count,
        });
    }
    headers
        .try_into()
        .map_err(|_| SimulationTriggerMetadataError::primary(PrimaryFailure::CatalogInvalid))
}

fn search_path_matches(actual: &str, expected: &str) -> bool {
    let actual = actual.as_bytes();
    let expected_parts = expected.split(',').collect::<Vec<_>>();
    let mut cursor = 0_usize;
    for (index, part) in expected_parts.iter().enumerate() {
        let part = part.as_bytes();
        let Some(end) = cursor.checked_add(part.len()) else {
            return false;
        };
        if part.is_empty() || actual.get(cursor..end) != Some(part) {
            return false;
        }
        cursor = end;
        if index + 1 < expected_parts.len() {
            if actual.get(cursor) != Some(&b',') {
                return false;
            }
            cursor += 1;
            while actual.get(cursor) == Some(&b' ') {
                cursor += 1;
            }
        }
    }
    cursor == actual.len()
}

fn validate_root_settings(
    rows: Vec<RawRow>,
    headers: &[RootHeaderEvidence; ROOT_HEADER_ACCEPT_LIMIT],
    budget: &mut LogicalPayloadBudget,
) -> Result<(), SimulationTriggerMetadataError> {
    let decoded = decode_many(
        rows,
        RawCategory::RootSetting,
        ROOT_SETTING_ACCEPT_LIMIT,
        ROOT_SETTING_FETCH_LIMIT,
        ROOT_SETTING_ACCEPT_LIMIT,
        budget,
        decode_root_setting,
    )?;
    let mut by_oid = HashMap::with_capacity(ROOT_HEADER_ACCEPT_LIMIT);
    for header in headers {
        by_oid.insert(header.oid, header);
    }
    let mut values =
        HashMap::<u32, HashMap<String, String>>::with_capacity(ROOT_HEADER_ACCEPT_LIMIT);
    let mut previous_order = None;
    for row in decoded {
        let oid = positive_oid(row.routine_oid)?;
        let header = by_oid.get(&oid).ok_or_else(|| {
            SimulationTriggerMetadataError::primary(PrimaryFailure::PolicyRejected)
        })?;
        let ordinal = required(row.ordinal)?;
        if ordinal <= 0 || ordinal as usize > header.config_count {
            return Err(SimulationTriggerMetadataError::primary(
                PrimaryFailure::PolicyRejected,
            ));
        }
        let order = (oid, ordinal);
        if previous_order.is_some_and(|previous| order <= previous) {
            return Err(SimulationTriggerMetadataError::primary(
                PrimaryFailure::CatalogInvalid,
            ));
        }
        previous_order = Some(order);
        let value = required(row.value)?;
        let (key, setting_value) = value.split_once('=').ok_or_else(|| {
            SimulationTriggerMetadataError::primary(PrimaryFailure::PolicyRejected)
        })?;
        if key.is_empty() || setting_value.is_empty() || setting_value.contains('=') {
            return Err(SimulationTriggerMetadataError::primary(
                PrimaryFailure::PolicyRejected,
            ));
        }
        let key = match key {
            "search_path" => "search_path",
            "lock_timeout" => "lock_timeout",
            "statement_timeout" => "statement_timeout",
            _ => {
                return Err(SimulationTriggerMetadataError::primary(
                    PrimaryFailure::PolicyRejected,
                ));
            }
        };
        if values
            .entry(oid)
            .or_default()
            .insert(key.to_owned(), setting_value.to_owned())
            .is_some()
        {
            return Err(SimulationTriggerMetadataError::primary(
                PrimaryFailure::PolicyRejected,
            ));
        }
    }

    for header in headers {
        let spec = ROOT_SPECS
            .iter()
            .find(|spec| spec.name == header.name)
            .ok_or_else(|| {
                SimulationTriggerMetadataError::primary(PrimaryFailure::CatalogInvalid)
            })?;
        let settings = values.get(&header.oid).ok_or_else(|| {
            SimulationTriggerMetadataError::primary(PrimaryFailure::PolicyRejected)
        })?;
        let expected_count = if spec.daily_timeouts { 3 } else { 1 };
        if settings.len() != expected_count
            || !settings
                .get("search_path")
                .is_some_and(|actual| search_path_matches(actual, spec.search_path))
            || (spec.daily_timeouts
                && (settings.get("lock_timeout").map(String::as_str) != Some("5s")
                    || settings.get("statement_timeout").map(String::as_str) != Some("30s")))
        {
            return Err(SimulationTriggerMetadataError::primary(
                PrimaryFailure::PolicyRejected,
            ));
        }
    }
    Ok(())
}

fn empty_attribute_shape(row: &TriggerRow) -> bool {
    if row.attribute_cardinality != Some(0) {
        return false;
    }
    match row.attribute_dimensions {
        None | Some(0) => {
            row.attribute_lower_bound.is_none() && row.attribute_upper_bound.is_none()
        }
        Some(1) => {
            (row.attribute_lower_bound.is_none() && row.attribute_upper_bound.is_none())
                || (row.attribute_lower_bound == Some(0) && row.attribute_upper_bound == Some(-1))
        }
        _ => false,
    }
}

#[allow(clippy::too_many_arguments)] // Separate catalog identities and bounds are required.
fn validate_trigger_rows(
    rows: Vec<RawRow>,
    category: RawCategory,
    expected: &[AttachmentSpec],
    accept_limit: usize,
    fetch_limit: usize,
    context: &ContextEvidence,
    relations: &RelationEvidence,
    roles: &HashMap<&'static str, u32>,
    root_oids: &HashMap<&'static str, u32>,
    budget: &mut LogicalPayloadBudget,
) -> Result<HashMap<AttachmentKey, TriggerEvidence>, SimulationTriggerMetadataError> {
    let decoded = decode_many(
        rows,
        category,
        accept_limit,
        fetch_limit,
        expected.len(),
        budget,
        |row, budget| decode_trigger(row, category, budget),
    )?;
    let migration_owner_oid = *roles
        .get(MIGRATION_OWNER_ROLE)
        .ok_or_else(|| SimulationTriggerMetadataError::primary(PrimaryFailure::CatalogInvalid))?;
    let mut seen_keys = HashSet::with_capacity(expected.len());
    let mut seen_oids = HashSet::with_capacity(expected.len());
    let mut result = HashMap::with_capacity(expected.len());
    let mut previous_order: Option<(i64, String, i64)> = None;

    for row in decoded {
        let spec = expected
            .iter()
            .find(|spec| {
                spec.relation == row.relation_name
                    && spec.name == row.trigger_name
                    && spec.function == row.function_name
            })
            .ok_or_else(|| {
                SimulationTriggerMetadataError::primary(PrimaryFailure::PolicyRejected)
            })?;
        let key = (
            row.relation_name.clone(),
            row.trigger_name.clone(),
            row.function_name.clone(),
        );
        let trigger_oid = positive_oid(row.trigger_oid)?;
        let relation_oid = positive_oid(row.relation_oid)?;
        let schema_oid = positive_oid(row.schema_oid)?;
        let relation_owner_oid = positive_oid(row.relation_owner_oid)?;
        let function_oid = positive_oid(row.function_oid)?;
        let function_schema_oid = positive_oid(row.function_schema_oid)?;
        let function_owner_oid = positive_oid(row.function_owner_oid)?;
        let expected_relation_oid = *relations.by_name.get(spec.relation).ok_or_else(|| {
            SimulationTriggerMetadataError::primary(PrimaryFailure::CatalogInvalid)
        })?;
        let expected_event_mask = 1 | 2 | 4 | 8 | 16;
        let update_count = if spec.update_of_status { 1 } else { 0 };
        let constraint_oid = if spec.constraint {
            Some(positive_oid(row.constraint_oid)?)
        } else {
            if row.constraint_oid != 0 {
                return Err(SimulationTriggerMetadataError::primary(
                    PrimaryFailure::PolicyRejected,
                ));
            }
            None
        };
        let root_oid = root_oids.get(spec.function);
        if row.function_name.starts_with(SIMULATION_FUNCTION_PREFIX) && root_oid.is_none() {
            return Err(SimulationTriggerMetadataError::primary(
                PrimaryFailure::PolicyRejected,
            ));
        }
        if root_oid.is_some_and(|expected_oid| *expected_oid != function_oid) {
            return Err(SimulationTriggerMetadataError::primary(
                PrimaryFailure::PolicyRejected,
            ));
        }
        let attrs_valid = if spec.update_of_status {
            row.attribute_dimensions == Some(1)
                && row.attribute_lower_bound == Some(0)
                && row.attribute_upper_bound == Some(0)
                && row.attribute_cardinality == Some(1)
        } else {
            empty_attribute_shape(&row)
        };
        let current_order = (row.relation_oid, row.trigger_name.clone(), row.trigger_oid);
        if previous_order
            .as_ref()
            .is_some_and(|previous| current_order <= *previous)
        {
            return Err(SimulationTriggerMetadataError::primary(
                PrimaryFailure::CatalogInvalid,
            ));
        }
        previous_order = Some(current_order);

        if !PROTECTED_TRIGGER_NAMES.contains(&row.trigger_name.as_str())
            || row.schema_name != PUBLIC_SCHEMA
            || schema_oid != context.schema_oid
            || row.relation_owner_name != MIGRATION_OWNER_ROLE
            || relation_owner_oid != migration_owner_oid
            || row.function_schema_name != PUBLIC_SCHEMA
            || function_schema_oid != context.schema_oid
            || row.function_owner_name != MIGRATION_OWNER_ROLE
            || function_owner_oid != migration_owner_oid
            || relation_oid != expected_relation_oid
            || row.is_internal
            || row.enabled != "O"
            || row.is_deferrable != spec.constraint
            || row.initially_deferred != spec.constraint
            || row.parent_oid != 0
            || row.event_bits < 0
            || row.event_bits & !127 != 0
            || row.event_bits & !expected_event_mask != 0
            || row.event_bits & 1 == 0
            || row.event_bits & 32 != 0
            || row.event_bits & 64 != 0
            || row.event_bits != spec.event_bits
            || row.argument_count != 0
            || row.trigger_args.is_null
            || row.trigger_args.length != Some(0)
            || !row.trigger_args.prefix.is_empty()
            || !row.when_is_null
            || row.constraint_relation_oid != 0
            || row.update_attribute_count != update_count
            || !attrs_valid
            || !row.old_transition_name_is_null
            || !row.new_transition_name_is_null
            || !seen_oids.insert(trigger_oid)
            || !seen_keys.insert(key.clone())
        {
            return Err(SimulationTriggerMetadataError::primary(
                PrimaryFailure::PolicyRejected,
            ));
        }
        result.insert(
            key,
            TriggerEvidence {
                trigger_oid,
                relation_oid,
                relation: row.relation_name,
                name: row.trigger_name,
                function_oid,
                function: row.function_name,
                event_bits: row.event_bits,
                constraint_oid,
                is_deferrable: row.is_deferrable,
                initially_deferred: row.initially_deferred,
            },
        );
    }
    if result.len() != expected.len()
        || expected.iter().any(|spec| {
            !result.contains_key(&(
                spec.relation.to_owned(),
                spec.name.to_owned(),
                spec.function.to_owned(),
            ))
        })
    {
        return Err(SimulationTriggerMetadataError::primary(
            PrimaryFailure::PolicyRejected,
        ));
    }
    Ok(result)
}

fn trigger_key(spec: &AttachmentSpec) -> AttachmentKey {
    (
        spec.relation.to_owned(),
        spec.name.to_owned(),
        spec.function.to_owned(),
    )
}

#[allow(clippy::too_many_arguments)] // Preserve independently checked trigger categories.
fn validate_trigger_census(
    forward_rows: Vec<RawRow>,
    adjacent_rows: Vec<RawRow>,
    reverse_rows: Vec<RawRow>,
    context: &ContextEvidence,
    relations: &RelationEvidence,
    roles: &HashMap<&'static str, u32>,
    roots: &[RootHeaderEvidence; ROOT_HEADER_ACCEPT_LIMIT],
    budget: &mut LogicalPayloadBudget,
) -> Result<
    (
        HashMap<AttachmentKey, TriggerEvidence>,
        [u32; REVERSE_HOOK_ACCEPT_LIMIT],
    ),
    SimulationTriggerMetadataError,
> {
    let root_oids = roots
        .iter()
        .map(|root| (root.name, root.oid))
        .collect::<HashMap<_, _>>();
    let primary_expected = ATTACHMENT_SPECS
        .iter()
        .copied()
        .filter(|spec| {
            RELATION_SPECS
                .iter()
                .any(|relation| relation.name == spec.relation && relation.primary)
        })
        .collect::<Vec<_>>();
    let adjacent_expected = ATTACHMENT_SPECS
        .iter()
        .copied()
        .filter(|spec| {
            RELATION_SPECS
                .iter()
                .any(|relation| relation.name == spec.relation && relation.adjacent_config)
        })
        .collect::<Vec<_>>();
    if primary_expected.len() != FORWARD_TRIGGER_ACCEPT_LIMIT
        || adjacent_expected.len() != ADJACENT_HOOK_ACCEPT_LIMIT
        || ATTACHMENT_SPECS.len() != REVERSE_HOOK_ACCEPT_LIMIT
    {
        return Err(SimulationTriggerMetadataError::primary(
            PrimaryFailure::CatalogInvalid,
        ));
    }

    let forward = validate_trigger_rows(
        forward_rows,
        RawCategory::ForwardTrigger,
        &primary_expected,
        FORWARD_TRIGGER_ACCEPT_LIMIT,
        FORWARD_TRIGGER_FETCH_LIMIT,
        context,
        relations,
        roles,
        &root_oids,
        budget,
    )?;
    let adjacent = validate_trigger_rows(
        adjacent_rows,
        RawCategory::AdjacentHook,
        &adjacent_expected,
        ADJACENT_HOOK_ACCEPT_LIMIT,
        ADJACENT_HOOK_FETCH_LIMIT,
        context,
        relations,
        roles,
        &root_oids,
        budget,
    )?;
    let reverse = validate_trigger_rows(
        reverse_rows,
        RawCategory::ReverseHook,
        &ATTACHMENT_SPECS,
        REVERSE_HOOK_ACCEPT_LIMIT,
        REVERSE_HOOK_FETCH_LIMIT,
        context,
        relations,
        roles,
        &root_oids,
        budget,
    )?;
    for scoped in [&forward, &adjacent] {
        for (key, evidence) in scoped {
            if reverse.get(key) != Some(evidence) {
                return Err(SimulationTriggerMetadataError::primary(
                    PrimaryFailure::PolicyRejected,
                ));
            }
        }
    }
    let mut trigger_oids = [0; REVERSE_HOOK_ACCEPT_LIMIT];
    for (index, spec) in ATTACHMENT_SPECS.iter().enumerate() {
        trigger_oids[index] = reverse
            .get(&trigger_key(spec))
            .ok_or_else(|| SimulationTriggerMetadataError::primary(PrimaryFailure::CatalogInvalid))?
            .trigger_oid;
    }
    Ok((reverse, trigger_oids))
}

fn update_trigger_oids(
    triggers: &HashMap<AttachmentKey, TriggerEvidence>,
) -> Result<[u32; UPDATE_ATTRIBUTE_ACCEPT_LIMIT], SimulationTriggerMetadataError> {
    let mut values = Vec::with_capacity(UPDATE_ATTRIBUTE_ACCEPT_LIMIT);
    for spec in ATTACHMENT_SPECS.iter().filter(|spec| spec.update_of_status) {
        values.push(
            triggers
                .get(&trigger_key(spec))
                .ok_or_else(|| {
                    SimulationTriggerMetadataError::primary(PrimaryFailure::CatalogInvalid)
                })?
                .trigger_oid,
        );
    }
    values
        .try_into()
        .map_err(|_| SimulationTriggerMetadataError::primary(PrimaryFailure::CatalogInvalid))
}

fn constraint_trigger_oids(
    triggers: &HashMap<AttachmentKey, TriggerEvidence>,
) -> Result<[u32; USER_CONSTRAINT_ACCEPT_LIMIT], SimulationTriggerMetadataError> {
    let mut values = Vec::with_capacity(USER_CONSTRAINT_ACCEPT_LIMIT);
    for spec in ATTACHMENT_SPECS.iter().filter(|spec| spec.constraint) {
        values.push(
            triggers
                .get(&trigger_key(spec))
                .ok_or_else(|| {
                    SimulationTriggerMetadataError::primary(PrimaryFailure::CatalogInvalid)
                })?
                .trigger_oid,
        );
    }
    values
        .try_into()
        .map_err(|_| SimulationTriggerMetadataError::primary(PrimaryFailure::CatalogInvalid))
}

fn validate_update_attributes(
    rows: Vec<RawRow>,
    triggers: &HashMap<AttachmentKey, TriggerEvidence>,
    budget: &mut LogicalPayloadBudget,
) -> Result<(), SimulationTriggerMetadataError> {
    let decoded = decode_many(
        rows,
        RawCategory::UpdateAttribute,
        UPDATE_ATTRIBUTE_ACCEPT_LIMIT,
        UPDATE_ATTRIBUTE_FETCH_LIMIT,
        UPDATE_ATTRIBUTE_ACCEPT_LIMIT,
        budget,
        decode_update_attribute,
    )?;
    let mut expected = HashMap::<u32, u32>::new();
    for spec in ATTACHMENT_SPECS.iter().filter(|spec| spec.update_of_status) {
        let trigger = triggers.get(&trigger_key(spec)).ok_or_else(|| {
            SimulationTriggerMetadataError::primary(PrimaryFailure::CatalogInvalid)
        })?;
        expected.insert(trigger.trigger_oid, trigger.relation_oid);
    }
    let mut seen = HashSet::with_capacity(UPDATE_ATTRIBUTE_ACCEPT_LIMIT);
    let mut previous_oid = 0;
    for row in decoded {
        let trigger_oid = positive_oid(row.trigger_oid)?;
        let relation_oid = positive_oid(row.relation_oid)?;
        let ordinal = required(row.ordinal)?;
        let attribute_number = required(row.attribute_number)?;
        let attribute_relation_oid = positive_oid(required(row.attribute_relation_oid)?)?;
        if trigger_oid <= previous_oid
            || expected.get(&trigger_oid) != Some(&relation_oid)
            || ordinal != 1
            || attribute_number <= 0
            || attribute_number > i32::from(i16::MAX)
            || row.attribute_name.as_deref() != Some("status")
            || attribute_relation_oid != relation_oid
            || row.is_dropped != Some(false)
            || !seen.insert(trigger_oid)
        {
            return Err(SimulationTriggerMetadataError::primary(
                PrimaryFailure::PolicyRejected,
            ));
        }
        previous_oid = trigger_oid;
    }
    if seen.len() != UPDATE_ATTRIBUTE_ACCEPT_LIMIT {
        return Err(SimulationTriggerMetadataError::primary(
            PrimaryFailure::CatalogInvalid,
        ));
    }
    Ok(())
}

fn validate_user_constraints(
    rows: Vec<RawRow>,
    triggers: &HashMap<AttachmentKey, TriggerEvidence>,
    context: &ContextEvidence,
    budget: &mut LogicalPayloadBudget,
) -> Result<[u32; USER_CONSTRAINT_ACCEPT_LIMIT], SimulationTriggerMetadataError> {
    let decoded = decode_many(
        rows,
        RawCategory::UserConstraint,
        USER_CONSTRAINT_ACCEPT_LIMIT,
        USER_CONSTRAINT_FETCH_LIMIT,
        USER_CONSTRAINT_ACCEPT_LIMIT,
        budget,
        decode_user_constraint,
    )?;
    let mut expected = HashMap::<u32, (u32, u32, bool, bool)>::new();
    for spec in ATTACHMENT_SPECS.iter().filter(|spec| spec.constraint) {
        let trigger = triggers.get(&trigger_key(spec)).ok_or_else(|| {
            SimulationTriggerMetadataError::primary(PrimaryFailure::CatalogInvalid)
        })?;
        expected.insert(
            trigger.trigger_oid,
            (
                trigger.constraint_oid.ok_or_else(|| {
                    SimulationTriggerMetadataError::primary(PrimaryFailure::CatalogInvalid)
                })?,
                trigger.relation_oid,
                trigger.is_deferrable,
                trigger.initially_deferred,
            ),
        );
    }
    let mut seen_triggers = HashSet::with_capacity(USER_CONSTRAINT_ACCEPT_LIMIT);
    let mut seen_constraints = HashSet::with_capacity(USER_CONSTRAINT_ACCEPT_LIMIT);
    let mut constraint_oids = [0; USER_CONSTRAINT_ACCEPT_LIMIT];
    let mut previous_trigger_oid = 0;
    for (index, row) in decoded.into_iter().enumerate() {
        let trigger_oid = positive_oid(row.trigger_oid)?;
        let relation_oid = positive_oid(row.relation_oid)?;
        let trigger_constraint_oid = positive_oid(row.trigger_constraint_oid)?;
        let constraint_oid = positive_oid(required(row.constraint_oid)?)?;
        let constraint_relation_oid = positive_oid(required(row.constraint_relation_oid)?)?;
        let constraint_schema_oid = positive_oid(required(row.constraint_schema_oid)?)?;
        let (expected_oid, expected_relation_oid, expected_deferrable, expected_initially_deferred) =
            expected.get(&trigger_oid).ok_or_else(|| {
                SimulationTriggerMetadataError::primary(PrimaryFailure::PolicyRejected)
            })?;
        if trigger_oid <= previous_trigger_oid
            || trigger_constraint_oid != *expected_oid
            || constraint_oid != *expected_oid
            || relation_oid != *expected_relation_oid
            || constraint_relation_oid != *expected_relation_oid
            || constraint_schema_oid != context.schema_oid
            || row.constraint_schema_name.as_deref() != Some(PUBLIC_SCHEMA)
            || row.kind.as_deref() != Some("t")
            || row.is_deferrable != Some(*expected_deferrable)
            || row.initially_deferred != Some(*expected_initially_deferred)
            || row.constraint_type_oid != Some(0)
            || row.index_oid != Some(0)
            || row.foreign_relation_oid != Some(0)
            || !seen_triggers.insert(trigger_oid)
            || !seen_constraints.insert(constraint_oid)
        {
            return Err(SimulationTriggerMetadataError::primary(
                PrimaryFailure::PolicyRejected,
            ));
        }
        previous_trigger_oid = trigger_oid;
        constraint_oids[index] = constraint_oid;
    }
    if seen_triggers.len() != USER_CONSTRAINT_ACCEPT_LIMIT
        || expected.len() != USER_CONSTRAINT_ACCEPT_LIMIT
    {
        return Err(SimulationTriggerMetadataError::primary(
            PrimaryFailure::CatalogInvalid,
        ));
    }
    Ok(constraint_oids)
}

fn is_canceled(canceled: &AtomicBool) -> bool {
    canceled.load(Ordering::Acquire)
}

fn map_transport_failure(failure: TransportFailure, fallback: PrimaryFailure) -> PrimaryFailure {
    match failure {
        TransportFailure::Database if fallback == PrimaryFailure::SetupFailed => {
            PrimaryFailure::SetupFailed
        }
        TransportFailure::Database => PrimaryFailure::CatalogUnavailable,
        TransportFailure::State => PrimaryFailure::CatalogInvalid,
        TransportFailure::FetchBound => PrimaryFailure::BoundExceeded,
    }
}

async fn execute_checked<T: ObserverTransport>(
    transport: &mut T,
    statement: Statement,
    canceled: &AtomicBool,
    budget: &mut LogicalPayloadBudget,
    fallback: PrimaryFailure,
) -> Result<Vec<RawRow>, SimulationTriggerMetadataError> {
    if is_canceled(canceled) {
        let mut error = SimulationTriggerMetadataError::primary(PrimaryFailure::Canceled);
        error.canceled = true;
        return Err(error);
    }
    match transport.execute(statement).await {
        Err(failure) => {
            let mut error =
                SimulationTriggerMetadataError::primary(map_transport_failure(failure, fallback));
            error.canceled = is_canceled(canceled);
            Err(error)
        }
        Ok(rows) if is_canceled(canceled) => {
            for row in &rows {
                let _ = budget.charge_row(row);
            }
            let mut error = SimulationTriggerMetadataError::primary(PrimaryFailure::Canceled);
            error.canceled = true;
            Err(error)
        }
        Ok(rows) => Ok(rows),
    }
}

async fn prepare_observation<T: ObserverTransport>(
    transport: &mut T,
    canceled: &AtomicBool,
    timeout_ms: i64,
    budget: &mut LogicalPayloadBudget,
) -> Result<PreparedObservation, SimulationTriggerMetadataError> {
    let setup = execute_checked(
        transport,
        Statement::Configure { timeout_ms },
        canceled,
        budget,
        PrimaryFailure::SetupFailed,
    )
    .await?;
    validate_setup(setup, timeout_ms, budget)?;

    let context_rows = execute_checked(
        transport,
        Statement::Context,
        canceled,
        budget,
        PrimaryFailure::CatalogUnavailable,
    )
    .await?;
    let context = validate_context(
        decode_context_rows(
            context_rows,
            INITIAL_CONTEXT_ACCEPT_LIMIT,
            INITIAL_CONTEXT_FETCH_LIMIT,
            budget,
        )?,
        timeout_ms,
    )?;

    let roles_rows = execute_checked(
        transport,
        Statement::Roles,
        canceled,
        budget,
        PrimaryFailure::CatalogUnavailable,
    )
    .await?;
    let roles = validate_roles(roles_rows, &context, budget)?;

    let membership_rows = execute_checked(
        transport,
        Statement::WorkerMemberships {
            worker_oid: context.worker_oid,
        },
        canceled,
        budget,
        PrimaryFailure::CatalogUnavailable,
    )
    .await?;
    validate_worker_memberships(membership_rows, context.worker_oid, budget)?;

    let relation_rows = execute_checked(
        transport,
        Statement::Relations,
        canceled,
        budget,
        PrimaryFailure::CatalogUnavailable,
    )
    .await?;
    let relations = validate_relations(relation_rows, &context, &roles, budget)?;

    let inheritance_rows = execute_checked(
        transport,
        Statement::PrimaryInheritance {
            relation_oids: primary_relation_oids(&relations)?,
        },
        canceled,
        budget,
        PrimaryFailure::CatalogUnavailable,
    )
    .await?;
    validate_primary_inheritance(inheritance_rows, budget)?;

    let root_header_rows = execute_checked(
        transport,
        Statement::RootHeaders,
        canceled,
        budget,
        PrimaryFailure::CatalogUnavailable,
    )
    .await?;
    let roots = validate_root_headers(root_header_rows, &context, &roles, budget)?;
    let root_oids = std::array::from_fn(|index| roots[index].oid);

    let root_setting_rows = execute_checked(
        transport,
        Statement::RootSettings { root_oids },
        canceled,
        budget,
        PrimaryFailure::CatalogUnavailable,
    )
    .await?;
    validate_root_settings(root_setting_rows, &roots, budget)?;

    let forward_rows = execute_checked(
        transport,
        Statement::ForwardTriggers {
            relation_oids: primary_relation_oids(&relations)?,
            root_oids,
        },
        canceled,
        budget,
        PrimaryFailure::CatalogUnavailable,
    )
    .await?;
    let adjacent_rows = execute_checked(
        transport,
        Statement::AdjacentConfigHook {
            relation_oid: adjacent_relation_oid(&relations)?,
        },
        canceled,
        budget,
        PrimaryFailure::CatalogUnavailable,
    )
    .await?;
    let reverse_rows = execute_checked(
        transport,
        Statement::ReverseProtectedHooks { root_oids },
        canceled,
        budget,
        PrimaryFailure::CatalogUnavailable,
    )
    .await?;
    let (triggers, trigger_oids) = validate_trigger_census(
        forward_rows,
        adjacent_rows,
        reverse_rows,
        &context,
        &relations,
        &roles,
        &roots,
        budget,
    )?;

    let update_oids = update_trigger_oids(&triggers)?;
    let update_rows = execute_checked(
        transport,
        Statement::UpdateOfAttributes {
            trigger_oids: update_oids,
        },
        canceled,
        budget,
        PrimaryFailure::CatalogUnavailable,
    )
    .await?;
    validate_update_attributes(update_rows, &triggers, budget)?;

    let constrained_oids = constraint_trigger_oids(&triggers)?;
    let constraint_rows = execute_checked(
        transport,
        Statement::UserTriggerConstraints {
            trigger_oids: constrained_oids,
        },
        canceled,
        budget,
        PrimaryFailure::CatalogUnavailable,
    )
    .await?;
    let constraint_oids = validate_user_constraints(constraint_rows, &triggers, &context, budget)?;

    let final_context_rows = execute_checked(
        transport,
        Statement::FinalContext,
        canceled,
        budget,
        PrimaryFailure::CatalogUnavailable,
    )
    .await?;
    let final_context = validate_context(
        decode_context_rows(
            final_context_rows,
            FINAL_CONTEXT_ACCEPT_LIMIT,
            FINAL_CONTEXT_FETCH_LIMIT,
            budget,
        )?,
        timeout_ms,
    )?;
    if final_context != context {
        return Err(SimulationTriggerMetadataError::primary(
            PrimaryFailure::PolicyRejected,
        ));
    }

    let relation_oids = std::array::from_fn(|index| relations.ordered[index]);
    Ok(PreparedObservation {
        context,
        relations: RelationEvidence {
            ordered: relation_oids,
            by_name: relations.by_name,
        },
        root_oids,
        trigger_oids,
        constraint_oids,
    })
}

async fn close_before_begin<T: ObserverTransport>(
    transport: &mut T,
    mut error: SimulationTriggerMetadataError,
    canceled: &AtomicBool,
) -> SimulationTriggerMetadataError {
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
    mut error: SimulationTriggerMetadataError,
    canceled: &AtomicBool,
) -> SimulationTriggerMetadataError {
    error.rollback = if matches!(transport.execute(Statement::Rollback).await, Ok(rows) if rows.is_empty())
    {
        CleanupState::Confirmed
    } else {
        CleanupState::Unconfirmed
    };
    error.canceled |= is_canceled(canceled);
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
) -> Result<SimulationTriggerMetadataObservation, SimulationTriggerMetadataError> {
    let timeout_ms = timeout_millis(config)?;
    let mut budget = LogicalPayloadBudget::default();
    if is_canceled(canceled) {
        let mut error = SimulationTriggerMetadataError::primary(PrimaryFailure::Canceled);
        error.canceled = true;
        return Err(error);
    }
    if transport.acquire().await.is_err() {
        let mut error = SimulationTriggerMetadataError::primary(PrimaryFailure::AcquisitionFailed);
        error.canceled = is_canceled(canceled);
        return Err(error);
    }

    // Close-on-drop is armed before the first post-acquisition await.
    transport.arm_close_on_drop();
    if is_canceled(canceled) {
        return Err(close_before_begin(
            transport,
            SimulationTriggerMetadataError::primary(PrimaryFailure::Canceled),
            canceled,
        )
        .await);
    }

    match transport.execute(Statement::Begin).await {
        Ok(rows) if rows.is_empty() => {}
        Ok(_) => {
            let mut error =
                SimulationTriggerMetadataError::primary(PrimaryFailure::BeginResultInvalid);
            error.begin = BeginState::Uncertain;
            error.canceled = is_canceled(canceled);
            return Err(rollback_and_close(transport, error, canceled).await);
        }
        Err(_) => {
            let mut error = SimulationTriggerMetadataError::primary(PrimaryFailure::BeginFailed);
            error.begin = BeginState::Uncertain;
            error.canceled = is_canceled(canceled);
            return Err(rollback_and_close(transport, error, canceled).await);
        }
    }
    if is_canceled(canceled) {
        let mut error = SimulationTriggerMetadataError::primary(PrimaryFailure::Canceled);
        error.begin = BeginState::Started;
        return Err(rollback_and_close(transport, error, canceled).await);
    }

    let prepared = match prepare_observation(transport, canceled, timeout_ms, &mut budget).await {
        Ok(prepared) => prepared,
        Err(mut error) => {
            error.begin = BeginState::Started;
            return Err(rollback_and_close(transport, error, canceled).await);
        }
    };
    if is_canceled(canceled) {
        let mut error = SimulationTriggerMetadataError::primary(PrimaryFailure::Canceled);
        error.begin = BeginState::Started;
        return Err(rollback_and_close(transport, error, canceled).await);
    }

    let commit = transport.execute(Statement::Commit).await;
    let canceled_during_commit = is_canceled(canceled);
    let completion = match commit {
        Ok(rows) if rows.is_empty() => CompletionState::Acknowledged,
        _ => CompletionState::Unconfirmed,
    };
    if completion == CompletionState::Unconfirmed {
        let error = SimulationTriggerMetadataError {
            primary: PrimaryFailure::CompletionUnconfirmed,
            begin: BeginState::Started,
            completion,
            rollback: CleanupState::NotApplicable,
            close: CleanupState::NotApplicable,
            canceled: canceled_during_commit || is_canceled(canceled),
        };
        return Err(rollback_and_close(transport, error, canceled).await);
    }

    let close = if transport.close().await.is_ok() {
        CleanupState::Confirmed
    } else {
        CleanupState::Unconfirmed
    };
    let canceled_after_close = is_canceled(canceled);
    if close == CleanupState::Unconfirmed {
        return Err(SimulationTriggerMetadataError {
            primary: PrimaryFailure::CleanupUnconfirmed,
            begin: BeginState::Started,
            completion,
            rollback: CleanupState::NotApplicable,
            close,
            canceled: canceled_during_commit || canceled_after_close,
        });
    }
    if canceled_during_commit || canceled_after_close {
        return Err(SimulationTriggerMetadataError {
            primary: PrimaryFailure::Canceled,
            begin: BeginState::Started,
            completion,
            rollback: CleanupState::NotApplicable,
            close,
            canceled: true,
        });
    }
    Ok(SimulationTriggerMetadataObservation::completed(
        config.config_sha256(),
        prepared,
        budget.used,
    ))
}

/// Observe this finite metadata candidate on a fresh disposable connection.
/// Dropping the future after acquisition disposes the armed connection but is
/// not an awaited rollback or close acknowledgement.
pub(super) async fn observe_simulation_trigger_metadata(
    pool: &PgPool,
    config: &SimulationProcessConfig,
    canceled: &AtomicBool,
) -> Result<SimulationTriggerMetadataObservation, SimulationTriggerMetadataError> {
    let mut transport = SqlxObserverTransport::new(pool);
    observe_with_transport(&mut transport, config, canceled).await
}

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PrimaryFailure {
    InvalidBound,
    Canceled,
    AcquisitionFailed,
    BeginFailed,
    BeginResultInvalid,
    SetupFailed,
    SetupResultInvalid,
    CatalogUnavailable,
    CatalogInvalid,
    PolicyRejected,
    BoundExceeded,
    PayloadExceeded,
    CompletionUnconfirmed,
    CleanupUnconfirmed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BeginState {
    NotAttempted,
    Uncertain,
    Started,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CompletionState {
    NotAttempted,
    Acknowledged,
    Unconfirmed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CleanupState {
    NotApplicable,
    Confirmed,
    Unconfirmed,
}

/// Value-free failure classification. No SQL error, provider text, or catalog
/// value is retained in an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SimulationTriggerMetadataError {
    primary: PrimaryFailure,
    begin: BeginState,
    completion: CompletionState,
    rollback: CleanupState,
    close: CleanupState,
    canceled: bool,
}

impl SimulationTriggerMetadataError {
    fn primary(primary: PrimaryFailure) -> Self {
        Self {
            primary,
            begin: BeginState::NotAttempted,
            completion: CompletionState::NotAttempted,
            rollback: CleanupState::NotApplicable,
            close: CleanupState::NotApplicable,
            canceled: false,
        }
    }
}

impl fmt::Display for SimulationTriggerMetadataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("simulation trigger metadata observation failed closed")
    }
}

impl std::error::Error for SimulationTriggerMetadataError {}

/// Opaque SOURCE candidate produced only after a validated transaction,
/// acknowledged commit, and awaited connection close.
#[allow(dead_code)] // Metadata pins are retained for private review.
pub(super) struct SimulationTriggerMetadataObservation {
    config_sha256: String,
    policy_id: &'static str,
    policy_fingerprint: &'static str,
    database_oid: u32,
    worker_oid: u32,
    schema_oid: u32,
    migration_owner_oid: u32,
    relation_oids: [u32; RELATION_IDENTITY_ACCEPT_LIMIT],
    root_oids: [u32; ROOT_HEADER_ACCEPT_LIMIT],
    trigger_oids: [u32; REVERSE_HOOK_ACCEPT_LIMIT],
    constraint_oids: [u32; USER_CONSTRAINT_ACCEPT_LIMIT],
    logical_bytes: usize,
}

impl SimulationTriggerMetadataObservation {
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
        self.schema_oid
    }
    pub(super) fn migration_owner_oid(&self) -> u32 {
        self.migration_owner_oid
    }

    fn completed(config_sha256: &str, prepared: PreparedObservation, logical_bytes: usize) -> Self {
        Self {
            config_sha256: config_sha256.to_owned(),
            policy_id: POLICY_ID,
            policy_fingerprint: POLICY_FINGERPRINT,
            database_oid: prepared.context.database_oid,
            worker_oid: prepared.context.worker_oid,
            schema_oid: prepared.context.schema_oid,
            migration_owner_oid: prepared.context.migration_owner_oid,
            relation_oids: prepared.relations.ordered,
            root_oids: prepared.root_oids,
            trigger_oids: prepared.trigger_oids,
            constraint_oids: prepared.constraint_oids,
            logical_bytes,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Statement {
    Begin,
    Configure {
        timeout_ms: i64,
    },
    Context,
    Roles,
    WorkerMemberships {
        worker_oid: u32,
    },
    Relations,
    PrimaryInheritance {
        relation_oids: [u32; 4],
    },
    RootHeaders,
    RootSettings {
        root_oids: [u32; ROOT_HEADER_ACCEPT_LIMIT],
    },
    ForwardTriggers {
        relation_oids: [u32; 4],
        root_oids: [u32; ROOT_HEADER_ACCEPT_LIMIT],
    },
    AdjacentConfigHook {
        relation_oid: u32,
    },
    ReverseProtectedHooks {
        root_oids: [u32; ROOT_HEADER_ACCEPT_LIMIT],
    },
    UpdateOfAttributes {
        trigger_oids: [u32; UPDATE_ATTRIBUTE_ACCEPT_LIMIT],
    },
    UserTriggerConstraints {
        trigger_oids: [u32; USER_CONSTRAINT_ACCEPT_LIMIT],
    },
    FinalContext,
    Commit,
    Rollback,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum StatementKind {
    Begin,
    Configure,
    Context,
    Roles,
    WorkerMemberships,
    Relations,
    PrimaryInheritance,
    RootHeaders,
    RootSettings,
    ForwardTriggers,
    AdjacentConfigHook,
    ReverseProtectedHooks,
    UpdateOfAttributes,
    UserTriggerConstraints,
    FinalContext,
    Commit,
    Rollback,
}

impl Statement {
    fn kind(self) -> StatementKind {
        match self {
            Self::Begin => StatementKind::Begin,
            Self::Configure { .. } => StatementKind::Configure,
            Self::Context => StatementKind::Context,
            Self::Roles => StatementKind::Roles,
            Self::WorkerMemberships { .. } => StatementKind::WorkerMemberships,
            Self::Relations => StatementKind::Relations,
            Self::PrimaryInheritance { .. } => StatementKind::PrimaryInheritance,
            Self::RootHeaders => StatementKind::RootHeaders,
            Self::RootSettings { .. } => StatementKind::RootSettings,
            Self::ForwardTriggers { .. } => StatementKind::ForwardTriggers,
            Self::AdjacentConfigHook { .. } => StatementKind::AdjacentConfigHook,
            Self::ReverseProtectedHooks { .. } => StatementKind::ReverseProtectedHooks,
            Self::UpdateOfAttributes { .. } => StatementKind::UpdateOfAttributes,
            Self::UserTriggerConstraints { .. } => StatementKind::UserTriggerConstraints,
            Self::FinalContext => StatementKind::FinalContext,
            Self::Commit => StatementKind::Commit,
            Self::Rollback => StatementKind::Rollback,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
enum BindValue {
    I64(i64),
    I64Array(Vec<i64>),
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct StatementPlan {
    kind: StatementKind,
    sql: &'static str,
    binds: Vec<BindValue>,
    category: Option<RawCategory>,
    shape: &'static [PgCellKind],
    fetch_limit: usize,
}

fn oid_bind_array<const N: usize>(values: [u32; N]) -> Vec<i64> {
    values.into_iter().map(i64::from).collect()
}

fn timeout_bind_value(timeout_ms: i64) -> i64 {
    timeout_ms
}

fn statement_plan(statement: Statement) -> StatementPlan {
    let kind = statement.kind();
    let (sql, binds, category, shape, fetch_limit) = match statement {
        Statement::Begin => (BEGIN_SQL, Vec::new(), None, &[][..], 1),
        Statement::Configure { timeout_ms } => (
            SETUP_SQL,
            vec![BindValue::I64(timeout_bind_value(timeout_ms))],
            Some(RawCategory::Setup),
            &SETUP_ROW[..],
            SETUP_FETCH_LIMIT,
        ),
        Statement::Context => (
            CONTEXT_SQL,
            Vec::new(),
            Some(RawCategory::Context),
            &CONTEXT_ROW[..],
            INITIAL_CONTEXT_FETCH_LIMIT,
        ),
        Statement::Roles => (
            ROLES_SQL,
            Vec::new(),
            Some(RawCategory::Role),
            &ROLE_ROW[..],
            ROLE_FETCH_LIMIT,
        ),
        Statement::WorkerMemberships { worker_oid } => (
            WORKER_MEMBERSHIPS_SQL,
            vec![BindValue::I64(i64::from(worker_oid))],
            Some(RawCategory::Membership),
            &MEMBERSHIP_ROW[..],
            WORKER_MEMBERSHIP_FETCH_LIMIT,
        ),
        Statement::Relations => (
            RELATIONS_SQL,
            Vec::new(),
            Some(RawCategory::Relation),
            &RELATION_ROW[..],
            RELATION_IDENTITY_FETCH_LIMIT,
        ),
        Statement::PrimaryInheritance { relation_oids } => (
            PRIMARY_INHERITANCE_SQL,
            vec![BindValue::I64Array(oid_bind_array(relation_oids))],
            Some(RawCategory::Inheritance),
            &INHERITANCE_ROW[..],
            PRIMARY_INHERITANCE_FETCH_LIMIT,
        ),
        Statement::RootHeaders => (
            ROOT_HEADERS_SQL,
            Vec::new(),
            Some(RawCategory::RootHeader),
            &ROOT_HEADER_ROW[..],
            ROOT_HEADER_FETCH_LIMIT,
        ),
        Statement::RootSettings { root_oids } => (
            ROOT_SETTINGS_SQL,
            vec![BindValue::I64Array(oid_bind_array(root_oids))],
            Some(RawCategory::RootSetting),
            &ROOT_SETTING_ROW[..],
            ROOT_SETTING_FETCH_LIMIT,
        ),
        Statement::ForwardTriggers {
            relation_oids,
            root_oids,
        } => (
            FORWARD_TRIGGERS_SQL,
            vec![
                BindValue::I64Array(oid_bind_array(relation_oids)),
                BindValue::I64Array(oid_bind_array(root_oids)),
            ],
            Some(RawCategory::ForwardTrigger),
            &TRIGGER_ROW[..],
            FORWARD_TRIGGER_FETCH_LIMIT,
        ),
        Statement::AdjacentConfigHook { relation_oid } => (
            ADJACENT_CONFIG_HOOK_SQL,
            vec![BindValue::I64(i64::from(relation_oid))],
            Some(RawCategory::AdjacentHook),
            &TRIGGER_ROW[..],
            ADJACENT_HOOK_FETCH_LIMIT,
        ),
        Statement::ReverseProtectedHooks { root_oids } => (
            REVERSE_PROTECTED_HOOKS_SQL,
            vec![BindValue::I64Array(oid_bind_array(root_oids))],
            Some(RawCategory::ReverseHook),
            &TRIGGER_ROW[..],
            REVERSE_HOOK_FETCH_LIMIT,
        ),
        Statement::UpdateOfAttributes { trigger_oids } => (
            UPDATE_OF_ATTRIBUTES_SQL,
            vec![BindValue::I64Array(oid_bind_array(trigger_oids))],
            Some(RawCategory::UpdateAttribute),
            &UPDATE_ATTRIBUTE_ROW[..],
            UPDATE_ATTRIBUTE_FETCH_LIMIT,
        ),
        Statement::UserTriggerConstraints { trigger_oids } => (
            USER_TRIGGER_CONSTRAINTS_SQL,
            vec![BindValue::I64Array(oid_bind_array(trigger_oids))],
            Some(RawCategory::UserConstraint),
            &USER_CONSTRAINT_ROW[..],
            USER_CONSTRAINT_FETCH_LIMIT,
        ),
        Statement::FinalContext => (
            CONTEXT_SQL,
            Vec::new(),
            Some(RawCategory::Context),
            &CONTEXT_ROW[..],
            FINAL_CONTEXT_FETCH_LIMIT,
        ),
        Statement::Commit => ("COMMIT", Vec::new(), None, &[][..], 1),
        Statement::Rollback => ("ROLLBACK", Vec::new(), None, &[][..], 1),
    };
    StatementPlan {
        kind,
        sql,
        binds,
        category,
        shape,
        fetch_limit,
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

    async fn fetch_rows(&mut self, plan: StatementPlan) -> Result<Vec<RawRow>, TransportFailure> {
        let query = match plan.binds.as_slice() {
            [] => sqlx::query(plan.sql),
            [BindValue::I64(value)] => sqlx::query(plan.sql).bind(*value),
            [BindValue::I64Array(values)] => sqlx::query(plan.sql).bind(values.clone()),
            [BindValue::I64Array(first), BindValue::I64Array(second)] => sqlx::query(plan.sql)
                .bind(first.clone())
                .bind(second.clone()),
            _ => return Err(TransportFailure::State),
        };
        let rows = query
            .fetch_all(self.connection()?)
            .await
            .map_err(|_| TransportFailure::Database)?;
        raw_pg_rows(
            rows,
            plan.category.ok_or(TransportFailure::State)?,
            plan.shape,
            plan.fetch_limit,
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
        let plan = statement_plan(statement);
        if plan.category.is_some() {
            return self.fetch_rows(plan).await;
        }
        if !plan.binds.is_empty()
            || !plan.shape.is_empty()
            || plan.fetch_limit != 1
            || !matches!(
                plan.kind,
                StatementKind::Begin | StatementKind::Commit | StatementKind::Rollback
            )
        {
            return Err(TransportFailure::State);
        }
        let row = sqlx::query(plan.sql)
            .fetch_optional(self.connection()?)
            .await
            .map_err(|_| TransportFailure::Database)?;
        Ok(classify_command_result(row))
    }

    async fn close(&mut self) -> Result<(), TransportFailure> {
        let connection = self.connection.take().ok_or(TransportFailure::State)?;
        connection
            .close()
            .await
            .map_err(|_| TransportFailure::Database)
    }
}

fn pg_column_count(shape: &[PgCellKind]) -> usize {
    shape
        .iter()
        .map(|kind| match kind {
            PgCellKind::Text { .. } | PgCellKind::Bytes { .. } => 3,
            _ => 1,
        })
        .sum()
}

fn column_matches(row: &PgRow, index: usize, name: &str, type_name: &str) -> bool {
    row.columns()
        .get(index)
        .is_some_and(|column| column.name() == name && column.type_info().name() == type_name)
}

fn raw_pg_cell(row: &PgRow, index: &mut usize, kind: PgCellKind) -> RawCell {
    match kind {
        PgCellKind::Bool(alias) | PgCellKind::OptionalBool(alias) => {
            let at = *index;
            *index += 1;
            if !column_matches(row, at, alias, "BOOL") {
                return RawCell::WrongType;
            }
            match row.try_get::<Option<bool>, _>(at) {
                Ok(Some(value)) => RawCell::Bool(value),
                Ok(None) if matches!(kind, PgCellKind::OptionalBool(_)) => RawCell::Null,
                _ => RawCell::WrongType,
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
                Ok(None) if matches!(kind, PgCellKind::OptionalI32(_)) => RawCell::Null,
                _ => RawCell::WrongType,
            }
        }
        PgCellKind::I64(alias) | PgCellKind::OptionalI64(alias) => {
            let at = *index;
            *index += 1;
            if !column_matches(row, at, alias, "INT8") {
                return RawCell::WrongType;
            }
            match row.try_get::<Option<i64>, _>(at) {
                Ok(Some(value)) => RawCell::I64(value),
                Ok(None) if matches!(kind, PgCellKind::OptionalI64(_)) => RawCell::Null,
                _ => RawCell::WrongType,
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
                row.try_get_raw(value_at),
            ) {
                (Ok(Some(is_null)), Ok(original_bytes), Ok(value)) if !value.is_null() => {
                    match value.as_bytes() {
                        Ok(value) => RawCell::Text {
                            is_null,
                            original_bytes,
                            value: value.to_vec(),
                        },
                        Err(_) => RawCell::WrongType,
                    }
                }
                _ => RawCell::WrongType,
            }
        }
        PgCellKind::Bytes {
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
                || !column_matches(row, value_at, value_alias, "BYTEA")
            {
                return RawCell::WrongType;
            }
            match (
                row.try_get::<Option<bool>, _>(null_at),
                row.try_get::<Option<i64>, _>(bytes_at),
                row.try_get::<Option<Vec<u8>>, _>(value_at),
            ) {
                (Ok(Some(is_null)), Ok(original_bytes), Ok(Some(prefix))) => RawCell::Bytes {
                    is_null,
                    original_bytes,
                    prefix,
                },
                _ => RawCell::WrongType,
            }
        }
    }
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
    Ok(rows
        .into_iter()
        .map(|row| {
            if row.columns().len() != pg_column_count(shape) {
                return RawRow::new(category, vec![RawCell::WrongType; shape.len()]);
            }
            let mut index = 0;
            let cells = shape
                .iter()
                .map(|kind| raw_pg_cell(&row, &mut index, *kind))
                .collect();
            RawRow::new(category, cells)
        })
        .collect())
}
