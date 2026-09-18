//! Operator-only current-day calendar acquisition, independent of EOD curation.
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::Utc;
use domain::{BatchId, TradingDate, UtcTimestamp};
use kis_client::{ProductionReadCoordination, ReadCoordinationMode};
use market_data::contract::{
    FetchMode, MARKET_KR, PROVIDER_KIS, PROVIDER_KIS_CALENDAR, ResponseKind,
};
use market_data::ingest::{IngestRequest, ingest_kis_calendar_with_batch_id};
use market_data::normalize::normalize_kis_calendar_batch;
use market_data::publication::CalendarPublicationBundle;
use market_data::storage::{ManifestEntry, RawStore};
use serde::Serialize;

use crate::calendar_claim::CalendarDayLock;
use crate::intraday_quotes::{
    IntradayMarketState, IntradaySessionWindowContract, IntradaySessionWindowSource,
};
use crate::sink::PostgresPublicationSink;
use crate::worker::{
    AppEnvironment, ResearchWorkerConfig, build_postgres_pool, build_production_kis_provider,
    current_kst_date,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct CalendarBootstrapError(pub &'static str);

#[derive(Debug, Serialize)]
pub struct CalendarBootstrapSummary {
    pub schema_version: u32,
    pub date: TradingDate,
    pub source_batch_id: BatchId,
    pub normalized_batch_id: BatchId,
    pub reused_existing_source: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalendarSourceMode {
    Explicit(BatchId),
    ReuseExisting,
}

struct CalendarCoreConfig<'a> {
    raw_root: &'a Path,
    entitlement_reference: &'a str,
    session_windows: Option<&'a IntradaySessionWindowContract>,
}

enum CalendarSourcePlan {
    Existing(ManifestEntry),
    Capture(BatchId),
}

#[async_trait]
trait CalendarSourceCapture: Send + Sync {
    async fn capture(
        &self,
        store: &RawStore,
        request: &IngestRequest,
        entitlement_reference: &str,
        source_batch_id: BatchId,
    ) -> Result<ManifestEntry, CalendarBootstrapError>;
}

struct ProductionCalendarSourceCapture<'a> {
    config: &'a ResearchWorkerConfig,
}

#[async_trait]
impl CalendarSourceCapture for ProductionCalendarSourceCapture<'_> {
    async fn capture(
        &self,
        store: &RawStore,
        request: &IngestRequest,
        entitlement_reference: &str,
        source_batch_id: BatchId,
    ) -> Result<ManifestEntry, CalendarBootstrapError> {
        let provider = build_production_kis_provider(self.config)
            .map_err(|_| CalendarBootstrapError("CALENDAR_PROVIDER_CONFIG_INVALID"))?;
        ingest_kis_calendar_with_batch_id(
            store,
            &provider,
            request,
            Some(entitlement_reference),
            source_batch_id,
        )
        .await
        .map(|outcome| outcome.entry)
        .map_err(|_| CalendarBootstrapError("CALENDAR_CAPTURE_FAILED"))
    }
}

#[async_trait]
trait CalendarPublisher: Send + Sync {
    async fn publish(
        &self,
        bundle: &CalendarPublicationBundle,
    ) -> Result<(), CalendarBootstrapError>;
}

struct PostgresCalendarPublisher {
    pool: sqlx::PgPool,
}

#[async_trait]
impl CalendarPublisher for PostgresCalendarPublisher {
    async fn publish(
        &self,
        bundle: &CalendarPublicationBundle,
    ) -> Result<(), CalendarBootstrapError> {
        PostgresPublicationSink::new(self.pool.clone())
            .publish_calendar(bundle)
            .await
            .map(|_| ())
            .map_err(|_| CalendarBootstrapError("CALENDAR_PUBLICATION_FAILED"))
    }
}

struct ReuseOnlyCalendarSourceCapture;

#[async_trait]
impl CalendarSourceCapture for ReuseOnlyCalendarSourceCapture {
    async fn capture(
        &self,
        _store: &RawStore,
        _request: &IngestRequest,
        _entitlement_reference: &str,
        _source_batch_id: BatchId,
    ) -> Result<ManifestEntry, CalendarBootstrapError> {
        Err(CalendarBootstrapError("CALENDAR_REUSE_CAPTURE_FORBIDDEN"))
    }
}

const REUSE_EXISTING_KIS_CREDENTIAL_SENTINEL: &str =
    "reuse-existing-source-does-not-read-kis-credentials";

