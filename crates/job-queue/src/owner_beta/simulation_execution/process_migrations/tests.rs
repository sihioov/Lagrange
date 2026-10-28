// Fake polling and deliberately nested branch fixtures model exact failure points.
#![allow(clippy::never_loop, clippy::collapsible_if)]

use super::*;
use std::collections::VecDeque;
use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

#[cfg(feature = "r4-pg-observer-tests")]
use super::super::pg_test_common;

#[cfg(feature = "r4-pg-observer-tests")]
#[tokio::test]
async fn r4_migration_ledger_pg_projection() {
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
    let schema_oid: i64 = sqlx::query_scalar(
        "SELECT oid::pg_catalog.int8 FROM pg_catalog.pg_namespace WHERE nspname = 'public'",
    )
    .fetch_one(&pool)
    .await
    .expect("public schema exists");
    let database_oid: i64 = sqlx::query_scalar(
        "SELECT oid::pg_catalog.int8 FROM pg_catalog.pg_database WHERE datname = current_database()",
    )
    .fetch_one(&pool)
    .await
    .expect("scratch database exists");
    let ledger_oid: i64 = sqlx::query_scalar(
        "SELECT relation.oid::pg_catalog.int8 FROM pg_catalog.pg_class AS relation \
         JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace \
         WHERE namespace.nspname = 'public' AND relation.relname = '_sqlx_migrations'",
    )
    .fetch_one(&pool)
    .await
    .expect("migration ledger exists");
    let worker_oid = u32::try_from(worker_oid).expect("worker OID fits u32");
    let public_schema_oid = u32::try_from(schema_oid).expect("schema OID fits u32");
    let database_oid = u32::try_from(database_oid).expect("database OID fits u32");
    let ledger_oid = u32::try_from(ledger_oid).expect("ledger OID fits u32");
    let mut transport = SqlxObserverTransport::new(&pool);
    transport
        .acquire()
        .await
        .unwrap_or_else(|_| panic!("acquire observer connection"));
    transport.arm_close_on_drop();
    assert!(
        transport
            .execute(Statement::Begin)
            .await
            .unwrap_or_else(|_| panic!("BEGIN"))
            .is_empty()
    );
    let statements = [
        ("Configure", Statement::Configure { timeout_ms: 30_000 }),
        ("Context", Statement::Context),
        ("Memberships", Statement::Memberships { worker_oid }),
        ("Relation", Statement::Relation { public_schema_oid }),
        ("Columns", Statement::Columns { ledger_oid }),
        (
            "PrimaryKey",
            Statement::PrimaryKey {
                public_schema_oid,
                ledger_oid,
            },
        ),
        (
            "RawAcls",
            Statement::RawAcls {
                database_oid,
                public_schema_oid,
                ledger_oid,
                worker_oid,
            },
        ),
        ("Ledger", Statement::Ledger),
    ];
    for (label, statement) in statements {
        let rows = transport
            .execute(statement)
            .await
            .unwrap_or_else(|_| panic!("real migration SQLx projection: {label}"));
        if matches!(
            label,
            "Context" | "Relation" | "Columns" | "PrimaryKey" | "Ledger"
        ) {
            assert!(!rows.is_empty(), "empty migration PG projection: {label}");
        }
        assert!(
            rows.iter().all(|row| !row
                .cells
                .iter()
                .any(|cell| matches!(cell, RawCell::WrongType))),
            "wrong migration PG cell type: {label}"
        );
    }
    assert!(
        transport
            .execute(Statement::Commit)
            .await
            .unwrap_or_else(|_| panic!("COMMIT"))
            .is_empty()
    );
    transport
        .close()
        .await
        .unwrap_or_else(|_| panic!("close observer connection"));
    pool.close().await;
    db.drop_db().await;
    eprintln!("R4_MIGRATION_PG:CONTEXT_LEDGER_AND_COMMANDS:PASS");
}

const TEST_DATABASE_OID: u32 = 10;
const TEST_SCHEMA_OID: u32 = 20;
const TEST_WORKER_OID: u32 = 100;
const TEST_OWNER_OID: u32 = 30;
const TEST_LEDGER_OID: u32 = 40;
const TEST_INDEX_OID: u32 = 41;
const TEST_TIMEOUT_MS: i32 = 5_000;

fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = Box::pin(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => panic!("in-memory fake unexpectedly yielded"),
        }
    }
}

fn text(value: &str) -> RawCell {
    RawCell::Text(value.to_owned())
}

fn i32_value(value: i32) -> RawCell {
    RawCell::I32(value)
}

fn i64_value(value: i64) -> RawCell {
    RawCell::I64(value)
}

fn boolean(value: bool) -> RawCell {
    RawCell::Bool(value)
}

fn raw_row(category: RawCategory, cells: Vec<RawCell>) -> RawRow {
    RawRow::new(category, cells)
}

fn context_row() -> RawRow {
    raw_row(
        RawCategory::Context,
        vec![
            text(WORKER_ROLE),
            text(WORKER_ROLE),
            text("simulation_test"),
            text("simulation_test"),
            text(PUBLIC_SCHEMA),
            i64_value(i64::from(TEST_WORKER_OID)),
            i64_value(i64::from(TEST_DATABASE_OID)),
            i64_value(i64::from(TEST_SCHEMA_OID)),
            i64_value(i64::from(TEST_OWNER_OID)),
            i64_value(11),
            i64_value(12),
            boolean(true),
            boolean(false),
            boolean(false),
            boolean(false),
            boolean(false),
            boolean(false),
            text("on"),
            text("origin"),
            boolean(true),
            text("repeatable read"),
            boolean(true),
            boolean(false),
            boolean(true),
            boolean(false),
            boolean(false),
            boolean(false),
            boolean(false),
            boolean(false),
        ],
    )
}

fn membership_row() -> RawRow {
    raw_row(
        RawCategory::Membership,
        vec![
            i64_value(i64::from(TEST_WORKER_OID)),
            i64_value(i64::from(TEST_OWNER_OID)),
            i64_value(11),
            boolean(false),
            boolean(false),
            boolean(false),
        ],
    )
}

fn relation_row() -> RawRow {
    raw_row(
        RawCategory::Relation,
        vec![
            i64_value(i64::from(TEST_SCHEMA_OID)),
            text(PUBLIC_SCHEMA),
            text(LEDGER_NAME),
            i64_value(i64::from(TEST_LEDGER_OID)),
            i64_value(i64::from(TEST_OWNER_OID)),
            text(MIGRATION_OWNER_ROLE),
            text("r"),
            text("p"),
            boolean(false),
            boolean(false),
            boolean(false),
            boolean(false),
            boolean(true),
            boolean(false),
            boolean(false),
            boolean(false),
            boolean(false),
            boolean(false),
            boolean(false),
            boolean(false),
        ],
    )
}

fn column_row(attnum: i32, name: &str, type_name: &str) -> RawRow {
    raw_row(
        RawCategory::Column,
        vec![
            i64_value(i64::from(TEST_LEDGER_OID)),
            text(LEDGER_NAME),
            i32_value(attnum),
            text(name),
            text(PUBLIC_SCHEMA_PG),
            text(type_name),
            text("b"),
            i64_value(0),
            boolean(true),
            boolean(false),
            text(""),
            text(""),
            i32_value(0),
            i32_value(-1),
            boolean(true),
            boolean(false),
            boolean(false),
            boolean(false),
            boolean(false),
            boolean(false),
            boolean(false),
            boolean(false),
        ],
    )
}

fn columns_rows() -> Vec<RawRow> {
    [
        (1, "version", "int8"),
        (2, "description", "text"),
        (3, "installed_on", "timestamptz"),
        (4, "success", "bool"),
        (5, "checksum", "bytea"),
        (6, "execution_time", "int8"),
    ]
    .into_iter()
    .map(|(attnum, name, type_name)| column_row(attnum, name, type_name))
    .collect()
}

