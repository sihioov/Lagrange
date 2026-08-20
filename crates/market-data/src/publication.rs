//! Typed, transport-neutral publication records derived from immutable Raw batches.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{FixedOffset, TimeZone, Utc};
use domain::{BatchId, TradingDate, UtcTimestamp};
use serde::Deserialize;
use serde_json::Value;

use crate::contract::{
    FetchMode, MARKET_KR, PROVIDER_KIS, PROVIDER_KIS_NORMALIZED, PROVIDER_KRX, ResponseKind,
    StoredFile,
};
use crate::normalize::{NormalizationLineage, deterministic_kis_normalized_batch_id};
use crate::storage::{FileEntry, ManifestEntry, RawStore, StoreError};
use crate::validate::validate_response;

/// Stable batch kind stored by downstream publication sinks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataBatchKind {
    Eod,
    EodUnavailable,
    Reference,
    Calendar,
    CorporateActions,
    InvestorFlow,
    MarketStatus,
    Fundamentals,
    IndexMembership,
    SectorClassification,
}

impl DataBatchKind {
    pub const fn as_db_str(self) -> &'static str {
        match self {
            Self::Eod => "EOD",
            Self::EodUnavailable => "EOD_UNAVAILABLE",
            Self::Reference => "REFERENCE",
            Self::Calendar => "CALENDAR",
            Self::CorporateActions => "CORPORATE_ACTIONS",
            Self::InvestorFlow => "INVESTOR_FLOW",
            Self::MarketStatus => "MARKET_STATUS",
            Self::Fundamentals => "FUNDAMENTALS",
            Self::IndexMembership => "INDEX_MEMBERSHIP",
            Self::SectorClassification => "SECTOR_CLASSIFICATION",
        }
    }
}

/// Stable session type stored by downstream publication sinks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CalendarSessionType {
    Trading,
    Closed,
}

