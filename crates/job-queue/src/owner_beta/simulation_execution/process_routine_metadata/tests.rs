// The test transport retains optional cells and one-step polling fixtures.
#![allow(dead_code, clippy::never_loop)]

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
#[path = "../../../../tests/common/mod.rs"]
mod pg_test_common;

#[cfg(feature = "r4-pg-observer-tests")]
#[tokio::test]
async fn r4_routine_metadata_pg_projection() {
    let db = pg_test_common::ScratchDb::create()
        .await
        .expect("disposable DATABASE_URL is required");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&db.role_url("worker"))
        .await
        .expect("connect directly as worker to disposable PostgreSQL");
    let worker_oid: i64 = sqlx::query_scalar(
        "SELECT oid::pg_catalog.int8 FROM pg_catalog.pg_roles WHERE rolname = 'worker'",
    )
    .fetch_one(&pool)
    .await
    .expect("worker role exists");
    let mut role_oids = [0_u32; 6];
    for (index, name) in ROLE_NAMES.iter().enumerate() {
        let oid: i64 = sqlx::query_scalar(
            "SELECT oid::pg_catalog.int8 FROM pg_catalog.pg_roles WHERE rolname = $1",
        )
        .bind(*name)
        .fetch_one(&pool)
        .await
        .expect("required role exists");
        role_oids[index] = u32::try_from(oid).expect("role OID fits u32");
    }
    let mut routine_oids = [0_u32; 3];
    for (index, name) in ROUTINE_QUERY_ORDER.iter().enumerate() {
        let oid: i64 = sqlx::query_scalar(
            "SELECT routine.oid::pg_catalog.int8 FROM pg_catalog.pg_proc AS routine \
             JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = routine.pronamespace \
             WHERE namespace.nspname = 'public' AND routine.proname = $1 \
             ORDER BY routine.oid LIMIT 1",
        )
        .bind(*name)
        .fetch_one(&pool)
        .await
        .expect("required version helper exists");
        routine_oids[index] = u32::try_from(oid).expect("routine OID fits u32");
    }
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
        Statement::Memberships {
            worker_oid: u32::try_from(worker_oid).expect("worker OID fits u32"),
        },
        Statement::Routines,
        Statement::Arguments,
        Statement::RawAcl,
        Statement::Effective {
            routine_oids,
            role_oids,
        },
        Statement::FinalContext,
    ];
    for statement in statements {
        let rows = transport.execute(statement).await.unwrap_or_else(|error| {
            panic!("real routine SQLx projection {statement:?}: {error:?}")
        });
        if matches!(
            statement,
            Statement::Context
                | Statement::Roles
                | Statement::Routines
                | Statement::Arguments
                | Statement::Effective { .. }
                | Statement::FinalContext
        ) {
            assert!(
                !rows.is_empty(),
                "empty routine PG projection: {statement:?}"
            );
        }
        assert!(
            rows.iter().all(|row| !row
                .cells
                .iter()
                .any(|cell| matches!(cell, RawCell::WrongType))),
            "wrong routine PG cell type: {statement:?}"
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
    pool.close().await;
    db.drop_db().await;
    eprintln!("R4_ROUTINE_PG:CATALOG_EFFECTIVE_AND_COMMANDS:PASS");
}

const VALID_CONFIG: &str = r##"{"schema":"owner-beta-simulation-process-v1","code_commit":"0123456789abcdef0123456789abcdef01234567","manifest_format":"LAGRANGE_RELEASE_MANIFEST_V3","manifest_sha256":"sha256:1111111111111111111111111111111111111111111111111111111111111111","image_id":"sha256:2222222222222222222222222222222222222222222222222222222222222222","receipt_sha256":"sha256:3333333333333333333333333333333333333333333333333333333333333333","binary_sha256":"sha256:4444444444444444444444444444444444444444444444444444444444444444","source_input_sha256":"sha256:5555555555555555555555555555555555555555555555555555555555555555","recipe_sha256":"sha256:6666666666666666666666666666666666666666666666666666666666666666","build_origin":"official-release-v1","package":"job-queue","bin":"owner-beta-simulation-runner","target":"x86_64-unknown-linux-musl","profile":"release","toolchain":"1.97.1","owner":"11111111-2222-3333-4444-555555555555","entitlement_reference":"fixture:synthetic-owner-rights-v1","entitlement_sha256":"sha256:7777777777777777777777777777777777777777777777777777777777777777","historical_root":"/synthetic/history/δ","reader_uid":10001,"reader_gid":10001,"paths":{"config":"/run/owner-beta-simulation/config.json","artifact_root":"/data/owner-beta-artifacts","receipt":"/usr/share/lagrange/owner-beta-simulation/artifact.json","complete":"/usr/share/lagrange/owner-beta-simulation/COMPLETE","executable":"/usr/local/bin/owner-beta-simulation-runner","worker_id":"owner-beta-simulation-runner"},"limits":{"memory_bytes":1234567,"nano_cpus":2345678,"health_tmpfs_bytes":3456789,"pids_limit":42,"startup_timeout_seconds":123,"health_max_age_seconds":234,"stop_timeout_seconds":345}}"##;
const TEST_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const TEST_WORKER_OID: u32 = 16_384;
const TEST_OWNER_OID: u32 = 16_385;
const TEST_APP_OID: u32 = 16_386;
const TEST_ADMIN_OID: u32 = 16_387;
const TEST_AUDIT_OID: u32 = 16_388;
const TEST_RESEARCH_OID: u32 = 16_389;
const TEST_DATABASE_OID: u32 = 16_390;
const TEST_SCHEMA_OID: u32 = 2_200;
const TEST_DATABASE_OWNER_OID: u32 = 16_391;
const TEST_SCHEMA_OWNER_OID: u32 = 16_392;
const TEST_PG_CATALOG_OID: u32 = 11;
const TEST_SQL_LANGUAGE_OID: u32 = 14;
const TEST_BOOL_OID: u32 = 16;
const FIXTURE_ROUTINE_QUERY_ORDER: [&str; 3] = [
    "owner_beta_simulation_version_binding_matches",
    "owner_beta_simulation_version_namespace_matches",
    "owner_beta_simulation_payload_matches",
];
const FIXTURE_ROUTINE_QUERY_RANKS: [(&str, u8); 3] = [
    ("owner_beta_simulation_version_binding_matches", 1),
    ("owner_beta_simulation_version_namespace_matches", 2),
    ("owner_beta_simulation_payload_matches", 3),
];
const EXPECTED_EFFECTIVE_ROUTINE_OIDS: [u32; 3] = [52_001, 52_002, 52_003];
const EXPECTED_EFFECTIVE_ROLE_OIDS: [u32; 6] = [16_384, 16_385, 16_386, 16_387, 16_388, 16_389];

fn valid_config() -> SimulationProcessConfig {
    let hash = Sha256::digest(VALID_CONFIG.as_bytes());
    let expected = format!("sha256:{hash:x}");
    SimulationProcessConfig::from_json(VALID_CONFIG.as_bytes(), TEST_COMMIT, &expected)
        .expect("synthetic checked config")
}

fn text(value: &str) -> RawCell {
    RawCell::Text {
        value: value.to_owned(),
        original_bytes: i64::try_from(value.len()).expect("fixture string length"),
    }
}

fn optional_text(value: Option<&str>) -> RawCell {
    value.map(text).unwrap_or(RawCell::Null)
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
            text("on"),
            text("origin"),
            bool_cell(true),
            text("repeatable read"),
            bool_cell(true),
            bool_cell(false),
            bool_cell(true),
            bool_cell(false),
            bool_cell(false),
            bool_cell(false),
            bool_cell(false),
            bool_cell(false),
        ],
    )
}

