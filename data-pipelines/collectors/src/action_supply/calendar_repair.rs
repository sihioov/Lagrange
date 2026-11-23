//! One bounded historical calendar Raw capture for the first D-06 coverage gap.

use std::path::Path;

use domain::{BatchId, TradingDate, UtcTimestamp};
use kis_client::ProductionReadCoordination;
use market_data::contract::{PROVIDER_KIS_CALENDAR, ResponseKind};
use market_data::daily_price_only::calendar::{eligible_calendar_date, prove_calendar_range};
use market_data::daily_price_only::verify::{DailyVerificationError, RawEvidenceSnapshots};
use market_data::ingest::{IngestRequest, ingest_kis_calendar_repair_with_batch_id};
use market_data::{KisProvider, providers::kis::KisRead, storage::RawStore};
use serde::Serialize;

use super::{CONFIG_KEY, Config, SupplyError, calendar_projections, decode_config, snapshots};
use crate::calendar_claim::CalendarDayLock;

const REPAIR_DATE: &str = "2026-11-15";
const CONFIRM_ENV: &str = "KIS_R2_CALENDAR_REPAIR_CONFIRM";
const CONFIRM: &str = "I_APPROVE_20261115_CALENDAR_ONE_GET_RAW_ONLY";

#[derive(Debug, thiserror::Error)]
pub enum CalendarRepairError {
    #[error("CALENDAR_REPAIR_CONFIG")]
    Config,
    #[error("CALENDAR_REPAIR_STATE")]
    State,
    #[error("CALENDAR_REPAIR_UNEXPECTED_COVERAGE")]
    UnexpectedCoverage,
    #[error("CALENDAR_REPAIR_CONSUMED_INCOMPLETE")]
    ConsumedIncomplete,
    #[error("CALENDAR_REPAIR_CAPTURE_FAILED")]
    Capture,
    #[error("CALENDAR_REPAIR_SOURCE:{0}")]
    Source(&'static str),
}

#[derive(Debug, Serialize)]
pub struct CalendarRepairResult {
    outcome: &'static str,
    target_date: TradingDate,
    covered_through: TradingDate,
    batch_id: Option<BatchId>,
    raw_only: bool,
}

pub async fn run_calendar_gap_repair(
    check: bool,
) -> Result<CalendarRepairResult, CalendarRepairError> {
    let env = |key| std::env::var(key).map_err(|_| CalendarRepairError::Config);
    if env("KIS_READ_COORDINATION_MODE")? != "shared_required"
        || env("RESEARCH_RAW_ROOT")? != "/data"
        || ProductionReadCoordination::from_env().is_err()
        || (!check
            && (env(CONFIRM_ENV)? != CONFIRM
                || env("KIS_APP_KEY_FILE")? != "/run/secrets/kis_app_key"
                || env("KIS_APP_SECRET_FILE")? != "/run/secrets/kis_app_secret"))
    {
        return Err(CalendarRepairError::Config);
    }
    let entitlement = env("RESEARCH_ENTITLEMENT_REFERENCE")?;
    let wire = decode_config(&env(CONFIG_KEY)?).map_err(|_| CalendarRepairError::Config)?;
    let config: Config = serde_json::from_slice(&wire).map_err(|_| CalendarRepairError::Config)?;
    config
        .validate(&env("LAGRANGE_CODE_COMMIT")?, &entitlement)
        .map_err(|_| CalendarRepairError::Config)?;
    // The existing DB-free action-range Compose service runs as the Raw owner.
    if !config.enabled || config.raw_owner_uid != 10001 || unsafe { libc::geteuid() } != 10001 {
        return Err(CalendarRepairError::Config);
    }
    let root = Path::new("/data");
    let now = UtcTimestamp::now();
    if check {
        return check_repair(root, &config, &entitlement, now);
    }
    repair_with(root, &config, &entitlement, now, || {
        crate::worker::build_production_one_calendar_get(
            Path::new("/run/secrets/kis_app_key"),
            Path::new("/run/secrets/kis_app_secret"),
        )
        .map_err(|_| CalendarRepairError::Capture)
    })
    .await
}

fn repair_range(now: UtcTimestamp) -> Result<(TradingDate, TradingDate), CalendarRepairError> {
    let target = TradingDate::parse(REPAIR_DATE).map_err(|_| CalendarRepairError::Config)?;
    let through = eligible_calendar_date(now).map_err(source)?;
    if through <= target
        || through
            > target
                .checked_add_days(23)
                .map_err(|_| CalendarRepairError::Config)?
    {
        return Err(CalendarRepairError::UnexpectedCoverage);
    }
    Ok((target, through))
}

fn check_repair(
    root: &Path,
    config: &Config,
    entitlement: &str,
    now: UtcTimestamp,
) -> Result<CalendarRepairResult, CalendarRepairError> {
    let (target, through) = repair_range(now)?;
    let consumed = CalendarDayLock::inspect(&root.join("raw"), target)
        .map_err(|_| CalendarRepairError::State)?;
    let complete = verify_coverage(root, config, entitlement, now, target, through, consumed)?;
    let outcome = match (complete, consumed) {
        (false, _) => "WOULD_CAPTURE_NO_RESERVATION",
        (true, Some(_)) => "VERIFIED_PRIOR_CAPTURE",
        (true, None) => "ALREADY_COVERED",
    };
    Ok(CalendarRepairResult {
        outcome,
        target_date: target,
        covered_through: through,
        batch_id: consumed,
        raw_only: true,
    })
}

async fn repair_with<R: KisRead>(
    root: &Path,
    config: &Config,
    entitlement: &str,
    now: UtcTimestamp,
    make_provider: impl FnOnce() -> Result<KisProvider<R>, CalendarRepairError>,
) -> Result<CalendarRepairResult, CalendarRepairError> {
    let (target, through) = repair_range(now)?;
    let guard = CalendarDayLock::acquire(&root.join("raw"), target)
        .map_err(|_| CalendarRepairError::State)?;
    let consumed = guard.existing().map_err(|_| CalendarRepairError::State)?;
    let complete = verify_coverage(root, config, entitlement, now, target, through, consumed)?;
    if complete {
        return Ok(CalendarRepairResult {
            outcome: if consumed.is_some() {
                "VERIFIED_PRIOR_CAPTURE"
            } else {
                "ALREADY_COVERED"
            },
            target_date: target,
            covered_through: through,
            batch_id: consumed,
            raw_only: true,
        });
    }
    // The lock protects the same target against current and historical
    // collectors. A crash from this point consumes the allowance permanently.
    let batch_id = BatchId::generate();
    guard
        .consume(batch_id)
        .map_err(|_| CalendarRepairError::State)?;
    let provider = make_provider()?;
    let request = IngestRequest::new("kr".into(), target, now);
    ingest_kis_calendar_repair_with_batch_id(
        &RawStore::new(root),
        &provider,
        &request,
        entitlement,
        through,
        batch_id,
    )
    .await
    .map_err(|_| CalendarRepairError::Capture)?;
    verify_coverage(
        root,
        config,
        entitlement,
        now,
        target,
        through,
        Some(batch_id),
    )?;
    Ok(CalendarRepairResult {
        outcome: "RAW_CAPTURE_VERIFIED",
        target_date: target,
        covered_through: through,
        batch_id: Some(batch_id),
        raw_only: true,
    })
}

fn verify_coverage(
    root: &Path,
    config: &Config,
    entitlement: &str,
    now: UtcTimestamp,
    target: TradingDate,
    through: TradingDate,
    consumed: Option<BatchId>,
) -> Result<bool, CalendarRepairError> {
    let snapshots = snapshots(root, config.raw_owner_uid).map_err(supply_source)?;
    let evidence =
        RawEvidenceSnapshots::new(&snapshots.iter().collect::<Vec<_>>(), entitlement, now)
            .map_err(source)?;
    let projections = calendar_projections(&snapshots, &evidence, config).map_err(supply_source)?;
    let all = projections.iter().collect::<Vec<_>>();
    let first = config
        .initial_civil_frontier
        .checked_add_days(1)
        .map_err(|_| CalendarRepairError::Config)?;
    if let Some(batch) = consumed {
        let claimed = all
            .iter()
            .copied()
            .filter(|projection| {
                let file = projection.source_file();
                file.provider == PROVIDER_KIS_CALENDAR
                    && file.batch_date == target
                    && file.batch_id == batch
                    && file.copied_from.is_empty()
            })
            .collect::<Vec<_>>();
        let exact_batch = snapshots.iter().any(|snapshot| {
            snapshot.scope().provider() == PROVIDER_KIS_CALENDAR
                && snapshot.batches().iter().any(|entry| {
                    entry.batch_id() == batch
                        && entry.batch_date() == target
                        && entry.files().len() == 1
                })
        });
        if !exact_batch
            || claimed.len() != 1
            || prove_calendar_range(&claimed, target, through).is_err()
        {
            return Err(CalendarRepairError::ConsumedIncomplete);
        }
        prove_calendar_range(&all, first, through).map_err(source)?;
        return Ok(true);
    }
    match prove_calendar_range(&all, first, through) {
        Ok(_) => return Ok(true),
        Err(DailyVerificationError::CalendarCoverageGap) => {}
        Err(error) => return Err(source(error)),
    }
    let preceding = target
        .checked_add_days(-1)
        .map_err(|_| CalendarRepairError::Config)?;
    prove_calendar_range(&all, first, preceding).map_err(source)?;
    let end = target
        .checked_add_days(23)
        .map_err(|_| CalendarRepairError::Config)?;
    if all.iter().any(|projection| {
        projection
            .days()
            .iter()
            .any(|day| (target..=end).contains(&day.session_date()))
    }) {
        return Err(CalendarRepairError::UnexpectedCoverage);
    }
    // A calendar request for the repair date under any entitlement is
    // already retained evidence of a prior attempt, including KIS EOD.
    let target_query = target.to_iso().replace('-', "");
    if snapshots
        .iter()
        .flat_map(|snapshot| snapshot.batches())
        .any(|batch| {
            batch.files().any(|file| {
                let request = file.request();
                file.kind() == ResponseKind::Calendar
                    && (batch.batch_date() == target
                        || (request.endpoint
                            == kis_client::read_coordination::ReadChannel::CheckHoliday
                                .pair()
                                .0
                            && request
                                .query
                                .iter()
                                .any(|(key, value)| key == "BASS_DT" && value == &target_query)))
            })
        })
    {
        return Err(CalendarRepairError::State);
    }
    Ok(false)
}

fn source(error: DailyVerificationError) -> CalendarRepairError {
    CalendarRepairError::Source(error.code())
}

fn supply_source(error: SupplyError) -> CalendarRepairError {
    match error {
        SupplyError::Source(code) => CalendarRepairError::Source(code),
        _ => CalendarRepairError::Config,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use kis_client::{KisError, MarketDataReply};
    use market_data::contract::{FetchMode, PROVIDER_KIS, RawEnvelope, RequestMetadata};
    use market_data::storage::BatchSpec;
    use serde_json::json;

    use super::super::tests::{config, fixture_until};
    use super::*;

    const RIGHTS: &str = "fixture:licensed";

    #[derive(Debug)]
    struct Response<'a> {
        calls: &'a AtomicUsize,
        first: TradingDate,
        last: TradingDate,
        duplicate_field: bool,
    }

    impl KisRead for Response<'_> {
        async fn get(
            &self,
            path: &str,
            tr_id: &str,
            query: &[(String, String)],
            continuation: Option<&str>,
        ) -> Result<MarketDataReply, KisError> {
            assert_eq!(path, "/uapi/domestic-stock/v1/quotations/chk-holiday");
            assert_eq!(tr_id, "CTCA0903R");
            assert!(continuation.is_none());
            assert_eq!(
                query,
                [
                    ("BASS_DT".into(), "20261115".into()),
                    ("CTX_AREA_FK".into(), String::new()),
                    ("CTX_AREA_NK".into(), String::new())
                ]
            );
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut rows = Vec::new();
            let mut day = self.first;
            while day <= self.last {
                rows.push(json!({"bass_dt": day.to_iso().replace('-', ""), "opnd_yn": "Y"}));
                day = day.checked_add_days(1).unwrap();
            }
            Ok(MarketDataReply {
                body: {
                    let document = json!({"rt_cd":"0","output":rows}).to_string();
                    if self.duplicate_field {
                        document
                            .replacen(r#""opnd_yn":"Y""#, r#""opnd_yn":"N","opnd_yn":"Y""#, 1)
                            .into_bytes()
                    } else {
                        document.into_bytes()
                    }
                },
                // Contradictory continuation metadata is never followed.
                continuation: Some("M".into()),
            })
        }
    }

    fn date(text: &str) -> TradingDate {
        TradingDate::parse(text).unwrap()
    }

    fn now() -> UtcTimestamp {
        UtcTimestamp::parse_rfc3339("2026-11-23T08:00:00Z").unwrap()
    }

    fn forbid() -> Result<KisProvider<Response<'static>>, CalendarRepairError> {
        panic!("verification must not construct a provider");
    }