fn load_reuse_existing_config(
    values: &HashMap<String, String>,
) -> Result<ResearchWorkerConfig, CalendarBootstrapError> {
    let key_path = values.get("KIS_APP_KEY_FILE").map(PathBuf::from);
    let secret_path = values.get("KIS_APP_SECRET_FILE").map(PathBuf::from);
    let database_password_path = values.get("DB_PASSWORD_FILE").map(PathBuf::from);
    if database_password_path.as_deref() == key_path.as_deref()
        || database_password_path.as_deref() == secret_path.as_deref()
    {
        return Err(CalendarBootstrapError("CALENDAR_CONFIG_INVALID"));
    }
    ResearchWorkerConfig::from_map_with_reader(values, |path| {
        if key_path.as_deref() == Some(path) || secret_path.as_deref() == Some(path) {
            return Ok(REUSE_EXISTING_KIS_CREDENTIAL_SENTINEL.to_owned());
        }
        std::fs::read_to_string(path)
    })
    .map_err(|_| CalendarBootstrapError("CALENDAR_CONFIG_INVALID"))
}

pub async fn run_calendar_once(
    values: &HashMap<String, String>,
    date: TradingDate,
    source_mode: CalendarSourceMode,
) -> Result<CalendarBootstrapSummary, CalendarBootstrapError> {
    let now = Utc::now();
    if values.get("OWNER_INTRADAY_QUOTES_MODE").map(String::as_str) != Some("owner_only")
        || values
            .get("OWNER_INTRADAY_SESSION_WINDOWS_SOURCE")
            .map(String::as_str)
            != Some("operational_v1")
        || date != current_kst_date(now)
    {
        return Err(CalendarBootstrapError(
            "CALENDAR_BOOTSTRAP_MODE_OR_DATE_INVALID",
        ));
    }
    let coordination = ProductionReadCoordination::from_env()
        .map_err(|_| CalendarBootstrapError("CALENDAR_COORDINATION_INVALID"))?;
    if coordination.mode() != ReadCoordinationMode::SharedRequired {
        return Err(CalendarBootstrapError(
            "CALENDAR_SHARED_COORDINATION_REQUIRED",
        ));
    }
    let windows =
        IntradaySessionWindowContract::from_source(IntradaySessionWindowSource::OperationalV1)
            .map_err(|_| CalendarBootstrapError("CALENDAR_SESSION_WINDOW_INVALID"))?;
    if windows.state_at(now, false) == IntradayMarketState::Unknown {
        return Err(CalendarBootstrapError("CALENDAR_SESSION_WINDOW_INVALID"));
    }
    // Explicit acquisition retains the existing complete config path. Reuse
    // validates the same production config but substitutes an in-memory
    // sentinel for KIS key/secret reads; it cannot construct a provider.
    let config = match source_mode {
        CalendarSourceMode::Explicit(_) => ResearchWorkerConfig::from_map(values)
            .map_err(|_| CalendarBootstrapError("CALENDAR_CONFIG_INVALID"))?,
        CalendarSourceMode::ReuseExisting => load_reuse_existing_config(values)?,
    };
    if config.app_env != AppEnvironment::Production
        || config.fetch_mode != FetchMode::Credentialed
        || config.database.user != "research_writer"
    {
        return Err(CalendarBootstrapError("CALENDAR_CONFIG_INVALID"));
    }
    let core_config = CalendarCoreConfig {
        raw_root: &config.raw_root,
        entitlement_reference: &config.entitlement_reference,
        session_windows: Some(&windows),
    };
    let capture = ProductionCalendarSourceCapture { config: &config };
    let pool = build_postgres_pool(&config.database);
    let publisher = PostgresCalendarPublisher { pool: pool.clone() };
    let result =
        run_calendar_once_core(&core_config, date, source_mode, &capture, &publisher).await;
    pool.close().await;
    result
}

/// Runs the standalone core with the production PostgreSQL calendar publisher
/// while allowing an offline integration test to supply its migrated pool.
/// The capture adapter deliberately rejects any capture, so this seam can only
/// exercise an already committed source/claim and cannot become a new provider
/// or Raw acquisition path.
#[doc(hidden)]
pub async fn run_calendar_once_with_postgres_pool_for_test(
    raw_root: &Path,
    date: TradingDate,
    source_mode: CalendarSourceMode,
    entitlement_reference: &str,
    pool: sqlx::PgPool,
) -> Result<CalendarBootstrapSummary, CalendarBootstrapError> {
    let config = CalendarCoreConfig {
        raw_root,
        entitlement_reference,
        session_windows: None,
    };
    let capture = ReuseOnlyCalendarSourceCapture;
    let publisher = PostgresCalendarPublisher { pool };
    run_calendar_once_core(&config, date, source_mode, &capture, &publisher).await
}

