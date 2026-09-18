//! Operator-only current-day calendar acquisition, independent of EOD curation.
use std::collections::HashMap;

use chrono::Utc;
use domain::{BatchId, TradingDate, UtcTimestamp};
use kis_client::{ProductionReadCoordination, ReadCoordinationMode};
use market_data::contract::{
    FetchMode, MARKET_KR, PROVIDER_KIS, PROVIDER_KIS_CALENDAR, ResponseKind,
};
use market_data::ingest::{IngestRequest, ingest_kis_calendar_with_batch_id};
use market_data::normalize::normalize_kis_calendar_batch;
use market_data::publication::CalendarPublicationBundle;
use market_data::storage::RawStore;
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

pub async fn run_calendar_once(
    values: &HashMap<String, String>,
    date: TradingDate,
    source_batch_id: BatchId,
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
    // No credentials, token client, Raw writes or DB pool before the day proof.
    let config = ResearchWorkerConfig::from_map(values)
        .map_err(|_| CalendarBootstrapError("CALENDAR_CONFIG_INVALID"))?;
    if config.app_env != AppEnvironment::Production
        || config.fetch_mode != FetchMode::Credentialed
        || config.database.user != "research_writer"
    {
        return Err(CalendarBootstrapError("CALENDAR_CONFIG_INVALID"));
    }
    let store = RawStore::new(&config.raw_root);
    let guard = CalendarDayLock::acquire(&config.raw_root.join("raw"), date)
        .map_err(|_| CalendarBootstrapError("CALENDAR_ATTEMPT_STATE_INVALID_OR_BUSY"))?;
    let claim = guard
        .existing()
        .map_err(|_| CalendarBootstrapError("CALENDAR_ATTEMPT_STATE_INVALID_OR_BUSY"))?;
    if claim.is_some_and(|claimed| claimed != source_batch_id) {
        return Err(CalendarBootstrapError("CALENDAR_ATTEMPT_ID_CONFLICT"));
    }
    let entries = store
        .read_reconciled_manifest(PROVIDER_KIS_CALENDAR, MARKET_KR)
        .map_err(|_| CalendarBootstrapError("CALENDAR_RAW_INVALID"))?;
    if entries
        .iter()
        .any(|entry| entry.date == date && entry.batch_id != source_batch_id)
    {
        return Err(CalendarBootstrapError("CALENDAR_ATTEMPT_ID_CONFLICT"));
    }
    let existing = entries
        .into_iter()
        .find(|entry| entry.batch_id == source_batch_id);
    let reused_existing_source = existing.is_some();
    let source = match existing {
        Some(source) => {
            if claim != Some(source_batch_id) {
                return Err(CalendarBootstrapError("CALENDAR_ATTEMPT_STATE_MISSING"));
            }
            source
        }
        None => {
            if claim.is_some() {
                return Err(CalendarBootstrapError("CALENDAR_ATTEMPT_INDETERMINATE"));
            }
            // A prior EOD calendar acquisition on this KST day consumes the
            // broker's once-daily allowance too. Do not recapture it here.
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
            let provider = build_production_kis_provider(&config)
                .map_err(|_| CalendarBootstrapError("CALENDAR_PROVIDER_CONFIG_INVALID"))?;
            let now = Utc::now();
            if current_kst_date(now) != date
                || windows.state_at(now, false) == IntradayMarketState::Unknown
            {
                return Err(CalendarBootstrapError("CALENDAR_SESSION_WINDOW_INVALID"));
            }
            guard
                .consume(source_batch_id)
                .map_err(|_| CalendarBootstrapError("CALENDAR_ATTEMPT_STATE_INVALID_OR_BUSY"))?;
            let request = IngestRequest {
                market: MARKET_KR.to_owned(),
                date,
                now: UtcTimestamp::now(),
            };
            ingest_kis_calendar_with_batch_id(
                &store,
                &provider,
                &request,
                Some(&config.entitlement_reference),
                source_batch_id,
            )
            .await
            .map_err(|_| CalendarBootstrapError("CALENDAR_CAPTURE_FAILED"))?
            .entry
        }
    };
    if source.date != date
        || source.mode != FetchMode::Credentialed
        || source.entitlement_reference.as_deref() != Some(config.entitlement_reference.as_str())
        || current_kst_date(source.retrieved_at.as_datetime()) != date
        || source.retrieved_at.as_datetime() > Utc::now()
    {
        return Err(CalendarBootstrapError("CALENDAR_SOURCE_IDENTITY_INVALID"));
    }
    let normalized = normalize_kis_calendar_batch(&store, &source)
        .map_err(|_| CalendarBootstrapError("CALENDAR_NORMALIZATION_FAILED"))?;
    let bundle = CalendarPublicationBundle::from_raw(&store, &normalized.entry)
        .map_err(|_| CalendarBootstrapError("CALENDAR_PUBLICATION_EVIDENCE_INVALID"))?;
    if current_kst_date(Utc::now()) != date {
        return Err(CalendarBootstrapError(
            "CALENDAR_BOOTSTRAP_DATE_ROLLED_OVER",
        ));
    }
    let pool = build_postgres_pool(&config.database);
    let result = PostgresPublicationSink::new(pool.clone())
        .publish_calendar(&bundle)
        .await;
    pool.close().await;
    result.map_err(|_| CalendarBootstrapError("CALENDAR_PUBLICATION_FAILED"))?;
    Ok(CalendarBootstrapSummary {
        schema_version: 1,
        date,
        source_batch_id,
        normalized_batch_id: normalized.entry.batch_id,
        reused_existing_source,
    })
}