    #[tokio::test]
    async fn calendar_gap_consumes_a_claim_then_commits_and_verifies_just_one_source() {
        let temp = fixture_until(date("2026-11-14")).await;
        let root = temp.path();
        let calls = AtomicUsize::new(0);
        let result = repair_with(root, &config(), RIGHTS, now(), || {
            // The claim and exclusive lock already exist before provider construction.
            let target = date(REPAIR_DATE);
            assert!(
                root.join("raw/.calendar-bootstrap/2026-11-15.json")
                    .is_file()
            );
            assert!(CalendarDayLock::acquire(&root.join("raw"), target).is_err());
            Ok(KisProvider::kr_etf_core(Response {
                calls: &calls,
                first: target,
                last: date("2026-12-08"),
                duplicate_field: false,
            }))
        })
        .await
        .unwrap();
        assert_eq!(result.outcome, "RAW_CAPTURE_VERIFIED");
        assert_eq!(result.covered_through, date("2026-11-23"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let replay = repair_with(root, &config(), RIGHTS, now(), forbid)
            .await
            .unwrap();
        assert_eq!(replay.outcome, "VERIFIED_PRIOR_CAPTURE");
        assert_eq!(replay.batch_id, result.batch_id);
    }

    #[tokio::test]
    async fn calendar_gap_with_missing_target_is_not_committed_or_replayed() {
        let temp = fixture_until(date("2026-11-14")).await;
        let calls = AtomicUsize::new(0);
        let result = repair_with(temp.path(), &config(), RIGHTS, now(), || {
            Ok(KisProvider::kr_etf_core(Response {
                calls: &calls,
                first: date("2026-11-16"),
                last: date("2026-12-08"),
                duplicate_field: false,
            }))
        })
        .await;
        assert!(matches!(result, Err(CalendarRepairError::Capture)));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let stored = RawStore::new(temp.path())
            .read_reconciled_manifest(PROVIDER_KIS_CALENDAR, "kr")
            .unwrap();
        assert_eq!(stored.len(), 1);
        let replay = repair_with(temp.path(), &config(), RIGHTS, now(), forbid).await;
        assert!(matches!(
            replay,
            Err(CalendarRepairError::ConsumedIncomplete)
        ));
    }

    #[tokio::test]
    async fn calendar_gap_with_a_short_valid_response_is_still_incomplete() {
        let temp = fixture_until(date("2026-11-14")).await;
        let calls = AtomicUsize::new(0);
        let result = repair_with(temp.path(), &config(), RIGHTS, now(), || {
            Ok(KisProvider::kr_etf_core(Response {
                calls: &calls,
                first: date(REPAIR_DATE),
                last: date("2026-11-20"),
                duplicate_field: false,
            }))
        })
        .await;
        assert!(matches!(result, Err(CalendarRepairError::Capture)));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            RawStore::new(temp.path())
                .read_reconciled_manifest(PROVIDER_KIS_CALENDAR, "kr")
                .unwrap()
                .len(),
            1
        );
        let replay = repair_with(temp.path(), &config(), RIGHTS, now(), forbid).await;
        assert!(matches!(
            replay,
            Err(CalendarRepairError::ConsumedIncomplete)
        ));
    }

    #[tokio::test]
    async fn duplicate_provider_keys_consume_the_claim_without_changing_supplier_raw() {
        let temp = fixture_until(date("2026-11-14")).await;
        let root = temp.path();
        let manifest = root.join("raw/manifests/provider=kis-calendar/market=kr/manifest.jsonl");
        let before = std::fs::read(&manifest).unwrap();
        let calls = AtomicUsize::new(0);
        let check = check_repair(root, &config(), RIGHTS, now()).unwrap();
        assert_eq!(check.outcome, "WOULD_CAPTURE_NO_RESERVATION");
        assert!(!root.join("raw/.calendar-bootstrap").exists());

        let result = repair_with(root, &config(), RIGHTS, now(), || {
            Ok(KisProvider::kr_etf_core(Response {
                calls: &calls,
                first: date(REPAIR_DATE),
                last: date("2026-12-08"),
                duplicate_field: true,
            }))
        })
        .await;
        assert!(matches!(result, Err(CalendarRepairError::Capture)));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(std::fs::read(&manifest).unwrap(), before);
        assert!(
            root.join("raw/.calendar-bootstrap/2026-11-15.json")
                .is_file()
        );
        assert!(matches!(
            check_repair(root, &config(), RIGHTS, now()),
            Err(CalendarRepairError::ConsumedIncomplete)
        ));
        assert!(matches!(
            repair_with(root, &config(), RIGHTS, now(), forbid).await,
            Err(CalendarRepairError::ConsumedIncomplete)
        ));
    }

    #[tokio::test]
    async fn retained_composite_calendar_blocks_a_second_get_under_other_entitlement() {
        let temp = fixture_until(date("2026-11-14")).await;
        let store = RawStore::new(temp.path());
        let target = date(REPAIR_DATE);
        let batch = BatchId::generate();
        let (endpoint, _) = kis_client::read_coordination::ReadChannel::CheckHoliday.pair();
        let file = RawEnvelope::new(
            batch,
            ResponseKind::Calendar,
            "calendar-page-01.json",
            br#"{"rt_cd":"0","output":[{"bass_dt":"20261115","opnd_yn":"N"}]}"#.to_vec(),
            now(),
            RequestMetadata {
                endpoint: endpoint.into(),
                query: vec![("BASS_DT".into(), "20261115".into())],
                headers: vec![],
                mode: FetchMode::Credentialed,
            },
        );
        store
            .store_batch(
                &BatchSpec {
                    provider: PROVIDER_KIS,
                    market: "kr",
                    date: &target,
                    batch_id: batch,
                    entitlement_reference: Some("fixture:another-right"),
                    mode: FetchMode::Credentialed,
                },
                &[file],
            )
            .unwrap();
        assert!(matches!(
            check_repair(temp.path(), &config(), RIGHTS, now()),
            Err(CalendarRepairError::State)
        ));
        assert!(!temp.path().join("raw/.calendar-bootstrap").exists());
    }

    #[tokio::test]
    async fn calendar_gap_that_is_already_covered_leaves_the_allowance_unused() {
        let temp = fixture_until(date("2026-11-23")).await;
        let result = repair_with(temp.path(), &config(), RIGHTS, now(), forbid)
            .await
            .unwrap();
        assert_eq!(result.outcome, "ALREADY_COVERED");
        assert!(
            !temp
                .path()
                .join("raw/.calendar-bootstrap/2026-11-15.json")
                .exists()
        );
    }
}