async fn run_calendar_once_core(
    config: &CalendarCoreConfig<'_>,
    date: TradingDate,
    source_mode: CalendarSourceMode,
    capture: &dyn CalendarSourceCapture,
    publisher: &dyn CalendarPublisher,
) -> Result<CalendarBootstrapSummary, CalendarBootstrapError> {
    let store = RawStore::new(config.raw_root);
    let guard = CalendarDayLock::acquire(&config.raw_root.join("raw"), date)
        .map_err(|_| CalendarBootstrapError("CALENDAR_ATTEMPT_STATE_INVALID_OR_BUSY"))?;
    let source_plan = resolve_calendar_source(&store, &guard, date, source_mode)?;
    let now = Utc::now();
    if current_kst_date(now) != date
        || config
            .session_windows
            .is_some_and(|windows| windows.state_at(now, false) == IntradayMarketState::Unknown)
    {
        return Err(CalendarBootstrapError("CALENDAR_SESSION_WINDOW_INVALID"));
    }
    let reused_existing_source = matches!(&source_plan, CalendarSourcePlan::Existing(_));
    let source = match source_plan {
        CalendarSourcePlan::Existing(source) => source,
        CalendarSourcePlan::Capture(source_batch_id) => {
            guard
                .consume(source_batch_id)
                .map_err(|_| CalendarBootstrapError("CALENDAR_ATTEMPT_STATE_INVALID_OR_BUSY"))?;
            let request = IngestRequest {
                market: MARKET_KR.to_owned(),
                date,
                now: UtcTimestamp::now(),
            };
            let source = capture
                .capture(
                    &store,
                    &request,
                    config.entitlement_reference,
                    source_batch_id,
                )
                .await?;
            if source.batch_id != source_batch_id {
                return Err(CalendarBootstrapError("CALENDAR_SOURCE_IDENTITY_INVALID"));
            }
            source
        }
    };
    validate_calendar_source(&store, &source, date, config.entitlement_reference)?;
    let normalized = normalize_kis_calendar_batch(&store, &source)
        .map_err(|_| CalendarBootstrapError("CALENDAR_NORMALIZATION_FAILED"))?;
    let bundle = CalendarPublicationBundle::from_raw(&store, &normalized.entry)
        .map_err(|_| CalendarBootstrapError("CALENDAR_PUBLICATION_EVIDENCE_INVALID"))?;
    if current_kst_date(Utc::now()) != date {
        return Err(CalendarBootstrapError(
            "CALENDAR_BOOTSTRAP_DATE_ROLLED_OVER",
        ));
    }
    publisher.publish(&bundle).await?;
    Ok(CalendarBootstrapSummary {
        schema_version: 1,
        date,
        source_batch_id: source.batch_id,
        normalized_batch_id: normalized.entry.batch_id,
        reused_existing_source,
    })
}

