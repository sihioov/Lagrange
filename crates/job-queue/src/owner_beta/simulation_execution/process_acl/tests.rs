use std::future::pending;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use tokio::sync::oneshot;

use super::super::SimulationProcessConfig;
use super::{
    ACL_ROW_CAP, ACLS_SQL, AclRow, BEGIN_SQL, COLUMN_ROW_CAP, COLUMNS_SQL, CONTEXT_SQL, CatalogRow,
    ColumnRow, ContextRow, MEMBERSHIP_SQL, MembershipRow, ObserverTransport, PAIR_COLUMNS,
    RELATIONS_SQL, RUN_COLUMNS, SETUP_SQL, SimulationWorkerAclError as Error,
    SimulationWorkerAclObservation, Statement, TransportFailure, WORKER_ROLE,
    observe_with_transport, timeout_millis,
};

const TEST_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

#[test]
fn command_result_discards_row_payload_and_rejects_presence() {
    assert!(super::classify_command_result(None::<String>).is_empty());
    assert!(matches!(
        super::classify_command_result(Some(String::from("sentinel"))).as_slice(),
        [CatalogRow::Command]
    ));
}
const VALID_CONFIG: &str = r###"{"schema":"owner-beta-simulation-process-v1","code_commit":"0123456789abcdef0123456789abcdef01234567","manifest_format":"LAGRANGE_RELEASE_MANIFEST_V3","manifest_sha256":"sha256:1111111111111111111111111111111111111111111111111111111111111111","image_id":"sha256:2222222222222222222222222222222222222222222222222222222222222222","receipt_sha256":"sha256:3333333333333333333333333333333333333333333333333333333333333333","binary_sha256":"sha256:4444444444444444444444444444444444444444444444444444444444444444","source_input_sha256":"sha256:5555555555555555555555555555555555555555555555555555555555555555","recipe_sha256":"sha256:6666666666666666666666666666666666666666666666666666666666666666","build_origin":"official-release-v1","package":"job-queue","bin":"owner-beta-simulation-runner","target":"x86_64-unknown-linux-musl","profile":"release","toolchain":"1.97.1","owner":"11111111-2222-3333-4444-555555555555","entitlement_reference":"fixture:synthetic-owner-rights-v1","entitlement_sha256":"sha256:7777777777777777777777777777777777777777777777777777777777777777","historical_root":"/synthetic/history/δ","reader_uid":10001,"reader_gid":10001,"paths":{"config":"/run/owner-beta-simulation/config.json","artifact_root":"/data/owner-beta-artifacts","receipt":"/usr/share/lagrange/owner-beta-simulation/artifact.json","complete":"/usr/share/lagrange/owner-beta-simulation/COMPLETE","executable":"/usr/local/bin/owner-beta-simulation-runner","worker_id":"owner-beta-simulation-runner"},"limits":{"memory_bytes":1234567,"nano_cpus":2345678,"health_tmpfs_bytes":3456789,"pids_limit":42,"startup_timeout_seconds":123,"health_max_age_seconds":234,"stop_timeout_seconds":345}}"###;