impl CalendarSessionType {
    pub const fn as_db_str(self) -> &'static str {
        match self {
            Self::Trading => "TRADING",
            Self::Closed => "CLOSED",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationBundle {
    pub source_batch_id: BatchId,
    pub provider: String,
    pub market: String,
    pub target_date: TradingDate,
    pub retrieved_at: UtcTimestamp,
    pub fetch_mode: FetchMode,
    pub files: Vec<PublicationFile>,
    pub calendar_facts: Vec<CalendarFact>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationFile {
    pub file_name: String,
    pub kind: DataBatchKind,
    pub content_sha256: String,
    pub storage_path: String,
    pub bytes_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarFact {
    pub exchange: String,
    pub session_date: TradingDate,
    pub session_type: CalendarSessionType,
    pub timezone: String,
    pub source: String,
    pub source_version: String,
    pub content_sha256: String,
}

#[derive(Debug, thiserror::Error)]
pub enum PublicationError {
    #[error("verified Raw read failed: {0}")]
    Store(#[from] StoreError),
    #[error("publication supports only {expected_scopes}, got {provider}/{market}")]
    UnsupportedManifestScope {
        expected_scopes: &'static str,
        provider: String,
        market: String,
    },
    #[error("publication scope {provider}/{market} requires {expected} fetch mode, got {actual}")]
    UnsupportedManifestMode {
        provider: String,
        market: String,
        expected: FetchMode,
        actual: FetchMode,
    },
    #[error("noncanonical normalized publication manifest: {reason}")]
    NonCanonicalNormalizedManifest { reason: String },
    #[error("canonical {kind} file {file_name} failed validation: {reason}")]
    InvalidCanonicalFile {
        kind: ResponseKind,
        file_name: String,
        reason: String,
    },
    #[error("canonical provenance invalid for {file_name}: {reason}")]
    InvalidCanonicalProvenance { file_name: String, reason: String },
    #[error("manifest size mismatch for {file_name}: manifest={manifest_size}, read={read_size}")]
    SizeMismatch {
        file_name: String,
        manifest_size: u64,
        read_size: u64,
    },
    #[error("file {file_name} size {size} exceeds PostgreSQL bigint")]
    SizeExceedsPostgresBigint { file_name: String, size: u64 },
    #[error("unexpected content hash for {file_name}: {value}")]
    UnexpectedContentHash { file_name: String, value: String },
    #[error("non-UTF8 storage path for {file_name}")]
    NonUtf8StoragePath { file_name: String },
    #[error(
        "verified Raw file count differs from manifest: manifest={manifest_count}, read={read_count}"
    )]
    ReadbackFileCountMismatch {
        manifest_count: usize,
        read_count: usize,
    },
    #[error(
        "verified Raw file name differs from manifest: manifest={manifest_file_name}, read={read_file_name}"
    )]
    ReadbackFileNameMismatch {
        manifest_file_name: String,
        read_file_name: String,
    },
    #[error("malformed bars file {file_name}: {reason}")]
    MalformedBars { file_name: String, reason: String },
    #[error("invalid bar date in {file_name}: {value}")]
    InvalidBarDate { file_name: String, value: String },
    #[error("malformed calendar file {file_name}: {reason}")]
    MalformedCalendar { file_name: String, reason: String },
    #[error("calendar {file_name} has unsupported timezone {timezone}")]
    UnsupportedCalendarTimezone { file_name: String, timezone: String },
    #[error("calendar {file_name} declares unexpected session times {open}/{close}")]
    InvalidCalendarSessionTimes {
        file_name: String,
        open: String,
        close: String,
    },
    #[error("invalid calendar date in {file_name}: {value}")]
    InvalidCalendarDate { file_name: String, value: String },
    #[error("invalid calendar {field} timestamp in {file_name}: {value}")]
    InvalidCalendarTimestamp {
        file_name: String,
        field: String,
        value: String,
    },
    #[error("calendar UTC instant is inconsistent in {file_name} for {date} {field}: {value}")]
    InconsistentCalendarInstant {
        file_name: String,
        date: String,
        field: String,
        value: String,
    },
    #[error("calendar contains a session and holiday for {date} in {file_name}")]
    CalendarDateBothSessionAndHoliday { file_name: String, date: String },
    #[error("conflicting calendar facts for {exchange} {date} from {source_version}")]
    ConflictingCalendarFact {
        exchange: String,
        date: TradingDate,
        source_version: String,
    },
    #[error("conflicting calendar provenance for source version {source_version}")]
    ConflictingCalendarProvenance { source_version: String },
}

#[derive(Deserialize)]
struct BarsDoc {
    bars: Vec<BarsRow>,
}

#[derive(Deserialize)]
struct BarsRow {
    date: String,
}

#[derive(Deserialize)]
struct CalendarDoc {
    calendar_id: String,
    schema_version: i64,
    source: String,
    timezone: String,
    session_times_local: CalendarSessionTimes,
    sessions: Vec<CalendarSession>,
    holidays: Vec<CalendarHoliday>,
}

#[derive(Deserialize)]
struct CalendarSessionTimes {
    open: String,
    close: String,
}

#[derive(Deserialize)]
struct CalendarSession {
    date: String,
    open_utc: String,
    close_utc: String,
}

#[derive(Deserialize)]
struct CalendarHoliday {
    date: String,
}

struct VerifiedRawFile<'a> {
    entry: &'a FileEntry,
    bytes: &'a [u8],
    content_sha256: String,
    storage_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CalendarProvenance {
    source: String,
    timezone: String,
    content_sha256: String,
}

struct ParsedCalendar {
    source_version: String,
    provenance: CalendarProvenance,
    facts: Vec<CalendarFact>,
}

impl PublicationBundle {
    /// Reads every file back through Raw verification before producing any facts.
    pub fn from_raw(store: &RawStore, manifest: &ManifestEntry) -> Result<Self, PublicationError> {
        let normalized = match (manifest.provider.as_str(), manifest.market.as_str()) {
            (PROVIDER_KRX, MARKET_KR) => false,
            (PROVIDER_KIS_NORMALIZED, MARKET_KR) => {
                if manifest.mode != FetchMode::Credentialed {
                    return Err(PublicationError::UnsupportedManifestMode {
                        provider: manifest.provider.clone(),
                        market: manifest.market.clone(),
                        expected: FetchMode::Credentialed,
                        actual: manifest.mode,
                    });
                }
                validate_normalized_manifest(manifest)?;
                true
            }
            _ => {
                return Err(PublicationError::UnsupportedManifestScope {
                    expected_scopes: "krx/kr or kis-normalized/kr",
                    provider: manifest.provider.clone(),
                    market: manifest.market.clone(),
                });
            }
        };
        let stored = store.read_batch_bytes(&manifest.provider, &manifest.market, manifest)?;
        if stored.len() != manifest.files.len() {
            return Err(PublicationError::ReadbackFileCountMismatch {
                manifest_count: manifest.files.len(),
                read_count: stored.len(),
            });
        }

        let verified_files: Vec<_> = manifest
            .files
            .iter()
            .zip(&stored)
            .map(|(entry, stored_file)| validate_file_metadata(entry, stored_file))
            .collect::<Result<_, _>>()?;
        if normalized {
            validate_normalized_provenance(manifest, &verified_files)?;
        }

        let mut files = Vec::with_capacity(verified_files.len());
        let mut calendar_facts = BTreeMap::new();
        let mut calendar_provenance = BTreeMap::new();
        for verified in verified_files {
            if normalized {
                validate_response(verified.entry.kind, verified.bytes).map_err(|error| {
                    PublicationError::InvalidCanonicalFile {
                        kind: verified.entry.kind,
                        file_name: verified.entry.file_name.clone(),
                        reason: error.to_string(),
                    }
                })?;
            }
            let kind = match verified.entry.kind {
                ResponseKind::Bars => {
                    classify_bars(&verified.entry.file_name, verified.bytes, manifest.date)?
                }
                ResponseKind::Reference => DataBatchKind::Reference,
                ResponseKind::Calendar => {
                    let parsed = parse_calendar(
                        &verified.entry.file_name,
                        verified.bytes,
                        &verified.content_sha256,
                    )?;
                    register_calendar_provenance(
                        &mut calendar_provenance,
                        &parsed.source_version,
                        &parsed.provenance,
                    )?;
                    for fact in parsed.facts {
                        insert_calendar_fact(&mut calendar_facts, fact)?;
                    }
                    DataBatchKind::Calendar
                }
                ResponseKind::CorporateActions => DataBatchKind::CorporateActions,
                ResponseKind::InvestorFlow => DataBatchKind::InvestorFlow,
                ResponseKind::MarketStatus => DataBatchKind::MarketStatus,
                ResponseKind::Fundamentals => DataBatchKind::Fundamentals,
                ResponseKind::IndexMembership => DataBatchKind::IndexMembership,
                ResponseKind::SectorClassification => DataBatchKind::SectorClassification,
                ResponseKind::CandidateMaster
                | ResponseKind::DisclosureIndex
                | ResponseKind::DisclosureEntityMaster
                | ResponseKind::DisclosureEntityProfile
                | ResponseKind::DisclosureVersionMembership => {
                    return Err(PublicationError::UnsupportedManifestScope {
                        expected_scopes: "krx/kr or kis-normalized/kr",
                        provider: manifest.provider.clone(),
                        market: manifest.market.clone(),
                    });
                }
            };
            files.push(PublicationFile {
                file_name: verified.entry.file_name.clone(),
                kind,
                content_sha256: verified.content_sha256,
                storage_path: verified.storage_path,
                bytes_size: verified.entry.size_bytes,
            });
        }
        Ok(Self {
            source_batch_id: manifest.batch_id,
            provider: manifest.provider.clone(),
            market: manifest.market.clone(),
            target_date: manifest.date,
            retrieved_at: manifest.retrieved_at,
            fetch_mode: manifest.mode,
            files,
            calendar_facts: calendar_facts.into_values().collect(),
        })
    }
}

const NORMALIZED_PUBLICATION_FILES: [(ResponseKind, &str); 4] = [
    (ResponseKind::Bars, "bars.json"),
    (ResponseKind::Reference, "reference.json"),
    (ResponseKind::Calendar, "calendar.json"),
    (ResponseKind::CorporateActions, "corporate-actions.json"),
];

fn validate_normalized_manifest(manifest: &ManifestEntry) -> Result<(), PublicationError> {
    if manifest.files.len() != NORMALIZED_PUBLICATION_FILES.len() {
        return Err(PublicationError::NonCanonicalNormalizedManifest {
            reason: format!(
                "expected exactly {} files, got {}",
                NORMALIZED_PUBLICATION_FILES.len(),
                manifest.files.len()
            ),
        });
    }
    for (kind, file_name) in NORMALIZED_PUBLICATION_FILES {
        let matches = manifest
            .files
            .iter()
            .filter(|file| file.kind == kind && file.file_name == file_name)
            .count();
        if matches != 1 {
            return Err(PublicationError::NonCanonicalNormalizedManifest {
                reason: format!(
                    "expected exactly one {kind} file named {file_name}, got {matches}"
                ),
            });
        }
    }
    if manifest
        .files
        .iter()
        .any(|file| file.request.mode != FetchMode::Credentialed)
    {
        return Err(PublicationError::NonCanonicalNormalizedManifest {
            reason: "all normalized file requests must be credentialed".to_owned(),
        });
    }
    Ok(())
}

const NORMALIZER: &str = "kis-wire-to-canonical-v2";
const NORMALIZER_SCHEMA_VERSION: u32 = 1;

fn invalid_provenance(file_name: &str, reason: impl Into<String>) -> PublicationError {
    PublicationError::InvalidCanonicalProvenance {
        file_name: file_name.to_owned(),
        reason: reason.into(),
    }
}

fn parse_normalization_lineage(
    file_name: &str,
    bytes: &[u8],
) -> Result<NormalizationLineage, PublicationError> {
    let document: Value = serde_json::from_slice(bytes)
        .map_err(|error| invalid_provenance(file_name, format!("invalid JSON: {error}")))?;
    let lineage_value = document
        .get("_lineage")
        .ok_or_else(|| invalid_provenance(file_name, "missing _lineage"))?;
    let lineage: NormalizationLineage =
        serde_json::from_value(lineage_value.clone()).map_err(|error| {
            invalid_provenance(file_name, format!("invalid _lineage shape: {error}"))
        })?;
    let canonical_lineage = serde_json::to_value(&lineage).map_err(|error| {
        invalid_provenance(file_name, format!("cannot serialize _lineage: {error}"))
    })?;
    if lineage_value != &canonical_lineage {
        return Err(invalid_provenance(
            file_name,
            "_lineage contains noncanonical or unknown fields",
        ));
    }
    Ok(lineage)
}

fn validate_lineage_fields(
    file_name: &str,
    manifest: &ManifestEntry,
    lineage: &NormalizationLineage,
) -> Result<(), PublicationError> {
    if lineage.schema_version != NORMALIZER_SCHEMA_VERSION {
        return Err(invalid_provenance(
            file_name,
            format!(
                "unsupported normalizer schema version {}",
                lineage.schema_version
            ),
        ));
    }
    if lineage.normalizer != NORMALIZER {
        return Err(invalid_provenance(
            file_name,
            format!("unexpected normalizer {}", lineage.normalizer),
        ));
    }
    if lineage.upstream_provider != PROVIDER_KIS || lineage.upstream_market != MARKET_KR {
        return Err(invalid_provenance(
            file_name,
            format!(
                "unexpected upstream scope {}/{}",
                lineage.upstream_provider, lineage.upstream_market
            ),
        ));
    }
    if deterministic_kis_normalized_batch_id(lineage.upstream_batch_id) != manifest.batch_id {
        return Err(invalid_provenance(
            file_name,
            "normalized batch id does not match upstream batch lineage",
        ));
    }
    if lineage.upstream_batch_id.as_uuid().is_nil() {
        return Err(invalid_provenance(
            file_name,
            "upstream_batch_id must not be nil",
        ));
    }
    if lineage.upstream_files.is_empty() {
        return Err(invalid_provenance(
            file_name,
            "upstream_files must be nonempty",
        ));
    }

    let mut file_names = BTreeSet::new();
    let mut tuples = BTreeSet::new();
    for source_file in &lineage.upstream_files {
        if source_file.file_name.trim().is_empty() {
            return Err(invalid_provenance(
                file_name,
                "upstream file names must be nonempty",
            ));
        }
        if !file_names.insert(source_file.file_name.as_str()) {
            return Err(invalid_provenance(
                file_name,
                format!("duplicate upstream file {}", source_file.file_name),
            ));
        }
        if !tuples.insert((
            source_file.kind,
            source_file.file_name.clone(),
            source_file.content_hash.clone(),
        )) {
            return Err(invalid_provenance(
                file_name,
                format!(
                    "duplicate upstream evidence tuple for {}",
                    source_file.file_name
                ),
            ));
        }
    }

    let mut sorted = lineage.upstream_files.clone();
    sorted.sort_by(|left, right| (left.kind, &left.file_name).cmp(&(right.kind, &right.file_name)));
    if sorted != lineage.upstream_files {
        return Err(invalid_provenance(
            file_name,
            "upstream_files are not in canonical kind/name order",
        ));
    }

    let kinds: BTreeSet<_> = lineage
        .upstream_files
        .iter()
        .map(|source_file| source_file.kind)
        .collect();
    let expected_kinds = [
        ResponseKind::Bars,
        ResponseKind::Reference,
        ResponseKind::Calendar,
        ResponseKind::CorporateActions,
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    if !expected_kinds.is_subset(&kinds) {
        return Err(invalid_provenance(
            file_name,
            "upstream_files must contain all four EOD response kinds",
        ));
    }
    Ok(())
}

fn validate_normalized_request(
    file: &FileEntry,
    lineage: &NormalizationLineage,
) -> Result<(), PublicationError> {
    if file.request.mode != FetchMode::Credentialed {
        return Err(invalid_provenance(
            &file.file_name,
            "normalized request must be credentialed",
        ));
    }
    let expected_endpoint = format!("kis.normalized/{NORMALIZER}/{}", file.kind);
    if file.request.endpoint != expected_endpoint {
        return Err(invalid_provenance(
            &file.file_name,
            format!(
                "unexpected normalized endpoint {}, expected {expected_endpoint}",
                file.request.endpoint
            ),
        ));
    }

    let mut upstream_batch_id = None;
    let mut upstream_lineage = None;
    if file.request.query.len() != 2 {
        return Err(invalid_provenance(
            &file.file_name,
            "normalized request query must contain exactly upstream_batch_id and upstream_lineage",
        ));
    }
    for (key, value) in &file.request.query {
        match key.as_str() {
            "upstream_batch_id" if upstream_batch_id.is_none() => upstream_batch_id = Some(value),
            "upstream_lineage" if upstream_lineage.is_none() => upstream_lineage = Some(value),
            "upstream_batch_id" | "upstream_lineage" => {
                return Err(invalid_provenance(
                    &file.file_name,
                    format!("duplicate normalized query key {key}"),
                ));
            }
            _ => {
                return Err(invalid_provenance(
                    &file.file_name,
                    format!("unexpected normalized query key {key}"),
                ));
            }
        }
    }
    let expected_batch_id = lineage.upstream_batch_id.to_string();
    if upstream_batch_id != Some(&expected_batch_id) {
        return Err(invalid_provenance(
            &file.file_name,
            "upstream_batch_id query does not match document lineage",
        ));
    }
    let expected_lineage = serde_json::to_string(lineage).map_err(|error| {
        invalid_provenance(
            &file.file_name,
            format!("cannot serialize expected lineage: {error}"),
        )
    })?;
    if upstream_lineage != Some(&expected_lineage) {
        return Err(invalid_provenance(
            &file.file_name,
            "upstream_lineage query does not match document lineage",
        ));
    }
    Ok(())
}

fn validate_normalized_provenance(
    manifest: &ManifestEntry,
    verified_files: &[VerifiedRawFile<'_>],
) -> Result<(), PublicationError> {
    let mut lineages = Vec::with_capacity(verified_files.len());
    for verified in verified_files {
        let lineage = parse_normalization_lineage(&verified.entry.file_name, verified.bytes)?;
        validate_lineage_fields(&verified.entry.file_name, manifest, &lineage)?;
        validate_normalized_request(verified.entry, &lineage)?;
        lineages.push((verified.entry.file_name.as_str(), lineage));
    }
    let Some((first_file, first_lineage)) = lineages.first() else {
        return Err(invalid_provenance(
            "<manifest>",
            "normalized publication has no files",
        ));
    };
    for (file_name, lineage) in &lineages[1..] {
        if lineage != first_lineage {
            return Err(invalid_provenance(
                file_name,
                format!("lineage differs from canonical file {first_file}"),
            ));
        }
    }
    Ok(())
}

fn validate_file_metadata<'a>(
    entry: &'a FileEntry,
    stored_file: &'a StoredFile,
) -> Result<VerifiedRawFile<'a>, PublicationError> {
    if entry.file_name != stored_file.file_name {
        return Err(PublicationError::ReadbackFileNameMismatch {
            manifest_file_name: entry.file_name.clone(),
            read_file_name: stored_file.file_name.clone(),
        });
    }
    if entry.size_bytes > i64::MAX as u64 {
        return Err(PublicationError::SizeExceedsPostgresBigint {
            file_name: entry.file_name.clone(),
            size: entry.size_bytes,
        });
    }
    let read_size = stored_file.bytes.len() as u64;
    if entry.size_bytes != read_size {
        return Err(PublicationError::SizeMismatch {
            file_name: entry.file_name.clone(),
            manifest_size: entry.size_bytes,
            read_size,
        });
    }
    let content_sha256 = database_hash(&entry.file_name, entry.content_hash.as_str())?;
    let storage_path = stored_file
        .storage_path
        .clone()
        .into_os_string()
        .into_string()
        .map_err(|_| PublicationError::NonUtf8StoragePath {
            file_name: entry.file_name.clone(),
        })?;
    Ok(VerifiedRawFile {
        entry,
        bytes: &stored_file.bytes,
        content_sha256,
        storage_path,
    })
}

fn parse_calendar(
    file_name: &str,
    bytes: &[u8],
    content_sha256: &str,
) -> Result<ParsedCalendar, PublicationError> {
    let doc: CalendarDoc =
        serde_json::from_slice(bytes).map_err(|error| PublicationError::MalformedCalendar {
            file_name: file_name.to_owned(),
            reason: error.to_string(),
        })?;
    if doc.calendar_id.trim().is_empty() {
        return Err(PublicationError::MalformedCalendar {
            file_name: file_name.to_owned(),
            reason: "calendar_id must be nonempty".to_owned(),
        });
    }
    if doc.source.trim().is_empty() {
        return Err(PublicationError::MalformedCalendar {
            file_name: file_name.to_owned(),
            reason: "source must be nonempty".to_owned(),
        });
    }
    if doc.timezone != "Asia/Seoul" {
        return Err(PublicationError::UnsupportedCalendarTimezone {
            file_name: file_name.to_owned(),
            timezone: doc.timezone,
        });
    }
    if doc.session_times_local.open != "09:00:00" || doc.session_times_local.close != "15:30:00" {
        return Err(PublicationError::InvalidCalendarSessionTimes {
            file_name: file_name.to_owned(),
            open: doc.session_times_local.open,
            close: doc.session_times_local.close,
        });
    }

    let source_version = format!("{}:schema-{}", doc.calendar_id, doc.schema_version);
    let mut facts = Vec::with_capacity(doc.sessions.len() + doc.holidays.len());
    let mut session_dates = BTreeSet::new();
    for session in doc.sessions {
        let date = calendar_date(file_name, &session.date)?;
        verify_calendar_instant(file_name, &date, "open_utc", &session.open_utc, 9, 0)?;
        verify_calendar_instant(file_name, &date, "close_utc", &session.close_utc, 15, 30)?;
        session_dates.insert(date);
        facts.push(CalendarFact {
            exchange: "KRX".to_owned(),
            session_date: date,
            session_type: CalendarSessionType::Trading,
            timezone: "Asia/Seoul".to_owned(),
            source: doc.source.clone(),
            source_version: source_version.clone(),
            content_sha256: content_sha256.to_owned(),
        });
    }
    for holiday in doc.holidays {
        let date = calendar_date(file_name, &holiday.date)?;
        if session_dates.contains(&date) {
            return Err(PublicationError::CalendarDateBothSessionAndHoliday {
                file_name: file_name.to_owned(),
                date: date.to_iso(),
            });
        }
        facts.push(CalendarFact {
            exchange: "KRX".to_owned(),
            session_date: date,
            session_type: CalendarSessionType::Closed,
            timezone: "Asia/Seoul".to_owned(),
            source: doc.source.clone(),
            source_version: source_version.clone(),
            content_sha256: content_sha256.to_owned(),
        });
    }
    Ok(ParsedCalendar {
        source_version,
        provenance: CalendarProvenance {
            source: doc.source,
            timezone: doc.timezone,
            content_sha256: content_sha256.to_owned(),
        },
        facts,
    })
}

fn register_calendar_provenance(
    provenances: &mut BTreeMap<String, CalendarProvenance>,
    source_version: &str,
    incoming: &CalendarProvenance,
) -> Result<(), PublicationError> {
    match provenances.get(source_version) {
        Some(existing) if existing == incoming => Ok(()),
        Some(_) => Err(PublicationError::ConflictingCalendarProvenance {
            source_version: source_version.to_owned(),
        }),
        None => {
            provenances.insert(source_version.to_owned(), incoming.clone());
            Ok(())
        }
    }
}

fn calendar_date(file_name: &str, value: &str) -> Result<TradingDate, PublicationError> {
    TradingDate::parse(value).map_err(|_| PublicationError::InvalidCalendarDate {
        file_name: file_name.to_owned(),
        value: value.to_owned(),
    })
}

fn verify_calendar_instant(
    file_name: &str,
    date: &TradingDate,
    field: &str,
    value: &str,
    expected_hour: u32,
    expected_minute: u32,
) -> Result<(), PublicationError> {
    let timestamp = UtcTimestamp::parse_rfc3339(value).map_err(|_| {
        PublicationError::InvalidCalendarTimestamp {
            file_name: file_name.to_owned(),
            field: field.to_owned(),
            value: value.to_owned(),
        }
    })?;
    let seoul_offset = FixedOffset::east_opt(9 * 60 * 60).expect("valid Seoul offset");
    let expected_local = date
        .as_naive_date()
        .and_hms_opt(expected_hour, expected_minute, 0)
        .expect("contract session time is valid");
    let expected_utc = seoul_offset
        .from_local_datetime(&expected_local)
        .single()
        .expect("Asia/Seoul fixed offset has one local instant")
        .with_timezone(&Utc);
    if timestamp.as_datetime() != expected_utc {
        return Err(PublicationError::InconsistentCalendarInstant {
            file_name: file_name.to_owned(),
            date: date.to_iso(),
            field: field.to_owned(),
            value: value.to_owned(),
        });
    }
    Ok(())
}

fn insert_calendar_fact(
    facts: &mut BTreeMap<(TradingDate, String, String), CalendarFact>,
    incoming: CalendarFact,
) -> Result<(), PublicationError> {
    let key = (
        incoming.session_date,
        incoming.exchange.clone(),
        incoming.source_version.clone(),
    );
    if let Some(existing) = facts.get(&key) {
        if existing == &incoming {
            return Ok(());
        }
        return Err(PublicationError::ConflictingCalendarFact {
            exchange: incoming.exchange,
            date: incoming.session_date,
            source_version: incoming.source_version,
        });
    }
    facts.insert(key, incoming);
    Ok(())
}

fn database_hash(file_name: &str, value: &str) -> Result<String, PublicationError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(PublicationError::UnexpectedContentHash {
            file_name: file_name.to_owned(),
            value: value.to_owned(),
        });
    };
    if hex.len() != 64
        || !hex.bytes().all(|byte| byte.is_ascii_hexdigit())
        || hex != hex.to_ascii_lowercase()
    {
        return Err(PublicationError::UnexpectedContentHash {
            file_name: file_name.to_owned(),
            value: value.to_owned(),
        });
    }
    Ok(hex.to_owned())
}

fn classify_bars(
    file_name: &str,
    bytes: &[u8],
    target_date: TradingDate,
) -> Result<DataBatchKind, PublicationError> {
    let doc: BarsDoc =
        serde_json::from_slice(bytes).map_err(|error| PublicationError::MalformedBars {
            file_name: file_name.to_owned(),
            reason: error.to_string(),
        })?;
    let mut has_target_date = false;
    for row in doc.bars {
        let date = TradingDate::parse(&row.date).map_err(|_| PublicationError::InvalidBarDate {
            file_name: file_name.to_owned(),
            value: row.date,
        })?;
        has_target_date |= date == target_date;
    }
    Ok(if has_target_date {
        DataBatchKind::Eod
    } else {
        DataBatchKind::EodUnavailable
    })
}
