//! Stage4A normalization for bounded KIS daily-bar range captures.
//!
//! This module is deliberately an intermediate Raw contract.  It reads only
//! `provider=kis-daily-range` bars windows, validates the request metadata and
//! the `output2` historical rows again, then writes one deterministic,
//! bars-only batch per expected session under
//! `provider=kis-daily-range-normalized`.  It does not create a publication,
//! Curated, adjusted-price, action, calendar, or instrument-master record.
//!
//! The expected session list is supplied by a validated scheduler together
//! with its calendar identity and hash.  Weekdays are never treated as
//! sessions here.  The source is KIS's current vendor snapshot acquired at
//! `retrieved_at`; this intermediate contract is not a strict historical PIT
//! dataset because KIS supplies no availability, revision, or knowledge-time
//! evidence for the historical response.
//!
//! The approved calendar and fixed-listing bytes are embedded at compile time
//! rather than opened from an operator-selected path.  This makes a runtime
//! symlink, `..` path, or alternate artifact unable to change the scheduler
//! identity; the checked-in generator separately enforces regular-file and
//! path safety before those bytes enter a release.

use std::collections::{BTreeMap, BTreeSet};
use std::thread;
use std::time::Duration;

use domain::{BatchId, ContentHash, TradingDate, UtcTimestamp};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::contract::{
    FetchMode, MARKET_KR, PROVIDER_KIS_DAILY_RANGE, PROVIDER_KIS_DAILY_RANGE_NORMALIZED,
    RawEnvelope, RequestMetadata, ResponseKind, StoredFile,
};
use crate::providers::kis::KR_ETF_CORE_SYMBOLS;
use crate::storage::{BatchSpec, FileEntry, ManifestEntry, RawStore, StoreError};

/// Version of the Stage4A intermediate schema and mapping.
pub const RANGE_NORMALIZER: &str = "kis-daily-range-to-session-bars-v1";
pub const RANGE_NORMALIZER_SCHEMA_VERSION: u32 = 1;
pub const RANGE_NORMALIZED_SCHEMA_VERSION: u32 = 1;

const APPROVED_CALENDAR_ID: &str = "xkrx-historical-session-dates";
const APPROVED_LISTING_SNAPSHOT_ID: &str = "kr-etf-core-v1";
const APPROVED_LISTING_SNAPSHOT_HASH: &str =
    "sha256:267dc7aa065c6647ce634218fb8514fa49547a110ffc3d30f3bc00819ef7e992";
const APPROVED_EFFECTIVE_FROM: &str = "2020-01-31";
const APPROVED_CALENDAR_BYTES: &[u8] = include_bytes!("../../../data/calendars/xkrx/calendar.json");
const APPROVED_CALENDAR_MANIFEST_BYTES: &[u8] =
    include_bytes!("../../../data/calendars/xkrx/manifest.json");
const APPROVED_LISTING_BYTES: &[u8] =
    include_bytes!("../../../configs/universes/kr-etf-core-v1.yaml");

const DAILY_BARS_ENDPOINT: &str = "/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice";
const COLLISION_RETRIES: usize = 100;
const COLLISION_RETRY_DELAY: Duration = Duration::from_millis(2);

/// A scheduler-validated calendar/session selection for one captured range.
///
/// `sessions` is an exact, sorted, unique list.  It is not reconstructed from
/// weekdays and every date must fall inside `start..=end`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectedRangeSessions {
    pub calendar_id: String,
    pub calendar_hash: ContentHash,
    pub start: TradingDate,
    pub end: TradingDate,
    pub sessions: Vec<TradingDate>,
    pub listing_snapshot_id: String,
    pub listing_snapshot_hash: ContentHash,
}

impl ExpectedRangeSessions {
    pub fn new(
        calendar_id: impl Into<String>,
        calendar_hash: ContentHash,
        start: TradingDate,
        end: TradingDate,
        sessions: Vec<TradingDate>,
    ) -> Result<Self, RangeNormalizeError> {
        let expected = Self {
            calendar_id: calendar_id.into(),
            calendar_hash,
            start,
            end,
            sessions,
            listing_snapshot_id: APPROVED_LISTING_SNAPSHOT_ID.to_owned(),
            listing_snapshot_hash: approved_listing_snapshot_hash(),
        };
        expected.validate()?;
        Ok(expected)
    }

    pub fn validate(&self) -> Result<(), RangeNormalizeError> {
        if self.calendar_id.trim().is_empty() {
            return Err(RangeNormalizeError::InvalidExpectedSessions {
                reason: "calendar_id is empty".to_owned(),
            });
        }
        if self.calendar_id != APPROVED_CALENDAR_ID
            || self.calendar_hash != approved_calendar_hash()
        {
            return Err(RangeNormalizeError::InvalidExpectedSessions {
                reason: "calendar is not the approved embedded XKRX artifact".to_owned(),
            });
        }
        let listing_hash = approved_listing_snapshot_hash();
        let expected_listing_hash = ContentHash::parse(APPROVED_LISTING_SNAPSHOT_HASH)
            .expect("checked-in listing snapshot hash is valid");
        if listing_hash != expected_listing_hash
            || self.listing_snapshot_id != APPROVED_LISTING_SNAPSHOT_ID
            || self.listing_snapshot_hash != listing_hash
        {
            return Err(RangeNormalizeError::InvalidExpectedSessions {
                reason: "listing snapshot is not the approved fixed ETF snapshot".to_owned(),
            });
        }
        if self.end < self.start {
            return Err(RangeNormalizeError::InvalidExpectedSessions {
                reason: "calendar range end precedes start".to_owned(),
            });
        }
        let approved = parse_approved_calendar_artifact()?;
        if self.start < approved.range_start || self.end > approved.range_end {
            return Err(RangeNormalizeError::CalendarRangeOutOfBounds {
                start: self.start,
                end: self.end,
                supported_start: approved.range_start,
                supported_end: approved.range_end,
            });
        }
        let approved_selection = approved
            .sessions
            .into_iter()
            .filter(|date| *date >= self.start && *date <= self.end)
            .collect::<Vec<_>>();
        let approved_sessions = approved_selection.iter().copied().collect::<BTreeSet<_>>();
        let actual_sessions = self.sessions.iter().copied().collect::<BTreeSet<_>>();
        if actual_sessions != approved_sessions {
            return Err(RangeNormalizeError::InvalidExpectedSessions {
                reason: "session selection does not exactly match the approved XKRX range"
                    .to_owned(),
            });
        }
        let mut previous = None;
        for date in &self.sessions {
            if *date < self.start || *date > self.end {
                return Err(RangeNormalizeError::InvalidExpectedSessions {
                    reason: format!("session {date} is outside {}..={}", self.start, self.end),
                });
            }
            if !approved_sessions.contains(date) {
                return Err(RangeNormalizeError::InvalidExpectedSessions {
                    reason: format!("session {date} is not in the approved XKRX artifact"),
                });
            }
            if previous.is_some_and(|old| *date <= old) {
                return Err(RangeNormalizeError::InvalidExpectedSessions {
                    reason: "sessions must be strictly sorted and unique".to_owned(),
                });
            }
            previous = Some(*date);
        }
        Ok(())
    }

