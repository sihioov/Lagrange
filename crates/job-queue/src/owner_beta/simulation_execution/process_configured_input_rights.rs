//! Private configured-input and exact-license observation source candidate.
//!
//! This module observes one checked configuration and one already sealed V3
//! artifact. It does not read files, establish legal permission, or authorize
//! publication or execution by another process.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use chrono::NaiveDate;
use domain::TradingDate;
use market_data::{
    ApprovedHistoricalPriceOnlyArtifact, HistoricalPriceOnlyBar,
    HistoricalPriceOnlyV3ArtifactApprovalSummary,
};
use sqlx::{
    Column, PgConnection, PgPool, Postgres, Row, TypeInfo, ValueRef, pool::PoolConnection,
    postgres::PgRow,
};
use uuid::Uuid;

use super::SimulationProcessConfig;

const POLICY_ID: &str = "simulation-configured-input-rights-source-v1";
const POLICY_FINGERPRINT: &str =
    "sha256:fcc9c749352a77b307dc69f1f08ae8c365b6906504b5964e930f0c0892e3498f";

const EXPECTED_INSTRUMENTS: usize = 11;
const EXPECTED_SESSIONS: usize = 2_452;
const EXPECTED_BARS: usize = 26_972;
const EXPECTED_START: &str = "2016-08-29";
const EXPECTED_END: &str = "2026-08-28";

const SETUP_ACCEPT_ROWS: usize = 1;
const SETUP_FETCH_ROWS: usize = SETUP_ACCEPT_ROWS + 1;
const CONTEXT_ACCEPT_ROWS: usize = 1;
const CONTEXT_FETCH_ROWS: usize = CONTEXT_ACCEPT_ROWS + 1;
const OWNER_ACCEPT_ROWS: usize = 1;
const OWNER_FETCH_ROWS: usize = OWNER_ACCEPT_ROWS + 1;
const ENTITLEMENT_ACCEPT_ROWS: usize = 64;
const ENTITLEMENT_FETCH_ROWS: usize = ENTITLEMENT_ACCEPT_ROWS + 1;

const SHORT_TEXT_ACCEPT_BYTES: usize = 128;
const SHORT_TEXT_FETCH_BYTES: usize = SHORT_TEXT_ACCEPT_BYTES + 1;
const REFERENCE_ACCEPT_BYTES: usize = 4_096;
const REFERENCE_FETCH_BYTES: usize = REFERENCE_ACCEPT_BYTES + 1;
const LICENSE_HASH_ACCEPT_BYTES: usize = 64;
const LICENSE_HASH_FETCH_BYTES: usize = LICENSE_HASH_ACCEPT_BYTES + 1;
const STATUS_ACCEPT_BYTES: usize = 7;
const STATUS_FETCH_BYTES: usize = STATUS_ACCEPT_BYTES + 1;
const JSON_ACCEPT_BYTES: usize = 4_096;
const JSON_FETCH_BYTES: usize = JSON_ACCEPT_BYTES + 1;
const MAX_IDENTIFIER_COUNT: usize = 32;
const MAX_IDENTIFIER_BYTES: usize = 128;
const MAX_LOGICAL_PAYLOAD_BYTES: usize = 1_048_576;
const RAW_TRIPLET_OVERHEAD_BYTES: usize = 9;
const DATE_INPUT_LOGICAL_BYTES: usize = EXPECTED_SESSIONS * 4;
const LICENSE_MAX_FETCHED_PAYLOAD: usize = 808_210;
const ALL_MAX_FETCHED_PAYLOAD: usize = 810_900;
const CANCEL_CHECK_INTERVAL: usize = 128;
const MAX_STATEMENT_TIMEOUT_MS: i64 = 30_000;

const BEGIN_SQL: &str = "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY";
const CONFIGURE_SQL: &str = r#"WITH s AS MATERIALIZED (
 SELECT pg_catalog.set_config('search_path', 'pg_catalog', true)::pg_catalog.text AS search_path,
        pg_catalog.set_config('statement_timeout', $1::pg_catalog.int8::pg_catalog.text || 'ms', true)::pg_catalog.text AS statement_timeout,
        pg_catalog.set_config('lock_timeout', $1::pg_catalog.int8::pg_catalog.text || 'ms', true)::pg_catalog.text AS lock_timeout
)
SELECT (s.search_path IS NULL)::pg_catalog.bool AS search_path_is_null,
       pg_catalog.octet_length(pg_catalog.convert_to(s.search_path::pg_catalog.text, 'UTF8'))::pg_catalog.int8 AS search_path_bytes,
       pg_catalog.substr(pg_catalog.convert_to(COALESCE(s.search_path::pg_catalog.text, ''), 'UTF8'), 1, 129)::pg_catalog.bytea AS search_path_prefix,
       (s.statement_timeout IS NULL)::pg_catalog.bool AS statement_timeout_is_null,
       pg_catalog.octet_length(pg_catalog.convert_to(s.statement_timeout::pg_catalog.text, 'UTF8'))::pg_catalog.int8 AS statement_timeout_bytes,
       pg_catalog.substr(pg_catalog.convert_to(COALESCE(s.statement_timeout::pg_catalog.text, ''), 'UTF8'), 1, 129)::pg_catalog.bytea AS statement_timeout_prefix,
       (s.lock_timeout IS NULL)::pg_catalog.bool AS lock_timeout_is_null,
       pg_catalog.octet_length(pg_catalog.convert_to(s.lock_timeout::pg_catalog.text, 'UTF8'))::pg_catalog.int8 AS lock_timeout_bytes,
       pg_catalog.substr(pg_catalog.convert_to(COALESCE(s.lock_timeout::pg_catalog.text, ''), 'UTF8'), 1, 129)::pg_catalog.bytea AS lock_timeout_prefix
FROM s LIMIT 2"#;

const CONTEXT_SQL: &str = r#"WITH c AS MATERIALIZED (
 SELECT current_user::pg_catalog.text AS current_user,
        session_user::pg_catalog.text AS session_user,
        pg_catalog.current_database()::pg_catalog.text AS database_name
)
SELECT (c.current_user IS NULL)::pg_catalog.bool AS current_user_is_null,
       pg_catalog.octet_length(pg_catalog.convert_to(c.current_user::pg_catalog.text, 'UTF8'))::pg_catalog.int8 AS current_user_bytes,
       pg_catalog.substr(pg_catalog.convert_to(COALESCE(c.current_user::pg_catalog.text, ''), 'UTF8'), 1, 129)::pg_catalog.bytea AS current_user_prefix,
       (c.session_user IS NULL)::pg_catalog.bool AS session_user_is_null,
       pg_catalog.octet_length(pg_catalog.convert_to(c.session_user::pg_catalog.text, 'UTF8'))::pg_catalog.int8 AS session_user_bytes,
       pg_catalog.substr(pg_catalog.convert_to(COALESCE(c.session_user::pg_catalog.text, ''), 'UTF8'), 1, 129)::pg_catalog.bytea AS session_user_prefix,
       (c.database_name IS NULL)::pg_catalog.bool AS database_name_is_null,
       pg_catalog.octet_length(pg_catalog.convert_to(c.database_name::pg_catalog.text, 'UTF8'))::pg_catalog.int8 AS database_name_bytes,
       pg_catalog.substr(pg_catalog.convert_to(COALESCE(c.database_name::pg_catalog.text, ''), 'UTF8'), 1, 129)::pg_catalog.bytea AS database_name_prefix,
       d.oid::pg_catalog.int8 AS database_oid,
       r.oid::pg_catalog.int8 AS worker_oid,
       pg_catalog.pg_backend_pid()::pg_catalog.int4 AS backend_pid,
       (pg_catalog.current_setting('row_security') = 'on')::pg_catalog.bool AS row_security_on,
       (pg_catalog.current_setting('session_replication_role') = 'origin')::pg_catalog.bool AS replication_origin,
       pg_catalog.current_setting('transaction_read_only')::pg_catalog.bool AS transaction_read_only,
       (pg_catalog.current_setting('transaction_isolation') = 'repeatable read')::pg_catalog.bool AS repeatable_read,
       (pg_catalog.current_setting('search_path') = 'pg_catalog')::pg_catalog.bool AS search_path_is_catalog,
       st.setting::pg_catalog.int8 AS statement_timeout_ms,
       lt.setting::pg_catalog.int8 AS lock_timeout_ms,
       (st.unit = 'ms')::pg_catalog.bool AS statement_unit_ms,
       (lt.unit = 'ms')::pg_catalog.bool AS lock_unit_ms
FROM c
JOIN pg_catalog.pg_database d ON d.datname::pg_catalog.text = c.database_name
JOIN pg_catalog.pg_roles r ON r.rolname = 'worker'
JOIN pg_catalog.pg_settings st ON st.name = 'statement_timeout'
JOIN pg_catalog.pg_settings lt ON lt.name = 'lock_timeout'
LIMIT 2"#;

const OWNER_SQL: &str = r#"SELECT u.id::pg_catalog.uuid AS owner_id,
       EXISTS (
         SELECT 1 FROM public.user_roles ur
         WHERE ur.user_id = u.id AND ur.role_id COLLATE pg_catalog."C" = 'owner' COLLATE pg_catalog."C"
       )::pg_catalog.bool AS owner_role_observed
FROM public.users u
WHERE u.id = $1::pg_catalog.uuid
ORDER BY u.id
LIMIT 2"#;

