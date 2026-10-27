// Bounded fake loops and indexed policy mutations are intentional fixtures.
#![allow(
    dead_code,
    clippy::collapsible_if,
    clippy::never_loop,
    clippy::needless_range_loop
)]

use std::future::Future;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::task::{Context, Poll, Waker};

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use super::*;

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
const TEST_RELATION_OIDS: [u32; 2] = [52_100, 52_101];
const TEST_POLICY_OID_BASE: u32 = 53_000;

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
        value: value.to_owned(),
    }
}

fn text_with_original(value: &str, original_bytes: Option<i64>, is_null: bool) -> RawCell {
    RawCell::Text {
        is_null,
        original_bytes,
        value: value.to_owned(),
    }
}

fn null_text() -> RawCell {
    text_with_original("", None, true)
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

fn role_oid_for(name: &str) -> u32 {
    match name {
        "worker" => TEST_WORKER_OID,
        "migration_owner" => TEST_OWNER_OID,
        "app" => TEST_APP_OID,
        "admin" => TEST_ADMIN_OID,
        "audit_writer" => TEST_AUDIT_OID,
        "research_writer" => TEST_RESEARCH_OID,
        _ => 0,
    }
}

fn role_rows() -> Vec<RawRow> {
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
                    i64_cell(i64::from(role_oid_for(name))),
                ],
            )
        })
        .collect()
}

fn relation_row(slot: usize, relation_oid: u32) -> RawRow {
    row(
        RawCategory::Relation,
        vec![
            i64_cell(i64::from(TEST_SCHEMA_OID)),
            text("public"),
            text(RELATION_SPECS[slot].name),
            i64_cell(i64::from(relation_oid)),
            i64_cell(i64::from(TEST_OWNER_OID)),
            text("migration_owner"),
            text("r"),
            text("p"),
            bool_cell(false),
            bool_cell(true),
            bool_cell(true),
        ],
    )
}

fn relation_rows() -> Vec<RawRow> {
    vec![
        relation_row(0, TEST_RELATION_OIDS[0]),
        relation_row(1, TEST_RELATION_OIDS[1]),
    ]
}

fn policy_oid_at(index: usize) -> u32 {
    TEST_POLICY_OID_BASE + index as u32
}

fn policy_header_row(index: usize) -> RawRow {
    let spec = POLICY_SPECS[index];
    row(
        RawCategory::PolicyHeader,
        vec![
            i64_cell(i64::from(policy_oid_at(index))),
            i64_cell(i64::from(TEST_RELATION_OIDS[spec.relation_slot])),
            text(spec.name),
            text(spec.command),
            bool_cell(spec.permissive),
            bool_cell(false),
            i32_cell(1),
            i32_cell(1),
            i32_cell(1),
            bool_cell(spec.using_is_null),
            bool_cell(spec.with_check_is_null),
        ],
    )
}

fn policy_header_rows() -> Vec<RawRow> {
    (0..POLICY_SPECS.len()).map(policy_header_row).collect()
}

fn policy_role_row(index: usize) -> RawRow {
    let spec = POLICY_SPECS[index];
    row(
        RawCategory::PolicyRole,
        vec![
            i64_cell(i64::from(policy_oid_at(index))),
            i64_cell(i64::from(TEST_RELATION_OIDS[spec.relation_slot])),
            i64_cell(1),
            i64_cell(i64::from(role_oid_for(spec.role))),
        ],
    )
}

fn policy_role_rows() -> Vec<RawRow> {
    (0..POLICY_SPECS.len()).map(policy_role_row).collect()
}

fn membership_row(member_oid: u32) -> RawRow {
    row(
        RawCategory::Membership,
        vec![
            i64_cell(i64::from(member_oid)),
            i64_cell(i64::from(TEST_APP_OID)),
            i64_cell(i64::from(TEST_OWNER_OID)),
            bool_cell(false),
            bool_cell(false),
            bool_cell(true),
        ],
    )
}