const WORKER_OID: u32 = 16_384;
const DATABASE_OID: u32 = 16_385;
const SCHEMA_OID: u32 = 2_200;
const OWNER_OID: u32 = 16_386;
const SCHEMA_OWNER_OID: u32 = 16_387;
const RUNS_OID: u32 = 45_000;
const PAIRS_OID: u32 = 45_001;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Point {
    Acquire,
    Arm,
    Begin,
    Setup,
    Context,
    Membership,
    Relations,
    Columns,
    Acls,
    Commit,
    Rollback,
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Event {
    Acquire,
    Armed,
    Statement(Statement),
    Close,
    Dropped,
}

struct FakeTransport {
    events: Arc<Mutex<Vec<Event>>>,
    canceled: Arc<AtomicBool>,
    context_rows: Vec<CatalogRow>,
    membership_rows: Vec<CatalogRow>,
    relation_rows: Vec<CatalogRow>,
    column_rows: Vec<CatalogRow>,
    acl_rows: Vec<CatalogRow>,
    failures: Vec<(Point, TransportFailure)>,
    cancel_at: Option<Point>,
    pending_at: Option<Point>,
    pending_reached: Option<oneshot::Sender<()>>,
    malformed_nonquery: Option<Point>,
}

impl FakeTransport {
    fn valid() -> Self {
        let mut acl_rows = vec![
            acl(RUNS_OID, None, WORKER_OID, OWNER_OID, "SELECT", false),
            acl(PAIRS_OID, None, WORKER_OID, OWNER_OID, "SELECT", false),
        ];
        for (index, column) in RUN_COLUMNS.iter().enumerate() {
            if super::runs_update_column(column.name) {
                acl_rows.push(acl(
                    RUNS_OID,
                    Some((index + 1) as i32),
                    WORKER_OID,
                    OWNER_OID,
                    "UPDATE",
                    false,
                ));
            }
        }
        for (index, column) in PAIR_COLUMNS.iter().enumerate() {
            if super::pair_insert_column(column.name) {
                acl_rows.push(acl(
                    PAIRS_OID,
                    Some((index + 1) as i32),
                    WORKER_OID,
                    OWNER_OID,
                    "INSERT",
                    false,
                ));
            }
        }

        Self {
            events: Arc::new(Mutex::new(Vec::new())),
            canceled: Arc::new(AtomicBool::new(false)),
            context_rows: vec![CatalogRow::Context(ContextRow {
                current_user: WORKER_ROLE.to_owned(),
                session_user: WORKER_ROLE.to_owned(),
                observed_database_name: "lagrange".to_owned(),
                database_name: "lagrange".to_owned(),
                schema_name: "public".to_owned(),
                worker_oid: i64::from(WORKER_OID),
                database_oid: i64::from(DATABASE_OID),
                schema_oid: i64::from(SCHEMA_OID),
                database_owner_oid: i64::from(OWNER_OID),
                schema_owner_oid: i64::from(SCHEMA_OWNER_OID),
                can_login: true,
                is_superuser: false,
                can_create_database: false,
                can_create_role: false,
                can_replicate: false,
                bypasses_rls: false,
                row_security: "on".to_owned(),
                session_replication_role: "origin".to_owned(),
                transaction_read_only: true,
                transaction_isolation: "repeatable read".to_owned(),
                database_connect: true,
                database_create: false,
                schema_usage: true,
                schema_create: false,
                database_effective_grant_option: false,
                schema_effective_grant_option: false,
                database_grant_option: false,
                schema_grant_option: false,
            })],
            membership_rows: Vec::new(),
            relation_rows: vec![
                relation("owner_beta_simulation_runs", RUNS_OID),
                relation("owner_beta_simulation_result_pairs", PAIRS_OID),
            ],
            column_rows: columns(),
            acl_rows,
            failures: Vec::new(),
            cancel_at: None,
            pending_at: None,
            pending_reached: None,
            malformed_nonquery: None,
        }
    }

    fn statement_point(statement: Statement) -> Point {
        match statement {
            Statement::Begin => Point::Begin,
            Statement::Configure { .. } => Point::Setup,
            Statement::Context => Point::Context,
            Statement::Membership { .. } => Point::Membership,
            Statement::Relations { .. } => Point::Relations,
            Statement::Columns { .. } => Point::Columns,
            Statement::Acls { .. } => Point::Acls,
            Statement::Commit => Point::Commit,
            Statement::Rollback => Point::Rollback,
        }
    }

    fn record(&self, event: Event) {
        self.events.lock().expect("fake event mutex").push(event);
    }

    fn event_log(&self) -> Vec<Event> {
        self.events.lock().expect("fake event mutex").clone()
    }

    fn fail(&mut self, point: Point, failure: TransportFailure) {
        self.failures.push((point, failure));
    }

    fn set_canceled(&self) {
        self.canceled.store(true, Ordering::Release);
    }
}

impl Drop for FakeTransport {
    fn drop(&mut self) {
        self.record(Event::Dropped);
    }
}

#[async_trait]
impl ObserverTransport for FakeTransport {
    async fn acquire(&mut self) -> Result<(), TransportFailure> {
        self.record(Event::Acquire);
        if let Some((_, failure)) = self
            .failures
            .iter()
            .find(|(point, _)| *point == Point::Acquire)
        {
            return Err(*failure);
        }
        if self.cancel_at == Some(Point::Acquire) {
            self.set_canceled();
        }
        Ok(())
    }

    fn arm_close_on_drop(&mut self) {
        self.record(Event::Armed);
        if self.cancel_at == Some(Point::Arm) {
            self.set_canceled();
        }
    }

    async fn execute(&mut self, statement: Statement) -> Result<Vec<CatalogRow>, TransportFailure> {
        let point = Self::statement_point(statement);
        self.record(Event::Statement(statement));
        if self.pending_at == Some(point) {
            if let Some(entered) = self.pending_reached.take() {
                let _ = entered.send(());
            }
            return pending().await;
        }
        if let Some((_, failure)) = self
            .failures
            .iter()
            .find(|(failed_point, _)| *failed_point == point)
        {
            return Err(*failure);
        }
        if self.cancel_at == Some(point) {
            self.set_canceled();
        }
        if matches!(
            statement,
            Statement::Begin | Statement::Commit | Statement::Rollback
        ) {
            return Ok(super::classify_command_result(
                (self.malformed_nonquery == Some(point)).then_some(()),
            ));
        }
        if self.malformed_nonquery == Some(point) {
            return Ok(vec![self.acl_rows[0].clone()]);
        }
        Ok(match statement {
            Statement::Context => self.context_rows.clone(),
            Statement::Membership { .. } => self.membership_rows.clone(),
            Statement::Relations { .. } => self.relation_rows.clone(),
            Statement::Columns { .. } => self.column_rows.clone(),
            Statement::Acls { .. } => self.acl_rows.clone(),
            Statement::Begin
            | Statement::Configure { .. }
            | Statement::Commit
            | Statement::Rollback => Vec::new(),
        })
    }

    async fn close(&mut self) -> Result<(), TransportFailure> {
        self.record(Event::Close);
        if self.cancel_at == Some(Point::Close) {
            self.set_canceled();
        }
        if let Some((_, failure)) = self
            .failures
            .iter()
            .find(|(point, _)| *point == Point::Close)
        {
            return Err(*failure);
        }
        Ok(())
    }
}

async fn observe(
    fake: &mut FakeTransport,
    config: &SimulationProcessConfig,
) -> Result<SimulationWorkerAclObservation, Error> {
    let canceled = Arc::clone(&fake.canceled);
    observe_with_transport(fake, config, &canceled).await
}

fn checked_config() -> SimulationProcessConfig {
    checked_config_with_timeout(123)
}

fn checked_config_with_timeout(seconds: u64) -> SimulationProcessConfig {
    let bytes = VALID_CONFIG.replace(
        "\"startup_timeout_seconds\":123",
        &format!("\"startup_timeout_seconds\":{seconds}"),
    );
    let digest = format!("sha256:{:x}", Sha256::digest(bytes.as_bytes()));
    SimulationProcessConfig::from_json(bytes.as_bytes(), TEST_COMMIT, &digest)
        .expect("the synthetic process config is checked from its exact bytes")
}

fn relation(name: &str, oid: u32) -> CatalogRow {
    CatalogRow::Relation(super::RelationRow {
        schema_oid: i64::from(SCHEMA_OID),
        schema_name: "public".to_owned(),
        relation_name: name.to_owned(),
        relation_oid: i64::from(oid),
        owner_oid: i64::from(OWNER_OID),
        owner_name: Some("migration_owner".to_owned()),
        relation_kind: "r".to_owned(),
        persistence: "p".to_owned(),
        is_partition: false,
        has_inheritance_edge: false,
        table_select: true,
        table_insert: false,
        table_update: false,
        table_delete: false,
        table_truncate: false,
        table_references: false,
        table_trigger: false,
        table_effective_grant_option: false,
    })
}

fn columns() -> Vec<CatalogRow> {
    let mut rows = Vec::with_capacity(RUN_COLUMNS.len() + PAIR_COLUMNS.len());
    for (index, spec) in RUN_COLUMNS.iter().enumerate() {
        rows.push(column(
            RUNS_OID,
            "owner_beta_simulation_runs",
            (index + 1) as i32,
            spec,
            false,
        ));
    }
    for (index, spec) in PAIR_COLUMNS.iter().enumerate() {
        rows.push(column(
            PAIRS_OID,
            "owner_beta_simulation_result_pairs",
            (index + 1) as i32,
            spec,
            true,
        ));
    }
    rows
}

fn column(
    relation_oid: u32,
    relation_name: &str,
    attnum: i32,
    spec: &super::ColumnSpec,
    pairs: bool,
) -> CatalogRow {
    CatalogRow::Column(ColumnRow {
        relation_oid: i64::from(relation_oid),
        relation_name: relation_name.to_owned(),
        attnum,
        column_name: spec.name.to_owned(),
        type_namespace: Some("pg_catalog".to_owned()),
        type_name: Some(spec.type_name.to_owned()),
        not_null: spec.not_null,
        is_dropped: false,
        generated: String::new(),
        identity: String::new(),
        column_select: true,
        column_insert: pairs && super::pair_insert_column(spec.name),
        column_update: !pairs && super::runs_update_column(spec.name),
        column_references: false,
        column_effective_grant_option: false,
    })
}

fn acl(
    relation_oid: u32,
    attnum: Option<i32>,
    grantee_oid: u32,
    grantor_oid: u32,
    privilege_type: &str,
    is_grantable: bool,
) -> CatalogRow {
    CatalogRow::Acl(AclRow {
        relation_oid: i64::from(relation_oid),
        attnum,
        grantee_oid: i64::from(grantee_oid),
        grantor_oid: i64::from(grantor_oid),
        privilege_type: privilege_type.to_owned(),
        is_grantable,
    })
}

fn context_mut(fake: &mut FakeTransport) -> &mut ContextRow {
    match &mut fake.context_rows[0] {
        CatalogRow::Context(row) => row,
        _ => panic!("context fixture variant"),
    }
}

fn relation_mut(fake: &mut FakeTransport, relation_oid: u32) -> &mut super::RelationRow {
    fake.relation_rows
        .iter_mut()
        .find_map(|row| match row {
            CatalogRow::Relation(row) if row.relation_oid == i64::from(relation_oid) => Some(row),
            _ => None,
        })
        .expect("relation fixture")
}

fn column_mut<'a>(fake: &'a mut FakeTransport, relation_oid: u32, name: &str) -> &'a mut ColumnRow {
    fake.column_rows
        .iter_mut()
        .find_map(|row| match row {
            CatalogRow::Column(row)
                if row.relation_oid == i64::from(relation_oid) && row.column_name == name =>
            {
                Some(row)
            }
            _ => None,
        })
        .expect("column fixture")
}