    /// Load the checked-in, package-free XKRX dates-only artifact and select
    /// the requested inclusive range.  The loader verifies the artifact and
    /// manifest hash/size, sorted/disjoint dates, full civil-date coverage,
    /// and the audit-only source schedule shape before returning any dates.
    pub fn approved_xkrx(
        start: TradingDate,
        end: TradingDate,
    ) -> Result<Self, RangeNormalizeError> {
        let artifact = parse_approved_calendar_artifact()?;
        if start < artifact.range_start || end > artifact.range_end || end < start {
            return Err(RangeNormalizeError::CalendarRangeOutOfBounds {
                start,
                end,
                supported_start: artifact.range_start,
                supported_end: artifact.range_end,
            });
        }
        if start < TradingDate::parse(APPROVED_EFFECTIVE_FROM).expect("approved date") {
            return Err(RangeNormalizeError::CalendarRangeOutOfBounds {
                start,
                end,
                supported_start: TradingDate::parse(APPROVED_EFFECTIVE_FROM)
                    .expect("approved date"),
                supported_end: artifact.range_end,
            });
        }
        let sessions = artifact
            .sessions
            .into_iter()
            .filter(|date| *date >= start && *date <= end)
            .collect();
        let expected = Self {
            calendar_id: artifact.calendar_id,
            calendar_hash: artifact.calendar_hash,
            start,
            end,
            sessions,
            listing_snapshot_id: APPROVED_LISTING_SNAPSHOT_ID.to_owned(),
            listing_snapshot_hash: approved_listing_snapshot_hash(),
        };
        expected.validate()?;
        Ok(expected)
    }
}

/// One source file retained in each normalized session's lineage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RangeNormalizationSourceFile {
    pub kind: ResponseKind,
    pub file_name: String,
    pub content_hash: ContentHash,
    pub size_bytes: u64,
    pub request: RequestMetadata,
}

/// Row-level source evidence retained for each normalized bar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RangeNormalizationSourceRow {
    pub source_file_name: String,
    pub source_file_hash: ContentHash,
    pub source_file_size_bytes: u64,
    pub source_query_start: TradingDate,
    pub source_query_end: TradingDate,
    pub symbol: String,
    pub row_date: TradingDate,
}

/// Exact upstream and calendar provenance carried in every Stage4A document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RangeNormalizationLineage {
    pub schema_version: u32,
    pub normalizer: String,
    pub upstream_provider: String,
    pub upstream_market: String,
    pub upstream_batch_id: BatchId,
    pub upstream_manifest_hash: ContentHash,
    pub source_start: TradingDate,
    pub source_end: TradingDate,
    pub source_files: Vec<RangeNormalizationSourceFile>,
    pub calendar_id: String,
    pub calendar_hash: ContentHash,
    pub listing_snapshot_id: String,
    pub listing_snapshot_hash: ContentHash,
    pub selected_session: TradingDate,
    pub source_rows: Vec<RangeNormalizationSourceRow>,
    pub acquired_at: UtcTimestamp,
    pub availability_evidence: bool,
    pub revision_evidence: bool,
    pub knowledge_time_evidence: bool,
}

/// One normalized immutable session batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeNormalizationOutcome {
    pub source_batch_id: BatchId,
    pub session_date: TradingDate,
    pub entry: ManifestEntry,
    pub files: Vec<StoredFile>,
    pub lineage: RangeNormalizationLineage,
}