fn role_rows() -> Vec<RawRow> {
    let oids = [
        TEST_WORKER_OID,
        TEST_OWNER_OID,
        TEST_APP_OID,
        TEST_ADMIN_OID,
        TEST_AUDIT_OID,
        TEST_RESEARCH_OID,
    ];
    ROLE_NAMES
        .iter()
        .enumerate()
        .map(|(index, name)| {
            row(
                RawCategory::Role,
                vec![
                    i32_cell((index + 1) as i32),
                    text(name),
                    text(name),
                    i64_cell(i64::from(oids[index])),
                ],
            )
        })
        .collect()
}

fn routine_oid(name: &str) -> u32 {
    match name {
        "owner_beta_simulation_payload_matches" => 52_001,
        "owner_beta_simulation_version_binding_matches" => 52_002,
        "owner_beta_simulation_version_namespace_matches" => 52_003,
        _ => 0,
    }
}

fn type_oid(name: &str) -> u32 {
    match name {
        "bool" => TEST_BOOL_OID,
        "text" => 25,
        "jsonb" => 3_802,
        "uuid" => 2_950,
        "date" => 1_082,
        _ => 0,
    }
}

fn routine_header(spec: &RoutineSpec) -> RawRow {
    let mut cells = vec![
        text(spec.name),
        i64_cell(i64::from(routine_oid(spec.name))),
        i64_cell(i64::from(TEST_SCHEMA_OID)),
        text("public"),
        i64_cell(i64::from(TEST_OWNER_OID)),
        text("migration_owner"),
        text("f"),
        i64_cell(i64::from(TEST_SQL_LANGUAGE_OID)),
        text("sql"),
        i64_cell(i64::from(TEST_BOOL_OID)),
        i64_cell(i64::from(TEST_PG_CATALOG_OID)),
        text("pg_catalog"),
        text("bool"),
        bool_cell(false),
        text("i"),
        bool_cell(false),
        bool_cell(false),
        bool_cell(false),
        text("u"),
        i32_cell(spec.arguments.len() as i32),
        i32_cell(0),
        i64_cell(0),
        bool_cell(true),
        bool_cell(true),
        bool_cell(true),
        i32_cell(1),
        i32_cell(1),
        i32_cell(spec.arguments.len() as i32),
        i32_cell(1),
        i32_cell(1),
        i32_cell(1),
        text("search_path=pg_catalog"),
    ];
    assert_eq!(cells.len(), ROUTINE_SHAPE.len());
    row(RawCategory::Routine, std::mem::take(&mut cells))
}

fn routine_rows() -> Vec<RawRow> {
    FIXTURE_ROUTINE_QUERY_ORDER
        .iter()
        .map(|name| {
            let spec = spec_by_name(name).expect("known routine spec");
            routine_header(spec)
        })
        .collect()
}

fn argument_rows() -> Vec<RawRow> {
    let mut rows = Vec::with_capacity(ARGUMENT_ACCEPT_LIMIT);
    for name in FIXTURE_ROUTINE_QUERY_ORDER {
        let spec = spec_by_name(name).expect("known routine spec");
        for (index, argument) in spec.arguments.iter().enumerate() {
            let oid = type_oid(argument.type_name);
            rows.push(row(
                RawCategory::Argument,
                vec![
                    text(spec.name),
                    i64_cell(i64::from(routine_oid(spec.name))),
                    i32_cell((index + 1) as i32),
                    text(argument.name),
                    i64_cell(i64::from(oid)),
                    i64_cell(i64::from(oid)),
                    i64_cell(i64::from(TEST_PG_CATALOG_OID)),
                    text("pg_catalog"),
                    text(argument.type_name),
                ],
            ));
        }
    }
    rows
}

fn acl_row(routine_name: &str, grantee_oid: u32, grantee_name: &str) -> RawRow {
    row(
        RawCategory::RawAcl,
        vec![
            text(routine_name),
            i64_cell(i64::from(routine_oid(routine_name))),
            i64_cell(i64::from(grantee_oid)),
            text(grantee_name),
            i64_cell(i64::from(TEST_OWNER_OID)),
            text("migration_owner"),
            text("EXECUTE"),
            bool_cell(false),
        ],
    )
}

fn raw_acl_rows() -> Vec<RawRow> {
    vec![
        acl_row(
            "owner_beta_simulation_version_binding_matches",
            TEST_APP_OID,
            "app",
        ),
        acl_row(
            "owner_beta_simulation_version_binding_matches",
            TEST_WORKER_OID,
            "worker",
        ),
    ]
}

fn effective_rows() -> Vec<RawRow> {
    let role_oids = [
        TEST_WORKER_OID,
        TEST_OWNER_OID,
        TEST_APP_OID,
        TEST_ADMIN_OID,
        TEST_AUDIT_OID,
        TEST_RESEARCH_OID,
    ];
    let mut rows = Vec::with_capacity(EFFECTIVE_ACCEPT_LIMIT);
    for spec in ROUTINE_SPECS {
        for (index, role_name) in ROLE_NAMES.iter().enumerate() {
            let can_execute = *role_name == MIGRATION_OWNER_ROLE
                || (spec.name == "owner_beta_simulation_version_binding_matches"
                    && (*role_name == "app" || *role_name == WORKER_ROLE));
            rows.push(row(
                RawCategory::Effective,
                vec![
                    text(spec.name),
                    i64_cell(i64::from(routine_oid(spec.name))),
                    text(role_name),
                    i64_cell(i64::from(role_oids[index])),
                    bool_cell(can_execute),
                    bool_cell(*role_name == MIGRATION_OWNER_ROLE),
                ],
            ));
        }
    }
    rows
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum StatementKind {
    Begin,
    Configure,
    Context,
    Roles,
    Memberships,
    Routines,
    Arguments,
    RawAcl,
    Effective,
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
            Self::Memberships { .. } => StatementKind::Memberships,
            Self::Routines => StatementKind::Routines,
            Self::Arguments => StatementKind::Arguments,
            Self::RawAcl => StatementKind::RawAcl,
            Self::Effective { .. } => StatementKind::Effective,
            Self::FinalContext => StatementKind::FinalContext,
            Self::Commit => StatementKind::Commit,
            Self::Rollback => StatementKind::Rollback,
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
    events: Arc<Mutex<Vec<Event>>>,
    canceled: Arc<AtomicBool>,
    armed: bool,
    context: RawRow,
    final_context: RawRow,
    context_overflow: bool,
    roles: Vec<RawRow>,
    memberships: Vec<RawRow>,
    routines: Vec<RawRow>,
    arguments: Vec<RawRow>,
    raw_acl: Vec<RawRow>,
    effective: Vec<RawRow>,
    context_reads: usize,
    fail_at: Option<Point>,
    cancel_at: Option<Point>,
    pending_at: Option<StatementKind>,
    unexpected_at: Option<StatementKind>,
    close_fails: bool,
}

impl FakeTransport {
    fn valid() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
            canceled: Arc::new(AtomicBool::new(false)),
            armed: false,
            context: context_row(),
            final_context: context_row(),
            context_overflow: false,
            roles: role_rows(),
            memberships: Vec::new(),
            routines: routine_rows(),
            arguments: argument_rows(),
            raw_acl: raw_acl_rows(),
            effective: effective_rows(),
            context_reads: 0,
            fail_at: None,
            cancel_at: None,
            pending_at: None,
            unexpected_at: None,
            close_fails: false,
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
}

#[async_trait]
impl ObserverTransport for FakeTransport {
    async fn acquire(&mut self) -> Result<(), TransportFailure> {
        self.events.lock().expect("event log").push(Event::Acquire);
        self.cancel(Point::Acquire);
        if self.fail_at == Some(Point::Acquire) {
            return Err(TransportFailure::Database);
        }
        Ok(())
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
            let _pending_drop = PendingDrop {
                events: Arc::clone(&self.events),
                armed: self.armed,
            };
            std::future::pending::<()>().await;
            return Err(TransportFailure::State);
        }
        if self.fail_at == Some(Point::Statement(kind)) {
            return Err(TransportFailure::Database);
        }
        if matches!(
            statement,
            Statement::Begin | Statement::Commit | Statement::Rollback
        ) {
            return Ok(super::classify_command_result(
                (self.unexpected_at == Some(kind)).then_some(()),
            ));
        }
        if self.unexpected_at == Some(kind) {
            return Ok(vec![row(RawCategory::Context, vec![RawCell::WrongType])]);
        }
        match statement {
            Statement::Begin
            | Statement::Configure { .. }
            | Statement::Commit
            | Statement::Rollback => Ok(Vec::new()),
            Statement::Context => {
                self.context_reads += 1;
                let mut rows = vec![self.context.clone()];
                if self.context_overflow {
                    rows.push(self.context.clone());
                }
                Ok(rows)
            }
            Statement::Roles => Ok(self.roles.clone()),
            Statement::Memberships { .. } => Ok(self.memberships.clone()),
            Statement::Routines => Ok(self.routines.clone()),
            Statement::Arguments => Ok(self.arguments.clone()),
            Statement::RawAcl => Ok(self.raw_acl.clone()),
            Statement::Effective { .. } => Ok(self.effective.clone()),
            Statement::FinalContext => Ok(vec![self.final_context.clone()]),
        }
    }

    async fn close(&mut self) -> Result<(), TransportFailure> {
        self.events.lock().expect("event log").push(Event::Close);
        self.cancel(Point::Close);
        if self.close_fails || self.fail_at == Some(Point::Close) {
            Err(TransportFailure::Database)
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
) -> Result<SimulationVersionHelperMetadataObservation, SimulationVersionHelperMetadataError> {
    let config = valid_config();
    let canceled = Arc::clone(&fake.canceled);
    block_on(observe_with_transport(fake, &config, &canceled))
}

fn assert_no_observation(fake: &mut FakeTransport) {
    assert!(observe(fake).is_err());
    assert!(fake.event_log().contains(&Event::Close));
}

fn assert_error(fake: &mut FakeTransport, expected: SimulationVersionHelperMetadataError) {
    match observe(fake) {
        Err(actual) => assert_eq!(actual, expected),
        Ok(_) => panic!("unexpected metadata observation"),
    }
}

fn replace_cell(row: &mut RawRow, index: usize, value: RawCell) {
    row.cells[index] = value;
}

fn statement_seen(events: &[Event], kind: StatementKind) -> bool {
    events
        .iter()
        .any(|event| matches!(event, Event::Statement(statement) if statement.kind() == kind))
}

fn statements_seen(events: &[Event]) -> Vec<Statement> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::Statement(statement) => Some(*statement),
            _ => None,
        })
        .collect()
}

