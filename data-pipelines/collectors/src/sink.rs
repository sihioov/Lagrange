use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Timelike, Utc};
use domain::{BatchId, TradingDate, UtcTimestamp};
use market_data::contract::{FetchMode, MARKET_KR, PROVIDER_KIS_NORMALIZED, PROVIDER_KRX};
use market_data::publication::{CalendarFact, DataBatchKind, PublicationBundle, PublicationFile};
use sqlx::{FromRow, PgPool, Postgres, Transaction};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DatabaseScope {
    provider: &'static str,
    market: &'static str,
}

const DB_SCOPE: DatabaseScope = DatabaseScope {
    provider: "KRX",
    market: "KR",
};
const DB_PROVIDER: &str = DB_SCOPE.provider;
const DB_MARKET: &str = DB_SCOPE.market;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublicationScope {
    Krx,
    KisNormalized,
}

impl PublicationScope {
    const fn database_scope(self) -> DatabaseScope {
        match self {
            Self::Krx | Self::KisNormalized => DB_SCOPE,
        }
    }

    const fn required_mode(self) -> Option<FetchMode> {
        match self {
            Self::Krx => None,
            Self::KisNormalized => Some(FetchMode::Credentialed),
        }
    }
}

fn publication_scope(provider: &str, market: &str) -> Option<PublicationScope> {
    match (provider, market) {
        (PROVIDER_KRX, MARKET_KR) => Some(PublicationScope::Krx),
        (PROVIDER_KIS_NORMALIZED, MARKET_KR) => Some(PublicationScope::KisNormalized),
        _ => None,
    }
}

fn sqlstate_is_retryable(code: &str) -> bool {
    code.starts_with("08")
        || matches!(
            code,
            "40001" | "40P01" | "55P03" | "57014" | "57P01" | "57P02" | "57P03"
        )
}

fn canonical_retrieved_at(timestamp: DateTime<Utc>) -> DateTime<Utc> {
    timestamp
        .with_nanosecond(0)
        .expect("zero nanoseconds are always valid")
}

fn postgres_retrieved_at(timestamp: UtcTimestamp) -> DateTime<Utc> {
    // UtcTimestamp's durable JSON contract serializes whole seconds.  An
    // initial in-process publication can still carry subsecond precision,
    // while crash recovery reconstructs the same timestamp from batch.json
    // without those fractions. Persist the durable precision so replay does
    // not conflict with its own immutable manifest.
    canonical_retrieved_at(timestamp.as_datetime())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishOutcome {
    Published,
    AlreadyPublished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationState {
    Missing,
    Complete,
    Partial,
}

#[derive(Debug, thiserror::Error)]
pub enum SinkError {
    #[error("publication conflict: {0}")]
    Conflict(String),
    #[error("publication invariant violation: {0}")]
    Invariant(String),
    #[error("retryable database failure")]
    RetryableDatabase(#[source] sqlx::Error),
    #[error("permanent database failure")]
    PermanentDatabase(#[source] sqlx::Error),
    #[error("publication database conflict: {context}")]
    DatabaseConflict {
        context: String,
        #[source]
        source: sqlx::Error,
    },
}

impl SinkError {
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::RetryableDatabase(_))
    }

    pub fn from_sqlx(error: sqlx::Error) -> Self {
        let retryable = match &error {
            sqlx::Error::Io(_)
            | sqlx::Error::Tls(_)
            | sqlx::Error::Protocol(_)
            | sqlx::Error::PoolTimedOut
            | sqlx::Error::PoolClosed
            | sqlx::Error::WorkerCrashed => true,
            sqlx::Error::Database(database) => database
                .code()
                .is_some_and(|code| sqlstate_is_retryable(code.as_ref())),
            _ => false,
        };
        if retryable {
            Self::RetryableDatabase(error)
        } else {
            Self::PermanentDatabase(error)
        }
    }
}

#[async_trait]
pub trait PublicationSink: Send + Sync {
    async fn publication_state(&self, batch_id: BatchId) -> Result<PublicationState, SinkError>;
    async fn publish(&self, bundle: &PublicationBundle) -> Result<PublishOutcome, SinkError>;
    async fn has_eod(&self, date: TradingDate) -> Result<bool, SinkError>;
}

#[derive(Clone)]
pub struct PostgresPublicationSink {
    pool: PgPool,
}

impl PostgresPublicationSink {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn has_eod_for_mode(
        &self,
        date: TradingDate,
        mode: market_data::FetchMode,
    ) -> Result<bool, SinkError> {
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM data_batches
              WHERE provider=$1 AND market=$2 AND batch_date=$3 AND kind='EOD'
                AND fetch_mode=$4)",
        )
        .bind(DB_PROVIDER)
        .bind(DB_MARKET)
        .bind(date.as_naive_date())
        .bind(mode.as_str())
        .fetch_one(&self.pool)
        .await
        .map_err(SinkError::from_sqlx)
    }
}