/// Typed failures for the isolated Stage4A boundary.
#[derive(Debug, thiserror::Error)]
pub enum RangeNormalizeError {
    #[error(
        "range normalization supports only {expected_provider}/{expected_market}, got {provider}/{market}"
    )]
    UnsupportedScope {
        expected_provider: &'static str,
        expected_market: &'static str,
        provider: String,
        market: String,
    },
    #[error("range normalization requires credentialed Raw evidence")]
    UnsupportedMode,
    #[error("invalid expected calendar/session selection: {reason}")]
    InvalidExpectedSessions { reason: String },
    #[error("approved XKRX calendar artifact is invalid: {reason}")]
    CalendarArtifact { reason: String },
    #[error(
        "requested range {start}..={end} is outside approved calendar range {supported_start}..={supported_end}"
    )]
    CalendarRangeOutOfBounds {
        start: TradingDate,
        end: TradingDate,
        supported_start: TradingDate,
        supported_end: TradingDate,
    },
    #[error("source range manifest date {actual} does not equal expected start {expected}")]
    SourceDateMismatch {
        expected: TradingDate,
        actual: TradingDate,
    },
    #[error("source range manifest has no daily-bar files")]
    MissingSourceFiles,
    #[error("source range file {file_name} has unexpected response kind {kind}")]
    UnexpectedSourceKind {
        file_name: String,
        kind: ResponseKind,
    },
    #[error("source range file {file_name} has unexpected endpoint {endpoint}")]
    UnexpectedEndpoint { file_name: String, endpoint: String },
    #[error("source range file {file_name} has invalid query: {reason}")]
    InvalidQuery { file_name: String, reason: String },
    #[error("source range file {file_name} has invalid continuation metadata")]
    InvalidContinuation { file_name: String },
    #[error("source range file {file_name} is malformed: {reason}")]
    Malformed { file_name: String, reason: String },
    #[error("source range file {file_name} has an invalid {field}: {value}")]
    InvalidField {
        file_name: String,
        field: String,
        value: String,
    },
    #[error("source range file {file_name} contains date {date} outside its request")]
    DateOutOfQuery {
        file_name: String,
        date: TradingDate,
    },
    #[error("source range file {file_name} output2 is not newest-to-oldest")]
    ReversedOrder { file_name: String },
    #[error("source range contains duplicate {symbol} row for {date}")]
    DuplicateRow { symbol: String, date: TradingDate },
    #[error("source range contains conflicting {symbol} row for {date}")]
    ConflictingRow { symbol: String, date: TradingDate },
    #[error("source range file {file_name} size differs from its manifest metadata")]
    EvidenceSizeMismatch { file_name: String },
    #[error("source range contains date {date} that is absent from the validated session list")]
    OutOfSession { date: TradingDate },
    #[error(
        "validated session {date} does not contain exactly the fixed 11 ETF rows: missing={missing:?}, unexpected={unexpected:?}"
    )]
    SessionCoverage {
        date: TradingDate,
        missing: Vec<String>,
        unexpected: Vec<String>,
    },
    #[error("source Raw read failed: {0}")]
    Store(#[from] StoreError),
    #[error("normalized batch {batch_id} conflicts with existing immutable evidence: {reason}")]
    ExistingBatchConflict { batch_id: BatchId, reason: String },
    #[error("normalized batch serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Debug)]
struct ValidatedCalendarArtifact {
    calendar_id: String,
    calendar_hash: ContentHash,
    range_start: TradingDate,
    range_end: TradingDate,
    sessions: Vec<TradingDate>,
}

fn approved_listing_snapshot_hash() -> ContentHash {
    ContentHash::from_bytes(APPROVED_LISTING_BYTES)
}

fn approved_calendar_hash() -> ContentHash {
    ContentHash::from_bytes(APPROVED_CALENDAR_BYTES)
}

fn calendar_error(reason: impl Into<String>) -> RangeNormalizeError {
    RangeNormalizeError::CalendarArtifact {
        reason: reason.into(),
    }
}

fn parse_artifact_date(value: &Value, context: &str) -> Result<TradingDate, RangeNormalizeError> {
    let date = value
        .as_str()
        .ok_or_else(|| calendar_error(format!("{context} date is not a string")))?;
    TradingDate::parse(date)
        .map_err(|_| calendar_error(format!("{context} date is invalid: {date}")))
}

fn parse_date_array(
    root: &Map<String, Value>,
    field: &str,
) -> Result<Vec<TradingDate>, RangeNormalizeError> {
    root.get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| calendar_error(format!("{field} must be an array")))?
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let object = row
                .as_object()
                .ok_or_else(|| calendar_error(format!("{field}[{index}] is not an object")))?;
            parse_artifact_date(
                object
                    .get("date")
                    .ok_or_else(|| calendar_error(format!("{field}[{index}] lacks date")))?,
                &format!("{field}[{index}]"),
            )
        })
        .collect()
}

fn assert_sorted_unique(dates: &[TradingDate], field: &str) -> Result<(), RangeNormalizeError> {
    if dates.windows(2).any(|window| window[0] >= window[1]) {
        return Err(calendar_error(format!(
            "{field} is not strictly sorted/unique"
        )));
    }
    Ok(())
}