fn resolve_calendar_source(
    store: &RawStore,
    guard: &CalendarDayLock,
    date: TradingDate,
    source_mode: CalendarSourceMode,
) -> Result<CalendarSourcePlan, CalendarBootstrapError> {
    let claim = guard
        .existing()
        .map_err(|_| CalendarBootstrapError("CALENDAR_ATTEMPT_STATE_INVALID_OR_BUSY"))?;
    let entries = store
        .read_reconciled_manifest(PROVIDER_KIS_CALENDAR, MARKET_KR)
        .map_err(|_| CalendarBootstrapError("CALENDAR_RAW_INVALID"))?;
    let target_sources: Vec<_> = entries
        .iter()
        .filter(|entry| entry.date == date)
        .cloned()
        .collect();
    if target_sources.len() > 1 {
        return Err(CalendarBootstrapError("CALENDAR_MULTIPLE_SOURCES"));
    }
    let target_source = target_sources.into_iter().next();

    match source_mode {
        CalendarSourceMode::Explicit(source_batch_id) => {
            if claim.is_some_and(|claimed| claimed != source_batch_id) {
                return Err(CalendarBootstrapError("CALENDAR_ATTEMPT_ID_CONFLICT"));
            }
            if entries
                .iter()
                .any(|entry| entry.batch_id == source_batch_id && entry.date != date)
            {
                return Err(CalendarBootstrapError("CALENDAR_SOURCE_IDENTITY_INVALID"));
            }
            if target_source
                .as_ref()
                .is_some_and(|source| source.batch_id != source_batch_id)
            {
                return Err(CalendarBootstrapError("CALENDAR_ATTEMPT_ID_CONFLICT"));
            }
            match target_source {
                Some(source) => {
                    if claim != Some(source_batch_id) {
                        return Err(CalendarBootstrapError("CALENDAR_ATTEMPT_STATE_MISSING"));
                    }
                    Ok(CalendarSourcePlan::Existing(source))
                }
                None => {
                    if claim.is_some() {
                        return Err(CalendarBootstrapError("CALENDAR_ATTEMPT_INDETERMINATE"));
                    }
                    // A prior EOD calendar acquisition on this KST day consumes
                    // the broker's once-daily allowance too. Do not recapture it.
                    let legacy = store
                        .read_reconciled_manifest(PROVIDER_KIS, MARKET_KR)
                        .map_err(|_| CalendarBootstrapError("CALENDAR_RAW_INVALID"))?;
                    if legacy.iter().any(|entry| {
                        current_kst_date(entry.retrieved_at.as_datetime()) == date
                            && entry
                                .files
                                .iter()
                                .any(|file| file.kind == ResponseKind::Calendar)
                    }) {
                        return Err(CalendarBootstrapError("CALENDAR_ALREADY_CAPTURED_BY_EOD"));
                    }
                    Ok(CalendarSourcePlan::Capture(source_batch_id))
                }
            }
        }
        CalendarSourceMode::ReuseExisting => match (claim, target_source) {
            (Some(claim), Some(source)) if claim == source.batch_id => {
                Ok(CalendarSourcePlan::Existing(source))
            }
            (Some(_), Some(_)) => Err(CalendarBootstrapError("CALENDAR_ATTEMPT_ID_CONFLICT")),
            (Some(_), None) => Err(CalendarBootstrapError("CALENDAR_ATTEMPT_INDETERMINATE")),
            (None, Some(_)) => Err(CalendarBootstrapError("CALENDAR_ATTEMPT_STATE_MISSING")),
            (None, None) => Err(CalendarBootstrapError("CALENDAR_EXISTING_SOURCE_MISSING")),
        },
    }
}