#[test]
fn good_control_uses_the_shared_raw_decoder_and_returns_only_metadata() {
    let mut fake = FakeTransport::valid();
    let observed = observe(&mut fake).expect("synthetic metadata control");
    assert_eq!(observed.config_sha256(), valid_config().config_sha256());
    assert_eq!(observed.policy_id(), POLICY_ID);
    assert_eq!(observed.policy_fingerprint(), POLICY_FINGERPRINT);
    assert_eq!(observed.database_oid(), TEST_DATABASE_OID);
    assert_eq!(observed.worker_oid(), TEST_WORKER_OID);
    assert_eq!(observed.public_schema_oid(), TEST_SCHEMA_OID);
    assert_eq!(observed.migration_owner_oid(), TEST_OWNER_OID);
    assert_eq!(
        observed.routine_oids(),
        &[
            routine_oid("owner_beta_simulation_payload_matches"),
            routine_oid("owner_beta_simulation_version_binding_matches"),
            routine_oid("owner_beta_simulation_version_namespace_matches"),
        ]
    );
    let events = fake.event_log();
    assert_eq!(events.first(), Some(&Event::Acquire));
    assert_eq!(events.get(1), Some(&Event::Armed));
    assert_eq!(events.last(), Some(&Event::Close));
    assert!(statement_seen(&events, StatementKind::Commit));
    assert!(!statement_seen(&events, StatementKind::Rollback));
    assert_eq!(fake.context_reads, 1);
}

#[test]
fn statement_and_sqlx_bind_values_match_independent_expected_values() {
    let mut fake = FakeTransport::valid();
    assert!(observe(&mut fake).is_ok());
    let statements = statements_seen(&fake.event_log());

    assert!(statements.contains(&Statement::Configure {
        timeout_ms: 123_000,
    }));
    assert!(statements.contains(&Statement::Memberships { worker_oid: 16_384 }));
    assert!(statements.contains(&Statement::Effective {
        routine_oids: [52_001, 52_002, 52_003],
        role_oids: [16_384, 16_385, 16_386, 16_387, 16_388, 16_389],
    }));

    assert_eq!(timeout_bind_value(123_000), "123000");
    assert_eq!(membership_worker_bind_value(16_384), 16_384);
    assert_eq!(
        effective_bind_values(
            EXPECTED_EFFECTIVE_ROUTINE_OIDS,
            EXPECTED_EFFECTIVE_ROLE_OIDS,
        ),
        [
            52_001, 52_002, 52_003, 16_384, 16_385, 16_386, 16_387, 16_388, 16_389,
        ]
    );
}

#[test]
fn routine_sql_rank_matches_independent_literal_fixture_order() {
    assert_eq!(
        FIXTURE_ROUTINE_QUERY_ORDER,
        [
            "owner_beta_simulation_version_binding_matches",
            "owner_beta_simulation_version_namespace_matches",
            "owner_beta_simulation_payload_matches",
        ]
    );
    assert_eq!(ROUTINE_QUERY_ORDER, FIXTURE_ROUTINE_QUERY_ORDER);

    assert!(ROUTINES_SQL.contains("ORDER BY CASE routine_name"));
    assert!(ARGUMENTS_SQL.contains("ORDER BY CASE p.proname"));
    for (name, rank) in FIXTURE_ROUTINE_QUERY_RANKS {
        let ranked = format!("WHEN '{name}' THEN {rank}");
        assert!(ROUTINES_SQL.contains(&ranked));
        assert!(ARGUMENTS_SQL.contains(&ranked));
    }
    assert!(ROUTINES_SQL.contains("          routine_oid\n LIMIT 4"));
    assert!(ARGUMENTS_SQL.contains("          p.oid, argument.ordinality\n LIMIT 26"));
}

#[test]
fn timeout_uses_only_checked_startup_seconds() {
    assert_eq!(timeout_millis(1), Ok(1_000));
    assert_eq!(timeout_millis(123), Ok(123_000));
    assert_eq!(
        timeout_millis(0),
        Err(SimulationVersionHelperMetadataError::InvalidBound)
    );
    assert_eq!(
        timeout_millis(u64::MAX),
        Err(SimulationVersionHelperMetadataError::InvalidBound)
    );
    assert_eq!(
        timeout_millis((i32::MAX as u64) / 1000 + 1),
        Err(SimulationVersionHelperMetadataError::InvalidBound)
    );
}

#[test]
fn positive_oid_rejects_zero_negative_and_values_above_u32() {
    assert_eq!(positive_oid(1), Ok(1));
    assert_eq!(positive_oid(i64::from(u32::MAX)), Ok(u32::MAX));
    assert_eq!(
        positive_oid(0),
        Err(SimulationVersionHelperMetadataError::CatalogInvalid)
    );
    assert_eq!(
        positive_oid(-1),
        Err(SimulationVersionHelperMetadataError::CatalogInvalid)
    );
    assert_eq!(
        positive_oid(i64::from(u32::MAX) + 1),
        Err(SimulationVersionHelperMetadataError::CatalogInvalid)
    );
    assert_eq!(
        positive_oid(i64::MAX),
        Err(SimulationVersionHelperMetadataError::CatalogInvalid)
    );
}