fn acl_row_mut<'a>(
    fake: &'a mut FakeTransport,
    relation_oid: u32,
    attnum: Option<i32>,
    privilege: &str,
) -> &'a mut AclRow {
    fake.acl_rows
        .iter_mut()
        .find_map(|row| match row {
            CatalogRow::Acl(row)
                if row.relation_oid == i64::from(relation_oid)
                    && row.attnum == attnum
                    && row.privilege_type == privilege =>
            {
                Some(row)
            }
            _ => None,
        })
        .expect("ACL fixture")
}

fn assert_raw_acl_fixture_allowed(fake: &FakeTransport) {
    assert_eq!(
        super::compare_acls(
            fake.acl_rows.clone(),
            super::RelationEvidence {
                runs_oid: RUNS_OID,
                pairs_oid: PAIRS_OID,
            },
            WORKER_OID,
        ),
        Ok(())
    );
}

fn assert_error(result: Result<SimulationWorkerAclObservation, Error>, expected: Error) {
    match result {
        Err(actual) => assert_eq!(actual, expected),
        Ok(_) => panic!("unexpected observation evidence"),
    }
}

fn count(events: &[Event], expected: Event) -> usize {
    events.iter().filter(|event| **event == expected).count()
}

fn assert_precommit_cleanup(events: &[Event]) {
    assert_eq!(count(events, Event::Close), 1);
    let statements: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            Event::Statement(statement) => Some(*statement),
            _ => None,
        })
        .collect();
    assert_eq!(count_statement(&statements, Statement::Rollback), 1);
    let rollback = statements
        .iter()
        .position(|statement| *statement == Statement::Rollback)
        .expect("rollback event");
    assert!(rollback + 1 == statements.len());
}

fn count_statement(statements: &[Statement], expected: Statement) -> usize {
    statements
        .iter()
        .filter(|statement| **statement == expected)
        .count()
}

fn assert_grant_option_literals(sql: &str, expected: &[&str]) {
    let actual: Vec<_> = sql
        .split('\'')
        .enumerate()
        .filter_map(|(index, literal)| {
            if index % 2 == 1 {
                literal.strip_suffix(" WITH GRANT OPTION")
            } else {
                None
            }
        })
        .collect();
    assert_eq!(actual.as_slice(), expected);
}

#[tokio::test]
async fn positive_column_only_acl_observation_commits_closes_and_binds_checked_config() {
    let config = checked_config();
    let mut fake = FakeTransport::valid();
    assert!(!relation_mut(&mut fake, RUNS_OID).table_insert);
    assert!(!relation_mut(&mut fake, RUNS_OID).table_update);
    assert!(!relation_mut(&mut fake, PAIRS_OID).table_insert);
    assert!(!relation_mut(&mut fake, PAIRS_OID).table_update);

    let observed = match observe(&mut fake, &config).await {
        Ok(observed) => observed,
        Err(_) => panic!("valid synthetic ACL fixture was rejected"),
    };
    assert_eq!(observed.config_sha256(), config.config_sha256());
    assert_eq!(observed.database_oid(), DATABASE_OID);
    assert_eq!(observed.worker_oid(), WORKER_OID);
    assert_eq!(observed.public_schema_oid(), SCHEMA_OID);
    assert_eq!(observed.runs_oid(), RUNS_OID);
    assert_eq!(observed.pairs_oid(), PAIRS_OID);

    let events = fake.event_log();
    let statements: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            Event::Statement(statement) => Some(*statement),
            _ => None,
        })
        .collect();
    assert_eq!(
        statements,
        vec![
            Statement::Begin,
            Statement::Configure {
                timeout_ms: 123_000
            },
            Statement::Context,
            Statement::Membership {
                worker_oid: WORKER_OID
            },
            Statement::Relations {
                schema_oid: SCHEMA_OID
            },
            Statement::Columns {
                runs_oid: RUNS_OID,
                pairs_oid: PAIRS_OID
            },
            Statement::Acls {
                runs_oid: RUNS_OID,
                pairs_oid: PAIRS_OID,
                worker_oid: WORKER_OID
            },
            Statement::Commit,
        ]
    );
    assert_eq!(count(&events, Event::Armed), 1);
    assert_eq!(count(&events, Event::Close), 1);
    let arm = events
        .iter()
        .position(|event| *event == Event::Armed)
        .expect("close-on-drop armed");
    let first_sql = events
        .iter()
        .position(|event| matches!(event, Event::Statement(_)))
        .expect("first SQL event");
    let commit = events
        .iter()
        .position(|event| *event == Event::Statement(Statement::Commit))
        .expect("commit event");
    let close = events
        .iter()
        .position(|event| *event == Event::Close)
        .expect("close event");
    assert!(arm < first_sql);
    assert!(commit < close);
}

