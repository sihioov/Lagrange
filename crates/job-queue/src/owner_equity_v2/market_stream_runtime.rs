//! Safe runtime configuration, evidence-backed KRX day inputs and owned supervision.
//!
//! Immutable startup pins and existing local/database evidence govern the daemon.
//! Private supervision preserves transport, lease, observation and cleanup ownership;
//! storage still rechecks resolved evidence before publication.

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveTime, SecondsFormat, TimeZone, Timelike, Utc};
use collectors::intraday_quotes::{
    IntradaySessionDisposition, IntradaySessionWindowContract, IntradaySessionWindowSource,
};
use market_data::{FIXED_30_ID_LIST_SHA256, STREAM_WIRE_VERSION};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use super::intraday::{
    IntradaySessionProof, IntradaySessionWindow, IntradayStorageError, MarketStreamCalendarReader,
    OwnerIntradayQuoteRepository,
};
use super::market_stream::{MarketStreamStorageError, StreamSessionProof};

/// Startup pins supplied by the owner-only immutable runtime configuration.
/// Secret material and filesystem paths are deliberately absent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerMarketStreamRuntimeConfig {
    slot: Uuid,
    grant: Uuid,
    contract_sha256: String,
    credential_generation: u64,
    holder: Uuid,
}

impl OwnerMarketStreamRuntimeConfig {
    pub fn from_values(
        slot: Uuid,
        grant: Uuid,
        contract_sha256: &str,
        credential_generation: u64,
        holder: Uuid,
    ) -> Result<Self, MarketStreamRuntimeError> {
        if slot.is_nil()
            || grant.is_nil()
            || holder.is_nil()
            || credential_generation == 0
            || !canonical_unprefixed_sha256(contract_sha256)
        {
            return Err(MarketStreamRuntimeError::ConfigurationInvalid);
        }
        Ok(Self {
            slot,
            grant,
            contract_sha256: contract_sha256.to_owned(),
            credential_generation,
            holder,
        })
    }

    pub(super) const fn slot(&self) -> Uuid {
        self.slot
    }

    pub(super) const fn grant(&self) -> Uuid {
        self.grant
    }

    pub(super) fn contract_sha256(&self) -> &str {
        &self.contract_sha256
    }

    pub(super) const fn credential_generation(&self) -> u64 {
        self.credential_generation
    }

    pub(super) const fn holder(&self) -> Uuid {
        self.holder
    }
}

/// Finite, redacted startup/day-resolution errors. Provider and SQL text never
/// enter this type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MarketStreamRuntimeError {
    #[error("MARKET_STREAM_RUNTIME_CONFIGURATION_INVALID")]
    ConfigurationInvalid,
    #[error("MARKET_STREAM_RUNTIME_GRANT_UNAVAILABLE")]
    GrantUnavailable,
    #[error("MARKET_STREAM_RUNTIME_STORAGE_UNAVAILABLE")]
    StorageUnavailable,
    #[error("MARKET_STREAM_RUNTIME_CALENDAR_UNAVAILABLE")]
    CalendarUnavailable,
    #[error("MARKET_STREAM_RUNTIME_WINDOW_UNAVAILABLE")]
    WindowUnavailable,
    #[error("MARKET_STREAM_RUNTIME_DAY_INVALID")]
    DayInvalid,
    #[error("MARKET_STREAM_RUNTIME_DEMAND_INVALID")]
    DemandInvalid,
    #[error("MARKET_STREAM_RUNTIME_LEASE_UNAVAILABLE")]
    LeaseUnavailable,
    #[error("MARKET_STREAM_RUNTIME_TRANSPORT_UNAVAILABLE")]
    TransportUnavailable,
    #[error("MARKET_STREAM_RUNTIME_TERMINAL")]
    Terminal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DayResolution {
    MissingCalendar,
    MissingWindow,
    Closed,
    Open(ResolvedMarketStreamDay),
}

/// A validated current day. Its only constructor is the resolver below (plus
/// a private unit-test seam); fields and proof constructors stay private.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ResolvedMarketStreamDay {
    resolution_id: Uuid,
    calendar: IntradaySessionProof,
    session: StreamSessionProof,
    window: IntradaySessionWindow,
    transport: kis_client::market_stream_wire::MarketStreamSessionProof,
}

impl ResolvedMarketStreamDay {
    pub(super) const fn resolution_id(&self) -> Uuid {
        self.resolution_id
    }

    pub(super) fn session(&self) -> StreamSessionProof {
        self.session.clone()
    }

    pub(super) fn calendar(&self) -> &IntradaySessionProof {
        &self.calendar
    }

    pub(super) const fn session_date(&self) -> NaiveDate {
        self.window.session_date
    }

    pub(super) const fn open_at(&self) -> DateTime<Utc> {
        self.window.open_at
    }

    pub(super) const fn close_at(&self) -> DateTime<Utc> {
        self.window.close_at
    }

    pub(super) const fn transport_proof(
        &self,
    ) -> kis_client::market_stream_wire::MarketStreamSessionProof {
        self.transport
    }