fn primary_key_row() -> RawRow {
    raw_row(
        RawCategory::PrimaryKey,
        vec![
            i64_value(i64::from(TEST_LEDGER_OID)),
            i64_value(i64::from(TEST_SCHEMA_OID)),
            i64_value(i64::from(TEST_SCHEMA_OID)),
            text(PUBLIC_SCHEMA),
            text("_sqlx_migrations_pkey"),
            text("p"),
            i32_value(1),
            i32_value(1),
            boolean(true),
            boolean(false),
            boolean(false),
            i64_value(i64::from(TEST_INDEX_OID)),
            i32_value(1),
            i32_value(1),
            boolean(true),
            boolean(true),
            boolean(true),
            boolean(true),
            boolean(true),
            boolean(true),
            boolean(true),
            i64_value(i64::from(TEST_SCHEMA_OID)),
            text(PUBLIC_SCHEMA),
            text("i"),
            text("p"),
            i64_value(i64::from(TEST_OWNER_OID)),
        ],
    )
}

fn table_select_acl() -> RawRow {
    raw_row(
        RawCategory::Acl,
        vec![
            i32_value(3),
            i64_value(i64::from(TEST_LEDGER_OID)),
            RawCell::Null,
            i64_value(i64::from(TEST_WORKER_OID)),
            i64_value(i64::from(TEST_OWNER_OID)),
            text("SELECT"),
            boolean(false),
        ],
    )
}

fn from_hex(value: &str) -> Vec<u8> {
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digit = |byte: u8| match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                _ => 0,
            };
            (digit(pair[0]) << 4) | digit(pair[1])
        })
        .collect()
}

fn ledger_row(index: usize) -> RawRow {
    let entry = &BASELINE[index];
    raw_row(
        RawCategory::Ledger,
        vec![
            i64_value(i64::from(TEST_LEDGER_OID)),
            i64_value(entry.version),
            boolean(true),
            i32_value(48),
            RawCell::Bytes(from_hex(entry.sha384_hex)),
        ],
    )
}

fn ledger_rows() -> Vec<RawRow> {
    (0..EXPECTED_ROW_COUNT).map(ledger_row).collect()
}

struct ScriptStep {
    statement: Statement,
    result: Result<Vec<RawRow>, TransportFailure>,
    cancel_after: bool,
}

impl ScriptStep {
    fn success(statement: Statement, rows: Vec<RawRow>) -> Self {
        Self {
            statement,
            result: Ok(rows),
            cancel_after: false,
        }
    }
}

fn successful_steps() -> Vec<ScriptStep> {
    vec![
        ScriptStep::success(Statement::Begin, Vec::new()),
        ScriptStep::success(
            Statement::Configure {
                timeout_ms: TEST_TIMEOUT_MS,
            },
            Vec::new(),
        ),
        ScriptStep::success(Statement::Context, vec![context_row()]),
        ScriptStep::success(
            Statement::Memberships {
                worker_oid: TEST_WORKER_OID,
            },
            Vec::new(),
        ),
        ScriptStep::success(
            Statement::Relation {
                public_schema_oid: TEST_SCHEMA_OID,
            },
            vec![relation_row()],
        ),
        ScriptStep::success(
            Statement::Columns {
                ledger_oid: TEST_LEDGER_OID,
            },
            columns_rows(),
        ),
        ScriptStep::success(
            Statement::PrimaryKey {
                public_schema_oid: TEST_SCHEMA_OID,
                ledger_oid: TEST_LEDGER_OID,
            },
            vec![primary_key_row()],
        ),
        ScriptStep::success(
            Statement::RawAcls {
                database_oid: TEST_DATABASE_OID,
                public_schema_oid: TEST_SCHEMA_OID,
                ledger_oid: TEST_LEDGER_OID,
                worker_oid: TEST_WORKER_OID,
            },
            vec![table_select_acl()],
        ),
        ScriptStep::success(Statement::Ledger, ledger_rows()),
        ScriptStep::success(Statement::Commit, Vec::new()),
    ]
}

fn successful_steps_with_relation_oid(ledger_oid: u32) -> Vec<ScriptStep> {
    let mut steps = successful_steps();
    for step in &mut steps {
        match &mut step.statement {
            Statement::Relation { .. } => {
                let mut row = relation_row();
                row.cells[3] = i64_value(i64::from(ledger_oid));
                step.result = Ok(vec![row]);
            }
            Statement::Columns {
                ledger_oid: statement_oid,
            } => {
                *statement_oid = ledger_oid;
                let mut rows = columns_rows();
                for row in &mut rows {
                    row.cells[0] = i64_value(i64::from(ledger_oid));
                }
                step.result = Ok(rows);
            }
            Statement::PrimaryKey {
                ledger_oid: statement_oid,
                ..
            } => {
                *statement_oid = ledger_oid;
                let mut row = primary_key_row();
                row.cells[0] = i64_value(i64::from(ledger_oid));
                step.result = Ok(vec![row]);
            }
            Statement::RawAcls {
                ledger_oid: statement_oid,
                ..
            } => {
                *statement_oid = ledger_oid;
                let mut row = table_select_acl();
                row.cells[1] = i64_value(i64::from(ledger_oid));
                step.result = Ok(vec![row]);
            }
            Statement::Ledger => {
                let mut rows = ledger_rows();
                for row in &mut rows {
                    row.cells[0] = i64_value(i64::from(ledger_oid));
                }
                step.result = Ok(rows);
            }
            _ => {}
        }
    }
    steps
}

struct FakeTransport {
    steps: VecDeque<ScriptStep>,
    events: Vec<Statement>,
    acquired: bool,
    armed: bool,
    closed: bool,
    fail_acquire: bool,
    fail_close: bool,
    fail_rollback: bool,
    cancel_after_acquire: bool,
    cancel_after_close: bool,
    cancel_flag: Option<Arc<AtomicBool>>,
    pending_statement: Option<Statement>,
}

impl FakeTransport {
    fn new(steps: Vec<ScriptStep>) -> Self {
        Self {
            steps: steps.into(),
            events: Vec::new(),
            acquired: false,
            armed: false,
            closed: false,
            fail_acquire: false,
            fail_close: false,
            fail_rollback: false,
            cancel_after_acquire: false,
            cancel_after_close: false,
            cancel_flag: None,
            pending_statement: None,
        }
    }

    fn step_mut(&mut self, statement: Statement) -> &mut ScriptStep {
        self.steps
            .iter_mut()
            .find(|step| step.statement == statement)
            .unwrap()
    }

    fn set_query_rows(&mut self, statement: Statement, rows: Vec<RawRow>) {
        self.step_mut(statement).result = Ok(rows);
    }

    fn set_cancel_after(&mut self, statement: Statement) {
        self.step_mut(statement).cancel_after = true;
    }
}

#[async_trait::async_trait]
impl ObserverTransport for FakeTransport {
    async fn acquire(&mut self) -> Result<(), TransportFailure> {
        if self.fail_acquire {
            return Err(TransportFailure::Database);
        }
        self.acquired = true;
        if self.cancel_after_acquire {
            if let Some(flag) = self.cancel_flag.as_ref() {
                flag.store(true, Ordering::Release);
            }
        }
        Ok(())
    }

    fn arm_close_on_drop(&mut self) {
        self.armed = true;
    }

    async fn execute(&mut self, statement: Statement) -> Result<Vec<RawRow>, TransportFailure> {
        self.events.push(statement);
        if self.pending_statement == Some(statement) {
            std::future::pending::<()>().await;
        }
        if statement == Statement::Rollback {
            return if self.fail_rollback {
                Err(TransportFailure::Database)
            } else {
                Ok(super::classify_command_result(None::<()>))
            };
        }
        let step = self.steps.pop_front().ok_or(TransportFailure::State)?;
        if step.statement != statement {
            return Err(TransportFailure::State);
        }
        if step.cancel_after {
            if let Some(flag) = self.cancel_flag.as_ref() {
                flag.store(true, Ordering::Release);
            }
        }
        if matches!(statement, Statement::Begin | Statement::Commit) {
            step.result
                .map(|rows| super::classify_command_result(rows.into_iter().next()))
        } else {
            step.result
        }
    }