#[test]
fn timeout_milliseconds_are_checked_and_static_sql_is_bounded_and_catalog_only() {
    assert_eq!(timeout_millis(123), Ok(123_000));
    assert_eq!(timeout_millis(0), Err(Error::InvalidBound));
    assert_eq!(timeout_millis(u64::MAX), Err(Error::InvalidBound));
    assert_eq!(timeout_millis(2_147_484), Err(Error::InvalidBound));

    assert!(BEGIN_SQL.contains("ISOLATION LEVEL REPEATABLE READ READ ONLY"));
    assert!(SETUP_SQL.contains("pg_catalog.set_config('search_path'"));
    assert!(SETUP_SQL.contains("statement_timeout"));
    assert!(SETUP_SQL.contains("lock_timeout"));
    assert!(CONTEXT_SQL.contains("LIMIT 2"));
    assert!(MEMBERSHIP_SQL.contains("LIMIT 2"));
    assert!(RELATIONS_SQL.contains("LIMIT 3"));
    assert!(COLUMNS_SQL.contains("LIMIT 129"));
    assert!(ACLS_SQL.contains("LIMIT 257"));
    assert!(COLUMNS_SQL.contains("a.attnum > 0"));
    assert!(!COLUMNS_SQL.contains("AND NOT a.attisdropped"));
    assert!(ACLS_SQL.contains("pg_catalog.acldefault('r'"));
    assert!(ACLS_SQL.contains("pg_catalog.aclexplode"));
    assert!(RELATIONS_SQL.contains("pg_catalog.pg_class"));
    assert!(COLUMNS_SQL.contains("pg_catalog.pg_attribute"));
    assert!(!COLUMNS_SQL.contains("public.owner_beta_simulation_runs"));
    assert_grant_option_literals(
        CONTEXT_SQL,
        &["CONNECT", "CREATE", "TEMPORARY", "USAGE", "CREATE"],
    );
    assert_grant_option_literals(
        RELATIONS_SQL,
        &[
            "SELECT",
            "INSERT",
            "UPDATE",
            "DELETE",
            "TRUNCATE",
            "REFERENCES",
            "TRIGGER",
        ],
    );
    assert_grant_option_literals(COLUMNS_SQL, &["SELECT", "INSERT", "UPDATE", "REFERENCES"]);
    assert!(COLUMNS_SQL.contains(
        "CASE WHEN NOT a.attisdropped\n            THEN (\n                pg_catalog.has_column_privilege("
    ));
}

#[tokio::test]
async fn invalid_millisecond_bound_fails_before_acquiring_a_connection() {
    let config = checked_config_with_timeout(3_000_000);
    let mut fake = FakeTransport::valid();
    assert_error(observe(&mut fake, &config).await, Error::InvalidBound);
    let events = fake.event_log();
    assert_eq!(count(&events, Event::Acquire), 0);
    assert_eq!(count(&events, Event::Armed), 0);
    assert_eq!(count(&events, Event::Close), 0);
}

#[tokio::test]
async fn acquisition_and_begin_failures_close_only_when_a_connection_exists() {
    let config = checked_config();
    let mut acquire = FakeTransport::valid();
    acquire.fail(Point::Acquire, TransportFailure::Database);
    assert_error(
        observe(&mut acquire, &config).await,
        Error::AcquisitionFailed,
    );
    let events = acquire.event_log();
    assert_eq!(count(&events, Event::Acquire), 1);
    assert_eq!(count(&events, Event::Armed), 0);
    assert_eq!(count(&events, Event::Close), 0);
    assert!(
        events
            .iter()
            .all(|event| !matches!(event, Event::Statement(_)))
    );

    let mut begin = FakeTransport::valid();
    begin.fail(Point::Begin, TransportFailure::Database);
    assert_error(observe(&mut begin, &config).await, Error::BeginFailed);
    let events = begin.event_log();
    assert_eq!(count(&events, Event::Close), 1);
    assert!(!events.contains(&Event::Statement(Statement::Rollback)));

    let mut bad_begin_shape = FakeTransport::valid();
    bad_begin_shape.malformed_nonquery = Some(Point::Begin);
    assert_error(
        observe(&mut bad_begin_shape, &config).await,
        Error::CatalogInvalid,
    );
    let events = bad_begin_shape.event_log();
    assert_precommit_cleanup(&events);
}

#[tokio::test]
async fn setup_and_each_catalog_failure_roll_back_then_close_without_later_queries() {
    let config = checked_config();
    for point in [
        Point::Setup,
        Point::Context,
        Point::Membership,
        Point::Relations,
        Point::Columns,
        Point::Acls,
    ] {
        let mut fake = FakeTransport::valid();
        fake.fail(point, TransportFailure::Database);
        let expected = match point {
            Point::Setup => Error::SetupFailed,
            _ => Error::CatalogUnavailable,
        };
        assert_error(observe(&mut fake, &config).await, expected);
        let events = fake.event_log();
        assert_precommit_cleanup(&events);
        let statements: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                Event::Statement(statement) => Some(*statement),
                _ => None,
            })
            .collect();
        assert_eq!(statements.last(), Some(&Statement::Rollback));
        assert!(!statements.contains(&Statement::Commit));
    }

    let mut decode = FakeTransport::valid();
    decode.fail(Point::Context, TransportFailure::Decode);
    assert_error(observe(&mut decode, &config).await, Error::CatalogInvalid);
    assert_precommit_cleanup(&decode.event_log());
}

#[tokio::test]
async fn precommit_rollback_or_close_failure_is_unconfirmed_and_both_are_attempted() {
    let config = checked_config();
    let mut rollback_failure = FakeTransport::valid();
    rollback_failure.fail(Point::Context, TransportFailure::Database);
    rollback_failure.fail(Point::Rollback, TransportFailure::Database);
    assert_error(
        observe(&mut rollback_failure, &config).await,
        Error::CleanupUnconfirmed,
    );
    let events = rollback_failure.event_log();
    assert_eq!(count(&events, Event::Close), 1);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, Event::Statement(Statement::Rollback)))
            .count(),
        1
    );

    let mut close_failure = FakeTransport::valid();
    close_failure.fail(Point::Context, TransportFailure::Database);
    close_failure.fail(Point::Close, TransportFailure::Database);
    assert_error(
        observe(&mut close_failure, &config).await,
        Error::CleanupUnconfirmed,
    );
    assert_precommit_cleanup(&close_failure.event_log());

    let mut begin_close_failure = FakeTransport::valid();
    begin_close_failure.fail(Point::Begin, TransportFailure::Database);
    begin_close_failure.fail(Point::Close, TransportFailure::Database);
    assert_error(
        observe(&mut begin_close_failure, &config).await,
        Error::CleanupUnconfirmed,
    );
    assert_eq!(count(&begin_close_failure.event_log(), Event::Close), 1);
}

