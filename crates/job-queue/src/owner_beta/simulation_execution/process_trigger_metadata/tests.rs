// The fake's bounded poll loop is intentionally single step.
#![allow(clippy::never_loop)]

use std::future::Future;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::task::{Context, Poll, Waker};

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use super::*;

#[cfg(feature = "r4-pg-observer-tests")]
use super::super::pg_test_common;

// The DB harness enables this feature against its disposable, migrated PG.
// Ordinary unit runs remain independent of DATABASE_URL and a live database.
#[cfg(feature = "r4-pg-observer-tests")]
#[tokio::test]
async fn r4_trigger_metadata_pg_projection() {
    let db = pg_test_common::ScratchDb::create()
        .await
        .expect("disposable DATABASE_URL is required");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&db.role_url("worker"))
        .await
        .expect("connect directly as worker to disposable PostgreSQL");

    async fn public_relation_oid(pool: &PgPool, name: &str) -> u32 {
        let oid: i64 = sqlx::query_scalar(
            "SELECT relation.oid::pg_catalog.int8 FROM pg_catalog.pg_class AS relation \
             JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace \
             WHERE namespace.nspname = 'public' AND relation.relname = $1",
        )
        .bind(name)
        .fetch_one(pool)
        .await
        .expect("required public relation exists");
        u32::try_from(oid).expect("relation OID fits u32")
    }

    let relation_names = [
        "job_attempts",
        "jobs",
        "owner_beta_simulation_result_pairs",
        "owner_beta_simulation_runs",
    ];
    let mut relation_oids = [0_u32; 4];
    for (index, name) in relation_names.iter().enumerate() {
        relation_oids[index] = public_relation_oid(&pool, name).await;
    }
    let adjacent_relation_oid = public_relation_oid(&pool, "user_strategy_configs").await;
    let worker_oid: i64 = sqlx::query_scalar(
        "SELECT oid::pg_catalog.int8 FROM pg_catalog.pg_roles WHERE rolname = 'worker'",
    )
    .fetch_one(&pool)
    .await
    .expect("worker role exists");
    let mut root_oids = [0_u32; ROOT_HEADER_ACCEPT_LIMIT];
    for (index, spec) in ROOT_SPECS.iter().enumerate() {
        let oid: i64 = sqlx::query_scalar(
            "SELECT routine.oid::pg_catalog.int8 FROM pg_catalog.pg_proc AS routine \
             JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = routine.pronamespace \
             WHERE namespace.nspname = 'public' AND routine.proname = $1 \
             ORDER BY routine.oid LIMIT 1",
        )
        .bind(spec.name)
        .fetch_one(&pool)
        .await
        .expect("required trigger routine exists");
        root_oids[index] = u32::try_from(oid).expect("routine OID fits u32");
    }
    let trigger_ids: Vec<i64> = sqlx::query_scalar(
        "SELECT trigger.oid::pg_catalog.int8 FROM pg_catalog.pg_trigger AS trigger \
         JOIN pg_catalog.pg_class AS relation ON relation.oid = trigger.tgrelid \
         JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace \
         WHERE namespace.nspname = 'public' ORDER BY trigger.oid LIMIT 13",
    )
    .fetch_all(&pool)
    .await
    .expect("read public trigger OIDs");
    assert!(trigger_ids.len() >= USER_CONSTRAINT_ACCEPT_LIMIT);
    let mut constraint_trigger_oids = [0_u32; USER_CONSTRAINT_ACCEPT_LIMIT];
    for (index, oid) in trigger_ids.iter().enumerate() {
        constraint_trigger_oids[index] = u32::try_from(*oid).expect("trigger OID fits u32");
    }
    let update_trigger_oids = [constraint_trigger_oids[0], constraint_trigger_oids[1]];

    let mut transport = SqlxObserverTransport::new(&pool);
    transport
        .acquire()
        .await
        .expect("acquire observer connection");
    transport.arm_close_on_drop();
    assert!(
        transport
            .execute(Statement::Begin)
            .await
            .expect("BEGIN")
            .is_empty()
    );
    let statements = [
        Statement::Configure { timeout_ms: 30_000 },
        Statement::Context,
        Statement::Roles,
        Statement::WorkerMemberships {
            worker_oid: u32::try_from(worker_oid).expect("worker OID fits u32"),
        },
        Statement::Relations,
        Statement::PrimaryInheritance { relation_oids },
        Statement::RootHeaders,
        Statement::RootSettings { root_oids },
        Statement::ForwardTriggers {
            relation_oids,
            root_oids,
        },
        Statement::AdjacentConfigHook {
            relation_oid: adjacent_relation_oid,
        },
        Statement::ReverseProtectedHooks { root_oids },
        Statement::UpdateOfAttributes {
            trigger_oids: update_trigger_oids,
        },
        Statement::UserTriggerConstraints {
            trigger_oids: constraint_trigger_oids,
        },
        Statement::FinalContext,
    ];
    for statement in statements {
        let plan = statement_plan(statement);
        let rows = transport
            .execute(statement)
            .await
            .unwrap_or_else(|error| panic!("real SQLx projection {:?}: {:?}", plan.kind, error));
        // A correct catalog can have no worker memberships or inheritance.
        if !matches!(
            plan.kind,
            StatementKind::WorkerMemberships | StatementKind::PrimaryInheritance
        ) {
            assert!(!rows.is_empty(), "empty PG projection: {:?}", plan.kind);
        }
        assert!(
            rows.iter().all(|row| row.cells.len() == plan.shape.len()
                && !row
                    .cells
                    .iter()
                    .any(|cell| matches!(cell, RawCell::WrongType))),
            "wrong PG projection shape or type: {:?}",
            plan.kind
        );
    }
    assert!(
        transport
            .execute(Statement::Commit)
            .await
            .expect("COMMIT")
            .is_empty()
    );
    transport.close().await.expect("close observer connection");
    // Projection shape alone does not prove that the adopted catalog is
    // accepted. Exercise the complete observer against the migrated baseline.
    let canceled = AtomicBool::new(false);
    observe_simulation_trigger_metadata(&pool, &valid_config(), &canceled)
        .await
        .expect("complete migrated trigger baseline must be accepted");
    let migration_owner = PgPool::connect(&db.role_url("migration_owner"))
        .await
        .expect("connect as the disposable database routine owner");
    sqlx::query("ALTER FUNCTION public.assert_candidate_publication_settlement() SECURITY INVOKER")
        .execute(&migration_owner)
        .await
        .expect("mutate only this scratch database's trigger security contract");
    let rejected = observe_simulation_trigger_metadata(&pool, &valid_config(), &canceled).await;
    assert!(
        matches!(rejected, Err(error) if error.primary == PrimaryFailure::PolicyRejected),
        "migration63 requires SECURITY DEFINER"
    );
    migration_owner.close().await;
    pool.close().await;
    db.drop_db().await;
    eprintln!("R4_TRIGGER_PG:13_PROJECTIONS_AND_COMMANDS:PASS");
    eprintln!("R4_TRIGGER_PG:FULL_BASELINE_AND_SECURITY_MUTATION:PASS");
}

const VALID_CONFIG: &str = r##"{"schema":"owner-beta-simulation-process-v1","code_commit":"0123456789abcdef0123456789abcdef01234567","manifest_format":"LAGRANGE_RELEASE_MANIFEST_V3","manifest_sha256":"sha256:1111111111111111111111111111111111111111111111111111111111111111","image_id":"sha256:2222222222222222222222222222222222222222222222222222222222222222","receipt_sha256":"sha256:3333333333333333333333333333333333333333333333333333333333333333","binary_sha256":"sha256:4444444444444444444444444444444444444444444444444444444444444444","source_input_sha256":"sha256:5555555555555555555555555555555555555555555555555555555555555555","recipe_sha256":"sha256:6666666666666666666666666666666666666666666666666666666666666666","build_origin":"official-release-v1","package":"job-queue","bin":"owner-beta-simulation-runner","target":"x86_64-unknown-linux-musl","profile":"release","toolchain":"1.97.1","owner":"11111111-2222-3333-4444-555555555555","entitlement_reference":"fixture:synthetic-owner-rights-v1","entitlement_sha256":"sha256:7777777777777777777777777777777777777777777777777777777777777777","historical_root":"/synthetic/history/δ","reader_uid":10001,"reader_gid":10001,"paths":{"config":"/run/owner-beta-simulation/config.json","artifact_root":"/data/owner-beta-artifacts","receipt":"/usr/share/lagrange/owner-beta-simulation/artifact.json","complete":"/usr/share/lagrange/owner-beta-simulation/COMPLETE","executable":"/usr/local/bin/owner-beta-simulation-runner","worker_id":"owner-beta-simulation-runner"},"limits":{"memory_bytes":1234567,"nano_cpus":2345678,"health_tmpfs_bytes":3456789,"pids_limit":42,"startup_timeout_seconds":123,"health_max_age_seconds":234,"stop_timeout_seconds":345}}"##;
const TEST_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const TEST_CONFIG_TIMEOUT_MS: i64 = 123_000;
const TEST_WORKER_OID: u32 = 16_384;
const TEST_OWNER_OID: u32 = 16_385;
const TEST_DATABASE_OID: u32 = 16_390;
const TEST_DATABASE_OWNER_OID: u32 = 16_391;
const TEST_SCHEMA_OID: u32 = 2_200;
const TEST_SCHEMA_OWNER_OID: u32 = 16_392;
const TEST_PG_CATALOG_OID: u32 = 11;
const TEST_PLPGSQL_OID: u32 = 13_625;
const TEST_TRIGGER_TYPE_OID: u32 = 2_279;
const TEST_RELATION_OID_BASE: u32 = 52_100;
const TEST_ROOT_OID_BASE: u32 = 53_000;
const TEST_TRIGGER_OID_BASE: u32 = 54_000;
const TEST_CONSTRAINT_OID_BASE: u32 = 55_000;
const TEST_LEGACY_FUNCTION_OID_BASE: u32 = 56_000;

const EXPECTED_RELATION_NAMES: [&str; 10] = [
    "job_attempts",
    "jobs",
    "owner_beta_daily_publication_claims",
    "owner_beta_daily_reservations",
    "owner_beta_recommendation_items",
    "owner_beta_recommendation_runs",
    "owner_beta_simulation_result_pairs",
    "owner_beta_simulation_runs",
    "stock_analysis_runs",
    "user_strategy_configs",
];

const EXPECTED_ROOT_NAMES: [&str; 17] = [
    "assert_candidate_publication_settlement",
    "daily_0060_attempt_guard",
    "daily_0060_config_epoch",
    "daily_0060_consistency",
    "daily_0060_job_guard",
    "jobs_protect_owner_beta_recommendation_lineage",
    "jobs_reject_candidate_scheduled_mutation",
    "jobs_reject_scheduled_recommendation_mutation",
    "owner_beta_simulation_attempt_guard",
    "owner_beta_simulation_deferred_guard",
    "owner_beta_simulation_jobs_insert_guard",
    "owner_beta_simulation_jobs_lineage_guard",
    "owner_beta_simulation_jobs_validate_insert_payload",
    "owner_beta_simulation_pairs_validate_insert",
    "owner_beta_simulation_runs_lock_config_on_success",
    "owner_beta_simulation_runs_validate_binding",
    "sync_recommendation_run_from_terminal_job",
];

const EXPECTED_ATTACHMENT_KEYS: [(&str, &str, &str); 27] = [
    (
        "jobs",
        "jobs_protect_scheduled_recommendation_lineage",
        "jobs_reject_scheduled_recommendation_mutation",
    ),
    (
        "jobs",
        "jobs_sync_recommendation_terminal_run",
        "sync_recommendation_run_from_terminal_job",
    ),
    (
        "jobs",
        "jobs_protect_candidate_scheduled_lineage",
        "jobs_reject_candidate_scheduled_mutation",
    ),
    (
        "stock_analysis_runs",
        "stock_analysis_run_requires_settlement",
        "assert_candidate_publication_settlement",
    ),
    (
        "jobs",
        "candidate_job_requires_publication",
        "assert_candidate_publication_settlement",
    ),
    (
        "job_attempts",
        "candidate_attempt_requires_publication",
        "assert_candidate_publication_settlement",
    ),
    (
        "jobs",
        "jobs_protect_owner_beta_recommendation_lineage",
        "jobs_protect_owner_beta_recommendation_lineage",
    ),
    (
        "user_strategy_configs",
        "daily_0060_config_epoch",
        "daily_0060_config_epoch",
    ),
    ("jobs", "daily_0060_job_guard", "daily_0060_job_guard"),
    (
        "job_attempts",
        "daily_0060_attempt_guard",
        "daily_0060_attempt_guard",
    ),
    ("jobs", "daily_0060_consistency", "daily_0060_consistency"),
    (
        "job_attempts",
        "daily_0060_consistency",
        "daily_0060_consistency",
    ),
    (
        "owner_beta_recommendation_runs",
        "daily_0060_consistency",
        "daily_0060_consistency",
    ),
    (
        "owner_beta_recommendation_items",
        "daily_0060_consistency",
        "daily_0060_consistency",
    ),
    (
        "owner_beta_daily_reservations",
        "daily_0060_consistency",
        "daily_0060_consistency",
    ),
    (
        "owner_beta_daily_publication_claims",
        "daily_0060_consistency",
        "daily_0060_consistency",
    ),
    (
        "jobs",
        "owner_beta_simulation_jobs_insert_guard",
        "owner_beta_simulation_jobs_insert_guard",
    ),
    (
        "jobs",
        "owner_beta_simulation_jobs_lineage_guard",
        "owner_beta_simulation_jobs_lineage_guard",
    ),
    (
        "owner_beta_simulation_runs",
        "owner_beta_simulation_runs_validate_binding",
        "owner_beta_simulation_runs_validate_binding",
    ),
    (
        "owner_beta_simulation_runs",
        "owner_beta_simulation_runs_lock_config_on_success",
        "owner_beta_simulation_runs_lock_config_on_success",
    ),
    (
        "jobs",
        "owner_beta_simulation_jobs_validate_insert_payload",
        "owner_beta_simulation_jobs_validate_insert_payload",
    ),
    (
        "owner_beta_simulation_result_pairs",
        "owner_beta_simulation_pairs_validate_insert",
        "owner_beta_simulation_pairs_validate_insert",
    ),
    (
        "job_attempts",
        "owner_beta_simulation_attempt_guard",
        "owner_beta_simulation_attempt_guard",
    ),
    (
        "jobs",
        "owner_beta_simulation_jobs_deferred_guard",
        "owner_beta_simulation_deferred_guard",
    ),
    (
        "owner_beta_simulation_runs",
        "owner_beta_simulation_runs_deferred_guard",
        "owner_beta_simulation_deferred_guard",
    ),
    (
        "owner_beta_simulation_result_pairs",
        "owner_beta_simulation_pairs_deferred_guard",
        "owner_beta_simulation_deferred_guard",
    ),
    (
        "job_attempts",
        "owner_beta_simulation_attempts_deferred_guard",
        "owner_beta_simulation_deferred_guard",
    ),
];