fn validate_calendar_source(
    store: &RawStore,
    source: &ManifestEntry,
    date: TradingDate,
    entitlement_reference: &str,
) -> Result<(), CalendarBootstrapError> {
    if source.provider != PROVIDER_KIS_CALENDAR
        || source.market != MARKET_KR
        || source.date != date
        || source.mode != FetchMode::Credentialed
        || source.entitlement_reference.as_deref() != Some(entitlement_reference)
        || current_kst_date(source.retrieved_at.as_datetime()) != date
        || source.retrieved_at.as_datetime() > Utc::now()
        || source.files.len() != 1
        || source.files[0].kind != ResponseKind::Calendar
        || source.files[0].request.mode != FetchMode::Credentialed
        || source.files[0].response_continuation.is_some()
        || source.files[0].copied_from.is_some()
    {
        return Err(CalendarBootstrapError("CALENDAR_SOURCE_IDENTITY_INVALID"));
    }
    store
        .read_batch_bytes(PROVIDER_KIS_CALENDAR, MARKET_KR, source)
        .map_err(|_| CalendarBootstrapError("CALENDAR_SOURCE_IDENTITY_INVALID"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::pipeline::ensure_kis_calendar_source;
    use async_trait::async_trait;
    use kis_client::{KisError, MarketDataReply};
    use market_data::ingest::IngestRequest;
    use market_data::providers::kis::{KisProvider, KisRead};
    use market_data::publication::CalendarEvidence;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use tokio::sync::Notify;

    #[derive(Debug, Clone)]
    struct CalendarFixtureReader {
        calls: Arc<AtomicUsize>,
    }

    impl CalendarFixtureReader {
        fn new(calls: Arc<AtomicUsize>) -> Self {
            Self { calls }
        }
    }

    impl KisRead for CalendarFixtureReader {
        async fn get(
            &self,
            _path: &str,
            _tr_id: &str,
            query: &[(String, String)],
            _continuation: Option<&str>,
        ) -> Result<MarketDataReply, KisError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let date = query
                .iter()
                .find(|(key, _)| key == "BASS_DT")
                .map(|(_, value)| value.as_str())
                .unwrap_or_default();
            let body =
                format!(r#"{{"rt_cd":"0","output":[{{"bass_dt":"{date}","opnd_yn":"Y"}}]}}"#);
            Ok(MarketDataReply {
                body: body.into_bytes(),
                continuation: None,
            })
        }
    }

    #[derive(Debug)]
    struct FixtureCapture {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl CalendarSourceCapture for FixtureCapture {
        async fn capture(
            &self,
            store: &RawStore,
            request: &IngestRequest,
            entitlement_reference: &str,
            source_batch_id: BatchId,
        ) -> Result<ManifestEntry, CalendarBootstrapError> {
            let provider = KisProvider::kr_etf_core(CalendarFixtureReader::new(self.calls.clone()));
            ingest_kis_calendar_with_batch_id(
                store,
                &provider,
                request,
                Some(entitlement_reference),
                source_batch_id,
            )
            .await
            .map(|outcome| outcome.entry)
            .map_err(|_| CalendarBootstrapError("TEST_CALENDAR_CAPTURE_FAILED"))
        }
    }

    #[derive(Debug)]
    struct BlockingFixtureCapture {
        capture_calls: Arc<AtomicUsize>,
        broker_calls: Arc<AtomicUsize>,
        started: Arc<Notify>,
        release: Arc<Notify>,
    }

    #[async_trait]
    impl CalendarSourceCapture for BlockingFixtureCapture {
        async fn capture(
            &self,
            store: &RawStore,
            request: &IngestRequest,
            entitlement_reference: &str,
            source_batch_id: BatchId,
        ) -> Result<ManifestEntry, CalendarBootstrapError> {
            self.capture_calls.fetch_add(1, Ordering::SeqCst);
            self.started.notify_one();
            self.release.notified().await;
            let provider =
                KisProvider::kr_etf_core(CalendarFixtureReader::new(self.broker_calls.clone()));
            ingest_kis_calendar_with_batch_id(
                store,
                &provider,
                request,
                Some(entitlement_reference),
                source_batch_id,
            )
            .await
            .map(|outcome| outcome.entry)
            .map_err(|_| CalendarBootstrapError("TEST_CALENDAR_CAPTURE_FAILED"))
        }
    }

    #[derive(Debug, Default)]
    struct RejectCapture {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl CalendarSourceCapture for RejectCapture {
        async fn capture(
            &self,
            _store: &RawStore,
            _request: &IngestRequest,
            _entitlement_reference: &str,
            _source_batch_id: BatchId,
        ) -> Result<ManifestEntry, CalendarBootstrapError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(CalendarBootstrapError("TEST_UNEXPECTED_PROVIDER_CAPTURE"))
        }
    }

    #[derive(Debug, Default)]
    struct RecordingPublisher {
        calls: AtomicUsize,
        evidence: Mutex<Vec<CalendarEvidence>>,
    }

    #[async_trait]
    impl CalendarPublisher for RecordingPublisher {
        async fn publish(
            &self,
            bundle: &CalendarPublicationBundle,
        ) -> Result<(), CalendarBootstrapError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.evidence
                .lock()
                .expect("calendar evidence lock")
                .push(bundle.calendar_evidence().clone());
            Ok(())
        }
    }

    fn core_config<'a>(
        raw_root: &'a Path,
        entitlement_reference: &'a str,
    ) -> CalendarCoreConfig<'a> {
        CalendarCoreConfig {
            raw_root,
            entitlement_reference,
            session_windows: None,
        }
    }

    fn current_date() -> TradingDate {
        current_kst_date(Utc::now())
    }

    fn current_request(date: TradingDate) -> IngestRequest {
        IngestRequest::new(MARKET_KR.to_owned(), date, UtcTimestamp::now())
    }

    fn source_bytes(store: &RawStore, source: &ManifestEntry) -> Vec<Vec<u8>> {
        store
            .read_batch_bytes(PROVIDER_KIS_CALENDAR, MARKET_KR, source)
            .expect("source bytes")
            .into_iter()
            .map(|file| file.bytes)
            .collect()
    }

    #[tokio::test]
    async fn calendar_standalone_core_reuses_eod_first_source_without_provider_or_raw_mutation() {
        let temp = tempfile::tempdir().expect("raw tempdir");
        let data_root = temp.path().join("data");
        std::fs::create_dir_all(data_root.join("raw")).expect("raw root");
        let store = RawStore::new(&data_root);
        let date = current_date();
        let entitlement = "entitlement://calendar-bootstrap-test";
        let eod_calls = Arc::new(AtomicUsize::new(0));
        let eod_provider = KisProvider::kr_etf_core(CalendarFixtureReader::new(eod_calls.clone()));
        let source = ensure_kis_calendar_source(
            &store,
            &eod_provider,
            &current_request(date),
            Some(entitlement),
        )
        .await
        .expect("EOD-first source acquisition")
        .expect("dedicated EOD calendar source");
        assert_eq!(eod_calls.load(Ordering::SeqCst), 1);

        let source_manifest_before =
            std::fs::read(store.manifest_path(PROVIDER_KIS_CALENDAR, MARKET_KR))
                .expect("source manifest");
        let source_bytes_before = source_bytes(&store, &source);
        let claim_path = data_root
            .join("raw/.calendar-bootstrap")
            .join(format!("{}.json", date.to_iso()));
        let claim_before = std::fs::read(&claim_path).expect("calendar claim");
        let capture = RejectCapture::default();
        let publisher = RecordingPublisher::default();
        let config = core_config(&data_root, entitlement);

        let first = run_calendar_once_core(
            &config,
            date,
            CalendarSourceMode::ReuseExisting,
            &capture,
            &publisher,
        )
        .await
        .expect("standalone reuse after EOD");
        let second = run_calendar_once_core(
            &config,
            date,
            CalendarSourceMode::ReuseExisting,
            &capture,
            &publisher,
        )
        .await
        .expect("standalone reuse replay");

        assert!(first.reused_existing_source);
        assert_eq!(first.source_batch_id, source.batch_id);
        assert_eq!(first.source_batch_id, second.source_batch_id);
        assert_eq!(first.normalized_batch_id, second.normalized_batch_id);
        assert_eq!(capture.calls.load(Ordering::SeqCst), 0);
        assert_eq!(publisher.calls.load(Ordering::SeqCst), 2);
        let evidence = publisher.evidence.lock().expect("calendar evidence lock");
        assert_eq!(evidence.len(), 2);
        assert!(evidence.iter().all(|value| {
            value.source_batch_id == first.normalized_batch_id
                && value.retrieved_at == source.retrieved_at
        }));
        drop(evidence);
        assert_eq!(
            std::fs::read(store.manifest_path(PROVIDER_KIS_CALENDAR, MARKET_KR))
                .expect("source manifest replay"),
            source_manifest_before
        );
        assert_eq!(source_bytes(&store, &source), source_bytes_before);
        assert_eq!(
            std::fs::read(&claim_path).expect("claim replay"),
            claim_before
        );
    }

    #[tokio::test]
    async fn calendar_standalone_core_explicit_bootstrap_replay_and_wrong_uuid_are_strict() {
        let temp = tempfile::tempdir().expect("raw tempdir");
        let data_root = temp.path().join("data");
        std::fs::create_dir_all(data_root.join("raw")).expect("raw root");
        let date = current_date();
        let entitlement = "entitlement://calendar-bootstrap-test";
        let capture_calls = Arc::new(AtomicUsize::new(0));
        let capture = FixtureCapture {
            calls: capture_calls.clone(),
        };
        let publisher = RecordingPublisher::default();
        let config = core_config(&data_root, entitlement);
        let source_batch_id = BatchId::generate();

        let first = run_calendar_once_core(
            &config,
            date,
            CalendarSourceMode::Explicit(source_batch_id),
            &capture,
            &publisher,
        )
        .await
        .expect("explicit standalone bootstrap");
        let replay = run_calendar_once_core(
            &config,
            date,
            CalendarSourceMode::Explicit(source_batch_id),
            &capture,
            &publisher,
        )
        .await
        .expect("explicit standalone replay");
        let wrong = run_calendar_once_core(
            &config,
            date,
            CalendarSourceMode::Explicit(BatchId::generate()),
            &capture,
            &publisher,
        )
        .await
        .expect_err("wrong explicit UUID must conflict");

        assert!(!first.reused_existing_source);
        assert!(replay.reused_existing_source);
        assert_eq!(first.source_batch_id, source_batch_id);
        assert_eq!(replay.source_batch_id, source_batch_id);
        assert_eq!(capture_calls.load(Ordering::SeqCst), 1);
        assert_eq!(wrong.0, "CALENDAR_ATTEMPT_ID_CONFLICT");
    }

    #[tokio::test]
    async fn calendar_standalone_core_concurrent_attempts_capture_once() {
        let temp = tempfile::tempdir().expect("raw tempdir");
        let data_root = temp.path().join("data");
        std::fs::create_dir_all(data_root.join("raw")).expect("raw root");
        let date = current_date();
        let entitlement = "entitlement://calendar-bootstrap-test";
        let capture_calls = Arc::new(AtomicUsize::new(0));
        let broker_calls = Arc::new(AtomicUsize::new(0));
        let capture = BlockingFixtureCapture {
            capture_calls: capture_calls.clone(),
            broker_calls: broker_calls.clone(),
            started: Arc::new(Notify::new()),
            release: Arc::new(Notify::new()),
        };
        let started = capture.started.clone();
        let release = capture.release.clone();
        let publisher = RecordingPublisher::default();
        let config = core_config(&data_root, entitlement);
        let source_batch_id = BatchId::generate();

        let first = run_calendar_once_core(
            &config,
            date,
            CalendarSourceMode::Explicit(source_batch_id),
            &capture,
            &publisher,
        );
        let second = run_calendar_once_core(
            &config,
            date,
            CalendarSourceMode::Explicit(source_batch_id),
            &capture,
            &publisher,
        );
        let release_capture = async move {
            started.notified().await;
            release.notify_one();
        };
        let (first, second, ()) = tokio::join!(first, second, release_capture);
        let successes = [first.as_ref(), second.as_ref()]
            .into_iter()
            .filter(|result| result.is_ok())
            .count();
        assert_eq!(successes, 1);
        assert!(
            first
                .as_ref()
                .err()
                .is_some_and(|error| { error.0 == "CALENDAR_ATTEMPT_STATE_INVALID_OR_BUSY" })
                || second
                    .as_ref()
                    .err()
                    .is_some_and(|error| { error.0 == "CALENDAR_ATTEMPT_STATE_INVALID_OR_BUSY" })
        );
        assert_eq!(capture_calls.load(Ordering::SeqCst), 1);
        assert_eq!(broker_calls.load(Ordering::SeqCst), 1);
        assert_eq!(publisher.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn calendar_standalone_core_rejects_multiple_same_date_sources() {
        let temp = tempfile::tempdir().expect("raw tempdir");
        let data_root = temp.path().join("data");
        std::fs::create_dir_all(data_root.join("raw")).expect("raw root");
        let store = RawStore::new(&data_root);
        let date = current_date();
        let entitlement = "entitlement://calendar-bootstrap-test";
        let calls = Arc::new(AtomicUsize::new(0));
        let first_id = BatchId::generate();
        let first = FixtureCapture {
            calls: calls.clone(),
        };
        let first_config = core_config(&data_root, entitlement);
        run_calendar_once_core(
            &first_config,
            date,
            CalendarSourceMode::Explicit(first_id),
            &first,
            &RecordingPublisher::default(),
        )
        .await
        .expect("first source");

        let second_id = BatchId::generate();
        let second_provider = KisProvider::kr_etf_core(CalendarFixtureReader::new(calls.clone()));
        ingest_kis_calendar_with_batch_id(
            &store,
            &second_provider,
            &current_request(date),
            Some(entitlement),
            second_id,
        )
        .await
        .expect("duplicate source fixture");

        let error = run_calendar_once_core(
            &first_config,
            date,
            CalendarSourceMode::ReuseExisting,
            &RejectCapture::default(),
            &RecordingPublisher::default(),
        )
        .await
        .expect_err("multiple source IDs must fail closed");
        assert_eq!(error.0, "CALENDAR_MULTIPLE_SOURCES");
    }

    #[tokio::test]
    async fn calendar_standalone_core_reuse_rejects_missing_and_claim_only_state() {
        let entitlement = "entitlement://calendar-bootstrap-test";
        let empty = tempfile::tempdir().expect("empty raw tempdir");
        let empty_root = empty.path().join("data");
        std::fs::create_dir_all(empty_root.join("raw")).expect("empty raw root");
        let empty_config = core_config(&empty_root, entitlement);
        let capture = RejectCapture::default();
        let publisher = RecordingPublisher::default();
        let missing = run_calendar_once_core(
            &empty_config,
            current_date(),
            CalendarSourceMode::ReuseExisting,
            &capture,
            &publisher,
        )
        .await
        .expect_err("reuse without a source must fail closed");
        assert_eq!(missing.0, "CALENDAR_EXISTING_SOURCE_MISSING");

        let claim_only = tempfile::tempdir().expect("claim-only tempdir");
        let claim_root = claim_only.path().join("data");
        std::fs::create_dir_all(claim_root.join("raw")).expect("claim-only raw root");
        let date = current_date();
        let claimed_id = BatchId::generate();
        let guard = CalendarDayLock::acquire(&claim_root.join("raw"), date).expect("day lock");
        guard.consume(claimed_id).expect("claim");
        drop(guard);
        let claim_config = core_config(&claim_root, entitlement);
        let indeterminate = run_calendar_once_core(
            &claim_config,
            date,
            CalendarSourceMode::ReuseExisting,
            &capture,
            &publisher,
        )
        .await
        .expect_err("claim-only reuse must remain indeterminate");
        assert_eq!(indeterminate.0, "CALENDAR_ATTEMPT_INDETERMINATE");
        assert_eq!(capture.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn calendar_standalone_core_rejects_tampered_existing_source() {
        let temp = tempfile::tempdir().expect("raw tempdir");
        let data_root = temp.path().join("data");
        std::fs::create_dir_all(data_root.join("raw")).expect("raw root");
        let store = RawStore::new(&data_root);
        let date = current_date();
        let entitlement = "entitlement://calendar-bootstrap-test";
        let calls = Arc::new(AtomicUsize::new(0));
        let source = ensure_kis_calendar_source(
            &store,
            &KisProvider::kr_etf_core(CalendarFixtureReader::new(calls.clone())),
            &current_request(date),
            Some(entitlement),
        )
        .await
        .expect("EOD source")
        .expect("dedicated source");
        let path = store
            .batch_dir(
                PROVIDER_KIS_CALENDAR,
                MARKET_KR,
                &source.date,
                &source.batch_id,
            )
            .join(&source.files[0].file_name);
        std::fs::write(path, b"tampered").expect("tamper fixture");
        let config = core_config(&data_root, entitlement);
        let error = run_calendar_once_core(
            &config,
            date,
            CalendarSourceMode::ReuseExisting,
            &RejectCapture::default(),
            &RecordingPublisher::default(),
        )
        .await
        .expect_err("tampered source must fail closed");
        assert!(matches!(
            error.0,
            "CALENDAR_RAW_INVALID" | "CALENDAR_SOURCE_IDENTITY_INVALID"
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn reuse_existing_config_does_not_read_kis_credential_files() {
        let temp = tempfile::tempdir().expect("config tempdir");
        let password = temp.path().join("db-password");
        std::fs::write(&password, "database-password").expect("database password");
        let mut values = HashMap::new();
        values.insert("APP_ENV".to_owned(), "production".to_owned());
        values.insert("RESEARCH_FETCH_MODE".to_owned(), "credentialed".to_owned());
        values.insert(
            "RESEARCH_RAW_ROOT".to_owned(),
            temp.path().join("raw").display().to_string(),
        );
        values.insert(
            "RESEARCH_CURATED_ROOT".to_owned(),
            temp.path().join("curated").display().to_string(),
        );
        values.insert(
            "RESEARCH_ENTITLEMENT_REFERENCE".to_owned(),
            "entitlement://calendar-bootstrap-test".to_owned(),
        );
        values.insert("DB_HOST".to_owned(), "127.0.0.1".to_owned());
        values.insert("DB_PORT".to_owned(), "5432".to_owned());
        values.insert("DB_NAME".to_owned(), "research".to_owned());
        values.insert("DB_USER".to_owned(), "research_writer".to_owned());
        values.insert(
            "DB_PASSWORD_FILE".to_owned(),
            password.display().to_string(),
        );
        values.insert(
            "KIS_APP_KEY_FILE".to_owned(),
            temp.path().join("missing-key").display().to_string(),
        );
        values.insert(
            "KIS_APP_SECRET_FILE".to_owned(),
            temp.path().join("missing-secret").display().to_string(),
        );
        let config = load_reuse_existing_config(&values).expect("reuse config");
        assert_eq!(config.fetch_mode, FetchMode::Credentialed);
        assert_eq!(config.database.user, "research_writer");
    }
}