#[tokio::test]
async fn commit_and_close_uncertainty_never_roll_back_or_return_evidence() {
    let config = checked_config();

    let mut commit_failure = FakeTransport::valid();
    commit_failure.fail(Point::Commit, TransportFailure::Database);
    assert_error(
        observe(&mut commit_failure, &config).await,
        Error::CompletionUnconfirmed,
    );
    let events = commit_failure.event_log();
    assert_eq!(count(&events, Event::Close), 1);
    assert!(!events.contains(&Event::Statement(Statement::Rollback)));

    let mut malformed_commit = FakeTransport::valid();
    malformed_commit.malformed_nonquery = Some(Point::Commit);
    assert_error(
        observe(&mut malformed_commit, &config).await,
        Error::CompletionUnconfirmed,
    );
    assert_eq!(count(&malformed_commit.event_log(), Event::Close), 1);
    assert!(
        !malformed_commit
            .event_log()
            .contains(&Event::Statement(Statement::Rollback))
    );

    let mut both_unknown = FakeTransport::valid();
    both_unknown.fail(Point::Commit, TransportFailure::Database);
    both_unknown.fail(Point::Close, TransportFailure::Database);
    assert_error(
        observe(&mut both_unknown, &config).await,
        Error::CompletionAndCleanupUnconfirmed,
    );
    assert_eq!(count(&both_unknown.event_log(), Event::Close), 1);
    assert!(
        !both_unknown
            .event_log()
            .contains(&Event::Statement(Statement::Rollback))
    );

    let mut close_failure = FakeTransport::valid();
    close_failure.fail(Point::Close, TransportFailure::Database);
    assert_error(
        observe(&mut close_failure, &config).await,
        Error::CleanupUnconfirmed,
    );
    assert!(
        close_failure
            .event_log()
            .contains(&Event::Statement(Statement::Commit))
    );
    assert!(
        !close_failure
            .event_log()
            .contains(&Event::Statement(Statement::Rollback))
    );
}

#[tokio::test]
async fn context_identity_role_flags_and_session_settings_fail_closed() {
    let config = checked_config();
    let mut wrong_session = FakeTransport::valid();
    context_mut(&mut wrong_session).session_user = "app".to_owned();
    assert_error(
        observe(&mut wrong_session, &config).await,
        Error::AccessRejected,
    );
    assert_precommit_cleanup(&wrong_session.event_log());

    let mut wrong_current_user = FakeTransport::valid();
    context_mut(&mut wrong_current_user).current_user = "app".to_owned();
    assert_error(
        observe(&mut wrong_current_user, &config).await,
        Error::AccessRejected,
    );

    for flag in 0..6 {
        let mut fake = FakeTransport::valid();
        let context = context_mut(&mut fake);
        match flag {
            0 => context.is_superuser = true,
            1 => context.can_create_database = true,
            2 => context.can_create_role = true,
            3 => context.can_replicate = true,
            4 => context.bypasses_rls = true,
            _ => context.can_login = false,
        }
        assert_error(observe(&mut fake, &config).await, Error::AccessRejected);
        assert_precommit_cleanup(&fake.event_log());
    }

    for mutation in 0..9 {
        let mut fake = FakeTransport::valid();
        let context = context_mut(&mut fake);
        match mutation {
            0 => context.row_security = "off".to_owned(),
            1 => context.session_replication_role = "replica".to_owned(),
            2 => context.transaction_read_only = false,
            3 => context.transaction_isolation = "read committed".to_owned(),
            4 => context.database_connect = false,
            5 => context.database_create = true,
            6 => context.schema_usage = false,
            7 => context.schema_create = true,
            _ => context.database_grant_option = true,
        }
        assert_error(observe(&mut fake, &config).await, Error::AccessRejected);
        assert_precommit_cleanup(&fake.event_log());
    }

    let mut schema_grant_option = FakeTransport::valid();
    context_mut(&mut schema_grant_option).schema_grant_option = true;
    assert_error(
        observe(&mut schema_grant_option, &config).await,
        Error::AccessRejected,
    );
}

#[tokio::test]
async fn context_requires_positive_bounded_oids_and_valid_names_and_distinct_owners() {
    let config = checked_config();
    let mut zero_oid = FakeTransport::valid();
    context_mut(&mut zero_oid).database_oid = 0;
    assert_error(observe(&mut zero_oid, &config).await, Error::CatalogInvalid);

    let mut overflow_oid = FakeTransport::valid();
    context_mut(&mut overflow_oid).worker_oid = i64::from(u32::MAX) + 1;
    assert_error(
        observe(&mut overflow_oid, &config).await,
        Error::CatalogInvalid,
    );

    let mut empty_database = FakeTransport::valid();
    context_mut(&mut empty_database).database_name.clear();
    assert_error(
        observe(&mut empty_database, &config).await,
        Error::CatalogInvalid,
    );

    let mut long_database = FakeTransport::valid();
    context_mut(&mut long_database).database_name = "x".repeat(64);
    assert_error(
        observe(&mut long_database, &config).await,
        Error::CatalogInvalid,
    );

    let mut control_database = FakeTransport::valid();
    context_mut(&mut control_database).database_name = "bad\nname".to_owned();
    assert_error(
        observe(&mut control_database, &config).await,
        Error::CatalogInvalid,
    );

    let mut worker_owns_database = FakeTransport::valid();
    context_mut(&mut worker_owns_database).database_owner_oid = i64::from(WORKER_OID);
    assert_error(
        observe(&mut worker_owns_database, &config).await,
        Error::AccessRejected,
    );

    let mut worker_owns_schema = FakeTransport::valid();
    context_mut(&mut worker_owns_schema).schema_owner_oid = i64::from(WORKER_OID);
    assert_error(
        observe(&mut worker_owns_schema, &config).await,
        Error::AccessRejected,
    );
}

#[tokio::test]
async fn any_outgoing_worker_membership_is_rejected_including_noinherit_set_role_edges() {
    let config = checked_config();
    for (admin, inherit, set) in [(false, false, true), (false, true, true)] {
        let mut fake = FakeTransport::valid();
        fake.membership_rows
            .push(CatalogRow::Membership(MembershipRow {
                member_oid: i64::from(WORKER_OID),
                role_oid: i64::from(OWNER_OID),
                grantor_oid: i64::from(OWNER_OID),
                admin_option: admin,
                inherit_option: inherit,
                set_option: set,
            }));
        assert_error(observe(&mut fake, &config).await, Error::AccessRejected);
        let events = fake.event_log();
        assert_precommit_cleanup(&events);
        assert!(!events.contains(&Event::Statement(Statement::Relations {
            schema_oid: SCHEMA_OID
        })));
    }

    let mut ordinary_edge = FakeTransport::valid();
    ordinary_edge
        .membership_rows
        .push(CatalogRow::Membership(MembershipRow {
            member_oid: i64::from(WORKER_OID),
            role_oid: i64::from(45_123),
            grantor_oid: i64::from(OWNER_OID),
            admin_option: false,
            inherit_option: true,
            set_option: false,
        }));
    assert_error(
        observe(&mut ordinary_edge, &config).await,
        Error::AccessRejected,
    );
}