fn parse_approved_calendar_artifact() -> Result<ValidatedCalendarArtifact, RangeNormalizeError> {
    let actual_hash = approved_calendar_hash();
    let manifest: Value = serde_json::from_slice(APPROVED_CALENDAR_MANIFEST_BYTES)?;
    let manifest_object = manifest
        .as_object()
        .ok_or_else(|| calendar_error("calendar manifest is not an object"))?;
    if manifest_object.get("artifact").and_then(Value::as_str) != Some("calendar.json") {
        return Err(calendar_error("calendar manifest names the wrong artifact"));
    }
    let manifest_hash = manifest_object
        .get("artifact_sha256")
        .and_then(Value::as_str)
        .ok_or_else(|| calendar_error("calendar manifest lacks artifact_sha256"))?;
    if ContentHash::parse(manifest_hash).map_err(|_| calendar_error("invalid manifest hash"))?
        != actual_hash
    {
        return Err(calendar_error(
            "calendar artifact hash does not match manifest",
        ));
    }
    let manifest_size = manifest_object
        .get("artifact_size_bytes")
        .and_then(Value::as_u64)
        .ok_or_else(|| calendar_error("calendar manifest lacks artifact_size_bytes"))?;
    if manifest_size != APPROVED_CALENDAR_BYTES.len() as u64 {
        return Err(calendar_error(
            "calendar artifact size does not match manifest",
        ));
    }
    if manifest_object
        .get("manifest_schema_version")
        .and_then(Value::as_u64)
        != Some(1)
        || manifest_object.get("calendar_id").and_then(Value::as_str) != Some(APPROVED_CALENDAR_ID)
        || manifest_object.get("contract").and_then(Value::as_str)
            != Some("historical-session-dates-only")
        || manifest_object.get("exchange").and_then(Value::as_str) != Some("KRX")
    {
        return Err(calendar_error("calendar manifest identity is invalid"));
    }
    let artifact: Value = serde_json::from_slice(APPROVED_CALENDAR_BYTES)?;
    let root = artifact
        .as_object()
        .ok_or_else(|| calendar_error("calendar artifact is not an object"))?;
    if root.get("artifact_schema_version").and_then(Value::as_u64) != Some(2)
        || root.get("contract").and_then(Value::as_str) != Some("historical-session-dates-only")
        || root.get("representation").and_then(Value::as_str) != Some("dates-only")
        || root.get("exchange").and_then(Value::as_str) != Some("KRX")
        || root.get("source_authority").and_then(Value::as_str) != Some("third-party-derived")
        || root.get("source_schedule_purpose").and_then(Value::as_str)
            != Some("audit-only; not a publication or curation calendar")
    {
        return Err(calendar_error(
            "calendar artifact contract/authority fields are invalid",
        ));
    }
    let calendar_id = root
        .get("calendar_id")
        .and_then(Value::as_str)
        .ok_or_else(|| calendar_error("calendar artifact lacks calendar_id"))?;
    if calendar_id != APPROVED_CALENDAR_ID {
        return Err(calendar_error(
            "calendar artifact id is not the approved XKRX id",
        ));
    }
    let range = root
        .get("range")
        .and_then(Value::as_object)
        .ok_or_else(|| calendar_error("calendar artifact lacks range"))?;
    let range_start = TradingDate::parse(
        range
            .get("start")
            .and_then(Value::as_str)
            .ok_or_else(|| calendar_error("calendar range lacks start"))?,
    )
    .map_err(|_| calendar_error("calendar range start is invalid"))?;
    let range_end = TradingDate::parse(
        range
            .get("end")
            .and_then(Value::as_str)
            .ok_or_else(|| calendar_error("calendar range lacks end"))?,
    )
    .map_err(|_| calendar_error("calendar range end is invalid"))?;
    if range_end < range_start {
        return Err(calendar_error("calendar range end precedes start"));
    }
    let effective = root
        .get("effective_from")
        .and_then(Value::as_str)
        .ok_or_else(|| calendar_error("calendar artifact lacks effective_from"))?;
    if effective != APPROVED_EFFECTIVE_FROM || range_start.to_iso() != APPROVED_EFFECTIVE_FROM {
        return Err(calendar_error(
            "calendar effective range is not the approved fixed range",
        ));
    }
    let sessions = parse_date_array(root, "sessions")?;
    let non_sessions = parse_date_array(root, "non_sessions")?;
    assert_sorted_unique(&sessions, "sessions")?;
    assert_sorted_unique(&non_sessions, "non_sessions")?;
    let session_set = sessions.iter().copied().collect::<BTreeSet<_>>();
    let non_session_set = non_sessions.iter().copied().collect::<BTreeSet<_>>();
    if sessions
        .iter()
        .chain(non_sessions.iter())
        .any(|date| *date < range_start || *date > range_end)
    {
        return Err(calendar_error(
            "calendar date lies outside its declared range",
        ));
    }
    let manifest_range = manifest_object
        .get("range")
        .and_then(Value::as_object)
        .ok_or_else(|| calendar_error("calendar manifest lacks range"))?;
    let manifest_start = range_start.to_iso();
    let manifest_end = range_end.to_iso();
    if manifest_range.get("start").and_then(Value::as_str) != Some(manifest_start.as_str())
        || manifest_range.get("end").and_then(Value::as_str) != Some(manifest_end.as_str())
        || manifest_object.get("session_count").and_then(Value::as_u64)
            != Some(sessions.len() as u64)
        || manifest_object
            .get("non_session_count")
            .and_then(Value::as_u64)
            != Some(non_sessions.len() as u64)
    {
        return Err(calendar_error(
            "calendar manifest range/counts differ from artifact",
        ));
    }
    if session_set.intersection(&non_session_set).next().is_some() {
        return Err(calendar_error("sessions and non_sessions overlap"));
    }
    let mut cursor = range_start;
    loop {
        if !session_set.contains(&cursor) && !non_session_set.contains(&cursor) {
            return Err(calendar_error(format!(
                "civil date {cursor} is not covered"
            )));
        }
        if cursor == range_end {
            break;
        }
        cursor = cursor
            .checked_add_days(1)
            .map_err(|_| calendar_error("calendar civil range overflowed"))?;
    }
    let schedule = root
        .get("source_schedule")
        .and_then(Value::as_array)
        .ok_or_else(|| calendar_error("source_schedule must be an array"))?;
    if schedule.len() != sessions.len() {
        return Err(calendar_error(
            "source_schedule count differs from sessions",
        ));
    }
    for (index, row) in schedule.iter().enumerate() {
        let object = row
            .as_object()
            .ok_or_else(|| calendar_error(format!("source_schedule[{index}] is not an object")))?;
        let date = parse_artifact_date(
            object
                .get("date")
                .ok_or_else(|| calendar_error(format!("source_schedule[{index}] lacks date")))?,
            &format!("source_schedule[{index}]"),
        )?;
        if date != sessions[index] {
            return Err(calendar_error("source_schedule dates differ from sessions"));
        }
        for field in [
            "open_local",
            "open_utc",
            "close_local",
            "close_utc",
            "break_start_local",
            "break_start_utc",
            "break_end_local",
            "break_end_utc",
        ] {
            if !object.contains_key(field) {
                return Err(calendar_error(format!(
                    "source_schedule[{index}] lacks {field}"
                )));
            }
        }
    }
    let source_hash = root
        .get("source_hash")
        .and_then(Value::as_str)
        .ok_or_else(|| calendar_error("calendar artifact lacks source_hash"))?;
    ContentHash::parse(source_hash)
        .map_err(|_| calendar_error("calendar source_hash is invalid"))?;
    Ok(ValidatedCalendarArtifact {
        calendar_id: calendar_id.to_owned(),
        calendar_hash: actual_hash,
        range_start,
        range_end,
        sessions,
    })
}

/// Deterministic UUID-v5 identity for one source range and one expected
/// session.  The source manifest hash prevents a changed range manifest from
/// silently reusing an earlier session batch.
pub fn deterministic_range_normalized_batch_id(
    source: &ManifestEntry,
    source_manifest_hash: &ContentHash,
    session: TradingDate,
) -> BatchId {
    let name = format!(
        "provider={PROVIDER_KIS_DAILY_RANGE_NORMALIZED}\nnormalizer={RANGE_NORMALIZER}\nsource_batch={}\nsource_manifest_hash={}\nsession={session}",
        source.batch_id, source_manifest_hash
    );
    BatchId::from_uuid(Uuid::new_v5(&Uuid::NAMESPACE_URL, name.as_bytes()))
}

/// The production identity includes the approved calendar and listing
/// snapshots as well as the immutable source range. Changing any of those
/// inputs creates a new Raw revision instead of reusing a prior session batch.
pub fn deterministic_range_normalized_batch_id_with_identity(
    source: &ManifestEntry,
    source_manifest_hash: &ContentHash,
    session: TradingDate,
    calendar_hash: &ContentHash,
    listing_snapshot_hash: &ContentHash,
) -> BatchId {
    let name = format!(
        "provider={PROVIDER_KIS_DAILY_RANGE_NORMALIZED}\nnormalizer={RANGE_NORMALIZER}\nsource_batch={}\nsource_manifest_hash={}\ncalendar_hash={}\nlisting_snapshot_hash={}\nsession={session}",
        source.batch_id, source_manifest_hash, calendar_hash, listing_snapshot_hash
    );
    BatchId::from_uuid(Uuid::new_v5(&Uuid::NAMESPACE_URL, name.as_bytes()))
}