const EXPECTED_EVENT_BITS: [i32; 27] = [
    31, 17, 31, 29, 29, 29, 27, 23, 31, 31, 29, 29, 29, 29, 29, 29, 7, 27, 31, 19, 7, 31, 31, 29,
    29, 29, 29,
];

const EXPECTED_ROOT_SETTINGS: [(&str, &str); 25] = [
    (
        "assert_candidate_publication_settlement",
        "search_path=pg_catalog",
    ),
    ("daily_0060_attempt_guard", "search_path=pg_catalog,pg_temp"),
    ("daily_0060_attempt_guard", "lock_timeout=5s"),
    ("daily_0060_attempt_guard", "statement_timeout=30s"),
    ("daily_0060_config_epoch", "search_path=pg_catalog,pg_temp"),
    ("daily_0060_config_epoch", "lock_timeout=5s"),
    ("daily_0060_config_epoch", "statement_timeout=30s"),
    ("daily_0060_consistency", "search_path=pg_catalog,pg_temp"),
    ("daily_0060_consistency", "lock_timeout=5s"),
    ("daily_0060_consistency", "statement_timeout=30s"),
    ("daily_0060_job_guard", "search_path=pg_catalog,pg_temp"),
    ("daily_0060_job_guard", "lock_timeout=5s"),
    ("daily_0060_job_guard", "statement_timeout=30s"),
    (
        "jobs_protect_owner_beta_recommendation_lineage",
        "search_path=pg_catalog",
    ),
    (
        "jobs_reject_candidate_scheduled_mutation",
        "search_path=pg_catalog",
    ),
    (
        "jobs_reject_scheduled_recommendation_mutation",
        "search_path=pg_catalog,public",
    ),
    (
        "owner_beta_simulation_attempt_guard",
        "search_path=pg_catalog,pg_temp",
    ),
    (
        "owner_beta_simulation_deferred_guard",
        "search_path=pg_catalog,pg_temp",
    ),
    (
        "owner_beta_simulation_jobs_insert_guard",
        "search_path=pg_catalog,pg_temp",
    ),
    (
        "owner_beta_simulation_jobs_lineage_guard",
        "search_path=pg_catalog,pg_temp",
    ),
    (
        "owner_beta_simulation_jobs_validate_insert_payload",
        "search_path=pg_catalog,pg_temp",
    ),
    (
        "owner_beta_simulation_pairs_validate_insert",
        "search_path=pg_catalog,pg_temp",
    ),
    (
        "owner_beta_simulation_runs_lock_config_on_success",
        "search_path=pg_catalog,pg_temp",
    ),
    (
        "owner_beta_simulation_runs_validate_binding",
        "search_path=pg_catalog,pg_temp",
    ),
    (
        "sync_recommendation_run_from_terminal_job",
        "search_path=pg_catalog,pg_temp",
    ),
];

fn valid_config() -> SimulationProcessConfig {
    let hash = Sha256::digest(VALID_CONFIG.as_bytes());
    let expected = format!("sha256:{hash:x}");
    SimulationProcessConfig::from_json(VALID_CONFIG.as_bytes(), TEST_COMMIT, &expected)
        .expect("synthetic checked config")
}

fn text(value: &str) -> RawCell {
    RawCell::Text {
        is_null: false,
        original_bytes: Some(i64::try_from(value.len()).expect("fixture text length")),
        value: value.as_bytes().to_vec(),
    }
}

fn null_text() -> RawCell {
    RawCell::Text {
        is_null: true,
        original_bytes: None,
        value: Vec::new(),
    }
}

fn i64_cell(value: i64) -> RawCell {
    RawCell::I64(value)
}

fn i32_cell(value: i32) -> RawCell {
    RawCell::I32(value)
}

fn bool_cell(value: bool) -> RawCell {
    RawCell::Bool(value)
}

fn row(category: RawCategory, cells: Vec<RawCell>) -> RawRow {
    RawRow::new(category, cells)
}

fn relation_oid(name: &str) -> u32 {
    RELATION_SPECS
        .iter()
        .position(|spec| spec.name == name)
        .map(|index| TEST_RELATION_OID_BASE + index as u32)
        .expect("known relation fixture")
}

fn root_oid(name: &str) -> Option<u32> {
    ROOT_SPECS
        .iter()
        .position(|spec| spec.name == name)
        .map(|index| TEST_ROOT_OID_BASE + index as u32)
}

fn context_row() -> RawRow {
    row(
        RawCategory::Context,
        vec![
            text("worker"),
            text("worker"),
            text("simulation_test"),
            text("simulation_test"),
            text("public"),
            i64_cell(i64::from(TEST_WORKER_OID)),
            i64_cell(i64::from(TEST_DATABASE_OID)),
            i64_cell(i64::from(TEST_SCHEMA_OID)),
            i64_cell(i64::from(TEST_OWNER_OID)),
            i64_cell(i64::from(TEST_DATABASE_OWNER_OID)),
            i64_cell(i64::from(TEST_SCHEMA_OWNER_OID)),
            bool_cell(true),
            bool_cell(false),
            bool_cell(false),
            bool_cell(false),
            bool_cell(false),
            bool_cell(false),
            bool_cell(false),
            text("on"),
            text("origin"),
            bool_cell(true),
            text("repeatable read"),
            text("pg_catalog"),
            text("123000ms"),
            text("123000ms"),
        ],
    )
}

fn role_rows() -> Vec<RawRow> {
    [
        ("worker", TEST_WORKER_OID),
        ("migration_owner", TEST_OWNER_OID),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (name, oid))| {
        row(
            RawCategory::Role,
            vec![
                i32_cell((index + 1) as i32),
                text(name),
                text(name),
                i64_cell(i64::from(oid)),
            ],
        )
    })
    .collect()
}

fn relation_rows() -> Vec<RawRow> {
    RELATION_SPECS
        .iter()
        .enumerate()
        .map(|(index, spec)| {
            row(
                RawCategory::Relation,
                vec![
                    i32_cell((index + 1) as i32),
                    text(spec.name),
                    i64_cell(i64::from(TEST_RELATION_OID_BASE + index as u32)),
                    i64_cell(i64::from(TEST_SCHEMA_OID)),
                    text(PUBLIC_SCHEMA),
                    text(spec.name),
                    i64_cell(i64::from(TEST_OWNER_OID)),
                    text(MIGRATION_OWNER_ROLE),
                    text("r"),
                    text("p"),
                    bool_cell(false),
                ],
            )
        })
        .collect()
}

fn root_header_rows() -> Vec<RawRow> {
    ROOT_SPECS
        .iter()
        .enumerate()
        .map(|(index, spec)| {
            let config_count = if spec.daily_timeouts { 3 } else { 1 };
            row(
                RawCategory::RootHeader,
                vec![
                    i32_cell((index + 1) as i32),
                    text(spec.name),
                    i64_cell(i64::from(TEST_ROOT_OID_BASE + index as u32)),
                    i64_cell(i64::from(TEST_SCHEMA_OID)),
                    text(PUBLIC_SCHEMA),
                    text(spec.name),
                    i64_cell(i64::from(TEST_OWNER_OID)),
                    text(MIGRATION_OWNER_ROLE),
                    text("f"),
                    i32_cell(0),
                    i32_cell(0),
                    i32_cell(0),
                    i64_cell(0),
                    bool_cell(false),
                    bool_cell(spec.security_definer),
                    bool_cell(true),
                    bool_cell(true),
                    bool_cell(true),
                    bool_cell(true),
                    i64_cell(i64::from(TEST_PLPGSQL_OID)),
                    text("plpgsql"),
                    i64_cell(i64::from(TEST_TRIGGER_TYPE_OID)),
                    i64_cell(i64::from(TEST_PG_CATALOG_OID)),
                    text(PG_CATALOG_SCHEMA),
                    text("trigger"),
                    bool_cell(false),
                    i32_cell(1),
                    i32_cell(1),
                    i32_cell(config_count),
                ],
            )
        })
        .collect()
}

fn root_setting_rows() -> Vec<RawRow> {
    let mut values = Vec::with_capacity(ROOT_SETTING_ACCEPT_LIMIT);
    for (index, spec) in ROOT_SPECS.iter().enumerate() {
        let oid = TEST_ROOT_OID_BASE + index as u32;
        let mut settings = vec![format!("search_path={}", spec.search_path)];
        if spec.daily_timeouts {
            settings.push("lock_timeout=5s".to_owned());
            settings.push("statement_timeout=30s".to_owned());
        }
        for (ordinal, value) in settings.into_iter().enumerate() {
            values.push((
                oid,
                row(
                    RawCategory::RootSetting,
                    vec![
                        i64_cell(i64::from(oid)),
                        i32_cell((ordinal + 1) as i32),
                        text(&value),
                    ],
                ),
            ));
        }
    }
    values.sort_by_key(|(oid, row)| {
        (
            *oid,
            match &row.cells[1] {
                RawCell::I32(value) => *value,
                _ => 0,
            },
        )
    });
    values.into_iter().map(|(_, row)| row).collect()
}