#[tokio::test]
async fn relation_identity_owner_partition_and_table_privileges_are_exact() {
    let config = checked_config();
    for mutation in 0..15 {
        let mut fake = FakeTransport::valid();
        let row = relation_mut(&mut fake, RUNS_OID);
        match mutation {
            0 => row.schema_name = "other".to_owned(),
            1 => row.schema_oid = 88,
            2 => {
                row.owner_name = Some("worker".to_owned());
                row.owner_oid = i64::from(WORKER_OID);
            }
            3 => row.owner_name = None,
            4 => row.relation_kind = "v".to_owned(),
            5 => row.persistence = "t".to_owned(),
            6 => row.is_partition = true,
            7 => row.has_inheritance_edge = true,
            8 => row.table_select = false,
            9 => row.table_insert = true,
            10 => row.table_update = true,
            11 => row.table_delete = true,
            12 => row.table_truncate = true,
            13 => row.table_references = true,
            14 => row.table_trigger = true,
            _ => unreachable!("the table privilege cases are explicitly enumerated"),
        }
        assert!(observe(&mut fake, &config).await.is_err());
        assert_precommit_cleanup(&fake.event_log());
    }

    let mut missing = FakeTransport::valid();
    missing.relation_rows.pop();
    assert_error(observe(&mut missing, &config).await, Error::CatalogInvalid);

    let mut duplicate_name = FakeTransport::valid();
    duplicate_name.relation_rows[1] = relation("owner_beta_simulation_runs", PAIRS_OID);
    assert_error(
        observe(&mut duplicate_name, &config).await,
        Error::CatalogInvalid,
    );

    let mut duplicate_oid = FakeTransport::valid();
    duplicate_oid.relation_rows[1] = relation("owner_beta_simulation_result_pairs", RUNS_OID);
    assert_error(
        observe(&mut duplicate_oid, &config).await,
        Error::CatalogInvalid,
    );

    let mut inconsistent_migration_owner = FakeTransport::valid();
    relation_mut(&mut inconsistent_migration_owner, PAIRS_OID).owner_oid = 99_999;
    assert_error(
        observe(&mut inconsistent_migration_owner, &config).await,
        Error::CatalogInvalid,
    );

    let mut missing_result = FakeTransport::valid();
    missing_result.relation_rows.clear();
    assert_error(
        observe(&mut missing_result, &config).await,
        Error::CatalogInvalid,
    );
}

#[tokio::test]
async fn exact_thirty_nine_column_shape_rejects_missing_extra_dropped_and_malformed_rows() {
    let config = checked_config();

    let mut missing = FakeTransport::valid();
    missing.column_rows.pop();
    assert_error(observe(&mut missing, &config).await, Error::CatalogInvalid);

    let mut extra = FakeTransport::valid();
    extra.column_rows.push(extra.column_rows[0].clone());
    assert_error(observe(&mut extra, &config).await, Error::CatalogInvalid);

    let mut duplicate = FakeTransport::valid();
    duplicate.column_rows[1] = duplicate.column_rows[0].clone();
    assert_error(
        observe(&mut duplicate, &config).await,
        Error::CatalogInvalid,
    );

    for mutation in 0..7 {
        let mut fake = FakeTransport::valid();
        let row = column_mut(&mut fake, RUNS_OID, "id");
        match mutation {
            0 => row.attnum = 3,
            1 => row.type_namespace = Some("other".to_owned()),
            2 => row.type_name = Some("text".to_owned()),
            3 => row.not_null = false,
            4 => row.is_dropped = true,
            5 => row.generated = "s".to_owned(),
            _ => row.identity = "a".to_owned(),
        }
        assert_error(observe(&mut fake, &config).await, Error::CatalogInvalid);
        assert_precommit_cleanup(&fake.event_log());
    }

    let mut unknown_type = FakeTransport::valid();
    column_mut(&mut unknown_type, RUNS_OID, "id").type_name = None;
    assert_error(
        observe(&mut unknown_type, &config).await,
        Error::CatalogInvalid,
    );

    let mut long_column = FakeTransport::valid();
    column_mut(&mut long_column, RUNS_OID, "id").column_name = "x".repeat(64);
    assert_error(
        observe(&mut long_column, &config).await,
        Error::CatalogInvalid,
    );
}

#[tokio::test]
async fn column_effective_write_grants_match_only_the_migration_matrix() {
    let config = checked_config();
    for (relation_oid, name, insert, update, references) in [
        (RUNS_OID, "created_at", false, true, false),
        (RUNS_OID, "id", true, false, false),
        (RUNS_OID, "id", false, false, true),
        (PAIRS_OID, "created_at", true, false, false),
        (PAIRS_OID, "created_at", false, true, false),
        (PAIRS_OID, "run_id", false, false, true),
    ] {
        let mut fake = FakeTransport::valid();
        let row = column_mut(&mut fake, relation_oid, name);
        row.column_insert = insert;
        row.column_update = update;
        row.column_references = references;
        assert_error(observe(&mut fake, &config).await, Error::AccessRejected);
    }

    for (relation_oid, name) in [(RUNS_OID, "status"), (PAIRS_OID, "run_id")] {
        let mut fake = FakeTransport::valid();
        let row = column_mut(&mut fake, relation_oid, name);
        if relation_oid == RUNS_OID {
            row.column_update = false;
        } else {
            row.column_insert = false;
        }
        assert_error(observe(&mut fake, &config).await, Error::AccessRejected);
    }
}