#[test]
fn worker_context_policy_and_final_identity_are_checked() {
    for (index, value) in [
        (0, text("admin")),
        (1, text("admin")),
        (11, bool_cell(false)),
        (12, bool_cell(true)),
        (13, bool_cell(true)),
        (14, bool_cell(true)),
        (15, bool_cell(true)),
        (16, bool_cell(true)),
        (17, text("off")),
        (18, text("replica")),
        (19, bool_cell(false)),
        (20, text("read committed")),
        (21, bool_cell(false)),
        (22, bool_cell(true)),
        (23, bool_cell(false)),
        (24, bool_cell(true)),
        (25, bool_cell(true)),
        (26, bool_cell(true)),
        (27, bool_cell(true)),
        (28, bool_cell(true)),
    ] {
        let mut fake = FakeTransport::valid();
        replace_cell(&mut fake.context, index, value);
        assert_no_observation(&mut fake);
        assert!(!statement_seen(&fake.event_log(), StatementKind::Roles));
    }

    for index in [5, 6, 7, 8, 9, 10] {
        let mut fake = FakeTransport::valid();
        replace_cell(&mut fake.context, index, i64_cell(0));
        assert_no_observation(&mut fake);
        assert!(!statement_seen(&fake.event_log(), StatementKind::Roles));
    }

    let mut changed_database = FakeTransport::valid();
    replace_cell(&mut changed_database.final_context, 6, i64_cell(99_999));
    assert_no_observation(&mut changed_database);
    assert!(!statement_seen(
        &changed_database.event_log(),
        StatementKind::Commit
    ));

    let mut changed_schema_owner = FakeTransport::valid();
    replace_cell(
        &mut changed_schema_owner.final_context,
        10,
        i64_cell(99_998),
    );
    assert_no_observation(&mut changed_schema_owner);
    assert!(!statement_seen(
        &changed_schema_owner.event_log(),
        StatementKind::Commit
    ));
}