#[derive(FromRow)]
struct ExistingBatchRow {
    batch_date: NaiveDate,
    kind: String,
    storage_path: String,
    content_sha256: String,
    bytes_size: i64,
    retrieved_at: DateTime<Utc>,
    source_file_name: String,
    fetch_mode: String,
}

#[derive(FromRow)]
struct ExistingHistoryRow {
    session_type: String,
    timezone: String,
    source: String,
    content_sha256: String,
}

#[derive(FromRow)]
struct ExistingProjectionRow {
    session_type: String,
    timezone: String,
    source: String,
    source_version: String,
    content_sha256: Option<String>,
    retrieved_at: Option<DateTime<Utc>>,
}

fn exact_file_shape(files: &[PublicationFile]) -> bool {
    if files.len() != 4 {
        return false;
    }
    let unique_names: BTreeSet<_> = files.iter().map(|file| file.file_name.as_str()).collect();
    let bars = files
        .iter()
        .filter(|file| {
            matches!(
                file.kind,
                DataBatchKind::Eod | DataBatchKind::EodUnavailable
            )
        })
        .count();
    bars == 1
        && unique_names.len() == 4
        && files
            .iter()
            .filter(|file| file.kind == DataBatchKind::Reference)
            .count()
            == 1
        && files
            .iter()
            .filter(|file| file.kind == DataBatchKind::Calendar)
            .count()
            == 1
        && files
            .iter()
            .filter(|file| file.kind == DataBatchKind::CorporateActions)
            .count()
            == 1
}

const NORMALIZED_FILE_SHAPE: [(DataBatchKind, &str); 4] = [
    (DataBatchKind::Eod, "bars.json"),
    (DataBatchKind::Reference, "reference.json"),
    (DataBatchKind::Calendar, "calendar.json"),
    (DataBatchKind::CorporateActions, "corporate-actions.json"),
];

fn exact_normalized_file_shape(files: &[PublicationFile]) -> bool {
    if files.len() != NORMALIZED_FILE_SHAPE.len() {
        return false;
    }
    let bars = files
        .iter()
        .filter(|file| {
            file.file_name == "bars.json"
                && matches!(
                    file.kind,
                    DataBatchKind::Eod | DataBatchKind::EodUnavailable
                )
        })
        .count();
    bars == 1
        && NORMALIZED_FILE_SHAPE[1..].iter().all(|(kind, file_name)| {
            files
                .iter()
                .filter(|file| file.kind == *kind && file.file_name == *file_name)
                .count()
                == 1
        })
}