/// Normalize a bounded range into deterministic per-session batches.
pub fn normalize_kis_daily_range_batch(
    raw: &RawStore,
    source: &ManifestEntry,
    expected: &ExpectedRangeSessions,
) -> Result<Vec<RangeNormalizationOutcome>, RangeNormalizeError> {
    expected.validate()?;
    validate_source(source, expected)?;
    let stored = raw.read_batch_bytes(&source.provider, &source.market, source)?;
    let source_manifest_hash = source_manifest_hash(source)?;
    let source_files = source_lineage(source);
    let rows = parse_range_rows(source, &stored, expected)?;
    let mut outcomes = Vec::with_capacity(expected.sessions.len());
    for session in &expected.sessions {
        let session_rows = rows.get(session).cloned().unwrap_or_default();
        let expected_symbols = KR_ETF_CORE_SYMBOLS
            .iter()
            .map(|symbol| (*symbol).to_owned())
            .collect::<BTreeSet<_>>();
        let actual_symbols = session_rows.keys().cloned().collect::<BTreeSet<_>>();
        if actual_symbols != expected_symbols {
            return Err(RangeNormalizeError::SessionCoverage {
                date: *session,
                missing: expected_symbols
                    .difference(&actual_symbols)
                    .cloned()
                    .collect(),
                unexpected: actual_symbols
                    .difference(&expected_symbols)
                    .cloned()
                    .collect(),
            });
        }
        let lineage = RangeNormalizationLineage {
            schema_version: RANGE_NORMALIZER_SCHEMA_VERSION,
            normalizer: RANGE_NORMALIZER.to_owned(),
            upstream_provider: source.provider.clone(),
            upstream_market: source.market.clone(),
            upstream_batch_id: source.batch_id,
            upstream_manifest_hash: source_manifest_hash.clone(),
            source_start: expected.start,
            source_end: expected.end,
            source_files: source_files.clone(),
            calendar_id: expected.calendar_id.clone(),
            calendar_hash: expected.calendar_hash.clone(),
            listing_snapshot_id: expected.listing_snapshot_id.clone(),
            listing_snapshot_hash: expected.listing_snapshot_hash.clone(),
            selected_session: *session,
            source_rows: session_rows
                .values()
                .map(|bar| bar.source.clone())
                .collect(),
            acquired_at: source.retrieved_at,
            availability_evidence: false,
            revision_evidence: false,
            knowledge_time_evidence: false,
        };
        let envelope = session_envelope(*session, &session_rows, source, &lineage)?;
        let batch_id = deterministic_range_normalized_batch_id_with_identity(
            source,
            &source_manifest_hash,
            *session,
            &expected.calendar_hash,
            &expected.listing_snapshot_hash,
        );
        let envelope = RawEnvelope {
            batch_id,
            ..envelope
        };
        let spec = BatchSpec {
            provider: PROVIDER_KIS_DAILY_RANGE_NORMALIZED,
            market: &source.market,
            date: session,
            batch_id,
            entitlement_reference: source.entitlement_reference.as_deref(),
            mode: FetchMode::Credentialed,
        };
        let expected_entry = expected_entry(&spec, source, std::slice::from_ref(&envelope));
        let outcome = match load_existing(
            raw,
            &expected_entry,
            std::slice::from_ref(&envelope),
            lineage.clone(),
        )? {
            Some(outcome) => outcome,
            None => store_one(raw, &spec, &[envelope], &expected_entry, lineage.clone())?,
        };
        outcomes.push(outcome);
    }
    Ok(outcomes)
}

/// Alias with the shorter name used by scheduler integrations.
pub fn normalize_kis_daily_range(
    raw: &RawStore,
    source: &ManifestEntry,
    expected: &ExpectedRangeSessions,
) -> Result<Vec<RangeNormalizationOutcome>, RangeNormalizeError> {
    normalize_kis_daily_range_batch(raw, source, expected)
}

fn validate_source(
    source: &ManifestEntry,
    expected: &ExpectedRangeSessions,
) -> Result<(), RangeNormalizeError> {
    if source.provider != PROVIDER_KIS_DAILY_RANGE || source.market != MARKET_KR {
        return Err(RangeNormalizeError::UnsupportedScope {
            expected_provider: PROVIDER_KIS_DAILY_RANGE,
            expected_market: MARKET_KR,
            provider: source.provider.clone(),
            market: source.market.clone(),
        });
    }
    if source.mode != FetchMode::Credentialed {
        return Err(RangeNormalizeError::UnsupportedMode);
    }
    if source.date != expected.start {
        return Err(RangeNormalizeError::SourceDateMismatch {
            expected: expected.start,
            actual: source.date,
        });
    }
    if source.files.is_empty() {
        return Err(RangeNormalizeError::MissingSourceFiles);
    }
    let mut names = BTreeSet::new();
    for file in &source.files {
        if !names.insert(file.file_name.as_str()) {
            return Err(RangeNormalizeError::Malformed {
                file_name: file.file_name.clone(),
                reason: "duplicate file name in range manifest".to_owned(),
            });
        }
        if file.kind != ResponseKind::Bars {
            return Err(RangeNormalizeError::UnexpectedSourceKind {
                file_name: file.file_name.clone(),
                kind: file.kind,
            });
        }
    }
    Ok(())
}

fn source_manifest_hash(source: &ManifestEntry) -> Result<ContentHash, RangeNormalizeError> {
    Ok(ContentHash::from_bytes(&serde_json::to_vec(source)?))
}