#[tokio::test]
async fn raw_acl_rejects_public_grant_options_unknown_and_unexpected_writes() {
    let config = checked_config();

    let mut public = FakeTransport::valid();
    public
        .acl_rows
        .push(acl(RUNS_OID, None, 0, OWNER_OID, "SELECT", false));
    assert_error(observe(&mut public, &config).await, Error::AccessRejected);

    let mut table_grant_option = FakeTransport::valid();
    acl_row_mut(&mut table_grant_option, RUNS_OID, None, "SELECT").is_grantable = true;
    assert_error(
        observe(&mut table_grant_option, &config).await,
        Error::AccessRejected,
    );

    let mut column_grant_option = FakeTransport::valid();
    acl_row_mut(&mut column_grant_option, RUNS_OID, Some(25), "UPDATE").is_grantable = true;
    assert_error(
        observe(&mut column_grant_option, &config).await,
        Error::AccessRejected,
    );

    for (relation_oid, attnum, privilege) in [
        (RUNS_OID, None, "INSERT"),
        (RUNS_OID, None, "UPDATE"),
        (RUNS_OID, Some(2), "UPDATE"),
        (RUNS_OID, Some(8), "UPDATE"),
        (RUNS_OID, Some(29), "UPDATE"),
        (PAIRS_OID, Some(7), "INSERT"),
    ] {
        let mut fake = FakeTransport::valid();
        fake.acl_rows.push(acl(
            relation_oid,
            attnum,
            WORKER_OID,
            OWNER_OID,
            privilege,
            false,
        ));
        assert_error(observe(&mut fake, &config).await, Error::AccessRejected);
    }

    let mut unknown = FakeTransport::valid();
    unknown.acl_rows.push(acl(
        RUNS_OID, None, WORKER_OID, OWNER_OID, "MAINTAIN", false,
    ));
    assert_error(observe(&mut unknown, &config).await, Error::AccessRejected);

    let mut unknown_public = FakeTransport::valid();
    unknown_public
        .acl_rows
        .push(acl(RUNS_OID, None, 0, OWNER_OID, "MAINTAIN", false));
    assert_error(
        observe(&mut unknown_public, &config).await,
        Error::AccessRejected,
    );

    let mut no_grantor = FakeTransport::valid();
    acl_row_mut(&mut no_grantor, RUNS_OID, None, "SELECT").grantor_oid = 0;
    assert_error(
        observe(&mut no_grantor, &config).await,
        Error::CatalogInvalid,
    );

    let mut unknown_grantee = FakeTransport::valid();
    acl_row_mut(&mut unknown_grantee, RUNS_OID, None, "SELECT").grantee_oid = 45_678;
    assert_error(
        observe(&mut unknown_grantee, &config).await,
        Error::CatalogInvalid,
    );
}

#[tokio::test]
async fn effective_grant_options_fail_shared_flow_when_raw_acls_are_allowed() {
    let config = checked_config();

    let mut database = FakeTransport::valid();
    let context = context_mut(&mut database);
    context.database_effective_grant_option = true;
    assert!(!context.database_grant_option);
    assert_raw_acl_fixture_allowed(&database);
    assert_error(observe(&mut database, &config).await, Error::AccessRejected);

    let mut schema = FakeTransport::valid();
    let context = context_mut(&mut schema);
    context.schema_effective_grant_option = true;
    assert!(!context.schema_grant_option);
    assert_raw_acl_fixture_allowed(&schema);
    assert_error(observe(&mut schema, &config).await, Error::AccessRejected);

    for relation_oid in [RUNS_OID, PAIRS_OID] {
        let mut table = FakeTransport::valid();
        relation_mut(&mut table, relation_oid).table_effective_grant_option = true;
        assert_raw_acl_fixture_allowed(&table);
        assert_error(observe(&mut table, &config).await, Error::AccessRejected);
    }

    let mut column = FakeTransport::valid();
    column_mut(&mut column, RUNS_OID, "id").column_effective_grant_option = true;
    assert_raw_acl_fixture_allowed(&column);
    assert_error(observe(&mut column, &config).await, Error::AccessRejected);
}

#[tokio::test]
async fn direct_raw_worker_grants_are_required_even_when_effective_checks_are_true() {
    let config = checked_config();
    let mut missing_run_update = FakeTransport::valid();
    missing_run_update.acl_rows.retain(|row| {
        !matches!(
            row,
            CatalogRow::Acl(AclRow {
                relation_oid,
                attnum: Some(25),
                privilege_type,
                ..
            }) if *relation_oid == i64::from(RUNS_OID) && privilege_type == "UPDATE"
        )
    });
    assert!(column_mut(&mut missing_run_update, RUNS_OID, "status").column_update);
    assert_error(
        observe(&mut missing_run_update, &config).await,
        Error::AccessRejected,
    );

    let mut missing_pair_insert = FakeTransport::valid();
    missing_pair_insert.acl_rows.retain(|row| {
        !matches!(
            row,
            CatalogRow::Acl(AclRow {
                relation_oid,
                attnum: Some(1),
                privilege_type,
                ..
            }) if *relation_oid == i64::from(PAIRS_OID) && privilege_type == "INSERT"
        )
    });
    assert!(column_mut(&mut missing_pair_insert, PAIRS_OID, "run_id").column_insert);
    assert_error(
        observe(&mut missing_pair_insert, &config).await,
        Error::AccessRejected,
    );

    let mut missing_table_select = FakeTransport::valid();
    missing_table_select.acl_rows.retain(|row| {
        !matches!(
            row,
            CatalogRow::Acl(AclRow {
                relation_oid,
                attnum: None,
                privilege_type,
                ..
            }) if *relation_oid == i64::from(RUNS_OID) && privilege_type == "SELECT"
        )
    });
    assert!(relation_mut(&mut missing_table_select, RUNS_OID).table_select);
    assert_error(
        observe(&mut missing_table_select, &config).await,
        Error::AccessRejected,
    );

    let mut redundant_column_select = FakeTransport::valid();
    redundant_column_select.acl_rows.push(acl(
        RUNS_OID,
        Some(1),
        WORKER_OID,
        OWNER_OID,
        "SELECT",
        false,
    ));
    assert!(observe(&mut redundant_column_select, &config).await.is_ok());
}

#[tokio::test]
async fn malformed_duplicate_unknown_and_overlimit_raw_acl_rows_fail_closed() {
    let config = checked_config();
    let mut duplicate = FakeTransport::valid();
    duplicate.acl_rows.push(duplicate.acl_rows[0].clone());
    assert_error(
        observe(&mut duplicate, &config).await,
        Error::CatalogInvalid,
    );

    let mut bad_relation = FakeTransport::valid();
    acl_row_mut(&mut bad_relation, RUNS_OID, None, "SELECT").relation_oid = 0;
    assert_error(
        observe(&mut bad_relation, &config).await,
        Error::CatalogInvalid,
    );

    let mut bad_attnum = FakeTransport::valid();
    bad_attnum.acl_rows.push(acl(
        RUNS_OID,
        Some(99),
        WORKER_OID,
        OWNER_OID,
        "SELECT",
        false,
    ));
    assert_error(
        observe(&mut bad_attnum, &config).await,
        Error::CatalogInvalid,
    );

    let mut long_privilege = FakeTransport::valid();
    acl_row_mut(&mut long_privilege, RUNS_OID, None, "SELECT").privilege_type = "X".repeat(33);
    assert_error(
        observe(&mut long_privilege, &config).await,
        Error::CatalogInvalid,
    );

    let mut over_cap = FakeTransport::valid();
    over_cap
        .acl_rows
        .resize(ACL_ROW_CAP + 1, over_cap.acl_rows[0].clone());
    assert_error(observe(&mut over_cap, &config).await, Error::CatalogInvalid);
}