fn validate_bundle(bundle: &PublicationBundle) -> Result<(), SinkError> {
    let scope = publication_scope(&bundle.provider, &bundle.market).ok_or_else(|| {
        SinkError::Invariant(format!(
            "unsupported Raw scope {}/{}",
            bundle.provider, bundle.market
        ))
    })?;
    let database_scope = scope.database_scope();
    debug_assert_eq!(database_scope.provider, DB_PROVIDER);
    debug_assert_eq!(database_scope.market, DB_MARKET);
    if scope
        .required_mode()
        .is_some_and(|expected| bundle.fetch_mode != expected)
    {
        return Err(SinkError::Invariant(format!(
            "Raw scope {}/{} requires credentialed fetch mode",
            bundle.provider, bundle.market
        )));
    }
    if !exact_file_shape(&bundle.files) {
        return Err(SinkError::Conflict(
            "bundle is not the exact four-response shape".to_owned(),
        ));
    }
    if scope == PublicationScope::KisNormalized && !exact_normalized_file_shape(&bundle.files) {
        return Err(SinkError::Conflict(
            "normalized bundle does not have the canonical four-file shape".to_owned(),
        ));
    }
    for file in &bundle.files {
        i64::try_from(file.bytes_size).map_err(|_| {
            SinkError::Invariant(format!("file {} exceeds PostgreSQL bigint", file.file_name))
        })?;
        if file.file_name.is_empty() || file.storage_path.is_empty() {
            return Err(SinkError::Invariant(
                "publication file names and paths must be nonempty".to_owned(),
            ));
        }
        if file.content_sha256.len() != 64
            || !file
                .content_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(SinkError::Invariant(format!(
                "file {} has a noncanonical content hash",
                file.file_name
            )));
        }
    }
    let calendar_hash = bundle
        .files
        .iter()
        .find(|file| file.kind == DataBatchKind::Calendar)
        .expect("exact file shape includes one calendar file")
        .content_sha256
        .as_str();
    let mut source_versions = BTreeMap::new();
    for fact in &bundle.calendar_facts {
        if fact.exchange != DB_PROVIDER || fact.timezone != "Asia/Seoul" {
            return Err(SinkError::Invariant(format!(
                "unsupported calendar scope for {} {}",
                fact.exchange, fact.session_date
            )));
        }
        if fact.content_sha256.len() != 64
            || !fact
                .content_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(SinkError::Invariant(format!(
                "calendar {} {} has a noncanonical content hash",
                fact.exchange, fact.session_date
            )));
        }
        if fact.content_sha256 != calendar_hash {
            return Err(SinkError::Invariant(format!(
                "calendar {} {} is not anchored to the calendar file hash",
                fact.exchange, fact.session_date
            )));
        }
        let key = (fact.exchange.as_str(), fact.source_version.as_str());
        let provenance = (
            fact.source.as_str(),
            fact.timezone.as_str(),
            fact.content_sha256.as_str(),
        );
        if source_versions
            .insert(key, provenance)
            .is_some_and(|existing| existing != provenance)
        {
            return Err(SinkError::Invariant(format!(
                "calendar source version has conflicting provenance for {} {}",
                fact.exchange, fact.source_version
            )));
        }
    }
    Ok(())
}

fn exact_kind_shape(kinds_and_names: &[(String, String)]) -> bool {
    if kinds_and_names.len() != 4 {
        return false;
    }
    let names: BTreeSet<_> = kinds_and_names
        .iter()
        .map(|(_, name)| name.as_str())
        .collect();
    let count = |kind: &str| {
        kinds_and_names
            .iter()
            .filter(|(candidate, _)| candidate == kind)
            .count()
    };
    names.len() == 4
        && count("EOD") + count("EOD_UNAVAILABLE") == 1
        && count("REFERENCE") == 1
        && count("CALENDAR") == 1
        && count("CORPORATE_ACTIONS") == 1
}

fn semantic_conflict(error: sqlx::Error, context: impl Into<String>) -> SinkError {
    if error
        .as_database_error()
        .and_then(|database| database.code())
        .is_some_and(|code| code == "23505")
    {
        SinkError::DatabaseConflict {
            context: context.into(),
            source: error,
        }
    } else {
        SinkError::from_sqlx(error)
    }
}

fn history_matches(existing: &ExistingHistoryRow, fact: &CalendarFact) -> bool {
    existing.session_type == fact.session_type.as_db_str()
        && existing.timezone == fact.timezone
        && existing.source == fact.source
        && existing.content_sha256 == fact.content_sha256
}

fn projection_matches(existing: &ExistingProjectionRow, fact: &CalendarFact) -> bool {
    existing.session_type == fact.session_type.as_db_str()
        && existing.timezone == fact.timezone
        && existing.source == fact.source
        && existing.source_version == fact.source_version
        && existing.content_sha256.as_deref() == Some(fact.content_sha256.as_str())
}

