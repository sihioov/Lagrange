//! Private, bounded observation of SQLx's finite migration ledger.
//!
//! This records one completed dedicated-connection observation. It is not
//! startup readiness, a pool-wide claim, migration execution proof, rights
//! authorization, or reusable permission.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use sha2::{Digest, Sha256, Sha384};
use sqlx::{PgConnection, PgPool, Postgres, Row, pool::PoolConnection, postgres::PgRow};

use super::SimulationProcessConfig;

const POLICY_ID: &str = "r4-simulation-ledger-baseline-64-v1";
const READER_FLOOR: u64 = 63;
const REQUIRED_EXTENSION: u64 = 64;
const EXPECTED_ROW_COUNT: usize = 64;
const EXPECTED_FINGERPRINT: &str =
    "sha256:a4886f4c56091690702790d1defacf3b1a388c718a94c026822e02a4b580d91f";
const WORKER_ROLE: &str = "worker";
const MIGRATION_OWNER_ROLE: &str = "migration_owner";
const PUBLIC_SCHEMA: &str = "public";
const LEDGER_NAME: &str = "_sqlx_migrations";
const PAYLOAD_LIMIT: usize = 16 * 1024;

const CONTEXT_FETCH_LIMIT: usize = 2;
const MEMBERSHIP_FETCH_LIMIT: usize = 1;
const RELATION_FETCH_LIMIT: usize = 2;
const COLUMN_FETCH_LIMIT: usize = 7;
const PRIMARY_KEY_FETCH_LIMIT: usize = 2;
const RAW_ACL_FETCH_LIMIT: usize = 65;
const RAW_ACL_ACCEPT_LIMIT: usize = 64;
const LEDGER_FETCH_LIMIT: usize = 65;
const LEDGER_ACCEPT_LIMIT: usize = 64;

const BEGIN_SQL: &str = "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY";
const SETUP_SQL: &str = "\
    SELECT pg_catalog.set_config('search_path', 'pg_catalog', true), \
           pg_catalog.set_config('statement_timeout', $1::pg_catalog.text, true), \
           pg_catalog.set_config('lock_timeout', $1::pg_catalog.text, true)";

const CONTEXT_SQL: &str = r#"
SELECT current_user::pg_catalog.text AS current_user,
       session_user::pg_catalog.text AS session_user,
       pg_catalog.current_database()::pg_catalog.text AS observed_database_name,
       database.datname::pg_catalog.text AS database_name,
       public_schema.nspname::pg_catalog.text AS schema_name,
       worker.oid::pg_catalog.int8 AS worker_oid,
       database.oid::pg_catalog.int8 AS database_oid,
       public_schema.oid::pg_catalog.int8 AS schema_oid,
       migration_owner.oid::pg_catalog.int8 AS migration_owner_oid,
       database.datdba::pg_catalog.int8 AS database_owner_oid,
       public_schema.nspowner::pg_catalog.int8 AS schema_owner_oid,
       worker.rolcanlogin AS can_login,
       worker.rolsuper AS is_superuser,
       worker.rolcreatedb AS can_create_database,
       worker.rolcreaterole AS can_create_role,
       worker.rolreplication AS can_replicate,
       worker.rolbypassrls AS bypasses_rls,
       pg_catalog.current_setting('row_security')::pg_catalog.text AS row_security,
       pg_catalog.current_setting('session_replication_role')::pg_catalog.text
           AS session_replication_role,
       pg_catalog.current_setting('transaction_read_only')::pg_catalog.bool
           AS transaction_read_only,
       pg_catalog.current_setting('transaction_isolation')::pg_catalog.text
           AS transaction_isolation,
       pg_catalog.has_database_privilege(current_user, database.oid, 'CONNECT')
           AS database_connect,
       pg_catalog.has_database_privilege(current_user, database.oid, 'CREATE')
           AS database_create,
       pg_catalog.has_schema_privilege(current_user, public_schema.oid, 'USAGE')
           AS schema_usage,
       pg_catalog.has_schema_privilege(current_user, public_schema.oid, 'CREATE')
           AS schema_create,
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
       ) AS database_effective_grant_option,
       (
           pg_catalog.has_schema_privilege(
               current_user, public_schema.oid, 'USAGE WITH GRANT OPTION'
           )
           OR pg_catalog.has_schema_privilege(
               current_user, public_schema.oid, 'CREATE WITH GRANT OPTION'
           )
       ) AS schema_effective_grant_option,
       EXISTS (
           SELECT 1
             FROM pg_catalog.aclexplode(
                 COALESCE(
                     database.datacl,
                     pg_catalog.acldefault('d', database.datdba)
                 )
             ) AS acl
            WHERE acl.grantee IN (0::pg_catalog.oid, worker.oid)
              AND acl.is_grantable
              AND acl.privilege_type IN ('CONNECT', 'CREATE', 'TEMPORARY')
       ) AS database_raw_grant_option,
       EXISTS (
           SELECT 1
             FROM pg_catalog.aclexplode(
                 COALESCE(
                     public_schema.nspacl,
                     pg_catalog.acldefault('n', public_schema.nspowner)
                 )
             ) AS acl
            WHERE acl.grantee IN (0::pg_catalog.oid, worker.oid)
              AND acl.is_grantable
              AND acl.privilege_type IN ('USAGE', 'CREATE')
       ) AS schema_raw_grant_option
  FROM pg_catalog.pg_database AS database
  JOIN pg_catalog.pg_namespace AS public_schema
    ON public_schema.nspname = 'public'
  JOIN pg_catalog.pg_roles AS worker
    ON worker.rolname = current_user
  JOIN pg_catalog.pg_roles AS migration_owner
    ON migration_owner.rolname = 'migration_owner'
 WHERE database.datname = pg_catalog.current_database()
 LIMIT 2
"#;

const MEMBERSHIPS_SQL: &str = r#"
SELECT membership.member::pg_catalog.int8 AS member_oid,
       membership.roleid::pg_catalog.int8 AS role_oid,
       membership.grantor::pg_catalog.int8 AS grantor_oid,
       membership.admin_option,
       membership.inherit_option,
       membership.set_option
  FROM pg_catalog.pg_auth_members AS membership
 WHERE membership.member = $1::pg_catalog.oid
 ORDER BY membership.roleid, membership.grantor
 LIMIT 1
"#;

const RELATION_SQL: &str = r#"
SELECT namespace.oid::pg_catalog.int8 AS schema_oid,
       namespace.nspname::pg_catalog.text AS schema_name,
       relation.relname::pg_catalog.text AS relation_name,
       relation.oid::pg_catalog.int8 AS relation_oid,
       relation.relowner::pg_catalog.int8 AS owner_oid,
       owner.rolname::pg_catalog.text AS owner_name,
       relation.relkind::pg_catalog.text AS relation_kind,
       relation.relpersistence::pg_catalog.text AS persistence,
       relation.relispartition AS is_partition,
       EXISTS (
           SELECT 1
             FROM pg_catalog.pg_inherits AS inheritance
            WHERE inheritance.inhrelid = relation.oid
               OR inheritance.inhparent = relation.oid
       ) AS has_inheritance_edge,
       relation.relrowsecurity AS row_security,
       relation.relforcerowsecurity AS force_row_security,
       pg_catalog.has_table_privilege(current_user, relation.oid, 'SELECT')
           AS table_select,
       pg_catalog.has_table_privilege(current_user, relation.oid, 'INSERT')
           AS table_insert,
       pg_catalog.has_table_privilege(current_user, relation.oid, 'UPDATE')
           AS table_update,
       pg_catalog.has_table_privilege(current_user, relation.oid, 'DELETE')
           AS table_delete,
       pg_catalog.has_table_privilege(current_user, relation.oid, 'TRUNCATE')
           AS table_truncate,
       pg_catalog.has_table_privilege(current_user, relation.oid, 'REFERENCES')
           AS table_references,
       pg_catalog.has_table_privilege(current_user, relation.oid, 'TRIGGER')
           AS table_trigger,
       (
           pg_catalog.has_table_privilege(
               current_user, relation.oid, 'SELECT WITH GRANT OPTION'
           )
           OR pg_catalog.has_table_privilege(
               current_user, relation.oid, 'INSERT WITH GRANT OPTION'
           )
           OR pg_catalog.has_table_privilege(
               current_user, relation.oid, 'UPDATE WITH GRANT OPTION'
           )
           OR pg_catalog.has_table_privilege(
               current_user, relation.oid, 'DELETE WITH GRANT OPTION'
           )
           OR pg_catalog.has_table_privilege(
               current_user, relation.oid, 'TRUNCATE WITH GRANT OPTION'
           )
           OR pg_catalog.has_table_privilege(
               current_user, relation.oid, 'REFERENCES WITH GRANT OPTION'
           )
           OR pg_catalog.has_table_privilege(
               current_user, relation.oid, 'TRIGGER WITH GRANT OPTION'
           )
       ) AS table_effective_grant_option
  FROM pg_catalog.pg_class AS relation
  JOIN pg_catalog.pg_namespace AS namespace
    ON namespace.oid = relation.relnamespace
  JOIN pg_catalog.pg_roles AS owner
    ON owner.oid = relation.relowner
 WHERE namespace.oid = $1::pg_catalog.oid
   AND namespace.nspname = 'public'
   AND relation.relname = '_sqlx_migrations'
 LIMIT 2
"#;

const COLUMNS_SQL: &str = r#"
SELECT attribute.attrelid::pg_catalog.int8 AS relation_oid,
       relation.relname::pg_catalog.text AS relation_name,
       attribute.attnum::pg_catalog.int4 AS attnum,
       attribute.attname::pg_catalog.text AS column_name,
       type_namespace.nspname::pg_catalog.text AS type_namespace,
       data_type.typname::pg_catalog.text AS type_name,
       data_type.typtype::pg_catalog.text AS type_kind,
       data_type.typbasetype::pg_catalog.int8 AS type_base_oid,
       attribute.attnotnull AS not_null,
       attribute.attisdropped AS is_dropped,
       attribute.attgenerated::pg_catalog.text AS generated,
       attribute.attidentity::pg_catalog.text AS identity,
       attribute.attndims::pg_catalog.int4 AS dimensions,
       attribute.atttypmod::pg_catalog.int4 AS type_modifier,
       pg_catalog.has_column_privilege(
           current_user, attribute.attrelid, attribute.attnum, 'SELECT'
       ) AS column_select,
       pg_catalog.has_column_privilege(
           current_user, attribute.attrelid, attribute.attnum, 'INSERT'
       ) AS column_insert,
       pg_catalog.has_column_privilege(
           current_user, attribute.attrelid, attribute.attnum, 'UPDATE'
       ) AS column_update,
       pg_catalog.has_column_privilege(
           current_user, attribute.attrelid, attribute.attnum, 'REFERENCES'
       ) AS column_references,
       pg_catalog.has_column_privilege(
           current_user, attribute.attrelid, attribute.attnum,
           'SELECT WITH GRANT OPTION'
       ) AS select_grant_option,
       pg_catalog.has_column_privilege(
           current_user, attribute.attrelid, attribute.attnum,
           'INSERT WITH GRANT OPTION'
       ) AS insert_grant_option,
       pg_catalog.has_column_privilege(
           current_user, attribute.attrelid, attribute.attnum,
           'UPDATE WITH GRANT OPTION'
       ) AS update_grant_option,
       pg_catalog.has_column_privilege(
           current_user, attribute.attrelid, attribute.attnum,
           'REFERENCES WITH GRANT OPTION'
       ) AS references_grant_option
  FROM pg_catalog.pg_attribute AS attribute
  JOIN pg_catalog.pg_class AS relation
    ON relation.oid = attribute.attrelid
  JOIN pg_catalog.pg_type AS data_type
    ON data_type.oid = attribute.atttypid
  JOIN pg_catalog.pg_namespace AS type_namespace
    ON type_namespace.oid = data_type.typnamespace
 WHERE attribute.attrelid = $1::pg_catalog.oid
   AND attribute.attnum > 0
 ORDER BY attribute.attnum
 LIMIT 7
"#;

const PRIMARY_KEY_SQL: &str = r#"
SELECT constraint_row.conrelid::pg_catalog.int8 AS relation_oid,
       constraint_row.connamespace::pg_catalog.int8 AS schema_oid,
       constraint_namespace.oid::pg_catalog.int8 AS constraint_namespace_oid,
       constraint_namespace.nspname::pg_catalog.text AS constraint_namespace,
       constraint_row.conname::pg_catalog.text AS constraint_name,
       constraint_row.contype::pg_catalog.text AS constraint_kind,
       pg_catalog.cardinality(constraint_row.conkey)::pg_catalog.int4 AS key_count,
       constraint_row.conkey[1]::pg_catalog.int4 AS first_attnum,
       constraint_row.convalidated AS is_validated,
       constraint_row.condeferrable AS is_deferrable,
       constraint_row.condeferred AS is_deferred,
       constraint_row.conindid::pg_catalog.int8 AS index_oid,
       index_row.indnatts::pg_catalog.int4 AS index_attribute_count,
       index_row.indnkeyatts::pg_catalog.int4 AS index_key_count,
       index_row.indisunique AS is_unique,
       index_row.indisprimary AS is_primary,
       index_row.indisvalid AS is_valid,
       index_row.indisready AS is_ready,
       index_row.indimmediate AS is_immediate,
       index_row.indexprs IS NULL AS has_no_expressions,
       index_row.indpred IS NULL AS has_no_predicate,
       index_namespace.oid::pg_catalog.int8 AS index_namespace_oid,
       index_namespace.nspname::pg_catalog.text AS index_namespace,
       index_relation.relkind::pg_catalog.text AS index_kind,
       index_relation.relpersistence::pg_catalog.text AS index_persistence,
       index_relation.relowner::pg_catalog.int8 AS index_owner_oid
  FROM pg_catalog.pg_constraint AS constraint_row
  JOIN pg_catalog.pg_namespace AS constraint_namespace
    ON constraint_namespace.oid = constraint_row.connamespace
  JOIN pg_catalog.pg_index AS index_row
    ON index_row.indexrelid = constraint_row.conindid
  JOIN pg_catalog.pg_class AS index_relation
    ON index_relation.oid = index_row.indexrelid
  JOIN pg_catalog.pg_namespace AS index_namespace
    ON index_namespace.oid = index_relation.relnamespace
 WHERE constraint_row.conrelid = $1::pg_catalog.oid
   AND constraint_row.connamespace = $2::pg_catalog.oid
   AND constraint_row.contype = 'p'
 LIMIT 2