fn source_lineage(source: &ManifestEntry) -> Vec<RangeNormalizationSourceFile> {
    let mut files = source
        .files
        .iter()
        .map(|file| RangeNormalizationSourceFile {
            kind: file.kind,
            file_name: file.file_name.clone(),
            content_hash: file.content_hash.clone(),
            size_bytes: file.size_bytes,
            request: file.request.clone(),
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.file_name.cmp(&right.file_name));
    files
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedRangeBar {
    value: Map<String, Value>,
    source: RangeNormalizationSourceRow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RangeWindowSummary {
    query_start: TradingDate,
    query_end: TradingDate,
    oldest_date: TradingDate,
    file_name: String,
}

type SessionRows = BTreeMap<TradingDate, BTreeMap<String, ParsedRangeBar>>;

fn parse_range_rows(
    source: &ManifestEntry,
    stored: &[StoredFile],
    expected: &ExpectedRangeSessions,
) -> Result<SessionRows, RangeNormalizeError> {
    let expected_sessions = expected.sessions.iter().copied().collect::<BTreeSet<_>>();
    let mut rows: SessionRows = BTreeMap::new();
    let mut windows = BTreeMap::<String, Vec<RangeWindowSummary>>::new();
    // Manifest order is the provider's response order.  Do not sort these
    // files: per-symbol window continuity is defined by the response order.
    for metadata in &source.files {
        let file = stored
            .iter()
            .find(|stored| stored.file_name == metadata.file_name)
            .ok_or_else(|| RangeNormalizeError::Malformed {
                file_name: metadata.file_name.clone(),
                reason: "manifest evidence was not read back".to_owned(),
            })?;
        if file.bytes.len() as u64 != metadata.size_bytes {
            return Err(RangeNormalizeError::EvidenceSizeMismatch {
                file_name: metadata.file_name.clone(),
            });
        }
        let (symbol, query_start, query_end) = validate_file_request(metadata, expected)?;
        let document: Value = serde_json::from_slice(&file.bytes).map_err(|error| {
            RangeNormalizeError::Malformed {
                file_name: metadata.file_name.clone(),
                reason: format!("invalid JSON: {error}"),
            }
        })?;
        let object = document
            .as_object()
            .ok_or_else(|| RangeNormalizeError::Malformed {
                file_name: metadata.file_name.clone(),
                reason: "response must be a JSON object".to_owned(),
            })?;
        if object.get("rt_cd").and_then(Value::as_str) != Some("0") {
            return Err(RangeNormalizeError::InvalidField {
                file_name: metadata.file_name.clone(),
                field: "rt_cd".to_owned(),
                value: object
                    .get("rt_cd")
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "<missing>".to_owned()),
            });
        }
        let output = object
            .get("output2")
            .and_then(Value::as_array)
            .ok_or_else(|| RangeNormalizeError::Malformed {
                file_name: metadata.file_name.clone(),
                reason: "output2 must be an array".to_owned(),
            })?;
        if output.is_empty() {
            return Err(RangeNormalizeError::Malformed {
                file_name: metadata.file_name.clone(),
                reason: "output2 must not be empty for a selected range".to_owned(),
            });
        }
        let mut previous = None;
        for row in output {
            let row = row
                .as_object()
                .ok_or_else(|| RangeNormalizeError::Malformed {
                    file_name: metadata.file_name.clone(),
                    reason: "output2 row must be an object".to_owned(),
                })?;
            let date = parse_kis_date(
                &metadata.file_name,
                required_string(row, &metadata.file_name, "stck_bsop_date")?,
            )?;
            if previous.is_some_and(|old| date >= old) {
                return Err(RangeNormalizeError::ReversedOrder {
                    file_name: metadata.file_name.clone(),
                });
            }
            previous = Some(date);
            if date < query_start || date > query_end {
                return Err(RangeNormalizeError::DateOutOfQuery {
                    file_name: metadata.file_name.clone(),
                    date,
                });
            }
            if !expected_sessions.contains(&date) {
                return Err(RangeNormalizeError::OutOfSession { date });
            }
            validate_ohlcv_fields(row, &metadata.file_name)?;
            let canonical = canonical_bar(row, &metadata.file_name, &symbol, date);
            let parsed = ParsedRangeBar {
                value: canonical,
                source: RangeNormalizationSourceRow {
                    source_file_name: metadata.file_name.clone(),
                    source_file_hash: metadata.content_hash.clone(),
                    source_file_size_bytes: metadata.size_bytes,
                    source_query_start: query_start,
                    source_query_end: query_end,
                    symbol: symbol.clone(),
                    row_date: date,
                },
            };
            if let Some(previous) = rows
                .entry(date)
                .or_default()
                .insert(symbol.clone(), parsed.clone())
            {
                if previous.value == parsed.value {
                    return Err(RangeNormalizeError::DuplicateRow { symbol, date });
                }
                return Err(RangeNormalizeError::ConflictingRow { symbol, date });
            }
        }
        let oldest_date = previous.expect("non-empty output2 was checked above");
        windows.entry(symbol).or_default().push(RangeWindowSummary {
            query_start,
            query_end,
            oldest_date,
            file_name: metadata.file_name.clone(),
        });
    }
    for (symbol, bounds) in windows {
        if bounds.first().map(|item| item.query_end) != Some(expected.end) {
            return Err(RangeNormalizeError::InvalidQuery {
                file_name: bounds
                    .first()
                    .map(|item| item.file_name.clone())
                    .unwrap_or(symbol),
                reason: "first daily-range window must end at the requested range end".to_owned(),
            });
        }
        for pair in bounds.windows(2) {
            let expected_next_end = pair[0].oldest_date.previous_day();
            if pair[0].query_start != expected.start
                || pair[1].query_start != expected.start
                || pair[1].query_end != expected_next_end
            {
                return Err(RangeNormalizeError::InvalidQuery {
                    file_name: pair[1].file_name.clone(),
                    reason: format!(
                        "window end must equal previous response oldest date minus one civil day (expected {}, got {})",
                        expected_next_end, pair[1].query_end
                    ),
                });
            }
        }
    }
    Ok(rows)
}

fn validate_file_request(
    metadata: &FileEntry,
    expected: &ExpectedRangeSessions,
) -> Result<(String, TradingDate, TradingDate), RangeNormalizeError> {
    if metadata.request.mode != FetchMode::Credentialed {
        return Err(RangeNormalizeError::InvalidQuery {
            file_name: metadata.file_name.clone(),
            reason: "range source request is not credentialed".to_owned(),
        });
    }
    if metadata.request.endpoint != DAILY_BARS_ENDPOINT {
        return Err(RangeNormalizeError::UnexpectedEndpoint {
            file_name: metadata.file_name.clone(),
            endpoint: metadata.request.endpoint.clone(),
        });
    }
    let mut query = BTreeMap::<&str, &str>::new();
    for (key, value) in &metadata.request.query {
        if query.insert(key.as_str(), value.as_str()).is_some() {
            return Err(RangeNormalizeError::InvalidQuery {
                file_name: metadata.file_name.clone(),
                reason: format!("duplicate query key {key}"),
            });
        }
    }
    let required = [
        "FID_COND_MRKT_DIV_CODE",
        "FID_INPUT_ISCD",
        "FID_INPUT_DATE_1",
        "FID_INPUT_DATE_2",
        "FID_PERIOD_DIV_CODE",
        "FID_ORG_ADJ_PRC",
    ];
    if query.len() != required.len() || required.iter().any(|key| !query.contains_key(key)) {
        return Err(RangeNormalizeError::InvalidQuery {
            file_name: metadata.file_name.clone(),
            reason: "query keys must be the six documented daily-range fields".to_owned(),
        });
    }
    if query["FID_COND_MRKT_DIV_CODE"] != "J"
        || query["FID_PERIOD_DIV_CODE"] != "D"
        || query["FID_ORG_ADJ_PRC"] != "1"
    {
        return Err(RangeNormalizeError::InvalidQuery {
            file_name: metadata.file_name.clone(),
            reason: "requires J/D/original-price=1 request".to_owned(),
        });
    }
    let symbol = query["FID_INPUT_ISCD"];
    if !KR_ETF_CORE_SYMBOLS.contains(&symbol) {
        return Err(RangeNormalizeError::InvalidQuery {
            file_name: metadata.file_name.clone(),
            reason: format!("symbol {symbol} is outside the fixed 11 ETF universe"),
        });
    }
    let query_start = parse_query_date(&metadata.file_name, query["FID_INPUT_DATE_1"])?;
    let query_end = parse_query_date(&metadata.file_name, query["FID_INPUT_DATE_2"])?;
    if query_start != expected.start || query_end < query_start || query_end > expected.end {
        return Err(RangeNormalizeError::InvalidQuery {
            file_name: metadata.file_name.clone(),
            reason: format!(
                "query bounds must be {}..={} within source range",
                expected.start, expected.end
            ),
        });
    }
    let continuation = metadata
        .request
        .headers
        .iter()
        .filter(|(key, _)| key == "tr_cont")
        .collect::<Vec<_>>();
    if continuation.len() != 1 || !continuation[0].1.is_empty() {
        return Err(RangeNormalizeError::InvalidContinuation {
            file_name: metadata.file_name.clone(),
        });
    }
    Ok((symbol.to_owned(), query_start, query_end))
}

fn parse_query_date(file_name: &str, value: &str) -> Result<TradingDate, RangeNormalizeError> {
    if value.len() != 8 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(RangeNormalizeError::InvalidField {
            file_name: file_name.to_owned(),
            field: "query date".to_owned(),
            value: value.to_owned(),
        });
    }
    TradingDate::parse(&format!("{}-{}-{}", &value[..4], &value[4..6], &value[6..])).map_err(|_| {
        RangeNormalizeError::InvalidField {
            file_name: file_name.to_owned(),
            field: "query date".to_owned(),
            value: value.to_owned(),
        }
    })
}

fn parse_kis_date(file_name: &str, value: &str) -> Result<TradingDate, RangeNormalizeError> {
    if value.len() != 8 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(RangeNormalizeError::InvalidField {
            file_name: file_name.to_owned(),
            field: "stck_bsop_date".to_owned(),
            value: value.to_owned(),
        });
    }
    parse_query_date(file_name, value)
}