async fn load_batch_rows(
    tx: &mut Transaction<'_, Postgres>,
    batch_id: BatchId,
) -> Result<Vec<ExistingBatchRow>, SinkError> {
    sqlx::query_as(
        "SELECT batch_date, kind, storage_path, content_sha256, bytes_size, retrieved_at, \
                source_file_name, fetch_mode \
         FROM data_batches \
         WHERE provider=$1 AND market=$2 AND source_batch_id=$3 \
         ORDER BY source_file_name",
    )
    .bind(DB_PROVIDER)
    .bind(DB_MARKET)
    .bind(batch_id.as_uuid())
    .fetch_all(&mut **tx)
    .await
    .map_err(SinkError::from_sqlx)
}

fn batch_rows_match(
    rows: &[ExistingBatchRow],
    bundle: &PublicationBundle,
    retrieved_at: DateTime<Utc>,
) -> Result<(), SinkError> {
    let expected: BTreeMap<_, _> = bundle
        .files
        .iter()
        .map(|file| (file.file_name.as_str(), file))
        .collect();
    let shape: Vec<_> = rows
        .iter()
        .map(|row| (row.kind.clone(), row.source_file_name.clone()))
        .collect();
    if !exact_kind_shape(&shape) {
        return Err(SinkError::Conflict(
            "existing source batch is partial or noncanonical".to_owned(),
        ));
    }
    for row in rows {
        let Some(file) = expected.get(row.source_file_name.as_str()) else {
            return Err(SinkError::Conflict(
                "existing source batch contains an unexpected file".to_owned(),
            ));
        };
        let size = i64::try_from(file.bytes_size)
            .map_err(|_| SinkError::Invariant("file size exceeds PostgreSQL bigint".to_owned()))?;
        if row.batch_date != bundle.target_date.as_naive_date()
            || row.kind != file.kind.as_db_str()
            || row.storage_path != file.storage_path
            || row.content_sha256 != file.content_sha256
            || row.bytes_size != size
            // Releases before the whole-second persistence fix may already
            // contain PostgreSQL microseconds from the initial in-memory
            // publication. The immutable manifest cannot reproduce those
            // fractions, so compare at its canonical serialized precision.
            || canonical_retrieved_at(row.retrieved_at) != retrieved_at
            || row.fetch_mode != bundle.fetch_mode.as_str()
        {
            return Err(SinkError::Conflict(format!(
                "existing publication evidence differs for {}",
                file.file_name
            )));
        }
    }
    Ok(())
}

async fn insert_batch_rows(
    tx: &mut Transaction<'_, Postgres>,
    bundle: &PublicationBundle,
    retrieved_at: DateTime<Utc>,
) -> Result<(), SinkError> {
    for file in &bundle.files {
        sqlx::query(
            "INSERT INTO data_batches \
             (provider, market, batch_date, kind, storage_path, content_sha256, bytes_size, \
              retrieved_at, source_batch_id, source_file_name, fetch_mode) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
        )
        .bind(DB_PROVIDER)
        .bind(DB_MARKET)
        .bind(bundle.target_date.as_naive_date())
        .bind(file.kind.as_db_str())
        .bind(&file.storage_path)
        .bind(&file.content_sha256)
        .bind(i64::try_from(file.bytes_size).expect("bundle size validated"))
        .bind(retrieved_at)
        .bind(bundle.source_batch_id.as_uuid())
        .bind(&file.file_name)
        .bind(bundle.fetch_mode.as_str())
        .execute(&mut **tx)
        .await
        .map_err(|error| semantic_conflict(error, "source-file lineage is already occupied"))?;
    }
    Ok(())
}