"#;

const RAW_ACLS_SQL: &str = r#"
WITH acl_rows AS (
    SELECT 1::pg_catalog.int4 AS category,
           NULL::pg_catalog.int8 AS relation_oid,
           NULL::pg_catalog.int4 AS attnum,
           acl.grantee::pg_catalog.int8 AS grantee_oid,
           acl.grantor::pg_catalog.int8 AS grantor_oid,
           acl.privilege_type::pg_catalog.text AS privilege_type,
           acl.is_grantable
      FROM pg_catalog.pg_database AS database
      CROSS JOIN LATERAL pg_catalog.aclexplode(
          COALESCE(
              database.datacl,
              pg_catalog.acldefault('d', database.datdba)
          )
      ) AS acl
     WHERE database.oid = $1::pg_catalog.oid
       AND acl.grantee IN (0::pg_catalog.oid, $4::pg_catalog.oid)
    UNION ALL
    SELECT 2::pg_catalog.int4,
           NULL::pg_catalog.int8,
           NULL::pg_catalog.int4,
           acl.grantee::pg_catalog.int8,
           acl.grantor::pg_catalog.int8,
           acl.privilege_type::pg_catalog.text,
           acl.is_grantable
      FROM pg_catalog.pg_namespace AS namespace
      CROSS JOIN LATERAL pg_catalog.aclexplode(
          COALESCE(
              namespace.nspacl,
              pg_catalog.acldefault('n', namespace.nspowner)
          )
      ) AS acl
     WHERE namespace.oid = $2::pg_catalog.oid
       AND acl.grantee IN (0::pg_catalog.oid, $4::pg_catalog.oid)
    UNION ALL
    SELECT 3::pg_catalog.int4,
           relation.oid::pg_catalog.int8,
           NULL::pg_catalog.int4,
           acl.grantee::pg_catalog.int8,
           acl.grantor::pg_catalog.int8,
           acl.privilege_type::pg_catalog.text,
           acl.is_grantable
      FROM pg_catalog.pg_class AS relation
      CROSS JOIN LATERAL pg_catalog.aclexplode(
          COALESCE(
              relation.relacl,
              pg_catalog.acldefault('r', relation.relowner)
          )
      ) AS acl
     WHERE relation.oid = $3::pg_catalog.oid
       AND acl.grantee IN (0::pg_catalog.oid, $4::pg_catalog.oid)
    UNION ALL
    SELECT 4::pg_catalog.int4,
           attribute.attrelid::pg_catalog.int8,
           attribute.attnum::pg_catalog.int4,
           acl.grantee::pg_catalog.int8,
           acl.grantor::pg_catalog.int8,
           acl.privilege_type::pg_catalog.text,
           acl.is_grantable
      FROM pg_catalog.pg_attribute AS attribute
      CROSS JOIN LATERAL pg_catalog.aclexplode(attribute.attacl) AS acl
     WHERE attribute.attrelid = $3::pg_catalog.oid
       AND attribute.attnum > 0
       AND acl.grantee IN (0::pg_catalog.oid, $4::pg_catalog.oid)
)
SELECT acl_rows.category,
       acl_rows.relation_oid,
       acl_rows.attnum,
       acl_rows.grantee_oid,
       acl_rows.grantor_oid,
       acl_rows.privilege_type,
       acl_rows.is_grantable
  FROM acl_rows
 ORDER BY acl_rows.category,
          acl_rows.relation_oid NULLS FIRST,
          acl_rows.attnum NULLS FIRST,
          acl_rows.grantee_oid,
          acl_rows.privilege_type,
          acl_rows.grantor_oid,
          acl_rows.is_grantable
 LIMIT 65
"#;

const LEDGER_SQL: &str = r#"
SELECT ledger.tableoid::pg_catalog.int8 AS ledger_oid,
       ledger.version::pg_catalog.int8 AS version,
       ledger.success,
       pg_catalog.octet_length(ledger.checksum)::pg_catalog.int4 AS checksum_len,
       CASE
           WHEN pg_catalog.octet_length(ledger.checksum) = 48
               THEN ledger.checksum
           ELSE NULL::pg_catalog.bytea
       END AS checksum
  FROM public._sqlx_migrations AS ledger
 ORDER BY ledger.version
 LIMIT 65
"#;

struct ExpectedMigration {
    version: i64,
    path: &'static str,
    byte_len: usize,
    sha256_hex: &'static str,
    sha384_hex: &'static str,
    bytes: &'static [u8],
}