    async fn close(&mut self) -> Result<(), TransportFailure> {
        self.closed = true;
        if self.cancel_after_close {
            if let Some(flag) = self.cancel_flag.as_ref() {
                flag.store(true, Ordering::Release);
            }
        }
        if self.fail_close {
            Err(TransportFailure::Database)
        } else {
            Ok(())
        }
    }
}

fn run_observer(
    transport: &mut FakeTransport,
    canceled: &AtomicBool,
    timeout_ms: i32,
) -> Result<SimulationMigrationLedgerObservation, SimulationMigrationLedgerError> {
    block_on(observe_with_transport(
        transport,
        "sha256:test-config",
        canceled,
        validate_embedded_baseline().unwrap(),
        timeout_ms,
    ))
}

fn test_context_evidence() -> ContextEvidence {
    ContextEvidence {
        database_oid: TEST_DATABASE_OID,
        worker_oid: TEST_WORKER_OID,
        public_schema_oid: TEST_SCHEMA_OID,
        migration_owner_oid: TEST_OWNER_OID,
    }
}

#[test]
fn accepted_baseline_is_finite_raw_and_fingerprint_pinned() {
    let baseline = validate_embedded_baseline().unwrap();
    assert_eq!(BASELINE.len(), 64);
    assert_eq!(BASELINE[0].version, 1);
    assert_eq!(BASELINE[63].version, 64);
    assert_eq!(baseline.fingerprint_text, EXPECTED_FINGERPRINT);
    assert_eq!(POLICY_ID, "r4-simulation-ledger-baseline-64-v1");
    assert!(
        BASELINE
            .iter()
            .all(|entry| entry.path.starts_with("migrations/") && entry.path.ends_with(".up.sql"))
    );
}

#[test]
fn baseline_rejects_down_paths_duplicates_and_version_gaps() {
    let mut entries: Vec<_> = BASELINE
        .iter()
        .map(|entry| ExpectedMigration {
            version: entry.version,
            path: entry.path,
            byte_len: entry.byte_len,
            sha256_hex: entry.sha256_hex,
            sha384_hex: entry.sha384_hex,
            bytes: entry.bytes,
        })
        .collect();
    entries[7].path = "migrations/0008_not_an_up.down.sql";
    assert!(matches!(
        validate_baseline_metadata(&entries),
        Err(SimulationMigrationLedgerError::BaselineInvalid)
    ));

    entries[7].path = entries[6].path;
    assert!(matches!(
        validate_baseline_metadata(&entries),
        Err(SimulationMigrationLedgerError::BaselineInvalid)
    ));

    entries[7].path = BASELINE[7].path;
    entries[7].version = 65;
    assert!(matches!(
        validate_baseline_metadata(&entries),
        Err(SimulationMigrationLedgerError::BaselineInvalid)
    ));
}

#[test]
fn baseline_rejects_byte_edits_and_newline_normalization() {
    let entry = &BASELINE[0];
    let mut changed = entry.bytes.to_vec();
    changed[0] = if changed[0] == b'X' { b'Y' } else { b'X' };
    assert!(matches!(
        validate_expected_entry(entry, &changed),
        Err(SimulationMigrationLedgerError::BaselineInvalid)
    ));

    let mut normalized = entry.bytes.to_vec();
    normalized.push(b'\n');
    assert!(matches!(
        validate_expected_entry(entry, &normalized),
        Err(SimulationMigrationLedgerError::BaselineInvalid)
    ));
}

#[test]
fn timeout_bound_rejects_zero_and_i32_millisecond_overflow() {
    assert_eq!(timeout_millis(5).unwrap(), TEST_TIMEOUT_MS);
    assert!(matches!(
        timeout_millis(0),
        Err(SimulationMigrationLedgerError::InvalidBound)
    ));
    let too_large = (i32::MAX as u64 / 1000) + 1;
    assert!(matches!(
        timeout_millis(too_large),
        Err(SimulationMigrationLedgerError::InvalidBound)
    ));
}

#[test]
fn invalid_bound_and_pre_acquire_cancel_do_no_transport_io() {
    let mut invalid = FakeTransport::new(successful_steps());
    let canceled = AtomicBool::new(false);
    assert!(matches!(
        run_observer(&mut invalid, &canceled, 0),
        Err(SimulationMigrationLedgerError::InvalidBound)
    ));
    assert!(!invalid.acquired);
    assert!(invalid.events.is_empty());

    let mut canceled_before = FakeTransport::new(successful_steps());
    let canceled = AtomicBool::new(true);
    assert!(matches!(
        run_observer(&mut canceled_before, &canceled, TEST_TIMEOUT_MS),
        Err(SimulationMigrationLedgerError::Canceled)
    ));
    assert!(!canceled_before.acquired);
    assert!(canceled_before.events.is_empty());
}

#[test]
fn acquisition_failure_does_not_arm_or_close_a_connection() {
    let mut fake = FakeTransport::new(successful_steps());
    fake.fail_acquire = true;
    let canceled = AtomicBool::new(false);
    assert!(matches!(
        run_observer(&mut fake, &canceled, TEST_TIMEOUT_MS),
        Err(SimulationMigrationLedgerError::AcquisitionFailed)
    ));
    assert!(!fake.armed);
    assert!(!fake.closed);
    assert!(fake.events.is_empty());
}

#[test]
fn canceled_immediately_after_acquire_arms_then_closes_without_begin() {
    let canceled = Arc::new(AtomicBool::new(false));
    let mut fake = FakeTransport::new(successful_steps());
    fake.cancel_after_acquire = true;
    fake.cancel_flag = Some(Arc::clone(&canceled));
    assert!(matches!(
        run_observer(&mut fake, canceled.as_ref(), TEST_TIMEOUT_MS),
        Err(SimulationMigrationLedgerError::Canceled)
    ));
    assert!(fake.acquired);
    assert!(fake.armed);
    assert!(fake.closed);
    assert!(fake.events.is_empty());
}

#[test]
fn begin_failure_closes_without_claiming_rollback() {
    let mut fake = FakeTransport::new(vec![ScriptStep {
        statement: Statement::Begin,
        result: Err(TransportFailure::Database),
        cancel_after: false,
    }]);
    let canceled = AtomicBool::new(false);
    assert!(matches!(
        run_observer(&mut fake, &canceled, TEST_TIMEOUT_MS),
        Err(SimulationMigrationLedgerError::BeginFailed)
    ));
    assert!(fake.armed && fake.closed);
    assert!(fake.events == vec![Statement::Begin]);
}

#[test]
fn odd_begin_result_rolls_back_once_then_closes() {
    let mut fake = FakeTransport::new(vec![ScriptStep::success(
        Statement::Begin,
        vec![context_row()],
    )]);
    let canceled = AtomicBool::new(false);
    assert!(matches!(
        run_observer(&mut fake, &canceled, TEST_TIMEOUT_MS),
        Err(SimulationMigrationLedgerError::BeginResultInvalid)
    ));
    assert!(fake.closed);
    assert!(fake.events == vec![Statement::Begin, Statement::Rollback]);
}

#[test]
fn successful_observation_has_exact_order_and_retains_checked_identity() {
    let mut fake = FakeTransport::new(successful_steps());
    let canceled = AtomicBool::new(false);
    let observation = run_observer(&mut fake, &canceled, TEST_TIMEOUT_MS).unwrap();
    let expected = vec![
        Statement::Begin,
        Statement::Configure {
            timeout_ms: TEST_TIMEOUT_MS,
        },
        Statement::Context,
        Statement::Memberships {
            worker_oid: TEST_WORKER_OID,
        },
        Statement::Relation {
            public_schema_oid: TEST_SCHEMA_OID,
        },
        Statement::Columns {
            ledger_oid: TEST_LEDGER_OID,
        },
        Statement::PrimaryKey {
            public_schema_oid: TEST_SCHEMA_OID,
            ledger_oid: TEST_LEDGER_OID,
        },
        Statement::RawAcls {
            database_oid: TEST_DATABASE_OID,
            public_schema_oid: TEST_SCHEMA_OID,
            ledger_oid: TEST_LEDGER_OID,
            worker_oid: TEST_WORKER_OID,
        },
        Statement::Ledger,
        Statement::Commit,
    ];
    assert!(fake.events == expected);
    assert!(fake.acquired && fake.armed && fake.closed);
    assert_eq!(observation.config_sha256(), "sha256:test-config");
    assert_eq!(observation.baseline_policy_id(), POLICY_ID);
    assert_eq!(observation.baseline_fingerprint(), EXPECTED_FINGERPRINT);
    assert_eq!(observation.database_oid(), TEST_DATABASE_OID);
    assert_eq!(observation.worker_oid(), TEST_WORKER_OID);
    assert_eq!(observation.public_schema_oid(), TEST_SCHEMA_OID);
    assert_eq!(observation.migration_owner_oid(), TEST_OWNER_OID);
    assert_eq!(observation.ledger_oid(), TEST_LEDGER_OID);
    assert_eq!(observation.ledger_row_count(), 64);
    assert_eq!(
        observation.ordered_ledger_digest(),
        expected_ledger_digest()
    );
}