async fn verify_or_insert_history(
    tx: &mut Transaction<'_, Postgres>,
    bundle: &PublicationBundle,
    replay: bool,
    retrieved_at: DateTime<Utc>,
) -> Result<(), SinkError> {
    lock_and_verify_source_versions(tx, bundle).await?;
    for fact in &bundle.calendar_facts {
        let mut existing: Option<ExistingHistoryRow> = sqlx::query_as(
            "SELECT session_type, timezone, source, content_sha256 \
             FROM trading_calendar_versions \
             WHERE exchange=$1 AND session_date=$2 AND source_version=$3",
        )
        .bind(&fact.exchange)
        .bind(fact.session_date.as_naive_date())
        .bind(&fact.source_version)
        .fetch_optional(&mut **tx)
        .await
        .map_err(SinkError::from_sqlx)?;
        if existing.is_none() {
            if replay {
                return Err(SinkError::Conflict(format!(
                    "published batch is missing calendar history for {} {} {}",
                    fact.exchange, fact.session_date, fact.source_version
                )));
            }
            sqlx::query(
                "INSERT INTO trading_calendar_versions \
                 (exchange, session_date, session_type, timezone, source, source_version, \
                  source_batch_id, content_sha256, retrieved_at) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) \
                 ON CONFLICT (exchange, session_date, source_version) DO NOTHING",
            )
            .bind(&fact.exchange)
            .bind(fact.session_date.as_naive_date())
            .bind(fact.session_type.as_db_str())
            .bind(&fact.timezone)
            .bind(&fact.source)
            .bind(&fact.source_version)
            .bind(bundle.source_batch_id.as_uuid())
            .bind(&fact.content_sha256)
            .bind(retrieved_at)
            .execute(&mut **tx)
            .await
            .map_err(SinkError::from_sqlx)?;
            existing = sqlx::query_as(
                "SELECT session_type, timezone, source, content_sha256 \
                 FROM trading_calendar_versions \
                 WHERE exchange=$1 AND session_date=$2 AND source_version=$3",
            )
            .bind(&fact.exchange)
            .bind(fact.session_date.as_naive_date())
            .bind(&fact.source_version)
            .fetch_optional(&mut **tx)
            .await
            .map_err(SinkError::from_sqlx)?;
        }
        let existing = existing.ok_or_else(|| {
            SinkError::Invariant("calendar history insert did not produce a visible row".to_owned())
        })?;
        if !history_matches(&existing, fact) {
            return Err(SinkError::Conflict(format!(
                "calendar source version differs for {} {} {}",
                fact.exchange, fact.session_date, fact.source_version
            )));
        }
    }
    Ok(())
}

async fn lock_and_verify_source_versions(
    tx: &mut Transaction<'_, Postgres>,
    bundle: &PublicationBundle,
) -> Result<(), SinkError> {
    let keys: BTreeSet<_> = bundle
        .calendar_facts
        .iter()
        .map(|fact| (fact.exchange.as_str(), fact.source_version.as_str()))
        .collect();
    // Take every source-version lock first, so concurrent publishers serialize
    // on the same keys regardless of which sessions each one carries.
    for (exchange, source_version) in keys {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1), hashtext($2))")
            .bind(exchange)
            .bind(source_version)
            .execute(&mut **tx)
            .await
            .map_err(SinkError::from_sqlx)?;
    }
    // Compare within one session date, not across all of them.
    //
    // A source version used to be required to name exactly one document. That
    // holds for a published calendar file -- KRX's yearly one covers many dates
    // and every row repeats its hash -- but KIS `chk-holiday` is normalized to
    // the date requested, so each day is a different document while
    // `calendar_id` and `schema_version`, the only two inputs to the version
    // string, stay constant. Every second session therefore conflicted with the
    // first and the pipeline could publish exactly one day, forever.
    //
    // Scoping to the session date keeps what the check is for: if the calendar
    // for a date we already recorded comes back with different bytes, that is
    // still caught, for a per-date source and a yearly file alike -- a rewritten
    // file changes the hash on each of its dates. What it stops asserting is
    // that two dates must share one document, which is the one thing a per-date
    // source can never satisfy. The table's own UNIQUE constraint has always
    // been (exchange, session_date, source_version); this cross-date rule lived
    // only here.
    for fact in &bundle.calendar_facts {
        let mismatch: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM trading_calendar_versions \
             WHERE exchange=$1 AND source_version=$2 AND session_date=$3 \
               AND (source <> $4 OR timezone <> $5 OR content_sha256 <> $6))",
        )
        .bind(&fact.exchange)
        .bind(&fact.source_version)
        .bind(fact.session_date.as_naive_date())
        .bind(&fact.source)
        .bind(&fact.timezone)
        .bind(&fact.content_sha256)
        .fetch_one(&mut **tx)
        .await
        .map_err(SinkError::from_sqlx)?;
        if mismatch {
            return Err(SinkError::Conflict(format!(
                "calendar source version differs for {} {} {}",
                fact.exchange,
                fact.source_version,
                fact.session_date.to_iso()
            )));
        }
    }
    Ok(())
}

