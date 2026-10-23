//! Bounded Raw-only recurring corporate-action acquisition.
//! The protected installer supplies configuration; only independently verified
//! committed Raw advances coverage. A consumed attempt is never replayed.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path};
use std::time::{Duration, Instant};

use chrono::{FixedOffset, Timelike};
use domain::{BatchId, ContentHash, TradingDate, UtcTimestamp};
use market_data::contract::{ResponseKind, SourceFileReference};
use market_data::daily_price_only::{
    calendar::{expected_calendar_session, project_calendar_candidates},
    reconstruct::reconstruct_actions,
    source::{ApprovedRawScope, CommittedRawReader, ScanLimitValues, ScanLimits},
    supply::{
        ActionSupplyProposal, SupplyDayStateClaim, SupplyProposalConfig,
        plan_action_supply_proposal,
    },
    verify::{DailyVerificationError, RawEvidenceSnapshots, resolve_original},
};
use market_data::ingest::ingest_kis_action_range_with_batch_id;
use market_data::storage::RawStore;
use market_data::{KisActionRangeScope, KisProvider, providers::kis::KisRead};
use rustix::fs::{Mode, OFlags, mkdirat, open, openat};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const CONFIG_KEY: &str = "RESEARCH_R2_ACTION_SUPPLY_CONFIG_HEX";
const REGISTRY: &[u8] = include_bytes!(
    "../../../configs/evidence/kis-historical-price-only-v3-approved-artifacts.json"
);
const WIRE_RESERVATION: u16 = 280;

// JSON quotes are deliberately forbidden by the production dotenv parser.
// This is a bounded transport encoding of nonsecret configuration, not encryption.
fn decode_config(value: &str) -> Result<Vec<u8>, SupplyError> {
    if value.is_empty()
        || value.len() > 8192
        || !value.len().is_multiple_of(2)
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(SupplyError::Config);
    }
    (0..value.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i + 2], 16).map_err(|_| SupplyError::Config))
        .collect()
}