fn expected_ledger_digest() -> String {
    let baseline = validate_embedded_baseline().unwrap();
    let mut digest = Sha256::new();
    digest.update(b"lagrange/simulation-migration-observation/v1");
    digest.update([0u8]);
    digest.update(baseline.fingerprint);
    digest.update(TEST_LEDGER_OID.to_be_bytes());
    digest.update(64u64.to_be_bytes());
    for entry in BASELINE.iter() {
        let (_, checksum) = validate_expected_entry(entry, entry.bytes).unwrap();
        digest.update(entry.version.to_be_bytes());
        digest.update([1u8]);
        digest.update(checksum);
    }
    format!("sha256:{:x}", digest.finalize())
}

#[test]
fn precommit_statement_failures_preserve_typed_failure_and_cleanup() {
    let cases = [
        (
            Statement::Configure {
                timeout_ms: TEST_TIMEOUT_MS,
            },
            SimulationMigrationLedgerError::SetupFailed,
        ),
        (
            Statement::Context,
            SimulationMigrationLedgerError::CatalogUnavailable,
        ),
        (
            Statement::Memberships {
                worker_oid: TEST_WORKER_OID,
            },
            SimulationMigrationLedgerError::CatalogUnavailable,
        ),
        (
            Statement::Relation {
                public_schema_oid: TEST_SCHEMA_OID,
            },
            SimulationMigrationLedgerError::CatalogUnavailable,
        ),
        (
            Statement::Columns {
                ledger_oid: TEST_LEDGER_OID,
            },
            SimulationMigrationLedgerError::CatalogUnavailable,
        ),
        (
            Statement::PrimaryKey {
                public_schema_oid: TEST_SCHEMA_OID,
                ledger_oid: TEST_LEDGER_OID,
            },
            SimulationMigrationLedgerError::CatalogUnavailable,
        ),
        (
            Statement::RawAcls {
                database_oid: TEST_DATABASE_OID,
                public_schema_oid: TEST_SCHEMA_OID,
                ledger_oid: TEST_LEDGER_OID,
                worker_oid: TEST_WORKER_OID,
            },
            SimulationMigrationLedgerError::CatalogUnavailable,
        ),
        (
            Statement::Ledger,
            SimulationMigrationLedgerError::LedgerUnavailable,
        ),
    ];
    for (statement, expected_error) in cases {
        let mut fake = FakeTransport::new(successful_steps());
        fake.step_mut(statement).result = Err(TransportFailure::Database);
        let canceled = AtomicBool::new(false);
        let result = run_observer(&mut fake, &canceled, TEST_TIMEOUT_MS);
        assert!(matches!(result, Err(error) if error == expected_error));
        assert!(fake.events.contains(&Statement::Rollback));
        assert!(fake.closed);
    }
}

#[test]
fn precommit_cleanup_uncertainty_takes_precedence_over_original_error() {
    let mut rollback_unknown = FakeTransport::new(successful_steps());
    rollback_unknown.step_mut(Statement::Context).result = Err(TransportFailure::Database);
    rollback_unknown.fail_rollback = true;
    let canceled = AtomicBool::new(false);
    assert!(matches!(
        run_observer(&mut rollback_unknown, &canceled, TEST_TIMEOUT_MS),
        Err(SimulationMigrationLedgerError::CompletionUnconfirmed)
    ));
    assert!(rollback_unknown.closed);

    let mut both_unknown = FakeTransport::new(successful_steps());
    both_unknown.step_mut(Statement::Context).result = Err(TransportFailure::Database);
    both_unknown.fail_rollback = true;
    both_unknown.fail_close = true;
    let canceled = AtomicBool::new(false);
    assert!(matches!(
        run_observer(&mut both_unknown, &canceled, TEST_TIMEOUT_MS),
        Err(SimulationMigrationLedgerError::CompletionAndCleanupUnconfirmed)
    ));

    let mut close_unknown = FakeTransport::new(successful_steps());
    close_unknown.step_mut(Statement::Context).result = Err(TransportFailure::Database);
    close_unknown.fail_close = true;
    let canceled = AtomicBool::new(false);
    assert!(matches!(
        run_observer(&mut close_unknown, &canceled, TEST_TIMEOUT_MS),
        Err(SimulationMigrationLedgerError::CleanupUnconfirmed)
    ));
}

#[test]
fn cancellation_is_checked_at_each_observer_await() {
    let statements = [
        Statement::Begin,
        Statement::Configure {
            timeout_ms: TEST_TIMEOUT_MS,
        },
        Statement::Context,
        Statement::Memberships {
            worker_oid: TEST_WORKER_OID,
        },
        Statement::Relation {
            public_schema_oid: TEST_SCHEMA_OID,
        },
        Statement::Columns {
            ledger_oid: TEST_LEDGER_OID,
        },
        Statement::PrimaryKey {
            public_schema_oid: TEST_SCHEMA_OID,
            ledger_oid: TEST_LEDGER_OID,
        },
        Statement::RawAcls {
            database_oid: TEST_DATABASE_OID,
            public_schema_oid: TEST_SCHEMA_OID,
            ledger_oid: TEST_LEDGER_OID,
            worker_oid: TEST_WORKER_OID,
        },
        Statement::Ledger,
        Statement::Commit,
    ];
    for statement in statements {
        let canceled = Arc::new(AtomicBool::new(false));
        let mut fake = FakeTransport::new(successful_steps());
        fake.cancel_flag = Some(Arc::clone(&canceled));
        fake.set_cancel_after(statement);
        assert!(matches!(
            run_observer(&mut fake, canceled.as_ref(), TEST_TIMEOUT_MS),
            Err(SimulationMigrationLedgerError::Canceled)
        ));
        assert!(fake.closed);
        assert!(fake.armed);
        if statement == Statement::Commit {
            assert!(!fake.events.contains(&Statement::Rollback));
        } else {
            assert!(fake.events.contains(&Statement::Rollback));
        }
    }
}

#[test]
fn dropped_observation_future_has_armed_close_on_drop_and_no_async_cleanup_claim() {
    let canceled = AtomicBool::new(false);
    let mut fake = FakeTransport::new(successful_steps());
    fake.pending_statement = Some(Statement::Context);
    {
        let future = observe_with_transport(
            &mut fake,
            "sha256:test-config",
            &canceled,
            validate_embedded_baseline().unwrap(),
            TEST_TIMEOUT_MS,
        );
        let mut future = Box::pin(future);
        let mut context = Context::from_waker(Waker::noop());
        assert!(matches!(future.as_mut().poll(&mut context), Poll::Pending));
    }
    assert!(fake.acquired && fake.armed);
    assert!(!fake.closed);
    assert!(fake.events.contains(&Statement::Context));
}