async fn locked_projection(
    tx: &mut Transaction<'_, Postgres>,
    fact: &CalendarFact,
) -> Result<Option<ExistingProjectionRow>, SinkError> {
    sqlx::query_as(
        "SELECT session_type, timezone, source, source_version, content_sha256, retrieved_at \
         FROM trading_calendars WHERE exchange=$1 AND session_date=$2 FOR UPDATE",
    )
    .bind(&fact.exchange)
    .bind(fact.session_date.as_naive_date())
    .fetch_optional(&mut **tx)
    .await
    .map_err(SinkError::from_sqlx)
}

async fn projection_has_history(
    tx: &mut Transaction<'_, Postgres>,
    fact: &CalendarFact,
    projection: &ExistingProjectionRow,
) -> Result<bool, SinkError> {
    let Some(content_sha256) = projection.content_sha256.as_deref() else {
        return Ok(true);
    };
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM trading_calendar_versions \
         WHERE exchange=$1 AND session_date=$2 AND session_type=$3 AND timezone=$4 \
           AND source=$5 AND source_version=$6 AND content_sha256=$7)",
    )
    .bind(&fact.exchange)
    .bind(fact.session_date.as_naive_date())
    .bind(&projection.session_type)
    .bind(&projection.timezone)
    .bind(&projection.source)
    .bind(&projection.source_version)
    .bind(content_sha256)
    .fetch_one(&mut **tx)
    .await
    .map_err(SinkError::from_sqlx)
}

async fn verify_or_advance_projections(
    tx: &mut Transaction<'_, Postgres>,
    bundle: &PublicationBundle,
    replay: bool,
    retrieved_at: DateTime<Utc>,
) -> Result<(), SinkError> {
    for fact in &bundle.calendar_facts {
        let mut projection = locked_projection(tx, fact).await?;
        if projection.is_none() {
            if replay {
                return Err(SinkError::Conflict(format!(
                    "published batch is missing calendar projection for {} {}",
                    fact.exchange, fact.session_date
                )));
            }
            sqlx::query(
                "INSERT INTO trading_calendars \
                 (exchange, session_date, session_type, timezone, source, source_version, \
                  source_batch_id, content_sha256, retrieved_at) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) \
                 ON CONFLICT (exchange, session_date) DO NOTHING",
            )
            .bind(&fact.exchange)
            .bind(fact.session_date.as_naive_date())
            .bind(fact.session_type.as_db_str())
            .bind(&fact.timezone)
            .bind(&fact.source)
            .bind(&fact.source_version)
            .bind(bundle.source_batch_id.as_uuid())
            .bind(&fact.content_sha256)
            .bind(retrieved_at)
            .execute(&mut **tx)
            .await
            .map_err(|error| semantic_conflict(error, "calendar projection insert conflict"))?;
            projection = locked_projection(tx, fact).await?;
        }
        let projection = projection.ok_or_else(|| {
            SinkError::Invariant(
                "calendar projection insert did not produce a visible row".to_owned(),
            )
        })?;
        if !projection_has_history(tx, fact, &projection).await? {
            return Err(SinkError::Conflict(format!(
                "calendar projection has no matching history for {} {}",
                fact.exchange, fact.session_date
            )));
        }
        let incoming_time = retrieved_at;
        match projection.retrieved_at {
            None if replay => {
                return Err(SinkError::Conflict(
                    "published batch has only a legacy calendar projection".to_owned(),
                ));
            }
            None => update_projection(tx, bundle, fact, retrieved_at).await?,
            Some(existing_time)
                if incoming_time > canonical_retrieved_at(existing_time) && replay =>
            {
                return Err(SinkError::Conflict(
                    "published batch projection is older than its evidence".to_owned(),
                ));
            }
            Some(existing_time) if incoming_time > canonical_retrieved_at(existing_time) => {
                update_projection(tx, bundle, fact, retrieved_at).await?
            }
            Some(existing_time) if incoming_time == canonical_retrieved_at(existing_time) => {
                if !projection_matches(&projection, fact) {
                    return Err(SinkError::Conflict(format!(
                        "equal-time calendar facts differ for {} {}",
                        fact.exchange, fact.session_date
                    )));
                }
            }
            Some(_) => {}
        }
    }
    Ok(())
}