fn inheritance_row(child_oid: u32, parent_oid: u32) -> RawRow {
    row(
        RawCategory::Inheritance,
        vec![
            i64_cell(i64::from(child_oid)),
            i64_cell(i64::from(parent_oid)),
        ],
    )
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
    relations: Vec<RawRow>,
    inheritance: Vec<RawRow>,
    policy_headers: Vec<RawRow>,
    policy_roles: Vec<RawRow>,
    context_reads: usize,
    fail_at: Option<Point>,
    typed_failure_at: Option<(StatementKind, TransportFailure)>,
    rollback_fails: bool,
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
            relations: relation_rows(),
            inheritance: Vec::new(),
            policy_headers: policy_header_rows(),
            policy_roles: policy_role_rows(),
            context_reads: 0,
            fail_at: None,
            typed_failure_at: None,
            rollback_fails: false,
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
        if let Some((failed_kind, failure)) = self.typed_failure_at {
            if failed_kind == kind {
                return Err(failure);
            }
        }
        if self.fail_at == Some(Point::Statement(kind))
            || (kind == StatementKind::Rollback && self.rollback_fails)
        {
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
            Statement::Relations => Ok(self.relations.clone()),
            Statement::Inheritance { .. } => Ok(self.inheritance.clone()),
            Statement::PolicyHeaders { .. } => Ok(self.policy_headers.clone()),
            Statement::PolicyRoles { .. } => Ok(self.policy_roles.clone()),
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
) -> Result<SimulationRlsMetadataObservation, SimulationRlsMetadataError> {
    let config = valid_config();
    let canceled = Arc::clone(&fake.canceled);
    block_on(observe_with_transport(fake, &config, &canceled))
}

fn assert_error_primary(
    fake: &mut FakeTransport,
    expected: PrimaryFailure,
) -> SimulationRlsMetadataError {
    match observe(fake) {
        Ok(_) => panic!("unexpected RLS metadata observation"),
        Err(error) => {
            assert_eq!(error.primary, expected);
            error
        }
    }
}

fn assert_canceled_transport_failure(
    fake: &mut FakeTransport,
    expected_primary: PrimaryFailure,
    expected_prefix: &[Statement],
    rollback: CleanupState,
    close: CleanupState,
) -> SimulationRlsMetadataError {
    let error = assert_error_primary(fake, expected_primary);
    assert!(error.canceled);
    assert_eq!(error.completion, CompletionState::NotAttempted);
    assert_eq!(error.rollback, rollback);
    assert_eq!(error.close, close);
    let events = fake.event_log();
    let mut expected_statements = expected_prefix.to_vec();
    expected_statements.push(Statement::Rollback);
    assert_eq!(statements_seen(&events), expected_statements);
    assert!(!statement_seen(&events, StatementKind::Commit));
    let rollback_at = events
        .iter()
        .position(|event| *event == Event::Statement(Statement::Rollback))
        .expect("rollback attempted after a precommit failure");
    assert_eq!(events.get(rollback_at + 1), Some(&Event::Close));
    assert_eq!(events.last(), Some(&Event::Close));
    error
}

fn precommit_statement_flow() -> [Statement; 10] {
    [
        Statement::Begin,
        Statement::Configure {
            timeout_ms: 123_000,
        },
        Statement::Context,
        Statement::Roles,
        Statement::Memberships {
            worker_oid: TEST_WORKER_OID,
        },
        Statement::Relations,
        Statement::Inheritance {
            relation_oids: TEST_RELATION_OIDS,
        },
        Statement::PolicyHeaders {
            relation_oids: TEST_RELATION_OIDS,
        },
        Statement::PolicyRoles {
            relation_oids: TEST_RELATION_OIDS,
        },
        Statement::FinalContext,
    ]
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

fn statement_seen(events: &[Event], kind: StatementKind) -> bool {
    events
        .iter()
        .any(|event| matches!(event, Event::Statement(statement) if statement.kind() == kind))
}

fn assert_no_observation(fake: &mut FakeTransport) {
    assert!(observe(fake).is_err());
    assert!(fake.event_log().contains(&Event::Close));
}

fn replace_cell(row: &mut RawRow, index: usize, value: RawCell) {
    row.cells[index] = value;
}

fn top_level_from(sql: &str, start: usize) -> Option<usize> {
    let bytes = sql.as_bytes();
    let mut depth = 0_i32;
    let mut quote = None;
    let mut index = start;
    while index < bytes.len() {
        let byte = bytes[index];
        if let Some(active_quote) = quote {
            if byte == active_quote {
                if active_quote == b'\'' && bytes.get(index + 1) == Some(&b'\'') {
                    index += 2;
                    continue;
                }
                quote = None;
            }
            index += 1;
            continue;
        }
        match byte {
            b'\'' | b'"' => quote = Some(byte),
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
        if depth == 0
            && index + 4 <= bytes.len()
            && sql[index..index + 4].eq_ignore_ascii_case("FROM")
            && (index == 0 || !bytes[index - 1].is_ascii_alphanumeric())
            && bytes
                .get(index + 4)
                .is_some_and(|next| next.is_ascii_whitespace())
        {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn top_level_expressions(projection: &str) -> Vec<&str> {
    let bytes = projection.as_bytes();
    let mut values = Vec::new();
    let mut depth = 0_i32;
    let mut quote = None;
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if let Some(active_quote) = quote {
            if byte == active_quote {
                if active_quote == b'\'' && bytes.get(index + 1) == Some(&b'\'') {
                    index += 2;
                    continue;
                }
                quote = None;
            }
            index += 1;
            continue;
        }
        match byte {
            b'\'' | b'"' => quote = Some(byte),
            b'(' => depth += 1,
            b')' => depth -= 1,
            b',' if depth == 0 => {
                values.push(projection[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
        index += 1;
    }
    if start < projection.len() {
        values.push(projection[start..].trim());
    }
    values
}

fn projection_alias_and_type(expression: &str) -> Option<(String, String)> {
    let normalized = expression.split_whitespace().collect::<Vec<_>>().join(" ");
    let (value, alias) = normalized.rsplit_once(" AS ")?;
    let alias = alias.trim();
    let (_, cast_type) = value.rsplit_once("::pg_catalog.")?;
    let type_name = cast_type.split_whitespace().next()?.to_ascii_uppercase();
    Some((alias.to_owned(), type_name))
}

fn expected_projection(shape: &[PgCellKind]) -> Vec<(&'static str, &'static str)> {
    let mut result = Vec::new();
    for kind in shape {
        match kind {
            PgCellKind::Bool(alias) => result.push((*alias, "BOOL")),
            PgCellKind::I32(alias) | PgCellKind::OptionalI32(alias) => {
                result.push((*alias, "INT4"));
            }
            PgCellKind::I64(alias) | PgCellKind::OptionalI64(alias) => {
                result.push((*alias, "INT8"));
            }
            PgCellKind::Text {
                null_alias,
                bytes_alias,
                value_alias,
            } => {
                result.push((*null_alias, "BOOL"));
                result.push((*bytes_alias, "INT8"));
                result.push((*value_alias, "TEXT"));
            }
        }
    }
    result
}

fn assert_projection(sql: &str, shape: &[PgCellKind]) {
    let trimmed = sql.trim_start();
    let after_select = trimmed
        .strip_prefix("SELECT")
        .expect("top-level SELECT")
        .trim_start();
    let from = top_level_from(trimmed, trimmed.len() - after_select.len()).expect("top-level FROM");
    let expressions = top_level_expressions(&trimmed[trimmed.len() - after_select.len()..from]);
    let expected = expected_projection(shape);
    assert_eq!(expressions.len(), expected.len());
    for (expression, (alias, type_name)) in expressions.iter().zip(expected) {
        assert_eq!(
            projection_alias_and_type(expression),
            Some((alias.to_owned(), type_name.to_owned())),
            "projection expression: {expression}"
        );
    }
}

#[test]
fn every_catalog_projection_has_exact_top_level_order_alias_and_cast_type() {
    assert_projection(CONTEXT_SQL, &CONTEXT_SHAPE);
    assert_projection(ROLES_SQL, &ROLE_SHAPE);
    assert_projection(MEMBERSHIPS_SQL, &MEMBERSHIP_SHAPE);
    assert_projection(RELATIONS_SQL, &RELATION_SHAPE);
    assert_projection(INHERITANCE_SQL, &INHERITANCE_SHAPE);
    assert_projection(POLICY_HEADERS_SQL, &POLICY_HEADER_SHAPE);
    assert_projection(POLICY_ROLES_SQL, &POLICY_ROLE_SHAPE);
    assert_projection(FINAL_CONTEXT_SQL, &CONTEXT_SHAPE);
}

#[test]
fn fixed_sql_is_bounded_catalog_only_and_never_reads_expression_or_body_text() {
    assert_eq!(
        RELATION_NAMES,
        [
            "owner_beta_simulation_runs",
            "owner_beta_simulation_result_pairs"
        ]
    );
    assert!(BEGIN_SQL.contains("ISOLATION LEVEL REPEATABLE READ READ ONLY"));
    assert!(SETUP_SQL.contains("search_path"));
    assert!(SETUP_SQL.contains("statement_timeout"));
    assert!(SETUP_SQL.contains("lock_timeout"));
    assert!(CONTEXT_SQL.contains("LIMIT 2"));
    assert!(ROLES_SQL.contains("LIMIT 7"));
    assert!(MEMBERSHIPS_SQL.contains("LIMIT 1"));
    assert!(RELATIONS_SQL.contains("LIMIT 3"));
    assert!(INHERITANCE_SQL.contains("LIMIT 1"));
    assert!(POLICY_HEADERS_SQL.contains("LIMIT 11"));
    assert!(POLICY_ROLES_SQL.contains("LIMIT 11"));
    assert!(
        POLICY_HEADERS_SQL.contains("policy.polrelid IN ($1::pg_catalog.oid, $2::pg_catalog.oid)")
    );
    assert!(!POLICY_HEADERS_SQL.contains("polname ="));
    assert!(!POLICY_HEADERS_SQL.contains("polcmd ="));
    assert!(!POLICY_HEADERS_SQL.contains("polpermissive ="));
    assert!(!POLICY_HEADERS_SQL.contains("unnest"));
    assert!(POLICY_ROLES_SQL.contains("CROSS JOIN LATERAL pg_catalog.unnest(policy.polroles)"));
    assert!(POLICY_ROLES_SQL.contains("WITH ORDINALITY"));
    assert!(!POLICY_ROLES_SQL.contains("polname ="));
    for forbidden in [
        "pg_get_expr",
        "prosrc",
        "prosqlbody",
        "polqual::",
        "polwithcheck::",
    ] {
        assert!(!POLICY_HEADERS_SQL.contains(forbidden));
        assert!(!POLICY_ROLES_SQL.contains(forbidden));
    }
}

#[test]
fn embedded_baseline_identity_relation_roles_and_ten_policy_contracts_are_exact() {
    assert_eq!(POLICY_ID, "r4-simulation-rls-metadata-v1");
    assert_eq!(
        POLICY_FINGERPRINT,
        "sha256:12add1b0db073a458329c7342e3da0b7ea3f26c37b857b6a5b1459e3d5e317c0"
    );
    assert_eq!(
        ROLE_NAMES,
        [
            "worker",
            "migration_owner",
            "app",
            "admin",
            "audit_writer",
            "research_writer"
        ]
    );
    let expected_relations = [
        "owner_beta_simulation_runs",
        "owner_beta_simulation_result_pairs",
    ];
    assert_eq!(RELATION_SPECS.len(), expected_relations.len());
    for (index, (actual, expected)) in RELATION_SPECS.iter().zip(expected_relations).enumerate() {
        assert_eq!(actual.name, expected);
        assert_eq!(actual.slot, index);
    }

    let expected = [
        (
            0,
            "owner_beta_simulation_runs_admin_select",
            "r",
            "admin",
            true,
            false,
            true,
        ),
        (
            0,
            "owner_beta_simulation_runs_app_insert",
            "a",
            "app",
            true,
            true,
            false,
        ),
        (
            0,
            "owner_beta_simulation_runs_app_select",
            "r",
            "app",
            true,
            false,
            true,
        ),
        (
            0,
            "owner_beta_simulation_runs_migration_owner_actor",
            "*",
            "migration_owner",
            true,
            false,
            false,
        ),
        (
            0,
            "owner_beta_simulation_runs_worker_select",
            "r",
            "worker",
            true,
            false,
            true,
        ),
        (
            0,
            "owner_beta_simulation_runs_worker_update",
            "w",
            "worker",
            true,
            false,
            false,
        ),
        (
            1,
            "owner_beta_simulation_pairs_app_select",
            "r",
            "app",
            true,
            false,
            true,
        ),
        (
            1,
            "owner_beta_simulation_pairs_migration_owner_actor",
            "*",
            "migration_owner",
            true,
            false,
            false,
        ),
        (
            1,
            "owner_beta_simulation_pairs_worker_insert",
            "a",
            "worker",
            true,
            true,
            false,
        ),
        (
            1,
            "owner_beta_simulation_pairs_worker_select",
            "r",
            "worker",
            true,
            false,
            true,
        ),
    ];
    assert_eq!(POLICY_SPECS.len(), expected.len());
    for (actual, (slot, name, command, role, permissive, using_null, check_null)) in
        POLICY_SPECS.iter().zip(expected)
    {
        assert_eq!(actual.relation_slot, slot);
        assert_eq!(actual.name, name);
        assert_eq!(actual.command, command);
        assert_eq!(actual.role, role);
        assert_eq!(actual.permissive, permissive);
        assert_eq!(actual.using_is_null, using_null);
        assert_eq!(actual.with_check_is_null, check_null);
    }
}

#[test]
fn good_control_uses_the_shared_decoder_and_returns_only_opaque_metadata() {
    let mut fake = FakeTransport::valid();
    let observed = observe(&mut fake).expect("synthetic metadata control");
    assert_eq!(observed.config_sha256(), valid_config().config_sha256());
    assert_eq!(observed.policy_id(), POLICY_ID);
    assert_eq!(observed.policy_fingerprint(), POLICY_FINGERPRINT);
    assert_eq!(observed.database_oid(), TEST_DATABASE_OID);
    assert_eq!(observed.worker_oid(), TEST_WORKER_OID);
    assert_eq!(observed.public_schema_oid(), TEST_SCHEMA_OID);
    assert_eq!(observed.migration_owner_oid(), TEST_OWNER_OID);
    assert_eq!(observed.relation_oids(), &TEST_RELATION_OIDS);
    assert_eq!(
        statements_seen(&fake.event_log()),
        vec![
            Statement::Begin,
            Statement::Configure {
                timeout_ms: 123_000
            },
            Statement::Context,
            Statement::Roles,
            Statement::Memberships {
                worker_oid: TEST_WORKER_OID
            },
            Statement::Relations,
            Statement::Inheritance {
                relation_oids: TEST_RELATION_OIDS
            },
            Statement::PolicyHeaders {
                relation_oids: TEST_RELATION_OIDS
            },
            Statement::PolicyRoles {
                relation_oids: TEST_RELATION_OIDS
            },
            Statement::FinalContext,
            Statement::Commit,
        ]
    );
    assert_eq!(fake.event_log().last(), Some(&Event::Close));
}

#[test]
fn strict_worker_context_checks_identity_owner_flags_and_grants() {
    let mutations = [
        (0, text("admin")),
        (1, text("admin")),
        (2, text("other_database")),
        (4, text("other_schema")),
        (8, i64_cell(i64::from(TEST_WORKER_OID))),
        (9, i64_cell(i64::from(TEST_WORKER_OID))),
        (10, i64_cell(i64::from(TEST_WORKER_OID))),
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
    ];
    for (index, value) in mutations {
        let mut fake = FakeTransport::valid();
        replace_cell(&mut fake.context, index, value);
        assert_error_primary(&mut fake, PrimaryFailure::PolicyRejected);
        assert_eq!(fake.context_reads, 1);
        assert!(!statement_seen(
            &fake.event_log(),
            StatementKind::PolicyHeaders
        ));
    }
    for index in 5..=7 {
        let mut fake = FakeTransport::valid();
        replace_cell(&mut fake.context, index, i64_cell(0));
        assert_error_primary(&mut fake, PrimaryFailure::CatalogInvalid);
    }
}

#[test]
fn role_resolution_requires_six_exact_distinct_positive_named_roles() {
    let mut missing = FakeTransport::valid();
    missing.roles.remove(3);
    assert_error_primary(&mut missing, PrimaryFailure::CatalogInvalid);

    let mut extra = FakeTransport::valid();
    extra.roles.push(role_rows()[0].clone());
    assert_error_primary(&mut extra, PrimaryFailure::BoundExceeded);

    let mut duplicate_oid = FakeTransport::valid();
    replace_cell(
        &mut duplicate_oid.roles[2],
        3,
        i64_cell(i64::from(TEST_OWNER_OID)),
    );
    assert_error_primary(&mut duplicate_oid, PrimaryFailure::PolicyRejected);

    let mut wrong_ordinal = FakeTransport::valid();
    replace_cell(&mut wrong_ordinal.roles[0], 0, i32_cell(2));
    assert_error_primary(&mut wrong_ordinal, PrimaryFailure::PolicyRejected);

    let mut wrong_expected_name = FakeTransport::valid();
    replace_cell(&mut wrong_expected_name.roles[1], 1, text("admin"));
    assert_error_primary(&mut wrong_expected_name, PrimaryFailure::PolicyRejected);

    let mut wrong_actual_name = FakeTransport::valid();
    replace_cell(&mut wrong_actual_name.roles[4], 2, text("app"));
    assert_error_primary(&mut wrong_actual_name, PrimaryFailure::PolicyRejected);

    let mut null_actual_name = FakeTransport::valid();
    replace_cell(&mut null_actual_name.roles[4], 2, null_text());
    assert_error_primary(&mut null_actual_name, PrimaryFailure::PolicyRejected);

    let mut null_role_oid = FakeTransport::valid();
    replace_cell(&mut null_role_oid.roles[5], 3, RawCell::Null);
    assert_error_primary(&mut null_role_oid, PrimaryFailure::CatalogInvalid);

    let mut role_oid_disagrees_with_context = FakeTransport::valid();
    replace_cell(
        &mut role_oid_disagrees_with_context.roles[0],
        3,
        i64_cell(i64::from(TEST_APP_OID)),
    );
    assert_error_primary(
        &mut role_oid_disagrees_with_context,
        PrimaryFailure::PolicyRejected,
    );
}

#[test]
fn worker_membership_is_zero_even_when_inheritance_is_disabled() {
    let mut fake = FakeTransport::valid();
    fake.memberships.push(membership_row(TEST_WORKER_OID));
    assert_error_primary(&mut fake, PrimaryFailure::PolicyRejected);
    let statements = statements_seen(&fake.event_log());
    assert!(statements.contains(&Statement::Memberships {
        worker_oid: TEST_WORKER_OID
    }));
    assert!(!statements.contains(&Statement::Relations));
}

#[test]
fn relation_metadata_checks_every_identity_and_rls_flag() {
    let bad_rows = [
        (0, i64_cell(i64::from(TEST_SCHEMA_OID + 1))),
        (1, text("not_public")),
        (2, text("unexpected_relation")),
        (4, i64_cell(i64::from(TEST_APP_OID))),
        (5, text("app")),
        (6, text("p")),
        (7, text("u")),
        (8, bool_cell(true)),
        (9, bool_cell(false)),
        (10, bool_cell(false)),
    ];
    for (index, replacement) in bad_rows {
        let mut fake = FakeTransport::valid();
        replace_cell(&mut fake.relations[0], index, replacement);
        assert_error_primary(&mut fake, PrimaryFailure::PolicyRejected);
    }

    let mut policy_relation_reference_mismatch = FakeTransport::valid();
    replace_cell(
        &mut policy_relation_reference_mismatch.relations[0],
        3,
        i64_cell(52_199),
    );
    assert_error_primary(
        &mut policy_relation_reference_mismatch,
        PrimaryFailure::CatalogInvalid,
    );

    let mut missing = FakeTransport::valid();
    missing.relations.pop();
    assert_error_primary(&mut missing, PrimaryFailure::CatalogInvalid);

    let mut extra = FakeTransport::valid();
    extra.relations.push(relation_row(0, 52_102));
    assert_error_primary(&mut extra, PrimaryFailure::BoundExceeded);

    let mut duplicate_name = FakeTransport::valid();
    replace_cell(
        &mut duplicate_name.relations[1],
        2,
        text(RELATION_SPECS[0].name),
    );
    assert_error_primary(&mut duplicate_name, PrimaryFailure::PolicyRejected);

    let mut duplicate_oid = FakeTransport::valid();
    replace_cell(
        &mut duplicate_oid.relations[1],
        3,
        i64_cell(i64::from(TEST_RELATION_OIDS[0])),
    );
    assert_error_primary(&mut duplicate_oid, PrimaryFailure::PolicyRejected);
}

#[test]
fn either_direction_of_relation_inheritance_is_rejected() {
    for (child, parent) in [
        (TEST_RELATION_OIDS[0], 54_000),
        (54_000, TEST_RELATION_OIDS[0]),
        (TEST_RELATION_OIDS[1], 54_001),
        (54_001, TEST_RELATION_OIDS[1]),
    ] {
        let mut fake = FakeTransport::valid();
        fake.inheritance.push(inheritance_row(child, parent));
        assert_error_primary(&mut fake, PrimaryFailure::PolicyRejected);
    }

    let mut unrelated_edge = FakeTransport::valid();
    unrelated_edge
        .inheritance
        .push(inheritance_row(54_001, 54_002));
    assert_error_primary(&mut unrelated_edge, PrimaryFailure::CatalogInvalid);
}

#[test]
fn all_ten_policy_identities_flags_and_clause_null_shapes_are_exact() {
    for index in 0..POLICY_SPECS.len() {
        let mut missing = FakeTransport::valid();
        missing.policy_headers.remove(index);
        assert_error_primary(&mut missing, PrimaryFailure::CatalogInvalid);

        let mut wrong_relation = FakeTransport::valid();
        let other_slot = 1 - POLICY_SPECS[index].relation_slot;
        replace_cell(
            &mut wrong_relation.policy_headers[index],
            1,
            i64_cell(i64::from(TEST_RELATION_OIDS[other_slot])),
        );
        assert_error_primary(&mut wrong_relation, PrimaryFailure::PolicyRejected);

        let mut wrong_name = FakeTransport::valid();
        replace_cell(
            &mut wrong_name.policy_headers[index],
            2,
            text("unknown_policy"),
        );
        assert_error_primary(&mut wrong_name, PrimaryFailure::PolicyRejected);

        let mut wrong_command = FakeTransport::valid();
        replace_cell(&mut wrong_command.policy_headers[index], 3, text("x"));
        assert_error_primary(&mut wrong_command, PrimaryFailure::PolicyRejected);

        let mut not_permissive = FakeTransport::valid();
        replace_cell(
            &mut not_permissive.policy_headers[index],
            4,
            bool_cell(false),
        );
        assert_error_primary(&mut not_permissive, PrimaryFailure::PolicyRejected);

        let mut wrong_using_shape = FakeTransport::valid();
        replace_cell(
            &mut wrong_using_shape.policy_headers[index],
            9,
            bool_cell(!POLICY_SPECS[index].using_is_null),
        );
        assert_error_primary(&mut wrong_using_shape, PrimaryFailure::PolicyRejected);

        let mut wrong_check_shape = FakeTransport::valid();
        replace_cell(
            &mut wrong_check_shape.policy_headers[index],
            10,
            bool_cell(!POLICY_SPECS[index].with_check_is_null),
        );
        assert_error_primary(&mut wrong_check_shape, PrimaryFailure::PolicyRejected);
    }

    let mut extra = FakeTransport::valid();
    extra.policy_headers.push(policy_header_row(0));
    assert_error_primary(&mut extra, PrimaryFailure::BoundExceeded);

    let mut duplicate_oid = FakeTransport::valid();
    replace_cell(
        &mut duplicate_oid.policy_headers[1],
        0,
        i64_cell(i64::from(policy_oid_at(0))),
    );
    assert_error_primary(&mut duplicate_oid, PrimaryFailure::PolicyRejected);

    let mut duplicate_identity = FakeTransport::valid();
    duplicate_identity.policy_headers[9] = policy_header_row(0);
    assert_error_primary(&mut duplicate_identity, PrimaryFailure::PolicyRejected);
}

#[test]
fn malformed_policy_role_array_shapes_stop_before_scalar_expansion() {
    let malformed = [
        (5, bool_cell(true)),
        (6, RawCell::Null),
        (6, i32_cell(0)),
        (6, i32_cell(2)),
        (7, RawCell::Null),
        (7, i32_cell(0)),
        (8, RawCell::Null),
        (8, i32_cell(0)),
        (8, i32_cell(2)),
    ];
    for (index, value) in malformed {
        let mut fake = FakeTransport::valid();
        replace_cell(&mut fake.policy_headers[0], index, value);
        assert_error_primary(&mut fake, PrimaryFailure::PolicyRejected);
        assert!(!statement_seen(
            &fake.event_log(),
            StatementKind::PolicyRoles
        ));
    }
}

#[test]
fn policy_role_rows_link_the_exact_header_oid_relation_ordinal_and_role() {
    for (index, field, replacement, expected) in [
        (0, 0, i64_cell(0), PrimaryFailure::CatalogInvalid),
        (1, 0, i64_cell(54_000), PrimaryFailure::PolicyRejected),
        (
            2,
            1,
            i64_cell(i64::from(TEST_RELATION_OIDS[1])),
            PrimaryFailure::PolicyRejected,
        ),
        (3, 2, i64_cell(2), PrimaryFailure::PolicyRejected),
        (4, 3, i64_cell(0), PrimaryFailure::CatalogInvalid),
        (5, 3, i64_cell(-1), PrimaryFailure::CatalogInvalid),
        (
            6,
            3,
            i64_cell(i64::from(u32::MAX) + 1),
            PrimaryFailure::CatalogInvalid,
        ),
        (7, 3, i64_cell(59_999), PrimaryFailure::PolicyRejected),
    ] {
        let mut fake = FakeTransport::valid();
        replace_cell(&mut fake.policy_roles[index], field, replacement);
        assert_error_primary(&mut fake, expected);
    }

    let mut missing = FakeTransport::valid();
    missing.policy_roles.pop();
    assert_error_primary(&mut missing, PrimaryFailure::CatalogInvalid);

    let mut extra = FakeTransport::valid();
    extra.policy_roles.push(policy_role_row(0));
    assert_error_primary(&mut extra, PrimaryFailure::BoundExceeded);

    let mut duplicate_link = FakeTransport::valid();
    duplicate_link.policy_roles[9] = policy_role_row(0);
    assert_error_primary(&mut duplicate_link, PrimaryFailure::PolicyRejected);
}

#[test]
fn oid_conversion_rejects_public_zero_negative_and_overflow_but_accepts_maximum() {
    assert_eq!(positive_oid(1), Ok(1));
    assert_eq!(positive_oid(i64::from(u32::MAX)), Ok(u32::MAX));
    for invalid in [0, -1, i64::from(u32::MAX) + 1] {
        assert_eq!(
            positive_oid(invalid).map_err(|error| error.primary),
            Err(PrimaryFailure::CatalogInvalid)
        );
    }
}

#[test]
fn primitive_decoder_rejects_bad_type_category_count_null_pair_and_text_contract() {
    let mut budget = LogicalPayloadBudget::default();
    let mut wrong_type = RawDecoder::new(
        row(RawCategory::Context, vec![RawCell::WrongType]),
        RawCategory::Context,
        &[PgCellKind::Bool("x")],
        &mut budget,
    )
    .expect("shape count is valid");
    assert_eq!(
        wrong_type.boolean().map_err(|error| error.primary),
        Err(PrimaryFailure::CatalogInvalid)
    );

    let mut budget = LogicalPayloadBudget::default();
    assert!(matches!(
        RawDecoder::new(
            row(RawCategory::Role, vec![bool_cell(true)]),
            RawCategory::Context,
            &[PgCellKind::Bool("x")],
            &mut budget,
        )
        .map_err(|error| error.primary),
        Err(PrimaryFailure::CatalogInvalid)
    ));

    let mut budget = LogicalPayloadBudget::default();
    let mut wrong_count = RawDecoder::new(
        row(RawCategory::Context, vec![bool_cell(true)]),
        RawCategory::Context,
        &[PgCellKind::Bool("x"), PgCellKind::Bool("y")],
        &mut budget,
    );
    assert_eq!(
        wrong_count
            .as_mut()
            .map(|_| ())
            .map_err(|error| error.primary),
        Err(PrimaryFailure::CatalogInvalid)
    );

    for cell in [
        text_with_original("x", Some(1), true),
        text_with_original("", None, false),
        text_with_original("x", Some(2), false),
    ] {
        let mut budget = LogicalPayloadBudget::default();
        let mut decoder = RawDecoder::new(
            row(RawCategory::Context, vec![cell]),
            RawCategory::Context,
            &[PgCellKind::Text {
                null_alias: "n",
                bytes_alias: "b",
                value_alias: "v",
            }],
            &mut budget,
        )
        .expect("text row shape");
        assert_eq!(
            decoder.text().map_err(|error| error.primary),
            Err(PrimaryFailure::CatalogInvalid)
        );
    }
}

#[test]
fn text_byte_cap_accepts_exact_boundary_and_rejects_overflow_sentinel() {
    let shape = [PgCellKind::Text {
        null_alias: "n",
        bytes_alias: "b",
        value_alias: "v",
    }];
    let exact = "x".repeat(TEXT_ACCEPT_BYTES);
    let mut budget = LogicalPayloadBudget::default();
    let mut decoder = RawDecoder::new(
        row(
            RawCategory::Context,
            vec![text_with_original(
                &exact,
                Some(TEXT_ACCEPT_BYTES as i64),
                false,
            )],
        ),
        RawCategory::Context,
        &shape,
        &mut budget,
    )
    .expect("exact text boundary");
    assert_eq!(
        decoder.text().expect("accepted text").len(),
        TEXT_ACCEPT_BYTES
    );
    decoder.finish().expect("one cell consumed");

    let multibyte_exact = "é".repeat(512);
    let mut budget = LogicalPayloadBudget::default();
    let mut decoder = RawDecoder::new(
        row(
            RawCategory::Context,
            vec![text_with_original(&multibyte_exact, Some(1024), false)],
        ),
        RawCategory::Context,
        &shape,
        &mut budget,
    )
    .expect("multibyte exact byte boundary");
    assert_eq!(
        decoder.text().expect("multibyte accepted").len(),
        TEXT_ACCEPT_BYTES
    );

    for cell in [
        text_with_original(
            &"x".repeat(TEXT_FETCH_BYTES),
            Some(TEXT_FETCH_BYTES as i64),
            false,
        ),
        text_with_original("", Some((TEXT_FETCH_BYTES + 1) as i64), false),
        text_with_original(&("é".repeat(512) + "x"), Some(1025), false),
    ] {
        let mut budget = LogicalPayloadBudget::default();
        let mut decoder = RawDecoder::new(
            row(RawCategory::Context, vec![cell]),
            RawCategory::Context,
            &shape,
            &mut budget,
        )
        .expect("bounded text sentinel row");
        assert_eq!(
            decoder.text().map_err(|error| error.primary),
            Err(PrimaryFailure::BoundExceeded)
        );
    }
}

#[test]
fn final_context_identity_and_checked_policy_are_reread_before_commit() {
    let mut fake = FakeTransport::valid();
    replace_cell(
        &mut fake.final_context,
        0,
        text_with_original("admin", Some(5), false),
    );
    let error = assert_error_primary(&mut fake, PrimaryFailure::PolicyRejected);
    assert_eq!(error.rollback, CleanupState::Confirmed);
    assert_eq!(error.close, CleanupState::Confirmed);
    let events = fake.event_log();
    assert!(statement_seen(&events, StatementKind::FinalContext));
    assert!(!statement_seen(&events, StatementKind::Commit));

    let mut policy_changed = FakeTransport::valid();
    replace_cell(&mut policy_changed.final_context, 17, text("off"));
    let error = assert_error_primary(&mut policy_changed, PrimaryFailure::PolicyRejected);
    assert_eq!(error.rollback, CleanupState::Confirmed);
    assert_eq!(error.close, CleanupState::Confirmed);
    assert!(!statement_seen(
        &policy_changed.event_log(),
        StatementKind::Commit
    ));
}

#[test]
fn each_query_cap_accepts_only_its_fixed_rows_and_rejects_the_sentinel() {
    let mut context = FakeTransport::valid();
    context.context_overflow = true;
    assert_error_primary(&mut context, PrimaryFailure::BoundExceeded);

    let mut roles = FakeTransport::valid();
    roles.roles.push(role_rows()[0].clone());
    assert_error_primary(&mut roles, PrimaryFailure::BoundExceeded);

    let mut membership = FakeTransport::valid();
    membership.memberships.push(membership_row(TEST_WORKER_OID));
    assert_error_primary(&mut membership, PrimaryFailure::PolicyRejected);

    let mut relations = FakeTransport::valid();
    relations.relations.push(relation_row(0, 52_102));
    assert_error_primary(&mut relations, PrimaryFailure::BoundExceeded);

    let mut inheritance = FakeTransport::valid();
    inheritance
        .inheritance
        .push(inheritance_row(TEST_RELATION_OIDS[0], 54_000));
    assert_error_primary(&mut inheritance, PrimaryFailure::PolicyRejected);

    let mut policies = FakeTransport::valid();
    policies.policy_headers.push(policy_header_row(0));
    assert_error_primary(&mut policies, PrimaryFailure::BoundExceeded);

    let mut policy_roles = FakeTransport::valid();
    policy_roles.policy_roles.push(policy_role_row(0));
    assert_error_primary(&mut policy_roles, PrimaryFailure::BoundExceeded);
}

fn fixture_prefix_payload_bytes() -> usize {
    let mut budget = LogicalPayloadBudget::default();
    budget
        .charge_row(&context_row(), &CONTEXT_SHAPE)
        .expect("context fixture budget");
    for row in role_rows() {
        budget
            .charge_row(&row, &ROLE_SHAPE)
            .expect("role fixture budget");
    }
    for row in relation_rows() {
        budget
            .charge_row(&row, &RELATION_SHAPE)
            .expect("relation fixture budget");
    }
    for row in policy_header_rows() {
        budget
            .charge_row(&row, &POLICY_HEADER_SHAPE)
            .expect("policy-header fixture budget");
    }
    for row in policy_role_rows() {
        budget
            .charge_row(&row, &POLICY_ROLE_SHAPE)
            .expect("policy-role fixture budget");
    }
    budget.used
}

#[test]
fn logical_payload_budget_uses_checked_addition_and_charges_final_context() {
    let mut exact = LogicalPayloadBudget {
        used: LOGICAL_PAYLOAD_LIMIT - 2,
    };
    exact
        .charge(&RawCell::Bool(true), Some(PgCellKind::Bool("flag")))
        .expect("exact payload boundary");
    assert_eq!(exact.used, LOGICAL_PAYLOAD_LIMIT);

    let mut over = LogicalPayloadBudget {
        used: LOGICAL_PAYLOAD_LIMIT - 1,
    };
    assert_eq!(
        over.charge(&RawCell::Bool(true), Some(PgCellKind::Bool("flag")))
            .map_err(|error| error.primary),
        Err(PrimaryFailure::PayloadExceeded)
    );

    let mut checked_overflow = LogicalPayloadBudget { used: usize::MAX };
    assert_eq!(
        checked_overflow
            .charge(&RawCell::Bool(true), Some(PgCellKind::Bool("flag")))
            .map_err(|error| error.primary),
        Err(PrimaryFailure::PayloadExceeded)
    );

    let prior = fixture_prefix_payload_bytes();
    let mut fake = FakeTransport::valid();
    let mut budget = LogicalPayloadBudget {
        used: LOGICAL_PAYLOAD_LIMIT - prior - 1,
    };
    let config = valid_config();
    let canceled = Arc::clone(&fake.canceled);
    let result = block_on(observe_with_transport_and_budget(
        &mut fake,
        &config,
        &canceled,
        &mut budget,
    ));
    let error = match result {
        Ok(_) => panic!("final context must exceed the seeded cumulative budget"),
        Err(error) => error,
    };
    assert_eq!(error.primary, PrimaryFailure::PayloadExceeded);
    let events = fake.event_log();
    assert!(statement_seen(&events, StatementKind::FinalContext));
    assert!(statement_seen(&events, StatementKind::Rollback));
    assert!(!statement_seen(&events, StatementKind::Commit));
    assert_eq!(events.last(), Some(&Event::Close));
}

#[test]
fn timeout_is_checked_before_acquire_and_fixed_bind_helpers_are_exact() {
    assert_eq!(timeout_millis(1).map_err(|error| error.primary), Ok(1_000));
    assert_eq!(
        timeout_millis(123).map_err(|error| error.primary),
        Ok(123_000)
    );
    assert_eq!(
        timeout_millis(0).map_err(|error| error.primary),
        Err(PrimaryFailure::InvalidBound)
    );
    assert_eq!(
        timeout_millis(u64::MAX).map_err(|error| error.primary),
        Err(PrimaryFailure::InvalidBound)
    );
    assert_eq!(
        timeout_millis((i32::MAX as u64) / 1000 + 1).map_err(|error| error.primary),
        Err(PrimaryFailure::InvalidBound)
    );
    assert_eq!(timeout_bind_value(123_000), "123000");
    assert_eq!(worker_oid_bind_value(TEST_WORKER_OID), 16_384);
    assert_eq!(
        relation_oid_bind_values(TEST_RELATION_OIDS),
        [52_100, 52_101]
    );

    let mut fake = FakeTransport::valid();
    let config = config_with_timeout(3_000_000);
    let canceled = Arc::clone(&fake.canceled);
    let result = block_on(observe_with_transport(&mut fake, &config, &canceled));
    assert!(matches!(
        result.map_err(|error| error.primary),
        Err(PrimaryFailure::InvalidBound)
    ));
    assert!(!fake.event_log().contains(&Event::Acquire));
}

fn config_with_timeout(seconds: u64) -> SimulationProcessConfig {
    let json = VALID_CONFIG.replace(
        "\"startup_timeout_seconds\":123",
        &format!("\"startup_timeout_seconds\":{seconds}"),
    );
    let hash = Sha256::digest(json.as_bytes());
    let expected = format!("sha256:{hash:x}");
    SimulationProcessConfig::from_json(json.as_bytes(), TEST_COMMIT, &expected)
        .expect("synthetic timeout config")
}

#[test]
fn setup_acquire_query_commit_and_cleanup_failures_are_classified_without_raw_text() {
    let mut acquire = FakeTransport::valid();
    acquire.fail_at = Some(Point::Acquire);
    let error = assert_error_primary(&mut acquire, PrimaryFailure::AcquisitionFailed);
    assert_eq!(error.close, CleanupState::NotApplicable);
    assert!(!acquire.event_log().contains(&Event::Close));

    let mut begin = FakeTransport::valid();
    begin.fail_at = Some(Point::Statement(StatementKind::Begin));
    let error = assert_error_primary(&mut begin, PrimaryFailure::BeginFailed);
    assert_eq!(error.rollback, CleanupState::Confirmed);
    assert_eq!(error.close, CleanupState::Confirmed);

    let mut invalid_begin = FakeTransport::valid();
    invalid_begin.unexpected_at = Some(StatementKind::Begin);
    let error = assert_error_primary(&mut invalid_begin, PrimaryFailure::BeginResultInvalid);
    assert_eq!(error.rollback, CleanupState::Confirmed);

    let mut setup = FakeTransport::valid();
    setup.fail_at = Some(Point::Statement(StatementKind::Configure));
    let error = assert_error_primary(&mut setup, PrimaryFailure::SetupFailed);
    assert_eq!(error.rollback, CleanupState::Confirmed);

    let mut malformed_setup = FakeTransport::valid();
    malformed_setup.unexpected_at = Some(StatementKind::Configure);
    assert_error_primary(&mut malformed_setup, PrimaryFailure::SetupFailed);

    let failures = [
        (StatementKind::Context, PrimaryFailure::CatalogUnavailable),
        (StatementKind::Roles, PrimaryFailure::CatalogUnavailable),
        (
            StatementKind::Memberships,
            PrimaryFailure::CatalogUnavailable,
        ),
        (StatementKind::Relations, PrimaryFailure::CatalogUnavailable),
        (
            StatementKind::Inheritance,
            PrimaryFailure::CatalogUnavailable,
        ),
        (
            StatementKind::PolicyHeaders,
            PrimaryFailure::CatalogUnavailable,
        ),
        (
            StatementKind::PolicyRoles,
            PrimaryFailure::CatalogUnavailable,
        ),
        (
            StatementKind::FinalContext,
            PrimaryFailure::CatalogUnavailable,
        ),
    ];
    for (kind, expected) in failures {
        let mut fake = FakeTransport::valid();
        fake.fail_at = Some(Point::Statement(kind));
        let error = assert_error_primary(&mut fake, expected);
        assert_eq!(error.rollback, CleanupState::Confirmed);
        assert_eq!(error.close, CleanupState::Confirmed);
        assert!(!statement_seen(&fake.event_log(), StatementKind::Commit));
    }

    let mut commit = FakeTransport::valid();
    commit.fail_at = Some(Point::Statement(StatementKind::Commit));
    let error = assert_error_primary(&mut commit, PrimaryFailure::CompletionUnconfirmed);
    assert_eq!(error.completion, CompletionState::Unconfirmed);
    assert_eq!(error.rollback, CleanupState::NotApplicable);
    assert_eq!(error.close, CleanupState::Confirmed);
    assert!(!statement_seen(
        &commit.event_log(),
        StatementKind::Rollback
    ));

    let mut commit_close = FakeTransport::valid();
    commit_close.fail_at = Some(Point::Statement(StatementKind::Commit));
    commit_close.close_fails = true;
    let error = assert_error_primary(&mut commit_close, PrimaryFailure::CompletionUnconfirmed);
    assert_eq!(error.completion, CompletionState::Unconfirmed);
    assert_eq!(error.close, CleanupState::Unconfirmed);

    let mut acknowledged_close = FakeTransport::valid();
    acknowledged_close.close_fails = true;
    let error = assert_error_primary(&mut acknowledged_close, PrimaryFailure::CleanupUnconfirmed);
    assert_eq!(error.completion, CompletionState::Acknowledged);
    assert_eq!(error.close, CleanupState::Unconfirmed);
}

#[test]
fn same_await_database_failures_keep_primary_and_record_cancellation_for_every_query() {
    let flow = precommit_statement_flow();
    for (index, statement) in flow.iter().copied().enumerate().skip(1) {
        let kind = statement.kind();
        let mut fake = FakeTransport::valid();
        fake.fail_at = Some(Point::Statement(kind));
        fake.cancel_at = Some(Point::Statement(kind));
        let expected_primary = if kind == StatementKind::Configure {
            PrimaryFailure::SetupFailed
        } else {
            PrimaryFailure::CatalogUnavailable
        };
        assert_canceled_transport_failure(
            &mut fake,
            expected_primary,
            &flow[..=index],
            CleanupState::Confirmed,
            CleanupState::Confirmed,
        );
    }
}

#[test]
fn same_await_state_and_fetch_bound_failures_keep_primary_with_cancellation() {
    let flow = precommit_statement_flow();
    for (kind, transport_failure, expected_primary) in [
        (
            StatementKind::Relations,
            TransportFailure::State,
            PrimaryFailure::CatalogInvalid,
        ),
        (
            StatementKind::PolicyHeaders,
            TransportFailure::FetchBound,
            PrimaryFailure::BoundExceeded,
        ),
    ] {
        let index = flow
            .iter()
            .position(|statement| statement.kind() == kind)
            .expect("target query is in the exact precommit flow");
        let mut fake = FakeTransport::valid();
        fake.typed_failure_at = Some((kind, transport_failure));
        fake.cancel_at = Some(Point::Statement(kind));
        assert_canceled_transport_failure(
            &mut fake,
            expected_primary,
            &flow[..=index],
            CleanupState::Confirmed,
            CleanupState::Confirmed,
        );
    }
}

#[test]
fn same_await_primary_cancellation_and_failed_cleanup_states_stay_combined() {
    let flow = precommit_statement_flow();
    let index = flow
        .iter()
        .position(|statement| statement.kind() == StatementKind::Roles)
        .expect("roles query is in the exact precommit flow");
    let mut fake = FakeTransport::valid();
    fake.fail_at = Some(Point::Statement(StatementKind::Roles));
    fake.cancel_at = Some(Point::Statement(StatementKind::Roles));
    fake.rollback_fails = true;
    fake.close_fails = true;
    assert_canceled_transport_failure(
        &mut fake,
        PrimaryFailure::CatalogUnavailable,
        &flow[..=index],
        CleanupState::Unconfirmed,
        CleanupState::Unconfirmed,
    );
}

#[test]
fn primary_rollback_close_and_cancellation_uncertainty_are_preserved_together() {
    let mut all_cleanup_uncertain = FakeTransport::valid();
    all_cleanup_uncertain.fail_at = Some(Point::Statement(StatementKind::Roles));
    all_cleanup_uncertain.rollback_fails = true;
    all_cleanup_uncertain.close_fails = true;
    let error = assert_error_primary(
        &mut all_cleanup_uncertain,
        PrimaryFailure::CatalogUnavailable,
    );
    assert_eq!(error.rollback, CleanupState::Unconfirmed);
    assert_eq!(error.close, CleanupState::Unconfirmed);
    assert_eq!(error.completion, CompletionState::NotAttempted);

    let mut canceled_rollback = FakeTransport::valid();
    canceled_rollback.fail_at = Some(Point::Statement(StatementKind::Context));
    canceled_rollback.cancel_at = Some(Point::Statement(StatementKind::Rollback));
    let error = assert_error_primary(&mut canceled_rollback, PrimaryFailure::CatalogUnavailable);
    assert!(error.canceled);
    assert_eq!(error.rollback, CleanupState::Confirmed);
    assert_eq!(error.close, CleanupState::Confirmed);

    let mut canceled_commit = FakeTransport::valid();
    canceled_commit.cancel_at = Some(Point::Statement(StatementKind::Commit));
    canceled_commit.fail_at = Some(Point::Statement(StatementKind::Commit));
    let error = assert_error_primary(&mut canceled_commit, PrimaryFailure::CompletionUnconfirmed);
    assert!(error.canceled);
    assert_eq!(error.completion, CompletionState::Unconfirmed);
    assert_eq!(error.close, CleanupState::Confirmed);

    let mut canceled_commit_close = FakeTransport::valid();
    canceled_commit_close.cancel_at = Some(Point::Statement(StatementKind::Commit));
    canceled_commit_close.fail_at = Some(Point::Statement(StatementKind::Commit));
    canceled_commit_close.close_fails = true;
    let error = assert_error_primary(
        &mut canceled_commit_close,
        PrimaryFailure::CompletionUnconfirmed,
    );
    assert!(error.canceled);
    assert_eq!(error.completion, CompletionState::Unconfirmed);
    assert_eq!(error.close, CleanupState::Unconfirmed);
}

#[test]
fn cancellation_at_every_awaited_boundary_withholds_observation_and_closes() {
    let points = [
        Point::Acquire,
        Point::Arm,
        Point::Statement(StatementKind::Begin),
        Point::Statement(StatementKind::Configure),
        Point::Statement(StatementKind::Context),
        Point::Statement(StatementKind::Roles),
        Point::Statement(StatementKind::Memberships),
        Point::Statement(StatementKind::Relations),
        Point::Statement(StatementKind::Inheritance),
        Point::Statement(StatementKind::PolicyHeaders),
        Point::Statement(StatementKind::PolicyRoles),
        Point::Statement(StatementKind::FinalContext),
        Point::Statement(StatementKind::Commit),
        Point::Close,
    ];
    for point in points {
        let mut fake = FakeTransport::valid();
        fake.cancel_at = Some(point);
        let error = assert_error_primary(&mut fake, PrimaryFailure::Canceled);
        assert!(error.canceled);
        assert!(fake.event_log().contains(&Event::Close));
    }

    let mut canceled_before_acquire = FakeTransport::valid();
    canceled_before_acquire
        .canceled
        .store(true, Ordering::Release);
    assert_error_primary(&mut canceled_before_acquire, PrimaryFailure::Canceled);
    assert!(
        !canceled_before_acquire
            .event_log()
            .contains(&Event::Acquire)
    );
    assert!(!canceled_before_acquire.event_log().contains(&Event::Close));
}

#[test]
fn pending_future_drop_is_only_claimed_after_close_on_drop_was_armed() {
    let mut fake = FakeTransport::valid();
    fake.pending_at = Some(StatementKind::PolicyHeaders);
    let config = valid_config();
    let canceled = Arc::clone(&fake.canceled);
    let mut future = Box::pin(observe_with_transport(&mut fake, &config, &canceled));
    let mut context = Context::from_waker(Waker::noop());
    assert!(matches!(future.as_mut().poll(&mut context), Poll::Pending));
    drop(future);
    let events = fake.event_log();
    assert!(events.contains(&Event::PendingDroppedAfterArm(true)));
    assert!(!events.contains(&Event::Close));
}
#[test]
fn command_result_discards_row_payload_and_rejects_presence() {
    assert!(super::classify_command_result(None::<String>).is_empty());
    let rows = super::classify_command_result(Some(String::from("sentinel")));
    assert_eq!(rows.len(), 1);
    assert!(rows[0].cells.is_empty());
}