fn required_string<'a>(
    row: &'a Map<String, Value>,
    file_name: &str,
    field: &str,
) -> Result<&'a str, RangeNormalizeError> {
    row.get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| RangeNormalizeError::InvalidField {
            file_name: file_name.to_owned(),
            field: field.to_owned(),
            value: "<missing-or-empty>".to_owned(),
        })
}

fn validate_ohlcv_fields(
    row: &Map<String, Value>,
    file_name: &str,
) -> Result<(), RangeNormalizeError> {
    for field in [
        "stck_oprc",
        "stck_hgpr",
        "stck_lwpr",
        "stck_clpr",
        "acml_vol",
    ] {
        let value = row
            .get(field)
            .ok_or_else(|| RangeNormalizeError::InvalidField {
                file_name: file_name.to_owned(),
                field: field.to_owned(),
                value: "<missing>".to_owned(),
            })?;
        match value {
            Value::String(value) if !value.trim().is_empty() => {}
            Value::Number(_) => {}
            _ => {
                return Err(RangeNormalizeError::InvalidField {
                    file_name: file_name.to_owned(),
                    field: field.to_owned(),
                    value: value.to_string(),
                });
            }
        }
    }
    if let Some(value) = row.get("acml_tr_pbmn") {
        match value {
            Value::String(value) if !value.trim().is_empty() => {}
            Value::Number(_) => {}
            _ => {
                return Err(RangeNormalizeError::InvalidField {
                    file_name: file_name.to_owned(),
                    field: "acml_tr_pbmn".to_owned(),
                    value: value.to_string(),
                });
            }
        }
    }
    Ok(())
}

fn canonical_bar(
    row: &Map<String, Value>,
    file_name: &str,
    symbol: &str,
    date: TradingDate,
) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert(
        "instrument".to_owned(),
        Value::String(format!("{symbol}.KRX")),
    );
    out.insert("date".to_owned(), Value::String(date.to_iso()));
    for (output, input) in [
        ("open", "stck_oprc"),
        ("high", "stck_hgpr"),
        ("low", "stck_lwpr"),
        ("close", "stck_clpr"),
        ("volume", "acml_vol"),
    ] {
        out.insert(
            output.to_owned(),
            row.get(input)
                .cloned()
                .unwrap_or_else(|| panic!("validated field {input} missing from {file_name}")),
        );
    }
    if let Some(value) = row.get("acml_tr_pbmn") {
        out.insert("value".to_owned(), value.clone());
    }
    out
}