const BASELINE: [ExpectedMigration; EXPECTED_ROW_COUNT] = [
    ExpectedMigration {
        version: 1,
        path: "migrations/0001_identity.up.sql",
        byte_len: 3365,
        sha256_hex: "afbece7014bdf13a3197d87ece5427d4a80706048bde2263322f0e8403c327f6",
        sha384_hex: "db5d4f3f1aef7649f8818aa73407f809c4769e33822da0def81822a2a4c4ca4dd3302048d3a3fee058535e701400dcc2",
        bytes: include_bytes!("../../../../../migrations/0001_identity.up.sql"),
    },
    ExpectedMigration {
        version: 2,
        path: "migrations/0002_strategy.up.sql",
        byte_len: 2937,
        sha256_hex: "7fbbd6b485b58d05179887504edd253b6f433db571299e5536b2d81794cae3f6",
        sha384_hex: "4ee6fb9a0e5d8e589ee3aab104f17186e198e063ae853893474e84dc81463b3300ba6bde7603e210f7ce27f9a9f54198",
        bytes: include_bytes!("../../../../../migrations/0002_strategy.up.sql"),
    },
    ExpectedMigration {
        version: 3,
        path: "migrations/0003_market_data.up.sql",
        byte_len: 6537,
        sha256_hex: "cde9074a74439108348e021bc2d4a3f692ffaafc6e522dbb6428feed5c9fbc98",
        sha384_hex: "15d92eac923673c0d3f17160a4d19b525e9771168651577087d57628cd5da70ac55251c8e69b76ee8f0174773125d44a",
        bytes: include_bytes!("../../../../../migrations/0003_market_data.up.sql"),
    },
    ExpectedMigration {
        version: 4,
        path: "migrations/0004_research.up.sql",
        byte_len: 4733,
        sha256_hex: "a3af0844913a913b852f2215711043d0379bc2f8b737359a1252fcc6f1a779a4",
        sha384_hex: "f8b952afbe92d74a7e3713e9c6d573a4a7c14159ce95dad7ea003edc8efe1ecbffe0fd12a4070e609603400bebaaca19",
        bytes: include_bytes!("../../../../../migrations/0004_research.up.sql"),
    },
    ExpectedMigration {
        version: 5,
        path: "migrations/0005_jobs.up.sql",
        byte_len: 3095,
        sha256_hex: "e5ff2a85776f4fee73bdcfcc6e92df5195dedd47b5eaeff214bbb726a95a9320",
        sha384_hex: "af58468b3c7789ca514edc81605416931d54af9e4d7b270b060751923827427efc3191e13562a3b15f49d3b8339505f4",
        bytes: include_bytes!("../../../../../migrations/0005_jobs.up.sql"),
    },
    ExpectedMigration {
        version: 6,
        path: "migrations/0006_backtest.up.sql",
        byte_len: 3811,
        sha256_hex: "abcbbc58a6b2422e069373fe829473f6cac5919cf5815c2925afb154c3fe25ed",
        sha384_hex: "8c43b406bae29e4a6376dec6592a3da88fd66175c101a22e50e7380471890e89216af7340a6c2c172519fe4280a064b2",
        bytes: include_bytes!("../../../../../migrations/0006_backtest.up.sql"),
    },
    ExpectedMigration {
        version: 7,
        path: "migrations/0007_portfolio_trading.up.sql",
        byte_len: 7365,
        sha256_hex: "38dd9133061e6e0cc6886936f989db0370c135f67f0ae1dc4d9974bb0bf33b31",
        sha384_hex: "b17c0fa277677b2a4f1595299e5e62eff565c06e1f6d33895d32c5a93e48c1c7d66e345fc0c1698b1a3ea40b711e877f",
        bytes: include_bytes!("../../../../../migrations/0007_portfolio_trading.up.sql"),
    },
    ExpectedMigration {
        version: 8,
        path: "migrations/0008_operations.up.sql",
        byte_len: 2213,
        sha256_hex: "016e58c6d714c744db2e44488f870bd1123de0490add3390fdc0dcdcff7827fe",
        sha384_hex: "e3b789829dbdd35ddc90f06499a127c3fe8a4bf5994617ebfc605d5703814ac0972898ba61ed62cc1fac0d2f4d4a79cc",
        bytes: include_bytes!("../../../../../migrations/0008_operations.up.sql"),
    },
    ExpectedMigration {
        version: 9,
        path: "migrations/0009_grants.up.sql",
        byte_len: 2624,
        sha256_hex: "b4a706ddd668b208a19e909f3bc189e09a34445de795faad1a1a92d6099aceb1",
        sha384_hex: "113f940e2f296a830085861b58d0e8c2dcbe674b5d6ff9eb49c17ac40b4f01f6e3c2219094a200986461864ac06c6fb3",
        bytes: include_bytes!("../../../../../migrations/0009_grants.up.sql"),
    },
    ExpectedMigration {
        version: 10,
        path: "migrations/0010_rls.up.sql",
        byte_len: 7812,
        sha256_hex: "b8334d3d6a166a3f20cc9978d9e819a966f6e0e93269d9b22539790a043434ba",
        sha384_hex: "1bc0a5f632ae5b878eadf2edbfd1f733dc9fb22b32a593894d9bc51d98be1d2a6c17f62b3b4ff19c0f86fff324ed4d41",
        bytes: include_bytes!("../../../../../migrations/0010_rls.up.sql"),
    },
    ExpectedMigration {
        version: 11,
        path: "migrations/0011_notifications.up.sql",
        byte_len: 4103,
        sha256_hex: "f52df5b6b5083ca828701082d4b87be4ce1aa90a25c05f6d90d4ff233f7e0291",
        sha384_hex: "6adaf3148fc1aa270174f72dd7114131eba7d4700e7a13ab2bbeb2507aa6ca6f5c21e991a3950c9adfff6762efbea125",
        bytes: include_bytes!("../../../../../migrations/0011_notifications.up.sql"),
    },
    ExpectedMigration {
        version: 12,
        path: "migrations/0012_robustness.up.sql",
        byte_len: 3950,
        sha256_hex: "4b4e98d80e0a9e91e020c060cd319e5b91b566b053281ecf907d7c2bac06d575",
        sha384_hex: "9ed6fda7b01851a422100b9069b7c036e2753b29a754093bfd4f38cbf4d50430092ef6fab2a269093d71f260e819c24a",
        bytes: include_bytes!("../../../../../migrations/0012_robustness.up.sql"),
    },
    ExpectedMigration {
        version: 13,
        path: "migrations/0013_paper_bindings.up.sql",
        byte_len: 4012,
        sha256_hex: "b44772c013853047d3dcbc00dbd6dc3ed8b7c7b2417a8290f6ba8de2f3f9cabd",
        sha384_hex: "0b16f77fa60989cec865f769ebe99d06571f5710a5a2f8b7eeb4e9c07bb0dc12a4a7f2ed2032a67c9ed06695197cb56d",
        bytes: include_bytes!("../../../../../migrations/0013_paper_bindings.up.sql"),
    },
    ExpectedMigration {
        version: 14,
        path: "migrations/0014_pending_targets.up.sql",
        byte_len: 4395,
        sha256_hex: "26abe6dfbb4dba8f7bf9f07ca4286c84fd9ee04c9feb060fb3faabe8e1837bc1",
        sha384_hex: "dce3eb06d44df1d5f12649417c467c2e07538e255a7088065b3072539f749043d005743fed3fe360b91ecf2fd1390e41",
        bytes: include_bytes!("../../../../../migrations/0014_pending_targets.up.sql"),
    },
    ExpectedMigration {
        version: 15,
        path: "migrations/0015_pending_target_provenance.up.sql",
        byte_len: 1057,
        sha256_hex: "d0c5839a0a3f43eee67fe6c381014e6554e1006665ba8272ac5343952d974d81",
        sha384_hex: "a1be98f0d631d1ff0524f675708e637b2c3001738cfd6289df496869d69bbdf086b6e7b02548475ff566298814c98efb",
        bytes: include_bytes!("../../../../../migrations/0015_pending_target_provenance.up.sql"),
    },
    ExpectedMigration {
        version: 16,
        path: "migrations/0016_broker_connections.up.sql",
        byte_len: 6577,
        sha256_hex: "2427a5c68c550f89ca0778ec032e0face220a980c4425b28f2a90baa1b6d70da",
        sha384_hex: "a66559b953854f750dda1cad39fabcf964a15d4bb0d6593e1137e09468314f9fc7dd4d7ccd7f85e9b2459a02f193b408",
        bytes: include_bytes!("../../../../../migrations/0016_broker_connections.up.sql"),
    },
    ExpectedMigration {
        version: 17,
        path: "migrations/0017_session_auth_context.up.sql",
        byte_len: 1163,
        sha256_hex: "4b65894dcd6f8f26d4cdb8ce9e34e2268afcf583971c260fe2230cbe017ff92e",
        sha384_hex: "cef3843a1352f6fd493c753df2fdc335c1631dada15ede84e29e794ff25df22633240a50a04676299ffb71ded8b59e15",
        bytes: include_bytes!("../../../../../migrations/0017_session_auth_context.up.sql"),
    },
    ExpectedMigration {
        version: 18,
        path: "migrations/0018_risk_gateway.up.sql",
        byte_len: 7818,
        sha256_hex: "9fc0801ccdbe68a354ea923050e5871a65cf896a09c911b527696787457b81db",
        sha384_hex: "823888b8df387db6c791990a052e3a65a2fae7c83f9343475c497db0516347d4c11e243239b01e63562ed0dda522427d",
        bytes: include_bytes!("../../../../../migrations/0018_risk_gateway.up.sql"),
    },
    ExpectedMigration {
        version: 19,
        path: "migrations/0019_order_intents.up.sql",
        byte_len: 8149,
        sha256_hex: "6ccdadd7284983fc12b10db6c6e479ade20d940a30c67311c680c3ac47afb99b",
        sha384_hex: "c41a5abdb15169a5a8d374415c6900dc3b8e7b68a270e505a783316fa8d784824687bcafc51eaf9460ace9d439c5957c",
        bytes: include_bytes!("../../../../../migrations/0019_order_intents.up.sql"),
    },
    ExpectedMigration {
        version: 20,
        path: "migrations/0020_order_intent_client_key.up.sql",
        byte_len: 1889,
        sha256_hex: "c0781cfc0d5a5a5d35c60dc8e17dc2c0e3bd8ac825cb86a032c45a98e673c940",
        sha384_hex: "93bacfc3a23eb661689e14063058216812ea9a24510bd6f56064e075b5ab7272035ffef27e293f3082a4ec9d8c9f318e",
        bytes: include_bytes!("../../../../../migrations/0020_order_intent_client_key.up.sql"),
    },
    ExpectedMigration {
        version: 21,
        path: "migrations/0021_worker_reads_strategy_configs.up.sql",
        byte_len: 1647,
        sha256_hex: "2982a9fd36ff6d378b084c019b7c19c513a8620706f92b3e6c13066288a558a0",
        sha384_hex: "6bd3a79d11b6a8301c2960612bd972aac4189696f1c1a38cbe3be66b95bac6b343557d14888fbed2ccee2e8bc819017d",
        bytes: include_bytes!(
            "../../../../../migrations/0021_worker_reads_strategy_configs.up.sql"
        ),
    },
    ExpectedMigration {
        version: 22,
        path: "migrations/0022_research_publication.up.sql",
        byte_len: 4517,
        sha256_hex: "d6b455538f052b6082a0a5185f3ab890ff90949878f4f39e37b55ba00bc9953b",
        sha384_hex: "638b43144c284aa993061afd7af5d4980edb203c51076cbcdf5f944fc5d67649d339ec113a3fb5264527453a21030c52",
        bytes: include_bytes!("../../../../../migrations/0022_research_publication.up.sql"),
    },
    ExpectedMigration {
        version: 23,
        path: "migrations/0023_research_publication_validate_constraints.up.sql",
        byte_len: 504,
        sha256_hex: "757f0ac4b6aeb00c783af21f1a7e86d1772d2e3246d21f28677b191d155962f2",
        sha384_hex: "c5af71264f6a5bf06c2861f1171306f841f1250b5c854efe1cda539ed648fc882f699d31e30ab6393d0cf3e84a913ec3",
        bytes: include_bytes!(
            "../../../../../migrations/0023_research_publication_validate_constraints.up.sql"
        ),
    },
    ExpectedMigration {
        version: 24,
        path: "migrations/0024_research_publication_source_index.up.sql",
        byte_len: 571,
        sha256_hex: "aefbfd2c2833c977e02b130cc4b823a5b43186f2ad11b42ef7a8c8705131b22e",
        sha384_hex: "80bb779eeacdc73690366d09507fc801f463d5119e72ababceb2a830f4f3b85d0e40633b54b24d7028e1f751074ff5f7",
        bytes: include_bytes!(
            "../../../../../migrations/0024_research_publication_source_index.up.sql"
        ),
    },
    ExpectedMigration {
        version: 25,
        path: "migrations/0025_research_calendar_version_lookup.up.sql",
        byte_len: 558,
        sha256_hex: "2f3f2303efc27ccec94ba4de54afda65c857639925092b95b2b2f689adfb2cde",
        sha384_hex: "67c3af2e89b6c8cc0a1eb71a21530ad97592072beaf34565e1a615f19d1f396331dd7feb394aed664b569357026ecdba",
        bytes: include_bytes!(
            "../../../../../migrations/0025_research_calendar_version_lookup.up.sql"
        ),
    },
    ExpectedMigration {
        version: 26,
        path: "migrations/0026_recommendation_pipeline.up.sql",
        byte_len: 15358,
        sha256_hex: "b9b71439f61db95e640bf00d3a0f54ef58e8e2a54d988dadb9cd20ec5b063813",
        sha384_hex: "5c565e81e6d0fcd76c7fae97bde4a38c071e10fefaf03276dcbccec633856b0ea915f9a3683b20cd3eaa3c004eee76d8",
        bytes: include_bytes!("../../../../../migrations/0026_recommendation_pipeline.up.sql"),
    },
    ExpectedMigration {
        version: 27,
        path: "migrations/0027_recommendation_run_job_index.up.sql",
        byte_len: 438,
        sha256_hex: "90cf0bba6639265c21decd57fdb729fae2022b5269c7f4bf859673dd346702f2",
        sha384_hex: "736d5384c111dfe593a3623a2a927c2ae3e2be7a1b40e2abca09e7a0b48ccaf0283c61add58de180a1cda9092ab2a6d7",
        bytes: include_bytes!("../../../../../migrations/0027_recommendation_run_job_index.up.sql"),
    },
    ExpectedMigration {
        version: 28,
        path: "migrations/0028_recommendation_scheduled_identity_index.up.sql",
        byte_len: 330,
        sha256_hex: "1fb169b2e3f67db20dec3f434f170d37b305f367f7990de5bb73ddb755468d54",
        sha384_hex: "e44740de23e0e759f2dc6bb5cfb15eed699e822632870bde32be688924a6b9d081e7683952b1f81fe450eb7bb0b98d07",
        bytes: include_bytes!(
            "../../../../../migrations/0028_recommendation_scheduled_identity_index.up.sql"
        ),
    },
    ExpectedMigration {
        version: 29,
        path: "migrations/0029_recommendation_item_unique_index.up.sql",
        byte_len: 235,
        sha256_hex: "43f011e7af85a3163d6808253ef640a8dfdac592d93f409dd908963f8028b013",
        sha384_hex: "815b758c2a5bb6ae21af8dbd2ee29560a9fc1add64e289a1a5db5c813265ebae87546003526972adcff611e20c5c241c",
        bytes: include_bytes!(
            "../../../../../migrations/0029_recommendation_item_unique_index.up.sql"
        ),
    },
    ExpectedMigration {
        version: 30,
        path: "migrations/0030_recommendation_item_unique_constraint.up.sql",
        byte_len: 304,
        sha256_hex: "b703d983290c6a9836f49ebdbb28df647766ee252a284a403304b56079386702",
        sha384_hex: "c047fe44691049105bf8a44abed0cd675307c4851919e3fce65c31a38a5d9fd186802afa9d37a24e02cfad803267a3ab",
        bytes: include_bytes!(
            "../../../../../migrations/0030_recommendation_item_unique_constraint.up.sql"
        ),
    },
    ExpectedMigration {
        version: 31,
        path: "migrations/0031_target_portfolio_run_index.up.sql",
        byte_len: 245,
        sha256_hex: "6d4c87972bbdac1272b38ba8988b39394fc6728c92829b4cb80a57c8ee9706b2",
        sha384_hex: "85ec6b8b0a40b2507bdfa8d558db560db6a4eb41860abc6ab44ce49a54f387383bc163d0199eb48cf0b18a4dc18cd552",
        bytes: include_bytes!("../../../../../migrations/0031_target_portfolio_run_index.up.sql"),
    },
    ExpectedMigration {
        version: 32,
        path: "migrations/0032_jobs_typed_claim_index.up.sql",
        byte_len: 314,
        sha256_hex: "c2945c268f0af378c2a5488b09aab6458825435d5afc309c62b1f709c440be90",
        sha384_hex: "5bca3d939f951e7d5420d9562d7dfac34e77f4dba81856f159d32310b308ea207ad44a701ee7fc5cb725448543c0f718",
        bytes: include_bytes!("../../../../../migrations/0032_jobs_typed_claim_index.up.sql"),
    },
    ExpectedMigration {
        version: 33,
        path: "migrations/0033_recommendation_rollback_guard.up.sql",
        byte_len: 2202,
        sha256_hex: "ff0057e0ec0990deb92cebe14da98be0d8c3d15f5f2cdcbd8169cef1353f8c25",
        sha384_hex: "7f97d2ee5e17fda44b51b32f9315a2584451a5f86391164f72253ce91b090b19857ea8451c7c2e5d3a27ecd69ed550a8",
        bytes: include_bytes!(
            "../../../../../migrations/0033_recommendation_rollback_guard.up.sql"
        ),
    },
    ExpectedMigration {
        version: 34,
        path: "migrations/0034_recommendation_publication_locks.up.sql",
        byte_len: 2640,
        sha256_hex: "dbaebf85cf703e1802206b231cd0906ede7fbf37007c340513ac6683167b1aca",
        sha384_hex: "a68ce511a57bd3d7f981cbcc6e76b82311d6e23f0a234b15fa0c3c68560df4803ed451396bb640c2cbc49ba4f94aaf33",
        bytes: include_bytes!(
            "../../../../../migrations/0034_recommendation_publication_locks.up.sql"
        ),
    },
    ExpectedMigration {
        version: 35,
        path: "migrations/0035_recommendation_entitlement_lock.up.sql",
        byte_len: 6384,
        sha256_hex: "f20a61719f4902c1de19d10df66d93d106a83a8c5bf5502c0fae5dcf1aa75d1e",
        sha384_hex: "595db06429d056363830f41c0edbff4bf333102d0d742c9f251056d7a747e62f3de4ccb7a9ce753517790c91bbe87157",
        bytes: include_bytes!(
            "../../../../../migrations/0035_recommendation_entitlement_lock.up.sql"
        ),
    },
    ExpectedMigration {
        version: 36,
        path: "migrations/0036_recommendation_submission_dataset_lock.up.sql",
        byte_len: 1431,
        sha256_hex: "5229be096d6987919c2a179180f54cac8506226ccf11fe56401ed46a7ab1d8c0",
        sha384_hex: "e12e1ad66c35219579e41ac0ddde6640f8464004de238f735be4d61a07059b711a0c8ccb4cbb57af50bf85c562c3b568",
        bytes: include_bytes!(
            "../../../../../migrations/0036_recommendation_submission_dataset_lock.up.sql"
        ),
    },
    ExpectedMigration {
        version: 37,
        path: "migrations/0037_paper_recommendation_execution.up.sql",
        byte_len: 15591,
        sha256_hex: "971a84539fdf8d17f8aba80154451444220906c8a2c4a9e753d33a939334dab9",
        sha384_hex: "18edf5b4a056295c78b00e3e3e53dbdc49b9b825248828eef24d1a1c53f3cf05a8007d0ff24185e295b368d2e352dfdc",
        bytes: include_bytes!(
            "../../../../../migrations/0037_paper_recommendation_execution.up.sql"
        ),
    },
    ExpectedMigration {
        version: 38,
        path: "migrations/0038_paper_rebalance_previews.up.sql",
        byte_len: 56984,
        sha256_hex: "2586403d3634bddf5d1105a529d1eef1c3a16e1cf451c9b8dc6cc0c7600ae503",
        sha384_hex: "a99a4b5fee1a8caa771be5402cbed5e2027412ee34ece95ea17db78282c0b530adb7e00b2f0ff49f91cb71dfbeb6e0c3",
        bytes: include_bytes!("../../../../../migrations/0038_paper_rebalance_previews.up.sql"),
    },
    ExpectedMigration {
        version: 39,
        path: "migrations/0039_auth_audit_outbox.up.sql",
        byte_len: 11338,
        sha256_hex: "bb6a53b2b4a755f5e80d584350e46a25024878c9bdf3e58c567117a2e355b1a2",
        sha384_hex: "c2dd1ec6cd96e9e104d5f9e4b97f029b858f85cc52f769e534e185917585aebd94a8ae0106b4bea657e7ac2d02741f34",
        bytes: include_bytes!("../../../../../migrations/0039_auth_audit_outbox.up.sql"),
    },
    ExpectedMigration {
        version: 40,
        path: "migrations/0040_identity_provisioning.up.sql",
        byte_len: 30067,
        sha256_hex: "89fc95ea7467e17582d28d3fed6199a3766c69505494bd16b02e5eddb3a64770",
        sha384_hex: "56fa17f8a35057260df41e5a061d3fd318ac2c947a4cceb6f4081f2339f9707588d73ed70a863e646b89207f3f378a51",
        bytes: include_bytes!("../../../../../migrations/0040_identity_provisioning.up.sql"),
    },
    ExpectedMigration {
        version: 41,
        path: "migrations/0041_paper_settlement_outbox.up.sql",
        byte_len: 44747,
        sha256_hex: "d545b13e555eed04c9228824804d81336b93c2438442bed4873b29ba6a055c82",
        sha384_hex: "0d44ffe54e1eccaf85bce6d7ecff4de05caaa19d8d9bd976a35e851998b9563e04dd34146ca4753b6b30494b3abd8397",
        bytes: include_bytes!("../../../../../migrations/0041_paper_settlement_outbox.up.sql"),
    },
    ExpectedMigration {
        version: 42,
        path: "migrations/0042_candidate_source_contracts.up.sql",
        byte_len: 96202,
        sha256_hex: "dfc0b622212f7379113fdda216c198a3229b407cdd6185ed2d7074c4021bfcc3",
        sha384_hex: "0c27ed68f4968796d97ac202f13f4d099a8fb9329f93570839b0bf76e2ef2e46850d9758727c3acc1c745309d3c98521",
        bytes: include_bytes!("../../../../../migrations/0042_candidate_source_contracts.up.sql"),
    },
    ExpectedMigration {
        version: 43,
        path: "migrations/0043_candidate_analysis_surfaces.up.sql",
        byte_len: 24439,
        sha256_hex: "79a0f3b218ef2f26cb6a5be18762ad9701f80d1c05e2ce904f16b6206e8e250c",
        sha384_hex: "a07892f0af22c51fdd1bddf4684262f68c249a3018007daa499154933b8958155ef76129044d15831a629813e9811f83",
        bytes: include_bytes!("../../../../../migrations/0043_candidate_analysis_surfaces.up.sql"),
    },
    ExpectedMigration {
        version: 44,
        path: "migrations/0044_candidate_pipeline.up.sql",
        byte_len: 62219,
        sha256_hex: "a44c910f7f7b346ab1f154d7a050ebe23165615e85de2f1e5a15cd8c9be36324",
        sha384_hex: "0e03215fce5180d1fd59d6e3c443463cbf2dcb55d7742b4f63d7b8a7315d50b3c97ccf2661a362722eaec47a37b782a0",
        bytes: include_bytes!("../../../../../migrations/0044_candidate_pipeline.up.sql"),
    },
    ExpectedMigration {
        version: 45,
        path: "migrations/0045_candidate_multi_universe.up.sql",
        byte_len: 100779,
        sha256_hex: "c26694b5c7563b0b19e34d60a13b7f522c75e13138b70d4c60314b368d8ce121",
        sha384_hex: "be432b2d2bef0fb9ef774c0a6c780ce771c5bce828bb3d2c19daab3bfccaf14ad394f5fce023c23f4f75d5acb1ba4234",
        bytes: include_bytes!("../../../../../migrations/0045_candidate_multi_universe.up.sql"),
    },
    ExpectedMigration {
        version: 46,
        path: "migrations/0046_candidate_price_rights_revalidation.up.sql",
        byte_len: 35222,
        sha256_hex: "f3da384e85d6cd1166de0828b4f1d22fedf74397c6f63109d28726e5697ee13b",
        sha384_hex: "a7a6a2456b331a743b11166c5c2cc24fa25c38e202499c6a38bd20cbaa3fb185e669f297089028671007145041f94153",
        bytes: include_bytes!(
            "../../../../../migrations/0046_candidate_price_rights_revalidation.up.sql"
        ),
    },
    ExpectedMigration {
        version: 47,
        path: "migrations/0047_candidate_worker_price_entitlement_attestation.up.sql",
        byte_len: 296,
        sha256_hex: "0264434177af72463b3b9c4b5df181d0857592aa92f580488a297a6fd1e2bc26",
        sha384_hex: "1a5a69528884133086cc377189183ef30a245fff0f6bc44c063327044e97dd96aa9f67b6008b835d680c525a7aea17e9",
        bytes: include_bytes!(
            "../../../../../migrations/0047_candidate_worker_price_entitlement_attestation.up.sql"
        ),
    },
    ExpectedMigration {
        version: 48,
        path: "migrations/0048_candidate_price_generation_reuse_binding.up.sql",
        byte_len: 6434,
        sha256_hex: "7170798a7b468eeefa265ae7f69618f19073acb24dd080de9406c16631fba9ea",
        sha384_hex: "d464f498caa06e5934d8e158abbe795eda8af3d65693e5559066181e2275c1470b4ea3d004d6da750d0afa9a943ab2c8",
        bytes: include_bytes!(
            "../../../../../migrations/0048_candidate_price_generation_reuse_binding.up.sql"
        ),
    },
    ExpectedMigration {
        version: 49,
        path: "migrations/0049_owner_beta_price_recommendations.up.sql",
        byte_len: 16755,
        sha256_hex: "cdbcd6f707096911be2bdf4812c6657527fe5e92420146daa675b39b5e0e69a3",
        sha384_hex: "f156306fbec6c6643ba829141937512383a14d7c39e5d8a3cf72f0da78e3aa7874e302bb68b1a7b7962ef969f5508b71",
        bytes: include_bytes!(
            "../../../../../migrations/0049_owner_beta_price_recommendations.up.sql"
        ),
    },
    ExpectedMigration {
        version: 50,
        path: "migrations/0050_owner_beta_strategy_snapshots.up.sql",
        byte_len: 10530,
        sha256_hex: "ad86f4b093dbd43bf2a886279eb146431653457ceaca208984c4a6600e7e9988",
        sha384_hex: "1c996486eede47db6c1233cc5ecf27ad7d323d392cf993482da16491765cb05d60995c84e05256cb0d181cc919015ed2",
        bytes: include_bytes!(
            "../../../../../migrations/0050_owner_beta_strategy_snapshots.up.sql"
        ),
    },
    ExpectedMigration {
        version: 51,
        path: "migrations/0051_owner_beta_target_publication.up.sql",
        byte_len: 2240,
        sha256_hex: "ab3245d7fa44f7e02a265a624fbd9b8acc7873ebc242d98dd6bb5ce0564a678a",
        sha384_hex: "7da9ded1ad62173b14842a3d269d228a98c1caebae0073f40ae1393e82a0eb4bf34dfdb4a50675380b167b1cf935ea12",
        bytes: include_bytes!(
            "../../../../../migrations/0051_owner_beta_target_publication.up.sql"
        ),
    },
    ExpectedMigration {
        version: 52,
        path: "migrations/0052_owner_beta_strategy_config_lock.up.sql",
        byte_len: 2103,
        sha256_hex: "c366a92d2193cbfd1816c7f08df204709750df2473e9f19b06ff4b188560a5d8",
        sha384_hex: "3691916cb642473a1a8815bcabbef42d5199a3fbf1d1371b005b9b059375df5132f79616abd13e7419d6c63fbd5257d7",
        bytes: include_bytes!(
            "../../../../../migrations/0052_owner_beta_strategy_config_lock.up.sql"
        ),
    },
    ExpectedMigration {
        version: 53,
        path: "migrations/0053_owner_managed_equity_universe_v2.up.sql",
        byte_len: 51472,
        sha256_hex: "68cbeb3685afb583c0237a9de61f2a4df1b24ab7335d2e7cb79e68607bb5bef3",
        sha384_hex: "8dd36e40692a303f22585ca0a8a31bede7abf9a6e4a08722c02e423d691c84e764c5c219d4e66ef1943d4c9b1ac28915",
        bytes: include_bytes!(
            "../../../../../migrations/0053_owner_managed_equity_universe_v2.up.sql"
        ),
    },
    ExpectedMigration {
        version: 54,
        path: "migrations/0054_owner_intraday_quotes.up.sql",
        byte_len: 16666,
        sha256_hex: "95360f7ef536777f3ce5ff49afdb6c40966caae489547904e7c844a03c66033d",
        sha384_hex: "0470be28978b1607c36b075b10dc466eb715805d00cdba667e055ea97d0488c4d2d501584590b6a2a5ce243d09c309b7",
        bytes: include_bytes!("../../../../../migrations/0054_owner_intraday_quotes.up.sql"),
    },
    ExpectedMigration {
        version: 55,
        path: "migrations/0055_owner_market_stream.up.sql",
        byte_len: 72899,
        sha256_hex: "3a2aed8bbe0cf51f668b51ad83a557d460ab18e8c69c3d55a2936b74cfb555a9",
        sha384_hex: "e6835fe6a21fbacad01214d2040761481dd76446aef497a65ff5125b9fafb75188438358becb887fc5478020f6a6cf01",
        bytes: include_bytes!("../../../../../migrations/0055_owner_market_stream.up.sql"),
    },
    ExpectedMigration {
        version: 56,
        path: "migrations/0056_owner_market_stream_runtime.up.sql",
        byte_len: 10400,
        sha256_hex: "0b5935d092dfb02634009d9e9a24fa51907acce2da28b34a448281e467e0bb1a",
        sha384_hex: "71007010c82365ac65208ca797c17393d86562f8f685274ccb3f88fc62661ebfc553ae669e11169af6a76e23a243edef",
        bytes: include_bytes!("../../../../../migrations/0056_owner_market_stream_runtime.up.sql"),
    },
    ExpectedMigration {
        version: 57,
        path: "migrations/0057_owner_market_stream_api_binding.up.sql",
        byte_len: 2429,
        sha256_hex: "d10ff923c6608b9ee36137bf6c7a793a20b2a3927b4371700c47c1f564c503e0",
        sha384_hex: "ffa654ef1c5b17b4f8424260217c0686708e54e5ba1366f87a082f4917eff67da4299b239ac2f89502cdde11a8384073",
        bytes: include_bytes!(
            "../../../../../migrations/0057_owner_market_stream_api_binding.up.sql"
        ),
    },
    ExpectedMigration {
        version: 58,
        path: "migrations/0058_service_status_reads.up.sql",
        byte_len: 29402,
        sha256_hex: "1cd935e97e441423f33670fad7cd0db81d8c627518dc6acc4906e77c1722f4f9",
        sha384_hex: "575beb92ec5726a0efb82a883c091970eb90558f95ec841c86e3baa33886803196d9d57889e924dda9f4b6ee0e027247",
        bytes: include_bytes!("../../../../../migrations/0058_service_status_reads.up.sql"),
    },
    ExpectedMigration {
        version: 59,
        path: "migrations/0059_research_execution_status.up.sql",
        byte_len: 3431,
        sha256_hex: "f2a7df36c2983823eb8834af9d10a7c91b75f44602134943e313de804d3fbb0c",
        sha384_hex: "e702876167a5200b18de88af54a517f9f9caefe2a363164aaabfb298ff6d71a5dd0b3eb6f5ee84fe63e7a7a651476f63",
        bytes: include_bytes!("../../../../../migrations/0059_research_execution_status.up.sql"),
    },
    ExpectedMigration {
        version: 60,
        path: "migrations/0060_owner_beta_daily_inputs.up.sql",
        byte_len: 139458,
        sha256_hex: "47b56ac14394aeb94f4ce90979de5628ab25414adc7fe70f1188b132071ce305",
        sha384_hex: "b7f33b4da392262a76e4fbf14574f3bf51c92e9faa3f74a65a3b8ef5aaf8676d407e92df4eb93a06d382f86c1b3e5646",
        bytes: include_bytes!("../../../../../migrations/0060_owner_beta_daily_inputs.up.sql"),
    },
    ExpectedMigration {
        version: 61,
        path: "migrations/0061_owner_beta_daily_supersession.up.sql",
        byte_len: 7713,
        sha256_hex: "f7e62e6a21a41b391290d4af8fd8c25c136bc6f03262813fbbe13b5b95312521",
        sha384_hex: "19ae8399868018001c37838dd489ce78a096292a371fde87e19ab43a0d8fa852883b91eb675cfb09643272aa263d9c06",
        bytes: include_bytes!(
            "../../../../../migrations/0061_owner_beta_daily_supersession.up.sql"
        ),
    },
    ExpectedMigration {
        version: 62,
        path: "migrations/0062_owner_beta_daily_admitter_readback.up.sql",
        byte_len: 8912,
        sha256_hex: "b93c1bb238f78a57a826a8d03d1cc070253fe98cae2169eaebd0c57e285f3e4b",
        sha384_hex: "e68573d08a108ac259479e599c55a6f5e5295cbfc96d7a9c6a61d0deb4b4ea586c00c492087d37fdf840cc8da7849b9a",
        bytes: include_bytes!(
            "../../../../../migrations/0062_owner_beta_daily_admitter_readback.up.sql"
        ),
    },
    ExpectedMigration {
        version: 63,
        path: "migrations/0063_owner_beta_price_only_simulations.up.sql",
        byte_len: 67805,
        sha256_hex: "8cc91cf8a0bf3b85e13288f4941408c03ba4d1156acaa3c5559446bb4200f1a3",
        sha384_hex: "caad894582e8348a49a6e964d3b616864829fd8299ae17f9e2c0dc746278ec9d02fd8f286bd6c349984772be15fae043",
        bytes: include_bytes!(
            "../../../../../migrations/0063_owner_beta_price_only_simulations.up.sql"
        ),
    },
    ExpectedMigration {
        version: 64,
        path: "migrations/0064_owner_beta_simulation_v2.up.sql",
        byte_len: 20536,
        sha256_hex: "a06612ca959e94bd772c1aff06a65992450e34cff5b8b9ccaf4f004245853a10",
        sha384_hex: "30d3ba43720d0dbf82abf8cf5378a53e0b807f31235ba035bb79200fc4df87d07dc3acf8dbb17b409bfb9bcc8df0c645",
        bytes: include_bytes!("../../../../../migrations/0064_owner_beta_simulation_v2.up.sql"),
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(super) enum SimulationMigrationLedgerError {
    #[error("simulation migration ledger observation has an invalid bound")]
    InvalidBound,
    #[error("simulation migration ledger source baseline is invalid")]
    BaselineInvalid,
    #[error("simulation migration ledger observation was canceled")]
    Canceled,
    #[error("simulation migration ledger connection acquisition failed")]
    AcquisitionFailed,
    #[error("simulation migration ledger transaction could not be started")]
    BeginFailed,
    #[error("simulation migration ledger transaction returned an unexpected begin result")]
    BeginResultInvalid,
    #[error("simulation migration ledger transaction setup failed")]
    SetupFailed,
    #[error("simulation migration catalog observation is unavailable")]
    CatalogUnavailable,
    #[error("simulation migration catalog observation is invalid")]
    CatalogInvalid,
    #[error("simulation migration catalog row decoding failed")]
    DecodeFailed,
    #[error("simulation migration catalog bounds were exceeded")]
    LimitExceeded,
    #[error("simulation migration worker access policy was rejected")]
    AccessRejected,
    #[error("simulation migration ledger is unavailable")]
    LedgerUnavailable,
    #[error("simulation migration ledger observation is invalid")]
    LedgerInvalid,
    #[error("simulation migration ledger contains an unsuccessful migration")]
    LedgerDirty,
    #[error("simulation migration ledger is missing an expected predecessor")]
    MissingPredecessor,
    #[error("simulation migration ledger checksum does not match the source baseline")]
    ChecksumMismatch,
    #[error("simulation migration ledger includes a version outside the adopted baseline")]
    UnknownBaselineVersion,
    #[error("simulation migration transaction completion is unconfirmed")]
    CompletionUnconfirmed,
    #[error("simulation migration completion and cleanup are unconfirmed")]
    CompletionAndCleanupUnconfirmed,
    #[error("simulation migration connection cleanup is unconfirmed")]
    CleanupUnconfirmed,
}

#[allow(dead_code)] // The validated ledger pins are retained as opaque evidence.
pub(super) struct SimulationMigrationLedgerObservation {
    config_sha256: String,
    baseline_policy_id: String,
    baseline_fingerprint: String,
    database_oid: u32,
    worker_oid: u32,
    public_schema_oid: u32,
    migration_owner_oid: u32,
    ledger_oid: u32,
    ledger_row_count: u64,
    ordered_ledger_digest: String,
}

#[allow(dead_code)]
impl SimulationMigrationLedgerObservation {
    fn completed(
        config_sha256: &str,
        baseline_fingerprint: &str,
        identity: LedgerIdentity,
        ledger: LedgerEvidence,
    ) -> Self {
        Self {
            config_sha256: config_sha256.to_owned(),
            baseline_policy_id: POLICY_ID.to_owned(),
            baseline_fingerprint: baseline_fingerprint.to_owned(),
            database_oid: identity.database_oid,
            worker_oid: identity.worker_oid,
            public_schema_oid: identity.public_schema_oid,
            migration_owner_oid: identity.migration_owner_oid,
            ledger_oid: identity.ledger_oid,
            ledger_row_count: ledger.row_count,
            ordered_ledger_digest: ledger.digest_text,
        }
    }

    pub(super) fn config_sha256(&self) -> &str {
        &self.config_sha256
    }

    pub(super) fn baseline_policy_id(&self) -> &str {
        &self.baseline_policy_id
    }

    pub(super) fn baseline_fingerprint(&self) -> &str {
        &self.baseline_fingerprint
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

    pub(super) fn ledger_oid(&self) -> u32 {
        self.ledger_oid
    }

    pub(super) fn ledger_row_count(&self) -> u64 {
        self.ledger_row_count
    }

    pub(super) fn ordered_ledger_digest(&self) -> &str {
        &self.ordered_ledger_digest
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RawCategory {
    Context,
    Membership,
    Relation,
    Column,
    PrimaryKey,
    Acl,
    Ledger,
}

#[derive(Clone, PartialEq, Eq)]
enum RawCell {
    Bool(bool),
    I32(i32),
    I64(i64),
    Text(String),
    Bytes(Vec<u8>),
    Null,
    WrongType,
}

#[derive(Clone)]
struct RawRow {
    category: RawCategory,
    cells: Vec<RawCell>,
}

impl RawRow {
    fn new(category: RawCategory, cells: Vec<RawCell>) -> Self {
        Self { category, cells }
    }
}

/// Tracks the cumulative logical payload of decoded cells, capped by PAYLOAD_LIMIT (16 KiB).
///
/// Text and byte cells charge their byte length; bool charges 1 byte; i32/u32 charge 4;
/// i64 charges 8; null and wrong-type cells charge 1. Addition is checked.
///
/// Protocol, SQLx, container, and error-buffer allocations are excluded. This is neither
/// a physical-memory cap nor proof that the connection was closed.
#[derive(Default)]
struct LogicalPayloadBudget {
    used: usize,
}

impl LogicalPayloadBudget {
    fn charge(&mut self, cell: &RawCell) -> Result<(), SimulationMigrationLedgerError> {
        let bytes = match cell {
            RawCell::Text(value) => value.len(),
            RawCell::Bytes(value) => value.len(),
            RawCell::Bool(_) => 1,
            RawCell::I32(_) => 4,
            RawCell::I64(_) => 8,
            RawCell::Null | RawCell::WrongType => 1,
        };
        self.used = self
            .used
            .checked_add(bytes)
            .ok_or(SimulationMigrationLedgerError::LimitExceeded)?;
        if self.used > PAYLOAD_LIMIT {
            return Err(SimulationMigrationLedgerError::LimitExceeded);
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
        expected_cells: usize,
        budget: &'a mut LogicalPayloadBudget,
    ) -> Result<Self, SimulationMigrationLedgerError> {
        if row.category != category || row.cells.len() != expected_cells {
            return Err(SimulationMigrationLedgerError::CatalogInvalid);
        }
        Ok(Self {
            cells: row.cells.into_iter(),
            budget,
        })
    }

    fn next(&mut self) -> Result<RawCell, SimulationMigrationLedgerError> {
        let cell = self.cells.next().unwrap_or(RawCell::WrongType);
        self.budget.charge(&cell)?;
        Ok(cell)
    }

    fn bool(&mut self) -> Result<bool, SimulationMigrationLedgerError> {
        match self.next()? {
            RawCell::Bool(value) => Ok(value),
            _ => Err(SimulationMigrationLedgerError::DecodeFailed),
        }
    }

    fn i32(&mut self) -> Result<i32, SimulationMigrationLedgerError> {
        match self.next()? {
            RawCell::I32(value) => Ok(value),
            _ => Err(SimulationMigrationLedgerError::DecodeFailed),
        }
    }

    fn i64(&mut self) -> Result<i64, SimulationMigrationLedgerError> {
        match self.next()? {
            RawCell::I64(value) => Ok(value),
            _ => Err(SimulationMigrationLedgerError::DecodeFailed),
        }
    }

    fn optional_i32(&mut self) -> Result<Option<i32>, SimulationMigrationLedgerError> {
        match self.next()? {
            RawCell::I32(value) => Ok(Some(value)),
            RawCell::Null => Ok(None),
            _ => Err(SimulationMigrationLedgerError::DecodeFailed),
        }
    }

    fn optional_i64(&mut self) -> Result<Option<i64>, SimulationMigrationLedgerError> {
        match self.next()? {
            RawCell::I64(value) => Ok(Some(value)),
            RawCell::Null => Ok(None),
            _ => Err(SimulationMigrationLedgerError::DecodeFailed),
        }
    }

    fn text(&mut self, max_bytes: usize) -> Result<String, SimulationMigrationLedgerError> {
        self.text_inner(max_bytes, false)
    }

    fn text_allow_empty(
        &mut self,
        max_bytes: usize,
    ) -> Result<String, SimulationMigrationLedgerError> {
        self.text_inner(max_bytes, true)
    }

    fn text_inner(
        &mut self,
        max_bytes: usize,
        allow_empty: bool,
    ) -> Result<String, SimulationMigrationLedgerError> {
        match self.next()? {
            RawCell::Text(value)
                if value.len() <= max_bytes
                    && (allow_empty || !value.is_empty())
                    && !value.as_bytes().contains(&0) =>
            {
                Ok(value)
            }
            _ => Err(SimulationMigrationLedgerError::DecodeFailed),
        }
    }

    fn optional_bytes(
        &mut self,
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, SimulationMigrationLedgerError> {
        match self.next()? {
            RawCell::Bytes(value) if value.len() <= max_bytes => Ok(Some(value)),
            RawCell::Null => Ok(None),
            _ => Err(SimulationMigrationLedgerError::DecodeFailed),
        }
    }

    fn finish(self) -> Result<(), SimulationMigrationLedgerError> {
        if self.cells.len() == 0 {
            Ok(())
        } else {
            Err(SimulationMigrationLedgerError::CatalogInvalid)
        }
    }
}

#[derive(Clone, Copy)]
enum PgCellKind {
    Bool,
    I32,
    I64,
    Text { max_bytes: usize, allow_empty: bool },
    OptionalBytes { max_bytes: usize },
}

use PgCellKind::{Bool as PB, I32 as PI32, I64 as PI64, OptionalBytes as PBytes, Text as PText};

const CONTEXT_SHAPE: [PgCellKind; 29] = [
    PText {
        max_bytes: 63,
        allow_empty: false,
    },
    PText {
        max_bytes: 63,
        allow_empty: false,
    },
    PText {
        max_bytes: 63,
        allow_empty: false,
    },
    PText {
        max_bytes: 63,
        allow_empty: false,
    },
    PText {
        max_bytes: 63,
        allow_empty: false,
    },
    PI64,
    PI64,
    PI64,
    PI64,
    PI64,
    PI64,
    PB,
    PB,
    PB,
    PB,
    PB,
    PB,
    PText {
        max_bytes: 32,
        allow_empty: false,
    },
    PText {
        max_bytes: 32,
        allow_empty: false,
    },
    PB,
    PText {
        max_bytes: 32,
        allow_empty: false,
    },
    PB,
    PB,
    PB,
    PB,
    PB,
    PB,
    PB,
    PB,
];

const MEMBERSHIP_SHAPE: [PgCellKind; 6] = [PI64, PI64, PI64, PB, PB, PB];

const RELATION_SHAPE: [PgCellKind; 20] = [
    PI64,
    PText {
        max_bytes: 63,
        allow_empty: false,
    },
    PText {
        max_bytes: 63,
        allow_empty: false,
    },
    PI64,
    PI64,
    PText {
        max_bytes: 63,
        allow_empty: false,
    },
    PText {
        max_bytes: 32,
        allow_empty: false,
    },
    PText {
        max_bytes: 32,
        allow_empty: false,
    },
    PB,
    PB,
    PB,
    PB,
    PB,
    PB,
    PB,
    PB,
    PB,
    PB,
    PB,
    PB,
];

const COLUMN_SHAPE: [PgCellKind; 22] = [
    PI64,
    PText {
        max_bytes: 63,
        allow_empty: false,
    },
    PI32,
    PText {
        max_bytes: 63,
        allow_empty: false,
    },
    PText {
        max_bytes: 63,
        allow_empty: false,
    },
    PText {
        max_bytes: 63,
        allow_empty: false,
    },
    PText {
        max_bytes: 32,
        allow_empty: false,
    },
    PI64,
    PB,
    PB,
    PText {
        max_bytes: 32,
        allow_empty: true,
    },
    PText {
        max_bytes: 32,
        allow_empty: true,
    },
    PI32,
    PI32,
    PB,
    PB,
    PB,
    PB,
    PB,
    PB,
    PB,
    PB,
];

const PRIMARY_KEY_SHAPE: [PgCellKind; 26] = [
    PI64,
    PI64,
    PI64,
    PText {
        max_bytes: 63,
        allow_empty: false,
    },
    PText {
        max_bytes: 63,
        allow_empty: false,
    },
    PText {
        max_bytes: 32,
        allow_empty: false,
    },
    PI32,
    PI32,
    PB,
    PB,
    PB,
    PI64,
    PI32,
    PI32,
    PB,
    PB,
    PB,
    PB,
    PB,
    PB,
    PB,
    PI64,
    PText {
        max_bytes: 63,
        allow_empty: false,
    },
    PText {
        max_bytes: 32,
        allow_empty: false,
    },
    PText {
        max_bytes: 32,
        allow_empty: false,
    },
    PI64,
];

const ACL_SHAPE: [PgCellKind; 7] = [
    PI32,
    PI64,
    PI32,
    PI64,
    PI64,
    PText {
        max_bytes: 32,
        allow_empty: false,
    },
    PB,
];

const LEDGER_SHAPE: [PgCellKind; 5] = [PI64, PI64, PB, PI32, PBytes { max_bytes: 48 }];

fn raw_pg_cell(row: &PgRow, index: usize, kind: PgCellKind) -> Result<RawCell, TransportFailure> {
    match kind {
        PgCellKind::Bool => match row.try_get::<Option<bool>, _>(index) {
            Ok(Some(value)) => Ok(RawCell::Bool(value)),
            Ok(None) => Ok(RawCell::Null),
            Err(_) => Ok(RawCell::WrongType),
        },
        PgCellKind::I32 => match row.try_get::<Option<i32>, _>(index) {
            Ok(Some(value)) => Ok(RawCell::I32(value)),
            Ok(None) => Ok(RawCell::Null),
            Err(_) => Ok(RawCell::WrongType),
        },
        PgCellKind::I64 => match row.try_get::<Option<i64>, _>(index) {
            Ok(Some(value)) => Ok(RawCell::I64(value)),
            Ok(None) => Ok(RawCell::Null),
            Err(_) => Ok(RawCell::WrongType),
        },
        PgCellKind::Text {
            max_bytes,
            allow_empty,
        } => match row.try_get::<Option<&str>, _>(index) {
            Ok(Some(value))
                if value.len() <= max_bytes
                    && (allow_empty || !value.is_empty())
                    && !value.as_bytes().contains(&0) =>
            {
                Ok(RawCell::Text(value.to_owned()))
            }
            Ok(Some(value)) if value.len() > max_bytes => Err(TransportFailure::OverLimit),
            Ok(Some(_)) => Ok(RawCell::WrongType),
            Ok(None) => Ok(RawCell::Null),
            Err(_) => Ok(RawCell::WrongType),
        },
        PgCellKind::OptionalBytes { max_bytes } => match row.try_get::<Option<&[u8]>, _>(index) {
            Ok(Some(value)) if value.len() <= max_bytes => Ok(RawCell::Bytes(value.to_vec())),
            Ok(Some(_)) => Err(TransportFailure::OverLimit),
            Ok(None) => Ok(RawCell::Null),
            Err(_) => Ok(RawCell::WrongType),
        },
    }
}

fn raw_pg_rows(
    rows: Vec<PgRow>,
    category: RawCategory,
    shape: &[PgCellKind],
    fetch_limit: usize,
) -> Result<Vec<RawRow>, TransportFailure> {
    if rows.len() > fetch_limit {
        return Err(TransportFailure::OverLimit);
    }
    rows.iter()
        .map(|row| {
            if row.columns().len() != shape.len() {
                return Err(TransportFailure::UnexpectedShape);
            }
            let cells = shape
                .iter()
                .enumerate()
                .map(|(index, kind)| raw_pg_cell(row, index, *kind))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(RawRow::new(category, cells))
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Statement {
    Begin,
    Configure {
        timeout_ms: i32,
    },
    Context,
    Memberships {
        worker_oid: u32,
    },
    Relation {
        public_schema_oid: u32,
    },
    Columns {
        ledger_oid: u32,
    },
    PrimaryKey {
        public_schema_oid: u32,
        ledger_oid: u32,
    },
    RawAcls {
        database_oid: u32,
        public_schema_oid: u32,
        ledger_oid: u32,
        worker_oid: u32,
    },
    Ledger,
    Commit,
    Rollback,
}

#[derive(Clone, Copy)]
enum TransportFailure {
    Database,
    State,
    OverLimit,
    UnexpectedShape,
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

    async fn fetch_context(&mut self) -> Result<Vec<RawRow>, TransportFailure> {
        let rows = sqlx::query(CONTEXT_SQL)
            .fetch_all(self.connection()?)
            .await
            .map_err(|_| TransportFailure::Database)?;
        raw_pg_rows(
            rows,
            RawCategory::Context,
            &CONTEXT_SHAPE,
            CONTEXT_FETCH_LIMIT,
        )
    }

    async fn fetch_memberships(
        &mut self,
        worker_oid: u32,
    ) -> Result<Vec<RawRow>, TransportFailure> {
        let rows = sqlx::query(MEMBERSHIPS_SQL)
            .bind(i64::from(worker_oid))
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

    async fn fetch_relation(
        &mut self,
        public_schema_oid: u32,
    ) -> Result<Vec<RawRow>, TransportFailure> {
        let rows = sqlx::query(RELATION_SQL)
            .bind(i64::from(public_schema_oid))
            .fetch_all(self.connection()?)
            .await
            .map_err(|_| TransportFailure::Database)?;
        raw_pg_rows(
            rows,
            RawCategory::Relation,
            &RELATION_SHAPE,
            RELATION_FETCH_LIMIT,
        )
    }

    async fn fetch_columns(&mut self, ledger_oid: u32) -> Result<Vec<RawRow>, TransportFailure> {
        let rows = sqlx::query(COLUMNS_SQL)
            .bind(i64::from(ledger_oid))
            .fetch_all(self.connection()?)
            .await
            .map_err(|_| TransportFailure::Database)?;
        raw_pg_rows(rows, RawCategory::Column, &COLUMN_SHAPE, COLUMN_FETCH_LIMIT)
    }

    async fn fetch_primary_key(
        &mut self,
        public_schema_oid: u32,
        ledger_oid: u32,
    ) -> Result<Vec<RawRow>, TransportFailure> {
        let rows = sqlx::query(PRIMARY_KEY_SQL)
            .bind(i64::from(ledger_oid))
            .bind(i64::from(public_schema_oid))
            .fetch_all(self.connection()?)
            .await
            .map_err(|_| TransportFailure::Database)?;
        raw_pg_rows(
            rows,
            RawCategory::PrimaryKey,
            &PRIMARY_KEY_SHAPE,
            PRIMARY_KEY_FETCH_LIMIT,
        )
    }

    async fn fetch_raw_acls(
        &mut self,
        database_oid: u32,
        public_schema_oid: u32,
        ledger_oid: u32,
        worker_oid: u32,
    ) -> Result<Vec<RawRow>, TransportFailure> {
        let rows = sqlx::query(RAW_ACLS_SQL)
            .bind(i64::from(database_oid))
            .bind(i64::from(public_schema_oid))
            .bind(i64::from(ledger_oid))
            .bind(i64::from(worker_oid))
            .fetch_all(self.connection()?)
            .await
            .map_err(|_| TransportFailure::Database)?;
        raw_pg_rows(rows, RawCategory::Acl, &ACL_SHAPE, RAW_ACL_FETCH_LIMIT)
    }

    async fn fetch_ledger(&mut self) -> Result<Vec<RawRow>, TransportFailure> {
        let rows = sqlx::query(LEDGER_SQL)
            .fetch_all(self.connection()?)
            .await
            .map_err(|_| TransportFailure::Database)?;
        raw_pg_rows(rows, RawCategory::Ledger, &LEDGER_SHAPE, LEDGER_FETCH_LIMIT)
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
                let timeout = timeout_ms.to_string();
                sqlx::query(SETUP_SQL)
                    .bind(timeout)
                    .execute(self.connection()?)
                    .await
                    .map_err(|_| TransportFailure::Database)?;
                Ok(Vec::new())
            }
            Statement::Context => self.fetch_context().await,
            Statement::Memberships { worker_oid } => self.fetch_memberships(worker_oid).await,
            Statement::Relation { public_schema_oid } => {
                self.fetch_relation(public_schema_oid).await
            }
            Statement::Columns { ledger_oid } => self.fetch_columns(ledger_oid).await,
            Statement::PrimaryKey {
                public_schema_oid,
                ledger_oid,
            } => self.fetch_primary_key(public_schema_oid, ledger_oid).await,
            Statement::RawAcls {
                database_oid,
                public_schema_oid,
                ledger_oid,
                worker_oid,
            } => {
                self.fetch_raw_acls(database_oid, public_schema_oid, ledger_oid, worker_oid)
                    .await
            }
            Statement::Ledger => self.fetch_ledger().await,
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
    database_raw_grant_option: bool,
    schema_raw_grant_option: bool,
}

#[derive(Clone)]
#[allow(dead_code)] // Membership fields are decoded before nonempty rows are rejected.
struct MembershipRow {
    member_oid: i64,
    role_oid: i64,
    grantor_oid: i64,
    admin_option: bool,
    inherit_option: bool,
    set_option: bool,
}

#[derive(Clone)]
struct RelationRow {
    schema_oid: i64,
    schema_name: String,
    relation_name: String,
    relation_oid: i64,
    owner_oid: i64,
    owner_name: String,
    relation_kind: String,
    persistence: String,
    is_partition: bool,
    has_inheritance_edge: bool,
    row_security: bool,
    force_row_security: bool,
    table_select: bool,
    table_insert: bool,
    table_update: bool,
    table_delete: bool,
    table_truncate: bool,
    table_references: bool,
    table_trigger: bool,
    table_effective_grant_option: bool,
}

#[derive(Clone)]
struct ColumnRow {
    relation_oid: i64,
    relation_name: String,
    attnum: i32,
    column_name: String,
    type_namespace: String,
    type_name: String,
    type_kind: String,
    type_base_oid: i64,
    not_null: bool,
    is_dropped: bool,
    generated: String,
    identity: String,
    dimensions: i32,
    type_modifier: i32,
    column_select: bool,
    column_insert: bool,
    column_update: bool,
    column_references: bool,
    select_grant_option: bool,
    insert_grant_option: bool,
    update_grant_option: bool,
    references_grant_option: bool,
}

#[derive(Clone)]
struct PrimaryKeyRow {
    relation_oid: i64,
    schema_oid: i64,
    constraint_namespace_oid: i64,
    constraint_namespace: String,
    constraint_name: String,
    constraint_kind: String,
    key_count: i32,
    first_attnum: i32,
    is_validated: bool,
    is_deferrable: bool,
    is_deferred: bool,
    index_oid: i64,
    index_attribute_count: i32,
    index_key_count: i32,
    is_unique: bool,
    is_primary: bool,
    is_valid: bool,
    is_ready: bool,
    is_immediate: bool,
    has_no_expressions: bool,
    has_no_predicate: bool,
    index_namespace_oid: i64,
    index_namespace: String,
    index_kind: String,
    index_persistence: String,
    index_owner_oid: i64,
}

#[derive(Clone)]
struct RawAclRow {
    category: i32,
    relation_oid: Option<i64>,
    attnum: Option<i32>,
    grantee_oid: i64,
    grantor_oid: i64,
    privilege_type: String,
    is_grantable: bool,
}

#[derive(Clone)]
struct LedgerRow {
    ledger_oid: i64,
    version: i64,
    success: bool,
    checksum_len: i32,
    checksum: Option<Vec<u8>>,
}

fn decode_context(
    row: RawRow,
    budget: &mut LogicalPayloadBudget,
) -> Result<ContextRow, SimulationMigrationLedgerError> {
    let mut d = RawDecoder::new(row, RawCategory::Context, CONTEXT_SHAPE.len(), budget)?;
    let value = ContextRow {
        current_user: d.text(63)?,
        session_user: d.text(63)?,
        observed_database_name: d.text(63)?,
        database_name: d.text(63)?,
        schema_name: d.text(63)?,
        worker_oid: d.i64()?,
        database_oid: d.i64()?,
        schema_oid: d.i64()?,
        migration_owner_oid: d.i64()?,
        database_owner_oid: d.i64()?,
        schema_owner_oid: d.i64()?,
        can_login: d.bool()?,
        is_superuser: d.bool()?,
        can_create_database: d.bool()?,
        can_create_role: d.bool()?,
        can_replicate: d.bool()?,
        bypasses_rls: d.bool()?,
        row_security: d.text(32)?,
        session_replication_role: d.text(32)?,
        transaction_read_only: d.bool()?,
        transaction_isolation: d.text(32)?,
        database_connect: d.bool()?,
        database_create: d.bool()?,
        schema_usage: d.bool()?,
        schema_create: d.bool()?,
        database_effective_grant_option: d.bool()?,
        schema_effective_grant_option: d.bool()?,
        database_raw_grant_option: d.bool()?,
        schema_raw_grant_option: d.bool()?,
    };
    d.finish()?;
    Ok(value)
}

fn decode_membership(
    row: RawRow,
    budget: &mut LogicalPayloadBudget,
) -> Result<MembershipRow, SimulationMigrationLedgerError> {
    let mut d = RawDecoder::new(row, RawCategory::Membership, MEMBERSHIP_SHAPE.len(), budget)?;
    let value = MembershipRow {
        member_oid: d.i64()?,
        role_oid: d.i64()?,
        grantor_oid: d.i64()?,
        admin_option: d.bool()?,
        inherit_option: d.bool()?,
        set_option: d.bool()?,
    };
    d.finish()?;
    Ok(value)
}

fn decode_relation(
    row: RawRow,
    budget: &mut LogicalPayloadBudget,
) -> Result<RelationRow, SimulationMigrationLedgerError> {
    let mut d = RawDecoder::new(row, RawCategory::Relation, RELATION_SHAPE.len(), budget)?;
    let value = RelationRow {
        schema_oid: d.i64()?,
        schema_name: d.text(63)?,
        relation_name: d.text(63)?,
        relation_oid: d.i64()?,
        owner_oid: d.i64()?,
        owner_name: d.text(63)?,
        relation_kind: d.text(32)?,
        persistence: d.text(32)?,
        is_partition: d.bool()?,
        has_inheritance_edge: d.bool()?,
        row_security: d.bool()?,
        force_row_security: d.bool()?,
        table_select: d.bool()?,
        table_insert: d.bool()?,
        table_update: d.bool()?,
        table_delete: d.bool()?,
        table_truncate: d.bool()?,
        table_references: d.bool()?,
        table_trigger: d.bool()?,
        table_effective_grant_option: d.bool()?,
    };
    d.finish()?;
    Ok(value)
}

fn decode_column(
    row: RawRow,
    budget: &mut LogicalPayloadBudget,
) -> Result<ColumnRow, SimulationMigrationLedgerError> {
    let mut d = RawDecoder::new(row, RawCategory::Column, COLUMN_SHAPE.len(), budget)?;
    let value = ColumnRow {
        relation_oid: d.i64()?,
        relation_name: d.text(63)?,
        attnum: d.i32()?,
        column_name: d.text(63)?,
        type_namespace: d.text(63)?,
        type_name: d.text(63)?,
        type_kind: d.text(32)?,
        type_base_oid: d.i64()?,
        not_null: d.bool()?,
        is_dropped: d.bool()?,
        generated: d.text_allow_empty(32)?,
        identity: d.text_allow_empty(32)?,
        dimensions: d.i32()?,
        type_modifier: d.i32()?,
        column_select: d.bool()?,
        column_insert: d.bool()?,
        column_update: d.bool()?,
        column_references: d.bool()?,
        select_grant_option: d.bool()?,
        insert_grant_option: d.bool()?,
        update_grant_option: d.bool()?,
        references_grant_option: d.bool()?,
    };
    d.finish()?;
    Ok(value)
}

fn decode_primary_key(
    row: RawRow,
    budget: &mut LogicalPayloadBudget,
) -> Result<PrimaryKeyRow, SimulationMigrationLedgerError> {
    let mut d = RawDecoder::new(
        row,
        RawCategory::PrimaryKey,
        PRIMARY_KEY_SHAPE.len(),
        budget,
    )?;
    let value = PrimaryKeyRow {
        relation_oid: d.i64()?,
        schema_oid: d.i64()?,
        constraint_namespace_oid: d.i64()?,
        constraint_namespace: d.text(63)?,
        constraint_name: d.text(63)?,
        constraint_kind: d.text(32)?,
        key_count: d.i32()?,
        first_attnum: d.i32()?,
        is_validated: d.bool()?,
        is_deferrable: d.bool()?,
        is_deferred: d.bool()?,
        index_oid: d.i64()?,
        index_attribute_count: d.i32()?,
        index_key_count: d.i32()?,
        is_unique: d.bool()?,
        is_primary: d.bool()?,
        is_valid: d.bool()?,
        is_ready: d.bool()?,
        is_immediate: d.bool()?,
        has_no_expressions: d.bool()?,
        has_no_predicate: d.bool()?,
        index_namespace_oid: d.i64()?,
        index_namespace: d.text(63)?,
        index_kind: d.text(32)?,
        index_persistence: d.text(32)?,
        index_owner_oid: d.i64()?,
    };
    d.finish()?;
    Ok(value)
}

fn decode_acl(
    row: RawRow,
    budget: &mut LogicalPayloadBudget,
) -> Result<RawAclRow, SimulationMigrationLedgerError> {
    let mut d = RawDecoder::new(row, RawCategory::Acl, ACL_SHAPE.len(), budget)?;
    let value = RawAclRow {
        category: d.i32()?,
        relation_oid: d.optional_i64()?,
        attnum: d.optional_i32()?,
        grantee_oid: d.i64()?,
        grantor_oid: d.i64()?,
        privilege_type: d.text(32)?,
        is_grantable: d.bool()?,
    };
    d.finish()?;
    Ok(value)
}

fn decode_ledger(
    row: RawRow,
    budget: &mut LogicalPayloadBudget,
) -> Result<LedgerRow, SimulationMigrationLedgerError> {
    let mut d = RawDecoder::new(row, RawCategory::Ledger, LEDGER_SHAPE.len(), budget)?;
    let value = LedgerRow {
        ledger_oid: d.i64()?,
        version: d.i64()?,
        success: d.bool()?,
        checksum_len: d.i32()?,
        checksum: d.optional_bytes(48)?,
    };
    d.finish()?;
    Ok(value)
}

#[derive(Clone, Copy)]
struct BaselineEvidence {
    fingerprint: [u8; 32],
    fingerprint_text: &'static str,
}

#[derive(Clone, Copy)]
struct ContextEvidence {
    database_oid: u32,
    worker_oid: u32,
    public_schema_oid: u32,
    migration_owner_oid: u32,
}

#[derive(Clone, Copy)]
struct LedgerIdentity {
    database_oid: u32,
    worker_oid: u32,
    public_schema_oid: u32,
    migration_owner_oid: u32,
    ledger_oid: u32,
}

struct LedgerEvidence {
    row_count: u64,
    digest_text: String,
}

fn validate_baseline_metadata(
    entries: &[ExpectedMigration],
) -> Result<(), SimulationMigrationLedgerError> {
    if entries.len() != EXPECTED_ROW_COUNT {
        return Err(SimulationMigrationLedgerError::BaselineInvalid);
    }
    let mut paths = HashSet::new();
    for (index, entry) in entries.iter().enumerate() {
        let relative = entry
            .path
            .strip_prefix("migrations/")
            .ok_or(SimulationMigrationLedgerError::BaselineInvalid)?;
        if entry.version != (index as i64) + 1
            || !entry.path.ends_with(".up.sql")
            || relative.is_empty()
            || relative.contains('/')
            || !relative.is_ascii()
            || !paths.insert(entry.path)
        {
            return Err(SimulationMigrationLedgerError::BaselineInvalid);
        }
    }
    Ok(())
}

fn validate_expected_entry(
    entry: &ExpectedMigration,
    bytes: &[u8],
) -> Result<([u8; 32], [u8; 48]), SimulationMigrationLedgerError> {
    if bytes.len() != entry.byte_len || std::str::from_utf8(bytes).is_err() {
        return Err(SimulationMigrationLedgerError::BaselineInvalid);
    }
    let sha256 = Sha256::digest(bytes);
    let sha384 = Sha384::digest(bytes);
    if format!("{:x}", sha256) != entry.sha256_hex || format!("{:x}", sha384) != entry.sha384_hex {
        return Err(SimulationMigrationLedgerError::BaselineInvalid);
    }
    let mut raw256 = [0u8; 32];
    let mut raw384 = [0u8; 48];
    raw256.copy_from_slice(&sha256);
    raw384.copy_from_slice(&sha384);
    Ok((raw256, raw384))
}

fn validate_embedded_baseline() -> Result<BaselineEvidence, SimulationMigrationLedgerError> {
    validate_baseline_metadata(&BASELINE)?;
    let mut fingerprint = Sha256::new();
    fingerprint.update(b"lagrange/simulation-migration-baseline/v1");
    fingerprint.update([0u8]);
    fingerprint.update(READER_FLOOR.to_be_bytes());
    fingerprint.update(REQUIRED_EXTENSION.to_be_bytes());
    fingerprint.update((EXPECTED_ROW_COUNT as u64).to_be_bytes());

    for entry in BASELINE.iter() {
        let path = entry.path.as_bytes();
        let path_len = u32::try_from(path.len())
            .map_err(|_| SimulationMigrationLedgerError::BaselineInvalid)?;
        let size = u64::try_from(entry.byte_len)
            .map_err(|_| SimulationMigrationLedgerError::BaselineInvalid)?;
        let (sha256, sha384) = validate_expected_entry(entry, entry.bytes)?;
        fingerprint.update(entry.version.to_be_bytes());
        fingerprint.update(path_len.to_be_bytes());
        fingerprint.update(path);
        fingerprint.update(size.to_be_bytes());
        fingerprint.update(sha256);
        fingerprint.update(sha384);
    }
    let digest = fingerprint.finalize();
    let digest_text = format!("sha256:{:x}", digest);
    if digest_text != EXPECTED_FINGERPRINT {
        return Err(SimulationMigrationLedgerError::BaselineInvalid);
    }
    let mut raw = [0u8; 32];
    raw.copy_from_slice(&digest);
    Ok(BaselineEvidence {
        fingerprint: raw,
        fingerprint_text: EXPECTED_FINGERPRINT,
    })
}

struct PreparedObservation {
    identity: LedgerIdentity,
    ledger: LedgerEvidence,
}

pub(super) async fn observe_simulation_migration_ledger(
    pool: &PgPool,
    config: &SimulationProcessConfig,
    canceled: &AtomicBool,
) -> Result<SimulationMigrationLedgerObservation, SimulationMigrationLedgerError> {
    let baseline = validate_embedded_baseline()?;
    if is_canceled(canceled) {
        return Err(SimulationMigrationLedgerError::Canceled);
    }
    let timeout_ms = timeout_millis(config.limits().startup_timeout_seconds())?;
    let mut transport = SqlxObserverTransport::new(pool);
    observe_with_transport(
        &mut transport,
        config.config_sha256(),
        canceled,
        baseline,
        timeout_ms,
    )
    .await
}

fn timeout_millis(timeout_seconds: u64) -> Result<i32, SimulationMigrationLedgerError> {
    let timeout_ms = timeout_seconds
        .checked_mul(1000)
        .filter(|value| *value > 0 && *value <= i32::MAX as u64)
        .ok_or(SimulationMigrationLedgerError::InvalidBound)?;
    Ok(timeout_ms as i32)
}

fn is_canceled(canceled: &AtomicBool) -> bool {
    canceled.load(Ordering::Acquire)
}

async fn observe_with_transport<T: ObserverTransport>(
    transport: &mut T,
    config_sha256: &str,
    canceled: &AtomicBool,
    baseline: BaselineEvidence,
    timeout_ms: i32,
) -> Result<SimulationMigrationLedgerObservation, SimulationMigrationLedgerError> {
    if timeout_ms <= 0 {
        return Err(SimulationMigrationLedgerError::InvalidBound);
    }
    if is_canceled(canceled) {
        return Err(SimulationMigrationLedgerError::Canceled);
    }
    transport
        .acquire()
        .await
        .map_err(|_| SimulationMigrationLedgerError::AcquisitionFailed)?;
    transport.arm_close_on_drop();

    if is_canceled(canceled) {
        return close_without_transaction(transport, SimulationMigrationLedgerError::Canceled)
            .await;
    }

    let begin = transport.execute(Statement::Begin).await;
    let begin_rows = match begin {
        Ok(rows) => rows,
        Err(_) => {
            return close_without_transaction(
                transport,
                SimulationMigrationLedgerError::BeginFailed,
            )
            .await;
        }
    };
    if !begin_rows.is_empty() {
        return rollback_and_close(
            transport,
            SimulationMigrationLedgerError::BeginResultInvalid,
        )
        .await;
    }
    if is_canceled(canceled) {
        return rollback_and_close(transport, SimulationMigrationLedgerError::Canceled).await;
    }

    let prepared = match prepare_observation(transport, canceled, baseline, timeout_ms).await {
        Ok(prepared) => prepared,
        Err(error) => return rollback_and_close(transport, error).await,
    };
    if is_canceled(canceled) {
        return rollback_and_close(transport, SimulationMigrationLedgerError::Canceled).await;
    }

    let commit = transport.execute(Statement::Commit).await;
    match commit {
        Ok(rows) if rows.is_empty() => {
            if transport.close().await.is_err() {
                return Err(SimulationMigrationLedgerError::CleanupUnconfirmed);
            }
            if is_canceled(canceled) {
                return Err(SimulationMigrationLedgerError::Canceled);
            }
            Ok(SimulationMigrationLedgerObservation::completed(
                config_sha256,
                baseline.fingerprint_text,
                prepared.identity,
                prepared.ledger,
            ))
        }
        _ => {
            if transport.close().await.is_err() {
                Err(SimulationMigrationLedgerError::CompletionAndCleanupUnconfirmed)
            } else {
                Err(SimulationMigrationLedgerError::CompletionUnconfirmed)
            }
        }
    }
}

async fn close_without_transaction<T: ObserverTransport>(
    transport: &mut T,
    original: SimulationMigrationLedgerError,
) -> Result<SimulationMigrationLedgerObservation, SimulationMigrationLedgerError> {
    if transport.close().await.is_err() {
        Err(SimulationMigrationLedgerError::CleanupUnconfirmed)
    } else {
        Err(original)
    }
}

async fn rollback_and_close<T: ObserverTransport>(
    transport: &mut T,
    original: SimulationMigrationLedgerError,
) -> Result<SimulationMigrationLedgerObservation, SimulationMigrationLedgerError> {
    let rollback = transport.execute(Statement::Rollback).await;
    let rollback_confirmed = matches!(rollback, Ok(rows) if rows.is_empty());
    let close_confirmed = transport.close().await.is_ok();
    match (rollback_confirmed, close_confirmed) {
        (true, true) => Err(original),
        (true, false) => Err(SimulationMigrationLedgerError::CleanupUnconfirmed),
        (false, true) => Err(SimulationMigrationLedgerError::CompletionUnconfirmed),
        (false, false) => Err(SimulationMigrationLedgerError::CompletionAndCleanupUnconfirmed),
    }
}

async fn prepare_observation<T: ObserverTransport>(
    transport: &mut T,
    canceled: &AtomicBool,
    baseline: BaselineEvidence,
    timeout_ms: i32,
) -> Result<PreparedObservation, SimulationMigrationLedgerError> {
    let no_rows = execute_checked(
        transport,
        Statement::Configure { timeout_ms },
        canceled,
        SimulationMigrationLedgerError::SetupFailed,
    )
    .await?;
    if !no_rows.is_empty() {
        return Err(SimulationMigrationLedgerError::SetupFailed);
    }

    let mut budget = LogicalPayloadBudget::default();
    let context_rows = fetch_checked(
        transport,
        Statement::Context,
        canceled,
        SimulationMigrationLedgerError::CatalogUnavailable,
    )
    .await?;
    check_fetch_bound(context_rows.len(), CONTEXT_FETCH_LIMIT)?;
    let context_raw = exactly_one(
        context_rows,
        SimulationMigrationLedgerError::CatalogUnavailable,
    )?;
    let context = decode_context(context_raw, &mut budget)?;
    let context = validate_context(context)?;

    let membership_rows = fetch_checked(
        transport,
        Statement::Memberships {
            worker_oid: context.worker_oid,
        },
        canceled,
        SimulationMigrationLedgerError::CatalogUnavailable,
    )
    .await?;
    check_fetch_bound(membership_rows.len(), MEMBERSHIP_FETCH_LIMIT)?;
    if let Some(row) = membership_rows.into_iter().next() {
        let _ = decode_membership(row, &mut budget)?;
        return Err(SimulationMigrationLedgerError::AccessRejected);
    }

    let relation_rows = fetch_checked(
        transport,
        Statement::Relation {
            public_schema_oid: context.public_schema_oid,
        },
        canceled,
        SimulationMigrationLedgerError::CatalogUnavailable,
    )
    .await?;
    check_fetch_bound(relation_rows.len(), RELATION_FETCH_LIMIT)?;
    let relation_raw = exactly_one(
        relation_rows,
        SimulationMigrationLedgerError::CatalogUnavailable,
    )?;
    let relation = decode_relation(relation_raw, &mut budget)?;
    let relation_oid = validate_relation(relation, context)?;

    let column_rows = fetch_checked(
        transport,
        Statement::Columns {
            ledger_oid: relation_oid,
        },
        canceled,
        SimulationMigrationLedgerError::CatalogUnavailable,
    )
    .await?;
    check_fetch_bound(column_rows.len(), COLUMN_FETCH_LIMIT)?;
    if column_rows.len() > 6 {
        return Err(SimulationMigrationLedgerError::LimitExceeded);
    }
    let columns = column_rows
        .into_iter()
        .map(|row| decode_column(row, &mut budget))
        .collect::<Result<Vec<_>, _>>()?;
    validate_columns(&columns, relation_oid)?;

    let primary_key_rows = fetch_checked(
        transport,
        Statement::PrimaryKey {
            public_schema_oid: context.public_schema_oid,
            ledger_oid: relation_oid,
        },
        canceled,
        SimulationMigrationLedgerError::CatalogUnavailable,
    )
    .await?;
    check_fetch_bound(primary_key_rows.len(), PRIMARY_KEY_FETCH_LIMIT)?;
    let primary_key_raw = exactly_one(
        primary_key_rows,
        SimulationMigrationLedgerError::CatalogUnavailable,
    )?;
    let primary_key = decode_primary_key(primary_key_raw, &mut budget)?;
    validate_primary_key(primary_key, context, relation_oid)?;

    let acl_rows = fetch_checked(
        transport,
        Statement::RawAcls {
            database_oid: context.database_oid,
            public_schema_oid: context.public_schema_oid,
            ledger_oid: relation_oid,
            worker_oid: context.worker_oid,
        },
        canceled,
        SimulationMigrationLedgerError::CatalogUnavailable,
    )
    .await?;
    check_fetch_bound(acl_rows.len(), RAW_ACL_FETCH_LIMIT)?;
    if acl_rows.len() > RAW_ACL_ACCEPT_LIMIT {
        return Err(SimulationMigrationLedgerError::LimitExceeded);
    }
    let raw_acls = acl_rows
        .into_iter()
        .map(|row| decode_acl(row, &mut budget))
        .collect::<Result<Vec<_>, _>>()?;
    validate_raw_acls(&raw_acls, context, relation_oid)?;

    let ledger_rows = fetch_checked(
        transport,
        Statement::Ledger,
        canceled,
        SimulationMigrationLedgerError::LedgerUnavailable,
    )
    .await?;
    check_fetch_bound(ledger_rows.len(), LEDGER_FETCH_LIMIT)?;
    let ledger = ledger_rows
        .into_iter()
        .map(|row| decode_ledger(row, &mut budget))
        .collect::<Result<Vec<_>, _>>()?;
    let identity = LedgerIdentity {
        database_oid: context.database_oid,
        worker_oid: context.worker_oid,
        public_schema_oid: context.public_schema_oid,
        migration_owner_oid: context.migration_owner_oid,
        ledger_oid: relation_oid,
    };
    let ledger = validate_ledger(ledger, identity, baseline)?;
    Ok(PreparedObservation { identity, ledger })
}

async fn execute_checked<T: ObserverTransport>(
    transport: &mut T,
    statement: Statement,
    canceled: &AtomicBool,
    failure: SimulationMigrationLedgerError,
) -> Result<Vec<RawRow>, SimulationMigrationLedgerError> {
    if is_canceled(canceled) {
        return Err(SimulationMigrationLedgerError::Canceled);
    }
    let result = transport.execute(statement).await;
    match result {
        Ok(rows) => {
            if is_canceled(canceled) {
                Err(SimulationMigrationLedgerError::Canceled)
            } else {
                Ok(rows)
            }
        }
        Err(error) => Err(map_transport_failure(error, failure)),
    }
}

async fn fetch_checked<T: ObserverTransport>(
    transport: &mut T,
    statement: Statement,
    canceled: &AtomicBool,
    failure: SimulationMigrationLedgerError,
) -> Result<Vec<RawRow>, SimulationMigrationLedgerError> {
    execute_checked(transport, statement, canceled, failure).await
}

fn map_transport_failure(
    failure: TransportFailure,
    unavailable: SimulationMigrationLedgerError,
) -> SimulationMigrationLedgerError {
    match failure {
        TransportFailure::Database => unavailable,
        TransportFailure::State => SimulationMigrationLedgerError::CatalogInvalid,
        TransportFailure::OverLimit => SimulationMigrationLedgerError::LimitExceeded,
        TransportFailure::UnexpectedShape => SimulationMigrationLedgerError::CatalogInvalid,
    }
}

fn check_fetch_bound(
    actual: usize,
    fetch_limit: usize,
) -> Result<(), SimulationMigrationLedgerError> {
    if actual > fetch_limit {
        Err(SimulationMigrationLedgerError::LimitExceeded)
    } else {
        Ok(())
    }
}

fn exactly_one(
    mut rows: Vec<RawRow>,
    empty_error: SimulationMigrationLedgerError,
) -> Result<RawRow, SimulationMigrationLedgerError> {
    match rows.len() {
        0 => Err(empty_error),
        1 => Ok(rows.remove(0)),
        _ => Err(SimulationMigrationLedgerError::CatalogInvalid),
    }
}

fn positive_oid(value: i64) -> Result<u32, SimulationMigrationLedgerError> {
    let oid = u32::try_from(value).map_err(|_| SimulationMigrationLedgerError::CatalogInvalid)?;
    if oid == 0 {
        Err(SimulationMigrationLedgerError::CatalogInvalid)
    } else {
        Ok(oid)
    }
}

fn validate_context(row: ContextRow) -> Result<ContextEvidence, SimulationMigrationLedgerError> {
    let worker_oid = positive_oid(row.worker_oid)?;
    let database_oid = positive_oid(row.database_oid)?;
    let public_schema_oid = positive_oid(row.schema_oid)?;
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
        return Err(SimulationMigrationLedgerError::CatalogInvalid);
    }

    if !row.can_login
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
        || row.database_raw_grant_option
        || row.schema_raw_grant_option
    {
        return Err(SimulationMigrationLedgerError::AccessRejected);
    }

    Ok(ContextEvidence {
        database_oid,
        worker_oid,
        public_schema_oid,
        migration_owner_oid,
    })
}

fn validate_relation(
    row: RelationRow,
    context: ContextEvidence,
) -> Result<u32, SimulationMigrationLedgerError> {
    let relation_oid = positive_oid(row.relation_oid)?;
    let owner_oid = positive_oid(row.owner_oid)?;
    if row.schema_oid != i64::from(context.public_schema_oid)
        || row.schema_name != PUBLIC_SCHEMA
        || row.relation_name != LEDGER_NAME
        || row.owner_oid != i64::from(context.migration_owner_oid)
        || row.owner_name != MIGRATION_OWNER_ROLE
        || owner_oid == context.worker_oid
        || row.relation_kind != "r"
        || row.persistence != "p"
        || row.is_partition
        || row.has_inheritance_edge
        || row.row_security
        || row.force_row_security
    {
        return Err(SimulationMigrationLedgerError::CatalogInvalid);
    }
    if !row.table_select
        || row.table_insert
        || row.table_update
        || row.table_delete
        || row.table_truncate
        || row.table_references
        || row.table_trigger
        || row.table_effective_grant_option
    {
        return Err(SimulationMigrationLedgerError::AccessRejected);
    }
    Ok(relation_oid)
}

fn validate_columns(
    rows: &[ColumnRow],
    relation_oid: u32,
) -> Result<(), SimulationMigrationLedgerError> {
    if rows.len() != 6 {
        return Err(SimulationMigrationLedgerError::CatalogInvalid);
    }
    let expected = [
        (1, "version", "int8"),
        (2, "description", "text"),
        (3, "installed_on", "timestamptz"),
        (4, "success", "bool"),
        (5, "checksum", "bytea"),
        (6, "execution_time", "int8"),
    ];
    for (row, (attnum, name, type_name)) in rows.iter().zip(expected) {
        if row.relation_oid != i64::from(relation_oid)
            || row.relation_name != LEDGER_NAME
            || row.attnum != attnum
            || row.column_name != name
            || row.type_namespace != PUBLIC_SCHEMA_PG
            || row.type_name != type_name
            || row.type_kind != "b"
            || row.type_base_oid != 0
            || !row.not_null
            || row.is_dropped
            || !row.generated.is_empty()
            || !row.identity.is_empty()
            || row.dimensions != 0
            || row.type_modifier != -1
        {
            return Err(SimulationMigrationLedgerError::CatalogInvalid);
        }
        if !row.column_select
            || row.column_insert
            || row.column_update
            || row.column_references
            || row.select_grant_option
            || row.insert_grant_option
            || row.update_grant_option
            || row.references_grant_option
        {
            return Err(SimulationMigrationLedgerError::AccessRejected);
        }
    }
    Ok(())
}

fn validate_primary_key(
    row: PrimaryKeyRow,
    context: ContextEvidence,
    relation_oid: u32,
) -> Result<u32, SimulationMigrationLedgerError> {
    let index_oid = positive_oid(row.index_oid)?;
    let index_owner_oid = positive_oid(row.index_owner_oid)?;
    if row.relation_oid != i64::from(relation_oid)
        || row.schema_oid != i64::from(context.public_schema_oid)
        || row.constraint_namespace_oid != i64::from(context.public_schema_oid)
        || row.constraint_namespace != PUBLIC_SCHEMA
        || row.constraint_name.is_empty()
        || row.constraint_kind != "p"
        || row.key_count != 1
        || row.first_attnum != 1
        || !row.is_validated
        || row.is_deferrable
        || row.is_deferred
        || row.index_attribute_count != 1
        || row.index_key_count != 1
        || !row.is_unique
        || !row.is_primary
        || !row.is_valid
        || !row.is_ready
        || !row.is_immediate
        || !row.has_no_expressions
        || !row.has_no_predicate
        || row.index_namespace_oid != i64::from(context.public_schema_oid)
        || row.index_namespace != PUBLIC_SCHEMA
        || row.index_kind != "i"
        || row.index_persistence != "p"
        || index_owner_oid != context.migration_owner_oid
    {
        return Err(SimulationMigrationLedgerError::CatalogInvalid);
    }
    Ok(index_oid)
}

const PUBLIC_SCHEMA_PG: &str = "pg_catalog";

fn validate_raw_acls(
    rows: &[RawAclRow],
    context: ContextEvidence,
    ledger_oid: u32,
) -> Result<(), SimulationMigrationLedgerError> {
    let mut seen = HashSet::new();
    let mut worker_table_selects = 0usize;
    for row in rows {
        let key = (
            row.category,
            row.relation_oid,
            row.attnum,
            row.grantee_oid,
            row.grantor_oid,
            row.privilege_type.clone(),
            row.is_grantable,
        );
        if !seen.insert(key) {
            return Err(SimulationMigrationLedgerError::AccessRejected);
        }
        if row.grantee_oid != 0 && row.grantee_oid != i64::from(context.worker_oid) {
            return Err(SimulationMigrationLedgerError::CatalogInvalid);
        }
        let _ = positive_oid(row.grantor_oid)?;
        if row.is_grantable {
            return Err(SimulationMigrationLedgerError::AccessRejected);
        }
        match row.category {
            1 => {
                if row.relation_oid.is_some()
                    || row.attnum.is_some()
                    || !matches!(
                        row.privilege_type.as_str(),
                        "CONNECT" | "CREATE" | "TEMPORARY"
                    )
                {
                    return Err(SimulationMigrationLedgerError::CatalogInvalid);
                }
                if row.grantee_oid == i64::from(context.worker_oid)
                    && row.privilege_type == "CREATE"
                {
                    return Err(SimulationMigrationLedgerError::AccessRejected);
                }
            }
            2 => {
                if row.relation_oid.is_some()
                    || row.attnum.is_some()
                    || !matches!(row.privilege_type.as_str(), "USAGE" | "CREATE")
                {
                    return Err(SimulationMigrationLedgerError::CatalogInvalid);
                }
            }
            3 => {
                if row.relation_oid != Some(i64::from(ledger_oid)) || row.attnum.is_some() {
                    return Err(SimulationMigrationLedgerError::CatalogInvalid);
                }
                if row.grantee_oid == 0 {
                    return Err(SimulationMigrationLedgerError::AccessRejected);
                }
                if !matches!(
                    row.privilege_type.as_str(),
                    "SELECT"
                        | "INSERT"
                        | "UPDATE"
                        | "DELETE"
                        | "TRUNCATE"
                        | "REFERENCES"
                        | "TRIGGER"
                ) {
                    return Err(SimulationMigrationLedgerError::CatalogInvalid);
                }
                if row.privilege_type != "SELECT"
                    || row.grantor_oid != i64::from(context.migration_owner_oid)
                {
                    return Err(SimulationMigrationLedgerError::AccessRejected);
                }
                worker_table_selects = worker_table_selects
                    .checked_add(1)
                    .ok_or(SimulationMigrationLedgerError::LimitExceeded)?;
            }
            4 => {
                if row.relation_oid != Some(i64::from(ledger_oid))
                    || !matches!(
                        row.attnum,
                        Some(attnum) if (1..=6).contains(&attnum)
                    )
                    || !matches!(
                        row.privilege_type.as_str(),
                        "SELECT" | "INSERT" | "UPDATE" | "REFERENCES"
                    )
                {
                    return Err(SimulationMigrationLedgerError::CatalogInvalid);
                }
                return Err(SimulationMigrationLedgerError::AccessRejected);
            }
            _ => return Err(SimulationMigrationLedgerError::CatalogInvalid),
        }
    }
    if worker_table_selects != 1 {
        return Err(SimulationMigrationLedgerError::AccessRejected);
    }
    Ok(())
}

fn validate_ledger(
    rows: Vec<LedgerRow>,
    identity: LedgerIdentity,
    baseline: BaselineEvidence,
) -> Result<LedgerEvidence, SimulationMigrationLedgerError> {
    if rows
        .iter()
        .any(|row| row.version > REQUIRED_EXTENSION as i64)
    {
        return Err(SimulationMigrationLedgerError::UnknownBaselineVersion);
    }
    if rows.len() > LEDGER_ACCEPT_LIMIT {
        return Err(SimulationMigrationLedgerError::LedgerInvalid);
    }
    if rows.len() != EXPECTED_ROW_COUNT {
        return Err(SimulationMigrationLedgerError::MissingPredecessor);
    }

    let mut digest = Sha256::new();
    digest.update(b"lagrange/simulation-migration-observation/v1");
    digest.update([0u8]);
    digest.update(baseline.fingerprint);
    digest.update(identity.ledger_oid.to_be_bytes());
    digest.update((EXPECTED_ROW_COUNT as u64).to_be_bytes());

    for (index, row) in rows.iter().enumerate() {
        let expected_version = BASELINE[index].version;
        if row.version > REQUIRED_EXTENSION as i64 {
            return Err(SimulationMigrationLedgerError::UnknownBaselineVersion);
        }
        if row.version != expected_version {
            if row.version > expected_version {
                return Err(SimulationMigrationLedgerError::MissingPredecessor);
            }
            return Err(SimulationMigrationLedgerError::LedgerInvalid);
        }
        if row.ledger_oid != i64::from(identity.ledger_oid) {
            return Err(SimulationMigrationLedgerError::LedgerInvalid);
        }
        if !row.success {
            return Err(SimulationMigrationLedgerError::LedgerDirty);
        }
        if row.checksum_len != 48 || row.checksum.as_ref().is_none_or(|value| value.len() != 48) {
            return Err(SimulationMigrationLedgerError::ChecksumMismatch);
        }
        let checksum = row
            .checksum
            .as_ref()
            .ok_or(SimulationMigrationLedgerError::ChecksumMismatch)?;
        let (_, expected_checksum) =
            validate_expected_entry(&BASELINE[index], BASELINE[index].bytes)?;
        if checksum.as_slice() != &expected_checksum[..] {
            return Err(SimulationMigrationLedgerError::ChecksumMismatch);
        }
        digest.update(row.version.to_be_bytes());
        digest.update([1u8]);
        digest.update(expected_checksum);
    }

    Ok(LedgerEvidence {
        row_count: rows.len() as u64,
        digest_text: format!("sha256:{:x}", digest.finalize()),
    })
}

#[cfg(test)]
mod tests;