async fn update_projection(
    tx: &mut Transaction<'_, Postgres>,
    bundle: &PublicationBundle,
    fact: &CalendarFact,
    retrieved_at: DateTime<Utc>,
) -> Result<(), SinkError> {
    sqlx::query(
        "UPDATE trading_calendars SET session_type=$3, timezone=$4, source=$5, \
         source_version=$6, source_batch_id=$7, content_sha256=$8, retrieved_at=$9 \
         WHERE exchange=$1 AND session_date=$2",
    )
    .bind(&fact.exchange)
    .bind(fact.session_date.as_naive_date())
    .bind(fact.session_type.as_db_str())
    .bind(&fact.timezone)
    .bind(&fact.source)
    .bind(&fact.source_version)
    .bind(bundle.source_batch_id.as_uuid())
    .bind(&fact.content_sha256)
    .bind(retrieved_at)
    .execute(&mut **tx)
    .await
    .map_err(SinkError::from_sqlx)?;
    Ok(())
}

#[async_trait]
impl PublicationSink for PostgresPublicationSink {
    async fn publication_state(&self, batch_id: BatchId) -> Result<PublicationState, SinkError> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT kind, source_file_name FROM data_batches \
             WHERE provider=$1 AND market=$2 AND source_batch_id=$3",
        )
        .bind(DB_PROVIDER)
        .bind(DB_MARKET)
        .bind(batch_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(SinkError::from_sqlx)?;
        Ok(if rows.is_empty() {
            PublicationState::Missing
        } else if exact_kind_shape(&rows) {
            PublicationState::Complete
        } else {
            PublicationState::Partial
        })
    }

    async fn publish(&self, bundle: &PublicationBundle) -> Result<PublishOutcome, SinkError> {
        validate_bundle(bundle)?;
        let retrieved_at = postgres_retrieved_at(bundle.retrieved_at);
        let mut tx = self.pool.begin().await.map_err(SinkError::from_sqlx)?;
        let lock_key = i64::from_be_bytes(
            bundle.source_batch_id.as_uuid().as_bytes()[..8]
                .try_into()
                .expect("UUID has at least eight bytes"),
        );
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(lock_key)
            .execute(&mut *tx)
            .await
            .map_err(SinkError::from_sqlx)?;

        let rows = load_batch_rows(&mut tx, bundle.source_batch_id).await?;
        let replay = !rows.is_empty();
        if replay {
            batch_rows_match(&rows, bundle, retrieved_at)?;
        } else {
            insert_batch_rows(&mut tx, bundle, retrieved_at).await?;
        }
        verify_or_insert_history(&mut tx, bundle, replay, retrieved_at).await?;
        verify_or_advance_projections(&mut tx, bundle, replay, retrieved_at).await?;
        tx.commit().await.map_err(SinkError::from_sqlx)?;
        Ok(if replay {
            PublishOutcome::AlreadyPublished
        } else {
            PublishOutcome::Published
        })
    }

    async fn has_eod(&self, date: TradingDate) -> Result<bool, SinkError> {
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM data_batches \
             WHERE provider=$1 AND market=$2 AND batch_date=$3 AND kind='EOD')",
        )
        .bind(DB_PROVIDER)
        .bind(DB_MARKET)
        .bind(date.as_naive_date())
        .fetch_one(&self.pool)
        .await
        .map_err(SinkError::from_sqlx)
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use market_data::contract::{
        FetchMode, MARKET_KR, PROVIDER_KIS, PROVIDER_KIS_NORMALIZED, PROVIDER_KRX,
    };

    use super::{
        DB_MARKET, DB_PROVIDER, PublicationScope, SinkError, exact_normalized_file_shape,
        publication_scope, sqlstate_is_retryable,
    };

    #[test]
    fn sqlstate_retry_classification_is_structural_and_stable() {
        assert!(sqlstate_is_retryable("08006"));
        assert!(sqlstate_is_retryable("40P01"));
        assert!(sqlstate_is_retryable("40001"));
        assert!(sqlstate_is_retryable("55P03"));
        assert!(sqlstate_is_retryable("57014"));
        assert!(!sqlstate_is_retryable("23505"));
        assert!(!sqlstate_is_retryable("23514"));
    }

    #[test]
    fn sqlx_error_classes_preserve_sources_with_sanitized_display() {
        let retryable = SinkError::from_sqlx(sqlx::Error::PoolClosed);
        assert!(matches!(&retryable, SinkError::RetryableDatabase(_)));
        assert!(retryable.source().is_some());
        assert_eq!(retryable.to_string(), "retryable database failure");

        let permanent = SinkError::from_sqlx(sqlx::Error::RowNotFound);
        assert!(matches!(&permanent, SinkError::PermanentDatabase(_)));
        assert!(!permanent.is_retryable());
        assert!(permanent.source().is_some());
        assert_eq!(permanent.to_string(), "permanent database failure");
    }

    #[test]
    fn raw_publication_scopes_map_to_the_legacy_database_scope() {
        let legacy = publication_scope(PROVIDER_KRX, MARKET_KR).expect("legacy scope");
        let normalized =
            publication_scope(PROVIDER_KIS_NORMALIZED, MARKET_KR).expect("normalized scope");

        assert_eq!(legacy, PublicationScope::Krx);
        assert_eq!(normalized, PublicationScope::KisNormalized);
        assert_eq!(legacy.database_scope().provider, DB_PROVIDER);
        assert_eq!(legacy.database_scope().market, DB_MARKET);
        assert_eq!(normalized.database_scope().provider, DB_PROVIDER);
        assert_eq!(normalized.database_scope().market, DB_MARKET);
        assert_eq!(legacy.required_mode(), None);
        assert_eq!(normalized.required_mode(), Some(FetchMode::Credentialed));
        assert!(publication_scope(PROVIDER_KIS, MARKET_KR).is_none());
        assert!(publication_scope(PROVIDER_KRX, "other").is_none());
    }

    #[test]
    fn normalized_file_shape_requires_canonical_names_and_kinds() {
        let files = [
            super::PublicationFile {
                file_name: "bars.json".to_owned(),
                kind: super::DataBatchKind::Eod,
                content_sha256: "a".repeat(64),
                storage_path: "bars".to_owned(),
                bytes_size: 1,
            },
            super::PublicationFile {
                file_name: "reference.json".to_owned(),
                kind: super::DataBatchKind::Reference,
                content_sha256: "b".repeat(64),
                storage_path: "reference".to_owned(),
                bytes_size: 1,
            },
            super::PublicationFile {
                file_name: "calendar.json".to_owned(),
                kind: super::DataBatchKind::Calendar,
                content_sha256: "c".repeat(64),
                storage_path: "calendar".to_owned(),
                bytes_size: 1,
            },
            super::PublicationFile {
                file_name: "corporate-actions.json".to_owned(),
                kind: super::DataBatchKind::CorporateActions,
                content_sha256: "d".repeat(64),
                storage_path: "actions".to_owned(),
                bytes_size: 1,
            },
        ];
        assert!(exact_normalized_file_shape(&files));

        let mut holiday = files.to_vec();
        holiday[0].kind = super::DataBatchKind::EodUnavailable;
        assert!(exact_normalized_file_shape(&holiday));

        let mut noncanonical = files.to_vec();
        noncanonical[0].file_name = "bars-response.json".to_owned();
        assert!(!exact_normalized_file_shape(&noncanonical));
    }
}