#[test]
fn exactly_six_distinct_resolved_role_names_and_oids_are_required() {
    let mut missing = FakeTransport::valid();
    missing.roles.pop();
    assert_no_observation(&mut missing);

    let mut duplicate = FakeTransport::valid();
    duplicate.roles[5].cells[3] = i64_cell(i64::from(TEST_AUDIT_OID));
    assert_no_observation(&mut duplicate);

    let mut wrong_name = FakeTransport::valid();
    wrong_name.roles[2].cells[2] = text("someone_else");
    assert_no_observation(&mut wrong_name);

    let mut null_name = FakeTransport::valid();
    null_name.roles[2].cells[2] = RawCell::Null;
    assert_no_observation(&mut null_name);

    let mut outgoing_membership = FakeTransport::valid();
    outgoing_membership.memberships.push(row(
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
    assert_no_observation(&mut outgoing_membership);
    assert!(!statement_seen(
        &outgoing_membership.event_log(),
        StatementKind::Routines
    ));
}

#[test]
fn public_routine_overload_and_header_metadata_mutations_fail_before_arguments() {
    let mut missing = FakeTransport::valid();
    missing.routines.pop();
    assert_no_observation(&mut missing);
    assert!(!statement_seen(
        &missing.event_log(),
        StatementKind::Arguments
    ));

    let mut duplicate_name = FakeTransport::valid();
    duplicate_name.routines[1].cells[0] = text("owner_beta_simulation_version_binding_matches");
    assert_no_observation(&mut duplicate_name);
    assert!(!statement_seen(
        &duplicate_name.event_log(),
        StatementKind::Arguments
    ));

    let mut wrong_name = FakeTransport::valid();
    wrong_name.routines[0].cells[0] = text("owner_beta_simulation_wrong_matches");
    assert_no_observation(&mut wrong_name);
    assert!(!statement_seen(
        &wrong_name.event_log(),
        StatementKind::Arguments
    ));

    let mut changed_language_oid = FakeTransport::valid();
    replace_cell(&mut changed_language_oid.routines[1], 7, i64_cell(900));
    assert_no_observation(&mut changed_language_oid);
    assert!(!statement_seen(
        &changed_language_oid.event_log(),
        StatementKind::Arguments
    ));

    let mut changed_return_oid = FakeTransport::valid();
    replace_cell(&mut changed_return_oid.routines[1], 9, i64_cell(901));
    assert_no_observation(&mut changed_return_oid);
    assert!(!statement_seen(
        &changed_return_oid.event_log(),
        StatementKind::Arguments
    ));

    let mut fourth_overload = FakeTransport::valid();
    fourth_overload.routines.push(routine_header(
        spec_by_name("owner_beta_simulation_payload_matches").expect("spec"),
    ));
    assert_error(
        &mut fourth_overload,
        SimulationVersionHelperMetadataError::BoundExceeded,
    );
    assert!(!statement_seen(
        &fourth_overload.event_log(),
        StatementKind::Arguments
    ));

    let mutations = [
        (3, text("other_schema")),
        (4, i64_cell(99_001)),
        (5, text("other_owner")),
        (6, text("p")),
        (7, i64_cell(-1)),
        (8, text("plpgsql")),
        (9, i64_cell(99_002)),
        (10, i64_cell(99_003)),
        (11, text("other_catalog")),
        (12, text("text")),
        (13, bool_cell(true)),
        (14, text("s")),
        (15, bool_cell(true)),
        (16, bool_cell(true)),
        (17, bool_cell(true)),
        (18, text("s")),
        (19, i32_cell(15)),
        (20, i32_cell(1)),
        (21, i64_cell(25)),
        (22, bool_cell(false)),
        (23, bool_cell(false)),
        (24, bool_cell(false)),
        (25, i32_cell(2)),
        (26, i32_cell(0)),
        (27, i32_cell(15)),
        (28, i32_cell(2)),
        (29, i32_cell(0)),
        (30, i32_cell(2)),
        (31, text("search_path=public")),
    ];
    for (index, value) in mutations {
        let mut fake = FakeTransport::valid();
        replace_cell(&mut fake.routines[0], index, value);
        assert_no_observation(&mut fake);
        assert!(!statement_seen(&fake.event_log(), StatementKind::Arguments));
    }

    let mut null_argnames = FakeTransport::valid();
    replace_cell(&mut null_argnames.routines[0], 25, RawCell::Null);
    assert_no_observation(&mut null_argnames);
    let mut empty_argnames = FakeTransport::valid();
    replace_cell(&mut empty_argnames.routines[0], 25, i32_cell(0));
    assert_no_observation(&mut empty_argnames);
    let mut null_config = FakeTransport::valid();
    replace_cell(&mut null_config.routines[0], 31, RawCell::Null);
    assert_no_observation(&mut null_config);
    let mut empty_config = FakeTransport::valid();
    replace_cell(&mut empty_config.routines[0], 31, text(""));
    assert_no_observation(&mut empty_config);
}

#[test]
fn ordered_named_argument_types_namespace_and_counts_are_checked() {
    for (index, value) in [
        (0, text("other_routine")),
        (1, i64_cell(99_000)),
        (2, i32_cell(0)),
        (3, text("other_argument")),
        (4, i64_cell(0)),
        (5, i64_cell(99_000)),
        (6, i64_cell(99_000)),
        (7, text("other_catalog")),
        (8, text("int4")),
    ] {
        let mut fake = FakeTransport::valid();
        replace_cell(&mut fake.arguments[0], index, value);
        assert_no_observation(&mut fake);
        assert!(!statement_seen(&fake.event_log(), StatementKind::RawAcl));
    }

    let mut missing = FakeTransport::valid();
    missing.arguments.pop();
    assert_no_observation(&mut missing);
    let mut twenty_sixth = FakeTransport::valid();
    twenty_sixth
        .arguments
        .push(twenty_sixth.arguments[0].clone());
    assert_error(
        &mut twenty_sixth,
        SimulationVersionHelperMetadataError::BoundExceeded,
    );

    let mut too_many_one_routine = FakeTransport::valid();
    too_many_one_routine.arguments[5].cells[2] = i32_cell(7);
    assert_no_observation(&mut too_many_one_routine);
    assert_eq!(ARGUMENTS_PER_ROUTINE_LIMIT, 16);
    assert_eq!(ARGUMENT_ACCEPT_LIMIT, 25);
    assert_eq!(ARGUMENT_FETCH_LIMIT, 26);
}

#[test]
fn every_fixed_row_cap_fetches_one_overflow_sentinel_and_rejects_it() {
    let mut context = FakeTransport::valid();
    context.context_overflow = true;
    assert_error(
        &mut context,
        SimulationVersionHelperMetadataError::BoundExceeded,
    );

    let mut roles = FakeTransport::valid();
    roles.roles.push(roles.roles[0].clone());
    assert_error(
        &mut roles,
        SimulationVersionHelperMetadataError::BoundExceeded,
    );

    let mut membership = FakeTransport::valid();
    membership.memberships.push(row(
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
    assert_error(
        &mut membership,
        SimulationVersionHelperMetadataError::BoundExceeded,
    );

    let mut routines = FakeTransport::valid();
    routines.routines.push(routines.routines[0].clone());
    assert_error(
        &mut routines,
        SimulationVersionHelperMetadataError::BoundExceeded,
    );

    let mut arguments = FakeTransport::valid();
    arguments.arguments.push(arguments.arguments[0].clone());
    assert_error(
        &mut arguments,
        SimulationVersionHelperMetadataError::BoundExceeded,
    );

    let mut acl = FakeTransport::valid();
    acl.raw_acl = (0..RAW_ACL_FETCH_LIMIT)
        .map(|_| {
            acl_row(
                "owner_beta_simulation_version_binding_matches",
                TEST_APP_OID,
                "app",
            )
        })
        .collect();
    assert_error(
        &mut acl,
        SimulationVersionHelperMetadataError::BoundExceeded,
    );

    let mut effective = FakeTransport::valid();
    effective.effective.push(effective.effective[0].clone());
    assert_error(
        &mut effective,
        SimulationVersionHelperMetadataError::BoundExceeded,
    );

    assert_eq!((CONTEXT_ACCEPT_LIMIT, CONTEXT_FETCH_LIMIT), (1, 2));
    assert_eq!((ROLE_ACCEPT_LIMIT, ROLE_FETCH_LIMIT), (6, 7));
    assert_eq!((MEMBERSHIP_ACCEPT_LIMIT, MEMBERSHIP_FETCH_LIMIT), (0, 1));
    assert_eq!((ROUTINE_ACCEPT_LIMIT, ROUTINE_FETCH_LIMIT), (3, 4));
    assert_eq!((ARGUMENT_ACCEPT_LIMIT, ARGUMENT_FETCH_LIMIT), (25, 26));
    assert_eq!((RAW_ACL_ACCEPT_LIMIT, RAW_ACL_FETCH_LIMIT), (64, 65));
    assert_eq!((EFFECTIVE_ACCEPT_LIMIT, EFFECTIVE_FETCH_LIMIT), (18, 19));
}

#[test]
fn raw_acl_rejects_public_unknown_grantor_unknown_grantee_extra_privileges_and_grant_options() {
    let mut public = FakeTransport::valid();
    replace_cell(&mut public.raw_acl[0], 2, i64_cell(0));
    replace_cell(&mut public.raw_acl[0], 3, RawCell::Null);
    assert_no_observation(&mut public);
    assert!(!statement_seen(
        &public.event_log(),
        StatementKind::Effective
    ));

    let mut unknown_grantee = FakeTransport::valid();
    replace_cell(&mut unknown_grantee.raw_acl[0], 2, i64_cell(99_999));
    replace_cell(&mut unknown_grantee.raw_acl[0], 3, text("outsider"));
    assert_no_observation(&mut unknown_grantee);

    let mut unknown_grantor = FakeTransport::valid();
    replace_cell(&mut unknown_grantor.raw_acl[0], 4, i64_cell(99_998));
    replace_cell(&mut unknown_grantor.raw_acl[0], 5, text("outsider"));
    assert_no_observation(&mut unknown_grantor);

    let mut wrong_privilege = FakeTransport::valid();
    replace_cell(&mut wrong_privilege.raw_acl[0], 6, text("SELECT"));
    assert_no_observation(&mut wrong_privilege);

    let mut grantable = FakeTransport::valid();
    replace_cell(&mut grantable.raw_acl[0], 7, bool_cell(true));
    assert_no_observation(&mut grantable);

    for missing_index in [0, 1] {
        let mut missing_direct_allowance = FakeTransport::valid();
        missing_direct_allowance.raw_acl.remove(missing_index);
        assert_no_observation(&mut missing_direct_allowance);
        assert!(!statement_seen(
            &missing_direct_allowance.event_log(),
            StatementKind::Effective
        ));
    }

    let mut duplicate = FakeTransport::valid();
    duplicate.raw_acl.push(duplicate.raw_acl[0].clone());
    assert_no_observation(&mut duplicate);

    let mut owner_self_entry = FakeTransport::valid();
    owner_self_entry.raw_acl.push(acl_row(
        "owner_beta_simulation_version_binding_matches",
        TEST_OWNER_OID,
        "migration_owner",
    ));
    assert!(observe(&mut owner_self_entry).is_ok());

    let mut raw_acl_sentinel = FakeTransport::valid();
    raw_acl_sentinel.raw_acl = (0..RAW_ACL_FETCH_LIMIT)
        .map(|_| {
            acl_row(
                "owner_beta_simulation_version_binding_matches",
                TEST_APP_OID,
                "app",
            )
        })
        .collect();
    assert_error(
        &mut raw_acl_sentinel,
        SimulationVersionHelperMetadataError::BoundExceeded,
    );
}

#[test]
fn effective_execute_and_grant_option_are_independently_enforced_for_all_eighteen_pairs() {
    let valid = FakeTransport::valid();
    let mut owner_authority = valid;
    let owner_payload_index = 1;
    replace_cell(
        &mut owner_authority.effective[owner_payload_index],
        5,
        bool_cell(true),
    );
    assert!(observe(&mut owner_authority).is_ok());

    for index in 0..EFFECTIVE_ACCEPT_LIMIT {
        let routine_name = ROUTINE_SPECS[index / ROLE_NAMES.len()].name;
        let role_name = ROLE_NAMES[index % ROLE_NAMES.len()];
        let should_execute = role_name == MIGRATION_OWNER_ROLE
            || (routine_name == "owner_beta_simulation_version_binding_matches"
                && (role_name == "app" || role_name == WORKER_ROLE));
        if !should_execute {
            let mut fake = FakeTransport::valid();
            replace_cell(&mut fake.effective[index], 4, bool_cell(true));
            assert_no_observation(&mut fake);
            assert!(!statement_seen(
                &fake.event_log(),
                StatementKind::FinalContext
            ));
        }
        if role_name != MIGRATION_OWNER_ROLE {
            let mut fake = FakeTransport::valid();
            replace_cell(&mut fake.effective[index], 5, bool_cell(true));
            assert_no_observation(&mut fake);
            assert!(!statement_seen(
                &fake.event_log(),
                StatementKind::FinalContext
            ));
        }
    }

    let mut owner_missing = FakeTransport::valid();
    replace_cell(&mut owner_missing.effective[1], 4, bool_cell(false));
    assert_no_observation(&mut owner_missing);
    assert_eq!(EFFECTIVE_ACCEPT_LIMIT, 18);
    assert_eq!(EFFECTIVE_FETCH_LIMIT, 19);
}

#[test]
fn required_binding_execute_bits_must_remain_enabled() {
    for role_name in ["worker", "app"] {
        let mut fake = FakeTransport::valid();
        let index = fake
            .effective
            .iter()
            .position(|row| {
                row.cells[0] == text("owner_beta_simulation_version_binding_matches")
                    && row.cells[2] == text(role_name)
            })
            .expect("required binding execution pair");
        replace_cell(&mut fake.effective[index], 4, bool_cell(false));
        assert_no_observation(&mut fake);
        assert!(!statement_seen(
            &fake.event_log(),
            StatementKind::FinalContext
        ));
    }
}

#[test]
fn row_text_and_cumulative_payload_caps_include_sentinels_and_checked_addition() {
    let mut budget = LogicalPayloadBudget::default();
    let accepted = "a".repeat(TEXT_ACCEPT_BYTES);
    let mut decoder = RawDecoder::new(
        row(RawCategory::Role, vec![text(&accepted)]),
        RawCategory::Role,
        1,
        &mut budget,
    );
    assert_eq!(
        decoder
            .as_mut()
            .expect("decoder")
            .text()
            .expect("1024 bytes")
            .len(),
        1024
    );

    let over = "b".repeat(TEXT_FETCH_BYTES);
    let mut budget = LogicalPayloadBudget::default();
    let mut decoder = RawDecoder::new(
        row(
            RawCategory::Role,
            vec![RawCell::Text {
                value: over.clone(),
                original_bytes: TEXT_FETCH_BYTES as i64,
            }],
        ),
        RawCategory::Role,
        1,
        &mut budget,
    )
    .expect("decoder shape");
    assert_eq!(
        decoder.text(),
        Err(SimulationVersionHelperMetadataError::BoundExceeded)
    );

    let mut truncated_budget = LogicalPayloadBudget::default();
    let mut truncated = RawDecoder::new(
        row(
            RawCategory::Role,
            vec![RawCell::Text {
                value: "c".repeat(256),
                original_bytes: 1_026,
            }],
        ),
        RawCategory::Role,
        1,
        &mut truncated_budget,
    )
    .expect("decoder shape");
    assert_eq!(
        truncated.text(),
        Err(SimulationVersionHelperMetadataError::BoundExceeded)
    );

    let multibyte = "🧠".repeat(256);
    assert_eq!(multibyte.len(), 1024);
    let mut budget = LogicalPayloadBudget::default();
    let mut decoder = RawDecoder::new(
        row(RawCategory::Role, vec![text(&multibyte)]),
        RawCategory::Role,
        1,
        &mut budget,
    )
    .expect("decoder shape");
    assert_eq!(decoder.text().expect("UTF-8 byte boundary").len(), 1024);

    let mut budget = LogicalPayloadBudget {
        used: LOGICAL_PAYLOAD_LIMIT - 8,
    };
    assert_eq!(budget.charge(&RawCell::I64(7)), Ok(()));
    assert_eq!(budget.used, LOGICAL_PAYLOAD_LIMIT);
    assert_eq!(
        budget.charge(&RawCell::Bool(true)),
        Err(SimulationVersionHelperMetadataError::PayloadExceeded)
    );
    let mut overflow = LogicalPayloadBudget { used: usize::MAX };
    assert_eq!(
        overflow.charge(&RawCell::I32(1)),
        Err(SimulationVersionHelperMetadataError::PayloadExceeded)
    );

    let mut null_budget = LogicalPayloadBudget::default();
    assert_eq!(null_budget.charge(&RawCell::Null), Ok(()));
    assert_eq!(null_budget.used, 1);
    assert_eq!(TEXT_ACCEPT_BYTES, 1024);
    assert_eq!(TEXT_FETCH_BYTES, 1025);
}

#[test]
fn malformed_raw_types_and_field_counts_fail_closed() {
    let mut wrong_type = FakeTransport::valid();
    replace_cell(&mut wrong_type.context, 5, text("not-an-oid"));
    assert_no_observation(&mut wrong_type);

    let mut malformed_context = FakeTransport::valid();
    malformed_context.context.cells.pop();
    assert_no_observation(&mut malformed_context);

    let mut wrong_category = FakeTransport::valid();
    wrong_category.roles[0].category = RawCategory::Context;
    assert_no_observation(&mut wrong_category);

    let mut null_required = FakeTransport::valid();
    replace_cell(&mut null_required.routines[0], 8, RawCell::Null);
    assert_no_observation(&mut null_required);
}

fn is_sql_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$'
}

fn find_top_level_keyword(sql: &str, keyword: &[u8], minimum: usize) -> Option<usize> {
    let bytes = sql.as_bytes();
    let mut index = 0;
    let mut depth = 0usize;
    let mut in_string = false;
    while index < bytes.len() {
        if in_string {
            if bytes[index] == b'\'' {
                if bytes.get(index + 1) == Some(&b'\'') {
                    index += 2;
                    continue;
                }
                in_string = false;
            }
            index += 1;
            continue;
        }
        match bytes[index] {
            b'\'' => {
                in_string = true;
                index += 1;
            }
            b'(' => {
                depth += 1;
                index += 1;
            }
            b')' => {
                depth = depth.saturating_sub(1);
                index += 1;
            }
            _ => {
                let end = index.saturating_add(keyword.len());
                let matches = depth == 0
                    && index >= minimum
                    && bytes
                        .get(index..end)
                        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(keyword))
                    && (index == 0 || !is_sql_identifier_byte(bytes[index - 1]))
                    && (end == bytes.len() || !is_sql_identifier_byte(bytes[end]));
                if matches {
                    return Some(index);
                }
                index += 1;
            }
        }
    }
    None
}

fn split_top_level_projection(projection: &str) -> Option<Vec<&str>> {
    let bytes = projection.as_bytes();
    let mut columns = Vec::new();
    let mut start = 0;
    let mut index = 0;
    let mut depth = 0usize;
    let mut in_string = false;
    while index < bytes.len() {
        if in_string {
            if bytes[index] == b'\'' {
                if bytes.get(index + 1) == Some(&b'\'') {
                    index += 2;
                    continue;
                }
                in_string = false;
            }
            index += 1;
            continue;
        }
        match bytes[index] {
            b'\'' => {
                in_string = true;
                index += 1;
            }
            b'(' => {
                depth += 1;
                index += 1;
            }
            b')' => {
                depth = depth.saturating_sub(1);
                index += 1;
            }
            b',' if depth == 0 => {
                columns.push(projection[start..index].trim());
                start = index + 1;
                index += 1;
            }
            _ => index += 1,
        }
    }
    if in_string || depth != 0 {
        return None;
    }
    columns.push(projection[start..].trim());
    columns
        .iter()
        .all(|column| !column.is_empty())
        .then_some(columns)
}

fn outer_projection(sql: &str) -> Option<Vec<&str>> {
    let select = find_top_level_keyword(sql, b"SELECT", 0)?;
    let projection_start = select + b"SELECT".len();
    let from = find_top_level_keyword(sql, b"FROM", projection_start)?;
    split_top_level_projection(&sql[projection_start..from])
}

fn expected_projection(shape: &[PgCellKind]) -> Vec<(String, &'static str)> {
    let mut expected = Vec::new();
    for kind in shape {
        match kind {
            PgCellKind::Bool(alias) => expected.push(((*alias).to_owned(), "bool")),
            PgCellKind::I32(alias) | PgCellKind::OptionalI32(alias) => {
                expected.push(((*alias).to_owned(), "int4"));
            }
            PgCellKind::I64(alias) => expected.push(((*alias).to_owned(), "int8")),
            PgCellKind::Text {
                bytes_alias,
                value_alias,
            } => {
                expected.push(((*bytes_alias).to_owned(), "int8"));
                expected.push(((*value_alias).to_owned(), "text"));
            }
        }
    }
    expected
}

fn projection_matches(sql: &str, shape: &[PgCellKind]) -> bool {
    let Some(columns) = outer_projection(sql) else {
        return false;
    };
    let expected = expected_projection(shape);
    if columns.len() != expected.len() {
        return false;
    }
    columns
        .iter()
        .zip(expected)
        .all(|(column, (alias, sql_type))| {
            let Some((expression, actual_alias)) = column.rsplit_once(" AS ") else {
                return false;
            };
            let cast = format!("::pg_catalog.{sql_type}");
            actual_alias.trim() == alias.as_str() && expression.trim_end().ends_with(&cast)
        })
}

#[test]
fn sql_projection_aliases_types_order_caps_and_body_exclusion_are_fixed() {
    assert!(projection_matches(CONTEXT_SQL, &CONTEXT_SHAPE));
    assert!(projection_matches(ROLES_SQL, &ROLE_SHAPE));
    assert!(projection_matches(MEMBERSHIPS_SQL, &MEMBERSHIP_SHAPE));
    assert!(projection_matches(ROUTINES_SQL, &ROUTINE_SHAPE));
    assert!(projection_matches(ARGUMENTS_SQL, &ARGUMENT_SHAPE));
    assert!(projection_matches(RAW_ACL_SQL, &RAW_ACL_SHAPE));
    assert!(projection_matches(
        EFFECTIVE_PRIVILEGES_SQL,
        &EFFECTIVE_SHAPE
    ));

    assert_eq!(pg_column_count(&CONTEXT_SHAPE), 37);
    assert_eq!(pg_column_count(&ROLE_SHAPE), 6);
    assert_eq!(pg_column_count(&ROUTINE_SHAPE), 42);
    assert_eq!(pg_column_count(&ARGUMENT_SHAPE), 13);
    assert_eq!(pg_column_count(&RAW_ACL_SHAPE), 12);
    assert_eq!(pg_column_count(&EFFECTIVE_SHAPE), 8);
    assert!(CONTEXT_SQL.contains("LIMIT 2"));
    assert!(ROLES_SQL.contains("LIMIT 7"));
    assert!(MEMBERSHIPS_SQL.contains("LIMIT 1"));
    assert!(ROUTINES_SQL.contains("LIMIT 4"));
    assert!(ARGUMENTS_SQL.contains("LIMIT 26"));
    assert!(RAW_ACL_SQL.contains("LIMIT 65"));
    assert!(EFFECTIVE_PRIVILEGES_SQL.contains("LIMIT 19"));
    assert!(ARGUMENTS_SQL.contains("owner_beta_simulation_version_binding_matches"));
    assert!(MEMBERSHIPS_SQL.contains("$1::pg_catalog.oid"));
    assert!(SETUP_SQL.contains("$1::pg_catalog.text"));
    for bind in 1..=9 {
        assert!(EFFECTIVE_PRIVILEGES_SQL.contains(&format!("${bind}::pg_catalog.oid")));
    }
    assert!(EFFECTIVE_PRIVILEGES_SQL.contains("'EXECUTE WITH GRANT OPTION'"));

    let sql = [
        CONTEXT_SQL,
        ROLES_SQL,
        MEMBERSHIPS_SQL,
        ROUTINES_SQL,
        ARGUMENTS_SQL,
        RAW_ACL_SQL,
        EFFECTIVE_PRIVILEGES_SQL,
    ]
    .join("\n");
    for forbidden in [
        "prosrc",
        "prosqlbody",
        "pg_get_functiondef",
        "pg_get_expr",
        "SET ROLE",
        "SELECT owner_beta_simulation_payload_matches(",
        "SELECT owner_beta_simulation_version_binding_matches(",
        "SELECT owner_beta_simulation_version_namespace_matches(",
    ] {
        assert!(
            !sql.contains(forbidden),
            "forbidden source text: {forbidden}"
        );
    }
    for query in [
        CONTEXT_SQL,
        ROLES_SQL,
        MEMBERSHIPS_SQL,
        ROUTINES_SQL,
        ARGUMENTS_SQL,
        RAW_ACL_SQL,
    ] {
        assert!(query.contains("pg_catalog."));
    }
    assert!(CONTEXT_SQL.contains("worker.oid::pg_catalog.int8 AS worker_oid"));
    assert!(ROLES_SQL.contains("expected_roles.ordinal::pg_catalog.int4 AS role_ordinal"));
    assert!(ARGUMENTS_SQL.contains("argument.ordinality::pg_catalog.int4 AS argument_ordinal"));
    assert!(RAW_ACL_SQL.contains("acl.grantee::pg_catalog.int8 AS grantee_oid"));
    assert!(EFFECTIVE_PRIVILEGES_SQL.contains("$1::pg_catalog.oid"));
    assert!(EFFECTIVE_PRIVILEGES_SQL.contains("$9::pg_catalog.oid"));
}

#[test]
fn sql_projection_regression_rejects_omitted_reordered_and_extra_outer_columns() {
    let omitted = ROUTINES_SQL.replace(
        "returns_set::pg_catalog.bool AS returns_set",
        "returns_set::pg_catalog.bool",
    );
    assert_ne!(omitted, ROUTINES_SQL);
    assert!(!projection_matches(&omitted, &ROUTINE_SHAPE));

    let reordered = MEMBERSHIPS_SQL.replace(
        "membership.member::pg_catalog.int8 AS member_oid,\n       membership.roleid::pg_catalog.int8 AS role_oid",
        "membership.roleid::pg_catalog.int8 AS role_oid,\n       membership.member::pg_catalog.int8 AS member_oid",
    );
    assert_ne!(reordered, MEMBERSHIPS_SQL);
    assert!(!projection_matches(&reordered, &MEMBERSHIP_SHAPE));

    let extra = MEMBERSHIPS_SQL.replace(
        "SELECT membership.member::pg_catalog.int8 AS member_oid,",
        "SELECT membership.member::pg_catalog.int8 AS member_oid,\n       membership.member::pg_catalog.int8 AS unexpected_oid,",
    );
    assert_ne!(extra, MEMBERSHIPS_SQL);
    assert!(!projection_matches(&extra, &MEMBERSHIP_SHAPE));
}

#[test]
fn lifecycle_failures_preserve_primary_rollback_commit_and_close_uncertainty() {
    let mut acquire = FakeTransport::valid();
    acquire.fail_at = Some(Point::Acquire);
    assert_error(
        &mut acquire,
        SimulationVersionHelperMetadataError::AcquisitionFailed,
    );
    assert!(!acquire.event_log().contains(&Event::Close));

    let mut begin = FakeTransport::valid();
    begin.fail_at = Some(Point::Statement(StatementKind::Begin));
    assert_error(
        &mut begin,
        SimulationVersionHelperMetadataError::BeginFailed,
    );
    assert!(begin.event_log().contains(&Event::Close));

    let mut setup = FakeTransport::valid();
    setup.fail_at = Some(Point::Statement(StatementKind::Configure));
    assert_error(
        &mut setup,
        SimulationVersionHelperMetadataError::SetupFailed,
    );
    assert!(statement_seen(&setup.event_log(), StatementKind::Rollback));

    for kind in [
        StatementKind::Context,
        StatementKind::Roles,
        StatementKind::Memberships,
        StatementKind::Routines,
        StatementKind::Arguments,
        StatementKind::RawAcl,
        StatementKind::Effective,
        StatementKind::FinalContext,
    ] {
        let mut read_failure = FakeTransport::valid();
        read_failure.fail_at = Some(Point::Statement(kind));
        assert_error(
            &mut read_failure,
            SimulationVersionHelperMetadataError::CatalogUnavailable,
        );
        assert!(statement_seen(
            &read_failure.event_log(),
            StatementKind::Rollback
        ));
        assert!(read_failure.event_log().contains(&Event::Close));
        assert!(!statement_seen(
            &read_failure.event_log(),
            StatementKind::Commit
        ));
    }

    let mut rollback_response = FakeTransport::valid();
    rollback_response.fail_at = Some(Point::Statement(StatementKind::Context));
    rollback_response.unexpected_at = Some(StatementKind::Rollback);
    assert_error(
        &mut rollback_response,
        SimulationVersionHelperMetadataError::RollbackUnconfirmed {
            primary: PrimaryFailure::CatalogUnavailable,
        },
    );
    assert!(rollback_response.event_log().contains(&Event::Close));

    let mut canceled_during_rollback = FakeTransport::valid();
    canceled_during_rollback.fail_at = Some(Point::Statement(StatementKind::Context));
    canceled_during_rollback.cancel_at = Some(Point::Statement(StatementKind::Rollback));
    assert_error(
        &mut canceled_during_rollback,
        SimulationVersionHelperMetadataError::CatalogUnavailable,
    );
    assert!(canceled_during_rollback.event_log().contains(&Event::Close));
    assert!(!statement_seen(
        &canceled_during_rollback.event_log(),
        StatementKind::Commit
    ));

    let mut primary_and_both_cleanup_fail = FakeTransport::valid();
    replace_cell(&mut primary_and_both_cleanup_fail.routines[0], 6, text("p"));
    primary_and_both_cleanup_fail.fail_at = Some(Point::Statement(StatementKind::Rollback));
    primary_and_both_cleanup_fail.close_fails = true;
    assert_error(
        &mut primary_and_both_cleanup_fail,
        SimulationVersionHelperMetadataError::RollbackAndCloseUnconfirmed {
            primary: PrimaryFailure::PolicyRejected,
        },
    );
    assert!(statement_seen(
        &primary_and_both_cleanup_fail.event_log(),
        StatementKind::Rollback
    ));
    assert!(
        primary_and_both_cleanup_fail
            .event_log()
            .contains(&Event::Close)
    );

    let mut commit = FakeTransport::valid();
    commit.fail_at = Some(Point::Statement(StatementKind::Commit));
    assert_error(
        &mut commit,
        SimulationVersionHelperMetadataError::CompletionUnconfirmed,
    );
    assert!(!statement_seen(
        &commit.event_log(),
        StatementKind::Rollback
    ));

    let mut commit_close = FakeTransport::valid();
    commit_close.fail_at = Some(Point::Statement(StatementKind::Commit));
    commit_close.close_fails = true;
    assert_error(
        &mut commit_close,
        SimulationVersionHelperMetadataError::CompletionAndCleanupUnconfirmed,
    );

    let mut cleanup = FakeTransport::valid();
    cleanup.close_fails = true;
    assert_error(
        &mut cleanup,
        SimulationVersionHelperMetadataError::CleanupUnconfirmed,
    );

    let mut unexpected_begin = FakeTransport::valid();
    unexpected_begin.unexpected_at = Some(StatementKind::Begin);
    assert_error(
        &mut unexpected_begin,
        SimulationVersionHelperMetadataError::BeginResultInvalid,
    );
    assert!(statement_seen(
        &unexpected_begin.event_log(),
        StatementKind::Rollback
    ));

    let mut unexpected_setup = FakeTransport::valid();
    unexpected_setup.unexpected_at = Some(StatementKind::Configure);
    assert_error(
        &mut unexpected_setup,
        SimulationVersionHelperMetadataError::SetupFailed,
    );
    assert!(statement_seen(
        &unexpected_setup.event_log(),
        StatementKind::Rollback
    ));

    let mut unexpected_commit = FakeTransport::valid();
    unexpected_commit.unexpected_at = Some(StatementKind::Commit);
    assert_error(
        &mut unexpected_commit,
        SimulationVersionHelperMetadataError::CompletionUnconfirmed,
    );
}

#[test]
fn combined_cancellation_commit_close_and_primary_cleanup_failures_are_preserved() {
    let mut canceled_commit = FakeTransport::valid();
    canceled_commit.cancel_at = Some(Point::Statement(StatementKind::Commit));
    canceled_commit.fail_at = Some(Point::Statement(StatementKind::Commit));
    assert_error(
        &mut canceled_commit,
        SimulationVersionHelperMetadataError::CompletionUnconfirmed,
    );
    assert!(canceled_commit.event_log().contains(&Event::Close));

    let mut canceled_commit_and_close = FakeTransport::valid();
    canceled_commit_and_close.cancel_at = Some(Point::Statement(StatementKind::Commit));
    canceled_commit_and_close.fail_at = Some(Point::Statement(StatementKind::Commit));
    canceled_commit_and_close.close_fails = true;
    assert_error(
        &mut canceled_commit_and_close,
        SimulationVersionHelperMetadataError::CompletionAndCleanupUnconfirmed,
    );

    let mut canceled_close = FakeTransport::valid();
    canceled_close.cancel_at = Some(Point::Close);
    canceled_close.close_fails = true;
    assert_error(
        &mut canceled_close,
        SimulationVersionHelperMetadataError::CloseUnconfirmed {
            primary: PrimaryFailure::Canceled,
        },
    );
    assert!(statement_seen(
        &canceled_close.event_log(),
        StatementKind::Commit
    ));

    let mut primary_and_close = FakeTransport::valid();
    replace_cell(&mut primary_and_close.context, 0, text("admin"));
    primary_and_close.close_fails = true;
    assert_error(
        &mut primary_and_close,
        SimulationVersionHelperMetadataError::CloseUnconfirmed {
            primary: PrimaryFailure::PolicyRejected,
        },
    );
    assert!(statement_seen(
        &primary_and_close.event_log(),
        StatementKind::Rollback
    ));
}

#[test]
fn cancellation_at_each_observer_boundary_never_returns_evidence() {
    let points = [
        Point::Acquire,
        Point::Arm,
        Point::Statement(StatementKind::Begin),
        Point::Statement(StatementKind::Configure),
        Point::Statement(StatementKind::Context),
        Point::Statement(StatementKind::Roles),
        Point::Statement(StatementKind::Memberships),
        Point::Statement(StatementKind::Routines),
        Point::Statement(StatementKind::Arguments),
        Point::Statement(StatementKind::RawAcl),
        Point::Statement(StatementKind::Effective),
        Point::Statement(StatementKind::FinalContext),
        Point::Statement(StatementKind::Commit),
        Point::Close,
    ];
    for point in points {
        let mut fake = FakeTransport::valid();
        fake.cancel_at = Some(point);
        assert_no_observation(&mut fake);
        if matches!(
            point,
            Point::Statement(StatementKind::Commit) | Point::Close
        ) {
            assert!(statement_seen(&fake.event_log(), StatementKind::Commit));
        }
    }

    let mut already_canceled = FakeTransport::valid();
    already_canceled.canceled.store(true, Ordering::Release);
    assert_error(
        &mut already_canceled,
        SimulationVersionHelperMetadataError::Canceled,
    );
    assert!(!statement_seen(
        &already_canceled.event_log(),
        StatementKind::Begin
    ));
}

#[test]
fn dropping_a_pending_observation_has_armed_the_disposable_connection_and_yields_no_token() {
    let mut fake = FakeTransport::valid();
    fake.pending_at = Some(StatementKind::Context);
    let config = valid_config();
    let canceled = Arc::clone(&fake.canceled);
    let mut future = Box::pin(observe_with_transport(&mut fake, &config, &canceled));
    let mut context = Context::from_waker(Waker::noop());
    assert!(matches!(future.as_mut().poll(&mut context), Poll::Pending));
    drop(future);
    let events = fake.event_log();
    let armed = events
        .iter()
        .position(|event| *event == Event::Armed)
        .expect("arm event");
    let first_sql = events
        .iter()
        .position(|event| matches!(event, Event::Statement(_)))
        .expect("SQL event");
    assert!(armed < first_sql);
    assert!(events.contains(&Event::PendingDroppedAfterArm(true)));
    assert!(!statement_seen(&events, StatementKind::Commit));
}
#[test]
fn command_result_discards_row_payload_and_rejects_presence() {
    assert!(super::classify_command_result(None::<String>).is_empty());
    let rows = super::classify_command_result(Some(String::from("sentinel")));
    assert_eq!(rows.len(), 1);
    assert!(rows[0].cells.is_empty());
}