#[test]
fn context_identity_security_and_grant_option_checks_fail_closed() {
    let cases = [
        (
            0,
            text("other"),
            SimulationMigrationLedgerError::CatalogInvalid,
        ),
        (
            9,
            i64_value(i64::from(TEST_WORKER_OID)),
            SimulationMigrationLedgerError::CatalogInvalid,
        ),
        (
            10,
            i64_value(i64::from(TEST_WORKER_OID)),
            SimulationMigrationLedgerError::CatalogInvalid,
        ),
        (
            11,
            boolean(false),
            SimulationMigrationLedgerError::AccessRejected,
        ),
        (
            12,
            boolean(true),
            SimulationMigrationLedgerError::AccessRejected,
        ),
        (
            16,
            boolean(true),
            SimulationMigrationLedgerError::AccessRejected,
        ),
        (
            17,
            text("off"),
            SimulationMigrationLedgerError::AccessRejected,
        ),
        (
            18,
            text("replica"),
            SimulationMigrationLedgerError::AccessRejected,
        ),
        (
            19,
            boolean(false),
            SimulationMigrationLedgerError::AccessRejected,
        ),
        (
            20,
            text("read committed"),
            SimulationMigrationLedgerError::AccessRejected,
        ),
        (
            21,
            boolean(false),
            SimulationMigrationLedgerError::AccessRejected,
        ),
        (
            22,
            boolean(true),
            SimulationMigrationLedgerError::AccessRejected,
        ),
        (
            23,
            boolean(false),
            SimulationMigrationLedgerError::AccessRejected,
        ),
        (
            24,
            boolean(true),
            SimulationMigrationLedgerError::AccessRejected,
        ),
        (
            25,
            boolean(true),
            SimulationMigrationLedgerError::AccessRejected,
        ),
        (
            26,
            boolean(true),
            SimulationMigrationLedgerError::AccessRejected,
        ),
        (
            27,
            boolean(true),
            SimulationMigrationLedgerError::AccessRejected,
        ),
        (
            28,
            boolean(true),
            SimulationMigrationLedgerError::AccessRejected,
        ),
    ];
    for (index, cell, expected_error) in cases {
        let mut row = context_row();
        row.cells[index] = cell;
        let mut fake = FakeTransport::new(successful_steps());
        fake.set_query_rows(Statement::Context, vec![row]);
        let canceled = AtomicBool::new(false);
        let result = run_observer(&mut fake, &canceled, TEST_TIMEOUT_MS);
        assert!(matches!(result, Err(error) if error == expected_error));
        assert!(fake.closed);
    }
}

#[test]
fn any_outgoing_worker_membership_is_rejected_after_shared_decoding() {
    let mut fake = FakeTransport::new(successful_steps());
    fake.set_query_rows(
        Statement::Memberships {
            worker_oid: TEST_WORKER_OID,
        },
        vec![membership_row()],
    );
    let canceled = AtomicBool::new(false);
    assert!(matches!(
        run_observer(&mut fake, &canceled, TEST_TIMEOUT_MS),
        Err(SimulationMigrationLedgerError::AccessRejected)
    ));
    assert!(fake.events.contains(&Statement::Rollback));
    assert!(fake.closed);
}

#[test]
fn relation_shape_inheritance_and_effective_privileges_fail_closed() {
    let cases = [
        (6, text("v"), SimulationMigrationLedgerError::CatalogInvalid),
        (
            8,
            boolean(true),
            SimulationMigrationLedgerError::CatalogInvalid,
        ),
        (
            9,
            boolean(true),
            SimulationMigrationLedgerError::CatalogInvalid,
        ),
        (
            13,
            boolean(true),
            SimulationMigrationLedgerError::AccessRejected,
        ),
        (
            19,
            boolean(true),
            SimulationMigrationLedgerError::AccessRejected,
        ),
    ];
    for (index, cell, expected_error) in cases {
        let mut row = relation_row();
        row.cells[index] = cell;
        let mut fake = FakeTransport::new(successful_steps());
        fake.set_query_rows(
            Statement::Relation {
                public_schema_oid: TEST_SCHEMA_OID,
            },
            vec![row],
        );
        let canceled = AtomicBool::new(false);
        let result = run_observer(&mut fake, &canceled, TEST_TIMEOUT_MS);
        assert!(matches!(result, Err(error) if error == expected_error));
        assert!(fake.closed);
    }
}

#[test]
fn relation_oid_checked_bounds_and_upper_boundary_flow_through_observer() {
    for raw_oid in [-1_i64, 0, i64::from(u32::MAX) + 1] {
        let mut row = relation_row();
        row.cells[3] = i64_value(raw_oid);
        let mut fake = FakeTransport::new(successful_steps());
        fake.set_query_rows(
            Statement::Relation {
                public_schema_oid: TEST_SCHEMA_OID,
            },
            vec![row],
        );
        let canceled = AtomicBool::new(false);
        let result = run_observer(&mut fake, &canceled, TEST_TIMEOUT_MS);
        assert!(matches!(
            result,
            Err(SimulationMigrationLedgerError::CatalogInvalid)
        ));
        assert!(fake.events.contains(&Statement::Rollback));
        assert!(!fake.events.contains(&Statement::Commit));
        assert!(fake.closed);
    }

    let mut fake = FakeTransport::new(successful_steps_with_relation_oid(u32::MAX));
    let canceled = AtomicBool::new(false);
    let observation = run_observer(&mut fake, &canceled, TEST_TIMEOUT_MS).unwrap();
    assert_eq!(observation.ledger_oid(), u32::MAX);
    assert!(fake.events.contains(&Statement::Commit));
    assert!(!fake.events.contains(&Statement::Rollback));
    assert!(fake.closed);
}

fn column_projection_aliases(sql: &str) -> Vec<&str> {
    let select_start = sql.find("SELECT").expect("fixed column SQL has SELECT") + "SELECT".len();
    let bytes = sql.as_bytes();
    let mut index = select_start;
    let mut projection_start = select_start;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut projections = Vec::new();
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if byte == b'\'' {
                if bytes.get(index + 1) == Some(&b'\'') {
                    index += 2;
                    continue;
                }
                in_string = false;
            }
            index += 1;
            continue;
        }
        if byte == b'\'' {
            in_string = true;
        } else if byte == b'(' {
            depth += 1;
        } else if byte == b')' {
            depth = depth
                .checked_sub(1)
                .expect("balanced column SQL expression");
        } else if depth == 0
            && byte == b'F'
            && sql[index..].starts_with("FROM")
            && (index == 0 || bytes[index - 1].is_ascii_whitespace())
            && bytes
                .get(index + 4)
                .is_some_and(|next| next.is_ascii_whitespace())
        {
            projections.push(sql[projection_start..index].trim());
            break;
        } else if depth == 0 && byte == b',' {
            projections.push(sql[projection_start..index].trim());
            projection_start = index + 1;
        }
        index += 1;
    }
    projections
        .into_iter()
        .map(|projection| {
            projection
                .rsplit_once(" AS ")
                .expect("every fixed column projection has an alias")
                .1
                .trim()
        })
        .collect()
}

#[test]
fn fixed_column_sql_projection_matches_shape_and_common_decoder() {
    let aliases = column_projection_aliases(COLUMNS_SQL);
    assert_eq!(
        aliases,
        vec![
            "relation_oid",
            "relation_name",
            "attnum",
            "column_name",
            "type_namespace",
            "type_name",
            "type_kind",
            "type_base_oid",
            "not_null",
            "is_dropped",
            "generated",
            "identity",
            "dimensions",
            "type_modifier",
            "column_select",
            "column_insert",
            "column_update",
            "column_references",
            "select_grant_option",
            "insert_grant_option",
            "update_grant_option",
            "references_grant_option",
        ]
    );
    assert_eq!(aliases.len(), COLUMN_SHAPE.len());
    let decoded = decode_column(
        column_row(1, "version", "int8"),
        &mut LogicalPayloadBudget::default(),
    )
    .unwrap();
    assert!(decoded.column_select);
    assert!(!decoded.column_insert);
    assert!(!decoded.column_update);
    assert!(!decoded.column_references);
    assert!(!decoded.select_grant_option);
    assert!(!decoded.insert_grant_option);
    assert!(!decoded.update_grant_option);
    assert!(!decoded.references_grant_option);
}