fn session_envelope(
    session: TradingDate,
    rows: &BTreeMap<String, ParsedRangeBar>,
    source: &ManifestEntry,
    lineage: &RangeNormalizationLineage,
) -> Result<RawEnvelope, RangeNormalizeError> {
    let bars = rows
        .values()
        .map(|row| Value::Object(row.value.clone()))
        .collect::<Vec<_>>();
    let document = json!({
        "schema_version": RANGE_NORMALIZED_SCHEMA_VERSION,
        "dataset_kind": "kis-daily-range-bars",
        "date": session,
        "bars": bars,
        "acquired_at": source.retrieved_at,
        "pit": {
            "mode": "acquisition-time-vendor-snapshot",
            "strict": false,
            "availability_evidence": false,
            "revision_evidence": false,
            "knowledge_time_evidence": false
        },
        "_lineage": lineage
    });
    Ok(RawEnvelope::new(
        BatchId::generate(),
        ResponseKind::Bars,
        format!("bars-{session}.json"),
        serde_json::to_vec(&document)?,
        source.retrieved_at,
        RequestMetadata {
            endpoint: format!("kis.range.normalized/{RANGE_NORMALIZER}/bars"),
            query: vec![
                ("source_batch_id".to_owned(), source.batch_id.to_string()),
                (
                    "source_manifest_hash".to_owned(),
                    lineage.upstream_manifest_hash.to_string(),
                ),
                ("session_date".to_owned(), session.to_iso()),
            ],
            headers: Vec::new(),
            mode: FetchMode::Credentialed,
        },
    ))
}

fn expected_entry(
    spec: &BatchSpec<'_>,
    source: &ManifestEntry,
    envelopes: &[RawEnvelope],
) -> ManifestEntry {
    ManifestEntry {
        batch_id: spec.batch_id,
        provider: spec.provider.to_owned(),
        market: spec.market.to_owned(),
        date: *spec.date,
        retrieved_at: source.retrieved_at,
        mode: spec.mode,
        entitlement_reference: spec.entitlement_reference.map(str::to_owned),
        files: envelopes
            .iter()
            .map(|envelope| FileEntry {
                kind: envelope.kind,
                file_name: envelope.file_name.clone(),
                content_hash: envelope.content_hash.clone(),
                size_bytes: envelope.bytes.len() as u64,
                request: envelope.request.clone(),
            })
            .collect(),
    }
}

fn load_existing(
    raw: &RawStore,
    expected: &ManifestEntry,
    envelopes: &[RawEnvelope],
    lineage: RangeNormalizationLineage,
) -> Result<Option<RangeNormalizationOutcome>, RangeNormalizeError> {
    let existing = raw
        .read_reconciled_manifest(PROVIDER_KIS_DAILY_RANGE_NORMALIZED, MARKET_KR)?
        .into_iter()
        .find(|entry| entry.batch_id == expected.batch_id);
    let Some(entry) = existing else {
        return Ok(None);
    };
    if &entry != expected {
        return Err(RangeNormalizeError::ExistingBatchConflict {
            batch_id: entry.batch_id,
            reason: "manifest metadata or content hash differs".to_owned(),
        });
    }
    let files = raw.read_batch_bytes(PROVIDER_KIS_DAILY_RANGE_NORMALIZED, MARKET_KR, &entry)?;
    compare_evidence(entry.batch_id, &files, envelopes)?;
    Ok(Some(RangeNormalizationOutcome {
        source_batch_id: lineage.upstream_batch_id,
        session_date: lineage.selected_session,
        entry,
        files,
        lineage,
    }))
}

fn store_one(
    raw: &RawStore,
    spec: &BatchSpec<'_>,
    envelopes: &[RawEnvelope],
    expected: &ManifestEntry,
    lineage: RangeNormalizationLineage,
) -> Result<RangeNormalizationOutcome, RangeNormalizeError> {
    match raw.store_batch(spec, envelopes) {
        Ok(entry) => finish_stored(raw, entry, envelopes, expected, lineage),
        Err(StoreError::FileExists { .. }) => {
            for _ in 0..COLLISION_RETRIES {
                if let Some(outcome) = load_existing(raw, expected, envelopes, lineage.clone())? {
                    return Ok(outcome);
                }
                thread::sleep(COLLISION_RETRY_DELAY);
            }
            Err(RangeNormalizeError::Store(StoreError::FileExists {
                path: raw
                    .batch_dir(spec.provider, spec.market, spec.date, &spec.batch_id)
                    .display()
                    .to_string(),
            }))
        }
        Err(error) => Err(RangeNormalizeError::Store(error)),
    }
}

fn finish_stored(
    raw: &RawStore,
    entry: ManifestEntry,
    envelopes: &[RawEnvelope],
    expected: &ManifestEntry,
    lineage: RangeNormalizationLineage,
) -> Result<RangeNormalizationOutcome, RangeNormalizeError> {
    if &entry != expected {
        return Err(RangeNormalizeError::ExistingBatchConflict {
            batch_id: entry.batch_id,
            reason: "RawStore returned metadata different from deterministic contract".to_owned(),
        });
    }
    let files = raw.read_batch_bytes(PROVIDER_KIS_DAILY_RANGE_NORMALIZED, MARKET_KR, &entry)?;
    compare_evidence(entry.batch_id, &files, envelopes)?;
    Ok(RangeNormalizationOutcome {
        source_batch_id: lineage.upstream_batch_id,
        session_date: lineage.selected_session,
        entry,
        files,
        lineage,
    })
}

fn compare_evidence(
    batch_id: BatchId,
    files: &[StoredFile],
    envelopes: &[RawEnvelope],
) -> Result<(), RangeNormalizeError> {
    if files.len() != envelopes.len() {
        return Err(RangeNormalizeError::ExistingBatchConflict {
            batch_id,
            reason: "stored evidence count differs".to_owned(),
        });
    }
    for envelope in envelopes {
        let Some(file) = files
            .iter()
            .find(|file| file.file_name == envelope.file_name)
        else {
            return Err(RangeNormalizeError::ExistingBatchConflict {
                batch_id,
                reason: format!("stored evidence {} is missing", envelope.file_name),
            });
        };
        if file.bytes != envelope.bytes {
            return Err(RangeNormalizeError::ExistingBatchConflict {
                batch_id,
                reason: format!("stored evidence {} bytes differ", envelope.file_name),
            });
        }
    }
    Ok(())
}