const ENTITLEMENTS_SQL: &str = r#"WITH e AS MATERIALIZED (
 SELECT id, contract_reference, contract_document_sha256, status,
        covered_datasets, covered_uses, effective_from, effective_until
 FROM public.data_entitlements
 WHERE contract_reference COLLATE pg_catalog."C" = $1::pg_catalog.text COLLATE pg_catalog."C"
   AND contract_document_sha256 COLLATE pg_catalog."C" = $2::pg_catalog.text COLLATE pg_catalog."C"
 ORDER BY id
 LIMIT 65
)
SELECT e.id::pg_catalog.uuid AS entitlement_id,
       (e.contract_reference IS NULL)::pg_catalog.bool AS reference_is_null,
       pg_catalog.octet_length(pg_catalog.convert_to(e.contract_reference::pg_catalog.text, 'UTF8'))::pg_catalog.int8 AS reference_bytes,
       pg_catalog.substr(pg_catalog.convert_to(COALESCE(e.contract_reference::pg_catalog.text, ''), 'UTF8'), 1, 4097)::pg_catalog.bytea AS reference_prefix,
       (e.contract_document_sha256 IS NULL)::pg_catalog.bool AS license_hash_is_null,
       pg_catalog.octet_length(pg_catalog.convert_to(e.contract_document_sha256::pg_catalog.text, 'UTF8'))::pg_catalog.int8 AS license_hash_bytes,
       pg_catalog.substr(pg_catalog.convert_to(COALESCE(e.contract_document_sha256::pg_catalog.text, ''), 'UTF8'), 1, 65)::pg_catalog.bytea AS license_hash_prefix,
       (e.status IS NULL)::pg_catalog.bool AS status_is_null,
       pg_catalog.octet_length(pg_catalog.convert_to(e.status::pg_catalog.text, 'UTF8'))::pg_catalog.int8 AS status_bytes,
       pg_catalog.substr(pg_catalog.convert_to(COALESCE(e.status::pg_catalog.text, ''), 'UTF8'), 1, 8)::pg_catalog.bytea AS status_prefix,
       (e.covered_datasets IS NULL)::pg_catalog.bool AS datasets_is_null,
       pg_catalog.octet_length(pg_catalog.convert_to(e.covered_datasets::pg_catalog.text, 'UTF8'))::pg_catalog.int8 AS datasets_bytes,
       pg_catalog.substr(pg_catalog.convert_to(COALESCE(e.covered_datasets::pg_catalog.text, ''), 'UTF8'), 1, 4097)::pg_catalog.bytea AS datasets_prefix,
       (e.covered_uses IS NULL)::pg_catalog.bool AS uses_is_null,
       pg_catalog.octet_length(pg_catalog.convert_to(e.covered_uses::pg_catalog.text, 'UTF8'))::pg_catalog.int8 AS uses_bytes,
       pg_catalog.substr(pg_catalog.convert_to(COALESCE(e.covered_uses::pg_catalog.text, ''), 'UTF8'), 1, 4097)::pg_catalog.bytea AS uses_prefix,
       e.effective_from::pg_catalog.date AS effective_from,
       e.effective_until::pg_catalog.date AS effective_until,
       EXISTS (
         SELECT 1 FROM pg_catalog.unnest($3::pg_catalog.date[]) AS d(session_date)
         WHERE e.effective_from <= d.session_date AND e.effective_until >= d.session_date
       )::pg_catalog.bool AS covers_any_required
FROM e
ORDER BY e.id
LIMIT 65"#;

const ROLLBACK_SQL: &str = "ROLLBACK";
const COMMIT_SQL: &str = "COMMIT";

#[derive(Clone, Copy, PartialEq, Eq)]
enum RawCategory {
    Command,
    Setup,
    InitialContext,
    Owner,
    Entitlements,
    FinalContext,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PgCellKind {
    Bool(&'static str),
    I32(&'static str),
    I64(&'static str),
    Uuid(&'static str),
    Date(&'static str),
    BytesTriplet {
        null_alias: &'static str,
        length_alias: &'static str,
        prefix_alias: &'static str,
        accept_bytes: usize,
        fetch_bytes: usize,
    },
}

const SETUP_ROW: &[PgCellKind] = &[
    PgCellKind::BytesTriplet {
        null_alias: "search_path_is_null",
        length_alias: "search_path_bytes",
        prefix_alias: "search_path_prefix",
        accept_bytes: SHORT_TEXT_ACCEPT_BYTES,
        fetch_bytes: SHORT_TEXT_FETCH_BYTES,
    },
    PgCellKind::BytesTriplet {
        null_alias: "statement_timeout_is_null",
        length_alias: "statement_timeout_bytes",
        prefix_alias: "statement_timeout_prefix",
        accept_bytes: SHORT_TEXT_ACCEPT_BYTES,
        fetch_bytes: SHORT_TEXT_FETCH_BYTES,
    },
    PgCellKind::BytesTriplet {
        null_alias: "lock_timeout_is_null",
        length_alias: "lock_timeout_bytes",
        prefix_alias: "lock_timeout_prefix",
        accept_bytes: SHORT_TEXT_ACCEPT_BYTES,
        fetch_bytes: SHORT_TEXT_FETCH_BYTES,
    },
];

const CONTEXT_ROW: &[PgCellKind] = &[
    PgCellKind::BytesTriplet {
        null_alias: "current_user_is_null",
        length_alias: "current_user_bytes",
        prefix_alias: "current_user_prefix",
        accept_bytes: SHORT_TEXT_ACCEPT_BYTES,
        fetch_bytes: SHORT_TEXT_FETCH_BYTES,
    },
    PgCellKind::BytesTriplet {
        null_alias: "session_user_is_null",
        length_alias: "session_user_bytes",
        prefix_alias: "session_user_prefix",
        accept_bytes: SHORT_TEXT_ACCEPT_BYTES,
        fetch_bytes: SHORT_TEXT_FETCH_BYTES,
    },
    PgCellKind::BytesTriplet {
        null_alias: "database_name_is_null",
        length_alias: "database_name_bytes",
        prefix_alias: "database_name_prefix",
        accept_bytes: SHORT_TEXT_ACCEPT_BYTES,
        fetch_bytes: SHORT_TEXT_FETCH_BYTES,
    },
    PgCellKind::I64("database_oid"),
    PgCellKind::I64("worker_oid"),
    PgCellKind::I32("backend_pid"),
    PgCellKind::Bool("row_security_on"),
    PgCellKind::Bool("replication_origin"),
    PgCellKind::Bool("transaction_read_only"),
    PgCellKind::Bool("repeatable_read"),
    PgCellKind::Bool("search_path_is_catalog"),
    PgCellKind::I64("statement_timeout_ms"),
    PgCellKind::I64("lock_timeout_ms"),
    PgCellKind::Bool("statement_unit_ms"),
    PgCellKind::Bool("lock_unit_ms"),
];

const OWNER_ROW: &[PgCellKind] = &[
    PgCellKind::Uuid("owner_id"),
    PgCellKind::Bool("owner_role_observed"),
];

const ENTITLEMENTS_ROW: &[PgCellKind] = &[
    PgCellKind::Uuid("entitlement_id"),
    PgCellKind::BytesTriplet {
        null_alias: "reference_is_null",
        length_alias: "reference_bytes",
        prefix_alias: "reference_prefix",
        accept_bytes: REFERENCE_ACCEPT_BYTES,
        fetch_bytes: REFERENCE_FETCH_BYTES,
    },
    PgCellKind::BytesTriplet {
        null_alias: "license_hash_is_null",
        length_alias: "license_hash_bytes",
        prefix_alias: "license_hash_prefix",
        accept_bytes: LICENSE_HASH_ACCEPT_BYTES,
        fetch_bytes: LICENSE_HASH_FETCH_BYTES,
    },
    PgCellKind::BytesTriplet {
        null_alias: "status_is_null",
        length_alias: "status_bytes",
        prefix_alias: "status_prefix",
        accept_bytes: STATUS_ACCEPT_BYTES,
        fetch_bytes: STATUS_FETCH_BYTES,
    },
    PgCellKind::BytesTriplet {
        null_alias: "datasets_is_null",
        length_alias: "datasets_bytes",
        prefix_alias: "datasets_prefix",
        accept_bytes: JSON_ACCEPT_BYTES,
        fetch_bytes: JSON_FETCH_BYTES,
    },
    PgCellKind::BytesTriplet {
        null_alias: "uses_is_null",
        length_alias: "uses_bytes",
        prefix_alias: "uses_prefix",
        accept_bytes: JSON_ACCEPT_BYTES,
        fetch_bytes: JSON_FETCH_BYTES,
    },
    PgCellKind::Date("effective_from"),
    PgCellKind::Date("effective_until"),
    PgCellKind::Bool("covers_any_required"),
];

#[derive(Clone, PartialEq, Eq)]
enum RawCell {
    Null,
    WrongType,
    Bool(bool),
    I32(i32),
    I64(i64),
    Uuid(Uuid),
    Date(NaiveDate),
    Bytes {
        is_null: bool,
        original_bytes: Option<i64>,
        prefix: Vec<u8>,
    },
}

#[derive(Clone, PartialEq, Eq)]
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
    fn charge_date_input(
        &mut self,
        dates: &[TradingDate],
    ) -> Result<(), SimulationConfiguredInputRightsError> {
        let bytes = dates.len().checked_mul(4).ok_or_else(|| {
            SimulationConfiguredInputRightsError::primary(PrimaryFailure::PayloadExceeded)
        })?;
        self.charge(bytes)
    }

    fn charge_cell(&mut self, cell: &RawCell) -> Result<(), SimulationConfiguredInputRightsError> {
        let bytes = match cell {
            RawCell::Bytes { prefix, .. } => prefix.len().checked_add(RAW_TRIPLET_OVERHEAD_BYTES),
            RawCell::Uuid(_) => Some(16),
            RawCell::Date(_) => Some(4),
            RawCell::Bool(_) | RawCell::Null | RawCell::WrongType => Some(1),
            RawCell::I32(_) => Some(4),
            RawCell::I64(_) => Some(8),
        }
        .ok_or_else(|| {
            SimulationConfiguredInputRightsError::primary(PrimaryFailure::PayloadExceeded)
        })?;
        self.charge(bytes)
    }

    fn charge(&mut self, bytes: usize) -> Result<(), SimulationConfiguredInputRightsError> {
        self.used = self
            .used
            .checked_add(bytes)
            .filter(|total| *total <= MAX_LOGICAL_PAYLOAD_BYTES)
            .ok_or_else(|| {
                SimulationConfiguredInputRightsError::primary(PrimaryFailure::PayloadExceeded)
            })?;
        Ok(())
    }

    fn charge_rows(&mut self, rows: &[RawRow]) -> Result<(), SimulationConfiguredInputRightsError> {
        for row in rows {
            for cell in &row.cells {
                self.charge_cell(cell)?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq)]
struct SummaryFields {
    artifact_manifest_sha256: String,
    candidate_content_sha256: String,
    price_manifest_line_sha256: String,
    action_manifest_line_sha256: String,
    range_start: TradingDate,
    range_end: TradingDate,
    instruments: Vec<String>,
    instrument_count: usize,
    session_count: usize,
    row_count: usize,
    bars_sha256: String,
    bars_row_count: usize,
}

#[derive(Clone, PartialEq, Eq)]
struct PinFields {
    candidate_content_sha256: String,
    artifact_manifest_sha256: String,
    stage5_manifest_sha256: String,
    action_manifest_sha256: String,
    approval_registry_sha256: String,
}

#[derive(Clone, PartialEq, Eq)]
struct PrimitiveInputBar {
    instrument: String,
    session_date: TradingDate,
}

enum InputBars<'a> {
    Sealed(&'a [HistoricalPriceOnlyBar]),
    #[cfg(test)]
    Primitive(&'a [PrimitiveInputBar]),
}

impl InputBars<'_> {
    fn len(&self) -> usize {
        match self {
            Self::Sealed(bars) => bars.len(),
            #[cfg(test)]
            Self::Primitive(bars) => bars.len(),
        }
    }

    fn iter(&self) -> InputBarIter<'_> {
        match self {
            Self::Sealed(bars) => InputBarIter::Sealed(bars.iter()),
            #[cfg(test)]
            Self::Primitive(bars) => InputBarIter::Primitive(bars.iter()),
        }
    }
}

enum InputBarIter<'a> {
    Sealed(std::slice::Iter<'a, HistoricalPriceOnlyBar>),
    #[cfg(test)]
    Primitive(std::slice::Iter<'a, PrimitiveInputBar>),
}

impl Iterator for InputBarIter<'_> {
    type Item = PrimitiveInputBar;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Sealed(bars) => bars.next().map(|bar| PrimitiveInputBar {
                instrument: bar.instrument_id.to_string(),
                session_date: bar.session_date,
            }),
            #[cfg(test)]
            Self::Primitive(bars) => bars.next().cloned(),
        }
    }
}

struct ApprovedInputView<'a> {
    summary: Option<SummaryFields>,
    pins: PinFields,
    bars: InputBars<'a>,
}

impl<'a> ApprovedInputView<'a> {
    fn from_sealed(
        approved: &'a ApprovedHistoricalPriceOnlyArtifact,
        canceled: &AtomicBool,
    ) -> Result<Self, SimulationConfiguredInputRightsError> {
        if canceled.load(Ordering::SeqCst) {
            return Err(SimulationConfiguredInputRightsError::canceled());
        }
        let summary = approved.v3_approval_summary().ok_or_else(|| {
            SimulationConfiguredInputRightsError::primary(PrimaryFailure::InputInvalid)
        })?;
        let bars = approved.bars();
        check_sealed_summary_bounds(summary, bars.len())?;
        let summary_fields = SummaryFields {
            artifact_manifest_sha256: summary.artifact_manifest_sha256().as_str().to_owned(),
            candidate_content_sha256: summary.candidate_content_sha256().as_str().to_owned(),
            price_manifest_line_sha256: summary.price_manifest_line_sha256().as_str().to_owned(),
            action_manifest_line_sha256: summary.action_manifest_line_sha256().as_str().to_owned(),
            range_start: summary.range_start(),
            range_end: summary.range_end(),
            instruments: summary.instruments().to_vec(),
            instrument_count: summary.instrument_count(),
            session_count: summary.session_count(),
            row_count: summary.row_count(),
            bars_sha256: summary.bars_sha256().as_str().to_owned(),
            bars_row_count: summary.bars_row_count(),
        };
        let pins = approved.pins();
        Ok(Self {
            summary: Some(summary_fields),
            pins: PinFields {
                candidate_content_sha256: pins.candidate_content_sha256().as_str().to_owned(),
                artifact_manifest_sha256: pins.artifact_manifest_sha256().as_str().to_owned(),
                stage5_manifest_sha256: pins.stage5_manifest_sha256().as_str().to_owned(),
                action_manifest_sha256: pins.action_manifest_sha256().as_str().to_owned(),
                approval_registry_sha256: pins.approval_registry_sha256().as_str().to_owned(),
            },
            bars: InputBars::Sealed(bars),
        })
    }
}

fn expected_date(value: &'static str) -> Result<TradingDate, SimulationConfiguredInputRightsError> {
    TradingDate::parse(value)
        .map_err(|_| SimulationConfiguredInputRightsError::primary(PrimaryFailure::InputInvalid))
}

fn check_sealed_summary_bounds(
    summary: &HistoricalPriceOnlyV3ArtifactApprovalSummary,
    bars_len: usize,
) -> Result<(), SimulationConfiguredInputRightsError> {
    let expected_start = expected_date(EXPECTED_START)?;
    let expected_end = expected_date(EXPECTED_END)?;
    let instruments = summary.instruments();
    if bars_len != EXPECTED_BARS
        || summary.row_count() != EXPECTED_BARS
        || summary.bars_row_count() != EXPECTED_BARS
        || instruments.len() != EXPECTED_INSTRUMENTS
        || summary.instrument_count() != EXPECTED_INSTRUMENTS
        || summary.session_count() != EXPECTED_SESSIONS
        || summary.range_start() != expected_start
        || summary.range_end() != expected_end
    {
        return Err(SimulationConfiguredInputRightsError::primary(
            PrimaryFailure::InputInvalid,
        ));
    }
    let mut seen = BTreeSet::new();
    for instrument in instruments {
        if instrument.is_empty()
            || instrument.len() > MAX_IDENTIFIER_BYTES
            || !seen.insert(instrument.as_str())
        {
            return Err(SimulationConfiguredInputRightsError::primary(
                PrimaryFailure::InputInvalid,
            ));
        }
    }
    Ok(())
}

struct ConfigView {
    config_sha256: String,
    owner: Uuid,
    entitlement_reference: String,
    entitlement_sha256: String,
    startup_timeout_seconds: u64,
}

impl ConfigView {
    fn from_checked(config: &SimulationProcessConfig) -> Self {
        Self {
            config_sha256: config.config_sha256().to_owned(),
            owner: config.owner(),
            entitlement_reference: config.entitlement_reference().to_owned(),
            entitlement_sha256: config.entitlement_sha256().to_owned(),
            startup_timeout_seconds: config.limits().startup_timeout_seconds(),
        }
    }
}

struct ValidatedConfig {
    config_sha256: String,
    owner: Uuid,
    entitlement_reference: String,
    bare_license_hash: String,
    timeout_ms: i64,
}

fn valid_reference(value: &str) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.chars().count() <= 512
        && value.len() <= REFERENCE_ACCEPT_BYTES
        && !value
            .chars()
            .any(|character| matches!(character as u32, 0x00..=0x1f | 0x7f..=0x9f))
}

fn bare_license_hash(value: &str) -> Option<&str> {
    let bare = value.strip_prefix("sha256:")?;
    (bare.len() == LICENSE_HASH_ACCEPT_BYTES
        && bare
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte)))
    .then_some(bare)
}