#[test]
fn each_column_grant_option_is_denied_by_common_observer() {
    for index in [18, 19, 20, 21] {
        let mut rows = columns_rows();
        rows[0].cells[index] = boolean(true);
        let mut fake = FakeTransport::new(successful_steps());
        fake.set_query_rows(
            Statement::Columns {
                ledger_oid: TEST_LEDGER_OID,
            },
            rows,
        );
        let canceled = AtomicBool::new(false);
        let result = run_observer(&mut fake, &canceled, TEST_TIMEOUT_MS);
        assert!(matches!(
            result,
            Err(SimulationMigrationLedgerError::AccessRejected)
        ));
        assert!(fake.events.contains(&Statement::Rollback));
        assert!(!fake.events.contains(&Statement::Commit));
        assert!(fake.closed);
    }
}

#[test]
fn exact_column_types_nullability_and_worker_privileges_are_checked() {
    let cases = [
        (
            5,
            text("varchar"),
            SimulationMigrationLedgerError::CatalogInvalid,
        ),
        (
            8,
            boolean(false),
            SimulationMigrationLedgerError::CatalogInvalid,
        ),
        (
            9,
            boolean(true),
            SimulationMigrationLedgerError::CatalogInvalid,
        ),
        (
            14,
            boolean(false),
            SimulationMigrationLedgerError::AccessRejected,
        ),
        (
            16,
            boolean(true),
            SimulationMigrationLedgerError::AccessRejected,
        ),
        (
            20,
            boolean(true),
            SimulationMigrationLedgerError::AccessRejected,
        ),
    ];
    for (index, cell, expected_error) in cases {
        let mut rows = columns_rows();
        rows[0].cells[index] = cell;
        let mut fake = FakeTransport::new(successful_steps());
        fake.set_query_rows(
            Statement::Columns {
                ledger_oid: TEST_LEDGER_OID,
            },
            rows,
        );
        let canceled = AtomicBool::new(false);
        let result = run_observer(&mut fake, &canceled, TEST_TIMEOUT_MS);
        assert!(matches!(result, Err(error) if error == expected_error));
        assert!(fake.closed);
    }

    let mut extra = columns_rows();
    extra.push(column_row(7, "extra", "text"));
    let mut fake = FakeTransport::new(successful_steps());
    fake.set_query_rows(
        Statement::Columns {
            ledger_oid: TEST_LEDGER_OID,
        },
        extra,
    );
    let canceled = AtomicBool::new(false);
    assert!(matches!(
        run_observer(&mut fake, &canceled, TEST_TIMEOUT_MS),
        Err(SimulationMigrationLedgerError::LimitExceeded)
    ));
}

#[test]
fn primary_key_metadata_requires_one_valid_public_version_key() {
    let valid_relation_oid = validate_relation(
        decode_relation(relation_row(), &mut LogicalPayloadBudget::default()).unwrap(),
        test_context_evidence(),
    )
    .unwrap();
    let cases = [
        (6, i32_value(2)),
        (7, i32_value(2)),
        (9, boolean(true)),
        (12, i32_value(2)),
        (15, boolean(false)),
        (19, boolean(false)),
        (21, i64_value(999)),
        (23, text("hash")),
        (25, i64_value(i64::from(TEST_WORKER_OID))),
    ];
    for (index, cell) in cases {
        let mut row = primary_key_row();
        row.cells[index] = cell;
        let decoded = decode_primary_key(row, &mut LogicalPayloadBudget::default()).unwrap();
        assert!(matches!(
            validate_primary_key(decoded, test_context_evidence(), valid_relation_oid),
            Err(SimulationMigrationLedgerError::CatalogInvalid)
        ));
    }
}

fn decoded_acls(rows: Vec<RawRow>) -> Vec<RawAclRow> {
    let mut budget = LogicalPayloadBudget::default();
    rows.into_iter()
        .map(|row| decode_acl(row, &mut budget).unwrap())
        .collect()
}

#[test]
fn raw_acl_requires_exact_direct_worker_select_and_rejects_public_or_extra_grants() {
    let valid = decoded_acls(vec![table_select_acl()]);
    assert!(validate_raw_acls(&valid, test_context_evidence(), TEST_LEDGER_OID).is_ok());

    let mut public_grant = table_select_acl();
    public_grant.cells[3] = i64_value(0);
    assert!(matches!(
        validate_raw_acls(
            &decoded_acls(vec![public_grant]),
            test_context_evidence(),
            TEST_LEDGER_OID
        ),
        Err(SimulationMigrationLedgerError::AccessRejected)
    ));

    let mut grantable = table_select_acl();
    grantable.cells[6] = boolean(true);
    assert!(matches!(
        validate_raw_acls(
            &decoded_acls(vec![grantable]),
            test_context_evidence(),
            TEST_LEDGER_OID
        ),
        Err(SimulationMigrationLedgerError::AccessRejected)
    ));

    let mut wrong_grantor = table_select_acl();
    wrong_grantor.cells[4] = i64_value(11);
    assert!(matches!(
        validate_raw_acls(
            &decoded_acls(vec![wrong_grantor]),
            test_context_evidence(),
            TEST_LEDGER_OID
        ),
        Err(SimulationMigrationLedgerError::AccessRejected)
    ));

    assert!(matches!(
        validate_raw_acls(
            &decoded_acls(vec![table_select_acl(), table_select_acl()]),
            test_context_evidence(),
            TEST_LEDGER_OID
        ),
        Err(SimulationMigrationLedgerError::AccessRejected)
    ));

    let mut worker_insert = table_select_acl();
    worker_insert.cells[5] = text("INSERT");
    assert!(matches!(
        validate_raw_acls(
            &decoded_acls(vec![table_select_acl(), worker_insert]),
            test_context_evidence(),
            TEST_LEDGER_OID
        ),
        Err(SimulationMigrationLedgerError::AccessRejected)
    ));

    let mut worker_column_select = table_select_acl();
    worker_column_select.cells[0] = i32_value(4);
    worker_column_select.cells[2] = i32_value(1);
    assert!(matches!(
        validate_raw_acls(
            &decoded_acls(vec![table_select_acl(), worker_column_select]),
            test_context_evidence(),
            TEST_LEDGER_OID
        ),
        Err(SimulationMigrationLedgerError::AccessRejected)
    ));

    let mut invalid_privilege = table_select_acl();
    invalid_privilege.cells[5] = text("EXECUTE");
    assert!(matches!(
        validate_raw_acls(
            &decoded_acls(vec![invalid_privilege]),
            test_context_evidence(),
            TEST_LEDGER_OID
        ),
        Err(SimulationMigrationLedgerError::CatalogInvalid)
    ));
}

#[test]
fn logical_payload_budget_charges_cell_kinds_cumulatively_and_checks_overflow() {
    let cases = [
        (RawCell::Text("abc".to_owned()), 3),
        (RawCell::Bytes(vec![1, 2]), 2),
        (RawCell::Bool(true), 1),
        (RawCell::I32(7), 4),
        (RawCell::I64(9), 8),
        (RawCell::Null, 1),
        (RawCell::WrongType, 1),
    ];
    for (cell, expected_charge) in cases {
        let mut budget = LogicalPayloadBudget::default();
        budget.charge(&cell).unwrap();
        assert_eq!(budget.used, expected_charge);
    }

    let mut cumulative = LogicalPayloadBudget::default();
    for _ in 0..PAYLOAD_LIMIT - 1 {
        cumulative.charge(&RawCell::Bool(false)).unwrap();
    }
    assert_eq!(cumulative.used, PAYLOAD_LIMIT - 1);
    cumulative.charge(&RawCell::Bool(false)).unwrap();
    assert_eq!(cumulative.used, PAYLOAD_LIMIT);
    assert!(matches!(
        cumulative.charge(&RawCell::Null),
        Err(SimulationMigrationLedgerError::LimitExceeded)
    ));

    let mut overflow = LogicalPayloadBudget { used: usize::MAX };
    assert!(matches!(
        overflow.charge(&RawCell::Bool(false)),
        Err(SimulationMigrationLedgerError::LimitExceeded)
    ));
}

