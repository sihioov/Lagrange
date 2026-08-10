//! Typed, transport-neutral publication records derived from immutable Raw batches.

use chrono::{FixedOffset, TimeZone, Utc};
use domain::{BatchId, TradingDate, UtcTimestamp};
use serde::Deserialize;

use crate::contract::{FetchMode, ResponseKind, StoredFile};
use crate::storage::{FileEntry, ManifestEntry, RawStore, StoreError};

/// Stable batch kind stored by downstream publication sinks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataBatchKind {
    Eod,
    EodUnavailable,
    Reference,
    Calendar,
    CorporateActions,
}

impl DataBatchKind {
    pub const fn as_db_str(self) -> &'static str {
        match self {
            Self::Eod => "EOD",
            Self::EodUnavailable => "EOD_UNAVAILABLE",
            Self::Reference => "REFERENCE",
            Self::Calendar => "CALENDAR",
            Self::CorporateActions => "CORPORATE_ACTIONS",
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
}

#[derive(Debug, thiserror::Error)]
pub enum PublicationError {
    #[error("verified Raw read failed: {0}")]
    Store(#[from] StoreError),
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
    #[error("verified Raw file count differs from manifest: manifest={manifest_count}, read={read_count}")]
    ReadbackFileCountMismatch {
        manifest_count: usize,
        read_count: usize,
    },
    #[error("verified Raw file name differs from manifest: manifest={manifest_file_name}, read={read_file_name}")]
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

impl PublicationBundle {
    /// Reads every file back through Raw verification before producing any facts.
    pub fn from_raw(store: &RawStore, manifest: &ManifestEntry) -> Result<Self, PublicationError> {
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
            .map(|(entry, stored_file)| validate_file_metadata(store, manifest, entry, stored_file))
            .collect::<Result<_, _>>()?;

        let mut files = Vec::with_capacity(verified_files.len());
        let mut calendar_facts = Vec::new();
        for verified in verified_files {
            let kind = match verified.entry.kind {
                ResponseKind::Bars => {
                    classify_bars(&verified.entry.file_name, verified.bytes, manifest.date)?
                }
                ResponseKind::Reference => DataBatchKind::Reference,
                ResponseKind::Calendar => {
                    let facts = parse_calendar(&verified.entry.file_name, verified.bytes)?;
                    for fact in facts {
                        insert_calendar_fact(&mut calendar_facts, fact)?;
                    }
                    DataBatchKind::Calendar
                }
                ResponseKind::CorporateActions => DataBatchKind::CorporateActions,
            };
            files.push(PublicationFile {
                file_name: verified.entry.file_name.clone(),
                kind,
                content_sha256: verified.content_sha256,
                storage_path: verified.storage_path,
                bytes_size: verified.entry.size_bytes,
            });
        }
        calendar_facts.sort_by_key(|fact: &CalendarFact| fact.session_date);

        Ok(Self {
            source_batch_id: manifest.batch_id,
            provider: manifest.provider.clone(),
            market: manifest.market.clone(),
            target_date: manifest.date,
            retrieved_at: manifest.retrieved_at,
            fetch_mode: manifest.mode,
            files,
            calendar_facts,
        })
    }
}

fn validate_file_metadata<'a>(
    store: &RawStore,
    manifest: &ManifestEntry,
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
    let path = store
        .batch_dir(
            &manifest.provider,
            &manifest.market,
            &manifest.date,
            &manifest.batch_id,
        )
        .join(&entry.file_name);
    let storage_path =
        path.into_os_string()
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

fn parse_calendar(file_name: &str, bytes: &[u8]) -> Result<Vec<CalendarFact>, PublicationError> {
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
    for session in doc.sessions {
        let date = calendar_date(file_name, &session.date)?;
        verify_calendar_instant(file_name, &date, "open_utc", &session.open_utc, 9, 0)?;
        verify_calendar_instant(file_name, &date, "close_utc", &session.close_utc, 15, 30)?;
        facts.push(CalendarFact {
            exchange: "KRX".to_owned(),
            session_date: date,
            session_type: CalendarSessionType::Trading,
            timezone: "Asia/Seoul".to_owned(),
            source: doc.source.clone(),
            source_version: source_version.clone(),
        });
    }
    for holiday in doc.holidays {
        let date = calendar_date(file_name, &holiday.date)?;
        if facts.iter().any(|fact| {
            fact.session_date == date && fact.session_type == CalendarSessionType::Trading
        }) {
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
        });
    }
    Ok(facts)
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
    facts: &mut Vec<CalendarFact>,
    incoming: CalendarFact,
) -> Result<(), PublicationError> {
    if let Some(existing) = facts.iter().find(|existing| {
        existing.exchange == incoming.exchange
            && existing.session_date == incoming.session_date
            && existing.source_version == incoming.source_version
    }) {
        if existing == &incoming {
            return Ok(());
        }
        return Err(PublicationError::ConflictingCalendarFact {
            exchange: incoming.exchange,
            date: incoming.session_date,
            source_version: incoming.source_version,
        });
    }
    facts.push(incoming);
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