fn timeout_millis(seconds: u64) -> Result<i64, SimulationConfiguredInputRightsError> {
    let product = seconds
        .checked_mul(1_000)
        .and_then(|value| i64::try_from(value).ok())
        .ok_or_else(|| {
            SimulationConfiguredInputRightsError::primary(PrimaryFailure::InvalidBound)
        })?;
    let millis = product.min(MAX_STATEMENT_TIMEOUT_MS);
    if millis <= 0 || millis > i64::from(i32::MAX) {
        return Err(SimulationConfiguredInputRightsError::primary(
            PrimaryFailure::InvalidBound,
        ));
    }
    Ok(millis)
}

fn validate_config_view(
    config: ConfigView,
) -> Result<ValidatedConfig, SimulationConfiguredInputRightsError> {
    let Some(bare_hash) = bare_license_hash(&config.entitlement_sha256) else {
        return Err(SimulationConfiguredInputRightsError::primary(
            PrimaryFailure::InvalidBound,
        ));
    };
    if !valid_reference(&config.entitlement_reference) || config.config_sha256.is_empty() {
        return Err(SimulationConfiguredInputRightsError::primary(
            PrimaryFailure::InvalidBound,
        ));
    }
    Ok(ValidatedConfig {
        config_sha256: config.config_sha256,
        owner: config.owner,
        entitlement_reference: config.entitlement_reference,
        bare_license_hash: bare_hash.to_owned(),
        timeout_ms: timeout_millis(config.startup_timeout_seconds)?,
    })
}

struct PreparedInput {
    pins: PinFields,
    artifact_bars_sha256: String,
    dates: Vec<TradingDate>,
    instrument_count: usize,
    bar_count: usize,
}

fn validate_input_view(
    view: ApprovedInputView<'_>,
    canceled: &AtomicBool,
) -> Result<PreparedInput, SimulationConfiguredInputRightsError> {
    if canceled.load(Ordering::SeqCst) {
        return Err(SimulationConfiguredInputRightsError::canceled());
    }
    let summary = view.summary.ok_or_else(|| {
        SimulationConfiguredInputRightsError::primary(PrimaryFailure::InputInvalid)
    })?;
    let bars_len = view.bars.len();
    if bars_len != EXPECTED_BARS
        || summary.row_count != EXPECTED_BARS
        || summary.bars_row_count != EXPECTED_BARS
        || summary.instruments.len() != EXPECTED_INSTRUMENTS
        || summary.instrument_count != EXPECTED_INSTRUMENTS
        || summary.session_count != EXPECTED_SESSIONS
        || summary.range_start != expected_date(EXPECTED_START)?
        || summary.range_end != expected_date(EXPECTED_END)?
        || summary.candidate_content_sha256 != view.pins.candidate_content_sha256
        || summary.artifact_manifest_sha256 != view.pins.artifact_manifest_sha256
        || summary.price_manifest_line_sha256 != view.pins.stage5_manifest_sha256
        || summary.action_manifest_line_sha256 != view.pins.action_manifest_sha256
    {
        return Err(SimulationConfiguredInputRightsError::primary(
            PrimaryFailure::InputInvalid,
        ));
    }

    let mut instrument_indexes = BTreeMap::<String, usize>::new();
    for (index, instrument) in summary.instruments.iter().enumerate() {
        if instrument.is_empty()
            || instrument.len() > MAX_IDENTIFIER_BYTES
            || instrument_indexes
                .insert(instrument.clone(), index)
                .is_some()
        {
            return Err(SimulationConfiguredInputRightsError::primary(
                PrimaryFailure::InputInvalid,
            ));
        }
    }
    let full_mask = (1_u16 << EXPECTED_INSTRUMENTS) - 1;
    let mut date_masks = BTreeMap::<TradingDate, u16>::new();
    for (index, bar) in view.bars.iter().enumerate() {
        if index % CANCEL_CHECK_INTERVAL == 0 && canceled.load(Ordering::SeqCst) {
            return Err(SimulationConfiguredInputRightsError::canceled());
        }
        if bar.instrument.is_empty() || bar.instrument.len() > MAX_IDENTIFIER_BYTES {
            return Err(SimulationConfiguredInputRightsError::primary(
                PrimaryFailure::InputInvalid,
            ));
        }
        let Some(instrument_index) = instrument_indexes.get(&bar.instrument).copied() else {
            return Err(SimulationConfiguredInputRightsError::primary(
                PrimaryFailure::InputInvalid,
            ));
        };
        if bar.session_date < summary.range_start || bar.session_date > summary.range_end {
            return Err(SimulationConfiguredInputRightsError::primary(
                PrimaryFailure::InputInvalid,
            ));
        }
        if !date_masks.contains_key(&bar.session_date) {
            if date_masks.len() >= EXPECTED_SESSIONS {
                return Err(SimulationConfiguredInputRightsError::primary(
                    PrimaryFailure::InputInvalid,
                ));
            }
            date_masks.insert(bar.session_date, 0);
        }
        let mask = date_masks.get_mut(&bar.session_date).ok_or_else(|| {
            SimulationConfiguredInputRightsError::primary(PrimaryFailure::InputInvalid)
        })?;
        let bit = 1_u16 << instrument_index;
        if *mask & bit != 0 {
            return Err(SimulationConfiguredInputRightsError::primary(
                PrimaryFailure::InputInvalid,
            ));
        }
        *mask |= bit;
    }
    if canceled.load(Ordering::SeqCst) {
        return Err(SimulationConfiguredInputRightsError::canceled());
    }
    if date_masks.len() != EXPECTED_SESSIONS
        || date_masks.values().any(|mask| *mask != full_mask)
        || date_masks.keys().next().copied() != Some(summary.range_start)
        || date_masks.keys().next_back().copied() != Some(summary.range_end)
    {
        return Err(SimulationConfiguredInputRightsError::primary(
            PrimaryFailure::InputInvalid,
        ));
    }
    let dates = date_masks.keys().copied().collect::<Vec<_>>();
    if dates.len() != EXPECTED_SESSIONS {
        return Err(SimulationConfiguredInputRightsError::primary(
            PrimaryFailure::InputInvalid,
        ));
    }

    Ok(PreparedInput {
        pins: view.pins,
        artifact_bars_sha256: summary.bars_sha256,
        dates,
        instrument_count: EXPECTED_INSTRUMENTS,
        bar_count: bars_len,
    })
}

