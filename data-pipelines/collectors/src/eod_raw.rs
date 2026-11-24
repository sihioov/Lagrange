//! Explicit EOD source acquisition for the independent R2 price-only consumer.
//!
//! A committed source is not canonical publication, a verified daily input, or
//! recommendation authority. Legacy normalization remains a separate contract.

use std::collections::BTreeMap;

use domain::{BatchId, TradingDate, UtcTimestamp};
use market_data::{
    contract::{MARKET_KR, PROVIDER_KIS, PROVIDER_KIS_CALENDAR, ResponseKind},
    ingest::{
        IngestError, IngestRequest, ResponseValidationDiagnostic,
        ingest_kis_raw_eod_with_calendar_source, ingest_kis_recovery_eod,
        verified_existing_kis_eod_source, verified_existing_kis_recovery_eod_source,
        verify_committed_kis_calendar_original,
    },
    providers::kis::{KisProvider, KisRead},
    storage::{ManifestEntry, RawStore},
};

use crate::{
    PipelineError, calendar_claim::CalendarDayLock, pipeline::ensure_kis_calendar_source_locked,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EodRawSource {
    pub batch_id: BatchId,
    pub reused: bool,
    pub calendar_claim_preexisting: bool,
    pub calendar_batch_id: Option<BatchId>,
}

fn invalid(detail: &'static str) -> PipelineError {
    PipelineError::Ingest {
        source: IngestError::ResponseShape {
            detail: detail.to_owned(),
        },
    }
}

/// Read and verify an existing committed source before constructing any
/// new delivery. Action-only range evidence is not an EOD source. A conflicting
/// or malformed price-bearing entry is an error, never permission to refetch.
pub fn existing_eod_source(
    store: &RawStore,
    request: &IngestRequest,
    entitlement: &str,
) -> Result<Option<ManifestEntry>, PipelineError> {
    verified_existing_kis_eod_source(store, request, entitlement)
        .map_err(|source| PipelineError::Ingest { source })
}

/// Reuse exact immutable Raw, or capture once through the existing fixed ETF11
/// provider. New acquisition shares the durable calendar claim.
/// No canonical normalizer, publication sink, or database is constructed here.
pub async fn capture_eod_source<R: KisRead>(
    store: &RawStore,
    provider: &KisProvider<R>,
    request: &IngestRequest,
    entitlement: &str,
    current_day: bool,
) -> Result<EodRawSource, PipelineError> {
    capture_eod_source_with_calendar(store, provider, request, entitlement, current_day, None)
        .await
        .map(|(source, _)| source)
}

/// Historical multi-date callers retain the returned committed calendar
/// original. Its coverage is checked before any non-calendar request; a claim
/// already reserved for this target takes precedence and cannot be bypassed.
pub async fn capture_eod_source_with_calendar<R: KisRead>(
    store: &RawStore,
    provider: &KisProvider<R>,
    request: &IngestRequest,
    entitlement: &str,
    current_day: bool,
    historical_calendar: Option<&ManifestEntry>,
) -> Result<(EodRawSource, Option<ManifestEntry>), PipelineError> {
    if request.market != MARKET_KR || entitlement.trim().is_empty() {
        return Err(invalid("EOD_RAW_REQUEST_INVALID"));
    }
    if current_day && historical_calendar.is_some() {
        return Err(invalid("EOD_RAW_CURRENT_DAY_REQUIRES_OWN_CALENDAR"));
    }
    let guard = CalendarDayLock::acquire(&store.root().join("raw"), request.date)
        .map_err(|_| invalid("EOD_RAW_SOURCE_STATE_INVALID_OR_BUSY"))?;
    // Only the executing path reconciles crash residue. The check command
    // requires existing committed evidence and never creates a lock/manifest.
    store
        .read_reconciled_manifest(PROVIDER_KIS, MARKET_KR)
        .map_err(|source| PipelineError::Manifest { source })?;
    let existing = existing_eod_source(store, request, entitlement)?;
    let has_claim = guard
        .existing()
        .map_err(|_| invalid("EOD_RAW_CALENDAR_STATE_INVALID"))?
        .is_some();
    let calendar =
        if current_day || has_claim || (existing.is_none() && historical_calendar.is_none()) {
            ensure_kis_calendar_source_locked(store, provider, request, Some(entitlement), &guard)
                .await?
        } else {
            historical_calendar.cloned()
        };
    if let Some(source) = existing {
        return Ok((
            EodRawSource {
                batch_id: source.batch_id,
                reused: true,
                calendar_claim_preexisting: has_claim,
                calendar_batch_id: calendar.as_ref().map(|entry| entry.batch_id),
            },
            calendar,
        ));
    }
    let calendar = calendar.ok_or_else(|| invalid("EOD_RAW_CALENDAR_ORIGINAL_REQUIRED"))?;
    let captured = ingest_kis_raw_eod_with_calendar_source(
        store,
        provider,
        request,
        Some(entitlement),
        &calendar,
    )
    .await
    .map_err(|source| PipelineError::Ingest { source })?;
    Ok((
        EodRawSource {
            batch_id: captured.batch_id,
            reused: false,
            calendar_claim_preexisting: has_claim,
            calendar_batch_id: Some(calendar.batch_id),
        },
        Some(calendar),
    ))
}

/// A checked historical EOD workload. Keep the shared calendar namespace lock
/// until all missing EOD batches are committed and their receipts are emitted.
pub(crate) struct ExistingCalendarRecovery {
    pub(crate) targets: Vec<ExistingCalendarTarget>,
    _guard: CalendarDayLock,
}

pub(crate) struct ExistingCalendarTarget {
    pub(crate) date: TradingDate,
    pub(crate) existing: Option<EodRawSource>,
    calendar: ManifestEntry,
    claim_preexisting: bool,
}

fn calendar_recovery_error(code: &'static str) -> PipelineError {
    PipelineError::Ingest {
        source: IngestError::MalformedResponse {
            kind: ResponseKind::Calendar,
            reason: code.to_owned(),
            diagnostic: Some(ResponseValidationDiagnostic {
                code,
                endpoint: "/uapi/domestic-stock/v1/quotations/chk-holiday".to_owned(),
                file_name: "calendar-page-01.json".to_owned(),
            }),
        },
    }
}

pub(crate) fn prepare_existing_calendar_recovery(
    store: &RawStore,
    dates: &[TradingDate],
    entitlement: &str,
    now: UtcTimestamp,
) -> Result<ExistingCalendarRecovery, PipelineError> {
    let (Some(first), Some(last)) = (dates.first(), dates.last()) else {
        return Err(invalid("EOD_RAW_REQUEST_INVALID"));
    };
    let guard = CalendarDayLock::acquire(&store.root().join("raw"), *first)
        .map_err(|_| calendar_recovery_error("KIS_CALENDAR_ATTEMPT_STATE_INVALID_OR_BUSY"))?;
    let calendar_entries = store
        .read_committed_manifest(PROVIDER_KIS_CALENDAR, MARKET_KR)
        .map_err(|source| PipelineError::Manifest { source })?;
    let mut originals = BTreeMap::new();
    for entry in calendar_entries.into_iter().filter(|entry| {
        entry.date <= *last && entry.entitlement_reference.as_deref() == Some(entitlement)
    }) {
        let claim = guard
            .existing_for(entry.date)
            .map_err(|_| calendar_recovery_error("KIS_CALENDAR_ATTEMPT_STATE_INVALID_OR_BUSY"))?;
        if claim.is_some_and(|batch| batch != entry.batch_id) || originals.contains_key(&entry.date)
        {
            return Err(calendar_recovery_error("KIS_CALENDAR_ATTEMPT_ID_CONFLICT"));
        }
        let rows = verify_committed_kis_calendar_original(store, &entry, entitlement, now)
            .map_err(|source| PipelineError::Ingest { source })?;
        originals.insert(entry.date, (entry, rows));
    }
    // The executing Raw workflow reconciles source commit residue before
    // interpreting a missing batch as permission to fetch one.
    store
        .read_reconciled_manifest(PROVIDER_KIS, MARKET_KR)
        .map_err(|source| PipelineError::Manifest { source })?;
    let mut targets = Vec::with_capacity(dates.len());
    for &date in dates {
        let claim = guard
            .existing_for(date)
            .map_err(|_| calendar_recovery_error("KIS_CALENDAR_ATTEMPT_STATE_INVALID_OR_BUSY"))?;
        if claim.is_some() && !originals.contains_key(&date) {
            return Err(calendar_recovery_error("KIS_CALENDAR_SOURCE_INDETERMINATE"));
        }
        let coverage = originals
            .range(..=date)
            .rev()
            .filter_map(|(_, (entry, rows))| rows.get(&date).map(|&open| (entry, open)));
        let (calendar, open) = coverage
            .clone()
            .next()
            .ok_or_else(|| calendar_recovery_error("KIS_CALENDAR_TARGET_MISSING"))?;
        if coverage.clone().any(|(_, other_open)| open != other_open) {
            return Err(calendar_recovery_error("KIS_CALENDAR_SOURCE_CONFLICT"));
        }
        if !open {
            return Err(calendar_recovery_error("KIS_CALENDAR_TARGET_CLOSED"));
        }
        let request = IngestRequest::new(MARKET_KR.to_owned(), date, now);
        let existing = verified_existing_kis_recovery_eod_source(store, &request, entitlement)
            .map_err(|source| PipelineError::Ingest { source })?
            .map(|source| checked_existing_recovery_source(&source, coverage, claim))
            .transpose()?;
        targets.push(ExistingCalendarTarget {
            date,
            existing,
            calendar: calendar.clone(),
            claim_preexisting: claim.is_some(),
        });
    }
    Ok(ExistingCalendarRecovery {
        targets,
        _guard: guard,
    })
}

fn checked_existing_recovery_source<'a>(
    source: &ManifestEntry,
    mut coverage: impl Iterator<Item = (&'a ManifestEntry, bool)>,
    claim: Option<BatchId>,
) -> Result<EodRawSource, PipelineError> {
    let conflict = || calendar_recovery_error("EOD_RAW_CALENDAR_ORIGINAL_CONFLICT");
    let calendar = source
        .files
        .iter()
        .find(|file| file.kind == ResponseKind::Calendar)
        .ok_or_else(conflict)?;
    let link = calendar
        .copied_from
        .as_ref()
        .ok_or_else(|| calendar_recovery_error("EOD_RAW_CALENDAR_ORIGINAL_REQUIRED"))?;
    // Preserve the already captured witness when a new agreeing original was
    // added later. A consumed claim for the target still names its only witness.
    let (original, _) = coverage
        .find(|(original, open)| {
            *open
                && original.batch_id == link.batch_id
                && claim.is_none_or(|claimed| {
                    original.date == source.date && original.batch_id == claimed
                })
        })
        .ok_or_else(conflict)?;
    let file = &original.files[0];
    if link.provider != original.provider
        || link.market != original.market
        || link.retrieved_at != original.retrieved_at
        || link.file_name != file.file_name
        || link.content_hash != file.content_hash
        || source.retrieved_at < original.retrieved_at
        || calendar.file_name != file.file_name
        || calendar.content_hash != file.content_hash
        || calendar.size_bytes != file.size_bytes
        || calendar.request != file.request
        || calendar.response_continuation != file.response_continuation
    {
        return Err(conflict());
    }
    Ok(EodRawSource {
        batch_id: source.batch_id,
        reused: true,
        calendar_claim_preexisting: claim.is_some(),
        calendar_batch_id: Some(original.batch_id),
    })
}

impl ExistingCalendarTarget {
    pub(crate) async fn capture<R: KisRead>(
        &self,
        store: &RawStore,
        provider: &KisProvider<R>,
        entitlement: &str,
    ) -> Result<EodRawSource, PipelineError> {
        let request = IngestRequest::new(MARKET_KR.to_owned(), self.date, UtcTimestamp::now());
        let capture =
            ingest_kis_recovery_eod(store, provider, &request, entitlement, &self.calendar)
                .await
                .map_err(|source| PipelineError::Ingest { source })?;
        Ok(EodRawSource {
            batch_id: capture.batch_id,
            reused: false,
            calendar_claim_preexisting: self.claim_preexisting,
            calendar_batch_id: Some(self.calendar.batch_id),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use kis_client::{KisError, MarketDataReply};
    use market_data::ingest::ingest_kis_calendar_with_batch_id;
    use serde_json::json;

    use super::*;

    const ENTITLEMENT: &str = "test-entitlement";

    #[derive(Debug)]
    struct ReplyFixture<'a> {
        calendar: &'a [(&'a str, &'a str)],
        calls: &'a AtomicUsize,
        price_fault: Option<PriceFault>,
    }

    #[derive(Debug, Clone, Copy)]
    enum PriceFault {
        EmptyBars,
        WrongBarDate,
        WrongReferenceSymbol,
    }

    impl KisRead for ReplyFixture<'_> {
        async fn get(
            &self,
            path: &str,
            _tr_id: &str,
            query: &[(String, String)],
            _continuation: Option<&str>,
        ) -> Result<MarketDataReply, KisError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let field = |key| {
                query
                    .iter()
                    .find(|(name, _)| name == key)
                    .unwrap()
                    .1
                    .as_str()
            };
            let body = if path.ends_with("/chk-holiday") {
                assert!(
                    !self.calendar.is_empty(),
                    "recovery must never query the calendar"
                );
                let rows: Vec<_> = self
                    .calendar
                    .iter()
                    .map(|(date, open)| json!({"bass_dt":date,"opnd_yn":open}))
                    .collect();
                json!({"rt_cd":"0","output":rows})
            } else if path.ends_with("/inquire-daily-itemchartprice") {
                let symbol = field("FID_INPUT_ISCD");
                let date = match self.price_fault {
                    Some(PriceFault::WrongBarDate) => "20261021",
                    _ => field("FID_INPUT_DATE_1"),
                };
                let rows = match self.price_fault {
                    Some(PriceFault::EmptyBars) => vec![],
                    _ => vec![json!({
                        "stck_bsop_date":date,"stck_oprc":"100",
                        "stck_hgpr":"102","stck_lwpr":"99","stck_clpr":"101","acml_vol":"1"
                    })],
                };
                json!({"rt_cd":"0","output1":{"stck_shrn_iscd":symbol},"output2":rows})
            } else if path.ends_with("/inquire-price") {
                let symbol = match self.price_fault {
                    Some(PriceFault::WrongReferenceSymbol) => "000001",
                    _ => field("FID_INPUT_ISCD"),
                };
                json!({"rt_cd":"0","output":{"stck_shrn_iscd":symbol}})
            } else if path.contains("/ksdinfo/") {
                json!({"rt_cd":"0","output1":[]})
            } else {
                panic!("unexpected fixture endpoint");
            };
            Ok(MarketDataReply {
                body: serde_json::to_vec(&body).unwrap(),
                continuation: None,
            })
        }
    }

    fn date(value: &str) -> TradingDate {
        TradingDate::parse(value).unwrap()
    }

    async fn original(
        store: &RawStore,
        origin: TradingDate,
        rows: &[(&str, &str)],
        entitlement: &str,
        claimed: bool,
    ) -> ManifestEntry {
        let batch = BatchId::generate();
        if claimed {
            let guard = CalendarDayLock::acquire(&store.root().join("raw"), origin).unwrap();
            guard.consume(batch).unwrap();
        }
        let calls = AtomicUsize::new(0);
        let fixture = ReplyFixture {
            calendar: rows,
            calls: &calls,
            price_fault: None,
        };
        ingest_kis_calendar_with_batch_id(
            store,
            &KisProvider::kr_etf_core(fixture),
            &IngestRequest::new(MARKET_KR.to_owned(), origin, UtcTimestamp::now()),
            Some(entitlement),
            batch,
        )
        .await
        .unwrap()
        .entry
    }

    fn expect_error(result: Result<ExistingCalendarRecovery, PipelineError>, code: &str) {
        let Err(error) = result else {
            panic!("expected offline refusal {code}");
        };
        assert!(error.to_string().contains(code), "{error}");
    }

    #[tokio::test]
    async fn eod_raw_existing_calendars_switch_and_replay_without_calendar_gets() {
        let temp = tempfile::tempdir().unwrap();
        let store = RawStore::new(temp.path());
        std::fs::create_dir(store.root().join("raw")).unwrap();
        let first = original(
            &store,
            date("2026-10-22"),
            &[("20261022", "Y"), ("20261113", "Y"), ("20261114", "N")],
            ENTITLEMENT,
            true,
        )
        .await;
        let second = original(
            &store,
            date("2026-11-15"),
            &[("20261115", "N"), ("20261116", "Y"), ("20261123", "Y")],
            ENTITLEMENT,
            true,
        )
        .await;
        let dates = [
            date("2026-10-22"),
            date("2026-11-13"),
            date("2026-11-16"),
            date("2026-11-23"),
        ];
        let recovery =
            prepare_existing_calendar_recovery(&store, &dates, ENTITLEMENT, UtcTimestamp::now())
                .unwrap();
        let calls = AtomicUsize::new(0);
        let fixture = KisProvider::kr_etf_core(ReplyFixture {
            calendar: &[],
            calls: &calls,
            price_fault: None,
        });
        let expected = [
            first.batch_id,
            first.batch_id,
            second.batch_id,
            second.batch_id,
        ];
        for (target, batch) in recovery.targets.iter().zip(expected) {
            assert!(target.existing.is_none());
            let source = target.capture(&store, &fixture, ENTITLEMENT).await.unwrap();
            assert_eq!(source.calendar_batch_id, Some(batch));
            assert_eq!(source.calendar_claim_preexisting, target.date == dates[0]);
        }
        drop(recovery);
        let replay =
            prepare_existing_calendar_recovery(&store, &dates, ENTITLEMENT, UtcTimestamp::now())
                .unwrap();
        for (target, batch) in replay.targets.iter().zip(expected) {
            let source = target.existing.unwrap();
            assert!(source.reused);
            assert_eq!(source.calendar_batch_id, Some(batch));
        }
        drop(replay);
        let calendar_entries = store
            .read_committed_manifest(PROVIDER_KIS_CALENDAR, MARKET_KR)
            .unwrap();
        assert_eq!(calendar_entries, vec![first, second]);
        assert_eq!(calls.load(Ordering::Relaxed), dates.len() * 99);
        let new_original =
            original(&store, dates[1], &[("20261113", "Y")], ENTITLEMENT, false).await;
        let replay =
            prepare_existing_calendar_recovery(&store, &dates, ENTITLEMENT, UtcTimestamp::now())
                .unwrap();
        assert_eq!(
            replay.targets[1].existing.unwrap().calendar_batch_id,
            Some(expected[1])
        );
        drop(replay);
        {
            let lock = CalendarDayLock::acquire(&store.root().join("raw"), dates[1]).unwrap();
            lock.consume(new_original.batch_id).unwrap();
        }
        expect_error(
            prepare_existing_calendar_recovery(&store, &dates, ENTITLEMENT, UtcTimestamp::now()),
            "EOD_RAW_CALENDAR_ORIGINAL_CONFLICT",
        );
    }

    #[tokio::test]
    async fn eod_raw_existing_calendars_fail_closed_on_missing_conflicting_or_unsettled_sources() {
        let temp = tempfile::tempdir().unwrap();
        let store = RawStore::new(temp.path());
        std::fs::create_dir(store.root().join("raw")).unwrap();
        let first = date("2026-10-22");
        let next = date("2026-10-23");
        let later = date("2026-10-26");
        original(
            &store,
            first,
            &[("20261022", "Y"), ("20261023", "Y")],
            ENTITLEMENT,
            false,
        )
        .await;
        let foreign = original(
            &store,
            next,
            &[("20261023", "Y"), ("20261026", "Y")],
            "other-entitlement",
            false,
        )
        .await;
        expect_error(
            prepare_existing_calendar_recovery(
                &store,
                &[first, later],
                ENTITLEMENT,
                UtcTimestamp::now(),
            ),
            "KIS_CALENDAR_TARGET_MISSING",
        );
        {
            let lock = CalendarDayLock::acquire(&store.root().join("raw"), later).unwrap();
            lock.consume(BatchId::generate()).unwrap();
        }
        expect_error(
            prepare_existing_calendar_recovery(
                &store,
                &[first, next, later],
                ENTITLEMENT,
                UtcTimestamp::now(),
            ),
            "KIS_CALENDAR_SOURCE_INDETERMINATE",
        );
        original(&store, next, &[("20261023", "N")], ENTITLEMENT, false).await;
        expect_error(
            prepare_existing_calendar_recovery(
                &store,
                &[first, next],
                ENTITLEMENT,
                UtcTimestamp::now(),
            ),
            "KIS_CALENDAR_SOURCE_CONFLICT",
        );
        {
            let lock = CalendarDayLock::acquire(&store.root().join("raw"), next).unwrap();
            lock.consume(foreign.batch_id).unwrap();
        }
        expect_error(
            prepare_existing_calendar_recovery(
                &store,
                &[first, next],
                ENTITLEMENT,
                UtcTimestamp::now(),
            ),
            "KIS_CALENDAR_ATTEMPT_ID_CONFLICT",
        );
        assert!(
            store
                .read_committed_manifest(PROVIDER_KIS, MARKET_KR)
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn eod_raw_recovery_rejects_invalid_target_prices_on_capture_and_replay() {
        for (fault, code) in [
            (PriceFault::EmptyBars, "EOD_RAW_RECOVERY_TARGET_BAR_INVALID"),
            (
                PriceFault::WrongBarDate,
                "EOD_RAW_RECOVERY_TARGET_BAR_INVALID",
            ),
            (
                PriceFault::WrongReferenceSymbol,
                "EOD_RAW_RECOVERY_REFERENCE_IDENTITY_INVALID",
            ),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let store = RawStore::new(temp.path());
            std::fs::create_dir(store.root().join("raw")).unwrap();
            let target = date("2026-10-22");
            let calendar = original(&store, target, &[("20261022", "Y")], ENTITLEMENT, false).await;
            let calls = AtomicUsize::new(0);
            let provider = KisProvider::kr_etf_core(ReplyFixture {
                calendar: &[],
                calls: &calls,
                price_fault: Some(fault),
            });
            let recovery = prepare_existing_calendar_recovery(
                &store,
                &[target],
                ENTITLEMENT,
                UtcTimestamp::now(),
            )
            .unwrap();
            let error = recovery.targets[0]
                .capture(&store, &provider, ENTITLEMENT)
                .await
                .unwrap_err();
            assert!(error.to_string().contains(code), "{fault:?}: {error}");
            assert!(
                store
                    .read_committed_manifest(PROVIDER_KIS, MARKET_KR)
                    .unwrap()
                    .is_empty()
            );
            drop(recovery);

            // Previously committed shape-only evidence also has to pass the recovery check.
            let request = IngestRequest::new(MARKET_KR.to_owned(), target, UtcTimestamp::now());
            ingest_kis_raw_eod_with_calendar_source(
                &store,
                &provider,
                &request,
                Some(ENTITLEMENT),
                &calendar,
            )
            .await
            .unwrap();
            expect_error(
                prepare_existing_calendar_recovery(
                    &store,
                    &[target],
                    ENTITLEMENT,
                    UtcTimestamp::now(),
                ),
                code,
            );
        }
    }

    #[tokio::test]
    async fn eod_raw_existing_calendars_reject_legacy_receipts_without_provenance() {
        let temp = tempfile::tempdir().unwrap();
        let store = RawStore::new(temp.path());
        std::fs::create_dir(store.root().join("raw")).unwrap();
        let target = date("2026-10-23");
        original(
            &store,
            date("2026-10-22"),
            &[("20261022", "Y"), ("20261023", "Y")],
            ENTITLEMENT,
            false,
        )
        .await;
        let calls = AtomicUsize::new(0);
        let provider = KisProvider::kr_etf_core(ReplyFixture {
            calendar: &[("20261023", "Y")],
            calls: &calls,
            price_fault: None,
        });
        let request = IngestRequest::new(MARKET_KR.to_owned(), target, UtcTimestamp::now());
        market_data::ingest::ingest_kis_bundle(&store, &provider, &request, Some(ENTITLEMENT))
            .await
            .unwrap();
        expect_error(
            prepare_existing_calendar_recovery(&store, &[target], ENTITLEMENT, UtcTimestamp::now()),
            "EOD_RAW_CALENDAR_ORIGINAL_REQUIRED",
        );
    }
}