fn trigger_rows(category: RawCategory, specs: &[AttachmentSpec]) -> Vec<RawRow> {
    let mut rows = specs
        .iter()
        .map(|spec| {
            let index = ATTACHMENT_SPECS
                .iter()
                .position(|candidate| {
                    candidate.relation == spec.relation
                        && candidate.name == spec.name
                        && candidate.function == spec.function
                })
                .expect("trigger key in independent fixture");
            let trigger_oid = TEST_TRIGGER_OID_BASE + index as u32;
            let function_oid =
                root_oid(spec.function).unwrap_or(TEST_LEGACY_FUNCTION_OID_BASE + index as u32);
            let update = spec.update_of_status;
            let constraint_oid = if spec.constraint {
                TEST_CONSTRAINT_OID_BASE + index as u32
            } else {
                0
            };
            row(
                category,
                vec![
                    i64_cell(i64::from(trigger_oid)),
                    i64_cell(i64::from(relation_oid(spec.relation))),
                    i64_cell(i64::from(TEST_SCHEMA_OID)),
                    text(PUBLIC_SCHEMA),
                    text(spec.relation),
                    i64_cell(i64::from(TEST_OWNER_OID)),
                    text(MIGRATION_OWNER_ROLE),
                    text(spec.name),
                    i64_cell(i64::from(function_oid)),
                    i64_cell(i64::from(TEST_SCHEMA_OID)),
                    text(PUBLIC_SCHEMA),
                    text(spec.function),
                    i64_cell(i64::from(TEST_OWNER_OID)),
                    text(MIGRATION_OWNER_ROLE),
                    bool_cell(false),
                    text("O"),
                    i64_cell(0),
                    i32_cell(spec.event_bits),
                    i32_cell(0),
                    i32_cell(1),
                    i32_cell(0),
                    i32_cell(if update { 0 } else { -1 }),
                    if update { i32_cell(1) } else { i32_cell(0) },
                    RawCell::Bytes {
                        is_null: false,
                        original_bytes: Some(0),
                        prefix: Vec::new(),
                    },
                    bool_cell(true),
                    i64_cell(0),
                    i64_cell(i64::from(constraint_oid)),
                    i32_cell(if update { 1 } else { 0 }),
                    bool_cell(true),
                    bool_cell(true),
                    bool_cell(spec.constraint),
                    bool_cell(spec.constraint),
                ],
            )
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| {
        let relation = match &row.cells[1] {
            RawCell::I64(value) => *value,
            _ => 0,
        };
        let name = match &row.cells[7] {
            RawCell::Text { value, .. } => String::from_utf8_lossy(value).into_owned(),
            _ => String::new(),
        };
        let oid = match &row.cells[0] {
            RawCell::I64(value) => *value,
            _ => 0,
        };
        (relation, name, oid)
    });
    rows
}

fn primary_specs() -> Vec<AttachmentSpec> {
    ATTACHMENT_SPECS
        .iter()
        .copied()
        .filter(|spec| {
            RELATION_SPECS
                .iter()
                .any(|relation| relation.name == spec.relation && relation.primary)
        })
        .collect()
}

fn adjacent_specs() -> Vec<AttachmentSpec> {
    ATTACHMENT_SPECS
        .iter()
        .copied()
        .filter(|spec| {
            RELATION_SPECS
                .iter()
                .any(|relation| relation.name == spec.relation && relation.adjacent_config)
        })
        .collect()
}

fn update_attribute_rows() -> Vec<RawRow> {
    let mut rows = ATTACHMENT_SPECS
        .iter()
        .enumerate()
        .filter(|(_, spec)| spec.update_of_status)
        .map(|(index, spec)| {
            let trigger_oid = TEST_TRIGGER_OID_BASE + index as u32;
            let relation = relation_oid(spec.relation);
            row(
                RawCategory::UpdateAttribute,
                vec![
                    i64_cell(i64::from(trigger_oid)),
                    i64_cell(i64::from(relation)),
                    i64_cell(1),
                    i32_cell(4),
                    text("status"),
                    i64_cell(i64::from(relation)),
                    bool_cell(false),
                ],
            )
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| match &row.cells[0] {
        RawCell::I64(value) => *value,
        _ => 0,
    });
    rows
}

fn user_constraint_rows() -> Vec<RawRow> {
    let mut rows = ATTACHMENT_SPECS
        .iter()
        .enumerate()
        .filter(|(_, spec)| spec.constraint)
        .map(|(index, spec)| {
            let trigger_oid = TEST_TRIGGER_OID_BASE + index as u32;
            let relation = relation_oid(spec.relation);
            let constraint = TEST_CONSTRAINT_OID_BASE + index as u32;
            row(
                RawCategory::UserConstraint,
                vec![
                    i64_cell(i64::from(trigger_oid)),
                    i64_cell(i64::from(relation)),
                    i64_cell(i64::from(constraint)),
                    i64_cell(i64::from(constraint)),
                    i64_cell(i64::from(relation)),
                    i64_cell(i64::from(TEST_SCHEMA_OID)),
                    text(PUBLIC_SCHEMA),
                    text("t"),
                    bool_cell(true),
                    bool_cell(true),
                    i64_cell(0),
                    i64_cell(0),
                    i64_cell(0),
                ],
            )
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| match &row.cells[0] {
        RawCell::I64(value) => *value,
        _ => 0,
    });
    rows
}

#[derive(Clone)]
struct Fixture {
    setup: Vec<RawRow>,
    context: Vec<RawRow>,
    final_context: Vec<RawRow>,
    roles: Vec<RawRow>,
    memberships: Vec<RawRow>,
    relations: Vec<RawRow>,
    inheritance: Vec<RawRow>,
    root_headers: Vec<RawRow>,
    root_settings: Vec<RawRow>,
    forward: Vec<RawRow>,
    adjacent: Vec<RawRow>,
    reverse: Vec<RawRow>,
    update_attributes: Vec<RawRow>,
    constraints: Vec<RawRow>,
}

impl Fixture {
    fn valid() -> Self {
        Self {
            setup: vec![row(
                RawCategory::Setup,
                vec![text("pg_catalog"), text("123000ms"), text("123000ms")],
            )],
            context: vec![context_row()],
            final_context: vec![context_row()],
            roles: role_rows(),
            memberships: Vec::new(),
            relations: relation_rows(),
            inheritance: Vec::new(),
            root_headers: root_header_rows(),
            root_settings: root_setting_rows(),
            forward: trigger_rows(RawCategory::ForwardTrigger, &primary_specs()),
            adjacent: trigger_rows(RawCategory::AdjacentHook, &adjacent_specs()),
            reverse: trigger_rows(RawCategory::ReverseHook, &ATTACHMENT_SPECS),
            update_attributes: update_attribute_rows(),
            constraints: user_constraint_rows(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Point {
    Acquire,
    Arm,
    Statement(StatementKind),
    Close,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Event {
    Acquire,
    Armed,
    Statement(Statement),
    Close,
    PendingDroppedAfterArm(bool),
}

struct PendingDrop {
    events: Arc<Mutex<Vec<Event>>>,
    armed: bool,
}

impl Drop for PendingDrop {
    fn drop(&mut self) {
        self.events
            .lock()
            .expect("event log")
            .push(Event::PendingDroppedAfterArm(self.armed));
    }
}

struct FakeTransport {
    fixture: Fixture,
    events: Arc<Mutex<Vec<Event>>>,
    canceled: Arc<AtomicBool>,
    armed: bool,
    context_reads: usize,
    fail_at: Option<Point>,
    failure: TransportFailure,
    cancel_at: Option<Point>,
    pending_at: Option<StatementKind>,
    pending_acquire: bool,
    unexpected_at: Option<StatementKind>,
    rollback_fails: bool,
    close_fails: bool,
    begin_nonempty: bool,
    commit_nonempty: bool,
}

impl FakeTransport {
    fn valid() -> Self {
        Self {
            fixture: Fixture::valid(),
            events: Arc::new(Mutex::new(Vec::new())),
            canceled: Arc::new(AtomicBool::new(false)),
            armed: false,
            context_reads: 0,
            fail_at: None,
            failure: TransportFailure::Database,
            cancel_at: None,
            pending_at: None,
            pending_acquire: false,
            unexpected_at: None,
            rollback_fails: false,
            close_fails: false,
            begin_nonempty: false,
            commit_nonempty: false,
        }
    }

    fn event_log(&self) -> Vec<Event> {
        self.events.lock().expect("event log").clone()
    }

    fn cancel(&self, point: Point) {
        if self.cancel_at == Some(point) {
            self.canceled.store(true, Ordering::Release);
        }
    }

    fn failure_at(&mut self, point: Point, failure: TransportFailure) {
        self.fail_at = Some(point);
        self.failure = failure;
    }

    fn rows(&mut self, statement: Statement) -> Result<Vec<RawRow>, TransportFailure> {
        if matches!(
            statement,
            Statement::Begin | Statement::Commit | Statement::Rollback
        ) {
            let plan = statement_plan(statement);
            assert_eq!(plan.kind, statement.kind());
            assert!(plan.binds.is_empty() && plan.category.is_none() && plan.shape.is_empty());
            assert_eq!(plan.fetch_limit, 1);
            let unexpected = self.unexpected_at == Some(statement.kind())
                || (statement == Statement::Begin && self.begin_nonempty)
                || (statement == Statement::Commit && self.commit_nonempty);
            return Ok(super::classify_command_result(unexpected.then_some(())));
        }
        if self.unexpected_at == Some(statement.kind()) {
            return Ok(vec![row(RawCategory::Context, vec![RawCell::WrongType])]);
        }
        Ok(match statement {
            Statement::Begin | Statement::Commit | Statement::Rollback => Vec::new(),
            Statement::Configure { .. } => self.fixture.setup.clone(),
            Statement::Context => {
                self.context_reads += 1;
                self.fixture.context.clone()
            }
            Statement::Roles => self.fixture.roles.clone(),
            Statement::WorkerMemberships { .. } => self.fixture.memberships.clone(),
            Statement::Relations => self.fixture.relations.clone(),
            Statement::PrimaryInheritance { .. } => self.fixture.inheritance.clone(),
            Statement::RootHeaders => self.fixture.root_headers.clone(),
            Statement::RootSettings { .. } => self.fixture.root_settings.clone(),
            Statement::ForwardTriggers { .. } => self.fixture.forward.clone(),
            Statement::AdjacentConfigHook { .. } => self.fixture.adjacent.clone(),
            Statement::ReverseProtectedHooks { .. } => self.fixture.reverse.clone(),
            Statement::UpdateOfAttributes { .. } => self.fixture.update_attributes.clone(),
            Statement::UserTriggerConstraints { .. } => self.fixture.constraints.clone(),
            Statement::FinalContext => self.fixture.final_context.clone(),
        })
    }
}

#[async_trait]
impl ObserverTransport for FakeTransport {
    async fn acquire(&mut self) -> Result<(), TransportFailure> {
        self.events.lock().expect("event log").push(Event::Acquire);
        self.cancel(Point::Acquire);
        if self.pending_acquire {
            let _drop = PendingDrop {
                events: Arc::clone(&self.events),
                armed: self.armed,
            };
            std::future::pending::<()>().await;
        }
        if self.fail_at == Some(Point::Acquire) {
            Err(self.failure)
        } else {
            Ok(())
        }
    }

    fn arm_close_on_drop(&mut self) {
        self.armed = true;
        self.events.lock().expect("event log").push(Event::Armed);
        self.cancel(Point::Arm);
    }

    async fn execute(&mut self, statement: Statement) -> Result<Vec<RawRow>, TransportFailure> {
        let kind = statement.kind();
        self.events
            .lock()
            .expect("event log")
            .push(Event::Statement(statement));
        self.cancel(Point::Statement(kind));
        if self.pending_at == Some(kind) {
            let _drop = PendingDrop {
                events: Arc::clone(&self.events),
                armed: self.armed,
            };
            std::future::pending::<()>().await;
        }
        if self.fail_at == Some(Point::Statement(kind))
            || (kind == StatementKind::Rollback && self.rollback_fails)
        {
            return Err(self.failure);
        }
        self.rows(statement)
    }

    async fn close(&mut self) -> Result<(), TransportFailure> {
        self.events.lock().expect("event log").push(Event::Close);
        self.cancel(Point::Close);
        if self.close_fails || self.fail_at == Some(Point::Close) {
            Err(self.failure)
        } else {
            Ok(())
        }
    }
}

fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = Box::pin(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => panic!("fixture unexpectedly yielded"),
        }
    }
}

fn observe(
    fake: &mut FakeTransport,
) -> Result<SimulationTriggerMetadataObservation, SimulationTriggerMetadataError> {
    let config = valid_config();
    let canceled = Arc::clone(&fake.canceled);
    block_on(observe_with_transport(fake, &config, &canceled))
}

fn observe_error(fake: &mut FakeTransport) -> SimulationTriggerMetadataError {
    match observe(fake) {
        Ok(_) => panic!("unexpected trigger metadata observation"),
        Err(error) => error,
    }
}

fn poll_pending_then_drop(fake: &mut FakeTransport) {
    let config = valid_config();
    let canceled = Arc::clone(&fake.canceled);
    let mut future = Box::pin(observe_with_transport(fake, &config, &canceled));
    let mut context = Context::from_waker(Waker::noop());
    assert!(matches!(future.as_mut().poll(&mut context), Poll::Pending));
    drop(future);
}

fn text_at(row: &RawRow, index: usize) -> &str {
    match &row.cells[index] {
        RawCell::Text { value, .. } => std::str::from_utf8(value).expect("fixture UTF-8 text"),
        _ => panic!("fixture text cell"),
    }
}

fn set_text(row: &mut RawRow, index: usize, value: &str) {
    row.cells[index] = text(value);
}

fn set_i64(row: &mut RawRow, index: usize, value: i64) {
    row.cells[index] = i64_cell(value);
}

fn set_i32(row: &mut RawRow, index: usize, value: i32) {
    row.cells[index] = i32_cell(value);
}

fn set_bool(row: &mut RawRow, index: usize, value: bool) {
    row.cells[index] = bool_cell(value);
}

fn trigger_index(rows: &[RawRow], relation: &str, name: &str) -> usize {
    rows.iter()
        .position(|row| text_at(row, 4) == relation && text_at(row, 7) == name)
        .expect("fixture trigger")
}

fn event_kinds(fake: &FakeTransport) -> Vec<Event> {
    fake.event_log()
}

#[test]
fn adopted_candidate_names_settings_and_counts_are_independent_literals() {
    assert_eq!(
        RELATION_SPECS.map(|spec| spec.name),
        EXPECTED_RELATION_NAMES
    );
    let roots = ROOT_SPECS.map(|spec| spec.name);
    assert_eq!(roots, EXPECTED_ROOT_NAMES);

    let attachments = ATTACHMENT_SPECS.map(|spec| (spec.relation, spec.name, spec.function));
    assert_eq!(attachments, EXPECTED_ATTACHMENT_KEYS);
    assert_eq!(
        ATTACHMENT_SPECS.map(|spec| spec.event_bits),
        EXPECTED_EVENT_BITS
    );
    assert_eq!(PROTECTED_TRIGGER_NAMES.len(), 22);
    assert_eq!(ATTACHMENT_SPECS.len(), 27);
    assert_eq!(ROOT_SPECS.len(), 17);
    assert_eq!(RELATION_SPECS.len(), 10);
    assert_eq!(ROOT_SETTING_ACCEPT_LIMIT, 25);
    assert_eq!(
        ATTACHMENT_SPECS
            .iter()
            .filter(|spec| spec.update_of_status)
            .count(),
        2
    );
    assert_eq!(
        ATTACHMENT_SPECS
            .iter()
            .filter(|spec| spec.constraint)
            .count(),
        13
    );
    let update_indices = ATTACHMENT_SPECS
        .iter()
        .enumerate()
        .filter_map(|(index, spec)| spec.update_of_status.then_some(index))
        .collect::<Vec<_>>();
    assert_eq!(update_indices, [1, 19]);
    let constraint_indices = ATTACHMENT_SPECS
        .iter()
        .enumerate()
        .filter_map(|(index, spec)| spec.constraint.then_some(index))
        .collect::<Vec<_>>();
    assert_eq!(
        constraint_indices,
        [3, 4, 5, 10, 11, 12, 13, 14, 15, 23, 24, 25, 26]
    );

    let mut settings = Vec::new();
    for spec in ROOT_SPECS {
        settings.push((spec.name, format!("search_path={}", spec.search_path)));
        if spec.daily_timeouts {
            settings.push((spec.name, "lock_timeout=5s".to_owned()));
            settings.push((spec.name, "statement_timeout=30s".to_owned()));
        }
    }
    let expected = EXPECTED_ROOT_SETTINGS.map(|(name, value)| (name, value.to_owned()));
    assert_eq!(settings.len(), 25);
    assert_eq!(settings.as_slice(), expected.as_slice());

    let mut protected = ATTACHMENT_SPECS
        .iter()
        .map(|spec| spec.name)
        .collect::<Vec<_>>();
    protected.sort_unstable();
    protected.dedup();
    let mut pinned = PROTECTED_TRIGGER_NAMES.to_vec();
    pinned.sort_unstable();
    pinned.dedup();
    assert_eq!(protected, pinned);
}

fn projection_signature(shape: &[PgCellKind]) -> Vec<(String, &'static str)> {
    let mut columns = Vec::new();
    for cell in shape {
        match cell {
            PgCellKind::Bool(alias) | PgCellKind::OptionalBool(alias) => {
                columns.push(((*alias).to_owned(), "BOOL"));
            }
            PgCellKind::I32(alias) | PgCellKind::OptionalI32(alias) => {
                columns.push(((*alias).to_owned(), "INT4"));
            }
            PgCellKind::I64(alias) | PgCellKind::OptionalI64(alias) => {
                columns.push(((*alias).to_owned(), "INT8"));
            }
            PgCellKind::Text {
                null_alias,
                bytes_alias,
                value_alias,
            } => {
                columns.push(((*null_alias).to_owned(), "BOOL"));
                columns.push(((*bytes_alias).to_owned(), "INT8"));
                columns.push(((*value_alias).to_owned(), "TEXT"));
            }
            PgCellKind::Bytes {
                null_alias,
                bytes_alias,
                value_alias,
            } => {
                columns.push(((*null_alias).to_owned(), "BOOL"));
                columns.push(((*bytes_alias).to_owned(), "INT8"));
                columns.push(((*value_alias).to_owned(), "BYTEA"));
            }
        }
    }
    columns
}

fn assert_projection(shape: &[PgCellKind], expected: &str) {
    let actual = projection_signature(shape)
        .into_iter()
        .map(|(alias, type_name)| format!("{alias}:{type_name}"))
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(actual, expected);
}

fn is_sql_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$'
}

fn sql_word_at(bytes: &[u8], at: usize, word: &str) -> bool {
    let end = at.saturating_add(word.len());
    end <= bytes.len()
        && bytes[at..end].eq_ignore_ascii_case(word.as_bytes())
        && (at == 0 || !is_sql_word_byte(bytes[at - 1]))
        && (end == bytes.len() || !is_sql_word_byte(bytes[end]))
}

fn top_level_keyword(sql: &str, keyword: &str, from: usize) -> Option<(usize, usize)> {
    let bytes = sql.as_bytes();
    let mut depth = 0usize;
    let mut quote = None;
    let mut at = 0usize;
    while at < bytes.len() {
        if let Some(delimiter) = quote {
            if bytes[at] == delimiter {
                if bytes.get(at + 1) == Some(&delimiter) {
                    at += 2;
                    continue;
                }
                quote = None;
            }
            at += 1;
            continue;
        }
        match bytes[at] {
            b'\'' | b'"' => quote = Some(bytes[at]),
            b'(' => depth = depth.checked_add(1)?,
            b')' => {
                depth = depth.checked_sub(1)?;
            }
            _ if depth == 0 && at >= from && sql_word_at(bytes, at, keyword) => {
                return Some((at, at + keyword.len()));
            }
            _ => {}
        }
        at += 1;
    }
    None
}

fn split_top_level_projection(projection: &str) -> Option<Vec<&str>> {
    let bytes = projection.as_bytes();
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut quote = None;
    let mut start = 0usize;
    let mut at = 0usize;
    while at < bytes.len() {
        if let Some(delimiter) = quote {
            if bytes[at] == delimiter {
                if bytes.get(at + 1) == Some(&delimiter) {
                    at += 2;
                    continue;
                }
                quote = None;
            }
            at += 1;
            continue;
        }
        match bytes[at] {
            b'\'' | b'"' => quote = Some(bytes[at]),
            b'(' => depth = depth.checked_add(1)?,
            b')' => depth = depth.checked_sub(1)?,
            b',' if depth == 0 => {
                let part = projection[start..at].trim();
                if part.is_empty() {
                    return None;
                }
                parts.push(part);
                start = at + 1;
            }
            _ => {}
        }
        at += 1;
    }
    if quote.is_some() || depth != 0 {
        return None;
    }
    let part = projection[start..].trim();
    if part.is_empty() {
        return None;
    }
    parts.push(part);
    Some(parts)
}

fn parse_projection_item(item: &str) -> Option<(String, String)> {
    let (alias_at, alias_end) = top_level_keyword(item, "AS", 0)?;
    let expression = item[..alias_at].trim();
    let alias = item[alias_end..].trim();
    if alias.is_empty() || !alias.bytes().all(is_sql_word_byte) {
        return None;
    }
    let cast_at = expression.rfind("::pg_catalog.")?;
    let type_name = expression[cast_at + "::pg_catalog.".len()..].trim();
    if type_name.is_empty() || !type_name.bytes().all(is_sql_word_byte) {
        return None;
    }
    Some((alias.to_owned(), type_name.to_ascii_uppercase()))
}

fn parse_sql_projection(sql: &str) -> Option<Vec<(String, String)>> {
    let (_, select_end) = top_level_keyword(sql, "SELECT", 0)?;
    let (from_at, _) = top_level_keyword(sql, "FROM", select_end)?;
    split_top_level_projection(&sql[select_end..from_at])?
        .into_iter()
        .map(parse_projection_item)
        .collect()
}

fn assert_failure_rollback_close_order(fake: &FakeTransport, failure: StatementKind) {
    let events = fake.event_log();
    let failed_statement = events
        .iter()
        .position(
            |event| matches!(event, Event::Statement(statement) if statement.kind() == failure),
        )
        .expect("failed statement event");
    let rollback = events
        .iter()
        .position(|event| *event == Event::Statement(Statement::Rollback))
        .expect("awaited rollback event");
    let close = events
        .iter()
        .position(|event| *event == Event::Close)
        .expect("awaited close event");
    assert!(
        failed_statement < rollback && rollback < close,
        "failure, awaited rollback, and awaited close order"
    );
}

fn expected_sql_projection(shape: &[PgCellKind]) -> Vec<(String, String)> {
    projection_signature(shape)
        .into_iter()
        .map(|(alias, type_name)| (alias, type_name.to_owned()))
        .collect()
}

#[test]
fn real_statement_sql_projections_match_the_shared_raw_decoder_shapes() {
    let statements: [(&str, &[PgCellKind]); 14] = [
        (SETUP_SQL, &SETUP_ROW),
        (CONTEXT_SQL, &CONTEXT_ROW),
        (ROLES_SQL, &ROLE_ROW),
        (WORKER_MEMBERSHIPS_SQL, &MEMBERSHIP_ROW),
        (RELATIONS_SQL, &RELATION_ROW),
        (PRIMARY_INHERITANCE_SQL, &INHERITANCE_ROW),
        (ROOT_HEADERS_SQL, &ROOT_HEADER_ROW),
        (ROOT_SETTINGS_SQL, &ROOT_SETTING_ROW),
        (FORWARD_TRIGGERS_SQL, &TRIGGER_ROW),
        (ADJACENT_CONFIG_HOOK_SQL, &TRIGGER_ROW),
        (REVERSE_PROTECTED_HOOKS_SQL, &TRIGGER_ROW),
        (UPDATE_OF_ATTRIBUTES_SQL, &UPDATE_ATTRIBUTE_ROW),
        (USER_TRIGGER_CONSTRAINTS_SQL, &USER_CONSTRAINT_ROW),
        (CONTEXT_SQL, &CONTEXT_ROW),
    ];
    for (sql, shape) in statements {
        assert_eq!(
            parse_sql_projection(sql),
            Some(expected_sql_projection(shape)),
            "SQL projection does not match its raw decoder shape"
        );
    }
}

#[test]
fn bounded_projection_parser_rejects_omitted_reordered_extra_type_and_alias_drift() {
    let expected = vec![
        ("first_oid".to_owned(), "INT8".to_owned()),
        ("enabled".to_owned(), "BOOL".to_owned()),
    ];
    let invalid = [
        "SELECT 1::pg_catalog.int8 AS first_oid FROM source",
        "SELECT true::pg_catalog.bool AS enabled, 1::pg_catalog.int8 AS first_oid FROM source",
        "SELECT 1::pg_catalog.int8 AS first_oid, true::pg_catalog.bool AS enabled, 2::pg_catalog.int8 AS extra FROM source",
        "SELECT 1::pg_catalog.int4 AS first_oid, true::pg_catalog.bool AS enabled FROM source",
        "SELECT 1::pg_catalog.int8 AS other_oid, true::pg_catalog.bool AS enabled FROM source",
    ];
    for sql in invalid {
        assert_ne!(parse_sql_projection(sql), Some(expected.clone()));
    }
}

#[test]
fn sqlx_projection_alias_order_and_primitive_types_are_fixed() {
    assert_projection(
        &SETUP_ROW,
        "search_path_is_null:BOOL search_path_bytes:INT8 search_path:TEXT statement_timeout_is_null:BOOL statement_timeout_bytes:INT8 statement_timeout:TEXT lock_timeout_is_null:BOOL lock_timeout_bytes:INT8 lock_timeout:TEXT",
    );
    assert_projection(
        &ROLE_ROW,
        "role_ordinal:INT4 expected_name_is_null:BOOL expected_name_bytes:INT8 expected_name:TEXT actual_name_is_null:BOOL actual_name_bytes:INT8 actual_name:TEXT role_oid:INT8",
    );
    assert_projection(
        &MEMBERSHIP_ROW,
        "member_oid:INT8 role_oid:INT8 grantor_oid:INT8 admin_option:BOOL inherit_option:BOOL set_option:BOOL",
    );
    assert_projection(&INHERITANCE_ROW, "child_oid:INT8 parent_oid:INT8");
    assert_projection(
        &ROOT_SETTING_ROW,
        "routine_oid:INT8 setting_ordinal:INT4 setting_is_null:BOOL setting_bytes:INT8 setting_value:TEXT",
    );
    assert_projection(
        &UPDATE_ATTRIBUTE_ROW,
        "trigger_oid:INT8 relation_oid:INT8 attribute_ordinal:INT8 attribute_number:INT4 attribute_name_is_null:BOOL attribute_name_bytes:INT8 attribute_name:TEXT attribute_relation_oid:INT8 attribute_is_dropped:BOOL",
    );
    assert_projection(
        &USER_CONSTRAINT_ROW,
        "trigger_oid:INT8 relation_oid:INT8 trigger_constraint_oid:INT8 constraint_oid:INT8 constraint_relation_oid:INT8 constraint_schema_oid:INT8 constraint_schema_name_is_null:BOOL constraint_schema_name_bytes:INT8 constraint_schema_name:TEXT constraint_kind_is_null:BOOL constraint_kind_bytes:INT8 constraint_kind:TEXT is_deferrable:BOOL initially_deferred:BOOL constraint_type_oid:INT8 index_oid:INT8 foreign_relation_oid:INT8",
    );
    assert_projection(
        &CONTEXT_ROW,
        concat!(
            "current_user_is_null:BOOL current_user_bytes:INT8 current_user:TEXT ",
            "session_user_is_null:BOOL session_user_bytes:INT8 session_user:TEXT ",
            "observed_database_name_is_null:BOOL observed_database_name_bytes:INT8 observed_database_name:TEXT ",
            "database_name_is_null:BOOL database_name_bytes:INT8 database_name:TEXT ",
            "schema_name_is_null:BOOL schema_name_bytes:INT8 schema_name:TEXT ",
            "worker_oid:INT8 database_oid:INT8 schema_oid:INT8 migration_owner_oid:INT8 database_owner_oid:INT8 schema_owner_oid:INT8 ",
            "worker_can_login:BOOL worker_is_superuser:BOOL worker_can_create_database:BOOL worker_can_create_role:BOOL worker_can_replicate:BOOL worker_bypasses_rls:BOOL worker_inherits_roles:BOOL ",
            "row_security_is_null:BOOL row_security_bytes:INT8 row_security:TEXT ",
            "session_replication_role_is_null:BOOL session_replication_role_bytes:INT8 session_replication_role:TEXT ",
            "transaction_read_only:BOOL transaction_isolation_is_null:BOOL transaction_isolation_bytes:INT8 transaction_isolation:TEXT ",
            "search_path_is_null:BOOL search_path_bytes:INT8 search_path:TEXT ",
            "statement_timeout_is_null:BOOL statement_timeout_bytes:INT8 statement_timeout:TEXT ",
            "lock_timeout_is_null:BOOL lock_timeout_bytes:INT8 lock_timeout:TEXT"
        ),
    );
    assert_projection(
        &RELATION_ROW,
        concat!(
            "relation_ordinal:INT4 expected_name_is_null:BOOL expected_name_bytes:INT8 expected_name:TEXT ",
            "relation_oid:INT8 schema_oid:INT8 schema_name_is_null:BOOL schema_name_bytes:INT8 schema_name:TEXT ",
            "actual_name_is_null:BOOL actual_name_bytes:INT8 actual_name:TEXT owner_oid:INT8 ",
            "owner_name_is_null:BOOL owner_name_bytes:INT8 owner_name:TEXT ",
            "relation_kind_is_null:BOOL relation_kind_bytes:INT8 relation_kind:TEXT ",
            "persistence_is_null:BOOL persistence_bytes:INT8 persistence:TEXT is_partition:BOOL"
        ),
    );
    assert_projection(
        &ROOT_HEADER_ROW,
        concat!(
            "expected_ordinal:INT4 expected_name_is_null:BOOL expected_name_bytes:INT8 expected_name:TEXT ",
            "routine_oid:INT8 schema_oid:INT8 schema_name_is_null:BOOL schema_name_bytes:INT8 schema_name:TEXT ",
            "actual_name_is_null:BOOL actual_name_bytes:INT8 actual_name:TEXT owner_oid:INT8 ",
            "owner_name_is_null:BOOL owner_name_bytes:INT8 owner_name:TEXT ",
            "routine_kind_is_null:BOOL routine_kind_bytes:INT8 routine_kind:TEXT ",
            "argument_count:INT4 input_type_count:INT4 argument_defaults_count:INT4 variadic_oid:INT8 ",
            "returns_set:BOOL security_definer:BOOL all_argument_types_is_null:BOOL argument_modes_is_null:BOOL argument_names_is_null:BOOL argument_defaults_is_null:BOOL language_oid:INT8 ",
            "language_name_is_null:BOOL language_name_bytes:INT8 language_name:TEXT return_type_oid:INT8 return_namespace_oid:INT8 ",
            "return_namespace_name_is_null:BOOL return_namespace_name_bytes:INT8 return_namespace_name:TEXT ",
            "return_type_name_is_null:BOOL return_type_name_bytes:INT8 return_type_name:TEXT ",
            "config_is_null:BOOL config_dimensions:INT4 config_lower_bound:INT4 config_cardinality:INT4"
        ),
    );
    assert_projection(
        &TRIGGER_ROW,
        concat!(
            "trigger_oid:INT8 relation_oid:INT8 schema_oid:INT8 ",
            "schema_name_is_null:BOOL schema_name_bytes:INT8 schema_name:TEXT ",
            "relation_name_is_null:BOOL relation_name_bytes:INT8 relation_name:TEXT relation_owner_oid:INT8 ",
            "relation_owner_name_is_null:BOOL relation_owner_name_bytes:INT8 relation_owner_name:TEXT ",
            "trigger_name_is_null:BOOL trigger_name_bytes:INT8 trigger_name:TEXT function_oid:INT8 function_schema_oid:INT8 ",
            "function_schema_name_is_null:BOOL function_schema_name_bytes:INT8 function_schema_name:TEXT ",
            "function_name_is_null:BOOL function_name_bytes:INT8 function_name:TEXT function_owner_oid:INT8 ",
            "function_owner_name_is_null:BOOL function_owner_name_bytes:INT8 function_owner_name:TEXT ",
            "is_internal:BOOL enabled_is_null:BOOL enabled_bytes:INT8 enabled:TEXT parent_oid:INT8 event_bits:INT4 argument_count:INT4 ",
            "attribute_dimensions:INT4 attribute_lower_bound:INT4 attribute_upper_bound:INT4 attribute_cardinality:INT4 ",
            "trigger_args_is_null:BOOL trigger_args_bytes:INT8 trigger_args_prefix:BYTEA when_is_null:BOOL ",
            "constraint_relation_oid:INT8 constraint_oid:INT8 update_attribute_count:INT4 old_transition_name_is_null:BOOL new_transition_name_is_null:BOOL is_deferrable:BOOL initially_deferred:BOOL"
        ),
    );
    assert_eq!(
        pg_column_count(&TRIGGER_ROW),
        projection_signature(&TRIGGER_ROW).len()
    );
    assert_eq!(
        pg_column_count(&CONTEXT_ROW),
        projection_signature(&CONTEXT_ROW).len()
    );
    assert_eq!(
        pg_column_count(&RELATION_ROW),
        projection_signature(&RELATION_ROW).len()
    );
    assert_eq!(
        pg_column_count(&ROOT_HEADER_ROW),
        projection_signature(&ROOT_HEADER_ROW).len()
    );
    assert_eq!(
        pg_column_count(&SETUP_ROW),
        projection_signature(&SETUP_ROW).len()
    );
    assert_eq!(
        pg_column_count(&ROLE_ROW),
        projection_signature(&ROLE_ROW).len()
    );
    assert_eq!(
        pg_column_count(&MEMBERSHIP_ROW),
        projection_signature(&MEMBERSHIP_ROW).len()
    );
    assert_eq!(
        pg_column_count(&INHERITANCE_ROW),
        projection_signature(&INHERITANCE_ROW).len()
    );
    assert_eq!(
        pg_column_count(&ROOT_SETTING_ROW),
        projection_signature(&ROOT_SETTING_ROW).len()
    );
    assert_eq!(
        pg_column_count(&UPDATE_ATTRIBUTE_ROW),
        projection_signature(&UPDATE_ATTRIBUTE_ROW).len()
    );
    assert_eq!(
        pg_column_count(&USER_CONSTRAINT_ROW),
        projection_signature(&USER_CONSTRAINT_ROW).len()
    );
}

#[test]
fn typed_statement_plans_keep_timeout_and_oid_array_binds_exact() {
    let primary = [
        TEST_RELATION_OID_BASE + 7,
        TEST_RELATION_OID_BASE + 6,
        TEST_RELATION_OID_BASE + 1,
        TEST_RELATION_OID_BASE,
    ];
    let roots = std::array::from_fn(|index| TEST_ROOT_OID_BASE + index as u32);
    let update = [TEST_TRIGGER_OID_BASE + 1, TEST_TRIGGER_OID_BASE + 19];
    let constraints = [
        TEST_TRIGGER_OID_BASE + 3,
        TEST_TRIGGER_OID_BASE + 4,
        TEST_TRIGGER_OID_BASE + 5,
        TEST_TRIGGER_OID_BASE + 10,
        TEST_TRIGGER_OID_BASE + 11,
        TEST_TRIGGER_OID_BASE + 12,
        TEST_TRIGGER_OID_BASE + 13,
        TEST_TRIGGER_OID_BASE + 14,
        TEST_TRIGGER_OID_BASE + 15,
        TEST_TRIGGER_OID_BASE + 23,
        TEST_TRIGGER_OID_BASE + 24,
        TEST_TRIGGER_OID_BASE + 25,
        TEST_TRIGGER_OID_BASE + 26,
    ];
    let roots_i64 = (0..17)
        .map(|index| i64::from(TEST_ROOT_OID_BASE + index as u32))
        .collect::<Vec<_>>();
    let primary_i64 = primary.map(i64::from).to_vec();
    let update_i64 = update.map(i64::from).to_vec();
    let constraints_i64 = constraints.map(i64::from).to_vec();

    let configure = statement_plan(Statement::Configure {
        timeout_ms: TEST_CONFIG_TIMEOUT_MS,
    });
    assert_eq!(configure.kind, StatementKind::Configure);
    assert_eq!(configure.sql, SETUP_SQL);
    assert_eq!(configure.binds, vec![BindValue::I64(123_000)]);
    assert_eq!(timeout_bind_value(123_000), 123_000_i64);

    let inheritance = statement_plan(Statement::PrimaryInheritance {
        relation_oids: primary,
    });
    assert_eq!(inheritance.sql, PRIMARY_INHERITANCE_SQL);
    assert_eq!(
        inheritance.binds,
        vec![BindValue::I64Array(primary_i64.clone())]
    );
    assert_eq!(inheritance.category, Some(RawCategory::Inheritance));
    assert_eq!(inheritance.fetch_limit, 1);

    let settings = statement_plan(Statement::RootSettings { root_oids: roots });
    assert_eq!(settings.sql, ROOT_SETTINGS_SQL);
    assert_eq!(settings.binds, vec![BindValue::I64Array(roots_i64.clone())]);
    assert_eq!(settings.category, Some(RawCategory::RootSetting));
    assert_eq!(settings.fetch_limit, 26);

    let forward = statement_plan(Statement::ForwardTriggers {
        relation_oids: primary,
        root_oids: roots,
    });
    assert_eq!(forward.sql, FORWARD_TRIGGERS_SQL);
    assert_eq!(
        forward.binds,
        vec![
            BindValue::I64Array(primary_i64),
            BindValue::I64Array(roots_i64.clone())
        ]
    );
    assert_eq!(forward.category, Some(RawCategory::ForwardTrigger));
    assert_eq!(forward.fetch_limit, 22);

    let adjacent = statement_plan(Statement::AdjacentConfigHook {
        relation_oid: TEST_RELATION_OID_BASE + 9,
    });
    assert_eq!(adjacent.sql, ADJACENT_CONFIG_HOOK_SQL);
    assert_eq!(
        adjacent.binds,
        vec![BindValue::I64(i64::from(TEST_RELATION_OID_BASE + 9))]
    );
    assert_eq!(adjacent.fetch_limit, 2);

    let reverse = statement_plan(Statement::ReverseProtectedHooks { root_oids: roots });
    assert_eq!(reverse.sql, REVERSE_PROTECTED_HOOKS_SQL);
    assert_eq!(reverse.binds, vec![BindValue::I64Array(roots_i64)]);
    assert_eq!(reverse.fetch_limit, 28);

    let update_plan = statement_plan(Statement::UpdateOfAttributes {
        trigger_oids: update,
    });
    assert_eq!(update_plan.sql, UPDATE_OF_ATTRIBUTES_SQL);
    assert_eq!(update_plan.binds, vec![BindValue::I64Array(update_i64)]);
    assert_eq!(update_plan.fetch_limit, 3);

    let constraints_plan = statement_plan(Statement::UserTriggerConstraints {
        trigger_oids: constraints,
    });
    assert_eq!(constraints_plan.sql, USER_TRIGGER_CONSTRAINTS_SQL);
    assert_eq!(
        constraints_plan.binds,
        vec![BindValue::I64Array(constraints_i64)]
    );
    assert_eq!(constraints_plan.fetch_limit, 14);

    let membership = statement_plan(Statement::WorkerMemberships {
        worker_oid: TEST_WORKER_OID,
    });
    assert_eq!(
        membership.binds,
        vec![BindValue::I64(i64::from(TEST_WORKER_OID))]
    );
    assert_eq!(membership.fetch_limit, 1);
}

#[test]
fn a_complete_fake_catalog_yields_only_a_private_observation_after_close() {
    let mut fake = FakeTransport::valid();
    let observation = observe(&mut fake).expect("valid raw fake catalog");
    assert_eq!(observation.config_sha256(), observation.config_sha256);
    assert_eq!(observation.database_oid(), TEST_DATABASE_OID);
    assert_eq!(observation.worker_oid(), TEST_WORKER_OID);
    assert_eq!(observation.public_schema_oid(), TEST_SCHEMA_OID);
    assert_eq!(observation.migration_owner_oid(), TEST_OWNER_OID);
    assert_eq!(observation.policy_id, POLICY_ID);
    assert_eq!(observation.policy_fingerprint, POLICY_FINGERPRINT);
    assert_eq!(observation.database_oid, TEST_DATABASE_OID);
    assert_eq!(observation.worker_oid, TEST_WORKER_OID);
    assert_eq!(observation.schema_oid, TEST_SCHEMA_OID);
    assert_eq!(observation.migration_owner_oid, TEST_OWNER_OID);
    assert_eq!(observation.relation_oids.len(), 10);
    assert_eq!(observation.root_oids.len(), 17);
    assert_eq!(observation.trigger_oids.len(), 27);
    assert_eq!(observation.constraint_oids.len(), 13);
    assert!(observation.logical_bytes > 0 && observation.logical_bytes <= LOGICAL_PAYLOAD_LIMIT);

    let events = event_kinds(&fake);
    assert_eq!(events.first(), Some(&Event::Acquire));
    assert_eq!(events.get(1), Some(&Event::Armed));
    assert_eq!(events.last(), Some(&Event::Close));
    let statements = events
        .iter()
        .filter_map(|event| match event {
            Event::Statement(statement) => Some(*statement),
            _ => None,
        })
        .collect::<Vec<_>>();
    let primary = [
        TEST_RELATION_OID_BASE + 7,
        TEST_RELATION_OID_BASE + 6,
        TEST_RELATION_OID_BASE + 1,
        TEST_RELATION_OID_BASE,
    ];
    let roots = std::array::from_fn(|index| TEST_ROOT_OID_BASE + index as u32);
    let update = [TEST_TRIGGER_OID_BASE + 1, TEST_TRIGGER_OID_BASE + 19];
    let constraints = [
        TEST_TRIGGER_OID_BASE + 3,
        TEST_TRIGGER_OID_BASE + 4,
        TEST_TRIGGER_OID_BASE + 5,
        TEST_TRIGGER_OID_BASE + 10,
        TEST_TRIGGER_OID_BASE + 11,
        TEST_TRIGGER_OID_BASE + 12,
        TEST_TRIGGER_OID_BASE + 13,
        TEST_TRIGGER_OID_BASE + 14,
        TEST_TRIGGER_OID_BASE + 15,
        TEST_TRIGGER_OID_BASE + 23,
        TEST_TRIGGER_OID_BASE + 24,
        TEST_TRIGGER_OID_BASE + 25,
        TEST_TRIGGER_OID_BASE + 26,
    ];
    assert_eq!(
        statements,
        vec![
            Statement::Begin,
            Statement::Configure {
                timeout_ms: TEST_CONFIG_TIMEOUT_MS
            },
            Statement::Context,
            Statement::Roles,
            Statement::WorkerMemberships {
                worker_oid: TEST_WORKER_OID
            },
            Statement::Relations,
            Statement::PrimaryInheritance {
                relation_oids: primary
            },
            Statement::RootHeaders,
            Statement::RootSettings { root_oids: roots },
            Statement::ForwardTriggers {
                relation_oids: primary,
                root_oids: roots
            },
            Statement::AdjacentConfigHook {
                relation_oid: TEST_RELATION_OID_BASE + 9
            },
            Statement::ReverseProtectedHooks { root_oids: roots },
            Statement::UpdateOfAttributes {
                trigger_oids: update
            },
            Statement::UserTriggerConstraints {
                trigger_oids: constraints
            },
            Statement::FinalContext,
            Statement::Commit,
        ]
    );
}

fn assert_primary(
    fake: &mut FakeTransport,
    expected: PrimaryFailure,
) -> SimulationTriggerMetadataError {
    let error = observe_error(fake);
    assert_eq!(error.primary, expected);
    assert!(
        fake.event_log().contains(&Event::Close)
            || error.primary == PrimaryFailure::AcquisitionFailed
            || error.primary == PrimaryFailure::Canceled
    );
    error
}

#[test]
fn every_admitted_category_rejects_its_fetch_plus_one_row() {
    let mut fake = FakeTransport::valid();
    fake.fixture.setup.push(fake.fixture.setup[0].clone());
    assert_primary(&mut fake, PrimaryFailure::BoundExceeded);

    let mut fake = FakeTransport::valid();
    fake.fixture.context.push(fake.fixture.context[0].clone());
    assert_primary(&mut fake, PrimaryFailure::BoundExceeded);

    let mut fake = FakeTransport::valid();
    fake.fixture
        .final_context
        .push(fake.fixture.final_context[0].clone());
    assert_primary(&mut fake, PrimaryFailure::BoundExceeded);

    let mut fake = FakeTransport::valid();
    fake.fixture.roles.push(fake.fixture.roles[0].clone());
    assert_primary(&mut fake, PrimaryFailure::BoundExceeded);

    let mut fake = FakeTransport::valid();
    fake.fixture.memberships.push(row(
        RawCategory::Membership,
        vec![
            i64_cell(i64::from(TEST_WORKER_OID)),
            i64_cell(16_400),
            i64_cell(i64::from(TEST_OWNER_OID)),
            bool_cell(false),
            bool_cell(false),
            bool_cell(false),
        ],
    ));
    assert_primary(&mut fake, PrimaryFailure::BoundExceeded);

    let mut fake = FakeTransport::valid();
    fake.fixture
        .relations
        .push(fake.fixture.relations[0].clone());
    assert_primary(&mut fake, PrimaryFailure::BoundExceeded);

    let mut fake = FakeTransport::valid();
    fake.fixture.inheritance.push(row(
        RawCategory::Inheritance,
        vec![i64_cell(10), i64_cell(11)],
    ));
    assert_primary(&mut fake, PrimaryFailure::BoundExceeded);

    let mut fake = FakeTransport::valid();
    fake.fixture
        .root_headers
        .push(fake.fixture.root_headers[0].clone());
    assert_primary(&mut fake, PrimaryFailure::BoundExceeded);

    let mut fake = FakeTransport::valid();
    fake.fixture
        .root_settings
        .push(fake.fixture.root_settings[0].clone());
    assert_primary(&mut fake, PrimaryFailure::BoundExceeded);

    let mut fake = FakeTransport::valid();
    fake.fixture.forward.push(fake.fixture.forward[0].clone());
    assert_primary(&mut fake, PrimaryFailure::BoundExceeded);

    let mut fake = FakeTransport::valid();
    fake.fixture.adjacent.push(fake.fixture.adjacent[0].clone());
    assert_primary(&mut fake, PrimaryFailure::BoundExceeded);

    let mut fake = FakeTransport::valid();
    fake.fixture.reverse.push(fake.fixture.reverse[0].clone());
    assert_primary(&mut fake, PrimaryFailure::BoundExceeded);

    let mut fake = FakeTransport::valid();
    fake.fixture
        .update_attributes
        .push(fake.fixture.update_attributes[0].clone());
    assert_primary(&mut fake, PrimaryFailure::BoundExceeded);

    let mut fake = FakeTransport::valid();
    fake.fixture
        .constraints
        .push(fake.fixture.constraints[0].clone());
    assert_primary(&mut fake, PrimaryFailure::BoundExceeded);
}

#[test]
fn context_roles_relations_and_context_drift_fail_closed() {
    let mut wrong_user = FakeTransport::valid();
    set_text(&mut wrong_user.fixture.context[0], 0, "unreviewed_role");
    assert_primary(&mut wrong_user, PrimaryFailure::PolicyRejected);

    let mut elevated_worker = FakeTransport::valid();
    set_bool(&mut elevated_worker.fixture.context[0], 12, true);
    assert_primary(&mut elevated_worker, PrimaryFailure::PolicyRejected);

    let mut role_name = FakeTransport::valid();
    set_text(&mut role_name.fixture.roles[0], 2, "unexpected_worker");
    assert_primary(&mut role_name, PrimaryFailure::PolicyRejected);

    let mut role_oid = FakeTransport::valid();
    set_i64(&mut role_oid.fixture.roles[0], 3, 0);
    assert_primary(&mut role_oid, PrimaryFailure::CatalogInvalid);

    let mut unexpected_membership = FakeTransport::valid();
    unexpected_membership.fixture.memberships.push(row(
        RawCategory::Membership,
        vec![
            i64_cell(i64::from(TEST_WORKER_OID)),
            i64_cell(i64::from(TEST_OWNER_OID)),
            i64_cell(i64::from(TEST_OWNER_OID)),
            bool_cell(false),
            bool_cell(false),
            bool_cell(false),
        ],
    ));
    assert_primary(&mut unexpected_membership, PrimaryFailure::BoundExceeded);

    let mut relation_name = FakeTransport::valid();
    set_text(
        &mut relation_name.fixture.relations[0],
        5,
        "redirected_relation",
    );
    assert_primary(&mut relation_name, PrimaryFailure::PolicyRejected);

    let mut relation_owner = FakeTransport::valid();
    set_i64(
        &mut relation_owner.fixture.relations[0],
        6,
        i64::from(TEST_WORKER_OID),
    );
    assert_primary(&mut relation_owner, PrimaryFailure::PolicyRejected);

    let mut relation_oid = FakeTransport::valid();
    set_i64(
        &mut relation_oid.fixture.relations[0],
        2,
        i64::from(u32::MAX) + 1,
    );
    assert_primary(&mut relation_oid, PrimaryFailure::CatalogInvalid);

    let mut inherited = FakeTransport::valid();
    inherited.fixture.inheritance.push(row(
        RawCategory::Inheritance,
        vec![i64_cell(10), i64_cell(11)],
    ));
    assert_primary(&mut inherited, PrimaryFailure::BoundExceeded);

    let mut drifted = FakeTransport::valid();
    set_i64(
        &mut drifted.fixture.final_context[0],
        6,
        i64::from(TEST_DATABASE_OID + 1),
    );
    assert_primary(&mut drifted, PrimaryFailure::PolicyRejected);
}

#[test]
fn root_overloads_oids_and_proconfig_shapes_fail_closed() {
    let mut missing = FakeTransport::valid();
    missing.fixture.root_headers.pop();
    assert_primary(&mut missing, PrimaryFailure::CatalogInvalid);

    let mut wrong_identity = FakeTransport::valid();
    set_text(
        &mut wrong_identity.fixture.root_headers[0],
        5,
        "unexpected_overload",
    );
    assert_primary(&mut wrong_identity, PrimaryFailure::PolicyRejected);

    let mut zero_oid = FakeTransport::valid();
    set_i64(&mut zero_oid.fixture.root_headers[0], 2, 0);
    assert_primary(&mut zero_oid, PrimaryFailure::CatalogInvalid);

    let mut oversized_oid = FakeTransport::valid();
    set_i64(
        &mut oversized_oid.fixture.root_headers[0],
        2,
        i64::from(u32::MAX) + 1,
    );
    assert_primary(&mut oversized_oid, PrimaryFailure::CatalogInvalid);

    let mut wrong_owner = FakeTransport::valid();
    set_i64(
        &mut wrong_owner.fixture.root_headers[0],
        6,
        i64::from(TEST_WORKER_OID),
    );
    assert_primary(&mut wrong_owner, PrimaryFailure::PolicyRejected);

    let mut wrong_language = FakeTransport::valid();
    set_i64(
        &mut wrong_language.fixture.root_headers[0],
        19,
        i64::from(TEST_PG_CATALOG_OID),
    );
    assert_primary(&mut wrong_language, PrimaryFailure::CatalogInvalid);

    let mut wrong_security = FakeTransport::valid();
    set_bool(&mut wrong_security.fixture.root_headers[1], 14, false);
    assert_primary(&mut wrong_security, PrimaryFailure::PolicyRejected);

    let mut wrong_dimensions = FakeTransport::valid();
    set_i32(&mut wrong_dimensions.fixture.root_headers[1], 26, 2);
    assert_primary(&mut wrong_dimensions, PrimaryFailure::PolicyRejected);

    let mut wrong_lower_bound = FakeTransport::valid();
    set_i32(&mut wrong_lower_bound.fixture.root_headers[1], 27, 0);
    assert_primary(&mut wrong_lower_bound, PrimaryFailure::PolicyRejected);

    let mut wrong_cardinality = FakeTransport::valid();
    set_i32(&mut wrong_cardinality.fixture.root_headers[1], 28, 2);
    assert_primary(&mut wrong_cardinality, PrimaryFailure::PolicyRejected);
}

#[test]
fn root_setting_keys_order_values_and_lengths_fail_closed() {
    let mut unknown_key = FakeTransport::valid();
    set_text(
        &mut unknown_key.fixture.root_settings[1],
        2,
        "application_name=extra",
    );
    assert_primary(&mut unknown_key, PrimaryFailure::PolicyRejected);

    let mut duplicate_key = FakeTransport::valid();
    set_text(
        &mut duplicate_key.fixture.root_settings[2],
        2,
        "search_path=pg_catalog,pg_temp",
    );
    assert_primary(&mut duplicate_key, PrimaryFailure::PolicyRejected);

    let mut wrong_timeout = FakeTransport::valid();
    set_text(
        &mut wrong_timeout.fixture.root_settings[2],
        2,
        "lock_timeout=6s",
    );
    assert_primary(&mut wrong_timeout, PrimaryFailure::PolicyRejected);

    let mut wrong_ordinal = FakeTransport::valid();
    set_i32(&mut wrong_ordinal.fixture.root_settings[1], 1, 0);
    assert_primary(&mut wrong_ordinal, PrimaryFailure::PolicyRejected);

    let mut duplicate_order = FakeTransport::valid();
    set_i32(&mut duplicate_order.fixture.root_settings[2], 1, 1);
    assert_primary(&mut duplicate_order, PrimaryFailure::CatalogInvalid);
}

#[test]
fn root_search_path_allows_only_optional_ascii_spaces_after_commas() {
    let expected = "pg_catalog,pg_temp";
    assert!(search_path_matches("pg_catalog,pg_temp", expected));
    assert!(search_path_matches("pg_catalog, pg_temp", expected));
    assert!(search_path_matches("pg_catalog,    pg_temp", expected));
    for invalid in [
        " pg_catalog,pg_temp",
        "pg_catalog,pg_temp ",
        "pg_catalog,\tpg_temp",
        "pg_catalog,\npg_temp",
        "\"pg_catalog\",pg_temp",
        "pg_catalog,$user",
        "pg_catalog,pg_temp,public",
    ] {
        assert!(
            !search_path_matches(invalid, expected),
            "accepted {invalid:?}"
        );
    }

    let mut spaced_catalog_value = FakeTransport::valid();
    let index = spaced_catalog_value
        .fixture
        .root_settings
        .iter()
        .position(|row| {
            text_at(row, 2)
                .strip_prefix("search_path=")
                .is_some_and(|path| path.contains(','))
        })
        .expect("a pinned root has a comma-separated search_path");
    let original = text_at(&spaced_catalog_value.fixture.root_settings[index], 2);
    let path = original
        .strip_prefix("search_path=")
        .expect("search_path setting")
        .replace(',', ",    ");
    set_text(
        &mut spaced_catalog_value.fixture.root_settings[index],
        2,
        &format!("search_path={path}"),
    );
    assert!(observe(&mut spaced_catalog_value).is_ok());
}

#[test]
fn shared_raw_decoder_rejects_wrong_types_null_pairs_and_length_lies() {
    let mut wrong_type = FakeTransport::valid();
    wrong_type.fixture.context[0].cells[0] = RawCell::WrongType;
    assert_primary(&mut wrong_type, PrimaryFailure::CatalogInvalid);

    let mut null_required_text = FakeTransport::valid();
    null_required_text.fixture.context[0].cells[0] = null_text();
    assert_primary(&mut null_required_text, PrimaryFailure::CatalogInvalid);

    let mut inconsistent_null_pair = FakeTransport::valid();
    inconsistent_null_pair.fixture.context[0].cells[0] = RawCell::Text {
        is_null: false,
        original_bytes: None,
        value: b"worker".to_vec(),
    };
    assert_primary(&mut inconsistent_null_pair, PrimaryFailure::CatalogInvalid);

    let mut invalid_utf8 = FakeTransport::valid();
    invalid_utf8.fixture.context[0].cells[0] = RawCell::Text {
        is_null: false,
        original_bytes: Some(1),
        value: vec![0xff],
    };
    assert_primary(&mut invalid_utf8, PrimaryFailure::CatalogInvalid);

    let mut lied_length = FakeTransport::valid();
    lied_length.fixture.context[0].cells[0] = RawCell::Text {
        is_null: false,
        original_bytes: Some(2),
        value: b"worker".to_vec(),
    };
    assert_primary(&mut lied_length, PrimaryFailure::PolicyRejected);

    let mut oversized_text = FakeTransport::valid();
    oversized_text.fixture.context[0].cells[0] = RawCell::Text {
        is_null: false,
        original_bytes: Some(1025),
        value: vec![b'x'; 1025],
    };
    assert_primary(&mut oversized_text, PrimaryFailure::PolicyRejected);

    let mut oversized_total = FakeTransport::valid();
    oversized_total.fixture.context[0].cells[0] = RawCell::Text {
        is_null: false,
        original_bytes: Some(LOGICAL_PAYLOAD_LIMIT as i64),
        value: vec![b'x'; LOGICAL_PAYLOAD_LIMIT],
    };
    assert_primary(&mut oversized_total, PrimaryFailure::PayloadExceeded);

    let mut false_claim = FakeTransport::valid();
    false_claim.fixture.context[0].cells[0] = RawCell::Text {
        is_null: false,
        original_bytes: Some(LOGICAL_PAYLOAD_LIMIT as i64),
        value: b"worker".to_vec(),
    };
    assert_primary(&mut false_claim, PrimaryFailure::PolicyRejected);

    let mut oversized_null_payload = FakeTransport::valid();
    oversized_null_payload.fixture.context[0].cells[0] = RawCell::Text {
        is_null: true,
        original_bytes: None,
        value: vec![b'x'; LOGICAL_PAYLOAD_LIMIT],
    };
    assert_primary(&mut oversized_null_payload, PrimaryFailure::PayloadExceeded);

    let mut fake_transport_state_error = FakeTransport::valid();
    fake_transport_state_error.failure_at(
        Point::Statement(StatementKind::Context),
        TransportFailure::State,
    );
    assert_primary(
        &mut fake_transport_state_error,
        PrimaryFailure::CatalogInvalid,
    );

    let mut setup_oversized = FakeTransport::valid();
    setup_oversized.fixture.setup[0].cells[0] = RawCell::Text {
        is_null: false,
        original_bytes: Some(0),
        value: vec![b'x'; LOGICAL_PAYLOAD_LIMIT],
    };
    assert_primary(&mut setup_oversized, PrimaryFailure::PayloadExceeded);
}

#[test]
fn positive_oid_conversion_has_an_explicit_i64_and_u32_boundary() {
    assert_eq!(positive_oid(1).expect("positive OID"), 1);
    assert_eq!(
        positive_oid(i64::from(u32::MAX)).expect("largest OID"),
        u32::MAX
    );
    assert_eq!(
        positive_oid(0).expect_err("zero OID").primary,
        PrimaryFailure::CatalogInvalid
    );
    assert_eq!(
        positive_oid(-1).expect_err("negative OID").primary,
        PrimaryFailure::CatalogInvalid
    );
    assert_eq!(
        positive_oid(i64::from(u32::MAX) + 1)
            .expect_err("out-of-range OID")
            .primary,
        PrimaryFailure::CatalogInvalid
    );
}

#[test]
fn logical_payload_budget_uses_the_fixed_cell_costs_and_checked_limit() {
    let mut budget = LogicalPayloadBudget::default();
    for cell in [
        text("é"),
        RawCell::Bytes {
            is_null: false,
            original_bytes: Some(1),
            prefix: vec![0x41],
        },
        i64_cell(1),
        i32_cell(1),
        bool_cell(true),
        RawCell::Null,
        RawCell::WrongType,
    ] {
        budget.charge(&cell).expect("within logical budget");
    }
    assert_eq!(budget.used, 34);

    let mut at_limit = LogicalPayloadBudget {
        used: LOGICAL_PAYLOAD_LIMIT - 9,
    };
    at_limit
        .charge(&RawCell::Bytes {
            is_null: false,
            original_bytes: Some(1),
            prefix: vec![0x41],
        })
        .expect("exact logical budget limit");
    assert_eq!(at_limit.used, LOGICAL_PAYLOAD_LIMIT);
    assert_eq!(
        at_limit
            .charge(&bool_cell(false))
            .expect_err("over logical budget")
            .primary,
        PrimaryFailure::PayloadExceeded
    );
    assert_eq!(at_limit.used, LOGICAL_PAYLOAD_LIMIT);

    let mut false_claim = LogicalPayloadBudget::default();
    false_claim
        .charge(&RawCell::Text {
            is_null: false,
            original_bytes: Some(LOGICAL_PAYLOAD_LIMIT as i64),
            value: b"x".to_vec(),
        })
        .expect("charge materialized bytes, not the claim");
    assert_eq!(false_claim.used, 9);

    let mut bytea_false_claim = LogicalPayloadBudget::default();
    bytea_false_claim
        .charge(&RawCell::Bytes {
            is_null: false,
            original_bytes: Some(LOGICAL_PAYLOAD_LIMIT as i64),
            prefix: vec![0x41],
        })
        .expect("charge materialized bytea prefix");
    assert_eq!(bytea_false_claim.used, 9);

    let mut oversized_bytea_prefix = LogicalPayloadBudget::default();
    assert_eq!(
        oversized_bytea_prefix
            .charge(&RawCell::Bytes {
                is_null: true,
                original_bytes: None,
                prefix: vec![0xff; LOGICAL_PAYLOAD_LIMIT],
            })
            .expect_err("charge malformed materialized bytea before validation")
            .primary,
        PrimaryFailure::PayloadExceeded
    );

    let mut overflow = LogicalPayloadBudget { used: usize::MAX };
    assert_eq!(
        overflow
            .charge(&bool_cell(false))
            .expect_err("checked cumulative overflow")
            .primary,
        PrimaryFailure::PayloadExceeded
    );
    assert_eq!(overflow.used, usize::MAX);
}

#[test]
fn shared_raw_text_decoder_accepts_multibyte_utf8_and_rejects_invalid_bytes() {
    let shape = [PgCellKind::Text {
        null_alias: "is_null",
        bytes_alias: "byte_length",
        value_alias: "value",
    }];
    let mut budget = LogicalPayloadBudget::default();
    let mut valid = RawDecoder::new(
        RawRow::new(
            RawCategory::Context,
            vec![RawCell::Text {
                is_null: false,
                original_bytes: Some(2),
                value: "é".as_bytes().to_vec(),
            }],
        ),
        RawCategory::Context,
        &shape,
        &mut budget,
    )
    .expect("bounded raw row");
    assert_eq!(valid.text().expect("valid UTF-8"), "é");
    valid.finish().expect("all cells consumed");

    let mut invalid = RawDecoder::new(
        RawRow::new(
            RawCategory::Context,
            vec![RawCell::Text {
                is_null: false,
                original_bytes: Some(1),
                value: vec![0xff],
            }],
        ),
        RawCategory::Context,
        &shape,
        &mut budget,
    )
    .expect("bounded malformed row");
    assert_eq!(
        invalid.text().expect_err("invalid UTF-8").primary,
        PrimaryFailure::CatalogInvalid
    );
}

#[test]
fn trigger_numeric_candidates_and_update_vector_bounds_fail_closed() {
    let mut negative_bits = FakeTransport::valid();
    let index = trigger_index(
        &negative_bits.fixture.forward,
        "jobs",
        "jobs_protect_scheduled_recommendation_lineage",
    );
    set_i32(&mut negative_bits.fixture.forward[index], 17, -1);
    assert_primary(&mut negative_bits, PrimaryFailure::PolicyRejected);

    let mut unknown_bits = FakeTransport::valid();
    let index = trigger_index(
        &unknown_bits.fixture.reverse,
        "jobs",
        "jobs_protect_scheduled_recommendation_lineage",
    );
    set_i32(&mut unknown_bits.fixture.reverse[index], 17, 128);
    assert_primary(&mut unknown_bits, PrimaryFailure::PolicyRejected);

    let mut row_level_missing = FakeTransport::valid();
    let index = trigger_index(
        &row_level_missing.fixture.forward,
        "jobs",
        "jobs_protect_scheduled_recommendation_lineage",
    );
    set_i32(&mut row_level_missing.fixture.forward[index], 17, 30);
    assert_primary(&mut row_level_missing, PrimaryFailure::PolicyRejected);

    let mut update_lower_bound = FakeTransport::valid();
    let index = trigger_index(
        &update_lower_bound.fixture.forward,
        "jobs",
        "jobs_sync_recommendation_terminal_run",
    );
    set_i32(&mut update_lower_bound.fixture.forward[index], 20, 1);
    assert_primary(&mut update_lower_bound, PrimaryFailure::PolicyRejected);

    let mut update_extra_attribute = FakeTransport::valid();
    let index = trigger_index(
        &update_extra_attribute.fixture.reverse,
        "jobs",
        "jobs_sync_recommendation_terminal_run",
    );
    set_i32(&mut update_extra_attribute.fixture.reverse[index], 22, 2);
    assert_primary(&mut update_extra_attribute, PrimaryFailure::PolicyRejected);
}

#[test]
fn constraint_crosslinks_reject_non_slice_properties_and_flag_drift() {
    let mut wrong_schema = FakeTransport::valid();
    set_text(&mut wrong_schema.fixture.constraints[0], 6, "other_schema");
    assert_primary(&mut wrong_schema, PrimaryFailure::PolicyRejected);

    let mut not_deferrable = FakeTransport::valid();
    set_bool(&mut not_deferrable.fixture.constraints[0], 8, false);
    assert_primary(&mut not_deferrable, PrimaryFailure::PolicyRejected);

    let mut wrong_type_oid = FakeTransport::valid();
    set_i64(&mut wrong_type_oid.fixture.constraints[0], 10, 1);
    assert_primary(&mut wrong_type_oid, PrimaryFailure::PolicyRejected);

    let mut index_backed = FakeTransport::valid();
    set_i64(&mut index_backed.fixture.constraints[0], 11, 1);
    assert_primary(&mut index_backed, PrimaryFailure::PolicyRejected);

    let mut foreign_relation = FakeTransport::valid();
    set_i64(&mut foreign_relation.fixture.constraints[0], 12, 1);
    assert_primary(&mut foreign_relation, PrimaryFailure::PolicyRejected);

    let mut detached_relation = FakeTransport::valid();
    let index = trigger_index(
        &detached_relation.fixture.reverse,
        "jobs",
        "candidate_job_requires_publication",
    );
    set_i64(&mut detached_relation.fixture.reverse[index], 25, 1);
    assert_primary(&mut detached_relation, PrimaryFailure::PolicyRejected);
}

#[test]
fn missing_extra_unknown_and_redirected_trigger_rows_fail_closed() {
    let mut missing = FakeTransport::valid();
    missing.fixture.forward.pop();
    assert_primary(&mut missing, PrimaryFailure::CatalogInvalid);

    let mut unknown_forward = FakeTransport::valid();
    let index = trigger_index(
        &unknown_forward.fixture.forward,
        "jobs",
        "jobs_protect_scheduled_recommendation_lineage",
    );
    set_text(
        &mut unknown_forward.fixture.forward[index],
        7,
        "jobs_unreviewed_shared_hook",
    );
    assert_primary(&mut unknown_forward, PrimaryFailure::PolicyRejected);

    let mut wrong_relation = FakeTransport::valid();
    let index = trigger_index(
        &wrong_relation.fixture.reverse,
        "stock_analysis_runs",
        "stock_analysis_run_requires_settlement",
    );
    set_text(&mut wrong_relation.fixture.reverse[index], 4, "jobs");
    assert_primary(&mut wrong_relation, PrimaryFailure::PolicyRejected);

    let mut unknown_reverse = FakeTransport::valid();
    let index = trigger_index(
        &unknown_reverse.fixture.reverse,
        "owner_beta_daily_reservations",
        "daily_0060_consistency",
    );
    set_text(
        &mut unknown_reverse.fixture.reverse[index],
        7,
        "daily_0060_unreviewed_hook",
    );
    assert_primary(&mut unknown_reverse, PrimaryFailure::PolicyRejected);

    let mut internal_protected = FakeTransport::valid();
    let index = trigger_index(
        &internal_protected.fixture.forward,
        "jobs",
        "jobs_protect_scheduled_recommendation_lineage",
    );
    set_bool(&mut internal_protected.fixture.forward[index], 14, true);
    assert_primary(&mut internal_protected, PrimaryFailure::PolicyRejected);

    let mut prefix_redirect = FakeTransport::valid();
    let index = trigger_index(
        &prefix_redirect.fixture.reverse,
        "jobs",
        "owner_beta_simulation_jobs_insert_guard",
    );
    set_text(
        &mut prefix_redirect.fixture.reverse[index],
        11,
        "owner_beta_simulation_redirected_guard",
    );
    assert_primary(&mut prefix_redirect, PrimaryFailure::PolicyRejected);
}

#[test]
fn trigger_header_flags_event_masks_and_array_shapes_are_checked() {
    let mut enabled = FakeTransport::valid();
    let index = trigger_index(
        &enabled.fixture.forward,
        "jobs",
        "jobs_protect_scheduled_recommendation_lineage",
    );
    set_text(&mut enabled.fixture.forward[index], 15, "D");
    assert_primary(&mut enabled, PrimaryFailure::PolicyRejected);

    let mut event_mask = FakeTransport::valid();
    let index = trigger_index(
        &event_mask.fixture.reverse,
        "jobs",
        "jobs_protect_scheduled_recommendation_lineage",
    );
    set_i32(&mut event_mask.fixture.reverse[index], 17, 127);
    assert_primary(&mut event_mask, PrimaryFailure::PolicyRejected);

    let mut parent = FakeTransport::valid();
    let index = trigger_index(
        &parent.fixture.forward,
        "jobs",
        "jobs_protect_scheduled_recommendation_lineage",
    );
    set_i64(&mut parent.fixture.forward[index], 16, 77);
    assert_primary(&mut parent, PrimaryFailure::PolicyRejected);

    let mut update_shape = FakeTransport::valid();
    let index = trigger_index(
        &update_shape.fixture.forward,
        "jobs",
        "jobs_sync_recommendation_terminal_run",
    );
    set_i32(&mut update_shape.fixture.forward[index], 20, 1);
    assert_primary(&mut update_shape, PrimaryFailure::PolicyRejected);

    for empty_shape in [
        [RawCell::Null, RawCell::Null, RawCell::Null, i32_cell(0)],
        [i32_cell(0), RawCell::Null, RawCell::Null, i32_cell(0)],
    ] {
        let mut fake = FakeTransport::valid();
        for rows in [&mut fake.fixture.forward, &mut fake.fixture.reverse] {
            let index = trigger_index(
                rows,
                "jobs",
                "jobs_protect_scheduled_recommendation_lineage",
            );
            rows[index].cells[19..23].clone_from_slice(&empty_shape);
        }
        assert!(observe(&mut fake).is_ok());
    }
}

#[test]
fn trigger_defer_flags_match_ordinary_and_constraint_censuses() {
    let mut ordinary_forward = FakeTransport::valid();
    let index = trigger_index(
        &ordinary_forward.fixture.forward,
        "jobs",
        "jobs_protect_scheduled_recommendation_lineage",
    );
    set_bool(&mut ordinary_forward.fixture.forward[index], 30, true);
    assert_primary(&mut ordinary_forward, PrimaryFailure::PolicyRejected);

    let mut ordinary_adjacent = FakeTransport::valid();
    let index = ordinary_adjacent
        .fixture
        .adjacent
        .iter()
        .position(|row| text_at(row, 7) == "daily_0060_config_epoch")
        .expect("adjacent config hook fixture");
    set_bool(&mut ordinary_adjacent.fixture.adjacent[index], 31, true);
    assert_primary(&mut ordinary_adjacent, PrimaryFailure::PolicyRejected);

    let mut ordinary_reverse = FakeTransport::valid();
    let index = trigger_index(
        &ordinary_reverse.fixture.reverse,
        "jobs",
        "jobs_protect_scheduled_recommendation_lineage",
    );
    set_bool(&mut ordinary_reverse.fixture.reverse[index], 30, true);
    assert_primary(&mut ordinary_reverse, PrimaryFailure::PolicyRejected);

    let mut constraint_forward = FakeTransport::valid();
    let index = trigger_index(
        &constraint_forward.fixture.forward,
        "jobs",
        "candidate_job_requires_publication",
    );
    set_bool(&mut constraint_forward.fixture.forward[index], 31, false);
    assert_primary(&mut constraint_forward, PrimaryFailure::PolicyRejected);
}

#[test]
fn update_of_status_requires_the_resolved_live_attribute_crosslink() {
    let mut dropped = FakeTransport::valid();
    set_bool(&mut dropped.fixture.update_attributes[0], 6, true);
    assert_primary(&mut dropped, PrimaryFailure::PolicyRejected);

    let mut foreign_relation = FakeTransport::valid();
    set_i64(
        &mut foreign_relation.fixture.update_attributes[0],
        5,
        i64::from(relation_oid("job_attempts")),
    );
    assert_primary(&mut foreign_relation, PrimaryFailure::PolicyRejected);

    let mut wrong_name = FakeTransport::valid();
    set_text(
        &mut wrong_name.fixture.update_attributes[0],
        4,
        "other_status",
    );
    assert_primary(&mut wrong_name, PrimaryFailure::PolicyRejected);

    let mut wrong_ordinal = FakeTransport::valid();
    set_i64(&mut wrong_ordinal.fixture.update_attributes[0], 2, 2);
    assert_primary(&mut wrong_ordinal, PrimaryFailure::PolicyRejected);

    let mut missing = FakeTransport::valid();
    missing.fixture.update_attributes.pop();
    assert_primary(&mut missing, PrimaryFailure::CatalogInvalid);
}

#[test]
fn trigger_args_distinguish_empty_null_wrong_type_and_nonempty_bytes() {
    let mut accepted_empty = FakeTransport::valid();
    assert!(observe(&mut accepted_empty).is_ok());

    let mut null_args = FakeTransport::valid();
    let index = trigger_index(
        &null_args.fixture.forward,
        "jobs",
        "jobs_protect_scheduled_recommendation_lineage",
    );
    null_args.fixture.forward[index].cells[23] = RawCell::Bytes {
        is_null: true,
        original_bytes: None,
        prefix: Vec::new(),
    };
    assert_primary(&mut null_args, PrimaryFailure::PolicyRejected);

    let mut wrong_type = FakeTransport::valid();
    let index = trigger_index(
        &wrong_type.fixture.reverse,
        "jobs",
        "jobs_protect_scheduled_recommendation_lineage",
    );
    wrong_type.fixture.reverse[index].cells[23] = RawCell::WrongType;
    assert_primary(&mut wrong_type, PrimaryFailure::CatalogInvalid);

    let mut nonempty = FakeTransport::valid();
    let index = trigger_index(
        &nonempty.fixture.forward,
        "jobs",
        "jobs_protect_scheduled_recommendation_lineage",
    );
    nonempty.fixture.forward[index].cells[23] = RawCell::Bytes {
        is_null: false,
        original_bytes: Some(1),
        prefix: vec![0x41],
    };
    assert_primary(&mut nonempty, PrimaryFailure::PolicyRejected);
}

#[test]
fn all_thirteen_user_constraint_crosslinks_and_flags_are_checked() {
    let mut missing = FakeTransport::valid();
    missing.fixture.constraints.pop();
    assert_primary(&mut missing, PrimaryFailure::CatalogInvalid);

    let mut foreign_key = FakeTransport::valid();
    set_text(&mut foreign_key.fixture.constraints[0], 7, "f");
    assert_primary(&mut foreign_key, PrimaryFailure::PolicyRejected);

    let mut not_deferred = FakeTransport::valid();
    set_bool(&mut not_deferred.fixture.constraints[0], 9, false);
    assert_primary(&mut not_deferred, PrimaryFailure::PolicyRejected);

    let mut not_deferrable = FakeTransport::valid();
    set_bool(&mut not_deferrable.fixture.constraints[0], 8, false);
    assert_primary(&mut not_deferrable, PrimaryFailure::PolicyRejected);

    let mut wrong_relation = FakeTransport::valid();
    set_i64(
        &mut wrong_relation.fixture.constraints[0],
        4,
        i64::from(TEST_RELATION_OID_BASE + 1),
    );
    assert_primary(&mut wrong_relation, PrimaryFailure::PolicyRejected);

    for index in 0..USER_CONSTRAINT_ACCEPT_LIMIT {
        let mut substituted = FakeTransport::valid();
        set_i64(
            &mut substituted.fixture.constraints[index],
            1,
            i64::from(TEST_RELATION_OID_BASE + 100),
        );
        set_i64(
            &mut substituted.fixture.constraints[index],
            4,
            i64::from(TEST_RELATION_OID_BASE + 100),
        );
        assert_primary(&mut substituted, PrimaryFailure::PolicyRejected);
    }

    let mut detached_trigger = FakeTransport::valid();
    let index = trigger_index(
        &detached_trigger.fixture.reverse,
        "jobs",
        "candidate_job_requires_publication",
    );
    set_i64(
        &mut detached_trigger.fixture.reverse[index],
        26,
        i64::from(TEST_CONSTRAINT_OID_BASE + 99),
    );
    assert_primary(&mut detached_trigger, PrimaryFailure::PolicyRejected);

    let mut ordinary_with_constraint = FakeTransport::valid();
    let index = trigger_index(
        &ordinary_with_constraint.fixture.forward,
        "jobs",
        "jobs_protect_scheduled_recommendation_lineage",
    );
    set_i64(
        &mut ordinary_with_constraint.fixture.forward[index],
        26,
        i64::from(TEST_CONSTRAINT_OID_BASE),
    );
    assert_primary(
        &mut ordinary_with_constraint,
        PrimaryFailure::PolicyRejected,
    );
}

#[test]
fn same_await_database_failures_keep_the_primary_and_record_cancellation() {
    let queries = [
        StatementKind::Configure,
        StatementKind::Context,
        StatementKind::Roles,
        StatementKind::WorkerMemberships,
        StatementKind::Relations,
        StatementKind::PrimaryInheritance,
        StatementKind::RootHeaders,
        StatementKind::RootSettings,
        StatementKind::ForwardTriggers,
        StatementKind::AdjacentConfigHook,
        StatementKind::ReverseProtectedHooks,
        StatementKind::UpdateOfAttributes,
        StatementKind::UserTriggerConstraints,
        StatementKind::FinalContext,
    ];
    for kind in queries {
        let mut fake = FakeTransport::valid();
        fake.failure_at(Point::Statement(kind), TransportFailure::Database);
        fake.cancel_at = Some(Point::Statement(kind));
        let expected = if kind == StatementKind::Configure {
            PrimaryFailure::SetupFailed
        } else {
            PrimaryFailure::CatalogUnavailable
        };
        let error = assert_primary(&mut fake, expected);
        assert!(error.canceled);
        assert_eq!(error.begin, BeginState::Started);
        assert_eq!(error.rollback, CleanupState::Confirmed);
        assert_eq!(error.close, CleanupState::Confirmed);
        assert_failure_rollback_close_order(&fake, kind);
    }
}

#[test]
fn same_await_state_and_fetch_bound_failures_keep_their_classification() {
    let mut state = FakeTransport::valid();
    state.failure_at(
        Point::Statement(StatementKind::Roles),
        TransportFailure::State,
    );
    state.cancel_at = Some(Point::Statement(StatementKind::Roles));
    let error = assert_primary(&mut state, PrimaryFailure::CatalogInvalid);
    assert!(error.canceled);
    assert_eq!(error.rollback, CleanupState::Confirmed);
    assert_eq!(error.close, CleanupState::Confirmed);
    assert_failure_rollback_close_order(&state, StatementKind::Roles);

    let mut fetch_bound = FakeTransport::valid();
    fetch_bound.failure_at(
        Point::Statement(StatementKind::RootSettings),
        TransportFailure::FetchBound,
    );
    fetch_bound.cancel_at = Some(Point::Statement(StatementKind::RootSettings));
    let error = assert_primary(&mut fetch_bound, PrimaryFailure::BoundExceeded);
    assert!(error.canceled);
    assert_eq!(error.rollback, CleanupState::Confirmed);
    assert_eq!(error.close, CleanupState::Confirmed);
    assert_failure_rollback_close_order(&fetch_bound, StatementKind::RootSettings);
}

#[test]
fn primary_cancellation_rollback_and_close_uncertainty_are_independent() {
    let mut fake = FakeTransport::valid();
    fake.failure_at(
        Point::Statement(StatementKind::Configure),
        TransportFailure::Database,
    );
    fake.cancel_at = Some(Point::Statement(StatementKind::Configure));
    fake.rollback_fails = true;
    fake.close_fails = true;

    let error = assert_primary(&mut fake, PrimaryFailure::SetupFailed);
    assert!(error.canceled);
    assert_eq!(error.begin, BeginState::Started);
    assert_eq!(error.rollback, CleanupState::Unconfirmed);
    assert_eq!(error.close, CleanupState::Unconfirmed);
    assert!(
        fake.event_log()
            .contains(&Event::Statement(Statement::Rollback))
    );
    assert!(fake.event_log().contains(&Event::Close));
}

#[test]
fn unconfirmed_commit_rolls_back_then_closes_without_claiming_completion() {
    let mut failed = FakeTransport::valid();
    failed.failure_at(
        Point::Statement(StatementKind::Commit),
        TransportFailure::Database,
    );
    failed.cancel_at = Some(Point::Statement(StatementKind::Commit));
    failed.rollback_fails = true;
    failed.close_fails = true;
    let error = assert_primary(&mut failed, PrimaryFailure::CompletionUnconfirmed);
    assert!(error.canceled);
    assert_eq!(error.begin, BeginState::Started);
    assert_eq!(error.completion, CompletionState::Unconfirmed);
    assert_eq!(error.rollback, CleanupState::Unconfirmed);
    assert_eq!(error.close, CleanupState::Unconfirmed);
    assert_failure_rollback_close_order(&failed, StatementKind::Commit);

    let mut nonempty = FakeTransport::valid();
    nonempty.commit_nonempty = true;
    nonempty.cancel_at = Some(Point::Statement(StatementKind::Commit));
    let error = assert_primary(&mut nonempty, PrimaryFailure::CompletionUnconfirmed);
    assert!(error.canceled);
    assert_eq!(error.completion, CompletionState::Unconfirmed);
    assert_eq!(error.rollback, CleanupState::Confirmed);
    assert_eq!(error.close, CleanupState::Confirmed);
    assert_failure_rollback_close_order(&nonempty, StatementKind::Commit);
}

#[test]
fn commit_state_and_fetch_bound_failures_preserve_uncertainty_and_cleanup() {
    for failure in [TransportFailure::State, TransportFailure::FetchBound] {
        for (rollback_fails, close_fails, expected_rollback, expected_close) in [
            (
                false,
                false,
                CleanupState::Confirmed,
                CleanupState::Confirmed,
            ),
            (
                true,
                false,
                CleanupState::Unconfirmed,
                CleanupState::Confirmed,
            ),
            (
                false,
                true,
                CleanupState::Confirmed,
                CleanupState::Unconfirmed,
            ),
            (
                true,
                true,
                CleanupState::Unconfirmed,
                CleanupState::Unconfirmed,
            ),
        ] {
            let mut fake = FakeTransport::valid();
            fake.failure_at(Point::Statement(StatementKind::Commit), failure);
            fake.cancel_at = Some(Point::Statement(StatementKind::Commit));
            fake.rollback_fails = rollback_fails;
            fake.close_fails = close_fails;

            let error = assert_primary(&mut fake, PrimaryFailure::CompletionUnconfirmed);
            assert_eq!(error.primary, PrimaryFailure::CompletionUnconfirmed);
            assert!(error.canceled);
            assert_eq!(error.begin, BeginState::Started);
            assert_eq!(error.completion, CompletionState::Unconfirmed);
            assert_eq!(error.rollback, expected_rollback);
            assert_eq!(error.close, expected_close);
            assert_failure_rollback_close_order(&fake, StatementKind::Commit);
        }
    }
}

#[test]
fn postcommit_and_close_cancellation_never_publish_an_observation() {
    let mut during_commit = FakeTransport::valid();
    during_commit.cancel_at = Some(Point::Statement(StatementKind::Commit));
    let error = assert_primary(&mut during_commit, PrimaryFailure::Canceled);
    assert!(error.canceled);
    assert_eq!(error.completion, CompletionState::Acknowledged);
    assert_eq!(error.close, CleanupState::Confirmed);

    let mut during_close = FakeTransport::valid();
    during_close.cancel_at = Some(Point::Close);
    let error = assert_primary(&mut during_close, PrimaryFailure::Canceled);
    assert!(error.canceled);
    assert_eq!(error.completion, CompletionState::Acknowledged);
    assert_eq!(error.close, CleanupState::Confirmed);
}

#[test]
fn prebegin_acquisition_and_begin_failures_preserve_lifecycle_states() {
    let mut pre_canceled = FakeTransport::valid();
    pre_canceled.canceled.store(true, Ordering::Release);
    let error = assert_primary(&mut pre_canceled, PrimaryFailure::Canceled);
    assert!(error.canceled);
    assert_eq!(error.begin, BeginState::NotAttempted);
    assert!(!pre_canceled.event_log().contains(&Event::Acquire));

    let mut acquire_failed = FakeTransport::valid();
    acquire_failed.failure_at(Point::Acquire, TransportFailure::Database);
    acquire_failed.cancel_at = Some(Point::Acquire);
    let error = assert_primary(&mut acquire_failed, PrimaryFailure::AcquisitionFailed);
    assert!(error.canceled);
    assert_eq!(error.begin, BeginState::NotAttempted);
    assert!(!acquire_failed.event_log().contains(&Event::Armed));

    let mut canceled_before_begin = FakeTransport::valid();
    canceled_before_begin.cancel_at = Some(Point::Arm);
    let error = assert_primary(&mut canceled_before_begin, PrimaryFailure::Canceled);
    assert!(error.canceled);
    assert_eq!(error.begin, BeginState::NotAttempted);
    assert_eq!(error.close, CleanupState::Confirmed);
    assert!(
        !canceled_before_begin
            .event_log()
            .contains(&Event::Statement(Statement::Begin))
    );

    let mut begin_failed = FakeTransport::valid();
    begin_failed.failure_at(
        Point::Statement(StatementKind::Begin),
        TransportFailure::Database,
    );
    begin_failed.cancel_at = Some(Point::Statement(StatementKind::Begin));
    let error = assert_primary(&mut begin_failed, PrimaryFailure::BeginFailed);
    assert!(error.canceled);
    assert_eq!(error.begin, BeginState::Uncertain);
    assert_eq!(error.rollback, CleanupState::Confirmed);
    assert_eq!(error.close, CleanupState::Confirmed);

    let mut begin_nonempty = FakeTransport::valid();
    begin_nonempty.begin_nonempty = true;
    begin_nonempty.cancel_at = Some(Point::Statement(StatementKind::Begin));
    let error = assert_primary(&mut begin_nonempty, PrimaryFailure::BeginResultInvalid);
    assert!(error.canceled);
    assert_eq!(error.begin, BeginState::Uncertain);
    assert_eq!(error.rollback, CleanupState::Confirmed);
    assert_eq!(error.close, CleanupState::Confirmed);
}

#[test]
fn dropping_pending_futures_records_only_guarded_disposal() {
    let mut before_arm = FakeTransport::valid();
    before_arm.pending_acquire = true;
    poll_pending_then_drop(&mut before_arm);
    assert!(
        before_arm
            .event_log()
            .contains(&Event::PendingDroppedAfterArm(false))
    );
    assert!(!before_arm.event_log().contains(&Event::Armed));
    assert!(!before_arm.event_log().contains(&Event::Close));

    let mut after_arm = FakeTransport::valid();
    after_arm.pending_at = Some(StatementKind::Begin);
    poll_pending_then_drop(&mut after_arm);
    assert!(
        after_arm
            .event_log()
            .contains(&Event::PendingDroppedAfterArm(true))
    );
    assert!(after_arm.event_log().contains(&Event::Armed));
    assert!(!after_arm.event_log().contains(&Event::Close));
    assert!(
        !after_arm
            .event_log()
            .contains(&Event::Statement(Statement::Rollback))
    );
}
#[test]
fn command_result_discards_row_payload_and_rejects_presence() {
    assert!(super::classify_command_result(None::<String>).is_empty());
    let rows = super::classify_command_result(Some(String::from("sentinel")));
    assert_eq!(rows.len(), 1);
    assert!(rows[0].cells.is_empty());
}

#[test]
fn rollback_row_keeps_cleanup_unconfirmed_after_primary_failure() {
    let mut fake = FakeTransport::valid();
    fake.failure_at(
        Point::Statement(StatementKind::Configure),
        TransportFailure::Database,
    );
    fake.unexpected_at = Some(StatementKind::Rollback);
    let error = assert_primary(&mut fake, PrimaryFailure::SetupFailed);
    assert_eq!(error.rollback, CleanupState::Unconfirmed);
    assert_eq!(error.close, CleanupState::Confirmed);
    assert_failure_rollback_close_order(&fake, StatementKind::Configure);
}