#[tokio::test]
async fn every_catalog_collection_rejects_cap_plus_one_and_missing_context_rows() {
    let config = checked_config();

    let mut context_cap = FakeTransport::valid();
    context_cap
        .context_rows
        .push(context_cap.context_rows[0].clone());
    assert_error(
        observe(&mut context_cap, &config).await,
        Error::CatalogInvalid,
    );

    let mut missing_context = FakeTransport::valid();
    missing_context.context_rows.clear();
    assert_error(
        observe(&mut missing_context, &config).await,
        Error::CatalogUnavailable,
    );

    let mut membership_cap = FakeTransport::valid();
    membership_cap.membership_rows = vec![
        CatalogRow::Membership(MembershipRow {
            member_oid: i64::from(WORKER_OID),
            role_oid: 45_100,
            grantor_oid: i64::from(OWNER_OID),
            admin_option: false,
            inherit_option: false,
            set_option: true,
        }),
        CatalogRow::Membership(MembershipRow {
            member_oid: i64::from(WORKER_OID),
            role_oid: 45_101,
            grantor_oid: i64::from(OWNER_OID),
            admin_option: false,
            inherit_option: true,
            set_option: false,
        }),
    ];
    assert_error(
        observe(&mut membership_cap, &config).await,
        Error::CatalogInvalid,
    );

    let mut relation_cap = FakeTransport::valid();
    relation_cap
        .relation_rows
        .push(relation("owner_beta_simulation_runs", 45_002));
    assert_error(
        observe(&mut relation_cap, &config).await,
        Error::CatalogInvalid,
    );

    let mut column_cap = FakeTransport::valid();
    column_cap
        .column_rows
        .resize(COLUMN_ROW_CAP + 1, column_cap.column_rows[0].clone());
    assert_error(
        observe(&mut column_cap, &config).await,
        Error::CatalogInvalid,
    );
}

#[tokio::test]
async fn cancellation_before_acquisition_and_after_acquisition_never_issues_sql() {
    let config = checked_config();
    let mut before = FakeTransport::valid();
    before.canceled.store(true, Ordering::Release);
    assert_error(observe(&mut before, &config).await, Error::Canceled);
    assert_eq!(count(&before.event_log(), Event::Acquire), 0);

    for point in [Point::Acquire, Point::Arm] {
        let mut after = FakeTransport::valid();
        after.cancel_at = Some(point);
        assert_error(observe(&mut after, &config).await, Error::Canceled);
        let events = after.event_log();
        assert_eq!(count(&events, Event::Close), 1);
        assert!(
            events
                .iter()
                .all(|event| !matches!(event, Event::Statement(_)))
        );
        assert_eq!(count(&events, Event::Armed), 1);
    }
}

#[tokio::test]
async fn cancellation_during_setup_each_query_and_before_commit_rolls_back_and_closes() {
    let config = checked_config();
    for point in [
        Point::Setup,
        Point::Context,
        Point::Membership,
        Point::Relations,
        Point::Columns,
        Point::Acls,
    ] {
        let mut fake = FakeTransport::valid();
        fake.cancel_at = Some(point);
        assert_error(observe(&mut fake, &config).await, Error::Canceled);
        let events = fake.event_log();
        assert_precommit_cleanup(&events);
        let statements: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                Event::Statement(statement) => Some(*statement),
                _ => None,
            })
            .collect();
        assert!(!statements.contains(&Statement::Commit));
    }
}

#[tokio::test]
async fn cancellation_during_commit_or_close_still_closes_and_withholds_evidence() {
    let config = checked_config();
    for point in [Point::Commit, Point::Close] {
        let mut fake = FakeTransport::valid();
        fake.cancel_at = Some(point);
        assert_error(observe(&mut fake, &config).await, Error::Canceled);
        let events = fake.event_log();
        assert_eq!(count(&events, Event::Close), 1);
        assert!(events.contains(&Event::Statement(Statement::Commit)));
        assert!(!events.contains(&Event::Statement(Statement::Rollback)));
    }
}

#[tokio::test]
async fn dropping_a_pending_query_future_keeps_no_evidence_and_never_reuses_connection() {
    let config = checked_config();
    let mut fake = FakeTransport::valid();
    let events = Arc::clone(&fake.events);
    let canceled = Arc::clone(&fake.canceled);
    let (entered, pending_reached) = oneshot::channel();
    fake.pending_at = Some(Point::Context);
    fake.pending_reached = Some(entered);

    let task =
        tokio::spawn(async move { observe_with_transport(&mut fake, &config, &canceled).await });
    pending_reached
        .await
        .expect("fake reached the pending catalog query");
    task.abort();
    assert!(task.await.is_err());

    let events = events.lock().expect("fake event mutex").clone();
    assert_eq!(count(&events, Event::Armed), 1);
    assert!(events.contains(&Event::Statement(Statement::Begin)));
    assert!(events.contains(&Event::Statement(Statement::Context)));
    assert!(!events.contains(&Event::Close));
    assert!(events.contains(&Event::Dropped));
    // Drop models ownership and armed close-on-drop intent only. It is not an
    // awaited PostgreSQL rollback or close receipt.
}

#[tokio::test]
async fn successful_query_sequence_has_no_application_or_role_switch_statements() {
    let config = checked_config();
    let mut fake = FakeTransport::valid();
    assert!(observe(&mut fake, &config).await.is_ok());
    let statements: Vec<_> = fake
        .event_log()
        .iter()
        .filter_map(|event| match event {
            Event::Statement(statement) => Some(*statement),
            _ => None,
        })
        .collect();
    assert!(
        !statements
            .iter()
            .any(|statement| matches!(statement, Statement::Rollback))
    );
    assert_eq!(
        count_statement(
            &statements,
            Statement::Membership {
                worker_oid: WORKER_OID
            }
        ),
        1
    );
    let catalog_sql = [
        CONTEXT_SQL,
        MEMBERSHIP_SQL,
        RELATIONS_SQL,
        COLUMNS_SQL,
        ACLS_SQL,
    ]
    .join("\n");
    for forbidden in [
        "SET ROLE",
        "FOR UPDATE",
        "FOR SHARE",
        "INSERT INTO",
        "UPDATE public.",
        "DELETE FROM",
    ] {
        assert!(!catalog_sql.contains(forbidden));
    }
}