#[test]
fn raw_decoder_null_wrong_type_and_payload_bounds_share_production_path() {
    let mut null_checksum = ledger_rows();
    null_checksum[0].cells[4] = RawCell::Null;
    assert!(matches!(
        validate_ledger(
            decoded_ledger_rows(null_checksum),
            test_ledger_identity(),
            validate_embedded_baseline().unwrap()
        ),
        Err(SimulationMigrationLedgerError::ChecksumMismatch)
    ));

    let mut wrong_type = ledger_row(0);
    wrong_type.cells[1] = RawCell::WrongType;
    assert!(matches!(
        decode_ledger(wrong_type, &mut LogicalPayloadBudget::default()),
        Err(SimulationMigrationLedgerError::DecodeFailed)
    ));

    let mut oversized = context_row();
    oversized.cells[0] = RawCell::Text("x".repeat(PAYLOAD_LIMIT + 1));
    assert!(matches!(
        decode_context(oversized, &mut LogicalPayloadBudget::default()),
        Err(SimulationMigrationLedgerError::LimitExceeded)
    ));

    let mut long_identifier = context_row();
    long_identifier.cells[0] = RawCell::Text("x".repeat(64));
    assert!(matches!(
        decode_context(long_identifier, &mut LogicalPayloadBudget::default()),
        Err(SimulationMigrationLedgerError::DecodeFailed)
    ));
}

fn ledger_row_with(version: i64, checksum: Vec<u8>) -> RawRow {
    raw_row(
        RawCategory::Ledger,
        vec![
            i64_value(i64::from(TEST_LEDGER_OID)),
            i64_value(version),
            boolean(true),
            i32_value(48),
            RawCell::Bytes(checksum),
        ],
    )
}

fn decoded_ledger_rows(rows: Vec<RawRow>) -> Vec<LedgerRow> {
    let mut budget = LogicalPayloadBudget::default();
    rows.into_iter()
        .map(|row| decode_ledger(row, &mut budget).unwrap())
        .collect()
}

fn test_ledger_identity() -> LedgerIdentity {
    LedgerIdentity {
        database_oid: TEST_DATABASE_OID,
        worker_oid: TEST_WORKER_OID,
        public_schema_oid: TEST_SCHEMA_OID,
        migration_owner_oid: TEST_OWNER_OID,
        ledger_oid: TEST_LEDGER_OID,
    }
}

#[test]
fn ledger_requires_exact_order_success_tableoid_and_raw_checksum() {
    let baseline = validate_embedded_baseline().unwrap();
    let valid = validate_ledger(
        decoded_ledger_rows(ledger_rows()),
        test_ledger_identity(),
        baseline,
    )
    .unwrap();
    assert_eq!(valid.row_count, 64);
    assert!(valid.digest_text.starts_with("sha256:"));

    let mut missing = ledger_rows();
    missing.pop();
    assert!(matches!(
        validate_ledger(
            decoded_ledger_rows(missing),
            test_ledger_identity(),
            baseline
        ),
        Err(SimulationMigrationLedgerError::MissingPredecessor)
    ));

    let mut duplicate = ledger_rows();
    duplicate[2] = ledger_row(1);
    assert!(matches!(
        validate_ledger(
            decoded_ledger_rows(duplicate),
            test_ledger_identity(),
            baseline
        ),
        Err(SimulationMigrationLedgerError::LedgerInvalid)
    ));

    let mut gap = ledger_rows();
    gap[2] = ledger_row(3);
    assert!(matches!(
        validate_ledger(decoded_ledger_rows(gap), test_ledger_identity(), baseline),
        Err(SimulationMigrationLedgerError::MissingPredecessor)
    ));

    let mut dirty = ledger_rows();
    dirty[5].cells[2] = boolean(false);
    assert!(matches!(
        validate_ledger(decoded_ledger_rows(dirty), test_ledger_identity(), baseline),
        Err(SimulationMigrationLedgerError::LedgerDirty)
    ));

    let mut wrong_table = ledger_rows();
    wrong_table[4].cells[0] = i64_value(999);
    assert!(matches!(
        validate_ledger(
            decoded_ledger_rows(wrong_table),
            test_ledger_identity(),
            baseline
        ),
        Err(SimulationMigrationLedgerError::LedgerInvalid)
    ));

    let mut wrong_checksum = ledger_rows();
    if let RawCell::Bytes(bytes) = &mut wrong_checksum[8].cells[4] {
        bytes[0] ^= 1;
    }
    assert!(matches!(
        validate_ledger(
            decoded_ledger_rows(wrong_checksum),
            test_ledger_identity(),
            baseline
        ),
        Err(SimulationMigrationLedgerError::ChecksumMismatch)
    ));

    let mut wrong_checksum_length = ledger_rows();
    wrong_checksum_length[9].cells[3] = i32_value(47);
    assert!(matches!(
        validate_ledger(
            decoded_ledger_rows(wrong_checksum_length),
            test_ledger_identity(),
            baseline
        ),
        Err(SimulationMigrationLedgerError::ChecksumMismatch)
    ));

    let mut null_checksum = ledger_rows();
    null_checksum[10].cells[4] = RawCell::Null;
    assert!(matches!(
        validate_ledger(
            decoded_ledger_rows(null_checksum),
            test_ledger_identity(),
            baseline
        ),
        Err(SimulationMigrationLedgerError::ChecksumMismatch)
    ));
}

#[test]
fn ledger_unknown_65_is_distinguished_from_duplicate_65() {
    let mut unknown = ledger_rows();
    unknown.push(ledger_row_with(65, from_hex(BASELINE[63].sha384_hex)));
    assert!(matches!(
        validate_ledger(
            decoded_ledger_rows(unknown),
            test_ledger_identity(),
            validate_embedded_baseline().unwrap()
        ),
        Err(SimulationMigrationLedgerError::UnknownBaselineVersion)
    ));

    let mut duplicate = ledger_rows();
    duplicate.push(ledger_row(63));
    assert!(matches!(
        validate_ledger(
            decoded_ledger_rows(duplicate),
            test_ledger_identity(),
            validate_embedded_baseline().unwrap()
        ),
        Err(SimulationMigrationLedgerError::LedgerInvalid)
    ));

    let mut fake = FakeTransport::new(successful_steps());
    let mut query_rows = ledger_rows();
    query_rows.push(ledger_row_with(65, from_hex(BASELINE[63].sha384_hex)));
    fake.set_query_rows(Statement::Ledger, query_rows);
    let canceled = AtomicBool::new(false);
    assert!(matches!(
        run_observer(&mut fake, &canceled, TEST_TIMEOUT_MS),
        Err(SimulationMigrationLedgerError::UnknownBaselineVersion)
    ));
    assert!(fake.events.contains(&Statement::Rollback));
}

#[test]
fn query_fetch_accept_limits_are_enforced_before_row_processing() {
    let mut context_duplicates = FakeTransport::new(successful_steps());
    context_duplicates.set_query_rows(Statement::Context, vec![context_row(), context_row()]);
    let canceled = AtomicBool::new(false);
    assert!(matches!(
        run_observer(&mut context_duplicates, &canceled, TEST_TIMEOUT_MS),
        Err(SimulationMigrationLedgerError::CatalogInvalid)
    ));

    let mut context_overfetch = FakeTransport::new(successful_steps());
    context_overfetch.set_query_rows(
        Statement::Context,
        vec![context_row(), context_row(), context_row()],
    );
    let canceled = AtomicBool::new(false);
    assert!(matches!(
        run_observer(&mut context_overfetch, &canceled, TEST_TIMEOUT_MS),
        Err(SimulationMigrationLedgerError::LimitExceeded)
    ));

    let mut acl_over_accept = FakeTransport::new(successful_steps());
    acl_over_accept.set_query_rows(
        Statement::RawAcls {
            database_oid: TEST_DATABASE_OID,
            public_schema_oid: TEST_SCHEMA_OID,
            ledger_oid: TEST_LEDGER_OID,
            worker_oid: TEST_WORKER_OID,
        },
        vec![table_select_acl(); RAW_ACL_ACCEPT_LIMIT + 1],
    );
    let canceled = AtomicBool::new(false);
    assert!(matches!(
        run_observer(&mut acl_over_accept, &canceled, TEST_TIMEOUT_MS),
        Err(SimulationMigrationLedgerError::LimitExceeded)
    ));

    let mut acl_overfetch = FakeTransport::new(successful_steps());
    acl_overfetch.set_query_rows(
        Statement::RawAcls {
            database_oid: TEST_DATABASE_OID,
            public_schema_oid: TEST_SCHEMA_OID,
            ledger_oid: TEST_LEDGER_OID,
            worker_oid: TEST_WORKER_OID,
        },
        vec![table_select_acl(); RAW_ACL_FETCH_LIMIT + 1],
    );
    let canceled = AtomicBool::new(false);
    assert!(matches!(
        run_observer(&mut acl_overfetch, &canceled, TEST_TIMEOUT_MS),
        Err(SimulationMigrationLedgerError::LimitExceeded)
    ));
}