#[derive(Debug, thiserror::Error)]
pub enum SupplyError {
    #[error("ACTION_SUPPLY_CONFIG")]
    Config,
    #[error("ACTION_SUPPLY_STATE")]
    State,
    #[error("ACTION_SUPPLY_CONSUMED_INCOMPLETE")]
    ConsumedIncomplete,
    #[error("ACTION_SUPPLY_GAP_LIMIT")]
    GapLimit,
    #[error("ACTION_SUPPLY_SOURCE:{0}")]
    Source(&'static str),
    #[error("ACTION_SUPPLY_CAPTURE")]
    Capture,
    #[error("ACTION_SUPPLY_TIMEOUT_CONSUMED")]
    Timeout,
    #[error("ACTION_SUPPLY_PLANNING_DEADLINE")]
    PlanningDeadline,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Config {
    schema: String,
    enabled: bool,
    release_commit: String,
    owner: Uuid,
    policy_sha256: ContentHash,
    activation_date: TradingDate,
    base_candidate_sha256: ContentHash,
    initial_civil_frontier: TradingDate,
    entitlement_reference_sha256: ContentHash,
    raw_owner_uid: u32,
    timeout_seconds: u64,
}

impl Config {
    fn validate(&self, commit: &str, entitlement: &str) -> Result<(), SupplyError> {
        let registry: serde_json::Value =
            serde_json::from_slice(REGISTRY).map_err(|_| SupplyError::Config)?;
        let records = registry["approved_artifacts"]
            .as_array()
            .filter(|r| r.len() == 1)
            .ok_or(SupplyError::Config)?;
        let base = &records[0];
        if self.schema != "r2-action-supply-v1"
            || self.release_commit != commit
            || commit.len() != 40
            || !commit
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || self.owner.is_nil()
            || self.raw_owner_uid == u32::MAX
            || !(1..=300).contains(&self.timeout_seconds)
            || self.entitlement_reference_sha256 != ContentHash::from_bytes(entitlement.as_bytes())
            || base["candidate_content_sha256"].as_str()
                != Some(self.base_candidate_sha256.as_str())
            || base["range_end"].as_str() != Some(self.initial_civil_frontier.to_iso().as_str())
            || self.activation_date <= self.initial_civil_frontier
        {
            return Err(SupplyError::Config);
        }
        Ok(())
    }

    fn hash(&self) -> Result<ContentHash, SupplyError> {
        serde_json::to_vec(self)
            .map(|bytes| ContentHash::from_bytes(&bytes))
            .map_err(|_| SupplyError::Config)
    }
}

#[derive(Debug, Serialize)]
pub struct SupplyResult {
    pub outcome: &'static str,
    pub start: Option<TradingDate>,
    pub end: Option<TradingDate>,
    pub batch_id: Option<BatchId>,
    /// Conservative allowance burned before dispatch, never actual wire count.
    pub reserved_wire_gets: u16,
    pub raw_only: bool,
    pub admitted: bool,
}

fn outcome(reason: &'static str) -> SupplyResult {
    SupplyResult {
        outcome: reason,
        start: None,
        end: None,
        batch_id: None,
        reserved_wire_gets: 0,
        raw_only: true,
        admitted: false,
    }
}

// Same measured source-read ceilings as the protected daily-input installation.
fn scan_limits() -> Result<ScanLimits, SupplyError> {
    ScanLimits::new(ScanLimitValues {
        max_manifest_bytes: 8_388_608,
        max_manifest_records: 128,
        max_batches: 128,
        max_files_per_batch: 256,
        max_files: 8192,
        max_batch_metadata_bytes: 262_144,
        max_source_file_bytes: 131_072,
        max_retained_bytes: 67_108_864,
        max_open_descriptors: 16_384,
        max_data_root_bytes: 4096,
        max_data_root_components: 32,
        max_metadata_json_depth: 16,
    })
    .map_err(|error| SupplyError::Source(error.code()))
}

struct SourcePlan {
    start: Option<TradingDate>,
    end: TradingDate,
}

/// The snapshots and all shared manifest locks die before this function returns.
fn source_plan(
    root: &Path,
    config: &Config,
    entitlement: &str,
    today: TradingDate,
    now: UtcTimestamp,
    consumed: Option<&Claim>,
) -> Result<SourcePlan, SupplyError> {
    let snapshots = ["kis", "kis-calendar"]
        .into_iter()
        .map(|provider| {
            CommittedRawReader::new(root)
                .snapshot(
                    ApprovedRawScope::new(provider, "kr", config.raw_owner_uid)
                        .map_err(|e| SupplyError::Source(e.code()))?,
                    scan_limits()?,
                )
                .map_err(|e| SupplyError::Source(e.code()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let evidence =
        RawEvidenceSnapshots::new(&snapshots.iter().collect::<Vec<_>>(), entitlement, now)
            .map_err(|e| SupplyError::Source(e.code()))?;
    let mut originals = Vec::new();
    for snapshot in &snapshots {
        for batch in snapshot
            .batches()
            .iter()
            .filter(|b| evidence.matches_entitlement(b))
        {
            for file in batch.files().filter(|f| f.kind() == ResponseKind::Calendar) {
                originals.push(
                    resolve_original(
                        &evidence,
                        &SourceFileReference {
                            provider: snapshot.scope().provider().into(),
                            market: "kr".into(),
                            batch_id: batch.batch_id(),
                            file_name: file.file_name().into(),
                            content_hash: file.content_hash().clone(),
                            retrieved_at: batch.retrieved_at(),
                        },
                    )
                    .map_err(|e| SupplyError::Source(e.code()))?,
                );
            }
        }
    }
    let projections = project_calendar_candidates(&originals, config.owner, &config.policy_sha256)
        .map_err(|e| SupplyError::Source(e.code()))?;
    let first = config
        .initial_civil_frontier
        .checked_add_days(1)
        .map_err(|_| SupplyError::Config)?;
    let expected = expected_calendar_session(&projections.iter().collect::<Vec<_>>(), first, now)
        .map_err(|e| SupplyError::Source(e.code()))?;
    if let Some(claim) = consumed {
        // File/receipt existence alone is not completed acquisition. Resolve the
        // exact originally reserved batch and independently check its range.
        let batch = snapshots[0]
            .batches()
            .iter()
            .find(|b| b.batch_id() == claim.batch_id && evidence.matches_entitlement(b))
            .ok_or(SupplyError::ConsumedIncomplete)?;
        if batch.files().count() < 7 || claim.end > expected.session() {
            return Err(SupplyError::ConsumedIncomplete);
        }
        let document = reconstruct_actions(
            &evidence,
            claim
                .start
                .checked_add_days(-1)
                .map_err(|_| SupplyError::State)?,
            claim.end,
        )
        .map_err(|e| SupplyError::Source(e.code()))?;
        if document
            .witnesses()
            .iter()
            .filter(|chain| {
                chain.range_start == claim.start
                    && chain.range_end == claim.end
                    && chain.scope == market_data::daily_price_only::DailyActionScopeV1::WholeMarket
                    && chain
                        .pages
                        .iter()
                        .all(|page| page.batch_id == claim.batch_id)
            })
            .count()
            != 7
        {
            return Err(SupplyError::ConsumedIncomplete);
        }
    }
    let span = expected
        .session()
        .as_naive_date()
        .signed_duration_since(first.as_naive_date())
        .num_days();
    if !(0..370).contains(&span) {
        return Err(SupplyError::GapLimit);
    }
    let mut frontier = config.initial_civil_frontier;
    let mut gap = false;
    let mut day = first;
    while day <= expected.session() {
        match reconstruct_actions(
            &evidence,
            day.checked_add_days(-1).map_err(|_| SupplyError::Config)?,
            day,
        ) {
            Ok(_) if !gap => frontier = day,
            Ok(_) => {}
            Err(DailyVerificationError::SourceCoverageGap) => gap = true,
            Err(error) => return Err(SupplyError::Source(error.code())),
        }
        day = day.checked_add_days(1).map_err(|_| SupplyError::Config)?;
    }
    let proposal = plan_action_supply_proposal(
        &SupplyProposalConfig::Proposed {
            owner: config.owner,
            policy_sha256: config.policy_sha256.clone(),
            activation_date: config.activation_date,
        },
        Some(&expected),
        frontier,
        &SupplyDayStateClaim {
            kst_date: today,
            range_attempt_reserved: consumed.is_some(),
            wire_gets_consumed: if consumed.is_some() {
                WIRE_RESERVATION
            } else {
                0
            },
        },
        now,
    )
    .map_err(|error| match error {
        market_data::daily_price_only::supply::SupplyPlanError::GapLimit => SupplyError::GapLimit,
        _ => SupplyError::Source("SUPPLY_PLAN_INVALID"),
    })?;
    Ok(SourcePlan {
        start: match proposal {
            ActionSupplyProposal::Range(range) => Some(range.start()),
            ActionSupplyProposal::Noop(_) => None,
        },
        end: expected.session(),
    })
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Claim {
    schema: String,
    day: TradingDate,
    config_sha256: ContentHash,
    start: TradingDate,
    end: TradingDate,
    batch_id: BatchId,
    reserved_wire_gets: u16,
}

struct DayStore {
    raw: File,
    dir: File,
    lock: File,
    day: TradingDate,
    uid: u32,
}

fn safe_file(file: &File, uid: u32, directory: bool) -> Result<(), SupplyError> {
    let m = file.metadata().map_err(|_| SupplyError::State)?;
    let valid = if directory {
        m.is_dir() && m.mode() & 0o7777 == 0o700
    } else {
        m.is_file() && m.nlink() == 1 && m.mode() & 0o7777 == 0o600 && m.len() <= 4096
    };
    if !valid || m.uid() != uid {
        return Err(SupplyError::State);
    }
    Ok(())
}

impl DayStore {
    fn open(
        root: &Path,
        day: TradingDate,
        uid: u32,
        create: bool,
    ) -> Result<Option<Self>, SupplyError> {
        if !root.is_absolute() {
            return Err(SupplyError::State);
        }
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let mut held = open("/", flags, Mode::empty()).map_err(|_| SupplyError::State)?;
        for part in root.components() {
            match part {
                Component::RootDir => {}
                Component::Normal(part) => {
                    held =
                        openat(&held, part, flags, Mode::empty()).map_err(|_| SupplyError::State)?
                }
                _ => return Err(SupplyError::State),
            }
        }
        let raw: File = held.into();
        let meta = raw.metadata().map_err(|_| SupplyError::State)?;
        if meta.uid() != uid || meta.mode() & 0o022 != 0 {
            return Err(SupplyError::State);
        }
        if create {
            match mkdirat(&raw, ".r2-action-supply", Mode::from_raw_mode(0o700)) {
                Ok(()) => raw.sync_all().map_err(|_| SupplyError::State)?,
                Err(rustix::io::Errno::EXIST) => {}
                Err(_) => return Err(SupplyError::State),
            }
        }
        let dir: File = match openat(&raw, ".r2-action-supply", flags, Mode::empty()) {
            Ok(fd) => fd.into(),
            Err(rustix::io::Errno::NOENT) if !create => return Ok(None),
            Err(_) => return Err(SupplyError::State),
        };
        safe_file(&dir, uid, true)?;
        let flags = OFlags::RDWR
            | OFlags::NOFOLLOW
            | OFlags::CLOEXEC
            | OFlags::NONBLOCK
            | if create {
                OFlags::CREATE
            } else {
                OFlags::empty()
            };
        let lock: File = openat(&dir, "supply.lock", flags, Mode::from_raw_mode(0o600))
            .map_err(|_| SupplyError::State)?
            .into();
        safe_file(&lock, uid, false)?;
        lock.try_lock().map_err(|_| SupplyError::State)?;
        if create {
            dir.sync_all().map_err(|_| SupplyError::State)?;
        }
        Ok(Some(Self {
            raw,
            dir,
            lock,
            day,
            uid,
        }))
    }

    fn name(&self) -> String {
        format!("{}.json", self.day.to_iso())
    }

    fn read(&self, config: &ContentHash) -> Result<Option<Claim>, SupplyError> {
        self.verify_namespace()?;
        let mut file: File = match openat(
            &self.dir,
            self.name(),
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        ) {
            Ok(fd) => fd.into(),
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(_) => return Err(SupplyError::State),
        };
        safe_file(&file, self.uid, false)?;
        let mut bytes = Vec::new();
        (&mut file)
            .take(4097)
            .read_to_end(&mut bytes)
            .map_err(|_| SupplyError::State)?;
        if bytes.len() > 4096 {
            return Err(SupplyError::State);
        }
        let c: Claim = serde_json::from_slice(&bytes).map_err(|_| SupplyError::State)?;
        if c.schema != "r2-action-supply-claim-v1"
            || c.day != self.day
            || &c.config_sha256 != config
            || c.reserved_wire_gets != WIRE_RESERVATION
            || c.start > c.end
            || c.end > c.day
            || c.batch_id.to_string() == Uuid::nil().to_string()
            || c.end
                .as_naive_date()
                .signed_duration_since(c.start.as_naive_date())
                .num_days()
                >= 14
        {
            return Err(SupplyError::State);
        }
        Ok(Some(c))
    }

    fn consume(&self, claim: &Claim) -> Result<(), SupplyError> {
        self.verify_namespace()?;
        let mut file: File = openat(
            &self.dir,
            self.name(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(|_| SupplyError::State)?
        .into();
        let bytes = serde_json::to_vec(claim).map_err(|_| SupplyError::State)?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| SupplyError::State)?;
        self.dir.sync_all().map_err(|_| SupplyError::State)?;
        safe_file(&file, self.uid, false)?;
        self.verify_namespace()
    }

    fn verify_namespace(&self) -> Result<(), SupplyError> {
        let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
        let dir: File = openat(
            &self.raw,
            ".r2-action-supply",
            flags | OFlags::DIRECTORY,
            Mode::empty(),
        )
        .map_err(|_| SupplyError::State)?
        .into();
        let lock: File = openat(&dir, "supply.lock", flags, Mode::empty())
            .map_err(|_| SupplyError::State)?
            .into();
        for (named, held, directory) in [(&dir, &self.dir, true), (&lock, &self.lock, false)] {
            safe_file(named, self.uid, directory)?;
            let a = named.metadata().map_err(|_| SupplyError::State)?;
            let b = held.metadata().map_err(|_| SupplyError::State)?;
            if (a.dev(), a.ino()) != (b.dev(), b.ino()) {
                return Err(SupplyError::State);
            }
        }
        Ok(())
    }
}

/// Explicit CLI only. No database construction, legacy normalization or
/// admission. `check` performs no reservation, transport construction or capture.
pub async fn run(
    values: &HashMap<String, String>,
    check: bool,
) -> Result<SupplyResult, SupplyError> {
    run_with(values, check, UtcTimestamp::now(), |key, secret| {
        crate::worker::build_production_kis_provider_from_files(key, secret)
            .map_err(|_| SupplyError::Capture)
    })
    .await
}

async fn run_with<R: KisRead>(
    values: &HashMap<String, String>,
    check: bool,
    now: UtcTimestamp,
    make_provider: impl FnOnce(&Path, &Path) -> Result<KisProvider<R>, SupplyError>,
) -> Result<SupplyResult, SupplyError> {
    let started = Instant::now();
    let Some(bytes) = values.get(CONFIG_KEY).filter(|s| !s.is_empty()) else {
        return Ok(outcome("DISABLED"));
    };
    let bytes = decode_config(bytes)?;
    let config: Config = serde_json::from_slice(&bytes).map_err(|_| SupplyError::Config)?;
    if !config.enabled {
        return Ok(outcome("DISABLED"));
    }
    let value = |key: &str| {
        values
            .get(key)
            .map(String::as_str)
            .ok_or(SupplyError::Config)
    };
    let entitlement = value("RESEARCH_ENTITLEMENT_REFERENCE")?;
    config.validate(value("LAGRANGE_CODE_COMMIT")?, entitlement)?;
    if value("APP_ENV")? != "production"
        || value("RESEARCH_FETCH_MODE")? != "credentialed"
        || value("KIS_READ_COORDINATION_MODE")? != "shared_required"
        || value("RESEARCH_CANDIDATE_ENABLED").unwrap_or("false") != "false"
    {
        return Err(SupplyError::Config);
    }
    let kst = now
        .as_datetime()
        .with_timezone(&FixedOffset::east_opt(9 * 3600).unwrap());
    let today =
        TradingDate::parse(&kst.date_naive().to_string()).map_err(|_| SupplyError::Config)?;
    if today < config.activation_date {
        return Ok(outcome("BEFORE_ACTIVATION"));
    }
    if (kst.hour(), kst.minute()) < (16, 30) {
        return Ok(outcome("BEFORE_CUTOFF"));
    }
    let root = Path::new(value("RESEARCH_RAW_ROOT")?);
    let hash = config.hash()?;
    let store = DayStore::open(&root.join("raw"), today, config.raw_owner_uid, !check)?;
    let consumed = store.as_ref().map(|s| s.read(&hash)).transpose()?.flatten();
    let plan = source_plan(root, &config, entitlement, today, now, consumed.as_ref())?;
    let budget = Duration::from_secs(config.timeout_seconds);
    if started.elapsed() >= budget {
        return Err(SupplyError::PlanningDeadline);
    }
    if consumed.is_some() {
        let mut result = outcome("VERIFIED_PRIOR_CAPTURE");
        result.reserved_wire_gets = WIRE_RESERVATION;
        result.batch_id = consumed.map(|c| c.batch_id);
        return Ok(result);
    }
    let Some(start) = plan.start else {
        return Ok(outcome("ALREADY_COVERED"));
    };
    if check {
        let mut result = outcome("WOULD_CAPTURE_NO_RESERVATION");
        result.start = Some(start);
        result.end = Some(plan.end);
        return Ok(result);
    }
    let claim = Claim {
        schema: "r2-action-supply-claim-v1".into(),
        day: today,
        config_sha256: hash,
        start,
        end: plan.end,
        batch_id: BatchId::generate(),
        reserved_wire_gets: WIRE_RESERVATION,
    };
    store.as_ref().ok_or(SupplyError::State)?.consume(&claim)?;
    // The entire conservative retry/page allowance is now durable and consumed.
    // A crash during credential construction or capture cannot restore it.
    let key = Path::new(value("KIS_APP_KEY_FILE")?);
    let secret = Path::new(value("KIS_APP_SECRET_FILE")?);
    if !key.is_absolute() || !secret.is_absolute() {
        return Err(SupplyError::Config);
    }
    let provider = make_provider(key, secret)?;
    let remaining = budget
        .checked_sub(started.elapsed())
        .ok_or(SupplyError::Timeout)?;
    tokio::time::timeout(
        remaining,
        ingest_kis_action_range_with_batch_id(
            &RawStore::new(root),
            &provider,
            "kr",
            start,
            plan.end,
            now,
            KisActionRangeScope::WholeMarket,
            Some(entitlement),
            claim.batch_id,
        ),
    )
    .await
    .map_err(|_| SupplyError::Timeout)?
    .map_err(|_| SupplyError::Capture)?;
    source_plan(root, &config, entitlement, today, now, Some(&claim))?;
    if started.elapsed() >= budget {
        return Err(SupplyError::Timeout);
    }
    Ok(SupplyResult {
        outcome: "RAW_CAPTURE_INDEPENDENTLY_VERIFIED",
        start: Some(start),
        end: Some(plan.end),
        batch_id: Some(claim.batch_id),
        reserved_wire_gets: WIRE_RESERVATION,
        raw_only: true,
        admitted: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kis_client::{KisError, MarketDataReply};
    use market_data::ingest::{IngestRequest, ingest_kis_calendar_with_batch_id};
    use serde_json::json;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    fn date(s: &str) -> TradingDate {
        TradingDate::parse(s).unwrap()
    }
    fn now() -> UtcTimestamp {
        UtcTimestamp::parse_rfc3339("2026-08-31T08:00:00Z").unwrap()
    }
    fn config() -> Config {
        Config {
            schema: "r2-action-supply-v1".into(),
            enabled: true,
            release_commit: "a".repeat(40),
            owner: Uuid::from_u128(1),
            policy_sha256: ContentHash::from_bytes(b"policy"),
            activation_date: date("2026-08-31"),
            base_candidate_sha256: serde_json::from_value(json!(
                "sha256:b0fb82f6a580f3def13a1e1e34bea68e30d95dc720306e7ee54b6c3199cf402d"
            ))
            .unwrap(),
            initial_civil_frontier: date("2026-08-28"),
            entitlement_reference_sha256: ContentHash::from_bytes(b"fixture:licensed"),
            raw_owner_uid: unsafe { libc::geteuid() },
            timeout_seconds: 300,
        }
    }

    #[derive(Debug, Clone)]
    struct Reader {
        calls: Arc<AtomicUsize>,
        fail: bool,
        delayed: bool,
        calendar_end: TradingDate,
    }
    impl KisRead for Reader {
        async fn get(
            &self,
            path: &str,
            _: &str,
            _: &[(String, String)],
            _: Option<&str>,
        ) -> Result<MarketDataReply, KisError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.delayed {
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            if self.fail {
                return Err(KisError::Connect {
                    reason: "fixture".into(),
                });
            }
            let body = if path.ends_with("chk-holiday") {
                let mut rows = Vec::new();
                let mut day = date("2026-08-29");
                while day <= self.calendar_end {
                    rows.push(json!({"bass_dt": day.to_iso().replace('-', ""),
                        "opnd_yn": if day < date("2026-08-31") { "N" } else { "Y" }}));
                    day = day.checked_add_days(1).unwrap();
                }
                json!({"rt_cd":"0","output":rows})
            } else {
                json!({"rt_cd":"0","output1":[]})
            };
            Ok(MarketDataReply {
                body: serde_json::to_vec(&body).unwrap(),
                continuation: Some("F".into()),
            })
        }
    }
    fn reader(fail: bool) -> Reader {
        Reader {
            calls: Arc::new(AtomicUsize::new(0)),
            fail,
            delayed: false,
            calendar_end: date("2026-08-31"),
        }
    }

    fn values(root: &Path, c: &Config) -> HashMap<String, String> {
        HashMap::from([
            (
                CONFIG_KEY.into(),
                serde_json::to_vec(c)
                    .unwrap()
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect(),
            ),
            ("APP_ENV".into(), "production".into()),
            ("RESEARCH_FETCH_MODE".into(), "credentialed".into()),
            (
                "KIS_READ_COORDINATION_MODE".into(),
                "shared_required".into(),
            ),
            ("LAGRANGE_CODE_COMMIT".into(), c.release_commit.clone()),
            (
                "RESEARCH_ENTITLEMENT_REFERENCE".into(),
                "fixture:licensed".into(),
            ),
            ("RESEARCH_RAW_ROOT".into(), root.to_str().unwrap().into()),
            ("KIS_APP_KEY_FILE".into(), "/not-read/key".into()),
            ("KIS_APP_SECRET_FILE".into(), "/not-read/secret".into()),
        ])
    }
    fn forbid(_: &Path, _: &Path) -> Result<KisProvider<Reader>, SupplyError> {
        panic!("provider must not be constructed");
    }
    fn protect(path: &Path) {
        let m = std::fs::metadata(path).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(m.mode() & !0o022)).unwrap();
        if m.is_dir() {
            for e in std::fs::read_dir(path).unwrap() {
                protect(&e.unwrap().path());
            }
        }
    }
    async fn fixture() -> tempfile::TempDir {
        fixture_until(date("2026-08-31")).await
    }
    async fn fixture_until(end: TradingDate) -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        let store = RawStore::new(temp.path());
        let mut r = reader(false);
        r.calendar_end = end;
        let p = KisProvider::kr_etf_core(r);
        ingest_kis_calendar_with_batch_id(
            &store,
            &p,
            &IngestRequest::new(
                "kr".into(),
                end,
                UtcTimestamp::parse_rfc3339(&format!("{}T08:00:00Z", end.to_iso())).unwrap(),
            ),
            Some("fixture:licensed"),
            BatchId::generate(),
        )
        .await
        .unwrap();
        store.read_reconciled_manifest("kis", "kr").unwrap();
        protect(temp.path());
        temp
    }

    #[test]
    fn supplier_config_cannot_invent_base_frontier_or_expand_deadline() {
        for value in ["7", "zz", "7B7D", "{\"unsafe\":true}", "é"] {
            assert!(decode_config(value).is_err());
        }
        assert!(decode_config(&"00".repeat(4097)).is_err());
        assert_eq!(decode_config("7b7d").unwrap(), b"{}");
        let mut c = config();
        c.validate(&"a".repeat(40), "fixture:licensed").unwrap();
        c.initial_civil_frontier = date("2026-08-29");
        assert!(c.validate(&"a".repeat(40), "fixture:licensed").is_err());
        c = config();
        c.timeout_seconds = 301;
        assert!(c.validate(&"a".repeat(40), "fixture:licensed").is_err());
        c = config();
        assert!(c.validate(&"b".repeat(40), "fixture:licensed").is_err());
        assert!(c.validate(&"a".repeat(40), "foreign").is_err());
    }

    #[tokio::test]
    async fn disabled_and_early_supplier_never_construct_provider_or_state() {
        assert_eq!(
            run_with(&HashMap::new(), false, now(), forbid)
                .await
                .unwrap()
                .outcome,
            "DISABLED"
        );
        let temp = tempfile::tempdir().unwrap();
        let mut c = config();
        c.enabled = false;
        assert_eq!(
            run_with(&values(temp.path(), &c), false, now(), forbid)
                .await
                .unwrap()
                .outcome,
            "DISABLED"
        );
        c.enabled = true;
        c.activation_date = date("2026-09-01");
        assert_eq!(
            run_with(&values(temp.path(), &c), false, now(), forbid)
                .await
                .unwrap()
                .outcome,
            "BEFORE_ACTIVATION"
        );
        c = config();
        assert_eq!(
            run_with(
                &values(temp.path(), &c),
                false,
                UtcTimestamp::parse_rfc3339("2026-08-31T07:29:59Z").unwrap(),
                forbid
            )
            .await
            .unwrap()
            .outcome,
            "BEFORE_CUTOFF"
        );
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
    }

    #[test]
    fn supplier_claim_is_durable_exclusive_and_rejects_tampering() {
        let t = tempfile::tempdir().unwrap();
        let c = config();
        let hash = c.hash().unwrap();
        let claim = Claim {
            schema: "r2-action-supply-claim-v1".into(),
            day: date("2026-08-31"),
            config_sha256: hash.clone(),
            start: date("2026-08-29"),
            end: date("2026-08-31"),
            batch_id: BatchId::generate(),
            reserved_wire_gets: 280,
        };
        let s = DayStore::open(t.path(), claim.day, c.raw_owner_uid, true)
            .unwrap()
            .unwrap();
        assert!(DayStore::open(t.path(), claim.day, c.raw_owner_uid, true).is_err());
        s.consume(&claim).unwrap();
        assert!(s.consume(&claim).is_err());
        drop(s);
        let s = DayStore::open(t.path(), claim.day, c.raw_owner_uid, false)
            .unwrap()
            .unwrap();
        assert_eq!(s.read(&hash).unwrap().unwrap().batch_id, claim.batch_id);
        assert!(s.read(&ContentHash::from_bytes(b"other-config")).is_err());
        drop(s);
        let path = t.path().join(".r2-action-supply/2026-08-31.json");
        std::fs::write(path, b"{partial").unwrap();
        let s = DayStore::open(t.path(), claim.day, c.raw_owner_uid, false)
            .unwrap()
            .unwrap();
        assert!(s.read(&hash).is_err());
    }

    #[tokio::test]
    async fn supplier_source_check_does_not_reserve_and_capture_revalidates_on_restart() {
        let t = fixture().await;
        let v = values(t.path(), &config());
        let result = run_with(&v, true, now(), forbid).await.unwrap();
        assert_eq!(result.outcome, "WOULD_CAPTURE_NO_RESERVATION");
        assert_eq!(result.start, Some(date("2026-08-29")));
        assert!(!t.path().join("raw/.r2-action-supply").exists());
        let r = reader(false);
        let copy = r.clone();
        let result = run_with(&v, false, now(), move |_, _| {
            Ok(KisProvider::kr_etf_core(copy))
        })
        .await
        .unwrap();
        assert_eq!(result.outcome, "RAW_CAPTURE_INDEPENDENTLY_VERIFIED");
        assert_eq!(r.calls.load(Ordering::SeqCst), 7);
        assert_eq!(result.reserved_wire_gets, 280);
        assert!(!result.admitted);
        assert_eq!(
            run_with(&v, false, now(), forbid).await.unwrap().outcome,
            "VERIFIED_PRIOR_CAPTURE"
        );
    }

    #[tokio::test]
    async fn failed_supplier_burns_day_allowance_and_does_not_retry() {
        let t = fixture().await;
        let v = values(t.path(), &config());
        let r = reader(true);
        let copy = r.clone();
        assert!(matches!(
            run_with(&v, false, now(), move |_, _| Ok(KisProvider::kr_etf_core(
                copy
            )))
            .await,
            Err(SupplyError::Capture)
        ));
        assert_eq!(r.calls.load(Ordering::SeqCst), 1);
        assert!(matches!(
            run_with(&v, false, now(), forbid).await,
            Err(SupplyError::ConsumedIncomplete)
        ));
    }

    #[tokio::test]
    async fn supplier_calendar_gap_and_foreign_entitlement_do_not_dispatch() {
        let t = fixture().await;
        let mut c = config();
        c.entitlement_reference_sha256 = ContentHash::from_bytes(b"other");
        let mut v = values(t.path(), &c);
        v.insert("RESEARCH_ENTITLEMENT_REFERENCE".into(), "other".into());
        assert!(matches!(
            run_with(&v, false, now(), forbid).await,
            Err(SupplyError::Source(_))
        ));
        let v = values(t.path(), &config());
        let later = UtcTimestamp::parse_rfc3339("2026-09-02T08:00:00Z").unwrap();
        assert!(matches!(
            run_with(&v, false, later, forbid).await,
            Err(SupplyError::Source(_))
        ));
    }

    #[test]
    fn supplier_held_namespace_replacement_and_symlink_claim_fail() {
        let t = tempfile::tempdir().unwrap();
        let c = config();
        let s = DayStore::open(t.path(), date("2026-08-31"), c.raw_owner_uid, true)
            .unwrap()
            .unwrap();
        let dir = t.path().join(".r2-action-supply");
        std::os::unix::fs::symlink(t.path().join("outside"), dir.join(s.name())).unwrap();
        assert!(s.read(&c.hash().unwrap()).is_err());
        std::fs::rename(&dir, t.path().join("retained")).unwrap();
        let replacement = DayStore::open(t.path(), s.day, c.raw_owner_uid, true)
            .unwrap()
            .unwrap();
        assert!(s.verify_namespace().is_err());
        assert!(replacement.verify_namespace().is_ok());
    }

    #[tokio::test]
    async fn supplier_rejects_large_gap_without_chopping_or_reserving() {
        let t = fixture_until(date("2026-09-15")).await;
        let v = values(t.path(), &config());
        let later = UtcTimestamp::parse_rfc3339("2026-09-15T08:00:00Z").unwrap();
        assert!(matches!(
            run_with(&v, false, later, forbid).await,
            Err(SupplyError::GapLimit)
        ));
        assert!(
            !t.path()
                .join("raw/.r2-action-supply/2026-09-15.json")
                .exists()
        );
    }

    #[tokio::test]
    async fn supplier_timeout_consumes_attempt_and_never_replays() {
        let t = fixture().await;
        let mut c = config();
        c.timeout_seconds = 1;
        let v = values(t.path(), &c);
        let mut r = reader(false);
        r.delayed = true;
        let copy = r.clone();
        assert!(matches!(
            run_with(&v, false, now(), move |_, _| Ok(KisProvider::kr_etf_core(
                copy
            )))
            .await,
            Err(SupplyError::Timeout)
        ));
        assert_eq!(r.calls.load(Ordering::SeqCst), 1);
        assert!(matches!(
            run_with(&v, false, now(), forbid).await,
            Err(SupplyError::ConsumedIncomplete)
        ));
    }

    #[tokio::test]
    async fn next_day_uses_verified_frontier_without_reusing_previous_reservation() {
        let t = fixture().await;
        let v = values(t.path(), &config());
        let first = run_with(&v, false, now(), |_, _| {
            Ok(KisProvider::kr_etf_core(reader(false)))
        })
        .await
        .unwrap();
        let later = UtcTimestamp::parse_rfc3339("2026-09-01T08:00:00Z").unwrap();
        let mut calendar = reader(false);
        calendar.calendar_end = date("2026-09-01");
        ingest_kis_calendar_with_batch_id(
            &RawStore::new(t.path()),
            &KisProvider::kr_etf_core(calendar),
            &IngestRequest::new("kr".into(), date("2026-09-01"), later),
            Some("fixture:licensed"),
            BatchId::generate(),
        )
        .await
        .unwrap();
        let r = reader(false);
        let copy = r.clone();
        let second = run_with(&v, false, later, move |_, _| {
            Ok(KisProvider::kr_etf_core(copy))
        })
        .await
        .unwrap();
        assert_eq!(second.start, Some(date("2026-09-01")));
        assert_eq!(second.end, second.start);
        assert_ne!(first.batch_id, second.batch_id);
        assert_eq!(r.calls.load(Ordering::SeqCst), 7);
        for day in ["2026-08-31", "2026-09-01"] {
            assert!(
                t.path()
                    .join(format!("raw/.r2-action-supply/{day}.json"))
                    .is_file()
            );
        }
        assert_eq!(
            run_with(&v, false, later, forbid).await.unwrap().outcome,
            "VERIFIED_PRIOR_CAPTURE"
        );
    }

    #[tokio::test]
    async fn unrelated_complete_batch_cannot_satisfy_consumed_identity() {
        let t = fixture().await;
        let c = config();
        let s = DayStore::open(
            &t.path().join("raw"),
            date("2026-08-31"),
            c.raw_owner_uid,
            true,
        )
        .unwrap()
        .unwrap();
        s.consume(&Claim {
            schema: "r2-action-supply-claim-v1".into(),
            day: s.day,
            config_sha256: c.hash().unwrap(),
            start: date("2026-08-29"),
            end: s.day,
            batch_id: BatchId::generate(),
            reserved_wire_gets: WIRE_RESERVATION,
        })
        .unwrap();
        drop(s);
        ingest_kis_action_range_with_batch_id(
            &RawStore::new(t.path()),
            &KisProvider::kr_etf_core(reader(false)),
            "kr",
            date("2026-08-29"),
            date("2026-08-31"),
            now(),
            KisActionRangeScope::WholeMarket,
            Some("fixture:licensed"),
            BatchId::generate(),
        )
        .await
        .unwrap();
        assert!(matches!(
            run_with(&values(t.path(), &c), false, now(), forbid).await,
            Err(SupplyError::ConsumedIncomplete)
        ));
    }
}