#[derive(Clone, PartialEq, Eq)]
struct ContextEvidence {
    current_user: String,
    session_user: String,
    database_name: String,
    database_oid: u32,
    worker_oid: u32,
    backend_pid: i32,
    row_security_on: bool,
    replication_origin: bool,
    transaction_read_only: bool,
    repeatable_read: bool,
    search_path_is_catalog: bool,
    statement_timeout_ms: i64,
    lock_timeout_ms: i64,
    statement_unit_ms: bool,
    lock_unit_ms: bool,
}

struct PreparedObservation {
    context: ContextEvidence,
    matching_license_row_ids: Vec<Uuid>,
}

/// Opaque, value-free result type. Construction stays inside the acknowledged
/// commit and confirmed-close path below.
pub(super) struct SimulationConfiguredInputRightsObservation {
    config_sha256: String,
    policy_id: &'static str,
    policy_fingerprint: &'static str,
    owner_uuid: Uuid,
    exact_reference: String,
    bare_license_hash: String,
    candidate_content_sha256: String,
    artifact_manifest_sha256: String,
    stage5_manifest_sha256: String,
    action_manifest_sha256: String,
    approval_registry_sha256: String,
    artifact_bars_sha256: String,
    actual_sessions: Vec<TradingDate>,
    instrument_count: usize,
    session_count: usize,
    bar_count: usize,
    database_name: String,
    database_oid: u32,
    worker_oid: u32,
    backend_pid: i32,
    matching_license_row_ids: Vec<Uuid>,
    logical_bytes: usize,
}

impl SimulationConfiguredInputRightsObservation {
    pub(super) fn config_sha256(&self) -> &str {
        &self.config_sha256
    }
    pub(super) fn database_oid(&self) -> u32 {
        self.database_oid
    }
    pub(super) fn worker_oid(&self) -> u32 {
        self.worker_oid
    }

    fn completed(
        config: ValidatedConfig,
        input: PreparedInput,
        prepared: PreparedObservation,
        logical_bytes: usize,
    ) -> Self {
        let session_count = input.dates.len();
        Self {
            config_sha256: config.config_sha256,
            policy_id: POLICY_ID,
            policy_fingerprint: POLICY_FINGERPRINT,
            owner_uuid: config.owner,
            exact_reference: config.entitlement_reference,
            bare_license_hash: config.bare_license_hash,
            candidate_content_sha256: input.pins.candidate_content_sha256,
            artifact_manifest_sha256: input.pins.artifact_manifest_sha256,
            stage5_manifest_sha256: input.pins.stage5_manifest_sha256,
            action_manifest_sha256: input.pins.action_manifest_sha256,
            approval_registry_sha256: input.pins.approval_registry_sha256,
            artifact_bars_sha256: input.artifact_bars_sha256,
            actual_sessions: input.dates,
            instrument_count: input.instrument_count,
            session_count,
            bar_count: input.bar_count,
            database_name: prepared.context.database_name,
            database_oid: prepared.context.database_oid,
            worker_oid: prepared.context.worker_oid,
            backend_pid: prepared.context.backend_pid,
            matching_license_row_ids: prepared.matching_license_row_ids,
            logical_bytes,
        }
    }
}

impl fmt::Debug for SimulationConfiguredInputRightsObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SimulationConfiguredInputRightsObservation")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum StatementKind {
    Begin,
    Configure,
    InitialContext,
    Owner,
    Entitlements,
    FinalContext,
    Commit,
    Rollback,
}

enum Statement {
    Begin,
    Configure {
        timeout_ms: i64,
    },
    InitialContext,
    Owner {
        owner: Uuid,
    },
    Entitlements {
        reference: String,
        bare_hash: String,
        dates: Vec<NaiveDate>,
    },
    FinalContext,
    Commit,
    Rollback,
}

impl Statement {
    fn kind(&self) -> StatementKind {
        match self {
            Self::Begin => StatementKind::Begin,
            Self::Configure { .. } => StatementKind::Configure,
            Self::InitialContext => StatementKind::InitialContext,
            Self::Owner { .. } => StatementKind::Owner,
            Self::Entitlements { .. } => StatementKind::Entitlements,
            Self::FinalContext => StatementKind::FinalContext,
            Self::Commit => StatementKind::Commit,
            Self::Rollback => StatementKind::Rollback,
        }
    }
}

enum BindValue {
    I64(i64),
    Uuid(Uuid),
    Text(String),
    DateArray(Vec<NaiveDate>),
}

struct StatementPlan {
    kind: StatementKind,
    sql: &'static str,
    binds: Vec<BindValue>,
    category: Option<RawCategory>,
    shape: &'static [PgCellKind],
    accept_rows: usize,
    fetch_rows: usize,
}