#[test]
fn commit_error_or_odd_result_never_rolls_back_or_retries() {
    let mut failed_commit = FakeTransport::new(successful_steps());
    failed_commit.step_mut(Statement::Commit).result = Err(TransportFailure::Database);
    let canceled = AtomicBool::new(false);
    assert!(matches!(
        run_observer(&mut failed_commit, &canceled, TEST_TIMEOUT_MS),
        Err(SimulationMigrationLedgerError::CompletionUnconfirmed)
    ));
    assert!(!failed_commit.events.contains(&Statement::Rollback));
    assert!(failed_commit.closed);

    let mut odd_commit = FakeTransport::new(successful_steps());
    odd_commit.step_mut(Statement::Commit).result = Ok(vec![context_row()]);
    let canceled = AtomicBool::new(false);
    assert!(matches!(
        run_observer(&mut odd_commit, &canceled, TEST_TIMEOUT_MS),
        Err(SimulationMigrationLedgerError::CompletionUnconfirmed)
    ));
    assert!(!odd_commit.events.contains(&Statement::Rollback));

    let mut both_unknown = FakeTransport::new(successful_steps());
    both_unknown.step_mut(Statement::Commit).result = Err(TransportFailure::Database);
    both_unknown.fail_close = true;
    let canceled = AtomicBool::new(false);
    assert!(matches!(
        run_observer(&mut both_unknown, &canceled, TEST_TIMEOUT_MS),
        Err(SimulationMigrationLedgerError::CompletionAndCleanupUnconfirmed)
    ));
}

#[test]
fn confirmed_commit_still_requires_close_and_postcommit_cancel_withholds_evidence() {
    let mut close_unknown = FakeTransport::new(successful_steps());
    close_unknown.fail_close = true;
    let canceled = AtomicBool::new(false);
    assert!(matches!(
        run_observer(&mut close_unknown, &canceled, TEST_TIMEOUT_MS),
        Err(SimulationMigrationLedgerError::CleanupUnconfirmed)
    ));

    let canceled = Arc::new(AtomicBool::new(false));
    let mut after_commit = FakeTransport::new(successful_steps());
    after_commit.cancel_flag = Some(Arc::clone(&canceled));
    after_commit.set_cancel_after(Statement::Commit);
    assert!(matches!(
        run_observer(&mut after_commit, canceled.as_ref(), TEST_TIMEOUT_MS),
        Err(SimulationMigrationLedgerError::Canceled)
    ));
    assert!(after_commit.closed);
    assert!(!after_commit.events.contains(&Statement::Rollback));

    let canceled_after_close = Arc::new(AtomicBool::new(false));
    let mut after_close = FakeTransport::new(successful_steps());
    after_close.cancel_flag = Some(Arc::clone(&canceled_after_close));
    after_close.cancel_after_close = true;
    assert!(matches!(
        run_observer(
            &mut after_close,
            canceled_after_close.as_ref(),
            TEST_TIMEOUT_MS
        ),
        Err(SimulationMigrationLedgerError::Canceled)
    ));
    assert!(after_close.closed);
    assert!(!after_close.events.contains(&Statement::Rollback));
}

#[test]
fn cancellation_does_not_erase_postcommit_cleanup_uncertainty() {
    let canceled = Arc::new(AtomicBool::new(false));
    let mut fake = FakeTransport::new(successful_steps());
    fake.cancel_flag = Some(Arc::clone(&canceled));
    fake.set_cancel_after(Statement::Commit);
    fake.fail_close = true;
    assert!(matches!(
        run_observer(&mut fake, canceled.as_ref(), TEST_TIMEOUT_MS),
        Err(SimulationMigrationLedgerError::CleanupUnconfirmed)
    ));
}

#[test]
fn raw_wrong_type_is_decoded_before_policy_rejection() {
    let mut fake = FakeTransport::new(successful_steps());
    let mut membership = membership_row();
    membership.cells[0] = RawCell::WrongType;
    fake.set_query_rows(
        Statement::Memberships {
            worker_oid: TEST_WORKER_OID,
        },
        vec![membership],
    );
    let canceled = AtomicBool::new(false);
    assert!(matches!(
        run_observer(&mut fake, &canceled, TEST_TIMEOUT_MS),
        Err(SimulationMigrationLedgerError::DecodeFailed)
    ));
    assert!(fake.closed);
}

#[test]
fn raw_acl_rejects_invalid_oids_attnums_and_missing_direct_grant() {
    let cases = [
        {
            let mut row = table_select_acl();
            row.cells[3] = i64_value(-1);
            row
        },
        {
            let mut row = table_select_acl();
            row.cells[4] = i64_value(0);
            row
        },
        {
            let mut row = table_select_acl();
            row.cells[1] = i64_value(999);
            row
        },
        {
            let mut row = table_select_acl();
            row.cells[0] = i32_value(9);
            row
        },
        {
            let mut row = table_select_acl();
            row.cells[0] = i32_value(4);
            row.cells[2] = i32_value(0);
            row
        },
    ];
    for row in cases {
        assert!(matches!(
            validate_raw_acls(
                &decoded_acls(vec![row]),
                test_context_evidence(),
                TEST_LEDGER_OID
            ),
            Err(SimulationMigrationLedgerError::CatalogInvalid)
        ));
    }
    assert!(matches!(
        validate_raw_acls(&[], test_context_evidence(), TEST_LEDGER_OID),
        Err(SimulationMigrationLedgerError::AccessRejected)
    ));
}

#[test]
fn ledger_rejects_nonpositive_version_wrong_checksum_type_and_wrong_width() {
    let baseline = validate_embedded_baseline().unwrap();

    let mut nonpositive = ledger_rows();
    nonpositive[0].cells[1] = i64_value(0);
    assert!(matches!(
        validate_ledger(
            decoded_ledger_rows(nonpositive),
            test_ledger_identity(),
            baseline
        ),
        Err(SimulationMigrationLedgerError::LedgerInvalid)
    ));

    let mut wrong_type = ledger_rows();
    wrong_type[0].cells[4] = RawCell::WrongType;
    let mut wrong_type_fake = FakeTransport::new(successful_steps());
    wrong_type_fake.set_query_rows(Statement::Ledger, wrong_type);
    let canceled = AtomicBool::new(false);
    let wrong_type_result = run_observer(&mut wrong_type_fake, &canceled, TEST_TIMEOUT_MS);
    assert!(matches!(
        wrong_type_result,
        Err(SimulationMigrationLedgerError::DecodeFailed)
    ));
    assert!(wrong_type_fake.events.contains(&Statement::Rollback));
    assert!(!wrong_type_fake.events.contains(&Statement::Commit));
    assert!(wrong_type_fake.closed);

    let mut wrong_width = ledger_rows();
    wrong_width[0].cells[4] = RawCell::Bytes(vec![0u8; 47]);
    assert!(matches!(
        validate_ledger(
            decoded_ledger_rows(wrong_width),
            test_ledger_identity(),
            baseline
        ),
        Err(SimulationMigrationLedgerError::ChecksumMismatch)
    ));
}
#[test]
fn command_result_discards_row_payload_and_rejects_presence() {
    assert!(super::classify_command_result(None::<String>).is_empty());
    let rows = super::classify_command_result(Some(String::from("sentinel")));
    assert_eq!(rows.len(), 1);
    assert!(rows[0].cells.is_empty());
}