    /// Reuse the correlation UUID only when every piece of resolved lineage
    /// and the canonical digest still match. The UUID itself is not evidence.
    pub(super) fn retain_same_lineage_correlation(&mut self, previous: &StreamSessionProof) {
        if previous.session_date == self.session.session_date
            && previous.calendar_source_batch_id == self.session.calendar_source_batch_id
            && previous.calendar_content_sha256 == self.session.calendar_content_sha256
            && previous.window_contract_sha256 == self.session.window_contract_sha256
            && previous.session_proof_sha256 == self.session.session_proof_sha256
        {
            self.session.session_proof_id = previous.session_proof_id;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SnapshotWindowEvidence {
    date: NaiveDate,
    open_at: DateTime<Utc>,
    close_at: DateTime<Utc>,
    contract_sha256: String,
}

#[cfg(test)]
pub(super) fn snapshot_window_fixture(
    date: NaiveDate,
    open_at: DateTime<Utc>,
    close_at: DateTime<Utc>,
    contract_sha256: String,
) -> SnapshotWindowEvidence {
    SnapshotWindowEvidence {
        date,
        open_at,
        close_at,
        contract_sha256,
    }
}

impl SnapshotWindowEvidence {
    pub(super) const fn date(&self) -> NaiveDate {
        self.date
    }

    pub(super) const fn open_at(&self) -> DateTime<Utc> {
        self.open_at
    }

    pub(super) const fn close_at(&self) -> DateTime<Utc> {
        self.close_at
    }

    pub(super) fn contract_sha256(&self) -> &str {
        &self.contract_sha256
    }

    pub(super) fn contains(&self, now: DateTime<Utc>) -> bool {
        self.open_at <= now && now < self.close_at
    }
}

pub(super) fn load_snapshot_window_contract() -> Option<IntradaySessionWindowContract> {
    IntradaySessionWindowContract::from_configured_source().ok()
}

pub(super) fn snapshot_window_for(
    contract: Option<&IntradaySessionWindowContract>,
    date: NaiveDate,
) -> Option<SnapshotWindowEvidence> {
    let contract = contract?;
    let entry = contract.entry(date)?;
    if entry.disposition == IntradaySessionDisposition::Closed {
        return None;
    }
    let (open_at, close_at) = entry.utc_bounds()?;
    Some(SnapshotWindowEvidence {
        date,
        open_at,
        close_at,
        contract_sha256: contract.window_contract_sha256().to_owned(),
    })
}

pub(super) async fn resolve_day(
    owner_user_id: Uuid,
    calendar: &OwnerIntradayQuoteRepository,
    windows_source: IntradaySessionWindowSource,
) -> Result<DayResolution, MarketStreamRuntimeError> {
    resolve_day_with_calendar(
        owner_user_id,
        DayResolutionCalendar::Legacy(calendar),
        windows_source,
    )
    .await
    .map_err(map_legacy_day_resolution_error)
}

pub(super) async fn resolve_day_for_runtime(
    owner_user_id: Uuid,
    calendar: MarketStreamCalendarReader<'_>,
    windows_source: IntradaySessionWindowSource,
) -> Result<DayResolution, MarketStreamStorageError> {
    resolve_day_with_calendar(
        owner_user_id,
        DayResolutionCalendar::Runtime(calendar),
        windows_source,
    )
    .await
    .map_err(map_runtime_day_resolution_error)
}

enum DayResolutionCalendar<'a> {
    Legacy(&'a OwnerIntradayQuoteRepository),
    Runtime(MarketStreamCalendarReader<'a>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DayResolutionFailure {
    CalendarStorage(IntradayStorageError),
    RuntimeValidation(MarketStreamRuntimeError),
}

async fn calendar_database_time(
    calendar: &DayResolutionCalendar<'_>,
) -> Result<DateTime<Utc>, DayResolutionFailure> {
    match calendar {
        DayResolutionCalendar::Legacy(repository) => repository
            .current_database_time()
            .await
            .map_err(DayResolutionFailure::CalendarStorage),
        DayResolutionCalendar::Runtime(reader) => reader
            .current_database_time()
            .await
            .map_err(DayResolutionFailure::CalendarStorage),
    }
}

async fn calendar_session_proof(
    calendar: &DayResolutionCalendar<'_>,
    owner_user_id: Uuid,
    window_contract_sha256: &str,
) -> Result<Option<IntradaySessionProof>, DayResolutionFailure> {
    match calendar {
        DayResolutionCalendar::Legacy(repository) => repository
            .resolve_current_session_proof(owner_user_id, window_contract_sha256)
            .await
            .map_err(DayResolutionFailure::CalendarStorage),
        DayResolutionCalendar::Runtime(reader) => reader
            .resolve_current_session_proof(owner_user_id, window_contract_sha256)
            .await
            .map_err(DayResolutionFailure::CalendarStorage),
    }
}

async fn resolve_day_with_calendar(
    owner_user_id: Uuid,
    calendar: DayResolutionCalendar<'_>,
    windows_source: IntradaySessionWindowSource,
) -> Result<DayResolution, DayResolutionFailure> {
    if owner_user_id.is_nil() {
        return Err(DayResolutionFailure::RuntimeValidation(
            MarketStreamRuntimeError::DayInvalid,
        ));
    }
    let initial_now = calendar_database_time(&calendar).await?;
    let initial_date = kst_date(initial_now).map_err(DayResolutionFailure::RuntimeValidation)?;
    #[cfg(feature = "market-stream-db-tests")]
    let window_result = integration_fixture_window::load_window_contract(windows_source);
    #[cfg(not(feature = "market-stream-db-tests"))]
    let window_result = IntradaySessionWindowContract::from_source(windows_source);
    let windows = match window_result {
        Ok(windows) => windows,
        Err(_) => return Ok(DayResolution::MissingWindow),
    };
    let entry = match windows.entry(initial_date) {
        Some(entry) => entry,
        None => return Ok(DayResolution::MissingWindow),
    };
    if entry.disposition == IntradaySessionDisposition::Closed {
        return Ok(DayResolution::Closed);
    }
    let Some((open_at, close_at)) = entry.utc_bounds() else {
        return Ok(DayResolution::MissingWindow);
    };

    let calendar_proof =
        match calendar_session_proof(&calendar, owner_user_id, windows.window_contract_sha256())
            .await?
        {
            Some(proof) => proof,
            None => return Ok(DayResolution::MissingCalendar),
        };
    let fresh_now = calendar_database_time(&calendar).await?;
    if calendar_proof.session_date != initial_date
        || kst_date(fresh_now).map_err(DayResolutionFailure::RuntimeValidation)? != initial_date
    {
        return Ok(DayResolution::MissingCalendar);
    }
    if !(open_at <= fresh_now && fresh_now < close_at) {
        return Ok(DayResolution::Closed);
    }
    let window = IntradaySessionWindow::new(initial_date, open_at, close_at).map_err(|_| {
        DayResolutionFailure::RuntimeValidation(MarketStreamRuntimeError::WindowUnavailable)
    })?;
    let session_proof_sha256 = canonical_day_proof_sha256(
        initial_date,
        calendar_proof.calendar_source_batch_id,
        &calendar_proof.calendar_content_sha256,
        windows.window_contract_sha256(),
        open_at,
        close_at,
    );
    let session = StreamSessionProof::new(
        initial_date,
        Uuid::new_v4(),
        session_proof_sha256,
        calendar_proof.calendar_source_batch_id,
        calendar_proof.calendar_content_sha256.clone(),
        windows.window_contract_sha256().to_owned(),
    )
    .map_err(|_| DayResolutionFailure::RuntimeValidation(MarketStreamRuntimeError::DayInvalid))?;
    let transport = make_transport_proof(initial_date, entry.open_local, entry.close_local)
        .map_err(DayResolutionFailure::RuntimeValidation)?;
    Ok(DayResolution::Open(ResolvedMarketStreamDay {
        resolution_id: Uuid::new_v4(),
        calendar: calendar_proof,
        session,
        window,
        transport,
    }))
}

fn map_legacy_day_resolution_error(error: DayResolutionFailure) -> MarketStreamRuntimeError {
    match error {
        DayResolutionFailure::CalendarStorage(_) => MarketStreamRuntimeError::StorageUnavailable,
        DayResolutionFailure::RuntimeValidation(error) => error,
    }
}

pub(super) fn map_runtime_calendar_storage_error(
    error: IntradayStorageError,
) -> MarketStreamStorageError {
    match error {
        IntradayStorageError::CommitUnknown => MarketStreamStorageError::CommitUnknown,
        _ => MarketStreamStorageError::DatabaseUnavailable,
    }
}

fn map_runtime_day_resolution_error(error: DayResolutionFailure) -> MarketStreamStorageError {
    match error {
        DayResolutionFailure::CalendarStorage(error) => map_runtime_calendar_storage_error(error),
        DayResolutionFailure::RuntimeValidation(_) => MarketStreamStorageError::InvalidInput,
    }
}

pub(super) fn canonical_day_proof_sha256(
    date: NaiveDate,
    calendar_batch_id: Uuid,
    calendar_content_sha256: &str,
    window_contract_sha256: &str,
    open_at: DateTime<Utc>,
    close_at: DateTime<Utc>,
) -> String {
    let canonical = format!(
        "owner-market-stream-day-v1\n{date}\n{calendar_batch_id}\n{}\n{window_contract_sha256}\n{}\n{}\n",
        calendar_content_sha256.to_ascii_lowercase(),
        open_at.to_rfc3339_opts(SecondsFormat::Secs, true),
        close_at.to_rfc3339_opts(SecondsFormat::Secs, true),
    );
    format!("sha256:{:x}", Sha256::digest(canonical.as_bytes()))
}

pub(super) fn fixed_stream_contract_is_canonical(
    network_contract_sha256: &str,
    identity_list_sha256: &str,
    wire_version: &str,
) -> bool {
    canonical_unprefixed_sha256(network_contract_sha256)
        && network_contract_sha256 == network_contract_sha256.to_ascii_lowercase()
        && identity_list_sha256 == FIXED_30_ID_LIST_SHA256
        && wire_version == STREAM_WIRE_VERSION
}

fn make_transport_proof(
    date: NaiveDate,
    open_local: Option<NaiveTime>,
    close_local: Option<NaiveTime>,
) -> Result<kis_client::market_stream_wire::MarketStreamSessionProof, MarketStreamRuntimeError> {
    let (Some(open), Some(close)) = (open_local, close_local) else {
        return Err(MarketStreamRuntimeError::WindowUnavailable);
    };
    let date_number = date
        .format("%Y%m%d")
        .to_string()
        .parse::<u32>()
        .map_err(|_| MarketStreamRuntimeError::DayInvalid)?;
    let kst = FixedOffset::east_opt(9 * 60 * 60).ok_or(MarketStreamRuntimeError::DayInvalid)?;
    let midnight = kst
        .from_local_datetime(&date.and_time(NaiveTime::MIN))
        .single()
        .ok_or(MarketStreamRuntimeError::DayInvalid)?
        .with_timezone(&Utc)
        .timestamp_millis();
    kis_client::market_stream_wire::MarketStreamSessionProof::new(
        date_number,
        hms(open),
        hms(close),
        midnight,
    )
    .map_err(|_| MarketStreamRuntimeError::WindowUnavailable)
}

fn hms(time: NaiveTime) -> u32 {
    time.hour() * 10_000 + time.minute() * 100 + time.second()
}

fn kst_date(now: DateTime<Utc>) -> Result<NaiveDate, MarketStreamRuntimeError> {
    Ok(now
        .with_timezone(
            &FixedOffset::east_opt(9 * 60 * 60).ok_or(MarketStreamRuntimeError::DayInvalid)?,
        )
        .date_naive())
}

fn canonical_unprefixed_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
pub(super) fn fixture_day(
    calendar: IntradaySessionProof,
    open_at: DateTime<Utc>,
    close_at: DateTime<Utc>,
    proof_id: Uuid,
) -> Result<ResolvedMarketStreamDay, MarketStreamRuntimeError> {
    let window = IntradaySessionWindow::new(calendar.session_date, open_at, close_at)
        .map_err(|_| MarketStreamRuntimeError::WindowUnavailable)?;
    let digest = canonical_day_proof_sha256(
        calendar.session_date,
        calendar.calendar_source_batch_id,
        &calendar.calendar_content_sha256,
        &calendar.window_contract_sha256,
        open_at,
        close_at,
    );
    let session = StreamSessionProof::new(
        calendar.session_date,
        proof_id,
        digest,
        calendar.calendar_source_batch_id,
        calendar.calendar_content_sha256.clone(),
        calendar.window_contract_sha256.clone(),
    )
    .map_err(|_| MarketStreamRuntimeError::DayInvalid)?;
    let transport = make_transport_proof(
        calendar.session_date,
        Some(
            open_at
                .with_timezone(&FixedOffset::east_opt(9 * 60 * 60).unwrap())
                .time(),
        ),
        Some(
            close_at
                .with_timezone(&FixedOffset::east_opt(9 * 60 * 60).unwrap())
                .time(),
        ),
    )?;
    Ok(ResolvedMarketStreamDay {
        resolution_id: Uuid::new_v4(),
        calendar,
        session,
        window,
        transport,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_day_hash_includes_each_line_and_final_newline() {
        let date = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let batch = Uuid::parse_str("11111111-2222-4333-8444-555555555555").unwrap();
        let open = DateTime::parse_from_rfc3339("2026-10-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let close = DateTime::parse_from_rfc3339("2026-10-01T06:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let expected = format!(
            "owner-market-stream-day-v1\n2026-10-01\n{batch}\n{}\nsha256:{}\n2026-10-01T00:00:00Z\n2026-10-01T06:30:00Z\n",
            "a".repeat(64),
            "b".repeat(64),
        );
        assert_eq!(
            canonical_day_proof_sha256(
                date,
                batch,
                &"A".repeat(64),
                &format!("sha256:{}", "b".repeat(64)),
                open,
                close,
            ),
            format!("sha256:{:x}", Sha256::digest(expected.as_bytes()))
        );
        assert_ne!(
            canonical_day_proof_sha256(
                date,
                batch,
                &"a".repeat(64),
                &format!("sha256:{}", "b".repeat(64)),
                open,
                close,
            ),
            format!(
                "sha256:{:x}",
                Sha256::digest(expected.trim_end().as_bytes())
            )
        );
    }

    #[test]
    fn runtime_config_requires_non_nil_pins_positive_generation_and_canonical_hash() {
        let slot = Uuid::new_v4();
        let grant = Uuid::new_v4();
        let holder = Uuid::new_v4();
        let hash = "a".repeat(64);
        assert!(OwnerMarketStreamRuntimeConfig::from_values(slot, grant, &hash, 1, holder).is_ok());
        assert_eq!(
            OwnerMarketStreamRuntimeConfig::from_values(Uuid::nil(), grant, &hash, 1, holder),
            Err(MarketStreamRuntimeError::ConfigurationInvalid)
        );
        assert_eq!(
            OwnerMarketStreamRuntimeConfig::from_values(
                slot,
                grant,
                &hash.to_ascii_uppercase(),
                1,
                holder
            ),
            Err(MarketStreamRuntimeError::ConfigurationInvalid)
        );
        assert_eq!(
            OwnerMarketStreamRuntimeConfig::from_values(slot, grant, &hash, 0, holder),
            Err(MarketStreamRuntimeError::ConfigurationInvalid)
        );
    }

    #[test]
    fn snapshot_window_is_half_open_and_uses_the_contract_hash() {
        let date = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let bytes = synthetic_window(date, "09:00:00", "09:01:00");
        let digest = format!("sha256:{:x}", Sha256::digest(&bytes));
        let windows = IntradaySessionWindowContract::from_bytes(&bytes, &digest).unwrap();
        let evidence = snapshot_window_for(Some(&windows), date).unwrap();
        assert_eq!(evidence.contract_sha256(), digest);
        assert!(evidence.contains(evidence.open_at()));
        assert!(!evidence.contains(evidence.close_at()));
    }

    fn synthetic_window(date: NaiveDate, open: &str, close: &str) -> Vec<u8> {
        format!(
            "{{\"schema_version\":1,\"exchange\":\"KRX\",\"timezone\":\"Asia/Seoul\",\"entries\":[{{\"date\":\"{date}\",\"disposition\":\"SPECIAL\",\"open_local\":\"{open}\",\"close_local\":\"{close}\",\"evidence_url\":\"https://global.krx.co.kr/synthetic\",\"evidence_retrieved_at\":\"2026-10-01T00:00:00Z\",\"evidence_sha256\":\"sha256:{}\"}}]}}",
            "c".repeat(64)
        )
        .into_bytes()
    }
}

#[cfg(test)]
mod runtime_calendar_tests {
    use super::{
        DayResolutionFailure, MarketStreamRuntimeError, map_legacy_day_resolution_error,
        map_runtime_calendar_storage_error, map_runtime_day_resolution_error,
    };
    use crate::owner_equity_v2::intraday::IntradayStorageError;
    use crate::owner_equity_v2::market_stream::MarketStreamStorageError;

    #[test]
    fn runtime_day_error_mapping_preserves_commit_unknown() {
        assert_eq!(
            map_runtime_calendar_storage_error(IntradayStorageError::CommitUnknown),
            MarketStreamStorageError::CommitUnknown
        );
        for error in [
            IntradayStorageError::DatabaseUnavailable,
            IntradayStorageError::InvalidInput,
        ] {
            assert_eq!(
                map_runtime_calendar_storage_error(error),
                MarketStreamStorageError::DatabaseUnavailable
            );
        }

        assert_eq!(
            map_legacy_day_resolution_error(DayResolutionFailure::CalendarStorage(
                IntradayStorageError::CommitUnknown,
            )),
            MarketStreamRuntimeError::StorageUnavailable
        );
        assert_eq!(
            map_legacy_day_resolution_error(DayResolutionFailure::CalendarStorage(
                IntradayStorageError::DatabaseUnavailable,
            )),
            MarketStreamRuntimeError::StorageUnavailable
        );
        assert_eq!(
            map_legacy_day_resolution_error(DayResolutionFailure::RuntimeValidation(
                MarketStreamRuntimeError::DayInvalid,
            )),
            MarketStreamRuntimeError::DayInvalid
        );
        assert_eq!(
            map_legacy_day_resolution_error(DayResolutionFailure::RuntimeValidation(
                MarketStreamRuntimeError::WindowUnavailable,
            )),
            MarketStreamRuntimeError::WindowUnavailable
        );
        assert_eq!(
            map_runtime_day_resolution_error(DayResolutionFailure::RuntimeValidation(
                MarketStreamRuntimeError::WindowUnavailable,
            )),
            MarketStreamStorageError::InvalidInput
        );
    }
}
/// A runtime whose operational capabilities remain private to its owner.
///
/// Construction validates only immutable metadata and client configuration.
/// No approval request, connection reservation, or database operation occurs
/// during construction. The polled runtime performs bounded control reads
/// while waiting; positive demand is required before reservation and claim.
pub struct OwnerMarketStreamRuntime {
    client: kis_client::MarketStreamClient,
    repository: super::market_stream::RuntimeMarketStreamRepository,
    calendar: super::intraday::OwnerIntradayQuoteRepository,
    windows_source: collectors::intraday_quotes::IntradaySessionWindowSource,
    config: OwnerMarketStreamRuntimeConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketStreamRuntimeExit {
    Shutdown,
}

impl OwnerMarketStreamRuntime {
    pub fn new(
        repository: super::market_stream::OwnerMarketStreamRepository,
        calendar: super::intraday::OwnerIntradayQuoteRepository,
        windows: collectors::intraday_quotes::IntradaySessionWindowSource,
        approval: kis_client::market_stream_approval::ApprovalClient,
        config: OwnerMarketStreamRuntimeConfig,
    ) -> Result<Self, MarketStreamRuntimeError> {
        if !approval.matches_runtime_binding(config.slot(), config.credential_generation()) {
            return Err(MarketStreamRuntimeError::ConfigurationInvalid);
        }
        let client = kis_client::MarketStreamClient::production(approval)
            .map_err(|_| MarketStreamRuntimeError::TransportUnavailable)?;
        let repository = repository.runtime_repository();
        Ok(Self {
            client,
            repository,
            calendar,
            windows_source: windows,
            config,
        })
    }

    pub async fn run_daemon(
        self,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) -> Result<MarketStreamRuntimeExit, MarketStreamRuntimeError> {
        if runtime_supervisor::shutdown_requested(&mut shutdown) {
            return Ok(MarketStreamRuntimeExit::Shutdown);
        }
        let grant = self
            .repository
            .load_runtime_grant(&self.config)
            .await
            .map_err(runtime_supervisor::map_storage_error)?;
        if runtime_supervisor::shutdown_requested(&mut shutdown) {
            return Ok(MarketStreamRuntimeExit::Shutdown);
        }
        runtime_supervisor::run(self, grant, shutdown).await
    }
}

mod runtime_supervisor {
    use super::{
        DayResolution, MarketStreamRuntimeError, OwnerMarketStreamRuntime, ResolvedMarketStreamDay,
    };
    use crate::owner_equity_v2::market_stream::{
        DesiredSet, MarketStreamStorageError, RuntimeGrant, RuntimeMarketStreamRepository,
        RuntimeMarketStreamStorageError, RuntimeStatusCommit, RuntimeTransition,
        StreamProducerLease, StreamSessionProof,
    };
    use crate::owner_equity_v2::market_stream_producer::{
        MarketStreamProducerError, OwnerMarketStreamProducer, RuntimeOwnedExit,
        RuntimeOwnedProducer, normalize_demand,
    };
    use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
    use kis_client::market_stream::{MarketStreamError, MarketStreamSession};
    use kis_client::market_stream_state::StateError;
    use std::collections::{BTreeMap, BTreeSet};
    use std::future::Future;
    use std::pin::Pin;
    use std::time::{Duration, Instant};
    use tokio::sync::watch;

    pub(super) const OBSERVATION_SPACING: Duration = Duration::from_secs(1);
    pub(super) const RENEWAL_SPACING: Duration = Duration::from_secs(5);
    pub(super) const MIN_LEASE_MARGIN: chrono::Duration = chrono::Duration::seconds(5);
    const CONNECT_STOP_BOUND: Duration = Duration::from_secs(5);
    const CLOSE_BOUND: Duration = Duration::from_secs(5);

    pub(super) type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum ParentStopReason {
        ExternalShutdown,
        NoDemand,
        DayClosed,
        FreshEpochRequired,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Phase {
        Waiting,
        Connecting,
        StartingEpoch,
        Active,
        Closing,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum StageKind {
        Tick,
        Claim,
        Status,
        Connect,
        StartEpoch,
        RunOwned,
        Close,
    }

    enum StageResult {
        Tick,
        Claimed(Result<StreamProducerLease, RuntimeMarketStreamStorageError>),
        Status(
            RuntimeTransition,
            Result<RuntimeStatusCommit, RuntimeMarketStreamStorageError>,
        ),
        Connected(Result<MarketStreamSession, MarketStreamError>),
        Started(Result<RuntimeOwnedProducer, MarketStreamProducerError>),
        Owned(Result<RuntimeOwnedExit, MarketStreamProducerError>),
        Closed(Result<(), MarketStreamError>),
        ConnectTimedOut,
        CloseTimedOut,
    }

    struct Observation {
        started: Instant,
        completed: Instant,
        grant: Result<RuntimeGrant, RuntimeMarketStreamStorageError>,
        demand: Result<DesiredSet, RuntimeMarketStreamStorageError>,
        day: Result<DayResolution, RuntimeMarketStreamStorageError>,
        renewal: Option<Result<StreamProducerLease, RuntimeMarketStreamStorageError>>,
        renewal_requested: bool,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(super) struct DemandSnapshot {
        pub(super) raw: DesiredSet,
        normalized: BTreeMap<super::super::market_stream::StreamIdentity, u32>,
        pub(super) completed_at: Instant,
        pub(super) sequence: u64,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(super) struct Lineage {
        pub(super) session_date: NaiveDate,
        pub(super) calendar_batch_id: uuid::Uuid,
        pub(super) calendar_content_sha256: String,
        pub(super) window_contract_sha256: String,
        pub(super) session_proof_sha256: String,
        pub(super) open_at: DateTime<Utc>,
        pub(super) close_at: DateTime<Utc>,
    }

    struct CorrelationMemo {
        lineage: Lineage,
        proof: StreamSessionProof,
    }

    #[derive(Clone)]
    pub(super) struct LeaseAuthority {
        pub(super) slot: uuid::Uuid,
        pub(super) grant: uuid::Uuid,
        pub(super) revision: uuid::Uuid,
        pub(super) owner: uuid::Uuid,
        pub(super) holder: uuid::Uuid,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum LeaseRefreshDisposition {
        Forward,
        DrainWithoutForwarding,
        Invalid,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum DayReadiness {
        MissingCalendar,
        MissingWindow,
        Closed,
        Open,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) struct ConnectAdmission {
        pub(super) positive_demand: bool,
        pub(super) day: DayReadiness,
        pub(super) anchor_reserved: bool,
        pub(super) lease_valid: bool,
        pub(super) post_claim_observation: bool,
        pub(super) stopping: bool,
        pub(super) terminal: bool,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum OwnerCompletion {
        ReplaceAllowed,
        Shutdown,
        Terminal,
    }

    pub(super) fn map_storage_error(
        error: RuntimeMarketStreamStorageError,
    ) -> MarketStreamRuntimeError {
        match error {
            RuntimeMarketStreamStorageError::DeadlineExceeded
            | RuntimeMarketStreamStorageError::Terminal
            | RuntimeMarketStreamStorageError::Storage(MarketStreamStorageError::CommitUnknown) => {
                MarketStreamRuntimeError::Terminal
            }
            RuntimeMarketStreamStorageError::Storage(MarketStreamStorageError::RightsInvalid) => {
                MarketStreamRuntimeError::GrantUnavailable
            }
            RuntimeMarketStreamStorageError::Storage(
                MarketStreamStorageError::ProducerHeld | MarketStreamStorageError::ProducerLost,
            ) => MarketStreamRuntimeError::LeaseUnavailable,
            RuntimeMarketStreamStorageError::Storage(_) => {
                MarketStreamRuntimeError::StorageUnavailable
            }
        }
    }

    pub(super) fn normalize_unique_demand(
        demand: &DesiredSet,
        expected_slot: uuid::Uuid,
        expected_owner: uuid::Uuid,
    ) -> Result<BTreeMap<super::super::market_stream::StreamIdentity, u32>, MarketStreamRuntimeError>
    {
        let normalized = normalize_demand(demand, expected_slot, expected_owner)
            .map_err(|_| MarketStreamRuntimeError::DemandInvalid)?;
        let mut symbols = BTreeSet::new();
        if normalized
            .keys()
            .any(|identity| !symbols.insert(identity.symbol().to_owned()))
        {
            return Err(MarketStreamRuntimeError::DemandInvalid);
        }
        Ok(normalized)
    }

    pub(super) fn connect_is_admissible(value: ConnectAdmission) -> bool {
        value.positive_demand
            && value.day == DayReadiness::Open
            && value.anchor_reserved
            && value.lease_valid
            && value.post_claim_observation
            && !value.stopping
            && !value.terminal
    }

    pub(super) fn lease_authority(
        config: &super::OwnerMarketStreamRuntimeConfig,
        grant: &RuntimeGrant,
    ) -> LeaseAuthority {
        LeaseAuthority {
            slot: config.slot(),
            grant: grant.grant_id(),
            revision: grant.grant_revision(),
            owner: grant.owner_user_id(),
            holder: config.holder(),
        }
    }

    pub(super) fn lease_has_authority(
        lease: &StreamProducerLease,
        expected: &LeaseAuthority,
    ) -> bool {
        !lease.credential_slot_id.is_nil()
            && !lease.grant_id.is_nil()
            && !lease.grant_revision.is_nil()
            && !lease.owner_user_id.is_nil()
            && !lease.holder_id.is_nil()
            && lease.fencing_token > 0
            && lease.credential_slot_id == expected.slot
            && lease.grant_id == expected.grant
            && lease.grant_revision == expected.revision
            && lease.owner_user_id == expected.owner
            && lease.holder_id == expected.holder
    }

    pub(super) fn lease_refresh_disposition(
        previous: Option<&StreamProducerLease>,
        next: &StreamProducerLease,
        expected: &LeaseAuthority,
        now: DateTime<Utc>,
    ) -> LeaseRefreshDisposition {
        let Some(minimum_expiry) = now.checked_add_signed(MIN_LEASE_MARGIN) else {
            return LeaseRefreshDisposition::Invalid;
        };
        if !lease_has_authority(next, expected)
            || next.heartbeat_at > now
            || next.lease_expires_at <= minimum_expiry
        {
            return LeaseRefreshDisposition::Invalid;
        }
        let Some(previous) = previous else {
            return LeaseRefreshDisposition::Forward;
        };
        if !lease_has_authority(previous, expected)
            || next.fencing_token != previous.fencing_token
            || next.heartbeat_at < previous.heartbeat_at
            || next.lease_expires_at < previous.lease_expires_at
            || next.gap_generation < previous.gap_generation
        {
            return LeaseRefreshDisposition::Invalid;
        }
        if next.gap_generation > previous.gap_generation {
            LeaseRefreshDisposition::DrainWithoutForwarding
        } else {
            LeaseRefreshDisposition::Forward
        }
    }

    pub(super) fn lineage_of(day: &ResolvedMarketStreamDay) -> Lineage {
        let session = day.session();
        Lineage {
            session_date: session.session_date,
            calendar_batch_id: session.calendar_source_batch_id,
            calendar_content_sha256: session.calendar_content_sha256,
            window_contract_sha256: session.window_contract_sha256,
            session_proof_sha256: session.session_proof_sha256,
            open_at: day.open_at(),
            close_at: day.close_at(),
        }
    }

    pub(super) fn same_lineage(left: &Lineage, right: &Lineage) -> bool {
        left == right
    }

    pub(super) fn day_is_current_open(day: &ResolvedMarketStreamDay, now: DateTime<Utc>) -> bool {
        let Some(kst) = FixedOffset::east_opt(9 * 60 * 60) else {
            return false;
        };
        day.open_at() <= now
            && now < day.close_at()
            && now.with_timezone(&kst).date_naive() == day.session_date()
    }

    pub(super) fn observation_is_fresh(
        started: Instant,
        completed: Instant,
        consumed: Instant,
    ) -> bool {
        completed >= started
            && consumed >= completed
            && completed.duration_since(started) <= OBSERVATION_SPACING
            && consumed.duration_since(completed) <= OBSERVATION_SPACING
    }

    pub(super) fn next_cycle_start(previous_start: Option<Instant>, now: Instant) -> Instant {
        previous_start
            .and_then(|start| start.checked_add(OBSERVATION_SPACING))
            .filter(|minimum| *minimum > now)
            .unwrap_or(now)
    }

    pub(super) fn renewal_is_due(force: bool, last_success: Option<Instant>, now: Instant) -> bool {
        force
            || last_success
                .is_some_and(|last| now.saturating_duration_since(last) >= RENEWAL_SPACING)
    }

    pub(super) fn reconnect_is_premutation_wait(error: &MarketStreamError) -> bool {
        matches!(
            error,
            MarketStreamError::State(StateError::ReconnectNotReady)
        )
    }

    pub(super) fn owner_completion(
        external_shutdown: bool,
        terminal: bool,
        control_recovered: bool,
        owner_result: &Result<RuntimeOwnedExit, MarketStreamProducerError>,
    ) -> OwnerCompletion {
        if terminal || !control_recovered || owner_result.is_err() {
            OwnerCompletion::Terminal
        } else if external_shutdown {
            OwnerCompletion::Shutdown
        } else {
            OwnerCompletion::ReplaceAllowed
        }
    }

    pub(super) fn pre_epoch_clean_close_allows_replacement(
        close_succeeded: bool,
        terminal: bool,
    ) -> bool {
        close_succeeded && !terminal
    }

    pub(super) fn map_producer_error(
        _error: MarketStreamProducerError,
    ) -> MarketStreamRuntimeError {
        MarketStreamRuntimeError::Terminal
    }

    pub(super) fn next_demand_snapshot(
        previous_sequence: u64,
        raw: DesiredSet,
        normalized: BTreeMap<super::super::market_stream::StreamIdentity, u32>,
        completed_at: Instant,
    ) -> DemandSnapshot {
        DemandSnapshot {
            raw,
            normalized,
            completed_at,
            sequence: previous_sequence.saturating_add(1),
        }
    }

    pub(super) fn shutdown_requested(receiver: &mut watch::Receiver<bool>) -> bool {
        let value = *receiver.borrow_and_update();
        value || receiver.has_changed().is_err()
    }

    pub(super) fn latch_parent_stop(
        latched: &mut Option<ParentStopReason>,
        reason: ParentStopReason,
        shutdown: Option<&watch::Sender<bool>>,
    ) {
        if latched.is_none() {
            *latched = Some(reason);
        }
        if let Some(shutdown) = shutdown {
            shutdown.send_replace(true);
        }
    }

    pub(super) fn observation_if_nonterminal<O>(
        terminal: bool,
        make: impl FnOnce() -> BoxFuture<O>,
    ) -> Option<BoxFuture<O>> {
        if terminal { None } else { Some(make()) }
    }

    pub(super) enum PumpEvent<S, O> {
        StageReady(S),
        ObservationReady(O),
    }

    /// Holds one stage and one control observation in place across every
    /// timer, watch and observation completion. A finished stage is retained
    /// until the already-started observation has also been recovered.
    pub(super) struct PinnedStagePump<S, O> {
        stage: Option<BoxFuture<S>>,
        stage_result: Option<S>,
        observation: Option<BoxFuture<O>>,
    }

    impl<S, O> PinnedStagePump<S, O> {
        pub(super) fn new(stage: BoxFuture<S>, observation: Option<BoxFuture<O>>) -> Self {
            Self {
                stage: Some(stage),
                stage_result: None,
                observation,
            }
        }

        pub(super) fn stage_pending(&self) -> bool {
            self.stage.is_some()
        }

        pub(super) fn set_observation(&mut self, observation: BoxFuture<O>) {
            debug_assert!(self.observation.is_none());
            debug_assert!(self.stage.is_some());
            self.observation = Some(observation);
        }

        pub(super) fn abandon_stage(&mut self, result: S) {
            if self.stage.is_some() {
                self.stage = None;
                self.stage_result = Some(result);
            }
        }

        pub(super) async fn next_event(&mut self) -> PumpEvent<S, O> {
            loop {
                if self.stage_result.is_some() && self.observation.is_none() {
                    return PumpEvent::StageReady(
                        self.stage_result.take().expect("stage result was checked"),
                    );
                }
                let stage_enabled = self.stage.is_some();
                let observation_enabled = self.observation.is_some();
                enum Ready<S, O> {
                    Stage(S),
                    Observation(O),
                }
                let stage = &mut self.stage;
                let observation = &mut self.observation;
                let ready = tokio::select! {
                    biased;
                    result = async {
                        stage
                            .as_mut()
                            .expect("stage branch is enabled")
                            .as_mut()
                            .await
                    }, if stage_enabled => Ready::Stage(result),
                    result = async {
                        observation
                            .as_mut()
                            .expect("observation branch is enabled")
                            .as_mut()
                            .await
                    }, if observation_enabled => Ready::Observation(result),
                };
                match ready {
                    Ready::Stage(result) => {
                        self.stage = None;
                        self.stage_result = Some(result);
                    }
                    Ready::Observation(result) => {
                        self.observation = None;
                        return PumpEvent::ObservationReady(result);
                    }
                }
            }
        }
    }

    struct ActiveSenders {
        demand: watch::Sender<DesiredSet>,
        lease: watch::Sender<StreamProducerLease>,
        shutdown: watch::Sender<bool>,
    }

    struct RuntimeSupervisor {
        client: kis_client::MarketStreamClient,
        repository: RuntimeMarketStreamRepository,
        calendar: super::super::intraday::OwnerIntradayQuoteRepository,
        windows_source: collectors::intraday_quotes::IntradaySessionWindowSource,
        config: super::OwnerMarketStreamRuntimeConfig,
        grant: RuntimeGrant,
        authority: LeaseAuthority,
        demand: Option<DemandSnapshot>,
        latest_day: Option<DayResolution>,
        latest_open_day: Option<ResolvedMarketStreamDay>,
        correlation_memo: Option<CorrelationMemo>,
        lease: Option<StreamProducerLease>,
        lease_renewed_at: Option<Instant>,
        force_renewal: bool,
        post_claim_renewal_ready: bool,
        anchor: Option<kis_client::MarketStreamConnectionOwner>,
        pending_session: Option<MarketStreamSession>,
        pending_owner: Option<RuntimeOwnedProducer>,
        epoch_lease: Option<StreamProducerLease>,
        epoch_gap_generation: Option<u64>,
        incarnation_day: Option<ResolvedMarketStreamDay>,
        active_senders: Option<ActiveSenders>,
        phase: Phase,
        parent_stop: Option<ParentStopReason>,
        external_shutdown: bool,
        terminal_error: Option<MarketStreamRuntimeError>,
        last_observation_started: Option<Instant>,
        last_observation_completed: Option<Instant>,
        observation_sequence: u64,
        last_no_epoch_transition: Option<RuntimeTransition>,
        last_status_started: Option<Instant>,
    }

    enum Action {
        ReturnShutdown,
        ReturnError(MarketStreamRuntimeError),
        Tick,
        Claim,
        Status(RuntimeTransition),
        Connect,
        StartEpoch,
        RunOwned,
        Close,
    }

    impl RuntimeSupervisor {
        fn new(runtime: OwnerMarketStreamRuntime, grant: RuntimeGrant) -> Self {
            let authority = lease_authority(&runtime.config, &grant);
            Self {
                client: runtime.client,
                repository: runtime.repository,
                calendar: runtime.calendar,
                windows_source: runtime.windows_source,
                config: runtime.config,
                grant,
                authority,
                demand: None,
                latest_day: None,
                latest_open_day: None,
                correlation_memo: None,
                lease: None,
                lease_renewed_at: None,
                force_renewal: false,
                post_claim_renewal_ready: false,
                anchor: None,
                pending_session: None,
                pending_owner: None,
                epoch_lease: None,
                epoch_gap_generation: None,
                incarnation_day: None,
                active_senders: None,
                phase: Phase::Waiting,
                parent_stop: None,
                external_shutdown: false,
                terminal_error: None,
                last_observation_started: None,
                last_observation_completed: None,
                observation_sequence: 0,
                last_no_epoch_transition: None,
                last_status_started: None,
            }
        }

        fn is_stopping(&self) -> bool {
            self.parent_stop.is_some() || self.terminal_error.is_some()
        }

        fn latch_terminal(&mut self, error: MarketStreamRuntimeError) {
            if self.terminal_error.is_none() {
                self.terminal_error = Some(error);
            }
            self.signal_owned_shutdown();
        }

        fn request_stop(&mut self, reason: ParentStopReason) {
            latch_parent_stop(
                &mut self.parent_stop,
                reason,
                self.active_senders
                    .as_ref()
                    .map(|senders| &senders.shutdown),
            );
        }

        fn request_external_shutdown(&mut self) {
            self.external_shutdown = true;
            self.parent_stop = Some(ParentStopReason::ExternalShutdown);
            self.signal_owned_shutdown();
        }

        fn signal_owned_shutdown(&mut self) {
            if let Some(senders) = &self.active_senders {
                senders.shutdown.send_replace(true);
            }
        }

        fn next_observation(&self) -> BoxFuture<Observation> {
            let not_before = next_cycle_start(self.last_observation_started, Instant::now());
            let repository = self.repository.clone();
            let grant_repository = repository.clone();
            let demand_repository = repository.clone();
            let day_repository = repository.clone();
            let renewal_repository = repository;
            let calendar = self.calendar.clone();
            let config = self.config.clone();
            let owner_user_id = self.grant.owner_user_id();
            let windows_source = self.windows_source;
            let force_renewal = self.force_renewal;
            let lease_renewed_at = self.lease_renewed_at;
            let current_lease = self.lease.clone();
            Box::pin(async move {
                tokio::time::sleep_until(tokio::time::Instant::from_std(not_before)).await;
                let started = Instant::now();
                let lease_to_renew = if renewal_is_due(force_renewal, lease_renewed_at, started) {
                    current_lease
                } else {
                    None
                };
                let renewal_requested = lease_to_renew.is_some();
                let renewal_future = async {
                    match lease_to_renew.as_ref() {
                        Some(lease) => renewal_repository
                            .renew_stream_producer(lease)
                            .await
                            .map(Some),
                        None => Ok(None),
                    }
                };
                let (grant, demand, day, renewal) = tokio::join!(
                    grant_repository.load_runtime_grant(&config),
                    demand_repository.read_stream_demand(config.slot()),
                    day_repository.resolve_runtime_day(owner_user_id, &calendar, windows_source),
                    renewal_future,
                );
                let completed = Instant::now();
                Observation {
                    started,
                    completed,
                    grant,
                    demand,
                    day,
                    renewal: renewal.transpose(),
                    renewal_requested,
                }
            })
        }

        fn observe_shutdown_change(&mut self, receiver: &mut watch::Receiver<bool>) -> bool {
            match receiver.has_changed() {
                Err(_) => {
                    self.request_external_shutdown();
                    true
                }
                Ok(_) => {
                    let value = *receiver.borrow_and_update();
                    if value || receiver.has_changed().is_err() {
                        self.request_external_shutdown();
                        true
                    } else {
                        false
                    }
                }
            }
        }

        fn accept_observation(
            &mut self,
            observation: Observation,
        ) -> Result<(), MarketStreamRuntimeError> {
            let consumed = Instant::now();
            if !observation_is_fresh(observation.started, observation.completed, consumed) {
                return Err(MarketStreamRuntimeError::Terminal);
            }
            self.last_observation_started = Some(observation.started);
            self.last_observation_completed = Some(observation.completed);
            self.observation_sequence = self.observation_sequence.saturating_add(1);

            let grant = observation.grant.map_err(map_storage_error)?;
            if grant != self.grant {
                return Err(MarketStreamRuntimeError::GrantUnavailable);
            }
            let raw_demand = observation.demand.map_err(map_storage_error)?;
            let normalized = normalize_unique_demand(
                &raw_demand,
                self.grant.credential_slot_id(),
                self.grant.owner_user_id(),
            )?;
            let resolved_day = observation.day.map_err(map_storage_error)?;
            if observation.renewal_requested != observation.renewal.is_some() {
                return Err(MarketStreamRuntimeError::Terminal);
            }
            let renewed_lease = observation
                .renewal
                .map(|result| result.map_err(map_storage_error))
                .transpose()?;

            let now_wall = Utc::now();
            let prior_lease = self.lease.clone();
            let mut gap_raised = false;
            if let Some(next_lease) = renewed_lease {
                match lease_refresh_disposition(
                    prior_lease.as_ref(),
                    &next_lease,
                    &self.authority,
                    now_wall,
                ) {
                    LeaseRefreshDisposition::Invalid => {
                        return Err(MarketStreamRuntimeError::LeaseUnavailable);
                    }
                    LeaseRefreshDisposition::DrainWithoutForwarding => gap_raised = true,
                    LeaseRefreshDisposition::Forward => {}
                }
                self.lease = Some(next_lease.clone());
                self.lease_renewed_at = Some(observation.completed);
                self.force_renewal = false;
                if self.lease.is_some() && !self.post_claim_renewal_ready {
                    self.post_claim_renewal_ready = true;
                }
            } else if let Some(current_lease) = self.lease.as_ref() {
                if lease_refresh_disposition(
                    Some(current_lease),
                    current_lease,
                    &self.authority,
                    now_wall,
                ) != LeaseRefreshDisposition::Forward
                {
                    return Err(MarketStreamRuntimeError::LeaseUnavailable);
                }
            }

            let day = match resolved_day {
                DayResolution::Open(mut day) if day_is_current_open(&day, now_wall) => {
                    let next_lineage = lineage_of(&day);
                    if let Some(memo) = self.correlation_memo.as_ref()
                        && same_lineage(&memo.lineage, &next_lineage)
                    {
                        day.retain_same_lineage_correlation(&memo.proof);
                    }
                    let proof = day.session();
                    self.correlation_memo = Some(CorrelationMemo {
                        lineage: lineage_of(&day),
                        proof,
                    });
                    DayResolution::Open(day)
                }
                DayResolution::Open(_) | DayResolution::Closed => DayResolution::Closed,
                DayResolution::MissingCalendar => DayResolution::MissingCalendar,
                DayResolution::MissingWindow => DayResolution::MissingWindow,
            };
            let next_open_day = match &day {
                DayResolution::Open(day) => Some(day.clone()),
                DayResolution::MissingCalendar
                | DayResolution::MissingWindow
                | DayResolution::Closed => None,
            };

            self.demand = Some(next_demand_snapshot(
                self.observation_sequence.saturating_sub(1),
                raw_demand.clone(),
                normalized,
                observation.completed,
            ));
            self.latest_day = Some(day.clone());
            self.latest_open_day = next_open_day.clone();

            if let Some(senders) = &self.active_senders {
                senders.demand.send_replace(raw_demand.clone());
            }
            if gap_raised
                && matches!(
                    self.phase,
                    Phase::Connecting | Phase::StartingEpoch | Phase::Active
                )
            {
                self.request_stop(ParentStopReason::FreshEpochRequired);
            }

            if matches!(
                self.phase,
                Phase::Connecting | Phase::StartingEpoch | Phase::Active
            ) {
                let Some(incarnation_day) = self.incarnation_day.as_ref() else {
                    return Err(MarketStreamRuntimeError::Terminal);
                };
                match next_open_day.as_ref() {
                    None => self.request_stop(ParentStopReason::DayClosed),
                    Some(next_day) => {
                        if !same_lineage(&lineage_of(incarnation_day), &lineage_of(next_day)) {
                            self.request_stop(ParentStopReason::FreshEpochRequired);
                        }
                    }
                }
            }

            if raw_demand.items.is_empty() {
                if matches!(self.phase, Phase::Waiting) {
                    self.drop_pre_epoch_binding();
                } else if !matches!(self.phase, Phase::Closing) {
                    self.request_stop(ParentStopReason::NoDemand);
                }
            }

            if let (Some(lease), Some(epoch_gap)) = (self.lease.as_ref(), self.epoch_gap_generation)
                && !self.is_stopping()
                && lease.gap_generation == epoch_gap
            {
                self.epoch_lease = Some(lease.clone());
            }
            if let (Some(lease), Some(senders)) =
                (self.lease.as_ref(), self.active_senders.as_ref())
                && !self.is_stopping()
                && !gap_raised
                && self.epoch_gap_generation == Some(lease.gap_generation)
            {
                senders.lease.send_replace(lease.clone());
                self.epoch_lease = Some(lease.clone());
            }

            if matches!(day, DayResolution::Open(_)) {
                self.last_no_epoch_transition = None;
            }
            Ok(())
        }

        fn drop_pre_epoch_binding(&mut self) {
            if matches!(self.phase, Phase::Waiting) {
                self.anchor = None;
                self.lease = None;
                self.lease_renewed_at = None;
                self.force_renewal = false;
                self.post_claim_renewal_ready = false;
                self.last_no_epoch_transition = None;
            }
        }

        fn action(&mut self) -> Action {
            if self.pending_session.is_some() {
                if self.is_stopping() || !self.session_start_is_admissible() {
                    return Action::Close;
                }
                return Action::StartEpoch;
            }
            if self.pending_owner.is_some() {
                return Action::RunOwned;
            }
            if let Some(error) = self.terminal_error {
                return Action::ReturnError(error);
            }
            if self.external_shutdown {
                return Action::ReturnShutdown;
            }
            let Some(demand) = self.demand.as_ref() else {
                return Action::Tick;
            };
            if !self.current_observation_is_fresh() {
                return Action::Tick;
            }
            if demand.normalized.is_empty() {
                self.drop_pre_epoch_binding();
                return Action::Tick;
            }
            if self.phase == Phase::Waiting && (self.anchor.is_none() || self.lease.is_none()) {
                return Action::Claim;
            }
            if !self.post_claim_renewal_ready {
                return Action::Tick;
            }
            let Some(day) = self.latest_day.as_ref() else {
                return Action::Tick;
            };
            match day {
                DayResolution::MissingCalendar
                | DayResolution::MissingWindow
                | DayResolution::Closed => {
                    let transition = match day {
                        DayResolution::MissingCalendar => RuntimeTransition::CalendarUnavailable,
                        DayResolution::MissingWindow => RuntimeTransition::WindowUnavailable,
                        DayResolution::Closed => RuntimeTransition::SessionClosed,
                        DayResolution::Open(_) => unreachable!(),
                    };
                    if self.last_no_epoch_transition == Some(transition) {
                        return Action::Tick;
                    }
                    let now = Instant::now();
                    if self.last_status_started.is_some_and(|last| {
                        now.saturating_duration_since(last) < OBSERVATION_SPACING
                    }) {
                        return Action::Tick;
                    }
                    return Action::Status(transition);
                }
                DayResolution::Open(_) => {
                    self.last_no_epoch_transition = None;
                }
            }
            let admission = ConnectAdmission {
                positive_demand: !demand.normalized.is_empty(),
                day: match self.latest_day.as_ref() {
                    Some(DayResolution::Open(day)) if day_is_current_open(day, Utc::now()) => {
                        DayReadiness::Open
                    }
                    Some(DayResolution::MissingCalendar) => DayReadiness::MissingCalendar,
                    Some(DayResolution::MissingWindow) => DayReadiness::MissingWindow,
                    Some(DayResolution::Closed) | Some(DayResolution::Open(_)) => {
                        DayReadiness::Closed
                    }
                    None => DayReadiness::MissingCalendar,
                },
                anchor_reserved: self.anchor.is_some(),
                lease_valid: self.lease_is_current(),
                post_claim_observation: self.post_claim_renewal_ready,
                stopping: self.is_stopping(),
                terminal: self.terminal_error.is_some(),
            };
            if connect_is_admissible(admission) {
                Action::Connect
            } else {
                Action::Tick
            }
        }

        fn current_observation_is_fresh(&self) -> bool {
            match (
                self.last_observation_started,
                self.last_observation_completed,
            ) {
                (Some(started), Some(completed)) => {
                    observation_is_fresh(started, completed, Instant::now())
                }
                _ => false,
            }
        }

        fn lease_is_current(&self) -> bool {
            self.lease.as_ref().is_some_and(|lease| {
                lease_refresh_disposition(Some(lease), lease, &self.authority, Utc::now())
                    == LeaseRefreshDisposition::Forward
            })
        }

        fn session_start_is_admissible(&self) -> bool {
            self.current_observation_is_fresh()
                && !self.is_stopping()
                && self.lease_is_current()
                && self
                    .demand
                    .as_ref()
                    .is_some_and(|demand| !demand.normalized.is_empty())
                && self.latest_open_day.as_ref().is_some_and(|day| {
                    self.incarnation_day.as_ref().is_some_and(|incarnation| {
                        day_is_current_open(day, Utc::now())
                            && same_lineage(&lineage_of(day), &lineage_of(incarnation))
                    })
                })
        }

        async fn drive_stage(
            &mut self,
            stage: BoxFuture<StageResult>,
            kind: StageKind,
            shutdown: &mut watch::Receiver<bool>,
        ) -> StageResult {
            let observation = observation_if_nonterminal(self.terminal_error.is_some(), || {
                self.next_observation()
            });
            let mut pump = PinnedStagePump::new(stage, observation);
            let mut stop_deadline = if kind == StageKind::Connect && self.is_stopping() {
                Some(tokio::time::Instant::now() + CONNECT_STOP_BOUND)
            } else {
                None
            };
            let mut shutdown_open = true;
            loop {
                if kind == StageKind::Connect
                    && pump.stage_pending()
                    && self.is_stopping()
                    && stop_deadline.is_none()
                {
                    stop_deadline = Some(tokio::time::Instant::now() + CONNECT_STOP_BOUND);
                }
                let event = if let Some(deadline) = stop_deadline {
                    tokio::select! {
                        event = pump.next_event() => Some(event),
                        _ = shutdown.changed(), if shutdown_open => {
                            if self.observe_shutdown_change(shutdown) {
                                shutdown_open = false;
                            }
                            if kind == StageKind::Tick {
                                pump.abandon_stage(StageResult::Tick);
                            }
                            None
                        }
                        _ = tokio::time::sleep_until(deadline), if kind == StageKind::Connect => {
                            stop_deadline = None;
                            self.latch_terminal(MarketStreamRuntimeError::Terminal);
                            pump.abandon_stage(StageResult::ConnectTimedOut);
                            None
                        }
                    }
                } else {
                    tokio::select! {
                        event = pump.next_event() => Some(event),
                        _ = shutdown.changed(), if shutdown_open => {
                            if self.observe_shutdown_change(shutdown) {
                                shutdown_open = false;
                            }
                            if kind == StageKind::Tick {
                                pump.abandon_stage(StageResult::Tick);
                            }
                            None
                        }
                    }
                };
                match event {
                    Some(PumpEvent::ObservationReady(observation)) => {
                        if let Err(error) = self.accept_observation(observation) {
                            self.latch_terminal(error);
                        }
                        if pump.stage_pending()
                            && let Some(observation) =
                                observation_if_nonterminal(self.terminal_error.is_some(), || {
                                    self.next_observation()
                                })
                        {
                            pump.set_observation(observation);
                        }
                    }
                    Some(PumpEvent::StageReady(result)) => return result,
                    None => {}
                }
            }
        }

        fn clear_incarnation(&mut self) {
            self.active_senders = None;
            self.pending_session = None;
            self.pending_owner = None;
            self.anchor = None;
            self.lease = None;
            self.lease_renewed_at = None;
            self.force_renewal = false;
            self.post_claim_renewal_ready = false;
            self.epoch_lease = None;
            self.epoch_gap_generation = None;
            self.incarnation_day = None;
            self.phase = Phase::Waiting;
            if self.terminal_error.is_none() && !self.external_shutdown {
                self.parent_stop = None;
            }
        }

        fn claim_stage(&self) -> BoxFuture<StageResult> {
            let repository = self.repository.clone();
            let slot = self.config.slot();
            let holder = self.config.holder();
            let revision = self.grant.grant_revision();
            Box::pin(async move {
                StageResult::Claimed(
                    repository
                        .claim_stream_producer(slot, holder, revision)
                        .await,
                )
            })
        }

        fn status_stage(&self, transition: RuntimeTransition) -> BoxFuture<StageResult> {
            let repository = self.repository.clone();
            let Some(lease) = self.lease.clone() else {
                return Box::pin(async move {
                    StageResult::Status(transition, Err(RuntimeMarketStreamStorageError::Terminal))
                });
            };
            Box::pin(async move {
                StageResult::Status(
                    transition,
                    repository
                        .record_runtime_status(&lease, None, transition)
                        .await,
                )
            })
        }

        fn connect_stage(&mut self) -> Result<BoxFuture<StageResult>, MarketStreamRuntimeError> {
            let anchor = self
                .anchor
                .take()
                .ok_or(MarketStreamRuntimeError::Terminal)?;
            let day = self
                .latest_open_day
                .clone()
                .ok_or(MarketStreamRuntimeError::CalendarUnavailable)?;
            self.incarnation_day = Some(day.clone());
            self.phase = Phase::Connecting;
            Ok(Box::pin(async move {
                StageResult::Connected(anchor.connect(day.transport_proof()).await)
            }))
        }

        fn start_epoch_stage(
            &mut self,
        ) -> Result<BoxFuture<StageResult>, MarketStreamRuntimeError> {
            let session = self
                .pending_session
                .take()
                .ok_or(MarketStreamRuntimeError::Terminal)?;
            let lease = self
                .lease
                .clone()
                .ok_or(MarketStreamRuntimeError::LeaseUnavailable)?;
            let day = self
                .latest_open_day
                .clone()
                .ok_or(MarketStreamRuntimeError::CalendarUnavailable)?;
            self.epoch_gap_generation = Some(lease.gap_generation);
            self.epoch_lease = Some(lease.clone());
            self.incarnation_day = Some(day.clone());
            self.phase = Phase::StartingEpoch;
            let repository = self.repository.clone();
            Ok(Box::pin(async move {
                StageResult::Started(
                    OwnerMarketStreamProducer::start_resolved(repository, lease, day, session)
                        .await,
                )
            }))
        }

        fn run_owned_stage(&mut self) -> Result<BoxFuture<StageResult>, MarketStreamRuntimeError> {
            let owner = self
                .pending_owner
                .take()
                .ok_or(MarketStreamRuntimeError::Terminal)?;
            let demand = self
                .demand
                .as_ref()
                .map(|snapshot| snapshot.raw.clone())
                .ok_or(MarketStreamRuntimeError::DemandInvalid)?;
            let lease = self
                .epoch_lease
                .clone()
                .ok_or(MarketStreamRuntimeError::LeaseUnavailable)?;
            let (demand_tx, demand_rx) = watch::channel(demand);
            let (lease_tx, lease_rx) = watch::channel(lease);
            let shutdown_value = self.is_stopping() || self.external_shutdown;
            let (shutdown_tx, shutdown_rx) = watch::channel(shutdown_value);
            self.active_senders = Some(ActiveSenders {
                demand: demand_tx,
                lease: lease_tx,
                shutdown: shutdown_tx,
            });
            self.phase = Phase::Active;
            Ok(Box::pin(async move {
                StageResult::Owned(owner.run_owned(demand_rx, lease_rx, shutdown_rx).await)
            }))
        }

        fn close_stage(&mut self) -> Result<BoxFuture<StageResult>, MarketStreamRuntimeError> {
            let session = self
                .pending_session
                .take()
                .ok_or(MarketStreamRuntimeError::Terminal)?;
            self.phase = Phase::Closing;
            Ok(Box::pin(async move {
                match tokio::time::timeout(CLOSE_BOUND, session.close()).await {
                    Ok(result) => StageResult::Closed(result),
                    Err(_) => StageResult::CloseTimedOut,
                }
            }))
        }

        async fn run_loop(
            mut self,
            shutdown: &mut watch::Receiver<bool>,
        ) -> Result<super::MarketStreamRuntimeExit, MarketStreamRuntimeError> {
            loop {
                if shutdown_requested(shutdown) {
                    self.request_external_shutdown();
                }
                match self.action() {
                    Action::ReturnShutdown => {
                        return Ok(super::MarketStreamRuntimeExit::Shutdown);
                    }
                    Action::ReturnError(error) => return Err(error),
                    Action::Tick => {
                        let due = next_cycle_start(self.last_observation_started, Instant::now());
                        let stage: BoxFuture<StageResult> = Box::pin(async move {
                            tokio::time::sleep_until(tokio::time::Instant::from_std(due)).await;
                            StageResult::Tick
                        });
                        let result = self.drive_stage(stage, StageKind::Tick, shutdown).await;
                        self.finish_stage(result);
                    }
                    Action::Claim => {
                        if self.anchor.is_none() {
                            match self.client.reserve_connection() {
                                Ok(anchor) => self.anchor = Some(anchor),
                                Err(_) => {
                                    self.latch_terminal(
                                        MarketStreamRuntimeError::TransportUnavailable,
                                    );
                                    continue;
                                }
                            }
                        }
                        let stage = self.claim_stage();
                        let result = self.drive_stage(stage, StageKind::Claim, shutdown).await;
                        self.finish_stage(result);
                    }
                    Action::Status(transition) => {
                        self.last_status_started = Some(Instant::now());
                        let stage = self.status_stage(transition);
                        let result = self.drive_stage(stage, StageKind::Status, shutdown).await;
                        self.finish_stage(result);
                    }
                    Action::Connect => match self.connect_stage() {
                        Ok(stage) => {
                            let result =
                                self.drive_stage(stage, StageKind::Connect, shutdown).await;
                            self.finish_stage(result);
                        }
                        Err(error) => self.latch_terminal(error),
                    },
                    Action::StartEpoch => match self.start_epoch_stage() {
                        Ok(stage) => {
                            let result = self
                                .drive_stage(stage, StageKind::StartEpoch, shutdown)
                                .await;
                            self.finish_stage(result);
                        }
                        Err(error) => self.latch_terminal(error),
                    },
                    Action::RunOwned => match self.run_owned_stage() {
                        Ok(stage) => {
                            let result =
                                self.drive_stage(stage, StageKind::RunOwned, shutdown).await;
                            self.finish_stage(result);
                        }
                        Err(error) => self.latch_terminal(error),
                    },
                    Action::Close => match self.close_stage() {
                        Ok(stage) => {
                            let result = self.drive_stage(stage, StageKind::Close, shutdown).await;
                            self.finish_stage(result);
                        }
                        Err(error) => self.latch_terminal(error),
                    },
                }
            }
        }

        fn finish_stage(&mut self, result: StageResult) {
            match result {
                StageResult::Tick => {}
                StageResult::Claimed(result) => match result {
                    Err(error) => self.latch_terminal(map_storage_error(error)),
                    Ok(lease) => {
                        let disposition =
                            lease_refresh_disposition(None, &lease, &self.authority, Utc::now());
                        if disposition != LeaseRefreshDisposition::Forward {
                            self.latch_terminal(MarketStreamRuntimeError::LeaseUnavailable);
                        } else {
                            self.lease = Some(lease);
                            self.lease_renewed_at = Some(Instant::now());
                            self.force_renewal = true;
                            self.post_claim_renewal_ready = false;
                            if self
                                .demand
                                .as_ref()
                                .is_none_or(|demand| demand.normalized.is_empty())
                            {
                                self.drop_pre_epoch_binding();
                            }
                        }
                    }
                },
                StageResult::Status(transition, result) => match result {
                    Err(error) => self.latch_terminal(map_storage_error(error)),
                    Ok(commit) if commit.current_epoch().is_some() => {
                        self.latch_terminal(MarketStreamRuntimeError::Terminal);
                    }
                    Ok(_) => {
                        self.last_no_epoch_transition = Some(transition);
                        // A status commit can advance gap metadata. Only a new
                        // database renewal may supply the pre-connect lease.
                        self.force_renewal = true;
                        self.post_claim_renewal_ready = false;
                    }
                },
                StageResult::Connected(result) => match result {
                    Err(error) if reconnect_is_premutation_wait(&error) => {
                        self.clear_incarnation();
                    }
                    Err(error) => self.latch_terminal(map_connect_error(error)),
                    Ok(session) => self.pending_session = Some(session),
                },
                StageResult::Started(result) => match result {
                    Err(error) => self.latch_terminal(map_producer_error(error)),
                    Ok(owner) => self.pending_owner = Some(owner),
                },
                StageResult::Owned(result) => {
                    let decision = owner_completion(
                        self.external_shutdown,
                        self.terminal_error.is_some(),
                        true,
                        &result,
                    );
                    match result {
                        Err(_) => self.latch_terminal(MarketStreamRuntimeError::Terminal),
                        Ok(_) => match decision {
                            OwnerCompletion::ReplaceAllowed => self.clear_incarnation(),
                            OwnerCompletion::Shutdown => self.clear_incarnation(),
                            OwnerCompletion::Terminal => {
                                self.latch_terminal(MarketStreamRuntimeError::Terminal);
                                self.clear_incarnation();
                            }
                        },
                    }
                }
                StageResult::Closed(result) => match result {
                    Err(_) => self.latch_terminal(MarketStreamRuntimeError::Terminal),
                    Ok(())
                        if pre_epoch_clean_close_allows_replacement(
                            true,
                            self.terminal_error.is_some(),
                        ) =>
                    {
                        self.clear_incarnation()
                    }
                    Ok(()) => self.clear_incarnation(),
                },
                StageResult::ConnectTimedOut | StageResult::CloseTimedOut => {
                    self.latch_terminal(MarketStreamRuntimeError::Terminal);
                    self.clear_incarnation();
                }
            }
        }
    }

    fn map_connect_error(error: MarketStreamError) -> MarketStreamRuntimeError {
        match error {
            MarketStreamError::State(_) => MarketStreamRuntimeError::Terminal,
            _ => MarketStreamRuntimeError::TransportUnavailable,
        }
    }

    pub(super) async fn run(
        runtime: OwnerMarketStreamRuntime,
        grant: RuntimeGrant,
        mut shutdown: watch::Receiver<bool>,
    ) -> Result<super::MarketStreamRuntimeExit, MarketStreamRuntimeError> {
        if shutdown_requested(&mut shutdown) {
            return Ok(super::MarketStreamRuntimeExit::Shutdown);
        }
        RuntimeSupervisor::new(runtime, grant)
            .run_loop(&mut shutdown)
            .await
    }
}

#[cfg(test)]
mod supervisor_tests {
    use super::runtime_supervisor::*;
    use crate::owner_equity_v2::intraday::IntradaySessionProof;
    use crate::owner_equity_v2::market_stream::{
        DesiredSet, DesiredStreamItem, MarketStreamStorageError, RuntimeMarketStreamStorageError,
        StreamIdentity, StreamProducerLease,
    };
    use crate::owner_equity_v2::market_stream_producer::{
        MarketStreamProducerError, RuntimeOwnedExit,
    };
    use chrono::{DateTime, NaiveDate, Utc};
    use kis_client::market_stream::MarketStreamError;
    use kis_client::market_stream_state::StateError;
    use std::collections::BTreeMap;
    use std::future::Future;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::{Duration, Instant};
    use tokio::sync::Notify;

    fn id(value: u128) -> uuid::Uuid {
        uuid::Uuid::from_u128(value)
    }

    fn identity(
        owner: uuid::Uuid,
        symbol: &str,
        membership: u128,
        generation_id: u128,
        generation: u64,
    ) -> StreamIdentity {
        StreamIdentity::new(
            owner,
            id(membership),
            id(generation_id),
            format!("{symbol}.KRX"),
            generation,
        )
        .expect("valid in-memory identity")
    }

    fn demand(
        slot: uuid::Uuid,
        owner: uuid::Uuid,
        items: Vec<(StreamIdentity, u32)>,
    ) -> DesiredSet {
        DesiredSet {
            credential_slot_id: slot,
            owner_user_id: owner,
            items: items
                .into_iter()
                .map(|(identity, reference_count)| DesiredStreamItem {
                    identity,
                    reference_count,
                })
                .collect(),
        }
    }

    #[test]
    fn supervisor_demand_requires_exact_binding_and_unique_symbols() {
        let slot = id(1);
        let other_slot = id(2);
        let owner = id(3);
        let other_owner = id(4);
        let one = identity(owner, "005930", 10, 11, 1);
        let two = identity(owner, "000660", 12, 13, 1);

        let valid = demand(slot, owner, vec![(one.clone(), 1), (two, 3)]);
        let accepted = normalize_unique_demand(&valid, slot, owner).unwrap();
        assert_eq!(accepted.len(), 2);
        assert!(normalize_unique_demand(&demand(slot, owner, vec![]), slot, owner).is_ok());

        assert_eq!(
            normalize_unique_demand(&valid, other_slot, owner),
            Err(super::super::MarketStreamRuntimeError::DemandInvalid)
        );
        assert_eq!(
            normalize_unique_demand(
                &demand(slot, other_owner, vec![(one.clone(), 1)]),
                slot,
                owner,
            ),
            Err(super::super::MarketStreamRuntimeError::DemandInvalid)
        );
        assert_eq!(
            normalize_unique_demand(&demand(slot, owner, vec![(one.clone(), 0)]), slot, owner,),
            Err(super::super::MarketStreamRuntimeError::DemandInvalid)
        );
        assert_eq!(
            normalize_unique_demand(
                &demand(slot, owner, vec![(one.clone(), 1), (one.clone(), 2)]),
                slot,
                owner,
            ),
            Err(super::super::MarketStreamRuntimeError::DemandInvalid)
        );

        let same_symbol_new_membership = identity(owner, "005930", 20, 21, 2);
        assert_eq!(
            normalize_unique_demand(
                &demand(slot, owner, vec![(one, 1), (same_symbol_new_membership, 1)],),
                slot,
                owner,
            ),
            Err(super::super::MarketStreamRuntimeError::DemandInvalid)
        );
    }

    #[test]
    fn supervisor_waiting_requires_anchor_lease_and_fresh_open_observation() {
        let ready = ConnectAdmission {
            positive_demand: true,
            day: DayReadiness::Open,
            anchor_reserved: true,
            lease_valid: true,
            post_claim_observation: true,
            stopping: false,
            terminal: false,
        };
        assert!(connect_is_admissible(ready));
        for day in [
            DayReadiness::MissingCalendar,
            DayReadiness::MissingWindow,
            DayReadiness::Closed,
        ] {
            assert!(!connect_is_admissible(ConnectAdmission { day, ..ready }));
        }
        assert!(!connect_is_admissible(ConnectAdmission {
            positive_demand: false,
            ..ready
        }));
        assert!(!connect_is_admissible(ConnectAdmission {
            anchor_reserved: false,
            ..ready
        }));
        assert!(!connect_is_admissible(ConnectAdmission {
            lease_valid: false,
            ..ready
        }));
        assert!(!connect_is_admissible(ConnectAdmission {
            post_claim_observation: false,
            ..ready
        }));
        assert!(!connect_is_admissible(ConnectAdmission {
            stopping: true,
            ..ready
        }));
        assert!(!connect_is_admissible(ConnectAdmission {
            terminal: true,
            ..ready
        }));
    }

    fn fixed_time(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .expect("fixed timestamp")
            .with_timezone(&Utc)
    }

    fn lease_authority_for_test() -> LeaseAuthority {
        LeaseAuthority {
            slot: id(30),
            grant: id(31),
            revision: id(32),
            owner: id(33),
            holder: id(34),
        }
    }

    fn lease_for_test(now: DateTime<Utc>) -> StreamProducerLease {
        StreamProducerLease {
            credential_slot_id: id(30),
            grant_id: id(31),
            grant_revision: id(32),
            owner_user_id: id(33),
            holder_id: id(34),
            fencing_token: 7,
            gap_generation: 4,
            lease_expires_at: now + chrono::Duration::seconds(20),
            heartbeat_at: now - chrono::Duration::seconds(1),
        }
    }

    #[test]
    fn supervisor_lease_refresh_rejects_authority_regression_and_drains_gap() {
        let now = fixed_time("2026-10-02T01:00:00Z");
        let authority = lease_authority_for_test();
        let old = lease_for_test(now);
        let mut renewed = old.clone();
        renewed.heartbeat_at = now - chrono::Duration::milliseconds(500);
        renewed.lease_expires_at += chrono::Duration::seconds(5);
        assert_eq!(
            lease_refresh_disposition(Some(&old), &renewed, &authority, now),
            LeaseRefreshDisposition::Forward
        );
        assert_eq!(old, lease_for_test(now));

        let mut gap = renewed.clone();
        gap.gap_generation += 1;
        assert_eq!(
            lease_refresh_disposition(Some(&old), &gap, &authority, now),
            LeaseRefreshDisposition::DrainWithoutForwarding
        );
        assert_eq!(old.gap_generation, 4);
        let mut lower_gap = renewed.clone();
        lower_gap.gap_generation -= 1;
        assert_eq!(
            lease_refresh_disposition(Some(&old), &lower_gap, &authority, now),
            LeaseRefreshDisposition::Invalid
        );

        let mut cases = Vec::new();
        let mut changed = renewed.clone();
        changed.credential_slot_id = id(35);
        cases.push(changed);
        let mut changed = renewed.clone();
        changed.grant_id = id(35);
        cases.push(changed);
        let mut changed = renewed.clone();
        changed.grant_revision = id(35);
        cases.push(changed);
        let mut changed = renewed.clone();
        changed.owner_user_id = id(35);
        cases.push(changed);
        let mut changed = renewed.clone();
        changed.holder_id = id(35);
        cases.push(changed);
        let mut changed = renewed.clone();
        changed.fencing_token = 0;
        cases.push(changed);
        let mut changed = renewed.clone();
        changed.fencing_token = old.fencing_token + 1;
        cases.push(changed);
        let mut changed = renewed.clone();
        changed.fencing_token = old.fencing_token - 1;
        cases.push(changed);
        let mut changed = renewed.clone();
        changed.credential_slot_id = uuid::Uuid::nil();
        cases.push(changed);
        for invalid in cases {
            assert_eq!(
                lease_refresh_disposition(Some(&old), &invalid, &authority, now),
                LeaseRefreshDisposition::Invalid
            );
        }

        let mut regressed_heartbeat = renewed.clone();
        regressed_heartbeat.heartbeat_at = old.heartbeat_at - chrono::Duration::milliseconds(1);
        assert_eq!(
            lease_refresh_disposition(Some(&old), &regressed_heartbeat, &authority, now),
            LeaseRefreshDisposition::Invalid
        );
        let mut regressed_expiry = renewed.clone();
        regressed_expiry.lease_expires_at =
            old.lease_expires_at - chrono::Duration::milliseconds(1);
        assert_eq!(
            lease_refresh_disposition(Some(&old), &regressed_expiry, &authority, now),
            LeaseRefreshDisposition::Invalid
        );
        let mut future_heartbeat = renewed.clone();
        future_heartbeat.heartbeat_at = now + chrono::Duration::nanoseconds(1);
        assert_eq!(
            lease_refresh_disposition(Some(&old), &future_heartbeat, &authority, now),
            LeaseRefreshDisposition::Invalid
        );
        let mut short_expiry = renewed.clone();
        short_expiry.lease_expires_at = now + MIN_LEASE_MARGIN;
        assert_eq!(
            lease_refresh_disposition(Some(&old), &short_expiry, &authority, now),
            LeaseRefreshDisposition::Invalid
        );
    }

    fn lineage_for_test() -> Lineage {
        Lineage {
            session_date: NaiveDate::from_ymd_opt(2026, 10, 2).expect("fixed date"),
            calendar_batch_id: id(40),
            calendar_content_sha256: "a".repeat(64),
            window_contract_sha256: format!("sha256:{}", "b".repeat(64)),
            session_proof_sha256: format!("sha256:{}", "c".repeat(64)),
            open_at: fixed_time("2026-10-02T00:00:00Z"),
            close_at: fixed_time("2026-10-02T08:00:00Z"),
        }
    }

    #[test]
    fn supervisor_lineage_changes_require_drain_and_keep_correlation_nonauthoritative() {
        let original = lineage_for_test();
        let independent_correlation_ids = (id(41), id(42));
        assert_ne!(independent_correlation_ids.0, independent_correlation_ids.1);
        assert!(same_lineage(&original, &lineage_for_test()));

        let mut changed = original.clone();
        changed.session_date = NaiveDate::from_ymd_opt(2026, 10, 3).expect("fixed date");
        assert!(!same_lineage(&original, &changed));
        let mut changed = original.clone();
        changed.calendar_batch_id = id(43);
        assert!(!same_lineage(&original, &changed));
        let mut changed = original.clone();
        changed.calendar_content_sha256 = "d".repeat(64);
        assert!(!same_lineage(&original, &changed));
        let mut changed = original.clone();
        changed.window_contract_sha256 = format!("sha256:{}", "e".repeat(64));
        assert!(!same_lineage(&original, &changed));
        let mut changed = original.clone();
        changed.session_proof_sha256 = format!("sha256:{}", "f".repeat(64));
        assert!(!same_lineage(&original, &changed));
        let mut changed = original.clone();
        changed.open_at += chrono::Duration::seconds(1);
        assert!(!same_lineage(&original, &changed));
        let mut changed = original.clone();
        changed.close_at -= chrono::Duration::seconds(1);
        assert!(!same_lineage(&original, &changed));

        let date = NaiveDate::from_ymd_opt(2026, 10, 2).expect("fixed date");
        let calendar = IntradaySessionProof::new(
            date,
            id(40),
            "a".repeat(64),
            format!("sha256:{}", "b".repeat(64)),
        )
        .expect("synthetic calendar proof");
        let day = super::fixture_day(
            calendar,
            fixed_time("2026-10-02T00:00:00Z"),
            fixed_time("2026-10-02T08:00:00Z"),
            id(44),
        )
        .expect("existing test-only day fixture");
        assert!(day_is_current_open(&day, day.open_at()));
        assert!(!day_is_current_open(&day, day.close_at()));
    }

    #[test]
    fn supervisor_fresh_observation_and_schedule_do_not_replay_or_catch_up() {
        let start = Instant::now();
        let completion = start + Duration::from_secs(1);
        assert!(observation_is_fresh(start, completion, completion));
        assert!(!observation_is_fresh(
            start,
            completion + Duration::from_nanos(1),
            completion + Duration::from_nanos(1),
        ));
        assert!(!observation_is_fresh(
            start,
            completion,
            completion + Duration::from_secs(1) + Duration::from_nanos(1),
        ));

        let delayed_now = start + Duration::from_secs(8);
        assert_eq!(next_cycle_start(Some(start), delayed_now), delayed_now);
        assert_eq!(
            next_cycle_start(Some(delayed_now), delayed_now),
            delayed_now + OBSERVATION_SPACING
        );
        assert!(!renewal_is_due(
            false,
            Some(start),
            start + Duration::from_millis(4_999)
        ));
        assert!(renewal_is_due(false, Some(start), start + RENEWAL_SPACING));
        assert!(renewal_is_due(
            true,
            Some(start),
            start + Duration::from_millis(1)
        ));

        let slot = id(50);
        let owner = id(51);
        let unchanged = demand(slot, owner, vec![]);
        let first = next_demand_snapshot(0, unchanged.clone(), BTreeMap::new(), start);
        let second = next_demand_snapshot(
            first.sequence,
            unchanged,
            BTreeMap::new(),
            start + OBSERVATION_SPACING,
        );
        assert_eq!(first.raw, second.raw);
        assert_eq!(first.sequence, 1);
        assert_eq!(second.sequence, 2);
        assert_ne!(first.completed_at, second.completed_at);
    }

    #[test]
    fn supervisor_only_premutation_backoff_can_wait_again() {
        assert!(reconnect_is_premutation_wait(&MarketStreamError::State(
            StateError::ReconnectNotReady
        )));
        for error in [
            MarketStreamError::State(StateError::PriorSessionUncertain),
            MarketStreamError::State(StateError::InvalidState),
            MarketStreamError::State(StateError::UnsafePath),
            MarketStreamError::State(StateError::LockBusy),
            MarketStreamError::State(StateError::Io),
            MarketStreamError::Io,
            MarketStreamError::AckTimeout,
            MarketStreamError::Closed,
        ] {
            assert!(!reconnect_is_premutation_wait(&error));
        }
        assert_eq!(
            map_storage_error(RuntimeMarketStreamStorageError::Storage(
                MarketStreamStorageError::CommitUnknown,
            )),
            super::super::MarketStreamRuntimeError::Terminal
        );
        assert_eq!(
            map_storage_error(RuntimeMarketStreamStorageError::DeadlineExceeded),
            super::super::MarketStreamRuntimeError::Terminal
        );
        assert_eq!(
            map_storage_error(RuntimeMarketStreamStorageError::Storage(
                MarketStreamStorageError::RightsInvalid,
            )),
            super::super::MarketStreamRuntimeError::GrantUnavailable
        );
        assert_eq!(
            map_storage_error(RuntimeMarketStreamStorageError::Storage(
                MarketStreamStorageError::ProducerLost,
            )),
            super::super::MarketStreamRuntimeError::LeaseUnavailable
        );
        assert_eq!(
            map_producer_error(MarketStreamProducerError::NotReady),
            super::super::MarketStreamRuntimeError::Terminal
        );
        assert_eq!(
            super::super::MarketStreamRuntimeError::Terminal.to_string(),
            "MARKET_STREAM_RUNTIME_TERMINAL"
        );
    }

    #[test]
    fn supervisor_stop_latches_and_replacement_requires_complete_clean_result() {
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let (demand_tx, demand_rx) = tokio::sync::watch::channel(1_u32);
        let mut stop = None;
        demand_tx.send_replace(0);
        latch_parent_stop(&mut stop, ParentStopReason::NoDemand, Some(&shutdown_tx));
        demand_tx.send_replace(1);
        let coalesced_demand = *demand_rx.borrow();
        let stopped_on_zero = *shutdown_rx.borrow();
        latch_parent_stop(&mut stop, ParentStopReason::DayClosed, Some(&shutdown_tx));
        let remained_stopped = *shutdown_rx.borrow();
        drop(shutdown_tx);
        drop(shutdown_rx);
        drop(demand_tx);
        drop(demand_rx);
        assert_eq!(coalesced_demand, 1);
        assert!(stopped_on_zero);
        assert!(remained_stopped);
        assert_eq!(stop, Some(ParentStopReason::NoDemand));
        let clean_exits = [
            RuntimeOwnedExit::Shutdown,
            RuntimeOwnedExit::NoDemand,
            RuntimeOwnedExit::DayClosed,
            RuntimeOwnedExit::CleanClosed,
            RuntimeOwnedExit::FreshEpochRequired,
        ];
        for exit in clean_exits {
            let result = Ok(exit);
            assert_eq!(
                owner_completion(false, false, true, &result),
                OwnerCompletion::ReplaceAllowed
            );
            assert_eq!(
                owner_completion(true, false, true, &result),
                OwnerCompletion::Shutdown
            );
            assert_eq!(
                owner_completion(false, true, true, &result),
                OwnerCompletion::Terminal
            );
            assert_eq!(
                owner_completion(false, false, false, &result),
                OwnerCompletion::Terminal
            );
        }
        assert_eq!(
            owner_completion(
                false,
                false,
                true,
                &Err(MarketStreamProducerError::Terminal),
            ),
            OwnerCompletion::Terminal
        );
        assert!(pre_epoch_clean_close_allows_replacement(true, false));
        assert!(!pre_epoch_clean_close_allows_replacement(false, false));
        assert!(!pre_epoch_clean_close_allows_replacement(true, true));
        assert_ne!(
            ParentStopReason::ExternalShutdown,
            ParentStopReason::FreshEpochRequired
        );
    }

    struct DropAck(Arc<AtomicUsize>);

    impl Drop for DropAck {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn immediate_control(counter: Arc<AtomicUsize>, value: u8) -> BoxFuture<u8> {
        Box::pin(async move {
            counter.fetch_add(1, Ordering::SeqCst);
            value
        })
    }

    #[tokio::test]
    async fn supervisor_stage_ticks_preserve_one_pending_future_and_drain_observation() {
        let stage_polls = Arc::new(AtomicUsize::new(0));
        let stage_drops = Arc::new(AtomicUsize::new(0));
        let release_stage = Arc::new(Notify::new());
        let stage_poll_counter = Arc::clone(&stage_polls);
        let stage_drop_counter = Arc::clone(&stage_drops);
        let stage_release = Arc::clone(&release_stage);
        let constructed = Arc::new(AtomicUsize::new(0));
        constructed.fetch_add(1, Ordering::SeqCst);
        let stage: BoxFuture<u8> = Box::pin(async move {
            let _drop_ack = DropAck(stage_drop_counter);
            let notified = stage_release.notified();
            tokio::pin!(notified);
            std::future::poll_fn(move |context| {
                stage_poll_counter.fetch_add(1, Ordering::SeqCst);
                notified.as_mut().poll(context)
            })
            .await;
            7
        });
        let control_count = Arc::new(AtomicUsize::new(0));
        let mut pump = PinnedStagePump::new(
            stage,
            Some(immediate_control(Arc::clone(&control_count), 1)),
        );
        let release_control = Arc::new(Notify::new());
        let control_polled = Arc::new(AtomicUsize::new(0));
        let control_release = Arc::clone(&release_control);
        let control_poll_counter = Arc::clone(&control_polled);
        let observed = tokio::time::timeout(Duration::from_secs(2), async {
            if !matches!(pump.next_event().await, PumpEvent::ObservationReady(1)) {
                return Err("first observation did not finish independently");
            }
            pump.set_observation(immediate_control(Arc::clone(&control_count), 2));
            if !matches!(pump.next_event().await, PumpEvent::ObservationReady(2)) {
                return Err("second observation did not preserve the stage");
            }
            pump.set_observation(immediate_control(Arc::clone(&control_count), 3));
            if !matches!(pump.next_event().await, PumpEvent::ObservationReady(3)) {
                return Err("third observation did not preserve the stage");
            }
            let stages_constructed = constructed.load(Ordering::SeqCst);
            let controls_finished = control_count.load(Ordering::SeqCst);
            pump.set_observation(Box::pin(async move {
                let notified = control_release.notified();
                tokio::pin!(notified);
                std::future::poll_fn(move |context| {
                    control_poll_counter.fetch_add(1, Ordering::SeqCst);
                    notified.as_mut().poll(context)
                })
                .await;
                9
            }));
            release_stage.notify_one();
            if tokio::time::timeout(Duration::from_millis(20), pump.next_event())
                .await
                .is_ok()
            {
                return Err("stage result escaped before the in-flight observation");
            }
            let stage_was_polled = stage_polls.load(Ordering::SeqCst) > 0;
            let stage_finished_first = stage_drops.load(Ordering::SeqCst) == 1;
            let control_was_polled = control_polled.load(Ordering::SeqCst) > 0;
            release_control.notify_one();
            if !matches!(pump.next_event().await, PumpEvent::ObservationReady(9)) {
                return Err("held observation did not finish");
            }
            let final_stage = matches!(pump.next_event().await, PumpEvent::StageReady(7));
            Ok((
                stages_constructed,
                controls_finished,
                stage_was_polled,
                stage_finished_first,
                control_was_polled,
                final_stage,
            ))
        })
        .await;
        // These are inline synthetic futures, with no spawned children. Release
        // both gates and drop every retained future before checking any result.
        release_stage.notify_one();
        release_control.notify_one();
        drop(pump);
        let (stages, controls, stage_polled, stage_finished, control_polled, final_stage) =
            observed
                .expect("bounded pump scenario")
                .expect("pump events");
        assert_eq!(stages, 1);
        assert_eq!(controls, 3);
        assert!(stage_polled && stage_finished && control_polled && final_stage);
        assert_eq!(stage_drops.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn supervisor_terminal_cleanup_does_not_construct_new_observations() {
        let constructed = Arc::new(AtomicUsize::new(0));
        let polled = Arc::new(AtomicUsize::new(0));
        let observation = observation_if_nonterminal(true, || {
            constructed.fetch_add(1, Ordering::SeqCst);
            immediate_control(Arc::clone(&polled), 9)
        });
        let mut terminal = PinnedStagePump::new(Box::pin(async { 7_u8 }), observation);
        let terminal_result =
            tokio::time::timeout(Duration::from_secs(1), terminal.next_event()).await;
        drop(terminal);
        let terminal_counts = (
            constructed.load(Ordering::SeqCst),
            polled.load(Ordering::SeqCst),
        );

        let observation = observation_if_nonterminal(false, || {
            constructed.fetch_add(1, Ordering::SeqCst);
            immediate_control(Arc::clone(&polled), 9)
        });
        let mut ordinary = PinnedStagePump::new(Box::pin(async { 7_u8 }), observation);
        let ordinary_result = tokio::time::timeout(Duration::from_secs(1), async {
            let observation = matches!(ordinary.next_event().await, PumpEvent::ObservationReady(9));
            if !observation {
                return (false, false);
            }
            let stage = matches!(ordinary.next_event().await, PumpEvent::StageReady(7));
            (observation, stage)
        })
        .await;
        drop(ordinary);

        assert!(matches!(terminal_result, Ok(PumpEvent::StageReady(7))));
        assert_eq!(terminal_counts, (0, 0));
        assert_eq!(
            ordinary_result.expect("bounded ordinary cleanup"),
            (true, true)
        );
        assert_eq!(constructed.load(Ordering::SeqCst), 1);
        assert_eq!(polled.load(Ordering::SeqCst), 1);
    }
}

#[cfg(all(test, feature = "market-stream-db-tests"))]
#[path = "market_stream_runtime_tests.rs"]
mod runtime_database_tests;

/// Restricted integration entry, absent from production/default builds.
/// Both remote endpoints must pass the existing explicit loopback validators.
/// The caller supplies a task-owned synthetic domain and a validated window;
/// the real daemon still resolves calendar lineage and owns opaque receipts.
#[cfg(feature = "market-stream-db-tests")]
impl OwnerMarketStreamRuntime {
    #[allow(clippy::too_many_arguments)]
    pub async fn run_loopback_fixture(
        repository: super::market_stream::OwnerMarketStreamRepository,
        calendar: super::intraday::OwnerIntradayQuoteRepository,
        config: OwnerMarketStreamRuntimeConfig,
        domain: kis_client::market_stream_state::MarketStreamDomain,
        approval_origin: &str,
        websocket_endpoint: &str,
        windows: tokio::sync::watch::Receiver<Option<IntradaySessionWindowContract>>,
        shutdown: tokio::sync::watch::Receiver<bool>,
    ) -> Result<MarketStreamRuntimeExit, MarketStreamRuntimeError> {
        // These literals are fixture markers, never real App Key material.
        let approval = kis_client::market_stream_approval::ApprovalClient::for_loopback(
            approval_origin,
            kis_client::secret::Secret::new("WP7_SYNTHETIC_APP_KEY".to_owned()),
            kis_client::secret::Secret::new("WP7_SYNTHETIC_APP_SECRET".to_owned()),
            domain,
            config.credential_generation().to_string(),
        )
        .map_err(|_| MarketStreamRuntimeError::ConfigurationInvalid)?;
        if !approval.matches_runtime_binding(config.slot(), config.credential_generation()) {
            return Err(MarketStreamRuntimeError::ConfigurationInvalid);
        }
        let transport =
            kis_client::market_stream::MarketStreamConfig::loopback(approval, websocket_endpoint)
                .map_err(|_| MarketStreamRuntimeError::ConfigurationInvalid)?
                // The real daemon and its admission hints use wall time. Override
                // the deterministic loopback default for this running fixture.
                .with_clock(std::sync::Arc::new(kis_client::clock::SystemClock));
        let runtime = Self {
            client: kis_client::MarketStreamClient::new(transport),
            repository: repository.runtime_repository(),
            calendar,
            // This fallback is never read while the fixture window scope is present.
            windows_source: IntradaySessionWindowSource::ReleaseV1,
            config,
        };
        integration_fixture_window::WINDOW_SCOPE
            .scope(windows, runtime.run_daemon(shutdown))
            .await
    }
}

#[cfg(feature = "market-stream-db-tests")]
mod integration_fixture_window {
    use collectors::intraday_quotes::{
        IntradaySessionWindowContract, IntradaySessionWindowError, IntradaySessionWindowSource,
    };

    tokio::task_local! {
        pub(super) static WINDOW_SCOPE:
            tokio::sync::watch::Receiver<Option<IntradaySessionWindowContract>>;
    }

    pub(super) fn load_window_contract(
        source: IntradaySessionWindowSource,
    ) -> Result<IntradaySessionWindowContract, IntradaySessionWindowError> {
        match WINDOW_SCOPE.try_with(|receiver| receiver.borrow().clone()) {
            Ok(Some(contract)) => Ok(contract),
            Ok(None) => Err(IntradaySessionWindowError::Missing),
            Err(_) => {
                // Retain the existing D4 unit-test scope exactly when no new scope exists.
                #[cfg(test)]
                {
                    super::runtime_database_tests::load_window_contract(source)
                }
                #[cfg(not(test))]
                {
                    IntradaySessionWindowContract::from_source(source)
                }
            }
        }
    }
}