fn statement_plan(statement: Statement) -> StatementPlan {
    let (kind, sql, binds, category, shape, accept_rows, fetch_rows) = match statement {
        Statement::Begin => (
            StatementKind::Begin,
            BEGIN_SQL,
            Vec::new(),
            Some(RawCategory::Command),
            &[][..],
            0,
            1,
        ),
        Statement::Configure { timeout_ms } => (
            StatementKind::Configure,
            CONFIGURE_SQL,
            vec![BindValue::I64(timeout_ms)],
            Some(RawCategory::Setup),
            SETUP_ROW,
            SETUP_ACCEPT_ROWS,
            SETUP_FETCH_ROWS,
        ),
        Statement::InitialContext => (
            StatementKind::InitialContext,
            CONTEXT_SQL,
            Vec::new(),
            Some(RawCategory::InitialContext),
            CONTEXT_ROW,
            CONTEXT_ACCEPT_ROWS,
            CONTEXT_FETCH_ROWS,
        ),
        Statement::Owner { owner } => (
            StatementKind::Owner,
            OWNER_SQL,
            vec![BindValue::Uuid(owner)],
            Some(RawCategory::Owner),
            OWNER_ROW,
            OWNER_ACCEPT_ROWS,
            OWNER_FETCH_ROWS,
        ),
        Statement::Entitlements {
            reference,
            bare_hash,
            dates,
        } => (
            StatementKind::Entitlements,
            ENTITLEMENTS_SQL,
            vec![
                BindValue::Text(reference),
                BindValue::Text(bare_hash),
                BindValue::DateArray(dates),
            ],
            Some(RawCategory::Entitlements),
            ENTITLEMENTS_ROW,
            ENTITLEMENT_ACCEPT_ROWS,
            ENTITLEMENT_FETCH_ROWS,
        ),
        Statement::FinalContext => (
            StatementKind::FinalContext,
            CONTEXT_SQL,
            Vec::new(),
            Some(RawCategory::FinalContext),
            CONTEXT_ROW,
            CONTEXT_ACCEPT_ROWS,
            CONTEXT_FETCH_ROWS,
        ),
        Statement::Commit => (
            StatementKind::Commit,
            COMMIT_SQL,
            Vec::new(),
            Some(RawCategory::Command),
            &[][..],
            0,
            1,
        ),
        Statement::Rollback => (
            StatementKind::Rollback,
            ROLLBACK_SQL,
            Vec::new(),
            Some(RawCategory::Command),
            &[][..],
            0,
            1,
        ),
    };
    StatementPlan {
        kind,
        sql,
        binds,
        category,
        shape,
        accept_rows,
        fetch_rows,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct ProjectionColumn {
    alias: &'static str,
    pg_type: &'static str,
}

fn projection_columns(shape: &[PgCellKind]) -> Vec<ProjectionColumn> {
    let mut columns = Vec::new();
    for cell in shape {
        match cell {
            PgCellKind::Bool(alias) => columns.push(ProjectionColumn {
                alias,
                pg_type: "BOOL",
            }),
            PgCellKind::I32(alias) => columns.push(ProjectionColumn {
                alias,
                pg_type: "INT4",
            }),
            PgCellKind::I64(alias) => columns.push(ProjectionColumn {
                alias,
                pg_type: "INT8",
            }),
            PgCellKind::Uuid(alias) => columns.push(ProjectionColumn {
                alias,
                pg_type: "UUID",
            }),
            PgCellKind::Date(alias) => columns.push(ProjectionColumn {
                alias,
                pg_type: "DATE",
            }),
            PgCellKind::BytesTriplet {
                null_alias,
                length_alias,
                prefix_alias,
                ..
            } => {
                columns.push(ProjectionColumn {
                    alias: null_alias,
                    pg_type: "BOOL",
                });
                columns.push(ProjectionColumn {
                    alias: length_alias,
                    pg_type: "INT8",
                });
                columns.push(ProjectionColumn {
                    alias: prefix_alias,
                    pg_type: "BYTEA",
                });
            }
        }
    }
    columns
}

fn is_sql_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn sql_word_at(sql: &str, index: usize, word: &str) -> bool {
    let bytes = sql.as_bytes();
    let word_bytes = word.as_bytes();
    if bytes.get(index..index + word_bytes.len()) != Some(word_bytes) {
        return false;
    }
    let before_ok = index == 0 || !is_sql_word_byte(bytes[index - 1]);
    let after = index + word_bytes.len();
    let after_ok = after == bytes.len() || !is_sql_word_byte(bytes[after]);
    before_ok && after_ok
}

fn outer_projection_range(sql: &str) -> Option<(usize, usize)> {
    let bytes = sql.as_bytes();
    let mut index = 0;
    let mut depth = 0_usize;
    let mut quote = None;
    let mut select_end = None;
    while index < bytes.len() {
        if let Some(delimiter) = quote {
            if bytes[index] == delimiter {
                if bytes.get(index + 1) == Some(&delimiter) {
                    index += 2;
                    continue;
                }
                quote = None;
            }
            index += 1;
            continue;
        }
        match bytes[index] {
            b'\'' | b'"' => quote = Some(bytes[index]),
            b'(' => depth = depth.checked_add(1)?,
            b')' => depth = depth.checked_sub(1)?,
            _ if depth == 0 && sql_word_at(sql, index, "SELECT") => {
                select_end = Some(index + "SELECT".len());
                index += "SELECT".len();
                continue;
            }
            _ => {}
        }
        index += 1;
    }
    let start = select_end?;
    let mut index = start;
    depth = 0;
    quote = None;
    while index < bytes.len() {
        if let Some(delimiter) = quote {
            if bytes[index] == delimiter {
                if bytes.get(index + 1) == Some(&delimiter) {
                    index += 2;
                    continue;
                }
                quote = None;
            }
            index += 1;
            continue;
        }
        match bytes[index] {
            b'\'' | b'"' => quote = Some(bytes[index]),
            b'(' => depth = depth.checked_add(1)?,
            b')' => depth = depth.checked_sub(1)?,
            _ if depth == 0 && sql_word_at(sql, index, "FROM") => return Some((start, index)),
            _ => {}
        }
        index += 1;
    }
    None
}

fn split_top_level_projection(value: &str) -> Option<Vec<&str>> {
    let bytes = value.as_bytes();
    let mut ranges = Vec::new();
    let mut start = 0;
    let mut depth = 0_usize;
    let mut quote = None;
    let mut index = 0;
    while index < bytes.len() {
        if let Some(delimiter) = quote {
            if bytes[index] == delimiter {
                if bytes.get(index + 1) == Some(&delimiter) {
                    index += 2;
                    continue;
                }
                quote = None;
            }
            index += 1;
            continue;
        }
        match bytes[index] {
            b'\'' | b'"' => quote = Some(bytes[index]),
            b'(' => depth = depth.checked_add(1)?,
            b')' => depth = depth.checked_sub(1)?,
            b',' if depth == 0 => {
                ranges.push(start..index);
                start = index + 1;
            }
            _ => {}
        }
        index += 1;
    }
    if quote.is_some() || depth != 0 {
        return None;
    }
    ranges.push(start..bytes.len());
    Some(
        ranges
            .into_iter()
            .map(|range| value[range].trim())
            .collect(),
    )
}

fn projection_signature(sql: &str) -> Option<Vec<(String, String)>> {
    let (start, end) = outer_projection_range(sql)?;
    let expressions = split_top_level_projection(sql.get(start..end)?)?;
    let mut signature = Vec::with_capacity(expressions.len());
    for expression in expressions {
        let (value, alias) = expression.rsplit_once(" AS ")?;
        let alias = alias.trim();
        if alias.is_empty() || !alias.bytes().all(is_sql_word_byte) {
            return None;
        }
        let cast_at = value.rfind("::pg_catalog.")? + "::pg_catalog.".len();
        let cast_value = value.get(cast_at..)?.trim_start();
        let cast_len = cast_value
            .bytes()
            .take_while(|byte| is_sql_word_byte(*byte))
            .count();
        if cast_len == 0 {
            return None;
        }
        signature.push((
            alias.to_owned(),
            cast_value[..cast_len].to_ascii_uppercase(),
        ));
    }
    Some(signature)
}

fn projection_matches(sql: &str, shape: &[PgCellKind]) -> bool {
    let Some(actual) = projection_signature(sql) else {
        return false;
    };
    let expected = projection_columns(shape)
        .into_iter()
        .map(|column| (column.alias.to_owned(), column.pg_type.to_owned()))
        .collect::<Vec<_>>();
    actual == expected
}

fn statement_plan_is_valid(plan: &StatementPlan) -> bool {
    let (expected_sql, expected_category, expected_shape, expected_accept, expected_fetch) =
        match plan.kind {
            StatementKind::Begin => (BEGIN_SQL, Some(RawCategory::Command), &[][..], 0, 1),
            StatementKind::Configure => (
                CONFIGURE_SQL,
                Some(RawCategory::Setup),
                SETUP_ROW,
                SETUP_ACCEPT_ROWS,
                SETUP_FETCH_ROWS,
            ),
            StatementKind::InitialContext => (
                CONTEXT_SQL,
                Some(RawCategory::InitialContext),
                CONTEXT_ROW,
                CONTEXT_ACCEPT_ROWS,
                CONTEXT_FETCH_ROWS,
            ),
            StatementKind::Owner => (
                OWNER_SQL,
                Some(RawCategory::Owner),
                OWNER_ROW,
                OWNER_ACCEPT_ROWS,
                OWNER_FETCH_ROWS,
            ),
            StatementKind::Entitlements => (
                ENTITLEMENTS_SQL,
                Some(RawCategory::Entitlements),
                ENTITLEMENTS_ROW,
                ENTITLEMENT_ACCEPT_ROWS,
                ENTITLEMENT_FETCH_ROWS,
            ),
            StatementKind::FinalContext => (
                CONTEXT_SQL,
                Some(RawCategory::FinalContext),
                CONTEXT_ROW,
                CONTEXT_ACCEPT_ROWS,
                CONTEXT_FETCH_ROWS,
            ),
            StatementKind::Commit => (COMMIT_SQL, Some(RawCategory::Command), &[][..], 0, 1),
            StatementKind::Rollback => (ROLLBACK_SQL, Some(RawCategory::Command), &[][..], 0, 1),
        };
    if plan.sql != expected_sql
        || plan.category != expected_category
        || plan.shape != expected_shape
        || plan.accept_rows != expected_accept
        || plan.fetch_rows != expected_fetch
    {
        return false;
    }

    let expected_bind_types: &[&str] = match plan.kind {
        StatementKind::Configure => &["INT8"],
        StatementKind::Owner => &["UUID"],
        StatementKind::Entitlements => &["TEXT", "TEXT", "DATE[]"],
        _ => &[],
    };
    let actual_bind_types = plan
        .binds
        .iter()
        .map(|bind| match bind {
            BindValue::I64(_) => "INT8",
            BindValue::Uuid(_) => "UUID",
            BindValue::Text(_) => "TEXT",
            BindValue::DateArray(_) => "DATE[]",
        })
        .collect::<Vec<_>>();
    if actual_bind_types != expected_bind_types {
        return false;
    }
    match (plan.kind, plan.binds.as_slice()) {
        (StatementKind::Begin | StatementKind::Commit | StatementKind::Rollback, []) => true,
        (StatementKind::InitialContext | StatementKind::FinalContext, []) => {
            projection_matches(plan.sql, plan.shape)
        }
        (StatementKind::Configure, [BindValue::I64(timeout_ms)]) => {
            (1..=MAX_STATEMENT_TIMEOUT_MS).contains(timeout_ms)
                && projection_matches(plan.sql, plan.shape)
        }
        (StatementKind::Owner, [BindValue::Uuid(_)]) => projection_matches(plan.sql, plan.shape),
        (
            StatementKind::Entitlements,
            [
                BindValue::Text(reference),
                BindValue::Text(hash),
                BindValue::DateArray(dates),
            ],
        ) => {
            valid_reference(reference)
                && bare_license_hash(&format!("sha256:{hash}")).is_some()
                && dates.len() == EXPECTED_SESSIONS
                && dates.windows(2).all(|pair| pair[0] < pair[1])
                && dates.first().copied()
                    == expected_date(EXPECTED_START)
                        .ok()
                        .map(|date| date.as_naive_date())
                && dates.last().copied()
                    == expected_date(EXPECTED_END)
                        .ok()
                        .map(|date| date.as_naive_date())
                && projection_matches(plan.sql, plan.shape)
        }
        _ => false,
    }
}

fn maximum_shape_payload(shape: &[PgCellKind], row_count: usize) -> Option<usize> {
    let per_row = shape.iter().try_fold(0_usize, |total, cell| {
        let width = match cell {
            PgCellKind::Bool(_) => 1,
            PgCellKind::I32(_) => 4,
            PgCellKind::I64(_) => 8,
            PgCellKind::Uuid(_) => 16,
            PgCellKind::Date(_) => 4,
            PgCellKind::BytesTriplet { fetch_bytes, .. } => {
                fetch_bytes.checked_add(RAW_TRIPLET_OVERHEAD_BYTES)?
            }
        };
        total.checked_add(width)
    })?;
    per_row.checked_mul(row_count)
}

fn maximum_fetched_payloads() -> Option<usize> {
    maximum_shape_payload(SETUP_ROW, SETUP_FETCH_ROWS)?
        .checked_add(maximum_shape_payload(CONTEXT_ROW, CONTEXT_FETCH_ROWS)?.checked_mul(2)?)?
        .checked_add(maximum_shape_payload(OWNER_ROW, OWNER_FETCH_ROWS)?)?
        .checked_add(maximum_shape_payload(
            ENTITLEMENTS_ROW,
            ENTITLEMENT_FETCH_ROWS,
        )?)
}

fn pg_column_count(shape: &[PgCellKind]) -> usize {
    shape
        .iter()
        .map(|cell| match cell {
            PgCellKind::BytesTriplet { .. } => 3,
            _ => 1,
        })
        .sum()
}

fn column_matches(row: &PgRow, index: usize, alias: &str, pg_type: &str) -> bool {
    row.columns()
        .get(index)
        .is_some_and(|column| column.name() == alias && column.type_info().name() == pg_type)
}

fn raw_pg_cell(
    row: &PgRow,
    index: &mut usize,
    kind: PgCellKind,
) -> Result<RawCell, TransportFailure> {
    match kind {
        PgCellKind::Bool(alias) => {
            let at = *index;
            *index += 1;
            if !column_matches(row, at, alias, "BOOL") {
                return Ok(RawCell::WrongType);
            }
            Ok(match row.try_get::<Option<bool>, _>(at) {
                Ok(Some(value)) => RawCell::Bool(value),
                Ok(None) => RawCell::Null,
                Err(_) => RawCell::WrongType,
            })
        }
        PgCellKind::I32(alias) => {
            let at = *index;
            *index += 1;
            if !column_matches(row, at, alias, "INT4") {
                return Ok(RawCell::WrongType);
            }
            Ok(match row.try_get::<Option<i32>, _>(at) {
                Ok(Some(value)) => RawCell::I32(value),
                Ok(None) => RawCell::Null,
                Err(_) => RawCell::WrongType,
            })
        }
        PgCellKind::I64(alias) => {
            let at = *index;
            *index += 1;
            if !column_matches(row, at, alias, "INT8") {
                return Ok(RawCell::WrongType);
            }
            Ok(match row.try_get::<Option<i64>, _>(at) {
                Ok(Some(value)) => RawCell::I64(value),
                Ok(None) => RawCell::Null,
                Err(_) => RawCell::WrongType,
            })
        }
        PgCellKind::Uuid(alias) => {
            let at = *index;
            *index += 1;
            if !column_matches(row, at, alias, "UUID") {
                return Ok(RawCell::WrongType);
            }
            Ok(match row.try_get::<Option<Uuid>, _>(at) {
                Ok(Some(value)) => RawCell::Uuid(value),
                Ok(None) => RawCell::Null,
                Err(_) => RawCell::WrongType,
            })
        }
        PgCellKind::Date(alias) => {
            let at = *index;
            *index += 1;
            if !column_matches(row, at, alias, "DATE") {
                return Ok(RawCell::WrongType);
            }
            Ok(match row.try_get::<Option<NaiveDate>, _>(at) {
                Ok(Some(value)) => RawCell::Date(value),
                Ok(None) => RawCell::Null,
                Err(_) => RawCell::WrongType,
            })
        }
        PgCellKind::BytesTriplet {
            null_alias,
            length_alias,
            prefix_alias,
            fetch_bytes,
            ..
        } => {
            let null_at = *index;
            let length_at = null_at + 1;
            let prefix_at = null_at + 2;
            *index += 3;
            if !column_matches(row, null_at, null_alias, "BOOL")
                || !column_matches(row, length_at, length_alias, "INT8")
                || !column_matches(row, prefix_at, prefix_alias, "BYTEA")
            {
                return Ok(RawCell::WrongType);
            }
            let is_null = match row.try_get::<Option<bool>, _>(null_at) {
                Ok(Some(value)) => value,
                _ => return Ok(RawCell::WrongType),
            };
            let original_bytes = match row.try_get::<Option<i64>, _>(length_at) {
                Ok(value) => value,
                Err(_) => return Ok(RawCell::WrongType),
            };
            let value = match row.try_get_raw(prefix_at) {
                Ok(value) if !value.is_null() => value,
                _ => return Ok(RawCell::WrongType),
            };
            let bytes = value.as_bytes().map_err(|_| TransportFailure::State)?;
            if bytes.len() > fetch_bytes {
                return Err(TransportFailure::FetchBound);
            }
            Ok(RawCell::Bytes {
                is_null,
                original_bytes,
                prefix: bytes.to_vec(),
            })
        }
    }
}

fn raw_pg_rows(
    rows: Vec<PgRow>,
    category: RawCategory,
    shape: &[PgCellKind],
    fetch_rows: usize,
) -> Result<Vec<RawRow>, TransportFailure> {
    if rows.len() > fetch_rows {
        return Err(TransportFailure::FetchBound);
    }
    rows.into_iter()
        .map(|row| {
            if row.columns().len() != pg_column_count(shape) {
                return Ok(RawRow::new(category, vec![RawCell::WrongType; shape.len()]));
            }
            let mut index = 0;
            let cells = shape
                .iter()
                .map(|kind| raw_pg_cell(&row, &mut index, *kind))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(RawRow::new(category, cells))
        })
        .collect()
}

struct RawDecoder {
    category: RawCategory,
    cells: std::vec::IntoIter<RawCell>,
}

impl RawDecoder {
    fn new(
        row: RawRow,
        category: RawCategory,
        shape: &[PgCellKind],
    ) -> Result<Self, SimulationConfiguredInputRightsError> {
        if row.category != category || row.cells.len() != shape.len() {
            return Err(SimulationConfiguredInputRightsError::primary(
                PrimaryFailure::RawInvalid,
            ));
        }
        Ok(Self {
            category,
            cells: row.cells.into_iter(),
        })
    }

    fn take(&mut self) -> Result<RawCell, SimulationConfiguredInputRightsError> {
        self.cells.next().ok_or_else(|| {
            SimulationConfiguredInputRightsError::primary(PrimaryFailure::RawInvalid)
        })
    }

    fn bool(&mut self) -> Result<bool, SimulationConfiguredInputRightsError> {
        match self.take()? {
            RawCell::Bool(value) => Ok(value),
            _ => Err(SimulationConfiguredInputRightsError::primary(
                PrimaryFailure::RawInvalid,
            )),
        }
    }

    fn i32(&mut self) -> Result<i32, SimulationConfiguredInputRightsError> {
        match self.take()? {
            RawCell::I32(value) => Ok(value),
            _ => Err(SimulationConfiguredInputRightsError::primary(
                PrimaryFailure::RawInvalid,
            )),
        }
    }

    fn i64(&mut self) -> Result<i64, SimulationConfiguredInputRightsError> {
        match self.take()? {
            RawCell::I64(value) => Ok(value),
            _ => Err(SimulationConfiguredInputRightsError::primary(
                PrimaryFailure::RawInvalid,
            )),
        }
    }

    fn uuid(&mut self) -> Result<Uuid, SimulationConfiguredInputRightsError> {
        match self.take()? {
            RawCell::Uuid(value) => Ok(value),
            _ => Err(SimulationConfiguredInputRightsError::primary(
                PrimaryFailure::RawInvalid,
            )),
        }
    }

    fn date(&mut self) -> Result<NaiveDate, SimulationConfiguredInputRightsError> {
        match self.take()? {
            RawCell::Date(value) => Ok(value),
            _ => Err(SimulationConfiguredInputRightsError::primary(
                PrimaryFailure::RawInvalid,
            )),
        }
    }

    fn bytes(
        &mut self,
        accept_bytes: usize,
        fetch_bytes: usize,
    ) -> Result<String, SimulationConfiguredInputRightsError> {
        let RawCell::Bytes {
            is_null,
            original_bytes,
            prefix,
        } = self.take()?
        else {
            return Err(SimulationConfiguredInputRightsError::primary(
                PrimaryFailure::RawInvalid,
            ));
        };
        if is_null {
            if original_bytes.is_none() && prefix.is_empty() {
                return Err(SimulationConfiguredInputRightsError::primary(
                    PrimaryFailure::RawInvalid,
                ));
            }
            return Err(SimulationConfiguredInputRightsError::primary(
                PrimaryFailure::RawInvalid,
            ));
        }
        let original_bytes = original_bytes
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| {
                SimulationConfiguredInputRightsError::primary(PrimaryFailure::RawInvalid)
            })?;
        if prefix.len() > fetch_bytes {
            return Err(SimulationConfiguredInputRightsError::primary(
                PrimaryFailure::BoundExceeded,
            ));
        }
        if prefix.len() != original_bytes.min(fetch_bytes) {
            return Err(SimulationConfiguredInputRightsError::primary(
                PrimaryFailure::RawInvalid,
            ));
        }
        if original_bytes > accept_bytes {
            return Err(SimulationConfiguredInputRightsError::primary(
                PrimaryFailure::BoundExceeded,
            ));
        }
        String::from_utf8(prefix)
            .map_err(|_| SimulationConfiguredInputRightsError::primary(PrimaryFailure::RawInvalid))
    }

    fn finish(self) -> Result<(), SimulationConfiguredInputRightsError> {
        let _ = self.category;
        if self.cells.len() == 0 {
            Ok(())
        } else {
            Err(SimulationConfiguredInputRightsError::primary(
                PrimaryFailure::RawInvalid,
            ))
        }
    }
}

fn prepare_raw_rows(
    rows: Vec<RawRow>,
    category: RawCategory,
    shape: &[PgCellKind],
    minimum_rows: usize,
    accept_rows: usize,
    fetch_rows: usize,
    missing_failure: PrimaryFailure,
    budget: &mut LogicalPayloadBudget,
) -> Result<Vec<RawDecoder>, SimulationConfiguredInputRightsError> {
    budget.charge_rows(&rows)?;
    if rows.len() > fetch_rows || rows.len() > accept_rows {
        return Err(SimulationConfiguredInputRightsError::primary(
            PrimaryFailure::BoundExceeded,
        ));
    }
    if rows.len() < minimum_rows {
        return Err(SimulationConfiguredInputRightsError::primary(
            missing_failure,
        ));
    }
    if rows
        .iter()
        .any(|row| row.category != category || row.cells.len() != shape.len())
    {
        return Err(SimulationConfiguredInputRightsError::primary(
            PrimaryFailure::RawInvalid,
        ));
    }
    rows.into_iter()
        .map(|row| RawDecoder::new(row, category, shape))
        .collect()
}

fn validate_setup(
    rows: Vec<RawRow>,
    budget: &mut LogicalPayloadBudget,
) -> Result<(), SimulationConfiguredInputRightsError> {
    let mut decoded = prepare_raw_rows(
        rows,
        RawCategory::Setup,
        SETUP_ROW,
        1,
        SETUP_ACCEPT_ROWS,
        SETUP_FETCH_ROWS,
        PrimaryFailure::SetupResultInvalid,
        budget,
    )?;
    let mut row = decoded.pop().ok_or_else(|| {
        SimulationConfiguredInputRightsError::primary(PrimaryFailure::SetupResultInvalid)
    })?;
    let search_path = row.bytes(SHORT_TEXT_ACCEPT_BYTES, SHORT_TEXT_FETCH_BYTES)?;
    let statement_timeout = row.bytes(SHORT_TEXT_ACCEPT_BYTES, SHORT_TEXT_FETCH_BYTES)?;
    let lock_timeout = row.bytes(SHORT_TEXT_ACCEPT_BYTES, SHORT_TEXT_FETCH_BYTES)?;
    row.finish()?;
    if search_path != "pg_catalog" || statement_timeout.is_empty() || lock_timeout.is_empty() {
        return Err(SimulationConfiguredInputRightsError::primary(
            PrimaryFailure::SetupResultInvalid,
        ));
    }
    Ok(())
}

fn positive_oid(value: i64) -> Option<u32> {
    u32::try_from(value).ok().filter(|oid| *oid > 0)
}

fn validate_context(
    rows: Vec<RawRow>,
    category: RawCategory,
    timeout_ms: i64,
    budget: &mut LogicalPayloadBudget,
) -> Result<ContextEvidence, SimulationConfiguredInputRightsError> {
    let mut decoded = prepare_raw_rows(
        rows,
        category,
        CONTEXT_ROW,
        1,
        CONTEXT_ACCEPT_ROWS,
        CONTEXT_FETCH_ROWS,
        PrimaryFailure::PolicyRejected,
        budget,
    )?;
    let mut row = decoded.pop().ok_or_else(|| {
        SimulationConfiguredInputRightsError::primary(PrimaryFailure::PolicyRejected)
    })?;
    let current_user = row.bytes(SHORT_TEXT_ACCEPT_BYTES, SHORT_TEXT_FETCH_BYTES)?;
    let session_user = row.bytes(SHORT_TEXT_ACCEPT_BYTES, SHORT_TEXT_FETCH_BYTES)?;
    let database_name = row.bytes(SHORT_TEXT_ACCEPT_BYTES, SHORT_TEXT_FETCH_BYTES)?;
    let database_oid = row.i64()?;
    let worker_oid = row.i64()?;
    let backend_pid = row.i32()?;
    let row_security_on = row.bool()?;
    let replication_origin = row.bool()?;
    let transaction_read_only = row.bool()?;
    let repeatable_read = row.bool()?;
    let search_path_is_catalog = row.bool()?;
    let statement_timeout_ms = row.i64()?;
    let lock_timeout_ms = row.i64()?;
    let statement_unit_ms = row.bool()?;
    let lock_unit_ms = row.bool()?;
    row.finish()?;

    let Some(database_oid) = positive_oid(database_oid) else {
        return Err(SimulationConfiguredInputRightsError::primary(
            PrimaryFailure::PolicyRejected,
        ));
    };
    let Some(worker_oid) = positive_oid(worker_oid) else {
        return Err(SimulationConfiguredInputRightsError::primary(
            PrimaryFailure::PolicyRejected,
        ));
    };
    if current_user != "worker"
        || session_user != "worker"
        || database_name.is_empty()
        || backend_pid <= 0
        || !row_security_on
        || !replication_origin
        || !transaction_read_only
        || !repeatable_read
        || !search_path_is_catalog
        || statement_timeout_ms != timeout_ms
        || lock_timeout_ms != timeout_ms
        || !statement_unit_ms
        || !lock_unit_ms
    {
        return Err(SimulationConfiguredInputRightsError::primary(
            PrimaryFailure::PolicyRejected,
        ));
    }
    Ok(ContextEvidence {
        current_user,
        session_user,
        database_name,
        database_oid,
        worker_oid,
        backend_pid,
        row_security_on,
        replication_origin,
        transaction_read_only,
        repeatable_read,
        search_path_is_catalog,
        statement_timeout_ms,
        lock_timeout_ms,
        statement_unit_ms,
        lock_unit_ms,
    })
}

fn compare_contexts(
    initial: &ContextEvidence,
    final_context: &ContextEvidence,
) -> Result<(), SimulationConfiguredInputRightsError> {
    if initial == final_context {
        Ok(())
    } else {
        Err(SimulationConfiguredInputRightsError::primary(
            PrimaryFailure::PolicyRejected,
        ))
    }
}

fn validate_owner(
    rows: Vec<RawRow>,
    expected_owner: Uuid,
    budget: &mut LogicalPayloadBudget,
) -> Result<(), SimulationConfiguredInputRightsError> {
    let mut decoded = prepare_raw_rows(
        rows,
        RawCategory::Owner,
        OWNER_ROW,
        1,
        OWNER_ACCEPT_ROWS,
        OWNER_FETCH_ROWS,
        PrimaryFailure::PolicyRejected,
        budget,
    )?;
    let mut row = decoded.pop().ok_or_else(|| {
        SimulationConfiguredInputRightsError::primary(PrimaryFailure::PolicyRejected)
    })?;
    let owner = row.uuid()?;
    let owner_role_observed = row.bool()?;
    row.finish()?;
    if owner != expected_owner || !owner_role_observed {
        return Err(SimulationConfiguredInputRightsError::primary(
            PrimaryFailure::PolicyRejected,
        ));
    }
    Ok(())
}

fn decode_identifiers(
    value: String,
) -> Result<BTreeSet<String>, SimulationConfiguredInputRightsError> {
    let parsed: serde_json::Value = serde_json::from_str(&value)
        .map_err(|_| SimulationConfiguredInputRightsError::primary(PrimaryFailure::RawInvalid))?;
    let serde_json::Value::Array(values) = parsed else {
        return Err(SimulationConfiguredInputRightsError::primary(
            PrimaryFailure::RawInvalid,
        ));
    };
    if values.len() > MAX_IDENTIFIER_COUNT {
        return Err(SimulationConfiguredInputRightsError::primary(
            PrimaryFailure::BoundExceeded,
        ));
    }
    let mut identifiers = BTreeSet::new();
    for value in values {
        let serde_json::Value::String(identifier) = value else {
            return Err(SimulationConfiguredInputRightsError::primary(
                PrimaryFailure::RawInvalid,
            ));
        };
        if identifier.is_empty()
            || identifier.len() > MAX_IDENTIFIER_BYTES
            || identifier.chars().any(char::is_control)
            || !identifiers.insert(identifier)
        {
            return Err(SimulationConfiguredInputRightsError::primary(
                PrimaryFailure::PolicyRejected,
            ));
        }
    }
    Ok(identifiers)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EntitlementStatus {
    Pending,
    Active,
    Expired,
    Revoked,
}

struct EntitlementRecord {
    id: Uuid,
    status: EntitlementStatus,
    datasets: BTreeSet<String>,
    uses: BTreeSet<String>,
    effective_from: NaiveDate,
    effective_until: NaiveDate,
    covers_any_required: bool,
}

fn decode_entitlement_record(
    mut row: RawDecoder,
    config: &ValidatedConfig,
) -> Result<EntitlementRecord, SimulationConfiguredInputRightsError> {
    let id = row.uuid()?;
    let reference = row.bytes(REFERENCE_ACCEPT_BYTES, REFERENCE_FETCH_BYTES)?;
    let license_hash = row.bytes(LICENSE_HASH_ACCEPT_BYTES, LICENSE_HASH_FETCH_BYTES)?;
    let status = row.bytes(STATUS_ACCEPT_BYTES, STATUS_FETCH_BYTES)?;
    let datasets = decode_identifiers(row.bytes(JSON_ACCEPT_BYTES, JSON_FETCH_BYTES)?)?;
    let uses = decode_identifiers(row.bytes(JSON_ACCEPT_BYTES, JSON_FETCH_BYTES)?)?;
    let effective_from = row.date()?;
    let effective_until = row.date()?;
    let covers_any_required = row.bool()?;
    row.finish()?;

    if reference != config.entitlement_reference || license_hash != config.bare_license_hash {
        return Err(SimulationConfiguredInputRightsError::primary(
            PrimaryFailure::PolicyRejected,
        ));
    }
    let status = match status.as_str() {
        "PENDING" => EntitlementStatus::Pending,
        "ACTIVE" => EntitlementStatus::Active,
        "EXPIRED" => EntitlementStatus::Expired,
        "REVOKED" => EntitlementStatus::Revoked,
        _ => {
            return Err(SimulationConfiguredInputRightsError::primary(
                PrimaryFailure::PolicyRejected,
            ));
        }
    };
    if effective_from > effective_until {
        return Err(SimulationConfiguredInputRightsError::primary(
            PrimaryFailure::PolicyRejected,
        ));
    }
    Ok(EntitlementRecord {
        id,
        status,
        datasets,
        uses,
        effective_from,
        effective_until,
        covers_any_required,
    })
}

fn validate_entitlements(
    rows: Vec<RawRow>,
    config: &ValidatedConfig,
    dates: &[TradingDate],
    canceled: &AtomicBool,
    budget: &mut LogicalPayloadBudget,
) -> Result<Vec<Uuid>, SimulationConfiguredInputRightsError> {
    let decoded_rows = prepare_raw_rows(
        rows,
        RawCategory::Entitlements,
        ENTITLEMENTS_ROW,
        1,
        ENTITLEMENT_ACCEPT_ROWS,
        ENTITLEMENT_FETCH_ROWS,
        PrimaryFailure::PolicyRejected,
        budget,
    )?;
    let mut entitlements = Vec::with_capacity(decoded_rows.len());
    let mut previous_id = None;
    for (index, row) in decoded_rows.into_iter().enumerate() {
        if index % CANCEL_CHECK_INTERVAL == 0 && canceled.load(Ordering::SeqCst) {
            return Err(SimulationConfiguredInputRightsError::canceled());
        }
        let entitlement = decode_entitlement_record(row, config)?;
        if previous_id.is_some_and(|previous| entitlement.id <= previous) {
            return Err(SimulationConfiguredInputRightsError::primary(
                PrimaryFailure::PolicyRejected,
            ));
        }
        previous_id = Some(entitlement.id);
        entitlements.push(entitlement);
    }

    let mut covered_sessions = vec![false; dates.len()];
    for (row_index, entitlement) in entitlements.iter().enumerate() {
        let eligible = entitlement.status == EntitlementStatus::Active
            && entitlement.datasets.contains("krx_eod_bars")
            && entitlement.uses.contains("backtest");
        let mut covers_any = false;
        for (date_index, date) in dates.iter().enumerate() {
            if date_index % CANCEL_CHECK_INTERVAL == 0 && canceled.load(Ordering::SeqCst) {
                return Err(SimulationConfiguredInputRightsError::canceled());
            }
            let date = date.as_naive_date();
            let covers = entitlement.effective_from <= date && date <= entitlement.effective_until;
            covers_any |= covers;
            if eligible && covers {
                covered_sessions[date_index] = true;
            }
        }
        if covers_any != entitlement.covers_any_required {
            return Err(SimulationConfiguredInputRightsError::primary(
                PrimaryFailure::RawInvalid,
            ));
        }
        if row_index % CANCEL_CHECK_INTERVAL == 0 && canceled.load(Ordering::SeqCst) {
            return Err(SimulationConfiguredInputRightsError::canceled());
        }
    }
    if canceled.load(Ordering::SeqCst) {
        return Err(SimulationConfiguredInputRightsError::canceled());
    }
    if covered_sessions.iter().any(|covered| !covered) {
        return Err(SimulationConfiguredInputRightsError::primary(
            PrimaryFailure::PolicyRejected,
        ));
    }
    Ok(entitlements.into_iter().map(|row| row.id).collect())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

fn classify_command_result<T>(row: Option<T>) -> Vec<RawRow> {
    row.map(|_| RawRow::new(RawCategory::Command, Vec::new()))
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
        if !statement_plan_is_valid(&plan) {
            return Err(TransportFailure::State);
        }
        let query = match plan.binds.as_slice() {
            [] => sqlx::query(plan.sql),
            [BindValue::I64(value)] => sqlx::query(plan.sql).bind(*value),
            [BindValue::Uuid(value)] => sqlx::query(plan.sql).bind(*value),
            [
                BindValue::Text(reference),
                BindValue::Text(hash),
                BindValue::DateArray(dates),
            ] => sqlx::query(plan.sql)
                .bind(reference.clone())
                .bind(hash.clone())
                .bind(dates.clone()),
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
            plan.fetch_rows,
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
        if !statement_plan_is_valid(&plan) {
            return Err(TransportFailure::State);
        }
        match plan.kind {
            StatementKind::Configure
            | StatementKind::InitialContext
            | StatementKind::Owner
            | StatementKind::Entitlements
            | StatementKind::FinalContext => self.fetch_rows(plan).await,
            StatementKind::Begin | StatementKind::Commit | StatementKind::Rollback => {
                if !plan.binds.is_empty() || plan.category != Some(RawCategory::Command) {
                    return Err(TransportFailure::State);
                }
                let row = sqlx::query(plan.sql)
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

fn map_transport_failure(failure: TransportFailure, fallback: PrimaryFailure) -> PrimaryFailure {
    match failure {
        TransportFailure::Database | TransportFailure::State => fallback,
        TransportFailure::FetchBound => PrimaryFailure::BoundExceeded,
    }
}

fn is_canceled(canceled: &AtomicBool) -> bool {
    canceled.load(Ordering::SeqCst)
}

async fn execute_checked<T: ObserverTransport>(
    transport: &mut T,
    statement: Statement,
    canceled: &AtomicBool,
    budget: &mut LogicalPayloadBudget,
    fallback: PrimaryFailure,
) -> Result<Vec<RawRow>, SimulationConfiguredInputRightsError> {
    if is_canceled(canceled) {
        return Err(SimulationConfiguredInputRightsError::canceled());
    }
    match transport.execute(statement).await {
        Err(failure) => {
            let mut error = SimulationConfiguredInputRightsError::primary(map_transport_failure(
                failure, fallback,
            ));
            error.canceled = is_canceled(canceled);
            Err(error)
        }
        Ok(rows) if is_canceled(canceled) => {
            let mut error = match budget.charge_rows(&rows) {
                Ok(()) => SimulationConfiguredInputRightsError::canceled(),
                Err(error) => error,
            };
            error.canceled = true;
            Err(error)
        }
        Ok(rows) => Ok(rows),
    }
}

async fn prepare_observation<T: ObserverTransport>(
    transport: &mut T,
    config: &ValidatedConfig,
    input: &PreparedInput,
    canceled: &AtomicBool,
    budget: &mut LogicalPayloadBudget,
) -> Result<PreparedObservation, SimulationConfiguredInputRightsError> {
    let setup_rows = execute_checked(
        transport,
        Statement::Configure {
            timeout_ms: config.timeout_ms,
        },
        canceled,
        budget,
        PrimaryFailure::SetupFailed,
    )
    .await?;
    validate_setup(setup_rows, budget)?;

    let context_rows = execute_checked(
        transport,
        Statement::InitialContext,
        canceled,
        budget,
        PrimaryFailure::QueryUnavailable,
    )
    .await?;
    let context = validate_context(
        context_rows,
        RawCategory::InitialContext,
        config.timeout_ms,
        budget,
    )?;

    let owner_rows = execute_checked(
        transport,
        Statement::Owner {
            owner: config.owner,
        },
        canceled,
        budget,
        PrimaryFailure::QueryUnavailable,
    )
    .await?;
    validate_owner(owner_rows, config.owner, budget)?;

    let dates = input
        .dates
        .iter()
        .map(TradingDate::as_naive_date)
        .collect::<Vec<_>>();
    let entitlement_rows = execute_checked(
        transport,
        Statement::Entitlements {
            reference: config.entitlement_reference.clone(),
            bare_hash: config.bare_license_hash.clone(),
            dates,
        },
        canceled,
        budget,
        PrimaryFailure::QueryUnavailable,
    )
    .await?;
    let matching_license_row_ids =
        validate_entitlements(entitlement_rows, config, &input.dates, canceled, budget)?;

    let final_context_rows = execute_checked(
        transport,
        Statement::FinalContext,
        canceled,
        budget,
        PrimaryFailure::QueryUnavailable,
    )
    .await?;
    let final_context = validate_context(
        final_context_rows,
        RawCategory::FinalContext,
        config.timeout_ms,
        budget,
    )?;
    compare_contexts(&context, &final_context)?;
    Ok(PreparedObservation {
        context,
        matching_license_row_ids,
    })
}

async fn close_before_begin<T: ObserverTransport>(
    transport: &mut T,
    mut error: SimulationConfiguredInputRightsError,
    canceled: &AtomicBool,
) -> SimulationConfiguredInputRightsError {
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
    mut error: SimulationConfiguredInputRightsError,
    canceled: &AtomicBool,
    budget: &mut LogicalPayloadBudget,
) -> SimulationConfiguredInputRightsError {
    error.rollback = match transport.execute(Statement::Rollback).await {
        Ok(rows) if rows.is_empty() => CleanupState::Confirmed,
        Ok(rows) => {
            let _ = budget.charge_rows(&rows);
            CleanupState::Unconfirmed
        }
        Err(_) => CleanupState::Unconfirmed,
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
    config_view: ConfigView,
    input_view: ApprovedInputView<'_>,
    canceled: &AtomicBool,
) -> Result<SimulationConfiguredInputRightsObservation, SimulationConfiguredInputRightsError> {
    if is_canceled(canceled) {
        return Err(SimulationConfiguredInputRightsError::canceled());
    }
    let config = validate_config_view(config_view)?;
    let input = validate_input_view(input_view, canceled)?;
    let mut budget = LogicalPayloadBudget::default();
    budget.charge_date_input(&input.dates)?;
    if is_canceled(canceled) {
        return Err(SimulationConfiguredInputRightsError::canceled());
    }

    let acquired = transport.acquire().await;
    let canceled_during_acquire = is_canceled(canceled);
    if acquired.is_err() {
        let mut error =
            SimulationConfiguredInputRightsError::primary(PrimaryFailure::AcquisitionFailed);
        error.canceled = canceled_during_acquire;
        return Err(error);
    }

    // Arm guarded disposal synchronously before any later await or SQL.
    transport.arm_close_on_drop();
    if canceled_during_acquire {
        let mut error = SimulationConfiguredInputRightsError::canceled();
        return Err(close_before_begin(transport, error, canceled).await);
    }

    let begin_result = transport.execute(Statement::Begin).await;
    let canceled_during_begin = is_canceled(canceled);
    match begin_result {
        Ok(rows) if rows.is_empty() => {}
        Ok(rows) => {
            let _ = budget.charge_rows(&rows);
            let mut error =
                SimulationConfiguredInputRightsError::primary(PrimaryFailure::BeginResultInvalid);
            error.begin = BeginState::Uncertain;
            error.canceled = canceled_during_begin;
            return Err(rollback_and_close(transport, error, canceled, &mut budget).await);
        }
        Err(_) => {
            let mut error =
                SimulationConfiguredInputRightsError::primary(PrimaryFailure::BeginFailed);
            error.begin = BeginState::Uncertain;
            error.canceled = canceled_during_begin;
            return Err(rollback_and_close(transport, error, canceled, &mut budget).await);
        }
    }
    if canceled_during_begin {
        let mut error = SimulationConfiguredInputRightsError::canceled();
        error.begin = BeginState::Started;
        return Err(rollback_and_close(transport, error, canceled, &mut budget).await);
    }

    let prepared =
        match prepare_observation(transport, &config, &input, canceled, &mut budget).await {
            Ok(prepared) => prepared,
            Err(mut error) => {
                error.begin = BeginState::Started;
                error.canceled |= is_canceled(canceled);
                return Err(rollback_and_close(transport, error, canceled, &mut budget).await);
            }
        };
    if is_canceled(canceled) {
        let mut error = SimulationConfiguredInputRightsError::canceled();
        error.begin = BeginState::Started;
        return Err(rollback_and_close(transport, error, canceled, &mut budget).await);
    }

    let commit_result = transport.execute(Statement::Commit).await;
    let canceled_during_commit = is_canceled(canceled);
    let completion = match &commit_result {
        Ok(rows) if rows.is_empty() => CompletionState::Acknowledged,
        _ => CompletionState::Unconfirmed,
    };
    if completion == CompletionState::Unconfirmed {
        if let Ok(rows) = &commit_result {
            let _ = budget.charge_rows(rows);
        }
        let error = SimulationConfiguredInputRightsError {
            primary: PrimaryFailure::CompletionUnconfirmed,
            begin: BeginState::Started,
            completion,
            rollback: CleanupState::NotApplicable,
            close: CleanupState::NotApplicable,
            canceled: canceled_during_commit,
        };
        return Err(rollback_and_close(transport, error, canceled, &mut budget).await);
    }

    let close_result = transport.close().await;
    let canceled_after_close = is_canceled(canceled);
    if close_result.is_err() {
        return Err(SimulationConfiguredInputRightsError {
            primary: PrimaryFailure::CleanupUnconfirmed,
            begin: BeginState::Started,
            completion,
            rollback: CleanupState::NotApplicable,
            close: CleanupState::Unconfirmed,
            canceled: canceled_during_commit || canceled_after_close,
        });
    }
    if canceled_during_commit || canceled_after_close {
        return Err(SimulationConfiguredInputRightsError {
            primary: PrimaryFailure::Canceled,
            begin: BeginState::Started,
            completion,
            rollback: CleanupState::NotApplicable,
            close: CleanupState::Confirmed,
            canceled: true,
        });
    }
    Ok(SimulationConfiguredInputRightsObservation::completed(
        config,
        input,
        prepared,
        budget.used,
    ))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrimaryFailure {
    InvalidBound,
    InputInvalid,
    Canceled,
    AcquisitionFailed,
    BeginFailed,
    BeginResultInvalid,
    SetupFailed,
    SetupResultInvalid,
    QueryUnavailable,
    RawInvalid,
    PolicyRejected,
    BoundExceeded,
    PayloadExceeded,
    CompletionUnconfirmed,
    CleanupUnconfirmed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BeginState {
    NotAttempted,
    Uncertain,
    Started,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CompletionState {
    NotAttempted,
    Acknowledged,
    Unconfirmed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CleanupState {
    NotApplicable,
    Confirmed,
    Unconfirmed,
}

/// Value-free error axes. No SQLx diagnostic, owner, license, row, or database
/// value is retained or formatted.
pub(super) struct SimulationConfiguredInputRightsError {
    primary: PrimaryFailure,
    begin: BeginState,
    completion: CompletionState,
    rollback: CleanupState,
    close: CleanupState,
    canceled: bool,
}

impl SimulationConfiguredInputRightsError {
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

    fn canceled() -> Self {
        let mut error = Self::primary(PrimaryFailure::Canceled);
        error.canceled = true;
        error
    }
}

impl fmt::Debug for SimulationConfiguredInputRightsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SimulationConfiguredInputRightsError")
    }
}

impl fmt::Display for SimulationConfiguredInputRightsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("configured input and rights observation failed closed")
    }
}

impl std::error::Error for SimulationConfiguredInputRightsError {}

/// Observe checked configuration rights for the existing sealed V3 artifact.
/// This private SOURCE candidate makes no file, legal-grant, or parent-health
/// claim; reapproval of the artifact remains the child's responsibility.
pub(super) async fn observe_simulation_configured_input_rights(
    pool: &PgPool,
    config: &SimulationProcessConfig,
    approved: &ApprovedHistoricalPriceOnlyArtifact,
    canceled: &AtomicBool,
) -> Result<SimulationConfiguredInputRightsObservation, SimulationConfiguredInputRightsError> {
    if is_canceled(canceled) {
        return Err(SimulationConfiguredInputRightsError::canceled());
    }
    let config_view = ConfigView::from_checked(config);
    let input_view = ApprovedInputView::from_sealed(approved, canceled)?;
    let mut transport = SqlxObserverTransport::new(pool);
    observe_with_transport(&mut transport, config_view, input_view, canceled).await
}

#[cfg(test)]
mod tests;
